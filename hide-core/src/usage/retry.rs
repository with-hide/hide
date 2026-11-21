use crate::zoneinfo::{days_from_civil, days_in_month};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
pub(super) fn parse_retry_after(value: &str) -> Option<Duration> {
    parse_retry_after_at(
        value,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
}

fn parse_retry_after_at(value: &str, now_unix_seconds: u64) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let retry_at = parse_http_date(value)?;
    Some(Duration::from_secs(
        retry_at.saturating_sub(now_unix_seconds),
    ))
}

fn parse_http_date(value: &str) -> Option<u64> {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    let (day, month, year, time) = match parts.as_slice() {
        // IMF-fixdate: Sun, 06 Nov 1994 08:49:37 GMT
        [weekday, day, month, year, time, "GMT"] if weekday.ends_with(',') => (
            day.parse::<i64>().ok()?,
            parse_http_month(month)?,
            year.parse::<i64>().ok()?,
            *time,
        ),
        // Obsolete RFC 850 form: Sunday, 06-Nov-94 08:49:37 GMT
        [weekday, date, time, "GMT"] if weekday.ends_with(',') => {
            let date = date.split('-').collect::<Vec<_>>();
            let [day, month, year] = date.as_slice() else {
                return None;
            };
            let short_year = year.parse::<i64>().ok()?;
            let year = if short_year >= 70 {
                1_900 + short_year
            } else {
                2_000 + short_year
            };
            (
                day.parse::<i64>().ok()?,
                parse_http_month(month)?,
                year,
                *time,
            )
        }
        // ANSI C asctime form: Sun Nov  6 08:49:37 1994
        [_weekday, month, day, time, year] => (
            day.parse::<i64>().ok()?,
            parse_http_month(month)?,
            year.parse::<i64>().ok()?,
            *time,
        ),
        _ => return None,
    };
    let time = time.split(':').collect::<Vec<_>>();
    let [hour, minute, second] = time.as_slice() else {
        return None;
    };
    let hour = hour.parse::<i64>().ok()?;
    let minute = minute.parse::<i64>().ok()?;
    let second = second.parse::<i64>().ok()?;
    if !(1..=days_in_month(year, month)).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return None;
    }
    let timestamp = days_from_civil(year, month, day)
        .checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)?;
    u64::try_from(timestamp).ok()
}

pub(super) fn parse_http_month(value: &str) -> Option<i64> {
    Some(match value {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_after_preserves_delay_seconds_and_http_dates() {
        assert_eq!(
            parse_retry_after_at("120", 784111717),
            Some(Duration::from_secs(120))
        );
        for value in [
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
        ] {
            assert_eq!(
                parse_retry_after_at(value, 784111717),
                Some(Duration::from_secs(60))
            );
        }
    }
}
