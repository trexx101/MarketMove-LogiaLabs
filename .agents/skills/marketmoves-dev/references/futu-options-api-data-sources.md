# Futu OpenD API: Options Data Source Gotchas

Captured 2026-08-21 while fixing the options tape recorder.

## Three APIs, three different data shapes

| API | Returns | Bid/Ask | Greeks | OI | Use for |
|---|---|---|---|---|---|
| `get_option_chain` | Contract metadata only | ❌ | ❌ | ❌ | Finding available contracts |
| `get_option_quote` | Limited quote data | ❌ (mid_price only) | "N/A" often | ❌ | Do NOT use |
| `get_market_snapshot` | Full market data | ✅ bid_price/ask_price | ✅ option_delta etc. | ✅ option_open_interest | Chain discovery + polling |

### `get_option_chain` — metadata only

Columns: `code, name, lot_size, stock_type, option_type, stock_owner, strike_time,
strike_price, suspension, stock_id, index_option_type, expiration_cycle,
option_standard_type, option_settlement_mode`

**No pricing data.** No bid, ask, last, volume, OI, IV, delta. If you try to
read `row.get("bid_price", 0)` from a chain row, you get the default (0).
Every contract fails liquidity filters.

Correct usage: get the list of option codes, then batch-snapshot them.

### `get_option_quote` — wrong data, missing fields

Returns `price` (not `last_price`), `mid_price`, `intrinsic_value`, `time_value`,
`breakeven_point`. Greeks and volume are **"N/A"** for US options even with
OPRA subscription active. The `price` field can be nonsensical (e.g. 441 for
a 500-strike QQQ call when QQQ is at 528).

**Do not use for options data.** Use `get_market_snapshot` instead.

### `get_market_snapshot` — the correct API

Accepts up to 400 codes per call. Returns a DataFrame with these option columns:

| Column | Type | Notes |
|---|---|---|
| `code` | str | Contract code for lookup |
| `bid_price` | float | Actual bid |
| `ask_price` | float | Actual ask |
| `last_price` | float | Last trade price |
| `volume` | float | Day volume |
| `option_type` | str | "CALL" or "PUT" |
| `option_strike_price` | float | Strike price |
| `option_open_interest` | float | Open interest |
| `option_implied_volatility` | float | IV (may be 0 for deep ITM/OTM) |
| `option_delta` | float | Delta (0.0 for deep ITM calls) |
| `option_gamma` | float | Gamma |
| `option_vega` | float | Vega |
| `option_theta` | float | Theta |
| `option_rho` | float | Rho |
| `option_valid` | str | "TRUE" or "FALSE" |

Filter by `option_valid == "TRUE"` before processing.

## Batch discovery pattern

```python
# 1. Get chain metadata
ret, chain_df = ctx.get_option_chain(underlying, start=expiry, end=expiry)
codes = [str(row["code"]) for _, row in chain_df.iterrows()
         if str(row.get("option_type", "")).upper() in ("CALL", "PUT")]

# 2. Batch snapshot all codes + underlying (for underlying_price)
ret, snap_df = ctx.get_market_snapshot(codes + [underlying])

# 3. Build lookup and filter
for _, row in snap_df.iterrows():
    if str(row.get("option_valid", "")).upper() != "TRUE":
        continue
    bid = float(row.get("bid_price", 0))
    ask = float(row.get("ask_price", 0))
    delta = float(row.get("option_delta", 0))
    oi = int(float(row.get("option_open_interest", 0)))
    # ... liquidity filters ...
```

## Pandas truth-value gotcha

```python
# WRONG — raises "The truth value of a DataFrame is ambiguous":
if not exp_data["strike_time"]:

# CORRECT:
if exp_data["strike_time"].empty:
```

This applies to any Series derived from a DataFrame column. Use `.empty`,
`len()`, or `.bool()` — never `not`/`if` on a Series.

## Quota

`get_market_snapshot` counts against the Futu quote quota. At 60% recorder
allocation on tier 20 (12 slots), a single snapshot call for 6 contracts +
3 underlyings = 9 codes per 15s poll is well within the 30 req/30s limit.
Chain discovery (once per day) may snapshot 300+ codes across 3 underlyings
— batch into groups of 400 to minimize API calls.