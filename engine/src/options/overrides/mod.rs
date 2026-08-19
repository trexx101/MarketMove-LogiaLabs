//! Hardcoded overrides — non-optimizable risk layer (rail tier)
//!
//! DTE floor, delta-drift band, earnings blackout.
//! Thresholds are DB-configurable via the options config store (rail tier:
//! bounded, changeable but never disabled); see `OverridesConfig`.

use crate::options::exit_arbiter::{ExitSignal, ExitSource};
use chrono::{DateTime, NaiveDate, Utc};

#[cfg(test)]
mod tests;

/// Config for the hardcoded override rail. Defaults match the original
/// hard-coded values (DTE < 7, delta outside [0.15, 0.70], earnings ≤ 2d).
#[derive(Debug, Clone)]
pub struct OverridesConfig {
    pub dte_exit_min: i64,
    pub delta_drift_min: f64,
    pub delta_drift_max: f64,
    pub earnings_blackout_days: i64,
}

impl Default for OverridesConfig {
    fn default() -> Self {
        Self {
            dte_exit_min: 7,
            delta_drift_min: 0.15,
            delta_drift_max: 0.70,
            earnings_blackout_days: 2,
        }
    }
}

/// Hardcoded risk overrides
pub struct HardcodedOverrides {
    config: OverridesConfig,
}

impl HardcodedOverrides {
    pub fn new() -> Self {
        Self::with_config(OverridesConfig::default())
    }

    pub fn with_config(config: OverridesConfig) -> Self {
        Self { config }
    }

    /// Check DTE override: fires when DTE < dte_exit_min
    pub fn check_dte(&self, dte: i64) -> Option<ExitSignal> {
        let floor = self.config.dte_exit_min;
        if dte < floor {
            Some(ExitSignal {
                source: ExitSource::DteOverride,
                priority: 3,
                reason: format!("DTE {} < {}", dte, floor),
                timestamp: Utc::now(),
            })
        } else {
            None
        }
    }

    /// Check delta drift override: fires when delta outside [min, max]
    pub fn check_delta_drift(&self, delta: f64) -> Option<ExitSignal> {
        let lo = self.config.delta_drift_min;
        let hi = self.config.delta_drift_max;
        if delta < lo || delta > hi {
            Some(ExitSignal {
                source: ExitSource::DteOverride,
                priority: 3,
                reason: format!("Delta {:.3} outside [{:.2}, {:.2}]", delta, lo, hi),
                timestamp: Utc::now(),
            })
        } else {
            None
        }
    }

    /// Check earnings blackout: fires when within earnings_blackout_days of earnings
    pub fn check_earnings_blackout(
        &self,
        earnings_date: NaiveDate,
        today: NaiveDate,
    ) -> Option<ExitSignal> {
        let days_until = (earnings_date - today).num_days();

        // Fire if earnings within 0..=blackout days (inclusive)
        if days_until >= 0 && days_until <= self.config.earnings_blackout_days {
            Some(ExitSignal {
                source: ExitSource::DteOverride,
                priority: 3,
                reason: format!("Earnings in {} days", days_until),
                timestamp: Utc::now(),
            })
        } else {
            None
        }
    }
}
