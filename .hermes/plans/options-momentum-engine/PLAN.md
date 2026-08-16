# Turnkey Options Momentum Engine — Multi-Phase Plan

**Branch:** `feature/options-momentum-engine`
**Status:** Approved design (grill session 2026-08-16) — not yet under construction
**Stack:** Rust execution engine · SvelteKit UI · Moomoo Futu OpenD (local, VPS)
**Trading universe (v1):** QQQ, SMH, XLF · **Mode:** paper first, tiered to live

---

## 0. Settled Design Decisions (from grill session)

These are locked. Any change requires a design discussion, not a judgment call during implementation.

| # | Area | Decision |
|---|------|----------|
| D1 | Backtest fuel | Phased hybrid: synthetic premiums (A) optimize; live recorder tape (B) validates; vendor data (C) deferred |
| D2 | Recorder | Separate process, own OpenD connection, QUOTE-only subs (candles synthesized from ticks), one tradeable chain per underlying, roll ~5 days pre-expiry, full-ladder snapshots via request/response every 15 min |
| D3 | Quota budget | ~60% recorder / 40% live trading; recorder sheds subscriptions first on pressure. Quota = config value, assumed tier 60 (VERIFY in OpenD before Phase 1) |
| D4 | OPRA quote-right | $7.49/mo OPRA card purchased — prerequisite for US options LV1 |
| D5 | Cadence | Hybrid: daily-candle entries; tick-driven exits on underlying price. Hourly entries = v2 parked item |
| D6 | Macro | Risk layer, never strategy layer. VIX level + 5d slope gate (Futu). FOMC/CPI/NFP = hardcoded YAML calendar, entry blackout 24h. FRED 10Y-2Y = v2. Strategy swaps can never disable it |
| D7 | Exit execution | Fresh `get_option_quote()` before exit. Ladder: `BID + k×tick` (3s) → `BID` (3s) → `BID − max_slippage` (10s). Never raw market orders |
| D8 | Slippage budget | Dynamic: multiplier of entry-time spread, capped at % of premium |
| D9 | Circuit breaker | Stage-3 failure → cancel, residual stays open, `STATUS = CIRCUIT_BREAKER` in DB, entry halt + cooldown (15 min default) or manual clear, phone/Slack alert. Accepted cost: unhedged gamma bleed on residual during cooldown |
| D10 | Reconciliation | B + write-ahead intent: DB owns strategy context, broker owns facts, stage transitions persisted pre-send. Startup gate: exits auto-resume, entries never, mismatches quarantine (`RECONCILING`) |
| D11 | Hyperopt | Nightly batch, never during market hours, throttled to idle cores. ≤6 tunable params, ≥100 trades minimum, walk-forward + embargo, neighborhood-stability check |
| D12 | Promotion gate | Evidence gate (not calendar): ≥30 paper trades AND ≥4 weeks tape AND synthetic-vs-paper divergence ≤ ±25%. Tiered sizing: paper → 1-contract micro → full slider size after 60 clean live trades |
| D13 | Hot-swap | Sunset model: positions bound to `strategy_version_id` forever; dethroned versions manage residuals to termination (~45 days). Promotion = operator button in UI, only at daily candle boundary, never mid-exit-ladder. Per-version P&L attribution in trade history |
| D14 | ExitArbiter | Single arbiter, fixed priority: force-close > circuit breaker > hardcoded overrides > trailing stop > minimal_roi > signal reversal. Triggers emit signals; only the arbiter places orders |
| D15 | Hardcoded overrides | In code, non-optimizable, all strategy versions: DTE < 7 → exit; delta drift outside [0.15, 0.70] → exit; earnings within 2 days → entry blackout (held positions unaffected) |
| D16 | Force-close | Skips stage ladder, straight to `BID − max_slippage` deep-limit |
| D17 | Chain selection | Monthly expiry preferred over weeklies; minimize \|delta − 0.45\| among liquidity-passing candidates; DTE-distance-to-37 tie-break. No candidate → `SKIPPED_ENTRY` event, no trade |
| D18 | Liquidity floors | Hard, CONFIGURABLE: bid > 0, spread ≤ 8% of mid, OI ≥ 100 (defaults calibrated to QQQ/SMH/XLF) |
| D19 | Rolling | None in v1. Exit is exit; re-entry only on fresh signal. Revisit v2 with real tape |
| D20 | Sizing | `contracts = floor(equity × risk% / (stop_distance × delta × 100))`, capped by: debit ≤ premium-% slider (binding constraint expected), contracts ≤ 10% of OI, 1 position/underlying (max 3 total), deployed premium ≤ 25% of account |
| D21 | UI sliders | Map to risk %, never contract count. Portfolio caps = fixed config, read-only display |

---

## 1. Architecture Overview

```
                        ┌────────────────────────────────────┐
                        │  OpenD (local)  ── OPRA LV1 card   │
                        └───────┬────────────────┬───────────┘
                    QUOTE pushes│                │request/response
              ┌─────────────────┴──┐      ┌──────┴──────────────┐
              │  TAPE RECORDER      │      │  TRADING ENGINE      │
              │  (separate process, │      │  (existing Rust core)│
              │  own connection)    │      │                      │
              │  → parquet tape     │      │  Daily candle        │
              └────────┬───────────┘      │  strategy layer      │
                       │                   │    ↓                 │
                       │ validation replay │  ExitArbiter         │
                       └───────────────►   │    ↓                 │
                                           │  Staged exit ladder  │
              ┌─────────────────────┐      │    ↓                 │
              │  HYPEROPT (nightly  │      │  Moomoo orders       │
              │  batch, idle cores) │      │                      │
              │  A: synthetic sim   │      │  Reconciliation gate │
              │  → candidates ──────┼────► │  (startup)           │
              └─────────────────────┘      └──────┬───────────────┘
                                                  │ WS + REST
                                          ┌───────┴───────┐
                                          │ SvelteKit UI   │
                                          │ Auto-Pilot     │
                                          └───────────────┘
```

Key invariants:
- Strategy layer decides entries only; it never places orders. All exits route through ExitArbiter → staged ladder.
- Risk layer (macro calendar, hardcoded overrides, portfolio caps) is outside the strategy version lifecycle — hot-swaps cannot touch it.
- Every order action is write-ahead logged before the send.
- Paper executor (existing) is the first live consumer; live trading is the same code path with a different execution backend.

---

## 2. Phases

Phase ordering is dependency-driven. Phases 1–2 can run in parallel once Phase 0 lands. Phases 3–4 are the core and largely sequential. UI (Phase 6) starts against stubs after Phase 3's API shapes freeze.

---

### Phase 0 — Prerequisites & Foundations

**Goal:** verify broker facts, define config schema, create DB schema for options domain.

**Tasks**
1. Verify account option quota tier in OpenD (`get_subscription_info` or app) — sets the D3 budget (20/60/200). Record actual number in config.
2. Verify OPRA card active: subscribe one option chain QUOTE, confirm greeks fields (`implied_volatility`, `delta`, `gamma`, `theta`) present in pushed quotes.
3. Config schema (`options_engine` section): quota budget split, liquidity floors (spread cap %, OI min), sizing sliders + caps, cooldown duration, DTE window [30,45], delta target 0.45, delta-drift band, slippage multiplier + premium cap, macro thresholds, paper/micro/full mode flag.
4. DB migrations (repo convention: generate + migrate, NEVER push):
   - `option_positions` (UUID PK): underlying, contract code, strategy_version_id, entry basis (underlying px, spread at entry, slippage budget snapshot), qty, qty_filled_residual, status enum (`OPEN`, `EXITING_STAGE1/2/3`, `CIRCUIT_BREAKER`, `RECONCILING`, `CLOSED`), dte_at_entry, delta_at_entry.
   - `strategy_versions`: id, family, params JSON snapshot, status (`CANDIDATE`, `PAPER`, `MICRO`, `LIVE`, `SUNSETTING`, `RETIRED`), promotion metadata.
   - `exit_signals` / `order_intents` (write-ahead): position_id, trigger source, priority, stage, intended action, persisted-before-send flag.
   - `option_tape_meta`: recorded chains, quota accounting.
5. OpenD client extension (Rust): option chain fetch (`get_option_chain`), option quote request/response, option order placement/modification/cancel — none of this exists in the current equity-only client.

**Acceptance criteria**
- Quota tier recorded; a test subscription of one chain returns greeks.
- Migrations applied clean; `cargo build` green; config parses with defaults matching the D-table.

**Risks:** account under 10K HKD → only 20 chains; redesign recorder budget (fewer underlyings recorded). Mitigation: D3 budget is config, not code.

---

### Phase 1 — Tape Recorder (Mode B)

**Goal:** accumulate the real-data tape that the promotion gate depends on. Starts early because it is calendar-bound — every day not recording is lost data.

**Tasks**
1. Separate binary/process, own OpenD connection. Quota accounting at startup; refuse to exceed its allocation; shed subscriptions under pressure with alert.
2. Chain selection for recording: per underlying, nearest chain in [30,45] DTE window, monthly preferred. Roll subscription forward ~5 days before current chain exits window. Unsubscribe rolled-out chains (releases quota).
3. Subscribe QUOTE only. Synthesize candles locally from ticks (1m base; aggregate to 5m/15m/1h/1d on write). Record: bid, ask, last, volume, OI, IV, delta, gamma, theta, underlying price, timestamp — every tick into ring buffer, flush to parquet (partitioned by underlying/chain/date).
4. Slow-cadence full-ladder snapshots: every 15 min, request/response `get_option_chain` + quotes for the DTE window; store as ladder snapshots (for later spread/OI analysis).
5. Macro recorder: VIX quote stream + daily close; write to same tape store. FOMC/CPI/NFP YAML calendar file checked in (this year + next, source: published Fed/BLS schedules).
6. Health: heartbeat metric, gap detection (missing ticks > N seconds → alert), daily tape summary report.

**Acceptance criteria**
- 5 consecutive trading days of gap-free recording on QQQ (then add SMH, XLF).
- Parquet readable by the backtester (schema contract test).
- Quota usage visible: recorder + engine never exceed account quota; kill-switch verified.

**Risks:** OpenD push rate limits / disconnects overnight. Mitigation: reconnect loop + gap flagging (gaps are data, silently missing gaps are the real bug).

---

### Phase 2 — Synthetic Options Backtester (Mode A)

**Goal:** the simulator the hyperopt runs on. Underlying OHLCV (unlimited Futu history) + synthetic option pricing.

**Tasks**
1. Option pricing model: BSM with per-underlying IV assumption (realized vol × 1.1 default, configurable per symbol). Produce: premium, delta, gamma, theta at any (underlying_px, IV, DTE, strike). Document model limitations explicitly — no IV term structure, no smile, no IV-crush event modeling.
2. Fill model: synthetic bid/ask = model price ± (spread assumption, configurable, default 4% of mid); fills execute at bid (sells) / ask (buys). Slippage model mirrors D8 budget mechanics so backtest and live use the *same* slippage code path.
3. Event loop: daily-candle entry decisions, tick-level (or 5-min resampled) underlying-driven exit evaluation, DTE/delta-drift/theta overrides, staged-ladder simulation with the synthetic book.
4. Metrics: per-trade (entry/exit basis, P&L, slippage paid, exit trigger source), aggregate (CAGR, max DD, win rate, profit factor, avg trades/yr), and the divergence-report fields the promotion gate will consume.
5. Strategy family interfaces (Rust trait): signal on daily candles (EMA/MACD breakout, Ichimoku as first members), parameter schema with ≤6 tunables, `informative_pairs` hook for VIX regime.
6. Walk-forward harness: 5y train / 1y validate windows, embargo gap ≥ longest horizon + buffer, reuse the pattern from the equities ML pipeline.

**Acceptance criteria**
- Replay a hand-computed scenario trade end-to-end and match P&L to the cent (unit test).
- Walk-forward run on QQQ with a fixed strategy completes; output includes divergence-ready report.
- Runtime: full 5y × parameter grid backtest < 30 min on VPS (vectorized).

**Risks:** model fiction vs reality gap. That is *expected* and is exactly what the tape validation gate exists to catch — do not over-invest in pricing fidelity now.

---

### Phase 3 — Options State Machine & Execution Core

**Goal:** the heart. ExitArbiter, staged ladder, circuit breaker, reconciliation. Build against paper executor first.

**Tasks**
1. `ExitArbiter`: single owner of exit decisions. All sources (stop, ROI, overrides, breaker, UI force-close, signal reversal) emit `ExitSignal{priority, source}`. Fixed priority table per D14. Arbiter selects winner per position, serializes decisions (one active exit protocol per position; no concurrent exits).
2. Hardcoded override module (D15): DTE check (calendar-driven, evaluated daily + on candle), delta-drift (on every option quote update), earnings blackout (calendar, entry-side). In code, not config-tunable, not version-scoped. Unit tests for each priority interaction.
3. Staged exit ladder executor (D7): fresh quote fetch → Stage 1 `BID + k×tick` (tick-relative, not dollar) → Stage 2 `BID` → Stage 3 `BID − max_slippage`. Timers 3s/3s/10s. Order modification via OpenD. Partial-fill tracking: residual qty owned by state machine; exit "complete" only when flat or breaker fires.
4. Circuit breaker (D9): `CIRCUIT_BREAKER` DB status, per-underlying entry halt + cooldown timer, operator alert via existing alerting path, manual-clear API.
5. Write-ahead intent log: persist stage transition BEFORE every order send/modification (one row per transition).
6. Startup reconciliation gate (D10): pull open orders + positions from Moomoo, diff vs DB. Mismatch → `RECONCILING` quarantine (position-level), exits auto-resume via fresh quote + fresh ladder, entries blocked until gate clean. Reuse the equity engine's `sync_from_db` lessons — this is the same bug class, worse blast radius.
7. Hysteresis on trailing stop: re-arm requires recovery band (0.5 × ATR above stop) to prevent whipsaw churn through spreads.
8. Paper mode: same ladder, fills against observed bid/ask (existing paper executor semantics extended for options).

**Acceptance criteria**
- Kill -9 the engine mid-Stage-2 during paper trading; restart → reconciliation detects resting order, resumes exit, no duplicate orders, no state loss.
- Priority table proven by unit tests: simultaneous stop + DTE override + force-close → exactly one winner, force-close wins.
- Circuit breaker fires in a simulated thin-book scenario; entry halt + alert verified.

**Risks:** OpenD order-modify latency on fast moves. Mitigation: stage timers are config; degrade path (deep-limit) is already designed in.

---

### Phase 4 — Strategy Layer, Chain Selection & Sizing

**Goal:** entries. Daily-candle signal evaluation → macro gate → chain selection → sizing → order.

**Tasks**
1. Daily candle pipeline for underlyings: reuse existing candle infrastructure; `process_only_new_candles` semantics — compute indicators once per confirmed close, act on next decision tick.
2. Macro risk gate: VIX level + 5d slope (configurable thresholds) blocks new entries; calendar blackout blocks within 24h of FOMC/CPI/NFP. Gate output is a per-underlying `ENTRY_ALLOWED/DENIED(reason)` — auditable in UI.
3. Chain selector (D17/D18): fetch DTE-window chains, filter [30,45] DTE, prefer monthly, apply configurable liquidity floors (bid>0, spread ≤ 8% mid, OI ≥ 100), pick min |delta − 0.45|. No candidate → `SKIPPED_ENTRY` event with reason code; never relax filters.
4. Position sizing module (D20): formula + three caps. Emits `SKIPPED_ENTRY` with reason when any cap binds.
5. Entry execution: limit at ask (buys), staged analog of the exit ladder (2 stages max — entries are not emergencies; unfilled entry simply cancels).
6. Wire everything to ExitArbiter ownership at fill: position row created with full entry-basis snapshot (underlying px, spread, slippage budget).

**Acceptance criteria**
- Paper trade placed on QQQ through the full pipeline: signal → gate → chain → size → fill → arbiter takes ownership.
- `SKIPPED_ENTRY` fires correctly on: blackout day, no-liquidity scenario (simulated), caps binding.
- No entry possible while reconciliation gate is dirty or breaker is active (integration test).

---

### Phase 5 — Hyperopt Loop & Promotion Pipeline

**Goal:** nightly batch optimizer + evidence-gated promotion, nothing trading real size.

**Tasks**
1. Nightly scheduler: runs post-market, CPU-throttled (idle cores), hard-stop before next open. Never runs during market hours.
2. Optimizer: per strategy family, ≤6 params, grid/random search over backtester (Phase 2), ≥100-trade minimum filter, walk-forward + embargo.
3. Neighborhood-stability check: champion's parameter neighbors must perform within tolerance; lone sharp peaks rejected as overfit artifacts. Report includes the stability evidence.
4. Candidate store: immutable param snapshots + backtest reports, versioned IDs (feeds D13 sunset model).
5. Promotion pipeline state machine: `CANDIDATE → PAPER → MICRO → LIVE`, with the evidence gate (D12) enforced by code, not convention: ≥30 paper trades + ≥4 weeks tape + divergence ≤ ±25% before MICRO; 60 clean live trades before full size.
6. Tape validation replay: run candidate against recorded tape (Phase 1 data) as the final pre-MICRO check.

**Acceptance criteria**
- One full nightly run end-to-end: optimize → rank → store → report in UI.
- Promotion gate provably blocks: attempt to promote a candidate with 12 paper trades → rejected with reason.

---

### Phase 6 — UI: Strategy Auto-Pilot (SvelteKit)

**Goal:** operator control surface. Build against Phase 3–5 API shapes.

**Tasks**
1. Auto-Pilot panel: per-underlying status (armed / breaker / cooldown / quarantined), active strategy version + status, deployed premium vs cap (read-only).
2. Risk sliders (D21): risk-per-trade % (0.25–3), max premium per position % (1–10). Portfolio caps displayed read-only. Sliders write config; changes take effect at next candle boundary.
3. Strategy management: candidate list with backtest metrics + stability evidence, **Promote button** (operator approval, D13), per-version P&L attribution in trade history (existing global trade history + `strategy_version_id` badge).
4. Positions view: open options positions, entry basis, current delta vs entry, DTE countdown, exit-stage indicator (which stage is active, residual qty), circuit-breaker flags.
5. Emergency controls: force-close per position (D16, deep-limit), force-close-all, manual breaker clear. Confirmation dialogs.
6. Events feed: `SKIPPED_ENTRY` reasons, gate denials (macro blackout), promotion events, reconciliation events.

**Acceptance criteria**
- Every engine state (breaker, quarantine, stage, cooldown) is visible in UI without reading logs.
- Force-close button verified end-to-end in paper mode.

**Convention:** follow `DESIGN.md` design system; pass DESIGN.md to any sub-agent building components (per repo AGENTS.md).

---

### Phase 7 — Paper Trading Campaign & Evidence Collection

**Goal:** run the whole system in paper mode, accumulate the evidence the promotion gate demands.

**Tasks**
1. Deploy full stack paper mode on VPS; run through ≥4 weeks of live markets.
2. Monitor: reconciliation gate clean on every restart, recorder gap-free, breaker false-positive rate, divergence between synthetic predictions and paper realized P&L (the D12 metric, tracked continuously).
3. Weekly review ritual: divergence report, SKIPPED_ENTRY distribution, exit trigger mix, slippage-vs-budget stats.
4. Tune only config-layer values (liquidity floors, slippage multiplier, cooldown) — strategy params are hyperopt's job, never hand-tuned live.

**Exit criteria → Phase 8:** evidence gate satisfied for ≥1 strategy version; no unresolved reconciliation incidents; zero unexplained state transitions in the audit log.

---

### Phase 8 — Tiered Live Go-Live

**Goal:** real money, in the smallest increments the broker allows.

1. MICRO tier: 1-contract positions, full ladder, full alerting. Duration: until 60 clean trades.
2. Review: realized slippage vs budget, fill-rate by stage, breaker incidence. Any Stage-3 incidence → root-cause before size increase.
3. FULL tier: slider sizing enabled. Ongoing weekly evidence-gate re-check (a promoted strategy that diverges gets dethroned → sunset model takes over its residuals).
4. Post-go-live v2 parking lot: hourly entries, position rolling, FRED yield-spread filter, vendor historical data (C), delta-hedged exits.

---

## 3. Cross-Phase Conventions

- **Schema changes:** generate + migrate, never push (repo rule). UUIDs for all non-auth PKs.
- **State transitions:** every position/order state change persisted before the external action. Audit log is the source of truth for post-mortems.
- **Testing:** unit for arbiter priority + sizing + pricing; integration for reconciliation restart; scenario tests (kill -9 mid-stage, thin-book, whipsaw, gap-down) before each go-live tier.
- **Config vs code:** risk rails and priority table = code. All thresholds/floors/budgets = config.
- **Alerting:** breaker, reconciliation mismatch, recorder gaps, quota pressure, stage-3 incidence — all route to the existing operator alert path.

## 4. Open Items

1. **Verify account quota tier** (blocks Phase 0 sign-off).
2. Hourly-entry cadence — v2 decision.
3. Alert channel choice for circuit breaker (phone push vs Slack vs both).
4. Earnings calendar source for per-ticker dates (vs the macro event calendar which is hand-maintained).

## 5. Explicit Non-Goals (v1)

- No position rolling. No multi-leg/spread strategies. No short premium (selling options). No market orders. No auto-promotion without operator click. No vendor data. No sub-daily entries.
