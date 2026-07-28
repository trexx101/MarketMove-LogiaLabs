<script>
  import { onMount } from 'svelte';
  import { fetchAccuracy } from '../api.js';
  import { wsConnected, status } from '../stores.js';

  let accuracyData = null;
  let error = null;

  onMount(async () => {
    try {
      accuracyData = await fetchAccuracy();
    } catch (e) {
      error = e.message;
    }
  });

  $: staleness = $status?.staleness_secs ?? 0;
  $: stale = staleness > 120;
  $: icDrift = accuracyData?.ic_drift;
</script>

<div class="model-health">
  <div class="panel-header">Model Health</div>

  <div class="health-row">
    <span class="label">WS Connection</span>
    <span class="status {$wsConnected ? 'ok' : 'down'}">
      {$wsConnected ? 'Connected' : 'Disconnected'}
    </span>
  </div>

  <div class="health-row">
    <span class="label">Data Staleness</span>
    <span class="status {stale ? 'warn' : 'ok'}">{staleness}s</span>
  </div>

  <div class="health-row">
    <span class="label">IC Drift</span>
    {#if icDrift != null}
      <span class="status {Math.abs(icDrift) > 0.1 ? 'warn' : 'ok'}">
        {icDrift.toFixed(4)}
      </span>
    {:else}
      <span class="status na">N/A</span>
    {/if}
  </div>

  {#if error && !accuracyData}
    <div class="hint">Accuracy endpoint not available</div>
  {/if}
</div>

<style>
  .model-health {
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

  .health-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.85rem;
  }

  .label { color: #8b949e; }

  .status {
    font-family: monospace;
    font-size: 0.8rem;
  }
  .status.ok { color: #3fb950; }
  .status.down { color: #f85149; }
  .status.warn { color: #d29922; }
  .status.na { color: #8b949e; }

  .hint {
    font-size: 0.7rem;
    color: #8b949e;
    margin-top: 0.25rem;
  }
</style>
