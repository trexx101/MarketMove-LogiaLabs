# Options Execution Path — Implementation Plan

**Branch:** `feature/options-momentum-engine` (HEAD `6807c94`)
**Problem:** No options executor exists. A PAPER-stage candidate can be promoted, but nothing opens positions → no n_trades / mean_IC evidence accumulates → MICRO→LIVE is blocked.
**Scope:** Close the 8-point gap so a PAPER-stage candidate can open paper option positions and accumulate promotion evidence.
**Out of scope:** Live Moomoo options executor (Phase D, post-paper-validation), new UI, new DB tables beyond one schema migration.

---

## Current State (verified in source)

| # | Gap | Location |
|---|-----|----------|
| 1 | `OptionsScheduler` declared but **not spawned** | `main.rs:412` — comment "not-yet-enabled" |
| 2 | All 4 data sources are hardcoded mocks | `options_scheduler.rs:410-453` — `fetch_vix()→20.0`, `fetch_candidate_chains()→[]`, `fetch_account_equity()→100k`, `stop_distance→0.0`, `current_portfolio_premium→0.0` |
| 3 | `EntryExecutor` is pure state machine (computes limit prices) | `options/entry_executor.rs` — no broker call |
| 4 | No options-specific broker executor | `exec/` has `moomoo.rs` + `paper.rs` (equity only); no `exec/options_*.rs` |
| 5 | `OptionsPaperExecutor` only does **exits** (staged ladder) | `options/paper_executor.rs:264` — `initiate_exit` + `try_fill` only |
| 6 | No `option_positions` row creation anywhere | Table exists (`db.rs:210-228`) with `strategy_version_id NOT NULL`, but nothing INSERTs |
| 7 | No exit signal → arbiter → executor loop | `ExitArbiter` exists (`exit_arbiter/mod.rs`) but no caller feeds it signals |
| 8 | No trade attribution on `option_fills` | `option_fills` schema has `position_id` but **no** `strategy_version_id` |

---

## Phases (dependency-ordered)

### Phase A — Wire the Scheduler (2-3 days)

**Goal:** `OptionsScheduler` runs on a tokio task, fetches real data, but **only logs/skips** — no order placement yet. This is the scaffold.

**A1: Replace mock data sources** (M)
- `options_scheduler.rs`
- `fetch_vix()`: query `vix_close` from the existing `candles` table or a new `vix` table populated by the data pipeline. If no VIX pipeline exists yet, source from a daily API (Finnhub or yfinance). The key is **real** VIX, not a constant.
- `fetch_candidate_chains(equity)`: query the existing `option_tape_meta` table (already populated by the background recorder per the hybrid design) for the underlying, filter by DTE window (30-45) and delta window (0.40-0.50) using the `delta_at_entry` column. This returns `Vec<CandidateChain>` with `symbol`, `expiry`, `strike`, `option_type`, `delta`, `dte`, `ask`, `bid`, `volume`.
- `fetch_account_equity()`: call the Moomoo `get_accounts.py` / `get_all_portfolios.py` script (already in the moomooapi skill) or query the existing `trading_models.budget_usd` as a fallback. The paper-mode path can use a fixed configured value from `options_config_kv`.
- `stop_distance`: derived from the model's 1d prediction magnitude + volatility (e.g. `abs(pred_1d) * vol_scale`). Sourced from the same prediction that drives entry.
- `current_portfolio_premium`: sum of `entry_premium * qty` from open `option_positions`.

**A2: Spawn `OptionsScheduler` in `main.rs`** (S)
- `main.rs`: add `let options_scheduler = OptionsScheduler::new(...);` + `tokio::spawn(options_scheduler.run());` in the same section as the `EquityScheduler` spawn (around line 380).
- Gate on config: only spawn if `options_config_kv` has `options_enabled=true`. Default off until Inah flips it.
- The scheduler should log `OPTIONS_SCHEDULER_ENABLED` at startup.
- **Human intervention point:** Inah must set `options_enabled=true` in the DB before the scheduler will do anything.

**A3: Gate on strategy status** (S)
- In `run_entry_pipeline`, before the macro gate, query `strategy_versions` for the equity's **current** status. Only proceed if `status IN ('PAPER', 'MICRO', 'LIVE')`. If `CANDIDATE`, skip with `EntrySkipReason::NotPromoted`.
- This prevents pre-promotion candidates from opening positions.

**Test A:** Unit test that `OptionsScheduler::new` + `run()` produces a `SKIP` result when no PAPER/LIVE strategy exists for QQQ. Integration: engine boots, scheduler task starts, logs `OPTIONS_SCHEDULER_ENABLED`, first tick skips (no chains).

---

### Phase B — Paper Entry + Position Creation (3-5 days)

**Goal:** A paper option entry creates a real `option_positions` row + `option_fills` rows. No broker call.

**B1: Add entry capability to `OptionsPaperExecutor`** (M)
- `options/paper_executor.rs`: add `initiate_entry(...)` method.
  - Params: `underlying`, `contract_code`, `strategy_version_id`, `ask`, `contracts`, `entry_underlying_price`, `delta`, `dte`, `slippage_budget`.
  - Creates a new `option_positions` row:
    - `id`: UUID (per project rule)
    - `underlying`, `contract_code`, `strategy_version_id` (NOT NULL)
    - `entry_underlying_price`, `entry_premium` (= ask × 100), `entry_spread` (= ask − bid), `entry_slippage_budget`
    - `qty` = contracts, `qty_filled_residual` = 0
    - `status` = `'OPEN'`
    - `dte_at_entry`, `delta_at_entry`
    - `created_at`, `updated_at` = now
  - Records an `option_fills` row: `position_id`, `stage='ENTRY'`, `price=ask`, `quantity=contracts`, `timestamp=now`.
  - Logs `info!` with position_id.
- **Human intervention point:** `strategy_version_id` must be valid — it comes from the strategy that passed the macro gate + chain selector. If the promotion pipeline is the source of `strategy_version_id`, the scheduler must look it up (see A3).

**B2: Wire `EntryExecutor` state machine → paper fill** (M)
- `options_scheduler.rs`: replace the `executor.mark_filled()` mock (line 359) with a call to `OptionsPaperExecutor::initiate_entry(...)`.
- The `EntryExecutor` (2-stage ladder) computes the limit price; the paper executor "fills" at that price (paper mode = market-able, so it fills immediately at ask).
- If the 2-stage ladder is not filled in the time window, mark `status='CANCELED'` on the `option_positions` row and log the skip.
- **In paper mode, the 2-stage ladder is effectively a no-op** (immediate fill at ask). The ladder matters in live mode where limit orders can be rejected. Design for both: the `EntryExecutor` state machine stays, but the paper path calls `initiate_entry` directly on Stage 1 without waiting.

**B3: Publish `POSITION_OPENED` event** (S)
- After the `option_positions` row is created, call `db::insert_event(..., "trade", "info", ..., "options::position_opened", ...)` with the position ID, contract code, delta, DTE.
- This makes the position visible in the Events tab immediately.

**Test B:** 
- Unit: `OptionsPaperExecutor::initiate_entry` creates a row in the test SQLite DB with all fields populated.
- Unit: `EntryExecutor` 2-stage ladder advances correctly (existing tests should still pass).
- Integration: scheduler tick with a PAPER strategy + mock chain → `option_positions` row created, `option_fills` row created, event published.

---

### Phase C — Exit Path (3-5 days)

**Goal:** Open positions can be closed. Exit signals flow through the arbiter. Fills are recorded.

**C1: Wire exit signal sources → `ExitArbiter`** (M)
- The exit sources (from `ExitSource` enum): `OperatorForceClose`, `CircuitBreaker`, `DteOverride`, `TrailingStop`, `RoiTable`, `SignalReversal`.
- In the scheduler's exit loop (new `run_exit_check()` called on each tick for each open position):
  - `TrailingStop`: compare current underlying price against `entry_underlying_price` + trailing distance (configurable via `options_config_kv`). If breached, emit `ExitSignal{source: TrailingStop, priority: 4}`.
  - `DteOverride`: if `dte_at_entry - current_dte >= max_hold_days` (configurable), emit `ExitSignal{source: DteOverride, priority: 3}`.
  - `CircuitBreaker`: query the existing `CircuitBreaker` state (options/circuit_breaker/). If tripped, emit `ExitSignal{source: CircuitBreaker, priority: 2}`.
  - `SignalReversal`: if the model's 1d prediction for the underlying has flipped sign since entry, emit `ExitSignal{source: SignalReversal, priority: 6}`.
  - `OperatorForceClose`: from the API endpoint (future; for now, just the enum exists).
  - `RoiTable`: not yet designed — skip for this phase.
- Feed all signals into `ExitArbiter::select_winner()`. The winner (if any) proceeds to exit execution.

**C2: Execute exit via `OptionsPaperExecutor`** (M)
- `options/paper_executor.rs`: the existing `initiate_exit` + `try_fill` methods already implement the staged exit ladder. Wire the arbiter's winner to this.
  - `initiate_exit(position_id, current_bid, tick_size)` → starts the staged ladder.
  - `try_fill(position_id, observed_bid, observed_ask, timestamp)` → simulates a fill at the current ladder price.
  - On final fill: update `option_positions.status='CLOSED'`, set `realized_pnl` (exit price − entry price) × qty × 100, set `closed_at`, record an `option_fills` row with `stage='EXIT'`.
  - Publish `POSITION_CLOSED` event with realized PnL.

**C3: Record exit intent before sending** (S)
- Before the paper fill, record an `exit_intent_log` row (position_id, stage, order_id (mock), limit_price, quantity, timestamp). This is the audit trail.

**Test C:**
- Unit: `ExitArbiter` priority table (existing tests pass).
- Unit: Trailing stop trigger → `ExitSignal` emitted with correct priority.
- Unit: DTE override trigger.
- Integration: open a position, let the tick loop run, trigger trailing stop → position closes, `realized_pnl` computed, `option_fills` has ENTRY + EXIT rows.

---

### Phase D — Moomoo Live Executor (post-paper-validation)

**Not in scope for this plan** — this is the "next phase" after the paper path has accumulated evidence and a candidate promotes to MICRO.

**D1: `OptionsMoomooExecutor`** (L)
- `exec/options_moomoo.rs`: wraps `place_order.py` for US options.
- `place_order.py` already supports `--trd-env REAL` (or `SIMULATE`), `--type OPTION`, `--code <contract_code>`, `--quantity <contracts>`, `--price <limit>`, `--side Buy|Sell`, `--confirmed` (REQUIRED for real).
- The executor:
  - Maps the `contract_code` (e.g. `QQQ  260918C 62500`) to the OpenD API format.
  - Uses `--trd-env SIMULATE` for the MICRO stage (paper-verified but with real broker simulation).
  - Uses `--trd-env REAL` + `--confirmed` for the LIVE stage (requires OpenD GUI trade unlock).
  - `--confirmed` is a **hard gate** — it must be set explicitly, not auto-filled.
- **Human intervention points:**
  - Inah must enable **US options trading permission** on the Moomoo/Futu account (currently disabled — blocks all options order placement).
  - Inah must purchase/enable **OPRA real-time quotes** (the $7.49/m card is bought but not activated in the account).
  - **OpenD GUI trade unlock** is manual — cannot be automated. Must be done before any REAL order.
  - The `place_order.py` `--confirmed` flag is a per-call safety gate.

**D2: Paper↔Live toggle for options** (M)
- Extend `POST /api/mode` to support `options_mode: paper|micro|live`.
- `MICRO` → `--trd-env SIMULATE` (broker-side paper, real quotes).
- `LIVE` → `--trd-env REAL --confirmed` (real money).
- TOTP flow from the equity path (Phase 3) applies to the LIVE flip.

**Out of scope here:** the live executor is its own plan, its own review cycle. This plan ends at the paper path.

---

### Phase E — Trade Attribution + Promotion Evidence (2-3 days)

**Goal:** `option_fills` and `option_positions` carry enough attribution for the promotion gate to count them.

**E1: Schema migration — add `strategy_version_id` to `option_fills`** (S)
- `drizzle generate` + `drizzle migrate` (per project rule, NOT `drizzle push`).
- `ALTER TABLE option_fills ADD COLUMN strategy_version_id TEXT NOT NULL DEFAULT ''`.
- Backfill: for existing rows (there shouldn't be any yet — the feature is pre-deploy), set to `''`.
- New entries always populate from the `option_positions.strategy_version_id`.

**E2: Wire promotion evidence from option_fills** (M)
- The promotion gate (`strategy_versions` table, `min_days` + `n_trades` + `mean_ic`) currently reads from the equity pipeline.
- Add a query in the hyperopt / promotion applier that counts `option_fills` rows per `strategy_version_id` where `stage IN ('ENTRY', 'EXIT')` and the position's `status='CLOSED'`.
- `n_trades` = count of closed positions.
- `mean_ic` = mean of `realized_pnl / entry_premium` across closed positions (this is the proxy for "did the model's prediction match the actual outcome").
- `min_days` = `MAX(closed_at) - MIN(created_at)` in days across the closed positions for that `strategy_version_id`.
- This gives the promotion gate real evidence instead of the current "0.0 for everything."

**Test E:**
- Integration: after 5 paper trades (open + close) for a PAPER strategy, the promotion applier sees `n_trades=5`, computes a `mean_ic`, and `min_days` from the `option_fills` timestamps.
- Schema: `option_fills.strategy_version_id` column exists and is populated on new rows.

---

## Definition of Done (full feature)

- [ ] `OptionsScheduler` spawned in `main.rs`, gated by `options_config_kv.options_enabled`
- [ ] All 4 mock data sources replaced with real fetchers (VIX, chains, equity, stop_distance)
- [ ] Entry: `option_positions` row created on paper entry, `option_fills` row recorded
- [ ] Exit: arbiter picks winner, `OptionsPaperExecutor` executes staged exit, position closed with `realized_pnl`
- [ ] `option_fills` carries `strategy_version_id` (schema migrated)
- [ ] Promotion gate reads `n_trades`, `mean_ic`, `min_days` from option fills
- [ ] `cargo test --lib` passes (baseline: 23 pre-existing config failures)
- [ ] Integration: engine boots, scheduler runs, a paper option round-trip (open → hold → close) completes with correct attribution
- [ ] Events tab shows `options::entry`, `options::position_opened`, `options::exit`, `options::position_closed`
- [ ] **Human intervention points documented**: US options permission, OPRA quotes, OpenD unlock

---

## Human Intervention Points (flagged, per Inah's instruction)

| # | What | When | Who |
|---|------|------|-----|
| H1 | Enable US options trading permission on Moomoo/Futu account | Before Phase D (live) — **not needed for paper** | Inah (Moomoo app → settings → trading permissions) |
| H2 | Activate OPRA real-time quote subscription ($7.49/m card already purchased) | Before Phase D — **not needed for paper** | Inah (Moomoo app → data subscriptions) |
| H3 | Set `options_enabled=true` in `options_config_kv` DB table | After Phase A+B deploy, before first paper trade | Inah (SQL or API) |
| H4 | OpenD GUI trade unlock (manual) | Before any `--trd-env REAL` order (Phase D) | Inah (OpenD desktop app) |
| H5 | Flip `options_mode` to `MICRO` or `LIVE` via `POST /api/mode` | After paper evidence accumulates | Inah (UI or curl) |

**None of H1-H5 are needed for the PAPER phase (A+B+C+E).** Paper execution uses simulated fills against the existing tape data — no broker call, no OPRA subscription, no OpenD unlock.

---

## Risks & Failure Surfaces

1. **Chain selector returns stale data.** `option_tape_meta` is populated by a background recorder. If the recorder is down or the tape is stale (>1 day old), the chain selector will pick expired or unavailable contracts. **Mitigation:** add a freshness check in `fetch_candidate_chains` — skip if tape is >4h old, log a warning.

2. **`strategy_version_id` is the only link between a position and the promotion gate.** If the promotion pipeline is re-run and a `strategy_version` row is deleted/recreated, the `strategy_version_id` on existing `option_positions` rows becomes dangling. **Mitigation:** `strategy_versions.version_id` is a UUID that is never reused. If a version is superseded, old positions keep their reference (the old version is not deleted, it's marked `CANDIDATE→PAPER→MICRO→LIVE→SUPERSEDED` or similar).

3. **2-stage entry ladder in paper mode is a no-op but the state machine still runs.** If the scheduler crashes mid-tick, the `EntryExecutor` state is in memory and lost. **Mitigation:** for paper mode, skip the 2-stage ladder entirely (immediate fill). For live mode (Phase D), the ladder state must be persisted to `exit_intent_log` or a new `entry_intent_log` table before the order is submitted.

4. **Concurrent entry + exit on the same position.** The scheduler's entry tick and exit tick are on the same tokio task, so they can't truly race. But if a new entry is initiated while an exit is in the staged ladder, the position could be double-closed. **Mitigation:** `option_positions.status` is checked atomically before both entry and exit. Entry requires `status` to be absent (no open position for that underlying+strategy). Exit requires `status='OPEN'`.

5. **Disk / DB bloat.** Every paper round-trip creates 2 `option_fills` rows + 1 `option_positions` row + 2 events. Over months, this is thousands of rows. **Mitigation:** the existing `engine_events` table already has a retention window. `option_fills` and `option_positions` can have a similar GC (delete closed positions >90 days old, keep the aggregate).

---

## Sequence Summary

```
Phase A (2-3d):  Scheduler wired, real data, status gate
  └─ A1: data sources
  └─ A2: spawn in main.rs
  └─ A3: PAPER+ status gate
       ↓
Phase B (3-5d):  Paper entry, position creation
  └─ B1: OptionsPaperExecutor.initiate_entry
  └─ B2: EntryExecutor → paper fill
  └─ B3: POSITION_OPENED event
       ↓
Phase C (3-5d):  Exit path, arbiter, close
  └─ C1: exit signal sources → ExitArbiter
  └─ C2: staged exit → close position
  └─ C3: exit intent log
       ↓
Phase E (2-3d):  Attribution + promotion evidence
  └─ E1: schema migration (option_fills.strategy_version_id)
  └─ E2: promotion gate reads option fills
       ↓
Phase D (later):  Moomoo live executor (own plan, own review)
```

**Total for paper path (A+B+C+E):** ~10-16 engineering days, one engineer.
