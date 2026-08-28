//! Integration test — exercises the full hyperopt pipeline end-to-end
//!
//! Tests the complete flow: runner → store → promotion → API query
//! This validates that all P6 components work together correctly.

use sqlx::sqlite::SqlitePoolOptions;

use crate::db;
use crate::hyperopt::{CandidateStore, PromotionPipeline};
use crate::hyperopt::candidate_store::CandidateStatus;
use crate::hyperopt::promotion::{PromotionEvidence, PromotionStage};

/// Helper to create a test DB pool with schema
async fn test_pool() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();

    // Create schema
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS strategy_versions (
            id                      TEXT    PRIMARY KEY,
            equity                  TEXT    NOT NULL DEFAULT 'QQQ',
            family                  TEXT    NOT NULL,
            params_json             TEXT    NOT NULL,
            status                  TEXT    NOT NULL DEFAULT 'NEW',
            promotion_metadata_json TEXT    NOT NULL DEFAULT '{}',
            created_at              INTEGER NOT NULL,
            updated_at              INTEGER NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    pool
}

/// Integration test: full pipeline from candidate creation through promotion
#[tokio::test]
async fn test_full_pipeline_candidate_to_paper() {
    let pool = test_pool().await;
    let store = CandidateStore::new(pool.clone());
    let pipeline = PromotionPipeline::new();

    // Step 1: Store a candidate (simulating runner output)
    let mut params = std::collections::HashMap::new();
    params.insert("ema_fast".to_string(), 12.0);
    params.insert("ema_slow".to_string(), 26.0);

    let version_id = store
        .store(
            "QQQ",
            "ema_macd_breakout",
            params.clone(),
            0.05, // mean_ic
            0.01, // std_ic
            150,  // n_trades
            vec![0.04, 0.05, 0.06], // fold_ics
        )
        .await
        .unwrap();

    // Step 2: Verify candidate was stored with correct status
    let snapshot = store.get(&version_id).await.unwrap().unwrap();
    assert_eq!(snapshot.equity, "QQQ");
    assert_eq!(snapshot.strategy_family, "ema_macd_breakout");
    assert_eq!(snapshot.params, params);
    assert_eq!(snapshot.mean_ic, 0.05);
    assert_eq!(snapshot.status, CandidateStatus::New);

    // Step 3: Verify DB helper functions work
    let total = db::count_strategy_versions(&pool, "QQQ").await.unwrap();
    assert_eq!(total, 1);

    let by_status = db::count_strategy_versions_by_status(&pool, "QQQ")
        .await
        .unwrap();
    assert_eq!(by_status.get("NEW"), Some(&1));

    // Step 4: Promote candidate with sufficient evidence
    let evidence = PromotionEvidence {
        n_trades: 150,
        ic: 0.05,
        sharpe: 1.5,
        days_observed: 0,
        fold_ics: vec![],
    };

    let result = pipeline.promote(&store, &version_id, &evidence).await.unwrap();
    assert!(result.promoted);
    assert_eq!(result.to_stage, PromotionStage::Paper);

    // Step 5: Verify status was updated in DB
    let snapshot = store.get(&version_id).await.unwrap().unwrap();
    assert_eq!(snapshot.status, CandidateStatus::Paper);

    // Step 6: Verify DB helper reflects new status
    let by_status = db::count_strategy_versions_by_status(&pool, "QQQ")
        .await
        .unwrap();
    assert_eq!(by_status.get("PAPER"), Some(&1));
    assert_eq!(by_status.get("NEW"), None);
}

/// Integration test: multi-equity pipeline with equity scoping
#[tokio::test]
async fn test_multi_equity_pipeline() {
    let pool = test_pool().await;
    let store = CandidateStore::new(pool.clone());
    let pipeline = PromotionPipeline::new();

    // Store candidates for multiple equities
    let qqq_id = store
        .store("QQQ", "ema_macd_breakout", std::collections::HashMap::new(), 0.05, 0.01, 150, vec![0.05])
        .await
        .unwrap();

    let smh_id = store
        .store("SMH", "ema_macd_breakout", std::collections::HashMap::new(), 0.06, 0.01, 150, vec![0.06])
        .await
        .unwrap();

    let xlf_id = store
        .store("XLF", "ema_macd_breakout", std::collections::HashMap::new(), 0.07, 0.01, 150, vec![0.07])
        .await
        .unwrap();

    // Verify equity scoping
    let qqq_count = db::count_strategy_versions(&pool, "QQQ").await.unwrap();
    let smh_count = db::count_strategy_versions(&pool, "SMH").await.unwrap();
    let xlf_count = db::count_strategy_versions(&pool, "XLF").await.unwrap();

    assert_eq!(qqq_count, 1);
    assert_eq!(smh_count, 1);
    assert_eq!(xlf_count, 1);

    // Promote QQQ candidate
    let evidence = PromotionEvidence {
        n_trades: 150,
        ic: 0.05,
        sharpe: 1.5,
        days_observed: 0,
        fold_ics: vec![],
    };

    let result = pipeline.promote(&store, &qqq_id, &evidence).await.unwrap();
    assert!(result.promoted);

    // Verify only QQQ was promoted
    let qqq_snapshot = store.get(&qqq_id).await.unwrap().unwrap();
    let smh_snapshot = store.get(&smh_id).await.unwrap().unwrap();
    let xlf_snapshot = store.get(&xlf_id).await.unwrap().unwrap();

    assert_eq!(qqq_snapshot.status, CandidateStatus::Paper);
    assert_eq!(smh_snapshot.status, CandidateStatus::New);
    assert_eq!(xlf_snapshot.status, CandidateStatus::New);

    // Verify DB helpers reflect correct state
    let qqq_by_status = db::count_strategy_versions_by_status(&pool, "QQQ").await.unwrap();
    assert_eq!(qqq_by_status.get("PAPER"), Some(&1));

    let smh_by_status = db::count_strategy_versions_by_status(&pool, "SMH").await.unwrap();
    assert_eq!(smh_by_status.get("NEW"), Some(&1));
}

/// Integration test: promotion failure leaves candidate unchanged
#[tokio::test]
async fn test_promotion_failure_idempotent() {
    let pool = test_pool().await;
    let store = CandidateStore::new(pool.clone());
    let pipeline = PromotionPipeline::new();

    // Store a candidate
    let version_id = store
        .store("QQQ", "ema_macd_breakout", std::collections::HashMap::new(), 0.05, 0.01, 150, vec![0.05])
        .await
        .unwrap();

    // Attempt promotion with insufficient evidence
    let evidence = PromotionEvidence {
        n_trades: 50, // < 100 required
        ic: 0.05,
        sharpe: 1.5,
        days_observed: 0,
        fold_ics: vec![],
    };

    let result = pipeline.promote(&store, &version_id, &evidence).await.unwrap();
    assert!(!result.promoted);
    assert!(result.reason.contains("Insufficient trades"));

    // Verify candidate status unchanged
    let snapshot = store.get(&version_id).await.unwrap().unwrap();
    assert_eq!(snapshot.status, CandidateStatus::New);

    // Verify DB reflects unchanged state
    let by_status = db::count_strategy_versions_by_status(&pool, "QQQ").await.unwrap();
    assert_eq!(by_status.get("NEW"), Some(&1));
}

/// Integration test: multi-stage promotion pipeline
#[tokio::test]
async fn test_multi_stage_promotion() {
    let pool = test_pool().await;
    let store = CandidateStore::new(pool.clone());
    let pipeline = PromotionPipeline::new();

    // Store a candidate
    let version_id = store
        .store("QQQ", "ema_macd_breakout", std::collections::HashMap::new(), 0.05, 0.01, 150, vec![0.05])
        .await
        .unwrap();

    // Stage 1: Candidate → Paper
    let evidence = PromotionEvidence {
        n_trades: 150,
        ic: 0.05,
        sharpe: 1.5,
        days_observed: 0,
        fold_ics: vec![],
    };

    let result = pipeline.promote(&store, &version_id, &evidence).await.unwrap();
    assert!(result.promoted);
    assert_eq!(result.to_stage, PromotionStage::Paper);

    let snapshot = store.get(&version_id).await.unwrap().unwrap();
    assert_eq!(snapshot.status, CandidateStatus::Paper);

    // Stage 2: Paper → Micro. The days gate reads the LIVE observation
    // clock (updated_at), NOT the evidence field — simulate 15 days in
    // PAPER by backdating updated_at. Evidence days_observed is deliberately
    // 0 to prove it is ignored.
    let fifteen_days_ago = chrono::Utc::now().timestamp() - 15 * 86_400;
    sqlx::query("UPDATE strategy_versions SET updated_at = ? WHERE id = ?")
        .bind(fifteen_days_ago)
        .bind(&version_id)
        .execute(&pool)
        .await
        .unwrap();

    let evidence = PromotionEvidence {
        n_trades: 30,
        ic: 0.05,
        sharpe: 1.5,
        days_observed: 0, // ignored — live clock says 15 >= 14
        fold_ics: vec![],
    };

    let result = pipeline.promote(&store, &version_id, &evidence).await.unwrap();
    assert!(result.promoted, "stage 2 denied: {}", result.reason);
    assert_eq!(result.to_stage, PromotionStage::Micro);

    let snapshot = store.get(&version_id).await.unwrap().unwrap();
    assert_eq!(snapshot.status, CandidateStatus::Micro);

    // Stage 3: Micro → Live — backdate again to simulate 15 days in MICRO.
    sqlx::query("UPDATE strategy_versions SET updated_at = ? WHERE id = ?")
        .bind(fifteen_days_ago)
        .bind(&version_id)
        .execute(&pool)
        .await
        .unwrap();

    let evidence = PromotionEvidence {
        n_trades: 50,
        ic: 0.05,
        sharpe: 2.0,
        days_observed: 0, // ignored — live clock says 15 >= 14
        fold_ics: vec![],
    };

    let result = pipeline.promote(&store, &version_id, &evidence).await.unwrap();
    assert!(result.promoted);
    assert_eq!(result.to_stage, PromotionStage::Live);

    let snapshot = store.get(&version_id).await.unwrap().unwrap();
    assert_eq!(snapshot.status, CandidateStatus::Live);

    // Verify final DB state
    let by_status = db::count_strategy_versions_by_status(&pool, "QQQ").await.unwrap();
    assert_eq!(by_status.get("LIVE"), Some(&1));
}
