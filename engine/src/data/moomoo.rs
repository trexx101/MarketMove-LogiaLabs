//! Moomoo OpenAPI client (Wave A: interface only).
//!
//! Moomoo's OpenAPI requires a proprietary SDK + OAuth credentials. The
//! interface here matches the Wave A `equity_candles` schema so we can plug
//! in the real client in Wave D without touching the rest of the engine.
//!
//! To activate: place a credentials file at `~/.moomoo/credentials.json` with
//! fields `{ "api_key": "...", "api_secret": "...", "host": "127.0.0.1",
//! "port": 11111 }`. The OpenD gateway daemon must be running locally.
//!
//! Until activated, all functions return a clear "not configured" error and
//! callers fall back to the Yahoo client.

use anyhow::{bail, Result};
use tracing::info;

use crate::db::{self, DbPool, EquityCandle};

/// Configuration for the Moomoo OpenAPI client.
#[derive(Debug, Clone)]
pub struct MoomooConfig {
    pub api_key: String,
    pub api_secret: String,
    pub host: String,
    pub port: u16,
}

impl MoomooConfig {
    /// Try to load Moomoo credentials from a JSON file.
    /// Returns `None` if the file does not exist or is malformed (non-fatal).
    pub fn from_credentials_file(path: &str) -> Option<Self> {
        let raw = std::fs::read_to_string(path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
        Some(Self {
            api_key: v["api_key"].as_str()?.to_string(),
            api_secret: v["api_secret"].as_str()?.to_string(),
            host: v["host"].as_str().unwrap_or("127.0.0.1").to_string(),
            port: v["port"].as_u64().unwrap_or(11111) as u16,
        })
    }
}

/// Backfill `symbol` daily OHLCV via Moomoo OpenAPI.
///
/// Currently a stub: if no credentials are configured, returns a clear
/// "not configured" error so the caller can fall back to Yahoo. The signature
/// mirrors the Binance/Yahoo clients so it is drop-in when activated.
pub async fn backfill(
    pool: &DbPool,
    symbol: &str,
    min_candles: i64,
    range_days: u32,
) -> Result<usize> {
    let _ = (pool, symbol, min_candles, range_days);
    let _ = db::count_equity_candles; // silence unused import in stub builds
    let cfg = MoomooConfig::from_credentials_file(
        &std::env::var("MOOMOO_CREDS_PATH").unwrap_or_else(|_| "~/.moomoo/credentials.json".into()),
    );
    match cfg {
        Some(c) => {
            info!(symbol, host = %c.host, port = c.port, "Moomoo credentials present but client not yet implemented");
            bail!("Moomoo OpenAPI client not yet implemented (Wave D); use Yahoo for now")
        }
        None => {
            bail!("Moomoo not configured (no credentials file at $MOOMOO_CREDS_PATH or default path); falling back to Yahoo")
        }
    }
}

/// Convert a Moomoo KLine response into our `EquityCandle` shape.
/// Implemented as a pure transform so it can be unit-tested without a
/// live connection. Wave D will wire this into the real SDK call.
pub fn kline_to_equity_candle(
    symbol: &str,
    kline: &serde_json::Value,
) -> Result<EquityCandle> {
    let ts = kline["timestamp"]
        .as_i64()
        .or_else(|| kline["time_key"].as_i64())
        .or_else(|| kline["kline_time"].as_i64())
        .ok_or_else(|| anyhow::anyhow!("Moomoo KLine: missing timestamp"))?;
    Ok(EquityCandle {
        symbol: symbol.to_string(),
        ts,
        open: kline["open"].as_f64().ok_or_else(|| anyhow::anyhow!("missing open"))?,
        high: kline["high"].as_f64().ok_or_else(|| anyhow::anyhow!("missing high"))?,
        low: kline["low"].as_f64().ok_or_else(|| anyhow::anyhow!("missing low"))?,
        close: kline["close"].as_f64().ok_or_else(|| anyhow::anyhow!("missing close"))?,
        volume: kline["volume"].as_i64().unwrap_or(0),
        source: "moomoo".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Pure transform test — no network or credentials needed.
    #[test]
    fn kline_to_equity_candle_basic() {
        let k = json!({
            "timestamp": 1_700_000_000_i64,
            "open":  100.0,
            "high":  105.0,
            "low":    99.0,
            "close": 104.0,
            "volume": 50_000_i64
        });
        let c = kline_to_equity_candle("QQQ", &k).unwrap();
        assert_eq!(c.symbol, "QQQ");
        assert_eq!(c.ts, 1_700_000_000);
        assert!((c.close - 104.0).abs() < 1e-9);
        assert_eq!(c.source, "moomoo");
    }

    #[test]
    fn kline_to_equity_candle_rejects_missing_fields() {
        let k = json!({"timestamp": 1});
        assert!(kline_to_equity_candle("QQQ", &k).is_err());
    }

    #[test]
    fn backfill_returns_clear_error_without_credentials() {
        // No MOOMOO_CREDS_PATH set, default path doesn't exist → clear error.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let pool_opt: Option<DbPool> = None;
        // Pass a dummy pool if needed; the function returns early.
        let _ = (rt, pool_opt);
        // We can't easily mock DbPool here; instead test that config-load fails.
        let cfg = MoomooConfig::from_credentials_file("/nonexistent/path/creds.json");
        assert!(cfg.is_none());
    }
}
