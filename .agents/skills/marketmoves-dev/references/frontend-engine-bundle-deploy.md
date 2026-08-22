# Frontend → Engine Image Deploy Loop (2026-08-16)

The Svelte SPA is bundled into the engine Docker image at build time
(there is no separate frontend container in prod). Any UI change —
text, layout, field name, store wiring — requires this sequence:

## Sequence

```bash
# 1. Build the SPA on the host
cd /home/ubuntu/projects/MarketMoves/frontend
npm run build                                  # → dist/index-*.js (hashed)

# 2. Rebuild the engine image (copies dist/ into the image)
cd /home/ubuntu/projects/MarketMoves
docker build -f engine/Dockerfile -t marketmarkovnet/engine:latest .

# 3. Restart engine — preserves sibling service names on compose v1.29.2
docker-compose -f deploy/docker-compose.yml stop engine \
  && docker-compose -f deploy/docker-compose.yml rm -f engine \
  && docker-compose -f deploy/docker-compose.yml up -d --no-deps engine

# 4. Wait for health
sleep 25 && docker inspect mmn-engine --format '{{.State.Health.Status}}'

# 5. Verify the served bundle hash matches what you built
NEW=$(ls /home/ubuntu/projects/MarketMoves/frontend/dist/assets/index-*.js | grep -oE 'index-[A-Za-z0-9_-]+\.js')
SERVED=$(curl -sS http://localhost:9080/ | grep -oE 'index-[A-Za-z0-9_-]+\.js')
[ "$NEW" = "$SERVED" ] && echo "OK" || echo "MISMATCH — engine serving old bundle"

# 6. Spot-check that the served JS contains the changes you just made
curl -sS "http://localhost:9080/$SERVED" | grep -o "<needle>"
```

If step 5 shows MISMATCH, the engine image wasn't rebuilt or the proxy
is caching. Rebuild + restart. Do NOT trust "200 OK" alone.

If step 6 returns no match, the change didn't make it into the bundle
— recheck the source file, rebuild, redeploy.

## Why the SPA lives in the engine image

`engine/Dockerfile` has a multi-stage build with a `frontend-build`
stage that `COPY`s `frontend/dist/` from the host context. The engine's
axum router serves `/app/frontend/` (the embedded SPA) at `/`. There is
no nginx, no Caddy static-serve, no separate frontend container.

This is intentional simplicity: one container to deploy, one healthcheck
to monitor, one image to roll back. The tradeoff is the deploy-loop
discipline above.

## Local dev vs prod are NOT equivalent

| Concern | Local dev | Prod (this VPS) |
|---|---|---|
| Edit frontend | HMR via `npm run dev` → vite at :5173 | Edit files, run `npm run build` |
| See the change | Auto-reload in browser | Rebuild engine image + restart |
| Backend + frontend change | Either side independently | One commit, one rebuild, one deploy |
| Field rename in Rust response | No HMR — restart engine manually | Same — plus rebuild + restart |

`npm run dev` running on the host is a **development aid only**. It is
not part of the prod stack. Changes visible at `localhost:5173` are
invisible at `localhost:9080` until the engine image is rebuilt.

## Field-name rename — one-commit rule

When you rename a field on a serialized Rust response struct:

1. **Before commit:** `grep -rn '<old_field_name>' frontend/src` must
   return zero hits. If it returns hits, the Svelte consumer still
   reads the old name — update it in the same commit.
2. **Before commit:** the Svelte change must be in `frontend/dist/` —
   re-run `npm run build` and verify the bundle references the new
   field name with `grep`.
3. **Before commit:** the engine Dockerfile ENV block is unchanged
   (no field name there), but if a config field is renamed, the
   `AppState` literal, the relevant test file's `Config { }` literal,
   and the `StrategyConfigResponse`/`StrategyConfigUpdate` structs
   all need updating in the same commit (see "Adding a new strategy
   param" pitfall in SKILL.md).
4. **After deploy:** verify served bundle matches built bundle AND
   verify the live JSON response contains the new field name.

Half-deploys (Rust shipped, Svelte not; or Svelte rebuilt but engine
not restarted) leave the UI silently reading fields the API no longer
emits. Symptom: every metric shows `N/A` despite the API returning
valid JSON.

## Common failure modes (2026-08-16)

| Symptom | Cause | Fix |
|---|---|---|
| UI shows `N/A` for everything after rename | Svelte reads old field name; API renamed it | Grep frontend for old name, update consumer, rebuild |
| UI shows old data after restart | Browser cache or proxy cache | Hard refresh (Ctrl+Shift+R) and check served bundle hash |
| `npm run build` succeeds but UI doesn't update | Engine image not rebuilt | `docker build -f engine/Dockerfile …` then restart engine |
| Dashboard works at `localhost:5173` but not `:9080` | You're looking at the vite dev server, not the prod bundle | The dev server is irrelevant to prod; rebuild + restart |
| API works via `docker exec curl …` but UI shows 502 | Caddy proxy is still pointing at the old engine container's IP | Transient (~30s); wait for engine healthcheck then hard-refresh |

## See also

- `SKILL.md` → "Docker Build & Deploy" — the canonical rebuild sequence
  and the `ContainerConfig KeyError` pitfall
- `SKILL.md` → "Renaming API response fields breaks the dashboard
  silently" — the field-rename pitfall with grep self-test
- `SKILL.md` → "Verify new frontend bundle is actually being served"
  — the bundle-hash verification snippet