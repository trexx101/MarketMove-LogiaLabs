# Multi-Model Registry — bootstrap pattern + DB API

Session work captured 2026-08-05. Companion to `.hermes/plans/2026-08-05_nvda-multi-asset-and-sentiment-overlay.md` §8.

## The problem (and why the model is the unit, not the symbol)

Originally the engine treated `cfg.symbol` as the single source of truth for "what we trade." To run QQQ + NVDA in parallel, you'd need:

- Two independent schedulers (one per symbol)
- Two paper executors (each with its own position state)
- Two norm-stats files (per-model feature scaling)
- Per-model strategy thresholds (NVDA may need different entry/exit)
- Per-model budget tracking (how much of the portfolio to allocate)

The **trading_models** registry is the abstraction that makes this tractable. Each row owns a primary+inverse symbol pair, the path to its trained artifact, the dollar budget, and enable/disable state. The engine boots one `(scheduler, executor)` pair per `enabled = 1` row.

## Schema

```sql
CREATE TABLE IF NOT EXISTS trading_models (
    model_id        TEXT    PRIMARY KEY,        -- uuid (text), NOT a ticker
    primary_symbol  TEXT    NOT NULL,
    inverse_symbol  TEXT    NOT NULL,
    model_path      TEXT    NOT NULL,
    norm_stats_path TEXT    NOT NULL,
    budget_usd      REAL    NOT NULL DEFAULT 5000.0,
    enabled         INTEGER NOT NULL DEFAULT 1,
    deployed_at     INTEGER NOT NULL,
    last_wf_ic      REAL,
    last_wf_at      INTEGER,
    notes           TEXT
);
```

Two indexes:

```sql
CREATE INDEX trading_models_enabled_idx       ON trading_models (enabled);
CREATE INDEX trading_models_primary_symbol_idx ON trading_models (primary_symbol);
```

Sub-second timestamp pitfall: `deployed_at` is `Utc::now().timestamp()` (seconds). If multiple rows are inserted in one tick, sort by `deployed_at` alone is non-deterministic. Tests must use membership checks, not positional access.

## DB API (engine/src/db.rs)

The `TradingModel` struct provides `pair()` (derived `"PRIMARY/INVERSE"` label) and `is_enabled()` (mirror of the `enabled` column).

| Function | Purpose |
|---|---|
| `register_model(pool, uuid, primary, inverse, model_path, norm_stats_path, budget_usd, notes) -> TradingModel` | Insert a new model. Always enabled. |
| `load_model_by_id(pool, id) -> Option<TradingModel>` | Single-row lookup. |
| `load_all_models(pool) -> Vec<TradingModel>` | Full registry, newest-first. |
| `load_enabled_models(pool) -> Vec<TradingModel>` | The set the engine boots, oldest-first. |
| `update_model_enabled(pool, id, enabled) -> Option<TradingModel>` | Toggle. Returns `None` for unknown id. |
| `record_walk_forward_result(pool, id, last_wf_ic, last_wf_at) -> Result` | Persist IC + timestamp from a walk-forward run. |
| `bootstrap_default_model(primary, inverse, norm_stats) -> TradingModel` | Pure function, no DB. Synthetic model with reserved id `"bootstrap-default"`. |
| `resolve_active_models(pool, primary, inverse, norm_stats) -> (Vec<TradingModel>, usize)` | The bootstrap resolver. Returns `(models, loaded_count)` where `loaded_count=0` means cold-start fallback. |

## Bootstrap resolver pattern

The resolver is the entry point at startup. It encapsulates the "registry or fallback" decision so it can be unit-tested without spinning up the full engine.

```rust
pub async fn resolve_active_models(
    pool: &DbPool,
    primary_symbol: &str,
    inverse_symbol: &str,
    norm_stats_path: &str,
) -> Result<(Vec<TradingModel>, usize)> {
    let rows = load_enabled_models(pool).await?;
    if rows.is_empty() {
        Ok((vec![bootstrap_default_model(primary_symbol, inverse_symbol, norm_stats_path)], 0))
    } else {
        let n = rows.len();
        Ok((rows, n))
    }
}
```

Why the `(Vec, usize)` tuple return? The `loaded_count` lets the caller log "cold-start" vs "registry" without re-querying, and it's a cheap way to make assertions in tests (`assert_eq!(count, 0)`) without inspecting the vec.

**First attempt was inline in `main.rs`** — turned out to be untestable from a binary's `main` function. Extracting to `db.rs` let the test module add `resolve_active_models_falls_back_to_bootstrap_when_empty` and `resolve_active_models_uses_registry_when_present` directly. **Lesson:** write resolvers as `pub` functions in the data layer from the start, not inside `main()`.

## Per-model bootstrap loop in main.rs

```rust
let (active_models, _loaded_count) = db::resolve_active_models(
    &pool,
    &cfg.symbol,
    &cfg.short_symbol,
    &cfg.norm_stats_path,
).await?;

// ... logging

for (idx, model) in active_models.iter().enumerate() {
    // Per-model norm_stats load (fail-fast on missing file)
    let model_norm_stats = features::equities_v2::EquityNormStats::load_named(&model.norm_stats_path)?;

    // Per-model paper executor
    let model_executor = Arc::new(RwLock::new(build_paper_executor_for_model(
        &cfg, pool.clone(), &model.primary_symbol, &model.inverse_symbol, tx.clone(),
    )));

    // Per-model strategy params (shared via Arc — future per-model tuning)
    let model_strategy_params = Arc::new(RwLock::new(EquityStrategyParams { /* ... */ }));

    // CRITICAL: clone fields into the spawned task to avoid borrowing active_models
    let model_primary = model.primary_symbol.clone();
    let model_tx = tx.clone();
    tokio::spawn(async move {
        let sched = EquityScheduler::new(
            pool.clone(),
            model_primary,
            &zmq_endpoint,
            model_norm_stats,
            feature_window_size,
            model_strategy_params,
            trading_mode.clone(),  // shared across models — runtime mode toggle
            model_executor,
            Some(model_tx),
            Some(event_logger.clone()),
        ).await?;
        sched.run().await
    });
}
```

Two critical borrow rules:

1. **Clone fields into the spawned task** (`model_primary = model.primary_symbol.clone()`). The `tokio::spawn` requires `'static`, so you can't borrow `model` from the loop iterator.
2. **Per-model `tx`** — each telemetry sender is a clone of the broadcast channel. Multiple senders to the same channel are fine; the receiver sees both. This is what lets the frontend (in a future step) route events by `model_id`.

## API endpoints (planned §8.4)

| Endpoint | Purpose |
|---|---|
| `GET /api/models` | Registry, all rows. |
| `POST /api/models` | Register a new model. Validates `model_path` + `norm_stats_path` exist before insert. |
| `PUT /api/models/{id}/enabled` | Toggle on/off. |
| `PUT /api/strategy-config` | Now takes `{model_id, ...params}` so per-model thresholds work. |

Frontend changes (planned §8.6–8.8) are out of scope for this reference — see plan §8.

## Verification harness (the 12-check ad-hoc pattern)

For any new `db::` API surface, the ad-hoc verification pattern is:

```rust
// /tmp/hermes-verify-<feature>-<date>/src/main.rs
use engine::db;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut passed = 0; let mut failed = 0;
    let pool = db::open("sqlite::memory:").await?;

    // 12 named assertions covering the full API surface
    // ... each prints PASS or FAIL with detail

    println!("SUMMARY: {passed} passed, {failed} failed");
    if failed == 0 { Ok(()) } else { std::process::exit(1) }
}
```

Plus a `Cargo.toml` that links `engine` as a path dep + `tokio` + `sqlx` (for `PRAGMA` queries). Run from the crate root: `cargo run --quiet`. Always delete the crate after the run.

This was used for both step 1 (trading_models CRUD) and step 2 (bootstrap resolver). 12/12 + 9/9 PASS respectively. **Don't reuse the same crate across commits** — the point is fresh, self-contained evidence.
