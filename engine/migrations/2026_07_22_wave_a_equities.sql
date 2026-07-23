-- Wave A: equities data layer
-- Creates equity_candles and macro_features tables for QQQ / macro ingestion.
-- These do NOT touch the crypto `candles` table — both coexist.

CREATE TABLE IF NOT EXISTS equity_candles (
    symbol TEXT    NOT NULL,    -- e.g. 'QQQ', 'AAPL', 'NVDA', '^VIX', 'TLT', 'GLD'
    ts     INTEGER NOT NULL,    -- unix seconds, midnight UTC for daily bars
    open   REAL    NOT NULL,
    high   REAL    NOT NULL,
    low    REAL    NOT NULL,
    close  REAL    NOT NULL,
    volume INTEGER NOT NULL DEFAULT 0,
    source TEXT    NOT NULL DEFAULT 'yahoo',  -- 'yahoo' | 'moomoo' | 'fred'
    PRIMARY KEY (symbol, ts)
);

CREATE INDEX IF NOT EXISTS equity_candles_symbol_ts_idx
    ON equity_candles (symbol, ts DESC);

CREATE TABLE IF NOT EXISTS equity_predictions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    symbol        TEXT    NOT NULL,    -- 'QQQ'
    candle_ts     INTEGER NOT NULL UNIQUE,
    pred_1d       REAL    NOT NULL,
    pred_5d       REAL    NOT NULL,
    pred_21d      REAL    NOT NULL,
    regime        TEXT    NOT NULL DEFAULT 'unknown',  -- 'uptrend' | 'downtrend' | 'neutral'
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
    side          TEXT    NOT NULL,     -- 'buy' | 'sell'
    qty           REAL    NOT NULL,     -- share count
    price         REAL    NOT NULL,
    fee           REAL    NOT NULL,
    realized_pnl  REAL    NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS equity_trades_symbol_ts_idx
    ON equity_trades (symbol, candle_ts DESC);

-- Daily data ingestion watermarks: track last successful fetch per (source, symbol)
-- so we can resume partial backfills without re-pulling everything.
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
