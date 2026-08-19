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

    // Build evidence from snapshot metadata
    let evidence = crate::hyperopt::promotion::PromotionEvidence {
        n_trades: candidate.n_trades,
        ic: candidate.mean_ic,
        sharpe: 0.0, // Would come from backtest results
        days_observed: 0, // Would come from timestamps
    };

    // Attempt promotion
    match pipeline.promote(&store, &id, &evidence).await {
        Ok(result) => {
            if result.promoted {
                Ok(Json(PromoteResponse {
                    success: true,
                    message: format!("Promoted to {:?}", result.to_stage),
                }))
            } else {
                Ok(Json(PromoteResponse {
                    success: false,
                    message: result.reason,
                }))
            }
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
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
