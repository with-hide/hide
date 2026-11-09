//! Each pane's letters and the agents it started, in time order, for the turn
//! card's "그 사이 오간 편지" (`docs/UI_BEHAVIOR.md`, the turn card).
//!
//! The index is built from a whole ledger by the delivery worker, outside
//! `Mutex<Runtime>`, before it publishes that ledger; the runtime compares
//! only its digest under the lock and lays the rows again only when it moved
//! (`Runtime::publish_delivery`). A row then reads its own pane's slice from
//! the request's time on: two binary searches and at most
//! [`CARD_LETTERS`] lines, whatever the ledger holds.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use serde::Serialize;

use super::ledger::{Ledger, Letter, State};
use crate::request_view::RequestSender;

/// The newest letters a turn card lists; older ones are only counted.
pub(crate) const CARD_LETTERS: usize = 8;
/// The longest first line of a letter a card line carries; the card cuts it
/// to its width.
const LINE_CHARS: usize = 80;

/// Which way a line on the card went, seen from the row's pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LetterDirection {
    Sent,
    Received,
    /// The pane started an agent (`hide agent spawn`).
    Started,
}

/// What a letter's line is marked with on the card, by the first that holds:
/// a sent letter still waiting for its answer, a reply, a block, a report,
/// and a request the pane received, which asks it something. A request the
/// pane sent that was answered, and a started agent, carry none.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LetterMark {
    WaitingAnswer,
    Reply,
    Block,
    Report,
    Question,
}

/// One line of the card's letter list.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct LetterLineSnapshot {
    pub direction: LetterDirection,
    /// The other side: the agent the letter went to or came from, the
    /// operator, or the agent started, by its screen name (`docs/delivery.md`,
    /// Names on the wire and on screen).
    pub peer: RequestSender,
    /// An unnamed agent's pane, whose title the row lays over `peer` (its
    /// runtime's name) when the pane is on the rows: a title changes every
    /// turn, so the index never holds one.
    #[serde(skip)]
    pub(crate) titled_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark: Option<LetterMark>,
    /// The letter's first line, its one-line summary by the letters' rule;
    /// absent for a started agent and a letter with no words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    pub at_unix_ms: u64,
}

/// The pane's letters since the shown request: how many, how many interim
/// reports were folded, and the newest lines, oldest first.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LettersSinceSnapshot {
    /// Letters and started agents, interim reports left out.
    pub total: u32,
    /// Interim reports (`progress`), counted and not listed.
    pub progress: u32,
    /// The newest [`CARD_LETTERS`] of `total`, oldest first.
    pub recent: Vec<LetterLineSnapshot>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct PaneLetters {
    /// Every letter and started agent but interim reports, by time.
    lines: Vec<LetterLineSnapshot>,
    /// When each interim report was written, in order.
    progress_at: Vec<u64>,
}

/// The index: each pane's lines, and a digest of all of them that tells two
/// indexes apart without comparing them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Correspondence {
    panes: BTreeMap<String, PaneLetters>,
    digest: u64,
}

impl Correspondence {
    /// Builds the index of `ledger`. Runs off the runtime lock: the ledger
    /// holds at most `LETTER_LIMIT` letters, and every one is visited once.
    pub fn of(ledger: &Ledger) -> Self {
        let mut panes: BTreeMap<String, PaneLetters> = BTreeMap::new();
        for letter in &ledger.letters {
            if letter.state == State::Cancelled {
                continue;
            }
            let line = letter
                .body
                .lines()
                .find_map(|line| crate::display_text::one_line(line, LINE_CHARS));
            let progress = letter.kind == "progress";
            for (pane, direction, peer) in [
                (&letter.sender, LetterDirection::Sent, &letter.recipient),
                (&letter.recipient, LetterDirection::Received, &letter.sender),
            ] {
                if pane.code_owned() {
                    continue;
                }
                let entry = panes.entry(pane.pane_id.clone()).or_default();
                if progress {
                    entry.progress_at.push(letter.created_at_unix_ms);
                    continue;
                }
                let (peer, titled_by) = peer_of(ledger, peer);
                entry.lines.push(LetterLineSnapshot {
                    direction,
                    peer,
                    titled_by,
                    mark: mark(letter, direction),
                    line: line.clone(),
                    at_unix_ms: letter.created_at_unix_ms,
                });
            }
        }
        for spawn in &ledger.spawns {
            // The caller's registration names its pane; a spawn whose caller
            // is not registered any more has no pane to list it on.
            let Some(parent) = ledger.agents.iter().find(|agent| agent.id == spawn.parent) else {
                continue;
            };
            if parent.actor.code_owned() {
                continue;
            }
            panes
                .entry(parent.actor.pane_id.clone())
                .or_default()
                .lines
                .push(LetterLineSnapshot {
                    direction: LetterDirection::Started,
                    peer: crate::request_view::named(&spawn.name),
                    titled_by: None,
                    mark: None,
                    line: None,
                    at_unix_ms: spawn.reserved_at_unix_ms,
                });
        }
        let mut hasher = std::hash::DefaultHasher::new();
        for (pane, letters) in &mut panes {
            letters.lines.sort_by_key(|line| line.at_unix_ms);
            letters.progress_at.sort_unstable();
            pane.hash(&mut hasher);
            letters.lines.hash(&mut hasher);
            letters.progress_at.hash(&mut hasher);
        }
        Self {
            panes,
            digest: hasher.finish(),
        }
    }

    /// Tells this index from another one built from a different ledger.
    pub(crate) fn digest(&self) -> u64 {
        self.digest
    }

    /// `pane`'s letters written at or after `from`, or `None` when there are
    /// none.
    pub(crate) fn since(&self, pane: &str, from: u64) -> Option<LettersSinceSnapshot> {
        let letters = self.panes.get(pane)?;
        let start = letters.lines.partition_point(|line| line.at_unix_ms < from);
        let progress =
            letters.progress_at.len() - letters.progress_at.partition_point(|at| *at < from);
        let total = letters.lines.len() - start;
        if total == 0 && progress == 0 {
            return None;
        }
        let shown = total.min(CARD_LETTERS);
        Some(LettersSinceSnapshot {
            total: u32::try_from(total).unwrap_or(u32::MAX),
            progress: u32::try_from(progress).unwrap_or(u32::MAX),
            recent: letters.lines[letters.lines.len() - shown..].to_vec(),
        })
    }
}

/// `actor`'s screen name as the bell gives it (`doorbell::bell_sender`): the
/// name it registered under, else its pane's title, which the row lays, else
/// its runtime's name. The wire name (`coordination::wire_name`) is never
/// shown, since an unnamed agent's is its registration id.
fn peer_of(ledger: &Ledger, actor: &super::Actor) -> (RequestSender, Option<String>) {
    if actor.operator_owned() {
        return (RequestSender::Operator, None);
    }
    let registered = ledger
        .agents
        .iter()
        .rev()
        .find(|record| record.actor.same_identity(actor))
        .and_then(|record| record.name.as_deref());
    match registered {
        Some(name) => (crate::request_view::named(name), None),
        None => (
            crate::request_view::named(&crate::sidebar::provider_name(Some(&actor.kind))),
            Some(actor.pane_id.clone()),
        ),
    }
}

fn mark(letter: &Letter, direction: LetterDirection) -> Option<LetterMark> {
    if direction == LetterDirection::Sent && letter.waiting_answer {
        return Some(LetterMark::WaitingAnswer);
    }
    if letter.reply_to.is_some() {
        return Some(LetterMark::Reply);
    }
    match letter.kind.as_str() {
        "block" => Some(LetterMark::Block),
        "report" => Some(LetterMark::Report),
        "request" if direction == LetterDirection::Received => Some(LetterMark::Question),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::Actor;

    fn actor(pane: &str, name: &str) -> Actor {
        Actor {
            pane_id: pane.into(),
            name: name.into(),
            kind: "claude".into(),
            device_id: "device".into(),
            session: Some(format!("session-{pane}")),
        }
    }

    fn letter(id: u64, from: &Actor, to: &Actor, kind: &str, body: &str, at: u64) -> Letter {
        let mut letter: Letter = serde_json::from_value(serde_json::json!({
            "id": format!("letter-{id}"), "intent": format!("intent-{id}"),
            "sender": from, "recipient": to, "kind": kind, "body": body,
            "state": "delivered", "waiting_answer": false, "reply_to": null,
            "created_at_unix_ms": at, "finished_at_unix_ms": null,
            "bell_errors": 0, "bell_sent": false
        }))
        .unwrap();
        letter.waiting_answer = kind == "request" || kind == "block";
        letter
    }

    /// `actor`'s registration under its own name.
    fn registered(id: &str, actor: &Actor) -> crate::coordination::AgentRecord {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": actor.name, "machine": "device", "host_scope": "scope",
            "native_machine": "device", "session": actor.session, "instance": "i",
            "terminal_id": null, "pane": actor.pane_id, "parent": null, "project": null,
            "actor": actor, "ended": false
        }))
        .unwrap()
    }

    fn named(name: &str) -> RequestSender {
        RequestSender::Named(name.into())
    }

    #[test]
    fn a_pane_lists_what_it_sent_and_received_since_the_request_and_folds_interim_reports() {
        let lead = actor("w1:p1", "lead");
        let child = actor("w1:p2", "child");
        let device = actor("remote:mini:pane:w2:p1", "mini-worker");
        let mut answered = letter(3, &lead, &child, "request", "계획 승인\n자세한 내용", 300);
        answered.waiting_answer = false;
        let mut reply = letter(4, &child, &lead, "report", "확인했습니다", 400);
        reply.reply_to = Some("letter-3".into());
        let mut cancelled = letter(9, &lead, &child, "report", "취소된 편지", 450);
        cancelled.state = State::Cancelled;
        let mut ledger = Ledger {
            letters: vec![
                letter(1, &lead, &child, "request", "요청 전에 보낸 편지", 100),
                letter(2, &child, &lead, "progress", "요청 전 중간 보고", 150),
                answered,
                reply,
                cancelled,
                letter(5, &child, &lead, "progress", "절반 했습니다", 500),
                letter(
                    6,
                    &device,
                    &lead,
                    "block",
                    "  디스크가 찼습니다  \n로그",
                    600,
                ),
                letter(7, &lead, &device, "request", "어떤 폴더를 지울까요?", 700),
            ],
            ..Ledger::default()
        };
        for (n, agent) in [&lead, &child, &device].into_iter().enumerate() {
            ledger.agents.push(registered(&format!("agent-{n}"), agent));
        }
        let index = Correspondence::of(&ledger);

        let since = index.since("w1:p1", 200).unwrap();
        assert_eq!((since.total, since.progress), (4, 1));
        let lines: Vec<_> = since
            .recent
            .iter()
            .map(|line| {
                (
                    line.direction,
                    line.peer.clone(),
                    line.mark,
                    line.line.as_deref(),
                    line.at_unix_ms,
                )
            })
            .collect();
        assert_eq!(
            lines,
            vec![
                (
                    LetterDirection::Sent,
                    named("child"),
                    None,
                    Some("계획 승인"),
                    300
                ),
                (
                    LetterDirection::Received,
                    named("child"),
                    Some(LetterMark::Reply),
                    Some("확인했습니다"),
                    400
                ),
                (
                    LetterDirection::Received,
                    named("mini-worker"),
                    Some(LetterMark::Block),
                    Some("디스크가 찼습니다"),
                    600
                ),
                (
                    LetterDirection::Sent,
                    named("mini-worker"),
                    Some(LetterMark::WaitingAnswer),
                    Some("어떤 폴더를 지울까요?"),
                    700
                ),
            ]
        );
        // A device's agent reads its own letters by the address the core
        // names its pane by, as its row does.
        let device_lines = index.since("remote:mini:pane:w2:p1", 0).unwrap();
        assert_eq!(device_lines.recent[1].mark, Some(LetterMark::Question));
        assert_eq!(index.since("w1:p1", 701), None);
    }

    #[test]
    fn only_the_newest_lines_are_listed_and_the_rest_counted() {
        let lead = actor("w1:p1", "lead");
        let child = actor("w1:p2", "child");
        let ledger = Ledger {
            letters: (0..20)
                .map(|n| {
                    letter(
                        n + 1,
                        &child,
                        &lead,
                        "report",
                        &format!("보고 {n}"),
                        1_000 + n,
                    )
                })
                .collect(),
            ..Ledger::default()
        };
        let since = Correspondence::of(&ledger).since("w1:p1", 0).unwrap();
        assert_eq!(since.total, 20);
        assert_eq!(since.recent.len(), CARD_LETTERS);
        assert_eq!(since.recent[0].line.as_deref(), Some("보고 12"));
        assert_eq!(
            since.recent[CARD_LETTERS - 1].line.as_deref(),
            Some("보고 19")
        );
    }

    #[test]
    fn a_started_agent_is_listed_on_its_callers_pane_and_moves_the_digest() {
        let lead = actor("w1:p1", "lead");
        let mut ledger = Ledger::default();
        let before = Correspondence::of(&ledger);
        ledger.agents.push(registered("agent-1", &lead));
        ledger.spawns.push(serde_json::from_value(serde_json::json!({
            "id": "spawn-2", "parent": "agent-1", "intent": "impl", "name": "auto-mission-impl",
            "kind": "claude", "repo": "/repo", "branch": "b", "requested_path": null, "args": [],
            "path": null, "pane": null, "child": null, "reserved_at_unix_ms": 1_300
        })).unwrap());
        let after = Correspondence::of(&ledger);
        let since = after.since("w1:p1", 1_000).unwrap();
        assert_eq!(since.recent[0].direction, LetterDirection::Started);
        assert_eq!(since.recent[0].peer, named("auto-mission-impl"));
        assert_ne!(before.digest(), after.digest());
        assert_eq!(after.digest(), Correspondence::of(&ledger).digest());
    }
}
