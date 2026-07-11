# Feature 01 — Repo Scaffold & Workspace

**Depends on:** none
**Goal:** Establish the monorepo layout, toolchain configs, and shared conventions so all subsequent features have a stable foundation.

## Requirements

- Cargo workspace for the Rust execution core under `/engine`.
- Python project (uv-managed) for the inference service under `/inference`.
- Static frontend directory under `/frontend`.
- Deployment assets under `/deploy`, gitignored model dir `/models`, and tests under `/tests`.
- Root `.gitignore` excluding `.env`, `/models/*`, `*.db`, target/venv artifacts.
- Root `README.md` describing structure and how to run each part.

## Technical Implementation Steps

1. `cargo new --bin engine` (or a workspace `Cargo.toml` with a single member) and add core deps: `tokio`, `axum`, `serde`, `serde_json`, `sqlx`/`rusqlite`, `tokio-tungstenite`, `reqwest`, a zmq crate (`tmq` or `zmq`), `anyhow`, `tracing`.
2. `uv init inference` with `pyproject.toml`; add `torch` (CPU), `pyzmq`, `numpy`.
3. Create `/frontend/index.html` placeholder and `/deploy`, `/models` (with `.gitkeep`), `/tests` dirs.
4. Write root `.gitignore` and `README.md`.
5. Add a shared config schema doc (`deploy/config.md`) enumerating env vars from `REQUIREMENTS.md`.

## Acceptance Criteria

- [ ] `cargo build` succeeds in `/engine`.
- [ ] `uv sync` succeeds in `/inference`.
- [ ] `git status` shows `.env`, `/models/*`, `*.db` ignored.
- [ ] Directory tree matches the layout in `REQUIREMENTS.md`.
