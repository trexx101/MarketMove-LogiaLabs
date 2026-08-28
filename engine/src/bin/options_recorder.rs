//! Options Tape Recorder binary (P1)
//!
//! Spawns Python script to discover option contracts and poll quotes.
//! Reads JSON lines from stdout, writes them to parquet files partitioned
//! by underlying/chain/date, and POSTs heartbeats to the engine API.
//!
//! Architecture:
//! - Python script handles: chain discovery, market-hours gating, polling
//! - Rust binary handles: parquet writing, heartbeat POST, process lifecycle
//! - Engine is sole DB writer (recorder POSTs to /api/internal/tape/heartbeat)

use anyhow::{Context, Result};
use engine::config::Config;
use engine::options_recorder::TapeRow;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

#[derive(Deserialize, Debug)]
struct QuoteJson {
    timestamp_ms: i64,
    underlying: String,
    chain_code: String,
    contract_code: String,
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
    #[serde(default)]
    session: Option<String>,
}

impl QuoteJson {
    fn to_tape_row(&self) -> TapeRow {
        TapeRow {
            timestamp_ms: self.timestamp_ms,
            underlying: self.underlying.clone(),
            chain_code: self.chain_code.clone(),
            contract_code: self.contract_code.clone(),
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

/// POST a heartbeat to the engine API.
async fn post_heartbeat(
    client: &reqwest::Client,
    engine_url: &str,
    tape_id: &str,
    underlying: &str,
    chain_code: &str,
    quota_json: &str,
) {
    let body = serde_json::json!({
        "tape_id": tape_id,
        "underlying": underlying,
        "chain_code": chain_code,
        "quota_accounting_json": quota_json,
    });

    let url = format!("{}/api/internal/tape/heartbeat", engine_url);
    match client.post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::debug!("heartbeat posted for {}", tape_id);
        }
        Ok(resp) => {
            warn!(
                "heartbeat POST for {} returned status {}",
                tape_id,
                resp.status()
            );
        }
        Err(e) => {
            warn!("heartbeat POST for {} failed: {}", tape_id, e);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let _ = dotenvy::dotenv();

    let config = Config::from_env().context("Failed to load config")?;
    let opts = &config.options;

    let underlyings: Vec<&str> = opts.underlyings.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if underlyings.is_empty() {
        return Err(anyhow::anyhow!("No underlyings configured (OPT_UNDERLYINGS is empty)"));
    }

    info!("Options tape recorder starting");
    info!("Underlyings: {:?}", underlyings);
    info!("Quota tier: {}, recorder allocation: {} (60%)", opts.quota_tier, (opts.quota_tier as f64 * 0.6) as u32);
    info!("DTE window: {}-{}, delta target: {}", opts.dte_min, opts.dte_max, opts.delta_target);
    info!("Poll interval: 15s, market hours only (09:25-16:05 ET)");

    let writers: Arc<Mutex<HashMap<String, TapeWriter>>> = Arc::new(Mutex::new(HashMap::new()));
    let base_dir = PathBuf::from(
        std::env::var("OPT_TAPE_DIR").unwrap_or_else(|_| "./data/options_tape".to_string())
    );

    // HTTP client for heartbeat POSTs
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .context("Failed to build HTTP client")?;

    let engine_url = std::env::var("ENGINE_URL").unwrap_or_else(|_| "http://127.0.0.1:9080".to_string());

    // Spawn Python script
    // CARGO_MANIFEST_DIR = engine/ crate dir, parent = repo root
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".agents/skills/moomooapi/scripts/quote/record_option_quotes.py");

    let underlyings_str = underlyings.join(",");
    let mut child = Command::new("python3")
        .arg(&script)
        .arg("--underlyings")
        .arg(&underlyings_str)
        .arg("--dte-min")
        .arg(opts.dte_min.to_string())
        .arg("--dte-max")
        .arg(opts.dte_max.to_string())
        .arg("--delta-target")
        .arg(opts.delta_target.to_string())
        .arg("--bid-min")
        .arg(opts.bid_min.to_string())
        .arg("--spread-cap")
        .arg(opts.spread_cap_pct.to_string())
        .arg("--oi-min")
        .arg(opts.oi_min.to_string())
        .arg("--interval")
        .arg("15.0")
        .arg("--engine-url")
        .arg(&engine_url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn Python recorder")?;

    let stdout = child.stdout.take().context("Failed to get stdout")?;
    let stderr = child.stderr.take().context("Failed to get stderr")?;
    let mut reader = BufReader::new(stdout).lines();

    // Spawn a task to log stderr (scan results, errors, idle status)
    let mut stderr_reader = BufReader::new(stderr);
    tokio::spawn(async move {
        let mut buf = String::new();
        loop {
            buf.clear();
            match stderr_reader.read_line(&mut buf).await {
                Ok(0) => break,
                Ok(_) => {
                    let line = buf.trim();
                    if !line.is_empty() {
                        // Try to parse as JSON for structured logging
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                            if json.get("scan_complete").is_some() {
                                info!(stderr = %json, "Python: chain scan complete");
                            } else if json.get("scan_error").is_some() {
                                warn!(stderr = %json, "Python: chain scan error");
                            } else if json.get("idle").is_some() {
                                info!(stderr = %json, "Python: idle (outside market hours)");
                            } else if json.get("calendar_loaded").is_some() {
                                info!(stderr = %json, "Python: trading calendar loaded");
                            } else if json.get("calendar_fallback").is_some() {
                                warn!(stderr = %json, "Python: calendar fallback (weekday-only)");
                            } else if json.get("calendar_error").is_some() {
                                warn!(stderr = %json, "Python: calendar fetch error");
                            } else if json.get("error").is_some() {
                                warn!(stderr = %json, "Python: error");
                            } else if json.get("fatal_error").is_some() {
                                error!(stderr = %json, "Python: fatal error");
                            } else {
                                info!(stderr = %json, "Python: status");
                            }
                        } else {
                            info!(stderr = line, "Python stderr");
                        }
                    }
                }
                Err(e) => {
                    warn!("stderr read error: {}", e);
                    break;
                }
            }
        }
    });

    // Track which underlyings we've seen quotes for (for heartbeat)
    let seen_underlyings: Arc<Mutex<HashMap<String, (String, String)>>> =
        Arc::new(Mutex::new(HashMap::new())); // underlying → (chain_code, tape_id)

    // Read JSON lines from Python
    let mut quote_count: u64 = 0;
    while let Ok(Some(line)) = reader.next_line().await {
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<QuoteJson>(&line) {
            Ok(quote) => {
                let row = quote.to_tape_row();
                let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
                let key = format!("{}:{}", row.underlying, row.chain_code);

                // Track for heartbeat
                {
                    let mut seen = seen_underlyings.lock().await;
                    let tape_id = row.underlying
                        .replace("US.", "")
                        .to_lowercase()
                        + "-tape";
                    seen.insert(row.underlying.clone(), (row.chain_code.clone(), tape_id));
                }

                // Write to parquet
                let mut writers_guard = writers.lock().await;
                let writer = writers_guard.entry(key.clone()).or_insert_with(|| {
                    TapeWriter::new(&base_dir, &row.underlying, &row.chain_code, &date)
                        .unwrap_or_else(|e| {
                            error!("Failed to create parquet writer: {}", e);
                            panic!("parquet writer creation failed");
                        })
                });

                // Check if date rolled — need new file
                if writer.current_date != date {
                    if let Err(e) = writer.flush() {
                        error!("Failed to flush writer on date roll: {}", e);
                    }
                    *writer = TapeWriter::new(&base_dir, &row.underlying, &row.chain_code, &date)
                        .unwrap_or_else(|e| {
                            error!("Failed to create new parquet writer: {}", e);
                            panic!("parquet writer creation failed");
                        });
                }

                if let Err(e) = writer.write_rows(&[row]) {
                    error!("Failed to write row: {}", e);
                }

                quote_count += 1;
            }
            Err(e) => {
                warn!("Failed to parse quote JSON: {} (line: {})", e, line);
            }
        }

        // Post heartbeats every poll cycle (every 6 quotes = 1 full poll of 6 contracts)
        if quote_count % 6 == 0 {
            let seen = seen_underlyings.lock().await;
            for (underlying, (chain_code, tape_id)) in seen.iter() {
                let quota_json = serde_json::json!({
                    "contracts": 2,
                    "poll_secs": 15,
                    "quotes_written": quote_count,
                })
                .to_string();

                post_heartbeat(
                    &http_client,
                    &engine_url,
                    tape_id,
                    underlying,
                    chain_code,
                    &quota_json,
                )
                .await;
            }
        }
    }

    // Wait for child to exit
    let status = child.wait().await?;
    if !status.success() {
        error!("Python recorder exited with non-zero status: {:?}", status);
    }

    // Flush all writers
    let mut writers_guard = writers.lock().await;
    for (key, writer) in writers_guard.iter_mut() {
        if let Err(e) = writer.flush() {
            error!("Failed to flush writer for {}: {}", key, e);
        }
    }

    info!("Shutting down. Total quotes written: {}", quote_count);

    Ok(())
}
