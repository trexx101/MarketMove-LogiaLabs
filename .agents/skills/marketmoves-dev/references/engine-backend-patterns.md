# Engine Backend Patterns (API endpoints, DB migrations, tests)

Captured 2026-08-19. Commits: events `68ab737`, options endpoints `827782d`, heartbeat `c36724f`, bin fix `d710952`.

## Two CRITICAL traps (both bit us live)

### 1. DDL is split on `;` — never put a semicolon in a SQL comment

`db::open` and every test pool run `DDL.split(';').map(str::trim)` and execute
each fragment. A `;` inside a `-- comment` splits the statement mid-comment
and breaks EVERYTHING downstream (all tables after that point silently fail
to create in tests; startup corrupts the same way).

Wrong: `last_heartbeat_ts INTEGER -- ms epoch; NULL = never beat`
Right: `last_heartbeat_ts INTEGER -- ms epoch (NULL = never beat)`

### 2. lib and bin are SEPARATE crate trees — `mod` declarations must exist in BOTH

`engine/src/lib.rs` and `engine/src/main.rs` each declare their own `mod`
list. `cargo test --lib` compiles ONLY the lib tree. If you add a module to
lib.rs (or reference `crate::options` from api/mod.rs which is shared), you
MUST also declare it in main.rs or `cargo build --bin engine` fails with
`cannot find X in crate`. After ANY change to module declarations, verify:

    cargo build --bin engine && cargo build --bin options_recorder

(The `options_recorder` bin has its own tree too.)

## DB migration contract

`CREATE TABLE IF NOT EXISTS` never alters a pre-existing table. For a new
column on a live table (e.g. `last_heartbeat_ts` on `option_tape_meta`):

1. Update the DDL string (for fresh installs).
2. Write an idempotent `migrate_*` fn using `pragma_table_info` probe:
   `SELECT COUNT(*) FROM pragma_table_info('T') WHERE name = 'col'` → ALTER if 0.
3. Register it in `db::open` alongside `migrate_predictions` etc.
4. Test the legacy-table path explicitly (create table WITHOUT the column,
   run migration twice, then exercise the column).

Live DB is `data/candles.db` (sqlite://). Verify migrations by booting once:
`timeout 15 cargo run -q --bin engine` and inspect with sqlite3.

## Endpoint test pattern (axum, no tower needed)

Do NOT use `tower::ServiceExt::oneshot` — it isn't a dev-dependency and the
response types don't implement Deserialize anyway. Call handlers directly:

```rust
use axum::extract::State;
fn test_state(pool: db::DbPool) -> State<AppState> {
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    State(AppState {
        pool,
        trading_mode: std::sync::Arc::new(tokio::sync::RwLock::new(crate::config::TradingMode::Paper)),
        strategy_params: std::sync::Arc::new(tokio::sync::RwLock::new(crate::strategy::EquityStrategyParams::default())),
        symbol: "QQQ".into(), tx,
        parity_marker_path: String::new(), parity_max_age_secs: 300,
        totp_secret: String::new(), zmq_endpoint: String::new(), norm_stats_path: String::new(),
    })
}
// Test pool: full DDL (for in-memory schema) or minimal CREATE TABLE set.
// Query params: axum::extract::Query(std::collections::HashMap::new())
// Call: handle_list_positions(state, axum::extract::Query(q)).await
```

Full-DDL test pool: iterate `db::DDL.split(';')` + `db::migrate_option_positions`.
Minimal pool (when DDL is too heavy): create only the tables the handler touches
(e.g. engine_events, pending_promotions) — remember to include `engine_events`
if the code path calls `insert_event`, else events silently fail (logged, not fatal).

## Event publishing conventions (7.0f)

- All lifecycle events go through `db::insert_event` — no new event plumbing.
- Categories are a closed set from the DDL comment:
  `trade | data | system | strategy | alert | advisor`. Do NOT invent new
  category strings (e.g. "hyperopt" is wrong — use `strategy` with
  source `hyperopt::promotion`).
- ENTRY_INITIATED → trade/info; PROMOTION outcome → strategy (info if
  promoted, warn if denied); SKIPPED_ENTRY → strategy (already wired via
  `skipped_entry()` helper).
- Event inserts are fire-and-log: `if let Err(e) = ... { error!(...) }` —
  never fail the trade path on event persistence.
- `GET /api/events?category=&mode=&severity=&equity=&since=&limit=`
  (limit clamped 1..1000, default 100). Searchable by category and mode per
  Events-tab requirement.

## Options API surface (7.0c)

- `GET /api/options/positions` — underlying/status/limit filters (db::list_option_positions).
- `GET /api/options/trades` — CLOSED positions only; a position row IS the
  linked lifecycle (entry_premium at open, realized_pnl/closed_at at close).
- `GET/PUT /api/options/config` — registry-backed; PUT is a flattened
  `{key: value}` map, unknown keys → rejected list, all-rejected → 400.
  Mode comes from `state.trading_mode.read().await.to_string()`.
- `GET /api/hyperopt/runs`, `GET /api/options/tape/status` (heartbeat_age_secs,
  healthy/stale/never_beat counts; stale > 120s; NULL heartbeat = never beat = stale).
- Struct gotcha: the store is `OptionsConfigStore` (not `ConfigStore`).
- Response structs that get `.unwrap()`ed in tests need `#[derive(Debug)]`.

## Known-baseline test failures (NOT regressions)

4 tests fail on every run due to `.env` leakage in the config tests:
`config::tests::defaults_load_when_env_unset`, `config::tests::live_mode_falls_back_to_paper`,
`config::tests::shorting_default_is_disabled`, `api::tests::accuracy_returns_503_when_no_resolved`.
As of 2026-08-19 the suite baseline is 351 passed / 4 failed.
