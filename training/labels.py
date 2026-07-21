#!/usr/bin/env python3
"""
D2: Volatility-scaled penetration labels + feature matrix builder.
Produced by DeepSeek-R1, corrected by Hermes agent:
- Ported GARCH vol_regime + CUSUM vol_break from D1 Rust to Python (parity).
- Fixed DataFrame construction (removed broken lambdas).
- Fixed build_feature_matrix to actually compute vol features (was `pass`).

Label design: for each bar t, compute whether price crosses ±k% within N future
bars, where k = c * rolling_ATR(t). FORWARD-ONLY (uses future highs/lows only).
Embargo = max(72, horizon + N) to prevent lookahead bias.
"""
import numpy as np
import pandas as pd
from typing import Tuple, Dict


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
    c: float = 0.15,
    horizons_bars: Tuple[int, int, int] = (12, 36, 72),
) -> Dict[str, np.ndarray]:
    """
    Compute volatility-scaled penetration labels for 1H/4H/24H horizons.

    For each bar t: k = c * ATR(t). Barriers = close_t * (1 ± k).
    Label = direction of first barrier hit (+1 up, -1 down, 0 none)
    and magnitude = how far price penetrated beyond the barrier, scaled by k.

    Args:
        df: DataFrame with ['open','high','low','close','volume'] columns.
        lookback: ATR window.
        c: ATR scaling factor for barrier width.
        horizons_bars: (1H, 4H, 24H) in bars.

    Returns:
        {'1H': (directions, magnitudes), '4H': ..., '24H': ...,
         'timestamps': np.ndarray, 'embargo': int}
    """
    df = df.copy()
    df['atr'] = compute_atr(df, window=lookback)
    df['k'] = c * df['atr']

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
            future_highs = highs[i + 1: i + 1 + h]
            future_lows = lows[i + 1: i + 1 + h]
            if len(future_highs) == 0:
                continue

            first_dir = 0
            max_pen = 0.0
            for j in range(len(future_highs)):
                fh = future_highs[j]
                fl = future_lows[j]
                if first_dir == 0:
                    if fh >= upper:
                        first_dir = 1
                        max_pen = (fh - upper) / (close_t * k_t)
                    elif fl <= lower:
                        first_dir = -1
                        max_pen = (lower - fl) / (close_t * k_t)
                elif first_dir == 1 and fh > upper:
                    pen = (fh - upper) / (close_t * k_t)
                    max_pen = max(max_pen, pen)
                elif first_dir == -1 and fl < lower:
                    pen = (lower - fl) / (close_t * k_t)
                    max_pen = max(max_pen, pen)

            directions[h][i] = first_dir
            magnitudes[h][i] = first_dir * max_pen if first_dir != 0 else 0.0

    embargo = max(72, max_horizon + lookback)
    valid_slice = slice(lookback, n - max_horizon)

    h_keys = list(horizons_bars)
    h_labels = ['H1', 'H2', 'H3']

    result = {
        'embargo': embargo,
        'horizon_bars': horizons_bars,
        'timestamps': df.index[valid_slice].values if hasattr(df.index, 'values') else np.arange(valid_slice.start, valid_slice.stop),
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

    vol_regime/vol_break are computed from closes (parity with Rust D1).
    funding_rate/basis_z/ob_imbalance come from the DataFrame columns.
    llm_bull_prob defaults to 0.5 (neutral) until the D4 LLM adapter is wired.

    Returns (X, norm_stats) where X is (n_samples, 6) and norm_stats has
    schema_version=2.
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
        'funding_rate': np.random.randn(n) * 0.001,
        'basis_z': np.random.randn(n),
        'ob_imbalance': np.random.randn(n) * 0.5,
    })

    # Test label generation
    labels = volatility_scaled_labels(df, lookback=50, horizons_bars=(10, 20, 30))
    assert len(labels['H1'][0]) > 0, "labels should be non-empty"
    print(f"Label test passed: {len(labels['H1'][0])} samples, embargo={labels['embargo']}")
    print(f"Penetration rates: {labels['penetration_rates']}")

    # Test feature building
    X, stats = build_feature_matrix(df, lookback=50)
    assert X.shape[1] == 6, f"expected 6 features, got {X.shape[1]}"
    assert stats['schema_version'] == 2
    print(f"Feature test passed: X={X.shape}, norm_stats v2 with {len(stats['mean'])} features")
