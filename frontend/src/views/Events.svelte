<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchEvents } from '../lib/api.js';

  const CATEGORIES = ['ALL', 'trade', 'data', 'system', 'strategy', 'alert', 'advisor'];
  const MODES = ['ALL', 'paper', 'live'];
  const SEVERITIES = ['ALL', 'info', 'warn', 'error'];

  let category = 'ALL';
  let mode = 'ALL';
  let severity = 'ALL';
  let equity = '';
  let events = [];
  let loading = true;
  let error = '';
  let refreshInterval;
  let expandedId = null;

  async function load() {
    error = '';
    try {
      const opts = { limit: 200 };
      if (category !== 'ALL') opts.category = category;
      if (mode !== 'ALL') opts.mode = mode;
      if (severity !== 'ALL') opts.severity = severity;
      if (equity.trim()) opts.equity = equity.trim().toUpperCase();
      const res = await fetchEvents(opts);
      events = res.events || [];
    } catch (e) {
      error = e.message;
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    load();
    refreshInterval = setInterval(load, 15000);
  });
  onDestroy(() => clearInterval(refreshInterval));

  function toggleExpand(id) {
    expandedId = expandedId === id ? null : id;
  }

  function fmtTime(ts) {
    if (!ts) return '—';
    try {
      return new Date(ts).toLocaleString();
    } catch {
      return String(ts);
    }
  }

  function fmtTimeShort(ts) {
    if (!ts) return '—';
    try {
      const d = new Date(ts);
      return d.toLocaleDateString().slice(0, 5) + ' ' + d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
    } catch {
      return String(ts);
    }
  }

  function payloadPretty(e) {
    try {
      if (e.payload == null) return '';
      if (typeof e.payload === 'object' && Object.keys(e.payload).length === 0) return '';
      return JSON.stringify(e.payload, null, 2);
    } catch {
      return String(e.payload);
    }
  }

  function sevClass(s) {
    return (s || '').toLowerCase();
  }

  function catClass(c) {
    return (c || '').toLowerCase();
  }

  function modeBadgeClass(m) {
    return (m || '').toLowerCase() === 'live' ? 'mode-live' : 'mode-paper';
  }
</script>

<div class="view">
  <div class="view-header">
    <h1>Events</h1>
    <div class="controls">
      <select bind:value={category} on:change={load} title="Category">
        {#each CATEGORIES as c}
          <option value={c}>{c === 'ALL' ? 'All categories' : c}</option>
        {/each}
      </select>
      <select bind:value={mode} on:change={load} title="Mode">
        {#each MODES as m}
          <option value={m}>{m === 'ALL' ? 'All modes' : m}</option>
        {/each}
      </select>
      <select bind:value={severity} on:change={load} title="Severity">
        {#each SEVERITIES as s}
          <option value={s}>{s === 'ALL' ? 'All severities' : s}</option>
        {/each}
      </select>
      <input bind:value={equity} placeholder="Equity" on:keydown={(e) => e.key === 'Enter' && load()} />
      <button on:click={load}>Refresh</button>
    </div>
  </div>

  {#if error}
    <div class="error">Error: {error}</div>
  {/if}

  <div class="table-card">
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th>Time</th>
            <th>Severity</th>
            <th>Category</th>
            <th>Mode</th>
            <th>Equity</th>
            <th>Source</th>
            <th>Message</th>
          </tr>
        </thead>
        <tbody>
          {#if loading}
            <tr><td colspan="7" class="center">Loading...</td></tr>
          {:else if events.length === 0}
            <tr><td colspan="7" class="center">No events match the current filters</td></tr>
          {:else}
            {#each events as e (e.id)}
              <tr class="event-row sev-{sevClass(e.severity)}" on:click={() => toggleExpand(e.id)}>
                <td class="mono" title={fmtTime(e.ts)}>{fmtTimeShort(e.ts)}</td>
                <td><span class="sev-dot sev-{sevClass(e.severity)}"></span>{e.severity}</td>
                <td><span class="cat-badge cat-{catClass(e.category)}">{e.category}</span></td>
                <td><span class="mode-badge {modeBadgeClass(e.mode)}">{e.mode}</span></td>
                <td class="mono">{e.equity || '—'}</td>
                <td class="mono">{e.source}</td>
                <td class="msg">{e.message}</td>
              </tr>
              {#if expandedId === e.id && payloadPretty(e)}
                <tr class="payload-row">
                  <td colspan="7">
                    <pre>{payloadPretty(e)}</pre>
                  </td>
                </tr>
              {/if}
            {/each}
          {/if}
        </tbody>
      </table>
    </div>
  </div>
</div>

<style>
  .view { padding: 1.25rem; }

  .view-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1.25rem;
    flex-wrap: wrap;
    gap: 0.75rem;
  }

  h1 {
    font-size: 1.25rem;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .controls { display: flex; gap: 0.4rem; flex-wrap: wrap; }

  select, input {
    background: var(--bg-inset);
    border: 1px solid var(--border);
    color: var(--text-primary);
    padding: 0.35rem 0.55rem;
    border-radius: var(--radius-xs);
    font-size: 0.82rem;
    font-family: inherit;
  }
  select:focus, input:focus { outline: none; border-color: var(--accent); }
  input { width: 90px; }

  button {
    background: var(--accent);
    border: none;
    color: #fff;
    padding: 0.35rem 0.9rem;
    border-radius: var(--radius-xs);
    cursor: pointer;
    font-size: 0.82rem;
    font-weight: 500;
    font-family: inherit;
    transition: background 0.15s;
  }
  button:hover { background: var(--accent-dark); }

  .error {
    background: var(--red-subtle);
    border: 1px solid var(--red);
    color: var(--text-primary);
    padding: 0.6rem 0.9rem;
    border-radius: var(--radius-xs);
    font-size: 0.82rem;
    margin-bottom: 1rem;
  }

  .table-card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .table-wrap { overflow-x: auto; }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.8rem;
  }

  th {
    text-align: left;
    padding: 0.55rem 0.75rem;
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
    font-weight: 600;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }

  td {
    padding: 0.45rem 0.75rem;
    border-bottom: 1px solid var(--border);
    color: var(--text-primary);
    white-space: nowrap;
  }

  .event-row { cursor: pointer; }
  .event-row:hover td { background: var(--bg-surface-hover); }
  tr:last-child td { border-bottom: none; }

  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  .center { text-align: center; color: var(--text-muted); padding: 2rem; }

  .msg {
    white-space: normal;
    max-width: 420px;
    line-height: 1.35;
  }

  .sev-dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    margin-right: 0.35rem;
    vertical-align: middle;
  }
  .sev-dot.sev-info { background: var(--text-secondary); }
  .sev-dot.sev-warn { background: var(--yellow); }
  .sev-dot.sev-error { background: var(--red); }

  .cat-badge, .mode-badge {
    display: inline-block;
    padding: 0.12rem 0.5rem;
    border-radius: var(--radius-xs);
    font-size: 0.68rem;
    font-weight: 600;
    letter-spacing: 0.03em;
  }

  .cat-trade { background: var(--green-subtle); color: var(--green); }
  .cat-strategy { background: var(--accent-subtle); color: var(--accent); }
  .cat-data { background: rgba(139, 141, 154, 0.14); color: var(--text-secondary); }
  .cat-system { background: rgba(139, 141, 154, 0.14); color: var(--text-secondary); }
  .cat-alert { background: var(--red-subtle); color: var(--red); }
  .cat-advisor { background: var(--yellow-subtle); color: var(--yellow); }

  .mode-live { background: var(--red-subtle); color: var(--red); }
  .mode-paper { background: rgba(139, 141, 154, 0.14); color: var(--text-secondary); }

  .payload-row td {
    background: var(--bg-inset);
    padding: 0.6rem 0.9rem;
  }
  .payload-row pre {
    margin: 0;
    font-family: var(--font-mono);
    font-size: 0.72rem;
    color: var(--text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
