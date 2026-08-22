# Multi-symbol model bundle sync (MarketMoves example)

Session-specific notes for deploying multiple per-symbol model bundles from a
Colab-trained artifact set into a Docker Compose stack.

## Artifact layout produced by the training notebook

The generic `EQ_Equities_Model.ipynb` notebook writes per-symbol artifacts like:

```
/models/
  QQQ/
    qqq_tcn_v1.pt
    qqq_lgbm_h1_v1.pkl
    qqq_lgbm_h5_v1.pkl
    qqq_lgbm_h21_v1.pkl
    norm_stats_qqq_v1.json
    model_meta_qqq_v1.json
  SMH/
    smh_tcn_v1.pt
    ...
```

The `SYMBOL` env var in Colab controls the ticker; the save paths use
`{symbol_key}` = `SYMBOL.lower()`.

## Key-shape contract in `model_meta_*.json`

The live inference loader (`inference/equity_model.py`) reads per-horizon keys:

```json
{
  "label_mean_1d": 0.035,
  "label_std_1d":  0.727,
  "label_mean_5d": 0.197,
  "label_std_5d":  1.477,
  "label_mean_21d": 0.619,
  "label_std_21d": 2.199
}
```

NOT a list like `"label_std": [0.727, 1.477, 2.199]`. If the meta file uses the
list shape, the loader silently falls back to hardcoded defaults.

## Inference discovery convention

`equity_model.py` discovers bundles by iterating `MODELS_DIR` subdirectories.
It uses the directory name as the symbol key and the lower-cased directory name
as the file-name prefix:

```python
sym = entry.name                               # e.g. "QQQ"
lgbm_h1 = entry / f"{sym.lower()}_lgbm_h1_v1.pkl"   # qqq_lgbm_h1_v1.pkl
meta_path = entry / f"model_meta_{sym.lower()}_v1.json"
```

Therefore the directory MUST be named exactly the symbol (e.g. `QQQ`, not
`QQQ_DAILY`), and all files inside must use that same lower-cased prefix.
Mismatching directory names cause silent skips or "missing LGBM files".

## Named Docker volume sync

The production compose uses a named volume, not a host bind-mount:

```yaml
volumes:
  - models:/models:ro
```

Updating files in the host `./models/` is NOT enough. To populate the named
volume on a live VPS:

```bash
# Stop the inference container first so the volume is not busy.
docker stop mmn-inference

# Clear the named volume and copy the new bundles into it.
sudo bash -c 'rm -rf /var/lib/docker/volumes/deploy_models/_data/*'
sudo cp -r models/QQQ models/SMH models/XLF /var/lib/docker/volumes/deploy_models/_data/

# Recreate the container from the rebuilt image.
docker-compose -f deploy/docker-compose.yml up -d inference
```

Verify in inference logs that all ensembles loaded:

```
loaded model bundle for symbol=QQQ ...
loaded model bundle for symbol=SMH ...
loaded model bundle for symbol=XLF ...
equity inference configured: 3 ensembles
```

## label_std magnitude note

For ATR-scaled labels (`clip(fut_ret / (atr/close), -3, 3)`), the saved
`label_std_*` values are the std of those ATR-scaled labels. They are much
larger than raw-return stds (e.g. ~0.73 for 1d, ~1.48 for 5d, ~2.20 for 21d).
The live `predict()` currently returns `blend_z * label_std`, which is still in
ATR-scaled units. To obtain a true raw log-return, the caller must also
multiply by the current `atr_ratio = ATR(14)/close`.

## Disk space check before building

Docker image layers accumulate. Before a Rust engine rebuild, run:

```bash
docker image prune -f
```

In one session this reclaimed 15.58 GB and resolved a "no space left on device"
error during the engine image build.
