use axum::{extract::State, response::Json};
use serde::Serialize;

use crate::{db, strategy};

use super::{internal_error, ts_to_rfc3339, ApiResult, AppState};

#[derive(Serialize)]
pub(crate) struct ChartResponse {
    pub candles: Vec<CandleDto>,
    pub sma: Vec<SmaPoint>,
}

#[derive(Serialize)]
pub(crate) struct CandleDto {
    pub ts: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub vwap: f64,
}

#[derive(Serialize)]
pub(crate) struct SmaPoint {
    pub ts: String,
    pub value: f64,
}

pub(crate) async fn handle_chart(State(state): State<AppState>) -> ApiResult<ChartResponse> {
    let sma_window = state.strategy_params.read().await.sma_window;
    let limit = (sma_window * 2).min(500);
    let candles = db::fetch_recent_equity_candles(&state.pool, &state.symbol, limit as i64)
        .await
        .map_err(|e| internal_error("fetch_recent_equity_candles", e))?;

    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let mut sma_points = Vec::new();

    for i in 0..candles.len() {
        let (mean, valid) = strategy::compute_sma(&closes[..=i], sma_window);
        if valid {
            sma_points.push(SmaPoint {
                ts: ts_to_rfc3339(candles[i].ts),
                value: mean,
            });
        }
    }

    let candle_dtos: Vec<CandleDto> = candles.iter().map(equity_candle_to_dto).collect();

    Ok(Json(ChartResponse {
        candles: candle_dtos,
        sma: sma_points,
    }))
}

fn equity_candle_to_dto(c: &db::EquityCandle) -> CandleDto {
    CandleDto {
        ts: ts_to_rfc3339(c.ts),
        open: c.open,
        high: c.high,
        low: c.low,
        close: c.close,
        volume: c.volume as f64,
        vwap: c.close,
    }
}
