# MarketMarkovNet

An automated BTC/USD swing-trading platform pairing a high-performance Rust
(Tokio/Axum) execution core with a Python (PyTorch/ZeroMQ) inference microservice
running the `MarketMarkovNet` model. The system ingests Kraken v2 market data,
computes features hourly, requests predictions over ZeroMQ, applies a hysteresis +
regime-filtered trading state machine, and executes trades (paper or live). It is
deployed via Docker Compose on an Ubuntu 24.04 CPU VPS with a minimal web control
room for observability. Paper trading is the default; live Kraken execution is
gated behind a verified parity check.

## Architecture

```
+-------------------+        ZMQ REQ/REP (JSON)        +-------------------+
|   inference       |  <-----------------------------> |     engine        |
|   (Python / Torch) |   tcp://*:5555                  |   (Rust / Tokio)  |
|   ZMQ REP server  |                                  |                   |
+-------------------+                                  |  - WS ingest      |
                                                       |  - feature pipe   |
                                                       |  - logic engine   |
                                                       |  - execution      |
                                                       |        |          |
                                                       |        v          |
                                                       |   +-----------+   |
                                                       |   |  SQLite   |   |
                                                       |   +-----------+   |
                                                       |        |          |
+-------------------+   HTTP (Axum)                     |        v          |
|    frontend       |  <-----------------------------> |  /api/* (Axum)    |
|    (vanilla SPA)  |                                  +-------------------+
+-------------------+                                            |
                                                                  v
                                                          +----------------+
                                                          |   Kraken v2    |
                                                          |   WS + REST    |
                                                          +----------------+
```

- **inference/** — Python (PyTorch CPU) microservice. Loads `model.pt` +
  `norm_stats.json`, answers ZMQ `REQ` messages with a JSON `{pred_1h, pred_4h,
  pred_24h}` payload.
- **engine/** — Rust workspace member. Tokio runtime: Kraken WS ingest → SQLite
  → rolling feature pipeline (Rust-side z-score normalization) → ZMQ REQ → state
  machine → paper/Kraken execution → Axum telemetry.
- **frontend/** — Static vanilla SPA served by Axum. Polls `/api/status`,
  `/api/predictions`, `/api/trades`; renders charts and a status panel.
- **deploy/** — Docker Compose files, provisioning notes, environment-variable
  reference.
- **models/** — `model.pt` and `norm_stats.json` (user-supplied, gitignored).
- **tests/** — Parity/regression harness fixtures (Feature 13).
- **plans/** — Approved plan & per-feature specs (see
  `plans/market-markov-net/REQUIREMENTS.md`).

## Quick start

### Engine (Rust)

```bash
cd /home/ubuntu/projects/MarketMoves
cargo build                      # workspace build
cargo run --bin engine           # launch (needs .env + models + ZMQ peer)
```

### Inference (Python)

```bash
cd /home/ubuntu/projects/MarketMoves/inference
uv sync                         # resolves pyproject.toml, creates .venv
uv run python inference_engine.py
```

The full ZMQ REP loop is implemented in **Feature 04**.

### Frontend

The SPA is served by the Axum telemetry service in production. For local
development open `frontend/index.html` in a browser (status polling will 404
until the engine is up).

### Docker Compose

Compose files land in `deploy/` in **Feature 14**. The canonical command once
they exist:

```bash
cd /home/ubuntu/projects/MarketMoves/deploy
docker compose up -d
```

## Environment variables

| Var | Purpose | Example |
|-----|---------|---------|
| `KRAKEN_API_KEY` | Kraken key (Query+Trade only, Withdraw disabled) | — |
| `KRAKEN_API_SECRET` | Kraken secret | — |
| `TRADING_MODE` | `paper` or `live` | `paper` |
| `ZMQ_ENDPOINT` | Inference socket | `tcp://inference:5555` |
| `MAGNITUDE_THRESHOLD` | Signal threshold (z-score magnitude) | `0.005` |
| `PAPER_FEE` | Simulated fee rate (paper mode) | `0.0015` |
| `SMA_WINDOW` | Regime SMA window (hours) | `200` |
| `HTTP_PORT` | Axum server port | `8080` |
| `SYMBOL` | Trading pair | `BTC/USD` |

See `deploy/config.md` for the full reference and `plans/market-markov-net/REQUIREMENTS.md`
for the authoritative source.

## Trading mode

`TRADING_MODE=paper` (default) — orders are simulated locally, fees are taken
from `PAPER_FEE`, fills are written to the `trades` table with `mode='paper'`,
and no Kraken REST calls are made.

`TRADING_MODE=live` — orders are signed and sent to Kraken. The engine refuses
to start in live mode unless the **parity gate** has been satisfied (see
below). Kraken API keys must have Withdraw disabled.

## Parity gate

Parity is the release gate. The Rust feature pipeline must reproduce the Colab
pandas feature engineering bit-for-bit: `log_return`, `TR`/`ATR(72)`, typical
price, `VWAP`, `vwap_dev`, rolling z-score windows, and the 200-hour SMA regime
filter. Until Feature 13 (parity harness) reports a clean run against the
golden fixtures, `TRADING_MODE=live` will refuse to start. Drift between
Binance (training distribution) and Kraken (live distribution) is logged but
does not block launch.

## Plan & feature specs

The full plan and per-feature implementation specs live in
[`plans/market-markov-net/`](./plans/market-markov-net/):

- `REQUIREMENTS.md` — confirmed product decisions, tech stack, data model,
  feature table.
- `features/` — one spec per feature (01..14), each with requirements, technical
  steps, and acceptance criteria.

## License

MIT. See [Cargo.toml](./Cargo.toml) workspace metadata.
