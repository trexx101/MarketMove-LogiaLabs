# Opus 5 Code Review — Full Findings (2026-08-13)

Date: 2026-08-13. Model: `openrouter/anthropic/claude-opus-5` via OmniRoute proxy.
Total cost: $1.04 (89,652 input / 23,750 output tokens) of $10 budget.
5 passes, each ~6-13K tokens input, ~4K tokens output.

## Pass summary

| Pass | Focus | Input | Output | Cost |
|------|-------|-------|--------|------|
| 1a | strategy.rs + config.rs | 23,827 | 8,000 | $0.32 |
| 1b-i | scheduler.rs (restart, ordering, zero-pred) | 11,070 | 3,750 | $0.15 |
| 1b-ii | bridge.rs + exec/paper.rs (ZMQ, paper exec) | 12,146 | 4,000 | $0.16 |
| 2a | equities_v2.rs + equities_features.py (skew) | 22,071 | 4,000 | $0.21 |
| 2b | equity_model.py + parity.rs (blending, parity) | 20,538 | 4,000 | $0.20 |

## CRITICAL findings

### 1. ZMQ REQ socket poisons after first timeout (bridge.rs)

When the ZMQ timeout fires, the `recv()` future is dropped mid-await while
the REQ socket is in "awaiting reply" state. ZMQ REQ enforces strict
send→recv→send→recv lockstep. The socket is never closed or reconnected.
Every subsequent `send()` errors out → all retries fail instantly → the
scheduler can never reach inference again until engine restart.

**Impact:** One inference timeout permanently kills predictions until
restart. Likely cause of 6 days of no predictions.

**Fix:** After timeout, close and recreate the socket before retrying.
Or switch to ZMQ DEALER (no lockstep).

### 2. VIX/TLT joined by array index, not timestamp (equities_v2.rs)

VIX and TLT closes are aligned to QQQ by array index. Any missing VIX/TLT
bar (holiday mismatch, vendor gap) shifts the entire series by one day
silently. The Rust guard degrades to `vec![0.0; n]` (all zeros = "calm
market"), presenting a data outage as a benign regime with no log.

**Fix:** Join on timestamp, not index. Assert full coverage or fail loudly.

### 3. Z-score blending is mathematically wrong (equity_model.py)

Three issues:
1. `_pooled_std` is not a pooled std — concatenates TCN and LGBM raw
   outputs and takes std of the union, including between-model mean offset
   as variance. Correct pooled std is `sqrt((s1² + s2²)/2)`.
2. De-normalization `blend_z * combined_std * atr_ratio` treats `blend_z`
   as unit-variance, but for 0.5/0.5 blend of correlated z-scores,
   `Var = 0.5(1+ρ)` → systematic shrinkage.
3. Scale is non-stationary — `combined_std` changes every bar as buffer
   fills. Same feature window produces different prediction depending on
   call history.

**Fix:** Use training-time label statistics for de-normalization, not live
prediction dispersion.

## HIGH findings

### 4. Short held into bullish regime never force-exited (strategy.rs)

Only exit for short is `pred_1d > short_exit_threshold` (0.0005). If price
rises above SMA40 but pred_1d stays low (z-score cold start → 0.0), position
holds permanently. Bullish branch does `return current;` retaining Short.

**This caused NVDA stuck short for a week while NVDA rallied $217→$224.**

**Fix:** Add `if current == Short && bullish → Flat`.

### 5. Invalid SMA allows shorts during warmup (strategy.rs)

When `sma_valid == false`, `bullish = false`, falls through to short-entry
block. Engine opens shorts with zero regime information. Crypto engine
correctly guards with `if !input.sma_valid { return current; }`.

**Fix:** Gate short entry on `input.sma_valid && input.current_close <= input.sma`.

### 6. No NaN/finite guards (strategy.rs + scheduler.rs)

If `pred_1d` is NaN, every comparison is `false` → position freezes silently.
No error log, no health-check flag. Same for NaN in SMA/close.

**Fix:** Add `if !pred_1d.is_finite() { log_error; return current; }`.

### 7. All-zero predictions accepted silently (scheduler.rs)

`finalize_candle` does zero validation on predictions. An all-zero prediction
(z-score cold start, inference failure) is persisted at info! level as normal,
then fed to strategy which sees `0.0 < entry_threshold` and forces flatten
of an open position on a dead signal.

**Fix:** Check `if pred_1d == 0.0 && pred_5d == 0.0 && pred_21d == 0.0 { warn; skip strategy; }`.

### 8. Engine restart skips intermediate candles (scheduler.rs)

After restart, `last_processed_ts` is `None` (no DB recovery). Scheduler
only processes the single latest candle_ts. If engine was down for 5 days,
the 4 intermediate candles are permanently skipped — no backfill loop.

**Fix:** After restart, query `MAX(candle_ts) FROM equity_predictions` and
process all candles from there to latest.

## MEDIUM findings

### 9. Drawdown divergence: Rust uses `high`, Python uses `close`

Rust `equities_v2.rs` uses rolling max over `highs`. Python
`equities_features.py` uses `np.max(closes[...])`. The Rust comment says
"the deployed QQQ model was trained on the close-based feature" but serving
code now uses `high`. The parity test asserts the skew is present rather
than catching it.

**Fix:** Use `close` in Rust to match training, or retrain with `high`.

### 10. Clipping applied at wrong stage (equities_v2.rs)

Rust clips 4 features on raw values. The notebook's `clip(-5, 5)` was
likely applied to normalized z-scores, not raw features. If so, Rust is
clipping the wrong stage and inference z-scores are effectively unbounded.

**Fix:** Verify which side of `normalize()` the notebook clips. Apply clip
at the same stage in Rust.

### 11. Parity harness never tests Rust (parity.rs)

The `--verify-fixture` mode recomputes features with the same Python
functions and diffs Python against Python. Reports `PARITY OK` even while
the drawdown divergence is live. No Rust test loads the parity fixture and
compares `to_array()` output.

**Fix:** Add a Rust test that loads `equity_feature_parity.json` and
compares Rust feature output against it.

### 12. No backoff in ZMQ retries (bridge.rs)

Retry loop has no sleep between attempts. If `send` fails fast, all retries
burn in microseconds — zero recovery time. Combined with socket-poisoning
bug, system has no resilience to transient inference failures.

## Profitability assessment

- **QQQ at 0.034 IC is marginal.** IC of 0.034 × daily_return_std ≈ 0.034
  × 1.2% ≈ 0.04% expected return per trade. That's 4bps against 15bps fees
  — net negative unless trading less frequently or only on high-conviction
  signals.
- **NVDA at 0.082 IC is viable** — but only if z-score blending bug is
  fixed. Broken blending shrinks predictions, reducing effective IC below
  the reported 0.082.
- **Regime filter (SMA40) likely hurting NVDA.** NVDA in persistent uptrend
  (close > SMA40 for months) → short side rarely fires. Long side requires
  `pred_1d > 0.001` which is very low bar → model is essentially always long
  in bull market = beta exposure with fees.
- **Recommendation:** (a) raise entry thresholds, (b) reduce trade
  frequency, (c) fix blending before evaluating profitability.

## Recommended fix priority

1. ZMQ socket recovery — without this, one timeout kills all predictions
2. Short regime-conflict exit — NVDA stuck short was most visible symptom
3. Z-score blending math — predictions are systematically wrong
4. VIX/TLT timestamp alignment — silent feature corruption
5. NaN/zero prediction guards — prevents trading on dead signals
6. Engine restart backfill — prevents skipped candles after downtime
7. Drawdown `high` vs `close` — train/serve skew
8. Parity test coverage — catch future skew
