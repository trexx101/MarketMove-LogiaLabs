use axum::{extract::State, response::Json};
use serde::Serialize;

use crate::{db, features::equities_v2, market_hours::MarketState, strategy};

use super::{internal_error, ts_to_rfc3339, ApiResult, AppState};

#[derive(Serialize)]
pub(crate) struct StatusResponse {
    pub mode: String,
    pub symbol: String,
    pub position: String,
    pub entry_price: Option<f64>,
    pub realized_pnl: f64,
    pub unrealized_pnl: Option<f64>,
    pub last_candle_ts: Option<String>,
    pub last_close: Option<f64>,
    pub pred_1d: Option<f64>,
    pub pred_5d: Option<f64>,
    pub pred_21d: Option<f64>,
    pub pred_1h_approx: Option<f64>,
    pub pred_5h_approx: Option<f64>,
    pub staleness_secs: u64,
    pub sma_200: Option<f64>,
}

pub(crate) async fn handle_market_state() -> Json<MarketState> {
    Json(crate::market_hours::market_state(chrono::Utc::now().timestamp()))
}

pub(crate) async fn handle_status(State(state): State<AppState>) -> ApiResult<StatusResponse> {
    let pool = &state.pool;

    let position_raw = db::load_position(pool)
        .await
        .map_err(|e| internal_error("load_position", e))?;
    let position = strategy::Position::from_i64(position_raw);

    let realized_pnl = db::sum_equity_realized_pnl(pool, &state.symbol)
        .await
        .map_err(|e| internal_error("sum_equity_realized_pnl", e))?;

    let candle = db::fetch_latest_equity_candle(pool, &state.symbol)
        .await
        .map_err(|e| internal_error("fetch_latest_equity_candle", e))?;

    let (entry_price, unrealized_pnl) = match position {
        strategy::Position::Flat => (None, None),
        strategy::Position::Long | strategy::Position::Short => {
            let entry = db::fetch_entry_trade_price(pool)
                .await
                .map_err(|e| internal_error("fetch_entry_trade_price", e))?;
            let last_close = candle.as_ref().map(|c| c.close);
            let unrealized = match (entry, last_close) {
                (Some(ep), Some(lc)) => match position {
                    strategy::Position::Long => Some((lc - ep) * 1.0),
                    strategy::Position::Short => Some((ep - lc) * 1.0),
                    strategy::Position::Flat => None,
                },
                _ => None,
            };
            (entry, unrealized)
        }
    };

    let latest_pred = db::fetch_latest_equity_prediction(pool, &state.symbol)
        .await
        .map_err(|e| internal_error("fetch_latest_equity_prediction", e))?;

    let staleness_secs = match db::latest_equity_candle_ts(pool, &state.symbol)
        .await
        .map_err(|e| internal_error("latest_equity_candle_ts", e))?
    {
        Some(ts) => {
            let now = chrono::Utc::now().timestamp();
            now.saturating_sub(ts).max(0) as u64
        }
        None => u64::MAX,
    };

    let sma_200 = {
        let closes: Option<Vec<f64>> = if candle.is_some() {
            db::fetch_equity_candles_desc(&state.pool, &state.symbol, 200)
                .await
                .map(|rows| rows.into_iter().map(|c| c.close).collect())
                .ok()
        } else {
            None
        };
        closes
            .and_then(|c| equities_v2::rolling_sma(&c, 200).into_iter().last())
            .filter(|&v| v.is_finite())
    };

    // Phase 3.4: trading mode lives behind an Arc<RwLock<_>> for the runtime
    // toggle. Read briefly to render the current value.
    let mode = *state.trading_mode.read().await;

    Ok(Json(StatusResponse {
        mode: mode.to_string(),
        symbol: state.symbol.clone(),
        position: position.to_string(),
        entry_price,
        realized_pnl,
        unrealized_pnl,
        last_candle_ts: candle.as_ref().map(|c| ts_to_rfc3339(c.ts)),
        last_close: candle.as_ref().map(|c| c.close),
        pred_1d: latest_pred.as_ref().map(|p| p.pred_1d),
        pred_5d: latest_pred.as_ref().map(|p| p.pred_5d),
        pred_21d: latest_pred.as_ref().map(|p| p.pred_21d),
        pred_1h_approx: latest_pred.as_ref().map(|p| p.pred_1d / 6.5),
        pred_5h_approx: latest_pred.as_ref().map(|p| p.pred_1d * (5.0 / 6.5)),
        staleness_secs,
        sma_200,
    }))
}
