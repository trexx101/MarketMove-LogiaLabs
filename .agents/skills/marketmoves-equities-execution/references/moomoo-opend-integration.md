# Moomoo OpenD Integration — Live Executor Reference

## Architecture
Moomoo (Futu) uses **OpenD** — a local desktop daemon that bridges to Futu's servers.
- Default: `127.0.0.1:11111` (protobuf-over-TCP)
- No public REST API; no Rust crate
- Official `moomoo` Python SDK wraps the OpenD connection
- The Rust `LiveExecutor` must shell out to Python scripts in `.agents/skills/moomooapi/scripts/`

## Environment variables (read by `scripts/common.py`)
| Var | Default | Purpose |
|-----|---------|---------|
| `FUTU_OPEND_HOST` | `127.0.0.1` | OpenD host |
| `FUTU_OPEND_PORT` | `11111` | OpenD port |
| `FUTU_TRD_ENV` | `SIMULATE` | `SIMULATE` (paper) or `REAL` (live) |
| `FUTU_LOGIN_ACCOUNT` | (empty) | Futu login account |
| `FUTU_LOGIN_PWD` | (empty) | Futu login password |
| `FUTU_DEFAULT_MARKET` | `NONE` | Default market |
| `FUTU_SECURITY_FIRM` | (empty) | Security firm (FUTUINC for US) |

## place_order.py CLI contract
Location: `.agents/skills/moomooapi/scripts/trade/place_order.py`

### Key arguments
```
--code US.QQQ          # Market-prefixed code (US., HK., SG., etc.)
--side BUY|SELL        # Trade direction
--quantity 10          # Share count (integer)
--price 500.00         # Limit price (required for NORMAL orders; any value for MARKET)
--order-type NORMAL|MARKET
--trd-env REAL|SIMULATE
--confirmed            # REQUIRED to actually submit REAL orders
--json                 # JSON output mode
--acc-id 123456        # Account ID (optional; auto-detected if omitted)
--security-firm FUTUINC  # Firm (FUTUINC=US, FUTUSECURITIES=HK, FUTUSG=SG)
--session RTH|ETH|OVERNIGHT|ALL  # US stocks only
```

### Exit codes
- `0` — order submitted successfully
- `1` — error (invalid args, OpenD unreachable, SDK error)
- `2` — **preview only** (REAL mode without `--confirmed`); order NOT executed

### JSON output (success)
```json
{
  "order_id": "123456789",
  "code": "US.QQQ",
  "side": "BUY",
  "quantity": 10,
  "price": 500.0,
  "order_type": "NORMAL",
  "trd_env": "REAL",
  "status": "submitted"
}
```

### JSON output (error)
```json
{"error": "error message"}
```

### Critical: `--confirmed` gate
When `--trd-env REAL` is set but `--confirmed` is NOT passed, the script prints a
preview JSON and exits with code 2. The order is NOT submitted. The LiveExecutor
must pass `--confirmed` to actually place live orders.

## Trade context creation (from common.py)
- `create_trade_context(market, security_firm)` → `OpenSecTradeContext` (stocks/ETFs)
- `create_future_trade_context(security_firm)` → `OpenFutureTradeContext` (futures)
- For US equities (QQQ, PSQ): use `create_trade_context` with market=US
- Connection checks OpenD reachability via socket before creating context
- SDK version check: `ai_type` param requires SDK >= 10.4.6408

## LiveExecutor integration plan
1. `engine/src/exec/live.rs` — `MoomooLiveExecutor` struct
2. `set_target_position(target, close, ts)` — same interface as PaperExecutor
3. For each fill:
   - Resolve symbol: Long → `US.{primary_symbol}`, Short → `US.{short_symbol}`
   - Resolve side: entry = BUY, exit = SELL (same as PaperExecutor PSQ remap)
   - Shell out: `python3 place_order.py --code US.QQQ --side BUY --quantity N --price P --trd-env REAL --confirmed --json`
   - Parse JSON output; extract `order_id`, `price` (fill price)
   - On exit code 2: treat as "needs confirmation" error (shouldn't happen if `--confirmed` is passed)
   - On exit code 1: parse `error` field, return `Err`
4. Bid-ask spread check before PSQ entry:
   - Query order book via `get_orderbook.py --code US.PSQ --json`
   - If `(ask - bid) / mid > spread_threshold` (default 0.5%), abort → Flat
5. OpenD watchdog:
   - TCP connect check to `FUTU_OPEND_HOST:FUTU_OPEND_PORT` each cycle
   - If unreachable: force Flat, broadcast `TelemetryEvent::StalenessAlert`

## Other useful scripts (in `.agents/skills/moomooapi/scripts/`)
- `trade/get_accounts.py` — list trading accounts + unlock status
- `trade/get_orders.py` — query order history/status
- `trade/get_positions.py` — query current positions
- `quote/get_snapshot.py` — real-time quote (for fill price estimation)
- `quote/get_orderbook.py` — bid/ask order book (for spread check)

## API limits
- Max 15 order requests per 30 seconds per account ID
- Min 0.02 seconds between consecutive orders
- Real accounts require manual trade-password unlock in OpenD GUI
