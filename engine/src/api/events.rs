//! Events API — searchable feed of engine events (db::insert_event / search_events).
//!
//! GET /api/events?category=&mode=&severity=&equity=&since=&limit=
//!
//! All filters optional. Searchable by category (trade | data | system |
//! strategy | alert | advisor) and mode (paper | live), per the Events tab
//! requirement. Results newest-first.

use axum::{extract::State, http::StatusCode, response::Json};
use serde::Serialize;

use crate::api::AppState;
use crate::db;

#[derive(Serialize)]
pub struct EventResponse {
    pub id: i64,
    pub ts: i64,
    pub ts_rfc3339: String,
    pub category: String,
    pub severity: String,
    pub mode: String,
    pub source: String,
    pub message: String,
    pub payload: serde_json::Value,
    pub equity: Option<String>,
}

#[derive(Serialize)]
pub struct EventsListResponse {
    pub events: Vec<EventResponse>,
    pub count: usize,
}

/// GET /api/events
pub async fn handle_events(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<EventsListResponse>, StatusCode> {
    let limit: i64 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
        .clamp(1, 1000);

    let events = db::search_events(
        &state.pool,
        params.get("category").map(String::as_str),
        params.get("mode").map(String::as_str),
        params.get("severity").map(String::as_str),
        params.get("equity").map(String::as_str),
        params.get("since").and_then(|v| v.parse().ok()),
        limit,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "failed to search events");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let events: Vec<EventResponse> = events
        .into_iter()
        .map(|ev| EventResponse {
            id: ev.id,
            ts: ev.ts,
            ts_rfc3339: chrono::DateTime::from_timestamp(ev.ts, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default(),
            category: ev.category,
            severity: ev.severity,
            mode: ev.mode,
            source: ev.source,
            message: ev.message,
            payload: serde_json::from_str(&ev.payload_json).unwrap_or(serde_json::json!({})),
            equity: ev.equity,
        })
        .collect();

    Ok(Json(EventsListResponse {
        count: events.len(),
        events,
    }))
}

/// GET /api/events/archive — placeholder until archive API is implemented.
pub async fn handle_archives() -> &'static str {
    "event archive API not yet implemented"
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn events_pool() -> db::DbPool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS engine_events (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                ts           INTEGER NOT NULL,
                category     TEXT    NOT NULL,
                severity     TEXT    NOT NULL,
                mode         TEXT    NOT NULL,
                source       TEXT    NOT NULL,
                message      TEXT    NOT NULL,
                payload_json TEXT    NOT NULL DEFAULT '{}',
                equity       TEXT
            );
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    fn events_state(pool: db::DbPool) -> State<AppState> {
        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        let pool_clone = pool.clone();
        let event_logger = std::sync::Arc::new(crate::event::EventLogger::new(
            pool.clone(),
            None,
            std::sync::Arc::new(tokio::sync::RwLock::new(crate::config::TradingMode::Paper)),
        ));
        State(AppState {
            pool: pool_clone,
            trading_mode: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::config::TradingMode::Paper,
            )),
            strategy_params: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::strategy::EquityStrategyParams::default(),
            )),
            symbol: "QQQ".into(),
            tx,
            parity_marker_path: String::new(),
            parity_max_age_secs: 300,
            totp_secret: String::new(),
            zmq_endpoint: String::new(),
            norm_stats_path: String::new(),
            promotion_gates: crate::config::PromotionGatesConfig::default(),
            short_symbol: "PSQ".into(),
            event_logger: std::sync::Arc::new(crate::event::EventLogger::new(
                pool.clone(),
                None,
                std::sync::Arc::new(tokio::sync::RwLock::new(crate::config::TradingMode::Paper)),
            )),
            advisor: None,
            strategy_params_by_model: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
        })
    }

    async fn insert(pool: &db::DbPool, category: &str, mode: &str, message: &str) {
        db::insert_event(pool, category, "info", mode, "test::source", message, "{}", Some("QQQ"))
            .await
            .unwrap();
    }

    async fn get_events(
        state: State<AppState>,
        query: std::collections::HashMap<String, String>,
    ) -> EventsListResponse {
        let Json(resp) = handle_events(state, axum::extract::Query(query))
            .await
            .unwrap();
        resp
    }

    #[tokio::test]
    async fn test_events_searchable_by_category_and_mode() {
        let pool = events_pool().await;
        insert(&pool, "trade", "paper", "ENTRY_INITIATED QQQ").await;
        insert(&pool, "strategy", "paper", "SKIPPED_ENTRY QQQ").await;
        insert(&pool, "trade", "live", "ENTRY_INITIATED QQQ").await;

        let state = events_state(pool);

        // No filters → all 3
        let resp = get_events(state.clone(), Default::default()).await;
        assert_eq!(resp.count, 3);

        // category=trade → 2
        let mut q = std::collections::HashMap::new();
        q.insert("category".into(), "trade".into());
        let resp = get_events(state.clone(), q).await;
        assert_eq!(resp.count, 2);

        // category=trade&mode=live → 1
        let mut q = std::collections::HashMap::new();
        q.insert("category".into(), "trade".into());
        q.insert("mode".into(), "live".into());
        let resp = get_events(state.clone(), q).await;
        assert_eq!(resp.count, 1);
        assert_eq!(resp.events[0].mode, "live");
        assert_eq!(resp.events[0].category, "trade");

        // equity + category filter
        let mut q = std::collections::HashMap::new();
        q.insert("equity".into(), "QQQ".into());
        q.insert("category".into(), "strategy".into());
        let resp = get_events(state.clone(), q).await;
        assert_eq!(resp.count, 1);
        assert!(resp.events[0].message.contains("SKIPPED_ENTRY"));

        // limit respected
        let mut q = std::collections::HashMap::new();
        q.insert("limit".into(), "1".into());
        let resp = get_events(state.clone(), q).await;
        assert_eq!(resp.count, 1);
    }
}
