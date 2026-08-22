# Reach the engine directly + read the DB (behind Caddy, root-owned volume)

Captured 2026-08-22. The dashboard proxy (`mmn-proxy`/Caddy on :9080) wraps
everything in `basic_auth`, and the DB volume is root-owned — two barriers to
hands-on inspection. Both have clean bypasses that need NO credentials and NO
sudo.

## 401 from `:9080` = Caddy, not the engine

Caddy publishes `:9080→mmn-engine:8080` with `basic_auth`. A raw
`curl http://localhost:9080/api/...` returns **401** because you sent no
credentials. The engine itself has NO auth middleware on most `/api/*` routes
(only CORS) — so reachable directly on the internal network. In the browser the
page works because your cached Basic auth rides along on every same-origin
request.

**Never assume 401 means a broken endpoint.** Confirm the route exists by
hitting the engine directly (below) before blaming code.

## Hit the engine directly — throwaway container on the internal network

Robust even if the engine image lacks `curl`/`sqlite3` (the documented
`docker exec mmn-engine curl` only works when curl is installed in the image):

```bash
# internal network name (all deploy containers share it):
NET=$(docker inspect mmn-engine --format '{{range $k,_ := .NetworkSettings.Networks}}{{$k}}{{end}}')

docker run --rm --network "$NET" curlimages/curl:latest \
  -s -m 8 http://mmn-engine:8080/api/hyperopt/QQQ/candidates
```

Read-only GETs — safe against a live engine. Confirmed working for the
hyperopt endpoints (candidates/status/runs) and applies to the other `/api/*`.

## Read the SQLite DB without sudo

The host can't read the volume dir directly (`/var/lib/docker/volumes/deploy_data/_data/`
is root-owned; `cp` silently fails on stderr). `docker cp` runs via the root
daemon and reads it fine. Copy read-only, never open the live file for writes:

```bash
docker cp mmn-engine:/app/data/candles.db /tmp/candles_ro.db
sqlite3 "file:/tmp/candles_ro.db?mode=ro" "SELECT status, COUNT(*) FROM strategy_versions GROUP BY status;"
```

`candles.db` holds `equity_candles`, `strategy_versions`, `pending_promotions`,
`hyperopt_runs`, `option_positions`, `engine_events`, etc. (Alternative to the
`sudo python3` recipe in `db-inspection-and-multimodel-accuracy.md`.)