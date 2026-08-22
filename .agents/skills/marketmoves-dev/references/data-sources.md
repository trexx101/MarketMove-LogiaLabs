# MarketMoves Data Sources — Status (2026-08-01 post-consolidation)

## Data Source Routing (priority order)

| Priority | Source | Data | Symbols | Status |
|----------|--------|------|---------|--------|
| 1 (equities) | **Moomoo OpenD** | OHLCV + live quote | QQQ + 10 constituents | 🔜 Needs OpenD on VPS |
| 1f (equities fallback) | **Yahoo Finance** | OHLCV + live quote | QQQ + 10 constituents | ✅ Active |
| 1 (VIX) | **CBOE CSV** | VIX daily | $VIX | ✅ Active (1286 rows) |
| 1f (VIX fallback) | **Yahoo ^VIX** | VIX | ^VIX | ✅ Active (503 rows) |
| 1 (macro) | **FRED JSON API v2** | 10Y yield, DXY | $UST10Y, $DXY | ✅ Active (1247+1244 rows) |
| 1 (sentiment) | **Stub (neutral)** | Daily sentiment | All symbols | ✅ Stub active |
| 2 (sentiment) | **Finnhub** | News sentiment | All symbols | 🔜 Phase 2 |

## How routing works

At startup and on daily top-up, `backfill_equities()` in `data/mod.rs`:

1. **Equities**: TCP-check `moomoo::is_available()` (FUTU_OPEND_HOST:PORT).
   If reachable → Moomoo `get_kline.py`. If Moomoo fails per-symbol →
   Yahoo fallback for that symbol. If OpenD not reachable at all →
   Yahoo for all.

2. **VIX**: CBOE CSV from `cdn.cboe.com` (free, no auth). If CBOE fails
   (parses 0 rows) → Yahoo `^VIX` fallback.

3. **Macro**: FRED JSON API v2 (`api.stlouisfed.org`). Requires
   `FRED_API_KEY` env var. Gracefully logs warning and skips if key missing.

4. **Sentiment**: Phase 1 stub returns 0.5 for all symbols. `sentiment_cache`
   table seeded at startup. Phase 2 wires Finnhub `news_sentiment`.

## CBOE VIX — implementation notes

- **URL**: `https://cdn.cboe.com/api/global/us_indices/daily_prices/VIX_History.csv`
- **Format**: `DATE,OPEN,HIGH,LOW,CLOSE` with dates as `MM/DD/YYYY`
- **CRITICAL PITFALL**: Date format is `%m/%d/%Y`, NOT `%Y-%m-%d`. Using the wrong
  format string causes `parse_cboe_csv` to return 0 rows (all dates fail
  to parse) and the message "CBOE VIX: parsed 0 valid rows". The fallback
  triggers silently (Yahoo ^VIX), so this bug can go undetected — verify
  data landed with:
  `docker logs mmn-engine | grep "CBOE VIX backfill complete fetched="`
- **Coverage**: 1990-01-02 to present, ~11k rows. `range_days` parameter
  filters rows by cutoff.
- **Volume**: Always 0 (not provided by CBOE).
- **Trade source**: Stored as `source = "cboe"` in `equity_candles`.

## FRED JSON API v2 — implementation notes

- **URL**: `https://api.stlouisfed.org/fred/series/observations`
- **Query params**: `series_id=DGS10&api_key=KEY&file_type=json&output_type=1`
- **Series mapping**:
  - `$VIX` → `VIXCLS` (now sourced from CBOE, mapping retained)
  - `$UST10Y` → `DGS10`
  - `$DXY` → `DTWEXBGS`
- **Response**: `{"observations": [{"date": "2024-01-02", "value": "13.42"}, ...]}`
- **Missing values**: Encoded as `"."` — skip them.
- **FRED_API_KEY**: Free key from https://fred.stlouisfed.org/apikey takes
  30 seconds. 120 req/min on free tier. Without key, returns clear warning
  and skips with 0 rows (no crash).
- **Timing**: `api.stlouisfed.org` uses a different CDN domain than the
  old CSV endpoint `fred.stlouisfed.org` — works from VPS where old endpoint
  timed out.

## Moomoo OpenAPI — implementation notes

- **Script paths**: `.agents/skills/moomooapi/scripts/quote/get_kline.py`
  and `get_snapshot.py`
- **Gateway**: OpenD TCP at `FUTU_OPEND_HOST:FUTU_OPEND_PORT` (default
  127.0.0.1:11111)
- **Rust module**: `engine/src/data/moomoo.rs` shells out via
  `tokio::process::Command` — same pattern as `exec/moomoo.rs`
- **See**: `references/moomoo-subprocess-pattern.md` for full details

## API Endpoint Data Map (post-consolidation)

| Endpoint | Primary Source | Fallback | Freshness |
|---|---|---|---|
| `GET /api/chart` | Moomoo OpenD | Yahoo | DB candles + live quote bundled |
| `GET /api/quote` | Moomoo OpenD | Yahoo | Per-request, real-time |
| `GET /api/predictions` | `equity_predictions` DB | — | Set by scheduler |
| `GET /api/status` | In-process | — | Real-time |
| `WS /ws` | In-process signal_state | — | Real-time |

## Database Schema (post-consolidation)

Tables: `equity_candles`, `equity_predictions`, `equity_trades`,
`equity_ingest_state`, `sentiment_cache`, `strategy_configs`,
`mode_switches`, `backtest_results`.

New: `sentiment_cache(symbol, date, score, source)` — ready for Phase 2
Finnhub wiring. Stub seeds neutral 0.5 at startup via
`sentiment::seed_sentiment_cache()`.

### Ingestion Cadence — Wall-Clock-Gated Dual Runs (2026-08-03)

The equities ingestion supervisor (`run_equities_ingestion` in `data/mod.rs`)
runs the `backfill_equities()` top-up task at **two UTC wall-clock targets**:

| Target | Time | Purpose |
|--------|------|---------|
| **Post-close** | 22:00 UTC (16:00 ET + 90min) | Catch today's QQQ close before it becomes stale overnight |
| **Pre-market** | 07:00 UTC (03:00 ET) | Safety net 2.5h before open; ensures Monday morning has Friday's close |

Both targets have a ±30-minute tolerance window. On cold-start (first tick
after engine boots), the task runs immediately regardless of wall-clock time.

**Why not a single midnight UTC run?** US markets close at 21:30 UTC.
A midnight UTC run that fires before Friday's bar has been published by
Moomoo/Yahoo misses the entire day. The gap then persists until the next
midnight — a full 24h cycle lost. On a Friday this means **zero new data
until Monday pre-market** (48-72h staleness).

**Scheduler vs ingestion separation:** the scheduler (`scheduler.rs:77`)
polls the DB every 5 minutes looking for a new `equity_candles.ts`. It
does NOT fetch candles — it only runs inference when the timestamp has
grown. Ingestion must have already written fresh data for the scheduler
to pick it up.

## Env vars (post-consolidation)

New vars to set for full coverage:
- `FRED_API_KEY` — enables treasury yield + DXY
- `FUTU_OPEND_HOST` — OpenD gateway host (use `host.docker.internal` in Docker)
- `FUTU_OPEND_PORT` — OpenD TCP port (default 11111)

Engine works fully without any of these — falls back to Yahoo for
equities, CBOE for VIX, stub for sentiment. FRED macro gracefully
degrades to 0.0 without key.

**CRITICAL:** Docker Compose v1 `${VAR:-default}` syntax in the
`environment:` block reads from the HOST SHELL at parse time, NOT from the
`env_file` (`.env`). If `FRED_API_KEY` is only in `.env`, the container
gets empty string. Fix: hardcode the value directly (no `${}` syntax)
or `export FRED_API_KEY=...` before `docker-compose up`. Verify inside
container: `docker exec mmn-engine sh -c 'echo $FRED_API_KEY'`.
