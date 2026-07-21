#!/usr/bin/env python3
"""
D2: Volatility-scaled penetration labels + adaptive vol regime + dynamic barrier calib.
"""
import numpy as np
import pandas as pd
from typing import Tuple, Dict, List

MAG_CLIP = 3.0

# ── Adaptive vol regime (replaces dead GARCH) ──────────────────────────────────

def vol_regime(closes: np.ndarray) -> float:
    """Recent realized vol percentile vs history. Scaled [0,2]."""
    if len(closes) < 20: return 1.0
    returns = np.log(closes[1:] / closes[:-1])
    returns = returns[np.isfinite(returns)]
    if len(returns) < 10: return 1.0
    w = min(20, len(returns))
    recent = np.std(returns[-w:]) * np.sqrt(8760)
    look = min(500, len(returns))
    vols = [np.std(returns[i-w:i]) * np.sqrt(8760) for i in range(w, look)]
    if len(vols) < 10: return 1.0
    return 2.0 * np.mean(np.array(vols) <= recent)


def vol_break(closes: np.ndarray) -> float:
    """CUSUM structural break detector [0,1]."""
    if len(closes) < 10: return 0.0
    ar = np.abs(np.log(closes[1:] / closes[:-1]))
    ar = ar[np.isfinite(ar)]
    if len(ar) < 5: return 0.0
    m, s = ar.mean(), max(ar.std(), 1e-8)
    cusum = 0.0
    for r in ar: cusum = max(0.0, cusum + r - m - 0.5 * s)
    return min(1.0, 1.0 - np.exp(-cusum / s))


def compute_atr(df: pd.DataFrame, window: int = 14) -> pd.Series:
    hl = df['high'] - df['low']
    hc = np.abs(df['high'] - df['close'].shift(1))
    lc = np.abs(df['low'] - df['close'].shift(1))
    tr = pd.concat([hl, hc, lc], axis=1).max(axis=1).fillna(0)
    return tr.ewm(span=window, adjust=False).mean()


# ── Labels ─────────────────────────────────────────────────────────────────────

def volatility_scaled_labels(
    df: pd.DataFrame, lookback: int = 200, c: float = 2.0,
    horizons_bars: Tuple[int, int, int] = (12, 36, 72), mag_clip: float = MAG_CLIP,
) -> Dict[str, np.ndarray]:
    """Penetration labels with time-weighted magnitude."""
    df = df.copy()
    df['atr'] = compute_atr(df, window=lookback)
    df['k'] = c * df['atr'] / df['close']
    mx = max(horizons_bars); n = len(df)
    dirs = {h: np.zeros(n) for h in horizons_bars}
    mags = {h: np.zeros(n) for h in horizons_bars}
    cl, hi, lo, ks = df['close'].values, df['high'].values, df['low'].values, df['k'].values

    for i in range(lookback, n - mx):
        ct, kt = cl[i], ks[i]
        if kt <= 0 or ct <= 0: continue
        up, lo_b = ct * (1 + kt), ct * (1 - kt)
        for h in horizons_bars:
            fh, fl = hi[i+1:i+1+h], lo[i+1:i+1+h]
            if len(fh) == 0: continue
            first_d, hit_j = 0, None
            for j in range(len(fh)):
                if fh[j] >= up: first_d, hit_j = 1, j; break
                elif fl[j] <= lo_b: first_d, hit_j = -1, j; break
            if first_d != 0 and hit_j is not None:
                t_factor = 1.0 - hit_j / h
                pen = (fh[hit_j] - up) / (ct * kt) if first_d == 1 else (lo_b - fl[hit_j]) / (ct * kt)
                sm = first_d * (0.5 * t_factor + 0.5 * min(pen, 2.0))
            else:
                sm = 0.0
            dirs[h][i] = first_d
            mags[h][i] = np.clip(sm, -mag_clip, mag_clip)

    embargo = max(72, mx + lookback)
    vs = slice(lookback, n - mx)
    hk, hlbl = list(horizons_bars), ['H1', 'H2', 'H3']
    res = {'embargo': embargo, 'horizon_bars': horizons_bars,
           'timestamps': df.index[vs].values if hasattr(df.index, 'values') else np.arange(vs.start, vs.stop)}
    pen_rates = {}
    for h, l in zip(hk, hlbl):
        res[l] = (dirs[h][vs], mags[h][vs])
        pen_rates[l] = float(np.mean(dirs[h][vs] != 0))
    res['penetration_rates'] = pen_rates
    return res


def calibrate_barrier_c(df: pd.DataFrame, target_penetration: float = 0.5,
                        horizons_bars: Tuple[int, int] = (12,)) -> float:
    """Brute-force c to hit ~target penetration."""
    best_c, best_diff = 2.0, float('inf')
    for c in np.linspace(0.5, 6.0, 24):
        lbl = volatility_scaled_labels(df.iloc[:1000], c=c, horizons_bars=horizons_bars)
        pr = lbl['penetration_rates'].get('H1', 0.0)
        diff = abs(pr - target_penetration)
        if diff < best_diff: best_diff, best_c = diff, c
    return best_c


# ── Feature matrix ─────────────────────────────────────────────────────────────

def build_feature_matrix(
    df: pd.DataFrame, lookback: int = 200, normalize: bool = True,
) -> Tuple[np.ndarray, dict, List[str]]:
    """10-dim feature matrix with robust scaling. Replaces 6-dim (no more llm stub)."""
    cl, hi, lo, vo = df['close'].values, df['high'].values, df['low'].values, df['volume'].values
    n = len(df)
    rows = []
    for i in range(lookback, n):
        wc, wv = cl[i-lookback:i], vo[i-lookback:i]
        vr = vol_regime(wc)
        vb = vol_break(wc)
        fr = df['funding_rate'].iloc[i] if 'funding_rate' in df.columns else 0.0
        bz = df['basis_z'].iloc[i] if 'basis_z' in df.columns else 0.0
        ob = df['ob_imbalance'].iloc[i] if 'ob_imbalance' in df.columns else 0.0
        m12 = (cl[i] / cl[i-12] - 1.0) if i >= 12 else 0.0
        m72 = (cl[i] / cl[i-72] - 1.0) if i >= 72 else 0.0
        vp_d = (cl[i] / cl[i-24] - 1.0) * (2.0 - vo[i] / np.mean(wv[-24:])) if i >= 24 and np.mean(wv[-24:]) > 0 else 0.0
        vs = np.std(wc[-12:]) if len(wc) >= 12 else 0.0
        vl = np.std(wc[-72:]) if len(wc) >= 72 else vs
        vt = (vs / vl - 1.0) if vl > 0 else 0.0
        rr = np.mean(hi[i-12:i] - lo[i-12:i]) if i >= 12 else 0.0
        rh = np.mean(hi[i-72:i-12] - lo[i-72:i-12]) if i >= 72 else rr
        rc = (rr / rh - 1.0) if rh > 0 else 0.0
        rows.append([vr, vb, fr, bz, ob, m12, m72, vp_d, vt, rc])

    X = np.array(rows, dtype=np.float64)
    fnames = ['vol_regime', 'vol_break', 'funding_rate', 'basis_z', 'ob_imbalance',
              'momentum_12h', 'momentum_72h', 'vp_divergence', 'vol_term', 'range_compression']
    if normalize:
        meds = np.median(X, axis=0)
        mads = np.median(np.abs(X - meds), axis=0)
        mads = np.where(mads < 1e-8, 1.0, mads)
        X = (X - meds) / (1.4826 * mads)
        ns = {'schema_version': 4, 'feature_names': fnames, 'medians': meds.tolist(), 'mads': mads.tolist()}
    else:
        ns = {'schema_version': 2, 'feature_names': fnames, 'mean': np.mean(X, axis=0).tolist(), 'std': np.std(X, axis=0).tolist()}
    return X, ns, fnames


if __name__ == "__main__":
    np.random.seed(42)
    n = 500
    p = 100.0 * np.exp(np.cumsum(np.random.randn(n) * 0.001))
    df = pd.DataFrame({'open': p, 'high': p * (1 + np.abs(np.random.randn(n)*0.002)),
                       'low': p * (1 - np.abs(np.random.randn(n)*0.002)), 'close': p,
                       'volume': np.random.rand(n)*1000, 'taker_buy_base': np.random.rand(n)*500,
                       'funding_rate': np.random.randn(n)*0.001, 'basis_z': np.random.randn(n),
                       'ob_imbalance': np.random.randn(n)*0.5})
    lbl = volatility_scaled_labels(df, lookback=50, horizons_bars=(10, 20, 30))
    assert len(lbl['H1'][0]) > 0
    print(f"Labels: {len(lbl['H1'][0])}  embargo={lbl['embargo']}  pen_rates={lbl['penetration_rates']}")
    X, ns, fn = build_feature_matrix(df, lookback=50)
    assert X.shape[1] == 10
    print(f"Features: {X.shape}  {len(fn)} dims  v={ns['schema_version']}")
    bc = calibrate_barrier_c(df, target_penetration=0.5, horizons_bars=(10,))
    print(f"Calibrated c={bc:.2f}")