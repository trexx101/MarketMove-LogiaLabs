//! Events API — historical event queries.
//!
//! `GET /api/events` returns recent events from the `engine_events` table,
//! filterable by category, mode, and timestamp.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::db::DbPool;

use super::{internal_error, ApiResult, AppState};

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    #[serde(default = "default_limit")]
    pub limit: u32,
    pub category: Option<String>,
    pub since: Option<i64>,
    pub mode: Option<String>,
}

fn default_limit() -> u32 {
    100
}

#[derive(Debug, Serialize)]
pub struct EventRow {
    pub id: i64,
    pub ts: i64,
    pub category: String,
    pub severity: String,
    pub mode: String,
    pub source: String,
    pub message: String,
    pub payload: serde_json::Value,
}

pub async fn handle_events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> ApiResult<Vec<EventRow>> {
    let limit = query.limit.min(500) as i64;
    let mut sql = String::from(
        "SELECT id, ts, category, severity, mode, source, message, payload_json \
         FROM engine_events WHERE 1=1",
    );
    let mut binds: Vec<String> = Vec::new();

    if let Some(cat) = &query.category {
        sql.push_str(" AND category = ?");
        binds.push(cat.clone());
    }
    if let Some(since) = query.since {
        sql.push_str(" AND ts >= ?");
        binds.push(since.to_string());
    }
    if let Some(mode) = &query.mode {
        sql.push_str(" AND mode = ?");
        binds.push(mode.clone());
    }
    sql.push_str(" ORDER BY ts DESC LIMIT ?");
    binds.push(limit.to_string());

    let mut q = sqlx::query(&sql);
    for b in &binds {
        q = q.bind(b);
    }

    let rows = q
        .fetch_all(&state.pool)
        .await
        .map_err(|e| internal_error("events query", anyhow::anyhow!(e)))?;

    let events: Vec<EventRow> = rows
        .iter()
        .map(|r| EventRow {
            id: r.get("id"),
            ts: r.get("ts"),
            category: r.get("category"),
            severity: r.get("severity"),
            mode: r.get("mode"),
            source: r.get("source"),
            message: r.get("message"),
            payload: serde_json::from_str(r.get("payload_json"))
                .unwrap_or(serde_json::json!({})),
        })
        .collect();

    Ok(Json(events))
}

#[derive(Debug, Serialize)]
pub struct ArchiveInfo {
    pub filename: String,
    pub size_bytes: u64,
}

pub async fn handle_archives(
    State(_state): State<AppState>,
) -> ApiResult<Vec<ArchiveInfo>> {
    let archives = crate::archive::list_archives()
        .map_err(|e| internal_error("list archives", e))?;
    Ok(Json(
        archives
            .into_iter()
            .map(|a| ArchiveInfo {
                filename: a.filename,
                size_bytes: a.size_bytes,
            })
            .collect(),
    ))
}