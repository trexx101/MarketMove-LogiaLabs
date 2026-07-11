# Feature 03 — Kraken Credentials & Config Management

**Depends on:** 01
**Goal:** Provide secure credential handling and a unified config loader for both the Rust and Python components.

## Requirements

- `.env.example` documenting every env var from `REQUIREMENTS.md`.
- Checklist for generating restricted Kraken keys (Query + Trade only; Withdraw disabled).
- Rust config loader (typed struct) and Python config loader reading the same env vars.
- Secrets never committed; loaded via env / compose secrets.

## Technical Implementation Steps

1. Write `.env.example` and `deploy/KRAKEN_KEYS.md` with the exact Kraken UI permission checklist.
2. Rust: a `config` module deserializing env into a `Config` struct (`envy` or manual `std::env`), with defaults for `TRADING_MODE=paper`, `MAGNITUDE_THRESHOLD=0.005`, `PAPER_FEE=0.0015`, `SMA_WINDOW=200`.
3. Python: `inference/config.py` reading `ZMQ_ENDPOINT`, model paths.
4. Fail-fast validation: missing live keys in `live` mode aborts startup with a clear error.

## Acceptance Criteria

- [ ] `cargo build` passes; config loads defaults when env unset.
- [ ] Python config imports and resolves model paths.
- [ ] `.env` is gitignored; only `.env.example` is tracked.
- [ ] Starting in `live` mode without keys exits with a descriptive error.
