use std::collections::HashMap;

use axum::{extract::State, response::Json};
use serde::Serialize;

use crate::db;

use super::{internal_error, ts_to_rfc3339, ApiResult, AppState};

#[derive(Debug, Serialize)]
pub(crate) struct AccuracyResponse {
    directional_1d: f64,
    directional_5d: f64,
    directional_21d: f64,
    mae_1d: f64,
    mae_5d: f64,
    mae_21d: f64,
    resolved_count: usize,
}

#[derive(Serialize)]
pub(crate) struct PredictionsResponse {
    pub latest: Option<PredictionDto>,
    pub history: Vec<PredictionDto>,
}

#[derive(Serialize)]
pub(crate) struct PredictionDto {
    pub candle_ts: String,
    pub pred_1h: f64,
    pub pred_4h: f64,
    pub pred_24h: f64,
    pub pred_1h_approx: f64,
    pub pred_5h_approx: f64,
    pub created_at: String,
    pub actual_1h: Option<f64>,
    pub actual_4h: Option<f64>,
    pub actual_24h: Option<f64>,
}

pub(crate) async fn handle_predictions(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> ApiResult<PredictionsResponse> {
    let symbol = params.get("symbol").cloned().unwrap_or_else(|| state.symbol.clone());
    let history = db::fetch_recent_equity_predictions(&state.pool, &symbol, 48)
        .await
        .map_err(|e| internal_error("fetch_recent_equity_predictions", e))?;

    let latest = history.first().map(|row| equity_prediction_to_dto(row));
    let history_dtos: Vec<PredictionDto> = history.iter().map(equity_prediction_to_dto).collect();

    Ok(Json(PredictionsResponse {
        latest,
        history: history_dtos,
    }))
}

pub(crate) async fn handle_accuracy(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> ApiResult<AccuracyResponse> {
    let symbol = params.get("symbol").cloned().unwrap_or_else(|| state.symbol.clone());
    // Optional `since` window in days. When provided (>0), accuracy is computed
    // only over predictions at/after (now - since_days). This lets the dashboard
    // show reliability over a specific recent window (e.g. 30/60/90 days) rather
    // than the hardcoded most-recent-500 all-time snapshot.
    let since_ts = match params.get("since").and_then(|v| v.parse::<i64>().ok()) {
        Some(days) if days > 0 => {
            chrono::Utc::now().timestamp() - days * 86_400
        }
        _ => 0,
    };
    let stats = db::fetch_equity_accuracy_since(&state.pool, &symbol, since_ts)
        .await
        .map_err(|e| internal_error("fetch_equity_accuracy", e))?;

    if stats.resolved_count == 0 {
        return Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "equity accuracy not yet implemented or no resolved predictions".to_string(),
        ));
    }

    Ok(Json(AccuracyResponse {
        directional_1d: stats.directional_1d,
        directional_5d: stats.directional_5d,
        directional_21d: stats.directional_21d,
        mae_1d: stats.mae_1d,
        mae_5d: stats.mae_5d,
        mae_21d: stats.mae_21d,
        resolved_count: stats.resolved_count,
    }))
}

pub(crate) fn prediction_to_dto(row: &db::PredictionRow) -> PredictionDto {
    let daily = row.pred_24h;
    PredictionDto {
        candle_ts: ts_to_rfc3339(row.candle_ts),
        pred_1h: row.pred_1h,
        pred_4h: row.pred_4h,
        pred_24h: row.pred_24h,
        pred_1h_approx: daily / 6.5,
        pred_5h_approx: daily * (5.0 / 6.5),
        created_at: ts_to_rfc3339(row.created_at),
        actual_1h: row.actual_1h,
        actual_4h: row.actual_4h,
        actual_24h: row.actual_24h,
    }
}

fn equity_prediction_to_dto(row: &db::EquityPredictionRow) -> PredictionDto {
    let daily = row.pred_1d;
    PredictionDto {
        candle_ts: ts_to_rfc3339(row.candle_ts),
        pred_1h: daily / 6.5,
        pred_4h: daily * (4.0 / 6.5),
        pred_24h: daily,
        pred_1h_approx: daily / 6.5,
        pred_5h_approx: daily * (5.0 / 6.5),
        created_at: ts_to_rfc3339(row.created_at),
        actual_1h: None,
        actual_4h: None,
        actual_24h: None,
    }
}
