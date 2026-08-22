# MarketMoves — locked contracts (QQQ equities pivot)

Condensed, non-obvious facts for quick re-grounding. Verify against the repo before trusting any
number; this is a memory aid, not the source of truth.

## Wave structure
- A = data client (DONE): `backfill_equities()` seeds 11 Yahoo symbols (QQQ, AAPL, MSFT, NVDA, GOOG,
  AMZN, META, TSLA, TLT, GLD, UUP) + 3 FRED macro ($VIX=VIXCLS, $UST10Y=DGS10, $DXY=DTWEXBGS) →
  SQLite `equity_candles` (PK symbol,ts; ts = midnight UTC). Supervisor: `run_equities_ingestion()`.
- B = features (DONE): 8-dim pipeline, `engine/src/features/equities_v2.rs`, 14 tests.
- C = models (DONE): LightGBM + TCN, horizons 1d/5d/21d, walk-forward 5y/1y, **deploy gate IC > 0.03**
  (matches `training/train_tcn.py` `ic_gate = 0.03`). Wave C ALSO owns the daily strategy/bridge
  rewrite (retire crypto `next_position` dormant, add `predict_v3` + daily `next_equity_position`).
- D = paper (active). E = live (not yet wired).

## Feature contract (Wave B) — DO NOT REORDER
`compute_equity_features(qqq, vix_close, tlt_close)` → `EquityFeatureRow`, `to_array()` order:
0 trend_slope (ln SMA50[t]/SMA50[t-20]); 1 trend_adx (Wilder ADX14); 2 rsi_14 (Wilder RSI14);
3 vix_regime (<=0 or <18→0, <25→1, else 2); 4 tlt_corr_20d (20d Pearson QQQ vs TLT);
5 rvol_20d (vol[t]/mean vol[t-20..t]); 6 gap_pct ((open[t]-close[t-1])/close[t-1]);
7 drawdown_from_50d_high. `EQ_FEATURE_DIM = 8`. vix/tlt must be timestamp-align to qqq else 0.0.
Norm: robust median/MAD → (x-median)/(1.4826*MAD), MAD≈0 ⇒ 0.0. `EquityNormStats` →
`models/norm_stats_qqq_v1.json`. Must match at train AND inference.

## Backbone to port
`training/train_tcn.py`: CausalConv1d + 7 ResidualBlocks dilations [1,2,4,8,16,32,64], GroupNorm(1),
SiLU, dropout 0.1, hidden=64, 3 horizon heads, adaptive `loss_weights` (softmax), SmoothL1Loss,
AdamW(5e-4, wd 1e-4), OneCycleLR(pct 0.3), grad-clip 1.0, epochs 150, early-stop patience 10.
`training/labels.py`: ATR-penetration barriers `k=c*ATR/close`, time-weighted magnitude, mag_clip=3.0,
`calibrate_barrier_c` to ~50% penetration. Walk-forward: `TimeSeriesSplit(n_splits=5, gap=embargo)`.
Gate: `check_deploy_gate` → IC > ic_gate AND equity > 0.

## Serving / storage
DB `equity_predictions(pred_1d, pred_5d, pred_21d, regime, features_json, source)`.
`engine/src/bridge.rs`: `predict` (legacy [[f64;3]]) + `predict_v2` ([[f64;6]], schema_version=2,
dormant). Wave C added `predict_v3` (8-dim / extended, daily horizons).
`engine/src/strategy.rs` `next_equity_position` consumes `pred_1d` + `pred_5d` + 200-day SMA regime —
EQUITIES contract (replaces dormant crypto `next_position` pred_4h/pred_24h).

## Strategy (Wave C equities) — note: defaults vs contract
`next_equity_position` is currently configured long/flat-only (entry_threshold 0.003,
exit_threshold -0.001, SMA 200). BUT the `Position` enum supports Short (= -1) and the user wants
shorting available as a first-class option. Long/flat-only is a CONFIG DEFAULT, not a locked
contract. Extension to long/short/flat is an open design decision, not a contract break.

## Frontend — replaceable, not locked
`frontend/` is a vanilla JS SPA (no build step) with 4 panels: status, predictions, chart (uPlot),
accuracy. This is an MVP reference, NOT a locked design. The user wants a full control-room
redesign: strategy lab, PnL tracking, AI advisor, backtest viz. Served as static files by Axum
`ServeDir` with SPA fallback — that's the only hard constraint on the frontend.

## API surface (reference — can be extended)
9 GET endpoints under `/api/`: status, market_state, predictions, accuracy, chart, equity/data,
equity/backfill, equity/macro, equity/features. All return JSON with permissive CORS. New POST
endpoints (mode toggle, backtest, strategies, advisor) are expected for the control-room redesign.

## Contract mismatch (RESOLVED in Wave C)
crypto 1h/4h/24h vs equities 1d/5d/21d; FEATURE_DIM=6 (crypto) vs 8 (equities); `next_position`
pred_4h/pred_24h vs daily pred_1d/pred_5d/pred_21d. Resolved: crypto path retired dormant,
`predict_v3` + `next_equity_position` added for daily equities.

## Control Room Redesign — resolved design decisions (2026-07-28)

The user received a Gemini design document proposing a 4-phase platform revamp. The design
decisions below are now RESOLVED (the user accepted the design doc and asked for implementation
plans). These are not yet coded but are the agreed-upon architecture:

### Phase sequence (plan at `.hermes/plans/control-room/`)
- Phase 0: Foundation — split `api.rs` into `engine/src/api/` module tree, add 4 new DB tables
  (strategy_configs, mode_switches, advisor_log, backtest_results), add crate deps (rhai,
  rust-embed, totp-rs, mime_guess), scaffold Vite+Svelte frontend, wire rust-embed SPA fallback.
- Phase 1: Dashboard + WebSocket — Svelte widget dashboard, tokio-tungstenite broadcast channel
  replacing 5s polling, retire old frontend.
- Phase 2: Strategy Lab — Rust replay backtest engine, Rhai scripting sandbox, Strategy Lab UI
  with Standard (sliders) + Advanced (Monaco editor) modes, A/B comparison.
- Phase 3: Execution + Shorting — extend EquityStrategyParams with shorting fields (default off),
  PSQ inverse-ETF execution, runtime paper/live toggle with TOTP + parity validation.
- Phase 4: AI Advisor — `engine/src/advisor.rs` module, DeepSeek V4 Flash via OpenRouter, hourly
  cached briefing, SSE streaming chat, advisor_log audit, one-click "Test in Strategy Lab".

### Resolved open decisions
| OD  | Topic              | Decision                                             |
|-----|--------------------|------------------------------------------------------|
| OD1 | Frontend framework | Vite + Svelte, compiled via rust-embed into binary   |
| OD2 | Backtest engine    | Rust-compiled replay (parity with live executor)     |
| OD3 | Advisor model      | DeepSeek V4 Flash primary, Claude 3.5 Sonnet fallback|
| OD4 | Real-time transport| WebSocket (tokio-tungstenite broadcast channel)      |
| OD5 | Strategy storage   | SQLite strategy_configs table                        |
| OD6 | Live toggle safety | TOTP + parity marker validation (7-day max age)      |
| OD7 | Shorting mechanism | Inverse ETF (PSQ for QQQ) — no margin/borrow needed  |
| OD8 | Strategy plugins   | Rhai embedded scripting (pure-Rust sandbox)          |
| OD9 | Real-time data     | Moomoo OpenD (local protobuf TCP, port 11111)        |

### New API endpoints (additive — existing 9 GETs untouched)
- `GET/POST /api/mode` — runtime paper/live toggle with TOTP
- `POST /api/backtest` — historical strategy replay
- `GET/POST /api/strategies` — save/load strategy configs
- `GET /api/advisor/briefing` — cached hourly LLM briefing
- `POST /api/advisor/ask` — SSE streaming chat
- `GET /api/v1/ws` — WebSocket telemetry (PnlTick, PredictionUpdate, FeatureUpdate, TradeFill, ModeChange)

### New DB tables (additive — existing 5 equity tables untouched)
- `strategy_configs` (id, name, strategy_type, script_body, params_json, is_active, timestamps)
- `mode_switches` (id, previous_mode, new_mode, parity_marker_age_secs, authorized_by, timestamp)
- `advisor_log` (id, interaction_type, prompt_context_json, model_used, response_json, suggested_action, timestamp)
- `backtest_results` (id, strategy_id FK, start_ts, end_ts, metrics_json, equity_curve_json, timestamp)

### Key codebase facts verified during planning (2026-07-28)
- `engine/src/api.rs` is a single 19K char file — no `api/` directory yet. AppState is inline.
- `engine/src/exec/` has `mod.rs` + `paper.rs` only. No live executor. ExecutorKind enum has Paper variant only.
- `PaperExecutor` already handles Position::Short PnL correctly (`(entry - exit) * qty - fee`).
- `EquityStrategyParams` has 3 fields (entry_threshold, exit_threshold, sma_window). No shorting fields yet.
- `TradingMode` is parsed from env at startup (static). No runtime toggle. No TOTP.
- `llm.rs` has the OpenRouter client pattern (RwLock cache, hourly TTL) — reusable for advisor.
- `tokio-tungstenite` is already in Cargo.toml workspace deps but not used yet.
- `tower-http` ServeDir is the current static serving mechanism — will be replaced by rust-embed.
- Frontend has no package.json — pure vanilla JS ES modules with uPlot vendor lib.
