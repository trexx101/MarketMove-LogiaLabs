<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchOptionTrades } from '../lib/api.js';

  const UNDERLYINGS = ['ALL', 'QQQ', 'SMH', 'XLF'];

  let underlying = 'ALL';
  let trades = [];
  let loading = true;
  let error = '';
  let refreshInterval;

  $: totalPnl = trades.reduce((sum, t) => sum + (t.realized_pnl || 0), 0);
  $: wins = trades.filter((t) => (t.realized_pnl || 0) > 0).length;
  $: winRate = trades.length > 0 ? ((wins / trades.length) * 100).toFixed(1) + '%' : '—';

  async function load() {
    error = '';
    try {
      const opts = { limit: 200 };
      if (underlying !== 'ALL') opts.underlying = underlying;
      const res = await fetchOptionTrades(opts);
      trades = res.trades || [];
    } catch (e) {
      error = e.message;
      trades = [];
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

  function holdDays(t) {
    if (!t.created_at || !t.closed_at) return '—';
    const days = (t.closed_at - t.created_at) / 86400000;
    return days.toFixed(1) + 'd';
  }
</script>

<div class="view">
  <div class="view-header">
    <h1>Options — Trade History</h1>
    <div class="controls">
      <select bind:value={underlying} on:change={load}>
        {#each UNDERLYINGS as u}
          <option value={u}>{u}</option>
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
      <span class="stat-label">Closed Trades</span>
      <span class="stat-value">{trades.length}</span>
    </div>
    <div class="stat">
      <span class="stat-label">Total Realized PnL</span>
      <span class="stat-value {pnlColor(totalPnl)}">{fmt(totalPnl)}</span>
    </div>
    <div class="stat">
      <span class="stat-label">Win Rate</span>
      <span class="stat-value">{winRate}</span>
    </div>
    <div class="stat">
      <span class="stat-label">Wins</span>
      <span class="stat-value pos">{wins}</span>
    </div>
  </div>

  <div class="table-card">
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th>Underlying</th>
            <th>Contract</th>
            <th>Qty</th>
            <th>Entry Premium</th>
            <th>Entry Px</th>
            <th>DTE @ Entry</th>
            <th>Delta @ Entry</th>
            <th>Realized PnL</th>
            <th>Opened</th>
            <th>Closed</th>
            <th>Hold</th>
          </tr>
        </thead>
        <tbody>
          {#if loading}
            <tr><td colspan="11" class="center">Loading...</td></tr>
          {:else if trades.length === 0}
            <tr><td colspan="11" class="center">No closed trades yet</td></tr>
          {:else}
            {#each trades as t}
              <tr>
                <td class="mono">{t.underlying}</td>
                <td class="mono contract" title={t.contract_code}>{t.contract_code}</td>
                <td class="mono">{t.qty}</td>
                <td class="mono">{fmt(t.entry_premium)}</td>
                <td class="mono">{fmt(t.entry_underlying_price)}</td>
                <td class="mono">{t.dte_at_entry}</td>
                <td class="mono">{fmt(t.delta_at_entry, 3)}</td>
                <td class="mono {pnlColor(t.realized_pnl)}">{fmt(t.realized_pnl)}</td>
                <td class="mono" title={fmtTime(t.created_at)}>{fmtDate(t.created_at)}</td>
                <td class="mono" title={fmtTime(t.closed_at)}>{fmtDate(t.closed_at)}</td>
                <td class="mono">{holdDays(t)}</td>
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
  .stat-value.pos { color: var(--green); }
  .stat-value.neg { color: var(--red); }

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
</style>
