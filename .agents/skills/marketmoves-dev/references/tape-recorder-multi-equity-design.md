# Tape Recorder — Multi-Equity Design (implemented 2026-08-20)

**Status:** Code complete. Config, API endpoint, Python script, Rust binary,
and systemd unit all written. Engine image rebuilt. Deployment blocked by
pre-existing norm_stats crash-loop (see
`references/merge-recovery-and-boot-crashloop.md`).

## Five Finalized Design Decisions (locked with Inah 2026-08-20)

1. **Poll interval: 15s** — not 5s. 6 contracts × 1 req per 15s = 12 req/30s
   against 30 req/30s limit. 5s would hit 24 req/30s — dangerous on OpenD
   latency spikes (queue → rate-limit ban).

2. **Contracts: 1 Call + 1 Put per underlying** (delta ~0.45) = 6 total.
   Phase 5: no rolling, no multi-leg. Only record contracts the engine would
   execute. No adjacent strikes.

3. **Market hours only** (9:25–16:05 ET, 5-min buffer before/after).
   Recorder sleeps outside this window. No 24/7 recording — would flood
   parquet with 16.5 hours of null/stale greeks daily.

4. **Heartbeat via REST POST** — `POST /api/internal/tape/heartbeat`.
   Recorder is fire-and-forget. Engine is sole DB writer. Avoids SQLITE_BUSY
   and file permission issues crossing host systemd ↔ Docker boundary.

5. **No mid-day emergency re-scan** — continue logging the same contract even
   if liquidity degrades. Re-scan would break the Phase 2 synthetic backtester
   by changing the recorded contract mid-stream (BSM divergence check needs a
   continuous series for the same contract). Daily chain rollover only: re-scan
   runs when DTE drops below `dte_min` (30), before the next session open.

## Architecture: Two-Process Model

```
Python script (record_option_quotes.py)          Rust binary (options_recorder)
  ├─ Chain discovery (daily, before open)         ├─ Spawns Python via tokio::process
  │   ├─ get_option_expiration_date               ├─ Reads JSON lines from stdout
  │   ├─ Filter 30-45 DTE                          ├─ Deserializes → TapeRow
  │   ├─ get_option_chain (nearest expiry)        ├─ Writes parquet (per underlying/chain/date)
  │   ├─ Filter: bid>=0.01, spread<=8%, OI>=100   └─ After each poll batch:
  │   └─ Pick CALL + PUT with delta ~0.45              └─ POST /api/internal/tape/heartbeat
  ├─ Poll loop (every 15s, market hours only)
  │   ├─ For each contract: get_option_quote
  │   └─ Output JSON line per contract
  └─ Re-scan when DTE < dte_min (chain rollover, daily only)
```

## What Was Implemented

### Config (`engine/src/config.rs`)
- Added `underlyings: String` field to `OptionsEngineConfig`
- Defaults to `"US.QQQ"`, overridable via `OPT_UNDERLYINGS` env var
- Test assertion added to `options_config_defaults` test

### API Endpoint (`engine/src/api/options.rs` + `api/mod.rs`)
- New `POST /api/internal/tape/heartbeat` endpoint
- Accepts: `{ tape_id, underlying, chain_code, quota_accounting_json }`
- Calls `db::touch_tape_heartbeat()` — engine is sole DB writer
- Route registered in `api/mod.rs` alongside existing `GET /api/options/tape/status`

### Python Script (`.agents/skills/moomooapi/scripts/quote/record_option_quotes.py`)
- Full rewrite from scratch
- Chain discovery: `get_option_expiration_date` → filter DTE → `get_option_chain` → liquidity filter → pick delta-0.45 CALL + PUT
- Uses `get_option_quote([OptionStrategyLeg])` (NOT `get_stock_quote` — that was Bug #1)
- Market-hours gating: 9:25–16:05 ET, idle loop outside
- 15s poll loop, JSON lines to stdout
- Multi-underlying via `--underlyings` arg (comma-separated)
- No mid-day re-scan: contracts locked at morning scan

### Rust Binary (`engine/src/bin/options_recorder.rs`)
- Full rewrite
- Config-driven underlyings from `OPT_UNDERLYINGS` env var
- Spawns Python script, reads JSON lines from stdout
- Deserializes to `TapeRow`, writes parquet via `ArrowWriter`
- POSTs heartbeat to engine after each poll batch
- Daily chain scan before open (Python handles this)
- Idle loop outside market hours

### Systemd Unit (`~/.config/systemd/user/options-recorder.service`)
- `After=opend.service` — waits for OpenD
- Runs release binary: `target/release/options_recorder`
- `Restart=on-failure`, `RestartSec=10`
- Environment: `OPT_UNDERLYINGS`, `RUST_LOG`, `ENGINE_API_URL`

### Dockerfile Fix (`engine/Dockerfile`)
- Warmup layer now creates stub `engine/src/bin/options_recorder.rs`
- Without this, `cargo build --release` in the warmup layer fails because
  Cargo.toml declares a `[[bin]]` target whose source file doesn't exist

### Docker Compose Fix (`deploy/docker-compose.yml`)
- Engine healthcheck `start_period` increased from 20s → 90s
- Engine startup takes ~60-70s (equity backfill + VIX + FRED + 13-symbol
  sentiment seeding, all synchronous before HTTP bind)
- 20s start_period caused restart loop: 20s + 3×15s = 65s < startup time
- Retries increased from 3 → 5

## Parquet Storage

```
data/options_tape/
├── US.QQQ/
│   └── US.QQQ260919/
│       └── 2026-08-20.parquet
├── US.SMH/
│   └── US.SMH260918/
│       └── 2026-08-20.parquet
└── US.XLF/
    └── US.XLF260920/
        └── 2026-08-20.parquet
```

14-column Arrow schema: timestamp, underlying, chain_code, contract_code,
bid, ask, last, volume, open_interest, IV, delta, gamma, theta, underlying_price.

## Deployment Status

- ✅ Config field added + test passing
- ✅ POST heartbeat endpoint added + route registered
- ✅ Python script rewritten
- ✅ Rust binary rewritten, release build compiled
- ✅ Systemd unit created
- ✅ Docker image rebuilt with new endpoint
- ✅ Dockerfile warmup stub added for new `[[bin]]` target
- ✅ Healthcheck timing fixed
- ❌ Engine container not yet healthy — blocked by pre-existing norm_stats
  path mismatch in `trading_models` registry (same bug as
  `references/merge-recovery-and-boot-crashloop.md` Failure 3)
- ❌ Recorder not yet started — depends on engine being healthy first
- ❌ End-to-end heartbeat test not yet run

## Next Steps

1. Fix norm_stats paths in `trading_models` registry (UPDATE SQL or fix volume mount)
2. Restart engine container, verify `GET /api/options/tape/status` responds
3. Test `POST /api/internal/tape/heartbeat` returns 200
4. Start recorder: `systemctl --user start options-recorder`
5. Verify parquet files appear under `data/options_tape/`
6. Verify OptionsMonitor UI shows 3 healthy tapes
