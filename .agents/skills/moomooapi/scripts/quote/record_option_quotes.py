#!/usr/bin/env python3
"""
Options tape recorder: discover contracts, poll quotes, stream JSON lines to stdout.

Design decisions (2026-08-20):
- Polls every 15s (6 contracts × 1 req = 12 req/30s vs 60/30s limit = 80% headroom)
- 1 Call + 1 Put per underlying, delta closest to 0.45
- Market hours only: 09:25–16:05 ET (5-min buffer before open / after close)
- No mid-day re-scan: contract selection locked at morning scan, record the ugly data
- Daily chain scan runs before open when DTE < dte_min

Output: JSON lines to stdout, one object per contract per poll:
    {"timestamp_ms": 1234567890000, "underlying": "US.QQQ",
     "chain_code": "US.QQQ260919", "contract_code": "US.QQQ260919C00530000",
     "bid": 5.2, "ask": 5.3, "last": 5.25, "volume": 100, "oi": 2500,
     "iv": 0.28, "delta": 0.45, "gamma": 0.003, "theta": -0.15,
     "underlying_price": 530.0,
     "session": "pre|regular|post", "underlying_code": "US.QQQ"}

Usage:
    python3 record_option_quotes.py \\
        --underlyings US.QQQ,US.SMH,US.XLF \\
        --dte-min 30 --dte-max 45 \\
        --delta-target 0.45 --bid-min 0.01 --spread-cap 0.08 --oi-min 100 \\
        --interval 15

Environment:
    FUTU_OPEND_HOST (default 127.0.0.1)
    FUTU_OPEND_PORT (default 11111)
"""

import argparse
import json
import sys
import os as _os
import time
import signal
from datetime import datetime, timedelta, timezone

sys.path.insert(0, _os.path.normpath(_os.path.join(_os.path.dirname(_os.path.abspath(__file__)), "..")))
from common import create_quote_context, safe_close, RET_OK, TradeDateMarket

# US market hours ET (UTC-4 in DST, UTC-5 standard)
# We use America/New_York via zoneinfo (Python 3.9+) for correctness
try:
    from zoneinfo import ZoneInfo
    ET = ZoneInfo("America/New_York")
except ImportError:
    # Fallback: UTC-4 (works for DST months March-November)
    ET = timezone(timedelta(hours=-4))

# Market session boundaries (ET)
PRE_OPEN_MINUTES = 5      # Start recording 5 min before open
POST_CLOSE_MINUTES = 5    # Stop recording 5 min after close

# ---------------------------------------------------------------------------
# Trading calendar (weekends + holidays)
# ---------------------------------------------------------------------------

# In-memory cache — set at startup, never changes for the lifetime of the process
_trading_days = None  # frozenset of "YYYY-MM-DD" strings, or False for fallback, or None before init


def _fetch_trading_days_from_opend(ctx, lookback_days=90, lookahead_days=365):
    """
    Fetch US trading days from OpenD.

    Returns a frozenset of "YYYY-MM-DD" strings, or None on failure.
    Uses request_trading_days (not get_trading_days — that method hangs).
    """
    start = (datetime.now(ET) - timedelta(days=lookback_days)).strftime("%Y-%m-%d")
    end   = (datetime.now(ET) + timedelta(days=lookahead_days)).strftime("%Y-%m-%d")

    ret, data = ctx.request_trading_days(market=TradeDateMarket.US, start=start, end=end)
    if ret != RET_OK:
        print(json.dumps({"calendar_error": f"request_trading_days returned {ret}"}),
              file=sys.stderr, flush=True)
        return None

    if data is None or (hasattr(data, "empty") and data.empty):
        print(json.dumps({"calendar_error": "empty response"}),
              file=sys.stderr, flush=True)
        return None

    trading_dates = set()
    if hasattr(data, "iloc"):
        # DataFrame: column "time" has date strings
        for i in range(len(data)):
            row = data.iloc[i]
            t = str(row.get("time", ""))
            if t:
                trading_dates.add(t)
    elif isinstance(data, (list, tuple)):
        for item in data:
            t = str(item.get("time", "")) if isinstance(item, dict) else str(item)
            if t:
                trading_dates.add(t)

    if not trading_dates:
        return None
    return frozenset(trading_dates)


def _weekday_fallback(now_et):
    """Fallback when OpenD is unreachable: Mon–Fri only."""
    return now_et.weekday() < 5  # 0=Mon, 4=Fri


def init_trading_calendar(ctx):
    """
    Initialise the trading calendar cache. Call once at startup.

    Returns True on success (OpenD calendar loaded), False on fallback to weekday check.
    """
    global _trading_days
    result = _fetch_trading_days_from_opend(ctx)
    if result is not None:
        _trading_days = result
        return True
    else:
        print(json.dumps({"calendar_fallback": "Using weekday-only filter (Mon–Fri)"}),
              file=sys.stderr, flush=True)
        _trading_days = False  # sentinel: use weekday fallback
        return False


def is_trading_day(now_et=None):
    """
    Check whether now_et falls on a trading day.

    Covers weekends AND market holidays when the OpenD calendar is available.
    Falls back to weekday-only check (Mon–Fri) if the calendar failed to load.
    """
    if now_et is None:
        now_et = datetime.now(ET)

    if _trading_days is None:
        # Calendar not initialised yet — warn and fall back
        return _weekday_fallback(now_et)

    if _trading_days is False:
        # Explicit fallback sentinel
        return _weekday_fallback(now_et)

    date_str = now_et.strftime("%Y-%m-%d")
    return date_str in _trading_days
MARKET_OPEN = (9, 30)     # 09:30 ET
MARKET_CLOSE = (16, 0)    # 16:00 ET


def is_market_hours(now_et=None):
    """Check if current time is within recording window (09:25–16:05 ET) on a trading day."""
    if now_et is None:
        now_et = datetime.now(ET)

    if not is_trading_day(now_et):
        return False

    open_dt = now_et.replace(hour=MARKET_OPEN[0], minute=MARKET_OPEN[1], second=0, microsecond=0)
    close_dt = now_et.replace(hour=MARKET_CLOSE[0], minute=MARKET_CLOSE[1], second=0, microsecond=0)
    start = open_dt - timedelta(minutes=PRE_OPEN_MINUTES)
    end = close_dt + timedelta(minutes=POST_CLOSE_MINUTES)
    return start <= now_et <= end


def get_session_label(now_et=None):
    """Return 'pre', 'regular', or 'post' for the current session phase."""
    if now_et is None:
        now_et = datetime.now(ET)
    open_dt = now_et.replace(hour=MARKET_OPEN[0], minute=MARKET_OPEN[1], second=0, microsecond=0)
    close_dt = now_et.replace(hour=MARKET_CLOSE[0], minute=MARKET_CLOSE[1], second=0, microsecond=0)
    start = open_dt - timedelta(minutes=PRE_OPEN_MINUTES)
    end = close_dt + timedelta(minutes=POST_CLOSE_MINUTES)
    if start <= now_et < open_dt:
        return "pre"
    elif open_dt <= now_et <= close_dt:
        return "regular"
    elif close_dt < now_et <= end:
        return "post"
    return None


def seconds_until_market_open(now_et=None):
    """Seconds until recording window opens."""
    if now_et is None:
        now_et = datetime.now(ET)
    open_dt = now_et.replace(hour=MARKET_OPEN[0], minute=MARKET_OPEN[1], second=0, microsecond=0)
    start = open_dt - timedelta(minutes=PRE_OPEN_MINUTES)
    if now_et < start:
        return (start - now_et).total_seconds()
    return 0


def seconds_until_next_market_open(now_et=None):
    """
    Seconds until the next trading day's recording start (market open - PRE_OPEN_MINUTES).

    Walks forward up to 14 days, skipping weekends and holidays via is_trading_day().
    Falls back to 86400 (24h) if no trading day found in range.
    """
    if now_et is None:
        now_et = datetime.now(ET)

    recording_start_minutes = MARKET_OPEN[0] * 60 + MARKET_OPEN[1] - PRE_OPEN_MINUTES

    for offset in range(14):
        day = now_et + timedelta(days=offset)
        # If it's the same day and we're already past recording start, skip to tomorrow
        if offset == 0 and (now_et.hour * 60 + now_et.minute) >= recording_start_minutes:
            continue
        candidate = day.replace(hour=MARKET_OPEN[0], minute=MARKET_OPEN[1], second=0, microsecond=0)
        candidate = candidate - timedelta(minutes=PRE_OPEN_MINUTES)
        if candidate > now_et and is_trading_day(candidate):
            return max(0, (candidate - now_et).total_seconds())

    return 86400


def discover_contracts(ctx, underlying, dte_min, dte_max, delta_target,
                       bid_min, spread_cap_pct, oi_min):
    """
    Discover the best Call and Put for an underlying.

    Steps:
    1. Get option expiration dates
    2. Filter to dte_min–dte_max window
    3. Pick nearest valid expiry
    4. Get option chain for that expiry
    5. Batch snapshot all chain codes + underlying to get real pricing
    6. Filter: bid >= bid_min, spread <= spread_cap of mid, OI >= oi_min
    7. From surviving contracts, pick 1 Call + 1 Put closest to delta_target

    Returns: dict with 'call' and 'put' contract codes, plus 'expiry' and 'chain_code'
    """
    # 1. Get expiration dates
    ret, exp_data = ctx.get_option_expiration_date(underlying)
    if ret != RET_OK:
        return {"error": f"get_option_expiration_date failed: {exp_data}"}

    if exp_data is None or "strike_time" not in exp_data or exp_data["strike_time"].empty:
        return {"error": f"No expiration dates for {underlying}"}

    # 2. Filter to DTE window
    today = datetime.now(ET).date()
    target_start = today + timedelta(days=dte_min)
    target_end = today + timedelta(days=dte_max)

    valid_exps = []
    for exp_str in exp_data["strike_time"]:
        try:
            exp_date = datetime.strptime(exp_str, "%Y-%m-%d").date()
            if target_start <= exp_date <= target_end:
                valid_exps.append((exp_str, exp_date))
        except ValueError:
            continue

    if not valid_exps:
        return {"error": f"No expirations in {dte_min}-{dte_max} DTE window for {underlying}"}

    # 3. Pick nearest expiry
    valid_exps.sort(key=lambda x: abs((x[1] - today).days))
    chosen_exp, chosen_date = valid_exps[0]

    # 4. Get option chain
    ret, chain_df = ctx.get_option_chain(underlying, start=chosen_exp, end=chosen_exp)
    if ret != RET_OK:
        return {"error": f"get_option_chain failed: {chain_df}"}

    if chain_df is None or chain_df.empty:
        return {"error": f"Empty option chain for {underlying} exp {chosen_exp}"}

    # Extract chain code from first contract code
    first_code = str(chain_df.iloc[0].get("code", ""))
    if first_code:
        for i, c in enumerate(first_code):
            if c in ("C", "P") and i > 3:
                chain_code = first_code[:i]
                break
        else:
            chain_code = first_code
    else:
        chain_code = underlying

    # 5. Batch snapshot all chain codes + underlying
    # Collect all option codes
    all_codes = []
    for _, row in chain_df.iterrows():
        code = str(row.get("code", ""))
        opt_type = str(row.get("option_type", "")).upper()
        if opt_type in ("CALL", "PUT") and code:
            all_codes.append(code)

    if not all_codes:
        return {"error": f"No valid option codes in chain for {underlying}"}

    # Fetch snapshot for all codes + underlying in one batch (max 400 per call)
    snapshot_codes = all_codes + [underlying]
    SNAPSHOT_BATCH = 400
    snapshot_rows = []
    underlying_price = 0.0

    for batch_start in range(0, len(snapshot_codes), SNAPSHOT_BATCH):
        batch = snapshot_codes[batch_start:batch_start + SNAPSHOT_BATCH]
        ret, snap_df = ctx.get_market_snapshot(batch)
        if ret != RET_OK:
            return {"error": f"get_market_snapshot failed: {snap_df}"}

        if snap_df is not None and not snap_df.empty:
            for i in range(len(snap_df)):
                row = snap_df.iloc[i]
                code = str(row.get("code", ""))
                if code == underlying:
                    underlying_price = _safe_float(row.get("last_price", 0))
                elif str(row.get("option_valid", "")).upper() in ("TRUE", "1"):
                    snapshot_rows.append(row)

    if underlying_price <= 0:
        # Fallback: try to get underlying price separately
        ret, snap_df = ctx.get_market_snapshot([underlying])
        if ret == RET_OK and snap_df is not None and not snap_df.empty:
            underlying_price = _safe_float(snap_df.iloc[0].get("last_price", 0))

    # 6. Filter contracts by liquidity from snapshot data
    candidates = {"call": None, "put": None}
    for row in snapshot_rows:
        code = str(row.get("code", ""))
        opt_type = str(row.get("option_type", "")).upper()
        if opt_type not in ("CALL", "PUT"):
            continue

        bid = _safe_float(row.get("bid_price", 0))
        ask = _safe_float(row.get("ask_price", 0))
        last = _safe_float(row.get("last_price", 0))
        volume = int(_safe_float(row.get("volume", 0)))
        oi = int(_safe_float(row.get("option_open_interest", 0)))
        iv = _safe_float(row.get("option_implied_volatility", 0))
        delta = _safe_float(row.get("option_delta", 0))
        gamma = _safe_float(row.get("option_gamma", 0))
        theta = _safe_float(row.get("option_theta", 0))
        strike = _safe_float(row.get("option_strike_price", 0))

        # Liquidity filters
        if bid < bid_min:
            continue
        mid = (bid + ask) / 2 if (bid + ask) > 0 else 0
        if mid > 0:
            spread = (ask - bid) / mid
            if spread > spread_cap_pct:
                continue
        if oi < oi_min:
            continue

        # 7. Track closest-to-target delta (absolute delta)
        target_key = "call" if opt_type == "CALL" else "put"
        delta_abs = abs(abs(delta) - delta_target)

        if candidates[target_key] is None or delta_abs < candidates[target_key]["_delta_diff"]:
            candidates[target_key] = {
                "code": code,
                "bid": bid,
                "ask": ask,
                "last": last,
                "volume": volume,
                "oi": oi,
                "iv": iv,
                "delta": delta,
                "gamma": gamma,
                "theta": theta,
                "stock_price": underlying_price,
                "strike": strike,
                "_delta_diff": delta_abs,
            }

    # Build result
    result = {
        "underlying": underlying,
        "expiry": chosen_exp,
        "chain_code": chain_code,
    }

    for key in ("call", "put"):
        c = candidates[key]
        if c:
            result[key] = {"code": c["code"], "strike": c["strike"]}
        else:
            result[key] = None

    if not result["call"] and not result["put"]:
        result["error"] = "No contracts passed liquidity filters"

    return result


def _safe_float(val):
    """Safely convert to float, handling None, NaN, 'N/A' strings."""
    if val is None:
        return 0.0
    sval = str(val).strip()
    if sval in ("", "nan", "NaN", "None", "N/A", "n/a"):
        return 0.0
    try:
        return float(sval)
    except (ValueError, TypeError):
        return 0.0


def poll_contracts(ctx, contracts_info, interval):
    """
    Poll option quotes for all contracts and stream JSON lines to stdout.

    contracts_info: list of dicts with keys:
        - code: contract code
        - underlying: underlying code
        - chain_code: chain code

    Uses get_market_snapshot for all contracts + underlyings in one batch call.
    """
    session = get_session_label()
    if session is None:
        return False  # Outside recording window

    now_ms = int(time.time() * 1000)

    if not contracts_info:
        return True

    # Collect all contract codes + unique underlyings
    codes = [c["code"] for c in contracts_info]
    underlyings = list(set(c["underlying"] for c in contracts_info))
    snapshot_codes = codes + underlyings

    ret, snap_df = ctx.get_market_snapshot(snapshot_codes)
    if ret != RET_OK:
        print(json.dumps({"error": f"get_market_snapshot failed: {snap_df}"}),
              file=sys.stderr, flush=True)
        return False

    if snap_df is None or snap_df.empty:
        return True

    # Build lookup: code → row
    row_by_code = {}
    for i in range(len(snap_df)):
        row = snap_df.iloc[i]
        row_by_code[str(row.get("code", ""))] = row

    # Extract underlying prices
    underlying_prices = {}
    for u in underlyings:
        if u in row_by_code:
            underlying_prices[u] = _safe_float(row_by_code[u].get("last_price", 0))

    for c in contracts_info:
        code = c["code"]
        row = row_by_code.get(code)
        if row is None:
            continue

        bid = _safe_float(row.get("bid_price", 0))
        ask = _safe_float(row.get("ask_price", 0))
        last = _safe_float(row.get("last_price", 0))
        volume = int(_safe_float(row.get("volume", 0)))
        oi = int(_safe_float(row.get("option_open_interest", 0)))
        iv = _safe_float(row.get("option_implied_volatility", 0))
        delta = _safe_float(row.get("option_delta", 0))
        gamma = _safe_float(row.get("option_gamma", 0))
        theta = _safe_float(row.get("option_theta", 0))
        underlying_price = underlying_prices.get(c["underlying"], 0.0)

        quote = {
            "timestamp_ms": now_ms,
            "underlying": c["underlying"],
            "chain_code": c["chain_code"],
            "contract_code": code,
            "bid": bid,
            "ask": ask,
            "last": last,
            "volume": volume,
            "oi": oi,
            "iv": iv,
            "delta": delta,
            "gamma": gamma,
            "theta": theta,
            "underlying_price": underlying_price,
            "session": session,
        }
        print(json.dumps(quote), flush=True)

    return True


def main():
    parser = argparse.ArgumentParser(description="Options tape recorder")
    parser.add_argument("--underlyings", required=True,
                        help="Comma-separated underlying codes (e.g. US.QQQ,US.SMH,US.XLF)")
    parser.add_argument("--dte-min", type=int, default=30, help="Min DTE (default 30)")
    parser.add_argument("--dte-max", type=int, default=45, help="Max DTE (default 45)")
    parser.add_argument("--delta-target", type=float, default=0.45, help="Target delta (default 0.45)")
    parser.add_argument("--bid-min", type=float, default=0.01, help="Min bid price (default 0.01)")
    parser.add_argument("--spread-cap", type=float, default=0.08, help="Max spread %% of mid (default 0.08)")
    parser.add_argument("--oi-min", type=int, default=100, help="Min open interest (default 100)")
    parser.add_argument("--interval", type=float, default=15.0, help="Poll interval seconds (default 15)")
    parser.add_argument("--engine-url", default="http://127.0.0.1:9080",
                        help="Engine API base URL for heartbeat (default http://127.0.0.1:9080)")
    args = parser.parse_args()

    underlyings = [u.strip() for u in args.underlyings.split(",") if u.strip()]
    if not underlyings:
        print(json.dumps({"error": "No underlyings provided"}), file=sys.stderr)
        sys.exit(1)

    # Graceful shutdown
    running = [True]
    def handle_signal(signum, frame):
        running[0] = False
    signal.signal(signal.SIGTERM, handle_signal)
    signal.signal(signal.SIGINT, handle_signal)

    ctx = None
    try:
        ctx = create_quote_context()

        # Load trading calendar (weekends + holidays) from OpenD
        ok = init_trading_calendar(ctx)
        if ok:
            print(json.dumps({"calendar_loaded": True, "trading_days": len(_trading_days)}),
                  file=sys.stderr, flush=True)

        # State: per-underlying contract selection (locked at morning scan)
        selections = {}  # underlying → {call: code, put: code, chain_code, expiry}

        while running[0]:
            now_et = datetime.now(ET)

            # Check if we need to run the daily chain scan
            need_scan = False
            if not selections:
                need_scan = True
            else:
                today = now_et.date()
                # Check if any underlying has no selection or DTE expired
                for u in underlyings:
                    if u not in selections or not selections[u].get("call") and not selections[u].get("put"):
                        need_scan = True
                        break
                # Roll the chain when a held expiry drifts below dte_min: the
                # docstring promises a daily scan when DTE < dte_min, but the
                # old condition only rescanned on a missing selection. Without
                # this, a chain selected at morning scan stays locked forever
                # and slides out of the 30-45 DTE window (observed: Sept-25
                # chain down to 28 DTE, never rolled). Parse the held expiry
                # and roll once its DTE falls below dte_min.
                if not need_scan:
                    for u in underlyings:
                        sel = selections.get(u) or {}
                        exp_str = sel.get("expiry")
                        if exp_str:
                            try:
                                exp_date = datetime.strptime(exp_str, "%Y-%m-%d").date()
                                if (exp_date - today).days < args.dte_min:
                                    need_scan = True
                                    print(json.dumps({"chain_roll": True, "underlying": u,
                                                      "expiry": exp_str,
                                                      "dte": (exp_date - today).days,
                                                      "reason": "DTE below dte_min"}),
                                          file=sys.stderr, flush=True)
                                    break
                            except ValueError:
                                continue

            if need_scan:
                # Only scan during pre-market or if market is about to open
                secs_to_open = seconds_until_market_open(now_et)
                if secs_to_open <= 300 or is_market_hours(now_et):
                    # Run chain scan
                    for u in underlyings:
                        result = discover_contracts(
                            ctx, u, args.dte_min, args.dte_max, args.delta_target,
                            args.bid_min, args.spread_cap, args.oi_min
                        )
                        if "error" in result:
                            print(json.dumps({"scan_error": result["error"], "underlying": u}),
                                  file=sys.stderr, flush=True)
                        else:
                            selections[u] = result
                            print(json.dumps({"scan_complete": True, "underlying": u,
                                            "expiry": result["expiry"],
                                            "call": result.get("call"),
                                            "put": result.get("put")}),
                                  file=sys.stderr, flush=True)

            # Build the active contract list from selections
            active_contracts = []
            for u, sel in selections.items():
                chain_code = sel.get("chain_code", u)
                for key in ("call", "put"):
                    if sel.get(key):
                        active_contracts.append({
                            "code": sel[key]["code"],
                            "underlying": u,
                            "chain_code": chain_code,
                        })

            if not active_contracts:
                # No contracts discovered yet — wait and retry
                time.sleep(30)
                continue

            # Check market hours (now also gates on is_trading_day via the calendar)
            if not is_market_hours(now_et):
                secs_to_next = seconds_until_next_market_open(now_et)
                print(json.dumps({"idle": True, "seconds_to_next_open": int(secs_to_next)}),
                      file=sys.stderr, flush=True)
                # Sleep in chunks for signal responsiveness, max 300s
                sleep_time = min(secs_to_next, 300)
                time.sleep(sleep_time)
                continue

            # Poll contracts
            polled = poll_contracts(ctx, active_contracts, args.interval)
            if not polled:
                # Session ended during polling — wait
                time.sleep(30)
                continue

            # Sleep for the poll interval (in chunks for signal responsiveness)
            remaining = args.interval
            while remaining > 0 and running[0]:
                chunk = min(remaining, 5)
                time.sleep(chunk)
                remaining -= chunk

    except Exception as e:
        print(json.dumps({"fatal_error": str(e)}), file=sys.stderr, flush=True)
        sys.exit(1)
    finally:
        safe_close(ctx)


if __name__ == "__main__":
    main()
