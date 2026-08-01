//! Yahoo Finance v8 chart API client (free, no auth, deep history).
//!
//! Used as the **primary** equities data source for Wave A. Moomoo adapter
//! will replace it when OpenAPI credentials are wired (planned Wave D).
//!
//! Endpoint: `https://query1.finance.yahoo.com/v8/finance/chart/{symbol}?interval=1d&period1=...&period2=...`
//! Returns OHLCV arrays in the `chart.result[0].indicators.quote[0]` block.
//!
//! Symbols used: `QQQ`, `AAPL`, `MSFT`, `NVDA`, `GOOG`, `AMZN`, `META`, `TSLA`,
//! `TLT`, `GLD`, `UUP` (DXY proxy), `^VIX`.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::db::{self, DbPool, EquityCandle};

const REST_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const BACKOFF_INIT: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Daily interval. Yahoo supports 1m/5m/15m/30m/60m/1d/1wk/1mo.
const INTERVAL: &str = "1d";

/// Fetch daily OHLCV for `symbol` and upsert into the DB.
///
/// `min_candles` gates backfill: if the DB already has ≥ N rows AND the
/// latest candle is within `stale_threshold_secs`, this is a no-op.
/// If the data is stale (latest candle older than the threshold) the
/// backfill runs regardless of row count. Pass `stale_threshold_secs = 0`
/// to always fetch (used by the manual API refresh endpoint).
///
/// `range` is a Yahoo Finance period hint: "1y", "5y", "10y", "max".
pub async fn backfill(
    pool: &DbPool,
    symbol: &str,
    min_candles: i64,
    range: &str,
    stale_threshold_secs: i64,
) -> Result<usize> {
    let count = db::count_equity_candles(pool, symbol).await?;

    let now_ts = chrono::Utc::now().timestamp();
    let latest_ts = db::latest_equity_candle_ts(pool, symbol).await?;
    let is_stale = stale_threshold_secs == 0
        || latest_ts.map_or(true, |ts| now_ts.saturating_sub(ts) > stale_threshold_secs);

    if count >= min_candles && !is_stale {
        debug!(symbol, count, min_candles, "sufficient equity candles — skipping backfill");
        return Ok(0);
    }
    if is_stale {
        info!(symbol, count, latest_ts, now_ts, days_old = (now_ts - latest_ts.unwrap_or(0)) / 86400, "equity candles stale — forcing refresh");
    } else {
        info!(symbol, count, min_candles, "insufficient equity candles — starting Yahoo Finance backfill");
    }

    let client = reqwest::Client::builder()
        .use_native_tls()
        .user_agent("Mozilla/5.0 (MarketMarkovNet; equities-wave-a)")
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .context("building reqwest client")?;

    let candles = fetch_chart(&client, symbol, range).await?;
    let fetched = candles.len();
    debug!(symbol, fetched, "Yahoo returned candles");

    let mut last_ts: i64 = 0;
    for c in &candles {
        db::upsert_equity_candle(pool, c)
            .await
            .with_context(|| format!("upsert {} ts={}", symbol, c.ts))?;
        last_ts = last_ts.max(c.ts);
    }

    db::update_ingest_state(pool, "yahoo", symbol, last_ts, fetched as i64, None)
        .await
        .context("update_ingest_state")?;

    Ok(fetched)
}

/// Fetch a single symbol from Yahoo and parse the response.
async fn fetch_chart(
    client: &reqwest::Client,
    symbol: &str,
    range: &str,
) -> Result<Vec<EquityCandle>> {
    let url = format!("{REST_URL}/{symbol}?interval={INTERVAL}&range={range}");
    let mut backoff = BACKOFF_INIT;

    let resp_text = loop {
        let resp = client
            .get(&url)
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => break r.text().await.unwrap_or_default(),
            Ok(r) if r.status().as_u16() == 429 => {
                warn!(symbol, "Yahoo rate-limited (429); sleeping {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(BACKOFF_MAX);
                continue;
            }
            Ok(r) => bail!("Yahoo HTTP {} for {}", r.status(), symbol),
            Err(e) => {
                warn!(symbol, "Yahoo request error: {e:#}; sleeping {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(BACKOFF_MAX);
                if backoff == BACKOFF_MAX {
                    return Err(e).with_context(|| format!("Yahoo fetch {symbol}"));
                }
                continue;
            }
        }
    };

    let v: Value = serde_json::from_str(&resp_text).context("decode Yahoo JSON")?;
    parse_chart(&v, symbol)
}

/// Parse Yahoo chart JSON: `chart.result[0]` has `timestamp[]` and
/// `indicators.quote[0]` with arrays of OHLCV.
fn parse_chart(v: &Value, symbol: &str) -> Result<Vec<EquityCandle>> {
    let result = &v["chart"]["result"];
    if !result.is_array() || result.as_array().unwrap().is_empty() {
        bail!("Yahoo: empty chart.result for {symbol}");
    }
    let result = &result[0];
    let timestamps = result["timestamp"]
        .as_array()
        .context("Yahoo: missing timestamp array")?;
    let quote = &result["indicators"]["quote"][0];
    let opens = quote["open"].as_array().context("Yahoo: missing open")?;
    let highs = quote["high"].as_array().context("Yahoo: missing high")?;
    let lows = quote["low"].as_array().context("Yahoo: missing low")?;
    let closes = quote["close"].as_array().context("Yahoo: missing close")?;
    let volumes = quote["volume"].as_array();

    let mut candles = Vec::with_capacity(timestamps.len());
    for i in 0..timestamps.len() {
        let ts = match timestamps[i].as_i64() {
            Some(t) => t,
            None => continue,
        };
        let o = opens.get(i).and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
        let h = highs.get(i).and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
        let l = lows.get(i).and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
        let c = closes.get(i).and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
        if !(o.is_finite() && h.is_finite() && l.is_finite() && c.is_finite()) {
            debug!(symbol, ts, "skipping row with NaN prices");
            continue;
        }
        let vol = volumes
            .and_then(|v| v.get(i).and_then(|x| x.as_i64()))
            .unwrap_or(0);
        candles.push(EquityCandle {
            symbol: symbol.to_string(),
            ts,
            open: o,
            high: h,
            low: l,
            close: c,
            volume: vol,
            source: "yahoo".to_string(),
        });
    }
    if candles.is_empty() {
        bail!("Yahoo: parsed 0 valid candles for {symbol}");
    }
    Ok(candles)
}

/// Convenience: backfill a list of symbols (sequential, to respect rate limits).
pub async fn backfill_many(
    pool: &DbPool,
    symbols: &[&str],
    min_candles: i64,
    range: &str,
    stale_threshold_secs: i64,
) -> Result<usize> {
    let mut total = 0;
    for s in symbols {
        match backfill(pool, s, min_candles, range, stale_threshold_secs).await {
            Ok(n) => total += n,
            Err(e) => {
                warn!(symbol = s, "backfill error: {e:#}");
                let _ = db::update_ingest_state(pool, "yahoo", s, 0, 0, Some(&format!("{e:#}"))).await;
            }
        }
        // Be polite to Yahoo — 200ms between calls.
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(total)
}

// ─── Live quote ───────────────────────────────────────────────────────────────

/// A thin snapshot of the current market price for one symbol.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Quote {
    pub symbol: String,
    pub price: f64,
    pub prev_close: f64,
    pub change: f64,
    pub change_pct: f64,
    pub timestamp: i64,
}

/// Fetch just the current quote for `symbol` from Yahoo Finance.
/// Uses the same chart endpoint as `backfill` but only parses the `meta`
/// block so it is very fast (no OHLCV history needed).
pub async fn fetch_quote(symbol: &str) -> Result<Quote> {
    let client = reqwest::Client::builder()
        .use_native_tls()
        .user_agent("Mozilla/5.0 (MarketMarkovNet; equities-wave-a)")
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(8))
        .build()
        .context("building reqwest client")?;

    let url = format!("{REST_URL}/{symbol}?interval=1d&range=5d");
    let resp_text = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Yahoo quote request for {symbol}"))?
        .text()
        .await
        .context("read Yahoo quote response")?;

    let v: Value =
        serde_json::from_str(&resp_text).context("decode Yahoo quote JSON")?;

    let result = &v["chart"]["result"];
    if !result.is_array() || result.as_array().unwrap().is_empty() {
        bail!("Yahoo: empty chart result for quote {symbol}");
    }
    let meta = &result[0]["meta"];

    let price = meta["regularMarketPrice"]
        .as_f64()
        .context("missing regularMarketPrice")?;
    let prev_close = meta["chartPreviousClose"]
        .as_f64()
        .or_else(|| meta["regularMarketPreviousClose"].as_f64())
        .unwrap_or(price);
    let change = price - prev_close;
    let change_pct = if prev_close > 0.0 {
        (change / prev_close) * 100.0
    } else {
        0.0
    };
    let timestamp = meta["regularMarketTime"].as_i64().unwrap_or(0);

    Ok(Quote {
        symbol: symbol.to_string(),
        price,
        prev_close,
        change,
        change_pct,
        timestamp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Smoke test: parse a synthetic Yahoo response.
    #[test]
    fn parse_chart_extracts_ohlcv() {
        let payload = json!({
            "chart": {
                "result": [{
                    "meta": {"symbol": "QQQ"},
                    "timestamp": [1_700_000_000, 1_700_086_400, 1_700_172_800],
                    "indicators": {
                        "quote": [{
                            "open":  [400.0, 405.0, 410.0],
                            "high":  [407.0, 409.0, 415.0],
                            "low":   [399.0, 403.0, 408.0],
                            "close": [404.0, 408.0, 414.0],
                            "volume": [1_000_000_i64, 1_100_000, 1_200_000]
                        }]
                    }
                }],
                "error": null
            }
        });
        let candles = parse_chart(&payload, "QQQ").unwrap();
        assert_eq!(candles.len(), 3);
        assert_eq!(candles[0].symbol, "QQQ");
        assert_eq!(candles[0].ts, 1_700_000_000);
        assert!((candles[0].close - 404.0).abs() < 1e-9);
        assert_eq!(candles[0].volume, 1_000_000);
        assert_eq!(candles[0].source, "yahoo");
    }

    /// Skip rows where any price is NaN (Yahoo returns nulls for splits).
    #[test]
    fn parse_chart_skips_nan_rows() {
        let payload = json!({
            "chart": {
                "result": [{
                    "timestamp": [1, 2, 3],
                    "indicators": {
                        "quote": [{
                            "open":  [400.0, null, 410.0],
                            "high":  [407.0, 409.0, 415.0],
                            "low":   [399.0, 403.0, 408.0],
                            "close": [404.0, 408.0, 414.0],
                            "volume": [1, 2, 3]
                        }]
                    }
                }],
                "error": null
            }
        });
        let candles = parse_chart(&payload, "QQQ").unwrap();
        assert_eq!(candles.len(), 2, "middle NaN row should be skipped");
        assert_eq!(candles[0].ts, 1);
        assert_eq!(candles[1].ts, 3);
    }

    /// Empty result → error.
    #[test]
    fn parse_chart_empty_errors() {
        let payload = json!({"chart": {"result": [], "error": null}});
        assert!(parse_chart(&payload, "QQQ").is_err());
    }
}
