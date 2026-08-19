<script>
  import { onMount } from 'svelte';
  import { fetchOptionsConfig, saveOptionsConfig } from '../lib/api.js';

  let entries = [];
  let drafts = {};       // key -> current edited value
  let loading = true;
  let error = '';
  let saving = false;
  let saveResult = null; // { applied, rejected: [...] }

  // UI grouping by key prefix. Order here controls display order.
  const GROUPS = [
    { id: 'sizing',    title: 'Sizing & Deployment', keys: ['risk_pct', 'max_premium_pct', 'deployed_cap_pct', 'contracts_cap', 'positions_per_underlying', 'max_positions'] },
    { id: 'chain',     title: 'Chain Selection',     keys: ['dte_min', 'dte_max', 'dte_exit_min', 'delta_target'] },
    { id: 'entries',   title: 'Entry Ladder',        keys: ['entry_stage1_secs', 'entry_stage2_secs'] },
    { id: 'exits',     title: 'Exits & Trailing',    keys: ['exit_stage1_secs', 'exit_stage2_secs', 'exit_stage3_secs', 'trail_pct', 'trail_rearm_band_atr', 'delta_recheck_band', 'delta_drift_min', 'delta_drift_max', 'cooldown_seconds'] },
    { id: 'macro',     title: 'Macro Gate',          keys: ['vix_level_gate', 'vix_slope_threshold', 'vix_slope_window', 'earnings_blackout_days', 'iv_spike_multiplier'] },
    { id: 'liquidity', title: 'Liquidity & Execution', keys: ['bid_min', 'spread_cap_pct', 'oi_min', 'slippage_multiplier', 'slippage_premium_cap_pct'] },
    { id: 'rail',      title: 'Risk Rails',          keys: ['max_consecutive_losses', 'blackout_hours'] },
  ];

  $: byKey = Object.fromEntries(entries.map((e) => [e.key, e]));
  $: grouped = GROUPS.map((g) => ({ ...g, items: g.keys.map((k) => byKey[k]).filter(Boolean) }))
    .filter((g) => g.items.length > 0);
  // Any keys the registry has that we didn't classify — show them in a catch-all group
  $: ungrouped = entries.filter((e) => !GROUPS.some((g) => g.keys.includes(e.key)));

  $: dirtyKeys = entries
    .filter((e) => drafts[e.key] !== undefined && drafts[e.key] !== e.value)
    .map((e) => e.key);
  $: dirtyCount = dirtyKeys.length;

  async function load() {
    error = '';
    saveResult = null;
    try {
      const res = await fetchOptionsConfig();
      entries = res.entries || [];
      drafts = Object.fromEntries(entries.map((e) => [e.key, e.value]));
    } catch (e) {
      error = e.message;
      entries = [];
    } finally {
      loading = false;
    }
  }

  onMount(load);

  async function save() {
    if (dirtyCount === 0) return;
    saving = true;
    saveResult = null;
    error = '';
    try {
      const payload = {};
      for (const key of dirtyKeys) payload[key] = drafts[key];
      const res = await saveOptionsConfig(payload);
      saveResult = res;
      await load(); // re-sync with persisted values
    } catch (e) {
      error = e.message;
    } finally {
      saving = false;
    }
  }

  function reset() {
    drafts = Object.fromEntries(entries.map((e) => [e.key, e.value]));
    saveResult = null;
  }

  function tierClass(t) {
    return (t || '').toLowerCase();
  }

  function fmt(v) {
    if (v == null || isNaN(v)) return '—';
    return Number(v).toFixed(4).replace(/\.?0+$/, '');
  }

  function handleSlider(key, e) {
    drafts = { ...drafts, [key]: parseFloat(e.target.value) };
  }

  function handleInt(key, e) {
    const v = parseInt(e.target.value, 10);
    drafts = { ...drafts, [key]: isNaN(v) ? drafts[key] : v };
  }
</script>

<div class="view">
  <div class="view-header">
    <h1>Options — Settings</h1>
    <div class="save-bar">
      {#if dirtyCount > 0}
        <button class="secondary" on:click={reset} disabled={saving}>Discard</button>
      {/if}
      <button on:click={save} disabled={saving || dirtyCount === 0}>
        {saving ? 'Saving...' : `Save ${dirtyCount > 0 ? `(${dirtyCount})` : ''}`}
      </button>
    </div>
  </div>

  <p class="hint">
    Strategy keys apply freely. <span class="rail-tag">RAIL</span> keys are bounded risk limits —
    every change is event-logged for the audit trail. Values apply from the next pipeline run.
  </p>

  {#if error}
    <div class="error">Error: {error}</div>
  {/if}

  {#if saveResult}
    <div class="result {saveResult.rejected && saveResult.rejected.length > 0 ? 'warn' : 'ok'}">
      Applied {saveResult.applied} change{saveResult.applied === 1 ? '' : 's'}.
      {#if saveResult.rejected && saveResult.rejected.length > 0}
        <div class="rejected">
          Rejected:
          <ul>{#each saveResult.rejected as r}<li>{r}</li>{/each}</ul>
        </div>
      {/if}
    </div>
  {/if}

  {#if loading}
    <div class="loading">Loading...</div>
  {:else}
    {#each grouped as group}
      <section class="group">
        <h2>{group.title}</h2>
        <div class="group-body">
          {#each group.items as entry (entry.key)}
            <div class="setting" class:dirty={drafts[entry.key] !== entry.value}>
              <div class="setting-head">
                <span class="setting-label">{entry.label}</span>
                <span class="tier-badge tier-{tierClass(entry.tier)}">{entry.tier}</span>
              </div>
              <p class="setting-desc">{entry.description}</p>
              <div class="setting-input">
                {#if entry.kind === 'int'}
                  <input
                    type="number"
                    min={entry.min}
                    max={entry.max}
                    step="1"
                    value={drafts[entry.key]}
                    on:input={(e) => handleInt(entry.key, e)}
                  />
                {:else}
                  <div class="slider-row">
                    <input
                      type="range"
                      min={entry.min}
                      max={entry.max}
                      step={(entry.max - entry.min) / 200}
                      value={drafts[entry.key]}
                      on:input={(e) => handleSlider(entry.key, e)}
                    />
                    <span class="slider-value">{fmt(drafts[entry.key])}</span>
                  </div>
                {/if}
              </div>
              <div class="setting-bounds">range: {fmt(entry.min)} – {fmt(entry.max)} · default: {fmt(entry.default)}</div>
            </div>
          {/each}
        </div>
      </section>
    {/each}

    {#if ungrouped.length > 0}
      <section class="group">
        <h2>Other</h2>
        <div class="group-body">
          {#each ungrouped as entry (entry.key)}
            <div class="setting" class:dirty={drafts[entry.key] !== entry.value}>
              <div class="setting-head">
                <span class="setting-label">{entry.label}</span>
                <span class="tier-badge tier-{tierClass(entry.tier)}">{entry.tier}</span>
              </div>
              <p class="setting-desc">{entry.description}</p>
              <div class="setting-input">
                {#if entry.kind === 'int'}
                  <input type="number" min={entry.min} max={entry.max} step="1"
                    value={drafts[entry.key]} on:input={(e) => handleInt(entry.key, e)} />
                {:else}
                  <div class="slider-row">
                    <input type="range" min={entry.min} max={entry.max}
                      step={(entry.max - entry.min) / 200}
                      value={drafts[entry.key]} on:input={(e) => handleSlider(entry.key, e)} />
                    <span class="slider-value">{fmt(drafts[entry.key])}</span>
                  </div>
                {/if}
              </div>
              <div class="setting-bounds">range: {fmt(entry.min)} – {fmt(entry.max)} · default: {fmt(entry.default)}</div>
            </div>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .view { padding: 1.25rem; max-width: 1100px; }

  .view-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.5rem;
    flex-wrap: wrap;
    gap: 0.75rem;
  }

  h1 {
    font-size: 1.25rem;
    font-weight: 600;
    color: var(--text-primary);
    letter-spacing: -0.01em;
  }

  .hint {
    font-size: 0.78rem;
    color: var(--text-secondary);
    margin-bottom: 1.25rem;
  }

  .rail-tag {
    display: inline-block;
    background: var(--yellow-subtle);
    color: var(--yellow);
    padding: 0 0.35rem;
    border-radius: var(--radius-xs);
    font-size: 0.65rem;
    font-weight: 700;
    letter-spacing: 0.04em;
    vertical-align: middle;
  }

  .save-bar { display: flex; gap: 0.4rem; }

  button {
    background: var(--accent);
    color: #fff;
    border: none;
    padding: 0.35rem 0.9rem;
    border-radius: var(--radius-xs);
    cursor: pointer;
    font-size: 0.82rem;
    font-weight: 500;
    font-family: inherit;
    transition: background 0.15s;
  }
  button:hover:not(:disabled) { background: var(--accent-dark); }
  button:disabled { opacity: 0.45; cursor: default; }
  button.secondary {
    background: var(--bg-inset);
    border: 1px solid var(--border);
    color: var(--text-primary);
  }
  button.secondary:hover:not(:disabled) { background: var(--bg-surface-hover); }

  .error {
    background: var(--red-subtle);
    border: 1px solid var(--red);
    color: var(--text-primary);
    padding: 0.6rem 0.9rem;
    border-radius: var(--radius-xs);
    font-size: 0.82rem;
    margin-bottom: 1rem;
  }

  .result {
    padding: 0.6rem 0.9rem;
    border-radius: var(--radius-xs);
    font-size: 0.82rem;
    margin-bottom: 1rem;
  }
  .result.ok { background: var(--green-subtle); border: 1px solid var(--green); }
  .result.warn { background: var(--yellow-subtle); border: 1px solid var(--yellow); }
  .rejected ul { margin: 0.3rem 0 0 1.2rem; }

  .loading { color: var(--text-muted); padding: 2rem; text-align: center; }

  .group { margin-bottom: 1.5rem; }

  h2 {
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-secondary);
    font-weight: 700;
    margin-bottom: 0.6rem;
  }

  .group-body {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 0.75rem;
  }

  .setting {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0.8rem 0.9rem;
    transition: border-color 0.15s;
  }
  .setting.dirty { border-color: var(--accent); }

  .setting-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.25rem;
  }

  .setting-label {
    font-size: 0.82rem;
    font-weight: 500;
    color: var(--text-primary);
  }

  .tier-badge {
    font-size: 0.6rem;
    font-weight: 700;
    letter-spacing: 0.05em;
    padding: 0.1rem 0.4rem;
    border-radius: var(--radius-xs);
    text-transform: uppercase;
  }
  .tier-strategy { background: var(--accent-subtle); color: var(--accent); }
  .tier-rail { background: var(--yellow-subtle); color: var(--yellow); }

  .setting-desc {
    font-size: 0.72rem;
    color: var(--text-muted);
    line-height: 1.35;
    margin-bottom: 0.6rem;
  }

  .setting-input input[type='number'] {
    width: 100%;
    background: var(--bg-inset);
    border: 1px solid var(--border);
    color: var(--text-primary);
    padding: 0.35rem 0.55rem;
    border-radius: var(--radius-xs);
    font-size: 0.82rem;
    font-family: var(--font-mono);
  }
  .setting-input input[type='number']:focus { outline: none; border-color: var(--accent); }

  .slider-row { display: flex; align-items: center; gap: 0.6rem; }

  .slider-row input[type='range'] {
    flex: 1;
    accent-color: var(--accent);
  }

  .slider-value {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 0.78rem;
    color: var(--text-primary);
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    padding: 0.15rem 0.5rem;
    min-width: 72px;
    text-align: right;
  }

  .setting-bounds {
    font-size: 0.66rem;
    color: var(--text-muted);
    font-family: var(--font-mono);
    margin-top: 0.45rem;
  }
</style>
