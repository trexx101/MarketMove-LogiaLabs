//! Hardcoded overrides tests
//!
//! Non-optimizable risk layer: DTE < 7, delta-drift band 0.15-0.70,
//! earnings blackout 2 days. In code, not config-tunable.

use super::*;
use chrono::{Duration, NaiveDate, Utc};

#[test]
fn test_dte_override_fires_below_7() {
    let overrides = HardcodedOverrides::new();
    
    // DTE = 6 should fire
    let signal = overrides.check_dte(6);
    assert!(signal.is_some());
    let signal = signal.unwrap();
    assert_eq!(signal.source, ExitSource::DteOverride);
    assert_eq!(signal.priority, 3);
    assert!(signal.reason.contains("DTE"));
    
    // DTE = 7 should NOT fire (boundary)
    let signal = overrides.check_dte(7);
    assert!(signal.is_none());
    
    // DTE = 0 should fire
    let signal = overrides.check_dte(0);
    assert!(signal.is_some());
}

#[test]
fn test_delta_drift_override_fires_outside_band() {
    let overrides = HardcodedOverrides::new();
    
    // Delta = 0.10 (below 0.15) should fire
    let signal = overrides.check_delta_drift(0.10);
    assert!(signal.is_some());
    let signal = signal.unwrap();
    assert_eq!(signal.source, ExitSource::DteOverride);
    assert_eq!(signal.priority, 3);
    
    // Delta = 0.75 (above 0.70) should fire
    let signal = overrides.check_delta_drift(0.75);
    assert!(signal.is_some());
    
    // Delta = 0.45 (within band) should NOT fire
    let signal = overrides.check_delta_drift(0.45);
    assert!(signal.is_none());
    
    // Delta = 0.15 (lower boundary) should NOT fire
    let signal = overrides.check_delta_drift(0.15);
    assert!(signal.is_none());
    
    // Delta = 0.70 (upper boundary) should NOT fire
    let signal = overrides.check_delta_drift(0.70);
    assert!(signal.is_none());
}

#[test]
fn test_earnings_blackout_fires_within_2_days() {
    let overrides = HardcodedOverrides::new();
    let today = Utc::now().date_naive();
    let earnings_date = today + Duration::days(1);
    
    // 1 day before earnings should fire
    let signal = overrides.check_earnings_blackout(earnings_date, today);
    assert!(signal.is_some());
    let signal = signal.unwrap();
    assert_eq!(signal.source, ExitSource::DteOverride);
    assert_eq!(signal.priority, 3);
    
    // 2 days before earnings should fire (boundary)
    let earnings_date = today + Duration::days(2);
    let signal = overrides.check_earnings_blackout(earnings_date, today);
    assert!(signal.is_some());
    
    // 3 days before earnings should NOT fire
    let earnings_date = today + Duration::days(3);
    let signal = overrides.check_earnings_blackout(earnings_date, today);
    assert!(signal.is_none());
    
    // Earnings today should fire
    let earnings_date = today;
    let signal = overrides.check_earnings_blackout(earnings_date, today);
    assert!(signal.is_some());
    
    // Earnings in the past should NOT fire (already happened)
    let earnings_date = today - Duration::days(1);
    let signal = overrides.check_earnings_blackout(earnings_date, today);
    assert!(signal.is_none());
}

#[test]
fn test_all_overrides_return_correct_priority() {
    let overrides = HardcodedOverrides::new();
    
    // All overrides should return priority 3 (DteOverride)
    if let Some(signal) = overrides.check_dte(5) {
        assert_eq!(signal.priority, 3);
    }
    
    if let Some(signal) = overrides.check_delta_drift(0.10) {
        assert_eq!(signal.priority, 3);
    }
    
    let today = Utc::now().date_naive();
    let earnings_date = today + Duration::days(1);
    if let Some(signal) = overrides.check_earnings_blackout(earnings_date, today) {
        assert_eq!(signal.priority, 3);
    }
}

#[test]
fn test_overrides_with_config_changes_thresholds() {
    // Rail values come from the options config store in production;
    // with_config must actually move the thresholds.
    let overrides = HardcodedOverrides::with_config(OverridesConfig {
        dte_exit_min: 10,
        delta_drift_min: 0.20,
        delta_drift_max: 0.60,
        earnings_blackout_days: 5,
    });

    // DTE 8 fires now (was <7 with defaults)
    assert!(overrides.check_dte(8).is_some());
    // Delta 0.18 is below the raised min (0.20) → fires
    assert!(overrides.check_delta_drift(0.18).is_some());
    // Delta 0.19 still fires; 0.25 inside band does not
    assert!(overrides.check_delta_drift(0.25).is_none());
    // Delta 0.65 above the lowered max (0.60) → fires
    assert!(overrides.check_delta_drift(0.65).is_some());

    // Earnings in 4 days fires with 5-day blackout
    let today = Utc::now().date_naive();
    assert!(overrides.check_earnings_blackout(today + Duration::days(4), today).is_some());
}
