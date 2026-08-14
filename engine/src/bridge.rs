use anyhow::{anyhow, Result};
use serde_json::json;
use std::time::Duration;
use tracing::{debug, warn};
use zeromq::{ReqSocket, Socket, SocketRecv, SocketSend, ZmqMessage};

use crate::features::core::FEATURE_DIM;
use crate::features::equities_v2::EQ_FEATURE_DIM;

/// Predictions returned by the inference service.
#[derive(Debug, Clone)]
pub struct Prediction {
    pub pred_1h: f64,
    pub pred_4h: f64,
    pub pred_24h: f64,
}

/// Predictions for the QQQ daily equities model (Wave C).
/// Three horizons: 1-day, 5-day, 21-day expected returns.
#[derive(Debug, Clone)]
pub struct EquityPrediction {
    pub pred_1d: f64,
    pub pred_5d: f64,
    pub pred_21d: f64,
}

/// ZeroMQ REQ client that sends feature windows to a Python inference service.
pub struct ZmqBridge {
    socket: ReqSocket,
    endpoint: String,
}

impl ZmqBridge {
    /// Connect a new REQ socket to `endpoint` (e.g. `"tcp://127.0.0.1:5555"`).
    pub async fn connect(endpoint: &str) -> Result<Self> {
        let mut socket = ReqSocket::new();
        socket
            .connect(endpoint)
            .await
            .map_err(|e| anyhow!("ZMQ connect to {endpoint} failed: {e}"))?;
        Ok(Self { socket, endpoint: endpoint.to_string() })
    }

    /// Send `feature_window` and receive a `Prediction`.
    ///
    /// Applies `timeout` around the entire send+recv round-trip.
    /// Returns `Err` on timeout, ZMQ error, or if the response contains an
    /// `"error"` key.
    pub async fn predict(
        &mut self,
        feature_window: &[[f64; 3]],
        timeout: Duration,
    ) -> Result<Prediction> {
        let payload = json!({ "feature_window": feature_window }).to_string();
        debug!(bytes = payload.len(), "sending inference request");

        let fut = async {
            self.socket
                .send(ZmqMessage::from(payload.clone()))
                .await
                .map_err(|e| anyhow!("ZMQ send failed: {e}"))?;

            let reply: ZmqMessage = self
                .socket
                .recv()
                .await
                .map_err(|e| anyhow!("ZMQ recv failed: {e}"))?;

            let bytes = reply
                .get(0)
                .map(|b| b.as_ref())
                .unwrap_or(b"");

            debug!(bytes = bytes.len(), "received inference response");

            let value: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|e| anyhow!("failed to parse response JSON: {e}"))?;

            if let Some(err_msg) = value.get("error") {
                return Err(anyhow!("inference service error: {err_msg}"));
            }

            let pred_1h = value
                .get("pred_1h")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| anyhow!("missing or invalid field 'pred_1h'"))?;
            let pred_4h = value
                .get("pred_4h")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| anyhow!("missing or invalid field 'pred_4h'"))?;
            let pred_24h = value
                .get("pred_24h")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| anyhow!("missing or invalid field 'pred_24h'"))?;

            Ok(Prediction {
                pred_1h,
                pred_4h,
                pred_24h,
            })
        };

        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| anyhow!("inference request timed out after {timeout:?}"))?
    }

    /// Retry wrapper: call `predict` up to `retries` times (total attempts = retries + 1).
    ///
    /// On each failure, logs a warning with the attempt number and error.
    /// On final failure, returns the last error.
    pub async fn predict_with_retry(
        &mut self,
        feature_window: &[[f64; 3]],
        timeout: Duration,
        retries: u32,
    ) -> Result<Prediction> {
        let total_attempts = retries + 1;
        let mut last_err = anyhow!("no attempts made");

        for attempt in 1..=total_attempts {
            match self.predict(feature_window, timeout).await {
                Ok(pred) => return Ok(pred),
                Err(e) => {
                    last_err = e;
                    warn!(
                        attempt,
                        total_attempts,
                        error = %last_err,
                        "inference attempt failed"
                    );
                }
            }
        }

        Err(last_err)
    }

    // -----------------------------------------------------------------------
    // V2 inference path (Wave 5) — 6-dim feature window + schema_version.
    // Dormant until the new model clears the walk-forward OOS IC gate.
    // -----------------------------------------------------------------------

    /// Send a V2 (6-dim) feature window to the inference service.
    pub async fn predict_v2(
        &mut self,
        feature_window: &[[f64; FEATURE_DIM]],
        timeout: Duration,
    ) -> Result<Prediction> {
        let payload = json!({
            "schema_version": 2,
            "feature_window": feature_window,
        })
        .to_string();
        debug!(bytes = payload.len(), "sending V2 inference request");

        let fut = async {
            self.socket
                .send(ZmqMessage::from(payload.clone()))
                .await
                .map_err(|e| anyhow!("ZMQ send failed: {e}"))?;
            let reply: ZmqMessage = self
                .socket
                .recv()
                .await
                .map_err(|e| anyhow!("ZMQ recv failed: {e}"))?;
            let bytes = reply.get(0).map(|b| b.as_ref()).unwrap_or(b"");
            let value: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|e| anyhow!("failed to parse response JSON: {e}"))?;
            if let Some(err_msg) = value.get("error") {
                return Err(anyhow!("inference service error: {err_msg}"));
            }
            Ok(Prediction {
                pred_1h: value.get("pred_1h").and_then(|v| v.as_f64())
                    .ok_or_else(|| anyhow!("missing or invalid field 'pred_1h'"))?,
                pred_4h: value.get("pred_4h").and_then(|v| v.as_f64())
                    .ok_or_else(|| anyhow!("missing or invalid field 'pred_4h'"))?,
                pred_24h: value.get("pred_24h").and_then(|v| v.as_f64())
                    .ok_or_else(|| anyhow!("missing or invalid field 'pred_24h'"))?,
            })
        };

        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| anyhow!("V2 inference request timed out after {timeout:?}"))?
    }

    /// V2 retry wrapper (mirrors `predict_with_retry`).
    #[allow(dead_code)]
    pub async fn predict_v2_with_retry(
        &mut self,
        feature_window: &[[f64; FEATURE_DIM]],
        timeout: Duration,
        retries: u32,
    ) -> Result<Prediction> {
        let total_attempts = retries + 1;
        let mut last_err = anyhow!("no attempts made");
        for _ in 1..=total_attempts {
            match self.predict_v2(feature_window, timeout).await {
                Ok(pred) => return Ok(pred),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    // -----------------------------------------------------------------------
    // V3 inference path (Wave C) — 8-dim equities feature window.
    // Returns EquityPrediction (1d/5d/21d horizons).
    // -----------------------------------------------------------------------

    /// Send a V3 (8-dim) equities feature window to the inference service.
    /// `atr_ratio` = ATR(14) / close for the latest candle — used by the service
    /// to denormalize predictions back to raw log-return space.
    pub async fn predict_v3(
        &mut self,
        symbol: &str,
        feature_window: &[[f64; EQ_FEATURE_DIM]],
        atr_ratio: f64,
        timeout: Duration,
    ) -> Result<EquityPrediction> {
        let payload = json!({
            "schema_version": 3,
            "symbol": symbol,
            "feature_window": feature_window,
            "atr_ratio": atr_ratio,
        })
        .to_string();
        debug!(bytes = payload.len(), "sending V3 inference request");

        let fut = async {
            self.socket
                .send(ZmqMessage::from(payload.clone()))
                .await
                .map_err(|e| anyhow!("ZMQ send failed: {e}"))?;
            let reply: ZmqMessage = self
                .socket
                .recv()
                .await
                .map_err(|e| anyhow!("ZMQ recv failed: {e}"))?;
            let bytes = reply.get(0).map(|b| b.as_ref()).unwrap_or(b"");
            let value: serde_json::Value = serde_json::from_slice(bytes)
                .map_err(|e| anyhow!("failed to parse response JSON: {e}"))?;
            if let Some(err_msg) = value.get("error") {
                return Err(anyhow!("inference service error: {err_msg}"));
            }
            Ok(EquityPrediction {
                pred_1d: value.get("pred_1d").and_then(|v| v.as_f64())
                    .ok_or_else(|| anyhow!("missing or invalid field 'pred_1d'"))?,
                pred_5d: value.get("pred_5d").and_then(|v| v.as_f64())
                    .ok_or_else(|| anyhow!("missing or invalid field 'pred_5d'"))?,
                pred_21d: value.get("pred_21d").and_then(|v| v.as_f64())
                    .ok_or_else(|| anyhow!("missing or invalid field 'pred_21d'"))?,
            })
        };

        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| anyhow!("V3 inference request timed out after {timeout:?}"))?
    }

    /// V3 retry wrapper with socket recovery.
    ///
    /// After a timeout, the REQ socket is in an invalid state (send→recv lockstep violated).
    /// We must reconnect before retrying.
    pub async fn predict_v3_with_retry(
        &mut self,
        symbol: &str,
        feature_window: &[[f64; EQ_FEATURE_DIM]],
        atr_ratio: f64,
        timeout: Duration,
        retries: u32,
    ) -> Result<EquityPrediction> {
        let total_attempts = retries + 1;
        let mut last_err = anyhow!("no attempts made");
        for attempt in 1..=total_attempts {
            match self.predict_v3(symbol, feature_window, atr_ratio, timeout).await {
                Ok(pred) => return Ok(pred),
                Err(e) => {
                    last_err = e;
                    warn!(attempt, total_attempts, error = %last_err, "V3 inference attempt failed");
                    // After timeout or error, reconnect to reset REQ socket state
                    if attempt < total_attempts {
                        if let Err(reconnect_err) = self.reconnect().await {
                            warn!(error = %reconnect_err, "failed to reconnect ZMQ socket");
                        }
                        // Brief backoff to avoid tight retry loop
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        }
        Err(last_err)
    }

    /// Reconnect the ZMQ REQ socket to reset its state machine.
    ///
    /// After a timeout, the socket is stuck in "awaiting reply" state.
    /// Closing and reconnecting resets it to a clean state.
    pub async fn reconnect(&mut self) -> Result<()> {
        // Create a new socket and connect to the same endpoint
        let mut new_socket = ReqSocket::new();
        new_socket
            .connect(&self.endpoint)
            .await
            .map_err(|e| anyhow!("ZMQ reconnect to {} failed: {}", self.endpoint, e))?;
        self.socket = new_socket;
        debug!("ZMQ socket reconnected to {}", self.endpoint);
        Ok(())
    }
}
