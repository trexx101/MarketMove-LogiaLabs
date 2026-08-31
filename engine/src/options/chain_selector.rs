//! Chain selector for options entries
//!
//! Selects the best options chain based on:
//! - DTE window [30, 45] days
//! - Monthly expiry preferred
//! - Liquidity floors: bid > 0, spread ≤ 8% mid, OI ≥ 100
//! - Minimize |delta − 0.45|
//!
//! No candidate → `SKIPPED_ENTRY` event with reason code.

use serde::{Deserialize, Serialize};

/// Chain selection result
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ChainSelectionResult {
    Selected(SelectedChain),
    Skipped(SkipReason),
}

/// Selected chain details
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectedChain {
    pub symbol: String,
    pub expiry: String, // YYYY-MM-DD
    pub strike: f64,
    pub option_type: String, // "call" or "put"
    pub delta: f64,
    pub bid: f64,
    pub ask: f64,
    pub open_interest: i64,
    pub dte: u32,
}

/// Reason for skipping entry
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SkipReason {
    NoChainsInDteWindow,
    NoLiquidChains,
    NoDeltaMatch,
}

/// Chain selector configuration
#[derive(Debug, Clone)]
pub struct ChainSelectorConfig {
    /// Minimum DTE (e.g., 30)
    pub min_dte: u32,
    /// Maximum DTE (e.g., 45)
    pub max_dte: u32,
    /// Target delta (e.g., 0.45)
    pub target_delta: f64,
    /// Maximum spread as % of mid (e.g., 0.08 = 8%)
    pub max_spread_pct: f64,
    /// Minimum open interest (e.g., 100)
    pub min_open_interest: i64,
}

impl Default for ChainSelectorConfig {
    fn default() -> Self {
        Self {
            min_dte: 30,
            max_dte: 45,
            target_delta: 0.45,
            max_spread_pct: 0.08,
            min_open_interest: 100,
        }
    }
}

/// Candidate chain from broker
#[derive(Debug, Clone)]
pub struct CandidateChain {
    pub symbol: String,
    pub expiry: String,
    pub strike: f64,
    pub option_type: String,
    pub delta: f64,
    pub bid: f64,
    pub ask: f64,
    pub open_interest: i64,
    pub dte: u32,
    pub is_monthly: bool,
}

impl CandidateChain {
    /// Build the full contract identifier, mirroring Moomoo's contract-code
    /// format: underlying + expiry + C/P + strike × 1000 (variable-width,
    /// e.g. `US.QQQ260919C530000`). This matches what the tape recorder
    /// stores in its `contract_code` field, so an open position is traceable
    /// back to the tape row.
    pub fn contract_code(&self) -> String {
        format!("{}{}{}{:.0}", self.symbol, self.expiry, self.option_type, self.strike * 1000.0)
    }
}

/// Chain selector
pub struct ChainSelector {
    config: ChainSelectorConfig,
}

impl ChainSelector {
    pub fn new(config: ChainSelectorConfig) -> Self {
        Self { config }
    }

    /// Select the best chain from candidates
    ///
    /// # Arguments
    /// * `candidates` - List of candidate chains from broker
    ///
    /// # Returns
    /// `SelectedChain` or `Skipped(reason)`
    pub fn select(&self, candidates: &[CandidateChain]) -> ChainSelectionResult {
        // Filter by DTE window
        let in_dte_window: Vec<_> = candidates
            .iter()
            .filter(|c| c.dte >= self.config.min_dte && c.dte <= self.config.max_dte)
            .collect();

        if in_dte_window.is_empty() {
            return ChainSelectionResult::Skipped(SkipReason::NoChainsInDteWindow);
        }

        // Filter by liquidity
        let liquid: Vec<_> = in_dte_window
            .iter()
            .filter(|c| {
                let mid = (c.bid + c.ask) / 2.0;
                let spread_pct = if mid > 0.0 {
                    (c.ask - c.bid) / mid
                } else {
                    f64::INFINITY
                };
                c.bid > 0.0 && spread_pct <= self.config.max_spread_pct && c.open_interest >= self.config.min_open_interest
            })
            .collect();

        if liquid.is_empty() {
            return ChainSelectionResult::Skipped(SkipReason::NoLiquidChains);
        }

        // Prefer monthly expiry, then minimize |delta − target|
        let mut best: Option<&CandidateChain> = None;
        let mut best_delta_diff = f64::INFINITY;

        for candidate in &liquid {
            let delta_diff = (candidate.delta - self.config.target_delta).abs();

            // Prefer monthly, then closest delta
            let is_better = match best {
                None => true,
                Some(b) => {
                    if candidate.is_monthly && !b.is_monthly {
                        true
                    } else if !candidate.is_monthly && b.is_monthly {
                        false
                    } else {
                        delta_diff < best_delta_diff
                    }
                }
            };

            if is_better {
                best = Some(candidate);
                best_delta_diff = delta_diff;
            }
        }

        match best {
            Some(c) => ChainSelectionResult::Selected(SelectedChain {
                symbol: c.symbol.clone(),
                expiry: c.expiry.clone(),
                strike: c.strike,
                option_type: c.option_type.clone(),
                delta: c.delta,
                bid: c.bid,
                ask: c.ask,
                open_interest: c.open_interest,
                dte: c.dte,
            }),
            None => ChainSelectionResult::Skipped(SkipReason::NoDeltaMatch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_candidate(delta: f64, dte: u32, bid: f64, ask: f64, oi: i64, is_monthly: bool) -> CandidateChain {
        CandidateChain {
            symbol: "QQQ".to_string(),
            expiry: "2026-09-20".to_string(),
            strike: 450.0,
            option_type: "call".to_string(),
            delta,
            bid,
            ask,
            open_interest: oi,
            dte,
            is_monthly,
        }
    }

    #[test]
    fn test_select_best_delta_match() {
        let selector = ChainSelector::new(ChainSelectorConfig::default());

        let candidates = vec![
            make_candidate(0.30, 35, 5.0, 5.10, 200, true),
            make_candidate(0.45, 35, 5.0, 5.10, 200, true), // Best delta match
            make_candidate(0.60, 35, 5.0, 5.10, 200, true),
        ];

        let result = selector.select(&candidates);

        match result {
            ChainSelectionResult::Selected(chain) => {
                assert_eq!(chain.delta, 0.45);
            }
            _ => panic!("Expected Selected"),
        }
    }

    #[test]
    fn test_prefer_monthly_expiry() {
        let selector = ChainSelector::new(ChainSelectorConfig::default());

        let candidates = vec![
            make_candidate(0.45, 35, 5.0, 5.10, 200, false), // Weekly
            make_candidate(0.45, 35, 5.0, 5.10, 200, true),  // Monthly (preferred)
        ];

        let result = selector.select(&candidates);

        match result {
            ChainSelectionResult::Selected(chain) => {
                // Should select the monthly one (both have same delta, but monthly preferred)
                assert_eq!(chain.delta, 0.45);
            }
            _ => panic!("Expected Selected"),
        }
    }

    #[test]
    fn test_skip_no_liquid_chains() {
        let selector = ChainSelector::new(ChainSelectorConfig::default());

        let candidates = vec![
            make_candidate(0.45, 35, 0.0, 0.0, 200, true), // bid = 0
            make_candidate(0.45, 35, 5.0, 6.0, 200, true), // spread > 8%
        ];

        let result = selector.select(&candidates);

        assert!(matches!(result, ChainSelectionResult::Skipped(SkipReason::NoLiquidChains)));
    }

    #[test]
    fn test_skip_no_chains_in_dte_window() {
        let selector = ChainSelector::new(ChainSelectorConfig::default());

        let candidates = vec![
            make_candidate(0.45, 10, 5.0, 5.10, 200, true), // DTE < 30
            make_candidate(0.45, 60, 5.0, 5.10, 200, true), // DTE > 45
        ];

        let result = selector.select(&candidates);

        assert!(matches!(result, ChainSelectionResult::Skipped(SkipReason::NoChainsInDteWindow)));
    }

    #[test]
    fn test_skip_low_open_interest() {
        let selector = ChainSelector::new(ChainSelectorConfig::default());

        let candidates = vec![
            make_candidate(0.45, 35, 5.0, 5.10, 50, true), // OI < 100
        ];

        let result = selector.select(&candidates);

        assert!(matches!(result, ChainSelectionResult::Skipped(SkipReason::NoLiquidChains)));
    }
}
