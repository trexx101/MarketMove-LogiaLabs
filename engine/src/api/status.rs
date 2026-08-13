use axum::{extract::{State, Query}, response::Json};
use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::api::{internal_error, ApiResult};
use crate::db::{self, DbPool};
use crate::strategy;
use crate::api::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct StatusQuery {
    pub(crate) model_id: Option<String>,
    pub(crate) symbol: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub mode: String,
    pub model_id: String,
    pub symbol: String,
    pub short_symbol: String,
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

/// Resolve a status request to a (model_id, primary_symbol, short_symbol) tuple.
/// Priority: explicit `model_id` query param -> explicit `symbol` -> legacy state default.
async fn resolve_status_symbols(
    state: &AppState,
    query: &StatusQuery,
) -> anyhow::Result<(String, String, String)> {
    if let Some(mid) = &query.model_id {
        let row = sqlx::query_as::<_, (String, String)>(
            "SELECT primary_symbol, inverse_symbol FROM trading_models WHERE model_id = ?1"
        )
        .bind(mid)
        .fetch_optional(&state.pool)
        .await
        .context("resolve_status_symbols: lookup trading_models")?;
        if let Some((primary, inverse)) = row {
            return Ok((mid.clone(), primary, inverse));
        }
    }
    if let Some(sym) = &query.symbol {
        return Ok(("legacy".to_string(), sym.clone(), state.short_symbol.clone()));
    }
    Ok(("legacy".to_string(), state.symbol.clone(), state.short_symbol.clone()))
}

pub(crate) async fn handle_status(
    State(state): State<AppState>,
    Query(query): Query<StatusQuery>,
) -> ApiResult<StatusResponse> {
    let pool = &state.pool;

    let (model_id, primary_symbol, short_symbol) = resolve_status_symbols(&state, &query)
        .await
        .map_err(|e| internal_error("resolve_status_symbols", e))?;

    let mode = {
        let mode_lock = state.trading_mode.read().await;
        format!("{:?}", *mode_lock).to_lowercase()
    };

    let position_raw = db::load_position(pool, &model_id, &primary_symbol)
        .await
        .map_err(|e| internal_error("load_position", e))?;
    let position = strategy::Position::from_i64(position_raw);

    let realized_pnl = db::sum_equity_realized_pnl(pool, &primary_symbol)
        .await
        .map_err(|e| internal_error("sum_equity_realized_pnl", e))?;

    let candle = db::fetch_latest_equity_candle(pool, &primary_symbol)
        .await
        .map_err(|e| internal_error("fetch_latest_equity_candle", e))?;

    let (entry_price, unrealized_pnl) = match position {
        strategy::Position::Flat => (0.0, 0.0),
        strategy::Position::Long => {
            let entry = db::fetch_equity_entry_trade_price(pool, &primary_symbol)
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
            let psq_entry = db::fetch_equity_entry_trade_price(pool, &short_symbol)
                .await
                .map_err(|e| internal_error("fetch_equity_entry_trade_price(psq)", e))?;
            let entry_ts = db::fetch_equity_entry_trade_ts(pool, &short_symbol)
                .await
                .map_err(|e| internal_error("fetch_equity_entry_trade_ts", e))?;
            let last_close = candle.as_ref().map(|c| c.close).unwrap_or(0.0);
            let unrealized = match (psq_entry, entry_ts) {
                (Some(psq_ep), Some(ets)) => {
                    let qqq_at_entry = db::fetch_equity_close_at_ts(pool, &primary_symbol, ets)
                        .await
                        .map_err(|e| internal_error("fetch_equity_close_at_ts", e))?;
                    match qqq_at_entry {
                        Some(qqq_ep) if qqq_ep > 0.0 => {
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

    let latest_pred = db::fetch_latest_equity_prediction(pool, &primary_symbol)
        .await
        .map_err(|e| internal_error("fetch_latest_equity_prediction", e))?;

    let staleness_secs = match db::latest_equity_candle_ts(pool, &primary_symbol)
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
    // For per-model requests, try the per-model params; fall back to default.
    let params_arc = {
        let by_model = state.strategy_params_by_model.read().await;
        if let Some(arc) = by_model.get(&model_id) {
            arc.clone()
        } else {
            state.strategy_params.clone()
        }
    };
    let (sma, sma_valid, strategy_snapshot) = {
        let params = params_arc.read().await;
        let sma_window = params.sma_window;

        let chart_data = db::fetch_equity_candles_asc(pool, &primary_symbol, sma_window as i64)
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
        model_id: model_id.clone(),
        symbol: primary_symbol.clone(),
        short_symbol: short_symbol.clone(),
        position: position.to_string(),
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
