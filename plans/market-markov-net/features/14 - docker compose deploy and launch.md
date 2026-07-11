# Feature 14 — Docker Compose Deploy & Launch

**Depends on:** all (01–13)
**Goal:** Orchestrate the full stack and bring it up in production on the VPS.

## Requirements

- `docker-compose.yml` defining: the Rust engine, the Python inference service, and a reverse proxy (Caddy/Nginx) terminating 80/443 → Axum.
- Internal network so the engine reaches inference on `:5555`; 5555 never published to host.
- Volumes for `/models` (ro) and the SQLite DB (persistent).
- Env/secrets injected from `.env`; restart policies set.
- `docker-compose up -d` for production; monitored via the control room.

## Technical Implementation Steps

1. `deploy/docker-compose.yml`: services `engine`, `inference`, `proxy`; internal network; volumes; healthchecks; `restart: unless-stopped`.
2. `engine/Dockerfile`: multi-stage Rust build → slim runtime.
3. Reverse proxy config routing `/` and `/api/*` to Axum, TLS via Let's Encrypt (Caddy auto or Nginx+certbot).
4. Wire secrets/env; document `up -d` / logs / update procedure in `deploy/README.md`.
5. Bring up on VPS; verify control room reachable on 80/443 and 5555 not externally reachable.

## Acceptance Criteria

- [ ] `docker compose up -d` starts all services; healthchecks green.
- [ ] Control room reachable on 80/443; `/api/*` responds.
- [ ] Port 5555 not reachable from outside the compose network (verified).
- [ ] SQLite + models persist across container restarts.
- [ ] Engine successfully reaches inference over ZMQ inside the network.
