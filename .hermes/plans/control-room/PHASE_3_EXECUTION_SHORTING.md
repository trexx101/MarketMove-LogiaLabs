# Phase 3 — Execution Overhaul & Shorting

**Goal**: Extend the strategy engine to support shorting via inverse ETFs (PSQ for QQQ),
implement the runtime paper/live toggle with TOTP + parity validation, and build the
Moomoo LiveExecutor. After this phase, the system can go short and can trade live.

**Estimated effort**: ~2 weeks
**Can deploy independently**: Yes — shorting and live toggle are additive features.
**Depends on**: Phase 0 (DB tables, API module tree), Phase 1 (AppState with broadcast channel)

---

## 3.1 Extend `EquityStrategyParams` with Shorting

### Current state (`engine/src/strategy.rs:130`)
```rust
pub struct EquityStrategyParams {
    pub entry_threshold: f64,
    pub exit_threshold: f64,
    pub sma_window: usize,
}
```
Strategy is long/flat only. `next_equity_position` blocks all new entries when close < SMA200.

### Target state
```rust
pub struct EquityStrategyParams {
    pub entry_threshold: f64,
    pub exit_threshold: f64,
    pub sma_window: usize,
    pub enable_shorting: bool,           // default false
    pub short_entry_threshold: f64,      // e.g. -0.004
    pub short_exit_threshold: f64,       // e.g. 0.001
}
```

### Updated `next_equity_position` logic
```
Bullish regime (close > SMA200, sma_valid):
  - Long entry:  pred_1d > entry_threshold AND pred_5d > 0
  - Long exit:   pred_1d < exit_threshold
  - (existing behavior — unchanged)

Bearish regime (close <= SMA200 OR sma_invalid):
  - Allow long exits (existing behavior)
  - IF enable_shorting:
    - Short entry: pred_1d < short_entry_threshold
    - Short exit:  pred_1d > short_exit_threshold → return Flat
    - Block short entry if already Long (must exit to Flat first)
  - ELSE: no new entries (existing behavior)

Transitions:
  - Long → Short: NOT allowed directly. Must go Long → Flat → Short.
    The executor handles this in two fills (sell QQQ, then buy PSQ).
  - Short → Long: NOT allowed directly. Must go Short → Flat → Long.
```

### Files to modify
- **MODIFY** `engine/src/strategy.rs` — extend struct + update `next_equity_position`
- **MODIFY** `engine/src/config.rs` — add `enable_shorting`, `short_entry_threshold`,
  `short_exit_threshold` env vars with defaults (shorting OFF by default)
- **MODIFY** `engine/src/strategy.rs` tests — add test cases for short entry/exit/hold

### Key constraint
- `enable_shorting` defaults to `false`. Existing behavior is 100% preserved when shorting is off.
- The `Default` impl for `EquityStrategyParams` must not change (backward compat).

---

## 3.2 PSQ Inverse ETF Execution

### Design
When `Position::Short` is the target:
1. If currently Long (QQQ): sell QQQ to Flat, then buy PSQ.
2. If currently Flat: buy PSQ.
3. If currently Short (already holding PSQ): no action.

When transitioning from Short to Flat or Long:
1. Sell PSQ to Flat.
2. If target is Long: buy QQQ.

### PaperExecutor changes (`engine/src/exec/paper.rs`)
- Add a `short_symbol` field (default `"PSQ"`). When the target is `Position::Short`,
  the executor records the trade against the short symbol, not the primary symbol.
- PnL for short: `(entry_price - exit_price) * qty - fees` (same as current logic —
  the PaperExecutor already handles Short PnL correctly).
- The `FillResult` struct gains an optional `symbol` field so the UI can display which
  instrument was traded.

### LiveExecutor (`engine/src/exec/live.rs`) — NEW
```rust
pub struct MoomooLiveExecutor {
    // OpenD gateway connection (via engine/src/data/moomoo.rs)
    // Environments: TrdEnv_Simulate (paper) or TrdEnv_Real (live)
}
```
- Implements the same `set_target_position` interface as PaperExecutor.
- Routes orders through Moomoo OpenD (port 11111 protobuf TCP).
- For Short: places a BUY order for PSQ (not a traditional short sell — no borrow needed).
- Bid-ask spread check: before entering PSQ, query the order book. If spread > threshold
  (configurable, e.g. 0.5%), abort and default to Flat.

### ExecutorKind enum update (`engine/src/exec/mod.rs`)
```rust
pub enum ExecutorKind {
    Paper(paper::PaperExecutor),
    Live(live::MoomooLiveExecutor),
}
```

### Files to create/modify
- **MODIFY** `engine/src/exec/paper.rs` — add short_symbol support, FillResult.symbol
- **CREATE** `engine/src/exec/live.rs` — MoomooLiveExecutor
- **MODIFY** `engine/src/exec/mod.rs` — add Live variant to ExecutorKind
- **MODIFY** `engine/src/data/moomoo.rs` — implement order placement + quote query
  (currently only interface stubs — fill in the actual OpenD protobuf calls)

---

## 3.3 Runtime Paper/Live Toggle

### AppState changes (`engine/src/api/mod.rs`)
```rust
#[derive(Clone)]
pub struct AppState {
    pool: db::DbPool,
    trading_mode: Arc<RwLock<TradingMode>>,  // CHANGED: was static TradingMode
    symbol: String,
    sma_window: usize,
    tx: broadcast::Sender<TelemetryEvent>,
    parity_marker_path: String,              // NEW
    totp_secret: String,                     // NEW
}
```

### `GET /api/mode` (`engine/src/api/mode.rs`) — NEW
```json
// Response:
{
  "mode": "paper" | "live",
  "last_switch": 1718920000,
  "parity_valid": true
}
```
- Reads `trading_mode.read()` from AppState.
- Checks parity marker file age (valid if < 604,800 seconds / 7 days).

### `POST /api/mode` (`engine/src/api/mode.rs`) — NEW
```json
// Request:
{ "mode": "live", "auth_token": "123456" }

// Response:
{ "success": true, "message": "switched to live" }
// Or 403:
{ "success": false, "message": "TOTP invalid" }
```

### Validation flow
1. **Parity check**: Read `parity_verified.json`. If timestamp > 7 days old, reject with 403.
2. **TOTP check**: Verify the 6-digit code against `totp_secret` using `totp-rs`.
3. **Mode switch**: `trading_mode.write()` to update the mode.
4. **Audit log**: Insert a row into `mode_switches` table.
5. **Broadcast**: Publish `TelemetryEvent::ModeChange` on the WebSocket channel.
6. **Executor swap**: The scheduler checks `trading_mode.read()` before each execution
   cycle and uses PaperExecutor or LiveExecutor accordingly.

### TOTP setup
- On first run, if `TOTP_SECRET` env var is not set, generate a new secret and log a
  QR code URL (otpauth://) that the user scans with Google Authenticator.
- Store the secret in an env var or a file at `~/.marketmoves/totp_secret`.

### Route registration
```rust
.route("/api/mode", get(mode::handle_get_mode))
.route("/api/mode", post(mode::handle_set_mode))
```

### Files to create/modify
- **CREATE** `engine/src/api/mode.rs`
- **MODIFY** `engine/src/api/mod.rs` — add `mod mode;`, register routes, change AppState
- **MODIFY** `engine/src/main.rs` — create `Arc<RwLock<TradingMode>>`, pass to AppState
- **MODIFY** `engine/src/scheduler.rs` — read trading_mode at each cycle, select executor
- **MODIFY** `engine/src/config.rs` — add `totp_secret`, `parity_marker_path` fields

---

## 3.4 Frontend: Mode Toggle UI

### Dashboard mode badge (update `StatusPanel.svelte`)
- Already shows PAPER / LIVE badge from WS `ModeChange` events.
- Add a "Switch Mode" button next to the badge.
- Clicking it opens a modal:

```
+----------------------------------+
| Switch to LIVE Trading           |
+----------------------------------+
| ⚠️ This will execute real orders |
| via Moomoo OpenD.                |
|                                  |
| Parity status: ✅ Valid          |
| (verified 2 days ago)            |
|                                  |
| TOTP Code: [______]              |
|                                  |
| [Cancel]  [Switch to LIVE]      |
+----------------------------------+
```

### Components
```
frontend/src/lib/components/
  ModeToggle.svelte    # Modal dialog with TOTP input + parity status
```

- Calls `POST /api/mode` with the TOTP code.
- Shows success/error message.
- On success, the WS `ModeChange` event updates the badge automatically.

---

## 3.5 Test requirements

### Backend
- Unit test: `next_equity_position` with `enable_shorting=true` — short entry in bearish regime.
- Unit test: `next_equity_position` with `enable_shorting=true` — short exit to flat.
- Unit test: `next_equity_position` with `enable_shorting=false` — behavior unchanged (regression).
- Unit test: PaperExecutor handles Long → Short transition (sell QQQ, buy PSQ).
- Unit test: PaperExecutor handles Short → Long transition (sell PSQ, buy QQQ).
- Unit test: TOTP validation accepts a valid code, rejects an invalid one.
- Integration test: `POST /api/mode` rejects when parity marker is expired.
- Integration test: `POST /api/mode` rejects when TOTP is wrong.
- Integration test: `POST /api/mode` succeeds and writes to `mode_switches` table.

### Frontend
- Manual: Mode badge shows PAPER by default.
- Manual: Click "Switch Mode" → modal opens → enter wrong TOTP → error shown.
- Manual: Enter correct TOTP → mode switches to LIVE → badge updates via WS.
- Manual: Shorting disabled by default — no short positions taken in paper mode.
- Manual: Enable shorting via env var → strategy takes short positions in bearish regime.

---

## 3.6 Rollout steps

1. Deploy shorting logic first (paper mode only) — verify PSQ trades execute correctly
   in the PaperExecutor.
2. Deploy TOTP + mode toggle UI — but keep the system in paper mode.
3. Test TOTP flow: generate secret, scan QR, verify code acceptance.
4. Only after all tests pass: do a live mode switch test with a minimal position size.
5. Monitor the `mode_switches` audit table for the transition record.

---

## 3.7 Risk notes

- **PSQ tracking error**: Inverse ETFs have compounding drift over multi-day holds.
  Mitigation: the model's horizons are 1D/5D/21D — relatively short. Document the risk
  in the UI (tooltip on the Short position badge).
- **PSQ bid-ask spread**: During volatility spikes, PSQ spreads widen. Mitigation:
  the LiveExecutor checks spread before entry and aborts if > threshold. The PaperExecutor
  does not check spread (simulated fills at close price) — document this gap.
- **TOTP secret management**: If the secret is lost, the user is locked out of live mode.
  Mitigation: store the secret in a file with restricted permissions, and provide a
  recovery procedure (regenerate secret, which requires shell access to the VPS).
- **Moomoo OpenD 2FA**: The OpenD daemon may require 2FA/CAPTCHA on restart. The engine
  must detect OpenD disconnection and halt new orders. Mitigation: watchdog ping in the
  scheduler loop — if OpenD is unreachable, force Flat and alert via WS.
- **Direct Long→Short transition**: The strategy must never return Short when current is
  Long. The `next_equity_position` function must return Flat first. The executor relies
  on this. Add an assertion or guard in the executor that rejects Long→Short directly.
