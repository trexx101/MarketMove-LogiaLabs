const API_BASE = '/api';

/**
 * Fetch current system status.
 * @returns {Promise<object>} StatusResponse
 */
export async function fetchStatus() {
  const res = await fetch(`${API_BASE}/status`);
  if (!res.ok) throw new Error(`status: ${res.status}`);
  return res.json();
}

/**
 * Fetch recent predictions.
 * @returns {Promise<object>} PredictionsResponse
 */
export async function fetchPredictions() {
  const res = await fetch(`${API_BASE}/predictions`);
  if (!res.ok) throw new Error(`predictions: ${res.status}`);
  return res.json();
}

/**
 * Fetch chart data (candles + SMA).
 * @returns {Promise<object>} ChartResponse
 */
export async function fetchChart() {
  const res = await fetch(`${API_BASE}/chart`);
  if (!res.ok) throw new Error(`chart: ${res.status}`);
  return res.json();
}

/**
 * Fetch model accuracy / IC drift metrics.
 * @returns {Promise<object|null>} AccuracyResponse or null if unavailable
 */
export async function fetchAccuracy() {
  const res = await fetch(`${API_BASE}/accuracy`);
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
 * Fetch current strategy configuration (Phase 3).
 * @returns {Promise<object>} StrategyConfigResponse
 */
export async function fetchStrategyConfig() {
  const res = await fetch(`${API_BASE}/strategy-config`);
  if (!res.ok) throw new Error(`strategy-config: ${res.status}`);
  return res.json();
}

/**
 * Update strategy configuration at runtime (Phase 3).
 * @param {object} params - partial {entry_threshold?, exit_threshold?, sma_window?, pred_5d_filter?, enable_shorting?, short_entry_threshold?, short_exit_threshold?}
 * @returns {Promise<object>} Updated strategy config
 */
export async function saveStrategyConfig(params) {
  const res = await fetch(`${API_BASE}/strategy-config`, {
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
