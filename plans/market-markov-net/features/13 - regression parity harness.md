# Feature 13 — Regression / Parity Harness

**Depends on:** 07, 08
**Goal:** Prove the production Rust pipeline matches the Colab backtest — the gate that unlocks live trading.

**Status:** Implemented. See `engine/src/parity.rs`, `engine/tests/parity_harness.rs`,
`tests/fixtures/parity_golden_168h.json`, and the live-mode guard in
`engine::config::verify_parity_marker`.

## Requirements

- Feed an identical 7-day historical candle set through the Rust feature + inference + logic path and through the Colab reference outputs.
- Assert parity on: computed features, model predictions, and strategy entry/exit timestamps + directions.
- Produce a parity report; set a `parity_verified` flag/artifact that the live-mode guard checks.

## Technical Implementation Steps

1. Export reference artifacts from Colab (features, predictions, signals) for a 7-day window → `/tests/fixtures`.
2. `/tests` harness: run the Rust pipeline offline over the same candles (feature computation + a recorded/replayed inference response or the live service).
3. Diff features (tolerance ~1e-6), predictions, and signal timestamps; emit a report.
4. On full parity, write the `parity_verified` marker consumed by Feature 09's live guard.

## Acceptance Criteria

- [x] Feature parity within tolerance across the 7-day window.
  - Verified by `engine/tests/parity_harness.rs::seven_day_parity_check_passes_within_tolerance` and `engine/src/parity::tests::run_parity_passes_on_self_consistent_fixture`.
  - Tolerance: 1e-6.
- [x] Prediction parity within tolerance.
  - Verified by the same test (`predictions` stage).
- [x] Entry/exit timestamps + directions match Colab exactly.
  - Verified by the `signals` stage (exact match on `Position::{Flat,Long,Short}` per candle).
  - Timestamp alignment is checked in the harness; misalignment fails the run.
- [x] Parity report generated; `parity_verified` marker written only on success.
  - `ParityReport` printed to stdout (cargo test) and serialized via the marker.
  - `engine::parity::write_marker` writes `parity_verified.json` on Pass; the live-mode guard (`engine::config::verify_parity_marker`) reads it and rejects stale/missing markers.

## Live-mode gate integration

`engine::config::Config::from_env` now requires a fresh marker
(`PARITY_MARKER_PATH`, default `parity_verified.json`) when
`TRADING_MODE=live`. The marker must be younger than `PARITY_MAX_AGE_SECS`
(default 7 days = 604800 seconds). The engine refuses to start otherwise.

