<script>
  import { onMount } from 'svelte';
  import { fetchEquityFeatures } from '../api.js';
  import { features, activeModelId, models } from '../stores.js';

  const FEATURE_DEFS = [
    { key: 'trend_slope',            label: 'trend_slope',  center: 0,   scale: 0.05 },
    { key: 'trend_adx',              label: 'trend_adx',    center: 25,  scale: 50   },
    { key: 'rsi_14',                 label: 'rsi_14',       center: 50,  scale: 50   },
    { key: 'vix_regime',             label: 'vix_regime',   center: 1,   scale: 2    },
    { key: 'tlt_corr_20d',           label: 'tlt_corr',     center: 0,   scale: 1    },
    { key: 'rvol_20d',               label: 'rvol_20d',     center: 1,   scale: 1    },
    { key: 'gap_pct',                label: 'gap_pct',      center: 0,   scale: 0.03 },
    { key: 'drawdown_from_50d_high', label: 'dd_50d',       center: 0,   scale: 0.1  },
  ];

  let localFeatures = null;
  let error = null;
  let lastModelId = null;

  // §8: reload features when the active model changes.
  $: if ($activeModelId && $activeModelId !== lastModelId) {
    lastModelId = $activeModelId;
    loadFeatures();
  }

  async function loadFeatures() {
    try {
      const m = $models.find((mm) => mm.model_id === $activeModelId);
      const sym = m?.primary_symbol || 'QQQ';
      const data = await fetchEquityFeatures(sym, 500);
      if (data.latest) {
        localFeatures = data.latest;
      }
    } catch (e) {
      error = e.message;
    }
  }

  onMount(loadFeatures);

  $: if ($features) {
    localFeatures = $features;
  }

  $: resolvedFeatures = (() => {
    if (!localFeatures) return null;
    if (localFeatures.trend_slope !== undefined) return localFeatures;
    if (Array.isArray(localFeatures.features)) {
      const obj = {};
      FEATURE_DEFS.forEach((d, i) => {
        obj[d.key] = localFeatures.features[i] ?? 0;
      });
      return obj;
    }
    return localFeatures;
  })();

  $: displayValues = (() => {
    if (!resolvedFeatures) return FEATURE_DEFS.map((d) => ({ ...d, norm: 0, raw: null }));
    return FEATURE_DEFS.map((d) => {
      const raw = resolvedFeatures[d.key];
      const norm = raw != null ? (raw - d.center) / d.scale : 0;
      return { ...d, norm, raw };
    });
  })();

  function barColor(v) {
    if (v > 0) return 'var(--green)';
    if (v < 0) return 'var(--red)';
    return 'var(--text-secondary)';
  }

  function fmtRaw(v) {
    if (v == null) return '';
    if (Math.abs(v) < 0.01) return v.toFixed(4);
    return v.toFixed(2);
  }
</script>

<div class="card">
  <div class="card-header">Feature Inspector</div>
  {#if error}
    <div class="error">Error: {error}</div>
  {/if}
  <div class="bars">
    {#each displayValues as fv}
      <div class="bar-row">
        <span class="bar-label">{fv.label}</span>
        <div class="bar-track">
          <div class="bar-zero"></div>
          <div
            class="bar-fill"
            style="background:{barColor(fv.norm)}; {fv.norm >= 0 ? 'left: 50%;' : 'right: 50%;'} width: {Math.min(50, Math.abs(fv.norm) * 50)}%;"
          ></div>
        </div>
        <span class="bar-raw">{fmtRaw(fv.raw)}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0.85rem;
  }

  .card-header {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-secondary);
    font-weight: 600;
    margin-bottom: 0.6rem;
  }

  .bars {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .bar-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-size: 0.75rem;
  }

  .bar-label {
    width: 70px;
    color: var(--text-secondary);
    font-family: var(--font-mono);
    font-size: 0.72rem;
    flex-shrink: 0;
  }

  .bar-track {
    flex: 1;
    height: 16px;
    background: var(--bg-inset);
    border-radius: var(--radius-xs);
    position: relative;
    overflow: hidden;
  }

  .bar-zero {
    position: absolute;
    left: 50%;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--border-light);
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
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: 0.7rem;
    flex-shrink: 0;
  }

  .error { color: var(--red); font-size: 0.78rem; }
</style>
