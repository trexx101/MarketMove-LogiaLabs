# Phase 0 — Foundation

**Goal**: Lay the groundwork for all subsequent phases without breaking anything.
After this phase, the backend has a clean API module tree, the new DB tables exist,
the Svelte frontend compiles and is served via rust-embed, and the old frontend still
works at `/legacy` as a fallback.

**Estimated effort**: 3–4 days
**Can deploy independently**: Yes — no user-visible change (old frontend still served)

---

## 0.1 Split `api.rs` into a module tree

### Current state
- Single `engine/src/api.rs` (~19K chars) with all routes, handlers, and `AppState` inline.
- `lib.rs` declares `pub mod api;` — the module is a flat file.

### Target state
```
engine/src/api/
  mod.rs          — Router assembly, AppState, re-exports
  status.rs       — GET /api/status, GET /api/market_state
  predictions.rs  — GET /api/predictions, GET /api/accuracy
  chart.rs        — GET /api/chart
  equity.rs       — GET /api/equity/data, /api/equity/backfill, /api/equity/macro, /api/equity/features
  static_serve.rs — rust-embed SPA fallback (replaces ServeDir)
```

### Steps
1. Create `engine/src/api/` directory.
2. Move `AppState` and `router()` into `api/mod.rs`. Make `AppState` fields `pub(crate)`.
3. Move each handler group into its corresponding sub-module.
4. Update `lib.rs`: `pub mod api;` stays the same (Rust resolves `api/mod.rs`).
5. Update `main.rs`: `mod api;` stays the same.
6. Run `cargo check` — must compile with zero behavior change.

### Key constraint
- All 9 existing routes must remain at the same paths with the same request/response schemas.
- The `fallback_service(ServeDir::new("frontend"))` stays for now — it will be replaced
  by `static_serve.rs` in step 0.4 below.

---

## 0.2 Add new crate dependencies

### `engine/Cargo.toml` additions
```toml
# Already in workspace deps:
# tokio-tungstenite = "0.24"   # WebSocket (Phase 1)
# tower-http = "0.6"            # already present

# NEW:
rust-embed = { version = "8", features = ["compression"] }
rhai = { version = "1.19", features = ["sync"] }
totp-rs = { version = "5", features = ["gen_secret", "qr"] }
mime_guess = "2"
```

### Verification
```bash
cd /home/ubuntu/projects/MarketMoves
cargo check
```

---

## 0.3 Create new DB tables

### DDL (append to `engine/src/db.rs` `DDL` constant)

```sql
CREATE TABLE IF NOT EXISTS strategy_configs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    strategy_type TEXT NOT NULL,   -- 'threshold' or 'rhai'
    script_body TEXT,
    params_json TEXT NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS mode_switches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    previous_mode TEXT NOT NULL,
    new_mode TEXT NOT NULL,
    parity_marker_age_secs INTEGER NOT NULL,
    authorized_by TEXT NOT NULL,
    timestamp INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS advisor_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    interaction_type TEXT NOT NULL,   -- 'briefing' or 'chat'
    prompt_context_json TEXT NOT NULL,
    model_used TEXT NOT NULL,
    response_json TEXT NOT NULL,
    suggested_action TEXT,
    timestamp INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS backtest_results (
    id TEXT PRIMARY KEY,
    strategy_id TEXT NOT NULL,
    start_ts INTEGER NOT NULL,
    end_ts INTEGER NOT NULL,
    metrics_json TEXT NOT NULL,
    equity_curve_json TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    FOREIGN KEY(strategy_id) REFERENCES strategy_configs(id) ON DELETE CASCADE
);
```

### Steps
1. Append the DDL to the existing `DDL` constant in `engine/src/db.rs`.
2. Add Rust structs + query functions for each table (following the existing pattern
   in `db.rs` — e.g. `StrategyConfigRow`, `insert_strategy_config()`, `fetch_strategy_configs()`).
3. Run the engine once to create the tables (the `open()` function applies DDL on startup).
4. Verify with `sqlite3 data/candles.db ".tables"`.

### Key constraint
- Do NOT modify the existing 5 equity tables or the 5 legacy crypto tables.
- Use `IF NOT EXISTS` so existing databases are not affected.

---

## 0.4 Scaffold Vite + Svelte frontend

### Target structure
```
frontend/
  package.json
  vite.config.js
  index.html              # Vite entry (replaces old index.html)
  src/
    main.js               # Mounts Svelte app
    App.svelte            # Root shell with sidebar navigation
    lib/
      api.js              # REST wrapper (replaces old api.js)
      stores.js           # Reactive global state (Svelte stores)
      websocket.js         # WebSocket connection manager (Phase 1)
      components/          # Shared UI components (ChartContainer, ParamInput, etc.)
    views/
      Dashboard.svelte     # Phase 1
      StrategyLab.svelte   # Phase 2
      Ledger.svelte        # Phase 1
      Advisor.svelte       # Phase 4
  dist/                    # Vite build output (gitignored, built at compile time)
  legacy/                  # Old vanilla JS frontend moved here
    index.html
    app.js
    api.js
    style.css
    views/
    vendor/
```

### Steps
1. `mv frontend/index.html frontend/app.js frontend/api.js frontend/style.css frontend/views frontend/vendor frontend/legacy/`
   — move all old frontend files into `frontend/legacy/`.
2. Create `frontend/package.json`:
   ```json
   {
     "name": "marketmoves-control-room",
     "private": true,
     "type": "module",
     "scripts": {
       "dev": "vite",
       "build": "vite build",
       "preview": "vite preview"
     },
     "devDependencies": {
       "@sveltejs/vite-plugin-svelte": "^3",
       "svelte": "^4",
       "vite": "^5"
     }
   }
   ```
3. Create `frontend/vite.config.js`:
   ```js
   import { defineConfig } from 'vite';
   import { svelte } from '@sveltejs/vite-plugin-svelte';

   export default defineConfig({
     plugins: [svelte()],
     build: {
       outDir: 'dist',
       emptyOutDir: true,
     },
     server: {
       proxy: {
         '/api': 'http://localhost:3000',
       },
     },
   });
   ```
4. Create minimal `frontend/index.html`, `frontend/src/main.js`, `frontend/src/App.svelte`
   with a placeholder "Control Room — Phase 0 scaffold" message.
5. Create `frontend/src/lib/api.js` — port the 4 existing fetch functions from the old
   `api.js`, keeping the same endpoint paths.
6. `cd frontend && npm install && npm run build` — verify `dist/` is produced.

### Key constraint
- The old frontend must remain accessible at `/legacy/` during development.
- `npm run build` must succeed before proceeding to Phase 1.

---

## 0.5 Wire `rust-embed` into Axum

### New file: `engine/src/api/static_serve.rs`

```rust
use rust_embed::RustEmbed;
use axum::{
    response::{Html, IntoResponse, Response},
    http::{StatusCode, header, Uri},
};

#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct Assets;

pub async fn spa_fallback_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Serve exact asset (CSS, JS, images)
    if let Some(content) = Assets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return (
            [(header::CONTENT_TYPE, mime.as_ref())],
            content.data,
        ).into_response();
    }

    // Fallback to index.html for client-side routing
    if let Some(index) = Assets::get("index.html") {
        return Html(index.data).into_response();
    }

    (StatusCode::NOT_FOUND, "404 Not Found").into_response()
}
```

### Modify `engine/src/api/mod.rs`
- Add `mod static_serve;`
- Replace the `fallback_service(ServeDir::new("frontend"))` with:
  ```rust
  .fallback(static_serve::spa_fallback_handler)
  ```
- Add a route for the legacy frontend:
  ```rust
  .nest_service("/legacy", ServeDir::new("frontend/legacy"))
  ```

### Build integration
- Add a `build.rs` or cargo build script step that runs `npm run build` in `frontend/`
  before compiling the Rust binary. Alternatively, document that `npm run build` must be
  run manually before `cargo build` (simpler for now — automate later).

### Verification
1. `cd frontend && npm run build`
2. `cargo build`
3. Run the engine, open `http://localhost:3000/` — should show the Svelte scaffold.
4. Open `http://localhost:3000/legacy/` — should show the old vanilla JS frontend.
5. All 9 existing API endpoints must still work.

---

## 0.6 Test requirements

- `cargo test` — all existing tests pass (no behavior change).
- `cargo check` — no warnings from the new module structure.
- `npm run build` — Svelte frontend compiles.
- Manual: both `/` (Svelte) and `/legacy/` (old) load in browser.
- Manual: all 9 API endpoints return the same responses as before.

---

## 0.7 Risk notes

- **rust-embed path**: The `#[folder = "frontend/dist/"]` attribute requires `dist/` to
  exist at compile time. If `npm run build` hasn't run, compilation fails. Mitigation:
  add a check in `build.rs` or document the build order.
- **ServeDir for /legacy**: The `tower-http` `ServeDir` is already a dependency. The legacy
  nest should work without new deps.
- **Module split regression**: Moving 19K chars of code into sub-modules risks subtle
  import errors. Mitigation: do the split in one commit, run `cargo check` immediately,
  fix any visibility issues.
