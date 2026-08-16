# PLTR Multi-Asset Expansion Plan

> **Status:** Analysis only. No code changes committed in this session.
> **Goal:** Extend MarketMoves from QQQ-only to support large-cap tech tickers (starting with PLTR) using the existing infrastructure, while preserving the proven QQQ model and deploy-gate discipline.

---

## Executive Summary

The current QQQ engine is **three layers QQQ-specific** — weights, norm stats, and feature semantics — but the surrounding architecture (inference path, feature computation, scheduler, executor) is already symbol-agnostic. PLTR is mechanically reachable. The constraint is **data depth**: PLTR has ~5.2 years of usable history vs QQQ's 25y, which collapses the walk-forward IC gate from "robust on 15 folds" to "noise on 0–1 folds."

**Recommended sequencing:** Option A (PLTR-only retrain with current pipeline + shrunk walk-forward windows) → paper-trade in parallel → Option D-Solo (add sector-relative features) only if A clears the gate → Option D-Pool (multi-ticker pooled training) only when you have ≥3 validated tickers.

The session identified one pre-existing bug (notebook vs deployed inference blend mismatch) and two minor divergences (drawdown source, clipping) that should be addressed but do not block PLTR work.

---

## Part 1 — QQQ Specificity Audit (the three layers)

### Layer 1: Learned Weights

The deployed model (`models/qqq_tcn_v1.pt` + 3 LightGBMs) was trained on QQQ daily candles only. The TCN's 7 residual blocks and the LightGBM heads encode "QQQ's behavior under this regime structure." PLTR has a fundamentally different regime structure (single-name vs index, higher idiosyncratic vol, different sector beta). The weights will not transfer.

### Layer 2: Normalization Statistics — `models/norm_stats_qqq_v1.json`

The 8 raw features get z-scored by QQQ's median/MAD before inference. Reading the actual artifact:

| Feature | QQQ median | QQQ MAD |
|---|---|---|
| `trend_slope` | 0.0146 | 0.0178 |
| `rsi_14` | 55.13 | 14.0 (MAD_FLOOR hit) |
| `tlt_corr_20d` | -0.0054 | 0.419 |
| `rvol_20d` | 0.959 | 0.186 |

PLTR's distributions are wildly different. PLTR `rvol_20d=1.5` under QQQ's norm becomes `(1.5 - 0.96) / (1.4826 × 0.19) ≈ +1.9σ` — a regime the QQQ-trained model has rarely seen and never learned a calibrated response for. Even if the architecture "generalized" perfectly, you still need PLTR's own median/MAD.

### Layer 3: `tlt_corr_20d` Feature Semantics

The Rust code at `engine/src/features/equities_v2.rs:120` computes:

```rust
let tlt_corr: Vec<f64> = match tlt_close {
    Some(t) if t.len() == n => rolling_correlation(&closes, t, 20),
    _ => vec![0.0; n],
};
```

The parameter name is `qqq` but the function takes any `&[EquityCandle]`. So for PLTR this becomes "PLTR vs TLT 20d correlation." The label `tlt_corr_20d` is asset-agnostic by code, but the *learned mapping* in the model is fitted to QQQ-vs-TLT dynamics. Different from PLTR-vs-TLT.

`vix_regime` (VIX bucketed 0/1/2) is genuinely macro-agnostic and transfers fine.

---

## Part 2 — Architecture Verdict

The wiring IS symbol-agnostic. Reading the code:

- `engine/src/config.rs:191` — `let symbol = env_or("SYMBOL", "BTC/USD");`
- `engine/src/bridge.rs:214` — `predict_v3` sends a generic 8-dim window; the wire protocol has no symbol field
- `inference/equity_model.py` — loads whatever `.pt`/`.pkl` paths the env vars point at; no symbol baked into the model loader

**Conclusion:** If `SYMBOL=PLTR` is set and PLTR candles are ingested into `equity_candles`, the engine already supports it. The break is the model artifacts, not the plumbing.

---

## Part 3 — Option A: PLTR-Only Retrain

### Feature Audit: All 8 are Asset-Agnostic by Construction

| Feature | Asset-agnostic? | Reasoning |
|---|---|---|
| `trend_slope` | ✅ | Pure log ratio of SMA50 over 20 bars. Scale-invariant. |
| `trend_adx` | ✅ | Wilder ADX on own OHLC. 0–100 bounded. |
| `rsi_14` | ✅ | Wilder RSI on own closes. 0–100 bounded. |
| `vix_regime` | ✅ | Pure macro bucketing. |
| `tlt_corr_20d` | ⚠️ generic-by-code, learned-response-is-QQQ | Asset-vs-TLT 20d correlation. Code doesn't care about asset name. |
| `rvol_20d` | ✅ | Volume / 20d mean volume. Scale-invariant. |
| `gap_pct` | ✅ | Overnight gap ratio. |
| `drawdown_from_50d_high` | ✅ | Log ratio to rolling high. |

The Rust parameter `qqq: &[EquityCandle]` in `compute_equity_features()` is just a name. No QQQ hardcoding in the feature engineering code.

### What Option A Requires

1. **Data ingestion:** PLTR daily from 2020-09-30 via Yahoo/Moomoo. Reuse existing pipeline with `SYMBOL=PLTR`.
2. **Norm stats recomputation:** PLTR-specific median/MAD via the same `robust_normalize()` from the Colab notebook, with the existing `MAD_FLOORS` table preserved.
3. **Walk-forward retraining:** Same notebook (`models/colab/QQQ_Equities_Model.ipynb`), same architecture, same hyperparameters, retrained on PLTR history.
4. **Artifact rename:** `pltr_tcn_v1.pt`, `pltr_lgbm_h{1,5,21}_v1.pkl`, `norm_stats_pltr_v1.json`, `model_meta_pltr_v1.json`.
5. **Engine wiring via env vars:** `TCN_PATH=pltr_tcn_v1.pt`, `LGBM_H{1,5,21}_PATH=pltr_lgbm_h{1,5,21}_v1.pkl`, `NORM_STATS_PATH=norm_stats_pltr_v1.json`.

**No Rust code changes needed for Option A.**

---

## Part 4 — The Hard Constraint: PLTR's Data Depth

### The Walk-Forward Structure Problem

The actual training source is `models/colab/QQQ_Equities_Model.ipynb` (not `training/train_tcn.py`, which is the dormant V2 BTC pipeline). The notebook's validation is:

```python
SEQ_LEN = 126
HORIZONS = [1, 5, 21]
EMBARGO_DAYS = max(SEQ_LEN, 21) + 10  # 136
train_size = 5 * 252   # 1260 bars
step_size = 252        # 1 year
```

Rolling 5-year train, 1-year val, step 1 year. With QQQ's ~25y history that gives ~15 folds. With PLTR's ~5.2y (post-warmup), it gives **0–1 folds**.

Standard error on Pearson r at N=252 OOS bars is ~0.06. The IC gate at 0.03 has SE bigger than itself. **The current gate structure cannot validate PLTR.**

### Three Honest Paths Forward

**Path 1 — Shrink the walk-forward windows (recommended):**
Change `train_size = 2*252, step_size = 126`. With PLTR's 5.2y you get ~5 folds × 126 OOS bars each. Mean IC across 5 folds has SE ~0.018. A mean IC of 0.06 clears the gate with ~99% confidence. Trade-off: shorter training windows mean less regime coverage per fold (PLTR's 2022 bear won't be in the same fold as the 2023–2025 rally).

**Path 2 — TimeSeriesSplit replacement:**
Use `sklearn.model_selection.TimeSeriesSplit(n_splits=5, gap=136)`. Expanding window, not rolling. Better fold count, but a different validation question than what QQQ was tested with — breaks direct IC comparison.

**Path 3 — Skip the gate, paper-trade immediately:**
Train the model, deploy to paper mode, validate on 3–6 months of live performance. Slow feedback loop, but the only path that gives real OOS data.

**Recommended:** Path 1 + Path 3 in parallel. Get an IC number for the static validation, and immediately start paper-trading for real OOS. Don't deploy on 1-fold IC.

---

## Part 5 — Option D: Sector-Relative Features

### What's Actually Sector-Relative

Two distinct design layers:

1. **Feature changes:** Add explicit asset-vs-sector comparisons
2. **Training scheme:** Either single-asset with sector features (D-Solo) or pooled across sector constituents (D-Pool)

### Feature Changes (8 → 11)

The sector ETF for tech is **XLK** (State Street Technology Select Sector SPDR) — broad sector definition, long history, high liquidity.

| New feature | Definition | Captures |
|---|---|---|
| `corr_vs_sector_20d` | 20d rolling Pearson: PLTR vs XLK | Stock-specific vs sector-wide moves |
| `relative_strength_20d` | `(pltr/pltr[t-20]) - (xlk/xlk[t-20])` | Outperformance vs sector over 20d |
| `sector_relative_volume` | `pltr_vol / mean(xlk_vol[t-20:t])` scaled | Idiosyncratic attention proxy |

What's NOT added:
- Beta to sector — correlated with `relative_strength_20d` and `corr_vs_sector_20d`, adds noise at daily horizon
- Sector-relative drawdown — captured indirectly by `relative_strength_20d`

### MAD_FLOORS for New Features

The notebook's `MAD_FLOORS` table prevents catastrophic normalization when a slice has near-zero variance. New entries:

| New feature | Empirical MAD | Floor |
|---|---|---|
| `corr_vs_sector_20d` | ~0.15–0.20 | 0.10 |
| `relative_strength_20d` | ~0.05 | 0.01 |
| `sector_relative_volume` | varies | 0.10 |

### D-Solo vs D-Pool

**D-Solo:** Train PLTR-only with 11 features. Same pipeline, longer Colab run.

**D-Pool:** Train pooled across PLTR, NVDA, MSFT, AAPL, ORCL, CRM, ADBE, AVGO, CSCO, TXN (~10 tickers × 15y = ~38k bars).

**D-Pool critical pitfalls:**
1. **Sequence boundary leakage** — TCN uses 126-bar sequences; concatenating tickers naively leaks last-bars-of-NVDA into first-bars-of-PLTR. Must use per-ticker sequences with hard reset.
2. **Per-ticker norm stats** — NVDA 80% annualized vol vs CSCO 25% vol; pooling MAD collapses relative magnitudes and the model can't distinguish regimes. Compute norm stats per ticker, normalize independently.
3. **Calendar-based walk-forward splits** — OOS evaluation by date, not sequence index.
4. **Survivorship bias** — pool only tickers that existed at training start. Don't retroactively include delisted names.

### The Architectural Decision: Absolute vs Relative Labels

If you subtract sector returns from labels:

```python
pltr_ret = (df['close'].shift(-h) - df['close']) / df['close']
xlk_ret = (df['xlk_close'].shift(-h) - df['xlk_close']) / df['xlk_close']
relative_ret = pltr_ret - xlk_ret
labels[f'mag_{h}d'] = np.clip(relative_ret / (vol_scale + 1e-6), -MAG_CLIP, MAG_CLIP)
```

The model becomes "predict PLTR's relative outperformance vs XLK." This is the natural target for sector-relative features, but **changes the meaning of `pred_1d` from "PLTR's expected log return" to "PLTR's expected log return minus XLK's."** The strategy layer (entry/exit thresholds, position sizing, inverse-ETF pair logic) must be re-tuned. **Cannot have it both ways cleanly** — either commit to relative-strength strategy or accept noise from absolute targets.

### Rust Pipeline Changes for Option D

Extend `compute_equity_features()` signature:

```rust
pub fn compute_equity_features(
    qqq: &[EquityCandle],
    vix_close: Option<&[f64]>,
    tlt_close: Option<&[f64]>,
    sector_close: Option<&[f64]>,   // NEW: XLK-aligned
    sector_volume: Option<&[i64]>,  // NEW: for sector_relative_volume
)
```

Ingest XLK daily into `equity_candles` table alongside PLTR. Update Python parity test in `engine/tests/fixtures/equity_feature_parity.json`. **Engineering estimate: 5–6 days total** (2–3 days Rust + Python parity, 1 day training notebook changes, 1–2 days Colab retraining + walk-forward).

---

## Part 6 — Inverse-ETF Mapping (Tech Sector)

The engine supports short positions via `cfg.short_symbol` (PSQ for QQQ). For PLTR we need a -1x inverse:

| Ticker | Inverse ETF | Issuer | Target | Inception | Notes |
|---|---|---|---|---|---|
| PLTR | **PLTD** | Direxion | -100% | 2024-12-10 | Daily reset, ~20 months history. PLTU is the +2x bull. |
| NVDA | **NVDD** | Direxion | -100% | — | NVDU is +2x bull. Most-traded single-stock bear. |
| TSLA | **TSLS** | Direxion | -100% | — | TSLL is +2x bull. High AUM. |
| AAPL | **AAPD** | Direxion | -100% | — | AAPU is +2x bull. |
| MSFT | **MSFD** | Direxion | -100% | — | MSFU is +2x bull. |
| AMZN | **AMZD** | Direxion | -100% | — | AMZU is +2x bull. |
| META | **METD** | Direxion | -100% | — | METU is +2x bull. |
| GOOGL | **GGLS** | Direxion | -100% | — | GGLL is +2x bull. |
| AMD | **AMDD** | Direxion | -100% | — | AMUU is +2x bull. |
| AVGO | **AVS** | Direxion | -100% | — | AVL is +2x bull. |
| MU | **MUD** | Direxion | -100% | — | MUU is +2x bull. |
| NFLX | **NFXS** | Direxion | -100% | — | NFXL is +2x bull. |
| ORCL | **ORCS** | Direxion | -100% | — | ORCU is +2x bull. |
| PANW | **PALD** | Direxion | -100% | — | PALU is +2x bull. |
| QCOM | **QCMD** | Direxion | -100% | — | QCMU is +2x bull. |
| TSM | **TSMZ** | Direxion | -100% | — | TSMX is +2x bull. |

**Sector-level inverses (alternative to single-stock inverses):**

| Ticker | Direction | Index | Notes |
|---|---|---|---|
| TECL / **TECS** | +3x / -3x | Technology Select Sector | Highest AUM tech sector inverse (~$96M). 0.95% expense ratio. |
| **TECZ** | -1x | Technology Select Sector | Direxion 1x tech bear. Lower vol decay than TECS. |
| SPDN | -1x | S&P 500 | Index-level short. |
| QQQD | -1x | Magnificent 7 | Direxion Mag-7 bear. |

**PnL formula for single-stock inverses** (same as PSQ for QQQ):
```
PLTD_return ≈ -(PLTR_return)
PLTD_current ≈ PLTD_entry * (1 - PLTR_return)
PnL = PLTD_entry - PLTD_current ≈ PLTD_entry * (PLTR_current/PLTR_entry - 1)
```

**Caveat for PLTR specifically:** PLTD inception is 2024-12-10 — only ~20 months of history. The PLTR inverse-ETF PnL calculation during the PLTR model training window (2020-09 → 2024-12) cannot be validated against PLTD. PLTR short positions pre-2024-12 have no inverse instrument to trade against. Options:
- Train only on 2024-12 onwards for the short leg (loses 4 years of bear-market data)
- Use sector inverse (TECS or TECZ) as a proxy — sacrifices precision for coverage
- Accept the gap and skip the short leg for pre-Dec-2024

**Single-stock inverse decay:** All -1x products reset daily. Over multi-day holds they drift. For a daily strategy (which QQQ is, with `pred_5d_filter` filter), this is mostly negligible, but worth tracking.

---

## Part 7 — Pre-Existing Divergences (Bugs Worth Fixing)

Found while reviewing the actual notebook against the deployed inference. These exist independently of PLTR work but will affect any retraining.

### Divergence 1: Blend Strategy Mismatch (medium severity)

**Notebook (Cell 14):** Per-fold z-score before averaging:
```python
l_pred = (lgbm_preds[:, idx] - lgbm_preds[:, idx].mean()) / (lgbm_std + 1e-8)
t_pred = (tcn_preds[:, idx] - tcn_preds[:, idx].mean()) / (tcn_std + 1e-8)
ens_pred = (l_pred + t_pred) / 2.0
```

**Deployed inference (`inference/equity_model.py`):** Raw weighted blend, no per-window standardization:
```python
label = self.tcn_weight * tcn_preds[h] + self.lgbm_weight * lgbm_preds[h]
raw_log_return = label * atr_ratio
```

If TCN outputs std=0.3 and LightGBM outputs std=0.05 in the same input distribution, deployed blend gives LightGBM essentially zero weight regardless of `lgbm_weight`. The notebook's blend treats them as equal contributors. **The 20.2% CAGR backtest validates the notebook's blend; live trading executes the deployed blend.** These are measuring different things.

**Fix:** Either add per-window stats buffer to deployed inference (matches notebook), or use precomputed per-model calibration scales from a held-out set, or document the divergence and trust paper trading as ground truth.

### Divergence 2: `drawdown_from_50d_high` Source Differs (low-medium severity)

**Notebook (Cell 8):** Uses `df['high'].rolling(50).max()`:
```python
roll_max = df['high'].rolling(50).max()
f['drawdown_from_50d_high'] = ((df['close'] - roll_max) / roll_max).fillna(0)
```

**Rust pipeline (`engine/src/features/equities_v2.rs:425`):** Uses rolling max over `closes`:
```rust
let high = closes[i - window..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
if high > 0.0 {
    out[i] = (closes[i] - high) / high;
}
```

Rust computes over `closes`, notebook computes over `highs`. Small systematic difference (notebook's drawdown is slightly more negative on average). Means the model's training distribution doesn't exactly match inference distribution — same category as the norm-stats issue, but smaller magnitude.

**Fix:** Change Rust line 425 to use the `high` field. `EquityCandle` has `high`. One-line change.

### Divergence 3: Feature Clipping Differs (low severity)

**Notebook:** Clips `['trend_slope', 'rvol_20d', 'gap_pct', 'drawdown_from_50d_high']` to `[-5, +5]`:
```python
clip_cols = ['trend_slope', 'rvol_20d', 'gap_pct', 'drawdown_from_50d_high']
f[clip_cols] = f[clip_cols].clip(lower=-5.0, upper=5.0)
```

**Rust:** No clipping. Live inference can produce values outside the trained range. For QQQ this rarely matters. For PLTR where `gap_pct` can hit ±0.10 on earnings days, clipping becomes more important.

**Fix:** Match training-side clipping in inference (Rust side), or document and accept asymmetry.

---

## Part 8 — Implementation Sequence (Recommended)

```
WEEK 1 — OPTION A (PLTR-only retrain)
  Day 1: Ingest PLTR daily into equity_candles (Yahoo/Moomoo)
  Day 1: Copy notebook, change symbols=['PLTR', 'TLT', '^VIX']
  Day 1: Change train_size=2*252, step_size=126 in walk-forward loop
  Day 2: Colab run, walk-forward validation
  Day 2: If gate passes → export pltr_* artifacts
  Day 2: Set SYMBOL=PLTR, SHORT_SYMBOL=PLTD, point TCN_PATH/LGBM_*_PATH/NORM_STATS_PATH at PLTR artifacts
  Day 2-3: Deploy to paper mode in parallel with QQQ

WEEK 1-2 — PAPER TRADING
  Run PLTR model in paper mode alongside QQQ
  Track: pred vs realized over 3+ months
  If realized Sharpe > 0 over the window → proceed to Option D
  If realized Sharpe < 0 over 3 months → stop, debug, consider whether the IC gate failure was correct

WEEK 3-4 — OPTION D-SOLO (sector features, single asset)
  Day 1-2: Rust feature pipeline changes (compute_equity_features signature, XLK ingestion)
  Day 1: Python parity test extension
  Day 2: Update notebook with new features, MAD_FLOORS entries, vix_regime categorical preserved
  Day 2-3: Retrain PLTR with 11-feature set
  Day 3: Compare IC and importance ranking to Option A's 8-feature result
  Day 4: Decide: deploy D-Solo or stay on A based on importance ranking

WEEK 4+ — OPTION D-POOL (only if 3+ tickers validated)
  Add NVDA, MSFT, AAPL models first via Option A
  Each one: 1-2 days Colab, IC gate, paper trade
  After 3+ validated: design pooled training scheme with per-ticker norm stats
  Engineering lift: 1-2 weeks for data pipeline + leakage handling + walk-forward
```

---

## Part 9 — Prioritization

By impact-per-day-of-effort:

1. **Path 1 (shrink walk-forward windows) + Path 3 (paper trade)** — 2 days. Get an IC number *and* real OOS data.
2. **Fix Divergence 2 (drawdown source)** — 1 hour Rust change. Free correctness win.
3. **Fix Divergence 1 (blend strategy)** — 1 day. Decide on z-score vs raw blend, document, deploy. Backtest metric needs to match what live trading executes.
4. **Option D-Solo** — 5–6 days. Only if Path 1+3 shows PLTR has *some* signal worth refining.

**Things I would NOT prioritize:**
- Fixing Divergence 3 unless PLTR data shows clipped values matter
- Pooled multi-ticker training (D-Pool) until ≥3 tickers are validated
- Switching to TimeSeriesSplit — different question than what QQQ was validated on

---

## Part 10 — Open Questions (require Inah input)

1. **How much paper-mode QQQ history exists?** If 1+ months of `pred_1d` outputs and realized returns are available, we can compute the actual deployed-model IC and compare to notebook-reported IC. Tells us whether Divergence 1 is materially hurting live performance.

2. **Qualitative PLTR confidence:** Has PLTR shown any regime change in the last 6 months that would lower the bar for the IC gate? Deploy-gate failure is asymmetric — a model that passes walk-forward can still be bad in production. Any domain knowledge (recent price action, earnings dynamics, sector momentum) should weight into the decision.

3. **Sector ETF choice for Option D:** Default is XLK (broad tech, liquid, long history). Alternative is IGV (software-focused, narrower). XLK gives more data per fold; IGV is more sector-aligned with PLTR specifically.

4. **Absolute vs relative targets for Option D:** Commit to relative-strength strategy (subtract sector return from labels) or accept noise from absolute targets? This is an architectural decision that affects strategy-layer tuning.

5. **Pre-2024-12 PLTR shorts:** PLTD inception is 2024-12-10. For training data pre-Dec-2024, what's the short instrument? (a) skip those rows for the short leg, (b) use TECS/TECZ as proxy, (c) accept the gap.

---

## Appendix A — Notebook Source Reference

`models/colab/QQQ_Equities_Model.ipynb` — 15 cells, 7 substantive code cells. Key constants:

```python
SEQ_LEN = 126
HORIZONS = [1, 5, 21]
EMBARGO_DAYS = max(SEQ_LEN, 21) + 10  # = 136
IC_GATE = 0.03
MAG_CLIP = 3.0
```

LGBM hyperparameters:
```python
lgb.LGBMRegressor(
    objective='huber',
    n_estimators=100,
    max_depth=6,
    learning_rate=0.01,
    random_state=42,
    categorical_feature=[vix_regime_col_idx]
)
```

TCN training:
- Per fold: 15 epochs, AdamW lr=5e-4, SmoothL1Loss
- Final model: 30 epochs full retrain on all data
- Architecture: 7 residual blocks with dilations [1,2,4,8,16,32,64], hidden_dim=64

Deploy gate:
```python
avg_ic_all = np.nanmean(list(mean_ics.values()))
min_eq = min(all_equities_summed)
if avg_ic_all < IC_GATE or min_eq <= 0:
    return  # GATE FAILED
```

Ensemble blend (NOT what deployed inference does):
```python
# Per-fold z-score before averaging
l_pred = (lgbm_preds - mean) / (std + 1e-8)
t_pred = (tcn_preds - mean) / (std + 1e-8)
ens_pred = (l_pred + t_pred) / 2.0
```

## Appendix B — Artifact Inventory

Current QQQ artifacts in `models/`:
- `qqq_tcn_v1.pt` (751KB)
- `qqq_lgbm_h1_v1.pkl`, `qqq_lgbm_h5_v1.pkl`, `qqq_lgbm_h21_v1.pkl`
- `norm_stats_qqq_v1.json` (8-feature median/MAD dict)
- `model_meta_qqq_v1.json` (schema_version, features, horizons, LGBM paths)
- `colab/QQQ_Equities_Model.ipynb` (training source)

PLTR artifacts (after Option A):
- `pltr_tcn_v1.pt`
- `pltr_lgbm_h{1,5,21}_v1.pkl`
- `norm_stats_pltr_v1.json`
- `model_meta_pltr_v1.json`

XLK artifact (after Option D):
- `xlk_candles` rows in `equity_candles` table (or parallel `equity_sector_candles`)

## Appendix C — Glossary

- **IC (Information Coefficient):** Pearson correlation between model predictions and realized returns. Gate at 0.03 means mean OOS IC across folds must exceed 0.03.
- **MAD (Median Absolute Deviation):** Robust scale estimator. Used in place of std for normalization since it's less affected by outliers.
- **MAD_FLOORS:** Per-feature minimum MAD values to prevent explosive normalization when a training slice has near-zero variance.
- **SEQ_LEN:** Number of consecutive daily bars fed to the TCN as a single input sample. 126 here.
- **EMBARGO_DAYS:** Gap between train and validation windows to prevent label leakage.
- **WALK-FORWARD:** Validation method where you train on past data, predict on future, then advance the window. Mimics live trading.
