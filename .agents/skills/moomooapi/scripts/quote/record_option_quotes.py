#!/usr/bin/env python3
"""
Options tape recorder: poll option quotes and stream to stdout.

Usage:
    python3 record_option_quotes.py --contracts US.QQQ260919C00530000,US.QQQ260919P00520000 --interval 5 --duration 3600

Output: JSON lines to stdout, one per poll:
    {"timestamp_ms": 1234567890000, "contract": "US.QQQ260919C00530000", 
     "bid": 5.2, "ask": 5.3, "last": 5.25, "volume": 100, "oi": 2500,
     "iv": 0.28, "delta": 0.45, "gamma": 0.003, "theta": -0.15,
     "underlying_price": 530.0}
"""

import sys
import json
import time
import argparse
from moomoo import *

def poll_quotes(ctx, contracts, interval, duration):
    """Poll quotes for contracts and stream to stdout."""
    # Subscribe to QUOTE (request/response mode, no push)
    ret, err = ctx.subscribe(contracts, [SubType.QUOTE], subscribe_push=False)
    if ret != RET_OK:
        print(json.dumps({"error": f"Subscription failed: {err}"}), flush=True)
        return
    
    start_time = time.time()
    while time.time() - start_time < duration:
        # Poll quotes
        ret, data = ctx.get_stock_quote(contracts)
        if ret == RET_OK:
            for _, row in data.iterrows():
                quote = {
                    "timestamp_ms": int(time.time() * 1000),
                    "contract": row.get("code", ""),
                    "bid": float(row.get("bid_price", 0)),
                    "ask": float(row.get("ask_price", 0)),
                    "last": float(row.get("last_price", 0)),
                    "volume": int(row.get("volume", 0)),
                    "oi": int(row.get("open_interest", 0)),
                    "iv": float(row.get("implied_volatility", 0)),
                    "delta": float(row.get("delta", 0)),
                    "gamma": float(row.get("gamma", 0)),
                    "theta": float(row.get("theta", 0)),
                    "underlying_price": float(row.get("stock_price", 0)),
                }
                print(json.dumps(quote), flush=True)
        
        time.sleep(interval)

def main():
    parser = argparse.ArgumentParser(description="Poll option quotes and stream to stdout")
    parser.add_argument("--contracts", required=True, help="Comma-separated contract codes")
    parser.add_argument("--interval", type=float, default=5.0, help="Poll interval in seconds")
    parser.add_argument("--duration", type=int, default=3600, help="Recording duration in seconds")
    args = parser.parse_args()
    
    contracts = [c.strip() for c in args.contracts.split(",")]
    
    # Connect to OpenD
    ctx = OpenQuoteContext(host="127.0.0.1", port=11111)
    
    try:
        poll_quotes(ctx, contracts, args.interval, args.duration)
    finally:
        ctx.close()

if __name__ == "__main__":
    main()
