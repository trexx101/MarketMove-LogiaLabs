<script>
  import { onMount, onDestroy } from 'svelte';
  import { createChart, CandlestickSeries, LineSeries, createSeriesMarkers } from 'lightweight-charts';
  import { fetchChart, fetchEquityTrades } from '../api.js';
  import { status, predictions, activeModelId, models, chartData } from '../stores.js';

  // ── Props ────────────────────────────────────────────────────────────────
  export let symbol = null;

  // ── State ────────────────────────────────────────────────────────────────
  let container;
  let chart = null;
  let candleSeries = null;
  let markersPlugin = null;
  let sma20 = null, sma50 = null, sma200 = null;
  let tenkan = null, kijun = null, senkouA = null, senkouB = null, chikou = null;
  let chartTimer;
  let resizeObserver;

  // Toggles
  let showSMA = true;
  let showIchimoku = true;
  let showPreds = true;

  // Data
  let candles = [];
  let indicators = null;
  let predictionMarkers = [];
  let livePrice = null;
  let trades = [];
  let preds = null;
  let lastClose = null;
  let stale = false;
  let liveQuote = null;

  // Lookup maps for crosshair hover
  let candleByTime = new Map();
  let predByTime = new Map();

  // Hover legend state
  let crosshairInfo = null;

  // Resolve the symbol
  $: resolvedSymbol = symbol || ($models.find((m) => m.model_id === $activeModelId)?.primary_symbol) || 'QQQ';

  // Timeframe selector — limit-based (matches plan Decision D).
  const TIMEFRAMES = [
    { label: '1M',  limit: 21   },
    { label: '3M',  limit: 63   },
    { label: '6M',  limit: 126  },
    { label: '1Y',  limit: 252  },
    { label: 'ALL', limit: 1500 },
  ];
  let activeTimeframe = '6M';

  // ── Data fetching ───────────────────────────────────────────────────────

  async function refreshChart() {
    try {
      const tf = TIMEFRAMES.find((t) => t.label === activeTimeframe) ?? TIMEFRAMES[2];
      const data = await fetchChart(tf.limit, resolvedSymbol);
      candles = data.candles || [];
      indicators = data.indicators || null;
      predictionMarkers = data.predictions || [];
      stale = !!data.stale;

      if (data.live_quote) {
        livePrice = data.live_quote.price;
        liveQuote = data.live_quote;
      }

      updateChart();
    } catch (e) {
      console.warn('chart refresh failed:', e.message);
    }
  }

  async function refreshTrades() {
    try {
      const data = await fetchEquityTrades(resolvedSymbol, 200);
      trades = data.trades || [];
      updateTradeMarkers();
    } catch (e) {
      // silent — no trades yet is fine
    }
  }

  function setTimeframe(label) {
    if (label === activeTimeframe) return;
    activeTimeframe = label;
    refreshChart();
  }

  // ── Exposed hooks (called by Dashboard) ─────────────────────────────────
  export function setPredictions(p, close) {
    preds = {
      pred_1d:  p.pred_1d  ?? p.pred_24h ?? p.pred_1h,
      pred_5d:  p.pred_5d  ?? p.pred_5h_approx,
      pred_21d: p.pred_21d ?? null,
    };
    lastClose = close;
  }

  export function setLivePrice(price) {
    livePrice = price;
  }

  // ── Chart update functions ──────────────────────────────────────────────

  function updateChart() {
    if (!chart || !candles.length) return;

    const candleData = candles.map((c) => ({
      time: c.time,
      open: c.open,
      high: c.high,
      low: c.low,
      close: c.close,
    }));
    candleSeries.setData(candleData);

    // Build lookup maps for crosshair hover
    candleByTime = new Map(candles.map((c) => [c.time, c]));
    predByTime = new Map(predictionMarkers.map((m) => [m.candle_ts, m]));

    updateSMA();
    updateIchimoku();
    updatePredictionMarkers();
    updateTradeMarkers();

    chart.timeScale().fitContent();
  }

  function updateSMA() {
    if (!indicators?.sma || !showSMA) {
      sma20.setData([]);
      sma50.setData([]);
      sma200.setData([]);
      return;
    }

    const mapSMA = (values) =>
      values
        .map((v, i) => (v != null ? { time: candles[i]?.time, value: v } : null))
        .filter(Boolean);

    sma20.setData(mapSMA(indicators.sma['20']));
    sma50.setData(mapSMA(indicators.sma['50']));
    sma200.setData(mapSMA(indicators.sma['200']));
  }

  function updateIchimoku() {
    if (!indicators?.ichimoku || !showIchimoku) {
      [tenkan, kijun, senkouA, senkouB, chikou].forEach((s) => s && s.setData([]));
      return;
    }

    const ichi = indicators.ichimoku;
    const mapIchi = (values) =>
      values
        .map((v, i) => (v != null ? { time: candles[i]?.time, value: v } : null))
        .filter(Boolean);

    tenkan.setData(mapIchi(ichi.tenkan));
    kijun.setData(mapIchi(ichi.kijun));
    senkouA.setData(mapIchi(ichi.senkou_a));
    senkouB.setData(mapIchi(ichi.senkou_b));
    chikou.setData(mapIchi(ichi.chikou));
  }

  function updatePredictionMarkers() {
    if (!showPreds || !predictionMarkers.length) {
      markersPlugin.setMarkers([]);
      return;
    }

    // Show markers for predictions with all horizons resolved (actuals present).
    // No text — clean glyphs only. Color = direction hit (green) or miss (red).
    const markers = predictionMarkers
      .filter((m) => m.actual_1d != null || m.actual_5d != null || m.actual_21d != null)
      .map((m) => {
        const pred = m.pred_5d;
        const actual = m.actual_5d ?? m.actual_1d ?? 0;
        const diff = actual - pred;
        const correctDir = (pred >= 0 && actual >= 0) || (pred < 0 && actual < 0);
        const color = correctDir ? '#149e61' : '#e5484d';

        return {
          time: m.candle_ts,
          position: diff >= 0 ? 'belowBar' : 'aboveBar',
          color,
          shape: 'circle',
          text: '',
          size: 2,
        };
      });

    markersPlugin.setMarkers(markers);
  }

  function updateTradeMarkers() {
    if (!trades.length) return;

    // Merge trade markers with prediction markers.
    const existingMarkers = markersPlugin.markers();
    const tradeMarkers = trades.map((t) => ({
      time: t.ts,
      position: t.side === 'long' ? 'belowBar' : 'aboveBar',
      color: t.side === 'long' ? '#149e61' : '#e5484d',
      shape: t.side === 'long' ? 'arrowUp' : 'arrowDown',
      text: t.side === 'long' ? 'L' : 'S',
      size: 3,
    }));

    // Prediction markers + trade markers (trade markers on top).
    const predMarkers = existingMarkers;
    markersPlugin.setMarkers([...predMarkers, ...tradeMarkers]);
  }

  // ── Helpers for crosshair legend ────────────────────────────────────────

  function fmtTime(t) {
    if (t == null) return '';
    if (typeof t === 'number') {
      // Unix timestamp
      const d = new Date(t * 1000);
      return d.toISOString().slice(0, 10);
    }
    return String(t).slice(0, 10);
  }

  function fmtPct(v) {
    if (v == null) return '—';
    const sign = v >= 0 ? '+' : '';
    return `${sign}${(v * 100).toFixed(1)}%`;
  }

  function fmtPrice(v) {
    if (v == null) return '—';
    return v.toFixed(2);
  }

  // ── Lifecycle ────────────────────────────────────────────────────────────

  onMount(async () => {
    // Create chart
    chart = createChart(container, {
      layout: {
        background: { color: '#0c0d12' },
        textColor: '#8b8d9a',
      },
      grid: {
        vertLines: { color: '#1c1d27' },
        horzLines: { color: '#1c1d27' },
      },
      crosshair: {
        mode: 1,
      },
      rightPriceScale: {
        borderColor: '#252631',
        scaleMargins: { top: 0.1, bottom: 0.1 },
      },
      timeScale: {
        borderColor: '#252631',
        timeVisible: true,
        secondsVisible: false,
      },
      width: container.clientWidth,
      height: container.clientHeight || 420,
    });

    // Candlestick series
    candleSeries = chart.addSeries(CandlestickSeries, {
      upColor: '#149e61',
      downColor: '#e5484d',
      borderUpColor: '#149e61',
      borderDownColor: '#e5484d',
      wickUpColor: '#149e61',
      wickDownColor: '#e5484d',
    });

    // Markers plugin (v5 API: createSeriesMarkers replaces series.setMarkers)
    markersPlugin = createSeriesMarkers(candleSeries, []);

    // SMA lines
    sma20 = chart.addSeries(LineSeries, {
      color: '#7132f5',
      lineWidth: 1,
      priceLineVisible: false,
      lastValueVisible: false,
    });
    sma50 = chart.addSeries(LineSeries, {
      color: '#d29922',
      lineWidth: 1,
      priceLineVisible: false,
      lastValueVisible: false,
    });
    sma200 = chart.addSeries(LineSeries, {
      color: '#e5484d',
      lineWidth: 1,
      priceLineVisible: false,
      lastValueVisible: false,
    });

    // Ichimoku lines
    tenkan = chart.addSeries(LineSeries, {
      color: '#5c5e6e',
      lineWidth: 1,
      priceLineVisible: false,
      lastValueVisible: false,
    });
    kijun = chart.addSeries(LineSeries, {
      color: '#8b8d9a',
      lineWidth: 1.5,
      priceLineVisible: false,
      lastValueVisible: false,
    });
    senkouA = chart.addSeries(LineSeries, {
      color: 'rgba(113, 50, 245, 0.35)',
      lineWidth: 1,
      lineStyle: 2,
      priceLineVisible: false,
      lastValueVisible: false,
    });
    senkouB = chart.addSeries(LineSeries, {
      color: 'rgba(113, 50, 245, 0.35)',
      lineWidth: 1,
      lineStyle: 2,
      priceLineVisible: false,
      lastValueVisible: false,
    });
    chikou = chart.addSeries(LineSeries, {
      color: '#d29922',
      lineWidth: 1,
      lineStyle: 2,
      priceLineVisible: false,
      lastValueVisible: false,
    });

    // Crosshair subscription — drives the hover legend
    chart.subscribeCrosshairMove((param) => {
      if (!param.time || !param.point) {
        crosshairInfo = null;
        return;
      }

      const candle = candleByTime.get(param.time);
      if (!candle) {
        crosshairInfo = null;
        return;
      }

      const pred = predByTime.get(param.time) || null;

      crosshairInfo = {
        time: param.time,
        symbol: resolvedSymbol,
        open: candle.open,
        high: candle.high,
        low: candle.low,
        close: candle.close,
        pred_1d: pred?.pred_1d ?? null,
        pred_5d: pred?.pred_5d ?? null,
        pred_21d: pred?.pred_21d ?? null,
        actual_1d: pred?.actual_1d ?? null,
        actual_5d: pred?.actual_5d ?? null,
        actual_21d: pred?.actual_21d ?? null,
        // Forward preds for the last candle only
        forward_1d: null,
        forward_5d: null,
        forward_21d: null,
      };

      // If this is the last candle, attach forward predictions
      if (preds && lastClose != null) {
        const lastTime = candles[candles.length - 1]?.time;
        if (param.time === lastTime) {
          crosshairInfo.forward_1d = preds.pred_1d ?? null;
          crosshairInfo.forward_5d = preds.pred_5d ?? null;
          crosshairInfo.forward_21d = preds.pred_21d ?? null;
        }
      }
    });

    // Load data
    await refreshChart();
    await refreshTrades();

    // Resize handling
    resizeObserver = new ResizeObserver(() => {
      if (chart && container) {
        chart.applyOptions({
          width: container.clientWidth,
          height: container.clientHeight || 420,
        });
      }
    });
    resizeObserver.observe(container);

    // Polling
    chartTimer = setInterval(async () => {
      await refreshChart();
      await refreshTrades();
    }, 30_000);
  });

  onDestroy(() => {
    if (resizeObserver) resizeObserver.disconnect();
    if (chartTimer) clearInterval(chartTimer);
    if (chart) {
      chart.remove();
      chart = null;
    }
  });

  // ── Store reactivity ─────────────────────────────────────────────────────

  $: if ($chartData && $chartData.candles) {
    candles = $chartData.candles || candles;
    indicators = $chartData.indicators || indicators;
    predictionMarkers = $chartData.predictions || predictionMarkers;
    updateChart();
  }

  $: if ($predictions) {
    const p = $predictions.latest;
    if (p) {
      preds = {
        pred_1d:  p.pred_1d  ?? p.pred_24h ?? p.pred_1h,
        pred_5d:  p.pred_5d  ?? p.pred_5h_approx,
        pred_21d: p.pred_21d ?? null,
      };
    }
  }

  $: if ($status) {
    lastClose = $status.last_close ?? lastClose;
  }
</script>

<div class="chart-wrap">
  <div class="chart-toolbar">
    <div class="timeframe-group">
      {#each TIMEFRAMES as tf (tf.label)}
        <button
          class="tf-btn"
          class:active={activeTimeframe === tf.label}
          on:click={() => setTimeframe(tf.label)}
        >
          {tf.label}
        </button>
      {/each}
    </div>

    <div class="toggle-group">
      <button class="toggle-btn" class:active={showSMA} on:click={() => { showSMA = !showSMA; updateChart(); }}>
        SMA
      </button>
      <button class="toggle-btn" class:active={showIchimoku} on:click={() => { showIchimoku = !showIchimoku; updateChart(); }}>
        Ichimoku
      </button>
      <button class="toggle-btn" class:active={showPreds} on:click={() => { showPreds = !showPreds; updateChart(); }}>
        Preds
      </button>
    </div>

    {#if stale}
      <span class="stale-badge">STALE</span>
    {/if}
  </div>

  <div class="chart-canvas" bind:this={container}>
    {#if crosshairInfo}
      <div class="crosshair-legend">
        <div class="cl-row cl-header">
          <span class="cl-symbol">{crosshairInfo.symbol}</span>
          <span class="cl-date">{fmtTime(crosshairInfo.time)}</span>
        </div>
        <div class="cl-row cl-ohlc">
          <span class="cl-label">O</span><span class="cl-val">{fmtPrice(crosshairInfo.open)}</span>
          <span class="cl-label">H</span><span class="cl-val">{fmtPrice(crosshairInfo.high)}</span>
          <span class="cl-label">L</span><span class="cl-val">{fmtPrice(crosshairInfo.low)}</span>
          <span class="cl-label">C</span><span class="cl-val">{fmtPrice(crosshairInfo.close)}</span>
        </div>
        {#if crosshairInfo.pred_1d != null || crosshairInfo.pred_5d != null || crosshairInfo.pred_21d != null}
          <div class="cl-row cl-preds">
            <span class="cl-label">Pred</span>
            <span class="cl-horizon">1d:{fmtPct(crosshairInfo.pred_1d)}</span>
            <span class="cl-horizon">5d:{fmtPct(crosshairInfo.pred_5d)}</span>
            <span class="cl-horizon">21d:{fmtPct(crosshairInfo.pred_21d)}</span>
          </div>
        {/if}
        {#if crosshairInfo.actual_1d != null || crosshairInfo.actual_5d != null || crosshairInfo.actual_21d != null}
          <div class="cl-row cl-actuals">
            <span class="cl-label">Act</span>
            <span class="cl-horizon">
              1d:{fmtPct(crosshairInfo.actual_1d)}
              {#if crosshairInfo.actual_1d != null}
                <span class="cl-dir" class:cl-hit={(crosshairInfo.pred_1d >= 0 && crosshairInfo.actual_1d >= 0) || (crosshairInfo.pred_1d < 0 && crosshairInfo.actual_1d < 0)} class:cl-miss={!((crosshairInfo.pred_1d >= 0 && crosshairInfo.actual_1d >= 0) || (crosshairInfo.pred_1d < 0 && crosshairInfo.actual_1d < 0))}>
                  {((crosshairInfo.pred_1d >= 0 && crosshairInfo.actual_1d >= 0) || (crosshairInfo.pred_1d < 0 && crosshairInfo.actual_1d < 0)) ? '✓' : '✗'}
                </span>
              {/if}
            </span>
            <span class="cl-horizon">
              5d:{fmtPct(crosshairInfo.actual_5d)}
              {#if crosshairInfo.actual_5d != null}
                <span class="cl-dir" class:cl-hit={(crosshairInfo.pred_5d >= 0 && crosshairInfo.actual_5d >= 0) || (crosshairInfo.pred_5d < 0 && crosshairInfo.actual_5d < 0)} class:cl-miss={!((crosshairInfo.pred_5d >= 0 && crosshairInfo.actual_5d >= 0) || (crosshairInfo.pred_5d < 0 && crosshairInfo.actual_5d < 0))}>
                  {((crosshairInfo.pred_5d >= 0 && crosshairInfo.actual_5d >= 0) || (crosshairInfo.pred_5d < 0 && crosshairInfo.actual_5d < 0)) ? '✓' : '✗'}
                </span>
              {/if}
            </span>
            <span class="cl-horizon">
              21d:{fmtPct(crosshairInfo.actual_21d)}
              {#if crosshairInfo.actual_21d != null}
                <span class="cl-dir" class:cl-hit={(crosshairInfo.pred_21d >= 0 && crosshairInfo.actual_21d >= 0) || (crosshairInfo.pred_21d < 0 && crosshairInfo.actual_21d < 0)} class:cl-miss={!((crosshairInfo.pred_21d >= 0 && crosshairInfo.actual_21d >= 0) || (crosshairInfo.pred_21d < 0 && crosshairInfo.actual_21d < 0))}>
                  {((crosshairInfo.pred_21d >= 0 && crosshairInfo.actual_21d >= 0) || (crosshairInfo.pred_21d < 0 && crosshairInfo.actual_21d < 0)) ? '✓' : '✗'}
                </span>
              {/if}
            </span>
          </div>
        {/if}
        {#if crosshairInfo.forward_1d != null || crosshairInfo.forward_5d != null || crosshairInfo.forward_21d != null}
          <div class="cl-row cl-forward">
            <span class="cl-label">Fwd</span>
            <span class="cl-horizon">1d:{fmtPct(crosshairInfo.forward_1d)}</span>
            <span class="cl-horizon">5d:{fmtPct(crosshairInfo.forward_5d)}</span>
            <span class="cl-horizon">21d:{fmtPct(crosshairInfo.forward_21d)}</span>
          </div>
        {/if}
      </div>
    {/if}
  </div>

  {#if liveQuote}
    <div class="live-bar">
      <span class="live-price">{liveQuote.price.toFixed(2)}</span>
      {#if liveQuote.change != null}
        <span class="live-change" class:positive={liveQuote.change >= 0} class:negative={liveQuote.change < 0}>
          {liveQuote.change >= 0 ? '+' : ''}{liveQuote.change.toFixed(2)}
          ({liveQuote.change_pct >= 0 ? '+' : ''}{liveQuote.change_pct.toFixed(2)}%)
        </span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .chart-wrap {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 420px;
    background: var(--bg-base, #0c0d12);
    border: 1px solid var(--border, #252631);
    border-radius: var(--radius, 8px);
    overflow: hidden;
  }

  .chart-toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border, #252631);
    flex-shrink: 0;
    flex-wrap: wrap;
  }

  .timeframe-group {
    display: flex;
    gap: 2px;
  }

  .tf-btn {
    background: var(--bg-surface, #15161e);
    color: var(--text-secondary, #8b8d9a);
    border: 1px solid var(--border, #252631);
    padding: 4px 10px;
    font-size: 0.75rem;
    font-family: var(--font-mono, monospace);
    cursor: pointer;
    border-radius: var(--radius-xs, 3px);
    transition: all 0.15s;
  }
  .tf-btn:hover {
    background: var(--bg-surface-hover, #1c1d27);
    color: var(--text-primary, #ececf1);
  }
  .tf-btn.active {
    background: var(--accent, #7132f5);
    color: #fff;
    border-color: var(--accent, #7132f5);
  }

  .toggle-group {
    display: flex;
    gap: 4px;
    margin-left: auto;
  }

  .toggle-btn {
    background: transparent;
    color: var(--text-muted, #5c5e6e);
    border: 1px solid var(--border, #252631);
    padding: 4px 8px;
    font-size: 0.7rem;
    font-family: var(--font-mono, monospace);
    cursor: pointer;
    border-radius: var(--radius-xs, 3px);
    transition: all 0.15s;
  }
  .toggle-btn:hover {
    color: var(--text-primary, #ececf1);
  }
  .toggle-btn.active {
    color: var(--accent, #7132f5);
    border-color: var(--accent, #7132f5);
    background: var(--accent-subtle, rgba(113, 50, 245, 0.1));
  }

  .stale-badge {
    background: var(--red-subtle, rgba(229, 72, 77, 0.15));
    color: var(--red, #e5484d);
    padding: 2px 8px;
    font-size: 0.65rem;
    font-family: var(--font-mono, monospace);
    border-radius: var(--radius-xs, 3px);
    letter-spacing: 0.06em;
  }

  .chart-canvas {
    flex: 1;
    min-height: 0;
    position: relative;
  }

  .live-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-top: 1px solid var(--border, #252631);
    font-family: var(--font-mono, monospace);
    font-size: 0.8rem;
    flex-shrink: 0;
  }
  .live-price {
    color: var(--text-primary, #ececf1);
    font-weight: 600;
  }
  .live-change.positive {
    color: var(--green, #149e61);
  }
  .live-change.negative {
    color: var(--red, #e5484d);
  }

  /* ── Crosshair hover legend ─────────────────────────────────────────── */

  .crosshair-legend {
    position: absolute;
    top: 8px;
    left: 8px;
    z-index: 10;
    background: rgba(12, 13, 18, 0.92);
    border: 1px solid var(--border, #252631);
    border-radius: 6px;
    padding: 8px 10px;
    font-family: var(--font-mono, monospace);
    font-size: 0.7rem;
    line-height: 1.5;
    pointer-events: none;
    min-width: 260px;
  }

  .cl-row {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }

  .cl-header {
    margin-bottom: 4px;
    padding-bottom: 4px;
    border-bottom: 1px solid var(--border, #252631);
  }

  .cl-symbol {
    color: var(--text-primary, #ececf1);
    font-weight: 600;
  }

  .cl-date {
    color: var(--text-secondary, #8b8d9a);
    margin-left: auto;
  }

  .cl-ohlc {
    margin-bottom: 3px;
  }

  .cl-label {
    color: var(--text-muted, #5c5e6e);
    width: 18px;
    flex-shrink: 0;
  }

  .cl-val {
    color: var(--text-primary, #ececf1);
    min-width: 52px;
    text-align: right;
  }

  .cl-preds, .cl-actuals, .cl-forward {
    margin-top: 1px;
  }

  .cl-horizon {
    color: var(--text-secondary, #8b8d9a);
    min-width: 70px;
  }

  .cl-dir {
    margin-left: 2px;
  }

  .cl-hit {
    color: var(--green, #149e61);
  }

  .cl-miss {
    color: var(--red, #e5484d);
  }

  .cl-forward .cl-horizon {
    color: var(--accent, #7132f5);
  }
</style>