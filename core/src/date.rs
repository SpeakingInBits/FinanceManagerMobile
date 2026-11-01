//! Local-time calendar helpers over epoch millis, mirroring the web app's `Date`-based utilities.

use chrono::{Datelike, Local, LocalResult, NaiveDate, TimeZone, Timelike};

use crate::model::Millis;

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Current time in epoch millis.
pub fn now() -> Millis {
    Local::now().timestamp_millis()
}

/// (year, zero-based month, day) of `millis` in local time.
pub fn ymd(millis: Millis) -> (i32, u32, u32) {
    let d = to_local(millis);
    (d.year(), d.month0(), d.day())
}

fn to_local(millis: Millis) -> chrono::DateTime<Local> {
    match Local.timestamp_millis_opt(millis) {
        LocalResult::Single(d) | LocalResult::Ambiguous(d, _) => d,
        LocalResult::None => Local::now(),
    }
}

/// Number of days in the given (year, zero-based month), normalizing month overflow.
pub fn days_in_month(year: i32, month0: i32) -> u32 {
    let (y, m) = normalize_month(year, month0);
    let (ny, nm) = normalize_month(y, m as i32 + 1);
    let first = NaiveDate::from_ymd_opt(y, m + 1, 1).expect("valid date");
    let next = NaiveDate::from_ymd_opt(ny, nm + 1, 1).expect("valid date");
    (next - first).num_days() as u32
}

fn normalize_month(year: i32, month0: i32) -> (i32, u32) {
    (year + month0.div_euclid(12), month0.rem_euclid(12) as u32)
}

/// Epoch millis for a local wall-clock time, like `new Date(y, m, d, h, mi, s, ms)`. Month
/// overflow is normalized; a time skipped by a DST transition resolves forward, as browsers do.
pub fn local_millis(year: i32, month0: i32, day: u32, h: u32, mi: u32, s: u32, ms: u32) -> Millis {
    let (y, m) = normalize_month(year, month0);
    let day = day.clamp(1, days_in_month(y, m as i32));
    let date = NaiveDate::from_ymd_opt(y, m + 1, day).expect("valid date");
    let naive = date.and_hms_milli_opt(h, mi, s, ms).expect("valid time");
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(d) => d.timestamp_millis(),
        LocalResult::Ambiguous(earliest, _) => earliest.timestamp_millis(),
        LocalResult::None => {
            // Inside a DST gap: step forward an hour at a time until the wall time exists.
            let mut probe = naive;
            for _ in 0..4 {
                probe += chrono::Duration::hours(1);
                if let Some(d) = Local.from_local_datetime(&probe).earliest() {
                    return d.timestamp_millis();
                }
            }
            naive.and_utc().timestamp_millis()
        }
    }
}

/// Epoch millis at local midnight of the given date.
pub fn local_midnight(year: i32, month0: i32, day: u32) -> Millis {
    local_millis(year, month0, day, 0, 0, 0, 0)
}

/// Returns `[start, end]` epoch-millis bounds for the month containing `millis`.
pub fn month_bounds(millis: Millis) -> (Millis, Millis) {
    let (y, m, _) = ymd(millis);
    let start = local_midnight(y, m as i32, 1);
    let last = days_in_month(y, m as i32);
    let end = local_millis(y, m as i32, last, 23, 59, 59, 999);
    (start, end)
}

/// Epoch millis for the first day of the month containing `millis`.
pub fn start_of_month(millis: Millis) -> Millis {
    let (y, m, _) = ymd(millis);
    local_midnight(y, m as i32, 1)
}

/// The start-of-month `delta` months away from the month containing `millis`.
pub fn shift_month(millis: Millis, delta: i32) -> Millis {
    let (y, m, _) = ymd(millis);
    local_midnight(y, m as i32 + delta, 1)
}

/// Formats as e.g. "Jul 6, 2026".
pub fn format_date(millis: Millis) -> String {
    let (y, m, d) = ymd(millis);
    format!("{} {d}, {y}", &MONTH_NAMES[m as usize][..3])
}

/// Formats a month as e.g. "July 2026".
pub fn format_month_year(millis: Millis) -> String {
    let (y, m, _) = ymd(millis);
    format!("{} {y}", MONTH_NAMES[m as usize])
}

/// Parses a "YYYY-MM-DD" date into epoch millis at local midnight.
pub fn date_input_to_millis(value: &str) -> Option<Millis> {
    let date = NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok()?;
    Some(local_midnight(
        date.year(),
        date.month0() as i32,
        date.day(),
    ))
}

/// Formats epoch millis as "YYYY-MM-DD".
pub fn millis_to_date_input(millis: Millis) -> String {
    let (y, m, d) = ymd(millis);
    format!("{y:04}-{:02}-{d:02}", m + 1)
}

/// Milliseconds past local midnight, for keeping a time of day when re-dating.
pub fn millis_of_day(millis: Millis) -> u32 {
    let d = to_local(millis);
    d.num_seconds_from_midnight() * 1000 + d.timestamp_subsec_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_bounds_cover_whole_month() {
        let mid = local_millis(2026, 1, 14, 12, 0, 0, 0);
        let (start, end) = month_bounds(mid);
        assert_eq!(millis_to_date_input(start), "2026-02-01");
        assert_eq!(millis_to_date_input(end), "2026-02-28");
        assert_eq!(start_of_month(mid), start);
    }

    #[test]
    fn shift_month_crosses_years() {
        let jan = local_midnight(2026, 0, 31);
        assert_eq!(millis_to_date_input(shift_month(jan, -1)), "2025-12-01");
        assert_eq!(millis_to_date_input(shift_month(jan, 13)), "2027-02-01");
    }

    #[test]
    fn date_input_round_trips() {
        let ms = date_input_to_millis("2024-02-29").unwrap();
        assert_eq!(millis_to_date_input(ms), "2024-02-29");
        assert!(date_input_to_millis("2024-13-01").is_none());
    }

    #[test]
    fn formats_like_en_us_intl() {
        let ms = local_midnight(2026, 6, 6);
        assert_eq!(format_date(ms), "Jul 6, 2026");
        assert_eq!(format_month_year(ms), "July 2026");
    }

    #[test]
    fn days_in_month_handles_leap_years() {
        assert_eq!(days_in_month(2024, 1), 29);
        assert_eq!(days_in_month(2025, 1), 28);
        assert_eq!(days_in_month(2025, 12), 31); // January 2026
    }
}
