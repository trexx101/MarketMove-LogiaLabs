# deploy/

Production deployment for MarketMoves — the QQQ daily-equities control room.

## Status

**Paper mode is the entire production story.** The stack runs the full
inference + strategy + paper-execution pipeline end-to-end, with no broker
credentials on the host and the runtime mode toggle disabled at the config
layer. Going live is opt-in (see *Going live* below) and gated by the
deploy gate in the engine.

```
deploy/
├── README.md            # this file
├── config.md            # env-var reference
├── PROVISIONING.md      # VPS provisioning checklist
├── setup.sh             # VPS hardening script
├── docker-compose.yml   # full stack
└── Caddyfile           # reverse-proxy config
```

## Stack

- `docker-compose.yml` — three services (`inference`, `engine`, `proxy`)
  on an internal `deploy_mmn` bridge network.
- `Caddyfile` — reverse proxy. Auto-issues a Let's Encrypt cert when
  `HOST` is set to a public hostname; falls back to plain HTTP on `:80`
  for local use. The proxy is the only service that publishes host ports.
- `engine/Dockerfile` lives at `engine/Dockerfile` (build context is the
  workspace root). The engine image is built in two stages: a Node stage
  builds the Svelte frontend, and a Rust stage builds the engine binary
  + native-tls deps. The runtime image serves the compiled `dist/` at
  `/`, so the Svelte UI is baked in — no manual workaround at deploy time.

## Quick start (local dev)

This is the simplest path. No TLS, all loopback, paper mode.

```bash
# 1. Configure
cp .env.example .env
chmod 0600 .env
# .env defaults: TRADING_MODE=paper, HOST unset.

# 2. Build + launch
docker compose -f deploy/docker-compose.yml build
docker compose -f deploy/docker-compose.yml up -d

# 3. Verify
docker compose -f deploy/docker-compose.yml ps        # all "healthy"
curl -s http://localhost:9080/api/status | jq
curl -s http://localhost:9080/api/mode   | jq        # Phase 3 mode toggle
```

Open `http://localhost:9080/` for the Svelte control room. The status
panel shows PAPER. Click ⇄ to open the mode toggle modal — the parity
marker will show as expired (no marker exists yet), so the live-flip
button is disabled.

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
   inference containers at `/models:ro`. After the first `docker compose up`,
   populate the named volume:

   ```bash
   docker run --rm \
       -v deploy_models:/target \
       -v /opt/marketmoves/models:/src:ro \
       alpine cp -a /src/. /target/
   ```

3. **Configure** in `/opt/marketmoves/app/.env`. `docker-compose.yml`'s
   `env_file: ../.env` resolves to the workspace root:

   ```bash
   cp .env.example .env
   chmod 0600 .env
   $EDITOR .env
   ```

   Set at minimum:
   - `HOST=` to your public hostname (Caddy will auto-issue a Let's
     Encrypt cert on this).
   - `TRADING_MODE=paper` (the default — leave it).
   - Leave `LIVE_EXECUTOR=paper`, `MOOMOO_TRD_ENV=SIMULATE`, and
     `TOTP_SECRET` empty for the initial deploy.

4. **Build and bring up**:

   ```bash
   docker compose -f deploy/docker-compose.yml build
   docker compose -f deploy/docker-compose.yml up -d
   ```

5. **Verify**:

   ```bash
   docker compose -f deploy/docker-compose.yml ps        # all "healthy"
   docker compose -f deploy/docker-compose.yml logs -f engine
   curl -fsSL https://$HOST/api/status | jq
   curl -fsSL https://$HOST/api/mode   | jq
   ```

6. **Confirm the network is closed**:

   ```bash
   # From outside the VPS — these ports must be unreachable.
   nc -vz $VPS_PUBLIC_IP 5555  || echo "5555 blocked (good)"
   nc -vz $VPS_PUBLIC_IP 8080  || echo "8080 blocked (good)"
   # Only 80/443 (proxied via Caddy) are reachable.
   ```

## Going live

The stack ships in paper mode. Going live is a deliberate, multi-step
process gated by the engine's deploy gate:

1. **Verify the deploy gate** — the equity strategy must clear
   walk-forward OOS IC > 0.03 with a positive equity curve on the
   replay backtest. Run the gate locally before promoting:
   ```bash
   cargo run --release --bin parity-harness
   ```
   The harness writes `parity_verified.json` with `verified_at`,
   `fixture_sha256`, `max_abs_error`.

2. **Refresh the parity marker** on the VPS — copy the marker into the
   engine's data volume:
   ```bash
   scp parity_verified.json deploy@$VPS:/tmp/
   docker compose -f deploy/docker-compose.yml cp \
       /tmp/parity_verified.json mmn-engine:/app/data/parity_verified.json
   ```

3. **Provision Moomoo OpenD** on the VPS. Install the OpenD daemon,
   unlock the trade GUI. Set `MOOMOO_SECURITY_FIRM` and
   `MOOMOO_CREDS_PATH` in `.env`. OpenD is required even for paper-mode
   trades routed through the Moomoo executor — it runs in `SIMULATE`
   against the broker's paper-account ledger.

4. **Switch `LIVE_EXECUTOR=moomoo`** in `.env`. Rebuild the engine
   image (`docker compose build engine`).

5. **Generate + persist TOTP** — start the engine once in paper mode
   with `TOTP_SECRET=` empty. The engine logs an `otpauth://` URL on
   startup. Scan it with your authenticator app, then persist the
   secret in `.env` so the next restart doesn't regenerate:
   ```bash
   # .env
   TOTP_SECRET=RUYYDERXOLN2IUMSPRUAIIMA34G2P2
   ```

6. **Flip to live from the dashboard** — open the mode toggle modal in
   the Svelte UI, enter the 6-digit TOTP code from your authenticator
   app. The endpoint validates the TOTP, re-checks the parity marker
   is fresh (< `PARITY_MAX_AGE_SECS`), flips the engine's
   `TradingMode`, and writes a row to the `mode_switches` audit table.

7. **Verify the audit row** — `mode_switches` is the only ground truth
   for "who flipped when". The engine never flips to live without
   logging it.

## Operations

### Tail logs

```bash
docker compose -f deploy/docker-compose.yml logs -f          # all services
docker compose -f deploy/docker-compose.yml logs -f engine   # one service
```

### Restart a single service

```bash
docker compose -f deploy/docker-compose.yml restart engine
```

### Update after a code change

```bash
cd /opt/marketmoves/app
git pull
docker compose -f deploy/docker-compose.yml build engine
docker compose -f deploy/docker-compose.yml up -d engine
```

The named `data` volume is reused — SQLite + parity marker persist.

### Refresh the parity marker (required weekly when running live)

The engine refuses `TRADING_MODE=live` if the parity marker at
`/app/data/parity_verified.json` is older than `PARITY_MAX_AGE_SECS`
(default 7 days). The runtime `/api/mode` endpoint re-checks the
marker at request time — startup freshness is not sufficient.

```bash
# On the host:
cargo run --release --bin parity-harness
scp parity_verified.json deploy@$VPS:/tmp/
docker compose -f deploy/docker-compose.yml cp \
    /tmp/parity_verified.json mmn-engine:/app/data/parity_verified.json
```

### Backups

The only stateful volume is `data` (SQLite + parity marker). Back it
up off-host:

```bash
docker compose -f deploy/docker-compose.yml stop engine
docker run --rm \
    -v deploy_data:/data:ro \
    -v $(pwd):/backup \
    alpine tar czf /backup/data-$(date -u +%Y%m%dT%H%M%SZ).tgz -C /data .
docker compose -f deploy/docker-compose.yml start engine
```

`.env` is **not** in the volume — back it up separately to a secrets
manager (never to the repo).

### Tear down

```bash
docker compose -f deploy/docker-compose.yml down            # keep volumes
docker compose -f deploy/docker-compose.yml down -v         # destroy volumes
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
- **Paper mode has no broker credentials.** The `MoomooExecutor` is
  dead code when `LIVE_EXECUTOR=paper` (the default). Even with
  `TRADING_MODE=live`, the engine falls back to `PaperExecutor` if
  `LIVE_EXECUTOR` is not set to `moomoo` — see
  [`engine/src/main.rs:build_executor_for_mode`](../engine/src/main.rs).

## Common pitfalls

| Symptom | Cause | Fix |
|---------|-------|-----|
| Engine container stays `unhealthy` after rebuild | The compose healthcheck pattern expects `"mode":"paper"` in `/api/status`. The pre-Phase 3 binary used `"trading_mode"`. | Rebuild the engine image — `engine/Dockerfile` and `deploy/docker-compose.yml` are aligned. |
| Svelte UI loads but JS console shows `404 /assets/index-*.js` | The engine is serving the Svelte dev shell (`/src/main.js`) instead of the built bundle. The Dockerfile's Node build stage failed silently. | Run `docker compose build --no-cache engine` and check the build output. |
| `/api/mode` returns 200 with `parity_valid: false` | The parity marker is missing or stale. Expected on a fresh deploy. | Run `parity-harness` and copy the marker into the data volume. |
| FRED backfill logs `connection timed out` for VIXCLS / DGS10 / DTWEXBGS | FRED's Akamai edge is unreachable from this VPS (no IPv6 route, SYN hangs). The macro features degrade to 0.0 and the engine falls back to Yahoo `^VIX` for `$VIX`. | Documented in `engine/src/data/fred.rs`. No action needed in paper mode. |
| `docker-compose` (v1) errors out with `KeyError: ContainerConfig` | The CLI/server version mismatch. Compose v1 doesn't work with Docker server 25+. | Use `docker compose` (v2). Refresh your shell's PATH after installing the plugin. |

## See also

- `config.md` — env-var reference.
- `PROVISIONING.md` — VPS setup.
- [`../README.md`](../README.md) — top-level overview.
- [`.hermes/plans/control-room/PHASE_3_EXECUTION_SHORTING.md`](../.hermes/plans/control-room/PHASE_3_EXECUTION_SHORTING.md) — the live-deploy steps and rationale.
