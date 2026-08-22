# Directional Accuracy: Per-Symbol & Time-Windowed (2026-08-16)

The accuracy pipeline that backs the dashboard's "ModelHealth" panel
(`directional_1d/5d/21d`, `MAE_1d/5d/21d`, `resolved_count`) was hardened
in three ways this session. Recipe below documents the post-fix shape.

## Endpoints

```
GET /api/accuracy?symbol=QQQ                          # all-time, default
GET /api/accuracy?symbol=SMH&since=30                 # last 30 days
GET /api/accuracy?symbol=XLF&since=90                 # last 90 days
```

- `symbol` (optional): defaults to the engine's primary `cfg.symbol`.
  Already supported pre-fix; verified that `?symbol=SMH` and `?symbol=XLF`
  work as long as predictions exist in `equity_predictions` for that symbol.
- `since` (optional, days): returns accuracy over predictions at/after
  `now - since_days`. When absent or `0`, the endpoint falls back to
  most-recent-500 all-time. Valid range: any positive integer (days).

Response shape (`engine/src/api/predictions.rs::AccuracyResponse`):
```json
{
  "directional_1d": 51.3,
  "directional_5d": 49.1,
  "directional_21d": 55.8,
  "mae_1d": 0.0081,
  "mae_5d": 0.0142,
  "mae_21d": 0.0263,
  "resolved_count": 187
}
```

Field-name history: pre-fix this struct used `directional_1h/4h/24h` even
though the equity query computed 1d/5d/21d. Renamed to match honest
semantics. **Frontend code that reads these fields by old `*_1h` names
will see `undefined`** — verify consumer code was updated alongside.

## How accuracy is computed

`engine/src/db.rs::fetch_equity_accuracy_since(pool, symbol, since_ts)`:

1. Fetch `equity_predictions` for symbol (filtered by `since_ts` if > 0,
   capped at 500 if not).
2. For each prediction at `candle_ts`, resolve each horizon (1d/5d/21d) by
   finding the candle nearest `candle_ts + offset` in `equity_candles`
   (3-day tolerance to skip weekends/holidays). Uses
   `find_closest_close()` binary search.
3. Compute directional accuracy: `(pred >= 0) == (actual >= 0)` where
   `actual = ln(future_close / base_close).ln()`.
4. Compute MAE: `|pred - actual|`.
5. `resolved_count` = number of 1d resolutions (most populated).

**Sample-size warning:** the 21d horizon takes 21 calendar days to
resolve. With `--since 30`, you get at most ~9 resolved 21d samples —
statistically meaningless. Use `--since 90+` for the 21d metric to be
useful. The 1d horizon resolves in 1-3 days, so any window works.

## Required pre-conditions

For accuracy to compute for a symbol X:

1. **Candles exist:** at least 2+ rows in `equity_candles` for symbol X.
   Verify: `SELECT COUNT(*) FROM equity_candles WHERE symbol='X'`.
2. **Predictions exist:** at least 1 row in `equity_predictions` for X,
   AND enough future candles exist to resolve at least one horizon. A
   prediction needs `today + horizon_days` of candles to compute the
   actual. Predictions from the last `horizon` days cannot resolve
   themselves — they need FUTURE candles.

**Fresh-model symptom:** if `equity_predictions` has only 1 row for a
symbol (today's candle), `resolved_count` will be 0 and the endpoint
returns HTTP 503 "no resolved predictions". This is the symptom of the
fresh-model backfill pitfall — see `SKILL.md` "Fresh-model backfill
silently skips."

## Fix the underlying data: scheduler backfill (already deployed)

For a fresh model (e.g. SMH enabled today):
```bash
# 1. engine/src/scheduler.rs backfill block now seeds from earliest candle
#    when last_processed_ts is None (committed 03bd6a2).
# 2. Rebuild engine:
cd /home/ubuntu/projects/MarketMoves
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .
# 3. Restart engine (stop+rm+up avoids the docker-compose v1 KeyError):
docker-compose -f deploy/docker-compose.yml stop engine \
  && docker-compose -f deploy/docker-compose.yml rm -f engine \
  && docker-compose -f deploy/docker-compose.yml up -d --no-deps engine
# 4. Watch backfill progress:
docker logs mmn-engine -f | grep -E "backfilling|equity prediction persisted"
# 5. Once backfill done, verify:
sudo python3 - <<'PY'
import sqlite3
db = sqlite3.connect('/var/lib/docker/volumes/deploy_data/_data/candles.db')
c = db.cursor()
print("=== predictions per symbol ===")
for s,n,*_ in c.execute("SELECT symbol, COUNT(*) FROM equity_predictions GROUP BY symbol"):
    print(f"  {s}: {n}")
PY
```

## Backfill cost

Wall-clock estimate from Aug 16, 2026 first-run:
- ~2s per inference call (TCN + LGBM ZMQ)
- 1255 candles per symbol x 2 fresh symbols (SMH + XLF) = 2510 inferences
- ~84 minutes total for both to fully backfill to inception
- For "good enough" 90d: 180 days x 2 = 360 inferences = ~12 minutes

The 90-day cap is now deployable via `BACKFILL_DAYS` env var (default 90).
Deployed as `e130837` (Config + scheduler wiring) and `4c6dff2`
(`docker-compose.yml` default). Set to 0 to disable the cap and accept
full-history runtime.

**Important follow-up bug:** the cap alone is not enough. After applying
the cap fix, you may see predictions persist for every historical candle
but with byte-identical values across distinct `candle_ts` — that means
the scheduler fed every historical candle the *same current* feature
window. Symptom: 9 SMH rows all show `pred_1d=0.0539...` for 9 different
timestamps. See the "Fresh-model backfill produces IDENTICAL predictions
for every candle" pitfall in SKILL.md for the root cause and fix
(`fetch_equity_candles_asc_before` in `engine/src/db.rs` +
`process(candle_ts)` updated to call it, committed `d566a1b`).

## Why accuracy was pinned to QQQ before the fix

The endpoint accepted `?symbol=` already, but `trading_models` table was
the only place models were registered. Before SMH/XLF were added there
(2026-08-16), `?symbol=SMH` would return 0 predictions and 503. After
they were added: 1 prediction each (today's), still 503. After backfill:
hundreds of rows per symbol, accuracy populates normally.
