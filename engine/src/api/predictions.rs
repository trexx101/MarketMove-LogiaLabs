use axum::extract::{Query, State};
use axum::response::Json;
use serde::{Deserialize, Serialize};

use crate::db;

use super::{internal_error, ts_to_rfc3339, ApiResult, AppState};

#[derive(Debug, Deserialize)]
pub(crate) struct SymbolQuery {
    #[serde(default)]
    symbol: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AccuracyResponse {
    directional_1h: f64,
    directional_4h: f64,
    directional_24h: f64,
    mae_1h: f64,
    mae_4h: f64,
    mae_24h: f64,
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
    Query(params): Query<SymbolQuery>,
    State(state): State<AppState>,
) -> ApiResult<PredictionsResponse> {
    let symbol = params.symbol.as_deref().unwrap_or(&state.symbol);
    let history = db::fetch_recent_equity_predictions(&state.pool, symbol, 48)
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
    Query(params): Query<SymbolQuery>,
    State(state): State<AppState>,
) -> ApiResult<AccuracyResponse> {
    let symbol = params.symbol.as_deref().unwrap_or(&state.symbol);
    let stats = db::fetch_equity_accuracy(&state.pool, symbol)
        .await
        .map_err(|e| internal_error("fetch_equity_accuracy", e))?;

    Ok(Json(AccuracyResponse {
        directional_1h: stats.directional_1h,
        directional_4h: stats.directional_4h,
        directional_24h: stats.directional_24h,
        mae_1h: stats.mae_1h,
        mae_4h: stats.mae_4h,
        mae_24h: stats.mae_24h,
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