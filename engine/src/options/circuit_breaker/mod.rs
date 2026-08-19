//! Circuit breaker for options trading
//!
//! Triggers when:
//! - Stage 3 timeout (10s) with unfilled orders
//! - Consecutive losses exceed threshold
//! - Abnormal volatility detected
//!
//! When triggered:
//! - Cancels all pending orders
//! - Halts new entries
//! - Emits ExitSignal with priority 2

use chrono::{DateTime, Duration, Utc};

use crate::options::exit_arbiter::{ExitSignal, ExitSource};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitBreakerTrigger {
    Stage3Timeout,
    ConsecutiveLosses,
    AbnormalVolatility,
}

#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    triggered: bool,
    trigger_reason: Option<CircuitBreakerTrigger>,
    trigger_time: Option<DateTime<Utc>>,
    halt_duration: Duration,
    consecutive_losses: u32,
    max_consecutive_losses: u32,
}

impl CircuitBreaker {
    pub fn new(halt_duration_secs: i64, max_consecutive_losses: u32) -> Self {
        Self {
            triggered: false,
            trigger_reason: None,
            trigger_time: None,
            halt_duration: Duration::seconds(halt_duration_secs),
            consecutive_losses: 0,
            max_consecutive_losses,
        }
    }

    pub fn is_triggered(&self) -> bool {
        self.triggered
    }

    pub fn trigger(&mut self, reason: CircuitBreakerTrigger) -> ExitSignal {
        self.triggered = true;
        self.trigger_reason = Some(reason);
        self.trigger_time = Some(Utc::now());

        ExitSignal {
            priority: 2,
            source: ExitSource::CircuitBreaker,
            reason: format!("Circuit breaker triggered: {:?}", reason),
            timestamp: Utc::now(),
        }
    }

    pub fn record_loss(&mut self) -> Option<ExitSignal> {
        self.consecutive_losses += 1;
        if self.consecutive_losses >= self.max_consecutive_losses {
            Some(self.trigger(CircuitBreakerTrigger::ConsecutiveLosses))
        } else {
            None
        }
    }

    pub fn record_win(&mut self) {
        self.consecutive_losses = 0;
    }

    pub fn check_volatility(&mut self, current_iv: f64, baseline_iv: f64) -> Option<ExitSignal> {
        // Trigger if IV is 2x baseline (abnormal volatility)
        if current_iv > baseline_iv * 2.0 {
            Some(self.trigger(CircuitBreakerTrigger::AbnormalVolatility))
        } else {
            None
        }
    }

    pub fn can_resume(&self) -> bool {
        if !self.triggered {
            return true;
        }

        if let Some(trigger_time) = self.trigger_time {
            let elapsed = Utc::now() - trigger_time;
            elapsed >= self.halt_duration
        } else {
            false
        }
    }

    pub fn reset(&mut self) {
        self.triggered = false;
        self.trigger_reason = None;
        self.trigger_time = None;
        self.consecutive_losses = 0;
    }
}
