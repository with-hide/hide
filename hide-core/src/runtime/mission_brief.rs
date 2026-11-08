//! A node's brief as its panel shows it: the first prompt its worker was
//! spawned on, which the node keeps from the moment its Conductor binds the
//! worker (`hide mission node <key> --agent-id`).
//!
//! The delivery ledger holds the same words as a letter, but lets it go a
//! day after it finished (`delivery::RETENTION_MS`), and a mission lasts for
//! days, so the node keeps its own copy: the one place the text is read
//! from once it is bound. The map's view carries only the brief's size,
//! which says whether the panel has a brief to fold; its text rides the
//! snapshot for the one node whose folded box the operator opened, so a
//! mission of sixty-four nodes never sends sixty-four briefs of up to
//! 16 KiB with every change.

use serde::{Deserialize, Serialize};

use super::Runtime;

/// The brief the operator asked to read, for one node at a time. `text` is
/// null when the node holds none, so the panel draws nothing rather than an
/// empty box.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MissionBriefSnapshot {
    pub mission: Option<String>,
    pub node: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BriefReadPayload {
    pub mission: String,
    pub node: String,
}

impl Runtime {
    /// The brief the spawn that made registration `agent` was started on,
    /// when it submitted one: the words the worker began with.
    pub(super) fn worker_brief(&self, agent: &str) -> Option<String> {
        let ledger = self.delivery_state().ok()?;
        ledger
            .spawns
            .iter()
            .find(|spawn| spawn.brief_phase().went_in() && spawn.child.as_deref() == Some(agent))
            .and_then(|spawn| spawn.brief.clone())
    }

    /// `mission_brief_read`: the panel's folded box was opened. A copy of
    /// the node's own text, kept for the next read to replace.
    pub(super) fn mission_brief_read(&mut self, payload: BriefReadPayload) -> bool {
        let text = self
            .missions
            .book
            .get(&payload.mission)
            .and_then(|mission| mission.node(&payload.node))
            .and_then(|node| node.brief.clone());
        let next = MissionBriefSnapshot {
            mission: Some(payload.mission),
            node: Some(payload.node),
            text,
        };
        if self.snapshot.mission_brief == next {
            return false;
        }
        self.snapshot.mission_brief = next;
        true
    }
}
