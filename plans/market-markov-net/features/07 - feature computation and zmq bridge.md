# Feature 07 — Feature Computation & ZMQ Bridge

**Depends on:** 06, 04
**Goal:** Compute the model's input features hourly (matching Colab exactly), normalize Rust-side, and exchange them with the inference service over ZeroMQ.

## Requirements

- Reproduce the Colab feature pipeline bit-for-bit: `log_return`, True Range, `ATR(72)`, typical price, `VWAP`, `vwap_dev`, and **rolling z-score normalization** using the same window sizes as training.
- Apply z-score normalization Rust-side using `norm_stats.json` / rolling stats consistent with training.
- Hourly scheduler aligned to candle-close boundaries triggers computation.
- Serialize the normalized feature window to JSON, REQ to `ZMQ_ENDPOINT`, receive predictions, persist to `predictions` (including `features_json` for parity/drift auditing).

## Technical Implementation Steps

1. `engine/src/features.rs`: port each feature transform; carefully match rolling-window edge handling (min-periods, alignment) to pandas.
2. `engine/src/normalize.rs`: rolling z-score using training-consistent params.
3. `engine/src/bridge.rs`: ZMQ REQ client (timeout + retry), JSON encode/decode.
4. `engine/src/scheduler.rs`: fire on the hour after candle close; assemble the feature window from SQLite.
5. Persist prediction + raw feature snapshot.
6. Create a golden fixture (a slice of candles + expected features exported from Colab) under `/tests` for validation.

## Acceptance Criteria

- [ ] Rust-computed features match the Colab fixture within a tight tolerance (e.g. 1e-6).
- [ ] Hourly scheduler fires once per closed candle.
- [ ] Round-trip ZMQ call returns and persists predictions with `features_json`.
- [ ] ZMQ timeout is handled gracefully (no crash, logged, retried).
- [ ] `cargo build` + `cargo clippy` pass.
