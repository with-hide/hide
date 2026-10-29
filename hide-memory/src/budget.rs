//! The two consumers of Memory's single UTC-day call budget, and the pace
//! background analysis writes its groups at.

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

/// Why background analysis starts no further group now. A group is the unit
/// both limits count: one extraction call and at most one comparison call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupDeferral {
    /// The session wrote [`crate::SESSION_DAILY_GROUP_LIMIT`] groups this
    /// UTC day; its other sessions' turns go on.
    SessionDay,
    /// The last hour holds [`crate::HOURLY_GROUP_LIMIT`] groups of every
    /// session; the whole queue waits.
    Hour,
}

impl GroupDeferral {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionDay => "session_daily_groups",
            Self::Hour => "hourly_groups",
        }
    }

    pub fn limit(self) -> u64 {
        match self {
            Self::SessionDay => crate::SESSION_DAILY_GROUP_LIMIT,
            Self::Hour => crate::HOURLY_GROUP_LIMIT,
        }
    }
}

pub(crate) fn group_deferral(session_today: u64, last_hour: u64) -> Option<GroupDeferral> {
    if session_today >= crate::SESSION_DAILY_GROUP_LIMIT {
        Some(GroupDeferral::SessionDay)
    } else if last_hour >= crate::HOURLY_GROUP_LIMIT {
        Some(GroupDeferral::Hour)
    } else {
        None
    }
}
