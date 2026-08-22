# Hyperopt AutoPilot ↔ nightly runner promotion path

Captured 2026-08-22 (audit of the Strategy Auto-Pilot UI, options section).
Extends `phase7-options-architecture.md` (D13 gate mechanics) with the
candidate data path and the promotion-stage wiring. Verify interactively:
see `marketmoves-ops` `references/reach-engine-and-db-directly.md`.

## API contract (engine `api/hyperopt.rs`)

All `/api/hyperopt/*` routes are **behind Caddy basic-auth** like the rest of the
dashboard, but the engine router itself has NO auth middleware on them (only
CORS) — a raw `401` from `:9080` is Caddy, not the engine.

- `GET /api/hyperopt/{eq}/candidates` → `{equity, candidates:[CandidateResponse]}`
- `GET /api/hyperopt/{eq}/status` → `{equity, pipeline_state, total_candidates, by_status}`
- `GET /api/hyperopt/runs?limit=N` → `{runs, count}`
- `POST /api/hyperopt/{eq}/promote/{id}` body `{target_status}`

**CandidateResponse fields** (frontend `AutoPilot.svelte` reads these exact
names): `id, equity, strategy, status, mean_ic, std_ic, n_trades, params,
created_at`. `mean_ic`/`std_ic`/`n_trades` are read back from the candidate's
`promotion_metadata_json` (the nightly runner stores them there).

**Status strings** (`CandidateStatus::as_str()`): `NEW, STABLE, UNSTABLE,
PAPER, MICRO, LIVE, RETIRED`. The DB schema's `strategy_versions.status`
**default is `'CANDIDATE'`**, which `CandidateStatus::from_str` does NOT
recognize (latent — the store always writes an explicit status, so it's only
hit if a row is created with the default).

## Pitfall: `target_status` must be `PAPER | MICRO | LIVE`

The promote handler validates the body against exactly `PAPER/MICRO/LIVE`
(`hyperopt.rs::promote_candidate`). Anything else → `"{target_status} must be
one of PAPER, MICRO, LIVE"`, success=false.

**The frontend shipped `body: { target_status: 'AUTO' }` (api.js), so EVERY
promote click failed.** Do not use `CANDIDATE`, `STABLE`, or `AUTO` here. Even
with a valid target, `PromotionPipeline::promote` derives the next stage from
the candidate's CURRENT status, so the target should be the immediate next
hop (`NEW→PAPER`, `PAPER→MICRO`, `MICRO→LIVE`) — pass `nextStage(c.status)`,
don't let the client skip stages.

## Gate reality: IC + trade count are the live gate (sharpe/days disabled)

`PromotionPipeline` gates per transition on `min_trades, min_ic, min_sharpe,
min_days`. **`min_sharpe` and `min_days` are now `0.0` (disabled)** because the
pipeline has no per-candidate backtest (Sharpe) or age-observation source —
the hyperopt runner emits only walk-forward rank IC + trade counts, and the
promote handler builds `evidence { sharpe: 0.0, days_observed: 0 }`. The live
gates are the real metrics: `min_trades/min_ic` = `100/0.03 → 30/0.03 →
50/0.04` (candidate→paper → paper→micro → micro→live).

The request-time dry-run (`PromotionPipeline::check_snapshot`) and the boundary
apply (`PromotionPipeline::promote`) use the **same** queue-time evidence, so
request and apply gates always agree. Re-enable sharpe/days when a real
backtest + observation tracker land — the gate structure already enforces them.

## Apply path (correction to Phase 7)

Phase 7 said the boundary applier runs inside `OptionsScheduler::process_equity` —
**but OptionsScheduler is NOT spawned in main.rs** (only per-model
EquitySchedulers + the nightly hyperopt runner are). So promotions queued via
the API would sit in `pending_promotions` forever.

Fix (2026-08-22): a dedicated **promotion-applier task** in `main.rs` —
polls every 120s, tracks the last-seen daily candle ts per equity, and calls
`hyperopt::promotion::apply_pending_promotions(pool, equity, mode, &pipeline,
&store)` only when a NEW candle boundary appears. It iterates
`RunnerConfig::default().equities` (the same set the nightly runner stores
candidates for). This applies promotions WITHOUT enabling the OptionsScheduler
entry pipeline (which would emit mocked entry trades).

`apply_pending_promotions` re-checks the mid-exit gate (OPEN option position
for that equity → stays queued), uses queue-time evidence, marks the row
`PROMOTED to X` / `DENIED: reason`, and publishes a `strategy`-category event.

## Scope divergence: runner = QQQ only, UI = QQQ/SMH/XLF

`RunnerConfig::default().equities = ["QQQ"]` and `OptionsSchedulerConfig`
defaults to `["QQQ"]`, but `AutoPilot.svelte` renders QQQ/SMH/XLF tabs. SMH/XLF
will show "no candidates" until the runner config widens — not a bug, a scope
default. `pipeline_state` in the status response is hardcoded `"idle"` (mock),
not yet wired to a real scheduler state.

## State before first real run

`strategy_versions`, `pending_promotions`, and `hyperopt_runs` were all
empty (0 rows) as of 2026-08-22 — the nightly runner wasn't deployed, so the
AutoPilot page honestly reflects "no candidates yet." Equity-candle inputs
(`equity_candles` for QQQ/SMH/XLF, 1255 bars) were populated and current.