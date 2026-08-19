//! Candidate store — versioned parameter snapshots with backtest reports
//!
//! Immutable storage for optimization candidates. Each candidate gets a
//! unique version ID and stores parameters + backtest metrics.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Candidate status in the store
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateStatus {
    /// Newly optimized, not yet validated
    New,
    /// Passed stability check
    Stable,
    /// Failed stability check
    Unstable,
    /// Running in paper mode
    Paper,
    /// Running in micro mode
    Micro,
    /// Promoted to live
    Live,
    /// Retired (replaced or underperformed)
    Retired,
}

/// Candidate snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateSnapshot {
    pub version_id: String,
    pub params: HashMap<String, f64>,
    pub status: CandidateStatus,
    pub mean_ic: f64,
    pub std_ic: f64,
    pub n_trades: usize,
    pub fold_ics: Vec<f64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Candidate store
pub struct CandidateStore {
    candidates: Arc<RwLock<HashMap<String, CandidateSnapshot>>>,
    counter: Arc<RwLock<u64>>,
}

impl CandidateStore {
    pub fn new() -> Self {
        Self {
            candidates: Arc::new(RwLock::new(HashMap::new())),
            counter: Arc::new(RwLock::new(0)),
        }
    }

    /// Generate version ID
    fn generate_version_id(&self, timestamp: chrono::DateTime<chrono::Utc>) -> String {
        let mut counter = self.counter.write().unwrap();
        *counter += 1;
        format!("v{}_{}", timestamp.format("%Y%m%d_%H%M%S"), counter)
    }

    /// Store a new candidate
    pub fn store(&self, params: HashMap<String, f64>, mean_ic: f64, std_ic: f64, n_trades: usize, fold_ics: Vec<f64>) -> String {
        let now = chrono::Utc::now();
        let version_id = self.generate_version_id(now);

        let snapshot = CandidateSnapshot {
            version_id: version_id.clone(),
            params,
            status: CandidateStatus::New,
            mean_ic,
            std_ic,
            n_trades,
            fold_ics,
            created_at: now,
            updated_at: now,
        };

        let mut candidates = self.candidates.write().unwrap();
        candidates.insert(version_id.clone(), snapshot);
        version_id
    }

    /// Get candidate by version ID
    pub fn get(&self, version_id: &str) -> Option<CandidateSnapshot> {
        let candidates = self.candidates.read().unwrap();
        candidates.get(version_id).cloned()
    }

    /// Update candidate status
    pub fn update_status(&self, version_id: &str, status: CandidateStatus) -> bool {
        let mut candidates = self.candidates.write().unwrap();
        if let Some(snapshot) = candidates.get_mut(version_id) {
            snapshot.status = status;
            snapshot.updated_at = chrono::Utc::now();
            true
        } else {
            false
        }
    }

    /// List all candidates
    pub fn list(&self) -> Vec<CandidateSnapshot> {
        let candidates = self.candidates.read().unwrap();
        candidates.values().cloned().collect()
    }

    /// List candidates by status
    pub fn list_by_status(&self, status: CandidateStatus) -> Vec<CandidateSnapshot> {
        let candidates = self.candidates.read().unwrap();
        candidates.values()
            .filter(|c| c.status == status)
            .cloned()
            .collect()
    }

    /// Get best candidate by mean IC
    pub fn get_best(&self) -> Option<CandidateSnapshot> {
        let candidates = self.candidates.read().unwrap();
        candidates.values()
            .max_by(|a, b| a.mean_ic.partial_cmp(&b.mean_ic).unwrap())
            .cloned()
    }
}

impl Default for CandidateStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_candidate() {
        let store = CandidateStore::new();
        let mut params = HashMap::new();
        params.insert("threshold".to_string(), 0.5);

        let version_id = store.store(params.clone(), 0.05, 0.01, 150, vec![0.04, 0.05, 0.06]);
        
        let snapshot = store.get(&version_id).unwrap();
        assert_eq!(snapshot.params, params);
        assert_eq!(snapshot.mean_ic, 0.05);
        assert_eq!(snapshot.status, CandidateStatus::New);
    }

    #[test]
    fn test_update_status() {
        let store = CandidateStore::new();
        let params = HashMap::new();
        let version_id = store.store(params, 0.05, 0.01, 150, vec![0.05]);

        assert!(store.update_status(&version_id, CandidateStatus::Stable));
        
        let snapshot = store.get(&version_id).unwrap();
        assert_eq!(snapshot.status, CandidateStatus::Stable);
    }

    #[test]
    fn test_list_by_status() {
        let store = CandidateStore::new();
        
        store.store(HashMap::new(), 0.05, 0.01, 150, vec![0.05]);
        store.store(HashMap::new(), 0.06, 0.01, 150, vec![0.06]);
        
        let new_candidates = store.list_by_status(CandidateStatus::New);
        assert_eq!(new_candidates.len(), 2);
        
        let stable_candidates = store.list_by_status(CandidateStatus::Stable);
        assert_eq!(stable_candidates.len(), 0);
    }

    #[test]
    fn test_get_best() {
        let store = CandidateStore::new();
        
        store.store(HashMap::new(), 0.05, 0.01, 150, vec![0.05]);
        store.store(HashMap::new(), 0.08, 0.01, 150, vec![0.08]);
        store.store(HashMap::new(), 0.06, 0.01, 150, vec![0.06]);
        
        let best = store.get_best().unwrap();
        assert_eq!(best.mean_ic, 0.08);
    }

    #[test]
    fn test_list_all() {
        let store = CandidateStore::new();
        
        store.store(HashMap::new(), 0.05, 0.01, 150, vec![0.05]);
        store.store(HashMap::new(), 0.06, 0.01, 150, vec![0.06]);
        
        let all = store.list();
        assert_eq!(all.len(), 2);
    }
}
