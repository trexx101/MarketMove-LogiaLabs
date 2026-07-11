use std::env;
use std::fmt;
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub trading_mode: TradingMode,
    pub zmq_endpoint: String,
    pub magnitude_threshold: f64,
    pub paper_fee: f64,
    pub sma_window: usize,
    pub http_port: u16,
    pub symbol: String,
    pub kraken_api_key: Option<String>,
    pub kraken_api_secret: Option<String>,
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

        let kraken_api_key = optional_env("KRAKEN_API_KEY");
        let kraken_api_secret = optional_env("KRAKEN_API_SECRET");

        if trading_mode == TradingMode::Live
            && (kraken_api_key.is_none() || kraken_api_secret.is_none())
        {
            return Err(anyhow!(
                "TRADING_MODE=live requires KRAKEN_API_KEY and KRAKEN_API_SECRET to be set.\n\
                 See deploy/KRAKEN_KEYS.md for the permission checklist.\n\
                 Refusing to start."
            ));
        }

        Ok(Self {
            trading_mode,
            zmq_endpoint,
            magnitude_threshold,
            paper_fee,
            sma_window,
            http_port,
            symbol,
            kraken_api_key,
            kraken_api_secret,
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn optional_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
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
        assert!(cfg.kraken_api_key.is_none());
        assert!(cfg.kraken_api_secret.is_none());
    }

    #[test]
    fn paper_mode_ignores_missing_keys() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("TRADING_MODE", "paper");
        let cfg = Config::from_env().expect("paper mode must not require keys");
        assert_eq!(cfg.trading_mode, TradingMode::Paper);
        assert!(cfg.kraken_api_key.is_none());
    }

    #[test]
    fn live_mode_without_keys_fails_fast() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("TRADING_MODE", "live");
        let err = Config::from_env().expect_err("live mode without keys must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("TRADING_MODE=live"));
        assert!(msg.contains("KRAKEN_API_KEY"));
        assert!(msg.contains("KRAKEN_API_SECRET"));
        assert!(msg.contains("deploy/KRAKEN_KEYS.md"));
    }

    #[test]
    fn live_mode_with_only_key_fails_fast() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("TRADING_MODE", "live");
        env::set_var("KRAKEN_API_KEY", "abc");
        let err = Config::from_env().expect_err("live mode with only key must fail");
        let msg = format!("{:#}", err);
        assert!(msg.contains("KRAKEN_API_SECRET"));
    }

    #[test]
    fn live_mode_with_empty_key_fails_fast() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("TRADING_MODE", "live");
        env::set_var("KRAKEN_API_KEY", "");
        env::set_var("KRAKEN_API_SECRET", "secret");
        let err = Config::from_env().expect_err("empty KRAKEN_API_KEY must be treated as missing");
        assert!(format!("{:#}", err).contains("KRAKEN_API_KEY"));
    }

    #[test]
    fn live_mode_with_both_keys_succeeds() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_engine_env();
        env::set_var("TRADING_MODE", "live");
        env::set_var("KRAKEN_API_KEY", "abc");
        env::set_var("KRAKEN_API_SECRET", "xyz");
        let cfg = Config::from_env().expect("live mode with both keys must load");
        assert_eq!(cfg.trading_mode, TradingMode::Live);
        assert_eq!(cfg.kraken_api_key.as_deref(), Some("abc"));
        assert_eq!(cfg.kraken_api_secret.as_deref(), Some("xyz"));
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
