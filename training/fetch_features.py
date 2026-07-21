#!/usr/bin/env python3
"""
Fetches real Binance Futures data (funding rates, futures klines for basis)
and computes order-flow imbalance from spot kline taker volume.

Usage:
    from fetch_features import fetch_and_merge_features
    df = fetch_and_merge_features(data_dir, spot_columns)

Or run standalone to produce a merged CSV:
    python fetch_features.py --data-dir /path/to/BTCUSDT_1H --output merged.csv
"""
import json
import os
import sys
import time
import urllib.request
import numpy as np
import pandas as pd
from typing import List


FUTURES_REST = "https://fapi.binance.com"
SPOT_REST = "https://api.binance.com"


def _fetch_json(url: str, max_retries: int = 3) -> list:
    """Fetch JSON from URL with retry."""
    for attempt in range(max_retries):
        try:
            req = urllib.request.Request(url)
            with urllib.request.urlopen(req, timeout=30) as resp:
                return json.loads(resp.read().decode())
        except Exception as e:
            if attempt < max_retries - 1:
                print(f"  retry {attempt+1}/{max_retries}: {e}")
                time.sleep(1)
            else:
                raise


def fetch_funding_rates(symbol: str, start_ms: int, end_ms: int) -> pd.DataFrame:
    """
    Fetch historical funding rates from Binance Futures.
    Returns DataFrame with columns: [funding_time, funding_rate].
    """
    all_records = []
    cursor = start_ms
    while cursor < end_ms:
        url = (f"{FUTURES_REST}/fapi/v1/fundingRate?symbol={symbol}"
               f"&startTime={cursor}&endTime={end_ms}&limit=1000")
        data = _fetch_json(url)
        if not data:
            break
        for r in data:
            all_records.append({
                'funding_time': int(r['fundingTime']),
                'funding_rate': float(r['fundingRate']),
            })
        cursor = int(data[-1]['fundingTime']) + 1
        if len(data) < 1000:
            break
        time.sleep(0.1)  # rate limit

    if not all_records:
        return pd.DataFrame(columns=['funding_time', 'funding_rate'])

    df = pd.DataFrame(all_records)
    df['funding_time'] = df['funding_time'].astype('int64')
    return df.sort_values('funding_time').reset_index(drop=True)


def fetch_futures_klines(symbol: str, interval: str, start_ms: int, end_ms: int) -> pd.DataFrame:
    """
    Fetch historical futures klines from Binance Futures.
    Returns DataFrame with standard kline columns.
    """
    all_records = []
    cursor = start_ms
    while cursor < end_ms:
        url = (f"{FUTURES_REST}/fapi/v1/klines?symbol={symbol}"
               f"&interval={interval}&startTime={cursor}&endTime={end_ms}&limit=1000")
        data = _fetch_json(url)
        if not data:
            break
        for r in data:
            all_records.append({
                'open_time': int(r[0]),
                'open': float(r[1]),
                'high': float(r[2]),
                'low': float(r[3]),
                'close': float(r[4]),
                'volume': float(r[5]),
                'close_time': int(r[6]),
            })
        cursor = int(data[-1][0]) + 1
        if len(data) < 1000:
            break
        time.sleep(0.1)

    if not all_records:
        return pd.DataFrame(columns=['open_time', 'open', 'high', 'low', 'close', 'volume', 'close_time'])

    df = pd.DataFrame(all_records)
    df['open_time'] = df['open_time'].astype('int64')
    return df.sort_values('open_time').reset_index(drop=True)


def compute_ob_imbalance(df: pd.DataFrame) -> pd.Series:
    """
    Order-flow imbalance from taker buy volume vs total volume.
    ob_imbalance = (2 * taker_buy_base / volume) - 1
    Range: [-1, 1] where +1 = all buys, -1 = all sells, 0 = balanced.
    """
    vol = df['volume'].replace(0, np.nan)
    ob = (2.0 * df['taker_buy_base'] / vol) - 1.0
    return ob.fillna(0.0).clip(-1.0, 1.0)


def compute_basis_z(spot_close: pd.Series, futures_close: pd.Series, window: int = 72) -> pd.Series:
    """
    Basis = (futures_close - spot_close) / spot_close.
    Z-scored over rolling window to normalize.
    """
    basis = (futures_close - spot_close) / spot_close
    rolling_mean = basis.rolling(window=window, min_periods=10).mean()
    rolling_std = basis.rolling(window=window, min_periods=10).std().replace(0, np.nan)
    z = (basis - rolling_mean) / rolling_std
    return z.fillna(0.0).clip(-5.0, 5.0)


def fetch_and_merge_features(
    data_dir: str,
    spot_columns: List[str],
    symbol: str = "BTCUSDT",
) -> pd.DataFrame:
    """
    Load spot kline CSVs, fetch futures funding rates + futures klines,
    compute funding_rate, basis_z, ob_imbalance, and merge into one DataFrame.

    Returns DataFrame with columns:
        open_time, open, high, low, close, volume, close_time, quote_vol, trades,
        taker_buy_base, taker_buy_quote, funding_rate, basis_z, ob_imbalance
    """
    import glob

    # ── 1. Load spot klines ──────────────────────────────────────────────────
    all_files = sorted(glob.glob(os.path.join(data_dir, "*.csv")))
    df_list = [pd.read_csv(f, names=spot_columns) for f in all_files]
    df = pd.concat(df_list, ignore_index=True)
    df['open_time'] = df['open_time'].astype('int64')
    df = df.sort_values('open_time').reset_index(drop=True)
    print(f"Loaded {len(df)} spot candles from {len(all_files)} files")

    # ── 2. Compute ob_imbalance from taker volume (already in CSV) ───────────
    df['ob_imbalance'] = compute_ob_imbalance(df)
    print(f"ob_imbalance: mean={df['ob_imbalance'].mean():.4f} std={df['ob_imbalance'].std():.4f}")

    # ── 3. Fetch funding rates from Binance Futures ──────────────────────────
    start_ms = int(df['open_time'].iloc[0]) * 1000
    end_ms = int(df['open_time'].iloc[-1]) * 1000 + 3600_000
    print(f"Fetching funding rates {symbol} from {start_ms} to {end_ms}...")
    funding_df = fetch_funding_rates(symbol, start_ms, end_ms)

    if len(funding_df) > 0:
        # Forward-fill funding rate from 8h to 1h.
        funding_df['funding_hour'] = funding_df['funding_time'] // 3_600_000
        df['funding_hour'] = df['open_time'] // 3600
        # Merge: each spot bar gets the most recent funding rate.
        merged = df.merge(funding_df[['funding_hour', 'funding_rate']], on='funding_hour', how='left')
        merged['funding_rate'] = merged['funding_rate'].ffill().fillna(0.0)
        df['funding_rate'] = merged['funding_rate']
        df = df.drop(columns=['funding_hour'])
        print(f"funding_rate: mean={df['funding_rate'].mean():.6f} std={df['funding_rate'].std():.6f}")
    else:
        print("WARNING: no funding rate data fetched — using 0.0")
        df['funding_rate'] = 0.0

    # ── 4. Fetch futures klines for basis calculation ────────────────────────
    print(f"Fetching futures klines {symbol} 1h...")
    fut_df = fetch_futures_klines(symbol, "1h", start_ms, end_ms)

    if len(fut_df) > 0:
        # Align by open_time (both are 1h bars starting at same hour).
        df = df.merge(fut_df[['open_time', 'close']].rename(columns={'close': 'fut_close'}),
                      on='open_time', how='left')
        df['fut_close'] = df['fut_close'].ffill().fillna(df['close'])
        df['basis_z'] = compute_basis_z(df['close'], df['fut_close'], window=72)
        df = df.drop(columns=['fut_close'])
        print(f"basis_z: mean={df['basis_z'].mean():.4f} std={df['basis_z'].std():.4f}")
    else:
        print("WARNING: no futures klines fetched — basis_z using 0.0")
        df['basis_z'] = 0.0

    # ── 5. Fill any remaining NaNs ────────────────────────────────────────────
    for col in ['funding_rate', 'basis_z', 'ob_imbalance']:
        df[col] = df[col].replace([np.inf, -np.inf], 0.0).fillna(0.0)

    print(f"Final DataFrame: {df.shape} with columns: {[c for c in df.columns if c != 'ignore']}")
    return df


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--data-dir", default=os.environ.get('DATA_DIR', '/content/drive/MyDrive/QuantData/BTCUSDT_1H'))
    parser.add_argument("--output", default="merged_features.csv")
    args = parser.parse_args()

    columns = ['open_time', 'open', 'high', 'low', 'close', 'volume', 'close_time',
               'quote_vol', 'trades', 'taker_buy_base', 'taker_buy_quote', 'ignore']
    df = fetch_and_merge_features(args.data_dir, columns)
    df.to_csv(args.output, index=False)
    print(f"Saved merged features to {args.output}")