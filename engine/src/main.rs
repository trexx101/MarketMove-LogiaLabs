mod api;
mod bridge;
mod config;
mod data;
mod db;
mod exec;
mod features;
mod normalize;
mod market_hours;
// `parity` is referenced indirectly through `config::Config::from_env` (the
// live-mode guard). The bin doesn't use any parity items directly.
#[allow(dead_code)]
mod parity;
mod scheduler;
mod strategy;

use std::process;

use tracing::{error, info, warn};
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
    // Wave C: loads the QQQ median/MAD norm stats (name-keyed JSON format).
    let norm_stats = match features::equities_v2::EquityNormStats::load_named(&cfg.norm_stats_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("norm_stats error: {:#}", e);
            process::exit(1);
        }
    };

    // Construct executor based on trading mode.
    // NOTE: Kraken is retired (Wave 5). The engine runs paper mode only; live
    // execution will be re-added against Binance when the new model graduates
    // the walk-forward OOS IC gate.
    let executor = match cfg.trading_mode {
        config::TradingMode::Paper => {
            info!(fee = cfg.paper_fee, "using paper executor");
            exec::ExecutorKind::Paper(exec::paper::PaperExecutor::new(pool.clone(), cfg.paper_fee))
        }
        config::TradingMode::Live => {
            warn!("TRADING_MODE=live requested but live execution is not yet wired to Binance; falling back to paper executor");
            exec::ExecutorKind::Paper(exec::paper::PaperExecutor::new(pool.clone(), cfg.paper_fee))
        }
    };

    // Run the equities REST backfill synchronously BEFORE spawning the
    // ingestion supervisor. This seeds QQQ + constituents + macro history so
    // downstream features have enough lookback. Idempotent: re-runs top up
    // only missing rows. (Crypto data source is retired — Wave A equities.)
    if let Err(e) = data::backfill_equities(&pool).await {
        eprintln!("equities backfill fatal error: {:#}", e);
        process::exit(1);
    }

    // Spawn the daily equities scheduler (inference pipeline) as a background task.
    // Wave C: replaces the crypto hourly scheduler with a daily QQQ pipeline.
    let scheduler_pool = pool.clone();
    let zmq_endpoint = cfg.zmq_endpoint.clone();
    let feature_window_size = cfg.feature_window_size;
    let symbol = cfg.symbol.clone();
    let eq_strategy_params = strategy::EquityStrategyParams {
        entry_threshold: cfg.magnitude_threshold,
        exit_threshold: -cfg.magnitude_threshold / 3.0,
        sma_window: cfg.sma_window,
    };
    tokio::spawn(async move {
        match scheduler::EquityScheduler::new(
            scheduler_pool,
            symbol,
            &zmq_endpoint,
            norm_stats,
            feature_window_size,
            eq_strategy_params,
            executor,
        ).await {
            Ok(mut sched) => {
                if let Err(e) = sched.run().await {
                    error!("equity scheduler fatal error: {e:#}");
                }
            }
            Err(e) => error!("equity scheduler init error: {e}"),
        }
    });

    // Spawn the hourly LLM regime cache task (D4). Runs in background; never
    // blocks the per-bar path. If OPENROUTER_API_KEY is unset, it's a no-op
    // (reads 0.5 neutral). Configurable via LLM_MODEL, LLM_API_BASE, LLM_CACHE_TTL.
    let llm_cfg = features::llm::LlmRegimeConfig::from_env();
    features::llm::spawn_regime_cache_task(llm_cfg).await;

    // Spawn the hourly actuals-computation task. Runs every hour with a 5-min
    // initial delay so it doesn't race the first scheduler tick. Follows the
    // same pattern as the retention task in engine/src/data/mod.rs.
    let actuals_pool = pool.clone();
    tokio::spawn(async move {
        // First tick 5 minutes from now, then every hour.
        let start = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        let mut interval = tokio::time::interval_at(start, std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            match db::compute_actuals(&actuals_pool).await {
                Ok(n) => info!(updated = n, "actuals: updated predictions"),
                Err(e) => tracing::error!("actuals: compute error: {e:#}"),
            }
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

    // Equities ingestion supervisor (daily cadence). This blocks until a
    // fatal error. It keeps QQQ/constituents/macro series fresh by re-pulling
    // only the most recent trading days each day. (Crypto WS loop is retired;
    // daily bars need no streaming feed.)
    if let Err(e) = data::run_equities_ingestion(pool).await {
        eprintln!("equities ingestion fatal error: {:#}", e);
        process::exit(1);
    }
}
