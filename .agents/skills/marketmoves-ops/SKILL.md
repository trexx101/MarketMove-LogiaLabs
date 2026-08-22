---
name: marketmoves-ops
description: "Operate the MarketMoves trading stack."
version: 1.0.0
metadata:
  hermes:
    category: project
    tags: [marketmoves, devops, deployment, inference, trading, operations]
---

# MarketMoves live operations

> **Local engine boot & endpoint smoke test** (port contention, NORM_STATS_PATH, startup delay, kill-by-PID): see `references/local-engine-boot-smoke-test.md`.

Use this skill when:
- Rebuilding / redeploying engine or inference containers on the VPS.
- Swapping models (adding/removing symbols from `trading_models`).
- Restarting inference after model-artifact or code changes.
- Calibrating or verifying strategy thresholds.
- Diagnosing zero/missing predictions, prediction-magnitude blowups, or failed container starts.
- Managing the options tape recorder (start/stop/restart, troubleshooting failed chain discovery or heartbeat errors).
- Booting the engine locally to smoke-test new API endpoints: see `references/local-engine-smoke-test.md` (port 8080 collision is silent, `NORM_STATS_PATH` required, ~8s startup, config PUT partial-apply round-trip).

## PnL and trade recording

**Pitfall — executor loses position state on restart.** `PaperExecutor`
starts as `Position::Flat` on every container restart, ignoring the DB
position. This silently skips exit trades → `equized_pnl` stays 0,
dashboard shows "Waiting for PnL data." Fix: `sync_from_db()` is called
after construction in `build_paper_executor_for_model()`. See
`references/paper-executor-and-pnl-pipeline.md` for full root-cause
analysis of the three PnL bugs (executor sync, model_id attribution,
PnL curve fetch scope).

**Pitfall — backfill feeds wrong feature window.** During fresh-model
backfill, each historical candle must get the feature window ending at
that candle's `ts`, not the latest N candles. `fetch_equity_candles_asc`
always returns the latest N — use `fetch_equity_candles_asc_before` for
backfill. If backfilled predictions are byte-identical across candles,
the feature window is not being sliced correctly. See
`references/backfill-and-api-field-rename.md`.

**Pitfall — API field rename breaks the dashboard silently.** When you
rename response fields on the backend (e.g. `directional_1h/4h/24h` →
`directional_1d/5d/21d`), the frontend still reads the old names and
renders `undefined` → `N/A`. **Always `grep -rn "old_field_name"
frontend/src/` after any API response field rename.** The build won't
catch this — it's a runtime contract mismatch, not a type error.

## Full deploy workflow (engine + frontend)

**Docker Compose is unreliable on this host** (`docker-compose` v1 has
`KeyError: 'ContainerConfig'` issues; `docker compose` v2 errors with
`unknown shorthand flag: 'f'`). Use direct `docker` commands instead.

### Rebuild and redeploy the engine container

**On this host the engine is a bare `docker run`, not compose-managed** (no
`com.docker.compose.*` labels), so recreate it by hand. To reproduce the
container byte-identically, **inspect the live container first** rather than
trusting compose/Dockerfile: `docker inspect mmn-engine` for `Config.Env`,
`HostConfig.RestartPolicy`, `HostConfig.ExtraHosts`, `HostConfig.PortBindings`,
`Mounts`, `NetworkSettings.Networks`. The deployed engine has **no published
`-p` ports** (Caddy proxies only; `--expose 8080` is internal). Every rebuild
must keep `--add-host host.docker.internal:host-gateway` and the
`deploy_data`/`deploy_models` volumes, or OpenD access and the SQLite DB
silently drop.

```bash
# 1. Build the Rust binary
cargo build --release

# 2. Build the Docker image (copies binary + frontend SPA)
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .

# 3. Stop, remove, recreate
docker stop mmn-engine && docker rm mmn-engine
docker run -d --name mmn-engine --restart unless-stopped \
  --user 1000:1000 \
  --network deploy_mmn \
  --add-host host.docker.internal:host-gateway \
  -v deploy_data:/app/data \
  -v deploy_models:/models:ro \
  --expose 8080 \
  --env-file /home/ubuntu/projects/MarketMoves/.env \
  marketmarkovnet/engine:latest
```

**Wait for healthy** (up to 90s due to `start_period`):
```bash
watch -n 5 'docker inspect mmn-engine --format "{{.State.Health.Status}}"'
```

### Smoke-test after redeploy

The proxy (port 9080) requires basic auth. Test endpoints directly
inside the container:

```bash
# Status (with per-symbol support)
docker exec mmn-engine curl -s http://127.0.0.1:8080/api/status | python3 -m json.tool
docker exec mmn-engine curl -s 'http://127.0.0.1:8080/api/status?symbol=NVDA' | python3 -m json.tool

# Chart — stale flag should be False for symbols with recent data
docker exec mmn-engine curl -s 'http://127.0.0.1:8080/api/chart?limit=5&symbol=QQQ' | python3 -c "import json,sys; d=json.load(sys.stdin); print('stale:', d['stale'], 'candles:', len(d.get('candles',[])))"

# Predictions
docker exec mmn-engine curl -s 'http://127.0.0.1:8080/api/predictions?symbol=SMH' | python3 -m json.tool | head -15

# Accuracy
docker exec mmn-engine curl -s 'http://127.0.0.1:8080/api/accuracy?symbol=NVDA' | python3 -c "import json,sys; d=json.load(sys.stdin); print('1d dir:', d.get('directional_1d'), 'resolved:', d.get('resolved_count'))"
```

### Frontend-only deploy

The engine Docker image embeds the SPA from `frontend/dist/`. If you
only changed frontend code:

```bash
cd frontend && npm run build
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .
docker stop mmn-engine && docker rm mmn-engine
docker run -d --name mmn-engine --restart unless-stopped \
  --user 1000:1000 --network deploy_mmn \
  --add-host host.docker.internal:host-gateway \
  -v deploy_data:/app/data -v deploy_models:/models:ro \
  --expose 8080 \
  --env-file /home/ubuntu/projects/MarketMoves/.env \
  marketmarkovnet/engine:latest
```

### View engine logs

```bash
docker logs mmn-engine --tail 50 -f
```

### Nightly hyperopt run & deployment runbook

**Inah expects the per-service redeploy runbook to be maintained, not reverse-engineered at deploy time.** The single source of truth lives at **repo root `DEPLOYMENT_SUMMARY.md`** — it documents the 6 services (engine / inference / caddy as docker; options-recorder / opend / hermes-gateway as `systemctl --user` units), the exact `docker run` / systemd spec for each, health checks, volume/network invariants, and gotchas. When you perform any deploy, update that file if the spec changed. Read it before deploying instead of re-deriving from `docker inspect` every time.

The nightly hyperopt is a **self-waking loop inside the engine**: polls every 30 min, gate on `SchedulerState::CanRun`, and runs **at most once per window** (20:30→04:30 UTC). To confirm the pipeline fired:

```bash
docker logs mmn-engine | grep -E 'hyperopt run|Starting nightly|Nightly run complete'
# expected: "hyperopt run: success=true candidates=N equities=3"
# first fire is IMMEDIATE at boot if inside the window; otherwise waits for next 20:30 UTC.
```

RunnerConfig default universe (QQQ/SMH/XLF) is the single source of truth for **both** the nightly candidate producer and the promotion applier in `main.rs` — widen equities there only. See `references/hyperopt-runbook-and-scheduler-loop.md` for the wrong-vs-right loop pattern and the open negative-IC finding.

## Core principles

1. **Docker Compose v1.29.2 against a modern daemon** has a well-known `KeyError: 'ContainerConfig'` when `up -d` inspects an existing container. The fix is to remove the stale container before recreating it (see `references/docker-compose-v1-workaround.md`).
2. **Engine config precedence** for thresholds and other env vars is: current shell env > `engine/Dockerfile` `ENV` defaults > `deploy/docker-compose.yml` defaults. If a value refuses to change, check `env | grep THRESHOLD` and `docker exec mmn-engine env`.
3. **Model artifacts live in the `models/` named volume**, mounted read-only at `/models`. The inference loader discovers per-symbol bundles under `/models/<SYMBOL>/` (`model_meta_<sym>_v1.json`, `norm_stats_<sym>_v1.json`, `<sym>_tcn_v1.pt`, `<sym>_lgbm_h{1,5,21}_v1.pkl`).
4. **Multi-model trading is driven by the `trading_models` SQLite table.** One scheduler is spawned per enabled row. The engine falls back to a single `SYMBOL`/`SHORT_SYMBOL` bootstrap model only when the table is empty.
   - **Pitfall — newly-enabled models don't backfill.** The scheduler's `run()` only `process()`es a candle when `ts > last_processed_ts`. For a fresh model `last_processed_ts` is `None`, so it logs "starting fresh" and waits for the *next* new daily candle — it backfills only if a prior prediction exists. Result: a model enabled today shows exactly 1 prediction and builds history forward only. "Only 1 prediction for SMH/XLF" is almost always this, not a broken pipeline. See `references/db-inspection-and-multimodel-accuracy.md`.
   - **`/api/accuracy` is single-symbol + windowless.** Endpoint pins `symbol` to `cfg.symbol` (QQQ); `fetch_equity_accuracy` has a hardcoded `LIMIT 500` and no date filter. There is no `?symbol=`/`?since=` param. Per-model, time-scoped accuracy requires code changes (see reference).
5. **Predictions are in raw log-return space only if the `atr_ratio` de-normalization is correct.** `label_std` stored in `model_meta_*` is the std of the *ATR-scaled* labels (`return / ATR_ratio`). Inference must multiply by the current `atr_ratio` to return to raw returns.
6. **Sector ETF inverse symbols are usually leveraged.** SMH has no clean -1x inverse; the common inverse tickers are `SOXS` (3×) for semis and `FAZ` (3×) for financials. Size/hedge accordingly or run long-only.

## Quick checks

| Symptom | Check |
|---|---|
| Dashboard shows zero predictions | `SELECT * FROM equity_predictions ORDER BY candle_ts DESC LIMIT 5;` — delete bad zero rows, restart engine to backfill. |
| Predictions look huge (>100%) | Inference de-norm is missing `atr_ratio`. Verify `label_std` is ATR-scaled and the code does `pred * label_std * atr_ratio`. |
| Threshold change in `.env` has no effect | Check shell env with `env` and container env with `docker exec mmn-engine env`. `unset` stale shell vars. |
| `docker-compose up -d` crashes with `KeyError: 'ContainerConfig'` | Remove the affected container(s) first, then `up -d`. See references. |
| New symbol not trading | Verify row in `trading_models`, per-symbol model files in `/models/<SYMBOL>/`, and inference logs loading the bundle. |
| `mmn-proxy` shows `unhealthy` in `docker ps` | **Cosmetic false negative.** Caddy's healthcheck `wget http://127.0.0.1:80/` hits the auth-gated catch-all route → 401 → reports unhealthy permanently even while proxying fine. Check `docker logs mmn-proxy` for the *real* error: `502 dial tcp ... 8080 connection refused` means the engine was briefly down at that timestamp, not a current proxy fault. Do not block a deploy on `unhealthy` proxy. |
| Inference container `Exited` but engine still healthy | The engine ingests equity candles **directly from Yahoo** (`equity candles stale — forcing refresh ... count=1255`), so equity/Hyperopt work without inference. Only ML predictions stall at `actuals: updated predictions updated=0`. Restore with `docker start <inf-container>` (currently `576a1d601953_mmn-inference`). |

## Model-swap procedure

1. Place model bundle in `models/<SYMBOL>/` with all required files.
2. Ensure `model_meta_<symbol>_v1.json` contains `label_mean_1d/5d/21d` and `label_std_1d/5d/21d` keys.
3. Copy the directory into the `deploy_models` volume.
4. Insert or update the `trading_models` row (`enabled=1`, correct `norm_stats_path`, `inverse_symbol`, `budget_usd`).
5. Restart `mmn-engine`.
6. Verify in engine logs: `bootstrapping scheduler ... primary=<SYMBOL>` and `equity prediction persisted ... symbol=<SYMBOL>`.

## Threshold calibration

- Predictions are raw log returns after the `atr_ratio` fix.
- Start from the distribution of historical `pred_1d`/`pred_5d`/`pred_21d` for the symbol.
- A good first entry threshold is near the 75th percentile of `pred_1d` for long-only symbols, or use the same base threshold across symbols and tune per-model via the API.
- Per-model strategy params exist in memory; the API endpoint is `PUT /api/strategy-config?model_id=<model_id>`.

## Pitfalls

- **Do not trust shell env overrides silently.** Compose passes current shell variables before `.env`. Always `unset` stale vars after editing `.env`.
- **Do not leave inverse_symbol blank.** The executor/scheduler require a value. Use a real leveraged inverse as a placeholder and disable shorting at the strategy level if you do not want shorts.
- **Old zero predictions block backfill.** The scheduler uses `last_processed_ts`; zero rows from the old inference code prevent re-processing unless deleted.
- **10-sample inference warmup.** For the first 10 days after an inference restart, z-scoring is disabled and a raw 0.5/0.5 blend is used. Predictions during this window are valid but noisier.

## References

- `references/docker-compose-v1-workaround.md` — ContainerConfig KeyError fix.
- `references/caddy-auth-bypass-internal-api.md` — two `route` blocks: one for `/api/internal/*` (no auth), one for everything else with `basic_auth`.
- `references/reach-engine-and-db-directly.md` — bypass Caddy: 401 on :9080 is Caddy not the engine; hit engine on the `deploy_mmn` network via a throwaway `curlimages/curl` container; read the root-owned SQLite volume DB via `docker cp` + host `sqlite3` read-only (no sudo).
- `references/options-recorder-operations.md` — systemd service, Python venv, troubleshooting, parquet output.
- `references/model-artifact-units.md` — label_std, atr_ratio, and return-space de-normalization.
- `references/trading-models-registry.md` — Schema and workflow for adding/removing models.
- `references/db-inspection-and-multimodel-accuracy.md` — `sudo python3` recipe for the root-owned Docker volume DB; multi-model accuracy gap (fresh-model-no-backfill + `/api/accuracy` windowless).
