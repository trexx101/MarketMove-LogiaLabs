pub mod moomoo;
pub mod paper;

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
    /// Instrument traded (primary symbol for long, inverse-ETF symbol for short).
    pub symbol: String,
    pub qty: f64,
    pub price: f64,
    pub fee: f64,
    pub realized_pnl: f64,
    #[allow(dead_code)]
    pub ts: i64,
}

pub enum ExecutorKind {
    Paper(paper::PaperExecutor),
    /// Moomoo OpenD executor (Phase 3.3). Shells out to place_order.py.
    /// Default `trd_env=SIMULATE`; flip to REAL via MOOMOO_TRD_ENV=REAL after
    /// the OpenD GUI has been unlocked.
    Moomoo(moomoo::MoomooExecutor),
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
            Self::Moomoo(e) => e.set_target_position(target, close, ts).await,
        }
    }
}
