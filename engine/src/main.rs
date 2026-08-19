mod advisor;
mod api;
mod bridge;
mod config;
mod data;
mod db;
mod event;
mod exec;
mod archive;
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

use crate::db::TradingModel;

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

    // Phase 3.4 — runtime mode toggle. Each scheduler holds its own
    // `Arc<RwLock<ExecutorKind>>` (built per model in the loop below) and
    // shares this `Arc<RwLock<TradingMode>>`. The `/api/mode` endpoint flips
    // the mode value; each scheduler re-reads it at the start of its next
    // cycle. (Live Moomoo swap is still single-model and gated behind
    // explicit configuration; per-model live swap is a future story.)
    let trading_mode = std::sync::Arc::new(tokio::sync::RwLock::new(cfg.trading_mode));

    // Event logger — wraps DB + telemetry sender + mode ref for unified
    // event persistence and broadcast.
    let event_logger = std::sync::Arc::new(crate::event::EventLogger::new(
        pool.clone(),
        Some(tx.clone()),
        trading_mode.clone(),
    ));

    // Emit engine-started event.
    event_logger
        .emit(crate::event::EngineEvent::engine_started(
            cfg.trading_mode,
            &cfg.symbol,
        ))
        .await;

    // Run the equities REST backfill synchronously BEFORE spawning the
    // ingestion supervisor. This seeds QQQ + constituents + macro history so
    // downstream features have enough lookback. Idempotent: re-runs top up
    // only missing rows. (Crypto data source is retired — Wave A equities.)
    if let Err(e) = data::backfill_equities(&pool, 3 * 24 * 3600).await {
        eprintln!("equities backfill fatal error: {:#}", e);
        process::exit(1);
    }

    // Seed sentiment cache (Phase 4 — Finnhub). Runs at startup so the
    // advisor has sentiment data available from day one. Falls back to
    // stub if FINNHUB_API_KEY is missing.
    if let Err(e) = data::sentiment::seed_sentiment_cache(&pool, data::EQUITY_SYMBOLS).await {
        eprintln!("sentiment seed warning: {:#}", e);
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

    // Resolve the set of trading models this engine will run (§8).
    //
    // If the `trading_models` registry has at least one row, every enabled
    // row becomes an independent (scheduler, executor) pair. If the
    // registry is empty (cold start, fresh DB, or operator removed all
    // rows), fall back to a single bootstrap model built from
    // Config::symbol + Config::short_symbol + Config::norm_stats_path so
    // existing paper-mode behavior survives unchanged.
    let (active_models, _loaded_count) = match db::resolve_active_models(
        &pool,
        &cfg.symbol,
        &cfg.short_symbol,
        &cfg.norm_stats_path,
    )
    .await
    {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("trading_models resolver error: {:#}", e);
            process::exit(1);
        }
    };
    if active_models.first().map(|m| m.model_id.as_str()) == Some("bootstrap-default") {
        info!(
            symbol = %cfg.symbol,
            short_symbol = %cfg.short_symbol,
            "trading_models registry is empty — bootstrapping a single model from Config defaults"
        );
    } else {
        info!(
            count = active_models.len(),
            "loaded enabled models from trading_models registry"
        );
    }

    // Per-model strategy params map (§8.6). Populated in the per-model
    // loop below, then handed to the API router so PUT /api/strategy-config
    // ?model_id=X can target a specific model's running scheduler.
    let strategy_params_by_model: std::sync::Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<String, std::sync::Arc<tokio::sync::RwLock<strategy::EquityStrategyParams>>>,
        >,
    > = std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));

    // Spawn one daily equities scheduler per resolved model. Each model
    // gets its own norm_stats file, its own paper executor (configured
    // for that primary+short pair), and its own strategy_params handle
    // so per-model threshold tuning can land in a follow-up without
    // touching this loop. The executor variable still exists below for
    // the legacy `/api/mode` flip path; we keep the FIRST model as the
    // canonical handle that the API mutates.
    for (idx, model) in active_models.iter().enumerate() {
        let model_label = model.pair();
        info!(
            model_idx = idx,
            model_id = %model.model_id,
            primary = %model.primary_symbol,
            inverse = %model.inverse_symbol,
            budget_usd = model.budget_usd,
            norm_stats = %model.norm_stats_path,
            "bootstrapping scheduler"
        );

        // Per-model norm_stats — fail fast on missing file for any model.
        let model_norm_stats =
            match features::equities_v2::EquityNormStats::load_named(&model.norm_stats_path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "norm_stats error for {model_label}: {:#}",
                        e
                    );
                    process::exit(1);
                }
            };

        // Per-model paper executor. Each executor holds its own position
        // state keyed by primary_symbol, so two schedulers trade QQQ and
        // NVDA independently without contention.
        let model_tx_for_exec = tx.clone();
        let model_executor = std::sync::Arc::new(tokio::sync::RwLock::new(
            build_paper_executor_for_model(
                &cfg,
                pool.clone(),
                &model.primary_symbol,
                &model.inverse_symbol,
                model_tx_for_exec,
            ),
        ));

        let model_pool = pool.clone();
        let zmq_endpoint = cfg.zmq_endpoint.clone();
        let feature_window_size = cfg.feature_window_size;
        let model_strategy_params = std::sync::Arc::new(tokio::sync::RwLock::new(
            strategy::EquityStrategyParams {
                entry_threshold: cfg.entry_threshold,
                exit_threshold: cfg.exit_threshold,
                sma_window: cfg.sma_window,
                enable_shorting: cfg.enable_shorting,
                short_entry_threshold: cfg.short_entry_threshold,
                short_exit_threshold: cfg.short_exit_threshold,
                pred_5d_filter: cfg.pred_5d_filter,
                enable_sentiment_overlay: false,
                sentiment_reduce_threshold: -0.5,
                sentiment_exit_threshold: -0.8,
                sentiment_min_articles: 15,
            },
        ));
        // §8.6: register this model's params in the shared map so the
        // API can target it via PUT /api/strategy-config?model_id=X.
        strategy_params_by_model
            .write()
            .await
            .insert(model.model_id.clone(), model_strategy_params.clone());
        let model_trading_mode = trading_mode.clone();
        let model_strategy_params_clone = model_strategy_params.clone();
        let model_event_logger = event_logger.clone();
        // Clone fields we need into owned strings so the spawned task
        // doesn't borrow from `active_models`.
        let model_primary = model.primary_symbol.clone();
        let model_id_for_scheduler = model.model_id.clone();
        let model_pair_for_scheduler = model.pair();
        let model_tx = tx.clone();
        tokio::spawn(async move {
            match scheduler::EquityScheduler::new(
                model_pool,
                model_primary,
                model_id_for_scheduler,
                model_pair_for_scheduler,
                &zmq_endpoint,
                model_norm_stats,
                feature_window_size,
                model_strategy_params_clone,
                model_trading_mode,
                model_executor,
                Some(model_tx),
                Some(model_event_logger),
            )
            .await
            {
                Ok(mut sched) => {
                    if let Err(e) = sched.run().await {
                        error!(model = %model_label, "equity scheduler fatal error: {e:#}");
                    }
                }
                Err(e) => error!(model = %model_label, "equity scheduler init error: {e}"),
            }
        });
    }

    // Spawn the hourly LLM regime cache task (D4). Runs in background; never
    // blocks the per-bar path. If OPENROUTER_API_KEY is unset, it's a no-op
    // (reads 0.5 neutral). Configurable via LLM_MODEL, LLM_API_BASE, LLM_CACHE_TTL.
    let llm_cfg = features::llm::LlmRegimeConfig::from_env();
    features::llm::spawn_regime_cache_task(llm_cfg).await;

    // Phase 4: Advisor — daily pre-market briefing task.
    let advisor_cfg = advisor::AdvisorConfig::from_env(&cfg);
    let advisor_state = if advisor_cfg.is_enabled() {
        let state = std::sync::Arc::new(advisor::AdvisorState::new(advisor_cfg, tx.clone()));
        let briefing_pool = pool.clone();
        let briefing_symbol = cfg.symbol.clone();
        let briefing_state = state.clone();
        let notify = state.notify.clone();
        tokio::spawn(async move {
            advisor::briefing::run_briefing_loop(
                briefing_state,
                briefing_pool,
                briefing_symbol,
                notify,
            ).await;
        });
        info!("advisor background task started");
        Some(state)
    } else {
        info!("advisor disabled — no OPENROUTER_API_KEY or config");
        None
    };

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
    let app = api::router(pool.clone(), &cfg_for_api, tx, advisor_state, event_logger, strategy_params_by_model);
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
/// Build the paper executor for a specific resolved model (§8).
///
/// Each per-model loop iteration in `main()` calls this with the model's
/// own `primary_symbol` and `inverse_symbol`, so QQQ and NVDA schedulers
/// run independent paper executors. Currently always returns Paper;
/// live-Moomoo execution stays single-model and operator-gated.
fn build_paper_executor_for_model(
    cfg: &config::Config,
    pool: db::DbPool,
    primary_symbol: &str,
    inverse_symbol: &str,
    tx: tokio::sync::broadcast::Sender<api::ws::TelemetryEvent>,
) -> exec::ExecutorKind {
    info!(
        fee = cfg.paper_fee,
        primary = %primary_symbol,
        inverse = %inverse_symbol,
        "using paper executor for model"
    );
    exec::ExecutorKind::Paper(exec::paper::PaperExecutor::new_for_symbol(
        pool,
        cfg.paper_fee,
        primary_symbol,
        inverse_symbol,
        Some(tx),
    ))
}

#[allow(dead_code)]
/// Legacy helper retained for the Moomoo live execution path (§3.3).
/// Not used by the per-model bootstrap loop in `main()`. Will be wired
/// back in once the live execution path supports per-model routing.
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
