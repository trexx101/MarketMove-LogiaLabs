# Feature 12 — Paper-Trading Verification

**Depends on:** 09
**Goal:** Verify the simulation toggle fully bypasses live API calls and that fee/PnL simulation is correct.

## Requirements

- Confirm `TRADING_MODE=paper` issues zero Kraken order requests.
- Validate static fee application (0.15%) and PnL accounting across a sequence of simulated trades.
- Confirm switching to `live` requires explicit config + keys and is otherwise blocked.

## Technical Implementation Steps

1. Add an integration test driving the strategy → executor path in paper mode; assert no outbound order HTTP (mock/spy the Kraken client).
2. Feed a scripted signal sequence; assert resulting trades, fees, and cumulative PnL match expected values.
3. Test the live-mode guard (missing keys / missing parity flag → refuse).

## Acceptance Criteria

- [ ] Test proves no Kraken order calls in paper mode.
- [ ] Simulated fees + PnL match expected fixture values.
- [ ] Live mode is blocked without keys/parity flag.
