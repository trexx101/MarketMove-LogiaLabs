# Model artifact units: label_std, atr_ratio, and return-space de-normalization

## The labels the models learn

In the training notebook (`models/colab/EQ_Equities_Model.ipynb`), the label for each horizon is:

```python
mag_h = clip(fut_ret_h / (ATR_ratio + eps), -3, +3)
```

where `ATR_ratio = ATR(14) / close`. The label is therefore in **ATR-scaled return space**, not raw return space.

## label_std is the std of the ATR-scaled labels

When `compute_label_stats(labels)` runs at the end of training, it returns:

```python
label_std_1d = std(mag_1d)  # e.g. ~0.73 for QQQ
```

This is the std of the *ATR-scaled* labels, not the raw returns.

## How to return to raw log-return space

Inference multiplies the z-scored model output by `label_std` **and then by the current `atr_ratio`**:

```python
raw_log_return = blend_z * label_std * atr_ratio
```

If `atr_ratio` is omitted, the prediction is still in ATR-scaled space and will look huge (e.g. 73% for a z-score of 1 when `label_std=0.73`).

## Typical magnitudes

| Symbol | label_std_1d | Typical atr_ratio | Effective 1d std (return space) |
|---|---|---|---|
| QQQ | ~0.727 | ~0.0169 | ~0.0123 (1.23%) |
| SMH | ~0.737 | ~0.0200 | ~0.0147 (1.47%) |
| XLF | ~0.727 | ~0.0120 | ~0.0087 (0.87%) |

## Hotspot: warmup blend

During the first 10 inference requests, the z-score buffer is too short, so inference falls back to a raw 0.5/0.5 model blend. This raw blend is also in ATR-scaled space and must be multiplied by `atr_ratio` before returning.
