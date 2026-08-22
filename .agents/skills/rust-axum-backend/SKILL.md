---
name: rust-axum-backend
description: Use when adding Rust features with axum, AppState, or SSE.
category: software-development
---

# Rust + Axum Backend Patterns

Patterns for adding new features to a Rust project that has both a `lib.rs`
and `main.rs` (bin+lib crate roots), an axum HTTP router, shared `AppState`,
and SSE streaming.

## Adding a New Module — Bin+Lib Duality

When a Rust workspace has both `lib.rs` and `main.rs` as crate roots,
adding a new `mod foo;` to `lib.rs` is NOT enough — the bin target
(`main.rs`) compiles separately and needs its own declaration.

```rust
// lib.rs — makes the module available to tests and external crates
pub mod foo;

// main.rs — makes the module available to the bin target
mod foo;
```

**Pitfall**: `error[E0433]: cannot find 'foo' in 'crate'` means the
module is declared in `lib.rs` but not in `main.rs`. The bin target
does NOT see `lib.rs` modules.

## sqlx + SQLite type-affinity trap (COALESCE/SUM panics)

`sqlx::query(...).fetch_one()` panics at runtime with
`ColumnDecode { index: "\"col\"", source: "mismatched types; Rust type
`f64` (as SQL type `REAL`) is not compatible with SQL type `INTEGER`" }`
when the same column returns DIFFERENT SQLite storage classes depending
on the data. SQLite is dynamically typed — the declared column type is
only an affinity hint, and expressions like `COALESCE(SUM(x), 0)` return
a REAL when rows match (sum of a REAL column) but an INTEGER when no
rows match (the bare `0` literal).

This flips the Rust decode type back and forth: decode as `f64` → works
for the rows-present case, panics for the rows-absent case (and vice
versa for `i64`). You cannot fix it by swapping `f64`↔`i64` — each swap
breaks the other case.

**Fix**: force a single storage class in SQL. Either CAST the aggregate
AND use a matching literal:

```rust
// BEFORE — panics when there are zero rows
"SELECT COALESCE(SUM(CASE WHEN side='buy' THEN qty ELSE -qty END), 0) AS net_qty ..."

// AFTER — CAST to REAL, literal 0.0, decode as f64
"SELECT COALESCE(CAST(SUM(CASE WHEN side='buy' THEN qty ELSE -qty END) AS REAL), 0.0) AS net_qty ..."
let net_qty: f64 = row.get("net_qty");
```

The two halves must agree: `CAST(... AS REAL)` pairs with `0.0` and `f64`
decode; `CAST(... AS INTEGER)` pairs with `0` and `i64` decode. Mixing
them is the same bug.

**Also note**: `sqlx` does NOT surface these as `Err` from the query
itself — the panic is at `row.get()` / `row.try_get()` decode time, in a
`tokio-rt-worker` thread, which can take down a request silently (the
HTTP handler never returns). Wrap aggregate queries that can return zero
rows in `try_get()` + `unwrap_or` OR fix the SQL type as above.

## Adding New State to AppState (axum)

When adding a new shared state field (e.g. a background task handle):

1. **Add the field to the struct** in `api/mod.rs`:
   ```rust
   pub struct AppState {
       // ... existing fields ...
       pub advisor: Option<Arc<AdvisorState>>,
   }
   ```

2. **Update the router function signature** to accept the new field:
   ```rust
   pub fn router(
       pool: DbPool,
       config: &Config,
       tx: TelemetrySender,
       advisor: Option<Arc<AdvisorState>>,  // NEW
   ) -> Router { ... }
   ```

3. **Update the AppState construction** inside `router()`:
   ```rust
   let state = AppState {
       pool,
       // ... existing fields ...
       advisor,  // NEW
   };
   ```

4. **Update ALL callers** of `router()` — check `main.rs` and any test
   files that construct `AppState` directly. Missing field → `E0063`.

5. **Update ALL test `AppState` constructors** — every `AppState { ... }`
   in test files must include the new field. Use `None` for disabled state.

6. **Pass the state from `main.rs`**:
   ```rust
   let advisor_state = Some(Arc::new(AdvisorState::new(...)));
   let app = api::router(pool.clone(), &cfg, tx, advisor_state);
   ```

## SSE Streaming with Axum

Use `Sse<impl Stream<Item = Result<Event, Infallible>>>` with a single
`try_unfold` stream — do NOT attempt multiple return types.

**Wrong (type mismatch across branches):**
```rust
if error {
    return Sse::new(stream::once(async { Ok(event) }));   // type A
}
return Sse::new(stream::try_unfold(...));                  // type B
// ERROR: mismatched types — Sse<T> is parameterized by a single type
```

**Right (single stream, handle errors inside the closure):**
```rust
let stream = stream::try_unfold(
    (state, false),
    move |(state, sent)| async move {
        if sent {
            return Ok::<_, Infallible>(None);
        }
        // Handle errors here, emit error events, never return early
        // from the outer function.
        let event = Event::default().data("error").event("error");
        Ok(Some((event, (state, true))))
    },
);
Sse::new(stream)
```

## Config Addition Checklist

When adding a new `pub field: Type` to the `Config` struct:

1. Add the field to `pub struct Config { ... }`
2. Add env var parsing in `Config::from_env()`:
   ```rust
   let finnhub_api_key = env_or("FINNHUB_API_KEY", "");
   ```
3. Add the field to the `Ok(Self { ... })` return
4. Add the env var to the `clear_engine_env()` test helper
5. Check for any direct `Config { ... }` constructors in tests:
   `grep -rn "Config {" --include='*.rs' | grep -v "from_env"`
   Add the new field with a sensible default (e.g. `""` for strings).

## Arc Clone Before Move (Async Spawn)

When spawning an `async move` block that borrows from a value, clone
the fields you need BEFORE the block — even when the value is a plain
struct (not just `Arc`).

```rust
// Pattern: iterating over a non-Arc collection
for model in &active_models {
    // Clone EVERY borrowed field to an owned value BEFORE the spawn.
    let model_primary = model.primary_symbol.clone();
    let model_id = model.model_id.clone();
    let model_pair = model.pair().to_string(); // &str → String if needed
    let model_pool = pool.clone();              // Arc IS Clone, fine
    let model_tx = tx.clone();
    tokio::spawn(async move {
        // `model` is NOT moved in here — only the cloned locals are.
        // Using `model.primary_symbol` directly inside the spawn would
        // require moving `model` and break the loop.
        scheduler::EquityScheduler::new(
            model_pool, model_primary, model_id, model_pair, /* ... */
        ).await;
    });
}
```

**Pitfall (Arc-specific case):** `Arc` does not implement `Copy`.
Using `state.field` inside the async move block after `state` is moved
→ borrow error. Same rule applies to `&Vec<T>`, `&[T]`, `&str`, or
anything where the iterator borrows from a value that lives outside
the spawn block.

**Generalized rule:** any captured borrow in `tokio::spawn(async move { ... })`
must be cloned to an owned value before the block. The compiler will
catch this with E0373/E0505; the cure is to lift every captured
expression into a `let` BEFORE `tokio::spawn`.

## Patch Tool Pitfalls for Rust Source

`patch` is fuzzy text matching. On structured docs (JSON/YAML/TOML) it
corrupts escaping — covered by `safe-structured-edits`. On **Rust
source**, it has its own class of failures that silently produce
files that LOOK right but FAIL TO COMPILE.

### Pitfall 1: silent mis-indentation

If `new_string` has different indentation than the surrounding context,
`patch` happily writes it. The file compiles only if you're lucky; it
usually does not, and the error message points to a line far away
from where the corruption actually happened.

**Repro:** patching `ws.rs::tests` to update a test signature. The
`new_string` had 8-space indent when the surrounding `mod tests {`
block had 4-space indent for items. Result: every test ended up at
8-space indent, the inner brace depth doubled, and `cargo build` died
with "unexpected closing delimiter".

**Fix — surgical dedent via `execute_code`:**
```python
with open("/path/to/file.rs") as f:
    src = f.read()
i = src.find("#[cfg(test)]\nmod tests")
j = src.find("\n}\n", i) + 2
block = src[i:j+1]
dedented = "\n".join(
    line[4:] if line.startswith("    ") else line
    for line in block.split("\n")
)
open("/path/to/file.rs", "w").write(src[:i] + dedented + src[j+1:])
```

Then `cargo build --bin <target>` to confirm. The single-line fix
path (`patch` again with corrected indentation) often fails because
the surrounding context is now itself mis-indented.

### Pitfall 2: `old_string` matches inside a deeper nesting level than intended

When two near-identical blocks exist (e.g., two `TelemetryEvent::PnlTick { ... }`
sites in a long function), `patch` matches the FIRST one. If your
`old_string` doesn't include enough surrounding context to disambiguate,
you can patch the wrong block AND corrupt its closing braces.

**Repro:** patching `scheduler.rs` to add `model_id`/`pair` to two
`PnlTick` send sites. Both sites had identical field bodies — the
discriminator was a comment line ABOVE the first site. My `old_string`
for the 2nd site accidentally matched a substring that ended at the
first site's `}` — so the replacement consumed an extra `}` from
outside the target block. Build failed with mismatched braces.

**Fix — recovery:**
```bash
git checkout HEAD -- engine/src/scheduler.rs
# Then redo with more surrounding context:
# - Use the comment-discriminator (e.g. "// Publish PnL tick after trade execution.")
#   OR include 2-3 lines BEFORE the target block as anchor.
# - Verify uniqueness with `search_files` showing all matches first.
```

If the patch destroys more than 5 lines of structure, the recovery
time of `git checkout + redo` is faster than surgical repair.

### Pitfall 3: `patch` after another patch may have shifted the file

After a successful patch that adds lines, subsequent patches that
match lines AFTER the new ones can mismatch because line numbers
shifted. Re-read the file (with `read_file` offset/limit) before each
patch — don't trust line-number anchors from a previous read.

### Compile-check after every Rust patch

After any `patch` to a `.rs` file:
```bash
cargo build --bin <bin-target> 2>&1 | grep -E "^error" | head -10
```
A 2-second compile check that catches indentation/brace corruption
beats a 5-minute hunt for the bug. Run this BEFORE moving on to the
next file.

## Back-Compat Shim for Added Required Fields

When adding a required field to a public struct (`PaperExecutor`,
`Config`, etc.) and the existing constructor is called from N places
(tests, legacy paths), the safe refactor is:

1. Add the new field to the struct.
2. In the EXISTING constructor, stamp a synthetic default
   (e.g. `model_id: "legacy".to_string()`, `pair: format!("{}/{}", ...)`).
3. Add a NEW `new_for_X(...)` constructor that takes the new field
   explicitly. Wire the bootstrap path (e.g., `main.rs` per-model loop)
   through the new constructor.
4. Internal callers that already KNOW the model_id/pair (tests,
   bootstrap resolver) use the new constructor. Legacy callers and
   tests pass through unchanged.

**Why this beats "touch every caller":** zero churn in test files and
back-compat callers; the new field is always populated (no `Option<T>`
ambiguity); the type signature documents which path is the new
canonical one vs. the legacy default.

**Repro:** added `model_id`/`pair` to `PaperExecutor`. Existing
constructor `new_for_symbol` got synthetic defaults; new constructor
`new_for_model` took explicit args. Zero test churn.

## Rust Format Specs (Not Python)

| Python | Rust |
|--------|------|
| `{:.1f}` | `{:.1}` |
| `{:.2f}` | `{:.2}` |
| `{:.4f}` | `{:.4}` |

Rust format spec uses `.precision` without the `f` suffix. The `f`
suffix is Python-only.

## SqliteRow `.get()` Needs `use sqlx::Row`

When using `row.get(0)` on `SqliteRow`, you need:
```rust
use sqlx::Row;
```
Without it: `error[E0599]: no method named 'get' found for struct 'SqliteRow'`.

## Crate Pitfalls

### flate2 Feature is `zlib`, Not `gzip`

The `flate2` crate uses `features = ["zlib"]` for gzip compression — there is no `gzip` feature:

```toml
# WRONG
flate2 = { version = "1", features = ["gzip"] }  # error: no feature `gzip`

# CORRECT
flate2 = { version = "1", features = ["zlib"] }
```

The resulting `GzEncoder` from `flate2::write::GzEncoder` produces gzip output when using the `zlib` feature.

### `anyhow::Error` Required for `internal_error` Helper

The `internal_error` helper in API modules typically expects `anyhow::Error`. When passing a `sqlx::Error` or other error type, wrap it:

```rust
.map_err(|e| internal_error("query", anyhow::anyhow!(e)))?
```

## Event Emission from Orchestrator, Not Leaf Components

When a component returns structured results (e.g., an executor returning `Vec<FillResult>`), the caller that holds the event logger should emit events — not the component itself.

**Why:**
- Keeps leaf components (executors, data fetchers) simple and testable
- Centralizes event logic where the logger is owned
- Avoids threading `event_logger` through every constructor

**Pattern:**
```rust
// In scheduler (has event_logger)
match executor.set_target_position(new_pos, price, ts).await {
    Ok(fills) => {
        for fill in &fills {
            // Emit event HERE, not inside executor
            if let Some(logger) = &self.event_logger {
                logger.emit(EngineEvent::trade_fill(...)).await;
            }
        }
    }
}
```

The executor stays pure: it writes to DB and returns structured fills. The scheduler, which orchestrates and owns the logger, handles the event emission.

## Adding a Per-Model Shared Map to AppState

When the backend runs multiple models concurrently and the API needs to
address a specific model's state (e.g. `PUT /api/strategy-config?model_id=X`),
add a `HashMap<String, Arc<RwLock<T>>>` to `AppState`, keyed by `model_id`.

The map must be created **before** the per-model bootstrap loop, populated
**during** the loop (each iteration inserts its model's `Arc<RwLock<T>>`),
then passed into `router()`. The running scheduler holds a clone of the
same `Arc<RwLock<T>>`, so API mutations take effect live on the targeted
model without affecting other models.

```rust
// 1. Add field to AppState
pub struct AppState {
    // ... existing fields ...
    pub strategy_params_by_model: Arc<RwLock<HashMap<String, Arc<RwLock<EquityStrategyParams>>>>>,
}

// 2. Create the map BEFORE the per-model loop in main.rs
let strategy_params_by_model = Arc::new(RwLock::new(HashMap::new()));

// 3. Populate DURING the loop
for model in &active_models {
    let model_params = Arc::new(RwLock::new(EquityStrategyParams { ... }));
    strategy_params_by_model
        .write()
        .await
        .insert(model.model_id.clone(), model_params.clone());
    // Pass model_params.clone() to the scheduler — same Arc handle
    tokio::spawn(async move { /* scheduler uses model_params_clone */ });
}

// 4. Pass to router()
let app = api::router(pool, &cfg, tx, advisor, strategy_params_by_model);

// 5. API handler resolves the right Arc by model_id, falls back to default
async fn handle_get(State(state): State<AppState>, Query(q): Query<ModelIdQuery>) {
    let params = {
        let map = state.strategy_params_by_model.read().await;
        q.model_id.as_ref()
            .and_then(|id| map.get(id).cloned())
            .unwrap_or_else(|| state.strategy_params.clone())
    };
    let sp = params.read().await;
    // ...
}
```

**Key pattern:** the API handler resolves the `Arc<RwLock<T>>` by reading the
map, then drops the map read lock before acquiring the write lock on the
individual params. This avoids holding two locks and allows concurrent access
to different models.

When `model_id` is omitted or unknown, the handler falls back to the default
(global) `strategy_params` field — preserving backward compatibility.

## References

- `references/common-patterns.md` — strum enum serialization, BTreeMap time-series grouping, safe chrono timestamp parsing, gzipped JSON archives, adding Serialize to structs for event payloads.
- `references/per-model-symbol-api.md` — evolving single-symbol axum handlers to per-model / per-symbol handlers, including DB schema changes and test updates.

## Duplicate `.route()` Calls — Panic at Runtime, Not Compile Time

Axum's router rejects duplicate `.route("/api/X", ...)` registrations at the
**first request** (or first `Router::with_state` call) with a panic from
`axum-0.7.9/src/routing/path_router.rs:70`:

```
thread 'main' panicked at axum-0.7.9/src/routing/path_router.rs:70:22:
Overlapping method route. Handler for `GET /api/events` already exists
```

The build succeeds. The bin compiles. Tests run fine. The container starts,
eprintln logs that look healthy, then panics as the router is built — often
breaking the healthcheck and producing a crash-loop with no obvious error in
the path the agent was looking at.

**When this happens:** after a merge that concatenated two branches' `.route(...)`
registrations (e.g. `feature/nvda-multi-asset-and-sentiment-overlay` from
the MarketMoves merge had routed `events` itself, and the long-lived HEAD
branch also routed `events` — both `.route("/api/events", get(...))` lines
ended up in the merged `api/mod.rs`).

**Always run after merging router files:**

```bash
# Find every path that appears more than once in the router
grep -oE '"/api/[^"]*"' engine/src/api/mod.rs | sort | uniq -d
# (the `:d` flag in uniq prints only duplicated lines)
```

`GET /api/X` + `POST /api/X` is fine (different methods, same path) — what
catches the panic is the same method twice. The `uniq -d` output will list
both the legitimate GET+POST pair and the duplicate. Disambiguate by
checking the surrounding `.route(...)` line: only `.route("/api/X", get(...))`
appearing twice is the bug.

**Fix:** delete the duplicate line. Pick the canonical one (the later
registration is usually the one from the feature branch; keep the one
closer to its module's other routes for readability).

**Audit any time `api/mod.rs` is touched across a merge boundary.** Merge
tools auto-merge `.route(...)` lines as if they were ordinary code; the
axum router is the only thing that knows two identical prefixes+suffs are
illegal.