# Merge Recovery & Engine Boot Crash-Loop (MarketMoves)

Two validated, non-obvious failure modes from the 2026-08-19 merge of
`feature/nvda-multi-asset-and-sentiment-overlay` into Phase 7 HEAD. Both are
reproducible and the fixes below are confirmed working.

---

## Failure 3 — `trading_models.norm_stats_path` prefix mismatch (silent `process::exit(1)` crash-loop)

### Symptom
Container `restarting` / `unhealthy`, exit code 1. Log ends at:

    INFO engine: bootstrapping scheduler model_idx=2 model_id=qqq-v1 primary=QQQ ...
    norm_stats error for SMH/SOXS: No such file or directory (os error 2)

…then the process exits. No Rust panic, no backtrace — just the eprintln in
`main.rs` bootstrap loop followed by `process::exit(1)`.

### Root cause (DISTINCT from "file genuinely missing")
The `trading_models` registry rows stored `norm_stats_path = /models/SMH/norm_stats_smh_v1.json`
(leading slash → root `/models`), but the volume is mounted at `/app/models`.
So the file EXISTS at `/app/models/SMH/norm_stats_smh_v1.json` but the path
points to `/models/SMH/...` which does not exist. The same-branch QQQ model
worked before only because `NORM_STATS_PATH` env passed a flat
`/app/models/norm_stats_qqq_v1.json`.

### Diagnostic (run against the data volume)
```bash
docker run --rm -v deploy_data:/app/data alpine sh -c \
  "apk add --no-cache sqlite >/dev/null 2>&1; \
   sqlite3 /app/data/candles.db 'SELECT model_id, norm_stats_path, enabled FROM trading_models;'"
```
If the path starts with `/models/` (not `/app/models/`) → mismatch.
Confirm the file is actually present:
```bash
docker run --rm -v deploy_models:/app/models alpine sh -c "find /app/models -name 'norm_stats*'"
```

### Fix — UPDATE the rows (do NOT re-seed; the data is live)
```bash
docker run --rm -v deploy_data:/app/data alpine sh -c \
  "apk add --no-cache sqlite >/dev/null 2>&1; \
   sqlite3 /app/data/candles.db \
   \"UPDATE trading_models SET norm_stats_path = REPLACE(norm_stats_path, '/models/', '/app/models/') WHERE norm_stats_path LIKE '/models/%';\""
```
Then `docker restart mmn-engine`.

### Re-occurrence (2026-08-20)
Same crash-loop re-appeared after rebuilding the engine image with new code
(tape recorder endpoint). The `trading_models` rows still had the wrong prefix.
The diagnostic command needs `sqlite3` inside the container — the engine
image does NOT have `sqlite3` installed. Use the alpine sidecar approach above,
or query via the engine's API if available. The norm_stats files DO exist at
`/models/SMH/norm_stats_smh_v1.json` (the Docker volume `deploy_models` is
mounted at `/models` inside the container, NOT `/app/models`). The mismatch is
that `trading_models` rows point to `/app/models/SMH/...` but the volume mount
is at `/models/SMH/...`. Check the compose file's volume mount path to
determine which prefix is correct for the current deployment.

---

## Failure 4 — docker-compose v1.29.2 `ContainerConfig` error on image rebuild

### Symptom
After rebuilding the engine image (`docker build -f engine/Dockerfile -t
marketmarkovnet/engine:latest .`), `docker-compose -f deploy/docker-compose.yml
up -d engine` fails with:

    ERROR: for mmn-engine  'ContainerConfig'

### Root cause
docker-compose v1.29.2 (Python implementation, not Go-based `docker compose`)
has a bug where it tries to read `ContainerConfig` from the old container's
image manifest. Newer Docker daemon versions don't populate this field in the
same way, causing the Python client to crash during the convergence plan
(comparing old vs new container config).

### Fix — remove old container first, then `up`
```bash
docker stop mmn-engine 2>/dev/null
docker rm mmn-engine 2>/dev/null
# Also remove any orphaned recreated containers
docker ps -a --format '{{.ID}} {{.Names}}' | grep -i engine
docker rm -f <orphaned_container_id>
# Now create fresh
docker-compose -f deploy/docker-compose.yml up -d engine
```
The key: compose must CREATE a new container, not RECREATE from the old one.
If any stopped container with a name matching the compose service exists,
compose will try to recreate it and hit the bug.

---

## Failure 5 — Engine healthcheck `start_period` too short → restart loop

### Symptom
Engine container shows `health: starting` forever, restart count climbing.
Healthcheck logs show exit code 1 with empty output. The HTTP server never
binds because Docker kills the container before startup completes.

### Root cause
The engine's startup sequence is synchronous and takes ~60-70 seconds:
1. Equity OHLCV backfill (Yahoo Finance) — ~3s
2. CBOE VIX backfill — ~0.5s
3. FRED macro series (UST10Y, DXY) — ~2s
4. Sentiment cache seeding (13 symbols × Finnhub, all 403) — ~8s
5. Per-model bootstrap (norm_stats load, scheduler spawn) — ~2s per model
6. Only THEN does `TcpListener::bind` execute and HTTP server start

The healthcheck had `start_period: 20s, retries: 3` → 20s + 3×15s = 65s
total tolerance. The engine needs ~70s. One bad timing run and Docker
kills it, it restarts, and the cycle never breaks.

### Fix
```yaml
healthcheck:
  test: ["CMD-SHELL", "curl --silent --fail --max-time 4 http://127.0.0.1:8080/api/status | grep -q '\"mode\":\"paper\"\\|\"mode\":\"live\"' || exit 1"]
  interval: 15s
  timeout: 5s
  start_period: 90s   # was 20s
  retries: 5          # was 3
```

### Diagnostic
```bash
docker inspect mmn-engine --format '{{json .State.Health.Log}}' | python3 -c \
  "import sys,json; logs=json.load(sys.stdin); [print(f'{l[\"Start\"]} exit={l[\"ExitCode\"]}') for l in logs[-5:]]"
```
If all recent entries show `exit=1` with empty output, the HTTP server
isn't up yet. Check `docker logs mmn-engine 2>&1 | grep "http server listening"`
— if that line never appears, the engine hasn't reached the TCP bind.
