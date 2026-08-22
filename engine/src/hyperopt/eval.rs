//! Real walk-forward rank-IC evaluation for hyperopt.
//!
//! Replaces the placeholder objective (the old `Optimizer::evaluate` returned
//! `None` and the runner stored mock IC values). This computes a defensible,
//! data-driven objective directly from the `equity_candles` table, with no
//! dependence on live model predictions:
//!
//!   rank IC (Spearman) between a strategy's per-bar signal and the forward
//!   return over `horizon` bars, averaged over walk-forward test folds with
//!   an embargo (lookahead guard). `n_trades` is the number of bars with a
//!   non-zero signal — the trading opportunities the promotion gates care about.
//!
//! Only families that actually have a signal implementation are evaluated;
//! an unsupported family logs a warning and is skipped rather than silently
//! scored with the wrong signal.

use std::collections::HashMap;
use tracing::warn;

use super::optimizer::{Optimizer, OptimizationResult, ParamConfig, ParamDef};

/// Evaluation hyperparameters shared across the pipeline.
#[derive(Debug, Clone)]
pub struct EvalSpec {
    /// Forward return horizon, in trading-day bars (5 ≈ one week).
    pub horizon: usize,
    /// Minimum number of candles required to run a fold at all.
    pub min_bars: usize,
    /// Minimum non-zero-signal bars required to accept a candidate.
    pub min_trades: usize,
    /// IC deploy gate (matches the QQQ training notebook's IC_GATE = 0.03).
    pub ic_gate: f64,
    /// Minimum signal points per fold for the rank correlation to count.
    pub min_per_fold: usize,
}

impl Default for EvalSpec {
    fn default() -> Self {
        Self {
            horizon: 5,
            min_bars: 400,   // ~2 years of daily bars
            min_trades: 100, // candidate → paper promotion gate
            ic_gate: 0.03,
            min_per_fold: 20,
        }
    }
}

fn param(cfg: &HashMap<String, f64>, name: &str, default: f64) -> f64 {
    cfg.get(name).copied().unwrap_or(default)
}

/// SMA-regime momentum signal: the normalized signed distance from a trailing
/// simple moving average, zeroed below a magnitude threshold. This is the same
/// trend/regime family the live equity engine trades on.
pub fn sma_momentum_signal(closes: &[f64], cfg: &HashMap<String, f64>) -> Vec<f64> {
    let window = param(cfg, "sma_window", 200.0).max(2.0) as usize;
    let threshold = param(cfg, "threshold", 0.0);
    let mut sig = vec![0.0; closes.len()];
    if closes.len() < window {
        return sig;
    }
    // Rolling sum so repeats drive this in O(n), not O(n·window).
    let mut cum = 0.0;
    for i in 0..closes.len() {
        cum += closes[i];
        if i >= window {
            cum -= closes[i - window];
        }
        if i + 1 >= window {
            let sma = cum / window as f64;
            if sma > 0.0 {
                let mom = (closes[i] - sma) / sma;
                if mom.abs() >= threshold {
                    sig[i] = mom;
                }
            }
        }
    }
    sig
}

/// Forward (simple) return, aligned to the signal bar. `NaN` where undefined.
pub fn forward_returns(closes: &[f64], horizon: usize) -> Vec<f64> {
    let mut r = vec![f64::NAN; closes.len()];
    for i in 0..closes.len() {
        if i + horizon < closes.len() && closes[i] > 0.0 {
            r[i] = (closes[i + horizon] - closes[i]) / closes[i];
        }
    }
    r
}

/// Average-rank transform (ties share the average rank) — needed for a correct
/// Spearman correlation on quantized signals like ours.
fn ranks(v: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| v[a].partial_cmp(&v[b]).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = vec![0.0; v.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && v[idx[j + 1]] == v[idx[i]] {
            j += 1;
        }
        let avg = (i + j + 2) as f64 / 2.0; // 1-based average rank over the tie block
        for k in i..=j {
            out[idx[k]] = avg;
        }
        i = j + 1;
    }
    out
}

fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len();
    if n < 2 {
        return 0.0;
    }
    let mx = x.iter().sum::<f64>() / n as f64;
    let my = y.iter().sum::<f64>() / n as f64;
    let (mut num, mut dx, mut dy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (a, b) = (x[i] - mx, y[i] - my);
        num += a * b;
        dx += a * a;
        dy += b * b;
    }
    if dx <= 0.0 || dy <= 0.0 {
        return 0.0;
    }
    num / (dx.sqrt() * dy.sqrt())
}

/// Spearman rank correlation over the jointly-finite elements of `x` and `y`.
pub fn spearman(x: &[f64], y: &[f64]) -> f64 {
    debug_assert_eq!(x.len(), y.len());
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for i in 0..x.len() {
        if x[i].is_finite() && y[i].is_finite() {
            xs.push(x[i]);
            ys.push(y[i]);
        }
    }
    if xs.len() < 2 {
        return 0.0;
    }
    pearson(&ranks(&xs), &ranks(&ys))
}

/// Walk-forward rank-IC for one parameter set. Returns `None` when data is
/// insufficient or no fold clears `min_per_fold`. `n_trades` counts
/// non-zero-signal bars across the ENTIRE series (the gate metric), not just
/// the test folds, so it reflects the deployable trade frequency.
pub fn evaluate_params(
    closes: &[f64],
    params: &HashMap<String, f64>,
    spec: &EvalSpec,
    opt: &Optimizer,
) -> Option<OptimizationResult> {
    if closes.len() < spec.min_bars {
        return None;
    }
    let sig = sma_momentum_signal(closes, params);
    let rets = forward_returns(closes, spec.horizon);
    let splits = opt.generate_splits(closes.len());

    let mut fold_ics = Vec::new();
    for sp in &splits {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for i in sp.test_start..sp.test_end.min(closes.len()) {
            if i + spec.horizon >= closes.len() {
                continue;
            }
            let rr = rets[i];
            if rr.is_finite() {
                xs.push(sig[i]);
                ys.push(rr);
            }
        }
        if xs.len() >= spec.min_per_fold {
            fold_ics.push(spearman(&xs, &ys));
        }
    }
    if fold_ics.is_empty() {
        return None;
    }

    let mean_ic = fold_ics.iter().sum::<f64>() / fold_ics.len() as f64;
    let std_ic = if fold_ics.len() > 1 {
        (fold_ics
            .iter()
            .map(|v| (v - mean_ic).powi(2))
            .sum::<f64>()
            / (fold_ics.len() as f64 - 1.0))
            .sqrt()
    } else {
        0.0
    };
    let n_trades = sig.iter().filter(|s| s.abs() > 0.0).count();

    Some(OptimizationResult {
        params: ParamConfig {
            params: params.clone(),
        },
        mean_ic,
        std_ic,
        n_trades,
        fold_ics,
    })
}

/// Family-aware signal selection. Returns `None` for any family without an
/// implemented signal so we never score a candidate with the wrong objective.
pub fn signal_for(
    family: &str,
    closes: &[f64],
    params: &HashMap<String, f64>,
) -> Option<Vec<f64>> {
    match family {
        "sma_regime" => Some(sma_momentum_signal(closes, params)),
        other => {
            warn!("hyperopt: no signal implementation for family '{other}'; skipping");
            None
        }
    }
}

/// Parameter grid for a family. Empty means the family is not optimizable yet.
pub fn param_defs_for_family(family: &str) -> Vec<ParamDef> {
    match family {
        "sma_regime" => vec![
            ParamDef {
                name: "sma_window".to_string(),
                values: vec![50.0, 100.0, 200.0],
            },
            ParamDef {
                name: "threshold".to_string(),
                values: vec![0.0, 0.005, 0.01],
            },
        ],
        _ => Vec::new(),
    }
}

/// Convenience: mean IC for a parameter set (used by the stability check).
pub fn mean_ic(
    closes: &[f64],
    params: &HashMap<String, f64>,
    spec: &EvalSpec,
    opt: &Optimizer,
) -> f64 {
    evaluate_params(closes, params, spec, opt)
        .map(|r| r.mean_ic)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spearman_perfect_monotone() {
        let x: Vec<f64> = (0..20).map(|i| i as f64).collect();
        let y: Vec<f64> = x.iter().map(|v| v * 3.0 + 1.0).collect();
        let r = spearman(&x, &y);
        assert!((r - 1.0).abs() < 1e-9, "r={r}");
    }

    #[test]
    fn test_spearman_ties() {
        let x = vec![1.0, 1.0, 2.0, 2.0, 3.0, 3.0];
        let y = vec![3.0, 3.0, 2.0, 2.0, 1.0, 1.0];
        // Perfectly anti-monotone with ties → -1.
        let r = spearman(&x, &y);
        assert!((r + 1.0).abs() < 1e-9, "r={r}");
    }

    #[test]
    fn test_signal_positive_ic_on_trend() {
        // Monotonic rising series: sma-momentum is positive and returns are positive.
        let closes: Vec<f64> = (0..1000).map(|i| 100.0 + i as f64 * 0.1).collect();
        let mut params = HashMap::new();
        params.insert("sma_window".to_string(), 20.0);
        params.insert("threshold".to_string(), 0.0);
        let sig = sma_momentum_signal(&closes, &params);
        let rets = forward_returns(&closes, 5);
        let r = spearman(&sig, &rets);
        assert!(r > 0.5, "expected trending series to give positive rank IC, got r={r}");
    }

    #[test]
    fn test_evaluate_params_insufficient_data() {
        let closes: Vec<f64> = (0..50).map(|i| i as f64).collect();
        let opt = Optimizer::new(crate::hyperopt::optimizer::OptimizerConfig::default());
        let res = evaluate_params(&closes, &HashMap::new(), &EvalSpec::default(), &opt);
        assert!(res.is_none(), "min_bars=400 should reject 50 samples");
    }
}