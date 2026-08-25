#!/usr/bin/env python3
"""
Wave B: Python reference implementation of the 8 equities features.

This is the parity reference for `engine/src/features/equities_v2.rs`.
Feature order MUST match the Rust `EquityFeatureRow::to_array()`:

  0: trend_slope     — ln(SMA50[t]) - ln(SMA50[t-20])
  1: trend_adx       — 14-period ADX (Wilder's smoothing)
  2: rsi_14          — 14-period RSI (Wilder's smoothing)
  3: vix_regime      — VIX bucket: <18→0, 18-25→1, >25→2
  4: tlt_corr_20d    — 20-day rolling Pearson correlation
  5: rvol_20d        — volume[t] / mean(volume[t-20:t])
  6: gap_pct         — (open[t] - close[t-1]) / close[t-1]
  7: drawdown_from_50d_high — (close[t] - max(close[t-50:t+1])) / max(...)

Usage:
    python3 equities_features.py --generate-fixture
    python3 equities_features.py --verify-fixture path/to/fixture.json
"""
import json
import argparse
import numpy as np
from pathlib import Path


def rolling_sma(values: np.ndarray, window: int) -> np.ndarray:
    """Simple rolling SMA. NaN before the window is full."""
    n = len(values)
    out = np.full(n, np.nan)
    if window == 0 or n < window:
        return out
    cumsum = np.cumsum(values)
    out[window - 1] = cumsum[window - 1] / window
    out[window:] = (cumsum[window:] - cumsum[:-window]) / window
    return out


def wilder_smooth(values: np.ndarray, period: int) -> np.ndarray:
    """Wilder's smoothing (EMA with alpha = 1/period)."""
    n = len(values)
    out = np.full(n, np.nan)
    if n < period or period == 0:
        return out
    out[period - 1] = np.mean(values[:period])
    for i in range(period, n):
        out[i] = (out[i - 1] * (period - 1) + values[i]) / period
    return out


def adx_14(opens, highs, lows, closes):
    """14-period ADX using Wilder's smoothing."""
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
        tr[i] = max(highs[i] - lows[i],
                    abs(highs[i] - closes[i - 1]),
                    abs(lows[i] - closes[i - 1]))

    period = 14
    atr = wilder_smooth(tr, period)
    smoothed_plus = wilder_smooth(plus_dm, period)
    smoothed_minus = wilder_smooth(minus_dm, period)

    plus_di = np.zeros(n)
    minus_di = np.zeros(n)
    dx = np.zeros(n)

    for i in range(period, n):
        if atr[i] > 0:
            plus_di[i] = 100.0 * smoothed_plus[i] / atr[i]
            minus_di[i] = 100.0 * smoothed_minus[i] / atr[i]
        s = plus_di[i] + minus_di[i]
        if s > 0:
            dx[i] = 100.0 * abs(plus_di[i] - minus_di[i]) / s

    adx_vals = wilder_smooth(dx, period)
    out = np.where(np.isfinite(adx_vals), adx_vals, 0.0)
    return out


def rsi_14(closes: np.ndarray) -> np.ndarray:
    """14-period RSI using Wilder's smoothing."""
    n = len(closes)
    out = np.full(n, 50.0)  # neutral during warmup
    if n < 15:
        return out

    gains = np.zeros(n)
    losses = np.zeros(n)
    for i in range(1, n):
        diff = closes[i] - closes[i - 1]
        if diff >= 0:
            gains[i] = diff
        else:
            losses[i] = -diff

    period = 14
    avg_gain = np.mean(gains[1:period + 1])
    avg_loss = np.mean(losses[1:period + 1])

    def rsi_val(ag, al):
        if al < 1e-12:
            return 100.0
        rs = ag / al
        return 100.0 - 100.0 / (1.0 + rs)

    out[period] = rsi_val(avg_gain, avg_loss)
    for i in range(period + 1, n):
        avg_gain = (avg_gain * (period - 1) + gains[i]) / period
        avg_loss = (avg_loss * (period - 1) + losses[i]) / period
        out[i] = rsi_val(avg_gain, avg_loss)
    return out


def vix_regime(vix: np.ndarray) -> np.ndarray:
    """Bucket VIX: <18→0, 18-25→1, >25→2."""
    out = np.zeros(len(vix))
    for i, v in enumerate(vix):
        if v <= 0:
            out[i] = 0.0
        elif v < 18:
            out[i] = 0.0
        elif v < 25:
            out[i] = 1.0
        else:
            out[i] = 2.0
    return out


def rolling_correlation(a: np.ndarray, b: np.ndarray, window: int) -> np.ndarray:
    """20-day rolling Pearson correlation."""
    n = len(a)
    out = np.zeros(n)
    if n < window + 1 or window == 0:
        return out
    for i in range(window, n):
        x = a[i - window:i]
        y = b[i - window:i]
        if np.std(x) < 1e-12 or np.std(y) < 1e-12:
            out[i] = 0.0
        else:
            out[i] = np.corrcoef(x, y)[0, 1]
    return out


def rvol_20d(volumes: np.ndarray) -> np.ndarray:
    """Relative volume: volume[t] / mean(volume[t-20:t])."""
    n = len(volumes)
    out = np.zeros(n)
    if n < 21:
        return out
    for i in range(20, n):
        avg = np.mean(volumes[i - 20:i])
        out[i] = volumes[i] / avg if avg > 0 else 0.0
    return out


def gap_pct(opens: np.ndarray, closes: np.ndarray) -> np.ndarray:
    """Overnight gap: (open[t] - close[t-1]) / close[t-1]."""
    n = len(opens)
    out = np.zeros(n)
    for i in range(1, n):
        if closes[i - 1] > 0:
            out[i] = (opens[i] - closes[i - 1]) / closes[i - 1]
    return out


def drawdown_from_high(highs: np.ndarray, closes: np.ndarray, window: int = 50) -> np.ndarray:
    """Drawdown from rolling N-day high (using high prices, not closes)."""
    n = len(closes)
    out = np.zeros(n)
    if n < window + 1:
        return out
    for i in range(window, n):
        roll_high = np.max(highs[i - window:i + 1])
        if roll_high > 0:
            out[i] = (closes[i] - roll_high) / roll_high
    return out


def compute_equity_features(opens, highs, lows, closes, volumes, vix=None, tlt=None):
    """Compute all 8 features. Returns list of dicts."""
    n = len(closes)
    closes_arr = np.asarray(closes, dtype=np.float64)
    opens_arr = np.asarray(opens, dtype=np.float64)
    highs_arr = np.asarray(highs, dtype=np.float64)
    lows_arr = np.asarray(lows, dtype=np.float64)
    volumes_arr = np.asarray(volumes, dtype=np.float64)

    sma50 = rolling_sma(closes_arr, 50)
    adx = adx_14(opens_arr, highs_arr, lows_arr, closes_arr)
    rsi = rsi_14(closes_arr)

    vix_arr = np.asarray(vix, dtype=np.float64) if vix is not None else None
    tlt_arr = np.asarray(tlt, dtype=np.float64) if tlt is not None else None

    vix_feat = vix_regime(vix_arr) if vix_arr is not None else np.zeros(n)
    tlt_corr = rolling_correlation(closes_arr, tlt_arr, 20) if tlt_arr is not None else np.zeros(n)
    rvol = rvol_20d(volumes_arr)
    gaps = gap_pct(opens_arr, closes_arr)
    dd = drawdown_from_high(highs_arr, closes_arr, 50)

    rows = []
    for i in range(n):
        if i >= 20 and np.isfinite(sma50[i]) and np.isfinite(sma50[i - 20]) and sma50[i - 20] > 0:
            ts = np.log(sma50[i] / sma50[i - 20])
        else:
            ts = 0.0

        rows.append({
            "timestamp": i,
            "trend_slope": float(ts),
            "trend_adx": float(adx[i]),
            "rsi_14": float(rsi[i]),
            "vix_regime": float(vix_feat[i]),
            "tlt_corr_20d": float(tlt_corr[i]),
            "rvol_20d": float(rvol[i]),
            "gap_pct": float(gaps[i]),
            "drawdown_from_50d_high": float(dd[i]),
        })
    return rows


def generate_synthetic_data(n=120, seed=42):
    """Generate synthetic OHLCV + VIX + TLT for parity testing."""
    rng = np.random.RandomState(seed)
    # Random walk with slight uptrend
    returns = rng.normal(0.001, 0.015, n)
    closes = 100.0 * np.exp(np.cumsum(returns))
    opens = np.roll(closes, 1)
    opens[0] = closes[0]
    opens += rng.normal(0, 0.1, n)  # small overnight gap
    highs = np.maximum(opens, closes) + rng.uniform(0.1, 0.8, n)
    lows = np.minimum(opens, closes) - rng.uniform(0.1, 0.8, n)
    volumes = rng.randint(800_000, 2_000_000, n).astype(np.float64)
    # VIX oscillating between 14 and 30
    vix = 18.0 + 8.0 * np.sin(np.linspace(0, 4 * np.pi, n))
    # TLT: slight negative correlation with QQQ
    tlt = 90.0 - 0.3 * np.cumsum(returns) + rng.normal(0, 0.2, n)

    return {
        "opens": opens.tolist(),
        "highs": highs.tolist(),
        "lows": lows.tolist(),
        "closes": closes.tolist(),
        "volumes": volumes.tolist(),
        "vix": vix.tolist(),
        "tlt": tlt.tolist(),
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--generate-fixture", action="store_true")
    parser.add_argument("--verify-fixture", type=str, default=None)
    parser.add_argument("--output", type=str, default=None)
    args = parser.parse_args()

    if args.generate_fixture:
        data = generate_synthetic_data(n=120, seed=42)
        features = compute_equity_features(
            data["opens"], data["highs"], data["lows"],
            data["closes"], data["volumes"],
            vix=data["vix"], tlt=data["tlt"]
        )
        fixture = {"input": data, "expected": features}
        out_path = args.output or "engine/tests/fixtures/equity_feature_parity.json"
        Path(out_path).parent.mkdir(parents=True, exist_ok=True)
        with open(out_path, "w") as f:
            json.dump(fixture, f, indent=2)
        print(f"Fixture written to {out_path} ({len(features)} rows)")

    elif args.verify_fixture:
        with open(args.verify_fixture) as f:
            fixture = json.load(f)
        data = fixture["input"]
        expected = fixture["expected"]
        computed = compute_equity_features(
            data["opens"], data["highs"], data["lows"],
            data["closes"], data["volumes"],
            vix=data["vix"], tlt=data["tlt"]
        )
        max_diff = 0.0
        for i, (exp, comp) in enumerate(zip(expected, computed)):
            for key in ["trend_slope", "trend_adx", "rsi_14", "vix_regime",
                        "tlt_corr_20d", "rvol_20d", "gap_pct", "drawdown_from_50d_high"]:
                diff = abs(exp[key] - comp[key])
                if diff > max_diff:
                    max_diff = diff
                if diff > 1e-6:
                    print(f"MISMATCH row={i} feature={key}: expected={exp[key]:.10f} computed={comp[key]:.10f} diff={diff:.2e}")
        if max_diff < 1e-6:
            print(f"PARITY OK: max_diff={max_diff:.2e} across {len(expected)} rows x 8 features")
        else:
            print(f"PARITY FAILED: max_diff={max_diff:.2e}")
            return 1
    else:
        parser.print_help()


if __name__ == "__main__":
    import sys
    sys.exit(main() or 0)
