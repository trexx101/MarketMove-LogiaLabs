# Phase 5 — Multi-Model Registry + Sentiment Overlay: Implementation Recipes

> Status (2026-08-05): **§8 steps 1-6 + frontend (steps 7-14) + 2B/2E
> SHIPPED.** 10 commits on `feature/nvda-multi-asset-and-sentiment-overlay`:
> - `c63db48` — trading_models DB registry (Recipe 1)
> - `d3621a3` — per-model bootstrap loop (Recipe 2)
> - `acdbf4f` — telemetry model_id+pair attribution (Recipe 3)
> - `49b0919` — emit_for_model on scheduler emits (Recipe 3 cont.)
> - `5ca339e` — /api/models CRUD endpoints (Recipe 3 cont.)
> - `2b51851` — per-model strategy-config via model_id (Recipe 3 cont.)
> - `4959e48` — frontend multi-model store partitioning (Recipe 7)
> - `7cb0a89` — NVDD + PSQ added to EQUITY_SYMBOLS (step 2B)
> - `57c542c` — model registration SQL script (step 2E)
>
> **Still deferred:** Sentiment overlay (Recipes 4-6), live VPS deploy
> + integration test. 193 Rust tests green, frontend builds clean.

This file is the concrete how-to companion to the SKILL.md "Phase 5"
section. The architecture pivoted in 2026-08-05 from env-driven
`Vec<SymbolConfig>` to a **DB-backed `trading_models` registry**.
Recipes 1-3 are SHIPPED; Recipes 4-6 (sentiment) remain deferred;
Recipe 7 (frontend) is SHIPPED; Recipe 8 (verification) is SHIPPED.

---

## Recipe 1 — `trading_models` registry (db.rs) — SHIPPED (c63db48)

The DB-backed registry. `engine/src/db.rs::DDL` gains a new table:
```sql
CREATE TABLE IF NOT EXISTS trading_models (
    model_id        TEXT    PRIMARY KEY,
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
CREATE INDEX IF NOT EXISTS trading_models_enabled_idx
    ON trading_models (enabled);
```

**Pitfall — `DDL.split(';')` is a naive splitter.** Any `--` line comment
containing `;` will break every test in `db::tests::*`. Rephrase such
comments — for example, use `e.g. NVDA/NVDD` -> `e.g. NVDA-NVDD` or move
the semicolon-bearing example out of the comment and into a `--` line
that does not split. See SKILL.md "DDL comment pitfall" for the full
story; it caused 14 test failures on 2026-08-05 from a single
incidental `;`.

Also added: `bootstrap_default_model()` (in db.rs, NOT main.rs, for
unit-testability — binary crates can't easily host unit tests) and
`resolve_active_models()` which loads enabled rows or falls back to
bootstrap. 17 db tests pass (12 pre-existing + 5 new: bootstrap,
resolve-empty, resolve-with-rows, + the original 3 registry tests).

### Struct + CRUD

```rust
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct TradingModel {
    pub model_id: String,
    pub primary_symbol: String,
    pub inverse_symbol: String,
    pub model_path: String,
    pub norm_stats_path: String,
    pub budget_usd: f64,
    pub enabled: bool,
    pub deployed_at: i64,
    pub last_wf_ic: Option<f64>,
    pub last_wf_at: Option<i64>,
    pub notes: Option<String>,
}

impl TradingModel {
    pub fn pair(&self) -> String {
        format!("{}/{}", self.primary_symbol, self.inverse_symbol).to_uppercase()
    }
    pub fn is_enabled(&self) -> bool { self.enabled }
}
```

CRUD functions: `register_model`, `update_model_enabled`,
`load_model_by_id`, `load_all_models`, `load_enabled_models`,
`record_walk_forward_result`, `bootstrap_default_model`,
`resolve_active_models`.

---

## Recipe 2 — Bootstrap + main.rs scheduler loop — SHIPPED (d3621a3)

### Cold-start fallback (preserves Wave A behavior)

When `trading_models` is empty at startup, `resolve_active_models()`
returns a synthetic `bootstrap-default` model built from
`Config::symbol` / `Config::short_symbol` / `Config::norm_stats_path`.
The bootstrap model_id is the literal string `"bootstrap-default"`
(NOT `bootstrap-<symbol>` as originally planned — the resolver returns
a single TradingModel without writing to the DB).

### Scheduler loop

Key invariants (all shipped in d3621a3):
- One `EquityScheduler` task per enabled model, each with its own
  `norm_stats` and `bridge`.
- One `PaperExecutor` per model (NOT shared) — each model gets its own
  via `build_paper_executor_for_model()`.
- `model_id` and `pair` are cloned into each spawned task for telemetry
  attribution (§8.3).
- `strategy_params_by_model` map is populated during the loop and passed
  to `router()` for per-model PUT targeting (§8.6).

**Pitfall — engine CWD vs repo root.** The engine uses relative paths
like `models/norm_stats_qqq_v1.json`. These resolve correctly when the
engine is run from the repo root (`cargo run --bin engine --manifest-path
engine/Cargo.toml`), but fail with "No such file or directory" when run
from inside `engine/`. The production DB is at `engine/data/candles.db`
(CWD-relative), NOT the repo-level `data/candles.db`. When registering
models via SQL, target `engine/data/candles.db`, not `data/candles.db`.

---

## Recipe 3 — Telemetry enrichment + API endpoints — SHIPPED (acdbf4f, 49b0919, 5ca339e, 2b51851)

### Telemetry enrichment (§8.3 — acdbf4f)

5 per-model TelemetryEvent variants gained `model_id: String` +
`pair: String`: PnlTick, PredictionUpdate, FeatureUpdate, TradeFill,
StalenessAlert. EngineEvent stays shape-stable but gets
`emit_for_model(model_id, payload)` helper for Events tab attribution.
Global variants (ModeChange, StrategyConfigChange, AdvisorBriefing)
untouched.

`EquityScheduler` struct gained `model_id` and `pair` fields.
`PaperExecutor` gained `model_id`, `pair` fields + `new_for_model()`
constructor. `main.rs` plumbs `model.model_id` + `model.pair()` into
both constructors.

**Pitfall — `pub(crate)` vs `pub` for external verification harnesses.**
Ad-hoc verification crates under `/tmp/` that link `engine` as a path
dependency cannot access `pub(crate)` modules. Use `pub mod` for modules
that external harnesses need to call directly (models, strategy_config).
The in-crate unit tests cover the `TelemetryEvent` serde roundtrips that
external harnesses can't reach (TelemetryEvent is crate-private).

### emit_for_model on scheduler (§8.4 — 49b0919)

3 `EventLogger::emit()` call sites in `scheduler.rs` swapped to
`emit_for_model(event, &self.model_id, Some(&self.pair))`. Global
call sites (main.rs engine_started, api/mode.rs, api/strategy_config.rs)
left on `emit()` — they are model-agnostic.

**Pitfall — `replace_all` for identical emission sites.** The two
`trade_fill` emission sites in scheduler.rs are textually identical.
Use `replace_all=true` to patch both in one call.

### /api/models CRUD (§8.5 — 5ca339e)

`GET /api/models`, `POST /api/models`, `PUT /api/models/:id/enabled`.
Thin wrappers over existing db functions. 4 new api tests (list empty,
register+list, toggle, 404).

### Per-model strategy-config (§8.6 — 2b51851)

`GET/PUT /api/strategy-config?model_id=X`. `AppState` gained
`strategy_params_by_model: Arc<RwLock<HashMap<String, Arc<RwLock<EquityStrategyParams>>>>>.
`main.rs` populates the map during the per-model loop. `router()`
signature updated to accept the map. `resolve_params()` helper picks
the right handle; falls back to default when model_id is unknown.

**Pitfall — AppState construction blast radius.** Adding a field to
`AppState` means every construction site needs updating. There are only
2: `api/mod.rs` (router builder) and `api/tests.rs` (test helper).
Both need the new field. The `router()` call in the SPA integration test
also needs the extra arg.

4 new tests: get default, get fallback on unknown id, get per-model,
put per-model with live Arc verification (the test holds a clone of
the Arc and verifies the write propagated).

---

## Recipe 4 — `db::fetch_recent_sentiment` (DEFERRED)

Next chunk after sentiment overlay work begins. Lives next to
`migrate_sentiment_cache` in `engine/src/db.rs`:

```rust
pub async fn fetch_recent_sentiment(
    pool: &DbPool, symbol: &str, lookback_hours: i64,
) -> Result<Option<(f64, i64)>> {
    let since = chrono::Utc::now().timestamp() - lookback_hours * 3600;
    let row: Option<(f64, i64)> = sqlx::query_as(
        "SELECT score, buzz FROM sentiment_cache
         WHERE symbol = ? AND ts >= ? ORDER BY ts DESC LIMIT 1"
    )
    .bind(symbol).bind(since).fetch_optional(pool).await?;
    Ok(row)
}
```

WARNING: The existing `sentiment_cache` DDL has columns `symbol`, `date`,
`score`, `source`, `buzz`, `weekly_avg`. There is no `ts` column -- only
`date` (TEXT 'YYYY-MM-DD'). The `WHERE ts >= ?` filter above needs to
either be `WHERE date >= ?` (and bind a date string), or the table
needs a `ts` migration. Verify against the actual DDL before landing.

---

## Recipe 5 — `apply_sentiment_overlay` + scheduler.rs hook (DEFERRED, spec amended 2026-08-05)

**Locked 2026-08-05 — implementation deviation:** The plan §3C
originally proposed `apply_sentiment_overlay(target, score, count, params)
-> (Position, f64)` with a `f64` size_multiplier to halve qty when
score < -0.5. After design review, Inah approved a **cleaner
interpretation that avoids executor surgery**:

```rust
/// Apply sentiment overlay to a target Position.
/// Returns the Position AFTER applying the overlay. NO size_multiplier —
/// halving qty mid-hold was rejected as invasive.
pub fn apply_sentiment_overlay(
    target: Position,
    score: f64,
    article_count: i64,
    params: &EquityStrategyParams,
) -> Position {
    // Overlay off OR insufficient data → no effect.
    if !params.enable_sentiment_overlay
        || article_count < params.sentiment_min_articles
    {
        return target;
    }
    // Rule 2 (hard exit): extreme negative → flatten any position.
    if score < params.sentiment_exit_threshold {
        return Position::Flat;
    }
    // Rule 1 (block entries): moderate negative → if currently Flat,
    // stay Flat (do not enter). If holding, keep holding — exits still
    // fire via next_equity_position's exit_threshold logic.
    if score < params.sentiment_reduce_threshold {
        return match target {
            Position::Flat => Position::Flat,
            Position::Long | Position::Short => target,
        };
    }
    target  // neutral or positive sentiment → no effect
}
```

Integration point in `scheduler.rs::evaluate_and_execute_strategy` (NOT
`finalize_candle` — the strategy evaluation happens one function deeper):

```rust
let mut target = strategy::next_equity_position(current_pos, &input, &params);

// Sentiment overlay (applied per-model, AFTER next_equity_position)
let (score, article_count) = db::fetch_recent_sentiment(&pool, &symbol, 48)
    .await?
    .unwrap_or((0.0, 0));
target = strategy::apply_sentiment_overlay(target, score, article_count, &params);
```

The overlay runs **after** `next_equity_position` so the base
state-machine exits still fire normally (pred_1d < exit_threshold
exits). The overlay only changes outcomes by either forcing Flat on
extreme negative sentiment OR keeping the target Flat when the base
machine would have entered.

**Do NOT thread a size_multiplier through the executor's `qty` argument.**

---

## Recipe 6 — Sentiment fields on `EquityStrategyParams` (DEFERRED)

```rust
pub struct EquityStrategyParams {
    #[serde(default)]
    pub enable_sentiment_overlay: bool,        // default false
    #[serde(default = "default_sentiment_reduce")]
    pub sentiment_reduce_threshold: f64,       // default -0.5
    #[serde(default = "default_sentiment_exit")]
    pub sentiment_exit_threshold: f64,         // default -0.8
    #[serde(default = "default_sentiment_min_articles")]
    pub sentiment_min_articles: i64,           // default 15
}
```

---

## Recipe 7 — Frontend multi-model UI — SHIPPED (4959e48)

### stores.js refactor

The flat stores (`status`, `predictions`, `features`, `trades`,
`accuracy`, `chartData`) are now **derived stores** that proxy to the
active model's slice. The internal `_slices` writable holds
`{ [model_id]: { status, predictions, features, trades, accuracy, chartData } }`.

Key new exports:
- `activeModelId` — writable store, the selected model_id
- `models` — writable store, array of TradingModel from /api/models
- `modelSlice` — derived store, the active model's full slice
- `updateSlice(modelId, field, updater)` — functional update to a slice field
- `setSlice(modelId, field, value)` — non-functional set

Legacy flat stores are `derived([activeModelId, _slices])` — they
return the active model's field. This means **un-migrated components
that read `$status` automatically get per-model data** once
`activeModelId` is set.

### websocket.js routing

`handleMessage()` reads `msg.model_id` from per-model events
(PnlTick, PredictionUpdate, FeatureUpdate, TradeFill, StalenessAlert)
and calls `updateSlice(mid, field, ...)`. Global events (ModeChange,
StrategyConfigChange, EngineEvent) stay on the global stores.

### api.js additions

`fetchModels()`, `registerModel(body)`, `setModelEnabled(modelId, enabled)`.
`fetchStrategyConfig(modelId)` and `saveStrategyConfig(params, modelId)`
append `?model_id=X` as a query param.

### Dashboard.svelte model selector

A `<select>` in the dashboard header binds to `$activeModelId`. On
mount, fetches `/api/models`, picks the first enabled model, loads its
data into the correct slice. `loadModelData(modelId, symbol)` fetches
status, predictions, chart, accuracy, trades into the model's slice.

### Component updates

- `StrategyConfigPanel.svelte` — passes `$activeModelId` to API calls,
  reloads config on model switch
- `PnLEquityCurve.svelte` — resets `pnlHistory` on model switch, loads
  per-model trade history using the model's `primary_symbol`
- `FeatureInspector.svelte` — reloads features on model switch using
  the model's `primary_symbol`
- `Events.svelte` — shows `model_id` badge per event row (from
  `payload.model_id`)

**Pitfall — Svelte store isolation testing.** To test store
partitioning logic in Node (without a browser), the test script must be
run from inside `frontend/` so that `svelte` resolves from
`node_modules/`. Running from `/tmp/` fails with
`ERR_MODULE_NOT_FOUND: Cannot find package 'svelte'`. Copy the test
script into `frontend/`, run it, then delete it.

---

## Recipe 8 — Verification gates — SHIPPED

```bash
# Build clean
cd /home/ubuntu/projects/MarketMoves && cargo build --bin engine

# Full lib tests (193 pass, 23 pre-existing config failures)
cargo test --lib

# Targeted: db tests (17 pass)
cargo test --lib db::tests::

# Targeted: api tests (11 pass + 1 pre-existing accuracy failure)
cargo test --lib api::tests::

# Targeted: event tests (emit_for_model)
cargo test --lib event::tests::emit_for_model

# Frontend builds
cd frontend && npm run build

# Engine boot verification (run from repo root, NOT engine/)
timeout 120 cargo run --bin engine --manifest-path engine/Cargo.toml 2>&1 \
  | grep -iE "loaded enabled|bootstrapping scheduler|model_idx"

# API endpoint verification (engine must be running)
curl -s http://localhost:8080/api/models | python3 -m json.tool
curl -s "http://localhost:8080/api/strategy-config?model_id=nvda-v1" | python3 -m json.tool
```

### Ad-hoc verification pattern

External Rust harnesses under `/tmp/hermes-verify-*/` link `engine` as
a path dependency. They can test DB persistence, broadcast wire format,
and public constructors. They CANNOT access crate-private types like
`TelemetryEvent` — those are covered by in-crate unit tests.

**Pitfall — `pub` vs `pub(crate)` for harnesses.** If the harness needs
to call handlers directly (e.g. `api::models::handle_list_models`),
the module must be `pub mod`, not `pub(crate) mod`. `pub(crate)` only
works within the same crate.

**Pitfall — harness CWD after `rm -rf`.** When deleting the harness
directory, the shell's CWD becomes invalid. The next `cd` command fails
with "No such file or directory". Fix: always `cd` to the repo root
before running subsequent commands.

### Step 2B — NVDD + PSQ ingest (SHIPPED — 7cb0a89)

One-line change to `EQUITY_SYMBOLS` in `engine/src/data/mod.rs`: added
`"PSQ"` (QQQ inverse ETF, was missing from ingest list) and `"NVDD"`
(NVDA inverse ETF, the short leg for the NVDA model). Both are standard
Yahoo Finance tickers.

### Step 2E — Model registration (SHIPPED — 57c542c)

`scripts/register_models.sql` inserts `qqq-v1` and `nvda-v1` into the
`trading_models` table. Uses `INSERT OR IGNORE` for idempotency. Run
against `engine/data/candles.db` (the engine's CWD-relative DB), NOT
the repo-level `data/candles.db`.

**Pitfall — production DB doesn't have trading_models table yet.** The
table is created by the engine's DDL on first boot. If registering
models against a DB that hasn't been booted with the new engine, you
must create the table manually first (the SQL script assumes the table
exists). The DDL's naive `;` splitter means you can't just run the
whole DDL from Python — create the specific table with a direct
`CREATE TABLE IF NOT EXISTS` statement.

---

## Cross-references

- Plan doc: `.hermes/plans/2026-08-05_nvda-multi-asset-and-sentiment-overlay.md`
- Branch: `feature/nvda-multi-asset-and-sentiment-overlay` (10 commits
  from `b429614` through `57c542c`)
- Parity fixes (1A/1B/1D): `engine/src/features/equities_v2.rs`
- Notebook: `models/colab/EQ_Equities_Model.ipynb` (SYMBOL-parameterized)
- NVDA artifacts: `models/NVDA/` — mean IC 0.0823 (gate 0.03, 2.74x margin)
- QQQ artifacts: `models/` — mean IC 0.034
- Registration SQL: `scripts/register_models.sql`