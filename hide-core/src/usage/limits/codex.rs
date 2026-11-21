use super::*;
use serde_json::Value;
pub(super) fn parse(body: &str, now_s: u64) -> Result<LimitReading, Cause> {
    let value: Value = serde_json::from_str(body).map_err(|_| Cause::FormatChanged)?;
    let limits = value.get("rate_limit").unwrap_or(&value);
    let mut windows: Vec<WindowReading> = Vec::new();
    for name in ["primary_window", "secondary_window"] {
        let Some(window) = limits.get(name).filter(|w| !w.is_null()) else {
            continue;
        };
        let kind = match window.get("limit_window_seconds").and_then(Value::as_u64) {
            Some(18_000) => WindowKind::FiveHour,
            Some(604_800) => WindowKind::Weekly,
            _ => continue,
        };
        let used = window
            .get("used_percent")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && (0.0..=100.0).contains(n));
        let reset = window
            .get("reset_at")
            .or_else(|| window.get("resets_at"))
            .and_then(Value::as_u64);
        let outcome = match (used, reset) {
            (Some(used_percent), Some(resets_at_s)) => WindowOutcome::Open {
                used_percent,
                resets_at_s,
            },
            (Some(0.0), None) if kind == WindowKind::FiveHour => WindowOutcome::NotStarted,
            _ => WindowOutcome::Unreadable {
                cause: Cause::FormatChanged,
            },
        };
        if let Some(previous) = windows.iter_mut().find(|w| w.kind == kind) {
            previous.outcome = WindowOutcome::Unreadable {
                cause: Cause::FormatChanged,
            };
        } else {
            windows.push(WindowReading {
                kind,
                scope: Scope::All,
                outcome,
            });
        }
    }
    if windows.is_empty() {
        return Err(Cause::FormatChanged);
    }
    complete(LimitSource::CodexUsage, windows, now_s)
}
