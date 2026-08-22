---
name: docker-compose-deployment
title: Deploy and troubleshoot local Docker Compose stacks
description: Use when deploying or debugging local Docker Compose stacks.
triggers:
  - deploy
  - docker compose
  - docker-compose
  - stack redeploy
  - container recreation
  - compose error
---

# Docker Compose Local Deployment

## 1. Decide v1 vs v2

| Tool | Command | Common on |
|------|---------|-----------|
| Compose v1 plugin | `docker compose …` | Docker 20.10+ with plugin installed |
| docker-compose standalone | `docker-compose …` | Ubuntu/Debian packages, older installs |

Try v1 first; fall back to v2 only if v1 is absent:

```bash
# v1
which docker-compose && docker-compose -f deploy/docker-compose.yml ps

# v2
which docker && docker compose -f deploy/docker-compose.yml ps
```

## 2. Standard redeploy loop

Build, then recreate with the new image:

```bash
# v1
docker-compose -f deploy/docker-compose.yml build
docker-compose -f deploy/docker-compose.yml up -d

# v2
docker compose -f deploy/docker-compose.yml build
docker compose -f deploy/docker-compose.yml up -d
```

## 3. When `up -d` fails with `KeyError: 'ContainerConfig'`

This happens when an older `docker-compose` binary cannot read metadata from an image built by a newer Docker daemon. Do not keep retrying `up -d`.

Workaround: stop + remove the affected container explicitly, then recreate it:

```bash
# v1
docker-compose -f deploy/docker-compose.yml stop  <service>
docker-compose -f deploy/docker-compose.yml rm -f <service>
docker-compose -f deploy/docker-compose.yml up -d <service>

# v2
docker compose -f deploy/docker-compose.yml stop  <service>
docker compose -f deploy/docker-compose.yml rm -f <service>
docker compose -f deploy/docker-compose.yml up -d <service>
```

## 4. Verify the deployment

```bash
# Container state
docker-compose -f deploy/docker-compose.yml ps

# Tail recent logs
docker-compose -f deploy/docker-compose.yml logs --tail 30 <service>

# HTTP health check (adjust path/port)
curl -s http://localhost:9080/api/status
```

## 5. Pitfalls

- **Forgetting `--build`**: `up -d` alone may reuse the old image.
- **Volume/container mismatch**: removing the container is safe; removing named volumes (`-v`) is usually not.
- **Healthcheck timing**: wait for the container state to become `healthy` before trusting it.
- **Named volume out of sync with host `models/`**: Compose may mount a named
  volume (e.g. `models:/models:ro`) rather than the host directory. Updating
  `./models/` on the host does NOT update the running container. See §6.
- **Healthcheck on an auth-gated route reports `unhealthy` forever.** If a
  service's healthcheck targets a path behind `basic_auth`/auth middleware
  (e.g. Caddy `wget --spider http://127.0.0.1:80/` on a route requiring auth),
  the probe gets a 401 and the container is marked `unhealthy` permanently
  *even while it proxies fine*. This is a **false negative** — read `docker
  logs` for the real error (`502 connection refused` = upstream was briefly
  down at that timestamp) rather than trusting the health state. Not a deploy
  blocker; exempt the healthcheck path from auth or pass creds.
- **Recreating a bare `docker run` container**: when a stack is deployed via
  raw `docker run` (no `com.docker.compose.*` labels), reproduce it
  byte-identically by inspecting the *live* container, not by trusting
  compose/Dockerfile:

  ```bash
  docker inspect <name> \
    --format '{{json .Config.Env}} {{json .Config.Entrypoint}} \
  {{json .HostConfig.RestartPolicy}} {{json .HostConfig.ExtraHosts}} \
  {{json .HostConfig.PortBindings}} {{json .Mounts}} {{json .NetworkSettings.Networks}}'
  ```

  The actual container frequently diverges from compose (e.g. no published
  `-p` ports because an external proxy fronts it, no `--user` override).
  Run with `--env-file` + the inspected volumes/network/extra_hosts/restart to
  keep the recreate faithful.
- **Disk exhaustion during build**: accumulated Docker images can fill the disk
  and cause "no space left on device" mid-build. Run `docker image prune -f`
  first; in one session this reclaimed 15.58 GB.

## 6. Syncing model artifacts in a named volume

When the compose file uses a named volume for model artifacts:

```yaml
services:
  inference:
    volumes:
      - models:/models:ro
```

Updating the host `./models/` directory is not enough. The container sees the
named volume's backing data, not the host path. To refresh models on a live VPS:

```bash
# 1. Stop the container that holds the volume.
docker-compose -f deploy/docker-compose.yml stop inference

# 2. Clear and repopulate the named volume's host mount point.
sudo bash -c 'rm -rf /var/lib/docker/volumes/deploy_models/_data/*'
sudo cp -r models/QQQ models/SMH models/XLF /var/lib/docker/volumes/deploy_models/_data/

# 3. Recreate the container from the rebuilt image.
docker-compose -f deploy/docker-compose.yml up -d --build inference
```

Verify the new models are visible inside the container:

```bash
docker exec mmn-inference find /models -maxdepth 2 -type f | sort
```

## 7. One-shot full redeploy command

```bash
COMPOSE_FILE=deploy/docker-compose.yml
docker image prune -f
docker-compose -f "$COMPOSE_FILE" build && docker-compose -f "$COMPOSE_FILE" up -d
```
