<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchEquityData } from '../lib/api.js';

  let canvas;
  let container;
  let rows = [];
  let symbol = 'QQQ';
  let loading = true;
  let error = null;
  let resizeObserver;

  onMount(async () => {
    await loadData();
    resizeObserver = new ResizeObserver(() => drawCurve());
    if (container) resizeObserver.observe(container);
  });

  onDestroy(() => {
    if (resizeObserver) resizeObserver.disconnect();
  });

  async function loadData() {
    loading = true;
    error = null;
    try {
      const data = await fetchEquityData(symbol, 500);
      rows = data.data || [];
    } catch (e) {
      error = e.message;
    }
    loading = false;
    drawCurve();
  }

  function fmtDate(ts) {
    if (!ts) return '—';
    try {
      const d = new Date(ts);
      return d.toISOString().slice(0, 10);
    } catch {
      return String(ts);
    }
  }

  function fmt(v, dec = 2) {
    if (v == null || isNaN(v)) return '—';
    return Number(v).toFixed(dec);
  }

  function fmtVol(v) {
    if (v == null || isNaN(v)) return '—';
    if (v >= 1e6) return (v / 1e6).toFixed(1) + 'M';
    if (v >= 1e3) return (v / 1e3).toFixed(0) + 'K';
    return String(v);
  }

  function drawCurve() {
    if (!canvas || !rows.length) return;
    const ctx = canvas.getContext('2d');
    const dpr = window.devicePixelRatio || 1;
    const w = container.clientWidth;
    const h = container.clientHeight || 150;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    canvas.style.width = w + 'px';
    canvas.style.height = h + 'px';
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, w, h);

    const padL = 50, padR = 10, padT = 10, padB = 20;
    const cw = w - padL - padR;
    const ch = h - padT - padB;

    // Cumulative equity from close prices (normalized to start at 0%)
    const closes = rows.map((r) => r.close).filter((c) => c != null);
    if (closes.length < 2) return;

    const start = closes[0];
    const equity = closes.map((c) => ((c - start) / start) * 100);

    let minE = Math.min(...equity, 0);
    let maxE = Math.max(...equity, 0);
    const range = maxE - minE || 1;

    const n = equity.length;
    const xStep = cw / (n - 1);
    const yScale = (v) => padT + ch - ((v - minE) / range) * ch;

    // Grid
    ctx.font = '10px monospace';
    ctx.fillStyle = '#484f58';
    ctx.strokeStyle = '#21262d';
    for (let i = 0; i <= 4; i++) {
      const y = padT + (ch / 4) * i;
      ctx.beginPath();
      ctx.moveTo(padL, y);
      ctx.lineTo(w - padR, y);
      ctx.stroke();
      const val = maxE - (range / 4) * i;
      ctx.fillText(val.toFixed(1) + '%', 2, y + 3);
    }

    // Zero line
    const zeroY = yScale(0);
    ctx.strokeStyle = '#484f58';
    ctx.setLineDash([2, 2]);
    ctx.beginPath();
    ctx.moveTo(padL, zeroY);
    ctx.lineTo(w - padR, zeroY);
    ctx.stroke();
    ctx.setLineDash([]);

    // Equity line
    const lastE = equity[n - 1];
    ctx.strokeStyle = lastE >= 0 ? '#3fb950' : '#f85149';
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const x = padL + i * xStep;
      const y = yScale(equity[i]);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();

    // Fill
    ctx.lineTo(padL + (n - 1) * xStep, zeroY);
    ctx.lineTo(padL, zeroY);
    ctx.closePath();
    ctx.fillStyle = lastE >= 0 ? '#3fb95022' : '#f8514922';
    ctx.fill();
  }
</script>

<div class="ledger">
  <div class="ledger-header">
    <h2>Ledger — {symbol}</h2>
    <div class="controls">
      <input bind:value={symbol} placeholder="Symbol" on:keydown={(e) => e.key === 'Enter' && loadData()} />
      <button on:click={loadData}>Load</button>
    </div>
  </div>

  {#if error}
    <div class="error">Error: {error}</div>
  {/if}

  <div class="equity-curve-container" bind:this={container}>
    <canvas bind:this={canvas}></canvas>
    {#if loading}
      <div class="loading">Loading…</div>
    {/if}
  </div>

  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          <th>Date</th>
          <th>Open</th>
          <th>High</th>
          <th>Low</th>
          <th>Close</th>
          <th>Volume</th>
        </tr>
      </thead>
      <tbody>
        {#if loading}
          <tr><td colspan="6" class="center">Loading…</td></tr>
        {:else if rows.length === 0}
          <tr><td colspan="6" class="center">No data</td></tr>
        {:else}
          {#each rows as r}
            <tr>
              <td class="mono">{fmtDate(r.ts)}</td>
              <td class="mono">{fmt(r.open)}</td>
              <td class="mono">{fmt(r.high)}</td>
              <td class="mono">{fmt(r.low)}</td>
              <td class="mono">{fmt(r.close)}</td>
              <td class="mono">{fmtVol(r.volume)}</td>
            </tr>
          {/each}
        {/if}
      </tbody>
    </table>
  </div>
</div>

<style>
  .ledger {
    padding: 1rem;
  }

  .ledger-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1rem;
  }

  h2 {
    font-size: 1.1rem;
    color: #c9d1d9;
  }

  .controls {
    display: flex;
    gap: 0.5rem;
  }

  input {
    background: #0d1117;
    border: 1px solid #30363d;
    color: #c9d1d9;
    padding: 0.3rem 0.5rem;
    border-radius: 4px;
    font-size: 0.85rem;
    width: 80px;
  }

  button {
    background: #238636;
    color: #fff;
    border: none;
    padding: 0.3rem 0.8rem;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.85rem;
  }
  button:hover { background: #2ea043; }

  .equity-curve-container {
    position: relative;
    width: 100%;
    height: 150px;
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 8px;
    margin-bottom: 1rem;
    overflow: hidden;
  }

  .loading {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    color: #8b949e;
  }

  .table-wrap {
    max-height: 500px;
    overflow-y: auto;
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 8px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.8rem;
  }

  th {
    text-align: left;
    color: #8b949e;
    font-weight: 500;
    padding: 0.4rem 0.6rem;
    border-bottom: 1px solid #30363d;
    position: sticky;
    top: 0;
    background: #161b22;
    z-index: 1;
  }

  td {
    padding: 0.3rem 0.6rem;
    color: #c9d1d9;
    border-bottom: 1px solid #21262d;
  }

  .mono { font-family: monospace; font-variant-numeric: tabular-nums; }
  .center { text-align: center; color: #8b949e; }
  .error { color: #f85149; margin-bottom: 1rem; }
</style>
