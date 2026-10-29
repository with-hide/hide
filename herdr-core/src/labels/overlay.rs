//! What a worker's labels lay onto a session projection.
//!
//! The coordinator takes one overlay from its worker each time it publishes
//! and the runtime keeps the last one, so a projection that reaches the
//! runtime by another path (a workspace created or an agent closed, each
//! with its own fresh Herdr projection) keeps the same labels and elapsed
//! times instead of drawing provider names until the next publish. Each
//! label still carries the session it was proven for and is laid only while
//! the pane's current reference proves it (D-04), so a stale overlay can
//! never put one session's words on another.

use std::collections::HashMap;

use hide_session::label_reference_token;
use hide_session::turns::Waiting;

use super::analysis::LabelEnd;
use super::facts::SessionFacts;
use super::store::{self, PaneRecord, WakeRead};
use crate::request_view::RowFacts;
use crate::sidebar::{
    AgentLabel, SessionAgentPayload, SessionAgentSessionPayload, SessionSnapshotPayload,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LabelOverlay {
    panes: HashMap<String, OverlayPane>,
    records: HashMap<String, super::record::RecordBinding>,
}

#[derive(Clone, Debug, PartialEq)]
struct OverlayPane {
    changed_unix_ms: u64,
    /// `None` while this daemon does not generate labels (another daemon on
    /// the same Herdr holds the generator role, D-10): only elapsed time is
    /// laid.
    label: Option<ProvenLabel>,
}

#[derive(Clone, Debug, PartialEq)]
struct ProvenLabel {
    owner: Option<String>,
    native_session_id: Option<String>,
    native_source_path: Option<String>,
    proven_reference: Option<String>,
    /// `None` while the operator has turned agent summaries off (D-11): the
    /// row stands on its facts alone.
    summary: Option<Summary>,
    facts: RowFacts,
    /// What the session's last complete read says the agent waits for, with
    /// the Herdr state it was read under.
    turn: Option<(u64, Option<Waiting>)>,
    user_turn: Option<(u64, hide_session::turns::UserTurnFact)>,
    /// The expiries of the background tasks the session read proves alive,
    /// with the Herdr state it was read under.
    wake: Option<(u64, WakeRead)>,
    /// The Herdr state whose turn end the session read still owes.
    turn_end_owed: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
struct Summary {
    goal: Option<String>,
    line: String,
    end: Option<LabelEnd>,
    recent_turns: Vec<store::RecentTurn>,
}

impl LabelOverlay {
    pub(crate) fn of_records<'a>(
        records: impl IntoIterator<Item = (&'a String, &'a PaneRecord)>,
        labels_shown: bool,
        summaries: bool,
    ) -> Self {
        let panes = records
            .into_iter()
            .map(|(pane_id, record)| {
                let label = labels_shown.then(|| ProvenLabel {
                    owner: record.owner.clone(),
                    native_session_id: record.native_session_id.clone(),
                    native_source_path: record.native_source_path.clone(),
                    proven_reference: record.proven_reference.clone(),
                    summary: summaries.then(|| Summary {
                        goal: record.goal.clone(),
                        line: record.line.clone(),
                        end: record.end,
                        recent_turns: record.recent_turns.clone(),
                    }),
                    facts: row_facts(&record.facts),
                    turn: record.turn_read(),
                    user_turn: record.user_turn(),
                    wake: record.wake_read(),
                    turn_end_owed: record.turn_end_owed,
                });
                (
                    pane_id.clone(),
                    OverlayPane {
                        changed_unix_ms: record.changed_unix_ms,
                        label,
                    },
                )
            })
            .collect();
        Self {
            panes,
            records: HashMap::new(),
        }
    }

    pub(crate) fn with_bindings(
        mut self,
        records: impl IntoIterator<Item = (String, super::record::RecordBinding)>,
    ) -> Self {
        self.records = records.into_iter().collect();
        self
    }

    /// What the agent waits for in its current Herdr state, as its session
    /// read says (PRD codex-plan-approval-hold D-06, D-07): `None` when no
    /// read of the session it runs now was made for that state, or the
    /// records read do not settle it. The caller decides what not knowing
    /// means for an agent whose read reports no turns.
    pub(crate) fn waiting(&self, agent: &SessionAgentPayload) -> Option<Waiting> {
        let pane_id = agent.pane_id.as_deref().or(agent.id.as_deref())?;
        let label = self.panes.get(pane_id)?.label.as_ref()?;
        let reference = agent.agent_session.as_ref().and_then(|session| {
            label_reference_token(
                agent.agent.as_deref().unwrap_or_default(),
                &session.kind,
                &session.value,
            )
        });
        if !label.proves(reference.as_deref()) {
            return None;
        }
        let (seq, waiting) = label.turn?;
        (agent.state_change_seq == Some(seq))
            .then_some(waiting)
            .flatten()
    }

    /// The native session and its lifecycle reference this pane's label
    /// proves for `kind`'s `reported` reference, as [`Self::apply`] lays
    /// them on a row. A sleeping row is drawn after the overlay is laid, so
    /// what acts on its conversation asks here.
    pub(crate) fn proven_native(
        &self,
        pane_id: &str,
        kind: &str,
        reported: &SessionAgentSessionPayload,
    ) -> Option<(String, Option<SessionAgentSessionPayload>)> {
        let label = self.panes.get(pane_id)?.label.as_ref()?;
        let reference = label_reference_token(kind, &reported.kind, &reported.value);
        if !label.proves(reference.as_deref()) {
            return None;
        }
        let (session, reference) = label.native(kind, Some(reported));
        Some((session?, reference))
    }

    pub(crate) fn apply(&self, payload: &mut SessionSnapshotPayload) {
        self.apply_at(payload, unix_milliseconds());
    }

    /// [`Self::apply`] at a given time: a task that announced its own expiry
    /// stops counting when it passes, whatever else changed.
    fn apply_at(&self, payload: &mut SessionSnapshotPayload, now: u64) {
        for agent in &mut payload.agents {
            let awaiting_operator = matches!(
                self.waiting(agent),
                Some(Waiting::Question | Waiting::PlanApproval)
            );
            let Some(pane_id) = agent.pane_id.as_deref().or(agent.id.as_deref()) else {
                continue;
            };
            let current_provider = agent
                .agent
                .as_deref()
                .and_then(hide_session::Agent::from_kind);
            let current_reference = agent.agent_session.as_ref().and_then(|r| {
                label_reference_token(
                    agent.agent.as_deref().unwrap_or_default(),
                    &r.kind,
                    &r.value,
                )
            });
            if let Some(record) = self.records.get(pane_id).filter(|r| {
                current_provider.is_some_and(|provider| provider.as_str() == r.provider)
                    && current_reference == r.reported_reference
                    && (r.metadata.native_title.is_some()
                        || r.metadata.manual_title.is_some()
                        || r.metadata.goal.is_some()
                        || !r.metadata.summary.is_empty()
                        || !r.metadata.created_prs.is_empty())
            }) {
                agent.record = Some(record.clone());
            }
            let Some(pane) = self.panes.get(pane_id) else {
                continue;
            };
            agent.changed_at_unix_ms = Some(pane.changed_unix_ms);
            let Some(label) = pane.label.as_ref() else {
                continue;
            };
            let reference = agent.agent_session.as_ref().and_then(|session| {
                label_reference_token(
                    agent.agent.as_deref().unwrap_or_default(),
                    &session.kind,
                    &session.value,
                )
            });
            if !label.proves(reference.as_deref()) {
                continue;
            }
            let mut facts = label.facts.clone();
            // Older stores/helpers classified Pi's source-less name as manual.
            // Its /name command and extensions write the same native record.
            if agent
                .agent
                .as_deref()
                .and_then(hide_session::Agent::from_kind)
                == Some(hide_session::Agent::Pi)
            {
                facts.native_title = facts.native_custom_title.take().or(facts.native_title);
            }
            (facts.native_session_id, facts.native_reference) = label.native(
                agent.agent.as_deref().unwrap_or_default(),
                agent.agent_session.as_ref(),
            );
            facts.awaiting_operator = awaiting_operator;
            facts.user_turn = label.user_turn.as_ref().and_then(|(seq, fact)| {
                (agent.state_change_seq == Some(*seq)).then(|| fact.clone())
            });
            if let Some((_, read)) = label
                .wake
                .as_ref()
                .filter(|(seq, _)| agent.state_change_seq == Some(*seq))
            {
                let live = read
                    .expiries
                    .iter()
                    .filter(|expiry| expiry.is_none_or(|at| at > now));
                facts.wake_devices = u32::try_from(live.count()).unwrap_or(u32::MAX);
                facts.wake_vanished = read.vanished;
            }
            facts.turn_end_unread =
                agent.state_change_seq.is_some() && label.turn_end_owed == agent.state_change_seq;
            if let Some(summary) = &label.summary {
                let working = agent.agent_status.as_deref() == Some("working");
                let asking = summary.end == Some(LabelEnd::Question);
                // A pane draws a provider's refusal as the block it is for
                // the operator now; only the session record tells the two
                // apart (PRD session-record-goal-flow B9).
                let as_pane = |end: LabelEnd| match end {
                    LabelEnd::Limited => LabelEnd::Blocked,
                    end => end,
                };
                let blocked = summary.end.map(as_pane) == Some(LabelEnd::Blocked);
                let line = (!summary.line.trim().is_empty()).then(|| summary.line.clone());
                // The sidebar's two lines keep their meaning: the line is the
                // reply asked for when the turn ended on a question, and the
                // progress otherwise, which for a block is its cause.
                agent.label = Some(AgentLabel {
                    task: summary.goal.clone(),
                    progress: line.clone().filter(|_| !asking),
                    expected_reply: line.clone().filter(|_| asking),
                    question: asking && !working,
                    blocked: blocked && !working,
                });
                facts.end = summary.end.map(as_pane);
                facts.line = line;
                facts.recent_turns.clone_from(&summary.recent_turns);
                for turn in &mut facts.recent_turns {
                    turn.end = turn.end.map(as_pane);
                }
            }
            agent.facts = Some(facts);
        }
    }
}

pub(super) fn unix_milliseconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The part of a session's facts a row is built from.
fn row_facts(facts: &SessionFacts) -> RowFacts {
    RowFacts {
        native_reference: None,
        native_session_id: None,
        native_title: facts.title.clone(),
        native_custom_title: facts.custom_title.clone(),
        operator_request: facts.operator_request.clone(),
        other_request: facts.other_request.clone(),
        last_reply_at_unix_ms: facts.last_reply_at_unix_ms,
        created_prs: facts
            .created_prs
            .iter()
            .map(|pr| (pr.repository.clone(), pr.number, pr.sighted_at_unix_ms))
            .collect(),
        sighted_prs: facts
            .sightings
            .iter()
            .map(|sighting| (sighting.repository.to_ascii_lowercase(), sighting.number))
            .collect(),
        end: None,
        line: None,
        recent_turns: Vec::new(),
        awaiting_operator: false,
        user_turn: None,
        wake_devices: 0,
        turn_end_unread: false,
        wake_vanished: false,
    }
}

impl ProvenLabel {
    /// The native session this label proves for `kind`, and the reference a
    /// lifecycle proof of it takes: the one its reader names, a file reader's
    /// proven path or the id Herdr reported (OpenCode's session, agy's
    /// conversation, which agy's reader finds its transcript by).
    fn native(
        &self,
        kind: &str,
        reported: Option<&SessionAgentSessionPayload>,
    ) -> (Option<String>, Option<SessionAgentSessionPayload>) {
        let session = self
            .native_session_id
            .as_ref()
            .filter(|id| label_reference_token(kind, "id", id).as_deref() == self.owner.as_deref())
            .cloned();
        let reference = session.as_ref().and_then(|_| {
            if hide_session::Agent::from_kind(kind).is_some_and(|native| {
                native.requires_native_file_proof() && native.proof_reference_kind() == "path"
            }) {
                self.native_source_path
                    .as_ref()
                    .map(|path| SessionAgentSessionPayload {
                        kind: "path".into(),
                        value: path.clone(),
                    })
            } else {
                reported.cloned()
            }
        });
        (session, reference)
    }

    fn proves(&self, reference: Option<&str>) -> bool {
        store::proves(
            self.owner.as_deref(),
            self.proven_reference.as_deref(),
            reference,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A proven label's row, for `kind` reported by Herdr as `reported`.
    fn proven_row(kind: &str, reported: (&str, &str)) -> SessionAgentPayload {
        let id = "13cffb10-48e7-4642-b333-c4f57952b637";
        let record = PaneRecord {
            owner: label_reference_token(kind, "id", id),
            native_session_id: Some(id.to_owned()),
            native_source_path: Some(format!("/home/me/.native/{id}.jsonl")),
            ..PaneRecord::default()
        };
        let overlay = LabelOverlay::of_records([(&"w1:p1".to_owned(), &record)], true, false);
        let mut payload: SessionSnapshotPayload =
            crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [{
                "id": "w1:p1", "pane_id": "w1:p1", "agent": kind, "agent_status": "idle",
                "state_change_seq": 1,
                "agent_session": {"kind": reported.0, "value": reported.1},
            }]}))
            .unwrap();
        overlay.apply(&mut payload);
        payload.agents.remove(0)
    }

    #[test]
    fn an_agy_row_carries_the_conversation_id_its_fork_and_sleep_prove() {
        let id = "13cffb10-48e7-4642-b333-c4f57952b637";
        let row = proven_row("antigravity", ("id", id));
        let reference = row.facts.unwrap().native_reference.unwrap();
        assert_eq!(
            (reference.kind.as_str(), reference.value.as_str()),
            ("id", id)
        );
        assert_eq!(
            reference.kind,
            hide_session::Agent::Antigravity.proof_reference_kind()
        );
        // A file reader's row still carries the proven path.
        let row = proven_row("pi", ("id", id));
        let reference = row.facts.unwrap().native_reference.unwrap();
        assert_eq!(reference.kind, "path");
        assert_eq!(
            reference.kind,
            hide_session::Agent::Pi.proof_reference_kind()
        );
    }
}
