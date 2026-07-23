# models/

Model artifacts for **Wave C** — the QQQ daily-equities model (LightGBM + TCN ensemble).
This directory replaces the retired crypto (`MarketMarkovNet`) artifacts.

## Layout

```
models/
├── qqq_tcn_v1.pt              # TCN state dict (8-feature input, 3 horizon heads)
├── qqq_lgbm_h1_v1.pkl         # LightGBM model for the 1-day horizon
├── qqq_lgbm_h5_v1.pkl         # LightGBM model for the 5-day horizon
├── qqq_lgbm_h21_v1.pkl        # LightGBM model for the 21-day horizon
├── model_meta_qqq_v1.json     # schema_version, feature names, horizons, lgbm paths
├── norm_stats_qqq_v1.json     # robust median/MAD per feature (Wave B contract)
└── QQQ_DAILY/
    └── stock_data.csv         # raw training data (1999→today): Date,OHLCV,TLT,VIX
```

All artifacts are **user-supplied** (output of the Colab training script
`train_equities_colab.py`) and are gitignored (`/models/*`, with `!.gitkeep` and
`!README.md` allowed) so they never get committed.

## Feature contract (MUST match `engine/src/features/equities_v2.rs`)

8 fixed-order features, normalized with robust median/MAD:

| idx | name | idx | name |
|-----|------|-----|------|
| 0 | trend_slope | 4 | tlt_corr_20d |
| 1 | trend_adx | 5 | rvol_20d |
| 2 | rsi_14 | 6 | gap_pct |
| 3 | vix_regime | 7 | drawdown_from_50d_high |

`norm_stats_qqq_v1.json` carries `medians`/`mads` keyed by these names; the Rust
`EquityNormStats` loader reads this exact shape: `(x - median) / (1.4826 * MAD)`.

## `model_meta_qqq_v1.json`

```json
{
  "schema_version": 2,
  "features": ["trend_slope","trend_adx","rsi_14","vix_regime",
               "tlt_corr_20d","rvol_20d","gap_pct","drawdown_from_50d_high"],
  "horizons": [1, 5, 21],
  "lgbm_model_paths": {"1": "...qqq_lgbm_h1_v1.pkl", "5": "...qqq_lgbm_h5_v1.pkl",
                        "21": "...qqq_lgbm_h21_v1.pkl"}
}
```

## ⚠️ Engine integration status (read before wiring up)

The artifacts above are complete, but the **Rust engine glue to consume them has not
been written yet** (it is the implementation half of Wave C). Known gaps:

1. **Norm-stats loader mismatch.** `engine/src/normalize.rs` currently loads only the
   OLD crypto schema (`mean`/`std`, 3 features). There is no loader yet for the
   `medians`/`mads` 8-feature `EquityNormStats` shape. The Wave C plan requires adding
   a `predict_v3` path + an `EquityNormStats::load` consumer before this model can run.
2. **No TCN/LightGBM inference in Rust.** The `.pt` and `.pkl` files are loaded by the
   Python inference service, not the Rust engine directly. `inference/config.py` still
   hardcodes `/models/model.pt` + `/models/norm_stats.json` (the dead crypto paths).
   Point the inference service at the `qqq_*` files (or rename per the table above) and
   update `inference/config.py` + `engine/src/bridge.rs` (`predict_v3`) accordingly.
3. **Strategy rewrite pending.** `engine/src/strategy.rs` (`next_position`) still
   consumes `pred_4h`/`pred_24h` (crypto horizons). Wave C must redefine it for
   `pred_1d`/`pred_5d`/`pred_21d` before these predictions are tradeable.

## Retired artifacts (removed)

The following crypto-era files were deleted during the Wave C cleanup and are no longer
referenced by any code path: `market_markov_net_20260720_133508.pt`,
`norm_stats_20260720_133508.json`, `norm_stats.json` (crypto mean/std),
`normalization_stats_20260711_132142.npz`, `Crypto_Markov_Head_V2.ipynb`.
