<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchChart } from '../api.js';
  import { chartData } from '../stores.js';

  let canvas;
  let container;
  let candles = [];
  let sma = [];
  let preds = null;
  let lastClose = null;
  let error = null;
  let resizeObserver;

  onMount(async () => {
    try {
      const data = await fetchChart();
      candles = data.candles || [];
      sma = data.sma || [];
      chartData.set(data);
    } catch (e) {
      error = e.message;
    }
    draw();

    resizeObserver = new ResizeObserver(() => draw());
    if (container) resizeObserver.observe(container);
  });

  onDestroy(() => {
    if (resizeObserver) resizeObserver.disconnect();
  });

  // Redraw when chartData store changes (e.g. from WS)
  $: if ($chartData) {
    candles = $chartData.candles || candles;
    sma = $chartData.sma || sma;
    draw();
  }

  export function setPredictions(p, close) {
    preds = p;
    lastClose = close;
    draw();
  }

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

    const padL = 50, padR = 10, padT = 10, padB = 20;
    const cw = w - padL - padR;
    const ch = h - padT - padB;

    // Price range
    let minP = Infinity, maxP = -Infinity;
    for (const c of candles) {
      if (c.low < minP) minP = c.low;
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
    const range = maxP - minP || 1;
    minP -= range * 0.05;
    maxP += range * 0.05;

    const n = candles.length;
    const xStep = cw / n;
    const yScale = (price) => padT + ch - ((price - minP) / (maxP - minP)) * ch;

    // Grid lines
    ctx.strokeStyle = '#21262d';
    ctx.lineWidth = 1;
    ctx.font = '10px monospace';
    ctx.fillStyle = '#484f58';
    for (let i = 0; i <= 4; i++) {
      const y = padT + (ch / 4) * i;
      ctx.beginPath();
      ctx.moveTo(padL, y);
      ctx.lineTo(w - padR, y);
      ctx.stroke();
      const price = maxP - ((maxP - minP) / 4) * i;
      ctx.fillText(price.toFixed(2), 2, y + 3);
    }

    // Candles
    const candleW = Math.max(1, xStep * 0.7);
    for (let i = 0; i < n; i++) {
      const c = candles[i];
      const x = padL + i * xStep + xStep / 2;
      const isUp = c.close >= c.open;
      ctx.strokeStyle = isUp ? '#3fb950' : '#f85149';
      ctx.fillStyle = isUp ? '#3fb950' : '#f85149';

      // Wick
      ctx.beginPath();
      ctx.moveTo(x, yScale(c.high));
      ctx.lineTo(x, yScale(c.low));
      ctx.stroke();

      // Body
      const yO = yScale(c.open);
      const yC = yScale(c.close);
      const bodyTop = Math.min(yO, yC);
      const bodyH = Math.max(1, Math.abs(yC - yO));
      ctx.fillRect(x - candleW / 2, bodyTop, candleW, bodyH);
    }

    // SMA line
    if (sma.length) {
      ctx.strokeStyle = '#58a6ff';
      ctx.lineWidth = 1.5;
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

    // Prediction cones
    if (preds && lastClose != null && n > 0) {
      const lastX = padL + (n - 1) * xStep + xStep / 2;
      const lastY = yScale(lastClose);
      const cones = [
        { label: '1D', pred: preds.pred_1d, bars: 1 },
        { label: '5D', pred: preds.pred_5d, bars: 5 },
        { label: '21D', pred: preds.pred_21d, bars: 21 },
      ];
      ctx.setLineDash([4, 3]);
      ctx.font = '9px monospace';
      for (const cone of cones) {
        if (cone.pred == null) continue;
        const targetPrice = lastClose * (1 + cone.pred);
        const endX = lastX + cone.bars * xStep;
        const endY = yScale(targetPrice);
        ctx.strokeStyle = cone.pred >= 0 ? '#3fb95088' : '#f8514988';
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(lastX, lastY);
        ctx.lineTo(endX, endY);
        ctx.stroke();
        ctx.fillStyle = cone.pred >= 0 ? '#3fb950' : '#f85149';
        ctx.fillText(cone.label, endX + 2, endY);
      }
      ctx.setLineDash([]);
    }
  }
</script>

<div class="chart-container" bind:this={container}>
  {#if error}
    <div class="chart-error">Chart: {error}</div>
  {/if}
  <canvas bind:this={canvas}></canvas>
  {#if !candles.length && !error}
    <div class="chart-loading">Loading chart…</div>
  {/if}
</div>

<style>
  .chart-container {
    position: relative;
    width: 100%;
    height: 300px;
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 8px;
    overflow: hidden;
  }
  canvas { display: block; }
  .chart-loading, .chart-error {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    color: #8b949e;
    font-size: 0.85rem;
  }
  .chart-error { color: #f85149; }
</style>
