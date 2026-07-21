//! D4: OpenRouter hourly-cached LLM regime feature.
//!
//! Spawns a background task that wakes every hour (configurable TTL), sends a
//! concise prompt to an OpenRouter chat model asking it to classify the BTC
//! market regime, parses the response into a `llm_bull_prob` in [0,1], and
//! writes it to a process-wide `RwLock`. The per-bar feature pipeline reads
//! the cached value instantly via `read_cached_bull_prob()` — the LLM call is
//! NEVER in the per-bar latency path.
//!
//! On timeout or error: falls back to the last cached value (or 0.5 neutral).

use std::sync::RwLock;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::json;
use tracing::{info, warn};

/// Process-wide cache for the latest LLM regime probability.
/// `0.5` = neutral/absent (no signal yet).
static LLM_BULL_PROB: RwLock<f64> = RwLock::new(0.5);

/// Read the most recently cached LLM bull probability in [0,1].
pub fn read_cached_bull_prob() -> f64 {
    match LLM_BULL_PROB.read() {
        Ok(g) => *g,
        Err(_) => 0.5,
    }
}

/// Update the cached LLM bull probability.
pub fn write_cached_bull_prob(p: f64) {
    if let Ok(mut g) = LLM_BULL_PROB.write() {
        *g = p.clamp(0.0, 1.0);
    }
}

/// Configuration for the LLM regime cache task.
#[derive(Debug, Clone)]
pub struct LlmRegimeConfig {
    /// OpenRouter (or OmniRoute proxy) API key.
    pub api_key: String,
    /// Model ID, e.g. "openrouter/google/gemini-3.1-flash-lite" or "auto/gemini".
    pub model: String,
    /// Base URL for chat completions.
    /// Default: "https://openrouter.ai/api/v1/chat/completions"
    /// For OmniRoute proxy: "http://host.docker.internal:20128/v1/chat/completions"
    pub api_base: String,
    /// Cache TTL in seconds (typically 3600 = hourly).
    pub cache_ttl_seconds: u64,
    /// Request timeout in seconds (typically 10).
    pub request_timeout_seconds: u64,
}

impl Default for LlmRegimeConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "openrouter/google/gemini-3.1-flash-lite".to_string(),
            api_base: "https://openrouter.ai/api/v1/chat/completions".to_string(),
            cache_ttl_seconds: 3600,
            request_timeout_seconds: 10,
        }
    }
}

impl LlmRegimeConfig {
    /// Load from environment variables. All optional — if `api_key` is empty,
    /// the task runs as a no-op (always 0.5 neutral).
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("OPENROUTER_API_KEY") {
            cfg.api_key = v.trim().to_string();
        }
        if let Ok(v) = std::env::var("LLM_MODEL") {
            if !v.trim().is_empty() {
                cfg.model = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("LLM_API_BASE") {
            if !v.trim().is_empty() {
                cfg.api_base = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("LLM_CACHE_TTL") {
            if let Ok(n) = v.trim().parse::<u64>() {
                cfg.cache_ttl_seconds = n;
            }
        }
        if let Ok(v) = std::env::var("LLM_TIMEOUT") {
            if let Ok(n) = v.trim().parse::<u64>() {
                cfg.request_timeout_seconds = n;
            }
        }
        cfg
    }

    /// Returns true if the API key is set (task will make real calls).
    pub fn is_enabled(&self) -> bool {
        !self.api_key.is_empty()
    }
}

/// Spawn the hourly regime-cache task. Runs in the background; never blocks.
///
/// If `api_key` is empty, the task is a no-op (stays at 0.5 neutral) and logs
/// a warning on startup. The per-bar pipeline still works — it just reads 0.5.
pub async fn spawn_regime_cache_task(cfg: LlmRegimeConfig) {
    if !cfg.is_enabled() {
        warn!("OPENROUTER_API_KEY not set — LLM regime feature disabled (using 0.5 neutral)");
        return;
    }

    info!(
        model = %cfg.model,
        ttl = cfg.cache_ttl_seconds,
        "spawning LLM regime cache task"
    );

    let ttl = Duration::from_secs(cfg.cache_ttl_seconds.max(60));
    tokio::spawn(async move {
        // First call immediately (don't wait for the first tick).
        run_one_update(&cfg).await;

        let mut interval = tokio::time::interval(ttl);
        loop {
            interval.tick().await;
            run_one_update(&cfg).await;
        }
    });
}

/// Make a single LLM call, parse the response, and update the cache.
async fn run_one_update(cfg: &LlmRegimeConfig) {
    match fetch_regime_prob(cfg).await {
        Ok(prob) => {
            info!(bull_prob = prob, "LLM regime updated");
            write_cached_bull_prob(prob);
        }
        Err(e) => {
            warn!(error = %e, "LLM regime fetch failed — keeping cached value");
        }
    }
}

/// Call the OpenRouter chat completions endpoint and parse a bull probability.
///
/// The prompt asks the model to classify BTC's near-term regime and respond
/// with a single number. We parse the first float we find in [0,1].
async fn fetch_regime_prob(cfg: &LlmRegimeConfig) -> Result<f64> {
    let prompt = "You are a crypto market regime classifier. Given recent BTC price action, \
    classify the likely regime for the next 24 hours. Respond with ONLY a single number \
    from 0.0 (strongly bearish) to 1.0 (strongly bullish). 0.5 means neutral. \
    Do not include any other text.";

    let payload = json!({
        "model": cfg.model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 10,
        "temperature": 0.3,
    });

    let timeout = Duration::from_secs(cfg.request_timeout_seconds.max(3));
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .context("building reqwest client for LLM")?;

    let resp = client
        .post(&cfg.api_base)
        .header("Authorization", format!("Bearer {}", cfg.api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .context("LLM API request failed")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
 anyhow::bail!("LLM API returned {status}: {body}");
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .context("parsing LLM response JSON")?;

    // Extract the assistant's message content.
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();

    if content.is_empty() {
 anyhow::bail!("LLM response has empty content");
    }

    // Parse the first float in [0,1] from the response.
    parse_bull_prob(&content)
}

/// Parse a bull probability from the LLM response text.
///
/// Tries to find a float in [0,1]. If the value is >1.0, interprets it as a
/// percentage (e.g. 70 → 0.7). Returns 0.5 (neutral) on failure.
fn parse_bull_prob(text: &str) -> Result<f64> {
    // Find the first number-like substring.
    for token in text.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| !c.is_ascii_digit() && c != '.' && c != '-');
        if let Ok(v) = cleaned.parse::<f64>() {
            let prob = if v > 1.0 { v / 100.0 } else { v };
            if (0.0..=1.0).contains(&prob) {
                return Ok(prob);
            }
        }
    }
    // Fallback: scan for keywords.
    let lower = text.to_lowercase();
    if lower.contains("bull") || lower.contains("long") || lower.contains("up") {
        return Ok(0.7);
    }
    if lower.contains("bear") || lower.contains("short") || lower.contains("down") {
        return Ok(0.3);
    }
    if lower.contains("neutral") || lower.contains("side") {
        return Ok(0.5);
    }

    anyhow::bail!("could not parse bull probability from: {text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_float() {
        assert!((parse_bull_prob("0.75").unwrap() - 0.75).abs() < 1e-9);
        assert!((parse_bull_prob("0.3").unwrap() - 0.3).abs() < 1e-9);
        assert!((parse_bull_prob("0.5").unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn parse_percentage() {
        assert!((parse_bull_prob("70").unwrap() - 0.7).abs() < 1e-9);
        assert!((parse_bull_prob("30").unwrap() - 0.3).abs() < 1e-9);
    }

    #[test]
    fn parse_with_text() {
        assert!((parse_bull_prob("Bullish 0.8").unwrap() - 0.8).abs() < 1e-9);
        assert!((parse_bull_prob("Likely: 0.6").unwrap() - 0.6).abs() < 1e-9);
    }

    #[test]
    fn parse_keyword_fallback() {
        assert!((parse_bull_prob("bullish").unwrap() - 0.7).abs() < 1e-9);
        assert!((parse_bull_prob("bearish").unwrap() - 0.3).abs() < 1e-9);
        assert!((parse_bull_prob("neutral market").unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn parse_garbage_fails() {
        assert!(parse_bull_prob("hello world").is_err());
    }

    #[test]
    fn read_write_cache() {
        write_cached_bull_prob(0.8);
        assert!((read_cached_bull_prob() - 0.8).abs() < 1e-9);
        write_cached_bull_prob(1.5); // clamps to 1.0
        assert!((read_cached_bull_prob() - 1.0).abs() < 1e-9);
        write_cached_bull_prob(-0.5); // clamps to 0.0
        assert!((read_cached_bull_prob() - 0.0).abs() < 1e-9);
        // Reset to neutral for other tests.
        write_cached_bull_prob(0.5);
    }

    #[test]
    fn config_defaults_sensible() {
        let cfg = LlmRegimeConfig::default();
        assert!(!cfg.is_enabled()); // no API key by default
        assert_eq!(cfg.cache_ttl_seconds, 3600);
        assert!(cfg.api_base.contains("openrouter.ai"));
    }

    #[test]
    fn config_from_env_disabled_without_key() {
        std::env::remove_var("OPENROUTER_API_KEY");
        let cfg = LlmRegimeConfig::from_env();
        assert!(!cfg.is_enabled());
    }
}
