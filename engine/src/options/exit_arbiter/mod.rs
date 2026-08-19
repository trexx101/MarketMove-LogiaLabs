//! ExitArbiter — single owner of exit decisions
//!
//! All exit sources emit ExitSignal{priority, source}. The arbiter selects
//! the highest-priority winner per position, serializes decisions.

use chrono::{DateTime, Utc};

#[cfg(test)]
mod tests;

/// Exit source with fixed priority (D14)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitSource {
    OperatorForceClose = 1,
    CircuitBreaker = 2,
    DteOverride = 3,
    TrailingStop = 4,
    RoiTable = 5,
    SignalReversal = 6,
}

/// Exit signal emitted by all exit sources
#[derive(Debug, Clone)]
pub struct ExitSignal {
    pub source: ExitSource,
    pub priority: u8,
    pub reason: String,
    pub timestamp: DateTime<Utc>,
}

/// ExitArbiter selects the highest-priority exit signal
pub struct ExitArbiter;

impl ExitArbiter {
    pub fn new() -> Self {
        Self
    }

    /// Select the highest-priority winner from a set of signals
    /// Returns None if no signals present
    pub fn select_winner(&self, signals: &[ExitSignal]) -> Option<ExitSignal> {
        signals.iter().min_by_key(|s| s.priority).cloned()
    }
}
