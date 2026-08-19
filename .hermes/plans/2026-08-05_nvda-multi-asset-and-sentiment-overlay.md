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

| **Decision (locked 2026-08-05):** Fix going forward only. Deployed QQQ model keeps running on the `close`-based feature; harness tolerates the divergence. QQQ retrain deferred until it shows measurable degradation or until we re-baseline the notebook anyway.

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

| **Decision (locked 2026-08-05):** Option A — reconcile inference to notebook's z-score blend. Inference will carry a rolling prediction history buffer (default 60 predictions, per-horizon) and replicate the notebook's per-fold within-batch z-score standardization. This means notebook IC numbers are the ground truth — no re-baseline needed.

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

**Overlay logic (locked 2026-08-05, implementation deviation noted 2026-08-05):**
```rust
pub fn apply_sentiment_overlay(
    signal: Position,
    score: f64,
    article_count: i64,
    params: &EquityStrategyParams,
) -> Position {
    if !params.enable_sentiment_overlay || article_count < params.sentiment_min_articles {
        return signal;  // overlay off or insufficient data → no effect
    }
    // Rule 2 (hard exit takes precedence): extreme negative → flatten any position
    if score < params.sentiment_exit_threshold {
        return Position::Flat;
    }
    // Rule 1: moderate negative → block new entries only (exits still fire normally)
    if score < params.sentiment_reduce_threshold {
        return match signal {
            Position::Flat => Position::Flat,
            // Holding a position: keep it (exits still fire via next_equity_position's exit_threshold), but...
            // ...block new entries by forcing Flat if currently Flat going to Long/Short.
            // Implementation: the overlay returns 'signal' AS-IS for existing positions;
            // next_equity_position's entry rules are gated separately by the score.
            _ => signal,
        };
    }
    signal  // neutral or positive sentiment → no effect
}
```

**Deviation note (2026-08-05):** The plan §3C originally proposed a `f64` size_multiplier
to halve qty on -0.5 score. Inah approved a cleaner interpretation that avoids executor
surgery: score < -0.8 forces Flat; -0.8 ≤ score < -0.5 blocks new entries only (see
`next_equity_position` integration where the entry thresholds are suppressed). Halving
qty mid-hold is rejected as invasive.

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

**Status: ALL LOCKED 2026-08-05.**

Before kicking off execution, decisions on these four:

1. **Parity 1A (drawdown fix):** Retrain QQQ as part of the fix (clean, deploy-able artifact stays honest), or only fix going forward for NVDA and let the harness tolerate the divergence (fast, dirty)?
   **→ DECIDED: Fix going forward only.** Deployed QQQ model keeps running on the `close`-based feature; harness tolerates the divergence. QQQ retrain deferred.
2. **Parity 1C (blend):** Reconcile inference to notebook's z-score blend (Option A — carry rolling prediction history buffer), or re-baseline the notebook to the flat 0.5/0.5 raw blend (Option B — delete the z-score standardization, accept smaller numbers as new ground truth)?
   **→ DECIDED: Option A — reconcile inference.** Rolling prediction history buffer in inference (default 60, per-horizon). Notebook IC numbers stay ground truth.
3. **NVDD drift:** Accept the daily-reset structural drift on overnight holds, or restrict NVDD to intraday-only (requires market-hours-aware exit logic, adds complexity)?
   **→ DECIDED: Accept NVDD daily-reset drift as documented behavior.** Drift is bounded (∝ |NVDA overnight gap| × funding). No intraday-only restriction.
4. **Overlay thresholds:** Calibrate from a 2-week paper run first, or ship -0.5 / -0.8 / 5-article as-is and tune live based on observed forced-exit frequency?
   **→ DECIDED: Ship as-is.** A/B testable + reversible via `StrategyConfig` UI. Live forced-exit frequency is the calibration signal.

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

**Plan originally locked 2026-08-05. Amended 2026-08-05 to add multi-model registry architecture (see §8 below).**

---

## §8. Architectural Pivot — Multi-Model Registry (amended 2026-08-05)

After the 4 decisions in §0–§7 were locked, an architectural review surfaced
that **the unit of meaning is the model, not the symbol.** The trading engine
must therefore be keyed by *model* (primary + inverse + budget + thresholds)
rather than by a single hardcoded `Config::symbol`.

### §8.1 Locked UI decisions

1. **Active model via dropdown** in the dashboard header. Single selection
   drives `StatusPanel`, `CandlestickChart`, `FeatureInspector`, `ModelHealth`,
   `PnLEquityCurve`, and `TradeHistory`. Existing 12-col grid layout stays.
2. **The Events tab is the cross-model surface.** No new "alert strip"
   component — the existing `Events.svelte` view becomes model-agnostic and
   shows every event from every model with `model_id` and `pair` chips.
   Click an event row → switch active dropdown to that model.
3. **Per-model budget** on `StatusPanel`: `$5,000 budget / $2,400 used
   (3 open positions)`. Derived from `budget_usd` minus sum of
   (entry × qty) for that model's open positions.
4. **Per-model strategy params** (entry/exit/short thresholds) move from
   `Config` to a row in the new `trading_models` registry. `PUT
   /api/strategy-config` takes a `model_id` parameter.

### §8.2 New `trading_models` table

```sql
CREATE TABLE IF NOT EXISTS trading_models (
  model_id        TEXT PRIMARY KEY,        -- uuid, NOT the ticker
  primary_symbol  TEXT NOT NULL,           -- e.g. 'NVDA'
  inverse_symbol  TEXT NOT NULL,           -- e.g. 'NVDD'
  model_path      TEXT NOT NULL,           -- path to .pt file
  norm_stats_path TEXT NOT NULL,           -- path to norm_stats json
  budget_usd      REAL NOT NULL DEFAULT 5000.0,
  enabled         INTEGER NOT NULL DEFAULT 1,
  deployed_at     INTEGER NOT NULL,         -- ts of registry insert
  last_wf_ic      REAL,                     -- last walk-forward mean IC
  last_wf_at      INTEGER,                  -- last walk-forward date
  notes           TEXT
);
```

Migration follows the existing `migrate_sentiment_cache` pattern. No `drizzle
push` equivalent — ordered migrations via `PRAGMA user_version`.

### §8.3 Engine bootstrap behavior

At startup, `main.rs` queries `SELECT * FROM trading_models WHERE enabled = 1`
and spawns one `EquityScheduler` + one `PaperExecutor::new_for_symbol` per
row. If the registry is empty, fall back to `Config::symbol` / `Config::
short_symbol` so a fresh DB still produces paper trades on the default
symbol (preserves Wave A behavior).

### §8.4 Telemetry event enrichment

Every `TelemetryEvent` variant gains two new fields:

- `model_id: String` — the row's `model_id`
- `pair: String` — display label, e.g. `"NVDA/NVDD"`

`TradeFill` already has `symbol`; `model_id` is additive.

### §8.5 Events persistence migration

```sql
ALTER TABLE engine_events ADD COLUMN model_id TEXT;
ALTER TABLE engine_events ADD COLUMN pair TEXT;
```

Existing rows get `model_id = NULL` / `pair = NULL`. `Events.svelte` renders
those as `(legacy)` so historical events stay visible.

### §8.6 New API endpoints

| Method | Path | Body | Returns |
|---|---|---|---|
| `GET` | `/api/models` | — | `Vec<TradingModel>` |
| `POST` | `/api/models` | `{primary_symbol, inverse_symbol, model_path, norm_stats_path, budget_usd?, notes?}` | `TradingModel` (with new `model_id`) |
| `PUT` | `/api/models/{id}/enabled` | `{enabled: bool}` | `TradingModel` |

Validation on `POST`: `model_path` and `norm_stats_path` must exist on disk;
refuse otherwise.

### §8.7 Modified API endpoint

`PUT /api/strategy-config` body shape changes from flat params to:

```json
{
  "model_id": "<uuid>",
  "entry_threshold": 0.003,
  "exit_threshold": -0.001,
  "sma_window": 200,
  "pred_5d_filter": true,
  "enable_shorting": false,
  "short_entry_threshold": -0.004,
  "short_exit_threshold": 0.001
}
```

Backend storage becomes `Arc<RwLock<HashMap<String, EquityStrategyParams>>>`
keyed by `model_id`. Each scheduler reads its own slice at cycle start.

### §8.8 Frontend store refactor

`stores.js` becomes:

- `models` — `Writable<TradingModel[]>` (registry)
- `activeModelId` — `Writable<string>` (drives the dropdown)
- `statusByModel` — `Writable<Map<string, StatusSlice>>`
- `predictionsByModel` — `Writable<Map<string, PredictionsSlice>>`
- `featuresByModel` — `Writable<Map<string, FeaturesSlice>>`
- `tradesByModel` — `Writable<Map<string, TradeSlice[]>>`
- `chartDataByModel` — `Writable<Map<string, ChartSlice>>`
- Legacy `status` / `predictions` / `features` / `trades` / `chartData` —
  derived stores that mirror the slice for the active model, so existing
  components (`StatusPanel`, etc.) read from them unchanged.

`websocket.js` handlers route each event by `msg.model_id` into the per-model
map. The legacy stores are updated reactively from the active model's slice.

### §8.9 PnL aggregation rule

- Active model's `StatusPanel` shows per-model Realized + Unrealized PnL.
- Bottom row of `StatusPanel`: portfolio totals = sum across all enabled
  models of `budget_usd` and `realized_pnl`, plus count of open positions.
- `PnLEquityCurve` renders one line per enabled model (semi-transparent) plus
  the portfolio-total line on top (bold).

### §8.10 Sentiment overlay

- Defaults: `enable_sentiment_overlay = false`, `sentiment_min_articles = 15`.
- Per-model application in the strategy layer — one model's bad sentiment
  doesn't affect another's positions.
- New "Sentiment" row on `StatusPanel`: score + article count when overlay
  enabled and minimum article threshold met; `—` otherwise.

### §8.11 Implementation order (15 steps)

| # | File(s) | Approx LOC | Notes |
|---|---|---|---|
| 1 | `engine/src/db.rs` (`trading_models` migration + loader/CRUD) | 150 | Migration + `load_enabled_models`, `register_model`, `update_model_enabled` |
| 2 | `engine/src/main.rs` (bootstrap loop) | 80 | Spawn one scheduler + executor per enabled row |
| 3 | `engine/src/api/ws.rs` (`TelemetryEvent` enrichment) | 60 | Add `model_id`, `pair` fields |
| 4 | `engine/src/db.rs` (`engine_events` migration) | 40 | Add `model_id`, `pair` columns |
| 5 | `engine/src/api/models.rs` (new file) | 120 | GET/POST/PUT endpoints |
| 6 | `engine/src/api/strategy_config.rs` + `AppState` | 50 | `HashMap<String, EquityStrategyParams>` keyed by model_id |
| 7 | `frontend/src/lib/stores.js` | 80 | `models`, `activeModelId`, per-model maps, derived legacy stores |
| 8 | `frontend/src/lib/websocket.js` | 40 | Route every event by `msg.model_id` |
| 9 | `frontend/src/views/Dashboard.svelte` | 30 | Header dropdown |
| 10 | `frontend/src/lib/components/StatusPanel.svelte` | 40 | Budget row + Portfolio totals row |
| 11 | `frontend/src/views/Events.svelte` | 25 | `model_id`/`pair` chips, click-to-switch |
| 12 | `frontend/src/lib/components/PnLEquityCurve.svelte` | 40 | Multi-line per model + portfolio total |
| 13 | `frontend/src/lib/components/StrategyConfigPanel.svelte` | 25 | Model selector in PUT body |
| 14 | `frontend/src/lib/api.js` | 40 | New API functions for `/api/models` + modified `/api/strategy-config` |
| 15 | Build + test cycle | — | `cargo build`, `cargo test --lib`, vite build |

Sentiment overlay logic (Option 3, the 4-strategy fields) is intentionally
deferred to a follow-up commit after the registry lands. The plan stays
sequential: registry → sentiment overlay → paper deploy.

### §8.12 Commit staging

Two commits, on `feature/nvda-multi-asset-and-sentiment-overlay`:

1. **Backend batch** (steps 1–6): DB migration, bootstrap loader, telemetry
   enrichment, API endpoints, AppState update. Includes test updates per
   the standing AppState construction rule.
2. **Frontend batch** (steps 7–14): store refactor, websocket routing,
   Dashboard dropdown, StatusPanel Budget row, Events chips, PnL curve
   multi-line, StrategyConfigPanel selector, api.js additions.

Per project convention, neither commit is pushed to origin without explicit
direction.

### §8.13 Bootstrap / cold-start behavior

When `trading_models` is empty at startup:

- If `Config::symbol` is set (default `"QQQ"`, with `Config::short_symbol =
  "PSQ"`), the engine auto-registers a row named `qqq-bootstrap` with
  `budget_usd = 5000` and `model_path`/`norm_stats_path` derived from
  `Config::norm_stats_path`. This preserves Wave A behavior on a fresh DB.
- If `Config::symbol` is empty, the engine starts in a "no models" mode
  where the engine serves the dashboard with all-zero metrics and no
  schedulers run. The user must POST a model via `/api/models` to start
  trading.

### §8.14 Verification

After backend batch lands:

- `cargo build` clean (no new warnings beyond the pre-existing 23
  `config::tests` env-var-collision baseline).
- `cargo test --lib` — same baseline as before the registry work (no new
  failures introduced).
- New tests: `db::tests::register_and_load_model`,
  `api::models::tests::register_validates_paths_exist`.
- Manual: `curl -X POST http://localhost:9080/api/models -d '{"primary_
  symbol":"NVDA", ...}'` returns the registered row with a generated uuid.

After frontend batch lands:

- `vite build` clean.
- Manual: dashboard header dropdown shows both `qqq-bootstrap` and the
  manually-registered NVDA model. Selecting NVDA → StatusPanel updates
  with NVDA's slice. Events tab shows entries for both models with the
  `pair` chip.

### §8.15 Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Bootstrap row collides with manually-registered model | Low | Medium | Use deterministic uuid prefix `bootstrap-<symbol>` so re-running with same `Config::symbol` is idempotent |
| `Map<model_id, slice>` partition miss (event arrives before registry loads) | Medium | Medium | WS handlers hold a "pending events" buffer keyed by `model_id` and flush on registry load |
| Per-model `HashMap<RwLock>` write contention | Low | Low | Each scheduler holds its own entry; reads are via the same `Arc<RwLock<HashMap>>` but contend only on PUT |
| StatusPanel Portfolio total math off-by-one | Medium | Low | Computed once on the active-model change + on each `TradeFill` event; tested in the dashboard smoke |

---

**Plan amended. Awaiting green-light to start step 1 (db.rs migration + trading_models CRUD).**
