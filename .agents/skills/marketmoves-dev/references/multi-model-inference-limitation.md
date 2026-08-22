# Multi-Model Inference Limitation & Prediction Schema Fix

**Status: RESOLVED (2026-08-07).** Option B (multi-model inference service)
and the schema fix are now implemented. See `references/multi-model-inference.md`
for the implementation details and `references/z-score-blending.md` for the
blending architecture.

## Problem (historical)

When the engine runs multiple models (e.g., `qqq-v1` and `nvda-v1`), both
schedulers send inference requests to the **same** inference container.
The inference service loads **one** model bundle at startup (TCN + 3 LGBM
models) from environment variables:

```
TCN_PATH=/models/qqq_tcn_v1.pt
LGBM_H1_PATH=/models/qqq_lgbm_h1_v1.pkl
LGBM_H5_PATH=/models/qqq_lgbm_h5_v1.pkl
LGBM_H21_PATH=/models/qqq_lgbm_h21_v1.pkl
```

**Consequence:** Even if the NVDA scheduler sends a request, it gets
predictions from the QQQ model. The predictions are written to
`equity_predictions` with `symbol='NVDA'`, but they are QQQ predictions.

**Second problem:** The `equity_predictions` table has a
`UNIQUE(candle_ts)` constraint. When both QQQ and NVDA schedulers run,
they generate predictions with the same timestamp. The second insert
fails with a conflict, so only one symbol's predictions survive.

## Detection

Symptom: Dashboard shows QQQ predictions correctly, but NVDA shows blank
predictions (null `pred_1d`, `pred_5d`, `pred_21d`).

Check the database:
```sql
SELECT symbol, COUNT(*) FROM equity_predictions GROUP BY symbol;
-- Returns only ('QQQ', N), no NVDA rows
```

Check `/api/accuracy?symbol=NVDA`:
```json
{"error": "equity accuracy not yet implemented or no resolved predictions"}
```

## Root Cause Confirmation

```bash
# Inference logs show only one model loaded at startup
docker logs mmn-inference 2>&1 | grep -i "configured"
# Shows: tcn=/models/qqq_tcn_v1.pt lgbm=[...qqq...]

# Engine logs show NVDA scheduler sending requests
docker logs mmn-engine 2>&1 | grep -i "nvda"
# Shows: model_id=nvda-v1 primary=NVDA ... but predictions go to inference

# The inference service has no model selection logic
cat inference/equity_model.py | grep -i "model_id\|symbol\|select"
# Returns nothing — the service is model-agnostic and stateless
```

## Schema Fix

Change the unique constraint on `equity_predictions` from
`UNIQUE(candle_ts)` to `UNIQUE(symbol, candle_ts)`:

```rust
// engine/src/db.rs — in the DDL string
CREATE TABLE IF NOT EXISTS equity_predictions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    symbol        TEXT    NOT NULL,
    candle_ts     INTEGER NOT NULL,
    pred_1d       REAL    NOT NULL,
    pred_5d       REAL    NOT NULL,
    pred_21d      REAL    NOT NULL,
    regime        TEXT    NOT NULL DEFAULT 'unknown',
    features_json TEXT    NOT NULL DEFAULT '{}',
    created_at    INTEGER NOT NULL,
    source        TEXT    NOT NULL DEFAULT 'qqq_tcn_v1',
    UNIQUE(symbol, candle_ts)  -- <-- FIX: was UNIQUE(candle_ts)
)
```

**Migration for existing DBs:**
```rust
pub async fn migrate_equity_predictions_symbol_unique(pool: &DbPool) -> Result<()> {
    // SQLite cannot drop UNIQUE constraints directly; must recreate table
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS equity_predictions_new (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            symbol TEXT NOT NULL,
            candle_ts INTEGER NOT NULL,
            pred_1d REAL NOT NULL,
            pred_5d REAL NOT NULL,
            pred_21d REAL NOT NULL,
            regime TEXT NOT NULL DEFAULT 'unknown',
            features_json TEXT NOT NULL DEFAULT '{}',
            created_at INTEGER NOT NULL,
            source TEXT NOT NULL DEFAULT 'qqq_tcn_v1',
            UNIQUE(symbol, candle_ts)
        )"#
    ).execute(pool).await?;

    // Copy data
    sqlx::query(
        r#"INSERT INTO equity_predictions_new
           (symbol, candle_ts, pred_1d, pred_5d, pred_21d, regime, features_json, created_at, source)
           SELECT symbol, candle_ts, pred_1d, pred_5d, pred_21d, regime, features_json, created_at, source
           FROM equity_predictions"#
    ).execute(pool).await?;

    // Swap tables
    sqlx::query("DROP TABLE equity_predictions").execute(pool).await?;
    sqlx::query("ALTER TABLE equity_predictions_new RENAME TO equity_predictions")
        .execute(pool).await?;

    Ok(())
}
```

## Solution Paths

### Option A: Per-Model Inference Containers (Recommended)

Run a separate inference container for each model:

```yaml
# deploy/docker-compose.yml
services:
  inference-qqq:
    image: marketmarkovnet/inference:latest
    environment:
      TCN_PATH: /models/qqq_tcn_v1.pt
      LGBM_H1_PATH: /models/qqq_lgbm_h1_v1.pkl
      # ... etc
      ZMQ_BIND: tcp://0.0.0.0:5555

  inference-nvda:
    image: marketmarkovnet/inference:latest
    environment:
      TCN_PATH: /models/NVDA/nvda_tcn_v1.pt
      LGBM_H1_PATH: /models/NVDA/nvda_lgbm_h1_v1.pkl
      # ... etc
      ZMQ_BIND: tcp://0.0.0.0:5556
```

Update `trading_models` registry to include `inference_endpoint` per model:

```sql
ALTER TABLE trading_models ADD COLUMN inference_endpoint TEXT NOT NULL DEFAULT 'tcp://inference:5555';
UPDATE trading_models SET inference_endpoint='tcp://inference-qqq:5555' WHERE model_id='qqq-v1';
UPDATE trading_models SET inference_endpoint='tcp://inference-nvda:5556' WHERE model_id='nvda-v1';
```

Modify `EquityScheduler::new` to accept a per-model ZMQ endpoint instead of
using `cfg.zmq_endpoint` globally.

### Option B: Multi-Model Inference Service

Modify `inference/equity_model.py` to load multiple model bundles and select
by `model_id` in the request:

```python
# Request format:
{"schema_version": 3, "model_id": "nvda-v1", "feature_window": [...]}

# Service loads all models at startup:
models = {
    "qqq-v1": EquityEnsemble(tcn=load_tcn('/models/qqq_tcn_v1.pt'), ...),
    "nvda-v1": EquityEnsemble(tcn=load_tcn('/models/NVDA/nvda_tcn_v1.pt'), ...),
}

# Predict uses model_id to select:
ensemble = models[request["model_id"]]
preds = ensemble.predict(feature_window)
```

Pros: One container, simpler deployment.
Cons: Higher memory usage (all models resident), more complex code.

### Option C: Backfill with Python Script (Immediate Fix)

Before the infrastructure supports multi-model inference, generate NVDA
predictions offline and insert them into `equity_predictions`.

**Prerequisites:** The host Python environment needs `torch`, `lightgbm`,
`scikit-learn`, `numpy`, and `sqlite3`. The `marketmarkovnet/inference` venv
has `torch` and `lightgbm` but may be missing `scikit-learn` — install it first:

```bash
cd /home/ubuntu/projects/MarketMoves/inference
uv pip install scikit-learn
```

**The full working script** is at `references/backfill-nvda-predictions.py`.
It loads the NVDA model bundle, fetches NVDA candles from the SQLite DB,
computes 8-dim features, normalizes with NVDA norm_stats, generates
predictions for each day with enough history (SEQ_LEN=126), and inserts
them into `equity_predictions` with `ON CONFLICT(symbol, candle_ts) DO UPDATE`.

Key steps in the script:
1. Load NVDA TCN + 3 LGBM models from `/models/NVDA/`
2. Load NVDA norm_stats JSON
3. Fetch NVDA candles from `equity_candles` table
4. Fetch VIX and TLT candles for the feature regressors
5. Compute 8-dim features using the same formulas as `training/equities_features.py`
6. Normalize with median/MAD from NVDA norm_stats
7. For each candle with enough history, create a 126-window slice and call `EquityEnsemble.predict()`
8. Insert with `ON CONFLICT(symbol, candle_ts)` to avoid overwriting existing QQQ predictions

**Pitfall — scikit-learn missing:** The `lightgbm.sklearn.LGBMRegressor` class
requires `scikit-learn` to be installed. Without it, `predict()` raises
`TypeError: 'NoneType' object is not callable`. Install with `uv pip install scikit-learn`.

**Pitfall — DB access:** The Docker volume is owned by root. Copy the DB to
a writable location first:

```bash
sudo cp /var/lib/docker/volumes/deploy_data/_data/candles.db /tmp/candles_nvda.db
sudo chown ubuntu:ubuntu /tmp/candles_nvda.db
```

Run the script, then copy the DB back:

```bash
sudo cp /tmp/candles_nvda.db /var/lib/docker/volumes/deploy_data/_data/candles.db
sudo chown 1000:1000 /var/lib/docker/volumes/deploy_data/_data/candles.db
```

**Expected output:** ~1069 predictions inserted for NVDA (1255 candles minus
FEATURE_WINDOW_SIZE + 60 warmup = 1069). After inserting, `/api/accuracy?symbol=NVDA`
returns directional accuracy and MAE from the backfilled predictions.

## Verification After Fix

```bash
# Check predictions exist for both symbols
docker exec mmn-engine sqlite3 /app/data/candles.db \
  "SELECT symbol, COUNT(*) FROM equity_predictions GROUP BY symbol"
# Should show: QQQ|N, NVDA|M

# Check accuracy endpoint works for both
curl -s "http://localhost:9080/api/accuracy?symbol=QQQ" | python3 -m json.tool
curl -s "http://localhost:9080/api/accuracy?symbol=NVDA" | python3 -m json.tool
# Both should return directional accuracy values, not 503

# Check dashboard model switcher
# Switch from QQQ to NVDA in the UI; predictions should populate
```

## Related

- `references/multi-model-registry.md` — The `trading_models` table schema
- `references/multi-model-dashboard-migration.md` — Frontend per-model store
- `references/deploy-multi-model-2026-08-06.md` — Full deploy sequence
