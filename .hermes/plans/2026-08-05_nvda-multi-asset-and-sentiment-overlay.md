# NVDA Multi-Asset Expansion + Sentiment Risk Overlay

> **Status:** Plan only. No code changes committed in this session.
> **Source of truth:** `models/colab/QQQ_Equities_Model.ipynb` (cells 8, 10, 14). `training/equities_features.py` is a stale helper with its own divergences — do NOT cite it as source.
> **Goal:** Extend MarketMoves from QQQ-only to support NVDA (long) / NVDD (short), wire a real Finnhub sentiment feed, and implement sentiment as a **discrete risk overlay** at the strategy layer (Option 3).

---

## Executive Summary

NVDA is the second ticker. Unlike PLTR (5.2y history → walk-forward IC gate collapses), NVDA has 25+ years of daily history — enough to run the same 15-fold walk-forward validation as QQQ. As a high-vol single name (~50% annualized vol), it gives the vol-scaled model more signal than QQQ. The short leg uses **NVDD** (Direxion -1x NVDA daily) rather than PSQ because PSQ shorts QQQ, not NVDA.

Three independent workstreams, sequenced for risk:

1. **Parity fixes** — three divergences between notebook and Rust inference, identified by reading cells 8/10/14 of the actual notebook.
2. **NVDA/NVDD pipeline** — generalize notebook → ingest → train → walk-forward IC gate → norm stats → paper mode.
3. **Sentiment Option 3 (risk overlay)** — wire `fetch_sentiment` to Finnhub, scheduler calls daily, strategy reads `sentiment_cache` to halve size or force-exit.

Sentiment is a **strategy-layer risk overlay**, not a TCN feature. Rationale: shallow training history, non-stationarity, and continuous-TCN coupling can't be backtested. Discrete overlay is A/B testable and reversible.

---

## Part 1 — Parity Fixes (Immediate, Blocking)

These are the three divergences between the notebook (ground truth) and the deployed Rust/Python pipeline. Verified by reading `models/colab/QQQ_Equities_Model.ipynb` directly.

### 1A. `drawdown_from_50d_high` must use rolling max of `high`, not `close`

**Notebook (cell 8, ground truth):**
```python
# 7: drawdown_from_50d_high
roll_max = df['high'].rolling(50).max()
f['drawdown_from_50d_high'] = ((df['close'] - roll_max) / roll_max).fillna(0)
```

**Rust engine (current, `engine/src/features/equities_v2.rs:418`):**
```rust
let high = closes[i - window..=i].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
```
Uses `closes`, not `high`. Produces systematically less-negative values (never sees intraday high).

**Fix (Rust):**
- File: `engine/src/features/equities_v2.rs`
- Signature: `compute_equity_features` already takes `&[EquityCandle]` which has a `high` field — thread it through.
- Rename `drawdown_from_high(closes, window)` → `drawdown_from_high(highs, closes, window)` using `highs[i-window..=i].max()`.
- Update call site at `equities_v2.rs:133` and `scheduler.rs:124`.

**Validation:**
```bash
cd engine && cargo test --test equity_feature_parity -- --nocapture
```
Expected: max diff for `drawdown_from_50d_high` < 1e-9 after regeneration.

**Critical consequence:** this is a **feature definition change**. The deployed QQQ model was trained on the `close`-based feature. After this fix:
- The parity fixture must be regenerated.
- QQQ must be retrained to keep parity.
- The retrained model must redeploy before NVDA training starts.

**Decision needed:** retrain QQQ as part of 1A (clean), or only fix going forward and let the harness tolerate the divergence (fast, dirty)?

### 1B. Feature clipping is missing in Rust

**Notebook (cell 8, end of `compute_features`):**
```python
# Clip extreme edges for robustness (only on unbounded features)
clip_cols = ['trend_slope', 'rvol_20d', 'gap_pct', 'drawdown_from_50d_high']
f[clip_cols] = f[clip_cols].clip(lower=-5.0, upper=5.0)
```

**Rust engine (current):** no clipping applied after `compute_equity_features`. Outliers in live data (earnings gaps, flash crashes) feed un-clipped values into normalization, producing extreme z-scores that the TCN has never seen.

**Fix:**
- File: `engine/src/features/equities_v2.rs`
- After the row assembly loop, apply `clip(-5.0, 5.0)` to those 4 columns.
- Add a parity test case with synthetic spikes to confirm clipping triggers.

**Validation:** parity fixture regenerated, max diff < 1e-9 for all 8 features.

### 1C. Z-score blend reconciliation

**Notebook (cell 14, walk-forward loop):**
```python
# Z-Score Ensemble Blending
for idx, h in enumerate(HORIZONS):
    lgbm_std = lgbm_preds_aligned[:, idx].std()
    tcn_std = tcn_preds[:, idx].std()
    l_pred = (lgbm_preds_aligned[:, idx] - lgbm_preds_aligned[:, idx].mean()) / (lgbm_std + 1e-8)
    t_pred = (tcn_preds[:, idx] - tcn_preds[:, idx].mean()) / (tcn_std + 1e-8)
    ens_pred = (l_pred + t_pred) / 2.0
```

**Inference service (current, `inference/equity_model.py:185-191`):**
```python
self.tcn_weight = tcn_weight  # default 0.5
self.lgbm_weight = lgbm_weight  # default 0.5
# ...weighted average of raw outputs
```

**Analysis:** The notebook's z-score blend is **per-fold within-batch standardization**. It's not a rolling-history blend — it's standardizing each model's predictions over the validation window, then averaging. For a single live prediction (one feature window → one prediction), this **degenerates to a flat blend** by construction (a single value standardized against itself is 0).

Implication: the notebook's IC numbers are computed against z-score-blended predictions over a fold of ~252 predictions. For a live single prediction, the flat 0.5/0.5 raw blend is the natural extension. The two blends will produce **different** numbers in any regime where the TCN and LGBM disagree in absolute scale — the notebook backtest is on standardized output, live is on raw output.

**Two valid reconciliations:**
- **Option A (reconcile to notebook):** carry a rolling prediction history buffer in inference, replicate the z-score blend per-horizon over the last N (e.g. 60) predictions. Matches notebook exactly.
- **Option B (re-baseline to flat blend):** re-run the notebook walk-forward using the flat 0.5/0.5 raw blend (just delete the z-score standardization), get new IC numbers, accept those as ground truth. Simpler, fewer moving parts.

**Decision needed:** Option A (reconcile) or Option B (re-baseline)?

This blocks NVDA training because we don't know which blend to train against.

### 1D. Optional: MAD_FLOORS are in notebook only

**Notebook (cell 10):**
```python
MAD_FLOORS = {
    'trend_slope': 0.005,
    'trend_adx':   5.0,
    'rsi_14':     14.0,
    'vix_regime':   1.0,
    'tlt_corr_20d': 0.10,
    'rvol_20d':     0.10,
    'gap_pct':      0.001,
    'drawdown_from_50d_high': 0.01,
}
```

**Rust norm stats (current):** check `models/norm_stats_qqq_v1.json`. If the Rust normalizer does not apply MAD floors, live normalization can produce extreme values for live data in tail regimes. The notebook comment explicitly says these floors "match the long-run empirical MADs of each feature."

**Action:** inspect `engine/src/normalize.rs` and the persisted norm stats artifact. If floors are missing, add them. This is a small fix and probably should be part of the parity work regardless of the 1A decision.

---

## Part 2 — NVDA + NVDD Pipeline (Sequenced After Parity)

### 2A. Generalize notebook to accept `--symbol`

**Files:**
- `models/colab/QQQ_Equities_Model.ipynb` — refactor to read `SYMBOL` env var, default `QQQ`. Export paths become `models/{symbol}_tcn_v1.pt`, `models/{symbol}_lgbm_h{h}_v1.pkl`, `models/norm_stats_{symbol}_v1.json`, `models/model_meta_{symbol}_v1.json`.
- Add a `SYMBOL` parameter at the top of cell 4.

**Steps:**
1. Refactor `fetch_data(cache_file, symbols)` to take symbol list dynamically.
2. Refactor export paths to use `SYMBOL` lowercase.
3. Save refactored notebook as `models/colab/EQ_Equities_Model.ipynb` (the source-of-truth notebook).
4. Keep QQQ-specific copy as `QQQ_Equities_Model.ipynb` for archival/audit, or delete it after the refactor lands.

### 2B. Ingest NVDA + NVDD daily candles

**Files:**
- `engine/src/data/yahoo.rs` — already fetches any ticker; verify NVDA works (`yfinance` ticker `NVDA`).
- New: `engine/src/data/nvdd.rs` — NVDD (`NVDD`) via Yahoo, only from 2022-07-13 IPO onward. Short history is OK because NVDD uses NVDA's predictions (no separate model).
- `engine/src/db.rs` — `equity_candles` table is symbol-agnostic; no schema change needed.
- `engine/src/main.rs` — extend startup to seed NVDA + NVDD backfill.
- `engine/src/scheduler.rs` — refactor `EquityScheduler` to support multiple symbols. Simplest: `Vec<(String, EquityScheduler)>` keyed by symbol, or instantiate one scheduler per symbol in `main.rs`.

**Steps:**
1. Add NVDA + NVDD to a configurable tracked symbols list (env: `EQUITY_SYMBOLS=QQQ,NVDA`, with NVDD auto-paired for short leg).
2. Smoke test: `cargo run -- --ingest-once NVDA --days 9000` → confirm rows in `equity_candles`.
3. Smoke test: `cargo run -- --ingest-once NVDD` → confirm rows from 2022-07-13 onward.

### 2C. Train NVDA ensemble

**Run the refactored notebook with `SYMBOL=NVDA`** (or extract a Python CLI that replicates cell 14's main):

**Walk-forward config (notebook defaults):**
- Train: 5y (1260 days), Val: 1y (252 days), Step: 1y, Embargo: 136 days (SEQ_LEN=126 + HORIZONS=21 + 10)
- IC gate: 0.03 across all 3 horizons
- 15 folds across 25y history

**Steps:**
1. Generate `models/nvda_tcn_v1.pt`, `models/nvda_lgbm_h{1,5,21}_v1.pkl`, `models/norm_stats_nvda_v1.json`.
2. **Verify IC ≥ 0.03 across all 3 horizons** (notebook prints `=== WALK-FORWARD RESULTS ===` with per-horizon IC).
3. If IC fails the gate: examine per-fold ICs. Likely culprits:
   - Pre-2010 NVDA regime (small-cap, dot-com bust survivor, very different vol profile).
   - Constrain initial training to post-2010 (15y).
   - Consider dropping `tlt_corr_20d` if NVDA's bond correlation is too regime-dependent (this would require retraining — defer).

### 2D. NVDD short-leg architecture

NVDD is a -1x daily-reset inverse. Implications:
- **Daily path matches -NVDA** (before fees/funding).
- **Overnight drift** — NVDD's overnight return ≠ -NVDA's overnight due to daily reset mechanics.
- **Data depth** — NVDD IPO 2022-07-13. ~3y history. Not enough for walk-forward validation.

**Approach: do NOT train a separate NVDD model.** Use NVDA's pred_1d signal:
- NVDA model produces pred_1d, pred_5d, pred_21d for NVDA.
- Strategy layer reads NVDA predictions and decides `Long NVDA | Long NVDD | Flat`.
- Entry conditions:
  - **Long NVDA**: pred_1d > `entry_threshold` AND SMA regime = bull AND pred_5d_filter (or override)
  - **Long NVDD (short proxy)**: pred_1d < `short_entry_threshold` AND SMA regime = bear
  - **Flat**: neither triggered, or sentiment overlay says so
- Executor maps `Long NVDA` → `Buy NVDA` and `Long NVDD` → `Buy NVDD`.

**Files:**
- `engine/src/strategy.rs` — extend `EquityStrategyParams` with an inverse symbol map (default: `NVDA -> NVDD`, `QQQ -> PSQ`).
- `engine/src/exec/moomoo.rs` — confirm NVDD is tradeable. (US-listed ETF, yes.)
- `engine/src/scheduler.rs::finalize_candle` — when symbol is NVDA, also fetch/invert prediction for NVDD leg (or rely on executor to receive a synthetic NVDD "prediction" computed from NVDA signal).

### 2E. Paper mode deployment

- `deploy/.env` → `EQUITY_SYMBOLS=QQQ,NVDA`, `TRADING_MODE=paper`.
- Deploy: `docker compose up -d --build`.
- Confirm: both schedulers fire, both models load, both prediction rows land in `equity_predictions`, both columns visible in `/api/status`.

---

## Part 3 — Sentiment Risk Overlay (Option 3)

### 3A. Wire Finnhub to `fetch_sentiment`

**Files:**
- `engine/src/data/sentiment.rs` — replace stub at line 20.
- `engine/src/config.rs` — add `finnhub_api_key: Option<String>` to AppState.
- `engine/Cargo.toml` — confirm `reqwest` with `json` feature is present (likely yes; verify).
- `deploy/.env` — `FINNHUB_API_KEY=...`.

**Implementation:**
```rust
// Replace body of fetch_sentiment()
let api_key = match &state.finnhub_api_key {
    Some(k) => k,
    None => return Ok(0.5),  // graceful degradation: no key → neutral
};
let url = format!("https://finnhub.io/api/v1/news-sentiment?symbol={}", symbol);
let resp: FinnhubSentimentResp = reqwest::Client::new()
    .get(&url)
    .header("X-Finnhub-Token", api_key)
    .timeout(Duration::from_secs(5))
    .send().await?
    .json().await?;
let bullish = resp.sentiment.bullish_percent;
let bearish = resp.sentiment.bearish_percent;
let buzz    = resp.buzz.articles_in_last_week as f64;
let score   = (bullish - bearish).clamp(-1.0, 1.0);
let weight  = (buzz / 10.0).min(1.0);  // require 10+ articles/week to be influential
Ok(score * weight)
```

**Failure handling:** if API call fails (timeout, 429, network) → log warning, return `0.5` (neutral), do not panic. Cache write is still attempted so the table row exists.

### 3B. Scheduler hook

**Files:**
- `engine/src/scheduler.rs` — new `async fn refresh_sentiment_cache(&self)`.

**Steps:**
1. After successful `process()`, call `refresh_sentiment_cache(&self.symbol)`.
2. UPSERT into `sentiment_cache` with today's date, score, source=`finnhub`.
3. Skip if today's row already has `source = 'finnhub'` (idempotent within a day).

### 3C. Strategy-layer risk overlay

**Files:**
- `engine/src/strategy.rs` — new `apply_sentiment_overlay(signal: Position, sentiment: f64, article_count: i64) -> (Position, f64)`.
- `engine/src/strategy.rs` — extend `EquityStrategyParams`:
  - `enable_sentiment_overlay: bool` (default `true`)
  - `sentiment_reduce_threshold: f64` (default `-0.5`)
  - `sentiment_exit_threshold: f64` (default `-0.8`)
  - `sentiment_min_articles: i64` (default `5`)

**Overlay logic:**
```rust
pub fn apply_sentiment_overlay(
    signal: Position,
    score: f64,
    article_count: i64,
    params: &EquityStrategyParams,
) -> (Position, f64) {
    if !params.enable_sentiment_overlay || article_count < params.sentiment_min_articles {
        return (signal, 1.0);  // overlay off or insufficient data → no effect
    }
    // Rule 2 (hard exit takes precedence): extreme negative → flatten any position
    if score < params.sentiment_exit_threshold {
        return (Position::Flat, 0.0);
    }
    // Rule 1: moderate negative → halve size
    if score < params.sentiment_reduce_threshold {
        return (signal, 0.5);
    }
    (signal, 1.0)  // neutral or positive sentiment → no effect
}
```

Returns `(Position, size_multiplier)`. Executor multiplies the position's notional by the size multiplier before sizing shares.

**A/B testing:**
- Toggle via `/api/strategy-config` UI panel (`frontend/src/lib/components/StrategyConfig.svelte`).
- Run paper mode 2-4 weeks with overlay ON, capture trade log.
- Toggle OFF, run 2-4 weeks, compare:
  - Max drawdown
  - Win rate
  - Sharpe
  - Number of forced exits (count forced Flat transitions)
- Pick whichever version has better risk-adjusted return; document the decision.

**Files:**
- `frontend/src/lib/components/StrategyConfig.svelte` — add overlay toggle + threshold inputs.
- `engine/src/api/strategy.rs` — accept new params in PUT body, validate ranges.

### 3D. Backfill (deferred, conditional)

Only worth pursuing if Option 3 shows measurable value in the A/B test. Finnhub free tier doesn't include historical sentiment — would need:
- Paid vendor (RavenPack, Refinitiv).
- Or: scrape archived news + own sentiment model. Significant engineering.

If Option 3 doesn't show value after 2-4 weeks → delete the overlay, revert `fetch_sentiment` to stub, don't backfill. YAGNI.

---

## Part 4 — Conditional Next Steps (Documented for Future Reference)

### If NVDA validates (IC ≥ 0.03 across all 3 horizons):
- **Sector-relative features (XLK reference):**
  - `corr_vs_xlk_20d` — NVDA vs XLK 20d rolling correlation
  - `relative_strength_20d` — NVDA 20d return minus XLK 20d return
  - `sector_relative_volume` — NVDA rvol_20d divided by XLK rvol_20d
- **Decision needed:** target = NVDA absolute return (cleaner, requires re-tuning strategy thresholds) or NVDA relative outperformance (NVDA - XLK) (different signal type, requires retuning).
- **Walk-forward stability check:** if IC variance across folds > 2× mean, NVDA model is regime-fragile. Consider ensemble of NVDA + QQQ + 1-2 others (multi-asset pooled training).

### If sentiment overlay shows value:
- **Sentiment as TCN feature (Option 1):** requires backfilling 5-10y of sentiment. Evaluate vendor cost vs scrape-and-label. Re-train TCN with sentiment as 9th feature. Walk-forward IC gate still applies.
- **Sentiment-driven regime filter:** extend overlay to also force-flat when `score > +0.7 && article_count > 20` (euphoria → mean reversion risk). A/B test separately.
- **Negative-sentiment short signal:** when score < -0.8 and we're Flat, consider entering Short NVDD preemptively (sentiment-as-entry, not just risk overlay). Requires its own walk-forward validation.

### If NVDA fails IC gate:
- Try single-name ETF proxies (SOXL for semis, ARKK for innovation) — same 25y history, lower vol.
- Or pause multi-asset work and improve QQQ strategy first (better regime filters, smarter position sizing).

---

## Open Questions for Inah

Before kicking off execution, decisions on these four:

1. **Parity 1A (drawdown fix):** Retrain QQQ as part of the fix (clean, deploy-able artifact stays honest), or only fix going forward for NVDA and let the harness tolerate the divergence (fast, dirty)?
2. **Parity 1C (blend):** Reconcile inference to notebook's z-score blend (Option A — carry rolling prediction history buffer), or re-baseline the notebook to the flat 0.5/0.5 raw blend (Option B — delete the z-score standardization, accept smaller numbers as new ground truth)?
3. **NVDD drift:** Accept the daily-reset structural drift on overnight holds, or restrict NVDD to intraday-only (requires market-hours-aware exit logic, adds complexity)?
4. **Overlay thresholds:** Calibrate from a 2-week paper run first, or ship -0.5 / -0.8 / 5-article as-is and tune live based on observed forced-exit frequency?

---

## Files Likely to Change

**Parity (1A, 1B, 1C, 1D):**
- `engine/src/features/equities_v2.rs`
- `engine/src/normalize.rs` (MAD_FLOORS check)
- `inference/equity_model.py`
- `engine/tests/fixtures/equity_feature_parity.json` (regenerate)
- `models/qqq_tcn_v1.pt` (regenerated if 1A retrain chosen)
- `models/qqq_lgbm_h{1,5,21}_v1.pkl` (regenerated if 1A retrain chosen)
- `models/norm_stats_qqq_v1.json` (regenerated if 1A retrain chosen)

**NVDA pipeline (2A-2E):**
- `models/colab/EQ_Equities_Model.ipynb` (new — generalized)
- `engine/src/data/yahoo.rs` (verify NVDA)
- `engine/src/data/nvdd.rs` (new)
- `engine/src/main.rs`
- `engine/src/scheduler.rs`
- `engine/src/strategy.rs` (inverse symbol map)
- `engine/src/exec/moomoo.rs` (NVDD order routing)
- `models/nvda_tcn_v1.pt` (new artifact)
- `models/nvda_lgbm_h{1,5,21}_v1.pkl` (new artifacts)
- `models/norm_stats_nvda_v1.json` (new artifact)
- `deploy/.env` (`EQUITY_SYMBOLS=QQQ,NVDA`)

**Sentiment overlay (3A-3C):**
- `engine/src/data/sentiment.rs`
- `engine/src/config.rs` (add `finnhub_api_key`)
- `engine/src/scheduler.rs`
- `engine/src/strategy.rs` (overlay function + params)
- `engine/src/api/strategy.rs` (PUT body validation)
- `frontend/src/lib/components/StrategyConfig.svelte` (overlay toggle UI)
- `deploy/.env` (`FINNHUB_API_KEY`)

---

## Verification Plan

After each track lands:
1. **Parity:** `cargo test --test equity_feature_parity` → max diff < 1e-9 across all 8 features. Notebook cell-8 fixture must produce identical JSON.
2. **NVDA:** Run notebook with `SYMBOL=NVDA` → walk-forward IC ≥ 0.03 across all 3 horizons. Paper-trade for 1 week, confirm both prediction columns land in DB.
3. **Sentiment:** Manual `curl https://finnhub.io/api/v1/news-sentiment?symbol=NVDA` → confirm response shape. Scheduler run → confirm `sentiment_cache` row updates with `source='finnhub'`. Then paper-trade with overlay ON for 1 week, toggle OFF, repeat.

---

## Risks Summary

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| NVDA walk-forward IC < 0.03 | Medium | High (blocker) | Try XLK/SOXL as proxy |
| Drawdown fix forces QQQ retrain | High (real semantic change) | Medium | Schedule retrain window, deploy off-hours |
| Z-score blend needs prediction history buffer | High | Medium | Option B (re-baseline) is simpler |
| Finnhub rate limits (60/min) | Low | Low | 2 calls/day, well under limit |
| NVDD structural drift | High | Medium | Document in plan; consider intraday-only |
| Sentiment overlay thresholds uncalibrated | High | Low | A/B testable, reversible |
| Rust MAD_FLOORS not applied | Medium | Medium (rare live regime) | Inspect `normalize.rs`, add if missing |

---

**Plan complete. Awaiting answers to the 4 open questions before execution.**
