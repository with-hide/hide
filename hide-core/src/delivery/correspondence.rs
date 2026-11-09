//! Each pane's letters and the agents it started, in time order, for the turn
//! card's letters (`docs/UI_BEHAVIOR.md`, the turn card): one line per agent
//! it worked with, and the whole list in time order when opened.
//!
//! The index is built from a whole ledger by the delivery worker, outside
//! `Mutex<Runtime>`, before it publishes that ledger; the runtime compares
//! only its digest under the lock and lays the rows again only when it moved
//! (`Runtime::publish_delivery`). A row then reads its own pane's slice from
//! the request's time on: two binary searches, one pass over that slice to
//! fold it into a line per agent, at most [`CARD_PEERS`] of those and
//! [`CARD_LETTERS`] lines in time order, whatever the ledger holds.

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};

use serde::Serialize;

use super::ledger::{Ledger, Letter, State};
use crate::request_view::RequestSender;

/// The newest letters a turn card lists; older ones are only counted.
pub(crate) const CARD_LETTERS: usize = 8;
/// The agents a turn card lists a line for, the latest first; the rest are
/// only counted.
pub(crate) const CARD_PEERS: usize = 6;
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
    /// An agent's line when all it wrote were interim reports.
    Progress,
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
    /// Which agent the other side is (its device, pane and session), so a
    /// line per agent can be folded from the lines.
    #[serde(skip)]
    pub(crate) peer_key: u64,
}

/// One agent the pane worked with since the request, in one line: what it
/// last said that matters, a block before a report and a report before an
/// interim one, else what the pane last sent it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PeerLineSnapshot {
    pub peer: RequestSender,
    #[serde(skip)]
    pub(crate) titled_by: Option<String>,
    /// The mark of the letter the line shows: a block, a question, a report
    /// or a reply it sent, `progress` when it sent only interim reports, and
    /// `waiting_answer` when the pane only wrote to it and waits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark: Option<LetterMark>,
    /// That letter's first line; absent for an agent the pane only started.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    pub at_unix_ms: u64,
    /// How many blocks it sent the pane since the request.
    #[serde(skip_serializing_if = "is_zero")]
    pub blocks: u32,
    /// The pane started it since the request.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub started: bool,
}

fn is_zero(count: &u32) -> bool {
    *count == 0
}

/// The pane's letters since the shown request: how many, a line per agent,
/// and the newest lines in time order for the whole list.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LettersSinceSnapshot {
    /// Letters and started agents, interim reports left out.
    pub total: u32,
    /// Interim reports (`progress`), counted and not listed.
    pub progress: u32,
    /// Letters, interim reports included and started agents left out.
    pub letters: u32,
    /// The newest [`CARD_LETTERS`] of `total`, oldest first.
    pub recent: Vec<LetterLineSnapshot>,
    /// A line for each of the [`CARD_PEERS`] agents the pane last worked
    /// with, the latest first.
    pub peers: Vec<PeerLineSnapshot>,
    /// The agents past [`CARD_PEERS`], counted and not listed.
    pub more_peers: u32,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct PaneLetters {
    /// Every letter and started agent but interim reports, by time.
    lines: Vec<LetterLineSnapshot>,
    /// Every interim report, by time; folded on the whole list, and an
    /// agent's line when it wrote nothing else.
    progress: Vec<LetterLineSnapshot>,
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
        // When each pane last wrote another a report or a reply: a letter
        // still waiting for its answer reads answered on the card once its
        // recipient wrote either back. The ledger's own wait is left as it is.
        let mut answered: HashMap<(Who<'_>, Who<'_>), u64> = HashMap::new();
        for letter in &ledger.letters {
            if letter.state != State::Cancelled
                && (letter.kind == "report" || letter.reply_to.is_some())
            {
                let at = answered
                    .entry((who(&letter.sender), who(&letter.recipient)))
                    .or_default();
                *at = (*at).max(letter.created_at_unix_ms);
            }
        }
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
                let peer_key = key_of(who(peer));
                let (peer, titled_by) = peer_of(ledger, peer);
                let written = LetterLineSnapshot {
                    direction,
                    peer,
                    titled_by,
                    mark: if progress {
                        Some(LetterMark::Progress)
                    } else {
                        mark(letter, direction, &answered)
                    },
                    line: line.clone(),
                    at_unix_ms: letter.created_at_unix_ms,
                    peer_key,
                };
                if progress {
                    entry.progress.push(written);
                } else {
                    entry.lines.push(written);
                }
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
            // The child it became is the agent its letters name; one not
            // registered yet is known by its spawn alone.
            let child = spawn
                .child
                .as_ref()
                .and_then(|child| ledger.agents.iter().find(|agent| &agent.id == child));
            let peer_key = child.map_or_else(
                || key_of(("spawn", &spawn.id, None)),
                |child| key_of(who(&child.actor)),
            );
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
                    peer_key,
                });
        }
        let mut hasher = std::hash::DefaultHasher::new();
        for (pane, letters) in &mut panes {
            letters.lines.sort_by_key(|line| line.at_unix_ms);
            letters.progress.sort_by_key(|line| line.at_unix_ms);
            pane.hash(&mut hasher);
            letters.lines.hash(&mut hasher);
            letters.progress.hash(&mut hasher);
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
        let lines = &letters.lines[letters.lines.partition_point(|line| line.at_unix_ms < from)..];
        let progress = &letters.progress[letters
            .progress
            .partition_point(|line| line.at_unix_ms < from)..];
        if lines.is_empty() && progress.is_empty() {
            return None;
        }
        let started = lines
            .iter()
            .filter(|line| line.direction == LetterDirection::Started)
            .count();
        let (peers, more_peers) = peer_lines(lines, progress);
        let count = |count: usize| u32::try_from(count).unwrap_or(u32::MAX);
        Some(LettersSinceSnapshot {
            total: count(lines.len()),
            progress: count(progress.len()),
            letters: count(lines.len() - started + progress.len()),
            recent: lines[lines.len() - lines.len().min(CARD_LETTERS)..].to_vec(),
            peers,
            more_peers: count(more_peers),
        })
    }
}

/// What one agent's lines fold into.
#[derive(Default)]
struct Peer<'a> {
    received: Option<&'a LetterLineSnapshot>,
    progress: Option<&'a LetterLineSnapshot>,
    sent: Option<&'a LetterLineSnapshot>,
    started: Option<&'a LetterLineSnapshot>,
    blocks: u32,
    latest: u64,
}

/// The pane's lines since the request folded into a line per agent, the
/// agent last heard from or written to first, cut at [`CARD_PEERS`]; and how
/// many agents were cut. `lines` and `progress` are each in time order.
fn peer_lines(
    lines: &[LetterLineSnapshot],
    progress: &[LetterLineSnapshot],
) -> (Vec<PeerLineSnapshot>, usize) {
    let mut peers: HashMap<u64, Peer<'_>> = HashMap::new();
    for line in lines.iter().chain(progress) {
        let peer = peers.entry(line.peer_key).or_default();
        peer.latest = peer.latest.max(line.at_unix_ms);
        let slot = match (line.direction, line.mark) {
            (LetterDirection::Started, _) => &mut peer.started,
            (_, Some(LetterMark::Progress)) => &mut peer.progress,
            (LetterDirection::Received, mark) => {
                peer.blocks += u32::from(mark == Some(LetterMark::Block));
                &mut peer.received
            }
            (LetterDirection::Sent, _) => &mut peer.sent,
        };
        if slot.is_none_or(|kept| kept.at_unix_ms <= line.at_unix_ms) {
            *slot = Some(line);
        }
    }
    let mut folded: Vec<(u64, u64, PeerLineSnapshot)> = peers
        .into_iter()
        .filter_map(|(key, peer)| {
            // What it said comes first, an interim report only when it said
            // nothing else, then what the pane last wrote it, then its start.
            let shown = peer.received.or(peer.progress).or(peer.sent);
            let named = shown.or(peer.started)?;
            let mark = shown.and_then(|line| match line.direction {
                LetterDirection::Sent => (line.mark == Some(LetterMark::WaitingAnswer))
                    .then_some(LetterMark::WaitingAnswer),
                _ => line.mark,
            });
            Some((
                peer.latest,
                key,
                PeerLineSnapshot {
                    peer: named.peer.clone(),
                    titled_by: named.titled_by.clone(),
                    mark,
                    line: shown.and_then(|line| line.line.clone()),
                    at_unix_ms: named.at_unix_ms,
                    blocks: peer.blocks,
                    started: peer.started.is_some(),
                },
            ))
        })
        .collect();
    // The latest first; two agents at the same moment keep one order.
    folded.sort_by(|(a, a_key, _), (b, b_key, _)| b.cmp(a).then(a_key.cmp(b_key)));
    let more = folded.len().saturating_sub(CARD_PEERS);
    folded.truncate(CARD_PEERS);
    (folded.into_iter().map(|(_, _, line)| line).collect(), more)
}

fn key_of(who: Who<'_>) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    who.hash(&mut hasher);
    hasher.finish()
}

/// `actor`'s screen name as the bell gives it (`doorbell::bell_sender`): the
/// name it registered under unless Hide made it up for its pane
/// (`AgentRecord::screen_name`), else its pane's title, which the row lays,
/// else its runtime's name. The wire name (`coordination::wire_name`) is never
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
        .and_then(crate::coordination::AgentRecord::screen_name);
    match registered {
        Some(name) => (crate::request_view::named(name), None),
        None => (
            crate::request_view::named(&crate::sidebar::provider_name(Some(&actor.kind))),
            Some(actor.pane_id.clone()),
        ),
    }
}

/// One agent as a letter names it: its device, its pane and its session, so
/// a new session in the same pane, or another device's pane of the same id,
/// is another agent.
type Who<'a> = (&'a str, &'a str, Option<&'a str>);

fn who(actor: &super::Actor) -> Who<'_> {
    (&actor.device_id, &actor.pane_id, actor.session.as_deref())
}

fn mark(
    letter: &Letter,
    direction: LetterDirection,
    answered: &HashMap<(Who<'_>, Who<'_>), u64>,
) -> Option<LetterMark> {
    let answered_since = answered
        .get(&(who(&letter.recipient), who(&letter.sender)))
        .is_some_and(|at| *at > letter.created_at_unix_ms);
    if direction == LetterDirection::Sent && letter.waiting_answer && !answered_since {
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
    fn a_sent_letter_stops_waiting_on_the_card_once_its_recipient_reports_or_replies() {
        let lead = actor("w1:p1", "lead");
        let child = actor("w1:p2", "child");
        let other = actor("w1:p3", "other");
        let mut reply = letter(6, &other, &lead, "request", "답장입니다", 600);
        reply.reply_to = Some("letter-2".into());
        let ledger = Ledger {
            letters: vec![
                letter(1, &lead, &child, "request", "고쳐 주세요", 100),
                letter(2, &lead, &other, "request", "봐 주세요", 200),
                letter(3, &lead, &other, "request", "하나 더", 700),
                letter(4, &child, &lead, "progress", "하는 중", 300),
                letter(5, &child, &lead, "report", "고쳤습니다", 400),
                reply,
            ],
            ..Ledger::default()
        };
        let since = Correspondence::of(&ledger).since("w1:p1", 0).unwrap();
        let sent: Vec<_> = since
            .recent
            .iter()
            .filter(|line| line.direction == LetterDirection::Sent)
            .map(|line| (line.line.as_deref(), line.mark))
            .collect();
        // A progress note is no answer; a report or a reply from the same
        // recipient is, and only for what was sent before it.
        assert_eq!(
            sent,
            vec![
                (Some("고쳐 주세요"), None),
                (Some("봐 주세요"), None),
                (Some("하나 더"), Some(LetterMark::WaitingAnswer)),
            ]
        );
        // The ledger still waits: only the card reads it answered.
        assert!(ledger.letters[0].waiting_answer);
    }

    #[test]
    fn an_unrelated_or_cancelled_answer_does_not_clear_the_card_wait() {
        let lead = actor("w1:p1", "lead");
        let child = actor("w1:p2", "child");
        let mut another_session = child.clone();
        another_session.session = Some("another-session".into());
        let mut another_device = child.clone();
        another_device.device_id = "another-device".into();
        let mut another_lead = lead.clone();
        another_lead.session = Some("another-lead-session".into());
        let mut cancelled = letter(2, &child, &lead, "report", "취소", 200);
        cancelled.state = State::Cancelled;
        for answer in [
            letter(2, &child, &lead, "progress", "중간 보고", 200),
            letter(2, &child, &lead, "request", "새 질문", 200),
            letter(2, &child, &lead, "report", "이전 보고", 99),
            letter(2, &another_session, &lead, "report", "다른 세션", 200),
            letter(2, &another_device, &lead, "report", "다른 기기", 200),
            letter(2, &child, &another_lead, "report", "다른 수신 세션", 200),
            cancelled,
        ] {
            let ledger = Ledger {
                letters: vec![
                    letter(1, &lead, &child, "request", "봐 주세요", 100),
                    answer,
                ],
                ..Ledger::default()
            };
            let since = Correspondence::of(&ledger).since("w1:p1", 100).unwrap();
            assert_eq!(
                since.recent[0].mark,
                Some(LetterMark::WaitingAnswer),
                "{:?}",
                ledger.letters[1]
            );
            assert!(ledger.letters[0].waiting_answer);
        }
    }

    #[test]
    fn each_agent_folds_into_one_line_of_what_it_last_said_that_matters() {
        let lead = actor("w1:p1", "lead");
        let builder = actor("w1:p2", "builder");
        let reviewer = actor("w1:p3", "reviewer");
        let writer = actor("w1:p4", "writer");
        let quiet = actor("w1:p5", "quiet");
        let busy = actor("w1:p6", "busy");
        let mut ledger = Ledger {
            letters: vec![
                letter(1, &lead, &builder, "request", "고쳐 주세요", 100),
                letter(2, &builder, &lead, "block", "경로가 없어요", 200),
                letter(3, &lead, &builder, "report", "fixtures 아래에", 300),
                letter(4, &builder, &lead, "progress", "하는 중", 400),
                letter(5, &builder, &lead, "report", "고쳤습니다", 500),
                letter(6, &builder, &lead, "progress", "정리 중", 600),
                letter(7, &lead, &reviewer, "request", "봐 주세요", 150),
                letter(8, &writer, &lead, "request", "어디에 둘까요?", 700),
                letter(9, &lead, &quiet, "request", "답 주세요", 350),
                letter(10, &busy, &lead, "progress", "절반", 450),
            ],
            ..Ledger::default()
        };
        ledger.letters[2].reply_to = Some("letter-2".into());
        for (n, agent) in [&lead, &builder, &reviewer, &writer, &quiet, &busy]
            .into_iter()
            .enumerate()
        {
            ledger.agents.push(registered(&format!("agent-{n}"), agent));
        }
        // The lead started the writer; its start and its letters are one line.
        ledger.spawns.push(serde_json::from_value(serde_json::json!({
            "id": "spawn-1", "parent": "agent-0", "intent": "w", "name": "writer",
            "kind": "claude", "repo": "/repo", "branch": "b", "requested_path": null, "args": [],
            "path": null, "pane": "w1:p4", "child": "agent-3", "reserved_at_unix_ms": 120
        })).unwrap());
        let since = Correspondence::of(&ledger).since("w1:p1", 0).unwrap();
        let lines: Vec<_> = since
            .peers
            .iter()
            .map(|line| {
                (
                    line.peer.clone(),
                    line.mark,
                    line.line.as_deref(),
                    line.blocks,
                    line.started,
                )
            })
            .collect();
        assert_eq!(
            lines,
            vec![
                (
                    named("writer"),
                    Some(LetterMark::Question),
                    Some("어디에 둘까요?"),
                    0,
                    true
                ),
                // A later interim report does not hide the report before it.
                (
                    named("builder"),
                    Some(LetterMark::Report),
                    Some("고쳤습니다"),
                    1,
                    false
                ),
                (
                    named("busy"),
                    Some(LetterMark::Progress),
                    Some("절반"),
                    0,
                    false
                ),
                (
                    named("quiet"),
                    Some(LetterMark::WaitingAnswer),
                    Some("답 주세요"),
                    0,
                    false
                ),
                (
                    named("reviewer"),
                    Some(LetterMark::WaitingAnswer),
                    Some("봐 주세요"),
                    0,
                    false
                ),
            ]
        );
        assert_eq!((since.letters, since.more_peers), (10, 0));
    }

    #[test]
    fn agents_past_the_cap_are_counted() {
        let lead = actor("w1:p1", "lead");
        let others: Vec<_> = (0..9)
            .map(|n| actor(&format!("w2:p{n}"), &format!("a{n}")))
            .collect();
        let ledger = Ledger {
            letters: others
                .iter()
                .enumerate()
                .map(|(n, other)| {
                    letter(n as u64 + 1, other, &lead, "report", "끝", 100 + n as u64)
                })
                .collect(),
            ..Ledger::default()
        };
        let since = Correspondence::of(&ledger).since("w1:p1", 0).unwrap();
        assert_eq!(since.peers.len(), CARD_PEERS);
        assert_eq!(since.more_peers, 3);
        // The agent last heard from comes first.
        assert_eq!(since.peers[0].at_unix_ms, 108);
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
