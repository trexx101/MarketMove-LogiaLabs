//! Stateful staged exit ladder
//!
//! Wraps the price calculator with state tracking for position exits.
//! `position_id` is the UUID TEXT primary key of `option_positions`
//! (not an integer — the schema migration `migrate_option_positions`
//! rebuilds INTEGER id columns to TEXT).

use chrono::{DateTime, Duration, Utc};

/// Exit stage enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStage {
    Stage1,
    Stage2,
    Stage3,
    Complete,
}

/// Stateful ladder for a single position exit
#[derive(Debug, Clone)]
pub struct StagedExitLadder {
    position_id: String,
    current_stage: ExitStage,
    stage_start_time: DateTime<Utc>,
    current_bid: f64,
    tick_size: f64,
    k: f64,
    max_slippage: f64,
}

impl StagedExitLadder {
    /// Create a new ladder for a position
    pub fn new(position_id: impl Into<String>) -> Self {
        Self {
            position_id: position_id.into(),
            current_stage: ExitStage::Stage1,
            stage_start_time: Utc::now(),
            current_bid: 0.0,
            tick_size: 0.01,
            k: 2.0,
            max_slippage: 0.10,
        }
    }

    /// Start Stage 1 with initial bid
    pub fn start_stage_1(&mut self, bid: f64, tick_size: f64) {
        self.current_bid = bid;
        self.tick_size = tick_size;
        self.current_stage = ExitStage::Stage1;
        self.stage_start_time = Utc::now();
    }

    /// Get the position ID
    pub fn position_id(&self) -> &str {
        &self.position_id
    }

    /// Get the current stage
    pub fn current_stage(&self) -> ExitStage {
        self.current_stage
    }

    /// Calculate the current limit price
    pub fn current_limit_price(&self) -> f64 {
        match self.current_stage {
            ExitStage::Stage1 => self.current_bid + self.k * self.tick_size,
            ExitStage::Stage2 => self.current_bid,
            ExitStage::Stage3 => self.current_bid - self.max_slippage,
            ExitStage::Complete => 0.0,
        }
    }

    /// Check if we should advance to the next stage
    pub fn should_advance(&self, now: DateTime<Utc>) -> bool {
        let elapsed = now - self.stage_start_time;
        let duration = self.stage_duration();
        elapsed >= duration
    }

    /// Advance to the next stage with fresh bid
    pub fn advance(&mut self, fresh_bid: f64) {
        self.current_bid = fresh_bid;
        self.stage_start_time = Utc::now();

        self.current_stage = match self.current_stage {
            ExitStage::Stage1 => ExitStage::Stage2,
            ExitStage::Stage2 => ExitStage::Stage3,
            ExitStage::Stage3 => ExitStage::Complete,
            ExitStage::Complete => ExitStage::Complete,
        };
    }

    /// Get the duration for the current stage
    fn stage_duration(&self) -> Duration {
        match self.current_stage {
            ExitStage::Stage1 => Duration::seconds(3),
            ExitStage::Stage2 => Duration::seconds(3),
            ExitStage::Stage3 => Duration::seconds(10),
            ExitStage::Complete => Duration::zero(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage1_price_calculation() {
        let mut ladder = StagedExitLadder::new("pos-1");
        ladder.start_stage_1(5.0, 0.05);

        // Stage 1: bid + k*tick = 5.0 + 2.0*0.05 = 5.10
        assert_eq!(ladder.current_limit_price(), 5.10);
    }

    #[test]
    fn stage2_price_calculation() {
        let mut ladder = StagedExitLadder::new("pos-1");
        ladder.start_stage_1(5.0, 0.05);
        ladder.advance(5.05);

        // Stage 2: bid = 5.05
        assert_eq!(ladder.current_limit_price(), 5.05);
    }

    #[test]
    fn stage3_price_calculation() {
        let mut ladder = StagedExitLadder::new("pos-1");
        ladder.start_stage_1(5.0, 0.05);
        ladder.advance(5.05);
        ladder.advance(5.00);

        // Stage 3: bid - max_slippage = 5.00 - 0.10 = 4.90
        assert_eq!(ladder.current_limit_price(), 4.90);
    }

    #[test]
    fn advance_transitions_stages() {
        let mut ladder = StagedExitLadder::new("pos-1");
        ladder.start_stage_1(5.0, 0.05);

        assert_eq!(ladder.current_stage(), ExitStage::Stage1);
        ladder.advance(5.05);
        assert_eq!(ladder.current_stage(), ExitStage::Stage2);
        ladder.advance(5.00);
        assert_eq!(ladder.current_stage(), ExitStage::Stage3);
        ladder.advance(4.95);
        assert_eq!(ladder.current_stage(), ExitStage::Complete);
    }

    #[test]
    fn position_id_is_preserved() {
        let ladder = StagedExitLadder::new("0199e1b0-1234-5678-9abc-def012345678");
        assert_eq!(ladder.position_id(), "0199e1b0-1234-5678-9abc-def012345678");
    }

    #[test]
    fn should_advance_respects_duration() {
        let mut ladder = StagedExitLadder::new("pos-1");
        ladder.start_stage_1(5.0, 0.05);

        // Not enough time passed
        let now = Utc::now();
        assert!(!ladder.should_advance(now));

        // Wait 4 seconds (Stage 1 duration is 3s)
        let later = now + Duration::seconds(4);
        assert!(ladder.should_advance(later));
    }
}
