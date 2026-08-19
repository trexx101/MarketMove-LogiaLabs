//! Staged exit ladder
//!
//! Three-stage degrade path for option exits:
//! - Stage 1: BID + k×tick (stage1_secs timer)
//! - Stage 2: BID (stage2_secs timer)
//! - Stage 3: BID - max_slippage (stage3_secs timer)
//!
//! Critic fix: partial fill on Stage 3 → loop back to Stage 1 with fresh BID
//!
//! Timers are DB-configurable via the options config store; see `StagedLadderConfig`.

pub mod state;

#[cfg(test)]
mod tests;

pub use state::{ExitStage, StagedExitLadder};

/// Config for the staged exit ladder. Defaults match the original
/// hard-coded values (3s/3s/10s stage timers, tick = 0.01).
#[derive(Debug, Clone)]
pub struct StagedLadderConfig {
    pub stage1_secs: u64,
    pub stage2_secs: u64,
    pub stage3_secs: u64,
    /// Assumed tick size for Stage 1 pricing
    pub tick_size: f64,
}

impl Default for StagedLadderConfig {
    fn default() -> Self {
        Self {
            stage1_secs: 3,
            stage2_secs: 3,
            stage3_secs: 10,
            tick_size: 0.01,
        }
    }
}

/// Staged exit ladder configuration
pub struct StagedLadder {
    /// Multiplier for tick in Stage 1 (k)
    k: f64,
    /// Maximum slippage budget for Stage 3
    max_slippage: f64,
    /// Stage timers and tick size
    timers: StagedLadderConfig,
}

impl StagedLadder {
    /// Create a new staged ladder with default timers (3s/3s/10s)
    ///
    /// # Arguments
    /// * `k` - Multiplier for tick in Stage 1 (e.g., 0.05 means 5% of tick)
    /// * `max_slippage` - Maximum slippage budget for Stage 3 (absolute price)
    pub fn new(k: f64, max_slippage: f64) -> Self {
        Self::with_timers(k, max_slippage, StagedLadderConfig::default())
    }

    /// Create a new staged ladder with explicit stage timers (from the options config store)
    pub fn with_timers(k: f64, max_slippage: f64, timers: StagedLadderConfig) -> Self {
        Self { k, max_slippage, timers }
    }

    /// Calculate the limit price for a given stage
    ///
    /// # Arguments
    /// * `stage` - Stage number (1, 2, or 3)
    /// * `bid` - Current bid price
    ///
    /// # Returns
    /// Limit price for the stage
    pub fn stage_price(&self, stage: u8, bid: f64) -> f64 {
        match stage {
            1 => bid + self.k * self.timers.tick_size, // k×tick
            2 => bid,                                  // BID
            3 => bid - self.max_slippage,              // BID - max_slippage
            _ => bid,                                  // Fallback to BID
        }
    }

    /// Get the timer duration for a stage (in seconds)
    ///
    /// # Arguments
    /// * `stage` - Stage number (1, 2, or 3)
    ///
    /// # Returns
    /// Duration in seconds
    pub fn stage_duration(&self, stage: u8) -> u64 {
        match stage {
            1 => self.timers.stage1_secs,
            2 => self.timers.stage2_secs,
            3 => self.timers.stage3_secs,
            _ => 0,
        }
    }
}
