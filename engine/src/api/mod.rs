use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing::error;

use crate::{config::Config, db};

pub(crate) type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

#[derive(Clone)]
pub(crate) struct AppState {
    pub pool: db::DbPool,
    pub trading_mode: crate::config::TradingMode,
    pub symbol: String,
    pub sma_window: usize,
    pub tx: ws::TelemetrySender,
}

pub fn router(pool: db::DbPool, config: &Config, tx: ws::TelemetrySender) -> Router {
    let state = AppState {
        pool,
        trading_mode: config.trading_mode,
        symbol: config.symbol.clone(),
        sma_window: config.sma_window,
        tx,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/api/status", get(status::handle_status))
        .route("/api/market_state", get(status::handle_market_state))
        .route("/api/predictions", get(predictions::handle_predictions))
        .route("/api/accuracy", get(predictions::handle_accuracy))
        .route("/api/chart", get(chart::handle_chart))
        .route("/api/equity/data", get(equity::handle_equity_data))
        .route("/api/equity/backfill", get(equity::handle_equity_backfill))
        .route("/api/equity/macro", get(equity::handle_equity_macro))
        .route("/api/equity/features", get(equity::handle_equity_features))
        .route("/api/backtest", post(backtest::handle_backtest))
        .route("/api/strategies", get(crate::strategy_lab::api::handle_list_strategies))
        .route("/api/strategies", post(crate::strategy_lab::api::handle_save_strategy))
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

mod status;
mod predictions;
mod chart;
mod equity;
mod backtest;
pub(crate) mod ws;

#[cfg(test)]
mod tests;
