# Z-Score Blending Architecture

**Implemented:** 2026-08-07. Replaces raw weighted average blending in
`inference/equity_model.py::EquityEnsemble.predict()` with per-model
z-score normalization matching the Colab walk-forward evaluation pipeline.

**Updated:** 2026-08-13. Added `skip_buffer` flag to prevent healthcheck
requests from polluting the z-score prediction buffers.

**Updated:** 2026-08-14. Fixed de-normalization to multiply by both the
training-time `label_std` and the current `atr_ratio`; added raw-blend
warmup for the first 10 samples.

## Motivation

The Colab notebook (`models/colab/QQQ_Equities_Model.ipynb`, Cell 14) uses
per-fold z-score blending during walk-forward evaluation:

```python
l_pred = (lgbm_preds - mean) / (std + 1e-8)
t_pred = (tcn_preds  - mean) / (std + 1e-8)
ens_pred = (l_pred + t_pred) / 2.0
```

The deployed inference service used raw `0.5 * tcn + 0.5 * lgbm` with no
standardization. This caused a silent divergence: the notebook's backtest
metrics (IC, directional accuracy) were computed with z-score blending,
but live trading used raw blending. The result was a **prediction bias**
that made directional accuracy appear worse than the model's true capability.

## How it works

Each `EquityEnsemble` maintains two per-horizon rolling buffers (252 slots,
~1 year of trading days):

```python
self._tcn_buffer: dict[int, deque[float]]   # TCN predictions per horizon
self._lgbm_buffer: dict[int, deque[float]]  # LGBM predictions per horizon
```

On each prediction:

1. **Raw inference**: TCN and LGBM produce label-space (ATR-normalized) values.
2. **Z-score each model**: `z = (raw - mean(buffer)) / std(buffer, ddof=1)`
3. **Blend z-scores**: `blend_z = w_t * z_tcn + w_l * z_lgbm`
4. **Denormalize to return space**: `raw_return = blend_z * label_std * atr_ratio`
5. **Push to buffers**: append raw values to both buffers for future z-scoring
   (UNLESS `skip_buffer=True`).

The saved `label_std` is the per-horizon std of the ATR-scaled training
labels (`mag`), not the std of raw returns. Multiplying by `atr_ratio`
converts back to raw log-return space. Forgetting the `atr_ratio` factor
produces astronomical predictions. See `references/atr-label-scaling-contract.md`
for the full contract and example values.

## Warmup: raw blend for the first 10 predictions

When a buffer has fewer than 10 samples, z-score blending is degenerate
(std of 1–9 samples is unstable and can be 0). During warmup the ensemble
falls back to a raw 0.5/0.5 blend, but still converts to return space by
multiplying by `atr_ratio`:

```python
raw_pred = (tcn_weight * tcn_raw + lgbm_weight * lgbm_raw) * atr_ratio
```

This gives immediate, non-zero predictions after a restart instead of
waiting 2 trading days. After the 10th sample, the buffer has enough data
for stable z-scores and the z-scored de-norm takes over. This means the
first 10 predictions per symbol use the raw blend; predictions 11+
use z-score blending.

## `skip_buffer` flag (added 2026-08-13)

The `predict()` method accepts `skip_buffer: bool = False`. When `True`,
steps 1–4 execute normally but step 5 (push to buffers) is skipped.

### Why this is needed

The inference healthcheck (every 30s) sends `symbol=""` with `seq_len=1`
and all-zero features. Without `skip_buffer`, these zero-predictions are
pushed into the z-score buffers. Over 6 days, ~16,000 zero-predictions
flooded the NVDA ensemble's buffers (NVDA was the fallback for
`symbol=""` because it's alphabetically first). All real NVDA predictions
were then z-scored against a buffer full of zeros, producing ~0.0.

### Detection in `_handle_request`

```python
is_healthcheck = (
    len(feature_window) == 1
    and all(abs(v) < 1e-12 for v in feature_window[0])
)
preds = ensemble.predict(feature_window, atr_ratio=atr_ratio,
                         skip_buffer=is_healthcheck)
```

### Diagnosis of buffer pollution

- Inference logs show ONLY `symbol=""` and `seq_len=1` requests (no
  `seq_len=126` real requests from the scheduler).
- Healthcheck predictions show non-zero values (because the buffers have
  been filling from healthchecks), which masks the problem.
- Real predictions (when they do arrive) are ~0.0 because they're
  z-scored against a buffer full of zeros.

## Per-model isolation

Each model (QQQ, NVDA) has its own ensemble with independent buffers.
A QQQ prediction never affects NVDA z-scores and vice versa. The buffers
are per-model, per-horizon, and per-model-type (TCN vs LGBM).

## Implementation checklist for new models

When adding a new model (e.g., PLTR):

1. Train the model via the Colab notebook (produces TCN + 3 LGBM + norm_stats).
2. Place artifacts in `/models/{SYMBOL}/` with the naming convention
   `{symbol_lower}_tcn_v1.pt`, `{symbol_lower}_lgbm_h{1,5,21}_v1.pkl`,
   `norm_stats_{symbol_lower}_v1.json`.
3. The inference service auto-discovers the directory at startup.
4. Register the model in `trading_models` via the engine API.
5. The engine scheduler sends `symbol={SYMBOL}` in the ZMQ request.
6. The inference service routes to the correct ensemble.
7. First 10 predictions use the raw blend; after that, z-scores are
   meaningful.

## Divergence from notebook

The notebook uses **per-fold** z-score (mean/std computed from the current
walk-forward fold's predictions). The deployed service uses **rolling**
z-score (mean/std from the last 252 predictions). These are different
windows but both achieve the same goal: removing per-model bias.

The rolling window is a practical approximation — it doesn't require
knowing which fold a prediction belongs to, and it adapts to distribution
shift over time. The first 10 predictions will use the raw blend (not the
notebook's per-fold z-score), but the difference converges as the buffer
fills.

## Cold-start behavior

After each inference container restart, the z-score buffers are empty.
The first 10 predictions per symbol use the raw 0.5/0.5 blend multiplied by
`atr_ratio`. After 10 samples, z-score blending takes over. This is expected
behavior — the healthcheck no longer pollutes the buffers (skip_buffer flag),
so the warmup is clean. Do NOT interpret cold-start raw-blend predictions as
a bug.
