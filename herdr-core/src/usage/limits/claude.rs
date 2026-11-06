use super::*;
use crate::usage::retry::parse_http_month;
use crate::zoneinfo::{Zone, civil_from_days, days_from_civil, days_in_month};
use serde_json::Value;
const RESET_HORIZON_SECONDS: i64 = 8 * 86_400;

/// What the core's own node read for Claude Code: the account endpoint's
/// windows (`usage_api`), or the text `/usage` printed when that read failed
/// (`usage_text`).
pub(super) fn parse(body: &str, now_s: u64) -> Result<LimitReading, Cause> {
    let value: Value = serde_json::from_str(body).map_err(|_| Cause::FormatChanged)?;
    if let Some(windows) = value.get("usage_api") {
        return parse_api(windows, now_s);
    }
    match value.get("usage_text").and_then(Value::as_str) {
        Some(text) => parse_text(text, now_s),
        None => Err(Cause::FormatChanged),
    }
}

/// The endpoint's windows as the node copied them: `five_hour` and
/// `seven_day` for all models, null when the plan has no such window, and
/// `scoped`, the weekly windows of one model each.
fn parse_api(value: &Value, now_s: u64) -> Result<LimitReading, Cause> {
    let mut windows = Vec::new();
    for (name, kind) in [
        ("five_hour", WindowKind::FiveHour),
        ("seven_day", WindowKind::Weekly),
    ] {
        if let Some(window) = value.get(name).filter(|window| !window.is_null()) {
            windows.push(WindowReading {
                kind,
                scope: Scope::All,
                outcome: api_outcome(kind, window),
            });
        }
    }
    let scoped = value.get("scoped").and_then(Value::as_array);
    for window in scoped.into_iter().flatten() {
        // A window scoped to something other than a model has no row.
        let Some(label) = window.get("label").and_then(Value::as_str) else {
            continue;
        };
        if label.len() > 64 {
            return Err(Cause::Capped);
        }
        if windows.len() >= 16 {
            return Err(Cause::Capped);
        }
        let scope = Scope::Model {
            label: label.into(),
        };
        let outcome = api_outcome(WindowKind::Weekly, window);
        if let Some(previous) = windows
            .iter_mut()
            .find(|w: &&mut WindowReading| w.kind == WindowKind::Weekly && w.scope == scope)
        {
            previous.outcome = WindowOutcome::Unreadable {
                cause: Cause::FormatChanged,
            };
        } else {
            windows.push(WindowReading {
                kind: WindowKind::Weekly,
                scope,
                outcome,
            });
        }
    }
    complete(LimitSource::ClaudeUsage, windows, now_s)
}

/// A percent from 0 to 100 and an RFC 3339 reset; a zero five-hour window
/// with no reset has not started, and anything else missing is unreadable.
fn api_outcome(kind: WindowKind, window: &Value) -> WindowOutcome {
    let used = window
        .get("used_percent")
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && (0.0..=100.0).contains(n));
    let reset = window
        .get("resets_at")
        .and_then(Value::as_str)
        .and_then(hide_session::parse_rfc3339);
    match (used, reset) {
        (Some(used_percent), Some(resets_at_s)) => WindowOutcome::Open {
            used_percent,
            resets_at_s,
        },
        (Some(0.0), None) if kind == WindowKind::FiveHour => WindowOutcome::NotStarted,
        _ => WindowOutcome::Unreadable {
            cause: Cause::FormatChanged,
        },
    }
}

/// The text `claude -p /usage` prints.
fn parse_text(text: &str, now_s: u64) -> Result<LimitReading, Cause> {
    let mut windows = Vec::new();
    let mut recognized = false;
    let mut valid_lines = 0;
    for line in text
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("Current "))
    {
        recognized = true;
        let (kind, scope, canonical) = if let Some(body) = line.strip_prefix("Current session:") {
            (
                WindowKind::FiveHour,
                Scope::All,
                format!("Current week (all models):{body}"),
            )
        } else if let Some(rest) = line.strip_prefix("Current week (") {
            let Some((label, _)) = rest.split_once("):") else {
                continue;
            };
            if label.len() > 64 {
                return Err(Cause::Capped);
            }
            (
                WindowKind::Weekly,
                if label == "all models" {
                    Scope::All
                } else {
                    Scope::Model {
                        label: label.into(),
                    }
                },
                line.into(),
            )
        } else {
            continue;
        };
        if windows.len() >= 16 {
            return Err(Cause::Capped);
        }
        let outcome = match parse_usage_line(&canonical) {
            None => WindowOutcome::Unreadable {
                cause: Cause::FormatChanged,
            },
            Some(parsed) => {
                valid_lines += 1;
                match parsed.reset {
                    None if kind == WindowKind::FiveHour && parsed.used_percent == 0.0 => {
                        WindowOutcome::NotStarted
                    }
                    None => WindowOutcome::Unreadable {
                        cause: Cause::FormatChanged,
                    },
                    Some(reset) => match resolve_reset(&reset, now_s) {
                        Ok(resets_at_s) => WindowOutcome::Open {
                            used_percent: parsed.used_percent,
                            resets_at_s,
                        },
                        Err(cause) => WindowOutcome::Unreadable { cause },
                    },
                }
            }
        };
        if let Some(previous) = windows
            .iter_mut()
            .find(|w: &&mut WindowReading| w.kind == kind && w.scope == scope)
        {
            previous.outcome = WindowOutcome::Unreadable {
                cause: Cause::FormatChanged,
            };
        } else {
            windows.push(WindowReading {
                kind,
                scope,
                outcome,
            });
        }
    }
    if !recognized {
        return Err(Cause::SignedOut);
    }
    if valid_lines == 0 {
        return Err(Cause::FormatChanged);
    }
    complete(LimitSource::ClaudeUsage, windows, now_s)
}

/// One `Current week …` line, as printed: `Current week (<scope>): <n>%
/// (used|left)[ · resets <wall time>]`.
#[derive(Clone, Debug, PartialEq)]
struct UsageLine {
    /// `all models` or a model name.
    scope: String,
    used_percent: f64,
    /// The wall time after ` · resets `, unparsed; absent when the CLI
    /// printed no reset.
    reset: Option<String>,
}

/// A printed wall time: `<Mon> <D>[, <YYYY>] at <h>[:mm](am|pm) (<zone>)`.
#[derive(Clone, Debug, PartialEq)]
struct WallTime {
    year: Option<i64>,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    zone: String,
}

fn parse_usage_line(line: &str) -> Option<UsageLine> {
    static LINE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = LINE.get_or_init(|| {
        regex::Regex::new(
            r"^Current week \(([^)]+)\): (\d+(?:\.\d+)?)% (used|left)(?: · resets (.+))?$",
        )
        .expect("the usage line pattern compiles")
    });
    let captures = pattern.captures(line)?;
    let percent = captures[2].parse::<f64>().ok()?;
    if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
        return None;
    }
    let used_percent = if &captures[3] == "used" {
        percent
    } else {
        100.0 - percent
    };
    Some(UsageLine {
        scope: captures[1].to_owned(),
        used_percent,
        reset: captures.get(4).map(|reset| reset.as_str().to_owned()),
    })
}

fn parse_wall_time(text: &str) -> Option<WallTime> {
    static WALL_TIME: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = WALL_TIME.get_or_init(|| {
        regex::Regex::new(
            r"^([A-Z][a-z]{2}) (\d{1,2})(?:, (\d{4}))? at (\d{1,2})(?::(\d{2}))?(am|pm) \(([^)]+)\)$",
        )
        .expect("the wall time pattern compiles")
    });
    let captures = pattern.captures(text)?;
    let month = parse_http_month(&captures[1])?;
    let day = captures[2].parse::<i64>().ok()?;
    let year = captures
        .get(3)
        .map(|year| year.as_str().parse::<i64>())
        .transpose()
        .ok()?;
    let clock_hour = captures[4].parse::<i64>().ok()?;
    if !(1..=12).contains(&clock_hour) {
        return None;
    }
    let minute = captures
        .get(5)
        .map_or(Some(0), |minute| minute.as_str().parse::<i64>().ok())?;
    if !(0..=59).contains(&minute) {
        return None;
    }
    let hour = match (&captures[6], clock_hour) {
        ("am", 12) => 0,
        ("am", hour) => hour,
        ("pm", 12) => 12,
        (_, hour) => hour + 12,
    };
    Some(WallTime {
        year,
        month,
        day,
        hour,
        minute,
        zone: captures[7].to_owned(),
    })
}

/// The instant a printed wall time names. With a year printed it is that
/// wall clock in that zone. Without one, it is the first instant at or
/// after `now` whose wall clock in that zone matches - unless the wall time
/// matched within the last window, in which case it has just passed and
/// the reset that passed is meant.
fn resolve_reset(text: &str, now_unix_seconds: u64) -> Result<u64, Cause> {
    let line = parse_wall_time(text).ok_or(Cause::FormatChanged)?;
    let now = i64::try_from(now_unix_seconds).map_err(|_| Cause::FormatChanged)?;
    let zone = Zone::load(&line.zone).map_err(|_| Cause::FormatChanged)?;
    let (year_now, _, _) = civil_from_days(
        (now + i64::from(zone.offset_at(now).map_err(|_| Cause::FormatChanged)?))
            .div_euclid(86_400),
    );
    let years = match line.year {
        Some(year) => vec![year],
        None => vec![year_now - 1, year_now, year_now + 1],
    };
    let mut candidates = Vec::new();
    for year in years {
        if !(1..=days_in_month(year, line.month)).contains(&line.day) {
            continue;
        }
        let local = days_from_civil(year, line.month, line.day) * 86_400
            + line.hour * 3_600
            + line.minute * 60;
        candidates.extend(zone.instants_of(local).map_err(|_| Cause::FormatChanged)?);
    }
    candidates.sort_unstable();
    let next = candidates.iter().copied().find(|instant| *instant >= now);
    let previous = candidates
        .iter()
        .copied()
        .rev()
        .find(|instant| *instant < now);
    let chosen = match (next, previous) {
        (_, Some(previous)) if now - previous <= RESET_HORIZON_SECONDS => previous,
        (Some(next), _) => next,
        (None, Some(previous)) => previous,
        (None, None) => return Err(Cause::FormatChanged),
    };
    u64::try_from(chosen).map_err(|_| Cause::FormatChanged)
}
