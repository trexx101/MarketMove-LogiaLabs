---
name: svelte-vite-serve-from-built-dist
description: Use when Svelte SPA 404s behind Rust ServeDir.
category: software-development
triggers:
  - User asks to deploy a Svelte frontend change to a Rust/Axum service
  - curl localhost returns the Svelte dev shell pointing at /src/main.js and assets 404
  - VPS-side Svelte UI is missing or renders blank despite frontend/dist existing locally
  - Engine serves the Svelte source tree but Svelte components cannot run without Vite
  - Following Phase 0-1 Svelte scaffold + Rust binary serving pattern
---

# Serving a Built Svelte/Vite Frontend from a Rust/Axum Backend

## The bug you are here to fix

`engine/src/main.rs` and `engine/src/api/mod.rs` both serve static
frontend files via `ServeDir::new("frontend")`. When the binary
runs in a container with WORKDIR=`/app`, it reads from
`/app/frontend/`.

The Dockerfile currently copies the entire `frontend/` directory
into the image. But Svelte is NOT like a static HTML/CSS/JS site:
those `.svelte` component files need Vite's preprocessor/build
step. If you just `COPY frontend/ /app/frontend/` and start the
engine:

1. `curl http://localhost:8080/` returns the dev shell HTML with
   `<script type="module" src="/src/main.js">`.
2. `curl http://localhost:8080/src/main.js` returns 404 because
   ServeDir does not route untransformed Svelte source.
3. The browser console fails silently and the SPA renders blank.
4. API endpoints (`/api/status`, `/api/mode`) work fine. The bug
   looks like the API is broken but it is the SPA shell.

## The right fix — Dockerfile change, do this not the workaround

The clean shape is a **separate `frontend-build` stage** that runs
`npm ci && npm run build` once, then the runtime stage copies ONLY
`frontend/dist/` from that stage into `/app/frontend/`. Do NOT inline
the npm build into the Rust build stage — Node toolchain in the Rust
image wastes ~250MB and the layer cache for npm and cargo want
different invalidation rules.

### Canonical multi-stage Dockerfile (drop-in for engine/Dockerfile)

```dockerfile
# syntax=docker/dockerfile:1.7

# Stage 1 — frontend bundle (independent of Rust)
FROM node:20-bookworm-slim AS frontend-build
WORKDIR /build/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN npm run build          # writes /build/frontend/dist/

# Stage 2 — engine binary (independent of frontend)
FROM rust:1-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY engine/Cargo.toml engine/
# (existing cargo dep-warmup sentinel pattern — keep it)
RUN mkdir -p engine/src \
    && echo 'fn main() { println!("deps-warmup"); }' > engine/src/main.rs \
    && echo '' > engine/src/lib.rs \
    && cargo build --release --manifest-path engine/Cargo.toml \
    && rm -rf engine/src
COPY engine/ engine/
RUN touch engine/src/main.rs \
    && cargo build --release --manifest-path engine/Cargo.toml \
    && strip target/release/engine \
    && cp target/release/engine /usr/local/bin/engine

# Stage 3 — runtime (Debian slim)
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl tini libsqlite3-0 libssl3 \
    && rm -rf /var/lib/apt/lists/*
RUN groupadd --system --gid 1000 mmn \
    && useradd --system --uid 1000 --gid mmn --home /app --shell /usr/sbin/nologin mmn
WORKDIR /app
RUN mkdir -p /app/data && chown -R mmn:mmn /app
COPY --from=build /usr/local/bin/engine /usr/local/bin/engine
# ONLY the built dist — NOT the source tree. The engine's
# ServeDir::new("frontend") reads /app/frontend/index.html + assets/.
# --chown is mandatory so the mmn user can traverse parent dirs.
COPY --from=frontend-build --chown=mmn:mmn /build/frontend/dist/ /app/frontend/
USER mmn
EXPOSE 8080
# ... ENV / ENTRYPOINT / HEALTHCHECK ...
```

Then rebuild + recreate:
```bash
docker build -f engine/Dockerfile -t <image>:latest .
docker rm -f <container>
docker compose -f deploy/docker-compose.yml up -d
```

Verification: served HTML has `<script src="/assets/index-HASH.js">`,
bundle returns 200, `/src/main.js` returns 404 (dev shell gone).

### The old in-line recipe was wrong

The earlier version of this skill recommended:
```dockerfile
RUN npm ci --omit=dev || npm install
```
…inside the Rust build stage. Three problems: (1) the `|| npm install`
silently swallows `npm ci` failures and degrades to a non-reproducible
install with whatever `package.json` says — exactly the lockfile-drift
class of bugs this skill is trying to prevent; (2) Node toolchain
stays in the Rust build image (~250MB extra); (3) if you add
cross-compilation later, the Node toolchain needs host-arch binaries
that may break the cross build. Use `npm ci --no-audit --no-fund`
unconditionally — if the lockfile is stale, `npm ci` will fail
loudly at build time, which is the correct signal.

### Single-stage fallback when 3 stages won't fit

If CI or image-size budgets forbid three stages, keep Rust+Node in
one stage but split the npm COPY into its own layer ABOVE the source
COPY:

```dockerfile
FROM rust:1-bookworm AS build
WORKDIR /build
# ... cargo warmup ...
COPY engine/ engine/
# Frontend deps BEFORE source so lockfile-only edits don't bust cargo
COPY frontend/package.json frontend/package-lock.json frontend/
RUN cd frontend && npm ci --no-audit --no-fund
COPY frontend/ frontend/
RUN cd frontend && npm run build
RUN touch engine/src/main.rs && cargo build --release ...
```

The order `manifests → npm ci → source COPY → npm run build → cargo
build` is the npm equivalent of cargo's `Cargo.toml → dep warmup →
source COPY → build` pattern: lockfile-only edits don't trigger a
full source-tree COPY.

## The workaround when you cannot change the Dockerfile yet

Common case: the Dockerfile change is blocked by another in-flight
change, OR you just need a quick deploy to show the UI today. You
can build locally, then inject the `dist/` contents into the
running container.

### Step 1 — Build locally on the host

```bash
cd frontend
npm install
npm run build
```

Output goes to `frontend/dist/`:

```
dist/index.html
dist/assets/index-XXXXXX.js
dist/assets/index-XXXXXX.css
```

### Step 2 — Verify the dist contents

```bash
ls frontend/dist/
ls frontend/dist/assets/
cat frontend/dist/index.html
```

`index.html` should reference `/assets/index-*.js`.

### Step 3 — Swap dist into the running container

```bash
docker exec mmn-engine bash -c '
  cd /app/frontend
  cp dist/index.html ./index.html
  cp -r dist/assets ./assets
  rm -f src/main.js
  rm -rf src dist node_modules legacy
  ls /app/frontend/
'
```

Removing `src/` is critical — if the old Svelte source remains,
ServeDir can serve those `.svelte` files when the dist lookup
misses, causing weird hydration issues.

### Step 4 — Verify

```bash
curl -s http://localhost:<port>/ | grep 'src="/assets'
curl -sI http://localhost:<port>/assets/index-HASH.js
```

Shell should return the bundle reference; bundle should be 200 with
`text/javascript`.

### Step 5 — Note for yourself

This is a workaround that disappears the next time the image is
rebuilt from the unmodified Dockerfile. Commit the Dockerfile
change so the next `docker-compose build engine` produces the right
output.

## What about the legacy vanilla JS frontend

Some deployments carry BOTH:

- `frontend/dist/` — built Svelte bundle (new)
- `frontend/app.js`, `frontend/views/`, `frontend/legacy/` — vanilla
  JS SPA from before the Svelte migration

If the Dockerfile copies the source tree, the engine serves whichever
file `curl localhost:8080/` resolves first — usually the legacy one.

Tell the two apart quickly:

```bash
# Legacy vanilla SPA:
curl -s http://localhost:8080/ | grep 'app.js'
# Svelte SPA:
curl -s http://localhost:8080/ | grep 'assets/index-.*\.js'
```

If you see `app.js` you need to (a) build the Svelte dist, (b)
delete or rename the legacy `index.html` before rebuilding the
image.

## Companion pitfalls

### HEALTHCHECK command left over from legacy binary

When the API payload renames a field (e.g. `trading_mode` → `mode`),
the Dockerfile HEALTHCHECK's `grep -q '"trading_mode"'` fails and
the container shows "unhealthy" even though `/api/status` returns
200. Engine is alive and serving; the healthcheck just cannot see
the new field. Update BOTH `engine/Dockerfile`'s `HEALTHCHECK`
directive AND `deploy/docker-compose.yml`'s `engine.healthcheck.test`
array to grep for the new field.

Verification: `docker inspect mmn-engine --format '{{.State.Health.Status}}'`
should return `healthy` after the fix.

### docker-compose v1 KeyError workaround

On hosts with docker-compose v1 (e.g. 1.29.2) talking to a newer
Docker server (28+), `docker-compose up -d` fails with
`KeyError: 'ContainerConfig'`. Workaround: do the lifecycle
manually with `docker run`, mirroring the compose env.

```bash
# Capture the old container's env + mounts + network first
docker inspect mmn-engine --format '{{json .HostConfig.Mounts}}'
docker inspect mmn-engine --format '{{range .Config.Env}}{{println .}}{{end}}'

# Tear down + recreate with the same config
docker rm -f mmn-engine
docker run -d \
  --name mmn-engine \
  --restart unless-stopped \
  --network deploy_mmn \
  -u 1000:1000 \
  -v deploy_data:/app/data \
  -v deploy_models:/models:ro \
  --env-file .env \
  -e TRADING_MODE=paper \
  -e ZMQ_ENDPOINT=tcp://inference:5555 \
  -e HTTP_PORT=8080 \
  -e DATABASE_URL=sqlite:///app/data/candles.db \
  -e NORM_STATS_PATH=/models/norm_stats_qqq_v1.json \
  -e PARITY_MARKER_PATH=/app/data/parity_verified.json \
  -e RUST_LOG=info \
  marketmarkovnet/engine:latest
```

The volumes (`deploy_data`, `deploy_models`) and network
(`deploy_mmn`) survive — no data loss. After recreate, verify:

```bash
docker ps --format '{{.Names}}\t{{.Status}}'
curl localhost:9080/api/status
```

### Container name conflict on docker-compose up -d after rebuild

After `docker-compose build engine`, `docker-compose up -d engine` fails with:

```
ERROR: for mmn-engine  Cannot create container for service engine: Conflict.
The container name "/mmn-engine" is already in use by container "<id>".
```

`docker-compose up -d` does NOT remove the old container when the image hash
changed — it tries to create a new one with the same name. Fix: `docker rm -f`
the old container first, then `up -d`:

```bash
docker rm -f mmn-engine
docker-compose up -d engine
```

`docker-compose rm -f engine` does NOT work — it only removes *stopped*
containers, and the old one is still running. Use `docker rm -f` directly.

### Auto-minted TOTP_SECRET disappears on restart

When the engine starts with `TOTP_SECRET=` (empty), it mints a
fresh secret and logs the otpauth URL. If you do not persist that
secret in `.env`, the next restart mints a new one and you have to
re-scan the QR in your authenticator. For paper-only deployments
it is harmless — for any deployment that will ever flip to live,
persist immediately:

```bash
# paste the secret from the engine WARN log into .env
TOTP_SECRET=<from engine log>
docker rm -f mmn-engine && docker run ... marketmarkovnet/engine:latest
```

Or accept the regeneration if paper-only.

## Verification checklist

- [ ] `curl http://localhost:<port>/` returns Svelte shell pointing
  at `/assets/index-HASH.js`, not legacy `app.js` or `/src/main.js`
- [ ] `curl -sI http://localhost:<port>/assets/index-HASH.js` returns
  200, `content-type: text/javascript`
- [ ] `docker inspect mmn-engine --format '{{.State.Health.Status}}'`
  returns `healthy` (after HEALTHCHECK + compose `test:` are aligned
  to the new API payload)
- [ ] If the change was the Dockerfile workaround only (not the real
  fix), TODO is logged to swap `COPY frontend/` for a Node build
  stage + `COPY dist/`
