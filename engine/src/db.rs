use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::{sqlite::SqlitePoolOptions, Row, SqlitePool};
use tracing::info;

pub type DbPool = SqlitePool;

pub const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS candles (
    ts      INTEGER PRIMARY KEY,
    open    REAL    NOT NULL,
    high    REAL    NOT NULL,
    low     REAL    NOT NULL,
    close   REAL    NOT NULL,
    volume  REAL    NOT NULL,
    vwap    REAL    NOT NULL
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

    info!("database ready at {database_url}");
    Ok(pool)
}

/// Insert or update a candle row identified by its open-time unix timestamp.
pub async fn upsert_candle(pool: &DbPool, c: &Candle) -> Result<()> {
    sqlx::query(
        "INSERT INTO candles (ts, open, high, low, close, volume, vwap)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(ts) DO UPDATE SET
           open   = excluded.open,
           high   = excluded.high,
           low    = excluded.low,
           close  = excluded.close,
           volume = excluded.volume,
           vwap   = excluded.vwap",
    )
    .bind(c.ts)
    .bind(c.open)
    .bind(c.high)
    .bind(c.low)
    .bind(c.close)
    .bind(c.volume)
    .bind(c.vwap)
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
        "INSERT OR REPLACE INTO predictions (candle_ts, pred_1h, pred_4h, pred_24h, features_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
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
        "SELECT ts, open, high, low, close, volume, vwap
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
