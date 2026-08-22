---
name: moomoo-opend-ops
description: Keep Futu OpenD alive on a headless VPS; fix login failures.
metadata:
  version: 0.1.0
  author: hermes-curator
---

# moomoo OpenD Operations (headless VPS)

You manage Futu/moomoo **OpenD** — the local gateway the `moomoo` Python SDK talks to
(`OpenQuoteContext(host='127.0.0.1', port=11111)`). On a headless VPS the default
"launch it in a terminal" approach is broken: the process is a child of your shell and
dies the moment the SSH/VS Code session closes, and OpenD is a GUI app that needs an
interactive login.

This skill covers the **ops/deployment** layer only. The Futu-provided
`install-moomoo-opend` and `moomooapi` skills (user-installed, off-limits to edit) cover
download/install and the SDK scripting. If you find those skills wrong or outdated,
recommend `hermes curator adopt <name>` — do not patch them yourself.

## When to use
- "OpenD dies when I close my connection" → wrap it in a systemd user service.
- SDK: `init connect fail ... Abnormal event timeout` or `get_global_state` returns
  `qot_logined: False` → diagnose login state (below).
- First-time bring-up on a fresh VPS.

## Deploy as a systemd --user service (the fix for "dies on disconnect")
Do NOT run OpenD with `&` in an SSH session — that still dies. Use a user service.

1. Confirm systemd is available and linger is on (linger keeps user services alive after
   logout):
   ```bash
   loginctl show-user "$USER" | grep -i linger   # want Linger=yes
   loginctl enable-linger "$USER"                # if not already
   ```
2. Write the unit. `WorkingDirectory` MUST be the OpenD install folder (it reads
   `OpenD.xml` and writes the login cache relative to cwd). `Type=simple`,
   `Restart=on-failure`.
   - Template: `references/systemd-opend.service` (copy, fix the two paths, adjust
     `WorkingDirectory`).
3. Enable + start:
   ```bash
   systemctl --user daemon-reload
   systemctl --user enable opend.service
   systemctl --user start  opend.service
   systemctl --user status opend.service --no-pager
   ```
4. Verify it is detached from any shell: `ps -o pid,ppid,cmd -p <opend_pid>` — PPID
   should be the `systemd --user` manager (a `systemd` process), NOT a `bash`/SSH shell.

## Pitfalls (these bit us; encode them)
- **Killing OpenD invalidates the Futu session.** Futu server-side sessions do not
  survive a process kill. If you `kill` OpenD (e.g. to free port 11111 before starting
  the service), the new instance goes to `ProgramStatusType_Loging` → **"Please enter
  account"** and will NOT auto-login from the cached `Broker.dat`. There is no
  programmatic login — the `moomoo` SDK forbids embedding credentials / `unlock_trade`
  workarounds. A fresh login requires the **GUI**: SSH X11 forward (`ssh -X`) or a VNC
  server, launch `OpenD`, log in once with "remember me", then the service auto-logins
  on subsequent restarts *while the token stays valid*. So: plan logins; avoid killing
  OpenD casually.
- **Two OpenD copies = confusion.** Users often end up with stray installs
  (`/home/ubuntu/opend`, `/opt/moomoo-opend/...`). Only one should bind 11111. Before
  starting the service, kill ALL other OpenD processes and confirm the port is free
  (`ss -ltnp | grep 11111`).
- **`pkill -f OpenD` can take out your own command shell** (OpenD's process group can
  include the launching shell). Prefer `kill -TERM <pid>` by explicit PID, then verify
  with `ss` that 11111 is free before starting the service.
- **OpenD writes its own logs**, not to stdout: `~/.com.moomoo.OpenD/Log/GTWLog_0_*.log`.
  A `StandardOutput=append:...` redirect in the unit may capture nothing (systemd
  version-dependent) — read OpenD's own log for boot/login state. `journalctl --user -u
  opend.service` can show "No entries" even when the service is active.

## Diagnose "SDK can't connect / not logged in"
1. Port bound? `ss -ltnp | grep 11111`. If free → OpenD isn't running.
2. Login state via SDK:
   ```python
   from moomoo import OpenQuoteContext, RET_OK
   c = OpenQuoteContext(host='127.0.0.1', port=11111)
   r, d = c.get_global_state()
   # d.get('qot_logined'), d.get('trd_logined')  -> True when logged in
   ```
   `init connect fail ... timeout` means the port is bound but OpenD isn't answering
   `InitConnect` — almost always because it's blocked at the login prompt.
3. Confirm via OpenD's log: grep for `Please enter account` (needs login) vs
   `ProgramStatusType_LoginSucceed` / quote login success.

## Verify the feed is alive (not just "connected")
A bound port + `qot_logined:True` is necessary but not sufficient. Confirm real data
flows: `get_option_quote` / `get_stock_quote` should return populated fields. Known
gotcha: option greeks (delta/gamma/theta/vega/rho) and `implied_volatility` are
LIVE-computed and only pushed during US option trading hours (09:30–16:00 ET,
Tue–Fri). Outside hours the contract's static fields (price, mark_price,
intrinsic_value) still populate but greeks come back as the literal string `'N/A'` /
IV `0.0`. Distinguish "entitlement missing" from "market closed" by checking whether
static fields populate while greeks are N/A.

## Trading calendar API

Use `request_trading_days` (NOT `get_trading_days` — that method hangs) to fetch
the US market calendar and check holidays/weekends:

```python
from moomoo import TradeDateMarket, RET_OK
ret, data = ctx.request_trading_days(market=TradeDateMarket.US, start='2026-08-01', end='2026-09-30')
# Returns a LIST of dicts, NOT a DataFrame:
#   [{'time': '2026-08-03', 'trade_date_type': 'WHOLE'}, ...]
```

**Pitfall — `request_trading_days` returns a `list`, not a `DataFrame`.**
Code that calls `.iloc` or `.empty` on the return value will `AttributeError`.
Check `isinstance(data, list)` before DataFrame methods. The reference script
`get_trading_days.py` in the moomooapi skill uses the correct type handling.
