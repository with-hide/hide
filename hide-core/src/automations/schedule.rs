//! When an automation runs (PRD automations D-06, B6, B7): one of the form's
//! choices (every day, weekdays, chosen days of the week, every N minutes,
//! every N hours from a time) or a five-field cron line, read in the zone of
//! the machine that runs it.
//!
//! Every choice compiles to the same cron form and one walk finds its next
//! time, so a choice and the cron line it equals answer alike. The shortest
//! period is five minutes (D-16): a cron line that would run two times
//! closer than that is refused. The calendar comes from `jiff`, which the core already uses
//! for zones: a time a spring-forward skips runs at the time the clock jumps
//! to, and a time a fall-back repeats runs once, at its first instant.

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

/// How far ahead a next time is looked for: a cron line with no time in five
/// years (the 30th of February) never runs and is refused.
const HORIZON_DAYS: u32 = 5 * 366;

/// The longest cron line read; a longer one is refused, not cut.
const CRON_CAP: usize = 128;

/// No two runs of a schedule are closer than this (D-16).
const SHORTEST_PERIOD_MINUTES: u8 = 5;

/// The periods the form offers in minutes (D-06): each divides the hour,
/// so every hour runs alike.
pub const EVERY_MINUTES: [u8; 4] = [5, 10, 15, 30];

/// A day of the week, as the form and the CLI name it.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Sun,
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
}

impl Weekday {
    const ALL: [Self; 7] = [
        Self::Sun,
        Self::Mon,
        Self::Tue,
        Self::Wed,
        Self::Thu,
        Self::Fri,
        Self::Sat,
    ];

    /// Cron's number for the day: Sunday is 0.
    fn number(self) -> u8 {
        self as u8
    }
}

/// A wall time, `HH:MM`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Clock {
    pub hour: u8,
    pub minute: u8,
}

impl Clock {
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        let (hour, minute) = text.split_once(':').ok_or("schedule_time_invalid")?;
        let field = |value: &str, max: u8| {
            (value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| value.parse::<u8>().ok())
                .flatten()
                .filter(|value| *value <= max)
                .ok_or("schedule_time_invalid")
        };
        Ok(Self {
            hour: field(hour, 23)?,
            minute: field(minute, 59)?,
        })
    }
}

impl Serialize for Clock {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{:02}:{:02}", self.hour, self.minute))
    }
}

impl<'de> Deserialize<'de> for Clock {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// What the operator chose (B6): kept as chosen, so the form opens on it
/// again, and read through [`Schedule::compile`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Schedule {
    Daily {
        time: Clock,
    },
    Weekdays {
        time: Clock,
    },
    Weekly {
        days: Vec<Weekday>,
        time: Clock,
    },
    /// Every `minutes` minutes, one of `EVERY_MINUTES`, counted from each
    /// hour's start: every 15 minutes is :00, :15, :30 and :45.
    EveryMinutes {
        minutes: u8,
    },
    /// Every `hours` hours (1 to 12) on the minute of `time`, counted from
    /// its hour: every 3 hours from 09:00 is 00:00, 03:00, ..., 21:00.
    EveryHours {
        hours: u8,
        time: Clock,
    },
    Cron {
        expr: String,
    },
}

/// The fields of a cron line, each the set of values it allows.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Cron {
    minutes: Vec<u8>,
    hours: Vec<u8>,
    days: Vec<u8>,
    months: Vec<u8>,
    weekdays: Vec<u8>,
    /// Cron's rule: when both the day of the month and the day of the week
    /// are restricted, a day matching either one runs.
    days_restricted: bool,
    weekdays_restricted: bool,
}

/// How the screen words a schedule (B6): the choice it equals, or the cron
/// line itself when it equals none. The screen writes the words.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Meaning {
    /// The choice the schedule reads as; a cron line that equals a choice
    /// reads as that choice ("평일 09:00").
    pub reads_as: Schedule,
}

impl Schedule {
    /// The schedule as a cron line's sets, or why it cannot run (B7). Each
    /// reason is a code the form and the CLI word.
    fn compile(&self) -> Result<Cron, &'static str> {
        let every_day = || (0..7).collect::<Vec<u8>>();
        let at = |time: &Clock, weekdays: Vec<u8>, restricted: bool| Cron {
            minutes: vec![time.minute],
            hours: vec![time.hour],
            days: (1..=31).collect(),
            months: (1..=12).collect(),
            weekdays,
            days_restricted: false,
            weekdays_restricted: restricted,
        };
        let cron = match self {
            Self::Daily { time } => at(time, every_day(), false),
            Self::Weekdays { time } => at(time, (1..=5).collect(), true),
            Self::Weekly { days, time } => {
                if days.is_empty() {
                    return Err("schedule_days_missing");
                }
                let mut numbers: Vec<u8> = days.iter().map(|day| day.number()).collect();
                numbers.sort_unstable();
                numbers.dedup();
                let restricted = numbers.len() < 7;
                at(time, numbers, restricted)
            }
            Self::EveryMinutes { minutes } => {
                if !EVERY_MINUTES.contains(minutes) {
                    return Err("schedule_minutes_out_of_range");
                }
                Cron {
                    minutes: (0..60).step_by(usize::from(*minutes)).collect(),
                    hours: (0..24).collect(),
                    ..at(&Clock { hour: 0, minute: 0 }, every_day(), false)
                }
            }
            Self::EveryHours { hours, time } => {
                if !(1..=12).contains(hours) {
                    return Err("schedule_hours_out_of_range");
                }
                let mut cron = at(time, every_day(), false);
                cron.hours = (0..24u8)
                    .filter(|hour| (hour + 24 - time.hour) % hours == 0)
                    .collect();
                cron
            }
            Self::Cron { expr } => parse_cron(expr)?,
        };
        if too_often(&cron) {
            return Err("schedule_too_often");
        }
        Ok(cron)
    }

    /// Refuses a schedule that cannot run, with why (B7, B10): the same
    /// check the form, the CLI and a stored automation's load make.
    pub fn validate(&self, zone: &TimeZone) -> Result<(), &'static str> {
        let cron = self.compile()?;
        // A line that names no real day never runs.
        next_of(&cron, 0, zone).ok_or("schedule_never")?;
        Ok(())
    }

    /// The first time after `after_unix_ms` the schedule runs, in `zone`;
    /// `None` for a schedule that cannot run.
    pub fn next_after(&self, after_unix_ms: u64, zone: &TimeZone) -> Option<u64> {
        next_of(&self.compile().ok()?, after_unix_ms, zone)
    }

    /// The choice the schedule reads as (B6): a cron line equal to a choice
    /// reads as that choice, anything else as itself.
    pub fn meaning(&self) -> Meaning {
        let reads_as = match self {
            Self::Cron { .. } => self
                .compile()
                .ok()
                .and_then(|cron| choice_of(&cron))
                .unwrap_or_else(|| self.clone()),
            choice => choice.clone(),
        };
        Meaning { reads_as }
    }
}

/// Whether two runs could come closer than `SHORTEST_PERIOD_MINUTES`
/// (D-16): two minutes of an hour, or the last minute of an hour and the
/// first of the next one that runs too. An hour that ends a day is taken as
/// followed by the next day's first, which any day may be.
fn too_often(cron: &Cron) -> bool {
    let (Some(first), Some(last)) = (cron.minutes.first(), cron.minutes.last()) else {
        return true;
    };
    let within = cron
        .minutes
        .windows(2)
        .any(|pair| pair[1] - pair[0] < SHORTEST_PERIOD_MINUTES);
    let across = 60 + first - last < SHORTEST_PERIOD_MINUTES
        && cron
            .hours
            .iter()
            .any(|hour| cron.hours.contains(&((hour + 1) % 24)));
    within || across
}

/// The choice a cron line equals, when it equals one.
fn choice_of(cron: &Cron) -> Option<Schedule> {
    if cron.days_restricted || cron.months.len() != 12 {
        return None;
    }
    if cron.minutes.len() > 1 {
        // Every N minutes: every hour of every day, a step of N from :00.
        let step = cron.minutes[1];
        let stepped: Vec<u8> = (0..60).step_by(usize::from(step)).collect();
        return (EVERY_MINUTES.contains(&step)
            && stepped == cron.minutes
            && cron.hours.len() == 24
            && cron.weekdays.len() == 7)
            .then_some(Schedule::EveryMinutes { minutes: step });
    }
    let minute = *cron.minutes.first()?;
    let weekdays: Vec<Weekday> = cron
        .weekdays
        .iter()
        .map(|number| Weekday::ALL[usize::from(*number)])
        .collect();
    let all_days = weekdays.len() == 7;
    let first_hour = *cron.hours.first()?;
    if cron.hours.len() == 1 {
        let time = Clock {
            hour: first_hour,
            minute,
        };
        return Some(if all_days {
            Schedule::Daily { time }
        } else if cron.weekdays == [1, 2, 3, 4, 5] {
            Schedule::Weekdays { time }
        } else {
            Schedule::Weekly {
                days: weekdays,
                time,
            }
        });
    }
    // Every N hours: the hours a step of N from the first one makes.
    let step = cron.hours.get(1)? - first_hour;
    let stepped: Vec<u8> = (0..24u8)
        .filter(|hour| (hour + 24 - first_hour) % step == 0)
        .collect();
    (all_days && (1..=12).contains(&step) && stepped == cron.hours).then_some(
        Schedule::EveryHours {
            hours: step,
            time: Clock {
                hour: first_hour,
                minute,
            },
        },
    )
}

fn next_of(cron: &Cron, after_unix_ms: u64, zone: &TimeZone) -> Option<u64> {
    let after = Timestamp::from_millisecond(i64::try_from(after_unix_ms).ok()?).ok()?;
    let start = after.to_zoned(zone.clone());
    let mut date = start.date();
    // The first day's hours well before `after` cannot be next; two hours
    // spare a clock change. A five-minute schedule's missed times are
    // counted by this walk under the core's lock (`take_due`).
    let first_hour = u8::try_from(start.hour()).unwrap_or(0).saturating_sub(2);
    for day in 0..HORIZON_DAYS {
        if day_matches(cron, date) {
            for hour in &cron.hours {
                if day == 0 && *hour < first_hour {
                    continue;
                }
                for minute in &cron.minutes {
                    let local = date.at(*hour as i8, *minute as i8, 0, 0);
                    // A skipped time runs where the clock jumps to; a
                    // repeated one at its first instant.
                    let Ok(at) = zone.to_ambiguous_zoned(local).compatible() else {
                        continue;
                    };
                    let at = at.timestamp();
                    if at > after {
                        return u64::try_from(at.as_millisecond()).ok();
                    }
                }
            }
        }
        date = date.tomorrow().ok()?;
    }
    None
}

fn day_matches(cron: &Cron, date: Date) -> bool {
    let month = u8::try_from(date.month()).unwrap_or(0);
    if !cron.months.contains(&month) {
        return false;
    }
    let day = cron.days.contains(&u8::try_from(date.day()).unwrap_or(0));
    let weekday = cron
        .weekdays
        .contains(&u8::try_from(date.weekday().to_sunday_zero_offset()).unwrap_or(7));
    match (cron.days_restricted, cron.weekdays_restricted) {
        (true, true) => day || weekday,
        (true, false) => day,
        (false, true) => weekday,
        (false, false) => true,
    }
}

/// Reads a five-field cron line: numbers, `*`, lists, ranges and steps, and
/// three-letter month and day names. Seconds, years and the `@` forms are
/// not cron's five fields and are refused.
fn parse_cron(expr: &str) -> Result<Cron, &'static str> {
    const INVALID: &str = "schedule_cron_invalid";
    if expr.len() > CRON_CAP {
        return Err(INVALID);
    }
    let fields: Vec<&str> = expr.split_whitespace().collect();
    let [minutes, hours, days, months, weekdays] = fields.as_slice() else {
        return Err(INVALID);
    };
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
    let mut weekday_set = field(weekdays, 0, 7, &DAYS, 0)?;
    // 7 is Sunday too.
    if let Some(seven) = weekday_set.iter().position(|day| *day == 7) {
        weekday_set.remove(seven);
        if !weekday_set.contains(&0) {
            weekday_set.insert(0, 0);
        }
    }
    Ok(Cron {
        minutes: field(minutes, 0, 59, &[], 0)?,
        hours: field(hours, 0, 23, &[], 0)?,
        days: field(days, 1, 31, &[], 1)?,
        months: field(months, 1, 12, &MONTHS, 1)?,
        days_restricted: !days.starts_with('*'),
        weekdays_restricted: !weekdays.starts_with('*'),
        weekdays: weekday_set,
    })
}

/// One field's values, sorted: `*`, `a`, `a-b`, each with an optional `/s`,
/// joined by `,`. `names` spell the values from `first` on.
fn field(text: &str, min: u8, max: u8, names: &[&str], first: u8) -> Result<Vec<u8>, &'static str> {
    const INVALID: &str = "schedule_cron_invalid";
    let value = |part: &str| -> Result<u8, &'static str> {
        let lower = part.to_ascii_lowercase();
        if let Some(index) = names.iter().position(|name| *name == lower) {
            return Ok(first + index as u8);
        }
        (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse::<u8>().ok())
            .flatten()
            .filter(|value| (min..=max).contains(value))
            .ok_or(INVALID)
    };
    let mut values = Vec::new();
    for item in text.split(',') {
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => {
                let step = step.parse::<u8>().map_err(|_| INVALID)?;
                if step == 0 {
                    return Err(INVALID);
                }
                (range, step)
            }
            None => (item, 1),
        };
        let (low, high) = if range == "*" {
            (min, max)
        } else if let Some((low, high)) = range.split_once('-') {
            (value(low)?, value(high)?)
        } else {
            let low = value(range)?;
            // `a/s` runs from `a` to the field's end.
            (low, if item.contains('/') { max } else { low })
        };
        if low > high {
            return Err(INVALID);
        }
        values.extend((low..=high).step_by(usize::from(step)));
    }
    values.sort_unstable();
    values.dedup();
    if values.is_empty() {
        return Err(INVALID);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seoul() -> TimeZone {
        TimeZone::get("Asia/Seoul").unwrap()
    }

    fn new_york() -> TimeZone {
        TimeZone::get("America/New_York").unwrap()
    }

    /// Unix ms of a wall time in `zone`.
    fn at(zone: &TimeZone, text: &str) -> u64 {
        let local: jiff::civil::DateTime = text.parse().unwrap();
        u64::try_from(
            zone.to_ambiguous_zoned(local)
                .compatible()
                .unwrap()
                .timestamp()
                .as_millisecond(),
        )
        .unwrap()
    }

    fn time(text: &str) -> Clock {
        Clock::parse(text).unwrap()
    }

    fn cron(expr: &str) -> Schedule {
        Schedule::Cron { expr: expr.into() }
    }

    #[test]
    fn weekdays_skip_the_weekend() {
        let zone = seoul();
        let schedule = Schedule::Weekdays {
            time: time("09:00"),
        };
        // Friday 2026-11-20 10:00 → Monday 2026-11-23 09:00.
        let next = schedule.next_after(at(&zone, "2026-11-20T10:00"), &zone);
        assert_eq!(next, Some(at(&zone, "2026-11-23T09:00")));
        // Thursday 08:59 → the same morning.
        let next = schedule.next_after(at(&zone, "2026-11-19T08:59"), &zone);
        assert_eq!(next, Some(at(&zone, "2026-11-19T09:00")));
    }

    #[test]
    fn a_next_time_is_strictly_after_the_given_one() {
        let zone = seoul();
        let schedule = Schedule::Daily {
            time: time("09:00"),
        };
        let nine = at(&zone, "2026-11-19T09:00");
        assert_eq!(
            schedule.next_after(nine, &zone),
            Some(at(&zone, "2026-11-20T09:00"))
        );
    }

    #[test]
    fn every_three_hours_counts_from_the_chosen_hour_around_the_day() {
        let zone = seoul();
        let schedule = Schedule::EveryHours {
            hours: 3,
            time: time("09:30"),
        };
        let next = schedule.next_after(at(&zone, "2026-11-19T22:00"), &zone);
        assert_eq!(next, Some(at(&zone, "2026-11-20T00:30")));
        let next = schedule.next_after(at(&zone, "2026-11-19T09:31"), &zone);
        assert_eq!(next, Some(at(&zone, "2026-11-19T12:30")));
    }

    #[test]
    fn a_weekly_choice_runs_on_each_chosen_day() {
        let zone = seoul();
        let schedule = Schedule::Weekly {
            days: vec![Weekday::Tue, Weekday::Sat],
            time: time("07:15"),
        };
        // Thursday → Saturday, then Saturday → Tuesday.
        let saturday = at(&zone, "2026-11-21T07:15");
        assert_eq!(
            schedule.next_after(at(&zone, "2026-11-19T12:00"), &zone),
            Some(saturday)
        );
        assert_eq!(
            schedule.next_after(saturday, &zone),
            Some(at(&zone, "2026-11-24T07:15"))
        );
    }

    /// B7, D-16: a schedule that would run two times closer than five
    /// minutes, an unreadable line, a week with no day and periods the form
    /// does not offer are refused with why.
    #[test]
    fn a_schedule_that_cannot_run_is_refused_with_why() {
        let zone = seoul();
        for (schedule, reason) in [
            (cron("*/4 * * * *"), "schedule_too_often"),
            (cron("0,3 9 * * *"), "schedule_too_often"),
            (cron("* 9 * * *"), "schedule_too_often"),
            // 09:58 and then 10:00.
            (cron("0,58 9,10 * * *"), "schedule_too_often"),
            (cron("0,58 23,0 * * *"), "schedule_too_often"),
            (
                Schedule::EveryMinutes { minutes: 7 },
                "schedule_minutes_out_of_range",
            ),
            (cron("0 9 * *"), "schedule_cron_invalid"),
            (cron("0 25 * * *"), "schedule_cron_invalid"),
            (cron("@daily"), "schedule_cron_invalid"),
            (cron("0 9 30 2 *"), "schedule_never"),
            (
                Schedule::Weekly {
                    days: vec![],
                    time: time("09:00"),
                },
                "schedule_days_missing",
            ),
            (
                Schedule::EveryHours {
                    hours: 13,
                    time: time("09:00"),
                },
                "schedule_hours_out_of_range",
            ),
        ] {
            assert_eq!(schedule.validate(&zone), Err(reason), "{schedule:?}");
        }
        assert_eq!(Clock::parse("9:00"), Err("schedule_time_invalid"));
        assert_eq!(Clock::parse("24:00"), Err("schedule_time_invalid"));
        assert!(cron("0 */2 * * *").validate(&zone).is_ok());
        assert!(cron("*/5 * * * *").validate(&zone).is_ok());
        assert!(cron("0,58 9 * * *").validate(&zone).is_ok());
        for minutes in EVERY_MINUTES {
            assert!(Schedule::EveryMinutes { minutes }.validate(&zone).is_ok());
        }
    }

    /// D-06, D-16: every five minutes runs at each fifth minute of every
    /// hour, past midnight too.
    #[test]
    fn every_few_minutes_runs_from_each_hours_start() {
        let zone = seoul();
        let schedule = Schedule::EveryMinutes { minutes: 15 };
        let next = schedule.next_after(at(&zone, "2026-11-19T09:07"), &zone);
        assert_eq!(next, Some(at(&zone, "2026-11-19T09:15")));
        let next = schedule.next_after(at(&zone, "2026-11-19T23:50"), &zone);
        assert_eq!(next, Some(at(&zone, "2026-11-20T00:00")));
        let five = Schedule::EveryMinutes { minutes: 5 };
        let next = five.next_after(at(&zone, "2026-11-19T21:59"), &zone);
        assert_eq!(next, Some(at(&zone, "2026-11-19T22:00")));
    }

    /// B6: a cron line that equals a choice reads as that choice.
    #[test]
    fn a_cron_line_reads_as_the_choice_it_equals() {
        let cases = [
            (
                "0 9 * * 1-5",
                Schedule::Weekdays {
                    time: time("09:00"),
                },
            ),
            (
                "30 8 * * *",
                Schedule::Daily {
                    time: time("08:30"),
                },
            ),
            (
                "0 7 * * sat,sun",
                Schedule::Weekly {
                    days: vec![Weekday::Sun, Weekday::Sat],
                    time: time("07:00"),
                },
            ),
            (
                "15 1/4 * * *",
                Schedule::EveryHours {
                    hours: 4,
                    time: time("01:15"),
                },
            ),
            ("*/10 * * * *", Schedule::EveryMinutes { minutes: 10 }),
            ("0,30 * * * *", Schedule::EveryMinutes { minutes: 30 }),
            ("*/10 9 * * *", cron("*/10 9 * * *")),
            ("0 9 1 * *", cron("0 9 1 * *")),
        ];
        for (expr, choice) in cases {
            assert_eq!(cron(expr).meaning().reads_as, choice, "{expr}");
        }
    }

    /// The zone's own rules: a time a spring-forward skips runs where the
    /// clock lands, a repeated time once.
    #[test]
    fn daylight_saving_edges_run_once_and_never_vanish() {
        let zone = new_york();
        let schedule = Schedule::Daily {
            time: time("02:30"),
        };
        // 2026-03-08 02:30 does not exist in New York: it runs at 03:30.
        let next = schedule
            .next_after(at(&zone, "2026-03-07T12:00"), &zone)
            .unwrap();
        assert_eq!(next, at(&zone, "2026-03-08T03:30"));
        let one_thirty = Schedule::Daily {
            time: time("01:30"),
        };
        // 2026-11-01 01:30 happens twice; the run is at the first, and the
        // next one is the following day.
        let first = one_thirty
            .next_after(at(&zone, "2026-10-31T12:00"), &zone)
            .unwrap();
        let second = one_thirty.next_after(first, &zone).unwrap();
        assert_eq!(second - first, 25 * 3_600_000);
    }

    #[test]
    fn a_schedule_round_trips_as_the_operator_chose_it() {
        let schedule = Schedule::Weekly {
            days: vec![Weekday::Mon, Weekday::Fri],
            time: time("09:05"),
        };
        let text = serde_json::to_string(&schedule).unwrap();
        assert_eq!(
            text,
            r#"{"kind":"weekly","days":["mon","fri"],"time":"09:05"}"#
        );
        assert_eq!(serde_json::from_str::<Schedule>(&text).unwrap(), schedule);
    }
}
