---
name: rust-docker-review-redeploy
title: "Review, Build, and Redeploy a Rust + Docker-Compose Service"
description: "Review recent commits on a Rust backend deployed via docker-compose, verify locally, rebuild the image, redeploy, and confirm fixes are live — the full end-to-end loop."
category: software-development
triggers:
  - "User says: 'review the last few changes and redeploy'"
  - "User says: 'deploy the latest changes'"
  - "User says: 'review the changes against the plan and complete'"
  - "User asks to verify fixes are applied on a running server"
  - Rust project with a Dockerfile + docker-compose.yml deploy stack
  - Need to check if running Docker image is stale vs HEAD commit
  - Partially-implemented plan needs gap analysis before resuming
---

# Rust + Docker Review-and-Redeploy

Review recent commits on a Rust service deployed via `docker-compose`,
verify the code locally (build + test), rebuild the image, redeploy, and
confirm the new code is live in production. This is the full loop for a
VPS-hosted Rust service that uses Docker for deployment.

## When to Use

- A Rust project with `engine/Dockerfile` + `deploy/docker-compose.yml`
- User asks to "review and redeploy" or "deploy the latest changes"
- User asks to "review changes against the plan and complete" — see
  `references/plan-gap-analysis.md` for the gap-analysis procedure
- Need to check whether the running Docker image is stale vs the latest
  git commit
- The service uses `docker-compose` (v1 or v2) for orchestration

## Procedure

### 1. Gather Context (parallel)

```bash
# Git state
git status --short --branch
git log -8 --oneline --decorate
git diff --stat HEAD~3..HEAD          # or however many commits to review

# Code diff (actual changes, excluding generated files)
git diff HEAD~N..HEAD -- engine/ src/ Cargo.toml

# Current deploy state
docker ps --format 'table {{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Ports}}'
docker inspect <image>:latest --format '{{.Created}}'
git log -1 --format='%ci %H'

# Compose version
docker-compose --version   # or: docker compose version
```

Compare the image creation date vs HEAD commit date. If the image is
older than HEAD, the running container is stale — fixes are NOT deployed.

**Disk space check (run before `docker-compose build` on small VPS):**

```bash
df -h / | grep -v tmpfs   # if / is > 90% used, prune dangling images first
docker image prune -f     # reclaims 10–20GB typically; safe (active images protected)
```

A failed build with `no space left on device` mid-tar usually means
dangling Docker images are eating the disk, not the build context (see
the disk-full pitfall below). Pruning BEFORE the build prevents the
surprise failure.

### 2. Local Verification (build + test)

Always compile and test BEFORE rebuilding the Docker image — it's faster
and catches issues without waiting for a Docker build.

```bash
cd engine  # or wherever Cargo.toml lives
cargo build --release 2>&1 | tail -25
cargo test --release 2>&1 | tail -30
```

**Baseline check for test failures:** If tests fail, verify they aren't
pre-existing by running the same tests against the prior commit:

```bash
git stash  # if there are uncommitted changes
cargo test --release --lib 2>&1 | tail -5
git stash pop
```

If the failure count is identical before and after your changes, it's a
pre-existing issue, not a regression. Document it but don't block deploy.

**Per-module test isolation:** If tests fail in bulk but pass alone, it's
a shared-state poisoning issue (common with Rust tests sharing a Mutex or
static env). Run each module separately to identify the culprit:

```bash
cargo test --release --lib <module>::  # e.g. config::, db::, api::
```

### 3. Rebuild the Image

```bash
docker-compose -f deploy/docker-compose.yml build engine
```

**If `docker-compose build` fails with `KeyError: 'ContainerConfig'`** (v1.29.2 against Docker server 28+), bypass compose and use `docker build` directly:

```bash
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .
```

The image lands at the same `:latest` tag, so subsequent `docker run` invocations (see Section 4 alternative) pick it up unchanged. On a VPS with old compose v1 + a modern Docker engine, `docker build -f` is the realistic escape hatch — don't waste time debugging compose. After the build succeeds, the post-redeploy verification recipe (see Section 5 alternative below) covers proving the new image is the one actually serving.

**ALWAYS use `--no-cache` if static files were updated** — Docker layer caching can serve a stale `frontend/` even when the source changed. The build may also preserve `root` ownership from the build stage across all COPY'd files, which breaks the SPA when the runtime user is non-root: check `docker inspect <new-image> --format '{{.Config.User}}'` and verify `COPY --chown=<run_user>:<run_group>` is present for the frontend line in the Dockerfile (see SPA static-files pitfall below). Alternatively, fix the local filesystem ownership first: `sudo chown ubuntu:ubuntu frontend/ -R` before building.
frontend line in the Dockerfile (see SPA static-files pitfall above).
Alternatively, fix the local filesystem ownership first:
`sudo chown ubuntu:ubuntu frontend/ -R` before building.

### 4. Redeploy the Stack

**docker-compose v1 bug workaround:** v1.29.2 has a `KeyError:
'ContainerConfig'` bug when recreating containers. Remove old containers
first:

```bash
docker rm -f mmn-engine mmn-inference mmn-proxy  # adapt container names
docker-compose -f deploy/docker-compose.yml up -d
```

With v2 (`docker compose`), just `up -d` works — it recreates cleanly.

**After `up -d`, ALWAYS verify the running container uses the new image:**
compare the running container's image hash against the newly built one:

```bash
NEW_HASH=$(docker inspect marketmarkovnet/engine:latest --format '{{.Id}}')
RUNNING_HASH=$(docker inspect mmn-engine --format '{{.Image}}')
echo "New: $NEW_HASH"
echo "Running: $RUNNING_HASH"
# These must match — if not, the container wasn't recreated
```

Also verify the SPA static files are readable:
```bash
docker exec mmn-engine ls -la /app/frontend/views/
# Must show <run_user> <run_group>, NOT root root
curl -sI http://127.0.0.1:8080/views/accuracy.js | grep content-type
# Must be text/javascript, NOT text/html (HTML = SPA broken)
```

### 5. Verify Deployed State

Wait for healthchecks to settle (30s is usually enough), then verify:

```bash
# Container health
sleep 30
docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'

# Engine logs — check for migration messages, staleness warnings, errors
docker logs mmn-engine --tail 40

# API responses (via the proxy port — check docker-compose for the mapped port)
curl -s http://localhost:<http_port>/api/status
curl -s http://localhost:<http_port>/api/predictions
```

**Confirm fixes are live:** Look in the engine startup logs for evidence
of the new code executing — e.g. "migrated predictions: added column
actual_1h", staleness warning messages, or new API fields in responses.

**Internal vs external endpoint test:** If the external HTTPS endpoint
returns TLS errors or 404, test the engine directly inside the Docker
network to isolate whether it's a proxy/config issue or an engine issue:

```bash
docker exec mmn-proxy wget -qO- http://engine:8080/api/status
docker exec mmn-engine curl -s http://127.0.0.1:8080/api/status
```

**Ad-hoc verification recipe (no canonical test target exists):** when
the project has no `make smoke` / integration test that exercises the
deployed engine, write a one-off `bash` script in `/tmp` that probes
the deployed behavior end-to-end (healthcheck → API → Svelte bundle →
DB row counts → log structure → commit log). Use `[PASS]` / `[FAIL]`
markers and exit non-zero on any failure so the user sees real signal,
not a recap. Gotchas: `curl | grep` against the Svelte bundle SIGPIPEs
on early match (write to temp file first); `curl /api/v1/ws` without
`-v` masks the 101 status line behind the progress meter (use `-v` and
grep `^< HTTP`); log timestamps need awk time arithmetic not naive
subtraction (parse `T16:08:05` → seconds, then diff). Adapt the recipe
to your endpoints; delete the script after capturing the run output.

### 6. Independent Review (optional but recommended)

For non-trivial diffs, dispatch an independent reviewer subagent that
gets ONLY the diff — no shared context. See the `requesting-code-review`
skill for the full pipeline.

### 7. Commit incrementally — one fix per commit

When multiple semantically distinct changes land in the same session
(e.g. freshness gate fix + VIX fallback + SMA200 + Dockerfile chown),
commit them as separate commits rather than one mega-commit. Each
commit should:

- Touch one logical change (or one tightly-coupled set)
- Have a self-contained message with the **what** and the **why**
- Be verifiable on its own (build passes, tests pass, runtime works)

Why: when a future session needs to bisect or revert a specific
behavior, a single commit that mixes "freshness gate + VIX fallback +
SMA200 + frontend ownership" is much harder to reason about than four
commits each with a clean `git revert` target. The pattern:

```bash
git add <files for fix 1>
git commit -m "fix(data): <one-line summary> — <why>"

git add <files for fix 2>
git commit -m "feat(api): <one-line summary> — <why>"

# ... one commit per fix ...
```

Use `fix:` for bug fixes, `feat:` for new features, `chore:` for
cleanup, `docs:` for documentation, `refactor:` for non-behavior
changes. Keep the commit body short (1-3 lines) and put the rationale
in the second paragraph if it's not obvious.

## Pitfalls

- **Yahoo Finance backfill: 3-day hardcoded threshold is wrong for daily
  bars** — the ORIGINAL bug: `yahoo.rs::backfill()` had a hardcoded
  `stale_threshold_secs = 3 * 24 * 3600` (3 days) applied uniformly to ALL
  call sites. For a daily-bar system, this means: after initial backfill
  seeds 1250+ rows, the gate skips the fetch until exactly 3 days pass —
  even a single missed daily bar sits for 3 days. Staleness showed
  263,471s (3.05 days) in the UI. The fix: make threshold a parameter
  so each caller controls it:
    - `main.rs` startup: 3 days (conservative — just needs history)
    - `data/mod.rs` daily top-up: 18h (bars close 16:00 ET ~20:30 UTC;
      by next morning ~13:30 UTC new bar is available; anything older than
      ~18h means yesterday's bar was missed)
    - `api/equity.rs` manual refresh: 0 (always fetch, ignores gate)
  Call chain: `backfill(..., stale_threshold_secs)` ←
  `backfill_many(..., stale_threshold_secs)` ←
  `backfill_equities(pool, threshold)`. Also: `stale_threshold_secs = 0`
  is the escape-hatch for manual API endpoints — always fetches. Engine
  logs: `"sufficient equity candles — skipping backfill"` = gate passed;
  `"equity candles stale — forcing refresh"` = freshness triggered.
  `staleness_secs` in `/api/status` surfaces this: anything beyond ~5 min
  on a weekday = missing data. Also add a startup warning in `main.rs`
  for >72h candle age. The FRED macro series has the same pattern.

- **Stale data masquerading as a broken prediction engine** — after
  redeploy, the dashboard can show stale predictions not because the
  engine is broken but because no new candles have arrived. The
  staleness monitor (if wired) will flag this, but the root cause is
  often a WebSocket that connects and subscribes successfully yet
  receives zero confirmed OHLC messages. The REST backfill path is the
  reliable fallback. See `references/stale-data-forced-backfill.md` for
  the diagnosis + forced-refresh procedure.

- **Svelte Dashboard: status staleness freezes because WS StalenessAlert
  is never emitted** — the frontend fetches `/api/status` once on mount
  and then relies on WebSocket `StalenessAlert` events to update
  `staleness_secs`. But `StalenessAlert` is defined in the WS schema
  and handled in `websocket.js` — yet NO code path in the scheduler or
  ingestion ever broadcasts it. The symptom: staleness value is frozen at
  the same number (e.g. 263,471s) even after data is refreshed. Other
  panels may also be frozen. Fix: add a 30s polling interval to the
  Dashboard's `onMount`/`onDestroy`:

  ```svelte
  let statusInterval;
  onMount(async () => {
    // ... initial fetch ...
    connectWebSocket();
    // Poll /api/status every 30s — StalenessAlert is never emitted
    statusInterval = setInterval(async () => {
      try {
        const s = await fetchStatus();
        status.set(s);
      } catch (e) { /* silent — WS may still deliver */ }
    }, 30000);
  });
  onDestroy(() => {
    disconnectWebSocket();
    if (statusInterval) clearInterval(statusInterval);
  });
  ```

  Diagnosis: browser DevTools → Network → filter `/api/status` — if only
  ONE request (page load), polling is not running.

- **Docker layer cache serves stale binary** — if the Dockerfile has a
  "deps warmup" step (build dependencies with a placeholder, then copy
  real source), Docker's layer cache can keep the placeholder binary.
  The fix is a `touch` on the main source file before the real build to
  force cargo to detect the change. Check the Dockerfile for this pattern.

- **`docker-compose build` fails with "no space left on device" — root
  cause is usually dangling images, not the build context** — a small VPS
  (~96GB disk, 95% full after months of layered Rust/Python images)
  shows `Can't add file ... to tar: io: read/write on closed pipe` and
  `no space left on device` mid-context-tar. The fix is NOT to expand
  the disk or to `.dockerignore` more aggressively (a correct
  `.dockerignore` already excludes `.venv/`/`target/`/`data/`/`models/`).
  The real culprit is dangling/old Docker images in
  `docker system df`. `docker image prune -f` typically reclaims
  10–20GB on a VPS that's been rebuilding Rust/Python images for weeks
  — on the MarketMoves VPS it freed 15.58GB (21GB total free after),
  enough for a full Rust release build. Add a pre-deploy check to the
  procedure: if `df -h /` shows > 90% used, run `docker image prune -f`
  BEFORE `docker-compose build`. Active (running) containers' images are
  protected by `docker image prune`; only dangling images are removed.
  Also: `docker builder prune` cleans build cache if the image prune
  isn't enough (rare — image prune handles 95% of cases).

- **docker-compose v1 `ContainerConfig` KeyError** — v1.29.2 fails when
  recreating containers with newer Docker. Workaround: `docker rm -f` the
  containers before `up -d`. Consider upgrading to Compose v2 plugin.

- **`docker-compose -f` flag is unsupported on this VPS** — the installed
  `docker-compose` (v1.29.2 at `/usr/bin/docker-compose`) does NOT accept
  the `-f` flag (errors: `unknown shorthand flag: 'f' in -f`). But it DOES
  accept the positional compose file if you `cd` into the dir containing it:
  `cd /home/ubuntu/projects/MarketMoves && docker-compose up -d engine`
  (compose auto-discovers `deploy/docker-compose.yml`? NO — it looks for
  `docker-compose.yml` in cwd). On this box the reliable path is to avoid
  `docker-compose` entirely when the image was rebuilt: use `docker build`
  for the image and `docker run` (see lean-redeploy pitfall below) for the
  container. The compose file's env block is the authoritative env source —
  copy its `environment:` keys into the `docker run -e` flags. If you must
  use compose, `cd deploy && docker-compose up -d engine` (compose v1 reads
  `./docker-compose.yml` from cwd) — but the `ContainerConfig` KeyError will
  still fire on recreate, so prefer `docker run`.

- **Lean-redeploy via `docker run` (bypass broken compose entirely)** —
  when `docker-compose up -d` throws `KeyError: 'ContainerConfig'` and you
  only changed ONE service (e.g. engine), skip compose. Steps:
  1. `docker build --no-cache -f engine/Dockerfile -t marketmarkovnet/engine:latest .`
  2. `docker rm -f mmn-engine`
  3. `docker run -d --name mmn-engine --network deploy_mmn --restart unless-stopped --user 1000:1000 -v deploy_data:/app/data -v deploy_models:/models:ro -e HTTP_PORT=8080 -e DATABASE_URL=sqlite:///app/data/candles.db -e NORM_STATS_PATH=/models/norm_stats_qqq_v1.json -e TRADING_MODE=paper -e ZMQ_ENDPOINT=tcp://inference:5555 -e SYMBOL=QQQ -e RUST_LOG=info --env-file .env marketmarkovnet/engine:latest`
  The `--network deploy_mmn` keeps the engine on the same bridge as
  `mmn-inference` (healthy, untouched). `--env-file .env` pulls the rest
  (FRED key, Moomoo host, etc). This deploys the engine with ZERO effect on
  `mmn-proxy` or `mmn-inference`. Verify: `docker inspect mmn-engine --format
  '{{.Image}}'` must equal `docker inspect marketmarkovnet/engine:latest
  --format '{{.Id}}'`. DO NOT use `docker-compose down` — it destroys the
  network and restarts healthy siblings.
  **Lean variant (preferred when only one or two services changed):**
  `docker stop <svc> && docker rm -f <svc> && docker-compose up -d`. Removing
  ONLY the affected services (not all three) avoids disrupting healthy
  siblings like the proxy. Confirmed working on the MarketMoves VPS where
  removing `mmn-engine` + `mmn-inference` (but leaving `mmn-proxy` alone)
  was enough to bypass the prior-container inspect path that triggers the
  bug. The full `down && up` cycle destroys the network and restarts
  every container — overkill if only one image actually changed. Use the
  lean variant for engine-only or inference-only deploys; full `down+up`
  only when the proxy or compose-level config also needs restarting.
  Diagnostic: the failed `up -d` leaves the recreated container in a
  stray state with a name like `<hash>_mmn-inference` (not the canonical
  `mmn-inference`) — that means compose partially recreated before the
  bug fired. `docker rm -f <hash>_<svc>` to clean up, then retry the
  lean sequence.

- **HTTPS endpoint TLS errors on localhost** — Caddy with `tls internal`
  and on-demand TLS may not generate a cert for `localhost` after a
  restart. Test via the HTTP port instead, or via `docker exec` inside
  the network. This is a proxy config issue, not an engine issue.

- **`sqlite3` not in the container** — can't introspect the DB directly
  with `docker exec ... sqlite3`. Use the API endpoints or install
  sqlite3 in the image if frequent DB inspection is needed.

- **Test poisoning in Rust** — tests that share static state (env vars,
  Mutexes) can poison each other when run in parallel. A test that passes
  alone but fails in bulk is the signature. This is a test isolation bug,
  not a code regression — don't block deployment for it.

- **Graphify noise in diffs** — `git diff --stat` can show hundreds of
  changed files in `graphify-out/`. Always scope diffs to the source
  directories (`-- engine/ src/`) to see the real changes.

- **Dead code looks "done" in gap analysis** — a function can exist with
  passing tests but never be called from any runtime path. The cargo
  compiler flags this: `warning: function 'X' is never used`. When
  reconciling commits against a plan, grep for the function name in
  non-test entry points (main.rs, router(), scheduler). If the only hits
  are the definition file and tests, the todo is PARTIAL — the wiring is
  missing. See `references/plan-gap-analysis.md` for the full procedure.
  Concrete instance (MarketMoves W3): `db::compute_actuals` and
  `db::fetch_accuracy` both existed with passing tests but were never
  called from runtime — the actuals task spawn in `main.rs` and the
  `/api/accuracy` route registration in `api::router()` were the missing
  wiring that turned dead code into live code.

- **`#[derive(Debug)]` required on response DTOs when tests use
  `Result::unwrap_err()`** — axum handlers return
  `Result<Json<T>, (StatusCode, String)>`. A test that exercises the
  error path via `handle_x(state).await.unwrap_err()` needs `T: Debug`
  because `Result::unwrap_err` requires `T: Debug` on the Ok variant.
  Symptom: test fails to COMPILE (not a runtime failure) with `the trait
  Debug is not implemented for ResponseDto`. Fix: add
  `#[derive(Debug, Serialize)]` to the DTO. Costs a full release build
  cycle per occurrence (~25s on this project) — when a handler has a
  4xx/5xx error path AND a test asserts on it, add `Debug` upfront.

- **Background task spawn with delayed first tick** — when wiring an
  hourly background task (actuals, retention, etc.), the first tick
  should be delayed ~5 min after startup so it doesn't race the
  scheduler or REST backfill on a fresh boot. Use
  `tokio::time::interval_at(start, period)` with
  `start = Instant::now() + Duration::from_secs(300)`, NOT
  `tokio::time::interval` (which ticks immediately on first
  `tick().await`). The actuals task in `engine/src/main.rs` mirrors the
  retention task in `engine/src/data/mod.rs` — follow whichever is
  closest in scope when adding a new one.

- **Converting a Rust module from file to dir (file→dir collision)** —
  Rust resolves `mod features;` to EITHER `features.rs` (file) OR
  `features/mod.rs` (dir), NOT both. If both exist simultaneously the
  compiler errors: `E0761: file for module 'features' found at both
  "features.rs" and "features/mod.rs"`. If only the dir exists without
  `mod.rs`, the compiler silently ignores the sub-modules.
  FIX: before creating the dir, move the old file's content into
  `features/legacy.rs` (or similar), delete `features.rs`, then create
  `features/mod.rs` with `pub mod legacy;` + `pub use legacy::{compute_features, FeatureRow};`
  so existing callers (`crate::features::compute_features`) keep resolving
  without touching their imports. This is the non-breaking path for
  adding a V2 module alongside a running V1 pipeline.
  **git stash trap:** the standard `git stash && cargo test && git stash pop`
  baseline check FAILS after a file→dir split — stashing restores the
  old `api.rs` while the new `api/` directory still exists on disk,
  triggering E0761. To verify pre-existing failures, run individual
  test modules instead: `cargo test --lib api::tests` or
  `cargo test --lib -- --skip config::tests`.

- **Splitting a monolithic Rust file into a module tree** — when
  refactoring a large file (e.g. `api.rs` → `api/mod.rs` +
  `api/status.rs` + `api/chart.rs` …), the split has three cascading
  issues that each cost a compile cycle:
  1. **`pub(crate)` visibility cascade**: sub-modules need
     `use crate::db;` explicitly (not just `use super::*`), and
     response structs need `pub(crate)` on BOTH the struct AND every
     field that tests access directly. A struct annotated
     `pub(crate) struct Foo` with private fields compiles fine in the
     handler, but tests in `api/tests.rs` that do `resp.latest.unwrap()`
     fail with `E0616: field 'latest' of struct 'Foo' is private`.
  2. **Dead code from internal helpers**: functions that were called
     within the monolithic file (e.g. `candle_to_dto` for the crypto
     `Candle` type) become dead code when the split module only uses
     the equity variant. The compiler warns `function 'candle_to_dto'
     is never used`. Remove the dead function rather than suppressing.
  3. **Test helper visibility**: tests that reference handler functions
     directly (e.g. `predictions::prediction_to_dto(&row)`) need those
     functions marked `pub(crate)`, not private `fn`.
  See `references/rust-module-split.md` for the full step-by-step.

- **Dormant-scaffold pattern for live systems** — when refactoring a
  running engine to a new feature pipeline (e.g. 3-feature → 6-feature),
  do NOT switch the scheduler/inference path until the new model is
  trained AND passes its validation gate (e.g. walk-forward OOS IC > 0.05).
  Instead: add the new V2 modules (FeatureRowV2, FeatureSource trait,
  NormStatsV2, bridge::predict_v2) as DORMANT code that compiles but
  isn't called. Keep the V1 path (`compute_features`, `normalize_row`,
  `bridge::predict`) running with the existing model. Mark dormant code
  with `#[allow(dead_code)]` to suppress warnings. Switch the scheduler
  only after the new model clears the gate — this prevents breaking the
  live engine mid-refactor. Concrete instance (MarketMoves W5): the V2
  6-dim pipeline (`features/core.rs`, `features/crypto.rs`,
  `normalize::NormStatsV2`, `bridge::predict_v2`) compiles alongside
  the V1 3-dim path; the scheduler stays on V1 until the new TCN model
  graduates the walk-forward IC gate.

- **sqlx 0.8 does not auto-create the SQLite `.db` file** — only the
  parent directory. On a fresh deploy or test with a new DB path, the
  pool connect fails with "unable to open database file (code 14)" even
  though the directory exists. Fix: in `db::open()`, after
  `create_dir_all(parent)`, pre-create the file with
  `OpenOptions::new().create(true).append(true).open(path)` before
  `SqlitePoolOptions::connect()`. This is a sqlx 0.8 regression from
  earlier versions that did auto-create. Symptom: engine exits at
  startup with "database error: connecting to SQLite at ...: (code 14)
  unable to open database file" even though the directory is writable
  and the path is valid.

- **`cargo run` CWD is the crate dir, not the workspace root** — when
  the binary reads `.env` relative paths (e.g.
  `NORM_STATS_PATH=models/norm_stats.json`), they resolve relative to
  the *process* CWD, which for `cargo run -p engine` is `engine/`, not
  the workspace root. Symptom: `norm_stats error: reading norm stats
  file: /models/norm_stats.json: No such file or directory` — note the
  leading `/`, indicating the path resolved against the wrong CWD. Fix
  for testing: pass absolute paths via env vars
  (`NORM_STATS_PATH=/home/.../models/norm_stats.json`). Fix for
  production: the Dockerfile sets WORKDIR explicitly, so this only
  bites local `cargo run` sessions.

- **Adding struct fields breaks all construction sites** — when you add
  fields to a widely-constructed struct (e.g. `Candle` gains
  `funding_rate`, `basis_z`, `ob_imbalance`), EVERY `Candle { ... }`
  construction across the codebase must be updated, including: DB
  `from_row` mappings, `upsert` SQL+binds, test helpers (`fn candle()`
  in feature tests), integration test fixtures, and `From<Other> for
  Candle` impls (e.g. parity.rs `GoldenCandle → Candle`). The compiler
  will surface them one at a time; use `replace_all` patches for
  repeated patterns (e.g. `vwap: 100.0 })` →
  `vwap: 100.0, funding_rate: 0.0, basis_z: 0.0, ob_imbalance: 0.0 })`).
  Don't forget the SQLite DDL (`CREATE TABLE`) AND a migration
  (`ALTER TABLE ... ADD COLUMN ... DEFAULT 0`) for existing DBs — use
  the existing `PRAGMA table_info` + conditional `ALTER` pattern.

- **Config struct field additions cascade to test Mutex poisoning** —
  when you add fields to a `Config` (or similar widely-tested struct),
  every test that constructs it literally (`Config { ... }`) must be
  updated. If a test panics while holding a shared `static ENV_LOCK:
  Mutex`, it *poisons* the mutex, and every subsequent test that locks
  it fails with `PoisonError` — a cascade that looks like 16 tests
  failing simultaneously. Signature: tests pass individually with
  `--test-threads=1` up to the first one that hits the missing field,
  then ALL remaining tests fail with PoisonError regardless of their
  own logic. Root cause is the single missing field in the test
  construction, not the ENV_LOCK itself. Fix: update all literal
  `Config { ... }` constructions in tests to include the new fields.
  Also: verify the failure is pre-existing by `git stash && cargo test
  && git stash pop` — if the same test fails on the prior commit, it's
  a pre-existing `.env` override issue (e.g. `.env` sets
  `NORM_STATS_PATH=/models/...` which breaks
  `defaults_load_when_env_unset`), not your change.

- **dotenvy reloads `.env` on every `Config::from_env()` call** —
  `dotenvy::dotenv()` is called inside `from_env()`, so it re-reads the
  `.env` file EVERY time, re-setting env vars that a test's
  `clear_engine_env()` just removed. This means `env::remove_var()` +
  `Config::from_env()` still sees `.env` values — clearing env vars is
  not enough when `.env` exists. Symptom: test asserts
  `cfg.norm_stats_path == "models/norm_stats_qqq_v1.json"` (the code
  default) but gets `"/models/norm_stats_qqq_v1.json"` (the `.env`
  value). Fix: either update the `.env` file to match the new default,
  or update the test assertion to match the `.env` value. Do NOT try to
  make `clear_engine_env` delete the `.env` file — that breaks other
  tests. The `git stash` baseline check is essential here: if the test
  passed before your change, the `.env` value happened to match the old
  default; after changing the default, you must update `.env` too.

- **Search for existing implementations before creating new structs** —
  before writing a new struct/module (e.g. `EquityNormStats` in
  `normalize.rs`), grep the codebase for the same concept:
  `grep -rn "EquityNormStats" src/`. Rust allows identical struct names
  in different modules, but the duplicate will shadow or conflict with
  imports, and callers will split between the two implementations.
  Concrete instance: `EquityNormStats` already existed in
  `features/equities_v2.rs` (with `from_rows`, `save`, `load`,
  `normalize` methods and passing tests) when a second one was created
  in `normalize.rs` — the duplicate broke `scheduler.rs` and `main.rs`
  imports. Fix: use the canonical implementation; if you need a
  different loader format (e.g. name-keyed JSON vs positional arrays),
  add a method to the existing struct rather than creating a parallel
  type.

- **Training-side ↔ engine-side serialization format mismatch** —
  training code (Python/Colab) and the Rust engine may serialize the
  same concept differently. Example: training outputs
  `{"medians": {"trend_slope": 0.015, ...}, "mads": {...}}` (name-keyed
  maps) but the engine struct uses `{"median": [0.015, ...], "mad":
  [...]}` (positional arrays). Do NOT change either side — add a bridge
  loader (`load_named()`) on the engine struct that accepts the
  training format and maps name-keyed values to positional arrays using
  a canonical feature-name constant (`EQ_FEATURE_NAMES`). This keeps
  training output stable (no Colab re-run needed) and engine internals
  unchanged. Always add a test that loads the actual trained artifact
  file and asserts on known values to catch format drift.

- **No git remote — deploy is done directly on the VPS, not via `git push`** — some repos (e.g. MarketMoves) have no `origin` remote. After verifying locally and rebuilding, sync updated files to the VPS via scp/rsync/copy, then on the VPS run `docker build` + `docker rm -f <container>` + `docker compose up -d`. Do NOT assume `git push && ssh VPS 'git pull'` works. Always verify with `git remote -v` first.

- **Docker Hub `latest` overrides local build on `docker-compose up`** —
  if the Dockerfile is pushed to Docker Hub and `image:` in compose points
  to `marketmarkovnet/<service>:latest`, compose's default pull policy
  is `PullImageMissing` — it pulls the Hub image and overwrites your
  local build. Symptoms: engine container is old version (e.g. V1 inference
  engine running instead of V3 equity_model), despite local `docker build`
  completing successfully. Fix: either (a) use `image: sha256:<digest>` in
  compose to pin the exact image, (b) use a `dev` tag locally and avoid
  pushing, (c) add `:latest` after `docker build` with `--tag` but don't
  push, or (d) for the VPS deployment, just stop trying to push to Hub —
  build locally and `docker rm -f <container>` + `docker-compose up -d`
  to let compose create the container from the local image. The compose
  `build:` section (not `image:`) ensures local rebuild.

- **VPS has its own copy of compose/Caddyfile — local edits don't sync**
  — when deploying, the VPS has its own working directory (e.g.
  `/opt/marketmarkovnet/app/`) with its own copies of `.env`,
  `docker-compose.yml`, `Caddyfile`, and `models/`. Editing files in
  `/home/ubuntu/projects/MarketMoves/` does NOT update the VPS copies.
  Symptoms: inference healthcheck keeps using old V1 protocol despite
  `docker-compose.yml` edits in the repo. Fix: explicitly `rsync` or
  `cp` the updated files to the VPS before `docker-compose up`:
  `cp deploy/docker-compose.yml /opt/marketmarkovnet/app/deploy/docker-compose.yml`.
  This also applies to Caddyfile, `.env`, and any config that changes.
  The model files in `/var/lib/docker/volumes/deploy_models/_data/` are
  on the VPS volume — update them there directly, not just locally.
  After syncing: `docker rm -f <container>` + `docker-compose up -d` to
  force recreate with the new config (compose won't auto-recreate if the
  image tag is unchanged, even if the HEALTHCHECK command changed).

- **Docker compose volume name ≠ project name — model artifacts may be in
  the wrong volume** — compose's named volume `models:` is scoped to the
  compose project name. `docker volume ls | grep models` may show
  `deploy_models`, `marketmarkovnet_models`, or `marketmoves_models` depending
  on how `docker-compose.yml` was originally brought up. The container
  mounts whichever volume exists under the current project name. Always
  verify which volume is actually mounted before copying artifacts:
  `docker inspect <container> --format '{{range .Mounts}}{{.Source}} → {{.Destination}}{{"\n"}}{{end}}'`.
  Confirmed working check: `docker run --rm -v deploy_models:/data alpine ls /data/qqq_tcn_v1.pt`
  to see the actual file timestamps. Copy artifacts into the correct
  volume with `docker run --rm -v <volume>:/target -v $(pwd)/models:/src:ro alpine cp -a /src/. /target/`.

- **`docker-compose -f` is unsupported on this VPS** — the installed
  `/usr/bin/docker-compose` is v1.29.2 and does NOT accept `-f` (errors:
  `unknown shorthand flag: 'f' in -f`). It also throws `KeyError:
  'ContainerConfig'` on `up -d` when the engine image was rebuilt. RELIABLE
  ESCAPE: skip compose entirely, use `docker build` + `docker run` with
  `--network deploy_mmn` (see lean-redeploy pitfall below). Compose reads
  `./docker-compose.yml` from cwd, not via `-f`, and even then the KeyError
  fires. The compose file is still the authoritative env source — copy its
  `environment:` block into `docker run -e` flags.

- **Inference HEALTHCHECK must match the actual protocol** — if the
  inference service is updated from V1 (3 features: pred_1h/4h/24h) to
  V3 (8 features: pred_1d/5d/21d), both the Dockerfile HEALTHCHECK and
  the compose-level healthcheck override must be updated together.
  The compose `healthcheck.test` overrides the Dockerfile's HEALTHCHECK
  instruction — if the compose override still sends V1 requests, it
  masks the V3 container as "healthy" while the engine sends V3.
  Symptom: `docker logs mmn-inference` shows alternating V1 (seq_len=1,
  healthcheck) and V3 (seq_len=126, engine) requests — the V3 requests
  explode with bad norm_stats but the container stays "healthy" because
  the healthcheck is V1. Fix: update both the Dockerfile HEALTHCHECK
  CMD and the compose `test:` array to send V3-format payloads and
  assert on V3 keys (`pred_1d`, `pred_5d`, `pred_21d`).

- **Training-side norm_stats MAD collapse causes live-prediction explosion**
  — if a feature has near-zero MAD in the norm_stats artifact (e.g.
  RSI median=5, MAD=1e-6), a small gap between the training median and
  real live values explodes after normalization:
  `norm = (live - median) / (1.4826 * mad)` → with mad=1e-6 and live=60,
  norm ≈ 3.6×10⁷. The TCN sees out-of-distribution inputs and outputs
  unpredictable values → raw prediction × atr_ratio gives absurd results
  (e.g. pred_1d = -2987 for QQQ). Symptom: `GET /api/status` shows
  pred_1d in thousands instead of ±0.005. Diagnosis: check inference
  logs (`docker logs mmn-inference | grep pred_1d`) — seq_len=126
  requests (engine, real) will show huge values; seq_len=1 requests
  (healthcheck, all-zeros) show tiny sane values. Also check
  `GET /api/equity/features?symbol=QQQ` to see live feature values vs
  the artifact's medians. Fix: recompute norm_stats from the full
  training dataset with clamped MAD values (e.g. RSI MAD ≥ 0.5 on a
  0-100 scale), then upload the new artifact to the VPS model volume.
  See `.hermes/plans/fix-norm-stats-qqq.md` for the full Colab fix
  procedure. The UI will continue to show the live data (even exploding)
  since the API serves whatever the model outputs — the fix is purely
  on the training/artifacts side.

- **Svelte/Vite: Dockerfile must serve `frontend/dist/`, not the source tree**
  — `engine/src/api/mod.rs` falls back to `ServeDir::new("frontend")`.
  If the Dockerfile does `COPY frontend/ frontend/`, the engine
  serves the dev `index.html` pointing at `/src/main.js` (which
  404s). SPA renders blank despite a working API. The proper fix
  is a Node build stage (`npm ci && npm run build`) that bakes
  `frontend/dist/` into the image; the long-term fix is migrating
  to `rust-embed` so assets are compiled into the binary (see
  `references/rust-embed-svelte-scaffold.md`). As an inline
  workaround you can build the Svelte dist locally, then
  `docker exec mmn-engine bash -c 'cd /app/frontend && cp dist/index.html ./index.html && cp -r dist/assets ./assets && rm -rf src dist'`.
  After `curl localhost:8080/` verify it returns
  `<script src="/assets/index-HASH.js">`. See
  `svelte-vite-serve-from-built-dist` for the full recipe.

- **HEALTHCHECK command left over from a renamed API field** —
  when the API response renames a key (e.g. `trading_mode` → `mode`),
  the Dockerfile `HEALTHCHECK CMD` and the compose
  `engine.healthcheck.test` continue to grep for the old key. The
  container shows "unhealthy" even though `/api/status` returns 200.
  Engine is alive; the healthcheck just cannot see the new field.
  Update BOTH the Dockerfile `HEALTHCHECK` directive AND
  `deploy/docker-compose.yml`'s `engine.healthcheck.test` array.
  Verification: `docker inspect mmn-engine --format '{{.State.Health.Status}}'`
  → `healthy` after the fix.

- **SPA static files owned by root → all SPA panels silently go `—`**
  — `COPY --from=build` without `--chown` preserves the build stage's
  root ownership. If the container runs as a non-root user (e.g. `USER
  mmn`), that user can't traverse/execute the parent directories of
  static files, causing `ServeDir` to return the HTML fallback (`<!doctype
  html>`) instead of the `.js` file. The ES module import fails silently
  (whole graph aborts), and every SPA panel shows `—` dashes — but the
  API works perfectly (`curl /api/status` returns data). Diagnosis:
  `curl http://127.0.0.1:8080/views/accuracy.js` returns `<!doctype
  html>` not `text/javascript`, or from inside the container
  `ls -la /app/frontend/views/` shows `root root` ownership. The
  browser console shows `Failed to fetch dynamically imported module:
  /views/accuracy.js`. Fix: add `--chown=<run_user>:<run_group>` to the
  frontend COPY line in the Dockerfile:
  `COPY --from=build --chown=mmn:mmn /build/frontend/ /app/frontend/`.
  Also fix local ownership (`sudo chown ubuntu:ubuntu frontend/ -R`) so
  future no-cache rebuilds don't re-introduce the issue. After rebuilding,
  verify with: `docker exec <container> ls -la /app/frontend/views/` and
  `curl -sI http://127.0.0.1:8080/views/accuracy.js | grep content-type`
  (must be `text/javascript`, not `text/html`).

- **`docker build` success ≠ running container uses new image** — `docker
  build` completing without error only proves the image was built. The
  running container may still use the OLD image because: (a) compose
  pulled a different tag from a registry (see Docker Hub `latest`
  pitfall), or (b) old containers were recreated but still reference the
  cached image with the same tag. Always compare the running container's
  image hash against the newly built image:
  `docker inspect <container> --format '{{.Image}}'` vs the hash printed
  at `docker build` completion. If they match the new hash, the container
  IS using the new image. If not, `docker rm -f <container>` then
  `docker-compose up -d` to force the container to start from the
  current image on disk. Also check container timestamps: `docker ps
  --format '{{.Names}}\t{{.Image}}\t{{.Status}}'` — a recently built
  image combined with an "Up 3 days" container is a sure sign the
  container wasn't replaced.

- **`docker-compose build` WITHOUT `--no-cache` can serve stale source
  even when files changed on disk** — the Dockerfile pattern of
  `COPY engine/ engine/` + a placeholder warmup step (`echo 'fn main()
  { println!("deps-warmup"); }' > engine/src/main.rs`) creates a cached
  layer. If the warmup layer is cached and the `COPY engine/` layer's
  cache key doesn't detect the source change (e.g. only mtime changed,
  not content hash), `docker-compose build` silently reuses the cached
  layer with the OLD source. The build "succeeds" but the image contains
  the pre-fix binary. Symptom: source files show the fix (grep confirms
  it), cargo build succeeds on the host, the Docker image was "built"
  after the fix, but the running container exhibits the old behavior.
  Diagnosis: `stat -c '%Y' engine/src/<file>` (source mtime) vs
  `docker inspect <image> --format '{{.Created}}'` (image build time) —
  if mtime > build time, the fix postdates the image. Fix:
  `docker-compose build --no-cache engine` forces a full rebuild from
  current source, bypassing all layer caches. Always use `--no-cache`
  when verifying that source-level fixes are actually in the running
  container, not just in the repo. This is separate from (and more
  fundamental than) the "running container uses old image" check above —
  that check catches container-not-recreated; this one catches
  image-built-from-stale-source.

- **VIX feature mismatch: scheduler stores `^VIX`, features API queries `$VIX`** —
  the scheduler correctly loads `^VIX` from Yahoo and stores it in
  `equity_candles`. But the `/api/equity/features` handler was querying
  `$VIX` (FRED's symbol). Since FRED is unreachable, `$VIX` returns 0 rows,
  `align_series` fills with 0.0, and `vix_regime` becomes 0.0 regardless of
  actual VIX level. Symptom: `vix_regime` always 0.0 in
  `/api/equity/features?symbol=QQQ` even when VIX is ~19 and the
  scheduler predictions run correctly (because the scheduler reads
  `^VIX`). Diagnosis: compare `curl /api/equity/data?symbol=^VIX` (has
  rows) vs `curl /api/equity/data?symbol=$VIX` (0 rows). Fix: make the
  features API endpoint match the scheduler's symbol (`^VIX` not `$VIX`).
  When the API defaults to a small limit (e.g. 500), the `latest` object
  may show data from years back even though newer rows exist; pass
  `?limit=1260` to see the real latest. Always make the API and scheduler
  agree on symbol names for every cross-series feature (VIX, TLT, etc.).

- **FRED unreachable from VPS — fallback to Yahoo for macro series** — the
  FRED client (`engine/src/data/fred.rs`) works in development but the
  VPS IP is blocked by `fred.stlouisfed.org`. All three macro series
  (`$VIX`, `$UST10Y`, `$DXY`) return timeout errors. This cascades into
  `vix_regime=0.0` and breaks macro-aware features. Detection: after the
  FRED backfill, check row count for `$VIX` in the DB. If ≤ 1 (i.e. only
  the seed row from schema init), call the Yahoo backfill as fallback.
  The Yahoo `^VIX` ticker uses the same REST endpoint as equity OHLCV so
  no extra infrastructure is needed. Add this to `data/mod.rs`'s
  `backfill_equities()` AFTER the FRED call:

  ```rust
  let vix_count = crate::db::count_equity_candles(pool, "$VIX").await?;
  if vix_count <= 1 {
      match yahoo::backfill(pool, "^VIX", 1, "2y").await {
          Ok(n) if n > 0 => info!(rows = n, "Yahoo ^VIX fallback loaded"),
          Ok(_) => debug!("Yahoo ^VIX returned 0 new rows"),
          Err(e) => tracing::warn!(error = %e, "Yahoo ^VIX fallback failed"),
      }
  }
  ```

  Remember to also import `debug` from `tracing` in `data/mod.rs`:
  `use tracing::{debug, info};`. Yahoo stores data as `^VIX` (not
  `$VIX`) — both scheduler AND features API must query `^VIX` for this
  to work. For other FRED series that fail (DXY, UST10Y), the same
  pattern applies with the appropriate Yahoo fallback ticker.

  **Real-world tuning on the deploy path:** once you accept that FRED
  always fails, make the FRED client fast-fail (3-5s connect timeout,
  5s whole-request) so each backfill run wastes ~10s instead of ~90s
  on three hanging requests. Do NOT chase TLS-stack fixes
  (`use_native_tls()`, switching `rustls-tls` features) — these have
  been observed to NOT resolve the underlying Akamai-edge SYN hang
  from this VPS; the only effective fix is the shorter timeout + the
  Yahoo fallback. Use the log span of the 3 FRED timeout entries as a
  regression check: should be ≤ 15s end-to-end, NOT 60-90s.
  Verification: `docker logs --tail 500 mmn-engine 2>&1 | grep 'FRED.*timed out' | tail -3 | grep -oE 'T[0-9]{2}:[0-9]{2}:[0-9]{2}' | awk '{... convert to seconds, diff ...}'` should report `span < 15s`.

- **PyTorch `CausalConv1d` — nn.Module wrapping vs nn.Conv1d subclassing
  produces different state_dict key shapes** — the notebook's
  `CausalConv1d` wraps `nn.Conv1d` as `self.conv`, producing keys like
  `blocks.0.conv1.conv.weight`. If the inference `CausalConv1d` extends
  `nn.Conv1d` directly, `self.weight` lives at the top level, producing
  `blocks.0.conv1.weight`. Same PyTorch code, different key paths →
  28 unexpected keys on `load_state_dict(strict=True)`.  **Always
  inspect checkpoint keys before writing serving code.** The fix: match
  the training architecture exactly (nn.Module wrapper, not subclass).
  See `references/pytorch-state-dict-mismatch.md` for the full diagnosis
  and verification recipe.

- **Axum WebSocket broadcast channel — three trait-import pitfalls that each
  cost a compile cycle** — when adding a live telemetry/event stream via
  `tokio::sync::broadcast` + `axum::extract::ws`, the handler splits the
  WebSocket into a sink + stream and pumps broadcast events outbound.
  Three imports are required simultaneously or you get cryptic "method not
  found" errors: (1) `WebSocket::split()` needs `use futures::StreamExt;`,
  (2) `SplitSink::send()` needs `use futures::SinkExt;`, (3)
  `SplitStream` is a `Stream` — use `.next()` (from `StreamExt`), NOT
  `.recv()`. Also: the `ws` module must be declared `pub(crate) mod ws;`
  (not plain `mod ws;`) so the scheduler/executor can reference
  `crate::api::ws::TelemetrySender`. Pass `Option<TelemetrySender>` to
  producers so tests can pass `None`. `tx.send()` returns `Err` when there
  are zero subscribers — this is normal (no WS client connected); always
  discard with `let _ = tx.send(...)`. See
  `references/axum-websocket-broadcast-channel.md` for the full wiring
  pattern (channel creation in main.rs, AppState field, router route,
  producer wiring, and all test-update call sites).

## Verification Checklist

- [ ] Image build date > HEAD commit date (image is current)
- [ ] All containers show "healthy" status
- [ ] Engine startup logs show new code executing (migrations, new features)
- [ ] API endpoints respond with expected new fields
- [ ] No new test regressions vs baseline
- [ ] Staleness/monitoring logs appear if applicable
- [ ] Model artifacts in the correct Docker volume match the new build
  (check timestamps: `docker run --rm -v deploy_models:/data alpine ls -lh /data/qqq_tcn_v1.pt`)
  — the compose `volume: models:` scope may map to the wrong named volume
  after a project rename. Always verify the mounted volume, not the local dir.
- [ ] Running container image hash matches the newly built image
  (`docker inspect mmn-engine --format '{{.Image}}'` vs `docker inspect marketmarkovnet/engine:latest --format '{{.Id}}'`)
- [ ] SPA panels show live data (not `—` dashes) — verify
  `curl -sI http://127.0.0.1:8080/views/accuracy.js | grep content-type`
  returns `text/javascript` (not `text/html` = broken static file ownership)
- [ ] Model artifacts in the correct Docker volume match the new build
  (check timestamps: `docker run --rm -v deploy_models:/data alpine ls -lh /data/qqq_tcn_v1.pt`)
  — the compose `volume: models:` scope may map to the wrong named volume
  after a project rename. Always verify the mounted volume, not the local dir.

## Reference Files

- `references/plan-gap-analysis.md` — resuming a partially-implemented
  plan: mapping commits to todos, detecting dead code vs done,
  dispatching remaining waves via subagents
- `references/marketmoves-frontend-deploy.md` — MarketMoves Vite SPA
  serving model + post-deploy hash verification (use THIS for MM frontend)
- `references/stale-data-forced-backfill.md` — diagnosing stale dashboard
  data caused by a silent WebSocket feed, and forcing a REST backfill
  refresh to verify the prediction engine works
- `references/pytorch-state-dict-mismatch.md` — diagnosing
  `nn.Module` wrapper vs `nn.Conv1d` subclass key-shape divergence in
  PyTorch checkpoints; verification recipe and fix
- `references/spa-debugging.md` — diagnosing blank SPA panels (all `—`)
  when API works; ES module import failures, HTML fallback vs JS file
  serving, ownership root-cause and fix
- `references/fred-yahoo-vix-fallback.md` — FRED CSV endpoint unreachable
  from VPS; pattern for falling back to Yahoo `^VIX` (and the
  scheduler-vs-API symbol mismatch that breaks `vix_regime` even after
  the fallback data is loaded)
- `references/rust-module-split.md` — step-by-step for splitting a
  monolithic Rust file into a module tree: visibility cascade, dead code
  cleanup, git stash limitation, and test helper exports
- `references/rust-embed-svelte-scaffold.md` — pattern for embedding a
  compiled Svelte/Vite frontend into a Rust binary via rust-embed +
  build.rs, replacing ServeDir with a zero-asset-spike fallback handler
- `references/axum-websocket-broadcast-channel.md` — wiring pattern for
  adding a live telemetry/event WebSocket endpoint (`/api/v1/ws`) to an
  Axum engine using `tokio::sync::broadcast`: channel creation, AppState
  field, producer wiring (scheduler/executor), and the three
  trait-import pitfalls (StreamExt, SinkExt, SplitStream .next() vs .recv())
