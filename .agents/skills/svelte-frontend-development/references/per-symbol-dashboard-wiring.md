# Per-Symbol API Wiring for Multi-Model Dashboards

When a Svelte dashboard switches between trading models/assets, every REST helper that was hardcoded to a single symbol must accept a symbol (or model_id) parameter, and every caller must pass it.

## API helpers with optional symbol/model_id

```javascript
// frontend/src/lib/api.js

export async function fetchStatus(modelId = null, symbol = null) {
  let url = `${API_BASE}/status`;
  const params = new URLSearchParams();
  if (modelId) params.append('model_id', modelId);
  if (symbol) params.append('symbol', symbol);
  if (params.toString()) url += `?${params.toString()}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`status: ${res.status}`);
  return res.json();
}

export async function fetchPredictions(symbol = null) {
  let url = `${API_BASE}/predictions`;
  if (symbol) url += `?symbol=${encodeURIComponent(symbol)}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`predictions: ${res.status}`);
  return res.json();
}

export async function fetchChart(limit = 90, symbol = null) {
  let url = `${API_BASE}/chart?limit=${limit}`;
  if (symbol) url += `&symbol=${encodeURIComponent(symbol)}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`chart: ${res.status}`);
  return res.json();
}

export async function fetchAccuracy(symbol = null) {
  let url = `${API_BASE}/accuracy`;
  if (symbol) url += `?symbol=${encodeURIComponent(symbol)}`;
  const res = await fetch(url);
  if (!res.ok) return null;
  return res.json();
}
```

## Dashboard loader passes active model

```svelte
<script>
  import { activeModelId, models, setSlice } from '../lib/stores.js';
  import { fetchStatus, fetchPredictions, fetchChart, fetchAccuracy } from '../lib/api.js';

  async function loadModelData(modelId, symbol) {
    const mid = modelId || 'legacy';

    try {
      const s = await fetchStatus(modelId, symbol);
      setSlice(mid, 'status', s);
    } catch (e) { console.error('Failed to fetch status:', e); }

    try {
      const p = await fetchPredictions(symbol);
      setSlice(mid, 'predictions', p);
    } catch (e) { console.error('Failed to fetch predictions:', e); }

    try {
      const c = await fetchChart(90, symbol);
      setSlice(mid, 'chartData', c);
    } catch (e) { console.error('Failed to fetch chart:', e); }

    try {
      const a = await fetchAccuracy(symbol);
      if (a) setSlice(mid, 'accuracy', a);
    } catch (e) { console.error('Failed to fetch accuracy:', e); }
  }

  async function onModelChange(ev) {
    activeModelId.set(ev.target.value);
    const m = $models.find((mm) => mm.model_id === ev.target.value);
    if (m) await loadModelData(m.model_id, m.primary_symbol);
  }
</script>
```

## Periodic status poll must use the active model

```javascript
statusInterval = setInterval(async () => {
  const mid = get(activeModelId);
  if (!mid) return;
  const m = $models.find((mm) => mm.model_id === mid);
  if (!m) return;
  try {
    const s = await fetchStatus(mid, m.primary_symbol);
    setSlice(mid, 'status', s);
  } catch (e) {
    // Silent — WS may still be delivering updates
  }
}, 30000);
```

## StatusPanel: show active model + pair

```svelte
<script>
  import { status, models, activeModelId } from '../stores.js';

  $: s = $status;
  $: activeModel = $models.find((m) => m.model_id === $activeModelId);
  $: primarySymbol = activeModel?.primary_symbol || s?.symbol || '—';
  $: inverseSymbol = activeModel?.inverse_symbol || '';
</script>

<div class="row">
  <span class="label">Symbol</span>
  <span class="value">
    {primarySymbol}{inverseSymbol ? ` / ${inverseSymbol}` : ''}
  </span>
</div>

<div class="row">
  <span class="label">Model</span>
  <span class="value model-id">{activeModel?.model_id || '—'}</span>
</div>
```

## Key rule

The component that needs to switch must derive its symbol from the `models` list using `activeModelId`, not from the `status` store. The `status` store is per-model and only knows the symbol the backend returned for that model; the `models` store is the source of truth for the full pair (`primary_symbol` / `inverse_symbol`).
