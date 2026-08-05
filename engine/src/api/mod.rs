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
    /// Short instrument symbol (e.g. "PSQ") used to express short positions.
    pub short_symbol: String,
    pub tx: ws::TelemetrySender,
    /// Unified event logger for trades, data fetches, system events.
    pub event_logger: std::sync::Arc<crate::event::EventLogger>,
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
    /// Phase 4: AI advisor state. None if advisor is disabled.
    pub advisor: Option<std::sync::Arc<crate::advisor::AdvisorState>>,
}

pub fn router(
    pool: db::DbPool,
    config: &Config,
    tx: ws::TelemetrySender,
    advisor: Option<std::sync::Arc<crate::advisor::AdvisorState>>,
    event_logger: std::sync::Arc<crate::event::EventLogger>,
) -> Router {
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
        short_symbol: config.short_symbol.clone(),
        tx,
        event_logger,
        parity_marker_path: config.parity_marker_path.clone(),
        parity_max_age_secs: config.parity_max_age_secs,
        totp_secret: config.totp_secret.clone(),
        zmq_endpoint: config.zmq_endpoint.clone(),
        norm_stats_path: config.norm_stats_path.clone(),
        advisor,
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
        .route("/api/mode", get(mode::handle_get_mode))
        .route("/api/mode", post(mode::handle_set_mode))
        .route("/api/v1/ws", get(ws::ws_handler))
        .route("/api/advisor/briefing", get(advisor::handle_get_briefing))
        .route("/api/advisor/ask", post(advisor::handle_ask))
        .route("/api/advisor/refresh", post(advisor::handle_refresh))
        .route("/api/sentiment/history", get(advisor::handle_sentiment_history))
        .route("/api/events", get(events::handle_events))
        .route("/api/events/archive", get(events::handle_archives))
        .route("/api/models", get(models::handle_list_models))
        .route("/api/models", post(models::handle_register_model))
        .route("/api/models/:id/enabled", put(models::handle_set_enabled))
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
mod quote;
mod strategy_config;
mod advisor;
mod events;
mod models;

#[cfg(test)]
mod tests;
