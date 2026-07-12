mod bridge;
mod config;
mod data;
mod db;
mod features;
mod normalize;
mod scheduler;

use std::process;

use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = match config::Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {:#}", e);
            e.chain().for_each(|c| eprintln!("  caused by: {c}"));
            process::exit(1);
        }
    };

    info!(mode = %cfg.trading_mode, symbol = %cfg.symbol, "engine configured");
    info!(key_present = cfg.kraken_api_key.is_some(), "kraken key status");
    info!(zmq = %cfg.zmq_endpoint, "inference endpoint");
    info!(
        threshold = cfg.magnitude_threshold,
        fee = cfg.paper_fee,
        sma = cfg.sma_window,
        "strategy params"
    );
    info!(port = cfg.http_port, "http port");
    info!(db = %cfg.database_url, "database");
    info!(norm_stats = %cfg.norm_stats_path, window = cfg.feature_window_size, "features");

    // Open SQLite database (creates file + applies DDL if needed).
    let pool = match db::open(&cfg.database_url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("database error: {:#}", e);
            process::exit(1);
        }
    };

    // Load normalization statistics — fail fast if the file is missing.
    let norm_stats = match normalize::NormStats::load(&cfg.norm_stats_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("norm_stats error: {:#}", e);
            process::exit(1);
        }
    };

    // Spawn the hourly scheduler (inference pipeline) as a background task.
    let scheduler_pool = pool.clone();
    let zmq_endpoint = cfg.zmq_endpoint.clone();
    let feature_window_size = cfg.feature_window_size;
    tokio::spawn(async move {
        match scheduler::Scheduler::new(scheduler_pool, &zmq_endpoint, norm_stats, feature_window_size).await {
            Ok(mut sched) => {
                if let Err(e) = sched.run().await {
                    error!("scheduler fatal error: {e:#}");
                }
            }
            Err(e) => error!("scheduler init error: {e:#}"),
        }
    });

    // Run the data pipeline (REST backfill → retention loop → WS ingestion).
    // This blocks until a fatal error.
    if let Err(e) = data::run(pool, &cfg.symbol, cfg.sma_window).await {
        eprintln!("data pipeline fatal error: {:#}", e);
        process::exit(1);
    }
}
