# Opus 5 Code Review — Pass 1a: Strategy & Config

Date: 2026-08-13. Model: `openrouter/anthropic/claude-opus-5` via OmniRoute proxy.
Cost: $0.51 (23,827 in / 8,000 out tokens).

## High-severity findings

### 1. Invalid SMA allows shorts during warmup

When `sma_valid == false` (first 40 candles, or any data gap), the `bullish`
flag is `false`, which falls through to the short-entry block in
`next_equity_position()`. The engine can open shorts with zero regime
information. The crypto engine (`next_position`) correctly guards against
this with `if !input.sma_valid { return current; }`.

**Status: NOT YET FIXED.** Gate short entry on `input.sma_valid && input.current_close <= input.sma`.

### 2. Short held into bullish regime never force-exited

The only exit for a short is `pred_1d > short_exit_threshold` (default 0.0005).
If NVDA enters short and price rises above SMA40 but `pred_1d` stays at 0.0004,
the position holds permanently — the bullish branch does `return current;` and
explicitly retains `Short`. This is the most expensive failure mode in the state
machine: shorting into a bull market with no regime-conflict exit.

This was the exact mechanism behind NVDA stuck short for a week while NVDA
rallied from $217 to $224. pred_1d was 0.0 (z-score cold start) so
0.0 > 0.0005 was false, and the bullish regime check never flattens shorts.

**Status: FIXED 2026-08-13 (deleted corrupt position, engine restart with
correct candle data). But the code bug remains — next occurrence will
hit again.**

Fix: add hard regime-conflict exit: `if current == Short && bullish → Flat`.

### 3. No NaN/finite guards

If `pred_1d` is NaN (inference timeout, bad normalization), every comparison
is `false` → step 1 doesn't exit, entry conditions fail → returns `current`
forever with no error. Same for `sma`/`current_close` NaN → `bullish = false`
→ silently in bearish regime.

**Status: NOT YET FIXED.** Add `if !pred_1d.is_finite() || !sma.is_finite() || !close.is_finite() { log_error; return current; }`.

## Medium-severity findings

### 4. Asymmetric confirmation filter

Long entry can require `pred_5d > 0` (if `pred_5d_filter` is enabled), but
short entry has no equivalent filter. Creates asymmetry where the model must
"prove itself" for longs but can short on a single-horizon signal.

**Status: DOCUMENTED.** The config exposes this via `pred_5d_filter`; user
can decide whether to add a symmetric `short_5d_filter` param.

## Other notes

- Entry/exit threshold default values are **reasonable** given the ICs (0.034,
  0.082). The thresholds are small enough to let signal through but the real
  issue is the regime filter + NaN guards, not the numeric values.
- Config validation is adequate for numeric bounds but doesn't catch logic
  errors (e.g., entry < exit for shorts). A misconfigured short_entry > 0
  would cause silent failures.
- The `Position::from_i64/as_i64` mapping handles all valid states but
  invalid i64 values (2, -2, etc.) silently collapse to Flat — no error
  log or healthcheck failure.