/**
 * Status view — renders engine status into the status panel.
 *
 * @param {HTMLElement} rootEl  — #status-body
 * @param {object}      data    — response from GET /api/status
 */

function fmtPrice(val) {
  if (val == null) return "—";
  return "$" + Number(val).toLocaleString("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
}

function fmtPnl(val) {
  if (val == null) return "—";
  const n = Number(val);
  const sign = n >= 0 ? "+" : "";
  return sign + "$" + n.toLocaleString("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
}

function pnlClass(val) {
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

export function render(rootEl, data) {
  if (!data) return;

  setText("st-mode", data.mode || "—");
  setText("st-symbol", data.symbol || "—");
  setText("st-last", data.last_candle_ts || "—");
  setText("st-close", fmtPrice(data.last_close));
  setText("st-position", data.position || "—");
  setText("st-entry", fmtPrice(data.entry_price));

  setText("st-realized", fmtPnl(data.realized_pnl), pnlClass(data.realized_pnl));
  setText("st-unrealized", fmtPnl(data.unrealized_pnl), pnlClass(data.unrealized_pnl));
}
