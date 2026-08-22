# Git Branch Divergence & Merge Reconciliation (MarketMoves)

## The hazard
This project runs **parallel long-lived branches** that diverge at a shared base.
Symptoms that look like "a bug I introduced" are often an **un-merged feature branch**:
the deployed HEAD was built on an *older* base than the branch holding the real feature work.

Concrete case (2026-08-19): Phase 7 UI was committed on `e33adc6` (base `190922f`).
`feature/nvda-multi-asset-and-sentiment-overlay` was 16 commits ahead of that base and held
the multi-equity Dashboard dropdown, PnL fix, and per-model trade history. Deploying `e33adc6`
baked the old Dashboard → user saw a "regressed" UI that was actually just stale.

## First-move diagnostics (before touching code)
1. `git log --oneline -15 <changed-file>` — is the feature in git history at all?
2. `git branch -a --contains <suspect-commit>` — which branches have it.
3. `git log --oneline HEAD..<feature-branch>` — what's ahead of deployed HEAD.
4. `git show <feature-branch>:<file>` | `grep` for the missing feature (e.g. `select|underlying|SMH`).
5. Confirm working tree is clean (`git status --short`) before merging.

If the feature exists on another branch and NOT on deployed HEAD → it's a merge problem, not a bug.

## Merge reconciliation playbook (10+ conflicted files, frontend + Rust)
Order: resolve frontend (user-facing priority) first, then backend.

### Files you typically keep one side of
- `git checkout --ours <file>` when your version is canonically better (e.g. Events view
  with severity/equity filters vs. the feature branch's simpler one). Then verify 0 conflict
  markers remain in that file.

### Files you must hand-merge (concatenate both)
- `App.svelte`: merge nav items + view-switch blocks from both. Watch for:
  - duplicate nav buttons for the same view (dedupe)
  - duplicate `{:else if currentView === 'x'}` blocks in the render switch (dedupe)
- `api.js`: keep both sets of exported functions. Remove duplicate function defs
  (e.g. two `fetchEvents` — keep the one your Events view actually calls).
- `Cargo.toml` / root `Cargo.toml`: both dependency sets — add, don't replace.
- `Cargo.lock`: conflict markers appear as BOTH package blocks missing. Re-insert both
  `[[package]]` entries (don't drop either). TOML parse error at line N = unresolved lock conflict.
- `.gitignore`: union both ignore patterns.

### Files needing manual port (when you `checkout --ours` and lose the other side's functions)
If you took HEAD's `db.rs` but the feature branch added tables/functions HEAD lacks:
1. `git show <feature-branch>:engine/src/db.rs | grep -n "pub async fn <name>"` to locate.
2. Extract the function body block and paste into HEAD's db.rs **before** `#[cfg(test)]`.
3. Also port any **DDL** the functions need (e.g. `advisor_chat_log` table) into the schema block.
4. Add `Deserialize` to `#[derive(...)]` if the ported struct is deserialized.
5. If `main.rs`/`api/mod.rs` (auto-merged) references a function you didn't port, that function
   must exist or the bin won't compile — grep `db::` refs in main.rs to enumerate requirements.

### Rust compile-error sweep after merge
- `E0425 cannot find value handle_X` → handler renamed across branches. Align the router
  (`api/mod.rs`) route to the actual handler name in the kept file.
- `E0428 defined multiple times` → duplicate function from a port. Delete the copy
  (use a Python brace-matching script to find the matching `}` if needed).
- `E0063 missing fields in AppState` → new AppState fields added on one branch. Update every
  test helper that constructs `AppState { ... }` (search `State(AppState {`).
- `E0382 borrow of moved value: pool` → `pool` moved into the struct before a later
  `.clone()` for `event_logger`. Clone `pool` into a local BEFORE the struct literal, or
  construct `event_logger` first using `pool.clone()`.

## Verification after merge
- `cargo build --release --bin engine` → must be clean (warnings OK).
- `npx vite build` → must succeed (unused-CSS warnings are harmless).
- `cargo test --release --lib` → confirm failures are PRE-EXISTING, not new.
  Known pre-existing set on this repo: `config::tests` x3 + `api::tests` x2 caused by
  repo `.env` leakage (`SMA_WINDOW=40` etc.) — NOT merge-induced. Count before/after.
- Then commit → rebuild Docker image → recreate `mmn-engine` via `docker run`
  (compose v1 is broken; see deploy skill) → smoke-test live endpoints.
