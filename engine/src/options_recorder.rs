//! Options Tape Recorder (P1)
//! 
//! Separate process, own OpenD connection, QUOTE-only subscriptions.
//! Records option ticks to parquet, partitioned by underlying/chain/date.
//! Quota accounting enforces D3 budget (60% recorder / 40% live).

use arrow::array::{ArrayRef, Float64Array, Int64Array, StringArray, TimestampMillisecondArray};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use std::collections::VecDeque;
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// Build the parquet schema for the tape (D2).
pub fn build_tape_schema() -> Schema {
    Schema::new(vec![
        Field::new("timestamp", DataType::Timestamp(TimeUnit::Millisecond, None), false),
        Field::new("underlying", DataType::Utf8, false),
        Field::new("chain_code", DataType::Utf8, false),
        Field::new("contract_code", DataType::Utf8, false),
        Field::new("bid", DataType::Float64, false),
        Field::new("ask", DataType::Float64, false),
        Field::new("last", DataType::Float64, false),
        Field::new("volume", DataType::Int64, false),
        Field::new("open_interest", DataType::Int64, false),
        Field::new("implied_volatility", DataType::Float64, false),
        Field::new("delta", DataType::Float64, false),
        Field::new("gamma", DataType::Float64, false),
        Field::new("theta", DataType::Float64, false),
        Field::new("underlying_price", DataType::Float64, false),
    ])
}

/// Quota accounting for option subscriptions (D3).
/// Recorder gets 60% of tier quota.
pub struct QuotaAccount {
    tier: u32,
    max_recorder: u32,
    subscriptions: VecDeque<String>,
}

impl QuotaAccount {
    pub fn new(tier: u32) -> Self {
        Self {
            tier,
            max_recorder: (tier as f64 * 0.6) as u32, // 60% for recorder (D3)
            subscriptions: VecDeque::new(),
        }
    }
    
    pub fn try_subscribe(&mut self, contract_code: &str) -> bool {
        if self.used() >= self.max_recorder {
            return false;
        }
        self.subscriptions.push_back(contract_code.to_string());
        true
    }
    
    pub fn unsubscribe(&mut self, contract_code: &str) {
        self.subscriptions.retain(|c| c != contract_code);
    }
    
    pub fn used(&self) -> u32 {
        self.subscriptions.len() as u32
    }
    
    pub fn remaining(&self) -> u32 {
        self.tier.saturating_sub(self.used())
    }
    
    pub fn shed_oldest(&mut self) -> Option<String> {
        self.subscriptions.pop_front()
    }
}

/// One row of tape data.
#[derive(Clone)]
pub struct TapeRow {
    pub timestamp_ms: i64,
    pub underlying: String,
    pub chain_code: String,
    pub contract_code: String,
    pub bid: f64,
    pub ask: f64,
    pub last: f64,
    pub volume: i64,
    pub open_interest: i64,
    pub implied_volatility: f64,
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub underlying_price: f64,
}

/// Convert rows to arrow arrays (for parquet write).
pub fn build_tape_arrays(rows: &[TapeRow]) -> Vec<ArrayRef> {
    let timestamps: Vec<i64> = rows.iter().map(|r| r.timestamp_ms).collect();
    let underlyings: Vec<&str> = rows.iter().map(|r| r.underlying.as_str()).collect();
    let chains: Vec<&str> = rows.iter().map(|r| r.chain_code.as_str()).collect();
    let contracts: Vec<&str> = rows.iter().map(|r| r.contract_code.as_str()).collect();
    let bids: Vec<f64> = rows.iter().map(|r| r.bid).collect();
    let asks: Vec<f64> = rows.iter().map(|r| r.ask).collect();
    let lasts: Vec<f64> = rows.iter().map(|r| r.last).collect();
    let volumes: Vec<i64> = rows.iter().map(|r| r.volume).collect();
    let ois: Vec<i64> = rows.iter().map(|r| r.open_interest).collect();
    let ivs: Vec<f64> = rows.iter().map(|r| r.implied_volatility).collect();
    let deltas: Vec<f64> = rows.iter().map(|r| r.delta).collect();
    let gammas: Vec<f64> = rows.iter().map(|r| r.gamma).collect();
    let thetas: Vec<f64> = rows.iter().map(|r| r.theta).collect();
    let underlying_prices: Vec<f64> = rows.iter().map(|r| r.underlying_price).collect();
    
    vec![
        Arc::new(TimestampMillisecondArray::from(timestamps)),
        Arc::new(StringArray::from(underlyings)),
        Arc::new(StringArray::from(chains)),
        Arc::new(StringArray::from(contracts)),
        Arc::new(Float64Array::from(bids)),
        Arc::new(Float64Array::from(asks)),
        Arc::new(Float64Array::from(lasts)),
        Arc::new(Int64Array::from(volumes)),
        Arc::new(Int64Array::from(ois)),
        Arc::new(Float64Array::from(ivs)),
        Arc::new(Float64Array::from(deltas)),
        Arc::new(Float64Array::from(gammas)),
        Arc::new(Float64Array::from(thetas)),
        Arc::new(Float64Array::from(underlying_prices)),
    ]
}
