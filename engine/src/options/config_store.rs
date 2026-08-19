//! Options config store — DB-backed, UI-editable configuration (Phase 7)
//!
//! Replaces hardcoded literals across options modules with a single registry.
//! Every key carries:
//! - a **tier**: `strategy` (free to tune) or `rail` (risk rail — editable but
//!   bounded; changes are event-logged per D15/D16: rails can never be
//!   disabled, only tuned within bounds)
//! - a kind, default, and min/max bounds — out-of-bounds writes are rejected.
//!
//! Modules read values at decision time via [`OptionsConfigStore::get_f64`] /
//! [`get_i64`]; missing keys fall back to registry defaults.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::db::DbPool;

/// Editability/risk tier of a config key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigTier {
    /// Free to tune from the UI.
    Strategy,
    /// Risk rail. Editable but bounded; every change is event-logged.
    Rail,
}

impl ConfigTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfigTier::Strategy => "strategy",
            ConfigTier::Rail => "rail",
        }
    }

    pub fn parse(s: &str) -> ConfigTier {
        match s {
            "rail" => ConfigTier::Rail,
            _ => ConfigTier::Strategy,
        }
    }
}

/// Value kind for validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKind {
    F64,
    I64,
}

/// Static registry entry for one config key.
#[derive(Debug, Clone)]
pub struct ConfigSpec {
    pub key: &'static str,
    pub tier: ConfigTier,
    pub kind: ConfigKind,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    pub label: &'static str,
    pub description: &'static str,
}

/// The full registry of externalized options settings.
///
/// Order matters only for stable UI listing (grouped by section).
pub fn registry() -> Vec<ConfigSpec> {
    vec![
        // ── Sizing / deployment (strategy) ────────────────────────────────
        ConfigSpec {
            key: "risk_pct", tier: ConfigTier::Strategy, kind: ConfigKind::F64,
            default: 0.01, min: 0.001, max: 0.05,
            label: "Risk per trade (%)",
            description: "Fraction of account equity risked per position. Maps to contracts, never set directly.",
        },
        ConfigSpec {
            key: "max_premium_pct", tier: ConfigTier::Strategy, kind: ConfigKind::F64,
            default: 0.05, min: 0.01, max: 0.25,
            label: "Max premium (% of equity)",
            description: "Maximum premium deployed per position as fraction of equity.",
        },
        ConfigSpec {
            key: "deployed_cap_pct", tier: ConfigTier::Strategy, kind: ConfigKind::F64,
            default: 0.25, min: 0.05, max: 1.0,
            label: "Deployed capital cap (%)",
            description: "Maximum fraction of equity deployed across all options positions.",
        },
        ConfigSpec {
            key: "contracts_cap", tier: ConfigTier::Strategy, kind: ConfigKind::I64,
            default: 10.0, min: 1.0, max: 50.0,
            label: "Contracts cap",
            description: "Hard cap on contracts per position regardless of sizing math.",
        },
        ConfigSpec {
            key: "positions_per_underlying", tier: ConfigTier::Strategy, kind: ConfigKind::I64,
            default: 1.0, min: 1.0, max: 3.0,
            label: "Positions per underlying",
            description: "Maximum concurrent positions per underlying.",
        },
        ConfigSpec {
            key: "max_positions", tier: ConfigTier::Strategy, kind: ConfigKind::I64,
            default: 3.0, min: 1.0, max: 10.0,
            label: "Max total positions",
            description: "Maximum concurrent positions across all underlyings.",
        },
        // ── Chain selection (strategy) ────────────────────────────────────
        ConfigSpec {
            key: "dte_min", tier: ConfigTier::Strategy, kind: ConfigKind::I64,
            default: 30.0, min: 7.0, max: 60.0,
            label: "Min DTE at entry",
            description: "Minimum days to expiry for entry chain selection.",
        },
        ConfigSpec {
            key: "dte_max", tier: ConfigTier::Strategy, kind: ConfigKind::I64,
            default: 45.0, min: 7.0, max: 120.0,
            label: "Max DTE at entry",
            description: "Maximum days to expiry for entry chain selection.",
        },
        ConfigSpec {
            key: "delta_target", tier: ConfigTier::Strategy, kind: ConfigKind::F64,
            default: 0.45, min: 0.20, max: 0.70,
            label: "Target delta",
            description: "Target option delta for chain selection (0.40–0.50 per D-table).",
        },
        // ── Exit behaviour (strategy) ─────────────────────────────────────
        ConfigSpec {
            key: "trail_pct", tier: ConfigTier::Strategy, kind: ConfigKind::F64,
            default: 0.05, min: 0.01, max: 0.20,
            label: "Trailing stop (%)",
            description: "Trailing stop distance as fraction of premium price.",
        },
        ConfigSpec {
            key: "trail_rearm_band_atr", tier: ConfigTier::Strategy, kind: ConfigKind::F64,
            default: 0.5, min: 0.1, max: 2.0,
            label: "Trail re-arm band (×ATR)",
            description: "Price must recover by this multiple of ATR before trailing stop re-arms.",
        },
        ConfigSpec {
            key: "cooldown_seconds", tier: ConfigTier::Strategy, kind: ConfigKind::I64,
            default: 900.0, min: 60.0, max: 3600.0,
            label: "Breaker cooldown (s)",
            description: "Entry halt duration after circuit breaker trip.",
        },
        ConfigSpec {
            key: "vix_slope_threshold", tier: ConfigTier::Rail, kind: ConfigKind::F64,
            default: 0.5, min: 0.1, max: 3.0,
            label: "VIX slope threshold (per day)",
            description: "Rail: VIX rising faster than this per day blocks entries.",
        },
        ConfigSpec {
            key: "delta_recheck_band", tier: ConfigTier::Rail, kind: ConfigKind::F64,
            default: 0.05, min: 0.01, max: 0.20,
            label: "Delta re-check band (D11)",
            description: "Rail: re-check delta at exit order time; deviation beyond band aborts the exit leg.",
        },
        // ── Execution ladder timers (P3.10, default unchanged) ────────────
        ConfigSpec {
            key: "exit_stage1_secs", tier: ConfigTier::Strategy, kind: ConfigKind::I64,
            default: 3.0, min: 1.0, max: 30.0,
            label: "Exit stage 1 timer (s)",
            description: "Stage 1 (BID + k×tick) duration before escalating to stage 2.",
        },
        ConfigSpec {
            key: "exit_stage2_secs", tier: ConfigTier::Strategy, kind: ConfigKind::I64,
            default: 3.0, min: 1.0, max: 30.0,
            label: "Exit stage 2 timer (s)",
            description: "Stage 2 (BID + 2k×tick) duration before escalating to stage 3.",
        },
        ConfigSpec {
            key: "exit_stage3_secs", tier: ConfigTier::Strategy, kind: ConfigKind::I64,
            default: 10.0, min: 3.0, max: 60.0,
            label: "Exit stage 3 timer (s)",
            description: "Stage 3 (MARKET fallback) duration before declaring the exit failed.",
        },
        ConfigSpec {
            key: "entry_stage1_secs", tier: ConfigTier::Strategy, kind: ConfigKind::I64,
            default: 3.0, min: 1.0, max: 30.0,
            label: "Entry stage 1 timer (s)",
            description: "Entry stage 1 (BID) duration before escalating.",
        },
        ConfigSpec {
            key: "entry_stage2_secs", tier: ConfigTier::Strategy, kind: ConfigKind::I64,
            default: 10.0, min: 3.0, max: 60.0,
            label: "Entry stage 2 timer (s)",
            description: "Entry stage 2 (BID + k×tick) duration before cancelling.",
        },
        // ── Risk rails (D15/D16 — bounded, event-logged) ──────────────────
        ConfigSpec {
            key: "dte_exit_min", tier: ConfigTier::Rail, kind: ConfigKind::I64,
            default: 7.0, min: 1.0, max: 21.0,
            label: "Forced exit DTE",
            description: "Rail: positions below this DTE are force-exited. Cannot be disabled.",
        },
        ConfigSpec {
            key: "delta_drift_min", tier: ConfigTier::Rail, kind: ConfigKind::F64,
            default: 0.15, min: 0.05, max: 0.40,
            label: "Delta drift lower band",
            description: "Rail: delta below this triggers forced exit.",
        },
        ConfigSpec {
            key: "delta_drift_max", tier: ConfigTier::Rail, kind: ConfigKind::F64,
            default: 0.70, min: 0.40, max: 0.95,
            label: "Delta drift upper band",
            description: "Rail: delta above this triggers forced exit.",
        },
        ConfigSpec {
            key: "earnings_blackout_days", tier: ConfigTier::Rail, kind: ConfigKind::I64,
            default: 2.0, min: 0.0, max: 7.0,
            label: "Earnings blackout (days)",
            description: "Rail: no entries within this many days of earnings.",
        },
        ConfigSpec {
            key: "iv_spike_multiplier", tier: ConfigTier::Rail, kind: ConfigKind::F64,
            default: 2.0, min: 1.2, max: 5.0,
            label: "IV spike multiplier",
            description: "Rail: IV above baseline × this trips the volatility breaker.",
        },
        ConfigSpec {
            key: "max_consecutive_losses", tier: ConfigTier::Rail, kind: ConfigKind::I64,
            default: 4.0, min: 1.0, max: 10.0,
            label: "Max consecutive losses",
            description: "Rail: consecutive losing trades before circuit breaker trips.",
        },
        ConfigSpec {
            key: "blackout_hours", tier: ConfigTier::Rail, kind: ConfigKind::F64,
            default: 24.0, min: 1.0, max: 72.0,
            label: "Macro blackout window (h)",
            description: "Rail: entry blackout hours before FOMC/CPI/NFP events.",
        },
        // ── Liquidity rails (D18) ─────────────────────────────────────────
        ConfigSpec {
            key: "bid_min", tier: ConfigTier::Rail, kind: ConfigKind::F64,
            default: 0.01, min: 0.01, max: 1.0,
            label: "Min bid price",
            description: "Rail: contracts below this bid are illiquid and skipped.",
        },
        ConfigSpec {
            key: "spread_cap_pct", tier: ConfigTier::Rail, kind: ConfigKind::F64,
            default: 0.08, min: 0.01, max: 0.50,
            label: "Max spread (% of mid)",
            description: "Rail: contracts with wider spread/mid ratio are skipped.",
        },
        ConfigSpec {
            key: "oi_min", tier: ConfigTier::Rail, kind: ConfigKind::I64,
            default: 100.0, min: 0.0, max: 10000.0,
            label: "Min open interest",
            description: "Rail: contracts below this OI are skipped.",
        },
        ConfigSpec {
            key: "slippage_multiplier", tier: ConfigTier::Rail, kind: ConfigKind::F64,
            default: 1.0, min: 0.0, max: 5.0,
            label: "Slippage budget (×spread)",
            description: "Rail: exit slippage budget as multiplier of entry-time spread.",
        },
        ConfigSpec {
            key: "slippage_premium_cap_pct", tier: ConfigTier::Rail, kind: ConfigKind::F64,
            default: 0.05, min: 0.01, max: 0.25,
            label: "Slippage cap (% of premium)",
            description: "Rail: absolute slippage cap as fraction of premium.",
        },
        ConfigSpec {
            key: "vix_level_gate", tier: ConfigTier::Rail, kind: ConfigKind::F64,
            default: 30.0, min: 15.0, max: 60.0,
            label: "VIX level gate",
            description: "Rail: entries blocked when VIX exceeds this level.",
        },
        ConfigSpec {
            key: "vix_slope_window", tier: ConfigTier::Rail, kind: ConfigKind::I64,
            default: 5.0, min: 1.0, max: 20.0,
            label: "VIX slope window (days)",
            description: "Rail: lookback window for the VIX 5-day slope gate.",
        },
        ConfigSpec {
            key: "exit_stage1_secs", tier: ConfigTier::Rail, kind: ConfigKind::I64,
            default: 3.0, min: 1.0, max: 60.0,
            label: "Exit Stage 1 timer (s)",
            description: "Rail: seconds to hold the Stage 1 exit order (BID + k×tick) before degrading.",
        },
        ConfigSpec {
            key: "exit_stage2_secs", tier: ConfigTier::Rail, kind: ConfigKind::I64,
            default: 3.0, min: 1.0, max: 60.0,
            label: "Exit Stage 2 timer (s)",
            description: "Rail: seconds to hold the Stage 2 exit order (BID) before degrading.",
        },
        ConfigSpec {
            key: "exit_stage3_secs", tier: ConfigTier::Rail, kind: ConfigKind::I64,
            default: 10.0, min: 1.0, max: 120.0,
            label: "Exit Stage 3 timer (s)",
            description: "Rail: seconds to hold the Stage 3 exit order (BID − slippage) before retrying.",
        },
    ]
}

/// One config entry as returned to the UI.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigEntry {
    pub key: String,
    pub value: f64,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    pub tier: ConfigTier,
    pub kind: ConfigKind,
    pub label: String,
    pub description: String,
    pub updated_at: Option<i64>,
}

impl Serialize for ConfigKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            ConfigKind::F64 => "float",
            ConfigKind::I64 => "int",
        })
    }
}

/// DB-backed options configuration store.
#[derive(Clone)]
pub struct OptionsConfigStore {
    pool: DbPool,
    mode: String, // "paper" | "live" — used for event attribution
}

impl OptionsConfigStore {
    pub fn new(pool: DbPool, mode: &str) -> Self {
        Self { pool, mode: mode.to_string() }
    }

    /// Get a spec from the registry by key.
    fn spec(key: &str) -> Result<ConfigSpec> {
        registry()
            .into_iter()
            .find(|s| s.key == key)
            .ok_or_else(|| anyhow::anyhow!("unknown options config key: {key}"))
    }

    /// Read a numeric value (DB override or registry default).
    pub async fn get_f64(&self, key: &str) -> Result<f64> {
        let spec = Self::spec(key)?;
        match crate::db::get_options_config(&self.pool, key).await? {
            Some(json) => {
                let v: serde_json::Value = serde_json::from_str(&json)?;
                Ok(v.as_f64().unwrap_or(spec.default))
            }
            None => Ok(spec.default),
        }
    }

    /// Read an integer value (DB override or registry default).
    pub async fn get_i64(&self, key: &str) -> Result<i64> {
        Ok(self.get_f64(key).await?.round() as i64)
    }

    /// Validate and write a value. Rejects unknown keys, wrong kinds, and
    /// out-of-bounds values. Rail changes are event-logged.
    pub async fn set(&self, key: &str, value: f64) -> Result<()> {
        let spec = Self::spec(key)?;

        // Kind check: ints must be integral
        if spec.kind == ConfigKind::I64 && value.fract() != 0.0 {
            bail!("config key {key} requires an integer value");
        }
        if value < spec.min || value > spec.max {
            bail!(
                "config key {key}: value {value} outside bounds [{}, {}]",
                spec.min,
                spec.max
            );
        }

        let json = if spec.kind == ConfigKind::I64 {
            format!("{}", value as i64)
        } else {
            serde_json::to_string(&value)?
        };
        crate::db::set_options_config(&self.pool, key, &json, spec.tier.as_str()).await?;

        // Rail changes are always event-logged (D15/D16 audit trail)
        if spec.tier == ConfigTier::Rail {
            let payload = serde_json::json!({
                "key": key,
                "value": value,
                "min": spec.min,
                "max": spec.max,
            });
            crate::db::insert_event(
                &self.pool,
                "strategy",
                "warn",
                &self.mode,
                "options::config",
                &format!("risk rail changed: {key} = {value}"),
                &payload.to_string(),
                None,
            )
            .await?;
        }

        Ok(())
    }

    /// List all registry entries with current values for the UI.
    pub async fn list(&self) -> Result<Vec<ConfigEntry>> {
        let stored: std::collections::HashMap<String, (String, i64)> =
            crate::db::list_options_config(&self.pool)
                .await?
                .into_iter()
                .map(|(k, v, _tier, ts)| (k, (v, ts)))
                .collect();

        Ok(registry()
            .into_iter()
            .map(|spec| {
                let (value, updated_at) = match stored.get(spec.key) {
                    Some((json, ts)) => {
                        let v: serde_json::Value =
                            serde_json::from_str(json).unwrap_or(serde_json::Value::Null);
                        (v.as_f64().unwrap_or(spec.default), Some(*ts))
                    }
                    None => (spec.default, None),
                };
                ConfigEntry {
                    key: spec.key.to_string(),
                    value,
                    default: spec.default,
                    min: spec.min,
                    max: spec.max,
                    tier: spec.tier,
                    kind: spec.kind,
                    label: spec.label.to_string(),
                    description: spec.description.to_string(),
                    updated_at,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_store() -> OptionsConfigStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            r#"CREATE TABLE options_config_kv (
                key TEXT PRIMARY KEY, value_json TEXT NOT NULL,
                tier TEXT NOT NULL DEFAULT 'strategy', updated_at INTEGER NOT NULL)"#,
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"CREATE TABLE engine_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL,
                category TEXT NOT NULL, severity TEXT NOT NULL, mode TEXT NOT NULL,
                source TEXT NOT NULL, message TEXT NOT NULL,
                payload_json TEXT NOT NULL DEFAULT '{}', equity TEXT)"#,
        )
        .execute(&pool)
        .await
        .unwrap();
        OptionsConfigStore::new(pool, "paper")
    }

    #[tokio::test]
    async fn defaults_returned_when_unset() {
        let store = test_store().await;
        assert!((store.get_f64("risk_pct").await.unwrap() - 0.01).abs() < 1e-12);
        assert_eq!(store.get_i64("dte_exit_min").await.unwrap(), 7);
        assert!((store.get_f64("delta_target").await.unwrap() - 0.45).abs() < 1e-12);
    }

    #[tokio::test]
    async fn unknown_key_rejected() {
        let store = test_store().await;
        assert!(store.get_f64("nonexistent").await.is_err());
        assert!(store.set("nonexistent", 1.0).await.is_err());
    }

    #[tokio::test]
    async fn set_and_get_roundtrip() {
        let store = test_store().await;
        store.set("risk_pct", 0.02).await.unwrap();
        assert!((store.get_f64("risk_pct").await.unwrap() - 0.02).abs() < 1e-12);
    }

    #[tokio::test]
    async fn out_of_bounds_rejected() {
        let store = test_store().await;
        assert!(store.set("risk_pct", 0.99).await.is_err()); // max 0.05
        assert!(store.set("risk_pct", 0.0001).await.is_err()); // min 0.001
        // Value unchanged after rejection
        assert!((store.get_f64("risk_pct").await.unwrap() - 0.01).abs() < 1e-12);
    }

    #[tokio::test]
    async fn rail_bounds_enforced() {
        let store = test_store().await;
        // dte_exit_min bounds are [1, 21] — cannot disable the rail (0 rejected)
        assert!(store.set("dte_exit_min", 0.0).await.is_err());
        // delta_drift_min <= delta_drift_max is preserved by disjoint bounds
        assert!(store.set("delta_drift_min", 0.5).await.is_err()); // max 0.40
        assert!(store.set("delta_drift_max", 0.3).await.is_err()); // min 0.40
    }

    #[tokio::test]
    async fn int_keys_reject_fractional() {
        let store = test_store().await;
        assert!(store.set("contracts_cap", 2.5).await.is_err());
        store.set("contracts_cap", 5.0).await.unwrap();
        assert_eq!(store.get_i64("contracts_cap").await.unwrap(), 5);
    }

    #[tokio::test]
    async fn rail_change_emits_event() {
        let store = test_store().await;
        store.set("dte_exit_min", 10.0).await.unwrap();

        let events = crate::db::search_events(&store.pool, Some("strategy"), None, None, None, None, 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].message.contains("dte_exit_min"));
        assert_eq!(events[0].severity, "warn");
    }

    #[tokio::test]
    async fn strategy_change_no_event() {
        let store = test_store().await;
        store.set("risk_pct", 0.02).await.unwrap();

        let events = crate::db::search_events(&store.pool, None, None, None, None, None, 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 0);
    }

    #[tokio::test]
    async fn list_includes_all_registry_keys_with_values() {
        let store = test_store().await;
        store.set("risk_pct", 0.03).await.unwrap();

        let entries = store.list().await.unwrap();
        assert_eq!(entries.len(), registry().len());

        let risk = entries.iter().find(|e| e.key == "risk_pct").unwrap();
        assert!((risk.value - 0.03).abs() < 1e-12);
        assert!(risk.updated_at.is_some());

        let dte = entries.iter().find(|e| e.key == "dte_exit_min").unwrap();
        assert_eq!(dte.tier, ConfigTier::Rail);
        assert!(dte.updated_at.is_none());
    }
}
