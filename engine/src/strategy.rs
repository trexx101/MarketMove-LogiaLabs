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
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
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
    /// Require pred_5d > 0.0 as an additional confirmation filter for long entries.
    /// Defaults to true (original behavior). Set to false to fire more trades.
    #[serde(default = "default_pred_5d_filter")]
    pub pred_5d_filter: bool,
    /// Require pred_5d < 0.0 as an additional confirmation filter for short
    /// entries. Symmetric to `pred_5d_filter` for longs. Defaults to false
    /// (backward compatible — shorts did not previously require this).
    #[serde(default = "default_short_pred_5d_filter")]
    pub short_pred_5d_filter: bool,
    /// Enable sentiment-based risk overlay. Default false per §8.10.
    #[serde(default = "default_enable_sentiment_overlay")]
    pub enable_sentiment_overlay: bool,
    /// Sentiment score below which new entries are blocked (moderate negative).
    #[serde(default = "default_sentiment_reduce_threshold")]
    pub sentiment_reduce_threshold: f64,
    /// Sentiment score below which any position is forced to flat (extreme negative).
    #[serde(default = "default_sentiment_exit_threshold")]
    pub sentiment_exit_threshold: f64,
    /// Minimum article count required for the sentiment overlay to take effect.
    #[serde(default = "default_sentiment_min_articles")]
    pub sentiment_min_articles: i64,
}

fn default_short_entry_threshold() -> f64 {
    -0.004
}
fn default_short_exit_threshold() -> f64 {
    0.001
}
fn default_pred_5d_filter() -> bool {
    true
}
fn default_short_pred_5d_filter() -> bool {
    false
}
fn default_enable_sentiment_overlay() -> bool {
    false
}
fn default_sentiment_reduce_threshold() -> f64 {
    -0.5
}
fn default_sentiment_exit_threshold() -> f64 {
    -0.8
}
fn default_sentiment_min_articles() -> i64 {
    15
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
            pred_5d_filter: default_pred_5d_filter(),
            short_pred_5d_filter: default_short_pred_5d_filter(),
            enable_sentiment_overlay: default_enable_sentiment_overlay(),
            sentiment_reduce_threshold: default_sentiment_reduce_threshold(),
            sentiment_exit_threshold: default_sentiment_exit_threshold(),
            sentiment_min_articles: default_sentiment_min_articles(),
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
/// set, a bearish regime (`close <= SMA` or invalid SMA) can also produce a
/// `Short` target, executed via an inverse ETF (PSQ) by the executor.
///
/// Uses `pred_1d` as the primary signal with `pred_5d` confirmation, filtered
/// by the SMA regime.
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
    // NaN guard: if any input is non-finite, hold current position and log
    if !input.pred_1d.is_finite() || !input.pred_5d.is_finite() || !input.pred_21d.is_finite()
        || !input.current_close.is_finite() || !input.sma.is_finite()
    {
        tracing::error!(
            pred_1d = input.pred_1d,
            pred_5d = input.pred_5d,
            pred_21d = input.pred_21d,
            close = input.current_close,
            sma = input.sma,
            "non-finite input to next_equity_position, holding current position"
        );
        return current;
    }

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

    // --- 2. Regime-conflict exits (hard exits when regime contradicts position). ---
    let bullish = input.sma_valid && input.current_close > input.sma;

    // Short in bullish regime: force exit. This is the most expensive failure
    // mode — shorting into a rally has no upside cap and inverse-ETF drag.
    // Without this, a short held while the market trends up stays open forever
    // when pred_1d stays below the exit threshold (e.g. z-score cold start 0.0).
    if current == Position::Short && bullish {
        return Position::Flat;
    }

    // NOTE: a held Long in bearish regime is intentionally NOT force-exited
    // here — long exits remain prediction-driven (pred_1d < exit_threshold),
    // preserving the original long/flat state machine (see
    // `equity_long_holds_in_bearish_until_exit`).

    // --- 3. Enter a new position (regime-gated; never Long<->Short directly). ---
    if bullish {
        // Bullish regime: long entries only.
        if current == Position::Flat
            && input.pred_1d > params.entry_threshold
            && (!params.pred_5d_filter || input.pred_5d > 0.0)
        {
            return Position::Long;
        }
        // Long holds in bullish regime.
        return current;
    }

    // Bearish regime (close <= SMA OR sma_valid): no long entries.
    // Short entries only when enabled, currently Flat, AND sma_valid is true.
    // This prevents shorts during warmup when regime is unknown.
    if params.enable_shorting && current == Position::Flat && input.sma_valid {
        if input.pred_1d < params.short_entry_threshold
            && (!params.short_pred_5d_filter || input.pred_5d < 0.0)
        {
            return Position::Short;
        }
    }
    current
}

/// Sentiment risk overlay.
///
/// Returns the position after applying sentiment-based risk rules.
/// - If overlay is disabled or article count is below min_articles: no effect.
/// - score < exit_threshold: force Flat (hard exit).
/// - reduce_threshold <= score < exit_threshold: block new entries (return Flat if current is Flat).
/// - score >= reduce_threshold: no effect.
///
/// Note: the original plan proposed a size multiplier to halve position size on
/// moderate negative sentiment. We instead block new entries only, avoiding
/// invasive executor changes. See plan §3C deviation note.
pub fn apply_sentiment_overlay(
    signal: Position,
    score: f64,
    article_count: i64,
    params: &EquityStrategyParams,
) -> Position {
    if !params.enable_sentiment_overlay {
        return signal;
    }
    if article_count < params.sentiment_min_articles {
        return signal;
    }
    if score < params.sentiment_exit_threshold {
        return Position::Flat;
    }
    if score < params.sentiment_reduce_threshold {
        // Block new entries while still allowing normal exits.
        if signal != Position::Flat {
            return signal;
        }
        return Position::Flat;
    }
    signal
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
        pred_5d_filter: bool,
    ) -> EquityStrategyParams {
        EquityStrategyParams {
            entry_threshold: entry,
            exit_threshold: exit,
            sma_window: 200,
            enable_shorting,
            short_entry_threshold: short_entry,
            short_exit_threshold: short_exit,
            pred_5d_filter,
            short_pred_5d_filter: false,
            enable_sentiment_overlay: false,
            sentiment_reduce_threshold: -0.5,
            sentiment_exit_threshold: -0.8,
            sentiment_min_articles: 15,
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
        let p = eq_params(true, 0.003, -0.001, -0.004, 0.001, true);
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
        let p = eq_params(true, 0.003, -0.001, -0.004, 0.001, true);
        let input = eq_signal(-0.006, -0.01, 51000.0, 50000.0, true);
        let result = next_equity_position(Position::Flat, &input, &p);
        assert_eq!(result, Position::Flat);
    }

    #[test]
    fn equity_short_entry_requires_flat() {
        // Already Long: short entry must NOT fire — long exits first, no Short.
        let p = eq_params(true, 0.003, -0.001, -0.004, 0.001, true);
        let input = eq_signal(-0.006, -0.01, 49000.0, 50000.0, true);
        let result = next_equity_position(Position::Long, &input, &p);
        assert_eq!(result, Position::Flat);
    }

    #[test]
    fn equity_short_entry_blocked_by_pred_5d_filter() {
        // short_pred_5d_filter=true: a positive pred_5d must block short entry
        // even when pred_1d is strongly negative. Symmetric to the long filter.
        let p = EquityStrategyParams {
            enable_shorting: true,
            short_pred_5d_filter: true,
            short_entry_threshold: -0.001,
            ..Default::default()
        };
        let input = EquitySignalInput {
            pred_1d: -0.005, // below short_entry_threshold
            pred_5d: 0.002,  // POSITIVE — filter should block short
            pred_21d: -0.01,
            current_close: 49000.0,
            sma: 50000.0,
            sma_valid: true,
        };
        let result = next_equity_position(Position::Flat, &input, &p);
        assert_eq!(result, Position::Flat, "short should be blocked by pred_5d filter");
    }

    #[test]
    fn equity_short_entry_allowed_by_pred_5d_filter_when_negative() {
        // short_pred_5d_filter=true but pred_5d < 0 → short allowed.
        let p = EquityStrategyParams {
            enable_shorting: true,
            short_pred_5d_filter: true,
            short_entry_threshold: -0.001,
            ..Default::default()
        };
        let input = EquitySignalInput {
            pred_1d: -0.005,
            pred_5d: -0.002, // negative → passes filter
            pred_21d: -0.01,
            current_close: 49000.0,
            sma: 50000.0,
            sma_valid: true,
        };
        let result = next_equity_position(Position::Flat, &input, &p);
        assert_eq!(result, Position::Short, "short should be allowed when pred_5d < 0");
    }

    #[test]
    fn equity_short_exit_to_flat() {
        // Held Short, pred_1d recovers above short_exit_threshold → Flat.
        let p = eq_params(true, 0.003, -0.001, -0.004, 0.001, true);
        let input = eq_signal(0.003, 0.01, 51000.0, 50000.0, true);
        let result = next_equity_position(Position::Short, &input, &p);
        assert_eq!(result, Position::Flat);
    }

    #[test]
    fn equity_short_holds_when_signal_weak() {
        // Held Short, pred_1d still below short_exit_threshold → keep Short.
        let p = eq_params(true, 0.003, -0.001, -0.004, 0.001, true);
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
        let p = eq_params(true, 0.003, -0.001, -0.004, 0.001, true);
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
    fn equity_pred_5d_filter_false_allows_negative_pred_5d() {
        // pred_5d_filter=false: long entry fires even when pred_5d < 0.
        let p = eq_params(true, 0.002, -0.0005, -0.001, 0.0005, false);
        let input = eq_signal(0.003, -0.010, 400.0, 380.0, true); // bearish regime
        // close=400 < sma=380? No — wait, 400 > 380 so this is bullish. Let me fix.
        // Actually: close > sma = bullish. For bearish: close < sma.
        // pred_5d=-0.01 (negative), pred_1d=0.003 (positive, above entry=0.002)
        // In a bullish regime this should enter long even with pred_5d < 0 (filter=false).
        let input_bullish = eq_signal(0.003, -0.010, 400.0, 380.0, true);
        let result = next_equity_position(Position::Flat, &input_bullish, &p);
        assert_eq!(result, Position::Long);
    }

    #[test]
    fn equity_pred_5d_filter_true_blocks_negative_pred_5d() {
        // pred_5d_filter=true (default): negative pred_5d blocks long entry.
        let p = eq_params(true, 0.002, -0.0005, -0.001, 0.0005, true);
        let input = eq_signal(0.003, -0.010, 400.0, 380.0, true);
        let result = next_equity_position(Position::Flat, &input, &p);
        assert_eq!(result, Position::Flat); // blocked by pred_5d filter
    }

    #[test]
    fn equity_pred_5d_filter_default_is_true() {
        // Default params must have pred_5d_filter=true (backward compatible).
        let p = EquityStrategyParams::default();
        assert!(p.pred_5d_filter);
        // And the blocking behavior must hold.
        let input = eq_signal(0.005, -0.010, 51000.0, 50000.0, true);
        let result = next_equity_position(Position::Flat, &input, &p);
        assert_eq!(result, Position::Flat); // blocked by pred_5d
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

    #[test]
    fn sentiment_overlay_disabled_returns_signal() {
        let mut p = EquityStrategyParams::default();
        p.enable_sentiment_overlay = false;
        assert_eq!(apply_sentiment_overlay(Position::Long, -0.9, 20, &p), Position::Long);
    }

    #[test]
    fn sentiment_overlay_insufficient_articles_returns_signal() {
        let mut p = EquityStrategyParams::default();
        p.enable_sentiment_overlay = true;
        p.sentiment_min_articles = 20;
        assert_eq!(apply_sentiment_overlay(Position::Long, -0.9, 15, &p), Position::Long);
    }

    #[test]
    fn sentiment_overlay_exit_threshold_forces_flat() {
        let mut p = EquityStrategyParams::default();
        p.enable_sentiment_overlay = true;
        p.sentiment_exit_threshold = -0.8;
        p.sentiment_min_articles = 5;
        assert_eq!(apply_sentiment_overlay(Position::Long, -0.85, 20, &p), Position::Flat);
        assert_eq!(apply_sentiment_overlay(Position::Short, -0.85, 20, &p), Position::Flat);
    }

    #[test]
    fn sentiment_overlay_reduce_threshold_blocks_entries() {
        let mut p = EquityStrategyParams::default();
        p.enable_sentiment_overlay = true;
        p.sentiment_reduce_threshold = -0.5;
        p.sentiment_exit_threshold = -0.8;
        p.sentiment_min_articles = 5;
        // Moderate negative sentiment blocks new entries (Flat stays Flat)
        assert_eq!(apply_sentiment_overlay(Position::Flat, -0.6, 20, &p), Position::Flat);
        // But does not force an exit from an existing position
        assert_eq!(apply_sentiment_overlay(Position::Long, -0.6, 20, &p), Position::Long);
    }

    #[test]
    fn sentiment_overlay_positive_or_neutral_returns_signal() {
        let mut p = EquityStrategyParams::default();
        p.enable_sentiment_overlay = true;
        p.sentiment_reduce_threshold = -0.5;
        p.sentiment_exit_threshold = -0.8;
        p.sentiment_min_articles = 5;
        assert_eq!(apply_sentiment_overlay(Position::Long, 0.0, 20, &p), Position::Long);
        assert_eq!(apply_sentiment_overlay(Position::Flat, 0.2, 20, &p), Position::Flat);
    }
}
