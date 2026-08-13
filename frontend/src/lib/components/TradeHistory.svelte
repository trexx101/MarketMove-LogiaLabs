<script>
  import { onMount } from 'svelte';
  import { fetchEquityTrades } from '../api.js';

  let tradeList = [];

  async function loadTrades() {
    try {
      const data = await fetchEquityTrades('*', 200);
      tradeList = data?.trades || [];
    } catch (e) {
      console.error('Failed to load trade history:', e);
    }
  }

  onMount(() => {
    loadTrades();
  });

  function fmtTime(ts) {
    if (!ts) return '—';
    try {
      const d = new Date(ts);
      return d.toLocaleTimeString();
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

  function modelBadgeClass(id) {
    if (!id) return 'badge';
    // Deterministic muted color based on the model id string.
    let hash = 0;
    for (let i = 0; i < id.length; i++) hash = id.charCodeAt(i) + ((hash << 5) - hash);
    const hue = Math.abs(hash % 360);
    return `badge badge-hue-${hue}`;
  }
</script>

<div class="card">
  <div class="card-header">Trade History</div>
  {#if tradeList.length === 0}
    <div class="empty">No trades yet</div>
  {:else}
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th>Model</th>
            <th>Symbol</th>
            <th>Time</th>
            <th>Side</th>
            <th>Qty</th>
            <th>Price</th>
            <th>Fee</th>
            <th>PnL</th>
          </tr>
        </thead>
        <tbody>
          {#each tradeList as t}
            <tr>
              <td class="mono model-cell"><span class="model-badge">{t.model_id || '—'}</span></td>
              <td class="mono">{t.symbol || '—'}</td>
              <td class="mono">{fmtTime(t.ts)}</td>
              <td class="side-{(t.side || '').toLowerCase()}">{t.side || '—'}</td>
              <td class="mono">{t.qty ?? '—'}</td>
              <td class="mono">{fmt(t.price)}</td>
              <td class="mono">{fmt(t.fee)}</td>
              <td class="mono {pnlColor(t.realized_pnl)}">{fmt(t.realized_pnl)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<style>
  .card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0.85rem;
    overflow: hidden;
  }

  .card-header {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
    font-weight: 600;
    margin-bottom: 0.6rem;
  }

  .empty {
    color: var(--text-secondary);
    font-size: 0.82rem;
    padding: 1.5rem 0;
    text-align: center;
  }

  .table-wrap {
    max-height: 250px;
    overflow-y: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.78rem;
  }

  th {
    text-align: left;
    color: var(--text-secondary);
    font-weight: 500;
    padding: 0.35rem 0.5rem;
    border-bottom: 1px solid var(--border);
    position: sticky;
    top: 0;
    background: var(--bg-surface);
    z-index: 1;
  }

  td {
    padding: 0.35rem 0.5rem;
    color: var(--text-primary);
    border-bottom: 1px solid var(--border);
  }

  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  .pos { color: var(--green); }
  .neg { color: var(--red); }
  .side-buy { color: var(--green); font-weight: 600; }
  .side-sell { color: var(--red); font-weight: 600; }

  .model-cell {
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .model-badge {
    display: inline-block;
    padding: 0.1rem 0.4rem;
    border-radius: var(--radius-xs);
    background: var(--accent-subtle);
    color: var(--accent);
    font-size: 0.68rem;
    font-weight: 600;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
