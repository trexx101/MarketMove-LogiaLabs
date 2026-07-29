<script>
  import { onMount } from 'svelte';
  import { status, wsConnected } from '../stores.js';
  import { fetchMode, setMode } from '../api.js';

  $: s = $status;
  $: mode = s?.mode || 'PAPER';
  $: symbol = s?.symbol || 'QQQ';
  $: position = s?.position || 'flat';
  $: entryPrice = s?.entry_price;
  $: realizedPnl = s?.realized_pnl;
  $: unrealizedPnl = s?.unrealized_pnl;
  $: staleness = s?.staleness_secs ?? 0;
  $: lastClose = s?.last_close;

  // Phase 3.4: mode toggle modal
  let showModeModal = false;
  let modeInfo = null; // { parity_valid, parity_marker_age_secs, last_switch_ts }
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
      // The /api/mode response doesn't propagate to the status store directly;
      // the WebSocket ModeChange event will update the UI. Close the modal.
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

  $: stale = staleness > 120;
</script>

<div class="status-panel">
  <div class="panel-header">Status</div>

  <div class="row mode-row">
    <span class="label">Mode</span>
    <span class="badge mode-{mode.toLowerCase()}">{mode}</span>
    <button class="mode-toggle-btn" on:click={openModeModal} title="Switch mode">
      ⇄
    </button>
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
    <span class="value">{fmt(entryPrice)}</span>
  </div>

  <div class="row">
    <span class="label">Last</span>
    <span class="value">{fmt(lastClose)}</span>
  </div>

  <div class="row">
    <span class="label">Realized PnL</span>
    <span class="value {pnlColor(realizedPnl)}">{fmt(realizedPnl)}</span>
  </div>

  <div class="row">
    <span class="label">Unrealized PnL</span>
    <span class="value {pnlColor(unrealizedPnl)}">{fmt(unrealizedPnl)}</span>
  </div>

  <div class="row staleness {stale ? 'stale' : ''}">
    <span class="label">Staleness</span>
    <span class="value">{staleness}s</span>
  </div>

  <div class="row">
    <span class="label">WS</span>
    <span class="ws-dot {$wsConnected ? 'on' : 'off'}"></span>
  </div>
</div>

{#if showModeModal}
  <div class="modal-backdrop" on:click={closeModeModal}>
    <div class="modal" on:click|stopPropagation>
      <h3>Switch to {targetMode.toUpperCase()} Trading</h3>

      {#if targetMode === 'live'}
        <p class="warn">
          ⚠️ This will execute real orders against your broker.
        </p>
      {/if}

      <div class="status-block">
        {#if modeInfo}
          <div class="status-row">
            <span class="status-label">Parity marker:</span>
            <span class="{modeInfo.parity_valid ? 'ok' : 'bad'}">
              {modeInfo.parity_valid ? 'valid' : 'EXPIRED'}
            </span>
          </div>
          <div class="status-row">
            <span class="status-label">Age:</span>
            <span>{parityAgeLabel(modeInfo.parity_marker_age_secs)}</span>
          </div>
          {#if modeInfo.last_switch_ts}
            <div class="status-row">
              <span class="status-label">Last switch:</span>
              <span>{new Date(modeInfo.last_switch_ts * 1000).toLocaleString()}</span>
            </div>
          {/if}
        {:else}
          <div class="status-row">Loading…</div>
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
        <button class="btn-cancel" on:click={closeModeModal} disabled={modalBusy}>
          Cancel
        </button>
        <button
          class="btn-confirm target-{targetMode}"
          on:click={submitMode}
          disabled={modalBusy || (targetMode === 'live' && modeInfo && !modeInfo.parity_valid)}
        >
          {modalBusy ? 'Switching…' : `Switch to ${targetMode.toUpperCase()}`}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .status-panel {
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

  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.85rem;
  }

  .mode-row { gap: 0.4rem; }

  .label {
    color: #8b949e;
  }

  .value {
    color: #c9d1d9;
    font-variant-numeric: tabular-nums;
  }

  .value.pos { color: #3fb950; }
  .value.neg { color: #f85149; }

  .badge {
    padding: 0.1rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
    font-weight: 600;
  }

  .mode-paper { background: #1f6feb22; color: #58a6ff; }
  .mode-live { background: #da363322; color: #f85149; }

  .pos-long { background: #23863622; color: #3fb950; }
  .pos-short { background: #da363322; color: #f85149; }
  .pos-flat { background: #30363d; color: #8b949e; }

  .staleness.stale .value { color: #f85149; }

  .ws-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    display: inline-block;
  }
  .ws-dot.on { background: #3fb950; box-shadow: 0 0 4px #3fb950; }
  .ws-dot.off { background: #f85149; }

  .mode-toggle-btn {
    background: transparent;
    border: 1px solid #30363d;
    color: #8b949e;
    border-radius: 4px;
    padding: 0 0.4rem;
    font-size: 0.9rem;
    cursor: pointer;
    line-height: 1.4;
  }
  .mode-toggle-btn:hover { border-color: #58a6ff; color: #58a6ff; }

  /* Modal */
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .modal {
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 8px;
    padding: 1.5rem;
    min-width: 360px;
    max-width: 480px;
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
  }

  .modal h3 {
    margin: 0;
    color: #c9d1d9;
    font-size: 1.05rem;
  }

  .warn {
    color: #f85149;
    background: #da363322;
    padding: 0.5rem 0.75rem;
    border-radius: 4px;
    margin: 0;
    font-size: 0.85rem;
  }

  .status-block {
    background: #0d1117;
    border: 1px solid #30363d;
    border-radius: 4px;
    padding: 0.6rem 0.8rem;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .status-row {
    display: flex;
    justify-content: space-between;
    font-size: 0.85rem;
  }

  .status-label { color: #8b949e; }
  .ok { color: #3fb950; font-weight: 600; }
  .bad { color: #f85149; font-weight: 600; }

  .form-row {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .form-row label {
    color: #8b949e;
    font-size: 0.8rem;
  }

  .form-row input {
    background: #0d1117;
    border: 1px solid #30363d;
    color: #c9d1d9;
    padding: 0.5rem 0.6rem;
    border-radius: 4px;
    font-size: 1rem;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.3em;
  }

  .form-row input:focus {
    outline: none;
    border-color: #58a6ff;
  }

  .error {
    color: #f85149;
    background: #da363322;
    padding: 0.5rem 0.75rem;
    border-radius: 4px;
    font-size: 0.85rem;
  }

  .actions {
    display: flex;
    gap: 0.5rem;
    justify-content: flex-end;
  }

  .btn-cancel, .btn-confirm {
    padding: 0.5rem 1rem;
    border-radius: 4px;
    border: 1px solid #30363d;
    background: transparent;
    color: #c9d1d9;
    font-size: 0.85rem;
    cursor: pointer;
  }
  .btn-cancel:hover { border-color: #8b949e; }

  .btn-confirm.target-live {
    background: #da363344;
    border-color: #f85149;
    color: #f85149;
  }
  .btn-confirm.target-live:hover:not(:disabled) {
    background: #f85149;
    color: #fff;
  }
  .btn-confirm.target-paper {
    background: #1f6feb22;
    border-color: #58a6ff;
    color: #58a6ff;
  }
  .btn-confirm.target-paper:hover:not(:disabled) {
    background: #1f6feb;
    color: #fff;
  }
  .btn-confirm:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
