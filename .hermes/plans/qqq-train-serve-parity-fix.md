# QQQ Model Train/Serve Parity — Fix Plan

Created: 2026-07-26
Status: PAUSED — resuming next session

## Context

norm_stats bug is fixed (RSI clipping + ADX proxy). Walk-forward IC 0.0784, gate passed.
Three train/serve parity issues remain before deploying to mmn-inference container.

## Completed Work

1. **norm_stats_qqq_v1.json — FIXED**
   - rsi_14: median 5.0 → 55.13, MAD 1e-6 → 14.0
   - trend_adx: median 1.60 → 21.96, MAD 0.475 → 5.56 (real ADX, was ATR proxy)
   - All 8 features in correct order, values in expected ranges
   - Engine loads via `load_named()` (key→positional mapping). Parity OK.

2. **Feature order — MATCHES**
   - Notebook / engine `EQ_FEATURE_NAMES` / `model_meta_qqq_v1.json`: all identical

3. **Normalization formula — MATCHES**
   - Both: `(x - median) / (1.4826 * MAD)`

4. **SEQ_LEN / FEATURE_WINDOW_SIZE — MATCHES**
   - Notebook: `SEQ_LEN=126`, Engine: `FEATURE_WINDOW_SIZE=126`

5. **TCN state_dict — LOADS CLEANLY**
   - 70/70 keys match, `strict=True` load succeeds in inference `QqqTCN`

6. **LightGBM input shape — MATCHES**
   - Training: `model.fit(f_train_norm)` where f_train_norm is (N, 8)
   - Inference: `model.predict(last_row)` where last_row is (1, 8)

7. **Walk-forward results — GATE PASSED**
   ```
   Horizon  1d -> Ens IC: 0.0535 | Ens PnL:   32.48 || LGBM IC: 0.0405 | TCN IC: 0.0388
   Horizon  5d -> Ens IC: 0.0632 | Ens PnL: 143.38 || LGBM IC: 0.0731 | TCN IC: 0.0260
   Horizon 21d -> Ens IC: 0.1184 | Ens PnL: 629.01 || LGBM IC: 0.1777 | TCN IC: 0.0225
   GATE PASSED (Mean IC: 0.0784)
   ```

9. **Colab notebook fixes applied by user:**
   - RSI clipping: global `.clip(-5,5)` → only unbounded features clipped
   - ADX: ATR proxy → real Wilder ADX(14) with +DM/-DM/DX
   - RSI: `ewm` → Wilder SMA-seeded smoothing (matches engine)
   - `wilder_smooth` helper added
   - Debugging metrics: split LGBM/TCN IC+PnL tracking
   - Sanity check ranges updated for real ADX

10. **Rust test updated** (`engine/src/features/equities_v2.rs:658-685`)
    - Old: hardcoded specific values (`0.014625118390912132`, `mad[2] < 1e-5`)
    - New: property-based range checks (ADX median > 5, RSI MAD >= 5, no explosion)
    - 14/15 equities_v2 tests pass (artifact test skips when no norm_stats on disk)

11. **TCN residual SiLU — FIXED (2026-07-27)**
    - `inference/equity_model.py`: removed `self.activation` after residual add
    - Now matches notebook: `return out + residual` (no SiLU post-add)

## Remaining Work — 3 Parity Issues

### ISSUE 1 (CRITICAL): TCN conv padding — train/serve mismatch

**Training** (`notebook cell 12, EquitiesTCN.ResidualBlock`):
```python
self.conv1 = nn.Conv1d(channels, channels, kernel_size=3, padding=dilation, dilation=dilation)
```
Symmetric padding (dilation zeros on BOTH sides). At position t, kernel sees [t-d, t, t+d].
`t+d` is FUTURE data → **lookahead leakage**.

**Inference** (`inference/equity_model.py, QqqTCN.ResidualBlock`):
```python
self.conv1 = CausalConv1d(in_ch, out_ch, kernel_size, dilation)
# CausalConv1d pads (kernel_size-1)*dilation on LEFT only
```
Causal padding. At position t, kernel sees [t-2d, t-d, t]. All past, no leakage.

**Impact**: Same weights, different computation. Walk-forward IC (0.0784) was measured with
leaky architecture → optimistic. Deployed TCN will compute differently (likely worse).

**Fix**: Change notebook's `ResidualBlock` to use left-only causal padding to match inference.
Then rerun on Colab. IC may drop but will be honest + parity.

**Files to change**:
- `models/colab/QQQ_Equities_Model.ipynb` cell 12 — replace `nn.Conv1d` with causal conv

### ISSUE 2 (MEDIUM): Ensemble blending — train/serve mismatch

**Training** (walk-forward, notebook cell 14, L84-92):
```python
l_pred = (lgbm - fold_mean) / fold_std    # z-scored
t_pred = (tcn - fold_mean) / fold_std     # z-scored
ens = (l_pred + t_pred) / 2.0
```
Per-fold z-score blend. Requires prediction history (mean/std over fold).

**Inference** (`inference/equity_model.py:226-233`):
```python
label = 0.5 * tcn_preds[h] + 0.5 * lgbm_preds[h]   # raw blend
raw_log_return = label * atr_ratio
```
Raw weighted average. No z-scoring, no history needed.

**Impact**: If TCN and LGBM produce predictions on different scales, raw blend is dominated
by whichever model has larger variance. Walk-forward IC measured with z-score blend.

**Fix options**:
- (a) Add rolling z-score normalization buffer in inference service (complex, needs history)
- (b) Verify TCN and LGBM prediction magnitudes are similar post-deploy (simple, check first)
- (c) If scales differ significantly, implement (a)

**Files to change** (if needed):
- `inference/equity_model.py` — add prediction history buffer + z-score normalize

### ISSUE 3 (LOW): TCN residual connection — semantic difference
~~Training: `return x + res` (no post-residual activation)~~
~~Inference: `return self.activation(out + residual)` (extra SiLU after residual add)~~

**FIXED (2026-07-27).** `inference/equity_model.py` ResidualBlock.forward now returns
`out + residual` (no SiLU), matching notebook: `x_pad = x_pad + x`.

**Impact**: Same weights produce slightly different activations. Not a loading error (state_dict
matches). Minor forward-pass semantic difference.

**Fix**: Align forward passes — remove post-residual SiLU in inference OR add it to training.

**Files to change**:
- `inference/equity_model.py` ResidualBlock.forward — remove final `self.activation`
  OR
- `models/colab/QQQ_Equities_Model.ipynb` cell 12 — add SiLU after residual add

## Recommended Action Order

1. **Fix Issue 1** (causal padding in notebook) — highest impact, requires Colab rerun
2. **Fix Issue 3** (residual activation) ~~— trivial, do alongside Issue 1~~ **DONE 2026-07-27**
3. **Rerun on Colab** — verify IC still passes gate with causal padding
4. **Deploy new artifacts** to mmn-inference container:
   ```bash
   docker cp models/norm_stats_qqq_v1.json mmn-inference:/models/
   docker cp models/qqq_tcn_v1.pt mmn-inference:/models/
   docker cp models/qqq_lgbm_h1_v1.pkl mmn-inference:/models/
   docker cp models/qqq_lgbm_h5_v1.pkl mmn-inference:/models/
   docker cp models/qqq_lgbm_h21_v1.pkl mmn-inference:/models/
   docker restart mmn-inference
   # Wait 30s, then verify:
   curl -s localhost:9080/api/status  # |pred_1d| should be < 10
   ```
5. **Investigate Issue 2** (blending) — check prediction magnitudes post-deploy
6. **Update Rust test** — already done (property-based checks, will pass with new artifact)
7. **Run `graphify update .`** — refresh knowledge graph after code changes

## Key File Locations

- `models/colab/QQQ_Equities_Model.ipynb` — training notebook (cell 8: features, cell 10: normalize, cell 12: TCN, cell 14: execution)
- `models/norm_stats_qqq_v1.json` — normalization artifact (fixed)
- `models/model_meta_qqq_v1.json` — model metadata (schema v2)
- `models/qqq_tcn_v1.pt` — TCN checkpoint (70 state_dict keys)
- `models/qqq_lgbm_h{1,5,21}_v1.pkl` — LightGBM models
- `engine/src/features/equities_v2.rs` — Rust feature computation + norm_stats loader + tests
- `inference/equity_model.py` — Python inference service (TCN + LGBM ensemble)
- `engine/src/scheduler.rs` — orchestrates feature computation + normalization + ZMQ inference call

## Engine Guard (optional tightening)

`equities_v2.rs:198` has `if scale < 1e-12 { 0.0 }` — too loose. With the fixed norm_stats
this won't trigger (MADs are all > 0.1). Could tighten to `1e-3` as defense-in-depth but
not blocking. The property-based Rust test now catches bad artifacts.

## Walk-Forward Results (Current Run)

```
=== WALK-FORWARD RESULTS ===
Horizon  1d -> Ens IC: 0.0535 | Ens PnL:   32.48 || LGBM IC: 0.0405, PnL:   18.04 | TCN IC: 0.0388, PnL:    7.64
Horizon  5d -> Ens IC: 0.0632 | Ens PnL: 143.38 || LGBM IC: 0.0731, PnL: 209.03 | TCN IC: 0.0260, PnL:   91.73
Horizon 21d -> Ens IC: 0.1184 | Ens PnL: 629.01 || LGBM IC: 0.1777, PnL: 887.87 | TCN IC: 0.0225, PnL: 275.61

GATE PASSED (Mean IC: 0.0784). Minimum Horizon Equity PnL: 32.48.
norm_stats sanity check PASSED.
```

Note: These IC numbers were measured with symmetric (leaky) TCN conv padding and z-score
blending. After fixing to causal padding, IC will likely drop. LGBM IC is strong standalone
(0.04-0.18) so ensemble should still pass the 0.03 gate even if TCN contribution drops.
