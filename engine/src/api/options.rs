//! Options API endpoints — positions, trade history, config, hyperopt runs, tape status.
//!
//! GET  /api/options/positions?underlying=&status=&limit=
//! GET  /api/options/trades?underlying=&limit=          — closed positions (trade history)
//! GET  /api/options/config                              — full registry with values
//! PUT  /api/options/config                              — { "key": value } bulk write
//! GET  /api/hyperopt/runs?limit=
//! GET  /api/options/tape/status

use axum::{extract::State, http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};

use crate::api::AppState;
use crate::db;
use crate::options::config_store::{registry, OptionsConfigStore as ConfigStore};

// ── Positions ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct PositionsResponse {
    pub positions: Vec<db::OptionPosition>,
    pub count: usize,
}

/// GET /api/options/positions
pub async fn handle_list_positions(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<PositionsResponse>, StatusCode> {
    let limit: i64 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
        .clamp(1, 1000);

    let positions = db::list_option_positions(
        &state.pool,
        params.get("underlying").map(String::as_str),
        params.get("status").map(String::as_str),
        limit,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "failed to list option positions");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(PositionsResponse {
        count: positions.len(),
        positions,
    }))
}

// ── Trade history ────────────────────────────────────────────────────────────
// Trade history = CLOSED positions (linked open/closed lifecycle: a position
// row carries entry_premium at open and realized_pnl/closed_at at close).

#[derive(Serialize)]
pub struct TradesResponse {
    pub trades: Vec<db::OptionPosition>,
    pub count: usize,
}

/// GET /api/options/trades
pub async fn handle_list_trades(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<TradesResponse>, StatusCode> {
    let limit: i64 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
        .clamp(1, 1000);

    let trades = db::list_option_positions(
        &state.pool,
        params.get("underlying").map(String::as_str),
        Some("CLOSED"),
        limit,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "failed to list option trades");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(TradesResponse {
        count: trades.len(),
        trades,
    }))
}

// ── Config ───────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ConfigResponse {
    pub entries: Vec<crate::options::config_store::ConfigEntry>,
    pub count: usize,
}

/// GET /api/options/config
pub async fn handle_get_config(
    State(state): State<AppState>,
) -> Result<Json<ConfigResponse>, StatusCode> {
    let mode = state.trading_mode.read().await.to_string();
    let store = ConfigStore::new(state.pool.clone(), &mode);
    let entries = store.list().await.map_err(|e| {
        tracing::error!(error = %e, "failed to list options config");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(ConfigResponse {
        count: entries.len(),
        entries,
    }))
}

#[derive(Deserialize)]
pub struct ConfigPutRequest {
    /// key → value map. Bulk write; each key validated against the registry.
    #[serde(flatten)]
    pub values: std::collections::HashMap<String, f64>,
}

#[derive(Debug, Serialize)]
pub struct ConfigPutResponse {
    pub applied: usize,
    pub rejected: Vec<String>,
}

/// PUT /api/options/config
pub async fn handle_put_config(
    State(state): State<AppState>,
    Json(req): Json<ConfigPutRequest>,
) -> Result<Json<ConfigPutResponse>, StatusCode> {
    let mode = state.trading_mode.read().await.to_string();
    let store = ConfigStore::new(state.pool.clone(), &mode);
    let known_keys: std::collections::HashSet<String> =
        registry().into_iter().map(|s| s.key.to_string()).collect();

    let mut applied = 0;
    let mut rejected = Vec::new();

    for (key, value) in req.values {
        if !known_keys.contains(&key) {
            rejected.push(format!("{key}: unknown key"));
            continue;
        }
        match store.set(&key, value).await {
            Ok(()) => applied += 1,
            Err(e) => rejected.push(format!("{key}: {e}")),
        }
    }

    if applied == 0 && !rejected.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    Ok(Json(ConfigPutResponse { applied, rejected }))
}

// ── Hyperopt runs ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct RunsResponse {
    pub runs: Vec<db::HyperoptRun>,
    pub count: usize,
}

/// GET /api/hyperopt/runs
pub async fn handle_list_runs(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<RunsResponse>, StatusCode> {
    let limit: i64 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20)
        .clamp(1, 200);

    let runs = db::list_hyperopt_runs(&state.pool, limit).await.map_err(|e| {
        tracing::error!(error = %e, "failed to list hyperopt runs");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(RunsResponse {
        count: runs.len(),
        runs,
    }))
}

// ── Tape status ──────────────────────────────────────────────────────────────

/// A heartbeat older than this is considered stale (recorder is supposed to
/// beat on every healthy tick; Phase 0 recorder beats at least once/minute).
const HEARTBEAT_STALE_AFTER_SECS: i64 = 120;

#[derive(Serialize)]
pub struct TapeStatusEntry {
    #[serde(flatten)]
    pub meta: db::TapeMeta,
    /// Seconds since last heartbeat; None if the recorder never beat.
    pub heartbeat_age_secs: Option<i64>,
    pub heartbeat_stale: bool,
}

#[derive(Serialize)]
pub struct TapeStatusResponse {
    pub tapes: Vec<TapeStatusEntry>,
    pub count: usize,
    pub healthy: usize,
    pub stale: usize,
    pub never_beat: usize,
}

/// GET /api/options/tape/status
pub async fn handle_tape_status(
    State(state): State<AppState>,
) -> Result<Json<TapeStatusResponse>, StatusCode> {
    let tapes = db::list_tape_meta(&state.pool).await.map_err(|e| {
        tracing::error!(error = %e, "failed to list tape meta");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let now = chrono::Utc::now().timestamp_millis();
    let entries: Vec<TapeStatusEntry> = tapes
        .into_iter()
        .map(|meta| {
            let age = meta.last_heartbeat_ts.map(|ts| (now - ts) / 1000);
            let stale = age.map(|a| a > HEARTBEAT_STALE_AFTER_SECS).unwrap_or(true);
            TapeStatusEntry {
                meta,
                heartbeat_age_secs: age,
                heartbeat_stale: stale,
            }
        })
        .collect();

    let never_beat = entries.iter().filter(|e| e.meta.last_heartbeat_ts.is_none()).count();
    let stale = entries.iter().filter(|e| e.heartbeat_stale).count();
    let healthy = entries.len() - stale;

    Ok(Json(TapeStatusResponse {
        count: entries.len(),
        healthy,
        stale,
        never_beat,
        tapes: entries,
    }))
}

// ── Internal heartbeat endpoint (called by the recorder process) ──────────

#[derive(Deserialize)]
pub struct HeartbeatRequest {
    pub tape_id: String,
    pub underlying: String,
    pub chain_code: String,
    pub quota_accounting_json: String,
}

/// POST /api/internal/tape/heartbeat
///
/// Fire-and-forget heartbeat from the tape recorder process. The engine is
/// the sole writer to the database; the recorder never touches SQLite
/// directly. This avoids SQLITE_BUSY locks between the host process and
/// the Dockerized engine.
pub async fn handle_tape_heartbeat(
    State(state): State<AppState>,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    db::touch_tape_heartbeat(
        &state.pool,
        &req.tape_id,
        &req.underlying,
        &req.chain_code,
        &req.quota_accounting_json,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "tape heartbeat insert failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(serde_json::json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn options_pool() -> db::DbPool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for stmt in db::DDL.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query(stmt).execute(&pool).await.unwrap();
        }
        db::migrate_option_positions(&pool).await.unwrap();
        pool
    }

    fn options_state(pool: db::DbPool) -> State<AppState> {
        let (tx, _rx) = tokio::sync::broadcast::channel(16);
        let event_logger = std::sync::Arc::new(crate::event::EventLogger::new(
            pool.clone(),
            None,
            std::sync::Arc::new(tokio::sync::RwLock::new(crate::config::TradingMode::Paper)),
        ));
        State(AppState {
            pool: pool.clone(),
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

    async fn insert_position(pool: &db::DbPool, id: &str, underlying: &str, status: &str, pnl: Option<f64>) {
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO option_positions
             (id, underlying, contract_code, strategy_version_id, entry_underlying_price,
              entry_premium, entry_spread, entry_slippage_budget, qty, status,
              dte_at_entry, delta_at_entry, realized_pnl, closed_at, created_at, updated_at)
             VALUES (?1, ?2, 'C', 'v1', 450.0, 3.5, 0.05, 0.10, 1, ?3, 40, 0.45, ?4, NULL, ?5, ?5)",
        )
        .bind(id)
        .bind(underlying)
        .bind(status)
        .bind(pnl)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_positions_filtered_by_status() {
        let pool = options_pool().await;
        insert_position(&pool, "p1", "QQQ", "OPEN", None).await;
        insert_position(&pool, "p2", "QQQ", "CLOSED", Some(120.5)).await;
        insert_position(&pool, "p3", "SMH", "OPEN", None).await;

        let state = options_state(pool);

        // All positions
        let Json(resp) = handle_list_positions(state.clone(), axum::extract::Query(Default::default()))
            .await
            .unwrap();
        assert_eq!(resp.count, 3);

        // status=OPEN filter
        let mut q = std::collections::HashMap::new();
        q.insert("status".into(), "OPEN".into());
        let Json(resp) = handle_list_positions(state.clone(), axum::extract::Query(q)).await.unwrap();
        assert_eq!(resp.count, 2);

        // underlying filter
        let mut q = std::collections::HashMap::new();
        q.insert("underlying".into(), "SMH".into());
        let Json(resp) = handle_list_positions(state.clone(), axum::extract::Query(q)).await.unwrap();
        assert_eq!(resp.count, 1);
        assert_eq!(resp.positions[0].underlying, "SMH");
    }

    #[tokio::test]
    async fn test_trades_returns_only_closed_with_pnl() {
        let pool = options_pool().await;
        insert_position(&pool, "p1", "QQQ", "OPEN", None).await;
        insert_position(&pool, "p2", "QQQ", "CLOSED", Some(120.5)).await;

        let state = options_state(pool);

        let Json(resp) = handle_list_trades(state, axum::extract::Query(Default::default()))
            .await
            .unwrap();
        assert_eq!(resp.count, 1);
        assert_eq!(resp.trades[0].id, "p2");
        assert_eq!(resp.trades[0].realized_pnl, Some(120.5));
        assert_eq!(resp.trades[0].entry_premium, 3.5);
    }

    #[tokio::test]
    async fn test_config_get_returns_registry_with_defaults() {
        let pool = options_pool().await;
        let state = options_state(pool);

        let Json(resp) = handle_get_config(state).await.unwrap();
        assert!(resp.count >= 30, "registry should expose all 37 keys, got {}", resp.count);
        // Spot-check a known key carries its default value
        let risk = resp.entries.iter().find(|e| e.key == "risk_pct").unwrap();
        assert_eq!(risk.value, risk.default);
    }

    #[tokio::test]
    async fn test_config_put_applies_and_rejects() {
        let pool = options_pool().await;
        let state = options_state(pool.clone());

        let req = ConfigPutRequest {
            values: [
                ("risk_pct".to_string(), 0.02),
                ("bogus_key".to_string(), 1.0),
            ]
            .into_iter()
            .collect(),
        };
        let Json(resp) = handle_put_config(state.clone(), Json(req)).await.unwrap();
        assert_eq!(resp.applied, 1);
        assert_eq!(resp.rejected.len(), 1);

        // Value persisted
        let Json(get_resp) = handle_get_config(state.clone()).await.unwrap();
        let risk = get_resp.entries.iter().find(|e| e.key == "risk_pct").unwrap();
        assert_eq!(risk.value, 0.02);
    }

    #[tokio::test]
    async fn test_config_put_all_rejected_is_400() {
        let pool = options_pool().await;
        let state = options_state(pool);

        let req = ConfigPutRequest {
            values: [("nope".to_string(), 1.0)].into_iter().collect(),
        };
        let err = handle_put_config(state, Json(req)).await.unwrap_err();
        assert_eq!(err, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_hyperopt_runs_listed_newest_first() {
        let pool = options_pool().await;
        db::insert_hyperopt_run(&pool).await.unwrap();
        let state = options_state(pool);

        let Json(resp) = handle_list_runs(state, axum::extract::Query(Default::default()))
            .await
            .unwrap();
        assert_eq!(resp.count, 1);
        assert_eq!(resp.runs[0].status, "RUNNING");
    }

    #[tokio::test]
    async fn test_tape_status_empty_ok() {
        let pool = options_pool().await;
        let state = options_state(pool);
        let Json(resp) = handle_tape_status(state).await.unwrap();
        assert_eq!(resp.count, 0);
    }

    #[tokio::test]
    async fn test_heartbeat_touch_and_staleness() {
        let pool = options_pool().await;

        // Touch creates the row with a fresh heartbeat
        db::touch_tape_heartbeat(&pool, "tape-1", "QQQ", "QQQ250919P00450000", r#"{"used": 3}"#)
            .await
            .unwrap();
        let metas = db::list_tape_meta(&pool).await.unwrap();
        assert_eq!(metas.len(), 1);
        assert!(metas[0].last_heartbeat_ts.is_some());
        assert_eq!(metas[0].quota_accounting_json, r#"{"used": 3}"#);

        // Touch again updates in place (no duplicate row)
        db::touch_tape_heartbeat(&pool, "tape-1", "QQQ", "QQQ250919P00450000", r#"{"used": 4}"#)
            .await
            .unwrap();
        assert_eq!(db::list_tape_meta(&pool).await.unwrap().len(), 1);

        let state = options_state(pool.clone());
        let Json(resp) = handle_tape_status(state).await.unwrap();
        assert_eq!(resp.count, 1);
        assert_eq!(resp.healthy, 1);
        assert_eq!(resp.stale, 0);
        assert_eq!(resp.never_beat, 0);
        assert!(resp.tapes[0].heartbeat_age_secs.unwrap() <= 1);

        // Age the heartbeat past the stale threshold directly
        let stale_ts = chrono::Utc::now().timestamp_millis() - 300_000; // 5 min ago
        sqlx::query("UPDATE option_tape_meta SET last_heartbeat_ts = ?1 WHERE id = 'tape-1'")
            .bind(stale_ts)
            .execute(&pool)
            .await
            .unwrap();

        let state = options_state(pool);
        let Json(resp) = handle_tape_status(state).await.unwrap();
        assert_eq!(resp.stale, 1);
        assert_eq!(resp.healthy, 0);
        assert!(resp.tapes[0].heartbeat_stale);
    }

    #[tokio::test]
    async fn test_never_beat_counts_as_stale() {
        let pool = options_pool().await;
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO option_tape_meta (id, underlying, chain_code, quota_accounting_json, created_at)
             VALUES ('tape-x', 'SMH', 'C', '{}', ?1)",
        )
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

        let state = options_state(pool);
        let Json(resp) = handle_tape_status(state).await.unwrap();
        assert_eq!(resp.count, 1);
        assert_eq!(resp.never_beat, 1);
        assert_eq!(resp.stale, 1);
    }

    #[tokio::test]
    async fn test_migrate_option_tape_meta_idempotent() {
        // Pre-existing table WITHOUT the heartbeat column (the live-db case)
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE option_tape_meta (
                id TEXT PRIMARY KEY, underlying TEXT NOT NULL, chain_code TEXT NOT NULL,
                quota_accounting_json TEXT NOT NULL DEFAULT '{}', created_at INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();

        db::migrate_option_tape_meta(&pool).await.unwrap();
        db::migrate_option_tape_meta(&pool).await.unwrap(); // second run must be a no-op

        db::touch_tape_heartbeat(&pool, "t", "QQQ", "C", "{}").await.unwrap();
        assert!(db::list_tape_meta(&pool).await.unwrap()[0].last_heartbeat_ts.is_some());
    }
}
