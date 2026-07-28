use crate::strategy_lab::{BacktestMetrics, BacktestTrade};

/// Compute performance metrics from an equity curve and trade list.
///
/// * `equity_curve` — (timestamp, equity_value) pairs, one per bar.
/// * `trades` — list of completed trades.
/// * `buy_hold_return` — the asset's return over the same period.
pub(crate) fn compute(
    equity_curve: &[(i64, f64)],
    trades: &[BacktestTrade],
    buy_hold_return: f64,
) -> BacktestMetrics {
    if equity_curve.len() < 2 {
        return BacktestMetrics {
            cagr: 0.0,
            sharpe: 0.0,
            sortino: 0.0,
            max_drawdown: 0.0,
            win_rate: 0.0,
            profit_factor: 0.0,
            trade_count: trades.len(),
            total_return: 0.0,
            buy_hold_return,
        };
    }

    let initial_equity = equity_curve.first().map(|(_, v)| *v).unwrap_or(1.0);
    let final_equity = equity_curve.last().map(|(_, v)| *v).unwrap_or(initial_equity);
    let total_return = if initial_equity > 0.0 {
        (final_equity - initial_equity) / initial_equity
    } else {
        0.0
    };

    // Daily returns
    let daily_returns = compute_daily_returns(equity_curve);

    // CAGR
    let n_days = count_trading_days(equity_curve);
    let cagr = if n_days > 0 && initial_equity > 0.0 {
        (final_equity / initial_equity).powf(252.0 / n_days as f64) - 1.0
    } else {
        0.0
    };

    // Sharpe
    let mean_daily = if daily_returns.is_empty() {
        0.0
    } else {
        daily_returns.iter().sum::<f64>() / daily_returns.len() as f64
    };
    let variance: f64 = if daily_returns.len() > 1 {
        daily_returns
            .iter()
            .map(|r| (r - mean_daily).powi(2))
            .sum::<f64>()
            / (daily_returns.len() - 1) as f64
    } else {
        0.0
    };
    let std_daily = variance.sqrt();
    let sharpe = if std_daily > 0.0 {
        mean_daily / std_daily * (252.0_f64).sqrt()
    } else {
        0.0
    };

    // Sortino
    let downside_returns: Vec<f64> = daily_returns
        .iter()
        .filter(|&&r| r < 0.0)
        .copied()
        .collect();
    let downside_var: f64 = if downside_returns.len() > 1 {
        downside_returns
            .iter()
            .map(|r| (r - mean_daily).powi(2))
            .sum::<f64>()
            / (downside_returns.len() - 1) as f64
    } else {
        0.0
    };
    let downside_std = downside_var.sqrt();
    let sortino = if downside_std > 0.0 {
        mean_daily / downside_std * (252.0_f64).sqrt()
    } else {
        0.0
    };

    // Max drawdown
    let max_drawdown = compute_max_drawdown(equity_curve);

    // Win rate & profit factor
    let (win_rate, profit_factor) = compute_trade_stats(trades);

    BacktestMetrics {
        cagr,
        sharpe,
        sortino,
        max_drawdown,
        win_rate,
        profit_factor,
        trade_count: trades.len(),
        total_return,
        buy_hold_return,
    }
}

fn compute_daily_returns(equity_curve: &[(i64, f64)]) -> Vec<f64> {
    if equity_curve.len() < 2 {
        return vec![];
    }
    let mut returns = Vec::with_capacity(equity_curve.len() - 1);
    for w in equity_curve.windows(2) {
        let prev = w[0].1;
        let curr = w[1].1;
        if prev > 0.0 {
            returns.push((curr / prev) - 1.0);
        }
    }
    returns
}

fn count_trading_days(equity_curve: &[(i64, f64)]) -> usize {
    if equity_curve.len() < 2 {
        return 0;
    }
    let first_ts = equity_curve.first().map(|(t, _)| *t).unwrap_or(0);
    let last_ts = equity_curve.last().map(|(t, _)| *t).unwrap_or(0);
    // Each bar is one trading day; count the number of bars.
    // Use the actual day count from the timestamps for a more accurate annualization.
    let days = ((last_ts - first_ts) / 86400).max(1) as usize;
    days
}

fn compute_max_drawdown(equity_curve: &[(i64, f64)]) -> f64 {
    let mut peak = f64::MIN;
    let mut max_dd = 0.0_f64;
    for &(_, val) in equity_curve {
        if val > peak {
            peak = val;
        }
        if peak > 0.0 {
            let dd = (peak - val) / peak;
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }
    max_dd
}

fn compute_trade_stats(trades: &[BacktestTrade]) -> (f64, f64) {
    let total = trades.len();
    if total == 0 {
        return (0.0, 0.0);
    }
    let wins: Vec<f64> = trades
        .iter()
        .filter(|t| t.realized_pnl > 0.0)
        .map(|t| t.realized_pnl)
        .collect();
    let losses: Vec<f64> = trades
        .iter()
        .filter(|t| t.realized_pnl < 0.0)
        .map(|t| t.realized_pnl.abs())
        .collect();

    let win_rate = wins.len() as f64 / total as f64;
    let profit_factor = if losses.is_empty() {
        if wins.is_empty() {
            0.0
        } else {
            f64::INFINITY
        }
    } else {
        let total_wins: f64 = wins.iter().sum();
        let total_losses: f64 = losses.iter().sum();
        if total_losses > 0.0 {
            total_wins / total_losses
        } else {
            f64::INFINITY
        }
    };

    // Clamp INFINITY to a large number for JSON serialization
    let profit_factor = if profit_factor.is_infinite() {
        f64::MAX
    } else {
        profit_factor
    };

    (win_rate, profit_factor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_drawdown() {
        let curve = vec![
            (0, 1.0),
            (1, 1.2),
            (2, 0.8),
            (3, 0.9),
            (4, 1.1),
        ];
        // Peak=1.2 at bar 1, trough=0.8 at bar 2 → dd = (1.2-0.8)/1.2 = 0.3333
        let dd = compute_max_drawdown(&curve);
        assert!((dd - 0.3333333).abs() < 0.001);
    }

    #[test]
    fn test_trade_stats() {
        let trades = vec![
            BacktestTrade {
                entry_ts: 0,
                exit_ts: Some(1),
                side: "long".into(),
                entry_price: 100.0,
                exit_price: Some(110.0),
                realized_pnl: 10.0,
            },
            BacktestTrade {
                entry_ts: 2,
                exit_ts: Some(3),
                side: "long".into(),
                entry_price: 110.0,
                exit_price: Some(105.0),
                realized_pnl: -5.0,
            },
            BacktestTrade {
                entry_ts: 4,
                exit_ts: Some(5),
                side: "long".into(),
                entry_price: 105.0,
                exit_price: Some(115.0),
                realized_pnl: 10.0,
            },
        ];
        let (win_rate, pf) = compute_trade_stats(&trades);
        assert!((win_rate - 2.0 / 3.0).abs() < 0.001);
        assert!((pf - 20.0 / 5.0).abs() < 0.001);
    }

    #[test]
    fn test_empty_curve() {
        let m = compute(&[], &[], 0.0);
        assert_eq!(m.cagr, 0.0);
        assert_eq!(m.sharpe, 0.0);
        assert_eq!(m.trade_count, 0);
    }
}