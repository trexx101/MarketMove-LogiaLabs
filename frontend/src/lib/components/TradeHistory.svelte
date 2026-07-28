<script>
  import { trades } from '../stores.js';

  $: tradeList = $trades;

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
</script>

<div class="trade-history">
  <div class="panel-header">Trade History</div>
  {#if tradeList.length === 0}
    <div class="empty">No trades yet</div>
  {:else}
    <table>
      <thead>
        <tr>
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
            <td class="mono">{fmtTime(t.time)}</td>
            <td class="side-{(t.side || '').toLowerCase()}">{t.side || '—'}</td>
            <td class="mono">{t.qty ?? '—'}</td>
            <td class="mono">{fmt(t.price)}</td>
            <td class="mono">{fmt(t.fee)}</td>
            <td class="mono {pnlColor(t.realized_pnl)}">{fmt(t.realized_pnl)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

<style>
  .trade-history {
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 8px;
    padding: 0.75rem;
    overflow: hidden;
  }

  .panel-header {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #8b949e;
    margin-bottom: 0.5rem;
  }

  .empty {
    color: #8b949e;
    font-size: 0.85rem;
    padding: 1rem 0;
    text-align: center;
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
    padding: 0.3rem 0.5rem;
    border-bottom: 1px solid #30363d;
    position: sticky;
    top: 0;
    background: #161b22;
  }

  td {
    padding: 0.3rem 0.5rem;
    color: #c9d1d9;
    border-bottom: 1px solid #21262d;
  }

  .mono { font-family: monospace; font-variant-numeric: tabular-nums; }
  .pos { color: #3fb950; }
  .neg { color: #f85149; }
  .side-buy { color: #3fb950; font-weight: 600; }
  .side-sell { color: #f85149; font-weight: 600; }

  .table-wrap {
    max-height: 250px;
    overflow-y: auto;
  }
</style>
