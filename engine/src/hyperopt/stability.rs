//! Neighborhood stability check
//!
//! Validates that champion parameters are robust: small perturbations
//! should not cause large performance drops.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Stability check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilityResult {
    pub stable: bool,
    pub champion_ic: f64,
    pub mean_neighbor_ic: f64,
    pub min_neighbor_ic: f64,
    pub degradation_ratio: f64,
}

/// Neighborhood stability checker
pub struct StabilityChecker {
    /// Maximum allowed degradation ratio (neighbor IC / champion IC)
    pub max_degradation: f64,
    /// Perturbation step size for each parameter
    pub perturbation_step: f64,
}

impl Default for StabilityChecker {
    fn default() -> Self {
        Self {
            max_degradation: 0.7, // Neighbors must retain 70% of champion IC
            perturbation_step: 0.1, // 10% perturbation
        }
    }
}

impl StabilityChecker {
    pub fn new(max_degradation: f64, perturbation_step: f64) -> Self {
        Self {
            max_degradation,
            perturbation_step,
        }
    }

    /// Generate neighbor parameter configurations
    pub fn generate_neighbors(&self, champion: &HashMap<String, f64>) -> Vec<HashMap<String, f64>> {
        let mut neighbors = Vec::new();

        for (param_name, &value) in champion {
            // Generate +perturbation and -perturbation for each parameter
            let up = value * (1.0 + self.perturbation_step);
            let down = value * (1.0 - self.perturbation_step);

            let mut neighbor_up = champion.clone();
            neighbor_up.insert(param_name.clone(), up);
            neighbors.push(neighbor_up);

            let mut neighbor_down = champion.clone();
            neighbor_down.insert(param_name.clone(), down);
            neighbors.push(neighbor_down);
        }

        neighbors
    }

    /// Check stability of champion parameters
    pub fn check<F>(&self, champion: &HashMap<String, f64>, champion_ic: f64, mut eval_fn: F) -> StabilityResult
    where
        F: FnMut(&HashMap<String, f64>) -> f64,
    {
        let neighbors = self.generate_neighbors(champion);
        
        if neighbors.is_empty() {
            return StabilityResult {
                stable: true,
                champion_ic,
                mean_neighbor_ic: champion_ic,
                min_neighbor_ic: champion_ic,
                degradation_ratio: 1.0,
            };
        }

        let neighbor_ics: Vec<f64> = neighbors.iter().map(|n| eval_fn(n)).collect();
        let mean_neighbor_ic = neighbor_ics.iter().sum::<f64>() / neighbor_ics.len() as f64;
        let min_neighbor_ic = neighbor_ics.iter().cloned().fold(f64::INFINITY, f64::min);
        let degradation_ratio = min_neighbor_ic / champion_ic;

        StabilityResult {
            stable: degradation_ratio >= self.max_degradation,
            champion_ic,
            mean_neighbor_ic,
            min_neighbor_ic,
            degradation_ratio,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_neighbors_single_param() {
        let checker = StabilityChecker::default();
        let mut champion = HashMap::new();
        champion.insert("threshold".to_string(), 0.5);

        let neighbors = checker.generate_neighbors(&champion);
        assert_eq!(neighbors.len(), 2); // +10% and -10%

        // Check up perturbation
        assert_eq!(neighbors[0].get("threshold"), Some(&0.55));
        // Check down perturbation
        assert_eq!(neighbors[1].get("threshold"), Some(&0.45));
    }

    #[test]
    fn test_generate_neighbors_multiple_params() {
        let checker = StabilityChecker::default();
        let mut champion = HashMap::new();
        champion.insert("threshold".to_string(), 0.5);
        champion.insert("window".to_string(), 20.0);

        let neighbors = checker.generate_neighbors(&champion);
        assert_eq!(neighbors.len(), 4); // 2 params × 2 directions
    }

    #[test]
    fn test_stability_check_stable() {
        let checker = StabilityChecker::new(0.7, 0.1);
        let mut champion = HashMap::new();
        champion.insert("threshold".to_string(), 0.5);

        let champion_ic = 0.05;
        
        // Mock eval: neighbors perform at 90% of champion
        let eval_fn = |_params: &HashMap<String, f64>| -> f64 { 0.045 };

        let result = checker.check(&champion, champion_ic, eval_fn);
        assert!(result.stable);
        assert!((result.degradation_ratio - 0.9).abs() < 1e-10);
    }

    #[test]
    fn test_stability_check_unstable() {
        let checker = StabilityChecker::new(0.7, 0.1);
        let mut champion = HashMap::new();
        champion.insert("threshold".to_string(), 0.5);

        let champion_ic = 0.05;
        
        // Mock eval: one neighbor drops to 50% of champion
        let mut call_count = 0;
        let eval_fn = |_params: &HashMap<String, f64>| -> f64 {
            call_count += 1;
            if call_count == 1 { 0.025 } else { 0.045 }
        };

        let result = checker.check(&champion, champion_ic, eval_fn);
        assert!(!result.stable);
        assert_eq!(result.degradation_ratio, 0.5);
    }

    #[test]
    fn test_stability_check_empty_params() {
        let checker = StabilityChecker::default();
        let champion = HashMap::new();
        let champion_ic = 0.05;

        let result = checker.check(&champion, champion_ic, |_| 0.05);
        assert!(result.stable);
        assert_eq!(result.degradation_ratio, 1.0);
    }
}
