//! Briefing generation — LLM call, caching, and the background briefing loop.
//!
//! The briefing loop runs once per trading day at the configured hour (UTC).
//! It skips weekends and holidays, is idempotent (won't generate twice for
//! the same day), and can be woken early via `state.notify` when the scheduler
//! publishes a PredictionUpdate.

use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use tokio::sync::Notify;
use tracing::{debug, info, warn};

use super::{AdvisorBriefing, AdvisorConfig, AdvisorState, BriefingError, hash_context};
use super::context::build_context;
use super::prompt::{compile_system_prompt, compile_user_prompt, parse_response};

use crate::db::DbPool;

/// Generate a briefing by calling the LLM. Tries the primary model first,
/// falls back to `fallback_model` on transient failure.
pub async fn generate_briefing(
    state: &AdvisorState,
    pool: &DbPool,
    symbol: &str,
) -> Result<AdvisorBriefing, BriefingError> {
    let context = build_context(pool, symbol)
        .await
        .map_err(|e| BriefingError::Disabled(format!("context build failed: {e}")))?;

    let system = compile_system_prompt();
    let user = compile_user_prompt(&context);

    let (raw, model_used) = call_llm_with_fallback(
        &state.cfg,
        system,
        &user,
    )
    .await?;

    let today = trading_date_et();
    let briefing = parse_response(&raw, &model_used, Utc::now(), today)?;

    Ok(briefing)
}

/// Call the LLM with the primary model, falling back on failure.
async fn call_llm_with_fallback(
    cfg: &AdvisorConfig,
    system: &str,
    user: &str,
) -> Result<(String, String), BriefingError> {
    // Try primary.
    match call_openrouter(&cfg.api_key, &cfg.model, &cfg.api_base, system, user, cfg.request_timeout_seconds).await {
        Ok(raw) => return Ok((raw, cfg.model.clone())),
        Err(e) => warn!(model = %cfg.model, error = %e, "primary model failed, trying fallback"),
    }

    // Try fallback.
    if cfg.fallback_model.is_empty() || cfg.fallback_model == cfg.model {
        return Err(BriefingError::LlmUnavailable {
            status: 0,
            body: "primary model failed and no fallback configured".to_string(),
        });
    }

    match call_openrouter(&cfg.api_key, &cfg.fallback_model, &cfg.api_base, system, user, cfg.request_timeout_seconds).await {
        Ok(raw) => {
            info!(model = %cfg.fallback_model, "fallback model succeeded");
            Ok((raw, cfg.fallback_model.clone()))
        }
        Err(e) => Err(BriefingError::LlmUnavailable {
            status: 0,
            body: format!("both primary and fallback failed: {e}"),
        }),
    }
}

/// Call the OpenRouter chat completions endpoint.
pub async fn call_openrouter(
    api_key: &str,
    model: &str,
    api_base: &str,
    system: &str,
    user: &str,
    timeout_secs: u64,
) -> Result<String, BriefingError> {
    let payload = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ],
        "max_tokens": 1200,
        "temperature": 0.4,
        "top_p": 0.95,
        "stream": false
    });

    let timeout = Duration::from_secs(timeout_secs.max(5));
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| BriefingError::Disabled(format!("reqwest client: {e}")))?;

    let resp = client
        .post(api_base)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| BriefingError::LlmUnavailable {
            status: 0,
            body: format!("request failed: {e}"),
        })?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(BriefingError::LlmUnavailable {
            status: status.as_u16(),
            body,
        });
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| BriefingError::ParseFailed {
            raw: String::new(),
            reason: format!("response JSON parse: {e}"),
        })?;

    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    if content.is_empty() {
        return Err(BriefingError::ParseFailed {
            raw: serde_json::to_string(&body).unwrap_or_default(),
            reason: "LLM response has empty content".to_string(),
        });
    }

    Ok(content)
}

/// Run the briefing loop in the background.
///
/// Wakes at `briefing_hour_utc` each trading day, or on `state.notify()`.
/// Skips non-trading days. Skips if a briefing already exists for today.
pub async fn run_briefing_loop(
    state: Arc<AdvisorState>,
    pool: DbPool,
    symbol: String,
    scheduler_notify: Arc<Notify>,
) {
    if !state.cfg.is_enabled() {
        warn!("advisor is disabled — briefing loop will not start");
        return;
    }

    info!(
        model = %state.cfg.model,
        briefing_hour_utc = state.cfg.briefing_hour_utc,
        "advisor briefing loop starting"
    );

    // Generate immediately on startup (don't wait for the scheduled hour).
    // This handles the case where the engine restarts mid-day.
    // Skip the trading-day check for the initial startup briefing so we
    // always have at least one cached entry to show the user.
    info!(date = %trading_date_et(), "generating startup advisor briefing");
    let start = std::time::Instant::now();
    match generate_briefing(&state, &pool, &symbol).await {
        Ok(briefing) => {
            let latency_ms = start.elapsed().as_millis() as i64;
            info!(
                date = %briefing.for_date,
                model = %briefing.model_used,
                latency_ms,
                parse_status = %briefing.parse_status,
                "advisor briefing generated"
            );
            state.cache_briefing(&briefing.for_date, briefing.clone(), "startup".to_string());
            let event = crate::api::ws::TelemetryEvent::AdvisorBriefing {
                for_date: briefing.for_date.clone(),
                briefing,
            };
            let _ = state.tx.send(event);
        }
        Err(e) => {
            warn!(error = %e, "startup advisor briefing failed");
        }
    }

    loop {
        // Wait until next briefing time OR a manual refresh signal.
        let next = compute_next_briefing_ts(state.cfg.briefing_hour_utc);
        let sleep = tokio::time::sleep_until(next);

        tokio::select! {
            _ = sleep => {}
            _ = state.notify.notified() => {
                debug!("advisor: manual refresh triggered");
                // Don't wait for the next scheduled time — just run now.
                // But still check trading day.
            }
            _ = scheduler_notify.notified() => {
                debug!("advisor: scheduler triggered refresh");
                // Prediction update — run now if we're in a trading window.
            }
        }

        // ── 1. Check if today is a trading day ──
        let today = trading_date_et();
        let now = Utc::now();
        let ms = crate::market_hours::market_state(now.timestamp());
        if !ms.is_trading_day {
            debug!(date = %today, "non-trading day — skipping briefing");
            continue;
        }

        // ── 2. Check if briefing already exists for today ──
        if state.cached(&today).is_some() {
            debug!(date = %today, "briefing already cached for today — skipping");
            continue;
        }

        // ── 3. Generate ──
        info!(date = %today, "generating advisor briefing");
        let start = std::time::Instant::now();

        match generate_briefing(&state, &pool, &symbol).await {
            Ok(briefing) => {
                let latency_ms = start.elapsed().as_millis() as i64;
                info!(
                    date = %today,
                    model = %briefing.model_used,
                    latency_ms,
                    parse_status = %briefing.parse_status,
                    "advisor briefing generated"
                );

                // Cache it.
                let ctx_hash = "cached".to_string(); // simplified — compute from context
                state.cache_briefing(&today, briefing.clone(), ctx_hash);

                // Broadcast via WS.
                let event = crate::api::ws::TelemetryEvent::AdvisorBriefing {
                    for_date: today,
                    briefing: briefing.clone(),
                };
                let _ = state.tx.send(event);
            }
            Err(e) => {
                warn!(date = %today, error = %e, "advisor briefing generation failed");
            }
        }
    }
}

/// Try to generate a briefing for today. Logs but never panics.
async fn try_generate_briefing(state: &Arc<AdvisorState>, pool: &DbPool, symbol: &str) {
    let today = trading_date_et();
    let ms = crate::market_hours::market_state(Utc::now().timestamp());
    if !ms.is_trading_day {
        info!(date = %today, "non-trading day — skipping startup briefing");
        return;
    }
    if state.cached(&today).is_some() {
        info!(date = %today, "briefing already cached — skipping startup briefing");
        return;
    }

    info!(date = %today, "generating startup advisor briefing");
    let start = std::time::Instant::now();

    match generate_briefing(state, pool, symbol).await {
        Ok(briefing) => {
            let latency_ms = start.elapsed().as_millis() as i64;
            info!(
                date = %today,
                model = %briefing.model_used,
                latency_ms,
                parse_status = %briefing.parse_status,
                "advisor briefing generated"
            );
            state.cache_briefing(&today, briefing.clone(), "startup".to_string());
            let event = crate::api::ws::TelemetryEvent::AdvisorBriefing {
                for_date: today,
                briefing,
            };
            let _ = state.tx.send(event);
        }
        Err(e) => {
            warn!(date = %today, error = %e, "startup advisor briefing failed");
        }
    }
}

/// Compute the next briefing timestamp.
///
/// Returns the next occurrence of `hour_utc`:00 today, or tomorrow if
/// the current time is already past today's briefing hour.
fn compute_next_briefing_ts(hour_utc: u32) -> tokio::time::Instant {
    let now = Utc::now();
    let today = now.date_naive();

    let briefing_today = today
        .and_hms_opt(hour_utc, 0, 0)
        .and_then(|dt| dt.and_local_timezone(Utc).single())
        .unwrap_or(now);

    let target = if now >= briefing_today {
        // Already past today's time — schedule for tomorrow.
        let tomorrow = today.succ_opt().unwrap_or(today);
        tomorrow
            .and_hms_opt(hour_utc, 0, 0)
            .and_then(|dt| dt.and_local_timezone(Utc).single())
            .unwrap_or(now + chrono::Duration::hours(24))
    } else {
        briefing_today
    };

    let duration = (target - now).to_std().unwrap_or(Duration::from_secs(3600));
    tokio::time::Instant::now() + duration
}

/// Get the current trading date in Eastern Time (YYYY-MM-DD).
fn trading_date_et() -> String {
    // ET is UTC-5 (EST) or UTC-4 (EDT). Use the market_hours module's offset.
    // Simplified: use UTC date for now — the market_hours module handles
    // the actual ET offset. The briefing loop gates on is_trading_day which
    // already handles the ET conversion.
    let now = Utc::now();
    // EST/EDT approximation: subtract 4-5 hours then take the date.
    // For now, use UTC date — the briefing fires at 13:00 UTC which is
    // 08:00 ET, so the UTC date IS the trading date during pre-market.
    now.format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_next_briefing_ts_returns_future() {
        let ts = compute_next_briefing_ts(13);
        let now = tokio::time::Instant::now();
        assert!(ts > now, "next briefing should be in the future");
    }
}