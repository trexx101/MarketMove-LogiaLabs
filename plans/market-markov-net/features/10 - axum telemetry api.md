# Feature 10 — Axum Telemetry API

**Depends on:** 06, 08, 09
**Goal:** Expose the engine's live state over HTTP JSON endpoints for the control room.

## Requirements

- Lightweight Axum server on `HTTP_PORT` (internal; reverse-proxied on 80/443).
- Endpoints:
  - `GET /api/status` — mode (paper/live), current position, entry price, realized + unrealized PnL.
  - `GET /api/predictions` — latest neural outputs + recent history.
  - `GET /api/chart` — OHLCV + 200-SMA series for charting.
- Reads from the shared engine state / SQLite; non-blocking with the trading loop.

## Technical Implementation Steps

1. `engine/src/api.rs`: Axum router + handlers querying SQLite / shared state (`Arc`).
2. Define `serde` response DTOs for each endpoint.
3. Serve the SPA static assets (Feature 11) from the same server.
4. Add CORS/permissive headers as needed for local dev; keep bound to internal interface.

## Acceptance Criteria

- [ ] All three endpoints return valid JSON reflecting live engine state.
- [ ] Serving endpoints does not stall the trading/ingestion loop.
- [ ] `cargo build` + `cargo clippy` pass.
