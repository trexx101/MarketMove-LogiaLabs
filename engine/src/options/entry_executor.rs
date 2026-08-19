//! Entry executor for options
//!
//! 2-stage entry ladder (entries are not emergencies):
//! - Stage 1: Limit at ask (3s timer)
//! - Stage 2: Limit at ask + slippage (10s timer)
//!
//! Unfilled entry simply cancels after Stage 2.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Entry stage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryStage {
    Stage1,
    Stage2,
    Cancelled,
    Filled,
}

/// Entry executor state
#[derive(Debug, Clone)]
pub struct EntryExecutor {
    pub position_id: i64,
    pub current_stage: EntryStage,
    pub stage_start_time: DateTime<Utc>,
    pub ask_price: f64,
    pub slippage_budget: f64,
}

impl EntryExecutor {
    /// Create a new entry executor
    pub fn new(position_id: i64, ask_price: f64, slippage_budget: f64) -> Self {
        Self {
            position_id,
            current_stage: EntryStage::Stage1,
            stage_start_time: Utc::now(),
            ask_price,
            slippage_budget,
        }
    }

    /// Get the current limit price
    pub fn current_limit_price(&self) -> f64 {
        match self.current_stage {
            EntryStage::Stage1 => self.ask_price,
            EntryStage::Stage2 => self.ask_price + self.slippage_budget,
            EntryStage::Cancelled | EntryStage::Filled => 0.0,
        }
    }

    /// Check if we should advance to the next stage
    pub fn should_advance(&self, now: DateTime<Utc>) -> bool {
        let elapsed = now - self.stage_start_time;
        let duration = self.stage_duration();
        elapsed >= duration
    }

    /// Advance to the next stage
    pub fn advance(&mut self) {
        self.stage_start_time = Utc::now();
        self.current_stage = match self.current_stage {
            EntryStage::Stage1 => EntryStage::Stage2,
            EntryStage::Stage2 => EntryStage::Cancelled,
            EntryStage::Cancelled | EntryStage::Filled => self.current_stage,
        };
    }

    /// Mark as filled
    pub fn mark_filled(&mut self) {
        self.current_stage = EntryStage::Filled;
    }

    /// Cancel the entry
    pub fn cancel(&mut self) {
        self.current_stage = EntryStage::Cancelled;
    }

    /// Get the duration for the current stage
    fn stage_duration(&self) -> Duration {
        match self.current_stage {
            EntryStage::Stage1 => Duration::seconds(3),
            EntryStage::Stage2 => Duration::seconds(10),
            EntryStage::Cancelled | EntryStage::Filled => Duration::zero(),
        }
    }
}

/// Entry execution result
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntryResult {
    Filled { price: f64, stage: EntryStage },
    Pending { stage: EntryStage, limit_price: f64 },
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stage1_limit_price() {
        let executor = EntryExecutor::new(1, 5.0, 0.10);
        assert_eq!(executor.current_limit_price(), 5.0);
        assert_eq!(executor.current_stage, EntryStage::Stage1);
    }

    #[test]
    fn test_stage2_limit_price() {
        let mut executor = EntryExecutor::new(1, 5.0, 0.10);
        executor.advance();
        assert_eq!(executor.current_limit_price(), 5.10);
        assert_eq!(executor.current_stage, EntryStage::Stage2);
    }

    #[test]
    fn test_cancel_after_stage2() {
        let mut executor = EntryExecutor::new(1, 5.0, 0.10);
        executor.advance(); // Stage 1 → Stage 2
        executor.advance(); // Stage 2 → Cancelled
        assert_eq!(executor.current_stage, EntryStage::Cancelled);
    }

    #[test]
    fn test_mark_filled() {
        let mut executor = EntryExecutor::new(1, 5.0, 0.10);
        executor.mark_filled();
        assert_eq!(executor.current_stage, EntryStage::Filled);
    }

    #[test]
    fn test_should_advance_respects_duration() {
        let executor = EntryExecutor::new(1, 5.0, 0.10);
        let now = Utc::now();

        // Not enough time passed
        assert!(!executor.should_advance(now));

        // Wait 4 seconds (Stage 1 duration is 3s)
        let later = now + Duration::seconds(4);
        assert!(executor.should_advance(later));
    }

    #[test]
    fn test_stage2_longer_duration() {
        let mut executor = EntryExecutor::new(1, 5.0, 0.10);
        executor.advance(); // Now in Stage 2
        let now = Utc::now();

        // Stage 2 duration is 10s, so 5s should not advance
        let five_sec_later = now + Duration::seconds(5);
        assert!(!executor.should_advance(five_sec_later));

        // 11s should advance
        let eleven_sec_later = now + Duration::seconds(11);
        assert!(executor.should_advance(eleven_sec_later));
    }
}
