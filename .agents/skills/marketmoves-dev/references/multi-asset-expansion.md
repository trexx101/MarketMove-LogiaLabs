# Multi-Asset Expansion — Reference (PLTR and beyond)

Session work captured 2026-08-05. Companion to `.hermes/plans/2026-08-05_pltr-multi-asset-expansion.md`.

## Why this is non-trivial even though the wiring is symbol-agnostic

The engine paths (`engine/src/bridge.rs::predict_v3`, `engine/src/api/chart.rs`, the scheduler, executor) accept any symbol through `cfg.symbol` + `cfg.short_symbol`. The inference service (`inference/equity_model.py`) loads any `.pt`/`.pkl` paths the env vars point at. **The plumbing is universal.**

What is NOT universal:
1. **Learned weights** — `qqq_tcn_v1.pt` and the 3 LGBMs encode "QQQ behavior under QQQ regime structure."
2. **Norm stats** — `norm_stats_qqq_v1.json` applies QQQ median/MAD to every input. PLTR `rvol_20d=1.5` becomes `(1.5 - 0.96) / (1.4826 × 0.19) ≈ +1.9σ` — a regime the model has rarely seen.
3. **`tlt_corr_20d` semantics** — code is generic (asset vs TLT 20d Pearson), but the model's learned response was fitted to QQQ-vs-TLT dynamics. PLTR-vs-TLT is structurally different.

## Option A: PLTR-only retrain (recommended Phase 1)

**What stays the same:**
- All 8 features (asset-agnostic by construction except `tlt_corr_20d` semantics)
- Rust pipeline (`compute_equity_features()` parameter is just named `qqq`)
- Notebook hyperparameters (SEQ_LEN=126, IC_GATE=0.03, MAG_CLIP=3.0, MAD_FLOORS)
- Inference wire protocol (8-dim feature window)

**What changes:**
- Notebook: `symbols=['PLTR', 'TLT', '^VIX']` instead of `['QQQ', ...]`
- Walk-forward: `train_size = 2*252, step_size = 126` (NOT 5y/1y — see below)
- Artifact prefix: `pltr_tcn_v1.pt`, `pltr_lgbm_h{1,5,21}_v1.pkl`, `norm_stats_pltr_v1.json`
- Env vars: `SYMBOL=PLTR`, `SHORT_SYMBOL=PLTD`, `TCN_PATH=pltr_tcn_v1.pt`, `LGBM_H{1,5,21}_PATH=pltr_lgbm_h{1,5,21}_v1.pkl`, `NORM_STATS_PATH=norm_stats_pltr_v1.json`

**No Rust changes for Option A.**

## The walk-forward constraint (the silent showstopper)

Notebook walk-forward:
```python
SEQ_LEN = 126
HORIZONS = [1, 5, 21]
EMBARGO_DAYS = max(SEQ_LEN, 21) + 10  # = 136
train_size = 5 * 252   # 1260 bars
step_size = 252        # 1 year
```

| Asset | History post-warmup | Walk-forward folds | IC gate reliability |
|---|---|---|---|
| QQQ | ~25y | ~15 | High — 0.03 IC on 15 folds is robust |
| PLTR | ~5.2y | 0–1 | **Useless** — 0.03 IC on 1 fold with N=252 is noise (SE ≈ 0.06) |

**The current gate structure cannot validate PLTR.** Three honest workarounds:
1. Shrink windows to `train_size=2*252, step_size=126` → ~5 folds, SE on mean IC ≈ 0.018.
2. Replace with `TimeSeriesSplit(n_splits=5, gap=136)` — different question than what QQQ was tested with.
3. Skip the gate, paper-trade immediately for real OOS data.

**Recommendation:** Workaround 1 + 3 in parallel. Don't deploy on 1-fold IC.

## Option D: sector-relative features (recommended Phase 2, only if A clears gate)

Add 3 features (8 → 11), reference tech sector via XLK:

| New feature | Definition | Captures |
|---|---|---|
| `corr_vs_sector_20d` | 20d rolling Pearson: PLTR vs XLK | Stock-specific vs sector-wide moves |
| `relative_strength_20d` | `(pltr/pltr[t-20]) - (xlk/xlk[t-20])` | Outperformance vs sector over 20d |
| `sector_relative_volume` | `pltr_vol / mean(xlk_vol[t-20:t])` scaled | Idiosyncratic attention proxy |

**NOT added:** beta to sector (correlated with relative_strength and corr_vs_sector), sector-relative drawdown (captured indirectly).

### MAD_FLOORS entries for new features

| New feature | Empirical MAD | Floor |
|---|---|---|
| `corr_vs_sector_20d` | ~0.15–0.20 | 0.10 |
| `relative_strength_20d` | ~0.05 | 0.01 |
| `sector_relative_volume` | varies | 0.10 |

### Absolute vs relative labels — architectural decision

If you train on absolute returns but subtract sector returns from features, the labels have a residual sector component the features can't explain — noise added to training.

Cleaner option:
```python
pltr_ret = (df['close'].shift(-h) - df['close']) / df['close']
xlk_ret = (df['xlk_close'].shift(-h) - df['xlk_close']) / df['xlk_close']
relative_ret = pltr_ret - xlk_ret
labels[f'mag_{h}d'] = np.clip(relative_ret / (vol_scale + 1e-6), -MAG_CLIP, MAG_CLIP)
```

This commits to "predict PLTR's relative outperformance vs sector." Changes the meaning of `pred_1d` from absolute to relative. Strategy layer (thresholds, position sizing, inverse-ETF pair) must be re-tuned. **Cannot have both cleanly.**

### Rust pipeline changes for Option D

Extend `compute_equity_features()`:
```rust
pub fn compute_equity_features(
    qqq: &[EquityCandle],          // becomes "primary asset"
    vix_close: Option<&[f64]>,
    tlt_close: Option<&[f64]>,
    sector_close: Option<&[f64]>,   // NEW: XLK-aligned
    sector_volume: Option<&[i64]>,  // NEW
)
```

Ingest XLK daily into `equity_candles` table. Update Python parity test fixture in `engine/tests/fixtures/equity_feature_parity.json`.

**Engineering estimate:** 5–6 days total (2–3 days Rust + Python parity, 1 day training notebook, 1–2 days Colab retraining + walk-forward).

## Option D-Pool (only when ≥3 tickers validated)

Pooled training across PLTR + NVDA + MSFT + AAPL + ORCL + CRM + ADBE + AVGO + CSCO + TXN. ~10 tickers × 15y ≈ 38k bars.

**Critical pitfalls:**
1. **Sequence boundary leakage** — TCN uses 126-bar sequences; naive concatenation leaks last-bars-of-NVDA into first-bars-of-PLTR. Per-ticker sequences with hard reset required.
2. **Per-ticker norm stats** — NVDA 80% vol vs CSCO 25%; pooling MAD collapses relative magnitudes. Compute per ticker, normalize independently.
3. **Calendar-based walk-forward** — OOS by date, not sequence index.
4. **Survivorship bias** — pool only tickers that existed at training start.

## Implementation sequencing

```
WEEK 1 — OPTION A
  Day 1: Ingest PLTR daily (Yahoo/Moomoo)
  Day 1: Copy notebook, change symbols + shrink walk-forward
  Day 2: Colab run, walk-forward validation
  Day 2: If gate passes → export pltr_* artifacts
  Day 2-3: Deploy to paper mode in parallel with QQQ

WEEK 1-2 — PAPER TRADING
  Run PLTR model in paper mode alongside QQQ
  Track pred vs realized over 3+ months
  If realized Sharpe > 0 → proceed to Option D
  If realized Sharpe < 0 over 3 months → debug or stop

WEEK 3-4 — OPTION D-SOLO
  Day 1-2: Rust feature pipeline changes (sector ETF ingestion, parity)
  Day 2: Update notebook with new features + MAD_FLOORS entries
  Day 2-3: Retrain PLTR with 11-feature set
  Day 3: Compare IC and importance ranking to Option A's 8-feature result
  Day 4: Decide based on importance ranking whether to deploy D-Solo

WEEK 4+ — OPTION D-POOL (only if 3+ tickers validated)
  Add NVDA, MSFT, AAPL models first via Option A
  Each: 1-2 days Colab + IC gate + paper trade
  After 3+ validated: design pooled scheme with per-ticker norm stats
  Engineering lift: 1-2 weeks for data pipeline + leakage handling
```

## Prioritization by impact-per-day-of-effort

1. **Path 1 (shrink walk-forward) + Path 3 (paper trade)** — 2 days. Get IC number + real OOS.
2. **Fix Divergence 2 (drawdown source)** — 1 hour Rust change. Free correctness win.
3. **Fix Divergence 1 (blend strategy)** — 1 day. Backtest metric must match what live trading executes.
4. **Option D-Solo** — 5–6 days. Only if Path 1+3 shows PLTR has *some* signal worth refining.

**NOT prioritized:**
- Divergence 3 (clipping) unless PLTR data shows clipped values matter
- D-Pool until ≥3 tickers validated
- TimeSeriesSplit (different question than what QQQ was tested on)

## Open questions for the user

1. **Paper-mode QQQ history:** If 1+ months of `pred_1d` outputs and realized returns exist, compute actual deployed-model IC vs notebook-reported IC. Tells us whether Divergence 1 is materially hurting live performance.
2. **PLTR qualitative confidence:** Has PLTR shown regime change in last 6 months that would lower the IC gate bar? Domain knowledge should weight into the decision.
3. **Sector ETF choice:** Default XLK (broad tech). Alternative IGV (software-focused, narrower).
4. **Absolute vs relative targets for Option D:** Architectural decision affecting strategy-layer tuning.
5. **Pre-2024-12 PLTR shorts:** PLTD inception is 2024-12-10. (a) skip those rows for short leg, (b) use TECS/TECZ as proxy, (c) accept the gap.
