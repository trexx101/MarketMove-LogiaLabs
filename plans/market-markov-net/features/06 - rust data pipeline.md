# Feature 06 — Rust Data Pipeline (WS Ingestion + SQLite)

**Depends on:** 01
**Goal:** Ingest Kraken v2 OHLC data over WebSocket, backfill history via REST, and persist a rolling window to SQLite.

## Requirements

- Connect to Kraken v2 WebSocket and subscribe to the OHLC (1h) channel for `BTC/USD`.
- On cold start, backfill via Kraken REST to seed at least 200 hourly candles (for the 200-SMA + rolling windows).
- Async persistence to SQLite with rolling retention (keep 200h+ buffer).
- Robust reconnect/backoff on WS disconnect; dedupe candles by timestamp.

## Technical Implementation Steps

1. Define `candles` table + migrations (sqlx migrations or a startup DDL).
2. `engine/src/data/rest.rs`: Kraken OHLC REST backfill; upsert candles.
3. `engine/src/data/ws.rs`: `tokio-tungstenite` client, subscribe OHLC, parse v2 messages, persist closed candles.
4. Reconnect with exponential backoff; on reconnect, re-backfill any gap.
5. Retention task pruning rows older than the configured window.

## Acceptance Criteria

- [ ] Cold start seeds ≥ 200 candles from REST.
- [ ] New hourly candle from WS is persisted with correct OHLCV+VWAP.
- [ ] WS disconnect triggers reconnect + gap backfill (tested via forced drop).
- [ ] `cargo build` + `cargo clippy` pass.
