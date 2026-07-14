# deploy/

Production deployment for MarketMarkovNet.

## Status

**Feature 14 (docker-compose) shipped.** The stack:

- `docker-compose.yml` — three services (`inference`, `engine`, `proxy`)
  on an internal `mmn` bridge network.
- `Caddyfile` — reverse proxy, auto-issues a Let's Encrypt cert when
  `HOST` is set to a public hostname; plain HTTP on `:80` for local use.
- `Dockerfile.engine` lives at `engine/Dockerfile` (build context is the
  workspace root).

Companion docs:

- `KRAKEN_KEYS.md` — how to mint the Kraken key (Query + Trade only,
  Withdraw disabled).
- `config.md` — env-var reference (authoritative source).
- `PROVISIONING.md` — VPS setup checklist.
- `setup.sh` — `setup.sh` VPS hardening script (Feature 02).

## Layout

```
deploy/
├── README.md            # this file
├── config.md            # env-var reference
├── PROVISIONING.md      # VPS provisioning checklist
├── setup.sh             # hardening script (Feature 02)
├── KRAKEN_KEYS.md       # key creation checklist
├── docker-compose.yml   # full stack (Feature 14)
└── Caddyfile            # reverse-proxy config
```

## Quick start (production VPS)

1. **Provision the host** — see `PROVISIONING.md` and run `setup.sh`.
   Verify `ufw status` shows `22, 80, 443` allowed and nothing else.

2. **Install the repo + models** at `/opt/marketmarkovnet/`:

   ```bash
   sudo mkdir -p /opt/marketmarkovnet/{app,models,data}
   sudo chown -R $USER:$USER /opt/marketmarkovnet
   cd /opt/marketmarkovnet/app
   git clone <repo-url> .
   # Drop model.pt and norm_stats.json into /opt/marketmarkovnet/models/.
   chmod 0600 /opt/marketmarkovnet/models/*
   ```

   The `models/` directory is bind-mounted into both the engine and
   inference containers at `/models:ro`. After bringing the stack up
   once (so the named `models` volume is created), re-populate the
   volume with the real artifacts:

   ```bash
   docker run --rm \
       -v marketmarkovnet_models:/target \
       -v /opt/marketmarkovnet/models:/src:ro \
       alpine cp -a /src/. /target/
   ```

   (Subsequent updates: `docker run --rm -v
   marketmarkovnet_models:/target -v $(pwd)/models:/src:ro alpine cp
   -a /src/. /target/ && docker-compose -f deploy/docker-compose.yml
   restart inference`. The engine only reads `/models/norm_stats.json`
   and doesn't need to be restarted for model changes.)

3. **Configure secrets** in `/opt/marketmarkovnet/app/.env` (the
   compose file's `env_file` resolves to `../.env` from
   `deploy/docker-compose.yml`):

   ```bash
   cp .env.example .env
   chmod 0600 .env
   $EDITOR .env   # set KRAKEN_*, TRADING_MODE, HOST, etc.
   ```

4. **Build images and bring up**:

   ```bash
   docker-compose -f deploy/docker-compose.yml build
   docker-compose -f deploy/docker-compose.yml up -d
   ```

5. **Verify**:

   ```bash
   docker-compose -f deploy/docker-compose.yml ps            # all "healthy"
   docker-compose -f deploy/docker-compose.yml logs -f       # tail logs
   curl -fsSL https://$HOST/api/status | jq                  # JSON reply
   ```

6. **Confirm the network is closed**:

   ```bash
   # From a host *outside* the VPS, attempt to reach port 5555 — must fail.
   nc -vz $VPS_PUBLIC_IP 5555  || echo "5555 blocked (good)"
   # Port 8080 is also internal — must be unreachable.
   nc -vz $VPS_PUBLIC_IP 8080  || echo "8080 blocked (good)"
   # Port 80/443 reachable, redirects 80→443, /api/status works.
   ```

## Quick start (local dev)

For a no-TLS, all-loopback run:

```bash
# .env stays at the defaults; TRADING_MODE=paper, HOST unset.
docker-compose -f deploy/docker-compose.yml up -d
curl -s http://localhost/api/status | jq
```

Caddy falls back to plain HTTP on `:80` when `HOST` is unset or
`localhost`.

## Operations

### Tail logs

```bash
docker-compose -f deploy/docker-compose.yml logs -f          # all services
docker-compose -f deploy/docker-compose.yml logs -f engine   # one service
```

### Restart a single service

```bash
docker-compose -f deploy/docker-compose.yml restart engine
```

### Update the engine after a code change

```bash
cd /opt/marketmarkovnet/app
git pull
docker-compose -f deploy/docker-compose.yml build engine
docker-compose -f deploy/docker-compose.yml up -d engine
```

The new container starts; the old one is removed once the new
container is healthy. The named `data` volume is reused, so SQLite +
parity marker persist.

### Refresh the parity marker (required weekly when running live)

The engine refuses `TRADING_MODE=live` if the parity marker at
`/app/data/parity_verified.json` is older than `PARITY_MAX_AGE_SECS`
(default 7 days). Re-run the harness inside the engine container:

```bash
docker-compose -f deploy/docker-compose.yml exec engine \
    /usr/local/bin/engine --refresh-parity-marker
# (or whatever CLI entry point is added for this; for now, the
# operator runs the parity harness on the host and `docker cp` the
# marker into the data volume.)
```

If the engine image ships a CLI subcommand, the operator can run it
directly; otherwise, run the harness on the host (where the golden
fixture lives) and write the marker into the data volume:

```bash
# On the host:
cargo test --release -- --ignored parity_refresh   # or your runner
# Then:
docker-compose -f deploy/docker-compose.yml cp \
    ./parity_verified.json mmn-engine:/app/data/parity_verified.json
```

### Backups

The only stateful volume is `data` (SQLite + parity marker). Back it up
off-host:

```bash
docker-compose -f deploy/docker-compose.yml stop engine     # quiesce DB
docker run --rm \
    -v marketmarkovnet_data:/data:ro \
    -v $(pwd):/backup \
    alpine tar czf /backup/data-$(date -u +%Y%m%dT%H%M%SZ).tgz -C /data .
docker-compose -f deploy/docker-compose.yml start engine
```

`.env` is not in the volume — it must be backed up separately to a
secrets manager (never to the repo).

### Tear down

```bash
docker-compose -f deploy/docker-compose.yml down            # keep volumes
docker-compose -f deploy/docker-compose.yml down -v         # destroy volumes
```

## Security notes

- **Only the `proxy` service publishes host ports** (80 + 443). UFW
  must allow exactly these on the VPS. Port 5555 (ZMQ) and 8080 (Axum)
  must never be reachable from the public internet.
- **All services run as non-root** inside their containers.
- **`models` is mounted read-only** in both the engine and the
  inference containers.
- **The inference container has no secrets** (no Kraken keys, no
  anything else sensitive) — the `env_file: ../.env` on the
  inference service is wasteful but not insecure; the keys are
  ignored. Operators can trim it later.
- **Caddy handles ACME** for a real hostname. For staging, leave
  `HOST` unset and Caddy serves plain HTTP only.
- **TLS certs persist in the `caddy_data` volume**. Don't `down -v`
  on a production host without a backup plan.

## See also

- `../plans/market-markov-net/features/14 - docker-compose deploy and launch.md` — spec.
- `config.md` — env-var reference.
- `PROVISIONING.md` — VPS setup.
- `KRAKEN_KEYS.md` — Kraken key creation checklist.
