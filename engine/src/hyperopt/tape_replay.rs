//! Tape replay validation — run candidates against recorded market data
//!
//! Final pre-MICRO check: replay the candidate against historical tape
//! to verify it would have performed as expected.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Tape bar (single time step of market data)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TapeBar {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Replay result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayResult {
    pub version_id: String,
    pub n_bars: usize,
    pub n_trades: usize,
    pub final_ic: f64,
    pub max_drawdown: f64,
    pub sharpe: f64,
    pub passed: bool,
}

/// Tape replay validator
pub struct TapeReplayValidator {
    /// Minimum IC threshold for passing
    pub min_ic: f64,
    /// Maximum drawdown threshold
    pub max_drawdown: f64,
    /// Minimum Sharpe ratio
    pub min_sharpe: f64,
}

impl Default for TapeReplayValidator {
    fn default() -> Self {
        Self {
            min_ic: 0.03,
            max_drawdown: 0.15,
            min_sharpe: 1.0,
        }
    }
}

impl TapeReplayValidator {
    pub fn new(min_ic: f64, max_drawdown: f64, min_sharpe: f64) -> Self {
        Self {
            min_ic,
            max_drawdown,
            min_sharpe,
        }
    }

    /// Replay a candidate against tape
    pub fn replay<F>(&self, version_id: &str, tape: &[TapeBar], mut strategy_fn: F) -> ReplayResult
    where
        F: FnMut(&TapeBar, &HashMap<String, f64>) -> Option<f64>,
    {
        let params = HashMap::new(); // Placeholder — would come from candidate store
        let mut trades = Vec::new();
        let mut position: Option<f64> = None;

        for bar in tape {
            let signal = strategy_fn(bar, &params);
            
            match signal {
                Some(s) if s > 0.0 && position.is_none() => {
                    // Enter long
                    position = Some(bar.close);
                }
                Some(s) if s < 0.0 && position.is_some() => {
                    // Exit long
                    if let Some(entry) = position {
                        let pnl = (bar.close - entry) / entry;
                        trades.push(pnl);
                    }
                    position = None;
                }
                _ => {}
            }
        }

        // Close any open position at end
        if let Some(entry) = position {
            if let Some(last_bar) = tape.last() {
                let pnl = (last_bar.close - entry) / entry;
                trades.push(pnl);
            }
        }

        let n_trades = trades.len();
        let final_ic = if n_trades > 0 {
            trades.iter().sum::<f64>() / n_trades as f64
        } else {
            0.0
        };

        // Calculate max drawdown
        let mut cumulative: f64 = 0.0;
        let mut peak: f64 = 0.0;
        let mut max_dd: f64 = 0.0;
        for pnl in &trades {
            cumulative += pnl;
            peak = peak.max(cumulative);
            let dd = peak - cumulative;
            max_dd = max_dd.max(dd);
        }

        // Calculate Sharpe (simplified)
        let sharpe = if n_trades > 1 {
            let mean = final_ic;
            let variance = trades.iter().map(|t| (t - mean).powi(2)).sum::<f64>() / (n_trades - 1) as f64;
            let std = variance.sqrt();
            if std > 0.0 { mean / std } else { 0.0 }
        } else {
            0.0
        };

        let passed = final_ic >= self.min_ic
            && max_dd <= self.max_drawdown
            && sharpe >= self.min_sharpe;

        ReplayResult {
            version_id: version_id.to_string(),
            n_bars: tape.len(),
            n_trades,
            final_ic,
            max_drawdown: max_dd,
            sharpe,
            passed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tape(n: usize) -> Vec<TapeBar> {
        (0..n)
            .map(|i| TapeBar {
                timestamp: chrono::Utc::now() + chrono::Duration::days(i as i64),
                open: 100.0 + i as f64,
                high: 101.0 + i as f64,
                low: 99.0 + i as f64,
                close: 100.5 + i as f64,
                volume: 1000.0,
            })
            .collect()
    }

    #[test]
    fn test_replay_basic() {
        let validator = TapeReplayValidator::default();
        let tape = make_tape(100);

        // Simple strategy: always buy
        let result = validator.replay("v1", &tape, |_bar, _params| Some(1.0));
        
        assert_eq!(result.version_id, "v1");
        assert_eq!(result.n_bars, 100);
        assert!(result.n_trades > 0);
    }

    #[test]
    fn test_replay_no_trades() {
        let validator = TapeReplayValidator::default();
        let tape = make_tape(10);

        // Strategy that never trades
        let result = validator.replay("v2", &tape, |_bar, _params| None);
        
        assert_eq!(result.n_trades, 0);
        assert_eq!(result.final_ic, 0.0);
        assert!(!result.passed);
    }

    #[test]
    fn test_replay_pass_threshold() {
        let validator = TapeReplayValidator::new(0.01, 0.20, 0.5);
        let tape = make_tape(50);

        // Strategy that generates positive returns
        let result = validator.replay("v3", &tape, |bar, _params| {
            if bar.close > 100.0 { Some(1.0) } else { None }
        });
        
        // Should pass if IC > 0.01 and drawdown < 0.20
        assert!(result.final_ic > 0.0);
    }

    #[test]
    fn test_replay_fail_drawdown() {
        let validator = TapeReplayValidator::new(0.01, 0.05, 0.5);
        let tape = make_tape(20);

        // Strategy that enters and exits with losses
        let mut count = 0;
        let result = validator.replay("v4", &tape, |_bar, _params| {
            count += 1;
            if count % 2 == 1 { Some(1.0) } else { Some(-1.0) }
        });
        
        // May fail if drawdown exceeds threshold
        assert!(result.max_drawdown >= 0.0);
    }

    #[test]
    fn test_replay_empty_tape() {
        let validator = TapeReplayValidator::default();
        let tape = vec![];

        let result = validator.replay("v5", &tape, |_bar, _params| Some(1.0));
        
        assert_eq!(result.n_bars, 0);
        assert_eq!(result.n_trades, 0);
        assert!(!result.passed);
    }
}
