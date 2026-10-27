//! Small apps that live inside the Tools tab.
//!
//! Each one implements `Module` but is not registered on its own: the Tools
//! module (`modules/tools.rs`) owns them, shows a grid of tiles and draws the
//! page of whichever app is open. None of them runs anything in the
//! background unless you start it (focus timer, reminders).

pub mod actions;
pub mod breathe;
pub mod calc;
pub mod calendar;
pub mod dice;
pub mod focus;
pub mod launcher;
pub mod sysinfo;
pub mod todo;
pub mod wellness;
pub mod worldclock;

use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::System::SystemInformation::GetLocalTime;

/// Days since 1970-01-01 for a calendar date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Calendar date (year, month, day) for a day count since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 0 = Sunday … 6 = Saturday.
pub fn weekday(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

pub fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

/// ISO 8601 week number (weeks start on Monday, week 1 holds the first Thursday).
pub fn iso_week(y: i64, m: i64, d: i64) -> i64 {
    let days = days_from_civil(y, m, d);
    let mon_based = (weekday(days) + 6) % 7;
    let thursday = days - mon_based + 3;
    let ty = civil_from_days(thursday).0;
    (thursday - days_from_civil(ty, 1, 1)) / 7 + 1
}

pub const MONTHS: [&str; 12] = [
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
pub const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// The local date and time right now.
#[derive(Clone, Copy, Debug)]
pub struct Local {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
}

impl Local {
    pub fn now() -> Self {
        let t = unsafe { GetLocalTime() };
        Self {
            year: t.wYear as i64,
            month: t.wMonth as i64,
            day: t.wDay as i64,
            hour: t.wHour as i64,
            minute: t.wMinute as i64,
            second: t.wSecond as i64,
        }
    }

    pub fn days(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    /// Seconds since 1970 if the local clock were UTC.
    pub fn as_secs(&self) -> i64 {
        self.days() * 86_400 + self.hour * 3600 + self.minute * 60 + self.second
    }
}

/// Seconds since 1970-01-01 UTC.
pub fn utc_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The PC's UTC offset in seconds, rounded to quarter hours.
pub fn local_offset_secs() -> i64 {
    let diff = Local::now().as_secs() - utc_secs();
    (diff as f64 / 900.0).round() as i64 * 900
}

/// Small random number generator (xorshift), seeded from the clock.
pub struct Rng(u64);

impl Rng {
    pub fn new() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9E37_79B9);
        // splitmix64 step so close seeds give different streams
        let mut z = nanos.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        Self(if z == 0 { 0x2545_F491_4F6C_DD1D } else { z })
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number in `0..n` (n > 0).
    pub fn below(&mut self, n: u32) -> u32 {
        ((self.next() >> 11) % n.max(1) as u64) as u32
    }
}

/// Run a program without a console window and wait for its output.
pub fn run_hidden(program: &str, args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new(program).args(args).creation_flags(0x0800_0000).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Start a program without a console window and do not wait for it.
pub fn spawn_hidden(program: &str, args: &[&str]) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new(program).args(args).creation_flags(0x0800_0000).spawn();
}

/// "1h 05m", "12m" or "45s" for a number of seconds.
pub fn fmt_duration(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        for days in [-1000, 0, 365, 19_000, 20_000, 30_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn weekdays_and_weeks() {
        // 2026-10-25 is a Sunday
        assert_eq!(weekday(days_from_civil(2026, 10, 25)), 0);
        assert_eq!(weekday(0), 4);
        assert_eq!(iso_week(2026, 1, 1), 1);
        assert_eq!(iso_week(2021, 1, 3), 53);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
    }

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(45), "45s");
        assert_eq!(fmt_duration(125), "2m");
        assert_eq!(fmt_duration(3900), "1h 05m");
    }
}
