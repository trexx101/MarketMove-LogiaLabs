---
name: marketmoves-equities-execution
description: Implement/verify MarketMoves equities shorting, PSQ remap, and live toggle.
---

# MarketMoves Equities Execution

## When to use
- Editing `engine/src/strategy.rs` (`EquityStrategyParams`, `next_equity_position`).
- Editing `engine/src/exec/paper.rs` / `exec/mod.rs` (PaperExecutor, FillResult, ExecutorKind).
- Editing `engine/src/exec/moomoo.rs` (MoomooExecutor via Python subprocess) or `engine/src/api/mode.rs` (runtime TOTP toggle).
- Wiring shorting / inverse-ETF (PSQ) execution, or the runtime paper↔live toggle with TOTP.
- **`POST /api/equity/backfill_predictions`** (2026-07-30): replays the inference pipeline
  over all historical candles, populating `equity_predictions` so backtests and accuracy
  metrics are meaningful. Always run this after a model change or fresh deploy. Route is
  `POST /api/equity/backfill_predictions`. AppState must have `zmq_endpoint` and
  `norm_stats_path` fields — adding fields to AppState requires updating the test fixture
  in `api/tests.rs` or you get E0063 at compile time.
- Integrating Moomoo OpenD (Futu) for live order execution.
- **Verifying the options data feed for the Options Momentum Engine** — probing option quota/OPRA tier, fetching an option chain, confirming greeks populate. See `references/options-opra-greeks-probe.md` (greeks are live-hours-only; `probe_option_quota.py` has two bugs in the protected `moomooapi` skill).
- Persisting or reading equity trades / PnL in `engine/src/db.rs`.
- Working from `.hermes/plans/control-room/PHASE_3_*.md`.
- Strategy parameter sweep / backtest-driven config tuning (`POST /api/backtest`).

## Core domain invariants (non-obvious — easy to get wrong)

### 1. PSQ inverse-ETF shorting: BUY to open, SELL to close — for BOTH long and short
A short is expressed by **buying PSQ** (ProShares Short QQQ, ~ -1x Nasdaq-100), not by short-selling QQQ. No borrow/locate. This means:
- **All opens are `TradeSide::Buy`** (buy QQQ for long, buy PSQ for short).
- **All closes are `TradeSide::Sell`** (sell QQQ to exit long, sell PSQ to exit short).
- The ONLY thing that distinguishes long from short is the **`symbol`** field (`QQQ` vs `PSQ`).
- ⚠️ The traditional "short = Sell to open" mapping is WRONG here and fails the PSQ tests. The executor's side arm is simply `Buy` for entry / `Sell` for exit, never branched on `Position::Short`.

### 2. Position-transition two-step is enforced by the STRATEGY, not the executor
`next_equity_position` must NEVER return `Short` directly from `Long`, nor `Long` directly from `Short`. A long first returns `Flat` (when pred_1d < exit_threshold), then a later tick may enter `Short`. The executor relies on this and produces the correct two fills:
- `Long → Short`: Sell QQQ, then Buy PSQ.
- `Short → Long`: Sell PSQ, then Buy QQQ.
When `enable_shorting=false` (default), the bearish regime must never yield `Short` (regression). A stray `Short` position with shorting off should flatten when `pred_1d > short_exit_threshold`.

### 3. Persist fills to `equity_trades` (symbol-aware), NOT legacy `trades`
Pre-existing bug: the executor wrote to the legacy `trades` table (no symbol column), but `status::handle_status` reads `sum_equity_realized_pnl` / `fetch_entry_trade_price` from the symbol-aware `equity_trades` table — so status PnL was always blind. The equities executor must call `db::insert_equity_trade(pool, symbol, ts, side, qty, price, fee, pnl)` (symbol = `QQQ` or `PSQ`). Add `fetch_recent_equity_trades(symbol, limit)` to read back per-symbol.

### 4. Config contract for shorting (engine/src/config.rs)
`from_env` reads: `ENABLE_SHORTING` (bool, default false), `SHORT_ENTRY_THRESHOLD` (f64, **must be < 0**, default -0.004), `SHORT_EXIT_THRESHOLD` (f64, must be > entry, default 0.001), `SHORT_SYMBOL` (default "PSQ"). `EquityStrategyParams` parses from JSON with `#[serde(default)]` + `#[serde(default = "fn")]` so backtest API calls that omit the new fields still deserialize. The `Default` impl must stay backward-compatible.

## Pitfalls
- **Don't branch the executor's `TradeSide` on `Position::Short`.** Both legs of a short are the same side direction as a long; only `symbol` differs. First implementation branched `Short => Sell` to open and the PSQ tests failed — fixed by making entry always `Buy`, exit always `Sell`.
- **`PaperExecutor::new` already takes 3 args in committed code** `(pool, fee_rate, tx: Option<TelemetrySender>)`. Integration tests under `engine/tests/` that call `PaperExecutor::new(pool, fee)` with 2 args were pre-existing compile failures (E0061). When you add a `new_for_symbol(pool, fee, primary, short, tx)` constructor, keep `new` as a thin delegate (QQQ/PSQ defaults) so old 3-arg call sites still compile.
- **Struct field reorder / duplicate breaks direct construction.** If you add fields to `Config` or `EquityStrategyParams`, every direct `Struct { .. }` literal (e.g. `api/tests.rs`, `scheduler.rs` test helper) must gain the new fields or you get E0063 (missing field). Prefer `..Default::default()` in tests to avoid this.
- **`TradeFill` telemetry variant and `FillResult` need the `symbol: String` field** once the executor attributes trades per-instrument; WS consumers and the serialization test must include it.
- **Write/read table mismatch** (point 3) silently breaks status PnL. Always pair a new `insert_*` with the `fetch_*` the API actually calls.

## Verification (condensed — full recipe in `code-change-verification`)
- Targeted: `cargo test -p engine --lib -- strategy exec api db` (substring filter on test path).
- New PSQ behavior: `cargo test -p engine --lib -- short` (covers strategy + executor + config shorting tests).
- Integration: `cargo test -p engine --test exec_parity --test paper_verification` (PnL hand-fixtures; must be neutralized for `.env` — see below).
- Pre-existing `config::tests` module is red on clean `main` too: `Config::from_env` calls `dotenvy::dotenv()` every invocation, which re-injects root `.env` (`SYMBOL=QQQ`) into the process during batch runs, poisoning `clear_engine_env`. Prove it's not yours via `git stash` diff of the failure set. **Move `.env` aside for the run only** (`mv .env .env.bak` … restore) — do NOT bake the workaround into the change.

## Phase 3.3 — Runtime Paper↔Live Toggle + Moomoo OpenD *(shipped)*

### Architecture: OpenD is a local gateway, not an HTTP API
Moomoo (Futu) does NOT expose a public REST API. It uses **OpenD** — a local daemon (default `127.0.0.1:11111`, protobuf-over-TCP) that bridges to Futu's servers. The official `moomoo` Python SDK wraps this connection. **There is no Rust crate.** The LiveExecutor must shell out to the Python scripts in `.agents/skills/moomooapi/scripts/trade/`.

The executor ships at **`engine/src/exec/moomoo.rs`** (NOT `live.rs` — the plan said `live.rs` but the existing data-layer module already used `data/moomoo.rs`, so the executor took the symmetric name `exec/moomoo.rs`). It is selected at startup via `LIVE_EXECUTOR=moomoo` in `main.rs`; when `LIVE_EXECUTOR` is unset or `paper`, the engine falls back to `PaperExecutor` even if `TRADING_MODE=live`.

Key OpenD facts:
- `TrdEnv.SIMULATE` = paper, `TrdEnv.REAL` = live. Real orders require the user to manually unlock the trade password in the OpenD GUI first.
- `place_order.py` returns exit code 2 (preview-only, NOT executed) when `--trd-env REAL` is set but `--confirmed` is not passed. The LiveExecutor must pass `--confirmed` to actually submit.
- Codes use market prefix: `US.QQQ`, `US.PSQ`, `HK.00700`, etc.
- Full contract: `references/moomoo-opend-integration.md`

### Startup wiring (Phase 3.3)
- `Config` gained `live_executor: String` (default `"paper"`, validated as `paper`|`moomoo`) and `moomoo_trd_env: String` (default `"SIMULATE"`, validated as `SIMULATE`|`REAL`).
- `main.rs::build_executor_for_mode` constructs `ExecutorKind::Moomoo(MoomooExecutor::new(symbol, short_symbol, trd_env))` when `LIVE_EXECUTOR=moomoo`; falls back to `PaperExecutor` otherwise.
- The executor is wrapped in `Arc<tokio::sync::RwLock<ExecutorKind>>` and passed to the scheduler (Phase 3.4 prerequisite).

### Phase 3.3 tests
- 8 unit tests in `exec::moomoo::tests`: long/short entry, long/short exit, no-change, long↔short two-step, `symbol_to_moomoo_code` (US-prefix), `TrdEnv::from_env` round-trip.

## Phase 3.4 — TOTP-Gated Runtime Mode Toggle *(shipped)*

### Runtime topology
- `AppState.trading_mode` is `Arc<tokio::sync::RwLock<TradingMode>>` (NOT `std::sync::RwLock` — handlers are async).
- `AppState` also carries `parity_marker_path`, `parity_max_age_secs`, `totp_secret` (base32 string).
- `EquityScheduler` holds `Arc<RwLock<TradingMode>>` and `Arc<RwLock<ExecutorKind>>`; it borrows both briefly at the start of each cycle and the `ModeChange` API can swap them between cycles without contention.
- The scheduler re-acquires `executor.write()` only for the duration of `set_target_position` — the toggle cannot land mid-order, but can land between orders.

### `engine/src/totp.rs` — TOTP helper
- `verify(secret_b32, code)` returns `Ok(true|false)` after validating shape (6 ASCII digits) and calling `TOTP::check(code, unix_secs)`. With `skew=1` at construction, the boundary ±30s window is automatic — do NOT re-implement drift manually.
- `generate_secret()` returns a base32-encoded 160-bit secret (no padding) suitable for `TOTP_SECRET`.
- `otpauth_url(secret, issuer, label)` produces `otpauth://totp/MarketMoves:control-room?secret=...&issuer=MarketMoves`.
- `load_or_generate_secret()` returns `(secret, was_generated: bool)`; if `was_generated`, the operator MUST persist `TOTP_SECRET=...` before the next restart or they will be locked out of live mode.
- Time source: `std::time::SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()` — `TOTP::check` takes `u64`, NOT `i64`.

### `engine/src/api/mode.rs` — endpoints
- `GET /api/mode` → `{ mode, parity_marker_age_secs, parity_valid, last_switch_ts }`. Reads from `state.trading_mode.read().await` (NOT `.to_string()` — the `Arc<RwLock>` doesn't implement `Display`).
- `POST /api/mode` body `{ "mode": "paper"|"live", "auth_token": "123456" }` → 5-step flow:
  1. Validate target mode string.
  2. TOTP check: `totp::verify(&state.totp_secret, &req.auth_token)`. **Returns 403 with plain-text `"TOTP invalid"` body** (NOT JSON — integration tests parse `.text()` not `.json()`).
  3. Parity re-check at request time (NOT just startup): `db::parity_marker_age_secs(&state.parity_marker_path)`; only enforced when `target == Live`. Returns 403 with `"parity marker is Xs old (max Ys)"`.
  4. Flip the shared `TradingMode` via `state.trading_mode.write().await`.
  5. Append `mode_switches` audit row + broadcast `TelemetryEvent::ModeChange` on the WS channel.
- **Asymmetric gating**: paper→live requires fresh parity; live→paper does NOT (always allowed). Encode this in the test: `post_mode_live_to_paper_does_not_require_parity` writes NO marker file and still succeeds.
- Visibility: `AppState`, `api::mode`, and the `handle_*` functions are `pub` so integration tests in `engine/tests/` can construct them with explicit wiring. The public `router()` constructor builds its own AppState, so integration tests build their own router with `axum::Router::new().route(...).with_state(state)`.

### `main.rs` startup behavior
- `load_or_generate_secret()` runs before `tracing_subscriber` is initialized (it's the first thing in `main`). If a fresh secret is minted, a `warn!` macro logs the otpauth URL so the operator can scan it. Persist before next restart.
- `Config` clones cheaply (added `Clone` derive) so `main.rs` can clone it, set the resolved `totp_secret`, and pass to `api::router(pool, &cfg, tx)`.
- If `TOTP_SECRET` was empty at startup, the cloned `cfg_for_api` carries the freshly generated secret — this is what `POST /api/mode` validates against.

### Phase 3.4 integration tests (`engine/tests/mode_toggle.rs`)
5 tests, all green:
1. `get_mode_returns_paper_by_default` — `GET /api/mode` returns `mode: "paper"`, `parity_valid: true`.
2. `post_mode_rejects_invalid_totp` — wrong code → 403, body contains `"TOTP"`.
3. `post_mode_rejects_stale_parity_marker` — fresh TOTP but 30-day-old marker → 403, body contains `"parity"` or `"old"`.
4. `post_mode_paper_to_live_with_valid_totp` — valid code + fresh marker → 200, body `mode: "live"`, `fetch_recent_mode_switches` returns 1 audit row.
5. `post_mode_live_to_paper_does_not_require_parity` — start with `Live` mode, no marker file, valid TOTP → 200 `mode: "paper"`. Demonstrates asymmetric gating.

### Frontend (`frontend/src/lib/components/StatusPanel.svelte`)
- The skill's plan called for a separate `ModeToggle.svelte` component; in practice the modal was embedded in `StatusPanel.svelte` (clicking the `⇄` button next to the mode badge). The modal calls `fetchMode()` on open to surface parity status, then `setMode(target, totpCode)` on submit. **Disable the confirm button when `target === "live" && !modeInfo.parity_valid`** so users can't fire a request that will be rejected.
- The WS `ModeChange` event updates the badge after a successful flip — no need to poll `/api/mode` post-submit.

### Visibility for tests
To call the handlers from `engine/tests/mode_toggle.rs`, these were changed from `pub(crate)` to `pub`:
- `engine::api::AppState` (struct + all fields)
- `engine::api::mode::handle_get_mode` / `handle_set_mode`
- `engine::api::mode::ModeResponse` / `SetModeRequest` / `SetModeResponse`
- The `mod mode;` declaration in `api/mod.rs` became `pub mod mode;`.
If you add a new handler and write an integration test, follow the same pattern.

### Pitfalls (Phase 3.3 + 3.4 — learned the hard way)
- **`place_order.py --json` exit code 2 is NOT an error** — it's the preview gate. The executor must pass `--confirmed` for REAL or parse exit 2 as "needs confirmation" (not as a hard failure).
- **`tokio::process::Command::output()` returns `Output { stdout: Vec<u8>, stderr: Vec<u8>, ... }`** — NOT `Option<Vec<u8>>` like `std::process::Output`. First-try code that mirrors std-style `if let Some(s) = output.stdout { s.read_to_string(...) }` will fail to compile. Use `String::from_utf8_lossy(&output.stdout).into_owned()` directly.
- **`totp-rs` feature flags gate method availability** — `Secret::generate_secret()` requires the `gen_secret` feature; `TOTP::new(algo, digits, skew, step, bytes, issuer, label)` with the 7-arg `issuer`/`label` form AND `get_url()` require the `otpauth` feature. Without `otpauth`, `new` is 5-arg and `get_url` doesn't exist. There's no `std` feature — base crate IS std-only.
- **`Secret::Encoded("...").to_bytes()` is the canonical decode path** — `to_encoded()` round-trips but is `Display`-based (`Raw` → hex, `Encoded` → base32 string). To get base32 out of a freshly generated `Secret::Raw`, call `to_bytes()` and re-encode yourself (or pull the `base32` crate transitively from `totp-rs`).
- **`TOTP::check` takes `u64` not `i64`** — convert from `chrono::Utc::now().timestamp()` via `SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()`.
- **Set `skew` on `TOTP::new` for the ±1-step boundary window** — don't do `check(t-30) || check(t) || check(t+30)` in user code. `TOTP::new(..., skew=1, ...)` makes `check(t)` already validate 3 windows. Cleaner and matches RFC 6238 examples.
- **`Arc<RwLock<TradingMode>>` does NOT implement `Display`** — handlers must `.read().await` to a `TradingMode` and call `.to_string()` on the copy, not on the `Arc`. Trying `state.trading_mode.to_string()` will fail with E0599.
- **Use `tokio::sync::RwLock`, not `std::sync::RwLock`** in async handlers — `std::sync::RwLock::read()` returns a guard that holds the lock across `.await` points, which is a deadlock hazard in async axum handlers. `tokio::sync::RwLock` is the async-friendly variant.
- **Direct construction of `Config` in tests** will break (E0063) when `totp_secret`, `parity_marker_path`, `parity_max_age_secs`, `live_executor`, `moomoo_trd_env` fields are added — list every field in test literals. `Config` does NOT derive `Default`. The integration test in `engine/tests/mode_toggle.rs::test_config` is the canonical exhaustive literal — copy its shape.
- **TOTP code validation is time-sensitive** — the `totp-rs` crate validates a ±30s window (with `skew=1`); tests must generate the code at `SystemTime::now()` via `totp::current_code(&secret)` not a fixed timestamp.
- **Frontend `StatusPanel.svelte` shows the mode badge as static text** — Phase 3.4 added the WS `ModeChange` event AND the embedded modal (clicking `⇄` button next to the badge). The original plan called for a separate `ModeToggle.svelte` component but we kept it inline in StatusPanel for cohesion. If you ever extract it, the API is `fetchMode()` (read) + `setMode(target, totpCode)` (write) in `frontend/src/lib/api.js`.
- **`POST /api/mode` errors return plain text, not JSON** — the handler returns `Err((StatusCode::FORBIDDEN, "TOTP invalid"))` which produces `text/plain` body. Tests must call `.text().await` not `.json().await` on 4xx responses. (The 200 success body IS JSON via `Json(SetModeResponse)`.)
- **The `mode_switches` table already exists in DDL** at `engine/src/db.rs` (~line 129) — no migration needed. Just add `db::insert_mode_switch` and `db::fetch_recent_mode_switches` helpers that bind to it.
- **`patch` tool can silently drop the `path` parameter`**, returning `error: path required` 5+ times in a row. Do NOT keep retrying identical args — that loops indefinitely. Workarounds in priority order: (1) use `write_file` to rewrite the whole file; (2) use `terminal sed -i 's/.../.../g' path` for single-line fixes; (3) re-issue once with the exact same JSON payload (sometimes the tool recovers). Do NOT narrate the loop; switch tools immediately. This is especially common in long sessions where the tool's internal state can drift — observe it and move on.
- **`pred_5d_filter: false` degrades win rate but improves CAGR** — on the SMA=40 config, true→false adds 1 long trade (+4% CAGR, +0.02 Sharpe) but drops win rate 83%→80%. The net is positive; validate on your specific threshold combo before assuming.
- **`#[serde(default = "fn")]` on new struct fields preserves back-compat** — when adding fields to `EquityStrategyParams` (or any serde-deserialized struct), using `#[serde(default = "fn")]` lets API callers omit the field and still deserialize. `Default` impl must also include the new field. If the new field has no sensible default, the serde approach is still cleaner than breaking every test site — add `#[serde(default)]` for bools that default to `false` or use a `default_` helper function.
- **`docker build` COPY fails silently if `.dockerignore` excludes source dir** — if `frontend/dist/` is missing in the image but exists on host, check `.dockerignore`. The negated pattern `!frontend/dist/` must appear AFTER any broader exclusion of `frontend/` itself. More reliably: use `COPY --from=frontend-build /build/frontend/ /app/frontend/` (copy the whole stage dir, not just `dist/`). The `frontend-build` stage can simply `COPY frontend/dist/ ./` if dist was pre-built on the host — no Node toolchain needed.
- **Named Docker volumes vs host bind mounts** — `-v deploy_models:/models:ro` mounts the named volume `deploy_models`; `-v /models:/models:ro` mounts the host path `/models` (which may not exist or be empty). If the container can't find `/models/norm_stats_qqq_v1.json`, check `docker volume inspect deploy_models` and ensure the volume is populated, or fix the `-v` flag to use the correct volume name. This was the cause of an "unhealthy" container after a rebuild — the wrong volume mount silently succeeded but provided an empty `/models`.
- **`touch engine/src/main.rs` forces cargo to detect source changes** — the Docker build uses a placeholder sentinel to warm the cargo cache; `touch engine/src/main.rs` in the same RUN layer as `cargo build` ensures the real source is compiled even if cargo's cache is reused. Omitting this step means the binary still contains the placeholder, not the new code.
- **Disk-full from `target/` is a silent build blocker** — incremental cache alone was 6.4 GB and release artifacts another 712 MB on this VM. When `cargo test` fails with `signal 7 [Bus error]` from `collect2` or `No space left on device`, check `df -h /home` and `du -sh target/debug/*` before debugging linker errors. `rm -rf target/debug/incremental target/release` reclaims space without losing source.

### MoomooExecutor pattern: separate the planner from the side-effects
`MoomooExecutor::plan_trades(target, current, primary, short) -> Vec<(String, TradeSide)>` is the pure-data fill matrix; `set_target_position` consumes it and shells out. The planner is what tests assert on (8 unit tests cover all 6 position transitions + no-change + long↔short two-step), and it lives next to the executor so any future change to the dispatch rules stays in one place. Keep this split for any future executor (Binance, IBKR) — it's the same shape.

## Phase 5 — Multi-Model Registry + Sentiment Risk Overlay *(2026-08-05 onward)*

Plan: `.hermes/plans/2026-08-05_nvda-multi-asset-and-sentiment-overlay.md`.
Branch convention: `feature/<plan-short-name>` cut from `main`, e.g.
`feature/nvda-multi-asset-and-sentiment-overlay`. Not committed until
explicit user go-ahead — see Workflow gate below.

### Architectural pivot (2026-08-05): **model** is the unit, not symbol

Earlier Phase 5 sketches (env-driven `Vec<SymbolConfig>`) were superseded
mid-session by a **DB-backed registry** after the user pointed out that the
strategy-layer signal is keyed on a *trained model*, which owns a
primary+inverse pair, a budget, and per-model thresholds — not on a bare
symbol. The registry is the runtime source of truth; `Config::symbol`
and `Config::short_symbol` remain as **bootstrap defaults** used when
the registry is empty (cold-start), so Wave A behavior survives.

The full recipe lives in `references/phase5-multi-asset-and-sentiment.md`
(DB-backed Recipes 1–8: registry table, API endpoints, bootstrap, telemetry
enrichment, store refactor). The summary here captures only the
non-obvious design choices.

### `trading_models` registry (db.rs)

```sql
CREATE TABLE IF NOT EXISTS trading_models (
    model_id        TEXT    PRIMARY KEY,        -- uuid, NOT a ticker
    primary_symbol  TEXT    NOT NULL,           -- e.g. NVDA
    inverse_symbol  TEXT    NOT NULL,           -- e.g. NVDD
    model_path      TEXT    NOT NULL,           -- path to model bundle
    norm_stats_path TEXT    NOT NULL,           -- path to norm stats json
    budget_usd      REAL    NOT NULL DEFAULT 5000.0,
    enabled         INTEGER NOT NULL DEFAULT 1,
    deployed_at     INTEGER NOT NULL,
    last_wf_ic      REAL,
    last_wf_at      INTEGER,
    notes           TEXT
);
```

`engine/src/db.rs` exposes `register_model`, `update_model_enabled`,
`load_model_by_id`, `load_all_models`, `load_enabled_models`,
`record_walk_forward_result`. `TradingModel::pair()` returns
`"NVDA/NVDD"`-style display labels. Five unit tests in `db::tests` cover
register-and-load, filter-by-enabled, toggle-unknown-id, and walk-forward
persistence — all green after the registry work landed.

### Bootstrap: cold-start falls back to Config defaults

When `trading_models` is empty at startup, `db::resolve_active_models()`
returns a synthetic row whose `model_id` is the literal string
`"bootstrap-default"` (NOT `bootstrap-<symbol>` — the resolver returns
a single `TradingModel` without writing to the DB, and uses the fixed
sentinel so it's easy to detect in code paths and WS telemetry). The
bootstrap row carries `Config::symbol`, `Config::short_symbol`,
`Config::norm_stats_path`, `budget_usd=0.0`, and `enabled=true`. A fresh
DB auto-deploys QQQ/PSQ paper trading on first start with no manual
registration required. The fixed model_id is what `main.rs` checks
with `active_models.first().map(|m| m.model_id.as_str()) ==
Some("bootstrap-default")` to emit the "trading_models registry is
empty — bootstrapping" log line.

### NVDD inverse-ETF: ZERO executor changes (unchanged from earlier Phase 5)

`MoomooExecutor::plan_trades(target, current, primary, short)` and
`PaperExecutor::new_for_symbol(pool, fee, primary, short, tx)` already
route `Position::Short` to `short_symbol`. The dispatch is **symbol-only**,
not branched on the underlying instrument. To add a new inverse pair
(NVDA → NVDD), only the registry needs a row — `exec/paper.rs` and
`exec/moomoo.rs` are untouched.

### Engine main.rs: one EquityScheduler task per enabled registry row

Cheapest pattern is **N scheduler tasks, each single-symbol**, sharing
global state via `Arc`. NOT a refactor of `EquityScheduler` to hold a
Vec<(String, NormStats, TCN+LGBM)> internally — that creates an
abstraction with one user and no second user to validate it against.

```rust
for model in db::load_enabled_models(&pool).await? {
    let norm_stats = EquityNormStats::load_named(&model.norm_stats_path)?;
    let sched_pool = pool.clone();
    let sched_zmq = cfg.zmq_endpoint.clone();
    let sched_window = cfg.feature_window_size;
    let sched_params = strategy_params.clone();
    let sched_trading_mode = trading_mode_arc.clone();
    let sched_executor = executor.clone();
    let sched_tx = tx.clone();
    let sched_event_logger = event_logger.clone();
    let model_id = model.model_id.clone();
    let primary = model.primary_symbol.clone();
    let inverse = model.inverse_symbol.clone();
    tokio::spawn(async move {
        let paper = PaperExecutor::new_for_symbol(
            sched_pool.clone(), 0.0015, &primary, &inverse, Some(sched_tx.clone()),
        );
        let executor_arc = Arc::new(RwLock::new(ExecutorKind::Paper(paper)));
        match scheduler::EquityScheduler::new(
            sched_pool, &primary, &sched_zmq, norm_stats,
            sched_window, sched_params, sched_trading_mode,
            executor_arc, Some(sched_tx), Some(sched_event_logger),
        ).await {
            Ok(mut sched) => {
                if let Err(e) = sched.run().await {
                    error!(model_id = %model_id, error = %e, "scheduler fatal");
                }
            }
            Err(e) => error!(model_id = %model_id, error = %e, "scheduler init"),
        }
    });
}
```

Key invariants:
- Each scheduler task owns its own `norm_stats`, `bridge` (ZMQ connection),
  `last_processed_ts`, and `PaperExecutor` instance. Independent.
- `strategy_params` is shared so a `/api/strategy-config` PUT (once it
  takes `model_id`) propagates per-model.
- ⚠️ **Executors are NOT shared across models.** Each model gets its own
  `PaperExecutor` because per-model `current_position` is held inside the
  executor. Sharing would cause one scheduler to read the wrong model's
  position. Moomoo executor is still shared because it has no per-instance
  position state (the broker is the source of truth).
- `TradingModel::pair()` is the WS telemetry attribution label — every
  `TelemetryEvent` variant carries `model_id` and `pair` (added in the
  registry follow-up commit, see `references/phase5-multi-asset-and-sentiment.md`
  Recipe 4 for the enrichment).

### Sentiment risk overlay (deferred to follow-up commit)

The sentiment overlay is a **separate commit** after the registry lands.
Per the locked 2026-08-05 plan: `enable_sentiment_overlay = false` by
default, `sentiment_min_articles = 15` (raised from 5 because the user
said "optional and lower weightage" — interpret as off-by-default +
higher min-articles, NOT a softer threshold). Thresholds stay at
`-0.5` / `-0.8`. Applied per-model in `scheduler.rs::evaluate_and_execute_strategy`
between `next_equity_position` and the executor call. The wire shape
and `apply_sentiment_overlay` skeleton live in
`references/phase5-multi-asset-and-sentiment.md` Recipe 5.

**Implementation deviation (locked 2026-08-05):** The plan §3C
originally proposed `apply_sentiment_overlay -> (Position, f64)` with a
`f64` size_multiplier to halve qty on `-0.5`. Inah approved a cleaner
interpretation that **avoids executor surgery**: overlay returns
`Position` only (no multiplier). The reduce rule doesn't halve qty
mid-hold; instead it returns `Flat` when the base state machine would
have entered Long/Short. Existing positions are unaffected on the
reduce rule (exits still fire normally via the base exit_threshold).
The exit rule (score < `-0.8`) still forces Flat regardless. Do NOT
add a `f64` multiplier return — the executor's `qty` is fixed at
construction time and threading a multiplier through every entry/exit
leg is invasive and was explicitly rejected.

### DDL comment pitfall: naive `;` splitter (caught 2026-08-05)

`engine/src/db.rs::open()` runs every new table by splitting the static
`DDL` string literal on `;` and executing each fragment. **The splitter
is naive about SQL `--` line comments.** A comment like:

```sql
-- JSON-pair examples: "QQQ/PSQ"; "NVDA/NVDD"
CREATE TABLE IF NOT EXISTS trading_models (...)
```

...silently breaks every test in `db::tests::*` (14 failures) because the
splitter severs mid-comment, leaving an unparseable fragment with
`near "..."` syntax errors that point at random test sites. The
existing DDL block pre-Phase-5 avoided the issue because no comment
contained a `;`.

**When adding tables to `DDL`:** keep `--` comments free of `;`
characters. Rephrase `e.g. NVDA/NVDD` as `e.g. NVDA-NVDD` if you have
to mention examples. Also keep `--`-column comments free of `;` for
column-level ones — same splitter applies. Symptom if you hit it:
*every* test in the suite fails with a fresh `near "..."` syntax error
and an in-memory test-pool that can't even apply the DDL.

This is a real fragility: future DDL edits should re-read `DDL` first
and grep for `;` inside `[^;"]*--` before saving.

### §8 steps 1-6 shipped (2026-08-05, commits c63db48–2b51851)

All 6 backend steps are committed and verified:

| Step | Commit | What |
|---|---|---|
| §8.1 | `c63db48` | `trading_models` DB table + CRUD + 5 tests |
| §8.2 | `d3621a3` | Per-model bootstrap loop in `main.rs` |
| §8.3 | `acdbf4f` | `model_id`+`pair` on 5 TelemetryEvent variants + `emit_for_model` helper |
| §8.4 | `49b0919` | Scheduler emit sites swapped to `emit_for_model` |
| §8.5 | `5ca339e` | `/api/models` GET/POST/PUT endpoints + 4 tests |
| §8.6 | `2b51851` | `/api/strategy-config?model_id=X` per-model routing + 4 tests |

193 lib tests green (+8 net new), 23 pre-existing config failures, no regressions.

### §8 steps 7-14 frontend shipped (2026-08-05, commit 4959e48)

Frontend refactored from flat single-symbol stores to per-model keyed
stores. See `references/phase5-multi-asset-and-sentiment.md` Recipe 7
for the full store partitioning pattern. `npm run build` clean (67
modules, 0 errors). 8/8 store isolation tests pass.

### Step 2B: NVDD + PSQ ingest (2026-08-05, commit 7cb0a89)

`EQUITY_SYMBOLS` in `engine/src/data/mod.rs` extended with `"PSQ"`
(was missing despite being the QQQ short leg) and `"NVDD"` (NVDA short
leg). Both are standard Yahoo Finance tickers — no fetcher changes needed.

### Step 2E: Model registration (2026-08-05, commit 57c542c)

`scripts/register_models.sql` inserts `qqq-v1` and `nvda-v1` into the
`trading_models` table. Engine boot verified: `count=2` models loaded
from registry, both schedulers bootstrapped with correct symbols and
norm_stats paths. Per-model strategy config PUT isolation verified via
live curl (NVDA threshold changed, QQQ untouched).

### Pitfall: engine CWD vs repo root for relative paths

The engine uses relative paths like `models/norm_stats_qqq_v1.json`.
These resolve correctly when run from the repo root
(`cargo run --bin engine --manifest-path engine/Cargo.toml`), but fail
with "No such file or directory" when run from inside `engine/`.
The production DB is at `engine/data/candles.db` (CWD-relative), NOT
the repo-level `data/candles.db`. When registering models via SQL or
debugging DB state, target `engine/data/candles.db`.

### Pitfall: `pub` vs `pub(crate)` for external verification harnesses

Ad-hoc verification crates under `/tmp/` that link `engine` as a path
dependency cannot access `pub(crate)` modules. Use `pub mod` for
modules that external harnesses need to call directly (e.g. `models`,
`strategy_config`). `TelemetryEvent` is crate-private and can't be
tested from outside — cover it with in-crate unit tests instead.

### Pitfall: AppState field addition blast radius

Adding a field to `AppState` means every construction site needs
updating. There are only 2 direct construction sites: `api/mod.rs`
(router builder) and `api/tests.rs` (test helper). The `router()`
call in the SPA integration test also needs the extra arg. When
`strategy_params_by_model` was added, all 3 were updated in one patch
set with no compile errors.

### Workflow gate — hold for explicit go-ahead on live-engine multi-file work

**User preference (persistent):** Inah dislikes autonomous "jump into
solution mode" moves. Explicit frustration 2026-08-03: *"I am not happy
that you jumped into solution mode without getting my input."* Also
2026-08-05: *"Discuss the plan and get input before making sweeping
multi-file changes."*

When the work scope crosses **3+ files in the live trading engine
path** (`strategy.rs`, `scheduler.rs`, `exec/*`, `main.rs`, `db.rs`,
`config.rs`, or anything that flows into order execution), even after a
clear scope proposal and locked decisions:

1. **Do NOT start editing on timeout default.** If `clarify` returns
   "user did not provide a response within the time limit", do NOT
   announce "proceeding with default scope" and start patching files.
   The Phase 5 multi-model registry work is a reference positive: the
   plan amendment was committed as a single doc-only commit FIRST, then
   code edits followed. That is the user's preferred gate pattern.
2. **Summarize where we are + what comes next, then stop.** A
   "Standing by for your call" message is the correct response, not
   a transition into solution mode. If the timeout fires after a
   design-pivoting question, surface the options and include "wait" as
   the obvious choice — do NOT pick one and start coding.
3. **Wait for explicit "proceed" or "yes".** Even if the user said
   "up to u" earlier, treat that as "you choose between options I
   named" — not "you may also start editing now." "up to u" never
   overrides the multi-file-live-engine gate.
4. **Check `honcho_profile` / `session_search` for recent user frustration
   signals** before starting any multi-file edit. If a recent turn
   contains "jump into solution mode", "not happy", "I asked you to
   wait", "this is a good place to pause", or similar, gate the work
   behind a confirmation round.
5. **Commit the plan doc update BEFORE editing code.** When a
   multi-file change has architectural implications (e.g. switching
   from `Vec<SymbolConfig>` to a DB-backed registry), update
   `.hermes/plans/`, commit it as its own commit on the feature
   branch, and ONLY THEN start editing code. This gives the user a
   single review surface for the architectural pivot and keeps the
   subsequent code commits historically accurate.

The cost of an unnecessary "shall I proceed?" is ~30 seconds of user
time. The cost of an unsolicited multi-file refactor to a live trading
system is a rollback or a frustrated user — both much worse.

## Phase 3 — Complete ✅ (commits 53e08ae + 8a4aba5)

Phase 3 is fully committed as of `8a4aba5` (chart auto-refresh + live quote + trade markers + prediction cones). All sub-components shipped:

| Sub-component | Commit |
|---|---|
| PSQ shorting, PaperExecutor remap, MoomooExecutor, TOTP, mode toggle, tests | `53e08ae` |
| Chart auto-refresh, `/api/quote`, live_quote, stale flag, 22-file frontend polish | `8a4aba5` |

**Only Phase 4 (AI Advisor) remains.** Phase 4 is fully additive — it touches `advisor.rs`, `api/advisor.rs`, and the Advisor frontend view; nothing in Phase 4 touches the execution path. Phase 4 is NOT a blocker for live trading.

### Deploy mechanism: local Docker, no git remote

**This repo has NO `origin` remote.** Deployment is done directly on the VPS via local Docker commands, not `git push`. Build runs on the VPS after syncing code.

```bash
# On the VPS — NOT on the dev machine (no internet access from dev machine):
# 1. Sync latest code to the VPS first (rsync, scp, or manual copy)
# 2. Build (Svelte frontend builds inside Docker via multi-stage)
docker build -t marketmarkovnet/engine:latest -f engine/Dockerfile .

# 3. Rolling restart (preserves named volumes: data + models)
docker compose -f deploy/docker-compose.yml up -d --no-deps engine

# Or full restart:
docker compose -f deploy/docker-compose.yml up -d
```

**Pre-deploy checklist (run on dev machine first):**
- `cargo check -p engine` passes ✅
- `cd frontend && npm run build` passes ✅
- `git status --porcelain` is empty (changes committed) ✅
- `git log --oneline -3` shows expected commits ✅

**Post-deploy verification:**
```bash
# Verify running container image matches what was just built
docker inspect mmn-engine --format '{{.Image}}'
docker inspect marketmarkovnet/engine:latest --format '{{.Id}}'

# Quick health check
docker ps --filter name=mmn-engine --format '{{.Status}}'
curl -s http://localhost:9080/api/mode | jq .
```

**Coded ≠ Deployed ≠ Live-verified.** After rebuild, always verify the container is actually running the new image before declaring the deploy done.

### Phase 4 — Not started

Tasks remaining:
- `engine/src/advisor.rs` — LLM briefing + chat (DeepSeek V4 Flash via OpenRouter)
- `engine/src/api/advisor.rs` — `GET /api/advisor/briefing`, `POST /api/advisor/ask` (SSE streaming)
- `engine/src/db.rs` — `insert_advisor_log`, `fetch_advisor_logs`
- `frontend/src/views/Advisor.svelte` — Briefing card, chat, suggested params → Strategy Lab
- `engine/src/main.rs` — spawn advisor background task if `ADVISOR_ENABLED=true`

## Frontend Control-Room API Contract (Phase 3 — shipped)

The control-room dashboard (`frontend/src/views/Dashboard.svelte`) has three
data sources: REST initial load, WS live updates, and the engine's static
serving of the built SPA. Mismatches between these cause "blank panel" bugs
that are NOT backend failures — the API returns data, but the frontend can't
read it.

### REST endpoints and their response shapes

- `GET /api/status` → `StatusResponse` with `pred_1d`, `pred_5d`, `pred_21d`
  (all `Option<f64>`), `mode`, `position`, `realized_pnl`, `unrealized_pnl`,
  `last_close`, `staleness_secs`, `sma_200`. Defined in `engine/src/api/status.rs`.
- `GET /api/predictions` → `{latest: PredictionDto|null, history: [PredictionDto]}`
  where `PredictionDto` has `pred_1h`, `pred_4h`, `pred_24h` (NOT `pred_1d`!),
  `pred_1h_approx`, `pred_5h_approx`, `actual_1h/4h/24h`. The equity variant
  (`equity_prediction_to_dto`) maps `pred_1d` → `pred_24h` and derives the
  hourly approximations as `daily / 6.5`. Defined in `engine/src/api/predictions.rs`.
- `GET /api/equity/features?symbol=QQQ&limit=N` → `{symbol, count, latest: EquityFeatureRow, rows: [EquityFeatureRow]}`
  where `EquityFeatureRow` has **named fields**: `timestamp`, `trend_slope`,
  `trend_adx`, `rsi_14`, `vix_regime`, `tlt_corr_20d`, `rvol_20d`, `gap_pct`,
  `drawdown_from_50d_high`. Defined in `engine/src/features/equities_v2.rs`.
  The 8 feature names MUST match `EQ_FEATURE_NAMES` in that file.

### WS event shapes (engine/src/api/ws.rs `TelemetryEvent`)

- `PredictionUpdate { pred_1d, pred_5d, pred_21d, timestamp }` — uses the
  `pred_Nd` naming (NOT `pred_Nh` like the REST predictions DTO).
- `FeatureUpdate { features: Vec<f64>, normalized: Vec<f64>, timestamp }` —
  sends **arrays** in `EQ_FEATURE_NAMES` order, NOT named fields.
- `PnlTick`, `TradeFill`, `ModeChange` — see ws.rs.
- `StalenessAlert` — **phantom event**: defined in the enum, has a
  serialization test, and is handled in the frontend WS manager, but **no
  backend code ever broadcasts it.** Do not rely on it for live staleness
  updates. The frontend must poll `/api/status` periodically instead (see
  "Staleness display" section above).

### The REST↔WS shape mismatch (fixed in this session)

The FeatureInspector component was blank because:
1. It expected `data.latest.normalized` and `data.latest.features` (arrays) —
   the REST API returns named fields (`trend_slope`, etc.), not arrays.
2. It used wrong feature names (`ret_1d`, `vol_atr`, `corr_vix`, `corr_tlt`)
   that don't exist in `EquityFeatureRow` — the real names are `trend_slope`,
   `trend_adx`, `rsi_14`, `vix_regime`, `tlt_corr_20d`, `rvol_20d`, `gap_pct`,
   `drawdown_from_50d_high`.
3. The WS `FeatureUpdate` sends arrays, so the component needed a normalization
   layer to handle both shapes.

The StatusPanel was missing predictions because:
1. The template had no rows for `pred_1d/5d/21d` even though `StatusResponse`
   carries them.
2. The WS `PredictionUpdate` handler only updated the `predictions` store, not
   the `status` store — so live prediction changes never reached StatusPanel.

### Deploy workflow (frontend changes)

The engine serves the built SPA from `frontend/dist/` via Rust `ServeDir`.
There is no separate frontend container — the engine Docker image bakes in
the built assets. To deploy frontend changes:

```bash
cd /home/ubuntu/projects/MarketMoves
# 1. Build the frontend
cd frontend && npm run build
# 2. Rebuild the engine Docker image (picks up new dist/)
cd ../deploy && docker-compose build engine
# 3. Remove old container (docker-compose up -d alone fails with name conflict)
docker rm -f mmn-engine
# 4. Start fresh container
docker-compose up -d engine
# 5. Verify the served bundle hash matches the built dist
curl -s http://localhost:9080/ | grep -oP 'index-[A-Za-z0-9_-]+\.js'
ls frontend/dist/assets/index-*.js
# Hashes must match
```

**Pitfall:** `docker-compose up -d engine` after a rebuild fails with
"container name already in use" — you must `docker rm -f mmn-engine` first.
`docker-compose rm -f engine` does NOT work if the container is running.

### Prediction scheduling cadence

- **Data pull**: Yahoo Finance daily OHLCV. Full backfill at startup, then
  top-up every 24 hours. The freshness gate is **parameterized** — startup
  uses 3 calendar days (conservative), daily top-up uses 18h (daily bars
  close at 16:00 ET; anything older means yesterday's bar was missed), and
  the manual API refresh endpoint passes 0 (always fetch). The parameter
  flows through `backfill()` → `backfill_many()` → `backfill_equities()`.
- **Scheduler poll**: every 5 minutes, checks if a new daily candle appeared.
  Only processes when there's actually a new candle — effectively once per
  trading day, after market close.
- **Actuals computation**: hourly background task backfilling actual returns
  against past predictions.
- One prediction per trading day, NOT per hour. The 5-minute poll is just to
  detect the new daily bar promptly.

### Staleness display: the phantom WS event + missing REST poll

The `staleness_secs` field in `StatusResponse` is computed as
`now_utc - latest_equity_candle_ts` (`engine/src/api/status.rs`). The
frontend displays it in `StatusPanel.svelte` and `ModelHealth.svelte`.

Two bugs were found and fixed in this area:

1. **`StalenessAlert` is a phantom WS event.** It is defined in the
   `TelemetryEvent` enum (`engine/src/api/ws.rs`), has a serialization test,
   and is handled in `frontend/src/lib/websocket.js` — but **no backend code
   ever sends it.** The scheduler, ingestion supervisor, and health monitor
   do not broadcast it. Any frontend field that relies on WS-pushed staleness
   will be frozen at its page-load value forever. Do not assume a WS event
   being defined + handled means it is actually emitted.

2. **The Svelte Dashboard had no REST polling fallback.** `Dashboard.svelte`
   fetched `/api/status` once on mount, then relied entirely on WS events.
   Since `StalenessAlert` is never emitted, the displayed staleness was frozen.
   Fix: add a 30s `setInterval` polling `/api/status` in `onMount`, cleared in
   `onDestroy`. This also catches staleness drift and any other field that
   lacks a WS push path. The WS layer remains for low-latency updates
   (PnlTick, TradeFill, ModeChange, PredictionUpdate); the poll is the
   safety net.

## Multi-Model Inference Service (2026-08-13)

### Architecture: per-symbol ensemble loading + symbol-routed ZMQ requests

The inference service (`inference/equity_model.py`) loads multiple model
ensembles keyed by symbol. On startup, it scans `MODELS_DIR` for
subdirectories containing `*tcn*.pt` files — each subdirectory is a
symbol bundle (e.g. `/models/NVDA/`). The flat layout (`/models/qqq_tcn_v1.pt`)
provides the default QQQ ensemble. The `main()` function builds a
`dict[str, EquityEnsemble]` and passes it to `run_service()`.

The engine sends `"symbol": "QQQ"` or `"symbol": "NVDA"` in the ZMQ V3
request payload (`bridge.rs::predict_v3`). The inference service's
`_handle_request` looks up the ensemble by symbol, falling back to the
first available for legacy clients without a symbol field.

### Z-score blending (matches notebook walk-forward evaluation)

Each `EquityEnsemble` maintains per-horizon prediction buffers
(`deque[maxlen=252]`) for both TCN and LGBM. Before blending, each
model's raw prediction is z-scored against its own buffer. This removes
per-model bias and matches the Colab notebook's walk-forward evaluation.

**Cold start**: the first ~2 predictions per symbol will be 0.0 (the
`_zscore` function returns 0.0 when buffer has <2 entries). This is
expected. Buffers fill within 2 trading days.

### Healthcheck buffer pollution (FIXED 2026-08-13)

**Critical pitfall:** The inference healthcheck sends `symbol=""` with
all-zero features (1×8 zeros). Without protection, these requests were
routed to the NVDA ensemble (alphabetically first: `NVDA` before `QQQ`)
and flooded the z-score buffers with zero-noise. Over 6 days, ~16,000
zero-predictions made every real NVDA prediction z-score to ~0.0.

**Fix:** `_handle_request` detects healthcheck requests
(`seq_len=1` + all-zero features) and passes `skip_buffer=True` to
`ensemble.predict()`. Healthcheck predictions still return valid
JSON but do NOT update the z-score buffers.

### Pitfalls (inference service)

- **`ORDER BY ts ASC LIMIT N` returns OLDEST N rows, not latest.**
  `fetch_equity_candles_asc` in `engine/src/db.rs` had this bug: it
  used `ORDER BY ts ASC LIMIT ?2` which returns the oldest N candles
  (2021 data) instead of the latest N (2026 data). This caused:
  wrong SMA (340 vs 711), wrong regime ("bear" vs "bull"), zero trades,
  and garbage NVDA predictions. **Fix:** use a subquery —
  `SELECT ... FROM (SELECT ... ORDER BY ts DESC LIMIT N) ORDER BY ts ASC`
  to get the latest N rows, then re-sort to chronological order.
  ALWAYS verify ASC+LIMIT queries in SQLite by checking the timestamp
  range of the returned rows.

- **Derived (read-only) Svelte stores do NOT have a `.set()` method.**
  `CandlestickChart.svelte` called `chartData.set(data)` where `chartData`
  is a `derived` store. This threw `Ll.set is not a function` (minified
  name) on every chart refresh. **Fix:** use `updateSlice($activeModelId,
  'chartData', data)` — the correct write path through the per-model
  slice store. Any store created with `derived(...)` is read-only; writes
  must go through the underlying `writable` store via `updateSlice`.

- **Inference container restart resets z-score buffers.** Every restart
  empties the prediction buffers. The first ~2 predictions per symbol
  will be 0.0 until buffers fill. This is expected, not a bug — but it
  means live predictions may be temporarily flat after a redeploy.

- **Per-model prediction buffers are in-memory, not persisted.** The
  z-score buffers live in the `EquityEnsemble` Python object. If the
  inference container restarts (OOM, deploy, crash), buffers are lost.
  This is acceptable for now but means predictions need a warmup period
  after every restart.

## References
- `references/multi-model-inference-architecture.md` — per-symbol ensemble loading, z-score blending implementation, healthcheck buffer pollution fix, and the `fetch_equity_candles_asc` SQL bug reproduction recipe.
- `references/psq-inverse-etf-remap.md` — exact fill matrix (symbol + side per transition), PnL formula, and the file/function map for the equities executor.
- `references/moomoo-opend-integration.md` — OpenD architecture, `place_order.py` CLI contract, trade context creation, and env vars for the LiveExecutor.
- `references/options-opra-greeks-probe.md` — Options Momentum Engine data-feed verification: `probe_option_quota.py` bugs (protected `moomooapi` skill — flag, don't own), `StrategyLegAction.BUY` enum fix, greeks are LIVE-HOURS-ONLY (not an entitlement gap when static fields populate), OPRA tier reading, and the correct minimal greeks fetch. **Use when verifying option quotes/greeks/OPRA or debugging a silent/false probe.**
- `references/totp-rs-integration.md` — `Cargo.toml` feature flags, working API surface, time-source conversion, common compile errors → fixes for the runtime mode toggle.
- `references/phase36-rollout-and-live-verification.md` — deploy gate, the coded/deployed/live-verified distinction, Phase 3.6 rollout checklist with VPS queries, live-flip playbook, the README-drift watch (which was actually rewritten as part of commit `53e08ae`), AND section 8 contains a reusable ad-hoc `bash` verification recipe you can adapt for any future Phase-3 deploy. **Use when the user asks "have we done Phase 3.6?", "can we go live?", "is Phase 4 a blocker?", "what's actually deployed?", "show me proof the deploy worked?"** — not for code-build tasks.
- `references/strategy-lab-ic-analysis.md` — updated 2026-07-30: `backfill_predictions` is now
  implemented and predictions table has 1125 rows. Directional accuracy: 62.4% / 58.0% / 67.5%
  across horizons on 500 resolved predictions. Read before any strategy backtest work.
- `references/strategy-sma-regime-analysis.md` — 2026-07-30: SMA=40 doubles Sharpe vs SMA=200
  (1.41 vs 1.05). Full parameter sweep results, recommended config for 7 trades/yr with
  18% CAGR and 1.41 Sharpe. pred_5d filter constraint and code-change recommendation.
- `references/phase5-multi-asset-and-sentiment.md` — Phase 5 implementation recipes:
  NVDD inverse-ETF config (3-line add), multi-symbol scheduler loop in main.rs,
  `SymbolConfig` migration from flat env vars, `db::fetch_recent_sentiment` query,
  `apply_sentiment_overlay` function + scheduler.rs hook, API contract, frontend
  toggle UI, verification gates. **Recipes 1-3 + 7-8 SHIPPED (2026-08-05).**
  Recipes 4-6 (sentiment overlay) remain deferred. **Use when resuming Phase 5
  work or reviewing multi-model architecture** — concrete code skeletons,
  shipped commit hashes, and pitfalls learned during implementation.

## Scripts
- `scripts/verify-phase.sh` — runnable verification recipe (copies into repo root, then `bash` it). Excludes the pre-existing `config::tests` and `parity_harness` failures.
