# Per-Model / Per-Symbol API Handlers in axum

When a Rust/axum backend evolves from a single-symbol system to multiple models running concurrently, every read/write path that implicitly uses `state.symbol` must become parameterizable by `model_id` or `symbol`.

## Database schema first

Before touching handlers, make the tables store the model identity. Typical minimum changes:

```sql
-- positions / position_events: add model_id + symbol
CREATE TABLE IF NOT EXISTS positions (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    model_id   TEXT    NOT NULL DEFAULT '',
    symbol     TEXT    NOT NULL DEFAULT '',
    candle_ts  INTEGER NOT NULL,
    position   INTEGER NOT NULL,
    pred_4h    REAL    NOT NULL,
    pred_24h   REAL    NOT NULL,
    regime     INTEGER NOT NULL,
    sma        REAL    NOT NULL,
    created_at INTEGER NOT NULL DEFAULT 0
);

-- trades: symbol may already exist; add model_id
CREATE TABLE IF NOT EXISTS equity_trades (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    model_id      TEXT    NOT NULL DEFAULT '',
    symbol        TEXT    NOT NULL,
    candle_ts     INTEGER NOT NULL,
    side          TEXT    NOT NULL,
    qty           REAL    NOT NULL,
    price         REAL    NOT NULL,
    fee           REAL    NOT NULL,
    realized_pnl  REAL    NOT NULL,
    created_at    INTEGER NOT NULL DEFAULT 0
);
```

Use `DEFAULT ''` so existing rows remain valid; backfill later if you need real values.

## Update writes before reads

The scheduler/executor must persist `model_id` and `symbol`. Example signatures:

```rust
pub async fn insert_position_event(
    pool: &DbPool,
    model_id: &str,
    symbol: &str,
    candle_ts: i64,
    position: i64,
    // ... other fields
) -> Result<()> { /* ... */ }

pub async fn insert_equity_trade(
    pool: &DbPool,
    model_id: &str,
    symbol: &str,
    // ...
) -> Result<()> { /* ... */ }
```

Then update every call site (`scheduler.rs`, `exec/paper.rs`, etc.). If a constructor doesn't yet receive `model_id`, pass a synthetic default (e.g. `"legacy"`) and add a new constructor that accepts the real value.

## Per-model position lookup

Replace the singleton `signal_state` read with a per-model lookup, falling back to the legacy singleton so tests and old data keep working:

```rust
pub async fn load_position(pool: &DbPool, model_id: &str, symbol: &str) -> Result<i64> {
    // Per-model: latest position event for this model/symbol.
    let row = sqlx::query(
        "SELECT position FROM positions WHERE model_id = ?1 AND symbol = ?2 ORDER BY candle_ts DESC LIMIT 1"
    )
    .bind(model_id)
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .context("load_position")?;

    if let Some(r) = row {
        return Ok(r.get(0));
    }

    // Legacy fallback for pre-migration rows / tests.
    match sqlx::query("SELECT position FROM signal_state WHERE id = 1")
        .fetch_one(pool)
        .await
    {
        Ok(row) => Ok(row.get(0)),
        Err(sqlx::Error::RowNotFound) => Ok(0),
        Err(e) => Err(e).context("load_position fallback"),
    }
}
```

## Handler: accept `symbol` or `model_id` query param

Use `axum::extract::Query` with a struct or a `HashMap<String, String>`.

```rust
use axum::extract::{State, Query};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub(crate) struct StatusQuery {
    model_id: Option<String>,
    symbol: Option<String>,
}

pub(crate) async fn handle_status(
    State(state): State<AppState>,
    Query(query): Query<StatusQuery>,
) -> ApiResult<StatusResponse> {
    let symbol = query.symbol.unwrap_or_else(|| state.symbol.clone());
    let short_symbol = state.short_symbol.clone(); // or resolve from model registry
    // ... use symbol/short_symbol instead of state.symbol/state.short_symbol
}
```

For handlers that already take `HashMap<String, String>`:

```rust
pub(crate) async fn handle_chart(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> ApiResult<ChartResponse> {
    let symbol = params.get("symbol").cloned().unwrap_or_else(|| state.symbol.clone());
    // ...
}
```

## Update tests

After adding `Query<T>` to a handler, every test call must provide it:

```rust
let Json(status) = status::handle_status(
    test_state(pool),
    axum::extract::Query(status::StatusQuery {
        model_id: None,
        symbol: None,
    }),
)
.await
.unwrap();
```

For `HashMap`-based handlers:

```rust
let result = predictions::handle_accuracy(
    test_state(pool),
    axum::extract::Query(std::collections::HashMap::new()),
)
.await;
```

## Build/test discipline

1. `cargo build --bin <target>` after every file patch to catch signature mismatches early.
2. `cargo test --lib` to verify test updates.
3. Expect pre-existing config test failures to remain unchanged; the goal is no new regressions.
