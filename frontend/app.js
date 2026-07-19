/**
 * MarketMarkovNet Control Room — SPA entry point.
 *
 * ## Module pattern
 *
 * Each view is an ES module that exports a single function:
 *
 *     export function render(rootEl, data) { ... }
 *
 * To add a new view:
 *   1. Create a new module in `views/` exporting `render(rootEl, data)`.
 *   2. Import it here.
 *   3. Add an entry to the `views` array below:
 *        { id: "panel-id", api: fetchFn, module: viewModule }
 *
 * The main loop fetches all APIs every 5 s and calls each view's `render`
 * with the corresponding data.  Errors are logged but do not stop the loop.
 */

import { fetchStatus, fetchPredictions, fetchChart, fetchAccuracy } from "./api.js";
import * as statusView from "./views/status.js";
import * as predictionsView from "./views/predictions.js";
import * as chartView from "./views/chart.js";
import * as accuracyView from "./views/accuracy.js";

// ── View registry ──────────────────────────────────────────

const views = [
  { id: "status-panel", api: fetchStatus, module: statusView },
  { id: "predictions-panel", api: fetchPredictions, module: predictionsView },
  { id: "chart-panel", api: fetchChart, module: chartView },
];

// Accuracy is polled less frequently (every 60s) since it only changes
// when predictions resolve. The /api/accuracy endpoint may return 503
// when no predictions have resolved yet — that's handled gracefully here.

async function tickAccuracy() {
  try {
    const data = await fetchAccuracy();
    const rootEl = document.getElementById("accuracy-panel");
    accuracyView.render(rootEl, data);
  } catch (err) {
    // 503 (no resolved predictions) or any other error → render fallback
    const rootEl = document.getElementById("accuracy-panel");
    accuracyView.render(rootEl, null);
  }
}

// ── Mode badge ─────────────────────────────────────────────

function updateModeBadge(mode) {
  const badge = document.getElementById("mode-badge");
  if (!badge) return;

  badge.classList.remove("mode-badge--paper", "mode-badge--live", "mode-badge--unknown");

  if (mode === "paper") {
    badge.textContent = "PAPER";
    badge.classList.add("mode-badge--paper");
  } else if (mode === "live") {
    badge.textContent = "LIVE";
    badge.classList.add("mode-badge--live");
  } else {
    badge.textContent = "—";
    badge.classList.add("mode-badge--unknown");
  }
}

// ── Poll loop ──────────────────────────────────────────────

async function tick() {
  for (const view of views) {
    try {
      const data = await view.api();
      const rootEl = document.getElementById(view.id);
      view.module.render(rootEl, data);

      // Update mode badge from status response
      if (view.api === fetchStatus && data && data.mode) {
        updateModeBadge(data.mode);
      }
    } catch (err) {
      console.warn("[tick]", view.id, err.message);
    }
  }
}

// Initial render + 5-second interval
tick();
const intervalId = setInterval(tick, 5000);

// Accuracy polled every 60 seconds (separate, slower cadence)
tickAccuracy();
const accuracyIntervalId = setInterval(tickAccuracy, 60000);

// Clean up on page hide (optional)
window.addEventListener("pagehide", () => {
  clearInterval(intervalId);
  clearInterval(accuracyIntervalId);
});
