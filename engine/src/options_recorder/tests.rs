use super::*;
use arrow::array::{Float64Array, Int64Array, StringArray, TimestampMillisecondArray};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use std::sync::Arc;

#[test]
fn test_parquet_schema_matches_design() {
    let schema = build_tape_schema();
    
    // Required columns per D2
    let required_fields = vec![
        ("timestamp", DataType::Timestamp(TimeUnit::Millisecond, None)),
        ("underlying", DataType::Utf8),
        ("chain_code", DataType::Utf8),
        ("contract_code", DataType::Utf8),
        ("bid", DataType::Float64),
        ("ask", DataType::Float64),
        ("last", DataType::Float64),
        ("volume", DataType::Int64),
        ("open_interest", DataType::Int64),
        ("implied_volatility", DataType::Float64),
        ("delta", DataType::Float64),
        ("gamma", DataType::Float64),
        ("theta", DataType::Float64),
        ("underlying_price", DataType::Float64),
    ];
    
    assert_eq!(schema.fields().len(), required_fields.len());
    
    for (i, (name, expected_type)) in required_fields.iter().enumerate() {
        let field = schema.field(i);
        assert_eq!(field.name(), name, "Field {} name mismatch", i);
        assert_eq!(field.data_type(), expected_type, "Field {} type mismatch", name);
    }
}

#[test]
fn test_quota_accounting_enforces_limit() {
    let mut quota = QuotaAccount::new(20); // Tier 20
    
    // Subscribe 12 chains (60% of 20 = 12)
    for i in 0..12 {
        let code = format!("US.QQQ26090{}C00500000", i);
        assert!(quota.try_subscribe(&code), "Should allow subscription {}", i);
    }
    
    assert_eq!(quota.used(), 12);
    assert_eq!(quota.remaining(), 8);
    
    // 13th should fail (exceeds 60% recorder allocation)
    let code13 = "US.QQQ260999C00500000".to_string();
    assert!(!quota.try_subscribe(&code13), "Should reject 13th subscription");
    
    // Unsubscribe one, then 13th should work
    quota.unsubscribe("US.QQQ260900C00500000");
    assert_eq!(quota.used(), 11);
    assert!(quota.try_subscribe(&code13));
}

#[test]
fn test_quota_shed_under_pressure() {
    let mut quota = QuotaAccount::new(20);
    
    // Fill to 60% (12 chains)
    for i in 0..12 {
        let code = format!("US.QQQ26090{}C00500000", i);
        quota.try_subscribe(&code);
    }
    
    // Simulate pressure: shed oldest subscription
    let shed = quota.shed_oldest();
    assert!(shed.is_some());
    assert_eq!(quota.used(), 11);
}

#[test]
fn test_tape_row_builder() {
    let row = TapeRow {
        timestamp_ms: 1234567890000,
        underlying: "US.QQQ".to_string(),
        chain_code: "US.QQQ260919".to_string(),
        contract_code: "US.QQQ260919C00530000".to_string(),
        bid: 5.20,
        ask: 5.30,
        last: 5.25,
        volume: 150,
        open_interest: 2500,
        implied_volatility: 0.28,
        delta: 0.45,
        gamma: 0.003,
        theta: -0.15,
        underlying_price: 530.0,
    };
    
    let arrays = build_tape_arrays(&[row.clone()]);
    assert_eq!(arrays.len(), 14); // 14 columns
    assert_eq!(arrays[0].len(), 1); // timestamp array has 1 row
}
