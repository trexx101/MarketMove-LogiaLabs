#!/usr/bin/env python3
"""
Fetches Binance Futures data from data.binance.vision ZIP downloads (no geo-block).
Funding rates + futures klines. Also computes ob_imbalance from CSV taker volume.
"""
import json, os, time, urllib.request, zipfile, io, glob
import numpy as np
import pandas as pd
from datetime import datetime, timedelta
from typing import List

VISION = "https://data.binance.vision/data/futures/um/daily"


def _fetch_zip_url(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.read()


def _read_csv_from_zip(raw: bytes) -> pd.DataFrame:
    with zipfile.ZipFile(io.BytesIO(raw)) as z:
        return pd.read_csv(z.open(z.namelist()[0]), header=None)


def fetch_binance_vision_funding(symbol: str, start_date: str, end_date: str) -> pd.DataFrame:
    """Monthly funding rate ZIPs from data.binance.vision.

    CSV format: calc_time (ms), funding_interval_hours (8), last_funding_rate (decimal).
    Records every 8h — forward-filled to hourly.
    """
    all_months = []
    start = pd.Timestamp(start_date)
    end = pd.Timestamp(end_date)
    cur = start.replace(day=1)
    while cur <= end:
        ms = cur.strftime('%Y-%m')
        url = f"{VISION.replace('daily', 'monthly')}/fundingRate/{symbol}/{symbol}-fundingRate-{ms}.zip"
        try:
            raw = _fetch_zip_url(url)
            with zipfile.ZipFile(io.BytesIO(raw)) as z:
                df = pd.read_csv(z.open(z.namelist()[0]), header=0)
            df['calc_time'] = pd.to_numeric(df['calc_time'], errors='coerce')
            df['last_funding_rate'] = pd.to_numeric(df['last_funding_rate'], errors='coerce')
            df = df.dropna(subset=['calc_time', 'last_funding_rate'])
            df['calc_time'] = df['calc_time'].astype('int64')
            # Binance records funding every 8h → resample to hourly forward-fill
            df['datetime'] = pd.to_datetime(df['calc_time'], unit='ms')
            df = df.set_index('datetime')[['last_funding_rate']].sort_index()
            hourly = df.resample('1h').ffill()
            hourly.index = (hourly.index.astype('int64') // 10**6).astype('int64')
            hourly = hourly.rename(columns={'last_funding_rate': 'funding_rate'})
            all_months.append(hourly)
        except Exception as e:
            print(f"  skip {ms}: {e}")
        cur = (cur + pd.offsets.MonthEnd(1)) + pd.Timedelta(days=1)
        time.sleep(0.1)
    if not all_months:
        return pd.DataFrame()
    full = pd.concat(all_months).sort_index()
    return full.reset_index().rename(columns={'index': 'funding_time_ms'})[['funding_time_ms', 'funding_rate']]


def fetch_binance_vision_klines(symbol: str, interval: str, start_date: str, end_date: str) -> pd.DataFrame:
    """Daily futures kline ZIPs from data.binance.vision."""
    all_days = []
    cur = datetime.strptime(start_date, '%Y-%m-%d')
    end = datetime.strptime(end_date, '%Y-%m-%d')
    while cur <= end:
        ds = cur.strftime('%Y-%m-%d')
        url = f"{VISION}/klines/{symbol}/{interval}/{symbol}-{interval}-{ds}.zip"
        try:
            raw = _fetch_zip_url(url)
            df = _read_csv_from_zip(raw)
            df.columns = ['open_time', 'open', 'high', 'low', 'close', 'volume', 'close_time',
                          'quote_vol', 'trades', 'taker_buy_base', 'taker_buy_quote', 'ignore']
            all_days.append(df[['open_time', 'close']].astype({'open_time': 'int64', 'close': 'float64'}))
        except Exception:
            pass
        cur += timedelta(days=1)
        time.sleep(0.05)
    return pd.concat(all_days, ignore_index=True).drop_duplicates('open_time').sort_values('open_time').reset_index(drop=True) if all_days else pd.DataFrame()


def compute_ob_imbalance(df: pd.DataFrame) -> pd.Series:
    vol = df['volume'].replace(0, np.nan)
    return ((2.0 * df['taker_buy_base'] / vol) - 1.0).fillna(0.0).clip(-1.0, 1.0)


def compute_basis_z(sc: pd.Series, fc: pd.Series, window: int = 72) -> pd.Series:
    b = (fc - sc) / sc
    m = b.rolling(window, min_periods=10).mean()
    s = b.rolling(window, min_periods=10).std().replace(0, np.nan)
    return ((b - m) / s).fillna(0.0).clip(-5.0, 5.0)


def ms_to_date(ms: int) -> str:
    """Convert epoch timestamp (in ms, µs, or seconds) to date string."""
    # Handle auto-detection: if > 1e15 it's microseconds, if > 1e12 it's milliseconds.
    ts = ms
    if ts > 1_000_000_000_000_000:  # > 1e15 → microseconds
        ts //= 1000
    if ts > 1_000_000_000_000:     # > 1e12 → milliseconds
        ts //= 1000
    return datetime.utcfromtimestamp(ts).strftime('%Y-%m-%d')


def fetch_and_merge_features(data_dir: str, spot_columns: List[str], symbol: str = "BTCUSDT") -> pd.DataFrame:
    all_files = sorted(glob.glob(os.path.join(data_dir, "*.csv")))
    df_list = [pd.read_csv(f, names=spot_columns) for f in all_files]
    df = pd.concat(df_list, ignore_index=True)
    df['open_time'] = df['open_time'].astype('int64')
    df = df.sort_values('open_time').reset_index(drop=True)
    print(f"Loaded {len(df)} spot candles from {len(all_files)} files")

    # ob_imbalance from CSV taker volume
    df['ob_imbalance'] = compute_ob_imbalance(df)
    print(f"ob_imbalance: mean={df['ob_imbalance'].mean():.4f} std={df['ob_imbalance'].std():.4f}")

    # Funding rates from Binance Vision ZIPs
    start_ms, end_ms = int(df['open_time'].iloc[0]), int(df['open_time'].iloc[-1]) + 3_600_000
    print(f"Fetching funding rates {symbol} from data.binance.vision...")
    funding_df = fetch_binance_vision_funding(symbol, ms_to_date(start_ms), ms_to_date(end_ms))
    if len(funding_df) > 0:
        funding_df['funding_hour'] = funding_df['funding_time_ms'] // 3_600_000
        df['funding_hour'] = df['open_time'] // 3_600_000
        merged = df.merge(funding_df[['funding_hour', 'funding_rate']], on='funding_hour', how='left')
        df['funding_rate'] = merged['funding_rate'].ffill().fillna(0.0)
        df = df.drop(columns=['funding_hour'])
        print(f"funding_rate: mean={df['funding_rate'].mean():.6f} std={df['funding_rate'].std():.6f}")
    else:
        print("WARNING: no funding data — using 0.0"); df['funding_rate'] = 0.0

    # Futures klines for basis_z
    print("Fetching futures klines from data.binance.vision...")
    fut_df = fetch_binance_vision_klines(symbol, "1h", ms_to_date(start_ms), ms_to_date(end_ms))
    if len(fut_df) > 0:
        df = df.merge(fut_df.rename(columns={'close': 'fut_close'}), on='open_time', how='left')
        df['fut_close'] = df['fut_close'].ffill().fillna(df['close'])
        df['basis_z'] = compute_basis_z(df['close'], df['fut_close'], window=72)
        df = df.drop(columns=['fut_close'])
        print(f"basis_z: mean={df['basis_z'].mean():.4f} std={df['basis_z'].std():.4f}")
    else:
        print("WARNING: no futures klines — basis_z using 0.0"); df['basis_z'] = 0.0

    for col in ['funding_rate', 'basis_z', 'ob_imbalance']:
        df[col] = df[col].replace([np.inf, -np.inf], 0.0).fillna(0.0)

    print(f"Final: {df.shape}")
    return df


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--data-dir", default=os.environ.get('DATA_DIR', '/content/drive/MyDrive/QuantData/BTCUSDT_1H'))
    parser.add_argument("--output", default="merged_features.csv")
    args = parser.parse_args()
    cols = ['open_time', 'open', 'high', 'low', 'close', 'volume', 'close_time',
            'quote_vol', 'trades', 'taker_buy_base', 'taker_buy_quote', 'ignore']
    df = fetch_and_merge_features(args.data_dir, cols)
    df.to_csv(args.output, index=False)
    print(f"Saved {args.output}")