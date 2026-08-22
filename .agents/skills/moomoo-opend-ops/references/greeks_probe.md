# Options greeks probe — bugs fixed & how to read output

Script: `.agents/skills/moomooapi/scripts/quote/probe_option_quota.py`
Run: `inference/.venv/bin/python3 quote/probe_option_quota.py --text`

## Two real bugs fixed (2026-08-19)
1. **Wrong action type (silent crash).** Original code:
   `leg.action = 1  # BUY` → moomoo rejects raw int:
   `ERROR. the type of action in option_legs is wrong`, `get_option_quote` returns -1,
   script died with NO output. Fix: `from moomoo import StrategyLegAction` and
   `leg.action = StrategyLegAction.BUY`.
2. **`'N/A'` treated as a valid greek.** The value check used
   `str(val) not in ["nan","NaN","None",""]` — but moomoo returns the literal string
   `'N/A'` for missing greeks, which passed. Also `float('N/A')` crashed the sample
   block. Fix: exclude `'N/A'`/`'n/a'`, wrap `float()` in try/except, and require a
   numeric (non-zero-IV) value before counting a greek as "available".

## How to interpret the verdict
- `✓ ACTIVE — N greeks fields available` → greeks populate (only possible during US
  market hours with a valid entitlement).
- `✗ INACTIVE or INSUFFICIENT — greeks not available` + all 6 missing → read the
  `greeks_note`: if static fields (price/mark_price) populate but greeks are `'N/A'`,
  it's almost always **market closed** (after 16:00 ET) or missing underlying US
  Basic-quotes entitlement — NOT a wrong API call.

## Greeks availability rules
- Greeks (delta/gamma/theta/vega/rho) + `implied_volatility` are LIVE-computed, pushed
  only during US option trading hours (09:30–16:00 ET, Mon–Fri).
- Outside hours: `price == last_close_price`, `open/high/low = N/A`, `volume = 0`,
  `implied_volatility = 0.0`, greeks = `'N/A'`. The data feed is alive; the session
  is closed.
- `get_option_quote` with a single `OptionStrategyLeg(StrategyLegAction.BUY)` is the
  correct call — greeks are NOT in `get_option_chain` (chain snapshot has no greek
  columns) and NOT unlocked by subscribing ORDER_BOOK/TICKER/BROKER.

## Re-run to get a clean PASS
Run the probe during US market hours. If greeks are STILL `'N/A'` during hours, that
points to a missing US Basic-quotes entitlement for the underlying (separate from the
$7.49/mo OPRA option-price card).
