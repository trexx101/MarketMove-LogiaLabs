mod api;
mod bridge;
mod config;
mod data;
mod db;
mod exec;
mod features;
mod normalize;
mod scheduler;
mod strategy;

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

    // Construct executor based on trading mode
    let executor = match cfg.trading_mode {
        config::TradingMode::Paper => {
            info!(fee = cfg.paper_fee, "using paper executor");
            exec::ExecutorKind::Paper(exec::paper::PaperExecutor::new(pool.clone(), cfg.paper_fee))
        }
        config::TradingMode::Live => {
            let key = cfg.kraken_api_key.as_deref().expect("live mode requires key");
            let secret = cfg.kraken_api_secret.as_deref().expect("live mode requires secret");
            match exec::kraken::KrakenExecutor::new(key, secret, &cfg.symbol) {
                Ok(k) => {
                    info!(symbol = %cfg.symbol, "using Kraken executor");
                    exec::ExecutorKind::Kraken(k)
                }
                Err(e) => {
                    eprintln!("Kraken executor init error: {e:#}");
                    process::exit(1);
                }
            }
        }
    };

    // Spawn the hourly scheduler (inference pipeline) as a background task.
    let scheduler_pool = pool.clone();
    let zmq_endpoint = cfg.zmq_endpoint.clone();
    let feature_window_size = cfg.feature_window_size;
    let strategy_params = strategy::StrategyParams {
        magnitude_threshold: cfg.magnitude_threshold,
        sma_window: cfg.sma_window,
    };
    tokio::spawn(async move {
        match scheduler::Scheduler::new(scheduler_pool, &zmq_endpoint, norm_stats, feature_window_size, strategy_params, executor).await {
            Ok(mut sched) => {
                if let Err(e) = sched.run().await {
                    error!("scheduler fatal error: {e:#}");
                }
            }
            Err(e) => error!("scheduler init error: {e:#}"),
        }
    });

    // Build and spawn the Axum telemetry server.
    let app = api::router(pool.clone(), &cfg);
    let bind_addr = format!("0.0.0.0:{}", cfg.http_port);
    match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(listener) => {
            info!(addr = %bind_addr, "http server listening");
            tokio::spawn(async move {
                if let Err(e) = axum::serve(listener, app).await {
                    error!("http server error: {e:#}");
                }
                info!("http server stopped");
            });
        }
        Err(e) => {
            eprintln!("http server bind error on {bind_addr}: {e:#}");
            process::exit(1);
        }
    }

    // Run the data pipeline (REST backfill → retention loop → WS ingestion).
    // This blocks until a fatal error.
    if let Err(e) = data::run(pool, &cfg.symbol, cfg.sma_window).await {
        eprintln!("data pipeline fatal error: {:#}", e);
        process::exit(1);
    }
}
