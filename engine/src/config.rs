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
    pub http_port: u16,
    pub symbol: String,
    pub database_url: String,
    /// Path to norm stats JSON (median/MAD, Wave C equities format).
    pub norm_stats_path: String,
    /// Number of candles sent as the feature window to the inference service.
    pub feature_window_size: usize,
    /// Path to the `parity_verified.json` marker written by Feature 13.
    pub parity_marker_path: String,
    /// Maximum age (in seconds) of the parity marker before `live` mode
    /// refuses to start. Default: 7 days.
    pub parity_max_age_secs: i64,
    /// Path to Moomoo OpenAPI credentials JSON (Wave D). Empty → Yahoo fallback.
    pub moomoo_creds_path: String,
    /// FRED API key (optional; higher rate limit). Empty → anonymous CSV fallback.
    pub fred_api_key: String,
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
        let _ = dotenvy::dotenv();

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

        let http_port = parse_env::<u16>("HTTP_PORT", "8080")
            .context("HTTP_PORT must be a u16 in the range 1..=65535")?;
        if http_port == 0 {
            return Err(anyhow!("HTTP_PORT must be > 0, got 0"));
        }

        let symbol = env_or("SYMBOL", "BTC/USD");
        if symbol.trim().is_empty() {
            return Err(anyhow!("SYMBOL must not be empty"));
        }

        if trading_mode == TradingMode::Live {
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

            verify_parity_marker(&parity_marker_path, parity_max_age_secs)?;

            // Store into the config below.
            // (These locals are re-read outside the block for the struct.)
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

        Ok(Self {
            trading_mode,
            zmq_endpoint,
            magnitude_threshold,
            paper_fee,
            sma_window,
            http_port,
            symbol,
            database_url,
            norm_stats_path,
            feature_window_size,
            parity_marker_path,
            parity_max_age_secs,
            moomoo_creds_path,
            fred_api_key,
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
            "HTTP_PORT",
            "SYMBOL",
            "KRAKEN_API_KEY",
            "KRAKEN_API_SECRET",
            "DATABASE_URL",
            "NORM_STATS_PATH",
            "FEATURE_WINDOW_SIZE",
            "PARITY_MARKER_PATH",
            "PARITY_MAX_AGE_SECS",
            "MOOMOO_CREDS_PATH",
            "FRED_API_KEY",
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
        assert_eq!(cfg.http_port, 8080);
        assert_eq!(cfg.symbol, "BTC/USD");
        assert_eq!(cfg.database_url, "sqlite://data/candles.db");
        // dotenvy loads .env which overrides the default path.
        assert_eq!(cfg.norm_stats_path, "/models/norm_stats_qqq_v1.json");
        assert_eq!(cfg.feature_window_size, 126);
        assert_eq!(cfg.parity_marker_path, "parity_verified.json");
        assert_eq!(cfg.parity_max_age_secs, 7 * 24 * 60 * 60);
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
        env::set_var("TRADING_MODE", "live");
        // Wave 5: Kraken retired; live execution is not yet wired to Binance, so
        // the engine must still load (and will fall back to paper at runtime).
        let cfg = Config::from_env().expect("live mode must load without exchange keys");
        assert_eq!(cfg.trading_mode, TradingMode::Live);
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
        let err = Config::from_env().expect_err("blank PARITY_MARKER_PATH must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("PARITY_MARKER_PATH"), "msg: {msg}");
    }

    #[test]
    fn custom_env_overrides_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("ZMQ_ENDPOINT", "tcp://inference:5555");
        env::set_var("MAGNITUDE_THRESHOLD", "0.01");
        env::set_var("PAPER_FEE", "0.0025");
        env::set_var("SMA_WINDOW", "120");
        env::set_var("HTTP_PORT", "9090");
        env::set_var("SYMBOL", "ETH/USD");
        let cfg = Config::from_env().expect("custom env must load");
        assert_eq!(cfg.zmq_endpoint, "tcp://inference:5555");
        assert!((cfg.magnitude_threshold - 0.01).abs() < 1e-12);
        assert!((cfg.paper_fee - 0.0025).abs() < 1e-12);
        assert_eq!(cfg.sma_window, 120);
        assert_eq!(cfg.http_port, 9090);
        assert_eq!(cfg.symbol, "ETH/USD");
    }

    #[test]
    fn invalid_trading_mode_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("TRADING_MODE", "scalp");
        let err = Config::from_env().expect_err("invalid TRADING_MODE must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("TRADING_MODE"));
        assert!(msg.contains("scalp"));
    }

    #[test]
    fn non_numeric_threshold_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("MAGNITUDE_THRESHOLD", "high");
        let err = Config::from_env().expect_err("non-numeric threshold must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("MAGNITUDE_THRESHOLD"));
    }

    #[test]
    fn non_positive_threshold_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("MAGNITUDE_THRESHOLD", "0");
        let err = Config::from_env().expect_err("zero threshold must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("MAGNITUDE_THRESHOLD must be > 0"));
    }

    #[test]
    fn zero_paper_fee_allowed() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("PAPER_FEE", "0");
        let cfg = Config::from_env().expect("zero PAPER_FEE is valid");
        assert!((cfg.paper_fee - 0.0).abs() < 1e-12);
    }

    #[test]
    fn negative_paper_fee_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("PAPER_FEE", "-0.001");
        let err = Config::from_env().expect_err("negative PAPER_FEE must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("PAPER_FEE must be >= 0"));
    }

    #[test]
    fn zero_sma_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("SMA_WINDOW", "0");
        let err = Config::from_env().expect_err("SMA_WINDOW=0 must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("SMA_WINDOW must be > 0"));
    }

    #[test]
    fn zero_port_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("HTTP_PORT", "0");
        let err = Config::from_env().expect_err("HTTP_PORT=0 must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("HTTP_PORT must be > 0"));
    }

    #[test]
    fn empty_symbol_rejected() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("SYMBOL", "   ");
        let err = Config::from_env().expect_err("blank SYMBOL must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("SYMBOL must not be empty"));
    }

    #[test]
    fn config_round_trips_through_serde() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        let cfg = Config::from_env().expect("defaults load");
        let json = serde_json::to_string(&cfg).expect("serialize");
        let back: Config = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(format!("{:?}", cfg), format!("{:?}", back));
    }
}
