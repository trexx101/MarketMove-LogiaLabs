use std::fmt;

// ---------------------------------------------------------------------------
// Position
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    Flat = 0,
    Long = 1,
    Short = -1,
}

impl Position {
    pub fn from_i64(v: i64) -> Self {
        match v {
            1 => Position::Long,
            -1 => Position::Short,
            _ => Position::Flat,
        }
    }

    pub fn as_i64(self) -> i64 {
        self as i64
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Position::Flat => write!(f, "flat"),
            Position::Long => write!(f, "long"),
            Position::Short => write!(f, "short"),
        }
    }
}

// ---------------------------------------------------------------------------
// StrategyParams
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StrategyParams {
    /// Entry threshold: both pred_4h and pred_24h must exceed this (absolute value).
    pub magnitude_threshold: f64,
    /// Rolling window size for SMA regime computation.
    pub sma_window: usize,
}

// ---------------------------------------------------------------------------
// SignalInput
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SignalInput {
    pub pred_4h: f64,
    pub pred_24h: f64,
    /// Most recent close price.
    pub current_close: f64,
    /// SMA of the last `sma_window` closes. Pass `0.0` if insufficient data (skips regime filter).
    pub sma: f64,
    /// True if there are at least `sma_window` candles available for a reliable SMA.
    pub sma_valid: bool,
}

// ---------------------------------------------------------------------------
// compute_sma
// ---------------------------------------------------------------------------

/// Compute the SMA over up to the last `window` elements in `closes`.
/// Returns `(mean, is_full_window)`.
pub fn compute_sma(closes: &[f64], window: usize) -> (f64, bool) {
    if closes.is_empty() || window == 0 {
        return (0.0, false);
    }
    let n = closes.len().min(window);
    let slice = &closes[closes.len() - n..];
    let mean = slice.iter().sum::<f64>() / n as f64;
    (mean, closes.len() >= window)
}

// ---------------------------------------------------------------------------
// next_position
// ---------------------------------------------------------------------------

/// Regime-filtered hysteresis state machine matching the Colab backtester.
pub fn next_position(current: Position, input: &SignalInput, params: &StrategyParams) -> Position {
    let threshold = params.magnitude_threshold;

    // Step 1 — Raw signal
    let raw_signal: i64 = if input.pred_4h > threshold && input.pred_24h > threshold {
        1
    } else if input.pred_4h < -threshold && input.pred_24h < -threshold {
        -1
    } else {
        0
    };

    // Step 2 — Regime (only when sma_valid)
    if !input.sma_valid {
        // Unknown regime: block all entries, hold current
        return current;
    }
    let regime: i64 = if input.current_close > input.sma { 1 } else { -1 };

    // Step 3 — Filtered signal (asymmetric regime gate)
    let filtered: i64 = if raw_signal == 1 && regime == 1 {
        1
    } else if raw_signal == -1 && regime == -1 {
        -1
    } else {
        0
    };

    // Step 4 — Hysteresis / forward-fill
    if filtered != 0 {
        Position::from_i64(filtered)
    } else {
        current
    }
}

// ---------------------------------------------------------------------------
// Equities strategy (Wave C) — daily horizons, long/flat only
// ---------------------------------------------------------------------------

/// Strategy parameters for the QQQ daily equities strategy.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct EquityStrategyParams {
    /// Entry threshold: pred_1d must exceed this (absolute) to go long.
    pub entry_threshold: f64,
    /// Exit threshold: if pred_1d falls below this (negative), exit to flat.
    pub exit_threshold: f64,
    /// Rolling window for SMA regime filter (e.g. 200).
    pub sma_window: usize,
}

impl Default for EquityStrategyParams {
    fn default() -> Self {
        Self {
            entry_threshold: 0.003,
            exit_threshold: -0.001,
            sma_window: 200,
        }
    }
}

/// Signal input for the daily equities strategy.
#[derive(Debug, Clone)]
pub struct EquitySignalInput {
    pub pred_1d: f64,
    pub pred_5d: f64,
    pub pred_21d: f64,
    pub current_close: f64,
    pub sma: f64,
    pub sma_valid: bool,
}

/// Daily equities position state machine.
/// Long/flat only (no shorting — QQQ has persistent positive drift).
/// Uses pred_1d as the primary signal with pred_5d confirmation,
/// filtered by SMA200 regime.
pub fn next_equity_position(
    current: Position,
    input: &EquitySignalInput,
    params: &EquityStrategyParams,
) -> Position {
    // Regime filter: if SMA invalid or close below SMA200, block new longs.
    if !input.sma_valid || input.current_close <= input.sma {
        // Allow exits in bearish regime but no new entries.
        if current == Position::Long && input.pred_1d < params.exit_threshold {
            return Position::Flat;
        }
        return if current == Position::Long { Position::Long } else { Position::Flat };
    }

    // Bullish regime (close > SMA200).
    // Entry: pred_1d > entry_threshold AND pred_5d > 0 (confirmation).
    if input.pred_1d > params.entry_threshold && input.pred_5d > 0.0 {
        return Position::Long;
    }

    // Exit: pred_1d < exit_threshold.
    if current == Position::Long && input.pred_1d < params.exit_threshold {
        return Position::Flat;
    }

    // Hold current position.
    current
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn params(threshold: f64) -> StrategyParams {
        StrategyParams {
            magnitude_threshold: threshold,
            sma_window: 200,
        }
    }

    fn signal(pred_4h: f64, pred_24h: f64, close: f64, sma: f64, sma_valid: bool) -> SignalInput {
        SignalInput {
            pred_4h,
            pred_24h,
            current_close: close,
            sma,
            sma_valid,
        }
    }

    #[test]
    fn next_position_long_entry_in_bullish_regime() {
        let input = signal(0.01, 0.01, 51000.0, 50000.0, true);
        let result = next_position(Position::Flat, &input, &params(0.005));
        assert_eq!(result, Position::Long);
    }

    #[test]
    fn next_position_short_entry_in_bearish_regime() {
        let input = signal(-0.01, -0.01, 49000.0, 50000.0, true);
        let result = next_position(Position::Flat, &input, &params(0.005));
        assert_eq!(result, Position::Short);
    }

    #[test]
    fn next_position_holds_long_on_neutral_signal() {
        // Signal below threshold → ffill Long
        let input = signal(0.002, 0.002, 51000.0, 50000.0, true);
        let result = next_position(Position::Long, &input, &params(0.005));
        assert_eq!(result, Position::Long);
    }

    #[test]
    fn next_position_holds_short_on_neutral_signal() {
        // Signal below threshold → ffill Short
        let input = signal(-0.002, -0.002, 49000.0, 50000.0, true);
        let result = next_position(Position::Short, &input, &params(0.005));
        assert_eq!(result, Position::Short);
    }

    #[test]
    fn next_position_no_long_in_bearish_regime() {
        // Long signal but close < sma → filtered=0 → ffill Flat
        let input = signal(0.01, 0.01, 49000.0, 50000.0, true);
        let result = next_position(Position::Flat, &input, &params(0.005));
        assert_eq!(result, Position::Flat);
    }

    #[test]
    fn next_position_no_short_in_bullish_regime() {
        // Short signal but close > sma → filtered=0 → ffill Flat
        let input = signal(-0.01, -0.01, 51000.0, 50000.0, true);
        let result = next_position(Position::Flat, &input, &params(0.005));
        assert_eq!(result, Position::Flat);
    }

    #[test]
    fn next_position_flips_long_to_short() {
        // Currently Long, short signal in bearish regime → Short
        let input = signal(-0.01, -0.01, 49000.0, 50000.0, true);
        let result = next_position(Position::Long, &input, &params(0.005));
        assert_eq!(result, Position::Short);
    }

    #[test]
    fn next_position_sma_invalid_blocks_all_entries() {
        // sma_valid=false → regime unknown → hold current (Flat stays Flat)
        let input = signal(0.01, 0.01, 51000.0, 50000.0, false);
        let result = next_position(Position::Flat, &input, &params(0.005));
        assert_eq!(result, Position::Flat);
    }

    #[test]
    fn compute_sma_full_window() {
        let closes = vec![42000.0_f64; 200];
        let (mean, valid) = compute_sma(&closes, 200);
        assert!(valid);
        assert!((mean - 42000.0).abs() < 1e-9);
    }

    #[test]
    fn compute_sma_partial_window() {
        // Only 50 closes but window=200 → valid=false, mean is average of those 50
        let closes: Vec<f64> = (1..=50).map(|x| x as f64).collect();
        let (mean, valid) = compute_sma(&closes, 200);
        assert!(!valid);
        let expected = (1.0 + 50.0) / 2.0; // arithmetic mean of 1..=50
        assert!((mean - expected).abs() < 1e-9);
    }

    #[test]
    fn compute_sma_empty_returns_false() {
        let (mean, valid) = compute_sma(&[], 200);
        assert!(!valid);
        assert_eq!(mean, 0.0);
    }

    #[test]
    fn position_roundtrip_i64() {
        assert_eq!(Position::from_i64(1), Position::Long);
        assert_eq!(Position::from_i64(-1), Position::Short);
        assert_eq!(Position::from_i64(0), Position::Flat);
        assert_eq!(Position::from_i64(99), Position::Flat);

        assert_eq!(Position::Long.as_i64(), 1);
        assert_eq!(Position::Short.as_i64(), -1);
        assert_eq!(Position::Flat.as_i64(), 0);
    }
}
