//! Values shared by internal-work collectors. A work item belongs to its
//! native session; it is never a separately registered Hide agent or pane.
//! Collection, persistence and state decisions remain with their owners.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    Subagent,
    Background,
}

/// A reported stop is an end of an execution, not proof that its parent's
/// assignment succeeded. Expiry is distinct from a reported end.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkState {
    #[default]
    Running,
    Ended,
    Stopped,
    Expired,
}

/// A native execution inside its enclosing session. Legacy checkpoints may
/// know an id without knowing its kind. Names are native descriptions, never
/// guessed from prompts, commands or output.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkRecord {
    pub id: String,
    pub kind: Option<WorkKind>,
    pub name: Option<String>,
    pub state: WorkState,
    pub expires_at_unix_ms: Option<u64>,
}

/// Totals from a hook or plugin. `done` counts ended executions, not
/// successful assignments. Whether totals are known is carried separately.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkCounts {
    #[serde(default)]
    pub working: u32,
    #[serde(default)]
    pub done: u32,
}

/// A collector may know only one total. Missing stays unknown even when
/// another total is a confirmed zero.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct WorkTotals {
    pub working: Option<u32>,
    pub done: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkSource {
    Hooks,
    Session,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkSupport {
    #[default]
    Unsupported,
    Counts,
    Records,
    CountsAndRecords,
}

/// An observation is current only within the native session and state its
/// collector proved. Absence of an observation never means a zero count.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WorkRead<T> {
    Unsupported,
    #[default]
    Unknown,
    Current {
        source: WorkSource,
        value: T,
    },
    Stale {
        source: WorkSource,
    },
    Lost {
        source: WorkSource,
    },
}

impl<T> WorkRead<T> {
    pub fn current(&self) -> Option<&T> {
        match self {
            Self::Current { value, .. } => Some(value),
            _ => None,
        }
    }
}

/// The native session's asynchronous executions, not an inventory of every
/// synchronous tool or subagent. Hook totals and these records overlap and
/// must never be added together. End history is bounded independently.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct WorkRecords {
    pub records: Vec<WorkRecord>,
    pub history_truncated: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct InternalWork {
    pub subagents: WorkRead<WorkTotals>,
    pub executions: WorkRead<WorkRecords>,
}

impl crate::AgentAdapter {
    /// Select implemented collectors, without starting one or deriving
    /// support from an agent name. Session records may describe subagents
    /// as well as commands; hook totals remain the complete subagent count.
    pub const fn work_support(&self, kind: WorkKind) -> WorkSupport {
        let records = matches!(self.session, Some(format) if format.reports_wake_devices());
        match kind {
            WorkKind::Subagent if self.subagent_counts.is_some() && records => {
                WorkSupport::CountsAndRecords
            }
            WorkKind::Subagent if self.subagent_counts.is_some() => WorkSupport::Counts,
            WorkKind::Subagent | WorkKind::Background if records => WorkSupport::Records,
            WorkKind::Background => WorkSupport::Unsupported,
            WorkKind::Subagent => WorkSupport::Unsupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_support_distinguishes_counts_records_and_absence() {
        for (kind, subagents, background) in [
            (
                "claude",
                WorkSupport::CountsAndRecords,
                WorkSupport::Records,
            ),
            ("codex", WorkSupport::Counts, WorkSupport::Unsupported),
            ("grok", WorkSupport::Counts, WorkSupport::Unsupported),
            ("opencode", WorkSupport::Counts, WorkSupport::Unsupported),
            ("omp", WorkSupport::Counts, WorkSupport::Unsupported),
            ("agy", WorkSupport::Counts, WorkSupport::Unsupported),
            ("pi", WorkSupport::Unsupported, WorkSupport::Unsupported),
        ] {
            let adapter = crate::adapter(kind).unwrap();
            assert_eq!(
                adapter.work_support(WorkKind::Subagent),
                subagents,
                "{kind}"
            );
            assert_eq!(
                adapter.work_support(WorkKind::Background),
                background,
                "{kind}"
            );
        }
    }
}
