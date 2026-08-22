# Stale Data Diagnosis + Forced REST Backfill

After redeploying a Rust service that ingests market data, the dashboard
can show a large `staleness_secs` value (e.g. 263,471s = 3 days) even
when the engine is healthy. Two independent issues can cause this:

1. **Data feed is silent** — no new candles are being written to the DB
2. **UI is not refreshing** — the frontend only fetched `/api/status` once
   on mount and never polled again (WS `StalenessAlert` was never emitted)

The two must be diagnosed independently.

---

## Diagnosis: Data Side

### 1. Check staleness via API (engine side, not browser)

```bash
docker exec mmn-engine curl -s "http://127.0.0.1:8080/api/status" | python3 -c "
import sys,json,datetime
d=json.load(sys.stdin)
ts=d.get('last_candle_ts','?')
s=d.get('staleness_secs',-1)
print(f'staleness_secs={s}  ({s/3600:.1f}h)')
print(f'last_candle_ts={ts}')
if ts!='?':
    dt=datetime.datetime.fromisoformat(ts.replace('Z','+00:00'))
    print(f'candle date={dt.date()}  (today={datetime.date.today()})')
"
```

If `staleness_secs` is hours/days, the data feed is silent. Proceed.

### 2. Check engine logs for ingestion activity

```bash
docker logs mmn-engine 2>&1 | grep -iE 'skipping backfill|stale|sufficient equity|forcing|refresh'
```

Key log messages:
- `"sufficient equity candles — skipping backfill"` → freshness gate passed, no fetch
- `"equity candles stale — forcing refresh"` → staleness threshold triggered
- `"equity prediction persisted"` → scheduler ran with new data

### 3. Check candle count + latest ts in the DB

```bash
docker exec mmn-engine sh -c 'ls -la /app/data/'
```

If `candles.db` shows July date and today is July 30, the DB has July 27
data — three days stale for a daily-bar system.

### 4. Check disk space (common silent failure)

```bash
df -h /
docker system df
```

**Disk full blocks DB writes silently.** The engine logs will show
`database or disk is full` on upsert attempts. Fix:

```bash
docker system prune -af   # frees reclaimable dangling images + stopped containers
df -h /
```

19 GB freed is typical. After pruning, retry the forced backfill.

---

## Forced Refresh Procedure (Equities / Yahoo Finance)

The equities backfill uses a freshness gate — it skips when:
- DB has ≥ `min_candles` rows AND
- Latest candle age < `stale_threshold_secs`

For a **daily-bar system**: threshold should be ~18h (bars close 16:00 ET;
anything older than ~18h means yesterday's bar was missed).

**Immediate fix** — force via the API endpoint (always fetches, ignores gate):

```bash
docker exec mmn-engine curl -s "http://127.0.0.1:8080/api/equity/backfill?symbol=QQQ&range=5y"
# Expected: {"symbol":"QQQ","rows_loaded":N,"already_had":M,"source":"yahoo"}
```

If the response is `502 Bad Gateway` with `"database or disk is full"`, fix
disk first (step 4 above), then retry.

After forced backfill, check:
```bash
docker exec mmn-engine curl -s "http://127.0.0.1:8080/api/status" | python3 -c "
import sys,json; d=json.load(sys.stdin)
print(f'staleness={d[\"staleness_secs\"]}s  last_candle={d[\"last_candle_ts\"]}')"
```

Staleness should drop from days to hours/minutes immediately.

---

## Diagnosis: UI Side

Even after data is refreshed, the browser may still show the old staleness
value because the frontend only fetched `/api/status` once on page load.

**Symptoms:** staleness value frozen at the same number for minutes/hours
despite data being fresh; other panels (predictions, chart) may also be
stale.

**Check:** open browser DevTools → Network → filter `/api/status` — if
there's only ONE request (on page load), the frontend is not polling.

**Root cause:** WS `StalenessAlert` is defined in the schema but never
broadcast by the scheduler or ingestion. The frontend relies on it but it
never fires, so the `staleness_secs` store value is frozen at mount time.

**Fix in Svelte Dashboard** (add to `onMount` / `onDestroy`):

```svelte
let statusInterval;
onMount(async () => {
  // ... initial fetch ...
  connectWebSocket();
  // Poll /api/status every 30s as a fallback
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

---

## Long-Term Fix: Staleness Threshold Calibration

The **root code fix** is in `data/yahoo.rs::backfill()`:

```rust
// OLD (wrong for daily bars): hardcoded 3-day threshold
let stale_threshold_secs = 3 * 24 * 3600;

// NEW: threshold is a parameter passed by caller
pub async fn backfill(pool: &DbPool, symbol: &str, min_candles: i64,
                      range: &str, stale_threshold_secs: i64) -> Result<usize>
```

3 days is wrong for daily bars: after initial backfill, the 3-day gate
means 3 days pass before ANY re-fetch triggers — a missed bar sits for
3 days. 18h is appropriate: US equity bars close at 16:00 ET (~20:30 UTC);
by next morning (~13:30 UTC), the new bar is available. Anything older
than ~18h means the bar was missed.

Update the call chain:
```
yahoo::backfill(..., stale_threshold_secs)
yahoo::backfill_many(..., stale_threshold_secs)
data::backfill_equities(pool, stale_threshold_secs)
  main.rs startup:     data::backfill_equities(&pool, 3*24*3600)
  data/mod.rs top-up: data::backfill_equities(&topup_pool, 18*3600)
  api/equity.rs:      yahoo::backfill(..., 0)  // always fetch
```

---

## Compose Note: `docker-compose` vs `docker compose`

On this VPS: `docker-compose` (v1) is broken (`KeyError: 'ContainerConfig'`);
`docker compose` (v2 plugin) works fine. Use `docker compose` for compose
operations. For individual container restart, bypass compose entirely:

```bash
# Get current container name (may have random suffix from compose project rename)
docker ps --format '{{.Names}}'

# Remove old container
docker rm -f c2743bbddd72_mmn-engine

# Restart with same image + flags
docker run -d \
  --name mmn-engine \
  --restart unless-stopped \
  --user 1000:1000 \
  --network deploy_mmn \
  -v deploy_data:/app/data \
  -v deploy_models:/models:ro \
  --env-file /path/to/.env \
  -e TRADING_MODE=paper \
  -e ZMQ_ENDPOINT=tcp://inference:5555 \
  ...other env vars... \
  --health-cmd "curl --silent --fail --max-time 4 http://127.0.0.1:8080/api/status | grep -q '\"mode\":\"paper\"\\|\"mode\":\"live\"' || exit 1" \
  --health-interval 15s --health-timeout 5s --health-start-period 20s --health-retries 3 \
  marketmarkovnet/engine:latest
```

After restart: `docker inspect --format='{{.State.Health.Status}}' mmn-engine`
should show `healthy` within ~30s.

---

## Shell Quoting Gotcha (DB introspection)

`sqlite3` inside `docker run ... sh -c '...'` with nested double-quotes
for timestamp queries will fail. Use Python inside the container instead:

```bash
docker exec mmn-engine /bin/sh -c 'python3 -c "
import sqlite3, datetime
conn = sqlite3.connect(\"candles.db\")
cur = conn.cursor()
cur.execute(\"SELECT symbol, ts, close FROM equity_candles WHERE symbol=chr(81)||chr(81)||chr(81) ORDER BY ts DESC LIMIT 3\")
for sym, ts, close in cur.fetchall():
    age = (datetime.datetime.now(datetime.timezone.utc).timestamp() - ts)
    print(f\"{sym}  ts={ts} ({datetime.datetime.fromtimestamp(ts, tz=datetime.timezone.utc).date()})  close={close}  age={age:.0f}s\")
conn.close()
"'
```

(QQQ = chr(81)*3 to avoid SQL string quoting issues.)
