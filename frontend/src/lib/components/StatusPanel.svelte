<script>
  import { onMount } from 'svelte';
  import { status } from '../stores.js';
  import { fetchMode, setMode } from '../api.js';

  $: s = $status;
  $: mode = s?.mode || 'PAPER';
  $: symbol = s?.symbol || 'QQQ';
  $: position = s?.position || 'flat';
  $: entryPrice = s?.entry_price;
  $: realizedPnl = s?.realized_pnl;
  $: unrealizedPnl = s?.unrealized_pnl;
  $: lastClose = s?.last_close;
  $: pred1d = s?.pred_1d;
  $: pred5d = s?.pred_5d;
  $: pred21d = s?.pred_21d;

  let showModeModal = false;
  let modeInfo = null;
  let targetMode = 'live';
  let totpCode = '';
  let modalError = '';
  let modalBusy = false;

  async function openModeModal() {
    modalError = '';
    totpCode = '';
    targetMode = mode === 'live' ? 'paper' : 'live';
    showModeModal = true;
    try {
      modeInfo = await fetchMode();
    } catch (e) {
      modalError = `failed to fetch mode: ${e.message}`;
    }
  }

  function closeModeModal() {
    showModeModal = false;
    modalError = '';
    totpCode = '';
  }

  async function submitMode() {
    if (!/^\d{6}$/.test(totpCode)) {
      modalError = 'TOTP must be 6 digits';
      return;
    }
    modalError = '';
    modalBusy = true;
    try {
      await setMode(targetMode, totpCode);
      closeModeModal();
    } catch (e) {
      modalError = e.message || 'unknown error';
    } finally {
      modalBusy = false;
    }
  }

  function fmt(v, dec = 2) {
    if (v == null || isNaN(v)) return '—';
    return Number(v).toFixed(dec);
  }

  function pnlColor(v) {
    if (v == null || isNaN(v)) return 'neutral';
    if (v > 0) return 'pos';
    if (v < 0) return 'neg';
    return 'neutral';
  }

  function positionLabel(p) {
    if (!p) return 'FLAT';
    const up = String(p).toUpperCase();
    if (up === 'LONG' || up === '1') return 'LONG';
    if (up === 'SHORT' || up === '-1') return 'SHORT';
    return 'FLAT';
  }

  function parityAgeLabel(secs) {
    if (secs == null) return '—';
    if (secs < 60) return `${secs}s`;
    if (secs < 3600) return `${Math.round(secs / 60)}m`;
    if (secs < 86400) return `${Math.round(secs / 3600)}h`;
    return `${Math.round(secs / 86400)}d`;
  }
</script>

<div class="card">
  <div class="card-header">Status</div>

  <div class="row mode-row">
    <span class="label">Mode</span>
    <div class="mode-cell">
      <span class="badge mode-{mode.toLowerCase()}">{mode}</span>
      <button class="icon-btn" on:click={openModeModal} title="Switch mode" aria-label="Switch mode">
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
          <path d="M3 4l4-2.5L11 4M3 10l4 2.5L11 10M3 4v6M11 4v6" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/>
        </svg>
      </button>
    </div>
  </div>

  <div class="row">
    <span class="label">Symbol</span>
    <span class="value">{symbol}</span>
  </div>

  <div class="row">
    <span class="label">Position</span>
    <span class="badge pos-{positionLabel(position).toLowerCase()}">{positionLabel(position)}</span>
  </div>

  <div class="row">
    <span class="label">Entry</span>
    <span class="value mono">{fmt(entryPrice)}</span>
  </div>

  <div class="row">
    <span class="label">Last</span>
    <span class="value mono">{fmt(lastClose)}</span>
  </div>

  <div class="divider"></div>

  <div class="row pred-row">
    <span class="label">Pred 1d</span>
    <span class="value mono {pnlColor(pred1d)}">{fmt(pred1d, 4)}</span>
  </div>

  <div class="row pred-row">
    <span class="label">Pred 5d</span>
    <span class="value mono {pnlColor(pred5d)}">{fmt(pred5d, 4)}</span>
  </div>

  <div class="row pred-row">
    <span class="label">Pred 21d</span>
    <span class="value mono {pnlColor(pred21d)}">{fmt(pred21d, 4)}</span>
  </div>

  <div class="divider"></div>

  <div class="row">
    <span class="label">Realized PnL</span>
    <span class="value mono {pnlColor(realizedPnl)}">{fmt(realizedPnl)}</span>
  </div>

  <div class="row">
    <span class="label">Unrealized</span>
    <span class="value mono {pnlColor(unrealizedPnl)}">{fmt(unrealizedPnl)}</span>
  </div>
</div>

{#if showModeModal}
  <div class="modal-backdrop"
       on:click={(e) => { if (e.target === e.currentTarget) closeModeModal(); }}
       on:keydown={(e) => { if (e.key === 'Escape') closeModeModal(); }}
       role="button" tabindex="0">
    <div class="modal" role="dialog" aria-modal="true">
      <h3>Switch to {targetMode.toUpperCase()} Trading</h3>

      {#if targetMode === 'live'}
        <p class="warn">
          This will execute real orders against your broker.
        </p>
      {/if}

      <div class="status-block">
        {#if modeInfo}
          <div class="status-row">
            <span class="status-label">Parity marker</span>
            <span class="status-val {modeInfo.parity_valid ? 'ok' : 'bad'}">
              {modeInfo.parity_valid ? 'Valid' : 'EXPIRED'}
            </span>
          </div>
          <div class="status-row">
            <span class="status-label">Age</span>
            <span class="status-val">{parityAgeLabel(modeInfo.parity_marker_age_secs)}</span>
          </div>
          {#if modeInfo.last_switch_ts}
            <div class="status-row">
              <span class="status-label">Last switch</span>
              <span class="status-val">{new Date(modeInfo.last_switch_ts * 1000).toLocaleString()}</span>
            </div>
          {/if}
        {:else}
          <div class="status-row">Loading...</div>
        {/if}
      </div>

      <div class="form-row">
        <label for="totp-input">TOTP Code (6 digits)</label>
        <input
          id="totp-input"
          type="text"
          inputmode="numeric"
          pattern="[0-9]{6}"
          maxlength="6"
          placeholder="123456"
          bind:value={totpCode}
          disabled={modalBusy}
        />
      </div>

      {#if modalError}
        <div class="error">{modalError}</div>
      {/if}

      <div class="actions">
        <button class="btn btn-ghost" on:click={closeModeModal} disabled={modalBusy}>
          Cancel
        </button>
        <button
          class="btn btn-confirm target-{targetMode}"
          on:click={submitMode}
          disabled={modalBusy || (targetMode === 'live' && modeInfo && !modeInfo.parity_valid)}
        >
          {modalBusy ? 'Switching...' : `Switch to ${targetMode.toUpperCase()}`}
        </button>
      </div>
    </div>
  </div>
{/if}

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

  .mode-row .mode-cell {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }

  .label {
    color: var(--text-secondary);
  }

  .value {
    color: var(--text-primary);
  }

  .value.mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  .value.pos { color: var(--green); }
  .value.neg { color: var(--red); }

  .divider {
    height: 1px;
    background: var(--border);
    margin: 0.2rem 0;
  }

  .badge {
    padding: 0.12rem 0.5rem;
    border-radius: var(--radius-xs);
    font-size: 0.7rem;
    font-weight: 600;
  }

  .mode-paper { background: var(--accent-subtle); color: var(--accent); }
  .mode-live { background: var(--red-subtle); color: var(--red); }

  .pos-long { background: var(--green-subtle); color: var(--green); }
  .pos-short { background: var(--red-subtle); color: var(--red); }
  .pos-flat { background: var(--bg-surface-hover); color: var(--text-secondary); }

  .icon-btn {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-secondary);
    border-radius: var(--radius-xs);
    padding: 0.15rem 0.3rem;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s;
  }
  .icon-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  /* Modal */
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.65);
    backdrop-filter: blur(4px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .modal {
    background: var(--bg-surface);
    border: 1px solid var(--border-light);
    border-radius: var(--radius);
    padding: 1.5rem;
    min-width: 360px;
    max-width: 480px;
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
    box-shadow: var(--shadow);
  }

  .modal h3 {
    margin: 0;
    color: var(--text-primary);
    font-size: 1.05rem;
    font-weight: 600;
  }

  .warn {
    color: var(--red);
    background: var(--red-subtle);
    padding: 0.5rem 0.75rem;
    border-radius: var(--radius-xs);
    margin: 0;
    font-size: 0.82rem;
  }

  .status-block {
    background: var(--bg-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
    padding: 0.65rem 0.8rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .status-row {
    display: flex;
    justify-content: space-between;
    font-size: 0.82rem;
  }

  .status-label { color: var(--text-secondary); }
  .status-val { color: var(--text-primary); }
  .status-val.ok { color: var(--green); font-weight: 600; }
  .status-val.bad { color: var(--red); font-weight: 600; }

  .form-row {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .form-row label {
    color: var(--text-secondary);
    font-size: 0.78rem;
  }

  .form-row input {
    background: var(--bg-inset);
    border: 1px solid var(--border);
    color: var(--text-primary);
    padding: 0.55rem 0.65rem;
    border-radius: var(--radius-xs);
    font-size: 1rem;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.3em;
  }

  .form-row input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .error {
    color: var(--red);
    background: var(--red-subtle);
    padding: 0.5rem 0.75rem;
    border-radius: var(--radius-xs);
    font-size: 0.82rem;
  }

  .actions {
    display: flex;
    gap: 0.5rem;
    justify-content: flex-end;
  }

  .btn {
    padding: 0.5rem 1rem;
    border-radius: var(--radius-xs);
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-primary);
    font-size: 0.82rem;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    transition: all 0.15s;
  }

  .btn-ghost:hover { border-color: var(--text-secondary); }

  .btn-confirm.target-live {
    background: var(--red-subtle);
    border-color: var(--red);
    color: var(--red);
  }
  .btn-confirm.target-live:hover:not(:disabled) {
    background: var(--red);
    color: #fff;
  }

  .btn-confirm.target-paper {
    background: var(--accent-subtle);
    border-color: var(--accent);
    color: var(--accent);
  }
  .btn-confirm.target-paper:hover:not(:disabled) {
    background: var(--accent);
    color: #fff;
  }

  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
