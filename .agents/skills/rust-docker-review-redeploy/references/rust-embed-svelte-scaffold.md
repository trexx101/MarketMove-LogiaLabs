# rust-embed + Svelte/Vite Scaffold for Rust Backends

Pattern for embedding a compiled Svelte/Vite frontend directly into a
Rust binary, replacing `ServeDir` (filesystem serving) with
`rust-embed` (compiled-in assets). This eliminates the runtime
dependency on a `frontend/` directory existing on the filesystem and
avoids the file-ownership class of bugs that plague Docker deployments
with `ServeDir`.

## When to use

- Rust/Axum backend serving a SPA frontend
- Want a single binary with no external static-file dependency
- Migrating from `ServeDir` to embedded assets
- Scaffolding a new Svelte/Vite frontend to replace a legacy vanilla JS SPA

## Workspace setup

### 1. Add crate dependencies

In the workspace `Cargo.toml`:
```toml
[workspace.dependencies]
rust-embed = { version = "8", features = ["compression"] }
mime_guess = "2"
```

In the engine crate's `Cargo.toml`:
```toml
rust-embed = { workspace = true }
mime_guess = { workspace = true }
```

### 2. Scaffold the Svelte/Vite frontend

Move the old frontend to `legacy/`:
```bash
cd frontend && mkdir -p legacy && mv api.js app.js index.html style.css views vendor legacy/
```

Create `frontend/package.json`:
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

Create `frontend/vite.config.js`:
```javascript
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  build: { outDir: 'dist', emptyOutDir: true },
  server: { proxy: { '/api': 'http://localhost:3000' } },
});
```

Create `frontend/svelte.config.js`:
```javascript
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';
export default { preprocess: vitePreprocess() };
```

Create `frontend/index.html` (Vite entry point):
```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>MarketMoves Control Room</title>
</head>
<body>
  <div id="app"></div>
  <script type="module" src="/src/main.js"></script>
</body>
</html>
```

Create `frontend/src/main.js`:
```javascript
import App from './App.svelte';
const app = new App({ target: document.getElementById('app') });
export default app;
```

Create `frontend/.gitignore`:
```
node_modules/
dist/
```

### 3. Build the frontend

```bash
cd frontend && npm install && npm run build
```

This produces `frontend/dist/` with `index.html` + hashed assets.

### 4. Create the rust-embed static serve module

`engine/src/api/static_serve.rs`:
```rust
use axum::{
    http::{header, Uri},
    response::{Html, IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct Assets;

/// Fallback handler for SPA routes — serves index.html for any
/// non-API, non-asset path so client-side routing works.
pub async fn spa_fallback(Uri(uri): Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    // Try the exact path first (e.g. /assets/index-abc123.js)
    if let Some(file) = Assets::get(path) {
        return serve_embedded(path, file);
    }
    // Fallback to index.html for SPA routing
    if let Some(file) = Assets::get("index.html") {
        return serve_embedded("index.html", file);
    }
    (axum::http::StatusCode::NOT_FOUND, "Not Found").into_response()
}

fn serve_embedded(
    path: &str,
    file: rust_embed::EmbeddedFile,
) -> Response {
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();
    (
        [(header::CONTENT_TYPE, mime.as_str())],
        file.data.into_owned(),
    )
        .into_response()
}
```

### 5. Wire into the router

In `api/mod.rs`, replace `ServeDir` fallback with the embedded handler:

```rust
// Before (ServeDir — requires filesystem at runtime):
.fallback_service(
    ServeDir::new("frontend")
        .not_found_service(ServeFile::new("frontend/index.html")),
)

// After (rust-embed — assets compiled into binary):
.route_fallback(static_serve::spa_fallback)
```

### 6. Add build.rs for frontend compilation

`engine/build.rs`:
```rust
use std::process::Command;

fn main() {
    // Only build frontend for release builds (not cargo test)
    if std::env::var("PROFILE").as_deref() == Ok("release") {
        let frontend_dir = std::path::Path::new("../frontend");
        if !frontend_dir.join("dist").exists() {
            if Command::new("npm")
                .arg("install")
                .current_dir(frontend_dir)
                .status()
                .is_ok()
            {
                let _ = Command::new("npm")
                    .arg("run")
                    .arg("build")
                    .current_dir(frontend_dir)
                    .status();
            }
        }
    }
    // rust-embed needs dist/ to exist at compile time
    let dist = std::path::Path::new("../frontend/dist");
    if !dist.exists() {
        std::fs::create_dir_all(dist).ok();
        std::fs::write(dist.join("index.html"),
            "<!DOCTYPE html><html><body>Frontend not built. Run: cd frontend && npm install && npm run build</body></html>"
        ).ok();
    }
    println!("cargo:rerun-if-changed=../frontend/dist");
}
```

## Verification

```bash
# Frontend builds
cd frontend && npm run build

# Rust compiles
cargo check --lib

# Tests pass (ServeDir tests need updating — see below)
cargo test --lib api::tests
```

## Test update for SPA fallback

Tests that asserted on `ServeDir` behavior need updating. The SPA
fallback now serves from embedded assets, not the filesystem. Update
title assertions to be flexible:

```rust
// Before:
assert!(body.contains("MarketMarkovNet"));

// After:
assert!(
    body.contains("MarketMoves") || body.contains("MarketMarkovNet"),
    "index.html should contain app title"
);
```

## Dev workflow

During development, run Vite's dev server separately for hot-reload:
```bash
cd frontend && npm run dev  # Vite dev server on :5173, proxies /api to :3000
cargo run -p engine          # Rust API on :3000
```

For production builds, the frontend is compiled into the Rust binary —
no separate static-file serving needed.
