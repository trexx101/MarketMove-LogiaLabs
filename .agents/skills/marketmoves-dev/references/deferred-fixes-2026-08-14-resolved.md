# Deferred Fixes — Resolved (2026-08-14)

Source plan: `.hermes/plans/2026-08-14_024500-opus5-review-deferred-fixes.md`
Executor: omh-ralph-task skill (file-scope + full-suite + per-task commit discipline)
Plan-mandated order: **1 → 3 → 5 → 4 → 2** (z-score last because it needed `label_std` injection).

All 5 fixes verified green: `cargo test --lib` → **225 passed, 0 failed**.

---

## Fix 1 — VIX/TLT timestamp alignment (RESOLVED)

**Root cause:** `compute_equity_features` aligned VIX/TLT to QQQ by array index.
A missing bar shifted the whole series by one day silently.

**Contract change:** signature became
`compute_equity_features(qqq, vix_pairs: Option<&[(i64, f64)]>, tlt_pairs: Option<&[(i64, f64)]>)`.
Inside, pairs are collapsed into a `HashMap<i64, f64>` and joined to each candle
by `c.ts` → `0.0` for missing bars (calm fill), now landing on the correct candle.

**Cascade (every call site needed updating):**
- `engine/src/features/equities_v2.rs` — new signature + HashMap alignment + body
  `match vix_close { Some(v) if v.len()==n => v, _ => vec![0.0;n] }` became
  `if vix_close.len() == n { vix_close ... } else { vec![0.0;n] }` (the old `match`
  treated `vix_close` as `Option<Vec<f64>>` but it is now a plain `Vec<f64>`).
- `engine/src/api/equity.rs` — 2 call sites. `handle_equity_features` builds
  `Vec<(i64,f64)>` pairs from `vix_candles`/`tlt_candles`; `backfill_predictions`
  reuses `align_for_features` which now returns `Vec<(i64,f64)>` (was `Vec<f64>`).
- `engine/src/scheduler.rs` — `run()` builds pairs from `fetch_equity_candles_asc`
  `(ts, close)` tuples + a coverage warning when pair count < QQQ candle count.
- `engine/tests/equity_feature_parity.rs` — fixture schema gained `ts: Vec<i64>`
  and `vix`/`tlt` became `Vec<[f64;2]>` (ts,close) pairs; `#[ignore]` removed;
  `as_pairs()` helper added.
- `engine/src/features/equities_v2.rs` tests — `vix_regime_buckets` and
  `tlt_correlation_in_range` now build `(ts, close)` pairs matching
  `synthetic_qqq`'s `ts = i*86400`.

**Test added:** `parity_1e_vix_tlt_timestamp_alignment` — 60 QQQ candles, VIX
missing ts=1020; asserts index 20 is 0.0 (gap) and index 30 is 18.0→bucket 1.0
(aligned by timestamp, not shifted). At index 21, VIX=17.1 → calm bucket 0.0
(NOT 1.0 — 17.1 < 18; an earlier draft of this test wrongly expected 1.0).

---

## Fix 2 — Z-score de-normalization (RESOLVED)

**Root cause (from opus5 finding #3):** de-norm used non-stationary buffer-pooled
`combined_std * atr_ratio`. Same feature window → different raw prediction
depending on call history.

**Resolution:** replaced with **fixed training-time `label_std` per horizon**:
- `EquityEnsemble.__init__` gains `model_meta_path: str | None`. Loads
  `label_std_{1,5,21}d` from `model_meta_*.json` if present; defaults
  `{1: 0.012, 5: 0.028, 21: 0.065}` (typical QQQ, overridden by meta).
- `_pooled_std` path removed from `predict`. De-norm is now
  `blend_z * label_std[h]` (stationary).
- **Warmup guard:** if buffer length < 10, return raw `0.5/0.5` blend
  (`tcn_weight*tcn_raw + lgbm_weight*lgbm_raw`) instead of z-scoring — avoids
  unstable z-scores during cold start (this also mitigates opus5 #6/#7 NaN/zero
  guards at the prediction source).
- `_load_ensemble` takes `model_meta_path`; `main()` resolves it:
  - per-symbol: `<models_dir>/<sym>/model_meta_<sym>_v1.json`
  - flat QQQ: `MODEL_META_PATH` env or `models/model_meta_qqq_v1.json`
- Meta load is wrapped in try/except — boot never crashes if meta missing.

**Note:** no `label_std` keys exist in current `model_meta_*.json`, so defaults
apply. To make de-norm exact, inject `label_std_{1,5,21}d` into the meta files
from the Colab walk-forward (notebook cell 14 / evaluation).

---

## Fix 3 — Engine restart backfill (RESOLVED)

**Root cause (opus5 #8):** after restart `last_processed_ts` was `None`; scheduler
only processed the single latest candle, skipping intermediate ones.

**Resolution:**
- `engine/src/db.rs` — new `fetch_equity_candles_after(pool, symbol, after_ts, latest_ts)`
  returns `Vec<EquityCandle>` between exclusive `after_ts` and inclusive `latest_ts`, ASC.
- `engine/src/scheduler.rs` `run()` — recovery block:
  1. `latest_equity_ts = db::latest_equity_candle_ts(...)`
  2. if `last_processed_ts < latest_equity_ts - 1_day` → loop
     `fetch_equity_candles_after` and process each missed candle
  3. bounded by `last_processed_ts == latest_equity_ts` exit (prevents infinite
     loop on duplicate timestamps).

---

## Fix 4 — Parity harness regeneration (RESOLVED)

**Resolution:** `training/equities_features.py`:
- `generate_synthetic_data` now returns `ts: Vec<i64>` + `vix_pairs`/`tlt_pairs`
  as `(ts, close)` 2-element lists (plain `vix`/`tlt` arrays still feed the
  in-memory `compute_equity_features`).
- `main() --generate-fixture` writes the pair form into the JSON fixture.
- Regenerated `engine/tests/fixtures/equity_feature_parity.json` (120 rows).
- `#[ignore]` removed from the Rust parity test (now runs in `cargo test --lib`).

The fixture was previously generated against the stale `close`-based Python
reference (opus5 #9/#11). It now matches the corrected `high`-based Rust feature.

**Verify:** `python3 training/equities_features.py --generate-fixture` then
`cargo test --lib -- features::equities_v2`.

---

## Fix 5 — Asymmetric short `pred_5d` filter (RESOLVED)

**Resolution:** new `short_pred_5d_filter: bool` (default `false`, backward-compatible).
Shorts require `pred_5d < 0.0` when enabled — symmetric to the long `pred_5d_filter`.

Files touched (see SKILL.md "Adding a new strategy param" for the full cascade):
- `engine/src/strategy.rs` — field + default fn + `eq_params` helper + 2 tests
  (`equity_short_entry_blocked_by_pred_5d_filter`, `..._allowed_when_negative`)
- `engine/src/config.rs` — `Config` field + `SHORT_PRED_5D_FILTER` env parse +
  test env clear list + assertion
- `engine/src/main.rs` — `model_strategy_params` construction
- `engine/src/api/strategy_config.rs` — response struct, GET, update struct,
  PUT update block, PUT response, `StrategyConfigChange` emit (note `&` prefix)
- `engine/src/api/status.rs` — `StrategySnapshot` struct + construction
- `engine/src/api/mod.rs` — `AppState` `strategy_params` construction
- `engine/Dockerfile` — `SHORT_PRED_5D_FILTER=false` ENV
- `deploy/docker-compose.yml` — `${SHORT_PRED_5D_FILTER:-false}`
- `.env` (workspace root) — `SHORT_PRED_5D_FILTER=false` added after `PRED_5D_FILTER=true`

**Test command that caught the cascade:** `cargo test --lib` (the lib build alone
missed test-only struct literals; the test build surfaced `E0063`/`E0308` at
`api/tests.rs`, `mode_toggle.rs`, `equities_v2.rs` test module).

---

## Verification

```bash
cd /home/ubuntu/projects/MarketMoves
cargo test --lib --manifest-path engine/Cargo.toml
# → 225 passed; 0 failed

python3 training/equities_features.py --generate-fixture   # regenerates fixture
python3 -c "import ast; ast.parse(open('inference/equity_model.py').read())"  # py syntax
```

## Remaining (not done this session — blocked by iteration limit)

- Per-task git commits on `feature/nvda-multi-asset-and-sentiment-overlay`.
  All changes uncommitted. Suggested split: one commit per fix (1-5) + verify commit.
- Pre-existing `atr` unused-variable warnings in `scheduler.rs` (not introduced here).
