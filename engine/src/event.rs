//! Unified event logging — persists key engine actions to `engine_events`
//! and broadcasts them on the telemetry WebSocket channel.
//!
//! Each event is a structured record: category, severity, mode (paper/live),
//! source module, human-readable message, and a JSON payload. The
//! `EventLogger` dual-writes: INSERT into SQLite + broadcast on the
//! existing `TelemetrySender` channel.

use chrono::Utc;
use serde_json::Value as JsonValue;
use strum::{Display, EnumString};
use tracing::error;

use crate::api::ws::{TelemetryEvent, TelemetrySender};
use crate::config::TradingMode;
use crate::db::DbPool;

#[derive(Debug, Clone, Copy, Display, EnumString)]
#[strum(serialize_all = "lowercase")]
pub enum EventCategory {
    Trade,
    Data,
    System,
    Strategy,
    Alert,
    Advisor,
}

#[derive(Debug, Clone, Copy, Display, EnumString)]
#[strum(serialize_all = "lowercase")]
pub enum EventSeverity {
    Info,
    Warn,
    Error,
}

/// An event to be persisted and broadcast.
pub struct EngineEvent {
    pub category: EventCategory,
    pub severity: EventSeverity,
    pub source: &'static str,
    pub message: String,
    pub payload: JsonValue,
}

impl EngineEvent {
    pub fn trade_fill(side: &str, symbol: &str, qty: f64, price: f64, fee: f64, pnl: f64) -> Self {
        Self {
            category: EventCategory::Trade,
            severity: EventSeverity::Info,
            source: "exec::paper",
            message: format!("{side} {symbol} @ {price:.2} (qty={qty:.2})"),
            payload: serde_json::json!({
                "side": side,
                "symbol": symbol,
                "qty": qty,
                "price": price,
                "fee": fee,
                "realized_pnl": pnl,
            }),
        }
    }

    pub fn data_fetch_failed(source: &'static str, symbol: &str, error: &str) -> Self {
        Self {
            category: EventCategory::Data,
            severity: EventSeverity::Error,
            source,
            message: format!("fetch failed for {symbol}: {error}"),
            payload: serde_json::json!({ "symbol": symbol, "error": error }),
        }
    }

    pub fn mode_changed(from: TradingMode, to: TradingMode, authorized_by: &str) -> Self {
        Self {
            category: EventCategory::System,
            severity: EventSeverity::Info,
            source: "api::mode",
            message: format!("mode changed: {from} → {to}"),
            payload: serde_json::json!({
                "from": format!("{from}"),
                "to": format!("{to}"),
                "authorized_by": authorized_by,
            }),
        }
    }

    pub fn strategy_config_changed(
        old: &crate::strategy::EquityStrategyParams,
        new: &crate::strategy::EquityStrategyParams,
    ) -> Self {
        Self {
            category: EventCategory::Strategy,
            severity: EventSeverity::Info,
            source: "api::strategy_config",
            message: "strategy config updated".to_string(),
            payload: serde_json::json!({ "old": old, "new": new }),
        }
    }

    pub fn prediction_persisted(pred_1d: f64, pred_5d: f64, pred_21d: f64, regime: &str) -> Self {
        Self {
            category: EventCategory::Strategy,
            severity: EventSeverity::Info,
            source: "scheduler",
            message: format!(
                "prediction persisted: 1d={pred_1d:.4}, 5d={pred_5d:.4}, 21d={pred_21d:.4}, regime={regime}"
            ),
            payload: serde_json::json!({
                "pred_1d": pred_1d,
                "pred_5d": pred_5d,
                "pred_21d": pred_21d,
                "regime": regime,
            }),
        }
    }

    pub fn staleness_alert(last_ts: Option<i64>, secs: i64) -> Self {
        Self {
            category: EventCategory::Alert,
            severity: EventSeverity::Warn,
            source: "scheduler",
            message: format!("staleness alert: {secs}s since last candle"),
            payload: serde_json::json!({
                "last_candle_ts": last_ts,
                "seconds_since_last": secs,
            }),
        }
    }

    pub fn advisor_briefing(for_date: &str, model: &str, latency_ms: u64) -> Self {
        Self {
            category: EventCategory::Advisor,
            severity: EventSeverity::Info,
            source: "advisor",
            message: format!("briefing generated for {for_date} via {model}"),
            payload: serde_json::json!({
                "for_date": for_date,
                "model": model,
                "latency_ms": latency_ms,
            }),
        }
    }

    pub fn engine_started(mode: TradingMode, symbol: &str) -> Self {
        Self {
            category: EventCategory::System,
            severity: EventSeverity::Info,
            source: "main",
            message: format!("engine started in {mode} mode for {symbol}"),
            payload: serde_json::json!({
                "mode": format!("{mode}"),
                "symbol": symbol,
            }),
        }
    }

    pub fn backtest_completed(strategy_id: &str, cagr: f64, sharpe: f64) -> Self {
        Self {
            category: EventCategory::Strategy,
            severity: EventSeverity::Info,
            source: "strategy_lab",
            message: format!(
                "backtest completed for {strategy_id}: CAGR={cagr:.1}%, Sharpe={sharpe:.2}"
            ),
            payload: serde_json::json!({
                "strategy_id": strategy_id,
                "cagr": cagr,
                "sharpe": sharpe,
            }),
        }
    }
}

/// Handles persistence + broadcast of engine events.
///
/// Holds a clone of the DB pool, an optional telemetry sender, and a
/// shared reference to the current trading mode (so events are tagged
/// with the mode at emission time, not at construction time).
pub struct EventLogger {
    pool: DbPool,
    tx: Option<TelemetrySender>,
    mode: std::sync::Arc<tokio::sync::RwLock<TradingMode>>,
}

impl EventLogger {
    pub fn new(
        pool: DbPool,
        tx: Option<TelemetrySender>,
        mode: std::sync::Arc<tokio::sync::RwLock<TradingMode>>,
    ) -> Self {
        Self { pool, tx, mode }
    }

    /// Persist the event to `engine_events` and broadcast on the telemetry channel.
    ///
    /// Errors are logged but never propagated — event logging is best-effort
    /// and must not break the engine's main loop.
    pub async fn emit(&self, event: EngineEvent) {
        let mode = *self.mode.read().await;
        let mode_str = match mode {
            TradingMode::Paper => "paper",
            TradingMode::Live => "live",
        };
        let ts = Utc::now().timestamp();
        let category = event.category.to_string();
        let severity = event.severity.to_string();
        let payload = event.payload.to_string();

        // Persist
        let res = sqlx::query(
            r#"INSERT INTO engine_events (ts, category, severity, mode, source, message, payload_json)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
        )
        .bind(ts)
        .bind(&category)
        .bind(&severity)
        .bind(mode_str)
        .bind(event.source)
        .bind(&event.message)
        .bind(&payload)
        .execute(&self.pool)
        .await;

        if let Err(e) = res {
            error!(error = %e, "failed to persist engine event");
        }

        // Broadcast
        if let Some(tx) = &self.tx {
            let _ = tx.send(TelemetryEvent::EngineEvent {
                ts,
                category,
                severity,
                mode: mode_str.to_string(),
                source: event.source.to_string(),
                message: event.message.clone(),
                payload: event.payload.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> DbPool {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for stmt in crate::db::DDL.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query(stmt).execute(&pool).await.unwrap();
        }
        pool
    }

    fn test_mode() -> std::sync::Arc<tokio::sync::RwLock<TradingMode>> {
        std::sync::Arc::new(tokio::sync::RwLock::new(TradingMode::Paper))
    }

    #[tokio::test]
    async fn emit_persists_to_db() {
        let pool = test_pool().await;
        let logger = EventLogger::new(pool.clone(), None, test_mode());

        logger
            .emit(EngineEvent::engine_started(TradingMode::Paper, "QQQ"))
            .await;

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM engine_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);

        let row: (String, String, String, String) =
            sqlx::query_as("SELECT category, severity, mode, source FROM engine_events")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.0, "system");
        assert_eq!(row.1, "info");
        assert_eq!(row.2, "paper");
        assert_eq!(row.3, "main");
    }

    #[tokio::test]
    async fn emit_trade_fill_persists_payload() {
        let pool = test_pool().await;
        let logger = EventLogger::new(pool.clone(), None, test_mode());

        logger
            .emit(EngineEvent::trade_fill("buy", "QQQ", 1.0, 500.0, 0.75, 0.0))
            .await;

        let row: (String, String) =
            sqlx::query_as("SELECT category, payload_json FROM engine_events")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.0, "trade");
        let payload: JsonValue = serde_json::from_str(&row.1).unwrap();
        assert_eq!(payload["side"], "buy");
        assert_eq!(payload["symbol"], "QQQ");
        assert_eq!(payload["price"], 500.0);
    }

    #[tokio::test]
    async fn emit_broadcasts_on_channel() {
        let pool = test_pool().await;
        let (tx, mut rx) = tokio::sync::broadcast::channel(64);
        let logger = EventLogger::new(pool, Some(tx), test_mode());

        logger
            .emit(EngineEvent::staleness_alert(Some(1000), 300))
            .await;

        let event = rx.recv().await.unwrap();
        match event {
            TelemetryEvent::EngineEvent {
                category, severity, ..
            } => {
                assert_eq!(category, "alert");
                assert_eq!(severity, "warn");
            }
            _ => panic!("expected EngineEvent variant"),
        }
    }
}
