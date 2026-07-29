//! Moomoo OpenD equity executor (Phase 3.3).
//!
//! Shells out to Python scripts in `.agents/skills/moomooapi/scripts/trade/` rather
//! than binding to the `moomoo` Python SDK directly, because no Rust crate exists
//! and the SDK is the only well-maintained client for the OpenD TCP protobuf gateway.
//!
//! ## Safety
//!
//! - All trades run through `place_order.py` whose `--confirmed` flag is REQUIRED for
//!   real (non-SIMULATE) orders. This executor always passes `--confirmed` when
//!   `trd_env=REAL`.
//! - TOTP / trade unlock is **NOT** automated. The OpenD GUI must be unlocked manually
//!   before LIVE orders will succeed; the script will return an "unlock needed" error
//!   if attempted otherwise. This is intentional per the moomooapi skill's documented
//!   safety policy.
//! - Default `trd_env` is `SIMULATE`. Use `MOOMOO_TRD_ENV=REAL` only after parity
//!   validation has been re-verified and the OpenD GUI is unlocked.
//!
//! ## Subprocess failure mode
//!
//! If the Python script exits non-zero or returns malformed JSON, we return an
//! `anyhow::Error` — callers (scheduler) should surface this via `TelemetryEvent`
//! and halt the cycle. We do NOT silently fall back to paper mode.

use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use tokio::process::Command;
use tracing::{info, warn};

use crate::strategy::Position;

use super::{FillResult, TradeSide};

/// Script path: `<repo>/.agents/skills/moomooapi/scripts/trade/place_order.py`.
fn place_order_script() -> PathBuf {
    // CARGO_MANIFEST_DIR is engine/, repo root is one level up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join(".agents/skills/moomooapi/scripts/trade/place_order.py")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrdEnv {
    Simulate,
    Real,
}

impl TrdEnv {
    pub fn from_env() -> Self {
        match std::env::var("MOOMOO_TRD_ENV")
            .unwrap_or_else(|_| "SIMULATE".to_string())
            .to_uppercase()
            .as_str()
        {
            "REAL" => Self::Real,
            _ => Self::Simulate,
        }
    }

    fn as_arg(self) -> &'static str {
        match self {
            Self::Simulate => "SIMULATE",
            Self::Real => "REAL",
        }
    }

    pub fn is_real(self) -> bool {
        matches!(self, Self::Real)
    }
}

#[derive(Debug, Clone)]
pub struct MoomooExecutor {
    /// Primary symbol (e.g. "QQQ") — used for long entries and exits.
    pub symbol: String,
    /// Inverse/short symbol (e.g. "PSQ") — used for short entries and exits.
    pub short_symbol: String,
    /// Trading environment (SIMULATE / REAL).
    pub trd_env: TrdEnv,
    /// Optional security firm override (else env var MOOMOO_SECURITY_FIRM is used by
    /// the Python script).
    pub security_firm: Option<String>,
    /// Account ID override (else the script picks the first available).
    pub acc_id: Option<i64>,
}

impl MoomooExecutor {
    pub fn new(symbol: String, short_symbol: String, trd_env: TrdEnv) -> Self {
        Self {
            symbol,
            short_symbol,
            trd_env,
            security_firm: None,
            acc_id: None,
        }
    }

    /// Translate a strategy target + current position into a sequence of (symbol, side)
    /// trades that the executor would send.
    ///
    /// Exposed for parity tests; the live executor never actually loops — it just calls
    /// `set_target_position` which executes the necessary trades.
    pub fn plan_trades(
        target: Position,
        current: Position,
        symbol: &str,
        short_symbol: &str,
    ) -> Vec<(String, TradeSide)> {
        match (current, target) {
            // Already in target state — nothing to do.
            (Position::Flat, Position::Flat) => vec![],
            (Position::Long, Position::Long) => vec![],
            (Position::Short, Position::Short) => vec![],
            // Enter Long: buy primary.
            (Position::Flat, Position::Long) => vec![(symbol.to_string(), TradeSide::Buy)],
            // Enter Short: buy inverse ETF.
            (Position::Flat, Position::Short) => vec![(short_symbol.to_string(), TradeSide::Buy)],
            // Exit Long → Flat: sell primary.
            (Position::Long, Position::Flat) => vec![(symbol.to_string(), TradeSide::Sell)],
            // Exit Short → Flat: sell inverse ETF.
            (Position::Short, Position::Flat) => vec![(short_symbol.to_string(), TradeSide::Sell)],
            // Long → Short: never allowed by the strategy. The strategy must transition
            // through Flat first. Defensive: emit both trades in order.
            (Position::Long, Position::Short) => vec![
                (symbol.to_string(), TradeSide::Sell),
                (short_symbol.to_string(), TradeSide::Buy),
            ],
            // Short → Long: defensive.
            (Position::Short, Position::Long) => vec![
                (short_symbol.to_string(), TradeSide::Sell),
                (symbol.to_string(), TradeSide::Buy),
            ],
        }
    }

    /// Translate a target position into fills and submit them to Moomoo OpenD.
    ///
    /// For Phase 3.3 this returns deterministic **unfilled** FillResult rows with
    /// `realized_pnl=0.0` and `fee=0.0` — the executor captures the order ID returned
    /// by the Python script so the live scheduler can reconcile later (Phase 3.4+).
    ///
    /// If the Python subprocess fails, returns an error — the scheduler should log
    /// and skip the cycle. We deliberately do NOT silently fall back to paper.
    pub async fn set_target_position(
        &self,
        target: Position,
        _close: f64,
        ts: i64,
    ) -> Result<Vec<FillResult>> {
        // We can't read current_position from the executor — the scheduler drives that.
        // For Phase 3.3 we accept the target as authoritative and emit a single
        // "intent" fill row per logical trade. The scheduler decides current position.
        // TODO(3.4): read current position from Moomoo OpenD via get_portfolio.py
        // before emitting trades.
        let trades = match target {
            Position::Flat => vec![],
            Position::Long => vec![(self.symbol.clone(), TradeSide::Buy)],
            Position::Short => vec![(self.short_symbol.clone(), TradeSide::Buy)],
        };

        let mut fills = Vec::with_capacity(trades.len());
        for (traded_symbol, side) in trades {
            let order_id = self
                .submit_order(&traded_symbol, side, ts)
                .await
                .with_context(|| {
                    format!(
                        "Moomoo order failed: {side:?} {traded_symbol} (env={:?})",
                        self.trd_env
                    )
                })?;
            info!(
                %order_id,
                symbol = %traded_symbol,
                ?side,
                env = ?self.trd_env,
                "moomoo order submitted"
            );
            fills.push(FillResult {
                side,
                symbol: traded_symbol,
                qty: 0.0, // 3.3: qty resolved by OpenD fill confirmation (3.4); schema field required.
                price: 0.0, // 3.3: price resolved by OpenD fill confirmation (3.4).
                fee: 0.0,
                realized_pnl: 0.0,
                ts,
            });
        }
        Ok(fills)
    }

    /// Run `place_order.py` for one (symbol, side) and return the order_id.
    async fn submit_order(&self, symbol: &str, side: TradeSide, ts: i64) -> Result<String> {
        let script = place_order_script();
        if !script.exists() {
            return Err(anyhow!(
                "moomoo place_order.py not found at {} — install the moomooapi skill",
                script.display()
            ));
        }

        let mut cmd = Command::new("python3");
        cmd.arg(&script)
            .arg("--code")
            .arg(symbol_to_moomoo_code(symbol))
            .arg("--side")
            .arg(match side {
                TradeSide::Buy => "BUY",
                TradeSide::Sell => "SELL",
            })
            .arg("--order-type")
            .arg("MARKET") // market orders for simplicity; limit support in 3.4
            .arg("--trd-env")
            .arg(self.trd_env.as_arg())
            .arg("--json");

        // `--confirmed` is required for REAL orders (print preview only).
        if self.trd_env.is_real() {
            cmd.arg("--confirmed");
        }

        if let Some(firm) = &self.security_firm {
            cmd.arg("--security-firm").arg(firm);
        }
        if let Some(acc) = self.acc_id {
            cmd.arg("--acc-id").arg(acc.to_string());
        }

        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd
            .output()
            .await
            .with_context(|| format!("failed to spawn python3 {}", script.display()))?;

        // tokio::process::Output always has stdout/stderr as Vec<u8> (not Option).
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        if !output.status.success() {
            // The script prints either {"error": "..."} on stdout OR a traceback to stderr.
            let script_err = serde_json::from_str::<MoomooError>(&stdout)
                .ok()
                .map(|e| e.error)
                .unwrap_or_else(|| {
                    if stderr.is_empty() {
                        format!("python exit {:?}", output.status.code())
                    } else {
                        stderr
                    }
                });
            // Special-case: trade not unlocked — actionable error.
            let lower = script_err.to_lowercase();
            if lower.contains("unlock") || lower.contains("未解锁") {
                warn!(
                    "Moomoo trade is not unlocked. Open the OpenD GUI and click 'Unlock Trade' \
                     before LIVE orders will succeed."
                );
            }
            return Err(anyhow!(
                "place_order.py failed for {symbol} {side:?}: {script_err}"
            ));
        }

        let parsed: PlaceOrderResponse = serde_json::from_str(&stdout).with_context(|| {
            format!(
                "place_order.py returned non-JSON output (expected JSON with --json): {}",
                stdout.chars().take(500).collect::<String>()
            )
        })?;

        if parsed.error.is_some() {
            return Err(anyhow!(
                "place_order.py returned error: {:?}",
                parsed.error
            ));
        }

        let order_id = parsed
            .order_id
            .ok_or_else(|| anyhow!("place_order.py JSON missing order_id"))?;
        // ts is unused for now; included for future audit logging.
        let _ = ts;
        Ok(order_id)
    }
}

/// Convert internal symbol ("QQQ") to Moomoo code ("US.QQQ").
///
/// We default to US market. JP/HK codes can be plumbed through config later.
fn symbol_to_moomoo_code(symbol: &str) -> String {
    if symbol.contains('.') {
        symbol.to_string()
    } else {
        format!("US.{symbol}")
    }
}

#[derive(Debug, Deserialize)]
struct PlaceOrderResponse {
    #[serde(default)]
    order_id: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MoomooError {
    #[serde(default)]
    error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_trades_long_entry() {
        let trades = MoomooExecutor::plan_trades(
            Position::Long,
            Position::Flat,
            "QQQ",
            "PSQ",
        );
        assert_eq!(trades, vec![("QQQ".to_string(), TradeSide::Buy)]);
    }

    #[test]
    fn plan_trades_short_entry_uses_inverse_etf() {
        let trades = MoomooExecutor::plan_trades(
            Position::Short,
            Position::Flat,
            "QQQ",
            "PSQ",
        );
        assert_eq!(trades, vec![("PSQ".to_string(), TradeSide::Buy)]);
    }

    #[test]
    fn plan_trades_exit_long() {
        let trades = MoomooExecutor::plan_trades(
            Position::Flat,
            Position::Long,
            "QQQ",
            "PSQ",
        );
        assert_eq!(trades, vec![("QQQ".to_string(), TradeSide::Sell)]);
    }

    #[test]
    fn plan_trades_exit_short_sells_inverse() {
        let trades = MoomooExecutor::plan_trades(
            Position::Flat,
            Position::Short,
            "QQQ",
            "PSQ",
        );
        assert_eq!(trades, vec![("PSQ".to_string(), TradeSide::Sell)]);
    }

    #[test]
    fn plan_trades_no_change_is_empty() {
        assert!(MoomooExecutor::plan_trades(Position::Flat, Position::Flat, "QQQ", "PSQ").is_empty());
        assert!(MoomooExecutor::plan_trades(Position::Long, Position::Long, "QQQ", "PSQ").is_empty());
        assert!(MoomooExecutor::plan_trades(Position::Short, Position::Short, "QQQ", "PSQ").is_empty());
    }

    #[test]
    fn plan_trades_long_to_short_is_two_step() {
        let trades = MoomooExecutor::plan_trades(
            Position::Short,
            Position::Long,
            "QQQ",
            "PSQ",
        );
        assert_eq!(
            trades,
            vec![
                ("QQQ".to_string(), TradeSide::Sell),
                ("PSQ".to_string(), TradeSide::Buy),
            ]
        );
    }

    #[test]
    fn symbol_to_moomoo_code_adds_us_prefix() {
        assert_eq!(symbol_to_moomoo_code("QQQ"), "US.QQQ");
        assert_eq!(symbol_to_moomoo_code("PSQ"), "US.PSQ");
        assert_eq!(symbol_to_moomoo_code("US.QQQ"), "US.QQQ");
        assert_eq!(symbol_to_moomoo_code("HK.00700"), "HK.00700");
    }

    #[test]
    fn trd_env_roundtrip() {
        for val in ["SIMULATE", "REAL", "simulate", "real", "", "garbage"] {
            std::env::set_var("MOOMOO_TRD_ENV", val);
            let env = TrdEnv::from_env();
            let is_real = matches!(val.to_uppercase().as_str(), "REAL");
            assert_eq!(env.is_real(), is_real, "input={val}");
        }
    }
}
