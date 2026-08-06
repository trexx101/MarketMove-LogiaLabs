//! WebSocket telemetry backend.
//!
//! Broadcast channel plumbing for live control-room events. The scheduler
//! and paper executor publish `TelemetryEvent` values onto a
//! `tokio::sync::broadcast` channel; the WebSocket handler subscribes and
//! forwards each event as a JSON message to connected clients.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tracing::error;

use super::AppState;

/// A telemetry event broadcast over the WebSocket `/api/v1/ws` channel.
///
/// Serialized as JSON with an internally-tagged `type` field so clients can
/// discriminate variants cheaply without inspecting the payload shape.
///
/// ## §8 multi-model attribution
///
/// Variants that carry per-model data include `model_id` and `pair`
/// fields so the frontend can partition incoming events by model without
/// inferring from any other field. Variants that are inherently global
/// (mode toggle, strategy config, advisor briefing, generic engine event)
/// keep their existing shape — those either happen once per session or
/// already carry their own routing metadata in the `payload`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TelemetryEvent {
    /// Periodic or post-trade PnL snapshot for a specific model.
    PnlTick {
        model_id: String,
        pair: String,
        realized_pnl: f64,
        unrealized_pnl: f64,
        position: String,
        entry_price: Option<f64>,
        last_close: Option<f64>,
        timestamp: i64,
    },
    /// Fresh model prediction (1d / 5d / 21d) after a candle is finalized.
    PredictionUpdate {
        model_id: String,
        pair: String,
        pred_1d: Option<f64>,
        pred_5d: Option<f64>,
        pred_21d: Option<f64>,
        timestamp: i64,
    },
    /// Raw + normalized feature window sent to the inference service.
    FeatureUpdate {
        model_id: String,
        pair: String,
        features: Vec<f64>,
        normalized: Vec<f64>,
        timestamp: i64,
    },
    /// A single executed trade fill (entry or exit leg) for a model.
    /// `symbol` is the actual instrument (e.g. "QQQ" or "PSQ"); `pair`
    /// is the canonical "<PRIMARY>/<INVERSE>" label.
    TradeFill {
        model_id: String,
        pair: String,
        side: String,
        symbol: String,
        qty: f64,
        price: f64,
        fee: f64,
        realized_pnl: f64,
        timestamp: i64,
    },
    /// Trading mode transition (paper <-> live). Global event; no model.
    ModeChange {
        mode: String,
        timestamp: i64,
    },
    /// Stale-data alert fired by a per-model scheduler.
    StalenessAlert {
        model_id: String,
        pair: String,
        last_candle_ts: Option<i64>,
        seconds_since_last: i64,
    },
    /// Strategy config changed via PUT /api/strategy-config. Global event;
    /// per-model strategy configs land in a future story.
    StrategyConfigChange {
        entry_threshold: f64,
        exit_threshold: f64,
        sma_window: usize,
        pred_5d_filter: bool,
        enable_shorting: bool,
        short_entry_threshold: f64,
        short_exit_threshold: f64,
        enable_sentiment_overlay: bool,
        sentiment_reduce_threshold: f64,
        sentiment_exit_threshold: f64,
        sentiment_min_articles: i64,
    },
    /// Advisor briefing generated (Phase 4). Pushed to dashboard on completion.
    AdvisorBriefing {
        for_date: String,
        /// The parsed briefing struct. Serialized inline.
        briefing: crate::advisor::AdvisorBriefing,
    },
    /// Unified engine event for the Events page. Persisted to `engine_events`
    /// and broadcast for real-time display. Per-model attribution lives in
    /// the JSON `payload` (set via `EventLogger::emit_for_model`) so the
    /// schema stays shape-stable.
    EngineEvent {
        ts: i64,
        category: String,
        severity: String,
        mode: String,
        source: String,
        message: String,
        payload: serde_json::Value,
    },
}

/// Sender half of the telemetry broadcast channel.
///
/// Held by `AppState`, the scheduler, and the paper executor. Cloning is
/// cheap (broadcast senders are `Arc`-backed). Consumers that don't have
/// a channel (tests) pass `None`.
pub type TelemetrySender = tokio::sync::broadcast::Sender<TelemetryEvent>;

/// Axum WebSocket upgrade handler.
///
/// Subscribes to the broadcast channel and forwards each event to the
/// client as a JSON text message. Silently ignores dropped subscribers.
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.tx.subscribe();
    ws.on_upgrade(move |socket| forward_events(socket, rx))
}

/// Forward broadcast events to a connected WebSocket client until either
/// the client disconnects or the broadcast channel closes.
async fn forward_events(socket: WebSocket, mut rx: tokio::sync::broadcast::Receiver<TelemetryEvent>) {
    let (mut sender, mut receiver) = socket.split();

    // Spawn the outbound broadcast → ws pump.
    let mut send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    match serde_json::to_string(&event) {
                        Ok(json) => {
                            if sender.send(Message::Text(json)).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "failed to serialize telemetry event");
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "telemetry subscriber lagged; skipping stale events");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
        let _ = sender.send(Message::Close(None)).await;
    });

    // Receive loop — wait for the client to close the socket.
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if matches!(msg, Message::Close(_)) {
                break;
            }
        }
    });

    // If either half finishes, cancel the other and exit.
    tokio::select! {
        _ = &mut send_task => { recv_task.abort(); }
        _ = &mut recv_task => { send_task.abort(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `tag = "type"` attribute must produce a JSON object whose
    /// `"type"` discriminant matches the variant name.
    #[test]
    fn telemetry_event_serialization_includes_type_tag() {
        let event = TelemetryEvent::PredictionUpdate {
            model_id: "qqq-v1".to_string(),
            pair: "QQQ/PSQ".to_string(),
            pred_1d: Some(0.005),
            pred_5d: Some(0.015),
            pred_21d: Some(0.04),
            timestamp: 1_700_000_000,
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "PredictionUpdate");
        assert_eq!(parsed["model_id"], "qqq-v1");
        assert_eq!(parsed["pair"], "QQQ/PSQ");
        assert_eq!(parsed["pred_1d"], 0.005);
        assert_eq!(parsed["pred_5d"], 0.015);
        assert_eq!(parsed["pred_21d"], 0.04);
        assert_eq!(parsed["timestamp"], 1_700_000_000);
    }

    /// TradeFill variant must serialize with `type = "TradeFill"` and
    /// carry both `model_id` and `pair` for frontend partitioning.
    #[test]
    fn trade_fill_serialization() {
        let event = TelemetryEvent::TradeFill {
            model_id: "qqq-v1".to_string(),
            pair: "QQQ/PSQ".to_string(),
            side: "buy".to_string(),
            symbol: "QQQ".to_string(),
            qty: 1.0,
            price: 500.0,
            fee: 0.75,
            realized_pnl: 0.0,
            timestamp: 42,
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "TradeFill");
        assert_eq!(parsed["model_id"], "qqq-v1");
        assert_eq!(parsed["pair"], "QQQ/PSQ");
        assert_eq!(parsed["side"], "buy");
        assert_eq!(parsed["symbol"], "QQQ");
        assert_eq!(parsed["qty"], 1.0);
        assert_eq!(parsed["price"], 500.0);
        assert_eq!(parsed["fee"], 0.75);
        assert_eq!(parsed["realized_pnl"], 0.0);
        assert_eq!(parsed["timestamp"], 42);
    }

    /// StalenessAlert variant with an `Option` field must round-trip
    /// alongside the new model_id/pair attribution fields.
    #[test]
    fn staleness_alert_serialization() {
        let event = TelemetryEvent::StalenessAlert {
            model_id: "nvda-v1".to_string(),
            pair: "NVDA/NVDD".to_string(),
            last_candle_ts: Some(1_000),
            seconds_since_last: 300,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "StalenessAlert");
        assert_eq!(parsed["model_id"], "nvda-v1");
        assert_eq!(parsed["pair"], "NVDA/NVDD");
        assert_eq!(parsed["last_candle_ts"], 1_000);
        assert_eq!(parsed["seconds_since_last"], 300);

        // Null variant of the option.
        let event_none = TelemetryEvent::StalenessAlert {
            model_id: "qqq-v1".to_string(),
            pair: "QQQ/PSQ".to_string(),
            last_candle_ts: None,
            seconds_since_last: 0,
        };
        let json_none = serde_json::to_string(&event_none).unwrap();
        let parsed_none: serde_json::Value = serde_json::from_str(&json_none).unwrap();
        assert!(parsed_none["last_candle_ts"].is_null());
    }

    /// Global variants must NOT carry model_id/pair — they remain
    /// shape-stable for back-compat with the Events tab and ModeChange
    /// handlers. Confirms the §8 design rule.
    #[test]
    fn mode_change_is_model_agnostic() {
    let event = TelemetryEvent::ModeChange {
        mode: "live".to_string(),
        timestamp: 1_700_000_001,
    };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "ModeChange");
    assert!(parsed.get("model_id").is_none(), "ModeChange must not carry model_id");
    assert!(parsed.get("pair").is_none(), "ModeChange must not carry pair");
}
}
