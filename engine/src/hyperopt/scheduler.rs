//! Nightly scheduler for hyperopt pipeline
//!
//! Runs post-market, CPU-throttled, hard-stop before next open.
//! Never runs during market hours.
//!
//! Supports timezone-aware scheduling: configure local market hours and timezone offset,
//! and the scheduler handles UTC conversion internally.

use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// Scheduler configuration (timezone-aware)
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Timezone offset in hours (e.g., +8 for Malaysia, -5 for ET)
    pub timezone_offset_hours: i32,
    /// Market open time in local timezone
    pub market_open_local: NaiveTime,
    /// Market close time in local timezone
    pub market_close_local: NaiveTime,
    /// Post-market buffer (minutes after close)
    pub post_market_buffer_mins: i64,
    /// Pre-market buffer (minutes before open)
    pub pre_market_buffer_mins: i64,
    /// Maximum run duration (hours)
    pub max_run_hours: i64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        // Default: Malaysia timezone (UTC+8)
        // US market hours in Malaysia: 9:30 PM - 4:00 AM local
        Self {
            timezone_offset_hours: 8,
            market_open_local: NaiveTime::from_hms_opt(21, 30, 0).unwrap(),
            market_close_local: NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
            post_market_buffer_mins: 30,
            pre_market_buffer_mins: 30,
            max_run_hours: 8,
        }
    }
}

impl SchedulerConfig {
    /// Convert local time to UTC
    pub fn local_to_utc(&self, local: NaiveTime) -> NaiveTime {
        let offset_seconds = (self.timezone_offset_hours as i64) * 3600;
        let local_seconds = local.signed_duration_since(
            NaiveTime::from_hms_opt(0, 0, 0).unwrap()
        ).num_seconds();
        let utc_seconds = (local_seconds - offset_seconds).rem_euclid(24 * 3600);
        NaiveTime::from_hms_opt(
            (utc_seconds / 3600) as u32,
            ((utc_seconds % 3600) / 60) as u32,
            (utc_seconds % 60) as u32,
        ).unwrap()
    }

    /// Get market open in UTC
    pub fn market_open_utc(&self) -> NaiveTime {
        self.local_to_utc(self.market_open_local)
    }

    /// Get market close in UTC
    pub fn market_close_utc(&self) -> NaiveTime {
        self.local_to_utc(self.market_close_local)
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
        let market_close_utc = self.config.market_close_utc();
        let market_open_utc = self.config.market_open_utc();
        let earliest_start_secs = market_close_utc.signed_duration_since(midnight).num_seconds()
            + self.config.post_market_buffer_mins * 60;
        let latest_start_secs = market_open_utc.signed_duration_since(midnight).num_seconds()
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
        let market_open_utc = self.config.market_open_utc();
        let market_close_utc = self.config.market_close_utc();
        
        // Handle midnight wrap: if open > close, market spans midnight
        if market_open_utc > market_close_utc {
            time_of_day >= market_open_utc || time_of_day < market_close_utc
        } else {
            time_of_day >= market_open_utc && time_of_day < market_close_utc
        }
    }

    /// Get next eligible run time
    pub fn next_run_time(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        let time_of_day = now.time();
        
        // Convert to seconds since midnight
        let midnight = NaiveTime::from_hms_opt(0, 0, 0).unwrap();
        let current_secs = time_of_day.signed_duration_since(midnight).num_seconds();
        let market_close_utc = self.config.market_close_utc();
        let earliest_start_secs = market_close_utc.signed_duration_since(midnight).num_seconds()
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

    /// UTC date of the hyperopt window that contains `now`.
    ///
    /// A window opens at (market_close + post_market_buffer) UTC, and the next
    /// opens ~24h later. [`Self::next_run_time`] returns that opening instant;
    /// this converts any `now` into the date of the window it belongs to, so a
    /// caller can run the pipeline exactly once per window even across the
    /// midnight wrap (the window spans [20:30, 04:30) UTC).
    pub fn window_start_date(&self, now: DateTime<Utc>) -> chrono::NaiveDate {
        let midnight = NaiveTime::from_hms_opt(0, 0, 0).unwrap();
        let current_secs = now.time().signed_duration_since(midnight).num_seconds();
        let market_close_utc = self.config.market_close_utc();
        let earliest_start_secs = market_close_utc.signed_duration_since(midnight).num_seconds()
            + self.config.post_market_buffer_mins * 60;
        if current_secs >= earliest_start_secs {
            now.date_naive()
        } else {
            // Before today's opening instant -> belongs to the window that
            // opened yesterday (post-midnight side of [20:30, 04:30)).
            now.date_naive() - Duration::days(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timezone_conversion_malaysia() {
        // Malaysia (UTC+8): 9:30 PM local = 1:30 PM UTC
        let config = SchedulerConfig {
            timezone_offset_hours: 8,
            market_open_local: NaiveTime::from_hms_opt(21, 30, 0).unwrap(),
            market_close_local: NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
            ..Default::default()
        };
        
        let open_utc = config.market_open_utc();
        assert_eq!(open_utc, NaiveTime::from_hms_opt(13, 30, 0).unwrap());
        
        let close_utc = config.market_close_utc();
        assert_eq!(close_utc, NaiveTime::from_hms_opt(20, 0, 0).unwrap());
    }

    #[test]
    fn test_timezone_conversion_et() {
        // ET (UTC-5): 9:30 AM local = 2:30 PM UTC
        let config = SchedulerConfig {
            timezone_offset_hours: -5,
            market_open_local: NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
            market_close_local: NaiveTime::from_hms_opt(16, 0, 0).unwrap(),
            ..Default::default()
        };
        
        let open_utc = config.market_open_utc();
        assert_eq!(open_utc, NaiveTime::from_hms_opt(14, 30, 0).unwrap());
        
        let close_utc = config.market_close_utc();
        assert_eq!(close_utc, NaiveTime::from_hms_opt(21, 0, 0).unwrap());
    }

    #[test]
    fn test_can_run_post_market_malaysia() {
        // Malaysia (UTC+8): market closes 4:00 AM local = 8:00 PM UTC
        // Post-market buffer: 30 min → window starts 8:30 PM UTC
        let scheduler = NightlyScheduler::new(SchedulerConfig {
            timezone_offset_hours: 8,
            market_open_local: NaiveTime::from_hms_opt(21, 30, 0).unwrap(),
            market_close_local: NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
            post_market_buffer_mins: 30,
            pre_market_buffer_mins: 30,
            max_run_hours: 8,
        });
        
        // 9:00 PM UTC = 5:00 AM Malaysia (1 hour after close)
        let now = Utc.with_ymd_and_hms(2026, 8, 19, 21, 0, 0).unwrap();
        let state = scheduler.check_state(now);
        assert_eq!(state, SchedulerState::CanRun);
    }

    #[test]
    fn test_cannot_run_during_market_hours_malaysia() {
        // Malaysia (UTC+8): market hours 9:30 PM - 4:00 AM local = 1:30 PM - 8:00 PM UTC
        let scheduler = NightlyScheduler::new(SchedulerConfig {
            timezone_offset_hours: 8,
            market_open_local: NaiveTime::from_hms_opt(21, 30, 0).unwrap(),
            market_close_local: NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
            post_market_buffer_mins: 30,
            pre_market_buffer_mins: 30,
            max_run_hours: 8,
        });
        
        // 3:00 PM UTC = 11:00 PM Malaysia (during market hours)
        let now = Utc.with_ymd_and_hms(2026, 8, 19, 15, 0, 0).unwrap();
        let state = scheduler.check_state(now);
        assert_eq!(state, SchedulerState::CannotRun);
    }

    #[test]
    fn test_window_start_date_midnight_wrap() {
        // Malaysia (UTC+8): window opens 20:30 UTC each evening.
        let scheduler = NightlyScheduler::new(SchedulerConfig {
            timezone_offset_hours: 8,
            market_open_local: NaiveTime::from_hms_opt(21, 30, 0).unwrap(),
            market_close_local: NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
            post_market_buffer_mins: 30,
            pre_market_buffer_mins: 30,
            max_run_hours: 8,
        });

        // Evening inside the window (21:00 UTC on 8/19) -> window started 8/19.
        let evening = Utc.with_ymd_and_hms(2026, 8, 19, 21, 0, 0).unwrap();
        assert_eq!(scheduler.window_start_date(evening), chrono::NaiveDate::from_ymd_opt(2026, 8, 19).unwrap());

        // Post-midnight inside the SAME window (02:00 UTC on 8/20) -> still 8/19.
        // This is the midnight wrap: a naive date-of-now would wrongly yield 8/20.
        let after_midnight = Utc.with_ymd_and_hms(2026, 8, 20, 2, 0, 0).unwrap();
        assert_eq!(scheduler.window_start_date(after_midnight), chrono::NaiveDate::from_ymd_opt(2026, 8, 19).unwrap());

        // Before the window opens (12:00 UTC on 8/19) -> belongs to the window
        // that opened 8/18, not today (which has not opened yet).
        let midday = Utc.with_ymd_and_hms(2026, 8, 19, 12, 0, 0).unwrap();
        assert_eq!(scheduler.window_start_date(midday), chrono::NaiveDate::from_ymd_opt(2026, 8, 18).unwrap());
    }

    #[test]
    fn test_is_market_hours_malaysia() {
        let scheduler = NightlyScheduler::new(SchedulerConfig {
            timezone_offset_hours: 8,
            market_open_local: NaiveTime::from_hms_opt(21, 30, 0).unwrap(),
            market_close_local: NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
            post_market_buffer_mins: 30,
            pre_market_buffer_mins: 30,
            max_run_hours: 8,
        });
        
        // 3:00 PM UTC = 11:00 PM Malaysia (during market)
        let during_market = Utc.with_ymd_and_hms(2026, 8, 19, 15, 0, 0).unwrap();
        assert!(scheduler.is_market_hours(during_market));
        
        // 9:00 PM UTC = 5:00 AM Malaysia (after market)
        let after_market = Utc.with_ymd_and_hms(2026, 8, 19, 21, 0, 0).unwrap();
        assert!(!scheduler.is_market_hours(after_market));
    }

    #[test]
    fn test_hard_stop_after_max_duration() {
        let scheduler = NightlyScheduler::new(SchedulerConfig {
            timezone_offset_hours: 8,
            market_open_local: NaiveTime::from_hms_opt(21, 30, 0).unwrap(),
            market_close_local: NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
            post_market_buffer_mins: 30,
            pre_market_buffer_mins: 30,
            max_run_hours: 4,
        });
        
        // Window starts 8:30 PM UTC, hard stop = 8:30 PM + 4h = 12:30 AM UTC next day
        // 2:00 AM UTC > 12:30 AM UTC → HardStop
        let now = Utc.with_ymd_and_hms(2026, 8, 20, 2, 0, 0).unwrap();
        let state = scheduler.check_state(now);
        assert_eq!(state, SchedulerState::HardStop);
    }

    #[test]
    fn test_next_run_time_before_window() {
        let scheduler = NightlyScheduler::new(SchedulerConfig {
            timezone_offset_hours: 8,
            market_open_local: NaiveTime::from_hms_opt(21, 30, 0).unwrap(),
            market_close_local: NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
            post_market_buffer_mins: 30,
            pre_market_buffer_mins: 30,
            max_run_hours: 8,
        });
        
        // 3:00 PM UTC (during market)
        let now = Utc.with_ymd_and_hms(2026, 8, 19, 15, 0, 0).unwrap();
        let next = scheduler.next_run_time(now);
        
        // Should be today at 8:30 PM UTC (30 min after 8:00 PM close)
        let expected = Utc.with_ymd_and_hms(2026, 8, 19, 20, 30, 0).unwrap();
        assert_eq!(next, expected);
    }

    #[test]
    fn test_next_run_time_after_window() {
        let scheduler = NightlyScheduler::new(SchedulerConfig {
            timezone_offset_hours: 8,
            market_open_local: NaiveTime::from_hms_opt(21, 30, 0).unwrap(),
            market_close_local: NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
            post_market_buffer_mins: 30,
            pre_market_buffer_mins: 30,
            max_run_hours: 8,
        });
        
        // 11:00 PM UTC (after earliest_start at 8:30 PM UTC)
        let now = Utc.with_ymd_and_hms(2026, 8, 19, 23, 0, 0).unwrap();
        let next = scheduler.next_run_time(now);
        
        // Should be tomorrow at 8:30 PM UTC (since we're past earliest_start)
        let expected = Utc.with_ymd_and_hms(2026, 8, 20, 20, 30, 0).unwrap();
        assert_eq!(next, expected);
    }
}
