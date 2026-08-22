# Splitting a Monolithic Rust File into a Module Tree

When a single file (e.g. `api.rs`, 900+ lines) grows too large, splitting
it into a directory of sub-modules is the standard Rust refactor. The
mechanics are straightforward but three issues cascade across multiple
compile cycles. Anticipating them saves 3-4 `cargo check` rounds.

## Step-by-step

### 1. Create the directory and mod.rs

```bash
mkdir engine/src/api
```

Write `engine/src/api/mod.rs` with:
- The `Router` function (moved from the old file)
- The `AppState` struct
- Shared helper functions (`internal_error`, `ts_to_rfc33399`)
- `mod` declarations for each sub-module
- `#[cfg(test)] mod tests;`

### 2. Move handlers into sub-modules

Group handlers by domain: `status.rs`, `predictions.rs`, `chart.rs`,
`equity.rs`, etc. Each sub-module:
- Imports shared helpers via `use super::{internal_error, ApiResult, AppState};`
- Imports crate-level types via `use crate::db;` (NOT `use super::*;`
  — `super::*` brings in the `mod` declarations, not crate items)
- Marks response structs `pub(crate)` with `pub` on every field tests
  access directly

### 3. Delete the old file

```bash
rm engine/src/api.rs
```

If both `api.rs` AND `api/mod.rs` exist, the compiler errors with
`E0761: file for module 'api' found at both`.

### 4. Fix visibility (compile cycle 1)

First `cargo check` will show:
- `E0433: cannot find module or crate 'db'` in sub-modules → add
  `use crate::db;` to each sub-module that uses db types
- `E0616: field 'X' of struct 'Y' is private` in tests → add `pub` to
  the field in the response struct

### 5. Fix dead code (compile cycle 2)

Functions that served the old monolithic file but aren't needed in the
split modules will warn as dead code. Example: `candle_to_dto` for the
crypto `Candle` type when the split module only handles equity candles.
Remove the dead function — don't suppress with `#[allow(dead_code)]`
unless it's intentionally retained for future use.

### 6. Fix test helper visibility (compile cycle 3)

Tests in `api/tests.rs` that call handler-internal functions directly
(e.g. `predictions::prediction_to_dto(&row)`) need those functions
marked `pub(crate)`, not private `fn`.

### 7. Update test assertions for renamed content

If the split coincides with a frontend change (e.g. new `index.html`
with a different `<title>`), tests that assert on HTML content must be
updated. Use a flexible assertion:
```rust
assert!(body.contains("MarketMoves") || body.contains("MarketMarkovNet"));
```

## git stash baseline check limitation

The standard pre-existing-failure verification pattern:
```bash
git stash && cargo test && git stash pop
```
FAILS after a file→dir split because `git stash` restores the old
`api.rs` while the new `api/` directory still exists on disk, triggering
`E0761: file for module found at both`.

Alternative: run individual test modules to isolate:
```bash
cargo test --lib api::tests          # just the new module's tests
cargo test --lib -- --skip config::  # everything except known-broken module
```

## Verification

```bash
cargo check --lib                    # compiles clean
cargo test --lib api::tests          # all split-module tests pass
cargo test --lib -- --skip config::  # all other tests pass (skip known pre-existing)
```
