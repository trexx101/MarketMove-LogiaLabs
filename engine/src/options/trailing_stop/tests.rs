//! Trailing stop with hysteresis tests
//!
//! Re-arm requires recovery band (0.5 × ATR above stop) to prevent
//! whipsaw churn through spreads.

use super::*;

#[test]
fn test_trailing_stop_initialization_call() {
    let stop = TrailingStop::new(100.0, 2.0, true); // Call option
    
    assert_eq!(stop.high_water_mark, 100.0);
    assert_eq!(stop.atr, 2.0);
    assert!(stop.armed);
    assert_eq!(stop.recovery_threshold, 1.0); // 0.5 × ATR
    assert_eq!(stop.stop_price, 95.0); // 100 × 0.95
}

#[test]
fn test_trailing_stop_initialization_put() {
    let stop = TrailingStop::new(100.0, 2.0, false); // Put option
    
    assert_eq!(stop.high_water_mark, 100.0);
    assert_eq!(stop.stop_price, 105.0); // 100 × 1.05
}

#[test]
fn test_trailing_stop_trails_up_call() {
    let mut stop = TrailingStop::new(100.0, 2.0, true);
    
    // Price moves up to 110
    let signal = stop.update(110.0, true);
    assert!(signal.is_none()); // No trigger yet
    
    // Stop should have trailed up
    assert_eq!(stop.high_water_mark, 110.0);
    assert_eq!(stop.stop_price, 104.5); // 110 × 0.95
}

#[test]
fn test_trailing_stop_trails_down_put() {
    let mut stop = TrailingStop::new(100.0, 2.0, false);
    
    // Price moves down to 90
    let signal = stop.update(90.0, false);
    assert!(signal.is_none()); // No trigger yet
    
    // Stop should have trailed down
    assert_eq!(stop.high_water_mark, 90.0);
    assert_eq!(stop.stop_price, 94.5); // 90 × 1.05
}

#[test]
fn test_trailing_stop_triggers_call() {
    let mut stop = TrailingStop::new(100.0, 2.0, true);
    
    // Price drops below stop
    let signal = stop.update(94.0, true);
    
    assert!(signal.is_some());
    let sig = signal.unwrap();
    assert_eq!(sig.priority, 4); // TrailingStop priority
    assert!(sig.reason.contains("Trailing stop triggered"));
    assert!(!stop.armed); // Disarmed after trigger
}

#[test]
fn test_trailing_stop_triggers_put() {
    let mut stop = TrailingStop::new(100.0, 2.0, false);
    
    // Price rises above stop
    let signal = stop.update(106.0, false);
    
    assert!(signal.is_some());
    let sig = signal.unwrap();
    assert_eq!(sig.priority, 4);
    assert!(!stop.armed);
}

#[test]
fn test_trailing_stop_hysteresis_no_rearm() {
    let mut stop = TrailingStop::new(100.0, 2.0, true);
    
    // Trigger the stop
    let _ = stop.update(94.0, true);
    assert!(!stop.armed);
    
    // Price recovers but not enough (needs 1.0 recovery = 0.5 × ATR)
    let signal = stop.update(95.5, true); // Only 1.5 above stop of 94.0
    
    // Should not re-arm yet (needs to reach 95.0 + 1.0 = 96.0)
    assert!(!stop.armed);
    assert!(signal.is_none());
}

#[test]
fn test_trailing_stop_hysteresis_rearms() {
    let mut stop = TrailingStop::new(100.0, 2.0, true);
    
    // Trigger the stop
    let _ = stop.update(94.0, true);
    assert!(!stop.armed);
    let stop_price_after_trigger = stop.stop_price;
    
    // Price recovers enough (needs stop_price + recovery_threshold)
    let recovery_price = stop_price_after_trigger + stop.recovery_threshold + 0.1;
    let signal = stop.update(recovery_price, true);
    
    // Should re-arm
    assert!(stop.armed);
    assert_eq!(stop.high_water_mark, recovery_price); // Reset to current price
    assert!(signal.is_none()); // No trigger during recovery
}

#[test]
fn test_trailing_stop_no_trail_when_disarmed() {
    let mut stop = TrailingStop::new(100.0, 2.0, true);
    
    // Trigger the stop
    let _ = stop.update(94.0, true);
    assert!(!stop.armed);
    
    // Price moves up but not enough to re-arm
    let _ = stop.update(95.0, true);
    
    // High water mark should not have changed
    assert_eq!(stop.high_water_mark, 100.0); // Original value, not updated
}

#[test]
fn test_trailing_stop_boundary_call() {
    let mut stop = TrailingStop::new(100.0, 2.0, true);
    
    // Price exactly at stop
    let signal = stop.update(95.0, true);
    
    // Should trigger (current_price <= stop_price)
    assert!(signal.is_some());
}

#[test]
fn test_trailing_stop_boundary_put() {
    let mut stop = TrailingStop::new(100.0, 2.0, false);
    
    // Price exactly at stop
    let signal = stop.update(105.0, false);
    
    // Should trigger (current_price >= stop_price)
    assert!(signal.is_some());
}

#[test]
fn test_trailing_stop_with_config_changes_trail_and_band() {
    // 10% trail instead of default 5%
    let mut stop = TrailingStop::with_config(
        100.0,
        2.0,
        true,
        TrailingStopConfig { trail_pct: 0.10, rearm_band_atr: 1.0 },
    );
    // Stop should be at 90.0 (10% trail), not 95.0
    assert!((stop.stop_price - 90.0).abs() < 1e-9);
    // Recovery threshold = 1.0 × ATR, not 0.5
    assert!((stop.recovery_threshold - 2.0).abs() < 1e-9);

    // New high trails with the configured pct
    stop.update(110.0, true);
    assert!((stop.stop_price - 99.0).abs() < 1e-9);
}
