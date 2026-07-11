# Provisioning

Outline for provisioning a fresh Ubuntu 24.04 LTS VPS for MarketMarkovNet.

> **Status:** outline only. Full content lands in **Feature 02** (VPS hardening
> & infra setup).

## 1. Base system

- Ubuntu 24.04 LTS, minimal install.
- Non-root user with `sudo` access.
- SSH key auth, password auth disabled.
- `ufw` default-deny, allow `22`, `80`, `443`. **Block** `5555` and any
  internal service ports from the public internet.

## 2. OS hardening (`deploy/setup.sh`)

- `unattended-upgrades` enabled.
- `fail2ban` for SSH.
- `sysctl` hardening (basic).
- Time sync (`chrony` or `systemd-timesyncd`).
- No root login over SSH.
- Disable unused services.

## 3. Docker

- `docker-ce`, `docker-ce-cli`, `containerd.io`, `docker-buildx-plugin`,
  `docker-compose-plugin`.
- User in `docker` group.
- `ufw` + `docker` integration (route `DOCKER-USER` chain) so container ports
  are not silently exposed.

## 4. Filesystem layout

```
/opt/marketmarkovnet/
├── app/                 # this repo (engine, inference, frontend, deploy)
├── models/              # model.pt, norm_stats.json (chmod 0600)
├── data/                # SQLite databases
└── .env                 # secrets (chmod 0600)
```

## 5. Reverse proxy

- Caddy or nginx in front of Axum (port 8080), TLS via Let's Encrypt.
- Static SPA served by Axum at `/`.

## 6. Backups

- `data/*.db` off-site.
- `.env` to a secrets manager (never to the repo).

## 7. Observability

- `journald` for engine logs (or stdout → Docker logs).
- Optional: Promtail/Loki or vector → log sink.

See `../plans/market-markov-net/features/02 - vps hardening.md` for the full
spec.
