# Dashboard Layout — Layout B (TradingView-Lite)

Canonical layout for `frontend/src/views/Dashboard.svelte`. Settled
2026-08-02 after evaluating Pro Terminal, TradingView-Lite, and
Minimal Monitor variants.

## Grid Map

12-column grid, `gap: 1rem`. Each section uses `display: flex;
flex-direction: column` and `min-width: 0` so children fill the shell
without overflowing.

```
┌────────────────────────────┬──────────────┐
│                            │              │
│   Chart (8/12)             │  StatusPanel │
│   CandlestickChart         │  (4/12)      │
│                            │              │
├────────────────────────────┴──────────────┤
│                                          │
│   PnL Equity Curve (full width)          │
│   PnLEquityCurve                         │
│                                          │
├──────────────┬─────────────┬──────────────┤
│  Features    │ ModelHealth │  Strategy    │
│  (4/12)      │  (4/12)     │  ConfigPanel │
│              │             │  (4/12)      │
├──────────────┴─────────────┴──────────────┤
│                                          │
│   TradeHistory (full width, bottom)      │
│   Horizontal band — table view           │
│                                          │
└──────────────────────────────────────────┘
```

## CSS Grid Assignments

```css
.chart-area    { grid-column: 1 / 9;   grid-row: 1; }
.rail          { grid-column: 9 / 13;  grid-row: 1; min-height: 420px; }

.pnl-area      { grid-column: 1 / 13;  grid-row: 2; }

.meta-features { grid-column: 1 / 5;   grid-row: 3; }
.meta-health   { grid-column: 5 / 9;   grid-row: 3; }
.meta-strategy { grid-column: 9 / 13;  grid-row: 3; }

/* Horizontal trade band at the absolute bottom */
.trade-area    { grid-column: 1 / 13;  grid-row: 4; }
```

## Responsive Breakpoints

### Wide screens (≥1600px)

Right rail stretches with the chart (which can grow tall):

```css
@media (min-width: 1600px) {
  .rail { min-height: 480px; }
}
```

### Tablets (≤1100px)

Chart full-width on top, StatusPanel below it (instead of beside), footer
becomes 2-up + 1, trade band stays full-width:

```css
@media (max-width: 1100px) {
  .chart-area     { grid-column: 1 / 13; grid-row: 1; }
  .rail           { grid-column: 1 / 13; grid-row: 2; min-height: 0; }
  .pnl-area       { grid-column: 1 / 13; grid-row: 3; }
  .meta-features  { grid-column: 1 / 7;  grid-row: 4; }
  .meta-health    { grid-column: 7 / 13; grid-row: 4; }
  .meta-strategy  { grid-column: 1 / 13; grid-row: 5; }
  .trade-area     { grid-column: 1 / 13; grid-row: 6; }
}
```

### Phones (≤640px)

Single column stack, all `grid-column: 1 / -1; grid-row: auto`.

## Panel Responsibilities

| Panel          | Owns                                                          |
| -------------- | ------------------------------------------------------------- |
| CandlestickChart | OHLC + SMA + volume + cones + crosshair + timeframe        |
| StatusPanel    | Mode, symbol, position, entry, last close, preds, PnL        |
| PnLEquityCurve | Aggregated equity curve over time                             |
| FeatureInspector | Per-feature values for the latest bar                       |
| ModelHealth    | WS connection, **data staleness (in hours)**, dir-acc, MAE   |
| StrategyConfigPanel | Strategy params + mode switch modal                       |
| TradeHistory   | Per-trade list with realized PnL per row (table, full width) |

**Hard rule:** never duplicate a metric across panels. If you add a
metric to one, audit the others and remove from the source.

## Component Slot Heights

- Chart card: `height: 420px` (price band 72% + volume band 22% + gap 6%)
- StatusPanel rail: `min-height: 420px` on desktop, grows to 480px on wide
- TradeHistory table: `max-height: 250px; overflow-y: auto` with sticky
  table headers so long trade lists scroll inside the band without
  expanding the dashboard height

## Why This Layout

- **Chart is the hero.** Trading dashboards earn the user's eyes through
  the chart. 8/12 width on row 1 keeps it large without forcing full-row.
- **Right rail = quick state.** StatusPanel next to the chart answers
  "what's the position right now?" in one glance.
- **PnL full-width.** The equity curve is the single most important
  performance metric — strip-chart shape is easier to read full-width.
- **Footer 3-up.** Three narrow panels (features/health/strategy) fit
  the leftover 12 cols; none of them need to be large.
- **Trade band at the bottom.** Long, tabular, full-width — that's what
  scroll happens for. Keeping it as the final row means the user can
  scroll down to inspect history without losing the chart above the fold.
