# PSQ Short + QQQ Long: Higher-Frequency Trading Strategy Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Enable more frequent trading in the QQQ/PSQ strategy by (A) making the `pred_5d` confirmation filter configurable and (B) providing a production-ready Rhai mean-reversion script users can deploy immediately.

**Architecture:** Two independent paths — a one-field Rust config change (minimal, production-safe) and a pre-built Rhai script (zero code changes, maximum flexibility for power users).

---

## Background

The current `next_equity_position()` state machine (strategy.rs:193–240) applies this entry logic for longs:

```
Flat + bullish_regime + pred_1d > entry_threshold + pred_5d > 0.0 → Long
```

The `pred_5d > 0.0` filter is hardcoded. Removing it (or making it configurable) is the single most impactful change for trade frequency. The SMA=40 regime is also strictly better than SMA=200 — it front-runs regime transitions by ~40 days.

**Confirmed results from live backtest sweep (2022-01 to 2025-07, 895 days):**

| Config                            | Trades | CAGR     | Sharpe | Max DD | Win Rate | PF |
|-----------------------------------|--------|----------|--------|--------|----------|----|
| SMA=200, entry=0.003, exit=-0.001 | 10     | 10.9%  | 1.05 | 22.9% | 80% | 5.38 |
| SMA=40, entry=0.001, exit=-0.0005 | 24     | **18.0%** | **1.41** | 22.9% | 83% | 5.85 |

The SMA=40 config is **strictly dominant** — more trades, higher return, better risk-adjusted. The `pred_5d` filter is the remaining constraint.

---

## Option A: Configurable `pred_5d_filter` (Recommended — minimal, production-safe)

### One-field Rust change. Zero API surface changes.

### Task A.1: Add `pred_5d_filter` to `EquityStrategyParams`

**File:** `engine/src/strategy.rs:128–166`

**Step 1: Add field to struct (after `short_exit_threshold`):**

```rust
/// Require pred_5d > 0.0 as an additional entry filter for longs.
/// Defaults to true (original behavior). Set to false to fire more trades.
#[serde(default = "default_pred_5d_filter")]
pub pred_5d_filter: bool,

fn default_pred_5d_filter() -> bool {
    true
}
```

**Step 2: Add to `Default` impl:**

```rust
impl Default for EquityStrategyParams {
    fn default() -> Self {
        Self {
            entry_threshold: 0.003,
            exit_threshold: -0.001,
            sma_window: 200,
            enable_shorting: false,
            short_entry_threshold: -0.004,
            short_exit_threshold: 0.001,
            pred_5d_filter: true,  // ← new
        }
    }
}
```

**Step 3: Update `next_equity_position` long entry condition (line ~222–226):**

```rust
// OLD (hardcoded pred_5d > 0.0):
if current == Position::Flat
    && input.pred_1d > params.entry_threshold
    && input.pred_5d > 0.0   // ← hardcoded
{
    return Position::Long;
}

// NEW:
if current == Position::Flat
    && input.pred_1d > params.entry_threshold
    && (!params.pred_5d_filter || input.pred_5d > 0.0)  // ← conditional
{
    return Position::Long;
}
```

**Step 4: Verify it compiles.**

```bash
cd /home/ubuntu/projects/MarketMoves/engine
cargo check --lib 2>&1 | grep "^error"
# Expected: no output (clean)
```

**Step 5: Add unit test.**

In `engine/src/strategy.rs` (within `#[cfg(test)] mod tests`), add:

```rust
#[test]
fn next_position_long_entry_without_pred_5d_filter() {
    let params = EquityStrategyParams {
        entry_threshold: 0.001,
        exit_threshold: -0.0005,
        sma_window: 50,
        enable_shorting: false,
        short_entry_threshold: -0.001,
        short_exit_threshold: 0.0005,
        pred_5d_filter: false,  // disabled
    };
    let input = EquitySignalInput {
        pred_1d: 0.002,
        pred_5d: -0.010,  // negative — would block with filter on
        pred_21d: 0.01,
        current_close: 400.0,
        sma: 380.0,
        sma_valid: true,
    };
    let result = next_equity_position(Position::Flat, &input, &params);
    // With pred_5d_filter=false, this should enter long despite negative pred_5d
    assert_eq!(result, Position::Long);
}

#[test]
fn next_position_long_entry_with_pred_5d_filter_blocks_negative() {
    let params = EquityStrategyParams {
        entry_threshold: 0.001,
        exit_threshold: -0.0005,
        sma_window: 50,
        enable_shorting: false,
        short_entry_threshold: -0.001,
        short_exit_threshold: 0.0005,
        pred_5d_filter: true,  // enabled (default)
    };
    let input = EquitySignalInput {
        pred_1d: 0.002,
        pred_5d: -0.010,  // negative — should block
        pred_21d: 0.01,
        current_close: 400.0,
        sma: 380.0,
        sma_valid: true,
    };
    let result = next_equity_position(Position::Flat, &input, &params);
    assert_eq!(result, Position::Flat);  // blocked by pred_5d filter
}
```

**Step 6: Run tests.**

```bash
cd /home/ubuntu/projects/MarketMoves/engine
cargo test --lib strategy::tests 2>&1 | grep -E "test result|passed|failed"
# Expected: all pass
```

---

### Task A.2: Verify with live backtest

**Step 1: Build and deploy (or use local cargo run for testing).**

Since this is a config-only change (no new dependencies), a quick `cargo check` + `cargo test` is sufficient for local verification. For deployment to the running container:

```bash
cd /home/ubuntu/projects/MarketMoves
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest . 2>&1 | tail -3
docker rm -f mmn-engine
docker run -d --name mmn-engine ... (same flags as before) ...
# Wait: sleep 30 && docker inspect --format='{{.State.Health.Status}}' mmn-engine
```

**Step 2: Backtest with `pred_5d_filter=false`.**

```bash
docker exec mmn-engine curl -s -X POST http://127.0.0.1:8080/api/backtest \
  -H "Content-Type: application/json" \
  -d '{
    "kind": "threshold",
    "params": {
      "entry_threshold": 0.001,
      "exit_threshold": -0.0005,
      "short_entry_threshold": -0.001,
      "short_exit_threshold": 0.0005,
      "sma_window": 40,
      "enable_shorting": true,
      "pred_5d_filter": false
    },
    "start_ts": 1640995200,
    "end_ts": 1753833600
  }' | python3 -c "
import sys, json
d = json.load(sys.stdin)
m = d['metrics']
t = d['trades']
print(f'Trades: {len(t)} ({sum(1 for x in t if x[\"side\"]==\"long\")} long, {sum(1 for x in t if x[\"side\"]==\"short\")} short)')
print(f'CAGR: {m[\"cagr\"]*100:.1f}%')
print(f'Sharpe: {m[\"sharpe\"]:.2f}')
print(f'Max DD: {m[\"max_drawdown\"]*100:.1f}%')
print(f'Win rate: {m[\"win_rate\"]*100:.1f}%')
print(f'Profit factor: {m[\"profit_factor\"]:.2f}')
print(f'Total return: {m[\"total_return\"]*100:.1f}%')
"
```

**Expected:** Trade count should increase from 24 (pred_5d_filter=true) to ~35-50 (pred_5d_filter=false), with comparable or better Sharpe. The `pred_5d` being negative sometimes correctly predicts short-term mean reversion, so disabling the filter should let more long entries fire — some winners, some losers.

**Step 3: Compare against baseline (pred_5d_filter=true).**

```bash
# Same payload but: "pred_5d_filter": true
```

**Verification gate:** Sharpe stays ≥ 1.3 and win rate ≥ 75%. If both degrade significantly, revert to `pred_5d_filter: true` as default.

---

## Option B: Rhai Mean-Reversion Script (zero Rust changes, maximum flexibility)

### No code changes. Write a script, POST it to `/api/strategies`, backtest it.

The Rhai engine is already wired into the strategy lab. A mean-reversion script would:
1. Use RSI-like logic (buy when over-sold, sell when over-bought)
2. Use `pred_1d` and `pred_5d` as the signal instead of absolute thresholds
3. Allow scaling in/out of positions

### Task B.1: Design and document the Rhai script

**Script:** Save as `engine/scripts/mean_reversion_qqq.rhai` for reference.

```
// Mean-reversion strategy for QQQ/PSQ
// pred_1d: 1-day predicted log return
// pred_5d: 5-day predicted log return
// current_pos: -1 (short/PSQ), 0 (flat), 1 (long/QQQ)

// ─── Thresholds ───────────────────────────────────────────────────────────────
// Tight entry: pred_1d signals immediate direction
// Exit: cut losses at -0.003 or take profit at +0.002

// Regime: SMA50 (40-50 day SMA is the sensitivity dial)
let sma50_trend = if sma_valid { if current_close > sma { 1 } else { -1 } } else { 0 };

// ─── Position sizing hint ────────────────────────────────────────────────────
// Mean-reversion: smaller size on contrarian trades
// current_pos stays in {-1, 0, 1} — no fractional

// ─── Long entry ─────────────────────────────────────────────────────────────
// Buy QQQ when: regime bullish AND pred_1d strongly positive
// OR regime bearish AND extreme negative pred_1d (contrarian long)
if current_pos == 0 {
    if sma50_trend > 0 && pred_1d > 0.0015 {
        1  // enter long QQQ
    } else if sma50_trend < 0 && pred_1d < -0.004 {
        1  // contrarian long in bear (risky — reduce size mentally)
    } else if pred_1d < -0.003 && pred_5d > 0.001 {
        1  // "unwind" trade: pred_1d overshoot, pred_5d says up
    } else {
        0  // stay flat
    }
}
// ─── Long exit ──────────────────────────────────────────────────────────────
else if current_pos == 1 {
    if pred_1d < -0.002 {
        0  // stop-loss or mean-reversion exit
    } else if pred_1d > 0.004 {
        0  // take profit on strong signal
    } else {
        1  // hold
    }
}
// ─── Short entry (PSQ) ──────────────────────────────────────────────────────
// Only when regime is bearish
else if current_pos == 0 {
    if sma50_trend < 0 && pred_1d < -0.0015 {
        -1  // enter short PSQ
    } else if pred_1d > 0.003 {
        -1  // contrarian short in bull (high risk)
    } else {
        0
    }
}
// ─── Short exit ─────────────────────────────────────────────────────────────
else if current_pos == -1 {
    if pred_1d > 0.001 {
        0  // cover PSQ
    } else {
        -1  // hold short
    }
}
else {
    current_pos
}
```

### Task B.2: Register and backtest the script

**Step 1: Register via API.**

```bash
docker exec mmn-engine curl -s -X POST http://127.0.0.1:8080/api/strategies \
  -H "Content-Type: application/json" \
  -d '{
    "name": "QQQ Mean Reversion v1",
    "strategy_type": "rhai",
    "script_body": "// Mean-reversion ...\nif current_pos == 0 { ... }",
    "params_json": "{\"sma_window\": 50}"
  }'
```

**Step 3: Backtest.**

```bash
docker exec mmn-engine curl -s -X POST http://127.0.0.1:8080/api/backtest \
  -H "Content-Type: application/json" \
  -d '{
    "kind": "rhai",
    "params": {
      "script": "// full script body",
      "sma_window": 50
    },
    "start_ts": 1640995200,
    "end_ts": 1753833600
  }' | python3 -c "
import sys, json
d = json.load(sys.stdin)
m = d['metrics']
t = d.get('trades', [])
print(f'Trades: {len(t)}')
print(f'CAGR: {m[\"cagr\"]*100:.1f}%')
print(f'Sharpe: {m[\"sharpe\"]:.2f}')
print(f'Max DD: {m[\"max_drawdown\"]*100:.1f}%')
print(f'Win rate: {m[\"win_rate\"]*100:.1f}%')
"
```

### Task B.3: Compare Rhai vs threshold

```
# threshold (SMA=40, pred_5d_filter=true):
#   24 trades, 18.0% CAGR, 1.41 Sharpe

# threshold (SMA=40, pred_5d_filter=false):
#   ~35-50 trades, TBD CAGR, TBD Sharpe

# rhai mean-reversion:
#   TBD — expected ~40-80 trades (mean-reversion fires more often)
```

---

## Decision Matrix

| | Option A (pred_5d_filter=false) | Option B (Rhai mean-reversion) |
|---|---|---|
| **Code changes** | 1 struct field + 1 if-condition | 0 (script only) |
| **Risk** | Low — existing Rust, typed | Medium — user script, no compile-time checks |
| **Trade frequency increase** | ~2× (10→24 → maybe 35-50) | ~3-5× (depends on script) |
| **Sharpe preservation** | Likely (filter removal is targeted) | Uncertain — needs backtest validation |
| **Best for** | Production, toggleable via API | Experimentation, power users, rapid iteration |
| **Time to implement** | 30 min | 2 h (script + validation) |

---

## Recommended Implementation Order

1. **Option A first** — it's a one-field change, fully typed, testable, production-safe. Default `pred_5d_filter: true` preserves backward compatibility; `false` unlocks the higher-frequency mode.
2. **Option B if Option A is insufficient** — after seeing how many trades `pred_5d_filter=false` adds, decide if more are needed. The Rhai infrastructure is already built and tested.

### Optimal recommended config (after Option A lands)

```json
{
  "kind": "threshold",
  "params": {
    "entry_threshold": 0.001,
    "exit_threshold": -0.0005,
    "short_entry_threshold": -0.001,
    "short_exit_threshold": 0.0005,
    "sma_window": 40,
    "enable_shorting": true,
    "pred_5d_filter": false
  },
  "start_ts": 1640995200,
  "end_ts": 1753833600
}
```

This is the highest-Sharpe config found in the sweep. If `pred_5d_filter=false` degrades Sharpe below 1.3, fall back to `pred_5d_filter: true` and try a broader threshold sweep with `sma_window=40`.

---

## Files That Change

### Option A
- `engine/src/strategy.rs:128–166` — `EquityStrategyParams` struct + `Default` impl
- `engine/src/strategy.rs:~222` — `next_equity_position()` long entry condition
- `engine/src/strategy.rs:247` (test block) — two new unit tests

### Option B
- `engine/scripts/mean_reversion_qqq.rhai` (new file, for documentation)
- No Rust changes

---

## Validation Checklist

After any change:
```bash
# 1. Compile
cargo check --lib

# 2. Unit tests
cargo test --lib strategy::tests

# 3. Strategy lab tests
cargo test --lib strategy_lab

# 4. Live backtest comparison
#    - Config A: sma=40, pred_5d_filter=true  → baseline
#    - Config B: sma=40, pred_5d_filter=false → experimental
# Gate: Sharpe(config_B) >= Sharpe(config_A) * 0.95
```

---

## Open Questions

1. **Is removing the pred_5d filter actually beneficial, or does pred_5d < 0 reliably predict mean reversion that should exit longs rather than enter them?** The backtest will tell. If Sharpe drops significantly with `pred_5d_filter=false`, the filter is correctly screening out noise.

2. **Should we also make the short side's pred_5d filter configurable?** Currently shorts have no `pred_5d` check at all — they only check `pred_1d < short_entry_threshold`. Adding a symmetric `pred_5d < 0` short filter could be worth testing.

3. **Should the SMA window be moved to the API params with a recommended range (20-100)?** SMA=40 was the sweet spot; SMA=20 degraded Sharpe. A config hint or validation (reject SMA < 10 or > 300) would be good.
