<script>
  import { onMount } from 'svelte';
  import { fetchEquityFeatures } from '../api.js';
  import { features } from '../stores.js';

  const FEATURE_NAMES = [
    'ret_1d', 'ret_5d', 'ret_21d', 'rsi_14',
    'trend_adx', 'vol_atr', 'corr_vix', 'corr_tlt',
  ];

  let localFeatures = null;
  let error = null;

  onMount(async () => {
    try {
      const data = await fetchEquityFeatures('QQQ', 500);
      if (data.latest) {
        localFeatures = data.latest;
      }
    } catch (e) {
      error = e.message;
    }
  });

  // Prefer WS feature updates over REST
  $: if ($features) {
    localFeatures = $features;
  }

  $: displayValues = (() => {
    if (!localFeatures) return FEATURE_NAMES.map((name) => ({ name, value: 0, raw: null }));
    const norm = localFeatures.normalized || [];
    const raw = localFeatures.features || [];
    return FEATURE_NAMES.map((name, i) => ({
      name,
      value: norm[i] != null ? norm[i] : 0,
      raw: raw[i] != null ? raw[i] : null,
    }));
  })();

  function barColor(v) {
    if (v > 0) return '#3fb950';
    if (v < 0) return '#f85149';
    return '#8b949e';
  }

  function fmtRaw(v) {
    if (v == null) return '';
    if (Math.abs(v) < 0.01) return v.toFixed(4);
    return v.toFixed(2);
  }
</script>

<div class="feature-inspector">
  <div class="panel-header">Feature Inspector</div>
  {#if error}
    <div class="error">Error: {error}</div>
  {/if}
  <div class="bars">
    {#each displayValues as fv}
      <div class="bar-row">
        <span class="bar-label">{fv.name}</span>
        <div class="bar-track">
          <div class="bar-zero"></div>
          <div
            class="bar-fill"
            style="background:{barColor(fv.value)}; {fv.value >= 0 ? 'left: 50%;' : 'right: 50%;'} width: {Math.min(50, Math.abs(fv.value) * 50)}%;"
          ></div>
        </div>
        <span class="bar-raw">{fmtRaw(fv.raw)}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .feature-inspector {
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 8px;
    padding: 0.75rem;
  }

  .panel-header {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #8b949e;
    margin-bottom: 0.5rem;
  }

  .bars {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .bar-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.75rem;
  }

  .bar-label {
    width: 70px;
    color: #8b949e;
    font-family: monospace;
    flex-shrink: 0;
  }

  .bar-track {
    flex: 1;
    height: 14px;
    background: #21262d;
    border-radius: 3px;
    position: relative;
    overflow: hidden;
  }

  .bar-zero {
    position: absolute;
    left: 50%;
    top: 0;
    bottom: 0;
    width: 1px;
    background: #484f58;
  }

  .bar-fill {
    position: absolute;
    top: 0;
    bottom: 0;
    border-radius: 2px;
    transition: width 0.3s ease;
  }

  .bar-raw {
    width: 50px;
    text-align: right;
    color: #c9d1d9;
    font-family: monospace;
    font-size: 0.7rem;
    flex-shrink: 0;
  }

  .error { color: #f85149; font-size: 0.8rem; }
</style>
