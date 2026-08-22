# Options Engine P1 Implementation Patterns

## Python-Rust Integration for OpenD

**Pattern:** Python script polls OpenD, streams JSON lines to stdout → Rust binary reads JSON, processes, writes parquet.

**Why:** OpenD has no Rust SDK; Python `moomoo` package is the only maintained client. Streaming via stdout avoids IPC complexity.

### Python Script: `record_option_quotes.py`

Location: `.agents/skills/moomooapi/scripts/quote/record_option_quotes.py`

> **⚠ BUG (found 2026-08-20, unfixed):** The script calls `ctx.get_stock_quote(contracts)`
> which is a **stock** quote API — it returns nothing or garbage for option contracts.
> Must be rewritten to use `ctx.get_option_quote([OptionStrategyLeg])`.
> See "Known Bugs" section below for all three recorder bugs and the fix design.

```python
# CURRENT (broken): polls get_stock_quote() — wrong API for options
# OUTPUT (claimed): JSON lines to stdout with flush=True
{
  "timestamp_ms": 1787110843519,
  "contract": "US.QQQ260901C660000",
  "bid": 0.0, "ask": 0.0, "last": 0.0,
  "volume": 0, "oi": 0,
  "iv": 28.994, "delta": 0.937, "gamma": 0.003, "theta": -0.241,
  "underlying_price": 0.0
}
```

**Test (currently broken):** `python3 record_option_quotes.py --contracts US.QQQ260901C660000 --interval 2 --duration 5`

### Rust Binary: `options_recorder.rs`

Location: `engine/src/bin/options_recorder.rs`

- Spawns Python via `tokio::process::Command`
- Reads stdout line-by-line with `BufReader`
- Deserializes JSON → `TapeRow` → writes parquet

**Contract code format:** `US.QQQ260901C660000` (underlying + expiry + type + strike, no leading zeros on strike). Parse with `splitn(2, |c| c == 'C' || c == 'P')` to extract underlying (`US.QQQ`) and chain (`US.QQQ260901`).

**Dependencies:** `arrow = "53"`, `parquet = { version = "53", features = ["snap", "arrow"] }` (both workspace + engine).

## Quota Accounting

Futu option quota is tiered by account assets: 20/60/200/400 chains. Each subscription (QUOTE or K-line) costs 1 quota per chain.

**Design:** 60% recorder / 40% live split (configurable via `OPT_RECORDER_QUOTA_PCT`). Recorder uses separate OpenD connection, subscribes QUOTE only (no K-line), synthesizes candles from ticks.

**Implementation:** `QuotaAccount` struct in `engine/src/options_recorder.rs`:
- `new(tier)` — initializes with `max_recorder = tier * 0.6`
- `try_subscribe(contract_code)` — returns `false` if quota exhausted
- `unsubscribe(contract_code)` — releases quota
- `used()` / `remaining()` — monitoring
- `shed_oldest()` — returns oldest subscription to free quota under pressure

**Testing:** 4 unit tests in `engine/src/options_recorder/tests.rs` verify quota enforcement, shedding, and parquet schema.

**Verified tier:** Account quota tier = 20 (not 60 as assumed in design doc D3). Config already correct.

## Parquet Schema

Tape partitioned by `underlying/chain/date.parquet`. Schema (14 columns):

```
timestamp (Timestamp[ms]), underlying (Utf8), chain_code (Utf8), contract_code (Utf8),
bid (Float64), ask (Float64), last (Float64), volume (Int64), open_interest (Int64),
implied_volatility (Float64), delta (Float64), gamma (Float64), theta (Float64),
underlying_price (Float64)
```

**Compression:** SNAPPY (via `parquet` crate with `arrow` feature enabled).

**Implementation:** `build_tape_schema()` and `build_tape_arrays()` in `engine/src/options_recorder.rs`. `TapeWriter` struct in binary manages per-chain writers with `Option<ArrowWriter>` to handle `close()` consuming `self`.

## Known Bugs — Tape Recorder (found 2026-08-20, unfixed)

The recorder binary and Python script were never started in production. Three
bugs prevent them from working even if launched:

### Bug 1: Wrong API in `record_option_quotes.py`
The Python script calls `ctx.get_stock_quote(contracts)` (line 32) — this is a
**stock** quote API. For option contracts it returns nothing or garbage.
**Fix:** Must use `ctx.get_option_quote([OptionStrategyLeg])` which returns
full option quotes with greeks (IV, delta, gamma, theta, vega, rho). See
`get_option_quote.py` in the moomooapi skill for the correct pattern:
```python
from moomoo import OptionStrategyLeg, StrategyLegAction
leg = OptionStrategyLeg()
leg.code = option_code
leg.action = StrategyLegAction.BUY
leg.quantity = 1
ret, data = ctx.get_option_quote([leg])
# data columns: price, volume, open_interest, implied_volatility,
#   delta, gamma, vega, theta, rho, strike_price, expire_time
```
**Note:** The `record_option_quotes.py` script's JSON schema field names
(`bid`, `ask`, `oi`, `iv`) do NOT match `get_option_quote` output columns
(`price`, `open_interest`, `implied_volatility`). The QuoteJson struct in the
Rust binary and the Python script must be updated together.

### Bug 2: No heartbeat call in `options_recorder.rs`
The Rust binary never calls `db::touch_tape_heartbeat()`. The heartbeat
table (`option_tape_meta`), API endpoint (`GET /api/options/tape/status`),
and UI panel (`OptionsMonitor.svelte`) are all built and functional, but the
recorder never writes a heartbeat. The API returns
`{"tapes":[],"count":0,"healthy":0}`.
**Fix:** After each poll batch, open a SQLite connection to the same
`data/candles.db` and call `db::touch_tape_heartbeat()` with tape_id,
underlying, chain_code, and quota JSON. One heartbeat row per underlying.

### Bug 3: Hardcoded single contract
`options_recorder.rs` line 130: `vec!["US.QQQ260919C00530000"]`. No dynamic
chain discovery, no multi-underlying support, no DTE/delta filtering.
**Fix:** Config-driven underlyings list (`OPT_UNDERLYINGS` env var),
chain discovery via `get_option_expiration_date` → `get_option_chain`,
filter to 30-45 DTE, delta ~0.45, bid/spread/OI liquidity gates.

See `references/tape-recorder-multi-equity-design.md` for the full multi-equity
recorder design that addresses all three bugs.

## Resuming from Lost Sessions

When the user says "there's a lost session that started working on X, review what is done and resume":

1. **Scan git log** for recent commits on the feature branch: `git log --all --oneline --since="2 days ago" | grep -i "<feature>"`
2. **Check what was done:** `git show --stat <commit-sha>` to see files touched and commit message
3. **Verify blockers are resolved:** If the commit message mentions a blocker (e.g., "US options permissions not yet enabled"), test whether it's still blocking:
   - For API permissions: try the operation directly (e.g., subscribe to option quotes)
   - For config issues: check if the config is now correct
4. **Resume from the next phase:** If P0 is done, move to P1. If P1 is partially done, check what's missing.

**Key:** Don't assume the blocker from the lost session is still blocking. Verify empirically before declaring work blocked.

## Commits in This Session

- `890cfa2` — P0a (config schema) + P0d (DB migrations) from lost session
- `5ce8103` — P1 core logic (parquet schema, quota accounting, tape builder)
- `69b2fa8` — P1 binary scaffold (TapeWriter, config integration)
- `18a52a4` — P1 OpenD integration (Python poller + Rust parquet writer)
