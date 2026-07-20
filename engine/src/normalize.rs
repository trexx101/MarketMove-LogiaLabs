use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::features::FeatureRow;
use crate::features::core::{FeatureRowV2, FEATURE_DIM};

/// Global z-score statistics for the three model features:
/// index 0 → `log_return`, 1 → `atr_72`, 2 → `vwap_dev`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormStats {
    pub mean: [f64; 3],
    pub std: [f64; 3],
}

impl NormStats {
    /// Load `NormStats` from a JSON file.
    ///
    /// Expected shape: `{"mean": [f64, f64, f64], "std": [f64, f64, f64]}`
    pub fn load(path: &str) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading norm stats file: {path}"))?;
        let stats: NormStats = serde_json::from_str(&text)
            .with_context(|| format!("parsing norm stats JSON from: {path}"))?;
        Ok(stats)
    }
}

/// Apply global z-score normalization to a single `FeatureRow`.
///
/// Feature order: `[log_return, atr_72, vwap_dev]`.
/// If `stats.std[i] == 0.0` the corresponding output element is `0.0`.
pub fn normalize_row(feat: &FeatureRow, stats: &NormStats) -> [f64; 3] {
    let raw = [feat.log_return, feat.atr_72, feat.vwap_dev];
    let mut out = [0.0_f64; 3];
    for i in 0..3 {
        out[i] = if stats.std[i] == 0.0 {
            0.0
        } else {
            (raw[i] - stats.mean[i]) / stats.std[i]
        };
    }
    out
}

// ---------------------------------------------------------------------------
// V2 normalization (Wave 5) — schema_versioned, backward-compatible.
// ---------------------------------------------------------------------------

fn default_schema_version() -> u32 { 1 }

/// V2 norm stats: supports both v1 (3 features, no schema field) and v2
/// (FEATURE_DIM features, schema_version=2). `mean`/`std` are `Vec` to handle
/// variable dimensionality at deserialization time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormStatsV2 {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub mean: Vec<f64>,
    pub std: Vec<f64>,
}

impl NormStatsV2 {
    /// Load V2 norm stats from a JSON file. Accepts both v1 (3-feature) and
    /// v2 (N-feature) shapes; validates dimensionality against `FEATURE_DIM`.
    pub fn load(path: &str) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading norm stats v2 file: {path}"))?;
        let stats: NormStatsV2 = serde_json::from_str(&text)
            .with_context(|| format!("parsing norm stats v2 JSON from: {path}"))?;

        if stats.schema_version == 1 {
            if stats.mean.len() != 3 || stats.std.len() != 3 {
                anyhow::bail!("v1 schema requires exactly 3 features, got {}", stats.mean.len());
            }
        } else if stats.schema_version == 2 {
            if stats.mean.len() != FEATURE_DIM || stats.std.len() != FEATURE_DIM {
                anyhow::bail!(
                    "v2 schema requires exactly {} features, got {}",
                    FEATURE_DIM,
                    stats.mean.len()
                );
            }
        } else {
            anyhow::bail!("unsupported schema_version: {}", stats.schema_version);
        }

        Ok(stats)
    }

    /// Normalize a V2 feature row into the fixed-size array for inference.
    pub fn normalize_row_v2(&self, feat: &FeatureRowV2) -> Result<[f64; FEATURE_DIM]> {
        if self.schema_version != 2 {
            anyhow::bail!("attempted to normalize V2 features with non-v2 stats");
        }
        let raw = feat.to_array();
        let mut out = [0.0_f64; FEATURE_DIM];
        for i in 0..FEATURE_DIM {
            let s = if self.std[i] == 0.0 { 1.0 } else { self.std[i] };
            out[i] = (raw[i] - self.mean[i]) / s;
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn feat(log_return: f64, atr_72: f64, vwap_dev: f64) -> FeatureRow {
        FeatureRow { log_return, atr_72, vwap_dev }
    }

    #[test]
    fn normalize_row_identity() {
        let stats = NormStats {
            mean: [0.0, 0.0, 0.0],
            std: [1.0, 1.0, 1.0],
        };
        let f = feat(0.5, 2.0, -0.1);
        let out = normalize_row(&f, &stats);
        assert!((out[0] - 0.5).abs() < 1e-12);
        assert!((out[1] - 2.0).abs() < 1e-12);
        assert!((out[2] - (-0.1)).abs() < 1e-12);
    }

    #[test]
    fn normalize_row_scales_correctly() {
        let stats = NormStats {
            mean: [0.001, 0.005, 0.0],
            std: [0.002, 0.003, 0.01],
        };
        let f = feat(0.003, 0.011, -0.02);

        // Manual: z[0] = (0.003 - 0.001) / 0.002 = 1.0
        //         z[1] = (0.011 - 0.005) / 0.003 = 2.0
        //         z[2] = (-0.02 - 0.0)  / 0.01   = -2.0
        let out = normalize_row(&f, &stats);
        assert!((out[0] - 1.0).abs() < 1e-9, "z[0] expected 1.0, got {}", out[0]);
        assert!((out[1] - 2.0).abs() < 1e-9, "z[1] expected 2.0, got {}", out[1]);
        assert!((out[2] - (-2.0)).abs() < 1e-9, "z[2] expected -2.0, got {}", out[2]);
    }

    #[test]
    fn normalize_row_std_zero_returns_zero() {
        let stats = NormStats {
            mean: [1.0, 1.0, 1.0],
            std: [0.0, 1.0, 0.0],
        };
        let f = feat(5.0, 3.0, 7.0);
        let out = normalize_row(&f, &stats);
        assert_eq!(out[0], 0.0, "std=0 should yield 0");
        assert!((out[1] - 2.0).abs() < 1e-12);
        assert_eq!(out[2], 0.0, "std=0 should yield 0");
    }

    #[test]
    fn load_roundtrips_json() {
        // Write a temp JSON file, load it, and verify values survive the round-trip.
        let original = NormStats {
            mean: [0.0012, 0.0034, -0.0001],
            std: [0.0056, 0.0078, 0.0009],
        };

        let json = serde_json::to_string(&original).expect("serialize failed");

        let path = std::env::temp_dir().join("norm_stats_test.json");
        std::fs::write(&path, json.as_bytes()).expect("write failed");
        let path_str = path.to_str().expect("temp path to str failed");

        let loaded = NormStats::load(path_str).expect("load failed");
        let _ = std::fs::remove_file(&path); // best-effort cleanup

        for i in 0..3 {
            assert!((loaded.mean[i] - original.mean[i]).abs() < 1e-15, "mean[{i}] mismatch");
            assert!((loaded.std[i] - original.std[i]).abs() < 1e-15, "std[{i}] mismatch");
        }
    }
}
