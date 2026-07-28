use axum::{extract::State, http::StatusCode, response::Json};
use serde::Deserialize;
use tracing::error;
use uuid::Uuid;

use crate::api::{ApiResult, AppState};
use crate::db;

/// `GET /api/strategies` — list all saved strategy configs.
pub(crate) async fn handle_list_strategies(
    State(state): State<AppState>,
) -> ApiResult<Vec<db::StrategyConfigRow>> {
    let rows = db::fetch_strategy_configs(&state.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to list strategies");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("list strategies: {e:#}"),
            )
        })?;
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub(crate) struct SaveStrategyPayload {
    pub name: String,
    pub strategy_type: String,
    /// Rhai script body (only for Rhai strategies).
    #[serde(default)]
    pub script_body: Option<String>,
    /// JSON-encoded params (threshold params or `{ "script": "..." }`).
    pub params_json: String,
}

/// `POST /api/strategies` — persist a strategy config.
pub(crate) async fn handle_save_strategy(
    State(state): State<AppState>,
    axum::extract::Json(payload): axum::extract::Json<SaveStrategyPayload>,
) -> ApiResult<db::StrategyConfigRow> {
    let id = Uuid::new_v4().to_string();

    db::insert_strategy_config(
        &state.pool,
        &id,
        &payload.name,
        &payload.strategy_type,
        payload.script_body.as_deref(),
        &payload.params_json,
    )
    .await
    .map_err(|e| {
        error!(error = %e, "failed to save strategy");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("save strategy: {e:#}"),
        )
    })?;

    // Re-read the freshly inserted row so the caller gets the canonical record.
    let rows = db::fetch_strategy_configs(&state.pool)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to re-read saved strategy");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read strategy: {e:#}"),
            )
        })?;

    let row = rows.into_iter().find(|r| r.id == id).ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "saved strategy not found after insert".to_string(),
        )
    })?;

    Ok(Json(row))
}
