# Control Room Platform Revamp — Master Plan

**Source design**: `.hermes/plans/Trading Control Room Platform Design.md`
**Scope**: Retire the current vanilla JS 4-panel SPA. Build a professional control room
with reactive frontend, WebSocket telemetry, strategy lab (Rhai), shorting (PSQ inverse ETF),
runtime paper/live toggle (TOTP), and AI trading advisor.

---

## Current State → Target State

| Layer          | Current                         | Target                                              |
|----------------|---------------------------------|-----------------------------------------------------|
| Frontend       | Vanilla JS, 4 panels, 5s poll   | Vite + Svelte, widget layout, WebSocket real-time   |
| Static serving | `ServeDir::new("frontend")`     | `rust-embed` — SPA compiled into binary             |
| API            | Single `api.rs` (9 GET routes)  | Split into `engine/src/api/` modules + 6 new routes |
| Strategy       | `EquityStrategyParams` long/flat| + shorting fields + Rhai plugin sandbox             |
| Execution      | `PaperExecutor` only            | + Live executor via Moomoo OpenD                    |
| Mode           | `TradingMode` from env (static) | `Arc<RwLock<TradingMode>>` runtime + TOTP           |
| Real-time      | 5s polling                      | tokio-tungstenite WebSocket broadcast               |
| DB             | 5 equity tables (locked)        | +4 new tables (configs, switches, advisor, backtest)|
| Advisor        | `llm.rs` crypto regime cache    | New `advisor.rs` — DeepSeek via OpenRouter          |

## Phase Sequence

**Phase 0 — Foundation** (3–4 days)
Backend groundwork that everything else depends on: split `api.rs` into a module tree,
add new crate deps (rhai, rust-embed, totp, etc.), create the 4 new DB tables, scaffold
the Vite + Svelte frontend, and wire `rust-embed` into Axum. The old frontend keeps working
during dev; hard cutover at deploy time.

**Phase 1 — Reactive Dashboard & WebSocket Telemetry** (~2 weeks)
Build the new Svelte dashboard with widget layout, replace polling with WebSocket broadcast,
visualize live PnL + 8-dim features + prediction cones. This is the phase where the old
frontend is retired and the new one goes live.

**Phase 2 — Strategy Lab & Rhai Integration** (~3 weeks)
Backtest engine (Rust replay over historical predictions), Rhai scripting sandbox,
Strategy Lab UI with parameter editor + Monaco code editor, A/B comparison.

**Phase 3 — Execution Overhaul & Shorting** (~2 weeks)
Extend `EquityStrategyParams` with shorting fields, implement PSQ inverse-ETF execution,
runtime paper/live toggle with TOTP + parity validation, mode_switches audit trail.

**Phase 4 — AI Trading Advisor** (~2 weeks)
`engine/src/advisor.rs` module, DeepSeek V4 Flash via OpenRouter, hourly cached briefing,
conversational chat with SSE streaming, advisor_log audit table, Strategy Lab integration.

## Locked Contracts (must not break)

- `equity_candles`, `equity_predictions`, `equity_trades` table schemas
- 8-dim feature vector order + `norm_stats_qqq_v1.json` normalization
- ZMQ V3 inference protocol (`predict_v3` → `pred_1d`, `pred_5d`, `pred_21d`)
- Deploy gate: IC > 0.03 + positive equity + parity marker
- Existing 9 GET API endpoints (additive only — new routes are separate)

## Key Design Decisions (resolved by the design doc)

| OD   | Topic                    | Decision                                              |
|------|--------------------------|-------------------------------------------------------|
| OD1  | Frontend framework       | Vite + Svelte, compiled via `rust-embed`             |
| OD2  | Backtest engine          | Rust-compiled replay (parity with live executor)     |
| OD3  | Advisor model            | DeepSeek V4 Flash primary, Claude 3.5 Sonnet fallback|
| OD4  | Real-time transport      | WebSocket (tokio-tungstenite broadcast channel)      |
| OD5  | Strategy config storage  | SQLite (`strategy_configs` table)                    |
| OD6  | Live toggle safety       | TOTP + parity marker validation (7-day max age)      |
| OD7  | Shorting mechanism       | Inverse ETF (PSQ for QQQ) — no margin/borrow         |
| OD8  | Strategy plugin system   | Rhai embedded scripting (pure-Rust sandbox)          |
| OD9  | Real-time data provider  | Moomoo OpenD (local protobuf TCP, priority 1)        |

## Files in This Plan

- `MASTER.md` — this file (overview + dependencies + decisions)
- `PHASE_0_FOUNDATION.md` — API split, DB schema, deps, Svelte scaffold
- `PHASE_1_DASHBOARD_WEBSOCKET.md` — Svelte dashboard, WS telemetry, old frontend retirement
- `PHASE_2_STRATEGY_LAB_RHAI.md` — Backtest engine, Rhai sandbox, Strategy Lab UI
- `PHASE_3_EXECUTION_SHORTING.md` — PSQ shorting, paper/live toggle, TOTP
- `PHASE_4_AI_ADVISOR.md` — LLM advisor module, prompt pipeline, SSE chat

## Per-Phase Deliverable Checklist

Each phase plan ends with:
- Exact files to create/modify (paths)
- Rust types / DB DDL / API contracts
- Test requirements
- Rollout / migration steps
- Risk notes
