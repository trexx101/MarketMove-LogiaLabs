#!/usr/bin/env python3
"""Backfill {symbol} historical predictions using trained {symbol} models.

Loads the TCN + LightGBM ensemble, fetches daily candles from the live
SQLite DB, computes 8-dim features, normalizes with norm_stats, generates
predictions for each day with enough history, and inserts them into the
equity_predictions table.

To adapt for a new symbol:
  1. Change MODELS_DIR to point to the model bundle directory
  2. Change the model filenames in load_models()
  3. Change SYMBOL and SOURCE constants
  4. Ensure norm_stats file exists at MODELS_DIR/norm_stats_{symbol}_v1.json

Run from the MarketMoves workspace root with the inference venv python:
    python3 /tmp/backfill_predictions.py
"""

import json
import os
import sys
import sqlite3
import pickle
from pathlib import Path

import numpy as np
import torch

# ── Configuration ──────────────────────────────────────────────────────────
MODELS_DIR = Path("/tmp/NVDA_models")  # Copy models here first with sudo
DB_PATH = "/tmp/candles_nvda.db"       # Copy DB here first with sudo
FEATURE_WINDOW_SIZE = 126              # matches SEQ_LEN from notebook
SYMBOL = "NVDA"
SOURCE = "nvda_tcn_v1"

# ── Feature helpers (from training/equities_features.py) ────────────────────

def rolling_sma(values, window):
    n = len(values)
    out = np.full(n, np.nan)
    if n < window:
        return out
    cumsum = np.cumsum(values)
    out[window - 1] = cumsum[window - 1] / window
    out[window:] = (cumsum[window:] - cumsum[:-window]) / window
    return out

def wilder_smooth(values, period):
    n = len(values)
    out = np.full(n, np.nan)
    if n < period:
        return out
    out[period - 1] = np.mean(values[:period])
    for i in range(period, n):
        out[i] = (out[i - 1] * (period - 1) + values[i]) / period
    return out

def adx_14(highs, lows, closes):
    n = len(closes)
    out = np.zeros(n)
    if n < 28:
        return out
    plus_dm = np.zeros(n)
    minus_dm = np.zeros(n)
    tr = np.zeros(n)
    for i in range(1, n):
        up_move = highs[i] - highs[i - 1]
        down_move = lows[i - 1] - lows[i]
        plus_dm[i] = up_move if (up_move > down_move and up_move > 0) else 0.0
        minus_dm[i] = down_move if (down_move > up_move and down_move > 0) else 0.0
        tr[i] = max(highs[i] - lows[i], abs(highs[i] - closes[i - 1]), abs(lows[i] - closes[i - 1]))
    period = 14
    atr = wilder_smooth(tr, period)
    smoothed_plus = wilder_smooth(plus_dm, period)
    smoothed_minus = wilder_smooth(minus_dm, period)
    dx = np.zeros(n)
    for i in range(period, n):
        if atr[i] > 0:
            pdi = 100.0 * smoothed_plus[i] / atr[i]
            mdi = 100.0 * smoothed_minus[i] / atr[i]
            s = pdi + mdi
            if s > 0:
                dx[i] = 100.0 * abs(pdi - mdi) / s
    adx_vals = wilder_smooth(dx, period)
    return np.where(np.isfinite(adx_vals), adx_vals, 0.0)

def rsi_14(closes):
    n = len(closes)
    out = np.full(n, 50.0)
    if n < 15:
        return out
    period = 14
    gains = np.zeros(n)
    losses = np.zeros(n)
    for i in range(1, n):
        diff = closes[i] - closes[i - 1]
        if diff >= 0:
            gains[i] = diff
        else:
            losses[i] = -diff
    avg_gain = np.mean(gains[1:period + 1])
    avg_loss = np.mean(losses[1:period + 1])
    def rsi_val(ag, al):
        if al < 1e-12:
            return 100.0
        return 100.0 - 100.0 / (1.0 + ag / al)
    out[period] = rsi_val(avg_gain, avg_loss)
    for i in range(period + 1, n):
        avg_gain = (avg_gain * (period - 1) + gains[i]) / period
        avg_loss = (avg_loss * (period - 1) + losses[i]) / period
        out[i] = rsi_val(avg_gain, avg_loss)
    return out

def vix_regime(vix):
    out = np.zeros(len(vix))
    for i, v in enumerate(vix):
        if v > 25:
            out[i] = 2.0
        elif v >= 18:
            out[i] = 1.0
    return out

def rvol_20d(volumes):
    n = len(volumes)
    out = np.zeros(n)
    if n < 21:
        return out
    for i in range(20, n):
        avg = np.mean(volumes[i - 20:i])
        out[i] = volumes[i] / avg if avg > 0 else 0.0
    return out

def gap_pct(opens, closes):
    n = len(opens)
    out = np.zeros(n)
    for i in range(1, n):
        if closes[i - 1] > 0:
            out[i] = (opens[i] - closes[i - 1]) / closes[i - 1]
    return out

def drawdown_from_high(closes, window=50):
    n = len(closes)
    out = np.zeros(n)
    if n < window + 1:
        return out
    for i in range(window, n):
        high = np.max(closes[i - window:i + 1])
        if high > 0:
            out[i] = (closes[i] - high) / high
    return out

def compute_features(opens, highs, lows, closes, volumes, vix=None, tlt=None):
    n = len(closes)
    sma50 = rolling_sma(closes, 50)
    adx = adx_14(highs, lows, closes)
    rsi = rsi_14(closes)
    vix_feat = vix_regime(vix) if vix is not None else np.zeros(n)
    tlt_corr = np.zeros(n)
    if tlt is not None and len(tlt) == n:
        for i in range(20, n):
            a = closes[i - 20:i]
            b = tlt[i - 20:i]
            if np.std(a) > 1e-12 and np.std(b) > 1e-12:
                tlt_corr[i] = np.corrcoef(a, b)[0, 1]
    rvol = rvol_20d(volumes)
    gaps = gap_pct(opens, closes)
    dd = drawdown_from_high(closes, 50)
    rows = []
    for i in range(n):
        if i >= 20 and np.isfinite(sma50[i]) and np.isfinite(sma50[i - 20]) and sma50[i - 20] > 0:
            ts = np.log(sma50[i] / sma50[i - 20])
        else:
            ts = 0.0
        rows.append([
            float(ts), float(adx[i]), float(rsi[i]),
            float(vix_feat[i]), float(tlt_corr[i]),
            float(rvol[i]), float(gaps[i]), float(dd[i]),
        ])
    return rows

# ── Normalization ──────────────────────────────────────────────────────────

def normalize(features, norm_stats):
    medians = np.array([
        norm_stats["medians"]["trend_slope"],
        norm_stats["medians"]["trend_adx"],
        norm_stats["medians"]["rsi_14"],
        norm_stats["medians"]["vix_regime"],
        norm_stats["medians"]["tlt_corr_20d"],
        norm_stats["medians"]["rvol_20d"],
        norm_stats["medians"]["gap_pct"],
        norm_stats["medians"]["drawdown_from_50d_high"],
    ])
    mads = np.array([
        max(norm_stats["mads"]["trend_slope"], 1e-8),
        max(norm_stats["mads"]["trend_adx"], 1e-8),
        max(norm_stats["mads"]["rsi_14"], 1e-8),
        max(norm_stats["mads"]["vix_regime"], 1e-8),
        max(norm_stats["mads"]["tlt_corr_20d"], 1e-8),
        max(norm_stats["mads"]["rvol_20d"], 1e-8),
        max(norm_stats["mads"]["gap_pct"], 1e-8),
        max(norm_stats["mads"]["drawdown_from_50d_high"], 1e-8),
    ])
    arr = np.asarray(features, dtype=np.float64)
    return (arr - medians) / mads

def compute_atr_ratio(highs, lows, closes, period=14):
    n = len(closes)
    if n < period + 1:
        return 0.005
    tr = np.zeros(n)
    for i in range(1, n):
        tr[i] = max(highs[i] - lows[i], abs(highs[i] - closes[i - 1]), abs(lows[i] - closes[i - 1]))
    atr = np.mean(tr[-period:])
    return atr / closes[-1] if closes[-1] > 0 else 0.005

# ── Model loading ──────────────────────────────────────────────────────────

# Import QqqTCN from inference module (must use absolute path from workspace)
sys.path.insert(0, "/home/ubuntu/projects/MarketMoves/inference")
from equity_model import QqqTCN, EquityEnsemble

def load_models():
    tcn = QqqTCN(in_dim=8, hidden_dim=64, dropout=0.1)
    tcn.load_state_dict(torch.load(
        str(MODELS_DIR / f"{SYMBOL.lower()}_tcn_v1.pt"),
        map_location="cpu", weights_only=True))
    tcn.eval()
    with open(MODELS_DIR / f"{SYMBOL.lower()}_lgbm_h1_v1.pkl", "rb") as f:
        lgbm_h1 = pickle.load(f)
    with open(MODELS_DIR / f"{SYMBOL.lower()}_lgbm_h5_v1.pkl", "rb") as f:
        lgbm_h5 = pickle.load(f)
    with open(MODELS_DIR / f"{SYMBOL.lower()}_lgbm_h21_v1.pkl", "rb") as f:
        lgbm_h21 = pickle.load(f)
    return EquityEnsemble(tcn, lgbm_h1, lgbm_h5, lgbm_h21, tcn_weight=0.5, lgbm_weight=0.5)

# ── Main ───────────────────────────────────────────────────────────────────

def main():
    print(f"Loading {SYMBOL} model ensemble...")
    ensemble = load_models()

    print("Loading norm_stats...")
    with open(MODELS_DIR / f"norm_stats_{SYMBOL.lower()}_v1.json", "r") as f:
        norm_stats = json.load(f)

    print("Connecting to DB...")
    conn = sqlite3.connect(DB_PATH)
    c = conn.cursor()

    # Fetch candles
    c.execute("""
        SELECT ts, open, high, low, close, volume
        FROM equity_candles
        WHERE symbol = ?
        ORDER BY ts ASC
    """, (SYMBOL,))
    rows = c.fetchall()
    print(f"Fetched {len(rows)} {SYMBOL} candles")

    if len(rows) < FEATURE_WINDOW_SIZE + 60:
        print(f"Not enough candles ({len(rows)}), need {FEATURE_WINDOW_SIZE + 60}")
        conn.close()
        return 1

    timestamps = [r[0] for r in rows]
    opens = np.array([r[1] for r in rows])
    highs = np.array([r[2] for r in rows])
    lows = np.array([r[3] for r in rows])
    closes = np.array([r[4] for r in rows])
    volumes = np.array([r[5] for r in rows])

    # Fetch VIX candles
    c.execute("""
        SELECT ts, close FROM equity_candles
        WHERE symbol = '^VIX' AND ts >= ? AND ts <= ?
        ORDER BY ts ASC
    """, (timestamps[0], timestamps[-1]))
    vix_map = {r[0]: r[1] for r in c.fetchall()}
    vix = np.array([vix_map.get(ts, 0.0) for ts in timestamps])

    # Fetch TLT candles
    c.execute("""
        SELECT ts, close FROM equity_candles
        WHERE symbol = 'TLT' AND ts >= ? AND ts <= ?
        ORDER BY ts ASC
    """, (timestamps[0], timestamps[-1]))
    tlt_map = {r[0]: r[1] for r in c.fetchall()}
    tlt = np.array([tlt_map.get(ts, 0.0) for ts in timestamps])

    print("Computing features...")
    features = compute_features(opens, highs, lows, closes, volumes, vix, tlt)

    print("Normalizing...")
    normed = normalize(features, norm_stats)

    print("Generating predictions...")
    start_idx = FEATURE_WINDOW_SIZE + 60
    inserted = 0
    for i in range(start_idx, len(rows)):
        window = normed[i - FEATURE_WINDOW_SIZE:i]
        atr_ratio = compute_atr_ratio(
            highs[i - 14:i + 1], lows[i - 14:i + 1], closes[i - 14:i + 1]
        )
        try:
            preds = ensemble.predict(window.tolist(), atr_ratio=float(atr_ratio))
        except Exception as e:
            print(f"  skip {timestamps[i]}: {e}")
            continue

        closes_slice = closes[:i + 1]
        sma = np.mean(closes_slice[-40:]) if len(closes_slice) >= 40 else closes_slice[-1]
        regime = "bull" if closes_slice[-1] > sma else "bear"

        created_at = int(timestamps[i] / 1000) if timestamps[i] > 1e10 else timestamps[i]

        c.execute("""
            INSERT INTO equity_predictions
                (symbol, candle_ts, pred_1d, pred_5d, pred_21d, regime, features_json, created_at, source)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(symbol, candle_ts) DO UPDATE SET
                pred_1d=excluded.pred_1d,
                pred_5d=excluded.pred_5d,
                pred_21d=excluded.pred_21d,
                regime=excluded.regime,
                features_json=excluded.features_json
        """, (
            SYMBOL, timestamps[i],
            preds["pred_1d"], preds["pred_5d"], preds["pred_21d"],
            regime, "[]", created_at, SOURCE,
        ))
        inserted += 1

    conn.commit()
    print(f"Inserted {inserted} {SYMBOL} predictions")
    conn.close()
    return 0

if __name__ == "__main__":
    sys.exit(main())