# Control Room Platform Redesign — Prompt for Gemini

You are a senior full-stack engineer + trading systems architect. Design a complete
"Control Room" web platform for MarketMarkovNet — a quantitative trading system that
runs a trained TCN+LightGBM ensemble model on QQQ daily equities data. The platform
must build upon the EXISTING system (described below), not replace it. Your proposal
must preserve every locked contract while extending the UI into a professional-grade
control room with strategy testing, paper/live toggle, PnL tracking, and an AI trading
advisor.

Read every section below as a HARD constraint unless marked "OPEN DECISION". Do not
invent APIs, DB tables, or endpoints that contradict the locked contracts. Where you
propose new endpoints, specify the exact route, method, request/response schema, and
which existing Rust module owns the handler.

---

## 1. SYSTEM ARCHITECTURE (locked facts)

### 1.1 Stack
- **Backend**: Rust (edition 2021), Axum 0.7, SQLx (SQLite), Tokio, ZeroMQ (zeromq crate).
- **Inference**: Python microservice (PyTorch + LightGBM), communicates via ZMQ REQ/REP.
- **Frontend**: Vanilla JS ES modules, no build step, no framework. Served as static
  files by Axum's `ServeDir` with SPA fallback to `index.html`.
- **Database**: SQLite. Single file at `data/candles.db` (configurable via `DATABASE_URL`).
- **Deploy**: Docker Compose on a VPS. Engine container + inference container.

### 1.2 Process flow (daily equities)
```
Yahoo Finance / FRED / Moomoo  →  Rust engine (data ingest)
                                    ↓
                              equity_candles table (SQLite)
                                    ↓
                    EquityScheduler (polls every 5 min for new daily candle)
                                    ↓
              compute_equity_features() → 8-dim feature vector per candle
                                    ↓
                    EquityNormStats (median/MAD normalization)
                                    ↓
              ZMQ REQ → Python inference (TCN + LightGBM ensemble)
                                    ↓
              pred_1d, pred_5d, pred_21d → equity_predictions table
                                    ↓
              next_equity_position() strategy → Position (Long/Flat)
                                    ↓
              PaperExecutor → equity_trades table → realized_pnl
                                    ↓
              Axum API → JSON → Vanilla JS SPA (polls every 5s)
```

### 1.3 Config (env-driven, `engine/src/config.rs`)
| Env var | Default | Purpose |
|---------|---------|---------|
| `TRADING_MODE` | `paper` | `paper` or `live`. Live requires fresh parity marker. |
| `ZMQ_ENDPOINT` | `tcp://127.0.0.1:5555` | ZMQ REP socket of inference service. |
| `MAGNITUDE_THRESHOLD` | `0.005` | Strategy entry threshold (crypto legacy). |
| `PAPER_FEE` | `0.0015` | Fee per paper trade (0.15%). |
| `SMA_WINDOW` | `200` | SMA regime filter window (days for equities). |
| `HTTP_PORT` | `8080` | Axum HTTP server port. |
| `SYMBOL` | `BTC/USD` | Traded symbol (set to `QQQ` for equities). |
| `DATABASE_URL` | `sqlite://data/candles.db` | SQLite path. |
| `NORM_STATS_PATH` | `models/norm_stats_qqq_v1.json` | Median/MAD normalization stats. |
| `FEATURE_WINDOW_SIZE` | `126` | TCN sequence length (~6 months daily bars). |
| `PARITY_MARKER_PATH` | `parity_verified.json` | Live-mode gate marker. |
| `PARITY_MAX_AGE_SECS` | `604800` | Max marker age (7 days) for live mode. |
| `MOOMOO_CREDS_PATH` | `~/.moomoo/credentials.json` | Moomoo OpenAPI credentials (Wave D, not yet active). |
| `FRED_API_KEY` | (empty) | Optional FRED API key for higher rate limits. |

---

## 2. DATABASE SCHEMA (locked — do not break)

SQLite tables in `engine/src/db.rs`:

### 2.1 Equity tables (active path)
```sql
equity_candles (symbol TEXT, ts INTEGER, open REAL, high REAL, low REAL,
                close REAL, volume INTEGER, PRIMARY KEY (symbol, ts))

equity_predictions (id INTEGER PRIMARY KEY, symbol TEXT, candle_ts INTEGER,
                    pred_1d REAL, pred_5d REAL, pred_21d REAL,
                    regime TEXT, features_json TEXT, created_at INTEGER,
                    source TEXT)

equity_trades (id INTEGER PRIMARY KEY, symbol TEXT, ts INTEGER,
               side TEXT, qty REAL, price REAL, fee REAL,
               realized_pnl REAL)

equity_ingest_state (symbol TEXT PRIMARY KEY, last_ts INTEGER, source TEXT)
```

### 2.2 Legacy crypto tables (dormant — do not remove, do not mutate)
```sql
candles, predictions, signal_state, positions, trades
```

---

## 3. API SURFACE (locked — existing endpoints)

All endpoints are `GET`, return JSON, and have permissive CORS (`Any` origin/method/header).

| Route | Handler | Returns |
|-------|---------|---------|
| `GET /api/status` | `handle_status` | `{mode, symbol, position, entry_price, realized_pnl, unrealized_pnl, last_candle_ts, last_close, pred_1d, pred_5d, pred_21d, pred_1h_approx, pred_5h_approx, staleness_secs, sma_200}` |
| `GET /api/market_state` | `handle_market_state` | Market open/closed state for current time. |
| `GET /api/predictions` | `handle_predictions` | `{latest: PredictionDto, history: [PredictionDto]}` — last 48 equity predictions. |
| `GET /api/accuracy` | `handle_accuracy` | Currently returns 503 ("equity accuracy not yet implemented"). Stub exists. |
| `GET /api/chart` | `handle_chart` | `{candles: [CandleDto], sma: [SmaPoint]}` — up to 500 recent candles + SMA overlay. |
| `GET /api/equity/data` | `handle_equity_data` | Raw equity candle data. |
| `GET /api/equity/backfill` | `handle_equity_backfill` | Trigger backfill of equity candles. |
| `GET /api/equity/macro` | `handle_equity_macro` | Macro series (VIX, UST10Y, DXY). |
| `GET /api/equity/features` | `handle_equity_features` | Computed feature rows for inspection. |

**PredictionDto shape**: `{candle_ts, pred_1h, pred_4h, pred_24h, pred_1h_approx, pred_5h_approx, created_at, actual_1h, actual_4h, actual_24h}`

**CandleDto shape**: `{ts, open, high, low, close, volume, vwap}`

**Static file serving**: `ServeDir::new("frontend")` with SPA fallback to `frontend/index.html`.

---

## 4. STRATEGY ENGINE (locked — `engine/src/strategy.rs`)

### 4.1 Position enum
```rust
pub enum Position { Flat = 0, Long = 1, Short = -1 }
```
The Rust type already supports Short. The current equities strategy is configured long/flat
only, but **the redesigned platform MUST support shorting as a first-class option** — the user
wants the ability to go short when the model predicts sustained downside. The design must
treat long/flat as the default but allow enabling long/short/flat (or any subset) per strategy
config. See §10.2 (Strategy Lab) and OD7.

### 4.2 Equity strategy (`next_equity_position`)
Current params (defaults shown):
```rust
pub struct EquityStrategyParams {
    pub entry_threshold: f64,   // default 0.003 — pred_1d must exceed this to go long
    pub exit_threshold: f64,    // default -0.001 — pred_1d below this exits to flat
    pub sma_window: usize,      // default 200 — SMA regime filter
}
```
**Current logic is long/flat only.** The redesign must extend this to an OPTIONAL short path:
when shorting is enabled in the strategy config, negative predictions below a `short_entry_threshold`
(in bearish regime: close < SMA200) should open a short position, with a corresponding
`short_exit_threshold` to cover. Specify the exact extended `EquityStrategyParams` fields and the
updated state-machine logic. The long/flat-only path must remain the default for backward compat.
pub struct EquitySignalInput {
    pub pred_1d: f64,
    pub pred_5d: f64,
    pub pred_21d: f64,
    pub current_close: f64,
    pub sma: f64,
    pub sma_valid: bool,
}
```

**Logic**:
1. Regime filter: if SMA invalid OR close <= SMA200 → block new longs (allow exits only).
2. Bullish regime (close > SMA200): entry if `pred_1d > entry_threshold AND pred_5d > 0`.
3. Exit: if long and `pred_1d < exit_threshold` → flat.
4. Otherwise hold current position.

### 4.3 Legacy crypto strategy (`next_position`)
Uses `pred_4h` + `pred_24h` with SMA regime, long/short/flat. **Dormant — do not mutate.**

### 4.4 Execution (`engine/src/exec/`)
- `PaperExecutor` only. Live executor not yet wired.
- `FillResult`: `{side: Buy|Sell, qty, price, fee, realized_pnl, ts}`.
- `paper_fee` configurable (default 0.0015 = 0.15%).

---

## 5. INFERENCE SERVICE (locked — `inference/`)

### 5.1 ZMQ protocol (V3, equities)
```
Request  → {"schema_version": 3, "feature_window": [[f0,f1,...,f7], ...]}
Response → {"pred_1d": float, "pred_5d": float, "pred_21d": float}
```
- `feature_window` is a sequence of 8-dim normalized feature vectors (seq_len × 8).
- The Rust engine normalizes features with median/MAD BEFORE sending.
- The TCN consumes the full sequence; LightGBM consumes only the last timestep.

### 5.2 Model architecture (`inference/equity_model.py`)
- **QqqTCN**: `input_proj: Linear(8→64)` → 7× `ResidualBlock` (CausalConv1d, dilations
  [1,2,4,8,16,32,64], GroupNorm(1,64), SiLU, dropout=0.1) → 3 horizon heads
  (`Linear(64→32) → SiLU → Dropout → Linear(32→1)`).
- **CausalConv1d**: left-only padding (no look-ahead). Wraps `nn.Conv1d` as `self.conv`.
- **LightGBM**: 3 separate Booster models (h1, h5, h21), loaded from pickle.
- **EquityEnsemble**: z-score normalized weighted average (default TCN 0.5, LGBM 0.5).
  Weights should be tuned on walk-forward OOS IC.

### 5.3 Legacy inference (`inference/model.py`, `inference/inference_engine.py`)
- `MarketMarkovNet`: 6-layer causal CNN, 3 heads (1h/4h/24h), 2 Markov refinement heads.
- ZMQ V1/V2 protocol. **Dormant — do not mutate.**

---

## 6. FEATURE PIPELINE (locked — `engine/src/features/equities_v2.rs`)

### 6.1 8-dim feature vector (MUST stay in this order)
| idx | name | definition | range |
|-----|------|-----------|-------|
| 0 | trend_slope | ln(SMA50[t] / SMA50[t-20]) | any |
| 1 | trend_adx | Wilder ADX(14), trend strength | 0–100 |
| 2 | rsi_14 | Wilder RSI(14), momentum | 0–100 |
| 3 | vix_regime | bucketed ^VIX: <18→0 (calm), <25→1 (normal), else 2 (stress) | {0,1,2} |
| 4 | tlt_corr_20d | 20-day rolling Pearson corr(QQQ close, TLT close) | -1..1 |
| 5 | rvol_20d | volume[t] / mean(volume[t-20..t]) | >=0 |
| 6 | gap_pct | (open[t] - close[t-1]) / close[t-1] | any |
| 7 | drawdown_from_50d_high | (close[t] - max(high[t-50..t])) / max(...) | <=0 |

### 6.2 Normalization
- Robust median/MAD: `(x - median) / (1.4826 * MAD)`. If MAD ≈ 0 → 0.0.
- Serialized to `norm_stats_qqq_v1.json`: `{median: [8 floats], mad: [8 floats]}`.
- MUST be identical at train and inference time.
- `EQ_FEATURE_DIM = 8`. Training/inference MUST consume this exact 8-vector in this order.

### 6.3 LLM regime feature (`engine/src/features/llm.rs`)
- OpenRouter hourly-cached LLM regime classification → `llm_bull_prob` in [0,1].
- Cached in process-wide `RwLock<f64>`. LLM call is NEVER in the per-bar latency path.
- Currently crypto-framed. Could be reframed for equities as a regime/strategy advisor.
- Config: `OPENROUTER_API_KEY`, `LLM_MODEL`, `LLM_API_BASE`, `LLM_CACHE_TTL_SECONDS`.

---

## 7. DATA SOURCES (locked — `engine/src/data/`)

### 7.1 Yahoo Finance (`yahoo.rs`) — PRIMARY
- Free, no auth, deep history. Endpoint: `query1.finance.yahoo.com/v8/finance/chart/{symbol}`.
- Symbols: QQQ, AAPL, MSFT, NVDA, GOOG, AMZN, META, TSLA, TLT, GLD, UUP, ^VIX.
- Daily interval. Stored in `equity_candles`.

### 7.2 FRED (`fred.rs`) — MACRO
- Free daily series. Optional API key for higher rate limits.
- Series: VIXCLS→$VIX, DGS10→$UST10Y, DTWEXBGS→$DXY.
- Stored in `equity_candles` with `$` prefix.

### 7.3 Moomoo OpenAPI (`moomoo.rs`) — WAVE D (interface only, not yet active)
- Requires OpenD gateway daemon + credentials JSON at `~/.moomoo/credentials.json`.
- Fields: `{api_key, api_secret, host, port}`.
- Until activated, all functions return "not configured" → callers fall back to Yahoo.

### 7.4 API recommendations for the control room (NEW — evaluate and recommend)
The control room platform needs capabilities that go beyond the current data sources.
For each category below, recommend specific APIs/services, justify the choice (cost,
reliability, coverage, ease of integration with Rust), and specify how they'd plug into
the existing `engine/src/data/` module pattern:

- **Real-time / intraday market data**: The platform currently only has daily candles
  from Yahoo. For a control room that tracks live PnL, intraday or real-time quotes are
  needed. Evaluate: Yahoo (free but rate-limited/unreliable for real-time), Polygon.io,
  Finnhub, Alpaca Markets, Tiingo, IEX Cloud, Twelve Data. Consider WebSocket streaming
  vs REST polling, free tier limits, and delay (15-min vs real-time).

- **Order execution / brokerage (for live trading)**: The system currently only has a
  paper executor. For eventual live trading, evaluate broker APIs that support equities
  (especially QQQ/ETF shorting): Alpaca (REST + streaming, supports shorting, paper +
  live), Interactive Brokers (TWS API / ib_async), Moomoo OpenAPI (already interfaced),
  Charles Schwab / TD Ameritrade. Consider: short availability/borrow fees, API rate
  limits, Rust client availability (or HTTP/REST wrapper), and paper-to-live parity.

- **Options / derivatives data** (optional, if shorting via options is viable): Evaluate
  whether the shorting requirement is better met via inverse ETFs (e.g. PSQ for QQQ),
  options (puts), or direct short selling. Recommend the simplest path.

- **News / sentiment feed** (for the AI advisor context): Evaluate news APIs that could
  feed the LLM advisor with market context: Alpha Vantage (news + sentiment), Finnhub
  (company news), Polygon.io (news), NewsAPI, GDELT (free global news). Consider free
  tier limits, sentiment scoring, and whether the API provides structured data the LLM
  can consume directly.

- **Alternative data** (optional, for feature enrichment): Evaluate whether any of these
  could extend the 8-feature pipeline as additive features (schema_version bump, base-8
  at indices 0..7 preserved): options flow / dark pool data, social sentiment (Reddit,
  X/Twitter), earnings calendars, economic calendars (FRED already partially covers this).

For each recommended API, specify: the Rust integration approach (new module under
`engine/src/data/`), the auth model (API key env var), rate limits, free tier adequacy,
and whether it needs a new DB table or extends `equity_candles`.

---

## 8. CURRENT FRONTEND (replaceable — `frontend/`)

The current SPA is a minimal MVP. **It does NOT need to be preserved as-is.** The existing
panels can be repurposed, redistributed, or replaced entirely. The goal is a professional
control room, not an incremental polish of the current layout. The only hard constraints are:
(1) it must still be served as static files by Axum's `ServeDir`, and (2) it must consume the
existing API endpoints (§3) plus any new ones you design.

### 8.1 Current structure (for reference — rewrite freely)
```
frontend/
  index.html          — SPA shell, dark theme, 4 panels in a simple grid
  style.css           — GitHub-dark palette (#0d1117 bg, #3fb950 green, #f85149 red, #58a6ff blue)
  app.js              — Entry point, view registry, 5s poll loop
  api.js              — fetchStatus, fetchPredictions, fetchChart, fetchAccuracy
  views/
    status.js         — mode, symbol, position, entry, PnL, staleness
    predictions.js    — pred 1D/5D/21D + scaled 1H/5H, history table
    chart.js          — uPlot candlestick + 200-SMA overlay
    accuracy.js       — directional accuracy + MAE (graceful 503 handling)
  vendor/
    uPlot.min.css, uPlot.min.js
```

### 8.2 What exists that can be reused
- uPlot charting library (candlestick + SMA overlay) — lightweight, no build step.
- ES module view pattern (`render(rootEl, data)`) — works but can be replaced.
- 5s poll loop + 60s accuracy poll.
- GitHub-dark color palette (keep as design system base if it fits the new design).

### 8.3 What's wrong with the current design
- 4 static panels in a fixed grid — no navigation, no drill-down, no responsiveness.
- No strategy testing UI, no PnL visualization, no trade history.
- No real-time updates (polling only).
- No dashboard layout flexibility (widgets, tabs, resizable panels).
- Read-only — no controls to change strategy params, toggle modes, or interact with the advisor.

---

## 9. DEPLOY GATE (locked)

- Walk-forward OOS evaluation: **IC > 0.03 AND mean OOS equity > 0** required to deploy.
- IC = Pearson correlation between predicted and actual magnitudes, per horizon.
- If gate fails: model is NOT deployed. System stays in paper mode with the last good model.
- Live mode additionally requires a fresh `parity_verified.json` marker (max 7 days old),
  written by the regression parity harness (Feature 13).

---

## 10. YOUR TASK — DESIGN THE CONTROL ROOM PLATFORM

Design a full-fledged control room web platform that builds on the above. The platform
must deliver these capabilities:

### 10.1 Dashboard (full redesign — not constrained to current panels)
- Propose a professional control room layout. The current 4-panel grid is a starting
  reference only — redistribute, merge, or replace panels as you see fit. Think trading
  desk, not status page.
- Recommend whether to keep vanilla JS / no-build-step or migrate to Vite + a lightweight
  framework (Preact, Svelte, Solid). Justify the trade-off: the SPA will grow significantly
  with strategy lab, PnL charts, advisor chat, and backtest visualization. If you recommend
  a framework, specify the exact migration path from the current ES module pattern and how
  the Axum `ServeDir` static serving adapts (e.g. Vite `dist/` output served by ServeDir).
- Must include at minimum: candlestick chart with position/overlay, PnL/equity curve,
  trade history log, feature inspector (8 live features + normalized values + sparklines),
  model health panel (staleness, inference latency, TCN vs LightGBM z-score spread, IC drift).
- Must be responsive and support a multi-panel / widget layout (tabs, resizable grids, or
  sidebar navigation — recommend the best approach for a trading dashboard).

### 10.2 Strategy Lab (NEW — test different strategies, full extensibility)
- Allow the user to define and backtest alternative strategy parameter sets WITHOUT
  touching Rust code. This means:
  - A UI form to set `entry_threshold`, `exit_threshold`, `sma_window`, `short_entry_threshold`,
    `short_exit_threshold`, `enable_shorting` (bool), and any new params you propose
    (e.g. `pred_5d_confirmation_threshold`, `vix_regime_filter`, `max_position_hold_days`).
  - A backtest engine that replays historical `equity_predictions` through the proposed
    strategy and shows: equity curve, max drawdown, Sharpe ratio, win rate, trade count,
    IC of the strategy vs buy-and-hold.
  - A/B comparison: run two strategy configs side by side and overlay their equity curves.
- **Shorting support**: the strategy config must allow enabling short positions. When enabled,
  negative predictions (below `short_entry_threshold`) in bearish regime open shorts; the
  backtest must correctly compute PnL for short trades (profit when price falls). Specify the
  exact short-entry/exit logic and how it integrates with the existing long/flat default.
- **Strategy extensibility / plugin system**: this is a hard requirement. The user wants the
  ability to test strategies that go beyond the current threshold-based state machine. Design
  a strategy interface that allows plugging in alternative strategy implementations. Options
  to evaluate (recommend one with justification):
  - (a) **WASM plugins**: user writes a strategy in any language that compiles to WASM; the
    Rust engine loads and executes it in a sandbox. Specify the host function API the plugin
    sees (e.g. `fn next_position(history: &[SignalInput], current: Position) -> Position`).
  - (b) **Embedded scripting**: embed a scripting language (Rhai, Lua via `mlua`, Deno core)
    in the Rust engine; user writes strategy logic as a script loaded at runtime. Specify the
    script API and sandboxing.
  - (c) **Python subprocess / IPC**: strategies defined as Python scripts that the Rust engine
    calls via ZMQ or stdin/stdout (similar to the existing inference bridge). Specify the protocol.
  - (d) **Declarative rule DSL**: a JSON/YAML rule language (if/then conditions on predictions
    and features) parsed by the Rust engine. Simpler but less expressive.
  The chosen approach must support: multi-position sizing, stop-loss / take-profit rules,
  multi-signal confirmation, and regime-conditioned logic. Strategies must be sandboxed
  (no filesystem/network access from user strategies).
- Specify the new API endpoint(s) needed (e.g. `POST /api/backtest` with strategy params/rules
  + date range → returns equity curve + metrics; `GET/POST /api/strategies` for save/load).
  Define the request/response schema.
- Specify whether the backtest engine runs in Rust (compiled strategy replay) or Python
  (notebook-style). Recommend the approach and justify.
- Allow saving/loading named strategy configs to SQLite (new table schema).

### 10.3 Paper/Live Toggle (NEW — runtime mode switch)
- Currently `TRADING_MODE` is set at startup via env var. Design a runtime toggle that:
  - Shows the current mode prominently (badge already exists — enhance it).
  - Allows switching paper → live from the UI with a confirmation dialog.
  - Enforces the parity marker check before allowing live mode (show marker age/freshness).
  - Logs all mode switches to a new `mode_switches` audit table.
  - Specifies the new API endpoint(s): `GET /api/mode`, `POST /api/mode` (with body
    `{mode: "live"}`, returns 403 if parity marker is stale/missing).
  - Addresses: can the Rust `TradingMode` enum be made mutable at runtime? If so, how
    (e.g. `Arc<RwLock<TradingMode>>` in `AppState`)? If not, what's the alternative
    (e.g. restart with new env, or a separate executor swap)?

### 10.4 PnL Tracking (enhance existing)
- The `equity_trades` table already stores `realized_pnl` per trade. Extend to show:
  - Daily PnL breakdown (realized + unrealized mark-to-market).
  - Cumulative equity curve (from `sum_equity_realized_pnl` + current unrealized).
  - Per-trade PnL log with entry/exit timestamps, prices, fees, and the prediction that
    triggered the trade.
  - Performance metrics: total return, CAGR, Sharpe, Sortino, max drawdown, win rate,
    profit factor, average win/loss.
- Specify which metrics can be computed from existing data and which need new DB queries
  or materialized views.

### 10.5 AI Trading Advisor (NEW — the key innovation)
This is the most important new feature. Design an integration where an LLM (via
OpenRouter, same as the existing `llm.rs` regime cache) acts as a "trading expert" that:

- **Consumes the trained model's predictions** (pred_1d, pred_5d, pred_21d) plus the
  8 live feature values, current position, recent PnL, and market state.
- **Suggests strategy adjustments**: e.g. "tighten entry_threshold to 0.004 given
  elevated VIX", or "pred_5d disagrees with pred_1d — consider waiting for confirmation".
- **Provides a daily briefing**: a natural-language summary of market regime, model
  confidence, and recommended action (enter / hold / exit / wait).
- **Acts as a second opinion**: when the model says "go long", the advisor can flag
  risk factors (e.g. "RSI overbought, VIX spiking, correlation breakdown with TLT").
- **Is NOT autonomous**: it advises only. The user makes the final call. The advisor's
  suggestions are logged for review but never auto-executed.

Design specifics:
- Specify the OpenRouter model(s) to use. Recommend a model that is strong at reasoning
  about financial data and strategy. Consider: `openrouter/anthropic/claude-sonnet-4`,
  `openrouter/openai/gpt-4o`, `openrouter/google/gemini-3.1-pro-preview`,
  `openrouter/deepseek/deepseek-r1-0528`. Justify your choice (cost, quality, speed).
- Specify the prompt template: what context is sent to the LLM (predictions, features,
  position, PnL, market state) and what format the response should take (structured JSON
  with fields like `{action, confidence, reasoning, suggested_params}`).
- Specify the API endpoint: e.g. `GET /api/advisor/briefing` → returns the latest cached
  briefing. `POST /api/advisor/ask` with a user question → streams the LLM response.
- Specify the caching strategy: like `llm.rs`, the daily briefing should be cached (TTL
  ~1h or on new prediction) so the LLM call is NOT in the per-bar path.
- Specify how the advisor integrates with the Strategy Lab: the advisor can suggest
  parameter changes that the user can one-click apply to a backtest.
- Specify the new Rust module (e.g. `engine/src/advisor.rs`) and how it reuses the
  `llm.rs` OpenRouter client pattern.
- Address safety: the advisor must never directly trigger trades. All suggestions require
  explicit user action. Log all advisor outputs to an `advisor_log` table for audit.

### 10.6 Real-time updates (enhance polling)
- The current 5s poll loop works but is wasteful. Evaluate: should the platform move to
  WebSocket (the `tokio-tungstenite` dep is already in Cargo.toml) or Server-Sent Events
  for push-based updates? Specify the migration path if so.
- At minimum, the PnL and position panels should update in real-time when a trade executes.

---

## 11. OPEN DECISIONS (state your recommended default + rationale)

- **OD1 Frontend framework**: Keep vanilla JS ES modules (zero disruption) vs migrate to
  Vite + Preact/Svelte (better DX for a growing SPA). RECOMMEND: state trade-offs.
- **OD2 Backtest engine**: Rust-compiled strategy replay (fast, type-safe, reuses
  `strategy.rs`) vs Python notebook (flexible, uses pandas, matches Colab training).
  RECOMMEND with justification.
- **OD3 Advisor model**: Which OpenRouter model for the trading advisor? Consider cost
  per request (the briefing runs ~hourly), reasoning quality on financial data, and
  latency. RECOMMEND a primary + a cheaper fallback.
- **OD4 Real-time transport**: WebSocket vs SSE vs keep polling. RECOMMEND.
- **OD5 Strategy config storage**: New SQLite table vs JSON files vs env vars.
  RECOMMEND.
- **OD6 Live mode toggle safety**: Should switching to live require a 2FA / password /
  time-delay? RECOMMEND the safety mechanism.
- **OD7 Shorting strategy**: How should shorting be implemented? (a) Direct short selling
  via broker (requires borrow availability, margin account, borrow fees), (b) inverse ETFs
  (e.g. PSQ for QQQ — simpler, no borrow, but expense ratio + tracking error), (c) options
  (puts — leveraged, time decay, complexity). RECOMMEND the simplest path that works with
  the user's Moomoo brokerage and a daily-horizon model. Also specify: should shorting be
  gated behind a config flag (default off) with a separate risk-budget cap?
- **OD8 Strategy plugin system**: Which extensibility approach? (a) WASM sandbox, (b) embedded
  scripting (Rhai/Lua), (c) Python IPC, (d) declarative DSL. Consider: sandboxing safety,
  expressiveness, user familiarity (the user trains models in Python/Colab), Rust crate
  maturity, and performance. RECOMMEND with justification.
- **OD9 Real-time data provider**: Which API for intraday/live quotes? Balance cost vs
  latency vs reliability. Consider whether the free Yahoo tier is sufficient for MVP or if
  a paid provider (Polygon, Alpaca, Finnhub) is needed from day one. RECOMMEND.

---

## 12. OUTPUT FORMAT

Return a structured design document with these sections:

1. **Executive Summary** — what the platform becomes, in 3 sentences.
2. **Architecture Diagram** — text/ASCII diagram of the extended system showing all
   new components and their connections to existing ones.
3. **New API Endpoints** — table: route, method, request schema, response schema,
   owning Rust module, purpose.
4. **New DB Tables** — schema for any new tables (strategy_configs, mode_switches,
   advisor_log, backtest_results, etc.).
5. **Frontend Plan** — full file structure, recommended framework (or no-framework
   justification), component/view layout, how the dashboard is organized (navigation,
   widget grid, etc.). The current 4-panel layout is a reference only — redesign freely.
6. **Strategy Lab Design** — UI mockup (text), backtest flow, A/B comparison, parameter
   form fields (including shorting params), save/load mechanism, AND the plugin/extensibility
   system (the chosen approach from OD8, with the strategy interface API spec).
7. **Shorting Design** — how short positions work end-to-end: strategy logic, execution
   (direct short vs inverse ETF vs options per OD7), PnL calculation, risk controls,
   and the config flag to enable/disable.
8. **Paper/Live Toggle Design** — runtime mode switch mechanism, safety checks,
   audit trail, API contract.
9. **PnL Tracking Design** — metrics computed, charts, data flow from equity_trades
   (including short trade PnL).
10. **AI Trading Advisor Design** — model choice, prompt template, response schema,
    caching strategy, API endpoints, Rust module structure, safety constraints,
    integration with Strategy Lab.
11. **Real-time Updates** — transport choice, migration path.
12. **API / Third-Party Service Recommendations** — table: category (real-time data,
    brokerage, news, alt data), recommended service, cost, free tier, Rust integration
    approach, priority (MVP vs later).
13. **Implementation Phases** — ordered phases (P1 = MVP, P2 = Strategy Lab, etc.)
    with estimated effort per phase. Each phase must be independently deployable.
14. **Risks & Open Questions** — what could go wrong, what needs user input.

Be precise and implementation-ready. Cite the specific file/module/contract you are
preserving or extending. Do not propose changes that break the locked contracts in
sections 2–6 and §9 (data sources, DB schema, feature pipeline, inference protocol,
deploy gate). Where you need a new contract (e.g. `predict_v4` or a new DB table),
specify it fully and show backward compatibility.
