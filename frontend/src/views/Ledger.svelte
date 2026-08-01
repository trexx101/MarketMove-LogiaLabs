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

  $: if ($tradesStore && $tradesStore.length > 0) {
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

    const padL = 62, padR = 12, padT = 12, padB = 25;
    const cw = w - padL - padR;
    const ch = h - padT - padB;

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
    ctx.fillStyle = '#5c5e6e';
    ctx.strokeStyle = '#1c1d27';
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
    ctx.strokeStyle = '#2e2f3d';
    ctx.setLineDash([2, 2]);
    ctx.beginPath();
    ctx.moveTo(padL, zeroY);
    ctx.lineTo(w - padR, zeroY);
    ctx.stroke();
    ctx.setLineDash([]);

    // Cumulative PnL line
    const lastP = points[n - 1];
    ctx.strokeStyle = lastP >= 0 ? '#149e61' : '#e5484d';
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
    ctx.fillStyle = lastP >= 0 ? 'rgba(20,158,97,0.12)' : 'rgba(229,72,77,0.12)';
    ctx.fill();

    // Trade markers
    for (let i = 0; i < n; i++) {
      const x = n > 1 ? padL + i * xStep : padL + cw / 2;
      const y = yScale(points[i]);
      const isBuy = trades[i].side.toLowerCase() === 'buy';
      ctx.fillStyle = isBuy ? '#149e61' : '#e5484d';
      ctx.beginPath();
      ctx.arc(x, y, 2.5, 0, Math.PI * 2);
      ctx.fill();
    }
  }
</script>

<div class="ledger">
  <div class="ledger-header">
    <h1>Ledger — {symbol}</h1>
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

  <div class="chart-card" bind:this={container}>
    <div class="chart-label">Equity Curve</div>
    <canvas bind:this={canvas}></canvas>
    {#if loading}
      <div class="loading">Loading...</div>
    {/if}
    {#if !loading && trades.length === 0}
      <div class="loading">No trades yet</div>
    {/if}
  </div>

  <div class="table-card">
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
            <tr><td colspan="8" class="center">Loading...</td></tr>
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
</div>

<style>
  .ledger {
    padding: 1.25rem;
  }

  .ledger-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1.25rem;
  }

  h1 {
    font-size: 1.25rem;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .controls {
    display: flex;
    gap: 0.4rem;
  }

  input {
    background: var(--bg-inset);
    border: 1px solid var(--border);
    color: var(--text-primary);
    padding: 0.35rem 0.55rem;
    border-radius: var(--radius-xs);
    font-size: 0.82rem;
    font-family: inherit;
    width: 80px;
  }
  input:focus {
    outline: none;
    border-color: var(--accent);
  }

  button {
    background: var(--accent);
    color: #fff;
    border: none;
    padding: 0.35rem 0.9rem;
    border-radius: var(--radius-xs);
    cursor: pointer;
    font-size: 0.82rem;
    font-weight: 500;
    font-family: inherit;
    transition: background 0.15s;
  }
  button:hover { background: var(--accent-dark); }

  .summary-row {
    display: flex;
    gap: 2rem;
    margin-bottom: 1.25rem;
    flex-wrap: wrap;
  }

  .stat {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .stat-label {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
    font-weight: 600;
  }

  .stat-value {
    font-size: 1.15rem;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--text-primary);
    font-weight: 500;
  }

  .stat-value.pos { color: var(--green); }
  .stat-value.neg { color: var(--red); }

  .chart-card {
    position: relative;
    width: 100%;
    height: 200px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    margin-bottom: 1rem;
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

  .loading {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    color: var(--text-secondary);
    font-size: 0.82rem;
  }

  .table-card {
    max-height: 500px;
    overflow-y: auto;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }

  .table-wrap {
    overflow-y: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.8rem;
  }

  th {
    text-align: left;
    color: var(--text-secondary);
    font-weight: 500;
    padding: 0.5rem 0.65rem;
    border-bottom: 1px solid var(--border);
    position: sticky;
    top: 0;
    background: var(--bg-surface);
    z-index: 1;
  }

  td {
    padding: 0.4rem 0.65rem;
    color: var(--text-primary);
    border-bottom: 1px solid var(--border);
  }

  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  .center { text-align: center; color: var(--text-secondary); }
  .error { color: var(--red); margin-bottom: 1rem; }
  .pos { color: var(--green); }
  .neg { color: var(--red); }
  .side-buy { color: var(--green); font-weight: 600; }
  .side-sell { color: var(--red); font-weight: 600; }
</style>
