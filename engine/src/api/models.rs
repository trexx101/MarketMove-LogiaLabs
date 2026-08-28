//! Models API — registry CRUD for trading models (§8).
//!
//! `GET    /api/models`           — list all registered models
//! `POST   /api/models`           — register a new model
//! `PUT    /api/models/:id/enabled` — toggle the enabled flag

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Deserialize;

use crate::db;

use super::{internal_error, ApiResult, AppState};

/// `GET /api/models` — return every registered model, newest-first.
pub async fn handle_list_models(State(state): State<AppState>) -> ApiResult<Vec<db::TradingModel>> {
    let models = db::load_all_models(&state.pool)
        .await
        .map_err(|e| internal_error("list_models", e))?;
    Ok(Json(models))
}

/// `POST /api/models` — register a new trading model.
///
/// The model is inserted with `enabled = 1` by default. Call
/// `PUT /api/models/:id/enabled` to disable it.
#[derive(Debug, Deserialize)]
pub struct RegisterModelBody {
    pub model_id: String,
    pub primary_symbol: String,
    pub inverse_symbol: String,
    pub model_path: String,
    pub norm_stats_path: String,
    pub budget_usd: f64,
    #[serde(default = "default_deploy_pct")]
    pub deploy_pct: f64,
    pub notes: Option<String>,
}

fn default_deploy_pct() -> f64 {
    0.25
}

pub async fn handle_register_model(
    State(state): State<AppState>,
    Json(body): Json<RegisterModelBody>,
) -> Result<(StatusCode, Json<db::TradingModel>), (StatusCode, String)> {
    let model = db::register_model(
        &state.pool,
        &body.model_id,
        &body.primary_symbol,
        &body.inverse_symbol,
        &body.model_path,
        &body.norm_stats_path,
        body.budget_usd,
        body.deploy_pct,
        body.notes.as_deref(),
    )
    .await
    .map_err(|e| internal_error("register_model", e))?;
    Ok((StatusCode::CREATED, Json(model)))
}

/// `PUT /api/models/:id/enabled` — toggle the enabled flag.
///
/// Body: `{"enabled": true|false}`
#[derive(Debug, Deserialize)]
pub struct SetEnabledBody {
    pub enabled: bool,
}

pub async fn handle_set_enabled(
    State(state): State<AppState>,
    Path(model_id): Path<String>,
    Json(body): Json<SetEnabledBody>,
) -> ApiResult<db::TradingModel> {
    let model = db::update_model_enabled(&state.pool, &model_id, body.enabled)
        .await
        .map_err(|e| internal_error("set_enabled", e))?
        .ok_or((StatusCode::NOT_FOUND, format!("model not found: {model_id}")))?;
    Ok(Json(model))
}
