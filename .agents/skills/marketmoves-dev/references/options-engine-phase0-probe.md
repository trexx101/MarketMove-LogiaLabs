# Options Engine Phase 0 — Quota & OPRA Verification

## Probe Script

**Location:** `.agents/skills/moomooapi/scripts/quote/probe_option_quota.py`

**Purpose:** Verify account option quota tier and OPRA card status before Phase 0 sign-off.

**What it checks:**
1. Current subscription quota (total_used, remain, own_used)
2. Fetches QQQ option chain, finds a contract in 30-45 DTE window
3. Subscribes QUOTE for one option contract
4. Fetches option quote and verifies greeks fields (`implied_volatility`, `delta`, `gamma`, `theta`, `vega`, `rho`)

**Usage:**
```bash
cd /home/ubuntu/projects/MarketMoves/.agents/skills/moomooapi/scripts/quote

# Human-readable output
python3 probe_option_quota.py --text

# JSON output (default)
python3 probe_option_quota.py

# Different underlying
python3 probe_option_quota.py --underlying US.SPY --text
```

**Expected output:**
- Quota tier info (20/60/200 chains based on account assets)
- Whether OPRA card is active (greeks fields present)
- Sample greeks values (IV, delta, gamma, theta, vega, rho)
- Any errors (OpenD unreachable, no expirations in window, etc.)

## Interpretation

**Quota tiers (Futu):**
- < 10K HKD assets → 20 chains
- 10K-100K HKD → 60 chains
- 100K-500K HKD → 200 chains
- > 500K HKD → 400 chains

**OPRA card:** $7.49/mo for US options LV1 quotes. Required for greeks fields in option quotes.

**Phase 0 acceptance:**
- Quota tier recorded in config (D3 budget: ~60% recorder / 40% live trading)
- OPRA card active (greeks fields present in probe output)
- If quota < 60, recorder budget must be redesigned (fewer underlyings)

## Pitfalls

**LSP errors are expected:** The moomoo SDK lives in the uv cache (`~/.cache/uv/archive-v0/...`), not a standard venv. Pyright will report "Import could not be resolved" — this is fine, the script runs correctly.

**OpenD must be running:** Script connects to `FUTU_OPEND_HOST:FUTU_OPEND_PORT` (default 127.0.0.1:11111). If OpenD is down, you'll get "Cannot connect to OpenD" error.

**Market hours matter:** Option quotes may be stale or missing outside US market hours (9:30-16:00 ET). The probe will still work but greeks values may be from last close.

**Quota usage:** Each option chain subscription uses 1 quota unit. The probe subscribes one chain, checks greeks, then unsubscribes. If you're at quota limit, the subscribe call will fail.

**SCRIPT BUG — greeks step uses wrong action type (caught 2026-08-19):**
The script sets `leg.action = 1` (raw int) before `get_option_quote([leg])`. The moomoo SDK rejects this:
```
get_option_quote ret: -1
msg: ERROR. the type of action in option_legs is wrong
```
Consequence: the script produces zero output — the error is swallowed into `result["errors"]` but the final print never runs, so it looks like a silent connection failure. Fix the script to use the `StrategyLegAction` enum:
```python
from moomoo import OptionStrategyLeg, StrategyLegAction
leg = OptionStrategyLeg()        # no constructor args — set attrs after
leg.code = option_code
leg.action = StrategyLegAction.BUY   # NOT 1, NOT TrdSide.BUY
leg.quantity = 1
```
`OptionStrategyLeg()` takes no constructor args (an earlier bug was passing `option_type=...`); set `.code` / `.action` / `.quantity` as attributes after.

**Interpreting greeks results — OPRA card state, not a bug:**
- Columns MISSING (no `implied_volatility` / `delta` / etc.) -> OPRA card not provisioned.
- Columns PRESENT but values N/A / `implied_volatility = 0.0` (observed 2026-08-19 on a live OpenD connection with quota working) -> card IS provisioned but the US options quote entitlement is inactive. Subscription succeeds and the greeks schema arrives; no market data fills the fields. This is the expected state until US options permissions are enabled in the Futu/Moomoo app + the $7.49/mo OPRA card is active. NOT a connectivity bug, NOT a script bug.
- Columns PRESENT with real floats -> fully active.

**Reproducible greeks probe (bypasses the broken script):**
```bash
cd /home/ubuntu/projects/MarketMoves/.agents/skills/moomooapi/scripts
/home/ubuntu/projects/MarketMoves/inference/.venv/bin/python3 - <<'PY'
import sys, time
sys.path.insert(0, '.')
from common import create_quote_context, RET_OK
from moomoo import OptionStrategyLeg, StrategyLegAction
ctx = create_quote_context()
code = "US.QQQ260925C450000"
ctx.subscribe([code], ["QUOTE"]); time.sleep(2)
leg = OptionStrategyLeg(); leg.code = code; leg.action = StrategyLegAction.BUY; leg.quantity = 1
ret, q = ctx.get_option_quote([leg])
print("ret", ret)
if ret == RET_OK:
    for f in ["implied_volatility","delta","gamma","theta","vega","rho"]:
        if f in q.columns: print(f, q.iloc[0][f])
ctx.close()
PY
```

## Related

- Plan: `.hermes/plans/options-momentum-engine/PLAN.md`
- Phase 0 tasks: verify quota tier, verify OPRA card, config schema, DB migrations, OpenD client extension
- Design decisions: D3 (quota budget), D4 (OPRA quote-right), D17 (chain selection), D18 (liquidity floors)
