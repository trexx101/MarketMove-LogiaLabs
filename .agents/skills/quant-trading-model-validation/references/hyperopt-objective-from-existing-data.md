# Hyperopt objective when the "real" backtester is a placeholder

Situation: a hyperopt/nightly-optimizer pipeline exists in an engine, but the IC
evaluation core was never filled in — the runner stores mock IC, the optimizer's
`evaluate()` returns `None`, or the backtest stub trades fabricated params. This
pattern rebuilds the objective from **data that actually exists in the DB**, so
you never fabricate a backtester or invent predictions.

## Principles
1. **Inventory what's real vs skeleton first.** Grep `run_slot` / `evaluate()` /
   the objective path for mock values (`mock_ic`, hardcoded IC, `None` returns)
   before assuming the pipeline runs. Often the grid + store + promotion gates
   are real and only the objective is fake.
2. **Build the objective from data you can actually load** (a candle/price table,
   not model predictions). A defensible, fully-computable default: rank IC of a
   strategy factor vs forward return over the horizon.
3. **Guard the signal family dispatch.** `signal_for(family)` returns `None` for
   any family with no implementation → log a warning and SKIP the slot. Never
   silently score a candidate with the wrong objective.
4. **Reuse existing promotion gates as the accept/reject bar** (`min_trades`,
   `min_ic`, `min_sharpe` per candidate→paper→micro→live) instead of inventing
   new thresholds.
5. **Noise floor test at the gate.** Sanity-check the objective on synthetic
   inputs: clean trend must give high positive IC, white-noise / random-walk must
   land BELOW the IC gate (so noise can never promote). If noise clears the gate,
   the objective is degenerate.

## The rank-IC objective (works with ties)
- Signal: rolling-SMA momentum — signed distance from a trailing SMA normalized
  by the SMA, zeroed below a magnitude threshold. `O(n)` via a rolling sum (a
  naive `O(n·window)` repeat is a real cost bug on long series).
- Label: simple forward return over `horizon` bars: `(close[t+h]-close[t])/close[t]`.
- Correlation: **Spearman via average-rank transform** — correct when the signal
  is quantized (ties). Pairwise average ranking, then Pearson on the ranks.
- Walk-forward: split into `n_folds` test segments each preceded by an embargo
  (e.g. 136 days ≈ 6 months, matching the training notebook's embargo) to block
  lookahead; accumulate fold ICs; mean IC + std IC across folds.
- `n_trades` = count of non-zero-signal bars across the ENTIRE series (deployable
  trade frequency) — the metric the `candidate_to_paper` gate uses, not the
  number of test-fold samples.

## Verify with a Python mirror before trusting the Rust
Rust unit tests may be un-runnable if the crate's test fixtures are stale (missing
fields in `Candle`/`Config`/`AppState` that other work introduced). Don't fight
the harness — mirror the exact algorithm in Python and assert the same invariants:
- perfect monotone → 1.0
- anti-monotone with ties → -1.0
- clean uptrend → IC ≈ 1.0 (a positive-rank sanity bound)
- random walk → IC near/under the gate (0.03) → noise does not promote
- rolling-SMA implementation identical to brute-force rolling mean

This both verifies the math and gives the user concrete numbers to review.

## Nightly-scheduler trigger gotcha
A "local timezone offset" in a scheduler config is NOT necessarily wrong just
because the host clock shows a different TZ — the scheduler typically computes the
window entirely in UTC from the offset and never reads the host clock. Check the
actual date-arithmetic (close→ post-market buffer → earliest start → hard stop)
before flagging a timezone bug. On a UTC host, `offset=+8` with US-close-aware
local times can be internally consistent and map to the correct UTC window.