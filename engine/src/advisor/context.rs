//! Build the `AdvisorContext` from current DB state.
//!
//! Pure read — never mutates anything. All context is assembled from the
//! latest DB rows: prediction + its stored feature_json, sentiment, macros,
//! current position, and recent closed trades.

use anyhow::{Context, Result};
use sqlx::Row;
use chrono::Utc;

use super::{AdvisorContext, EarningsEvent, FeatureSnapshot, MacroRelease, MacroSnapshot,
           RecentTrade};
use crate::db::{DbPool, EquityPredictionRow};

/// Build the full advisor context from the database.
///
/// Missing data is represented as `None` / empty vecs — never panics.
/// The prompt layer handles absent fields gracefully.
pub async fn build_context(
    pool: &DbPool,
    symbol: &str,
) -> Result<AdvisorContext> {
    let now = Utc::now();

    // ── 1. Market session state ────────────────────────────
    let ms = crate::market_hours::market_state(now.timestamp());
    let market_session = format!("{:?}", ms.session); // "Closed" | "PreMarket" | "Regular" | "AfterHours"

    // ── 2. Latest prediction + its feature snapshot ────────
    let (pred_1d, pred_5d, pred_21d, pred_ts, features) =
        fetch_latest_prediction_with_features(pool, symbol).await;

    // ── 3. Sentiment ───────────────────────────────────────
    let (sentiment_score, sentiment_buzz, sentiment_source) =
        fetch_latest_sentiment(pool, symbol).await;

    // ── 4. Macro context ───────────────────────────────────
    let macro_ctx = build_macro_snapshot(pool).await;

    // ── 5. Current position ───────────────────────────────
    let (position_side, entry_price, entry_ts, unrealized_pnl) =
        fetch_current_position(pool, symbol).await;

    // ── 6. Recent closed trades ────────────────────────────
    let recent_trades = fetch_recent_closed_trades(pool, symbol, 5).await;

    Ok(AdvisorContext {
        as_of: now,
        symbol: symbol.to_string(),
        market_session,
        next_open_utc: Some(ms.next_open_ts),
        next_close_utc: Some(ms.next_close_ts),
        is_trading_day: ms.is_trading_day,
        holiday_name: ms.holiday_name,
        pred_1d,
        pred_5d,
        pred_21d,
        pred_ts,
        features,
        sentiment_score,
        sentiment_buzz,
        sentiment_source,
        macro_ctx,
        position_side,
        entry_price,
        entry_ts,
        unrealized_pnl,
        realized_pnl_session: None, // computed from trade sum, can add later
        recent_trades,
    })
}

// ── internal helpers ────────────────────────────────────────────────

/// Fetch the latest prediction row and deserialize its stored feature_json
/// into a FeatureSnapshot. The feature values here are EXACTLY what the
/// model saw at inference time — same as what FeatureInspector.svelte
/// would display for the same candle_ts.
async fn fetch_latest_prediction_with_features(
    pool: &DbPool,
    symbol: &str,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<i64>, FeatureSnapshot) {
    let default_features = FeatureSnapshot::default();

    let row = match crate::db::fetch_latest_equity_prediction(pool, symbol).await {
        Ok(Some(r)) => r,
        _ => return (None, None, None, None, default_features),
    };

    let features = parse_features_json(&row.features_json).unwrap_or(default_features);

    (
        Some(row.pred_1d),
        Some(row.pred_5d),
        Some(row.pred_21d),
        Some(row.candle_ts),
        features,
    )
}

/// Parse the features_json column into a FeatureSnapshot.
///
/// The JSON can be either:
///   - {"trend_slope": 0.18, "trend_adx": 31.0, ...}  (name-keyed)
///   - [0.18, 31.0, ...]                               (array in fixed order)
fn parse_features_json(json_str: &str) -> Result<FeatureSnapshot> {
    let v: serde_json::Value = serde_json::from_str(json_str)?;

    if let Some(obj) = v.as_object() {
        let default = FeatureSnapshot::default();
        let get = |key: &str| obj.get(key).and_then(|v| v.as_f64()).unwrap_or_else(|| {
            // Use the struct default for missing keys so RSI isn't 0.
            match key {
                "rsi_14" => default.rsi_14,
                _ => 0.0,
            }
        });
        return Ok(FeatureSnapshot {
            trend_slope: get("trend_slope"),
            trend_adx: get("trend_adx"),
            rsi_14: get("rsi_14"),
            vix_regime: get("vix_regime"),
            tlt_corr_20d: get("tlt_corr_20d"),
            rvol_20d: get("rvol_20d"),
            gap_pct: get("gap_pct"),
            drawdown_from_50d_high: get("drawdown_from_50d_high"),
            staleness_secs: 0,
        });
    }

    // Array fallback — order must match compute_equity_features output.
    if let Some(arr) = v.as_array() {
        if arr.len() >= 8 {
            return Ok(FeatureSnapshot {
                trend_slope: arr[0].as_f64().unwrap_or(0.0),
                trend_adx: arr[1].as_f64().unwrap_or(0.0),
                rsi_14: arr[2].as_f64().unwrap_or(0.0),
                vix_regime: arr[3].as_f64().unwrap_or(0.0),
                tlt_corr_20d: arr[4].as_f64().unwrap_or(0.0),
                rvol_20d: arr[5].as_f64().unwrap_or(0.0),
                gap_pct: arr[6].as_f64().unwrap_or(0.0),
                drawdown_from_50d_high: arr[7].as_f64().unwrap_or(0.0),
                staleness_secs: 0,
            });
        }
    }

    Ok(FeatureSnapshot::default())
}

/// Fetch the latest sentiment cache row for the symbol.
async fn fetch_latest_sentiment(
    pool: &DbPool,
    symbol: &str,
) -> (Option<f64>, Option<i64>, String) {
    let row = sqlx::query(
        "SELECT score, source, buzz FROM sentiment_cache \
         WHERE symbol = ?1 ORDER BY date DESC LIMIT 1",
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await;

    match row {
        Ok(Some(r)) => {
            let score: f64 = r.get(0);
            let source: String = r.get(1);
            let buzz: i64 = r.get(2);
            (Some(score), Some(buzz), source)
        }
        _ => (None, None, "unavailable".to_string()),
    }
}

/// Build macro snapshot from equity_candles ($UST10Y, $DXY, $VIX).
async fn build_macro_snapshot(pool: &DbPool) -> MacroSnapshot {
    let ust = latest_two_closes(pool, "$UST10Y").await;
    let dxy = latest_two_closes(pool, "$DXY").await;
    let vix = latest_close(pool, "$VIX").await;

    // Earnings / macro releases: stubbed until Finnhub calendar is wired (§4.1 plan).
    // Future: call finnhub::fetch_earnings_calendar and parse macro releases.

    MacroSnapshot {
        ust_10y_latest: ust.0,
        ust_10y_prev: ust.1,
        dxy_latest: dxy.0,
        dxy_prev: dxy.1,
        vix_latest: vix,
        earnings_in_next_7d: Vec::new(),
        macro_releases_in_next_7d: Vec::new(),
    }
}

/// Get the latest close from equity_candles for a macro symbol.
async fn latest_close(pool: &DbPool, symbol: &str) -> Option<f64> {
    sqlx::query_scalar::<_, f64>(
        "SELECT close FROM equity_candles WHERE symbol = ?1 ORDER BY ts DESC LIMIT 1",
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// Get the latest and previous close values for a macro symbol.
async fn latest_two_closes(pool: &DbPool, symbol: &str) -> (Option<f64>, Option<f64>) {
    let rows: Vec<f64> = sqlx::query_scalar(
        "SELECT close FROM equity_candles WHERE symbol = ?1 ORDER BY ts DESC LIMIT 2",
    )
    .bind(symbol)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let latest = rows.first().copied();
    let prev = rows.get(1).copied();
    (latest, prev)
}

/// Fetch the current open position for the symbol.
async fn fetch_current_position(
    pool: &DbPool,
    symbol: &str,
) -> (String, Option<f64>, Option<i64>, Option<f64>) {
    // The positions table tracks position changes. The latest row gives us
    // the current side. But we also need entry price / unrealized PnL.
    // For now: query the positions table for the most recent position.
    let row = sqlx::query(
        "SELECT position FROM positions ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await;

    let side = match row {
        Ok(Some(r)) => {
            let pos_int: i64 = r.get(0);
            match pos_int {
                1 => "long".to_string(),
                -1 => "short".to_string(),
                _ => "flat".to_string(),
            }
        }
        _ => "flat".to_string(),
    };

    // If we have a position, find the entry trade (last buy for this position).
    let (entry_price, entry_ts) = if side != "flat" {
        let row = sqlx::query(
            "SELECT price, candle_ts FROM equity_trades \
             WHERE symbol = ?1 AND side = ?2 \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(symbol)
        .bind(match side.as_str() {
            "long" => "buy",
            "short" => "sell",
            _ => "", // unreachable — side is "flat"
        })
        .fetch_optional(pool)
        .await;

        match row {
            Ok(Some(r)) => {
                let price: f64 = r.get(0);
                let ts: i64 = r.get(1);
                (Some(price), Some(ts))
            }
            _ => (None, None),
        }
    } else {
        (None, None)
    };

    // Compute unrealized PnL from latest close vs entry price.
    let unrealized_pnl = if let (Some(ep), Some(_)) = (entry_price, entry_ts) {
        match latest_close(pool, symbol).await {
            Some(current_close) => {
                let mult = if side == "long" { 1.0 } else { -1.0 };
                Some((current_close - ep) * mult / ep) // fractional return
            }
            None => None,
        }
    } else {
        None
    };

    (side, entry_price, entry_ts, unrealized_pnl)
}

/// Fetch the last N closed trades. A trade is "closed" if it has a
/// matching sell that nets to zero quantity (simplified: take all trades,
/// sort by recency). For a proper implementation, group by position-id.
async fn fetch_recent_closed_trades(
    pool: &DbPool,
    symbol: &str,
    limit: usize,
) -> Vec<RecentTrade> {
    let rows = sqlx::query(
        "SELECT side, candle_ts, qty, price, realized_pnl \
         FROM equity_trades \
         WHERE symbol = ?1 \
         ORDER BY id DESC \
         LIMIT ?2",
    )
    .bind(symbol)
    .bind(limit as i64)
    .fetch_all(pool)
    .await;

    match rows {
        Ok(rs) => rs
            .into_iter()
            .map(|r| {
                let side: String = r.get(0);
                let ts: i64 = r.get(1);
                let qty: f64 = r.get(2);
                let price: f64 = r.get(3);
                let pnl: f64 = r.get(4);
                RecentTrade {
                    side,
                    entry_ts: ts,
                    exit_ts: ts, // simplified — need entry/exit tracking for accuracy
                    entry_price: price,
                    exit_price: price, // simplified
                    pnl,
                    bars_held: 1,
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

// ── Default impls ────────────────────────────────────────────────────

impl Default for FeatureSnapshot {
    fn default() -> Self {
        Self {
            trend_slope: 0.0,
            trend_adx: 0.0,
            rsi_14: 50.0,
            vix_regime: 0.0,
            tlt_corr_20d: 0.0,
            rvol_20d: 0.0,
            gap_pct: 0.0,
            drawdown_from_50d_high: 0.0,
            staleness_secs: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_features_json_name_keyed() {
        let json = r#"{
            "trend_slope": 0.18,
            "trend_adx": 31.0,
            "rsi_14": 58.2,
            "vix_regime": 1.0,
            "tlt_corr_20d": -0.31,
            "rvol_20d": 0.85,
            "gap_pct": 0.012,
            "drawdown_from_50d_high": -0.04
        }"#;
        let fs = parse_features_json(json).unwrap();
        assert!((fs.trend_slope - 0.18).abs() < 1e-9);
        assert!((fs.trend_adx - 31.0).abs() < 1e-9);
        assert!((fs.rsi_14 - 58.2).abs() < 1e-9);
        assert!((fs.drawdown_from_50d_high - (-0.04)).abs() < 1e-9);
    }

    #[test]
    fn parse_features_json_array() {
        let json = "[0.18, 31.0, 58.2, 1.0, -0.31, 0.85, 0.012, -0.04]";
        let fs = parse_features_json(json).unwrap();
        assert!((fs.trend_slope - 0.18).abs() < 1e-9);
        assert!((fs.trend_adx - 31.0).abs() < 1e-9);
    }

    #[test]
    fn parse_features_json_empty() {
        let fs = parse_features_json("{}").unwrap();
        assert_eq!(fs.trend_slope, 0.0);
        assert_eq!(fs.rsi_14, 50.0);
    }
}