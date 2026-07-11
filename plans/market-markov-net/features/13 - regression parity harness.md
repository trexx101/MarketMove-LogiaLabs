# Feature 13 — Regression / Parity Harness

**Depends on:** 07, 08
**Goal:** Prove the production Rust pipeline matches the Colab backtest — the gate that unlocks live trading.

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

- [ ] Feature parity within tolerance across the 7-day window.
- [ ] Prediction parity within tolerance.
- [ ] Entry/exit timestamps + directions match Colab exactly.
- [ ] Parity report generated; `parity_verified` marker written only on success.
