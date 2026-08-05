//! Runtime strategy config endpoints.
//!
//! - `GET /api/strategy-config` — return current strategy params.
//! - `PUT /api/strategy-config` — update strategy params at runtime.
//!
//! Both endpoints accept an optional `?model_id=X` query param (§8.6).
//! When supplied, the per-model params map is used; when omitted, the
//! default (global) params are used.
//!
//! The PUT endpoint validates all fields, updates the resolved
//! `Arc<RwLock<EquityStrategyParams>>`, broadcasts a `StrategyConfigChange`
//! telemetry event, and appends an audit log to the database.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::strategy::EquityStrategyParams;

use super::{internal_error, ApiResult, AppState};

#[derive(Debug, Deserialize)]
pub struct ModelIdQuery {
    pub model_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StrategyConfigResponse {
    pub model_id: Option<String>,
    pub entry_threshold: f64,
    pub exit_threshold: f64,
    pub sma_window: usize,
    pub pred_5d_filter: bool,
    pub enable_shorting: bool,
    pub short_entry_threshold: f64,
    pub short_exit_threshold: f64,
}

/// Resolve the `Arc<RwLock<EquityStrategyParams>>` for a given model_id.
///
/// If `model_id` is supplied and exists in the per-model map, return that
/// entry. Otherwise fall back to the default (global) params.
///
/// Returns the resolved params handle and the model_id that was used
/// (None means the default was used).
async fn resolve_params(
    state: &AppState,
    model_id: &Option<String>,
) -> (
    std::sync::Arc<tokio::sync::RwLock<EquityStrategyParams>>,
    Option<String>,
) {
    if let Some(id) = model_id {
        let map = state.strategy_params_by_model.read().await;
        if let Some(params) = map.get(id) {
            return (params.clone(), Some(id.clone()));
        }
        // Fall through to default if model_id not found in map.
    }
    (state.strategy_params.clone(), None)
}

pub async fn handle_get(
    State(state): State<AppState>,
    Query(query): Query<ModelIdQuery>,
) -> ApiResult<StrategyConfigResponse> {
    let (params, resolved_model_id) = resolve_params(&state, &query.model_id).await;
    let sp = params.read().await;
    Ok(Json(StrategyConfigResponse {
        model_id: resolved_model_id,
        entry_threshold: sp.entry_threshold,
        exit_threshold: sp.exit_threshold,
        sma_window: sp.sma_window,
        pred_5d_filter: sp.pred_5d_filter,
        enable_shorting: sp.enable_shorting,
        short_entry_threshold: sp.short_entry_threshold,
        short_exit_threshold: sp.short_exit_threshold,
    }))
}

#[derive(Debug, Deserialize)]
pub struct StrategyConfigUpdate {
    #[serde(default)]
    pub entry_threshold: Option<f64>,
    #[serde(default)]
    pub exit_threshold: Option<f64>,
    #[serde(default)]
    pub sma_window: Option<usize>,
    #[serde(default)]
    pub pred_5d_filter: Option<bool>,
    #[serde(default)]
    pub enable_shorting: Option<bool>,
    #[serde(default)]
    pub short_entry_threshold: Option<f64>,
    #[serde(default)]
    pub short_exit_threshold: Option<f64>,
}

pub async fn handle_put(
    State(state): State<AppState>,
    Query(query): Query<ModelIdQuery>,
    Json(update): Json<StrategyConfigUpdate>,
) -> Result<Json<StrategyConfigResponse>, (StatusCode, String)> {
    let (params, resolved_model_id) = resolve_params(&state, &query.model_id).await;
    let mut sp = params.write().await;

    // Capture old params for event emission.
    let old = sp.clone();

    // Validate and apply each field that was provided.
    if let Some(v) = update.entry_threshold {
        if v <= 0.0 {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("entry_threshold must be > 0, got {v}"),
            ));
        }
        sp.entry_threshold = v;
    }

    if let Some(v) = update.exit_threshold {
        if v >= 0.0 {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("exit_threshold must be < 0, got {v}"),
            ));
        }
        sp.exit_threshold = v;
    }

    if let Some(v) = update.sma_window {
        if v == 0 || v > 300 {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("sma_window must be 1..=300, got {v}"),
            ));
        }
        sp.sma_window = v;
    }

    if let Some(v) = update.pred_5d_filter {
        sp.pred_5d_filter = v;
    }

    if let Some(v) = update.enable_shorting {
        sp.enable_shorting = v;
    }

    if let Some(v) = update.short_entry_threshold {
        if v >= 0.0 {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("short_entry_threshold must be < 0, got {v}"),
            ));
        }
        sp.short_entry_threshold = v;
    }

    if let Some(v) = update.short_exit_threshold {
        if v <= sp.short_entry_threshold {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "short_exit_threshold ({v}) must be > short_entry_threshold ({})",
                    sp.short_entry_threshold
                ),
            ));
        }
        sp.short_exit_threshold = v;
    }

    let response = StrategyConfigResponse {
        model_id: resolved_model_id.clone(),
        entry_threshold: sp.entry_threshold,
        exit_threshold: sp.exit_threshold,
        sma_window: sp.sma_window,
        pred_5d_filter: sp.pred_5d_filter,
        enable_shorting: sp.enable_shorting,
        short_entry_threshold: sp.short_entry_threshold,
        short_exit_threshold: sp.short_exit_threshold,
    };

    // Drop the write lock before broadcasting (broadcast may re-read params).
    drop(sp);

    // Broadcast config change to WebSocket clients.
    let _ = state.tx.send(super::ws::TelemetryEvent::StrategyConfigChange {
        entry_threshold: response.entry_threshold,
        exit_threshold: response.exit_threshold,
        sma_window: response.sma_window,
        pred_5d_filter: response.pred_5d_filter,
        enable_shorting: response.enable_shorting,
        short_entry_threshold: response.short_entry_threshold,
        short_exit_threshold: response.short_exit_threshold,
    });

    // Emit event for the unified log — attribute to the model if resolved.
    let event = crate::event::EngineEvent::strategy_config_changed(
        &old,
        &crate::strategy::EquityStrategyParams {
            entry_threshold: response.entry_threshold,
            exit_threshold: response.exit_threshold,
            sma_window: response.sma_window,
            pred_5d_filter: response.pred_5d_filter,
            enable_shorting: response.enable_shorting,
            short_entry_threshold: response.short_entry_threshold,
            short_exit_threshold: response.short_exit_threshold,
        },
    );
    if let Some(ref mid) = resolved_model_id {
        state
            .event_logger
            .emit_for_model(event, mid, None)
            .await;
    } else {
        state.event_logger.emit(event).await;
    }

    info!(
        model_id = ?resolved_model_id,
        entry_threshold = response.entry_threshold,
        exit_threshold = response.exit_threshold,
        sma_window = response.sma_window,
        pred_5d_filter = response.pred_5d_filter,
        enable_shorting = response.enable_shorting,
        "strategy config updated"
    );

    Ok(Json(response))
}
