# Feature 09 — Execution Layer (Paper + Kraken)

**Depends on:** 08, 03
**Goal:** Execute target positions via a pluggable executor — a paper simulator (default) or live Kraken REST orders.

## Requirements

- `Executor` trait abstracting order placement.
- `PaperExecutor`: simulates fills at candle close/next open with a static fee (`PAPER_FEE`, 0.15%); no external calls.
- `KrakenExecutor`: places market orders via Kraken REST (Query+Trade key); handles auth signing, nonce, error responses.
- Mode selected by `TRADING_MODE`; live mode is refused unless keys are present.
- Track position and realized/unrealized PnL; persist `trades` and `positions`.
- externalize configuration to allowchange to system variables that may change like fees

## Technical Implementation Steps

1. `engine/src/exec/mod.rs`: define `Executor` trait (`fn set_target_position`).
2. `engine/src/exec/paper.rs`: simulate fills + fee accounting + PnL.
3. `engine/src/exec/kraken.rs`: signed REST order placement (`reqwest`), response parsing, retry/idempotency safeguards.
4. Wire strategy target → executor; persist resulting trades/positions.
5. Guardrails: max position size, refuse live without verified parity flag (ties to Feature 13).

## Acceptance Criteria

- [ ] In `paper` mode no network calls hit Kraken order endpoints; simulated fills + fees recorded.
- [ ] PnL math validated against a hand-computed fixture (incl. 0.15% fee).
- [ ] `KrakenExecutor` signing verified against Kraken's documented scheme (unit test on signature).
- [ ] Live mode blocked without keys / parity flag.
- [ ] `cargo build` + `cargo clippy` pass.
