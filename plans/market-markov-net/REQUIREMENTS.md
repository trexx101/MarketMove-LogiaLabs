# MarketMarkovNet — Requirements Index

## Overview

An automated BTC/USD swing-trading platform pairing a high-performance Rust
(Tokio/Axum) execution core with a Python (PyTorch/ZeroMQ) inference microservice
running the `MarketMarkovNet` model. The system ingests Kraken v2 market data,
computes features hourly, requests predictions over ZeroMQ, applies a hysteresis +
regime-filtered trading state machine, and executes trades (paper or live). It is
deployed via Docker Compose on an Ubuntu 24.04 CPU VPS with a minimal web control
room for observability. Paper trading is the default; live Kraken execution is gated
behind a verified parity check.

## Confirmed product decisions

- **Exchange:** Kraken (v2 WebSocket + REST). The Binance-trained model is deployed
  **as-is**; a parity/drift monitor logs all inference inputs so Binance↔Kraken
  distribution drift can be observed without blocking launch.
- **Model artifacts:** Supplied by the user (`model.pt`, `norm_stats.json`) and placed
  in `/models` (gitignored). No training is in scope.
- **Storage:** SQLite.
- **ZMQ payload:** JSON (REQ/REP).
- **Frontend:** Extensible vanilla SPA (no build toolchain) so additional interfaces
  can be added later.
- **Normalization:** Z-score normalization is applied **Rust-side** (the engine owns
  the rolling window and normalization stats); Python receives normalized features.
- **Target host:** CPU-only VPS (torch CPU wheels). VPS is available now; this repo is
  the local starting point.
- **Trading mode:** Paper trading is the default. Live execution is behind a config
  flag and the Phase-5 parity gate.

## Tech stack

- **Execution core:** Rust (stable), Tokio, `tokio-tungstenite` (WS), `axum` (HTTP),
  `sqlx`/`rusqlite` (SQLite), `zmq`/`tmq`, `reqwest` (Kraken REST), `serde`.
- **Inference:** Python 3.11 (uv), PyTorch (CPU), `pyzmq`.
- **Frontend:** Static HTML + vanilla JS + a lightweight chart lib (uPlot/Chart.js),
  served by Axum.
- **Infra:** Docker + Docker Compose, UFW, Ubuntu 24.04 LTS.

## Global constraints / rules

- **Parity is the release gate.** The Rust feature pipeline must reproduce the Colab
  pandas feature engineering bit-for-bit: `log_return`, `TR`/`ATR(72)`, typical price,
  `VWAP`, `vwap_dev`, rolling z-score windows, and the 200-hour SMA regime filter.
- UFW blocks port 5555 and all internal service ports; only 22 and 80/443 are exposed.
- Kraken API keys are **Query + Trade only, Withdraw disabled**.
- `.env`, `/models`, and `*.db` files are gitignored.
- All internal timestamps are UTC. Hourly compute aligns to candle-close boundaries.
- After each feature: run lint / type-check / `cargo build` (and container build where
  relevant) as quality gates.
- Prefer sub-agents for implementation; coordinate parallelizable features.

## Data model (SQLite)

- `candles(ts INTEGER PK, open, high, low, close, volume, vwap)` — rolling 200h+ window.
- `predictions(ts INTEGER PK, pred_1h, pred_4h, pred_24h, features_json)`
- `positions(ts INTEGER, side, entry_px, size, mode)`
- `trades(ts INTEGER, side, px, size, fee, pnl, mode)`
- `state(key TEXT PK, value TEXT)` — persisted state-machine + last-signal data.

## Environment variables

| Var | Purpose | Example |
|-----|---------|---------|
| `KRAKEN_API_KEY` | Kraken key (Query+Trade) | — |
| `KRAKEN_API_SECRET` | Kraken secret | — |
| `TRADING_MODE` | `paper` or `live` | `paper` |
| `ZMQ_ENDPOINT` | Inference socket | `tcp://inference:5555` |
| `MAGNITUDE_THRESHOLD` | Signal threshold | `0.005` |
| `PAPER_FEE` | Simulated fee rate | `0.0015` |
| `SMA_WINDOW` | Regime SMA window | `200` |
| `HTTP_PORT` | Axum server port | `8080` |
| `SYMBOL` | Trading pair | `BTC/USD` |

## Feature table

| # | Feature | Phase | Depends on | Parallel with |
|---|---------|-------|-----------|---------------|
| 01 | Repo scaffold & workspace | 1 | none | — |
| 02 | VPS hardening & infra setup | 1 | 01 | 03 |
| 03 | Kraken credentials & config | 1 | 01 | 02 |
| 04 | Python inference microservice | 2 | 01 | 06 |
| 05 | Inference Docker image | 2 | 04 | — |
| 06 | Rust data pipeline (WS + SQLite) | 3 | 01 | 04 |
| 07 | Feature computation & ZMQ bridge | 3 | 06, 04 | — |
| 08 | Logic engine (hysteresis + regime) | 3 | 07 | — |
| 09 | Execution layer (paper + Kraken) | 3 | 08, 03 | — |
| 10 | Axum telemetry API | 4 | 06, 08, 09 | 11 |
| 11 | Vanilla SPA control room | 4 | 10 | — |
| 12 | Paper-trading verification | 5 | 09 | — |
| 13 | Regression / parity harness | 5 | 07, 08 | — |
| 14 | Docker Compose deploy & launch | 5 | all | — |
