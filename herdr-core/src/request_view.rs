//! The request view's row (PRD overview-request-view): what the operator
//! asked each agent, what came of it, and what is theirs to do now.
//!
//! Everything here is derived from facts the core already holds, with no AI:
//! the session's own words the label worker read (`labels::facts`), Herdr's
//! state and Hide's read and demand axes on the row, the lineage, and the
//! pull requests GitHub reported. The verb is computed in `agent_state::turn`;
//! association and duty belong to `agent_state::work`. This module assembles the block.
//!
//! A row's pull requests are those of its checkout's branch and those its
//! session made (D-31, D-46). A pull request on several rows gives its duty
//! (fix, review) to one of them: on that branch's checkout the row nearest
//! its lineage root, else the row whose session printed it first.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::agent_state::turn::{RequestVerb, verb_of};
use crate::agent_state::work::{Linked, assign_duty, linked_pull_requests, shown_pull_requests};
use crate::display_text;
use crate::issues::IssueReference;
use crate::labels::analysis::LabelEnd;
use crate::labels::facts::{Reply, Request, Requester};
use crate::model::{
    GithubSnapshot, PullRequestBadge, PullRequestChecks, ReviewDecision, SidebarAgentSnapshot,
};

/// What the label worker's facts give a row, laid on by the overlay only
/// while the pane's reference proves the session they were read from.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RowFacts {
    /// The current reported locator whose native owner the overlay proved.
    pub(crate) native_reference: Option<crate::sidebar::SessionAgentSessionPayload>,
    /// Available only from a current reference-proven native read.
    pub(crate) native_session_id: Option<String>,
    pub(crate) native_title: Option<String>,
    /// A title whose native record explicitly identifies a manual rename.
    pub(crate) native_custom_title: Option<String>,
    pub(crate) operator_request: Option<Request>,
    pub(crate) other_request: Option<Request>,
    pub(crate) reply: Option<Reply>,
    /// `(lowercase owner/name, number, when its tool printed it)`.
    pub(crate) created_prs: Vec<(String, u64, u64)>,
    /// Every pull request address the session printed, newest twenty, as
    /// `(lowercase owner/name, number)`: a turn that just printed one is
    /// held until GitHub's read of it lands (status-marks D-09).
    pub(crate) sighted_prs: Vec<(String, u64)>,
    /// How the label analysis read the turn's end and its line; `None`
    /// without one (summaries off, a failed analysis, a turn not judged
    /// yet).
    pub(crate) end: Option<LabelEnd>,
    pub(crate) line: Option<String>,
    /// The session's last turn proposed a plan and waits for the operator to
    /// approve it, as read for the agent's current Herdr state (PRD
    /// codex-plan-approval-hold D-05).
    pub(crate) awaiting_operator: bool,
    /// A native unanswered question or plan, proven for this current session
    /// and Herdr state. Absence conveys no invented question text.
    pub(crate) user_turn: Option<hide_session::turns::UserTurnFact>,
    /// Native asynchronous executions, proven for this session and Herdr
    /// state. Unknown, lost and stale reads never prove a running task.
    pub(crate) executions:
        hide_agent_adapter::work::WorkRead<hide_agent_adapter::work::WorkRecords>,
    /// The turn that just ended in the agent's current Herdr state has not
    /// had its session read yet, so whether a background task will wake the
    /// agent is not known. The row is not yet a stop (#902).
    pub(crate) turn_end_unread: bool,
    /// The devices the agent waited for died with their process and nothing
    /// has begun since (B16).
    pub(crate) wake_vanished: bool,
}

/// Who sent the request a row shows.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum RequestSender {
    Operator,
    /// A letter sender, or the delegated child's parent by its title.
    Named(String),
    /// Something other than Hide's input wrote it.
    Agent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RequestLineSnapshot {
    pub text: String,
    /// The text was longer than the core keeps.
    pub cut: bool,
    pub images: u32,
    pub at_unix_ms: u64,
    pub sender: RequestSender,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReplySnapshot {
    pub text: String,
    pub cut: bool,
    pub at_unix_ms: u64,
}

/// One of a row's pull requests.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentPullRequestSnapshot {
    pub number: u32,
    pub title: String,
    pub url: String,
    pub badge: PullRequestBadge,
    pub checks: PullRequestChecks,
    pub review: Option<ReviewDecision>,
    /// Read by [`Self::state`] in the core; the shell draws the state.
    #[serde(skip_serializing)]
    pub is_draft: bool,
    pub head_branch: String,
    pub closing_issues: Vec<IssueReference>,
    /// Drawn as the row's chip or counted in its `+N` (D-43): open, or
    /// settled after the operator's last request.
    pub live: bool,
    /// This row holds the pull request's duty (D-31).
    pub duty: bool,
    /// The row's session made it (D-31), as opposed to its branch having it.
    pub created: bool,
    pub settled_at_unix_ms: Option<u64>,
}

impl AgentPullRequestSnapshot {
    /// Its one state ([`crate::model::PrState::of`]).
    pub fn state(&self) -> crate::model::PrState {
        crate::model::PrState::of(self.badge, self.is_draft, self.checks, self.review)
    }
}

/// The request view's part of an agent row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentRequestSnapshot {
    pub verb: RequestVerb,
    /// When the row took this verb; kept across a restart (D-40).
    pub verb_since_unix_ms: u64,
    /// The label's line for the turn (B18, B47); absent without one, when
    /// the row shows its reply instead (D-12).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// How the label read the turn's end, shown with the line when the row
    /// is expanded (D-28); absent without a label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<LabelEnd>,
    pub request: Option<RequestLineSnapshot>,
    /// Who sent a request after the operator's last one (B4).
    pub later_by: Option<RequestSender>,
    pub reply: Option<ReplySnapshot>,
    /// The chip first (D-46), then the other live ones, then settled ones.
    pub pull_requests: Vec<AgentPullRequestSnapshot>,
}

/// A row's verb and when it took it, kept in `core-state.json`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VerbRecord {
    pub verb: RequestVerb,
    pub since_unix_ms: u64,
    /// When the operator last opened the row's result (D-29): a pull
    /// request that settled before then is a result already seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_opened_unix_ms: Option<u64>,
}

/// Where a row lives, for its branch's pull requests.
pub(crate) struct RowPlace<'a> {
    pub(crate) branch: Option<&'a str>,
    /// The commit the checkout is on, which ties a settled pull request to it.
    pub(crate) head_sha: Option<&'a str>,
    pub(crate) root_path: &'a str,
}

/// Lays the request block on every row. `place` answers a local row's
/// checkout; a device's rows have none (their pull requests are not read).
/// `verbs` keeps each pane's verb and since; returns whether it changed.
pub(crate) fn apply<'a>(
    rows: &mut [SidebarAgentSnapshot],
    place: impl Fn(&str) -> Option<RowPlace<'a>>,
    github: &GithubSnapshot,
    verbs: &mut BTreeMap<String, VerbRecord>,
    now_unix_ms: u64,
) -> bool {
    let names: HashMap<String, String> = rows
        .iter()
        .map(|row| (row.pane_id.clone(), row.identity_label.clone()))
        .collect();
    let mut linked: Vec<Vec<Linked<'_>>> = rows
        .iter()
        .map(|row| linked_pull_requests(row, place(&row.pane_id), github))
        .collect();
    assign_duty(rows, &mut linked);
    let mut verbs_changed = false;
    for (row, linked) in rows.iter_mut().zip(linked) {
        let facts = row.row_facts.clone().unwrap_or_default();
        let parent = row
            .lineage_parent_pane_id
            .as_ref()
            .and_then(|parent| names.get(parent));
        let resolve = |request: &Request| sender(request, row.delegated, parent);
        let operator = facts
            .operator_request
            .as_ref()
            .map(|request| (request, resolve(request)));
        let other = facts
            .other_request
            .as_ref()
            .map(|request| (request, resolve(request)));
        let operator_at = operator
            .as_ref()
            .filter(|(_, sender)| *sender == RequestSender::Operator)
            .map(|(request, _)| request.at_unix_ms);
        let (shown, later_by) = match (operator, other) {
            (Some((request, RequestSender::Operator)), other) => (
                Some((request, RequestSender::Operator)),
                other
                    .filter(|(later, _)| later.at_unix_ms > request.at_unix_ms)
                    .map(|(_, sender)| sender),
            ),
            (one, two) => (
                [one, two]
                    .into_iter()
                    .flatten()
                    .max_by_key(|(request, _)| request.at_unix_ms),
                None,
            ),
        };
        let pull_requests = shown_pull_requests(linked, operator_at);
        let opened = verbs
            .get(&row.pane_id)
            .and_then(|record| record.result_opened_unix_ms);
        let verb = verb_of(row, &pull_requests, opened);
        let record = verbs.get(&row.pane_id).filter(|record| record.verb == verb);
        let since = match record {
            Some(record) => record.since_unix_ms,
            None => {
                verbs.insert(
                    row.pane_id.clone(),
                    VerbRecord {
                        verb,
                        since_unix_ms: now_unix_ms,
                        result_opened_unix_ms: opened,
                    },
                );
                verbs_changed = true;
                now_unix_ms
            }
        };
        row.request = Some(AgentRequestSnapshot {
            verb,
            verb_since_unix_ms: since,
            line: facts
                .line
                .as_deref()
                .and_then(|line| display_text::one_line(line, MAX_LINE_CHARS)),
            end: facts.end,
            request: shown.map(|(request, sender)| RequestLineSnapshot {
                text: display_text::block(&request.text),
                cut: request.cut,
                images: request.images,
                at_unix_ms: request.at_unix_ms,
                sender,
            }),
            later_by,
            reply: facts.reply.as_ref().map(|reply| ReplySnapshot {
                text: display_text::block(&reply.text),
                cut: reply.cut,
                at_unix_ms: reply.at_unix_ms,
            }),
            pull_requests,
        });
        row.state = crate::agent_state::row_state(row);
    }
    verbs_changed
}

/// Drops the verb records of the panes `gone` names. The caller names
/// only panes an authoritative topology no longer has, so a list that is
/// empty for a moment (a Herdr reconnect, before the first session) never
/// restarts every wait. Returns whether any went.
pub(crate) fn prune_verbs(
    verbs: &mut BTreeMap<String, VerbRecord>,
    gone: impl Fn(&str) -> bool,
) -> bool {
    let before = verbs.len();
    verbs.retain(|pane, _| !gone(pane));
    verbs.len() != before
}

/// Marks the row's result opened now, when its verb is `result`; a settled
/// pull request it showed is then seen. Returns whether it was marked.
pub(crate) fn open_result(
    verbs: &mut BTreeMap<String, VerbRecord>,
    pane_id: &str,
    now_unix_ms: u64,
) -> bool {
    match verbs.get_mut(pane_id) {
        Some(record) if record.verb == RequestVerb::Result => {
            record.result_opened_unix_ms = Some(now_unix_ms);
            true
        }
        _ => false,
    }
}

fn sender(request: &Request, delegated: bool, parent: Option<&String>) -> RequestSender {
    let parents_first = request.first && delegated;
    match &request.requester {
        Requester::Operator => RequestSender::Operator,
        Requester::Unobserved if parents_first => {
            parent.map_or(RequestSender::Agent, |name| named(name))
        }
        Requester::Unobserved => RequestSender::Operator,
        Requester::Named(name) => named(name),
        Requester::Agent if parents_first => {
            parent.map_or(RequestSender::Agent, |name| named(name))
        }
        Requester::Agent => RequestSender::Agent,
    }
}

/// The longest sender name a row shows; participant names are short handles.
const MAX_SENDER_CHARS: usize = 64;
/// The longest label line a row carries; the label keeps its own to 40.
const MAX_LINE_CHARS: usize = 200;
/// What the row writes for the operator and for an unnamed agent, so no
/// sender can pass as either (a participant name is whatever its sender chose).
const RESERVED_SENDERS: [&str; 3] = ["나", "에이전트", "operator"];

fn named(name: &str) -> RequestSender {
    match display_text::one_line(name, MAX_SENDER_CHARS) {
        Some(name)
            if !RESERVED_SENDERS.iter().any(|reserved| {
                display_text::skeleton(&name) == display_text::skeleton(reserved)
            }) =>
        {
            RequestSender::Named(name)
        }
        _ => RequestSender::Agent,
    }
}

#[cfg(test)]
mod tests;
