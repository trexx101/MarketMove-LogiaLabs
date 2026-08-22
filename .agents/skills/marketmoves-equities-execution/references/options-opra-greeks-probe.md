# Options Momentum Engine — OPRA / Greeks Probe Workflow

Operational knowledge for verifying the Moomoo (Futu) options data feed that the
Options Momentum Engine depends on (option chain, real-time quotes, greeks).
Captured 2026-08-19 during a live OpenD probe session.

## Where the scripts live

- Skill `moomooapi` (installed by URL to the workspace, **user-owned → protected**):
  `/home/ubuntu/projects/MarketMoves/.agents/skills/moomooapi/scripts/quote/probe_option_quota.py`
- Run with the project venv:
  `inference/.venv/bin/python3 quote/probe_option_quota.py --text`
  (CWD must be `.../moomooapi/scripts` so the `common` import resolves.)

## OpenD must be running

The probe connects to OpenD at `127.0.0.1:11111` (local daemon, not REST).
Check: `pgrep -af OpenD` and `ss -ltnp | grep 11111`. If OpenD isn't up,
`query_subscription` returns `PacketErr.Timeout` — that is a connection failure,
not an entitlement problem.

## Two bugs in `moomooapi`'s `probe_option_quota.py` (PROTECTED SKILL — flag, do not patch here)

These were found and fixed in the workspace copy this session. The `moomooapi`
skill is user-owned (URL-installed), so the durable fix belongs to the user —
recommend `hermes curator adopt moomooapi` so the corrected script ships.

1. **`leg.action = 1` (raw int) → `StrategyLegAction.BUY`.**
   `OptionStrategyLeg.action` must be the `moomoo.StrategyLegAction` enum, NOT
   a raw integer. With `1`, `get_option_quote([leg])` returns
   `ret=-1, "ERROR. the type of action in option_legs is wrong"`, which the
   original script swallowed into `errors` and then died **with zero output** —
   that is why the probe printed nothing. Fix:
   `from moomoo import OptionStrategyLeg, StrategyLegAction; leg.action = StrategyLegAction.BUY`.
2. **Greeks `'N/A'` string treated as a valid value.**
   The original value check was `str(val) not in ["nan","NaN","None",""]` — but
   moomoo returns the literal string `'N/A'` for missing live greeks, which
   passed the check. That produced a false "5 greeks ACTIVE" verdict AND a
   `float('N/A')` crash in the sample block. Correct check: treat
   `N/A`/`n/a` as missing, and `try: float(val)` before counting a field as present.

## Greeks are LIVE-HOURS-ONLY — the #1 misread

`implied_volatility`, `delta`, `gamma`, `theta`, `vega`, `rho` are **computed
from the live underlying IV surface and pushed only during US options trading
hours (09:30–16:00 ET, Tue–Fri).** Outside those hours they come back as
`'N/A'` / `0.0`.

This is NOT an entitlement failure when the rest of the option quote populates.
When the session is closed you will see:
- `price` == `last_close_price` (stale, last close)
- `open_price` / `high_price` / `low_price` = `N/A`, `volume = 0`
- greeks = `N/A`, `implied_volatility = 0.0`

**Tell closed-session apart from a real entitlement gap:** if `price`,
`mark_price`, `intrinsic_value`, `prob_of_profit`, `breakeven_point` populate
but greeks don't, the feed is alive — it's just after hours. Re-run during
market hours to confirm. If greeks are STILL `N/A` during market hours, the
remaining gap is the **underlying US Basic/Level-1 quotes entitlement** (the
account-level US market data package enabled in the Futu app), which greeks
derivation depends on even though the option's own price prints under the OPRA card.

## Reading a probe run

- `quota.option_remain_quota` (option subscriptions left) maps to the Futu
  option-quote tier: **19 remaining → tier 20** (not 60, as an earlier plan
  assumed). Confirm the actual tier from this number, don't hardcode it.
- `✓ ACTIVE` verdict is only meaningful if the listed greeks fields carry
  numeric values, not just column names. A correct probe checks value
  numeric-ness, not schema presence.
- `sub_list` in `query_subscription` shows currently-held QUOTE subs — useful
  to spot leftover subscriptions from earlier runs (e.g. `US.QQQ260925C450000`).

## Subscription types that do NOT unlock greeks

`SubType` in this SDK version has no `BASIC` member (only `QUOTE`, `RT_DATA`,
`ORDER_BOOK`, `TICKER`, `BROKER`, KLINE types). Subscribing an option to
`QUOTE`/`ORDER_BOOK`/`TICKER`/`BROKER` does not change greeks availability —
greeks come through `get_option_quote([leg])` and are gated by session hours +
the underlying's Basic entitlement, not by an option SubType.

## Correct minimal greeks fetch (verified working)

```python
from moomoo import OptionStrategyLeg, StrategyLegAction
leg = OptionStrategyLeg()
leg.code = "US.QQQ260925C450000"
leg.action = StrategyLegAction.BUY   # enum, NOT 1
leg.quantity = 1
ret, q = ctx.get_option_quote([leg])
# check value numeric-ness, not just `field in q.columns`
```

## Other gotcha: missing version stamp

`probe_option_quota.py` warns `Version stamp file not found:
/home/ubuntu/.moomoo_skill_version. Consider running /install-moomoo-opend to
install`. That stamp is set by the `install-moomoo-opend` skill's install step;
if it's missing, run that skill to finalize the OpenD install. It does not block
the probe.
