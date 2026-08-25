# MarketMoves — VPS Deployment Runbook

> **Status:** LIVE · Updated 2026-08-22 · VPS `vps-ab0567fd` (Ubuntu, user `ubuntu`)
> This replaces the July BTC/Kraken deployment doc. The stack is now a **QQQ/SMH/XLF equities + options** system on Futu/Moomoo.

---

## 1. Service inventory (what actually runs on this VPS)

| Service | How it runs | Network | Health signal | Data |
|---|---|---|---|---|
| **mmn-engine** | Docker container | host 8080, Docker net `deploy_mmn` | `docker ps` → `healthy` | `deploy_data` → `/app/data` |
| **mmn-inference** | Docker container | Docker net `deploy_mmn` (no host port) | `docker ps` → `healthy` | `deploy_models` → `/models` (ro) |
| **mmn-proxy** (Caddy) | Docker container | host 9080→80, 9443→443 | **always reports `unhealthy`** (auth 401) — FALSE NEGATIVE | bind Caddyfile + `deploy_caddy_data`, `deploy_caddy_config` |
| **options-recorder** | systemd **user** unit | host `127.0.0.1:9080` → engine | `systemctl --user status options-recorder` | `data/options_tape/` on host |
| **opend** (Futu gateway) | systemd **user** unit | host `127.0.0.1:11111` | `systemctl --user status opend` | OpenD working dir, cached `Broker.dat` |
| **hermes-gateway** | systemd **user** unit | messaging (Telegram etc.) | `systemctl --user status hermes-gateway` | — |

All named docker containers attach to the **shared network `deploy_mmn`**, so containers resolve each other by name.

---

## 2. Core invariants (do not change)

- **DB volume:** `deploy_data` → `/app/data` (SQLite at `/app/data/candles.db`). Removing this volume **wipes the DB** — never recreate it.
- **Models volume:** `deploy_models` → `/models` (mounted **read-only** into engine, engine also uses it for inference weights).
- **Engine container name must be `mmn-engine`** and be on `deploy_mmn` (Caddy routes to `mmn-engine:8080`).
- **`--env-file /home/ubuntu/projects/MarketMoves/.env`** is part of the engine run — it sets `TRADING_MODE=paper`, `SYMBOL=QQQ`, `ZMQ_ENDPOINT`, `DATABASE_URL` etc. Config **lives in `.env`**, not in the image.
- **`ZMQ_ENDPOINT` in `.env` MUST be `tcp://inference:5555`** (the compose-network service name) — never `tcp://127.0.0.1:5555`. Inference is a *separate* container (`mmn-inference`, IP `172.18.0.3`, no published host port), so `127.0.0.1:5555` inside the engine resolves to the engine's own loopback and nothing is listening there. The 127.0.0.1 value is only valid for a native/local `cargo run` setup where Python inference runs in the same process/host. Symptom if wrong: container stays **healthy**, candles keep ingesting, but all equity schedulers fail at boot (`equity scheduler init error: ZMQ connect ... Connect timed out after 30s`) and predictions silently stop while the UI shows fresh data. `.env.example` still ships the 127.0.0.1 value (legacy) — do not copy it blindly.
- **`docker-compose` v1.29.2 is BROKEN on this VPS** (`KeyError: 'ContainerConfig'`). All deploys use raw `docker run`. Do not "fix" a deploy by switching to compose.
- **Caddy auth:** the Caddyfile basic-auth password is **not** in the repo. Do not brute-force/replace it; verify internally via the docker network instead.

---

## 3. Redeploy procedures (per service)

### 3.1 ENGINE — `mmn-engine`

```bash
cd /home/ubuntu/projects/MarketMoves

# 1) Build the image (bakes in a fresh Rust binary + frontend dist)
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .

# 2) Swap the container (data survives on deploy_data)
docker rm -f mmn-engine
docker run -d \
  --name mmn-engine \
  --restart unless-stopped \
  --network deploy_mmn \
  --add-host host.docker.internal:host-gateway \
  -v deploy_data:/app/data \
  -v deploy_models:/models:ro \
  --env-file /home/ubuntu/projects/MarketMoves/.env \
  marketmarkovnet/engine:latest

# 3) Verify
sleep 30 && docker ps --filter name=mmn-engine --format '{{.Names}}: {{.Status}}'
docker logs mmn-engine --since 5m | tail -30
# expect: Up (healthy); boot lines for all 3 models + HTTP on :8080
```

**Nightly hyperopt** is a **self-waking loop inside the engine** (30-min poll, runs at most once per eligible window 20:30→04:30 UTC). It self-gates on `CanRun` and is guarded so it runs exactly once per window (no duplicate candidates). Confirm the pipeline fired:

```bash
docker logs mmn-engine | grep -E 'hyperopt run|Starting nightly|Nightly run complete'
# first fire is ≤30 min after boot if inside the window; otherwise at next 20:30 UTC
```

Recover old binary if rollback needed: the previous image id is in the docker build output; `docker tag <old-id> marketmarkovnet/engine:latest` then repeat step 2/3.

### 3.2 INFERENCE — `mmn-inference`

```bash
cd /home/ubuntu/projects/MarketMoves
docker build -f inference/Dockerfile -t marketmarkovnet/inference:latest .

# Recreate (internal-only, no host port; engine reaches it on deploy_mmn)
docker rm -f mmn-inference
docker run -d \
  --name mmn-inference \
  --restart unless-stopped \
  --network deploy_mmn \
  -v deploy_models:/models:ro \
  marketmarkovnet/inference:latest

# Verify
docker ps --filter name=inference --format '{{.Names}}: {{.Status}}'
docker logs mmn-inference --since 2m | tail -15
```
Role: ML inference inside the docker network (exposed on 5555 for the engine's ZMQ). Cold-start note: z-score de-norm/embedding state can read `0.0` for the first ~2 candles after a restart — predictions populate once real bars flow.

### 3.3 PROXY — `mmn-proxy` (Caddy)

```bash
cd /home/ubuntu/projects/MarketMoves
# Caddyfile is at deploy/Caddyfile (bind-mounted — no image build needed)

docker rm -f mmn-proxy
docker run -d \
  --name mmn-proxy \
  --restart unless-stopped \
  --network deploy_mmn \
  -p 9080:80 \
  -p 9443:443 \
  -v /home/ubuntu/projects/MarketMoves/deploy/Caddyfile:/etc/caddy/Caddyfile:ro \
  -v deploy_caddy_data:/data \
  -v deploy_caddy_config:/config \
  caddy:2-alpine \
  caddy run --config /etc/caddy/Caddyfile --adapter caddyfile

# Verify routes are live (via internal network, not external auth)
docker run --rm --network deploy_mmn caddy:2-alpine wget -qO- http://mmn-engine:8080/api/status
```
Do **not** trust `docker ps` health for the proxy — its healthcheck hits an auth-gated route and 401s, so it reads `unhealthy` even when it's proxying fine. Check the log for actual 5xx before troubleshooting.

---

### 3.4 OPTIONS-RECORDER — systemd user unit

```bash
# Rebuild the binary after code changes to the recorder
cd /home/ubuntu/projects/MarketMoves && cargo build -p options_recorder --release

# Restart the service
systemctl --user restart options-recorder
systemctl --user status options-recorder
journalctl --user -u options-recorder -n 50 --no-pager
```
Runs the host binary `target/release/options_recorder`, `After=opend.service`. Key env (pinned in the unit): underlyings `US.QQQ,US.SMH,US.XLF`, DTE 30–45, delta 0.45, **quota tier 20** (Inah's verified Futu tier — do NOT raise), tape dir `data/options_tape/`, OpenD at `127.0.0.1:11111`, engine via `http://127.0.0.1:9080`.

**Disk hygiene:** the tape dir grows daily. Before/after runs check `du -sh data/options_tape`; prune old tapes if it balloons (never auto-purge without Inah's OK).

### 3.5 OPEND — Futu gateway (systemd user unit)

```bash
systemctl --user restart opend
systemctl --user status opend
journalctl --user -u opend -n 30 --no-pager
```
Auto-login uses **cached `Broker.dat`** (`-login_by_remember=1`); **no password lives in the unit or on the cmdline**. If it fails to come up, verify OpenD can reach Futu, not the service file. Recorder depends on it (`After=opend.service`).

---

## 4. Unified health checklist

```bash
# Containers
docker ps --filter name=mmn --format '{{.Names}}: {{.Status}}'
#   mmn-engine + mmn-inference should be healthy; mmn-proxy "unhealthy" is EXPECTED

# systemd user units
systemctl --user is-active opend options-recorder hermes-gateway

# Engine API through the proxy (internal, bypasses basic auth)
docker run --rm --network deploy_mmn caddy:2-alpine wget -qO- http://mmn-engine:8080/api/status

# Hyperopt pipeline state (per equity)
docker run --rm --network deploy_mmn caddy:2-alpine \
  wget -qO- http://mmn-engine:8080/api/hyperopt/QQQ/status

# Tape recorder heartbeat
journalctl --user -u options-recorder -n 10 --no-pager
```

---

## 5. Known gotchas (encoded so you don't re-derive them)

1. **docker-compose v1 broken** → use raw `docker run` (section 3). Never downgrade/upgrade mid-deploy.
2. **Proxy always `unhealthy`** = false negative (401 healthcheck). Check logs for real 5xx.
3. **Inference z-score cold start** = `0.0` for ~2 bars after restart. Not a bug.
4. **Hyperopt run cadence** = nightly at **20:30→04:30 UTC**, once per window, self-waking. New engine boots do NOT necessarily re-run; the loop waits for CanRun. A run is "see it fired" via `docker logs ... | grep hyperopt run`.
5. **`deploy_data` is the DB** — recreate containers, never the volume. Restart does not wipe it; `docker rm` on the volume-backed mount is safe, removing the *volume* is not.
6. **OpenD on port 11111** is a systemd host process, **not** in docker — recorder connects to `127.0.0.1:11111` on the host.
7. **RunnerConfig default universe = QQQ, SMH, XLF** and it drives both the nightly candidate producer and the promotion applier (main.rs uses the same default). Widen/change equities in `RunnerConfig::default()` only — one source of truth.

---

## 6. Historical / fresh-install caveats (retained from July)

- **Full fresh provision** (`deploy/PROVISIONING.md`, `deploy/setup.sh`) covers Apache2 port-80 conflict, initial volume seeding, and Caddy TLS. Only relevant if this VPS is ever fully rebuilt.
- Earlier "model architecture mismatch" / "deps-warmup cache" / "sqlite 3-slash path" fixes are baked into the Dockerfiles/files now — don't re-apply blindly.