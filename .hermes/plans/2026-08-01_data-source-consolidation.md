# Data Source Consolidation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Replace unreliable Yahoo Finance + FRED with Moomoo OpenAPI (equities), CBOE (VIX), FRED JSON API (macro), and stub Finnhub (sentiment) — with a clean provider abstraction that supports both paper and live trading.

**Architecture:** A `DataSource` trait abstracts each provider. Moomoo is the primary equity provider (via Python subprocess to existing `get_kline.py` / `get_snapshot.py` scripts). CBOE and FRED are direct HTTP. Yahoo remains as fallback. The scheduler and chart endpoint call through the abstraction, not directly to `yahoo::`.

**Tech Stack:** Rust (engine), Python subprocess (Moomoo OpenD SDK), reqwest (CBOE/FRED), SQLite (equity_candles table)

---

## Current State

- `engine/src/data/yahoo.rs` — primary equity OHLCV + live quote (rate-limited)
- `engine/src/data/fred.rs` — macro CSV endpoint (100% timeout from VPS)
- `engine/src/data/moomoo.rs` — stub, returns "not configured" error
- `engine/src/data/mod.rs` — `backfill_equities()` calls yahoo then fred then yahoo-vix-fallback
- `engine/src/api/chart.rs` — calls `yahoo::backfill()` + `yahoo::fetch_quote()` directly
- `engine/src/api/quote.rs` — calls `yahoo::fetch_quote()` directly
- `engine/src/config.rs` — has `fred_api_key` and `moomoo_creds_path` fields (lines 66-76)
- `.agents/skills/moomooapi/scripts/quote/get_kline.py` — fully implemented, outputs JSON
- `.agents/skills/moomooapi/scripts/quote/get_snapshot.py` — fully implemented, outputs JSON
- `.agents/skills/moomooapi/scripts/common.py` — reads `FUTU_OPEND_HOST`/`FUTU_OPEND_PORT` env vars
- `engine/src/exec/moomoo.rs` — already shells out to Python scripts for trade execution

## Key Design Decisions

1. **Moomoo data via Python subprocess** (same pattern as `exec/moomoo.rs` shells out to `place_order.py`). No Rust SDK exists; the Python `moomoo` package is the only maintained client for the OpenD TCP gateway.

2. **Yahoo stays as fallback** — if OpenD is unreachable, the engine degrades to Yahoo (current behavior). This ensures paper trading works even without OpenD running.

3. **CBOE VIX replaces FRED for VIX** — free CSV from `cdn.cboe.com`, no auth, no rate limits. FRED JSON API v2 (with the user's free API key) is used for DGS10 and DTWEXBGS only.

4. **FRED JSON API v2 replaces FRED CSV** — `api.stlouisfed.org/fred/series/observations` is a different domain from the timing-out `fred.stlouisfed.org/graph/fredgraph.csv`. The free API key enables 120 req/min.

5. **Sentiment is stub-only** — add the DB table + API endpoint + a no-op fetcher. Wire the real Finnhub call later.

6. **Moomoo works for both paper and live** — the same OpenD gateway serves both. Paper mode uses `MOOMOO_TRD_ENV=SIMULATE`; live uses `REAL`. Data fetch is identical either way.

---

## Task 1: Add `FRED_API_KEY` to docker-compose and .env.example

**Objective:** Wire the FRED API key into the deploy config so the engine can read it at runtime.

**Files:**
- Modify: `deploy/docker-compose.yml` (engine service env block, ~line 92-111)
- Modify: `.env.example` (add FRED_API_KEY line after MOOMOO_TRD_ENV block)

**Steps:**

1. Add `FRED_API_KEY: ${FRED_API_KEY:-}` to the engine environment in docker-compose.yml
2. Add `# FRED_API_KEY=` to .env.example with a comment explaining it's free from https://fred.stlouisfed.org/apikey
3. Verify: `cargo check -p engine` passes (config.rs already reads `FRED_API_KEY` at line 223)

**Verification:**
```bash
cargo check -p engine 2>&1 | grep "^error" | head -5
# Expected: no errors
```

---

## Task 2: Create CBOE VIX data source module

**Objective:** Replace the FRED VIX dependency with a direct CBOE CSV fetch that has no auth or rate limits.

**Files:**
- Create: `engine/src/data/cboe.rs`

**Implementation:**

```rust
//! CBOE VIX historical data fetcher (free, no auth, no rate limits).
//!
//! Endpoint: https://cdn.cboe.com/api/global/us_indices/daily_prices/VIX_History.csv
//! Format: DATE,OPEN,HIGH,LOW,CLOSE — daily, 1990 to present, updated daily.
//!
//! Stored in equity_candles as symbol "$VIX" (same slot FRED used).

use anyhow::{Context, Result};
use tracing::{debug, info};

use crate::db::{self, DbPool, EquityCandle};

const CBOE_VIX_URL: &str = "https://cdn.cboe.com/api/global/us_indices/daily_prices/VIX_History.csv";

/// Fetch VIX daily history from CBOE and upsert into equity_candles as "$VIX".
/// `range_days` caps the lookback (0 = all history).
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
    let mut cutoff_ts: i64 = 0;
    if range_days > 0 {
        let now = chrono::Utc::now().timestamp();
        cutoff_ts = now - (range_days as i64) * 86_400;
    }

    for (i, line) in body.lines().enumerate() {
        if i == 0 { continue; } // header
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 5 { continue; }
        let date = parts[0].trim();
        let close: f64 = match parts[4].trim().parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let open: f64 = parts[1].trim().parse().unwrap_or(close);
        let high: f64 = parts[2].trim().parse().unwrap_or(close);
        let low: f64 = parts[3].trim().parse().unwrap_or(close);

        let ts = match chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc().timestamp()))
        {
            Some(t) => t,
            None => continue,
        };
        if cutoff_ts > 0 && ts < cutoff_ts { continue; }

        rows.push(EquityCandle {
            symbol: symbol.to_string(),
            ts,
            open, high, low, close,
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
        let csv = "DATE,OPEN,HIGH,LOW,CLOSE\n2024-01-02,13.0,14.0,12.5,13.42\n2024-01-03,13.5,14.5,13.0,13.27\n";
        let rows = parse_cboe_csv(csv, "$VIX", 0).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].symbol, "$VIX");
        assert!((rows[0].close - 13.42).abs() < 1e-9);
        assert_eq!(rows[0].source, "cboe");
    }

    #[test]
    fn parse_cboe_csv_filters_by_range() {
        let csv = "DATE,OPEN,HIGH,LOW,CLOSE\n2020-01-02,12.0,13.0,11.0,12.5\n2024-01-02,13.0,14.0,12.5,13.42\n";
        assert!(parse_cboe_csv(csv, "$VIX", 1).is_err()); // all rows older than 1 day
    }
}
```

**Verification:**
```bash
cargo test -p engine cboe -- --nocapture
cargo check -p engine 2>&1 | grep "^error"
# Expected: 2 tests pass, 0 errors
```

---

## Task 3: Rewrite FRED module to use JSON API v2 with API key

**Objective:** Replace the timing-out CSV endpoint with the JSON API that uses a different domain and supports the free API key.

**Files:**
- Modify: `engine/src/data/fred.rs` (rewrite `backfill_macro` to use JSON API)

**Implementation:**

Replace the CSV URL and `backfill_macro` function body with:

```rust
const FRED_API_URL: &str = "https://api.stlouisfed.org/fred/series/observations";

/// Fetch one macro series via FRED JSON API v2.
/// Requires FRED_API_KEY env var (free from https://fred.stlouisfed.org/apikey).
pub async fn backfill_macro(
    pool: &DbPool,
    symbol: &str,
    range_days: u32,
) -> Result<usize> {
    let series = match series_id(symbol) {
        Some(s) => s,
        None => bail!("FRED: unknown macro symbol '{symbol}'"),
    };
    let api_key = std::env::var("FRED_API_KEY")
        .unwrap_or_else(|_| "".to_string());
    if api_key.is_empty() {
        bail!("FRED_API_KEY not set — cannot use JSON API (get free key at https://fred.stlouisfed.org/apikey)");
    }

    let client = reqwest::Client::builder()
        .user_agent("MarketMarkovNet/equities")
        .timeout(std::time::Duration::from_secs(15))
        .connect_timeout(std::time::Duration::from_secs(8))
        .build()
        .context("building reqwest client")?;

    // Build URL with observation_start for range filter
    let observation_start = if range_days > 0 {
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::days(range_days as i64);
        cutoff.format("%Y-%m-%d").to_string()
    } else {
        String::new()
    };

    let mut url = format!(
        "{FRED_API_URL}?series_id={series}&api_key={api_key}&file_type=json&output_type=1"
    );
    if !observation_start.is_empty() {
        url.push_str(&format!("&observation_start={observation_start}"));
    }

    debug!(symbol, series, "FRED JSON API request");

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("FRED GET {url}"))?;

    if !resp.status().is_success() {
        bail!("FRED HTTP {} for {symbol}", resp.status());
    }

    let v: serde_json::Value = resp
        .json()
        .await
        .context("FRED JSON decode")?;

    let observations = v["observations"]
        .as_array()
        .context("FRED: missing observations array")?;

    let cutoff_ts: i64 = if range_days > 0 {
        chrono::Utc::now().timestamp() - (range_days as i64) * 86_400
    } else {
        0
    };

    let mut candles = Vec::new();
    for obs in observations {
        let date = obs["date"].as_str().unwrap_or("");
        let val_str = obs["value"].as_str().unwrap_or(".");
        if val_str == "." || val_str.is_empty() { continue; }
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
        if cutoff_ts > 0 && ts < cutoff_ts { continue; }
        candles.push(EquityCandle {
            symbol: symbol.to_string(),
            ts,
            open: close, high: close, low: close, close,
            volume: 0,
            source: "fred".to_string(),
        });
    }

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
```

**Tests:** Update existing `parse_fred_csv` tests to test the JSON parse path instead. Add a `parse_fred_json` function that takes `&serde_json::Value` and test it with synthetic JSON.

**Verification:**
```bash
cargo test -p engine fred -- --nocapture
cargo check -p engine 2>&1 | grep "^error"
```

---

## Task 4: Implement Moomoo data fetcher (Python subprocess)

**Objective:** Replace the stub `moomoo.rs` with a real data fetcher that shells out to the existing `get_kline.py` and `get_snapshot.py` Python scripts — same pattern as `exec/moomoo.rs`.

**Files:**
- Modify: `engine/src/data/moomoo.rs` (replace stub with real implementation)

**Implementation:**

```rust
//! Moomoo OpenAPI equity data client (Phase 4: data consolidation).
//!
//! Shells out to Python scripts in `.agents/skills/moomooapi/scripts/quote/`
//! — same pattern as `exec/moomoo.rs` shells out to `trade/place_order.py`.
//! No Rust SDK exists for the OpenD TCP gateway; the Python `moomoo` package
//! is the only maintained client.
//!
//! Requires: OpenD gateway running at $FUTU_OPEND_HOST:$FUTU_OPEND_PORT
//! (defaults to 127.0.0.1:11111).

use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use tokio::process::Command;
use tracing::{info, warn};

use crate::db::{self, DbPool, EquityCandle};

/// Script path: <repo>/.agents/skills/moomooapi/scripts/quote/get_kline.py
fn get_kline_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join(".agents/skills/moomooapi/scripts/quote/get_kline.py")
}

/// Script path: <repo>/.agents/skills/moomooapi/scripts/quote/get_snapshot.py
fn get_snapshot_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join(".agents/skills/moomooapi/scripts/quote/get_snapshot.py")
}

/// Convert a Moomoo ticker (e.g. "QQQ") to Moomoo code (e.g. "US.QQQ").
fn to_moomoo_code(symbol: &str) -> String {
    if symbol.starts_with("US.") || symbol.starts_with("HK.") {
        symbol.to_string()
    } else {
        format!("US.{symbol}")
    }
}

/// Check if OpenD is reachable (TCP connect test).
pub async fn is_available() -> bool {
    let host = std::env::var("FUTU_OPEND_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("FUTU_OPEND_PORT")
        .unwrap_or_else(|_| "11111".into())
        .parse()
        .unwrap_or(11111);
    tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .is_ok()
}

/// Backfill equity OHLCV via Moomoo `get_kline.py` (historical daily candles).
///
/// Fetches `count` daily candles (max 1000 per page, auto-paginated by the script).
/// Upserts into equity_candles with `source = "moomoo"`.
pub async fn backfill(
    pool: &DbPool,
    symbol: &str,
    min_candles: i64,
    range_days: u32,
) -> Result<usize> {
    let count = std::cmp::max(min_candles, 250) as u32;
    let code = to_moomoo_code(symbol);

    // Calculate start date from range_days
    let start = if range_days > 0 {
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::days(range_days as i64);
        Some(cutoff.format("%Y-%m-%d").to_string())
    } else {
        None
    };

    let script = get_kline_script();
    let mut cmd = Command::new("python3");
    cmd.arg(&script)
        .arg(&code)
        .arg("--ktype").arg("1d")
        .arg("--num").arg(count.to_string())
        .arg("--rehab").arg("forward")
        .arg("--json");
    if let Some(ref s) = start {
        cmd.arg("--start").arg(s);
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().await
        .context("spawning get_kline.py")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("get_kline.py failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let resp: KlineResponse = serde_json::from_str(&stdout)
        .with_context(|| format!("decode get_kline.py JSON: {stdout}"))?;

    let mut candles = Vec::new();
    for r in &resp.data {
        let ts = parse_time_key(&r.time)?;
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
    info!(symbol, fetched, "Moomoo kline fetched");

    for c in &candles {
        db::upsert_equity_candle(pool, c)
            .await
            .with_context(|| format!("upsert {symbol} ts={}", c.ts))?;
    }

    Ok(fetched)
}

/// Fetch a live quote via Moomoo `get_snapshot.py`.
/// Returns (last_price, prev_close, change, change_pct).
pub async fn fetch_quote(symbol: &str) -> Result<Quote> {
    let code = to_moomoo_code(symbol);
    let script = get_snapshot_script();

    let output = Command::new("python3")
        .arg(&script)
        .arg(&code)
        .arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output().await
        .context("spawning get_snapshot.py")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("get_snapshot.py failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let resp: SnapshotResponse = serde_json::from_str(&stdout)
        .with_context(|| format!("decode get_snapshot.py JSON: {stdout}"))?;

    let row = resp.data.into_iter().next()
        .ok_or_else(|| anyhow!("get_snapshot.py returned no data"))?;

    let price = row.last_price;
    let prev_close = row.prev_close;
    let change = price - prev_close;
    let change_pct = if prev_close > 0.0 { (change / prev_close) * 100.0 } else { 0.0 };

    Ok(Quote {
        symbol: symbol.to_string(),
        price,
        prev_close,
        change,
        change_pct,
        timestamp: chrono::Utc::now().timestamp(),
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Quote {
    pub symbol: String,
    pub price: f64,
    pub prev_close: f64,
    pub change: f64,
    pub change_pct: f64,
    pub timestamp: i64,
}

#[derive(Deserialize)]
struct KlineResponse {
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
    data: Vec<SnapshotRow>,
}

#[derive(Deserialize)]
struct SnapshotRow {
    last_price: f64,
    prev_close: f64,
}

/// Parse Moomoo time_key "2024-01-02 00:00:00" (US Eastern) → Unix timestamp.
fn parse_time_key(time_key: &str) -> Result<i64> {
    // Moomoo returns "yyyy-MM-dd HH:mm:ss" in US Eastern time.
    // For daily candles, the time is typically "yyyy-MM-dd 00:00:00".
    // We parse as naive datetime and treat as UTC midnight (close enough for daily bars).
    let dt = chrono::NaiveDateTime::parse_from_str(time_key, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("parse time_key: {time_key}"))?;
    Ok(dt.and_utc().timestamp())
}

// Keep existing kline_to_equity_candle for backward compat with tests.
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
    fn kline_to_equity_candle_basic() {
        let k = serde_json::json!({
            "timestamp": 1_700_000_000_i64,
            "open": 100.0, "high": 105.0, "low": 99.0,
            "close": 104.0, "volume": 50_000_i64
        });
        let c = kline_to_equity_candle("QQQ", &k).unwrap();
        assert_eq!(c.symbol, "QQQ");
        assert_eq!(c.source, "moomoo");
    }
}
```

**Verification:**
```bash
cargo test -p engine moomoo -- --nocapture
cargo check -p engine 2>&1 | grep "^error"
```

---

## Task 5: Update `data/mod.rs` to route through Moomoo-first with fallback

**Objective:** Make `backfill_equities()` try Moomoo first, fall back to Yahoo for equities; use CBOE for VIX, FRED JSON for DGS10+DXY.

**Files:**
- Modify: `engine/src/data/mod.rs` (rewrite `backfill_equities`)

**Implementation:**

Replace `backfill_equities` with:

```rust
pub async fn backfill_equities(pool: &DbPool, stale_threshold_secs: i64) -> Result<()> {
    // --- 1. Equity OHLCV: Moomoo first, Yahoo fallback ---
    let moomoo_ok = moomoo::is_available().await;
    if moomoo_ok {
        info!("Moomoo OpenD reachable — using as primary equity source");
        for s in EQUITY_SYMBOLS {
            match moomoo::backfill(pool, s, 250, 5 * 365).await {
                Ok(n) => info!(symbol = s, rows = n, "Moomoo backfill complete"),
                Err(e) => {
                    warn!(symbol = s, error = %e, "Moomoo backfill failed — trying Yahoo fallback");
                    match yahoo::backfill(pool, s, 250, "5y", stale_threshold_secs).await {
                        Ok(n) => info!(symbol = s, rows = n, "Yahoo fallback complete"),
                        Err(e2) => warn!(symbol = s, error = %e2, "Yahoo fallback also failed"),
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    } else {
        info!("Moomoo OpenD not reachable — using Yahoo for equities");
        let n_eq = yahoo::backfill_many(pool, EQUITY_SYMBOLS, 250, "5y", stale_threshold_secs).await?;
        info!(rows = n_eq, "equity OHLCV backfill complete (Yahoo)");
    }

    // --- 2. VIX: CBOE (free, no auth, no rate limits) ---
    match cboe::backfill_vix(pool, 5 * 365).await {
        Ok(n) => info!(rows = n, "CBOE VIX backfill complete"),
        Err(e) => {
            warn!(error = %e, "CBOE VIX failed — trying Yahoo ^VIX fallback");
            match yahoo::backfill(pool, "^VIX", 1, "2y", stale_threshold_secs).await {
                Ok(n) => info!(rows = n, "Yahoo ^VIX fallback complete"),
                Err(e2) => warn!(error = %e2, "VIX backfill failed entirely — VIX features will be 0.0"),
            }
        }
    }

    // --- 3. Macro: FRED JSON API (DGS10, DTWEXBGS) ---
    let n_macro = fred::backfill_all_default_macros(pool, 5 * 365).await;
    match n_macro {
        Ok(n) => info!(rows = n, "macro series backfill complete (FRED JSON API)"),
        Err(e) => warn!(error = %e, "FRED macro backfill failed — features degrade to 0.0"),
    }

    Ok(())
}
```

Also add `pub mod cboe;` to the module declarations at the top.

**Verification:**
```bash
cargo check -p engine 2>&1 | grep "^error"
# Test with OpenD not running — should fall back to Yahoo cleanly
cargo test -p engine data:: -- --nocapture
```

---

## Task 6: Update `api/chart.rs` and `api/quote.rs` to use Moomoo-first quote

**Objective:** The chart and quote endpoints should try Moomoo `fetch_quote` first, fall back to Yahoo `fetch_quote`.

**Files:**
- Modify: `engine/src/api/chart.rs` (line ~123, the `fetch_quote` call)
- Modify: `engine/src/api/quote.rs` (the `fetch_quote` call)

**Implementation:**

In `chart.rs`, replace:
```rust
let live_quote = match crate::data::yahoo::fetch_quote(&state.symbol).await {
```
with:
```rust
let live_quote = if crate::data::moomoo::is_available().await {
    match crate::data::moomoo::fetch_quote(&state.symbol).await {
        Ok(q) => Some(LiveQuote { price: q.price, prev_close: q.prev_close, change: q.change, change_pct: q.change_pct }),
        Err(e) => {
            tracing::warn!(error = %e, "Moomoo quote failed — trying Yahoo");
            match crate::data::yahoo::fetch_quote(&state.symbol).await {
                Ok(q) => Some(LiveQuote { price: q.price, prev_close: q.prev_close, change: q.change, change_pct: q.change_pct }),
                Err(e2) => { tracing::warn!(error = %e2, "Yahoo quote also failed"); None }
            }
        }
    }
} else {
    match crate::data::yahoo::fetch_quote(&state.symbol).await {
        Ok(q) => Some(LiveQuote { price: q.price, prev_close: q.prev_close, change: q.change, change_pct: q.change_pct }),
        Err(e) => { tracing::warn!(error = %e, "Yahoo quote failed"); None }
    }
};
```

Same pattern in `quote.rs`.

**Verification:**
```bash
cargo check -p engine 2>&1 | grep "^error"
# Test: curl http://<engine-ip>:8080/api/quote returns price
```

---

## Task 7: Create sentiment DB table and stub fetcher

**Objective:** Add the `equity_sentiment` table and a no-op fetcher that returns 0.5 (neutral). Wire the real Finnhub API later.

**Files:**
- Modify: `engine/src/db.rs` (add DDL for `equity_sentiment` table + CRUD functions)
- Create: `engine/src/data/sentiment.rs`

**DDL:**
```sql
CREATE TABLE IF NOT EXISTS equity_sentiment (
    symbol       TEXT NOT NULL,
    ts           INTEGER NOT NULL,
    sentiment_score  REAL NOT NULL,  -- 0.0 (bearish) to 1.0 (bullish)
    buzz_score      REAL NOT NULL DEFAULT 0.0,
    news_score      REAL NOT NULL DEFAULT 0.0,
    source       TEXT NOT NULL DEFAULT 'finnhub',
    PRIMARY KEY (symbol, ts)
);
```

**Stub fetcher:**
```rust
//! Sentiment data stub. Returns neutral 0.5.
//! Real implementation will call Finnhub `news_sentiment` endpoint (free, 60 req/min).

use crate::db::DbPool;
use anyhow::Result;

pub async fn backfill_sentiment(_pool: &DbPool, _symbol: &str) -> Result<usize> {
    // Phase 2: implement Finnhub news_sentiment call
    // GET https://finnhub.io/api/v1/news-sentiment?symbol=QQQ&token=<API_KEY>
    // Response: { buzz: { buzz, weeklyAverage, articlesInLastWeek },
    //             sentiment: { bearishPercent, bullishPercent },
    //             companyNewsScore }
    // Store bullishPercent as sentiment_score
    tracing::info!("sentiment fetcher is a stub — returning 0 rows");
    Ok(0)
}
```

Add `pub mod sentiment;` to `data/mod.rs` and `CREATE TABLE equity_sentiment` to DDL in `db.rs`.

**Verification:**
```bash
cargo check -p engine 2>&1 | grep "^error"
```

---

## Task 8: Update `.env.example` and `deploy/config.md` docs

**Objective:** Document all new env vars and the OpenD requirement.

**Files:**
- Modify: `.env.example`
- Modify: `deploy/config.md`
- Modify: `deploy/docker-compose.yml` (add FRED_API_KEY, FUTU_OPEND_HOST, FUTU_OPEND_PORT env vars)

**`.env.example` additions:**
```bash
# ── Moomoo OpenD (data + trade) ───────────────────────────
# OpenD must be running on the VPS for Moomoo data to work.
# Without it, the engine falls back to Yahoo Finance.
# FUTU_OPEND_HOST=127.0.0.1
# FUTU_OPEND_PORT=11111

# ── FRED API key (free) ───────────────────────────────────
# Get one at https://fred.stlouisfed.org/apikey (30 seconds).
# Required for treasury yield + DXY macro data.
FRED_API_KEY=
```

**Verification:**
```bash
# Verify docker-compose has the new env vars
grep -c "FRED_API_KEY\|FUTU_OPEND" deploy/docker-compose.yml
# Expected: >= 2
```

---

## Task 9: Build, deploy, and verify

**Objective:** Full build cycle with all sources wired.

**Steps:**
1. `cargo check -p engine` — 0 errors
2. `cargo test -p engine -- cboe fred moomoo sentiment` — all pass
3. `cd frontend && npm run build` — 0 errors, 0 warnings
4. `docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .`
5. `docker stop mmn-engine && docker rm mmn-engine && docker-compose -f deploy/docker-compose.yml up -d engine`
6. Wait 30s, check health: `docker ps --filter name=mmn-engine`
7. Check logs for source selection: `docker logs mmn-engine --since 1m 2>&1 | grep -i "moomoo\|cboe\|fred\|yahoo\|sentiment"`
8. Verify chart endpoint: `curl http://<engine-ip>:8080/api/chart?limit=3` — last candle should be recent with price near live quote
9. Verify quote endpoint: `curl http://<engine-ip>:8080/api/quote` — should return QQQ price

**Expected log output (without OpenD):**
```
INFO engine::data: Moomoo OpenD not reachable — using Yahoo for equities
INFO engine::data::yahoo: equity OHLCV backfill complete rows=1255
INFO engine::data::cboe: CBOE VIX backfill complete fetched=505
WARN engine::data::fred: FRED macro backfill failed — features degrade to 0.0
INFO engine::data::sentiment: sentiment fetcher is a stub — returning 0 rows
```

**Expected log output (with OpenD running):**
```
INFO engine::data: Moomoo OpenD reachable — using as primary equity source
INFO engine::data::moomoo: Moomoo kline fetched symbol=QQQ fetched=1255
INFO engine::data::cboe: CBOE VIX backfill complete fetched=505
INFO engine::data: FRED JSON API request
INFO engine::data: macro series backfill complete rows=...
```

---

## Summary of files changed

| File | Action | Description |
|---|---|---|
| `engine/src/data/cboe.rs` | Create | CBOE VIX CSV fetcher |
| `engine/src/data/moomoo.rs` | Rewrite | Real Python subprocess data fetcher |
| `engine/src/data/fred.rs` | Modify | Switch from CSV to JSON API v2 |
| `engine/src/data/mod.rs` | Modify | Moomoo-first routing with Yahoo fallback |
| `engine/src/data/sentiment.rs` | Create | Stub sentiment fetcher |
| `engine/src/db.rs` | Modify | Add equity_sentiment DDL + CRUD |
| `engine/src/api/chart.rs` | Modify | Moomoo-first quote with Yahoo fallback |
| `engine/src/api/quote.rs` | Modify | Moomoo-first quote with Yahoo fallback |
| `deploy/docker-compose.yml` | Modify | Add FRED_API_KEY, FUTU_OPEND_* env vars |
| `.env.example` | Modify | Document new env vars |
| `deploy/config.md` | Modify | Document new env vars |

## Risks & tradeoffs

1. **OpenD must run on the VPS** — Moomoo data won't work without it. The fallback to Yahoo ensures the engine still runs, but with rate-limit issues. OpenD installation is a separate prerequisite (Ubuntu/CentOS supported per docs).

2. **Python subprocess overhead** — each `get_kline.py` call spawns a Python interpreter (~200ms overhead). For 11 symbols × 1 call/day, this is acceptable (~2s total). For the 30s chart refresh poll, the snapshot call is lightweight.

3. **CBOE CSV format** — if CBOE changes the CSV format, the parser will bail. Low risk (format has been stable since 1990).

4. **FRED JSON API key** — the free tier is 120 req/min, more than enough for 3 series × daily calls.

5. **Sentiment is stub-only** — no real data until Finnhub is wired. The DB table exists so the feature pipeline can be extended later without schema migration.
