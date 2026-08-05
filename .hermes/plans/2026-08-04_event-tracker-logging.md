# Event Tracker & Logging Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Add a unified event log to the MarketMoves engine — a queryable, filterable history of trades, data fetches, strategy changes, mode switches, and alerts — with a dedicated Events page in the frontend and automatic archival.

**Architecture:** Append-only SQLite table (`engine_events`) for active events, a thin `EventLogger` struct that writes to DB + broadcasts on the existing WS channel, a REST endpoint for historical queries, and a daily archival task that exports old rows to compressed JSON files.

**Tech Stack:** Rust (Axum, sqlx, tokio), SQLite, Svelte 4, existing WebSocket infrastructure.

---

## Constraints & Decisions

1. **Retention:** 60 days in the active table (configurable via `EVENTS_RETENTION_DAYS` env var).
2. **Data-fetch events:** Only on failure or error — successful fetches are routine and stay as `tracing` only.
3. **Archive:** Compressed JSON files stored at `/app/data/events_archive/YYYY-MM.json.gz`, downloadable via future API endpoint.
4. **Paper vs Live:** Every event row carries the `mode` at emission time. Frontend renders a badge.
5. **No new container, no new volume.** Events live in the existing SQLite DB (`/app/data/candles.db`).

---

## Event Categories

| Category   | Examples |
|------------|----------|
| `trade`    | TradeFill (buy/sell), position open/close |
| `data`     | Yahoo/FRED/CBOE/Moomoo fetch (success + failure counts) |
| `system`   | Engine start, mode change, scheduler cycle errors |
| `strategy` | Config change, prediction persisted, backtest completed |
| `alert`    | Staleness alert, ZMQ reconnect, inference timeout |
| `advisor`  | Briefing generated, chat query |

---

## Task Breakdown

### Task 1: Add `engine_events` table to DDL

**Objective:** Create the append-only event log table.

**Files:**
- Modify: `engine/src/db.rs:9-197` (DDL constant)

**Step 1: Append table definition to DDL**

```sql
-- Append after the existing backtest_results table definition:

CREATE TABLE IF NOT EXISTS engine_events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts           INTEGER NOT NULL,
    category     TEXT    NOT NULL,  -- trade | data | system | strategy | alert | advisor
    severity     TEXT    NOT NULL,  -- info | warn | error
    mode         TEXT    NOT NULL,  -- paper | live
    source       TEXT    NOT NULL,  -- scheduler, data::yahoo, exec::paper, api::mode, etc.
    message      TEXT    NOT NULL,
    payload_json TEXT    NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS engine_events_ts_idx ON engine_events (ts DESC);
CREATE INDEX IF NOT EXISTS engine_events_category_ts_idx ON engine_events (category, ts DESC);
```

**Step 2: Run tests**

```bash
cd engine && cargo test db::tests --no-fail-fast
```

Expected: All existing DB tests pass (DDL is append-only).

---

### Task 2: Define `EngineEvent` enum and `EventLogger` struct

**Objective:** Create the domain types for event emission.

**Files:**
- Create: `engine/src/event.rs`

**Step 1: Write the event module**

```rust
use chrono::Utc;
use serde::Serialize;
use serde_json::Value as JsonValue;
use sqlx::SqlitePool;
use tracing::error;

use crate::api::ws::{TelemetryEvent, TelemetrySender};
use crate::config::TradingMode;
use crate::db::DbPool;

#[derive(Debug, Clone, Copy, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum EventCategory {
    Trade,
    Data,
    System,
    Strategy,
    Alert,
    Advisor,
}

#[derive(Debug, Clone, Copy, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum EventSeverity {
    Info,
    Warn,
    Error,
}

/// An event to be persisted and broadcast.
pub struct EngineEvent {
    pub category: EventCategory,
    pub severity: EventSeverity,
    pub source: &'static str,
    pub message: String,
    pub payload: JsonValue,
}

impl EngineEvent {
    pub fn trade_fill(
        side: &str,
        symbol: &str,
        qty: f64,
        price: f64,
        fee: f64,
        pnl: f64,
    ) -> Self {
        Self {
            category: EventCategory::Trade,
            severity: EventSeverity::Info,
            source: "exec::paper",
            message: format!("{side} {symbol} @ {price:.2} (qty={qty:.2})"),
            payload: serde_json::json!({
                "side": side,
                "symbol": symbol,
                "qty": qty,
                "price": price,
                "fee": fee,
                "realized_pnl": pnl,
            }),
        }
    }

    pub fn data_fetch_failed(source: &'static str, symbol: &str, error: &str) -> Self {
        Self {
            category: EventCategory::Data,
            severity: EventSeverity::Error,
            source,
            message: format!("fetch failed for {symbol}: {error}"),
            payload: serde_json::json!({ "symbol": symbol, "error": error }),
        }
    }

    pub fn mode_changed(from: TradingMode, to: TradingMode, authorized_by: &str) -> Self {
        Self {
            category: EventCategory::System,
            severity: EventSeverity::Info,
            source: "api::mode",
            message: format!("mode changed: {from} → {to}"),
            payload: serde_json::json!({ "from": format!("{from}"), "to": format!("{to}"), "authorized_by": authorized_by }),
        }
    }

    pub fn strategy_config_changed(old: &crate::strategy::EquityStrategyParams, new: &crate::strategy::EquityStrategyParams) -> Self {
        Self {
            category: EventCategory::Strategy,
            severity: EventSeverity::Info,
            source: "api::strategy_config",
            message: "strategy config updated".to_string(),
            payload: serde_json::json!({ "old": old, "new": new }),
        }
    }

    pub fn prediction_persisted(pred_1d: f64, pred_5d: f64, pred_21d: f64, regime: &str) -> Self {
        Self {
            category: EventCategory::Strategy,
            severity: EventSeverity::Info,
            source: "scheduler",
            message: format!("prediction persisted: 1d={pred_1d:.4}, 5d={pred_5d:.4}, 21d={pred_21d:.4}, regime={regime}"),
            payload: serde_json::json!({ "pred_1d": pred_1d, "pred_5d": pred_5d, "pred_21d": pred_21d, "regime": regime }),
        }
    }

    pub fn staleness_alert(last_ts: Option<i64>, secs: i64) -> Self {
        Self {
            category: EventCategory::Alert,
            severity: EventSeverity::Warn,
            source: "scheduler",
            message: format!("staleness alert: {}s since last candle", secs),
            payload: serde_json::json!({ "last_candle_ts": last_ts, "seconds_since_last": secs }),
        }
    }

    pub fn advisor_briefing(for_date: &str, model: &str, latency_ms: u64) -> Self {
        Self {
            category: EventCategory::Advisor,
            severity: EventSeverity::Info,
            source: "advisor",
            message: format!("briefing generated for {for_date} via {model}"),
            payload: serde_json::json!({ "for_date": for_date, "model": model, "latency_ms": latency_ms }),
        }
    }

    pub fn engine_started(mode: TradingMode, symbol: &str) -> Self {
        Self {
            category: EventCategory::System,
            severity: EventSeverity::Info,
            source: "main",
            message: format!("engine started in {mode} mode for {symbol}"),
            payload: serde_json::json!({ "mode": format!("{mode}"), "symbol": symbol }),
        }
    }

    pub fn backtest_completed(strategy_id: &str, cagr: f64, sharpe: f64) -> Self {
        Self {
            category: EventCategory::Strategy,
            severity: EventSeverity::Info,
            source: "strategy_lab",
            message: format!("backtest completed for {strategy_id}: CAGR={cagr:.1f}%, Sharpe={sharpe:.2f}"),
            payload: serde_json::json!({ "strategy_id": strategy_id, "cagr": cagr, "sharpe": sharpe }),
        }
    }
}

/// Handles persistence + broadcast of engine events.
pub struct EventLogger {
    pool: DbPool,
    tx: Option<TelemetrySender>,
    mode: std::sync::Arc<tokio::sync::RwLock<TradingMode>>,
}

impl EventLogger {
    pub fn new(pool: DbPool, tx: Option<TelemetrySender>, mode: std::sync::Arc<tokio::sync::RwLock<TradingMode>>) -> Self {
        Self { pool, tx, mode }
    }

    /// Persist the event to `engine_events` and broadcast on the telemetry channel.
    pub async fn emit(&self, event: EngineEvent) {
        let mode = *self.mode.read().await;
        let mode_str = match mode {
            TradingMode::Paper => "paper",
            TradingMode::Live => "live",
        };
        let ts = Utc::now().timestamp();
        let category = event.category.to_string();
        let severity = event.severity.to_string();
        let payload = event.payload.to_string();

        // Persist
        let pool = self.pool.clone();
        let res = sqlx::query(
            r#"INSERT INTO engine_events (ts, category, severity, mode, source, message, payload_json)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
        )
        .bind(ts)
        .bind(&category)
        .bind(&severity)
        .bind(mode_str)
        .bind(event.source)
        .bind(&event.message)
        .bind(&payload)
        .execute(&pool)
        .await;

        if let Err(e) = res {
            error!(error = %e, "failed to persist engine event");
        }

        // Broadcast
        if let Some(tx) = &self.tx {
            let _ = tx.send(TelemetryEvent::EngineEvent {
                ts,
                category,
                severity,
                mode: mode_str.to_string(),
                source: event.source.to_string(),
                message: event.message.clone(),
                payload: event.payload.clone(),
            });
        }
    }
}
```

**Step 2: Add `strum` to workspace dependencies**

In `Cargo.toml` under `[workspace.dependencies]`:

```toml
strum = { version = "0.26", features = ["derive"] }
```

In `engine/Cargo.toml`:

```toml
strum = { workspace = true }
```

**Step 3: Add `mod event;` to `engine/src/main.rs`**

At line ~10 with other module declarations:

```rust
mod event;
```

**Step 4: Run tests**

```bash
cd engine && cargo check
```

Expected: Compiles without errors.

---

### Task 3: Add `EngineEvent` variant to `TelemetryEvent`

**Objective:** Extend the existing telemetry enum to carry the unified event.

**Files:**
- Modify: `engine/src/api/ws.rs:27-86`

**Step 1: Add the new variant**

After `AdvisorBriefing` (line ~85), add:

```rust
    /// Unified engine event for the Events page (Phase 4).
    EngineEvent {
        ts: i64,
        category: String,
        severity: String,
        mode: String,
        source: String,
        message: String,
        payload: serde_json::Value,
    },
```

**Step 2: Run tests**

```bash
cd engine && cargo test api::ws::tests --no-fail-fast
```

Expected: All existing WS serialization tests pass.

---

### Task 4: Wire `EventLogger` into `AppState` and `main.rs`

**Objective:** Create and share the logger instance.

**Files:**
- Modify: `engine/src/api/mod.rs:18-42`
- Modify: `engine/src/main.rs:60-140`

**Step 1: Add `event_logger` field to `AppState`**

In `engine/src/api/mod.rs` after line ~29:

```rust
    /// Unified event logger for trades, data fetches, system events.
    pub event_logger: std::sync::Arc<crate::event::EventLogger>,
```

**Step 2: Construct `EventLogger` in `main.rs`**

After the broadcast channel creation (around line 88), add:

```rust
    // Event logger — wraps DB + telemetry sender + mode ref.
    let event_logger = std::sync::Arc::new(crate::event::EventLogger::new(
        pool.clone(),
        tx.clone(),
        trading_mode.clone(),
    ));
```

**Step 3: Pass `event_logger` to `AppState` in `router()`**

Modify `engine/src/api/mod.rs:router()` to accept and wire the logger:

```rust
pub fn router(
    pool: db::DbPool,
    config: &Config,
    tx: ws::TelemetrySender,
    advisor: Option<std::sync::Arc<crate::advisor::AdvisorState>>,
    event_logger: std::sync::Arc<crate::event::EventLogger>,  // ADD THIS
) -> Router {
    // ... existing state construction ...
    let state = AppState {
        // ... existing fields ...
        event_logger,  // ADD THIS
    };
    // ... rest unchanged ...
}
```

Update `main.rs` call site:

```rust
let app = api::router(pool.clone(), &cfg, tx.clone(), advisor, event_logger);
```

**Step 4: Run tests**

```bash
cd engine && cargo check
```

Expected: Compiles.

---

### Task 5: Add event emission at key action points

**Objective:** Replace scattered `info!()` calls with structured events.

**Files:**
- Modify: `engine/src/data/mod.rs:37-88` (data fetches)
- Modify: `engine/src/scheduler.rs:96-228` (prediction + strategy)
- Modify: `engine/src/exec/paper.rs:67-165` (trades)
- Modify: `engine/src/api/mode.rs` (mode change)
- Modify: `engine/src/api/strategy_config.rs` (config change)
- Modify: `engine/src/main.rs` (engine start)

**Step 1: Emit event in `data::mod.rs` on fetch failure only**

After each failed fetch (Moomoo, Yahoo fallback, CBOE, FRED), add:

```rust
// Example after Moomoo failure:
event_logger.emit(EngineEvent::data_fetch_failed("data::moomoo", s, &e.to_string())).await;

// Example after Yahoo fallback also fails:
event_logger.emit(EngineEvent::data_fetch_failed("data::yahoo", s, &e2.to_string())).await;
```

Successful fetches stay as `tracing::info!()` — they are routine and would create noise. Only failures become events.

This requires passing the `EventLogger` reference into `backfill_equities`. Change the signature:

```rust
pub async fn backfill_equities(pool: &DbPool, stale_threshold_secs: i64, event_logger: &crate::event::EventLogger) -> Result<()> {
```

Update call sites in `main.rs` and the ingestion supervisor.

**Step 2: Emit event on trade execution in `exec/paper.rs`**

In `set_target_position`, after each `db::insert_equity_trade`:

```rust
// Exit leg:
self.event_logger.emit(EngineEvent::trade_fill(
    "sell", exit_symbol, self.qty, close, fee, pnl
)).await;

// Entry leg:
self.event_logger.emit(EngineEvent::trade_fill(
    "buy", entry_symbol, self.qty, close, fee, 0.0
)).await;
```

This requires adding `event_logger: Arc<EventLogger>` to `PaperExecutor` struct and constructor.

**Step 3: Emit event on prediction persist in `scheduler.rs`**

After `db::insert_equity_prediction` succeeds:

```rust
if let Some(logger) = &self.event_logger {
    logger.emit(EngineEvent::prediction_persisted(
        pred.pred_1d, pred.pred_5d, pred.pred_21d, regime
    )).await;
}
```

Add `event_logger: Option<Arc<EventLogger>>` to `EquityScheduler` struct and constructor.

**Step 4: Emit event on mode change in `api/mode.rs`**

In `handle_set_mode`, after the mode is successfully flipped:

```rust
state.event_logger.emit(EngineEvent::mode_changed(
    prev_mode, new_mode, "totp"
)).await;
```

**Step 5: Emit event on strategy config change in `api/strategy_config.rs`**

In `handle_put`, after the update:

```rust
state.event_logger.emit(EngineEvent::strategy_config_changed(&old, &params)).await;
```

**Step 6: Emit engine start event in `main.rs`**

After the scheduler is spawned, before the server starts:

```rust
event_logger.emit(EngineEvent::engine_started(cfg.trading_mode, &cfg.symbol)).await;
```

**Step 7: Run tests**

```bash
cd engine && cargo test --no-fail-fast
```

Expected: All tests pass. Some tests will need `EventLogger` stubs added (pass `None` or a test logger).

---

### Task 6: Add `GET /api/events` endpoint

**Objective:** Historical event query surface.

**Files:**
- Create: `engine/src/api/events.rs`
- Modify: `engine/src/api/mod.rs`

**Step 1: Create `events.rs`**

```rust
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::db::DbPool;

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    #[serde(default = "default_limit")]
    pub limit: u32,
    pub category: Option<String>,
    pub since: Option<i64>,
    pub mode: Option<String>,
}

fn default_limit() -> u32 { 100 }

#[derive(Debug, Serialize)]
pub struct EventRow {
    pub id: i64,
    pub ts: i64,
    pub category: String,
    pub severity: String,
    pub mode: String,
    pub source: String,
    pub message: String,
    pub payload: serde_json::Value,
}

pub async fn handle_events(
    State(state): State<crate::api::AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<Vec<EventRow>>, (StatusCode, String)> {
    let limit = query.limit.min(500) as i64;
    let mut sql = String::from(
        "SELECT id, ts, category, severity, mode, source, message, payload_json FROM engine_events WHERE 1=1"
    );
    let mut binds: Vec<String> = Vec::new();

    if let Some(cat) = &query.category {
        sql.push_str(" AND category = ?");
        binds.push(cat.clone());
    }
    if let Some(since) = query.since {
        sql.push_str(" AND ts >= ?");
        binds.push(since.to_string());
    }
    if let Some(mode) = &query.mode {
        sql.push_str(" AND mode = ?");
        binds.push(mode.clone());
    }
    sql.push_str(" ORDER BY ts DESC LIMIT ?");
    binds.push(limit.to_string());

    let mut q = sqlx::query(&sql);
    for b in &binds {
        q = q.bind(b);
    }

    let rows = q.fetch_all(&state.pool).await
        .map_err(|e| crate::api::internal_error("events query", e))?;

    let events: Vec<EventRow> = rows.iter().map(|r| EventRow {
        id: r.get("id"),
        ts: r.get("ts"),
        category: r.get("category"),
        severity: r.get("severity"),
        mode: r.get("mode"),
        source: r.get("source"),
        message: r.get("message"),
        payload: serde_json::from_str(r.get("payload_json")).unwrap_or(serde_json::json!({})),
    }).collect();

    Ok(Json(events))
}
```

**Step 2: Register route in `mod.rs`**

Add to router:

```rust
.route("/api/events", get(events::handle_events))
```

Add module:

```rust
mod events;
```

**Step 3: Run tests**

```bash
cd engine && cargo test api::tests --no-fail-fast
```

Expected: Tests pass.

---

### Task 7: Add daily archival task

**Objective:** Export events older than retention to compressed JSON.

**Files:**
- Modify: `engine/src/scheduler.rs` (or add new module `engine/src/archive.rs`)

**Step 1: Add `flate2` dependency**

In `Cargo.toml`:

```toml
flate2 = { version = "1", features = ["gzip"] }
```

In `engine/Cargo.toml`:

```toml
flate2 = { workspace = true }
```

**Step 2: Implement archival function**

In `engine/src/scheduler.rs` or a new `archive.rs`:

```rust
use std::path::PathBuf;
use std::io::Write;
use flate2::write::GzEncoder;
use flate2::Compression;

pub async fn archive_old_events(pool: &DbPool, retention_days: i64) -> anyhow::Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - retention_days * 86_400;
    
    // Fetch old events
    let rows = sqlx::query(
        "SELECT id, ts, category, severity, mode, source, message, payload_json 
         FROM engine_events WHERE ts < ?1 ORDER BY ts ASC"
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(0);
    }

    // Group by month
    let mut months: std::collections::BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for r in &rows {
        let ts: i64 = r.get("ts");
        let dt = chrono::DateTime::from_timestamp(ts, 0).unwrap_or_else(|| chrono::Utc::now());
        let month_key = dt.format("%Y-%m").to_string();
        
        let entry = serde_json::json!({
            "id": r.get::<i64, _>("id"),
            "ts": ts,
            "category": r.get::<String, _>("category"),
            "severity": r.get::<String, _>("severity"),
            "mode": r.get::<String, _>("mode"),
            "source": r.get::<String, _>("source"),
            "message": r.get::<String, _>("message"),
            "payload": r.get::<String, _>("payload_json"),
        });
        months.entry(month_key).or_default().push(entry);
    }

    // Write each month to a gzipped JSON file
    let archive_dir = PathBuf::from("/app/data/events_archive");
    std::fs::create_dir_all(&archive_dir)?;

    for (month, events) in months {
        let path = archive_dir.join(format!("{month}.json.gz"));
        let file = std::fs::File::create(&path)?;
        let mut enc = GzEncoder::new(file, Compression::default());
        let json = serde_json::to_string(&events)?;
        enc.write_all(json.as_bytes())?;
        enc.finish()?;
    }

    // Delete archived rows from DB
    let deleted = sqlx::query("DELETE FROM engine_events WHERE ts < ?1")
        .bind(cutoff)
        .execute(pool)
        .await?
        .rows_affected() as usize;

    Ok(deleted)
}
```

**Step 3: Schedule archival in the ingestion supervisor**

In `data/mod.rs:run_equities_ingestion`, add a third spawn that runs once per day:

```rust
// --- 3. Daily event archival (keep retention_days of active events) ---
let archive_pool = pool.clone();
let retention_days = std::env::var("EVENTS_RETENTION_DAYS")
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(60);
tokio::spawn(async move {
    let start = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
    let mut interval = tokio::time::interval_at(start, std::time::Duration::from_secs(24 * 3600));
    loop {
        interval.tick().await;
        match archive_old_events(&archive_pool, retention_days).await {
            Ok(n) if n > 0 => info!(archived = n, "event archival complete"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "event archival failed"),
        }
    }
});
```

**Step 4: Run tests**

```bash
cd engine && cargo test --no-fail-fast
```

Expected: Tests pass.

---

### Task 8: Add `GET /api/events/archive` endpoint (list available archives)

**Objective:** Surface archive files for future download.

**Files:**
- Modify: `engine/src/api/events.rs`

**Step 1: Add endpoint**

```rust
#[derive(Debug, Serialize)]
pub struct ArchiveInfo {
    pub filename: String,
    pub size_bytes: u64,
}

pub async fn handle_archives(
    State(_state): State<crate::api::AppState>,
) -> Result<Json<Vec<ArchiveInfo>>, (StatusCode, String)> {
    let archive_dir = std::path::Path::new("/app/data/events_archive");
    if !archive_dir.exists() {
        return Ok(Json(Vec::new()));
    }

    let mut archives = Vec::new();
    for entry in std::fs::read_dir(archive_dir).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("archive dir read: {e}"))
    })? {
        let entry = entry.map_err(|e| {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("archive entry: {e}"))
        })?;
        let path = entry.path();
        if path.extension().map(|e| e == "gz").unwrap_or(false) {
            let filename = path.file_name().unwrap().to_string_lossy().to_string();
            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            archives.push(ArchiveInfo { filename, size_bytes });
        }
    }

    archives.sort_by(|a, b| b.filename.cmp(&a.filename)); // newest first
    Ok(Json(archives))
}
```

**Step 2: Register route**

```rust
.route("/api/events/archive", get(events::handle_archives))
```

---

### Task 9: Add `EVENTS_RETENTION_DAYS` to config

**Objective:** Make retention configurable.

**Files:**
- Modify: `engine/src/config.rs`
- Modify: `deploy/docker-compose.yml`

**Step 1: Add field to `Config`**

```rust
pub struct Config {
    // ... existing fields ...
    /// Event log retention in days. Events older than this are archived.
    pub events_retention_days: i64,
}
```

**Step 2: Load from env in `Config::from_env`**

```rust
events_retention_days: std::env::var("EVENTS_RETENTION_DAYS")
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(60),
```

**Step 3: Add to `docker-compose.yml` environment**

```yaml
EVENTS_RETENTION_DAYS: ${EVENTS_RETENTION_DAYS:-60}
```

---

### Task 10: Frontend — add `EngineEvent` handling in WS

**Objective:** Route new event type to a store.

**Files:**
- Modify: `frontend/src/lib/stores.js`
- Modify: `frontend/src/lib/websocket.js`

**Step 1: Add `events` store**

```js
/** Unified event log — prepended from WS, capped at 100 */
export const events = writable([]);
```

**Step 2: Handle `EngineEvent` in `handleMessage`**

Add case:

```js
case 'EngineEvent':
  events.update((list) => {
    const entry = {
      id: msg.id || null,
      ts: msg.ts,
      category: msg.category,
      severity: msg.severity,
      mode: msg.mode,
      source: msg.source,
      message: msg.message,
      payload: msg.payload,
    };
    return [entry, ...list].slice(0, 100);
  });
  break;
```

---

### Task 11: Frontend — add `fetchEvents` API function

**Objective:** Load historical events on page mount.

**Files:**
- Modify: `frontend/src/lib/api.js`

**Step 1: Add function**

```js
/**
 * Fetch historical engine events.
 * @param {number} [limit=100]
 * @param {string} [category] optional filter
 * @param {number} [since] timestamp filter
 * @param {string} [mode] 'paper' | 'live' filter
 * @returns {Promise<object[]>}
 */
export async function fetchEvents(limit = 100, category = null, since = null, mode = null) {
  let url = `${API_BASE}/events?limit=${limit}`;
  if (category) url += `&category=${encodeURIComponent(category)}`;
  if (since) url += `&since=${since}`;
  if (mode) url += `&mode=${mode}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`events: ${res.status}`);
  return res.json();
}
```

---

### Task 12: Frontend — create `Events.svelte` view

**Objective:** Dedicated Events page.

**Files:**
- Create: `frontend/src/views/Events.svelte`

**Step 1: Create the component**

```svelte
<script>
  import { onMount } from 'svelte';
  import { events } from '../lib/stores.js';
  import { connectWebSocket, disconnectWebSocket } from '../lib/websocket.js';
  import { fetchEvents } from '../lib/api.js';

  let loaded = false;
  let filterCategory = '';
  let filterMode = '';

  const categories = ['', 'trade', 'data', 'system', 'strategy', 'alert', 'advisor'];
  const modes = ['', 'paper', 'live'];

  async function load() {
    try {
      const hist = await fetchEvents(100, filterCategory || null, null, filterMode || null);
      events.set(hist);
      loaded = true;
    } catch (e) {
      console.error('Failed to load events:', e);
    }
  }

  onMount(() => {
    load();
    connectWebSocket();
    return () => disconnectWebSocket();
  });

  $: if (filterCategory || filterMode) load();

  function formatTs(ts) {
    if (!ts) return '';
    const d = new Date(ts * 1000);
    return d.toLocaleString();
  }

  function severityColor(sev) {
    if (sev === 'error') return 'color: #ff6b6b';
    if (sev === 'warn') return 'color: #ffd93d';
    return '';
  }

  function categoryIcon(cat) {
    const icons = {
      trade: '💰',
      data: '📊',
      system: '⚙️',
      strategy: '📈',
      alert: '⚠️',
      advisor: '🤖',
    };
    return icons[cat] || '📌';
  }
</script>

<div class="events-page">
  <div class="header">
    <h1>Events</h1>
    <div class="filters">
      <label>Category:
        <select bind:value={filterCategory}>
          {#each categories as cat}
            <option value={cat}>{cat || 'all'}</option>
          {/each}
        </select>
      </label>
      <label>Mode:
        <select bind:value={filterMode}>
          {#each modes as m}
            <option value={m}>{m || 'all'}</option>
          {/each}
        </select>
      </label>
      <button on:click={load}>Refresh</button>
    </div>
  </div>

  {#if !loaded}
    <p class="loading">Loading events...</p>
  {:else}
    <div class="event-list">
      {#each $events as ev}
        <div class="event-row" class:alert={ev.severity === 'error' || ev.severity === 'warn'}>
          <span class="ts">{formatTs(ev.ts)}</span>
          <span class="icon">{categoryIcon(ev.category)}</span>
          <span class="category">{ev.category}</span>
          <span class="severity" style={severityColor(ev.severity)}>{ev.severity}</span>
          <span class="mode badge {ev.mode}">{ev.mode}</span>
          <span class="message">{ev.message}</span>
          {#if ev.payload && Object.keys(ev.payload).length > 0}
            <details class="payload">
              <summary>payload</summary>
              <pre>{JSON.stringify(ev.payload, null, 2)}</pre>
            </details>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .events-page {
    padding: 1rem;
  }
  .header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1rem;
  }
  .filters {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }
  .filters label {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }
  .event-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .event-row {
    display: grid;
    grid-template-columns: 160px 24px 70px 50px 60px 1fr;
    align-items: start;
    gap: 0.5rem;
    padding: 0.5rem;
    background: #1e1e2e;
    border-radius: 4px;
  }
  .event-row.alert {
    background: #2e1e1e;
  }
  .ts {
    color: #6c7086;
    font-size: 0.85rem;
  }
  .category {
    font-weight: 500;
  }
  .mode.badge {
    padding: 2px 6px;
    border-radius: 3px;
    font-size: 0.75rem;
    text-transform: uppercase;
  }
  .mode.badge.paper {
    background: #1e3a5f;
    color: #89b4fa;
  }
  .mode.badge.live {
    background: #3a1e1e;
    color: #f38ba8;
  }
  .payload {
    grid-column: 1 / -1;
    margin-top: 0.25rem;
  }
  .payload pre {
    font-size: 0.8rem;
    background: #11111b;
    padding: 0.5rem;
    border-radius: 4px;
    overflow-x: auto;
  }
</style>
```

---

### Task 13: Frontend — add Events to navigation

**Objective:** Make Events a 5th view.

**Files:**
- Modify: `frontend/src/App.svelte`

**Step 1: Import Events**

```svelte
import Events from './views/Events.svelte';
```

**Step 2: Add nav button**

After the Advisor nav button (~line 70):

```svelte
<button class="nav-btn" class:active={currentView === 'events'} on:click={() => nav('events')}>
  <svg ...>...</svg>
  Events
</button>
```

**Step 3: Add view switch**

In the view section:

```svelte
{#if currentView === 'events'}
  <Events />
{/if}
```

---

### Task 14: Frontend — build and verify

**Objective:** Ensure the frontend compiles and renders.

**Files:**
- Build output: `frontend/dist/`

**Step 1: Build frontend**

```bash
cd frontend && npm run build
```

Expected: Build succeeds, new `index-*.js` bundle created.

**Step 2: Rebuild engine Docker image**

```bash
docker compose -f deploy/docker-compose.yml build engine
```

**Step 3: Restart engine**

```bash
docker compose -f deploy/docker-compose.yml stop engine && \
docker compose -f deploy/docker-compose.yml rm -f engine && \
docker compose -f deploy/docker-compose.yml up -d engine
```

**Step 4: Smoke test**

- Navigate to dashboard, click Events nav item
- Verify events page loads
- Trigger a data fetch or config change
- Verify event appears in real-time

---

## Verification Checklist

- [ ] `engine_events` table created (DDL migration)
- [ ] `EventLogger` writes to DB on each event
- [ ] WS broadcasts `EngineEvent` variant
- [ ] `GET /api/events` returns historical events
- [ ] Daily archival task runs and exports gzipped JSON
- [ ] Frontend Events page loads and shows events
- [ ] Real-time updates appear via WS
- [ ] Paper/Live badge renders correctly
- [ ] Category filter works
- [ ] Retention env var respected

---

## Risks & Open Questions

1. **DB write load:** Events are low-frequency (10-50/day). SQLite can handle thousands/sec. Negligible risk.
2. **WS message size:** `EngineEvent` payload is small (< 500 bytes typically). No concern.
3. **Archive download:** This plan lists archives but does not implement a download endpoint. Add in future phase if needed.
4. **Test coverage:** Some tests will need `EventLogger` stubs. Add a test helper that creates a logger with a test pool.

---

## Dependencies

- `strum` for enum string serialization
- `flate2` for gzip compression (archival)
- No external services

---

## Estimated Effort

- Backend (Tasks 1-9): 3-4 hours
- Frontend (Tasks 10-14): 2-3 hours
- Testing + verification: 1 hour
- **Total: 6-8 hours** (can be split across multiple sessions)