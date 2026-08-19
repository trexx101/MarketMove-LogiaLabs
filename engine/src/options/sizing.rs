//! Position sizing module for options entries
//!
//! Formula: contracts = floor(equity × risk% / (stop_distance × delta × 100))
//! Capped by:
//!   1. Max premium per position (equity × premium_cap%)
//!   2. Max contracts per position (hard cap)
//!   3. Max portfolio premium (equity × portfolio_cap%)
//!
//! Emits `SKIPPED_ENTRY` with reason when any cap binds.

use serde::{Deserialize, Serialize};

/// Sizing result
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SizingResult {
    Sized(SizingDecision),
    Skipped(SizingSkipReason),
}

/// Sizing decision details
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SizingDecision {
    pub contracts: u32,
    pub premium_per_contract: f64,
    pub total_premium: f64,
    pub binding_cap: Option<CapType>,
}

/// Reason for skipping entry
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SizingSkipReason {
    ZeroContracts { reason: String },
    PremiumExceedsCap { required: f64, cap: f64 },
    PortfolioCapReached { current_premium: f64, cap: f64 },
}

/// Which cap bound the sizing
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CapType {
    Formula,
    MaxPremiumPerPosition,
    MaxContractsPerPosition,
    MaxPortfolioPremium,
}

/// Sizing configuration
#[derive(Debug, Clone)]
pub struct SizingConfig {
    /// Risk per trade as fraction (e.g., 0.01 = 1%)
    pub risk_per_trade: f64,
    /// Max premium per position as fraction of equity (e.g., 0.05 = 5%)
    pub max_premium_per_position: f64,
    /// Max contracts per position (hard cap)
    pub max_contracts_per_position: u32,
    /// Max total portfolio premium as fraction of equity (e.g., 0.20 = 20%)
    pub max_portfolio_premium: f64,
}

impl Default for SizingConfig {
    fn default() -> Self {
        Self {
            risk_per_trade: 0.01,
            max_premium_per_position: 0.05,
            max_contracts_per_position: 10,
            max_portfolio_premium: 0.20,
        }
    }
}

/// Position sizer
pub struct PositionSizer {
    config: SizingConfig,
}

impl PositionSizer {
    pub fn new(config: SizingConfig) -> Self {
        Self { config }
    }

    /// Calculate position size
    ///
    /// # Arguments
    /// * `equity` - Current account equity
    /// * `stop_distance` - Distance to stop loss (in underlying price units)
    /// * `delta` - Option delta
    /// * `ask_price` - Option ask price (premium per contract)
    /// * `current_portfolio_premium` - Total premium already deployed
    ///
    /// # Returns
    /// `Sized(decision)` or `Skipped(reason)`
    pub fn size(
        &self,
        equity: f64,
        stop_distance: f64,
        delta: f64,
        ask_price: f64,
        current_portfolio_premium: f64,
    ) -> SizingResult {
        // Formula: contracts = floor(equity × risk% / (stop_distance × delta × 100))
        let formula_contracts = if stop_distance > 0.0 && delta > 0.0 {
            (equity * self.config.risk_per_trade / (stop_distance * delta * 100.0)).floor() as u32
        } else {
            0
        };

        if formula_contracts == 0 {
            return SizingResult::Skipped(SizingSkipReason::ZeroContracts {
                reason: "formula produced 0 contracts".to_string(),
            });
        }

        // Cap 1: Max premium per position
        let max_premium_budget = equity * self.config.max_premium_per_position;
        let contracts_by_premium = if ask_price > 0.0 {
            (max_premium_budget / (ask_price * 100.0)).floor() as u32
        } else {
            u32::MAX
        };

        // Cap 2: Max contracts per position
        let contracts_by_hard_cap = self.config.max_contracts_per_position;

        // Cap 3: Max portfolio premium
        let remaining_portfolio_budget =
            equity * self.config.max_portfolio_premium - current_portfolio_premium;
        let contracts_by_portfolio = if ask_price > 0.0 {
            (remaining_portfolio_budget / (ask_price * 100.0)).floor() as u32
        } else {
            u32::MAX
        };

        if contracts_by_portfolio == 0 {
            return SizingResult::Skipped(SizingSkipReason::PortfolioCapReached {
                current_premium: current_portfolio_premium,
                cap: equity * self.config.max_portfolio_premium,
            });
        }

        // Take the minimum across all caps
        let mut contracts = formula_contracts;
        let mut binding = CapType::Formula;

        if contracts_by_premium < contracts {
            contracts = contracts_by_premium;
            binding = CapType::MaxPremiumPerPosition;
        }

        if contracts_by_hard_cap < contracts {
            contracts = contracts_by_hard_cap;
            binding = CapType::MaxContractsPerPosition;
        }

        if contracts_by_portfolio < contracts {
            contracts = contracts_by_portfolio;
            binding = CapType::MaxPortfolioPremium;
        }

        if contracts == 0 {
            return SizingResult::Skipped(SizingSkipReason::ZeroContracts {
                reason: "all caps reduced to 0 contracts".to_string(),
            });
        }

        let total_premium = contracts as f64 * ask_price * 100.0;

        SizingResult::Sized(SizingDecision {
            contracts,
            premium_per_contract: ask_price,
            total_premium,
            binding_cap: Some(binding),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_formula_sizing() {
        let sizer = PositionSizer::new(SizingConfig::default());

        // equity=100k, risk=1%, stop=10, delta=0.45
        // contracts = floor(100000 * 0.01 / (10 * 0.45 * 100)) = floor(2.22) = 2
        let result = sizer.size(100_000.0, 10.0, 0.45, 5.0, 0.0);

        match result {
            SizingResult::Sized(d) => {
                assert_eq!(d.contracts, 2);
                assert_eq!(d.premium_per_contract, 5.0);
                assert_eq!(d.total_premium, 1000.0);
            }
            _ => panic!("Expected Sized"),
        }
    }

    #[test]
    fn test_premium_cap_binding() {
        let sizer = PositionSizer::new(SizingConfig {
            max_premium_per_position: 0.005, // 0.5% = $500
            ..Default::default()
        });

        // equity=100k, ask=5.0 → max contracts by premium = floor(500 / 500) = 1
        // Formula would give 2, but premium cap restricts to 1
        let result = sizer.size(100_000.0, 10.0, 0.45, 5.0, 0.0);

        match result {
            SizingResult::Sized(d) => {
                assert_eq!(d.contracts, 1);
                assert!(d.total_premium <= 500.0);
                assert_eq!(d.binding_cap, Some(CapType::MaxPremiumPerPosition));
            }
            _ => panic!("Expected Sized"),
        }
    }

    #[test]
    fn test_hard_cap_binding() {
        let sizer = PositionSizer::new(SizingConfig {
            max_contracts_per_position: 3,
            ..Default::default()
        });

        // Formula would give more, but hard cap = 3
        let result = sizer.size(1_000_000.0, 1.0, 0.45, 1.0, 0.0);

        match result {
            SizingResult::Sized(d) => {
                assert_eq!(d.contracts, 3);
                assert_eq!(d.binding_cap, Some(CapType::MaxContractsPerPosition));
            }
            _ => panic!("Expected Sized"),
        }
    }

    #[test]
    fn test_portfolio_cap_skip() {
        let sizer = PositionSizer::new(SizingConfig {
            max_portfolio_premium: 0.02, // 2% = $2000
            ..Default::default()
        });

        // Already at portfolio cap
        let result = sizer.size(100_000.0, 10.0, 0.45, 5.0, 2000.0);

        assert!(matches!(
            result,
            SizingResult::Skipped(SizingSkipReason::PortfolioCapReached { .. })
        ));
    }

    #[test]
    fn test_zero_contracts_skip() {
        let sizer = PositionSizer::new(SizingConfig::default());

        // stop_distance = 0 → formula gives 0
        let result = sizer.size(100_000.0, 0.0, 0.45, 5.0, 0.0);

        assert!(matches!(
            result,
            SizingResult::Skipped(SizingSkipReason::ZeroContracts { .. })
        ));
    }

    #[test]
    fn test_portfolio_cap_reduces_contracts() {
        let sizer = PositionSizer::new(SizingConfig {
            max_portfolio_premium: 0.015, // 1.5% = $1500
            ..Default::default()
        });

        // Already deployed $1000, remaining budget = $500
        // ask=5.0 → floor(500 / 500) = 1 contract
        let result = sizer.size(100_000.0, 10.0, 0.45, 5.0, 1000.0);

        match result {
            SizingResult::Sized(d) => {
                assert_eq!(d.contracts, 1);
                assert_eq!(d.binding_cap, Some(CapType::MaxPortfolioPremium));
            }
            _ => panic!("Expected Sized"),
        }
    }
}
