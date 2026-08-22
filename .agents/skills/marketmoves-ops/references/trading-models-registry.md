# `trading_models` registry workflow

## Schema

```sql
CREATE TABLE trading_models (
    model_id TEXT PRIMARY KEY,
    primary_symbol TEXT NOT NULL,
    inverse_symbol TEXT NOT NULL,
    model_path TEXT NOT NULL,
    norm_stats_path TEXT NOT NULL,
    budget_usd REAL NOT NULL DEFAULT 5000.0,
    enabled INTEGER NOT NULL DEFAULT 1,
    deployed_at INTEGER,
    last_wf_ic REAL,
    last_wf_at INTEGER,
    notes TEXT
);
```

## How the engine uses it

`engine/src/main.rs` calls `db::resolve_active_models(...)`, which returns every row with `enabled = 1`. The engine then spawns one `EquityScheduler` per model.

Each model gets:
- its own `norm_stats_path`
- its own paper executor
- its own in-memory `EquityStrategyParams`
- its own `last_processed_ts` watermark

## Adding a new model

```sql
INSERT INTO trading_models 
(model_id, primary_symbol, inverse_symbol, model_path, norm_stats_path, budget_usd, enabled, deployed_at, last_wf_ic, last_wf_at, notes)
VALUES 
('xlf-v1', 'XLF', 'FAZ', 'models/XLF/', '/models/XLF/norm_stats_xlf_v1.json', 5000.0, 1, strftime('%s','now'), 0.034, strftime('%s','now'), 'Notes');
```

## Inverse-symbol caveats

- **QQQ** has a clean -1x inverse: **PSQ**.
- **SMH** has no clean -1x inverse; **SOXS** is 3× leveraged.
- **XLF** has no clean -1x inverse; **FAZ** is 3× leveraged.

If you want to run SMH/XLF long-only, set their inverse symbols to these placeholders and disable shorting in their per-model strategy params:

```bash
PUT /api/strategy-config?model_id=smh-v1
{ "enable_shorting": false }
```

## Verifying models loaded

```bash
docker logs mmn-engine | grep "bootstrapping scheduler"
```

You should see one line per enabled model.

## Pitfall — `/app/models/` vs `/models/` path mismatch kills engine silently

The `deploy_models` Docker volume is mounted at `/models/` inside the container.
But `norm_stats_path` values in the `trading_models` table may point to
`/app/models/...` (e.g. from a dev environment or stale DB).

When the engine's per-model bootstrap loop (main.rs line ~228) hits a
`norm_stats_path` file that doesn't exist, it calls `process::exit(1)`.
**This is silent** — no panic, no error log, no stack trace. Docker sees
exit code 0 and restarts the container. Result: infinite restart loop.

**Fix:**
```bash
sudo sqlite3 /var/lib/docker/volumes/deploy_data/_data/candles.db \
  "UPDATE trading_models SET norm_stats_path = REPLACE(norm_stats_path, '/app/models/', '/models/') WHERE norm_stats_path LIKE '/app/models/%';"
```

**Verify:**
```bash
sudo sqlite3 /var/lib/docker/volumes/deploy_data/_data/candles.db \
  "SELECT model_id, enabled, norm_stats_path FROM trading_models;"
```

All paths should start with `/models/`, not `/app/models/`.

Disable rather than delete, so history is preserved:

```sql
UPDATE trading_models SET enabled = 0 WHERE model_id = 'nvda-v1';
```

Then restart `mmn-engine`.
