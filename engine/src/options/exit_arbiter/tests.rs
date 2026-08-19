//! ExitArbiter tests — priority table verification
//!
//! The ExitArbiter is the single owner of exit decisions. All sources emit
//! ExitSignal{priority, source}. The arbiter selects the highest-priority
//! winner per position, serializes decisions (one active exit per position).

use super::*;
use chrono::{Duration, Utc};

#[test]
fn test_force_close_beats_everything() {
    // Simultaneous: force-close + circuit breaker + DTE override + trailing stop
    // Expected: force-close wins (priority 1)
    let signals = vec![
        ExitSignal {
            source: ExitSource::TrailingStop,
            priority: 4,
            reason: "Trailing stop hit".to_string(),
            timestamp: Utc::now(),
        },
        ExitSignal {
            source: ExitSource::DteOverride,
            priority: 3,
            reason: "DTE < 7".to_string(),
            timestamp: Utc::now(),
        },
        ExitSignal {
            source: ExitSource::CircuitBreaker,
            priority: 2,
            reason: "Stage 3 timeout".to_string(),
            timestamp: Utc::now(),
        },
        ExitSignal {
            source: ExitSource::OperatorForceClose,
            priority: 1,
            reason: "Manual close".to_string(),
            timestamp: Utc::now(),
        },
    ];

    let arbiter = ExitArbiter::new();
    let winner = arbiter.select_winner(&signals);

    assert!(winner.is_some());
    let winner = winner.unwrap();
    assert_eq!(winner.source, ExitSource::OperatorForceClose);
    assert_eq!(winner.priority, 1);
}

#[test]
fn test_circuit_breaker_beats_overrides() {
    // Simultaneous: circuit breaker + DTE override + ROI table
    // Expected: circuit breaker wins (priority 2)
    let signals = vec![
        ExitSignal {
            source: ExitSource::RoiTable,
            priority: 5,
            reason: "ROI target hit".to_string(),
            timestamp: Utc::now(),
        },
        ExitSignal {
            source: ExitSource::DteOverride,
            priority: 3,
            reason: "DTE < 7".to_string(),
            timestamp: Utc::now(),
        },
        ExitSignal {
            source: ExitSource::CircuitBreaker,
            priority: 2,
            reason: "Stage 3 timeout".to_string(),
            timestamp: Utc::now(),
        },
    ];

    let arbiter = ExitArbiter::new();
    let winner = arbiter.select_winner(&signals);

    assert!(winner.is_some());
    let winner = winner.unwrap();
    assert_eq!(winner.source, ExitSource::CircuitBreaker);
    assert_eq!(winner.priority, 2);
}

#[test]
fn test_dte_override_beats_trailing_stop() {
    // Simultaneous: DTE override + trailing stop + signal reversal
    // Expected: DTE override wins (priority 3)
    let signals = vec![
        ExitSignal {
            source: ExitSource::SignalReversal,
            priority: 6,
            reason: "Signal flipped bearish".to_string(),
            timestamp: Utc::now(),
        },
        ExitSignal {
            source: ExitSource::TrailingStop,
            priority: 4,
            reason: "Trailing stop hit".to_string(),
            timestamp: Utc::now(),
        },
        ExitSignal {
            source: ExitSource::DteOverride,
            priority: 3,
            reason: "DTE < 7".to_string(),
            timestamp: Utc::now(),
        },
    ];

    let arbiter = ExitArbiter::new();
    let winner = arbiter.select_winner(&signals);

    assert!(winner.is_some());
    let winner = winner.unwrap();
    assert_eq!(winner.source, ExitSource::DteOverride);
    assert_eq!(winner.priority, 3);
}

#[test]
fn test_trailing_stop_beats_roi() {
    // Simultaneous: trailing stop + ROI table + signal reversal
    // Expected: trailing stop wins (priority 4)
    let signals = vec![
        ExitSignal {
            source: ExitSource::SignalReversal,
            priority: 6,
            reason: "Signal flipped".to_string(),
            timestamp: Utc::now(),
        },
        ExitSignal {
            source: ExitSource::RoiTable,
            priority: 5,
            reason: "ROI target".to_string(),
            timestamp: Utc::now(),
        },
        ExitSignal {
            source: ExitSource::TrailingStop,
            priority: 4,
            reason: "Trailing stop".to_string(),
            timestamp: Utc::now(),
        },
    ];

    let arbiter = ExitArbiter::new();
    let winner = arbiter.select_winner(&signals);

    assert!(winner.is_some());
    let winner = winner.unwrap();
    assert_eq!(winner.source, ExitSource::TrailingStop);
    assert_eq!(winner.priority, 4);
}

#[test]
fn test_roi_beats_signal_reversal() {
    // Simultaneous: ROI table + signal reversal
    // Expected: ROI wins (priority 5)
    let signals = vec![
        ExitSignal {
            source: ExitSource::SignalReversal,
            priority: 6,
            reason: "Signal flipped".to_string(),
            timestamp: Utc::now(),
        },
        ExitSignal {
            source: ExitSource::RoiTable,
            priority: 5,
            reason: "ROI target".to_string(),
            timestamp: Utc::now(),
        },
    ];

    let arbiter = ExitArbiter::new();
    let winner = arbiter.select_winner(&signals);

    assert!(winner.is_some());
    let winner = winner.unwrap();
    assert_eq!(winner.source, ExitSource::RoiTable);
    assert_eq!(winner.priority, 5);
}

#[test]
fn test_empty_signals_returns_none() {
    let signals: Vec<ExitSignal> = vec![];
    let arbiter = ExitArbiter::new();
    let winner = arbiter.select_winner(&signals);
    assert!(winner.is_none());
}

#[test]
fn test_single_signal_wins() {
    let signals = vec![ExitSignal {
        source: ExitSource::SignalReversal,
        priority: 6,
        reason: "Only signal".to_string(),
        timestamp: Utc::now(),
    }];

    let arbiter = ExitArbiter::new();
    let winner = arbiter.select_winner(&signals);

    assert!(winner.is_some());
    let winner = winner.unwrap();
    assert_eq!(winner.source, ExitSource::SignalReversal);
    assert_eq!(winner.priority, 6);
}

#[test]
fn test_priority_table_is_complete() {
    // Verify all 6 sources are defined with correct priorities
    assert_eq!(ExitSource::OperatorForceClose as u8, 1);
    assert_eq!(ExitSource::CircuitBreaker as u8, 2);
    assert_eq!(ExitSource::DteOverride as u8, 3);
    assert_eq!(ExitSource::TrailingStop as u8, 4);
    assert_eq!(ExitSource::RoiTable as u8, 5);
    assert_eq!(ExitSource::SignalReversal as u8, 6);
}
