# tests/

Parity and regression harness fixtures for MarketMarkovNet.

## Status

**Feature 13 implemented** — see
`../plans/market-markov-net/features/13 - regression parity harness.md`.

The harness lives in `engine/src/parity.rs` and is exercised end-to-end by
`engine/tests/parity_harness.rs`. Per-component parity tests sit alongside
in `engine/tests/{feature,strategy,exec}_parity.rs`.

## What goes here

- `fixtures/*.json` — small committed reference outputs (currently
  `parity_golden_168h.json`, 7-day hourly fixture).
- `fixtures/*.parquet` — large golden inputs (gitignored).
- `fixtures/*.h5` — alternate-format golden inputs (gitignored).
- Parity test runners (Rust) that compare the Rust feature pipeline
  output against the golden fixtures bit-for-bit.

## How to run

```bash
cargo test --test parity_harness           # end-to-end 7-day parity
cargo test --lib parity::                 # unit tests for the harness
cargo clippy --all-targets --all-features  # lint
```

## What gets checked

1. **Feature parity** — `log_return`, `atr_72`, `vwap_dev` per candle, within
   tolerance 1e-6.
2. **Prediction parity** — recorded `pred_1h/4h/24h` per candle, within
   tolerance 1e-6.
3. **Signal parity** — position state (Flat / Long / Short) per candle, exact
   match.
4. **Marker** — on success, writes `parity_verified.json` containing
   `verified_at`, `fixture_sha256`, `candles_compared`, `max_abs_error`,
   `tolerance`. Consumed by the live-mode guard in
   `engine::config::Config::from_env`.

## Why it matters

The Rust feature pipeline must reproduce the Colab pandas feature engineering
**bit-for-bit**: `log_return`, `TR`/`ATR(72)`, typical price, `VWAP`,
`vwap_dev`, rolling z-score windows, and the 200-hour SMA regime filter.

Until the parity harness reports a clean run (and the marker is fresh —
default 7 days), `TRADING_MODE=live` is blocked by the engine startup
check (`engine::config::verify_parity_marker`).

## Replacing the placeholder

The committed `parity_golden_168h.json` is a Rust-generated placeholder that
encodes a deterministic synthetic price walk. Before relying on the live
gate, replace it with a real Colab export:

1. Run the same 7-day window in the Colab training notebook and export
   per-candle features, predictions, and signals to JSON.
2. Save as `tests/fixtures/parity_golden_168h.json` (or a new file and
   update the path in `engine/tests/parity_harness.rs`).
3. Re-run `cargo test --test parity_harness` to confirm parity.
4. Re-run the harness to refresh the marker.

## Ignore policy

Large binary fixtures (`*.parquet`, `*.h5`) are covered by `.gitignore`. Small
JSON reference outputs are committed.
