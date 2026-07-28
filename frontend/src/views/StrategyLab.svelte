<script>
  import ParamSlider from '../lib/components/ParamSlider.svelte';
  import RhaiEditor from '../lib/components/RhaiEditor.svelte';
  import EquityCurveChart from '../lib/components/EquityCurveChart.svelte';
  import MetricsTable from '../lib/components/MetricsTable.svelte';
  import ABComparison from '../lib/components/ABComparison.svelte';
  import { fetchBacktest, fetchStrategies, saveStrategy } from '../lib/api.js';

  // Tab state
  let activeTab = 'standard'; // 'standard' | 'advanced'

  // Standard mode params
  let entryThreshold = 0.004;
  let exitThreshold = -0.002;
  let smaWindow = 150;

  // Advanced mode
  let rhaiScript = `// Default Threshold Strategy (Rhai)
// Entry: when signal > entry_threshold
// Exit: when signal < exit_threshold

let entry_threshold = 0.004;
let exit_threshold = -0.002;
let sma_window = 150;

fn should_enter(ctx) {
    ctx.signal > entry_threshold
}

fn should_exit(ctx) {
    ctx.signal < exit_threshold
}
`;

  // Date range
  let startDate = '';
  let endDate = '';

  // Strategy name
  let strategyName = '';

  // Loading / error states
  let loading = false;
  let error = '';
  let saveError = '';
  let saveSuccess = '';

  // Results
  let equityCurve = [];
  let buyHoldCurve = [];
  let metrics = {};
  let trades = [];

  // Saved strategies for A/B comparison
  let savedStrategies = [];

  // A/B comparison results
  let compareResultA = null;
  let compareResultB = null;

  // Initialize date range to last year
  function initDates() {
    const now = new Date();
    const oneYearAgo = new Date(now);
    oneYearAgo.setFullYear(oneYearAgo.getFullYear() - 1);
    endDate = now.toISOString().split('T')[0];
    startDate = oneYearAgo.toISOString().split('T')[0];
  }
  initDates();

  // Load saved strategies on mount
  async function loadStrategies() {
    try {
      savedStrategies = await fetchStrategies();
    } catch (e) {
      // Silently fail — strategies will just be empty
    }
  }
  loadStrategies();

  // Run backtest
  async function runBacktest() {
    loading = true;
    error = '';

    const startTs = startDate ? Math.floor(new Date(startDate).getTime() / 1000) : undefined;
    const endTs = endDate ? Math.floor(new Date(endDate).getTime() / 1000) : undefined;

    try {
      let params;
      if (activeTab === 'standard') {
        params = {
          kind: 'threshold',
          params: {
            entry_threshold: entryThreshold,
            exit_threshold: exitThreshold,
            sma_window: smaWindow,
          },
          start_ts: startTs,
          end_ts: endTs,
        };
      } else {
        params = {
          kind: 'rhai',
          params: {
            script: rhaiScript,
          },
          start_ts: startTs,
          end_ts: endTs,
        };
      }

      const result = await fetchBacktest(params);
      equityCurve = result.equity_curve || [];
      metrics = result.metrics || {};

      // Build buy & hold curve from metrics if available
      if (equityCurve.length > 0 && metrics.buy_hold_return != null) {
        const firstEq = equityCurve[0][1];
        const buyHoldReturn = metrics.buy_hold_return;
        buyHoldCurve = equityCurve.map(([ts, _]) => {
          const fraction = (ts - equityCurve[0][0]) / (equityCurve[equityCurve.length - 1][0] - equityCurve[0][0] || 1);
          return [ts, firstEq * (1 + buyHoldReturn * fraction)];
        });
      } else {
        buyHoldCurve = [];
      }

      trades = result.trades || [];
    } catch (e) {
      error = e.message;
      equityCurve = [];
      buyHoldCurve = [];
      metrics = {};
      trades = [];
    } finally {
      loading = false;
    }
  }

  // Save strategy
  async function handleSave() {
    if (!strategyName.trim()) {
      saveError = 'Please enter a strategy name.';
      return;
    }

    saveError = '';
    saveSuccess = '';

    try {
      let config;
      if (activeTab === 'standard') {
        config = {
          name: strategyName.trim(),
          strategy_type: 'threshold',
          params_json: JSON.stringify({
            entry_threshold: entryThreshold,
            exit_threshold: exitThreshold,
            sma_window: smaWindow,
          }),
        };
      } else {
        config = {
          name: strategyName.trim(),
          strategy_type: 'rhai',
          script_body: rhaiScript,
          params_json: JSON.stringify({}),
        };
      }

      await saveStrategy(config);
      saveSuccess = `Strategy "${strategyName.trim()}" saved successfully.`;
      strategyName = '';
      await loadStrategies();
    } catch (e) {
      saveError = e.message;
    }
  }

  // Handle A/B comparison results
  function handleCompare(event) {
    compareResultA = event.detail.a;
    compareResultB = event.detail.b;
  }

  // Format timestamp for display
  function fmtTs(ts) {
    return new Date(ts * 1000).toLocaleDateString();
  }

  // Trade count for display
  $: tradeCount = trades.length;
</script>

<div class="strategy-lab">
  <h1 class="page-title">Strategy Lab</h1>

  <!-- Tabs -->
  <div class="tabs">
    <button
      class="tab-btn"
      class:active={activeTab === 'standard'}
      on:click={() => activeTab = 'standard'}
    >
      Standard Mode
    </button>
    <button
      class="tab-btn"
      class:active={activeTab === 'advanced'}
      on:click={() => activeTab = 'advanced'}
    >
      Advanced Mode
    </button>
  </div>

  <!-- Parameter Configuration -->
  <div class="card">
    <div class="card-header">Strategy Configuration</div>

    {#if activeTab === 'standard'}
      <div class="params-grid">
        <ParamSlider
          label="Entry Threshold"
          min={0.001}
          max={0.010}
          step={0.001}
          bind:value={entryThreshold}
          dp={3}
        />
        <ParamSlider
          label="Exit Threshold"
          min={-0.005}
          max={0.000}
          step={0.0005}
          bind:value={exitThreshold}
          dp={3}
        />
        <ParamSlider
          label="SMA Window"
          min={50}
          max={300}
          step={10}
          bind:value={smaWindow}
          dp={0}
          unit=" bars"
        />
      </div>
    {:else}
      <RhaiEditor bind:value={rhaiScript} />
    {/if}
  </div>

  <!-- Date Range + Save -->
  <div class="card">
    <div class="card-header">Date Range &amp; Save</div>
    <div class="date-save-row">
      <div class="date-group">
        <label for="start-date">Start Date</label>
        <input id="start-date" type="date" bind:value={startDate} />
      </div>
      <div class="date-group">
        <label for="end-date">End Date</label>
        <input id="end-date" type="date" bind:value={endDate} />
      </div>

      <div class="save-group">
        <label for="strat-name">Strategy Name</label>
        <div class="save-row">
          <input
            id="strat-name"
            type="text"
            placeholder="My Strategy"
            bind:value={strategyName}
          />
          <button class="save-btn" on:click={handleSave}>Save</button>
        </div>
        {#if saveError}
          <div class="save-msg save-error">{saveError}</div>
        {/if}
        {#if saveSuccess}
          <div class="save-msg save-success">{saveSuccess}</div>
        {/if}
      </div>
    </div>
  </div>

  <!-- Run Backtest -->
  <div class="run-section">
    <button
      class="run-btn"
      disabled={loading}
      on:click={runBacktest}
    >
      {#if loading}
        <span class="spinner"></span> Running Backtest...
      {:else}
        ▶ Run Backtest
      {/if}
    </button>
    {#if error}
      <div class="error-msg">Error: {error}</div>
    {/if}
  </div>

  <!-- Results -->
  {#if equityCurve.length > 0}
    <div class="results-section">
      <h2 class="section-title">Results</h2>

      <div class="results-grid">
        <div class="card card-span">
          <div class="card-header">Equity Curve</div>
          <EquityCurveChart
            equityCurve={equityCurve}
            benchmarkCurve={buyHoldCurve.length > 0 ? buyHoldCurve : null}
            label="Strategy"
            benchmarkLabel="Buy & Hold"
          />
        </div>

        <MetricsTable {metrics} />

        {#if tradeCount > 0}
          <div class="card card-span">
            <div class="card-header">Trade History ({tradeCount} trades)</div>
            <div class="trade-list">
              <table>
                <thead>
                  <tr>
                    <th>Date</th>
                    <th>Action</th>
                    <th>Price</th>
                    <th>PnL</th>
                  </tr>
                </thead>
                <tbody>
                  {#each trades.slice(-20) as trade}
                    <tr>
                      <td>{fmtTs(trade.ts)}</td>
                      <td class="action-{trade.action || 'entry'}">{trade.action || 'ENTRY'}</td>
                      <td class="mono">{trade.price != null ? trade.price.toFixed(2) : '—'}</td>
                      <td class="mono {trade.pnl > 0 ? 'pos' : trade.pnl < 0 ? 'neg' : ''}">
                        {trade.pnl != null ? trade.pnl.toFixed(2) : '—'}
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          </div>
        {/if}
      </div>
    </div>
  {/if}

  <!-- A/B Comparison -->
  <div class="card">
    <div class="card-header">A/B Comparison</div>
    <ABComparison
      strategies={savedStrategies}
      on:compare={handleCompare}
    />
  </div>

  {#if compareResultA && compareResultB}
    <div class="results-section">
      <h2 class="section-title">Comparison Results</h2>
      <div class="results-grid">
        <div class="card card-span">
          <div class="card-header">
            {compareResultA.name} vs {compareResultB.name}
          </div>
          <EquityCurveChart
            equityCurve={compareResultA.equity_curve || []}
            benchmarkCurve={compareResultB.equity_curve || []}
            label={compareResultA.name}
            benchmarkLabel={compareResultB.name}
          />
        </div>
        <MetricsTable metrics={compareResultA.metrics || {}} />
        <MetricsTable metrics={compareResultB.metrics || {}} />
      </div>
    </div>
  {/if}
</div>

<style>
  .strategy-lab {
    padding: 1.5rem;
    max-width: 1200px;
  }

  .page-title {
    font-size: 1.4rem;
    font-weight: 600;
    color: #c9d1d9;
    margin-bottom: 1.25rem;
  }

  /* Tabs */
  .tabs {
    display: flex;
    gap: 0;
    margin-bottom: 1.25rem;
    border-bottom: 1px solid #30363d;
  }

  .tab-btn {
    padding: 0.6rem 1.2rem;
    cursor: pointer;
    color: #8b949e;
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    font-size: 0.9rem;
    font-family: inherit;
    transition: all 0.15s;
  }

  .tab-btn:hover {
    color: #c9d1d9;
    background: #21262d;
  }

  .tab-btn.active {
    color: #58a6ff;
    border-bottom-color: #58a6ff;
  }

  /* Cards */
  .card {
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 8px;
    padding: 1rem;
    margin-bottom: 1rem;
  }

  .card-header {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #8b949e;
    margin-bottom: 0.75rem;
    padding-bottom: 0.5rem;
    border-bottom: 1px solid #21262d;
  }

  .card-span {
    grid-column: 1 / -1;
  }

  /* Params grid */
  .params-grid {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  /* Date & Save */
  .date-save-row {
    display: flex;
    gap: 1rem;
    align-items: flex-end;
    flex-wrap: wrap;
  }

  .date-group,
  .save-group {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    flex: 1;
    min-width: 160px;
  }

  .date-group label,
  .save-group label {
    font-size: 0.75rem;
    color: #8b949e;
  }

  .date-group input[type="date"] {
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 4px;
    color: #c9d1d9;
    padding: 0.4rem 0.5rem;
    font-size: 0.82rem;
    font-family: inherit;
    outline: none;
  }

  .date-group input[type="date"]:focus {
    border-color: #58a6ff;
  }

  .save-row {
    display: flex;
    gap: 0.5rem;
  }

  .save-row input[type="text"] {
    flex: 1;
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 4px;
    color: #c9d1d9;
    padding: 0.4rem 0.5rem;
    font-size: 0.82rem;
    font-family: inherit;
    outline: none;
  }

  .save-row input[type="text"]:focus {
    border-color: #58a6ff;
  }

  .save-btn {
    padding: 0.4rem 1rem;
    background: #21262d;
    border: 1px solid #30363d;
    border-radius: 6px;
    color: #c9d1d9;
    font-size: 0.82rem;
    font-family: inherit;
    cursor: pointer;
    transition: background 0.15s;
    white-space: nowrap;
  }

  .save-btn:hover {
    background: #30363d;
  }

  .save-msg {
    font-size: 0.75rem;
    margin-top: 0.3rem;
  }

  .save-error {
    color: #f85149;
  }

  .save-success {
    color: #3fb950;
  }

  /* Run Button */
  .run-section {
    margin-bottom: 1rem;
    display: flex;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
  }

  .run-btn {
    padding: 0.65rem 1.75rem;
    background: #238636;
    border: 1px solid #2ea043;
    border-radius: 6px;
    color: #fff;
    font-size: 0.95rem;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
    transition: background 0.15s;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .run-btn:hover:not(:disabled) {
    background: #2ea043;
  }

  .run-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .error-msg {
    font-size: 0.85rem;
    color: #f85149;
    background: #da363322;
    border: 1px solid #da363344;
    border-radius: 4px;
    padding: 0.4rem 0.75rem;
  }

  /* Spinner */
  .spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255,255,255,0.3);
    border-top-color: #fff;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* Results */
  .results-section {
    margin-bottom: 1rem;
  }

  .section-title {
    font-size: 1.1rem;
    font-weight: 600;
    color: #c9d1d9;
    margin-bottom: 0.75rem;
  }

  .results-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
  }

  @media (max-width: 900px) {
    .results-grid {
      grid-template-columns: 1fr;
    }
  }

  /* Trade list */
  .trade-list {
    max-height: 300px;
    overflow-y: auto;
  }

  .trade-list table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.82rem;
  }

  .trade-list th {
    text-align: left;
    padding: 0.4rem 0.5rem;
    color: #8b949e;
    font-weight: 500;
    border-bottom: 1px solid #30363d;
    position: sticky;
    top: 0;
    background: #161b22;
  }

  .trade-list td {
    padding: 0.35rem 0.5rem;
    color: #c9d1d9;
    border-bottom: 1px solid #21262d;
  }

  .mono {
    font-family: monospace;
    font-variant-numeric: tabular-nums;
  }

  .pos { color: #3fb950; }
  .neg { color: #f85149; }

  .action-long,
  .action-entry { color: #3fb950; }
  .action-short,
  .action-exit { color: #f85149; }

  .trade-list::-webkit-scrollbar {
    width: 6px;
  }

  .trade-list::-webkit-scrollbar-track {
    background: #0d1117;
  }

  .trade-list::-webkit-scrollbar-thumb {
    background: #30363d;
    border-radius: 3px;
  }
</style>