use axum::{extract::State, http::StatusCode, response::Json};

use crate::strategy_lab;

use super::{ApiResult, AppState};

/// Request body for `POST /api/backtest`.
#[derive(serde::Deserialize)]
pub(crate) struct BacktestPayload {
    #[serde(default)]
    pub strategy_id: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub params: serde_json::Value,
    pub start_ts: i64,
    pub end_ts: i64,
}

/// `POST /api/backtest`
///
/// Runs a backtest over the specified time window and returns the equity curve,
/// performance metrics, and trade log.
pub(crate) async fn handle_backtest(
    State(state): State<AppState>,
    axum::extract::Json(payload): axum::extract::Json<BacktestPayload>,
) -> ApiResult<strategy_lab::BacktestResult> {
    let request = strategy_lab::BacktestRequest {
        strategy_id: payload.strategy_id,
        kind: payload.kind,
        params: payload.params,
        start_ts: payload.start_ts,
        end_ts: payload.end_ts,
    };

    let result = strategy_lab::replay::run_backtest(&state.pool, &state.symbol, &request)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "backtest failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("backtest error: {e:#}"),
            )
        })?;

    Ok(Json(result))
}