# Docker Compose v1.29 `KeyError: 'ContainerConfig'` workaround

## Symptom

`docker-compose up -d` fails with:

```
File "/usr/lib/python3/dist-packages/compose/service.py", line 1579, in get_container_data_volumes
    container.image_config['ContainerConfig'].get('Volumes') or {}
KeyError: 'ContainerConfig'
```

This happens when `docker-compose` (v1.29.2) tries to inspect an existing container that was created by a newer Docker daemon that no longer includes the `ContainerConfig` field.

## Fix

Remove the affected container(s), then recreate:

```bash
cd /home/ubuntu/projects/MarketMoves/deploy
docker rm -f mmn-engine mmn-inference
docker-compose up -d
```

If `docker-compose` itself is the old Python wrapper, prefer the newer `docker compose` binary when available.

## Prevention

After building a new image, don't rely on `docker-compose up -d` to replace running containers against this daemon/version combo. Explicitly remove the old container first, then recreate.
