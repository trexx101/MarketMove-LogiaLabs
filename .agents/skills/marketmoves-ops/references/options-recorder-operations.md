# Options Tape Recorder — Operations

Captured 2026-08-21 after wiring the recorder service.

## Architecture

```
options_recorder (Rust binary)
  └─ spawns: python3 record_option_quotes.py (stdout = JSON lines, stderr = logs)
       ├─ chain discovery: get_option_chain → get_market_snapshot → filter by delta/liquidity
       └─ poll loop (15s): get_market_snapshot(all contracts + underlyings) → JSON lines
           └─ POST heartbeat to engine after each batch
```

The Rust binary:
- Reads JSON lines from Python stdout
- Writes parquet files to `data/options_tape/{underlying}/{chain_code}/{YYYY-MM-DD}.parquet`
- POSTs to `ENGINE_URL/api/internal/tape/heartbeat` after each poll batch

## Systemd service

Location: `~/.config/systemd/user/options-recorder.service`

Key env vars:
- `OPT_UNDERLYINGS=US.QQQ,US.SMH,US.XLF`
- `ENGINE_URL=http://127.0.0.1:9080` (through Caddy proxy)
- `PATH=.../inference/.venv/bin:...` (needed for moomoo-api)

Commands:
```bash
systemctl --user start options-recorder
systemctl --user stop options-recorder
systemctl --user status options-recorder
journalctl --user -u options-recorder -f
```

## Python environment

The recorder's Python script needs `moomoo-api` (Futu SDK). It's installed in
the `inference/.venv` at `/home/ubuntu/projects/MarketMoves/inference/.venv/`.

The systemd unit MUST prepend that venv to PATH:
```
Environment=PATH=/home/ubuntu/projects/MarketMoves/inference/.venv/bin:/usr/local/bin:/usr/bin:/bin
```

Without this, `common.py` fails with `moomoo-api not installed` and the
recorder exits immediately.

## Troubleshooting

### "moomoo-api not installed" / exit code 1
→ PATH doesn't include the inference venv. Check the systemd unit.

### "No contracts passed liquidity filters"
→ Market is closed (bids are 0). Expected outside 09:25-16:05 ET. The recorder
  will rescan chains at next market open.

### "Failed to parse quote JSON"
→ Benign. The Python SDK prints init logs to stdout (e.g. `_init_connect_sync:
  New connect ready`). The Rust binary tries to parse these as JSON, fails,
  and logs a warning. Actual quote data lines are valid JSON and parse fine.

### Recorder sends heartbeats but tape status shows empty
→ Check that the proxy Caddyfile has the `/api/internal/*` auth bypass
  (see `references/caddy-auth-bypass-internal-api.md`). Without it, heartbeats
  get 401 and the engine never sees them.

### Docker engine restart loop after enabling new models
→ Check `norm_stats_path` in the DB. If it starts with `/app/models/` instead
  of `/models/`, the engine silently exits during bootstrap. See
  `references/trading-models-registry.md` pitfall.

## Rebuilding

The Rust binary doesn't embed the Python script — it spawns it by path. So
Python script changes take effect immediately (no rebuild needed). The binary
only needs rebuilding when the Rust code changes:

```bash
cd /home/ubuntu/projects/MarketMoves
cargo build --release --bin options_recorder
systemctl --user restart options-recorder
```

## Weekend and holiday handling

The recorder uses OpenD's `request_trading_days(TradeDateMarket.US)` to build a
trading calendar at startup (frozenset of ~154 `YYYY-MM-DD` strings covering
±1 year). The calendar gates `is_market_hours()` so the recorder stays idle
on weekends AND market holidays (e.g., Labor Day, Thanksgiving, July 4).

When outside market hours, `seconds_until_next_market_open()` walks forward up
to 14 days to find the next trading day's 09:25 ET recording start, skipping
weekends and holidays. The idle loop sleeps in max-300s chunks (signal
responsive).

If OpenD is unreachable at startup, the recorder falls back to a simple
`weekday() < 5` check (Mon–Fri only, no holiday awareness).

**Relevant functions** (in `record_option_quotes.py`):
- `init_trading_calendar(ctx)` — call once after `create_quote_context()`
- `is_trading_day(now_et)` — checks calendar or falls back to weekday
- `seconds_until_next_market_open(now_et)` — next recording start for idle sleep
- `is_market_hours(now_et)` — now calls `is_trading_day()` before time check

**Pitfall — systemd stop timeout with long idle sleeps.** The idle loop sleeps
`min(seconds_to_next_open, 300)` per iteration. On a Friday after close, the
next trading day is ~63 hours away, so the sleep is 300s. If systemd's
`TimeOutStopSec` (default 90s) is exceeded while the process is mid-sleep,
systemd sends SIGKILL. To stop the recorder cleanly, allow extra time:
`systemctl --user stop options-recorder` will eventually succeed after the
current sleep chunk ends (max 300s), or use `pkill -f record_option_quotes;
pkill -f options_recorder` in sequence — the Python process responds to
SIGTERM within 5s of its next sleep-chunk boundary, and the Rust binary exits
when the child dies.

## Parquet output

Files: `data/options_tape/{underlying}/{chain_code}/{YYYY-MM-DD}.parquet`

Each row: timestamp_ms, underlying, chain_code, contract_code, bid, ask, last,
volume, oi, iv, delta, gamma, theta, underlying_price, session.