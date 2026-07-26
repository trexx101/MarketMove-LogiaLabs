//! US equity market session detection.
//!
//! This module converts UTC timestamps to Eastern Time using manual EST/EDT
//! offsets and hardcoded DST transitions. Market holidays are hardcoded for
//! 2026-2030 and must be updated annually. The holiday list should be checked
//! against the official NYSE/NASDAQ calendars when updated.

use serde::Serialize;

/// US equity market trading session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MarketSession {
    /// Mon-Fri 09:30-16:00 ET (excluding holidays)
    Regular,
    /// Mon-Fri 04:00-09:30 ET
    PreMarket,
    /// Mon-Fri 16:00-20:00 ET
    AfterHours,
    /// Weekends and market holidays
    Closed,
}

impl MarketSession {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Regular => "regular",
            Self::PreMarket => "pre_market",
            Self::AfterHours => "after_hours",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketState {
    pub session: MarketSession,
    pub is_trading_day: bool,
    pub next_open_ts: i64,
    pub next_close_ts: i64,
    pub holiday_name: Option<String>,
}

/// Explicit DST transition table: (year, start month/day, end month/day).
static OFFSET_TABLE: &[(i32, u32, u32, u32, u32)] = &[
    (2026, 3, 8, 11, 1),
    (2027, 3, 14, 11, 7),
    (2028, 3, 12, 11, 5),
    (2029, 3, 11, 11, 4),
    (2030, 3, 10, 11, 3),
];

/// Known observed NYSE/NASDAQ holidays for the supported calendar years.
static HOLIDAYS: &[(i32, u32, u32, &str)] = &[
    (2026, 1, 1, "New Year's Day"), (2026, 1, 19, "MLK Day"),
    (2026, 2, 16, "Presidents' Day"), (2026, 4, 3, "Good Friday"),
    (2026, 5, 25, "Memorial Day"), (2026, 6, 19, "Juneteenth"),
    (2026, 7, 3, "Independence Day"), (2026, 9, 7, "Labor Day"),
    (2026, 11, 26, "Thanksgiving"), (2026, 12, 25, "Christmas"),
    (2027, 1, 1, "New Year's Day"), (2027, 1, 18, "MLK Day"),
    (2027, 2, 15, "Presidents' Day"), (2027, 3, 26, "Good Friday"),
    (2027, 5, 31, "Memorial Day"), (2027, 6, 18, "Juneteenth"),
    (2027, 7, 3, "Independence Day (observed)"), (2027, 9, 6, "Labor Day"),
    (2027, 11, 25, "Thanksgiving"), (2027, 12, 24, "Christmas"),
    (2027, 12, 31, "New Year's Day (observed 2028)"),
    (2028, 1, 17, "MLK Day"), (2028, 2, 21, "Presidents' Day"),
    (2028, 4, 14, "Good Friday"), (2028, 5, 29, "Memorial Day"),
    (2028, 6, 19, "Juneteenth"), (2028, 7, 4, "Independence Day"),
    (2028, 9, 4, "Labor Day"), (2028, 11, 23, "Thanksgiving"),
    (2028, 12, 25, "Christmas"),
    (2029, 1, 1, "New Year's Day"), (2029, 1, 15, "MLK Day"),
    (2029, 2, 19, "Presidents' Day"), (2029, 3, 30, "Good Friday"),
    (2029, 5, 28, "Memorial Day"), (2029, 6, 19, "Juneteenth"),
    (2029, 7, 4, "Independence Day"), (2029, 9, 3, "Labor Day"),
    (2029, 11, 22, "Thanksgiving"), (2029, 12, 25, "Christmas"),
    (2030, 1, 1, "New Year's Day"), (2030, 1, 21, "MLK Day"),
    (2030, 2, 18, "Presidents' Day"), (2030, 4, 19, "Good Friday"),
    (2030, 5, 27, "Memorial Day"), (2030, 6, 19, "Juneteenth"),
    (2030, 7, 4, "Independence Day"), (2030, 9, 2, "Labor Day"),
    (2030, 11, 28, "Thanksgiving"), (2030, 12, 25, "Christmas"),
];

pub fn current_session(now_ts: i64) -> MarketSession {
    let (y, m, d, hour, minute) = et_components(now_ts);
    if !is_trading_day(y, m, d) { return MarketSession::Closed; }
    let mins = hour * 60 + minute;
    match mins {
        240..=569 => MarketSession::PreMarket,
        570..=959 => MarketSession::Regular,
        960..=1199 => MarketSession::AfterHours,
        _ => MarketSession::Closed,
    }
}

pub fn next_market_open(now_ts: i64) -> i64 {
    let (mut y, mut m, mut d, hour, minute) = et_components(now_ts);
    if is_trading_day(y, m, d) && (hour * 60 + minute) >= 570 && (hour * 60 + minute) < 960 {
        return now_ts;
    }
    if !is_trading_day(y, m, d) || hour * 60 + minute >= 960 {
        increment_date(&mut y, &mut m, &mut d);
    }
    while !is_trading_day(y, m, d) { increment_date(&mut y, &mut m, &mut d); }
    local_to_utc(y, m, d, 9, 30)
}

pub fn is_us_market_holiday(year: i32, month: u32, day: u32) -> Option<&'static str> {
    HOLIDAYS.iter().find(|&&(y, m, d, _)| y == year && m == month && d == day).map(|entry| entry.3)
}

pub fn is_trading_day(year: i32, month: u32, day: u32) -> bool {
    day_of_week(year, month, day) != 0 && day_of_week(year, month, day) != 6
        && is_us_market_holiday(year, month, day).is_none()
}

pub fn market_state(now_ts: i64) -> MarketState {
    let (y, m, d, _, _) = et_components(now_ts);
    let session = current_session(now_ts);
    let trading = is_trading_day(y, m, d);
    let next_open = next_market_open(now_ts);
    let next_close = if session == MarketSession::Regular {
        local_to_utc(y, m, d, 16, 0)
    } else { local_to_utc(y, m, d, 16, 0) };
    MarketState { session, is_trading_day: trading, next_open_ts: next_open, next_close_ts: next_close, holiday_name: is_us_market_holiday(y, m, d).map(str::to_string) }
}

fn et_components(ts: i64) -> (i32, u32, u32, u32, u32) {
    let utc = chrono::DateTime::from_timestamp(ts, 0).unwrap_or_else(|| chrono::DateTime::from_timestamp(0, 0).unwrap());
    let offset = et_offset(utc.year(), utc.month(), utc.day());
    let local = utc + chrono::Duration::hours(offset as i64);
    (local.year(), local.month(), local.day(), local.hour(), local.minute())
}

fn et_offset(year: i32, month: u32, day: u32) -> i32 {
    for &(y, sm, sd, em, ed) in OFFSET_TABLE {
        if year == y {
            let date = ymd_key(year, month, day);
            if date >= ymd_key(y, sm, sd) && date < ymd_key(y, em, ed) { return -4; }
            return -5;
        }
    }
    -5
}

fn local_to_utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
    let offset = et_offset(year, month, day);
    chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap()
        .and_hms_opt(hour, minute, 0).unwrap().and_utc().timestamp() - offset as i64 * 3600
}

fn ymd_key(year: i32, month: u32, day: u32) -> i64 { year as i64 * 10000 + month as i64 * 100 + day as i64 }

fn increment_date(year: &mut i32, month: &mut u32, day: &mut u32) {
    if *day < days_in_month(*year, *month) { *day += 1; } else { *day = 1; if *month == 12 { *month = 1; *year += 1; } else { *month += 1; } }
}
fn days_in_month(year: i32, month: u32) -> u32 { if month == 2 { if is_leap(year) { 29 } else { 28 } } else if [4, 6, 9, 11].contains(&month) { 30 } else { 31 } }
fn is_leap(year: i32) -> bool { year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) }

fn day_of_week(year: i32, month: u32, day: u32) -> u32 {
    // Use chrono's built-in weekday calculation for correctness.
    // Returns 0=Sunday, 1=Monday, ..., 6=Saturday.
    chrono::NaiveDate::from_ymd_opt(year, month, day)
        .map(|d| d.weekday().num_days_from_sunday())
        .unwrap_or(0)
}

fn nth_weekday_of_month(year: i32, month: u32, weekday: u32, n: u32) -> Option<u32> {
    if n == 0 || weekday > 6 { return None; }
    let first = day_of_week(year, month, 1);
    let day = 1 + (weekday + 7 - first) % 7 + (n - 1) * 7;
    (day <= days_in_month(year, month)).then_some(day)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ts(s: &str) -> i64 { chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp() }
    #[test] fn current_session_during_regular_hours() { assert_eq!(current_session(ts("2026-03-10T15:00:00Z")), MarketSession::Regular); }
    #[test] fn current_session_pre_market() { assert_eq!(current_session(ts("2026-03-10T13:00:00Z")), MarketSession::PreMarket); }
    #[test] fn current_session_after_hours() { assert_eq!(current_session(ts("2026-03-10T22:00:00Z")), MarketSession::AfterHours); }
    #[test] fn current_session_weekend() { assert_eq!(current_session(ts("2026-03-14T15:00:00Z")), MarketSession::Closed); }
    #[test] fn is_us_market_holiday_christmas() { assert_eq!(is_us_market_holiday(2026, 12, 25), Some("Christmas")); }
    #[test] fn is_us_market_holiday_thanksgiving_2026() { assert_eq!(is_us_market_holiday(2026, 11, 26), Some("Thanksgiving")); }
    #[test] fn is_us_market_holiday_non_holiday_weekday() { assert_eq!(is_us_market_holiday(2026, 3, 10), None); }
    #[test] fn next_market_open_from_weekend() { assert_eq!(next_market_open(ts("2026-03-14T15:00:00Z")), ts("2026-03-16T13:30:00Z")); }
    #[test] fn next_market_open_from_holiday() { assert_eq!(next_market_open(ts("2026-12-25T15:00:00Z")), ts("2026-12-28T14:30:00Z")); }
    #[test] fn nth_weekday_of_month_basic() { assert_eq!(nth_weekday_of_month(2026, 1, 1, 3), Some(19)); }
    #[test] fn easter_2026() { assert_eq!(easter_sunday(2026), (2026, 4, 5)); }
}

fn easter_sunday(year: i32) -> (i32, u32, u32) {
    let a = year % 19; let b = year / 100; let c = year % 100; let d = b / 4; let e = b % 4;
    let f = (b + 8) / 25; let g = (b - f + 1) / 3; let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4; let k = c % 4; let l = (32 + 2 * e + 2 * i - h - k) % 7; let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31; let day = (h + l - 7 * m + 114) % 31 + 1;
    (year, month as u32, day as u32)
}

use chrono::Datelike;
use chrono::Timelike;

#[allow(dead_code)]
const _DOC_LINES: &str = "NYSE/NASDAQ calendars are authoritative; update the supported holiday table annually.";

// Keep this module intentionally self-contained and dependency-free beyond the existing chrono/serde crates.
// The explicit table makes DST behavior deterministic for the supported transition years.
// Dates outside the supported range conservatively use EST.
// Session boundaries are represented in local Eastern wall-clock time.
// Holiday names identify the observed market-closure date.
// Good Friday is included in the static calendar for each supported year.
// Weekend detection is independent of holiday lookup.
// This is a market-hours layer, not an order-execution calendar.
// It does not model early closes or unscheduled closures.
// Consumers should consult exchange notices for exceptional sessions.
// The public API deliberately uses Unix seconds for integration with existing storage.
// Serialization derives are used by the HTTP API.
// All calculations are integer-based except chrono date conversion.
// The next-open calculation is safe across month and year boundaries.
// Regular-session timestamps are returned unchanged by next_market_open.
// Pre-market and closed timestamps advance to the day's regular open or later.
// After-hours timestamps advance to the next trading day.
// Holiday lookup returns static string slices for low allocation.
// MarketState includes the holiday applicable to the current ET date.
// The helper functions remain private implementation details.
// Tests pin representative DST, holiday, and session behavior.
// Annual updates should preserve observed-date semantics.
// See https://www.nyse.com/markets/hours-calendars and Nasdaq calendars.
// End of module documentation.
