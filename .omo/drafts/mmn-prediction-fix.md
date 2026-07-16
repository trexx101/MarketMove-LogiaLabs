---
slug: mmn-prediction-fix
status: awaiting-approval
intent: clear
review_required: false
pending-action: write .omo/plans/mmn-prediction-fix.md (append todos after approval)
approach: 6-component plan: fix scheduler retry bug, fix data-pipeline/backfill ordering, guard against seq_len=1, add prediction history + accuracy columns, background actuals task + accuracy API, dashboard improvements
---

# Draft: mmn-prediction-fix

## Components (topology ledger)
| id | outcome (one line) | status | evidence path |
|----|--------------------|--------|---------------|
| A | Scheduler stops retry-loop every 30s; prediction only runs once per new confirmed hourly candle | active | engine/src/scheduler.rs:188, :55, :44-61 |
| B | REST backfill completes before scheduler spawns; no seq_len=1 startup race | active | engine/src/main.rs:91-108 (scheduler spawned BEFORE data::run at :131), engine/src/data/mod.rs:19-21 |
| C | process() guards against insufficient candles (< feature_window_size); logs seq_len | active | engine/src/scheduler.rs:65-87, inference/inference_engine.py:117 (seq_len logged) |
| D | predictions table gains actual_1h/4h/24h + accuracy columns (ALTER TABLE migration) | active | engine/src/db.rs:19-27 (predictions DDL), :161-184 (INSERT OR REPLACE) |
| E | Background task computes actuals from candle data + accuracy API endpoint | active | engine/src/db.rs (need new fn), engine/src/api.rs:166 (handle_predictions) |
| F | Dashboard shows prediction history, accuracy metrics (directional %, MAE), staleness indicator | active | frontend/views/predictions.js:36-60, frontend/views/status.js |

## Open assumptions (announced defaults)
| assumption | adopted default | rationale | reversible? |
|------------|----------------|-----------|-------------|
| Accuracy metrics | Directional + Magnitude (both) | User selected this; directional is most actionable for trading, magnitude catches model collapse | yes — add/remove columns |
| Actuals computation | Background task runs every hour, fills actuals for predictions whose N-hour horizon elapsed | User selected this; consistent, doesn't slow API | yes — switch to on-read |
| Backfill ordering | main.rs waits for REST backfill before spawning scheduler | User selected this; eliminates seq_len=1 race at startup | yes — revert to concurrent |
| Prediction retention | Keep one-per-candle model (INSERT OR REPLACE on candle_ts UNIQUE) | Current design is clean; accuracy needs one row per candle, not per request | reversible but breaks accuracy join |
| Candle staleness threshold | 2 hours without a new confirmed candle → log WARN + expose in API | Hourly candles; 2h gap = clearly broken WS | yes — env var |
| Actuals horizon resolution | Mark actual_1h/4h/24h as NULL until the candle at T+N exists; compute log_return(close[T+N] / close[T]) | Standard log-return matching training targets | no — schema-level |

## Findings (cited - path:lines)

### Bug 1: Scheduler 30-second retry loop
- `engine/src/scheduler.rs:188` — `self.last_processed_ts = Some(candle_ts)` is the LAST statement in `process()`, after ALL `?` operations (lines 117, 144, 155). If ANY post-inference DB op fails, `last_processed_ts` is never set → `:55` `ts > self.last_processed_ts.unwrap_or(0)` stays true every 30s poll → inference called every 30s with same stale candle.
- The error is swallowed at `:56-58` (`error!("failed to process candle")`) but `last_processed_ts` stays `None`.
- User sees: inference req_id incrementing every 30s with identical prediction values.

### Bug 2: seq_len=1 / insufficient candle data
- `inference/inference_engine.py:117` — logs `"seq_len": len(feature_window)`. User's log shows `seq_len:1`.
- `engine/src/scheduler.rs:67-77` — fetches `feature_window_size + 1` candles, takes last `feature_window_size`. With only 1-2 candles in DB, feature_window collapses to 1 row.
- `engine/src/main.rs:99-108` — scheduler is spawned via `tokio::spawn` BEFORE `data::run` (line 131) does the REST backfill. Race condition: scheduler finds 1 candle before backfill fetches 200+.
- `engine/src/config.rs:301` (test) — default `feature_window_size = 72`.
- `deploy/docker-compose.yml:100` — confirms `FEATURE_WINDOW_SIZE: ${FEATURE_WINDOW_SIZE:-72}`.
- Model (`inference/model.py:88-112`) uses 6-layer causal CNN with dilations 1+2+4+8+16+32=63. With seq_len=1, zero temporal context → near-zero predictions.

### Bug 3: Data pipeline not receiving new confirmed candles
- Dashboard shows candle_ts `2026-07-14 17:00Z` but current time is `2026-07-15 14:27Z` — 21h gap, no new confirmed hourly candles.
- `engine/src/data/ws.rs:141-143` — only persists candles with `confirm: true`. If Kraken WS stops sending confirmed candles, DB goes stale silently.
- No gap-detection or catchup mechanism beyond reconnect backfill (`engine/src/data/mod.rs:67-71`).

### Bug 4: Dashboard shows one prediction row
- `engine/src/db.rs:21` — `candle_ts INTEGER NOT NULL UNIQUE` + `:171` `INSERT OR REPLACE INTO predictions` → every 30s retry overwrites the same row for the same stale candle.
- `engine/src/api.rs:167` — fetches 48 predictions `ORDER BY candle_ts DESC` but only 1 exists.
- `frontend/views/predictions.js:44` — shows `history.slice(-10).reverse()` → with 1 row, shows 1 row.

### Missing: Accuracy tracking
- `engine/src/db.rs:19-27` — predictions table has pred_1h/4h/24h but NO actual_* columns.
- No background task to compute actual returns.
- No API endpoint for accuracy metrics.
- No frontend display of accuracy.

## Decisions (with rationale)
1. **Set `last_processed_ts` right after `db::insert_prediction` (scheduler.rs:104)** — not at the end. The prediction is the valuable output; strategy/execution is best-effort. If execution fails, the candle is still "processed" for prediction purposes.
2. **Wait for REST backfill before spawning scheduler** — move `tokio::spawn(scheduler)` AFTER `data::run`'s backfill step completes (split `data::run` into `backfill` + `ws_loop` or add a `ready` signal).
3. **Guard: skip inference if `candles.len() < feature_window_size + 1`** — log WARN, don't call ZMQ, don't insert prediction. This prevents seq_len=1 and saves inference cost.
4. **Add `actual_1h`, `actual_4h`, `actual_24h REAL` columns to predictions** (nullable, NULL until horizon elapses) — ALTER TABLE migration in DDL (IF NOT EXISTS via `ADD COLUMN`).
5. **Background actuals task** — every hour, find predictions where `actual_1h IS NULL AND candle_ts + 3600 <= latest_candle_ts`, compute `ln(close[T+1h] / close[T])` from candles table, UPDATE the row. Same for 4h (14400s) and 24h (86400s).
6. **Accuracy API endpoint** — `GET /api/accuracy` returns directional accuracy % (sign matched) and MAE for each horizon, over a configurable window (default last 100 resolved predictions).
7. **Dashboard** — add accuracy panel (directional %, MAE per horizon), predictions table with actuals column, staleness indicator ("last confirmed candle: N hours ago").

## Scope IN
- engine/src/scheduler.rs — fix retry bug, add insufficient-candle guard
- engine/src/main.rs — reorder backfill before scheduler spawn
- engine/src/data/mod.rs — split backfill from WS loop (or signal completion)
- engine/src/db.rs — ALTER TABLE predictions, add actuals computation function, accuracy query
- engine/src/api.rs — add /api/accuracy endpoint, include staleness in /api/status
- frontend/views/predictions.js — show actuals column, more rows
- frontend/index.html — add accuracy panel section
- frontend/views/status.js — show candle staleness
- Tests for all new logic

## Scope OUT (Must NOT have)
- Retraining the model or changing market threshold (out of scope — using existing model.pt)
- Changing the ZMQ protocol or inference service internals (inference is fine; the bug is in the engine)
- Changing the candle interval (stays 60-minute hourly)
- Live Kraken trading (stays paper mode)
- Removing the parity gate

## Open questions
(None — all forks resolved by user decisions.)

## Metis gap analysis (folded in)
- **CRITICAL: INSERT OR REPLACE destroys actuals** — Changed to `ON CONFLICT DO UPDATE` in Todo 3. Preserves row id and actual_* columns.
- **CRITICAL: ALTER TABLE IF NOT EXISTS not supported** — Changed to PRAGMA table_info check in Todo 3. Idempotent migration.
- **MEDIUM: data::run() split** — Committed to Option 1 (split into backfill + run_ws_and_retention) in Todo 2.
- **MEDIUM: Frontend wiring** — Added frontend/app.js, frontend/api.js, frontend/views/accuracy.js to scope. Todo 11 specifies all wiring.
- **MEDIUM: Actuals task timing** — Specified :05 past each hour to avoid contention with scheduler at :00. Todo 7.

## Approval gate
status: approved
<!-- When exploration is exhausted and unknowns are answered, set status: awaiting-approval. -->
<!-- That durable record is the loop guard: on a later turn read it and resume at the gate instead of re-running exploration. -->
