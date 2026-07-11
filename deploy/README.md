# deploy/

This directory holds Docker Compose files and provisioning notes for the
MarketMarkovNet deployment.

## Status

**Feature 02 (VPS hardening) + Feature 14 (Docker Compose) placeholders.**
The full content lands in those features:

- `setup.sh` — VPS hardening script (UFW, SSH, fail2ban, unattended-upgrades).
  Full content in **Feature 02**.
- `docker-compose.yml`, `Dockerfile.engine`, `Dockerfile.inference` — Compose
  stack. Full content in **Feature 14**.

## Layout (planned)

```
deploy/
├── README.md           # this file
├── config.md           # env-var reference
├── PROVISIONING.md     # VPS provisioning checklist
├── setup.sh            # hardening script (Feature 02)
├── docker-compose.yml  # full stack (Feature 14)
├── Dockerfile.engine   # Rust engine image (Feature 14)
└── Dockerfile.inference  # Python inference image (Feature 05)
```

## Quick reference

| File | Feature | Purpose |
|------|---------|---------|
| `config.md` | 01 | Env-var documentation |
| `PROVISIONING.md` | 02 | VPS provisioning outline |
| `setup.sh` | 02 | Hardening script (chmod 0644 until Feature 02) |
| `Dockerfile.inference` | 05 | Python image |
| `Dockerfile.engine` | 14 | Rust image |
| `docker-compose.yml` | 14 | Full stack |
