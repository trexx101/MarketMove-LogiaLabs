# Backfill feature-window bug + API field-rename contract

## Bug: Backfill fed every historical candle the same current feature window

**Symptom:** Fresh-model backfill writes N byte-identical predictions —
the model produces the same output for every historical candle because
the feature window is always "latest N candles" regardless of which
`candle_ts` is being processed.

**Root cause:** `fetch_equity_candles_asc(pool, symbol, limit)` always
returns the latest N candles. During backfill, each candle at `ts` should
get the feature window ending at `ts`, not the window ending at "now".

**Fix:** Added `fetch_equity_candles_asc_before(pool, symbol, end_ts, limit)`
that slices `ts <= end_ts`. The backfill loop in `scheduler.rs:process()`
uses this to get the correct feature window for each historical candle.

**Key lesson:** When backfilling predictions for any model, always verify
that predictions vary across backfilled candles. If they're identical,
the feature window is not being sliced correctly.

---

## Bug: API field rename causes silent N/A on dashboard

**Symptom:** Backend endpoint returns populated data, but the dashboard
renders `N/A` for every field.

**Root cause:** Backend renamed `AccuracyStats` fields from
`directional_1h/4h/24h` and `mae_1h` to `directional_1d/5d/21d` and
`mae_1d/5d/21d` (the old names were misleading — the horizons were always
daily, not hourly). The frontend `ModelHealth.svelte` still read
`accuracyData?.directional_1h` etc., so it got `undefined` for every
value.

**Detection:** The build (Vite/Svelte) does not catch this — it's a
runtime contract mismatch, not a compile-time type error. The served JS
references the old field names, the API returns the new ones, and the
gap is invisible until you load the page.

**Fix workflow after any API response field rename:**

```bash
# 1. Grep frontend for all old field names
grep -rn "directional_1h\|mae_1h\|directional_4h\|directional_24h" frontend/src/

# 2. After patching, rebuild frontend + engine image
cd frontend && npm run build
docker-compose -f deploy/docker-compose.yml build engine

# 3. Verify built JS references the new field names
grep -o "directional_1d\|mae_1d" frontend/dist/assets/*.js | sort | uniq -c

# 4. Restart and verify live API + served bundle match
docker-compose -f deploy/docker-compose.yml stop engine && \
docker-compose -f deploy/docker-compose.yml rm -f engine && \
docker-compose -f deploy/docker-compose.yml up -d engine
curl -s "http://localhost:9080/api/accuracy?symbol=SMH&since=90"
curl -s http://localhost:9080/ | grep -o 'assets/index-[A-Za-z0-9]*\.js'
```

**Key lesson:** API response field renames are a breaking contract change.
The frontend has no compile-time guard against them. Always grep + verify
the built bundle after renaming response fields.
