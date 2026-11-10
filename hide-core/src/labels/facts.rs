//! What a pane's session says without any AI (PRD overview-request-view
//! D-12, D-14, D-19, D-31): its own title, the last request and who sent it,
//! and the pull request addresses its tools printed.
//!
//! The worker folds each read into these facts and keeps them in the pane's
//! record, so a restarted daemon shows a pane with no new turn from what it
//! stored rather than reading its conversation again (B31). The request is
//! the conversation's own words, cut to [`MAX_TEXT_BYTES`]; local records live in the private `labels.json`.
//! Device records stay in memory, bar their marks: the session proof, the analyzed turns and the label (`store.rs`).
//!
//! Who sent a request is decided once per message and kept with its offset
//! (D-19): the Hide letter envelope's sender; else the operator when Hide saw them
//! submit to that pane just before the message was written; else, for a
//! message older than Hide's view of the pane, the operator unless lineage
//! says otherwise; else another agent. A delegated child's first request
//! belongs to its parent, which only the runtime knows, so the first request
//! is marked and the name is laid on later.

use std::collections::{BTreeMap, VecDeque};

use hide_session::label_transcript::{LabelEvent, LabelEventKind};
use hide_session::{ConversationCheckpoint, PrSighting};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::input::{Bell, Submit};

/// The most of a request's text kept.
pub(crate) const MAX_TEXT_BYTES: usize = 8 * 1024;
/// Pull request addresses kept per session, newest (by first sighting) last.
pub(crate) const MAX_SIGHTINGS: usize = 20;
/// Pull requests one session is recorded as having made.
const MAX_CREATED: usize = 20;

/// When GitHub made each pull request this daemon has read, by lowercase
/// `owner/name` and number.
pub(crate) type PullRequestTimes = BTreeMap<(String, u64), u64>;

/// A pull request this session made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CreatedPr {
    pub(crate) repository: String,
    pub(crate) number: u64,
    pub(crate) sighted_at_unix_ms: u64,
}

/// Requesters kept, by message offset.
const MAX_VERDICTS: usize = 32;
/// How long after a submit its message may be written by an agent that was
/// waiting for it, and by one that was running and queued it.
const SUBMIT_WINDOW_MS: u64 = 120_000;
const QUEUED_SUBMIT_WINDOW_MS: u64 = 30 * 60_000;
/// How much earlier than its submit a message may be stamped (the two clocks
/// are the conversation's and this daemon's).
pub(crate) const CLOCK_SLACK_MS: u64 = 2_000;

/// Who sent a person's message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub(crate) enum Requester {
    /// Hide saw the operator submit it.
    Operator,
    /// Written before Hide saw the pane's input and carrying no envelope:
    /// the operator's, unless it is a delegated child's first request.
    Unobserved,
    /// The sender a Hide letter envelope names.
    Named(String),
    /// Something other than Hide's input wrote it.
    Agent,
}

impl Requester {
    /// Counts as the operator's for the shown request (`나 ›`).
    pub(crate) fn is_operator(&self) -> bool {
        matches!(self, Self::Operator | Self::Unobserved)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Request {
    pub(crate) text: String,
    /// Where the message starts in the conversation, which keys its label
    /// summary (`PaneRecord::recent_turns`); absent on a record read before
    /// it was kept, which then shows its own first line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) offset: Option<u64>,
    /// The text was longer than [`MAX_TEXT_BYTES`].
    #[serde(default)]
    pub(crate) cut: bool,
    #[serde(default)]
    pub(crate) images: u32,
    pub(crate) at_unix_ms: u64,
    pub(crate) requester: Requester,
    /// The session's first request, which a delegated child's parent sent.
    #[serde(default)]
    pub(crate) first: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Verdict {
    offset: u64,
    requester: Requester,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct SessionFacts {
    /// The agent's own name for the session, and the operator's rename,
    /// which wins (`custom-title`).
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) custom_title: Option<String>,
    /// The last request that counts as the operator's.
    #[serde(default)]
    pub(crate) operator_request: Option<Request>,
    /// The last request someone else sent.
    #[serde(default)]
    pub(crate) other_request: Option<Request>,
    /// When the agent last said something, without its words: idle-agent
    /// sleep counts from the conversation's last line, and a row keeps no
    /// reply text, because nothing clears it when a new request starts.
    #[serde(default)]
    pub(crate) last_reply_at_unix_ms: Option<u64>,
    /// The canonical digests (`hide_agent_hooks::delivery::prompt_digest`)
    /// of the newest user messages, each once, newest last: the agent's own record of
    /// the input it took, which confirms a brief Hide typed into an agent
    /// whose prompt hook reports no digest (`delivery::typed`).
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub(crate) prompt_digests: VecDeque<String>,
    #[serde(default)]
    pub(crate) sightings: Vec<PrSighting>,
    /// The pull requests the session made, judged once against GitHub's
    /// creation time and kept (D-31), oldest first.
    #[serde(default)]
    pub(crate) created_prs: Vec<CreatedPr>,
    #[serde(default)]
    first_request_offset: Option<u64>,
    #[serde(default)]
    verdicts: VecDeque<Verdict>,
    /// Where each Claude Code subagent file was read up to.
    #[serde(default)]
    pub(crate) subagents: BTreeMap<String, ConversationCheckpoint>,
    /// The request a read saw last, which the agent's next words answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reply_to: Option<u64>,
    /// The head of the agent's last message after the operator's last
    /// request and after the newest request, at most those two, oldest
    /// first: the turn card shows the answer to the request it shows
    /// (`docs/UI_BEHAVIOR.md`, the turn card). Each is keyed by the offset
    /// of the request it answers, so a new request never shows an old
    /// answer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) replies: Vec<Reply>,
}

/// The agent's last message after one request: its first lines, its whole
/// length and when it was written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Reply {
    /// The offset of the request it answers.
    pub(crate) request_offset: u64,
    /// Its first [`REPLY_HEAD_LINES`] lines with words, each cut at
    /// [`REPLY_LINE_CHARS`] characters.
    pub(crate) head: String,
    /// The whole message's length in bytes, which picks it out of the
    /// turn's text when it is read in full (`label_transcript::WorkTurn`).
    pub(crate) bytes: u64,
    /// The message says more than its head.
    pub(crate) more: bool,
    pub(crate) at_unix_ms: u64,
}

/// The lines of a reply the turn card shows under its summary.
pub(crate) const REPLY_HEAD_LINES: usize = 2;
/// The longest line of a reply's head.
const REPLY_LINE_CHARS: usize = 160;
/// A head longer than this wraps past the card's two lines, so the card
/// offers the whole message.
const REPLY_SHOWN_CHARS: usize = 80;

impl Reply {
    fn of(request_offset: u64, text: &str, at_unix_ms: u64) -> Self {
        // The head is read as words: a table row, a code block and a rule
        // say nothing in two lines, and Markdown's marks are left out; the
        // whole message is still read as Markdown.
        let mut fence = false;
        let mut skipped = false;
        let mut lines = text.lines().filter_map(|line| {
            let plain = prose_line(line, &mut fence);
            skipped |= plain.is_none() && !line.trim().is_empty();
            plain.and_then(|plain| crate::display_text::one_line(&plain, usize::MAX))
        });
        let mut cut = false;
        let head: Vec<String> = lines
            .by_ref()
            .take(REPLY_HEAD_LINES)
            .map(|line| {
                cut |= line.chars().count() > REPLY_LINE_CHARS;
                line.chars().take(REPLY_LINE_CHARS).collect()
            })
            .collect();
        let rest = lines.next().is_some();
        let head = head.join("\n");
        let more = cut || rest || skipped || head.chars().count() > REPLY_SHOWN_CHARS;
        Self {
            request_offset,
            head,
            bytes: text.len() as u64,
            more,
            at_unix_ms,
        }
    }
}

/// One line of a Markdown message as the words it says: `None` for a table
/// row, a line of a code block or its fence, and a rule or heading underline; otherwise the line
/// without its heading, quote, list and task marks and its emphasis, code
/// and link marks. `fence` carries whether the line is inside a code block.
fn prose_line(line: &str, fence: &mut bool) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        *fence = !*fence;
        return None;
    }
    if *fence || trimmed.starts_with('|') {
        return None;
    }
    let marks: Vec<char> = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if marks.len() >= 3
        && ['-', '*', '_', '=']
            .iter()
            .any(|mark| marks.iter().all(|c| c == mark))
    {
        return None;
    }
    let mut rest = trimmed;
    loop {
        let before = rest;
        rest = rest.trim_start_matches('>').trim_start();
        let hashes = rest.len() - rest.trim_start_matches('#').len();
        if (1..=6).contains(&hashes) && rest[hashes..].starts_with(' ') {
            rest = rest[hashes..].trim_start();
        }
        for bullet in ["- ", "* ", "+ "] {
            if let Some(after) = rest.strip_prefix(bullet) {
                rest = after.trim_start();
            }
        }
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits > 0 && (rest[digits..].starts_with(". ") || rest[digits..].starts_with(") ")) {
            rest = rest[digits + 2..].trim_start();
        }
        for task in ["[ ] ", "[x] ", "[X] "] {
            if let Some(after) = rest.strip_prefix(task) {
                rest = after.trim_start();
            }
        }
        if rest == before {
            break;
        }
    }
    Some(inline_plain(rest))
}

/// A line's words without its emphasis and code marks, and a link or an
/// image as its text.
fn inline_plain(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        let link = if c == '[' {
            Some(&rest[1..])
        } else {
            rest.strip_prefix("![")
        };
        if let Some(after) = link
            && let Some(close) = after.find("](")
            && let Some(end) = after[close + 2..].find(')')
        {
            out.push_str(&after[..close]);
            rest = &after[close + 2 + end + 1..];
            continue;
        }
        if let Some(after) = ["**", "__", "~~"]
            .iter()
            .find_map(|mark| rest.strip_prefix(mark))
        {
            rest = after;
            continue;
        }
        if c != '`' {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

type Shown = (
    Option<String>,
    Option<String>,
    Option<Request>,
    Option<Request>,
    Option<u64>,
    usize,
    Vec<Reply>,
);

/// What the requester judgement knows about the pane's input.
pub(crate) struct InputView<'a> {
    pub(crate) submits: &'a [Submit],
    /// The bell lines Hide typed into the pane.
    pub(crate) bells: &'a [Bell],
    /// When this daemon began seeing the pane's input.
    pub(crate) observed_since_unix_ms: u64,
    /// The newest submit already matched to a message.
    pub(crate) claimed: &'a mut Option<Submit>,
}

/// The parts of a read the facts take.
pub(crate) struct ReadFacts<'a> {
    pub(crate) events: &'a [LabelEvent],
    pub(crate) title: Option<&'a str>,
    pub(crate) custom_title: Option<&'a str>,
    pub(crate) sightings: &'a [PrSighting],
    pub(crate) subagents: &'a BTreeMap<String, ConversationCheckpoint>,
}

impl SessionFacts {
    /// Folds one read in. Returns whether anything a row shows changed.
    pub(crate) fn fold(
        &mut self,
        read: ReadFacts<'_>,
        input: InputView<'_>,
        log: &LogTarget<'_>,
    ) -> bool {
        let before = self.shown();
        if let Some(title) = read.title {
            self.title = (!title.trim().is_empty()).then(|| title.trim().to_owned());
        }
        if let Some(title) = read.custom_title {
            self.custom_title = (!title.trim().is_empty()).then(|| title.trim().to_owned());
        }
        for (name, checkpoint) in read.subagents {
            self.subagents.insert(name.clone(), checkpoint.clone());
        }
        let mut input = input;
        // Not seen yet, or seen at one offset for good: whichever read first
        // holds a request records it, because an agent can write its session
        // before its first prompt and the prompt then arrives in a later read.
        // Only a rewritten conversation (`forget_offsets`) starts over.
        if self.first_request_offset.is_none() {
            self.first_request_offset = read
                .events
                .iter()
                .find(|event| event.kind == LabelEventKind::Human && is_request(event))
                .map(|event| event.offset);
        }
        for event in read.events {
            match event.kind {
                LabelEventKind::Human => {
                    self.request(event, &mut input, log);
                    if is_request(event) {
                        self.reply_to = Some(event.offset);
                    }
                }
                LabelEventKind::Assistant if !event.text.trim().is_empty() => {
                    self.last_reply_at_unix_ms = Some(event.at_unix_ms);
                    if let Some(request) = self.reply_to {
                        self.note_reply(Reply::of(request, &event.text, event.at_unix_ms));
                    }
                }
                _ => {}
            }
        }
        self.sight(read.sightings, log);
        before != self.shown()
    }

    /// Records each sighted pull request the session made: one its creating
    /// call gave back, or whose address a tool printed when GitHub made it
    /// (`links::made_by_print`). Returns whether one was added.
    pub(crate) fn judge_created(&mut self, times: &PullRequestTimes) -> bool {
        let mut added = false;
        for sighting in &self.sightings {
            let repository = sighting.repository.to_ascii_lowercase();
            let Some(&created) = times.get(&(repository.clone(), sighting.number)) else {
                continue;
            };
            let at = sighting.at_unix_ms;
            let made_then = crate::links::made_by_print(sighting.made, at, Some(created));
            let known = self
                .created_prs
                .iter()
                .any(|kept| kept.number == sighting.number && kept.repository == repository);
            if made_then && !known {
                if self.created_prs.len() >= MAX_CREATED {
                    self.created_prs.remove(0);
                }
                self.created_prs.push(CreatedPr {
                    repository,
                    number: sighting.number,
                    sighted_at_unix_ms: at,
                });
                added = true;
            }
        }
        added
    }

    /// Whether the label analysis reads the person's message at `offset`
    /// (D-09): the operator's, the session's first request (which a
    /// delegated child's parent sent), and one not judged yet.
    pub(crate) fn feeds_analysis(&self, offset: u64) -> bool {
        self.first_request_offset == Some(offset)
            || self
                .verdicts
                .iter()
                .find(|verdict| verdict.offset == offset)
                .is_none_or(|verdict| verdict.requester.is_operator())
    }

    /// Who sent the person's message at `offset`, once judged.
    pub(crate) fn requester_at(&self, offset: u64) -> Option<&Requester> {
        self.verdicts
            .iter()
            .find(|verdict| verdict.offset == offset)
            .map(|verdict| &verdict.requester)
    }

    /// Work is given by the operator for roots, and by the parent for
    /// delegated sessions. Reuse the kept input verdict and canonical letter.
    pub(crate) fn feeds_work(&self, event: &LabelEvent, parent: Option<&str>) -> bool {
        // The operator's View feedback is the operator's own prompt (D-12).
        if hide_session::envelope_from_operator(&event.text) {
            return parent.is_none();
        }
        if let Some((sender, kind)) = hide_session::envelope_header(&event.text) {
            return parent == Some(sender.as_str()) && kind == "request";
        }
        match parent {
            Some(_) => self.first_request_offset == Some(event.offset) && event.sender.is_none(),
            None => event.sender.is_none() && self.feeds_analysis(event.offset),
        }
    }

    /// Keeps `reply` as its request's answer, and only the answers to the
    /// operator's last request and to the newest one.
    fn note_reply(&mut self, reply: Reply) {
        self.replies
            .retain(|kept| kept.request_offset != reply.request_offset);
        self.replies.push(reply);
        let operator = self
            .operator_request
            .as_ref()
            .and_then(|request| request.offset);
        let newest = self.reply_to;
        self.replies.retain(|kept| {
            Some(kept.request_offset) == operator || Some(kept.request_offset) == newest
        });
    }

    /// A rewritten conversation: the offsets the requesters were kept by no
    /// longer name the same messages.
    pub(crate) fn forget_offsets(&mut self) {
        self.reply_to = None;
        self.replies.clear();
        self.verdicts.clear();
        self.first_request_offset = None;
        // A sender category may have disappeared from the rewritten file.
        // Rebuild both from the reread rather than retaining a removed prompt.
        self.operator_request = None;
        self.other_request = None;
        self.prompt_digests.clear();
    }

    /// What a row draws from these facts, to tell whether a read moved it.
    fn shown(&self) -> Shown {
        (
            self.title.clone(),
            self.custom_title.clone(),
            self.operator_request.clone(),
            self.other_request.clone(),
            self.last_reply_at_unix_ms,
            self.sightings.len(),
            self.replies.clone(),
        )
    }

    fn request(&mut self, event: &LabelEvent, input: &mut InputView<'_>, log: &LogTarget<'_>) {
        if !is_request(event) {
            return;
        }
        let first = self.first_request_offset == Some(event.offset);
        let requester = match self
            .verdicts
            .iter()
            .find(|verdict| verdict.offset == event.offset)
        {
            Some(verdict) => verdict.requester.clone(),
            None => {
                let requester = judge(event, input, log);
                if self.verdicts.len() >= MAX_VERDICTS {
                    self.verdicts.pop_front();
                }
                self.verdicts.push_back(Verdict {
                    offset: event.offset,
                    requester: requester.clone(),
                });
                requester
            }
        };
        // A read that starts again at the last human record, as the first
        // one on a new engine does, reads the same requests again.
        let digest = hide_agent_hooks::delivery::prompt_digest(&event.text);
        self.prompt_digests.retain(|known| *known != digest);
        if self.prompt_digests.len() >= MAX_PROMPT_DIGESTS {
            self.prompt_digests.pop_front();
        }
        self.prompt_digests.push_back(digest);
        let (text, cut) = capped(&event.text);
        let request = Request {
            text,
            offset: Some(event.offset),
            cut,
            images: event.images,
            at_unix_ms: event.at_unix_ms,
            requester,
            first,
        };
        if request.requester.is_operator() {
            self.operator_request = Some(request);
        } else {
            self.other_request = Some(request);
        }
    }

    /// Keeps each address once, at its first sighting, and the newest
    /// [`MAX_SIGHTINGS`] of them.
    fn sight(&mut self, sightings: &[PrSighting], log: &LogTarget<'_>) {
        for sighting in sightings {
            match self.sightings.iter_mut().find(|kept| {
                kept.number == sighting.number
                    && kept.repository.eq_ignore_ascii_case(&sighting.repository)
            }) {
                Some(kept) => kept.made |= sighting.made,
                None => self.sightings.push(sighting.clone()),
            }
        }
        if self.sightings.len() > MAX_SIGHTINGS {
            let dropped = self.sightings.len() - MAX_SIGHTINGS;
            self.sightings.sort_by_key(|sighting| sighting.at_unix_ms);
            self.sightings.drain(..dropped);
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "read.sightings_capped",
                "target": log.target,
                "pane_id": log.pane_id,
                "dropped": dropped,
            }));
        }
    }
}

/// Who a diagnostic is about.
pub(crate) struct LogTarget<'a> {
    pub(crate) target: &'a str,
    pub(crate) pane_id: &'a str,
}

/// The bell Hide typed that wrote `event`, by the record it kept (D-19).
pub(crate) fn rang<'a>(bells: &'a [Bell], event: &LabelEvent) -> Option<&'a Bell> {
    bells.iter().rev().find(|bell| {
        bell.line == event.text.trim()
            && bell.at_unix_ms <= event.at_unix_ms.saturating_add(CLOCK_SLACK_MS)
            && event.at_unix_ms.saturating_sub(bell.at_unix_ms) <= QUEUED_SUBMIT_WINDOW_MS
    })
}

fn judge(event: &LabelEvent, input: &mut InputView<'_>, log: &LogTarget<'_>) -> Requester {
    // A letter the operator's Send made reads as the operator's request
    // (PRD view-opener-feedback D-11).
    if event.sender.is_some() && hide_session::envelope_from_operator(&event.text) {
        return Requester::Operator;
    }
    if let Some(sender) = event.sender.as_ref() {
        return Requester::Named(sender.clone());
    }
    // A bell line Hide typed is the letter's, by the record Hide kept when
    // it typed it, not by how the words look (D-19).
    if let Some(bell) = rang(input.bells, event) {
        return Requester::Named(bell.sender.clone());
    }
    if event.at_unix_ms < input.observed_since_unix_ms {
        return Requester::Unobserved;
    }
    let fits = |submit: &Submit| {
        let window = if submit.while_working {
            QUEUED_SUBMIT_WINDOW_MS
        } else {
            SUBMIT_WINDOW_MS
        };
        submit.at_unix_ms <= event.at_unix_ms.saturating_add(CLOCK_SLACK_MS)
            && event.at_unix_ms.saturating_sub(submit.at_unix_ms) <= window
    };
    let after = input.claimed.map_or(0, |claimed| claimed.seq);
    if let Some(submit) = input
        .submits
        .iter()
        .find(|submit| submit.seq > after && fits(submit))
    {
        *input.claimed = Some(*submit);
        return Requester::Operator;
    }
    // One submit fits two messages: the earlier took it, and this one is
    // read as another agent's (PRD Risks).
    if input.claimed.as_ref().is_some_and(fits) {
        crate::diagnostic!(json!({
            "component": "labels",
            "kind": "requester.ambiguous",
            "target": log.target,
            "pane_id": log.pane_id,
            "offset": event.offset,
        }));
    }
    Requester::Agent
}

/// User messages whose digests a session's facts keep: enough for the brief
/// and the few prompts around it while Hide waits for its record.
const MAX_PROMPT_DIGESTS: usize = 8;

fn is_request(event: &LabelEvent) -> bool {
    !event.text.trim().is_empty() || event.images > 0
}

fn capped(text: &str) -> (String, bool) {
    if text.len() <= MAX_TEXT_BYTES {
        return (text.to_owned(), false);
    }
    let mut end = MAX_TEXT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The turn card's head reads an answer's words: a table, a code block and
    // a rule are passed over, and Markdown's marks are left out.
    #[test]
    fn a_reply_head_reads_the_words_of_a_markdown_answer() {
        let text = "## 결과\n\n| # | 고친 문제 | 상태 |\n|---|---|---|\n| 1 | 카드 | 완료 |\n\n```sh\ngh pr view\n```\n---\n- **두 문제** 모두 `main`에 [PR #12](https://x/12)로 들어감\n> 1. ~~남은~~ 일 없음";
        let reply = Reply::of(7, text, 1);
        assert_eq!(reply.head, "결과\n두 문제 모두 main에 PR #12로 들어감");
        assert!(
            reply.more,
            "the table and the line after are more than the head"
        );
        let plain = Reply::of(7, "그대로 둔 줄 snake_case_name *별표*\n둘째 줄", 1);
        assert_eq!(plain.head, "그대로 둔 줄 snake_case_name *별표*\n둘째 줄");
        assert!(!plain.more);
    }

    const LOG: LogTarget<'static> = LogTarget {
        target: "local",
        pane_id: "p1",
    };

    fn human(text: &str, at: u64, offset: u64) -> LabelEvent {
        LabelEvent {
            kind: LabelEventKind::Human,
            at_unix_ms: at,
            text: text.to_owned(),
            offset,
            images: 0,
            sender: None,
        }
    }

    fn submit(seq: u64, at: u64, while_working: bool) -> Submit {
        Submit {
            seq,
            at_unix_ms: at,
            while_working,
        }
    }

    fn fold(
        facts: &mut SessionFacts,
        events: &[LabelEvent],
        submits: &[Submit],
        claimed: &mut Option<Submit>,
    ) {
        fold_with_bells(facts, events, submits, &[], claimed);
    }

    fn fold_with_bells(
        facts: &mut SessionFacts,
        events: &[LabelEvent],
        submits: &[Submit],
        bells: &[Bell],
        claimed: &mut Option<Submit>,
    ) {
        facts.fold(
            ReadFacts {
                events,
                title: None,
                custom_title: None,
                sightings: &[],
                subagents: &BTreeMap::new(),
            },
            InputView {
                submits,
                bells,
                observed_since_unix_ms: 1_000,
                claimed,
            },
            &LOG,
        );
    }

    /// The record Claude Code 2.1.316 wrote for a brief Hide pasted, read by
    /// Hide's own reader, gives the digest of the brief Hide typed: the fact
    /// the core confirms the brief by (`Runtime::typed_input_work`).
    #[test]
    fn claude_codes_record_of_a_pasted_brief_has_the_digest_of_the_brief_hide_typed() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/claude-pasted-brief-session.json"
        ))
        .unwrap();
        let home = tempfile::tempdir().unwrap();
        let session = fixture["record"]["sessionId"].as_str().unwrap();
        let path = home
            .path()
            .join(".claude/projects")
            .join("-work")
            .join(format!("{session}.jsonl"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{}\n", fixture["record"])).unwrap();
        let read = hide_session::label_transcript::read(
            &hide_session::Accounts::new(home.path()),
            &hide_session::label_transcript::LabelTranscriptRequest {
                account: Default::default(),
                agent: hide_session::Agent::Claude,
                reference_kind: "path".into(),
                reference_value: path.to_str().unwrap().into(),
                cwd: Some("/work".into()),
                checkpoint: None,
                subagents: Default::default(),
                turns: None,
                tools: None,
            },
        )
        .unwrap();
        let mut facts = SessionFacts::default();
        fold(&mut facts, &read.events, &[], &mut None);
        let typed = fixture["typed"].as_str().unwrap();
        assert_eq!(
            Vec::from(facts.prompt_digests),
            [hide_agent_hooks::delivery::prompt_digest(typed)]
        );
    }

    fn assistant(text: &str, at: u64, offset: u64) -> LabelEvent {
        LabelEvent {
            kind: LabelEventKind::Assistant,
            ..human(text, at, offset)
        }
    }

    /// The turn card's answer is the agent's last message after the
    /// operator's request: later letters' turns keep their own answers beside
    /// it without pushing it out, and a rewritten conversation forgets both.
    #[test]
    fn the_operators_answer_stays_beside_the_newest_and_each_is_its_last_message() {
        let mut facts = SessionFacts::default();
        let mut claimed = None;
        let long = "네, 생길 수 있습니다.\n\n다만 흔하지 않습니다.\n| 상황 | 예 |";
        fold(
            &mut facts,
            &[
                human("기본 폴더 없이 프로필만 있을 수 있어?", 10_400, 1),
                assistant("확인해 볼게요", 10_500, 2),
                assistant(long, 10_600, 3),
            ],
            &[submit(1, 10_000, false)],
            &mut claimed,
        );
        let answer = &facts.replies[0];
        assert_eq!(answer.request_offset, 1);
        assert_eq!(answer.head, "네, 생길 수 있습니다.\n다만 흔하지 않습니다.");
        assert_eq!(answer.bytes, long.len() as u64);
        assert!(answer.more, "a third line is not in the head");
        for (offset, at) in [(4, 11_000), (6, 12_000)] {
            fold(
                &mut facts,
                &[
                    human("🦉 reviewer 보고: 승인", at, offset),
                    assistant("다음 단계로 갑니다", at + 1, offset + 1),
                ],
                &[],
                &mut claimed,
            );
        }
        assert_eq!(
            facts
                .replies
                .iter()
                .map(|reply| reply.request_offset)
                .collect::<Vec<_>>(),
            vec![1, 6]
        );
        assert!(!facts.replies[1].more);
        facts.forget_offsets();
        assert!(facts.replies.is_empty());
    }

    #[test]
    fn a_submit_just_before_a_message_makes_it_the_operators_and_anything_else_is_an_agents() {
        let mut facts = SessionFacts::default();
        let mut claimed = None;
        let submits = [submit(1, 10_000, false)];
        fold(
            &mut facts,
            &[
                human("fix the build", 10_400, 1),
                human("CI is red on #12", 11_000, 2),
            ],
            &submits,
            &mut claimed,
        );
        let operator = facts.operator_request.as_ref().unwrap();
        assert_eq!(
            (operator.text.as_str(), &operator.requester),
            ("fix the build", &Requester::Operator)
        );
        assert!(operator.first);
        let other = facts.other_request.as_ref().unwrap();
        assert_eq!(other.requester, Requester::Agent, "one submit, one message");
    }

    #[test]
    fn a_bell_line_hide_typed_is_the_senders_letter_even_after_an_operator_submit() {
        let mut facts = SessionFacts::default();
        let mut claimed = None;
        // The operator pressed Enter just before Hide typed the bell, which
        // the submit window alone would read as the operator's request.
        let submits = [submit(1, 10_000, false)];
        let bells = [Bell {
            at_unix_ms: 10_100,
            line: "🦉 letter from ci-lead".into(),
            sender: "ci-lead".into(),
            kind: "request".into(),
        }];
        fold_with_bells(
            &mut facts,
            &[human("🦉 letter from ci-lead", 10_300, 1)],
            &submits,
            &bells,
            &mut claimed,
        );
        assert!(facts.operator_request.is_none());
        let letter = facts.other_request.as_ref().unwrap();
        assert_eq!(letter.requester, Requester::Named("ci-lead".into()));
    }

    #[test]
    fn a_native_title_clear_removes_the_old_name_but_absence_preserves_it() {
        let mut facts = SessionFacts::default();
        let mut claimed = None;
        for (title, expected) in [
            (Some("Pi name"), Some("Pi name")),
            (None, Some("Pi name")),
            (Some(""), None),
        ] {
            facts.fold(
                ReadFacts {
                    events: &[],
                    title: None,
                    custom_title: title,
                    sightings: &[],
                    subagents: &BTreeMap::new(),
                },
                InputView {
                    submits: &[],
                    bells: &[],
                    observed_since_unix_ms: 1_000,
                    claimed: &mut claimed,
                },
                &LOG,
            );
            assert_eq!(facts.custom_title.as_deref(), expected);
        }
    }

    #[test]
    fn an_envelope_names_its_sender_and_an_unobserved_message_counts_as_the_operators() {
        let mut facts = SessionFacts::default();
        let mut claimed = None;
        let mut enveloped = human(
            "Hide letter letter-1 from ci-lead (claude) [request]\nrerun",
            5_000,
            2,
        );
        enveloped.sender = Some("ci-lead".to_owned());
        fold(
            &mut facts,
            &[human("old request", 500, 1), enveloped],
            &[],
            &mut claimed,
        );
        assert_eq!(
            facts.operator_request.as_ref().unwrap().requester,
            Requester::Unobserved
        );
        assert_eq!(
            facts.other_request.as_ref().unwrap().requester,
            Requester::Named("ci-lead".to_owned())
        );
    }

    #[test]
    fn a_prompt_queued_while_working_is_matched_when_it_is_written() {
        let mut facts = SessionFacts::default();
        let mut claimed = None;
        let submits = [submit(1, 10_000, true), submit(2, 12_000, true)];
        fold(
            &mut facts,
            &[
                human("first queued", 600_000, 1),
                human("second queued", 900_000, 2),
            ],
            &submits,
            &mut claimed,
        );
        assert_eq!(
            facts.operator_request.as_ref().unwrap().text,
            "second queued"
        );
        assert!(facts.other_request.is_none());
        assert_eq!(claimed, Some(submits[1]));
    }

    #[test]
    fn a_verdict_is_kept_so_a_read_from_the_anchor_does_not_judge_again() {
        let mut facts = SessionFacts::default();
        let mut claimed = None;
        fold(
            &mut facts,
            &[human("do it", 10_100, 7)],
            &[submit(1, 10_000, false)],
            &mut claimed,
        );
        let restored: SessionFacts =
            serde_json::from_value(serde_json::to_value(&facts).unwrap()).unwrap();
        let mut facts = restored;
        // A restarted daemon has no submits; the stored verdict stands.
        fold(&mut facts, &[human("do it", 10_100, 7)], &[], &mut None);
        assert_eq!(
            facts.operator_request.as_ref().unwrap().requester,
            Requester::Operator
        );
        assert!(facts.other_request.is_none());
    }

    /// D-31, and the review of #1859: mission-13's #1810 to #1813 were each
    /// printed by the `gh pr create` that made them up to 3.2 s before the
    /// time GitHub gave them, on another clock. A print its creating call
    /// gave back is the making up to 10 s before; a listing printed as early
    /// is a look.
    #[test]
    fn a_pull_request_is_the_sessions_when_its_creating_call_or_its_tool_printed_it_as_github_made_it()
     {
        let mut facts = SessionFacts::default();
        let sighting = |number, at_unix_ms, made| PrSighting {
            repository: "Acme/App".to_owned(),
            number,
            at_unix_ms,
            made,
        };
        facts.sight(
            &[
                sighting(1, 100_000, false),
                sighting(2, 100_000, false),
                sighting(3, 200_000, false),
                sighting(4, 300_000, true),
                sighting(5, 400_000, false),
                sighting(6, 500_000, false),
                sighting(7, 600_000, true),
            ],
            &LOG,
        );
        // A later read of the same address from its creating call.
        facts.sight(&[sighting(6, 500_100, true)], &LOG);
        let times = PullRequestTimes::from([
            (("acme/app".to_owned(), 1), 90_000), // printed 10 s after: made here
            (("acme/app".to_owned(), 2), 40_000), // printed a minute after: looked at
            (("acme/app".to_owned(), 3), 201_000), // printed a second before: made here
            (("acme/app".to_owned(), 4), 303_400), // its creating call, 3.4 s before: made here
            (("acme/app".to_owned(), 5), 403_000), // a listing 3 s before: looked at
            (("acme/app".to_owned(), 6), 509_000), // its creating call, read again: made here
            (("acme/app".to_owned(), 7), 612_000), // its creating call, 12 s before: not this making
        ]);
        assert!(facts.judge_created(&times));
        assert_eq!(
            facts
                .created_prs
                .iter()
                .map(|pr| pr.number)
                .collect::<Vec<_>>(),
            [1, 3, 4, 6]
        );
        assert!(!facts.judge_created(&times), "judged once");
    }

    #[test]
    fn long_text_is_cut_on_a_character_and_sightings_keep_the_newest_twenty_once_each() {
        let mut facts = SessionFacts::default();
        let long = "가".repeat(MAX_TEXT_BYTES);
        fold(&mut facts, &[human(&long, 500, 1)], &[], &mut None);
        let request = facts.operator_request.as_ref().unwrap();
        assert!(request.cut && request.text.len() <= MAX_TEXT_BYTES);

        let sightings: Vec<PrSighting> = (1..=25)
            .chain([3])
            .map(|number| PrSighting {
                repository: "o/r".to_owned(),
                number,
                at_unix_ms: number * 10,
                made: false,
            })
            .collect();
        facts.sight(&sightings, &LOG);
        assert_eq!(facts.sightings.len(), MAX_SIGHTINGS);
        assert_eq!(facts.sightings.first().unwrap().number, 6);
        assert_eq!(facts.sightings.last().unwrap().number, 25);
    }
    #[test]
    fn delegated_work_uses_the_parent_without_promoting_direct_operator_input_or_reports() {
        let mut facts = SessionFacts::default();
        let initial = human("initial delegated request", 500, 1);
        let direct = human("operator types into child", 10_100, 2);
        let parent = human(
            "Hide letter letter-1 from lead (claude) [request]\nContinue work",
            10_200,
            3,
        );
        let report = human(
            "Hide letter letter-2 from lead (claude) [report]\nStatus",
            10_300,
            4,
        );
        let peer = human(
            "Hide letter letter-3 from peer (codex) [request]\nOther work",
            10_400,
            5,
        );
        let progress = human(
            "Hide letter letter-4 from lead (claude) [progress]\nPlan: core first",
            10_500,
            6,
        );
        fold(
            &mut facts,
            &[
                initial.clone(),
                direct.clone(),
                parent.clone(),
                report.clone(),
                peer.clone(),
                progress.clone(),
            ],
            &[submit(1, 10_000, false)],
            &mut None,
        );
        assert!(facts.feeds_work(&initial, Some("lead")));
        assert!(facts.feeds_work(&parent, Some("lead")));
        for event in [&direct, &report, &peer, &progress] {
            assert!(!facts.feeds_work(event, Some("lead")));
        }
        assert!(facts.feeds_work(&direct, None));
        for event in [&parent, &report, &peer, &progress] {
            assert!(!facts.feeds_work(event, None));
        }
    }

    #[test]
    fn a_delegated_childs_first_request_feeds_work_when_it_arrives_in_a_later_read() {
        // The reader found the session store before the parent's prompt was
        // written: the first read holds no events. The prompt arrives in the
        // next read, and a later request in a third must not replace it.
        let mut facts = SessionFacts::default();
        let mut claimed = None;
        let no_subagents = BTreeMap::new();
        let first_request = human("parent's first prompt", 5_000, 1759);
        let later_request = human("a follow-up", 9_000, 1760);
        for events in [
            &[][..],
            std::slice::from_ref(&first_request),
            std::slice::from_ref(&later_request),
        ] {
            facts.fold(
                ReadFacts {
                    events,
                    title: None,
                    custom_title: None,
                    sightings: &[],
                    subagents: &no_subagents,
                },
                InputView {
                    submits: &[],
                    bells: &[],
                    observed_since_unix_ms: 1_000,
                    claimed: &mut claimed,
                },
                &LOG,
            );
        }
        assert!(
            facts.feeds_work(&first_request, Some("lead")),
            "the parent's first request must feed the child's work analysis"
        );
        assert!(
            !facts.feeds_work(&later_request, Some("lead")),
            "a later request does not replace the first"
        );
        let shown = facts
            .operator_request
            .as_ref()
            .or(facts.other_request.as_ref())
            .expect("the request was read");
        assert!(
            !shown.first,
            "the latest request is not the session's first"
        );
    }

    #[test]
    fn a_first_request_seen_before_a_restart_stays_the_first_after_it() {
        let mut facts = SessionFacts::default();
        let mut claimed = None;
        let first_request = human("parent's first prompt", 5_000, 1759);
        fold(&mut facts, &[], &[], &mut claimed);
        fold(
            &mut facts,
            std::slice::from_ref(&first_request),
            &[],
            &mut claimed,
        );
        let mut restored: SessionFacts =
            serde_json::from_value(serde_json::to_value(&facts).unwrap()).unwrap();
        fold(
            &mut restored,
            &[human("a follow-up", 9_000, 1760)],
            &[],
            &mut claimed,
        );
        assert!(restored.feeds_work(&first_request, Some("lead")));
    }
}
