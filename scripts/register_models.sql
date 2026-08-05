-- §8 step 2E: Register QQQ and NVDA trading models in the registry.
--
-- This is a one-time bootstrap insert. The engine's resolve_active_models()
-- will load these rows on startup instead of falling back to the
-- bootstrap-default synthetic model.
--
-- Run against the production DB:
--   python3 -c "import sqlite3; sqlite3.connect('data/candles.db').executescript(open('scripts/register_models.sql').read()); sqlite3.connect('data/candles.db').commit()"

INSERT OR IGNORE INTO trading_models
    (model_id, primary_symbol, inverse_symbol, model_path,
     norm_stats_path, budget_usd, enabled, deployed_at,
     last_wf_ic, last_wf_at, notes)
VALUES
    -- QQQ model (existing, trained 2026-07-27)
    ('qqq-v1', 'QQQ', 'PSQ', 'models/',
     'models/norm_stats_qqq_v1.json', 10000.0, 1,
     strftime('%s','now'), 0.034, strftime('%s','2026-07-27'),
     'LGBM huber/100/6/0.01 + TCN 7-block. IC gate 0.034. Walk-forward 5y/1y.'),

    -- NVDA model (trained 2026-08-05, IC=0.0823, 2.74x gate margin)
    ('nvda-v1', 'NVDA', 'NVDD', 'models/NVDA/',
     'models/NVDA/norm_stats_nvda_v1.json', 5000.0, 1,
     strftime('%s','now'), 0.0823, strftime('%s','2026-08-05'),
     'LGBM + TCN. Mean IC 0.0823. All 3 horizons positive. Walk-forward 5y/1y.');
