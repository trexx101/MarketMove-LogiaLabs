# Moomoo OpenD Headless Install on VPS

The Moomoo OpenD TCP gateway runs as a CLI daemon on the VPS. The engine
container talks to it via `FUTU_OPEND_HOST=host.docker.internal:11111`
(see `references/moomoo-subprocess-pattern.md` for the Docker networking).

## Two variants — pick the right tarball

| Variant | Filename pattern | Needs display? | VPS-ready? |
|---|---|---|---|
| GUI | `moomoo_OpenD-GUI_<ver>_Ubuntu18.04.AppImage` | yes (X server) | no |
| Server (headless) | `moomoo_OpenD_<ver>_Ubuntu18.04.tar.gz` | no | **yes** |

The GUI variant extracts as a single 200+ MB AppImage and **cannot run on a
headless VPS** — `AppImage --help` errors with
`Something went wrong trying to read the squashfs image. Cannot mount
AppImage, please check your FUSE setup.` even though the binary runs.

The server variant extracts as a directory with `OpenD` (ELF), `OpenD.xml`,
shared libs (`libcrypto.so.3`, `libcurl.so.4`, `libssl.so.3`, `libprotobuf.so.32`,
`libf3c*.so`, `WebSocket`, `Update`, `AppData.dat`, `Languages/`).

**Always grab the non-GUI tarball from**
`https://www.moomoo.com/download/OpenAPI`.

## Install steps (verified 2026-08-01 on Ubuntu 22.04)

```bash
# 1. Extract somewhere writable — the tarball may include a versioned subdir
cd /home/ubuntu/projects/MarketMoves
tar -xzf moomoo_OpenD_10.9.6918_Ubuntu18.04.tar.gz -C /opt/

# 2. Copy to /opt/* with predictable path, fix ownership, make executable
sudo mkdir -p /opt/moomoo-opend
sudo cp -r moomoo_OpenD_10.9.6918_Ubuntu18.04/* /opt/moomoo-opend/
sudo chown -R ubuntu:ubuntu /opt/moomoo-opend
sudo chmod +x /opt/moomoo-opend/OpenD

# 3. Edit /opt/moomoo-opend/OpenD.xml — replace placeholder creds:
#    <login_account>100000</login_account>       ← real moomoo ID/phone/email
#    <login_pwd>123456</login_pwd>                ← real password (or use login_pwd_md5)
#    <ip>127.0.0.1</ip>                          ← keep as 127.0.0.1 unless LAN access needed
#    <api_port>11111</api_port>                  ← standard port
#    chmod 600 /opt/moomoo-opend/OpenD.xml       ← contains plaintext password

# 4. Verify binary runs by starting it (Hermes terminal: use background=true)
#    DO NOT use shell-level `nohup ... &` — Hermes rejects that pattern.
#    Use: terminal(background=true, command="cd /opt/moomoo-opend && ./OpenD -cfg_file /opt/moomoo-opend/OpenD.xml")
```

## Startup behavior — what to expect in logs

```text
moomoo OpenD version info: 10.9.6918(20260721161500)
>>>
Start Time: 2026-08-02 02:23:35
>>>
moomoo OpenD is running
>>>
Configuration file loaded successfully
>>>
Server started
>>>
API Listening Address: 127.0.0.1:11111
>>>
API RSA Enabled: No
>>>
Login Method: Log in with Account and Password
>>>
Logging in
```

If credentials are wrong:
```text
Login failed,Password does not match
Login failed
moomoo OpenD has exited       ← exit_code = 14
```

## CLI flags (from `./OpenD --help`)

```
login_account            Login account (user ID / phone / email)
login_pwd                Plaintext password
login_pwd_md5            32-bit MD5 hex ciphertext (preferred over plaintext)
login_by_remember        Remember password (needs account, ignored if password given)
remember                 Toggle remember-password (-remember=0 to cancel)
login_region             Connection priority: hk / gz / sh / us / sg
cfg_file                 Absolute path to FutuOpenD config XML
```

`cfg_file` is the only required flag for unattended startup. Everything
else comes from the XML.

## Engine → OpenD networking

```yaml
# docker-compose.yml — mmn-engine service
services:
  mmn-engine:
    environment:
      FUTU_OPEND_HOST: host.docker.internal   # resolves to VPS host gateway
      FUTU_OPEND_PORT: "11111"
    extra_hosts:
      - "host.docker.internal:host-gateway"  # Linux only; macOS/Windows has it native
```

Verify reachability from inside the engine container:
```bash
docker exec mmn-engine sh -c 'echo > /dev/tcp/host.docker.internal/11111 && echo OK || echo FAIL'
```

The Rust-side `data::moomoo::is_available()` does the same `TcpStream::connect`
check before shelling out to Python — see `references/moomoo-subprocess-pattern.md`.

## Daemonizing OpenD (production)

For long-running use, run OpenD under `systemd` or `tmux`/`screen`, not via
`terminal(background=true)` (which is for one-shot dev verification):

```ini
# /etc/systemd/system/moomoo-opend.service
[Unit]
Description=moomoo OpenD TCP gateway
After=network.target

[Service]
Type=simple
User=ubuntu
ExecStart=/opt/moomoo-opend/OpenD -cfg_file /opt/moomoo-opend/OpenD.xml
Restart=on-failure
RestartSec=10

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now moomoo-opend
sudo systemctl status moomoo-opend
sudo journalctl -u moomoo-opend -f   # live logs
```

## Common pitfalls

- **GUI variant instead of server variant** — re-download from
  `https://www.moomoo.com/download/OpenAPI` and pick the non-GUI build.
  GUI tarballs start with `moomoo_OpenD-GUI_`.
- **gzip EOF errors during extraction** — tarball is truncated. Re-download.
- **`${VAR:-default}` in docker-compose doesn't read `.env`** — already
  documented in main SKILL.md; affects `FUTU_OPEND_*` too. Hardcode or
  `export` before `docker-compose up`.
- **Trading unlock must be manual** — per the moomoo install skill, the
  SDK's `unlock_trade` is refused for security. Click "Unlock Trade" in the
  OpenD GUI and enter the trading password. The headless daemon does not
  unlock trading by itself — it remains locked until you intervene.
- **OpenD binding to 127.0.0.1 only** — fine for `host.docker.internal`.
  If another machine needs to reach OpenD, change `<ip>0.0.0.0</ip>` in XML
  AND add firewall rule for port 11111.

## Verification recipe

After OpenD is running and the engine is restarted:

```bash
# 1. Engine reaches OpenD
docker exec mmn-engine sh -c 'echo > /dev/tcp/host.docker.internal/11111 && echo OK'

# 2. Live quote works (Moomoo-first routing)
docker exec mmn-engine curl -sf http://127.0.0.1:8080/api/quote
# Expect: {"symbol":"QQQ","price":<current>,"prev_close":<prev>,...}

# 3. Engine logs show OpenD reachable (not "not reachable")
docker logs mmn-engine 2>&1 | grep -iE "moomoo.*reachable|moomoo.*not reachable"
# Should NOT see "not reachable — using Yahoo"
```
