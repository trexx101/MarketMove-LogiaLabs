//! Tests for circuit breaker

use super::*;

#[test]
fn test_circuit_breaker_initial_state() {
    let cb = CircuitBreaker::new(60, 3);
    assert!(!cb.is_triggered());
    assert!(cb.can_resume());
}

#[test]
fn test_circuit_breaker_trigger_stage3_timeout() {
    let mut cb = CircuitBreaker::new(60, 3);
    let signal = cb.trigger(CircuitBreakerTrigger::Stage3Timeout);
    
    assert!(cb.is_triggered());
    assert_eq!(signal.priority, 2);
    assert_eq!(signal.source, ExitSource::CircuitBreaker);
    assert!(signal.reason.contains("Stage3Timeout"));
}

#[test]
fn test_circuit_breaker_consecutive_losses() {
    let mut cb = CircuitBreaker::new(60, 3);
    
    // First two losses don't trigger
    assert!(cb.record_loss().is_none());
    assert!(cb.record_loss().is_none());
    assert!(!cb.is_triggered());
    
    // Third loss triggers
    let signal = cb.record_loss();
    assert!(signal.is_some());
    assert!(cb.is_triggered());
}

#[test]
fn test_circuit_breaker_win_resets_losses() {
    let mut cb = CircuitBreaker::new(60, 3);
    
    cb.record_loss();
    cb.record_loss();
    cb.record_win(); // Reset counter
    
    // Now need 3 more losses to trigger
    assert!(cb.record_loss().is_none());
    assert!(cb.record_loss().is_none());
    assert!(cb.record_loss().is_some()); // Third loss triggers
    assert!(cb.is_triggered());
}

#[test]
fn test_circuit_breaker_volatility_trigger() {
    let mut cb = CircuitBreaker::new(60, 3);
    
    // Normal volatility - no trigger
    assert!(cb.check_volatility(0.25, 0.20).is_none());
    assert!(!cb.is_triggered());
    
    // 2x volatility - trigger
    let signal = cb.check_volatility(0.45, 0.20);
    assert!(signal.is_some());
    assert!(cb.is_triggered());
}

#[test]
fn test_circuit_breaker_halt_duration() {
    let mut cb = CircuitBreaker::new(5, 3); // 5 second halt
    
    cb.trigger(CircuitBreakerTrigger::Stage3Timeout);
    assert!(cb.is_triggered());
    assert!(!cb.can_resume());
    
    // Wait for halt duration to pass
    std::thread::sleep(std::time::Duration::from_secs(6));
    assert!(cb.can_resume());
}

#[test]
fn test_circuit_breaker_reset() {
    let mut cb = CircuitBreaker::new(60, 3);
    
    cb.trigger(CircuitBreakerTrigger::Stage3Timeout);
    assert!(cb.is_triggered());
    
    cb.reset();
    assert!(!cb.is_triggered());
    assert!(cb.can_resume());
}
