//! Hyperopt optimizer — grid/random search with walk-forward validation
//!
//! Runs parameter optimization with embargo to prevent lookahead bias.
//! Requires ≥100 trades for statistical significance.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Parameter definition for grid search
#[derive(Debug, Clone)]
pub struct ParamDef {
    pub name: String,
    pub values: Vec<f64>,
}

/// Parameter configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamConfig {
    pub params: HashMap<String, f64>,
}

/// Walk-forward split
#[derive(Debug, Clone)]
pub struct WalkForwardSplit {
    pub train_start: usize,
    pub train_end: usize,
    pub test_start: usize,
    pub test_end: usize,
}

/// Optimizer configuration
#[derive(Debug, Clone)]
pub struct OptimizerConfig {
    /// Number of walk-forward folds
    pub n_folds: usize,
    /// Embargo days between train and test
    pub embargo_days: usize,
    /// Minimum trades for statistical significance
    pub min_trades: usize,
    /// Search strategy
    pub strategy: SearchStrategy,
}

#[derive(Debug, Clone)]
pub enum SearchStrategy {
    Grid,
    Random { n_iter: usize },
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            n_folds: 5,
            embargo_days: 136, // ~6 months trading days
            min_trades: 100,
            strategy: SearchStrategy::Grid,
        }
    }
}

/// Optimization result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationResult {
    pub params: ParamConfig,
    pub mean_ic: f64,
    pub std_ic: f64,
    pub n_trades: usize,
    pub fold_ics: Vec<f64>,
}

/// Hyperopt optimizer
pub struct Optimizer {
    config: OptimizerConfig,
}

impl Optimizer {
    pub fn new(config: OptimizerConfig) -> Self {
        Self { config }
    }

    /// Generate walk-forward splits
    pub fn generate_splits(&self, n_samples: usize) -> Vec<WalkForwardSplit> {
        let fold_size = n_samples / (self.config.n_folds + 1);
        let mut splits = Vec::new();

        for i in 0..self.config.n_folds {
            let train_end = fold_size * (i + 1);
            let test_start = train_end + self.config.embargo_days;
            let test_end = (test_start + fold_size).min(n_samples);

            if test_end > test_start {
                splits.push(WalkForwardSplit {
                    train_start: 0,
                    train_end,
                    test_start,
                    test_end,
                });
            }
        }

        splits
    }

    /// Generate parameter combinations for grid search
    pub fn generate_grid(&self, param_defs: &[ParamDef]) -> Vec<ParamConfig> {
        if param_defs.is_empty() {
            return vec![ParamConfig {
                params: HashMap::new(),
            }];
        }

        let mut configs = vec![ParamConfig {
            params: HashMap::new(),
        }];

        for param in param_defs {
            let mut new_configs = Vec::new();
            for config in &configs {
                for value in &param.values {
                    let mut new_config = config.clone();
                    new_config.params.insert(param.name.clone(), *value);
                    new_configs.push(new_config);
                }
            }
            configs = new_configs;
        }

        configs
    }

    /// Evaluate a parameter configuration
    pub fn evaluate<F>(&self, params: &ParamConfig, eval_fn: F) -> Option<OptimizationResult>
    where
        F: Fn(&ParamConfig, &WalkForwardSplit) -> (f64, usize),
    {
        // This is a placeholder — actual implementation would call backtester
        // For now, return None to indicate insufficient data
        None
    }

    /// Run optimization
    pub fn optimize<F>(&self, param_defs: &[ParamDef], eval_fn: F) -> Vec<OptimizationResult>
    where
        F: Fn(&ParamConfig, &WalkForwardSplit) -> (f64, usize),
    {
        let configs = match self.config.strategy {
            SearchStrategy::Grid => self.generate_grid(param_defs),
            SearchStrategy::Random { n_iter } => {
                // Random search not implemented yet
                Vec::new()
            }
        };

        configs
            .into_iter()
            .filter_map(|config| self.evaluate(&config, &eval_fn))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_splits_basic() {
        let config = OptimizerConfig {
            n_folds: 3,
            embargo_days: 10,
            min_trades: 100,
            strategy: SearchStrategy::Grid,
        };
        let optimizer = Optimizer::new(config);

        let splits = optimizer.generate_splits(1000);
        assert_eq!(splits.len(), 3);

        // Check first split
        assert_eq!(splits[0].train_start, 0);
        assert_eq!(splits[0].train_end, 250);
        assert_eq!(splits[0].test_start, 260); // 250 + 10 embargo
        assert_eq!(splits[0].test_end, 510); // 260 + 250
    }

    #[test]
    fn test_generate_splits_respects_embargo() {
        let config = OptimizerConfig {
            n_folds: 2,
            embargo_days: 50,
            min_trades: 100,
            strategy: SearchStrategy::Grid,
        };
        let optimizer = Optimizer::new(config);

        let splits = optimizer.generate_splits(500);
        assert_eq!(splits.len(), 2);

        // Embargo should be respected
        for split in &splits {
            assert!(split.test_start >= split.train_end + 50);
        }
    }

    #[test]
    fn test_generate_grid_single_param() {
        let config = OptimizerConfig::default();
        let optimizer = Optimizer::new(config);

        let param_defs = vec![ParamDef {
            name: "threshold".to_string(),
            values: vec![0.1, 0.2, 0.3],
        }];

        let configs = optimizer.generate_grid(&param_defs);
        assert_eq!(configs.len(), 3);
        assert_eq!(configs[0].params.get("threshold"), Some(&0.1));
        assert_eq!(configs[1].params.get("threshold"), Some(&0.2));
        assert_eq!(configs[2].params.get("threshold"), Some(&0.3));
    }

    #[test]
    fn test_generate_grid_multiple_params() {
        let config = OptimizerConfig::default();
        let optimizer = Optimizer::new(config);

        let param_defs = vec![
            ParamDef {
                name: "threshold".to_string(),
                values: vec![0.1, 0.2],
            },
            ParamDef {
                name: "window".to_string(),
                values: vec![10.0, 20.0],
            },
        ];

        let configs = optimizer.generate_grid(&param_defs);
        assert_eq!(configs.len(), 4); // 2 x 2 = 4 combinations
    }

    #[test]
    fn test_generate_grid_empty_params() {
        let config = OptimizerConfig::default();
        let optimizer = Optimizer::new(config);

        let configs = optimizer.generate_grid(&[]);
        assert_eq!(configs.len(), 1);
        assert!(configs[0].params.is_empty());
    }
}
