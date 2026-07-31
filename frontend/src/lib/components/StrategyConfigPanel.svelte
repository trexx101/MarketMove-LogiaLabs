<script>
  import { onMount } from 'svelte';
  import { fetchStrategyConfig, saveStrategyConfig } from '../api.js';

  let config = {
    entry_threshold: 0.001,
    exit_threshold: -0.0005,
    sma_window: 40,
    pred_5d_filter: false,
    enable_shorting: true,
    short_entry_threshold: -0.001,
    short_exit_threshold: 0.0005,
  };

  let dirty = false;
  let saving = false;
  let error = '';
  let success = '';

  onMount(async () => {
    try {
      config = await fetchStrategyConfig();
    } catch (e) {
      error = `Failed to load: ${e.message}`;
    }
  });

  function markDirty() {
    dirty = true;
    success = '';
  }

  async function handleSave() {
    saving = true;
    error = '';
    success = '';
    try {
      config = await saveStrategyConfig(config);
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
      };
    }
    dirty = true;
  }

  function fmt(v, dec = 4) {
    if (v == null) return '';
    return Number(v).toFixed(dec);
  }
</script>

<div class="strategy-panel">
  <div class="panel-header">
    <span>Strategy Config</span>
    <span class="strategy-badge badge-threshold">
      SMA={config.sma_window}, pred5d={config.pred_5d_filter ? 'on' : 'off'}
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
    <!-- Entry / Exit -->
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
  </div>

  <button
    class="save-btn"
    on:click={handleSave}
    disabled={saving}
    class:dirty
  >
    {saving ? 'Saving…' : dirty ? 'Save Changes' : 'Saved'}
  </button>

  {#if error}
    <div class="msg error">{error}</div>
  {/if}
  {#if success}
    <div class="msg ok">{success}</div>
  {/if}
</div>

<style>
  .strategy-panel {
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 8px;
    padding: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .panel-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #8b949e;
  }

  .strategy-badge {
    padding: 0.1rem 0.5rem;
    border-radius: 4px;
    font-size: 0.65rem;
    font-weight: 600;
    text-transform: none;
  }

  .badge-threshold {
    background: #1f6feb22;
    color: #58a6ff;
  }

  .presets {
    display: flex;
    gap: 0.4rem;
  }

  .preset-btn {
    flex: 1;
    padding: 0.3rem 0.5rem;
    border: 1px solid #30363d;
    border-radius: 4px;
    background: transparent;
    color: #8b949e;
    font-size: 0.7rem;
    cursor: pointer;
    transition: border-color 0.15s;
  }

  .preset-btn:hover {
    border-color: #58a6ff;
    color: #58a6ff;
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
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
    color: #8b949e;
    font-size: 0.78rem;
  }

  .field-row input[type="number"] {
    width: 80px;
    background: #0d1117;
    border: 1px solid #30363d;
    color: #c9d1d9;
    padding: 0.25rem 0.4rem;
    border-radius: 4px;
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .field-row input[type="number"]:focus {
    outline: none;
    border-color: #58a6ff;
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
    accent-color: #58a6ff;
  }

  .save-btn {
    padding: 0.4rem 1rem;
    border: 1px solid #30363d;
    border-radius: 4px;
    background: transparent;
    color: #8b949e;
    font-size: 0.8rem;
    cursor: pointer;
    transition: all 0.15s;
  }

  .save-btn.dirty {
    border-color: #3fb950;
    color: #3fb950;
  }

  .save-btn.dirty:hover:not(:disabled) {
    background: #3fb950;
    color: #fff;
  }

  .save-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .msg {
    font-size: 0.75rem;
    padding: 0.3rem 0.5rem;
    border-radius: 4px;
  }

  .msg.error {
    color: #f85149;
    background: #da363322;
  }

  .msg.ok {
    color: #3fb950;
    background: #23863622;
  }
</style>