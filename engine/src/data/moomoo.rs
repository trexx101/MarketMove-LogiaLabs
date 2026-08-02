//! Moomoo OpenAPI equity data client (Phase 4: data consolidation).
//!
//! Shells out to Python scripts in `.agents/skills/moomooapi/scripts/quote/`
//! — same pattern as `exec/moomoo.rs` shells out to `trade/place_order.py`.
//! No Rust SDK exists for the OpenD TCP gateway; the Python `moomoo` package
//! is the only maintained client.
//!
//! Requires: OpenD gateway running at `$FUTU_OPEND_HOST:$FUTU_OPEND_PORT`
//! (defaults to 127.0.0.1:11111). The Python `common.py` module handles
//! OpenD connectivity checks and SDK version validation.
//!
//! API limits (per Moomoo docs):
//! - Historical K-line: 60 req/30s, daily K up to 20y history
//! - Market snapshot:   60 req/30s, max 400 symbols per call
//! - Historical quota:  each unique stock within 30 days uses 1 quota unit

use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use tokio::process::Command;
use tracing::{info, warn};

use crate::db::{self, DbPool, EquityCandle};

// ── Script paths ────────────────────────────────────────────────

/// Script path: `<repo>/.agents/skills/moomooapi/scripts/quote/get_kline.py`
fn get_kline_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join(".agents/skills/moomooapi/scripts/quote/get_kline.py")
}

/// Script path: `<repo>/.agents/skills/moomooapi/scripts/quote/get_snapshot.py`
fn get_snapshot_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join(".agents/skills/moomooapi/scripts/quote/get_snapshot.py")
}

// ── OpenD availability check ────────────────────────────────────

/// Check if the OpenD TCP gateway is reachable.
///
/// Does a quick `TcpStream::connect` to the host/port configured via
/// `FUTU_OPEND_HOST` / `FUTU_OPEND_PORT` env vars.
pub async fn is_available() -> bool {
    let host =
        std::env::var("FUTU_OPEND_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("FUTU_OPEND_PORT")
        .unwrap_or_else(|_| "11111".into())
        .parse()
        .unwrap_or(11111);
    tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .is_ok()
}

// ── Symbol conversion ───────────────────────────────────────────

/// Convert a ticker to Moomoo code format: "QQQ" → "US.QQQ".
/// Already-prefixed codes ("US.AAPL", "HK.00700") pass through unchanged.
fn to_moomoo_code(symbol: &str) -> String {
    if symbol.starts_with("US.") || symbol.starts_with("HK.") {
        symbol.to_string()
    } else {
        format!("US.{symbol}")
    }
}

// ── Historical K-line backfill ──────────────────────────────────

/// Backfill equity OHLCV via Moomoo `get_kline.py` (historical daily candles).
///
/// Fetches at least `min_candles` bars (daily, forward-adjusted) spanning
/// `range_days` lookback. The Python script auto-paginates beyond 1000 bars.
///
/// Upserts into `equity_candles` with `source = "moomoo"`.
pub async fn backfill(
    pool: &DbPool,
    symbol: &str,
    min_candles: i64,
    range_days: u32,
) -> Result<usize> {
    let count = std::cmp::max(min_candles, 250) as u32;
    let code = to_moomoo_code(symbol);

    let start = if range_days > 0 {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(range_days as i64);
        Some(cutoff.format("%Y-%m-%d").to_string())
    } else {
        None
    };

    let script = get_kline_script();
    let mut cmd = Command::new("python3");
    cmd.arg(&script)
        .arg(&code)
        .arg("--ktype")
        .arg("1d")
        .arg("--num")
        .arg(count.to_string())
        .arg("--rehab")
        .arg("forward")
        .arg("--json");
    if let Some(ref s) = start {
        cmd.arg("--start").arg(s);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = cmd.output().await.context("spawning get_kline.py")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("get_kline.py failed (code={code}): {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let resp: KlineResponse = serde_json::from_str(&stdout)
        .with_context(|| format!("decode get_kline.py JSON for {code}"))?;

    let mut candles = Vec::new();
    for r in &resp.data {
        let ts = parse_time_key(&r.time)
            .with_context(|| format!("parse time_key '{}'", r.time))?;
        candles.push(EquityCandle {
            symbol: symbol.to_string(),
            ts,
            open: r.open,
            high: r.high,
            low: r.low,
            close: r.close,
            volume: r.volume,
            source: "moomoo".to_string(),
        });
    }

    let fetched = candles.len();
    info!(symbol, code = %code, fetched, "Moomoo kline fetched");

    for c in &candles {
        db::upsert_equity_candle(pool, c)
            .await
            .with_context(|| format!("upsert {symbol} ts={}", c.ts))?;
    }

    Ok(fetched)
}

// ── Live quote ──────────────────────────────────────────────────

/// A live quote from Moomoo's `get_snapshot.py`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Quote {
    pub symbol: String,
    pub price: f64,
    pub prev_close: f64,
    pub change: f64,
    pub change_pct: f64,
    pub timestamp: i64,
}

/// Fetch a live quote via Moomoo `get_snapshot.py`.
///
/// Returns `Quote` with last_price, prev_close, and derived change fields.
pub async fn fetch_quote(symbol: &str) -> Result<Quote> {
    let code = to_moomoo_code(symbol);
    let script = get_snapshot_script();

    let output = Command::new("python3")
        .arg(&script)
        .arg(&code)
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("spawning get_snapshot.py")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("get_snapshot.py failed (code={code}): {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let resp: SnapshotResponse = serde_json::from_str(&stdout)
        .with_context(|| format!("decode get_snapshot.py JSON for {code}"))?;

    let row = resp
        .data
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("get_snapshot.py returned no data for {code}"))?;

    let price = row.last_price;
    let prev_close = row.prev_close;
    let change = price - prev_close;
    let change_pct = if prev_close > 0.0 {
        (change / prev_close) * 100.0
    } else {
        0.0
    };

    Ok(Quote {
        symbol: symbol.to_string(),
        price,
        prev_close,
        change,
        change_pct,
        timestamp: chrono::Utc::now().timestamp(),
    })
}

// ── JSON response types ─────────────────────────────────────────

#[derive(Deserialize)]
struct KlineResponse {
    #[serde(default)]
    data: Vec<KlineRow>,
}

#[derive(Deserialize)]
struct KlineRow {
    time: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: i64,
}

#[derive(Deserialize)]
struct SnapshotResponse {
    #[serde(default)]
    data: Vec<SnapshotRow>,
}

#[derive(Deserialize)]
struct SnapshotRow {
    last_price: f64,
    prev_close: f64,
}

// ── Time parsing ────────────────────────────────────────────────

/// Parse Moomoo time_key "2024-01-02 00:00:00" (US Eastern) → Unix
/// timestamp.
///
/// Daily candles have HH:mm:ss == "00:00:00". We parse as UTC midnight;
/// the ±5h Eastern offset doesn't matter for daily bar alignment.
fn parse_time_key(time_key: &str) -> Result<i64> {
    chrono::NaiveDateTime::parse_from_str(time_key, "%Y-%m-%d %H:%M:%S")
        .map(|dt| dt.and_utc().timestamp())
        .with_context(|| format!("parse time_key: {}", time_key))
}

// ── Legacy helpers (for tests / backward compat) ────────────────

/// Convert a generic JSON K-line object to an EquityCandle.
/// Kept for backward compatibility with tests that use synthetic JSON.
pub fn kline_to_equity_candle(
    symbol: &str,
    kline: &serde_json::Value,
) -> Result<EquityCandle> {
    let ts = kline["timestamp"]
        .as_i64()
        .or_else(|| kline["time_key"].as_i64())
        .or_else(|| kline["kline_time"].as_i64())
        .ok_or_else(|| anyhow!("Moomoo KLine: missing timestamp"))?;
    Ok(EquityCandle {
        symbol: symbol.to_string(),
        ts,
        open: kline["open"]
            .as_f64()
            .ok_or_else(|| anyhow!("missing open"))?,
        high: kline["high"]
            .as_f64()
            .ok_or_else(|| anyhow!("missing high"))?,
        low: kline["low"]
            .as_f64()
            .ok_or_else(|| anyhow!("missing low"))?,
        close: kline["close"]
            .as_f64()
            .ok_or_else(|| anyhow!("missing close"))?,
        volume: kline["volume"].as_i64().unwrap_or(0),
        source: "moomoo".to_string(),
    })
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_moomoo_code_adds_us_prefix() {
        assert_eq!(to_moomoo_code("QQQ"), "US.QQQ");
        assert_eq!(to_moomoo_code("US.AAPL"), "US.AAPL");
        assert_eq!(to_moomoo_code("HK.00700"), "HK.00700");
    }

    #[test]
    fn parse_time_key_basic() {
        let ts = parse_time_key("2024-01-02 00:00:00").unwrap();
        // 2024-01-02 00:00:00 UTC = 1704153600
        assert_eq!(ts, 1704153600);
    }

    #[test]
    fn kline_to_equity_candle_timestamp_field() {
        let k = serde_json::json!({
            "timestamp": 1_700_000_000_i64,
            "open": 100.0, "high": 105.0, "low": 99.0,
            "close": 104.0, "volume": 50_000_i64
        });
        let c = kline_to_equity_candle("QQQ", &k).unwrap();
        assert_eq!(c.symbol, "QQQ");
        assert_eq!(c.source, "moomoo");
    }

    #[test]
    fn kline_to_equity_candle_time_key_field() {
        let k = serde_json::json!({
            "time_key": 1_700_000_001_i64,
            "open": 200.0, "high": 205.0, "low": 199.0,
            "close": 204.0, "volume": 10_000_i64
        });
        let c = kline_to_equity_candle("AAPL", &k).unwrap();
        assert_eq!(c.ts, 1_700_000_001);
    }
}