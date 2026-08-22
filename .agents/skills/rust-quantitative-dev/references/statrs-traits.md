# statrs Crate Trait Imports

The `statrs` crate (v0.17+) requires explicit trait imports for probability distribution methods. The trait names are non-obvious.

## Correct Imports

```rust
// For PDF (probability density function) and CDF (cumulative distribution function):
use statrs::distribution::{Continuous, ContinuousCDF, Normal};

// Usage:
let normal = Normal::new(0.0, 1.0).unwrap();
let pdf_val = normal.pdf(0.5);   // from Continuous trait
let cdf_val = normal.cdf(0.5);   // from ContinuousCDF trait
```

## Common Mistakes

**Wrong**: Importing `PDF` trait
```rust
use statrs::distribution::{Normal, PDF}; // ❌ PDF trait doesn't exist
```

**Wrong**: Importing `ProbabilityDensityFunction`
```rust
use statrs::distribution::{Normal, ProbabilityDensityFunction}; // ❌ doesn't exist
```

**Wrong**: Expecting methods without trait import
```rust
use statrs::distribution::Normal;
let normal = Normal::new(0.0, 1.0).unwrap();
let pdf_val = normal.pdf(0.5); // ❌ won't compile without Continuous trait
```

## Trait Mapping

| Method | Required Trait | Notes |
|--------|---------------|-------|
| `pdf(x)` | `Continuous` | Probability density function |
| `cdf(x)` | `ContinuousCDF` | Cumulative distribution function |
| `inverse_cdf(p)` | `ContinuousCDF` | Quantile function |
| `mean()` | `Mean<f64>` | Distribution mean |
| `variance()` | `Variance<f64>` | Distribution variance |

## Pattern for Options Pricing

```rust
use statrs::distribution::{Continuous, ContinuousCDF, Normal};

pub fn price_option(s: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64 {
    let normal = Normal::new(0.0, 1.0).unwrap();
    let d1 = ((s / k).ln() + (r + 0.5 * sigma * sigma) * t) / (sigma * t.sqrt());
    let d2 = d1 - sigma * t.sqrt();
    
    // Call option
    s * normal.cdf(d1) - k * (-r * t).exp() * normal.cdf(d2)
}
```

## Version Compatibility

Tested with `statrs = "0.17"`. Earlier versions (0.16, 0.15) may have different trait names.
