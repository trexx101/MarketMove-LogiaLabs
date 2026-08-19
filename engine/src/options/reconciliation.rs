//! Reconciliation logic for comparing expected vs actual state
//!
//! Used to detect mismatches between our internal state and broker state.

/// Represents a mismatch between expected and actual state
#[derive(Debug, Clone, PartialEq)]
pub enum Mismatch {
    /// Order exists in expected state but not in actual
    MissingOrder(String),
    /// Order exists in actual state but not in expected
    OrphanedOrder(String),
    /// Order status differs between expected and actual
    StatusMismatch {
        order_id: String,
        expected: String,
        actual: String,
    },
    /// Position quantity differs
    PositionMismatch {
        symbol: String,
        expected: f64,
        actual: f64,
    },
}

/// Result of reconciliation
#[derive(Debug, Clone)]
pub struct ReconciliationResult {
    pub mismatches: Vec<Mismatch>,
    pub is_clean: bool,
}

impl ReconciliationResult {
    pub fn new() -> Self {
        Self {
            mismatches: Vec::new(),
            is_clean: true,
        }
    }

    pub fn add_mismatch(&mut self, mismatch: Mismatch) {
        self.is_clean = false;
        self.mismatches.push(mismatch);
    }
}

impl Default for ReconciliationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple order representation for reconciliation
#[derive(Debug, Clone, PartialEq)]
pub struct OrderState {
    pub id: String,
    pub status: String,
}

/// Simple position representation for reconciliation
#[derive(Debug, Clone, PartialEq)]
pub struct PositionState {
    pub symbol: String,
    pub quantity: f64,
}

/// Reconcile expected positions against actual broker state
pub fn reconcile_positions(
    expected_positions: &[PositionState],
    actual_positions: &[PositionState],
) -> ReconciliationResult {
    let mut result = ReconciliationResult::new();

    // Check for missing or mismatched positions
    for expected in expected_positions {
        match actual_positions.iter().find(|a| a.symbol == expected.symbol) {
            None => {
                // Position exists in expected but not in actual
                result.add_mismatch(Mismatch::PositionMismatch {
                    symbol: expected.symbol.clone(),
                    expected: expected.quantity,
                    actual: 0.0,
                });
            }
            Some(actual) => {
                if (expected.quantity - actual.quantity).abs() > 1e-6 {
                    result.add_mismatch(Mismatch::PositionMismatch {
                        symbol: expected.symbol.clone(),
                        expected: expected.quantity,
                        actual: actual.quantity,
                    });
                }
            }
        }
    }

    // Check for orphaned positions (exist in actual but not in expected)
    for actual in actual_positions {
        if !expected_positions.iter().any(|e| e.symbol == actual.symbol) {
            result.add_mismatch(Mismatch::PositionMismatch {
                symbol: actual.symbol.clone(),
                expected: 0.0,
                actual: actual.quantity,
            });
        }
    }

    result
}

/// Reconcile expected orders against actual broker state
pub fn reconcile_orders(
    expected_orders: &[OrderState],
    actual_orders: &[OrderState],
) -> ReconciliationResult {
    let mut result = ReconciliationResult::new();

    // Check for missing or mismatched orders
    for expected in expected_orders {
        match actual_orders.iter().find(|a| a.id == expected.id) {
            None => {
                result.add_mismatch(Mismatch::MissingOrder(expected.id.clone()));
            }
            Some(actual) => {
                if actual.status != expected.status {
                    result.add_mismatch(Mismatch::StatusMismatch {
                        order_id: expected.id.clone(),
                        expected: expected.status.clone(),
                        actual: actual.status.clone(),
                    });
                }
            }
        }
    }

    // Check for orphaned orders
    for actual in actual_orders {
        if !expected_orders.iter().any(|e| e.id == actual.id) {
            result.add_mismatch(Mismatch::OrphanedOrder(actual.id.clone()));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_order(id: &str, status: &str) -> OrderState {
        OrderState {
            id: id.to_string(),
            status: status.to_string(),
        }
    }

    #[test]
    fn test_clean_reconciliation() {
        let expected = vec![
            make_order("1", "filled"),
            make_order("2", "pending"),
        ];
        let actual = vec![
            make_order("1", "filled"),
            make_order("2", "pending"),
        ];

        let result = reconcile_orders(&expected, &actual);
        assert!(result.is_clean);
        assert_eq!(result.mismatches.len(), 0);
    }

    #[test]
    fn test_missing_order() {
        let expected = vec![make_order("1", "filled")];
        let actual = vec![];

        let result = reconcile_orders(&expected, &actual);
        assert!(!result.is_clean);
        assert_eq!(result.mismatches.len(), 1);
        assert!(matches!(&result.mismatches[0], Mismatch::MissingOrder(id) if id == "1"));
    }

    #[test]
    fn test_orphaned_order() {
        let expected = vec![];
        let actual = vec![make_order("1", "filled")];

        let result = reconcile_orders(&expected, &actual);
        assert!(!result.is_clean);
        assert_eq!(result.mismatches.len(), 1);
        assert!(matches!(&result.mismatches[0], Mismatch::OrphanedOrder(id) if id == "1"));
    }

    #[test]
    fn test_status_mismatch() {
        let expected = vec![make_order("1", "pending")];
        let actual = vec![make_order("1", "filled")];

        let result = reconcile_orders(&expected, &actual);
        assert!(!result.is_clean);
        assert_eq!(result.mismatches.len(), 1);
        assert!(matches!(
            &result.mismatches[0],
            Mismatch::StatusMismatch { order_id, .. } if order_id == "1"
        ));
    }

    #[test]
    fn test_multiple_mismatches() {
        let expected = vec![
            make_order("1", "pending"),
            make_order("2", "filled"),
        ];
        let actual = vec![
            make_order("1", "filled"),  // status mismatch
            make_order("3", "pending"), // orphaned
        ];

        let result = reconcile_orders(&expected, &actual);
        assert!(!result.is_clean);
        assert_eq!(result.mismatches.len(), 3); // missing "2", status mismatch "1", orphaned "3"
    }

    fn make_position(symbol: &str, quantity: f64) -> PositionState {
        PositionState {
            symbol: symbol.to_string(),
            quantity,
        }
    }

    #[test]
    fn test_clean_position_reconciliation() {
        let expected = vec![
            make_position("AAPL", 100.0),
            make_position("MSFT", 50.0),
        ];
        let actual = vec![
            make_position("AAPL", 100.0),
            make_position("MSFT", 50.0),
        ];

        let result = reconcile_positions(&expected, &actual);
        assert!(result.is_clean);
        assert_eq!(result.mismatches.len(), 0);
    }

    #[test]
    fn test_position_quantity_mismatch() {
        let expected = vec![make_position("AAPL", 100.0)];
        let actual = vec![make_position("AAPL", 75.0)];

        let result = reconcile_positions(&expected, &actual);
        assert!(!result.is_clean);
        assert_eq!(result.mismatches.len(), 1);
        assert!(matches!(
            &result.mismatches[0],
            Mismatch::PositionMismatch { symbol, expected: 100.0, actual: 75.0 } if symbol == "AAPL"
        ));
    }

    #[test]
    fn test_position_missing() {
        let expected = vec![make_position("AAPL", 100.0)];
        let actual = vec![];

        let result = reconcile_positions(&expected, &actual);
        assert!(!result.is_clean);
        assert_eq!(result.mismatches.len(), 1);
        assert!(matches!(
            &result.mismatches[0],
            Mismatch::PositionMismatch { symbol, expected: 100.0, actual: 0.0 } if symbol == "AAPL"
        ));
    }

    #[test]
    fn test_position_orphaned() {
        let expected = vec![];
        let actual = vec![make_position("AAPL", 100.0)];

        let result = reconcile_positions(&expected, &actual);
        assert!(!result.is_clean);
        assert_eq!(result.mismatches.len(), 1);
        assert!(matches!(
            &result.mismatches[0],
            Mismatch::PositionMismatch { symbol, expected: 0.0, actual: 100.0 } if symbol == "AAPL"
        ));
    }

    #[test]
    fn test_position_tolerance() {
        // Small floating point differences should be ignored
        let expected = vec![make_position("AAPL", 100.0)];
        let actual = vec![make_position("AAPL", 100.0000001)];

        let result = reconcile_positions(&expected, &actual);
        assert!(result.is_clean);
        assert_eq!(result.mismatches.len(), 0);
    }
}
