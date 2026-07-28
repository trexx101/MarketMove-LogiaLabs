<script>
  import { onMount, onDestroy } from 'svelte';
  import { status } from '../stores.js';

  let canvas;
  let container;
  let pnlHistory = [];
  let resizeObserver;

  // React to status store updates (WS PnlTick events)
  $: if ($status && $status.realized_pnl != null) {
    const ts = $status.last_candle_ts || Date.now();
    const pnl = $status.realized_pnl;
    // Only push if changed
    if (pnlHistory.length === 0 || pnlHistory[pnlHistory.length - 1].pnl !== pnl) {
      pnlHistory = [...pnlHistory, { ts, pnl }];
      if (pnlHistory.length > 200) pnlHistory = pnlHistory.slice(-200);
      draw();
    }
  }

  onMount(() => {
    draw();
    resizeObserver = new ResizeObserver(() => draw());
    if (container) resizeObserver.observe(container);
  });

  onDestroy(() => {
    if (resizeObserver) resizeObserver.disconnect();
  });

  function draw() {
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    const dpr = window.devicePixelRatio || 1;
    const w = container.clientWidth;
    const h = container.clientHeight || 200;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    canvas.style.width = w + 'px';
    canvas.style.height = h + 'px';
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, w, h);

    const padL = 50, padR = 10, padT = 10, padB = 20;
    const cw = w - padL - padR;
    const ch = h - padT - padB;

    if (pnlHistory.length < 2) {
      ctx.fillStyle = '#8b949e';
      ctx.font = '12px sans-serif';
      ctx.fillText('Waiting for PnL data…', w / 2 - 60, h / 2);
      return;
    }

    let minP = Infinity, maxP = -Infinity;
    for (const p of pnlHistory) {
      if (p.pnl < minP) minP = p.pnl;
      if (p.pnl > maxP) maxP = p.pnl;
    }
    if (minP > 0) minP = 0;
    if (maxP < 0) maxP = 0;
    const range = maxP - minP || 1;

    const n = pnlHistory.length;
    const xStep = cw / (n - 1);
    const yScale = (v) => padT + ch - ((v - minP) / range) * ch;

    // Zero line
    const zeroY = yScale(0);
    ctx.strokeStyle = '#30363d';
    ctx.lineWidth = 1;
    ctx.setLineDash([2, 2]);
    ctx.beginPath();
    ctx.moveTo(padL, zeroY);
    ctx.lineTo(w - padR, zeroY);
    ctx.stroke();
    ctx.setLineDash([]);

    // Grid + labels
    ctx.font = '10px monospace';
    ctx.fillStyle = '#484f58';
    for (let i = 0; i <= 4; i++) {
      const y = padT + (ch / 4) * i;
      ctx.strokeStyle = '#21262d';
      ctx.beginPath();
      ctx.moveTo(padL, y);
      ctx.lineTo(w - padR, y);
      ctx.stroke();
      const val = maxP - (range / 4) * i;
      ctx.fillText(val.toFixed(2), 2, y + 3);
    }

    // PnL line
    const lastPnl = pnlHistory[n - 1].pnl;
    ctx.strokeStyle = lastPnl >= 0 ? '#3fb950' : '#f85149';
    ctx.lineWidth = 2;
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const x = padL + i * xStep;
      const y = yScale(pnlHistory[i].pnl);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();

    // Fill area
    ctx.lineTo(padL + (n - 1) * xStep, zeroY);
    ctx.lineTo(padL, zeroY);
    ctx.closePath();
    ctx.fillStyle = lastPnl >= 0 ? '#3fb95022' : '#f8514922';
    ctx.fill();
  }
</script>

<div class="pnl-container" bind:this={container}>
  <canvas bind:this={canvas}></canvas>
</div>

<style>
  .pnl-container {
    width: 100%;
    height: 200px;
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 8px;
    overflow: hidden;
  }
  canvas { display: block; }
</style>
