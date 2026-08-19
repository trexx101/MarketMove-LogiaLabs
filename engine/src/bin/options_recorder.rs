//! Options Tape Recorder binary (P1)
//! 
//! Separate process, own OpenD connection, QUOTE-only subscriptions.
//! Records option ticks to parquet, partitioned by underlying/chain/date.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use engine::config::Config;
use engine::options_recorder::{build_tape_arrays, build_tape_schema, QuotaAccount, TapeRow};
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

struct TapeWriter {
    writer: Option<ArrowWriter<std::fs::File>>,
    current_date: String,
    current_chain: String,
}

impl TapeWriter {
    fn new(base_dir: &PathBuf, underlying: &str, chain: &str, date: &str) -> Result<Self> {
        let dir = base_dir.join(underlying).join(chain);
        std::fs::create_dir_all(&dir)?;
        
        let path = dir.join(format!("{}.parquet", date));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        
        let schema = build_tape_schema();
        let props = WriterProperties::builder()
            .set_compression(parquet::basic::Compression::SNAPPY)
            .build();
        
        let writer = ArrowWriter::try_new(file, Arc::new(schema), Some(props))?;
        
        Ok(Self {
            writer: Some(writer),
            current_date: date.to_string(),
            current_chain: chain.to_string(),
        })
    }
    
    fn write_rows(&mut self, rows: &[TapeRow]) -> Result<()> {
        let writer = self.writer.as_mut().context("Writer already closed")?;
        let arrays = build_tape_arrays(rows);
        let batch = arrow::record_batch::RecordBatch::try_new(
            Arc::new(build_tape_schema()),
            arrays,
        )?;
        writer.write(&batch)?;
        Ok(())
    }
    
    fn flush(&mut self) -> Result<()> {
        if let Some(writer) = self.writer.take() {
            writer.close()?;
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    
    let config = Config::from_env().context("Failed to load config")?;
    let opts = &config.options;
    
    info!("Options tape recorder starting");
    info!("Quota tier: {}, recorder allocation: {} (60%)", opts.quota_tier, (opts.quota_tier as f64 * 0.6) as u32);
    
    let mut quota = QuotaAccount::new(opts.quota_tier);
    let writers: Arc<Mutex<HashMap<String, TapeWriter>>> = Arc::new(Mutex::new(HashMap::new()));
    
    // TODO: Connect to OpenD, subscribe to chains, handle quote pushes
    // For now, just log that we're ready
    info!("Tape recorder ready (OpenD integration pending)");
    
    // Keep alive
    tokio::signal::ctrl_c().await?;
    info!("Shutting down");
    
    Ok(())
}
