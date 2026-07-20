//! Binance public REST + WebSocket — 1h kline ingestion (Wave 5: replaces Kraken).
//!
//! REST:  `GET https://api.binance.com/api/v3/klines?symbol=BTCUSDT&interval=1h&limit=N`
//! WS:    `wss://stream.binance.com:9443/ws/btcusdt@kline_1h`
//!
//! Only candles with `k.x == true` (closed) are persisted.
//! Funding/premiumIndex/depth/aggTrade endpoints are wired in the D1/D4 passes.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use crate::db::{self, Candle, DbPool};

const REST_URL: &str = "https://api.binance.com/api/v3/klines";
const WS_URL: &str = "wss://stream.binance.com:9443/ws";
/// 1-hour interval string for Binance.
const INTERVAL: &str = "1h";
/// Maximum candles returned per REST call (Binance caps at 1000).
const MAX_PER_CALL: usize = 1000;
const BACKOFF_INIT: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);

/// Convert a config symbol (`BTC/USD`) to a Binance REST/WS symbol (`btcusdt`).
fn to_binance_symbol(symbol: &str) -> String {
    symbol.replace('/', "").to_lowercase()
}

/// Fetch 1h klines from Binance REST and upsert them into the database.
///
/// Returns the number of candles inserted/updated. If the DB already has
/// ≥ `min_candles` rows, returns `Ok(0)` immediately (same gate as Kraken).
pub async fn backfill(pool: &DbPool, symbol: &str, min_candles: usize) -> Result<usize> {
    let count = db::count_candles(pool).await?;
    if count >= min_candles as i64 {
        info!(count, min_candles, "sufficient candles already in DB — skipping REST backfill");
        return Ok(0);
    }

    let pair = to_binance_symbol(symbol);
    info!(pair, min_candles, "starting Binance REST backfill");

    let client = reqwest::Client::builder()
        .user_agent("MarketMarkovNet/0.2")
        .build()
        .context("building reqwest client")?;

    let candles = fetch_klines(&client, &pair, min_candles.max(MAX_PER_CALL)).await?;
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
        warn!(final_count, min_candles, "backfill completed but still below min_candles");
    } else {
        info!(final_count, "backfill done — DB has sufficient candles");
    }

    Ok(upserted)
}

/// Fetch `limit` 1h klines from Binance REST. Returns candles oldest-first.
async fn fetch_klines(client: &reqwest::Client, pair: &str, limit: usize) -> Result<Vec<Candle>> {
    let url = format!("{REST_URL}?symbol={pair}&interval={INTERVAL}&limit={limit}");
    debug!(%url, "REST klines request");

    let resp: Value = client
        .get(&url)
        .send()
        .await
        .context("GET klines")?
        .json()
        .await
        .context("decode klines JSON")?;

    let arr = resp.as_array().context("Binance klines response is not an array")?;
    let mut rows = Vec::with_capacity(arr.len());
    for item in arr {
        match parse_kline(item) {
            Ok(row) => rows.push(row),
            Err(e) => warn!("skipping malformed REST kline: {e:#}"),
        }
    }
    // Binance returns newest-first; reverse to oldest-first for DB ordering.
    rows.reverse();
    Ok(rows)
}

/// Parse a Binance kline array:
/// `[open_time, open, high, low, close, volume, close_time, quote_vol, trades, ...]`
fn parse_kline(v: &Value) -> Result<Candle> {
    let arr = v.as_array().context("kline row is not an array")?;
    if arr.len() < 7 {
        bail!("kline row has {} fields, expected ≥7", arr.len());
    }

    // open_time is in milliseconds — convert to seconds.
    let ts_ms = arr[0]
        .as_i64()
        .or_else(|| arr[0].as_f64().map(|f| f as i64))
        .context("open_time field")?;
    let ts = ts_ms / 1000;

    let parse_price = |i: usize, name: &str| -> Result<f64> {
        arr[i]
            .as_str()
            .context(format!("{name} not a string"))?
            .parse::<f64>()
            .with_context(|| format!("{name} parse error"))
    };

    // Binance klines don't include a VWAP field; approximate with close.
    // The V2 feature pipeline computes rolling VWAP itself (see legacy.rs).
    let close = parse_price(4, "close")?;
    let volume = parse_price(5, "volume")?;

    Ok(Candle {
        ts,
        open: parse_price(1, "open")?,
        high: parse_price(2, "high")?,
        low: parse_price(3, "low")?,
        close,
        volume,
        vwap: close, // placeholder; rolling VWAP computed in feature pipeline
        funding_rate: 0.0,
        basis_z: 0.0,
        ob_imbalance: 0.0,
    })
}

/// Run the WebSocket kline ingestion loop, reconnecting with exponential
/// backoff on failure. Never returns under normal operation.
pub async fn run_loop(pool: &DbPool, symbol: &str) -> Result<()> {
    let mut backoff = BACKOFF_INIT;

    loop {
        info!("connecting to Binance WebSocket at {WS_URL}");
        match run_session(pool, symbol).await {
            Ok(()) => {
                warn!("WebSocket session ended cleanly; reconnecting");
                backoff = BACKOFF_INIT;
            }
            Err(e) => {
                error!("WebSocket error: {e:#}; reconnecting in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }

        // Gap backfill after reconnect.
        info!("gap backfill after reconnect");
        if let Err(e) = backfill(pool, symbol, 0).await {
            warn!("gap backfill error (non-fatal): {e:#}");
        }
    }
}

/// Run a single WebSocket session until the stream closes or an error occurs.
async fn run_session(pool: &DbPool, symbol: &str) -> Result<()> {
    let stream_symbol = to_binance_symbol(symbol);
    let ws_url = format!("{WS_URL}/{stream_symbol}@kline_1h");

    let (ws, _) = connect_async(&ws_url)
        .await
        .with_context(|| format!("WebSocket connect to {ws_url}"))?;
    let (mut sink, mut stream) = ws.split();

    info!(symbol, "subscribed to Binance kline_1h stream");

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

    // Binance kline message: { "e": "kline", "k": { ... } }
    let k = match v.get("k") {
        Some(k) => k,
        None => {
            debug!("ignoring non-kline message");
            return Ok(());
        }
    };

    // `k.x == true` signals the kline is closed.
    let is_closed = k["x"].as_bool().unwrap_or(false);
    if !is_closed {
        return Ok(());
    }

    match parse_ws_kline(k) {
        Ok(candle) => {
            let ts = candle.ts;
            let c = candle.close;
            let vol = candle.volume;
            db::upsert_candle(pool, &candle)
                .await
                .with_context(|| format!("upsert closed kline ts={ts}"))?;
            info!(ts, close = c, volume = vol, "closed kline persisted");
        }
        Err(e) => warn!("skipping malformed WS kline: {e:#}"),
    }

    Ok(())
}

/// Extract OHLCV from a Binance WS kline object.
fn parse_ws_kline(k: &Value) -> Result<Candle> {
    let ts_ms = k["t"]
        .as_i64()
        .or_else(|| k["t"].as_f64().map(|f| f as i64))
        .context("k.t (open time) missing")?;
    let ts = ts_ms / 1000;

    let parse_f64 = |key: &str| -> Result<f64> {
        k[key]
            .as_str()
            .with_context(|| format!("{key} not a string"))?
            .parse::<f64>()
            .with_context(|| format!("{key} parse error"))
    };

    let close = parse_f64("c")?;

    Ok(Candle {
        ts,
        open: parse_f64("o")?,
        high: parse_f64("h")?,
        low: parse_f64("l")?,
        close,
        volume: parse_f64("v")?,
        vwap: close, // placeholder
        funding_rate: 0.0,
        basis_z: 0.0,
        ob_imbalance: 0.0,
    })
}
