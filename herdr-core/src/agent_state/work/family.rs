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
//! so an unchanged list publishes nothing. A pull request read fresh carries
//! no read time, so GitHub answering the same again changes no list.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;

use crate::links::ProjectLinkSummary;
use crate::links::ledger::{SessionIdentity, SessionRecord};
use crate::model::{GithubSnapshot, PrState, SidebarAgentSnapshot};

/// The most pull requests one session's list carries; past it the worst
/// stay listed and the rest are counted in `omitted`, with a diagnostic.
pub const PULL_LIMIT: usize = 200;

/// The most recorded sessions one list follows down its lineage, the record
/// tree's own budget (`links/ledger.rs`); past it the list says so in
/// `lineage_capped`, with a diagnostic.
pub(crate) const SESSION_LIMIT: usize = 2_000;

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
    /// When a stale state was read; none while the read is fresh.
    pub last_read_at_unix_ms: Option<u64>,
}

/// A session's list, worst first, then the highest number.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SessionPulls {
    pub pulls: Vec<SessionPull>,
    /// Pull requests past [`PULL_LIMIT`], counted and not listed.
    #[serde(skip_serializing_if = "is_zero")]
    pub omitted: usize,
    /// The lineage held more than [`SESSION_LIMIT`] recorded sessions, and
    /// the ones past it were not followed. Logged, never drawn.
    #[serde(skip)]
    pub lineage_capped: bool,
}

pub(crate) fn is_zero(count: &usize) -> bool {
    *count == 0
}

/// The session record's `PR` section (D-14, B18): the list of its session,
/// the one its row draws with the same cap, its own first, then one group
/// per direct child holding that child's whole subtree's, each pull request
/// once.
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
#[derive(Clone, Copy)]
struct Read<'a> {
    state: PrState,
    title: &'a str,
    stale: bool,
    last_read_at_unix_ms: Option<u64>,
}

impl Read<'_> {
    /// Which of two machines' reads of one pull request a list takes: a
    /// fresh one over a stale one, then the later read, then the worse state
    /// and the title, so the answer never depends on which machine was read
    /// first. Each read time is its own machine's clock; a skew between them
    /// only orders two reads GitHub gave within one poll of each other.
    fn rank(&self) -> (bool, Option<u64>, std::cmp::Reverse<PrState>, &str) {
        (
            !self.stale,
            self.last_read_at_unix_ms,
            std::cmp::Reverse(self.state),
            self.title,
        )
    }

    /// The read time a list keeps: only a stale state's.
    fn stale_since(&self) -> Option<u64> {
        self.last_read_at_unix_ms.filter(|_| self.stale)
    }
}

/// Every pull request one walk down a lineage reached, before the cap.
struct Gathered {
    items: BTreeMap<String, SessionPull>,
    lineage_capped: bool,
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
    /// `sessions[i]` is the session `rows[i]` hosts, on its own machine;
    /// `github` holds this machine's read and each device's.
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
        let mut reads: HashMap<&str, Read> = HashMap::new();
        for project in github.into_iter().flat_map(|snapshot| &snapshot.projects) {
            for pull in &project.pull_requests {
                let read = Read {
                    state: PrState::of(pull.badge, pull.is_draft, pull.checks, pull.review),
                    title: pull.title.as_str(),
                    stale: project.status.stale,
                    last_read_at_unix_ms: project.status.last_success_at_unix_ms,
                };
                reads
                    .entry(pull.url.as_str())
                    .and_modify(|kept| {
                        if read.rank() > kept.rank() {
                            *kept = read;
                        }
                    })
                    .or_insert(read);
            }
        }
        Self {
            rows,
            sessions,
            by_pane,
            by_session,
            github: reads,
        }
    }

    /// The list of the session that `row` hosts or that `session` names, and
    /// of every descendant: the live ones by Herdr's lineage, the ended ones
    /// by the record's. `recorded` is what the link record keeps, freshest
    /// first. `None` when it has none.
    pub(crate) fn pulls(
        &self,
        row: Option<usize>,
        session: Option<&SessionIdentity>,
        recorded: &[&Recorded],
    ) -> Option<SessionPulls> {
        let Gathered {
            items,
            lineage_capped,
        } = self.gather(row, session, recorded);
        if items.is_empty() {
            return None;
        }
        let mut pulls: Vec<SessionPull> = items.into_values().collect();
        pulls.sort_by(|a, b| a.state.cmp(&b.state).then_with(|| b.number.cmp(&a.number)));
        let omitted = pulls.len().saturating_sub(PULL_LIMIT);
        pulls.truncate(PULL_LIMIT);
        Some(SessionPulls {
            pulls,
            omitted,
            lineage_capped,
        })
    }

    fn gather(
        &self,
        row: Option<usize>,
        session: Option<&SessionIdentity>,
        recorded: &[&Recorded],
    ) -> Gathered {
        let session = session.or_else(|| row.and_then(|row| self.sessions[row].as_ref()));
        let row = row.or_else(|| session.and_then(|session| self.by_session.get(session).copied()));
        let mut live = BTreeSet::new();
        let mut kept = BTreeSet::new();
        let mut lineage_capped = false;
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
                            last_read_at_unix_ms: read.and_then(Read::stale_since),
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
                if kept.contains(next) {
                    continue;
                }
                if kept.len() >= SESSION_LIMIT {
                    lineage_capped = true;
                    continue;
                }
                kept.insert(next);
                let host = self.by_session.get(next).copied();
                if let Some(host) = host {
                    rows.push(host);
                }
                let own = Some(next) == session;
                let name = host
                    .map(|host| &self.rows[host].identity_label)
                    .or_else(|| recorded.iter().find_map(|layer| layer.names.get(next)));
                for pull in recorded
                    .iter()
                    .flat_map(|layer| layer.pulls.get(next).into_iter().flatten())
                {
                    let item = match self.github.get(pull.url.as_str()) {
                        Some(read) => SessionPull {
                            url: pull.url.clone(),
                            number: pull.number,
                            title: read.title.to_owned(),
                            state: read.state,
                            own,
                            holders: name.cloned().into_iter().collect(),
                            stale: read.stale,
                            last_read_at_unix_ms: read.stale_since(),
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
                sessions.extend(
                    recorded
                        .iter()
                        .flat_map(|layer| layer.children.get(next).into_iter().flatten()),
                );
            }
        }
        Gathered {
            items,
            lineage_capped,
        }
    }

    /// The record's `PR` section for `record` and each of its children's
    /// records: the list of its session, its own first, then each direct
    /// child's, each pull request in the first group that holds it. Returns
    /// what `record`'s own list left out.
    pub(crate) fn lay_groups(&self, record: &mut SessionRecord, recorded: &[&Recorded]) -> Capped {
        let (groups, capped) = self.groups(record, recorded);
        record.pr_groups = groups;
        record.pr_omitted = capped.omitted;
        for child in &mut record.children {
            self.lay_groups(child, recorded);
        }
        capped
    }

    /// [`Self::lay_groups`] without writing, to compare with what a record
    /// already holds.
    pub(crate) fn groups_match(&self, record: &SessionRecord, recorded: &[&Recorded]) -> bool {
        let (groups, capped) = self.groups(record, recorded);
        record.pr_groups == groups
            && record.pr_omitted == capped.omitted
            && record
                .children
                .iter()
                .all(|child| self.groups_match(child, recorded))
    }

    /// The section of `record`: its session's list, capped as its row's is,
    /// cut into groups, and the count the cap left out. Each direct child's
    /// whole subtree says which of the list it holds; one reached through no
    /// named child (a live child Herdr has not named a session for yet) stays
    /// in this session's group, so the section holds exactly the list.
    fn groups(&self, record: &SessionRecord, recorded: &[&Recorded]) -> (Vec<PrGroup>, Capped) {
        let Some(all) = self.pulls(None, Some(&record.session), recorded) else {
            return (Vec::new(), Capped::default());
        };
        // The children the record keeps, the ones the link record has kept
        // since, then a live child neither has read yet.
        let mut children: Vec<(SessionIdentity, String)> = record
            .children
            .iter()
            .map(|child| (child.session.clone(), child.title.clone()))
            .collect();
        for layer in recorded {
            for child in layer.children.get(&record.session).into_iter().flatten() {
                if !children.iter().any(|(known, _)| known == child) {
                    let name = layer.names.get(child).unwrap_or(&child.session_id);
                    children.push((child.clone(), name.clone()));
                }
            }
        }
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
        // Which child's group each pull request of the list falls in.
        let descendants = all.pulls.iter().filter(|pull| !pull.own).count();
        let mut placed: HashMap<&str, usize> = HashMap::new();
        for (index, (session, _)) in children.iter().enumerate() {
            if placed.len() == descendants {
                break;
            }
            let held = self.gather(None, Some(session), recorded).items;
            for pull in all.pulls.iter().filter(|pull| !pull.own) {
                if held.contains_key(&pull.url) {
                    placed.entry(pull.url.as_str()).or_insert(index);
                }
            }
        }
        let mut theirs: Vec<Vec<SessionPull>> = vec![Vec::new(); children.len()];
        let mut own = Vec::new();
        for pull in &all.pulls {
            match placed.get(pull.url.as_str()) {
                Some(&index) => theirs[index].push(pull.clone()),
                None => own.push(pull.clone()),
            }
        }
        let mut groups = Vec::new();
        if !own.is_empty() {
            groups.push(PrGroup {
                child: None,
                name: record.title.clone(),
                pulls: own,
            });
        }
        for ((session, name), pulls) in children.into_iter().zip(theirs) {
            if !pulls.is_empty() {
                groups.push(PrGroup {
                    child: Some(session),
                    name,
                    pulls,
                });
            }
        }
        let capped = Capped {
            omitted: all.omitted,
            lineage: all.lineage_capped,
        };
        (groups, capped)
    }
}

/// What a session record's list left out: the count past [`PULL_LIMIT`],
/// and whether its lineage went past [`SESSION_LIMIT`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Capped {
    pub(crate) omitted: usize,
    pub(crate) lineage: bool,
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
            .pulls(Some(0), sessions[0].as_ref(), &[&recorded])
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
            .pulls(Some(1), sessions[1].as_ref(), &[&recorded])
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
        let list = sources.pulls(Some(0), None, &[]).unwrap();
        assert_eq!(numbers(&list), [(5, PrState::Merged), (4, PrState::Closed)]);
    }

    /// Two sessions holding one pull request list it once with both names;
    /// the same number in another repository is another pull request; a
    /// lineage that repeats, loops or names a gone pane still ends.
    #[test]
    fn a_shared_pull_request_is_listed_once_and_a_lineage_loop_ends() {
        let (mut rows, sessions) = rows(&[
            ("root", "s-root", &["child", "child", "gone"]),
            ("child", "s-child", &["root"]),
        ]);
        let shared = listed(42, PullRequestBadge::Open, PullRequestChecks::Failed);
        let mut foreign = listed(42, PullRequestBadge::Open, PullRequestChecks::Passing);
        foreign.url = "https://github.com/acme/other/pull/42".into();
        holds(&mut rows[0], &shared);
        holds(&mut rows[1], &shared);
        holds(&mut rows[1], &foreign);
        let github = github(vec![shared, foreign]);
        let sources = Sources::new(rows.iter(), &sessions, [&github]);
        let list = sources.pulls(Some(0), None, &[]).unwrap();
        assert_eq!(
            numbers(&list),
            [(42, PrState::Failed), (42, PrState::Mergeable)]
        );
        assert_eq!(list.pulls[0].holders, ["label root", "label child"]);
        assert!(list.pulls[0].own && !list.pulls[1].own);
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
        let list = sources.pulls(Some(0), None, &[&recorded]).unwrap();
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
                &[&Recorded::from_summaries([&summary(made)])],
            )
            .unwrap();
        let mut laid = tree.clone();
        sources.lay_groups(&mut laid, &[&Recorded::from_record(&tree)]);
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
        assert!(sources.groups_match(&laid, &[&Recorded::from_record(&tree)]));
        // The child's own record, opened from the list, groups its own subtree.
        assert_eq!(laid.children[0].pr_groups[0].pulls[0].number, 11);
        assert_eq!(
            laid.children[0].pr_groups[1].child,
            Some(session("s-grandchild"))
        );
    }

    fn read(pulls: Vec<PullRequestSnapshot>, at: u64, stale: bool) -> GithubSnapshot {
        let mut snapshot = github(pulls);
        snapshot.projects[0].status.last_success_at_unix_ms = Some(at);
        snapshot.projects[0].status.stale = stale;
        snapshot
    }

    fn urls(pulls: &[SessionPull]) -> BTreeSet<&str> {
        pulls.iter().map(|pull| pull.url.as_str()).collect()
    }

    /// The review of #1859: the reads of this machine and each device were
    /// keyed by address, so whichever came last won, and a device's older
    /// open read hid a merge this machine had just read.
    #[test]
    fn the_same_pull_request_read_on_several_machines_takes_the_fresh_latest_read_in_any_order() {
        let (mut rows, sessions) = rows(&[("agent", "s-agent", &[])]);
        let merged = listed(42, PullRequestBadge::Merged, PullRequestChecks::Passing);
        holds(&mut rows[0], &merged);
        let here = read(vec![merged], 9_000, false);
        let stale_newer = read(
            vec![listed(
                42,
                PullRequestBadge::Open,
                PullRequestChecks::Pending,
            )],
            12_000,
            true,
        );
        let fresh_older = read(
            vec![listed(
                42,
                PullRequestBadge::Open,
                PullRequestChecks::Failed,
            )],
            8_000,
            false,
        );
        let orders: [[&GithubSnapshot; 3]; 3] = [
            [&here, &stale_newer, &fresh_older],
            [&stale_newer, &fresh_older, &here],
            [&fresh_older, &here, &stale_newer],
        ];
        for order in orders {
            let sources = Sources::new(rows.iter(), &sessions, order);
            let list = sources.pulls(Some(0), None, &[]).unwrap();
            assert_eq!(
                numbers(&list),
                [(42, PrState::Merged)],
                "{order:?}",
                order = order.map(|read| read.projects[0].status.last_success_at_unix_ms)
            );
            assert!(!list.pulls[0].stale);
        }
        // An ended session's pull request takes the same read.
        let recorded =
            Recorded::from_summaries([&summary(&[("s-ended", None, "ended", &[(42, "review")])])]);
        for order in orders {
            let sources = Sources::new(rows.iter(), &sessions, order);
            let list = sources
                .pulls(None, Some(&session("s-ended")), &[&recorded])
                .unwrap();
            assert_eq!(numbers(&list), [(42, PrState::Merged)]);
        }
    }

    /// The review of #1859: every GitHub read moved the read time each pull
    /// request carried, so a list that had not changed was published again.
    #[test]
    fn github_answering_the_same_again_leaves_the_list_equal() {
        let (mut rows, sessions) = rows(&[("agent", "s-agent", &[])]);
        let open = listed(7, PullRequestBadge::Open, PullRequestChecks::Passing);
        holds(&mut rows[0], &open);
        let recorded =
            Recorded::from_summaries([&summary(&[("s-agent", None, "agent", &[(8, "merged")])])]);
        let list = |at: u64, stale: bool| {
            let github = read(
                vec![
                    open.clone(),
                    listed(8, PullRequestBadge::Merged, PullRequestChecks::Passing),
                ],
                at,
                stale,
            );
            Sources::new(rows.iter(), &sessions, [&github])
                .pulls(Some(0), None, &[&recorded])
                .unwrap()
        };
        assert_eq!(list(9_000, false), list(19_000, false));
        let stale = list(9_000, true);
        assert!(
            stale
                .pulls
                .iter()
                .all(|pull| pull.stale && pull.last_read_at_unix_ms == Some(9_000))
        );
        assert_ne!(
            stale,
            list(9_000, false),
            "a read that fails is drawn as stale"
        );
    }

    /// The review of #1859: the record cut the parent's list at the cap, then
    /// read each child's list again, so two ended children with 100 and 101
    /// merged pull requests drew 200 on the row and 201 in the record.
    #[test]
    fn the_record_lays_the_capped_list_its_row_draws() {
        let (rows, sessions) = rows(&[("lead", "s-lead", &[])]);
        let github = github(Vec::new());
        let first: Vec<(u32, &str)> = (1..=100).map(|number| (number, "merged")).collect();
        let second: Vec<(u32, &str)> = (101..=201).map(|number| (number, "merged")).collect();
        let summaries = Recorded::from_summaries([&summary(&[
            ("s-lead", None, "lead", &[]),
            ("s-first", Some("s-lead"), "first", &first),
            ("s-second", Some("s-lead"), "second", &second),
        ])]);
        let outputs = |id: &str, made: &[(u32, &str)]| {
            made.iter()
                .map(|(number, state)| output(id, *number, state))
                .collect()
        };
        let tree = record(
            "s-lead",
            "lead",
            Vec::new(),
            vec![
                record("s-first", "first", outputs("s-first", &first), Vec::new()),
                record(
                    "s-second",
                    "second",
                    outputs("s-second", &second),
                    Vec::new(),
                ),
            ],
        );
        let sources = Sources::new(rows.iter(), &sessions, [&github]);
        let row = sources
            .pulls(Some(0), sessions[0].as_ref(), &[&summaries])
            .unwrap();
        assert_eq!((row.pulls.len(), row.omitted), (PULL_LIMIT, 1));
        let mut laid = tree.clone();
        let tree_recorded = Recorded::from_record(&tree);
        let capped = sources.lay_groups(&mut laid, &[&summaries, &tree_recorded]);
        let grouped: Vec<SessionPull> = laid
            .pr_groups
            .iter()
            .flat_map(|group| group.pulls.clone())
            .collect();
        assert_eq!(urls(&grouped), urls(&row.pulls), "the same pull requests");
        assert_eq!(grouped.len(), PULL_LIMIT, "each once");
        assert_eq!(laid.pr_omitted, row.omitted);
        assert_eq!(
            capped,
            Capped {
                omitted: 1,
                lineage: false
            }
        );
        let sizes: Vec<usize> = laid
            .pr_groups
            .iter()
            .map(|group| group.pulls.len())
            .collect();
        assert_eq!(sizes, [99, 101], "the lowest number is the one left out");
        assert!(sources.groups_match(&laid, &[&summaries, &tree_recorded]));
        // Each child's own record holds its whole list: neither is past the cap.
        assert_eq!(laid.children[1].pr_groups[0].pulls.len(), 101);
        assert_eq!(laid.children[1].pr_omitted, 0);
    }

    /// A lineage past [`SESSION_LIMIT`] says so instead of ending quietly.
    #[test]
    fn a_lineage_past_the_session_limit_is_reported() {
        let ids: Vec<String> = (0..=SESSION_LIMIT)
            .map(|index| format!("s-{index}"))
            .collect();
        let chain: Vec<Kept<'_>> = ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let parent = index.checked_sub(1).map(|parent| ids[parent].as_str());
                let made: &[(u32, &str)] = if index == 0 { &[(1, "merged")] } else { &[] };
                (id.as_str(), parent, "step", made)
            })
            .collect();
        let recorded = Recorded::from_summaries([&summary(&chain)]);
        let github = github(Vec::new());
        let sources = Sources::new([], &[], [&github]);
        let list = sources
            .pulls(None, Some(&session("s-0")), &[&recorded])
            .unwrap();
        assert!(list.lineage_capped);
        let within = sources.pulls(None, Some(&session("s-1")), &[&recorded]);
        assert!(
            within.is_none(),
            "the rest of the chain made nothing and stays within the limit"
        );
    }

    /// The review of #1859: in A -> B -> C, B's 태스크 row under A showed only
    /// what B made while its pane and record showed C's too. B's one list
    /// holds both, while C works and after it ends.
    #[test]
    fn a_middle_sessions_list_holds_its_childs_pull_requests_live_and_ended() {
        let made = listed(41, PullRequestBadge::Open, PullRequestChecks::Passing);
        let childs = listed(42, PullRequestBadge::Merged, PullRequestChecks::Passing);
        let github = github(vec![made.clone(), childs.clone()]);
        let tree = record(
            "s-b",
            "b",
            Vec::new(),
            vec![record("s-c", "c", Vec::new(), Vec::new())],
        );
        let tree_recorded = Recorded::from_record(&tree);
        let check = |rows: &[SidebarAgentSnapshot],
                     sessions: &[Option<SessionIdentity>],
                     summaries: &Recorded| {
            let sources = Sources::new(rows.iter(), sessions, [&github]);
            let a = sources
                .pulls(Some(0), sessions[0].as_ref(), &[summaries])
                .unwrap();
            let b = sources
                .pulls(Some(1), sessions[1].as_ref(), &[summaries])
                .unwrap();
            assert_eq!(
                numbers(&b),
                [(41, PrState::Mergeable), (42, PrState::Merged)]
            );
            assert_eq!(urls(&a.pulls), urls(&b.pulls));
            assert!(b.pulls[0].own && !b.pulls[1].own);
            let mut laid = tree.clone();
            sources.lay_groups(&mut laid, &[summaries, &tree_recorded]);
            let grouped: Vec<(Option<&str>, u32)> = laid
                .pr_groups
                .iter()
                .flat_map(|group| {
                    group.pulls.iter().map(move |pull| {
                        (
                            group.child.as_ref().map(|child| child.session_id.as_str()),
                            pull.number,
                        )
                    })
                })
                .collect();
            assert_eq!(grouped, [(None, 41), (Some("s-c"), 42)]);
        };
        // C works.
        let (mut live, sessions) = rows(&[
            ("a", "s-a", &["b"]),
            ("b", "s-b", &["c"]),
            ("c", "s-c", &[]),
        ]);
        holds(&mut live[1], &made);
        holds(&mut live[2], &childs);
        let lineage = Recorded::from_summaries([&summary(&[
            ("s-a", None, "a", &[]),
            ("s-b", Some("s-a"), "b", &[]),
            ("s-c", Some("s-b"), "c", &[]),
        ])]);
        check(&live, &sessions, &lineage);
        // C ended: its pane is gone and the record keeps what it made.
        let (mut live, sessions) = rows(&[("a", "s-a", &["b"]), ("b", "s-b", &[])]);
        holds(&mut live[1], &made);
        let ended = Recorded::from_summaries([&summary(&[
            ("s-a", None, "a", &[]),
            ("s-b", Some("s-a"), "b", &[]),
            ("s-c", Some("s-b"), "c", &[(42, "merged")]),
        ])]);
        check(&live, &sessions, &ended);
    }
}
