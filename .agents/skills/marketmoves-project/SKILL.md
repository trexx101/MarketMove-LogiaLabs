---
name: marketmoves-project
description: MarketMoves QQQ equities trading project context
category: project
---

# MarketMoves Project Context

## Overview
- Phases A-E: data → features → model → paper → live
- Deploy gate: IC > 0.03 AND equity > 0 AND parity marker
- Phases 0-4 shipped

## Key Technical Details

### Strategy Parameters
- 2026-07-30: `pred_5d_filter` bool added to `EquityStrategyParams`
- Live backtest (SMA=40, pred_5d_filter=false): 25 trades, 20.2% CAGR, 1.43 Sharpe, 80% WR, 159% total return

### Deployment
- Docker volume: `-v deploy_models:/models:ro` (named vol)
- VPS disk fills fast: `docker system prune -af` before large builds

### Data
- Backfill staleness: threshold param fixed
- Dashboard: 30s `/api/status` polling

### V2 Model Status
- BTC TCN: 4 Colab runs, commit 40f4f10, walk-forward OOS mean IC ≈ 0 — no alpha
- V2 stays DORMANT; deploy gate correctly blocked it

## Verification Philosophy
Paper mode IS the live verification surface — no broker credentials on host during verification phases. Credential absence is the risk boundary, not just code-level gates.
