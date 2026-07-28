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
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 8px;
    padding: 0;
    overflow: hidden;
  }

  .table-header {
    padding: 0.6rem 0.75rem;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #8b949e;
    border-bottom: 1px solid #30363d;
    background: #0d1117;
  }

  .empty-state {
    padding: 1.5rem;
    text-align: center;
    color: #484f58;
    font-size: 0.85rem;
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  tr {
    border-bottom: 1px solid #21262d;
  }

  tr:last-child {
    border-bottom: none;
  }

  .metric-label {
    padding: 0.5rem 0.75rem;
    font-size: 0.82rem;
    color: #8b949e;
  }

  .metric-value {
    padding: 0.5rem 0.75rem;
    font-size: 0.85rem;
    font-variant-numeric: tabular-nums;
    font-family: monospace;
    text-align: right;
    color: #c9d1d9;
  }

  .metric-value.pos { color: #3fb950; }
  .metric-value.neg { color: #f85149; }
  .metric-value.neutral { color: #c9d1d9; }
</style>