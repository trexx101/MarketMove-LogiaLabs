//! AI Trading Advisor — daily pre-market briefing + conversational chat.
//!
//! Phase 4: Generates a prose digest of features, predictions, sentiment, and
//! macro context once per trading day before market open (default 13:00 UTC).
//! Also answers conversational follow-up questions with multi-turn history
//! streamed via SSE.
//!
//! The advisor is strictly advisory — zero execution authority. All parameter
//! suggestions must be backtested before applying.

pub mod briefing;
pub mod chat;
pub mod context;
pub mod prompt;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::config::Config;

// ── public types ────────────────────────────────────────────────────

/// System state snapshot sent to the LLM as context.
#[derive(Debug, Clone, Serialize)]
pub struct AdvisorContext {
    pub as_of: DateTime<Utc>,
    pub symbol: String,
    #[serde(rename = "market_session")]
    pub market_session: String,
    pub next_open_utc: Option<i64>,
    pub next_close_utc: Option<i64>,
    pub is_trading_day: bool,
    pub holiday_name: Option<String>,

    // Predictions (from equity_predictions)
    pub pred_1d: Option<f64>,
    pub pred_5d: Option<f64>,
    pub pred_21d: Option<f64>,
    pub pred_ts: Option<i64>,

    // Live feature vector (same names as FeatureInspector.svelte)
    pub features: FeatureSnapshot,

    // Sentiment
    pub sentiment_score: Option<f64>,
    pub sentiment_buzz: Option<i64>,
    pub sentiment_source: String,

    // Macro / calendar
    pub macro_ctx: MacroSnapshot,

    // Position state
    pub position_side: String,
    pub entry_price: Option<f64>,
    pub entry_ts: Option<i64>,
    pub unrealized_pnl: Option<f64>,
    pub realized_pnl_session: Option<f64>,

    // Recent closed trades (last 5)
    pub recent_trades: Vec<RecentTrade>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureSnapshot {
    pub trend_slope: f64,
    pub trend_adx: f64,
    pub rsi_14: f64,
    pub vix_regime: f64,
    pub tlt_corr_20d: f64,
    pub rvol_20d: f64,
    pub gap_pct: f64,
    pub drawdown_from_50d_high: f64,
    /// Seconds since the feature row was written. If >7200 (2h) during
    /// Regular market hours, the prompt flags staleness.
    #[serde(skip)]
    pub staleness_secs: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MacroSnapshot {
    pub ust_10y_latest: Option<f64>,
    pub ust_10y_prev: Option<f64>,
    pub dxy_latest: Option<f64>,
    pub dxy_prev: Option<f64>,
    pub vix_latest: Option<f64>,
    pub earnings_in_next_7d: Vec<EarningsEvent>,
    pub macro_releases_in_next_7d: Vec<MacroRelease>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EarningsEvent {
    pub date: String,
    pub hour_et: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MacroRelease {
    pub date: String,
    pub event: String,
    pub time_et: Option<String>,
    pub consensus: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentTrade {
    pub side: String,
    pub entry_ts: i64,
    pub exit_ts: i64,
    pub entry_price: f64,
    pub exit_price: f64,
    pub pnl: f64,
    pub bars_held: i64,
}

// ── briefing output ────────────────────────────────────────────────

/// The parsed LLM response. Always comes from a fenced JSON block.
/// On parse failure, `parse_status = "failed"` and `parse_error` is set
/// — NEVER silently substituted as a fake "hold".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorBriefing {
    pub model_used: String,
    pub as_of: DateTime<Utc>,
    /// The trading day this briefing covers (YYYY-MM-DD).
    pub for_date: String,
    /// Full prose digest (all sections concatenated, markdown).
    pub digest: String,
    /// Section-by-section prose. Every field is non-empty on a valid parse.
    pub sections: BriefingSections,
    /// Warnings surfaced separately so the UI can render them as banners.
    pub warnings: Vec<String>,
    /// Optional structured metadata the LLM may include.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_params: Option<serde_json::Value>,
    pub parse_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BriefingSections {
    pub regime: String,
    pub predictions: String,
    pub features: String,
    pub sentiment: String,
    pub macro_section: String,
    pub position_advice: String,
}

// ── error types ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum BriefingError {
    ParseFailed {
        raw: String,
        reason: String,
    },
    LlmUnavailable {
        status: u16,
        body: String,
    },
    Disabled(String),
}

impl std::fmt::Display for BriefingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BriefingError::ParseFailed { reason, .. } => write!(f, "parse failed: {reason}"),
            BriefingError::LlmUnavailable { status, body } => {
                write!(f, "LLM unavailable ({status}): {body}")
            }
            BriefingError::Disabled(reason) => write!(f, "disabled: {reason}"),
        }
    }
}

impl std::error::Error for BriefingError {}

// ── advisor state ───────────────────────────────────────────────────

/// Process-wide advisor state shared by the briefing loop and API handlers.
pub struct AdvisorState {
    pub cfg: AdvisorConfig,
    pub cache: Arc<RwLock<HashMap<String, CachedBriefing>>>,
    /// Signal from the scheduler when predictions update.
    pub notify: Arc<Notify>,
    /// WS broadcast channel for TelemetryEvent::AdvisorBriefing.
    pub tx: tokio::sync::broadcast::Sender<crate::api::ws::TelemetryEvent>,
}

#[derive(Debug, Clone)]
pub struct CachedBriefing {
    pub briefing: AdvisorBriefing,
    pub context_hash: String,
}

#[derive(Debug, Clone)]
pub struct AdvisorConfig {
    pub enabled: bool,
    pub api_key: String,
    pub model: String,
    pub api_base: String,
    pub fallback_model: String,
    pub briefing_hour_utc: u32,
    pub chat_history_turns: usize,
    pub chat_rate_limit_per_min: u32,
    pub cache_ttl_seconds: u64,
    pub request_timeout_seconds: u64,
}

impl AdvisorConfig {
    /// Build from `Config` + environment. All fields optional; defaults are
    /// safe for a disabled state (empty api_key).
    pub fn from_env(cfg: &Config) -> Self {
        let openrouter_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();

        Self {
            enabled: openrouter_key.trim().is_empty() == false,
            api_key: openrouter_key.trim().to_string(),
            model: std::env::var("ADVISOR_MODEL")
                .unwrap_or_else(|_| "google/gemini-3.1-pro-preview".to_string()),
            api_base: std::env::var("ADVISOR_API_BASE")
                .unwrap_or_else(|_| "https://openrouter.ai/api/v1/chat/completions".to_string()),
            fallback_model: std::env::var("ADVISOR_FALLBACK_MODEL")
                .unwrap_or_else(|_| "anthropic/claude-3.5-sonnet".to_string()),
            briefing_hour_utc: std::env::var("ADVISOR_BRIEFING_HOUR_UTC")
                .unwrap_or_else(|_| "13".to_string())
                .parse()
                .unwrap_or(13),
            chat_history_turns: std::env::var("ADVISOR_CHAT_HISTORY_TURNS")
                .unwrap_or_else(|_| "5".to_string())
                .parse()
                .unwrap_or(5),
            chat_rate_limit_per_min: std::env::var("ADVISOR_CHAT_RATE_LIMIT_PER_MIN")
                .unwrap_or_else(|_| "10".to_string())
                .parse()
                .unwrap_or(10),
            cache_ttl_seconds: std::env::var("ADVISOR_CACHE_TTL_SECONDS")
                .unwrap_or_else(|_| "21600".to_string()) // 6h
                .parse()
                .unwrap_or(21600),
            request_timeout_seconds: std::env::var("ADVISOR_REQUEST_TIMEOUT")
                .unwrap_or_else(|_| "15".to_string())
                .parse()
                .unwrap_or(15),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled && !self.api_key.is_empty()
    }
}

impl AdvisorState {
    pub fn new(
        cfg: AdvisorConfig,
        tx: tokio::sync::broadcast::Sender<crate::api::ws::TelemetryEvent>,
    ) -> Self {
        Self {
            cfg,
            cache: Arc::new(RwLock::new(HashMap::new())),
            notify: Arc::new(Notify::new()),
            tx,
        }
    }

    /// Read cached briefing for a date. Returns None if absent or stale.
    pub fn cached(&self, for_date: &str) -> Option<AdvisorBriefing> {
        let cache = self.cache.read().ok()?;
        cache.get(for_date).map(|cb| cb.briefing.clone())
    }

    /// Write a briefing to the cache.
    pub fn cache_briefing(&self, for_date: &str, briefing: AdvisorBriefing, context_hash: String) {
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(
                for_date.to_string(),
                CachedBriefing {
                    briefing,
                    context_hash,
                },
            );
        }
    }

    /// Signal the briefing loop to wake and regenerate for today.
    pub fn refresh(&self) {
        self.notify.notify_one();
    }
}

/// Hash the canonicalized AdvisorContext for cache dedup.
pub fn hash_context(ctx: &AdvisorContext) -> String {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_string(ctx).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}