//! Macro risk gate for options entries
//!
//! Blocks new entries based on:
//! - VIX level (configurable threshold, e.g., > 30)
//! - VIX 5d slope (configurable threshold, e.g., > 0.5/day)
//! - Calendar blackout (24h before FOMC/CPI/NFP)
//!
//! Output: `ENTRY_ALLOWED` or `ENTRY_DENIED(reason)` — auditable in UI.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Macro gate decision
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MacroGateDecision {
    Allowed,
    Denied(MacroGateDenialReason),
}

/// Reason for denying entry
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MacroGateDenialReason {
    VixLevelTooHigh { current: f64, threshold: f64 },
    VixSlopeTooSteep { slope_per_day: f64, threshold: f64 },
    CalendarBlackout { event: String, hours_until: f64 },
}

/// Macro gate configuration
#[derive(Debug, Clone)]
pub struct MacroGateConfig {
    /// VIX level threshold (e.g., 30.0)
    pub vix_level_threshold: f64,
    /// VIX 5-day slope threshold (per day, e.g., 0.5)
    pub vix_slope_threshold: f64,
    /// Calendar blackout window (hours before event, e.g., 24.0)
    pub blackout_hours: f64,
}

impl Default for MacroGateConfig {
    fn default() -> Self {
        Self {
            vix_level_threshold: 30.0,
            vix_slope_threshold: 0.5,
            blackout_hours: 24.0,
        }
    }
}

/// Calendar event (FOMC, CPI, NFP)
#[derive(Debug, Clone)]
pub struct CalendarEvent {
    pub name: String,
    pub datetime: DateTime<Utc>,
}

/// Macro risk gate
pub struct MacroGate {
    config: MacroGateConfig,
}

impl MacroGate {
    pub fn new(config: MacroGateConfig) -> Self {
        Self { config }
    }

    /// Evaluate macro gate for entry decision
    ///
    /// # Arguments
    /// * `current_vix` - Current VIX level
    /// * `vix_5d_ago` - VIX level 5 days ago (for slope calculation)
    /// * `now` - Current timestamp
    /// * `calendar_events` - Upcoming calendar events (FOMC/CPI/NFP)
    ///
    /// # Returns
    /// `ENTRY_ALLOWED` or `ENTRY_DENIED(reason)`
    pub fn evaluate(
        &self,
        current_vix: f64,
        vix_5d_ago: f64,
        now: DateTime<Utc>,
        calendar_events: &[CalendarEvent],
    ) -> MacroGateDecision {
        // Check VIX level
        if current_vix > self.config.vix_level_threshold {
            return MacroGateDecision::Denied(MacroGateDenialReason::VixLevelTooHigh {
                current: current_vix,
                threshold: self.config.vix_level_threshold,
            });
        }

        // Check VIX 5-day slope
        let slope_per_day = (current_vix - vix_5d_ago) / 5.0;
        if slope_per_day > self.config.vix_slope_threshold {
            return MacroGateDecision::Denied(MacroGateDenialReason::VixSlopeTooSteep {
                slope_per_day,
                threshold: self.config.vix_slope_threshold,
            });
        }

        // Check calendar blackout
        for event in calendar_events {
            let hours_until = (event.datetime - now).num_hours() as f64;
            if hours_until > 0.0 && hours_until <= self.config.blackout_hours {
                return MacroGateDecision::Denied(MacroGateDenialReason::CalendarBlackout {
                    event: event.name.clone(),
                    hours_until,
                });
            }
        }

        MacroGateDecision::Allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vix_level_denial() {
        let gate = MacroGate::new(MacroGateConfig {
            vix_level_threshold: 30.0,
            vix_slope_threshold: 0.5,
            blackout_hours: 24.0,
        });

        let now = Utc::now();
        let decision = gate.evaluate(35.0, 20.0, now, &[]);

        assert!(matches!(
            decision,
            MacroGateDecision::Denied(MacroGateDenialReason::VixLevelTooHigh { .. })
        ));
    }

    #[test]
    fn test_vix_slope_denial() {
        let gate = MacroGate::new(MacroGateConfig {
            vix_level_threshold: 30.0,
            vix_slope_threshold: 0.5,
            blackout_hours: 24.0,
        });

        let now = Utc::now();
        // VIX went from 20 to 25 in 5 days = 1.0/day slope
        let decision = gate.evaluate(25.0, 20.0, now, &[]);

        assert!(matches!(
            decision,
            MacroGateDecision::Denied(MacroGateDenialReason::VixSlopeTooSteep { .. })
        ));
    }

    #[test]
    fn test_calendar_blackout_denial() {
        let gate = MacroGate::new(MacroGateConfig {
            vix_level_threshold: 30.0,
            vix_slope_threshold: 0.5,
            blackout_hours: 24.0,
        });

        let now = Utc::now();
        let event = CalendarEvent {
            name: "FOMC".to_string(),
            datetime: now + Duration::hours(12),
        };

        let decision = gate.evaluate(20.0, 20.0, now, &[event]);

        assert!(matches!(
            decision,
            MacroGateDecision::Denied(MacroGateDenialReason::CalendarBlackout { .. })
        ));
    }

    #[test]
    fn test_allowed_when_all_clear() {
        let gate = MacroGate::new(MacroGateConfig {
            vix_level_threshold: 30.0,
            vix_slope_threshold: 0.5,
            blackout_hours: 24.0,
        });

        let now = Utc::now();
        let decision = gate.evaluate(20.0, 20.0, now, &[]);

        assert_eq!(decision, MacroGateDecision::Allowed);
    }

    #[test]
    fn test_calendar_event_outside_blackout_window() {
        let gate = MacroGate::new(MacroGateConfig {
            vix_level_threshold: 30.0,
            vix_slope_threshold: 0.5,
            blackout_hours: 24.0,
        });

        let now = Utc::now();
        let event = CalendarEvent {
            name: "FOMC".to_string(),
            datetime: now + Duration::hours(48), // Outside 24h window
        };

        let decision = gate.evaluate(20.0, 20.0, now, &[event]);

        assert_eq!(decision, MacroGateDecision::Allowed);
    }
}
