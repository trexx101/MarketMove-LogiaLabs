# mmn-prediction-fix - Work Plan

## TL;DR (For humans)
<!-- Fill this LAST, after the detailed plan below is written, so it summarizes the REAL plan. -->
<!-- Plain English for a non-engineer: NO file paths, NO todo numbers, NO wave/agent/tool names. -->

**What you'll get:** A trading system that predicts once per hour (not every 30 seconds), produces meaningful predictions (not near-zero), shows a growing history of predictions on the dashboard, and tracks how accurate those predictions have been over time — with a staleness warning when the market data feed goes silent.

**Why this approach:** The 30-second prediction loop is caused by the scheduler not remembering which candle it already processed when a downstream step fails — fixed by marking the candle as "done" as soon as the prediction is saved, before attempting any trade execution. The near-zero predictions are caused by the model receiving only 1 data point instead of 72 — fixed by ensuring the historical data backfill completes before the prediction loop starts. A critical secondary fix: `INSERT OR REPLACE` on the predictions table would destroy computed accuracy actuals on retry — changed to `ON CONFLICT DO UPDATE` to preserve existing actual_* values. Accuracy tracking is added as a background computation that fills in "what actually happened" for each past prediction, exposed through a new API endpoint and dashboard panel.

**What it will NOT do:** It will not retrain the model, change trading thresholds, modify the inference service, change the hourly candle interval, or touch live Kraken trading. The parity gate stays in place.

**Effort:** Medium
**Risk:** Low — all changes are additive or surgical fixes to existing code; no architectural rewrites
**Decisions to sanity-check:** (1) Setting `last_processed_ts` after prediction save means strategy/execution failures are silently logged but don't retry — this is intentional since the prediction is the valuable output. (2) The ALTER TABLE migration adds nullable columns — existing data is unaffected. (3) Accuracy metrics use directional % + MAE — both are standard for trading model evaluation.

Your next move: approve this plan, then run `$start-work` to begin implementation. Full execution detail follows below.

---

> TL;DR (machine): Medium effort, Low risk. 12 todos across 4 waves fixing scheduler retry bug, backfill ordering, seq_len=1 guard, predictions schema migration, accuracy tracking (background task + API), and dashboard improvements (actuals, accuracy panel, staleness indicator).

## Scope
### Must have
- Fix scheduler 30-second retry loop (set `last_processed_ts` after prediction persistence)
- Wait for REST backfill before spawning scheduler (eliminate seq_len=1 race)
- Guard against insufficient candles (skip inference if `candles.len() < feature_window_size + 1`)
- Migrate predictions table: add `actual_1h/4h/24h` columns via PRAGMA-based migration
- Fix `INSERT OR REPLACE` → `ON CONFLICT DO UPDATE` to preserve computed actuals
- Background task to compute actuals from candle data (hourly, at :05 past each hour)
- `GET /api/accuracy` endpoint (directional %, MAE per horizon)
- Dashboard: accuracy panel, prediction history with actuals, staleness indicator
- All new logic has tests

### Must NOT have (guardrails, anti-slop, scope boundaries)
- No model retraining or threshold changes
- No changes to inference service internals or ZMQ protocol
- No changes to candle interval (stays 60-minute hourly)
- No live Kraken trading changes or parity gate removal
- No new external dependencies beyond what's already in Cargo.toml / pyproject.toml
- No `as any` / `@ts-ignore` / type suppression in frontend JS

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Test decision: tests-after (existing test infrastructure: `cargo test` for Rust, `cargo test --release` for parity harness, manual `curl` for API endpoints, browser check for dashboard)
- Evidence: `.omo/evidence/` directory

## Execution strategy
### Parallel execution waves
> Target 5-8 todos per wave. Fewer than 3 (except the final) means you under-split.

**Wave 1** (3 todos, parallel): Scheduler fix, backfill ordering, schema migration
**Wave 2** (3 todos, parallel): Candle guard, staleness detection, DB actuals functions
**Wave 3** (3 todos, parallel): Actuals background task, accuracy API, staleness in status
**Wave 4** (3 todos, parallel): Dashboard predictions view, accuracy panel, staleness indicator

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| 1. Scheduler fix | — | 3 | 2, 5 |
| 2. Backfill ordering | — | — | 1, 5 |
| 3. Candle guard | 1 | — | 4, 6 |
| 4. Staleness detection | — | 9 | 3, 6 |
| 5. Schema migration | — | 6, 7 | 1, 2 |
| 6. DB actuals functions | 5 | 7, 8 | 3, 4 |
| 7. Actuals background task | 5, 6 | — | 8, 9 |
| 8. Accuracy API | 6 | 10, 11 | 7, 9 |
| 9. Staleness in status | 4 | 12 | 7, 8 |
| 10. Predictions view | 8 | — | 11, 12 |
| 11. Accuracy panel | 8 | — | 10, 12 |
| 12. Staleness indicator | 9 | — | 10, 11 |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->

### Wave 1: Core bug fixes + schema migration

- [ ] 1. Fix scheduler retry bug — set `last_processed_ts` after prediction persistence
  What to do / Must NOT do: In `engine/src/scheduler.rs`, move `self.last_processed_ts = Some(candle_ts)` from line 188 to immediately after `db::insert_prediction` succeeds (after line 104). Make strategy evaluation and execution best-effort: wrap lines 116-186 in a block that logs errors but does NOT propagate them via `?`. The prediction is the valuable output; strategy/execution failures should not prevent the candle from being marked as processed. MUST NOT change the inference call, feature computation, or normalization logic.
  Parallelization: Wave 1 | Blocked by: — | Blocks: 3
  References (executor has NO interview context - be exhaustive): `engine/src/scheduler.rs:44-192` (full `run()` and `process()` methods), `engine/src/db.rs:161-184` (`insert_prediction`), `engine/src/strategy.rs` (strategy evaluation), `engine/src/exec/mod.rs` (executor trait)
  Acceptance criteria (agent-executable): `cargo test --lib scheduler` passes; new test verifies that if `db::insert_position_event` returns Err, `last_processed_ts` is still set and the next poll does NOT re-process the same candle.
  QA scenarios (name the exact tool + invocation): happy: `cargo test --lib scheduler::tests::process_sets_last_processed_after_prediction` — passes. failure: `cargo test --lib scheduler::tests::process_does_not_retry_on_strategy_failure` — passes.
  Commit: Y | fix(scheduler): set last_processed_ts after prediction persistence to stop 30s retry loop

- [ ] 2. Fix backfill ordering — split data::run() and wait for backfill before scheduler
  What to do / Must NOT do: In `engine/src/data/mod.rs`, split the current `run()` function into two public functions: (a) `pub async fn backfill(pool: DbPool, symbol: &str, min_candles: usize) -> Result<()>` — runs the REST backfill and returns, (b) `pub async fn run_ws_and_retention(pool: DbPool, symbol: &str) -> Result<()>` — spawns the retention task and runs the WS loop (never returns under normal operation). In `engine/src/main.rs`, restructure startup: (1) call `data::backfill()` synchronously, (2) spawn the scheduler, (3) spawn the Axum server, (4) call `data::run_ws_and_retention()` which blocks. This ensures the scheduler never starts until 200+ candles are in the DB. MUST NOT change the backfill logic itself, the WS ingestion logic, or the retention pruning logic.
  Parallelization: Wave 1 | Blocked by: — | Blocks: —
  References (executor has NO interview context - be exhaustive): `engine/src/main.rs:91-134` (current startup sequence — restructure this), `engine/src/data/mod.rs:1-41` (current `run()` function — split into two), `engine/src/data/rest.rs` (backfill function — no changes), `engine/src/data/ws.rs:49-73` (WS loop — no changes)
  Acceptance criteria (agent-executable): `cargo build` succeeds; new integration test or manual verification confirms that when the engine starts, the scheduler does not call inference until after the REST backfill log line "REST backfill complete" appears. `data::backfill` and `data::run_ws_and_retention` are both public and callable from main.
  QA scenarios (name the exact tool + invocation): happy: `cargo build` — exit 0. failure: verify with `docker compose logs engine | head -20` that "REST backfill complete" appears before any "prediction persisted" log line.
  Commit: Y | fix(engine): wait for REST backfill before spawning scheduler to prevent seq_len=1

- [ ] 3. Migrate predictions schema: add actual columns + fix INSERT OR REPLACE
  What to do / Must NOT do: In `engine/src/db.rs`, make TWO changes: (A) Add three new nullable REAL columns to the predictions table. Since SQLite does NOT support `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`, add a `migrate_predictions` function that: (1) queries `PRAGMA table_info(predictions)` to get existing column names, (2) for each of `actual_1h`, `actual_4h`, `actual_24h`, if the column name is not in the pragma result, runs `ALTER TABLE predictions ADD COLUMN <name> REAL`. Call this function from `open()` after the DDL execution. (B) Change `insert_prediction` from `INSERT OR REPLACE INTO predictions` to `INSERT INTO predictions (candle_ts, pred_1h, pred_4h, pred_24h, features_json, created_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(candle_ts) DO UPDATE SET pred_1h = excluded.pred_1h, pred_4h = excluded.pred_4h, pred_24h = excluded.pred_24h, features_json = excluded.features_json, created_at = excluded.created_at`. This preserves the row `id` and any `actual_*` columns that were already computed. Update the `PredictionRow` struct to include `actual_1h: Option<f64>`, `actual_4h: Option<f64>`, `actual_24h: Option<f64>`. Update `fetch_recent_predictions` to SELECT these new columns. MUST NOT change any other DDL statements.
  Parallelization: Wave 1 | Blocked by: — | Blocks: 6, 7
  References (executor has NO interview context - be exhaustive): `engine/src/db.rs:8-56` (DDL), `engine/src/db.rs:19-27` (predictions table), `engine/src/db.rs:72-98` (open function — add migrate call here), `engine/src/db.rs:161-184` (insert_prediction — change SQL here), `engine/src/db.rs:302-354` (PredictionRow struct + fetch_recent_predictions), `engine/src/api.rs:76-82` (PredictionDto)
  Acceptance criteria (agent-executable): `cargo test --lib db` passes; new tests: (a) `migrate_predictions` adds columns on fresh DB, (b) `migrate_predictions` is idempotent on DB that already has columns, (c) `insert_prediction` with ON CONFLICT preserves existing actual_* values when re-inserting same candle_ts.
  QA scenarios (name the exact tool + invocation): happy: `cargo test --lib db::tests::migrate_predictions_adds_columns` — passes. `cargo test --lib db::tests::insert_prediction_preserves_actuals_on_conflict` — passes. failure: `cargo test` (full suite) — no regressions.
  Commit: Y | feat(db): add actual_1h/4h/24h columns + fix INSERT OR REPLACE to preserve actuals

### Wave 2: Guards and monitoring

- [ ] 4. Add insufficient-candle guard in scheduler process()
  What to do / Must NOT do: In `engine/src/scheduler.rs`, at the beginning of `process()` (after fetching candles, before computing features), add a guard: if `candles.len() < self.feature_window_size + 1`, log a WARN with the candle count and return Ok(()) without calling inference or inserting a prediction. This prevents seq_len=1 predictions even if the backfill ordering fix (Todo 2) somehow fails. MUST NOT change the feature computation or normalization logic.
  Parallelization: Wave 2 | Blocked by: 1 | Blocks: —
  References (executor has NO interview context - be exhaustive): `engine/src/scheduler.rs:65-87` (process() candle fetch and feature window), `engine/src/config.rs:301` (feature_window_size default = 72)
  Acceptance criteria (agent-executable): `cargo test --lib scheduler` passes; new test verifies that with fewer than `feature_window_size + 1` candles, `process()` returns Ok(()) without calling the ZMQ bridge (mock bridge or check that no prediction was inserted).
  QA scenarios (name the exact tool + invocation): happy: `cargo test --lib scheduler::tests::process_skips_when_insufficient_candles` — passes. failure: verify with `docker compose logs engine` that WARN "insufficient candles" appears on startup before backfill completes.
  Commit: Y | fix(scheduler): skip inference when insufficient candles to prevent seq_len=1

- [ ] 5. Add candle staleness detection in data pipeline
  What to do / Must NOT do: In `engine/src/data/mod.rs` or `engine/src/data/ws.rs`, add a periodic check (every 5 minutes) that compares the latest candle timestamp against the current time. If the gap exceeds 2 hours (7200 seconds), log a WARN: "candle staleness: no confirmed candle in {gap_secs}s". Expose this staleness value via a shared state or DB query so the API can report it. Add a `staleness_secs` field to the status API response. MUST NOT change the WS ingestion logic or the candle confirmation filter.
  Parallelization: Wave 2 | Blocked by: — | Blocks: 9
  References (executor has NO interview context - be exhaustive): `engine/src/data/mod.rs:1-41` (data pipeline), `engine/src/data/ws.rs:49-160` (WS loop + handle_text), `engine/src/db.rs:149-158` (latest_ts), `engine/src/api.rs:111-164` (handle_status)
  Acceptance criteria (agent-executable): `cargo test --lib data` passes; new test verifies that the staleness check logs a WARN when `latest_ts` is > 7200s behind current time. The status API response includes a `staleness_secs` field.
  QA scenarios (name the exact tool + invocation): happy: `cargo test --lib data::tests::staleness_warns_after_2h` — passes. failure: `curl -s http://localhost:8080/api/status | jq .staleness_secs` — returns a number.
  Commit: Y | feat(data): add candle staleness detection and expose in status API

- [ ] 6. Add DB functions for actuals computation and accuracy queries
  What to do / Must NOT do: In `engine/src/db.rs`, add three new async functions: (a) `compute_actuals(pool) -> Result<u64>` — finds predictions where `actual_1h IS NULL` and a candle exists at `candle_ts + 3600`, computes `ln(close[candle_ts + 3600] / close[candle_ts])`, and UPDATEs the `actual_1h` column. Same logic for `actual_4h` (candle_ts + 14400) and `actual_24h` (candle_ts + 86400). Returns the number of rows updated. (b) `fetch_accuracy(pool, limit: usize) -> Result<AccuracyStats>` — queries predictions where actuals are non-NULL, computes directional accuracy (sign match %) and MAE for each horizon over the last `limit` resolved predictions. Returns an `AccuracyStats` struct. (c) `AccuracyStats` struct with fields: `directional_1h: f64`, `directional_4h: f64`, `directional_24h: f64`, `mae_1h: f64`, `mae_4h: f64`, `mae_24h: f64`, `resolved_count: usize`. MUST NOT change existing DB functions.
  Parallelization: Wave 2 | Blocked by: 5 | Blocks: 7, 8
  References (executor has NO interview context - be exhaustive): `engine/src/db.rs:1-424` (full DB module), `engine/src/db.rs:19-27` (predictions DDL with new actual columns from Todo 3), `engine/src/db.rs:186-200` (fetch_recent_candles for candle lookup pattern), `engine/src/features.rs:46-56` (log_return computation pattern)
  Acceptance criteria (agent-executable): `cargo test --lib db` passes; new tests: (a) `compute_actuals` correctly fills actual_1h for a prediction where a candle exists at T+3600, (b) `fetch_accuracy` returns correct directional % and MAE for a known set of predictions+candles.
  QA scenarios (name the exact tool + invocation): happy: `cargo test --lib db::tests::compute_actuals_fills_null_columns` — passes. `cargo test --lib db::tests::fetch_accuracy_computes_directional_and_mae` — passes. failure: `cargo test --lib db::tests::compute_actuals_skips_unresolved_horizons` — passes (actuals stay NULL when no future candle exists).
  Commit: Y | feat(db): add compute_actuals and fetch_accuracy functions for prediction tracking

### Wave 3: Background task + API endpoints

- [ ] 7. Add background task to compute actuals hourly
  What to do / Must NOT do: In `engine/src/main.rs`, spawn a background task (similar to the retention task in `data/mod.rs:26-37`) that runs every hour (offset by 5 minutes from the hour to avoid contention with the scheduler which processes at :00) and calls `db::compute_actuals(&pool)`. Log the number of rows updated. Use the same `DbPool` as the rest of the engine. Wrap the UPDATE batch in a transaction for atomicity. Handle errors gracefully: log with `tracing::error!` and continue (don't crash the engine). This task should start after the scheduler is spawned. MUST NOT change the scheduler or data pipeline logic.
  Parallelization: Wave 3 | Blocked by: 5, 6 | Blocks: —
  References (executor has NO interview context - be exhaustive): `engine/src/main.rs:91-134` (startup sequence, where to add the spawn), `engine/src/data/mod.rs:26-37` (retention task pattern to follow), `engine/src/db.rs:84` (pool max_connections = 4 — shared pool), `engine/src/db.rs` (compute_actuals function from Todo 6)
  Acceptance criteria (agent-executable): `cargo build` succeeds; manual verification with `docker compose logs engine` shows "actuals: updated N predictions" log line after the first :05 mark. The task uses the same DbPool and wraps updates in a transaction.
  QA scenarios (name the exact tool + invocation): happy: `cargo build` — exit 0. failure: verify the task doesn't crash the engine by running `docker compose up -d && sleep 5 && docker compose ps` — all services healthy.
  Commit: Y | feat(engine): add hourly background task to compute prediction actuals

- [ ] 8. Add GET /api/accuracy endpoint
  What to do / Must NOT do: In `engine/src/api.rs`, add a new handler `handle_accuracy` that calls `db::fetch_accuracy(&state.pool, 100)` and returns a JSON response with the `AccuracyStats` struct. Add a new route `/api/accuracy` to the router. Add a new response DTO `AccuracyResponse` that serializes the accuracy stats. The endpoint should return 503 if no resolved predictions exist yet. MUST NOT change existing API endpoints.
  Parallelization: Wave 3 | Blocked by: 6 | Blocks: 10, 11
  References (executor has NO interview context - be exhaustive): `engine/src/api.rs:1-243` (full API module), `engine/src/api.rs:25-48` (router setup), `engine/src/api.rs:50-98` (response DTOs), `engine/src/api.rs:166-178` (handle_predictions as pattern), `engine/src/db.rs` (fetch_accuracy from Todo 6, AccuracyStats struct)
  Acceptance criteria (agent-executable): `cargo test --lib api` passes; `curl -s http://localhost:8080/api/accuracy | jq` returns JSON with `directional_1h`, `directional_4h`, `directional_24h`, `mae_1h`, `mae_4h`, `mae_24h`, `resolved_count` fields.
  QA scenarios (name the exact tool + invocation): happy: `curl -s http://localhost:8080/api/accuracy | jq .directional_1h` — returns a number between 0 and 100. failure: `curl -s -o /dev/null -w '%{http_code}' http://localhost:8080/api/accuracy` — returns 503 when no resolved predictions exist.
  Commit: Y | feat(api): add GET /api/accuracy endpoint for prediction accuracy metrics

- [ ] 9. Add staleness indicator to /api/status
  What to do / Must NOT do: In `engine/src/api.rs`, update the `StatusResponse` struct to include a `staleness_secs: u64` field. In `handle_status`, compute the staleness by comparing the latest candle timestamp against the current time (using `chrono::Utc::now().timestamp()`). If no candle exists, set staleness to u64::MAX or a sentinel value. MUST NOT change any other status fields.
  Parallelization: Wave 3 | Blocked by: 4 | Blocks: 12
  References (executor has NO interview context - be exhaustive): `engine/src/api.rs:50-67` (StatusResponse struct), `engine/src/api.rs:111-164` (handle_status), `engine/src/db.rs:149-158` (latest_ts), `engine/src/db.rs:394-415` (fetch_latest_candle)
  Acceptance criteria (agent-executable): `cargo test --lib api` passes; `curl -s http://localhost:8080/api/status | jq .staleness_secs` returns a non-negative integer.
  QA scenarios (name the exact tool + invocation): happy: `curl -s http://localhost:8080/api/status | jq .staleness_secs` — returns a number. failure: if no candles exist, returns a large sentinel value (e.g., 999999).
  Commit: Y | feat(api): add staleness_secs to status response for dashboard staleness indicator

### Wave 4: Dashboard improvements

- [ ] 10. Update predictions view to show actuals column
  What to do / Must NOT do: In `frontend/views/predictions.js`, update the `renderHistory` function to add three new columns to the prediction history table: "Act 1H", "Act 4H", "Act 24H". These columns show the actual log-return values from the API response (which now includes `actual_1h`, `actual_4h`, `actual_24h` fields in the PredictionDto). If an actual is null, display "—" (pending). Color-code the actual values the same way as predictions (green for positive, red for negative). Update the `PredictionDto` usage to read the new fields. MUST NOT change the chart rendering or status view.
  Parallelization: Wave 4 | Blocked by: 8 | Blocks: —
  References (executor has NO interview context - be exhaustive): `frontend/views/predictions.js:1-75` (full predictions view), `frontend/views/predictions.js:36-60` (renderHistory function), `frontend/views/predictions.js:8-16` (fmtPred, predClass helpers), `engine/src/api.rs:76-82` (PredictionDto with new actual fields)
  Acceptance criteria (agent-executable): Manual browser check: open the control room, navigate to the predictions section, verify the history table shows 7 columns (candle_ts, 1H, 4H, 24H, Act 1H, Act 4H, Act 24H). Null actuals show "—".
  QA scenarios (name the exact tool + invocation): happy: open browser at `http://localhost:9080` (or configured HOST), check predictions table has 7 columns. failure: if no actuals exist yet, all Act columns show "—".
  Commit: Y | feat(frontend): show actual returns columns in prediction history table

- [ ] 11. Add accuracy panel to dashboard
  What to do / Must NOT do: (A) In `frontend/index.html`, add a new section `<div id="accuracy-body">` in the dashboard layout (after the predictions section). (B) Create a new file `frontend/views/accuracy.js` that exports a `render(rootEl, data)` function. The function displays: directional accuracy as percentages per horizon (e.g., "1H: 58% | 4H: 62% | 24H: 55%"), MAE per horizon (e.g., "1H: 0.0032 | 4H: 0.0089 | 24H: 0.0156"), and resolved prediction count. If data is null or resolved_count is 0, show "No resolved predictions yet". (C) In `frontend/api.js`, add a `fetchAccuracy()` function that calls `GET /api/accuracy` and returns parsed JSON. (D) In `frontend/app.js`, add the accuracy view to the polling loop: call `fetchAccuracy()` alongside `fetchPredictions()` and `fetchStatus()`, then call `accuracy.render(document.getElementById('accuracy-body'), accuracyData)`. Poll accuracy every 60 seconds (less frequent than status/predictions). MUST NOT change existing views or the chart rendering.
  Parallelization: Wave 4 | Blocked by: 8 | Blocks: —
  References (executor has NO interview context - be exhaustive): `frontend/index.html` (dashboard layout — add accuracy section), `frontend/app.js` (polling loop — add accuracy fetch + render), `frontend/api.js:1-25` (add fetchAccuracy here, follow fetchPredictions pattern), `frontend/views/predictions.js:1-75` (pattern for view module), `frontend/views/status.js:1-52` (pattern for simple data display), `engine/src/api.rs` (AccuracyResponse DTO from Todo 8)
  Acceptance criteria (agent-executable): Manual browser check: the accuracy panel shows directional % and MAE values. If no resolved predictions, shows "No resolved predictions yet". `frontend/api.js` exports `fetchAccuracy`. `frontend/app.js` calls it in the polling loop.
  QA scenarios (name the exact tool + invocation): happy: open browser, verify accuracy panel shows numbers. failure: with fresh DB (no actuals), panel shows fallback message.
  Commit: Y | feat(frontend): add accuracy panel with directional %, MAE, and resolved count

- [ ] 12. Add staleness indicator to status view
  What to do / Must NOT do: In `frontend/views/status.js`, add a new row to the status panel showing "Candle staleness" with the value from `data.staleness_secs`. Format as human-readable: "< 1m" for < 60s, "Xm" for < 3600s, "Xh Ym" for >= 3600s. If staleness > 7200s (2 hours), add a CSS class `val-neg` to highlight it as a warning. In `frontend/index.html`, add a `<span id="st-staleness">` element in the status panel. MUST NOT change other status fields.
  Parallelization: Wave 4 | Blocked by: 9 | Blocks: —
  References (executor has NO interview context - be exhaustive): `frontend/views/status.js:1-52` (full status view), `frontend/views/status.js:31-38` (setText helper), `frontend/index.html` (status panel HTML), `engine/src/api.rs:50-67` (StatusResponse with staleness_secs)
  Acceptance criteria (agent-executable): Manual browser check: the status panel shows "Candle staleness: Xm" or "Xh Ym". If > 2h, the value is highlighted in red/warning color.
  QA scenarios (name the exact tool + invocation): happy: open browser, verify staleness shows and is formatted correctly. failure: if staleness > 7200s, verify it's highlighted as a warning.
  Commit: Y | feat(frontend): add candle staleness indicator to status panel

## Final verification wave
> Runs in parallel after ALL todos. ALL must APPROVE. Surface results and wait for the user's explicit okay before declaring complete.
- [ ] F1. Plan compliance audit — verify all 12 todos are implemented, all acceptance criteria pass, no scope creep
- [ ] F2. Code quality review — `cargo clippy --all-targets -- -D warnings` passes, no `as any` in frontend JS, no `unwrap()` in new code
- [ ] F3. Real manual QA — `docker compose -f deploy/docker-compose.yml build && up -d`, verify: (a) engine logs show "REST backfill complete" before any prediction, (b) predictions occur hourly not every 30s, (c) `curl /api/status` shows staleness_secs, (d) `curl /api/accuracy` returns accuracy stats, (e) dashboard shows accuracy panel and staleness indicator
- [ ] F4. Scope fidelity — verify no changes to inference service, ZMQ protocol, candle interval, live trading, or parity gate

## Commit strategy
- Each todo produces one atomic commit (12 commits total)
- Commits are ordered by wave: Wave 1 (3 commits) → Wave 2 (3 commits) → Wave 3 (3 commits) → Wave 4 (3 commits)
- Each commit is independently buildable (`cargo build` passes after each)
- Final verification wave does not produce commits (read-only audit)

## Success criteria
1. **Scheduler fix verified**: `docker compose logs inference` shows predictions arriving at ~1-hour intervals, NOT every 30 seconds
2. **seq_len fixed**: Inference logs show `"seq_len":72` (or the configured feature_window_size), NOT `"seq_len":1`
3. **Predictions meaningful**: Dashboard shows prediction values in the range the model was trained for (not near-zero)
4. **Prediction history accumulates**: Dashboard shows multiple prediction rows (one per hourly candle), not just one
5. **Accuracy tracking works**: `GET /api/accuracy` returns directional % and MAE after 24+ hours of operation (enough for 24h horizon to resolve)
6. **Dashboard shows accuracy**: Accuracy panel displays directional %, MAE, and resolved count
7. **Staleness visible**: Status panel shows candle staleness; if WS stops receiving candles, the indicator turns red after 2 hours
8. **All tests pass**: `cargo test` (full suite) passes with no regressions
