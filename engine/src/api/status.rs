use sqlx::Row;

use axum::{extract::{Query, State}, response::Json};
use serde::{Deserialize, Serialize};

use crate::api::{internal_error, ApiResult};
use crate::db::{self, DbPool};
use crate::api::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct StatusResponse {
    pub symbol: String,
    pub mode: String,
    pub position: String,
    pub entry_price: Option<f64>,
    pub realized_pnl: f64,
    pub unrealized_pnl: f64,
    pub last_close: Option<f64>,
    pub pred_1d: Option<f64>,
    pub pred_5d: Option<f64>,
    pub pred_21d: Option<f64>,
    pub position_ts: Option<i64>,
    pub last_candle_ts: Option<String>,
    pub staleness_secs: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StatusQuery {
    #[serde(default)]
    pub(crate) symbol: Option<String>,
}

/// Return the net quantity (buys - sells) for a given symbol from equity_trades.
async fn net_qty_for_symbol(pool: &DbPool, symbol: &str) -> anyhow::Result<f64> {
    let row = sqlx::query(
        "SELECT COALESCE(CAST(SUM(CASE WHEN side = 'buy' THEN qty ELSE -qty END) AS REAL), 0.0) AS net_qty
         FROM equity_trades WHERE symbol = ?1",
    )
    .bind(symbol)
    .fetch_one(pool)
    .await?;
    let net_qty: f64 = row.get("net_qty");
    Ok(net_qty)
}

/// Effective position resolved across the primary AND inverse (short) ETF symbols.
///
/// Returns `(position_sign, held_symbol)` where:
/// - `position_sign` = 1 (long primary), -1 (short via inverse ETF), or 0 (flat).
/// - `held_symbol` is the instrument actually held, used for entry/close/PnL lookups.
///   When flat, `held_symbol` defaults to `primary`.
async fn derive_effective_position(
    pool: &DbPool,
    primary: &str,
    inverse: &str,
) -> anyhow::Result<(i64, String)> {
    let primary_net = net_qty_for_symbol(pool, primary).await?;
    let inverse_net = net_qty_for_symbol(pool, inverse).await?;

    if primary_net > 0.0 && inverse_net <= 0.0 {
        Ok((1, primary.to_string()))
    } else if inverse_net > 0.0 && primary_net <= 0.0 {
        Ok((-1, inverse.to_string()))
    } else {
        Ok((0, primary.to_string()))
    }
}

/// Fetch unrealized PnL for the held instrument.
///
/// For a short position the held instrument is the inverse ETF (e.g. PSQ) —
/// its price change *is* the PnL, so no sign flip is needed (we own the shares).
async fn derive_unrealized_pnl(
    pool: &DbPool,
    held_symbol: &str,
    position: i64,
) -> anyhow::Result<f64> {
    if position == 0 {
        return Ok(0.0);
    }
    let entry = db::fetch_equity_entry_trade_price(pool, held_symbol).await?;
    let close = db::fetch_equity_close_at_ts(pool, held_symbol, chrono::Utc::now().timestamp()).await?;
    match (entry, close) {
        (Some(e), Some(c)) if c > 0.0 && e > 0.0 => {
            Ok((c - e) / e)
        }
        _ => Ok(0.0),
    }
}

pub(crate) async fn handle_status(
    Query(params): Query<StatusQuery>,
    State(state): State<AppState>,
) -> ApiResult<StatusResponse> {
    let pool = &state.pool;
    let symbol = params.symbol.as_deref().unwrap_or(&state.symbol);
    let inverse = &state.short_symbol;

    // Resolve effective position across primary + inverse ETF symbols.
    let (position, held_symbol) = derive_effective_position(pool, symbol, inverse)
        .await
        .map_err(|e| internal_error("derive_effective_position", e))?;

    // Realized PnL: sum across both legs (primary + inverse).
    let realized_pnl_primary = db::sum_equity_realized_pnl(pool, symbol)
        .await
        .map_err(|e| internal_error("sum_equity_realized_pnl (primary)", e))?;
    let realized_pnl_inverse = db::sum_equity_realized_pnl(pool, inverse)
        .await
        .map_err(|e| internal_error("sum_equity_realized_pnl (inverse)", e))?;
    let realized_pnl = realized_pnl_primary + realized_pnl_inverse;

    // Unrealized PnL & entry price: use the held instrument.
    let unrealized_pnl = derive_unrealized_pnl(pool, &held_symbol, position)
        .await
        .map_err(|e| internal_error("derive_unrealized_pnl", e))?;
    let entry_price = db::fetch_equity_entry_trade_price(pool, &held_symbol)
        .await
        .map_err(|e| internal_error("fetch_equity_entry_trade_price", e))?;

    // Candles & staleness: use the PRIMARY symbol (market data).
    let candle = db::fetch_latest_equity_candle(pool, symbol)
        .await
        .map_err(|e| internal_error("fetch_latest_equity_candle", e))?;
    let pred = db::fetch_latest_equity_prediction(pool, symbol)
        .await
        .map_err(|e| internal_error("fetch_latest_equity_prediction", e))?;

    // Close price: use the held instrument's close (what we actually own).
    let last_close = if held_symbol != *symbol {
        let hc = db::fetch_latest_equity_candle(pool, &held_symbol)
            .await
            .map_err(|e| internal_error("fetch_latest_equity_candle (held)", e))?;
        hc.as_ref().map(|c| c.close)
    } else {
        candle.as_ref().map(|c| c.close)
    };

    let last_candle_ts = candle.as_ref().map(|c| {
        chrono::DateTime::<chrono::Utc>::from_timestamp(c.ts, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    });
    let staleness_secs = candle
        .as_ref()
        .map(|c| chrono::Utc::now().timestamp() - c.ts)
        .unwrap_or(0);

    let entry_ts = db::fetch_equity_entry_trade_ts(pool, &held_symbol)
        .await
        .map_err(|e| internal_error("fetch_equity_entry_trade_ts", e))?;

    // Resolve trading mode from the shared Arc — read lock is cheap.
    let mode = {
        let guard = state.trading_mode.read().await;
        format!("{:?}", *guard).to_lowercase() // PAPER or LIVE
    };

    Ok(Json(StatusResponse {
        symbol: symbol.to_string(),
        mode,
        position: match position {
            1 => "long".into(),
            -1 => "short".into(),
            _ => "flat".into(),
        },
        entry_price,
        realized_pnl,
        unrealized_pnl,
        last_close,
        pred_1d: pred.as_ref().map(|p| p.pred_1d),
        pred_5d: pred.as_ref().map(|p| p.pred_5d),
        pred_21d: pred.as_ref().map(|p| p.pred_21d),
        position_ts: entry_ts,
        last_candle_ts,
        staleness_secs,
    }))
}

/// Wire-diagram endpoint: returns the same shape as handle_status but
/// without the position risk. Temporary until the UI kills it.
pub(crate) async fn handle_market_state(
    Query(params): Query<StatusQuery>,
    State(state): State<AppState>,
) -> ApiResult<StatusResponse> {
    handle_status(Query(params), State(state)).await
}