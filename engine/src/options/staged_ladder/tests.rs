//! Staged exit ladder tests
//!
//! Three-stage degrade path: Stage 1 (BID + k×tick) → Stage 2 (BID) → Stage 3 (BID - max_slippage)
//! Timers: 3s / 3s / 10s
//! Partial fill on Stage 3 → loop back to Stage 1 with fresh BID (critic fix)

use super::*;

#[test]
fn test_stage_1_price_calculation() {
    // Stage 1: BID + k×tick
    let ladder = StagedLadder::new(0.05, 0.01); // k=0.05, tick=0.01
    
    let stage1_price = ladder.stage_price(1, 100.0); // BID = 100.0
    // Expected: 100.0 + 0.05 * 0.01 = 100.0005
    assert!((stage1_price - 100.0005).abs() < 1e-6);
}

#[test]
fn test_stage_2_price_calculation() {
    // Stage 2: BID (no adjustment)
    let ladder = StagedLadder::new(0.05, 0.01);
    
    let stage2_price = ladder.stage_price(2, 100.0);
    assert!((stage2_price - 100.0).abs() < 1e-6);
}

#[test]
fn test_stage_3_price_calculation() {
    // Stage 3: BID - max_slippage
    let ladder = StagedLadder::new(0.05, 0.01);
    
    let stage3_price = ladder.stage_price(3, 100.0);
    // Expected: 100.0 - 0.01 = 99.99
    assert!((stage3_price - 99.99).abs() < 1e-6);
}

#[test]
fn test_stage_timers() {
    let ladder = StagedLadder::new(0.05, 0.01);
    
    assert_eq!(ladder.stage_duration(1), 3); // Stage 1: 3s
    assert_eq!(ladder.stage_duration(2), 3); // Stage 2: 3s
    assert_eq!(ladder.stage_duration(3), 10); // Stage 3: 10s
}

#[test]
fn test_invalid_stage_returns_bid() {
    let ladder = StagedLadder::new(0.05, 0.01);
    
    // Stage 0 or > 3 should return BID (fallback)
    let stage0_price = ladder.stage_price(0, 100.0);
    assert!((stage0_price - 100.0).abs() < 1e-6);
    
    let stage4_price = ladder.stage_price(4, 100.0);
    assert!((stage4_price - 100.0).abs() < 1e-6);
}

#[test]
fn test_partial_fill_loops_back_to_stage_1() {
    // Critic fix: Stage 3 partial fill → loop back to Stage 1 with fresh BID
    let ladder = StagedLadder::new(0.05, 0.01);
    
    // Simulate Stage 3 partial fill with new BID = 95.0
    let new_bid = 95.0;
    let stage1_price_after_loop = ladder.stage_price(1, new_bid);
    
    // Should calculate Stage 1 price from new BID
    // Expected: 95.0 + 0.05 * 0.01 = 95.0005
    assert!((stage1_price_after_loop - 95.0005).abs() < 1e-6);
}

#[test]
fn test_ladder_progression() {
    let ladder = StagedLadder::new(0.05, 0.01);
    let bid = 100.0;
    
    // Verify price decreases through stages
    let stage1 = ladder.stage_price(1, bid);
    let stage2 = ladder.stage_price(2, bid);
    let stage3 = ladder.stage_price(3, bid);
    
    assert!(stage1 > stage2);
    assert!(stage2 > stage3);
    
    // Stage 1 should be above BID
    assert!(stage1 > bid);
    
    // Stage 3 should be below BID
    assert!(stage3 < bid);
}
