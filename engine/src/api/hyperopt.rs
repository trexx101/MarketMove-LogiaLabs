use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};

use crate::hyperopt::{CandidateStore, PromotionPipeline};
use crate::{api::AppState, db};

#[derive(Serialize)]
pub struct CandidateResponse {
    pub id: String,
    pub equity: String,
    pub strategy: String,
    pub status: String,
    pub mean_ic: f64,
    pub std_ic: f64,
    pub n_trades: i32,
    pub params: serde_json::Value,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct CandidatesListResponse {
    pub equity: String,
    pub candidates: Vec<CandidateResponse>,
}

#[derive(Serialize)]
pub struct HyperoptStatusResponse {
    pub equity: String,
    pub pipeline_state: String,
    pub total_candidates: i64,
    pub by_status: std::collections::HashMap<String, i64>,
}

#[derive(Deserialize)]
pub struct PromoteRequest {
    pub target_status: String,
}

#[derive(Serialize)]
pub struct PromoteResponse {
    pub success: bool,
    pub message: String,
}

/// GET /api/hyperopt/:equity/candidates
pub async fn list_candidates(
    State(state): State<AppState>,
    Path(equity): Path<String>,
) -> Result<Json<CandidatesListResponse>, StatusCode> {
    let pool = &state.pool;
    let store = CandidateStore::new(pool.clone());

    let candidates = store
        .list_by_equity(&equity)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response = CandidatesListResponse {
        equity,
        candidates: candidates
            .into_iter()
            .map(|c| CandidateResponse {
                id: c.version_id,
                equity: c.equity,
                strategy: c.strategy_family,
                status: c.status.as_str().to_string(),
                mean_ic: c.mean_ic,
                std_ic: c.std_ic,
                n_trades: c.n_trades as i32,
                params: serde_json::to_value(&c.params).unwrap_or_default(),
                created_at: crate::api::ts_to_rfc3339(c.created_at),
            })
            .collect(),
    };

    Ok(Json(response))
}

/// GET /api/hyperopt/:equity/candidates/:id
pub async fn get_candidate(
    State(state): State<AppState>,
    Path((equity, id)): Path<(String, String)>,
) -> Result<Json<CandidateResponse>, StatusCode> {
    let pool = &state.pool;
    let store = CandidateStore::new(pool.clone());

    let candidate = store
        .get(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if candidate.equity != equity {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(CandidateResponse {
        id: candidate.version_id,
        equity: candidate.equity,
        strategy: candidate.strategy_family,
        status: candidate.status.as_str().to_string(),
        mean_ic: candidate.mean_ic,
        std_ic: candidate.std_ic,
        n_trades: candidate.n_trades as i32,
        params: serde_json::to_value(&candidate.params).unwrap_or_default(),
        created_at: crate::api::ts_to_rfc3339(candidate.created_at),
    }))
}

/// POST /api/hyperopt/:equity/promote/:id
///
/// D13: promotion never happens mid-exit. This endpoint only VALIDATES the
/// gate and QUEUES the request; the actual status flip happens at the next
/// daily candle boundary for the equity (see apply_pending_promotions).
pub async fn promote_candidate(
    State(state): State<AppState>,
    Path((equity, id)): Path<(String, String)>,
    Json(req): Json<PromoteRequest>,
) -> Result<Json<PromoteResponse>, StatusCode> {
    let pool = &state.pool;
    let store = CandidateStore::new(pool.clone());
    let pipeline = PromotionPipeline::new();

    // Verify candidate exists and belongs to equity
    let candidate = store
        .get(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if candidate.equity != equity {
        return Err(StatusCode::NOT_FOUND);
    }

    // Validate target status is a real stage
    if !matches!(req.target_status.as_str(), "PAPER" | "MICRO" | "LIVE") {
        return Ok(Json(PromoteResponse {
            success: false,
            message: "target_status must be one of PAPER, MICRO, LIVE".into(),
        }));
    }

    // Gate 1 (request time): no open positions on this equity — never promote mid-exit
    let open_positions = db::list_option_positions(pool, Some(&equity), Some("OPEN"), 1)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !open_positions.is_empty() {
        return Ok(Json(PromoteResponse {
            success: false,
            message: format!(
                "Promotion blocked: {} has open positions (mid-exit promotion is forbidden). \
                 Close or exit the position first.",
                equity
            ),
        }));
    }

    // Gate 2 (request time): dry-run the stage gates so obviously-unready
    // candidates fail fast with a useful message instead of queueing forever.
    let evidence = crate::hyperopt::promotion::PromotionEvidence {
        n_trades: candidate.n_trades,
        ic: candidate.mean_ic,
        sharpe: 0.0,     // Would come from backtest results
        days_observed: 0, // Would come from timestamps
    };
    let dry_run = pipeline.check_snapshot(&candidate, &evidence);
    if !dry_run.promoted {
        return Ok(Json(PromoteResponse {
            success: false,
            message: format!("Promotion gates not met: {}", dry_run.reason),
        }));
    }

    // Gates passed — queue for application at the next daily candle boundary.
    // Persist the evidence that was validated NOW so the boundary applier
    // applies exactly this evidence (never fabricates sharpe/days later).
    let evidence_json = serde_json::to_string(&evidence).unwrap_or_else(|_| "{}".into());
    db::queue_pending_promotion(pool, &id, &equity, &req.target_status, &evidence_json)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(PromoteResponse {
        success: true,
        message: format!(
            "Queued for promotion to {} at the next daily candle boundary for {}",
            req.target_status, equity
        ),
    }))
}

/// GET /api/hyperopt/:equity/status
pub async fn get_status(
    State(state): State<AppState>,
    Path(equity): Path<String>,
) -> Result<Json<HyperoptStatusResponse>, StatusCode> {
    let pool = &state.pool;

    // Get total candidates
    let total = db::count_strategy_versions(pool, &equity)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Get breakdown by status
    let by_status = db::count_strategy_versions_by_status(pool, &equity)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Get scheduler state (mock for now — would come from OptionsScheduler)
    let pipeline_state = "idle".to_string();

    Ok(Json(HyperoptStatusResponse {
        equity,
        pipeline_state,
        total_candidates: total,
        by_status,
    }))
}
