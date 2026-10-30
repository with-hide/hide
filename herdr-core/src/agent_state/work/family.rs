//! A session's pull requests (docs/status-model.md, A session's pull
//! requests): the ones it holds or made and every descendant's, live or
//! ended, each once. It is the one list every surface draws: the 태스크 row,
//! the pane header, the 프로젝트 tree and the session record.
//!
//! A live row brings what it holds now (`work::linked_pull_requests`, its
//! duty or its record's history); the link record brings what every session
//! it keeps made, along the lineage it keeps after the panes are gone
//! ([`Recorded`]). A pull request GitHub's list holds is drawn as GitHub has
//! it; one the list no longer holds, past its 200 newest, keeps the state the
//! record last read and is stale, never dropped.
//!
//! Each pass is in memory over rows and summaries already in hand, under the
//! lock, with no I/O; the rows it lays are compared before they are written,
//! so an unchanged list publishes nothing.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;

use crate::links::ProjectLinkSummary;
use crate::links::ledger::{SessionIdentity, SessionRecord};
use crate::model::{GithubSnapshot, PrState, SidebarAgentSnapshot};

/// The most pull requests one session's list carries; past it the worst
/// stay listed and the rest are counted in `omitted`, with a diagnostic.
pub const PULL_LIMIT: usize = 200;

/// The most recorded sessions one list follows down its lineage, the record
/// tree's own budget (`links/ledger.rs`).
const SESSION_LIMIT: usize = 2_000;

/// One pull request of a session's list as every surface draws it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SessionPull {
    pub url: String,
    pub number: u32,
    pub title: String,
    pub state: PrState,
    /// The session the list is of holds or made it itself, not only a
    /// descendant: the 프로젝트 tree's child rows draw these alone.
    pub own: bool,
    /// Who holds or made it, by name, the session itself among them.
    pub holders: Vec<String>,
    /// GitHub could not be read again, or its list no longer holds it:
    /// `state` is the one last read, at `last_read_at_unix_ms`.
    pub stale: bool,
    pub last_read_at_unix_ms: Option<u64>,
}

/// A session's list, worst first, then the highest number.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SessionPulls {
    pub pulls: Vec<SessionPull>,
    /// Pull requests past [`PULL_LIMIT`], counted and not listed.
    #[serde(skip_serializing_if = "is_zero")]
    pub omitted: usize,
}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

/// The session record's `PR` section (D-14, B18): the list of its session,
/// its own first, then one group per direct child holding that child's
/// whole subtree's, each pull request once.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PrGroup {
    /// `None` for this session's own.
    pub child: Option<SessionIdentity>,
    pub name: String,
    pub pulls: Vec<SessionPull>,
}

/// A pull request the link record holds for a session, as it last read it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordedPull {
    pub(crate) url: String,
    pub(crate) number: u32,
    pub(crate) title: String,
    pub(crate) state: PrState,
    pub(crate) last_read_at_unix_ms: Option<u64>,
}

/// What the link record keeps of sessions: who started whom, each one's
/// name and the pull requests each made. Built from the link worker's
/// project summaries for the rows, and from a record's own tree for that
/// record; both read the same ledger tables.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Recorded {
    children: BTreeMap<SessionIdentity, BTreeSet<SessionIdentity>>,
    names: HashMap<SessionIdentity, String>,
    pulls: HashMap<SessionIdentity, Vec<RecordedPull>>,
}

impl Recorded {
    pub(crate) fn from_summaries<'a>(
        summaries: impl IntoIterator<Item = &'a ProjectLinkSummary>,
    ) -> Self {
        let mut recorded = Self::default();
        for summary in summaries {
            for work in &summary.work {
                let name = work.manual_title.as_ref().unwrap_or(&work.title);
                recorded.names.insert(work.session.clone(), name.clone());
                if let Some(parent) = &work.parent {
                    recorded
                        .children
                        .entry(parent.clone())
                        .or_default()
                        .insert(work.session.clone());
                }
            }
            for creator in summary.creators.iter().filter(|output| output.kind == "pr") {
                let Ok(number) = u32::try_from(creator.number) else {
                    continue;
                };
                recorded.add(
                    &creator.source,
                    RecordedPull {
                        url: creator.url.clone(),
                        number,
                        title: creator.title.clone(),
                        state: recorded_state(&creator.state),
                        last_read_at_unix_ms: creator.last_read_at_unix_ms,
                    },
                );
            }
        }
        recorded
    }

    /// The record's own tree: each node's outputs include its descendants',
    /// so a node keeps only those its own session made.
    pub(crate) fn from_record(record: &SessionRecord) -> Self {
        fn walk(recorded: &mut Recorded, node: &SessionRecord) {
            let name = node.manual_title.as_ref().unwrap_or(&node.title);
            recorded.names.insert(node.session.clone(), name.clone());
            for output in &node.outputs {
                let Ok(number) = u32::try_from(output.number) else {
                    continue;
                };
                if output.kind == "pr" && output.source == node.session {
                    recorded.add(
                        &node.session,
                        RecordedPull {
                            url: output.url.clone(),
                            number,
                            title: output.title.clone(),
                            state: recorded_state(&output.state),
                            last_read_at_unix_ms: output.last_read_at_unix_ms,
                        },
                    );
                }
            }
            for child in &node.children {
                recorded
                    .children
                    .entry(node.session.clone())
                    .or_default()
                    .insert(child.session.clone());
                walk(recorded, child);
            }
        }
        let mut recorded = Self::default();
        walk(&mut recorded, record);
        recorded
    }

    fn add(&mut self, session: &SessionIdentity, pull: RecordedPull) {
        let pulls = self.pulls.entry(session.clone()).or_default();
        if !pulls.iter().any(|kept| kept.url == pull.url) {
            pulls.push(pull);
        }
    }
}

/// The state the record keeps (`ledger_outputs.state`): merged, closed, CI
/// failed, and otherwise open with checks it did not keep, which never read
/// as a pass.
fn recorded_state(state: &str) -> PrState {
    match state {
        "merged" => PrState::Merged,
        "closed" => PrState::Closed,
        "ci_failed" => PrState::Failed,
        _ => PrState::Pending,
    }
}

/// How GitHub's list has a pull request now.
struct Read<'a> {
    state: PrState,
    title: &'a str,
    stale: bool,
    last_read_at_unix_ms: Option<u64>,
}

/// The rows and GitHub's lists one pass reads, indexed once.
pub(crate) struct Sources<'a> {
    rows: Vec<&'a SidebarAgentSnapshot>,
    sessions: &'a [Option<SessionIdentity>],
    by_pane: HashMap<&'a str, usize>,
    by_session: HashMap<&'a SessionIdentity, usize>,
    github: HashMap<&'a str, Read<'a>>,
}

impl<'a> Sources<'a> {
    /// `sessions[i]` is the session `rows[i]` hosts, on its own machine.
    pub(crate) fn new(
        rows: impl IntoIterator<Item = &'a SidebarAgentSnapshot>,
        sessions: &'a [Option<SessionIdentity>],
        github: impl IntoIterator<Item = &'a GithubSnapshot>,
    ) -> Self {
        let rows: Vec<&SidebarAgentSnapshot> = rows.into_iter().collect();
        let by_pane = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.pane_id.as_str(), index))
            .collect();
        let by_session = sessions
            .iter()
            .enumerate()
            .filter_map(|(index, session)| Some((session.as_ref()?, index)))
            .collect();
        let github = github
            .into_iter()
            .flat_map(|snapshot| &snapshot.projects)
            .flat_map(|project| {
                project.pull_requests.iter().map(move |pull| {
                    (
                        pull.url.as_str(),
                        Read {
                            state: PrState::of(pull.badge, pull.is_draft, pull.checks, pull.review),
                            title: pull.title.as_str(),
                            stale: project.status.stale,
                            last_read_at_unix_ms: project.status.last_success_at_unix_ms,
                        },
                    )
                })
            })
            .collect();
        Self {
            rows,
            sessions,
            by_pane,
            by_session,
            github,
        }
    }

    /// The list of the session that `row` hosts or that `session` names, and
    /// of every descendant: the live ones by Herdr's lineage, the ended ones
    /// by the record's. `None` when it has none.
    pub(crate) fn pulls(
        &self,
        row: Option<usize>,
        session: Option<&SessionIdentity>,
        recorded: &Recorded,
    ) -> Option<SessionPulls> {
        let session = session.or_else(|| row.and_then(|row| self.sessions[row].as_ref()));
        let row = row.or_else(|| session.and_then(|session| self.by_session.get(session).copied()));
        let mut live = BTreeSet::new();
        let mut kept = BTreeSet::new();
        let mut rows: Vec<usize> = row.into_iter().collect();
        let mut sessions: Vec<&SessionIdentity> = session.into_iter().collect();
        let mut items: BTreeMap<String, SessionPull> = BTreeMap::new();
        while !rows.is_empty() || !sessions.is_empty() {
            while let Some(index) = rows.pop() {
                if !live.insert(index) {
                    continue;
                }
                let agent = &self.rows[index];
                let own = Some(index) == row;
                for pull in agent
                    .request
                    .iter()
                    .flat_map(|request| &request.pull_requests)
                    .filter(|pull| pull.duty || pull.provisional)
                {
                    let read = self.github.get(pull.url.as_str());
                    add(
                        &mut items,
                        SessionPull {
                            url: pull.url.clone(),
                            number: pull.number,
                            title: pull.title.clone(),
                            state: pull.state(),
                            own,
                            holders: vec![agent.identity_label.clone()],
                            stale: read.is_some_and(|read| read.stale),
                            last_read_at_unix_ms: read.and_then(|read| read.last_read_at_unix_ms),
                        },
                    );
                }
                if let Some(session) = &self.sessions[index] {
                    sessions.push(session);
                }
                rows.extend(
                    agent
                        .lineage_child_pane_ids
                        .iter()
                        .filter_map(|pane| self.by_pane.get(pane.as_str()).copied()),
                );
            }
            while let Some(next) = sessions.pop() {
                if kept.len() >= SESSION_LIMIT || !kept.insert(next) {
                    continue;
                }
                let host = self.by_session.get(next).copied();
                if let Some(host) = host {
                    rows.push(host);
                }
                let own = Some(next) == session;
                let name = host
                    .map(|host| &self.rows[host].identity_label)
                    .or_else(|| recorded.names.get(next));
                for pull in recorded.pulls.get(next).into_iter().flatten() {
                    let item = match self.github.get(pull.url.as_str()) {
                        Some(read) => SessionPull {
                            url: pull.url.clone(),
                            number: pull.number,
                            title: read.title.to_owned(),
                            state: read.state,
                            own,
                            holders: name.cloned().into_iter().collect(),
                            stale: read.stale,
                            last_read_at_unix_ms: read.last_read_at_unix_ms,
                        },
                        None => SessionPull {
                            url: pull.url.clone(),
                            number: pull.number,
                            title: pull.title.clone(),
                            state: pull.state,
                            own,
                            holders: name.cloned().into_iter().collect(),
                            stale: true,
                            last_read_at_unix_ms: pull.last_read_at_unix_ms,
                        },
                    };
                    add(&mut items, item);
                }
                sessions.extend(recorded.children.get(next).into_iter().flatten());
            }
        }
        if items.is_empty() {
            return None;
        }
        let mut pulls: Vec<SessionPull> = items.into_values().collect();
        pulls.sort_by(|a, b| a.state.cmp(&b.state).then_with(|| b.number.cmp(&a.number)));
        let omitted = pulls.len().saturating_sub(PULL_LIMIT);
        pulls.truncate(PULL_LIMIT);
        Some(SessionPulls { pulls, omitted })
    }

    /// The record's `PR` section for `record` and each of its children's
    /// records: the list of its session, its own first, then each direct
    /// child's, each pull request in the first group that holds it.
    pub(crate) fn lay_groups(&self, record: &mut SessionRecord, recorded: &Recorded) {
        record.pr_groups = self.groups(record, recorded);
        for child in &mut record.children {
            self.lay_groups(child, recorded);
        }
    }

    /// [`Self::lay_groups`] without writing, to compare with what a record
    /// already holds.
    pub(crate) fn groups_match(&self, record: &SessionRecord, recorded: &Recorded) -> bool {
        record.pr_groups == self.groups(record, recorded)
            && record
                .children
                .iter()
                .all(|child| self.groups_match(child, recorded))
    }

    fn groups(&self, record: &SessionRecord, recorded: &Recorded) -> Vec<PrGroup> {
        let Some(all) = self.pulls(None, Some(&record.session), recorded) else {
            return Vec::new();
        };
        let mut placed = BTreeSet::new();
        let mut groups = Vec::new();
        let own: Vec<SessionPull> = all.pulls.iter().filter(|pull| pull.own).cloned().collect();
        placed.extend(own.iter().map(|pull| pull.url.clone()));
        if !own.is_empty() {
            groups.push(PrGroup {
                child: None,
                name: record.title.clone(),
                pulls: own,
            });
        }
        // The children the record keeps, then a live child it has not read yet.
        let mut children: Vec<(SessionIdentity, String)> = record
            .children
            .iter()
            .map(|child| (child.session.clone(), child.title.clone()))
            .collect();
        if let Some(row) = self.by_session.get(&record.session) {
            for pane in &self.rows[*row].lineage_child_pane_ids {
                let Some(&child) = self.by_pane.get(pane.as_str()) else {
                    continue;
                };
                if let Some(session) = &self.sessions[child]
                    && !children.iter().any(|(known, _)| known == session)
                {
                    children.push((session.clone(), self.rows[child].identity_label.clone()));
                }
            }
        }
        for (session, name) in children {
            let Some(theirs) = self.pulls(None, Some(&session), recorded) else {
                continue;
            };
            let pulls: Vec<SessionPull> = theirs
                .pulls
                .into_iter()
                .filter(|pull| placed.insert(pull.url.clone()))
                .map(|pull| {
                    // Within the parent's record the child's state is the
                    // parent's list's, so the two never disagree.
                    all.pulls
                        .iter()
                        .find(|kept| kept.url == pull.url)
                        .cloned()
                        .unwrap_or(pull)
                })
                .collect();
            if !pulls.is_empty() {
                groups.push(PrGroup {
                    child: Some(session),
                    name,
                    pulls,
                });
            }
        }
        groups
    }
}

/// Adds one sighting of a pull request to a list: a second holder joins its
/// holders, and a fresh read replaces a stale one.
fn add(items: &mut BTreeMap<String, SessionPull>, item: SessionPull) {
    match items.get_mut(&item.url) {
        None => {
            items.insert(item.url.clone(), item);
        }
        Some(kept) => {
            kept.own |= item.own;
            for holder in item.holders {
                if !kept.holders.contains(&holder) {
                    kept.holders.push(holder);
                }
            }
            if kept.stale && !item.stale {
                kept.state = item.state;
                kept.title = item.title;
                kept.stale = false;
                kept.last_read_at_unix_ms = item.last_read_at_unix_ms;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::ledger::{OutputCreator, SessionWorkSummary, WorkOutput};
    use crate::model::{
        GithubProjectSnapshot, GithubStatusSnapshot, PullRequestBadge, PullRequestChecks,
        PullRequestSnapshot,
    };
    use crate::request_view::{AgentPullRequestSnapshot, AgentRequestSnapshot};

    const DEVICE: &str = "node-a";

    fn session(id: &str) -> SessionIdentity {
        SessionIdentity {
            device_id: DEVICE.into(),
            agent: "claude".into(),
            session_id: id.into(),
        }
    }

    fn url(number: u32) -> String {
        format!("https://github.com/acme/app/pull/{number}")
    }

    /// Live rows `(pane, session, children)`, as Herdr projects its agents.
    fn rows(
        live: &[(&str, &str, &[&str])],
    ) -> (Vec<SidebarAgentSnapshot>, Vec<Option<SessionIdentity>>) {
        let payload: crate::sidebar::SessionSnapshotPayload =
            serde_json::from_value(serde_json::json!({
                "agents": live.iter().map(|(pane, _, _)| serde_json::json!({
                    "pane_id": pane, "agent": "claude", "agent_status": "idle", "state_change_seq": 1,
                })).collect::<Vec<_>>()
            }))
            .unwrap();
        let mut rows = crate::sidebar::project_agents(payload).agents;
        for (row, (pane, id, children)) in rows.iter_mut().zip(live) {
            row.identity_label = format!("label {pane}");
            row.session_id = Some((*id).into());
            row.lineage_child_pane_ids = children.iter().map(|child| (*child).into()).collect();
        }
        let sessions = live.iter().map(|(_, id, _)| Some(session(id))).collect();
        (rows, sessions)
    }

    /// The row holds the duty of pull request `number`, as GitHub lists it.
    fn holds(row: &mut SidebarAgentSnapshot, pull: &PullRequestSnapshot) {
        let request = row.request.get_or_insert_with(|| AgentRequestSnapshot {
            verb: crate::agent_state::RequestVerb::Idle,
            verb_since_unix_ms: 0,
            line: None,
            end: None,
            request: None,
            request_summary: None,
            recent_turns: Vec::new(),
            later_by: None,
            pull_requests: Vec::new(),
        });
        request.pull_requests.push(AgentPullRequestSnapshot {
            number: pull.number,
            title: pull.title.clone(),
            url: pull.url.clone(),
            badge: pull.badge,
            checks: pull.checks,
            review: None,
            is_draft: false,
            head_branch: pull.head_branch.clone(),
            closing_issues: Vec::new(),
            live: true,
            duty: true,
            provisional: false,
            created: true,
            settled_at_unix_ms: None,
        });
    }

    fn listed(
        number: u32,
        badge: PullRequestBadge,
        checks: PullRequestChecks,
    ) -> PullRequestSnapshot {
        PullRequestSnapshot {
            closing_issues: Vec::new(),
            title: format!("PR {number}"),
            checks,
            number,
            head_branch: format!("branch-{number}"),
            base_branch: "main".into(),
            url: url(number),
            badge,
            review: None,
            is_draft: false,
            merged_at_unix_ms: None,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            closed_at_unix_ms: None,
            head_oid: None,
            cross_repository: false,
            work: Default::default(),
        }
    }

    fn github(pulls: Vec<PullRequestSnapshot>) -> GithubSnapshot {
        GithubSnapshot {
            projects: vec![GithubProjectSnapshot {
                root_path: "/work/app".into(),
                status: GithubStatusSnapshot {
                    last_success_at_unix_ms: Some(9_000),
                    ..Default::default()
                },
                pull_requests: pulls,
                ..Default::default()
            }],
        }
    }

    /// A session the record keeps: `(session, parent, name, made)`, each
    /// made pull request as `(number, recorded state)`.
    type Kept<'a> = (&'a str, Option<&'a str>, &'a str, &'a [(u32, &'a str)]);

    /// The link worker's summary of the sessions it keeps.
    fn summary(kept: &[Kept<'_>]) -> ProjectLinkSummary {
        ProjectLinkSummary {
            work: kept
                .iter()
                .map(|(id, parent, name, _)| SessionWorkSummary {
                    session: session(id),
                    title: (*name).into(),
                    fallback_title: String::new(),
                    native_title: None,
                    manual_title: None,
                    cwd: None,
                    ended_at_unix_ms: Some(1),
                    open_missed: 0,
                    subtree_open_missed: 0,
                    missed: Vec::new(),
                    open_asks: 0,
                    parent: parent.map(session),
                })
                .collect(),
            creators: kept
                .iter()
                .flat_map(|(id, _, _, made)| {
                    made.iter().map(move |(number, state)| OutputCreator {
                        source: session(id),
                        kind: "pr".into(),
                        repository: "acme/app".into(),
                        number: u64::from(*number),
                        url: url(*number),
                        title: format!("Recorded {number}"),
                        state: (*state).into(),
                        last_read_at_unix_ms: Some(5_000),
                    })
                })
                .collect(),
            ..Default::default()
        }
    }

    fn numbers(list: &SessionPulls) -> Vec<(u32, PrState)> {
        list.pulls
            .iter()
            .map(|pull| (pull.number, pull.state))
            .collect()
    }

    /// mission-17: a Conductor's 태스크 row showed 3 pull requests while its
    /// record held 15, because each child the Conductor ended took its pull
    /// requests out of the row.
    #[test]
    fn an_ended_childs_pull_requests_stay_in_its_parents_list() {
        let (mut rows, sessions) = rows(&[("lead", "s-lead", &["live"]), ("live", "s-live", &[])]);
        let open = listed(3, PullRequestBadge::Open, PullRequestChecks::Passing);
        holds(&mut rows[1], &open);
        let merged = listed(1, PullRequestBadge::Merged, PullRequestChecks::Passing);
        let github = github(vec![open, merged]);
        let recorded = Recorded::from_summaries([&summary(&[
            ("s-lead", None, "lead", &[]),
            ("s-live", Some("s-lead"), "live", &[]),
            (
                "s-ended",
                Some("s-lead"),
                "ended child",
                &[(1, "review"), (2, "merged")],
            ),
        ])]);
        let sources = Sources::new(rows.iter(), &sessions, [&github]);
        let lead = sources
            .pulls(Some(0), sessions[0].as_ref(), &recorded)
            .unwrap();
        assert_eq!(
            numbers(&lead),
            [
                (3, PrState::Mergeable),
                (2, PrState::Merged),
                (1, PrState::Merged)
            ],
            "the live child's, and the ended child's in GitHub's state where it lists them"
        );
        let ended = lead.pulls.iter().find(|pull| pull.number == 1).unwrap();
        assert_eq!(ended.holders, ["ended child"]);
        assert!(!ended.own, "a descendant's is not the lead's own");
        // The ended child's own row is gone; its live sibling lists only its own.
        let live = sources
            .pulls(Some(1), sessions[1].as_ref(), &recorded)
            .unwrap();
        assert_eq!(numbers(&live), [(3, PrState::Mergeable)]);
        assert!(live.pulls[0].own);
    }

    /// The operator: merged pull requests are all shown, closed ones dimmed,
    /// never left out.
    #[test]
    fn a_closed_pull_request_stays_listed_after_the_merged_ones() {
        let (mut rows, sessions) = rows(&[("agent", "s-agent", &[])]);
        let closed = listed(4, PullRequestBadge::Closed, PullRequestChecks::Failed);
        let merged = listed(5, PullRequestBadge::Merged, PullRequestChecks::Passing);
        holds(&mut rows[0], &closed);
        holds(&mut rows[0], &merged);
        let github = github(vec![closed, merged]);
        let sources = Sources::new(rows.iter(), &sessions, [&github]);
        let list = sources.pulls(Some(0), None, &Recorded::default()).unwrap();
        assert_eq!(numbers(&list), [(5, PrState::Merged), (4, PrState::Closed)]);
    }

    /// GitHub's list holds the 200 newest pull requests: one past it keeps the
    /// state the record last read, as stale, instead of leaving the list.
    #[test]
    fn a_pull_request_past_githubs_list_keeps_the_records_state_as_stale() {
        let (rows, sessions) = rows(&[("agent", "s-agent", &[])]);
        let github = github(Vec::new());
        let recorded = Recorded::from_summaries([&summary(&[(
            "s-agent",
            None,
            "agent",
            &[(6, "merged"), (7, "review")],
        )])]);
        let sources = Sources::new(rows.iter(), &sessions, [&github]);
        let list = sources.pulls(Some(0), None, &recorded).unwrap();
        assert_eq!(
            numbers(&list),
            [(7, PrState::Pending), (6, PrState::Merged)]
        );
        assert!(list.pulls.iter().all(|pull| pull.stale && pull.own));
        assert_eq!(list.pulls[0].title, "Recorded 7");
        assert_eq!(list.pulls[0].last_read_at_unix_ms, Some(5_000));
    }

    fn output(source: &str, number: u32, state: &str) -> WorkOutput {
        WorkOutput {
            kind: "pr".into(),
            repository: "acme/app".into(),
            number: u64::from(number),
            title: format!("Recorded {number}"),
            url: url(number),
            state: state.into(),
            stale: false,
            last_read_at_unix_ms: Some(5_000),
            created_at_unix_ms: 1,
            source: session(source),
            source_name: String::new(),
            sighted_at_unix_ms: 1,
            settled_at_unix_ms: None,
        }
    }

    fn record(
        id: &str,
        title: &str,
        outputs: Vec<WorkOutput>,
        children: Vec<SessionRecord>,
    ) -> SessionRecord {
        let mut outputs = outputs;
        outputs.extend(children.iter().flat_map(|child| child.outputs.clone()));
        SessionRecord {
            session: session(id),
            title: title.into(),
            outputs,
            children,
            ..Default::default()
        }
    }

    /// The record's `PR` section and the row read one list: the same pull
    /// requests in the same states, the record grouping a grandchild's under
    /// the direct child that led to it (D-14, B18).
    #[test]
    fn the_record_groups_the_same_list_the_row_draws() {
        let (mut rows, sessions) = rows(&[("lead", "s-lead", &[])]);
        let own = listed(10, PullRequestBadge::Open, PullRequestChecks::Failed);
        holds(&mut rows[0], &own);
        let github = github(vec![
            own,
            listed(12, PullRequestBadge::Closed, PullRequestChecks::None),
        ]);
        let made: &[Kept<'_>] = &[
            ("s-lead", None, "lead", &[]),
            ("s-child", Some("s-lead"), "child", &[(11, "merged")]),
            (
                "s-grandchild",
                Some("s-child"),
                "grandchild",
                &[(12, "review")],
            ),
        ];
        let tree = record(
            "s-lead",
            "lead",
            Vec::new(),
            vec![record(
                "s-child",
                "child",
                vec![output("s-child", 11, "merged")],
                vec![record(
                    "s-grandchild",
                    "grandchild",
                    vec![output("s-grandchild", 12, "review")],
                    Vec::new(),
                )],
            )],
        );
        let sources = Sources::new(rows.iter(), &sessions, [&github]);
        let row = sources
            .pulls(
                Some(0),
                sessions[0].as_ref(),
                &Recorded::from_summaries([&summary(made)]),
            )
            .unwrap();
        let mut laid = tree.clone();
        sources.lay_groups(&mut laid, &Recorded::from_record(&tree));
        let grouped: Vec<(Option<&str>, u32, PrState)> = laid
            .pr_groups
            .iter()
            .flat_map(|group| {
                group.pulls.iter().map(move |pull| {
                    (
                        group.child.as_ref().map(|child| child.session_id.as_str()),
                        pull.number,
                        pull.state,
                    )
                })
            })
            .collect();
        assert_eq!(
            grouped,
            [
                (None, 10, PrState::Failed),
                (Some("s-child"), 11, PrState::Merged),
                (Some("s-child"), 12, PrState::Closed),
            ]
        );
        let mut record_list: Vec<_> = grouped
            .iter()
            .map(|(_, number, state)| (*number, *state))
            .collect();
        let mut row_list = numbers(&row);
        record_list.sort();
        row_list.sort();
        assert_eq!(
            record_list, row_list,
            "the record and the row show the same pull requests"
        );
        assert!(sources.groups_match(&laid, &Recorded::from_record(&tree)));
        // The child's own record, opened from the list, groups its own subtree.
        assert_eq!(laid.children[0].pr_groups[0].pulls[0].number, 11);
        assert_eq!(
            laid.children[0].pr_groups[1].child,
            Some(session("s-grandchild"))
        );
    }
}
