# Fix: norm_stats for QQQ equities deploy
**Created:** 2026-07-26
**Status:** BLOCKED — requires Colab re-run
**Symptom:** `pred_1d = -2987` (exploded), `pred_5d = -2555`, `pred_21d = -853`
**Symptom also visible in:** `mmn-inference` container logs

---

## Root Cause

The training Colab notebook (QQQ_Equities_Model.ipynb) computed normalization stats
from a training-data time period where RSI happened to sit near its floor (RSI ≈ 5).
This produced:

```json
{
  "medians": {
    "trend_slope": 0.0146,
    "trend_adx":   1.60,    ← 0-100 scale; median 1.6 is unrealistically low
    "rsi_14":       5.0,     ← 0-100 scale; median 5.0 is near floor (not a real median)
    "vix_regime":   1.0,     ← 0/1/2/3 discrete; treating as continuous
    "tlt_corr_20d": -0.006,  ← OK
    "rvol_20d":     0.96,     ← OK
    "gap_pct":      0.00074,  ← OK
    "drawdown_from_50d_high": -0.028  ← OK
  },
  "mads": {
    "rsi_14":       1e-06,   ← BUG: near-zero → normalizes to billions for real RSI ~60
    ...
  }
}
```

When the Rust engine computes live features:

| Feature        | Live value | Median  | Normalized (÷ (1.48×MAD))  |
|----------------|------------|---------|-----------------------------|
| rsi_14         | 58.7       | 5.0     | (53.7) / (1.48e-6) ≈ 3.6×10⁷ |
| trend_adx      | 36.9       | 1.6     | 35.3 / 0.70 ≈ 50            |
| vix_regime     | 0.0        | 1.0     | -1.0 / 1.48 ≈ -0.68         |

The exploded RSI (billions) dominates the TCN input. The model was never trained
on features in this range, so it produces unpredictable output → *atr_ratio denorm
gives huge raw predictions.

---

## Fix (in Colab)

The Colab notebook's norm-stats computation needs to use the FULL dataset period
(not just the training window). RSI's true long-run median over 2000–2025 is ~50,
not 5. A robust fix:

1. In `QQQ_Equities_Model.ipynb`, replace the per-split norm-stats with a single
   pass over ALL 500+ rows before any train/val/test split.
2. Use rolling/expanding median (not full-history) for MAD to be robust to regime shifts.
3. Clamp RSI MAD to at least 0.5 (1% of range), ADX MAD to at least 1.0:
   ```
   mad['rsi_14']       = max(mad_raw, 0.5)   # 0-100 scale
   mad['trend_adx']   = max(mad_raw, 1.0)
   ```
4. OR: set RSI MAD to 14.0 (standard: 100/2/π ≈ 15.9 as a conservative estimate).

After fixing, save `norm_stats_qqq_v1.json` and re-upload to the model artifacts volume.

---

## After the fix (manual steps)

1. Upload new `norm_stats_qqq_v1.json` to `/models/` volume on VPS:
   ```bash
   docker cp norm_stats_qqq_v1.json mmn-inference:/models/
   docker restart mmn-inference
   ```
2. Or rebuild inference image with fixed artifact baked in.

3. Verify:
   ```bash
   curl http://localhost:9080/api/status | python3 -c "import json,sys; d=json.load(sys.stdin); print('pred_1d:', d['pred_1d'], '| expect |pred_1d| < 10')"
   ```
   — should show `|pred_1d| < 10` (raw log-return space, e.g. -0.005 to +0.005 ≈ -0.5% to +0.5% daily move).

4. Also verify: `curl http://localhost:9080/api/predictions` shows pred_1h_approx
   in a reasonable range (~-5 to +5, representing cents-per-share for QQQ).

---

## Why healthcheck passes

The `mmn-inference` container's HEALTHCHECK sends `[[0.0]*8]` (all zeros).
With all zeros → RSI is exactly 0.0, which is very close to median 5.0 → normalized ≈ 0.
So the model sees near-zero input → near-zero output → healthcheck passes.

The real prediction path (engine sends 126 real features) hits the RSI explosion.

---

## Related

- Engine logs: `docker logs mmn-inference --since 11:14 | grep pred_1d`
- API: `GET /api/status` (shows raw exploded predictions)
- API: `GET /api/predictions` (shows all horizons + approx 1h)
- Chart: `GET /api/chart` (candles from 2021-07-27, working fine)
