const API_BASE = '/api';

/**
 * Fetch current system status.
 * @returns {Promise<object>} StatusResponse
 */
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

/**
 * Fetch recent predictions.
 * @param {string|null} symbol - optional primary symbol filter
 * @returns {Promise<object>} PredictionsResponse
 */
export async function fetchPredictions(symbol = null) {
  let url = `${API_BASE}/predictions`;
  if (symbol) url += `?symbol=${encodeURIComponent(symbol)}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`predictions: ${res.status}`);
  return res.json();
}

/**
 * Fetch chart data (candles + SMA).
 * Default 90 daily candles (~6 months) so prediction cones have room to render.
 * Pass a different limit (10-1500) to widen/narrow the window.
 * @param {number} [limit=90] Number of candles to fetch
 * @param {string|null} symbol - optional primary symbol filter
 * @returns {Promise<object>} ChartResponse
 */
export async function fetchChart(limit = 90, symbol = null) {
  let url = `${API_BASE}/chart?limit=${limit}`;
  if (symbol) url += `&symbol=${encodeURIComponent(symbol)}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`chart: ${res.status}`);
  return res.json();
}

/**
 * Fetch model accuracy / IC drift metrics.
 * @param {string|null} symbol - optional primary symbol filter
 * @returns {Promise<object|null>} AccuracyResponse or null if unavailable
 */
export async function fetchAccuracy(symbol = null) {
  let url = `${API_BASE}/accuracy`;
  if (symbol) url += `?symbol=${encodeURIComponent(symbol)}`;
  const res = await fetch(url);
  if (!res.ok) return null;
  return res.json();
}

/**
 * Fetch equity OHLCV data for a symbol.
 * @param {string} symbol - e.g. "QQQ"
 * @param {number} limit - max rows
 * @returns {Promise<object>} EquityDataResponse
 */
export async function fetchEquityData(symbol = 'QQQ', limit = 500) {
  const res = await fetch(`${API_BASE}/equity/data?symbol=${symbol}&limit=${limit}`);
  if (!res.ok) throw new Error(`equity/data: ${res.status}`);
  return res.json();
}

/**
 * Fetch equity feature rows for a symbol.
 * @param {string} symbol - e.g. "QQQ"
 * @param {number} limit - max rows
 * @returns {Promise<object>} EquityFeaturesResponse
 */
export async function fetchEquityFeatures(symbol = 'QQQ', limit = 500) {
  const res = await fetch(`${API_BASE}/equity/features?symbol=${symbol}&limit=${limit}`);
  if (!res.ok) throw new Error(`equity/features: ${res.status}`);
  return res.json();
}

/**
 * Fetch the trading ledger (equity trades with cumulative PnL) for a symbol.
 * @param {string} symbol - e.g. "QQQ"
 * @param {number} limit - max rows
 * @returns {Promise<object>} EquityTradesResponse
 */
export async function fetchEquityTrades(symbol = 'QQQ', limit = 500) {
  const res = await fetch(`${API_BASE}/equity/trades?symbol=${symbol}&limit=${limit}`);
  if (!res.ok) throw new Error(`equity/trades: ${res.status}`);
  return res.json();
}

/**
 * Fetch historical engine events.
 * @param {number} [limit=100]
 * @param {string} [category] optional filter
 * @param {number} [since] timestamp filter
 * @param {string} [mode] 'paper' | 'live' filter
 * @returns {Promise<object[]>}
 */
export async function fetchEvents(limit = 100, category = null, since = null, mode = null) {
  let url = `${API_BASE}/events?limit=${limit}`;
  if (category) url += `&category=${encodeURIComponent(category)}`;
  if (since) url += `&since=${since}`;
  if (mode) url += `&mode=${mode}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`events: ${res.status}`);
  return res.json();
}

/**
 * Fetch current market state.
 * @returns {Promise<object>} MarketStateResponse
 */
export async function fetchMarketState() {
  const res = await fetch(`${API_BASE}/market_state`);
  if (!res.ok) throw new Error(`market_state: ${res.status}`);
  return res.json();
}

/**
 * Run a backtest with the given configuration.
 * @param {object} params - { strategy_id?, kind: "threshold"|"rhai", params: {entry_threshold, exit_threshold, sma_window} | {script}, start_ts, end_ts }
 * @returns {Promise<object>} BacktestResponse
 */
export async function fetchBacktest(params) {
  const res = await fetch(`${API_BASE}/backtest`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(params),
  });
  if (!res.ok) throw new Error(`backtest: ${res.status}`);
  return res.json();
}

/**
 * Fetch all saved strategies.
 * @returns {Promise<object[]>} Array of strategy objects
 */
export async function fetchStrategies() {
  const res = await fetch(`${API_BASE}/strategies`);
  if (!res.ok) throw new Error(`strategies: ${res.status}`);
  return res.json();
}

/**
 * Save a strategy configuration.
 * @param {object} config - { name, strategy_type, script_body?, params_json }
 * @returns {Promise<object>} Saved strategy
 */
export async function saveStrategy(config) {
  const res = await fetch(`${API_BASE}/strategies`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(config),
  });
  if (!res.ok) throw new Error(`strategies POST: ${res.status}`);
  return res.json();
}

/**
 * Fetch the current trading mode + parity marker status (Phase 3.4).
 * @returns {Promise<{mode: string, parity_marker_age_secs: number|null, parity_valid: boolean, last_switch_ts: number|null}>}
 */
export async function fetchMode() {
  const res = await fetch(`${API_BASE}/mode`);
  if (!res.ok) throw new Error(`mode: ${res.status}`);
  return res.json();
}

/**
 * Request a paper/live mode flip. Requires a valid 6-digit TOTP code.
 * @param {"paper"|"live"} mode - target mode
 * @param {string} authToken - 6-digit TOTP from the user's authenticator app
 * @returns {Promise<{success: boolean, message: string, mode: string}>}
 * @throws {Error} on 4xx/5xx (includes the engine's error message in the body)
 */
export async function setMode(mode, authToken) {
  const res = await fetch(`${API_BASE}/mode`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ mode, auth_token: authToken }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`mode ${mode} rejected (${res.status}): ${text}`);
  }
  return res.json();
}

/**
 * Fetch current live quote (price, change, pct change) for QQQ from Yahoo Finance.
 * Polled every 30s by the chart component.
 * @returns {Promise<{symbol:string, price:number, prev_close:number, change:number, change_pct:number, timestamp:number}>}
 */
export async function fetchQuote() {
  const res = await fetch(`${API_BASE}/quote`);
  if (!res.ok) throw new Error(`quote: ${res.status}`);
  return res.json();
}

/**
 * Fetch current strategy configuration (Phase 3).
 * @param {string|null} [modelId] - §8.6: if supplied, fetch per-model config.
 * @returns {Promise<object>} StrategyConfigResponse
 */
export async function fetchStrategyConfig(modelId = null) {
  let url = `${API_BASE}/strategy-config`;
  if (modelId) url += `?model_id=${encodeURIComponent(modelId)}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`strategy-config: ${res.status}`);
  return res.json();
}

/**
 * Update strategy configuration at runtime (Phase 3).
 * @param {object} params - partial {entry_threshold?, exit_threshold?, sma_window?, pred_5d_filter?, enable_shorting?, short_entry_threshold?, short_exit_threshold?}
 * @param {string|null} [modelId] - §8.6: if supplied, update per-model config.
 * @returns {Promise<object>} Updated strategy config
 */
export async function saveStrategyConfig(params, modelId = null) {
  let url = `${API_BASE}/strategy-config`;
  if (modelId) url += `?model_id=${encodeURIComponent(modelId)}`;
  const res = await fetch(url, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(params),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`strategy-config PUT rejected (${res.status}): ${text}`);
  }
  return res.json();
}

// ---------------------------------------------------------------------------
// §8 Models API
// ---------------------------------------------------------------------------

/**
 * Fetch all registered trading models.
 * @returns {Promise<object[]>} Array of TradingModel
 */
export async function fetchModels() {
  const res = await fetch(`${API_BASE}/models`);
  if (!res.ok) throw new Error(`models: ${res.status}`);
  return res.json();
}

/**
 * Register a new trading model.
 * @param {object} body - { model_id, primary_symbol, inverse_symbol, model_path, norm_stats_path, budget_usd, notes? }
 * @returns {Promise<object>} Created TradingModel
 */
export async function registerModel(body) {
  const res = await fetch(`${API_BASE}/models`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`models POST: ${res.status}`);
  return res.json();
}

/**
 * Toggle the enabled flag for a model.
 * @param {string} modelId
 * @param {boolean} enabled
 * @returns {Promise<object>} Updated TradingModel
 */
export async function setModelEnabled(modelId, enabled) {
  const res = await fetch(`${API_BASE}/models/${encodeURIComponent(modelId)}/enabled`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ enabled }),
  });
  if (!res.ok) throw new Error(`models/${modelId}/enabled: ${res.status}`);
  return res.json();
}
