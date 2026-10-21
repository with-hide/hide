//! Association of a session with PRs and issues, and the one holder of each duty.
pub(crate) mod board;
use crate::model::{GithubSnapshot, PullRequestChecks, PullRequestSnapshot, SidebarAgentSnapshot};
use crate::request_view::{AgentPullRequestSnapshot, RowPlace};
use std::collections::HashMap;

pub(crate) struct Linked<'a> {
    pull_request: &'a PullRequestSnapshot,
    on_branch: bool,
    /// When the session's tool printed it, for a pull request it made.
    sighted_at: Option<u64>,
    duty: bool,
    provisional: bool,
}

impl<'a> Linked<'a> {
    pub(crate) fn pull_request(&self) -> &'a PullRequestSnapshot {
        self.pull_request
    }

    pub(crate) fn duty(&self) -> bool {
        self.duty
    }
}

pub(crate) fn linked_pull_requests<'a>(
    row: &SidebarAgentSnapshot,
    place: Option<RowPlace<'_>>,
    github: &'a GithubSnapshot,
) -> Vec<Linked<'a>> {
    let Some(place) = place else {
        return Vec::new();
    };
    let mut linked: Vec<Linked<'a>> = Vec::new();
    if let Some(project) = github.project(place.root_path) {
        linked.extend(
            crate::github::pull_request_for_checkout(
                &project.pull_requests,
                place.branch,
                place.head_sha,
            )
            .map(|pull_request| Linked {
                pull_request,
                on_branch: true,
                sighted_at: None,
                duty: false,
                provisional: false,
            }),
        );
    }
    let created = row
        .row_facts
        .as_ref()
        .map(|facts| facts.created_prs.as_slice())
        .unwrap_or_default();
    let saved = row
        .record_metadata
        .as_ref()
        .map(|m| m.created_prs.as_slice())
        .unwrap_or_default();
    let provisional =
        row.record_connection != Some(crate::labels::record::RecordConnection::Confirmed);
    for ((repository, number, sighted_at), provisional) in created
        .iter()
        .map(|pr| (pr, false))
        .chain(saved.iter().map(|pr| (pr, provisional)))
    {
        let found = github
            .projects
            .iter()
            .flat_map(|project| project.pull_requests.iter())
            .find(|pull_request| {
                u64::from(pull_request.number) == *number
                    && hide_session::pull_request_addresses(&pull_request.url)
                        .first()
                        .is_some_and(|(repo, _)| repo.eq_ignore_ascii_case(repository))
            });
        let Some(pull_request) = found else {
            continue;
        };
        match linked
            .iter_mut()
            .find(|known| known.pull_request.url == pull_request.url)
        {
            Some(known) => {
                if !provisional {
                    known.sighted_at = Some(*sighted_at);
                    known.provisional = false;
                }
            }
            None => linked.push(Linked {
                pull_request,
                on_branch: false,
                sighted_at: Some(*sighted_at),
                duty: false,
                provisional,
            }),
        }
    }
    linked
}

/// Gives each pull request's duty to one row. On its branch's checkout that
/// is the row nearest its lineage root, then the one whose session printed
/// it, then the lowest pane id; with no row there, the one whose session
/// printed it first. `linked[i]` belongs to `rows[i]`. The rows' order never
/// decides: it follows activity, and the duty would move between the agents
/// of one checkout as they take turns working.
pub(crate) fn assign_duty(rows: &[SidebarAgentSnapshot], linked: &mut [Vec<Linked<'_>>]) {
    /// Off the checkout, lineage depth, when its session printed it, pane id:
    /// the smallest holds the duty.
    type Rank<'r> = (bool, usize, u64, &'r str);
    let mut holder: HashMap<&str, (Rank<'_>, usize, usize)> = HashMap::new();
    for (row, links) in linked.iter().enumerate() {
        let agent = &rows[row];
        for (index, link) in links.iter().enumerate() {
            if link.provisional {
                continue;
            }
            let rank = (
                !link.on_branch,
                if link.on_branch {
                    agent.lineage_depth
                } else {
                    0
                },
                link.sighted_at.unwrap_or(u64::MAX),
                agent.pane_id.as_str(),
            );
            let url = link.pull_request.url.as_str();
            if holder.get(url).is_none_or(|(held, _, _)| rank < *held) {
                holder.insert(url, (rank, row, index));
            }
        }
    }
    for (_, row, index) in holder.into_values() {
        linked[row][index].duty = true;
    }
}

fn settled_at(pull_request: &PullRequestSnapshot) -> Option<u64> {
    pull_request
        .merged_at_unix_ms
        .or(pull_request.closed_at_unix_ms)
}

fn is_open(pull_request: &PullRequestSnapshot) -> bool {
    !pull_request.badge.is_settled()
}

/// The order a row's live pull requests are looked at in (D-46).
fn urgency(pull_request: &AgentPullRequestSnapshot) -> u8 {
    match (pull_request.badge.is_settled(), pull_request.checks) {
        (false, PullRequestChecks::Failed) => 0,
        (false, PullRequestChecks::Passing | PullRequestChecks::None) => 1,
        (false, PullRequestChecks::Pending) => 2,
        _ => 3,
    }
}

pub(crate) fn shown_pull_requests(
    linked: Vec<Linked<'_>>,
    operator_at: Option<u64>,
) -> Vec<AgentPullRequestSnapshot> {
    let mut shown: Vec<(AgentPullRequestSnapshot, u64)> = linked
        .into_iter()
        .map(|link| {
            let pull_request = link.pull_request;
            let settled = settled_at(pull_request);
            let live = is_open(pull_request)
                || matches!((settled, operator_at), (Some(settled), Some(asked)) if settled > asked);
            let recency = pull_request
                .created_at_unix_ms
                .or(pull_request.updated_at_unix_ms)
                .unwrap_or(0);
            (
                AgentPullRequestSnapshot {
                    number: pull_request.number,
                    title: pull_request.title.clone(),
                    url: pull_request.url.clone(),
                    badge: pull_request.badge,
                    checks: pull_request.checks,
                    review: pull_request.review,
                    is_draft: pull_request.is_draft,
                    head_branch: pull_request.head_branch.clone(),
                    closing_issues: pull_request.closing_issues.clone(),
                    live,
                    duty: link.duty,
                    provisional: link.provisional,
                    created: link.sighted_at.is_some() && !link.provisional,
                    settled_at_unix_ms: settled,
                },
                recency,
            )
        })
        .collect();
    shown.sort_by(|(left, left_recency), (right, right_recency)| {
        right
            .live
            .cmp(&left.live)
            .then_with(|| urgency(left).cmp(&urgency(right)))
            .then_with(|| right_recency.cmp(left_recency))
            .then_with(|| right.number.cmp(&left.number))
    });
    shown
        .into_iter()
        .map(|(pull_request, _)| pull_request)
        .collect()
}
