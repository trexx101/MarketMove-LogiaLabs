<script>
  import { createEventDispatcher } from 'svelte';
  import { fetchStrategies, fetchBacktest } from '../api.js';

  const dispatch = createEventDispatcher();

  export let strategies = [];

  let strategyA = null;
  let strategyB = null;
  let loading = false;
  let error = '';

  $: strategiesLoaded = strategies.length > 0;

  async function compare() {
    if (!strategyA || !strategyB) return;

    loading = true;
    error = '';

    try {
      const now = Math.floor(Date.now() / 1000);
      const oneYearAgo = now - 365 * 24 * 60 * 60;

      const [resultA, resultB] = await Promise.all([
        fetchBacktest({
          strategy_id: strategyA,
          kind: 'threshold',
          params: {},
          start_ts: oneYearAgo,
          end_ts: now,
        }),
        fetchBacktest({
          strategy_id: strategyB,
          kind: 'threshold',
          params: {},
          start_ts: oneYearAgo,
          end_ts: now,
        }),
      ]);

      dispatch('compare', {
        a: { ...resultA, name: strategies.find(s => s.id === strategyA)?.name || 'Strategy A' },
        b: { ...resultB, name: strategies.find(s => s.id === strategyB)?.name || 'Strategy B' },
      });
    } catch (e) {
      error = e.message;
    } finally {
      loading = false;
    }
  }
</script>

<div class="ab-comparison">
  <div class="ab-header">A/B Comparison</div>

  {#if !strategiesLoaded}
    <div class="ab-hint">Save strategies first to compare them.</div>
  {:else}
    <div class="ab-selects">
      <div class="ab-select-group">
        <label for="sel-a">Strategy A</label>
        <select id="sel-a" bind:value={strategyA}>
          <option value={null}>-- Select --</option>
          {#each strategies as s}
            <option value={s.id}>{s.name}</option>
          {/each}
        </select>
      </div>
      <div class="ab-select-group">
        <label for="sel-b">Strategy B</label>
        <select id="sel-b" bind:value={strategyB}>
          <option value={null}>-- Select --</option>
          {#each strategies as s}
            <option value={s.id}>{s.name}</option>
          {/each}
        </select>
      </div>
    </div>

    <button
      class="compare-btn"
      disabled={!strategyA || !strategyB || loading}
      on:click={compare}
    >
      {#if loading}
        Running...
      {:else}
        Compare
      {/if}
    </button>

    {#if error}
      <div class="ab-error">{error}</div>
    {/if}
  {/if}
</div>

<style>
  .ab-comparison {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0.85rem;
    display: flex;
    flex-direction: column;
    gap: 0.65rem;
  }

  .ab-header {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
    font-weight: 600;
  }

  .ab-hint {
    font-size: 0.8rem;
    color: var(--text-muted);
    padding: 0.5rem 0;
  }

  .ab-selects {
    display: flex;
    gap: 0.75rem;
  }

  .ab-select-group {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .ab-select-group label {
    font-size: 0.75rem;
    color: var(--text-secondary);
  }

  .ab-select-group select {
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    color: var(--text-primary);
    padding: 0.4rem 0.5rem;
    font-size: 0.82rem;
    font-family: inherit;
    outline: none;
    cursor: pointer;
  }

  .ab-select-group select:focus {
    border-color: var(--accent);
  }

  .compare-btn {
    padding: 0.5rem 1rem;
    background: var(--accent);
    border: none;
    border-radius: var(--radius-xs);
    color: #fff;
    font-size: 0.82rem;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    transition: background 0.15s;
    align-self: flex-start;
  }

  .compare-btn:hover:not(:disabled) {
    background: var(--accent-dark);
  }

  .compare-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .ab-error {
    font-size: 0.78rem;
    color: var(--red);
    background: var(--red-subtle);
    border-radius: var(--radius-xs);
    padding: 0.4rem 0.6rem;
  }
</style>