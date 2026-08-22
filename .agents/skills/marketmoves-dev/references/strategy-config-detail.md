# Strategy Config Implementation Detail

> Session: 2026-07-31 — runtime strategy config externalization

## Borrow Checker Workaround

The scheduler's `process()` method calls `evaluate_and_execute_strategy()` which
takes `&mut self`. If `self.strategy_params.read().await` is held across that
call, Rust rejects it: "cannot borrow `*self` as mutable because it is also
borrowed as immutable."

Fix: extract `sma_window` into a local variable by scoping the read guard:

```rust
let sma_window = {
    let params = self.strategy_params.read().await;
    params.sma_window
};
let (sma, sma_valid) = strategy::compute_sma(&closes, sma_window);
```

Then inside `evaluate_and_execute_strategy()`, re-acquire:
```rust
let params = self.strategy_params.read().await;
let new_pos = strategy::next_equity_position(current_pos, &input, &params);
drop(params);  // release before any &mut self call
```

## Config::from_env() Flow

`Config` struct has 3 new fields:
- `entry_threshold: f64` — parsed from `ENTRY_THRESHOLD` (default 0.001)
- `exit_threshold: f64` — parsed from `EXIT_THRESHOLD` (default -0.0005)
- `pred_5d_filter: bool` — parsed from `PRED_5D_FILTER` (default false)

Note: `magnitude_threshold` still exists (used elsewhere); `entry_threshold`
is the new separate field. Previously `entry_threshold` was set FROM
`magnitude_threshold` and `exit_threshold` was a formula `-magnitude/3.0`.
Now both are explicit env vars.

## WS Event

```rust
StrategyConfigChange {
    entry_threshold: f64,
    exit_threshold: f64,
    sma_window: usize,
    pred_5d_filter: bool,
    enable_shorting: bool,
    short_entry_threshold: f64,
    short_exit_threshold: f64,
}
```

Broadcast after successful PUT. Frontend can listen via WS to auto-refresh.

## Docker Compose Env Syntax

Negative defaults need care in docker-compose.yml:
```yaml
# Correct — bash-style fallback with leading -
EXIT_THRESHOLD: ${EXIT_THRESHOLD:--0.0005}
SHORT_ENTRY_THRESHOLD: ${SHORT_ENTRY_THRESHOLD:--0.001}
```
The `:-` separator handles the negative number correctly.

## 10-Step Checklist for New Strategy Params

When adding a new field to EquityStrategyParams:
1. `engine/src/strategy.rs` — struct field + serde default fn + Default impl
2. `engine/src/config.rs` — Config field + env parsing + validation + Ok(Self)
3. `engine/src/api/status.rs` — StrategyInfo field + response construction
4. `engine/src/api/strategy_config.rs` — Response + Update structs
5. `engine/src/api/ws.rs` — TelemetryEvent variant
6. `engine/src/api/mod.rs` — AppState construction in router()
7. `engine/src/main.rs` — EquityStrategyParams construction
8. `engine/Dockerfile` — ENV default
9. `deploy/docker-compose.yml` — environment override
10. `.env` (project root) — add the env var with the desired startup default.
    This is the **source of truth** for container restarts — the in-memory
    `Arc<RwLock<EquityStrategyParams>>` is reconstructed from env vars on
    every restart. Without this step, PUT-saved runtime changes are lost
    on any container restart (including the stop+rm+up deploy sequence).