use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post, put},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing::error;

use crate::{config::Config, db};

use crate::strategy::EquityStrategyParams;

pub(crate) type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

#[derive(Clone)]
pub struct AppState {
    pub pool: db::DbPool,
    /// Current trading mode. Wrapped in `Arc<RwLock>` so the runtime toggle
    /// endpoint can flip it while the scheduler is reading it.
    pub trading_mode: std::sync::Arc<tokio::sync::RwLock<crate::config::TradingMode>>,
    /// Shared strategy params — mutable at runtime via /api/strategy-config.
    pub strategy_params: std::sync::Arc<tokio::sync::RwLock<EquityStrategyParams>>,
    pub symbol: String,
    pub tx: ws::TelemetrySender,
    /// Path to the parity marker JSON (re-checked at request time by /api/mode).
    pub parity_marker_path: String,
    /// Maximum age in seconds before the parity marker is considered stale.
    pub parity_max_age_secs: i64,
    /// Base32 TOTP secret used by /api/mode to authorize live-mode flips.
    pub totp_secret: String,
    /// ZMQ endpoint for the inference service (e.g. `tcp://127.0.0.1:5555`).
    pub zmq_endpoint: String,
    /// Path to the norm stats JSON file for feature normalization.
    pub norm_stats_path: String,
}

pub fn router(pool: db::DbPool, config: &Config, tx: ws::TelemetrySender) -> Router {
    let trading_mode = std::sync::Arc::new(tokio::sync::RwLock::new(config.trading_mode));
    let strategy_params = std::sync::Arc::new(tokio::sync::RwLock::new(EquityStrategyParams {
        entry_threshold: config.entry_threshold,
        exit_threshold: config.exit_threshold,
        sma_window: config.sma_window,
        enable_shorting: config.enable_shorting,
        short_entry_threshold: config.short_entry_threshold,
        short_exit_threshold: config.short_exit_threshold,
        pred_5d_filter: config.pred_5d_filter,
    }));
    let state = AppState {
        pool,
        trading_mode,
        strategy_params,
        symbol: config.symbol.clone(),
        tx,
        parity_marker_path: config.parity_marker_path.clone(),
        parity_max_age_secs: config.parity_max_age_secs,
        totp_secret: config.totp_secret.clone(),
        zmq_endpoint: config.zmq_endpoint.clone(),
        norm_stats_path: config.norm_stats_path.clone(),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/api/status", get(status::handle_status))
        .route("/api/market_state", get(status::handle_market_state))
        .route("/api/quote", get(quote::handle_quote))
        .route("/api/predictions", get(predictions::handle_predictions))
        .route("/api/accuracy", get(predictions::handle_accuracy))
        .route("/api/chart", get(chart::handle_chart))
        .route("/api/equity/data", get(equity::handle_equity_data))
        .route("/api/equity/backfill", get(equity::handle_equity_backfill))
        .route("/api/equity/backfill_predictions", post(equity::handle_backfill_predictions))
        .route("/api/equity/macro", get(equity::handle_equity_macro))
        .route("/api/equity/features", get(equity::handle_equity_features))
        .route("/api/equity/trades", get(equity::handle_equity_trades))
        .route("/api/backtest", post(backtest::handle_backtest))
        .route("/api/strategies", get(crate::strategy_lab::api::handle_list_strategies))
        .route("/api/strategies", post(crate::strategy_lab::api::handle_save_strategy))
        .route("/api/strategy-config", get(strategy_config::handle_get))
        .route("/api/strategy-config", put(strategy_config::handle_put))
        .route("/api/hyperopt/:equity/candidates", get(hyperopt::list_candidates))
        .route("/api/hyperopt/:equity/candidates/:id", get(hyperopt::get_candidate))
        .route("/api/hyperopt/:equity/promote/:id", post(hyperopt::promote_candidate))
        .route("/api/hyperopt/:equity/status", get(hyperopt::get_status))
        .route("/api/events", get(events::handle_list_events))
        .route("/api/options/positions", get(options::handle_list_positions))
        .route("/api/options/trades", get(options::handle_list_trades))
        .route("/api/options/config", get(options::handle_get_config))
        .route("/api/options/config", put(options::handle_put_config))
        .route("/api/options/tape/status", get(options::handle_tape_status))
        .route("/api/hyperopt/runs", get(options::handle_list_runs))
        .route("/api/mode", get(mode::handle_get_mode))
        .route("/api/mode", post(mode::handle_set_mode))
        .route("/api/v1/ws", get(ws::ws_handler))
        .layer(cors)
        .with_state(state)
        .fallback_service(
            ServeDir::new("frontend")
                .not_found_service(ServeFile::new("frontend/index.html")),
        )
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(crate) fn ts_to_rfc3339(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

pub(crate) fn internal_error(context: &str, err: anyhow::Error) -> (StatusCode, String) {
    error!(error = %err, context, "API handler error");
    (StatusCode::INTERNAL_SERVER_ERROR, format!("{context}: {err:#}"))
}

pub(crate) mod ws;
mod status;
mod predictions;
mod chart;
mod equity;
mod backtest;
pub mod mode;
pub mod hyperopt;
mod quote;
mod strategy_config;
pub mod events;
pub mod options;

#[cfg(test)]
mod tests;
