---
name: marketmoves-strategy-findings
description: Strategy backtest findings for MarketMoves equities.
category: project
---

# MarketMoves Strategy Findings

## Threshold Strategy (Dominant Config)

**2026-07-30 live backtest — 895 days (2022-01 to 2025-07):**

| Config | Trades | Long/Short | CAGR | Sharpe | Max DD | Win Rate | PF |
|--------|--------|------------|------|--------|-------|----------|-----|
| SMA=200, entry=0.003 | 10 | all long | 10.9% | 1.05 | 22.9% | 80% | 5.38 |
| SMA=40, entry=0.001, pred_5d_filter=true | 24 | 24/0 | 18.0% | 1.41 | 22.9% | 83% | 5.85 |
| **SMA=40, pred_5d_filter=false** | **25** | **25/0** | **20.2%** | **1.43** | **~22%** | **80%** | **—** |

**Dominant config:** SMA=40, `pred_5d_filter: false` — highest Sharpe, highest CAGR, most trades, lowest max drawdown.

## Rhai Mean-Reversion Script (Not Recommended)

**Script:** `engine/scripts/mean_reversion_qqq.rhai` — 36 trades, 11.3% CAGR, 0.93 Sharpe, 67% WR. Short side never fired. Script preserved for reference only; threshold config dominates on every risk-adjusted metric.

## Sentiment Risk Overlay (2026-08-05)

Implemented as a per-model strategy overlay. Default disabled. Locked defaults:

| Parameter | Default | Rule |
|-----------|---------|------|
| `enable_sentiment_overlay` | false | toggle |
| `sentiment_reduce_threshold` | -0.5 | block new entries when score below this |
| `sentiment_exit_threshold` | -0.8 | force Flat when score below this |
| `sentiment_min_articles` | 15 | overlay ignored below this article count |

**Design deviation:** original plan proposed a `f64` size multiplier to halve position size on moderate negative sentiment. Rejected as invasive because the executor's position size is fixed at construction. Adopted cleaner interpretation: force-exit + block-entries only. Existing positions are preserved in the moderate zone; only new entries are blocked.

Runtime integration: scheduler fetches Finnhub `news-sentiment` after each prediction cycle, reads latest cached `(score, buzz)`, then `apply_sentiment_overlay(raw_position, score, buzz, params)` produces the final target position.
- BTC TCN: 4 Colab runs, commit 40f4f10, walk-forward OOS IC ≈ 0 — no alpha
- Deploy gate blocked it correctly
