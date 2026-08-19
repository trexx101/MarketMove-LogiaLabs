<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchOptionPositions } from '../lib/api.js';

  const UNDERLYINGS = ['ALL', 'QQQ', 'SMH', 'XLF'];
  const STATUSES = ['ALL', 'OPEN', 'CLOSED'];

  let underlying = 'ALL';
  let status = 'OPEN';
  let positions = [];
  let loading = true;
  let error = '';
  let refreshInterval;

  $: openCount = positions.filter((p) => p.status === 'OPEN').length;
  $: closedCount = positions.filter((p) => p.status === 'CLOSED').length;

  async function load() {
    error = '';
    try {
      const opts = { limit: 200 };
      if (underlying !== 'ALL') opts.underlying = underlying;
      if (status !== 'ALL') opts.status = status;
      const res = await fetchOptionPositions(opts);
      positions = res.positions || [];
    } catch (e) {
      error = e.message;
      positions = [];
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    load();
    refreshInterval = setInterval(load, 30000);
  });
  onDestroy(() => clearInterval(refreshInterval));

  function fmt(v, dec = 2) {
    if (v == null || isNaN(v)) return '—';
    return Number(v).toFixed(dec);
  }

  function fmtTime(ts) {
    if (!ts) return '—';
    try {
      return new Date(ts).toLocaleString();
    } catch {
      return String(ts);
    }
  }

  function fmtDate(ts) {
    if (!ts) return '—';
    try {
      return new Date(ts).toISOString().slice(0, 10);
    } catch {
      return String(ts);
    }
  }

  function pnlColor(v) {
    if (v == null || isNaN(v)) return '';
    return v > 0 ? 'pos' : v < 0 ? 'neg' : '';
  }

  function statusClass(s) {
    return (s || '').toLowerCase();
  }
</script>

<div class="view">
  <div class="view-header">
    <h1>Options — Positions</h1>
    <div class="controls">
      <select bind:value={underlying} on:change={load}>
        {#each UNDERLYINGS as u}
          <option value={u}>{u}</option>
        {/each}
      </select>
      <select bind:value={status} on:change={load}>
        {#each STATUSES as s}
          <option value={s}>{s}</option>
        {/each}
      </select>
      <button on:click={load}>Refresh</button>
    </div>
  </div>

  {#if error}
    <div class="error">Error: {error}</div>
  {/if}

  <div class="summary-row">
    <div class="stat">
      <span class="stat-label">Showing</span>
      <span class="stat-value">{positions.length}</span>
    </div>
    <div class="stat">
      <span class="stat-label">Open</span>
      <span class="stat-value">{openCount}</span>
    </div>
    <div class="stat">
      <span class="stat-label">Closed</span>
      <span class="stat-value">{closedCount}</span>
    </div>
  </div>

  <div class="table-card">
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th>Underlying</th>
            <th>Contract</th>
            <th>Status</th>
            <th>Qty</th>
            <th>Filled</th>
            <th>Entry Px</th>
            <th>Premium</th>
            <th>DTE</th>
            <th>Delta</th>
            <th>Realized PnL</th>
            <th>Opened</th>
            <th>Closed</th>
          </tr>
        </thead>
        <tbody>
          {#if loading}
            <tr><td colspan="12" class="center">Loading...</td></tr>
          {:else if positions.length === 0}
            <tr><td colspan="12" class="center">No positions</td></tr>
          {:else}
            {#each positions as p}
              <tr>
                <td class="mono">{p.underlying}</td>
                <td class="mono contract" title={p.contract_code}>{p.contract_code}</td>
                <td><span class="badge status-{statusClass(p.status)}">{p.status}</span></td>
                <td class="mono">{p.qty}</td>
                <td class="mono">{p.qty_filled_residual}</td>
                <td class="mono">{fmt(p.entry_underlying_price)}</td>
                <td class="mono">{fmt(p.entry_premium)}</td>
                <td class="mono">{p.dte_at_entry}</td>
                <td class="mono">{fmt(p.delta_at_entry, 3)}</td>
                <td class="mono {pnlColor(p.realized_pnl)}">{fmt(p.realized_pnl)}</td>
                <td class="mono" title={fmtTime(p.created_at)}>{fmtDate(p.created_at)}</td>
                <td class="mono" title={fmtTime(p.closed_at)}>{fmtDate(p.closed_at)}</td>
              </tr>
            {/each}
          {/if}
        </tbody>
      </table>
    </div>
  </div>
</div>

<style>
  .view { padding: 1.25rem; }

  .view-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1.25rem;
    flex-wrap: wrap;
    gap: 0.75rem;
  }

  h1 {
    font-size: 1.25rem;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .controls { display: flex; gap: 0.4rem; }

  select, button {
    background: var(--bg-inset);
    border: 1px solid var(--border);
    color: var(--text-primary);
    padding: 0.35rem 0.55rem;
    border-radius: var(--radius-xs);
    font-size: 0.82rem;
    font-family: inherit;
  }
  select:focus, button:focus { outline: none; border-color: var(--accent); }
  button {
    background: var(--accent);
    border: none;
    color: #fff;
    cursor: pointer;
    font-weight: 500;
    transition: background 0.15s;
  }
  button:hover { background: var(--accent-dark); }

  .error {
    background: var(--red-subtle);
    border: 1px solid var(--red);
    color: var(--text-primary);
    padding: 0.6rem 0.9rem;
    border-radius: var(--radius-xs);
    font-size: 0.82rem;
    margin-bottom: 1rem;
  }

  .summary-row {
    display: flex;
    gap: 2rem;
    margin-bottom: 1.25rem;
    flex-wrap: wrap;
  }

  .stat { display: flex; flex-direction: column; gap: 0.2rem; }

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

  .table-card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .table-wrap { overflow-x: auto; }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.8rem;
  }

  th {
    text-align: left;
    padding: 0.55rem 0.75rem;
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
    font-weight: 600;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }

  td {
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--border);
    color: var(--text-primary);
    white-space: nowrap;
  }

  tr:last-child td { border-bottom: none; }
  tr:hover td { background: var(--bg-surface-hover); }

  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  .contract {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .center { text-align: center; color: var(--text-muted); padding: 2rem; }
  .pos { color: var(--green); }
  .neg { color: var(--red); }

  .badge {
    display: inline-block;
    padding: 0.12rem 0.5rem;
    border-radius: var(--radius-xs);
    font-size: 0.68rem;
    font-weight: 600;
    letter-spacing: 0.03em;
  }

  .status-open { background: var(--green-subtle); color: var(--green); }
  .status-closed { background: rgba(139, 141, 154, 0.14); color: var(--text-secondary); }
</style>
