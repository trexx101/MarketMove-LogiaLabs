//! Hyperopt module — nightly optimizer + promotion pipeline

pub mod scheduler;
pub mod optimizer;
pub mod stability;
pub mod candidate_store;
pub mod promotion;
pub mod tape_replay;
pub mod runner;

// Re-export commonly used types
pub use candidate_store::CandidateStore;
pub use promotion::PromotionPipeline;
