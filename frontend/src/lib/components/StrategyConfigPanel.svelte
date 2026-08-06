<script>
  import { onMount } from 'svelte';
  import { fetchStrategyConfig, saveStrategyConfig } from '../api.js';
  import { activeModelId } from '../stores.js';

  let config = {
    entry_threshold: 0.001,
    exit_threshold: -0.0005,
    sma_window: 40,
    pred_5d_filter: false,
    enable_shorting: true,
    short_entry_threshold: -0.001,
    short_exit_threshold: 0.0005,
    enable_sentiment_overlay: false,
    sentiment_reduce_threshold: -0.5,
    sentiment_exit_threshold: -0.8,
    sentiment_min_articles: 15,
  };

  let dirty = false;
  let saving = false;
  let error = '';
  let success = '';

  // §8.6: reload config when the active model changes.
  let lastLoadedModel = null;
  $: if ($activeModelId && $activeModelId !== lastLoadedModel) {
    lastLoadedModel = $activeModelId;
    loadConfig();
  }

  async function loadConfig() {
    try {
      config = await fetchStrategyConfig($activeModelId);
      dirty = false;
    } catch (e) {
      error = `Failed to load: ${e.message}`;
    }
  }

  onMount(loadConfig);

  function markDirty() {
    dirty = true;
    success = '';
  }

  async function handleSave() {
    saving = true;
    error = '';
    success = '';
    try {
      config = await saveStrategyConfig(config, $activeModelId);
      dirty = false;
      success = 'Saved';
      setTimeout(() => (success = ''), 3000);
    } catch (e) {
      error = e.message;
    } finally {
      saving = false;
    }
  }

  function loadPreset(name) {
    if (name === 'optimal') {
      config = {
        entry_threshold: 0.001,
        exit_threshold: -0.0005,
        sma_window: 40,
        pred_5d_filter: false,
        enable_shorting: true,
        short_entry_threshold: -0.001,
        short_exit_threshold: 0.0005,
        enable_sentiment_overlay: false,
        sentiment_reduce_threshold: -0.5,
        sentiment_exit_threshold: -0.8,
        sentiment_min_articles: 15,
      };
    } else if (name === 'conservative') {
      config = {
        entry_threshold: 0.003,
        exit_threshold: -0.001,
        sma_window: 200,
        pred_5d_filter: true,
        enable_shorting: false,
        short_entry_threshold: -0.004,
        short_exit_threshold: 0.001,
        enable_sentiment_overlay: false,
        sentiment_reduce_threshold: -0.5,
        sentiment_exit_threshold: -0.8,
        sentiment_min_articles: 15,
      };
    }
    dirty = true;
  }

  function fmt(v, dec = 4) {
    if (v == null) return '';
    return Number(v).toFixed(dec);
  }
</script>

<div class="card">
  <div class="card-header">
    <span>Strategy Config</span>
    <span class="strategy-badge">
      SMA={config.sma_window} / pred5d={config.pred_5d_filter ? 'on' : 'off'}
    </span>
  </div>

  <div class="presets">
    <button class="preset-btn" on:click={() => loadPreset('optimal')}>
      SMA=40 Optimal
    </button>
    <button class="preset-btn" on:click={() => loadPreset('conservative')}>
      SMA=200 Conservative
    </button>
  </div>

  <div class="fields">
    <div class="field-row">
      <label for="entry_threshold">Entry</label>
      <input
        id="entry_threshold"
        type="number"
        step="0.0005"
        min="0.0001"
        max="0.01"
        bind:value={config.entry_threshold}
        on:input={markDirty}
      />
    </div>

    <div class="field-row">
      <label for="exit_threshold">Exit</label>
      <input
        id="exit_threshold"
        type="number"
        step="0.0005"
        min="-0.005"
        max="-0.0001"
        bind:value={config.exit_threshold}
        on:input={markDirty}
      />
    </div>

    <div class="field-row">
      <label for="sma_window">SMA Window</label>
      <input
        id="sma_window"
        type="number"
        step="1"
        min="1"
        max="300"
        bind:value={config.sma_window}
        on:input={markDirty}
      />
    </div>

    <div class="field-row toggle-row">
      <label for="pred_5d_filter">pred_5d filter</label>
      <input
        id="pred_5d_filter"
        type="checkbox"
        bind:checked={config.pred_5d_filter}
        on:change={markDirty}
      />
    </div>

    <div class="field-row toggle-row">
      <label for="enable_shorting">Shorting (PSQ)</label>
      <input
        id="enable_shorting"
        type="checkbox"
        bind:checked={config.enable_shorting}
        on:change={markDirty}
      />
    </div>

    <div class="field-row" class:disabled={!config.enable_shorting}>
      <label for="short_entry">Short Entry</label>
      <input
        id="short_entry"
        type="number"
        step="0.0005"
        min="-0.01"
        max="-0.0005"
        bind:value={config.short_entry_threshold}
        on:input={markDirty}
        disabled={!config.enable_shorting}
      />
    </div>

    <div class="field-row" class:disabled={!config.enable_shorting}>
      <label for="short_exit">Short Exit</label>
      <input
        id="short_exit"
        type="number"
        step="0.0005"
        min="0.0005"
        max="0.01"
        bind:value={config.short_exit_threshold}
        on:input={markDirty}
        disabled={!config.enable_shorting}
      />
    </div>

    <div class="field-row section-label">
      <span>Sentiment Overlay</span>
    </div>

    <div class="field-row toggle-row">
      <label for="enable_sentiment_overlay">Enable sentiment overlay</label>
      <input
        id="enable_sentiment_overlay"
        type="checkbox"
        bind:checked={config.enable_sentiment_overlay}
        on:change={markDirty}
      />
    </div>

    <div class="field-row" class:disabled={!config.enable_sentiment_overlay}>
      <label for="sentiment_reduce">Reduce threshold</label>
      <input
        id="sentiment_reduce"
        type="number"
        step="0.05"
        min="-1.0"
        max="-0.05"
        bind:value={config.sentiment_reduce_threshold}
        on:input={markDirty}
        disabled={!config.enable_sentiment_overlay}
      />
    </div>

    <div class="field-row" class:disabled={!config.enable_sentiment_overlay}>
      <label for="sentiment_exit">Exit threshold</label>
      <input
        id="sentiment_exit"
        type="number"
        step="0.05"
        min="-1.0"
        max="-0.05"
        bind:value={config.sentiment_exit_threshold}
        on:input={markDirty}
        disabled={!config.enable_sentiment_overlay}
      />
    </div>

    <div class="field-row" class:disabled={!config.enable_sentiment_overlay}>
      <label for="sentiment_min_articles">Min articles</label>
      <input
        id="sentiment_min_articles"
        type="number"
        step="1"
        min="0"
        max="1000"
        bind:value={config.sentiment_min_articles}
        on:input={markDirty}
        disabled={!config.enable_sentiment_overlay}
      />
    </div>
  </div>

  <button
    class="save-btn"
    on:click={handleSave}
    disabled={saving}
    class:dirty
  >
    {saving ? 'Saving...' : dirty ? 'Save Changes' : 'Saved'}
  </button>

  {#if error}
    <div class="msg error">{error}</div>
  {/if}
  {#if success}
    <div class="msg ok">{success}</div>
  {/if}
</div>

<style>
  .card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0.85rem;
    display: flex;
    flex-direction: column;
    gap: 0.65rem;
  }

  .card-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
    font-weight: 600;
  }

  .strategy-badge {
    padding: 0.12rem 0.5rem;
    border-radius: var(--radius-xs);
    font-size: 0.65rem;
    font-weight: 500;
    text-transform: none;
    background: var(--accent-subtle);
    color: var(--accent);
    font-family: var(--font-mono);
  }

  .presets {
    display: flex;
    gap: 0.4rem;
  }

  .preset-btn {
    flex: 1;
    padding: 0.35rem 0.5rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    background: transparent;
    color: var(--text-secondary);
    font-size: 0.7rem;
    font-family: inherit;
    cursor: pointer;
    transition: all 0.15s;
  }

  .preset-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
  }

  .field-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .field-row.disabled {
    opacity: 0.4;
  }

  .field-row label {
    color: var(--text-secondary);
    font-size: 0.78rem;
  }

  .field-row.section-label {
    margin-top: 0.5rem;
    border-top: 1px solid var(--border);
    padding-top: 0.5rem;
  }

  .field-row.section-label span {
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-size: 0.65rem;
    color: var(--accent);
  }

  .field-row input[type="number"] {
    width: 80px;
    background: var(--bg-inset);
    border: 1px solid var(--border);
    color: var(--text-primary);
    padding: 0.3rem 0.45rem;
    border-radius: var(--radius-xs);
    font-size: 0.78rem;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .field-row input[type="number"]:focus {
    outline: none;
    border-color: var(--accent);
  }

  .field-row input:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  .toggle-row label {
    cursor: pointer;
  }

  .toggle-row input[type="checkbox"] {
    width: 16px;
    height: 16px;
    cursor: pointer;
    accent-color: var(--accent);
  }

  .save-btn {
    padding: 0.45rem 1rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    background: transparent;
    color: var(--text-secondary);
    font-size: 0.78rem;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    transition: all 0.15s;
  }

  .save-btn.dirty {
    border-color: var(--green);
    color: var(--green);
  }

  .save-btn.dirty:hover:not(:disabled) {
    background: var(--green);
    color: #fff;
  }

  .save-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .msg {
    font-size: 0.75rem;
    padding: 0.35rem 0.5rem;
    border-radius: var(--radius-xs);
  }

  .msg.error {
    color: var(--red);
    background: var(--red-subtle);
  }

  .msg.ok {
    color: var(--green);
    background: var(--green-subtle);
  }
</style>
