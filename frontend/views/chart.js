/**
 * Chart view — renders OHLCV candlestick chart with 200-SMA overlay using uPlot.
 *
 * @param {HTMLElement} rootEl  — #chart-container
 * @param {object}      data    — response from GET /api/chart
 */

let chart = null;
let resizeObserver = null;

const COLOR_UP = "#3fb950";
const COLOR_DOWN = "#f85149";
const COLOR_SMA = "#58a6ff";
const COLOR_GRID = "#30363d";
const COLOR_TEXT = "#8b949e";

/**
 * Draw candlestick bodies + wicks directly on the uPlot canvas.
 * Attached as a series `draw` hook so it fires each frame after scale computation.
 */
function drawCandles(u) {
  const xs = u.data[0];
  const opens = u.data[1];
  const highs = u.data[2];
  const lows = u.data[3];
  const closes = u.data[4];

  if (!xs || xs.length === 0) return;

  const dpr = window.devicePixelRatio || 1;
  const plotLeft = u.bbox.left / dpr;
  const plotTop = u.bbox.top / dpr;
  const plotW = u.bbox.width / dpr;
  const plotH = u.bbox.height / dpr;

  const candleW = Math.max(1, Math.min(8, (plotW / xs.length) * 0.6));

  const ctx = u.ctx;
  ctx.save();
  ctx.beginPath();
  ctx.rect(plotLeft, plotTop, plotW, plotH);
  ctx.clip();

  for (let i = 0; i < xs.length; i++) {
    if (opens[i] == null || closes[i] == null) continue;

    const x = u.valToPos(xs[i], "x");
    const yO = u.valToPos(opens[i], "y");
    const yH = u.valToPos(highs[i], "y");
    const yL = u.valToPos(lows[i], "y");
    const yC = u.valToPos(closes[i], "y");

    const isUp = closes[i] >= opens[i];
    const color = isUp ? COLOR_UP : COLOR_DOWN;

    // Wick
    ctx.strokeStyle = color;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(x, yH);
    ctx.lineTo(x, yL);
    ctx.stroke();

    // Body
    const bodyTop = Math.min(yO, yC);
    const bodyH = Math.max(Math.abs(yC - yO), 1);
    ctx.fillStyle = color;
    ctx.fillRect(x - candleW / 2, bodyTop, candleW, bodyH);
  }

  ctx.restore();
}

/** Return empty paths for invisible scale-contributing series. */
function emptyPaths() {
  return { stroke: new Path2D(), fill: new Path2D() };
}

/** Build uPlot data arrays from API response. */
function buildData(candles, sma) {
  const xs = [];
  const opens = [];
  const highs = [];
  const lows = [];
  const closes = [];

  for (const c of candles) {
    xs.push(Math.floor(new Date(c.ts).getTime() / 1000));
    opens.push(c.open);
    highs.push(c.high);
    lows.push(c.low);
    closes.push(c.close);
  }

  // Build SMA array aligned to xs (null for missing timestamps)
  const smaMap = new Map();
  for (const s of sma) {
    smaMap.set(Math.floor(new Date(s.ts).getTime() / 1000), s.value);
  }
  const smaVals = xs.map((t) => smaMap.get(t) ?? null);

  return [xs, opens, highs, lows, closes, smaVals];
}

/** Create or update the uPlot chart instance. */
function updateChart(container, candles, sma) {
  const UPlot = window.uPlot;
  if (!UPlot) return;

  const data = buildData(candles, sma);
  const width = container.clientWidth || 600;
  const height = 340;

  const series = [
    {},
    {
      label: "O",
      stroke: "rgba(0,0,0,0)",
      fill: "rgba(0,0,0,0)",
      width: 0,
      points: { show: false },
      paths: emptyPaths,
    },
    {
      label: "H",
      stroke: "rgba(0,0,0,0)",
      fill: "rgba(0,0,0,0)",
      width: 0,
      points: { show: false },
      paths: emptyPaths,
    },
    {
      label: "L",
      stroke: "rgba(0,0,0,0)",
      fill: "rgba(0,0,0,0)",
      width: 0,
      points: { show: false },
      paths: emptyPaths,
    },
    {
      label: "C",
      stroke: "rgba(0,0,0,0)",
      fill: "rgba(0,0,0,0)",
      width: 0,
      points: { show: false },
      paths: emptyPaths,
      hooks: { draw: [drawCandles] },
    },
  ];

  // Add SMA series only if we have SMA data
  const hasSma = sma && sma.length > 0;
  if (hasSma) {
    series.push({
      label: "SMA 200",
      stroke: COLOR_SMA,
      width: 1.5,
      points: { show: false },
    });
  }

  const opts = {
    width,
    height,
    cursor: { show: true },
    scales: {
      x: { time: true },
    },
    axes: [
      {
        stroke: COLOR_TEXT,
        grid: { stroke: COLOR_GRID, width: 1 },
        ticks: { stroke: COLOR_GRID, width: 1 },
      },
      {
        stroke: COLOR_TEXT,
        grid: { stroke: COLOR_GRID, width: 1 },
        ticks: { stroke: COLOR_GRID, width: 1 },
      },
    ],
    series,
  };

  if (chart) {
    chart.destroy();
    chart = null;
  }

  chart = new UPlot(opts, data, container);
}

function showPlaceholder(container, msg) {
  container.innerHTML = '<div class="chart-placeholder">' + msg + "</div>";
}

export function render(rootEl, data) {
  if (!rootEl) return;

  // Render into the inner container (not the panel wrapper)
  const container = rootEl.querySelector("#chart-container") || rootEl;

  const candles = data && data.candles ? data.candles : [];
  const sma = data && data.sma ? data.sma : [];

  if (candles.length === 0) {
    if (chart) {
      chart.destroy();
      chart = null;
    }
    showPlaceholder(container, "pending — no data yet");
    return;
  }

  // Ensure container is clean (remove placeholder if present)
  const placeholder = container.querySelector(".chart-placeholder");
  if (placeholder) placeholder.remove();

  updateChart(container, candles, sma);

  // Set up resize observer if not already done
  if (!resizeObserver && typeof ResizeObserver !== "undefined") {
    resizeObserver = new ResizeObserver(() => {
      if (chart && container.clientWidth > 0) {
        chart.setSize(container.clientWidth, 340);
      }
    });
    resizeObserver.observe(container);
  }
}
