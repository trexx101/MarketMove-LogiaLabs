# Claude Sonnet 4 Code Review — penetation-label TCN training pipeline

Origin: 2026-07-20, MarketMarkovNet Wave 5. The training pipeline had been
iterated 5+ times (GARCH constant, ATR NaN, barrel k units, barrier widening,
signed-regression target) and still produced negative walk-forward OOS IC.
A one-shot code review via `openrouter/anthropic/claude-sonnet-4` caught
8 issues — 3 critical, 3 high, 2 medium — none of which had been found
through iterative debugging.

## How to use this pattern

1. Assemble the full source of ALL training Python files + the latest Colab
   output into a single prompt.
2. Include specific questions about WHY edge is absent, not just "review this."
3. Ask for severity + concrete code snippet per issue, then a prioritized list.
4. Model: `openrouter/anthropic/claude-sonnet-4` (via OmniRoute proxy), temp 0.3,
   max_tokens 12K. Cost ~$0.15-0.30 for a 33K-char prompt.
5. Implement the fixes in priority order. Don't debate — just apply.

## The 8 issues found (in rank order)

1. **GARCH vol regime constant** (critical) — fixed params ω=0.05 dwarfed α·r²
   for BTC 1h returns (~1e-6 variance). σ² always ≈ ω/(1-β) = 0.333, scaled
   output = 0.732 with std=0.000024. Feature was dead across 35K bars.
   → Fix: adaptive realized-vol percentile over rolling 20-bar windows.

2. **Barriers too tight → 100% penetration** (critical) — c=0.5 gave barriers
   at ~0.2% from entry; BTC always moves >0.2% in 12h. Zero discrimination.
   → Fix: calibrate c empirically (2.0+ for BTC 1h) to get ~40-60% penetration.

3. **API geo-block → 2 of 6 features all zeros** (high) — Binance Futures API
   returned HTTP 451 from Colab. funding_rate and basis_z were 0.0.
   → Fix: `data.binance.vision` ZIP downloads (no geo-block).
   Note: the fallback to 0.0 was there, but the model can't find edge with
   2 useful features out of 6.

4. **Label magnitude logic error** (high) — `max_pen` accumulated unbounded
   penetration distance. Even after clip to [-3,3], 80%+ of values saturated.
   → Fix: time-weighted magnitude (0.5 * time_factor + 0.5 * min(pen, 2.0)),
   clean [-1.5, 1.5] range before clip. The time component captures
   "how quickly price reached the barrier" which is itself informative.

5. **No feature normalization** (high) — features had vastly different scales
   (vol_break 0-1, vol regime 0-2, basis_z ±5, funding_rate ±0.001) fed
   directly to the TCN.
   → Fix: median/MAD robust scaling instead of z-score.

6. **TCN architecture too simple** (medium) — plain conv stack with no residual
   connections, no input projection, single linear head per horizon.
   → Fix: ResidualBlock with GroupNorm+SiLU, input projection, MLP heads,
   learnable multi-task loss weights.

7. **Training loop issues** (medium) — fixed LR (no schedule), no early stopping,
   MSE loss (not Huber).
   → Fix: OneCycleLR, patience-based early stopping, SmoothL1Loss.

8. **Feature set too small** (medium) — 6 dims with 1 constant (llm_bull_prob=0.5).
   → Fix: 10 dims adding momentum_12h, momentum_72h, vol-price divergence,
   vol term structure, range compression.

## Impact

After applying all 8 fixes in priority order: walk-forward IC flipped from
negative (~-0.01 to -0.03) to — TBD (not yet re-run on Colab post-fix).
The immediate effect was that penetration rates became calibratable, feature
std was non-zero on more dims, and the loss curve became stable and
monotonic instead of flatlining at epoch 10.
