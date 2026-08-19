<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchHyperoptRuns, fetchTapeStatus } from '../lib/api.js';

  let runs = [];
  let tape = null;
  let loading = true;
  let error = '';
  let refreshInterval;

  async function load() {
    error = '';
    try {
      const [runsRes, tapeRes] = await Promise.all([fetchHyperoptRuns(20), fetchTapeStatus()]);
      runs = runsRes.runs || [];
      tape = tapeRes;
    } catch (e) {
      error = e.message;
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    load();
    refreshInterval = setInterval(load, 30000);
  });
  onDestroy(() => clearInterval(refreshInterval));

  function fmtTime(ts) {
    if (!ts) return '—';
    try {
      return new Date(ts).toLocaleString();
    } catch {
      return String(ts);
    }
  }

  function fmtAge(secs) {
    if (secs == null) return 'never';
    if (secs < 60) return `${secs}s ago`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
    return `${Math.floor(secs / 3600)}h ago`;
  }

  function fmtDuration(r) {
    if (!r.started_at || !r.finished_at) return '—';
    const s = (r.finished_at - r.started_at) / 1000;
    if (s < 60) return `${Math.round(s)}s`;
    return `${Math.floor(s / 60)}m ${Math.round(s % 60)}s`;
  }

  function statusClass(s) {
    return (s || '').toLowerCase();
  }
</script>

<div class="view">
  <div class="view-header">
    <h1>Options — Monitor</h1>
    <button on:click={load}>Refresh</button>
  </div>

  {#if error}
    <div class="error">Error: {error}</div>
  {/if}

  {#if loading}
    <div class="loading">Loading...</div>
  {:else}
    <section class="panel">
      <div class="panel-head">
        <h2>Tape Recorder</h2>
        {#if tape}
          <div class="tape-stats">
            <span class="tape-stat ok">{tape.healthy} healthy</span>
            {#if tape.stale > 0}
              <span class="tape-stat bad">{tape.stale} stale</span>
            {/if}
            {#if tape.never_beat > 0}
              <span class="tape-stat warn">{tape.never_beat} never beat</span>
            {/if}
          </div>
        {/if}
      </div>

      {#if !tape || tape.count === 0}
        <p class="empty">No tape recorders registered yet. The recorder process touches a heartbeat on every healthy tick.</p>
      {:else}
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th>Underlying</th>
                <th>Chain</th>
                <th>Last heartbeat</th>
                <th>State</th>
                <th>Quota accounting</th>
              </tr>
            </thead>
            <tbody>
              {#each tape.tapes as t}
                <tr>
                  <td class="mono">{t.underlying}</td>
                  <td class="mono contract" title={t.chain_code}>{t.chain_code}</td>
                  <td class="mono">{fmtAge(t.heartbeat_age_secs)}</td>
                  <td>
                    <span class="badge hb-{t.heartbeat_stale ? 'stale' : 'ok'}">
                      {t.heartbeat_stale ? 'STALE' : 'HEALTHY'}
                    </span>
                  </td>
                  <td class="mono quota">{t.quota_accounting_json}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>

    <section class="panel">
      <h2>Hyperopt Runs</h2>
      {#if runs.length === 0}
        <p class="empty">No hyperopt runs yet. The nightly loop starts at 21:30 UTC.</p>
      {:else}
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th>#</th>
                <th>Started</th>
                <th>Duration</th>
                <th>Status</th>
                <th>Equities</th>
                <th>Candidates stored</th>
                <th>Promoted</th>
                <th>Error</th>
              </tr>
            </thead>
            <tbody>
              {#each runs as r}
                <tr>
                  <td class="mono">{r.id}</td>
                  <td class="mono">{fmtTime(r.started_at)}</td>
                  <td class="mono">{fmtDuration(r)}</td>
                  <td><span class="badge status-{statusClass(r.status)}">{r.status}</span></td>
                  <td class="mono">{r.equities_processed}</td>
                  <td class="mono">{r.candidates_stored}</td>
                  <td class="mono">{r.candidates_promoted}</td>
                  <td class="mono err-cell" title={r.error || ''}>{r.error || '—'}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>
  {/if}
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

  .loading { color: var(--text-muted); padding: 2rem; text-align: center; }

  .panel {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 1rem 1.1rem;
    margin-bottom: 1.25rem;
  }

  .panel-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-bottom: 0.6rem;
  }

  h2 {
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-secondary);
    font-weight: 700;
    margin-bottom: 0.6rem;
  }

  .panel-head h2 { margin-bottom: 0; }

  .tape-stats { display: flex; gap: 0.6rem; }

  .tape-stat {
    font-size: 0.72rem;
    font-weight: 600;
    padding: 0.15rem 0.55rem;
    border-radius: var(--radius-xs);
  }
  .tape-stat.ok { background: var(--green-subtle); color: var(--green); }
  .tape-stat.bad { background: var(--red-subtle); color: var(--red); }
  .tape-stat.warn { background: var(--yellow-subtle); color: var(--yellow); }

  .empty {
    color: var(--text-muted);
    font-size: 0.82rem;
    padding: 0.5rem 0;
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
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--border);
    color: var(--text-primary);
    white-space: nowrap;
  }

  tr:last-child td { border-bottom: none; }
  tr:hover td { background: var(--bg-surface-hover); }

  .mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  .contract {
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .quota {
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 0.72rem;
  }

  .err-cell {
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-secondary);
  }

  .badge {
    display: inline-block;
    padding: 0.12rem 0.5rem;
    border-radius: var(--radius-xs);
    font-size: 0.68rem;
    font-weight: 600;
    letter-spacing: 0.03em;
  }

  .hb-ok { background: var(--green-subtle); color: var(--green); }
  .hb-stale { background: var(--red-subtle); color: var(--red); }

  .status-completed { background: var(--green-subtle); color: var(--green); }
  .status-running { background: var(--accent-subtle); color: var(--accent); }
  .status-failed { background: var(--red-subtle); color: var(--red); }
</style>
