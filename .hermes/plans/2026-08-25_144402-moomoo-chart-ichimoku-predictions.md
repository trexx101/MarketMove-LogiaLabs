# Plan: Moomoo Screener — Robust Chart, Ichimoku/SMA, Predictions Overlay

**Date:** 2026-08-25
**Scope:** Phase 1 (robust chart + Ichimoku + SMA + predictions overlay + timeframes) and Phase 2 (prediction-backtest overlay via additive `/api/chart` field). Phase 3 (intraday) stays aspirational.

## Goal
Replace the fragile chart with a production-grade lightweight-charts frontend that renders clean candles, moving averages (SMA), Ichimoku Cloud, and a predictions overlay — sourced from the existing Rust backend. Phase 2 adds the historical predictions/predicted-vs-actual overlay by extending the `/api/chart` response additively (no breaking change), feeding from the `equity_preds` table.

## Context / Current State (verified earlier)
- **Backend:** Rust. `src/chart.rs` (HTTP handler), `src/yahoo.rs` and `src/moomoo.rs` (both daily-only sources today), `src/predict.rs` (predictions), `equity_preds` table already exists with prediction rows.
- **Frontend:** Svelte. `CandlestickChart.svelte` (current chart component), `src/lib/api.js` (API client), `src/lib/stores.js` (state).
- **package.json:** already lists `lightweight-charts` dependency + tooling.
- **Current API shape:** GET `/api/chart?symbol=X...` returns `{ symbol, ... bars/points }`. Phase 2 must keep every existing field and only **add** new ones so the frontend degrades gracefully on old payloads.
+
## Milestones
1. **Backend Phase 1:** `/api/chart` returns candles + high-level indicators computed server-side (or supplies raw series for client-side Ichimoku/SMA — see Open Decision A). Timeframe param `1D|1W|1M` supported.
2. **Backend Phase 2:** `/api/chart` additively returns historical predictions (predicted vs actual) for overlay, aggregated from `equity_preds`. No existing field removed.
3. **Frontend Phase 1:** Rewrite `CandlestickChart.svelte` on lightweight-charts with candles, SMA lines, Ichimoku (cloud, tenkan, kijun, chikou, senkou A/B), markers, timeframe switcher wired to stores/api.js, predictions overlay toggle.
4. **Testing:** backend unit test on additive-payload contract + reshape; frontend build passes; manual sanity check vs a known ticker.

## Implementation Steps

### Step 1 — Backend: chart.rs (Phase 1)
- Add `timeframe` query param (`1D|1W|1M`); aggregate/cache daily bars accordingly (keep existing daily path as default `1D`).
- Compute lightweight indicators server-side **if Decision A = server-side**: SMA(20/50/200), Ichimoku components (tenkan/ kijun/ senkou A/B/ chikou/ cloud / base). Emit as series arrays aligned to the candle timestamps, plus a `cloud` area (upper/lower bounds).
- **Emit shape:** keep `{ symbol, ...existing }`; **add** `indicators: { sma: {20:[...],50:[...],200:[...]}, ichimoku: {...} }`.

### Step 2 — Backend: chart.rs (Phase 2)
- Query `equity_preds` for the symbol: for each candle date, find the prediction made previously (prediction timestamp < candle date) → `predicted` value; `actual` = that candle's close.
- **Add** `predictions: [{ time, predicted, actual }]` — nothing removed. Handle "no predictions" as empty array (frontend hides overlay).

### Step 3 — Frontend: CandlestickChart.svelte (Phase 1+2)
- Rebuilt on `lightweight-charts` (already in package.json). Components:
  - Candlestick series (timestamps + OHLC).
  - SMA line series (20/50/200; toggles).
  - Ichimoku: cloud = area between senkou A/B (fill), tenkan/kijun/chikou line series, optional `chikou` shift.
  - Predictions overlay: scatter/line series (`predicted` vs `actual`) gated by a toggle; markers for prediction points if available.
- Timeframe switcher (1D/1W/1M) lives in the component or parent, drives `stores.js` + `api.js` refetch.
- Resize handling (ResizeObserver).

### Step 4 — Frontend: api.js / stores.js
- `api.js`: add `fetchChart(symbol, { timeframe })`; pass through new `indicators` + `predictions` fields; defensive parse (empty arrays OK).
- `stores.js`: add `selectedTimeframe`, `showPredictions`, `showIchimoku`, `showSMA`.

### Step 5 — Testing & Verification
- Backend: unit test asserting additive union — old-shape consumers still parse; new fields present; `predictions` empty when no rows.
- `cargo build` / `cargo test`; frontend build passes.
- Manual: load a real ticker, toggle 1W/1M, toggle Ichimoku/SMA/predictions; confirm cloud renders and no NaN/infinite gaps.

## Dependencies / Known Constraints
- `equity_preds` schema: confirm exact columns (symbol, prediction_time, predicted price) before writing the join — **Decision B**.
- Daily-only sources today (yahoo.rs / moomoo.rs). Intraday = Phase 3 (aspirational; not built here).
- lightweight-charts version pinned in package.json — keep API calls compatible with that pinned major version.

## Risks / Pitfalls
- **Additive-contract drift:** ensure no existing `chart.rs` field is renamed/removed in Phase 2 — the frontend must not break against cached old payloads.
- **Ichimoku on short history:** senkou B & chikou need lookback (52 periods); guard against short history → emit empty arrays, never NaN/infinite.
- **Predictions alignment:** matching predictions to actual candles must dedupe, sort by time, and handle multiple predictions per day (take nearest prior).
- **Chunked/truncated bars:** large histories (1M) — make sure API paging doesn't clip the overlay series.

## Open Decisions (grill-me targets)
- **A. Indicator computation site:** server-side (backend computes, frontend draws; keeps logic in Rust/tests) vs client-side (backend sends raw OHLC, frontend computes Ichimoku/SMA). Recommend **server-side** for testability + contract stability.
- **B. equity_preds schema:** confirm columns before Step 2 (prediction timestamp vs bar date mapping).
- **C. Predictions overlay style:** line vs scatter vs markers; separate-pane vs overlay-on-candles.
- **D. Timeframe default & persistence:** default `1D`, persist selection or not.
- **E. SMA periods:** 20/50/200 assumed — confirm custom set (e.g. 10/30/60).

## Out of Scope (Phase 3, aspirational)
- Intraday / minute-level data & charting; live WebSocket updates; backtesting UI beyond prediction-vs-actual overlay.