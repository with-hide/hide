//! What a phone sees of the pull requests (PRD mobile-companion-ui B37-B45,
//! D-12, D-13): the rows the desktop's PR list draws unfolded, with the group
//! and reason the core's board decided (`work/board.rs`), the card the
//! desktop's `pr_card_read` filled for each, and the one feedback read a
//! hand-over asks for. It carries no local path, no body and no thread text:
//! only the structs below reach the wire.
//!
//! Which of the phone's four groups a row stands in, its line, and why Merge is
//! off are the web's own rules (`web/src/mobile/prModel.ts`,
//! `prCardModel.ts`), run on the phone over this value; nothing here decides
//! them.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AgentKey, Projection};
use crate::model::{GithubStatusSnapshot, PrFailedCheck, PullRequestSnapshot};

/// How many open pull requests, and how many just-merged ones, one frame
/// carries, in the order the board lists them; the rest are counted in
/// `omitted` and logged (engineering #15).
pub const MAX_PRS: usize = 48;
pub const MAX_SETTLED: usize = 8;
/// The most changed files one card sends; `files_total` still says how many
/// the pull request changes.
pub const MAX_FILES: usize = 50;
/// The most failed checks and change requests one feedback sends.
const FEEDBACK_ITEMS: usize = 20;
/// The longest change request kept, in characters. A prompt built from it is
/// refused by hided when it passes the phone's own limit, never cut.
const REQUEST_CHARS: usize = 2000;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PhonePrs {
    pub projects: Vec<PhoneProject>,
    pub items: Vec<PhonePr>,
    /// The one feedback read the core holds, which the phone matches to its own
    /// request id; none of the cards.
    pub feedback: Option<PhoneFeedback>,
    /// Pull requests past the caps, which no frame carries.
    #[serde(skip_serializing_if = "is_zero")]
    pub omitted: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

/// One project that has pull requests to list, with how its GitHub reads stand.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PhoneProject {
    pub id: String,
    pub label: String,
    pub device_id: String,
    /// The device's name for a project away from the paired Mac; `None` on the Mac.
    pub device_label: Option<String>,
    /// The checkouts' GitHub statuses, equal ones once: the phone's reading
    /// rule (`prReading`) runs over them.
    pub github: Vec<GithubStatusSnapshot>,
}

/// The second line's start the core decided (`board::Reason`), kind and values.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PhoneReason {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PhonePr {
    /// `<project id>#<number>`: a number alone repeats across projects.
    pub key: String,
    pub project: String,
    /// The list's own value, as the desktop reads it (`PullRequestSnapshot`'s wire form).
    #[serde(flatten)]
    pub pr: PullRequestSnapshot,
    /// The board's group and reason for it.
    pub group: String,
    pub reason: PhoneReason,
    /// The pull request's live agents on rows the phone lists, most urgent first.
    pub agents: Vec<AgentKey>,
    /// A checkout of its branch exists on its machine: the only place a
    /// hand-over from a phone starts, since a new worktree would move the
    /// desktop's screen.
    pub checkout: bool,
    pub card: Option<PhoneCard>,
}

/// What a card read beyond the list, without the body and the threads.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PhoneCard {
    #[serde(default)]
    pub reading: bool,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub files: Option<Vec<PhoneFile>>,
    #[serde(default)]
    pub files_total: u32,
    #[serde(default)]
    pub merge: Option<PhoneMerge>,
    #[serde(default)]
    pub failed: Vec<String>,
    #[serde(default)]
    pub write: Option<PhoneWrite>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PhoneFile {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
    #[serde(default)]
    pub generated: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PhoneMerge {
    pub methods: Vec<String>,
    pub queue: bool,
    pub can_merge: bool,
}

/// The Merge or branch update asked for from the card, and how it went.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PhoneWrite {
    pub action: String,
    pub head: String,
    #[serde(default)]
    pub method: Option<String>,
    pub phase: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// A feedback read's answer: the failed checks and the change requests a
/// hand-over's prompt names (`delegatePrompt`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PhoneFeedback {
    pub request_id: String,
    pub pr_number: u32,
    pub phase: String,
    pub failed_checks: Vec<PrFailedCheck>,
    pub change_requests: Vec<PhoneChangeRequest>,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PhoneChangeRequest {
    #[serde(default)]
    pub author: Option<String>,
    pub body: String,
}

#[derive(Deserialize)]
struct FeedbackIn {
    request_id: String,
    pr_number: u32,
    phase: String,
    #[serde(default)]
    failed_checks: Vec<PrFailedCheck>,
    #[serde(default)]
    change_requests: Vec<PhoneChangeRequest>,
    #[serde(default)]
    failure: Option<String>,
}

#[derive(Deserialize)]
struct BoardIn {
    #[serde(default)]
    rows: Vec<RowIn>,
    #[serde(default)]
    groups: Vec<GroupIn>,
}

#[derive(Deserialize)]
struct GroupIn {
    numbers: Vec<u32>,
}

#[derive(Deserialize)]
struct RowIn {
    number: u32,
    group: String,
    #[serde(default)]
    fold: Option<String>,
    reason: PhoneReason,
    #[serde(default)]
    agents: Vec<RowRefIn>,
    #[serde(default)]
    checkout_id: Option<String>,
}

#[derive(Deserialize)]
struct RowRefIn {
    pane_id: String,
}

/// One project of `navigator.workspaces` or a device's session, as far as the
/// phone reads it.
#[derive(Deserialize)]
struct WorkspaceIn {
    id: String,
    label: String,
    device_id: String,
    #[serde(default)]
    is_git: bool,
    #[serde(default)]
    is_home: bool,
    #[serde(default)]
    pull_requests: Vec<Value>,
    #[serde(default)]
    checkouts: Vec<CheckoutIn>,
    agent_scope: ScopeIn,
}

#[derive(Deserialize)]
struct ScopeIn {
    prs: BoardIn,
}

#[derive(Deserialize)]
struct CheckoutIn {
    #[serde(default)]
    github: Option<GithubStatusSnapshot>,
}

impl PhonePrs {
    /// The phone's pull requests from the same `rest` the agent list is built
    /// from, with `agents` the list built from it, so a row opens only an
    /// agent the phone can open.
    pub fn project(rest: &Value, node: &str, agents: &Projection) -> Self {
        let labels = device_labels(rest);
        let mut open = Vec::new();
        let mut settled = Vec::new();
        let mut projects = Vec::new();
        let cards = rest
            .pointer("/pr_work/cards")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for workspace in workspaces(rest) {
            let Some(workspace) = read::<WorkspaceIn>(workspace, "pr.workspace_invalid") else {
                continue;
            };
            if workspace.is_home || !has_pull_requests(&workspace) {
                continue;
            }
            projects.push(PhoneProject {
                id: workspace.id.clone(),
                label: cut(&workspace.label),
                device_id: workspace.device_id.clone(),
                device_label: (workspace.device_id != node).then(|| {
                    labels
                        .get(workspace.device_id.as_str())
                        .map_or_else(|| workspace.device_id.clone(), |label| (*label).to_owned())
                }),
                github: statuses(&workspace),
            });
            let mut pulls: HashMap<u32, PullRequestSnapshot> = HashMap::new();
            for value in &workspace.pull_requests {
                if let Some(pr) = read::<PullRequestSnapshot>(value, "pr.pull_request_invalid") {
                    pulls.insert(pr.number, pr);
                }
            }
            let rows: HashMap<u32, &RowIn> = workspace
                .agent_scope
                .prs
                .rows
                .iter()
                .map(|row| (row.number, row))
                .collect();
            // The board's own order: its groups, each in its order.
            for number in workspace
                .agent_scope
                .prs
                .groups
                .iter()
                .flat_map(|group| &group.numbers)
            {
                let (Some(row), Some(pr)) = (rows.get(number), pulls.remove(number)) else {
                    continue;
                };
                // A row the desktop folds (a team's review ask, nothing moved
                // for a week) is not counted there and is not listed here.
                if row.fold.is_some() {
                    continue;
                }
                let phone = phone_pr(&workspace, row, pr, cards, agents);
                if phone.reason.kind == "merged" {
                    settled.push(phone);
                } else {
                    open.push(phone);
                }
            }
        }
        settled.sort_by_key(|pr| std::cmp::Reverse(pr.pr.merged_at_unix_ms));
        let omitted = open.len().saturating_sub(MAX_PRS) + settled.len().saturating_sub(MAX_SETTLED);
        open.truncate(MAX_PRS);
        settled.truncate(MAX_SETTLED);
        let feedback = rest
            .pointer("/pr_work/feedback")
            .filter(|value| value.is_object())
            .and_then(|value| read::<FeedbackIn>(value, "pr.feedback_invalid"))
            .map(phone_feedback);
        Self {
            projects,
            items: open.into_iter().chain(settled).collect(),
            feedback,
            omitted,
        }
    }

    pub fn find(&self, key: &str) -> Option<&PhonePr> {
        self.items.iter().find(|pr| pr.key == key)
    }
}

/// `hasPullRequests` of the desktop's PR list: a Git project whose remote is GitHub.
fn has_pull_requests(workspace: &WorkspaceIn) -> bool {
    workspace.is_git
        && !workspace.checkouts.iter().any(|checkout| {
            checkout
                .github
                .as_ref()
                .is_some_and(|github| github.failure_category == Some(hide_node_link::gh::GithubFailureCategory::NoGithubRemote))
        })
}

fn statuses(workspace: &WorkspaceIn) -> Vec<GithubStatusSnapshot> {
    let mut all: Vec<GithubStatusSnapshot> = Vec::new();
    for status in workspace
        .checkouts
        .iter()
        .filter_map(|checkout| checkout.github.clone())
    {
        if !all.contains(&status) {
            all.push(status);
        }
    }
    all
}

fn phone_pr(
    workspace: &WorkspaceIn,
    row: &RowIn,
    pr: PullRequestSnapshot,
    cards: &[Value],
    agents: &Projection,
) -> PhonePr {
    let listed = |pane: &str| {
        agents
            .agents()
            .find(|agent| agent.pane_id == pane)
            .map(|agent| agent.key())
    };
    let card = cards
        .iter()
        .find(|card| {
            card.get("workspace_id").and_then(Value::as_str) == Some(workspace.id.as_str())
                && card.get("number").and_then(Value::as_u64) == Some(u64::from(pr.number))
        })
        .and_then(|card| read::<PhoneCard>(card, "pr.card_invalid"))
        .map(|mut card| {
            if let Some(files) = card.files.as_mut() {
                files.truncate(MAX_FILES);
            }
            card
        });
    let mut pr = pr;
    pr.title = cut(&pr.title);
    PhonePr {
        key: format!("{}#{}", workspace.id, pr.number),
        project: workspace.id.clone(),
        group: row.group.clone(),
        reason: row.reason.clone(),
        agents: row.agents.iter().filter_map(|row| listed(&row.pane_id)).collect(),
        checkout: row.checkout_id.is_some(),
        card,
        pr,
    }
}

fn phone_feedback(feedback: FeedbackIn) -> PhoneFeedback {
    PhoneFeedback {
        request_id: feedback.request_id,
        pr_number: feedback.pr_number,
        phase: feedback.phase,
        failed_checks: feedback
            .failed_checks
            .into_iter()
            .take(FEEDBACK_ITEMS)
            .collect(),
        change_requests: feedback
            .change_requests
            .into_iter()
            .take(FEEDBACK_ITEMS)
            .map(|request| PhoneChangeRequest {
                author: request.author,
                body: request.body.chars().take(REQUEST_CHARS).collect(),
            })
            .collect(),
        failure: feedback.failure,
    }
}

/// This Mac's projects and each connected device's, as the desktop's
/// `boardProjects` gathers them.
fn workspaces(rest: &Value) -> impl Iterator<Item = &Value> {
    let local = rest
        .pointer("/navigator/workspaces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let remote = rest
        .pointer("/status/remote")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|status| status.pointer("/session/workspaces")?.as_array())
        .flatten();
    local.chain(remote)
}

fn read<T: for<'de> Deserialize<'de>>(value: &Value, kind: &str) -> Option<T> {
    match T::deserialize(value) {
        Ok(read) => Some(read),
        Err(error) => {
            crate::diagnostic!(serde_json::json!({
                "component": "mobile_phone", "kind": kind, "message": error.to_string(),
            }));
            None
        }
    }
}

fn cut(text: &str) -> String {
    text.chars().take(200).collect()
}

/// A device's name by id, as the desktop's device list gives it.
fn device_labels(rest: &Value) -> HashMap<&str, &str> {
    rest.pointer("/navigator/devices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|device| {
            let label = device.get("label").and_then(Value::as_str)?;
            (!label.is_empty()).then_some((device.get("id")?.as_str()?, label))
        })
        .collect()
}
