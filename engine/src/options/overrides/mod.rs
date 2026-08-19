//! Hardcoded overrides — non-optimizable risk layer
//!
//! DTE < 7, delta-drift band 0.15-0.70, earnings blackout 2 days.
//! In code, not config-tunable.

use crate::options::exit_arbiter::{ExitSignal, ExitSource};
use chrono::{DateTime, NaiveDate, Utc};

#[cfg(test)]
mod tests;

/// Hardcoded risk overrides
pub struct HardcodedOverrides;

impl HardcodedOverrides {
    pub fn new() -> Self {
        Self
    }

    /// Check DTE override: fires when DTE < 7
    pub fn check_dte(&self, dte: i64) -> Option<ExitSignal> {
        if dte < 7 {
            Some(ExitSignal {
                source: ExitSource::DteOverride,
                priority: 3,
                reason: format!("DTE {} < 7", dte),
                timestamp: Utc::now(),
            })
        } else {
            None
        }
    }

    /// Check delta drift override: fires when delta outside [0.15, 0.70]
    pub fn check_delta_drift(&self, delta: f64) -> Option<ExitSignal> {
        if delta < 0.15 || delta > 0.70 {
            Some(ExitSignal {
                source: ExitSource::DteOverride,
                priority: 3,
                reason: format!("Delta {:.3} outside [0.15, 0.70]", delta),
                timestamp: Utc::now(),
            })
        } else {
            None
        }
    }

    /// Check earnings blackout: fires when within 2 days of earnings
    pub fn check_earnings_blackout(
        &self,
        earnings_date: NaiveDate,
        today: NaiveDate,
    ) -> Option<ExitSignal> {
        let days_until = (earnings_date - today).num_days();
        
        // Fire if earnings within 0-2 days (inclusive)
        if days_until >= 0 && days_until <= 2 {
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
