---
name: svelte-frontend-development
description: "Use when building Svelte 4 frontends with stores and WS."
version: 1.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [svelte, frontend, websocket, canvas, dashboard, vite, verification]
    related_skills: [frontend-dom-shim-verification, test-driven-development]
---

# Svelte Frontend Development

## Overview

Build Svelte 4 + Vite frontends with real-time data: component architecture,
Svelte stores fed by a WebSocket manager, canvas-based charts without external
charting libraries, and build-based verification when no test suite exists.

## When to Use

- Building a Svelte 4 dashboard or control-room frontend.
- Integrating a WebSocket feed into Svelte stores with auto-reconnect.
- Rendering charts on `<canvas>` without d3/chart.js/uplot/echarts.
- Verifying a Svelte frontend that has no test suite (build + structural checks).

**Don't use for:**
- Svelte 5 runes mode (`$state`/`$derived`/`$effect`) — this skill targets Svelte 4.
- React/Vue/vanilla JS frontends — use `frontend-dom-shim-verification` for
  vanilla JS view modules.
- Full E2E browser testing — use the `browser_*` tools or `dogfood` skill.

## Project Structure

```
frontend/
  src/
    main.js                    # mounts App
    App.svelte                 # shell: sidebar nav + view routing
    lib/
      stores.js                # writable stores (status, predictions, trades, etc.)
      websocket.js             # WS manager: connect, event dispatch, reconnect
      api.js                   # REST fetch wrappers
      components/
        StatusPanel.svelte     # reads from stores, displays mode/position/PnL
        CandlestickChart.svelte # canvas-based OHLC + SMA + prediction cones
        PnLEquityCurve.svelte  # canvas-based PnL line chart
        FeatureInspector.svelte # feature bars from store
        TradeHistory.svelte    # trade fill table from store
        ModelHealth.svelte     # WS connection + staleness indicators
    views/
      Dashboard.svelte         # grid layout, onMount REST + WS connect
      Ledger.svelte            # equity data table + curve
```

## Svelte Stores for Real-Time Data

All real-time state lives in writable stores. REST provides initial data; WS
events update stores incrementally. Components subscribe reactively — no manual
re-render calls.

```javascript
import { writable } from 'svelte/store';

export const wsConnected = writable(false);
export const status = writable(null);
export const trades = writable([]);  // arrays must init to []
```

**Key rule:** array stores (`trades`) must initialize to `[]`, not `null`.
Components that `{#each}` over them will throw on `null`.

## WebSocket Manager Pattern

A single `websocket.js` module owns the connection lifecycle and dispatches
parsed events to stores. Components never touch the WebSocket directly.

### Structure

```javascript
import { wsConnected, status, trades } from './stores.js';

let ws = null;
let backoff = 1000;
const MAX_BACKOFF = 30000;
let manuallyClosed = false;

export function connectWebSocket() { /* ... */ }
export function disconnectWebSocket() { manuallyClosed = true; /* close + cleanup */ }
```

### Event Dispatch

Each WS message has a `type` field. A `switch` dispatches to the appropriate
store update:

```javascript
function handleMessage(raw) {
  const msg = JSON.parse(raw.data);
  switch (msg.type) {
    case 'PnlTick':
      status.update(s => ({ ...s, realized_pnl: msg.realized_pnl, /* ... */ }));
      break;
    case 'TradeFill':
      trades.update(list => [{ ...msg }, ...list].slice(0, 50));
      break;
    // ...
  }
}
```

### Reconnection

Exponential backoff (1s → 30s cap), reset on successful open. Set
`manuallyClosed = true` before closing to suppress auto-reconnect.

```javascript
ws.onclose = () => {
  wsConnected.set(false);
  ws = null;
  if (!manuallyClosed) scheduleReconnect();
};
```

### Lifecycle in Views

The Dashboard view connects on `onMount` and disconnects on `onDestroy`:

```javascript
import { onMount, onDestroy } from 'svelte';
import { connectWebSocket, disconnectWebSocket } from '../lib/websocket.js';

onMount(() => { /* fetch REST data, then */ connectWebSocket(); });
onDestroy(() => { disconnectWebSocket(); });
```

### REST Polling Fallback for Fields Without WS Push

WS events are not always emitted for every field the frontend displays. A
common pattern: the backend defines a telemetry event variant in an enum,
writes a serialization test for it, and the frontend WS manager handles it —
but **no backend code actually broadcasts the event** (a "phantom event").
The frontend field then freezes at its initial REST load value and never
updates.

**Fix:** add a periodic REST poll of the status endpoint as a safety net.
WS remains for low-latency events (PnlTick, TradeFill, ModeChange); the poll
catches everything else. A 30s interval is sufficient for a daily-bar system;

for faster cadences, poll every 5–10s.

```javascript
let statusInterval;

onMount(async () => {
  // Initial REST load
  const s = await fetchStatus();
  status.set(s);
  connectWebSocket();

  // Safety-net poll for fields not pushed via WS
  statusInterval = setInterval(async () => {
    try {
      const s = await fetchStatus();
      status.set(s);
    } catch (e) { /* silent — WS may still be delivering */ }
  }, 30000);
});

onDestroy(() => {
  disconnectWebSocket();
  if (statusInterval) clearInterval(statusInterval);
});
```

**When adding a new WS event type:** verify that backend code actually
broadcasts it (grep for `tx.send(TelemetryEvent::YourVariant`) — not just
that the variant is defined and handled in the frontend. A variant that
exists in the enum but has no `tx.send()` call site is a phantom event.

## Canvas-Based Charting (No External Libraries)

For dashboards that need lightweight charts without adding d3/chart.js as
dependencies, use the Canvas 2D API directly. Two patterns recur: candlestick
and line/area charts.

### DPR Scaling + ResizeObserver

Every canvas chart needs this boilerplate for crisp rendering and responsive
resize:

```javascript
let canvas, container;
let resizeObserver;

onMount(() => {
  draw();
  resizeObserver = new ResizeObserver(() => draw());
  if (container) resizeObserver.observe(container);
});
onDestroy(() => { if (resizeObserver) resizeObserver.disconnect(); });

function draw() {
  const ctx = canvas.getContext('2d');
  const dpr = window.devicePixelRatio || 1;
  const w = container.clientWidth;
  const h = container.clientHeight || 300;
  canvas.width = w * dpr;
  canvas.height = h * dpr;
  canvas.style.width = w + 'px';
  canvas.style.height = h + 'px';
  ctx.scale(dpr, dpr);
  // ... draw using w/h (CSS pixels, not device pixels)
}
```

### Redraw on Store Changes

Svelte reactivity triggers redraw when a store updates:

```javascript
$: if ($chartData) {
  candles = $chartData.candles || candles;
  draw();
}
```

### Candlestick Drawing

For each candle: draw a vertical wick line (high→low), then a filled rectangle
body (open→close). Green for close ≥ open, red otherwise. Width per candle:
`xStep = chartWidth / candleCount`, body width: `max(1, xStep * 0.7)`.

**Canvas color/font gotcha:** `ctx.fillStyle`, `ctx.strokeStyle`, and
`ctx.font` do NOT resolve CSS variables. Use hardcoded hex values from
the design system directly in canvas code (e.g. `'#7132f5'` for accent).
For fonts, use the raw family name: `ctx.font = '10px monospace'` —
NOT `'10px var(--font-mono)'`, which silently falls back to default.

### Prediction Cones

Overlay dashed lines from the last candle close to projected prices at 1D/5D/21D
horizons. Color by direction (green for positive prediction, red for negative).

### Chart Enhancements (Forward-Look Buffer, Volume, Timeframe, Crosshair)

Four reusable patterns for extending a candlestick chart without breaking it.

#### Forward-Look Buffer for Overlays

When the chart draws anything that extends beyond the last data point
(prediction cones, forecast bands, projected event markers), dividing the canvas
width by the data length will silently clip those overlays off the right edge.

**Fix:** allocate extra slots in the X-scale that don't correspond to data:

```javascript
const n = candles.length;
const futureBars = 21; // longest horizon drawn (e.g. 21D cone)
const xSlots = n + futureBars;
const xStep = cw / xSlots;
const candleLeft = padL;
const candleRight = padL + (n - 1) * xStep + xStep;     // last candle's right edge
const futureRight = padL + xSlots * xStep;             // canvas right edge
```

Now data candles render in `candleLeft..candleRight`, and the future band has
`futureBars * xStep` of room past the last candle for overlays. Update
price-range calculation (`minP`/`maxP`) to include target prices so the
Y-axis covers the projection area.

The canvas right margin (`padR`) must also be wide enough for the right-edge
live-price pill — `~70px` is enough for the pill plus its text.

#### Volume Subplot

Allocate the chart's vertical space proportionally. Recompute `volH` every
draw, not once at mount, so the volume band scales with parent height:

```javascript
const padT = 36, padB = 28;
const priceH = (h - padT - padB) * 0.72;
const volH   = (h - padT - padB) * 0.22;
const gap    = (h - padT - padB) * 0.06;
const priceTop = padT;
const priceBot = priceTop + priceH;
const volTop = priceBot + gap;
const volBot = volTop + volH;
```

Each bar uses a local max-volume scale, not the price scale:

```javascript
let maxVol = 0;
for (const c of candles) if (c.volume > maxVol) maxVol = c.volume;
if (maxVol > 0) {
  const volScale = (v) => volBot - (v / maxVol) * volH;
  for (let i = 0; i < n; i++) {
    const x = padL + i * xStep + xStep / 2;
    const isUp = candles[i].close >= candles[i].open;
    ctx.fillStyle = isUp ? 'rgba(20, 158, 97, 0.55)' : 'rgba(229, 72, 77, 0.55)';
    const yT = volScale(candles[i].volume);
    ctx.fillRect(x - barW / 2, yT, barW, volBot - yT);
  }
}
```

Gate the entire block on `maxVol > 0` — empty candle arrays produce NaN
positions. Use ~55% opacity on the bars so price candles (drawn above) remain
visually dominant.

#### Timeframe Selector

A small array of `{label, limit}` objects feeds both the UI buttons and the
fetchChart param. Keep limits aligned with the backend's allowed range
(typical: 10–1500 candles).

```javascript
const TIMEFRAMES = [
  { label: '1M',  limit: 21   },   // ~21 trading days/month
  { label: '3M',  limit: 63   },
  { label: '6M',  limit: 126  },
  { label: '1Y',  limit: 252  },   // ~252 trading days/year
  { label: 'ALL', limit: 1500 },
];
let activeTimeframe = '6M';

async function refreshChart() {
  const tf = TIMEFRAMES.find(t => t.label === activeTimeframe);
  const data = await fetchChart(tf.limit);
  // ...
}

function setTimeframe(label) {
  if (label === activeTimeframe) return;
  activeTimeframe = label;
  refreshChart(); // re-fetch with the new limit, not just redraw
}
```

Buttons render in the chart card's top-right (z-index 2 above the canvas):

```svelte
<div class="tf-bar">
  {#each TIMEFRAMES as tf}
    <button class="tf-btn" class:active={activeTimeframe === tf.label}
            on:click={() => setTimeframe(tf.label)}>
      {tf.label}
    </button>
  {/each}
</div>
```

Re-fetching on timeframe change (rather than client-side filtering) is
correct: the backend can include the right stale-flag and live_quote for
the new range, and SMA is computed server-side at the right window.

#### Crosshair + OHLCV Tooltip

A two-step pattern: store hover state, recompute it on mousemove, draw the
overlay on top of everything else. The tooltip is a DOM element (not canvas
text) so it inherits CSS variables and stays crisp at any DPR.

```javascript
let hover = null; // { x, y, candle, i }

function onMouseMove(ev) {
  if (!canvas || !candles.length) return;
  const rect = canvas.getBoundingClientRect();
  const x = ev.clientX - rect.left;
  const y = ev.clientY - rect.top;

  const i = Math.floor((x - padL) / xStep);
  if (i < 0 || i >= n) {
    hover = null;
    draw();
    return;
  }
  hover = { x: padL + i * xStep + xStep / 2, y, candle: candles[i], i };
  draw();
}

function onMouseLeave() {
  if (hover) { hover = null; draw(); }
}
```

Draw the crosshair lines at the END of `draw()`, after everything else, so
they sit on top:

```javascript
if (hover) {
  ctx.save();
  ctx.strokeStyle = 'rgba(113, 50, 245, 0.4)'; // accent at 40% opacity
  ctx.setLineDash([2, 2]);
  ctx.beginPath();
  ctx.moveTo(hover.x, priceTop); ctx.lineTo(hover.x, volBot);
  ctx.moveTo(padL, hover.y);     ctx.lineTo(futureRight, hover.y);
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.restore();
}
```

Tooltip is rendered conditionally based on `hover`:

```svelte
<canvas bind:this={canvas}
        on:mousemove={onMouseMove}
        on:mouseleave={onMouseLeave}></canvas>

{#if hover && hover.candle}
  <div class="tooltip" style="left: {hover.x + 12}px; top: {hover.y - 12}px;">
    <div class="tt-date">{new Date(hover.candle.ts).toLocaleDateString()}</div>
    <div class="tt-row"><span>O</span> {hover.candle.open.toFixed(2)}</div>
    <div class="tt-row"><span>H</span> {hover.candle.high.toFixed(2)}</div>
    <div class="tt-row"><span>L</span> {hover.candle.low.toFixed(2)}</div>
    <div class="tt-row"><span>C</span> {hover.candle.close.toFixed(2)}</div>
    <div class="tt-row vol"><span>V</span> {Math.round(hover.candle.volume).toLocaleString()}</div>
  </div>
{/if}
```

Add `cursor: crosshair;` to the canvas so the user gets a visual cue that
the chart is interactive.

**Field name mismatch trap:** The REST `/api/predictions` endpoint may return
differently-named fields than the WS `PredictionUpdate` event. Example:
REST returns `pred_24h` / `pred_5h_approx` / `pred_1h`, but the WS event
sends `pred_1d` / `pred_5d` / `pred_21d`. If the chart reads
`preds.pred_1d` directly, it gets `undefined` from REST initial load and
the cones silently disappear. **Fix:** use a fallback chain that handles
both shapes:

```javascript
preds = {
  pred_1d: p.pred_1d ?? p.pred_24h ?? p.pred_1h,
  pred_5d: p.pred_5d ?? p.pred_5h_approx,
  pred_21d: p.pred_21d,
};
```

Always `curl` the REST endpoint and compare field names to what the
component reads — this is a specific instance of pitfall #9.

## Svelte 4 A11y Warning Resolution

**This is the #1 friction point when building Svelte 4 UIs.** The compiler
emits a11y warnings for click handlers on non-interactive elements. The
resolution ladder:

### What Doesn't Work

1. `on:click` on `<li>` → warns: "visible, non-interactive elements with
   on:click must be accompanied by a keyboard event handler"
2. Add `role="button" tabindex="0"` + `on:keydown` → warns: "Non-interactive
   element `<li>` cannot have interactive role 'button'"

### What Works

Use actual `<button>` elements inside `<li>` tags:

```svelte
<ul>
  <li>
    <button class="nav-btn" class:active={currentView === 'dashboard'}
            on:click={() => nav('dashboard')}>
      Dashboard
    </button>
  </li>
</ul>
```

Style the button to fill the list item:

```css
.sidebar li { list-style: none; }
.nav-btn {
  width: 100%;
  text-align: left;
  padding: 0.7rem 1.2rem;
  background: none;
  border: none;
  border-left: 3px solid transparent;
  font-family: inherit;
  font-size: 0.95rem;
  cursor: pointer;
}
.nav-btn.active {
  border-left-color: var(--accent);
  color: var(--accent);
}
```

The same pattern applies to overlay divs with click handlers — use
`<button>` or add `role="button" tabindex="0"` + keyboard handler (buttons
are preferred).

## Applying a Design System to an Existing Dashboard

When restyling an existing Svelte dashboard with a design system from
`popular-web-designs` (e.g. Kraken, Linear, Sentry), the workflow is:

1. **Load the design template** via `skill_view(name="popular-web-designs",
   file_path="templates/<site>.md")` to get exact color tokens, typography,
   radii, and component specs.
2. **Define CSS custom properties in `:global(:root)`** inside `App.svelte`.
   Translate the design system's palette into semantic variables
   (`--accent`, `--bg-surface`, `--green`, `--red`, `--radius`, etc.) so
   every component can reference them without hardcoding hex values.
3. **Migrate components one at a time** — replace hardcoded hex colors with
   `var(--token)` references. Start with `App.svelte` (global shell + sidebar),
   then `Dashboard.svelte` (grid), then each panel component.
4. **Canvas charts need hardcoded hex** — `ctx.fillStyle` / `ctx.strokeStyle`
   do NOT resolve CSS variables. Use the design system's hex values directly
   in canvas drawing code (e.g. `'#7132f5'` for Kraken purple, `'#149e61'`
   for Kraken green). Keep these in sync with the CSS variables manually.
5. **Build after each component** — `npm run build` catches broken
   `var()` references and a11y regressions early.
6. **Run a verification script** that greps for old hardcoded colors to
   find components you missed. See `references/design-system-restyling.md`
   for the migration checklist and dark-adaptation technique.

### Light-to-Dark Adaptation

Many `popular-web-designs` templates are light-theme (white backgrounds).
For a dark trading dashboard, invert the surface hierarchy while preserving
the accent and semantic colors:

| Design system (light) | Dark adaptation |
|---|---|
| White `#ffffff` surface | `#15161e` surface |
| Light gray border | `#252631` border |
| Near-black text `#101114` | `#ececf1` text |
| Accent (e.g. Kraken purple `#7132f5`) | Keep as-is |
| Green/red semantic | Keep as-is, use at 12-14% opacity for subtle backgrounds |

The accent color is the identity — preserve it exactly. The surface
hierarchy is what you invert. See `references/design-system-restyling.md`
for the full token mapping table.

## Build-Based Verification (No Test Suite)

When a Svelte frontend has no test suite, verify via a bash script that checks
structural properties + runs `npm run build`. See
`scripts/verify-frontend-build.sh` for a copyable template.

### Check Categories

1. **File existence** — all expected source files present
2. **Store exports** — `grep` for `export const <store>` in stores.js
3. **WebSocket handlers** — `grep` for each event type string
4. **API functions** — `grep` for `export async function <fn>`
5. **Syntax version** — no Svelte 5 runes (`$state`/`$derived`/`$effect`)
6. **No unexpected external libs** — grep for d3/chart.js/echarts if avoiding them
7. **Canvas usage** — grep for `canvas` in chart components
8. **Theme colors** — grep for expected hex colors across all source files
9. **Production build** — `npm run build` exits 0, no warnings, dist/ generated
10. **Lifecycle** — grep for `connectWebSocket`/`disconnectWebSocket` in views
11. **Responsive grid** — grep for `grid-template-columns` + `@media`
12. **Routing** — grep for each view name in App.svelte

### Running

```bash
bash /tmp/hermes-verify-frontend.sh
# Clean up after
rm /tmp/hermes-verify-frontend.sh
```

The script exits non-zero on any failure. Use `hermes-verify-` prefix for
temp scripts.

**Important:** This is structural + build verification, not runtime testing.
The build compiling cleanly validates that all components, imports, and stores
are syntactically correct and referenceable. Runtime behavior (WS events,
canvas rendering) requires a running backend + browser.

## Auto-Refresh + WS-Triggered Refetch

Canvas charts that display REST data need periodic refresh + reactive
refetch when a WS event signals the data shape has changed.

### Timer-Based Auto-Refresh

```javascript
let refreshTimer;

onMount(async () => {
  await refreshChart();
  refreshTimer = setInterval(async () => {
    await refreshChart();
  }, 60000); // 60s for daily-bar systems
});

onDestroy(() => {
  if (refreshTimer) clearInterval(refreshTimer);
});
```

### WS-Triggered Refetch (Store Clear Pattern)

When a config-change WS event fires (e.g. `StrategyConfigChange`), the
chart's SMA window may have changed — the old candle/SMA data is stale.
Pattern: the WS handler clears the store to `null`, and a reactive block
in the component detects the null and refetches.

In `websocket.js`:
```javascript
case 'StrategyConfigChange':
  chartData.set(null); // trigger refetch
  break;

case 'PnlTick':
  // Per-model event: route to the correct model slice
  if (msg.model_id) {
    updateSlice(msg.model_id, 'status', (s) => ({
      ...(s || {}),
      model_id: msg.model_id,
      symbol: msg.pair?.split('/')[0] || s?.symbol,
      realized_pnl: msg.realized_pnl,
      unrealized_pnl: msg.unrealized_pnl,
      position: msg.position,
      entry_price: msg.entry_price,
      last_close: msg.last_close,
      last_candle_ts: msg.timestamp,
    }));
  }
  break;
```

In the chart component:
```javascript
$: if ($chartData === null && candles.length) {
  refreshChart(); // refetch with new params
}
```

This two-phase approach (WS clears store → component reacts by refetching)
keeps the WS manager decoupled from API calls.

## Canvas Trade Markers

Overlay entry/exit markers on a candlestick chart. Map trade timestamps
to candle x-positions, then draw arrows:

```javascript
function tsToX(ts) {
  // Exact match first
  for (let i = 0; i < candles.length; i++) {
    if (candles[i].ts === ts) return padL + i * xStep + xStep / 2;
  }
  // Fallback: nearest candle by timestamp
  const target = new Date(ts).getTime();
  let bestI = 0, bestDist = Infinity;
  for (let i = 0; i < candles.length; i++) {
    const d = Math.abs(new Date(candles[i].ts).getTime() - target);
    if (d < bestDist) { bestDist = d; bestI = i; }
  }
  return padL + bestI * xStep + xStep / 2;
}

// Draw arrows: green up-triangle for long, red down-triangle for short
for (const t of trades) {
  const x = tsToX(t.ts);
  const y = yScale(t.price);
  const size = 6;
  ctx.fillStyle = t.side === 'long' ? '#3fb950' : '#f85149';
  ctx.beginPath();
  if (t.side === 'long') {
    ctx.moveTo(x, y - size);
    ctx.lineTo(x - size/2, y + size/2);
    ctx.lineTo(x + size/2, y + size/2);
  } else {
    ctx.moveTo(x, y + size);
    ctx.lineTo(x - size/2, y - size/2);
    ctx.lineTo(x + size/2, y - size/2);
  }
  ctx.closePath();
  ctx.fill();
}
```

Include trade prices in the Y-axis range calculation so markers stay
within the visible chart area.

## Multi-Model Store Partitioning

When a dashboard supports multiple trading models (or multiple assets) running
concurrently, flat singleton stores cause data collision: a `PnlTick` from
model A overwrites the status that model B just wrote. The fix is to partition
all per-model state by `model_id`.

### Architecture

Replace flat `writable` stores with a single `_slices` store (a `writable`
holding a plain object mapping `model_id → slice`), plus an `activeModelId`
writable that tracks which model the UI is currently displaying.

```javascript
import { writable, derived, get } from 'svelte/store';

export const activeModelId = writable(null);
export const models = writable([]);  // full list from /api/models

const _slices = writable({});

// Each slice holds all per-model telemetry:
// { status, predictions, features, trades, accuracy, chartData }

export const modelSlice = derived(
  [activeModelId, _slices],
  ([$id, $slices]) => $slices[$id] || { status: null, predictions: null, /* ... */ },
);
```

### updateSlice / setSlice Helpers

All writes go through two helpers that create the slice on first access:

```javascript
export function updateSlice(modelId, field, updater) {
  if (!modelId) return;
  _slices.update((slices) => {
    if (!slices[modelId]) {
      slices[modelId] = { status: null, predictions: null, features: null,
                           trades: [], accuracy: null, chartData: null };
    }
    const slice = slices[modelId];
    slice[field] = typeof updater === 'function' ? updater(slice[field]) : updater;
    return { ...slices };  // shallow clone for reactivity
  });
}

export function setSlice(modelId, field, value) {
  updateSlice(modelId, field, value);
}
```

**Key detail:** the `_slices.update()` callback must return `{ ...slices }`
(a shallow clone), not the mutated object. Svelte's reactivity checks
reference equality — mutating in place won't trigger subscribers.

### Legacy Derived Proxies (Backward Compat)

Existing components that import `status`, `predictions`, `trades`, etc. keep
working by making them `derived` stores that project the active model's slice:

```javascript
export const status = derived(
  [activeModelId, _slices],
  ([$id, $slices]) => $slices[$id]?.status ?? null,
);
export const trades = derived(
  [activeModelId, _slices],
  ([$id, $slices]) => $slices[$id]?.trades ?? [],
);
```

This means un-migrated components automatically follow `activeModelId` with
zero code changes. New components should read from `modelSlice` directly.

### WS Handler: Route by model_id

The WS `handleMessage` switch reads `msg.model_id` and routes per-model events
to the correct slice:

```javascript
function handleMessage(raw) {
  const msg = JSON.parse(raw.data);
  const mid = msg.model_id || null;

  switch (msg.type) {
    case 'PnlTick':
      if (mid) updateSlice(mid, 'status', (s) => ({
        ...(s || {}), model_id: mid,
        realized_pnl: msg.realized_pnl, /* ... */
      }));
      break;
    case 'TradeFill':
      if (mid) updateSlice(mid, 'trades', (list) =>
        [{ ...msg, model_id: mid }, ...list].slice(0, 50));
      break;
    case 'ModeChange':
      // Global — all models share the mode, update active slice only
      const activeId = get(activeModelId);
      if (activeId) updateSlice(activeId, 'status', (s) => ({ ...(s||{}), mode: msg.mode }));
      break;
    // ...
  }
}
```

Global events (ModeChange, StrategyConfigChange, EngineEvent) stay in global
stores — they are not per-model.

### Model Selector Dropdown in Dashboard

The Dashboard fetches `/api/models` on mount, picks the first enabled model
as active, and shows a `<select>` bound to `activeModelId`:

```svelte
<select class="model-selector" value={$activeModelId} on:change={onModelChange}>
  {#each modelOptions as opt}
    <option value={opt.model_id} disabled={!opt.enabled}>
      {opt.label}{#if !opt.enabled} (disabled){/if}
    </option>
  {/each}
</select>
```

When the active model changes, the Dashboard calls `loadModelData(modelId,
symbol)` which fetches per-model REST data into the correct slice via
`setSlice(modelId, 'status', ...)`.

### Per-Component Reactive Reload

Components that fetch their own REST data (PnL curve, FeatureInspector,
StrategyConfigPanel) need to reload when `activeModelId` changes. The pattern:

```javascript
let lastModelId = null;
$: if ($activeModelId && $activeModelId !== lastModelId) {
  lastModelId = $activeModelId;
  loadHistory();  // or loadFeatures(), loadConfig()
  draw();          // for canvas components, clear + redraw
}
```

Canvas chart components (PnLEquityCurve) must also reset their local state
(`pnlHistory = []`) on model switch to avoid blending data from different
models.

### StrategyConfigPanel: Per-Model API Calls

The strategy config API now accepts `?model_id=X`. The panel passes
`$activeModelId` to both fetch and save:

```javascript
config = await fetchStrategyConfig($activeModelId);
config = await saveStrategyConfig(config, $activeModelId);
```

### Store Partitioning Verification

Verify the partitioning logic with a standalone `.mjs` test that imports
`svelte/store` and replicates the `updateSlice`/`derived` contract inline
(can't import `stores.js` directly in Node — it uses Svelte's compiler). Run
it from inside the `frontend/` directory so `svelte` resolves from
`node_modules`:

```bash
cd frontend && node verify-stores.mjs && rm verify-stores.mjs
```

Test these behaviors:
1. Fresh `modelSlice` is empty when no active model
2. `updateSlice` on model A doesn't leak to model B (isolation)
3. Legacy derived stores follow `activeModelId` (switch model → legacy reads change)
4. Trades are isolated per model
5. `setSlice` non-functional update works
6. `modelSlice` derived updates reactively (subscribe, mutate, check)

### Events View: Model Attribution

The Events view is global (shows all models' events). The `model_id` lives in
`ev.payload.model_id` for per-model events. Extract it:

```javascript
function modelId(ev) {
  return ev.payload?.model_id || ev.model_id || null;
}
```

Show a badge in the event row when present:
```svelte
{#if modelId(ev)}
  <span class="model-badge">{modelId(ev)}</span>
{/if}
```

**See `references/marketmoves-multi-model-dashboard.md` for a concrete migration log (QQQ → QQQ+NVDA), including per-model API calls, hardcoded-title fixes, and global trade history with model attribution. See `references/per-symbol-dashboard-wiring.md` for the per-symbol REST wiring pattern (api.js helpers, Dashboard loader, periodic status poll, and StatusPanel model/pair display).**

### Pitfall: `get()` from svelte/store, not from stores.js

The WS handler needs `get(activeModelId)` to read the current active model for
global events. Import it from `svelte/store`, not from your own `stores.js`:

```javascript
// WRONG — stores.js doesn't export `get`
import { activeModelId, get } from './stores.js';

// RIGHT
import { activeModelId } from './stores.js';
import { get } from 'svelte/store';
```

## Common Pitfalls

1. **Array stores initialized to `null` instead of `[]`.** Components using
   `{#each $trades as t}` will throw. Always init array stores to `[]`.

2. **Forgetting `disconnectWebSocket` in `onDestroy`.** The WS connection
   survives view unmount and causes duplicate event handlers when navigating
   back. Always pair `connectWebSocket()` in `onMount` with
   `disconnectWebSocket()` in `onDestroy`.

3. **Canvas not resizing.** Without a `ResizeObserver`, the canvas renders at
   its initial size and doesn't adapt to container changes. Always observe the
   container and redraw on resize.

4. **DPR scaling applied but canvas style not set.** Setting `canvas.width =
   w * dpr` without `canvas.style.width = w + 'px'` makes the canvas appear
   2x/3x too large. Both must be set.

5. **Using `role="button"` on `<li>` to fix a11y warnings.** This produces a
   *new* warning. Use `<button>` inside `<li>` instead. See the a11y section.

6. **Backoff not reset on reconnect.** If `backoff` isn't reset to the initial
   value on `onopen`, reconnection after a brief outage uses the capped 30s
   delay forever. Reset `backoff = 1000` in `ws.onopen`.

7. **`manuallyClosed` flag not checked in `onclose`.** Without this guard,
   calling `disconnectWebSocket()` triggers an immediate reconnect attempt,
   defeating the purpose.

8. **Build warnings treated as acceptable.** Svelte a11y warnings are
   actionable — fix them rather than shipping with warnings. They indicate
   real accessibility issues.

9. **REST initial-load shape ≠ WS live-update shape.** When a component
   fetches initial data via REST and receives live updates via WS, the two
   payloads often have **different shapes**. Example: REST returns
   `{latest: {trend_slope: 0.06, rsi_14: 58.4, ...}}` (named fields), but
   the WS event sends `{features: [0.06, 58.4, ...], normalized: [...]}` (arrays).
   If the component reads `data.latest.normalized[i]` it works for WS but
   gets `undefined` from REST; if it reads `data.latest.trend_slope` it works
   for REST but gets `undefined` from WS. **Fix:** add a reactive normalization
   block that detects the shape and converts to a common internal representation
   before rendering:
   ```javascript
   $: resolved = (() => {
     if (!raw) return null;
     if (raw.trend_slope !== undefined) return raw;           // REST shape
     if (Array.isArray(raw.features)) {                        // WS shape
       const obj = {};
       FIELD_DEFS.forEach((d, i) => { obj[d.key] = raw.features[i] ?? 0; });
       return obj;
     }
     return raw;
   })();
   ```
   This is the #1 cause of "the dashboard shows nothing" when the backend is
   confirmed working — always curl the REST endpoint and compare its JSON shape
   to what the component's reactive block reads.

10. **WS events that carry data for multiple stores.** A single WS event may
    carry data relevant to more than one store. Example: a `PredictionUpdate`
    event carries `pred_1d/5d/21d` that belongs in both the `predictions` store
    (for the chart/predictions panel) AND the `status` store (for the status
    panel's prediction rows). If the handler only updates `predictions`, the
    status panel never shows live prediction changes. **Fix:** in the WS
    `switch` handler, call `.update()` on every store that needs the data:
    ```javascript
    case 'PredictionUpdate':
      predictions.update(p => ({ ...p, latest: { ...msg } }));
      status.update(s => ({ ...s, pred_1d: msg.pred_1d }));  // don't forget this
      break;
    ```
    When adding a new WS event type, enumerate which stores/components consume
    each field and update all of them.

11. **Component template missing rows for fields the API already returns.**
    The status API can return `pred_1d`, `pred_5d`, `pred_21d` correctly, the
    store can carry them, but if the component's template has no `<div class="row">`
    for them, they're invisible. When a user reports "X is missing from the
    dashboard," first curl the API to confirm the data is there, then check if
    the component actually renders a row for each field — not just whether the
    reactive binding exists.

12. **Backend 503 from a hardcoded stub, not a real error.** When a frontend
    component shows "N/A" or the console logs a 503 on an API call, the
    backend handler may be a stub that always returns
    `Err((StatusCode::SERVICE_UNAVAILABLE, "not yet implemented"))`. This
    looks identical to a server crash from the frontend's perspective. Before
    debugging the frontend, curl the endpoint:
    ```bash
    curl -s -o /dev/null -w '%{http_code}' http://localhost:9080/api/accuracy
    # 503 → grep the handler for SERVICE_UNAVAILABLE
    grep -rn 'SERVICE_UNAVAILABLE' engine/src/api/
    ```
    If the handler is a stub, implement it (call the real db function, return
    `Ok(Json(...))`) rather than treating the 503 as a transient server issue.
    The frontend's `fetchAccuracy()` should return `null` on any non-200
    (not throw), so the component degrades gracefully to "N/A" while the
    stub exists.

13. **On-the-fly accuracy when the predictions table has no actuals columns.**
    The crypto `predictions` table has `actual_1h/4h/24h` columns filled by
    `compute_actuals()`. The equity `equity_predictions` table does NOT — it
    only has `pred_1d/5d/21d`. To compute accuracy for equities, join
    predictions with future candle closes at runtime: for each prediction at
    `candle_ts`, look up `close[candle_ts + N_days]` from `equity_candles`,
    compute `actual = ln(future_close / base_close)`, then compare direction
    and MAE. Use a tolerance of ±3 calendar days when finding the future
    candle to skip weekends/holidays. See `db::fetch_equity_accuracy` +
    `find_closest_close` in the MarketMoves engine for the reference
    implementation.

14. **Phantom WS events — defined and handled but never emitted.** A WS event
    variant can exist in the backend telemetry enum, have a serialization test,
    and be handled in the frontend `switch` block — yet **no backend code ever
    broadcasts it**. The frontend field relying on that event will freeze at
    its initial REST load value and never update. This is invisible from the
    frontend side alone. **Fix:** when a displayed field seems frozen, grep
    the backend for `tx.send(TelemetryEvent::YourVariant` — if there are zero
    call sites, it's a phantom event. Add a REST polling fallback for that
    field (see "REST Polling Fallback" section above). Do NOT assume a WS
    event is emitted just because it's defined and handled.

15. **`export function` silently dropped during `write_file` full rewrites.**
    When a Svelte component is rewritten entirely (not patched), any `export
    function` declarations are silently omitted if they aren't in the new file.
    Components using `bind:this={chartComponent}` in the parent will call
    `chartComponent.someFunction()` and get `TypeError: chartComponent.someFunction
    is not a function` at runtime. Build does not catch this — `bind:this`
    is typed loosely and the error only surfaces in the browser.

    **Fix:** After any full file rewrite of a component that uses `bind:this`
    in its parent, grep every function the parent calls on the bound ref and
    verify each one is `export function` in the child:
    ```bash
    # Find all calls on the bound component ref in the parent
    grep -n 'chartComponent\.' src/views/Dashboard.svelte
    # Verify each one is exported in the child component
    grep 'export function' src/lib/components/CandlestickChart.svelte
    ```
    Always rebuild + redeploy after any full component rewrite — the running
    container will serve a stale bundle without a rebuild.

16. **Modal backdrop a11y — `on:click|stopPropagation` on inner div.**
    Putting `on:click|stopPropagation` on the inner modal `<div>` triggers
    "Non-interactive element should not be assigned mouse event listeners."
    Adding `role="dialog"` doesn't help. **Fix:** remove the `stopPropagation`
    handler from the inner div entirely. Instead, on the backdrop, check
    `e.target === e.currentTarget` before closing — this naturally ignores
    clicks that originate inside the modal. Add `on:keydown` for Escape
    on the backdrop for keyboard users:
    ```svelte
    <div class="modal-backdrop"
         on:click={(e) => { if (e.target === e.currentTarget) closeModeModal(); }}
         on:keydown={(e) => { if (e.key === 'Escape') closeModeModal(); }}
         role="button" tabindex="0">
      <div class="modal" role="dialog" aria-modal="true">
        <!-- modal content, no click handler needed -->
      </div>
    </div>
    ```
    This produces zero a11y warnings and is the correct pattern for
    click-outside-to-close modals in Svelte 4.

17. **Chart OHLCV data can be years stale while live price is current.**
    The candle DB may contain 2021-era candles ($365 for QQQ) while Yahoo
    live quote is current ($688). The chart endpoint returns stale candles
    without flagging them, so the chart Y-axis is centered on wrong-era
    data while the live price and prediction cones show current prices —
    visually broken and analytically wrong.

    **Defense (backend):** the chart response must ALWAYS include a
    `live_quote` field + a `stale` boolean:
    ```rust
    pub struct ChartResponse {
        pub candles: Vec<CandleDto>,
        pub sma: Vec<SmaPoint>,
        pub stale: bool,             // candles > 48h old
        pub live_quote: Option<LiveQuote>,  // ALWAYS included
    }
    ```
    The live_quote is fetched separately from the backfill (only Yahoo's
    meta block — fast) and is returned even when the backfill fails.

    **Defense (frontend):** use `live_quote` from the chart response (single
    round-trip — NOT a separate fetch) and re-center the Y-axis on the
    live price when stale:
    ```javascript
    const data = await fetchChart();
    isStale = !!data.stale;
    if (data.live_quote) livePrice = data.live_quote.price;

    // In draw() — AFTER computing minP/maxP from candles:
    if (isStale && livePrice != null) {
      const half = (maxP - minP) / 2;
      minP = livePrice - half;
      maxP = livePrice + half;
    }
    ```
    Stale candles still render behind the scenes (xStep computed from their
    count) but the visible Y-axis is anchored to today. Show the live
    price + a "stale data" badge in the UI so the user knows the chart is
    in a degraded state.

    **Yahoo rate-limiting:** VPS IPs are often 429'd by Yahoo. Inside the
    container, test with `docker exec mmn-engine curl -s -o /dev/null -w
    "%{http_code}\n" --max-time 10 "https://query1.finance.yahoo.com/..."`.
    If 429, the live_quote becomes `None` and the UI falls back to
    `last_close` from the store.

    18. **Triplicated price overlays: canvas header + canvas badge + DOM badge.**
        When adding a live price indicator to a canvas chart, it's easy to end
        up with the same price text drawn in three places:

        1. A canvas `fillText` at the top-left of the chart area (the "chart
           header" — `ctx.fillText('QQQ', padL, 12); ctx.fillText(price, ...)`)
        2. A canvas `roundRect` badge at the right edge of the live-price line
        3. A DOM `<div class="quote-badge">` element above the canvas with the
           price, change, change%, and staleness tag

        All three show the same number and diverge on different refresh cycles
        (the canvas redraws on every store update; the DOM badge updates with
        `liveQuote` from the API response; the values can briefly disagree).
        Users will see what looks like "the price is flickering."

        **Decision rule:** keep the DOM badge — it's the richest (price + change
        % + staleness tag in one place, easy to style, accessible). Remove the
        canvas text. Keep the canvas's *dashed live-price horizontal line* (the
        y-axis anchor) but drop the canvas badge text — the line alone is
        enough visual anchoring, the DOM badge handles the number.

        **Audit checklist when adding a new canvas overlay:**
        ```bash
        # In the chart component, find every place the price is drawn:
        grep -nE 'fillText|roundRect' src/lib/components/YourChart.svelte
        # For each canvas text/badge, ask: does the DOM already show this?
        # If yes → delete from the canvas.
        ```

        The same principle applies to timestamps, ticker symbols, and OHLC
        summary text — if the DOM component renders it, the canvas shouldn't.

    19. **Dashboard grid resizing breaks when grid-area assignments overlap.**
        When widening a card (e.g. making the chart span 2/3 instead of 1/3
        width), you must reshuffle ALL `grid-column` assignments so no two
        areas land on the same cell — CSS Grid's auto-placement will silently
        push the second one to a new row, leaving a gap where you expected a
        card.

        **Workflow:** draw the new layout on paper first (which card goes in
        which row × col), then patch all 6-8 `grid-column: x / y` lines in
        one batch. Verify with:
        ```bash
        grep -oE '\.[a-z-]+ \{ grid-column: [0-9]+ / [0-9]+' \
          src/views/Dashboard.svelte | sort
        # Every (column-start, column-end) pair must be unique per row.
        ```
        If two areas share the same `1 / 2`, the second one gets bumped down
        a row by auto-placement.

    20. **Prediction/forecast overlays clip off the right edge when xSlots equals data length.**
    When the chart's X-scale divides the canvas width by the candle count
    (`xStep = cw / n`), anything that extends past the last candle
    (prediction cones, forecast bands, projected event markers) gets drawn
    off-canvas. With 90 daily candles and a 21D prediction cone, the cone
    line, target dot, and price label all render past the canvas right edge
    and get clipped silently. The user sees the cone appear to vanish into
    the right border with no error.

    **Fix:** allocate extra X-slots for the future band that don't correspond
    to data:
    ```javascript
    const futureBars = 21; // longest horizon drawn
    const xSlots = n + futureBars;
    const xStep = cw / xSlots;
    const candleRight = padL + (n - 1) * xStep + xStep;
    const futureRight = padL + xSlots * xStep;
    ```
    Then `candleRight` is the edge of the data region, `futureRight` is the
    canvas edge, and overlays like `endX = lastX + cone.bars * xStep` land
    inside the canvas. The `padR` margin must also be wide enough for any
    right-edge pill (`~70px` for a price tag).

    **Audit pattern:** after any chart change, scan for `xStep` /
    `cw / n` patterns and verify the cone/forecast code uses the buffered
    `xSlots`, not `n`. If you find `cw / n` and a 21-bar cone, you have the
    clipping bug.

21. **Verification scripts need to check structural properties of the *change*,
    not just the file's existence.** When a Svelte chart rewrite adds four
    features (clipping fix, volume, timeframe, crosshair), `npm run build`
    passing is necessary but insufficient — it tells you the syntax is fine,
    not that you actually implemented what you claimed. Write a focused
    verification script with regex assertions on the specific patterns that
    prove each feature shipped (e.g. `xSlots = n + futureBars`,
    `volScale`, `TIMEFRAMES`, `onMouseMove`). Run it once, report pass/fail
    counts, then delete it. The user's tolerance for build-based verification
    doesn't extend to trusting that the change happened — show the proof.
    Place temp scripts under `/tmp/hermes-verify-*` so they don't pollute
    the repo, and clean up after.

22. **CSS custom properties cannot be used inside canvas drawing code.**
    `ctx.fillStyle`, `ctx.strokeStyle`, `ctx.font`, and `ctx.shadowColor` do
    NOT resolve CSS variables — `ctx.fillStyle = 'var(--accent)'` silently
    falls back to default black, and `ctx.font = '10px var(--font-mono)'`
    silently falls back to default sans-serif. Use the design system's hex
    values directly inside `draw()` (e.g. `'#7132f5'` for Kraken purple), and
    keep these in sync with the CSS variables in `:global(:root)` manually.
    DOM elements (`<div>`, `<button>`, `<span>`) can and should use CSS
    variables — only the canvas drawing calls are affected.

23. **Calling `.set()` on a `derived` store throws `X.set is not a function`
    (minified as `Ll.set`, `Ab.set`, etc.).** `derived` stores are read-only
    — they project from other stores and have no `.set()` method. A component
    that calls `chartData.set(data)` where `chartData` is a `derived` store
    will throw at runtime, not at build time. The error surfaces in the
    browser console as `TypeError: Ll.set is not a function` (the minified
    variable name replaces `chartData`), which is cryptic and doesn't point
    at the store definition.

    This is the #1 pitfall when migrating from flat `writable` stores to the
    multi-model `derived` proxy pattern (see "Multi-Model Store
    Partitioning"). Legacy code that called `writableStore.set(data)` will
    silently break when the store becomes `derived` — the build succeeds
    because `.set()` is valid JavaScript on any object, it just doesn't
    exist on the derived store's interface.

    **Fix:** replace `storeName.set(data)` with `updateSlice($activeModelId,
    'storeName', data)` (or the appropriate write helper for your store
    architecture). For the multi-model pattern, the `_slices` writable is
    the only store that should receive `.set()` / `.update()` calls — all
    `derived` proxies are read-only projections of it.

    **Audit pattern after migrating any store from `writable` to `derived`:**
    ```bash
    # Find every .set() call on the migrated store across all components
    grep -rn 'storeName\.set(' frontend/src/
    # Each hit is a runtime bug — replace with the write helper
    ```
    Common instances: `chartData.set(data)` in chart refresh callbacks,
    `status.set(data)` in REST fetch handlers, `predictions.set(data)` in
    model-load functions. All of these become `updateSlice` or `setSlice`
    calls when the store migrates to a `derived` proxy.

## Verification Checklist

- [ ] `npm run build` exits 0 with zero warnings
- [ ] All array stores initialized to `[]`
- [ ] Every `connectWebSocket()` paired with `disconnectWebSocket()` in `onDestroy`
- [ ] Canvas charts have ResizeObserver + DPR scaling + style width/height set
- [ ] No `role="button"` on `<li>` — use `<button>` inside `<li>` instead
- [ ] Modal backdrops use `e.target === e.currentTarget` check, not `stopPropagation` on inner div
- [ ] WS backoff reset on `onopen`, `manuallyClosed` checked in `onclose`
- [ ] Every `bind:this` ref's called methods are `export function` in the child component
- [ ] Chart endpoint bundles `live_quote` + `stale` flag; frontend re-centers on stale data
- [ ] No triplicated price/timestamp text between canvas `fillText` and DOM badges — keep one source of truth
- [ ] Dashboard grid-area assignments are unique per row (no two areas share the same `grid-column` span)
- [ ] Ad-hoc verification script run and all checks passed
- [ ] Temp verification script cleaned up from `/tmp`
