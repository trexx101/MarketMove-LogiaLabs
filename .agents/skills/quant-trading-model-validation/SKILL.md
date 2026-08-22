---
name: quant-trading-model-validation
title: "Quant Trading Model Edge Validation"
description: "Decide whether a trading model has REAL predictive edge before deploying it — walk-forward out-of-sample IC gate, directional/penetration labels vs raw-return regression, alpha-source selection, and the common traps (single-split regime luck, backtest-positive-from-staying-flat, directional hinge harming correlation). Use when building/reviewing a price-prediction or signal model for crypto/equities/futures, when live predictions look muted or near-zero, or when a backtest looks good but you distrust it."
category: mlops
triggers:
  - Building or reviewing a trading/price-prediction model (crypto, equities, futures)
  - A backtest looks profitable but you suspect it has no real edge
  - Live predictions are near-zero, muted, or worse than the notebook backtest
  - Deciding whether to retrain vs deploy vs reconsider the alpha source
  - "Does this model actually have quantitative edge?"
  - Designing training labels / validation splits for a time-series signal model
  - Building a hyperopt objective where the evaluator is a placeholder
---

> Reference: `references/hyperopt-objective-from-existing-data.md` — rebuild a
> placeholder hyperopt objective from existing price data (rank IC of a rolling-SMA
> factor vs forward return, walk-forward + embargo), guard unimplemented signal
> families, and verify with a Python mirror when the Rust test harness is stale.

# Quant Trading Model Edge Validation

The core discipline: **measure out-of-sample edge with walk-forward validation BEFORE
shipping weights.** A model can have a great single-split backtest and zero real edge.
Tuning the network never fixes a dead alpha source — the signal has to come from the
data, not a better fit on the same features.

## The deploy gate (non-negotiable)

Deploy ONLY if, on walk-forward / purged k-fold CV with embargo:
- **Mean OOS Information Coefficient (Pearson r of prediction vs realized) > ~0.03–0.05**
  on at least one horizon, AND
- **OOS equity curve positive** after realistic costs.

Secondary (report but don't block on alone): OOS Sharpe > 0, max drawdown, turnover.
Sharpe is noisy on few folds — keep IC as the PRIMARY gate.

If IC stays ≈ 0 (or negative) after adding real features → **STOP. Do not ship.** The
signal genuinely may not exist at that horizon. Reconsider the alpha source, not the net.

## The QQQ daily model: confirmed IC ≈ 0 (2026-07-30 session finding)

**Live DB state** (`/var/lib/docker/volumes/deploy_data/_data/candles.db`):
```
equity_predictions: 3 rows only (2026-07-24, 2026-07-27, 2026-07-30)
equity_candles: 14,298 rows (2021-08-02 → 2026-07-30)
```

The `EquityScheduler` processes one candle per poll cycle (`ts > last_processed_ts` guard) and inserts one row per cycle. It reaches the latest candle and stops — **it never retrodicts historical candles**. This is why the `equity_predictions` table covers 5 years of candles with only 3 prediction rows.

**Consequence**: `POST /api/backtest` over any historical range finds no predictions and falls back to `pred_1d = 0.0` for every bar → every held Long exits (0.0 < exit_threshold of -0.001). Backtests are meaningless.

**Evidence of no edge**:
- `/api/accuracy` returned `directional_1h: 66.7%` on 3 samples — statistically useless (2/3 correct from noise)
- `mae_1h = 0.00475`, `mae_4h = 0.0452` — right magnitude, but magnitude ≠ directional accuracy
- Prior session walk-forward OOS mean IC ≈ 0 — consistent with this

**The strategy state machine is mechanically correct** (10/10 tests pass). The problem is upstream. The TCN + LightGBM ensemble for QQQ daily produces predictions with near-zero rank correlation to actual returns. No threshold strategy can extract alpha from a model with IC ≈ 0.

**Action**: Do not attempt strategy parameter sweeps or lab experiments until the prediction history is populated AND IC is re-evaluated. Required fixes:
1. `POST /api/backfill-predictions` — replay scheduler logic over all historical candles to populate `equity_predictions`
2. Add rank IC (`corr(pred_1d, actual_1d)`) to `/api/accuracy` — directional % alone is insufficient
3. Retrain the model with a different label definition or features if OOS IC stays ≈ 0

## Traps that fake an edge (all seen in real sessions)

1. **Single contiguous train/test split = regime luck.** A 70/15/15 split on one bull
   year can show +29% that vanishes to +2% under multi-regime walk-forward. Always
   walk-forward across bull/bear/sideways.
2. **"Backtest beats buy-and-hold" in a bear market from staying FLAT.** If win-rate < 50%
   but return > B&H because B&H was deeply negative, the "edge" is *not trading*, not skill.
   Check win-rate and turnover, not just total return vs B&H.
3. **Raw-return regression has ~zero signal.** Regressing next-period log-return from OHLCV
   is near an efficient-market random walk (achievable IC ≈ 0–0.02). Switch the LABEL, not
   the model. Use **directional or eventual-move (penetration) labels**: "does price cross
   ±k% within N bars?" — far higher SNR, lower entropy, less noise. Scale k to volatility
   (e.g. k = c·rollingATR), don't hardcode per horizon.
4. **Directional hinge / aggressive scaling can make predictions ANTI-correlated.** A loss
   term that penalizes wrong-direction with a hinge, plus ×100 target scaling, can push
   eval correlation NEGATIVE ("confidently wrong"). If eval IC goes negative after adding a
   directional term, remove the term — keep MSE + horizon-monotonicity only.
5. **Look-ahead in penetration labels.** "Within N bars" must be computed FORWARD-only from
   bar t. Embargo must cover `max(standard_embargo, label_horizon + N)`, not just a flat 72h.
6. **Collinear features counted as independent edges.** Funding rate, perp basis, and
   order-flow imbalance all derive from the same order-book microstructure — in stress they
   collapse to one factor. Compute PCA/VIF on the feature block; the first principal
   component is the real signal. Don't treat 3 correlated microstructure features as 3 edges.
5. **Placeholder zero features → guaranteed no edge.** If 4 of 6 features are zeros
  (unwired funding/basis/ob), walk-forward IC stays ~0. Print feature stats BEFORE
  training; if `std ≈ 0` for any feature, the data pipeline is broken, not the model.
  Fix the feature source, don't retune the net.
6. **Barrier in absolute price units vs fraction (critical unit bug).** If `df['k'] = c * df['atr']`
  where ATR is in absolute USD (e.g. $1,000 for BTC 1h), then k = 0.15 × 1000 = 150, and
  `upper = close * (1 + 150)` = 151 × close ≈ $10.5M. Bitcoin never reaches $10.5M in 12–72h.
  Penetration rate = 0% always. Fix: `df['k'] = c * df['atr'] / df['close']` so k is a
  dimensionless fraction (e.g. 0.15 × 1000/70000 ≈ 0.00214). Barriers then sit at ±0.21%
  and get hit regularly. This was the single most destructive bug encountered — it blocked
  all 3 horizons across 3 Colab runs before the root cause was identified.
7. **NaN propagation through EWM on first-row NaN.** `compute_atr` computes TR via
  `pd.concat([high_low, high_close, low_close], axis=1).max(axis=1)`. At index 0,
  `close.shift(1)` is NaN, so .max() returns NaN. Then `tr.ewm(span=N, adjust=False).mean()`
  propagates NaN indefinitely → every ATR value is NaN → every k is NaN → every
  barrier is NaN → no barrier is ever hit → 0% penetration always. Fix: add `.fillna(0)`
  after `.max(axis=1)`. The EWM then starts from 0 and converges correctly in ~N steps.
8. **Timestamp format ambiguity in Binance CSVs.** The `open_time` column can be in seconds
  (10 digits), milliseconds (13 digits), or microseconds (16 digits) depending on the
  Binance endpoint. Feeding 16-digit µs to `utcfromtimestamp(ms/1000)` gives
  `year 57971 out of range`. Fix: auto-detect — `if ts > 1e15: ts//=1000` (µs→ms);
  `if ts > 1e12: ts//=1000` (ms→s); then convert. Every Binance data function must
  do this normalization before using the timestamp.
9. **Funding rate CSV has a header row AND lives at monthly path (not daily).**
   Binance Vision funding rate ZIPs at
   `data.binance.vision/data/futures/um/monthly/fundingRate/<symbol>/` (NOTE:
   `daily/fundingRate/` returns 404). CSV columns: `calc_time` (epoch ms, 13 digits),
   `funding_interval_hours` (always 8), `last_funding_rate` (decimal). NOT
   `symbol, fundingTime, fundingRate, markPrice` as previously documented.
   Monthly ZIP contains CSVs with header row. Using `header=None` treats the header
   as data, then `astype('int64')` crashes on the string header value, which gets
   silently swallowed → empty DataFrame → funding_rate stays 0.0.
   Fix: `pd.read_csv(z.open(z.namelist()[0]), header=0)` then rename columns:
   `{'calc_time': 'funding_time_ms', 'last_funding_rate': 'funding_rate'}`.
   Records every 8h — must resample to hourly: `df.resample('1h').ffill()`.
10. **100% penetration rate = no signal discrimination.**
   the label collapses to "how far" with no direction-classification component. Calibrate
   `c` empirically (test 5-10 values on a subset, pick the one giving ~40-60% penetration
   for the shortest horizon — shorter horizon is the bottleneck). Never assume a `c` value;
   test it on real data. Rule of thumb (BTC 1h, 12h horizon): c=0.15 is ~100% penetration;
   c=2.0+ is needed for ~50%.

## Where edge actually lives (crypto, 30min–1d), by edge-per-cost

1. **Funding rate + perpetual basis** — strongest, free (exchange futures REST). ~4h–1d.
2. **Order-book / trade-flow imbalance** — strong at 30m–4h, needs ws depth/trade streams.
3. **Volatility-regime + structural-break state** — an engagement gate (when to trade).
4. **LLM regime/sentiment feature** — cheap model, hourly, cached, as a low-freq FEATURE.
5. **Vision-on-chart feature** — weakest/highest-cost; make it optional, gate on its own IC.
6. **On-chain flows** — slow macro overlay.
Rule: OHLCV alone is the floor (≈no edge). Add an independent microstructure/sentiment
source to have any chance of clearing the gate.

## First-run diagnostic checklist (Colab or local)

When you run training for the first time and IC is near-zero or nan, work this
checklist BEFORE changing the model architecture:

1. **Print penetration rates per horizon.** If `pen_rate < 2%`: check ATR values for NaN
   (see trap #7 above). If `pen_rate > 95%`: widen barrier `c` (calibrate empirically,
   see fix stage 3 below). If `pen_rate ≈ 0%`: check k for NaN propagation (trap #7)
   AND check k for absolute-vs-fraction unit error (trap #6). Print a debug k value:
   `k.iloc[lookback+100]` should be a small ~0.001-0.05 for BTC 1h.
2. **Print feature means and stds.** Any feature with `std < 1e-6` is broken:
   - `vol_regime` std ≈ 0 → GARCH params are too large for the input variance (see
     adaptive percentile fix in volatility section)
   - `funding_rate` std ≈ 0 → data fetch is failing (check your API/endpoint)
   - `basis_z` std ≈ 0 → data fetch is failing (check timestamp format, trap #8)
   - `llm_bull_prob` std ≈ 0 → placeholder not wired yet (acceptable, flag it)
3. **Check label class balance.** `mag_mean` should be non-zero. If `mag_max = mag_clip`
   for most values, the magnitude is saturating — widen the clip or use time-weighted
   magnitude (fix stage 2).
4. **Check first fold loss trajectory.** If loss → 0.0000 within 10 epochs → labels are
   degenerate (all neutral). Go to point 1. If loss stays flat (no decrease over 20
   epochs) → learning rate may be too low, or features have no signal.
5. **Check first fold IC sign.** If IC is consistently negative: check for look-ahead
   or data leakage (train future leaking into test fold). If IC oscillates sign across
   folds: the signal is regime-dependent — increase epoch count and add LR scheduling
   to let the model converge to a robust minimum.

## Architecture guidance

- Keep the model **cheap and local** (TCN / small Transformer / S4), per-bar CPU-feasible.
- **train == serve**: train on the same exchange/venue you deploy on. Cross-venue training
  (train Binance / deploy Kraken) injects microstructure drift; document it as a known
  limitation if unavoidable. (Pairs with the `ml-train-serve-parity` skill for feature/norm
  parity once the model is chosen.)
- **Pluggable FeatureSource interface** so the same core model extends to other asset
  classes (crypto → equities: swap funding/on-chain for earnings/intraday-vol).
- **Cheap LLM/vision models belong OUT of the latency path** — call hourly, cache, with a
  timeout + stale-cache fallback; feed the cached output as a feature column.

### Training methodology (from Claude Sonnet 4 review, confirmed on two Colab runs)

These are NOT optional refinements — they directly prevent the "loss collapses,
IC=nan" degenerate case:

- **Loss function**: use `SmoothL1Loss` (Huber) instead of `MSELoss`. Huber is L1
  near zero and L2 far from zero, so it ignores the magnitude outliers that penetration
  labels naturally produce even after clipping. MSE punishes a 3.0 prediction on a 0.0
  target 9× harder than a 0.1 → MSE chases outliers instead of learning direction.
- **Learning rate schedule**: use `OneCycleLR` with `pct_start=0.3` instead of a fixed
  LR. OneCycle warms up (avoids early SGD chaos) then anneals (settles into a good
  minimum). Fixed-LR either plateaus at high loss (too large) or converges to a sharp
  minimum (too small). Call `scheduler.step()` after every BATCH, not every epoch.
- **Early stopping**: validate every K epochs on the OOS split; stop after `patience`
  epochs with no improvement. Without it the model overfits train noise especially with
  multiple folds each getting 50 epochs. Code: `if val_loss < best: best, wait = val, 0
  else: wait += 1; if wait >= patience: break`.
- **ResidualBlock architecture**: a plain stack of `CausalConv1d + GroupNorm + SiLU +
  Dropout` with no skip connections makes the gradient vanish through 6+ dilation
  layers during backprop through time. Wrap each pair of convs in a `ResidualBlock`
  with `GroupNorm(1, ch)`, a skip connection, and `SiLU` activation.
- **Learnable multi-task loss weights**: use `nn.Parameter(torch.ones(n_heads))` with
  softmax normalization. Without it one horizon's scale (e.g. longer horizon has larger
  magnitudes) dominates the gradient.
- **Validate on OOS every 5–10 epochs**, not just at the end. Track `best_val_loss`
  and early-stop. Print validation progress so you can see if OOS is tracking train
  or diverging.

### Feature normalization: median/MAD instead of z-score

Standard z-score `(X - mean) / std` is fragile for trading features — a single
flash-crash bar inflates `std` and compresses all other values. Use robust
median/MAD scaling:

```python
medians = np.median(X, axis=0)
mads = np.median(np.abs(X - medians), axis=0)
mads = np.where(mads < 1e-8, 1.0, mads)
X = (X - medians) / (1.4826 * mads)
```

Export `medians` and `mads` (not mean/std) for serving parity. Schema version
bump to 4+ when switching. 1.4826 is the consistency constant that makes
MAD approximate σ for Gaussian data.

### Volatility regime: realized-vol percentile replaces fixed GARCH

Fixed-parameter GARCH(1,1) with ω=0.05 produces a CONSTANT for BTC 1h data —
the log-return variance (~1e-6) is dwarfed by ω, so the α·r² term never fires.
The feature has std≈0.000024 across 35,000 bars. Zero signal.

Replace with adaptive realized-vol percentile:

```python
def vol_regime(closes):
    returns = np.log(closes[1:] / closes[:-1])
    w = min(20, len(returns))
    recent = np.std(returns[-w:]) * np.sqrt(8760)
    look = min(500, len(returns))
    vols = [np.std(returns[i-w:i]) * np.sqrt(8760) for i in range(w, look)]
    if len(vols) < 10: return 1.0
    return 2.0 * np.mean(np.array(vols) <= recent_vol)
```

This tells the model "where does current vol rank vs recent history" — real
variance even on low-noise data. Rust serving must clone the logic exactly
for train==serve parity (same percentile computation on the same window).

## Workflow

1. State the alpha hypothesis and horizon (30min–1d, hold hours→days for structural moves).
2. Pick label = volatility-scaled penetration / direction, NOT raw return.
3. Build walk-forward harness FIRST (folds + embargo + purge of known event windows).
4. Add features cheapest-independent-first; measure incremental OOS IC per feature.
5. Report IC + Sharpe + maxDD + turnover + OOS equity per fold.
6. Gate: IC > 0.03–0.05 AND positive OOS equity → proceed; else stop / reconsider source.
7. Only after the gate passes: wire into serving (see ml-train-serve-parity for parity).

## Getting an architecture/research plan from a stronger model

For a one-shot design plan, hand a fully-grounded prompt (include prior failure numbers,
the exact serving contract, and the decisions already locked) to a strong reasoning model
(e.g. DeepSeek-R1, Opus, Gemini-Pro). Instruct it to reason step-by-step AND self-critique
"top 3 ways this still yields zero edge". Then critique its output against the traps above
before committing the plan — reasoning models over-engineer storage and count collinear
features as independent edges. See the `model-routing-proxy` skill for calling a specific
model via OmniRoute.

For a high-quality code review of existing training files (Python quant/ML), use
**Claude Sonnet 4** via proxy as the one-shot reviewer. Feed it the full source +
latest Colab output + specific questions about why edge is absent. It catches
numerical bugs (GARCH constant, NaN ATR propagation, magnitude outlier domination),
architecture issues (no residuals, no scheduler), and methodology gaps (no early
stopping, no robust normalization). Implement its prioritized fix list in order of
impact. Claude Opus 4 is overkill for code review — Sonnet 4 is 90% as good at 10%
the price. DeepSeek-R1 is for the QUANT DESIGN (what to build); Claude Sonnet 4 is
for the CODE REVIEW (is what we built correct). Use the right model for the task.

## Label degeneracy: when penetration labels kill the loss

Penetration labels naturally produce heavy class imbalance: with 1h candles and
`k = 0.5 * ATR`, only ~0.01% of bars have a penetration event within 1-4 future
bars. The remaining 99.99% are direction=0 (neutral), magnitude=0.

Symptoms on first training run:
- Loss collapses to 0.0000 by epoch 10 — focal loss + magnitude mask both
  zero out on the dominant neutral class, so the model learns "always predict
  neutral" and loss → 0.
- OOS IC = `nan` — model outputs constant predictions (all neutral), std=0,
  Pearson r undefined. Equity ≈ 0.0 (sign(0) * anything = 0).

### Fix stage 1: signed-regression target (recommended)

Combine direction and magnitude into ONE continuous target `y = direction * magnitude`
(train with plain MSE). No separate classification + regression heads, no focal loss,
no class-imbalance issue. The model learns to predict the signed eventual-move
magnitude directly. Add `compute_ic` returning 0.0 (not nan) on zero-variance
predictions so the deploy gate stays sane.

### Fix stage 2: time-weighted magnitude (from Claude Sonnet 4 review)

The raw magnitude `max_pen` compounds penetration distance beyond barrier and can
reach 50×+ even after clipping to [-3,3] — most values saturate at ±3, losing
all signal. Replace with a time-weighted magnitude:

```python
t_factor = 1.0 - hit_j / h                    # [0,   1]  — earlier hit = higher
penetration = (fh - upper) / (close_t * k_t)   # [0, +inf)
mag = first_dir * (0.5 * t_factor + 0.5 * min(pen, 2.0))
```

Time factor: 1.0 if hit immediately, ∼0 if hit at the last bar before expiry.
Penetration clamped at 2.0. Combined range: [-1.5, 1.5] before clipping to
[-3, 3]. No saturation, real signal variation.

### Fix stage 3: barrier calibration

Calibrate barrier width `c` empirically BEFORE training:

```python
def calibrate_barrier_c(df, target=0.5, horizons_bars=(12,)):
    best, best_diff = 2.0, inf
    for c in np.linspace(0.5, 6.0, 24):
        lbl = volatility_scaled_labels(df.iloc[:1000], c=c, horizons_bars=horizons_bars)
        pr = lbl['penetration_rates']['H1']
        diff = abs(pr - target)
        if diff < best_diff: best, best_diff = c, diff
    return best
```

Always print penetration rates per horizon BEFORE training:
```python
dirs, mags = labels['H1']
print(f"H1: up={np.sum(dirs>0)} down={np.sum(dirs<0)} "
      f"pen_rate={np.mean(dirs!=0):.2%} mag_mean={np.mean(mags):.4f}")
```

## Pitfalls

- Never claim "verified/working" from a backtest alone — verify with walk-forward OOS IC.
- Notebooks (.ipynb) that produce the model are often gitignored; commit the PLAN and the
  serving code, and keep a verification script, not just the notebook.
- When editing training .ipynb files with agent tools, see `safe-structured-edits` — text
  patches corrupt JSON; rebuild via write_file.
- **Binance Futures API is geo-blocked from Colab** (HTTP 451). `fapi.binance.com`,
  `www.binance.com`, and `data.binance.com` all return 451 from US-based Colab IPs.
  Fix: use `data.binance.vision` CDN-hosted ZIP files (same format as spot CSVs, no
  geo-block). Funding rates at `.../data/futures/um/monthly/fundingRate/BTCUSDT/`
  (NOTE: `daily/fundingRate/` returns 404 — only monthly exists). Futures klines at
  `.../klines/BTCUSDT/1h/`. Each monthly ZIP contains CSVs. Funding rate CSV columns:
  `calc_time` (epoch ms, 13 digits), `funding_interval_hours` (always 8),
  `last_funding_rate` (decimal). Records every 8h — forward-fill to hourly before
  merging. This is part of training infrastructure, not just a serving concern —
  without it, 2 of 10 features stay zero and the gate cannot be passed.

## References
- `references/multi-model-orchestration.md` — OmniRoute proxy model IDs, sector
  allocation (R1 for quant cores, Gemini for scaffolding), R1 output bug patterns
  (Default trait, rand dep, hardcoded dict keys, TCN architecture), and the
  validation protocol for LLM-produced code.
