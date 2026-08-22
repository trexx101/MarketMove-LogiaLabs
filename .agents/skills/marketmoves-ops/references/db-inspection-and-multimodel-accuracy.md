# Live DB inspection + multi-model accuracy gap

## Inspecting the live SQLite DB (the reliable path)

The engine container (`mmn-engine`) is a **distroless/scratch Rust image**:
- `docker exec mmn-engine python3 ...` → `exec: "python3": executable file not found`
- No `sqlite3` CLI inside the container, and the host often lacks `sqlite3` too.

The DB lives in a Docker volume owned by root with mode `drwx-----x`:
- Volume mount source: `/var/lib/docker/volumes/deploy_data/_data`
- DB file: `/var/lib/docker/volumes/deploy_data/_data/candles.db`
- The `ubuntu` user **cannot** open it directly (`OperationalError: unable to open database file`) — needs `sudo`.

**Working pattern:**
```bash
sudo python3 - <<'PY'
import sqlite3
db = '/var/lib/docker/volumes/deploy_data/_data/candles.db'
conn = sqlite3.connect(db)
c = conn.cursor()
for row in c.execute("SELECT symbol, COUNT(*), MIN(candle_ts), MAX(candle_ts) FROM equity_predictions GROUP BY symbol"):
    print(row)
conn.close()
PY
```

Do NOT waste turns trying `sqlite3`, `docker exec ... python3`, or host `python3` without `sudo` on the volume path.

## Key tables for diagnosis

- `trading_models` — drives multi-model scheduling. Columns:
  `model_id, primary_symbol, inverse_symbol, model_path, norm_stats_path, budget_usd, enabled, enabled_at, mean_ic, first_candle_ts, notes`.
  One scheduler is spawned **per enabled row** in `main.rs` (loop over `resolve_active_models`).
- `equity_predictions` — per-symbol daily predictions (`symbol, candle_ts, pred_1d, pred_5d, pred_21d, regime, source`).
- `equity_candles` — OHLCV per symbol (`symbol, ts, open, high, low, close, volume, ...`).

## The multi-model accuracy gap (diagnostic, 2026-08-16)

When SMH/XLF showed only 1 prediction each while QQQ had ~1100, the cause was
**NOT a broken pipeline** — it was a fresh-model-no-backfill reality:

1. `trading_models` had `smh-v1`/`xlf-v1` with `enabled=1`, but `enabled_at`
   equaled the latest candle ts (models enabled that day).
2. The scheduler `run()` only calls `process()` when a *new* candle arrives
   (`ts > last_processed_ts`). On startup, if `last_processed_ts` is `None`
   (fresh model, no prior predictions), it logs "starting fresh" and does
   **no historical backfill** — it just waits for the next daily candle.
3. So a newly-enabled model accumulates exactly 1 prediction per day forward;
   history only builds over time. QQQ's 1137 rows are because it's run since April.

**`/api/accuracy` limitations (as of 2026-08-16):**
- `fetch_equity_accuracy(pool, symbol)` takes a symbol arg, but the endpoint
  pins it to `cfg.symbol` (QQQ). No `?symbol=` param exposed.
- Hardcoded `ORDER BY candle_ts DESC LIMIT 500` — no date-window filter.
  "last N days" is not a selectable input.
- Backtest endpoint (`/api/backtest`) is a strategy replay (Sharpe/DD/win-rate),
  has **no directional-accuracy output**.

**To make per-model, time-scoped directional accuracy real:**
- Scheduler: when `last_processed_ts == None`, backfill from earliest candle
  (or a `--backfill-days` cap) so new models seed history.
- `fetch_equity_accuracy`: add `since_ts` window param (keep LIMIT as ceiling).
- `/api/accuracy`: accept `?symbol=&since=`; default to `cfg.symbol` + unbounded.
- Dashboard: surface per-model 1d/5d/21d accuracy with a time-window selector.

**Statistical note:** directional accuracy for horizon 1d resolves against the
next-day close. <30 resolved samples is statistically meaningless; for useful
1d accuracy want ~60–90 days of daily data minimum.
