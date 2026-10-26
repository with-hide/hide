//! The record ledger (PRD hide-ai-suggestions B4, B5): what went wrong in a
//! Project, as facts the core already has, counted once per session or pull
//! request and never through a new AI call. A record is a fact with a key,
//! never the words around it; the investigation reads those through its own
//! tools.

use super::policy;
use serde::{Deserialize, Serialize};

/// A record's number inside its Project, never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecordId(pub u64);

/// Where a record came from (D-04, D-19, D-23, D-30, D-45). Each has its
/// own switch (B5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    /// A tool call or command still failed when its turn ended.
    FailedTool,
    /// An agent stopped on a block or a question for the operator.
    AgentWait,
    /// The operator corrected an agent, as Memory learned it at turn end.
    Correction,
    /// A problem the label analysis said was left over at a turn's end.
    LabelMissed,
    /// A pull request's check failed.
    CheckFailed,
    /// A review asked for changes and the pull request was merged after.
    ChangeRequest,
}

impl Signal {
    pub(crate) const ALL: [Self; 6] = [
        Self::FailedTool,
        Self::AgentWait,
        Self::Correction,
        Self::LabelMissed,
        Self::CheckFailed,
        Self::ChangeRequest,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FailedTool => "failed_tool",
            Self::AgentWait => "agent_wait",
            Self::Correction => "correction",
            Self::LabelMissed => "label_missed",
            Self::CheckFailed => "check_failed",
            Self::ChangeRequest => "change_request",
        }
    }

    /// Whether the record is read from GitHub, which rests while no GitHub
    /// connection answers (D-41).
    pub(crate) fn needs_github(self) -> bool {
        matches!(self, Self::CheckFailed | Self::ChangeRequest)
    }

    /// Whether it is counted by session rather than by pull request.
    pub(crate) fn per_session(self) -> bool {
        !self.needs_github()
    }
}

/// The agent session a record happened in, by the native ids the session
/// readers use.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SessionRef {
    pub agent: String,
    pub id: String,
}

/// A fact before the store numbered it: what a source hands the ledger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub signal: Signal,
    /// The key the record is grouped by without a model: a command's head,
    /// a check's name, a wait's cause, a card's number.
    pub tag: String,
    pub session: Option<SessionRef>,
    pub pr: Option<PrRef>,
    /// The checkout the record happened in, by its path on the Project's
    /// machine; a watch after a merge counts only checkouts holding it.
    pub checkout: Option<String>,
    pub at_unix_ms: u64,
    /// The operator said it should hold from now on ("앞으로 항상"), which
    /// lets one record cross the threshold (D-11).
    #[serde(default)]
    pub always: bool,
    /// The head commit of the pull request a GitHub record was seen on, so
    /// a watch after a merge can ask whether that head had the fix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
}

/// A pull request a record names.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PrRef {
    pub number: u32,
    pub url: String,
}

/// A numbered fact in the ledger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: RecordId,
    #[serde(flatten)]
    pub fact: Observation,
}

impl Observation {
    /// The key a second sighting folds into (B4: one count per session, or
    /// per pull request for a GitHub record). `None` for a fact missing the
    /// session or pull request its signal is counted by, which is refused.
    pub(crate) fn key(&self) -> Option<String> {
        let unit = if self.signal.per_session() {
            let session = self.session.as_ref()?;
            format!("s:{}:{}", session.agent, session.id)
        } else {
            format!("p:{}", self.pr.as_ref()?.number)
        };
        Some(format!(
            "{}\u{1f}{}\u{1f}{unit}",
            self.signal.as_str(),
            self.tag
        ))
    }

    /// The fact as the ledger keeps it: its tag cut to [`policy::TAG_CHARS`]
    /// with control characters dropped, or `None` when nothing is left.
    pub(crate) fn normalized(mut self) -> Option<Self> {
        self.tag = normalize_tag(&self.tag)?;
        Some(self)
    }
}

/// A tag as it is compared: trimmed, inner runs of white space made one
/// space, no control characters, at most [`policy::TAG_CHARS`].
pub(crate) fn normalize_tag(tag: &str) -> Option<String> {
    let words = tag
        .split_whitespace()
        .map(|word| word.chars().filter(|c| !c.is_control()).collect::<String>())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let cut = words.chars().take(policy::TAG_CHARS).collect::<String>();
    (!cut.is_empty()).then_some(cut)
}

/// One record as the investigation is handed it: its number and its counts,
/// never its words (D-32).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordEntry {
    pub id: RecordId,
    pub signal: Signal,
    pub tag: String,
    pub session: Option<SessionRef>,
    pub pr: Option<u32>,
    pub at_unix_ms: u64,
    pub always: bool,
    /// The line it is on, if any.
    pub line: Option<super::notebook::LineId>,
    /// No run has attached or passed it yet.
    pub fresh: bool,
}

/// The records an investigation runs over (the boundary's input).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecordIndex {
    pub entries: Vec<RecordEntry>,
}

impl RecordIndex {
    pub(crate) fn get(&self, id: RecordId) -> Option<&RecordEntry> {
        self.entries
            .binary_search_by_key(&id, |entry| entry.id)
            .ok()
            .map(|at| &self.entries[at])
    }

    pub(crate) fn fresh(&self) -> impl Iterator<Item = &RecordEntry> {
        self.entries.iter().filter(|entry| entry.fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(tag: &str, session: &str) -> Observation {
        Observation {
            signal: Signal::FailedTool,
            tag: tag.into(),
            session: Some(SessionRef {
                agent: "claude".into(),
                id: session.into(),
            }),
            pr: None,
            checkout: None,
            at_unix_ms: 1,
            always: false,
            head: None,
        }
    }

    #[test]
    fn a_fact_folds_once_per_session_or_pull_request() {
        assert_eq!(
            failed("cargo test", "a").key(),
            failed("cargo test", "a").key()
        );
        assert_ne!(
            failed("cargo test", "a").key(),
            failed("cargo test", "b").key()
        );
        assert_ne!(
            failed("cargo test", "a").key(),
            failed("cargo fmt", "a").key()
        );
        let check = |number| Observation {
            signal: Signal::CheckFailed,
            tag: "verify".into(),
            session: None,
            pr: Some(PrRef {
                number,
                url: format!("https://github.com/o/r/pull/{number}"),
            }),
            checkout: None,
            at_unix_ms: 1,
            always: false,
            head: None,
        };
        assert_eq!(check(3).key(), check(3).key());
        assert_ne!(check(3).key(), check(4).key());
        let mut missing = failed("cargo test", "a");
        missing.session = None;
        assert_eq!(missing.key(), None, "a session signal names its session");
    }

    #[test]
    fn a_tag_is_compared_trimmed_and_bounded() {
        assert_eq!(
            normalize_tag("  cargo \t test\n"),
            Some("cargo test".into())
        );
        assert_eq!(normalize_tag(" \u{7} "), None);
        assert_eq!(
            normalize_tag(&"x".repeat(500)).map(|tag| tag.chars().count()),
            Some(policy::TAG_CHARS)
        );
    }
}
