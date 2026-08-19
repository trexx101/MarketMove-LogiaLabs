//! Black-Scholes-Merton option pricing (P2)
//!
//! Generates synthetic option premiums from underlying OHLCV + constant IV.
//! Used by the optimizer when historical options data is unavailable (D1).

use statrs::distribution::{Continuous, ContinuousCDF, Normal};

/// Option type: call or put
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OptionType {
    Call,
    Put,
}

/// Option contract specification
#[derive(Debug, Clone)]
pub struct OptionSpec {
    pub underlying_price: f64,
    pub strike: f64,
    pub time_to_expiry_years: f64,
    pub risk_free_rate: f64,
    pub volatility: f64,
    pub option_type: OptionType,
}

/// Option price and greeks
#[derive(Debug, Clone)]
pub struct OptionPrice {
    pub price: f64,
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub vega: f64,
}

/// Calculate option price and greeks using Black-Scholes-Merton
pub fn price_option(spec: &OptionSpec) -> OptionPrice {
    let S = spec.underlying_price;
    let K = spec.strike;
    let T = spec.time_to_expiry_years;
    let r = spec.risk_free_rate;
    let sigma = spec.volatility;

    // Edge case: expired option
    if T <= 0.0 {
        let intrinsic = match spec.option_type {
            OptionType::Call => (S - K).max(0.0),
            OptionType::Put => (K - S).max(0.0),
        };
        return OptionPrice {
            price: intrinsic,
            delta: 0.0,
            gamma: 0.0,
            theta: 0.0,
            vega: 0.0,
        };
    }

    let sqrt_T = T.sqrt();
    let d1 = ((S / K).ln() + (r + 0.5 * sigma * sigma) * T) / (sigma * sqrt_T);
    let d2 = d1 - sigma * sqrt_T;

    let normal = Normal::new(0.0, 1.0).unwrap();
    let n_d1 = normal.pdf(d1);
    let n_d2 = normal.cdf(d2);
    let n_neg_d1 = normal.cdf(-d1);
    let n_neg_d2 = normal.cdf(-d2);

    let (price, delta, theta) = match spec.option_type {
        OptionType::Call => {
            let price = S * normal.cdf(d1) - K * (-r * T).exp() * normal.cdf(d2);
            let delta = normal.cdf(d1);
            let theta = -(S * n_d1 * sigma) / (2.0 * sqrt_T)
                - r * K * (-r * T).exp() * normal.cdf(d2);
            (price, delta, theta)
        }
        OptionType::Put => {
            let price = K * (-r * T).exp() * n_neg_d2 - S * n_neg_d1;
            let delta = -n_neg_d1;
            let theta = -(S * n_d1 * sigma) / (2.0 * sqrt_T)
                + r * K * (-r * T).exp() * n_neg_d2;
            (price, delta, theta)
        }
    };

    let gamma = n_d1 / (S * sigma * sqrt_T);
    let vega = S * n_d1 * sqrt_T;

    OptionPrice {
        price,
        delta,
        gamma,
        theta: theta / 365.0, // Convert to per-day
        vega: vega / 100.0,   // Convert to per 1% vol change
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_price_atm() {
        let spec = OptionSpec {
            underlying_price: 100.0,
            strike: 100.0,
            time_to_expiry_years: 1.0,
            risk_free_rate: 0.05,
            volatility: 0.2,
            option_type: OptionType::Call,
        };
        let result = price_option(&spec);
        // ATM call with 20% vol, 1 year, 5% rate should be ~$10.45
        assert!((result.price - 10.45).abs() < 0.1);
        assert!(result.delta > 0.5 && result.delta < 0.7);
    }

    #[test]
    fn test_put_price_atm() {
        let spec = OptionSpec {
            underlying_price: 100.0,
            strike: 100.0,
            time_to_expiry_years: 1.0,
            risk_free_rate: 0.05,
            volatility: 0.2,
            option_type: OptionType::Put,
        };
        let result = price_option(&spec);
        // ATM put should be ~$5.57
        assert!((result.price - 5.57).abs() < 0.1);
        assert!(result.delta < -0.3 && result.delta > -0.5);
    }

    #[test]
    fn test_expired_option() {
        let spec = OptionSpec {
            underlying_price: 105.0,
            strike: 100.0,
            time_to_expiry_years: 0.0,
            risk_free_rate: 0.05,
            volatility: 0.2,
            option_type: OptionType::Call,
        };
        let result = price_option(&spec);
        assert_eq!(result.price, 5.0); // Intrinsic value
        assert_eq!(result.delta, 0.0);
    }

    #[test]
    fn test_deep_itm_call() {
        let spec = OptionSpec {
            underlying_price: 150.0,
            strike: 100.0,
            time_to_expiry_years: 0.5,
            risk_free_rate: 0.05,
            volatility: 0.2,
            option_type: OptionType::Call,
        };
        let result = price_option(&spec);
        assert!(result.delta > 0.95); // Deep ITM ≈ 1.0
        assert!(result.price > 45.0); // Close to intrinsic
    }
}
