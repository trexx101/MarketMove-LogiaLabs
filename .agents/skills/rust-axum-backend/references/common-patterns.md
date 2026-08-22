# Common Rust Patterns for Backend Services

Patterns discovered across sessions working on Axum backends.

## strum for Enum String Serialization

When you need enums to serialize to/from lowercase strings in JSON and database columns:

```rust
use strum::{Display, EnumString};

#[derive(Debug, Clone, Copy, Display, EnumString)]
#[strum(serialize_all = "lowercase")]
pub enum EventCategory {
    Trade,
    Data,
    System,
    Strategy,
    Alert,
    Advisor,
}

// Usage:
let cat = EventCategory::Trade;
let s = cat.to_string();  // "trade"
let parsed: EventCategory = "trade".parse().unwrap();
```

**Dependency:**
```toml
strum = { version = "0.26", features = ["derive"] }
```

## BTreeMap for Time-Series Grouping

When archiving or batching events by time period (month, day, hour), use `BTreeMap<String, Vec<T>>` for automatic key sorting:

```rust
use std::collections::BTreeMap;

let mut months: BTreeMap<String, Vec<Event>> = BTreeMap::new();
for event in events {
    let month_key = format!("{}", event.timestamp.format("%Y-%m"));
    months.entry(month_key).or_default().push(event);
}

// Iteration is already sorted by key (oldest first)
for (month, events) in &months {
    // Process each month in order
}
```

## Safe Timestamp Parsing with chrono

`chrono::Timestamp::from_timestamp()` can panic. Use `timestamp_opt()` for safe parsing:

```rust
use chrono::{TimeZone, Utc};

let ts: i64 = 1785858626;
let dt = Utc
    .timestamp_opt(ts, 0)
    .single()
    .unwrap_or_else(Utc::now);
```

This returns `None` for invalid timestamps (out of range) instead of panicking.

## GzEncoder for Compressed JSON Files

When writing gzipped JSON archives:

```rust
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;

let file = std::fs::File::create("archive.json.gz")?;
let mut enc = GzEncoder::new(file, Compression::default());
let json = serde_json::to_string(&data)?;
enc.write_all(json.as_bytes())?;
enc.finish()?;  // Flushes and finishes the gzip stream
```

**Dependency:**
```toml
flate2 = { version = "1", features = ["zlib"] }  # NOT "gzip"
```

## Adding Serialize to Structs Used in Event Payloads

When a struct appears in event payloads (as JSON), it needs `Serialize`. If it only has `Deserialize`:

```rust
// Before (from config/API)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct StrategyParams { ... }

// After (also used in events)
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct StrategyParams { ... }
```

This is required when the struct is passed to `serde_json::json!({ "old": old, "new": new })`.