# Futu OpenD Options API — Reference

## Quota system

Two separate quota pools:
- **Equity quotes**: `total_used`, `remain` (e.g., 100 for standard tier)
- **Option quotes**: `option_used_quota`, `option_remain_quota` (20/60/200/400 by assets)

Query via: `ctx.query_subscription(is_all_conn=True)`

Returns dict with both pools. Option quota is the binding constraint for the options recorder.

## Key API methods for options engine

### get_option_expiration_date(code)
- Returns available expiration dates for an underlying
- **Known issue**: Can hang/timeout on some OpenD configs
- Workaround: use `get_option_chain(code, start=..., end=...)` with computed date range instead

### get_option_chain(code, start, end, option_type, option_cond_type, data_filter)
- `start`/`end`: date strings 'YYYY-MM-DD', max 30-day range
- `option_type`: `OptionType.ALL` / `CALL` / `PUT`
- Returns DataFrame with columns: `code`, `option_type`, `strike_time`, `expiration_date`, etc.
- **Requires**: US options quote permissions enabled (not just OPRA card)

### get_market_snapshot(codes) ← USE THIS FOR OPTION PRICING DATA

**This is the correct API for real-time option book and Greeks.** Use it instead of `get_option_quote`.

- Takes a list of codes (up to 400 per call, but US options limited to 20 per request
  under HK options/futures BMP permission).
- **Batch all contracts + underlyings in ONE call** (e.g. `[codes..., underlyings...]`)
  for maximum rate-limit efficiency.
- Returns DataFrame with all fields needed for a tape recorder:

| Field (option-specific) | What |
|---|---|
| `bid_price` | Bid |
| `ask_price` | Ask |
| `last_price` | Last trade price |
| `volume` | Volume |
| `option_open_interest` | Open interest |
| `option_implied_volatility` | IV |
| `option_delta`, `option_gamma`, `option_theta`, `option_vega`, `option_rho` | Greeks |
| `option_strike_price` | Strike price |
| `option_type` | "CALL" or "PUT" |
| `option_valid` | Must be `True`/`"TRUE"` for option fields to be meaningful |
| `code` | Contract code |
| `name` | Display name |

**Pitfall — option_valid gate:** Always check `option_valid` before reading
option-specific fields. If false, those fields are populated with stale equity
data, not option data. Batch the underlying's own code in the same snapshot call
to get `last_price` for that tick.

**Performance:** one `get_market_snapshot` call for all codes replaces N
individual `get_option_quote` calls, both faster and within rate limits.

### get_option_quote(option_legs) — DO NOT USE FOR DISCOVERY/POLLING

- Takes list of `OptionStrategyLeg(...)` objects.
- **Returns severely degraded data**: column names are flat (no `option_` prefix),
  greeks are "N/A" outside market hours, no `bid_price`/`ask_price` columns
  (only `price` for last-trade). The field set is insufficient for liquidity
  filtering or tape recording.
- **Only use**: for combo-leg strategy analysis, never for tape recording or
  chain discovery. For those, use `get_market_snapshot`.

## Pandas traps

### `not df["column"]` raises truth-value error

Moomoo's Python SDK returns DataFrames. Doing `if not df["col"]` or
`if not exp_data["strike_time"]` on a DataFrame column raises:
`ValueError: The truth value of a DataFrame is ambiguous.`

Always use `.empty`: `if df["col"].empty` or `df.empty` instead of truthy checks.

### subscribe(codes, sub_types) / unsubscribe(codes, sub_types)
- `sub_types` includes `"QUOTE"` for real-time option quotes
- Each option subscription consumes from `option_remain_quota`
- Must unsubscribe when done to free quota

## Permission model

1. **OPRA quote card** ($7.49/mo): Enables LV1 options data (greeks, OI)
2. **US options permissions**: Account-level toggle in Futu/Moomoo app
3. Both must be active for `get_option_chain()` to succeed

Error when permissions missing: "No permission to get quotes for US.QQQ. Please check US MarketOptions quote permissions."

## Python SDK location

- Package: `moomoo-api` (installed via `uv add moomoo-api` in inference venv)
- Key classes: `OpenQuoteContext`, `OptionStrategyLeg`, `OptionType`, `OptionCondType`
- Context creation: `from common import create_quote_context` (handles env vars, connection)
