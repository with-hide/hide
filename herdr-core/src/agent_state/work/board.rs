//! The PRs tool's list (PRD work-tab-pr-lens): which pull requests the
//! operator sees, the one group each stands in, and the one reason its second
//! line starts with.
//!
//! Every input is a GitHub value the 5-minute read carries or a fact only
//! Hide holds (the link record's sessions and the agents' own state); a
//! pull request body's headings and a repository's own scripts decide nothing
//! (D-05). The core computes group and reason, the web words them.
use crate::agent_state::tally::scope::{RowRef, row_references};
use crate::delivery::INACTIVITY_MS;
use crate::model::{
    ExtraQuery, MergeQueuePlace, MergeState, PullRequestBadge, PullRequestChecks,
    PullRequestSnapshot, ReviewAsk, ReviewDecision, SidebarAgentSnapshot, WorkspaceSnapshot,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// The groups in the order they are drawn (D-08, B7).
pub const GROUPS: [&str; 7] = [
    "needs_you",
    "working",
    "blocked",
    "waiting",
    "drafts",
    "bots",
    "merged",
];

/// A pull request nobody has touched for this long folds into its group's
/// `오래된 PR N` line (D-11).
pub const OLD_MS: u64 = 7 * 24 * 60 * 60 * 1000;

const LINE_LIMIT: usize = 160;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Board {
    pub rows: Vec<Row>,
    pub groups: Vec<Group>,
    /// Needs You rows that are not folded: the PRs tool's rail number
    /// (D-17, D-26; sidebar-task-lens B29).
    pub needs_you: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Group {
    pub group: &'static str,
    pub numbers: Vec<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Row {
    pub number: u32,
    pub group: &'static str,
    /// `team` (a review asked of a team) or `old` (D-11, D-15): drawn as a
    /// folded line inside the group and not counted.
    pub fold: Option<&'static str>,
    pub reason: Reason,
    /// The pull request's live agents, most urgent first (B21).
    pub agents: Vec<RowRef>,
    /// The pane of the agent a `reply` or `check` reason is about.
    pub agent_pane: Option<String>,
    /// The checkout holding the pull request's branch, which a merged row's
    /// `정리` acts on.
    pub checkout_id: Option<String>,
    /// Every checkout the pull request is the work of (D-19).
    pub checkout_ids: Vec<String>,
    pub issue: Option<Issue>,
}

/// The second line's start (D-16), as a kind the web words and the values it
/// words it with. Times are instants, never durations, so time passing
/// republishes nothing.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Reason {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_unix_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

impl Reason {
    fn of(kind: &'static str) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }
    fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
    fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }
    fn count(mut self, count: u32) -> Self {
        self.count = Some(count);
        self
    }
    fn since(mut self, at: Option<u64>) -> Self {
        self.since_unix_ms = at;
        self
    }
    fn base(mut self, base: &str) -> Self {
        self.base = Some(base.to_owned());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Issue {
    pub key: String,
    pub label: String,
    pub url: Option<String>,
    pub task_key: Option<String>,
}

/// What a live agent is doing on a pull request, most urgent first (D-08).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Stand {
    /// Waiting on a question, an approval or an error (`Needs You`).
    Asking,
    /// Working with no movement for the watch's first inactivity window (D-10).
    Stalled,
    Working,
}

fn stand(agent: &SidebarAgentSnapshot, now: u64) -> Option<Stand> {
    if agent.state.needs_you || agent.state.asking {
        Some(Stand::Asking)
    } else if agent.state.working {
        let quiet = agent
            .changed_at_unix_ms
            .is_some_and(|at| now.saturating_sub(at) >= INACTIVITY_MS);
        Some(if quiet {
            Stand::Stalled
        } else {
            Stand::Working
        })
    } else {
        // A finished, unread agent is Sessions' to show (D-12).
        None
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty());
    let line = line.unwrap_or_default();
    match line.char_indices().nth(LINE_LIMIT) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_owned(),
    }
}

/// What the agent asks, for a `reply` reason.
fn question(agent: &SidebarAgentSnapshot) -> String {
    let text = agent
        .expected_reply
        .as_deref()
        .or(agent.detail.as_deref())
        .or(agent.message.as_deref())
        .unwrap_or_default();
    first_line(text)
}

fn is_open(pr: &PullRequestSnapshot) -> bool {
    !pr.badge.is_settled()
}

/// Which pull requests the list shows (D-25): the operator's own, the ones a
/// session of theirs made or worked on, review asks made of them or their
/// team, and bots'. A merged one needs the first two. A teammate's pull
/// request none of these reach is not listed.
///
/// A pull request is left out only when every judgment says it is not the
/// operator's. While the account or the review asks are unknown (issue #1189)
/// one that may be theirs is listed, since leaving it out would hide a pull
/// request the operator may own.
pub(crate) fn listed(pr: &PullRequestSnapshot) -> bool {
    let work = &pr.work;
    let may_be_mine = work.mine != Some(false);
    match pr.badge {
        PullRequestBadge::Closed => false,
        PullRequestBadge::Merged => may_be_mine || !work.sessions.is_empty(),
        PullRequestBadge::Open | PullRequestBadge::Review => {
            may_be_mine
                || !work.sessions.is_empty()
                || work.asked_of_me != ReviewAsk::None
                || work.author_is_bot
        }
    }
}

struct Placed {
    group: &'static str,
    reason: Reason,
    fold: Option<&'static str>,
}

fn place(group: &'static str, reason: Reason) -> Placed {
    Placed {
        group,
        reason,
        fold: None,
    }
}

/// The failed checks that stop a merge. A check that is not required fails
/// the rollup too; GitHub says so with `UNSTABLE`, and that is `머지` (D-20).
fn blocking_failure(pr: &PullRequestSnapshot) -> bool {
    pr.checks == PullRequestChecks::Failed
        && !matches!(
            pr.work.merge_state,
            MergeState::Clean | MergeState::HasHooks | MergeState::Unstable
        )
}

fn failed_reason(pr: &PullRequestSnapshot) -> Reason {
    let failed = &pr.work.failed_checks;
    let reason = Reason::of("failed").count(failed.len().max(1) as u32);
    match failed.first().and_then(|check| check.name.clone()) {
        Some(name) => reason.name(name),
        None => reason,
    }
}

/// The reason a pull request carries when no agent or draft decides it: the
/// first of the six slots of D-08 from step 4 on that fits, or `None` when
/// GitHub has not computed the merge state yet.
fn standing(pr: &PullRequestSnapshot) -> Option<Placed> {
    let work = &pr.work;
    // Blocked is for work the operator or their sessions own: someone else's
    // pull request that was only asked of them is the review's, whatever its
    // checks say (D-08, D-25). Whether the operator wrote it is unknown while
    // the account is, and then the pull request is not judged at all.
    let owned = if !work.sessions.is_empty() || work.author_is_bot {
        true
    } else {
        match work.mine {
            Some(mine) => mine,
            None => return Some(unjudged(ExtraQuery::Account)),
        }
    };
    if owned && blocking_failure(pr) {
        return Some(place("blocked", failed_reason(pr)));
    }
    if owned && pr.review == Some(ReviewDecision::ChangesRequested) {
        return Some(place(
            "blocked",
            Reason::of("changes").since(pr.updated_at_unix_ms),
        ));
    }
    if owned && work.merge_state == MergeState::Dirty {
        return Some(place(
            "blocked",
            Reason::of("conflict").base(&pr.base_branch),
        ));
    }
    // A bot's pull request is nobody's but its own, so the account tells
    // nothing about it.
    let mine = match work.mine {
        Some(mine) => mine,
        None if work.author_is_bot => false,
        None => return Some(unjudged(ExtraQuery::Account)),
    };
    if mine {
        // A pull request in the merge queue is placed before this point, and
        // the states below are the ones a queued pull request can be in.
        if work.queue == MergeQueuePlace::Unknown
            && !matches!(
                work.merge_state,
                MergeState::Dirty | MergeState::Draft | MergeState::Unknown
            )
        {
            return Some(unjudged(ExtraQuery::MergeQueue));
        }
        match work.merge_state {
            MergeState::Clean | MergeState::HasHooks => {
                return Some(place("needs_you", Reason::of("merge")));
            }
            MergeState::Unstable => {
                let reason = Reason::of("merge");
                let reason = match work
                    .failed_checks
                    .first()
                    .and_then(|check| check.name.clone())
                {
                    Some(name) => reason.text(name),
                    None => reason,
                };
                return Some(place("needs_you", reason));
            }
            MergeState::Behind => {
                return Some(place(
                    "needs_you",
                    Reason::of("update").base(&pr.base_branch),
                ));
            }
            MergeState::Blocked => {
                if pr.checks == PullRequestChecks::Pending || work.pending_checks > 0 {
                    return Some(place(
                        "waiting",
                        Reason::of("ci").count(work.pending_checks.max(1)),
                    ));
                }
                if !work.review_requests.is_empty() {
                    return Some(place("waiting", waiting_review(pr)));
                }
                return Some(place("needs_you", Reason::of("assign_reviewer")));
            }
            MergeState::Dirty | MergeState::Draft | MergeState::Unknown => return None,
        }
    }
    // A pull request of someone else's: only the review asked of the
    // operator or their team moves it (D-15).
    match work.asked_of_me {
        ReviewAsk::Unknown => return Some(unjudged(ExtraQuery::ReviewRequests)),
        ReviewAsk::Direct => {
            return Some(place(
                "needs_you",
                Reason::of("review")
                    .name(work.author.clone().unwrap_or_default())
                    .since(work.review_requested_at_unix_ms.or(pr.created_at_unix_ms)),
            ));
        }
        ReviewAsk::Team => {
            let team = work
                .review_requests
                .iter()
                .find(|request| request.kind == crate::model::ReviewRequestKind::Team)
                .map(|request| request.name.clone());
            let mut reason = Reason::of("team_review")
                .since(work.review_requested_at_unix_ms.or(pr.created_at_unix_ms));
            if let Some(team) = team {
                reason = reason.name(team);
            }
            return Some(Placed {
                group: "needs_you",
                reason,
                fold: Some("team"),
            });
        }
        ReviewAsk::None => {}
    }
    if pr.checks == PullRequestChecks::Pending || work.pending_checks > 0 {
        return Some(place(
            "waiting",
            Reason::of("ci").count(work.pending_checks.max(1)),
        ));
    }
    Some(place("waiting", waiting_review(pr)))
}

/// A pull request a judgment it needs could not be made for: it waits in the
/// plain Waiting group, which says nothing about whose move it is, with the
/// question that was not answered (issue #1189). The group is never one it may
/// not belong to.
fn unjudged(query: ExtraQuery) -> Placed {
    let slug = match query {
        ExtraQuery::Account => "account",
        ExtraQuery::ReviewRequests => "review_requests",
        ExtraQuery::MergeQueue => "merge_queue",
    };
    place("waiting", Reason::of("unjudged").text(slug))
}

fn waiting_review(pr: &PullRequestSnapshot) -> Reason {
    let reviewers = pr
        .work
        .review_requests
        .iter()
        .map(|request| request.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let reason = Reason::of("waiting_review").since(
        pr.work
            .review_requested_at_unix_ms
            .or(pr.updated_at_unix_ms),
    );
    if reviewers.is_empty() {
        reason
    } else {
        reason.name(reviewers)
    }
}

/// The checkouts a pull request is the work of (D-19): the one on its branch,
/// the ones its recorded sessions ran in, and the ones its live agents sit in.
fn checkout_ids(
    workspace: &WorkspaceSnapshot,
    pr: &PullRequestSnapshot,
    agents: &[&SidebarAgentSnapshot],
) -> Vec<String> {
    workspace
        .checkouts
        .iter()
        .filter(|checkout| {
            crate::github::belongs_to_checkout(pr, checkout.branch.as_deref(), checkout.head_sha())
                || pr.work.checkouts.contains(&checkout.path)
                || checkout
                    .tabs
                    .iter()
                    .flat_map(|tab| &tab.panes)
                    .any(|pane| agents.iter().any(|agent| agent.pane_id == pane.id))
        })
        .map(|checkout| checkout.id.clone())
        .collect()
}

/// The board of one project. `previous` is where each pull request stood on
/// the last projection: a pull request GitHub has not computed a merge state
/// for keeps that place and reads `확인 중` (D-20, B12).
pub(crate) fn project(
    workspace: &WorkspaceSnapshot,
    agents: &[&SidebarAgentSnapshot],
    previous: &HashMap<u32, &'static str>,
    now: u64,
) -> Board {
    let tasks: HashMap<_, _> = workspace
        .tasks
        .tasks
        .iter()
        .map(|task| (task.key.as_str(), task))
        .collect();
    let references = row_references(agents);
    let mut board = Board::default();
    let mut seen = HashSet::new();
    for pr in workspace.pull_requests.iter().filter(|pr| listed(pr)) {
        if !seen.insert(pr.number) {
            continue;
        }
        let here: Vec<&SidebarAgentSnapshot> = {
            let mut here: Vec<_> = agents
                .iter()
                .copied()
                .filter(|agent| {
                    agent
                        .session_id
                        .as_ref()
                        .is_some_and(|id| pr.work.sessions.contains(id))
                })
                .filter(|agent| is_open(pr) && stand(agent, now).is_some())
                .collect();
            here.sort_by(|left, right| {
                stand(left, now)
                    .cmp(&stand(right, now))
                    .then_with(|| right.changed_at_unix_ms.cmp(&left.changed_at_unix_ms))
                    .then_with(|| left.pane_id.cmp(&right.pane_id))
            });
            here
        };
        let lead = here.first().copied();
        let lead_stand = lead.and_then(|agent| stand(agent, now));
        let placed = if !is_open(pr) {
            if pr.badge == PullRequestBadge::Merged {
                place("merged", Reason::of("merged").since(pr.merged_at_unix_ms))
            } else {
                continue;
            }
        } else if pr.work.author_is_bot {
            let inner = standing(pr).map_or_else(|| Reason::of("checking"), |p| p.reason);
            place("bots", inner)
        } else if let (Some(agent), Some(Stand::Asking)) = (lead, lead_stand) {
            place(
                "needs_you",
                Reason::of("reply")
                    .name(agent.identity_label.clone())
                    .text(question(agent)),
            )
        } else if let (Some(agent), Some(Stand::Stalled)) = (lead, lead_stand) {
            place(
                "needs_you",
                Reason::of("check")
                    .name(agent.identity_label.clone())
                    .since(agent.changed_at_unix_ms),
            )
        } else if let (Some(agent), Some(Stand::Working)) = (lead, lead_stand) {
            place(
                "working",
                Reason::of("working")
                    .name(agent.identity_label.clone())
                    .since(agent.changed_at_unix_ms),
            )
        } else if let MergeQueuePlace::At(position) = pr.work.queue {
            place("merged", Reason::of("queued").count(position))
        } else if pr.is_draft {
            place("drafts", Reason::of("draft").since(pr.created_at_unix_ms))
        } else {
            match standing(pr) {
                Some(placed) => placed,
                // GitHub has not computed the merge state: the pull request
                // keeps where it stood, or waits if it is new (B12).
                None => place(
                    previous.get(&pr.number).copied().unwrap_or("waiting"),
                    Reason::of("checking"),
                ),
            }
        };
        let old = is_open(pr)
            && placed.fold.is_none()
            && !matches!(placed.group, "merged")
            && pr
                .updated_at_unix_ms
                .is_some_and(|at| now.saturating_sub(at) >= OLD_MS);
        let fold = placed.fold.or(old.then_some("old"));
        let checkout = workspace.checkouts.iter().find(|checkout| {
            (checkout.is_worktree || checkout.exists)
                && checkout
                    .pull_request
                    .as_ref()
                    .is_some_and(|found| found.url == pr.url)
        });
        let linked = checkout
            .and_then(|checkout| checkout.task_key.as_deref())
            .and_then(|key| tasks.get(key));
        let issue = linked
            .map(|task| Issue {
                key: task.key.clone(),
                label: task.id.clone().unwrap_or_else(|| task.title.clone()),
                url: task.url.clone(),
                task_key: Some(task.key.clone()),
            })
            .or_else(|| {
                pr.closing_issues.first().map(|reference| {
                    let key = format!("github:{}#{}", reference.repository, reference.number);
                    let task = tasks.get(key.as_str());
                    let fallback = if workspace.home_issues.repository.as_deref()
                        == Some(reference.repository.as_str())
                    {
                        format!("#{}", reference.number)
                    } else {
                        format!("{}#{}", reference.repository, reference.number)
                    };
                    Issue {
                        key,
                        label: task.and_then(|t| t.id.clone()).unwrap_or(fallback),
                        url: Some(task.and_then(|t| t.url.clone()).unwrap_or_else(|| {
                            format!(
                                "https://github.com/{}/issues/{}",
                                reference.repository, reference.number
                            )
                        })),
                        task_key: task.map(|t| t.key.clone()),
                    }
                })
            });
        let agent_pane = matches!(placed.reason.kind, "reply" | "check")
            .then(|| lead.map(|agent| agent.pane_id.clone()))
            .flatten();
        board.rows.push(Row {
            number: pr.number,
            group: placed.group,
            fold,
            reason: placed.reason,
            agents: here
                .iter()
                .map(|agent| references[&(*agent as *const _)].clone())
                .collect(),
            agent_pane,
            checkout_id: checkout.map(|checkout| checkout.id.clone()),
            checkout_ids: checkout_ids(workspace, pr, agents),
            issue,
        });
    }
    board.needs_you = board
        .rows
        .iter()
        .filter(|row| row.group == "needs_you" && row.fold.is_none())
        .count();
    for group in GROUPS {
        let mut rows: Vec<(&Row, &PullRequestSnapshot)> = board
            .rows
            .iter()
            .filter(|row| row.group == group)
            .filter_map(|row| {
                workspace
                    .pull_requests
                    .iter()
                    .find(|pr| pr.number == row.number)
                    .map(|pr| (row, pr))
            })
            .collect();
        rows.sort_by_key(|(row, pr)| {
            (
                rank(row),
                std::cmp::Reverse(
                    if group == "merged" {
                        pr.merged_at_unix_ms
                    } else {
                        pr.updated_at_unix_ms
                    }
                    .unwrap_or(0),
                ),
                std::cmp::Reverse(pr.number),
            )
        });
        if !rows.is_empty() {
            board.groups.push(Group {
                group,
                numbers: rows.iter().map(|(row, _)| row.number).collect(),
            });
        }
    }
    board
}

/// Within a group: what is asked of the operator first; a pull request in the
/// merge queue tops Merging · merged (D-14).
fn rank(row: &Row) -> u8 {
    match row.reason.kind {
        "reply" => 0,
        "check" => 1,
        "review" => 2,
        "merge" => 3,
        "update" => 4,
        "assign_reviewer" => 5,
        "queued" => 0,
        _ => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PrFailedCheck, ReviewRequest, ReviewRequestKind};
    use crate::sidebar::{SessionSnapshotPayload, project_agents};
    use serde_json::json;

    const NOW: u64 = 10_000_000_000;

    fn pr(number: u32) -> PullRequestSnapshot {
        PullRequestSnapshot {
            closing_issues: Vec::new(),
            title: format!("PR {number}"),
            checks: PullRequestChecks::Passing,
            number,
            head_branch: format!("b{number}"),
            base_branch: "main".into(),
            url: format!("https://github.com/acme/app/pull/{number}"),
            badge: PullRequestBadge::Open,
            review: None,
            is_draft: false,
            merged_at_unix_ms: None,
            updated_at_unix_ms: Some(NOW - 1000),
            created_at_unix_ms: Some(NOW - 5000),
            closed_at_unix_ms: None,
            head_oid: None,
            cross_repository: false,
            work: crate::model::PullRequestWork {
                mine: Some(true),
                asked_of_me: ReviewAsk::None,
                queue: MergeQueuePlace::NotQueued,
                merge_state: MergeState::Clean,
                ..Default::default()
            },
        }
    }

    fn agent(
        pane: &str,
        session: &str,
        working: bool,
        asking: bool,
        at: u64,
    ) -> SidebarAgentSnapshot {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":
            [json!({"pane_id": pane, "agent": "claude", "agent_status": "idle", "state_change_seq": 1})]
        }))
        .unwrap();
        let mut row = project_agents(payload).agents.remove(0);
        row.session_id = Some(session.into());
        row.identity_label = format!("agent {pane}");
        row.changed_at_unix_ms = Some(at);
        row.state.working = working;
        row.state.asking = asking;
        row.state.needs_you = asking;
        row.expected_reply = asking.then(|| "Which branch?\nsecond line".to_owned());
        row
    }

    fn board(prs: Vec<PullRequestSnapshot>, agents: &[SidebarAgentSnapshot]) -> Board {
        board_after(prs, agents, &HashMap::new())
    }

    fn board_after(
        prs: Vec<PullRequestSnapshot>,
        agents: &[SidebarAgentSnapshot],
        previous: &HashMap<u32, &'static str>,
    ) -> Board {
        let mut workspace = crate::workspace::inspect_temporary(
            "/fixture/app",
            "device",
            &crate::workspace::PathIndex::default(),
        );
        workspace.pull_requests = prs;
        let agents: Vec<&SidebarAgentSnapshot> = agents.iter().collect();
        project(&workspace, &agents, previous, NOW)
    }

    fn row(board: &Board, number: u32) -> &Row {
        board.rows.iter().find(|row| row.number == number).unwrap()
    }

    #[test]
    fn a_clean_pull_request_of_mine_is_merge_in_needs_you() {
        let board = board(vec![pr(1)], &[]);
        assert_eq!(
            (row(&board, 1).group, row(&board, 1).reason.kind),
            ("needs_you", "merge")
        );
        assert_eq!(board.needs_you, 1);
    }

    #[test]
    fn the_merge_state_decides_conflict_update_and_waiting_for_a_pull_request_of_mine() {
        let mut dirty = pr(1);
        dirty.work.merge_state = MergeState::Dirty;
        let mut behind = pr(2);
        behind.work.merge_state = MergeState::Behind;
        let mut running = pr(3);
        running.work.merge_state = MergeState::Blocked;
        running.checks = PullRequestChecks::Pending;
        running.work.pending_checks = 4;
        let mut unreviewed = pr(4);
        unreviewed.work.merge_state = MergeState::Blocked;
        let mut asked = pr(5);
        asked.work.merge_state = MergeState::Blocked;
        asked.work.review_requests = vec![ReviewRequest {
            kind: ReviewRequestKind::User,
            name: "ann".into(),
        }];
        let board = board(vec![dirty, behind, running, unreviewed, asked], &[]);
        let place = |n| (row(&board, n).group, row(&board, n).reason.kind);
        assert_eq!(place(1), ("blocked", "conflict"));
        assert_eq!(place(2), ("needs_you", "update"));
        assert_eq!(place(3), ("waiting", "ci"));
        assert_eq!(row(&board, 3).reason.count, Some(4));
        assert_eq!(place(4), ("needs_you", "assign_reviewer"));
        assert_eq!(place(5), ("waiting", "waiting_review"));
    }

    #[test]
    fn a_failed_check_blocks_unless_github_calls_the_state_unstable() {
        let failed = |number, state| {
            let mut pr = pr(number);
            pr.checks = PullRequestChecks::Failed;
            pr.work.merge_state = state;
            pr.work.failed_checks = vec![PrFailedCheck {
                name: Some("unit".into()),
                url: None,
            }];
            pr
        };
        let board = board(
            vec![
                failed(1, MergeState::Blocked),
                failed(2, MergeState::Unstable),
            ],
            &[],
        );
        assert_eq!(
            (row(&board, 1).group, row(&board, 1).reason.kind),
            ("blocked", "failed")
        );
        assert_eq!(row(&board, 1).reason.name.as_deref(), Some("unit"));
        assert_eq!(
            (row(&board, 2).group, row(&board, 2).reason.kind),
            ("needs_you", "merge")
        );
        assert_eq!(row(&board, 2).reason.text.as_deref(), Some("unit"));
    }

    #[test]
    fn an_agent_of_the_pull_request_decides_before_the_checks_do() {
        let mut blocked = pr(1);
        blocked.checks = PullRequestChecks::Failed;
        blocked.work.merge_state = MergeState::Blocked;
        blocked.work.sessions = vec!["s1".into()];
        let mut other = pr(2);
        other.work.sessions = vec!["s2".into()];
        let mut third = pr(3);
        third.work.sessions = vec!["s3".into()];
        let agents = [
            agent("p1", "s1", true, false, NOW - 1000),
            agent("p2", "s2", true, true, NOW - 1000),
            agent("p3", "s3", true, false, NOW - INACTIVITY_MS - 1),
        ];
        let board = board(vec![blocked, other, third], &agents);
        assert_eq!(
            (row(&board, 1).group, row(&board, 1).reason.kind),
            ("working", "working")
        );
        let reply = row(&board, 2);
        assert_eq!((reply.group, reply.reason.kind), ("needs_you", "reply"));
        assert_eq!(reply.reason.text.as_deref(), Some("Which branch?"));
        assert_eq!(reply.agent_pane.as_deref(), Some("p2"));
        let check = row(&board, 3);
        assert_eq!((check.group, check.reason.kind), ("needs_you", "check"));
        assert_eq!(check.reason.since_unix_ms, Some(NOW - INACTIVITY_MS - 1));
    }

    #[test]
    fn a_finished_agent_and_an_agent_that_never_worked_the_branch_move_nothing() {
        let mut pr1 = pr(1);
        pr1.work.sessions = vec!["s1".into()];
        let finished = agent("p1", "s1", false, false, NOW);
        let elsewhere = agent("p9", "s9", true, false, NOW);
        let board = board(vec![pr1], &[finished, elsewhere]);
        assert_eq!(row(&board, 1).group, "needs_you");
        assert!(row(&board, 1).agents.is_empty());
    }

    #[test]
    fn a_draft_is_drafts_unless_an_agent_decides_first() {
        let mut draft = pr(1);
        draft.is_draft = true;
        draft.checks = PullRequestChecks::Failed;
        draft.work.merge_state = MergeState::Dirty;
        let mut worked = pr(2);
        worked.is_draft = true;
        worked.work.sessions = vec!["s2".into()];
        let board = board(vec![draft, worked], &[agent("p2", "s2", true, false, NOW)]);
        assert_eq!(row(&board, 1).group, "drafts");
        assert_eq!(row(&board, 2).group, "working");
    }

    #[test]
    fn bots_and_the_review_asks_of_others_pull_requests_are_listed_and_a_stranger_s_is_not() {
        let mut bot = pr(1);
        bot.work.mine = Some(false);
        bot.work.author_is_bot = true;
        let mut direct = pr(2);
        direct.work.mine = Some(false);
        direct.work.asked_of_me = ReviewAsk::Direct;
        direct.work.author = Some("ann".into());
        let mut team = pr(3);
        team.work.mine = Some(false);
        team.work.asked_of_me = ReviewAsk::Team;
        let mut stranger = pr(4);
        stranger.work.mine = Some(false);
        let board = board(vec![bot, direct, team, stranger], &[]);
        assert_eq!(row(&board, 1).group, "bots");
        assert_eq!(
            (row(&board, 2).group, row(&board, 2).reason.kind),
            ("needs_you", "review")
        );
        assert_eq!(row(&board, 3).fold, Some("team"));
        assert!(board.rows.iter().all(|row| row.number != 4));
        assert_eq!(board.needs_you, 1, "the team line is not counted (D-26)");
    }

    #[test]
    fn a_teammate_s_pull_request_asked_of_me_is_a_review_even_when_its_checks_fail() {
        let mut theirs = pr(1);
        theirs.work.mine = Some(false);
        theirs.work.asked_of_me = ReviewAsk::Direct;
        theirs.work.author = Some("ann".into());
        theirs.checks = PullRequestChecks::Failed;
        theirs.work.merge_state = MergeState::Dirty;
        let board = board(vec![theirs], &[]);
        assert_eq!(
            (row(&board, 1).group, row(&board, 1).reason.kind),
            ("needs_you", "review")
        );
    }

    #[test]
    fn a_queued_pull_request_leads_merging_and_a_merged_one_follows_it() {
        let mut queued = pr(1);
        queued.work.queue = MergeQueuePlace::At(2);
        let mut merged = pr(2);
        merged.badge = PullRequestBadge::Merged;
        merged.merged_at_unix_ms = Some(NOW - 10);
        let board = board(vec![merged, queued], &[]);
        let group = board.groups.iter().find(|g| g.group == "merged").unwrap();
        assert_eq!(group.numbers, vec![1, 2]);
        assert_eq!(row(&board, 1).reason.kind, "queued");
    }

    #[test]
    fn nothing_touched_for_seven_days_folds_into_old_and_is_not_counted() {
        let mut old = pr(1);
        old.updated_at_unix_ms = Some(NOW - OLD_MS);
        let board = board(vec![old], &[]);
        assert_eq!(row(&board, 1).fold, Some("old"));
        assert_eq!(board.needs_you, 0);
    }

    #[test]
    fn an_unknown_merge_state_keeps_the_last_place_and_a_new_pull_request_waits() {
        let mut unknown = pr(1);
        unknown.work.merge_state = MergeState::Unknown;
        let mut fresh = pr(2);
        fresh.work.merge_state = MergeState::Unknown;
        let previous = HashMap::from([(1, "needs_you")]);
        let board = board_after(vec![unknown, fresh], &[], &previous);
        assert_eq!(
            (row(&board, 1).group, row(&board, 1).reason.kind),
            ("needs_you", "checking")
        );
        assert_eq!(
            (row(&board, 2).group, row(&board, 2).reason.kind),
            ("waiting", "checking")
        );
    }

    #[test]
    fn a_pull_request_a_session_made_is_listed_even_when_someone_else_wrote_it() {
        let mut theirs = pr(1);
        theirs.work.mine = Some(false);
        theirs.work.sessions = vec!["s1".into()];
        theirs.work.merge_state = MergeState::Blocked;
        let board = board(vec![theirs], &[]);
        assert_eq!(row(&board, 1).group, "waiting");
    }

    /// A pull request read with every extra question unanswered: only the
    /// list's own values are known.
    fn unread(number: u32) -> PullRequestSnapshot {
        let mut pr = pr(number);
        pr.work.mine = None;
        pr.work.asked_of_me = ReviewAsk::Unknown;
        pr.work.queue = MergeQueuePlace::Unknown;
        pr
    }

    fn placed(board: &Board, number: u32) -> (&'static str, &'static str, Option<&str>) {
        let row = row(board, number);
        (row.group, row.reason.kind, row.reason.text.as_deref())
    }

    #[test]
    fn a_pull_request_whose_author_is_unknown_is_listed_in_waiting_and_judged_by_nothing() {
        let board = board(vec![unread(1)], &[]);
        assert_eq!(placed(&board, 1), ("waiting", "unjudged", Some("account")));
        assert_eq!(
            board.needs_you, 0,
            "an unjudged pull request asks nothing of the operator"
        );
    }

    #[test]
    fn what_the_list_alone_decides_is_decided_while_the_account_is_unknown() {
        let mut draft = unread(1);
        draft.is_draft = true;
        let mut bot = unread(2);
        bot.work.author_is_bot = true;
        bot.work.asked_of_me = ReviewAsk::None;
        let mut merged = unread(3);
        merged.badge = PullRequestBadge::Merged;
        merged.merged_at_unix_ms = Some(NOW - 10);
        let mut worked_failing = unread(4);
        worked_failing.work.sessions = vec!["s4".into()];
        worked_failing.checks = PullRequestChecks::Failed;
        worked_failing.work.merge_state = MergeState::Blocked;
        let mut worked_clean = unread(5);
        worked_clean.work.sessions = vec!["s5".into()];
        let board = board(vec![draft, bot, merged, worked_failing, worked_clean], &[]);
        assert_eq!(row(&board, 1).group, "drafts");
        assert_eq!(row(&board, 2).group, "bots");
        assert_ne!(
            row(&board, 2).reason.kind,
            "unjudged",
            "a bot is not the operator"
        );
        assert_eq!(placed(&board, 3).0, "merged");
        assert_eq!(placed(&board, 4).0, "blocked", "their own session owns it");
        assert_eq!(placed(&board, 5), ("waiting", "unjudged", Some("account")));
    }

    #[test]
    fn an_unknown_review_ask_of_a_teammate_s_pull_request_waits_and_a_known_one_still_moves() {
        let mut unasked = unread(1);
        unasked.work.mine = Some(false);
        let mut direct = unread(2);
        direct.work.mine = Some(false);
        direct.work.asked_of_me = ReviewAsk::Direct;
        direct.work.author = Some("ann".into());
        let mut none = unread(3);
        none.work.mine = Some(false);
        none.work.asked_of_me = ReviewAsk::None;
        let board = board(vec![unasked, direct, none], &[]);
        assert_eq!(
            placed(&board, 1),
            ("waiting", "unjudged", Some("review_requests"))
        );
        assert_eq!(placed(&board, 2).1, "review");
        assert!(
            board.rows.iter().all(|row| row.number != 3),
            "judged to be nobody's stays unlisted"
        );
    }

    #[test]
    fn an_unknown_queue_holds_back_only_the_pull_requests_a_queue_could_hold() {
        let unknown_queue = |number: u32, state: MergeState| {
            let mut pr = pr(number);
            pr.work.queue = MergeQueuePlace::Unknown;
            pr.work.merge_state = state;
            pr
        };
        let mut queued = unknown_queue(5, MergeState::Clean);
        queued.work.queue = MergeQueuePlace::At(1);
        let board = board(
            vec![
                unknown_queue(1, MergeState::Clean),
                unknown_queue(2, MergeState::Behind),
                unknown_queue(3, MergeState::Dirty),
                unknown_queue(4, MergeState::Unknown),
                queued,
                pr(6),
            ],
            &[],
        );
        assert_eq!(
            placed(&board, 1),
            ("waiting", "unjudged", Some("merge_queue"))
        );
        assert_eq!(
            placed(&board, 2),
            ("waiting", "unjudged", Some("merge_queue"))
        );
        assert_eq!(
            placed(&board, 3).0,
            "blocked",
            "a conflicted pull request leaves the queue"
        );
        assert_eq!(placed(&board, 4).1, "checking");
        assert_eq!(placed(&board, 5).1, "queued");
        assert_eq!(
            placed(&board, 6).1,
            "merge",
            "a queue that answered not queued is judged"
        );
        assert_eq!(board.needs_you, 1);
    }
}
