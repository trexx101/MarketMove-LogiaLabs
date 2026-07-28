<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchStatus, fetchPredictions, fetchChart, fetchAccuracy } from '../lib/api.js';
  import { connectWebSocket, disconnectWebSocket } from '../lib/websocket.js';
  import { status, predictions, chartData, accuracy } from '../lib/stores.js';

  import StatusPanel from '../lib/components/StatusPanel.svelte';
  import CandlestickChart from '../lib/components/CandlestickChart.svelte';
  import PnLEquityCurve from '../lib/components/PnLEquityCurve.svelte';
  import FeatureInspector from '../lib/components/FeatureInspector.svelte';
  import TradeHistory from '../lib/components/TradeHistory.svelte';
  import ModelHealth from '../lib/components/ModelHealth.svelte';

  let chartComponent;

  onMount(async () => {
    // Fetch initial data via REST (fallback if WS not available)
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

    // Connect WebSocket for real-time updates
    connectWebSocket();
  });

  onDestroy(() => {
    disconnectWebSocket();
  });

  // Update chart predictions when status or predictions change
  $: if (chartComponent && $status && $predictions) {
    const lastClose = $status.last_close;
    const preds = $predictions.latest;
    if (lastClose != null && preds) {
      chartComponent.setPredictions(preds, lastClose);
    }
  }
</script>

<div class="dashboard">
  <div class="grid">
    <div class="grid-item chart-area">
      <CandlestickChart bind:this={chartComponent} />
    </div>

    <div class="grid-item pnl-area">
      <PnLEquityCurve />
    </div>

    <div class="grid-item status-area">
      <StatusPanel />
    </div>

    <div class="grid-item feature-area">
      <FeatureInspector />
    </div>

    <div class="grid-item health-area">
      <ModelHealth />
    </div>

    <div class="grid-item trade-area">
      <TradeHistory />
    </div>
  </div>
</div>

<style>
  .dashboard {
    padding: 1rem;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 1rem;
  }

  .chart-area { grid-column: 1 / 2; }
  .pnl-area { grid-column: 2 / 3; }
  .status-area { grid-column: 3 / 4; }
  .feature-area { grid-column: 1 / 3; }
  .health-area { grid-column: 3 / 4; }
  .trade-area { grid-column: 1 / 4; }

  @media (max-width: 1024px) {
    .grid {
      grid-template-columns: repeat(2, 1fr);
    }
    .chart-area { grid-column: 1 / 3; }
    .pnl-area { grid-column: 1 / 2; }
    .status-area { grid-column: 2 / 3; }
    .feature-area { grid-column: 1 / 3; }
    .health-area { grid-column: 1 / 3; }
    .trade-area { grid-column: 1 / 3; }
  }

  @media (max-width: 640px) {
    .grid {
      grid-template-columns: 1fr;
    }
    .grid-item {
      grid-column: 1 / 2 !important;
    }
  }
</style>
