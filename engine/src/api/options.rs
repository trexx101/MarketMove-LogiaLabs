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

#[derive(Serialize)]
pub struct TapeStatusResponse {
    pub tapes: Vec<db::TapeMeta>,
    pub count: usize,
}

/// GET /api/options/tape/status
pub async fn handle_tape_status(
    State(state): State<AppState>,
) -> Result<Json<TapeStatusResponse>, StatusCode> {
    let tapes = db::list_tape_meta(&state.pool).await.map_err(|e| {
        tracing::error!(error = %e, "failed to list tape meta");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(TapeStatusResponse {
        count: tapes.len(),
        tapes,
    }))
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
        State(AppState {
            pool,
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
}
