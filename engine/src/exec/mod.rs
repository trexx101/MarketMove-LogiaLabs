pub mod paper;
pub mod kraken;

use anyhow::Result;
use crate::strategy::Position;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub struct FillResult {
    pub side: TradeSide,
    pub qty: f64,
    pub price: f64,
    pub fee: f64,
    pub realized_pnl: f64,
    #[allow(dead_code)]
    pub ts: i64,
}

pub enum ExecutorKind {
    Paper(paper::PaperExecutor),
    Kraken(kraken::KrakenExecutor),
}

impl ExecutorKind {
    pub async fn set_target_position(
        &mut self,
        target: Position,
        close: f64,
        ts: i64,
    ) -> Result<Vec<FillResult>> {
        match self {
            Self::Paper(e) => e.set_target_position(target, close, ts).await,
            Self::Kraken(e) => e.set_target_position(target, close, ts).await,
        }
    }
}
