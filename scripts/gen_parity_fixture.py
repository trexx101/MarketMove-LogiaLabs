#!/usr/bin/env python3
"""One-shot generator for tests/fixtures/parity_golden_168h.json.

Produces a 168-candle (7-day hourly) golden fixture for the parity harness
(Feature 13). The fixture encodes a deterministic synthetic price walk and
the expected features + signals that the Rust pipeline should reproduce.

This is a PLACEHOLDER for the Colab-exported reference. Replace this file
with the real Colab export before relying on the parity marker for live
trading. The Rust pipeline is deterministic, so re-running this script
produces an identical fixture (modulo the sha256 hash on the
parity marker, which captures the exact bytes).
"""
from __future__ import annotations

import json
import math
import sys
from pathlib import Path

# Match the Rust harness defaults in engine::parity.
SMA_WINDOW = 200
MAGNITUDE_THRESHOLD = 0.005
FEATURE_WINDOW_SIZE = 72
N = 168  # 7 days of hourly candles
TOLERANCE = 1e-6


def synth_candle(i: int) -> dict:
    # Linear-up walk with a small sinusoid: close = 100 + 0.05 * i + 0.5 * sin(i / 6).
    close = 100.0 + 0.05 * i + 0.5 * math.sin(i / 6.0)
    return {
        "ts": i * 3600,
        "open": close - 0.5,
        "high": close + 1.0,
        "low": close - 1.0,
        "close": close,
        "volume": 100.0 + 0.01 * i,
        "vwap": close,
    }


def compute_features(candles: list[dict]) -> list[dict]:
    """Mirror of engine::features::compute_features (Colab parity)."""
    n = len(candles)
    # True range per candle.
    tr = [0.0] * n
    for i, c in enumerate(candles):
        if i == 0:
            tr[i] = c["high"] - c["low"]
        else:
            prev_close = candles[i - 1]["close"]
            hl = c["high"] - c["low"]
            hpc = abs(c["high"] - prev_close)
            lpc = abs(c["low"] - prev_close)
            tr[i] = max(hl, hpc, lpc)

    rows: list[dict] = []
    window = 72
    for i in range(n):
        c = candles[i]
        # log return
        if i == 0:
            lr = 0.0
        else:
            prev = candles[i - 1]["close"]
            lr = math.log(c["close"] / prev) if prev > 0 else 0.0
        # ATR(72) = rolling mean of TR, min_periods=1
        start = max(0, i - window + 1)
        sl = tr[start : i + 1]
        atr = sum(sl) / len(sl)
        # vwap_dev
        vwap_dev = (c["close"] - c["vwap"]) / c["vwap"] if c["vwap"] != 0 else 0.0
        rows.append(
            {
                "candle_ts": c["ts"],
                "log_return": lr,
                "atr_72": atr,
                "vwap_dev": vwap_dev,
            }
        )
    return rows


def compute_sma(closes: list[float], window: int) -> tuple[float, bool]:
    if not closes or window == 0:
        return 0.0, False
    n = min(len(closes), window)
    slice_ = closes[len(closes) - n :]
    return sum(slice_) / n, len(closes) >= window


def next_position(
    current: int, pred_4h: float, pred_24h: float, close: float, sma: float, sma_valid: bool, threshold: float
) -> int:
    """Mirror of engine::strategy::next_position (Colab parity)."""
    if pred_4h > threshold and pred_24h > threshold:
        raw = 1
    elif pred_4h < -threshold and pred_24h < -threshold:
        raw = -1
    else:
        raw = 0
    if not sma_valid:
        return current
    regime = 1 if close > sma else -1
    if raw == 1 and regime == 1:
        filtered = 1
    elif raw == -1 and regime == -1:
        filtered = -1
    else:
        filtered = 0
    if filtered != 0:
        return filtered
    return current


def build_predictions(candles: list[dict]) -> list[dict]:
    """Synthetic recorded predictions: bullish drift above threshold most of the time.

    Two regimes:
    - Steps 0..20: below threshold (warmup + flat noise)
    - Steps 21..120: bullish (pred_4h, pred_24h above threshold)
    - Steps 121..167: bearish (pred_4h, pred_24h below -threshold)
    """
    preds: list[dict] = []
    for i, c in enumerate(candles):
        if i < 21:
            p1, p4, p24 = 0.0005, 0.001, 0.001
        elif i < 121:
            p1, p4, p24 = 0.002, 0.008, 0.012
        else:
            p1, p4, p24 = -0.002, -0.008, -0.012
        preds.append(
            {
                "candle_ts": c["ts"],
                "pred_1h": p1,
                "pred_4h": p4,
                "pred_24h": p24,
            }
        )
    return preds


def build_signals(candles: list[dict], preds: list[dict]) -> list[dict]:
    closes = [c["close"] for c in candles]
    current = 0
    signals: list[dict] = []
    for i, c in enumerate(candles):
        p = preds[i]
        sma, sma_valid = compute_sma(closes[: i + 1], SMA_WINDOW)
        pos = next_position(current, p["pred_4h"], p["pred_24h"], c["close"], sma, sma_valid, MAGNITUDE_THRESHOLD)
        signals.append({"candle_ts": c["ts"], "position": pos})
        current = pos
    return signals


def main(out_path: str) -> int:
    candles = [synth_candle(i) for i in range(N)]
    features = compute_features(candles)
    preds = build_predictions(candles)
    signals = build_signals(candles, preds)

    fixture = {
        "magnitude_threshold": MAGNITUDE_THRESHOLD,
        "sma_window": SMA_WINDOW,
        "feature_window_size": FEATURE_WINDOW_SIZE,
        "candles": candles,
        "predictions": preds,
        "expected_features": features,
        "expected_signals": signals,
    }

    p = Path(out_path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(fixture, indent=2))
    print(f"wrote {p} ({p.stat().st_size} bytes, {N} candles)")
    return 0


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "tests/fixtures/parity_golden_168h.json"
    sys.exit(main(out))
