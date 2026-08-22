# Phase 7 Options Architecture (config store, D13 promotion gate)

Captured 2026-08-19. Commits: schema `5a8a0ab`, config store + externalization `0bd5ece`.

## Config store (7.0b)

- `engine/src/options/config_store.rs` — DB-backed registry over the `options_config` table. Every key carries tier, kind (F64/I64), default/min/max/label/description. ~37 keys covering sizing, chain selection, trailing stop, circuit breaker, staged-ladder timers, macro gate, overrides.
- Tiers: **Rail** = risk guard; bounded and changeable via UI but never disabled. **Strategy** = optimizable.
- Exit-side modules take config structs; keep BOTH constructors (`::new()` = legacy defaults, `::with_config()` = store-driven):
  - `HardcodedOverrides::with_config(OverridesConfig)` — dte_exit_min, delta_drift_min/max, earnings_blackout_days
  - `TrailingStop::with_config(price, atr, is_call, TrailingStopConfig)` — trail_pct, rearm_band_atr
  - `CircuitBreaker::with_iv_multiplier(halt_secs, max_losses, iv_spike_multiplier)`
  - `StagedLadder::with_timers(k, max_slippage, StagedLadderConfig)` — stage1/2/3_secs, tick_size
- `OptionsScheduler` rebuilds MacroGate/ChainSelector/PositionSizer configs from the store on **every pipeline run** (`configs_from_store(store, boot_cfg)`) so Settings-page edits go live within one polling cycle without restart.
- Fallback chain: store → boot scheduler config → registry default. When adding a config field to a module, wire it through this chain or UI edits silently won't apply.
- SKIPPED_ENTRY: every early return in `run_entry_pipeline` records an `engine_events` row via `skipped_entry()` (category=strategy, source=options::entry, message `SKIPPED_ENTRY {equity}: {reason}`).

## D13 promotion gate (7.0d)

- Promotions NEVER flip `strategy_versions.status` directly from the API.
- `POST /api/hyperopt/:equity/promote/:id`:
  1. Reject if equity has OPEN option positions (mid-exit forbidden).
  2. Dry-run evidence gates via `PromotionPipeline::check_snapshot` (fails fast with reason instead of queueing forever).
  3. Queue into `pending_promotions` → response "Queued for next daily candle boundary".
- `apply_pending_promotions(pool, equity, pipeline, store)` runs at the daily candle boundary — hooked into `OptionsScheduler::process_equity` BEFORE the entry pipeline. It RE-CHECKS the mid-exit gate (position may have opened after queueing → stays queued), rebuilds evidence from the CURRENT snapshot, then promotes.
- Table `pending_promotions`: `UNIQUE(version_id, equity)` (UPSERT replaces prior pending request), `applied_at NULL` = pending, `applied_result` = `PROMOTED to X` | `DENIED: reason`.
- Helper split: `PromotionPipeline::stage_for_status(&CandidateStatus)` + `check_snapshot(&CandidateSnapshot, &PromotionEvidence)` are pure (no DB writes); `promote()` is the only DB-writing path.

## Test pitfalls

- OptionsScheduler test pool must CREATE `engine_events` table, otherwise `insert_event` silently fails and SKIPPED_ENTRY assertions fail. Test asserts via a cloned pool (`check_pool`) because the original moves into the scheduler.
- `config::tests` failures are PRE-EXISTING noise: `Config::from_env()` calls `dotenvy::dotenv()` which reads the repo's real `.env` (SMA_WINDOW=40, ENABLE_SHORTING=true, TRADING_MODE, thresholds). Failing tests: `defaults_load_when_env_unset`, `live_mode_falls_back_to_paper`, `shorting_default_is_disabled`. `env -u VARNAME cargo test` does NOT help (dotenvy re-reads inside the test process). Don't chase these; verify the failing set is unchanged vs baseline instead.
- `cargo test --lib a b` rejects multiple filter args — use a single shared prefix (e.g. `options::`) or run filters separately.

## Tooling note

- `patch` fuzzy matching fails on old_string blocks spanning ~40+ lines even with exact text; split large rewrites into 10–20 line sequential chunks.
