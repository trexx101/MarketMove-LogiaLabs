# P3 Execution Layer Patterns

## Reconciliation Module Structure

Pattern for comparing expected vs actual state (orders, positions):

```rust
// State representations
pub struct OrderState {
    pub id: String,
    pub status: String,
}

pub struct PositionState {
    pub symbol: String,
    pub quantity: f64,
}

// Mismatch detection
pub enum Mismatch {
    MissingOrder(String),
    OrphanedOrder(String),
    StatusMismatch { order_id: String, expected: String, actual: String },
    PositionMismatch { symbol: String, expected: f64, actual: f64 },
}

pub struct ReconciliationResult {
    pub mismatches: Vec<Mismatch>,
    pub is_clean: bool,
}

// Reconcile functions
pub fn reconcile_orders(expected: &[OrderState], actual: &[OrderState]) -> ReconciliationResult
pub fn reconcile_positions(expected: &[PositionState], actual: &[PositionState]) -> ReconciliationResult
```

**Key details:**
- Use 1e-6 tolerance for floating point comparisons in position quantities
- Check both directions: missing (expected but not actual) and orphaned (actual but not expected)
- Return structured result with `is_clean` flag for startup gate

## Write-Ahead Intent Logging

Pattern for persisting stage transitions BEFORE order sends:

```rust
pub enum ExitStage {
    Stage1,
    Stage2,
    Stage3,
    Complete,
}

impl std::fmt::Display for ExitStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExitStage::Stage1 => write!(f, "stage_1"),
            ExitStage::Stage2 => write!(f, "stage_2"),
            ExitStage::Stage3 => write!(f, "stage_3"),
            ExitStage::Complete => write!(f, "complete"),
        }
    }
}

pub struct IntentLogger {
    pool: sqlx::SqlitePool,
}

impl IntentLogger {
    // Log BEFORE sending order
    pub async fn log_intent(
        &self,
        position_id: i64,
        stage: ExitStage,
        limit_price: f64,
        quantity: f64,
    ) -> Result<i64, sqlx::Error>
    
    // Update with order ID AFTER order is sent
    pub async fn update_order_id(&self, intent_id: i64, order_id: &str) -> Result<(), sqlx::Error>
    
    // Query methods
    pub async fn get_latest_intent(&self, position_id: i64) -> Result<Option<IntentLogEntry>, sqlx::Error>
    pub async fn get_position_intents(&self, position_id: i64) -> Result<Vec<IntentLogEntry>, sqlx::Error>
}
```

**Key details:**
- Use Display trait for DB storage (TEXT field)
- Two-phase: log_intent() returns ID, then update_order_id() after send
- Enables crash recovery: if engine dies mid-stage, restart can detect incomplete intent
- DB table: `exit_intent_log (id, position_id, stage, order_id, limit_price, quantity, timestamp)`

## ExitArbiter Priority Table

Fixed priority (lower number = higher priority):
1. OperatorForceClose
2. CircuitBreaker
3. DteOverride
4. TrailingStop
5. RoiTable
6. SignalReversal

```rust
pub enum ExitSource {
    OperatorForceClose = 1,
    CircuitBreaker = 2,
    DteOverride = 3,
    TrailingStop = 4,
    RoiTable = 5,
    SignalReversal = 6,
}

pub struct ExitSignal {
    pub source: ExitSource,
    pub priority: u8,
    pub reason: String,
    pub timestamp: DateTime<Utc>,
}

pub struct ExitArbiter;

impl ExitArbiter {
    pub fn select_winner(&self, signals: &[ExitSignal]) -> Option<ExitSignal> {
        signals.iter().min_by_key(|s| s.priority).cloned()
    }
}
```

## Staged Exit Ladder

**Two-layer architecture:** stateless price calculator + stateful position tracker.

### Layer 1: Stateless Price Calculator (`StagedLadder`)
Pure function `stage_price(stage, bid) → price`. Easy to unit test, no side effects.

### Layer 2: Stateful Position Tracker (`StagedExitLadder`)
Wraps the calculator with per-position state: `position_id`, `current_stage`, `stage_start_time`, `current_bid`, `tick_size`.

Key methods:
- `start_stage_1(bid, tick_size)` — initialize
- `should_advance(now) -> bool` — check timer expiry
- `advance(fresh_bid)` — transition to next stage with updated bid
- `current_limit_price() -> f64` — compute price for current stage

Three-stage degrade path with timers:
- Stage 1: `BID + k×tick` (3s timer)
- Stage 2: `BID` (3s timer)
- Stage 3: `BID - max_slippage` (10s timer)

**Critic fix:** Partial fill on Stage 3 → loop back to Stage 1 with fresh BID, don't immediately circuit-break.

**Module structure:**
```
engine/src/options/staged_ladder/
├── mod.rs          # Re-exports + original StagedLadder
├── state.rs        # StagedExitLadder + ExitStage enum
└── tests.rs        # Tests for stateless calculator
```

**Pitfall:** When adding `state.rs`, declare `pub mod state;` once in `mod.rs`. Two `mod state;` declarations cause `E0428: defined multiple times`.

## Options Paper Executor

`OptionsPaperExecutor` fills against observed bid/ask from tape (not theoretical prices).

### Fill Logic
```rust
pub async fn try_fill(
    &mut self,
    position_id: i64,
    observed_bid: f64,
    observed_ask: f64,
    timestamp: DateTime<Utc>,
) -> Result<Option<FillResult>>
```

- `observed_bid >= limit_price` → fill at `min(observed_bid, limit_price)`
- Else check `should_advance(now)` → advance ladder if timer expired
- On fill: record to `option_fills` table, clear ladder

### DB Table
```sql
CREATE TABLE option_fills (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    position_id   INTEGER NOT NULL,
    stage         TEXT    NOT NULL,
    price         REAL    NOT NULL,
    quantity      REAL    NOT NULL,
    timestamp     INTEGER NOT NULL
);
```

### Poll Loop Pattern
```rust
loop {
    let (observed_bid, observed_ask) = get_market_data();
    if let Some(fill) = executor.try_fill(position_id, observed_bid, observed_ask, now)? {
        break; // Fill recorded, ladder cleared
    }
    sleep(100ms); // Ladder auto-advances if timer expired
}
```

## Circuit Breaker

Triggers:
- Stage3Timeout (10s with unfilled orders)
- ConsecutiveLosses (configurable threshold, e.g., 3)
- AbnormalVolatility (2x baseline IV)

State:
- `triggered: bool`
- `trigger_time: Option<DateTime<Utc>>`
- `halt_duration: Duration`
- `consecutive_losses: u32`

Methods:
- `trigger(reason)` → emits ExitSignal with priority 2
- `record_loss()` / `record_win()` → tracks consecutive losses
- `check_volatility(current_iv, baseline_iv)` → triggers on 2x
- `can_resume()` → checks if halt duration elapsed
- `reset()` → clears all state

## Hardcoded Overrides

Non-optimizable risk layer (in code, not config):
- DTE < 7 days → force exit
- Delta drift outside [0.15, 0.70] → exit
- Earnings blackout 2 days before → block entry

## Trailing Stop with Hysteresis

- Trails price movement (calls up, puts down)
- Hysteresis: requires 0.5 × ATR recovery to re-arm after trigger
- Prevents whipsaw churn through spreads
