<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchChart, fetchEquityTrades } from '../api.js';
  import { chartData, status, predictions } from '../stores.js';

  let canvas;
  let container;
  let candles = [];
  let sma = [];
  let trades = [];
  let preds = null;
  let lastClose = null;   // from store (EOD close — last completed candle)
  let livePrice = null;   // from chart's live_quote
  let isStale = false;    // chart candles are >48h old
  let liveQuote = null;   // full live quote for display
  let resizeObserver;
  let chartTimer;

  // ── Data fetching ───────────────────────────────────────────────────────────

  async function refreshChart() {
    try {
      const data = await fetchChart();
      candles = data.candles || [];
      sma = data.sma || [];
      isStale = !!data.stale;

      // Live quote comes bundled in the chart response — no extra round-trip needed.
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

  onMount(async () => {
    await refreshChart();
    await refreshTrades();
    draw();

    resizeObserver = new ResizeObserver(() => draw());
    if (container) resizeObserver.observe(container);

    // Refresh chart + live quote every 30s.
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

  // ── Drawing ───────────────────────────────────────────────────────────────

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

  // ── Drawing ───────────────────────────────────────────────────────────────
  function draw() {
    if (!canvas || !candles.length) return;
    const ctx = canvas.getContext('2d');
    const dpr = window.devicePixelRatio || 1;
    const w = container.clientWidth;
    const h = container.clientHeight || 300;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    canvas.style.width = w + 'px';
    canvas.style.height = h + 'px';
    ctx.scale(dpr, dpr);

    ctx.clearRect(0, 0, w, h);

    const padL = 56, padR = 12, padT = 36, padB = 22;
    const cw = w - padL - padR;
    const ch = h - padT - padB;

    // ── Price range ──────────────────────────────────────────────────────
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

    // Prediction targets expand the visible range
    const priceBase = livePrice ?? lastClose;
    if (preds && priceBase != null) {
      for (const [label, pred] of [
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

    const range = maxP - minP || 1;
    minP -= range * 0.06;
    maxP += range * 0.06;

    // When candle data is stale, re-center the chart around the live price.
    // The stale candles still render but the visible range is anchored to today.
    if (isStale && livePrice != null) {
      const half = (maxP - minP) / 2;
      minP = livePrice - half;
      maxP = livePrice + half;
    }

    const n = candles.length;
    const xStep = cw / n;
    const yScale = (price) => padT + ch - ((price - minP) / (maxP - minP)) * ch;

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

    // ── Grid lines ────────────────────────────────────────────────────────
    ctx.strokeStyle = '#1c1d27';
    ctx.lineWidth = 1;
    ctx.font = '10px monospace';
    ctx.fillStyle = '#5c5e6e';
    ctx.textAlign = 'right';
    for (let i = 0; i <= 4; i++) {
      const y = padT + (ch / 4) * i;
      ctx.beginPath();
      ctx.moveTo(padL, y);
      ctx.lineTo(w - padR, y);
      ctx.stroke();
      const price = maxP - ((maxP - minP) / 4) * i;
      ctx.fillText(price.toFixed(2), padL - 4, y + 3);
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

    // ── Live price line ────────────────────────────────────────────────────
    if (livePrice != null) {
      const liveY = yScale(livePrice);
      ctx.setLineDash([4, 3]);
      ctx.strokeStyle = '#7132f5';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(padL, liveY);
      ctx.lineTo(w - padR, liveY);
      ctx.stroke();
      ctx.setLineDash([]);

      // Badge
      const badge = `${livePrice.toFixed(2)}`;
      const bw = ctx.measureText(badge).width + 8;
      const bx = w - padR - bw - 2;
      const by = liveY - 10;
      ctx.fillStyle = '#7132f5';
      ctx.beginPath();
      ctx.roundRect(bx, by, bw, 16, 3);
      ctx.fill();
      ctx.fillStyle = '#ffffff';
      ctx.font = 'bold 10px monospace';
      ctx.textAlign = 'left';
      ctx.fillText(badge, bx + 4, by + 11);
      ctx.textAlign = 'left';
    }

    // ── Prediction cones (from live price or last close) ───────────────────
    if (preds && priceBase != null && n > 0) {
      const lastX = padL + (n - 1) * xStep + xStep / 2;
      const lastY = yScale(priceBase);

      // Projection calendar: 1 trading day ≈ 1 calendar day for simplicity
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

        // Cone line
        ctx.setLineDash([4, 3]);
        ctx.strokeStyle = col + 'aa';
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.moveTo(lastX, lastY);
        ctx.lineTo(endX, endY);
        ctx.stroke();

        // Projected price label
        ctx.fillStyle = col;
        ctx.fillText(
          `${cone.label} ${targetPrice.toFixed(2)}`,
          endX + 2,
          endY + 3
        );

        // Dot at target
        ctx.setLineDash([]);
        ctx.beginPath();
        ctx.arc(endX, endY, 3, 0, Math.PI * 2);
        ctx.fillStyle = col;
        ctx.fill();
      }
      ctx.setLineDash([]);
    }

    // ── Header overlay: live price badge ───────────────────────────────────
    if (livePrice != null) {
      ctx.font = '11px Inter, sans-serif';
      ctx.fillStyle = '#ececf1';
      ctx.textAlign = 'left';
      ctx.fillText('QQQ', padL, 12);
      ctx.font = 'bold 12px Inter, sans-serif';
      ctx.fillText(livePrice.toFixed(2), padL + 32, 12);
    }
  }
</script>

<div class="chart-card" bind:this={container}>
  <div class="chart-label">Price — QQQ OHLC + SMA</div>
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
  <canvas bind:this={canvas}></canvas>
  {#if !candles.length && !liveQuote}
    <div class="chart-loading">Loading chart…</div>
  {/if}
</div>

<style>
  .chart-card {
    position: relative;
    width: 100%;
    height: 300px;
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

  canvas { display: block; }

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
    top: 0.5rem;
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
</style>
