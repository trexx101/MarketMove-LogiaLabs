# OpenD systemd user service — full setup

Validated 2026-08-19 on this VPS (OpenD 10.10.7008, Ubuntu, systemd with Linger=yes).

## Unit file
Path: `~/.config/systemd/user/opend.service`

```
[Unit]
Description=moomoo OpenD (Futu OpenAPI gateway)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/home/ubuntu/projects/MarketMoves/moomoo_OpenD_10.10.7008_Ubuntu18.04/moomoo_OpenD_10.10.7008_Ubuntu18.04
# -login_by_remember=1 replays cached session token (Broker.dat) — NO password.
ExecStart=/home/ubuntu/projects/MarketMoves/moomoo_OpenD_10.10.7008_Ubuntu18.04/moomoo_OpenD_10.10.7008_Ubuntu18.04/OpenD -login_account=104483875 -login_by_remember=1 -lang=en
Restart=on-failure
RestartSec=5
TimeoutStopSec=30

[Install]
WantedBy=default.target
```

Notes:
- `WorkingDirectory` MUST be the OpenD folder (reads `OpenD.xml`, writes session cache relative to cwd).
- Do NOT put credentials in `OpenD.xml` — that file has no login fields. Login is via the cmdline flags above only.
- `Linger=yes` (confirmed via `loginctl show-user ubuntu`) lets the user service keep running after the session disconnects.

## Deploy commands
```bash
mkdir -p ~/.config/systemd/user
# write the unit file (use write_file / editor, not echo)
systemctl --user daemon-reload
systemctl --user enable opend.service
systemctl --user start opend.service
```

## Verify
```bash
systemctl --user is-active opend.service          # expect: active
ss -ltnp | grep 11111                             # OpenD should hold 127.0.0.1:11111
# login state via SDK:
python3 - <<'PY'
from moomoo import OpenQuoteContext, RET_OK
c=OpenQuoteContext(host='127.0.0.1',port=11111); r,d=c.get_global_state()
print(r, d.get('qot_logined'), d.get('trd_logined')) if r==RET_OK else print('ERR',d)
c.close()
PY
```
Expect `qot_logined: True, trd_logined: True` and gateway log line
`Login Method: Log in with remembered password`.

## Login-state diagnosis from gateway log
Log: `~/.com.moomoo.OpenD/Log/GTWLog_0_*.log` (newest by mtime).
- `ProgramStatusType_Loging` + `Please enter account` → remembered session invalid; needs one interactive login.
- `Login Method: Log in with remembered password` + `qotLogined:true` → good.

## Restart safely (without breaking the cached session)
`systemctl --user restart opend.service` is fine; the cached session usually survives a clean restart. A hard `kill -9` is what tends to invalidate it.
