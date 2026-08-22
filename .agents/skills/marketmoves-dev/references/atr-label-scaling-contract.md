# ATR-Scaled Label to Raw Return Contract

The live `EquityEnsemble.predict()` must return **raw log-return predictions**, but the training notebook and the model outputs operate in **ATR-scaled label space**. This is the most common source of "astronomical predictions" in the dashboard.

## Label definitions

In the notebook (`models/colab/QQQ_Equities_Model.ipynb` / `EQ_Equities_Model.ipynb`):

```python
fut_ret = (close.shift(-h) - close) / close            # raw h-day log/linear return
vol_scale = atr / close                                 # ATR(14) / close
mag_h = clip(fut_ret / (vol_scale + 1e-6), -3, +3)      # ATR-scaled label
```

- `fut_ret`: return in **return space** (unitless, e.g. 0.012 = 1.2%)
- `mag_h`: ATR-scaled label in **mag space** (typically [-3, 3])

The model weights were trained on `mag_h` directly (raw labels, not z-scored targets), so `tcn_raw` and `lgbm_raw` are in **mag space**.

## What `label_std` is

`label_std` saved in `model_meta_{symbol}_v1.json` is the per-horizon standard deviation of the raw `mag_h` columns:

```python
{"label_std_1d": std(mag_1d), "label_std_5d": std(mag_5d), "label_std_21d": std(mag_21d)}
```

Typical QQQ values: 0.727 (1d), 1.477 (5d), 2.199 (21d).

## De-normalization math

Z-scoring happens in **mag space**:

```
blend_z = zscore(tcn_raw) * 0.5 + zscore(lgbm_raw) * 0.5
```

To convert back to a raw log return, multiply by the mag-space std and then by the current ATR ratio:

```
raw_log_return = blend_z * label_std * atr_ratio
```

For QQQ on 2026-08-14:
- `atr_ratio = ATR / close ≈ 0.01686`
- `label_std_1d ≈ 0.727`
- `blend_z` is typically [-3, +3]
- Result: `raw_log_return ≈ blend_z * 0.727 * 0.01686 ≈ blend_z * 0.0123`

A z-score of +1 therefore predicts a +1.23% 1-day return, which matches the old hardcoded defaults (0.012).

## The bug we hit

`inference/equity_model.py` was multiplying by `label_std` but **not** by `atr_ratio`:

```python
# BUG: returns mag-space values, not return-space values
raw_log_return = blend_z * label_std
```

With `label_std_1d = 0.727` and `blend_z = 1.0`, this returned `0.727`, i.e. a predicted 72.7% 1-day move. The dashboard chart showed 1-day price targets like $1273 for QQQ at $733.

## Fix

Both the warmup raw blend and the z-scored de-norm must multiply by `atr_ratio`:

```python
# Warmup
raw_pred = (tcn_weight * tcn_raw + lgbm_weight * lgbm_raw) * atr_ratio

# Z-scored
raw_log_return = blend_z * label_std * atr_ratio
```

After the fix, real QQQ predictions landed at:
- `pred_1d ≈ 0.012` (1.2%)
- `pred_5d ≈ -0.059` (-5.9%)
- `pred_21d ≈ 0.001` (0.1%)

## Sanity checks

Before adjusting strategy thresholds, verify:

1. Inference log shows `atr_ratio` for real requests (not just healthchecks with `seq_len=1`).
2. Predicted returns are in the single-digit percent range, not hundreds of percent.
3. `pred_1d * close` gives a plausible 1-day dollar target.
4. The `label_std_*` keys in `model_meta_*.json` are mag-space values (~0.5-2.5), not return-space values (~0.01-0.07). If they're return-space, the `atr_ratio` factor would double-shrink and predictions would be too small.

## Reference values from production

QQQ 2026-08-14 real request:
```
"atr_ratio": 0.016864373684114472,
"pred_1d": 0.01244244084412973,
"pred_5d": -0.059034058662422347,
"pred_21d": 0.0010497883512504706
```

Old buggy request (before `atr_ratio` fix):
```
"pred_1d": 0.738,   # 73.8%  -> chart showed $1273 target
"pred_5d": -3.50,   # -350%  -> chart showed $1148 target
```
