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
    /// Enable shorting via inverse ETF (PSQ) in the bearish regime. Default off.
    #[serde(default)]
    pub enable_shorting: bool,
    /// Short entry threshold: pred_1d must fall below this (negative) to go short.
    #[serde(default = "default_short_entry_threshold")]
    pub short_entry_threshold: f64,
    /// Short exit threshold: pred_1d rising above this exits the short to flat.
    #[serde(default = "default_short_exit_threshold")]
    pub short_exit_threshold: f64,
}

fn default_short_entry_threshold() -> f64 {
    -0.004
}
fn default_short_exit_threshold() -> f64 {
    0.001
}

impl Default for EquityStrategyParams {
    fn default() -> Self {
        Self {
            entry_threshold: 0.003,
            exit_threshold: -0.001,
            sma_window: 200,
            enable_shorting: false,
            short_entry_threshold: default_short_entry_threshold(),
            short_exit_threshold: default_short_exit_threshold(),
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
///
/// Long/flat when shorting is disabled (default). When `enable_shorting` is
/// set, a bearish regime (`close <= SMA200` or invalid SMA) can also produce a
/// `Short` target, executed via an inverse ETF (PSQ) by the executor.
///
/// Uses `pred_1d` as the primary signal with `pred_5d` confirmation, filtered
/// by the SMA200 regime.
///
/// Transition safety (executor relies on this):
/// - `Long -> Short` is never returned directly. A long first exits to `Flat`,
///   then a later tick may enter `Short`.
/// - `Short -> Long` is never returned directly. A short first exits to `Flat`,
///   then a later tick may enter `Long`.
pub fn next_equity_position(
    current: Position,
    input: &EquitySignalInput,
    params: &EquityStrategyParams,
) -> Position {
    // --- 1. Exit the currently-held position (regime-agnostic). ---
    match current {
        Position::Long => {
            // Long exit whenever pred_1d drops below the exit threshold.
            if input.pred_1d < params.exit_threshold {
                return Position::Flat;
            }
        }
        Position::Short => {
            // Short exit when pred_1d recovers above the short-exit threshold.
            // This also flattens a stray short if shorting is disabled,
            // preserving the original long/flat-only behavior.
            if input.pred_1d > params.short_exit_threshold {
                return Position::Flat;
            }
        }
        Position::Flat => {}
    }

    // --- 2. Enter a new position (regime-gated; never Long<->Short directly). ---
    let bullish = input.sma_valid && input.current_close > input.sma;

    if bullish {
        // Bullish regime: long entries only.
        if current == Position::Flat
            && input.pred_1d > params.entry_threshold
            && input.pred_5d > 0.0
        {
            return Position::Long;
        }
        // Long holds; a held Short is retained until it exits above (step 1).
        return current;
    }

    // Bearish regime (close <= SMA200 OR sma_invalid): no long entries.
    // Short entries only when enabled and currently Flat.
    if params.enable_shorting && current == Position::Flat {
        if input.pred_1d < params.short_entry_threshold {
            return Position::Short;
        }
    }
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

    // --- next_equity_position shorting tests (Phase 3.1) ---

    fn eq_params(
        enable_shorting: bool,
        entry: f64,
        exit: f64,
        short_entry: f64,
        short_exit: f64,
    ) -> EquityStrategyParams {
        EquityStrategyParams {
            entry_threshold: entry,
            exit_threshold: exit,
            sma_window: 200,
            enable_shorting,
            short_entry_threshold: short_entry,
            short_exit_threshold: short_exit,
        }
    }

    fn eq_signal(
        pred_1d: f64,
        pred_5d: f64,
        close: f64,
        sma: f64,
        sma_valid: bool,
    ) -> EquitySignalInput {
        EquitySignalInput {
            pred_1d,
            pred_5d,
            pred_21d: 0.0,
            current_close: close,
            sma,
            sma_valid,
        }
    }

    #[test]
    fn equity_short_entry_in_bearish_regime() {
        // Shorting on, bearish regime, strongly negative pred_1d → Short.
        let p = eq_params(true, 0.003, -0.001, -0.004, 0.001);
        let input = eq_signal(-0.006, -0.01, 49000.0, 50000.0, true);
        let result = next_equity_position(Position::Flat, &input, &p);
        assert_eq!(result, Position::Short);
    }

    #[test]
    fn equity_short_entry_blocked_when_shorting_disabled() {
        // Shorting off (default) → bearish regime never yields Short (regression).
        let p = EquityStrategyParams::default();
        assert!(!p.enable_shorting);
        let input = eq_signal(-0.006, -0.01, 49000.0, 50000.0, true);
        let result = next_equity_position(Position::Flat, &input, &p);
        assert_eq!(result, Position::Flat);
    }

    #[test]
    fn equity_short_entry_requires_bearish_regime() {
        // Strongly negative pred_1d but bullish regime (close > SMA) → no Short.
        let p = eq_params(true, 0.003, -0.001, -0.004, 0.001);
        let input = eq_signal(-0.006, -0.01, 51000.0, 50000.0, true);
        let result = next_equity_position(Position::Flat, &input, &p);
        assert_eq!(result, Position::Flat);
    }

    #[test]
    fn equity_short_entry_requires_flat() {
        // Already Long: short entry must NOT fire — long exits first, no Short.
        let p = eq_params(true, 0.003, -0.001, -0.004, 0.001);
        let input = eq_signal(-0.006, -0.01, 49000.0, 50000.0, true);
        let result = next_equity_position(Position::Long, &input, &p);
        assert_eq!(result, Position::Flat);
    }

    #[test]
    fn equity_short_exit_to_flat() {
        // Held Short, pred_1d recovers above short_exit_threshold → Flat.
        let p = eq_params(true, 0.003, -0.001, -0.004, 0.001);
        let input = eq_signal(0.003, 0.01, 51000.0, 50000.0, true);
        let result = next_equity_position(Position::Short, &input, &p);
        assert_eq!(result, Position::Flat);
    }

    #[test]
    fn equity_short_holds_when_signal_weak() {
        // Held Short, pred_1d still below short_exit_threshold → keep Short.
        let p = eq_params(true, 0.003, -0.001, -0.004, 0.001);
        let input = eq_signal(-0.02, -0.03, 48000.0, 50000.0, true);
        let result = next_equity_position(Position::Short, &input, &p);
        assert_eq!(result, Position::Short);
    }

    #[test]
    fn equity_short_disabled_flattens_stray_short() {
        // Shorting off but a Short position exists; pred_1d > short_exit → Flat.
        let p = EquityStrategyParams::default();
        let input = eq_signal(0.005, 0.01, 51000.0, 50000.0, true);
        let result = next_equity_position(Position::Short, &input, &p);
        assert_eq!(result, Position::Flat);
    }

    #[test]
    fn equity_long_to_short_is_two_step() {
        // A long in a bearish regime with a strongly negative pred must first
        // return Flat (not jump directly to Short). Verified across two ticks.
        let p = eq_params(true, 0.003, -0.001, -0.004, 0.001);
        // Tick 1: Long + very negative pred → exit to Flat.
        let input = eq_signal(-0.006, -0.01, 49000.0, 50000.0, true);
        let t1 = next_equity_position(Position::Long, &input, &p);
        assert_eq!(t1, Position::Flat);
        // Tick 2: Flat + same signal → now allowed to enter Short.
        let t2 = next_equity_position(t1, &input, &p);
        assert_eq!(t2, Position::Short);
    }

    #[test]
    fn equity_long_entry_unchanged_in_bullish() {
        // Existing bullish long-entry behavior is preserved.
        let p = EquityStrategyParams::default();
        let input = eq_signal(0.005, 0.01, 51000.0, 50000.0, true);
        let result = next_equity_position(Position::Flat, &input, &p);
        assert_eq!(result, Position::Long);
    }

    #[test]
    fn equity_long_holds_in_bearish_until_exit() {
        // Held Long in bearish regime with pred above exit threshold → stays Long.
        let p = EquityStrategyParams::default();
        let input = eq_signal(0.0, 0.0, 49000.0, 50000.0, true);
        let result = next_equity_position(Position::Long, &input, &p);
        assert_eq!(result, Position::Long);
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
