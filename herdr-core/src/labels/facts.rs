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
const CLOCK_SLACK_MS: u64 = 2_000;

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
}

type Shown = (
    Option<String>,
    Option<String>,
    Option<Request>,
    Option<Request>,
    Option<u64>,
    usize,
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
                LabelEventKind::Human => self.request(event, &mut input, log),
                LabelEventKind::Assistant if !event.text.trim().is_empty() => {
                    self.last_reply_at_unix_ms = Some(event.at_unix_ms);
                }
                _ => {}
            }
        }
        self.sight(read.sightings, log);
        before != self.shown()
    }

    /// Records each sighted pull request whose address a tool printed when
    /// GitHub made it. Returns whether one was added.
    pub(crate) fn judge_created(&mut self, times: &PullRequestTimes) -> bool {
        let mut added = false;
        for sighting in &self.sightings {
            let repository = sighting.repository.to_ascii_lowercase();
            let Some(&created) = times.get(&(repository.clone(), sighting.number)) else {
                continue;
            };
            let at = sighting.at_unix_ms;
            let made_then = crate::links::printed_at_creation(at, created);
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

    /// A rewritten conversation: the offsets the requesters were kept by no
    /// longer name the same messages.
    pub(crate) fn forget_offsets(&mut self) {
        self.verdicts.clear();
        self.first_request_offset = None;
        // A sender category may have disappeared from the rewritten file.
        // Rebuild both from the reread rather than retaining a removed prompt.
        self.operator_request = None;
        self.other_request = None;
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
            let known = self.sightings.iter().any(|kept| {
                kept.number == sighting.number
                    && kept.repository.eq_ignore_ascii_case(&sighting.repository)
            });
            if !known {
                self.sightings.push(sighting.clone());
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

    #[test]
    fn a_pull_request_is_the_sessions_only_when_its_tool_printed_it_as_github_made_it() {
        let mut facts = SessionFacts::default();
        let sighting = |number, at_unix_ms| PrSighting {
            repository: "Acme/App".to_owned(),
            number,
            at_unix_ms,
        };
        facts.sight(
            &[
                sighting(1, 100_000),
                sighting(2, 100_000),
                sighting(3, 200_000),
                sighting(4, 300_000),
                sighting(5, 400_000),
            ],
            &LOG,
        );
        // GitHub's time is another clock's: a print before it is the same
        // making as a print after it.
        let times = PullRequestTimes::from([
            (("acme/app".to_owned(), 1), 90_000), // printed 10 s after: made here
            (("acme/app".to_owned(), 2), 40_000), // printed a minute after: looked at
            (("acme/app".to_owned(), 3), 201_000), // printed a second before: made here
            (("acme/app".to_owned(), 4), 303_400), // printed 3.4 s before: made here
            (("acme/app".to_owned(), 5), 431_000), // printed 31 s before: looked at
        ]);
        assert!(facts.judge_created(&times));
        assert_eq!(
            facts
                .created_prs
                .iter()
                .map(|pr| pr.number)
                .collect::<Vec<_>>(),
            [1, 3, 4]
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
