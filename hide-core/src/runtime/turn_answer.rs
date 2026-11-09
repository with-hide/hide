//! The whole answer the turn card's `전체 보기` opens: the agent's last
//! message in the turn the shown request began, read where the session runs.
//!
//! The row carries only the message's first lines
//! (`request_view::TurnAnswerSnapshot`), so a card the operator never opens
//! sends no long answer with every change; the text rides the snapshot for
//! the one answer the operator opened, as a mission node's brief does
//! (`mission_brief.rs`). The read is the backlog origin's: the link record
//! names the session's file and folder, and the session's own node answers
//! that one turn (`Call::SessionWorkTurn`), checked against the witness the
//! turn's judged end kept, on a worker thread off the lock. One read runs at a
//! time and only the newest asked waits behind it.

use std::sync::Arc;

use hide_node_link::protocol::Call;
use serde::{Deserialize, Serialize};

use super::Runtime;
use crate::links::ledger::{LedgerAnswer, LedgerCall, SessionIdentity};
use crate::links::worker::LinkClient;
use crate::node_access::NodeLink;

/// How long the session's node is given to answer.
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The answer the operator opened, for one card at a time.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct TurnAnswerReadSnapshot {
    pub pane_id: Option<String>,
    /// The offset of the request whose turn it answers.
    pub offset: Option<u64>,
    /// `reading`, `ready` or `failed`; absent while nothing was asked.
    pub phase: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TurnAnswerReadPayload {
    pub pane_id: String,
    pub offset: u64,
}

/// What one read needs, taken under the lock.
struct Asked {
    pane_id: String,
    offset: u64,
    session: SessionIdentity,
    witness: String,
    reply_bytes: u64,
}

#[derive(Default)]
pub(super) struct TurnAnswerWork {
    generation: u64,
    running: bool,
    pending: Option<(u64, Asked)>,
}

impl Runtime {
    /// `turn_answer_read`: the card's `전체 보기` was pressed.
    pub(super) fn turn_answer_read(&mut self, payload: TurnAnswerReadPayload) -> bool {
        let shown = &self.snapshot.turn_answer;
        if shown.pane_id.as_deref() == Some(payload.pane_id.as_str())
            && shown.offset == Some(payload.offset)
            && shown.phase.as_deref() != Some("failed")
        {
            return false;
        }
        let asked = self.asked_answer(&payload);
        self.turn_answer_work.generation = self.turn_answer_work.generation.wrapping_add(1);
        let generation = self.turn_answer_work.generation;
        self.snapshot.turn_answer = TurnAnswerReadSnapshot {
            pane_id: Some(payload.pane_id.clone()),
            offset: Some(payload.offset),
            phase: Some("reading".into()),
            text: None,
        };
        let asked = match asked {
            Ok(asked) => asked,
            Err(code) => {
                self.fail_turn_answer(&payload.pane_id, code);
                return true;
            }
        };
        if self.turn_answer_work.running {
            self.turn_answer_work.pending = Some((generation, asked));
        } else {
            self.start_turn_answer_read(generation, asked);
        }
        true
    }

    /// The row's session, the turn's witness and the message's length: an
    /// answer the card offered whole has all three.
    fn asked_answer(&self, payload: &TurnAnswerReadPayload) -> Result<Asked, &'static str> {
        let row = self
            .snapshot
            .navigator
            .agents
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|remote| remote.session.as_ref())
                    .flat_map(|session| session.agents.iter()),
            )
            .find(|row| row.pane_id == payload.pane_id)
            .ok_or("turn_answer_row_gone")?;
        let facts = row.row_facts.as_ref().ok_or("turn_answer_unread")?;
        let witness = facts
            .recent_turns
            .iter()
            .find(|turn| turn.offset == payload.offset)
            .and_then(|turn| turn.witness.clone())
            .ok_or("turn_answer_unjudged")?;
        let reply = facts
            .replies
            .iter()
            .find(|reply| reply.request_offset == payload.offset)
            .ok_or("turn_answer_unread")?;
        let session = SessionIdentity {
            device_id: row.device_id.0.clone(),
            agent: hide_session::Agent::from_kind(&row.agent_kind)
                .ok_or("turn_answer_unreadable")?
                .as_str()
                .to_owned(),
            session_id: row.session_id.clone().ok_or("turn_answer_unread")?,
        };
        Ok(Asked {
            pane_id: payload.pane_id.clone(),
            offset: payload.offset,
            session,
            witness,
            reply_bytes: reply.bytes,
        })
    }

    fn start_turn_answer_read(&mut self, generation: u64, asked: Asked) {
        let pane_id = asked.pane_id.clone();
        let Some(client) = self.links_work.client.clone() else {
            self.fail_turn_answer(&pane_id, "turn_answer_record_unavailable");
            return;
        };
        let node = match self.linked_node(&asked.session.device_id) {
            Ok((_, node)) => node,
            Err(_) => {
                self.fail_turn_answer(&pane_id, "turn_answer_device_unavailable");
                return;
            }
        };
        self.turn_answer_work.running = true;
        let spawned = self.spawn_issue_worker(
            "turn-answer",
            move || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    read_answer(&client, node, &asked)
                }))
                .unwrap_or_else(|_| Err("turn_answer_reader_stopped".into()))
            },
            move |runtime, result| runtime.finish_turn_answer(generation, result),
        );
        if let Err(error) = spawned {
            self.turn_answer_work.running = false;
            crate::diagnostic!(serde_json::json!({
                "component": "turn_card", "kind": "turn_answer.worker_unavailable", "error": error,
            }));
            self.fail_turn_answer(&pane_id, "turn_answer_worker_unavailable");
        }
    }

    fn finish_turn_answer(&mut self, generation: u64, result: Result<String, String>) -> bool {
        self.turn_answer_work.running = false;
        let mut changed = false;
        if generation == self.turn_answer_work.generation {
            let pane_id = self
                .snapshot
                .turn_answer
                .pane_id
                .clone()
                .unwrap_or_default();
            match result {
                Ok(text) => {
                    self.snapshot.turn_answer.phase = Some("ready".into());
                    self.snapshot.turn_answer.text = Some(text);
                }
                Err(code) => self.fail_turn_answer(&pane_id, &code),
            }
            changed = true;
        }
        if let Some((next, asked)) = self.turn_answer_work.pending.take() {
            self.start_turn_answer_read(next, asked);
            changed = true;
        }
        changed
    }

    /// The card says the whole answer could not be read; why goes to the
    /// log, since the operator can act on none of it (design principle 13).
    fn fail_turn_answer(&mut self, pane_id: &str, code: &str) {
        crate::diagnostic!(serde_json::json!({
            "component": "turn_card", "kind": "turn_answer.read_failed",
            "pane_id": pane_id, "code": code,
        }));
        self.snapshot.turn_answer.phase = Some("failed".into());
        self.snapshot.turn_answer.text = None;
    }
}

/// Reads the turn on the session's node and keeps its last message: the
/// turn's text is its messages joined by line breaks, so the last
/// `reply_bytes` of it are the message the card's head was taken from.
fn read_answer(
    client: &LinkClient,
    node: Arc<dyn NodeLink>,
    asked: &Asked,
) -> Result<String, String> {
    let LedgerAnswer::Record(record) = client.call(LedgerCall::Record(asked.session.clone()))?
    else {
        return Err("turn_answer_record_invalid".into());
    };
    let cwd = record.cwd.clone().ok_or("turn_answer_record_unplaced")?;
    let agent =
        hide_session::Agent::from_kind(&asked.session.agent).ok_or("turn_answer_unreadable")?;
    let turn = crate::node_access::call_as::<hide_session::label_transcript::WorkTurn>(
        node.as_ref(),
        Call::SessionWorkTurn {
            request: hide_session::label_transcript::WorkTurnRequest {
                agent,
                session_id: asked.session.session_id.clone(),
                path: record.path.clone(),
                cwd,
                offset: asked.offset,
                witness: asked.witness.clone(),
            },
        },
        READ_TIMEOUT,
    )
    .map_err(|error| error.to_string())?;
    Ok(last_message(&turn.assistant, asked.reply_bytes).to_owned())
}

/// The last `bytes` of `text` with its trailing line breaks left out, or the
/// whole text when it is shorter or the cut would split a character.
fn last_message(text: &str, bytes: u64) -> &str {
    let text = text.trim_end_matches('\n');
    usize::try_from(bytes)
        .ok()
        .and_then(|bytes| text.len().checked_sub(bytes))
        .filter(|start| text.is_char_boundary(*start))
        .map_or(text, |start| &text[start..])
}

#[cfg(test)]
mod tests {
    use super::last_message;

    #[test]
    fn the_last_message_is_cut_from_the_turns_text_by_its_length() {
        let last = "네, 실제로 생길 수 있습니다.\n| 상황 | 예 |";
        let turn = format!("먼저 확인해 볼게요\n{last}\n");
        assert_eq!(last_message(&turn, last.len() as u64), last);
        assert_eq!(last_message("짧음", 100), "짧음");
        // A length that falls inside a character keeps the whole text.
        assert_eq!(last_message("가나", 2), "가나");
    }
}
