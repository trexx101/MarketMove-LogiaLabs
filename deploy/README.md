# deploy/

Production deployment for MarketMarkovNet — QQQ equities daily model.

## Status

**Wave C (equities) deployed.** The stack:

- `docker-compose.yml` — three services (`inference`, `engine`, `proxy`)
  on an internal `mmn` bridge network.
- `Caddyfile` — reverse proxy, auto-issues a Let's Encrypt cert when
  `HOST` is set to a public hostname; plain HTTP on `:80` for local use.
- `engine/Dockerfile` lives at `engine/Dockerfile` (build context is the
  workspace root).

## Layout

```
deploy/
├── README.md            # this file
├── config.md            # env-var reference
├── PROVISIONING.md      # VPS provisioning checklist
├── setup.sh             # VPS hardening script
├── docker-compose.yml   # full stack
└── Caddyfile           # reverse-proxy config
```

## Quick start (production VPS)

1. **Provision the host** — see `PROVISIONING.md` and run `setup.sh`.
   Verify `ufw status` shows `22, 80, 443` allowed and nothing else.

2. **Install the repo + models** at `/opt/marketmoves/`:

   ```bash
   sudo mkdir -p /opt/marketmoves/{app,models,data}
   sudo chown -R $USER:$USER /opt/marketmoves
   cd /opt/marketmoves/app
   git clone <repo-url> .
   ```

   Drop model artifacts into `/opt/marketmoves/models/`:
   - `qqq_tcn_v1.pt` — TCN weights (from Colab export)
   - `qqq_lgbm_h1_v1.pkl`, `qqq_lgbm_h5_v1.pkl`, `qqq_lgbm_h21_v1.pkl` — LGBM models
   - `norm_stats_qqq_v1.json` — normalization stats

   The `models/` directory is bind-mounted into both the engine and
   inference containers at `/models:ro`. After bringing the stack up
   once (so the named `models` volume is created), populate it:

   ```bash
   docker run --rm \
       -v deploy_models:/target \
       -v /opt/marketmoves/models:/src:ro \
       alpine cp -a /src/. /target/
   ```

   Subsequent model updates (weights, norm stats):

   ```bash
   docker run --rm \
       -v deploy_models:/target \
       -v /opt/marketmoves/models:/src:ro \
       alpine cp -a /src/. /target/
   docker restart mmn-inference mmn-engine
   ```

3. **Configure** in `/opt/marketmoves/app/.env` (compose file's `env_file`
   resolves to `../.env` from `deploy/docker-compose.yml`):

   ```bash
   cp .env.example .env
   chmod 0600 .env
   $EDITOR .env   # set HOST, TRADING_MODE=paper, etc.
   ```

   Key variables:

   | Variable | Default | Description |
   |----------|---------|-------------|
   | `TRADING_MODE` | `paper` | `paper` or `live` |
   | `SYMBOL` | `QQQ` | Trade target |
   | `MAGNITUDE_THRESHOLD` | `0.005` | Entry threshold (pred must exceed this) |
   | `SMA_WINDOW` | `200` | SMA regime window |
   | `HOST` | unset | Public hostname for Let's Encrypt (leave unset for local) |

4. **Build images and bring up**:

   ```bash
   docker-compose -f deploy/docker-compose.yml build
   docker-compose -f deploy/docker-compose.yml up -d
   ```

5. **Verify**:

   ```bash
   docker-compose -f deploy/docker-compose.yml ps            # all "healthy"
   docker-compose -f deploy/docker-compose.yml logs -f       # tail logs
   curl -s http://localhost:9080/api/status | jq            # JSON reply
   ```

6. **Confirm the network is closed**:

   ```bash
   # From outside the VPS — ports 5555 and 8080 must be unreachable.
   nc -vz $VPS_PUBLIC_IP 5555  || echo "5555 blocked (good)"
   nc -vz $VPS_PUBLIC_IP 8080  || echo "8080 blocked (good)"
   # Port 9080 (Caddy HTTP) is the only public port.
   ```

## Quick start (local dev)

For a no-TLS, all-loopback run:

```bash
# .env stays at defaults: TRADING_MODE=paper, HOST unset.
docker-compose -f deploy/docker-compose.yml up -d
curl -s http://localhost:9080/api/status | jq
```

Caddy falls back to plain HTTP on `:80` when `HOST` is unset.

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

### Update after a code change

```bash
cd /opt/marketmoves/app
git pull
docker-compose -f deploy/docker-compose.yml build engine
docker-compose -f deploy/docker-compose.yml up -d engine
```

The named `data` volume is reused — SQLite + parity marker persist.

### Refresh the parity marker (required weekly when running live)

The engine refuses `TRADING_MODE=live` if the parity marker at
`/app/data/parity_verified.json` is older than `PARITY_MAX_AGE_SECS`
(default 7 days). Re-run parity verification on the host and copy the
marker into the data volume:

```bash
# On the host:
./target/release/test_harness --filter parity  # run the parity harness
cp parity_verified.json /tmp/
docker-compose -f deploy/docker-compose.yml cp \
    /tmp/parity_verified.json mmn-engine:/app/data/parity_verified.json
```

### Backups

The only stateful volume is `data` (SQLite + parity marker). Back it up
off-host:

```bash
docker-compose -f deploy/docker-compose.yml stop engine
docker run --rm \
    -v deploy_data:/data:ro \
    -v $(pwd):/backup \
    alpine tar czf /backup/data-$(date -u +%Y%m%dT%H%M%SZ).tgz -C /data .
docker-compose -f deploy/docker-compose.yml start engine
```

`.env` is not in the volume — back it up separately to a secrets manager
(never to the repo).

### Tear down

```bash
docker-compose -f deploy/docker-compose.yml down            # keep volumes
docker-compose -f deploy/docker-compose.yml down -v         # destroy volumes
```

## Security notes

- **Only the `proxy` service publishes host ports** (9080 + 9443).
  Port 5555 (ZMQ) and 8080 (Axum) must never be reachable from the
  public internet.
- **All services run as non-root** inside their containers.
- **`models` is mounted read-only** in both the engine and the
  inference containers.
- **TLS certs persist in the `caddy_data` volume**. Don't `down -v`
  on a production host without a backup plan.

## See also

- `config.md` — env-var reference.
- `PROVISIONING.md` — VPS setup.
