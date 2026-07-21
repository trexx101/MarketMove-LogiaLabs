#!/usr/bin/env python3
"""
D2: Volatility-scaled penetration labels + feature matrix builder.
Produces signed-magnitude labels clipped to [-3, 3] to prevent outlier domination.

Label design: for each bar t, compute whether price crosses ±k% within N future
bars, where k = c * rolling_ATR(t) / close(t). FORWARD-ONLY.
Embargo = max(72, max(horizons) + lookback) to prevent lookahead bias.
"""
import numpy as np
import pandas as pd
from typing import Tuple, Dict

MAG_CLIP = 3.0  # clip signed magnitude to [-3, 3]


# ── Volatility features (ported from Rust D1 for train==serve parity) ──────────

def vol_regime(closes: np.ndarray) -> float:
    """GARCH(1,1) volatility regime, scaled to [0,2]. Matches Rust impl."""
    if len(closes) < 2:
        return 1.0
    OMEGA, ALPHA, BETA, LONG_RUN_VOL = 0.05, 0.1, 0.85, 1.0
    returns = np.log(closes[1:] / closes[:-1])
    returns = returns[np.isfinite(returns)]
    if len(returns) == 0:
        return 1.0
    sigma2 = OMEGA / (1.0 - ALPHA - BETA)
    for r in returns:
        sigma2 = OMEGA + ALPHA * r**2 + BETA * sigma2
    current_vol = np.sqrt(sigma2)
    return 2.0 * current_vol / (current_vol + LONG_RUN_VOL)


def vol_break(closes: np.ndarray) -> float:
    """CUSUM structural break detector, scaled to [0,1]. Matches Rust impl."""
    if len(closes) < 10:
        return 0.0
    abs_returns = np.abs(np.log(closes[1:] / closes[:-1]))
    abs_returns = abs_returns[np.isfinite(abs_returns)]
    if len(abs_returns) < 5:
        return 0.0
    mean = abs_returns.mean()
    std_dev = max(abs_returns.std(), 1e-8)
    k = 0.5 * std_dev
    cusum = 0.0
    for r in abs_returns:
        cusum = max(0.0, cusum + r - mean - k)
    return min(1.0, 1.0 - np.exp(-cusum / std_dev))


# ── ATR for label k scaling ────────────────────────────────────────────────────

def compute_atr(df: pd.DataFrame, window: int = 14) -> pd.Series:
    """Average True Range with EMA smoothing."""
    high_low = df['high'] - df['low']
    high_close = np.abs(df['high'] - df['close'].shift(1))
    low_close = np.abs(df['low'] - df['close'].shift(1))
    tr = pd.concat([high_low, high_close, low_close], axis=1).max(axis=1).fillna(0)
    return tr.ewm(span=window, adjust=False).mean()


# ── Penetration labels ─────────────────────────────────────────────────────────

def volatility_scaled_labels(
    df: pd.DataFrame,
    lookback: int = 200,
    c: float = 0.5,
    horizons_bars: Tuple[int, int, int] = (12, 36, 72),
    mag_clip: float = MAG_CLIP,
) -> Dict[str, np.ndarray]:
    """
    Compute volatility-scaled penetration labels for 3 horizons.

    For each bar t: k = c * ATR(t) / close(t). Barrier = close_t * (1 ± k).
    Label = direction of first barrier hit * magnitude (clipped to ±mag_clip).

    Returns dict with H1/H2/H3 keys, penetration_rates, embargo, timestamps.
    """
    df = df.copy()
    df['atr'] = compute_atr(df, window=lookback)
    df['k'] = c * df['atr'] / df['close']

    max_horizon = max(horizons_bars)
    n = len(df)
    directions = {h: np.zeros(n, dtype=np.float64) for h in horizons_bars}
    magnitudes = {h: np.zeros(n, dtype=np.float64) for h in horizons_bars}

    closes = df['close'].values
    highs = df['high'].values
    lows = df['low'].values
    ks = df['k'].values

    for i in range(lookback, n - max_horizon):
        close_t = closes[i]
        k_t = ks[i]
        if k_t <= 0 or close_t <= 0:
            continue
        upper = close_t * (1 + k_t)
        lower = close_t * (1 - k_t)

        for h in horizons_bars:
            fh = highs[i + 1: i + 1 + h]
            fl = lows[i + 1: i + 1 + h]
            if len(fh) == 0:
                continue

            first_dir = 0
            max_pen = 0.0
            for j in range(len(fh)):
                if first_dir == 0:
                    if fh[j] >= upper:
                        first_dir = 1
                        max_pen = (fh[j] - upper) / (close_t * k_t)
                    elif fl[j] <= lower:
                        first_dir = -1
                        max_pen = (lower - fl[j]) / (close_t * k_t)
                elif first_dir == 1 and fh[j] > upper:
                    max_pen = max(max_pen, (fh[j] - upper) / (close_t * k_t))
                elif first_dir == -1 and fl[j] < lower:
                    max_pen = max(max_pen, (lower - fl[j]) / (close_t * k_t))

            directions[h][i] = first_dir
            signed_mag = first_dir * max_pen if first_dir != 0 else 0.0
            magnitudes[h][i] = np.clip(signed_mag, -mag_clip, mag_clip)

    embargo = max(72, max_horizon + lookback)
    valid_slice = slice(lookback, n - max_horizon)
    h_keys = list(horizons_bars)
    h_labels = ['H1', 'H2', 'H3']

    result = {
        'embargo': embargo,
        'horizon_bars': horizons_bars,
        'timestamps': df.index[valid_slice].values if hasattr(df.index, 'values')
                       else np.arange(valid_slice.start, valid_slice.stop),
    }
    pen_rates = {}
    for h, lbl in zip(h_keys, h_labels):
        result[lbl] = (directions[h][valid_slice], magnitudes[h][valid_slice])
        pen_rates[lbl] = float(np.mean(directions[h][valid_slice] != 0))
    result['penetration_rates'] = pen_rates

    return result


# ── Feature matrix ─────────────────────────────────────────────────────────────

def build_feature_matrix(
    df: pd.DataFrame,
    lookback: int = 200,
) -> Tuple[np.ndarray, dict]:
    """
    Build the 6-dim V2 feature matrix: [vol_regime, vol_break, funding_rate,
    basis_z, llm_bull_prob, ob_imbalance].
    """
    closes = df['close'].values
    n = len(df)
    features = []
    for i in range(lookback, n):
        window_closes = closes[i - lookback: i]
        vr = vol_regime(window_closes)
        vb = vol_break(window_closes)
        fr = df['funding_rate'].iloc[i] if 'funding_rate' in df.columns else 0.0
        bz = df['basis_z'].iloc[i] if 'basis_z' in df.columns else 0.0
        ob = df['ob_imbalance'].iloc[i] if 'ob_imbalance' in df.columns else 0.0
        features.append([vr, vb, fr, bz, 0.5, ob])

    X = np.array(features, dtype=np.float64)
    norm_stats = {
        'schema_version': 2,
        'mean': np.mean(X, axis=0).tolist(),
        'std': np.std(X, axis=0).tolist(),
    }
    return X, norm_stats


# ── Self-test ──────────────────────────────────────────────────────────────────

if __name__ == "__main__":
    np.random.seed(42)
    n = 500
    prices = 100.0 * np.exp(np.cumsum(np.random.randn(n) * 0.001))
    df = pd.DataFrame({
        'open': prices,
        'high': prices * (1 + np.abs(np.random.randn(n) * 0.002)),
        'low': prices * (1 - np.abs(np.random.randn(n) * 0.002)),
        'close': prices,
        'volume': np.random.rand(n) * 1000,
        'taker_buy_base': np.random.rand(n) * 500,
        'funding_rate': np.random.randn(n) * 0.001,
        'basis_z': np.random.randn(n),
        'ob_imbalance': np.random.randn(n) * 0.5,
    })

    labels = volatility_scaled_labels(df, lookback=50, horizons_bars=(10, 20, 30))
    assert len(labels['H1'][0]) > 0, "labels should be non-empty"
    print(f"Label test passed: {len(labels['H1'][0])} samples, embargo={labels['embargo']}")
    print(f"Penetration rates: {labels['penetration_rates']}")
    for hkey in ['H1', 'H2', 'H3']:
        dirs, mags = labels[hkey]
        print(f"  {hkey}: up={int(np.sum(dirs>0))} down={int(np.sum(dirs<0))} neutral={int(np.sum(dirs==0))} "
              f"mag_mean={np.mean(mags):.4f} mag_max={np.max(np.abs(mags)):.4f}")

    X, stats = build_feature_matrix(df, lookback=50)
    assert X.shape[1] == 6, f"expected 6 features, got {X.shape[1]}"
    assert stats['schema_version'] == 2
    print(f"Feature test passed: X={X.shape}, norm_stats v2 with {len(stats['mean'])} features")