<script>
  import { onMount, onDestroy } from 'svelte';
  import { fetchHyperoptCandidates, fetchHyperoptStatus, promoteCandidate } from '../lib/api.js';

  const EQUITIES = ['QQQ', 'SMH', 'XLF'];
  const STAGES = ['CANDIDATE', 'PAPER', 'MICRO', 'LIVE'];

  let equity = 'QQQ';
  let candidates = [];
  let pipelineStatus = null;
  let loading = false;
  let error = '';
  let refreshInterval;

  // Promotion state
  let confirmingId = null;
  let promotingId = null;
  let promoteResult = null; // { id, success, message }

  $: stageCounts = pipelineStatus?.by_status || {};

  async function load() {
    loading = true;
    error = '';
    try {
      const [candRes, statusRes] = await Promise.all([
        fetchHyperoptCandidates(equity),
        fetchHyperoptStatus(equity),
      ]);
      candidates = candRes.candidates || [];
      pipelineStatus = statusRes;
    } catch (e) {
      error = e.message;
      candidates = [];
      pipelineStatus = null;
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    load();
    refreshInterval = setInterval(load, 30000);
  });
  onDestroy(() => clearInterval(refreshInterval));

  function selectEquity(eq) {
    if (eq === equity) return;
    equity = eq;
    confirmingId = null;
    promoteResult = null;
    load();
  }

  function askPromote(id) {
    confirmingId = confirmingId === id ? null : id;
    promoteResult = null;
  }

  async function doPromote(id) {
    promotingId = id;
    promoteResult = null;
    try {
      const res = await promoteCandidate(equity, id);
      promoteResult = { id, ...res };
      confirmingId = null;
      await load(); // refresh statuses
    } catch (e) {
      promoteResult = { id, success: false, message: e.message };
    } finally {
      promotingId = null;
    }
  }

  function fmtIc(v) {
    return (v == null || isNaN(v)) ? '—' : v.toFixed(4);
  }

  function fmtDate(iso) {
    if (!iso) return '—';
    const d = new Date(iso);
    return isNaN(d) ? iso : d.toISOString().slice(0, 10);
  }

  function fmtParams(params) {
    if (!params || typeof params !== 'object') return '';
    return Object.entries(params)
      .map(([k, v]) => `${k}=${typeof v === 'number' ? +v.toFixed(3) : v}`)
      .join('  ');
  }

  function nextStage(status) {
    const s = status?.toUpperCase();
    if (s === 'NEW' || s === 'STABLE' || s === 'CANDIDATE') return 'PAPER';
    if (s === 'PAPER') return 'MICRO';
    if (s === 'MICRO') return 'LIVE';
    return null; // LIVE / UNSTABLE / RETIRED — no promotion
  }

  function statusClass(status) {
    const s = status?.toUpperCase() || '';
    if (s === 'LIVE') return 'status-live';
    if (s === 'MICRO') return 'status-micro';
    if (s === 'PAPER') return 'status-paper';
    if (s === 'UNSTABLE' || s === 'RETIRED') return 'status-dead';
    return 'status-new'; // NEW / STABLE / CANDIDATE
  }
</script>

<div class="autopilot">
  <header class="page-header">
    <div>
      <h1>Strategy Auto-Pilot</h1>
      <p class="subtitle">Hyperopt candidate pipeline · nightly optimizer · gated promotion</p>
    </div>
    <button class="btn btn-ghost" on:click={load} disabled={loading}>
      {loading ? 'Refreshing…' : 'Refresh'}
    </button>
  </header>

  <!-- Equity selector -->
  <div class="equity-tabs">
    {#each EQUITIES as eq}
      <button
        class="equity-tab"
        class:active={equity === eq}
        on:click={() => selectEquity(eq)}
      >
        {eq}
      </button>
    {/each}
  </div>

  {#if error}
    <div class="error-banner">{error}</div>
  {/if}

  <!-- Pipeline status -->
  <div class="pipeline">
    <div class="card">
      <div class="card-header">Promotion pipeline — {equity}</div>
      <div class="stage-flow">
        {#each STAGES as stage, i}
          <div class="stage" class:terminal={stage === 'LIVE'}>
            <div class="stage-name">{stage}</div>
            <div class="stage-count">
              {stageCounts[stage] ?? (stage === 'CANDIDATE' ? ((stageCounts['NEW'] || 0) + (stageCounts['STABLE'] || 0)) : 0)}
            </div>
          </div>
          {#if i < STAGES.length - 1}
            <div class="stage-arrow">→</div>
          {/if}
        {/each}
      </div>
      <div class="pipeline-meta">
        <span>Total candidates: <strong>{pipelineStatus?.total_candidates ?? 0}</strong></span>
        {#if pipelineStatus?.pipeline_state}
          <span class="pipeline-state">{pipelineStatus.pipeline_state}</span>
        {/if}
      </div>
    </div>
  </div>

  <!-- Candidates table -->
  <div class="card">
    <div class="card-header">Candidates ({candidates.length})</div>

    {#if loading && candidates.length === 0}
      <div class="empty">Loading candidates…</div>
    {:else if candidates.length === 0}
      <div class="empty">
        No candidates for {equity} yet. The nightly hyperopt runner stores candidates
        after each post-market optimization pass.
      </div>
    {:else}
      <table class="candidates-table">
        <thead>
          <tr>
            <th>Version</th>
            <th>Strategy</th>
            <th>Status</th>
            <th class="num">Mean IC</th>
            <th class="num">±IC</th>
            <th class="num">Trades</th>
            <th>Params</th>
            <th>Created</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each candidates as c (c.id)}
            <tr>
              <td class="mono">{c.id}</td>
              <td>{c.strategy}</td>
              <td><span class="badge {statusClass(c.status)}">{c.status}</span></td>
              <td class="num mono" class:good-ic={c.mean_ic >= 0.03}>{fmtIc(c.mean_ic)}</td>
              <td class="num mono">{fmtIc(c.std_ic)}</td>
              <td class="num mono">{c.n_trades}</td>
              <td class="mono params-cell" title={fmtParams(c.params)}>{fmtParams(c.params)}</td>
              <td class="muted">{fmtDate(c.created_at)}</td>
              <td class="action-cell">
                {#if nextStage(c.status)}
                  {#if confirmingId === c.id}
                    <div class="confirm-group">
                      <button
                        class="btn btn-promote"
                        disabled={promotingId === c.id}
                        on:click={() => doPromote(c.id)}
                      >
                        {promotingId === c.id ? 'Promoting…' : `→ ${nextStage(c.status)}`}
                      </button>
                      <button class="btn btn-ghost" on:click={() => (confirmingId = null)}>Cancel</button>
                    </div>
                  {:else}
                    <button class="btn btn-ghost" on:click={() => askPromote(c.id)}>Promote</button>
                  {/if}
                {:else}
                  <span class="muted">—</span>
                {/if}
              </td>
            </tr>
            {#if promoteResult && promoteResult.id === c.id}
              <tr class="result-row">
                <td colspan="9">
                  <div class="promote-result" class:ok={promoteResult.success} class:fail={!promoteResult.success}>
                    {promoteResult.success ? '✓' : '✗'} {promoteResult.message}
                  </div>
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    {/if}
  </div>

  <!-- Deferred panels (no backend endpoints yet) -->
  <div class="deferred-grid">
    <div class="card deferred">
      <div class="card-header">Risk sliders</div>
      <p class="deferred-note">Risk-per-trade % and max premium per position land with the config-write API.</p>
    </div>
    <div class="card deferred">
      <div class="card-header">Open positions</div>
      <p class="deferred-note">Entry basis, delta drift, DTE countdown, exit-stage indicator — pending positions API.</p>
    </div>
    <div class="card deferred">
      <div class="card-header">Events feed</div>
      <p class="deferred-note">SKIPPED_ENTRY reasons, gate denials, promotion events — pending events API.</p>
    </div>
  </div>
</div>

<style>
  .autopilot {
    padding: 1.25rem 1.5rem;
    max-width: 1400px;
  }

  .page-header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    margin-bottom: 1rem;
  }

  h1 {
    font-size: 1.15rem;
    font-weight: 600;
    margin: 0;
  }

  .subtitle {
    color: var(--text-muted);
    font-size: 0.8rem;
    margin: 0.25rem 0 0;
  }

  .equity-tabs {
    display: flex;
    gap: 0.4rem;
    margin-bottom: 1rem;
  }

  .equity-tab {
    padding: 0.45rem 1.1rem;
    border-radius: var(--radius-xs);
    border: 1px solid var(--border);
    background: var(--bg-surface);
    color: var(--text-secondary);
    font-weight: 600;
    font-size: 0.82rem;
    font-family: var(--font-mono);
    cursor: pointer;
    transition: all 0.15s;
  }
  .equity-tab:hover { border-color: var(--border-light); color: var(--text-primary); }
  .equity-tab.active {
    background: var(--accent-subtle);
    border-color: var(--accent);
    color: var(--accent);
  }

  .card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    margin-bottom: 1rem;
    overflow: hidden;
  }

  .card-header {
    padding: 0.65rem 0.9rem;
    border-bottom: 1px solid var(--border);
    font-size: 0.78rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
  }

  .error-banner {
    background: var(--red-subtle);
    border: 1px solid var(--red);
    color: var(--red);
    padding: 0.6rem 0.9rem;
    border-radius: var(--radius-xs);
    font-size: 0.82rem;
    margin-bottom: 1rem;
  }

  /* Pipeline stage flow */
  .stage-flow {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 1rem 0.9rem 0.75rem;
    flex-wrap: wrap;
  }

  .stage {
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    padding: 0.5rem 0.9rem;
    min-width: 92px;
    text-align: center;
  }
  .stage.terminal { border-color: var(--green); background: var(--green-subtle); }

  .stage-name {
    font-size: 0.68rem;
    font-weight: 600;
    letter-spacing: 0.08em;
    color: var(--text-secondary);
  }
  .stage.terminal .stage-name { color: var(--green); }

  .stage-count {
    font-size: 1.25rem;
    font-weight: 700;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  .stage-arrow { color: var(--text-muted); font-size: 0.9rem; }

  .pipeline-meta {
    display: flex;
    gap: 1.25rem;
    align-items: center;
    padding: 0 0.9rem 0.75rem;
    font-size: 0.78rem;
    color: var(--text-secondary);
  }
  .pipeline-meta strong { color: var(--text-primary); font-family: var(--font-mono); }
  .pipeline-state {
    background: var(--accent-subtle);
    color: var(--accent);
    padding: 0.15rem 0.5rem;
    border-radius: var(--radius-xs);
    font-size: 0.72rem;
    font-weight: 600;
  }

  /* Table */
  .candidates-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.8rem;
  }

  .candidates-table th {
    text-align: left;
    padding: 0.5rem 0.75rem;
    font-size: 0.7rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
  }

  .candidates-table td {
    padding: 0.55rem 0.75rem;
    border-bottom: 1px solid var(--border);
    vertical-align: middle;
  }

  .candidates-table tbody tr:last-child td { border-bottom: none; }
  .candidates-table tbody tr:hover td { background: var(--bg-surface-hover); }

  .num { text-align: right; }
  th.num { text-align: right; }
  .mono { font-family: var(--font-mono); font-variant-numeric: tabular-nums; }
  .muted { color: var(--text-muted); }
  .good-ic { color: var(--green); }

  .params-cell {
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-secondary);
    font-size: 0.72rem;
  }

  .badge {
    display: inline-block;
    padding: 0.15rem 0.5rem;
    border-radius: var(--radius-xs);
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.04em;
  }
  .status-new   { background: var(--accent-subtle); color: var(--accent); }
  .status-paper { background: var(--yellow-subtle); color: var(--yellow); }
  .status-micro { background: rgba(113, 50, 245, 0.22); color: #a47fff; }
  .status-live  { background: var(--green-subtle); color: var(--green); }
  .status-dead  { background: var(--red-subtle); color: var(--red); }

  .action-cell { white-space: nowrap; }

  .confirm-group { display: flex; gap: 0.35rem; }

  .btn {
    padding: 0.35rem 0.75rem;
    border-radius: var(--radius-xs);
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-primary);
    font-size: 0.78rem;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    transition: all 0.15s;
  }
  .btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .btn-ghost:hover { border-color: var(--text-secondary); }

  .btn-promote {
    background: var(--accent-subtle);
    border-color: var(--accent);
    color: var(--accent);
  }
  .btn-promote:hover:not(:disabled) { background: var(--accent); color: #fff; }

  .result-row td { padding: 0 0.75rem 0.55rem; border-bottom: 1px solid var(--border); }
  .promote-result {
    font-size: 0.78rem;
    padding: 0.45rem 0.7rem;
    border-radius: var(--radius-xs);
  }
  .promote-result.ok { background: var(--green-subtle); color: var(--green); }
  .promote-result.fail { background: var(--red-subtle); color: var(--red); }

  .empty {
    padding: 2rem 0.9rem;
    color: var(--text-muted);
    font-size: 0.82rem;
    text-align: center;
  }

  /* Deferred panels */
  .deferred-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: 1rem;
  }
  .deferred { opacity: 0.55; }
  .deferred-note {
    padding: 0.75rem 0.9rem;
    margin: 0;
    font-size: 0.78rem;
    color: var(--text-muted);
  }
</style>
