//! Wall-clock time in a named zone, from a real tz database.
//!
//! `claude -p /usage` prints a reset as a local wall time and an IANA zone
//! name (`Sep 24 at 1pm (Asia/Seoul)`), and the popover needs the instant.
//! The core carries no calendar arithmetic of its own, so the zone's rules
//! come from `jiff`: the system database under `/usr/share/zoneinfo` on macOS
//! and Linux, and the database `jiff` bundles on Windows, which has none. The
//! footer rule of a zone with DST is applied, so any year answers.

use std::path::{Component, Path, PathBuf};

use jiff::Timestamp;
use jiff::tz::TimeZone;

/// A zone's rules, read from the tz database.
pub(crate) struct Zone {
    zone: TimeZone,
}

impl Zone {
    /// Loads a zone by IANA name. The name is validated as a relative path of
    /// plain components before it reaches the database.
    pub(crate) fn load(name: &str) -> Result<Self, &'static str> {
        zone_path(name).ok_or("zone_name")?;
        let zone = TimeZone::get(name).map_err(|_| "zone_file")?;
        Ok(Self { zone })
    }

    /// The UTC offset in force at `unix`.
    pub(crate) fn offset_at(&self, unix: i64) -> Result<i32, &'static str> {
        let at = Timestamp::from_second(unix).map_err(|_| "zone_range")?;
        Ok(self.zone.to_offset(at).seconds())
    }

    /// Every instant at which the zone's wall clock reads `local` (seconds
    /// since the epoch as if the wall time were UTC): one normally, none
    /// inside a spring-forward gap, two inside a fall-back overlap. A zone
    /// changes offset at most once within a day of any instant, so the
    /// offsets worth trying are the ones in force a day either side.
    pub(crate) fn instants_of(&self, local: i64) -> Result<Vec<i64>, &'static str> {
        let mut offsets = Vec::with_capacity(3);
        for day in -1..=1 {
            offsets.push(self.offset_at(local + day * 86_400)?);
        }
        offsets.sort_unstable();
        offsets.dedup();
        let mut instants = Vec::new();
        for offset in offsets {
            let candidate = local - i64::from(offset);
            if self.offset_at(candidate)? == offset {
                instants.push(candidate);
            }
        }
        instants.sort_unstable();
        Ok(instants)
    }
}

/// A zone name as a relative path: `Area/Location` components of the
/// characters IANA names use, never absolute and never climbing.
fn zone_path(name: &str) -> Option<PathBuf> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'+'))
    {
        return None;
    }
    let path = Path::new(name);
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
        .then(|| path.to_path_buf())
}

/// Days since 1970-01-01 of a proleptic Gregorian date; the inverse of
/// [`civil_from_days`]. Howard Hinnant's algorithm.
pub(crate) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// `(year, month, day)` of a day count since 1970-01-01.
pub(crate) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (era * 400 + year_of_era + i64::from(month <= 2), month, day)
}

pub(crate) fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
        days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60
    }

    /// The expected instants are from `date -j -f` / Python `ZoneInfo`, not
    /// from this reader.
    #[test]
    fn seoul_has_one_instant_per_wall_time_at_plus_nine() {
        let zone = Zone::load("Asia/Seoul").unwrap();
        assert_eq!(
            zone.instants_of(local(2026, 9, 24, 13, 0)).unwrap(),
            vec![1_790_222_400]
        );
        assert_eq!(zone.offset_at(1_790_222_400).unwrap(), 9 * 3_600);
    }

    #[test]
    fn a_dst_zone_answers_the_gap_with_nothing_and_the_overlap_with_both() {
        let zone = Zone::load("America/New_York").unwrap();
        // 2026-03-08 02:30 does not exist: the clock jumps from 02:00 to 03:00.
        assert_eq!(
            zone.instants_of(local(2026, 3, 8, 2, 30)).unwrap(),
            Vec::<i64>::new()
        );
        // 2026-11-01 01:30 happens twice, EDT then EST.
        assert_eq!(
            zone.instants_of(local(2026, 11, 1, 1, 30)).unwrap(),
            vec![1_793_511_000, 1_793_514_600]
        );
        // 2026-09-24 13:00 EDT.
        assert_eq!(
            zone.instants_of(local(2026, 9, 24, 13, 0)).unwrap(),
            vec![1_790_269_200]
        );
    }

    #[test]
    fn a_rule_zone_answers_years_far_past_its_last_transition() {
        // Summer 2050 in New York is EDT (UTC-4), winter EST (UTC-5).
        let zone = Zone::load("America/New_York").unwrap();
        assert_eq!(zone.offset_at(local(2050, 6, 1, 12, 0)), Ok(-4 * 3_600));
        assert_eq!(zone.offset_at(local(2050, 12, 1, 12, 0)), Ok(-5 * 3_600));
        assert_eq!(
            Zone::load("Asia/Seoul")
                .unwrap()
                .offset_at(local(2050, 6, 1, 12, 0)),
            Ok(9 * 3_600)
        );
    }

    #[test]
    fn a_zone_name_never_leaves_the_database_directory() {
        for name in [
            "../../etc/passwd",
            "/etc/localtime",
            "Asia/../Seoul",
            "",
            "Asia/Seoul\n",
        ] {
            assert_eq!(Zone::load(name).err(), Some("zone_name"), "{name:?}");
        }
        assert_eq!(Zone::load("Mars/Olympus").err(), Some("zone_file"));
    }

    #[test]
    fn civil_conversions_round_trip_across_leap_days() {
        for (year, month, day) in [(1970, 1, 1), (2000, 2, 29), (2026, 9, 24), (2100, 3, 1)] {
            let days = days_from_civil(year, month, day);
            assert_eq!(civil_from_days(days), (year, month, day));
        }
        assert_eq!(days_from_civil(2026, 9, 24), 20_720);
    }
}
