use axum::{extract::State, response::Json};
use serde::Serialize;

use crate::api::{internal_error, ApiResult};
use crate::db::{self, DbPool};
use crate::strategy;
use crate::api::AppState;

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub mode: String,
    pub symbol: String,
    pub position: String,
    pub entry_price: f64,
    pub realized_pnl: f64,
    pub unrealized_pnl: f64,
    pub last_candle_ts: Option<String>,
    pub last_close: f64,
    pub pred_1d: Option<f64>,
    pub pred_5d: Option<f64>,
    pub pred_21d: Option<f64>,
    pub pred_1h_approx: Option<f64>,
    pub pred_5h_approx: Option<f64>,
    pub staleness_secs: u64,
    pub sma_200: Option<f64>,
    pub strategy: StrategySnapshot,
}

#[derive(Debug, Serialize)]
pub struct StrategySnapshot {
    pub entry_threshold: f64,
    pub exit_threshold: f64,
    pub sma_window: usize,
    pub pred_5d_filter: bool,
    pub enable_shorting: bool,
    pub short_entry_threshold: f64,
    pub short_exit_threshold: f64,
}

pub(crate) async fn handle_status(
    State(state): State<AppState>,
) -> ApiResult<StatusResponse> {
    let pool = &state.pool;

    let mode = {
        let mode_lock = state.trading_mode.read().await;
        format!("{:?}", *mode_lock).to_lowercase()
    };

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
        strategy::Position::Flat => (0.0, 0.0),
        strategy::Position::Long => {
            let entry = db::fetch_equity_entry_trade_price(pool, &state.symbol)
                .await
                .map_err(|e| internal_error("fetch_equity_entry_trade_price", e))?;
            let last_close = candle.as_ref().map(|c| c.close).unwrap_or(0.0);
            let unrealized = match entry {
                Some(ep) => (last_close - ep) * 1.0,
                None => 0.0,
            };
            (entry.unwrap_or(0.0), unrealized)
        }
        strategy::Position::Short => {
            // Short via inverse ETF (PSQ). The paper executor records PSQ entry
            // at its actual market price, but exits use the QQQ close passed by
            // the scheduler. We scale QQQ close into PSQ price space using the
            // PSQ/QQQ ratio observed at entry time.
            let psq_entry = db::fetch_equity_entry_trade_price(pool, &state.short_symbol)
                .await
                .map_err(|e| internal_error("fetch_equity_entry_trade_price(psq)", e))?;
            let entry_ts = db::fetch_equity_entry_trade_ts(pool, &state.short_symbol)
                .await
                .map_err(|e| internal_error("fetch_equity_entry_trade_ts", e))?;
            let last_close = candle.as_ref().map(|c| c.close).unwrap_or(0.0);
            let unrealized = match (psq_entry, entry_ts) {
                (Some(psq_ep), Some(ets)) => {
                    let qqq_at_entry = db::fetch_equity_close_at_ts(pool, &state.symbol, ets)
                        .await
                        .map_err(|e| internal_error("fetch_equity_close_at_ts", e))?;
                    match qqq_at_entry {
                        Some(qqq_ep) if qqq_ep > 0.0 => {
                            // PSQ is an inverse ETF (~-1x QQQ daily), so
                            // PSQ_return ≈ -(QQQ_return).
                            // PnL ≈ PSQ_entry * (QQQ_current / QQQ_entry - 1).
                            // Positive when QQQ rose (short loses), negative when QQQ fell (short wins).
                            psq_ep * (last_close / qqq_ep - 1.0)
                        }
                        _ => 0.0,
                    }
                }
                _ => 0.0,
            };
            (psq_entry.unwrap_or(0.0), unrealized)
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

    // Single read of strategy_params for SMA + snapshot.
    let (sma, sma_valid, strategy_snapshot) = {
        let params = state.strategy_params.read().await;
        let sma_window = params.sma_window;

        let chart_data = db::fetch_equity_candles_asc(pool, &state.symbol, sma_window as i64)
            .await
            .map_err(|e| internal_error("fetch_equity_candles_asc", e))?;
        let closes: Vec<f64> = chart_data.iter().map(|c| c.close).collect();
        let (sma, sma_valid) = strategy::compute_sma(&closes, sma_window);

        let snapshot = StrategySnapshot {
            entry_threshold: params.entry_threshold,
            exit_threshold: params.exit_threshold,
            sma_window: params.sma_window,
            pred_5d_filter: params.pred_5d_filter,
            enable_shorting: params.enable_shorting,
            short_entry_threshold: params.short_entry_threshold,
            short_exit_threshold: params.short_exit_threshold,
        };
        (sma, sma_valid, snapshot)
    };

    // Approximate shorter-horizon predictions (6.5 trading hours/day).
    let (pred_1h_approx, pred_5h_approx) = match &latest_pred {
        Some(p) => (
            Some(p.pred_1d / 6.5),
            Some(p.pred_1d * (5.0 / 6.5)),
        ),
        None => (None, None),
    };

    Ok(Json(StatusResponse {
        mode,
        position: position.to_string(),
        symbol: state.symbol.clone(),
        entry_price,
        unrealized_pnl,
        realized_pnl,
        last_candle_ts: candle.as_ref().map(|c| crate::api::ts_to_rfc3339(c.ts)),
        last_close: candle.as_ref().map(|c| c.close).unwrap_or(0.0),
        pred_1d: latest_pred.as_ref().map(|p| p.pred_1d),
        pred_5d: latest_pred.as_ref().map(|p| p.pred_5d),
        pred_21d: latest_pred.as_ref().map(|p| p.pred_21d),
        pred_1h_approx,
        pred_5h_approx,
        staleness_secs,
        sma_200: if sma_valid { Some(sma) } else { None },
        strategy: strategy_snapshot,
    }))
}

/// Return the current market state (open / pre-market / closed / weekend).
pub(crate) async fn handle_market_state() -> Json<crate::market_hours::MarketState> {
    Json(crate::market_hours::market_state(chrono::Utc::now().timestamp()))
}
