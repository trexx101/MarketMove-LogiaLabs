use serde::Serialize;

use crate::strategy::{EquitySignalInput, EquityStrategyParams};

pub mod metrics;
pub mod replay;
pub mod rhai_plugin;
pub mod api;

/// Kinds of strategies the backtest engine can evaluate.
#[derive(Debug, Clone)]
pub enum StrategyKind {
    /// The built-in threshold/hysteresis strategy (Wave C equities).
    Threshold(EquityStrategyParams),
    /// A user-supplied Rhai script that returns i64 position (-1/0/1).
    Rhai(String),
}

/// Request body for `POST /api/backtest`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BacktestRequest {
    /// Optional strategy ID for persistence.
    #[serde(default)]
    pub strategy_id: Option<String>,
    /// "threshold" or "rhai"
    pub kind: String,
    /// Strategy parameters (JSON blob for threshold, or `{script: "..."}` for Rhai).
    #[serde(default)]
    pub params: serde_json::Value,
    /// Unix timestamp (seconds) – start of the backtest window.
    pub start_ts: i64,
    /// Unix timestamp (seconds) – end of the backtest window.
    pub end_ts: i64,
}

/// Full result returned to the caller.
#[derive(Debug, Clone, Serialize)]
pub struct BacktestResult {
    pub equity_curve: Vec<(i64, f64)>,
    pub metrics: BacktestMetrics,
    pub trades: Vec<BacktestTrade>,
}

/// Computed performance statistics.
#[derive(Debug, Clone, Serialize)]
pub struct BacktestMetrics {
    pub cagr: f64,
    pub sharpe: f64,
    pub sortino: f64,
    pub max_drawdown: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub trade_count: usize,
    pub total_return: f64,
    pub buy_hold_return: f64,
}

/// One round-trip trade (or open trade at the end of the window).
#[derive(Debug, Clone, Serialize)]
pub struct BacktestTrade {
    pub entry_ts: i64,
    pub exit_ts: Option<i64>,
    pub side: String,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub realized_pnl: f64,
}

/// Parsed strategy kind from the API request body.
pub(crate) fn parse_strategy_kind(kind: &str, params: &serde_json::Value) -> Result<StrategyKind, String> {
    match kind {
        "threshold" => {
            let p: EquityStrategyParams = serde_json::from_value(params.clone())
                .map_err(|e| format!("invalid threshold params: {e}"))?;
            Ok(StrategyKind::Threshold(p))
        }
        "rhai" => {
            let script = params
                .get("script")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "missing 'script' field in params for Rhai strategy".to_string())?
                .to_string();
            Ok(StrategyKind::Rhai(script))
        }
        other => Err(format!("unknown strategy kind '{other}'; expected 'threshold' or 'rhai'")),
    }
}

/// Internal input passed to the position state machine for each bar during replay.
#[derive(Debug, Clone)]
pub(crate) struct BarInput {
    pub pred_1d: f64,
    pub pred_5d: f64,
    pub pred_21d: f64,
    pub close: f64,
    pub sma: f64,
    pub sma_valid: bool,
    pub current_pos: i64,
}

impl BarInput {
    pub fn to_equity_signal(&self) -> EquitySignalInput {
        EquitySignalInput {
            pred_1d: self.pred_1d,
            pred_5d: self.pred_5d,
            pred_21d: self.pred_21d,
            current_close: self.close,
            sma: self.sma,
            sma_valid: self.sma_valid,
        }
    }
}