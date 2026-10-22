//! The operator only receives delegated work that its parent cannot handle.
//! Doorbell and watch workers own the clocks; this reads their decisions.
use std::collections::BTreeMap;

use serde::Serialize;

use crate::delivery::doorbell::Hold;
use crate::delivery::ledger::{Ledger, State};
use crate::model::SidebarAgentSnapshot;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    ParentBlocked,
    Draft,
    BellExhausted,
    Undelivered,
    ChildBlocked,
    ObserverUnconfirmed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Escalation {
    pub cause: Cause,
    pub letter_id: Option<String>,
    pub since_unix_ms: Option<u64>,
    /// The existing delivery human notice already announces these two causes.
    pub human_notice: bool,
}

/// A later waiting reason is not a receipt or successful bell. The worker
/// removes a hold after ringing (or retiring) the letter; until then retain
/// the cause that already raised it. This table remains bounded by the
/// worker's current pending letters and is never used for doorbell verdicts.
pub(crate) fn retain_raised_holds(
    previous: &BTreeMap<String, Hold>,
    mut current: BTreeMap<String, Hold>,
) -> BTreeMap<String, Hold> {
    for (id, old) in previous {
        if matches!(
            old,
            Hold::Blocked | Hold::AwaitingOperator | Hold::Draft | Hold::Exhausted
        ) && let Some(next) = current.get_mut(id)
        {
            *next = *old;
        }
    }
    current
}

pub(crate) fn of(
    child: &SidebarAgentSnapshot,
    ledger: Option<&Ledger>,
    holds: &BTreeMap<String, Hold>,
) -> Option<Escalation> {
    let parent = child.lineage_parent_pane_id.as_deref()?;
    if !child.delegated {
        return None;
    }
    if child.blocked {
        return Some(Escalation {
            cause: Cause::ChildBlocked,
            letter_id: None,
            since_unix_ms: child.changed_at_unix_ms,
            human_notice: false,
        });
    }
    if child.activity == "working" {
        return None;
    }
    let ledger = ledger?;
    for letter in &ledger.letters {
        if matches!(
            letter.state,
            State::Acknowledged | State::Cancelled | State::Expired
        ) || child
            .declared_parent_session
            .as_ref()
            .is_some_and(|session| letter.recipient.session.as_ref() != Some(session))
            || ledger.letters.iter().any(|answer| {
                answer.reply_to.as_deref() == Some(&letter.id)
                    && answer.sender.same_identity(&letter.recipient)
            })
        {
            continue;
        }
        let cause = if let Some(warning) = &letter.watch_warning {
            if warning.ordinal != 1
                || !warning.parent_notified
                || warning.target.pane_id != child.pane_id
                || warning.target.session != child.lineage_session
                || letter.recipient.pane_id != parent
                || !ledger.watches.iter().any(|watch| {
                    watch.target.same_identity(&warning.target)
                        && watch.parent.same_identity(&letter.recipient)
                        && watch.last_activity_at_unix_ms == warning.activity_at_unix_ms
                })
            {
                continue;
            }
            Cause::ObserverUnconfirmed
        } else {
            if letter.sender.pane_id != child.pane_id
                || letter.sender.session != child.lineage_session
                || letter.recipient.pane_id != parent
                || !matches!(letter.kind.as_str(), "request" | "block")
                || took_up_request_after(child, letter.created_at_unix_ms)
            {
                continue;
            }
            if letter.state == State::Undelivered {
                Cause::Undelivered
            } else if letter.state == State::Pending {
                match holds.get(&letter.id) {
                    Some(Hold::Blocked | Hold::AwaitingOperator) => Cause::ParentBlocked,
                    Some(Hold::Draft) => Cause::Draft,
                    Some(Hold::Exhausted) => Cause::BellExhausted,
                    _ => continue,
                }
            } else {
                continue;
            }
        };
        return Some(Escalation {
            cause,
            letter_id: Some(letter.id.clone()),
            since_unix_ms: Some(letter.created_at_unix_ms),
            human_notice: matches!(cause, Cause::Undelivered | Cause::ObserverUnconfirmed),
        });
    }
    None
}

/// Whether the child's session took up a request written after `at`: a new
/// turn means the child moved on without the answer, so its letter no longer
/// raises it, however the turn ends. The time is the session's own record,
/// which the label store keeps across restarts (and reads again for a
/// device), never this daemon's view of the pane.
pub(crate) fn took_up_request_after(child: &SidebarAgentSnapshot, at: u64) -> bool {
    child.row_facts.as_ref().is_some_and(|facts| {
        [&facts.operator_request, &facts.other_request]
            .into_iter()
            .flatten()
            .any(|request| request.at_unix_ms > at)
    })
}

/// What the operator does about one raised descendant, in the order a root
/// shows them: an approval first, then an answer, a confirmation, a draft.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verb {
    Approval,
    Answer,
    Confirm,
    Draft,
}

impl Cause {
    /// `ParentBlocked` has no verb: the blocked parent's own approval stands
    /// for it (a raised child's `ChildBlocked`, or the root's own demand).
    pub(crate) fn verb(self) -> Option<Verb> {
        match self {
            Cause::ChildBlocked => Some(Verb::Approval),
            Cause::Undelivered | Cause::BellExhausted => Some(Verb::Answer),
            Cause::ObserverUnconfirmed => Some(Verb::Confirm),
            Cause::Draft => Some(Verb::Draft),
            Cause::ParentBlocked => None,
        }
    }
}

/// One raised descendant as its lineage root shows it (docs/status-model.md,
/// Delegated escalation): the verb, what to do, who asks, where Open goes and
/// since when. `what` is absent when no label line or letter says it; the
/// shell then words the verb's own fallback.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RaisedAsk {
    pub verb: Verb,
    pub what: Option<String>,
    /// A question the core asks for the raised descendant (`AskSubject`),
    /// which the shell words in place of `what`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<crate::model::AskSubject>,
    /// The raised descendant itself, where a tree draws the ask.
    pub raised_pane_id: String,
    /// The raised descendant's title, even when a draft names its parent as the holder.
    pub origin_title: String,
    /// The agent the band names: the raised descendant, or for a draft the
    /// parent whose input holds it.
    pub pane_id: String,
    pub title: String,
    pub agent_kind: String,
    /// The pane Open goes to: the raised descendant, or the parent holding a draft.
    pub open_pane_id: String,
    pub since_unix_ms: Option<u64>,
    /// For an answer: the parent that has not received the letter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreceived_by: Option<String>,
    /// Titles from the root to the agent named, for the name's hover.
    pub path: Vec<String>,
    pub checkout: Option<String>,
    /// The delivery notice already announces this cause (no second push).
    pub human_notice: bool,
}

/// Lifts every raised descendant to its lineage root (PRD D-10, D-27): the
/// root's asks, lead first, and each row's descendant mark. Rows are the
/// session rows in projection order; `letters` reads a letter's first line.
pub(crate) fn lift(
    rows: &mut [SidebarAgentSnapshot],
    letter_line: impl Fn(&str) -> Option<String>,
) {
    use std::collections::HashMap;
    let index: HashMap<String, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| (row.pane_id.clone(), i))
        .rev()
        .collect();
    let parent_of = |i: usize| -> Option<usize> {
        rows[i]
            .lineage_parent_pane_id
            .as_deref()
            .and_then(|parent| index.get(parent).copied())
            .filter(|&p| p != i)
    };
    // Ancestors nearest first, bounded by the row count against a cycle.
    let ancestors = |i: usize| -> Vec<usize> {
        let mut chain = Vec::new();
        let mut at = i;
        while let Some(parent) = parent_of(at) {
            if chain.contains(&parent) || chain.len() > rows.len() {
                break;
            }
            chain.push(parent);
            at = parent;
        }
        chain
    };
    let mut asks: HashMap<usize, Vec<RaisedAsk>> = HashMap::new();
    let mut raised_below: HashMap<usize, usize> = HashMap::new();
    let mut working_below: HashMap<usize, usize> = HashMap::new();
    // The most recently changed working descendant under each ancestor.
    let mut latest_working: HashMap<usize, usize> = HashMap::new();
    for i in 0..rows.len() {
        let chain = ancestors(i);
        if rows[i].activity == "working" {
            for &a in &chain {
                *working_below.entry(a).or_default() += 1;
                let latest = latest_working.entry(a).or_insert(i);
                if rows[i].changed_at_unix_ms > rows[*latest].changed_at_unix_ms {
                    *latest = i;
                }
            }
        }
        let Some(escalation) = rows[i].escalation.as_ref() else {
            continue;
        };
        let Some(verb) = escalation.cause.verb() else {
            continue;
        };
        // A child blocked on a native question asks for an answer, as its own
        // row does (`turn::demand_verb`), not for an approval.
        let verb = match escalation.cause {
            Cause::ChildBlocked => super::turn::demand_verb(&rows[i]).unwrap_or(verb),
            _ => verb,
        };
        let Some(&root) = chain.last() else {
            continue;
        };
        for &a in &chain {
            *raised_below.entry(a).or_default() += 1;
        }
        let row = &rows[i];
        let parent = chain.first().map(|&p| &rows[p]);
        let named = if verb == Verb::Draft {
            parent.unwrap_or(row)
        } else {
            row
        };
        let label_line = |row: &SidebarAgentSnapshot| {
            row.request
                .as_ref()
                .and_then(|request| request.line.clone())
                .or_else(|| row.progress.clone())
        };
        let what = match verb {
            Verb::Approval | Verb::Confirm => label_line(row),
            // A question the core asks for the child is said by its
            // subject, never by the label's line about the work around it.
            Verb::Answer
                if escalation.cause == Cause::ChildBlocked && row.ask_subject.is_some() =>
            {
                None
            }
            Verb::Answer if escalation.cause == Cause::ChildBlocked => {
                row.detail.clone().or_else(|| label_line(row))
            }
            Verb::Answer => escalation.letter_id.as_deref().and_then(&letter_line),
            Verb::Draft => None,
        };
        let named_at = index[&named.pane_id];
        let path = ancestors(named_at)
            .iter()
            .rev()
            .map(|&a| rows[a].identity_label.clone())
            .chain(std::iter::once(named.identity_label.clone()))
            .collect();
        asks.entry(root).or_default().push(RaisedAsk {
            verb,
            what,
            subject: row
                .ask_subject
                .filter(|_| verb == Verb::Answer && escalation.cause == Cause::ChildBlocked),
            raised_pane_id: row.pane_id.clone(),
            origin_title: row.identity_label.clone(),
            pane_id: named.pane_id.clone(),
            title: named.identity_label.clone(),
            agent_kind: named.agent_kind.clone(),
            open_pane_id: named.pane_id.clone(),
            since_unix_ms: escalation.since_unix_ms,
            // Only a letter can wait unreceived; a child's own native
            // question was never sent to its parent.
            unreceived_by: (verb == Verb::Answer && escalation.cause != Cause::ChildBlocked)
                .then(|| parent.map(|p| p.identity_label.clone()))
                .flatten(),
            path,
            checkout: named.checkout_label.clone(),
            human_notice: escalation.human_notice,
        });
    }
    let working_lines: HashMap<usize, Option<String>> = latest_working
        .into_iter()
        .map(|(a, i)| {
            let row = &rows[i];
            (
                a,
                row.request
                    .as_ref()
                    .and_then(|request| request.line.clone())
                    .or_else(|| row.progress.clone()),
            )
        })
        .collect();
    for (i, row) in rows.iter_mut().enumerate() {
        row.descendant_line = working_lines.get(&i).cloned().flatten();
        let mut raised = asks.remove(&i).unwrap_or_default();
        raised.sort_by_key(|ask| (ask.verb, ask.since_unix_ms));
        row.raised = raised;
        row.descendant_mark = match (raised_below.get(&i), working_below.get(&i)) {
            (Some(&count), _) => Some(DescendantMark {
                kind: MarkKind::Raised,
                count,
            }),
            (None, Some(&count)) => Some(DescendantMark {
                kind: MarkKind::Working,
                count,
            }),
            (None, None) => None,
        };
    }
}

/// The one mark a folded parent wears for its descendants (PRD D-40): the
/// raised ones if any, else the working ones, else none.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DescendantMark {
    pub kind: MarkKind,
    pub count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkKind {
    Raised,
    Working,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::{Actor, mailbox};
    use crate::sidebar::{SessionSnapshotPayload, project_agents};
    use serde_json::json;

    fn child() -> SidebarAgentSnapshot {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"pane_id":"child", "agent":"codex", "agent_status":"done", "state_change_seq":1}
        ]}))
        .unwrap();
        let mut row = project_agents(payload).agents.remove(0);
        row.delegated = true;
        row.lineage_parent_pane_id = Some("parent".into());
        row.lineage_session = Some("child-session".into());
        row.declared_parent_session = Some("parent-session".into());
        row
    }

    /// A delegated child of `kind` that Herdr reports as `status` and whose
    /// session says it waits for the operator.
    fn waiting_child(kind: &str, status: &str) -> SidebarAgentSnapshot {
        let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"pane_id":"child", "agent":kind, "agent_status":status, "state_change_seq":1}
        ]}))
        .unwrap();
        payload.agents[0].facts = Some(crate::request_view::RowFacts {
            awaiting_operator: true,
            ..Default::default()
        });
        let mut row = project_agents(payload).agents.remove(0);
        row.delegated = true;
        row.lineage_parent_pane_id = Some("parent".into());
        row
    }

    /// Herdr keeps Grok `working` while it waits for a plan approval or an
    /// answer, and what Grok waits for is read from its current state, so a
    /// delegated Grok child rises as blocked all the same.
    #[test]
    fn a_delegated_grok_child_waiting_for_the_operator_rises_while_herdr_reports_working() {
        for status in ["working", "idle"] {
            let row = waiting_child("grok", status);
            assert!(row.blocked, "{status}");
            let escalation = of(&row, None, &BTreeMap::new()).expect("raised");
            assert_eq!(escalation.cause, Cause::ChildBlocked);
        }
    }

    /// Codex's plan menu is read from how the last turn ended, which says
    /// nothing once Herdr reports the next turn running: that child is not
    /// held, and the same read holds it once it stops.
    #[test]
    fn a_codex_plan_menu_read_from_history_does_not_hold_a_working_child() {
        let working = waiting_child("codex", "working");
        assert!(!working.blocked);
        assert!(of(&working, None, &BTreeMap::new()).is_none());
        let stopped = waiting_child("codex", "idle");
        assert!(stopped.blocked);
        assert_eq!(
            of(&stopped, None, &BTreeMap::new()).map(|raised| raised.cause),
            Some(Cause::ChildBlocked)
        );
    }

    /// A root, its child and a grandchild the given cause raised, as `lift` reads them.
    fn raised_lineage(cause: Cause) -> Vec<SidebarAgentSnapshot> {
        let mut root = child();
        root.pane_id = "root".into();
        root.identity_label = "Root".into();
        root.delegated = false;
        root.lineage_parent_pane_id = None;
        let mut middle = child();
        middle.pane_id = "middle".into();
        middle.identity_label = "Middle".into();
        middle.lineage_parent_pane_id = Some("root".into());
        let mut raised = child();
        raised.pane_id = "raised".into();
        raised.identity_label = "Raised".into();
        raised.lineage_parent_pane_id = Some("middle".into());
        raised.escalation = Some(Escalation {
            cause,
            letter_id: None,
            since_unix_ms: Some(10),
            human_notice: false,
        });
        vec![root, middle, raised]
    }

    // agent-hierarchy-screens B3: a descendant blocked on a native question
    // asks the operator to answer it, in its own words.
    #[test]
    fn a_descendant_blocked_on_a_native_question_is_lifted_as_an_answer() {
        let mut rows = raised_lineage(Cause::ChildBlocked);
        rows[2].blocked = true;
        rows[2].demand = "question".into();
        rows[2].detail = Some("Keep the old stdin path?".into());
        rows[2].user_turn = Some(hide_session::turns::UserTurnFact {
            kind: hide_session::turns::UserTurnKind::Question,
            content: None,
        });
        lift(&mut rows, |_| None);
        let ask = &rows[0].raised[0];
        assert_eq!(ask.verb, Verb::Answer);
        assert_eq!(ask.what.as_deref(), Some("Keep the old stdin path?"));
        assert_eq!(ask.path, ["Root", "Middle", "Raised"]);
        assert_eq!(ask.unreceived_by, None);
    }

    // hide-plan B14: a raised question the core asks for the descendant
    // reaches its root as its subject, never as the label's line about the
    // work around it.
    #[test]
    fn a_raised_question_the_core_asks_for_a_descendant_keeps_its_subject() {
        let mut rows = raised_lineage(Cause::ChildBlocked);
        rows[2].blocked = true;
        rows[2].demand = "question".into();
        rows[2].progress = Some("Writing the core events".into());
        rows[2].user_turn = Some(hide_session::turns::UserTurnFact {
            kind: hide_session::turns::UserTurnKind::Question,
            content: None,
        });
        let subject = crate::model::AskSubject::PlanReview { steps: 4 };
        rows[2].ask_subject = Some(subject);
        lift(&mut rows, |_| None);
        let ask = &rows[0].raised[0];
        assert_eq!(
            (ask.verb, ask.what.as_deref(), ask.subject),
            (Verb::Answer, None, Some(subject))
        );
        rows[0].group = "needs_you".into();
        let state = super::super::turn::row_state(&rows[0]);
        assert_eq!(state.ask.and_then(|ask| ask.subject), Some(subject));
    }

    // B1 with agent-blocked-state B2: once its own block is read, a root that
    // waits on its children asks the raised approval, in the warning colour.
    #[test]
    fn a_read_block_on_a_waiting_root_gives_way_to_the_raised_ask() {
        let mut rows = raised_lineage(Cause::ChildBlocked);
        rows[2].blocked = true;
        rows[0].demand = "error".into();
        rows[0].unread = false;
        rows[0].group = "needs_you".into();
        rows[0].wait = Some(crate::model::AgentWait::Children);
        lift(&mut rows, |_| None);
        assert_eq!(super::super::turn::demand_verb(&rows[0]), None);
        rows[0].mark = super::super::tally::RowMark::of(&rows[0]);
        assert_eq!(rows[0].mark, super::super::tally::RowMark::Turn);
        let state = super::super::turn::row_state(&rows[0]);
        assert_eq!(state.ask.map(|ask| ask.verb), Some(Verb::Approval));
        assert_eq!(state.mark_tone.kind, "warning");
    }

    // B3, B6: a blocked menu stays an approval; a draft names and opens the
    // parent holding it, while the tree still draws it on the raised row.
    #[test]
    fn a_blocked_menu_is_an_approval_and_a_draft_opens_the_parent_holding_it() {
        let mut rows = raised_lineage(Cause::ChildBlocked);
        rows[2].blocked = true;
        lift(&mut rows, |_| None);
        assert_eq!(rows[0].raised[0].verb, Verb::Approval);

        let mut rows = raised_lineage(Cause::Draft);
        lift(&mut rows, |_| None);
        let ask = &rows[0].raised[0];
        assert_eq!(ask.verb, Verb::Draft);
        assert_eq!(
            (
                ask.raised_pane_id.as_str(),
                ask.pane_id.as_str(),
                ask.open_pane_id.as_str()
            ),
            ("raised", "middle", "middle")
        );
        assert_eq!(ask.path, ["Root", "Middle"]);
        assert_eq!(ask.origin_title, "Raised");
    }

    fn actor(pane: &str) -> Actor {
        Actor {
            pane_id: pane.into(),
            name: pane.into(),
            kind: "codex".into(),
            device_id: "local".into(),
            session: Some(format!("{pane}-session")),
        }
    }

    #[test]
    fn session_escalation_clears_on_ack_reply_cancel_or_successful_bell() {
        let child = child();
        for hold in [Hold::Blocked, Hold::Draft, Hold::Exhausted] {
            let mut ledger = Ledger::default();
            mailbox::send(
                &mut ledger,
                &actor("child"),
                &actor("parent"),
                "request",
                "question",
                "request",
                None,
                100,
            )
            .unwrap();
            let id = ledger.letters[0].id.clone();
            let holds = BTreeMap::from([(id.clone(), hold)]);
            assert!(of(&child, Some(&ledger), &holds).is_some());
            for waiting in [Hold::Working, Hold::Quiet, Hold::NotReady] {
                let retained = retain_raised_holds(&holds, BTreeMap::from([(id.clone(), waiting)]));
                assert_eq!(
                    of(&child, Some(&ledger), &retained),
                    of(&child, Some(&ledger), &holds)
                );
            }
            assert!(retain_raised_holds(&holds, BTreeMap::new()).is_empty());
            assert!(
                of(&child, Some(&ledger), &BTreeMap::new()).is_none(),
                "a successful bell clears the hold"
            );
            for state in [State::Acknowledged, State::Cancelled, State::Expired] {
                ledger.letters[0].state = state;
                assert!(of(&child, Some(&ledger), &holds).is_none());
            }
            ledger.letters[0].state = State::Pending;
            mailbox::send(
                &mut ledger,
                &actor("parent"),
                &actor("child"),
                "reply",
                "answer",
                "reply",
                Some(id),
                200,
            )
            .unwrap();
            assert!(
                of(&child, Some(&ledger), &holds).is_none(),
                "a causal parent answer clears the request"
            );
        }
    }

    // status-model.md, Delegated escalation: a child that took up a new turn
    // after its letter moved on without the answer, so stopping again does
    // not bring the letter's raise back. The 2026-10-09 case: a block letter
    // went undelivered, the child carried on and merged its PR, and stayed
    // in Needs You after it stopped.
    #[test]
    fn session_letter_raise_ends_for_good_once_the_child_takes_up_a_later_request() {
        use crate::labels::facts::{Request, Requester};
        use crate::request_view::RowFacts;
        let request = |at, requester| Request {
            text: "next".into(),
            offset: None,
            cut: false,
            images: 0,
            at_unix_ms: at,
            requester,
            first: false,
        };
        let cases = [
            (State::Undelivered, None, Cause::Undelivered),
            (State::Pending, Some(Hold::Blocked), Cause::ParentBlocked),
            (State::Pending, Some(Hold::Draft), Cause::Draft),
            (State::Pending, Some(Hold::Exhausted), Cause::BellExhausted),
        ];
        for (state, hold, cause) in cases {
            for requester in [Requester::Agent, Requester::Operator] {
                let mut child = child();
                let mut ledger = Ledger::default();
                mailbox::send(
                    &mut ledger,
                    &actor("child"),
                    &actor("parent"),
                    "block",
                    "question",
                    "block",
                    None,
                    100,
                )
                .unwrap();
                ledger.letters[0].state = state;
                let holds = hold
                    .map(|hold| BTreeMap::from([(ledger.letters[0].id.clone(), hold)]))
                    .unwrap_or_default();
                let facts = |at| RowFacts {
                    other_request: (requester == Requester::Agent)
                        .then(|| request(at, requester.clone())),
                    operator_request: (requester == Requester::Operator)
                        .then(|| request(at, requester.clone())),
                    ..RowFacts::default()
                };
                child.row_facts = Some(facts(90));
                assert_eq!(
                    of(&child, Some(&ledger), &holds).map(|raised| raised.cause),
                    Some(cause),
                    "the turn that wrote the letter is still the one waiting"
                );
                child.row_facts = Some(facts(100));
                assert_eq!(
                    of(&child, Some(&ledger), &holds).map(|raised| raised.cause),
                    Some(cause),
                    "a request no later than the letter is not a new turn"
                );
                child.row_facts = Some(facts(101));
                child.activity = "working".into();
                assert!(of(&child, Some(&ledger), &holds).is_none());
                child.activity = "stopped".into();
                assert!(
                    of(&child, Some(&ledger), &holds).is_none(),
                    "{cause:?} does not come back when the child stops again"
                );
                child.blocked = true;
                assert_eq!(
                    of(&child, Some(&ledger), &holds).map(|raised| raised.cause),
                    Some(Cause::ChildBlocked),
                    "the child's own menu still raises it"
                );
            }
        }
    }

    #[test]
    fn session_watch_escalates_only_the_unanswered_human_notified_current_episode() {
        let child = child();
        let mut ledger = Ledger::default();
        crate::delivery::watch::start(&mut ledger, &actor("parent"), &actor("child"), 100).unwrap();
        mailbox::send(
            &mut ledger,
            &actor("observer"),
            &actor("parent"),
            "warning",
            "warning",
            "request",
            None,
            200,
        )
        .unwrap();
        ledger.letters[0].watch_warning = Some(crate::delivery::watch::WarningReceipt {
            target: actor("child"),
            activity_at_unix_ms: 100,
            ordinal: 1,
            parent_notified: false,
        });
        assert!(of(&child, Some(&ledger), &BTreeMap::new()).is_none());
        ledger.letters[0]
            .watch_warning
            .as_mut()
            .unwrap()
            .parent_notified = true;
        let raised = of(&child, Some(&ledger), &BTreeMap::new()).unwrap();
        assert_eq!(raised.cause, Cause::ObserverUnconfirmed);
        assert!(
            raised.human_notice,
            "the existing watch notice owns the push"
        );
        ledger.watches[0].last_activity_at_unix_ms = 300;
        assert!(
            of(&child, Some(&ledger), &BTreeMap::new()).is_none(),
            "new activity ends the episode"
        );
        ledger.watches[0].last_activity_at_unix_ms = 100;
        ledger.watches.clear();
        assert!(
            of(&child, Some(&ledger), &BTreeMap::new()).is_none(),
            "cancelling the watch clears the raise"
        );
    }

    #[test]
    fn session_child_escalates_only_for_a_current_parent_hold_or_undelivered_letter() {
        let mut child = child();
        let actor = |pane: &str| Actor {
            pane_id: pane.into(),
            name: pane.into(),
            kind: "codex".into(),
            device_id: "local".into(),
            session: Some(format!("{pane}-session")),
        };
        let mut ledger = Ledger::default();
        mailbox::send(
            &mut ledger,
            &actor("child"),
            &actor("parent"),
            "request",
            "question",
            "request",
            None,
            100,
        )
        .unwrap();
        let id = ledger.letters[0].id.clone();
        assert!(of(&child, Some(&ledger), &BTreeMap::new()).is_none());
        for (hold, cause) in [
            (Hold::Blocked, Cause::ParentBlocked),
            (Hold::AwaitingOperator, Cause::ParentBlocked),
            (Hold::Draft, Cause::Draft),
            (Hold::Exhausted, Cause::BellExhausted),
        ] {
            let holds = BTreeMap::from([(id.clone(), hold)]);
            assert_eq!(of(&child, Some(&ledger), &holds).unwrap().cause, cause);
        }
        ledger.letters[0].state = State::Undelivered;
        assert!(
            of(&child, Some(&ledger), &BTreeMap::new())
                .unwrap()
                .human_notice
        );
        child.activity = "working".into();
        assert!(of(&child, Some(&ledger), &BTreeMap::new()).is_none());
        child.activity = "stopped".into();
        ledger.letters[0].recipient.session = Some("replacement-parent".into());
        assert!(of(&child, Some(&ledger), &BTreeMap::new()).is_none());
        child.blocked = true;
        assert_eq!(
            of(&child, None, &BTreeMap::new()).unwrap().cause,
            Cause::ChildBlocked
        );
    }

    #[test]
    fn a_reply_is_awaited_until_answered_overdue_or_from_another_session() {
        let mut ledger = Ledger::default();
        mailbox::send(
            &mut ledger,
            &actor("child"),
            &actor("parent"),
            "request",
            "question",
            "request",
            None,
            100,
        )
        .unwrap();
        let id = ledger.letters[0].id.clone();
        let session = Some("child-session");
        let awaited = |ledger: &Ledger, pane: &str, session: Option<&str>, now: u64| {
            ledger
                .awaited_answers(pane, session, now)
                .map(|letter| (letter.recipient.pane_id.clone(), letter.created_at_unix_ms))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            awaited(&ledger, "child", session, 200),
            vec![("parent".to_owned(), 100)],
            "the wait names who will answer"
        );
        assert!(awaited(&ledger, "child", Some("older"), 200).is_empty());
        assert!(awaited(&ledger, "child", None, 200).is_empty());
        assert!(awaited(&ledger, "parent", Some("parent-session"), 200).is_empty());
        let late = 100 + crate::delivery::ANSWER_WAIT_MS;
        assert!(awaited(&ledger, "child", session, late - 1).len() == 1);
        assert!(awaited(&ledger, "child", session, late).is_empty());
        mailbox::apply(
            &mut ledger,
            &actor("parent"),
            None,
            &crate::delivery::Command::Reply {
                id,
                intent: "answer".into(),
                body: "reply".into(),
            },
            300,
        )
        .unwrap();
        assert!(awaited(&ledger, "child", session, 400).is_empty());
    }
}
