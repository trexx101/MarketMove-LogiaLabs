# MarketMoves — QQQ Daily Equities Control Room

A daily-horizon automated trading platform for **QQQ** (Nasdaq‑100 ETF) with a
self-hosted **Control Room** dashboard. The Rust engine ingests OHLCV + macro
features, calls a Python (PyTorch) inference service over ZeroMQ, applies a
hysteresis + SMA-200 regime state machine, and simulates paper trades via an
inverse-ETF remap (PSQ for shorts). The Svelte frontend renders live telemetry
over WebSocket — candlestick chart, prediction cones, equity curve, feature
inspector, and a runtime paper/live mode toggle guarded by TOTP + a parity
harness.

**Current operating mode: PAPER.** The engine is shipped and deployed in
`TRADING_MODE=paper` with no Moomoo broker credentials on the host. The risk
of accidental live execution is zero at the configuration layer. The
runtime toggle is wired and tested, but it refuses to flip to **live**
without a fresh `parity_verified.json` marker — and that marker is never
written against a live account until the equity strategy clears the deploy
gate (IC > 0.03 + positive equity curve).

```
                ZMQ REQ/REP (JSON, V3 protocol)
  inference  ───────────────────────────────►  engine
  (Python / Torch)   tcp://*:5555             (Rust / Tokio)
                                                │
                                                │  +-----+
                                                │  │SQLite│
                                                │  +--+---+
                                                │     │
                                                ▼     ▼
                                          +------------+   /api/* (Axum)
                                          |  Control    │ ◄──────────────┐
                                          |  Room (SPA) │                │
                                          +------------+                │
                                                ▲                        │
                                                │   /api/v1/ws (WS)      │
                                          scheduler + paper/live toggle │
                                                                        │
                                            Moomoo OpenD  (only wired  │
                                            (subprocess)   when LIVE_   │
                                                            EXECUTOR=   │
                                                            moomoo set) │
```

## Architecture

```
.
├── inference/      Python (PyTorch CPU) microservice. QQQ TCN model
│                   (pred_1d / pred_5d / pred_21d). Loads
│                   models/model_qqq_tcn.pt + norm_stats_qqq_v1.json.
│                   Answers ZMQ REQ with V3 JSON predictions.
│
├── engine/         Rust workspace member. Tokio runtime:
│                   - Daily equities ingest (Yahoo + Moomoo quote
│                     fallback + FRED macro series)
│                   - 8-dim feature pipeline (median/MAD normalize)
│                   - ZMQ REQ → scheduler → state machine
│                   - PaperExecutor (default) or MoomooExecutor
│                     (shells out to .agents/skills/moomooapi/scripts)
│                   - Axum HTTP + WebSocket telemetry
│                   - Strategy Lab (Rhai sandbox + backtest engine)
│
├── frontend/       Svelte 4 SPA built with Vite. Served by the engine
│                   (via ServeDir). WebSocket-driven, no 5s polling.
│
├── models/         model_qqq_tcn.pt, norm_stats_qqq_v1.json (gitignored)
│
├── deploy/         Docker Compose stack, Caddy reverse proxy, env
│                   reference, provisioning notes.
│
├── .hermes/plans/  Control Room revamp plan (5 phases). See
│   control-room/   PHASE_3_EXECUTION_SHORTING.md for the current
│                   execution-overhaul spec.
│
└── Training_model_ Design document for the QQQ equities model.
    Design.md
```

## Control Room dashboard

The Svelte frontend is rendered at `/` from the engine binary. The
control-room plumbing is independently built and tested:

- **WebSocket telemetry** (Phase 1) — `/api/v1/ws` broadcasts
  `PredictionUpdate`, `TradeFill`, `PnlTick`, `ModeChange` events to
  every connected client. The old 5s polling loop is retired.
- **Status panel** — live mode badge (PAPER/LIVE), QQQ symbol, current
  position badge (LONG/FLAT/SHORT), entry price, last close, realized
  + unrealized PnL, data staleness, WS connection dot.
- **Candlestick chart** — candles + 200-day SMA + prediction cones
  (1d/5d/21d) overlaid on the most recent bars.
- **Equity curve** — running PnL over the trade history.
- **Feature inspector** — 8-dim feature vector for the latest bar, with
  median/MAD-normalized values.
- **Strategy Lab** (Phase 2) — Monaco code editor for Rhai strategies,
  parameter sliders, A/B comparison, backtest engine that replays
  historical predictions through the same executor that runs live.
- **Mode toggle modal** (Phase 3.4) — single button next to the mode
  badge opens a TOTP prompt. Live flips are blocked when the parity
  marker is stale.

## Quick start — paper mode (the supported deployment)

Paper mode is the entire production story for now. No broker credentials
are required. The instance you deploy is an unattended paper-trading
sandbox that exercises every code path (features → inference → strategy
→ PaperExecutor → telemetry) including the long→short transition
through PSQ.

### Local dev (host-side)

```bash
git clone https://github.com/your-org/MarketMoves
cd MarketMoves

cp .env.example .env
# IMPORTANT: leave TRADING_MODE=paper (the default).
# Leave KRAKEN_* and MOOMOO_* empty — they are not consulted in paper mode.

# 1) Inference service
cd inference
uv sync
uv run python -m inference.inference_engine     # listens on tcp://*:5555

# 2) Engine (separate terminal)
cd ..
cargo build --release
cargo run --release --bin engine                # serves :8080
```

Open `http://localhost:8080`. You should see PAPER mode in the
status panel, WebSocket dot green, and the equities ingest beginning
(typically 400+ days of QQQ + SPY + TLT + GLD + NVDA + AAPL + ^VIX on
first launch).

### Docker Compose (recommended)

```bash
cp .env.example .env
# .env defaults to TRADING_MODE=paper. Leave it.

docker compose -f deploy/docker-compose.yml build
docker compose -f deploy/docker-compose.yml up -d
docker compose -f deploy/docker-compose.yml ps

curl -fsSL http://localhost:8080/api/status | jq    # sanity
```

See `deploy/README.md` for the full operational guide (backups, log
spooling, parity-marker refresh, Caddy TLS).

## Trading mode — the paper-first contract

| Mode    | When used                                        | Risk of live orders                                   |
|---------|--------------------------------------------------|-------------------------------------------------------|
| `paper` | Default. Every deployment ships this.            | None. No broker credentials even loaded.              |
| `live`  | Reserved for after the equity strategy clears the deploy gate.  | Engine refuses to start without a fresh parity marker. |

### `TRADING_MODE=paper` (the current default)

- Orders are simulated locally (`engine::exec::paper::PaperExecutor`).
- Fees are taken from `PAPER_FEE` (default 0.15% per fill).
- Fills are written to `equity_trades` with `side='buy'/'sale'`.
- Longs track QQQ; shorts are remapped to a synthetic PSQ position
  (entry/exit price simulated at the close that triggered the signal).
- No Moomoo REST calls. No broker subprocess. The
  `MoomooExecutor` is dead code in paper mode (the engine never
  instantiates it).

### `TRADING_MODE=live` (not currently used)

This is *wired* but not *enabled*. The engine supports it, and the
in-process toggle (`POST /api/mode`) works, but turning it on requires:

1. The QQQ equity strategy clears the **deploy gate**:
   - Walk-forward OOS mean **IC > 0.03** (vs the current ~0 — the
     V2 BTC TCN was retired for this exact reason).
   - **Positive equity curve** on the replay backtest.
   - **Fresh parity marker** (`parity_verified.json`) younger than
     `PARITY_MAX_AGE_SECS` (default 7 days).
2. **Moomoo broker credentials** provisioned on the host
   (OpenD daemon installed, GUI unlocked, `MOOMOO_SECURITY_FIRM` set).
   *None of these are on the current VPS.*
3. `TOTP_SECRET` persisted in `.env` so the operator can authorize
   a live flip via the dashboard.

Until those three are met, the runtime toggle is **a safety control
that always shows PAPER** — clicking it shows the parity status
(uninitialized / expired) and the modal's "Switch to LIVE" button is
disabled.

### Runtime mode toggle (Phase 3.4)

The WebSocket-aware ticker is paired with `POST /api/mode`. Both
sides are fully implemented; the toggle is exercised only in tests
today.

```bash
# GET: current mode + parity maker age
curl http://localhost:8080/api/mode

# POST: flip to live (requires a valid TOTP code + fresh parity marker)
curl -X POST http://localhost:8080/api/mode \
  -H 'Content-Type: application/json' \
  -d '{"mode":"live","auth_token":"123456"}'
```

Validation order on a live flip:
1. TOTP accepted against `TOTP_SECRET` (SHA-1, 6 digits, ±1 step skew).
2. `parity_verified.json` is present and younger than `PARITY_MAX_AGE_SECS`.
3. The shared `Arc<RwLock<TradingMode>>` is flipped.
4. An audit row is appended to `mode_switches`.
5. A `TelemetryEvent::ModeChange` is broadcast over WebSocket.

## Parity gate

Parity is the single release gate for the engine. The Rust feature
pipeline must reproduce the Colab feature engineering exactly —
`log_return`, ATR(14) (Wilder smoothing), typical price, 200-day SMA
regime, 8-dim feature vector under the median/MAD normalization
shipped in `models/norm_stats_qqq_v1.json`. Until the parity harness
reports a clean run against the golden fixtures,
`TRADING_VERIFIED.json` is not written, and the engine refuses to
start in live mode.

The harness is invoked via `cargo run --bin parity-harness --release`
and writes a marker JSON with `verified_at`, `fixture_sha256`, and
`max_abs_error`. The runtime `/api/mode` endpoint re-checks the marker
age at request time — it is not sufficient to have a marker at
startup.

## Shorting (PSQ inverse-ETF)

The strategy engine supports shorting but defaults to **off**. The
parameter set is:

| Env var                  | Default | Meaning                                          |
|--------------------------|---------|--------------------------------------------------|
| `ENABLE_SHORTING`        | `false` | When true, the bearish regime can produce Short. |
| `SHORT_ENTRY_THRESHOLD`  | `-0.004`| pred_1d below this → short entry.                |
| `SHORT_EXIT_THRESHOLD`   | `0.001` | pred_1d above this → exit short to flat.         |
| `SHORT_SYMBOL`           | `PSQ`   | Inverse ETF used as the short instrument.        |

The executor never short-sells QQQ. Instead, a Short target is
implemented as a **buy of PSQ** (ProShares Short QQQ, –1x). This
sidesteps margin, borrow, and locate requirements. The
PaperExecutor records the trade against the `short_symbol` so the
dashboard can show "SHORT" without confusing it with a naked short.

Transition safety: the state machine never jumps Long → Short in one
tick. The executor relies on a two-step transition (Long → Flat →
Short) which is implemented as two separate fills per cycle.

## Strategy Lab (Phase 2)

The Control Room adds a backtest engine and a Rhai scripting sandbox:

- **Backtest engine** — replays persisted `equity_predictions` through
  the same `PaperExecutor` logic that runs live, so backtest PnL
  matches production PnL bit-for-bit.
- **Rhai sandbox** — pure-Rust embedded scripting. Strategies can
  read `pred_1d` / `pred_5d` / `pred_21d` and the regime, then return
  `Position` (Flat / Long / Short).
- **Strategy storage** — saved to `strategy_configs` (SQLite). One
  active strategy at a time.
- **A/B comparison** — run two strategies over the same historical
  period and compare metrics side-by-side.

The lab is the operator's primary tool for evaluating whether the
deploy gate has cleared before risking anything live.

## Environment variables

Reference table — authoritative copy lives at `deploy/config.md`.

| Var                       | Purpose                                                       | Example                       |
|---------------------------|---------------------------------------------------------------|-------------------------------|
| `TRADING_MODE`            | `paper` (default) or `live`                                   | `paper`                       |
| `ZMQ_ENDPOINT`            | Inference socket                                              | `tcp://inference:5555`        |
| `HTTP_PORT`               | Axum server port                                              | `8080`                        |
| `SYMBOL`                  | Primary trading instrument (longs)                            | `QQQ`                         |
| `SHORT_SYMBOL`            | Inverse ETF (shorts)                                          | `PSQ`                         |
| `ENABLE_SHORTING`         | Master switch for shorting                                    | `false`                       |
| `SHORT_ENTRY_THRESHOLD`   | Short entry threshold (pred_1d)                               | `-0.004`                      |
| `SHORT_EXIT_THRESHOLD`    | Short exit threshold (pred_1d)                                | `0.001`                       |
| `MAGNITUDE_THRESHOLD`     | Long entry threshold (pred_1d)                                | `0.005`                       |
| `PAPER_FEE`               | Simulated fee rate (paper mode)                               | `0.0015`                      |
| `SMA_WINDOW`              | Regime SMA window (days)                                      | `200`                         |
| `FEATURE_WINDOW_SIZE`     | Candles sent to inference                                     | `126`                         |
| `DATABASE_URL`            | SQLite DSN                                                    | `sqlite://data/candles.db`    |
| `NORM_STATS_PATH`         | Norm stats JSON                                               | `models/norm_stats_qqq_v1.json` |
| `PARITY_MARKER_PATH`      | Parity marker file                                            | `parity_verified.json`        |
| `PARITY_MAX_AGE_SECS`     | Max age of the marker (default 7 days)                        | `604800`                      |
| `TOTP_SECRET`             | Base32 TOTP secret for `/api/mode` (empty → engine mints one) | `JBSWY3DPEHPK3PXP`            |
| `LIVE_EXECUTOR`           | `paper` (default) or `moomoo`                                 | `paper`                       |
| `MOOMOO_TRD_ENV`          | `SIMULATE` or `REAL` (only consulted when LIVE_EXECUTOR=moomoo) | `SIMULATE`                |
| `MOOMOO_CREDS_PATH`       | Moomoo OpenAPI credentials JSON                               | `~/.moomoo/credentials.json`  |
| `FRED_API_KEY`            | FRED API key (optional; higher rate limit)                     | —                             |
| `RUST_LOG`                | Tracing filter                                                | `info`                        |

Note: the legacy `KRAKEN_*` and BTC/USD variables in the example file
are retained for migration history but are **not consulted** in the
current QQQ pipeline. The engine ignores them.

## Plan & feature specs

The 5-phase Control Room revamp plan lives at
[`.hermes/plans/control-room/`](.hermes/plans/control-room/):

- `MASTER.md` — overview, locked contracts, design decisions.
- `PHASE_0_FOUNDATION.md` — API split, DB schema, Svelte scaffold.
- `PHASE_1_DASHBOARD_WEBSOCKET.md` — reactive dashboard, telemetry.
- `PHASE_2_STRATEGY_LAB_RHAI.md` — backtest engine, Rhai sandbox.
- `PHASE_3_EXECUTION_SHORTING.md` — PSQ shorting, mode toggle, TOTP.
- `PHASE_4_AI_ADVISOR.md` — LLM advisor (not yet started).

The QQQ pivot itself is documented in `Training_model_Design.md` and
`.hermes/plans/Trading Control Room Platform Design.md`.

## License

MIT. See [`Cargo.toml`](./Cargo.toml) workspace metadata.
