/**
 * Accuracy view — renders prediction accuracy metrics into the accuracy panel.
 *
 * @param {HTMLElement} rootEl  — #accuracy-body
 * @param {object|null} data    — response from GET /api/accuracy, or null on error
 */

function setText(id, text, className) {
  const el = document.getElementById(id);
  if (!el) return;
  el.textContent = text;
  if (className) {
    el.className = "v " + className;
  }
}

function fmtPct(val) {
  if (val == null) return "—";
  // directional accuracy comes in 0..1 — format as percentage
  const pct = Number(val) * 100;
  return pct.toFixed(0) + "%";
}

function fmtMae(val) {
  if (val == null) return "—";
  return Number(val).toFixed(6);
}

export function render(rootEl, data) {
  if (!data || !data.resolved_count || data.resolved_count === 0) {
    setText("acc-directional", "No resolved predictions yet", "val-neutral");
    setText("acc-mae", "—", "val-neutral");
    setText("acc-resolved", "0", "val-neutral");
    return;
  }

  const dirStr =
    "1H: " + fmtPct(data.directional_1h) +
    " | 4H: " + fmtPct(data.directional_4h) +
    " | 24H: " + fmtPct(data.directional_24h);
  setText("acc-directional", dirStr, "val-neutral");

  const maeStr =
    "1H: " + fmtMae(data.mae_1h) +
    " | 4H: " + fmtMae(data.mae_4h) +
    " | 24H: " + fmtMae(data.mae_24h);
  setText("acc-mae", maeStr, "val-neutral");

  setText("acc-resolved", String(data.resolved_count), "val-neutral");
}
