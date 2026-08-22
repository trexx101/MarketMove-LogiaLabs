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
    symbol: Option<String>,
}

/// Derive current position from equity_trades for a given symbol.
/// Counts total qty: buys add, sells subtract. Returns 1 (long), -1 (short), or 0 (flat).
async fn derive_position(pool: &DbPool, symbol: &str) -> anyhow::Result<i64> {
    let row = sqlx::query(
        "SELECT COALESCE(CAST(SUM(CASE WHEN side = 'buy' THEN qty ELSE -qty END) AS REAL), 0.0) AS net_qty
         FROM equity_trades WHERE symbol = ?1",
    )
    .bind(symbol)
    .fetch_one(pool)
    .await?;
    let net_qty: f64 = row.get("net_qty");
    Ok(net_qty.signum() as i64)
}

/// Fetch unrealized PnL: last close vs entry price, with side sign.
async fn derive_unrealized_pnl(pool: &DbPool, symbol: &str, position: i64) -> anyhow::Result<f64> {
    if position == 0 {
        return Ok(0.0);
    }
    let entry = db::fetch_equity_entry_trade_price(pool, symbol).await?;
    let close = db::fetch_equity_close_at_ts(pool, symbol, chrono::Utc::now().timestamp()).await?;
    match (entry, close) {
        (Some(e), Some(c)) if c > 0.0 && e > 0.0 => {
            let pct = (c - e) / e;
            Ok(pct * position as f64)
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

    let position = derive_position(pool, symbol)
        .await
        .map_err(|e| internal_error("derive_position", e))?;
    let realized_pnl = db::sum_equity_realized_pnl(pool, symbol)
        .await
        .map_err(|e| internal_error("sum_equity_realized_pnl", e))?;
    let unrealized_pnl = derive_unrealized_pnl(pool, symbol, position)
        .await
        .map_err(|e| internal_error("derive_unrealized_pnl", e))?;
    let entry_price = db::fetch_equity_entry_trade_price(pool, symbol)
        .await
        .map_err(|e| internal_error("fetch_equity_entry_trade_price", e))?;
    let candle = db::fetch_latest_equity_candle(pool, symbol)
        .await
        .map_err(|e| internal_error("fetch_latest_equity_candle", e))?;
    let pred = db::fetch_latest_equity_prediction(pool, symbol)
        .await
        .map_err(|e| internal_error("fetch_latest_equity_prediction", e))?;

    let last_close = candle.as_ref().map(|c| c.close);
    let last_candle_ts = candle.as_ref().map(|c| {
        chrono::DateTime::<chrono::Utc>::from_timestamp(c.ts, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    });
    let staleness_secs = candle
        .as_ref()
        .map(|c| chrono::Utc::now().timestamp() - c.ts)
        .unwrap_or(0);

    let entry_ts = db::fetch_equity_entry_trade_ts(pool, symbol)
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