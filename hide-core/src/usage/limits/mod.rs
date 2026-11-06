//! Pure parsers. Transport, scheduling and UI wording do not belong here.
mod antigravity;
mod claude;
mod codex;
use super::model::{Cause, LimitReading, Scope, WindowKind, WindowOutcome, WindowReading};
use hide_agent_adapter::usage::{LimitSource, Presence};

pub(super) fn parse(source: LimitSource, body: &str, now_s: u64) -> Result<LimitReading, Cause> {
    match source {
        LimitSource::ClaudeUsage => claude::parse(body, now_s),
        LimitSource::CodexUsage => codex::parse(body, now_s),
        LimitSource::AntigravityUsage => antigravity::parse(body, now_s),
    }
}
fn complete(
    source: LimitSource,
    mut windows: Vec<WindowReading>,
    now_s: u64,
) -> Result<LimitReading, Cause> {
    if !windows.iter().any(|window| {
        matches!(
            window.outcome,
            WindowOutcome::Open { .. } | WindowOutcome::NotStarted | WindowOutcome::Expired { .. }
        )
    }) {
        return Err(Cause::FormatChanged);
    }
    for kind in [WindowKind::FiveHour, WindowKind::Weekly] {
        if !windows
            .iter()
            .any(|w| w.kind == kind && w.scope == Scope::All)
        {
            windows.push(WindowReading {
                kind,
                scope: Scope::All,
                outcome: match source.presence(kind) {
                    Presence::Always => WindowOutcome::Unreadable {
                        cause: Cause::FormatChanged,
                    },
                    Presence::PlanDependent => WindowOutcome::NotOffered,
                },
            });
        }
    }
    if windows.len() > 16 {
        return Err(Cause::Capped);
    }
    windows.sort_by_key(|w| {
        (
            match w.kind {
                WindowKind::FiveHour => 0,
                WindowKind::Weekly => 1,
                WindowKind::Monthly => 2,
            },
            w.scope != Scope::All,
        )
    });
    let mut reading = LimitReading { windows };
    reading.expire(now_s);
    Ok(reading)
}
#[cfg(test)]
mod tests;
