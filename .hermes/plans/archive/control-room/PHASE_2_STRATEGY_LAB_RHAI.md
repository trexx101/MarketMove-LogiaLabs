# Phase 2 — Strategy Lab & Rhai Integration

**Goal**: Build a strategy lab where the user can define, backtest, and compare trading
strategies without touching Rust code. Two modes: Standard (parameter sliders mapped to
`EquityStrategyParams`) and Advanced (Rhai scripting sandbox). Backtest runs as Rust
replay over historical predictions — guaranteeing parity with the live executor.

**Estimated effort**: ~3 weeks
**Can deploy independently**: Yes — Strategy Lab is additive to the dashboard.
**Depends on**: Phase 0 (DB tables, Svelte scaffold), Phase 1 (App.svelte shell, stores)

---

## 2.1 Backtest Engine (`engine/src/strategy_lab/`)

### Design
- Load historical `equity_predictions` + `equity_candles` from the DB into memory.
- Replay them bar-by-bar through the Rust strategy function (`next_equity_position` or
  Rhai script), simulating the PaperExecutor's fill logic.
- Compute metrics: CAGR, Sharpe, Sortino, max drawdown, win rate, profit factor, trade count.
- Return equity curve as an array of `{ts, equity}` points.

### New module structure
```
engine/src/strategy_lab/
  mod.rs              — public API: run_backtest()
  replay.rs           — historical replay engine
  metrics.rs          — performance metric calculations
  rhai_plugin.rs      — Rhai script evaluation sandbox
```

### Core types
```rust
// engine/src/strategy_lab/mod.rs
use crate::strategy::{Position, EquityStrategyParams, EquitySignalInput};

/// What kind of strategy to backtest.
pub enum StrategyKind {
    /// Use the built-in threshold strategy with custom params.
    Threshold(EquityStrategyParams),
    /// Use a user-defined Rhai script.
    Rhai(String),
}

/// Backtest request from the API.
pub struct BacktestRequest {
    pub strategy_id: Option<String>,
    pub kind: StrategyKind,
    pub start_ts: i64,
    pub end_ts: i64,
}

/// Backtest result returned to the API.
pub struct BacktestResult {
    pub equity_curve: Vec<(i64, f64)>,  // (timestamp, cumulative equity)
    pub metrics: BacktestMetrics,
    pub trades: Vec<BacktestTrade>,
}

#[derive(Serialize)]
pub struct BacktestMetrics {
    pub cagr: f64,
    pub sharpe: f64,
    pub sortino: f64,
    pub max_drawdown: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub trade_count: usize,
    pub total_return: f64,
    pub buy_hold_return: f64,  // benchmark
}

#[derive(Serialize)]
pub struct BacktestTrade {
    pub entry_ts: i64,
    pub exit_ts: Option<i64>,
    pub side: String,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub realized_pnl: f64,
}
```

### Replay engine (`engine/src/strategy_lab/replay.rs`)
```rust
/// Load historical predictions + candles, replay through the strategy,
/// simulate fills, and compute the equity curve + metrics.
pub async fn run_backtest(
    pool: &DbPool,
    request: &BacktestRequest,
) -> Result<BacktestResult> {
    // 1. Fetch equity_candles + equity_predictions for [start_ts, end_ts]
    let candles = db::fetch_equity_candles_asc(pool, "QQQ", start_ts, end_ts).await?;
    let predictions = db::fetch_equity_predictions_asc(pool, start_ts, end_ts).await?;

    // 2. Compute SMA series (200-day) from candles
    let sma_series = compute_sma_series(&candles, sma_window);

    // 3. Replay bar-by-bar
    let mut position = Position::Flat;
    let mut entry_price = 0.0;
    let mut equity = 1.0;  // normalized starting equity
    let mut equity_curve = Vec::new();
    let mut trades = Vec::new();
    let mut peak = 1.0;

    for (i, candle) in candles.iter().enumerate() {
        // Build signal input from the prediction at this timestamp
        let pred = predictions.get(i);
        let input = EquitySignalInput {
            pred_1d: pred.map(|p| p.pred_1d).unwrap_or(0.0),
            pred_5d: pred.map(|p| p.pred_5d).unwrap_or(0.0),
            pred_21d: pred.map(|p| p.pred_21d).unwrap_or(0.0),
            current_close: candle.close,
            sma: sma_series[i],
            sma_valid: i >= sma_window,
        };

        // Compute next position based on strategy kind
        let next = match &request.kind {
            StrategyKind::Threshold(params) => {
                crate::strategy::next_equity_position(position, &input, params)
            }
            StrategyKind::Rhai(script) => {
                rhai_plugin::evaluate_rhai_strategy(script, &input, position.as_i64())?
                .into()
            }
        };

        // Simulate fill if position changed
        if next != position {
            // Close existing position
            if position != Position::Flat {
                let pnl = match position {
                    Position::Long => (candle.close - entry_price) / entry_price,
                    Position::Short => (entry_price - candle.close) / entry_price,
                    _ => 0.0,
                };
                equity *= 1.0 + pnl - fee_rate;
                trades.push(BacktestTrade { ... });
            }
            // Open new position
            if next != Position::Flat {
                entry_price = candle.close;
            }
            position = next;
        }

        // Track equity (mark-to-market for open positions)
        let unrealized = match position {
            Position::Long => (candle.close - entry_price) / entry_price,
            Position::Short => (entry_price - candle.close) / entry_price,
            _ => 0.0,
        };
        equity_curve.push((candle.ts, equity * (1.0 + unrealized)));
        peak = peak.max(equity);
    }

    // 4. Compute metrics
    let metrics = metrics::compute(&equity_curve, &trades, buy_hold_return);

    Ok(BacktestResult { equity_curve, metrics, trades })
}
```

### Metrics (`engine/src/strategy_lab/metrics.rs`)
- `compute(equity_curve, trades, buy_hold) -> BacktestMetrics`
- CAGR: `(final_equity / initial) ^ (252 / n_days) - 1`
- Sharpe: `mean(daily_returns) / std(daily_returns) * sqrt(252)`
- Sortino: same but with downside deviation
- Max drawdown: `min(running_peak_to_trough)` from the equity curve
- Win rate: `winning_trades / total_trades`
- Profit factor: `sum(wins) / abs(sum(losses))`

---

## 2.2 Rhai Plugin Sandbox (`engine/src/strategy_lab/rhai_plugin.rs`)

### Design
- Pure-Rust scripting engine — no C bindings, no unsafe.
- Sandboxed: scripts cannot access filesystem, network, or environment.
- Execution limits: max expression depth 50, max operations 10,000.
- Input: `EquitySignalInput` fields pushed into Rhai scope.
- Output: integer (1=Long, 0=Flat, -1=Short).

### Implementation
```rust
// engine/src/strategy_lab/rhai_plugin.rs
use rhai::{Engine, Scope, EvalAltResult};
use crate::strategy::{Position, EquitySignalInput};

pub fn evaluate_rhai_strategy(
    script: &str,
    input: &EquitySignalInput,
    current_pos: i64,
) -> Result<i64, Box<EvalAltResult>> {
    let mut engine = Engine::new();

    // Strict sandboxing
    engine.set_max_expr_depths(50, 50);
    engine.set_max_operations(10_000);

    let mut scope = Scope::new();
    scope.push("pred_1d", input.pred_1d);
    scope.push("pred_5d", input.pred_5d);
    scope.push("pred_21d", input.pred_21d);
    scope.push("current_close", input.current_close);
    scope.push("sma", input.sma);
    scope.push("sma_valid", input.sma_valid);
    scope.push("current_pos", current_pos);

    engine.eval_with_scope::<i64>(&mut scope, script)
}

impl From<i64> for Position {
    fn from(v: i64) -> Self {
        Position::from_i64(v)
    }
}
```

### Example Rhai strategy (shown to the user in the UI)
```rhai
// Strategy: Asymmetric Momentum with VIX filtering
if sma_valid && current_close > sma {
    // Bullish Macro Regime
    if pred_1d > 0.003 && pred_5d > 0.0 {
        return 1;  // Enter Long
    } else if pred_1d < -0.001 {
        return 0;  // Exit to Flat
    }
} else {
    // Bearish Macro Regime
    if pred_1d < -0.005 {
        return -1; // Enter Short
    } else if pred_1d > 0.002 {
        return 0;  // Exit Short
    }
}
return current_pos;  // Hold current position
```

---

## 2.3 API Endpoints (`engine/src/api/strategy_lab.rs`, `engine/src/api/backtest.rs`)

### `POST /api/backtest`
```json
// Request:
{
  "strategy_id": "uuid-or-null",
  "kind": "threshold" | "rhai",
  "params": { ... } | { "script": "..." },
  "start_ts": 1600000000,
  "end_ts": 1700000000
}

// Response:
{
  "equity_curve": [[ts, equity], ...],
  "metrics": {
    "cagr": 0.15,
    "sharpe": 1.2,
    "sortino": 1.5,
    "max_drawdown": -0.10,
    "win_rate": 0.54,
    "profit_factor": 1.8,
    "trade_count": 42,
    "total_return": 0.35,
    "buy_hold_return": 0.22
  },
  "trades": [...]
}
```

### `GET /api/strategies`
Returns all saved strategy configs from `strategy_configs` table.

### `POST /api/strategies`
Saves a new strategy config (threshold params or Rhai script) to `strategy_configs`.

### Route registration in `api/mod.rs`
```rust
.route("/api/backtest", post(backtest::handle_backtest))
.route("/api/strategies", get(strategy_lab::handle_list_strategies))
.route("/api/strategies", post(strategy_lab::handle_save_strategy))
```

### Files to create
- **CREATE** `engine/src/strategy_lab/mod.rs`
- **CREATE** `engine/src/strategy_lab/replay.rs`
- **CREATE** `engine/src/strategy_lab/metrics.rs`
- **CREATE** `engine/src/strategy_lab/rhai_plugin.rs`
- **CREATE** `engine/src/api/backtest.rs`
- **CREATE** `engine/src/api/strategy_lab.rs`
- **MODIFY** `engine/src/api/mod.rs` — register new routes, add `mod backtest; mod strategy_lab;`
- **MODIFY** `engine/src/lib.rs` — add `pub mod strategy_lab;`

---

## 2.4 Strategy Lab UI (`frontend/src/views/StrategyLab.svelte`)

### Layout
```
+------------------------------------------------------------------+
| Strategy Lab                                                     |
+------------------------------------------------------------------+
| [Standard Mode] [Advanced Mode]         Date range: [Start] [End]|
+------------------------------------------------------------------+
| Standard Mode:                                                   |
|   entry_threshold:    [slider 0.001--0.010]   [0.003]            |
|   exit_threshold:      [slider -0.005--0.000] [-0.001]            |
|   sma_window:          [slider 50--300]        [200]              |
|   enable_shorting:     [checkbox]                                 |
|   short_entry_thresh:  [slider -0.010--0.000] [-0.004]            |
|   short_exit_thresh:   [slider 0.000--0.005]  [0.001]            |
|                                                                  |
| Advanced Mode (Rhai):                                            |
|   +--- Monaco Editor ---------------------+                      |
|   | if sma_valid && current_close > sma { |                      |
|   |   if pred_1d > 0.003 { return 1; }    |                      |
|   | }                                     |                      |
|   +---------------------------------------+                      |
|                                                                  |
| Strategy name: [___________]   [Save Strategy]                   |
+------------------------------------------------------------------+
| [Run Backtest]                                                   |
+------------------------------------------------------------------+
|  Equity Curve (uPlot)                                           |
|  --- Strategy    --- Buy & Hold                                  |
|  +-----+-----+-----+-----+-----+-----+-----+                  |
|  |     |     |     |     |     |     |     |     |              |
|  +-----+-----+-----+-----+-----+-----+-----+                  |
+------------------------------------------------------------------+
| Metrics Table:                                                   |
| CAGR: 15%   Sharpe: 1.2   MDD: -10%   Win Rate: 54%             |
+------------------------------------------------------------------+
| A/B Comparison: [Load Strategy B v]  [Compare]                   |
| (Overlays second equity curve on the chart)                     |
+------------------------------------------------------------------+
```

### Components
```
frontend/src/
  views/StrategyLab.svelte
  lib/components/
    ParamSlider.svelte       # Range slider + numeric input bound to a param
    RhaiEditor.svelte        # Monaco editor wrapper (lazy-loaded)
    EquityCurveChart.svelte  # uPlot line chart for backtest results
    MetricsTable.svelte      # Summary metrics display
    ABComparison.svelte      # Two-strategy overlay selector
```

### Monaco editor integration
- Install `monaco-editor` as a dev dependency.
- Lazy-load Monaco only when Advanced Mode is selected (code-split).
- Syntax highlighting for Rhai (use JavaScript mode as closest approximation — Rhai syntax
  is C-like, close enough).

---

## 2.5 Test requirements

### Backend
- Unit test: `replay.rs` with a small synthetic dataset (10 candles + predictions).
- Unit test: Rhai sandbox blocks infinite loops (max_operations triggers error).
- Unit test: Rhai sandbox returns correct position for the example script.
- Unit test: metrics calculations (CAGR, Sharpe, MDD) on a known equity curve.
- Integration test: `POST /api/backtest` returns valid result with real DB data.
- Integration test: `POST /api/strategies` saves and `GET /api/strategies` retrieves.

### Frontend
- Manual: Standard mode — adjust sliders, run backtest, see equity curve + metrics.
- Manual: Advanced mode — edit Rhai script, run backtest, see results.
- Manual: Save strategy, reload page, strategy appears in the saved list.
- Manual: A/B comparison — select two strategies, overlay curves.
- Manual: Invalid Rhai script shows error message (not a crash).

---

## 2.6 Risk notes

- **Backtest parity**: The replay engine must use the EXACT same `next_equity_position`
  logic as live trading. If the strategy logic diverges, backtests will be misleading.
  Mitigation: the Threshold strategy kind calls the same `crate::strategy::next_equity_position`
  function — guaranteed parity. Rhai scripts are user-defined and inherently custom —
  document this clearly.
- **Rhai version compatibility**: Rhai 1.19 API may differ from the design doc's assumed
  version. Check the Rhai docs for `set_max_expr_depths` and `set_max_operations` signatures.
- **DB read volume**: Backtesting years of daily data is ~250 rows/year — trivial for SQLite.
  No performance concern.
- **Monaco bundle size**: Monaco adds ~2MB to the JS bundle. Mitigation: lazy-load only
  when Advanced Mode is selected, use Vite's dynamic import.
