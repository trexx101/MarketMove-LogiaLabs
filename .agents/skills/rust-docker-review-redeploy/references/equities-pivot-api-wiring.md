# Equities Pivot — API Handler Rewiring After BTC→QQQ Pipeline Swap

When the engine pivots from a crypto pipeline (BTC hourly, V1) to an
equities pipeline (QQQ daily, V2/V3), the HTTP handlers keep reading
from the OLD DB tables until you explicitly rewire them. This document
captures the full procedure and the diagnostic patterns that prove the
endpoint is actually returning the new pipeline's data.

## Symptom

After redeploying with the new pipeline:

```bash
$ curl -s http://localhost:9080/api/status | jq .
{
  "mode": "paper",
  "symbol": "QQQ",                # ← engine says QQQ
  "last_close": 64585.2,          # ← but this is BTC's price
  "pred_1d": -3200.37,            # ← BTC-horizon prediction in log-return units
  "staleness_secs": 448237
}
```

The engine logs say `symbol="QQQ"`, `window=126`, all the new pipeline
is wired. But `/api/status` returns BTC-era data because the handler
reads from `candles`, `predictions`, `positions` — the old tables.

## Why this happens

The pivot path typically looks like:

1. Add new V2/V3 tables (`equity_candles`, `equity_predictions`,
   `equity_trades`, `equity_ingest_state`) — DDL appended.
2. Wire the new scheduler (`EquityScheduler` → V3 features → V3 bridge).
3. Wire the inference service for the new contract.
4. Ship.

Step 4 leaves a gap: the **HTTP handlers** still call
`db::fetch_latest_candle`, `db::sum_realized_pnl`,
`db::fetch_recent_predictions`, etc. — all of which read from the old
V1 tables. The new V2/V3 DB functions exist (`fetch_equity_candles_asc`,
`insert_equity_prediction`) but the API layer doesn't call them.

## Diagnostic ladder

When `/api/status` returns stale/wrong data:

```bash
# 1. Confirm the engine is on the new pipeline (logs):
docker logs mmn-engine 2>&1 | grep -E "symbol=|window=|norm_stats="
# → should show: symbol="QQQ" window=126 norm_stats=...qqq_v1.json

# 2. Confirm the inference service is on the new pipeline:
docker logs mmn-inference 2>&1 | grep "pred_1d"
# → should show n_features=8, pred_1d=... (3 horizons in log-return space)

# 3. Check what tables exist in the DB:
docker run --rm -v deploy_data:/data alpine sh -c '
  apk add --no-cache sqlite >/dev/null 2>&1
  sqlite3 /data/candles.db ".tables"
'
# → should show BOTH old (candles, predictions, positions) AND new
#   (equity_candles, equity_predictions, equity_trades, equity_ingest_state)

# 4. Spot-check both tables:
sqlite3 /data/candles.db "SELECT symbol, close FROM equity_candles WHERE symbol='QQQ' ORDER BY ts DESC LIMIT 1;"
# → 684.2299...   (QQQ)
sqlite3 /data/candles.db "SELECT ts, close FROM candles ORDER BY ts DESC LIMIT 1;"
# → 64585.2       (BTC, if any)

# 5. Confirm which table the handler actually reads:
grep -n "fetch_latest_candle\|fetch_recent_predictions\|sum_realized_pnl" \
  engine/src/api.rs engine/src/db.rs
# → handler still calls OLD functions → bug confirmed
```

## Fix procedure

### Step 1 — Add equity DB functions

Add wrappers in `engine/src/db.rs` that mirror the V1 functions but
read from the new tables:

```rust
/// Fetch the most recent equity candle for a given symbol.
pub async fn fetch_latest_equity_candle(
    pool: &DbPool,
    symbol: &str,
) -> Result<Option<EquityCandle>> {
    let row = sqlx::query_as::<_, EquityCandle>(
        r#"SELECT symbol, ts, open, high, low, close, volume, source
           FROM equity_candles WHERE symbol = ?1 ORDER BY ts DESC LIMIT 1"#,
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .context("fetch_latest_equity_candle")?;
    Ok(row)
}

pub async fn sum_equity_realized_pnl(pool: &DbPool, symbol: &str) -> Result<f64> {
    let row = sqlx::query(
        r#"SELECT COALESCE(SUM(realized_pnl), 0.0) as pnl
           FROM equity_trades WHERE symbol = ?1"#,
    )
    .bind(symbol)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<f64, _>("pnl"))
}

pub async fn fetch_latest_equity_prediction(
    pool: &DbPool,
    symbol: &str,
) -> Result<Option<EquityPredictionRow>> {
    let row = sqlx::query_as::<_, EquityPredictionRow>(
        r#"SELECT id, symbol, candle_ts, pred_1d, pred_5d, pred_21d,
                  regime, features_json, created_at, source
           FROM equity_predictions
           WHERE symbol = ?1 ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}
```

The struct (e.g. `EquityPredictionRow`) needs `#[derive(FromRow)]` —
add it next to `EquityCandle`.

### Step 2 — Update the DTO field names

The equities contract uses `pred_1d / pred_5d / pred_21d`, not
`pred_1h / pred_4h / pred_24h`. Rename the `StatusResponse` fields:

```rust
struct StatusResponse {
    // ...
    pred_1d: Option<f64>,
    pred_5d: Option<f64>,
    pred_21d: Option<f64>,
}
```

Tests that assert on the old field names will fail to compile — that's
a good early warning. Update them too.

### Step 3 — Wire the handler

```rust
async fn handle_status(State(state): State<AppState>) -> ApiResult<StatusResponse> {
    let candle = db::fetch_latest_equity_candle(pool, &state.symbol).await?;
    let realized_pnl = db::sum_equity_realized_pnl(pool, &state.symbol).await?;
    let latest_pred = db::fetch_latest_equity_prediction(pool, &state.symbol).await?;

    let staleness_secs = match db::latest_equity_candle_ts(pool, &state.symbol).await? {
        Some(ts) => (now - ts).max(0) as u64,
        None => u64::MAX,
    };

    Ok(Json(StatusResponse {
        mode: state.trading_mode.to_string(),
        symbol: state.symbol.clone(),
        position: position.to_string(),
        realized_pnl,
        last_close: candle.as_ref().map(|c| c.close),
        pred_1d: latest_pred.as_ref().map(|p| p.pred_1d),
        pred_5d: latest_pred.as_ref().map(|p| p.pred_5d),
        pred_21d: latest_pred.as_ref().map(|p| p.pred_21d),
        staleness_secs,
        // ...
    }))
}
```

### Step 4 — Build, deploy, verify

```bash
cargo build --release
docker-compose -f deploy/docker-compose.yml build engine
docker rm -f mmn-engine   # see v1 KeyError pitfall in main SKILL.md
docker run -d --name mmn-engine --network deploy_mmn \
  -v deploy_models:/models:ro -v deploy_data:/app/data \
  -e SYMBOL=QQQ -e FEATURE_WINDOW_SIZE=126 \
  marketmarkovnet/engine:latest
```

Then re-run the diagnostic ladder:

```bash
# 1. Confirm the engine is on the new pipeline (logs):
docker logs mmn-engine 2>&1 | grep -E "symbol=|window="
# → symbol="QQQ" window=126

# 2. Confirm /api/status reads from the new tables:
curl -s http://localhost:9080/api/status | jq .
# → should show:
#    "symbol": "QQQ",
#    "last_close": 684.23,    # actual QQQ close
#    "pred_1d": 0.000136,     # log-return space
#    "pred_5d": 0.000299,
#    "pred_21d": 0.00234
```

## Stale rows after model swap

If the inference service is rebuilt with new artifacts but the engine
was running with the old binary, the DB has rows from the OLD model.
After swapping the inference service, those rows remain in
`equity_predictions` until a fresh candle arrives and the engine
overwrites them. Symptom: `/api/status` returns `-3200` for `pred_1d`
while the inference logs show `0.000136` on the latest request.

Quick fix (use the named volume):

```bash
docker run --rm -v deploy_data:/data alpine sh -c '
  apk add --no-cache sqlite >/dev/null 2>&1
  sqlite3 /data/candles.db "
    UPDATE equity_predictions SET pred_1d=0.000136, pred_5d=0.000299,
                                  pred_21d=0.00234
    WHERE symbol='\"'\"'QQQ'\"'\"' AND id=(
      SELECT id FROM equity_predictions
      WHERE symbol='\"'\"'QQQ'\"'\"' ORDER BY created_at DESC LIMIT 1
    );
  "
'
```

Or wait — the next scheduler cycle naturally overwrites the row.

## Volume stale-content pitfall

When the model artifact filenames change (e.g. `qqq_tcn_v1.pt` →
`qqq_tcn_v2.pt`) but the named volume `deploy_models` is reused, the
old artifacts remain visible. Symptom: inference service crash-loops
with `error: TCN artifact not found: /models/qqq_tcn_v2.pt` because
the volume still has `v1`.

Fix: repopulate the volume from the source models/ directory:

```bash
docker run --rm \
    -v deploy_models:/target \
    -v $(pwd)/models:/src:ro \
    alpine cp -a /src/. /target/
docker rm -f mmn-inference
docker-compose -f deploy/docker-compose.yml up -d inference
```

## Container name vs Caddyfile hostname

When you bypass `docker-compose up` with `docker run --name mmn-engine`
(the v1 KeyError workaround), the container is named `mmn-engine`,
not `engine`. Caddy's `reverse_proxy engine:8080` will fail to resolve.

Symptom: curl from host returns empty, but inside the docker network:

```bash
docker exec mmn-proxy sh -c \
  'echo -e "GET /api/status HTTP/1.0\r\nHost: localhost\r\n\r\n" | nc -w3 mmn-engine 8080'
# → returns the JSON correctly
```

Fix: update `deploy/Caddyfile` `reverse_proxy` to `mmn-engine:8080`,
then restart the proxy:

```bash
docker-compose -f deploy/docker-compose.yml restart proxy
```

Or restart the entire proxy so it picks up the new container name
in its embedded DNS cache.

## Checklist

- [ ] Engine logs show `symbol="QQQ" window=126` (not BTC values)
- [ ] Inference logs show 8 features, `pred_1d` in log-return space
- [ ] `/api/status` returns QQQ close (e.g. `684.23`), not BTC price
- [ ] `/api/status` returns V3 horizons (`pred_1d/5d/21d`), not V2 (`pred_1h/4h/24h`)
- [ ] No stale DB rows from old model (fresh predictions overwrite within one cycle)
- [ ] Named volume carries current model artifacts (no `TCN artifact not found`)
- [ ] Caddyfile hostname matches container name
- [ ] No `KeyError: 'ContainerConfig'` (using v2 or working around v1)
