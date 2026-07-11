# tests/

Parity and regression harness fixtures for MarketMarkovNet.

## Status

**Feature 13 placeholder.** The full parity harness lands in
`../plans/market-markov-net/features/13 - regression parity harness.md`.

## What goes here

- `fixtures/*.parquet` — golden feature inputs (gitignored, large).
- `fixtures/*.h5` — golden feature inputs, alternate format (gitignored).
- `fixtures/*.json` — small reference outputs (committed).
- Parity test runners (Rust + Python) that compare the Rust feature pipeline
  output against the golden fixtures bit-for-bit.

## Why it matters

The Rust feature pipeline must reproduce the Colab pandas feature engineering
**bit-for-bit**: `log_return`, `TR`/`ATR(72)`, typical price, `VWAP`,
`vwap_dev`, rolling z-score windows, and the 200-hour SMA regime filter.

Until the parity harness reports a clean run, `TRADING_MODE=live` is blocked
by the engine startup check.

## Ignore policy

Large binary fixtures (`*.parquet`, `*.h5`) are covered by `.gitignore`. Small
JSON reference outputs are committed.
