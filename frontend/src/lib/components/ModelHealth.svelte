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
  $: dirAcc1d = accuracyData?.directional_1h;
  $: dirAcc5d = accuracyData?.directional_4h;
  $: dirAcc21d = accuracyData?.directional_24h;
  $: mae1d = accuracyData?.mae_1h;
  $: resolvedCount = accuracyData?.resolved_count;

  function fmtPct(v) {
    if (v == null || isNaN(v)) return 'N/A';
    return v.toFixed(1) + '%';
  }
  function fmtNum(v) {
    if (v == null || isNaN(v)) return 'N/A';
    return v.toFixed(4);
  }
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

  <div class="divider"></div>

  <div class="health-row">
    <span class="label">Dir. Acc. 1d</span>
    {#if dirAcc1d != null}
      <span class="status {dirAcc1d >= 50 ? 'ok' : 'warn'}">{fmtPct(dirAcc1d)}</span>
    {:else}
      <span class="status na">N/A</span>
    {/if}
  </div>

  <div class="health-row">
    <span class="label">Dir. Acc. 5d</span>
    {#if dirAcc5d != null}
      <span class="status {dirAcc5d >= 50 ? 'ok' : 'warn'}">{fmtPct(dirAcc5d)}</span>
    {:else}
      <span class="status na">N/A</span>
    {/if}
  </div>

  <div class="health-row">
    <span class="label">Dir. Acc. 21d</span>
    {#if dirAcc21d != null}
      <span class="status {dirAcc21d >= 50 ? 'ok' : 'warn'}">{fmtPct(dirAcc21d)}</span>
    {:else}
      <span class="status na">N/A</span>
    {/if}
  </div>

  <div class="health-row">
    <span class="label">MAE 1d</span>
    {#if mae1d != null}
      <span class="status">{fmtNum(mae1d)}</span>
    {:else}
      <span class="status na">N/A</span>
    {/if}
  </div>

  <div class="health-row">
    <span class="label">Resolved</span>
    <span class="status">{resolvedCount ?? 0}</span>
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

  .divider {
    height: 1px;
    background: #30363d;
    margin: 0.25rem 0;
  }

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
