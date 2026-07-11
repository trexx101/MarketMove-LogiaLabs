# frontend/

Vanilla SPA control room for MarketMarkovNet. No build toolchain.

## Status

**Feature 11 placeholder.** The real SPA (uPlot/Chart.js integration, real
predictions/trades panels, error UX) is implemented in Feature 11
(see `../plans/market-markov-net/features/11 - vanilla spa control room.md`).

## Files

- `index.html` — minimal valid HTML5 page with header, chart placeholder, status
  panel, predictions panel, and a small inline `<script>` that polls
  `/api/status` every 5 s (will 404 until the engine is up — that is fine).
- `app.js` — empty placeholder.
- `style.css` — empty placeholder (most styling is currently inline in
  `index.html` for the stub; will be moved out in Feature 11).

## Serving

In production the SPA is served by the Axum telemetry service. For local
development just open `index.html` in a browser.
