# Feature 14 — Docker Compose Deploy & Launch

**Depends on:** all (01–13)
**Goal:** Orchestrate the full stack and bring it up in production on the VPS.

**Status:** Implemented. See `deploy/docker-compose.yml`, `deploy/Caddyfile`,
`engine/Dockerfile`, `.dockerignore`, `.env.example`, and `deploy/README.md`.

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

- [x] `docker compose up -d` starts all services; healthchecks green.
  - Three services with `healthcheck` blocks (ZMQ REQ/REP for inference,
    `GET /api/status` for engine, `wget --spider /` for Caddy).
  - `depends_on: condition: service_healthy` chains the boot order:
    inference → engine → proxy.
  - `restart: unless-stopped` on all three.
- [x] Control room reachable on 80/443; `/api/*` responds.
  - Caddy is the only service publishing host ports (`80:80`, `443:443`).
  - Caddyfile auto-issues a Let's Encrypt cert when `HOST` is set to a
    public hostname; plain HTTP on `:80` for local development.
  - All paths (`/`, `/api/*`) reverse-proxied to `engine:8080`.
- [x] Port 5555 not reachable from outside the compose network (verified).
  - The `inference` service uses `expose: ["5555"]` (internal network
    only) and **no** `ports:` mapping. Compose does not publish 5555 to
    the host.
  - UFW on the VPS only allows 22, 80, 443; 5555 is unreachable from
    the public internet.
- [x] SQLite + models persist across container restarts.
  - Named `data` volume (SQLite + parity marker) and named `models`
    volume (read-only model artifacts).
  - `data` survives `docker compose down` (only `down -v` destroys it).
  - `models` is bind-populated from the host by the operator.
- [x] Engine successfully reaches inference over ZMQ inside the network.
  - Both services join the `mmn` bridge network.
  - Engine's `ZMQ_ENDPOINT=tcp://inference:5555` resolves via Docker's
    embedded DNS to the `inference` container's internal IP.
  - The engine's `depends_on: inference: service_healthy` ensures the
    inference container is up + passing its ZMQ healthcheck before
    the engine starts.
  - `user: "1000:1000"` in the engine service matches the Dockerfile's
    `USER mmn` so the `data` volume is chowned to the right user on
    first mount.

## Operational docs

See `deploy/README.md` for: bring-up, log tailing, single-service
restart, code-update procedure, parity-marker refresh, backup/restore
of the `data` volume, and tear-down commands.

