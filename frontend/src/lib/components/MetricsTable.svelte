<script>
  export let metrics = {};

  function fmt(v, dec = 2, pct = false) {
    if (v == null || isNaN(v)) return '—';
    if (pct) return (v * 100).toFixed(dec) + '%';
    return Number(v).toFixed(dec);
  }

  function colorClass(v) {
    if (v == null || isNaN(v)) return 'neutral';
    if (v > 0) return 'pos';
    if (v < 0) return 'neg';
    return 'neutral';
  }

  function colorClassInv(v) {
    // For drawdown: negative is bad
    if (v == null || isNaN(v)) return 'neutral';
    if (v > 0) return 'neg';
    if (v < 0) return 'pos';
    return 'neutral';
  }
</script>

<div class="metrics-table">
  <div class="table-header">Strategy Metrics</div>

  {#if Object.keys(metrics).length === 0}
    <div class="empty-state">No metrics available. Run a backtest to see results.</div>
  {:else}
    <table>
      <tbody>
        <tr>
          <td class="metric-label">Total Return</td>
          <td class="metric-value {colorClass(metrics.total_return)}">{fmt(metrics.total_return, 2, true)}</td>
        </tr>
        <tr>
          <td class="metric-label">Buy & Hold Return</td>
          <td class="metric-value {colorClass(metrics.buy_hold_return)}">{fmt(metrics.buy_hold_return, 2, true)}</td>
        </tr>
        <tr>
          <td class="metric-label">CAGR</td>
          <td class="metric-value {colorClass(metrics.cagr)}">{fmt(metrics.cagr, 2, true)}</td>
        </tr>
        <tr>
          <td class="metric-label">Sharpe Ratio</td>
          <td class="metric-value {colorClass(metrics.sharpe)}">{fmt(metrics.sharpe)}</td>
        </tr>
        <tr>
          <td class="metric-label">Sortino Ratio</td>
          <td class="metric-value {colorClass(metrics.sortino)}">{fmt(metrics.sortino)}</td>
        </tr>
        <tr>
          <td class="metric-label">Max Drawdown</td>
          <td class="metric-value {colorClassInv(metrics.max_drawdown)}">{fmt(metrics.max_drawdown, 2, true)}</td>
        </tr>
        <tr>
          <td class="metric-label">Win Rate</td>
          <td class="metric-value {colorClass(metrics.win_rate - 0.5)}">{fmt(metrics.win_rate, 1, true)}</td>
        </tr>
        <tr>
          <td class="metric-label">Profit Factor</td>
          <td class="metric-value {colorClass(metrics.profit_factor - 1)}">{fmt(metrics.profit_factor)}</td>
        </tr>
        <tr>
          <td class="metric-label">Trade Count</td>
          <td class="metric-value neutral">{metrics.trade_count ?? '—'}</td>
        </tr>
      </tbody>
    </table>
  {/if}
</div>

<style>
  .metrics-table {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0;
    overflow: hidden;
  }

  .table-header {
    padding: 0.6rem 0.75rem;
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
    font-weight: 600;
    border-bottom: 1px solid var(--border);
    background: var(--bg-inset);
  }

  .empty-state {
    padding: 1.5rem;
    text-align: center;
    color: var(--text-muted);
    font-size: 0.82rem;
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  tr {
    border-bottom: 1px solid var(--border);
  }

  tr:last-child {
    border-bottom: none;
  }

  .metric-label {
    padding: 0.5rem 0.75rem;
    font-size: 0.8rem;
    color: var(--text-secondary);
  }

  .metric-value {
    padding: 0.5rem 0.75rem;
    font-size: 0.82rem;
    font-variant-numeric: tabular-nums;
    font-family: var(--font-mono);
    text-align: right;
    color: var(--text-primary);
  }

  .metric-value.pos { color: var(--green); }
  .metric-value.neg { color: var(--red); }
  .metric-value.neutral { color: var(--text-primary); }
</style>