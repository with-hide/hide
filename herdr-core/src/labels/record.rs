//! Display history has no authority over the current pane or turn.

use crate::links::ledger::SessionMetadata;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordConnection {
    Checking,
    Confirmed,
    Unconfirmed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecordBinding {
    pub provider: String,
    /// The Herdr reference observed when this result was accepted. Even a
    /// display-only result cannot cross a later reported session change.
    pub reported_reference: Option<String>,
    pub connection: RecordConnection,
    pub metadata: SessionMetadata,
}

impl SessionMetadata {
    pub(crate) fn of_label(record: &super::store::PaneRecord) -> Self {
        Self {
            native_title: record.facts.title.clone(),
            manual_title: record.facts.custom_title.clone(),
            goal: record.goal.clone(),
            summary: record.line.clone(),
            created_prs: record
                .facts
                .created_prs
                .iter()
                .map(|pr| (pr.repository.clone(), pr.number, pr.sighted_at_unix_ms))
                .collect(),
            analysed_start: record.analysis_turn_start,
            analysed_end: record.analysis_turn_end,
        }
    }
}
