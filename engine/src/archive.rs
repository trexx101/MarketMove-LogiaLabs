//! Event archival — exports old events to compressed JSON files.
//!
//! Runs daily from the ingestion supervisor. Events older than
//! `retention_days` are exported to `/app/data/events_archive/YYYY-MM.json.gz`
//! and deleted from the active table.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use chrono::{TimeZone, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
use sqlx::Row;
use tracing::{info, warn};

use crate::db::DbPool;

/// Archive events older than `retention_days` to gzipped JSON files.
///
/// Returns the number of rows archived (and deleted from the active table).
pub async fn archive_old_events(pool: &DbPool, retention_days: i64) -> Result<usize> {
    let cutoff = Utc::now().timestamp() - retention_days * 86_400;

    // Fetch old events
    let rows = sqlx::query(
        "SELECT id, ts, category, severity, mode, source, message, payload_json \
         FROM engine_events WHERE ts < ?1 ORDER BY ts ASC",
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(0);
    }

    // Group by month
    let mut months: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for r in &rows {
        let ts: i64 = r.get("ts");
        let dt = Utc
            .timestamp_opt(ts, 0)
            .single()
            .unwrap_or_else(Utc::now);
        let month_key = dt.format("%Y-%m").to_string();

        let entry = serde_json::json!({
            "id": r.get::<i64, _>("id"),
            "ts": ts,
            "category": r.get::<String, _>("category"),
            "severity": r.get::<String, _>("severity"),
            "mode": r.get::<String, _>("mode"),
            "source": r.get::<String, _>("source"),
            "message": r.get::<String, _>("message"),
            "payload": r.get::<String, _>("payload_json"),
        });
        months.entry(month_key).or_default().push(entry);
    }

    // Write each month to a gzipped JSON file
    let archive_dir = PathBuf::from("/app/data/events_archive");
    std::fs::create_dir_all(&archive_dir)?;

    for (month, events) in months {
        let path = archive_dir.join(format!("{month}.json.gz"));
        let file = std::fs::File::create(&path)?;
        let mut enc = GzEncoder::new(file, Compression::default());
        let json = serde_json::to_string(&events)?;
        enc.write_all(json.as_bytes())?;
        enc.finish()?;
        info!(
            month,
            events = events.len(),
            path = %path.display(),
            "archived events to gzipped JSON"
        );
    }

    // Delete archived rows from DB
    let deleted = sqlx::query("DELETE FROM engine_events WHERE ts < ?1")
        .bind(cutoff)
        .execute(pool)
        .await?
        .rows_affected() as usize;

    Ok(deleted)
}

/// List available archive files in `/app/data/events_archive/`.
pub fn list_archives() -> Result<Vec<ArchiveInfo>> {
    let archive_dir = PathBuf::from("/app/data/events_archive");
    if !archive_dir.exists() {
        return Ok(Vec::new());
    }

    let mut archives = Vec::new();
    for entry in std::fs::read_dir(archive_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "gz").unwrap_or(false) {
            let filename = path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            archives.push(ArchiveInfo { filename, size_bytes });
        }
    }

    archives.sort_by(|a, b| b.filename.cmp(&a.filename)); // newest first
    Ok(archives)
}

#[derive(Debug, serde::Serialize)]
pub struct ArchiveInfo {
    pub filename: String,
    pub size_bytes: u64,
}