# Chrono Scheduling Patterns

## Midnight-Wrapping Time Windows

When scheduling tasks in a post-market window (e.g., 21:30 UTC to 14:00 UTC next day), `NaiveTime` doesn't support direct `Duration` addition. Convert to seconds-since-midnight, do arithmetic, then convert back.

**Wrong (won't compile):**
```rust
let earliest_start = self.config.market_close_utc
    + chrono::Duration::minutes(self.config.post_market_buffer_mins);
// ERROR: cannot add `Duration` to `NaiveTime`
```

**Right (seconds-since-midnight):**
```rust
let midnight = NaiveTime::from_hms_opt(0, 0, 0).unwrap();
let current_secs = time_of_day.signed_duration_since(midnight).num_seconds();
let earliest_start_secs = self.config.market_close_utc
    .signed_duration_since(midnight).num_seconds()
    + self.config.post_market_buffer_mins * 60;

// Check if in window (handles midnight wrap)
let in_window = if earliest_start_secs < latest_start_secs {
    // Normal case: window doesn't wrap midnight
    current_secs >= earliest_start_secs && current_secs < latest_start_secs
} else {
    // Window wraps midnight: e.g., 21:30 to 14:00 next day
    current_secs >= earliest_start_secs || current_secs < latest_start_secs
};
```

**Pitfall:** `num_seconds_from_midnight()` is **private** in chrono 0.4. Use `signed_duration_since(midnight).num_seconds()` instead.

## Hard-Stop with Midnight Wrap

When calculating hard-stop (e.g., 21:30 + 4h = 01:30 next day), use modulo arithmetic:

```rust
let hard_stop_secs = (earliest_start_secs + max_run_hours * 3600) % (24 * 3600);
let hard_stop_wrapped = earliest_start_secs + max_run_hours * 3600 >= 24 * 3600;

let past_hard_stop = if hard_stop_wrapped {
    // Hard stop is tomorrow, so we're past it if we're in early morning
    current_secs >= hard_stop_secs && current_secs < latest_start_secs
} else {
    // Hard stop is today
    current_secs >= hard_stop_secs
};
```

## `FnMut` vs `Fn` for Strategy Callbacks

When a closure needs mutable state (counters, accumulators), the trait bound must be `FnMut`, not `Fn`.

**Wrong (won't compile):**
```rust
pub fn check<F>(&self, champion: &HashMap<String, f64>, eval_fn: F)
where
    F: Fn(&HashMap<String, f64>) -> f64,
{
    // ...
}

// Test with mutable counter
let mut call_count = 0;
let eval_fn = |_params: &HashMap<String, f64>| -> f64 {
    call_count += 1;  // ERROR: cannot assign to `call_count`, as it is a captured variable in a `Fn` closure
    0.05
};
```

**Right:**
```rust
pub fn check<F>(&self, champion: &HashMap<String, f64>, mut eval_fn: F)
where
    F: FnMut(&HashMap<String, f64>) -> f64,
{
    // ...
}

// Test with mutable counter
let mut call_count = 0;
let eval_fn = |_params: &HashMap<String, f64>| -> f64 {
    call_count += 1;  // OK: FnMut allows mutation
    0.05
};
```

**When to use `FnMut`:**
- Closures that maintain internal state (counters, accumulators, caches)
- Test mocks that need to track invocation count
- Any callback that modifies captured variables

**When `Fn` is sufficient:**
- Pure functions with no side effects
- Read-only access to captured variables
- Stateless transformations

## Version ID Collision in Tests

When storing multiple candidates in rapid succession (same second), timestamp-based version IDs collide. Use a monotonic counter:

```rust
pub struct CandidateStore {
    candidates: Arc<RwLock<HashMap<String, CandidateSnapshot>>>,
    counter: Arc<RwLock<u64>>,
}

impl CandidateStore {
    fn generate_version_id(&self, timestamp: DateTime<Utc>) -> String {
        let mut counter = self.counter.write().unwrap();
        *counter += 1;
        format!("v{}_{}", timestamp.format("%Y%m%d_%H%M%S"), counter)
    }
}
```

**Pitfall:** Without the counter, all candidates stored in the same second get the same version ID, causing overwrites and test failures like "expected 2 candidates, got 1".
