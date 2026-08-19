//! Staged exit ladder
//!
//! Three-stage degrade path for option exits:
//! - Stage 1: BID + k×tick (3s timer)
//! - Stage 2: BID (3s timer)
//! - Stage 3: BID - max_slippage (10s timer)
//!
//! Critic fix: partial fill on Stage 3 → loop back to Stage 1 with fresh BID

#[cfg(test)]
mod tests;

/// Staged exit ladder configuration
pub struct StagedLadder {
    /// Multiplier for tick in Stage 1 (k)
    k: f64,
    /// Maximum slippage budget for Stage 3
    max_slippage: f64,
}

impl StagedLadder {
    /// Create a new staged ladder
    ///
    /// # Arguments
    /// * `k` - Multiplier for tick in Stage 1 (e.g., 0.05 means 5% of tick)
    /// * `max_slippage` - Maximum slippage budget for Stage 3 (absolute price)
    pub fn new(k: f64, max_slippage: f64) -> Self {
        Self { k, max_slippage }
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
            1 => bid + self.k * 0.01, // k×tick (assuming tick = 0.01)
            2 => bid,                   // BID
            3 => bid - self.max_slippage, // BID - max_slippage
            _ => bid,                   // Fallback to BID
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
            1 => 3,  // Stage 1: 3s
            2 => 3,  // Stage 2: 3s
            3 => 10, // Stage 3: 10s
            _ => 0,
        }
    }
}
