<script>
  import { status, wsConnected } from '../stores.js';

  $: s = $status;
  $: mode = s?.mode || 'PAPER';
  $: symbol = s?.symbol || 'QQQ';
  $: position = s?.position || 'flat';
  $: entryPrice = s?.entry_price;
  $: realizedPnl = s?.realized_pnl;
  $: unrealizedPnl = s?.unrealized_pnl;
  $: staleness = s?.staleness_secs ?? 0;
  $: lastClose = s?.last_close;

  function fmt(v, dec = 2) {
    if (v == null || isNaN(v)) return '—';
    return Number(v).toFixed(dec);
  }

  function pnlColor(v) {
    if (v == null || isNaN(v)) return 'neutral';
    if (v > 0) return 'pos';
    if (v < 0) return 'neg';
    return 'neutral';
  }

  function positionLabel(p) {
    if (!p) return 'FLAT';
    const up = String(p).toUpperCase();
    if (up === 'LONG' || up === '1') return 'LONG';
    if (up === 'SHORT' || up === '-1') return 'SHORT';
    return 'FLAT';
  }

  $: stale = staleness > 120;
</script>

<div class="status-panel">
  <div class="panel-header">Status</div>

  <div class="row">
    <span class="label">Mode</span>
    <span class="badge mode-{mode.toLowerCase()}">{mode}</span>
  </div>

  <div class="row">
    <span class="label">Symbol</span>
    <span class="value">{symbol}</span>
  </div>

  <div class="row">
    <span class="label">Position</span>
    <span class="badge pos-{positionLabel(position).toLowerCase()}">{positionLabel(position)}</span>
  </div>

  <div class="row">
    <span class="label">Entry</span>
    <span class="value">{fmt(entryPrice)}</span>
  </div>

  <div class="row">
    <span class="label">Last</span>
    <span class="value">{fmt(lastClose)}</span>
  </div>

  <div class="row">
    <span class="label">Realized PnL</span>
    <span class="value {pnlColor(realizedPnl)}">{fmt(realizedPnl)}</span>
  </div>

  <div class="row">
    <span class="label">Unrealized PnL</span>
    <span class="value {pnlColor(unrealizedPnl)}">{fmt(unrealizedPnl)}</span>
  </div>

  <div class="row staleness {stale ? 'stale' : ''}">
    <span class="label">Staleness</span>
    <span class="value">{staleness}s</span>
  </div>

  <div class="row">
    <span class="label">WS</span>
    <span class="ws-dot {$wsConnected ? 'on' : 'off'}"></span>
  </div>
</div>

<style>
  .status-panel {
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 8px;
    padding: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .panel-header {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #8b949e;
    margin-bottom: 0.25rem;
  }

  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.85rem;
  }

  .label {
    color: #8b949e;
  }

  .value {
    color: #c9d1d9;
    font-variant-numeric: tabular-nums;
  }

  .value.pos { color: #3fb950; }
  .value.neg { color: #f85149; }

  .badge {
    padding: 0.1rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
    font-weight: 600;
  }

  .mode-paper { background: #1f6feb22; color: #58a6ff; }
  .mode-live { background: #da363322; color: #f85149; }

  .pos-long { background: #23863622; color: #3fb950; }
  .pos-short { background: #da363322; color: #f85149; }
  .pos-flat { background: #30363d; color: #8b949e; }

  .staleness.stale .value { color: #f85149; }

  .ws-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    display: inline-block;
  }
  .ws-dot.on { background: #3fb950; box-shadow: 0 0 4px #3fb950; }
  .ws-dot.off { background: #f85149; }
</style>
