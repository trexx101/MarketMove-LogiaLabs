# MarketMoves Dashboard UI — Frontend Gotchas

The Svelte 4 frontend has a few structural traps that have shipped as
production bugs. This file captures them with the exact fix line so the
next session can diagnose in one read.

## Gotcha 1 — `chartData` is a DERIVED store, cannot `.set()` it

`frontend/src/lib/stores.js` exports:

```js
export const chartData = derived(
  [activeModelId, _slices],
  ([$id, $slices]) => $slices[$id]?.chartData ?? null,
);
```

It is **derived** from the active-model's slice — `writable` nowhere in its
chain. Calling `.set()` on it throws `TypeError: store.set is not a function`
(or is silently dropped in some Svelte 4 builds). This bit us live: the
chart's "1M/3M/6M/1Y/ALL" timeframe buttons were silent no-ops because
`CandlestickChart.refreshChart()` did:

```js
candles = data.candles || [];   // sets local OK
sma = data.sma || [];
...
chartData.set(data);            // ← throws, caught by try/catch
draw();                         // ← never reached
```

The fix: remove the `chartData.set(data)` line. The chart draws from local
`candles`/`sma`, which are already set. Initial/slice-driven data still
arrives via the existing `$: if ($chartData)` reactive block.

**Symptom in the wild:** clicks on timeframe buttons appear to do nothing
(no console error, no chart change). Backend was always correct; only the
frontend wiring was broken.

**Rule of thumb when adding a new global store:** if you write
`export const X = derived(...)`, NEVER `.set()` it from a component. Write
to the underlying `_slices` via `setSlice(modelId, field, value)` or use
`writable` for true global state.

## Gotcha 2 — Per-equity vs global trade history

The Dashboard's `loadModelData` was calling
`fetchEquityTrades(symbol || '*', 200)` where `symbol` was the selected
model's `primary_symbol` (QQQ/SMH/XLF). This made the SOXS buy (the
inverse-leg of a short) invisible when viewing SMH, and made each model's
history look sparse (XLF had 1 trade from 2026-06-10, looking like just
noise).

**Design intent (from dashboard preferences):** "trade history is global
across all models with model_id/symbol badge."

**Fix:** Dashboard fetches `fetchEquityTrades('*', 500)` always — global.
The backend's `handle_equity_trades` already supported `symbol='*'` via
`fetch_recent_all_equity_trades`. Required backend change: `EquityTradePoint`
response struct and the SELECT for both `fetch_recent_equity_trades` and
`fetch_recent_all_equity_trades` had to include the `symbol` column so the
UI could badge each row. The `TradeRow` struct gained a `symbol: String`
field; the legacy `fetch_recent_trades` (queries the no-symbol `trades`
table, `#[allow(dead_code)]`) gets `symbol: String::new()` as a default.

**UI:** `frontend/src/lib/components/TradeHistory.svelte` got a "Symbol"
column with a monospace badge per row. Header now reads "Trade History
(all symbols)" so the global nature is unmistakable.

## Gotcha 3 — `model_id` selector filters both the dashboard data AND the trade history by accident

If you grep Dashboard.svelte for `activeModelId`, every data fetch is
correctly keyed by the active model slice — EXCEPT trades, which used
`m.primary_symbol` directly. This double-binding (per-model slice + per-
symbol backend query) is what hid inverse-leg trades. Lean on the slice
pattern only; let the backend handle scoped queries via a single explicit
`*` for "all" or a clearly-documented `model_id` query param.

## Deployment note: `trading_models` registry paths must match the volume mount

When `feature/nvda-multi-asset-and-sentiment-overlay` merged, the new
per-model bootstrap loop hits `features::equities_v2::EquityNormStats::load_named`
for each enabled row. If the registry's `norm_stats_path` doesn't exist
on disk, `main.rs` calls `process::exit(1)` silently — the container
crash-loops with no panic message in the log, only the `eprintln` line
`norm_stats error for {model_label}: {e:#}` immediately before exit.

The `trading_models` rows were seeded externally with paths like
`/models/SMH/norm_stats_smh_v1.json` (leading slash, root-level) but the
container's volume is mounted at `/app/models`. The fix is to UPDATE the
rows to `/app/models/SMH/...` paths. See `references/merge-recovery-and-boot-crash-loop.md`
in the parent marketmoves-dev skill for the SQL.

## Duplicated routes panic at runtime, not compile time

Axum's router rejects duplicate `.route("/api/X", ...)` at app build time
with a panic from `axum-0.7.9/src/routing/path_router.rs:70`:

```
thread 'main' panicked at .../path_router.rs:70:22:
Overlapping method route. Handler for `GET /api/events` already exists
```

A merge that concatenated two branches' `.route(...)` registrations will
NOT compile cleanly — only the binary panics on startup. Always grep
`.route("/api/...` and `grep -oE '"/api/[^"]*"' api/mod.rs | sort | uniq -d`
after merging router files to confirm no duplicates (`:d` flag = lines
appearing more than once).

## Pre-existing test failures (baseline)

`cargo test --release --lib` in the merged repo has 4 known-failing tests
that are NOT regressions from the merge:

```
test api::tests::accuracy_returns_503_when_no_resolved ... FAILED
test config::tests::defaults_load_when_env_unset ... FAILED
test config::tests::live_mode_falls_back_to_paper ... FAILED
test config::tests::shorting_default_is_disabled ... FAILED
```

Root cause: repo `.env` leaks `SMA_WINDOW=40` and friends into `Config::from_env`
during tests, so the `defaults`/`live_mode`/`shorting_default` tests fail
because the env wins over the test's hardcoded "unset" assumption. The
`accuracy_returns_503_when_no_resolved` test depends on those config
defaults. Fix in a separate PR; do NOT block current work on it.

A fix to `TradeRow` (adding `symbol` field) actually reduced the failure
count from 5 to 4 — one test that was checking `TradeRow` construction
without `symbol` was satisfied by the new field.
