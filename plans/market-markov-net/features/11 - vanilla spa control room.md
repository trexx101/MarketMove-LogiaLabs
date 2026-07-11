# Feature 11 — Vanilla SPA Control Room

**Depends on:** 10
**Goal:** A minimal, extensible single-page app for live monitoring, served by Axum.

## Requirements

- No build toolchain: plain `index.html` + vanilla JS/CSS + a lightweight chart lib (uPlot or Chart.js via CDN/vendored).
- Panels: live OHLCV chart with 200-SMA overlay, current position + PnL, latest predictions (1H/4H/24H), and a clear **paper/live** mode badge.
- Polls `/api/status`, `/api/predictions`, `/api/chart` on an interval.
- Structured so new views/pages can be added later (simple router/module pattern).

## Technical Implementation Steps

1. `frontend/index.html` + `frontend/app.js` + `frontend/style.css`.
2. Fetch/poll helpers; render chart + panels; mode badge styling (paper=blue, live=red).
3. Modularize API access and view components for extensibility.
4. Serve via Axum static file route (Feature 10).

## Acceptance Criteria

- [ ] SPA loads and renders live chart, position/PnL, and predictions.
- [ ] Mode badge correctly reflects `TRADING_MODE`.
- [ ] Adding a new view requires only a new module (documented pattern).
- [ ] No bundler/build step required to run.
