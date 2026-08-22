# Axum WebSocket Broadcast Channel — Wiring Pattern

Adding a live telemetry/event stream to an Axum engine using
`tokio::sync::broadcast` + `axum::extract::ws`. This is the pattern
used for the MarketMoves `/api/v1/ws` control-room telemetry channel.

## Architecture (one channel, many publishers, N subscribers)

```
main.rs: broadcast::channel(64) ──┬── tx → AppState.tx (ws_handler subscribes)
                                  ├── tx.clone() → EquityScheduler.tx
                                  └── tx.clone() → PaperExecutor.tx
                                                                    │
                          ws_handler ── tx.subscribe() ── forward_events ── ws client
```

- **One** broadcast channel created in `main.rs`.
- `tx` clones handed to every producer (scheduler, executor, future
  health monitors). Cloning is cheap — broadcast senders are `Arc`-backed.
- The `ws_handler` calls `state.tx.subscribe()` PER connection, creating
  a new receiver each time. No global subscriber registry needed.

## Module structure

Create `engine/src/api/ws.rs` with the event enum, sender type alias,
and the upgrade handler:

```rust
// ws.rs
use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};   // ← BOTH required (see pitfalls)
use serde::{Deserialize, Serialize};

use super::AppState;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]               // internally-tagged: {"type":"TradeFill", ...}
pub enum TelemetryEvent {
    PnlTick { realized_pnl: f64, unrealized_pnl: f64, position: String,
              entry_price: Option<f64>, last_close: Option<f64>, timestamp: i64 },
    PredictionUpdate { pred_1d: Option<f64>, pred_5d: Option<f64>,
                       pred_21d: Option<f64>, timestamp: i64 },
    TradeFill { side: String, qty: f64, price: f64, fee: f64,
                realized_pnl: f64, timestamp: i64 },
    // ... other variants
}

pub type TelemetrySender = tokio::sync::broadcast::Sender<TelemetryEvent>;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.tx.subscribe();
    ws.on_upgrade(move |socket| forward_events(socket, rx))
}

async fn forward_events(socket: WebSocket, mut rx: broadcast::Receiver<TelemetryEvent>) {
    let (mut sender, mut receiver) = socket.split();  // needs StreamExt

    let mut send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let json = serde_json::to_string(&event).unwrap_or_default();
                    if sender.send(Message::Text(json)).await.is_err() { break; }  // SinkExt
                }
                Err(broadcast::error::RecvError::Lagged(n)) => { tracing::warn!(n, "lagged"); continue; }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        let _ = sender.send(Message::Close(None)).await;
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {   // .next() not .recv()
            if matches!(msg, Message::Close(_)) { break; }
        }
    });

    tokio::select! {
        _ = &mut send_task => { recv_task.abort(); }
        _ = &mut recv_task => { send_task.abort(); }
    }
}
```

Declare as `pub(crate)` in `api/mod.rs` (NOT plain `mod ws;`) so the
scheduler and executor can reference `crate::api::ws::TelemetrySender`:

```rust
// api/mod.rs
pub(crate) mod ws;

#[derive(Clone)]
pub(crate) struct AppState {
    pub pool: db::DbPool,
    // ... existing fields ...
    pub tx: ws::TelemetrySender,          // ← new field
}

pub fn router(pool: db::DbPool, config: &Config, tx: ws::TelemetrySender) -> Router {
    // ...
    Router::new()
        .route("/api/v1/ws", get(ws::ws_handler))   // ← new route
        // ...
}
```

## Wiring producers (scheduler, executor)

Pass `Option<TelemetrySender>` so test code can pass `None`:

```rust
// main.rs
let (tx, _rx) = tokio::sync::broadcast::channel(64);
// executor gets Some(tx.clone())
// scheduler gets Some(tx.clone())
// router gets tx
```

```rust
// In scheduler / executor — publish events:
if let Some(tx) = &self.tx {
    let _ = tx.send(TelemetryEvent::TradeFill { ... });
}
```

`tx.send()` returns `Err(SendError)` when there are **zero subscribers**.
This is NORMAL (no WebSocket client connected) — always use
`let _ = tx.send(...)` and discard the result.

## Trait import pitfalls (each costs a full compile cycle)

| Error | Missing import | Fix |
|-------|---------------|-----|
| `no method named 'split' found for struct 'WebSocket'` | `StreamExt` | `use futures::StreamExt;` |
| `no method named 'send' found for struct 'SplitSink'` | `SinkExt` | `use futures::SinkExt;` |
| `no method named 'recv' found for struct 'SplitStream'` | — | `SplitStream` is a `Stream`, not a receiver — use `.next()` (from `StreamExt`), not `.recv()` |
| `module 'ws' is private` (from scheduler.rs) | — | declare `pub(crate) mod ws;` not `mod ws;` |

Add `futures = { workspace = true }` to `Cargo.toml` if not present.

## Test updates

Every test that constructs `AppState { ... }` must now provide `tx`:

```rust
fn test_state(pool: db::DbPool) -> State<AppState> {
    let (tx, _rx) = tokio::sync::broadcast::channel(64);
    State(AppState { pool, /* ... */ tx })
}
```

Every `router(pool, &config)` call needs a tx arg:

```rust
let app = { let (tx, _rx) = broadcast::channel(64); router(pool, &config, tx) };
```

Every `PaperExecutor::new(pool, fee)` call gains a third arg:
`PaperExecutor::new(pool, fee, None)` (tests don't need telemetry).

## Verification

```bash
cargo check --lib 2>&1 | grep -E "^error"   # 0 errors
cargo test --lib api::ws::tests              # serialization tests
cargo test --lib api::tests                  # router + state tests
cargo test --lib exec::paper::tests          # executor tests
```

Front-end smoke test (if a WS client is wired): open devtools console
and check `ws://localhost:<port>/api/v1/ws` receives JSON messages with
a `"type"` field.
