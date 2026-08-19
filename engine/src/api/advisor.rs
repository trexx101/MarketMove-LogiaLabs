//! Advisor API handlers — GET /api/advisor/briefing, POST /api/advisor/ask,
//! GET /api/sentiment/history.
//!
//! Phase 4. All endpoints are additive — they don't touch the execution path.

use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::sse::{Event, Sse},
    Json,
};
use futures::stream::Stream;
use tracing::warn;

use super::AppState;

// ── GET /api/advisor/briefing?date=YYYY-MM-DD ────────────────────────

#[derive(serde::Deserialize)]
pub struct BriefingQuery {
    #[serde(default)]
    pub date: Option<String>,
}

#[derive(serde::Serialize)]
pub struct BriefingResponse {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub briefing: Option<crate::advisor::AdvisorBriefing>,
}

pub async fn handle_get_briefing(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<BriefingQuery>,
) -> (StatusCode, Json<BriefingResponse>) {
    let advisor = match &state.advisor {
        Some(a) => a,
        None => {
            return (
                StatusCode::OK,
                Json(BriefingResponse {
                    enabled: false,
                    reason: Some("advisor is not configured".to_string()),
                    briefing: None,
                }),
            );
        }
    };

    if !advisor.cfg.is_enabled() {
        return (
            StatusCode::OK,
            Json(BriefingResponse {
                enabled: false,
                reason: Some("OPENROUTER_API_KEY not set — advisor is disabled".to_string()),
                briefing: None,
            }),
        );
    }

    let for_date = params.date.unwrap_or_else(|| {
        chrono::Utc::now().format("%Y-%m-%d").to_string()
    });

    match advisor.cached(&for_date) {
        Some(briefing) => (
            StatusCode::OK,
            Json(BriefingResponse {
                enabled: true,
                reason: None,
                briefing: Some(briefing),
            }),
        ),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(BriefingResponse {
                enabled: true,
                reason: Some("briefing not yet generated for this date".to_string()),
                briefing: None,
            }),
        ),
    }
}

// ── POST /api/advisor/ask ────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct AskRequest {
    pub question: String,
    #[serde(default)]
    pub date: Option<String>,
}

pub async fn handle_ask(
    State(state): State<AppState>,
    Json(req): Json<AskRequest>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    use futures::stream;

    let advisor = state.advisor.clone();
    let pool = state.pool.clone();
    let symbol = state.symbol.clone();
    let question = req.question.clone();

    let stream = stream::try_unfold(
        (pool, advisor, symbol, question, false),
        move |(pool, advisor, symbol, question, sent)| {
            async move {
                if sent {
                    return Ok::<_, std::convert::Infallible>(None);
                }

                let advisor = match &advisor {
                    Some(a) => a,
                    None => {
                        let event = Event::default()
                            .data(r#"{"error": "advisor is not configured"}"#)
                            .event("error");
                        return Ok(Some((event, (pool, None, symbol, question, true))));
                    }
                };

                if !advisor.cfg.is_enabled() {
                    let event = Event::default()
                        .data(r#"{"error": "advisor is disabled"}"#)
                        .event("error");
                    return Ok(Some((event, (pool, Some(advisor.clone()), symbol, question, true))));
                }

                if question.trim().is_empty() {
                    let event = Event::default()
                        .data(r#"{"error": "question must not be empty"}"#)
                        .event("error");
                    return Ok(Some((event, (pool, Some(advisor.clone()), symbol, question, true))));
                }

                let history_turns = advisor.cfg.chat_history_turns;
                let history = crate::db::fetch_recent_chat_turns(&pool, history_turns)
                    .await
                    .unwrap_or_default();

                let start = std::time::Instant::now();
                match crate::advisor::chat::generate_chat_response(
                    advisor, &pool, &symbol, &question, &history,
                )
                .await
                {
                    Ok(raw) => {
                        let latency_ms = start.elapsed().as_millis() as i64;
                        let _ = crate::db::insert_advisor_chat_log(
                            &pool, &question, &raw, &advisor.cfg.model, latency_ms,
                        )
                        .await;

                        let event = Event::default()
                            .data(serde_json::json!({
                                "token": raw,
                                "done": true,
                                "response": {
                                    "full_prose": raw,
                                    "warnings": []
                                }
                            }).to_string())
                            .event("message");
                        Ok(Some((event, (pool, Some(advisor.clone()), symbol, question, true))))
                    }
                    Err(e) => {
                        warn!(error = %e, "advisor chat failed");
                        let event = Event::default()
                            .data(serde_json::json!({"error": format!("{e}")}).to_string())
                            .event("error");
                        Ok(Some((event, (pool, Some(advisor.clone()), symbol, question, true))))
                    }
                }
            }
        },
    );

    Sse::new(stream)
}

// ── GET /api/sentiment/history?symbol=QQQ&days=30 ────────────────────

#[derive(serde::Deserialize)]
pub struct SentimentHistoryQuery {
    #[serde(default = "default_symbol")]
    pub symbol: String,
    #[serde(default = "default_days")]
    pub days: u32,
}

fn default_symbol() -> String {
    "QQQ".to_string()
}

fn default_days() -> u32 {
    30
}

#[derive(serde::Serialize)]
pub struct SentimentHistoryRow {
    pub date: String,
    pub score: f64,
    pub source: String,
    pub buzz: i64,
    pub weekly_avg: f64,
}

pub async fn handle_sentiment_history(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<SentimentHistoryQuery>,
) -> Result<Json<Vec<SentimentHistoryRow>>, (StatusCode, String)> {
    let rows = sqlx::query(
        "SELECT date, score, source, buzz, weekly_avg \
         FROM sentiment_cache \
         WHERE symbol = ?1 \
         ORDER BY date DESC \
         LIMIT ?2",
    )
    .bind(&params.symbol)
    .bind(params.days as i64)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("sentiment history query failed: {e}"),
        )
    })?;

    let history: Vec<SentimentHistoryRow> = rows
        .iter()
        .map(|r| {
            use sqlx::Row;
            SentimentHistoryRow {
                date: r.get(0),
                score: r.get(1),
                source: r.get(2),
                buzz: r.get(3),
                weekly_avg: r.get(4),
            }
        })
        .collect();

    Ok(Json(history))
}

// ── POST /api/advisor/refresh ────────────────────────────────────────

pub async fn handle_refresh(
    State(state): State<AppState>,
) -> (StatusCode, Json<serde_json::Value>) {
    match &state.advisor {
        Some(advisor) => {
            advisor.refresh();
            (
                StatusCode::OK,
                Json(serde_json::json!({"ok": true, "message": "refresh triggered"})),
            )
        }
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"ok": false, "error": "advisor is not configured"})),
        ),
    }
}