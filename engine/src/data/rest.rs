//! Kraken public REST API — OHLC backfill.
//!
//! Endpoint: `GET https://api.kraken.com/0/public/OHLC`
//! Interval : 60 (1-hour candles).
//!
//! Response shape:
//! ```json
//! {
//!   "error": [],
//!   "result": {
//!     "XXBTZUSD": [[ts, open, high, low, close, vwap, volume, count], …],
//!     "last": 1234567890
//!   }
//! }
//! ```

use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::db::{self, Candle, DbPool};

const REST_URL: &str = "https://api.kraken.com/0/public/OHLC";
/// 1-hour interval in minutes (Kraken uses minutes).
const INTERVAL_MIN: u64 = 60;
/// Maximum candles returned per REST call (Kraken caps at 720).
const MAX_PER_CALL: usize = 720;

/// Convert a WebSocket-style symbol (`BTC/USD`) to a Kraken REST pair (`XBTUSD`).
fn to_rest_pair(symbol: &str) -> String {
    symbol.replace('/', "").replace("BTC", "XBT")
}

/// Fetch OHLC candles from Kraken REST and upsert them into the database.
///
/// Returns the total number of candles inserted/updated.
pub async fn backfill(pool: &DbPool, symbol: &str, min_candles: usize) -> Result<usize> {
    let count = db::count_candles(pool).await?;
    if count >= min_candles as i64 {
        info!(
            count,
            min_candles, "sufficient candles already in DB — skipping REST backfill"
        );
        return Ok(0);
    }

    let pair = to_rest_pair(symbol);
    info!(pair, min_candles, "starting REST backfill");

    let client = Client::builder()
        .user_agent("MarketMarkovNet/0.1")
        .build()
        .context("building reqwest client")?;

    let candles_needed = min_candles.max(MAX_PER_CALL);
    // Request enough history: each candle is `INTERVAL_MIN * 60` seconds wide.
    let since = chrono::Utc::now().timestamp()
        - (candles_needed as i64 * INTERVAL_MIN as i64 * 60);

    let candles = fetch_ohlc(&client, &pair, since).await?;
    let fetched = candles.len();
    info!(fetched, "REST returned candles");

    let mut upserted = 0usize;
    for c in &candles {
        db::upsert_candle(pool, c)
            .await
            .with_context(|| format!("upsert ts={}", c.ts))?;
        upserted += 1;
    }

    let final_count = db::count_candles(pool).await?;
    if final_count < min_candles as i64 {
        warn!(
            final_count,
            min_candles, "backfill completed but still below min_candles"
        );
    } else {
        info!(final_count, "backfill done — DB has sufficient candles");
    }

    Ok(upserted)
}

async fn fetch_ohlc(client: &Client, pair: &str, since: i64) -> Result<Vec<Candle>> {
    let url = format!(
        "{REST_URL}?pair={pair}&interval={INTERVAL_MIN}&since={since}"
    );
    debug!(%url, "REST OHLC request");

    let resp: Value = client
        .get(&url)
        .send()
        .await
        .context("GET OHLC")?
        .json()
        .await
        .context("decode OHLC JSON")?;

    // Surface any API-level errors.
    if let Some(errors) = resp["error"].as_array() {
        let msgs: Vec<_> = errors
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        if !msgs.is_empty() {
            bail!("Kraken REST error(s): {}", msgs.join(", "));
        }
    }

    let result = resp["result"]
        .as_object()
        .context("missing 'result' object in REST response")?;

    // The candle array is the first non-"last" key.
    let candle_array = result
        .iter()
        .find(|(k, _)| *k != "last")
        .and_then(|(_, v)| v.as_array())
        .context("no candle data array found in REST result")?;

    let mut rows = Vec::with_capacity(candle_array.len());
    for item in candle_array {
        match parse_row(item) {
            Ok(row) => rows.push(row),
            Err(e) => warn!("skipping malformed REST candle: {e:#}"),
        }
    }
    Ok(rows)
}

fn parse_row(v: &Value) -> Result<Candle> {
    let arr = v.as_array().context("candle row is not an array")?;
    // [ts, open, high, low, close, vwap, volume, count]
    if arr.len() < 7 {
        bail!("candle row has {} fields, expected ≥7", arr.len());
    }

    let ts = arr[0]
        .as_i64()
        .or_else(|| arr[0].as_f64().map(|f| f as i64))
        .context("ts field")?;

    let parse_price = |i: usize, name: &str| -> Result<f64> {
        arr[i]
            .as_str()
            .context(format!("{name} not a string"))?
            .parse::<f64>()
            .with_context(|| format!("{name} parse error"))
    };

    Ok(Candle {
        ts,
        open: parse_price(1, "open")?,
        high: parse_price(2, "high")?,
        low: parse_price(3, "low")?,
        close: parse_price(4, "close")?,
        vwap: parse_price(5, "vwap")?,
        volume: parse_price(6, "volume")?,
    })
}
