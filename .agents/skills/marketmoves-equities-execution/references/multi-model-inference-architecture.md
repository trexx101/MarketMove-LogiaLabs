# Multi-Model Inference Architecture

Session: 2026-08-13. Captures three bugs found and fixed in the
MarketMoves multi-model inference + dashboard stack.

## Bug 1: `fetch_equity_candles_asc` returned oldest candles, not latest

### Root cause

`engine/src/db.rs::fetch_equity_candles_asc` used:
```sql
SELECT ... FROM equity_candles WHERE symbol = ?1 ORDER BY ts ASC LIMIT ?2
```

With 1254 rows in the table and `LIMIT 186`, this returns the **oldest**
186 candles (August 2021 data), not the latest 186 (August 2026).

### Symptoms

- QQQ SMA40 computed as 340.42 (should be 710.97) — the engine was
  averaging 2021 prices (~$340) instead of 2026 prices (~$711).
- Regime computed as "bear" (close $723 > SMA $340 = should be bullish,
  but the engine was loading 2022 candles where close was ~$297 < SMA
  ~$340 = bearish).
- Zero trades: in bearish regime, `next_equity_position` allows no long
  entries. QQQ was flat despite pred_1d=0.025 > entry_threshold=0.001.
- NVDA predictions stuck at 0.0: features computed on 2021 NVDA prices
  (~$20) produced garbage feature vectors that the model couldn't
  extract signal from.
- Status API `sma_200` was 370.52 instead of 649.53 — exactly the
  average of QQQ SMA200 (649.53) and PSQ SMA200 (28.92), because the
  scheduler loaded candles for both symbols through the same broken
  query.

### Fix

Changed to subquery pattern:
```sql
SELECT ... FROM (
  SELECT ... FROM equity_candles WHERE symbol = ?1 ORDER BY ts DESC LIMIT ?2
) ORDER BY ts ASC
```

Inner query gets the latest N (DESC), outer re-sorts to chronological
order (ASC). This is the correct pattern for "latest N candles in
chronological order" in SQLite.

### Reproduction recipe

```python
import sqlite3
conn = sqlite3.connect('/tmp/candles.db')
c = conn.cursor()

# Wrong (old behavior):
c.execute("SELECT ts, close FROM equity_candles WHERE symbol='QQQ' ORDER BY ts ASC LIMIT 186")
rows = c.fetchall()
print(f"Oldest: ts={rows[0][0]}, close={rows[0][1]}")  # 2021 data!
print(f"Newest: ts={rows[-1][0]}, close={rows[-1][1]}")

# Correct (new behavior):
c.execute("""SELECT ts, close FROM (
  SELECT ts, close FROM equity_candles WHERE symbol='QQQ' ORDER BY ts DESC LIMIT 186
) ORDER BY ts ASC""")
rows = c.fetchall()
print(f"Oldest: ts={rows[0][0]}, close={rows[0][1]}")  # recent data
print(f"Newest: ts={rows[-1][0]}, close={rows[-1][1]}")  # latest
```

### Detection

The bug was detected by comparing the SMA stored in the `positions`
table against an independent SQLite query:
```sql
SELECT AVG(close) FROM (
  SELECT close FROM equity_candles WHERE symbol='QQQ' ORDER BY ts DESC LIMIT 40
)
```
If the stored SMA doesn't match the computed SMA, the candle query is
wrong.

---

## Bug 2: Healthcheck z-score buffer pollution

### Root cause

The inference Docker healthcheck sends a minimal ZMQ request:
```json
{"schema_version": 3, "feature_window": [[0.0]*8]}
```

This request has `symbol=""` (no symbol field) and `seq_len=1` with
all-zero features. The ensemble selection fallback
`ensembles.get(symbol) or next(iter(ensembles.values()))` routes
these to the **NVDA** ensemble (alphabetically first: `NVDA` before
`QQQ`).

Each healthcheck calls `ensemble.predict()` which pushes raw predictions
into the z-score buffers. Over 6 days at 30s intervals, ~16,000
zero-predictions flooded the NVDA buffers. Every real NVDA prediction
was then z-scored against a buffer full of zeros, producing ~0.0.

### Symptoms

- NVDA predictions persisted as 0.0 in `equity_predictions` table.
- Inference logs showed `symbol=""` for all 16,454 requests — no real
  symbol-tagged requests since Aug 7.
- QQQ predictions were non-zero (0.025) because healthchecks were
  routed to NVDA, not QQQ.

### Fix

Added healthcheck detection in `_handle_request`:
```python
is_healthcheck = (
    len(feature_window) == 1
    and all(abs(v) < 1e-12 for v in feature_window[0])
)
```

Pass `skip_buffer=True` to `ensemble.predict()`. The predict method
skips buffer appends when `skip_buffer=True`:
```python
if not skip_buffer:
    self._tcn_buffer[h].append(tcn_raw)
    self._lgbm_buffer[h].append(lgbm_raw)
```

Healthcheck still returns valid JSON (passes the healthcheck) but no
longer pollutes prediction statistics.

---

## Bug 3: Svelte derived store `.set()` error

### Root cause

`CandlestickChart.svelte` called `chartData.set(data)` on line 61.
But `chartData` is a `derived` store (created via `derived(...)`
in `stores.js`). Svelte derived stores are **read-only** — they don't
have a `.set()` method. The minified variable name `Ll` in production
builds maps to `chartData`, so the browser console showed:
`Ll.set is not a function`

### Symptoms

- Chart refresh failed silently on every 30s poll and model switch.
- The `catch` block logged `chart refresh failed: Ll.set is not a function`
  but the chart appeared frozen (no new candles loaded after initial mount).

### Fix

Replaced `chartData.set(data)` with `updateSlice($activeModelId, 'chartData', data)`.
The `updateSlice` function writes to the underlying `_slices` writable
store, which `chartData` derives from. This is the correct write path
for per-model telemetry.

### General rule

Any Svelte store created with `derived(...)` is read-only. To write
data that a derived store reads from, write to the underlying writable
store. In this codebase, that's `_slices` via `updateSlice(modelId, field, value)`.

---

## Multi-model inference service changes (full file list)

### Engine (Rust)
- `engine/src/bridge.rs` — `predict_v3` and `predict_v3_with_retry` accept `symbol: &str` parameter, sent in ZMQ request as `"symbol": symbol`
- `engine/src/scheduler.rs` — passes `self.symbol` to `predict_v3_with_retry`
- `engine/src/api/equity.rs` — backtest replay passes `state.symbol` to bridge
- `engine/src/db.rs` — `fetch_equity_candles_asc` fixed with subquery pattern; `equity_predictions` unique constraint changed from `candle_ts` to `(symbol, candle_ts)`; `migrate_multi_model()` adds `model_id`/`symbol` columns to existing tables

### Inference (Python)
- `inference/equity_model.py` — `EquityEnsemble` class with per-horizon z-score buffers; `_handle_request` routes by symbol and detects healthchecks; `run_service` takes `ensembles: dict[str, EquityEnsemble]`; `main()` discovers per-symbol model bundles from subdirectories

### Frontend (Svelte)
- `frontend/src/lib/components/CandlestickChart.svelte` — replaced `chartData.set(data)` with `updateSlice(...)`; dynamic title with active model symbols; reactive re-fetch on model switch
- `frontend/src/lib/components/StatusPanel.svelte` — active model display, no stale-data label
- `frontend/src/lib/components/ModelHealth.svelte` — reactive accuracy fetch, 1d/5d/21d labels
- `frontend/src/lib/components/TradeHistory.svelte` — global trades with model_id/symbol columns
- `frontend/src/views/Dashboard.svelte` — passes model_id/symbol to all fetchers; 30s status poller
- `frontend/src/lib/api.js` — model_id/symbol params on fetch helpers

### Deploy
- `deploy/docker-compose.yml` — added `MODELS_DIR=/models` to inference service

---

## Verification commands

```bash
# Check SMA is correct (should match independent SQLite computation)
curl -s "http://localhost:9080/api/status?model_id=qqq-v1&symbol=QQQ" | python3 -c "
import sys,json; d=json.load(sys.stdin); print(f'sma_200={d[\"sma_200\"]:.2f} close={d[\"last_close\"]:.2f} regime={\"bull\" if d[\"last_close\"] > d[\"sma_200\"] else \"bear\"}')"

# Check inference loaded both symbols
docker logs mmn-inference 2>&1 | grep "loaded ensembles for symbols"

# Check healthcheck is NOT polluting buffers (symbol="" is fine, no real predictions affected)
docker logs mmn-inference 2>&1 | grep '"symbol":""' | wc -l  # healthcheck count
docker logs mmn-inference 2>&1 | grep '"symbol":"NVDA"' | wc -l  # real NVDA requests
docker logs mmn-inference 2>&1 | grep '"symbol":"QQQ"' | wc -l  # real QQQ requests

# Verify no chartData.set() calls remain in frontend
grep -n "chartData.set" frontend/src/lib/components/CandlestickChart.svelte
# Should return nothing
```
