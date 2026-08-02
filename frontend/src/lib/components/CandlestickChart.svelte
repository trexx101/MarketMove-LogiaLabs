<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchChart, fetchEquityTrades } from '../api.js';
  import { chartData, status, predictions } from '../stores.js';

  // ── State ───────────────────────────────────────────────────────────────
  let canvas;
  let container;
  let candles = [];
  let sma = [];
  let trades = [];
  let preds = null;
  let lastClose = null;
  let livePrice = null;
  let isStale = false;
  let liveQuote = null;
  let resizeObserver;
  let chartTimer;

  // Timeframe selector — value is the limit param fed to fetchChart().
  // ~21 trading days/month, ~252/year. "ALL" up to the backend cap of 1500.
  const TIMEFRAMES = [
    { label: '1M',  limit: 21   },
    { label: '3M',  limit: 63   },
    { label: '6M',  limit: 126  },
    { label: '1Y',  limit: 252  },
    { label: 'ALL', limit: 1500 },
  ];
  let activeTimeframe = '6M';

  // For each candle, derive a label string once per draw to avoid recomputing.
  let candleLabels = [];

  // Crosshair / tooltip state
  let hover = null; // { x, y, candle, screenX, screenY }

  // ── Data fetching ───────────────────────────────────────────────────────

  async function refreshChart() {
    try {
      const tf = TIMEFRAMES.find((t) => t.label === activeTimeframe) ?? TIMEFRAMES[2];
      const data = await fetchChart(tf.limit);
      candles = data.candles || [];
      sma = data.sma || [];
      isStale = !!data.stale;

      if (data.live_quote) {
        livePrice = data.live_quote.price;
        liveQuote = data.live_quote;
      }

      chartData.set(data);
      draw();
    } catch (e) {
      console.warn('chart refresh failed:', e.message);
    }
  }

  async function refreshTrades() {
    try {
      const data = await fetchEquityTrades('QQQ', 200);
      trades = data.trades || [];
      draw();
    } catch (e) {
      // silent — no trades yet is fine
    }
  }

  function setTimeframe(label) {
    if (label === activeTimeframe) return;
    activeTimeframe = label;
    refreshChart();
  }

  onMount(async () => {
    await refreshChart();
    await refreshTrades();
    draw();

    resizeObserver = new ResizeObserver(() => draw());
    if (container) resizeObserver.observe(container);

    chartTimer = setInterval(async () => {
      await refreshChart();
      await refreshTrades();
    }, 30_000);
  });

  onDestroy(() => {
    if (resizeObserver) resizeObserver.disconnect();
    if (chartTimer) clearInterval(chartTimer);
  });

  // React to store changes
  $: if ($chartData) {
    candles = $chartData.candles || candles;
    sma = $chartData.sma || sma;
    draw();
  }

  $: if ($status) {
    lastClose = $status.last_close ?? lastClose;
    draw();
  }

  $: if ($predictions) {
    const p = $predictions.latest;
    if (p) {
      preds = {
        pred_1d:  p.pred_1d  ?? p.pred_24h ?? p.pred_1h,
        pred_5d:  p.pred_5d  ?? p.pred_5h_approx,
        pred_21d: p.pred_21d ?? null,
      };
      draw();
    }
  }

  // ── Exposed hooks (called by Dashboard) ─────────────────────────────────
  export function setPredictions(p, close) {
    preds = {
      pred_1d:  p.pred_1d  ?? p.pred_24h ?? p.pred_1h,
      pred_5d:  p.pred_5d  ?? p.pred_5h_approx,
      pred_21d: p.pred_21d ?? null,
    };
    lastClose = close;
    draw();
  }

  export function setLivePrice(price) {
    livePrice = price;
    draw();
  }

  // ── Drawing ─────────────────────────────────────────────────────────────

  // Format a timestamp as a short axis label e.g. "Sep 15" or "Mar 21 '25".
  function formatLabel(ts) {
    const d = new Date(ts);
    const month = d.toLocaleString('en-US', { month: 'short' });
    const day = d.getDate();
    const year = d.getFullYear();
    const sameYear = candleLabels.some((l) => l && l.year === year);
    return sameYear ? `${month} ${day}` : `${month} ${day} '${String(year).slice(2)}`;
  }

  function draw() {
    if (!canvas || !candles.length) return;
    const ctx = canvas.getContext('2d');
    const dpr = window.devicePixelRatio || 1;
    const w = container.clientWidth;
    const h = container.clientHeight || 420;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    canvas.style.width = w + 'px';
    canvas.style.height = h + 'px';
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, w, h);

    // Reserve bottom band for volume subplot, top for header/padding.
    const padL = 56, padR = 70, padT = 36, padB = 28;
    const priceH = (h - padT - padB) * 0.72;
    const volH = (h - padT - padB) * 0.22;
    const gap = (h - padT - padB) * 0.06;
    const priceTop = padT;
    const priceBot = priceTop + priceH;
    const volTop = priceBot + gap;
    const volBot = volTop + volH;
    const cw = w - padL - padR;
    const ch = priceBot - priceTop;

    // ── Cache candle labels (year-aware) once per draw ─────────────────────
    candleLabels = candles.map((c) => {
      const d = new Date(c.ts);
      return { year: d.getFullYear(), label: formatLabel(c.ts) };
    });

    // ── Price range over candles + sma + trades + prediction targets ─────
    let minP = Infinity, maxP = -Infinity;
    for (const c of candles) {
      if (c.low  < minP) minP = c.low;
      if (c.high > maxP) maxP = c.high;
    }
    if (sma.length) {
      for (const s of sma) {
        if (s.value != null) {
          if (s.value < minP) minP = s.value;
          if (s.value > maxP) maxP = s.value;
        }
      }
    }
    for (const t of trades) {
      if (t.price && t.price > 0) {
        if (t.price < minP) minP = t.price;
        if (t.price > maxP) maxP = t.price;
      }
    }

    const priceBase = livePrice ?? lastClose;
    if (preds && priceBase != null) {
      for (const [, pred] of [
        ['pred_1d',  preds.pred_1d],
        ['pred_5d',  preds.pred_5d],
        ['pred_21d', preds.pred_21d],
      ]) {
        if (pred == null) continue;
        const target = priceBase * (1 + pred);
        if (target < minP) minP = target;
        if (target > maxP) maxP = target;
      }
    }

    let range = maxP - minP || 1;
    minP -= range * 0.06;
    maxP += range * 0.06;
    range = maxP - minP;

    if (isStale && livePrice != null) {
      const half = range / 2;
      minP = livePrice - half;
      maxP = livePrice + half;
    }

    const n = candles.length;

    // ── X-scale: reserve future-bar buffer so cones don't clip ────────────
    // The longest cone is 21 bars. Draw candles across (n + 21) slots so the
    // 21D target lands inside the visible canvas, not clipped off the right.
    const futureBars = 21;
    const xSlots = n + futureBars;
    const xStep = cw / xSlots;
    const candleLeft = padL;
    const candleRight = padL + (n - 1) * xStep + xStep; // last candle's right edge
    const futureRight = padL + xSlots * xStep;

    const yScale = (price) => priceTop + ch - ((price - minP) / (maxP - minP)) * ch;

    function tsToX(ts) {
      for (let i = 0; i < n; i++) {
        if (candles[i].ts === ts) return padL + i * xStep + xStep / 2;
      }
      const target = new Date(ts).getTime();
      let bestI = -1, bestDist = Infinity;
      for (let i = 0; i < n; i++) {
        const ct = new Date(candles[i].ts).getTime();
        const d = Math.abs(ct - target);
        if (d < bestDist) { bestDist = d; bestI = i; }
      }
      if (bestI >= 0) return padL + bestI * xStep + xStep / 2;
      return null;
    }

    // ── Price grid + price labels ─────────────────────────────────────────
    ctx.strokeStyle = '#1c1d27';
    ctx.lineWidth = 1;
    ctx.font = '10px monospace';
    ctx.fillStyle = '#5c5e6e';
    ctx.textAlign = 'right';
    for (let i = 0; i <= 4; i++) {
      const y = priceTop + (ch / 4) * i;
      ctx.beginPath();
      ctx.moveTo(padL, y);
      ctx.lineTo(candleRight, y);
      ctx.stroke();
      const price = maxP - (range / 4) * i;
      ctx.fillText(price.toFixed(2), padL - 4, y + 3);
    }
    ctx.textAlign = 'left';

    // ── X-axis: date labels (one per ~10 candles, year change indicator) ──
    const labelStride = Math.max(1, Math.floor(n / 8));
    ctx.textAlign = 'center';
    ctx.fillStyle = '#5c5e6e';
    let lastYear = null;
    for (let i = 0; i < n; i += labelStride) {
      const x = padL + i * xStep + xStep / 2;
      const { label, year } = candleLabels[i];
      const showYear = year !== lastYear;
      lastYear = year;
      ctx.fillStyle = showYear ? '#9c9eae' : '#5c5e6e';
      ctx.fillText(label, x, priceBot + 12);
    }
    ctx.textAlign = 'left';

    // ── Candles ───────────────────────────────────────────────────────────
    const candleW = Math.max(1, xStep * 0.7);
    for (let i = 0; i < n; i++) {
      const c = candles[i];
      const x = padL + i * xStep + xStep / 2;
      const isUp = c.close >= c.open;
      ctx.strokeStyle = isUp ? '#149e61' : '#e5484d';
      ctx.fillStyle = isUp ? '#149e61' : '#e5484d';

      ctx.beginPath();
      ctx.moveTo(x, yScale(c.high));
      ctx.lineTo(x, yScale(c.low));
      ctx.stroke();

      const yO = yScale(c.open);
      const yC = yScale(c.close);
      const bodyTop = Math.min(yO, yC);
      const bodyH = Math.max(1, Math.abs(yC - yO));
      ctx.fillRect(x - candleW / 2, bodyTop, candleW, bodyH);
    }

    // ── SMA line ──────────────────────────────────────────────────────────
    if (sma.length) {
      ctx.strokeStyle = '#7132f5';
      ctx.lineWidth = 1.5;
      ctx.setLineDash([]);
      ctx.beginPath();
      let started = false;
      for (let i = 0; i < sma.length; i++) {
        if (sma[i].value == null) continue;
        const x = padL + i * xStep + xStep / 2;
        const y = yScale(sma[i].value);
        if (!started) { ctx.moveTo(x, y); started = true; }
        else ctx.lineTo(x, y);
      }
      ctx.stroke();
    }

    // ── Volume subplot ────────────────────────────────────────────────────
    let maxVol = 0;
    for (const c of candles) if (c.volume > maxVol) maxVol = c.volume;
    if (maxVol > 0) {
      const volScale = (v) => volBot - (v / maxVol) * volH;
      const barW = Math.max(1, xStep * 0.7);
      for (let i = 0; i < n; i++) {
        const c = candles[i];
        const x = padL + i * xStep + xStep / 2;
        const isUp = c.close >= c.open;
        ctx.fillStyle = isUp ? 'rgba(20, 158, 97, 0.55)' : 'rgba(229, 72, 77, 0.55)';
        const yT = volScale(c.volume);
        ctx.fillRect(x - barW / 2, yT, barW, volBot - yT);
      }
      // Volume axis label
      ctx.fillStyle = '#5c5e6e';
      ctx.font = '9px monospace';
      ctx.textAlign = 'right';
      ctx.fillText('VOL', padL - 4, volTop + 9);
      ctx.textAlign = 'left';
    }

    // ── Trade markers ─────────────────────────────────────────────────────
    if (trades.length) {
      ctx.font = '9px monospace';
      for (const t of trades) {
        const x = tsToX(t.ts);
        if (x == null) continue;
        const y = yScale(t.price);
        const isLong = t.side === 'long';
        const color = isLong ? '#149e61' : '#e5484d';

        ctx.strokeStyle = color;
        ctx.fillStyle = color;
        ctx.lineWidth = 1.5;
        ctx.setLineDash([]);

        if (isLong) {
          ctx.beginPath();
          ctx.moveTo(x, y - 6);
          ctx.lineTo(x - 3, y + 3);
          ctx.lineTo(x + 3, y + 3);
          ctx.closePath();
          ctx.fill();
          ctx.stroke();
        } else {
          ctx.beginPath();
          ctx.moveTo(x, y + 6);
          ctx.lineTo(x - 3, y - 3);
          ctx.lineTo(x + 3, y - 3);
          ctx.closePath();
          ctx.fill();
          ctx.stroke();
        }
        ctx.fillStyle = color;
        ctx.fillText(isLong ? 'L' : 'S', x + 5, y - 4);
      }
    }

    // ── Live price line ───────────────────────────────────────────────────
    if (livePrice != null) {
      const liveY = yScale(livePrice);
      ctx.setLineDash([4, 3]);
      ctx.strokeStyle = '#7132f5';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(padL, liveY);
      ctx.lineTo(futureRight, liveY);
      ctx.stroke();
      ctx.setLineDash([]);
      // Right-edge price tag
      ctx.fillStyle = '#7132f5';
      ctx.fillRect(futureRight + 2, liveY - 8, 60, 16);
      ctx.fillStyle = '#fff';
      ctx.textAlign = 'left';
      ctx.font = '10px monospace';
      ctx.fillText(livePrice.toFixed(2), futureRight + 6, liveY + 3);
      ctx.textAlign = 'left';
    }

    // ── Prediction cones (now inside the canvas thanks to xSlots buffer) ──
    if (preds && priceBase != null && n > 0) {
      const lastX = padL + (n - 1) * xStep + xStep / 2;
      const lastY = yScale(priceBase);

      const cones = [
        { label: '1D',  pred: preds.pred_1d,  bars: 1  },
        { label: '5D',  pred: preds.pred_5d,  bars: 5  },
        { label: '21D', pred: preds.pred_21d, bars: 21 },
      ];

      ctx.font = '9px monospace';
      for (const cone of cones) {
        if (cone.pred == null) continue;
        const targetPrice = priceBase * (1 + cone.pred);
        const endX = lastX + cone.bars * xStep;
        const endY = yScale(targetPrice);
        const col = cone.pred >= 0 ? '#149e61' : '#e5484d';

        ctx.setLineDash([4, 3]);
        ctx.strokeStyle = col + 'aa';
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.moveTo(lastX, lastY);
        ctx.lineTo(endX, endY);
        ctx.stroke();

        ctx.fillStyle = col;
        ctx.fillText(
          `${cone.label} ${targetPrice.toFixed(2)}`,
          endX + 2,
          endY + 3
        );

        ctx.setLineDash([]);
        ctx.beginPath();
        ctx.arc(endX, endY, 3, 0, Math.PI * 2);
        ctx.fillStyle = col;
        ctx.fill();
      }
      ctx.setLineDash([]);
    }

    // ── Crosshair (drawn last so it sits on top) ──────────────────────────
    if (hover) {
      ctx.save();
      ctx.strokeStyle = 'rgba(113, 50, 245, 0.4)';
      ctx.lineWidth = 1;
      ctx.setLineDash([2, 2]);
      ctx.beginPath();
      ctx.moveTo(hover.x, priceTop);
      ctx.lineTo(hover.x, volBot);
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(padL, hover.y);
      ctx.lineTo(futureRight, hover.y);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.restore();
    }
  }

  // ── Mouse handlers for crosshair + tooltip ──────────────────────────────
  function onMouseMove(ev) {
    if (!canvas || !candles.length) return;
    const rect = canvas.getBoundingClientRect();
    const x = ev.clientX - rect.left;
    const y = ev.clientY - rect.top;

    const padL = 56, padR = 70;
    const w = container.clientWidth;
    const cw = w - padL - padR;
    const n = candles.length;
    const xSlots = n + 21;
    const xStep = cw / xSlots;

    const i = Math.floor((x - padL) / xStep);
    if (i < 0 || i >= n) {
      hover = null;
      draw();
      return;
    }

    const c = candles[i];
    const candleX = padL + i * xStep + xStep / 2;
    hover = { x: candleX, y, candle: c, i };
    draw();
  }

  function onMouseLeave() {
    if (hover) {
      hover = null;
      draw();
    }
  }
</script>

<div class="chart-card" bind:this={container}>
  <div class="chart-label">Price — QQQ OHLC + SMA + Volume</div>
  {#if liveQuote}
    <div class="quote-badge" class:stale={isStale}>
      <span class="quote-price">{liveQuote.price.toFixed(2)}</span>
      <span class="quote-change" class:up={liveQuote.change >= 0} class:down={liveQuote.change < 0}>
        {liveQuote.change >= 0 ? '+' : ''}{liveQuote.change.toFixed(2)}
        ({liveQuote.change_pct >= 0 ? '+' : ''}{liveQuote.change_pct.toFixed(2)}%)
      </span>
      {#if isStale}
        <span class="stale-tag">stale data</span>
      {/if}
    </div>
  {/if}

  <div class="tf-bar">
    {#each TIMEFRAMES as tf}
      <button
        class="tf-btn"
        class:active={activeTimeframe === tf.label}
        on:click={() => setTimeframe(tf.label)}
        title="Last {tf.label} of candles"
      >
        {tf.label}
      </button>
    {/each}
  </div>

  <canvas
    bind:this={canvas}
    on:mousemove={onMouseMove}
    on:mouseleave={onMouseLeave}
  ></canvas>

  {#if hover && hover.candle}
    <div class="tooltip" style="left: {hover.x + 12}px; top: {hover.y - 12}px;">
      <div class="tt-date">{new Date(hover.candle.ts).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}</div>
      <div class="tt-row"><span>O</span> {hover.candle.open.toFixed(2)}</div>
      <div class="tt-row"><span>H</span> {hover.candle.high.toFixed(2)}</div>
      <div class="tt-row"><span>L</span> {hover.candle.low.toFixed(2)}</div>
      <div class="tt-row"><span>C</span> {hover.candle.close.toFixed(2)}</div>
      <div class="tt-row vol"><span>V</span> {Math.round(hover.candle.volume).toLocaleString()}</div>
    </div>
  {/if}

  {#if !candles.length && !liveQuote}
    <div class="chart-loading">Loading chart…</div>
  {/if}
</div>

<style>
  .chart-card {
    position: relative;
    width: 100%;
    height: 420px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .chart-label {
    position: absolute;
    top: 0.6rem;
    left: 0.75rem;
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
    font-weight: 600;
    z-index: 1;
    pointer-events: none;
  }

  canvas { display: block; cursor: crosshair; }

  .chart-loading {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    color: var(--text-secondary);
    font-size: 0.82rem;
  }

  .quote-badge {
    position: absolute;
    top: 1.85rem; /* below the chart-label so they don't collide */
    left: 0.75rem;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    z-index: 2;
    pointer-events: none;
  }

  .quote-price {
    font-size: 1rem;
    font-weight: 700;
    color: var(--text-primary);
    font-variant-numeric: tabular-nums;
  }

  .quote-change {
    font-size: 0.78rem;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }

  .quote-change.up { color: var(--green); }
  .quote-change.down { color: var(--red); }

  .stale-tag {
    font-size: 0.65rem;
    color: var(--orange);
    background: rgba(249, 160, 24, 0.12);
    border: 1px solid rgba(249, 160, 24, 0.3);
    border-radius: 3px;
    padding: 0.1rem 0.4rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .tf-bar {
    position: absolute;
    top: 0.5rem;
    right: 0.75rem;
    display: flex;
    gap: 0.2rem;
    z-index: 2;
  }

  .tf-btn {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-secondary);
    font-size: 0.66rem;
    font-weight: 600;
    padding: 0.18rem 0.5rem;
    border-radius: 3px;
    cursor: pointer;
    font-family: inherit;
    letter-spacing: 0.03em;
    transition: all 0.12s ease;
  }

  .tf-btn:hover {
    color: var(--text-primary);
    border-color: var(--accent);
  }

  .tf-btn.active {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }

  .tooltip {
    position: absolute;
    background: rgba(20, 21, 28, 0.95);
    border: 1px solid var(--accent);
    border-radius: 4px;
    padding: 0.4rem 0.55rem;
    font-size: 0.7rem;
    font-family: monospace;
    color: var(--text-primary);
    z-index: 3;
    pointer-events: none;
    min-width: 130px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
  }

  .tt-date {
    color: var(--text-secondary);
    margin-bottom: 0.25rem;
    font-weight: 600;
    border-bottom: 1px solid var(--border);
    padding-bottom: 0.2rem;
  }

  .tt-row {
    display: flex;
    justify-content: space-between;
    gap: 0.6rem;
    line-height: 1.4;
  }

  .tt-row span {
    color: var(--text-secondary);
    font-weight: 600;
  }

  .tt-row.vol {
    margin-top: 0.2rem;
    padding-top: 0.2rem;
    border-top: 1px solid var(--border);
    color: var(--text-secondary);
  }
</style>
