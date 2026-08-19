//! Options Tape Recorder binary (P1)
//! 
//! Spawns Python script to poll option quotes, reads JSON lines from stdout,
//! and writes them to parquet files partitioned by underlying/chain/date.

use anyhow::{Context, Result};
use engine::config::Config;
use engine::options_recorder::{QuotaAccount, TapeRow};
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

#[derive(Deserialize, Debug)]
struct QuoteJson {
    timestamp_ms: i64,
    contract: String,
    bid: f64,
    ask: f64,
    last: f64,
    volume: i64,
    oi: i64,
    iv: f64,
    delta: f64,
    gamma: f64,
    theta: f64,
    underlying_price: f64,
}

impl QuoteJson {
    fn to_tape_row(&self) -> TapeRow {
        // Parse contract code: US.QQQ260919C00530000
        // underlying = US.QQQ, chain = US.QQQ260919
        let parts: Vec<&str> = self.contract.splitn(2, |c| c == 'C' || c == 'P').collect();
        let underlying = parts[0][..7].to_string(); // US.QQQ
        let chain = parts[0].to_string(); // US.QQQ260919
        
        TapeRow {
            timestamp_ms: self.timestamp_ms,
            underlying,
            chain_code: chain,
            contract_code: self.contract.clone(),
            bid: self.bid,
            ask: self.ask,
            last: self.last,
            volume: self.volume,
            open_interest: self.oi,
            implied_volatility: self.iv,
            delta: self.delta,
            gamma: self.gamma,
            theta: self.theta,
            underlying_price: self.underlying_price,
        }
    }
}

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
        
        let schema = engine::options_recorder::build_tape_schema();
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
        let arrays = engine::options_recorder::build_tape_arrays(rows);
        let batch = arrow::record_batch::RecordBatch::try_new(
            Arc::new(engine::options_recorder::build_tape_schema()),
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
    
    // TODO: Determine which chains to record based on config
    // For now, use a hardcoded example
    let contracts = vec!["US.QQQ260919C00530000".to_string()];
    
    // Subscribe to quota
    for contract in &contracts {
        if !quota.try_subscribe(contract) {
            warn!("Quota limit reached, cannot subscribe to {}", contract);
            break;
        }
    }
    
    info!("Subscribed to {} contracts, spawning Python recorder", quota.used());
    
    // Spawn Python script
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join(".agents/skills/moomooapi/scripts/quote/record_option_quotes.py");
    
    let contracts_str = contracts.join(",");
    let mut child = Command::new("python3")
        .arg(&script)
        .arg("--contracts")
        .arg(&contracts_str)
        .arg("--interval")
        .arg("5.0")
        .arg("--duration")
        .arg("86400") // 24 hours
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn Python recorder")?;
    
    let stdout = child.stdout.take().context("Failed to get stdout")?;
    let mut reader = BufReader::new(stdout).lines();
    
    let base_dir = PathBuf::from("./data/options_tape");
    
    // Read JSON lines from Python
    while let Ok(Some(line)) = reader.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        
        match serde_json::from_str::<QuoteJson>(&line) {
            Ok(quote) => {
                let row = quote.to_tape_row();
                let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
                let key = format!("{}:{}", row.underlying, row.chain_code);
                
                let mut writers_guard = writers.lock().await;
                let writer = writers_guard.entry(key.clone()).or_insert_with(|| {
                    TapeWriter::new(&base_dir, &row.underlying, &row.chain_code, &date)
                        .expect("Failed to create writer")
                });
                
                if let Err(e) = writer.write_rows(&[row]) {
                    error!("Failed to write row: {}", e);
                }
            }
            Err(e) => {
                warn!("Failed to parse quote JSON: {} (line: {})", e, line);
            }
        }
    }
    
    // Wait for child to exit
    let status = child.wait().await?;
    if !status.success() {
        let stderr = child.stderr.take().unwrap();
        let mut stderr_reader = BufReader::new(stderr);
        let mut stderr_content = String::new();
        stderr_reader.read_to_string(&mut stderr_content).await?;
        error!("Python recorder exited with error: {}", stderr_content);
    }
    
    // Flush all writers
    let mut writers_guard = writers.lock().await;
    for (_, writer) in writers_guard.iter_mut() {
        if let Err(e) = writer.flush() {
            error!("Failed to flush writer: {}", e);
        }
    }
    
    info!("Shutting down");
    
    Ok(())
}
