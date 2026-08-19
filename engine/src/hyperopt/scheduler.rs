//! Nightly scheduler for hyperopt pipeline
//!
//! Runs post-market, CPU-throttled, hard-stop before next open.
//! Never runs during market hours.

use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// Scheduler configuration
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Market open time (ET) as UTC offset
    pub market_open_utc: NaiveTime,
    /// Market close time (ET) as UTC offset
    pub market_close_utc: NaiveTime,
    /// Post-market buffer (minutes after close)
    pub post_market_buffer_mins: i64,
    /// Pre-market buffer (minutes before open)
    pub pre_market_buffer_mins: i64,
    /// Maximum run duration (hours)
    pub max_run_hours: i64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        // ET is UTC-5 (simplified; DST handling would use proper timezone)
        // 9:30 AM ET = 14:30 UTC
        // 4:00 PM ET = 21:00 UTC
        Self {
            market_open_utc: NaiveTime::from_hms_opt(14, 30, 0).unwrap(),
            market_close_utc: NaiveTime::from_hms_opt(21, 0, 0).unwrap(),
            post_market_buffer_mins: 30,
            pre_market_buffer_mins: 30,
            max_run_hours: 8,
        }
    }
}

/// Scheduler state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedulerState {
    /// Can run (post-market window)
    CanRun,
    /// Cannot run (market hours or pre-market)
    CannotRun,
    /// Hard stop reached
    HardStop,
}

/// Nightly scheduler
pub struct NightlyScheduler {
    config: SchedulerConfig,
}

impl NightlyScheduler {
    pub fn new(config: SchedulerConfig) -> Self {
        Self { config }
    }

    /// Check if scheduler can run at given time
    pub fn check_state(&self, now: DateTime<Utc>) -> SchedulerState {
        let time_of_day = now.time();
        
        // Convert times to seconds since midnight for easier arithmetic
        let midnight = NaiveTime::from_hms_opt(0, 0, 0).unwrap();
        let current_secs = time_of_day.signed_duration_since(midnight).num_seconds();
        let earliest_start_secs = self.config.market_close_utc.signed_duration_since(midnight).num_seconds()
            + self.config.post_market_buffer_mins * 60;
        let latest_start_secs = self.config.market_open_utc.signed_duration_since(midnight).num_seconds()
            - self.config.pre_market_buffer_mins * 60;

        // Check if we're in the run window (handles midnight wrap)
        // Window: 21:30 UTC (77400s) to 14:00 UTC (50400s) wraps midnight
        let in_window = if earliest_start_secs < latest_start_secs {
            // Normal case: window doesn't wrap midnight
            current_secs >= earliest_start_secs && current_secs < latest_start_secs
        } else {
            // Window wraps midnight: e.g., 21:30 to 14:00 next day
            current_secs >= earliest_start_secs || current_secs < latest_start_secs
        };

        if !in_window {
            return SchedulerState::CannotRun;
        }

        // Check hard stop (max run duration from earliest start)
        // If earliest_start is 21:30 (77400s) and max_run_hours is 4, hard_stop is 01:30 (5400s) next day
        let hard_stop_secs = (earliest_start_secs + self.config.max_run_hours * 3600) % (24 * 3600);
        
        // If hard_stop wrapped (e.g., 21:30 + 4h = 01:30 next day),
        // we're past it if current_secs >= hard_stop_secs AND current_secs < latest_start_secs
        let hard_stop_wrapped = earliest_start_secs + self.config.max_run_hours * 3600 >= 24 * 3600;
        
        let past_hard_stop = if hard_stop_wrapped {
            // Hard stop is tomorrow, so we're past it if we're in early morning
            current_secs >= hard_stop_secs && current_secs < latest_start_secs
        } else {
            // Hard stop is today
            current_secs >= hard_stop_secs
        };

        if past_hard_stop {
            return SchedulerState::HardStop;
        }

        SchedulerState::CanRun
    }

    /// Check if current time is during market hours
    pub fn is_market_hours(&self, now: DateTime<Utc>) -> bool {
        let time_of_day = now.time();
        time_of_day >= self.config.market_open_utc && time_of_day < self.config.market_close_utc
    }

    /// Get next eligible run time
    pub fn next_run_time(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        let time_of_day = now.time();
        
        // Convert to seconds since midnight
        let midnight = NaiveTime::from_hms_opt(0, 0, 0).unwrap();
        let current_secs = time_of_day.signed_duration_since(midnight).num_seconds();
        let earliest_start_secs = self.config.market_close_utc.signed_duration_since(midnight).num_seconds()
            + self.config.post_market_buffer_mins * 60;

        // Calculate the earliest start time as seconds since midnight
        let earliest_start_time = NaiveTime::from_hms_opt(
            (earliest_start_secs / 3600) as u32,
            ((earliest_start_secs % 3600) / 60) as u32,
            (earliest_start_secs % 60) as u32,
        ).unwrap();

        // If before earliest_start today, run today
        if current_secs < earliest_start_secs {
            let today = now.date_naive();
            let run_datetime = today.and_time(earliest_start_time);
            return Utc.from_utc_datetime(&run_datetime);
        }

        // Otherwise, run tomorrow
        let tomorrow = (now + Duration::days(1)).date_naive();
        let run_datetime = tomorrow.and_time(earliest_start_time);
        Utc.from_utc_datetime(&run_datetime)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_run_post_market() {
        let scheduler = NightlyScheduler::new(SchedulerConfig::default());
        
        // 5:00 PM ET (1 hour after close) = 22:00 UTC
        // Window is 21:30 UTC to 14:00 UTC (wraps midnight)
        let now = Utc.with_ymd_and_hms(2026, 8, 19, 22, 0, 0).unwrap();
        let state = scheduler.check_state(now);
        assert_eq!(state, SchedulerState::CanRun);
    }

    #[test]
    fn test_cannot_run_during_market_hours() {
        let scheduler = NightlyScheduler::new(SchedulerConfig::default());
        
        // 12:00 PM ET = 17:00 UTC (during market hours)
        let now = Utc.with_ymd_and_hms(2026, 8, 19, 17, 0, 0).unwrap();
        let state = scheduler.check_state(now);
        assert_eq!(state, SchedulerState::CannotRun);
    }

    #[test]
    fn test_cannot_run_pre_market() {
        let scheduler = NightlyScheduler::new(SchedulerConfig::default());
        
        // 10:00 AM ET = 15:00 UTC (during market hours, between market open and post-market window)
        let now = Utc.with_ymd_and_hms(2026, 8, 19, 15, 0, 0).unwrap();
        let state = scheduler.check_state(now);
        assert_eq!(state, SchedulerState::CannotRun);
    }

    #[test]
    fn test_hard_stop_after_max_duration() {
        let scheduler = NightlyScheduler::new(SchedulerConfig {
            max_run_hours: 4,
            ..Default::default()
        });
        
        // 2:00 AM ET next day = 07:00 UTC (10 hours after window opens at 21:30)
        // Hard stop = 21:30 + 4h = 01:30 UTC next day
        // 07:00 UTC > 01:30 UTC → HardStop
        let now = Utc.with_ymd_and_hms(2026, 8, 20, 7, 0, 0).unwrap();
        let state = scheduler.check_state(now);
        assert_eq!(state, SchedulerState::HardStop);
    }

    #[test]
    fn test_is_market_hours() {
        let scheduler = NightlyScheduler::new(SchedulerConfig::default());
        
        // 12:00 PM ET = 17:00 UTC
        let during_market = Utc.with_ymd_and_hms(2026, 8, 19, 17, 0, 0).unwrap();
        assert!(scheduler.is_market_hours(during_market));
        
        // 6:00 PM ET = 23:00 UTC
        let after_market = Utc.with_ymd_and_hms(2026, 8, 19, 23, 0, 0).unwrap();
        assert!(!scheduler.is_market_hours(after_market));
    }

    #[test]
    fn test_next_run_time_before_window() {
        let scheduler = NightlyScheduler::new(SchedulerConfig::default());
        
        // 12:00 PM ET = 17:00 UTC (during market)
        let now = Utc.with_ymd_and_hms(2026, 8, 19, 17, 0, 0).unwrap();
        let next = scheduler.next_run_time(now);
        
        // Should be today at 4:30 PM ET = 21:30 UTC
        let expected = Utc.with_ymd_and_hms(2026, 8, 19, 21, 30, 0).unwrap();
        assert_eq!(next, expected);
    }

    #[test]
    fn test_next_run_time_after_window() {
        let scheduler = NightlyScheduler::new(SchedulerConfig::default());
        
        // 11:00 PM ET = 04:00 UTC next day (after window, but still in run window)
        let now = Utc.with_ymd_and_hms(2026, 8, 20, 04, 0, 0).unwrap();
        let next = scheduler.next_run_time(now);
        
        // Should be today at 4:30 PM ET = 21:30 UTC (since we're still in the window)
        let expected = Utc.with_ymd_and_hms(2026, 8, 20, 21, 30, 0).unwrap();
        assert_eq!(next, expected);
    }
}
