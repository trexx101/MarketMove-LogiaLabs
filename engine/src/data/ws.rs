//! Kraken v2 WebSocket OHLC ingestion.
//!
//! Endpoint: `wss://ws.kraken.com/v2`
//!
//! Subscribe message:
//! ```json
//! {"method":"subscribe","params":{"channel":"ohlc","symbol":["BTC/USD"],"interval":60}}
//! ```
//!
//! Incoming OHLC message shape:
//! ```json
//! {
//!   "channel": "ohlc",
//!   "type": "update",
//!   "data": [{
//!     "symbol": "BTC/USD",
//!     "open": "30000.00", "high": "30100.00", "low": "29900.00", "close": "30050.00",
//!     "vwap": "30020.00", "volume": "1.23456789",
//!     "interval_begin": "2023-01-01T00:00:00.000000Z",
//!     "timestamp": "2023-01-01T01:00:00.000000Z",
//!     "interval": 60,
//!     "confirm": true
//!   }]
//! }
//! ```
//!
//! Only candles with `"confirm": true` are persisted (closed candles).

use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use crate::db::{self, Candle, DbPool};

const WS_URL: &str = "wss://ws.kraken.com/v2";
/// 1-hour interval in minutes (Kraken v2 OHLC).
const INTERVAL_MIN: u64 = 60;
/// Initial reconnect delay.
const BACKOFF_INIT: Duration = Duration::from_secs(1);
/// Maximum reconnect delay.
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Run the WebSocket OHLC ingestion loop, reconnecting with exponential backoff on failure.
/// This function never returns under normal operation.
pub async fn run_loop(pool: &DbPool, symbol: &str) -> Result<()> {
    let mut backoff = BACKOFF_INIT;

    loop {
        info!("connecting to Kraken v2 WebSocket at {WS_URL}");
        match run_session(pool, symbol).await {
            Ok(()) => {
                // Clean close — reconnect immediately.
                warn!("WebSocket session ended cleanly; reconnecting");
                backoff = BACKOFF_INIT;
            }
            Err(e) => {
                error!("WebSocket error: {e:#}; reconnecting in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }

        // On every reconnect, backfill any gap that accumulated during downtime.
        info!("gap backfill after reconnect");
        if let Err(e) = crate::data::rest::backfill(pool, symbol, 0).await {
            warn!("gap backfill error (non-fatal): {e:#}");
        }
    }
}

/// Run a single WebSocket session until the stream closes or an error occurs.
async fn run_session(pool: &DbPool, symbol: &str) -> Result<()> {
    let (ws, _) = connect_async(WS_URL)
        .await
        .context("WebSocket connect")?;
    let (mut sink, mut stream) = ws.split();

    // Send subscribe message.
    let sub = serde_json::json!({
        "method": "subscribe",
        "params": {
            "channel": "ohlc",
            "symbol": [symbol],
            "interval": INTERVAL_MIN
        }
    });
    sink.send(Message::Text(sub.to_string()))
        .await
        .context("send subscribe")?;
    info!(symbol, "subscribed to OHLC channel");

    while let Some(msg) = stream.next().await {
        match msg.context("WebSocket stream error")? {
            Message::Text(text) => {
                if let Err(e) = handle_text(pool, &text).await {
                    warn!("message handling error (non-fatal): {e:#}");
                }
            }
            Message::Ping(data) => {
                sink.send(Message::Pong(data))
                    .await
                    .context("send pong")?;
            }
            Message::Close(frame) => {
                info!(?frame, "WebSocket close frame received");
                return Ok(());
            }
            _ => {}
        }
    }

    Ok(())
}

/// Process a single text message from the WebSocket.
async fn handle_text(pool: &DbPool, text: &str) -> Result<()> {
    let v: Value = serde_json::from_str(text).context("parse JSON")?;

    // Only handle OHLC updates/snapshots.
    match v["channel"].as_str() {
        Some("ohlc") => {}
        Some("heartbeat") | Some("status") => return Ok(()),
        other => {
            debug!(channel = ?other, "ignoring non-OHLC message");
            return Ok(());
        }
    }

    let data = match v["data"].as_array() {
        Some(arr) => arr,
        None => bail!("OHLC message missing 'data' array"),
    };

    for item in data {
        // `confirm: true` signals the candle interval is closed.
        let confirm = item["confirm"].as_bool().unwrap_or(false);
        if !confirm {
            continue;
        }

        match parse_candle(item) {
            Ok(candle) => {
                let ts = candle.ts;
                let c = candle.close;
                let vol = candle.volume;
                db::upsert_candle(pool, &candle)
                    .await
                    .with_context(|| format!("upsert confirmed candle ts={ts}"))?;
                info!(ts, close = c, volume = vol, "confirmed candle persisted");
            }
            Err(e) => warn!("skipping malformed WS candle: {e:#}"),
        }
    }

    Ok(())
}

/// Extract OHLCV+VWAP values from a single OHLC data item.
///
/// `interval_begin` is used as the canonical open-time timestamp.
fn parse_candle(item: &Value) -> Result<Candle> {
    let ts = parse_iso_ts(
        item["interval_begin"]
            .as_str()
            .context("interval_begin missing")?,
    )?;

    let parse_f64 = |key: &str| -> Result<f64> {
        item[key]
            .as_str()
            .with_context(|| format!("{key} not a string"))?
            .parse::<f64>()
            .with_context(|| format!("{key} parse error"))
    };

    Ok(Candle {
        ts,
        open: parse_f64("open")?,
        high: parse_f64("high")?,
        low: parse_f64("low")?,
        close: parse_f64("close")?,
        volume: parse_f64("volume")?,
        vwap: parse_f64("vwap")?,
    })
}

/// Parse an ISO 8601 UTC timestamp string (e.g. `"2023-01-01T00:00:00.000000Z"`)
/// into a Unix timestamp (seconds).
fn parse_iso_ts(s: &str) -> Result<i64> {
    use chrono::{DateTime, Utc};
    let dt: DateTime<Utc> = s
        .parse()
        .with_context(|| format!("parse ISO timestamp '{s}'"))?;
    Ok(dt.timestamp())
}
