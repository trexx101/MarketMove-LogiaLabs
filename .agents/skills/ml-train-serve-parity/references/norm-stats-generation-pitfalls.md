# Norm Stats Generation — Pitfalls & Fix History

Session: 2026-07-26. Regenerating `norm_stats_qqq_v1.json` for QQQ equities
deploy after `pred_1d = -2987` (explosion) in production.

## 1. Global clip squashing bounded features (RSI, ADX)

**Symptom:** `norm_stats_qqq_v1.json` had `rsi_14: median=5.0, MAD=1e-6`.
RSI ranges 0–100; a median of 5.0 is the floor, not a real median.

**Root cause:** `compute_features()` in the Colab notebook applied
`f = f.clip(lower=-5.0, upper=5.0)` to ALL features at the end. This was
intended to clip outlier-prone features like `trend_slope` and
`drawdown_from_50d_high`, but it also clipped `rsi_14` (0–100) to a max of
5.0 and `trend_adx` to a max of 5.0. Over a multi-year training set, RSI
spends significant time above 5.0, so the clipped median landed at exactly
the clip ceiling (5.0), and the MAD was ~0 (no variance above the ceiling).
The `mads[mads == 0] = 1e-6` fallback then set MAD to 1e-6.

**Normalization explosion:**
```
normalized = (58.7 - 5.0) / (1.4826 × 1e-6) ≈ 3.6 × 10^7
```
The TCN receives billions → outputs billions → `pred_1d = -2987`.

**Fix:** Apply clip ONLY to unbounded features:
```python
clip_cols = ['trend_slope', 'rvol_20d', 'gap_pct', 'drawdown_from_50d_high']
f[clip_cols] = f[clip_cols].clip(lower=-5.0, upper=5.0)
# rsi_14, trend_adx, vix_regime left unclipped
```

After the fix: `rsi_14: median=55.19, MAD=14.0` (floored). Walk-forward IC
0.0941 (gate 0.03), all horizons positive. Sanity check passed.

## 2. trend_adx and rsi_14: train/serve alignment (COMPLETED 2026-07-26)

**The mismatch (found in 2 sub-bugs):**

### 2a. trend_adx: ATR proxy vs real ADX

| Side | Formula | Typical QQQ value |
|------|---------|-------------------|
| Colab notebook (OLD) | `wilder_ema(tr, 14) / wilder_ema(close, 14) * 100` | 1–3 |
| Rust engine (`equities_v2.rs:adx_14`) | +DM/-DM → +DI/-DI → DX → Wilder smoothing | 10–50 |

Same column name (`trend_adx`), completely different feature. The model
was trained on the proxy (median 1.6, MAD 0.475); the engine feeds real ADX
(~20–40). After normalization, the model sees z-scores it was never trained
on: `(30 - 1.6) / (1.4826 × 0.475) ≈ 40` — large but not explosive.

### 2b. RSI smoothing seed mismatch

The notebook's `wilder_rsi` used `pd.Series.ewm(alpha=1/14, adjust=False)`,
which starts recursive smoothing from index 0. The engine's `rsi_14` seeds
avg_gain/avg_loss with the SIMPLE MEAN of the first 14 values, then recurses.
These produce different values during warmup and converge only slowly.

### Fix applied (Option A — align Colab to engine)

Rewrote `compute_features` in the Colab notebook with three engine-matching
functions:

```python
def wilder_smooth(s, n):
    """Wilder's smoothing: seed = SMA of first n, then recursive.
    Matches engine/src/features/equities_v2.rs:wilder_smooth exactly."""
    out = pd.Series(index=s.index, dtype=float)
    if len(s) < n:
        return out
    out.iloc[n - 1] = s.iloc[:n].sum() / n
    for i in range(n, len(s)):
        out.iloc[i] = (out.iloc[i - 1] * (n - 1) + s.iloc[i]) / n
    return out

def wilder_rsi(s, n=14):
    """RSI using Wilder's smoothing, seeded with SMA of first n gains/losses.
    Matches engine/src/features/equities_v2.rs:rsi_14 exactly.
    Warmup (first n bars) = 50.0 (neutral), same as engine."""
    delta = s.diff()
    gains = delta.clip(lower=0.0)
    losses = (-delta.clip(upper=0.0))
    out = pd.Series(50.0, index=s.index)
    if len(s) < n + 1:
        return out
    avg_gain = gains.iloc[1:n + 1].sum() / n   # SMA seed, NOT ewm
    avg_loss = losses.iloc[1:n + 1].sum() / n
    out.iloc[n] = 100.0 - 100.0 / (1.0 + (avg_gain / (avg_loss + 1e-12)))
    for i in range(n + 1, len(s)):
        avg_gain = (avg_gain * (n - 1) + gains.iloc[i]) / n
        avg_loss = (avg_loss * (n - 1) + losses.iloc[i]) / n
        out.iloc[i] = 100.0 - 100.0 / (1.0 + (avg_gain / (avg_loss + 1e-12)))
    return out

def adx_14(high, low, close, n=14):
    """Real ADX using +DM/-DM → +DI/-DI → DX → Wilder smoothing.
    Matches engine/src/features/equities_v2.rs:adx_14 exactly."""
    # ... (+DM, -DM, TR, ATR, smoothed DM, +DI, -DI, DX, ADX)
    # Full implementation in the notebook cell 8.
```

**Verified on 6886 rows of real QQQ data (1999-2026):**
- ADX: min=0.00, max=56.98, median=21.92 (engine scale, not proxy)
- RSI: min=19.15, max=85.59, median=55.10
- norm_stats: trend_adx median=21.90 MAD=5.59, rsi_14 median=55.10 MAD=14.00
- Live z-scores: ADX=25 → z=0.374, RSI=55 → z=-0.005 (both reasonable)
- Sanity check passes.

**Colab rerun results (2026-07-26, with real ADX + Wilder RSI):**
- Walk-forward: Mean IC 0.0784 (gate 0.03), all horizons positive.
  - 1d: Ens IC 0.0535, LGBM IC 0.0405, TCN IC 0.0388
  - 5d: Ens IC 0.0632, LGBM IC 0.0731, TCN IC 0.0260
  - 21d: Ens IC 0.1184, LGBM IC 0.1777, TCN IC 0.0225
- Exported norm_stats: trend_adx median=21.96, MAD=5.56; rsi_14 median=55.13, MAD=14.0
- Sanity check PASSED.
- Note: the 1d IC dropped from 0.0941 (ATR-proxy run) to 0.0784 (real ADX run),
  but the model now has train/serve parity. The ATR-proxy IC was inflated by
  the train/serve mismatch (the model was exploiting a feature that wouldn't
  exist at inference). The real-ADX IC is the honest number.

## 3. Sanity check thresholds

With real ADX (0-100 scale), the sanity check thresholds are:
```python
assert 5 <= adx_med <= 80   # real ADX median
assert adx_mad >= 1.0        # real ADX MAD
```

MAD_FLOORS for real ADX:
```python
MAD_FLOORS = {
    'trend_adx':  1.0,   # real ADX (was 0.1 for ATR proxy, 5.0 was too aggressive)
    'rsi_14':    14.0,
    ...
}
```

NOTE: If using the ATR proxy instead (NOT recommended — see §2), thresholds
must be `0.1 <= adx_med <= 80` and MAD_FLOOR=0.1. But the correct fix is to
use real ADX.

## 4. Per-feature MAD floors (defensive)

Even after fixing the clip bug, add per-feature minimum MAD floors so a
future training slice with near-zero variance can't reproduce the explosion:

```python
MAD_FLOORS = {
    'trend_slope':              0.005,
    'trend_adx':                1.0,   # real ADX; use 0.1 if ATR proxy
    'rsi_14':                  14.0,   # 0-100 scale; long-run MAD ~14
    'vix_regime':               1.0,   # discrete 0/1/2/3
    'tlt_corr_20d':             0.10,
    'rvol_20d':                 0.10,
    'gap_pct':                  0.001,
    'drawdown_from_50d_high':   0.01,
}
for col, floor in MAD_FLOORS.items():
    if col in mads.index:
        mads[col] = max(mads[col], floor)
```

## 5. Hardcoded Rust test assertions — RESOLVED

The Rust test `norm_stats_load_named_matches_trained_artifact`
(equities_v2.rs:672) previously hardcoded exact artifact values:
```rust
// OLD (fragile — broke on every retrain):
assert!((stats.median[0] - 0.014625118390912132).abs() < 1e-9);
assert!(stats.mad[2] < 1e-5);  // RSI MAD was 1e-6 — the bug itself!
```

**Fix applied (2026-07-26):** replaced with property-based range checks:
```rust
// NEW (survives every retrain, catches the bug class):
assert!(stats.median[1] > 5.0, "trend_adx median too low (ATR proxy?)");
assert!(stats.median[2] > 20.0, "rsi_14 median too low (clipped?)");
assert!(stats.mad[2] >= 5.0, "rsi_14 MAD too small (explosion bug)");
assert!(v.abs() < 1e6, "normalized value exploded");
```

This is strictly better — it validates the SHAPE of the artifact without
pinning to specific training-run values, AND it catches the explosion bug
class if it ever recurs.

## 6. Stale notebook file after Colab run

If you edit the notebook in Colab (e.g. changing sanity check thresholds)
but save/download a version that doesn't reflect your edits, the saved
`.ipynb` will have the old code while the output cells show the new results.
The artifact (`norm_stats_qqq_v1.json`) is correct (it was generated by the
code you actually ran), but the `.ipynb` is stale.

**Detection:** If the norm_stats values are inconsistent with the sanity
check thresholds in the `.ipynb` (e.g. `trend_adx` median=1.6 but the
notebook asserts `5 <= adx_med <= 80`), the notebook is stale.

**Fix:** Re-download the notebook from Colab after all edits, or apply the
edits locally with `json.load`/`json.dump` (Strategy A from the
`safe-structured-edits` skill) before uploading.
