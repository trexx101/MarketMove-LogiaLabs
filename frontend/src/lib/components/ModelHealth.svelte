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
  $: stalenessHours = (staleness / 3600).toFixed(staleness < 3600 ? 1 : 0);
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

<div class="card">
  <div class="card-header">Model Health</div>

  <div class="row">
    <span class="label">WS Connection</span>
    <span class="status {$wsConnected ? 'ok' : 'down'}">
      <span class="status-dot {$wsConnected ? 'ok' : 'down'}"></span>
      {$wsConnected ? 'Connected' : 'Disconnected'}
    </span>
  </div>

  <div class="row">
    <span class="label">Data Staleness</span>
    <span class="status {stale ? 'warn' : 'ok'}">{stalenessHours}h</span>
  </div>

  <div class="divider"></div>

  <div class="row">
    <span class="label">Dir. Acc. 1d</span>
    {#if dirAcc1d != null}
      <span class="status {dirAcc1d >= 50 ? 'ok' : 'warn'}">{fmtPct(dirAcc1d)}</span>
    {:else}
      <span class="status na">N/A</span>
    {/if}
  </div>

  <div class="row">
    <span class="label">Dir. Acc. 5d</span>
    {#if dirAcc5d != null}
      <span class="status {dirAcc5d >= 50 ? 'ok' : 'warn'}">{fmtPct(dirAcc5d)}</span>
    {:else}
      <span class="status na">N/A</span>
    {/if}
  </div>

  <div class="row">
    <span class="label">Dir. Acc. 21d</span>
    {#if dirAcc21d != null}
      <span class="status {dirAcc21d >= 50 ? 'ok' : 'warn'}">{fmtPct(dirAcc21d)}</span>
    {:else}
      <span class="status na">N/A</span>
    {/if}
  </div>

  <div class="row">
    <span class="label">MAE 1d</span>
    {#if mae1d != null}
      <span class="status">{fmtNum(mae1d)}</span>
    {:else}
      <span class="status na">N/A</span>
    {/if}
  </div>

  <div class="row">
    <span class="label">Resolved</span>
    <span class="status">{resolvedCount ?? 0}</span>
  </div>

  {#if error && !accuracyData}
    <div class="hint">Accuracy endpoint not available</div>
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
    gap: 0.45rem;
  }

  .card-header {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
    font-weight: 600;
    margin-bottom: 0.3rem;
  }

  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.82rem;
  }

  .label { color: var(--text-secondary); }

  .divider {
    height: 1px;
    background: var(--border);
    margin: 0.2rem 0;
  }

  .status {
    font-family: var(--font-mono);
    font-size: 0.78rem;
    display: flex;
    align-items: center;
    gap: 0.35rem;
    color: var(--text-primary);
  }

  .status-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
  }
  .status-dot.ok { background: var(--green); box-shadow: 0 0 5px var(--green); }
  .status-dot.down { background: var(--red); }

  .status.ok { color: var(--green); }
  .status.down { color: var(--red); }
  .status.warn { color: var(--yellow); }
  .status.na { color: var(--text-secondary); }

  .hint {
    font-size: 0.7rem;
    color: var(--text-secondary);
    margin-top: 0.25rem;
  }
</style>
