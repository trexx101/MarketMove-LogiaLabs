import {
  wsConnected,
  status,
  predictions,
  features,
  trades,
  chartData,
  events,
} from './stores.js';

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
 */
function handleMessage(raw) {
  let msg;
  try {
    msg = JSON.parse(raw.data);
  } catch {
    console.warn('[ws] non-JSON message, ignoring');
    return;
  }

  switch (msg.type) {
    case 'PnlTick':
      status.update((s) => ({
        ...(s || {}),
        realized_pnl: msg.realized_pnl,
        unrealized_pnl: msg.unrealized_pnl,
        position: msg.position,
        entry_price: msg.entry_price,
        last_close: msg.last_close,
        last_candle_ts: msg.timestamp,
      }));
      break;

    case 'PredictionUpdate':
      predictions.update((p) => ({
        ...(p || {}),
        latest: {
          ...(p?.latest || {}),
          pred_1d: msg.pred_1d,
          pred_5d: msg.pred_5d,
          pred_21d: msg.pred_21d,
          timestamp: msg.timestamp,
        },
      }));
      // Also update status store so StatusPanel shows live predictions
      status.update((s) => ({
        ...(s || {}),
        pred_1d: msg.pred_1d,
        pred_5d: msg.pred_5d,
        pred_21d: msg.pred_21d,
      }));
      break;

    case 'FeatureUpdate':
      features.set({
        features: msg.features,
        normalized: msg.normalized,
        timestamp: msg.timestamp,
      });
      break;

    case 'TradeFill':
      trades.update((list) => {
        const entry = {
          time: msg.timestamp,
          side: msg.side,
          qty: msg.qty,
          price: msg.price,
          fee: msg.fee,
          realized_pnl: msg.realized_pnl,
        };
        return [entry, ...list].slice(0, 50);
      });
      break;

    case 'ModeChange':
      status.update((s) => ({
        ...(s || {}),
        mode: msg.mode,
      }));
      break;

    case 'StalenessAlert':
      status.update((s) => ({
        ...(s || {}),
        last_candle_ts: msg.last_candle_ts,
        staleness_secs: msg.seconds_since_last,
      }));
      break;

    case 'StrategyConfigChange':
      // Strategy params changed — trigger chart refresh
      chartData.set(null);
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
