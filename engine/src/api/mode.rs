//! Runtime paper/live mode toggle (Phase 3.4).
//!
//! Implements the two endpoints:
//!
//! - `GET /api/mode` — return the current `TradingMode`, the parity-marker's
//!   age in seconds, and `parity_valid` (true iff the marker is present and
//!   younger than `parity_max_age_secs`).
//! - `POST /api/mode` — accept a JSON body `{ "mode": "live" | "paper",
//!   "auth_token": "<6-digit TOTP>" }`, validate the TOTP against the
//!   engine's secret, check the parity marker is fresh, then flip the shared
//!   `TradingMode` value and broadcast a `TelemetryEvent::ModeChange`.
//!
//! ## Safety
//!
//! - The TOTP is validated with `crate::totp::verify` (SHA-1, 6 digits, 30s
//!   period, ±1 step skew).
//! - The parity marker is re-checked at request time, not just at startup:
//!   flipping to `live` requires the marker to be < `parity_max_age_secs` old.
//! - Every successful flip is appended to `mode_switches` for audit.
//!
//! ## What's NOT in this module
//!
//! - The executor swap (Paper → Moomoo) is driven by the scheduler reading
//!   `trading_mode` at each cycle. This module only flips the shared value
//!   and broadcasts the change.

use axum::{extract::State, http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::TradingMode;
use crate::db;
use crate::totp;

use super::{internal_error, ws::TelemetryEvent, ApiResult, AppState};

#[derive(Debug, Serialize)]
pub struct ModeResponse {
    pub mode: String,
    pub parity_marker_age_secs: Option<i64>,
    pub parity_valid: bool,
    pub last_switch_ts: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SetModeRequest {
    /// Target mode: `"paper"` or `"live"`.
    pub mode: String,
    /// 6-digit TOTP code from the user's authenticator app.
    pub auth_token: String,
}

#[derive(Debug, Serialize)]
pub struct SetModeResponse {
    pub success: bool,
    pub message: String,
    pub mode: String,
}

pub async fn handle_get_mode(State(state): State<AppState>) -> ApiResult<ModeResponse> {
    let current = *state.trading_mode.read().await;
    let age = db::parity_marker_age_secs(&state.parity_marker_path);
    let parity_valid = matches!(age, Some(a) if a <= state.parity_max_age_secs);
    let last_switch = db::fetch_recent_mode_switches(&state.pool, 1)
        .await
        .map_err(|e| internal_error("fetch_recent_mode_switches", e))?
        .first()
        .map(|r| r.timestamp);

    Ok(Json(ModeResponse {
        mode: current.to_string(),
        parity_marker_age_secs: age,
        parity_valid,
        last_switch_ts: last_switch,
    }))
}

pub async fn handle_set_mode(
    State(state): State<AppState>,
    Json(req): Json<SetModeRequest>,
) -> Result<Json<SetModeResponse>, (StatusCode, String)> {
    let target = match req.mode.to_lowercase().as_str() {
        "paper" => TradingMode::Paper,
        "live" => TradingMode::Live,
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("mode must be 'paper' or 'live', got '{other}'"),
            ));
        }
    };

    // 1. TOTP check.
    let valid = totp::verify(&state.totp_secret, &req.auth_token)
        .map_err(|e| internal_error("totp_verify", e))?;
    if !valid {
        warn!(
            target = %target,
            "POST /api/mode rejected: invalid TOTP code"
        );
        return Err((
            StatusCode::FORBIDDEN,
            "TOTP invalid".to_string(),
        ));
    }

    // 2. Parity check (only when flipping to live).
    if target == TradingMode::Live {
        let age = db::parity_marker_age_secs(&state.parity_marker_path)
            .ok_or_else(|| {
                (
                    StatusCode::FORBIDDEN,
                    format!(
                        "parity marker not found at '{}' — re-run the parity harness",
                        state.parity_marker_path
                    ),
                )
            })?;
        if age > state.parity_max_age_secs {
            return Err((
                StatusCode::FORBIDDEN,
                format!(
                    "parity marker is {}s old (max {}s) — re-run the parity harness",
                    age, state.parity_max_age_secs
                ),
            ));
        }
    }

    // 3. Flip the shared mode.
    let previous = {
        let mut guard = state.trading_mode.write().await;
        let prev = *guard;
        *guard = target;
        prev
    };

    let now = chrono::Utc::now().timestamp();
    let parity_age = db::parity_marker_age_secs(&state.parity_marker_path).unwrap_or(-1);
    let authorized_by = format!("totp:{}", truncate(&state.totp_secret, 6));

    db::insert_mode_switch(
        &state.pool,
        &previous.to_string(),
        &target.to_string(),
        parity_age,
        &authorized_by,
        now,
    )
    .await
    .map_err(|e| internal_error("insert_mode_switch", e))?;

    // Emit event for the unified log.
    state
        .event_logger
        .emit(crate::event::EngineEvent::mode_changed(previous, target, &authorized_by))
        .await;

    // 4. Broadcast to control-room clients.
    let _ = state.tx.send(TelemetryEvent::ModeChange {
        mode: target.to_string(),
        timestamp: now,
    });

    info!(
        previous = %previous,
        new = %target,
        parity_age_secs = parity_age,
        "trading mode flipped"
    );

    Ok(Json(SetModeResponse {
        success: true,
        message: format!("switched to {}", target),
        mode: target.to_string(),
    }))
}

fn truncate(s: &str, n: usize) -> String {
    let chars: String = s.chars().take(n).collect();
    chars
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_works() {
        assert_eq!(truncate("ABCDEFGH", 4), "ABCD");
        assert_eq!(truncate("", 4), "");
    }
}
