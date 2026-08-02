//! CBOE VIX historical data fetcher (free, no auth, no rate limits).
//!
//! Endpoint: https://cdn.cboe.com/api/global/us_indices/daily_prices/VIX_History.csv
//! Format: DATE,OPEN,HIGH,LOW,CLOSE — daily, 1990 to present, updated daily.
//!
//! Stored in equity_candles as symbol "$VIX" (same slot FRED used).
//!
//! CBOE also provides a live quote endpoint:
//!   https://cdn.cboe.com/api/global/us_indices/daily_prices/vix.json
//! (always 15-min delayed; acceptable for daily macro context).

use anyhow::{Context, Result};
use tracing::{debug, info};

use crate::db::{self, DbPool, EquityCandle};

const CBOE_VIX_URL: &str =
    "https://cdn.cboe.com/api/global/us_indices/daily_prices/VIX_History.csv";

/// Fetch VIX daily history from CBOE and upsert into equity_candles as "$VIX".
/// `range_days` caps the lookback (0 = all history, ~11k rows).
pub async fn backfill_vix(pool: &DbPool, range_days: u32) -> Result<usize> {
    let client = reqwest::Client::builder()
        .user_agent("MarketMarkovNet/equities")
        .timeout(std::time::Duration::from_secs(15))
        .connect_timeout(std::time::Duration::from_secs(8))
        .build()
        .context("building reqwest client for CBOE")?;

    let body = client
        .get(CBOE_VIX_URL)
        .send()
        .await
        .context("CBOE VIX HTTP request")?
        .text()
        .await
        .context("CBOE VIX response body")?;

    let candles = parse_cboe_csv(&body, "$VIX", range_days)?;
    let fetched = candles.len();
    debug!(fetched, "CBOE VIX parsed rows");

    let mut last_ts: i64 = 0;
    for c in &candles {
        db::upsert_equity_candle(pool, c)
            .await
            .with_context(|| format!("upsert $VIX ts={}", c.ts))?;
        last_ts = last_ts.max(c.ts);
    }

    info!(fetched, last_ts, "CBOE VIX backfill complete");
    Ok(fetched)
}

/// Parse CBOE CSV: DATE,OPEN,HIGH,LOW,CLOSE
fn parse_cboe_csv(body: &str, symbol: &str, range_days: u32) -> Result<Vec<EquityCandle>> {
    let mut rows = Vec::new();
    let cutoff_ts: i64 = if range_days > 0 {
        let now = chrono::Utc::now().timestamp();
        now - (range_days as i64) * 86_400
    } else {
        0
    };

    for (i, line) in body.lines().enumerate() {
        if i == 0 {
            continue; // header
        }
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 5 {
            continue;
        }
        let date = parts[0].trim();
        let close: f64 = match parts[4].trim().parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let open: f64 = parts[1].trim().parse().unwrap_or(close);
        let high: f64 = parts[2].trim().parse().unwrap_or(close);
        let low: f64 = parts[3].trim().parse().unwrap_or(close);

        let ts = match chrono::NaiveDate::parse_from_str(date, "%m/%d/%Y")
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
            open,
            high,
            low,
            close,
            volume: 0,
            source: "cboe".to_string(),
        });
    }

    if rows.is_empty() {
        anyhow::bail!("CBOE VIX: parsed 0 valid rows");
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cboe_csv_basic() {
        let csv = "DATE,OPEN,HIGH,LOW,CLOSE\n01/02/2024,13.0,14.0,12.5,13.42\n01/03/2024,13.5,14.5,13.0,13.27\n";
        let rows = parse_cboe_csv(csv, "$VIX", 0).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].symbol, "$VIX");
        assert!((rows[0].close - 13.42).abs() < 1e-9);
        assert_eq!(rows[0].source, "cboe");
    }

    #[test]
    fn parse_cboe_csv_filters_by_range() {
        let csv = "DATE,OPEN,HIGH,LOW,CLOSE\n01/02/2020,12.0,13.0,11.0,12.5\n01/02/2024,13.0,14.0,12.5,13.42\n";
        assert!(parse_cboe_csv(csv, "$VIX", 1).is_err());
        // All rows older than 1 day → filtered out → empty → error
    }
}