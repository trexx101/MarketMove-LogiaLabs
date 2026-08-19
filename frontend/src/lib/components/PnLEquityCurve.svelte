<script>
  import { onMount, onDestroy } from 'svelte';
  import { status, activeModelId, models } from '../stores.js';
  import { fetchEquityTrades } from '../api.js';

  let canvas;
  let container;
  let pnlHistory = [];
  let resizeObserver;
  let lastModelId = null;

  $: if ($activeModelId && $activeModelId !== lastModelId) {
    lastModelId = $activeModelId;
    pnlHistory = [];
    loadHistory();
    draw();
  }

  $: if ($status && $status.realized_pnl != null) {
    const ts = $status.last_candle_ts || Date.now();
    const pnl = $status.realized_pnl;
    if (pnlHistory.length === 0 || pnlHistory[pnlHistory.length - 1].pnl !== pnl) {
      pnlHistory = [...pnlHistory, { ts, pnl }];
      if (pnlHistory.length > 200) pnlHistory = pnlHistory.slice(-200);
      draw();
    }
  }

  async function loadHistory() {
    try {
      const m = $models.find((mm) => mm.model_id === $activeModelId);
      const sym = m?.primary_symbol || '*';
      const td = await fetchEquityTrades(sym, 200);
      if (td.trades && td.trades.length > 0) {
        const sorted = [...td.trades].reverse();
        let cum = 0;
        const points = [];
        for (const t of sorted) {
          cum += (t.realized_pnl || 0);
          points.push({ ts: t.ts, pnl: cum });
        }
        if (points.length > 0) {
          pnlHistory = points;
          draw();
        }
      }
    } catch (e) {
      console.error('Failed to load PnL history:', e);
    }
  }

  onMount(() => {
    loadHistory();
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

    const padL = 52, padR = 12, padT = 12, padB = 22;
    const cw = w - padL - padR;
    const ch = h - padT - padB;

    if (pnlHistory.length < 2) {
      ctx.fillStyle = '#5c5e6e';
      ctx.font = '12px Inter, sans-serif';
      ctx.fillText('Waiting for PnL data...', w / 2 - 60, h / 2);
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

    // Grid
    ctx.font = '10px monospace';
    ctx.fillStyle = '#5c5e6e';
    for (let i = 0; i <= 4; i++) {
      const y = padT + (ch / 4) * i;
      ctx.strokeStyle = '#1c1d27';
      ctx.beginPath();
      ctx.moveTo(padL, y);
      ctx.lineTo(w - padR, y);
      ctx.stroke();
      const val = maxP - (range / 4) * i;
      ctx.fillText(val.toFixed(2), 2, y + 3);
    }

    // Zero line
    const zeroY = yScale(0);
    ctx.strokeStyle = '#2e2f3d';
    ctx.lineWidth = 1;
    ctx.setLineDash([2, 2]);
    ctx.beginPath();
    ctx.moveTo(padL, zeroY);
    ctx.lineTo(w - padR, zeroY);
    ctx.stroke();
    ctx.setLineDash([]);

    // PnL line
    const lastPnl = pnlHistory[n - 1].pnl;
    ctx.strokeStyle = lastPnl >= 0 ? '#149e61' : '#e5484d';
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
    ctx.fillStyle = lastPnl >= 0 ? 'rgba(20,158,97,0.12)' : 'rgba(229,72,77,0.12)';
    ctx.fill();
  }
</script>

<div class="pnl-card" bind:this={container}>
  <div class="pnl-label">PnL — Realized</div>
  <canvas bind:this={canvas}></canvas>
</div>

<style>
  .pnl-card {
    width: 100%;
    height: 200px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
    position: relative;
  }

  .pnl-label {
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
</style>
