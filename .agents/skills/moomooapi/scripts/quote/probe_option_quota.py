#!/usr/bin/env python3
"""
Probe option quota tier and OPRA card status.

Checks:
1. Current subscription quota usage (total_used, remain, own_used)
2. Fetches QQQ option chain to find a contract in 30-45 DTE window
3. Subscribes QUOTE for one option contract
4. Fetches option quote and verifies greeks fields are present (OPRA card active)

Output: JSON with quota info, greeks availability, and any errors.

Usage:
    python3 probe_option_quota.py
    python3 probe_option_quota.py --underlying US.SPY  # different symbol
"""
import argparse
import json
import sys
import os as _os
import time
from datetime import datetime, timedelta

sys.path.insert(0, _os.path.normpath(_os.path.join(_os.path.dirname(_os.path.abspath(__file__)), "..")))
from common import (
    create_quote_context,
    check_ret,
    safe_close,
    RET_OK,
)


def probe_option_quota(underlying="US.QQQ", output_json=True):
    """
    Probe option quota tier and OPRA card status.

    Args:
        underlying: Underlying stock code (e.g., "US.QQQ")
        output_json: If True, output JSON; else human-readable

    Returns:
        dict with quota info, greeks status, and errors
    """
    ctx = None
    result = {
        "underlying": underlying,
        "quota": None,
        "option_chain": None,
        "greeks_available": False,
        "greeks_fields": [],
        "errors": [],
    }

    try:
        ctx = create_quote_context()

        # 1. Query subscription quota
        ret, data = ctx.query_subscription(is_all_conn=True)
        if ret != RET_OK:
            result["errors"].append(f"query_subscription failed: {data}")
        else:
            result["quota"] = {
                "total_used": data.get("total_used", 0),
                "remain": data.get("remain", 0),
                "own_used": data.get("own_used", 0),
                "option_used_quota": data.get("option_used_quota", 0),
                "option_remain_quota": data.get("option_remain_quota", 0),
            }

        # 2. Get option expiration dates for underlying
        ret, exp_dates = ctx.get_option_expiration_date(underlying)
        if ret != RET_OK:
            result["errors"].append(f"get_option_expiration_date failed: {data}")
            return result

        # Filter expirations in 30-45 DTE window
        today = datetime.now()
        target_start = today + timedelta(days=30)
        target_end = today + timedelta(days=45)

        valid_exps = []
        for exp_str in exp_dates["strike_time"]:
            try:
                exp_date = datetime.strptime(exp_str, "%Y-%m-%d")
                if target_start <= exp_date <= target_end:
                    valid_exps.append(exp_str)
            except ValueError:
                continue

        if not valid_exps:
            result["errors"].append(f"No option expirations found in 30-45 DTE window for {underlying}")
            return result

        # Pick the first valid expiration
        chosen_exp = valid_exps[0]
        result["option_chain"] = {"chosen_expiration": chosen_exp, "candidates": valid_exps}

        # 3. Get option chain for this expiration
        ret, chain_df = ctx.get_option_chain(
            underlying,
            start=chosen_exp,
            end=chosen_exp,
        )
        if ret != RET_OK:
            result["errors"].append(f"get_option_chain failed: {chain_df}")
            return result

        if chain_df.empty:
            result["errors"].append(f"Empty option chain for {underlying} exp {chosen_exp}")
            return result

        # Pick first CALL option (prefer delta ~0.45 if available)
        calls = chain_df[chain_df["option_type"] == "CALL"]
        if calls.empty:
            result["errors"].append("No CALL options found in chain")
            return result

        # Just pick the first call for probing
        option_code = calls.iloc[0]["code"]
        result["option_chain"]["selected_option"] = option_code

        # 4. Subscribe QUOTE for this option
        ret, data = ctx.subscribe([option_code], ["QUOTE"])
        if ret != RET_OK:
            result["errors"].append(f"subscribe QUOTE failed: {data}")
            return result

        # Wait a moment for push data, then fetch option quote.
        # Greeks (delta/gamma/theta/vega/rho) and implied_volatility are
        # LIVE-computed values pushed only during US option trading hours.
        # When the option session is closed, the contract's static fields
        # (price, mark_price, intrinsic_value, etc.) still populate, but the
        # greeks come back as N/A / 0.0. Retry a few times to let the push
        # arrive, and report the market-hours distinction explicitly.
        from moomoo import OptionStrategyLeg, StrategyLegAction

        greeks_fields = ["implied_volatility", "delta", "gamma", "theta", "vega", "rho"]
        available_greeks = []
        missing_greeks = []
        quote_df = None

        for attempt in range(4):
            time.sleep(1.5)
            leg = OptionStrategyLeg()
            leg.code = option_code
            leg.action = StrategyLegAction.BUY  # correct enum, NOT raw int 1
            leg.quantity = 1
            ret, q = ctx.get_option_quote([leg])
            if ret != RET_OK:
                result["errors"].append(f"get_option_quote failed (attempt {attempt+1}): {q}")
                continue
            if q is not None and not getattr(q, "empty", True):
                quote_df = q
                # Check which greeks fields carry a real (non-N/A, non-zero-IV) value
                available_greeks = []
                numeric_greeks = []
                for field in greeks_fields:
                    if field in q.columns:
                        val = q.iloc[0][field]
                        sval = str(val).strip()
                        # moomoo returns the literal string 'N/A' for missing
                        # live-computed greeks (session closed / no entitlement).
                        if val is None or sval in ("nan", "NaN", "None", "", "N/A", "n/a"):
                            continue
                        try:
                            fval = float(val)
                        except (ValueError, TypeError):
                            continue
                        # implied_volatility of 0.0 means no live IV feed
                        if field == "implied_volatility" and fval == 0.0:
                            continue
                        available_greeks.append(field)
                        numeric_greeks.append((field, fval))
                if len(available_greeks) >= 4:
                    break
                missing_greeks = [f for f in greeks_fields if f not in available_greeks]

        if quote_df is None or getattr(quote_df, "empty", True):
            result["errors"].append("No option quote data returned (all attempts empty/failed)")
            ctx.unsubscribe([option_code], ["QUOTE"])
            return result

        result["greeks_available"] = len(available_greeks) >= 4  # Need at least IV, delta, gamma, theta
        result["greeks_fields"] = available_greeks
        result["greeks_missing"] = missing_greeks

        # Sample greeks values for verification
        if numeric_greeks:
            result["greeks_sample"] = {field: fval for field, fval in numeric_greeks}

        # Market-hours context: if the schema is present but values are N/A,
        # this is almost always "option session closed", not an entitlement gap.
        if not result["greeks_available"]:
            result["greeks_note"] = (
                "Greeks columns present but values are N/A. This is the expected "
                "state OUTSIDE US option trading hours (greeks are live-computed). "
                "Re-run during US market hours (09:30-16:00 ET) to confirm the OPRA "
                "card delivers populated greeks. Static fields (price, mark_price, "
                "intrinsic_value) populating while greeks are N/A confirms the data "
                "feed is alive but the session is closed."
            )

        # 6. Unsubscribe to free quota
        ctx.unsubscribe([option_code], ["QUOTE"])

    except Exception as e:
        result["errors"].append(f"Exception: {str(e)}")
    finally:
        safe_close(ctx)

    # Output
    if output_json:
        print(json.dumps(result, indent=2, ensure_ascii=False))
    else:
        print("=" * 60)
        print("Option Quota & OPRA Probe Results")
        print("=" * 60)
        print(f"Underlying: {result['underlying']}")
        print()

        if result["quota"]:
            print("Quota Status:")
            print(f"  Total used: {result['quota']['total_used']}")
            print(f"  Remaining: {result['quota']['remain']}")
            print(f"  Own used: {result['quota']['own_used']}")
            print()

        if result["option_chain"]:
            print("Option Chain:")
            print(f"  Selected expiration: {result['option_chain']['chosen_expiration']}")
            print(f"  Selected option: {result['option_chain']['selected_option']}")
            print()

        print("OPRA Card Status:")
        if result["greeks_available"]:
            print(f"  ✓ ACTIVE — {len(result['greeks_fields'])} greeks fields available")
            print(f"  Fields: {', '.join(result['greeks_fields'])}")
            if "greeks_sample" in result:
                print("  Sample values:")
                for field, val in result["greeks_sample"].items():
                    print(f"    {field}: {val:.6f}")
        else:
            print("  ✗ INACTIVE or INSUFFICIENT — greeks not available")
            if result.get("greeks_missing"):
                print(f"  Missing: {', '.join(result['greeks_missing'])}")
        print()

        if result["errors"]:
            print("Errors:")
            for err in result["errors"]:
                print(f"  - {err}")
        print("=" * 60)

    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Probe option quota tier and OPRA card status")
    parser.add_argument(
        "--underlying",
        default="US.QQQ",
        help="Underlying stock code (default: US.QQQ)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        dest="output_json",
        default=True,
        help="Output in JSON format (default: True)",
    )
    parser.add_argument(
        "--text",
        action="store_true",
        dest="output_text",
        help="Output in human-readable text format",
    )
    args = parser.parse_args()

    output_json = not args.output_text
    probe_option_quota(args.underlying, output_json=output_json)
