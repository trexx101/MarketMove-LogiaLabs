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

<div class="card">
  <div class="card-header">Trade History <span class="card-sub">(all symbols)</span></div>
  {#if tradeList.length === 0}
    <div class="empty">No trades yet</div>
  {:else}
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th>Time</th>
            <th>Symbol</th>
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
              <td>
                {#if t.symbol}
                  <span class="sym-badge">{t.symbol}</span>
                {:else}
                  &mdash;
                {/if}
              </td>
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
      .card-sub {
    font-weight: 400;
    font-size: 0.72rem;
    color: var(--text-secondary);
    margin-left: 0.4rem;
    opacity: 0.7;
  }

.sym-badge {
    display: inline-block;
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
    background: var(--bg-alt, rgba(255,255,255,0.06));
    border: 1px solid var(--border, rgba(255,255,255,0.1));
    font-family: monospace;
    font-size: 0.78rem;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: 0.02em;
  }

.side-buy { color: var(--green); font-weight: 600; }
  .side-sell { color: var(--red); font-weight: 600; }
</style>
