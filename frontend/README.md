# frontend/

The Svelte 4 control room for MarketMoves. A single-page app the engine
serves at `/` (Axum's `ServeDir` fallback). Telemetry is pushed over
WebSocket (`/api/v1/ws`); the legacy 5s polling loop is retired.

## Status

Phase 3.4 (Phase 1 dashboard + Phase 3 mode toggle UI). The Svelte
build replaces the vanilla SPA that used to live in `index.html` /
`app.js` / `views/`. The legacy files are preserved in `legacy/` for
reference only — they're not built, not served, and not part of the
active code path.

## Build

The frontend is built with Vite + Svelte. The engine's Dockerfile runs
the build in a Node stage and serves the compiled `dist/` artifacts, so
no manual steps are needed at deploy time. See
[`engine/Dockerfile`](../engine/Dockerfile).

```bash
cd frontend
npm ci          # install pinned deps
npm run build   # output -> dist/
npm run dev     # vite dev server (proxies /api -> http://localhost:3000)
```

## Layout

```
frontend/
├── src/
│   ├── main.js                     # entry point
│   ├── App.svelte                  # widget layout + WS subscription
│   └── lib/
│       ├── api.js                  # fetchStatus / fetchMode / setMode / etc.
│       ├── websocket.js            # tokio-tungstenite-compatible client
│       ├── stores.js               # Svelte stores (status, predictions, ...)
│       └── components/             # 11 widgets, see below
├── dist/                           # vite build output (committed for run via engine Docker)
│   ├── index.html
│   └── assets/
│       ├── index-*.js
│       └── index-*.css
├── legacy/                         # pre-Svelte vanilla SPA (do not build)
├── index.html                      # dev shell — points Vite at /src/main.js
├── package.json
├── svelte.config.js
└── vite.config.js
```

## Components

| File | Widget |
|------|--------|
| `StatusPanel.svelte` | Mode badge (PAPER/LIVE), QQQ symbol, position badge, PnL, staleness, WS dot. Includes the **mode toggle modal** (Phase 3.4) — TOTP prompt + parity-status indicator. |
| `CandlestickChart.svelte` | OHLCV + 200-day SMA + prediction cones (1d/5d/21d). |
| `FeatureInspector.svelte` | 8-dim feature vector for the latest bar (median/MAD-normalized). |
| `PnLEquityCurve.svelte` | Running PnL over the trade history. |
| `TradeHistory.svelte` | Recent fills (exit/entry for QQQ + PSQ). |
| `ModelHealth.svelte` | Inference health, prediction freshness. |
| `MetricsTable.svelte` | Backtest metrics (Sharpe, max DD, hit rate, IC). |
| `EquityCurveChart.svelte` | Strategy Lab equity curve. |
| `ABComparison.svelte` | Side-by-side A/B comparison of two strategies. |
| `ParamSlider.svelte` | Parameter editor for the active strategy. |
| `RhaiEditor.svelte` | Monaco editor for Rhai strategy scripts. |

## WebSocket protocol

The engine broadcasts `TelemetryEvent` JSON objects on `/api/v1/ws`:

```json
{ "kind": "PredictionUpdate", "pred_1d": 0.0178, "pred_5d": 0.0146, "pred_21d": 0.0304, "timestamp": 1785159000 }
{ "kind": "TradeFill", "side": "buy", "symbol": "QQQ", "qty": 1.0, "price": 682.13, "fee": 1.02, "realized_pnl": 0.0, "timestamp": 1785159000 }
{ "kind": "PnlTick", "realized_pnl": 0.0, "unrealized_pnl": 0.0, "position": "flat", "entry_price": null, "last_close": 682.13, "timestamp": 1785159000 }
{ "kind": "ModeChange", "mode": "paper", "timestamp": 1718920000 }
```

The frontend auto-reconnects with exponential backoff on disconnect.

## Developing locally

The dev server proxies `/api` to the engine on port 8080. Start the
engine first (in another terminal), then:

```bash
cd frontend
npm run dev          # vite at http://localhost:5173
```

The Svelte build is hot-reloaded. The WebSocket connection is at
`ws://localhost:5173/api/v1/ws` (proxied to the engine).

## Build troubleshooting

If the engine's Docker build fails at the `npm ci` step, your local
`node_modules/` may have a lockfile drift. Run:

```bash
cd frontend
rm -rf node_modules
npm install
git diff package-lock.json   # if non-empty, commit the refreshed lockfile
```

Then rebuild the engine image — the Dockerfile will pick up the new
lockfile.

## See also

- [`../README.md`](../README.md) — top-level architecture.
- [`../deploy/README.md`](../deploy/README.md) — production deployment.
- [`.hermes/plans/control-room/PHASE_1_DASHBOARD_WEBSOCKET.md`](../.hermes/plans/control-room/PHASE_1_DASHBOARD_WEBSOCKET.md) — WebSocket protocol design.
- [`.hermes/plans/control-room/PHASE_3_EXECUTION_SHORTING.md`](../.hermes/plans/control-room/PHASE_3_EXECUTION_SHORTING.md) — mode toggle UI design.
