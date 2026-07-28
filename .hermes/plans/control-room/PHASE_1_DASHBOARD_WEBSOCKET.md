# Phase 1 — Reactive Dashboard & WebSocket Telemetry

**Goal**: Build the new Svelte dashboard with a professional widget layout, replace the
5-second polling loop with WebSocket push telemetry, and retire the old vanilla JS frontend.
After this phase, the control room is the new Svelte app — live, reactive, professional.

**Estimated effort**: ~2 weeks
**Can deploy independently**: Yes — this is the user-visible cutover phase.
**Depends on**: Phase 0 (Svelte scaffold, rust-embed, API module tree)

---

## 1.1 WebSocket Backend (`engine/src/api/ws.rs`)

### Design
- A `tokio::sync::broadcast::Sender<TelemetryEvent>` is added to `AppState`.
- The scheduler / executor publishes events when state changes (new candle, trade fill,
  prediction update, position change, mode switch).
- The `/api/v1/ws` endpoint upgrades to WebSocket, subscribes to the broadcast channel,
  and forwards events as JSON.

### TelemetryEvent enum
```rust
// engine/src/api/ws.rs
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum TelemetryEvent {
    PnlTick {
        realized_pnl: f64,
        unrealized_pnl: f64,
        position: String,
        entry_price: Option<f64>,
        last_close: Option<f64>,
        timestamp: i64,
    },
    PredictionUpdate {
        pred_1d: Option<f64>,
        pred_5d: Option<f64>,
        pred_21d: Option<f64>,
        timestamp: i64,
    },
    FeatureUpdate {
        features: [f64; 8],
        normalized: [f64; 8],
        timestamp: i64,
    },
    TradeFill {
        side: String,
        qty: f64,
        price: f64,
        fee: f64,
        realized_pnl: f64,
        timestamp: i64,
    },
    ModeChange {
        mode: String,
        timestamp: i64,
    },
    StalenessAlert {
        last_candle_ts: Option<i64>,
        seconds_since_last: i64,
    },
}
```

### WebSocket handler
```rust
// engine/src/api/ws.rs
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.tx.subscribe();

    // Spawn a task to forward broadcast messages to the client
    let send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            let json = serde_json::to_string(&event).unwrap_or_default();
            if sender.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    });

    // Receive loop (handle ping/pong, ignore client messages for now)
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if msg.is_close() { break; }
        }
    });

    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }
}
```

### AppState changes (`engine/src/api/mod.rs`)
```rust
#[derive(Clone)]
pub struct AppState {
    pool: db::DbPool,
    trading_mode: crate::config::TradingMode,
    symbol: String,
    sma_window: usize,
    tx: tokio::sync::broadcast::Sender<TelemetryEvent>,  // NEW
}
```
- The broadcast channel is created in `router()` with a capacity of 64.
- `scheduler.rs` and `exec/paper.rs` receive a clone of `tx` and publish events.

### Route registration
```rust
// In api/mod.rs router():
.route("/api/v1/ws", get(ws::ws_handler))
```

### Files to create/modify
- **CREATE** `engine/src/api/ws.rs`
- **MODIFY** `engine/src/api/mod.rs` — add `tx` to AppState, register WS route
- **MODIFY** `engine/src/scheduler.rs` — publish `PnlTick`, `PredictionUpdate`, `FeatureUpdate` events
- **MODIFY** `engine/src/exec/paper.rs` — publish `TradeFill` events
- **MODIFY** `engine/src/main.rs` — create broadcast channel, pass to router + scheduler

---

## 1.2 Svelte Dashboard (`frontend/src/views/Dashboard.svelte`)

### Layout
The dashboard uses a CSS grid widget layout. On wide screens (trading desk), it's a
3-column layout. On narrow screens, it collapses to a single column.

```
+------------------------------------------------------------------+
| Sidebar Nav  |  Main Dashboard Area                              |
|              |                                                    |
| - Dashboard  |  +------------------+  +----------------------+   |
| - Strategy   |  | Candlestick Chart|  | PnL / Equity Curve   |   |
|   Lab        |  | (uPlot + SMA200  |  | (real-time WS update)|   |
| - Ledger     |  |  + pred cones)   |  |                      |   |
| - Advisor    |  +------------------+  +----------------------+   |
|              |  +------------------+  +----------------------+   |
| ---          |  | Status / Position|  | Feature Inspector    |   |
| Mode: PAPER  |  | (mode, symbol,   |  | (8-dim sparklines +  |   |
| Symbol: QQQ  |  |  entry, PnL)     |  |  normalized values)  |   |
|              |  +------------------+  +----------------------+   |
|              |  +--------------------------------------------+   |
|              |  | Trade History (recent fills from WS)       |   |
|              |  +--------------------------------------------+   |
+------------------------------------------------------------------+
```

### Components to build
```
frontend/src/
  App.svelte                      # Root shell: sidebar nav + <slot>
  lib/
    stores.js                     # Svelte stores: status, predictions, features, trades, wsConnected
    websocket.js                  # WebSocket manager: connect, reconnect, dispatch to stores
    api.js                        # REST fallback (initial load + endpoints not on WS)
    components/
      CandlestickChart.svelte     # uPlot candlestick + 200-SMA + prediction trajectory cones
      PnLEquityCurve.svelte       # Real-time PnL curve (realized + unrealized)
      StatusPanel.svelte          # Mode badge, symbol, position, entry, PnL, staleness
      FeatureInspector.svelte     # 8 feature sparklines + normalized bars
      TradeHistory.svelte         # Live trade fill log (from WS TradeFill events)
      ModelHealth.svelte          # Staleness, inference latency, IC drift (from /api/accuracy)
  views/
    Dashboard.svelte              # Assembles all components in the grid layout
    Ledger.svelte                 # Historical trades table + cumulative PnL
```

### WebSocket manager (`frontend/src/lib/websocket.js`)
```js
import { writable } from 'svelte/store';
import { status, predictions, features, trades, wsConnected } from './stores.js';

const WS_URL = `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/api/v1/ws`;

let ws = null;
let reconnectDelay = 1000;

export function connectWebSocket() {
  ws = new WebSocket(WS_URL);

  ws.onopen = () => {
    wsConnected.set(true);
    reconnectDelay = 1000; // reset backoff
  };

  ws.onmessage = (event) => {
    const msg = JSON.parse(event.data);
    switch (msg.type) {
      case 'PnlTick':        status.update(s => ({ ...s, ...msg })); break;
      case 'PredictionUpdate': predictions.set(msg); break;
      case 'FeatureUpdate':   features.set(msg); break;
      case 'TradeFill':       trades.update(t => [msg, ...t].slice(0, 50)); break;
      case 'ModeChange':      status.update(s => ({ ...s, mode: msg.mode })); break;
      case 'StalenessAlert':  status.update(s => ({ ...s, staleness: msg })); break;
    }
  };

  ws.onclose = () => {
    wsConnected.set(false);
    setTimeout(() => {
      reconnectDelay = Math.min(reconnectDelay * 2, 30000);
      connectWebSocket();
    }, reconnectDelay);
  };

  ws.onerror = () => ws.close();
}
```

### Stores (`frontend/src/lib/stores.js`)
```js
import { writable } from 'svelte/store';

export const wsConnected = writable(false);
export const status = writable(null);
export const predictions = writable(null);
export const features = writable(null);
export const trades = writable([]);
export const accuracy = writable(null);
```

### Initial load strategy
- On mount, the Dashboard calls REST endpoints (`/api/status`, `/api/predictions`,
  `/api/chart`, `/api/accuracy`) for the initial snapshot.
- WebSocket pushes deltas after that.
- The REST `/api/chart` endpoint is still used for the candlestick chart (historical data
  is too large for WS push — only new candles are pushed).

### Prediction trajectory cones
- The candlestick chart overlays three dashed lines projecting the 1D, 5D, and 21D
  predicted returns from the current close. These are computed client-side from the
  prediction values (e.g. `projected_1d = last_close * (1 + pred_1d)`).

---

## 1.3 Ledger view (`frontend/src/views/Ledger.svelte`)

- Fetches `/api/equity/data` for historical trades.
- Displays a table: date, side, qty, price, fee, realized PnL.
- Clicking a row expands to show the prediction values that triggered the trade
  (from `equity_predictions` joined on timestamp).
- Cumulative equity curve chart (uPlot line) from realized PnL over time.

---

## 1.4 Retire old frontend

### Steps
1. After the Svelte dashboard is functional and tested:
2. Remove the `/legacy` route from `api/mod.rs`.
3. Delete `frontend/legacy/` directory.
4. Remove `tower-http` `ServeDir` from the legacy route (keep `rust-embed` fallback only).
5. The old `frontend/README.md` can stay as a historical note.

### Rollback plan
- If the Svelte dashboard has issues after deploy, temporarily re-add the `/legacy` route
  and point users there. The old files can be kept in git history if needed.

---

## 1.5 Test requirements

### Backend
- `cargo test` — all existing tests + new WS tests.
- Unit test: `TelemetryEvent` serialization produces valid JSON with `type` tag.
- Integration test: broadcast channel delivers events to multiple subscribers.
- Manual: connect to `ws://localhost:3000/api/v1/ws` via `websocat` and verify events.

### Frontend
- `npm run build` — compiles without errors.
- Manual: dashboard loads, shows live data, WS connection indicator is green.
- Manual: when a trade executes, the Trade History and PnL panels update instantly.
- Manual: when the scheduler produces a new prediction, the Prediction panel updates.
- Manual: sidebar navigation switches between Dashboard and Ledger views.
- Manual: responsive layout works on narrow viewport (single column).

---

## 1.6 Rollout steps

1. Deploy to VPS with the old frontend still served at `/legacy`.
2. Verify the new dashboard works in production (WS connects, data flows).
3. Verify the legacy frontend still works as fallback.
4. After 1–2 days of stable operation, remove `/legacy` route and delete old files.
5. Update Docker build process: add `npm run build` step before `cargo build`.

---

## 1.7 Risk notes

- **WebSocket in production**: The VPS reverse proxy (nginx/caddy) must support WebSocket
  upgrade headers. Verify proxy config allows `Upgrade` and `Connection` headers.
- **Broadcast channel capacity**: If the client is slow, events may be dropped. The
  broadcast channel drops old messages when the buffer is full. Mitigation: capacity 64
  is generous for this low-frequency system (events are per-bar, not per-tick).
- **uPlot + Svelte integration**: uPlot is imperative (not reactive). The chart component
  must manage uPlot lifecycle manually (create on mount, update on data change, destroy
  on unmount). Use a `onMount` + reactive `$:` block pattern.
- **Prediction cones**: These are visual approximations. Clearly label them as model
  projections, not guaranteed trajectories. Use dashed lines with a distinct color.
