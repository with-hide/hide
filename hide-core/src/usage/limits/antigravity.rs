//! `agy -p "/usage" --output-format json` (agy 1.4.0): each group of models
//! shares weekly buckets that say what fraction is left and when it refills.
//! The Gemini group is what agy's own models spend, so it is the account's
//! weekly window; every other group is a window scoped to its models.
use super::*;
use serde_json::Value;

const GEMINI: &str = "gemini-weekly";

pub(super) fn parse(body: &str, now_s: u64) -> Result<LimitReading, Cause> {
    let value: Value = serde_json::from_str(body).map_err(|_| Cause::FormatChanged)?;
    let groups = value
        .pointer("/command/data/groups")
        .and_then(Value::as_array)
        .ok_or(Cause::FormatChanged)?;
    let mut windows = Vec::new();
    for group in groups {
        for bucket in group
            .get("buckets")
            .and_then(Value::as_array)
            .ok_or(Cause::FormatChanged)?
        {
            if bucket.get("window").and_then(Value::as_str) != Some("weekly") {
                continue;
            }
            let scope = if bucket.get("id").and_then(Value::as_str) == Some(GEMINI) {
                Scope::All
            } else {
                Scope::Model {
                    label: group
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|name| {
                            !name.trim().is_empty()
                                && name.len() <= 128
                                && !name.chars().any(char::is_control)
                        })
                        .ok_or(Cause::FormatChanged)?
                        .to_owned(),
                }
            };
            let left = bucket
                .get("remaining_fraction")
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && (0.0..=1.0).contains(n));
            let reset = bucket
                .get("reset_time")
                .and_then(Value::as_str)
                .and_then(hide_session::parse_rfc3339);
            let outcome = match (left, reset) {
                (Some(left), Some(resets_at_s)) => WindowOutcome::Open {
                    used_percent: ((1.0 - left) * 100.0).clamp(0.0, 100.0),
                    resets_at_s,
                },
                _ => WindowOutcome::Unreadable {
                    cause: Cause::FormatChanged,
                },
            };
            if windows.iter().any(|window: &WindowReading| {
                window.kind == WindowKind::Weekly && window.scope == scope
            }) {
                return Err(Cause::FormatChanged);
            }
            windows.push(WindowReading {
                kind: WindowKind::Weekly,
                scope,
                outcome,
            });
        }
    }
    if windows.is_empty() {
        return Err(Cause::FormatChanged);
    }
    complete(LimitSource::AntigravityUsage, windows, now_s)
}
