# SPA Debugging: ES Module Import Failures

When a single-page app shows all panels blank (`—`) but API calls work
(`curl /api/status` returns data), the cause is almost always a JavaScript
module import failure. The browser console will NOT show errors automatically
for failed dynamic `import()` calls — they only surface as unhandled
rejections or as broken DOM updates.

## Diagnosis: Find which module fails

Open the browser's JS console and run:

```js
// Test every view module individually (Promise.allSettled won't abort on first failure)
Promise.allSettled([
  import('/views/status.js'),
  import('/views/predictions.js'),
  import('/views/chart.js'),
  import('/views/accuracy.js'),
]).then(r => r.map((t, i) => ({
  view: ['status','predictions','chart','accuracy'][i],
  ok: t.status,
  err: t.status === 'rejected'
    ? (t.reason?.message || String(t.reason)).slice(0,120)
    : null
})))
```

The module that `status === 'rejected'` is the culprit. Common causes:

| Error message | Root cause | Fix |
|---|---|---|
| `Failed to fetch dynamically imported module` | File returns HTML fallback (404 or ServeDir fallback) | Check file ownership, see below |
| `Failed to parse URL` | Page in invalid/missing URL state | Reload the page |
| Module loads but `render` is undefined | Module doesn't export `render` | Check the view file |

## Diagnosis: Is the file actually being served?

From the browser console:

```js
fetch('/views/accuracy.js')
  .then(r => ({ ok: r.ok, status: r.status, ct: r.headers.get('content-type'), text: r.text() }))
  .then(r => r.text().then(t => ({ ...r, text: t.slice(0,100) })))
```

- HTTP 200 + `content-type: text/javascript` → file is served correctly
- HTTP 200 + `content-type: text/html` → file is falling through to the SPA fallback (ServeDir returned `index.html` for a non-existent path, or the server is returning HTML for the JS file)
- HTTP 404 → file doesn't exist in the container

From the server side (inside container):

```bash
# Check file exists and is readable by the runtime user
docker exec <container> ls -la /app/frontend/views/
# ownership must be <run_user> <run_group>, NOT root root

# Test the HTTP response directly
docker exec <container> curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:8080/views/accuracy.js
# must return 200; 404 = file missing; 301/302 = redirect (check URL)

docker exec <container> curl -sI http://127.0.0.1:8080/views/accuracy.js | grep -i content-type
# must be text/javascript; text/html = broken ServeDir
```

## Diagnosis: Ownership check

If `ls -la` shows `root root` for the frontend dirs and the container
runs as non-root (e.g. `USER mmn` in Dockerfile), the runtime user
can't traverse the directories. This causes `ServeDir` to fall through to
its fallback (the SPA's `index.html`), returning HTML for every static
file request including `.js` files.

Fix in Dockerfile:
```dockerfile
# Before (broken):
COPY --from=build /build/frontend/ /app/frontend/

# After (correct):
COPY --from=build --chown=mmn:mmn /build/frontend/ /app/frontend/
```

## Common failure modes

### 1. One failing module aborts the whole ES module graph
ES module imports are synchronous and one failure poisons the whole
graph. Even if only `views/accuracy.js` fails, the module that imports it
(typically `app.js`) will never execute its top-level code or start the
poll loop. Result: ALL panels stay at their HTML initial state (`—`).

### 2. HTML fallback hides the 404
When `ServeDir` can't find a file it returns `index.html` (the SPA
fallback), not a 404. The browser loads HTML as if it were JavaScript,
which causes a syntax error that also aborts the module graph.

### 3. Build cache preserves wrong ownership
`docker build --no-cache` does NOT fix ownership — it just bypasses
layer caching. The `--chown` flag on the COPY instruction is what fixes
it. Always add `--chown` AND verify the result with `docker exec ls -la`.

## Verification after fixing

After fixing ownership or adding `--chown`:
1. `docker build --no-cache -f engine/Dockerfile -t marketmarkovnet/engine:latest .`
2. `docker rm -f mmn-engine && docker-compose up -d`
3. `docker exec mmn-engine ls -la /app/frontend/views/` → confirms ownership
4. Browser: navigate to the app and verify panels show live data (not `—`)
