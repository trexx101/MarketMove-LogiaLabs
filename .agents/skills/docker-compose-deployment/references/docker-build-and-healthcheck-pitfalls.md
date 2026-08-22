# Docker Build & Healthcheck Pitfalls

## 1. Healthcheck `start_period` must exceed synchronous startup

### Symptom
Container shows `health: starting` forever, restart count climbing.
Healthcheck logs show exit=1 with empty output. The service never becomes
healthy because Docker kills and restarts it before startup completes.

### Root cause
Many services have synchronous startup sequences (data backfill, cache
seeding, model loading) that block the main thread before the HTTP server
binds. If `start_period + (retries × interval)` < actual startup time,
Docker considers the container unhealthy and the orchestrator restarts it.
The restart resets the clock, so it never converges.

### Fix
Calculate startup time from logs (`docker logs <container> 2>&1 | grep
"<startup complete marker>"`), then set:
```yaml
healthcheck:
  start_period: <startup_time_seconds + 30s buffer>
  retries: 5    # generous; don't make it tight
```

### Diagnostic
```bash
docker inspect <container> --format '{{json .State.Health.Log}}' | python3 -c \
  "import sys,json; logs=json.load(sys.stdin); [print(f'{l[\"Start\"]} exit={l[\"ExitCode\"]} out={l[\"Output\"][:100]}') for l in logs[-5:]]"
```
If all recent entries show `exit=1` with empty output, the service hasn't
started listening yet. Check if the startup-complete log line ever appears:
```bash
docker logs <container> 2>&1 | grep -c "<startup marker>"
```

## 2. Dockerfile warmup layer must stub ALL `[[bin]]` targets

### Symptom
Docker build fails at the warmup/dependency-caching layer with:
```
error: failed to find manifest
```
or
```
error: no cargo target matching `--bin <name>`
```

### Root cause
Cargo.toml declares a `[[bin]]` target with `path = "src/bin/my_binary.rs"`.
The warmup layer creates stub `src/main.rs` and `src/lib.rs` to pre-compile
dependencies, but doesn't create a stub for `src/bin/my_binary.rs`. Cargo
resolves the full target graph, finds the missing file, and fails.

### Fix
Create a stub for every `[[bin]]` target in the warmup layer:
```dockerfile
RUN mkdir -p engine/src/bin \
    && echo 'fn main() { println!("deps-warmup"); }' > engine/src/main.rs \
    && echo '' > engine/src/lib.rs \
    && echo 'fn main() {}' > engine/src/bin/my_binary.rs \
    && cargo build --release --manifest-path engine/Cargo.toml \
    && rm -rf engine/src
```

## 3. Checking if a container's process is listening (no `ss`/`netstat`)

When a container image doesn't include `ss`, `netstat`, or `lsof`, you can
check listening ports via `/proc/net/tcp`:

```bash
# Inside the container
cat /proc/net/tcp | grep ' 0A '
# 0A = LISTEN state in hex
# The port is the hex value after the colon in the local_address field
```

Decode the hex port:
```python
port = int("880B", 16)  # → 34827
```

This is useful when `curl` fails with "Connection refused" but the process
is running — it tells you whether the process is listening on a different
port than expected (e.g., env var not set correctly) or not listening at
all (still in startup).
