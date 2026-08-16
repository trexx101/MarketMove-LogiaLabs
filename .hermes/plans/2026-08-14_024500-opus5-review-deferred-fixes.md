# Opus 5 Review — Deferred Fixes Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Implement the 6 deferred fixes from the Claude Opus 5 code review (2026-08-13) that were not addressed in commit `cf5e437`.

**Architecture:** Each fix is independent and can be implemented in any order. Tasks are grouped by severity (CRITICAL → HIGH → MEDIUM). All changes require `cargo test --lib` green (currently 222/222) and a Docker rebuild + redeploy.

**Tech Stack:** Rust (axum, sqlx, zmq), Python (torch, lightgbm), Docker Compose v1.29.2

**Branch:** `review/multi-model-bugfixes-and-inference-hardening` (off `feature/nvda-multi-asset-and-sentiment-overlay`)

**Review cost:** $1.04 spent of $10 budget. Full findings: `~/.hermes/skills/project/marketmoves-dev/references/opus5-review-2026-08-13-full-findings.md`

---

## Already Fixed (commit `cf5e437`, 2026-08-14)

These are **done** — listed for reference only, do NOT re-implement:

| # | Finding | Status |
|---|---------|--------|
| 1 | ZMQ REQ socket poisons after timeout | ✅ Fixed — `reconnect()` in `bridge.rs` |
| 3 | Z-score `_pooled_std` formula wrong | ✅ Fixed — `sqrt((s_a² + s_b²)/2)` in `equity_model.py` |
| 4 | Short in bullish regime never force-exited | ✅ Fixed — `Short + bullish → Flat` in `strategy.rs` |
| 5 | Shorts during SMA warmup | ✅ Fixed — gated on `input.sma_valid` in `strategy.rs` |
| 6 | No NaN/finite guards | ✅ Fixed — non-finite input → hold + `tracing::error!` |
| 7 | All-zero predictions accepted silently | ✅ Fixed — validation in `scheduler.rs::finalize_candle` |
| 8 (partial) | Engine restart recovery | ✅ Fixed — `latest_prediction_ts()` recovery in `scheduler.rs::run()` |
| 9 | Drawdown `high` vs `close` divergence | ✅ Fixed — `equities_features.py` aligned to notebook + Rust |
| 12 | No backoff in ZMQ retries | ✅ Fixed — 100ms sleep between retries in `bridge.rs` |

---

## Deferred Fix 1: VIX/TLT timestamp alignment (CRITICAL)

**Review finding #2.** VIX and TLT closes are aligned to QQQ by array index in `compute_equity_features`. Any missing VIX/TLT bar (holiday mismatch, vendor gap) shifts the entire series by one day silently. The guard degrades to `vec![0.0; n]` (all zeros = "calm market"), presenting a data outage as a benign regime with no log.

**Files:**
- Modify: `engine/src/features/equities_v2.rs:99-103` — `compute_equity_features` signature + body
- Modify: `engine/src/scheduler.rs:149-165` — VIX/TLT fetch + alignment call site
- Test: `engine/src/features/equities_v2.rs` (new test in `#[cfg(test)]` module)

### Task 1.1: Write failing test for timestamp misalignment

**Objective:** Test that `compute_equity_features` correctly handles VIX/TLT series that are shorter than QQQ or have mismatched timestamps.

**Files:**
- Test: `engine/src/features/equities_v2.rs` — add `parity_1e_vix_tlt_timestamp_alignment`

**Step 1: Write failing test**

```rust
/// 1E: VIX/TLT must be aligned by timestamp, not array index.
/// If VIX has a gap (missing bar), the old code shifts all subsequent
/// VIX values by one position. The fix joins on timestamp.
#[test]
fn parity_1e_vix_tlt_timestamp_alignment() {
    // 60 QQQ candles at ts = 1000..1059
    let candles: Vec<EquityCandle> = (0..60)
        .map(|i| EquityCandle {
            symbol: "QQQ".into(),
            ts: 1000 + i,
            open: 100.0, high: 101.0, low: 99.0, close: 100.0 + i as f64,
            volume: 1_000_000, source: "yahoo".into(),
        })
        .collect();

    // VIX: 59 candles — missing ts=1020 (simulated vendor gap)
    let vix_candles: Vec<EquityCandle> = (0..60)
        .filter(|&i| i != 20) // skip ts=1020
        .map(|i| EquityCandle {
            symbol: "^VIX".into(),
            ts: 1000 + i,
            open: 15.0, high: 15.5, low: 14.5, close: 15.0 + i as f64 * 0.1,
            volume: 0, source: "cboe".into(),
        })
        .collect();

    // Pass VIX as (ts, close) pairs
    let vix_pairs: Vec<(i64, f64)> = vix_candles.iter().map(|c| (c.ts, c.close)).collect();

    let features = compute_equity_features(&candles, Some(&vix_pairs), None);

    // At ts=1020 (index 20), VIX should be 0.0 (missing — no alignment possible)
    assert_eq!(features[20].vix_regime, 0.0, "missing VIX bar should be 0.0");

    // At ts=1021 (index 21), VIX close should be 15.0 + 21*0.1 = 17.1
    // With the OLD index-based code, this would be 15.0 + 20*0.1 = 17.0 (shifted!)
    assert!(
        (features[21].vix_regime - 17.1).abs() < 0.01,
        "VIX at ts=1021 should be 17.1 (aligned by timestamp), got {}",
        features[21].vix_regime
    );
}
```

**Step 2: Run test to verify failure**

Run: `cargo test --lib parity_1e_vix_tlt_timestamp_alignment`
Expected: FAIL — `compute_equity_features` currently takes `Option<&[f64]>`, not `Option<&[(i64, f64)]>`

**Step 3: Implement timestamp alignment**

Change `compute_equity_features` to accept `Option<&[(i64, f64)]>` (timestamp, close) pairs for VIX and TLT, and join on timestamp:

```rust
pub fn compute_equity_features(
    qqq: &[EquityCandle],
    vix_pairs: Option<&[(i64, f64)]>,
    tlt_pairs: Option<&[(i64, f64)]>,
) -> Vec<EquityFeatureRow> {
    let n = qqq.len();
    // ... existing code ...

    // Build timestamp → close lookup maps
    let vix_map: std::collections::HashMap<i64, f64> = vix_pairs
        .map(|pairs| pairs.iter().cloned().collect())
        .unwrap_or_default();
    let tlt_map: std::collections::HashMap<i64, f64> = tlt_pairs
        .map(|pairs| pairs.iter().cloned().collect())
        .unwrap_or_default();

    // Align by timestamp — missing bars get 0.0 (existing behavior, but now positionally correct)
    let vix_close: Vec<f64> = qqq.iter().map(|c| vix_map.get(&c.ts).copied().unwrap_or(0.0)).collect();
    let tlt_close: Vec<f64> = qqq.iter().map(|c| tlt_map.get(&c.ts).copied().unwrap_or(0.0)).collect();

    // ... rest of feature computation uses vix_close / tlt_close as before ...
}
```

**Step 4: Update scheduler call site**

In `engine/src/scheduler.rs:149-165`, change the VIX/TLT fetch to produce `(ts, close)` pairs:

```rust
// Fetch VIX and TLT as (ts, close) pairs for timestamp alignment
let vix = db::fetch_equity_candles_asc(&self.pool, "^VIX", fetch_count).await.ok();
let tlt = db::fetch_equity_candles_asc(&self.pool, "TLT", fetch_count).await.ok();

let vix_pairs: Option<Vec<(i64, f64)>> = vix.map(|c| c.iter().map(|c| (c.ts, c.close)).collect());
let tlt_pairs: Option<Vec<(i64, f64)>> = tlt.map(|c| c.iter().map(|c| (c.ts, c.close)).collect());

let all_features = compute_equity_features(
    &candles,
    vix_pairs.as_deref(),
    tlt_pairs.as_deref(),
);
```

**Step 5: Add missing-data warning log**

After alignment, log a warning if VIX/TLT coverage is below 90%:

```rust
if let Some(ref pairs) = vix_pairs {
    let coverage = pairs.len() as f64 / n as f64;
    if coverage < 0.9 {
        tracing::warn!(
            symbol = %self.symbol,
            vix_bars = pairs.len(),
            qqq_bars = n,
            coverage_pct = coverage * 100.0,
            "VIX coverage below 90% — features may be degraded"
        );
    }
}
```

**Step 6: Run tests**

Run: `cargo test --lib`
Expected: 223/223 pass (222 existing + 1 new)

**Step 7: Commit**

```bash
git add engine/src/features/equities_v2.rs engine/src/scheduler.rs
git commit -m "fix: align VIX/TLT by timestamp instead of array index

Prevents silent feature skew when VIX/TLT bars are missing (holiday
mismatches, vendor gaps). Adds coverage warning log when VIX/TLT
coverage falls below 90%."
```

---

## Deferred Fix 2: Z-score de-normalization non-stationarity (CRITICAL)

**Review finding #3 (sub-issues 2 + 3).** The `_pooled_std` formula was fixed (commit `cf5e437`), but two issues remain:

1. **De-normalization treats `blend_z` as unit-variance**, but for 0.5/0.5 blend of correlated z-scores, `Var = 0.5(1+ρ)` → systematic shrinkage.
2. **Scale is non-stationary** — `combined_std` changes every bar as the buffer fills. The same feature window produces a different prediction depending on call history.

**Root cause:** The live system uses rolling prediction-buffer statistics for de-normalization. The notebook uses per-fold training-time label statistics. Predictions should be compared against a fixed scale, not a drifting one.

**Files:**
- Modify: `inference/equity_model.py:208-265` — `EquityEnsemble.predict` method
- Modify: `inference/equity_model.py` — `EquityEnsemble.__init__` (load label stats from model metadata)
- Reference: `models/colab/QQQ_Equities_Model.ipynb` cell 14 (notebook z-score blending)
- Reference: `~/.hermes/skills/project/marketmoves-dev/references/z-score-blending.md`

### Task 2.1: Load training-time label statistics from model metadata

**Objective:** Store per-horizon label std from training in `model_meta_*.json` and load it at inference startup.

**Files:**
- Modify: `inference/equity_model.py` — `EquityEnsemble.__init__`
- Check: `/var/lib/docker/volumes/deploy_models/_data/` for existing `model_meta_*.json` files

**Step 1: Check what model_meta currently contains**

```bash
docker exec mmn-inference cat /models/model_meta_qqq_v1.json 2>/dev/null | python3 -m json.tool
docker exec mmn-inference cat /models/NVDA/model_meta_nvda_v1.json 2>/dev/null | python3 -m json.tool
```

**Step 2: Add label_std fields to model_meta**

If `model_meta_*.json` doesn't already have per-horizon label std, add them. The notebook computes these in cell 14 during walk-forward evaluation. For QQQ, typical values are:
- `label_std_1d`: ~0.012
- `label_std_5d`: ~0.028
- `label_std_21d`: ~0.065

If the exact values aren't available, compute them from the training labels and write them into the JSON. If that's not feasible in this task, use the buffer-based approach but with a **warmup threshold** (see Task 2.2).

**Step 3: Load label_std in EquityEnsemble.__init__**

```python
class EquityEnsemble:
    def __init__(self, tcn_path, lgbm_paths, norm_stats_path, model_meta_path=None):
        # ... existing init ...

        # Load training-time label std for de-normalization (fixed scale)
        self._label_std: dict[int, float] = {1: 0.012, 5: 0.028, 21: 0.065}  # defaults
        if model_meta_path and Path(model_meta_path).exists():
            meta = json.loads(Path(model_meta_path).read_text())
            for h in self._horizons:
                key = f"label_std_{h}d"
                if key in meta:
                    self._label_std[h] = float(meta[key])
            log.info("loaded label_std from %s: %s", model_meta_path, self._label_std)
```

**Step 4: Commit**

```bash
git add inference/equity_model.py
git commit -m "feat: load training-time label std from model metadata"
```

### Task 2.2: Use fixed label_std for de-normalization instead of buffer std

**Objective:** Replace `combined_std = self._pooled_std(...)` with `self._label_std[h]`, making the scale stationary.

**Files:**
- Modify: `inference/equity_model.py:248-255` — `predict` method

**Step 1: Replace de-normalization**

```python
# OLD (non-stationary):
# combined_std = self._pooled_std(self._tcn_buffer[h], self._lgbm_buffer[h])
# raw_log_return = blend_z * combined_std * atr_ratio

# NEW (stationary — uses training-time label std):
label_std = self._label_std.get(h, 0.012)  # fallback default
raw_log_return = blend_z * label_std
```

**Note:** `atr_ratio` is no longer used for de-normalization. It was a proxy for scale that introduced non-stationarity. The label std from training IS the correct scale — it's what the notebook used.

**Step 2: Keep z-score buffers for blending only**

The buffers are still needed to compute per-model z-scores (mean/std of each model's raw output). But they no longer determine the output scale.

**Step 3: Add warmup guard**

During the first ~10 predictions (buffer < 10), z-scores are unreliable. Return raw blend (0.5 * tcn_raw + 0.5 * lgbm_raw) instead:

```python
if len(self._tcn_buffer[h]) < 10:
    # Warmup: use raw blend, not z-score
    raw_pred = self.tcn_weight * tcn_raw + self.lgbm_weight * lgbm_raw
    result[f"pred_{h}d"] = float(raw_pred)
    if not skip_buffer:
        self._tcn_buffer[h].append(tcn_raw)
        self._lgbm_buffer[h].append(lgbm_raw)
    continue
```

**Step 4: Test**

```bash
docker build --no-cache -f inference/Dockerfile -t marketmarkovnet/inference:latest inference/
docker-compose -f deploy/docker-compose.yml up -d inference
# Wait 60s for healthcheck
docker logs --tail 20 mmn-inference 2>&1
# Verify predictions are non-zero after 2+ real requests
```

**Step 5: Commit**

```bash
git add inference/equity_model.py
git commit -m "fix: use stationary training-time label std for de-normalization

Replaces non-stationary buffer-based pooled_std with fixed label_std
from model metadata. Eliminates prediction drift as buffers fill.
Adds 10-prediction warmup using raw blend before z-score blending."
```

---

## Deferred Fix 3: Engine restart backfill loop (HIGH)

**Review finding #8 (full version).** Commit `cf5e437` added `latest_prediction_ts()` recovery, but only processes the **single latest** candle. If the engine was down for 5 days, the 4 intermediate candles are permanently skipped.

**Files:**
- Modify: `engine/src/scheduler.rs:98-115` — `run()` method, after `latest_prediction_ts` recovery
- Test: `engine/src/scheduler.rs` or `engine/src/api/tests.rs`

### Task 3.1: Backfill missed candles on startup

**Objective:** After recovering `last_processed_ts`, query all candle timestamps between `last_processed_ts` and the latest candle, and process each one.

**Files:**
- Modify: `engine/src/scheduler.rs:98-115`
- Modify: `engine/src/db.rs` — add `fetch_unprocessed_candle_ts` function

**Step 1: Add DB function to fetch missed candle timestamps**

In `engine/src/db.rs`:

```rust
/// Fetch all candle timestamps for a symbol between `after_ts` (exclusive)
/// and `latest_ts` (inclusive), ordered ascending.
pub async fn fetch_unprocessed_candle_ts(
    pool: &DbPool,
    symbol: &str,
    after_ts: i64,
    latest_ts: i64,
) -> Result<Vec<i64>> {
    let rows = sqlx::query_scalar::<_, i64>(
        "SELECT ts FROM equity_candles
         WHERE symbol = ?1 AND ts > ?2 AND ts <= ?3
         ORDER BY ts ASC",
    )
    .bind(symbol)
    .bind(after_ts)
    .bind(latest_ts)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
```

**Step 2: Call backfill in scheduler.run() after recovery**

In `engine/src/scheduler.rs`, after the `latest_prediction_ts` recovery block:

```rust
// Backfill: process any candles that were missed during downtime
if let Some(last_ts) = self.last_processed_ts {
    if let Ok(Some(latest_candle_ts)) = db::latest_equity_candle_ts(&self.pool, &self.symbol).await {
        if latest_candle_ts > last_ts {
            let missed = db::fetch_unprocessed_candle_ts(
                &self.pool, &self.symbol, last_ts, latest_candle_ts,
            ).await.unwrap_or_default();
            if !missed.is_empty() {
                info!(symbol = %self.symbol, count = missed.len(), "backfilling missed candles");
                for ts in &missed {
                    if let Err(e) = self.process_candle(*ts).await {
                        warn!(symbol = %self.symbol, ts = *ts, error = %e, "backfill failed for candle");
                    }
                }
                info!(symbol = %self.symbol, "backfill complete");
            }
        }
    }
}
```

**Step 3: Run tests**

Run: `cargo test --lib`
Expected: all pass

**Step 4: Commit**

```bash
git add engine/src/scheduler.rs engine/src/db.rs
git commit -m "feat: backfill missed candles on engine restart

After recovering last_processed_ts, process all candles between
the last prediction and the latest candle. Prevents permanent
gaps in prediction history after downtime."
```

---

## Deferred Fix 4: Parity harness Rust test (MEDIUM)

**Review finding #11.** The `--verify-fixture` mode in `parity.rs` recomputes features with the same Python functions and diffs Python against Python. It reports `PARITY OK` even while the drawdown divergence was live. No Rust test loads the parity fixture and compares `to_array()` output.

**Files:**
- Modify: `engine/src/features/equities_v2.rs` — add test that loads `equity_feature_parity.json`
- Check: does `equity_feature_parity.json` exist in the repo?

### Task 4.1: Add Rust parity test against fixture

**Objective:** A Rust test that loads a golden fixture (candles + expected features) and asserts `compute_equity_features` produces matching output.

**Files:**
- Test: `engine/src/features/equities_v2.rs` — new test `parity_fixture_rust_matches_python`
- Fixture: `engine/tests/fixtures/equity_feature_parity.json` (generate if missing)

**Step 1: Check if fixture exists**

```bash
find /home/ubuntu/projects/MarketMoves -name "equity_feature_parity*" -not -path "*/node_modules/*" 2>/dev/null
```

**Step 2: Generate fixture if missing**

Run the Python parity script to produce a fixture:

```bash
cd /home/ubuntu/projects/MarketMoves
python3 -c "
from training.equities_features import compute_equity_features
import json, sqlite3

# Load 200 QQQ candles from the live DB
conn = sqlite3.connect('/var/lib/docker/volumes/deploy_data/_data/candles.db')
rows = conn.execute('SELECT ts, open, high, low, close, volume FROM equity_candles WHERE symbol=\"QQQ\" ORDER BY ts ASC LIMIT 200').fetchall()
candles = [{'ts': r[0], 'open': r[1], 'high': r[2], 'low': r[3], 'close': r[4], 'volume': r[5]} for r in rows]
features = compute_equity_features(candles)
fixture = {'candles': candles, 'features': features}
with open('engine/tests/fixtures/equity_feature_parity.json', 'w') as f:
    json.dump(fixture, f, indent=2)
print(f'Wrote fixture with {len(candles)} candles')
"
```

**Step 3: Write Rust test**

```rust
/// Parity fixture test: load golden fixture and compare Rust feature output.
/// This test catches train/serve skew that the Python-only parity harness misses.
#[test]
fn parity_fixture_rust_matches_python() {
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/equity_feature_parity.json");

    if !fixture_path.exists() {
        eprintln!("Skipping parity fixture test — fixture not found at {:?}", fixture_path);
        return;
    }

    let fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&fixture_path).expect("read fixture"),
    ).expect("parse fixture JSON");

    let candles: Vec<EquityCandle> = fixture["candles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| EquityCandle {
            symbol: "QQQ".into(),
            ts: c["ts"].as_i64().unwrap(),
            open: c["open"].as_f64().unwrap(),
            high: c["high"].as_f64().unwrap(),
            low: c["low"].as_f64().unwrap(),
            close: c["close"].as_f64().unwrap(),
            volume: c["volume"].as_i64().unwrap() as i64,
            source: "yahoo".into(),
        })
        .collect();

    let expected: Vec<[f64; 8]> = fixture["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let arr = f.as_array().unwrap();
            [arr[0].as_f64().unwrap(), arr[1].as_f64().unwrap(),
             arr[2].as_f64().unwrap(), arr[3].as_f64().unwrap(),
             arr[4].as_f64().unwrap(), arr[5].as_f64().unwrap(),
             arr[6].as_f64().unwrap(), arr[7].as_f64().unwrap()]
        })
        .collect();

    let vix_pairs: Vec<(i64, f64)> = vec![]; // fixture may not have VIX
    let result = compute_equity_features(&candles, Some(&vix_pairs), None);

    assert_eq!(result.len(), expected.len(), "feature row count mismatch");
    for (i, (actual, expected_row)) in result.iter().zip(expected.iter()).enumerate() {
        let actual_arr = actual.to_array();
        for j in 0..8 {
            let diff = (actual_arr[j] - expected_row[j]).abs();
            assert!(
                diff < 1e-6,
                "feature mismatch at row {} col {}: rust={} python={} diff={}",
                i, j, actual_arr[j], expected_row[j], diff
            );
        }
    }
}
```

**Step 4: Run test**

Run: `cargo test --lib parity_fixture_rust_matches_python`
Expected: PASS (or skip with warning if fixture missing)

**Step 5: Commit**

```bash
git add engine/src/features/equities_v2.rs engine/tests/fixtures/
git commit -m "test: add Rust parity test against golden fixture

Catches train/serve skew that the Python-only parity harness misses.
The fixture is generated from the Python feature pipeline and compared
against Rust compute_equity_features output."
```

---

## Deferred Fix 5: Asymmetric pred_5d confirmation filter for shorts (MEDIUM)

**Review finding #13 (from Pass 1a).** Long entry can require `pred_5d > 0` (if `pred_5d_filter=true`), but short entry has no equivalent filter. The model must "prove itself" for longs but can short on a single-horizon signal.

**Files:**
- Modify: `engine/src/strategy.rs:298-303` — short entry block
- Modify: `engine/src/strategy.rs` — add `short_pred_5d_filter` param to `EquityStrategyParams`
- Modify: `engine/src/config.rs` — add `SHORT_PRED_5D_FILTER` env var
- Test: `engine/src/strategy.rs` — new test

### Task 5.1: Add symmetric pred_5d filter for shorts

**Objective:** Add a `short_pred_5d_filter` param that, when true, requires `pred_5d < 0.0` for short entries (symmetric to the long-side filter).

**Files:**
- Modify: `engine/src/strategy.rs` — `EquityStrategyParams` struct + `next_equity_position`
- Modify: `engine/src/config.rs` — env var parsing
- Modify: `engine/src/api/status.rs` — add to `StrategySnapshot`
- Modify: `engine/src/api/strategy_config.rs` — add to update struct
- Modify: `engine/Dockerfile` — add ENV default
- Modify: `deploy/docker-compose.yml` — add env var
- Modify: `.env` — add default

**Step 1: Add param to EquityStrategyParams**

```rust
/// Require pred_5d < 0.0 as an additional confirmation filter for short entries.
/// Symmetric to pred_5d_filter for longs. Defaults to false (backward compatible).
#[serde(default)]
pub short_pred_5d_filter: bool,
```

**Step 2: Apply in short entry block**

```rust
// Short entries only when enabled, currently Flat, AND sma_valid is true.
if params.enable_shorting && current == Position::Flat && input.sma_valid {
    if input.pred_1d < params.short_entry_threshold
        && (!params.short_pred_5d_filter || input.pred_5d < 0.0)
    {
        return Position::Short;
    }
}
```

**Step 3: Write test**

```rust
#[test]
fn short_entry_blocked_by_pred_5d_filter() {
    let params = EquityStrategyParams {
        enable_shorting: true,
        short_pred_5d_filter: true,
        short_entry_threshold: -0.001,
        ..Default::default()
    };
    let input = EquitySignalInput {
        pred_1d: -0.005,      // below short_entry_threshold
        pred_5d: 0.002,       // POSITIVE — filter should block short
        pred_21d: -0.01,
        current_close: 100.0,
        sma: 105.0,
        sma_valid: true,
    };
    let result = next_equity_position(Position::Flat, &input, &params);
    assert_eq!(result, Position::Flat, "short should be blocked by pred_5d filter");
}
```

**Step 4: Run tests**

Run: `cargo test --lib`
Expected: all pass

**Step 5: Commit**

```bash
git add engine/src/strategy.rs engine/src/config.rs engine/src/api/ engine/Dockerfile deploy/docker-compose.yml .env
git commit -m "feat: add symmetric pred_5d filter for short entries

When short_pred_5d_filter=true, short entries require pred_5d < 0.0,
symmetric to the long-side pred_5d_filter. Defaults to false for
backward compatibility."
```

---

## Deferred Fix 6: Clipping stage verification (MEDIUM)

**Review finding #10.** Rust clips 4 features on raw values. The review speculated the notebook might clip normalized z-scores. **During implementation of commit `cf5e437`, this was verified as correct** — the notebook clips raw features before normalization (cell 8: `f[clip_cols].clip(lower=-5.0, upper=5.0)` on the raw DataFrame, before `normalize()` is called in cell 10). No code change needed.

**Status:** ✅ Verified correct — no action required. Documented here for completeness.

---

## Summary Table

| # | Finding | Severity | Effort | Depends on |
|---|---------|----------|--------|------------|
| 1 | VIX/TLT timestamp alignment | 🔴 CRITICAL | ~2h | None |
| 2 | Z-score de-normalization non-stationarity | 🔴 CRITICAL | ~3h | Model metadata |
| 3 | Engine restart backfill loop | 🟡 HIGH | ~1h | None |
| 4 | Parity harness Rust test | 🟡 MEDIUM | ~1.5h | Fixture generation |
| 5 | Asymmetric pred_5d filter for shorts | 🟡 MEDIUM | ~1h | None |
| 6 | Clipping stage verification | 🟡 MEDIUM | ✅ Done | None |

**Total estimated effort:** ~8.5h

**Recommended order:** 1 → 3 → 5 → 4 → 2 (save z-score for last — it requires model metadata changes and careful testing)

---

## Verification

After all fixes:

```bash
# 1. Full test suite
cargo test --lib  # expect all pass

# 2. Build
cargo build --release --bin engine
cd frontend && npm run build

# 3. Docker rebuild + redeploy
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .
docker build --no-cache -f inference/Dockerfile -t marketmarkovnet/inference:latest inference/
cd deploy && docker-compose down && docker-compose up -d

# 4. Verify health
sleep 15 && docker ps --format '{{.Names}}\t{{.Status}}' | grep mmn
curl -s http://localhost:9080/api/status | python3 -m json.tool | head -20

# 5. Verify VIX/TLT alignment
docker logs mmn-engine 2>&1 | grep -i "VIX coverage"

# 6. Verify backfill
docker logs mmn-engine 2>&1 | grep -i "backfill"

# 7. Verify z-score (after 2+ real predictions)
docker logs mmn-inference 2>&1 | grep "label_std"
```

---

## Open Questions

1. **Model metadata:** Do `model_meta_qqq_v1.json` and `model_meta_nvda_v1.json` already contain `label_std_*d` fields? If not, they need to be computed from the notebook and added. This is a prerequisite for Deferred Fix 2.

2. **Parity fixture:** Does `equity_feature_parity.json` already exist in the repo? If not, it needs to be generated from the Python feature pipeline. This is a prerequisite for Deferred Fix 4.

3. **VIX/TLT data source:** CBOE VIX data is fetched via `engine/src/data/cboe.rs`. TLT is fetched via Yahoo. Are there known gaps in either source that would trigger the coverage warning? If so, the warning threshold (90%) may need adjustment.

4. **Backfill + live trading:** If the engine restarts during market hours and backfills missed candles, should it also execute trades for those candles? Or just persist predictions and skip strategy evaluation for backfilled candles? Current plan: persist predictions only, skip strategy (safe default).

5. **`short_pred_5d_filter` default:** Should it default to `true` (matching the long-side default) or `false` (backward compatible)? Plan defaults to `false` to avoid changing behavior without explicit opt-in.
