<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchEquityTrades } from '../lib/api.js';
  import { trades as tradesStore } from '../lib/stores.js';

  let canvas;
  let container;
  let trades = [];
  let symbol = 'QQQ';
  let loading = true;
  let error = null;
  let totalPnl = 0;
  let resizeObserver;

  onMount(async () => {
    await loadData();
    resizeObserver = new ResizeObserver(() => drawCurve());
    if (container) resizeObserver.observe(container);
  });

  onDestroy(() => {
    if (resizeObserver) resizeObserver.disconnect();
  });

  // Live-update when a new trade fill arrives via WS
  $: if ($tradesStore && $tradesStore.length > 0) {
    // Only reload if we already have data (don't override initial load)
    if (!loading && trades.length > 0) {
      loadData();
    }
  }

  async function loadData() {
    loading = true;
    error = null;
    try {
      const data = await fetchEquityTrades(symbol, 500);
      trades = data.trades || [];
      totalPnl = data.total_realized_pnl || 0;
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

  function fmtTime(ts) {
    if (!ts) return '—';
    try {
      const d = new Date(ts);
      return d.toLocaleString();
    } catch {
      return String(ts);
    }
  }

  function fmt(v, dec = 2) {
    if (v == null || isNaN(v)) return '—';
    return Number(v).toFixed(dec);
  }

  function pnlColor(v) {
    if (v == null || isNaN(v)) return '';
    if (v > 0) return 'pos';
    if (v < 0) return 'neg';
    return '';
  }

  function drawCurve() {
    if (!canvas || !trades.length) return;
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

    const padL = 60, padR = 10, padT = 10, padB = 25;
    const cw = w - padL - padR;
    const ch = h - padT - padB;

    // Cumulative PnL from trades
    const points = trades.map((t) => t.cumulative_pnl);
    if (points.length < 1) return;

    let minP = Math.min(...points, 0);
    let maxP = Math.max(...points, 0);
    const range = maxP - minP || 1;

    const n = points.length;
    const xStep = n > 1 ? cw / (n - 1) : 0;
    const yScale = (v) => padT + ch - ((v - minP) / range) * ch;

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
      const val = maxP - (range / 4) * i;
      ctx.fillText(val.toFixed(2), 2, y + 3);
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

    // Cumulative PnL line
    const lastP = points[n - 1];
    ctx.strokeStyle = lastP >= 0 ? '#3fb950' : '#f85149';
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const x = n > 1 ? padL + i * xStep : padL + cw / 2;
      const y = yScale(points[i]);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();

    // Fill
    if (n > 1) {
      ctx.lineTo(padL + (n - 1) * xStep, zeroY);
      ctx.lineTo(padL, zeroY);
    }
    ctx.closePath();
    ctx.fillStyle = lastP >= 0 ? '#3fb95022' : '#f8514922';
    ctx.fill();

    // Trade markers
    for (let i = 0; i < n; i++) {
      const x = n > 1 ? padL + i * xStep : padL + cw / 2;
      const y = yScale(points[i]);
      const isBuy = trades[i].side.toLowerCase() === 'buy';
      ctx.fillStyle = isBuy ? '#3fb950' : '#f85149';
      ctx.beginPath();
      ctx.arc(x, y, 2.5, 0, Math.PI * 2);
      ctx.fill();
    }
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

  <div class="summary-row">
    <div class="stat">
      <span class="stat-label">Total Trades</span>
      <span class="stat-value">{trades.length}</span>
    </div>
    <div class="stat">
      <span class="stat-label">Total Realized PnL</span>
      <span class="stat-value {pnlColor(totalPnl)}">{fmt(totalPnl)}</span>
    </div>
    <div class="stat">
      <span class="stat-label">Win Rate</span>
      <span class="stat-value">
        {trades.length > 0
          ? ((trades.filter((t) => t.realized_pnl > 0).length / trades.filter((t) => t.realized_pnl !== 0).length || 0) * 100).toFixed(1) + '%'
          : '—'}
      </span>
    </div>
    <div class="stat">
      <span class="stat-label">Total Fees</span>
      <span class="stat-value">{fmt(trades.reduce((s, t) => s + (t.fee || 0), 0))}</span>
    </div>
  </div>

  <div class="equity-curve-container" bind:this={container}>
    <canvas bind:this={canvas}></canvas>
    {#if loading}
      <div class="loading">Loading…</div>
    {/if}
    {#if !loading && trades.length === 0}
      <div class="loading">No trades yet</div>
    {/if}
  </div>

  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          <th>#</th>
          <th>Time</th>
          <th>Side</th>
          <th>Qty</th>
          <th>Price</th>
          <th>Fee</th>
          <th>Realized PnL</th>
          <th>Cumulative PnL</th>
        </tr>
      </thead>
      <tbody>
        {#if loading}
          <tr><td colspan="8" class="center">Loading…</td></tr>
        {:else if trades.length === 0}
          <tr><td colspan="8" class="center">No trades yet</td></tr>
        {:else}
          {#each trades as t, i}
            <tr>
              <td class="mono">{i + 1}</td>
              <td class="mono">{fmtTime(t.ts)}</td>
              <td class="side-{(t.side || '').toLowerCase()}">{t.side || '—'}</td>
              <td class="mono">{t.qty ?? '—'}</td>
              <td class="mono">{fmt(t.price)}</td>
              <td class="mono">{fmt(t.fee)}</td>
              <td class="mono {pnlColor(t.realized_pnl)}">{fmt(t.realized_pnl)}</td>
              <td class="mono {pnlColor(t.cumulative_pnl)}">{fmt(t.cumulative_pnl)}</td>
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

  .summary-row {
    display: flex;
    gap: 1.5rem;
    margin-bottom: 1rem;
    flex-wrap: wrap;
  }

  .stat {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .stat-label {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #8b949e;
  }

  .stat-value {
    font-size: 1.1rem;
    font-family: monospace;
    font-variant-numeric: tabular-nums;
    color: #c9d1d9;
  }

  .equity-curve-container {
    position: relative;
    width: 100%;
    height: 200px;
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
  .pos { color: #3fb950; }
  .neg { color: #f85149; }
  .side-buy { color: #3fb950; font-weight: 600; }
  .side-sell { color: #f85149; font-weight: 600; }
</style>
