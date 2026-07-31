# Strategy Configuration UI & Runtime Config Plan

**Goal:** Make all strategy parameters configurable live from the UI — select which
strategy is running at any point, see what's active, and tune thresholds without
restarting the container.

**Current state (2026-07-31):**
- `sma_window`, `enable_shorting`, `short_entry_threshold`, `short_exit_threshold`
  are env vars → read once at startup, not changeable live
- `pred_5d_filter` is hardcoded `true` in `main.rs:166` — no env var
- `entry_threshold` is `MAGNITUDE_THRESHOLD` env var
- `exit_threshold` is computed from entry: `-entry / 3.0` (hardcoded formula)
- `/api/mode` already shows the pattern for runtime config toggling (Arc<RwLock<>> + API + WS broadcast)
- `/api/strategies` stores backtest configs in DB but they aren't applied to live trading
- `AppState` only has `sma_window` and `symbol` — no threshold/exit/filter fields

---

## Phase 1: Externalize all strategy params into AppState (backend)

### Task 1.1: Add strategy params to Config
**File:** `engine/src/config.rs`

Add env vars with optimal defaults:
- `PRED_5D_FILTER` bool, default `false` (optimal config)
- `ENTRY_THRESHOLD` f64, default `0.001` (replace `MAGNITUDE_THRESHOLD`)
- `EXIT_THRESHOLD` f64, default `-0.0005` (no more formula — explicit)

Update `Config` struct with:
```rust
pub entry_threshold: f64,
pub exit_threshold: f64,
pub pred_5d_filter: bool,
```

Update `from_env()`, `Default` test, and `clear_engine_env()` key list.

### Task 1.2: Add strategy params to AppState
**File:** `engine/src/api/mod.rs`

Wrap in `Arc<RwLock<>>` so the API can mutate at runtime:
```rust
pub entry_threshold: std::sync::Arc<tokio::sync::RwLock<f64>>,
pub exit_threshold: std::sync::Arc<tokio::sync::RwLock<f64>>,
pub pred_5d_filter: std::sync::Arc<tokio::sync::RwLock<bool>>,
pub enable_shorting: std::sync::Arc<tokio::sync::RwLock<bool>>,
pub short_entry_threshold: std::sync::Arc<tokio::sync::RwLock<f64>>,
pub short_exit_threshold: std::sync::Arc<tokio::sync::RwLock<f64>>,
```

### Task 1.3: Update main.rs to pass params from Config
**File:** `engine/src/main.rs`

Replace the hardcoded `eq_strategy_params` block with values from `cfg`:
```rust
let eq_strategy_params = strategy::EquityStrategyParams {
    entry_threshold: cfg.entry_threshold,
    exit_threshold: cfg.exit_threshold,
    sma_window: cfg.sma_window,
    enable_shorting: cfg.enable_shorting,
    short_entry_threshold: cfg.short_entry_threshold,
    short_exit_threshold: cfg.short_exit_threshold,
    pred_5d_filter: cfg.pred_5d_filter,
};
```

### Task 1.4: Update scheduler to read from shared state
**File:** `engine/src/main.rs` (scheduler loop)

The scheduler currently reads from the one-time `eq_strategy_params`. Change it
to read from `Arc<RwLock<EquityStrategyParams>>` at each cycle so runtime
changes take effect on the next bar.

### Task 1.5: Expose strategy params in /api/status
**File:** `engine/src/api/status.rs`

Add to StatusResponse:
```json
{
  "strategy": {
    "entry_threshold": 0.001,
    "exit_threshold": -0.0005,
    "sma_window": 40,
    "pred_5d_filter": false,
    "enable_shorting": true,
    "short_entry_threshold": -0.001,
    "short_exit_threshold": 0.0005
  }
}
```

---

## Phase 2: Runtime config API endpoints

### Task 2.1: GET /api/strategy-config
**File:** `engine/src/api/strategy_config.rs` (new)

Returns current strategy params from AppState. No auth required.

### Task 2.2: PUT /api/strategy-config
**File:** `engine/src/api/strategy_config.rs`

Accepts JSON body with partial or full strategy params. Validates:
- `entry_threshold` > 0
- `exit_threshold` < 0
- `sma_window` > 0 and <= 300
- `short_entry_threshold` < 0
- `short_exit_threshold` > `short_entry_threshold`

On success: updates shared state, broadcasts `StrategyConfigChange` WS event,
appends audit log to DB.

### Task 2.3: Wire routes in mod.rs
```rust
.route("/api/strategy-config", get(strategy_config::handle_get))
.route("/api/strategy-config", put(strategy_config::handle_put))
```

---

## Phase 3: Frontend — Strategy Config Panel

### Task 3.1: New component StrategyConfigPanel.svelte
**File:** `frontend/src/lib/components/StrategyConfigPanel.svelte`

Shows:
- Active strategy badge (e.g. "Threshold (SMA=40, pred_5d=off)")
- Editable inputs for each param:
  - Entry threshold (range: 0.0005–0.01, step: 0.0005)
  - Exit threshold (range: -0.005 to -0.0001, step: 0.0005)
  - SMA window (range: 20–300, step: 1)
  - pred_5d filter (toggle switch)
  - Enable shorting (toggle switch)
  - Short entry threshold (range: -0.01 to -0.0005)
  - Short exit threshold (range: 0.0005 to 0.01)
- Quick-preset buttons:
  - "SMA=40 Optimal" → entry=0.001, exit=-0.0005, sma=40, enable_shorting=true, short_entry=-0.001, short_exit=0.0005, pred_5d_filter=false
  - "SMA=200 Conservative" → entry=0.003, exit=-0.001, sma=200, enable_shorting=false, pred_5d_filter=true
  - "Rhai Mean-Reversion" → activate stored Rhai strategy
- Save button (PUT /api/strategy-config)

### Task 3.2: API helpers
**File:** `frontend/src/lib/api.js`

Add:
- `fetchStrategyConfig()` → GET /api/strategy-config
- `saveStrategyConfig(params)` → PUT /api/strategy-config

### Task 3.3: Update Dashboard layout
**File:** `frontend/src/views/Dashboard.svelte`

Add `StrategyConfigPanel` to the grid. New layout:
```
[candlestick] [PnL curve]   [status]
[strategy config] [features] [health]
[trade history — full width]
```

### Task 3.4: Update StatusPanel with strategy indicator
**File:** `frontend/src/lib/components/StatusPanel.svelte`

Add a strategy row showing active config summary, e.g.:
"Strategy: Threshold (SMA=40, no filter)"

---

## Phase 4: Dockerfile defaults & deploy

### Task 4.1: Update Dockerfile env defaults
**File:** `engine/Dockerfile`

Replace:
```
SMA_WINDOW=200  → SMA_WINDOW=40
```
Add:
```
ENTRY_THRESHOLD=0.001
EXIT_THRESHOLD=-0.0005
PRED_5D_FILTER=false
ENABLE_SHORTING=true
SHORT_ENTRY_THRESHOLD=-0.001
SHORT_EXIT_THRESHOLD=0.0005
```

### Task 4.2: Rebuild and redeploy
```bash
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .
docker rm -f mmn-engine
docker run -d --name mmn-engine ...
```

---

## Files that change

| File | Change |
|------|--------|
| `engine/src/config.rs` | Add 3 fields + env parsing |
| `engine/src/api/mod.rs` | Add 6 fields to AppState |
| `engine/src/api/status.rs` | Add strategy block to response |
| `engine/src/api/strategy_config.rs` | NEW — GET/PUT handlers |
| `engine/src/main.rs` | Pass params from Config, read from shared state |
| `engine/Dockerfile` | Update defaults to optimal |
| `frontend/src/lib/api.js` | Add fetchStrategyConfig/saveStrategyConfig |
| `frontend/src/lib/components/StrategyConfigPanel.svelte` | NEW |
| `frontend/src/lib/components/StatusPanel.svelte` | Add strategy row |
| `frontend/src/views/Dashboard.svelte` | Add panel to grid |
| `engine/src/strategy.rs` | No change (params already support all fields) |

---

## Validation

```bash
cd engine && cargo check --lib && cargo test --lib
cd frontend && npm run build
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .
```

---

## Open questions

1. **Should /api/strategy-config require TOTP auth like /api/mode?**
   Strategy changes are less dangerous than live-mode flips. No auth for paper
   mode; TOTP-gate only if trading_mode is `live`.

2. **Should we allow switching between threshold and Rhai strategies live?**
   The scheduler currently only runs the threshold strategy. Adding Rhai as a
   live option would require the scheduler to support StrategyKind dispatch.
   Leave for a future iteration — start with threshold-only runtime config.

3. **Should preset configs be stored server-side or client-side?**
   Client-side presets in JS are simpler and avoid DB migration. If we want
   named/saved configs later, the `/api/strategies` endpoint already supports
   persistence.
