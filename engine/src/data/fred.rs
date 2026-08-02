//! FRED (Federal Reserve Economic Data) macro fetcher — JSON API v2.
//!
//! FRED provides free daily series for treasury yields and trade-weighted USD.
//! A free API key is required for the JSON API (120 req/min on free tier).
//!
//! Endpoint: `https://api.stlouisfed.org/fred/series/observations?series_id=...&api_key=...&file_type=json`
//!
//! Note: VIX was previously sourced from FRED (series VIXCLS) but is now
//! sourced from CBOE directly (free, no auth, no rate limits). The VIXCLS
//! mapping remains for backward compatibility.
//!
//! Series used:
//!   - VIXCLS   → CBOE VIX — now handled by cboe::backfill_vix
//!   - DGS10    → 10Y Treasury constant-maturity yield
//!   - DTWEXBGS → Trade-weighted USD index (DXY proxy)
//!
//! Stored in `equity_candles` with symbol prefixes: `$VIX`, `$UST10Y`, `$DXY`.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use tracing::{debug, info, warn};

use crate::db::{self, DbPool, EquityCandle};

const FRED_API_URL: &str = "https://api.stlouisfed.org/fred/series/observations";

/// Number of series actually fetched via FRED JSON API (VIX moved to CBOE).
const DEFAULT_MACRO_SERIES: &[&str] = &["$UST10Y", "$DXY"];

/// Map our internal symbol to a FRED series id.
fn series_id(symbol: &str) -> Option<&'static str> {
    match symbol {
        "$VIX" => Some("VIXCLS"),
        "$UST10Y" => Some("DGS10"),
        "$DXY" => Some("DTWEXBGS"),
        _ => None,
    }
}

/// Fetch one macro series via FRED JSON API v2 and upsert into
/// `equity_candles`. Requires `FRED_API_KEY` env var.
///
/// `range_days` controls the `observation_start` filter sent to FRED.
pub async fn backfill_macro(
    pool: &DbPool,
    symbol: &str,
    range_days: u32,
) -> Result<usize> {
    let series = match series_id(symbol) {
        Some(s) => s,
        None => bail!("FRED: unknown macro symbol '{symbol}'"),
    };
    let api_key = std::env::var("FRED_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        bail!(
            "FRED_API_KEY not set — cannot use JSON API \
             (get a free key at https://fred.stlouisfed.org/apikey)"
        );
    }

    let client = reqwest::Client::builder()
        .user_agent("MarketMarkovNet/equities")
        .timeout(std::time::Duration::from_secs(15))
        .connect_timeout(std::time::Duration::from_secs(8))
        .build()
        .context("building reqwest client for FRED JSON API")?;

    let mut url = format!(
        "{FRED_API_URL}?series_id={series}&api_key={api_key}&file_type=json&output_type=1"
    );
    if range_days > 0 {
        let start = chrono::Utc::now() - chrono::Duration::days(range_days as i64);
        url.push_str(&format!("&observation_start={}", start.format("%Y-%m-%d")));
    }

    debug!(symbol, series, url = %url, "FRED JSON API request");

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("FRED GET {series}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        // Truncate body to avoid flooding logs
        let snippet = if body.len() > 200 { &body[..200] } else { &body };
        bail!("FRED HTTP {status} for {symbol} ({series}): {snippet}");
    }

    let v: serde_json::Value = resp.json().await.context("FRED JSON decode")?;

    let candles = parse_fred_json(&v, symbol, range_days)
        .with_context(|| format!("parse FRED JSON for {symbol}"))?;

    let fetched = candles.len();
    info!(symbol, series, fetched, "FRED returned macro points");

    let mut last_ts: i64 = 0;
    for c in &candles {
        db::upsert_equity_candle(pool, c)
            .await
            .with_context(|| format!("upsert {symbol} ts={}", c.ts))?;
        last_ts = last_ts.max(c.ts);
    }

    Ok(fetched)
}

/// Parse a FRED JSON API v2 response into EquityCandle rows.
///
/// Expects: `{ "observations": [{ "date": "2024-01-02", "value": "13.42" }, ...] }`
/// Missing values are encoded as `"."` — we skip them.
fn parse_fred_json(
    v: &serde_json::Value,
    symbol: &str,
    range_days: u32,
) -> Result<Vec<EquityCandle>> {
    let observations = v["observations"]
        .as_array()
        .context("FRED: missing observations array")?;

    let cutoff_ts: i64 = if range_days > 0 {
        chrono::Utc::now().timestamp() - (range_days as i64) * 86_400
    } else {
        0
    };

    let mut rows = Vec::new();
    for obs in observations {
        let date = obs["date"].as_str().unwrap_or("");
        let val_str = obs["value"].as_str().unwrap_or(".");
        if val_str == "." || val_str.is_empty() {
            continue;
        }
        let close: f64 = match val_str.parse() {
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
            open: close,
            high: close,
            low: close,
            close,
            volume: 0,
            source: "fred".to_string(),
        });
    }

    if rows.is_empty() {
        bail!("FRED JSON: parsed 0 valid rows for {symbol}");
    }
    Ok(rows)
}

/// Convenience: backfill the default macro series (DGS10, DTWEXBGS).
/// VIX is now sourced from CBOE, not FRED.
pub async fn backfill_all_default_macros(
    pool: &DbPool,
    range_days: u32,
) -> Result<usize> {
    let mut total = 0;
    for sym in DEFAULT_MACRO_SERIES {
        match backfill_macro(pool, sym, range_days).await {
            Ok(n) => {
                total += n;
            }
            Err(e) => {
                warn!("FRED {sym}: {e:#}");
                let _ = db::update_ingest_state(
                    pool, "fred", sym, 0, 0, Some(&format!("{e:#}")),
                )
                .await;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_mock_json(data: &[(&str, &str)]) -> serde_json::Value {
        let observations: Vec<serde_json::Value> = data
            .iter()
            .map(|(d, v)| {
                serde_json::json!({"date": d, "value": v})
            })
            .collect();
        serde_json::json!({"observations": observations})
    }

    #[test]
    fn parse_fred_json_handles_missing_and_normal() {
        let json = build_mock_json(&[
            ("2024-01-02", "13.42"),
            ("2024-01-03", "."),
            ("2024-01-04", "13.27"),
        ]);
        let rows = parse_fred_json(&json, "$VIX", 0).unwrap();
        assert_eq!(rows.len(), 2, "should skip the missing row");
        assert_eq!(rows[0].symbol, "$VIX");
        assert_eq!(rows[0].close, 13.42);
        assert_eq!(rows[0].source, "fred");
        assert!(rows[0].ts < rows[1].ts);
    }

    #[test]
    fn parse_fred_json_filters_by_range() {
        let json = build_mock_json(&[
            ("2020-01-02", "12.0"),
            ("2024-01-02", "13.0"),
            ("2024-06-02", "15.0"),
        ]);
        // Wide range: all 3 rows kept.
        let rows = parse_fred_json(&json, "$VIX", 10_000).unwrap();
        assert_eq!(rows.len(), 3);
        // Narrow range (1 day): should bail (0 valid rows from 2020).
        assert!(parse_fred_json(&json, "$VIX", 1).is_err());
    }

    #[test]
    fn parse_fred_json_empty_value_skipped() {
        let json = build_mock_json(&[("2024-01-02", "")]);
        assert!(parse_fred_json(&json, "$VIX", 0).is_err());
    }

    #[test]
    fn dgs10_maps_to_treasury_yield() {
        assert_eq!(series_id("$UST10Y"), Some("DGS10"));
    }

    #[test]
    fn unknown_symbol_returns_none() {
        assert_eq!(series_id("QQQ"), None);
    }
}