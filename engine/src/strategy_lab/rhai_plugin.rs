use rhai::{Engine, Scope, EvalAltResult};

use crate::strategy::EquitySignalInput;

/// Evaluate a user-supplied Rhai script that computes the next position.
///
/// Available variables in scope:
///   `pred_1d`, `pred_5d`, `pred_21d` — model predictions (log returns).
///   `current_close` — current bar close price.
///   `sma` — trailing SMA value (0.0 if not yet valid).
///   `sma_valid` — `true` once the SMA window is full.
///   `current_pos` — current position: -1 (short), 0 (flat), 1 (long).
///
/// The script must return an `i64` in {-1, 0, 1}.
pub fn evaluate_rhai_strategy(
    script: &str,
    input: &EquitySignalInput,
    current_pos: i64,
) -> Result<i64, Box<EvalAltResult>> {
    let mut engine = Engine::new();
    engine.set_max_expr_depths(50, 50);
    engine.set_max_operations(10_000);

    let mut scope = Scope::new();
    scope.push("pred_1d", input.pred_1d);
    scope.push("pred_5d", input.pred_5d);
    scope.push("pred_21d", input.pred_21d);
    scope.push("current_close", input.current_close);
    scope.push("sma", input.sma);
    scope.push("sma_valid", input.sma_valid);
    scope.push("current_pos", current_pos);

    engine.eval_with_scope::<i64>(&mut scope, script)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_input() -> EquitySignalInput {
        EquitySignalInput {
            pred_1d: 0.01,
            pred_5d: 0.005,
            pred_21d: 0.02,
            current_close: 500.0,
            sma: 480.0,
            sma_valid: true,
        }
    }

    #[test]
    fn test_rhai_simple_long() {
        let script = "if pred_1d > 0.005 && current_close > sma { 1 } else { 0 }";
        let result = evaluate_rhai_strategy(script, &test_input(), 0).unwrap();
        assert_eq!(result, 1);
    }

    #[test]
    fn test_rhai_flat_when_no_signal() {
        let script = "if pred_1d < -0.05 { -1 } else { 0 }";
        let result = evaluate_rhai_strategy(script, &test_input(), 0).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_rhai_uses_current_pos() {
        let script = "if current_pos == 1 && pred_1d < 0.0 { -1 } else { current_pos }";
        let result = evaluate_rhai_strategy(script, &test_input(), 1).unwrap();
        // pred_1d=0.01 > 0, so condition is false → returns current_pos=1
        assert_eq!(result, 1);
    }

    #[test]
    fn test_rhai_syntax_error() {
        let result = evaluate_rhai_strategy("this is not valid!!", &test_input(), 0);
        assert!(result.is_err());
    }
}