use super::*;

#[derive(Default)]
pub(super) struct IssueDetailWork {
    running: bool,
    generation: u64,
    pending: Option<(u64, IssueDetailRead)>,
}

/// Core-resolved identity retained while a detail read waits or runs.
#[derive(Clone)]
struct IssueDetailRead {
    workspace_id: String,
    device_id: String,
    task_key: String,
    after: Option<String>,
}
use crate::issues::{ISSUE_LIMIT, IssueCandidate, IssueLinkSnapshot, IssueReference};

pub(super) struct UnconfirmedIssueToken {
    pub value: String,
    pub observed: bool,
}

/// Most repositories one GitHub search covers: each is one more `--repo` on
/// the two `gh search` calls, and the cap bounds the projects resolved too.
const SEARCH_REPOSITORY_LIMIT: usize = crate::github::SEARCH_REPOSITORY_LIMIT;

/// Longest request id a search echoes back.
const SEARCH_REQUEST_ID_LIMIT: usize = 128;

/// What the web is told when a search fails; the real reason is in the log.
const SEARCH_FAILED_MESSAGE: &str = "GitHub 검색에 실패했습니다.";

pub(super) struct SearchRequest {
    pub(super) request_id: String,
    pub(super) query: String,
}

pub(super) struct ActiveSearch {
    /// Which worker this is, so only its answer is taken as the running
    /// search's (`GithubSearchWork::runs`).
    pub(super) run: u64,
    pub(super) invalidated: bool,
    /// The latest request the worker answers: a request for the same query
    /// joins it rather than starting another.
    pub(super) request: SearchRequest,
    /// Said beside a `ready` answer, when the search did not cover everything.
    pub(super) note: Option<String>,
}

/// The `github_search` in flight (PRD cmdk-navigation): at most one worker,
/// and at most one request waiting for it, the latest.
#[derive(Default)]
pub(super) struct GithubSearchWork {
    pub(super) runs: u64,
    pub(super) active: Option<ActiveSearch>,
    pub(super) pending: Option<SearchRequest>,
}

impl Runtime {
    /// Pure projection over the accepted catalog and metadata. No reader is
    /// scheduled unless the chosen identity changes or a user refreshes it.
    pub(super) fn sync_issues(&mut self) -> bool {
        let mut candidates = BTreeMap::new();
        let mut refresh = BTreeSet::new();
        let mut changed = false;

        let sessions: BTreeMap<_, _> = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| {
                workspace.checkouts.iter().filter_map(|checkout| {
                    workspace::authoritative_session_space(
                        &self.last_session_spaces,
                        workspace,
                        &checkout.path,
                    )
                    .map(|space| (checkout.id.clone(), space.id.clone()))
                })
            })
            .collect();
        let spaces = &self.last_session_spaces;
        settle_unconfirmed(
            &mut self.unconfirmed_issue_tokens,
            &self.issue_tokens.workspaces,
            |id| spaces.iter().any(|space| space.id == id),
        );
        let links = LinkTokens::new(
            &self.issue_tokens,
            Some(&self.unconfirmed_issue_tokens),
            &self.snapshot.navigator.agents,
            &self.invalid_links,
        );
        let settings = self.snapshot.ui_state.project_settings.clone();
        let sources = self.snapshot.ui_state.project_issue_sources.clone();
        let mut local_links = BTreeMap::new();
        for workspace in &mut self.snapshot.navigator.workspaces {
            if workspace.remote_target_id.is_some() || !workspace.is_git {
                continue;
            }
            let status = workspace
                .checkouts
                .first()
                .map(|checkout| checkout.github.clone())
                .unwrap_or_default();
            let choice = crate::project_settings::issue_choice(
                &settings,
                &sources,
                self.node.as_str(),
                &workspace.path,
            );
            // The old preference only disambiguates old bare-number links.
            // It never suppresses either source's rows.
            let local_source = crate::tasks::source_kind(
                choice,
                workspace.is_git,
                &workspace.home_issues,
                &status,
            ) == crate::tasks::SourceKind::Local;
            let mut linked = BTreeSet::new();
            for checkout in &mut workspace.checkouts {
                if self
                    .issue_write_pending
                    .as_ref()
                    .is_some_and(|pending| pending.checkout_id == checkout.id)
                {
                    if let Some(candidate) = self.issue_candidates.get(&checkout.id) {
                        candidates.insert(checkout.id.clone(), candidate.clone());
                    }
                    continue;
                }
                let repository = workspace.home_issues.repository.as_deref();
                let scope =
                    crate::project_settings::get(&settings, self.node.as_str(), &workspace.path);
                let session = sessions.get(&checkout.id).map(String::as_str);
                let candidate =
                    match links.checkout_link(session, checkout, local_source, repository, scope) {
                        // A Local project's link is the same chain read as a
                        // local id; it names no GitHub issue.
                        Some(ChainLink::Local(number)) => {
                            local_links.insert(checkout.id.clone(), number);
                            None
                        }
                        Some(ChainLink::Github(candidate)) => Some(candidate),
                        None => None,
                    };
                let issue = candidate.as_ref().and_then(|candidate| {
                    linked.insert(candidate.reference.clone());
                    if linked.len() > ISSUE_LIMIT {
                        return None;
                    }
                    workspace
                        .home_issues
                        .issues
                        .iter()
                        .find(|issue| issue.reference == candidate.reference)
                        .map(|issue| IssueLinkSnapshot {
                            issue: issue.clone(),
                            source: candidate.source.clone(),
                        })
                });
                if let Some(candidate) = candidate {
                    if self.issue_candidates.get(&checkout.id) != Some(&candidate) {
                        refresh.insert(workspace.path.clone());
                    }
                    candidates.insert(checkout.id.clone(), candidate);
                }
                if checkout.issue != issue {
                    checkout.issue = issue;
                    changed = true;
                }
            }
            if linked.len() > ISSUE_LIMIT && !workspace.home_issues.overflow {
                workspace.home_issues.overflow = true;
                changed = true;
            }
        }
        self.issue_candidates = candidates;
        self.local_issue_links = local_links;
        for project in refresh {
            self.refresh_pull_requests(&project);
        }
        changed
    }
}

impl Runtime {
    /// Projects each local project's issues, from the source it reads
    /// (`tasks::source_kind`), into the source-neutral task list the web
    /// reads, and names each checkout's task by its key. It runs after
    /// `sync_issues`, so a checkout's task is the issue it links to.
    pub(super) fn sync_tasks(&mut self) -> bool {
        let mut changed = false;
        let sources = &self.snapshot.ui_state.project_issue_sources;
        let settings = &self.snapshot.ui_state.project_settings;
        let store = &self.local_issues;
        for workspace in &mut self.snapshot.navigator.workspaces {
            if workspace.remote_target_id.is_some() {
                // A device's projects are not this machine's; a node's are
                // projected with its session (`project_node_issues`).
                continue;
            }
            let choice = crate::project_settings::issue_choice(
                settings,
                sources,
                self.node.as_str(),
                &workspace.path,
            );
            changed |= project_tasks(
                workspace,
                None,
                choice,
                store,
                None,
                &self.local_issue_links,
            );
        }
        changed
    }

    pub(super) fn set_checkout_issue(&mut self, payload: SetCheckoutPurposePayload) -> bool {
        match self.link_checkout_issue(&payload.checkout_id, &payload.text) {
            Ok(()) => true,
            Err((kind, message)) => {
                self.push_diagnostic(kind, message);
                false
            }
        }
    }

    /// A checkout an issue can be linked to: a linked worktree on a branch,
    /// in one of this machine's Git projects or one of a node that dials this
    /// core, with the node it is on.
    fn issue_checkout(
        &self,
        checkout_id: &str,
    ) -> Option<(Option<String>, WorkspaceSnapshot, CheckoutSnapshot)> {
        let find = |workspace: &WorkspaceSnapshot| {
            workspace
                .checkouts
                .iter()
                .find(|checkout| {
                    checkout.id == checkout_id && checkout.is_worktree && checkout.branch.is_some()
                })
                .map(|checkout| (workspace.clone(), checkout.clone()))
        };
        let own = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none() && workspace.is_git)
            .find_map(find)
            .map(|(workspace, checkout)| (None, workspace, checkout));
        own.or_else(|| {
            self.node_sessions().find_map(|(node, session)| {
                let (workspace, checkout) = session
                    .workspaces
                    .iter()
                    .filter(|workspace| workspace.is_git)
                    .find_map(find)?;
                Some((Some(node.to_owned()), workspace, checkout))
            })
        })
    }

    /// The link Hide keeps between a checkout and an issue: the branch's issue
    /// link and its Workspace token, written on a worker on the checkout's
    /// own machine: this machine's git and Herdr, or a node's through its
    /// link and its Herdr. `Err` names the diagnostic kind and why nothing
    /// was started; the write's own answer lands in
    /// `ingest_issue_operation_result`.
    pub(super) fn link_checkout_issue(
        &mut self,
        checkout_id: &str,
        text: &str,
    ) -> Result<(), (&'static str, String)> {
        let Some((node, workspace, checkout)) = self.issue_checkout(checkout_id) else {
            return Err((
                "checkout_issue.target_missing",
                "Issue target checkout is unavailable".to_owned(),
            ));
        };
        let local_source = text.trim().starts_with("L-")
            || text.trim().starts_with("l-")
            || (self.project_source_kind(&workspace) == crate::tasks::SourceKind::Local
                && IssueReference::parse(text, None).is_err());
        let text = if text.trim().is_empty() {
            String::new()
        } else if local_source {
            let known = crate::local_issues::parse_id(text).filter(|number| {
                matches!(&self.local_issues, Ok(store) if store.issue(node.as_deref(), &workspace.path, *number).is_some())
            });
            match known {
                Some(number) => crate::local_issues::display_id(number),
                None => {
                    return Err((
                        "checkout_issue.invalid",
                        format!("No local issue {} in {}", text.trim(), workspace.label),
                    ));
                }
            }
        } else {
            IssueReference::parse(text, workspace.home_issues.repository.as_deref())
                .map(|reference| reference.token())
                .map_err(|error| ("checkout_issue.invalid", error))?
        };
        // Removing a link reads nothing, so it needs no repository name.
        let read = if local_source || text.is_empty() {
            Ok(None)
        } else {
            self.github_place(node.as_deref(), &workspace.path)
                .map(|place| {
                    Some(crate::live::GhPlace {
                        node: self.own_node(),
                        place,
                    })
                })
        };
        let id = self
            .begin_task_operation(
                "checkout_issue",
                Some(workspace.path.clone()),
                checkout.branch.clone(),
                None,
                None,
            )
            .map_err(|error| ("checkout_issue.busy", error))?;
        // A link that cannot be read yet ends as the operator's failed task,
        // with why, rather than as a click that shows nothing.
        let read = match read {
            Ok(read) => read,
            Err(message) => {
                self.push_diagnostic("checkout_issue.unavailable", message.clone());
                self.op_timings
                    .finish(&super::op_timing::task_create_op_id(id), "failed");
                let operation = self
                    .snapshot
                    .task_operation
                    .as_mut()
                    .expect("the task operation was begun above");
                operation.phase = "failed".into();
                operation.message = Some(message);
                return Ok(());
            }
        };
        // The Workspace that carries the token: this machine's authoritative
        // one for the checkout, or the Herdr workspace that owns a node's.
        let (session, previous) = match node.as_deref() {
            None => {
                let session = workspace::authoritative_session_space(
                    &self.last_session_spaces,
                    &workspace,
                    &checkout.path,
                )
                .map(|space| space.id.clone());
                let previous = session
                    .as_ref()
                    .and_then(|id| self.issue_tokens.workspaces.get(id))
                    .cloned();
                (session, previous)
            }
            Some(node) => {
                let session = checkout.owner_workspace_id.clone();
                let previous = session.as_ref().and_then(|owner| {
                    self.device_raw_sessions
                        .get(node)?
                        .issue_tokens
                        .workspaces
                        .get(owner)
                        .cloned()
                });
                (session, previous)
            }
        };
        let request = crate::live::PurposeTaskRequest {
            id,
            checkout_id: checkout.id.clone(),
            repository_root: workspace.path,
            branch: checkout.branch,
            session_workspace_id: session,
            purpose: text.clone(),
        };
        self.issue_write_pending = Some(IssueWritePending {
            id,
            checkout_id: checkout.id,
            node: node.clone(),
        });
        let target = match node.as_deref() {
            None => self.local_worktree_target(),
            Some(node) => self.device_worktree_target(node),
        };
        let result = target.and_then(|target| {
            crate::live::spawn_issue_write(target, read, request.clone(), previous)
        });
        if let Err(error) = result {
            self.ingest_issue_operation_result(
                &request,
                Err(crate::live::IssueWriteFailure::unchanged(error)),
            );
        }
        Ok(())
    }

    pub(crate) fn ingest_issue_operation_result(
        &mut self,
        request: &crate::live::PurposeTaskRequest,
        result: Result<Option<crate::issues::IssueSnapshot>, crate::live::IssueWriteFailure>,
    ) -> bool {
        let Some(pending) = self
            .issue_write_pending
            .take_if(|pending| pending.id == request.id)
        else {
            return false;
        };
        if self.pr_link_checkout.as_deref() == Some(request.checkout_id.as_str()) {
            self.pr_link_checkout = None;
            self.settle_pr_link_write(result.as_ref().err().map(|error| error.detail.clone()));
        }
        if let Some(node) = pending.node {
            return self.ingest_node_issue_write(&node, request, result);
        }
        match result {
            Ok(issue) => {
                if let Some(id) = &request.session_workspace_id {
                    self.unconfirmed_issue_tokens.remove(id);
                }
                if !self
                    .github
                    .projects
                    .iter()
                    .any(|project| project.root_path == request.repository_root)
                {
                    self.github
                        .projects
                        .push(crate::model::GithubProjectSnapshot {
                            root_path: request.repository_root.clone(),
                            issues: Default::default(),
                            status: Default::default(),
                            pull_requests: Vec::new(),
                            ..Default::default()
                        });
                }
                if let Some(id) = &request.session_workspace_id {
                    if request.purpose.is_empty() {
                        self.issue_tokens.workspaces.remove(id);
                    } else {
                        self.issue_tokens
                            .workspaces
                            .insert(id.clone(), request.purpose.clone());
                    }
                }
                for workspace in &mut self.snapshot.navigator.workspaces {
                    if let Some(checkout) = workspace
                        .checkouts
                        .iter_mut()
                        .find(|checkout| checkout.id == request.checkout_id)
                    {
                        checkout.branch_issue =
                            (!request.purpose.is_empty()).then(|| request.purpose.clone());
                        checkout.issue = issue.clone().map(|issue| IssueLinkSnapshot {
                            issue,
                            source: "직접 연결".into(),
                        });
                    }
                }
                if let Some(issue) = issue
                    && let Some(project) = self
                        .github
                        .projects
                        .iter_mut()
                        .find(|project| project.root_path == request.repository_root)
                {
                    project
                        .issues
                        .issues
                        .retain(|known| known.reference != issue.reference);
                    project.issues.issues.insert(0, issue);
                    project.issues.issues.truncate(ISSUE_LIMIT);
                }
                if let Some(operation) = self.snapshot.task_operation.as_mut() {
                    operation.phase = "ready".into();
                }
                self.refresh_pull_requests(&request.repository_root);
                self.refresh_worktrees();
                self.apply_pull_requests();
            }
            Err(error) => {
                if error.unconfirmed_token
                    && let Some(id) = &request.session_workspace_id
                {
                    self.unconfirmed_issue_tokens.insert(
                        id.clone(),
                        UnconfirmedIssueToken {
                            value: request.purpose.clone(),
                            observed: false,
                        },
                    );
                }
                self.push_diagnostic("checkout_issue.save_failed", error.detail);
                if let Some(operation) = self.snapshot.task_operation.as_mut() {
                    operation.phase = "failed".into();
                }
            }
        }
        true
    }

    /// A node checkout's link was written on the node. Its rows show the new
    /// link at once, from the token and branch note the write set and the
    /// issue it read; the node's next session event and worktree read confirm
    /// them. A token the node may hold after a failed rollback is logged:
    /// the node's own Herdr is what the next session event reports.
    fn ingest_node_issue_write(
        &mut self,
        node: &str,
        request: &crate::live::PurposeTaskRequest,
        result: Result<Option<crate::issues::IssueSnapshot>, crate::live::IssueWriteFailure>,
    ) -> bool {
        match result {
            Ok(issue) => {
                let token = (!request.purpose.is_empty()).then(|| request.purpose.clone());
                if let Some(owner) = &request.session_workspace_id
                    && let Some(unconfirmed) = self.unconfirmed_node_issue_tokens.get_mut(node)
                {
                    unconfirmed.remove(owner);
                }
                if let Some(owner) = &request.session_workspace_id
                    && let Some(raw) = self.device_raw_sessions.get_mut(node)
                {
                    match &token {
                        Some(token) => raw
                            .issue_tokens
                            .workspaces
                            .insert(owner.clone(), token.clone()),
                        None => raw.issue_tokens.workspaces.remove(owner),
                    };
                }
                if let Some(branch) = &request.branch
                    && let Some(listed) = self.device_worktrees.get_mut(node)
                {
                    let note = listed
                        .notes
                        .entry(request.repository_root.clone())
                        .or_default()
                        .entry(branch.clone())
                        .or_default();
                    note.issue = token;
                }
                if let Some(issue) = issue
                    && let Some(project) = self.device_github.get_mut(node).and_then(|github| {
                        github
                            .projects
                            .iter_mut()
                            .find(|project| project.root_path == request.repository_root)
                    })
                {
                    project
                        .issues
                        .issues
                        .retain(|known| known.reference != issue.reference);
                    project.issues.issues.insert(0, issue);
                    project.issues.issues.truncate(ISSUE_LIMIT);
                }
                if let Some(operation) = self.snapshot.task_operation.as_mut() {
                    operation.phase = "ready".into();
                }
                self.request_device_worktrees(node, true);
            }
            Err(error) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "checkout_issue",
                    "kind": "node_write.failed",
                    "node": node,
                    "checkout_id": request.checkout_id,
                    "unconfirmed_token": error.unconfirmed_token,
                }));
                if error.unconfirmed_token
                    && let Some(owner) = &request.session_workspace_id
                {
                    self.unconfirmed_node_issue_tokens
                        .entry(node.to_owned())
                        .or_default()
                        .insert(
                            owner.clone(),
                            UnconfirmedIssueToken {
                                value: request.purpose.clone(),
                                observed: false,
                            },
                        );
                }
                self.push_diagnostic("checkout_issue.save_failed", error.detail);
                if let Some(operation) = self.snapshot.task_operation.as_mut() {
                    operation.phase = "failed".into();
                }
            }
        }
        self.refresh_device_catalog(node);
        true
    }

    /// Settles a node's unconfirmed tokens against the tokens its Herdr
    /// reports now, by the rule this machine's follow (`settle_unconfirmed`).
    pub(super) fn settle_node_issue_tokens(&mut self, node: &str, raw: &RemoteSessionSnapshot) {
        let Some(unconfirmed) = self.unconfirmed_node_issue_tokens.get_mut(node) else {
            return;
        };
        settle_unconfirmed(unconfirmed, &raw.issue_tokens.workspaces, |id| {
            raw.workspaces.iter().any(|workspace| {
                workspace
                    .session_workspace_ids
                    .iter()
                    .any(|known| known == id)
            })
        });
        if unconfirmed.is_empty() {
            self.unconfirmed_node_issue_tokens.remove(node);
        }
    }
}

/// The checkout issue link a worker is writing, and the node it is on
/// (`None` for this machine's).
pub(super) struct IssueWritePending {
    pub id: u64,
    pub checkout_id: String,
    pub node: Option<String>,
}

/// A Local issue as its panel reads it: the body and when it was made, from
/// the store the runtime already holds, so the read needs no worker. A
/// failure is a `local_issue.<code>` the panel words; why the store could not
/// be read was logged when it was loaded.
fn local_issue_detail(
    store: &Result<crate::local_issues::LocalIssueStore, String>,
    node: Option<&str>,
    key: &str,
) -> Result<crate::tasks::TaskDetail, String> {
    let (path, number) =
        crate::tasks::parse_local_key(key).ok_or_else(|| "local_issue.not_local".to_owned())?;
    match store {
        Err(_) => Err("local_issue.store_unreadable".to_owned()),
        Ok(store) => store
            .issue(node, path, number)
            .map(|issue| crate::tasks::TaskDetail {
                body: issue.body.clone(),
                created_at_unix_ms: Some(issue.created_at_unix_ms),
                ..Default::default()
            })
            .ok_or_else(|| "local_issue.missing".to_owned()),
    }
}

impl Runtime {
    /// The source a registered local project reads now (`tasks::source_kind`).
    pub(super) fn project_source_kind(
        &self,
        workspace: &WorkspaceSnapshot,
    ) -> crate::tasks::SourceKind {
        let status = workspace
            .checkouts
            .first()
            .map(|checkout| checkout.github.clone())
            .unwrap_or_default();
        crate::tasks::source_kind(
            crate::project_settings::issue_choice(
                &self.snapshot.ui_state.project_settings,
                &self.snapshot.ui_state.project_issue_sources,
                &workspace.device_id,
                &workspace.path,
            ),
            workspace.is_git,
            &workspace.home_issues,
            &status,
        )
    }

    pub(super) fn local_project_by_id(&self, workspace_id: &str) -> Option<WorkspaceSnapshot> {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id && workspace.remote_target_id.is_none())
            .cloned()
    }

    /// A project an issue action reaches, with the node it is on: one of
    /// this machine's (`None`), or one of a node that dials this core (PRD
    /// core-host-node-move D-29). A dialed device's projects keep their
    /// issues on that device and are not reached.
    pub(super) fn issue_project(
        &self,
        workspace_id: &str,
    ) -> Option<(Option<String>, WorkspaceSnapshot)> {
        if let Some(workspace) = self.local_project_by_id(workspace_id) {
            return Some((None, workspace));
        }
        self.node_sessions().find_map(|(node, session)| {
            let workspace = session
                .workspaces
                .iter()
                .find(|workspace| workspace.id == workspace_id)?;
            Some((Some(node.to_owned()), workspace.clone()))
        })
    }

    /// Where `gh` reads a project's repository: in its checkout on this
    /// machine, or, for a node's project, by the name the node read from its
    /// `origin` (D-16); never in a folder of this machine at the node's path.
    pub(super) fn github_place(
        &self,
        node: Option<&str>,
        root: &str,
    ) -> Result<crate::github::Where, String> {
        if let Some(crate::project_settings::RepositoryConnection::Repository { repository }) =
            crate::project_settings::get(
                &self.snapshot.ui_state.project_settings,
                node.unwrap_or(self.node.as_str()),
                root,
            )
            .and_then(|record| record.repository.as_ref())
        {
            return Ok(crate::github::Where::Repository(repository.clone()));
        }
        let Some(node) = node else {
            return Ok(crate::github::Where::Checkout(PathBuf::from(root)));
        };
        self.device_worktrees
            .get(node)
            .and_then(|listed| listed.projects.get(root))
            .and_then(|project| project.repository.clone())
            .map(crate::github::Where::Repository)
            .ok_or_else(|| "The node has not named this project's GitHub repository yet".to_owned())
    }

    /// Shows a change to a project's issues made here: this machine's
    /// projects are projected again, and a node's session rows are.
    pub(super) fn refresh_issue_rows(&mut self, node: Option<&str>) {
        match node {
            None => {
                self.apply_pull_requests();
            }
            Some(node) => {
                self.refresh_device_catalog(node);
            }
        }
    }

    /// Asks a node's repository again at its next read, as this machine's
    /// `refresh_pull_requests` does.
    pub(super) fn reread_node_github(&mut self, node: &str, root: &str) {
        self.refresh_device_pull_requests(node, root);
    }

    /// Runs `work` on its own thread and hands its answer to `ingest` under
    /// the lock; `Err` when there is no worker to run it on (a core built
    /// without one, as the tests build it).
    pub(super) fn spawn_issue_worker<T: Send + 'static>(
        &self,
        name: &str,
        work: impl FnOnce() -> T + Send + 'static,
        ingest: impl FnOnce(&mut Runtime, T) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let context = self
            .worker_context
            .clone()
            .ok_or_else(|| "No worker is available for this request".to_owned())?;
        thread::Builder::new()
            .name(format!("herdr-core-{name}"))
            .spawn(move || {
                let answer = work();
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => ingest(&mut guard, answer),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            })
            .map(|_| ())
            .map_err(|error| format!("{name} worker could not start: {error}"))
    }

    /// Settings › Issues: a project's source, or `auto` for its default.
    pub(super) fn set_issue_source(&mut self, payload: IssueSourceSetPayload) -> bool {
        let sources = &mut self.snapshot.ui_state.project_issue_sources;
        let changed = match payload.source.as_str() {
            "auto" => sources.remove(&payload.project_path).is_some(),
            crate::tasks::GITHUB | crate::tasks::LOCAL => {
                sources.insert(payload.project_path.clone(), payload.source.clone())
                    != Some(payload.source.clone())
            }
            other => {
                self.set_error(
                    "issue_source.unknown",
                    format!("No issue source named {other}"),
                    false,
                );
                return true;
            }
        };
        if !changed {
            return false;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "issues", "kind": "issue_source.set",
            "project": payload.project_path, "source": payload.source,
        }));
        self.persist_ui_state();
        self.apply_pull_requests();
        // The source is kept by path, so a node project at that path reads
        // its issues from the new source now too (D-29).
        let nodes: Vec<String> = self
            .node_sessions()
            .filter(|(_, session)| {
                session
                    .workspaces
                    .iter()
                    .any(|project| project.path == payload.project_path)
            })
            .map(|(node, _)| node.to_owned())
            .collect();
        for node in nodes {
            self.refresh_issue_rows(Some(&node));
        }
        true
    }

    /// Settings › Issues: how starting work from an issue behaves.
    pub(super) fn set_issue_settings(&mut self, payload: IssueSettingsSetPayload) -> bool {
        let settings = &mut self.snapshot.ui_state.issue_settings;
        let before = settings.clone();
        if let Some(value) = payload.ai_worktree_name {
            settings.ai_worktree_name = value;
        }
        if let Some(value) = payload.closes_instruction {
            settings.closes_instruction = value;
        }
        if *settings == before {
            return false;
        }
        self.persist_ui_state();
        true
    }

    /// A Local issue made in the store for `project_path` on `node` (this
    /// machine's for `None`), saved off the lock, and on the project's board
    /// at once; the store checks the title and body.
    pub(super) fn create_local_issue(
        &mut self,
        node: Option<&str>,
        project_path: &str,
        title: &str,
        body: &str,
    ) -> Result<u32, (crate::model::IssueFailure, String)> {
        let number = match self.local_issues.as_mut() {
            Err(reason) => {
                return Err((
                    crate::model::IssueFailure::new("local_issue.store_unreadable"),
                    format!("the local issue file could not be read: {reason}"),
                ));
            }
            Ok(store) => store
                .create(node, project_path, title, body, unix_milliseconds())
                .map_err(|error| (error.into(), error.to_string()))?,
        };
        self.persist_local_issues();
        self.refresh_issue_rows(node);
        Ok(number)
    }

    /// A GitHub issue Hide just made shows at once, in this machine's
    /// answer or the node's; the next read confirms it.
    pub(super) fn remember_created_issue(
        &mut self,
        node: Option<&str>,
        project_path: &str,
        issue: crate::issues::IssueSnapshot,
    ) {
        let github = match node {
            None => Some(&mut self.github),
            Some(node) => self.device_github.get_mut(node),
        };
        if let Some(project) = github.and_then(|github| {
            github.projects.iter_mut().find(|project| {
                project.root_path == project_path
                    && project.issues.repository.as_deref()
                        == Some(issue.reference.repository.as_str())
            })
        }) {
            project
                .issues
                .issues
                .retain(|known| known.reference != issue.reference);
            project.issues.issues.insert(0, issue);
            project.issues.issues.truncate(ISSUE_LIMIT);
        }
        match node {
            None => self.refresh_pull_requests(project_path),
            Some(node) => self.reread_node_github(node, project_path),
        }
        self.refresh_issue_rows(node);
    }

    /// An issue the backlog closed (`되돌리기`) leaves the open list at once;
    /// the next read confirms it.
    pub(super) fn forget_created_issue(
        &mut self,
        node: Option<&str>,
        project_path: &str,
        reference: &crate::issues::IssueReference,
    ) {
        let github = match node {
            None => Some(&mut self.github),
            Some(node) => self.device_github.get_mut(node),
        };
        if let Some(project) = github.and_then(|github| {
            github
                .projects
                .iter_mut()
                .find(|project| project.root_path == project_path)
        }) {
            project
                .issues
                .issues
                .retain(|known| known.reference != *reference);
        }
        match node {
            None => self.refresh_pull_requests(project_path),
            Some(node) => self.reread_node_github(node, project_path),
        }
        self.refresh_issue_rows(node);
    }

    /// `issue_detail_request`: an issue's body, labels, author, assignees and
    /// a bounded comment page, for its panel and the Start dialog. A memo
    /// is read from the store at once; GitHub reads run off the lock.
    pub(super) fn request_issue_detail(&mut self, payload: IssueDetailRequestPayload) -> bool {
        let Some((node, workspace)) = self.issue_project(&payload.workspace_id) else {
            return false;
        };
        let key = &payload.task_key;
        // A key only has authority within the selected project's projection.
        if !workspace.tasks.tasks.iter().any(|task| task.key == *key) {
            return false;
        }
        if payload.comments_after.is_none()
            && self
                .snapshot
                .issue_work
                .detail
                .as_ref()
                .is_some_and(|slot| {
                    slot.matches(&workspace.id, &workspace.device_id, key)
                        && slot.phase == "reading"
                })
        {
            return false;
        }
        if let Some(after) = &payload.comments_after {
            let valid = self
                .snapshot
                .issue_work
                .detail
                .as_ref()
                .is_some_and(|slot| {
                    slot.matches(&workspace.id, &workspace.device_id, key)
                        && slot.phase == "ready"
                        && !slot.comments_loading
                        && slot.comments_has_more
                        && slot.comments_cursor.as_ref() == Some(after)
                        && slot.comments.len() < crate::tasks::DETAIL_COMMENT_LIMIT
                });
            if !valid {
                return false;
            }
        }
        self.issue_detail_work.generation = self.issue_detail_work.generation.wrapping_add(1);
        let generation = self.issue_detail_work.generation;
        self.issue_detail_work.pending = None;
        if key.starts_with("local:") {
            let answer = local_issue_detail(&self.local_issues, node.as_deref(), key);
            self.snapshot.issue_work.detail = Some(crate::model::IssueDetailSnapshot::answered(
                workspace.id,
                workspace.device_id,
                key.clone(),
                answer,
            ));
            return true;
        }
        if payload.comments_after.is_some() {
            if let Some(slot) = self.snapshot.issue_work.detail.as_mut() {
                slot.comments_loading = true;
                slot.message = None;
            }
        } else {
            self.snapshot.issue_work.detail = Some(crate::model::IssueDetailSnapshot::reading(
                workspace.id.clone(),
                workspace.device_id.clone(),
                key.clone(),
            ));
        }
        // One worker and one latest pending request across windows; switching
        // items never accumulates subprocesses or applies an old response.
        let read = IssueDetailRead {
            workspace_id: workspace.id,
            device_id: workspace.device_id,
            task_key: payload.task_key,
            after: payload.comments_after,
        };
        if self.issue_detail_work.running {
            self.issue_detail_work.pending = Some((generation, read));
        } else {
            self.start_issue_detail_read(generation, read);
        }
        true
    }

    fn start_issue_detail_read(&mut self, generation: u64, read: IssueDetailRead) -> bool {
        self.issue_detail_work.running = true;
        let place = self
            .issue_project(&read.workspace_id)
            .filter(|(_, workspace)| {
                workspace.device_id == read.device_id
                    && workspace
                        .tasks
                        .tasks
                        .iter()
                        .any(|task| task.key == read.task_key)
            })
            .ok_or_else(|| "The project is no longer available".to_owned())
            .and_then(|(node, workspace)| self.github_place(node.as_deref(), &workspace.path));
        let reference = crate::tasks::parse_github_key(&read.task_key)
            .ok_or_else(|| "The item is not a GitHub issue".to_owned());
        let (place, reference) =
            match place.and_then(|place| reference.map(|reference| (place, reference))) {
                Ok(value) => value,
                Err(message) => {
                    return self.finish_issue_detail(generation, read, Err(message));
                }
            };
        let own = self.own_node();
        let worker_after = read.after.clone();
        let failed_read = read.clone();
        if let Err(message) = self.spawn_issue_worker(
            "issue-detail",
            move || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::github::issue_detail(
                        own.as_ref(),
                        &place,
                        &reference,
                        worker_after.as_deref(),
                    )
                }))
                .unwrap_or_else(|_| Err("The issue detail reader stopped".to_owned()))
            },
            move |runtime, result| runtime.finish_issue_detail(generation, read, result),
        ) {
            return self.finish_issue_detail(generation, failed_read, Err(message));
        }
        false
    }

    fn finish_issue_detail(
        &mut self,
        generation: u64,
        read: IssueDetailRead,
        result: Result<crate::tasks::TaskDetail, String>,
    ) -> bool {
        self.issue_detail_work.running = false;
        let mut changed = if generation == self.issue_detail_work.generation {
            self.ingest_issue_detail_page(
                &read.workspace_id,
                &read.device_id,
                &read.task_key,
                read.after.as_deref(),
                result,
            )
        } else {
            false
        };
        if let Some((next, payload)) = self.issue_detail_work.pending.take() {
            changed |= self.start_issue_detail_read(next, payload);
        }
        changed
    }

    #[cfg(test)]
    pub(crate) fn ingest_issue_detail(
        &mut self,
        workspace_id: &str,
        device_id: &str,
        key: &str,
        result: Result<crate::tasks::TaskDetail, String>,
    ) -> bool {
        self.ingest_issue_detail_page(workspace_id, device_id, key, None, result)
    }

    pub(super) fn ingest_issue_detail_page(
        &mut self,
        workspace_id: &str,
        device_id: &str,
        key: &str,
        after: Option<&str>,
        result: Result<crate::tasks::TaskDetail, String>,
    ) -> bool {
        if !self
            .issue_project(workspace_id)
            .is_some_and(|(_, workspace)| {
                workspace.device_id == device_id
                    && workspace.tasks.tasks.iter().any(|task| task.key == key)
            })
        {
            return false;
        }
        let Some(slot) = self.snapshot.issue_work.detail.as_mut().filter(|slot| {
            slot.matches(workspace_id, device_id, key)
                && match after {
                    Some(cursor) => {
                        slot.phase == "ready"
                            && slot.comments_loading
                            && slot.comments_cursor.as_deref() == Some(cursor)
                    }
                    None => slot.phase == "reading",
                }
        }) else {
            return false;
        };
        if let Err(reason) = &result {
            crate::diagnostic!(
                serde_json::json!({"component":"issues", "kind":"issue_detail.failed", "task":key, "reason":reason})
            );
        }
        if after.is_some() {
            slot.comments_loading = false;
            match result {
                Ok(detail) => {
                    let room =
                        crate::tasks::DETAIL_COMMENT_LIMIT.saturating_sub(slot.comments.len());
                    slot.comments_limited |=
                        detail.comments_limited || detail.comments.len() > room;
                    slot.comments.extend(detail.comments.into_iter().take(room));
                    slot.comment_count = detail.comment_count;
                    slot.comments_cursor = detail.comments_cursor;
                    slot.comments_has_more = detail.comments_has_more;
                    if slot.comments.len() >= crate::tasks::DETAIL_COMMENT_LIMIT
                        && slot.comments_has_more
                    {
                        slot.comments_limited = true;
                        slot.comments_has_more = false;
                    }
                    slot.message = None;
                }
                Err(message) => {
                    slot.message = Some(message);
                }
            }
        } else {
            *slot = crate::model::IssueDetailSnapshot::answered(
                workspace_id.to_owned(),
                device_id.to_owned(),
                key.to_owned(),
                result,
            );
        }
        true
    }

    /// The projects a search covers: this Mac's registered projects whose
    /// source is GitHub, in the catalog's order, once each by `owner/name`
    /// when the reader has resolved it (GitHub names ignore case); a project
    /// it has not read yet is resolved by the search's own worker. The cap
    /// leaves some out with the note to say so.
    pub(super) fn github_search_targets(
        &self,
    ) -> (Vec<crate::github::SearchTarget>, Option<String>) {
        let mut targets: Vec<crate::github::SearchTarget> = Vec::new();
        // A node's GitHub projects are searched by the name the node read,
        // with this machine's login (D-16); one not named yet is left out,
        // since its folder is not on this machine to resolve it in.
        let nodes = self
            .node_sessions()
            .flat_map(|(_, session)| session.workspaces.iter())
            .map(|workspace| (true, workspace));
        let own = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none())
            .map(|workspace| (false, workspace));
        for (on_node, workspace) in own.chain(nodes) {
            if !workspace.is_git
                || crate::tasks::source_kind(
                    None,
                    workspace.is_git,
                    &workspace.home_issues,
                    &workspace
                        .checkouts
                        .first()
                        .map(|checkout| checkout.github.clone())
                        .unwrap_or_default(),
                ) != crate::tasks::SourceKind::Github
            {
                continue;
            }
            let node = on_node.then_some(workspace.device_id.as_str());
            let repository = match self.github_place(node, &workspace.path) {
                Ok(crate::github::Where::Repository(repository)) => Some(repository),
                Ok(crate::github::Where::Checkout(_)) => workspace.home_issues.repository.clone(),
                Err(_) => None,
            }
            .filter(|repository| crate::issues::is_repository(repository));
            if on_node && repository.is_none() {
                continue;
            }
            if let Some(repository) = &repository
                && targets.iter().any(|known| {
                    known
                        .repository
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(repository))
                })
            {
                continue;
            }
            targets.push(crate::github::SearchTarget {
                root: std::path::PathBuf::from(&workspace.path),
                repository,
            });
        }
        let total = targets.len();
        if total <= SEARCH_REPOSITORY_LIMIT {
            return (targets, None);
        }
        targets.truncate(SEARCH_REPOSITORY_LIMIT);
        let note = format!("저장소 {total}개 중 {SEARCH_REPOSITORY_LIMIT}개만 검색했습니다.");
        (targets, Some(note))
    }

    /// `github_search`: pull requests and issues of this Mac's GitHub
    /// projects matching a query, answered in `issue_work.search` (PRD
    /// cmdk-navigation B16-B19). It runs only on this event, never on a timer.
    /// One search runs at a time: a request for the running search's query (or
    /// with its request id) joins it, any other waits as the one pending
    /// request, which a newer request replaces, and starts when the running
    /// search ends. The slot always shows the latest request as `working`.
    pub(super) fn request_github_search(&mut self, payload: GithubSearchPayload) -> bool {
        let request_id = payload.request_id;
        if request_id.is_empty() || request_id.chars().count() > SEARCH_REQUEST_ID_LIMIT {
            self.set_error(
                "github_search.invalid_request",
                "A GitHub search needs a request id of up to 128 characters",
                false,
            );
            return true;
        }
        let query = payload.query.trim();
        let query_chars = query.chars().count();
        crate::diagnostic!(serde_json::json!({
            "component": "issues", "kind": "github_search.requested",
            "request_id": request_id, "query_chars": query_chars,
        }));
        let refused = if query.is_empty() {
            Some("검색어를 입력하세요.")
        } else if query_chars > crate::github::SEARCH_QUERY_LIMIT {
            Some("검색어는 200자까지 입력할 수 있습니다.")
        } else {
            None
        };
        if let Some(message) = refused {
            // The refusal is the latest request, so one waiting is stale.
            self.github_search.pending = None;
            let echoed = query
                .chars()
                .take(crate::github::SEARCH_QUERY_LIMIT)
                .collect();
            self.snapshot.issue_work.search = Some(crate::model::GithubSearchSnapshot::failed(
                request_id, echoed, message,
            ));
            return true;
        }
        let request = SearchRequest {
            request_id,
            query: query.to_owned(),
        };
        match self.github_search.active.as_mut() {
            Some(active)
                if !active.invalidated
                    && (active.request.request_id == request.request_id
                        || active.request.query == request.query) =>
            {
                active.request.request_id = request.request_id.clone();
                self.github_search.pending = None;
                self.snapshot.issue_work.search =
                    Some(crate::model::GithubSearchSnapshot::working(
                        request.request_id,
                        active.request.query.clone(),
                    ));
            }
            Some(_) => {
                self.snapshot.issue_work.search =
                    Some(crate::model::GithubSearchSnapshot::working(
                        request.request_id.clone(),
                        request.query.clone(),
                    ));
                self.github_search.pending = Some(request);
            }
            None => {
                self.start_github_search(request);
            }
        }
        true
    }

    /// Starts one search worker for `request`, or answers it at once when
    /// there is nothing to search or no worker to search with.
    fn start_github_search(&mut self, request: SearchRequest) {
        let (targets, note) = self.github_search_targets();
        if note.is_some() {
            crate::diagnostic!(serde_json::json!({
                "component": "issues", "kind": "github_search.capped",
                "request_id": request.request_id, "limit": SEARCH_REPOSITORY_LIMIT,
            }));
        }
        if targets.is_empty() {
            self.snapshot.issue_work.search = Some(crate::model::GithubSearchSnapshot {
                phase: "ready".into(),
                ..crate::model::GithubSearchSnapshot::working(request.request_id, request.query)
            });
            return;
        }
        self.github_search.runs = self.github_search.runs.wrapping_add(1).max(1);
        let run = self.github_search.runs;
        self.snapshot.issue_work.search = Some(crate::model::GithubSearchSnapshot::working(
            request.request_id.clone(),
            request.query.clone(),
        ));
        let query = request.query.clone();
        self.github_search.active = Some(ActiveSearch {
            run,
            request,
            note,
            invalidated: false,
        });
        let node = self.own_node();
        let spawned = self.spawn_issue_worker(
            "github-search",
            move || {
                // A worker that unwound would leave the one search slot taken
                // for good, so a panic is a failed search like any other.
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::github::search(node.as_ref(), &targets, &query)
                }))
                .unwrap_or_else(|_| Err("the search worker panicked".to_owned()))
            },
            move |runtime, result| runtime.ingest_github_search(run, result),
        );
        if let Err(reason) = spawned
            && let Some(active) = self.github_search.active.take()
        {
            crate::diagnostic!(serde_json::json!({
                "component": "issues", "kind": "github_search.failed",
                "request_id": active.request.request_id, "reason": reason,
            }));
            self.snapshot.issue_work.search = Some(crate::model::GithubSearchSnapshot::failed(
                active.request.request_id,
                active.request.query,
                reason,
            ));
        }
    }

    /// A search worker's answer. It ends the running search; the slot takes
    /// it only while the slot still shows that search's request, so an answer
    /// for a request a newer one replaced is dropped. The pending request,
    /// if any, starts after it.
    pub(crate) fn ingest_github_search(
        &mut self,
        run: u64,
        result: Result<Vec<crate::model::GithubSearchResult>, String>,
    ) -> bool {
        if self
            .github_search
            .active
            .as_ref()
            .is_none_or(|active| active.run != run)
        {
            return false;
        }
        let Some(finished) = self.github_search.active.take() else {
            return false;
        };
        if let Err(reason) = &result {
            crate::diagnostic!(serde_json::json!({
                "component": "issues", "kind": "github_search.failed",
                "request_id": finished.request.request_id, "reason": reason,
            }));
        }
        let mut changed = false;
        if let Some(slot) = self.snapshot.issue_work.search.as_mut().filter(|slot| {
            !finished.invalidated
                && slot.request_id == finished.request.request_id
                && slot.phase == "working"
        }) {
            let answered = match result {
                Ok(results) => crate::model::GithubSearchSnapshot {
                    phase: "ready".into(),
                    results,
                    message: finished.note,
                    ..crate::model::GithubSearchSnapshot::working(
                        finished.request.request_id,
                        finished.request.query,
                    )
                },
                Err(_) => crate::model::GithubSearchSnapshot::failed(
                    finished.request.request_id,
                    finished.request.query,
                    SEARCH_FAILED_MESSAGE,
                ),
            };
            *slot = answered;
            changed = true;
        }
        if let Some(next) = self.github_search.pending.take() {
            self.start_github_search(next);
            changed = true;
        }
        changed
    }

    /// `worktree_name_suggest`: the background AI names the worktree. Off
    /// when Settings › Issues says so; the dialog's own name stands then.
    pub(super) fn suggest_worktree_name(&mut self, payload: WorktreeNameSuggestPayload) -> bool {
        // With Hide AI off, or no agent chosen, the dialog's own name stands
        // (B33, B47).
        if !self.snapshot.ui_state.issue_settings.ai_worktree_name || !self.ai_active() {
            return false;
        }
        let settings = self.ai_settings.clone().unwrap_or_default();
        let request_id = payload.request_id.clone();
        self.snapshot.issue_work.name = Some(crate::model::WorktreeNameSnapshot {
            request_id: request_id.clone(),
            phase: "working".into(),
            name: None,
            message: None,
        });
        let worker_request = request_id.clone();
        let node = self.own_node();
        let usage = self.ai_usage();
        if let Err(message) = self.spawn_issue_worker(
            "worktree-name",
            move || {
                crate::ai::suggest_worktree_name(
                    &node,
                    &settings,
                    &usage,
                    &worker_request,
                    &payload.prefix,
                    &payload.title,
                    &payload.body,
                )
            },
            move |runtime, result| runtime.ingest_worktree_name(&request_id, result),
        ) {
            self.snapshot.issue_work.name = Some(crate::model::WorktreeNameSnapshot {
                request_id: payload.request_id,
                phase: "failed".into(),
                name: None,
                message: Some(message),
            });
        }
        true
    }

    pub(crate) fn ingest_worktree_name(
        &mut self,
        request_id: &str,
        result: Result<String, String>,
    ) -> bool {
        let Some(slot) = self
            .snapshot
            .issue_work
            .name
            .as_mut()
            .filter(|slot| slot.request_id == request_id)
        else {
            return false;
        };
        crate::diagnostic!(serde_json::json!({
            "component": "issues", "kind": "worktree_name.answered",
            "ok": result.is_ok(),
        }));
        match result {
            Ok(name) => {
                slot.phase = "ready".into();
                slot.name = Some(name);
            }
            Err(message) => {
                slot.phase = "failed".into();
                slot.message = Some(message);
            }
        }
        true
    }

    /// `issue_set_open`: closes or reopens a Local issue. A GitHub issue is
    /// closed by the pull request that fixes it, on GitHub.
    pub(super) fn set_issue_open(&mut self, payload: IssueSetOpenPayload) -> bool {
        let Some((path, number)) = crate::tasks::parse_local_key(&payload.task_key) else {
            self.set_error(
                "issue_set_open.not_local",
                "Only a local issue is closed from Hide",
                false,
            );
            return true;
        };
        let node = match self.local_issue_project(&payload.workspace_id, path) {
            Ok(node) => node,
            Err(message) => {
                self.set_error("issue_set_open.unknown_project", message, false);
                return true;
            }
        };
        let Ok(store) = self.local_issues.as_mut() else {
            self.set_error(
                "issue_set_open.unavailable",
                "Local issues are not readable right now",
                true,
            );
            return true;
        };
        if !store.set_open(
            node.as_deref(),
            path,
            number,
            payload.open,
            unix_milliseconds(),
        ) {
            return false;
        }
        self.persist_local_issues();
        self.refresh_issue_rows(node.as_deref());
        true
    }

    /// The node whose store keeps a Local issue of `workspace_id` at
    /// `path`: the project the request names has to be the one its key
    /// names, so a key is never applied to another machine's project at the
    /// same path.
    fn local_issue_project(
        &self,
        workspace_id: &str,
        path: &str,
    ) -> Result<Option<String>, String> {
        match self.issue_project(workspace_id) {
            Some((node, workspace)) if workspace.path == path => Ok(node),
            Some(_) => Err("The issue is not this project's".to_owned()),
            None => Err("The issue's project is not on this Mac or a linked machine".to_owned()),
        }
    }

    /// `local_issue_update`: a Local issue's title and body, edited in its
    /// panel (PRD overview-lenses-issues D-41). The store changes under the
    /// lock and is written by the save thread; the answer, or why it was
    /// refused, is in `issue_work.update` for the web's request id. There is
    /// no GitHub counterpart.
    pub(super) fn update_local_issue(&mut self, payload: LocalIssueUpdatePayload) -> bool {
        let now = unix_milliseconds();
        let key = crate::tasks::parse_local_key(&payload.task_key);
        let node = key.map(|(path, _)| self.local_issue_project(&payload.workspace_id, path));
        use crate::model::IssueFailure;
        let updated = match (key, node.clone(), self.local_issues.as_mut()) {
            (None, _, _) | (_, None, _) => Err((
                IssueFailure::new("local_issue.not_local"),
                "only a local issue is edited in Hide".to_owned(),
            )),
            (_, Some(Err(message)), _) => {
                Err((IssueFailure::new("local_issue.project_unknown"), message))
            }
            (Some(_), _, Err(reason)) => Err((
                IssueFailure::new("local_issue.store_unreadable"),
                format!("the local issue file could not be read: {reason}"),
            )),
            (Some((path, number)), Some(Ok(node)), Ok(store)) => store
                .update(
                    node.as_deref(),
                    path,
                    number,
                    &payload.title,
                    &payload.body,
                    now,
                )
                .map_err(|error| (error.into(), error.to_string())),
        };
        crate::diagnostic!(serde_json::json!({
            "component": "issues", "kind": "local_issue.update",
            "request": payload.request_id, "task": payload.task_key, "ok": updated.is_ok(),
            "reason": updated.as_ref().err().map(|(_, detail)| detail),
        }));
        if matches!(updated, Ok(true)) {
            self.persist_local_issues();
            let node = node.and_then(Result::ok).flatten();
            self.refresh_issue_rows(node.as_deref());
        }
        self.snapshot.issue_work.update = Some(crate::model::IssueUpdateSnapshot {
            request_id: payload.request_id,
            task_key: payload.task_key,
            phase: if updated.is_ok() { "ready" } else { "failed" }.into(),
            failure: updated.err().map(|(failure, _)| failure),
        });
        true
    }

    /// Writes the Local store off the lock: one save thread at a time, and
    /// a change made while it writes is written after it (`write_ui_state`).
    pub(super) fn persist_local_issues(&mut self) {
        let Some(path) = self.local_issues_path.clone() else {
            return;
        };
        let Some(context) = self.worker_context.clone() else {
            // A runtime without a worker has no shared mutex to stay off.
            if let Ok(store) = &self.local_issues
                && let Err(reason) = crate::local_issues::save(&path, store)
            {
                self.push_diagnostic("local_issues.save_failed", reason);
            }
            return;
        };
        self.local_issues_save_pending = true;
        if self.local_issues_save_active {
            return;
        }
        self.local_issues_save_active = true;
        let spawned = thread::Builder::new()
            .name("hide-local-issues-save".into())
            .spawn(move || {
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                loop {
                    let store = {
                        let mut guard = runtime.lock().unwrap_or_else(|error| error.into_inner());
                        let store = match (&guard.local_issues, guard.local_issues_save_pending) {
                            (Ok(store), true) => store.clone(),
                            _ => {
                                guard.local_issues_save_active = false;
                                return;
                            }
                        };
                        guard.local_issues_save_pending = false;
                        store
                    };
                    if let Err(reason) = crate::local_issues::save(&path, &store) {
                        runtime
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .push_diagnostic(
                                "local_issues.save_failed",
                                format!("Local issues were not saved; they stay for this session: {reason}"),
                            );
                        context.notifier.notify();
                    }
                }
            });
        if let Err(error) = spawned {
            self.local_issues_save_active = false;
            self.push_diagnostic(
                "local_issues.save_failed",
                format!("The local issue save worker could not start: {error}"),
            );
        }
    }
}

impl Runtime {
    /// A node's projects' issues, read where they live (PRD
    /// core-host-node-move D-29): a GitHub project's from the repository the
    /// node named, read with this machine's login in the node's pull request
    /// pass (D-16), and a Local project's from this core's store under the
    /// node's id, the slot a core move wrote them into. A checkout's link is
    /// the chain a local checkout reads, from the node's Herdr tokens and
    /// branch notes. It runs with `derive_device_session`, so only when the
    /// node's session or one of these answers moved; an idle fetch keeps the
    /// published rows.
    pub(super) fn project_node_issues(&self, target: &str, session: &mut RemoteSessionSnapshot) {
        let listed = self.device_worktrees.get(target);
        let github = self.device_github.get(target);
        let sources = &self.snapshot.ui_state.project_issue_sources;
        // The agents a pane delegated to are known from the lineage, which
        // only the published session carries; a first derive has none yet.
        let lineage = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == target)
            .and_then(|status| status.session.as_ref())
            .map_or(&session.agents, |published| &published.agents);
        let links = LinkTokens::new(
            &session.issue_tokens,
            self.unconfirmed_node_issue_tokens.get(target),
            lineage,
            &self.invalid_links,
        );
        let (read, _) = self.node_github_repositories(target);
        let unread = crate::issues::ProjectIssuesSnapshot::default();
        for workspace in &mut session.workspaces {
            let effective_repository = match self.github_place(Some(target), &workspace.path) {
                Ok(crate::github::Where::Repository(repository)) => Some(repository),
                _ => None,
            };
            let asked_repository = effective_repository.as_ref();
            let found = github
                .and_then(|github| github.project(&workspace.path))
                .filter(|found| {
                    asked_repository.is_none_or(|repository| {
                        found
                            .issues
                            .repository
                            .as_ref()
                            .is_none_or(|read| read.eq_ignore_ascii_case(repository))
                    })
                });
            let asked = read.iter().any(|(root, _)| *root == workspace.path);
            let status = node_github_status(
                workspace.is_git,
                listed.and_then(|listed| listed.projects.get(&workspace.path)),
                found,
                asked,
                asked_repository.is_some(),
            );
            // The rows carry the issues as tasks; `home_issues` names only the
            // repository, which is all the shell reads of a node's. Before a
            // GitHub read answers with its name, or while every read fails,
            // the repository the node named is it, as a local project's is
            // the one its worktree read named.
            let issues = found.map_or(&unread, |found| &found.issues);
            let repository = asked_repository
                .cloned()
                .or_else(|| issues.repository.clone())
                .or_else(|| {
                    listed
                        .and_then(|listed| listed.projects.get(&workspace.path))
                        .and_then(|project| project.repository.clone())
                });
            workspace.home_issues = crate::issues::ProjectIssuesSnapshot {
                repository,
                ..Default::default()
            };
            let notes = listed.and_then(|listed| listed.notes.get(&workspace.path));
            let choice = crate::project_settings::issue_choice(
                &self.snapshot.ui_state.project_settings,
                sources,
                target,
                &workspace.path,
            );
            let local = crate::tasks::source_kind(
                choice,
                workspace.is_git,
                &workspace.home_issues,
                &status,
            ) == crate::tasks::SourceKind::Local;
            let repository = workspace.home_issues.repository.as_deref();
            let mut local_links = BTreeMap::new();
            let mut linked = BTreeSet::new();
            for checkout in &mut workspace.checkouts {
                checkout.github = status.clone();
                checkout.branch_issue = checkout
                    .branch
                    .as_ref()
                    .and_then(|branch| notes?.get(branch)?.issue.clone());
                let owner = checkout.owner_workspace_id.as_deref();
                let scope = crate::project_settings::get(
                    &self.snapshot.ui_state.project_settings,
                    target,
                    &workspace.path,
                );
                checkout.issue =
                    match links.checkout_link(owner, checkout, local, repository, scope) {
                        Some(ChainLink::Local(number)) => {
                            local_links.insert(checkout.id.clone(), number);
                            None
                        }
                        Some(ChainLink::Github(candidate)) => {
                            linked.insert(candidate.reference.clone());
                            issues
                                .issues
                                .iter()
                                .find(|issue| issue.reference == candidate.reference)
                                .map(|issue| IssueLinkSnapshot {
                                    issue: issue.clone(),
                                    source: candidate.source,
                                })
                        }
                        None => None,
                    };
            }
            // What the node's GitHub read brings beside the open list: the
            // linked issues it leaves out, closed or in another repository.
            if !linked.is_empty() {
                session.issue_links.insert(
                    workspace.path.clone(),
                    linked.into_iter().take(ISSUE_LIMIT).collect(),
                );
            }
            project_tasks(
                workspace,
                Some(issues),
                choice,
                &self.local_issues,
                Some(target),
                &local_links,
            );
        }
    }
}

/// A checkout's issue as the link chain names it.
enum ChainLink {
    Local(u32),
    Github(IssueCandidate),
}

/// What one machine's Herdr says about its checkouts' links, gathered once
/// per pass, for this machine's checkouts and a node's alike: each Herdr
/// workspace's manual token, the values a failed write here may have left
/// unconfirmed there, and each pane's token with those of every agent it
/// delegated to.
struct LinkTokens<'a> {
    workspaces: &'a BTreeMap<String, String>,
    unconfirmed: Option<&'a BTreeMap<String, UnconfirmedIssueToken>>,
    family: BTreeMap<String, BTreeMap<String, String>>,
    invalid: &'a InvalidLinks,
}

impl<'a> LinkTokens<'a> {
    /// `agents` carry the lineage that names a pane's descendants.
    fn new(
        tokens: &'a crate::wire::IssueTokens,
        unconfirmed: Option<&'a BTreeMap<String, UnconfirmedIssueToken>>,
        agents: &[crate::model::SidebarAgentSnapshot],
        invalid: &'a InvalidLinks,
    ) -> Self {
        Self {
            workspaces: &tokens.workspaces,
            unconfirmed,
            family: family_tokens(&tokens.panes, agents),
            invalid,
        }
    }

    /// A checkout's issue by the link chain (`chain_link`): the manual token
    /// of the Herdr workspace that carries it (`session`), unless it is a
    /// value a failed write left unconfirmed, then its panes' and their
    /// descendants' tokens, its branch note, its pull request and its branch.
    fn checkout_link(
        &self,
        session: Option<&str>,
        checkout: &crate::model::CheckoutSnapshot,
        local: bool,
        repository: Option<&str>,
        scope: Option<&crate::project_settings::ProjectSettings>,
    ) -> Option<ChainLink> {
        let manual = session.and_then(|id| {
            self.workspaces.get(id).filter(|value| {
                !self
                    .unconfirmed
                    .and_then(|unconfirmed| unconfirmed.get(id))
                    .is_some_and(|token| token.value == **value)
            })
        });
        let physical: BTreeSet<String> = checkout
            .tabs
            .iter()
            .flat_map(|tab| tab.panes.iter().map(|pane| pane.id.clone()))
            .collect();
        let pane = pane_token(&self.family, &physical);
        let bare_number = match manual.or(pane).or(checkout.branch_issue.as_ref()) {
            Some(value) => value
                .strip_prefix('#')
                .and_then(|number| number.parse::<u32>().ok()),
            None if checkout
                .pull_request
                .as_ref()
                .is_none_or(|pr| pr.closing_issues.is_empty()) =>
            {
                checkout
                    .branch
                    .as_deref()
                    .and_then(crate::issues::branch_number)
            }
            None => None,
        };
        let repository = scope
            .and_then(|record| {
                record.link_scope(&checkout.path, checkout.branch.as_deref(), bare_number?)
            })
            .map(|scope| scope.reference.repository.as_str())
            .or(repository);
        chain_link(local, manual, pane, checkout, repository, self.invalid)
    }
}

/// Most checkouts whose malformed link is remembered; past it the record
/// starts over, so a value already logged is at worst logged once more.
const INVALID_LINKS_LIMIT: usize = 1024;

/// The malformed link value each checkout's chain last logged. The chain
/// runs on every pass over a project, a node's included, so a value that
/// stays malformed is logged once for its checkout, not on each pass.
#[derive(Default)]
pub(super) struct InvalidLinks(std::cell::RefCell<BTreeMap<String, String>>);

impl InvalidLinks {
    fn log(&self, checkout_id: &str, value: &str) {
        let mut logged = self.0.borrow_mut();
        if logged.get(checkout_id).is_some_and(|last| last == value) {
            return;
        }
        if logged.len() >= INVALID_LINKS_LIMIT && !logged.contains_key(checkout_id) {
            logged.clear();
        }
        logged.insert(checkout_id.to_owned(), value.to_owned());
        crate::diagnostic!(
            serde_json::json!({"component":"checkout_issue","kind":"reference.invalid","checkout_id":checkout_id})
        );
    }
}

/// Keeps a failed write's token aside until the machine's own tokens show
/// how it settled: while the value is the one the write may have left, it is
/// not a link; a delayed event for that write must be seen before a later
/// replacement or clear confirms it, and a workspace that is gone drops it.
fn settle_unconfirmed(
    unconfirmed: &mut BTreeMap<String, UnconfirmedIssueToken>,
    tokens: &BTreeMap<String, String>,
    live: impl Fn(&str) -> bool,
) {
    unconfirmed.retain(|id, token| {
        if !live(id) {
            return false;
        }
        if tokens.get(id) == Some(&token.value) {
            token.observed = true;
            true
        } else {
            !token.observed
        }
    });
}

/// Each pane's issue token and every descendant agent's, by pane, so a
/// checkout links the issue an agent it delegated to names.
fn family_tokens(
    panes: &BTreeMap<String, String>,
    agents: &[crate::model::SidebarAgentSnapshot],
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut family: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (pane, token) in panes {
        family
            .entry(pane.clone())
            .or_default()
            .insert(pane.clone(), token.clone());
    }
    for agent in agents {
        if let Some(token) = panes.get(&agent.pane_id) {
            for ancestor in &agent.lineage_path_pane_ids {
                family
                    .entry(ancestor.clone())
                    .or_default()
                    .insert(agent.pane_id.clone(), token.clone());
            }
        }
    }
    family
}

/// The token a checkout's panes or their descendants carry: the lowest pane
/// id's, so the answer does not move with the order panes are listed in.
fn pane_token<'a>(
    family: &'a BTreeMap<String, BTreeMap<String, String>>,
    panes: &BTreeSet<String>,
) -> Option<&'a String> {
    panes
        .iter()
        .filter_map(|id| family.get(id))
        .flat_map(|tokens| tokens.iter())
        .min_by_key(|(id, _)| *id)
        .map(|(_, token)| token)
}

/// The issue a checkout links to: the manual token, then a pane's token,
/// then the branch's `issue` note, then for GitHub the pull request's first
/// closing issue, then the number the branch starts with. A Local project
/// reads the same chain as a local id (`L-3`); a pull request's closing
/// issues are GitHub's and do not apply.
fn chain_link(
    local: bool,
    manual: Option<&String>,
    pane: Option<&String>,
    checkout: &crate::model::CheckoutSnapshot,
    repository: Option<&str>,
    invalid: &InvalidLinks,
) -> Option<ChainLink> {
    // An explicit memo id is authoritative even in a GitHub project. Bare
    // numbers retain their historical local meaning only without GitHub.
    if let Some(value) = manual.or(pane).or(checkout.branch_issue.as_ref())
        && (value.starts_with("L-") || value.starts_with("l-"))
    {
        return crate::local_issues::parse_id(value).map(ChainLink::Local);
    }
    if local {
        let explicit = manual
            .map(|value| (value, "직접 연결"))
            .or_else(|| pane.map(|value| (value, "pane 토큰")))
            .or_else(|| {
                checkout
                    .branch_issue
                    .as_ref()
                    .map(|value| (value, "브랜치 설정"))
            });
        if let Some((value, source)) = explicit
            && let Ok(reference) = IssueReference::parse(value, None)
        {
            return Some(ChainLink::Github(IssueCandidate {
                reference,
                source: source.into(),
            }));
        }
    }
    if local {
        return manual
            .or(pane)
            .or(checkout.branch_issue.as_ref())
            .and_then(|value| crate::local_issues::parse_id(value))
            .or_else(|| {
                checkout
                    .branch
                    .as_deref()
                    .and_then(crate::issues::branch_number)
            })
            .map(ChainLink::Local);
    }
    let (value, source) = manual
        .map(|value| (value.clone(), "직접 연결".to_owned()))
        .or_else(|| pane.map(|value| (value.clone(), "pane 토큰".into())))
        .or_else(|| {
            checkout
                .branch_issue
                .as_ref()
                .map(|value| (value.clone(), "브랜치 설정".into()))
        })
        .or_else(|| {
            checkout.pull_request.as_ref().and_then(|pr| {
                pr.closing_issues
                    .first()
                    .map(|issue| (issue.token(), format!("PR #{} 본문", pr.number)))
            })
        })
        .or_else(|| {
            checkout
                .branch
                .as_deref()
                .and_then(crate::issues::branch_number)
                .map(|number| (format!("#{number}"), "브랜치 이름".into()))
        })?;
    match IssueReference::parse(&value, repository) {
        Ok(reference) => Some(ChainLink::Github(IssueCandidate { reference, source })),
        Err(_) => {
            // A missing repository will resolve after the first lookup;
            // malformed external metadata is diagnostic.
            if repository.is_some() || !value.starts_with('#') {
                invalid.log(&checkout.id, &value);
            }
            None
        }
    }
}

/// A project's task list from the source it reads, and each checkout's task
/// by its key, for this machine's project (`node` is `None`) and a node's
/// alike. `issues` is the project's GitHub list when its row does not carry
/// it (a node's); `local_links` are the Local issue numbers the chain named,
/// by checkout; a GitHub checkout's task is the issue `sync_issues` or
/// `project_node_issues` linked it to.
fn project_tasks(
    workspace: &mut crate::model::WorkspaceSnapshot,
    issues: Option<&crate::issues::ProjectIssuesSnapshot>,
    choice: Option<&str>,
    store: &Result<crate::local_issues::LocalIssueStore, String>,
    node: Option<&str>,
    local_links: &BTreeMap<String, u32>,
) -> bool {
    let mut changed = false;
    let status = workspace
        .checkouts
        .first()
        .map(|checkout| checkout.github.clone())
        .unwrap_or_default();
    let issues = issues.unwrap_or(&workspace.home_issues);
    let read = match store {
        Err(reason) => crate::tasks::LocalRead::Failed(reason),
        Ok(store) => crate::tasks::LocalRead::Ready(store.project(node, &workspace.path)),
    };
    let mut tasks =
        crate::tasks::local_tasks(&workspace.path, read, choice == Some(crate::tasks::LOCAL));
    if store.is_err() {
        tasks.tasks.extend(
            workspace
                .tasks
                .tasks
                .iter()
                .filter(|task| task.source == crate::tasks::LOCAL)
                .cloned(),
        );
    }
    if workspace.is_git
        && (issues.repository.is_some()
            || crate::tasks::source_kind(None, true, issues, &status)
                == crate::tasks::SourceKind::Github)
    {
        let github =
            crate::tasks::github_tasks(issues, &status, choice == Some(crate::tasks::GITHUB));
        tasks.sources.extend(github.sources);
        if choice != Some(crate::tasks::LOCAL) {
            tasks.source = github.source;
        }
        tasks.tasks.extend(github.tasks);
        tasks.overflow = github.overflow;
    }
    if workspace.tasks != tasks {
        workspace.tasks = tasks;
        changed = true;
    }
    for checkout in &mut workspace.checkouts {
        let memo = local_links.get(&checkout.id);
        let (key, closes) = if let Some(number) = memo {
            let key = Some(number)
                .map(|number| crate::tasks::local_key(&workspace.path, *number))
                .filter(|key| workspace.tasks.tasks.iter().any(|task| &task.key == key));
            (key, Vec::new())
        } else {
            let key = checkout
                .issue
                .as_ref()
                .map(|link| crate::tasks::github_key(&link.issue.reference));
            let closes: Vec<String> = checkout
                .pull_request
                .iter()
                .flat_map(|pr| pr.closing_issues.iter().map(crate::tasks::github_key))
                .filter(|closed| Some(closed) != key.as_ref())
                .filter(|closed| workspace.tasks.tasks.iter().any(|task| &task.key == closed))
                .collect();
            (key, closes)
        };
        if checkout.task_key != key || checkout.closes_task_keys != closes {
            checkout.task_key = key;
            checkout.closes_task_keys = closes;
            changed = true;
        }
    }
    changed
}

/// What a node project's GitHub read says: the answer the node's pull
/// request pass brought; else, for a repository the node refused to read,
/// that refusal, which no GitHub read follows; else, for a repository whose
/// `origin` the node said names no GitHub repository, no GitHub remote, so
/// its issues default to Local as this machine's do; else still reading
/// while the repository is among those read (`node_github_repositories`).
fn node_github_status(
    is_git: bool,
    listed: Option<&crate::model::ProjectWorktreesSnapshot>,
    found: Option<&crate::model::GithubProjectSnapshot>,
    asked: bool,
    has_repository: bool,
) -> crate::model::GithubStatusSnapshot {
    if let Some(found) = found {
        return found.status.clone();
    }
    match listed {
        Some(listed) if is_git && listed.unavailable_reason.is_some() => {
            crate::model::GithubStatusSnapshot {
                available: false,
                unavailable_reason: listed.unavailable_reason.clone(),
                ..Default::default()
            }
        }
        Some(listed) if is_git && listed.repository.is_none() && !has_repository => {
            // The answer a local project gets (`github::no_github_remote`):
            // nothing to draw, so no reason.
            crate::model::GithubStatusSnapshot {
                available: true,
                failure_category: Some(crate::model::GithubFailureCategory::NoGithubRemote),
                ..Default::default()
            }
        }
        _ => crate::model::GithubStatusSnapshot {
            loading: asked,
            ..Default::default()
        },
    }
}
