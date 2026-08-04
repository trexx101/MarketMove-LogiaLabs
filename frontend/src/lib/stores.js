import { writable } from 'svelte/store';

/** WebSocket connection status (boolean) */
export const wsConnected = writable(false);

/** Latest /api/status snapshot, updated by REST + WS PnlTick/ModeChange */
export const status = writable(null);

/** Predictions: { latest, history } from /api/predictions, updated by WS PredictionUpdate */
export const predictions = writable(null);

/** Feature vector: { features, normalized, timestamp } from WS FeatureUpdate */
export const features = writable(null);

/** Trade fills from WS TradeFill events — prepended, capped at 50 */
export const trades = writable([]);

/** Unified event log — prepended from WS, capped at 100 */
export const events = writable([]);

/** Accuracy / model health from /api/accuracy (may 503) */
export const accuracy = writable(null);

/** Chart data: { candles, sma } from /api/chart */
export const chartData = writable(null);
