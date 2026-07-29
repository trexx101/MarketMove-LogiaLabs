<script>
  import { onMount } from 'svelte';
  import { fetchEquityFeatures } from '../api.js';
  import { features } from '../stores.js';

  // Must match EQ_FEATURE_NAMES in engine/src/features/equities_v2.rs
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

  // Normalize WS payload {features: [...], normalized: [...]} into the same
  // named-field shape as the REST /api/equity/features response.
  $: resolvedFeatures = (() => {
    if (!localFeatures) return null;
    // REST shape: { trend_slope, trend_adx, ... }
    if (localFeatures.trend_slope !== undefined) return localFeatures;
    // WS shape: { features: [f0, f1, ...], normalized: [n0, n1, ...] }
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
