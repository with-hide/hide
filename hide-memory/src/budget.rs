//! The two consumers of Memory's single UTC-day call budget, and the pace
//! one session's background analysis and every session's together keep.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryCallKind {
    Analysis,
    Check,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryCallDeferral {
    DailyCap,
    CheckShareSpent,
    ReservedForChecks,
    /// The session made [`crate::SESSION_DAILY_CALL_LIMIT`] calls this UTC
    /// day; the other sessions' turns go on.
    SessionDay,
    /// Background analysis made [`crate::HOURLY_CALL_LIMIT`] calls in the
    /// last hour, every session together; the whole queue waits.
    Hour,
}

/// The session one background analysis call is made for.
#[derive(Clone, Copy, Debug)]
pub struct SessionCall<'a> {
    pub project: &'a str,
    pub provider: &'a str,
    pub session: &'a str,
}

/// The next reset of the same UTC-day counters admission reads.
pub fn memory_call_reset_at(now_unix_ms: u64) -> u64 {
    (now_unix_ms / crate::store::DAY_MS)
        .saturating_add(1)
        .saturating_mul(crate::store::DAY_MS)
}

pub(crate) fn deferral(
    total: u64,
    checks: u64,
    pending: u64,
    kind: MemoryCallKind,
) -> Option<MemoryCallDeferral> {
    if total >= crate::DAILY_ANALYSIS_CALL_LIMIT {
        return Some(MemoryCallDeferral::DailyCap);
    }
    let remaining = crate::MAINTENANCE_CALL_LIMIT.saturating_sub(checks);
    match kind {
        MemoryCallKind::Check if remaining == 0 => Some(MemoryCallDeferral::CheckShareSpent),
        MemoryCallKind::Analysis
            if crate::DAILY_ANALYSIS_CALL_LIMIT - total <= pending.min(remaining) =>
        {
            Some(MemoryCallDeferral::ReservedForChecks)
        }
        _ => None,
    }
}

pub(crate) fn pace(session_today: u64, last_hour: u64) -> Option<MemoryCallDeferral> {
    if session_today >= crate::SESSION_DAILY_CALL_LIMIT {
        Some(MemoryCallDeferral::SessionDay)
    } else if last_hour >= crate::HOURLY_CALL_LIMIT {
        Some(MemoryCallDeferral::Hour)
    } else {
        None
    }
}
