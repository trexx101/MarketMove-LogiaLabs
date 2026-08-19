mod api;
mod bridge;
mod config;
mod data;
mod db;
mod exec;
mod features;
mod hyperopt;
mod normalize;
mod market_hours;
mod options;
mod options_recorder;
mod options_scheduler;
// `parity` is referenced indirectly through `config::Config::from_env` (the
// live-mode guard). The bin doesn't use any parity items directly.
#[allow(dead_code)]
mod parity;
mod scheduler;
mod strategy;
mod strategy_lab;
mod totp;

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

    // Telemetry broadcast channel — published to by the scheduler and paper
    // executor, consumed by WebSocket clients at /api/v1/ws.
    let (tx, _rx) = tokio::sync::broadcast::channel(64);

    // Phase 3.4: TOTP secret for the runtime mode toggle. If TOTP_SECRET is
    // unset, generate a fresh one and log the otpauth URL so the operator
    // can scan it with their authenticator app. Persist via the TOTP_SECRET
    // env var (or a restricted-permission file) before the next restart.
    let (totp_secret, _totp_was_generated) = match totp::load_or_generate_secret() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("TOTP setup error: {:#}", e);
            process::exit(1);
        }
    };
    if _totp_was_generated {
        match totp::otpauth_url(&totp_secret, totp::ISSUER, totp::ACCOUNT_LABEL) {
            Ok(url) => {
                warn!(
                    "TOTP_SECRET was empty — generated a fresh secret.\n\
                     Scan this otpauth URL with your authenticator app, then\n\
                     persist TOTP_SECRET={} in your env before next restart:\n  {}",
                    totp_secret, url
                );
            }
            Err(e) => {
                warn!(error = %e, "failed to construct otpauth URL for fresh TOTP secret");
            }
        }
    } else {
        info!("TOTP_SECRET loaded from env");
    }

    // Construct executor based on trading mode.
    // NOTE: Kraken is retired (Wave 5). The engine runs paper mode only by default.
    // Live mode is now wired to Moomoo OpenD (Phase 3.3) for QQQ daily equities.
    // Selected via LIVE_EXECUTOR=moomoo (default: paper fallback for safety).
    //
    // Phase 3.4: the executor is wrapped in `Arc<RwLock<...>>` so the runtime
    // mode-toggle endpoint can swap Paper <-> Moomoo while the scheduler is
    // running. The scheduler reads the current value at each cycle.
    let executor = std::sync::Arc::new(tokio::sync::RwLock::new(
        build_executor_for_mode(
            cfg.trading_mode,
            &cfg,
            pool.clone(),
            tx.clone(),
        )
        .await,
    ));

    // Shared trading mode (Phase 3.4). The runtime mode toggle flips this
    // value; the scheduler reads it at each cycle to decide which executor
    // entry to use. Initial value mirrors Config::trading_mode.
    let trading_mode = std::sync::Arc::new(tokio::sync::RwLock::new(cfg.trading_mode));

    // Run the equities REST backfill synchronously BEFORE spawning the
    // ingestion supervisor. This seeds QQQ + constituents + macro history so
    // downstream features have enough lookback. Idempotent: re-runs top up
    // only missing rows. (Crypto data source is retired — Wave A equities.)
    if let Err(e) = data::backfill_equities(&pool, 3 * 24 * 3600).await {
        eprintln!("equities backfill fatal error: {:#}", e);
        process::exit(1);
    }

    // Warn if the primary symbol's latest candle is significantly behind.
    // A weekend gap is expected (no trading); anything beyond ~3 calendar days
    // at startup indicates a prior data-fetch failure.
    let now_ts = chrono::Utc::now().timestamp();
    if let Ok(Some(latest)) = db::latest_equity_candle_ts(&pool, &cfg.symbol).await {
        let age_h = (now_ts - latest) / 3600;
        if age_h > 72 {
            eprintln!(
                "WARNING: {symbol} latest candle is {age_h}h old (ts={latest}). \
                Data may be stale — check network access to Yahoo Finance.",
                symbol = cfg.symbol,
            );
            warn!(symbol = %cfg.symbol, latest_ts = latest, age_h, "candle data is stale at startup");
        }
    }

    // Spawn the daily equities scheduler (inference pipeline) as a background task.
    // Wave C: replaces the crypto hourly scheduler with a daily QQQ pipeline.
    let scheduler_pool = pool.clone();
    let zmq_endpoint = cfg.zmq_endpoint.clone();
    let feature_window_size = cfg.feature_window_size;
    let symbol = cfg.symbol.clone();
    let strategy_params = std::sync::Arc::new(tokio::sync::RwLock::new(
        strategy::EquityStrategyParams {
            entry_threshold: cfg.entry_threshold,
            exit_threshold: cfg.exit_threshold,
            sma_window: cfg.sma_window,
            enable_shorting: cfg.enable_shorting,
            short_entry_threshold: cfg.short_entry_threshold,
            short_exit_threshold: cfg.short_exit_threshold,
            pred_5d_filter: cfg.pred_5d_filter,
        },
    ));
    let scheduler_tx = tx.clone();
    let scheduler_trading_mode = trading_mode.clone();
    let scheduler_executor = executor.clone();
    let scheduler_strategy_params = strategy_params.clone();
    tokio::spawn(async move {
        match scheduler::EquityScheduler::new(
            scheduler_pool,
            symbol,
            &zmq_endpoint,
            norm_stats,
            feature_window_size,
            scheduler_strategy_params,
            scheduler_trading_mode,
            scheduler_executor,
            Some(scheduler_tx),
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

    // Build and spawn the Axum telemetry server. We pass an updated Config
    // with the resolved TOTP secret (which may have been generated above)
    // so the API endpoint can verify submitted codes.
    let mut cfg_for_api = cfg.clone();
    cfg_for_api.totp_secret = totp_secret.clone();
    let app = api::router(pool.clone(), &cfg_for_api, tx);
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

/// Build the appropriate executor for a given startup trading mode.
///
/// Phase 3.4: this is only called at startup with the initial mode. The
/// runtime swap (Paper -> Moomoo) happens via `POST /api/mode` while the
/// engine is running, with the scheduler picking up the new executor at
/// its next cycle.
async fn build_executor_for_mode(
    mode: config::TradingMode,
    cfg: &config::Config,
    pool: db::DbPool,
    tx: tokio::sync::broadcast::Sender<api::ws::TelemetryEvent>,
) -> exec::ExecutorKind {
    match mode {
        config::TradingMode::Paper => {
            info!(fee = cfg.paper_fee, "using paper executor");
            exec::ExecutorKind::Paper(exec::paper::PaperExecutor::new_for_symbol(
                pool,
                cfg.paper_fee,
                &cfg.symbol,
                &cfg.short_symbol,
                Some(tx),
            ))
        }
        config::TradingMode::Live => {
            let live_kind = std::env::var("LIVE_EXECUTOR").unwrap_or_else(|_| "paper".to_string());
            match live_kind.to_lowercase().as_str() {
                "moomoo" => {
                    let trd_env = exec::moomoo::TrdEnv::from_env();
                    info!(
                        trd_env = ?trd_env,
                        symbol = %cfg.symbol,
                        short_symbol = %cfg.short_symbol,
                        "using Moomoo OpenD executor (Phase 3.3)"
                    );
                    if trd_env.is_real() {
                        warn!(
                            "MOOMOO_TRD_ENV=REAL — orders will execute against the REAL account. \
                             Ensure OpenD is unlocked and parity is fresh (<7 days)."
                        );
                    }
                    let mut moo = exec::moomoo::MoomooExecutor::new(
                        cfg.symbol.clone(),
                        cfg.short_symbol.clone(),
                        trd_env,
                    );
                    if let Ok(firm) = std::env::var("MOOMOO_SECURITY_FIRM") {
                        moo.security_firm = Some(firm);
                    }
                    if let Ok(acc) = std::env::var("MOOMOO_ACC_ID") {
                        if let Ok(n) = acc.parse::<i64>() {
                            moo.acc_id = Some(n);
                        }
                    }
                    exec::ExecutorKind::Moomoo(moo)
                }
                _ => {
                    warn!(
                        "TRADING_MODE=live but LIVE_EXECUTOR is not 'moomoo' (got '{}'); \
                         falling back to paper executor for safety.",
                        live_kind
                    );
                    exec::ExecutorKind::Paper(exec::paper::PaperExecutor::new_for_symbol(
                        pool,
                        cfg.paper_fee,
                        &cfg.symbol,
                        &cfg.short_symbol,
                        Some(tx),
                    ))
                }
            }
        }
    }
}
