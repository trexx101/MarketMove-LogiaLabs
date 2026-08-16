use std::env;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

/// Binance connectivity configuration (Wave 5: Kraken retired).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinanceConfig {
    pub rest_endpoint_spot: String,
    pub rest_endpoint_futures: String,
    pub ws_endpoint_spot: String,
    pub ws_endpoint_futures: String,
}

/// OpenRouter configuration for the hourly-cached LLM/vision regime feature (D4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub openrouter_api_key: String,
    pub model_name: String,
    /// Cache TTL in seconds (typically 3600 for hourly).
    pub cache_ttl_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub trading_mode: TradingMode,
    pub zmq_endpoint: String,
    pub magnitude_threshold: f64,
    pub paper_fee: f64,
    pub sma_window: usize,
    /// Enable shorting via inverse ETF (PSQ) in the equities strategy.
    /// Default: false (long/flat only).
    pub enable_shorting: bool,
    /// Short entry threshold for the equities strategy (pred_1d below this → Short).
    pub short_entry_threshold: f64,
    /// Short exit threshold for the equities strategy (pred_1d above this → Flat).
    pub short_exit_threshold: f64,
    /// Long entry threshold for the equities strategy (pred_1d above this → Long).
    /// Default: 0.001 (optimal from sweep).
    pub entry_threshold: f64,
    /// Long exit threshold for the equities strategy (pred_1d below this → Flat).
    /// Default: -0.0005 (optimal from sweep).
    pub exit_threshold: f64,
    /// Require pred_5d > 0.0 as an additional entry filter for longs.
    /// Default: false (disabled — lets more trades fire).
    pub pred_5d_filter: bool,
    /// Require pred_5d < 0.0 as an additional entry filter for shorts.
    /// Default: false (disabled — backward compatible). Symmetric to
    /// `pred_5d_filter` for longs.
    pub short_pred_5d_filter: bool,
    /// Primary symbol traded for long positions (e.g. "QQQ").
    pub symbol: String,
    /// Inverse-ETF symbol used to express short positions (default "PSQ").
    pub short_symbol: String,
    pub http_port: u16,
    pub database_url: String,
    /// Path to norm stats JSON (median/MAD, Wave C equities format).
    pub norm_stats_path: String,
    /// Number of candles sent as the feature window to the inference service.
    pub feature_window_size: usize,
    /// Days of history to backfill when a model has no prior predictions
    /// (fresh-enable). Caps the fresh-model seed so a brand-new symbol
    /// doesn't try to backfill all 1255 candles (~7m/symbol) on first boot.
    /// When the cap is hit, the scheduler only seeds the last N days; older
    /// history accumulates one candle per day forward.
    /// Default: 90. Set to 0 to disable the cap (full history).
    pub backfill_days: i64,
    /// Path to the `parity_verified.json` marker written by Feature 13.
    pub parity_marker_path: String,
    /// Maximum age (in seconds) of the parity marker before `live` mode
    /// refuses to start. Default: 7 days.
    pub parity_max_age_secs: i64,
    /// Path to Moomoo OpenAPI credentials JSON (Wave D). Empty → Yahoo fallback.
    pub moomoo_creds_path: String,
    /// FRED API key (optional; higher rate limit). Empty → anonymous CSV fallback.
    pub fred_api_key: String,
    /// Finnhub API key for sentiment + calendar (free tier: 60 req/min).
    /// Empty → sentiment falls back to stub 0.5, calendar is omitted.
    pub finnhub_api_key: String,
    /// Live executor kind. When TRADING_MODE=live, the engine picks an executor
    /// based on this value. Phase 3.3 supports `paper` (fallback) and `moomoo`.
    /// Default: `paper` (safe fallback; explicit opt-in required for moomoo).
    pub live_executor: String,
    /// Moomoo OpenD trading environment. `SIMULATE` | `REAL`.
    /// Default: `SIMULATE`. Phase 3.3 reads this from env at runtime; this
    /// field is exposed on Config for tests and future API.
    pub moomoo_trd_env: String,
    /// Base32 TOTP secret used by `POST /api/mode` to authorize live-mode flips
    /// (Phase 3.4). Loaded from `TOTP_SECRET` env var; if empty, `main.rs`
    /// generates a fresh secret and logs the otpauth URL — the user must
    /// persist it before the next restart or they will be locked out of live mode.
    pub totp_secret: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TradingMode {
    Paper,
    Live,
}

impl FromStr for TradingMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "paper" => Ok(TradingMode::Paper),
            "live" => Ok(TradingMode::Live),
            other => Err(anyhow!(
                "TRADING_MODE must be 'paper' or 'live', got '{}'",
                other
            )),
        }
    }
}

impl fmt::Display for TradingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TradingMode::Paper => f.write_str("paper"),
            TradingMode::Live => f.write_str("live"),
        }
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let trading_mode_raw = env_or("TRADING_MODE", "paper");
        let trading_mode: TradingMode = trading_mode_raw
            .parse()
            .with_context(|| format!("invalid TRADING_MODE='{}'", trading_mode_raw))?;

        let zmq_endpoint = env_or("ZMQ_ENDPOINT", "tcp://127.0.0.1:5555");

        let magnitude_threshold = parse_env::<f64>("MAGNITUDE_THRESHOLD", "0.005")
            .context("MAGNITUDE_THRESHOLD must be a number")?;
        if magnitude_threshold <= 0.0 {
            return Err(anyhow!(
                "MAGNITUDE_THRESHOLD must be > 0, got {}",
                magnitude_threshold
            ));
        }

        let paper_fee = parse_env::<f64>("PAPER_FEE", "0.0015")
            .context("PAPER_FEE must be a number")?;
        if paper_fee < 0.0 {
            return Err(anyhow!("PAPER_FEE must be >= 0, got {}", paper_fee));
        }

        let sma_window = parse_env::<usize>("SMA_WINDOW", "200")
            .context("SMA_WINDOW must be a positive integer")?;
        if sma_window == 0 {
            return Err(anyhow!("SMA_WINDOW must be > 0, got 0"));
        }

        let enable_shorting = parse_env::<bool>("ENABLE_SHORTING", "false")
            .context("ENABLE_SHORTING must be 'true' or 'false'")?;

        // Negative thresholds for shorting; defaults match EquityStrategyParams::default().
        let short_entry_threshold = parse_env::<f64>("SHORT_ENTRY_THRESHOLD", "-0.004")
            .context("SHORT_ENTRY_THRESHOLD must be a number")?;
        if short_entry_threshold >= 0.0 {
            return Err(anyhow!(
                "SHORT_ENTRY_THRESHOLD must be < 0 (short entries need a negative pred), got {}",
                short_entry_threshold
            ));
        }

        let short_exit_threshold = parse_env::<f64>("SHORT_EXIT_THRESHOLD", "0.001")
            .context("SHORT_EXIT_THRESHOLD must be a number")?;
        if short_exit_threshold <= short_entry_threshold {
            return Err(anyhow!(
                "SHORT_EXIT_THRESHOLD must be > SHORT_ENTRY_THRESHOLD, got {} <= {}",
                short_exit_threshold,
                short_entry_threshold
            ));
        }

        let entry_threshold = parse_env::<f64>("ENTRY_THRESHOLD", "0.001")
            .context("ENTRY_THRESHOLD must be a number")?;
        if entry_threshold <= 0.0 {
            return Err(anyhow!("ENTRY_THRESHOLD must be > 0, got {}", entry_threshold));
        }

        let exit_threshold = parse_env::<f64>("EXIT_THRESHOLD", "-0.0005")
            .context("EXIT_THRESHOLD must be a number")?;
        if exit_threshold >= 0.0 {
            return Err(anyhow!("EXIT_THRESHOLD must be < 0, got {}", exit_threshold));
        }

        let pred_5d_filter = parse_env::<bool>("PRED_5D_FILTER", "false")
            .context("PRED_5D_FILTER must be 'true' or 'false'")?;

        let short_pred_5d_filter = parse_env::<bool>("SHORT_PRED_5D_FILTER", "false")
            .context("SHORT_PRED_5D_FILTER must be 'true' or 'false'")?;

        let http_port = parse_env::<u16>("HTTP_PORT", "8080")
            .context("HTTP_PORT must be a u16 in the range 1..=65535")?;
        if http_port == 0 {
            return Err(anyhow!("HTTP_PORT must be > 0, got 0"));
        }

        let symbol = env_or("SYMBOL", "BTC/USD");
        if symbol.trim().is_empty() {
            return Err(anyhow!("SYMBOL must not be empty"));
        }

        let short_symbol = env_or("SHORT_SYMBOL", "PSQ");
        if short_symbol.trim().is_empty() {
            return Err(anyhow!("SHORT_SYMBOL must not be empty"));
        }

        let parity_marker_path = env_or("PARITY_MARKER_PATH", "parity_verified.json");
        if parity_marker_path.trim().is_empty() {
            return Err(anyhow!("PARITY_MARKER_PATH must not be empty"));
        }

        let parity_max_age_secs = parse_env::<i64>("PARITY_MAX_AGE_SECS", "604800")
            .context("PARITY_MAX_AGE_SECS must be an integer (seconds)")?;
        if parity_max_age_secs <= 0 {
            return Err(anyhow!(
                "PARITY_MAX_AGE_SECS must be > 0, got {}",
                parity_max_age_secs
            ));
        }

        if trading_mode == TradingMode::Live {
            verify_parity_marker(&parity_marker_path, parity_max_age_secs)?;
        }

        let moomoo_creds_path = env_or("MOOMOO_CREDS_PATH", "~/.moomoo/credentials.json");
        if moomoo_creds_path.trim().is_empty() {
            return Err(anyhow!("MOOMOO_CREDS_PATH must not be empty"));
        }
        let fred_api_key = env_or("FRED_API_KEY", "");
        let finnhub_api_key = env_or("FINNHUB_API_KEY", "");

        // Phase 3.3: live executor selection. Defaults to "paper" so that
        // upgrading to TRADING_MODE=live without explicitly setting
        // LIVE_EXECUTOR=moomoo is a no-op safe fallback.
        let live_executor = env_or("LIVE_EXECUTOR", "paper");
        if !matches!(live_executor.to_lowercase().as_str(), "paper" | "moomoo") {
            return Err(anyhow!(
                "LIVE_EXECUTOR must be 'paper' or 'moomoo', got '{}'",
                live_executor
            ));
        }
        let moomoo_trd_env = env_or("MOOMOO_TRD_ENV", "SIMULATE");
        if !matches!(moomoo_trd_env.to_uppercase().as_str(), "SIMULATE" | "REAL") {
            return Err(anyhow!(
                "MOOMOO_TRD_ENV must be 'SIMULATE' or 'REAL', got '{}'",
                moomoo_trd_env
            ));
        }

        // Phase 3.4: TOTP secret for the runtime mode toggle. Empty is allowed
        // here — main.rs will generate a fresh secret on startup and log the
        // otpauth URL so the operator can scan it. Tests may also set this
        // directly to avoid the env dependency.
        let totp_secret = env_or("TOTP_SECRET", "");

        let database_url = env_or("DATABASE_URL", "sqlite://data/candles.db");
        if database_url.trim().is_empty() {
            return Err(anyhow!("DATABASE_URL must not be empty"));
        }

        let norm_stats_path = env_or("NORM_STATS_PATH", "models/norm_stats_qqq_v1.json");
        if norm_stats_path.trim().is_empty() {
            return Err(anyhow!("NORM_STATS_PATH must not be empty"));
        }

        let feature_window_size = parse_env::<usize>("FEATURE_WINDOW_SIZE", "126")
            .context("FEATURE_WINDOW_SIZE must be a positive integer")?;
        if feature_window_size == 0 {
            return Err(anyhow!("FEATURE_WINDOW_SIZE must be > 0, got 0"));
        }

        let backfill_days = parse_env::<i64>("BACKFILL_DAYS", "90")
            .context("BACKFILL_DAYS must be an integer")?;

        Ok(Self {
            trading_mode,
            zmq_endpoint,
            magnitude_threshold,
            paper_fee,
            sma_window,
            enable_shorting,
            short_entry_threshold,
            short_exit_threshold,
            entry_threshold,
            exit_threshold,
            pred_5d_filter,
            short_pred_5d_filter,
            http_port,
            symbol,
            short_symbol,
            database_url,
            norm_stats_path,
            feature_window_size,
            backfill_days,
            parity_marker_path,
            parity_max_age_secs,
            moomoo_creds_path,
            fred_api_key,
            finnhub_api_key,
            live_executor,
            moomoo_trd_env,
            totp_secret,
        })
    }
}

/// Verify the parity-verified marker exists and is fresh.
///
/// This is the live-mode gate from Feature 13. The marker is a small JSON
/// file written by `engine::parity::write_marker` after a clean
/// regression run. If it is missing, malformed, or older than
/// `max_age_secs`, the engine refuses to start in `live` mode.
fn verify_parity_marker(marker_path: &str, max_age_secs: i64) -> Result<()> {
    let path = PathBuf::from(marker_path);
    let marker = match crate::parity::read_marker(&path)
        .with_context(|| format!("reading parity marker at {marker_path}"))?
    {
        Some(m) => m,
        None => {
            return Err(anyhow!(
                "TRADING_MODE=live requires a fresh parity marker at '{marker_path}', \
                 but no marker was found.\n\
                 Run the parity harness (Feature 13) to produce it:\n\
                   cargo run --bin engine --bin parity-harness --release\n\
                 or invoke `engine::parity::run_parity` from a CLI and call \
                 `engine::parity::write_marker` on success.\n\
                 Refusing to start in live mode."
            ));
        }
    };

    let now = chrono::Utc::now().timestamp();
    if !marker.is_fresh(now, max_age_secs) {
        let age_secs = now - marker.verified_at;
        let age_hours = age_secs / 3600;
        return Err(anyhow!(
            "TRADING_MODE=live requires a fresh parity marker, but the marker at \
             '{marker_path}' is {age_hours} hours old (max allowed: {max_age_secs} seconds).\n\
             Re-run the parity harness to refresh the marker.\n\
             Refusing to start in live mode."
        ));
    }

    tracing::info!(
        marker = marker_path,
        verified_at = marker.verified_at,
        fixture_sha256 = %marker.fixture_sha256,
        max_abs_error = marker.max_abs_error,
        "parity marker accepted"
    );
    Ok(())
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn parse_env<T>(key: &str, default: &str) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    let raw = env::var(key).unwrap_or_else(|_| default.to_string());
    raw.parse::<T>()
        .map_err(|e| anyhow!("{}='{}' could not be parsed: {}", key, raw, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_engine_env() {
        for key in [
            "TRADING_MODE",
            "ZMQ_ENDPOINT",
            "MAGNITUDE_THRESHOLD",
            "PAPER_FEE",
            "SMA_WINDOW",
            "ENABLE_SHORTING",
            "SHORT_ENTRY_THRESHOLD",
            "SHORT_EXIT_THRESHOLD",
            "ENTRY_THRESHOLD",
            "EXIT_THRESHOLD",
            "PRED_5D_FILTER",
            "SHORT_PRED_5D_FILTER",
            "HTTP_PORT",
            "SYMBOL",
            "KRAKEN_API_KEY",
            "KRAKEN_API_SECRET",
            "SHORT_SYMBOL",
            "DATABASE_URL",
            "NORM_STATS_PATH",
            "FEATURE_WINDOW_SIZE",
            "PARITY_MARKER_PATH",
            "PARITY_MAX_AGE_SECS",
            "MOOMOO_CREDS_PATH",
            "FRED_API_KEY",
            "FINNHUB_API_KEY",
            "LIVE_EXECUTOR",
            "MOOMOO_TRD_ENV",
            "TOTP_SECRET",
        ] {
            env::remove_var(key);
        }
    }

    #[test]
    fn parses_trading_mode() {
        assert_eq!("paper".parse::<TradingMode>().unwrap(), TradingMode::Paper);
        assert_eq!("live".parse::<TradingMode>().unwrap(), TradingMode::Live);
        assert_eq!(
            "PAPER".parse::<TradingMode>().unwrap(),
            TradingMode::Paper
        );
        assert!("bogus".parse::<TradingMode>().is_err());
    }

    #[test]
    fn trading_mode_display() {
        assert_eq!(TradingMode::Paper.to_string(), "paper");
        assert_eq!(TradingMode::Live.to_string(), "live");
    }

    #[test]
    fn defaults_load_when_env_unset() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        let cfg = Config::from_env().expect("defaults should load");
        assert_eq!(cfg.trading_mode, TradingMode::Paper);
        assert_eq!(cfg.zmq_endpoint, "tcp://127.0.0.1:5555");
        assert!((cfg.magnitude_threshold - 0.005).abs() < 1e-12);
        assert!((cfg.paper_fee - 0.0015).abs() < 1e-12);
        assert_eq!(cfg.sma_window, 200);
        assert!(!cfg.enable_shorting);
        assert!((cfg.short_entry_threshold - (-0.004)).abs() < 1e-12);
        assert!((cfg.short_exit_threshold - 0.001).abs() < 1e-12);
        assert!((cfg.entry_threshold - 0.001).abs() < 1e-12);
        assert!((cfg.exit_threshold - (-0.0005)).abs() < 1e-12);
        assert!(!cfg.pred_5d_filter);
        assert!(!cfg.short_pred_5d_filter);
        assert_eq!(cfg.http_port, 8080);
        assert_eq!(cfg.symbol, "BTC/USD");
        assert_eq!(cfg.database_url, "sqlite://data/candles.db");
        // Code default (dotenv is loaded in main(), not from_env()); the
        // container overrides this to /models/norm_stats_qqq_v1.json via env.
        assert_eq!(cfg.norm_stats_path, "models/norm_stats_qqq_v1.json");
        assert_eq!(cfg.feature_window_size, 126);
        assert_eq!(cfg.parity_marker_path, "parity_verified.json");
        assert_eq!(cfg.parity_max_age_secs, 7 * 24 * 60 * 60);
        assert_eq!(cfg.live_executor, "paper");
        assert_eq!(cfg.moomoo_trd_env, "SIMULATE");
        // TOTP_SECRET defaults to empty — main.rs will mint a fresh one.
        assert_eq!(cfg.totp_secret, "");
    }

    #[test]
    fn totp_secret_loaded_from_env() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("TOTP_SECRET", "JBSWY3DPEHPK3PXP");
        let cfg = Config::from_env().expect("config should load with TOTP_SECRET");
        assert_eq!(cfg.totp_secret, "JBSWY3DPEHPK3PXP");
    }

    #[test]
    fn paper_mode_ignores_missing_keys() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("TRADING_MODE", "paper");
        let cfg = Config::from_env().expect("paper mode must not require keys");
        assert_eq!(cfg.trading_mode, TradingMode::Paper);
    }

    #[test]
    fn live_mode_falls_back_to_paper() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        // Write a fresh parity marker so from_env() passes verify_parity_marker.
        let marker_path = std::env::temp_dir().join("live_mode_falls_back_to_paper_marker.json");
        let marker = crate::parity::ParityMarker {
            verified_at: chrono::Utc::now().timestamp(),
            fixture_sha256: "abc".to_string(),
            candles_compared: 0,
            max_abs_error: 1e-9,
            tolerance: 1e-6,
            notes: "test marker".to_string(),
        };
        crate::parity::write_marker(&marker_path, &marker).expect("write test parity marker");
        env::set_var("PARITY_MARKER_PATH", marker_path.to_str().unwrap());
        env::set_var("TRADING_MODE", "live");
        let cfg = Config::from_env().expect("live mode must load with parity marker");
        assert_eq!(cfg.trading_mode, TradingMode::Live);
        let _ = std::fs::remove_file(&marker_path);
    }

    #[test]
    fn live_mode_without_parity_marker_fails_fast() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("TRADING_MODE", "live");
        // Point the engine at a path that does not exist.
        let bogus = std::env::temp_dir().join("parity_marker_does_not_exist_xyz_12345.json");
        let _ = std::fs::remove_file(&bogus);
        env::set_var("PARITY_MARKER_PATH", bogus.to_str().unwrap());
        let err = Config::from_env().expect_err("live mode without a marker must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("parity marker"), "msg: {msg}");
        assert!(msg.contains("Refusing to start in live mode"), "msg: {msg}");
    }

    #[test]
    fn live_mode_with_stale_parity_marker_fails_fast() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        // Marker verified 30 days ago, default max age is 7 days → stale.
        let marker_path = std::env::temp_dir().join("parity_marker_stale_test.json");
        let stale = crate::parity::ParityMarker {
            verified_at: chrono::Utc::now().timestamp() - 30 * 24 * 3600,
            fixture_sha256: "abc".to_string(),
            candles_compared: 168,
            max_abs_error: 1e-9,
            tolerance: 1e-6,
            notes: "stale marker".to_string(),
        };
        crate::parity::write_marker(&marker_path, &stale).expect("write stale marker");

        env::set_var("TRADING_MODE", "live");
        env::set_var("PARITY_MARKER_PATH", marker_path.to_str().unwrap());
        let err = Config::from_env().expect_err("stale marker must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("hours old"), "msg: {msg}");

        let _ = std::fs::remove_file(&marker_path);
    }

    #[test]
    fn paper_mode_does_not_check_parity_marker() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        // Paper mode must not require a parity marker.
        env::set_var("TRADING_MODE", "paper");
        let bogus = std::env::temp_dir().join("parity_marker_does_not_exist_xyz_67890.json");
        let _ = std::fs::remove_file(&bogus);
        env::set_var("PARITY_MARKER_PATH", bogus.to_str().unwrap());
        let cfg = Config::from_env().expect("paper mode must not check the marker");
        assert_eq!(cfg.trading_mode, TradingMode::Paper);
    }

    #[test]
    fn zero_parity_max_age_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("PARITY_MAX_AGE_SECS", "0");
        let err = Config::from_env().expect_err("PARITY_MAX_AGE_SECS=0 must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("PARITY_MAX_AGE_SECS must be > 0"), "msg: {msg}");
    }

    #[test]
    fn empty_parity_marker_path_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("PARITY_MARKER_PATH", "   ");
        let err = Config::from_env().expect_err("empty PARITY_MARKER_PATH must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("PARITY_MARKER_PATH must not be empty"), "msg: {msg}");
    }

    #[test]
    fn non_numeric_threshold_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("MAGNITUDE_THRESHOLD", "banana");
        let err = Config::from_env().expect_err("non-numeric MAGNITUDE_THRESHOLD must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("MAGNITUDE_THRESHOLD"), "msg: {msg}");
    }

    #[test]
    fn non_positive_threshold_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("MAGNITUDE_THRESHOLD", "-1");
        let err = Config::from_env().expect_err("non-positive MAGNITUDE_THRESHOLD must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("MAGNITUDE_THRESHOLD must be > 0"), "msg: {msg}");
    }

    #[test]
    fn negative_paper_fee_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("PAPER_FEE", "-0.001");
        let err = Config::from_env().expect_err("negative PAPER_FEE must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("PAPER_FEE must be >= 0"), "msg: {msg}");
    }

    #[test]
    fn zero_paper_fee_allowed() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("PAPER_FEE", "0");
        let cfg = Config::from_env().expect("PAPER_FEE=0 must be allowed");
        assert!((cfg.paper_fee - 0.0).abs() < 1e-12);
    }

    #[test]
    fn zero_sma_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("SMA_WINDOW", "0");
        let err = Config::from_env().expect_err("SMA_WINDOW=0 must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("SMA_WINDOW must be > 0"), "msg: {msg}");
    }

    #[test]
    fn zero_port_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("HTTP_PORT", "0");
        let err = Config::from_env().expect_err("HTTP_PORT=0 must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("HTTP_PORT must be > 0"), "msg: {msg}");
    }

    #[test]
    fn invalid_trading_mode_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("TRADING_MODE", "sideways");
        let err = Config::from_env().expect_err("invalid TRADING_MODE must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("TRADING_MODE must be"), "msg: {msg}");
    }

    #[test]
    fn empty_symbol_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("SYMBOL", "   ");
        let err = Config::from_env().expect_err("empty SYMBOL must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("SYMBOL must not be empty"), "msg: {msg}");
    }

    #[test]
    fn invalid_enable_shorting_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("ENABLE_SHORTING", "maybe");
        let err = Config::from_env().expect_err("invalid ENABLE_SHORTING must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("ENABLE_SHORTING"), "msg: {msg}");
    }

    #[test]
    fn shorting_default_is_disabled() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        let cfg = Config::from_env().expect("defaults");
        assert!(!cfg.enable_shorting);
    }

    #[test]
    fn shorting_enabled_via_env() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("ENABLE_SHORTING", "true");
        let cfg = Config::from_env().expect("ENABLE_SHORTING=true should load");
        assert!(cfg.enable_shorting);
    }

    #[test]
    fn short_entry_threshold_must_be_negative() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("SHORT_ENTRY_THRESHOLD", "0.001");
        let err = Config::from_env().expect_err("non-negative SHORT_ENTRY_THRESHOLD must fail");
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("SHORT_ENTRY_THRESHOLD must be < 0"),
            "msg: {msg}"
        );
    }

    #[test]
    fn short_exit_must_exceed_short_entry() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("SHORT_ENTRY_THRESHOLD", "-0.005");
        env::set_var("SHORT_EXIT_THRESHOLD", "-0.01");
        let err = Config::from_env()
            .expect_err("SHORT_EXIT_THRESHOLD <= SHORT_ENTRY_THRESHOLD must fail");
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("SHORT_EXIT_THRESHOLD must be > SHORT_ENTRY_THRESHOLD"),
            "msg: {msg}"
        );
    }
}
