<script>
  import { onMount } from 'svelte';
  import { events } from '../lib/stores.js';
  import { connectWebSocket, disconnectWebSocket } from '../lib/websocket.js';
  import { fetchEvents } from '../lib/api.js';

  let loaded = false;
  let filterCategory = '';
  let filterMode = '';

  const categories = ['', 'trade', 'data', 'system', 'strategy', 'alert', 'advisor'];
  const modes = ['', 'paper', 'live'];

  async function load() {
    try {
      const hist = await fetchEvents(100, filterCategory || null, null, filterMode || null);
      events.set(hist);
      loaded = true;
    } catch (e) {
      console.error('Failed to load events:', e);
    }
  }

  onMount(() => {
    load();
    connectWebSocket();
    return () => disconnectWebSocket();
  });

  $: if (filterCategory || filterMode) load();

  function formatTs(ts) {
    if (!ts) return '';
    const d = new Date(ts * 1000);
    return d.toLocaleString();
  }

  function severityColor(sev) {
    if (sev === 'error') return 'color: #ff6b6b';
    if (sev === 'warn') return 'color: #ffd93d';
    return '';
  }

  function categoryIcon(cat) {
    const icons = {
      trade: '💰',
      data: '📊',
      system: '⚙️',
      strategy: '📈',
      alert: '⚠️',
      advisor: '🤖',
    };
    return icons[cat] || '📌';
  }
</script>

<div class="events-page">
  <div class="header">
    <h1>Events</h1>
    <div class="filters">
      <label>Category:
        <select bind:value={filterCategory}>
          {#each categories as cat}
            <option value={cat}>{cat || 'all'}</option>
          {/each}
        </select>
      </label>
      <label>Mode:
        <select bind:value={filterMode}>
          {#each modes as m}
            <option value={m}>{m || 'all'}</option>
          {/each}
        </select>
      </label>
      <button on:click={load}>Refresh</button>
    </div>
  </div>

  {#if !loaded}
    <p class="loading">Loading events...</p>
  {:else}
    <div class="event-list">
      {#each $events as ev}
        <div class="event-row" class:alert={ev.severity === 'error' || ev.severity === 'warn'}>
          <span class="ts">{formatTs(ev.ts)}</span>
          <span class="icon">{categoryIcon(ev.category)}</span>
          <span class="category">{ev.category}</span>
          <span class="severity" style={severityColor(ev.severity)}>{ev.severity}</span>
          <span class="mode badge {ev.mode}">{ev.mode}</span>
          <span class="message">{ev.message}</span>
          {#if ev.payload && Object.keys(ev.payload).length > 0}
            <details class="payload">
              <summary>payload</summary>
              <pre>{JSON.stringify(ev.payload, null, 2)}</pre>
            </details>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .events-page {
    padding: 1rem;
  }
  .header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1rem;
  }
  .filters {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }
  .filters label {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }
  .event-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .event-row {
    display: grid;
    grid-template-columns: 160px 24px 70px 50px 60px 1fr;
    align-items: start;
    gap: 0.5rem;
    padding: 0.5rem;
    background: #1e1e2e;
    border-radius: 4px;
  }
  .event-row.alert {
    background: #2e1e1e;
  }
  .ts {
    color: #6c7086;
    font-size: 0.85rem;
  }
  .category {
    font-weight: 500;
  }
  .mode.badge {
    padding: 2px 6px;
    border-radius: 3px;
    font-size: 0.75rem;
    text-transform: uppercase;
  }
  .mode.badge.paper {
    background: #1e3a5f;
    color: #89b4fa;
  }
  .mode.badge.live {
    background: #3a1e1e;
    color: #f38ba8;
  }
  .payload {
    grid-column: 1 / -1;
    margin-top: 0.25rem;
  }
  .payload pre {
    font-size: 0.8rem;
    background: #11111b;
    padding: 0.5rem;
    border-radius: 4px;
    overflow-x: auto;
  }
</style>