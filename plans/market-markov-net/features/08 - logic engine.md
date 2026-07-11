# Feature 08 — Logic Engine (Hysteresis + Regime State Machine)

**Depends on:** 07
**Goal:** Translate predictions into a target position using the Colab hysteresis + regime-filtered swing logic.

## Requirements

- Entry signal only when **4H and 24H predictions align** and exceed `MAGNITUDE_THRESHOLD` (0.50%).
- **Hysteresis:** once a signal fires, hold the position (sticky / forward-fill semantics) until an opposing or neutral signal — minimizing turnover.
- **Regime filter:** 200-hour SMA of close; longs only allowed in bullish regime (close > SMA200), shorts only in bearish regime (asymmetric).
- Persist state-machine state so restarts resume correctly.

## Technical Implementation Steps

1. `engine/src/strategy.rs`: state machine with states {flat, long, short}; implement ffill-equivalent stickiness.
2. Compute SMA200 from `candles`; derive regime (+1/-1).
3. Signal rule: align 4H+24H, magnitude gate, regime gate → target position.
4. Persist last signal + position to `state`/`positions`.
5. Validate against Colab backtester logic (cells `b_0ifN-sOviQ`, `p7gjxTQjQjWa`) using the golden fixture.

## Acceptance Criteria

- [ ] On the golden fixture, emitted entry/exit signals match the Colab regime-filtered hysteresis backtester exactly (timestamps + direction).
- [ ] No position changes when signals are below threshold or against regime.
- [ ] State persists and resumes across restart.
- [ ] `cargo build` + `cargo clippy` pass.
