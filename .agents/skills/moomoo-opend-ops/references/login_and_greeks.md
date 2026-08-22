# OpenD auto-login (no password) + greeks interpretation

## Non-interactive auto-login — the `-login_by_remember` path
OpenD.xml has **NO credential fields** — you cannot put account/password there.
The working password-free mechanism is **command-line flags** on the OpenD binary:

```
OpenD -login_account=<UserID> -login_by_remember=1 -lang=en
```

- `-login_by_remember=1` replays the cached session token
  (`~/.com.moomoo.OpenD/F3CNN/Broker.dat`) — no password on cmdline or in any config.
- `-login_account` selects WHICH remembered account (UserID / email / phone).
  If phone, also pass `-area_code=+65` etc.
- `login_account` alone is "deprecated after 10.10" per the doc, but the
  `-login_by_remember=1` flag is the current, supported path.
- Doc: https://openapi.moomoo.com/moomoo-api-doc/en/opend/opend-cmd.html
  (VuePress SPA — the flags are in the `21.*.js` bundle's "Command Line Startup" table)

**Gotcha — session invalidation:** killing OpenD can invalidate the server-side
session. On next start it falls back to `"Please enter account"`
(`ProgramStatusType_Loging` → "Please enter account" in the gateway log at
`~/.com.moomoo.OpenD/Log/GTWLog_0_*.log`). `-login_by_remember` only works while the
cached session is valid. If it fails, ONE interactive login is needed to refresh the
cache — rare, not per-restart.

**Verify login (SDK):**
```python
from moomoo import OpenQuoteContext, RET_OK
c = OpenQuoteContext(host='127.0.0.1', port=11111)
r, d = c.get_global_state()
# d.get('qot_logined') / d.get('trd_logined')  -> True when logged in
c.close()
```
A successful remembered login logs: `Login Method: Log in with remembered password`
then `NotifyConnectStatusChanged ... "qotLogined":true,"trdLogined":true`.

## Greeks / option-quote probe gotchas
Script: `moomooapi/scripts/quote/probe_option_quota.py` (user-owned skill — off-limits).
Two real bugs found & fixed 2026-08-19:
1. `leg.action = 1` (raw int) → must be `StrategyLegAction.BUY`. Raw int caused
   `ERROR. the type of action in option_legs is wrong` → `get_option_quote` returns
   -1 → the script died silently with NO output.
2. The greeks check treated the literal string `'N/A'` as a valid value → false
   "5 greeks ACTIVE" verdict + `float('N/A')` crash. Must exclude `'N/A'`/`'n/a'`
   and require a numeric, non-zero-IV value.

**Reading greeks output:**
- Greeks (delta/gamma/theta/vega/rho) + `implied_volatility` return the literal
  string `'N/A'` / `0.0` when the **US options session is CLOSED** (after 16:00 ET,
  before 09:30 ET, weekends/holidays) OR the underlying US **Basic-quotes**
  entitlement is missing. They are LIVE-computed values, not static.
- Static fields (price, mark_price, intrinsic_value, prob_of_profit, breakeven) still
  populate when greeks are N/A. Use that to distinguish:
  - "feed alive but market closed" (price == last_close, open/high/low = N/A,
    volume = 0) → re-run during market hours.
  - "entitlement missing" (underlying `get_stock_quote` → "Please subscribe to Basic
    data first") → OPRA card covers option price only; greeks need underlying US
    Basic-quotes subscription.
- OPRA card ($7.49/mo, Inah active) enables option *price* data. Greeks need both the
  card AND the underlying's Basic quotes AND market hours.
