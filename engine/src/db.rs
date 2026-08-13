use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use sqlx::{sqlite::SqlitePoolOptions, FromRow, Row, SqlitePool};
use tracing::info;

pub type DbPool = SqlitePool;

pub const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS candles (
    ts           INTEGER PRIMARY KEY,
    open         REAL    NOT NULL,
    high         REAL    NOT NULL,
    low          REAL    NOT NULL,
    close        REAL    NOT NULL,
    volume       REAL    NOT NULL,
    vwap         REAL    NOT NULL,
    funding_rate REAL    NOT NULL DEFAULT 0,
    basis_z      REAL    NOT NULL DEFAULT 0,
    ob_imbalance REAL    NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS candles_ts_idx ON candles (ts DESC);
CREATE TABLE IF NOT EXISTS predictions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    candle_ts     INTEGER NOT NULL UNIQUE,
    pred_1h       REAL    NOT NULL,
    pred_4h       REAL    NOT NULL,
    pred_24h      REAL    NOT NULL,
    features_json TEXT    NOT NULL,
    created_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS predictions_candle_ts_idx ON predictions (candle_ts DESC);
CREATE TABLE IF NOT EXISTS signal_state (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    position   INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS positions (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    model_id   TEXT    NOT NULL DEFAULT '',
    symbol     TEXT    NOT NULL DEFAULT '',
    candle_ts  INTEGER NOT NULL,
    position   INTEGER NOT NULL,
    pred_4h    REAL    NOT NULL,
    pred_24h   REAL    NOT NULL,
    regime     INTEGER NOT NULL,
    sma        REAL    NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS positions_candle_ts_idx ON positions (candle_ts DESC);
CREATE TABLE IF NOT EXISTS trades (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    candle_ts     INTEGER NOT NULL,
    side          TEXT    NOT NULL,
    qty           REAL    NOT NULL,
    price         REAL    NOT NULL,
    fee           REAL    NOT NULL,
    realized_pnl  REAL    NOT NULL,
    created_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS trades_candle_ts_idx ON trades (candle_ts DESC);

-- Wave A (2026-07-22): equities data layer.
-- Independent of crypto `candles` table. Coexists for the transition period.
CREATE TABLE IF NOT EXISTS equity_candles (
    symbol TEXT    NOT NULL,
    ts     INTEGER NOT NULL,
    open   REAL    NOT NULL,
    high   REAL    NOT NULL,
    low    REAL    NOT NULL,
    close  REAL    NOT NULL,
    volume INTEGER NOT NULL DEFAULT 0,
    source TEXT    NOT NULL DEFAULT 'yahoo',
    PRIMARY KEY (symbol, ts)
);
CREATE INDEX IF NOT EXISTS equity_candles_symbol_ts_idx
    ON equity_candles (symbol, ts DESC);

CREATE TABLE IF NOT EXISTS equity_predictions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    symbol        TEXT    NOT NULL,
    candle_ts     INTEGER NOT NULL,
    pred_1d       REAL    NOT NULL,
    pred_5d       REAL    NOT NULL,
    pred_21d      REAL    NOT NULL,
    regime        TEXT    NOT NULL DEFAULT 'unknown',
    features_json TEXT    NOT NULL DEFAULT '{}',
    created_at    INTEGER NOT NULL,
    source        TEXT    NOT NULL DEFAULT 'qqq_tcn_v1',
    UNIQUE(symbol, candle_ts)
);
CREATE INDEX IF NOT EXISTS equity_predictions_ts_idx
    ON equity_predictions (candle_ts DESC);

CREATE TABLE IF NOT EXISTS equity_trades (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    model_id      TEXT    NOT NULL DEFAULT '',
    symbol        TEXT    NOT NULL,
    candle_ts     INTEGER NOT NULL,
    side          TEXT    NOT NULL,
    qty           REAL    NOT NULL,
    price         REAL    NOT NULL,
    fee           REAL    NOT NULL,
    realized_pnl  REAL    NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS equity_trades_symbol_ts_idx
    ON equity_trades (symbol, candle_ts DESC);

CREATE TABLE IF NOT EXISTS equity_ingest_state (
    source       TEXT    NOT NULL,
    symbol       TEXT    NOT NULL,
    last_ts      INTEGER NOT NULL DEFAULT 0,
    last_run_at  INTEGER NOT NULL DEFAULT 0,
    rows_loaded  INTEGER NOT NULL DEFAULT 0,
    error_count  INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT    NOT NULL DEFAULT '',
    PRIMARY KEY (source, symbol)
);

-- Sentiment cache (Phase 2: Finnhub). Stores daily aggregate sentiment for
-- each equity symbol. Stub returns neutral 0.5 until Finnhub is wired.
CREATE TABLE IF NOT EXISTS sentiment_cache (
    symbol       TEXT    NOT NULL,
    date         TEXT    NOT NULL,  -- YYYY-MM-DD
    score        REAL    NOT NULL DEFAULT 0.5,  -- [-1, 1] — neg → pos
    source       TEXT    NOT NULL DEFAULT 'stub',
    buzz         INTEGER NOT NULL DEFAULT 0,     -- articles in last week
    weekly_avg   REAL    NOT NULL DEFAULT 0.0,   -- Finnhub weeklyAverage
    PRIMARY KEY (symbol, date)
);

-- Control Room revamp: strategy configs, mode switches, advisor log, backtests.
CREATE TABLE IF NOT EXISTS strategy_configs (
    id            TEXT    PRIMARY KEY,
    name          TEXT    NOT NULL UNIQUE,
    strategy_type TEXT    NOT NULL,
    script_body   TEXT,
    params_json   TEXT    NOT NULL,
    is_active     INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

-- Phase 4: Advisor briefing log. One row per successful or attempted briefing.
CREATE TABLE IF NOT EXISTS advisor_briefing_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts           INTEGER NOT NULL,
    for_date     TEXT    NOT NULL,
    model_used   TEXT    NOT NULL,
    context_hash TEXT    NOT NULL,
    context_json TEXT    NOT NULL,
    briefing_json TEXT   NOT NULL,
    parse_status TEXT    NOT NULL,
    latency_ms   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS advisor_briefing_log_date_idx
    ON advisor_briefing_log (for_date DESC);

-- Phase 4: Advisor chat log. Multi-turn history source for conversational chat.
CREATE TABLE IF NOT EXISTS advisor_chat_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts           INTEGER NOT NULL,
    question     TEXT    NOT NULL,
    response     TEXT    NOT NULL,
    model_used   TEXT    NOT NULL,
    latency_ms   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS advisor_chat_log_ts_idx
    ON advisor_chat_log (ts DESC);

CREATE TABLE IF NOT EXISTS mode_switches (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    previous_mode         TEXT    NOT NULL,
    new_mode              TEXT    NOT NULL,
    parity_marker_age_secs INTEGER NOT NULL,
    authorized_by         TEXT    NOT NULL,
    timestamp             INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS advisor_log (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    interaction_type   TEXT    NOT NULL,
    prompt_context_json TEXT   NOT NULL,
    model_used         TEXT    NOT NULL,
    response_json      TEXT    NOT NULL,
    suggested_action   TEXT,
    timestamp          INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS backtest_results (
    id                TEXT    PRIMARY KEY,
    strategy_id       TEXT    NOT NULL,
    start_ts          INTEGER NOT NULL,
    end_ts            INTEGER NOT NULL,
    metrics_json      TEXT    NOT NULL,
    equity_curve_json TEXT    NOT NULL,
    timestamp         INTEGER NOT NULL,
    FOREIGN KEY(strategy_id) REFERENCES strategy_configs(id) ON DELETE CASCADE
);

-- Event log: unified append-only record of trades, data fetch failures,
-- strategy changes, mode switches, alerts, and advisor actions.
-- Archived to /app/data/events_archive/ after retention period.
CREATE TABLE IF NOT EXISTS engine_events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts           INTEGER NOT NULL,
    category     TEXT    NOT NULL,  -- trade | data | system | strategy | alert | advisor
    severity     TEXT    NOT NULL,  -- info | warn | error
    mode         TEXT    NOT NULL,  -- paper | live
    source       TEXT    NOT NULL,  -- scheduler, data::yahoo, exec::paper, api::mode, etc.
    message      TEXT    NOT NULL,
    payload_json TEXT    NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS engine_events_ts_idx ON engine_events (ts DESC);
CREATE INDEX IF NOT EXISTS engine_events_category_ts_idx ON engine_events (category, ts DESC);

-- Multi-model registry (§8 plan amendment 2026-08-05).
-- The trading unit of meaning is the model, not the symbol. Each row owns
-- a primary+inverse symbol pair, the path to its trained artifact, the
-- dollar budget allocated to it, and its enable/disable state. The engine
-- spawns one EquityScheduler + PaperExecutor per enabled=1 row at startup
-- while the Config symbol defaults are used as a cold-start fallback when
-- this table is empty.
CREATE TABLE IF NOT EXISTS trading_models (
    model_id        TEXT    PRIMARY KEY,        -- uuid text, not a ticker
    primary_symbol  TEXT    NOT NULL,           -- e.g. NVDA
    inverse_symbol  TEXT    NOT NULL,           -- e.g. NVDD
    model_path      TEXT    NOT NULL,           -- path to model bundle
    norm_stats_path TEXT    NOT NULL,           -- path to norm stats json
    budget_usd      REAL    NOT NULL DEFAULT 5000.0,
    enabled         INTEGER NOT NULL DEFAULT 1,
    deployed_at     INTEGER NOT NULL,
    last_wf_ic      REAL,
    last_wf_at      INTEGER,
    notes           TEXT
);
CREATE INDEX IF NOT EXISTS trading_models_enabled_idx
    ON trading_models (enabled);
CREATE INDEX IF NOT EXISTS trading_models_primary_symbol_idx
    ON trading_models (primary_symbol);
"#;

/// A single OHLCV + VWAP candle as stored in the database.
#[derive(Debug, Clone)]
pub struct Candle {
    /// Unix timestamp (seconds) of the candle's open time.
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub vwap: f64,
    /// Binance perpetual funding rate (per funding interval). 0.0 if unknown.
    pub funding_rate: f64,
    /// Spot-vs-perp basis Z-score (computed from spot vs futures price). 0.0 if unknown.
    pub basis_z: f64,
    /// Order-book imbalance from the depth stream in [-1, 1]. 0.0 if unknown.
    pub ob_imbalance: f64,
}

/// A single equity OHLCV candle as stored in the `equity_candles` table.
/// Wave A: separate type from crypto `Candle` to keep the two pipelines
/// isolated during the transition.
#[derive(Debug, Clone, FromRow)]
pub struct EquityCandle {
    /// Ticker symbol: 'QQQ', 'AAPL', '^VIX', 'TLT', 'GLD', 'NVDA', ...
    pub symbol: String,
    /// Unix timestamp (seconds) at midnight UTC of the trading day.
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: i64,
    /// 'yahoo' | 'moomoo' | 'fred'
    pub source: String,
}

/// A row from the `equity_predictions` table, used with sqlx::query_as.
#[derive(Debug, Clone, FromRow)]
pub struct EquityPredictionRow {
    pub id: i64,
    pub symbol: String,
    pub candle_ts: i64,
    pub pred_1d: f64,
    pub pred_5d: f64,
    pub pred_21d: f64,
    pub regime: String,
    pub features_json: String,
    pub created_at: i64,
    pub source: String,
}

/// A row from the `trading_models` registry (§8 plan amendment 2026-08-05).
///
/// The trading unit of meaning is the model, not the symbol. Each row
/// owns a primary+inverse symbol pair, the path to its trained artifact,
/// the dollar budget allocated to it, and its enable/disable state.
///
/// `pair` is a derived display label (e.g. `"QQQ/PSQ"`, `"NVDA/NVDD"`),
/// computed by `TradingModel::pair()`.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct TradingModel {
    pub model_id: String,
    pub primary_symbol: String,
    pub inverse_symbol: String,
    pub model_path: String,
    pub norm_stats_path: String,
    pub budget_usd: f64,
    pub enabled: bool,
    pub deployed_at: i64,
    pub last_wf_ic: Option<f64>,
    pub last_wf_at: Option<i64>,
    pub notes: Option<String>,
}

impl TradingModel {
    /// Derived display label: `"<PRIMARY>/<INVERSE>"`, uppercase.
    pub fn pair(&self) -> String {
        format!("{}/{}", self.primary_symbol, self.inverse_symbol).to_uppercase()
    }

    /// Whether this model is currently active in the registry.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Open (or create) the SQLite database and apply the startup DDL.
pub async fn open(database_url: &str) -> Result<DbPool> {
    // SQLite URLs like `sqlite://data/candles.db` need the parent directory to exist.
    if let Some(path) = database_url.strip_prefix("sqlite://") {
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating database directory {:?}", parent))?;
            }
        }
        // sqlx 0.8 does not auto-create the .db file on connect — only the
        // directory above. A missing file causes "unable to open database
        // file" (code 14). Pre-create an empty file so the first connect
        // succeeds on a fresh deploy.
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("creating database file {path}"))?;
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect(database_url)
        .await
        .with_context(|| format!("connecting to SQLite at {database_url}"))?;

    for stmt in DDL.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        sqlx::query(stmt)
            .execute(&pool)
            .await
            .with_context(|| format!("running DDL: {stmt}"))?;
    }

    migrate_predictions(&pool).await?;
    migrate_sentiment_cache(&pool).await?;
    migrate_trading_models(&pool).await?;
    migrate_multi_model(&pool).await?;

    info!("database ready at {database_url}");
    Ok(pool)
}

/// Add nullable `actual_*` columns to the predictions table if they don't exist.
/// SQLite does not support `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`, so we
/// query `PRAGMA table_info` first and only add missing columns.
pub async fn migrate_predictions(pool: &DbPool) -> Result<()> {
    let rows = sqlx::query("PRAGMA table_info(predictions)")
        .fetch_all(pool)
        .await
        .context("PRAGMA table_info(predictions)")?;

    let existing: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();

    for col in &["actual_1h", "actual_4h", "actual_24h"] {
        if !existing.iter().any(|name| name == col) {
            let sql = format!("ALTER TABLE predictions ADD COLUMN {col} REAL");
            sqlx::query(&sql)
                .execute(pool)
                .await
                .with_context(|| format!("adding column {col}"))?;
            info!("migrated predictions: added column {col}");
        }
    }

    migrate_candles(pool).await?;

    Ok(())
}

/// Add the V2 feature columns to `candles` if they don't exist (Wave 5).
pub async fn migrate_candles(pool: &DbPool) -> Result<()> {
    let rows = sqlx::query("PRAGMA table_info(candles)")
        .fetch_all(pool)
        .await
        .context("PRAGMA table_info(candles)")?;
    let existing: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();

    for col in &["funding_rate", "basis_z", "ob_imbalance"] {
        if !existing.iter().any(|name| name == col) {
            let sql = format!("ALTER TABLE candles ADD COLUMN {col} REAL NOT NULL DEFAULT 0");
            sqlx::query(&sql)
                .execute(pool)
                .await
                .with_context(|| format!("adding column candles.{col}"))?;
            info!("migrated candles: added column {col}");
        }
    }

    Ok(())
}

/// Add `buzz` and `weekly_avg` columns to sentiment_cache if they don't exist
/// (Phase 4 migration). Follows the same pattern as migrate_candles.
pub async fn migrate_sentiment_cache(pool: &DbPool) -> Result<()> {
    let rows = sqlx::query("PRAGMA table_info(sentiment_cache)")
        .fetch_all(pool)
        .await
        .context("PRAGMA table_info(sentiment_cache)")?;
    let existing: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();

    for (col, col_type, default) in &[
        ("buzz", "INTEGER", "0"),
        ("weekly_avg", "REAL", "0.0"),
    ] {
        if !existing.iter().any(|name| name == col) {
            let sql = format!(
                "ALTER TABLE sentiment_cache ADD COLUMN {col} {col_type} NOT NULL DEFAULT {default}"
            );
            sqlx::query(&sql)
                .execute(pool)
                .await
                .with_context(|| format!("adding column sentiment_cache.{col}"))?;
            info!("migrated sentiment_cache: added column {col}");
        }
    }

    // Fix: if weekly_avg was previously created as INTEGER, recreate it as REAL.
    let col_info: Vec<(String, String)> = rows
        .iter()
        .map(|r| (r.get::<String, _>(1), r.get::<String, _>(2)))
        .collect();
    if let Some(t) = col_info.iter().find(|(name, _)| name == "weekly_avg") {
        if t.1.to_uppercase() != "REAL" {
            info!("sentiment_cache.weekly_avg has wrong type ({}) — recreating as REAL", t.1);
            sqlx::query("ALTER TABLE sentiment_cache RENAME COLUMN weekly_avg TO weekly_avg_old")
                .execute(pool)
                .await
                .context("rename weekly_avg")?;
            sqlx::query("ALTER TABLE sentiment_cache ADD COLUMN weekly_avg REAL NOT NULL DEFAULT 0.0")
                .execute(pool)
                .await
                .context("add weekly_avg REAL")?;
            sqlx::query("UPDATE sentiment_cache SET weekly_avg = CAST(weekly_avg_old AS REAL)")
                .execute(pool)
                .await
                .context("copy weekly_avg data")?;
            sqlx::query("ALTER TABLE sentiment_cache DROP COLUMN weekly_avg_old")
                .execute(pool)
                .await
                .context("drop weekly_avg_old")?;
            info!("migrated sentiment_cache: weekly_avg type fixed to REAL");
        }
    }
    Ok(())
}

/// Idempotent migration for the `trading_models` registry
/// (§8 plan amendment 2026-08-05).
///
/// The base table is created via the `DDL` block; this function is the
/// additive-column hook for future schema changes (same PRAGMA table_info
/// pattern as `migrate_sentiment_cache`).
pub async fn migrate_trading_models(pool: &DbPool) -> Result<()> {
    let _ = pool; // no pending migrations; placeholder for future use
    Ok(())
}

/// §8 multi-model migration: add model_id/symbol columns to positions and
/// model_id to equity_trades if they are missing.
pub async fn migrate_multi_model(pool: &DbPool) -> Result<()> {
    // positions(model_id, symbol)
    let rows = sqlx::query("PRAGMA table_info(positions)")
        .fetch_all(pool)
        .await
        .context("PRAGMA table_info(positions)")?;
    let existing: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();

    for (col, col_type, default) in &[("model_id", "TEXT", "''"), ("symbol", "TEXT", "''")] {
        if !existing.iter().any(|name| name == col) {
            let sql = format!(
                "ALTER TABLE positions ADD COLUMN {col} {col_type} NOT NULL DEFAULT {default}"
            );
            sqlx::query(&sql)
                .execute(pool)
                .await
                .with_context(|| format!("adding column positions.{col}"))?;
            info!("migrated positions: added column {col}");
        }
    }

    // equity_trades(model_id)
    let rows = sqlx::query("PRAGMA table_info(equity_trades)")
        .fetch_all(pool)
        .await
        .context("PRAGMA table_info(equity_trades)")?;
    let existing: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();

    if !existing.iter().any(|name| name == "model_id") {
        sqlx::query("ALTER TABLE equity_trades ADD COLUMN model_id TEXT NOT NULL DEFAULT ''")
            .execute(pool)
            .await
            .context("adding column equity_trades.model_id")?;
        info!("migrated equity_trades: added column model_id");
    }

    Ok(())
}

/// Latest cached sentiment for a symbol, if any.
///
/// Returns (score, buzz) for the most recent row (by date) for the symbol.
/// If the table is empty, returns None so callers can fall back to stub.
pub async fn latest_sentiment(pool: &DbPool, symbol: &str) -> Result<Option<(f64, i64)>> {
    let row: Option<(f64, i64)> = sqlx::query_as(
        "SELECT score, buzz FROM sentiment_cache WHERE symbol = ?1 ORDER BY date DESC LIMIT 1"
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .context("latest_sentiment")?;
    Ok(row)
}

/// Insert a new model into the `trading_models` registry.
///
/// `model_id` must be a UUID-style string (caller is responsible for
/// generating it; the API layer uses `uuid::Uuid::new_v4()`).
/// Returns the freshly inserted row.
pub async fn register_model(
    pool: &DbPool,
    model_id: &str,
    primary_symbol: &str,
    inverse_symbol: &str,
    model_path: &str,
    norm_stats_path: &str,
    budget_usd: f64,
    notes: Option<&str>,
) -> Result<TradingModel> {
    let deployed_at = Utc::now().timestamp();
    sqlx::query(
        r#"INSERT INTO trading_models
               (model_id, primary_symbol, inverse_symbol, model_path,
                norm_stats_path, budget_usd, enabled, deployed_at,
                last_wf_ic, last_wf_at, notes)
           VALUES (?, ?, ?, ?, ?, ?, 1, ?, NULL, NULL, ?)"#,
    )
    .bind(model_id)
    .bind(primary_symbol)
    .bind(inverse_symbol)
    .bind(model_path)
    .bind(norm_stats_path)
    .bind(budget_usd)
    .bind(deployed_at)
    .bind(notes)
    .execute(pool)
    .await
    .context("register_model: INSERT trading_models")?;

    load_model_by_id(pool, model_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("register_model: row vanished after insert"))
}

/// Toggle the `enabled` flag for a registered model.
///
/// Returns `Ok(None)` if `model_id` does not exist (callers should map
/// that to a 404).
pub async fn update_model_enabled(
    pool: &DbPool,
    model_id: &str,
    enabled: bool,
) -> Result<Option<TradingModel>> {
    let rows = sqlx::query("UPDATE trading_models SET enabled = ? WHERE model_id = ?")
        .bind(if enabled { 1_i64 } else { 0_i64 })
        .bind(model_id)
        .execute(pool)
        .await
        .context("update_model_enabled: UPDATE trading_models")?
        .rows_affected();

    if rows == 0 {
        return Ok(None);
    }
    load_model_by_id(pool, model_id).await
}

/// Fetch a single model by id. Returns `Ok(None)` if the id is unknown.
pub async fn load_model_by_id(pool: &DbPool, model_id: &str) -> Result<Option<TradingModel>> {
    sqlx::query_as::<_, TradingModel>(
        "SELECT model_id, primary_symbol, inverse_symbol, model_path,
                norm_stats_path, budget_usd, enabled, deployed_at,
                last_wf_ic, last_wf_at, notes
           FROM trading_models
          WHERE model_id = ?",
    )
    .bind(model_id)
    .fetch_optional(pool)
    .await
    .context("load_model_by_id")
}

/// Fetch all registered models, newest-first by `deployed_at`.
pub async fn load_all_models(pool: &DbPool) -> Result<Vec<TradingModel>> {
    sqlx::query_as::<_, TradingModel>(
        "SELECT model_id, primary_symbol, inverse_symbol, model_path,
                norm_stats_path, budget_usd, enabled, deployed_at,
                last_wf_ic, last_wf_at, notes
           FROM trading_models
          ORDER BY deployed_at DESC",
    )
    .fetch_all(pool)
    .await
    .context("load_all_models")
}

/// Fetch all models with `enabled = 1`. This is the set the engine
/// bootstraps into schedulers at startup.
pub async fn load_enabled_models(pool: &DbPool) -> Result<Vec<TradingModel>> {
    sqlx::query_as::<_, TradingModel>(
        "SELECT model_id, primary_symbol, inverse_symbol, model_path,
                norm_stats_path, budget_usd, enabled, deployed_at,
                last_wf_ic, last_wf_at, notes
           FROM trading_models
          WHERE enabled = 1
          ORDER BY deployed_at ASC",
    )
    .fetch_all(pool)
    .await
    .context("load_enabled_models")
}

/// Update the `last_wf_ic` / `last_wf_at` columns after a walk-forward run.
pub async fn record_walk_forward_result(
    pool: &DbPool,
    model_id: &str,
    last_wf_ic: Option<f64>,
    last_wf_at: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE trading_models SET last_wf_ic = ?, last_wf_at = ? WHERE model_id = ?",
    )
    .bind(last_wf_ic)
    .bind(last_wf_at)
    .bind(model_id)
    .execute(pool)
    .await
    .context("record_walk_forward_result")?;
    Ok(())
}

/// Synthetic `TradingModel` from Config defaults for cold-start fallback
/// (§8 plan amendment 2026-08-05).
///
/// Used when `trading_models` is empty so existing single-symbol paper
/// deployments survive a fresh DB without operator action. The synthetic
/// `model_id` `"bootstrap-default"` is reserved and must never appear in
/// user-managed registry rows.
pub fn bootstrap_default_model(
    primary_symbol: &str,
    inverse_symbol: &str,
    norm_stats_path: &str,
) -> TradingModel {
    TradingModel {
        model_id: "bootstrap-default".to_string(),
        primary_symbol: primary_symbol.to_string(),
        inverse_symbol: inverse_symbol.to_string(),
        model_path: "<bootstrap>".to_string(),
        norm_stats_path: norm_stats_path.to_string(),
        budget_usd: 0.0,
        enabled: true,
        deployed_at: Utc::now().timestamp(),
        last_wf_ic: None,
        last_wf_at: None,
        notes: Some("bootstrap from Config defaults".to_string()),
    }
}

/// Resolve the set of trading models the engine will run at startup
/// (§8 plan amendment 2026-08-05).
///
/// - If the registry has at least one `enabled = 1` row, those rows are
///   returned in `deployed_at ASC` order.
/// - If the registry is empty, a single synthetic bootstrap model built
///   from the supplied `(primary_symbol, inverse_symbol, norm_stats_path)`
///   is returned.
///
/// `loaded_count` returns the number of registry rows that backed the
/// result (0 for the bootstrap case, >0 when registry rows were used).
/// Useful for logging the cold-start vs. registry path without exposing
/// the resolver's internals to callers.
pub async fn resolve_active_models(
    pool: &DbPool,
    primary_symbol: &str,
    inverse_symbol: &str,
    norm_stats_path: &str,
) -> Result<(Vec<TradingModel>, usize)> {
    let rows = load_enabled_models(pool).await?;
    if rows.is_empty() {
        Ok((
            vec![bootstrap_default_model(
                primary_symbol,
                inverse_symbol,
                norm_stats_path,
            )],
            0,
        ))
    } else {
        let n = rows.len();
        Ok((rows, n))
    }
}

/// Insert or update a candle row identified by its open-time unix timestamp.
pub async fn upsert_candle(pool: &DbPool, c: &Candle) -> Result<()> {
    sqlx::query(
        "INSERT INTO candles (ts, open, high, low, close, volume, vwap, funding_rate, basis_z, ob_imbalance)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(ts) DO UPDATE SET
           open   = excluded.open,
           high   = excluded.high,
           low    = excluded.low,
           close  = excluded.close,
           volume = excluded.volume,
           vwap   = excluded.vwap,
           funding_rate = excluded.funding_rate,
           basis_z      = excluded.basis_z,
           ob_imbalance = excluded.ob_imbalance",
    )
    .bind(c.ts)
    .bind(c.open)
    .bind(c.high)
    .bind(c.low)
    .bind(c.close)
    .bind(c.volume)
    .bind(c.vwap)
    .bind(c.funding_rate)
    .bind(c.basis_z)
    .bind(c.ob_imbalance)
    .execute(pool)
    .await
    .context("upsert_candle")?;
    Ok(())
}

/// Delete candles outside the rolling retention window, keeping the `keep` most recent rows.
pub async fn prune_old(pool: &DbPool, keep: usize) -> Result<u64> {
    let keep = keep as i64;
    let result = sqlx::query(
        "DELETE FROM candles
         WHERE ts NOT IN (SELECT ts FROM candles ORDER BY ts DESC LIMIT ?)",
    )
    .bind(keep)
    .execute(pool)
    .await
    .context("prune_old")?;
    Ok(result.rows_affected())
}

/// Return the current number of candle rows.
pub async fn count_candles(pool: &DbPool) -> Result<i64> {
    let row = sqlx::query("SELECT COUNT(*) FROM candles")
        .fetch_one(pool)
        .await
        .context("count_candles")?;
    Ok(row.get(0))
}

/// Return the most recent candle timestamp (unix seconds), or `None` if the table is empty.
// Used by the feature-computation layer (feature 07) to detect gaps on reconnect.
#[allow(dead_code)]
pub async fn latest_ts(pool: &DbPool) -> Result<Option<i64>> {
    let row = sqlx::query("SELECT MAX(ts) FROM candles")
        .fetch_one(pool)
        .await
        .context("latest_ts")?;
    Ok(row.get(0))
}

/// Upsert a prediction row keyed by candle_ts.
pub async fn insert_prediction(
    pool: &DbPool,
    candle_ts: i64,
    pred_1h: f64,
    pred_4h: f64,
    pred_24h: f64,
    features_json: &str,
) -> Result<()> {
    let created_at = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO predictions (candle_ts, pred_1h, pred_4h, pred_24h, features_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(candle_ts) DO UPDATE SET
           pred_1h = excluded.pred_1h,
           pred_4h = excluded.pred_4h,
           pred_24h = excluded.pred_24h,
           features_json = excluded.features_json,
           created_at = excluded.created_at",
    )
    .bind(candle_ts)
    .bind(pred_1h)
    .bind(pred_4h)
    .bind(pred_24h)
    .bind(features_json)
    .bind(created_at)
    .execute(pool)
    .await
    .context("insert_prediction")?;
    Ok(())
}

/// Fetch the `limit` most recent candles, ordered oldest-first (ascending ts).
pub async fn fetch_recent_candles(pool: &DbPool, limit: usize) -> Result<Vec<Candle>> {
    let rows = sqlx::query(
        "SELECT ts, open, high, low, close, volume, vwap, funding_rate, basis_z, ob_imbalance
         FROM candles
         ORDER BY ts DESC
         LIMIT ?",
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .context("fetch_recent_candles")?;

    let mut candles: Vec<Candle> = rows
        .iter()
        .map(|row| Candle {
            ts: row.get(0),
            open: row.get(1),
            high: row.get(2),
            low: row.get(3),
            close: row.get(4),
            volume: row.get(5),
            vwap: row.get(6),
            funding_rate: row.get(7),
            basis_z: row.get(8),
            ob_imbalance: row.get(9),
        })
        .collect();

    candles.reverse();
    Ok(candles)
}

/// Return the current position from `signal_state` (id = 1), or `0` if no row exists.
pub async fn load_position(pool: &DbPool, model_id: &str, symbol: &str) -> Result<i64> {
    // §8 per-model: prefer the latest position event for this model/symbol.
    let row = sqlx::query(
        "SELECT position FROM positions WHERE model_id = ?1 AND symbol = ?2 ORDER BY candle_ts DESC LIMIT 1"
    )
    .bind(model_id)
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .context("load_position")?;

    if let Some(r) = row {
        return Ok(r.get(0));
    }

    // Legacy fallback: the singleton signal_state row (used by pre-§8 code/tests).
    match sqlx::query("SELECT position FROM signal_state WHERE id = 1")
        .fetch_one(pool)
        .await
    {
        Ok(row) => Ok(row.get(0)),
        Err(sqlx::Error::RowNotFound) => Ok(0),
        Err(e) => Err(e).context("load_position fallback"),
    }
}

/// Upsert the current position into `signal_state` (singleton row id = 1).
pub async fn save_position(pool: &DbPool, position: i64) -> Result<()> {
    let updated_at = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO signal_state (id, position, updated_at) VALUES (1, ?, ?)
         ON CONFLICT(id) DO UPDATE SET position = excluded.position, updated_at = excluded.updated_at",
    )
    .bind(position)
    .bind(updated_at)
    .execute(pool)
    .await
    .context("save_position")?;
    Ok(())
}

/// Append a position-change event to the `positions` audit table.
pub async fn insert_position_event(
    pool: &DbPool,
    model_id: &str,
    symbol: &str,
    candle_ts: i64,
    position: i64,
    pred_4h: f64,
    pred_24h: f64,
    regime: i64,
    sma: f64,
) -> Result<()> {
    let created_at = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO positions (model_id, symbol, candle_ts, position, pred_4h, pred_24h, regime, sma, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(model_id)
    .bind(symbol)
    .bind(candle_ts)
    .bind(position)
    .bind(pred_4h)
    .bind(pred_24h)
    .bind(regime)
    .bind(sma)
    .bind(created_at)
    .execute(pool)
    .await
    .context("insert_position_event")?;
    Ok(())
}

pub async fn insert_trade(
    pool: &DbPool,
    candle_ts: i64,
    side: &str,
    qty: f64,
    price: f64,
    fee: f64,
    realized_pnl: f64,
) -> Result<()> {
    let created_at = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO trades (candle_ts, side, qty, price, fee, realized_pnl, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(candle_ts)
    .bind(side)
    .bind(qty)
    .bind(price)
    .bind(fee)
    .bind(realized_pnl)
    .bind(created_at)
    .execute(pool)
    .await
    .context("insert_trade")?;
    Ok(())
}

/// Insert an equity trade into `equity_trades` with explicit symbol attribution.
///
/// The equities pipeline (Wave C) reports PnL per-symbol (see
/// `sum_equity_realized_pnl` and `status::handle_status`), so every fill must
/// record which instrument was traded. For a short position this is the inverse
/// ETF symbol (e.g. PSQ), not the primary symbol (QQQ).
pub async fn insert_equity_trade(
    pool: &DbPool,
    model_id: &str,
    symbol: &str,
    candle_ts: i64,
    side: &str,
    qty: f64,
    price: f64,
    fee: f64,
    realized_pnl: f64,
) -> Result<()> {
    let created_at = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO equity_trades (model_id, symbol, candle_ts, side, qty, price, fee, realized_pnl, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(model_id)
    .bind(symbol)
    .bind(candle_ts)
    .bind(side)
    .bind(qty)
    .bind(price)
    .bind(fee)
    .bind(realized_pnl)
    .bind(created_at)
    .execute(pool)
    .await
    .context("insert_equity_trade")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Read helpers for the telemetry API
// ---------------------------------------------------------------------------

/// A prediction row as returned by the API read queries.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PredictionRow {
    pub id: i64,
    pub candle_ts: i64,
    pub pred_1h: f64,
    pub pred_4h: f64,
    pub pred_24h: f64,
    pub features_json: String,
    pub created_at: i64,
    pub actual_1h: Option<f64>,
    pub actual_4h: Option<f64>,
    pub actual_24h: Option<f64>,
}

/// A trade row as returned by the API read queries.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TradeRow {
    pub id: i64,
    pub symbol: String,
    pub model_id: String,
    pub candle_ts: i64,
    pub side: String,
    pub qty: f64,
    pub price: f64,
    pub fee: f64,
    pub realized_pnl: f64,
    pub created_at: i64,
}

/// Fetch the `limit` most recent predictions, ordered newest-first.
pub async fn fetch_recent_predictions(pool: &DbPool, limit: usize) -> Result<Vec<PredictionRow>> {
    let rows = sqlx::query(
        "SELECT id, candle_ts, pred_1h, pred_4h, pred_24h, features_json, created_at,
                actual_1h, actual_4h, actual_24h
         FROM predictions
         ORDER BY candle_ts DESC
         LIMIT ?",
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .context("fetch_recent_predictions")?;

    Ok(rows
        .iter()
        .map(|row| PredictionRow {
            id: row.get(0),
            candle_ts: row.get(1),
            pred_1h: row.get(2),
            pred_4h: row.get(3),
            pred_24h: row.get(4),
            features_json: row.get(5),
            created_at: row.get(6),
            actual_1h: row.get(7),
            actual_4h: row.get(8),
            actual_24h: row.get(9),
        })
        .collect())
}

/// Fetch the `limit` most recent trades, ordered newest-first.
#[allow(dead_code)]
pub async fn fetch_recent_trades(pool: &DbPool, limit: usize) -> Result<Vec<TradeRow>> {
    let rows = sqlx::query(
        "SELECT id, candle_ts, side, qty, price, fee, realized_pnl, created_at
         FROM trades
         ORDER BY id DESC
         LIMIT ?",
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .context("fetch_recent_trades")?;

    Ok(rows
        .iter()
        .map(|row| TradeRow {
            id: row.get(0),
            symbol: String::new(),
            model_id: String::new(),
            candle_ts: row.get(1),
            side: row.get(2),
            qty: row.get(3),
            price: row.get(4),
            fee: row.get(5),
            realized_pnl: row.get(6),
            created_at: row.get(7),
        })
        .collect())
}

/// Sum all realized PnL across the trades table.
pub async fn sum_realized_pnl(pool: &DbPool) -> Result<f64> {
    let row = sqlx::query("SELECT COALESCE(SUM(realized_pnl), 0.0) FROM trades")
        .fetch_one(pool)
        .await
        .context("sum_realized_pnl")?;
    Ok(row.get(0))
}

/// Fetch the `limit` most recent equity trades for a given symbol, newest-first.
/// The equities executor persists fills here (with symbol attribution, including
/// the inverse-ETF short symbol), so consumers must read by symbol.
pub async fn fetch_recent_equity_trades(
    pool: &DbPool,
    symbol: &str,
    limit: usize,
) -> Result<Vec<TradeRow>> {
    let rows = sqlx::query(
        "SELECT id, symbol, model_id, candle_ts, side, qty, price, fee, realized_pnl, created_at
         FROM equity_trades
         WHERE symbol = ?1
         ORDER BY id DESC
         LIMIT ?2",
    )
    .bind(symbol)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .context("fetch_recent_equity_trades")?;

    Ok(rows
        .iter()
        .map(|row| TradeRow {
            id: row.get(0),
            symbol: row.get(1),
            model_id: row.get(2),
            candle_ts: row.get(3),
            side: row.get(4),
            qty: row.get(5),
            price: row.get(6),
            fee: row.get(7),
            realized_pnl: row.get(8),
            created_at: row.get(9),
        })
        .collect())
}

/// Return recent equity trades across ALL symbols (QQQ + PSQ).
/// Used when the frontend requests trades without a symbol filter.
pub async fn fetch_recent_all_equity_trades(
    pool: &DbPool,
    limit: usize,
) -> Result<Vec<TradeRow>> {
    let rows = sqlx::query(
        "SELECT id, symbol, model_id, candle_ts, side, qty, price, fee, realized_pnl, created_at
         FROM equity_trades
         ORDER BY id DESC
         LIMIT ?1",
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .context("fetch_recent_all_equity_trades")?;

    Ok(rows
        .iter()
        .map(|row| TradeRow {
            id: row.get(0),
            symbol: row.get(1),
            model_id: row.get(2),
            candle_ts: row.get(3),
            side: row.get(4),
            qty: row.get(5),
            price: row.get(6),
            fee: row.get(7),
            realized_pnl: row.get(8),
            created_at: row.get(9),
        })
        .collect())
}

/// Return the most recent candle, or `None` if the table is empty.
pub async fn fetch_latest_candle(pool: &DbPool) -> Result<Option<Candle>> {
    let row = sqlx::query(
        "SELECT ts, open, high, low, close, volume, vwap, funding_rate, basis_z, ob_imbalance
         FROM candles
         ORDER BY ts DESC
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("fetch_latest_candle")?;

    Ok(row.map(|r| Candle {
        ts: r.get(0),
        open: r.get(1),
        high: r.get(2),
        low: r.get(3),
        close: r.get(4),
        volume: r.get(5),
        vwap: r.get(6),
        funding_rate: r.get(7),
        basis_z: r.get(8),
        ob_imbalance: r.get(9),
    }))
}

/// Fetch the price of the most recent entry (buy) trade for a given symbol.
/// Used to compute entry_price for open positions (Long=QQQ, Short=PSQ).
/// Filters on side='buy' so partial closes don't shadow the entry.
pub async fn fetch_equity_entry_trade_price(
    pool: &DbPool,
    symbol: &str,
) -> Result<Option<f64>> {
    let row = sqlx::query(
        "SELECT price FROM equity_trades WHERE symbol = ?1 AND side = 'buy' ORDER BY id DESC LIMIT 1",
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .context("fetch_equity_entry_trade_price")?;
    Ok(row.map(|r| r.get(0)))
}

/// Fetch the candle timestamp of the most recent equity trade for a given symbol.
pub async fn fetch_equity_entry_trade_ts(
    pool: &DbPool,
    symbol: &str,
) -> Result<Option<i64>> {
    let row = sqlx::query(
        "SELECT candle_ts FROM equity_trades WHERE symbol = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .context("fetch_equity_entry_trade_ts")?;
    Ok(row.map(|r| r.get(0)))
}

/// Fetch the close price at or before a given timestamp for a symbol.
/// Uses floor match (ts <= ?2) so a slight timestamp mismatch doesn't return None.
pub async fn fetch_equity_close_at_ts(
    pool: &DbPool,
    symbol: &str,
    candle_ts: i64,
) -> Result<Option<f64>> {
    let row = sqlx::query(
        "SELECT close FROM equity_candles WHERE symbol = ?1 AND ts <= ?2 ORDER BY ts DESC LIMIT 1",
    )
    .bind(symbol)
    .bind(candle_ts)
    .fetch_optional(pool)
    .await
    .context("fetch_equity_close_at_ts")?;
    Ok(row.map(|r| r.get(0)))
}

/// Accuracy metrics computed over resolved predictions.
#[derive(Debug, Clone)]
pub struct AccuracyStats {
    pub directional_1h: f64,
    pub directional_4h: f64,
    pub directional_24h: f64,
    pub mae_1h: f64,
    pub mae_4h: f64,
    pub mae_24h: f64,
    pub resolved_count: usize,
}

/// Fill in actual_1h/4h/24h for predictions where the future candle now exists.
/// Returns the number of rows updated.
pub async fn compute_actuals(pool: &DbPool) -> Result<u64> {
    let mut updated: u64 = 0;

    // Find predictions with at least one NULL actual
    let rows = sqlx::query(
        "SELECT p.candle_ts, p.id, p.actual_1h, p.actual_4h, p.actual_24h
         FROM predictions p
         WHERE p.actual_1h IS NULL OR p.actual_4h IS NULL OR p.actual_24h IS NULL"
    )
    .fetch_all(pool)
    .await
    .context("compute_actuals: fetch null predictions")?;

    for row in &rows {
        let candle_ts: i64 = row.get(0);
        let pred_id: i64 = row.get(1);
        let cur_1h: Option<f64> = row.get(2);
        let cur_4h: Option<f64> = row.get(3);
        let cur_24h: Option<f64> = row.get(4);

        // Get the base candle's close price
        let base_close: f64 = match sqlx::query("SELECT close FROM candles WHERE ts = ?")
            .bind(candle_ts)
            .fetch_optional(pool)
            .await?
        {
            Some(r) => r.get(0),
            None => continue,
        };

        let offsets: &[(i64, Option<f64>, &str)] = &[
            (3600, cur_1h, "actual_1h"),
            (14400, cur_4h, "actual_4h"),
            (86400, cur_24h, "actual_24h"),
        ];

        for &(offset, current, col) in offsets {
            if current.is_some() {
                continue; // already computed
            }
            let future_ts = candle_ts + offset;
            let future_close: Option<f64> = sqlx::query("SELECT close FROM candles WHERE ts = ?")
                .bind(future_ts)
                .fetch_optional(pool)
                .await?
                .map(|r| r.get(0));

            if let Some(fc) = future_close {
                if base_close > 0.0 {
                    let actual = (fc / base_close).ln();
                    let sql = format!("UPDATE predictions SET {col} = ? WHERE id = ?");
                    sqlx::query(&sql)
                        .bind(actual)
                        .bind(pred_id)
                        .execute(pool)
                        .await
                        .with_context(|| format!("compute_actuals: update {col}"))?;
                    updated += 1;
                }
            }
        }
    }

    Ok(updated)
}

/// Compute directional accuracy and MAE over resolved predictions.
pub async fn fetch_accuracy(pool: &DbPool) -> Result<AccuracyStats> {
    let rows = sqlx::query(
        "SELECT pred_1h, pred_4h, pred_24h, actual_1h, actual_4h, actual_24h
         FROM predictions
         WHERE actual_1h IS NOT NULL OR actual_4h IS NOT NULL OR actual_24h IS NOT NULL"
    )
    .fetch_all(pool)
    .await
    .context("fetch_accuracy")?;

    let mut count_1h: usize = 0;
    let mut count_4h: usize = 0;
    let mut count_24h: usize = 0;
    let mut dir_1h: usize = 0;
    let mut dir_4h: usize = 0;
    let mut dir_24h: usize = 0;
    let mut sum_ae_1h: f64 = 0.0;
    let mut sum_ae_4h: f64 = 0.0;
    let mut sum_ae_24h: f64 = 0.0;

    for row in &rows {
        let pred_1h: f64 = row.get(0);
        let pred_4h: f64 = row.get(1);
        let pred_24h: f64 = row.get(2);
        let actual_1h: Option<f64> = row.get(3);
        let actual_4h: Option<f64> = row.get(4);
        let actual_24h: Option<f64> = row.get(5);

        if let Some(a) = actual_1h {
            count_1h += 1;
            if (pred_1h >= 0.0) == (a >= 0.0) { dir_1h += 1; }
            sum_ae_1h += (pred_1h - a).abs();
        }
        if let Some(a) = actual_4h {
            count_4h += 1;
            if (pred_4h >= 0.0) == (a >= 0.0) { dir_4h += 1; }
            sum_ae_4h += (pred_4h - a).abs();
        }
        if let Some(a) = actual_24h {
            count_24h += 1;
            if (pred_24h >= 0.0) == (a >= 0.0) { dir_24h += 1; }
            sum_ae_24h += (pred_24h - a).abs();
        }
    }

    let resolved_count = rows.len();

    Ok(AccuracyStats {
        directional_1h: if count_1h > 0 { (dir_1h as f64 / count_1h as f64) * 100.0 } else { 0.0 },
        directional_4h: if count_4h > 0 { (dir_4h as f64 / count_4h as f64) * 100.0 } else { 0.0 },
        directional_24h: if count_24h > 0 { (dir_24h as f64 / count_24h as f64) * 100.0 } else { 0.0 },
        mae_1h: if count_1h > 0 { sum_ae_1h / count_1h as f64 } else { 0.0 },
        mae_4h: if count_4h > 0 { sum_ae_4h / count_4h as f64 } else { 0.0 },
        mae_24h: if count_24h > 0 { sum_ae_24h / count_24h as f64 } else { 0.0 },
        resolved_count,
    })
}

// =========================================================================
// Wave A: equities data layer — public API
// =========================================================================

/// Insert or update an equity candle row.
pub async fn upsert_equity_candle(pool: &DbPool, c: &EquityCandle) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO equity_candles (symbol, ts, open, high, low, close, volume, source)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
           ON CONFLICT(symbol, ts) DO UPDATE SET
               open=excluded.open, high=excluded.high, low=excluded.low,
               close=excluded.close, volume=excluded.volume, source=excluded.source"#,
    )
    .bind(&c.symbol)
    .bind(c.ts)
    .bind(c.open)
    .bind(c.high)
    .bind(c.low)
    .bind(c.close)
    .bind(c.volume)
    .bind(&c.source)
    .execute(pool)
    .await
    .context("upsert_equity_candle")?;
    Ok(())
}

/// Count equity candles for a symbol.
pub async fn count_equity_candles(pool: &DbPool, symbol: &str) -> Result<i64> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM equity_candles WHERE symbol = ?1")
        .bind(symbol)
        .fetch_one(pool)
        .await
        .context("count_equity_candles")?;
    Ok(row.get::<i64, _>("n"))
}

/// Fetch recent equity candles for a symbol, newest first.
pub async fn fetch_equity_candles(
    pool: &DbPool,
    symbol: &str,
    limit: i64,
) -> Result<Vec<EquityCandle>> {
    let rows = sqlx::query(
        r#"SELECT symbol, ts, open, high, low, close, volume, source
           FROM equity_candles
           WHERE symbol = ?1
           ORDER BY ts DESC
           LIMIT ?2"#,
    )
    .bind(symbol)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("fetch_equity_candles")?;
    Ok(rows
        .iter()
        .map(|r| EquityCandle {
            symbol: r.get::<String, _>("symbol"),
            ts: r.get::<i64, _>("ts"),
            open: r.get::<f64, _>("open"),
            high: r.get::<f64, _>("high"),
            low: r.get::<f64, _>("low"),
            close: r.get::<f64, _>("close"),
            volume: r.get::<i64, _>("volume"),
            source: r.get::<String, _>("source"),
        })
        .collect())
}

/// Fetch equity candles for a symbol, **oldest first** (ascending ts).
/// Used by the feature pipeline which needs chronological order.
pub async fn fetch_equity_candles_asc(
    pool: &DbPool,
    symbol: &str,
    limit: i64,
) -> Result<Vec<EquityCandle>> {
    // Subquery: grab the latest N rows (DESC), then re-sort ascending so
    // callers get chronological order.  Without the subquery, ASC LIMIT N
    // would return the OLDEST N rows (2021 data) instead of the latest.
    let rows = sqlx::query(
        r#"SELECT symbol, ts, open, high, low, close, volume, source
           FROM (
             SELECT symbol, ts, open, high, low, close, volume, source
             FROM equity_candles
             WHERE symbol = ?1
             ORDER BY ts DESC
             LIMIT ?2
           )
           ORDER BY ts ASC"#,
    )
    .bind(symbol)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("fetch_equity_candles_asc")?;
    Ok(rows
        .iter()
        .map(|r| EquityCandle {
            symbol: r.get::<String, _>("symbol"),
            ts: r.get::<i64, _>("ts"),
            open: r.get::<f64, _>("open"),
            high: r.get::<f64, _>("high"),
            low: r.get::<f64, _>("low"),
            close: r.get::<f64, _>("close"),
            volume: r.get::<i64, _>("volume"),
            source: r.get::<String, _>("source"),
        })
        .collect())
}

/// Fetch equity candles for a symbol, **newest first** (descending ts).
/// Used for computing trailing SMAs where we want the most recent values.
pub async fn fetch_equity_candles_desc(
    pool: &DbPool,
    symbol: &str,
    limit: i64,
) -> Result<Vec<EquityCandle>> {
    let rows = sqlx::query(
        r#"SELECT symbol, ts, open, high, low, close, volume, source
           FROM equity_candles
           WHERE symbol = ?1
           ORDER BY ts DESC
           LIMIT ?2"#,
    )
    .bind(symbol)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("fetch_equity_candles_desc")?;
    Ok(rows
        .iter()
        .map(|r| EquityCandle {
            symbol: r.get::<String, _>("symbol"),
            ts: r.get::<i64, _>("ts"),
            open: r.get::<f64, _>("open"),
            high: r.get::<f64, _>("high"),
            low: r.get::<f64, _>("low"),
            close: r.get::<f64, _>("close"),
            volume: r.get::<i64, _>("volume"),
            source: r.get::<String, _>("source"),
        })
        .collect())
}

/// Insert (or replace) a prediction row for the QQQ daily equities model.
pub async fn insert_equity_prediction(
    pool: &DbPool,
    symbol: &str,
    candle_ts: i64,
    pred_1d: f64,
    pred_5d: f64,
    pred_21d: f64,
    regime: &str,
    features_json: &str,
) -> Result<()> {
    let now = Utc::now().timestamp();
    sqlx::query(
        r#"INSERT INTO equity_predictions
               (symbol, candle_ts, pred_1d, pred_5d, pred_21d, regime, features_json, created_at, source)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'qqq_tcn_v1')
           ON CONFLICT(symbol, candle_ts) DO UPDATE SET
               pred_1d=excluded.pred_1d,
               pred_5d=excluded.pred_5d,
               pred_21d=excluded.pred_21d,
               regime=excluded.regime,
               features_json=excluded.features_json"#,
    )
    .bind(symbol)
    .bind(candle_ts)
    .bind(pred_1d)
    .bind(pred_5d)
    .bind(pred_21d)
    .bind(regime)
    .bind(features_json)
    .bind(now)
    .execute(pool)
    .await
    .context("insert_equity_prediction")?;
    Ok(())
}

/// Fetch the latest equity candle timestamp for a symbol (newest first).
/// Returns `None` if no candles exist.
pub async fn latest_equity_candle_ts(pool: &DbPool, symbol: &str) -> Result<Option<i64>> {
    let row = sqlx::query(
        r#"SELECT ts FROM equity_candles WHERE symbol = ?1 ORDER BY ts DESC LIMIT 1"#,
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .context("latest_equity_candle_ts")?;
    Ok(row.map(|r| r.get::<i64, _>("ts")))
}

/// Fetch the most recent equity candle for a given symbol.
pub async fn fetch_latest_equity_candle(
    pool: &DbPool,
    symbol: &str,
) -> Result<Option<EquityCandle>> {
    let row = sqlx::query_as::<_, EquityCandle>(
        r#"SELECT symbol, ts, open, high, low, close, volume, source
           FROM equity_candles WHERE symbol = ?1 ORDER BY ts DESC LIMIT 1"#,
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .context("fetch_latest_equity_candle")?;
    Ok(row.map(|r| r.into()))
}

/// Sum realized PnL for equity trades for a given symbol.
pub async fn sum_equity_realized_pnl(pool: &DbPool, symbol: &str) -> Result<f64> {
    let row = sqlx::query(
        r#"SELECT COALESCE(SUM(realized_pnl), 0.0) as pnl
           FROM equity_trades WHERE symbol = ?1"#,
    )
    .bind(symbol)
    .fetch_one(pool)
    .await
    .context("sum_equity_realized_pnl")?;
    Ok(row.get::<f64, _>("pnl"))
}

/// Fetch the most recent equity prediction for a given symbol.
pub async fn fetch_latest_equity_prediction(
    pool: &DbPool,
    symbol: &str,
) -> Result<Option<EquityPredictionRow>> {
    let row = sqlx::query_as::<_, EquityPredictionRow>(
        r#"SELECT id, symbol, candle_ts, pred_1d, pred_5d, pred_21d,
                  regime, features_json, created_at, source
           FROM equity_predictions
           WHERE symbol = ?1
           ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(symbol)
    .fetch_optional(pool)
    .await
    .context("fetch_latest_equity_prediction")?;
    Ok(row)
}

/// Fetch recent equity predictions for a symbol, newest-first.
pub async fn fetch_recent_equity_predictions(
    pool: &DbPool,
    symbol: &str,
    limit: usize,
) -> Result<Vec<EquityPredictionRow>> {
    let rows = sqlx::query_as::<_, EquityPredictionRow>(
        r#"SELECT id, symbol, candle_ts, pred_1d, pred_5d, pred_21d,
                  regime, features_json, created_at, source
           FROM equity_predictions
           WHERE symbol = ?1
           ORDER BY created_at DESC
           LIMIT ?2"#,
    )
    .bind(symbol)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .context("fetch_recent_equity_predictions")?;
    Ok(rows)
}

/// Compute directional accuracy, MAE, and IC over resolved equity predictions.
///
/// For each prediction at `candle_ts`, the actual return at horizon N is
/// `ln(close[ts+N] / close[ts])`, looked up from `equity_candles`.
/// The equity_predictions table has no actuals columns (unlike the crypto
/// `predictions` table), so we compute on-the-fly.
pub async fn fetch_equity_accuracy(pool: &DbPool, symbol: &str) -> Result<AccuracyStats> {
    // Fetch predictions and candles
    let preds = sqlx::query_as::<_, EquityPredictionRow>(
        r#"SELECT id, symbol, candle_ts, pred_1d, pred_5d, pred_21d,
                  regime, features_json, created_at, source
           FROM equity_predictions
           WHERE symbol = ?1
           ORDER BY candle_ts DESC
           LIMIT 500"#,
    )
    .bind(symbol)
    .fetch_all(pool)
    .await
    .context("fetch_equity_accuracy: predictions")?;

    let candles = sqlx::query("SELECT ts, close FROM equity_candles WHERE symbol = ?1 ORDER BY ts ASC")
        .bind(symbol)
        .fetch_all(pool)
        .await
        .context("fetch_equity_accuracy: candles")?;

    if preds.is_empty() || candles.len() < 2 {
        return Ok(AccuracyStats {
            directional_1h: 0.0,
            directional_4h: 0.0,
            directional_24h: 0.0,
            mae_1h: 0.0,
            mae_4h: 0.0,
            mae_24h: 0.0,
            resolved_count: 0,
        });
    }

    // Build candle close lookup: ts -> close
    let candle_map: std::collections::HashMap<i64, f64> = candles
        .iter()
        .map(|r| (r.get::<i64, _>(0), r.get::<f64, _>(1)))
        .collect();
    let candle_tss: Vec<i64> = {
        let mut v = candle_map.keys().copied().collect::<Vec<_>>();
        v.sort_unstable();
        v
    };

    let day = 86_400_i64;
    let horizons: &[(i64, &str)] = &[(day, "1d"), (5 * day, "5d"), (21 * day, "21d")];

    // Pred field selector per horizon
    let mut count_1d = 0usize;
    let mut count_5d = 0usize;
    let mut count_21d = 0usize;
    let mut dir_1d = 0usize;
    let mut dir_5d = 0usize;
    let mut dir_21d = 0usize;
    let mut sum_ae_1d = 0.0_f64;
    let mut sum_ae_5d = 0.0_f64;
    let mut sum_ae_21d = 0.0_f64;

    for p in &preds {
        let base_close = match candle_map.get(&p.candle_ts) {
            Some(&c) if c > 0.0 => c,
            _ => continue,
        };

        for &(offset, label) in horizons {
            // Find the candle closest to candle_ts + offset (must be >= target,
            // within 3 calendar days tolerance to skip weekends/holidays)
            let target = p.candle_ts + offset;
            let future_close = find_closest_close(&candle_tss, &candle_map, target, 3 * day);
            if let Some(fc) = future_close {
                if fc <= 0.0 {
                    continue;
                }
                let actual = (fc / base_close).ln();
                let pred_val = match label {
                    "1d" => p.pred_1d,
                    "5d" => p.pred_5d,
                    "21d" => p.pred_21d,
                    _ => continue,
                };

                let correct = (pred_val >= 0.0) == (actual >= 0.0);
                let ae = (pred_val - actual).abs();

                match label {
                    "1d" => {
                        count_1d += 1;
                        if correct { dir_1d += 1; }
                        sum_ae_1d += ae;
                    }
                    "5d" => {
                        count_5d += 1;
                        if correct { dir_5d += 1; }
                        sum_ae_5d += ae;
                    }
                    "21d" => {
                        count_21d += 1;
                        if correct { dir_21d += 1; }
                        sum_ae_21d += ae;
                    }
                    _ => {}
                }
            }
        }
    }

    let resolved_count = count_1d.max(count_5d).max(count_21d);

    Ok(AccuracyStats {
        directional_1h: if count_1d > 0 { (dir_1d as f64 / count_1d as f64) * 100.0 } else { 0.0 },
        directional_4h: if count_5d > 0 { (dir_5d as f64 / count_5d as f64) * 100.0 } else { 0.0 },
        directional_24h: if count_21d > 0 { (dir_21d as f64 / count_21d as f64) * 100.0 } else { 0.0 },
        mae_1h: if count_1d > 0 { sum_ae_1d / count_1d as f64 } else { 0.0 },
        mae_4h: if count_5d > 0 { sum_ae_5d / count_5d as f64 } else { 0.0 },
        mae_24h: if count_21d > 0 { sum_ae_21d / count_21d as f64 } else { 0.0 },
        resolved_count,
    })
}

/// Find the close price for the candle nearest to `target_ts`,
/// within `tolerance` seconds. Returns None if no candle is close enough.
fn find_closest_close(
    sorted_tss: &[i64],
    candle_map: &std::collections::HashMap<i64, f64>,
    target_ts: i64,
    tolerance: i64,
) -> Option<f64> {
    // Binary search for the insertion point
    let idx = sorted_tss.binary_search(&target_ts).unwrap_or_else(|i| i);
    // Check the closest candidates around the insertion point
    let mut best: Option<(i64, f64)> = None;
    for &check_idx in &[idx, idx.saturating_sub(1)] {
        if check_idx < sorted_tss.len() {
            let ts = sorted_tss[check_idx];
            let diff = (ts - target_ts).abs();
            if diff <= tolerance {
                let close = candle_map.get(&ts).copied();
                if let Some(c) = close {
                    if best.is_none() || diff < best.unwrap().0 {
                        best = Some((diff, c));
                    }
                }
            }
        }
    }
    best.map(|(_, c)| c)
}

/// Fetch the most recent `limit` equity candles for a symbol,
/// **newest-first** (descending ts). Callers that need ascending order
/// (e.g. chart rendering) should `.reverse()` the result.
pub async fn fetch_recent_equity_candles(
    pool: &DbPool,
    symbol: &str,
    limit: i64,
) -> Result<Vec<EquityCandle>> {
    let rows = sqlx::query_as::<_, EquityCandle>(
        r#"SELECT symbol, ts, open, high, low, close, volume, source
           FROM equity_candles
           WHERE symbol = ?1
           ORDER BY ts DESC
           LIMIT ?2"#,
    )
    .bind(symbol)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("fetch_recent_equity_candles")?;
    Ok(rows)
}

/// Update the ingest watermark for a (source, symbol) pair.
pub async fn update_ingest_state(
    pool: &DbPool,
    source: &str,
    symbol: &str,
    last_ts: i64,
    rows_loaded: i64,
    error_msg: Option<&str>,
) -> Result<()> {
    let now = Utc::now().timestamp();
    let err_increment = if error_msg.is_some() { 1 } else { 0 };
    sqlx::query(
        r#"INSERT INTO equity_ingest_state
               (source, symbol, last_ts, last_run_at, rows_loaded, error_count, last_error)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
           ON CONFLICT(source, symbol) DO UPDATE SET
               last_ts=excluded.last_ts,
               last_run_at=excluded.last_run_at,
               rows_loaded=excluded.rows_loaded,
               error_count=equity_ingest_state.error_count + ?6,
               last_error=COALESCE(NULLIF(?7, ''), equity_ingest_state.last_error)"#,
    )
    .bind(source)
    .bind(symbol)
    .bind(last_ts)
    .bind(now)
    .bind(rows_loaded)
    .bind(err_increment)
    .bind(error_msg.unwrap_or(""))
    .execute(pool)
    .await
    .context("update_ingest_state")?;
    Ok(())
}

/// Fetch equity predictions in a timestamp range, ascending by candle_ts.
/// Used by the backtest replay engine.
pub async fn fetch_equity_predictions_range(
    pool: &DbPool,
    symbol: &str,
    start_ts: i64,
    end_ts: i64,
) -> Result<Vec<EquityPredictionRow>> {
    let rows = sqlx::query_as::<_, EquityPredictionRow>(
        r#"SELECT id, symbol, candle_ts, pred_1d, pred_5d, pred_21d,
                  regime, features_json, created_at, source
           FROM equity_predictions
           WHERE symbol = ?1 AND candle_ts >= ?2 AND candle_ts <= ?3
           ORDER BY candle_ts ASC"#,
    )
    .bind(symbol)
    .bind(start_ts)
    .bind(end_ts)
    .fetch_all(pool)
    .await
    .context("fetch_equity_predictions_range")?;
    Ok(rows)
}

/// Fetch equity candles in a timestamp range, ascending by ts.
/// Used by the backtest replay engine.
pub async fn fetch_equity_candles_range_asc(
    pool: &DbPool,
    symbol: &str,
    start_ts: i64,
    end_ts: i64,
) -> Result<Vec<EquityCandle>> {
    let rows = sqlx::query_as::<_, EquityCandle>(
        r#"SELECT symbol, ts, open, high, low, close, volume, source
           FROM equity_candles
           WHERE symbol = ?1 AND ts >= ?2 AND ts <= ?3
           ORDER BY ts ASC"#,
    )
    .bind(symbol)
    .bind(start_ts)
    .bind(end_ts)
    .fetch_all(pool)
    .await
    .context("fetch_equity_candles_range_asc")?;
    Ok(rows)
}

/// A row from the `strategy_configs` table.
#[derive(Debug, Clone, FromRow, Serialize)]
pub struct StrategyConfigRow {
    pub id: String,
    pub name: String,
    pub strategy_type: String,
    pub script_body: Option<String>,
    pub params_json: String,
    pub is_active: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Insert a new strategy configuration.
pub async fn insert_strategy_config(
    pool: &DbPool,
    id: &str,
    name: &str,
    strategy_type: &str,
    script_body: Option<&str>,
    params_json: &str,
) -> Result<()> {
    let now = Utc::now().timestamp();
    sqlx::query(
        r#"INSERT INTO strategy_configs
               (id, name, strategy_type, script_body, params_json, is_active, created_at, updated_at)
           VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6)"#,
    )
    .bind(id)
    .bind(name)
    .bind(strategy_type)
    .bind(script_body)
    .bind(params_json)
    .bind(now)
    .execute(pool)
    .await
    .context("insert_strategy_config")?;
    Ok(())
}

/// Fetch all strategy configurations, newest first.
pub async fn fetch_strategy_configs(pool: &DbPool) -> Result<Vec<StrategyConfigRow>> {
    let rows = sqlx::query_as::<_, StrategyConfigRow>(
        r#"SELECT id, name, strategy_type, script_body, params_json,
                  is_active, created_at, updated_at
           FROM strategy_configs
           ORDER BY created_at DESC"#,
    )
    .fetch_all(pool)
    .await
    .context("fetch_strategy_configs")?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> DbPool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for stmt in DDL.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query(stmt).execute(&pool).await.unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn migrate_predictions_adds_columns() {
        let pool = test_pool().await;
        migrate_predictions(&pool).await.unwrap();

        let rows = sqlx::query("PRAGMA table_info(predictions)")
            .fetch_all(&pool)
            .await
            .unwrap();
        let names: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();

        assert!(names.contains(&"actual_1h".to_string()));
        assert!(names.contains(&"actual_4h".to_string()));
        assert!(names.contains(&"actual_24h".to_string()));
    }

    #[tokio::test]
    async fn migrate_predictions_is_idempotent() {
        let pool = test_pool().await;
        migrate_predictions(&pool).await.unwrap();
        migrate_predictions(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn insert_prediction_preserves_actuals_on_conflict() {
        let pool = test_pool().await;
        migrate_predictions(&pool).await.unwrap();

        insert_prediction(&pool, 1_000_000, 0.1, 0.2, 0.3, "[]")
            .await
            .unwrap();

        sqlx::query("UPDATE predictions SET actual_1h = ? WHERE candle_ts = ?")
            .bind(42.0_f64)
            .bind(1_000_000_i64)
            .execute(&pool)
            .await
            .unwrap();

        insert_prediction(&pool, 1_000_000, 0.4, 0.5, 0.6, "[1]")
            .await
            .unwrap();

        let rows = fetch_recent_predictions(&pool, 1).await.unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert!((row.pred_1h - 0.4).abs() < 1e-9);
        assert!((row.pred_4h - 0.5).abs() < 1e-9);
        assert!((row.pred_24h - 0.6).abs() < 1e-9);
        assert_eq!(row.features_json, "[1]");
        assert!((row.actual_1h.unwrap() - 42.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn compute_actuals_fills_null_columns() {
        let pool = test_pool().await;
        migrate_predictions(&pool).await.unwrap();

        // Seed a base candle and future candles
        upsert_candle(&pool, &Candle { ts: 1000, open: 100.0, high: 100.0, low: 100.0, close: 100.0, volume: 1.0, vwap: 100.0, funding_rate: 0.0, basis_z: 0.0, ob_imbalance: 0.0 }).await.unwrap();
        upsert_candle(&pool, &Candle { ts: 4600, open: 110.0, high: 110.0, low: 110.0, close: 110.0, volume: 1.0, vwap: 110.0, funding_rate: 0.0, basis_z: 0.0, ob_imbalance: 0.0 }).await.unwrap();
        upsert_candle(&pool, &Candle { ts: 15400, open: 105.0, high: 105.0, low: 105.0, close: 105.0, volume: 1.0, vwap: 105.0, funding_rate: 0.0, basis_z: 0.0, ob_imbalance: 0.0 }).await.unwrap();
        upsert_candle(&pool, &Candle { ts: 87400, open: 95.0, high: 95.0, low: 95.0, close: 95.0, volume: 1.0, vwap: 95.0, funding_rate: 0.0, basis_z: 0.0, ob_imbalance: 0.0 }).await.unwrap();

        // Insert a prediction
        insert_prediction(&pool, 1000, 0.01, 0.02, 0.03, "[]").await.unwrap();

        let updated = compute_actuals(&pool).await.unwrap();
        assert!(updated > 0, "should have updated at least one actual");

        let preds = fetch_recent_predictions(&pool, 1).await.unwrap();
        assert!(preds[0].actual_1h.is_some(), "actual_1h should be filled");
        assert!(preds[0].actual_4h.is_some(), "actual_4h should be filled");
        assert!(preds[0].actual_24h.is_some(), "actual_24h should be filled");

        // Verify 1h actual = ln(110/100)
        let expected_1h = (110.0_f64 / 100.0).ln();
        assert!((preds[0].actual_1h.unwrap() - expected_1h).abs() < 1e-9);
    }

    #[tokio::test]
    async fn compute_actuals_skips_unresolved_horizons() {
        let pool = test_pool().await;
        migrate_predictions(&pool).await.unwrap();

        upsert_candle(&pool, &Candle { ts: 1000, open: 100.0, high: 100.0, low: 100.0, close: 100.0, volume: 1.0, vwap: 100.0, funding_rate: 0.0, basis_z: 0.0, ob_imbalance: 0.0 }).await.unwrap();
        // Only add 1h candle, no 4h or 24h
        upsert_candle(&pool, &Candle { ts: 4600, open: 110.0, high: 110.0, low: 110.0, close: 110.0, volume: 1.0, vwap: 110.0, funding_rate: 0.0, basis_z: 0.0, ob_imbalance: 0.0 }).await.unwrap();

        insert_prediction(&pool, 1000, 0.01, 0.02, 0.03, "[]").await.unwrap();

        compute_actuals(&pool).await.unwrap();

        let preds = fetch_recent_predictions(&pool, 1).await.unwrap();
        assert!(preds[0].actual_1h.is_some(), "actual_1h should be filled");
        assert!(preds[0].actual_4h.is_none(), "actual_4h should remain NULL");
        assert!(preds[0].actual_24h.is_none(), "actual_24h should remain NULL");
    }

    #[tokio::test]
    async fn fetch_accuracy_computes_directional_and_mae() {
        let pool = test_pool().await;
        migrate_predictions(&pool).await.unwrap();

        // Insert prediction with known actuals
        insert_prediction(&pool, 1000, 0.01, 0.02, -0.03, "[]").await.unwrap();
        sqlx::query("UPDATE predictions SET actual_1h = 0.015, actual_4h = -0.01, actual_24h = -0.025 WHERE candle_ts = 1000")
            .execute(&pool).await.unwrap();

        let stats = fetch_accuracy(&pool).await.unwrap();
        assert_eq!(stats.resolved_count, 1);
        // 1h: pred=0.01 (+), actual=0.015 (+) → direction match
        assert!((stats.directional_1h - 100.0).abs() < 1e-9);
        // 4h: pred=0.02 (+), actual=-0.01 (-) → mismatch
        assert!((stats.directional_4h - 0.0).abs() < 1e-9);
        // 24h: pred=-0.03 (-), actual=-0.025 (-) → match
        assert!((stats.directional_24h - 100.0).abs() < 1e-9);
        // MAE: |0.01 - 0.015| = 0.005
        assert!((stats.mae_1h - 0.005).abs() < 1e-9);
    }

    // ===== Wave A: equity data layer tests ============================
    // (Helper functions live at module level above.)

    #[tokio::test]
    async fn equity_candles_upsert_and_count() {
        let pool = test_pool().await;
        let c = EquityCandle {
            symbol: "QQQ".into(),
            ts: 1_700_000_000,
            open: 400.0,
            high: 405.0,
            low: 399.0,
            close: 404.5,
            volume: 1_000_000,
            source: "yahoo".into(),
        };
        upsert_equity_candle(&pool, &c).await.unwrap();
        assert_eq!(count_equity_candles(&pool, "QQQ").await.unwrap(), 1);

        // Upsert same key with new close — count should stay 1.
        let c2 = EquityCandle { close: 410.0, ..c.clone() };
        upsert_equity_candle(&pool, &c2).await.unwrap();
        assert_eq!(count_equity_candles(&pool, "QQQ").await.unwrap(), 1);

        // Different symbol
        let c3 = EquityCandle {
            symbol: "TLT".into(),
            ts: 1_700_000_000,
            open: 100.0, high: 101.0, low: 99.0, close: 100.5,
            volume: 500_000, source: "yahoo".into(),
        };
        upsert_equity_candle(&pool, &c3).await.unwrap();
        assert_eq!(count_equity_candles(&pool, "TLT").await.unwrap(), 1);
        assert_eq!(count_equity_candles(&pool, "QQQ").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn fetch_equity_candles_returns_recent_first() {
        let pool = test_pool().await;
        for i in 0..5 {
            let c = EquityCandle {
                symbol: "QQQ".into(),
                ts: 1_700_000_000 + i * 86_400,
                open: 400.0, high: 405.0, low: 399.0, close: 404.0,
                volume: 1_000_000, source: "yahoo".into(),
            };
            upsert_equity_candle(&pool, &c).await.unwrap();
        }
        let rows = fetch_equity_candles(&pool, "QQQ", 3).await.unwrap();
        assert_eq!(rows.len(), 3);
        // Newest first.
        assert!(rows[0].ts > rows[1].ts);
        assert!(rows[1].ts > rows[2].ts);
    }

    #[tokio::test]
    async fn ingest_state_tracks_errors() {
        let pool = test_pool().await;
        update_ingest_state(&pool, "yahoo", "QQQ", 1_700_000_000, 250, None)
            .await.unwrap();
        // Calling again with an error should not lose the prior last_ts.
        update_ingest_state(&pool, "yahoo", "QQQ", 1_700_086_400, 251, Some("timeout"))
            .await.unwrap();
        let row = sqlx::query(
            "SELECT last_ts, rows_loaded, error_count, last_error
             FROM equity_ingest_state WHERE source='yahoo' AND symbol='QQQ'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let last_ts: i64 = row.get("last_ts");
        let err: String = row.get("last_error");
        assert_eq!(last_ts, 1_700_086_400);
        assert_eq!(err, "timeout");
    }

    // --- trading_models registry tests (§8 plan amendment 2026-08-05) ---

    #[tokio::test]
    async fn migrate_trading_models_is_idempotent() {
        let pool = test_pool().await;
        migrate_trading_models(&pool).await.unwrap();
        migrate_trading_models(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn register_and_load_model_roundtrip() {
        let pool = test_pool().await;
        let model = register_model(
            &pool,
            "test-uuid-qqq",
            "QQQ",
            "PSQ",
            "models/qqq_tcn_v1.pt",
            "models/norm_stats_qqq_v1.json",
            5000.0,
            Some("bootstrap"),
        )
        .await
        .unwrap();
        assert_eq!(model.model_id, "test-uuid-qqq");
        assert_eq!(model.primary_symbol, "QQQ");
        assert_eq!(model.inverse_symbol, "PSQ");
        assert_eq!(model.model_path, "models/qqq_tcn_v1.pt");
        assert!(model.enabled);
        assert!((model.budget_usd - 5000.0).abs() < 1e-9);
        assert_eq!(model.notes.as_deref(), Some("bootstrap"));
        assert!(model.last_wf_ic.is_none());
        assert!(model.last_wf_at.is_none());

        // Re-load by id
        let loaded = load_model_by_id(&pool, "test-uuid-qqq").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().pair(), "QQQ/PSQ");

        // Not found
        let missing = load_model_by_id(&pool, "no-such-id").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn load_enabled_models_filters_correctly() {
        let pool = test_pool().await;
        register_model(
            &pool,
            "u-on-1",
            "QQQ",
            "PSQ",
            "m/qqq.pt",
            "m/qqq.json",
            5000.0,
            None,
        )
        .await
        .unwrap();
        register_model(
            &pool,
            "u-off",
            "NVDA",
            "NVDD",
            "m/nvda.pt",
            "m/nvda.json",
            5000.0,
            None,
        )
        .await
        .unwrap();
        register_model(
            &pool,
            "u-on-2",
            "PLTR",
            "PSQ",
            "m/pltr.pt",
            "m/pltr.json",
            7500.0,
            None,
        )
        .await
        .unwrap();

        // Disable the middle one
        update_model_enabled(&pool, "u-off", false).await.unwrap();

        let enabled = load_enabled_models(&pool).await.unwrap();
        assert_eq!(enabled.len(), 2);
        // load_enabled_models filters by enabled=1; both ordering fields
        // are exercised separately because deployed_at can collide.
        let ids: Vec<&str> = enabled.iter().map(|m| m.model_id.as_str()).collect();
        assert!(ids.contains(&"u-on-1"), "enabled missing u-on-1: {ids:?}");
        assert!(ids.contains(&"u-on-2"), "enabled missing u-on-2: {ids:?}");
        assert!(!ids.contains(&"u-off"), "enabled wrongly includes u-off: {ids:?}");

        // load_all_models returns all three; ordering by deployed_at DESC is
        // non-deterministic when all three rows share the same timestamp
        // (sub-second inserts). Test membership instead.
        let all = load_all_models(&pool).await.unwrap();
        assert_eq!(all.len(), 3);
        let all_ids: Vec<&str> = all.iter().map(|m| m.model_id.as_str()).collect();
        assert!(all_ids.contains(&"u-on-1"), "missing u-on-1: {all_ids:?}");
        assert!(all_ids.contains(&"u-off"), "missing u-off: {all_ids:?}");
        assert!(all_ids.contains(&"u-on-2"), "missing u-on-2: {all_ids:?}");
        // Disabled row must still be returned by load_all_models.
        assert!(!all.iter().find(|m| m.model_id == "u-off").unwrap().enabled);
    }

    #[tokio::test]
    async fn update_model_enabled_returns_none_for_unknown_id() {
        let pool = test_pool().await;
        let result = update_model_enabled(&pool, "no-such", true).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn record_walk_forward_result_persists() {
        let pool = test_pool().await;
        register_model(
            &pool,
            "u-wf",
            "NVDA",
            "NVDD",
            "m/nvda.pt",
            "m/nvda.json",
            5000.0,
            None,
        )
        .await
        .unwrap();

        record_walk_forward_result(&pool, "u-wf", Some(0.0823), 1_700_000_000)
            .await
            .unwrap();

        let model = load_model_by_id(&pool, "u-wf").await.unwrap().unwrap();
        assert!(model.last_wf_ic.is_some());
        assert!((model.last_wf_ic.unwrap() - 0.0823).abs() < 1e-9);
        assert_eq!(model.last_wf_at, Some(1_700_000_000));

        // Failed gate: last_wf_ic = None, but timestamp still recorded.
        record_walk_forward_result(&pool, "u-wf", None, 1_700_086_400)
            .await
            .unwrap();
        let model = load_model_by_id(&pool, "u-wf").await.unwrap().unwrap();
        assert!(model.last_wf_ic.is_none());
        assert_eq!(model.last_wf_at, Some(1_700_086_400));
    }

    #[test]
    fn bootstrap_default_model_uses_config_values() {
        let m = bootstrap_default_model("QQQ", "PSQ", "models/norm_stats_qqq_v1.json");
        assert_eq!(m.model_id, "bootstrap-default");
        assert_eq!(m.primary_symbol, "QQQ");
        assert_eq!(m.inverse_symbol, "PSQ");
        assert_eq!(m.norm_stats_path, "models/norm_stats_qqq_v1.json");
        assert_eq!(m.pair(), "QQQ/PSQ");
        assert!(m.is_enabled());
        assert!((m.budget_usd - 0.0).abs() < 1e-9);
        assert!(m.last_wf_ic.is_none());
        assert!(m.last_wf_at.is_none());
        assert_eq!(
            m.notes.as_deref(),
            Some("bootstrap from Config defaults")
        );
        assert_eq!(m.model_path, "<bootstrap>");
        // Sanity: deployed_at is "now-ish" (within 5 seconds of test time).
        let now = Utc::now().timestamp();
        assert!((m.deployed_at - now).abs() < 5, "deployed_at drift too large");
    }

    #[tokio::test]
    async fn resolve_active_models_falls_back_to_bootstrap_when_empty() {
        let pool = test_pool().await;
        let (models, count) = resolve_active_models(
            &pool,
            "QQQ",
            "PSQ",
            "models/norm_stats_qqq_v1.json",
        )
        .await
        .unwrap();
        assert_eq!(count, 0, "loaded_count must be 0 for cold-start");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "bootstrap-default");
        assert_eq!(models[0].primary_symbol, "QQQ");
        assert_eq!(models[0].inverse_symbol, "PSQ");
        assert!(models[0].is_enabled());
    }

    #[tokio::test]
    async fn resolve_active_models_uses_registry_when_present() {
        let pool = test_pool().await;
        register_model(
            &pool,
            "u-a",
            "QQQ",
            "PSQ",
            "m/qqq.pt",
            "m/qqq.json",
            5000.0,
            None,
        )
        .await
        .unwrap();
        register_model(
            &pool,
            "u-b",
            "NVDA",
            "NVDD",
            "m/nvda.pt",
            "m/nvda.json",
            7500.0,
            None,
        )
        .await
        .unwrap();
        // Disable u-b so only u-a is enabled.
        update_model_enabled(&pool, "u-b", false).await.unwrap();

        let (models, count) = resolve_active_models(
            &pool,
            "QQQ",
            "PSQ",
            "models/norm_stats_qqq_v1.json",
        )
        .await
        .unwrap();
        assert_eq!(count, 1, "loaded_count must equal enabled rows");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "u-a");
        assert_eq!(models[0].primary_symbol, "QQQ");
        assert_eq!(models[0].inverse_symbol, "PSQ");
        assert!(models[0].is_enabled());

        // Re-enable u-b, resolver should now see both.
        update_model_enabled(&pool, "u-b", true).await.unwrap();
        let (models, count) = resolve_active_models(
            &pool,
            "QQQ",
            "PSQ",
            "models/norm_stats_qqq_v1.json",
        )
        .await
        .unwrap();
        assert_eq!(count, 2);
        assert_eq!(models.len(), 2);
        let ids: Vec<&str> = models.iter().map(|m| m.model_id.as_str()).collect();
        assert!(ids.contains(&"u-a"));
        assert!(ids.contains(&"u-b"));
    }
}

// ---------------------------------------------------------------------------
// Mode-switch audit (Phase 3.4)
// ---------------------------------------------------------------------------

/// One row of the `mode_switches` audit table.
#[derive(Debug, Clone)]
pub struct ModeSwitchRow {
    pub id: i64,
    pub previous_mode: String,
    pub new_mode: String,
    pub parity_marker_age_secs: i64,
    pub authorized_by: String,
    pub timestamp: i64,
}

/// Append a row to `mode_switches` so operators can audit who flipped the
/// paper/live toggle and when. `previous_mode` and `new_mode` are stored as
/// the lowercase strings ("paper" / "live") for easy filtering.
pub async fn insert_mode_switch(
    pool: &DbPool,
    previous_mode: &str,
    new_mode: &str,
    parity_marker_age_secs: i64,
    authorized_by: &str,
    timestamp: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO mode_switches
            (previous_mode, new_mode, parity_marker_age_secs, authorized_by, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(previous_mode)
    .bind(new_mode)
    .bind(parity_marker_age_secs)
    .bind(authorized_by)
    .bind(timestamp)
    .execute(pool)
    .await
    .context("insert_mode_switch")?;
    Ok(())
}

/// Return the most recent `limit` mode switches, newest-first.
pub async fn fetch_recent_mode_switches(pool: &DbPool, limit: usize) -> Result<Vec<ModeSwitchRow>> {
    let rows = sqlx::query(
        "SELECT id, previous_mode, new_mode, parity_marker_age_secs, authorized_by, timestamp
         FROM mode_switches
         ORDER BY id DESC
         LIMIT ?1",
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .context("fetch_recent_mode_switches")?;

    Ok(rows
        .iter()
        .map(|r| ModeSwitchRow {
            id: r.get(0),
            previous_mode: r.get(1),
            new_mode: r.get(2),
            parity_marker_age_secs: r.get(3),
            authorized_by: r.get(4),
            timestamp: r.get(5),
        })
        .collect())
}

/// Return the age (in seconds) of the parity marker, or `None` if the marker
/// is missing or cannot be read. Used by the mode-toggle endpoint to enforce
/// the freshness guard at request time (not just at startup).
pub fn parity_marker_age_secs(marker_path: &str) -> Option<i64> {
    let path = std::path::Path::new(marker_path);
    let marker = crate::parity::read_marker(path).ok().flatten()?;
    let now = chrono::Utc::now().timestamp();
    Some((now - marker.verified_at).max(0))
}

// ── Phase 4: Advisor DB helpers ─────────────────────────────────────

/// Insert a briefing attempt into the log.
pub async fn insert_advisor_briefing_log(
    pool: &DbPool,
    for_date: &str,
    model_used: &str,
    context_hash: &str,
    context_json: &str,
    briefing_json: &str,
    parse_status: &str,
    latency_ms: i64,
) -> Result<i64> {
    let ts = chrono::Utc::now().timestamp();
    let row = sqlx::query(
        "INSERT INTO advisor_briefing_log \
         (ts, for_date, model_used, context_hash, context_json, briefing_json, parse_status, latency_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(ts)
    .bind(for_date)
    .bind(model_used)
    .bind(context_hash)
    .bind(context_json)
    .bind(briefing_json)
    .bind(parse_status)
    .bind(latency_ms)
    .execute(pool)
    .await
    .context("insert_advisor_briefing_log")?;
    Ok(row.last_insert_rowid())
}

/// Insert a chat interaction into the log.
pub async fn insert_advisor_chat_log(
    pool: &DbPool,
    question: &str,
    response: &str,
    model_used: &str,
    latency_ms: i64,
) -> Result<i64> {
    let ts = chrono::Utc::now().timestamp();
    let row = sqlx::query(
        "INSERT INTO advisor_chat_log (ts, question, response, model_used, latency_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(ts)
    .bind(question)
    .bind(response)
    .bind(model_used)
    .bind(latency_ms)
    .execute(pool)
    .await
    .context("insert_advisor_chat_log")?;
    Ok(row.last_insert_rowid())
}

/// Fetch the most recent N chat turns (question, response) pairs, oldest first.
pub async fn fetch_recent_chat_turns(
    pool: &DbPool,
    limit: usize,
) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query(
        "SELECT question, response FROM advisor_chat_log \
         ORDER BY id DESC LIMIT ?1",
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .context("fetch_recent_chat_turns")?;

    // Reverse so oldest is first (chronological order for the prompt).
    let mut turns: Vec<(String, String)> = rows
        .iter()
        .map(|r| {
            let q: String = r.get(0);
            let a: String = r.get(1);
            (q, a)
        })
        .collect();
    turns.reverse();
    Ok(turns)
}
