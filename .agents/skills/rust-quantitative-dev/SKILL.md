---
name: rust-quantitative-dev
description: "Rust quant dev: options pricing, backtesting, stats."
version: 1.0.0
---

# Rust Quantitative Development

Building quantitative and financial applications in Rust: options pricing, backtesting engines, statistical modeling, and numerical computing.

## When to Use

- Implementing Black-Scholes-Merton or other options pricing models
- Building backtesting engines for trading strategies
- Statistical analysis of time series data (returns, volatility, correlations)
- Working with the `statrs` crate for probability distributions
- Numerical computing in Rust (linear algebra, optimization, Monte Carlo)

## Core Patterns

### Statistical Distributions (statrs crate)

The `statrs` crate provides probability distributions, but the trait imports are non-obvious. See `references/statrs-traits.md` for the correct import patterns.

**Key insight**: `pdf()` and `cdf()` methods require importing specific traits, not just the distribution type itself.

### Options Pricing

When implementing Black-Scholes-Merton or similar models:

1. **Edge cases first**: Handle expired options (T ≤ 0) before the main formula
2. **Greeks conversion**: Theta from BSM is per-year; divide by 365 for per-day. Vega is per 1.0 vol change; divide by 100 for per 1% change.
3. **Numerical stability**: Use `statrs::distribution::Normal` for CDF/PDF, not hand-rolled approximations.

### Time Series Data

When working with OHLCV data from databases:

- **Timestamp types vary**: Check the actual struct definition. `ts` might be `i64` (Unix seconds) or `NaiveDateTime`. Don't assume from field names.
- **Conversion helpers**: Write small `ts_to_datetime()` helpers at module scope rather than inline conversions scattered through the code.

## Pitfalls

- **Trait imports**: `statrs` distributions require explicit trait imports for methods like `pdf()`, `cdf()`. The trait name is often not what you'd guess (e.g., `Continuous` not `PDF`).
- **Type mismatches**: Database structs may use different timestamp types than you expect. Always check the actual `FromRow` derive.
- **Test drift**: When changing config defaults, update the corresponding tests. Pre-existing test failures from default changes are common.

## Support Files

- `references/statrs-traits.md` — Correct trait imports for statrs probability distributions
