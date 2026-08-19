use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use sqlx::{sqlite::SqlitePoolOptions, FromRow, Row, SqlitePool};
use tracing::{info, warn};

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
    candle_ts     INTEGER NOT NULL UNIQUE,
    pred_1d       REAL    NOT NULL,
    pred_5d       REAL    NOT NULL,
    pred_21d      REAL    NOT NULL,
    regime        TEXT    NOT NULL DEFAULT 'unknown',
    features_json TEXT    NOT NULL DEFAULT '{}',
    created_at    INTEGER NOT NULL,
    source        TEXT    NOT NULL DEFAULT 'qqq_tcn_v1'
);
CREATE INDEX IF NOT EXISTS equity_predictions_ts_idx
    ON equity_predictions (candle_ts DESC);

CREATE TABLE IF NOT EXISTS equity_trades (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
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

-- Options momentum engine tables (Phase 3).
-- All use TEXT PRIMARY KEY (UUID generated at app layer).

CREATE TABLE IF NOT EXISTS strategy_versions (
    id                      TEXT    PRIMARY KEY,
    equity                  TEXT    NOT NULL DEFAULT 'QQQ',
    family                  TEXT    NOT NULL,
    params_json             TEXT    NOT NULL,
    status                  TEXT    NOT NULL DEFAULT 'CANDIDATE',
    promotion_metadata_json TEXT    NOT NULL DEFAULT '{}',
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL
);

-- D13: promotions are queued here and applied ONLY at the daily candle
-- boundary for the equity, never mid-exit (checked again at apply time).
CREATE TABLE IF NOT EXISTS pending_promotions (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    version_id         TEXT    NOT NULL,
    equity             TEXT    NOT NULL,
    target_status      TEXT    NOT NULL,
    evidence_json      TEXT    NOT NULL DEFAULT '{}',  -- evidence validated at queue time
    requested_at       INTEGER NOT NULL,
    applied_at         INTEGER,          -- NULL until applied at boundary
    applied_result     TEXT,             -- 'PROMOTED' | 'DENIED: <reason>'
    UNIQUE(version_id, equity)           -- one pending request per candidate
);

CREATE TABLE IF NOT EXISTS option_positions (
    id                      TEXT    PRIMARY KEY,
    underlying              TEXT    NOT NULL,
    contract_code           TEXT    NOT NULL,
    strategy_version_id     TEXT    NOT NULL,
    entry_underlying_price  REAL    NOT NULL,
    entry_premium           REAL    NOT NULL DEFAULT 0.0,
    entry_spread            REAL    NOT NULL,
    entry_slippage_budget   REAL    NOT NULL,
    qty                     INTEGER NOT NULL,
    qty_filled_residual     INTEGER NOT NULL DEFAULT 0,
    status                  TEXT    NOT NULL DEFAULT 'OPEN',
    dte_at_entry            INTEGER NOT NULL,
    delta_at_entry          REAL    NOT NULL,
    realized_pnl            REAL,
    closed_at               INTEGER,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS option_positions_underlying_status_idx
    ON option_positions (underlying, status);

CREATE TABLE IF NOT EXISTS exit_signals (
    id                    TEXT    PRIMARY KEY,
    position_id           TEXT    NOT NULL,
    trigger_source        TEXT    NOT NULL,
    priority              INTEGER NOT NULL,
    stage                 INTEGER NOT NULL,
    intended_action       TEXT    NOT NULL,
    persisted_before_send INTEGER NOT NULL DEFAULT 0,
    created_at            INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS exit_signals_position_created_idx
    ON exit_signals (position_id, created_at);

CREATE TABLE IF NOT EXISTS option_tape_meta (
    id                    TEXT    PRIMARY KEY,
    underlying            TEXT    NOT NULL,
    chain_code            TEXT    NOT NULL,
    quota_accounting_json TEXT    NOT NULL DEFAULT '{}',
    created_at            INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS option_tape_meta_underlying_chain_idx
    ON option_tape_meta (underlying, chain_code);

CREATE TABLE IF NOT EXISTS exit_intent_log (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    position_id   TEXT    NOT NULL,
    stage         TEXT    NOT NULL,
    order_id      TEXT,
    limit_price   REAL    NOT NULL,
    quantity      REAL    NOT NULL,
    timestamp     TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS exit_intent_log_position_idx
    ON exit_intent_log (position_id, timestamp);

CREATE TABLE IF NOT EXISTS engine_events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts           INTEGER NOT NULL,
    category     TEXT    NOT NULL,  -- trade | data | system | strategy | alert | advisor
    severity     TEXT    NOT NULL,  -- info | warn | error
    mode         TEXT    NOT NULL,  -- paper | live
    source       TEXT    NOT NULL,  -- scheduler, data::yahoo, exec::paper, api::mode, etc.
    message      TEXT    NOT NULL,
    payload_json TEXT    NOT NULL DEFAULT '{}',
    equity       TEXT
);
CREATE INDEX IF NOT EXISTS engine_events_ts_idx ON engine_events (ts DESC);
CREATE INDEX IF NOT EXISTS engine_events_category_ts_idx ON engine_events (category, ts DESC);
CREATE INDEX IF NOT EXISTS engine_events_mode_idx ON engine_events (mode);

CREATE TABLE IF NOT EXISTS options_config_kv (
    key         TEXT    PRIMARY KEY,
    value_json  TEXT    NOT NULL,
    tier        TEXT    NOT NULL DEFAULT 'strategy',
    updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS hyperopt_runs (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at            INTEGER NOT NULL,
    finished_at           INTEGER,
    status                TEXT    NOT NULL DEFAULT 'RUNNING',
    equities_processed    INTEGER NOT NULL DEFAULT 0,
    candidates_stored     INTEGER NOT NULL DEFAULT 0,
    candidates_promoted   INTEGER NOT NULL DEFAULT 0,
    error                 TEXT
);

CREATE TABLE IF NOT EXISTS option_fills (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    position_id   TEXT    NOT NULL,
    stage         TEXT    NOT NULL,
    price         REAL    NOT NULL,
    quantity      REAL    NOT NULL,
    timestamp     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS option_fills_position_idx
    ON option_fills (position_id, timestamp);
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
    migrate_strategy_versions(&pool).await?;
    migrate_option_positions(&pool).await?;
    migrate_engine_events(&pool).await?;

    info!("database ready at {database_url}");
    Ok(pool)
}

/// Add `equity` column to strategy_versions if it doesn't exist.
pub async fn migrate_strategy_versions(pool: &DbPool) -> Result<()> {
    let rows = sqlx::query("PRAGMA table_info(strategy_versions)")
        .fetch_all(pool)
        .await
        .context("PRAGMA table_info(strategy_versions)")?;

    let existing: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();

    if !existing.iter().any(|name| name == "equity") {
        let sql = "ALTER TABLE strategy_versions ADD COLUMN equity TEXT NOT NULL DEFAULT 'QQQ'";
        sqlx::query(sql)
            .execute(pool)
            .await
            .context("adding column equity")?;
        info!("migrated strategy_versions: added column equity");
    }

    Ok(())
}

/// Migrate `option_positions` for existing databases: add columns introduced
/// in Phase 7 (`entry_premium`, `realized_pnl`, `closed_at`). Also rebuilds
/// `option_fills` / `exit_intent_log` if their `position_id` column is still
/// INTEGER (schema inconsistency fixed pre-live — tables created this week,
/// expected to be empty or near-empty in production).
pub async fn migrate_option_positions(pool: &DbPool) -> Result<()> {
    // 1. Add missing columns to option_positions
    let rows = sqlx::query("PRAGMA table_info(option_positions)")
        .fetch_all(pool)
        .await
        .context("PRAGMA table_info(option_positions)")?;

    let existing: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();

    let additions: &[(&str, &str)] = &[
        ("entry_premium", "REAL NOT NULL DEFAULT 0.0"),
        ("realized_pnl", "REAL"),
        ("closed_at", "INTEGER"),
    ];
    for (col, decl) in additions {
        if !existing.iter().any(|name| name == col) {
            let sql = format!("ALTER TABLE option_positions ADD COLUMN {col} {decl}");
            sqlx::query(&sql)
                .execute(pool)
                .await
                .with_context(|| format!("adding option_positions.{col}"))?;
            info!("migrated option_positions: added column {col}");
        }
    }

    // 2. Rebuild option_fills / exit_intent_log if position_id is INTEGER
    for (table, ddl) in [
        (
            "option_fills",
            r#"CREATE TABLE option_fills (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                position_id   TEXT    NOT NULL,
                stage         TEXT    NOT NULL,
                price         REAL    NOT NULL,
                quantity      REAL    NOT NULL,
                timestamp     INTEGER NOT NULL
            )"#,
        ),
        (
            "exit_intent_log",
            r#"CREATE TABLE exit_intent_log (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                position_id   TEXT    NOT NULL,
                stage         TEXT    NOT NULL,
                order_id      TEXT,
                limit_price   REAL    NOT NULL,
                quantity      REAL    NOT NULL,
                timestamp     TEXT    NOT NULL
            )"#,
        ),
    ] {
        let info = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(pool)
            .await
            .with_context(|| format!("PRAGMA table_info({table})"))?;
        let pos_id_type: Option<String> = info.iter().find_map(|r| {
            if r.get::<String, _>(1) == "position_id" {
                Some(r.get::<String, _>(2))
            } else {
                None
            }
        });
        if pos_id_type.as_deref() == Some("INTEGER") {
            let n: i64 = sqlx::query(&format!("SELECT COUNT(*) AS n FROM {table}"))
                .fetch_one(pool)
                .await?
                .get("n");
            let tmp = format!("{table}_new");
            let create_tmp = ddl.replace(table, &tmp);
            sqlx::query(&create_tmp).execute(pool).await?;
            if n > 0 {
                warn!("{table}: position_id is INTEGER with {n} rows — rebuilding (pre-live schema fix, rows cast to TEXT)");
                let copy = match table {
                    "option_fills" => format!(
                        "INSERT INTO {tmp} (id, position_id, stage, price, quantity, timestamp) \
                         SELECT id, CAST(position_id AS TEXT), stage, price, quantity, timestamp FROM {table}"
                    ),
                    _ => format!(
                        "INSERT INTO {tmp} (id, position_id, stage, order_id, limit_price, quantity, timestamp) \
                         SELECT id, CAST(position_id AS TEXT), stage, order_id, limit_price, quantity, timestamp FROM {table}"
                    ),
                };
                sqlx::query(&copy)
                    .execute(pool)
                    .await
                    .context("copying rows during rebuild")?;
            }
            sqlx::query(&format!("DROP TABLE {table}")).execute(pool).await?;
            sqlx::query(&format!("ALTER TABLE {tmp} RENAME TO {table}")).execute(pool).await?;
            info!("migrated {table}: position_id INTEGER -> TEXT");
        }
    }

    Ok(())
}

/// Add the nullable `equity` column to `engine_events` if missing
/// (table predates Phase 7 — existing rows keep equity = NULL).
pub async fn migrate_engine_events(pool: &DbPool) -> Result<()> {
    let rows = sqlx::query("PRAGMA table_info(engine_events)")
        .fetch_all(pool)
        .await
        .context("PRAGMA table_info(engine_events)")?;
    let existing: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();
    if !existing.iter().any(|n| n == "equity") {
        sqlx::query("ALTER TABLE engine_events ADD COLUMN equity TEXT")
            .execute(pool)
            .await
            .context("adding engine_events.equity")?;
        info!("migrated engine_events: added column equity");
    }
    Ok(())
}

/// Count strategy versions for a given equity.
pub async fn count_strategy_versions(pool: &DbPool, equity: &str) -> Result<i64> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM strategy_versions WHERE equity = ?1",
    )
    .bind(equity)
    .fetch_one(pool)
    .await
    .context("count_strategy_versions")?;
    Ok(row.get::<i64, _>("n"))
}

/// A pending promotion request (D13 queue).
#[derive(Debug, Clone)]
pub struct PendingPromotion {
    pub version_id: String,
    pub equity: String,
    pub target_status: String,
    /// JSON-serialized PromotionEvidence validated at queue time.
    pub evidence_json: String,
    pub requested_at: i64,
    pub applied_at: Option<i64>,
    pub applied_result: Option<String>,
}

/// Queue a promotion request (D13). Replaces any existing pending request
/// for the same candidate (UPSERT on (version_id, equity)).
pub async fn queue_pending_promotion(
    pool: &DbPool,
    version_id: &str,
    equity: &str,
    target_status: &str,
    evidence_json: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO pending_promotions (version_id, equity, target_status, evidence_json, requested_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(version_id, equity)
         DO UPDATE SET target_status = ?3, evidence_json = ?4, requested_at = ?5,
                       applied_at = NULL, applied_result = NULL",
    )
    .bind(version_id)
    .bind(equity)
    .bind(target_status)
    .bind(evidence_json)
    .bind(chrono::Utc::now().timestamp_millis())
    .execute(pool)
    .await
    .context("queue_pending_promotion")?;
    Ok(())
}

/// Fetch the pending (unapplied) promotion request for a candidate, if any.
pub async fn fetch_pending_promotion(
    pool: &DbPool,
    version_id: &str,
) -> Result<Option<PendingPromotion>> {
    let row = sqlx::query(
        "SELECT * FROM pending_promotions WHERE version_id = ?1 AND applied_at IS NULL",
    )
    .bind(version_id)
    .fetch_optional(pool)
    .await
    .context("fetch_pending_promotion")?;
    Ok(row.map(pending_promotion_from_row))
}

/// List all pending (unapplied) promotion requests.
pub async fn list_pending_promotions(pool: &DbPool) -> Result<Vec<PendingPromotion>> {
    let rows = sqlx::query(
        "SELECT * FROM pending_promotions WHERE applied_at IS NULL ORDER BY requested_at ASC",
    )
    .fetch_all(pool)
    .await
    .context("list_pending_promotions")?;
    Ok(rows.into_iter().map(pending_promotion_from_row).collect())
}

fn pending_promotion_from_row(row: sqlx::sqlite::SqliteRow) -> PendingPromotion {
    PendingPromotion {
        version_id: row.get("version_id"),
        equity: row.get("equity"),
        target_status: row.get("target_status"),
        evidence_json: row.get("evidence_json"),
        requested_at: row.get("requested_at"),
        applied_at: row.get("applied_at"),
        applied_result: row.get("applied_result"),
    }
}

/// Mark a pending promotion request as applied with its outcome.
pub async fn mark_pending_promotion_applied(
    pool: &DbPool,
    version_id: &str,
    result: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE pending_promotions SET applied_at = ?2, applied_result = ?3
         WHERE version_id = ?1 AND applied_at IS NULL",
    )
    .bind(version_id)
    .bind(chrono::Utc::now().timestamp_millis())
    .bind(result)
    .execute(pool)
    .await
    .context("mark_pending_promotion_applied")?;
    Ok(())
}

/// Count strategy versions grouped by status for a given equity.
pub async fn count_strategy_versions_by_status(
    pool: &DbPool,
    equity: &str,
) -> Result<std::collections::HashMap<String, i64>> {
    let rows = sqlx::query(
        "SELECT status, COUNT(*) AS n FROM strategy_versions WHERE equity = ?1 GROUP BY status",
    )
    .bind(equity)
    .fetch_all(pool)
    .await
    .context("count_strategy_versions_by_status")?;

    let mut result = std::collections::HashMap::new();
    for row in rows {
        let status: String = row.get("status");
        let count: i64 = row.get("n");
        result.insert(status, count);
    }
    Ok(result)
}

// ── Engine events ────────────────────────────────────────────────────────────

/// Insert an engine event. `payload_json` is an arbitrary JSON object string.
pub async fn insert_event(
    pool: &DbPool,
    category: &str,
    severity: &str,
    mode: &str,
    source: &str,
    message: &str,
    payload_json: &str,
    equity: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO engine_events (ts, category, severity, mode, source, message, payload_json, equity)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
    )
    .bind(Utc::now().timestamp())
    .bind(category)
    .bind(severity)
    .bind(mode)
    .bind(source)
    .bind(message)
    .bind(payload_json)
    .bind(equity)
    .execute(pool)
    .await
    .context("insert_event")?;
    Ok(())
}

/// A row from the engine_events table.
#[derive(Debug, Clone, Serialize)]
pub struct EngineEvent {
    pub id: i64,
    pub ts: i64,
    pub category: String,
    pub severity: String,
    pub mode: String,
    pub source: String,
    pub message: String,
    pub payload_json: String,
    pub equity: Option<String>,
}

/// Search engine events. All filters optional; results newest-first.
pub async fn search_events(
    pool: &DbPool,
    category: Option<&str>,
    mode: Option<&str>,
    severity: Option<&str>,
    equity: Option<&str>,
    since_ts: Option<i64>,
    limit: i64,
) -> Result<Vec<EngineEvent>> {
    let mut sql = String::from(
        "SELECT id, ts, category, severity, mode, source, message, payload_json, equity FROM engine_events WHERE 1=1",
    );
    if category.is_some() {
        sql.push_str(" AND category = ?");
    }
    if mode.is_some() {
        sql.push_str(" AND mode = ?");
    }
    if severity.is_some() {
        sql.push_str(" AND severity = ?");
    }
    if equity.is_some() {
        sql.push_str(" AND equity = ?");
    }
    if since_ts.is_some() {
        sql.push_str(" AND ts >= ?");
    }
    sql.push_str(" ORDER BY ts DESC, id DESC LIMIT ?");

    let mut q = sqlx::query(&sql);
    if let Some(c) = category {
        q = q.bind(c);
    }
    if let Some(m) = mode {
        q = q.bind(m);
    }
    if let Some(s) = severity {
        q = q.bind(s);
    }
    if let Some(e) = equity {
        q = q.bind(e);
    }
    if let Some(t) = since_ts {
        q = q.bind(t);
    }
    q = q.bind(limit);

    let rows = q.fetch_all(pool).await.context("search_events")?;
    Ok(rows
        .iter()
        .map(|r| EngineEvent {
            id: r.get("id"),
            ts: r.get("ts"),
            category: r.get("category"),
            severity: r.get("severity"),
            mode: r.get("mode"),
            source: r.get("source"),
            message: r.get("message"),
            payload_json: r.get("payload_json"),
            equity: r.get("equity"),
        })
        .collect())
}

/// Distinct category values present in engine_events (for UI filter dropdowns).
pub async fn event_categories(pool: &DbPool) -> Result<Vec<String>> {
    let rows = sqlx::query("SELECT DISTINCT category FROM engine_events ORDER BY category")
        .fetch_all(pool)
        .await
        .context("event_categories")?;
    Ok(rows.iter().map(|r| r.get::<String, _>("category")).collect())
}

// ── Options config KV store ──────────────────────────────────────────────────

/// Set an options config value. `tier` is 'strategy' (free to tune) or
/// 'rail' (risk rail — editable but bounded; changes are event-logged).
pub async fn set_options_config(
    pool: &DbPool,
    key: &str,
    value_json: &str,
    tier: &str,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO options_config_kv (key, value_json, tier, updated_at)
           VALUES (?1, ?2, ?3, ?4)
           ON CONFLICT(key) DO UPDATE SET value_json = ?2, tier = ?3, updated_at = ?4"#,
    )
    .bind(key)
    .bind(value_json)
    .bind(tier)
    .bind(Utc::now().timestamp())
    .execute(pool)
    .await
    .context("set_options_config")?;
    Ok(())
}

/// Get an options config value (raw JSON string) by key.
pub async fn get_options_config(pool: &DbPool, key: &str) -> Result<Option<String>> {
    let row = sqlx::query("SELECT value_json FROM options_config_kv WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .context("get_options_config")?;
    Ok(row.map(|r| r.get::<String, _>("value_json")))
}

/// List all options config entries as (key, value_json, tier, updated_at).
pub async fn list_options_config(pool: &DbPool) -> Result<Vec<(String, String, String, i64)>> {
    let rows = sqlx::query(
        "SELECT key, value_json, tier, updated_at FROM options_config_kv ORDER BY key",
    )
    .fetch_all(pool)
    .await
    .context("list_options_config")?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r.get::<String, _>("key"),
                r.get::<String, _>("value_json"),
                r.get::<String, _>("tier"),
                r.get::<i64, _>("updated_at"),
            )
        })
        .collect())
}

// ── Hyperopt runs ────────────────────────────────────────────────────────────

/// Record the start of a hyperopt run. Returns the run id.
pub async fn insert_hyperopt_run(pool: &DbPool) -> Result<i64> {
    let row = sqlx::query(
        "INSERT INTO hyperopt_runs (started_at) VALUES (?1) RETURNING id",
    )
    .bind(Utc::now().timestamp())
    .fetch_one(pool)
    .await
    .context("insert_hyperopt_run")?;
    Ok(row.get::<i64, _>("id"))
}

/// Mark a hyperopt run finished.
#[allow(clippy::too_many_arguments)]
pub async fn complete_hyperopt_run(
    pool: &DbPool,
    id: i64,
    status: &str,
    equities_processed: i64,
    candidates_stored: i64,
    candidates_promoted: i64,
    error: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE hyperopt_runs
           SET finished_at = ?2, status = ?3, equities_processed = ?4,
               candidates_stored = ?5, candidates_promoted = ?6, error = ?7
           WHERE id = ?1"#,
    )
    .bind(id)
    .bind(Utc::now().timestamp())
    .bind(status)
    .bind(equities_processed)
    .bind(candidates_stored)
    .bind(candidates_promoted)
    .bind(error)
    .execute(pool)
    .await
    .context("complete_hyperopt_run")?;
    Ok(())
}

/// A hyperopt run row.
#[derive(Debug, Clone, Serialize)]
pub struct HyperoptRun {
    pub id: i64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub equities_processed: i64,
    pub candidates_stored: i64,
    pub candidates_promoted: i64,
    pub error: Option<String>,
}

/// List recent hyperopt runs, newest first.
pub async fn list_hyperopt_runs(pool: &DbPool, limit: i64) -> Result<Vec<HyperoptRun>> {
    let rows = sqlx::query(
        r#"SELECT id, started_at, finished_at, status, equities_processed,
                  candidates_stored, candidates_promoted, error
           FROM hyperopt_runs ORDER BY started_at DESC, id DESC LIMIT ?1"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("list_hyperopt_runs")?;
    Ok(rows
        .iter()
        .map(|r| HyperoptRun {
            id: r.get("id"),
            started_at: r.get("started_at"),
            finished_at: r.get("finished_at"),
            status: r.get("status"),
            equities_processed: r.get("equities_processed"),
            candidates_stored: r.get("candidates_stored"),
            candidates_promoted: r.get("candidates_promoted"),
            error: r.get("error"),
        })
        .collect())
}

// ── Option positions ─────────────────────────────────────────────────────────

/// A row from option_positions.
#[derive(Debug, Clone, Serialize)]
pub struct OptionPosition {
    pub id: String,
    pub underlying: String,
    pub contract_code: String,
    pub strategy_version_id: String,
    pub entry_underlying_price: f64,
    pub entry_premium: f64,
    pub entry_spread: f64,
    pub entry_slippage_budget: f64,
    pub qty: i64,
    pub qty_filled_residual: i64,
    pub status: String,
    pub dte_at_entry: i64,
    pub delta_at_entry: f64,
    pub realized_pnl: Option<f64>,
    pub closed_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn row_to_option_position(r: &sqlx::sqlite::SqliteRow) -> OptionPosition {
    OptionPosition {
        id: r.get("id"),
        underlying: r.get("underlying"),
        contract_code: r.get("contract_code"),
        strategy_version_id: r.get("strategy_version_id"),
        entry_underlying_price: r.get("entry_underlying_price"),
        entry_premium: r.get("entry_premium"),
        entry_spread: r.get("entry_spread"),
        entry_slippage_budget: r.get("entry_slippage_budget"),
        qty: r.get("qty"),
        qty_filled_residual: r.get("qty_filled_residual"),
        status: r.get("status"),
        dte_at_entry: r.get("dte_at_entry"),
        delta_at_entry: r.get("delta_at_entry"),
        realized_pnl: r.get("realized_pnl"),
        closed_at: r.get("closed_at"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

/// List option positions, optionally filtered by underlying and/or status.
pub async fn list_option_positions(
    pool: &DbPool,
    underlying: Option<&str>,
    status: Option<&str>,
    limit: i64,
) -> Result<Vec<OptionPosition>> {
    let mut sql = String::from("SELECT * FROM option_positions WHERE 1=1");
    if underlying.is_some() {
        sql.push_str(" AND underlying = ?");
    }
    if status.is_some() {
        sql.push_str(" AND status = ?");
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ?");

    let mut q = sqlx::query(&sql);
    if let Some(u) = underlying {
        q = q.bind(u);
    }
    if let Some(s) = status {
        q = q.bind(s);
    }
    q = q.bind(limit);

    let rows = q.fetch_all(pool).await.context("list_option_positions")?;
    Ok(rows.iter().map(row_to_option_position).collect())
}

/// A fill row for an option position lifecycle.
#[derive(Debug, Clone, Serialize)]
pub struct OptionFill {
    pub id: i64,
    pub position_id: String,
    pub stage: String,
    pub price: f64,
    pub quantity: f64,
    pub timestamp: i64,
}

/// List fills for a position, chronological.
pub async fn list_option_fills(pool: &DbPool, position_id: &str) -> Result<Vec<OptionFill>> {
    let rows = sqlx::query(
        "SELECT id, position_id, stage, price, quantity, timestamp FROM option_fills WHERE position_id = ?1 ORDER BY timestamp ASC",
    )
    .bind(position_id)
    .fetch_all(pool)
    .await
    .context("list_option_fills")?;
    Ok(rows
        .iter()
        .map(|r| OptionFill {
            id: r.get("id"),
            position_id: r.get("position_id"),
            stage: r.get("stage"),
            price: r.get("price"),
            quantity: r.get("quantity"),
            timestamp: r.get("timestamp"),
        })
        .collect())
}

/// Tape recorder heartbeat: a meta row with last-heartbeat info.
#[derive(Debug, Clone, Serialize)]
pub struct TapeMeta {
    pub id: String,
    pub underlying: String,
    pub chain_code: String,
    pub quota_accounting_json: String,
    pub created_at: i64,
}

/// List tape recorder meta rows (heartbeat + quota accounting).
pub async fn list_tape_meta(pool: &DbPool) -> Result<Vec<TapeMeta>> {
    let rows = sqlx::query(
        "SELECT id, underlying, chain_code, quota_accounting_json, created_at FROM option_tape_meta ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .context("list_tape_meta")?;
    Ok(rows
        .iter()
        .map(|r| TapeMeta {
            id: r.get("id"),
            underlying: r.get("underlying"),
            chain_code: r.get("chain_code"),
            quota_accounting_json: r.get("quota_accounting_json"),
            created_at: r.get("created_at"),
        })
        .collect())
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
pub async fn load_position(pool: &DbPool) -> Result<i64> {
    match sqlx::query("SELECT position FROM signal_state WHERE id = 1")
        .fetch_one(pool)
        .await
    {
        Ok(row) => Ok(row.get(0)),
        Err(sqlx::Error::RowNotFound) => Ok(0),
        Err(e) => Err(e).context("load_position"),
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
    candle_ts: i64,
    position: i64,
    pred_4h: f64,
    pred_24h: f64,
    regime: i64,
    sma: f64,
) -> Result<()> {
    let created_at = Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO positions (candle_ts, position, pred_4h, pred_24h, regime, sma, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
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
        "INSERT INTO equity_trades (symbol, candle_ts, side, qty, price, fee, realized_pnl, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
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
        "SELECT id, candle_ts, side, qty, price, fee, realized_pnl, created_at
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

/// Return the price of the most recent trade (the entry price when position is open).
pub async fn fetch_entry_trade_price(pool: &DbPool) -> Result<Option<f64>> {
    let row = sqlx::query("SELECT price FROM trades ORDER BY id DESC LIMIT 1")
        .fetch_optional(pool)
        .await
        .context("fetch_entry_trade_price")?;
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
    let rows = sqlx::query(
        r#"SELECT symbol, ts, open, high, low, close, volume, source
           FROM equity_candles
           WHERE symbol = ?1
           ORDER BY ts ASC
           LIMIT ?2"#,
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
           ON CONFLICT(candle_ts) DO UPDATE SET
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

    // ── Phase 7 migration tests ──────────────────────────────────────────────

    /// Build a pool with the PRE-Phase-7 schema: option_positions without the
    /// new columns, option_fills / exit_intent_log with INTEGER position_id,
    /// and engine_events without the equity column.
    async fn old_schema_pool() -> DbPool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            r#"CREATE TABLE option_positions (
                id TEXT PRIMARY KEY, underlying TEXT NOT NULL, contract_code TEXT NOT NULL,
                strategy_version_id TEXT NOT NULL, entry_underlying_price REAL NOT NULL,
                entry_spread REAL NOT NULL, entry_slippage_budget REAL NOT NULL,
                qty INTEGER NOT NULL, qty_filled_residual INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'OPEN', dte_at_entry INTEGER NOT NULL,
                delta_at_entry REAL NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)"#,
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            r#"CREATE TABLE option_fills (
                id INTEGER PRIMARY KEY AUTOINCREMENT, position_id INTEGER NOT NULL,
                stage TEXT NOT NULL, price REAL NOT NULL, quantity REAL NOT NULL,
                timestamp INTEGER NOT NULL)"#,
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            r#"CREATE TABLE exit_intent_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT, position_id INTEGER NOT NULL,
                stage TEXT NOT NULL, order_id TEXT, limit_price REAL NOT NULL,
                quantity REAL NOT NULL, timestamp TEXT NOT NULL)"#,
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            r#"CREATE TABLE engine_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL,
                category TEXT NOT NULL, severity TEXT NOT NULL, mode TEXT NOT NULL,
                source TEXT NOT NULL, message TEXT NOT NULL,
                payload_json TEXT NOT NULL DEFAULT '{}')"#,
        )
        .execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn migrate_option_positions_adds_new_columns() {
        let pool = old_schema_pool().await;
        migrate_option_positions(&pool).await.unwrap();

        let rows = sqlx::query("PRAGMA table_info(option_positions)")
            .fetch_all(&pool).await.unwrap();
        let names: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();
        assert!(names.contains(&"entry_premium".to_string()));
        assert!(names.contains(&"realized_pnl".to_string()));
        assert!(names.contains(&"closed_at".to_string()));

        // Idempotent
        migrate_option_positions(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn migrate_option_positions_rebuilds_integer_position_ids() {
        let pool = old_schema_pool().await;

        // Seed rows with INTEGER position_ids (pre-live schema)
        sqlx::query(
            "INSERT INTO option_fills (position_id, stage, price, quantity, timestamp) VALUES (7, 'ENTRY', 1.25, 2.0, 100)",
        ).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO exit_intent_log (position_id, stage, order_id, limit_price, quantity, timestamp) VALUES (7, 'EXIT_STAGE_1', 'ord-1', 1.20, 2.0, '2026-08-19T00:00:00Z')",
        ).execute(&pool).await.unwrap();

        migrate_option_positions(&pool).await.unwrap();

        // position_id must now be TEXT and rows preserved
        let fill_row = sqlx::query("SELECT position_id, stage FROM option_fills")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(fill_row.get::<String, _>("position_id"), "7");
        assert_eq!(fill_row.get::<String, _>("stage"), "ENTRY");

        let intent_row = sqlx::query("SELECT position_id, order_id FROM exit_intent_log")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(intent_row.get::<String, _>("position_id"), "7");
        assert_eq!(intent_row.get::<String, _>("order_id"), "ord-1");

        // Verify column type flipped to TEXT
        let rows = sqlx::query("PRAGMA table_info(option_fills)")
            .fetch_all(&pool).await.unwrap();
        let pos_type: String = rows.iter()
            .find(|r| r.get::<String, _>(1) == "position_id")
            .map(|r| r.get::<String, _>(2))
            .unwrap();
        assert_eq!(pos_type, "TEXT");
    }

    #[tokio::test]
    async fn migrate_engine_events_adds_equity_column() {
        let pool = old_schema_pool().await;
        migrate_engine_events(&pool).await.unwrap();

        let rows = sqlx::query("PRAGMA table_info(engine_events)")
            .fetch_all(&pool).await.unwrap();
        let names: Vec<String> = rows.iter().map(|r| r.get::<String, _>(1)).collect();
        assert!(names.contains(&"equity".to_string()));

        // Idempotent
        migrate_engine_events(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn events_insert_and_search_roundtrip() {
        let pool = test_pool().await;
        insert_event(&pool, "strategy", "info", "paper", "options::entry",
            "SKIPPED_ENTRY: macro gate denied QQQ", r#"{"vix":24.1}"#, Some("QQQ"))
            .await.unwrap();
        insert_event(&pool, "trade", "warn", "paper", "options::exit",
            "circuit breaker tripped", "{}", Some("SMH"))
            .await.unwrap();

        // Search by category + mode
        let hits = search_events(&pool, Some("strategy"), Some("paper"), None, None, None, 50)
            .await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message, "SKIPPED_ENTRY: macro gate denied QQQ");
        assert_eq!(hits[0].equity.as_deref(), Some("QQQ"));

        // Search by equity
        let smh = search_events(&pool, None, None, None, Some("SMH"), None, 50).await.unwrap();
        assert_eq!(smh.len(), 1);
        assert_eq!(smh[0].severity, "warn");

        // Categories list
        let cats = event_categories(&pool).await.unwrap();
        assert!(cats.contains(&"strategy".to_string()));
        assert!(cats.contains(&"trade".to_string()));
    }

    #[tokio::test]
    async fn options_config_kv_set_get_list() {
        let pool = test_pool().await;
        set_options_config(&pool, "risk_pct", "0.01", "strategy").await.unwrap();
        set_options_config(&pool, "dte_exit_min", "7", "rail").await.unwrap();

        let v = get_options_config(&pool, "risk_pct").await.unwrap();
        assert_eq!(v.as_deref(), Some("0.01"));

        // Upsert overwrites
        set_options_config(&pool, "risk_pct", "0.02", "strategy").await.unwrap();
        let v = get_options_config(&pool, "risk_pct").await.unwrap();
        assert_eq!(v.as_deref(), Some("0.02"));

        let all = list_options_config(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].0, "risk_pct"); // ordered by key

        // Missing key
        let missing = get_options_config(&pool, "nope").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn hyperopt_runs_insert_complete_list() {
        let pool = test_pool().await;
        let id = insert_hyperopt_run(&pool).await.unwrap();

        // Running state visible
        let runs = list_hyperopt_runs(&pool, 10).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "RUNNING");
        assert!(runs[0].finished_at.is_none());

        complete_hyperopt_run(&pool, id, "SUCCESS", 3, 12, 0, None).await.unwrap();

        let runs = list_hyperopt_runs(&pool, 10).await.unwrap();
        assert_eq!(runs[0].status, "SUCCESS");
        assert_eq!(runs[0].equities_processed, 3);
        assert_eq!(runs[0].candidates_stored, 12);
        assert!(runs[0].finished_at.is_some());
        assert!(runs[0].error.is_none());

        // Failure path
        let id2 = insert_hyperopt_run(&pool).await.unwrap();
        complete_hyperopt_run(&pool, id2, "FAILED", 0, 0, 0, Some("tape missing")).await.unwrap();
        let runs = list_hyperopt_runs(&pool, 10).await.unwrap();
        assert_eq!(runs[0].status, "FAILED");
        assert_eq!(runs[0].error.as_deref(), Some("tape missing"));
    }

    #[tokio::test]
    async fn option_positions_and_fills_roundtrip() {
        let pool = test_pool().await;
        sqlx::query(
            r#"INSERT INTO option_positions
               (id, underlying, contract_code, strategy_version_id, entry_underlying_price,
                entry_premium, entry_spread, entry_slippage_budget, qty, dte_at_entry,
                delta_at_entry, created_at, updated_at)
               VALUES ('pos-1', 'QQQ', 'QQQ 260918C400', 'v_QQQ_1', 480.0, 5.25, 0.10, 0.5,
                       2, 34, 0.45, 100, 100)"#,
        ).execute(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO option_fills (position_id, stage, price, quantity, timestamp) VALUES ('pos-1', 'ENTRY', 5.25, 2.0, 100)",
        ).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO option_fills (position_id, stage, price, quantity, timestamp) VALUES ('pos-1', 'EXIT_STAGE_1', 5.90, 2.0, 200)",
        ).execute(&pool).await.unwrap();

        let open = list_option_positions(&pool, Some("QQQ"), Some("OPEN"), 10).await.unwrap();
        assert_eq!(open.len(), 1);
        assert!((open[0].entry_premium - 5.25).abs() < 1e-9);
        assert!(open[0].realized_pnl.is_none());

        let fills = list_option_fills(&pool, "pos-1").await.unwrap();
        assert_eq!(fills.len(), 2);
        assert_eq!(fills[0].stage, "ENTRY");
        assert_eq!(fills[1].stage, "EXIT_STAGE_1");

        // Filter by closed status returns nothing
        let closed = list_option_positions(&pool, None, Some("CLOSED"), 10).await.unwrap();
        assert_eq!(closed.len(), 0);
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

    // ===== Options momentum engine tables (Phase 3) ===================

    #[tokio::test]
    async fn options_tables_created_by_ddl() {
        let pool = test_pool().await;

        // Verify all 4 new tables exist
        let tables = vec![
            "strategy_versions",
            "option_positions",
            "exit_signals",
            "option_tape_meta",
        ];

        for table in tables {
            let row = sqlx::query(
                "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
            )
            .bind(table)
            .fetch_optional(&pool)
            .await
            .unwrap();

            assert!(row.is_some(), "table {} should exist", table);
        }

        // Verify indexes exist
        let indexes = vec![
            "option_positions_underlying_status_idx",
            "exit_signals_position_created_idx",
            "option_tape_meta_underlying_chain_idx",
        ];

        for index in indexes {
            let row = sqlx::query(
                "SELECT name FROM sqlite_master WHERE type='index' AND name=?",
            )
            .bind(index)
            .fetch_optional(&pool)
            .await
            .unwrap();

            assert!(row.is_some(), "index {} should exist", index);
        }
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
