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
 * May return 503 if not yet implemented.
 * @returns {Promise<object|null>} AccuracyResponse or null if unavailable
 */
export async function fetchAccuracy() {
  const res = await fetch(`${API_BASE}/accuracy`);
  if (res.status === 503) return null;
  if (!res.ok) throw new Error(`accuracy: ${res.status}`);
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
 * Fetch current market state.
 * @returns {Promise<object>} MarketStateResponse
 */
export async function fetchMarketState() {
  const res = await fetch(`${API_BASE}/market_state`);
  if (!res.ok) throw new Error(`market_state: ${res.status}`);
  return res.json();
}
