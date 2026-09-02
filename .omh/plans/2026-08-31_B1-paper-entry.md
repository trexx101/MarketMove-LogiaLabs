# B1 — Paper Entry + Position Creation (`OptionsPaperExecutor::initiate_entry`)

**Parent plan:** `.omh/plans/ralplan-options-execution-path.md` (Phase B, sub-task B1)
**Branch:** `feature/options-momentum-engine`
**Scope:** ONE method + its tests. No schema migration, no scheduler wiring, no UI.
**Status:** ✅ DONE (2026-08-31) — 420 passed / 0 failed (416 baseline + 4 new), release build clean, graph refreshed. Uncommitted — awaiting Inah sign-off. R2/R3/R4 resolved per plan (per-contract premium, `US.` underlying passed by B2 caller, residual = contracts). R1 done as part of B1.

---

## Context & Goal

After the macro gate → chain selector → sizing pipeline runs, nothing persists a
position. `paper_executor.rs` only has `initiate_exit` + `try_fill` (exits).
B1 adds the entry side:

> **One call to `OptionsPaperExecutor::initiate_entry(...)` atomically creates
> one `option_positions` row (status='OPEN') + one `option_fills` row
> (stage='ENTRY') + one `options::position_opened` event.**

Deliberately NOT in this task:
- Scheduler wiring (the `mark_filled()` mock at `options_scheduler.rs:359` stays — B2 replaces it)
- 2-stage entry-ladder persistence (plan risk #3: in paper mode the ladder is a no-op; skip it)
- `option_fills.strategy_version_id` column (Phase E1, separate migration)

---

## Findings from source (affect shape)

1. **`position_id` type mismatch (pre-existing, fix as part of B1).**
   `option_positions.id` is a UUID **TEXT** PK (db.rs:209). But
   `initiate_exit`/`try_fill`/`advance_ladder`/`get_ladder`/`cancel_exit` all
   take `position_id: i64`. The deployed exit pipeline papers over it with
   `i64::from_str_radix(pos_id.split('-').next(), 16)`
   (`options_scheduler.rs:~512`) — a lossy hack that makes
   `option_fills.position_id` never match the real UUID.
   **Fix: change all 5 methods to take `position_id: &str`** (it's a
   feature-branch-only call site; the schema migration
   `migrate_option_positions` already exists to rebuild INTEGER id columns).
   Exit-stage tests updated to string ids.

2. **`strategy_version_id` is available but unused in the entry path.**
   `fetch_strategy_status(equity)` (A3, `options_scheduler.rs:787`) already
   returns `(id, status)` — currently only used for the status gate. B2 will
   forward its `id` into `initiate_entry`; B1 only needs the parameter.

3. **Entry inputs all exist.** `CandidateChain` (symbol, expiry, strike,
   option_type, delta, bid, ask, open_interest, dte) +
   `SizingDecision.contracts` + `fetch_latest_prediction` (for
   `entry_underlying_price`) — no new data sources needed.

4. **Test-harness DDL is stale.** The in-file test `option_fills` DDL uses
   `position_id INTEGER`; must become TEXT to mirror production (db.rs:320-327).

---

## Changes

### 1. `engine/src/options/paper_executor.rs`

New method (primary deliverable):

```rust
pub async fn initiate_entry(&self, EntryEntryParams) -> Result<EntryOutcome>
```

```rust
pub struct EntryEntryParams {
    pub underlying: String,          // "US.QQQ" (recorder form) — see finding below
    pub contract_code: String,       // "QQQ  260930C 62500" (tape chain_code)
    pub strategy_version_id: String, // from fetch_strategy_status (A3)
    pub entry_underlying_price: f64, // last close of the equity
    pub ask: f64,
    pub bid: f64,
    pub contracts: u32,              // from PositionSizer
    pub delta: f64,
    pub dte: i64,
    pub slippage_budget: f64,
}

pub enum EntryOutcome {
    Opened { position_id: String, fill_price: f64 },
    Skipped(EntrySkipReason),
}

pub enum EntrySkipReason {
    DuplicateOpenPosition { existing_position_id: String },
}
```

Behavior:
1. **Duplicate guard** (plan risk #4): `SELECT id FROM option_positions
   WHERE underlying = ? AND status = 'OPEN'` → if any, return
   `Skipped(DuplicateOpenPosition)`. One open position per underlying.
2. **Insert `option_positions`** (all DDL columns, db.rs:209-228):
   | column | value |
   |---|---|
   | id | `Uuid::new_v4().to_string()` (project rule) |
   | underlying | as passed |
   | contract_code | as passed |
   | strategy_version_id | as passed (NOT NULL — no default) |
   | entry_underlying_price | as passed |
   | entry_premium | `ask` (per-contract) — **flagged for review, see R2** |
   | entry_spread | `ask - bid` |
   | entry_slippage_budget | as passed |
   | qty | contracts |
   | qty_filled_residual | contracts (paper = filled immediately) |
   | status | `'OPEN'` |
   | dte_at_entry / delta_at_entry | as passed |
   | realized_pnl / closed_at | NULL |
   | created_at / updated_at | `Utc::now().timestamp()` |
3. **Insert `option_fills`**: `position_id` = the new UUID, `stage='ENTRY'`,
   `price=ask`, `quantity=contracts`, `timestamp=now`.
4. **Event**: `db::insert_event(..., "trade", "info", ..., "options::position_opened",
   "POSITION_OPENED {equity}: {contract_code} @ {ask} x{contracts}",
   payload_json{position_id, contract_code, delta, dte, ask, contracts,
   strategy_version_id}, equity)`.
5. `info!` with position_id on success.

### 2. Signature fix (finding 1) — same file

- `initiate_exit(position_id: &str, ...)`, `try_fill(position_id: &str, ...)`,
  `advance_ladder(position_id: &str, ...)`, `get_ladder(position_id: &str)`,
  `cancel_exit(position_id: &str)`.
- `FillResult.position_id: String`.

### 3. Call site: `engine/src/options_scheduler.rs` `run_exit_pipeline`

- Delete the `pos_id_i64` hex-parse hack; pass `&pos_id` (String) directly.
- No other changes.

### 4. Tests (same file, `mod tests`)

- Update `test_pool()` DDL: `option_fills.position_id TEXT` (mirrors db.rs).
- **New:** `initiate_entry_creates_position_and_fill` — call with a fixture
  set; assert the `option_positions` row exists with every field populated
  (id is a UUID, status='OPEN', qty=contracts, entry_premium=ask, ...), the
  `option_fills` row has stage='ENTRY' and matching position_id, and
  `engine_events` got one `options::position_opened` row.
- **New:** `initiate_entry_skips_when_open_position_exists` — seed an OPEN
  position for the same underlying, call again, assert
  `Skipped(DuplicateOpenPosition)` and zero new rows.
- **New:** `initiate_entry_rejects_missing_strategy_version` — `strategy_version_id=""`
  returns an error (defends the NOT NULL / attribution chain before E1).
- **Update:** the 4 existing exit-ladder tests to string ids (expect
  unchanged semantics).

### 5. Not touched

- `options_scheduler.rs` entry pipeline (B2), `db.rs` (no migration in B1),
  `entry_executor.rs` (stays a pure state machine), UI, `main.rs`.

---

## Verification

1. `cargo test --lib` — new tests green; **no regressions vs baseline**
   (baseline to be re-measured at implementation start; last known
   416 pass / 0 fail from `ae5ac13` + 23 pre-existing config failures).
2. `cargo build --release` clean (it deploys to the engine container).
3. Static check: `grep -n "pos_id_i64" src/` → zero hits.

## Human intervention points

- **None for B1.** `options_enabled` stays off (H3 unchanged), so the entry
  pipeline never fires in production even after deploy. B1 ships dormant.
- No migration, no new config keys, no UI.

## Risks / flags for your review

- **R1 — signature change to `&str`:** additive in effect but changes 5
  public method signatures. Alternative: leave `i64` and have B1 use string
  only for the new entry path (keeps the hack alive). I recommend the fix —
  the current hex-prefix parse means EXIT fills are already recorded with
  mismatched position_ids and E1's attribution would inherit that dirt.
- **R2 — `entry_premium` semantics:** plan B1 says `entry_premium = ask ×
  100` (total premium). But the exit pipeline computes `realized_pnl` as
  `(exit_price − entry_premium) × qty × 100`, which only balances if
  `entry_premium` is the **per-contract premium (ask)**. I recommend
  per-contract; the `×100×qty` conversion happens at the PnL calc. Needs
  your call — the existing (uncommitted) test at
  `options_scheduler.rs:1169` inserts per-contract values.
- **R3 — `underlying` format:** recorder/tape uses `US.QQQ`; the scheduler
  passes the stripped `QQQ` to the exit pipeline's WHERE clause (which
  currently matches nothing). B1 should store `underlying` consistently with
  the exit query — I propose the scheduler passes the same value it queries
  with (i.e. `US.QQQ`), one change at the call site in B2. Flagging now so
  B2 doesn't create a second mismatch.
- **R4 — `qty_filled_residual`:** plan says 0 on creation; I recommend
  `= contracts` (paper fills immediately at ask, so the residual that
  still needs filling is the full qty — same convention as the existing
  paper equity executor). Confirm.
