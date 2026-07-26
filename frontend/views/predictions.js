/**
 * Predictions view — renders latest predictions and recent history.
 *
 * The top of the panel (`pred 1D`, `pred 5D`, `pred 21D`, `pred 1H (scaled)`,
 * `pred 5H (scaled)`) is sourced from `GET /api/status` because that's where
 * the equities-style daily predictions (`pred_1d/5d/21d`) live, plus the
 * API-computed intraday approximations (`pred_1h_approx/5h_approx`).
 *
 * The history table at the bottom is sourced from `GET /api/predictions`,
 * which exposes the same approximate fields on each history row.
 *
 * @param {HTMLElement} rootEl  — #predictions-body (latest) + #predictions-history
 * @param {object}      data    — response from GET /api/predictions
 */

import { fetchStatus } from "../api.js";

function fmtPred(val) {
  if (val == null) return "—";
  return Number(val).toFixed(6);
}

// Percentage with leading sign, 2 decimals. Used for the new scaled rows.
function fmtPct(val) {
  if (val == null) return "—";
  const n = Number(val);
  if (!Number.isFinite(n)) return "—";
  const sign = n >= 0 ? "+" : "";
  return sign + (n * 100).toFixed(2) + "%";
}

function predClass(val) {
  if (val == null) return "val-neutral";
  return Number(val) >= 0 ? "val-pos" : "val-neg";
}

function setText(id, text, className) {
  const el = document.getElementById(id);
  if (!el) return;
  el.textContent = text;
  if (className) {
    el.className = "v " + className;
  }
}

function fmtTs(ts) {
  if (!ts) return "—";
  try {
    return new Date(ts).toISOString().replace("T", " ").slice(0, 16) + "Z";
  } catch {
    return ts;
  }
}

// Refresh the top-of-panel rows (pred 1D, 5D, 21D and the scaled 1H/5H)
// from /api/status. Called both once on render() and on a recurring timer
// so the panel updates independently of the /api/predictions cadence.
async function refreshTopPanel() {
  let status = null;
  try {
    status = await fetchStatus();
  } catch (err) {
    // Status may be temporarily unavailable — leave the rows as "—".
    return;
  }
  if (!status) return;

  setText("p-1d", fmtPred(status.pred_1d), predClass(status.pred_1d));
  setText("p-5d", fmtPred(status.pred_5d), predClass(status.pred_5d));
  setText("p-21d", fmtPred(status.pred_21d), predClass(status.pred_21d));
  setText("p-1h-approx", fmtPct(status.pred_1h_approx), predClass(status.pred_1h_approx));
  setText("p-5h-approx", fmtPct(status.pred_5h_approx), predClass(status.pred_5h_approx));
}

function renderHistory(container, history) {
  if (!container) return;

  if (!history || history.length === 0) {
    container.innerHTML = "";
    return;
  }

  const rows = history.slice(-10).reverse();

  let html = '<table class="pred-history">';
  html += "<thead><tr>"
        + "<th>candle_ts</th>"
        + "<th>1H</th><th>4H</th><th>24H</th>"
        + '<th class="row--scaled" title="Scaled from the row\u2019s 24H prediction: pred_24h / 6.5">1H (scaled)</th>'
        + '<th class="row--scaled" title="Scaled from the row\u2019s 24H prediction: pred_24h * (5.0 / 6.5)">5H (scaled)</th>'
        + "<th>Act 1H</th><th>Act 4H</th><th>Act 24H</th>"
        + "</tr></thead>";
  html += "<tbody>";
  for (const row of rows) {
    html += "<tr>";
    html += "<td>" + fmtTs(row.candle_ts) + "</td>";
    html += '<td class="' + predClass(row.pred_1h) + '">' + fmtPred(row.pred_1h) + "</td>";
    html += '<td class="' + predClass(row.pred_4h) + '">' + fmtPred(row.pred_4h) + "</td>";
    html += '<td class="' + predClass(row.pred_24h) + '">' + fmtPred(row.pred_24h) + "</td>";
    html += '<td class="row--scaled ' + predClass(row.pred_1h_approx) + '">' + fmtPct(row.pred_1h_approx) + "</td>";
    html += '<td class="row--scaled ' + predClass(row.pred_5h_approx) + '">' + fmtPct(row.pred_5h_approx) + "</td>";
    html += '<td class="' + predClass(row.actual_1h) + '">' + fmtPred(row.actual_1h) + "</td>";
    html += '<td class="' + predClass(row.actual_4h) + '">' + fmtPred(row.actual_4h) + "</td>";
    html += '<td class="' + predClass(row.actual_24h) + '">' + fmtPred(row.actual_24h) + "</td>";
    html += "</tr>";
  }
  html += "</tbody></table>";

  container.innerHTML = html;
}

let _topPanelTimer = null;
function startTopPanelPolling() {
  if (_topPanelTimer != null) return;
  _topPanelTimer = setInterval(refreshTopPanel, 5000);
}

export function render(rootEl, data) {
  // Render the history table from the predictions payload (the legacy
  // `predictions` table is still the history source until Wave B is fully
  // wired; it carries the new approx fields on each row).
  if (data) {
    const historyEl = document.getElementById("predictions-history");
    renderHistory(historyEl, data.history);
  }

  // The top-of-panel rows are sourced from /api/status (which exposes
  // pred_1d/5d/21d and the API-computed pred_1h_approx/5h_approx).
  refreshTopPanel();
  startTopPanelPolling();
}
