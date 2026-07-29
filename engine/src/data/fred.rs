//! FRED (Federal Reserve Economic Data) macro fetcher.
//!
//! FRED provides free daily series for VIX, 10Y Treasury yield, DXY, etc.
//! No rate limits beyond courtesy; an API key is optional for higher quotas.
//!
//! Endpoint: `https://api.stlouisfed.org/fred/series/observations?series_id=...&file_type=json`
//! Fallback (no key): `https://fred.stlouisfed.org/graph/fredgraph.csv?id=...`
//!
//! Series used in Wave A:
//!   - VIXCLS   → CBOE VIX (daily close)
//!   - DGS10    → 10Y Treasury constant-maturity yield
//!   - DTWEXBGS → Trade-weighted USD index (DXY proxy)
//!
//! For the equities engine, FRED data is stored in `equity_candles` with
/// symbol prefixes: `$VIX`, `$UST10Y`, `$DXY`.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use tracing::{debug, info, warn};

use crate::db::{self, DbPool, EquityCandle};

const FRED_CSV_URL: &str = "https://fred.stlouisfed.org/graph/fredgraph.csv";

/// Map our internal symbol to a FRED series id.
fn series_id(symbol: &str) -> Option<&'static str> {
    match symbol {
        "$VIX" => Some("VIXCLS"),
        "$UST10Y" => Some("DGS10"),
        "$DXY" => Some("DTWEXBGS"),
        _ => None,
    }
}

/// Fetch one macro series and upsert into `equity_candles` with the given
/// `symbol` (e.g. `$VIX`). FRED CSV has two columns: `DATE,<value>`.
///
/// `range_days` controls the lookback window (FRED returns the whole
/// history by default; we cap it locally).
pub async fn backfill_macro(
    pool: &DbPool,
    symbol: &str,
    range_days: u32,
) -> Result<usize> {
    let series = match series_id(symbol) {
        Some(s) => s,
        None => bail!("FRED: unknown macro symbol '{symbol}'"),
    };

    // FRED's Akamai edge has been observed to hang indefinitely from this
    // VPS (no IPv6 route, Akamai's Anycast responses stall the SYN). The
    // backfill machinery logs the timeout and falls back to Yahoo ^VIX for
    // $VIX; the other two series (DGS10, DTWEXBGS) are macro-only and
    // degrade to 0.0 in features. Keep the timeout short so the failure is
    // cheap.
    let client = reqwest::Client::builder()
        .user_agent("MarketMarkovNet/equities")
        .timeout(std::time::Duration::from_secs(5))
        .connect_timeout(std::time::Duration::from_secs(3))
        .build()
        .context("building reqwest client")?;

    let url = format!("{FRED_CSV_URL}?id={series}");
    debug!(symbol, series, "FRED CSV request");

    let body = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("FRED GET {url}"))?
        .text()
        .await
        .context("FRED body decode")?;

    let candles = parse_fred_csv(&body, symbol, range_days)?;
    let fetched = candles.len();
    info!(symbol, series, fetched, "FRED returned macro points");

    let mut last_ts: i64 = 0;
    for c in &candles {
        db::upsert_equity_candle(pool, c)
            .await
            .with_context(|| format!("upsert {symbol} ts={}", c.ts))?;
        last_ts = last_ts.max(c.ts);
    }
    db::update_ingest_state(pool, "fred", symbol, last_ts, fetched as i64, None)
        .await
        .context("update_ingest_state")?;

    Ok(fetched)
}

/// Parse a FRED CSV response. Format:
///   DATE,VIXCLS
///   2024-01-02,13.42
///   2024-01-03,13.27
///   ...
/// Missing values are encoded as `.` (period). We treat them as NaN → skip.
fn parse_fred_csv(body: &str, symbol: &str, range_days: u32) -> Result<Vec<EquityCandle>> {
    let mut rows: Vec<EquityCandle> = Vec::new();
    let mut cutoff_ts: i64 = 0;
    if range_days > 0 {
        let now = chrono::Utc::now().timestamp();
        cutoff_ts = now - (range_days as i64) * 86_400;
    }

    for (i, line) in body.lines().enumerate() {
        if i == 0 {
            // header — skip
            continue;
        }
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 2 {
            continue;
        }
        let date = parts[0].trim();
        let val = parts[1].trim();
        if val == "." || val.is_empty() {
            continue;
        }
        let close: f64 = match val.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ts = match chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc().timestamp()))
        {
            Some(t) => t,
            None => continue,
        };
        if cutoff_ts > 0 && ts < cutoff_ts {
            continue;
        }
        rows.push(EquityCandle {
            symbol: symbol.to_string(),
            ts,
            open: close, // single-value series — open=high=low=close
            high: close,
            low: close,
            close,
            volume: 0,
            source: "fred".to_string(),
        });
    }
    if rows.is_empty() {
        bail!("FRED: parsed 0 valid rows for {symbol}");
    }
    Ok(rows)
}

/// Convenience: backfill all three default macro series.
pub async fn backfill_all_default_macros(pool: &DbPool, range_days: u32) -> Result<usize> {
    let mut total = 0;
    for sym in &["$VIX", "$UST10Y", "$DXY"] {
        match backfill_macro(pool, sym, range_days).await {
            Ok(n) => {
                total += n;
                warn_unused(range_days);
            }
            Err(e) => {
                warn!("FRED {sym}: {e:#}");
                let _ = db::update_ingest_state(pool, "fred", sym, 0, 0, Some(&format!("{e:#}"))).await;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(total)
}

fn warn_unused(_range: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a synthetic FRED CSV with header, valid rows, and a `.` (missing) row.
    #[test]
    fn parse_fred_csv_handles_missing_and_header() {
        let csv = "DATE,VIXCLS\n2024-01-02,13.42\n2024-01-03,.\n2024-01-04,13.27\n";
        let rows = parse_fred_csv(csv, "$VIX", 0).unwrap();
        assert_eq!(rows.len(), 2, "should skip the missing row");
        assert_eq!(rows[0].symbol, "$VIX");
        assert_eq!(rows[0].close, 13.42);
        assert_eq!(rows[0].source, "fred");
        assert!(rows[0].ts < rows[1].ts);
    }

    #[test]
    fn parse_fred_csv_filters_by_range() {
        let csv = "DATE,VIXCLS\n2020-01-02,12.0\n2024-01-02,13.0\n2024-06-02,15.0\n";
        // Wide range: all 3 rows kept.
        let rows = parse_fred_csv(csv, "$VIX", 10_000).unwrap();
        assert_eq!(rows.len(), 3, "all rows kept with 10k-day range");

        // 1-day range: all 3 sample rows are years old → filtered out → Err.
        assert!(parse_fred_csv(csv, "$VIX", 1).is_err());
    }

    #[test]
    fn series_id_maps_known_symbols() {
        assert_eq!(series_id("$VIX"), Some("VIXCLS"));
        assert_eq!(series_id("$UST10Y"), Some("DGS10"));
        assert_eq!(series_id("$DXY"), Some("DTWEXBGS"));
        assert_eq!(series_id("QQQ"), None);
    }
}
