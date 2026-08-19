const API_BASE = '/api';

/**
 * List hyperopt candidates for an equity.
 * @param {string} equity - e.g. "QQQ", "SMH", "XLF"
 * @returns {Promise<{equity: string, candidates: Array}>}
 */
export async function fetchHyperoptCandidates(equity) {
  const res = await fetch(`${API_BASE}/hyperopt/${equity}/candidates`);
  if (!res.ok) throw new Error(`hyperopt candidates: ${res.status}`);
  return res.json();
}

/**
 * Fetch hyperopt pipeline status for an equity.
 * @param {string} equity - e.g. "QQQ", "SMH", "XLF"
 * @returns {Promise<{equity: string, pipeline_state: string, total_candidates: number, by_status: Object}>}
 */
export async function fetchHyperoptStatus(equity) {
  const res = await fetch(`${API_BASE}/hyperopt/${equity}/status`);
  if (!res.ok) throw new Error(`hyperopt status: ${res.status}`);
  return res.json();
}

/**
 * Promote a candidate to the next stage (CANDIDATE -> PAPER -> MICRO -> LIVE).
 * Gated: promotion only succeeds if the candidate meets evidence requirements.
 * @param {string} equity - equity the candidate belongs to
 * @param {string} id - candidate version id
 * @returns {Promise<{success: boolean, message: string}>}
 */
export async function promoteCandidate(equity, id) {
  const res = await fetch(`${API_BASE}/hyperopt/${equity}/promote/${id}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ target_status: 'AUTO' }),
  });
  if (!res.ok) throw new Error(`promote: ${res.status}`);
  return res.json();
}

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
 * Default 90 daily candles (~6 months) so prediction cones have room to render.
 * Pass a different limit (10-1500) to widen/narrow the window.
 * @param {number} [limit=90] Number of candles to fetch
 * @returns {Promise<object>} ChartResponse
 */
export async function fetchChart(limit = 90) {
  const res = await fetch(`${API_BASE}/chart?limit=${limit}`);
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

// ── Options endpoints (Phase 7) ─────────────────────────────────────────────

/**
 * List option positions, optionally filtered by underlying and/or status.
 * @param {object} [opts]
 * @param {string} [opts.underlying] - e.g. "QQQ"
 * @param {string} [opts.status] - e.g. "OPEN", "CLOSED"
 * @param {number} [opts.limit=100]
 * @returns {Promise<{positions: Array, count: number}>}
 */
export async function fetchOptionPositions({ underlying, status, limit = 100 } = {}) {
  const params = new URLSearchParams();
  if (underlying) params.set('underlying', underlying);
  if (status) params.set('status', status);
  params.set('limit', String(limit));
  const res = await fetch(`${API_BASE}/options/positions?${params}`);
  if (!res.ok) throw new Error(`options/positions: ${res.status}`);
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
/**
 * List closed option trades (trade history).
 * @param {object} [opts]
 * @param {string} [opts.underlying]
 * @param {number} [opts.limit=100]
 * @returns {Promise<{trades: Array, count: number}>}
 */
export async function fetchOptionTrades({ underlying, limit = 100 } = {}) {
  const params = new URLSearchParams();
  if (underlying) params.set('underlying', underlying);
  params.set('limit', String(limit));
  const res = await fetch(`${API_BASE}/options/trades?${params}`);
  if (!res.ok) throw new Error(`options/trades: ${res.status}`);
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
/**
 * Fetch the full options config registry with current values.
 * @returns {Promise<{entries: Array, count: number}>}
 */
export async function fetchOptionsConfig() {
  const res = await fetch(`${API_BASE}/options/config`);
  if (!res.ok) throw new Error(`options/config GET: ${res.status}`);
  return res.json();
}

/**
 * Write one or more options config values.
 * @param {Object<string, number>} values - key → value map
 * @returns {Promise<{applied: number, rejected: string[]}>}
 */
export async function saveOptionsConfig(values) {
  const res = await fetch(`${API_BASE}/options/config`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(values),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`options/config PUT rejected (${res.status}): ${text}`);
  }
  return res.json();
}

/**
 * Fetch recent hyperopt runs.
 * @param {number} [limit=20]
 * @returns {Promise<{runs: Array, count: number}>}
 */
export async function fetchHyperoptRuns(limit = 20) {
  const res = await fetch(`${API_BASE}/hyperopt/runs?limit=${limit}`);
  if (!res.ok) throw new Error(`hyperopt/runs: ${res.status}`);
  return res.json();
}

/**
 * Fetch tape recorder status (heartbeat + quota accounting).
 * @returns {Promise<{tapes: Array, count: number, healthy: number, stale: number, never_beat: number}>}
 */
export async function fetchTapeStatus() {
  const res = await fetch(`${API_BASE}/options/tape/status`);
  if (!res.ok) throw new Error(`options/tape/status: ${res.status}`);
  return res.json();
}

/**
 * Fetch engine events, searchable by category/mode/severity/equity.
 * @param {object} [opts]
 * @param {string} [opts.category] - trade | data | system | strategy | alert | advisor
 * @param {string} [opts.mode] - paper | live
 * @param {string} [opts.severity] - info | warn | error
 * @param {string} [opts.equity] - e.g. "QQQ"
 * @param {number} [opts.since] - ms epoch lower bound
 * @param {number} [opts.limit=100]
 * @returns {Promise<{events: Array, count: number}>}
 */
export async function fetchEvents({ category, mode, severity, equity, since, limit = 100 } = {}) {
  const params = new URLSearchParams();
  if (category) params.set('category', category);
  if (mode) params.set('mode', mode);
  if (severity) params.set('severity', severity);
  if (equity) params.set('equity', equity);
  if (since) params.set('since', String(since));
  params.set('limit', String(limit));
  const res = await fetch(`${API_BASE}/events?${params}`);
  if (!res.ok) throw new Error(`events: ${res.status}`);
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
