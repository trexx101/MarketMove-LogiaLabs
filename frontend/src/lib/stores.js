import { writable, derived, get } from 'svelte/store';

/** WebSocket connection status (boolean) */
export const wsConnected = writable(false);

// ---------------------------------------------------------------------------
// §8 Multi-model stores
// ---------------------------------------------------------------------------

/**
 * The currently selected model_id. Components that display per-model data
 * read from this to pick the right slice. Defaults to null until
 * /api/models is loaded; Dashboard will set the first model on mount.
 */
export const activeModelId = writable(null);

/**
 * Full list of registered trading models from /api/models.
 * Array of { model_id, primary_symbol, inverse_symbol, model_path,
 *             norm_stats_path, budget_usd, enabled, deployed_at,
 *             last_wf_ic, last_wf_at, notes }
 */
export const models = writable([]);

/**
 * Per-model telemetry store. Each model_id maps to a slice object:
 *   { status, predictions, features, trades, accuracy, chartData }
 *
 * Components read via `modelSlice` derived store (below) which picks the
 * active model's slice, creating an empty one on first access.
 */
const _slices = writable({});

/** Get or create a slice for a model_id (mutates the _slices store). */
function ensureSlice(modelId) {
  if (!modelId) return null;
  const slices = get(_slices);
  if (!slices[modelId]) {
    slices[modelId] = {
      status: null,
      predictions: null,
      features: null,
      trades: [],
      accuracy: null,
      chartData: null,
    };
    _slices.set(slices);
  }
  return slices[modelId];
}

/**
 * Derived store: the telemetry slice for the active model.
 * Components subscribe to this instead of the old flat stores.
 */
export const modelSlice = derived(
  [activeModelId, _slices],
  ([$activeModelId, $slices]) => {
    if (!$activeModelId) {
      return {
        status: null,
        predictions: null,
        features: null,
        trades: [],
        accuracy: null,
        chartData: null,
      };
    }
    return $slices[$activeModelId] || {
      status: null,
      predictions: null,
      features: null,
      trades: [],
      accuracy: null,
      chartData: null,
    };
  },
);

/**
 * Update a specific field on a specific model's slice.
 * Used by the WS handler and REST fetchers.
 *
 *   updateSlice('qqq-v1', 'status', (s) => ({ ...s, mode: 'paper' }));
 *   updateSlice('qqq-v1', 'trades', (list) => [entry, ...list].slice(0, 50));
 */
export function updateSlice(modelId, field, updater) {
  if (!modelId) return;
  _slices.update((slices) => {
    if (!slices[modelId]) {
      slices[modelId] = {
        status: null,
        predictions: null,
        features: null,
        trades: [],
        accuracy: null,
        chartData: null,
      };
    }
    const slice = slices[modelId];
    const oldVal = slice[field];
    const newVal = typeof updater === 'function' ? updater(oldVal) : updater;
    slice[field] = newVal;
    return { ...slices };
  });
}

/**
 * Set a specific field on a specific model's slice (non-functional update).
 * Convenience for REST fetchers that just `.set()` the whole value.
 */
export function setSlice(modelId, field, value) {
  updateSlice(modelId, field, value);
}

// ---------------------------------------------------------------------------
// Global stores (not per-model)
// ---------------------------------------------------------------------------

/** Unified event log — prepended from WS, capped at 100.
 * Events are global (not per-model) since the Events view shows all models. */
export const events = writable([]);

// ---------------------------------------------------------------------------
// Legacy backward-compat: the old flat stores still exist for components
// that haven't been migrated yet. They proxy to the active model's slice.
// New components should use `modelSlice` + `activeModelId` instead.
// ---------------------------------------------------------------------------

export const status = derived(
  [activeModelId, _slices],
  ([$id, $slices]) => $slices[$id]?.status ?? null,
);
export const predictions = derived(
  [activeModelId, _slices],
  ([$id, $slices]) => $slices[$id]?.predictions ?? null,
);
export const features = derived(
  [activeModelId, _slices],
  ([$id, $slices]) => $slices[$id]?.features ?? null,
);
export const trades = derived(
  [activeModelId, _slices],
  ([$id, $slices]) => $slices[$id]?.trades ?? [],
);
export const accuracy = derived(
  [activeModelId, _slices],
  ([$id, $slices]) => $slices[$id]?.accuracy ?? null,
);
export const chartData = derived(
  [activeModelId, _slices],
  ([$id, $slices]) => $slices[$id]?.chartData ?? null,
);
