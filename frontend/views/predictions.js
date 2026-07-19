/**
 * Predictions view — renders latest predictions and recent history.
 *
 * @param {HTMLElement} rootEl  — #predictions-body (latest) + #predictions-history
 * @param {object}      data    — response from GET /api/predictions
 */

function fmtPred(val) {
  if (val == null) return "—";
  return Number(val).toFixed(6);
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

function renderHistory(container, history) {
  if (!container) return;

  if (!history || history.length === 0) {
    container.innerHTML = "";
    return;
  }

  const rows = history.slice(-10).reverse();

  let html = '<table class="pred-history">';
  html += "<thead><tr><th>candle_ts</th><th>1H</th><th>4H</th><th>24H</th><th>Act 1H</th><th>Act 4H</th><th>Act 24H</th></tr></thead>";
  html += "<tbody>";
  for (const row of rows) {
    html += "<tr>";
    html += "<td>" + fmtTs(row.candle_ts) + "</td>";
    html += '<td class="' + predClass(row.pred_1h) + '">' + fmtPred(row.pred_1h) + "</td>";
    html += '<td class="' + predClass(row.pred_4h) + '">' + fmtPred(row.pred_4h) + "</td>";
    html += '<td class="' + predClass(row.pred_24h) + '">' + fmtPred(row.pred_24h) + "</td>";
    html += '<td class="' + predClass(row.actual_1h) + '">' + fmtPred(row.actual_1h) + "</td>";
    html += '<td class="' + predClass(row.actual_4h) + '">' + fmtPred(row.actual_4h) + "</td>";
    html += '<td class="' + predClass(row.actual_24h) + '">' + fmtPred(row.actual_24h) + "</td>";
    html += "</tr>";
  }
  html += "</tbody></table>";

  container.innerHTML = html;
}

export function render(rootEl, data) {
  if (!data) return;

  const latest = data.latest;

  if (latest) {
    setText("p-1h", fmtPred(latest.pred_1h), predClass(latest.pred_1h));
    setText("p-4h", fmtPred(latest.pred_4h), predClass(latest.pred_4h));
    setText("p-24h", fmtPred(latest.pred_24h), predClass(latest.pred_24h));
  }

  const historyEl = document.getElementById("predictions-history");
  renderHistory(historyEl, data.history);
}
