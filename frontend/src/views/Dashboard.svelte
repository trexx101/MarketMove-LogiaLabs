<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchStatus, fetchPredictions, fetchChart, fetchAccuracy, fetchEquityTrades } from '../lib/api.js';
  import { connectWebSocket, disconnectWebSocket } from '../lib/websocket.js';
  import { status, predictions, chartData, accuracy, trades } from '../lib/stores.js';

  import StatusPanel from '../lib/components/StatusPanel.svelte';
  import CandlestickChart from '../lib/components/CandlestickChart.svelte';
  import PnLEquityCurve from '../lib/components/PnLEquityCurve.svelte';
  import FeatureInspector from '../lib/components/FeatureInspector.svelte';
  import TradeHistory from '../lib/components/TradeHistory.svelte';
  import ModelHealth from '../lib/components/ModelHealth.svelte';
  import StrategyConfigPanel from '../lib/components/StrategyConfigPanel.svelte';

  let chartComponent;
  let statusInterval;

  onMount(async () => {
    try {
      const s = await fetchStatus();
      status.set(s);
    } catch (e) {
      console.error('Failed to fetch status:', e);
    }

    try {
      const p = await fetchPredictions();
      predictions.set(p);
    } catch (e) {
      console.error('Failed to fetch predictions:', e);
    }

    try {
      const c = await fetchChart();
      chartData.set(c);
    } catch (e) {
      console.error('Failed to fetch chart:', e);
    }

    try {
      const a = await fetchAccuracy();
      if (a) accuracy.set(a);
    } catch (e) {
      console.error('Failed to fetch accuracy:', e);
    }

    try {
      const td = await fetchEquityTrades('*', 200);
      trades.set((td.trades || []).map(t => ({
        time: t.ts,
        side: t.side,
        qty: t.qty,
        price: t.price,
        fee: t.fee,
        realized_pnl: t.realized_pnl,
      })));
    } catch (e) {
      console.error('Failed to fetch trades:', e);
    }

    connectWebSocket();

    statusInterval = setInterval(async () => {
      try {
        const s = await fetchStatus();
        status.set(s);
      } catch (e) {
        // Silent — WS may still be delivering updates
      }
    }, 30000);
  });

  onDestroy(() => {
    disconnectWebSocket();
    if (statusInterval) clearInterval(statusInterval);
  });

  $: if (chartComponent && $status && $predictions) {
    const lastClose = $status.last_close;
    const preds = $predictions.latest;
    if (lastClose != null && preds) {
      chartComponent.setPredictions(preds, lastClose);
    }
  }
</script>

<!--
  Layout B — TradingView-Lite
  ┌────────────────────────────┬──────────────┐
  │                            │   STATUS     │
  │         CHART (hero)       ├──────────────┤
  │                            │   TRADES     │
  ├────────────────────────────┴──────────────┤
  │              PnL EQUITY CURVE             │
  ├────────────┬──────────────┬───────────────┤
  │  FEATURES  │   HEALTH     │   STRATEGY    │
  └────────────┴──────────────┴───────────────┘
-->
<div class="dashboard">
  <div class="dash-header">
    <h1>Dashboard</h1>
    <span class="dash-subtitle">Logia — Real-time monitoring</span>
  </div>

  <div class="grid">
    <section class="chart-area">
      <CandlestickChart bind:this={chartComponent} />
    </section>

    <aside class="rail">
      <StatusPanel />
    </aside>

    <section class="pnl-area">
      <PnLEquityCurve />
    </section>

    <section class="meta-features">
      <FeatureInspector />
    </section>

    <section class="meta-health">
      <ModelHealth />
    </section>

    <section class="meta-strategy">
      <StrategyConfigPanel />
    </section>

    <section class="trade-area">
      <TradeHistory />
    </section>
  </div>
</div>

<style>
  .dashboard {
    padding: 1.25rem;
  }

  .dash-header {
    display: flex;
    align-items: baseline;
    gap: 0.75rem;
    margin-bottom: 1.25rem;
  }

  .dash-header h1 {
    font-size: 1.25rem;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .dash-subtitle {
    font-size: 0.8rem;
    color: var(--text-secondary);
  }

  /* 12-col grid: chart 8 cols, right rail 4 cols */
  .grid {
    display: grid;
    grid-template-columns: repeat(12, 1fr);
    grid-auto-rows: min-content;
    gap: 1rem;
  }

  .chart-area  { grid-column: 1 / 9;  grid-row: 1; }
  .rail        { grid-column: 9 / 13; grid-row: 1; min-height: 420px; }

  .pnl-area    { grid-column: 1 / 13; grid-row: 2; }

  .meta-features  { grid-column: 1 / 5;  grid-row: 3; }
  .meta-health    { grid-column: 5 / 9;  grid-row: 3; }
  .meta-strategy  { grid-column: 9 / 13; grid-row: 3; }

  /* Horizontal trade band at the absolute bottom */
  .trade-area { grid-column: 1 / 13; grid-row: 4; }

  /* Make every panel container a flex shell so children fill cleanly */
  .grid > section,
  .grid > aside {
    display: flex;
    flex-direction: column;
    min-width: 0; /* let children shrink instead of overflowing */
  }

  .grid > section > :global(*),
  .grid > aside > :global(*) {
    flex: 1 1 auto;
    min-height: 0;
  }

  /* Wide screens: StatusPanel rail stretches with the chart */
  @media (min-width: 1600px) {
    .rail { min-height: 480px; }
  }

  /* Tablets: chart full-width, StatusPanel below it */
  @media (max-width: 1100px) {
    .chart-area  { grid-column: 1 / 13; grid-row: 1; }
    .rail        { grid-column: 1 / 13; grid-row: 2; min-height: 0; }
    .pnl-area    { grid-column: 1 / 13; grid-row: 3; }
    .meta-features { grid-column: 1 / 7;  grid-row: 4; }
    .meta-health   { grid-column: 7 / 13; grid-row: 4; }
    .meta-strategy { grid-column: 1 / 13; grid-row: 5; }
    .trade-area    { grid-column: 1 / 13; grid-row: 6; }
  }

  /* Phones: single column stack */
  @media (max-width: 640px) {
    .grid { grid-template-columns: 1fr; }
    .chart-area,
    .rail,
    .pnl-area,
    .meta-features,
    .meta-health,
    .meta-strategy,
    .trade-area {
      grid-column: 1 / -1;
      grid-row: auto;
      min-height: 0;
    }
  }
</style>
