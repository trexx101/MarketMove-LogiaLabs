import {
  wsConnected,
  events,
  updateSlice,
  activeModelId,
} from './stores.js';
import { get as storeGet } from 'svelte/store';

let ws = null;
let reconnectTimer = null;
let backoff = 1000; // start at 1s
const MAX_BACKOFF = 30000; // cap at 30s
let manuallyClosed = false;

/**
 * Build the WebSocket URL from the current page location.
 * Uses wss:// for https pages, ws:// for http.
 */
function buildWsUrl() {
  const proto = window.location.protocol === 'https:' ? 'wss' : 'ws';
  return `${proto}://${window.location.host}/api/v1/ws`;
}

/**
 * Handle an incoming WS message by dispatching to the appropriate store.
 *
 * Per-model events (PnlTick, PredictionUpdate, FeatureUpdate, TradeFill,
 * StalenessAlert) carry `model_id` and `pair` fields (§8.3). The handler
 * routes them to the correct model's slice via `updateSlice`.
 *
 * Global events (ModeChange, StrategyConfigChange, EngineEvent) are
 * dispatched to the global stores — they are not per-model.
 */
function handleMessage(raw) {
  let msg;
  try {
    msg = JSON.parse(raw.data);
  } catch {
    console.warn('[ws] non-JSON message, ignoring');
    return;
  }

  const mid = msg.model_id || null;

  switch (msg.type) {
    case 'PnlTick':
      if (mid) {
        updateSlice(mid, 'status', (s) => ({
          ...(s || {}),
          model_id: mid,
          symbol: msg.pair?.split('/')[0] || s?.symbol,
          realized_pnl: msg.realized_pnl,
          unrealized_pnl: msg.unrealized_pnl,
          position: msg.position,
          entry_price: msg.entry_price,
          last_close: msg.last_close,
          last_candle_ts: msg.timestamp,
        }));
      }
      break;

    case 'PredictionUpdate':
      if (mid) {
        updateSlice(mid, 'predictions', (p) => ({
          ...(p || {}),
          latest: {
            ...(p?.latest || {}),
            pred_1d: msg.pred_1d,
            pred_5d: msg.pred_5d,
            pred_21d: msg.pred_21d,
            timestamp: msg.timestamp,
          },
        }));
        // Also update status so StatusPanel shows live predictions
        updateSlice(mid, 'status', (s) => ({
          ...(s || {}),
          pred_1d: msg.pred_1d,
          pred_5d: msg.pred_5d,
          pred_21d: msg.pred_21d,
        }));
      }
      break;

    case 'FeatureUpdate':
      if (mid) {
        updateSlice(mid, 'features', {
          features: msg.features,
          normalized: msg.normalized,
          timestamp: msg.timestamp,
        });
      }
      break;

    case 'TradeFill':
      if (mid) {
        updateSlice(mid, 'trades', (list) => {
          const entry = {
            time: msg.timestamp,
            side: msg.side,
            qty: msg.qty,
            price: msg.price,
            fee: msg.fee,
            realized_pnl: msg.realized_pnl,
            model_id: mid,
          };
          return [entry, ...list].slice(0, 50);
        });
      }
      break;

    case 'ModeChange':
      // Global — all models share the same trading mode.
      // Update the active model's status slice.
      {
        const activeId = storeGet(activeModelId);
        if (activeId) {
          updateSlice(activeId, 'status', (s) => ({
            ...(s || {}),
            mode: msg.mode,
          }));
        }
      }
      break;

    case 'StalenessAlert':
      if (mid) {
        updateSlice(mid, 'status', (s) => ({
          ...(s || {}),
          last_candle_ts: msg.last_candle_ts,
          staleness_secs: msg.seconds_since_last,
        }));
      }
      break;

    case 'StrategyConfigChange':
      // Global — clear chart data for the active model to trigger refresh
      {
        const activeId = storeGet(activeModelId);
        if (activeId) {
          updateSlice(activeId, 'chartData', null);
        }
      }
      break;

    case 'EngineEvent':
      events.update((list) => {
        const entry = {
          id: msg.id || null,
          ts: msg.ts,
          category: msg.category,
          severity: msg.severity,
          mode: msg.mode,
          source: msg.source,
          message: msg.message,
          payload: msg.payload,
        };
        return [entry, ...list].slice(0, 100);
      });
      break;

    default:
      // Unknown event type — ignore silently
      break;
  }
}

/**
 * Connect to the WebSocket endpoint with auto-reconnect.
 */
export function connectWebSocket() {
  if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
    return; // already connected or connecting
  }

  manuallyClosed = false;
  const url = buildWsUrl();

  try {
    ws = new WebSocket(url);
  } catch {
    scheduleReconnect();
    return;
  }

  ws.onopen = () => {
    backoff = 1000; // reset backoff
    wsConnected.set(true);
  };

  ws.onmessage = handleMessage;

  ws.onclose = () => {
    wsConnected.set(false);
    ws = null;
    if (!manuallyClosed) {
      scheduleReconnect();
    }
  };

  ws.onerror = () => {
    // Close to trigger onclose + reconnect
    if (ws) {
      ws.close();
    }
  };
}

/**
 * Schedule a reconnect with exponential backoff (1s → 30s max).
 */
function scheduleReconnect() {
  if (reconnectTimer) clearTimeout(reconnectTimer);
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connectWebSocket();
  }, backoff);
  backoff = Math.min(backoff * 2, MAX_BACKOFF);
}

/**
 * Manually disconnect and stop auto-reconnect.
 */
export function disconnectWebSocket() {
  manuallyClosed = true;
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  if (ws) {
    ws.onclose = null; // prevent reconnect
    ws.close();
    ws = null;
  }
  wsConnected.set(false);
}
