<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchStatus, fetchPredictions, fetchChart, fetchAccuracy, fetchEquityTrades, fetchModels } from '../lib/api.js';
  import { connectWebSocket, disconnectWebSocket } from '../lib/websocket.js';
  import { status, predictions, chartData, accuracy, trades, activeModelId, models, setSlice } from '../lib/stores.js';

  import StatusPanel from '../lib/components/StatusPanel.svelte';
  import CandlestickChart from '../lib/components/CandlestickChart.svelte';
  import PnLEquityCurve from '../lib/components/PnLEquityCurve.svelte';
  import FeatureInspector from '../lib/components/FeatureInspector.svelte';
  import TradeHistory from '../lib/components/TradeHistory.svelte';
  import ModelHealth from '../lib/components/ModelHealth.svelte';
  import StrategyConfigPanel from '../lib/components/StrategyConfigPanel.svelte';

  let chartComponent;
  let statusInterval;

  // §8: Load the model registry, pick the first enabled model as active,
  // then fetch per-model data into the active model's slice.
  onMount(async () => {
    try {
      const modelList = await fetchModels();
      models.set(modelList);
      // Pick the first enabled model (or first model if none enabled).
      const first = modelList.find((m) => m.enabled) || modelList[0];
      if (first) {
        activeModelId.set(first.model_id);
        await loadModelData(first.model_id, first.primary_symbol);
      }
    } catch (e) {
      console.error('Failed to fetch models:', e);
      // Fallback: try the old single-model path with no model_id
      await loadModelData(null, 'QQQ');
    }

    connectWebSocket();

    statusInterval = setInterval(async () => {
      const mid = $activeModelId;
      if (!mid) return;
      try {
        const s = await fetchStatus();
        setSlice(mid, 'status', s);
      } catch (e) {
        // Silent — WS may still be delivering updates
      }
    }, 30000);
  });

  async function loadModelData(modelId, symbol) {
    // Fetch per-model data into the model's slice.
    const mid = modelId || 'legacy';

    try {
      const s = await fetchStatus();
      setSlice(mid, 'status', s);
    } catch (e) {
      console.error('Failed to fetch status:', e);
    }

    try {
      const p = await fetchPredictions();
      setSlice(mid, 'predictions', p);
    } catch (e) {
      console.error('Failed to fetch predictions:', e);
    }

    try {
      const c = await fetchChart();
      setSlice(mid, 'chartData', c);
    } catch (e) {
      console.error('Failed to fetch chart:', e);
    }

    try {
      const a = await fetchAccuracy();
      if (a) setSlice(mid, 'accuracy', a);
    } catch (e) {
      console.error('Failed to fetch accuracy:', e);
    }

    try {
      const td = await fetchEquityTrades('*', 500);
      setSlice(mid, 'trades', (td.trades || []).map((t) => ({
        time: t.ts,
        symbol: t.symbol,
        side: t.side,
        qty: t.qty,
        price: t.price,
        fee: t.fee,
        realized_pnl: t.realized_pnl,
      })));
    } catch (e) {
      console.error('Failed to fetch trades:', e);
    }
  }

  // When the active model changes, load its data if the slice is empty.
  let lastLoadedModel = null;
  $: if ($activeModelId && $activeModelId !== lastLoadedModel) {
    lastLoadedModel = $activeModelId;
    const m = $models.find((mm) => mm.model_id === $activeModelId);
    if (m) {
      loadModelData(m.model_id, m.primary_symbol);
    }
  }

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

  function onModelChange(e) {
    activeModelId.set(e.target.value);
  }

  $: modelOptions = $models.map((m) => ({
    model_id: m.model_id,
    label: `${m.primary_symbol}/${m.inverse_symbol}`,
    enabled: m.enabled,
  }));
</script>

<div class="dashboard">
  <div class="dash-header">
    <h1>Dashboard</h1>
    <select class="model-selector" value={$activeModelId} on:change={onModelChange}>
      {#each modelOptions as opt}
        <option value={opt.model_id} disabled={!opt.enabled}>
          {opt.label}{#if !opt.enabled} (disabled){/if}
        </option>
      {/each}
    </select>
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

  .model-selector {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    color: var(--text-primary);
    padding: 0.3rem 0.5rem;
    border-radius: var(--radius-xs);
    font-size: 0.82rem;
    font-family: var(--font-mono);
    cursor: pointer;
  }

  .model-selector:focus {
    outline: none;
    border-color: var(--accent);
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
