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

/// Only native-proven short display fields survive a pane. The existing reader
/// commits them through the ledger's indexed-only writer; no path or turn facts
/// enter this bounded, runtime-only pending set.
#[derive(Clone)]
pub(super) struct MetadataWrite {
    pub session: crate::links::ledger::SessionIdentity,
    pub owner: String,
    pub metadata: SessionMetadata,
    pub at_unix_ms: u64,
    pub due: std::time::Instant,
    pub merge_history: bool,
}

impl MetadataWrite {
    pub fn save(
        &self,
        client: &crate::links::worker::LinkClient,
    ) -> Result<SessionMetadata, String> {
        use crate::links::ledger::{LedgerAnswer, LedgerCall};
        if client.ledger_started_at().is_none() {
            return Err("record_ledger_unavailable".into());
        }
        let mut metadata = self.metadata.clone();
        if self.merge_history {
            let LedgerAnswer::Metadata(old) =
                client.call(LedgerCall::Metadata(self.session.clone()))?
            else {
                return Err("record_answer_mismatch".into());
            };
            if let Some(old) = old {
                // Native title and manual-title clears belong to this proven
                // file. Only analysis/PR history needs the initial ledger read.
                metadata.goal = metadata.goal.or(old.goal);
                if metadata.summary.is_empty() {
                    metadata.summary = old.summary;
                }
                metadata.analysed_start = metadata.analysed_start.or(old.analysed_start);
                metadata.analysed_end = metadata.analysed_end.or(old.analysed_end);
                for pr in old.created_prs {
                    if !metadata
                        .created_prs
                        .iter()
                        .any(|value| value.0 == pr.0 && value.1 == pr.1)
                    {
                        metadata.created_prs.push(pr);
                    }
                }
                metadata.created_prs.truncate(20);
            }
        }
        let LedgerAnswer::Metadata(Some(saved)) = client.call(LedgerCall::SaveMetadata {
            session: self.session.clone(),
            metadata: Box::new(metadata),
            at_unix_ms: self.at_unix_ms,
        })?
        else {
            return Err("record_answer_mismatch".into());
        };
        Ok(saved)
    }
}
