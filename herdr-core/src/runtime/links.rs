//! What the runtime hands the link worker, and what it publishes back
//! (PRD link-graph D-27, D-45).
//!
//! Facts leave the lock as owned copies, and only when a cheap fingerprint
//! of them moved: this runs inside `apply_pull_requests`, which every Herdr
//! session ingest reaches. Answers come back under a short lock and change
//! the snapshot only when they differ, each on its own delta revision.

use super::*;
use crate::links::ledger::{
    LedgerUpdate, MissedStatus, SessionIdentity, SessionRecord, SessionRecordSnapshot,
    SessionSearchOptions, SessionSearchPage, SessionSearchPresence, SessionSearchSnapshot,
};
use crate::links::worker::{
    LinkClient, LinkWorker, PanelAnswer, PanelRequest, PanelTarget, Paths, Sink,
};
use crate::links::{
    IssueSource, LinkPanelSnapshot, LinkSummariesSnapshot, LinkTarget, LinkedPr, OutputFact,
    PaneFact, ParentFact, PrFact, ProjectFacts, ProjectLinkSummary, WorktreeFact,
};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

#[derive(Default)]
pub(super) struct LinksWork {
    pub(super) client: Option<LinkClient>,
    pub(super) direct_inputs: Vec<crate::model::GraphDirectInput>,
    projects_print: Option<u64>,
    panes: Arc<Vec<PaneFact>>,
    parents: Arc<Vec<ParentFact>>,
    parents_print: Option<u64>,
    generation: u64,
    record_generation: u64,
    search_generation: u64,
    pub(super) connection_epochs: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LinksOpenPayload {
    pub workspace_id: String,
    pub target: LinkTarget,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecordOpenPayload {
    pub session: SessionIdentity,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecordSearchPayload {
    pub query: String,
    #[serde(default)]
    pub options: SessionSearchOptions,
}

impl RecordSearchPayload {
    pub(super) fn bounded(&self) -> bool {
        self.query.chars().count() <= 200 && self.options.bounded()
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MissedStatusPayload {
    pub session: SessionIdentity,
    pub id: String,
    pub status: MissedStatus,
}

struct RuntimeSink {
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
}

impl RuntimeSink {
    fn apply(&self, change: impl FnOnce(&mut Runtime) -> bool) {
        let Some(runtime) = self.runtime.upgrade() else {
            return;
        };
        let changed = match runtime.lock() {
            Ok(mut guard) => change(&mut guard),
            Err(_) => return,
        };
        drop(runtime);
        if changed {
            self.notifier.notify();
        }
    }
}

impl Sink for RuntimeSink {
    fn direct_observations(&self, facts: Vec<crate::links::observation::DirectFact>) {
        let mut inputs: Vec<_> = facts.iter().filter_map(|fact| fact.graph_input()).collect();
        inputs.sort_unstable_by(|a, b| {
            b.at_unix_ms
                .cmp(&a.at_unix_ms)
                .then_with(|| b.id.cmp(&a.id))
        });
        self.apply(move |runtime| runtime.ingest_direct_observations(inputs));
    }
    fn record(&self, generation: u64, answer: Result<SessionRecord, String>) {
        self.apply(move |runtime| runtime.ingest_session_record(generation, answer));
    }
    fn session_search(&self, generation: u64, answer: Result<SessionSearchPage, String>) {
        self.apply(move |runtime| runtime.ingest_work_search(generation, answer));
    }
    fn summaries(&self, summaries: BTreeMap<String, ProjectLinkSummary>) {
        self.apply(move |runtime| runtime.ingest_link_summaries(summaries));
    }

    fn panel(&self, generation: u64, answer: Result<PanelAnswer, String>) {
        self.apply(move |runtime| runtime.ingest_link_panel(generation, answer));
    }

    fn filling(&self, filling: bool) {
        self.apply(move |runtime| runtime.ingest_link_filling(filling));
    }

    fn pruned(&self, sessions: Vec<crate::links::ledger::SessionIdentity>) {
        self.apply(move |runtime| runtime.ingest_link_pruned(sessions));
    }

    fn deleted(&self, request: String, answer: Result<(), String>) {
        self.apply(move |runtime| runtime.ingest_archive_record_deleted(request, answer));
    }

    fn devices(&self) -> Vec<(String, Arc<dyn crate::node_access::NodeLink>)> {
        self.runtime
            .upgrade()
            .and_then(|runtime| {
                runtime
                    .lock()
                    .ok()
                    .map(|runtime| runtime.ready_device_channels())
            })
            .unwrap_or_default()
    }
}

/// Starts the worker beside the runtime that owns its answers.
pub(crate) fn spawn_worker(
    runtime: &Arc<Mutex<Runtime>>,
    notifier: ChangeNotifier,
) -> Result<LinkWorker, String> {
    let (state_path, node, own) = {
        let runtime = runtime.lock().map_err(|_| "runtime lock poisoned")?;
        (
            runtime.state_path.clone(),
            runtime.node.to_string(),
            runtime.own_node(),
        )
    };
    let directory = state_path
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .ok_or("the state folder is unknown")?;
    let paths = Paths {
        store: hide_kit::layout::links_store(directory),
        search: state_path.with_file_name("session-search.sqlite3"),
        local_device: node,
    };
    let worker = LinkWorker::spawn(
        paths,
        own,
        RuntimeSink {
            runtime: Arc::downgrade(runtime),
            notifier,
        },
    )?;
    let mut guard = runtime.lock().map_err(|_| "runtime lock poisoned")?;
    guard.links_work.client = Some(worker.client());
    guard.feed_links();
    // The archive's sessions reach retention once the worker is there.
    guard.resend_archive_retention();
    guard.restore_pending_backlog();
    Ok(worker)
}

/// A session file's agent name for a Hide agent kind.
pub(super) fn session_agent(kind: &str) -> Option<&'static str> {
    hide_session::Agent::from_kind(kind).map(|agent| agent.as_str())
}

impl Runtime {
    /// The proof owner obtains a handle under the lock, then admits off-lock.
    pub fn direct_observation_client(&self) -> Option<LinkClient> {
        self.links_work.client.clone()
    }

    fn ingest_direct_observations(&mut self, inputs: Vec<crate::model::GraphDirectInput>) -> bool {
        if self.links_work.direct_inputs == inputs {
            return false;
        }
        self.links_work.direct_inputs = inputs.clone();
        let mut graph = self
            .delivery_ledger
            .as_ref()
            .map_or_else(
                |_| crate::model::OverviewGraphSnapshot::default(),
                |ledger| {
                    crate::model::OverviewGraphSnapshot::from_ledger(ledger, unix_milliseconds())
                },
            )
            .with_direct_inputs(inputs);
        graph.sessions = std::mem::take(&mut self.snapshot.overview_graph.sessions);
        let changed = self.snapshot.overview_graph != graph;
        self.snapshot.overview_graph = graph;
        changed
    }

    pub(super) fn open_session_record(&mut self, payload: RecordOpenPayload) -> bool {
        self.links_work.record_generation += 1;
        let mut record = SessionRecordSnapshot {
            session: Some(payload.session.clone()),
            loading: true,
            ..Default::default()
        };
        match self.links_work.client.as_ref() {
            Some(client) => client.read_record(self.links_work.record_generation, payload.session),
            None => {
                record.loading = false;
                record.failure = Some("links_store_unavailable".into());
            }
        }
        self.snapshot.session_record = Some(Arc::new(record));
        true
    }

    pub(super) fn close_session_record(&mut self) -> bool {
        self.links_work.record_generation += 1;
        if let Some(client) = self.links_work.client.as_ref() {
            client.close_record();
        }
        self.snapshot.session_record = Some(Arc::new(SessionRecordSnapshot::default()));
        true
    }

    pub(super) fn search_work_sessions(&mut self, payload: RecordSearchPayload) -> bool {
        self.links_work.search_generation += 1;
        let mut search = SessionSearchSnapshot {
            query: payload.query.clone(),
            options: payload.options.clone(),
            loading: true,
            ..Default::default()
        };
        // Current state is meaningful only for an exact current native
        // identity. Retained native `ended_at` and title states are historic.
        let presence = self.session_search_presence();
        if presence.len() > crate::links::SUMMARY_SESSION_LIMIT {
            search.loading = false;
            search.failure = Some("ledger_search_presence_capacity".into());
        } else {
            match self.links_work.client.as_ref() {
                Some(client) => client.search_sessions(
                    self.links_work.search_generation,
                    payload.query,
                    payload.options,
                    self.ai_settings.as_ref().is_some_and(|settings| {
                        settings.enabled && settings.chosen && settings.agent_summary
                    }),
                    presence,
                ),
                None => {
                    search.loading = false;
                    search.failure = Some("links_store_unavailable".into());
                }
            }
        }
        self.snapshot.session_work_search = Some(Arc::new(search));
        true
    }

    fn session_search_presence(&self) -> Vec<SessionSearchPresence> {
        let mut states = BTreeMap::new();
        for archive in self.snapshot.ui_state.session_archive.records.values() {
            for member in archive.held().filter(|member| member.closed) {
                let Some(kind) = session_agent(&member.kind) else {
                    continue;
                };
                states.insert(
                    SessionIdentity {
                        device_id: member.machine.clone(),
                        agent: kind.into(),
                        session_id: member.native_session_id.clone(),
                    },
                    "archived".to_owned(),
                );
            }
        }
        let sources = std::iter::once((self.node.as_str(), &self.snapshot.navigator.agents)).chain(
            self.snapshot
                .status
                .remote
                .iter()
                .filter(|remote| remote.state == "connected")
                .filter_map(|remote| {
                    remote
                        .session
                        .as_ref()
                        .map(|session| (remote.target_id.as_str(), &session.agents))
                }),
        );
        'sources: for (owner, agents) in sources {
            for agent in agents {
                let Some(session_id) = &agent.session_id else {
                    continue;
                };
                let Some(kind) = session_agent(&agent.agent_kind) else {
                    continue;
                };
                let device = owner;
                let status = if agent.state.needs_you {
                    "needs_you"
                } else if agent.state.working {
                    "working"
                } else if agent.status_code == AgentStatusCode::Done {
                    "done"
                } else {
                    "current"
                };
                states.insert(
                    SessionIdentity {
                        device_id: device.into(),
                        agent: kind.into(),
                        session_id: session_id.clone(),
                    },
                    status.into(),
                );
                if states.len() > crate::links::SUMMARY_SESSION_LIMIT {
                    break 'sources;
                }
            }
        }
        states
            .into_iter()
            .map(|(session, status)| SessionSearchPresence { session, status })
            .collect()
    }

    pub(super) fn set_missed_status(&mut self, payload: MissedStatusPayload) -> bool {
        if !self
            .ai_settings
            .as_ref()
            .is_some_and(|settings| settings.enabled && settings.agent_summary)
        {
            return false;
        }
        let Some(client) = self.links_work.client.as_ref() else {
            return false;
        };
        if let Err(code) = client.ledger(LedgerUpdate::MissedStatus {
            session: payload.session.clone(),
            id: payload.id,
            status: payload.status,
        }) {
            self.snapshot.session_record = Some(Arc::new(SessionRecordSnapshot {
                session: self
                    .snapshot
                    .session_record
                    .as_ref()
                    .and_then(|held| held.session.clone()),
                failure: Some(code),
                ..Default::default()
            }));
            return true;
        }
        false
    }

    fn ingest_session_record(
        &mut self,
        generation: u64,
        answer: Result<SessionRecord, String>,
    ) -> bool {
        if generation != self.links_work.record_generation {
            return false;
        }
        let Some(held) = self.snapshot.session_record.as_ref() else {
            return false;
        };
        let Some(session) = held.session.clone() else {
            return false;
        };
        let mut snapshot = SessionRecordSnapshot {
            session: Some(session),
            ..Default::default()
        };
        match answer {
            Ok(record) => snapshot.record = Some(record),
            Err(code) => snapshot.failure = Some(code),
        }
        self.snapshot.session_record = Some(Arc::new(snapshot));
        true
    }

    fn ingest_work_search(
        &mut self,
        generation: u64,
        answer: Result<SessionSearchPage, String>,
    ) -> bool {
        if generation != self.links_work.search_generation {
            return false;
        }
        let Some(held) = self.snapshot.session_work_search.as_ref() else {
            return false;
        };
        let mut snapshot = SessionSearchSnapshot {
            query: held.query.clone(),
            options: held.options.clone(),
            ..Default::default()
        };
        match answer {
            Ok(page) => {
                snapshot.results = page.results;
                snapshot.next_cursor = page.next_cursor;
            }
            Err(code) => snapshot.failure = Some(code),
        }
        self.snapshot.session_work_search = Some(Arc::new(snapshot));
        true
    }
    /// Hands the worker the projects and the panes' sessions when either
    /// moved since the last time.
    pub(super) fn feed_links(&mut self) {
        let Some(client) = self.links_work.client.clone() else {
            return;
        };
        let print = self.link_projects_print();
        if self.links_work.projects_print != Some(print) {
            self.links_work.projects_print = Some(print);
            client.projects(Arc::new(self.link_projects()));
        }
        let panes = self.link_panes();
        if *self.links_work.panes != panes {
            self.links_work.panes = Arc::new(panes);
            client.panes(Arc::clone(&self.links_work.panes));
        }
        self.feed_link_parents();
    }

    /// What the project facts are built from, without building them.
    fn link_projects_print(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for workspace in self.catalog_workspaces() {
            (&workspace.id, &workspace.path, &workspace.device_id).hash(&mut hasher);
            self.links_work
                .connection_epochs
                .get(&hide_project::project_id(
                    &workspace.device_id,
                    Path::new(&workspace.path),
                ))
                .hash(&mut hasher);
            for checkout in &workspace.checkouts {
                (
                    &checkout.path,
                    &checkout.branch,
                    &checkout.task_key,
                    checkout
                        .worktree
                        .as_ref()
                        .and_then(|w| w.created_at_unix_ms),
                )
                    .hash(&mut hasher);
            }
            if let Some(project) = self.catalog_github_project(workspace) {
                (
                    project.pull_requests_read,
                    project.status.last_success_at_unix_ms,
                    project.status.stale,
                    project.pull_requests.len(),
                    &project.issues.repository,
                    &project.issues.repository_id,
                )
                    .hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    pub(super) fn link_projects(&self) -> Vec<ProjectFacts> {
        self.catalog_workspaces()
            .filter(|workspace| workspace.is_git && !workspace.is_home && !workspace.temporary)
            .map(|workspace| {
                let project = self.catalog_github_project(workspace);
                let prs = project
                    .map(|project| {
                        project
                            .pull_requests
                            .iter()
                            .filter_map(|pr| link_pr(pr, workspace))
                            .collect()
                    })
                    .unwrap_or_default();
                ProjectFacts {
                    key: hide_project::project_id(&workspace.device_id, Path::new(&workspace.path)),
                    device_id: workspace.device_id.clone(),
                    workspace_id: workspace.id.clone(),
                    root: workspace.path.clone(),
                    repository: project
                        .and_then(|project| project.issues.repository.as_deref())
                        .map(str::to_ascii_lowercase),
                    repository_id: project.and_then(|project| project.issues.repository_id.clone()),
                    connection_epoch: self
                        .links_work
                        .connection_epochs
                        .get(&hide_project::project_id(
                            &workspace.device_id,
                            Path::new(&workspace.path),
                        ))
                        .copied()
                        .unwrap_or(0),
                    worktrees: workspace
                        .checkouts
                        .iter()
                        .map(|checkout| WorktreeFact {
                            path: checkout.path.clone(),
                            branch: checkout.branch.clone(),
                            created_at_unix_ms: checkout
                                .worktree
                                .as_ref()
                                .and_then(|worktree| worktree.created_at_unix_ms),
                        })
                        .collect(),
                    prs,
                    prs_read: project.is_some_and(|project| project.pull_requests_read),
                    outputs: project
                        .map(|project| {
                            let mut outputs = project
                                .pull_requests
                                .iter()
                                .filter_map(|pr| {
                                    Some(OutputFact {
                                        kind: "pr".into(),
                                        repository: crate::links::repository_of(&pr.url)?,
                                        number: u64::from(pr.number),
                                        title: pr.title.clone(),
                                        url: pr.url.clone(),
                                        state:
                                            if pr.badge == crate::model::PullRequestBadge::Merged {
                                                "merged"
                                            } else if pr.badge
                                                == crate::model::PullRequestBadge::Closed
                                            {
                                                "closed"
                                            } else if pr.checks
                                                == crate::model::PullRequestChecks::Failed
                                            {
                                                "ci_failed"
                                            } else {
                                                "review"
                                            }
                                            .into(),
                                        created_at: pr.created_at_unix_ms,
                                        stale: project.status.stale,
                                        last_read_at: project.status.last_success_at_unix_ms,
                                    })
                                })
                                .collect::<Vec<_>>();
                            outputs.extend(project.issues.issues.iter().map(|issue| OutputFact {
                                kind: "issue".into(),
                                repository: issue.reference.repository.clone(),
                                number: u64::from(issue.reference.number),
                                title: issue.title.clone(),
                                url: issue.url.clone(),
                                state: issue.state.to_ascii_lowercase(),
                                created_at: issue.created_at_unix_ms,
                                stale: project.status.stale,
                                last_read_at: project.status.last_success_at_unix_ms,
                            }));
                            outputs
                        })
                        .unwrap_or_default(),
                }
            })
            .collect()
    }

    /// The agent sessions Hide panes carry now, with each pane's folder and
    /// its checkout's branch.
    pub(super) fn link_panes(&self) -> Vec<PaneFact> {
        let agents = self
            .link_lineage_agents()
            .filter_map(|agent| {
                let session = agent.session_id.as_deref()?;
                let kind = session_agent(&agent.agent_kind)?;
                Some((agent.pane_id.as_str(), (kind, session)))
            })
            .collect::<HashMap<_, _>>();
        let mut panes = Vec::new();
        for workspace in self.catalog_workspaces() {
            for checkout in &workspace.checkouts {
                for pane in checkout.tabs.iter().flat_map(|tab| tab.panes.iter()) {
                    let Some((kind, session)) = agents.get(pane.id.as_str()) else {
                        continue;
                    };
                    panes.push(PaneFact {
                        device_id: workspace.device_id.clone(),
                        agent: (*kind).to_owned(),
                        session_id: (*session).to_owned(),
                        cwd: if pane.cwd.is_empty() {
                            checkout.path.clone()
                        } else {
                            pane.cwd.clone()
                        },
                        branch: checkout
                            .branch
                            .clone()
                            .filter(|branch| hide_session::links::is_branch(branch)),
                    });
                }
            }
        }
        panes.sort_by(|left, right| {
            (&left.device_id, &left.agent, &left.session_id).cmp(&(
                &right.device_id,
                &right.agent,
                &right.session_id,
            ))
        });
        panes.dedup_by(|left, right| {
            left.device_id == right.device_id
                && left.agent == right.agent
                && left.session_id == right.session_id
        });
        panes
    }

    /// Registered lineage survives pane removal. Current resolved lineage
    /// also supplies parents learned outside a delivery registration.
    pub(super) fn feed_link_parents(&mut self) {
        let Some(client) = self.links_work.client.clone() else {
            return;
        };
        let print = self.link_parents_print();
        if self.links_work.parents_print == Some(print) {
            return;
        }
        // The first facts go out even when empty: the links worker holds a
        // descendant letter's turn until it has them.
        let first = self.links_work.parents_print.is_none();
        self.links_work.parents_print = Some(print);
        let parents = self.link_parent_facts();
        if first || *self.links_work.parents != parents {
            self.links_work.parents = Arc::new(parents);
            client.parents(Arc::clone(&self.links_work.parents));
        }
    }

    /// Hash borrowed authority facts before allocating a lineage projection.
    /// A terminal frame or status-only change does not rebuild its maps.
    pub(super) fn link_parents_print(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for workspace in self.catalog_workspaces() {
            workspace.device_id.hash(&mut hasher);
            for checkout in &workspace.checkouts {
                for tab in &checkout.tabs {
                    for pane in &tab.panes {
                        pane.id.hash(&mut hasher);
                    }
                }
            }
        }
        for agent in self.link_lineage_agents() {
            (
                &agent.pane_id,
                &agent.agent_kind,
                &agent.session_id,
                &agent.lineage_parent_pane_id,
                &agent.herdr_name,
            )
                .hash(&mut hasher);
        }
        if let Ok(ledger) = self.delivery_ledger.as_ref() {
            // A new letter may be a child's report (`link_parent_facts`);
            // every letter takes an id, so the next one moves the print.
            ledger.next_id.hash(&mut hasher);
            for record in &ledger.agents {
                (
                    &record.id,
                    &record.parent,
                    &record.actor.device_id,
                    &record.actor.kind,
                    &record.session,
                    record.wire_name(),
                )
                    .hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    fn link_lineage_agents(&self) -> impl Iterator<Item = &SidebarAgentSnapshot> {
        self.snapshot.navigator.agents.iter().chain(
            self.snapshot
                .status
                .remote
                .iter()
                .filter_map(|remote| remote.session.as_ref())
                .flat_map(|session| &session.agents),
        )
    }

    /// What GitHub last answered for a catalog project: this machine's
    /// reader's for its own, and a node's read (`device_github`) for one of
    /// a node's; a device this core does not read has none.
    pub(super) fn catalog_github_project(
        &self,
        workspace: &WorkspaceSnapshot,
    ) -> Option<&crate::model::GithubProjectSnapshot> {
        if workspace.device_id == self.node.as_str() {
            self.github.project(&workspace.path)
        } else {
            self.device_github
                .get(&workspace.device_id)?
                .project(&workspace.path)
        }
    }

    pub(super) fn link_parent_facts(&self) -> Vec<ParentFact> {
        let devices = self
            .catalog_workspaces()
            .flat_map(|workspace| {
                workspace.checkouts.iter().flat_map(|checkout| {
                    checkout.tabs.iter().flat_map(|tab| {
                        tab.panes
                            .iter()
                            .map(|pane| (pane.id.as_str(), workspace.device_id.as_str()))
                    })
                })
            })
            .collect::<HashMap<_, _>>();
        let by_pane = self
            .link_lineage_agents()
            .map(|agent| (agent.pane_id.as_str(), agent))
            .collect::<HashMap<_, _>>();
        let wire_name = |agent: &SidebarAgentSnapshot, device: &str| {
            let pane = if device == self.node.as_str() {
                agent.pane_id.clone()
            } else {
                format!("remote:{device}:pane:{}", agent.pane_id)
            };
            match (
                self.delivery_observations.get(&pane),
                self.delivery_ledger.as_ref(),
            ) {
                (Some(observed), Ok(ledger)) => {
                    crate::coordination::wire_name(ledger, &observed.actor)
                }
                (Some(observed), Err(_)) => observed.actor.name.clone(),
                // Delivery never saw this pane, so no letter names it.
                (None, _) => agent.pane_id.clone(),
            }
        };
        let mut parents = self
            .link_lineage_agents()
            .filter_map(|agent| {
                let parent = by_pane.get(agent.lineage_parent_pane_id.as_deref()?)?;
                Some(ParentFact {
                    device_id: (*devices.get(agent.pane_id.as_str())?).to_owned(),
                    agent: session_agent(&agent.agent_kind)?.to_owned(),
                    session_id: agent.session_id.clone()?,
                    parent_device_id: (*devices.get(parent.pane_id.as_str())?).to_owned(),
                    parent_agent: session_agent(&parent.agent_kind)?.to_owned(),
                    parent_session_id: parent.session_id.clone()?,
                    parent_name: wire_name(parent, devices.get(parent.pane_id.as_str())?),
                })
            })
            .map(|parent| {
                (
                    (
                        parent.device_id.clone(),
                        parent.agent.clone(),
                        parent.session_id.clone(),
                    ),
                    parent,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return parents.into_values().collect();
        };
        let records = ledger
            .agents
            .iter()
            .map(|record| (record.id.as_str(), record))
            .collect::<HashMap<_, _>>();
        let registered = ledger
            .agents
            .iter()
            .filter_map(|record| {
                let parent = records.get(record.parent.as_deref()?)?;
                if record.session.is_empty() || parent.session.is_empty() {
                    return None;
                }
                Some(ParentFact {
                    device_id: record.actor.device_id.clone(),
                    agent: session_agent(&record.actor.kind)?.to_owned(),
                    session_id: record.session.clone(),
                    parent_device_id: parent.actor.device_id.clone(),
                    parent_agent: session_agent(&parent.actor.kind)?.to_owned(),
                    parent_session_id: parent.session.clone(),
                    parent_name: parent.wire_name().to_owned(),
                })
            })
            .collect::<Vec<_>>();
        for registered in registered {
            parents.insert(
                (
                    registered.device_id.clone(),
                    registered.agent.clone(),
                    registered.session_id.clone(),
                ),
                registered,
            );
        }
        parents.into_values().collect()
    }

    /// The Projects `hide links` may read, this machine's and each device's
    /// (the record keys them by device), and which one holds the caller's
    /// checkout; none when the record has no home.
    pub(crate) fn links_scope(
        &self,
        context: &crate::workspace_control::Context,
    ) -> Option<crate::links::query::Scope> {
        let directory = self
            .state_path
            .parent()
            .filter(|directory| !directory.as_os_str().is_empty())?;
        let projects = self
            .catalog_workspaces()
            .filter(|workspace| workspace.is_git && !workspace.is_home && !workspace.temporary)
            .map(|workspace| crate::links::query::ScopeProject {
                key: hide_project::project_id(&workspace.device_id, Path::new(&workspace.path)),
                workspace_id: workspace.id.clone(),
                label: workspace.label.clone(),
                root: workspace.path.clone(),
                device_id: workspace.device_id.clone(),
                checkouts: workspace
                    .checkouts
                    .iter()
                    .map(|checkout| checkout.path.clone())
                    .collect(),
            })
            .collect::<Vec<_>>();
        let caller = self
            .catalog_workspaces()
            .find(|workspace| {
                workspace.device_id == context.device_id
                    && (workspace.id == context.workspace_id
                        || workspace
                            .checkouts
                            .iter()
                            .any(|checkout| checkout.path == context.checkout_path))
            })
            .map(|workspace| {
                hide_project::project_id(&workspace.device_id, Path::new(&workspace.path))
            })
            .unwrap_or_default();
        Some(crate::links::query::Scope {
            store: hide_kit::layout::links_store(directory),
            local_device: self.node.to_string(),
            caller,
            projects,
        })
    }

    /// `links_open`: reads the record for one pull request or issue of a
    /// project into the `link_panel` section; reading the same one again is
    /// the failure line's Retry (B25).
    pub(super) fn open_links(&mut self, payload: LinksOpenPayload) -> bool {
        let Some(workspace) = self
            .catalog_workspaces()
            .find(|workspace| workspace.id == payload.workspace_id)
        else {
            return false;
        };
        let key = hide_project::project_id(&workspace.device_id, Path::new(&workspace.path));
        self.links_work.generation += 1;
        let generation = self.links_work.generation;
        let same = self.snapshot.link_panel.as_ref().is_some_and(|panel| {
            panel.workspace_id == payload.workspace_id && panel.target == payload.target
        });
        let mut panel = match self.snapshot.link_panel.take() {
            // What the same target showed stays while it is read again (B24).
            Some(panel) if same => panel,
            _ => LinkPanelSnapshot {
                workspace_id: payload.workspace_id.clone(),
                target: payload.target.clone(),
                ..LinkPanelSnapshot::default()
            },
        };
        panel.loading = true;
        panel.failure = None;
        let Some(client) = self.links_work.client.as_ref() else {
            panel.loading = false;
            panel.failure = Some("links_worker_unavailable".to_owned());
            self.snapshot.link_panel = Some(panel);
            return true;
        };
        client.read(PanelRequest {
            generation,
            project: key,
            target: match &payload.target {
                LinkTarget::Pr { number } => PanelTarget::Pr(*number),
                LinkTarget::Issue { key } => PanelTarget::Issue(key.clone()),
            },
        });
        self.snapshot.link_panel = Some(panel);
        true
    }

    #[cfg(test)]
    pub(super) fn links_generation(&self) -> u64 {
        self.links_work.generation
    }

    /// `links_close`: the panel closed, so its read stops following writes.
    pub(super) fn close_links(&mut self) -> bool {
        self.links_work.generation += 1;
        if let Some(client) = self.links_work.client.as_ref() {
            client.close_panel();
        }
        false
    }

    pub(super) fn ingest_link_panel(
        &mut self,
        generation: u64,
        answer: Result<PanelAnswer, String>,
    ) -> bool {
        if generation != self.links_work.generation {
            return false;
        }
        let Some(held) = self.snapshot.link_panel.as_ref() else {
            return false;
        };
        let mut panel = LinkPanelSnapshot {
            workspace_id: held.workspace_id.clone(),
            target: held.target.clone(),
            ..LinkPanelSnapshot::default()
        };
        match answer {
            Err(code) => {
                // The lines read before stay under the failure line.
                panel.pr = held.pr.clone();
                panel.prs = held.prs.clone();
                panel.sessions = held.sessions.clone();
                panel.total = held.total;
                panel.failure = Some(code);
            }
            Ok(PanelAnswer::Pr(links)) => {
                if let Some(links) = links {
                    panel.pr = Some(LinkedPr {
                        state: crate::model::PrState::of(
                            links.pr.badge(),
                            false,
                            crate::model::PullRequestChecks::Unknown,
                            None,
                        ),
                        number: links.pr.number,
                        branch: links.pr.branch,
                        title: links.pr.title,
                        url: links.pr.url,
                        created_at_unix_ms: links.pr.created_at,
                        closed_at_unix_ms: links.pr.closed_at,
                        merged_at_unix_ms: links.pr.merged_at,
                        issues: links.issues,
                        worktrees: links.worktrees,
                    });
                    panel.sessions = links.sessions;
                    panel.total = links.total;
                }
            }
            Ok(PanelAnswer::Issue(links)) => {
                panel.prs = links.prs;
                panel.sessions = links.sessions;
                panel.total = links.total;
            }
        }
        if self.snapshot.link_panel.as_ref() == Some(&panel) {
            return false;
        }
        self.snapshot.link_panel = Some(panel);
        true
    }

    pub(super) fn ingest_link_summaries(
        &mut self,
        projects: BTreeMap<String, ProjectLinkSummary>,
    ) -> bool {
        for workspace in &self.snapshot.navigator.workspaces {
            let before = self
                .snapshot
                .link_summaries
                .as_ref()
                .and_then(|summaries| summaries.projects.get(&workspace.id));
            let after = projects.get(&workspace.id);
            if before.map(|summary| &summary.record_issues)
                != after.map(|summary| &summary.record_issues)
            {
                let generation = self
                    .github_generations
                    .entry(workspace.path.clone())
                    .or_default();
                *generation = generation.wrapping_add(1);
            }
        }
        let summaries = self
            .snapshot
            .link_summaries
            .get_or_insert_with(LinkSummariesSnapshot::default);
        if summaries.projects == projects {
            return false;
        }
        for workspace in &mut self.snapshot.navigator.workspaces {
            associate_landed(workspace, projects.get(&workspace.id));
            associate_pr_sessions(&mut workspace.pull_requests, projects.get(&workspace.id));
        }
        summaries.projects = projects;
        self.refresh_inactive_groups();
        self.refresh_agent_scopes();
        true
    }

    pub(super) fn ingest_link_filling(&mut self, filling: bool) -> bool {
        let summaries = self
            .snapshot
            .link_summaries
            .get_or_insert_with(LinkSummariesSnapshot::default);
        if summaries.filling == filling {
            return false;
        }
        summaries.filling = filling;
        true
    }
}

/// Decides each checkout's `landed` from Git and the project's record: its
/// HEAD is in the base, and the record names it among the checkouts whose
/// work landed. With no record (still filling, Copied history Off, a device
/// with no pull requests read) nothing is landed, so a checkout with no
/// commits of its own is never drawn as merged. Runs on every catalog
/// projection, so it compares before it writes.
pub(super) fn associate_landed(
    workspace: &mut WorkspaceSnapshot,
    summary: Option<&ProjectLinkSummary>,
) -> bool {
    let mut changed = false;
    for checkout in &mut workspace.checkouts {
        let landed = checkout
            .worktree
            .as_ref()
            .is_some_and(|worktree| worktree.merged == Some(true))
            && summary.is_some_and(|summary| summary.landed.contains(&checkout.path));
        if checkout.landed != landed {
            checkout.landed = landed;
            changed = true;
        }
    }
    changed
}

/// Attaches to each pull request the sessions the link record says made or
/// worked on it and the checkouts they ran in: what the PRs tool's list
/// reads to find a pull request's live agents (PRD work-tab-pr-lens D-09).
pub(super) fn associate_pr_sessions(
    pull_requests: &mut [crate::model::PullRequestSnapshot],
    summary: Option<&ProjectLinkSummary>,
) -> bool {
    let mut changed = false;
    for pull_request in pull_requests {
        let number = u64::from(pull_request.number);
        let sessions: Vec<String> = summary
            .and_then(|summary| summary.pr_workers.get(&number))
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let checkouts: Vec<String> = summary
            .and_then(|summary| summary.pr_checkouts.get(&number))
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        if pull_request.work.sessions != sessions || pull_request.work.checkouts != checkouts {
            pull_request.work.sessions = sessions;
            pull_request.work.checkouts = checkouts;
            changed = true;
        }
    }
    changed
}

/// A pull request GitHub answered, with the issues it is linked to now.
fn link_pr(
    pr: &crate::model::PullRequestSnapshot,
    workspace: &WorkspaceSnapshot,
) -> Option<PrFact> {
    let repository = crate::links::repository_of(&pr.url)?;
    if pr.cross_repository || !hide_session::links::is_branch(&pr.head_branch) {
        return None;
    }
    let mut issues = pr
        .closing_issues
        .iter()
        .map(|reference| (crate::tasks::github_key(reference), IssueSource::Closes))
        .collect::<Vec<_>>();
    let checkout = workspace
        .checkouts
        .iter()
        .find(|checkout| checkout.branch.as_deref() == Some(pr.head_branch.as_str()));
    if let Some(key) = checkout.and_then(|checkout| checkout.task_key.clone())
        && !issues.iter().any(|(held, _)| *held == key)
    {
        issues.push((key, IssueSource::Hide));
    }
    Some(PrFact {
        repository,
        number: u64::from(pr.number),
        branch: pr.head_branch.clone(),
        title: pr.title.clone(),
        url: pr.url.clone(),
        created_at: pr.created_at_unix_ms,
        closed_at: pr.closed_at_unix_ms,
        merged_at: pr.merged_at_unix_ms,
        issues,
        hide_issue_known: checkout.is_some(),
    })
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn retained_history_keeps_its_native_search_presence() {
        let mut runtime = crate::runtime::tests::runtime();
        let payload = crate::sidebar::owned_label_fixture(serde_json::json!({
            "agents": [{
                "pane_id":"w1:p1", "workspace_label":"History", "agent":"cursor",
                "agent_status":"idle",
                "tokens":{"status_idle":"○", "activity":"000000000000"}
            }]
        }))
        .unwrap();
        let mut agents = crate::sidebar::project_agents(payload).agents;
        agents[0].session_id = Some("retained-native-id".into());
        runtime.snapshot.navigator.agents = agents;
        let presence = runtime.session_search_presence();
        assert_eq!(presence.len(), 1);
        assert_eq!(presence[0].session.agent, "cursor");
        assert_eq!(presence[0].session.session_id, "retained-native-id");
        assert!(session_agent("unknown-reader").is_none());
    }

    #[test]
    fn history_late_search_answer_cannot_replace_the_new_request_or_its_cursor() {
        let mut runtime = crate::runtime::tests::runtime();
        runtime.links_work.client = Some(LinkClient::full_mailbox_for_test());
        let send = |runtime: &mut Runtime, request: &str, query: &str| {
            runtime.dispatch_json(&serde_json::to_vec(&serde_json::json!({
                "schema_version": 2, "kind": "session_work_search",
                "payload": { "query": query, "options": { "request_id": request, "device_id": "remote" } }
            })).unwrap());
        };
        send(&mut runtime, "history-first", "old");
        let first = runtime.links_work.search_generation;
        send(&mut runtime, "history-second", "new");
        assert!(!runtime.ingest_work_search(first, Err("late_read_failure".into())));
        let pending = runtime.snapshot.session_work_search.as_ref().unwrap();
        assert_eq!(
            pending.options.request_id.as_deref(),
            Some("history-second")
        );
        assert!(pending.loading);
        let cursor = crate::links::ledger::SessionSearchCursor {
            at_unix_ms: 123,
            device_id: "remote".into(),
            agent: "codex".into(),
            session_id: "reused-id".into(),
        };
        let page = SessionSearchPage {
            results: vec![],
            next_cursor: Some(cursor.clone()),
        };
        assert!(runtime.ingest_work_search(runtime.links_work.search_generation, Ok(page)));
        let ready = runtime.snapshot.session_work_search.as_ref().unwrap();
        assert!(!ready.loading);
        assert_eq!(ready.query, "new");
        assert_eq!(ready.options.request_id.as_deref(), Some("history-second"));
        assert_eq!(ready.next_cursor, Some(cursor));
        assert!(ready.failure.is_none());
    }

    #[test]
    fn history_event_accepts_legacy_queries_and_rejects_unknown_or_oversized_options() {
        let event = |payload| {
            serde_json::to_vec(&serde_json::json!({ "schema_version": 2, "kind": "session_work_search", "payload": payload })).unwrap()
        };
        assert!(crate::runtime::events::decode(&event(serde_json::json!({"query":""}))).is_ok());
        for payload in [
            serde_json::json!({"query":"x", "options":{"unknown":true}}),
            serde_json::json!({"query":"x", "options":{"status":"historically_working"}}),
            serde_json::json!({"query":"x", "options":{"request_id":"x".repeat(129)}}),
            serde_json::json!({"query":"x".repeat(201)}),
        ] {
            assert!(crate::runtime::events::decode(&event(payload)).is_err());
        }
    }
}
