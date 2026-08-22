---
name: marketmoves-dev
description: Use for MarketMoves dev work — engine/UI changes, options engine, Futu API limits, config, deploy.
category: project
---

# MarketMoves Development Patterns

> **Phase 7 options architecture** (config store + registry, D13 promotion gate: queue at API → apply at daily candle boundary, never mid-exit, exit modules take `with_config` constructors, configs re-read per pipeline run): see `references/phase7-options-architecture.md`.
>
> **Engine backend patterns** (API endpoints, DB migrations, tests, events): see `references/engine-backend-patterns.md`. Two traps that bit us live: (1) `DDL.split(';')` loader breaks on semicolons inside SQL comments; (2) lib.rs and main.rs are separate crate trees — new modules must be declared in BOTH or the bin fails to compile. Also: endpoint tests call handlers directly (no tower), idempotent migrations via pragma_table_info probe for live tables, closed event-category set (trade|data|system|strategy|alert|advisor).
>
> **Merge recovery & boot crash-loop** (`git checkout --ours` silently drops the other branch's functions → cascade of `cannot find function`; and the silent `process::exit(1)` crash-loop when a `trading_models` row's norm_stats file is missing): see `references/merge-recovery-and-boot-crashloop.md`.
>
> **Frontend (Svelte) conventions** for Options UI work: see `references/frontend-ui-conventions.md`. Two data-contract traps caught live: (1) `engine_events.ts` is SECONDS (`Utc::now().timestamp()`) while option positions/candles are MILLIS — frontend must normalize (`ts < 1e12 ? ts * 1000 : ts`) or dates render as 1970; (2) config registry `kind` serializes as `"int"`/`"float"`, NOT `"i64"`/`"f64"`.

## Collaboration style for this project

The user prefers a collaborative, step-by-step approach **for planning**,
but once a direction is set, they want **direct action over lengthy
explanation**. When they say "ok check why the dashboard shows zero and
pls fix", they mean: diagnose and execute the fix, don't give a 5-minute
lecture on the z-scoring algorithm first. After the fix, a concise summary
is welcome; before the fix, it's noise.

**Heuristic:** if the user asks "why is X happening" → short technical
answer + immediate fix. If they say "what should we do about Y" → present
options, get input, then execute. Avoid: "Let me explain the problem..."
when they've already signaled they want it fixed.

**Multi-bug diagnosis pattern:** When the dashboard shows wrong data
(zero PnL, N/A accuracy), trace the full data flow: DB → backend query
→ API response → frontend store → component render. The bug is often a
contract mismatch between layers (e.g. field renamed on backend but not
frontend, or executor in-memory state diverged from DB state). Verify
each layer independently with curl + sqlite queries before assuming
which layer is broken.

## CRITICAL: Which File Trained the Live QQQ Model?

There are two training sources in this repo, and only ONE produced the deployed model:

| File | Status | What it is |
|---|---|---|
| `models/colab/QQQ_Equities_Model.ipynb` | **AUTHORITATIVE** | The actual notebook that produced `qqq_tcn_v1.pt`, `qqq_lgbm_h{1,5,21}_v1.pkl`, `norm_stats_qqq_v1.json`. Source of truth. |
| `training/train_tcn.py` | **DORMANT V2 BTC pipeline** | Older code from Wave 3 (BTC TCN, 4 Colab runs, IC ≈ 0 — no alpha). Not used for QQQ. Do not reference for QQQ strategy/inference work. |

**Mistake to avoid:** Reading `train_tcn.py` first when doing model/strategy analysis. Its constants (`SEQ_LEN=72`, `TimeSeriesSplit(n_splits=5)`, 3-dim features) do NOT match the live model. Always start from the notebook.

**Key constants from the live notebook (`models/colab/QQQ_Equities_Model.ipynb`):**
- `SEQ_LEN = 126` (not 72)
- `HORIZONS = [1, 5, 21]`
- `EMBARGO_DAYS = max(SEQ_LEN, 21) + 10 = 136`
- `IC_GATE = 0.03`, `MAG_CLIP = 3.0`
- Walk-forward: 5y train / 1y val rolling, step 1y
- LGBM: `objective='huber', n_estimators=100, max_depth=6, lr=0.01, random_state=42`, `vix_regime` as `categorical_feature`
- TCN: 7 ResidualBlocks with dilations [1,2,4,8,16,32,64], 15 epochs/fold, 30 epochs final retrain, AdamW lr=5e-4, SmoothL1Loss

The feature engineering reference (per-feature formulas, parity fixtures) lives in `training/equities_features.py` — that file IS authoritative for features, even though `train_tcn.py` is dormant.

## Pre-existing notebook vs deployed-inference divergences

Three silent mismatches between `models/colab/QQQ_Equities_Model.ipynb` and `inference/equity_model.py` exist independent of any retraining work:

1. **Blend strategy mismatch (RESOLVED 2026-08-07).** Notebook Cell 14 uses per-fold z-score before averaging:
   ```python
   l_pred = (lgbm_preds - mean) / (std + 1e-8)
   t_pred  = (tcn_preds - mean) / (std + 1e-8)
   ens_pred = (l_pred + t_pred) / 2.0
   ```
   Deployed `equity_model.py::EquityEnsemble.predict` now uses **rolling z-score
   blending** with per-horizon prediction buffers (252 slots), matching the notebook's
   evaluation pipeline. See `references/z-score-blending.md` for the full architecture.
   Previously, raw 0.5/0.5 weighted average was used, which preserved per-model bias
   and caused poor directional accuracy (NVDA: 38% vs potential 50%+).

2. **`drawdown_from_50d_high` source differs (RESOLVED 2026-08-14).** Notebook Cell 8 uses `df['high'].rolling(50).max()`; Rust `engine/src/features/equities_v2.rs` was confirmed to use `highs` (fixed in parity fix 1A, 2026-08-05). Python training helper (`training/equities_features.py`) used `closes` — corrected to `highs` to match notebook + Rust. The stale Rust comment claiming "deployed model trained on close-based" was corrected — notebook is the source of truth and uses `high`.

3. **Z-score pooling formula fixed (2026-08-14).** `EquityEnsemble._pooled_std` previously concatenated TCN + LGBM buffers, which included between-model mean offset as variance. Fixed to proper pooled std: `sqrt((s_a^2 + s_b^2) / 2)`. This corrects a systematic prediction shrinkage bug — the old formula inflated variance, deflating z-scores and keeping predictions near zero.

4. **`label_std` is mag-space, not return-space (2026-08-14).** The notebook computes labels as `mag = clip(fut_ret / (ATR/close), -3, +3)`, so the saved `label_std_*` values are the standard deviation of **ATR-scaled** labels (~0.727 for 1d). The inference code must multiply by the current `atr_ratio` to get back to raw log-return space: `raw_return = z * label_std * atr_ratio`. Forgetting the `atr_ratio` factor produces astronomical predictions (e.g. 73.8% 1-day moves, chart price targets like $1273). **Always verify prediction magnitudes are reasonable before adjusting strategy thresholds.** See `references/atr-label-scaling-contract.md` for the full contract and sanity checks.

## Multi-asset expansion: constraint that bites any new ticker

The infrastructure (inference path, scheduler, executor, `cfg.symbol`/`cfg.short_symbol`) is already symbol-agnostic. But the model's `norm_stats_qqq_v1.json` is QQQ-specific — applying it to PLTR inputs puts features ~+2σ outside the trained distribution. **Every new ticker needs its own norm stats + retrained artifacts.**

The `norm_stats_qqq_v1.json` has `rsi_14.mad = 14.0` exactly (hits the `MAD_FLOORS` floor) and `vix_regime.mad = 1.0` (floor hit). For other tickers, the floor table stays identical — it's a safety net, not a target.

## Walk-forward validation: data depth kills the IC gate for young tickers

Notebook walk-forward is `5y train / 1y val, step 1y`. With QQQ's ~25y history this gives ~15 folds and a robust IC gate at 0.03. With PLTR's ~5.2y (IPO Sep 2020) you get **0–1 folds** — standard error on Pearson r at N=252 OOS bars is ~0.06, which is bigger than the 0.03 gate itself. **The current gate structure cannot validate PLTR.**

Workarounds:
- **Shrink windows:** `train_size = 2*252, step_size = 126` → ~5 folds × 126 OOS bars. SE on mean IC ~0.018.
- **TimeSeriesSplit:** `n_splits=5, gap=136`. Expanding window, different question than what QQQ was tested with.
- **Skip the gate, paper-trade immediately:** Real OOS but slow feedback (months).

Recommendation: shrink windows + paper-trade in parallel. Don't deploy on 1-fold IC.

## Inverse-ETF mapping for tech-sector single names

The engine supports short positions via `cfg.short_symbol` (PSQ for QQQ). The Direxion daily -1x single-stock ETF family covers most large-cap tech:

| Long | Short (-1x) | Notes |
|---|---|---|
| PLTR | PLTD | Inception 2024-12-10. Pre-Dec-2024 PLTR shorts have no inverse instrument; use TECS/TECZ (sector) as proxy or skip the short leg. |
| NVDA | NVDD | Most-traded single-stock bear. |
| TSLA | TSLS | High AUM. |
| AAPL | AAPD | |
| MSFT | MSFD | |
| AMZN | AMZD | |
| META | METD | |
| GOOGL | GGLS | |
| AMD | AMDD | |
| AVGO | AVS | |
| MU | MUD | |
| NFLX | NFXS | |
| ORCL | ORCS | |
| PANW | PALD | |
| QCOM | QCMD | |
| TSM | TSMZ | |

Sector-level alternatives: **TECL/TECS** (+/-3x tech sector, ~$96M AUM, 0.95% expense ratio), **TECZ** (-1x tech, lower vol decay than TECS), **SPDN** (-1x S&P 500), **QQQD** (-1x Magnificent 7 basket).

Same PnL formula as PSQ for QQQ:
```
inverse_return ≈ -(underlying_return)
inverse_current ≈ inverse_entry * (1 - underlying_return)
PnL = inverse_entry - inverse_current ≈ inverse_entry * (underlying_current/underlying_entry - 1)
```

All -1x products reset daily — multi-day holds drift from the underlying. Mostly negligible for daily strategies; track if you start holding for weeks.



Strategy params are shared between HTTP API and scheduler via
`Arc<RwLock<EquityStrategyParams>>`. Pattern mirrors `/api/mode`:

```
AppState.strategy_params  ──Arc──┐
                                 ├── same lock
EquityScheduler.strategy_params ──┘
```

- `GET /api/strategy-config` — read current params
- `PUT /api/strategy-config` — partial update, validates, broadcasts WS event
- Changes take effect on next bar — no restart needed
- Scheduler acquires read lock at cycle start, DROPS before calling
  `evaluate_and_execute_strategy()` (takes `&mut self`), re-acquires inside

### Adding a new strategy param
1. Add field to `EquityStrategyParams` in `engine/src/strategy.rs`
2. Add field + env var parsing to `Config` in `engine/src/config.rs`
3. Add to `StrategyInfo` struct in `engine/src/api/status.rs`
4. Add to `StrategyConfigResponse` + `StrategyConfigUpdate` in `engine/src/api/strategy_config.rs`
5. Add to `TelemetryEvent::StrategyConfigChange` in `engine/src/api/ws.rs`
6. Update `AppState` construction in `engine/src/api/mod.rs`
7. Update `EquityStrategyParams` construction in `engine/src/main.rs`
8. Add to Dockerfile `ENV` defaults
9. Add to `docker-compose.yml` environment overrides
10. Add to `.env` (project root) with the desired startup default —
    this is the source of truth on container restart (see Docker Pitfalls)

**CRITICAL PITFALL — the API does NOT serialize `EquityStrategyParams`
wholesale.** `StrategyConfigResponse`, `StrategyConfigUpdate`, the GET/PUT
handlers, and the `StrategyConfigChange` telemetry event each enumerate
every field by hand. Adding a field to the struct is necessary but NOT
sufficient — you must also add it to ALL of these sites or you get
`E0063: missing field` / `E0308: missing field` compile errors:
  - `engine/src/api/status.rs` — `StrategySnapshot` struct + the
    `StrategySnapshot { ... }` construction inside `handle_status()`
  - `engine/src/api/strategy_config.rs` — `StrategyConfigResponse` struct,
    the GET response build, `StrategyConfigUpdate` struct, the PUT update
    `if let Some(v) = update.X` block, the PUT response build, AND the
    `StrategyConfigChange` emit (`&crate::strategy::EquityStrategyParams { ... }`
    — note this needs a `&` prefix, not a bare struct literal)
  - `engine/src/api/mod.rs` — `AppState` `strategy_params` `EquityStrategyParams { ... }`
  - `engine/src/main.rs` — `model_strategy_params` `EquityStrategyParams { ... }`
  - Every test file that constructs `EquityStrategyParams { ... }`:
    `engine/src/strategy.rs` (eq_params helper + inline literals),
    `engine/src/api/tests.rs` (3-4 literals), `engine/tests/mode_toggle.rs`.
  - Every test that constructs `Config { ... }`: `engine/src/config.rs`
    (from_env test), `engine/src/api/tests.rs`, `engine/tests/mode_toggle.rs`.
  - Every test that constructs `StrategyConfigUpdate { ... }`:
    `engine/src/api/tests.rs` (2 literals).
Run `cargo test --lib` after adding a param — the test build catches the
struct-literal sites that the lib build does not.

**Pattern for the `short_pred_5d_filter` fix (2026-08-14):** to add a
symmetric short-side gate to the long `pred_5d_filter`, the field defaults
to `false` (backward-compatible — shorts previously had no pred_5d gate)
and is applied in the short-entry block:
```rust
if params.enable_shorting && current == Position::Flat && input.sma_valid {
    if input.pred_1d < params.short_entry_threshold
        && (!params.short_pred_5d_filter || input.pred_5d < 0.0)
    {
        return Position::Short;
    }
}
```
Env var is `SHORT_PRED_5D_FILTER` (read in config.rs, passed through
compose `.env`, default `false` in Dockerfile).

## Verify prediction scale before tuning thresholds

Before changing `entry_threshold`, `exit_threshold`, or any threshold that
acts on `pred_1d` / `pred_5d`, confirm the predictions are in **raw
log-return space** (single-digit percent range), not ATR-scaled label
space (hundreds of percent). Symptoms of the wrong scale:

- Chart price targets far outside the asset's daily move range (e.g.
  QQQ 1-day target of $1273 when price is $733).
- `pred_1d` values consistently > 0.1 (10%) or < -0.1 (-10%).
- Inference log shows real `seq_len=126` requests with `atr_ratio` but
  the stored `equity_predictions` are 50–100× larger than the inference log.

**Required checks before touching thresholds:**
1. Look at the latest real inference request (not the `seq_len=1`
   healthcheck): `docker logs mmn-inference | grep seq_len`.
2. Compare to the stored prediction: `SELECT pred_1d, pred_5d,
   pred_21d FROM equity_predictions ORDER BY candle_ts DESC LIMIT 1`.
3. Sanity: `pred_1d * close` should be a plausible 1-day dollar target.

If the scale is wrong, the fix is almost always the `atr_ratio` conversion
in `inference/equity_model.py`. See `references/atr-label-scaling-contract.md`
for the exact math and patch pattern. See
`references/inference-logs-real-vs-healthcheck.md` for how to tell real
requests from healthchecks in the inference logs.

See `references/threshold-recalibration-2026-08-14.md` for the concrete
threshold values, files that must be updated, and the verification steps
used after the 2026-08-14 prediction-scale fix.
See `references/directional-accuracy-per-symbol.md` for the per-symbol /
time-windowed `/api/accuracy` recipe (endpoints, sample-size pitfalls,
fresh-model backfill fix, wall-clock cost).
See `references/frontend-engine-bundle-deploy.md` for the
frontend-into-engine-image deploy loop (npm build → docker build →
docker-compose restart) and the served-bundle-hash verification pattern
that catches stale-cache UI bugs.
See `references/frontend-engine-bundle-deploy.md` for the
frontend-into-engine-image deploy loop (npm build → docker build →
docker-compose restart) and the served-bundle-hash verification pattern
that catches stale-cache UI bugs.

## Docker Pitfalls

- **Zero dashboard predictions after inference fix (2026-08-14).** When the
  inference logic changes from "returns 0.0 during cold start" to "returns
  raw blend immediately", the old zero predictions stay in
  `equity_predictions`. The scheduler recovers `last_processed_ts` from the
  latest prediction row, so it never reprocesses those candles. The
  dashboard shows the latest (zero) prediction until a new candle arrives.
  **Fix:** delete the zero rows, then restart the engine. The backfill will
  regenerate non-zero predictions using the new inference image.
  ```bash
  # From host, via the Docker volume (sudo required)
  sudo python3 - <<'PY'
  import sqlite3
  conn = sqlite3.connect('/var/lib/docker/volumes/deploy_data/_data/candles.db')
  c = conn.cursor()
  c.execute("""DELETE FROM equity_predictions
               WHERE symbol = 'QQQ'
               AND pred_1d = 0.0 AND pred_5d = 0.0 AND pred_21d = 0.0
               AND candle_ts >= (strftime('%s', 'now', '-10 days'))""")
  conn.commit(); conn.close()
  PY
  cd /home/ubuntu/projects/MarketMoves/deploy && docker-compose restart engine
  ```
  Then verify the latest prediction is non-zero:
  ```bash
  curl -s http://localhost:9080/api/predictions?symbol=QQQ&limit=1
  ```
  See `references/zero-dashboard-predictions-2026-08-14.md` for the full
  debug transcript, SQL queries, and the related norm_stats/model-path issue.
  See `references/threshold-recalibration-2026-08-14.md` for threshold values
  after a prediction-scale fix. See
  `references/inference-logs-real-vs-healthcheck.md` to distinguish real
  requests from healthcheck pings in the inference logs.

- **`docker-compose.yml` environment section OVERRIDES Dockerfile `ENV` defaults.
  Always update BOTH files when changing defaults.
- **`docker-compose build` fails with "no space left on device" mid-tar —
  root cause is dangling Docker images, not the build context** (caught
  2026-08-14). This VPS has a 96GB disk; after months of layered Rust + Python
  rebuilds, dangling images accumulated 19.7GB reclaimable. The engine
  `docker-compose build` failed at ~95% disk usage with
  `Can't add file ... to tar: io: read/write on closed pipe` /
  `no space left on device` partway through the build context — even though
  the `.dockerignore` correctly excludes `.venv/`/`target/`/`data/`. The
  culprit was old/dangling Docker images, NOT the build context size.
  **Fix:** `docker image prune -f` (reclaimed 15.58GB → 21G free → build
  succeeded). Pre-deploy check: `df -h /` shows > 90% used → prune first.
  `docker image prune` only touches dangling images; the running
  `mmn-engine`/`mmn-inference`/`mmn-proxy` images are protected. After
  multi-month deploy cycles, this prune is the first thing to try before
  expanding the disk or rewriting `.dockerignore`.
- **`${VAR:-default}` interpolation reads from the HOST SHELL, not from
  `env_file` (`.env`)**. Docker Compose v1 resolves `${VAR:-}` at compose-parse
  time using the shell environment, not the env_file declared with `env_file:`.
  If a value is only in `.env` and not exported in the shell, the container
  gets the default (empty) even though `.env` has the key.
  **Fix:** Either export the var before `docker-compose up`, or hardcode
  the value without `${}` syntax. This was the root cause of FRED_API_KEY
  arriving empty in the container despite being set in `.env`.
- **`.env` file edits may not take effect on subsequent `docker-compose up`**.
  Compose v1 caches the resolved values from the first parse. After editing
  `.env` (e.g. changing `SMA_WINDOW=200` → `SMA_WINDOW=40`), `docker-compose up`
  still passes the old values. **Fix:** pass env vars explicitly on the command
  line: `SMA_WINDOW=40 PRED_5D_FILTER=true docker-compose up -d engine`.
  This overrides any cached interpolation. Verify with `docker-compose config`.
- **In-memory strategy config resets on every container restart.** The
  `Arc<RwLock<EquityStrategyParams>>` is constructed from env vars at startup.
  PUT-saved runtime changes survive only until the next stop+rm+up cycle.
  `.env` (step 10 of the param checklist) is the source of truth for startup
  defaults. If a config value differs between `.env` and what you set via
  `curl -X PUT /api/strategy-config`, the PUT value wins — until restart.
- **Hermes `terminal(background=true)` rejects shell-level background wrappers**
  (`nohup ... &`, `disown`, `setsid`, trailing `&`). For long-running
  processes like moomoo OpenD, use `terminal(background=true, command=...)`
  so Hermes tracks the lifecycle. Reserve shell-level wrappers for
  systemd-managed production daemons — see `references/moomoo-opend-install.md`
  for the full moomoo-opend.service unit.
- Hot-swap via API is faster than rebuild: `curl -X PUT /api/strategy-config`
- Verify config: `curl /api/strategy-config | python3 -m json.tool`
- **`ContainerConfig` KeyError on recreate**: Docker Compose v1 (Python,
  installed by apt on Ubuntu) throws `KeyError: 'ContainerConfig'` when
  recreating a container with named volumes from an image with stale metadata.
  **Fix:** `docker-compose down && docker-compose up -d` (full down+up).
  This destroys and recreates the network — all three containers (engine,
  inference, proxy) restart. `rm -f engine` alone fails when compose also
  tries to recreate inference and hits the same KeyError. Down+up is the
  only reliable fix for this v1 bug.
  **Lean variant (engine + inference rebuild only, leaves `mmn-proxy`
  untouched):** when the proxy image didn't change but `docker-compose up -d`
  still hits the KeyError mid-way, `docker stop` + `docker rm -f` the failing
  service(s) ONLY (not the proxy), then `docker-compose up -d`. Removing just
  `mmn-engine` and `mmn-inference` was enough to bypass the prior-container
  inspect path that triggers the bug, while keeping `mmn-proxy` running
  (confirmed 2026-08-14). The `docker-compose up -d` may leave a stray
  container with a `<hash>_mmn-inference` name after a partial recreate
  attempt — `docker rm -f <hash>_<svc>` before the lean retry to clean up.
- **`${VAR:-default}` in `docker-compose.yml` reads from the project `.env`,
  NOT from `env_file`**. The `env_file:` directive on a service only
  injects variables into the container; it does NOT make those variables
  available for shell-style interpolation in the compose file itself. On
  this VPS, compose v1.29.2 uses the project `.env` (the file at the root
  of the compose project, which is `deploy/.env` by default). Because the
  real values live in the workspace-root `.env`, `FINNHUB_API_KEY` and
  `OPENROUTER_API_KEY` expand to empty unless you pass
  `--env-file /path/to/.env`. **Fix:** always use
  `docker-compose -f deploy/docker-compose.yml --env-file .env up -d` from
  `/home/ubuntu/projects/MarketMoves`.

- **Exported shell env vars silently override `.env` AND Dockerfile defaults
  (caught 2026-08-14).** Docker Compose v1 resolves `${VAR:-default}` using
  the **current shell environment** before it reads `.env`. If a previous
  session exported `ENTRY_THRESHOLD=0.001`, every subsequent `docker-compose up`
  will pass `0.001` into the container even after you change `.env` to
  `0.008` and rebuild the image with `ENTRY_THRESHOLD=0.008` in the
  Dockerfile. The container inspect will show the stale value, and the
  `/api/status` endpoint will report it. **Fix:** unexport the variables
  before bringing the stack up, or use an explicit clean environment:
  ```bash
  unset ENTRY_THRESHOLD EXIT_THRESHOLD SHORT_ENTRY_THRESHOLD SHORT_EXIT_THRESHOLD
  cd /home/ubuntu/projects/MarketMoves/deploy
  docker-compose -f deploy/docker-compose.yml --env-file .env up -d
  ```
  **Verification:** `docker exec mmn-engine env | grep -E "ENTRY|EXIT|SHORT"`
  must match the intended values. If it doesn't, check `env | grep -E "ENTRY|EXIT"`
  on the host first. This is a different failure mode from the `.env` path
  issue above — the file was correct, the shell was not.
- **Writing to the SQLite DB in a named volume from an ad-hoc container:**
  the `keinos/sqlite3` image runs as uid 100, so it cannot write a DB
  owned by uid 1000. Use an Alpine container and install sqlite as root:
  `docker run --rm -v deploy_data:/data -v ./scripts/register_models.sql:/q.sql:ro alpine sh -c "apk add --no-cache sqlite && sqlite3 /data/candles.db < /q.sql"`.
- **`docker compose` (v2 plugin) does NOT exist on this VPS** — only
  `/usr/bin/docker-compose` v1.29.2. Using `docker compose ...` gives
  "unknown command". Always use `docker-compose` with the hyphen.
- **`docker-compose up -d engine` recreates sibling services too**: when
  the engine image is rebuilt, `up -d engine` does NOT just touch the
  engine — compose may also recreate inference, giving it a name like
  `<hash>_mmn-inference` instead of `mmn-inference`. DNS still resolves
  via the service name (`inference` → container IP) so engine starts
  healthy, but `docker logs mmn-inference` will fail. Cosmetic only,
  but worth knowing when grepping logs by canonical name.

  **Mitigation:** `docker-compose up -d --no-deps engine` prevents compose
  from touching any service that isn't `engine`. BUT: on this VPS (compose
  v1.29.2) `--no-deps` alone still triggers the `KeyError: 'ContainerConfig'`
  bug below when paired with `--force-recreate`. The reliable sequence that
  avoids both the rename and the KeyError is:
  ```bash
  docker-compose stop engine && docker-compose rm -f engine \
    && docker-compose up -d --no-deps engine
  ```
  Inference keeps its canonical `mmn-inference` name, engine starts
  fresh on the new image, no other services are touched. This sequence
  is the standard "rebuild engine with frontend changes" loop on this VPS.
- **`--force-recreate` flag triggers the same `ContainerConfig` KeyError**
  on compose v1. Use the stop+rm+up sequence instead, not
  `up -d --force-recreate`.
- **Transient 502s from Caddy proxy during engine restart.** The Caddy
  proxy caches the engine container's IP. When you stop+rm+up the engine,
  the new container gets a new IP, but Caddy keeps trying the old one for
  ~30s until the health check passes and DNS re-resolves. During this
  window, the browser shows 502 errors for all API endpoints. The proxy
  logs show `dial tcp <old-ip>:8080: connect: connection refused`. This is
  transient — not a code bug. Verify the engine is healthy (`docker ps
  --filter name=mmn-engine`), then hard-refresh the browser. No proxy
  restart needed.

## Build & Test Commands

```bash
# Backend
cd engine && cargo check --lib
cd engine && cargo test --lib -- strategy scheduler rhai

# Frontend
cd frontend && npm run build

# Container
cd /home/ubuntu/projects/MarketMoves
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .
# Recreate ONLY engine — preserves sibling service names on compose v1.29.2:
docker-compose -f deploy/docker-compose.yml stop engine \
  && docker-compose -f deploy/docker-compose.yml rm -f engine \
  && docker-compose -f deploy/docker-compose.yml up -d --no-deps engine
# Note: `up -d engine` alone renames inference to <hash>_mmn-inference.
#       `up -d --force-recreate` triggers KeyError: 'ContainerConfig' on v1.
```

## Rhai Strategy Lab

- Scripts at `engine/scripts/*.rhai` — for backtest experimentation
- Exposed variables: pred_1d, pred_5d, pred_21d, current_close, sma, sma_valid, current_pos
- Must return i64: -1 (short), 0 (flat), 1 (long)
- Backtest via `POST /api/backtest` with `kind: "rhai"` and `params: {script: "..."}`
- Live scheduler runs threshold strategy only — Rhai is lab-only

## DB Schema Pitfalls

### DDL is a single string split by `;` — never insert new tables mid-statement

The startup DDL in `engine/src/db.rs` is one big raw string literal. The `open()` function
splits it on `;` and executes each fragment as a standalone SQL statement. This means:

- **Every `CREATE TABLE` must be a complete, self-contained statement** with its own
  closing `);`.
- **NEVER insert new `CREATE TABLE` statements between the `CREATE TABLE` line and its
  column definitions** of another table. The new table's DDL becomes part of the parent
  table's column list, producing `syntax error near "CREATE"`.
- **Always add new tables AFTER the closing `);` of the last table**, or in a clearly
  separated block.

Example of the WRONG pattern (caused a deploy crash):
```sql
CREATE TABLE IF NOT EXISTS strategy_configs (
-- Phase 4: Advisor briefing log
CREATE TABLE IF NOT EXISTS advisor_briefing_log (  -- STILL INSIDE strategy_configs!
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ...
);
    id TEXT PRIMARY KEY,    -- strategy_configs columns, now after the wrong table
```

### `DDL.split(';')` is naive about SQL `--` line comments (caught 2026-08-05)

The `open()` splitter does not understand SQL `--` comments. A comment
line that contains `;` gets split at that semicolon, leaving an unparseable
fragment. Symptom: **every** test in `db::tests::*` (and any other test that
constructs an in-memory DB) fails with `near "<word>": syntax error` pointing
at random test sites — they share the in-memory pool, so the first failure
poisons the rest. The error surfaces at the test run, NOT at the DDL patch,
which makes it look like a SQL or a code-regression bug.

Concrete example that triggered this in 2026-08-05 (Phase 5 `trading_models`
registry):

```sql
-- BAD — splits at the second `;` and leaves fragments unparseable:
-- JSON-pair examples: "QQQ/PSQ"; "NVDA/NVDD"
CREATE TABLE IF NOT EXISTS trading_models (...)
```

```sql
-- GOOD — rephrase to avoid `;` inside comments:
-- JSON-pair examples: "QQQ/PSQ" or "NVDA/NVDD"
CREATE TABLE IF NOT EXISTS trading_models (...)
```

**Self-test before saving any DDL edit:**

```bash
# Look for `;` inside `--` comments specifically:
awk '/^[[:space:]]*--.*;/ { print NR": "$0 }' engine/src/db.rs
```

This should return zero rows for the committed DDL block. If anything
prints, rephrase the comment to remove the literal `;`. The same rule
applies to `--` column comments inside a `CREATE TABLE` — a column
comment with `;` becomes part of the next column line.

Cheaper path: run `cargo test --lib -- db::tests` after every DDL patch
to catch the symptom in seconds. The 2026-08-05 catch happened at this
stage; debugging took ~5 minutes because `git blame` on the SQL error
didn't point at the comment line directly. Pre-emptive grep is faster.

### Sub-second timestamp ordering is non-deterministic (caught 2026-08-05)

`ORDER BY <ts> DESC` returns rows in **arbitrary order** when multiple
rows share the same second (e.g. three `register_model()` calls in a
test loop). Symptom: a test asserts `models[0].model_id == "u-on-2"`
and fails with `left: "u-on-1", right: "u-on-2"`.

Always use one of:

- Test **membership** (`contains(&..)`) not positional access when
  ordering is non-deterministic.
- Add a secondary sort key that IS unique (e.g. `ORDER BY deployed_at DESC, model_id ASC`).
- Pad timestamps with milliseconds or sub-second resolution at insert time.

The `trading_models.deployed_at` column is set to `Utc::now().timestamp()`
(seconds). Three calls in the same test loop all share the same second.
For registry tables that get bulk-loaded in a single tick, prefer
`Utc::now().timestamp_millis()` or add a secondary sort.

### Adding columns to existing tables requires a migration, not just DDL

`CREATE TABLE IF NOT EXISTS` skips the entire statement if the table already exists.
New columns added to the DDL won't be applied to an existing database. Use the
`PRAGMA table_info` + `ALTER TABLE ADD COLUMN` pattern:

```rust
pub async fn migrate_foo(pool: &DbPool) -> Result<()> {
    let rows = sqlx::query("PRAGMA table_info(foo)").fetch_all(pool).await?;
    let existing: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();
    for (col, col_type, default) in &[("buzz", "INTEGER", "0")] {
        if !existing.iter().any(|name| name == col) {
            let sql = format!(
                "ALTER TABLE foo ADD COLUMN {col} {col_type} NOT NULL DEFAULT {default}"
            );
            sqlx::query(&sql).execute(pool).await?;
        }
    }
    Ok(())
}
```

Key points:
- Call the migration in `open()` alongside `migrate_predictions()`.
- SQLite `ALTER TABLE ADD COLUMN` requires a constant DEFAULT.
- If a column was created with the wrong type, use RENAME→ADD→COPY→DROP
  (SQLite doesn't support `ALTER COLUMN TYPE`).
- `sqlx::Row::get()` requires `use sqlx::Row;` — missing this import produces
  `no method named 'get' found for struct SqliteRow`.

### Changing UNIQUE constraints requires table recreation

SQLite does not support `ALTER TABLE DROP CONSTRAINT` or changing a UNIQUE
constraint in-place. When a constraint needs to change (e.g., `UNIQUE(candle_ts)`
→ `UNIQUE(symbol, candle_ts)` for multi-asset support), the table must be
recreated:

```sql
-- 1. Create new table with correct constraint
CREATE TABLE equity_predictions_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    symbol TEXT NOT NULL,
    candle_ts INTEGER NOT NULL,
    ...,
    UNIQUE(symbol, candle_ts)
);

-- 2. Copy data
INSERT INTO equity_predictions_new SELECT * FROM equity_predictions;

-- 3. Recreate indices
CREATE INDEX equity_predictions_ts_idx_new ON equity_predictions_new (candle_ts DESC);

-- 4. Swap
DROP TABLE equity_predictions;
ALTER TABLE equity_predictions_new RENAME TO equity_predictions;
```

**Pitfall:** If the `ON CONFLICT` clause in the insert statement targets a
different constraint than the one on the table, the conflict resolution is
silently ignored. Always update both the DDL and the insert's `ON CONFLICT`
target together. The engine's `insert_equity_prediction` in `db.rs` must
match: `ON CONFLICT(symbol, candle_ts) DO UPDATE SET ...`.

**Live DB migration:** For a running database, stop the engine first, backup
the DB file, then run the migration via Python's `sqlite3` module (with
`sudo` for Docker volume access) — the Rust migration function in `db.rs` is
for fresh DBs on startup; existing DBs need the manual SQL approach. See
`references/multi-model-inference-limitation.md` for the full recipe
including the backfill script at `references/backfill-predictions.py`.

- **`fetch_equity_candles_asc` returns OLDEST N candles, not latest N
  (caught 2026-08-13)** — the query used `ORDER BY ts ASC LIMIT ?2`, which
  returns the first N rows by timestamp (e.g. 2021 data) instead of the
  most recent N (2026 data). With 1254 total rows and `LIMIT 186`, the
  scheduler loaded 2021 candles while processing 2026 timestamps. This
  caused: wrong SMA (340 vs 711), wrong regime ("bear" instead of
  "bull"), zero trades (no long entries in bear regime), and garbage
  predictions (features computed on 2021 prices). The status API was
  unaffected because it uses `fetch_recent_equity_candles` (DESC query,
  correct). Diagnosis: compare the SMA from the status API
  (`fetch_recent_equity_candles`) vs the SMA in the `positions` table
  (computed by the scheduler from `fetch_equity_candles_asc`). If they
  differ by ~2x, the ASC query is loading old data. Fix: use a subquery
  to grab the latest N rows first, then re-sort ascending:
  ```sql
  SELECT ... FROM (
    SELECT ... FROM equity_candles
    WHERE symbol = ?1 ORDER BY ts DESC LIMIT ?2
  ) ORDER BY ts ASC
  ```
  The `ORDER BY ts DESC LIMIT N` subquery gets the latest N; the outer
  `ORDER BY ts ASC` restores chronological order for the feature
  pipeline. **This is the same class of bug as the chart
  `fetch_recent_equity_candles` ASC/DESC bug (Failure mode A in the
  Chart Staleness section) but in a different function.** Always audit
  ALL candle-fetching functions when adding a new one — the same
  `ASC LIMIT` trap catches every variant.

- **Healthcheck requests pollute z-score prediction buffers (caught
  2026-08-13)** — the inference healthcheck (every 30s) sends
  `symbol=""` with `seq_len=1` and all-zero features. The z-score
  blending code pushes every prediction into per-horizon buffers. Over
  6 days, ~16,000 zero-predictions flooded the NVDA ensemble's buffers
  (NVDA was the fallback for `symbol=""` because it's alphabetically
  first). All real NVDA predictions were then z-scored against a buffer
  full of zeros, producing ~0.0. Diagnosis: inference logs show ONLY
  `symbol=""` and `seq_len=1` requests (no `seq_len=126` real
  requests). The healthcheck predictions show non-zero values (because
  the buffers have been filling), which masks the problem. Fix: detect
  healthcheck requests (`seq_len=1` + all-zero features) and pass
  `skip_buffer=True` to `EquityEnsemble.predict()`. When
  `skip_buffer=True`, the raw predictions are returned but NOT pushed
  into the z-score buffers. This keeps healthchecks from polluting real
  prediction statistics.

## Finnhub Sentiment

The `news_sentiment` endpoint requires a **paid plan** (Startup or higher).
The free-tier API key returns 403 Forbidden for all symbols. The engine's
sentiment module gracefully falls back to stub (score=0.5, source="stub").
The `company-news` endpoint is free but returns raw articles without scores.

## Adding a New API Endpoint (Backend)

When adding a new `GET /api/foo` handler (like `/api/quote`):

1. **Write the handler** at `engine/src/api/foo.rs`:
   ```rust
   use axum::{extract::State, response::Json};
   use serde::Serialize;
   use crate::data::yahoo; // or appropriate data source
   use super::{internal_error, ApiResult, AppState};

   #[derive(Serialize)]
   pub(crate) struct FooResponse { /* fields */ }

   pub(crate) async fn handle_foo(State(state): State<AppState>) -> ApiResult<FooResponse> {
       let data = foo::fetch_foo(&state.symbol)
           .await
           .map_err(|e| internal_error("fetch_foo", e))?;
       Ok(Json(data))
   }
   ```
2. **Wire into `engine/src/api/mod.rs`** — three places:
   - Add `mod foo;` to the module declarations at the bottom
   - Add `use super::foo;` if needed (usually not for `pub(crate)`)
   - Add `.route("/api/foo", get(foo::handle_foo))` to the `Router::new()` chain
3. **Test**: `cargo check -p engine` then `docker build` + `docker-compose up -d engine`
4. **Verify**: `curl http://<container-ip>:8080/api/foo`

### Critical mod.rs pitfalls
- `pub(crate) mod ws` must NOT be removed when adding modules — it is used in `AppState.tx`
- `mod strategy_config` must NOT be removed — used in router
- Use `cargo check -p engine` after every mod.rs edit to catch missing module declarations
- **Adding a field to `AppState` requires updating every test file that constructs
  `AppState` directly.** `api/tests.rs` has at least two places: the `test_state()`
  helper and the `router_serves_static_files_and_api` integration test. Missing the
  field produces `E0063: missing field 'foo' in initializer of 'api::AppState'`.
  Use `cargo test -p engine --lib` to catch these before deploying.

### SSE (Server-Sent Events) handlers

axum's `Sse` type is parameterized by a single stream type. You CANNOT mix
`stream::once` and `stream::try_unfold` in different return branches — the
compiler will reject them as mismatched types. Use a single `try_unfold` with
state that handles all error cases internally:

```rust
pub async fn handle_ask(State(state): State<AppState>, Json(req): Json<AskRequest>)
    -> Sse<impl Stream<Item = Result<Event, Infallible>>>
{
    let stream = stream::try_unfold(
        (state, req, false),  // (state, sent_flag)
        move |(state, req, sent)| async move {
            if sent { return Ok(None); }  // stream ended

            // Check preconditions INSIDE the unfold — not in separate return branches.
            if state.advisor.is_none() {
                let event = Event::default().data(r#"{"error":"disabled"}"#).event("error");
                return Ok(Some((event, (state, req, true))));
            }
            // ... generate response ...
            let event = Event::default().data(json!({"done":true}).to_string());
            Ok(Some((event, (state, req, true))))
        },
    );
    Sse::new(stream)
}
```

Key points:
- All error paths emit events through the same `try_unfold` stream.
- Use a `sent` boolean flag to track whether the stream has emitted its final event.
- The stream state tuple must be uniform across all branches.

## Adding a Live Quote Endpoint

`/api/quote` returns `{symbol, price, prev_close, change, change_pct, timestamp}`.
It parses Yahoo Finance's `meta` block from the chart endpoint — no OHLCV history needed:

```rust
// engine/src/data/yahoo.rs — fetch_quote()
pub async fn fetch_quote(symbol: &str) -> Result<Quote> {
    let url = format!("{REST_URL}/{symbol}?interval=1d&range=5d");
    let meta = &response["chart"]["result"][0]["meta"];
    let price = meta["regularMarketPrice"].as_f64().context("missing regularMarketPrice")?;
    let prev_close = meta["chartPreviousClose"].as_f64()
        .or_else(|| meta["regularMarketPreviousClose"].as_f64())
        .unwrap_or(price);
    // ...
}
```

## Canvas Chart: Live Price + Prediction Cones

The `CandlestickChart.svelte` pattern — **live price comes bundled in the
chart response, not a separate fetch**:

```javascript
// Single round-trip: refresh chart every 30s (gets candles + live quote)
let livePrice = null;
let liveQuote = null;   // full quote for display
let isStale = false;
let chartTimer;

async function refreshChart() {
  const data = await fetchChart();
  candles = data.candles || [];
  sma = data.sma || [];
  isStale = !!data.stale;
  if (data.live_quote) {
    livePrice = data.live_quote.price;
    liveQuote = data.live_quote;
  }
  chartData.set(data);
  draw();
}

onMount(() => { chartTimer = setInterval(refreshChart, 30_000); });
onDestroy(() => { if (chartTimer) clearInterval(chartTimer); });
```

**Bundling the live quote avoids a race condition** between candle fetch
and quote fetch (they could be from different Yahoo requests). See
`references/chart-auto-refresh-trade-markers.md` for the full pattern
including the **staleness re-centering** fix.

```javascript
// Canvas drawing — live price dashed line + badge
ctx.setLineDash([4, 3]);
ctx.strokeStyle = '#7132f5';
ctx.lineWidth = 1;
ctx.beginPath();
ctx.moveTo(padL, liveY);
ctx.lineTo(w - padR, liveY);
ctx.stroke();
ctx.setLineDash([]);
// Badge
ctx.fillStyle = '#7132f5';
ctx.beginPath();
ctx.roundRect(bx, by, bw, 16, 3);
ctx.fill();
ctx.fillStyle = '#fff';
ctx.fillText(`${livePrice.toFixed(2)}`, bx + 4, by + 11);
```

```javascript
// Prediction cones from live price
const cones = [
  { label: '1D',  pred: preds.pred_1d,  bars: 1  },
  { label: '5D',  pred: preds.pred_5d,  bars: 5  },
  { label: '21D', pred: preds.pred_21d, bars: 21 },
];
ctx.setLineDash([4, 3]);
for (const cone of cones) {
  const targetPrice = livePrice * (1 + cone.pred);
  const endX = lastX + cone.bars * xStep;
  const endY = yScale(targetPrice);
  ctx.strokeStyle = cone.pred >= 0 ? '#149e61aa' : '#e5484daa';
  ctx.beginPath();
  ctx.moveTo(lastX, lastY);
  ctx.lineTo(endX, endY);
  ctx.stroke();
  // Dot at target + label
  ctx.beginPath(); ctx.arc(endX, endY, 3, 0, Math.PI*2); ctx.fill();
  ctx.fillText(`${cone.label} ${targetPrice.toFixed(2)}`, endX+2, endY+3);
}
ctx.setLineDash([]);
```

- Expand price range to include prediction targets before drawing
- Canvas `ctx.roundRect` — native in Chrome/Edge, fallback needed for Firefox
- 3 cones: 1D (1 bar), 5D (5 bars), 21D (21 bars) from `pred_1d`, `pred_5d`, `pred_21d`

### Cone visibility scales with candle count (pitfall)

The cone endpoint formula is `lastX + cone.bars * xStep` where `xStep = cw / n`
(`n` = candle count, `cw` = chart width). So a 21D cone over 500 candles is
`21/500 ≈ 4%` of canvas width — nearly invisible at default zoom.

**Root cause of "I can't see my prediction cones":** default `limit=500`
(`ChartQuery::limit()` in `engine/src/api/chart.rs`).

**Fix:** pass `?limit=` to `/api/chart` (backend clamps `[10, 1500]`):

```javascript
// frontend/src/lib/api.js
export async function fetchChart(limit = 90) {
  const res = await fetch(`${API_BASE}/chart?limit=${limit}`);
  ...
}
```

At `limit=90` (~6 months daily), the 21D cone gets ~23% of canvas width.
At `limit=60` (~3 months), it gets ~35%. 60-90 is the sweet spot for
making all three cones legible without sacrificing trend context.

**Supported query params** on `/api/chart`:
- `?limit=N` — number of candles (default 500, clamped [10, 1500])
- `?range=1y|2y|5y|max` — backfill window (default `5y`)

The backend always attempts a Yahoo backfill on every call regardless of
`range` — `range` only affects how much the backfill pulls if it runs.

### Cones can clip off the right edge (pitfall, separate from above)

The above pitfall makes cones too SMALL. A second bug makes them get
CUT OFF the canvas. When `xStep = cw / n`, the last candle's center is
at `padL + (n-1) * xStep + xStep/2`, which is still inside the canvas.
But cone endX = `lastX + cone.bars * xStep` for a 21D cone extends
**21 bar-widths past** the last candle — if `n` is small (e.g. 90),
that's ~23% of canvas width OFF the right edge. The target dot, label,
and dashed line all get drawn against `ctx.clearRect`-clipped
coordinates and never appear.

**Symptom:** "the chart shows data missing from the end" — actually
it's the 21D cone being drawn outside the visible canvas, while the
candles themselves render correctly.

**Fix:** reserve a future-bar buffer in the X-scale:

```javascript
// Reserve space for the longest cone so it lands inside the visible area
const futureBars = 21;   // longest cone (21D)
const xSlots = n + futureBars;
const xStep = cw / xSlots;

// Candles occupy slots 0..n-1; cones extend into slots n..n+futureBars
// Live price line + grid: draw only across the candle region (padL to padL + n*xStep)
// Cones + prediction targets: drawn at lastX + cone.bars * xStep, now safely inside canvas
```

Also update `futureRight` (used by the live-price pill) to be
`padL + xSlots * xStep` so the right-edge price tag stays anchored at
the actual right edge of the canvas rather than at the last candle.

### Redeploying the full stack after major backend changes

When the engine image **and** the inference image both change (new model
artifacts, new engine code, etc.), the safest sequence on compose v1.29.2 is:

```bash
cd /home/ubuntu/projects/MarketMoves
docker-compose -f deploy/docker-compose.yml down
docker-compose -f deploy/docker-compose.yml --env-file .env up -d
```

**Why `down` + `up` instead of a rolling engine-only restart?**

- `docker-compose up -d --no-deps engine` only recreates the engine. If the
  inference container also needs a new image or new model paths, the engine
  will talk to stale inference.
- Compose v1.29.2 is unreliable with `--force-recreate` and named volumes;
  `down` + `up` avoids the `KeyError: 'ContainerConfig'` bug.
- `down` does **not** remove named volumes (`data`, `models`,
  `caddy_data`, `caddy_config`) unless you pass `-v`. Your SQLite DB and
  model artifacts survive.
- `--env-file .env` ensures `FINNHUB_API_KEY`, `OPENROUTER_API_KEY`, and any
  other `${VAR:-}` variables in the compose file resolve from the workspace
  root `.env` rather than defaulting to empty.

**If `docker-compose down` complains about active endpoints**, the
containers were started from a different compose project or manually.
Stop and remove them explicitly before bringing the stack up:

```bash
docker stop mmn-engine mmn-inference mmn-proxy
docker rm mmn-engine mmn-inference mmn-proxy
docker-compose -f deploy/docker-compose.yml up -d
```

**After the stack is up:**

1. Wait for `mmn-inference` healthcheck (60s start period).
2. Wait for `mmn-engine` to report `loaded enabled models from trading_models registry count=2`.
3. If the `trading_models` table is empty (fresh DB or after `down -v`),
   register the models:
   ```bash
   # Copy the registration script into the container and run it
   docker cp scripts/register_models.sql mmn-engine:/tmp/
   docker exec mmn-engine sqlite3 /app/data/candles.db ".read /tmp/register_models.sql"
   # Verify
   docker exec mmn-engine sqlite3 /app/data/candles.db \
     "SELECT model_id, primary_symbol, inverse_symbol FROM trading_models"
   ```
4. Verify health:
   ```bash
   curl -sS http://localhost:9080/api/status | python3 -m json.tool
   curl -sS http://localhost:9080/api/models | python3 -m json.tool
   curl -sS "http://localhost:9080/api/strategy-config?model_id=nvda-v1" | python3 -m json.tool
   ```

### Per-symbol model bundles vs engine flat paths (2026-08-14)

The inference service can load per-symbol directories (`/models/QQQ/`,
`/models/SMH/`, `/models/XLF/`) and discover multiple ensembles. The engine,
however, still expects these flat files at `/models/` root for its own
normalization and the legacy single-model fallback path:

- `norm_stats_qqq_v1.json`
- `qqq_tcn_v1.pt`
- `qqq_lgbm_h1_v1.pkl`
- `qqq_lgbm_h5_v1.pkl`
- `qqq_lgbm_h21_v1.pkl`

**When extracting a per-symbol zip bundle, copy the QQQ artifacts to BOTH
places:** the per-symbol directory (for inference multi-symbol discovery)
and the `/models/` root (for the engine). If the root files are missing,
engine bootstrap logs `norm_stats error for QQQ/PSQ: No such file or directory`
and crashes.

**Also disable any exempted models in `trading_models`.** Even if a model's
files are deleted, an enabled registry row causes the engine to load it at
startup and fail when `norm_stats` is missing. Disable with:

```bash
sudo python3 - <<'PY'
import sqlite3
conn = sqlite3.connect('/var/lib/docker/volumes/deploy_data/_data/candles.db')
c = conn.cursor()
c.execute("UPDATE trading_models SET enabled = 0 WHERE model_id = 'nvda-v1'")
conn.commit(); conn.close()
PY
```

Then restart the engine.

### Multi-model inference limitation (RESOLVED 2026-08-07)

The inference service now supports per-symbol model loading via `MODELS_DIR`
directory scanning and z-score blending with prediction buffers. See
`references/multi-model-inference.md` for the implementation and
`references/z-score-blending.md` for the blending architecture.

See `references/external-llm-code-review.md` for the external LLM code review
pattern via OmniRoute proxy (Opus 5, Kimi K3), prompt-size limits, sub-pass
splitting, timeout handling, and prompt design for architectural audits.
See `references/opus5-review-2026-08-13-full-findings.md` for the complete
5-pass Opus 5 review findings (12 issues: 3 critical, 5 high, 4 medium)
covering strategy, scheduler, bridge, features, inference, and parity.

See `references/deferred-fixes-2026-08-14-resolved.md` for the concrete
resolution of deferred fixes 1-5 (VIX/TLT timestamp alignment, z-score
de-normalization, engine restart backfill, parity harness regeneration,
asymmetric short pred_5d filter) — execution order, signature cascades,
and the test commands that verified each fix.

## Docker Build &amp; Deploy

```bash
# 1. Rebuild (must be from workspace root — Dockerfile copies frontend/dist/ and engine/src/)
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .

# 2. Rolling restart (preserves named volumes)
docker stop mmn-engine && docker rm mmn-engine
docker-compose -f deploy/docker-compose.yml up -d engine

# 3. Wait for healthy (20-30s)
sleep 25 && docker ps --filter name=mmn-engine --format '{{.Status}}'
# Should show: "Up X seconds (healthy)"

# 4. Verify new endpoint — get internal container IP first (compose networks have no host ports)
IP=$(docker inspect mmn-engine --format '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}')
curl -s "http://${IP}:8080/api/quote"
# For frontend: curl -s "http://${IP}:8080/" | grep -o 'index-[a-zA-Z0-9]*.js'

**IMPORTANT: containers in a compose network have no host ports mapped by default.
curl localhost:8080 from the host will FAIL — always get the internal IP via docker inspect.**
Quicker alternative for ad-hoc verification: `docker exec mmn-engine curl -s
http://127.0.0.1:8080/api/...` — runs inside the container so it resolves the
listening port directly. This is the standard way to hit `/api/accuracy`,
`/api/status`, `/api/strategy-config` etc. without extracting the container
IP. Confirmed via SMH/XLF accuracy verification on 2026-08-16: host-side
`curl localhost:8080` returned exit 7 (connection refused), and `docker exec`
worked first try.
```

**Docker layer caching:** `docker build` auto-detects changed files in the build context. Running `docker build` then `docker-compose up -d` is sufficient — no cache-bust flags needed when `frontend/dist/` has changed.

Volume is named `mmn_data` — `docker rm` does NOT destroy it. Only `docker-compose down -v` or explicit `docker volume rm` destroys data.

### Verify new frontend bundle is actually being served

After rebuilding the engine image with new `frontend/dist/`, the browser
keeps caching the old JS bundle. Before hard-refreshing, confirm the new
bundle hash is what the engine serves:

```bash
# 1. Get the hash from the freshly-built dist
NEW=$(ls /home/ubuntu/projects/MarketMoves/frontend/dist/assets/index-*.js | grep -oE 'index-[A-Za-z0-9_-]+\.js')
echo "built: $NEW"

# 2. Hit the engine through the proxy and grep index.html for the asset
SERVED=$(curl -sS http://localhost:9080/ | grep -oE 'index-[A-Za-z0-9_-]+\.js')
echo "served: $SERVED"

# 3. They must match exactly. If not, the engine image wasn't rebuilt
#    or the proxy is caching — rebuild + restart, don't trust a "200 OK".
[ "$NEW" = "$SERVED" ] && echo "OK" || echo "MISMATCH"
```

This caught a real failure mode: rebuilt dist, but engine still serving
old bundle because `docker build` cached an older layer. Cheaper than
debugging why the chart "still looks the same" in the browser.

### Frontend is bundled into the engine image — single deploy path for UI changes

The `engine/Dockerfile` has a `frontend-build` stage that copies the
host's `frontend/dist/` (output of `npm run build`) into the engine
image at `/app/frontend/`. The engine's axum router serves the SPA
from there. This means there is **no separate frontend container in
prod** — every UI change requires the full loop:

```bash
# 1. Build the SPA on the host
cd frontend && npm run build           # → frontend/dist/index-*.js

# 2. Rebuild the engine image (copies frontend/dist/ into the image)
cd /home/ubuntu/projects/MarketMoves
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .

# 3. Restart the engine (NOT just the proxy) so the new image loads
docker-compose -f deploy/docker-compose.yml stop engine \
  && docker-compose -f deploy/docker-compose.yml rm -f engine \
  && docker-compose -f deploy/docker-compose.yml up -d --no-deps engine

# 4. Verify the served bundle hash matches what you built (see above)
```

**Pitfall — vite HMR / `npm run dev` is irrelevant to prod.** Running
`npm run dev` and seeing the change locally does NOT mean the engine
image picked it up. The engine container serves only the files baked
into the image at build time. A change visible in `localhost:5173`
(the vite dev server) will be invisible at `localhost:9080` (the
proxy → engine bundle) until the engine image is rebuilt and
restarted.

**Pitfall — `cd frontend && npm run build` without rebuilding the
engine image leaves the engine serving yesterday's bundle.** The
proxy caches aggressively; even after the rebuild, hard-refresh the
browser to bypass the browser cache for `index.html` itself (the
asset is content-hashed but `index.html` references it).

**Ordering:** ship the backend code change + the Svelte consumer
update + the `npm run build` + the `docker build` + the engine
restart in one commit and one deploy. Half-deploys are the
common failure mode (Rust says yes, Svelte says no, UI shows
`undefined`).

See `references/multi-model-registry.md` for the `trading_models` DB schema,
DB API, bootstrap resolver pattern, per-model main.rs loop, and the
ad-hoc verification harness template (12-check pattern).
See `references/strategy-config-detail.md` for borrow-checker workaround,
env var syntax for negative defaults, and the 10-step param checklist.
See `references/omniroute-llm-proxy.md` for the OmniRoute proxy setup,
Docker-to-host networking for LLM features, streaming vs non-streaming,
and OpenRouter fallback.
See `references/chart-auto-refresh-trade-markers.md` for the three-phase
chart refresh pattern (timer + WS-triggered refetch + prediction cone
field name mismatch fix) AND the **chart staleness re-centering** pattern
(bundle live_quote into chart response, override Y-axis when candles stale).
See `references/data-sources.md` for the full data source routing architecture
(Moomoo→Yahoo→CBOE→FRED→sentiment), env vars, DB schema, and API endpoint map.
See `references/moomoo-subprocess-pattern.md` for the Python subprocess
pattern (tokio::process::Command), OpenD TCP reachability check,
symbol prefix conversion, and Docker networking for OpenD inside containers.
See `references/moomoo-opend-install.md` for the headless OpenD install on VPS
(GUI vs server variant, `cfg_file` flag, XML credentials, systemd daemon,
Docker host-gateway networking).
See `references/multi-model-inference.md` for the single-model inference
limitation and the path to per-model inference services.
See `references/multi-model-inference-limitation.md` for the runtime symptom
(NVDA predictions blank), the `equity_predictions` unique-constraint fix, and
three solution paths (per-model containers, multi-model service, Python backfill).
See `references/z-score-blending.md` for the z-score blending architecture
with prediction buffers, per-model bias analysis, and the NVDA bearish-bias
root cause.
See `references/backfill-predictions.py` for the parametrized Python script that
loads a trained model bundle, fetches candles from the SQLite DB, computes
8-dim features, normalizes, and backfills predictions into `equity_predictions`.
See `references/deploy-multi-model-2026-08-06.md` for the full redeploy
recipe after the NVDA multi-model + sentiment overlay work, including
model registration, norm_stats path fixes, and compose v1 env-file handling.

## Options Momentum Engine (planned feature, designed 2026-08)

A Freqtrade-inspired options module was designed and planned in a full

**Implementation status (as of 2026-08-19):**
- P1: Tape recorder (parquet schema, quota accounting, OpenD integration) ✅
- P2: Synthetic backtester (BSM pricing, premium synthesis) ✅
- P3: Execution core ✅
  - P3.1: ExitArbiter priority table ✅
  - P3.2: Hardcoded overrides (DTE, delta drift, earnings blackout) ✅
  - P3.3: Staged exit ladder (3 stages with timers, partial-fill loop-back) ✅
  - P3.4: Trailing stop with hysteresis (0.5 × ATR recovery band) ✅
  - P3.5: Circuit breaker (3 trigger types, halt/resume logic) ✅
  - P3.6: Reconciliation (orders + positions with 1e-6 tolerance) ✅
  - P3.7: Write-ahead intent log ✅
  - P3.8: Paper executor with staged ladder ✅
- P4: Strategy layer ✅
  - P4.1: Macro risk gate (VIX + calendar blackout) ✅
  - P4.2: Chain selector (DTE/delta/liquidity) ✅
  - P4.3: Position sizing (formula + caps) ✅
  - P4.4: Entry executor (2-stage ladder) ✅
  - P4.5: Entry integration (pre-entry guards) ✅
- P5: Hyperopt loop & promotion pipeline ✅
  - P5.1: Nightly scheduler (timezone-aware, post-market window) ✅
  - P5.2: Optimizer (walk-forward, embargo, grid/random search) ✅
  - P5.3: Stability check (neighborhood perturbation, degradation ratio) ✅
  - P5.4: Candidate store (versioned snapshots, equity-scoped) ✅
  - P5.5: Promotion pipeline (state machine with evidence gates) ✅
  - P5.6: Tape replay validation ✅
- P6: DB persistence + wiring (in progress)
  - P6.1: DB-backed candidate store with equity scoping ✅
  - P6.2: DB-backed promotion pipeline ✅
  - P6.3: Timezone-aware nightly runner ✅
  - P6.4: Options scheduler (separate tokio task) ✅
  - P6.5: API endpoints (equity-scoped) ✅
  - P6.6: Integration test ✅
- P7+: Live integration, UI/dashboard — pending

**Key P6 patterns:**

**Axum route type privacy pitfall:** When adding routes to `api/mod.rs`, all request/response structs in the handler module must be `pub` or you get "private type" compile errors. The route macro exposes the handler's types to the router.

**Hyperopt API gotchas:**
- `CandidateSnapshot` fields: `version_id` (not `id`), `strategy_family` (not `strategy`), `status: CandidateStatus` (enum, not String — use `.as_str().to_string()`)
- `PromotionPipeline::promote(&store, &id, &evidence)` — takes a `CandidateStore` ref, not a pool
- `PromotionStage` enum has no `as_str()` method — use `{:?}` debug format for display
- `PromotionEvidence` fields: `n_trades: usize`, `ic: f64`, `sharpe: f64`, `days_observed: usize`

**DB helpers added to `engine/src/db.rs`:**
- `count_strategy_versions(pool, equity)` → `Result<i64>`
- `count_strategy_versions_by_status(pool, equity)` → `Result<HashMap<String, i64>>`
- `strategy_versions` table has `equity TEXT NOT NULL DEFAULT 'QQQ'` column

**Test counts (2026-08-19):** 44 hyperopt tests (40 unit + 4 integration), 295 total passing

**Commits:** `2790884` (P6.4), `5985a6c` (P6.5), `4efb3ef` (P6.6)

**Timezone-aware scheduling:**
```rust
pub struct SchedulerConfig {
    pub timezone_offset_hours: i32,  // +8 for Malaysia, -5 for ET
    pub market_open_local: NaiveTime,
    pub market_close_local: NaiveTime,
    // ...
}
// Convert local → UTC internally: offset_seconds = (timezone_offset_hours as i64) * 3600
```

**DB-backed async testing:**
```rust
async fn test_store() -> CandidateStore {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    // Create schema in test setup
    sqlx::query("CREATE TABLE ...").execute(&pool).await.unwrap();
    CandidateStore::new(pool)
}
```

**Version ID collision prevention:** Use atomic counter alongside timestamp for rapid test execution:
```rust
let seq = self.counter.fetch_add(1, Ordering::SeqCst);
let version_id = format!("v{}_{}_{}", equity, now, seq);
```

**Equity-scoped design:** Candidates keyed by (equity, strategy_family, version_id). API endpoints: `GET /api/hyperopt/:equity/candidates`, `POST /api/hyperopt/:equity/promote/:id`.

**DB schema:** `strategy_versions` table has `equity` column (added via migration). Status stored as TEXT ("NEW", "STABLE", "PAPER", "MICRO", "LIVE", "RETIRED").

**Test counts:** P1-P4: 86 tests, P5: 34 tests, P6.1-P6.3: 40 tests total

**Pitfalls:**

**Timezone offset type casting:** When converting timezone offset to seconds, cast `i32` to `i64` before multiplication:
```rust
let offset_seconds = (self.timezone_offset_hours as i64) * 3600;
```
Without the cast, you get `expected i64, found i32` errors when subtracting from `local_seconds` (which is `i64`).

**Midnight-wrapping market hours:** When market hours span midnight (e.g., Malaysia 9:30 PM - 4:00 AM), handle the wrap in `is_market_hours()`:
```rust
if market_open_utc > market_close_utc {
    time_of_day >= market_open_utc || time_of_day < market_close_utc
} else {
    time_of_day >= market_open_utc && time_of_day < market_close_utc
}
```

**Scheduler test expectations:** When testing `next_run_time()`, if current time is past `earliest_start`, the function returns tomorrow's date, not today's. Adjust test expectations accordingly.

See `references/p3-execution-patterns.md` for detailed patterns and code examples.
grill-me session. Design is LOCKED (21 decisions) — do not re-decide or
"improve" these during implementation without a design discussion:
ExitArbiter priority, staged exit ladder, circuit breaker, sunset hot-swap,
synthetic-premium + tape validation, evidence-gated promotion, risk layer
outside strategy versions. Canonical plan:
`.hermes/plans/options-momentum-engine/PLAN.md` on branch
`feature/options-momentum-engine`. Quick-recall summary:
`references/options-momentum-engine-design.md`. Futu options API quota
facts (the binding constraint on everything options-related):
`references/futu-options-api-limits.md`.

**Phase 0 status (2026-08-17):** In progress. Quota & OPRA probe script written:
`.agents/skills/moomooapi/scripts/quote/probe_option_quota.py` — verifies
account quota tier and greeks availability. See
`references/options-engine-phase0-probe.md` for usage and interpretation.
Quota tier verification is the blocker for Phase 0 sign-off.

**Phase 3 status (2026-08-19):** ExitArbiter and risk layer modules implemented.
P3.1-P3.5 complete (exit_arbiter, overrides, staged_ladder, trailing_stop,
circuit_breaker). P3.6 reconciliation in progress.

### ExitSignal contract (CRITICAL)
All exit sources (overrides, trailing_stop, circuit_breaker, etc.) emit
`ExitSignal` from `exit_arbiter/mod.rs`. The struct has:
- `source: ExitSource` (enum, NOT String)
- `priority: u8`
- `reason: String`
- `timestamp: DateTime<Utc>` (REQUIRED)

**Pitfall (2026-08-19):** When creating a new exit module, do NOT assume the
struct shape. Always check `exit_arbiter/mod.rs` for the current definition.
Common mistakes:
- Using `source: "ModuleName".to_string()` instead of `source: ExitSource::ModuleName`
- Omitting `timestamp` field
- Using wrong priority value (check ExitSource enum for correct priority)

**Pattern:** Import both types:
```rust
use crate::options::exit_arbiter::{ExitSignal, ExitSource};
```

## Multi-Model Dashboard UI

When the engine runs multiple models (e.g. `qqq-v1`, `nvda-v1`), the
dashboard must partition telemetry by `model_id`. The flat singleton stores
(`status`, `predictions`, `chartData`, etc.) are kept for backward
compatibility but proxy to the active model's slice.

See `references/multi-model-dashboard-migration.md` for the per-model
store migration checklist, backend schema and migration changes required
for per-model status, the per-symbol endpoint contract, and the recurring
UI bugs (status symbol not updating, chart not switching, partial
ModelHealth changes, direction accuracy redefinition, global trade history,
and per-model strategy config guidance).

## Data Pipeline & Ingestion Cadence

### Architecture: ingestion supervisor vs scheduler

Two separate loops, NOT the same thing:

```
run_equities_ingestion (data/mod.rs)
  └─ Fetches candles from Moomoo/Yahoo → writes to equity_candles table
  └─ Runs on wall-clock cadence: 22:00 UTC (post-close) + 07:00 UTC (pre-open)
  └─ Does NOT run inference, does NOT make trading decisions

EquityScheduler (scheduler.rs)
  └─ Polls equity_candles every 5 min for new candle timestamps
  └─ When last_candle_ts > last_processed_ts → runs inference + strategy
  └─ Does NOT fetch candles — only processes what's already in the DB
```

**Critical implication:** if the ingestion supervisor has not written a new candle
to the DB, the scheduler will never produce new predictions — no matter how
often it polls. The scheduler is downstream of ingestion.

### Dual-cadence ingestion (committed f253b5d)

The old ingestion ran ONCE every 24h starting from midnight UTC. The timing gap:

```
Market close:    21:30 UTC (Fri)
Ingestion runs:  00:00 UTC (Sat)  ← 2.5h later, may miss late-arriving Friday close
Next run:        00:00 UTC (Sun)  ← no new data over weekend
Next run:        00:00 UTC (Mon)  ← still processing Friday close, not Monday's
```

On Monday morning before market open (09:30 ET = 13:30 UTC), the last candle
in the DB is still Friday's close → staleness shows ~72h. This is **expected
behavior given the cadence**, not a bug.

The fix replaces the single 24h loop with two wall-clock-gated runs:

- **22:00 UTC** (±30 min) — post-market catchup, ~30 min after US close
- **07:00 UTC** (±30 min) — pre-market safety net, ~2.5h before US open

Code in `engine/src/data/mod.rs` — `run_equities_ingestion()`. Uses
`chrono::Timelike` for `.hour()` and `.minute()` on `Utc::now()`.

### Staleness diagnosis checklist

When the dashboard shows high staleness (e.g. 72h):

1. **Check the last candle timestamp:**
   ```bash
   # From the running container
   docker exec mmn-engine sqlite3 /app/data/candles.db \
     "SELECT datetime(MAX(ts), 'unixepoch') FROM equity_candles WHERE symbol='QQQ'"
   ```
2. **Cross-reference against market hours:** is the market closed? Friday
   close → Monday morning is ~72h — expected. Check `engine/src/market_hours.rs`
   for the `MarketCalendar` logic.
3. **Check if ingestion actually ran:** `docker logs mmn-engine 2>&1 | grep -i 'top.up\|backfill\|ingestion'`
4. **Check if the scheduler is processing:** `docker logs mmn-engine 2>&1 | grep -i 'scheduler\|new candle\|inference'`
5. **Differentiate data staleness from prediction staleness:**
   - Data staleness = age of last candle in DB (driven by ingestion)
   - Prediction staleness = age of last inference run (driven by scheduler)
   - Both can be stale for different reasons. Fix the right one.

### Weekend staleness is normal

QQQ only trades Mon-Fri. A Friday close at 21:30 UTC with no new data until
Monday 13:30 UTC is ~64 hours of staleness. Add a few hours for ingestion
timing and 72h is exactly what you'd expect on a Monday morning. The dashboard
is showing the truth — the system is not broken.

**Do NOT treat weekend staleness as a bug.** The fix is the dual-cadence
ingestion, which ensures the Friday close gets captured at 22:00 UTC and the
scheduler can run inference before Monday open. But staleness will still show
~48-72h on Monday morning because the last candle IS from Friday.

## Chart Staleness — CRITICAL Pitfall (updated 2026-07-31)

**The single most common chart bug in this project.** Three distinct failure
modes, each requiring a different fix:

### Failure mode A: wrong ORDER BY on `fetch_recent_equity_candles` (fixed `8a4aba5`)

`engine/src/db.rs` — `fetch_recent_equity_candles` MUST use `ORDER BY ts DESC`
(returning newest-first) because it uses `LIMIT`. If it used `ORDER BY ts ASC`
with `LIMIT 10`, it would return the OLDEST 10 candles (e.g. Aug 2021),
not the newest. The chart would show prices at $360 when live QQQ is $688.

**The fix (committed `8a4aba5`):**
```rust
// db.rs — fetch_recent_equity_candles
ORDER BY ts DESC   // newest first, then limit — CORRECT
LIMIT ?2

// chart.rs — reverse for rendering (ascending ts order)
let candle_dtos: Vec<_> = candles.iter().rev().map(dto).collect();
```

The correct function for the **feature pipeline** is `fetch_equity_candles_asc`
(which does `ORDER BY ts ASC`). Do NOT swap these. If you see chart prices
from 2021, check whether `fetch_recent_equity_candles` accidentally uses ASC.

There is also a **separate** `candles` table (legacy crypto) and
`equity_candles` table (Wave A equities). The chart API reads from
`equity_candles`. `candles` is the retired BTC/ETH pipeline.

### Failure mode B: correct candles but stale live price

The candle DB can contain genuinely stale data (e.g. market closed Friday,
backfill fails Saturday morning) while the live quote is current ($688).
Chart shows prices at $360 — but with fix A applied the candles are
correct, yet the price axis still doesn't align.

**Defense:** Chart response ALWAYS includes a `live_quote` field + `stale`
boolean (committed `8a4aba5`). The frontend always re-centers Y-axis
on the live price when `stale === true`. Full implementation in
`references/chart-auto-refresh-trade-markers.md`.

### Failure mode C: separate live quote fetch causes race + double round-trip

Fetching live quote via a separate `/api/quote` call after the chart fetch
creates two problems: (1) two network round-trips, (2) quote and candles
may come from different Yahoo requests with slightly different timestamps.

**Fix (committed `8a4aba5`):** `live_quote` is bundled into the
`/api/chart` response itself — one round-trip, always consistent. The
separate `/api/quote` endpoint still exists (`engine/src/api/quote.rs`) for
UI components that need just the quote without the candle history, but the
chart component always reads `live_quote` from the chart response, never
from a separate fetch.

## Position & Trade Lifecycle

### The equity_trades table vs the legacy trades table

The project has **two** trade tables, and confusing them is a high-impact bug:

| Table | Phase | Schema | Status |
|---|---|---|---|
| `trades` | Legacy crypto (BTC/ETH) | `(id, ts, side, price, qty, fee, realized_pnl)` | Retired |
| `equity_trades` | Wave A equities (QQQ/PSQ) | `(id, ts, symbol, side, qty, price, ...)` | Active |

`db::fetch_entry_trade_price()` reads the **old** `trades` table. Since no crypto
trades exist, it always returns `None`. This caused `entry_price: null` in the
status response even when the engine had a live Short position with a real PSQ
entry trade. The legacy function has been removed.

**Fix:** `db::fetch_equity_entry_trade_price(pool, symbol)` queries `equity_trades`
with `WHERE symbol = ?1 AND side = 'buy' ORDER BY id DESC LIMIT 1`. Always use
the `equity_` prefix for any trade-related DB function in the equity pipeline.
The `side = 'buy'` filter is critical — without it, a partial close (sell) would
shadow the entry and return the wrong price as "entry".

### save_position() ordering — MUST be AFTER trade recording

The scheduler writes position state to the `signal_state` table via
`db::save_position()`. This is a singleton row that persists across restarts.

**Bug:** `save_position()` was called BEFORE `set_target_position()`. If the trade
execution failed (paper fill error, live API error), the position was already
persisted as "Short" — so the engine restarted claiming a position it never
actually entered. The trade was never recorded in `equity_trades`.

**Fix:** Move `save_position()` into the `Ok` path **after** `set_target_position`
completes and fills are logged to `equity_trades`. Only persist the position
when the trade is confirmed.

```rust
// CORRECT: save AFTER the trade is confirmed
match result {
    Ok(fills) => {
        for fill in &fills {
            db::record_equity_trade(pool, &symbol, fill).await?;
        }
        db::save_position(pool, new_pos.as_i64()).await?;  // HERE
    }
    // ...
}
```

### Frontend trade display — WebSocket-only is NOT enough

TradeHistory and PnLEquityCurve were populated solely by WebSocket `TradeFill`
events. On page load, there is no initial fetch of existing trades — the stores
start empty and only grow when new trades fire (which may have happened days ago).

**Fix:** Both components fetch existing trades on mount:
- `Dashboard.svelte` calls `fetchEquityTrades('*', 200)` and pushes into the
  `trades` store (mapping `ts` → `time` for WebSocket compatibility).
- `PnLEquityCurve.svelte` calls `fetchEquityTrades('*', 500)` and reconstructs
  cumulative PnL history from the trade list.

### `symbol=*` wildcard for cross-symbol trade queries

`GET /api/equity/trades?symbol=*` returns trades across ALL symbols (QQQ + PSQ).
Backend uses `db::fetch_recent_all_equity_trades()` which omits the `WHERE symbol`
clause. The response's `symbol` field reports `"*"`.

Use this when the dashboard needs a unified trade history (e.g. TradeHistory
band at the bottom). Individual symbol queries still work: `?symbol=QQQ`,
`?symbol=PSQ`.

### Short position PnL for inverse ETFs (PSQ)

PSQ is a -1x QQQ daily inverse ETF. The PnL formula:

```
PSQ_return ≈ -(QQQ_return)
PSQ_current ≈ PSQ_entry * (1 - QQQ_return)
PnL = PSQ_entry - PSQ_current ≈ PSQ_entry * (QQQ_current / QQQ_entry - 1)
```

- QQQ rose → QQQ_current > QQQ_entry → PnL > 0 → short **loses** money
- QQQ fell → QQQ_current < QQQ_entry → PnL < 0 → short **wins** money

The old formula treated PSQ as tracking QQQ positively (direct ratio scaling),
which inverted the sign. See `references/psq-inverse-etf-pnl.md` for the full
derivation and the `fetch_equity_close_at_ts` helper used to get QQQ close at
entry time.

### StatusResponse serialization MUST be snake_case

The entire frontend ecosystem (StatusPanel.svelte, websocket.js, stores.js)
reads status fields as **snake_case**: `entry_price`, `realized_pnl`,
`unrealized_pnl`, `last_close`, `pred_1d`, `pred_5d`, `pred_21d`.

**NEVER** add `#[serde(rename_all = "camelCase")]` to `StatusResponse`.
The frontend will get `undefined` for every field, and `fmt()` renders `'—'`
(em-dash). All status values appear blank/black in the UI.

Also: `entry_price`, `unrealized_pnl`, and `last_close` must be `f64`
(not `Option<f64>`) — the frontend expects always-present numbers, defaulting
to `0.0` for Flat positions. The original struct had them as non-optional.

### StrategySnapshot must include all 7 fields

The `strategy` field in `StatusResponse` is `StrategySnapshot` (not `Option`).
The struct must include: `entry_threshold`, `exit_threshold`, `sma_window`
(usize), `pred_5d_filter`, `enable_shorting`, `short_entry_threshold`,
`short_exit_threshold`. The frontend's StrategyConfigPanel reads these
directly from the status payload.

### Status handler refactor checklist

When modifying `handle_status()` or the `StatusResponse` struct, these are
the recurring pitfalls (found in code review of commit aabf8dc):

1. **Changing field types breaks tests.** If you change a field from
   `Option<f64>` to `f64`, every test that calls `.is_none()` or `.unwrap()`
   on that field must be updated to `== 0.0` or direct comparison. Run
   `cargo test --lib -- api::tests` after any struct change.

2. **`staleness_secs` None must be `u64::MAX`, not `0`.** When no candle
   exists, `0` means "fresh as possible" — the frontend's `stale > 120`
   check would show no-data as NOT stale. `u64::MAX` correctly signals
   extremely stale.

3. **`pred_1h_approx` / `pred_5h_approx` must use `/ 6.5`, not `/ 24`.**
   Daily predictions divided by 6.5 (trading hours/day) give the per-hour
   rate. Dividing by 24 (calendar hours) understates the intraday rate by
   ~3.7x. The `prediction_to_dto` function in `predictions.rs` uses 6.5 —
   `status.rs` must match.

4. **`fetch_equity_close_at_ts` must use floor match, not exact match.**
   `WHERE ts = ?2` returns None if the trade timestamp doesn't exactly match
   a candle row. Use `WHERE ts <= ?2 ORDER BY ts DESC LIMIT 1` so a slight
   timestamp mismatch still finds the closest prior candle.

5. **Consolidate `strategy_params` RwLock reads.** The handler needs
   `sma_window` for the SMA calculation and all params for the snapshot.
   Read the lock ONCE, compute both in the same scope. Don't acquire the
   lock three times in one handler.

6. **Remove dead code after refactoring.** When replacing a function (e.g.
   `fetch_entry_trade_price` → `fetch_equity_entry_trade_price`), delete the
   old function. Dead functions get copy-pasted into new code and cause
   confusion months later.

7. **Remove `console.log` debug statements before deploy.** Svelte
   `on:input` handlers fire on every keystroke — a `console.log` in
   `markDirty()` spams the browser console.

### Test data must use equity_trades, not legacy trades table

Tests that set up trade data for status/PnL assertions must call
`db::insert_equity_trade(pool, symbol, ...)` — NOT the legacy
`db::insert_trade(pool, ...)`. The status handler reads from `equity_trades`
(see "equity_trades table vs legacy trades table" above). A test that
inserts via `insert_trade` writes to the empty `trades` table; the handler
will never see the data and `entry_price` will be `0.0` instead of the
expected value. This was the root cause of test
`status_reports_unrealized_pnl_for_open_position` failing after commit
aabf8dc changed the handler to read from `equity_trades`.

### Shared struct field names must match the semantics that populate them (caught 2026-08-16)

When two unrelated code paths share a struct (e.g. crypto `fetch_accuracy`
and equity `fetch_equity_accuracy` both returning `AccuracyStats`), the
struct field names must be **honest about the dominant use case, not the
first-written one**. The legacy `AccuracyStats` declared
`directional_1h/4h/24h` + `mae_1h/4h/24h`, but the equity query computes
1d/5d/21d and assigns them into those hourly-named fields. Renaming the
struct fields to `1d/5d/21d` for the equity semantics caused 12
`E0425: cannot find value` errors on the first build — the crypto block's
local var names were still `count_4h/dir_4h/sum_ae_4h` while the struct
fields became `directional_5d`. Symptom: the build error points at every
field assignment in the **equity** function, but the actual fix is in the
**crypto** function (map hourly locals onto the renamed daily fields, or
vice versa).

**Self-test before renaming a shared struct's fields:**
```bash
# Find all functions that construct this struct
grep -n "AccuracyStats {" engine/src/db.rs
# Then re-read every construction site — each must be updated for the
# rename, AND each must verify its local-var names match the new fields.
```
Don't trust `pub struct X { ... }` to tell you how many sites need
patching. `grep "AccuracyStats {"` lists every construction; each one is
a potential compile error.

### Renaming API response fields breaks the dashboard silently (caught 2026-08-16)

When you rename a field on a serialized Rust response struct
(e.g. `AccuracyStats::directional_1h` → `directional_1d`), the
**Rust build still passes**, all `cargo test` still passes, and the
**API continues to serve JSON** — but the Svelte frontend reads the
field by its old name via `accuracyData?.directional_1h`, gets
`undefined`, and renders `N/A`. No error, no warning, just silent UI
breakage. Caught on the dashboard showing `Dir. Acc. 1d: N/A` after
the `1h/4h/24h` → `1d/5d/21d` rename, even though `/api/accuracy`
worked perfectly via `curl`.

**Self-test before pushing any response-field rename:**

```bash
# 1. Find every frontend consumer of the renamed field
grep -rn "directional_1h\|directional_4h\|directional_24h\|mae_1h" frontend/src

# 2. After the backend rename, rebuild frontend and confirm the bundle
#    references the NEW field names (not just the absence of old ones):
cd frontend && npm run build
grep -o "directional_1d\|directional_5d\|directional_21d\|mae_1d" \
  dist/assets/*.js | sort | uniq -c
# Expected: one line per new field, zero lines for old fields.

# 3. Confirm the live bundle hash served by the proxy matches the
#    freshly-built one (see "Verify the served bundle hash" pitfall
#    below and the verify-served-bundle command in the
#    "Docker Build &amp; Deploy" section).
```

**The contract for any change touching response field names:**

The Rust rename, the Svelte `ModelHealth.svelte` field remapping, the
`api.js` `fetchAccuracy` signature update, and the engine image
rebuild all ship in **one commit / one deploy**. Partial deploys
leave the UI reading fields that the API no longer emits.

**Why this is worse than a normal compile error:** Svelte has no
type-checking on response shapes (no tRPC, no generated types from
the Rust struct). A field mismatch is a silent runtime `undefined`.
The "Look for the field in JS before pushing" grep above is the
only defense.

**Generic pattern:** any time a JSON field name changes in a Rust
response struct, search the frontend for the old name BEFORE
deploying. `grep -rn '<old_field_name>' frontend/src` must return
zero hits. If you find hits, update the Svelte consumer in the same
commit.

### Fresh-model backfill silently skips (caught 2026-08-16)

`EquityScheduler::run()` (engine/src/scheduler.rs ~line 119) gates the
backfill behind `if let Some(last_ts) = self.last_processed_ts`. For a
freshly enabled model in `trading_models`, `last_processed_ts` is `None`
(no rows in `equity_predictions`), so the entire backfill block is
skipped. The scheduler then waits for the *next* new candle — a model
enabled today gets exactly **one** prediction until tomorrow's market
candle lands. Directional-accuracy metrics over the new symbol stay
empty for weeks because every prediction needs `today + horizon` candles
to resolve.

**Fix (committed `03bd6a2`, then capped at `e130837`):** when
`last_processed_ts == None`, seed the backfill start from the earliest
candle in `equity_candles` (new `db::earliest_equity_candle_ts` helper),
capped by `BACKFILL_DAYS` env (default 90, threaded Config → scheduler).
For SMH/XLF on first deploy this was ~84m for full 1255-candle history;
the 90-day cap brings it to ~7m/symbol without loss in accuracy
usefulness (a 30-90d window has ≥20 resolved 1d samples per symbol after
one full market day post-cap).

**Cost on subsequent deploys** (model already has history):
`last_processed_ts == Some(...)`, so the cap is bypassed — only the
true outage window is processed. The cap only applies to the fresh-model
seed branch.

**Diagnostic if you suspect "model X has no accuracy":**
```bash
sudo python3 - <<'PY'
import sqlite3
db = '/var/lib/docker/volumes/deploy_data/_data/candles.db'
c = sqlite3.connect(db).cursor()
for (sym, n) in c.execute(
    "SELECT symbol, COUNT(*) FROM equity_predictions GROUP BY symbol"
):
    print(sym, n)
PY
```
A symbol with `n == 1` (just today's row) is the symptom.

### Fresh-model backfill produces IDENTICAL predictions for every candle (caught 2026-08-16)

**This is the bug *behind* "fresh-model accuracy is still empty after
the cap fix above" — distinct from the silent-skip pitfall.** Symptom:
backfill completes ("backfill complete symbol=SMH count=62") but
accuracy is still empty AND persisted predictions look like 9 byte-identical
rows:
```
$ sqlite3 candles.db "SELECT candle_ts, pred_1d FROM equity_predictions WHERE symbol='SMH' ORDER BY candle_ts"
1779111000 | 0.05390121032854039
1779197400 | 0.05390121032854039   ← same pred for different candle_ts
1779283800 | 0.05390121032854039
...
```
Root cause: `EquityScheduler::process(candle_ts)` was calling
`db::fetch_equity_candles_asc(&pool, &symbol, fetch_count)` which ignores
`candle_ts` and always returns the **most recent N candles** (`ORDER BY ts DESC LIMIT ?2` subquery).
Every historical `candle_ts` was therefore fed the **same current
feature window**, producing the same prediction. The `candle_ts`
parameter only controlled what row got the `INSERT`, not what features
the model saw.

**Fix (committed `d566a1b`):** new
`db::fetch_equity_candles_asc_before(pool, symbol, end_ts, limit)` helper
adds `AND ts <= ?3` to the inner DESC subquery. Use it in
`process(candle_ts)` so backfill replays each historical candle with
the features actually available at that time:

```rust
let candles = db::fetch_equity_candles_asc_before(
    &self.pool, &self.symbol, candle_ts, fetch_count,
).await?;
```

Live path behaviour is unchanged (latest candle's `candle_ts` ≈ now,
and `ts <= now` includes all rows). Only backfill uses the new bound.

**Diagnostic for "backfill ran but rows are identical":**
```bash
sudo python3 - <<'PY'
import sqlite3
db = '/var/lib/docker/volumes/deploy_data/_data/candles.db'
c = sqlite3.connect(db).cursor()
print("distinct pred_1d values per symbol:")
for (sym,) in c.execute("SELECT DISTINCT symbol FROM equity_predictions"):
    n = c.execute(
        "SELECT COUNT(DISTINCT pred_1d) FROM equity_predictions WHERE symbol=?",
        (sym,)).fetchone()[0]
    total = c.execute(
        "SELECT COUNT(*) FROM equity_predictions WHERE symbol=?",
        (sym,)).fetchone()[0]
    print(f"  {sym}: {n} distinct / {total} rows")
PY
```
A symbol with 1-3 distinct `pred_1d` values across 50+ rows is the
symptom. (Some clustering near 0 from the existing
`pred_1d.abs() < 1e-10` skip path is expected and unrelated — focus
on the *spread* of distinct non-zero values.)

**Pattern lesson — add new fetch helpers, don't mutate existing
ascending ones.** `fetch_equity_candles_asc` has live callers in
`equity.rs` and `status.rs` that deliberately want the *current*
window. Renaming it breaks live callers; the new bounded helper goes
alongside. Same architectural lesson as the "fetch_equity_candles_asc
returns OLDEST N not latest N" pitfall below — every candle-fetch
helper has subtly different ordering semantics; adding new ones is
cheaper than auditing every caller when refactoring.

### Backfill verification: prefer UPSERT over DELETE to regenerate stale rows (caught 2026-08-16)

When backfill produces degenerate rows (e.g. the "identical
predictions" bug above), the instinct is to `DELETE FROM
equity_predictions WHERE symbol=...` and restart. On this VPS that
DELETE is **blocked by the `terminal` safety guard** because it's a
destructive write to the live DB. The user has to manually approve.

**Before falling back to DELETE, check whether the insert is already
an UPSERT.** `db::insert_equity_prediction` (engine/src/db.rs ~line 1603)
already has `ON CONFLICT(symbol, candle_ts) DO UPDATE SET pred_1d=...,
pred_5d=..., ...`. This means a rebuild + restart will **overwrite
the degenerate rows in place** when the fixed `process(candle_ts)` runs
the bounded fetch (`fetch_equity_candles_asc_before`) and re-INSERTs
with distinct predictions. No DELETE, no guard block, no manual
consent.

The gotcha: `last_processed_ts` recovery on restart pulls the MAX
candle_ts from existing rows. If ANY row exists (even degenerate ones),
the backfill skips from that ts *forward* — the historical rows below
it are NOT overwritten. So UPSERT only regenerates the *forward*
window, not the stale historical rows. The stale rows become a small
permanent tail (9 of 62 in the SMH/XLF case). Acceptable trade-off:
they don't materially distort accuracy (oldest dates, few rows), and
no destructive op is needed.

**When DELETE is unavoidable:**
- The user explicitly approved the destructive op AND the safety guard
  was bypassed, OR
- The architectural fix is so invasive that UPSERT can't be relied on
  (e.g. the schema needs a new column that wasn't back-filled).

**Verification after the UPSERT-based restart:**
```bash
# Wait ~2 min for the backfill (62 candles × ~2s inf/symbol = ~2min)
sleep 150
# Confirm rows were actually overwritten
sudo python3 - <<'PY'
import sqlite3
db = '/var/lib/docker/volumes/deploy_data/_data/candles.db'
c = sqlite3.connect(db).cursor()
for (sym,) in c.execute("SELECT DISTINCT symbol FROM equity_predictions"):
    n = c.execute(
        "SELECT COUNT(DISTINCT pred_1d) FROM equity_predictions WHERE symbol=?",
        (sym,)).fetchone()[0]
    total = c.execute(
        "SELECT COUNT(*) FROM equity_predictions WHERE symbol=?",
        (sym,)).fetchone()[0]
    print(f"  {sym}: {n} distinct / {total} rows")
PY
# A symbol with <10 distinct pred_1d across 50+ rows is still degraded.
```

### Config tests fail from shell env pollution

`engine/src/config.rs` tests like `defaults_load_when_env_unset` call
`Config::from_env()` which reads real environment variables. If the shell
has `SMA_WINDOW=40` exported (from `.env` sourcing or prior commands), the
test expects the default `200` but gets `40` — failure. This is NOT a code
bug; it's test isolation. 22 config tests fail in a polluted shell.

**Workaround:** run config tests with a clean environment:
```bash
env -i HOME=/home/ubuntu PATH=/usr/bin:/bin cargo test --lib -- config
```
Or just run the API/strategy tests that actually matter for code changes:
```bash
cargo test --lib -- api::tests strategy::tests::compute_sma
```

## Frontend-Backend API Path Conventions (Pitfall)

The backend axum router mounts routes under `/api/`. The Svelte frontend
defines `const API_BASE = '/api'` and then **does not repeat** the prefix:

```javascript
// ✅ CORRECT — backend already has /api, API_BASE already provides it
const res = await fetch(`${API_BASE}/advisor/briefing`);

// ❌ WRONG — produces /api/api/advisor/... → HTML 404 → 'Unexpected token <'
const res = await fetch(`${API_BASE}/api/advisor/briefing`);
```

**Symptom:** `SyntaxError: Unexpected token '<', "<!DOCTYPE ..." is not valid JSON`
— browser receives an HTML 404 page and tries to parse it as JSON.

**Check:** `grep -rn '/api/api' frontend/src/` should return nothing. Every
`fetch` that starts with `${API_BASE}/` must not embed a second `/api`.

Also applies to any `.svelte` or `.js` that calls backend endpoints.
`API_BASE = '/api'` is the single source of truth for the base path.

## Frontend Design System

The dashboard uses a Kraken-inspired dark theme adapted from the
`popular-web-designs` skill. CSS custom properties are defined in
`:global(:root)` inside `App.svelte` — all components reference
`var(--token)` instead of hardcoded hex. Canvas chart code uses raw hex
(canvas API doesn't resolve CSS variables). See the
`svelte-frontend-development` skill's `references/design-system-restyling.md`
for the full token mapping and migration checklist.

Key colors: accent `#7132f5` (Kraken purple), green `#149e61`, red
`#e5484d`, surface `#15161e`, border `#252631`. Font: Inter via Google
Fonts. Radii: 12px / 8px / 6px.

## Dashboard Panel Layout

The `Dashboard.svelte` view is a 12-column grid. The current
**layout B (TradingView-Lite)** arrangement — see
`references/dashboard-layout.md` for the full grid map, column spans,
and responsive breakpoints:

```
Row 1: [ Chart (8/12) ][ StatusPanel (4/12) ]
Row 2: [ PnL Equity Curve — full width ]
Row 3: [ Features (4) ][ ModelHealth (4) ][ Strategy (4) ]
Row 4: [ TradeHistory — full width, horizontal band at the bottom ]
```

### De-duplication rule for shared metrics (UX)

When the same metric (staleness, websocket state, position, etc.) is
shown in more than one panel, it looks like noise — the user has to
read both and reconcile. The rule:

- **Pick one home** per metric. Convention:
  - **ModelHealth** owns: data staleness, websocket connection, prediction
    accuracy (dir-acc, MAE), resolved count — anything about model/feed quality.
  - **StatusPanel** owns: mode, symbol, position, entry, last close,
    predictions (raw values, not accuracy), realized/unrealized PnL —
    anything about the current trade.
  - **TradeHistory** owns: trade list with realized PnL per trade
    (not aggregated; aggregated goes in PnLEquityCurve).
- **When moving a metric**, delete the old copy from the source panel
  (markup + reactive + CSS + import) — don't leave it as "shown in two
  places, but we're just removing it from this one" comments.

### Staleness display convention

Always render `staleness_secs` from the status payload as **hours**, not
seconds. A 47,000-second value (13h) is unreadable; `13.0h` is scannable.

```svelte
$: staleness = $status?.staleness_secs ?? 0;
$: stale = staleness > 120;             // 2-minute threshold unchanged
$: stalenessHours = (staleness / 3600).toFixed(staleness < 3600 ? 1 : 0);
<span>{stalenessHours}h</span>
```

Threshold semantics (the `stale > 120` check) stay in seconds — only the
display unit changes.

## Repository Cleanup Workflow

When cleaning stale code/plans from the repo, follow this phased approach
to avoid data loss and unnecessary approval friction.

### Phase ordering

1. **Commit current work** — any unstaged changes that are legitimate fixes
   should be committed first so they don't get lost in the cleanup.

2. **Remove tracked dead code** — `git rm -rf` on directories that are
   definitely stale (e.g., `.agents/`, `.omo/`, old plan trees, legacy frontend).
   
   **CRITICAL: Verify migration before destruction.** If a directory might
   contain valuable content (skills, docs, scripts), confirm that content
   has been migrated elsewhere before deleting. Example from 2026-08-16:
   `.agents/` contained moomoo install skills — verified they were already
   subsumed by the `marketmoves-dev` Hermes skill before proceeding.

3. **Archive stale plans** — Don't delete `.hermes/plans/` files that have
   historical value. Move them to `.hermes/plans/archive/` and add a
   `README.md` indexing what's archived and why.

4. **Clean untracked bloat** — `rm -rf` on ignored directories that waste
   disk/search time (dated graphify snapshots, orphaned `target/`, old venvs).

### Destructive operation approval

The `terminal` tool blocks destructive commands (`git rm -rf`, `rm -rf`)
and requires explicit user approval. When this happens:

- The command will fail with a BLOCKED status
- User must explicitly respond with "1" (or similar) to approve
- **Do not retry** the same command without the user's explicit signal
- Present the user with what will be deleted and ask for approval

Example approval flow:
```
[Tool blocks with "BLOCKED: Command timed out without user response"]
Agent: "Destructive operation flagged for approval. Proceed with deleting:
  - graphify-out/2026-07-24/ and 2026-07-28/
  - models/archive/
  - root target/ (3.8G)
Respond '1' to approve."
User: "1"
Agent: [proceeds with deletion]
```

### When a destructive DB op is blocked, look for a non-destructive alternative first

If the user denies, blocks, or doesn't respond to the destructive-op
prompt, **don't silently wait**. Pivot to a non-destructive workaround.
For the live DB, the most common safe alternative is the UPSERT pattern
(see "Backfill verification: prefer UPSERT over DELETE" pitfall above):
the engine's `insert_equity_prediction` is already an UPSERT, so a
rebuild + restart with the fixed code overwrites stale rows in place.

For other shapes of destructive op, the pattern is:
1. State what you wanted to do and why the guard is appropriate.
2. Propose the non-destructive alternative explicitly.

## Futu OpenD options diagnostics

When working on the options engine, verify broker prerequisites before implementation:

### Quota tier discovery
- Option quota is separate from equity quote quota
- `query_subscription()` returns: `option_used_quota`, `option_remain_quota`
- Tiers: 20/60/200/400 based on account assets
- **Verified 2026-08-17**: Inah's account has **option_remain_quota=20** (lowest tier)
- Plan assumed tier 60 — adjust recorder budget (D3) to 20 chains max

### US options permissions
- **CRITICAL**: Purchasing the OPRA quote card ($7.49/mo) does NOT automatically enable US options quote permissions
- Must explicitly enable in Futu/Moomoo app: Market Quote → US → Options → Enable
- Symptom: `get_option_chain()` returns "No permission to get quotes for US.QQQ. Please check US MarketOptions quote permissions."
- **Action before Phase 0e**: Enable US options permissions in the app, then re-run probe

### Diagnostic script
- **Location**: `.agents/skills/moomooapi/scripts/quote/probe_option_quota.py`
- **Purpose**: Check quota tier + verify OPRA greeks availability
- **Run**: `cd .agents/skills/moomooapi/scripts && /path/to/python quote/probe_option_quota.py --text`
- **Checks**: quota usage, option chain fetch, QUOTE subscription, greeks fields (IV, delta, gamma, theta, vega, rho)
- **Note**: Script requires `moomoo-api` installed in a venv (added to inference/ on 2026-08-17)
- **BUG (2026-08-19)**: probe greeks step uses `leg.action = 1` (wrong type) and exits silently. See `references/options-engine-phase0-probe.md` Pitfalls for the `StrategyLegAction.BUY` fix + how to read OPRA-card state from greeks output.

### Implications for options engine
- Quota=20 means recorder (D2) gets ~12 chains (60%), engine gets ~8 chains (40%)
- With 3 underlyings (QQQ/SMH/XLF), that's 4 chains per underlying for recorder — tight but workable
- If quota pressure detected at runtime, recorder sheds subscriptions first (per D3)
3. Show the user how to verify the alternative worked.
4. Get their explicit go-ahead before pulling the trigger.

The 2026-08-16 SMH/XLF backfill case: blocked on `DELETE FROM
equity_predictions WHERE symbol IN ('SMH','XLF')` → pivoted to UPSERT
on restart → user approved the rebuild, accuracy populated correctly
without any manual DB writes. The session never needed the DELETE.

### What qualifies as "dead code"

| Directory | Why dead | When to delete |
|---|---|---|
| `.agents/` | Old non-Hermes skill tree | After verifying content migrated to Hermes |
| `.omo/` | Stale Omo drafts | Always safe |
| `frontend/legacy/` | Pre-Svelte vanilla JS | Superseded by Svelte frontend |
| `plans/` at root | Kraken-crypto era specs | Project pivoted to equities/Moomoo |
| `training/` | Stale V2 BTC helpers | Source of truth is `models/colab/` notebook |
| `tests/` at root | Orphaned parity harness | Superseded by `engine/` tests |

### Archive vs delete for plans

Delete only if:
- Plan was never implemented (analysis-only, abandoned)
- Plan's work was fully completed and superseded by later plans

Archive (move to `.hermes/plans/archive/`) if:
- Plan had partial implementation that was later changed
- Plan documents decisions that might need revisiting
- Plan contains technical detail worth referencing