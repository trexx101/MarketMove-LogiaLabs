# MarketMoves Frontend Deploy Verification (Vite SPA)

This project's engine serves a **compiled Vite SPA**, not per-view modules.
The serving model differs from the generic `/views/*.js` dynamic-import
pattern described in `spa-debugging.md` — use THIS file for MarketMoves.

## Serving model

- Host: `npm run build` (or `npx vite build`) → `frontend/dist/`
  (`index.html` + `assets/index-<HASH>.js` + `assets/index-<HASH>.css`)
- Dockerfile `COPY --from=frontend-build /build/frontend/ /app/frontend/`
  (the `frontend-build` stage has NO Node toolchain — it just COPYs the
  pre-built host `dist/`)
- Runtime: `ServeDir::new("frontend")` serves `/app/frontend/` at `/`
- `index.html` references `/assets/index-<HASH>.js` (single hashed bundle)

## Pre-deploy checklist

1. **Build the host bundle FIRST.** `cd frontend && npx vite build`
   (or `npm run build`). Without this, the docker build bakes a STALE bundle.
2. **Build image with `--no-cache`** so the `frontend-build` COPY layer is
   not reused from a prior build:
   `docker build --no-cache -f engine/Dockerfile -t marketmarkovnet/engine:latest .`

## Post-deploy verification

```bash
# 1. Hash the container is serving NOW
docker exec mmn-engine curl -s http://127.0.0.1:8080/ | grep -o '/assets/index-[^"]*\.js'

# 2. Hash your last host build produced
ls -1 frontend/dist/assets/ | grep '\.js$'

# 3. Must match. Mismatch = cached layer, rebuild --no-cache + redeploy.

# 4. Asset content-type must be JS, not HTML fallback
docker exec mmn-engine curl -sI http://127.0.0.1:8080/assets/index-<HASH>.js \
  | grep -i 'content-type\|HTTP'
```

## Browser check

Hard-refresh (Cmd+Shift+R) to bust cached `index.html`. Navigate to new nav
items (OPTIONS section, Events tab under SYSTEM) and confirm they render.
A Svelte compile error would have failed `vite build` (look for
`✓ N modules transformed` with 0 errors) — if the build passed, the bundle
is safe to serve.

## Gotchas

- The `frontend-build` stage has no Node — host MUST build before docker build
- `docker build` without `--no-cache` may reuse a stale `dist/` COPY layer
- `--user 1000:1000` in `docker run` — do NOT add `--chown` to the COPY;
  the Dockerfile already does `COPY --chown=mmn:mmn`
- SPA panels going `—` after deploy = stale bundle or build skipped, NOT a
  runtime API failure (API endpoints return JSON fine via `curl`)
