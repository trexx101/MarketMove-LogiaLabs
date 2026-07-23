# Wave C — QQQ Equities Model: Planning Prompt for Gemini 3.1 Pro

You are a senior quant researcher + systems engineer. Your job is to FLESH OUT the implementation
plan for **Wave C** of the MarketMoves project: training a daily-horizon QQQ equities model
(LightGBM + TCN) with a strict walk-forward evaluation gate, fully aligned to the already-shipped
Wave A (data) and Wave B (features) contracts. Read every constraint below as a HARD requirement
unless explicitly marked "open decision". Do not invent data, APIs, or code that contradicts the
locked contracts. Where you propose changes, show exactly how they preserve the existing design.

**Expected deliverables:** (1) a precise, implementation-ready plan, AND (2) a single self-contained
Google Colab training script (`train_equities_colab.py`, paste-ready into one cell) that trains the
Wave C model end-to-end and exports the artifacts. Both are required — see §7/E2 and §9.

---

## 1. PROJECT CONTEXT (locked facts)

- Pivot: BTC/hourly crypto model (V2 TCN) showed walk-forward OOS mean IC ≈ 0 (no alpha) and is
  DORMANT. Project pivoted to **QQQ daily equities** via Moomoo brokerage.
- Wave A (DONE): equities data engine. `backfill_equities()` seeds 11 Yahoo symbols
  (QQQ, AAPL, MSFT, NVDA, GOOG, AMZN, META, TSLA, TLT, GLD, UUP) + 3 FRED macro series
  (VIXCLS→$VIX, DGS10→$UST10Y, DTWEXBGS→$DXY). Data lands in SQLite `equity_candles`
  (PK symbol,ts; ts = midnight UTC of trading day). Daily supervisor: `run_equities_ingestion()`.
- Wave B (DONE): 8-feature pipeline in `engine/src/features/equities_v2.rs`, 14 passing tests.
- Wave C (THIS): LightGBM+TCN, horizons 1d/5d/21d, walk-forward 5y/1y, deploy gate IC>0.03
  (locked — see decision D4).
- Wave D (next): paper trading. Wave E: live.

## 2. LOCKED FEATURE CONTRACT (Wave B) — DO NOT REORDER/REMOVE

`compute_equity_features(qqq: &[EquityCandle], vix_close: Option<&[f64]>, tlt_close: Option<&[f64]>)`
returns one `EquityFeatureRow` per daily candle. Fixed-order vector (indices 0..7), produced by
`to_array()`:

| idx | name | definition | range |
|-----|------|-----------|-------|
| 0 | trend_slope | ln(SMA50[t] / SMA50[t-20]) | any |
| 1 | trend_adx | Wilder ADX(14), trend strength | 0–100 |
| 2 | rsi_14 | Wilder RSI(14), momentum | 0–100 |
| 3 | vix_regime | bucketed ^VIX: <=0 or <18 → 0 (calm), <25 → 1 (normal), else 2 (stress) | {0,1,2} |
| 4 | tlt_corr_20d | 20-day rolling Pearson corr(QQQ close, TLT close) | -1..1 |
| 5 | rvol_20d | volume[t] / mean(volume[t-20..t]) | >=0 |
| 6 | gap_pct | (open[t] - close[t-1]) / close[t-1] | any |
| 7 | drawdown_from_50d_high | (close[t] - max(high[t-50..t])) / max(...) | <=0 |

- Normalization: **robust median/MAD** via `EquityNormStats` → `(x-median)/(1.4826*MAD)`,
  MAD≈0 ⇒ 0.0. Serialized to `models/norm_stats.json` (median[], mad[]). Must be used identically
  at TRAIN and INFERENCE.
- Warmup: missing lookback ⇒ 0.0 (never NaN). `vix_close`/`tlt_close` must be timestamp-aligned to
  `qqq`; if missing/short, those features are 0.0.
- `EQ_FEATURE_DIM = 8`. Training/inference MUST consume this exact 8-vector in this order.

## 3. LOCKED MODEL BACKBONE (port from crypto V2 `training/train_tcn.py`)

Reuse the proven architecture; only change `in_dim` and horizons:

- `CausalConv1d` (no look-ahead) + 7× `ResidualBlock` with dilations [1,2,4,8,16,32,64],
  `GroupNorm(1, ch)`, `SiLU`, dropout=0.1, residual 1×1.
- `input_proj: Linear(in_dim → hidden_dim=64)`; backbone; 3 horizon heads
  `Linear(hidden_dim → hidden_dim//2) → SiLU → Dropout → Linear(→1)`.
- Adaptive `loss_weights` (softmax) over the 3 heads; `SmoothL1Loss` (Huber-like, MAG_CLIP=3.0).
- Optimizer: AdamW(lr=5e-4, weight_decay=1e-4); `OneCycleLR(pct_start=0.3)`; grad-clip 1.0;
  epochs=150; early-stop on val loss (patience 10). `device` cuda/cpu auto.
- The crypto TCN consumes a **window** of shape `(seq_len=72, in_dim)` (72 hourly bars). For
  daily equities you must define `seq_len_daily` (OPEN DECISION D2).

## 4. LOCKED LABEL SCHEME (port from `training/labels.py`)

- `volatility_scaled_labels`: ATR-based penetration barrier `k = c * ATR/close`; direction = first
  touch of up/down barrier within horizon; magnitude = time-weighted + penetration-scaled, clipped to
  MAG_CLIP=3.0. Calibrate `barrier_c` to ~50% penetration (`calibrate_barrier_c`).
- Daily adaptation: compute on **daily** OHLC (horizons in trading DAYS, not hours). For QQQ daily
  the horizons are **1, 5, 21 trading days** (matching `equity_predictions.pred_1d/5d/21d`).
- Keep `embargo` to prevent leakage (OPEN DECISION D2: embargo in days = max(seq_len_daily, max_horizon_days)+buffer).
- Return per-horizon `(dirs, mags)`; the model predicts the continuous magnitude (sign carries direction).

## 5. LOCKED WALK-FORWARD + DEPLOY GATE

- Crypto used `TimeSeriesSplit(n_splits=5, gap=embargo)`. For daily QQQ the target scheme is
  **expanding/rolling walk-forward: train 5 years, validate 1 year, step 1 year** (OPEN DECISION D3:
  rolling vs expanding window; number of folds over available history ~1999→today).
- Evaluate each fold OOS: **IC = Pearson(pred, true_mag)** per horizon; equity = cumsum(sign(pred)*true).
- **Deploy gate (HARD):** train final model ONLY if mean OOS IC > gate AND mean OOS equity > 0 across
  folds. Gate value: **0.03 (LOCKED)** — matches the committed `train_tcn.py` `ic_gate = 0.03`.
- If gate fails: print "NO EDGE — DO NOT DEPLOY" and STOP (mirror `check_deploy_gate`).

## 6. LOCKED SERVING / STORAGE CONTRACT (alignment target)

- DB `equity_predictions`: `pred_1d, pred_5d, pred_21d, regime, features_json, source`.
  `source` should be like `qqq_tcn_v1` / `qqq_ens_v1`.
- The existing `bridge.rs` has `predict` (legacy, [[f64;3]]) and `predict_v2` ([[f64;6]],
  schema_version=2, DORMANT). Wave C adds a NEW equities path (e.g. `predict_v3`) that sends the
  8-dim (or extended) window + `schema_version` and receives `pred_1d/pred_5d/pred_21d`.
- The live `strategy.rs` (`next_position`) currently consumes `pred_4h`+`pred_24h` with a 200-bar
  SMA regime filter — that is the CRYPTO contract and must NOT be silently reused. Wave C's output
  is daily. **Wave C also owns the strategy/bridge rewrite**: redefine `Prediction`/`next_position`
  to consume `pred_1d`/`pred_5d`/`pred_21d` (daily horizons) with a daily-regime filter (e.g. 200-**day**
  SMA, or the new vix_regime feature). Wave D (paper) then only implements execution against the new
  daily DTO — it must not re-litigate the contract. Your plan must specify the exact daily prediction
  DTO and the new `next_position` signature so C→D is seamless and the crypto 1h/4h/24h path is
  cleanly retired (kept dormant, not mutated).

## 7. YOUR TASKS (produce a concrete plan)

A) **Architecture**: specify the LightGBM config (objective, n_estimators, depth, learning_rate,
   subsample, handling of the categorical vix_regime) AND the TCN port (in_dim, seq_len_daily,
   horizon heads). Show how both consume the EXACT 8-feature contract.

B) **Ensemble**: default = weighted average of z-scored LightGBM and TCN predictions per horizon,
   weights tuned on walk-forward OOS IC. Confirm or revise; show the math.

C) **Accuracy improvements that DO NOT break the design** (this is the core ask). For each, state:
   what it changes, the expected IC/robustness gain, and the EXACT mechanism that keeps the
   Wave-B 8-feature contract / norm-stats / serving DTO intact. Candidates to evaluate (accept,
   reject, or extend each with rationale):
   - Triple-barrier / asymmetric daily labels instead of single-penetration.
   - Per-feature robust clipping (fat-tail handling) at the norm layer (not changing feature defs).
   - Expanded walk-forward (5y/1y rolling) + stricter embargo.
   - LightGBM feature-importance pruning of noise features (without dropping base-8 indices).
   - Regime-conditioned stacking using vix_regime (feature 3) as a gating/condition var.
   - Class/penetration re-weighting (focal/weighted loss) for neutral-heavy labels.
   - Bootstrap prediction intervals to harden the deploy gate.
   - Any OTHER improvement you judge material — but it must preserve the base-8 contract or be an
     additive EXTENSION (see open decision D1).

D) **Alignment guarantees**: enumerate the explicit locks that keep training↔inference↔serving
   consistent (feature order, norm-stats JSON, schema_version, prediction DTO, deploy gate). Call out
   the current mismatches (crypto 1h/4h/24h strategy vs equities 1d/5d/21d; FEATURE_DIM=6 crypto vs
   8 equities; `next_position` pred_4h/pred_24h vs daily pred_1d/pred_5d/pred_21d) and how Wave C
   resolves them WITHOUT a mid-flight contract break (retire crypto path dormant, introduce
   `predict_v3` + daily `next_position`).

E) **Reproducibility / script structure**: mirror `train_tcn.py` (fetch → calibrate → build matrix →
   labels → walk-forward → gate → train_full → export `models/*.pt` + `norm_stats_*.json` +
   `model_meta_*.json`). Specify the new training script(s) for equities (e.g. `train_equities.py`)
   and what imports from `labels.py` it reuses vs rewrites for daily.

E2) **Google Colab training script (REQUIRED deliverable)**: along with the plan, you MUST produce a
    single, **self-contained, Colab-ready** `train_equities_colab.ipynb`-style script (delivered as a
    single `.py` cell-block file `train_equities_colab.py` the user pastes into a Colab notebook
    cell) that actually trains the Wave C model end-to-end. Requirements:
    - Header cell installs deps: `!pip install torch scikit-learn pandas numpy tqdm scipy lightgbm yfinance`.
    - Mounts Google Drive (`from google.colab import drive; drive.mount('/content/drive')`) and uses
      `/content/drive/MyDrive/QuantData/QQQ_DAILY` for data + model export.
    - **Data acquisition inside the script**: pulls QQQ (+ constituents) via `yfinance` (auto
      `pip install yfinance`), and VIX/UST10Y/DXY via FRED CSV (`fred.stlouisfed.org/graph/fredgraph.csv`)
      or `pandas_datareader` — no manual upload. Falls back gracefully if a series is missing.
    - Reuses the EXACT Wave B 8-feature definitions (§2) computed in Python (so the Colab features
      match `equities_v2.rs` to the decimal — include a parity printout comparing a few rows to the
      Rust contract if possible, else document the equivalence). Robust median/MAD normalization.
    - Daily labels via the §4 scheme (ATR-penetration, calibrated barrier_c, horizons in TRADING DAYS
      1/5/21). Include the `volatility_scaled_labels` + `calibrate_barrier_c` port (daily).
    - Walk-forward (rolling 5y/1y, embargo per D2) + deploy gate IC>0.03 AND equity>0 (§5). On pass:
      train final LightGBM + TCN ensemble and export `models/qqq_ens_v1_*.pt`,
      `norm_stats_*.json`, `model_meta_*.json` to Drive. On fail: print "NO EDGE — DO NOT DEPLOY".
    - Prints walk-forward IC/equity per fold + final feature-importance, so the user can read edge
      quality directly from Colab output.
    - Must run on Colab free GPU (`device='cuda' if torch.cuda.is_available() else 'cpu'`), with
      TCN `hidden_dim`/batch defaults that fit Colab memory.

F) **Future enhancement** (explicitly requested): propose ONE concrete, scoped future improvement
   (e.g. Wave C+ or a D-proactive feature) that builds on this design WITHOUT breaking the base-8
   contract — e.g. an EXTENDED feature schema_version adding cross-sectional constituent returns
   (AAPL/MSFT/NVDA…), FRED macro ($VIX/$UST10Y/$DXY) levels/trends, or an LLM regime head
   (the `llm.rs` hourly cache exists but is crypto-framed — propose an equities reframe). Specify
   the schema_version bump + backward-compat rule (base-8 always at indices 0..7).

## 8. OPEN DECISIONS (state your recommended default + rationale; user may override)

- D1 Feature scope: (a) strict base-8 only, or (b) base-8 IMMUTABLE + EXTENDED additive dim
  (schema_version bump, base-8 at 0..7). RECOMMEND (b).
- D2 TCN `seq_len_daily` and embargo in days. RECOMMEND seq_len ~126 trading days (≈6mo) with
  embargo = max(seq_len, 21)+buffer. Justify vs crypto's 72.
- D3 Walk-forward window: rolling 5y/1y vs expanding. RECOMMEND rolling.
- D4 IC gate: LOCKED at 0.03 (matches committed `train_tcn.py` `ic_gate = 0.03`).

## 9. OUTPUT FORMAT (so the plan is directly actionable)

Return BOTH:
(A) The structured plan (sections 1–8 below), AND
(B) The full runnable Colab script `train_equities_colab.py` as a fenced code block — paste-ready
    into a single Colab cell, satisfying E2 above (deps, Drive mount, yfinance+FRED fetch, 8-feature
    parity, daily labels, walk-forward + gate IC>0.03, LightGBM+TCN ensemble export). It MUST execute
    end-to-end on Colab free GPU.

Plan structure (section A):
1. Decisions (D1–D4 with chosen values + 1-line rationale each)
2. Model Architecture (LightGBM params + TCN port spec table)
3. Ensemble Method (formula + weight-tuning)
4. Accuracy Improvements (table: improvement | changes | gain | design-safety mechanism | accept/reject)
5. Alignment & Contract Locks (numbered list; include a "current mismatches → resolution" subsection)
6. Training Script Structure (file list + reuse/rewrite map vs labels.py/train_tcn.py)
7. Future Enhancement (one, scoped, with schema_version + compat rule)
8. Risks / Open Questions for the user
Keep it precise and implementation-ready; cite the specific file/contract you are preserving.
