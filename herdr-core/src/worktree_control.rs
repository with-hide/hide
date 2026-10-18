//! Worktree actions cross the socket boundary before publishing removal readiness.
use super::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};

use crate::agent_start::StartError;
use hide_node_link::git::{BranchConfigKey, GitCommand};
use hide_node_link::process::ProcessStart;
use hide_node_link::protocol::Call;

pub(crate) const CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);
const CONFIRM_POLL: Duration = Duration::from_millis(100);
/// A host's Git checks answer in seconds; a removal deletes a whole folder.
const HOST_CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const HOST_REMOVE_TIMEOUT: Duration = Duration::from_secs(120);
/// A git command on a repository's node: a checkout of a large tree is the
/// slowest; a branch check and a fetch carry the node's own bound.
const GIT_TIMEOUT: Duration = Duration::from_secs(120);
/// A read of a few pids' start times.
const PROCESS_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Where a worktree task runs: the Herdr that creates or closes the
/// checkout's panes and the file host that answers its Git checks and runs
/// its removal, both of the device that holds the repository (PRD S5.5
/// B27-B29). This machine is the in-process host and its own Herdr.
#[derive(Clone)]
pub struct WorktreeTarget {
    connector: Arc<dyn ApiConnector>,
    host: Arc<dyn crate::node_access::NodeLink>,
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    /// This machine: a purpose is mirrored into the branch description, and
    /// a provider missing from this PATH is refused before Herdr is asked.
    local: bool,
}

impl WorktreeTarget {
    pub(crate) fn local(
        context: &LiveContext,
        host: Arc<dyn crate::node_access::NodeLink>,
    ) -> Self {
        Self {
            connector: Arc::clone(&context.api_connector),
            host,
            runtime: context.runtime.clone(),
            notifier: context.notifier.clone(),
            local: true,
        }
    }

    pub(crate) fn device(
        context: &RemoteControlContext,
        host: Arc<dyn crate::node_access::NodeLink>,
    ) -> Self {
        Self {
            connector: Arc::clone(&context.api_connector),
            host,
            runtime: context.runtime.clone(),
            notifier: context.notifier.clone(),
            local: false,
        }
    }
}

/// The Herdr a new tab and the agent started in it go to: this machine's or
/// a device's. A tab needs no file helper, so a device whose helper is not
/// ready can still take one (PRD home-device-rail D-22).
#[derive(Clone)]
pub struct TabTarget {
    connector: Arc<dyn ApiConnector>,
    connection_generation: Option<u64>,
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    /// The node whose PATH must hold the chosen agent, asked before Herdr
    /// is; none for a device, whose own Herdr answers for its PATH.
    agent_node: Option<Arc<dyn crate::node_access::NodeLink>>,
}

impl TabTarget {
    pub(crate) fn local(context: &LiveContext, generation: u64) -> Self {
        Self {
            connector: Arc::clone(&context.api_connector),
            runtime: context.runtime.clone(),
            notifier: context.notifier.clone(),
            agent_node: Some(Arc::clone(&context.node)),
            connection_generation: Some(generation),
        }
    }

    pub(crate) fn device(context: &RemoteControlContext) -> Self {
        Self {
            connector: Arc::clone(&context.api_connector),
            runtime: context.runtime.clone(),
            notifier: context.notifier.clone(),
            agent_node: None,
            connection_generation: None,
        }
    }

    fn check_current(&self, id: u64) -> Result<(), String> {
        let runtime = self.runtime.upgrade().ok_or("runtime stopped")?;
        let runtime = runtime.lock().map_err(|_| "runtime unavailable")?;
        if !runtime.task_session_control_is_current(id, &self.connector, self.connection_generation)
        {
            return Err("session_resume_control_changed".into());
        }
        Ok(())
    }
}

/// One off-lock host check admits the entire close sequence, including
/// descendants outside the checkout. A refused or unavailable check closes none.
pub fn spawn_worktree_preflight(
    target: WorktreeTarget,
    id: u64,
    outside: Vec<String>,
) -> Result<(), String> {
    thread::Builder::new().name("herdr-core-worktree-preflight".into()).spawn(move || {
        let Some(runtime) = target.runtime.upgrade() else { return; };
        let request = match runtime.lock() {
            Ok(guard) => guard.worktree_preflight_request(id),
            Err(_) => return,
        };
        let Some(removal) = request else { return; };
        let result = crate::node_access::call_as::<()>(target.host.as_ref(),
            hide_node_link::protocol::Call::WorktreeRemovalCheck { removal }, HOST_REMOVE_TIMEOUT)
            .map_err(|error| format!("{}. No panes were closed.", error.to_string().trim_end_matches('.')));
        crate::diagnostic!(serde_json::json!({"component":"worktree_removal", "kind":"preflight_finished", "id":id, "accepted":result.is_ok()}));
        if let Ok(mut guard) = runtime.lock() {
            guard.ingest_worktree_preflight_result(id, outside, result);
        }
        target.notifier.notify();
    }).map(|_| ()).map_err(|_| "Worktree preflight worker could not start. No panes were closed; retry the review.".to_owned())
}

pub fn spawn_worktree_close(
    target: WorktreeTarget,
    id: u64,
    checkout_path: String,
    pane_ids: Vec<String>,
) -> Result<(), String> {
    let context = target.clone();
    thread::Builder::new()
        .name("herdr-core-worktree-close".into())
        .spawn(move || {
            let result = close_checkout_panes(
                context.connector.as_ref(),
                context.host.as_ref(),
                std::slice::from_ref(&checkout_path),
                &pane_ids,
                ProcessWait::for_folder_removal(context.local),
                CONFIRM_TIMEOUT,
            );
            // This thread is the removal's only executor: it closes the panes,
            // takes the runtime's confirmation, and runs Git itself. No shell
            // reports a completion, so a client cannot claim one.
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let confirmed = match runtime.lock() {
                Ok(mut guard) => {
                    guard.ingest_worktree_close_result(id, &pane_ids, result);
                    guard.confirmed_worktree_removal(id)
                }
                Err(error) => {
                    trace(
                        &checkout_path,
                        &pane_ids,
                        "publish_failed",
                        Some(&error.to_string()),
                    );
                    return;
                }
            };
            context.notifier.notify();
            let Some(request) = confirmed else {
                return;
            };
            trace(&checkout_path, &pane_ids, "remove_started", None);
            let outcome = remove_on_host(context.host.as_ref(), request);
            trace(
                &checkout_path,
                &pane_ids,
                if outcome.is_ok() {
                    "remove_finished"
                } else {
                    "remove_failed"
                },
                outcome.as_ref().err().map(String::as_str),
            );
            match runtime.lock() {
                Ok(mut guard) => {
                    guard.ingest_worktree_removal_result(id, outcome);
                }
                Err(error) => {
                    trace(
                        &checkout_path,
                        &pane_ids,
                        "publish_failed",
                        Some(&error.to_string()),
                    );
                    return;
                }
            }
            context.notifier.notify();
        })
        .map(|_| ())
        .map_err(|error| format!("worktree close worker could not be started: {error}"))
}

/// Runs the confirmed removal on the repository's own host. An answer that
/// never came leaves the removal's effect unknown, which is not a success.
fn remove_on_host(
    host: &dyn crate::node_access::NodeLink,
    removal: hide_node_link::worktrees::ConfirmedRemoval,
) -> Result<String, String> {
    let path = removal.checkout_path.clone();
    match crate::node_access::call_as::<hide_node_link::worktrees::RemovalOutcome>(
        host,
        hide_node_link::protocol::Call::WorktreeRemove { removal },
        HOST_REMOVE_TIMEOUT,
    ) {
        Ok(outcome) if outcome.removed => Ok(outcome.message),
        Ok(outcome) => Err(outcome.message),
        Err(crate::node_access::LinkError::Unknown(reason)) => Err(format!(
            "The removal of {path} was sent but its result is unknown ({reason}). Review the worktree again before retrying."
        )),
        Err(error) => Err(format!(
            "Worktree removal stopped: {error}. The worktree remains; panes already closed stay closed."
        )),
    }
}

/// Asks the repository's host whether `branch` may be created; nothing is
/// created by the question.
fn check_new_branch(
    host: &dyn crate::node_access::NodeLink,
    repository_root: &str,
    branch: &str,
) -> Result<(), String> {
    crate::node_access::call_as::<()>(
        host,
        hide_node_link::protocol::Call::BranchCheck {
            path: repository_root.to_owned(),
            branch: branch.to_owned(),
        },
        HOST_CHECK_TIMEOUT,
    )
    .map_err(|error| error.to_string())
}

/// The real path of an existing directory on the repository's host, `None`
/// when there is none there. A host that did not answer is an error, never a
/// missing folder: a real worktree is not rolled back for a helper hiccup.
fn host_directory(
    host: &dyn crate::node_access::NodeLink,
    path: &str,
) -> Result<Option<String>, String> {
    crate::node_access::call_as::<Option<String>>(
        host,
        hide_node_link::protocol::Call::Directory {
            path: path.to_owned(),
        },
        HOST_CHECK_TIMEOUT,
    )
    .map_err(|error| error.to_string())
}

/// `Remove project…`: closes every pane in the project's checkouts and waits
/// for Herdr to confirm, the same handshake a worktree deletion uses, then
/// hands the answer to the runtime, which alone removes the registration.
pub fn spawn_workspace_close(
    context: WorktreeTarget,
    workspace_id: String,
    checkout_paths: Vec<String>,
    pane_ids: Vec<String>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-workspace-close".into())
        .spawn(move || {
            let result = close_checkout_panes(
                context.connector.as_ref(),
                context.host.as_ref(),
                &checkout_paths,
                &pane_ids,
                ProcessWait::Skip,
                CONFIRM_TIMEOUT,
            );
            if let Some(runtime) = context.runtime.upgrade() {
                match runtime.lock() {
                    Ok(mut guard) => {
                        guard.ingest_workspace_close_result(&workspace_id, result);
                    }
                    Err(error) => {
                        trace(
                            &checkout_paths.join(","),
                            &pane_ids,
                            "publish_failed",
                            Some(&error.to_string()),
                        );
                        return;
                    }
                }
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("workspace close worker could not be started: {error}"))
}

pub fn spawn_worktree_open(
    context: LiveContext,
    checkout_path: String,
    repository_root: String,
    pane_id: Option<String>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-worktree-open".into())
        .spawn(move || {
            let result = open_worktree(
                context.api_connector.as_ref(),
                &checkout_path,
                &repository_root,
                pane_id.as_deref(),
            );
            if let Some(runtime) = context.runtime.upgrade() {
                match runtime.lock() {
                    Ok(mut guard) => {
                        guard.ingest_worktree_open_result(checkout_path.clone(), result);
                    }
                    Err(error) => {
                        trace(
                            &checkout_path,
                            &[],
                            "publish_failed",
                            Some(&error.to_string()),
                        );
                        return;
                    }
                }
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("worktree open worker could not be started: {error}"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeTaskRequest {
    pub id: u64,
    pub repository_root: String,
    pub branch: String,
    pub base_branch: Option<String>,
    pub agent_kind: Option<String>,
    pub focus: bool,
    pub purpose: Option<String>,
    /// The issue token to link the new worktree to (`owner/repo#N` or
    /// `L-N`), written like `set_checkout_issue` writes one, on the
    /// worktree's own machine: this one's or a node's that dials the core,
    /// never a dialed device's, which the runtime leaves this empty for.
    pub issue: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeTaskOutcome {
    pub path: String,
    pub pane_id: String,
    /// The tab Hide opened in an existing checkout, so the runtime places it
    /// under `path` rather than under its pane's birth cwd.
    pub created_tab_id: Option<String>,
    /// Worktree creation succeeded even when its optional purpose did not.
    /// The runtime records this as a diagnostic without turning the finished
    /// creation into a failed operation.
    pub purpose_error: Option<String>,
    /// A token-first creation save whose Git write and compensating token
    /// clear both failed. The runtime hides this unconfirmed value so the
    /// completed row follows the creation contract and shows its fallback.
    pub unconfirmed_purpose_token: Option<String>,
    /// Why the requested issue link could not be written; the worktree
    /// exists either way.
    pub issue_error: Option<String>,
    /// How the created pane's shell says it has finished starting, which
    /// the task's agent start waits for.
    pub(crate) shell: crate::agent_start::ShellStart,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurposeTaskRequest {
    pub id: u64,
    pub checkout_id: String,
    pub repository_root: String,
    pub branch: Option<String>,
    pub session_workspace_id: Option<String>,
    pub purpose: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PurposeTaskOutcome {
    Saved {
        purpose: String,
        token_written: bool,
    },
    GitFailed {
        purpose: String,
        token_written: bool,
        detail: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PurposeMirrorRequest {
    workspace_id: String,
    repository_root: String,
    branch: String,
    purpose: Option<String>,
}

enum PurposeMirrorMessage {
    Write(PurposeMirrorRequest),
    RemoveIssue {
        repository_root: String,
        branch: String,
        workspace_ids: Vec<String>,
    },
    Stop,
}

/// Mirrors new live workspace purpose values into Git without delaying the
/// session coordinator or taking the runtime lock. Each observed token change
/// is queued once; a failed write is diagnosed and is not retried until a
/// later token change makes a new intent.
pub struct PurposeMirror {
    sender: SyncSender<PurposeMirrorMessage>,
    worker: Option<thread::JoinHandle<()>>,
    observed: BTreeMap<(String, String, String), Option<String>>,
    issue_worktrees: BTreeSet<(String, String, String)>,
}

impl PurposeMirror {
    const QUEUE_CAPACITY: usize = 64;

    /// `node` holds the repositories whose purposes are mirrored.
    pub fn new(
        connector: Arc<dyn ApiConnector>,
        node: Arc<dyn crate::node_access::NodeLink>,
    ) -> Result<Self, String> {
        let (sender, receiver) = sync_channel(Self::QUEUE_CAPACITY);
        let worker = thread::Builder::new()
            .name("herdr-core-purpose-mirror".to_owned())
            .spawn(move || {
                let git = NodeGit(node.as_ref());
                while let Ok(message) = receiver.recv() {
                    let request = match message {
                        PurposeMirrorMessage::Write(request) => request,
                        PurposeMirrorMessage::Stop => break,
                        PurposeMirrorMessage::RemoveIssue { repository_root, branch, workspace_ids } => {
                            for id in workspace_ids {
                                let result = wire::workspace_issue_params(&id, None).and_then(|params| control_request(connector.as_ref(), "workspace.report_metadata", params));
                                if let Err(error) = result {
                                    crate::diagnostic!(serde_json::json!({"component":"checkout_issue","kind":"removed_worktree.token_cleanup_failed","workspace_id":id,"message":error}));
                                }
                            }
                            if let Err(error) = git.run(&repository_root, GitCommand::UnsetBranchConfig { branch, key: BranchConfigKey::Issue }) {
                                crate::diagnostic!(serde_json::json!({"component":"checkout_issue","kind":"removed_worktree.cleanup_failed","message":error}));
                            }
                            continue;
                        }
                    };
                    let branch = request.branch.clone();
                    let key = BranchConfigKey::Description;
                    let command = match request.purpose.clone() {
                        Some(value) => GitCommand::SetBranchConfig { branch, key, value },
                        None => GitCommand::UnsetBranchConfig { branch, key },
                    };
                    let result = git.run(&request.repository_root, command).map(|_| ());
                    match result {
                        Ok(()) => crate::diagnostic!(serde_json::json!({
                            "component": "checkout_purpose",
                            "kind": "mirror.saved",
                            "workspace_id": request.workspace_id,
                            "branch": request.branch,
                        })),
                        Err(message) => crate::diagnostic!(serde_json::json!({
                            "component": "checkout_purpose",
                            "kind": "mirror.failed",
                            "workspace_id": request.workspace_id,
                            "branch": request.branch,
                            "message": message,
                        })),
                    }
                }
            })
            .map_err(|error| format!("purpose mirror worker could not be started: {error}"))?;
        Ok(Self {
            sender,
            worker: Some(worker),
            observed: BTreeMap::new(),
            issue_worktrees: BTreeSet::new(),
        })
    }

    #[cfg(test)]
    fn recording() -> (Self, std::sync::mpsc::Receiver<PurposeMirrorMessage>) {
        let (sender, receiver) = sync_channel(Self::QUEUE_CAPACITY);
        (
            Self {
                sender,
                worker: None,
                observed: BTreeMap::new(),
                issue_worktrees: BTreeSet::new(),
            },
            receiver,
        )
    }

    pub fn sync(
        &mut self,
        spaces: &[workspace::SessionSpace],
        workspaces: &[WorkspaceSnapshot],
        unconfirmed_created_purposes: &HashMap<String, String>,
    ) {
        let current_issues: BTreeSet<_> = workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none())
            .flat_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .filter(|checkout| checkout.is_worktree)
                    .filter_map(|checkout| {
                        checkout.branch.as_ref().map(|branch| {
                            (
                                workspace.path.clone(),
                                checkout.path.clone(),
                                branch.clone(),
                            )
                        })
                    })
            })
            .collect();
        let mut retained = current_issues.clone();
        for (root, path, branch) in self.issue_worktrees.difference(&current_issues) {
            let project = workspaces.iter().find(|workspace| &workspace.path == root);
            if project.is_some_and(|project| {
                project
                    .checkouts
                    .iter()
                    .any(|checkout| &checkout.path == path && checkout.branch.is_none())
            }) {
                // Detaching keeps the last branch's cleanup ownership until removal.
                retained.insert((root.clone(), path.clone(), branch.clone()));
                continue;
            }
            // A branch switch or detached HEAD is not a removed worktree.
            if project.is_some_and(|project| {
                !project
                    .checkouts
                    .iter()
                    .any(|checkout| &checkout.path == path)
            }) && let Err(error) = self.sender.try_send(PurposeMirrorMessage::RemoveIssue {
                repository_root: root.clone(),
                branch: branch.clone(),
                workspace_ids: spaces
                    .iter()
                    .filter(|space| {
                        space
                            .cwds
                            .iter()
                            .any(|cwd| Path::new(cwd).starts_with(path))
                    })
                    .map(|space| space.id.clone())
                    .collect(),
            }) {
                retained.insert((root.clone(), path.clone(), branch.clone()));
                crate::diagnostic!(
                    serde_json::json!({"component":"checkout_issue","kind":"cleanup.queue_unavailable","message":error.to_string()})
                );
            }
        }
        self.issue_worktrees = retained;
        let mut current = BTreeSet::new();
        for workspace in workspaces {
            for checkout in &workspace.checkouts {
                let Some(branch) = checkout.branch.as_deref() else {
                    continue;
                };
                let Some(effective) =
                    workspace::effective_checkout_purpose(spaces, workspace, &checkout.path)
                else {
                    continue;
                };
                let key = (
                    workspace::comparison_by_names(Path::new(&workspace.path)),
                    workspace::comparison_by_names(Path::new(&checkout.path)),
                    branch.to_owned(),
                );
                current.insert(key.clone());
                let purpose = effective.purpose.map(str::to_owned);
                if purpose.as_deref().is_some_and(|purpose| {
                    unconfirmed_created_purposes
                        .get(&key.1)
                        .is_some_and(|unconfirmed| unconfirmed == purpose)
                }) {
                    // Do not remember the suppressed value as mirrored. Once
                    // Herdr confirms a clear or replacement, the next sync
                    // must treat that visible value as new work.
                    self.observed.remove(&key);
                    continue;
                }
                let previous = self.observed.get(&key);
                let first_live_value =
                    previous.is_none() && (purpose.is_some() || effective.has_shadowed_purpose);
                let changed_value = previous.is_some_and(|value| value != &purpose);
                self.observed.insert(key, purpose.clone());
                if !first_live_value && !changed_value {
                    continue;
                }
                let request = PurposeMirrorRequest {
                    workspace_id: effective.workspace_id.to_owned(),
                    repository_root: workspace.path.clone(),
                    branch: branch.to_owned(),
                    purpose,
                };
                if let Err(error) = self.sender.try_send(PurposeMirrorMessage::Write(request)) {
                    let kind = match error {
                        TrySendError::Full(_) => "mirror.queue_full",
                        TrySendError::Disconnected(_) => "mirror.worker_closed",
                    };
                    crate::diagnostic!(serde_json::json!({
                        "component": "checkout_purpose",
                        "kind": kind,
                        "workspace_id": effective.workspace_id,
                    }));
                }
            }
        }
        self.observed.retain(|key, _| current.contains(key));
    }
}

impl Drop for PurposeMirror {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = self.sender.send(PurposeMirrorMessage::Stop);
            if worker.join().is_err() {
                crate::diagnostic!(serde_json::json!({
                    "component": "checkout_purpose",
                    "kind": "mirror.join_failed",
                }));
            }
        }
    }
}

pub fn spawn_worktree_create(
    context: WorktreeTarget,
    request: WorktreeTaskRequest,
) -> Result<(), String> {
    spawn_worktree_task(context, request, |context, request| {
        check_new_branch(
            context.host.as_ref(),
            &request.repository_root,
            &request.branch,
        )
    })
}

/// A worktree on a branch that already exists, a pull request's (PRD
/// overview-lenses-prs D-46): Herdr checks out an existing local branch, and
/// a branch only `origin` has is fetched first and made from its
/// remote-tracking ref, which git sets as the new branch's upstream. No other
/// branch name is ever made. The checks run on the repository's own machine
/// (`context.host`), this one's or a node's.
pub fn spawn_existing_branch_worktree(
    context: WorktreeTarget,
    request: WorktreeTaskRequest,
) -> Result<(), String> {
    spawn_worktree_task(context, request, |context, request| {
        request.base_branch = existing_branch_base(
            &NodeGit(context.host.as_ref()),
            &request.repository_root,
            &request.branch,
        )?;
        Ok(())
    })
}

/// `None` when `branch` is a local branch, else `origin/<branch>` once it
/// has been fetched from `origin`.
fn existing_branch_base(
    git: &dyn GitCommands,
    repository_root: &str,
    branch: &str,
) -> Result<Option<String>, String> {
    let branch = branch.to_owned();
    let has_local = GitCommand::HasLocalBranch {
        branch: branch.clone(),
    };
    if git.run(repository_root, has_local).is_ok() {
        return Ok(None);
    }
    git.run(
        repository_root,
        GitCommand::FetchBranch {
            branch: branch.clone(),
        },
    )
    .map_err(|error| format!("fetch {branch} from origin: {error}"))?;
    Ok(Some(format!("origin/{branch}")))
}

fn spawn_worktree_task(
    context: WorktreeTarget,
    mut request: WorktreeTaskRequest,
    prepare: impl FnOnce(&WorktreeTarget, &mut WorktreeTaskRequest) -> Result<(), String>
    + Send
    + 'static,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-worktree-create".into())
        .spawn(move || {
            let result = prepare(&context, &mut request).and_then(|()| {
                create_worktree_observing_purpose(
                    context.connector.as_ref(),
                    context.host.as_ref(),
                    context.local,
                    &request,
                    |path, purpose| {
                        if let Some(runtime) = context.runtime.upgrade()
                            && let Ok(mut guard) = runtime.lock()
                        {
                            guard.begin_created_purpose_write(path, purpose);
                        }
                    },
                )
            });
            if let Some(runtime) = context.runtime.upgrade() {
                if let Ok(mut guard) = runtime.lock() {
                    guard.ingest_task_operation_result(request.id, result);
                } else {
                    return;
                }
                context.notifier.notify();
            }
            start_task_agent(
                &context.connector,
                &context.runtime,
                &context.notifier,
                context.local.then_some(context.host.as_ref()),
                request.id,
                &|| Ok(()),
            );
        })
        .map(|_| ())
        .map_err(|error| format!("worktree create worker could not be started: {error}"))
}

pub fn spawn_purpose_write(
    context: LiveContext,
    request: PurposeTaskRequest,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-purpose-write".into())
        .spawn(move || {
            let git = NodeGit(context.node.as_ref());
            let result = write_purpose(context.api_connector.as_ref(), &git, &request);
            if let Some(runtime) = context.runtime.upgrade() {
                if let Ok(mut guard) = runtime.lock() {
                    guard.ingest_purpose_operation_result(request.id, result);
                } else {
                    return;
                }
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("purpose worker could not be started: {error}"))
}

#[derive(Clone, Debug)]
pub struct IssueWriteFailure {
    pub detail: String,
    pub unconfirmed_token: bool,
}
impl IssueWriteFailure {
    pub fn unchanged(detail: String) -> Self {
        Self {
            detail,
            unconfirmed_token: false,
        }
    }
}

fn write_issue_metadata(
    connector: &dyn ApiConnector,
    git: &dyn GitCommands,
    request: &PurposeTaskRequest,
    previous: Option<&str>,
) -> Result<(), IssueWriteFailure> {
    let (detail, token_may_have_changed) =
        match write_workspace_metadata(connector, git, request, "issue", BranchConfigKey::Issue) {
            Ok(PurposeTaskOutcome::Saved { .. }) => return Ok(()),
            Ok(PurposeTaskOutcome::GitFailed {
                detail,
                token_written,
                ..
            }) => (detail, token_written),
            // A transport failure can arrive after the server accepted the token.
            Err(detail) => (detail, request.session_workspace_id.is_some()),
        };
    if token_may_have_changed && let Some(workspace_id) = request.session_workspace_id.as_deref() {
        let rollback = wire::workspace_issue_params(workspace_id, previous).and_then(|params| {
            control_request(connector, "workspace.report_metadata", params).map(|_| ())
        });
        if let Err(error) = rollback {
            return Err(IssueWriteFailure {
                detail: format!("{detail}; issue rollback failed: {error}"),
                unconfirmed_token: true,
            });
        }
    }
    Err(IssueWriteFailure::unchanged(detail))
}

/// Where `gh` reads a project's repository: with this machine's login
/// (`node`, the core's own), in the checkout here or by the repository's
/// name for a project on a node (D-16).
#[derive(Clone)]
pub(crate) struct GhPlace {
    pub node: Arc<dyn crate::node_access::NodeLink>,
    pub place: crate::github::Where,
}

/// A checkout's issue link: the branch's `issue` note and its Workspace
/// token, written on the checkout's own machine (`target`: this machine's
/// Herdr and node, or a node's), never another's git. A GitHub link reads
/// its issue first (`read`); a Local one names an issue already in the
/// core's store, so nothing is read.
pub(crate) fn spawn_issue_write(
    target: WorktreeTarget,
    read: Option<GhPlace>,
    request: PurposeTaskRequest,
    previous: Option<String>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-issue-write".into())
        .spawn(move || {
            let result = (|| {
                let issue = match read {
                    Some(read) if !request.purpose.is_empty() => {
                        let reference =
                            crate::issues::IssueReference::parse(&request.purpose, None)
                                .map_err(IssueWriteFailure::unchanged)?;
                        Some(
                            crate::github::read_linked_issue(
                                read.node.as_ref(),
                                &read.place,
                                &reference,
                            )
                            .map_err(IssueWriteFailure::unchanged)?,
                        )
                    }
                    _ => None,
                };
                write_issue_metadata(
                    target.connector.as_ref(),
                    &NodeGit(target.host.as_ref()),
                    &request,
                    previous.as_deref(),
                )?;
                Ok(issue)
            })();
            if let Some(runtime) = target.runtime.upgrade() {
                if let Ok(mut guard) = runtime.lock() {
                    guard.ingest_issue_operation_result(&request, result);
                }
                target.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("issue worker could not be started: {error}"))
}

pub fn spawn_remote_purpose_write(
    context: RemoteControlContext,
    request: PurposeTaskRequest,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-remote-purpose-write".into())
        .spawn(move || {
            let git = HerdrOnly;
            let result = write_purpose(context.api_connector.as_ref(), &git, &request);
            if let Some(runtime) = context.runtime.upgrade() {
                if let Ok(mut guard) = runtime.lock() {
                    guard.ingest_purpose_operation_result(request.id, result);
                } else {
                    return;
                }
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("remote purpose worker could not be started: {error}"))
}

/// `New agent here`: one new tab whose cwd is the checkout, in the checkout's
/// owner Herdr workspace, opened first when none is open (PRD
/// checkout-workspace-binding D-07). The result lands in the same task
/// operation slot the worktree sheet uses, and the shell starts the provider
/// in the returned pane exactly as it does there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckoutTabRequest {
    pub id: u64,
    pub agent_kind: Option<String>,
    pub checkout_path: String,
    pub label: String,
    pub host: crate::checkout_owner::TabHost,
    /// Whether Herdr focuses the tab it makes; false for a start that stays
    /// behind the operator's screen.
    pub focus: bool,
    pub resume_scope: Option<hide_session::SessionReadScope>,
    pub(crate) resume_reference: Option<crate::sidebar::SessionAgentSessionPayload>,
    /// The folder the tab's shell starts in: the checkout, or a folder inside
    /// it where a resumed conversation ran, since a CLI may find its
    /// conversations by folder; a folder that is gone refuses the start.
    pub cwd: String,
}

/// What the task's worker starts once the creation is settled: the pane, the
/// agent kind, its first prompt and its CLI arguments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingAgentStart {
    pub pane_id: String,
    pub kind: String,
    pub prompt: Option<String>,
    pub args: Vec<String>,
    pub resume_scope: Option<hide_session::SessionReadScope>,
    pub(crate) resume_reference: Option<crate::sidebar::SessionAgentSessionPayload>,
    pub(crate) codex_daemon: crate::codex_launch::CodexDaemon,
    pub(crate) shell: crate::agent_start::ShellStart,
    /// The worktree this task made, which an agent that asks for folder
    /// trust is told to trust before it starts there.
    pub(crate) made_worktree: Option<String>,
}

/// How starting the chosen agent in a task's created pane ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskAgentOutcome {
    Started,
    /// Herdr refused, or the agent is not installed; nothing started.
    Failed(String),
    /// Herdr did not answer; the agent may be running. The pane says which.
    Unknown(String),
}

/// `agent.start` waits up to this long for the pane's shell; the read waits a
/// little longer so a slow start is not reported as an unknown one.
const AGENT_START_TIMEOUT_MS: u64 = 120_000;

/// A PATH lookup on the node: a directory walk, nothing that should wait.
const AGENT_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Starts the agent the task chose, in the pane the task created, and hands
/// the answer back on its own axis. The creation was already published, so a
/// failure here never hides the worktree or the pane.
fn start_task_agent(
    connector: &Arc<dyn ApiConnector>,
    runtime: &Weak<Mutex<Runtime>>,
    notifier: &ChangeNotifier,
    agent_node: Option<&dyn crate::node_access::NodeLink>,
    id: u64,
    current: &(dyn Fn() -> Result<(), String> + Sync),
) {
    let Some(runtime) = runtime.upgrade() else {
        return;
    };
    let pending = match runtime.lock() {
        Ok(guard) => guard.pending_task_agent_start(id),
        Err(_) => return,
    };
    let Some(start) = pending else {
        return;
    };
    let generation = runtime
        .lock()
        .ok()
        .and_then(|runtime| runtime.task_session_control_generation(id, connector));
    let reader = start
        .resume_scope
        .as_ref()
        .map(|_| task_session_node(&runtime, id, &start.kind))
        .transpose();
    // The node that runs the task's pane, this machine's or a device's, which
    // a long start line asks whether the pane's shell is reading.
    let pane_node = task_session_node(&runtime, id, &start.kind).ok();
    let outcome = match reader {
        Err(reason) => TaskAgentOutcome::Failed(reason),
        Ok(reader) => {
            let expected = start.clone();
            let current_start = || {
                if let Some(reader) = reader.as_ref() {
                    current()?;
                    if !runtime
                        .lock()
                        .map_err(|_| "runtime unavailable")?
                        .task_session_control_is_current(id, connector, generation)
                    {
                        return Err("session_resume_control_changed".into());
                    }
                    let pending = runtime
                        .lock()
                        .map_err(|_| "runtime unavailable")?
                        .pending_task_agent_start(id);
                    if pending.as_ref() != Some(&expected)
                        || !Arc::ptr_eq(&task_session_node(&runtime, id, &expected.kind)?, reader)
                    {
                        return Err("session_resume_intent_changed".into());
                    }
                }
                Ok(())
            };
            let checked = CurrentSessionConnector {
                connector: connector.as_ref(),
                current: &current_start,
            };
            let launch_connector: &dyn ApiConnector = if reader.is_some() {
                &checked
            } else {
                connector.as_ref()
            };
            launch_with_prompt_checked(
                launch_connector,
                reader.as_deref().or(agent_node),
                pane_node.as_deref(),
                id,
                start,
                &current_start,
            )
        }
    };
    if let Ok(mut guard) = runtime.lock() {
        guard.ingest_task_agent_result(id, outcome);
    } else {
        return;
    }
    notifier.notify();
}

/// Starts the agent with its first prompt (PRD home-device-rail D-26).
///
/// The prompt goes as the CLI's declared argument (`--prompt` for OpenCode,
/// positional after `--` for the other launch dialects), held through startup questions (folder trust, sign-in)
/// and send it once those are answered. Nothing is ever typed into the pane,
/// because Herdr reports an agent on such a question as idle and ready, and a
/// typed prompt would answer the question. A prompt that cannot be passed,
/// or a start Herdr refuses, fails the start with the reason; the surface
/// that sent it keeps the text.
#[cfg(test)]
fn launch_with_prompt(
    connector: &dyn ApiConnector,
    agent_node: Option<&dyn crate::node_access::NodeLink>,
    id: u64,
    start: PendingAgentStart,
) -> TaskAgentOutcome {
    launch_with_prompt_checked(connector, agent_node, agent_node, id, start, &|| Ok(()))
}

fn launch_with_prompt_checked(
    connector: &dyn ApiConnector,
    agent_node: Option<&dyn crate::node_access::NodeLink>,
    pane_node: Option<&dyn crate::node_access::NodeLink>,
    id: u64,
    start: PendingAgentStart,
    current: &dyn Fn() -> Result<(), String>,
) -> TaskAgentOutcome {
    let PendingAgentStart {
        pane_id,
        kind,
        prompt,
        mut args,
        codex_daemon,
        resume_scope,
        resume_reference,
        shell,
        made_worktree,
    } = start;
    // The trust is written by the machine that runs the pane: `agent_node`
    // is none for a device, whose own Herdr answers for its PATH, and a
    // device's worktree still needs its trust recorded there.
    if let Some(path) = made_worktree.as_deref()
        && let Err(reason) = crate::agent_start::trust_worktree(pane_node, &kind, path)
    {
        return TaskAgentOutcome::Failed(reason);
    }
    if hide_agent_adapter::adapter(&kind)
        .filter(|adapter| {
            adapter
                .session
                .is_some_and(|format| format.requires_native_proof())
        })
        .and_then(|adapter| adapter.resume)
        .is_some_and(|dialect| args.first().map(String::as_str) == Some(dialect.resume_flag()))
        && !resume_scope
            .as_ref()
            .is_some_and(|scope| args.get(1) == Some(&scope.id))
    {
        return TaskAgentOutcome::Failed("session_resume_scope_required".into());
    }
    let Some(dialect) = hide_agent_adapter::adapter(&kind).and_then(|row| row.start) else {
        return TaskAgentOutcome::Failed("This agent has no supported start dialect".to_owned());
    };
    if let Some(prompt) = prompt {
        match prompt_argument(&prompt) {
            Ok(argument) => args.extend([dialect.prompt_flag().to_owned(), argument]),
            Err(message) => return TaskAgentOutcome::Failed(message),
        }
    }
    launch_agent(
        connector,
        agent_node,
        pane_node,
        id,
        &pane_id,
        shell,
        &kind,
        args,
        codex_daemon,
        &|| {
            current()?;
            if let Some(scope) = resume_scope.as_ref() {
                crate::live::confirm_session_launch(
                    agent_node.ok_or("session_route_reader_unavailable")?,
                    &kind,
                    &scope.id,
                    Some(&scope.cwd),
                    resume_reference.as_ref(),
                )?;
            }
            current()
        },
    )
    .into()
}

/// The longest first prompt, in bytes once encoded, that a start carries.
/// Herdr types the command into the pane's shell, and the slowest shell
/// measured bounds it: through the pinned Herdr, macOS bash took 27 s to take
/// a 76 KB command and did not finish 300 KB within 60 s, where zsh took 5 s;
/// 64 KB stays well inside the 120 s start wait on either.
const MAX_PROMPT_BYTES: usize = 64 * 1024;

/// The first prompt as one argument Herdr can type into the pane's shell,
/// which refuses a line break or a tab in an argument. Each line break becomes
/// U+2028 (LINE SEPARATOR), which Herdr passes through and the model reads as
/// a line break, and a tab four spaces; any other control character is refused
/// rather than dropped, and so is a prompt over [`MAX_PROMPT_BYTES`]. Every
/// start event checks its prompt with this before anything is created, so a
/// refused prompt leaves no tab or worktree behind; the worker encodes it.
pub(crate) fn prompt_argument(prompt: &str) -> Result<String, String> {
    let mut argument = String::with_capacity(prompt.len());
    let mut chars = prompt.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                chars.next_if_eq(&'\n');
                argument.push('\u{2028}');
            }
            '\n' => argument.push('\u{2028}'),
            '\t' => argument.push_str("    "),
            c if c.is_control() => {
                return Err(format!(
                    "The first prompt has a control character (U+{:04X}) that cannot be passed to the agent; remove it and start again.",
                    u32::from(c)
                ));
            }
            c => argument.push(c),
        }
    }
    if argument.len() > MAX_PROMPT_BYTES {
        return Err(format!(
            "The first prompt is longer than {} KB, more than the agent's command line can carry; shorten it and start again.",
            MAX_PROMPT_BYTES / 1024
        ));
    }
    Ok(argument)
}

pub fn spawn_task_agent_start(context: WorktreeTarget, id: u64) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-task-agent-start".into())
        .spawn(move || {
            start_task_agent(
                &context.connector,
                &context.runtime,
                &context.notifier,
                context.local.then_some(context.host.as_ref()),
                id,
                &|| Ok(()),
            )
        })
        .map(|_| ())
        .map_err(|error| format!("agent start worker could not be started: {error}"))
}

/// How one `agent.start` ended; a refusal keeps Herdr's code so a caller can
/// tell a refusal that typed nothing and can be asked differently.
enum Launch {
    Started,
    Failed(String),
    Refused { code: String, message: String },
    Unknown(String),
}

impl From<Launch> for TaskAgentOutcome {
    fn from(launch: Launch) -> Self {
        match launch {
            Launch::Started => TaskAgentOutcome::Started,
            Launch::Failed(message) => TaskAgentOutcome::Failed(message),
            Launch::Refused { code, message } => {
                TaskAgentOutcome::Failed(format!("Agent could not start: {code}: {message}"))
            }
            Launch::Unknown(message) => TaskAgentOutcome::Unknown(message),
        }
    }
}

#[allow(clippy::too_many_arguments)] // the task/pane launch identity plus its effect admission
fn launch_agent(
    connector: &dyn ApiConnector,
    agent_node: Option<&dyn crate::node_access::NodeLink>,
    pane_node: Option<&dyn crate::node_access::NodeLink>,
    id: u64,
    pane_id: &str,
    shell: crate::agent_start::ShellStart,
    kind: &str,
    args: Vec<String>,
    codex_daemon: crate::codex_launch::CodexDaemon,
    check: &dyn Fn() -> Result<(), String>,
) -> Launch {
    if let Err(reason) = check() {
        return Launch::Failed(reason);
    }
    // Herdr would type the command into the pane's shell and wait for an
    // agent that can never appear; say so before asking it. A device's PATH
    // is not this machine's, so there its Herdr answers for it.
    if let Some(node) = agent_node {
        match crate::node_access::call_as::<bool>(
            node,
            Call::AgentInstalled {
                name: kind.to_owned(),
            },
            AGENT_CHECK_TIMEOUT,
        ) {
            Ok(true) => {}
            Ok(false) => {
                return Launch::Failed(format!(
                    "{kind} is not installed on the daemon's PATH. Install it, then retry."
                ));
            }
            Err(error) => {
                return Launch::Failed(format!(
                    "Could not check whether {kind} is installed ({error}). Retry."
                ));
            }
        }
    }
    let name = crate::fork::task_agent_name(kind, pane_id);
    let params = match wire::agent_start_params(pane_id, &name, kind, args, codex_daemon) {
        Ok(params) => params,
        Err(message) => return Launch::Failed(message),
    };
    match crate::agent_start::start_at_shell_checked(
        connector,
        pane_node,
        &format!("herdr-core:task:{id}:agent"),
        pane_id,
        shell,
        params,
        Duration::from_millis(AGENT_START_TIMEOUT_MS + 5_000),
        check,
    ) {
        Ok(value) => match wire::started_agent(value) {
            Ok(_) => Launch::Started,
            Err(message) => Launch::Unknown(format!(
                "Herdr answered the agent start in an unexpected shape ({message}). Check the pane before retrying."
            )),
        },
        Err(StartError::NotStarted(message)) => Launch::Failed(message),
        Err(StartError::ShellStarting(screen)) => Launch::Failed(
            crate::agent_start::shell_starting_message(screen.as_deref()),
        ),
        Err(StartError::Herdr(ApiError::Remote { code, message })) => {
            Launch::Refused { code, message }
        }
        Err(StartError::Herdr(error)) => Launch::Unknown(format!(
            "Herdr did not confirm the agent start ({error}). Check the pane before retrying."
        )),
    }
}

pub fn spawn_checkout_tab_create(
    target: TabTarget,
    request: CheckoutTabRequest,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-checkout-tab-create".into())
        .spawn(move || open_tab_and_start_agent(&target, &request))
        .map(|_| ())
        .map_err(|error| format!("checkout tab worker could not be started: {error}"))
}

/// Opens the task's tab, publishes it, then starts the task's agent in it.
/// The tab's shell is started with Hide's shell integration, written by the
/// node of the machine that runs it (`agent_start::shell_environment`).
fn open_tab_and_start_agent(target: &TabTarget, request: &CheckoutTabRequest) {
    let node = target
        .runtime
        .upgrade()
        .and_then(|runtime| task_pane_node(&runtime, request.id).ok());
    let launch = crate::agent_start::shell_launch(
        node.as_deref(),
        &format!("herdr-core:task:{}:tab", request.id),
    );
    let result = (|| {
        if let Some(node) = node.as_deref() {
            folder_present(node, request)?;
        }
        if let Some(scope) = request.resume_scope.as_ref() {
            target.check_current(request.id)?;
            let runtime = target.runtime.upgrade().ok_or("runtime stopped")?;
            let kind = request
                .agent_kind
                .as_deref()
                .ok_or("session_resume_agent_required")?;
            let node = task_session_node(&runtime, request.id, kind)?;
            crate::live::confirm_session_launch(
                node.as_ref(),
                kind,
                &scope.id,
                Some(&scope.cwd),
                request.resume_reference.as_ref(),
            )?;
            if !Arc::ptr_eq(&node, &task_session_node(&runtime, request.id, kind)?) {
                return Err("session_resume_reader_changed".into());
            }
            target.check_current(request.id)?;
            let current = || {
                target.check_current(request.id)?;
                if !Arc::ptr_eq(&node, &task_session_node(&runtime, request.id, kind)?) {
                    return Err("session_resume_reader_changed".into());
                }
                Ok(())
            };
            return create_checkout_tab(
                &CurrentSessionConnector {
                    connector: target.connector.as_ref(),
                    current: &current,
                },
                request,
                launch.as_ref(),
            );
        }
        create_checkout_tab(target.connector.as_ref(), request, launch.as_ref())
    })();
    let Some(runtime) = target.runtime.upgrade() else {
        return;
    };
    if let Ok(mut guard) = runtime.lock() {
        guard.ingest_task_operation_result(request.id, result);
    } else {
        return;
    }
    drop(runtime);
    target.notifier.notify();
    start_task_agent(
        &target.connector,
        &target.runtime,
        &target.notifier,
        target.agent_node.as_deref(),
        request.id,
        &|| target.check_current(request.id),
    );
}

/// A tab asked for a folder inside its checkout opens there or not at all:
/// a conversation whose folder was removed since is refused with that
/// reason, as a wake is, and never started somewhere it was not. Asked of
/// the node of the machine that runs the tab, off the lock.
fn folder_present(
    node: &dyn crate::node_access::NodeLink,
    request: &CheckoutTabRequest,
) -> Result<(), String> {
    if request.cwd != request.checkout_path && !crate::node_access::is_directory(node, &request.cwd)
    {
        return Err(format!(
            "The working folder {} no longer exists",
            request.cwd
        ));
    }
    Ok(())
}

fn task_session_node(
    runtime: &Arc<Mutex<Runtime>>,
    id: u64,
    kind: &str,
) -> Result<Arc<dyn crate::node_access::NodeLink>, String> {
    let mut runtime = runtime.lock().map_err(|_| "runtime unavailable")?;
    let operation = runtime
        .snapshot()
        .task_operation
        .as_ref()
        .filter(|operation| operation.id == id)
        .ok_or("task superseded")?;
    if operation.agent_kind.as_deref() != Some(kind) {
        return Err("session_resume_intent_changed".into());
    }
    task_node(&mut runtime, id)
}

/// The node of the machine that runs task `id`'s pane.
fn task_pane_node(
    runtime: &Arc<Mutex<Runtime>>,
    id: u64,
) -> Result<Arc<dyn crate::node_access::NodeLink>, String> {
    let mut runtime = runtime.lock().map_err(|_| "runtime unavailable")?;
    task_node(&mut runtime, id)
}

fn task_node(
    runtime: &mut Runtime,
    id: u64,
) -> Result<Arc<dyn crate::node_access::NodeLink>, String> {
    let operation = runtime
        .snapshot()
        .task_operation
        .as_ref()
        .filter(|operation| operation.id == id)
        .ok_or("task superseded")?;
    let device = operation
        .device_id
        .clone()
        .unwrap_or_else(|| runtime.node().as_str().into());
    runtime.node_link(&device)
}

/// How long the helper may take to bring a Home in step: a stat per
/// project and a link each, at most [`hide_node_link::home::MAX_LINKS`].
const HOME_SYNC_TIMEOUT: Duration = Duration::from_secs(30);

/// A Home start or a new tab in Home (PRD home-device-rail D-04): the
/// device's registered projects its Home links, and the helper that makes
/// them.
pub struct HomeStartRequest {
    pub id: u64,
    pub device_id: String,
    pub projects: Vec<String>,
    pub host: Arc<dyn crate::node_access::NodeLink>,
}

/// Brings the device's Home in step with its projects, then opens a tab in
/// it and starts the task's agent there. The disk work runs on the helper,
/// off the runtime lock; the lock is taken only to publish each step.
pub fn spawn_home_start(target: TabTarget, request: HomeStartRequest) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-home-start".into())
        .spawn(move || {
            let HomeStartRequest {
                id,
                device_id,
                projects,
                host,
            } = request;
            let synced = sync_home(host.as_ref(), &projects);
            let Some(runtime) = target.runtime.upgrade() else {
                return;
            };
            let tab = match runtime.lock() {
                Ok(mut guard) => guard.ingest_home_start_sync(id, &device_id, &projects, synced),
                Err(_) => return,
            };
            drop(runtime);
            target.notifier.notify();
            if let Some(tab) = tab {
                open_tab_and_start_agent(&target, &tab);
            }
        })
        .map(|_| ())
        .map_err(|error| format!("Home start worker could not be started: {error}"))
}

/// Brings a device's Home links in step after its registrations changed
/// (D-06). Nothing reaches the screen: the answer is the diagnostic log's.
pub fn spawn_home_link_sync(
    runtime: Weak<Mutex<Runtime>>,
    device_id: String,
    projects: Vec<String>,
    host: Arc<dyn crate::node_access::NodeLink>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-home-link-sync".into())
        .spawn(move || {
            let synced = sync_home(host.as_ref(), &projects);
            if let Some(runtime) = runtime.upgrade()
                && let Ok(mut guard) = runtime.lock()
            {
                guard.ingest_home_link_sync(&device_id, &projects, synced);
            }
        })
        .map(|_| ())
        .map_err(|error| format!("Home link sync worker could not be started: {error}"))
}

fn sync_home(
    host: &dyn crate::node_access::NodeLink,
    projects: &[String],
) -> Result<hide_node_link::home::HomeSynced, crate::node_access::LinkError> {
    crate::node_access::call_as(
        host,
        hide_node_link::protocol::Call::HomeSync {
            projects: projects.to_vec(),
        },
        HOME_SYNC_TIMEOUT,
    )
}

fn create_checkout_tab(
    connector: &dyn ApiConnector,
    request: &CheckoutTabRequest,
    launch: Option<&hide_node_link::protocol::ShellLaunch>,
) -> Result<WorktreeTaskOutcome, String> {
    let env = launch.map(|launch| launch.env.clone()).unwrap_or_default();
    let command = launch.map(|launch| launch.command.as_slice());
    let tab = match &request.host {
        crate::checkout_owner::TabHost::Workspace(workspace_id) => super::create_tab_in(
            connector,
            workspace_id,
            &request.cwd,
            &request.label,
            request.focus,
            env,
            command,
        ),
        crate::checkout_owner::TabHost::Open(owner) => super::open_owner_tab(
            connector,
            owner,
            &request.cwd,
            &request.label,
            request.focus,
            env,
            command,
        ),
    }
    .map_err(|failure| failure.message().to_owned())?;
    // A tab that does not run the launch (Herdr's own shell, or no launch
    // for this shell) gives no mark.
    let shell = crate::agent_start::ShellStart::of_tab(tab.launched, launch);
    Ok(WorktreeTaskOutcome {
        path: request.checkout_path.clone(),
        pane_id: tab.pane_id,
        created_tab_id: Some(tab.tab_id),
        purpose_error: None,
        unconfirmed_purpose_token: None,
        issue_error: None,
        shell,
    })
}

pub fn spawn_branch_migration(
    context: LiveContext,
    host: Arc<dyn crate::node_access::NodeLink>,
    request: WorktreeTaskRequest,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-branch-migrate".into())
        .spawn(move || {
            let runner = NodeGit(host.as_ref());
            let result = migrate_branch(
                context.api_connector.as_ref(),
                host.as_ref(),
                &runner,
                &request,
            );
            if let Some(runtime) = context.runtime.upgrade() {
                if let Ok(mut guard) = runtime.lock() {
                    guard.ingest_task_operation_result(request.id, result);
                } else {
                    return;
                }
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("branch migration worker could not be started: {error}"))
}

fn create_worktree(
    connector: &dyn ApiConnector,
    host: &dyn crate::node_access::NodeLink,
    local: bool,
    request: &WorktreeTaskRequest,
) -> Result<WorktreeTaskOutcome, String> {
    create_worktree_observing_purpose(connector, host, local, request, |_, _| {})
}

fn create_worktree_observing_purpose(
    connector: &dyn ApiConnector,
    host: &dyn crate::node_access::NodeLink,
    local: bool,
    request: &WorktreeTaskRequest,
    on_purpose_write: impl FnOnce(&str, &str),
) -> Result<WorktreeTaskOutcome, String> {
    let params = wire::worktree_create_params(
        &request.repository_root,
        &request.branch,
        request.base_branch.as_deref(),
        request.focus,
    )?;
    let result = control_request(connector, "worktree.create", params)
        .map_err(|error| format!("create worktree: {error}"))?;
    let created = wire::created_worktree(result)?;
    // The folder is on the repository's host, which answers for it. A host
    // that cannot answer leaves the worktree as created and says so, rather
    // than rolling back what may be a good worktree (B27).
    let unverified = |error: String| {
        format!(
            "The worktree was created at {} but its host could not confirm it ({error}); it was kept and no agent was started in it. Once its host answers it is listed under its project, where you can start the agent in its pane or delete the worktree; creating it again is refused because its branch is in use",
            created.path
        )
    };
    let created_real = host_directory(host, &created.path).map_err(unverified)?;
    let path_exists = created_real.is_some();
    let listed = control_request(
        connector,
        "worktree.list",
        wire::worktree_list_params(&request.repository_root)?,
    )
    .and_then(|result| wire::listed_worktree_path(result, &request.branch));
    let identity_matches = created.branch.as_deref() == Some(request.branch.as_str())
        && path_exists
        && listed
            .as_ref()
            .ok()
            .and_then(|path| path.as_deref())
            .map(|path| host_directory(host, path).map(|real| (path, real)))
            .transpose()
            .map_err(unverified)?
            .is_some_and(|(_, real)| created_real.is_some() && real == created_real);
    if identity_matches {
        let purpose_failure = request.purpose.as_ref().and_then(|purpose| {
            on_purpose_write(&created.path, purpose);
            let git = NodeGit(host);
            let purpose_request = PurposeTaskRequest {
                id: request.id,
                checkout_id: String::new(),
                repository_root: request.repository_root.clone(),
                // A device's purpose lives in its Herdr metadata only; the
                // branch description mirror is this machine's (B30).
                branch: local.then(|| request.branch.clone()),
                session_workspace_id: Some(created.workspace_id.clone()),
                purpose: purpose.clone(),
            };
            write_created_purpose(connector, &git, &purpose_request)
        });
        // The issue link is a convenience on top of a good worktree: a
        // failure to write it is reported, never a failed creation.
        let issue_error = request.issue.as_ref().and_then(|issue| {
            let issue_request = PurposeTaskRequest {
                id: request.id,
                checkout_id: String::new(),
                repository_root: request.repository_root.clone(),
                branch: Some(request.branch.clone()),
                session_workspace_id: Some(created.workspace_id.clone()),
                purpose: issue.clone(),
            };
            write_issue_metadata(connector, &NodeGit(host), &issue_request, None)
                .err()
                .map(|failure| failure.detail)
        });
        return Ok(WorktreeTaskOutcome {
            path: created.path,
            pane_id: created.pane_id,
            created_tab_id: Some(created.tab_id),
            issue_error,
            purpose_error: purpose_failure
                .as_ref()
                .map(|failure| failure.detail.clone()),
            unconfirmed_purpose_token: purpose_failure
                .and_then(|failure| failure.unconfirmed_token),
            // `worktree.create` takes no environment (a follow-up).
            shell: crate::agent_start::ShellStart::Plain,
        });
    }

    let reason = if created.branch.as_deref() != Some(request.branch.as_str()) {
        format!(
            "created branch mismatch: requested {}, Herdr returned {}",
            request.branch,
            created.branch.as_deref().unwrap_or("detached")
        )
    } else if !path_exists {
        format!("created path is missing on disk: {}", created.path)
    } else {
        let detail = match &listed {
            Ok(Some(path)) => format!("Herdr listed the branch at {path}"),
            Ok(None) => "Herdr did not list the created branch".to_owned(),
            Err(error) => error.clone(),
        };
        format!(
            "created worktree is absent from Herdr's worktree list: {}",
            detail
        )
    };
    let rollback = control_request(
        connector,
        "worktree.remove",
        wire::worktree_remove_params(&created.workspace_id)?,
    );
    if let Err(error) = rollback {
        return Err(format!("{reason}; rollback failed: {error}"));
    }
    let still_listed = control_request(
        connector,
        "worktree.list",
        wire::worktree_list_params(&request.repository_root)?,
    )
    .and_then(|result| wire::listed_worktree_path(result, &request.branch))
    .map(|path| path.is_some())
    .unwrap_or(true);
    let residue = host_directory(host, &created.path);
    if let Err(error) = &residue {
        return Err(format!(
            "{reason}; the rollback could not be confirmed on the host ({error})"
        ));
    }
    if residue.is_ok_and(|real| real.is_some()) || still_listed {
        return Err(format!(
            "{reason}; rollback left residue at {} or in Herdr's worktree list",
            created.path
        ));
    }
    Err(format!("{reason}; the created worktree was rolled back"))
}

trait GitCommands {
    fn run(&self, cwd: &str, command: GitCommand) -> Result<String, String>;
}

/// The git of the node that holds the repository.
struct NodeGit<'a>(&'a dyn crate::node_access::NodeLink);

impl GitCommands for NodeGit<'_> {
    fn run(&self, cwd: &str, command: GitCommand) -> Result<String, String> {
        crate::node_access::call_as(
            self.0,
            Call::Git {
                root: cwd.to_owned(),
                command,
            },
            GIT_TIMEOUT,
        )
        .map_err(|error| error.to_string())
    }
}

/// A device checkout keeps its purpose in Herdr's metadata only; the branch
/// description mirror is this machine's (B30), so a device write never asks
/// for git.
struct HerdrOnly;

impl GitCommands for HerdrOnly {
    fn run(&self, _cwd: &str, _command: GitCommand) -> Result<String, String> {
        Err("A device checkout keeps its purpose in Herdr only".to_owned())
    }
}

fn write_purpose(
    connector: &dyn ApiConnector,
    git: &dyn GitCommands,
    request: &PurposeTaskRequest,
) -> Result<PurposeTaskOutcome, String> {
    write_workspace_metadata(
        connector,
        git,
        request,
        "purpose",
        BranchConfigKey::Description,
    )
}

fn write_workspace_metadata(
    connector: &dyn ApiConnector,
    git: &dyn GitCommands,
    request: &PurposeTaskRequest,
    token: &str,
    config_key: BranchConfigKey,
) -> Result<PurposeTaskOutcome, String> {
    let purpose = request.purpose.trim().to_owned();
    if purpose.chars().count() > 80 {
        return Err("Purpose must be 80 characters or fewer".to_owned());
    }
    if purpose.contains(['\n', '\r']) {
        return Err("Purpose must be one line".to_owned());
    }
    let token_written = request.session_workspace_id.is_some();
    if let Some(workspace_id) = request.session_workspace_id.as_deref() {
        control_request(
            connector,
            "workspace.report_metadata",
            if token == "issue" {
                wire::workspace_issue_params(
                    workspace_id,
                    (!purpose.is_empty()).then_some(purpose.as_str()),
                )?
            } else {
                wire::workspace_purpose_params(
                    workspace_id,
                    (!purpose.is_empty()).then_some(purpose.as_str()),
                )?
            },
        )
        .map_err(|error| format!("workspace purpose: {error}"))?;
    }
    let Some(branch) = request.branch.as_deref() else {
        return Ok(PurposeTaskOutcome::Saved {
            purpose,
            token_written,
        });
    };
    let branch = branch.to_owned();
    let command = if purpose.is_empty() {
        GitCommand::UnsetBranchConfig {
            branch,
            key: config_key,
        }
    } else {
        GitCommand::SetBranchConfig {
            branch,
            key: config_key,
            value: purpose.clone(),
        }
    };
    let git_result = git.run(&request.repository_root, command).map(|_| ());
    match git_result {
        Ok(()) => Ok(PurposeTaskOutcome::Saved {
            purpose,
            token_written,
        }),
        Err(detail) => Ok(PurposeTaskOutcome::GitFailed {
            purpose,
            token_written,
            detail,
        }),
    }
}

/// Creation closes its sheet after the worktree exists, even when purpose
/// persistence fails. Compensate a token-first partial save so the new row
/// shows its fallback instead of claiming a purpose Git did not preserve.
#[derive(Clone, Debug, Eq, PartialEq)]
struct CreatedPurposeFailure {
    detail: String,
    unconfirmed_token: Option<String>,
}

fn write_created_purpose(
    connector: &dyn ApiConnector,
    git: &dyn GitCommands,
    request: &PurposeTaskRequest,
) -> Option<CreatedPurposeFailure> {
    match write_purpose(connector, git, request) {
        Ok(PurposeTaskOutcome::Saved { .. }) => None,
        Ok(PurposeTaskOutcome::GitFailed {
            purpose,
            token_written: true,
            detail,
            ..
        }) => {
            let cleanup = request.session_workspace_id.as_deref().map(|workspace_id| {
                control_request(
                    connector,
                    "workspace.report_metadata",
                    wire::workspace_purpose_params(workspace_id, None)
                        .map_err(|error| error.to_string())?,
                )
                .map(|_| ())
                .map_err(|error| error.to_string())
            });
            match cleanup {
                Some(Ok(())) | None => Some(CreatedPurposeFailure {
                    detail,
                    unconfirmed_token: None,
                }),
                Some(Err(cleanup_error)) => Some(CreatedPurposeFailure {
                    detail: format!("{detail}; Herdr token cleanup failed: {cleanup_error}"),
                    unconfirmed_token: Some(purpose),
                }),
            }
        }
        Ok(PurposeTaskOutcome::GitFailed { detail, .. }) | Err(detail) => {
            Some(CreatedPurposeFailure {
                detail,
                unconfirmed_token: None,
            })
        }
    }
}

fn migrate_branch(
    connector: &dyn ApiConnector,
    host: &dyn crate::node_access::NodeLink,
    git: &dyn GitCommands,
    request: &WorktreeTaskRequest,
) -> Result<WorktreeTaskOutcome, String> {
    let status = git.run(&request.repository_root, GitCommand::Status)?;
    if !status.is_empty() {
        return Err(
            "preflight: commit or discard uncommitted changes before moving the branch".into(),
        );
    }
    let original = git.run(&request.repository_root, GitCommand::CurrentBranch)?;
    let Some(base) = request.base_branch.as_deref() else {
        return Err("preflight: the project base branch is unknown".into());
    };
    if original == base {
        return Err(format!("preflight: the main worktree is already on {base}"));
    }
    git.run(
        &request.repository_root,
        GitCommand::Checkout {
            branch: base.to_owned(),
        },
    )
    .map_err(|error| format!("checkout base branch: {error}"))?;

    let create_request = WorktreeTaskRequest {
        branch: original.clone(),
        focus: false,
        purpose: None,
        issue: None,
        ..request.clone()
    };
    match create_worktree(connector, host, true, &create_request) {
        Ok(outcome) => Ok(outcome),
        Err(create_error) => match git.run(
            &request.repository_root,
            GitCommand::Checkout {
                branch: original.clone(),
            },
        ) {
            Ok(_) => Err(format!(
                "create worktree: {create_error}; restored the main worktree to {original}"
            )),
            Err(restore_error) => {
                let current = git
                    .run(&request.repository_root, GitCommand::CurrentBranch)
                    .unwrap_or_else(|_| "an unknown branch".into());
                Err(format!(
                    "restore original branch: {restore_error}. The main worktree is now on {current}. After resolving the Git error, open the repository at {} and check out {original}.",
                    request.repository_root
                ))
            }
        },
    }
}

fn trace(_path: &str, pane_ids: &[String], stage: &str, error: Option<&str>) {
    eprintln!(
        "{}",
        json!({"event":"worktree.control", "pane_ids":pane_ids, "stage":stage, "failed":error.is_some()})
    );
}

/// Whether closing waits for the processes of the panes it closes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessWait {
    /// Until every shell and foreground process the panes held has ended.
    ForEnd,
    /// Herdr's own confirmation is enough.
    Skip,
}

impl ProcessWait {
    /// A running process holds its working folder only on Windows, and its pid
    /// means something only on the machine that runs it, so only a folder on
    /// this Windows machine is waited for; the cost elsewhere is a deletion
    /// that fails for a process that could not have blocked it.
    pub(crate) fn for_folder_removal(local: bool) -> Self {
        if local && cfg!(windows) {
            Self::ForEnd
        } else {
            Self::Skip
        }
    }
}

/// A process of a pane, identified by its pid and start so a pid the system
/// hands to another process later is not mistaken for it, and named by its
/// program so a failure tells the operator which one to end.
struct PaneProcess {
    pid: u32,
    started: u64,
    name: String,
}

/// What `node`'s kernel says about each of `pids`, in order.
fn process_starts(
    node: &dyn crate::node_access::NodeLink,
    pids: Vec<u32>,
) -> Result<Vec<ProcessStart>, String> {
    let asked = pids.len();
    let starts: Vec<ProcessStart> =
        crate::node_access::call_as(node, Call::ProcessStarts { pids }, PROCESS_READ_TIMEOUT)
            .map_err(|error| format!("the pane processes could not be read: {error}"))?;
    if starts.len() != asked {
        return Err(format!(
            "the pane processes could not be read: {asked} asked, {} answered",
            starts.len()
        ));
    }
    Ok(starts)
}

/// The processes of `pane_id`: the shell and foreground processes Herdr
/// reports, and every process under the shell, all read on `node`. Herdr
/// reports no process between the two, and one can work in the same folder:
/// on Windows a pane's `cmd.exe` runs the agent through `powershell.exe`,
/// which outlived both reported processes in CI and held the worktree folder
/// (issue 707). A process that already ended is not listed: there is nothing
/// to wait for.
fn pane_processes(
    connector: &dyn ApiConnector,
    node: &dyn crate::node_access::NodeLink,
    pane_id: &str,
) -> Result<Vec<PaneProcess>, String> {
    let value = control_request(
        connector,
        "pane.process_info",
        wire::pane_process_info_params(pane_id)?,
    )?;
    let group = wire::pane_process_group(value)?;
    // Pid 0 and 1 are the system's, never a pane's.
    let shell = group.shell_pid.filter(|pid| *pid > 1);
    let under_shell = match shell {
        Some(shell) => crate::node_access::call_as::<Vec<u32>>(
            node,
            Call::ProcessDescendants { pid: shell },
            PROCESS_READ_TIMEOUT,
        )
        .map_err(|error| format!("the processes under shell {shell} could not be read: {error}"))?,
        None => Vec::new(),
    };
    let mut pids: Vec<u32> = shell
        .into_iter()
        .chain(group.foreground_pids)
        .chain(under_shell)
        .filter(|pid| *pid > 1)
        .collect();
    pids.sort_unstable();
    pids.dedup();
    let mut running = Vec::new();
    for (pid, start) in pids.clone().into_iter().zip(process_starts(node, pids)?) {
        match start {
            ProcessStart::Running { started, name } => {
                running.push(PaneProcess { pid, started, name });
            }
            ProcessStart::Gone => {}
            ProcessStart::Unreadable { reason } => return Err(reason),
        }
    }
    Ok(running)
}

/// Waits until none of `held` is running or `deadline` passes, checking once
/// more at the deadline, and names the processes still running when it does.
/// `pause` is how long it lets the processes go on, which a test replaces.
fn wait_for_processes_to_end(
    node: &dyn crate::node_access::NodeLink,
    held: &mut Vec<PaneProcess>,
    deadline: Instant,
    mut pause: impl FnMut(Duration),
) -> Result<(), String> {
    loop {
        if !held.is_empty() {
            let starts = process_starts(node, held.iter().map(|process| process.pid).collect())?;
            let mut running = Vec::new();
            // Only a process that does not exist, or is another one now, has
            // ended; a process that cannot be read is not claimed to be gone.
            for (process, start) in held.drain(..).zip(starts) {
                match start {
                    // The program it runs now: one that started another
                    // since it was first read is named by what holds the
                    // folder.
                    ProcessStart::Running { started, name } if started == process.started => {
                        running.push(PaneProcess { name, ..process });
                    }
                    ProcessStart::Running { .. } | ProcessStart::Gone => {}
                    ProcessStart::Unreadable { reason } => return Err(reason),
                }
            }
            *held = running;
        }
        if held.is_empty() {
            return Ok(());
        }
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return Err(format!(
                "Herdr closed the panes but their processes are still running ({}), so the folder may still be held",
                held.iter()
                    .map(|process| format!("{} pid {}", process.name, process.pid))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        pause(CONFIRM_POLL.min(left));
    }
}

/// Closes `pane_ids` and waits until Herdr's snapshot lists none of them and
/// no pane at any of `paths`, and, when `wait` says so, until the processes the
/// panes held have ended, so the caller's next step cannot run beside a pane
/// that is still there. Herdr confirms a pane closed before its shell or agent
/// has exited, and on Windows a process that is still ending holds its working
/// folder, so the folder cannot be moved or deleted until it is gone. A worktree
/// deletion and a reviewed cleanup share this one pane-closing path.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
pub(crate) fn close_checkout_panes(
    connector: &dyn ApiConnector,
    node: &dyn crate::node_access::NodeLink,
    paths: &[String],
    pane_ids: &[String],
    wait: ProcessWait,
    timeout: Duration,
) -> Result<(), String> {
    close_checkout_panes_checked(connector, node, paths, pane_ids, wait, timeout, &|_| Ok(()))
}

/// Recheck the execution on both sides of the actual close connection.
/// Herdr answers one request per connection; every other close rule stays shared.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
pub(crate) fn close_checkout_panes_checked(
    connector: &dyn ApiConnector,
    node: &dyn crate::node_access::NodeLink,
    paths: &[String],
    pane_ids: &[String],
    wait: ProcessWait,
    timeout: Duration,
    before_close: &dyn Fn(&str) -> Result<(), String>,
) -> Result<(), String> {
    let path = paths.join(",");
    let path = path.as_str();
    let result = (|| {
        // Read every pane's processes before closing any, so a pane that
        // cannot be read closes none.
        let mut held = Vec::new();
        if wait == ProcessWait::ForEnd {
            for pane_id in pane_ids {
                held.extend(pane_processes(connector, node, pane_id)?);
            }
        }
        for pane_id in pane_ids {
            before_close(pane_id)?;
            let mut stream = connector.connect().map_err(|error| error.to_string())?;
            before_close(pane_id)?;
            trace(path, std::slice::from_ref(pane_id), "close_requested", None);
            hide_herdr_client::request_on_stream(
                stream.as_mut(),
                "herdr-core:pane.close",
                "pane.close",
                json!({"pane_id":pane_id}),
                timeout,
            )
            .map_err(|error| error.to_string())?;
        }
        if pane_ids.is_empty() {
            return Ok(());
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| {
                    format!(
                        "Timed out waiting for Herdr to confirm closed panes: {}",
                        pane_ids.join(", ")
                    )
                })?;
            let result =
                request_with_connector(connector, "session.snapshot", json!({}), remaining)
                    .map_err(|error| {
                        format!("session.snapshot close confirmation failed: {error}")
                    })?;
            // No serde defaults here: missing topology is never proof of absence.
            if result["type"] != "session_snapshot" {
                return Err("Herdr close confirmation is not a session_snapshot".into());
            }
            let panes = result
                .pointer("/snapshot/panes")
                .and_then(Value::as_array)
                .ok_or("Herdr close confirmation is missing snapshot.panes")?;
            let (present, pane_at_checkout) = confirmation_state(panes, paths)?;
            if pane_ids.iter().all(|id| !present.contains(id)) && !pane_at_checkout {
                // Herdr has nothing more to say; only the processes remain.
                return wait_for_processes_to_end(node, &mut held, deadline, thread::sleep);
            }
            thread::sleep(CONFIRM_POLL.min(deadline.saturating_duration_since(Instant::now())));
        }
    })();
    trace(
        path,
        pane_ids,
        if result.is_ok() {
            "close_confirmed"
        } else {
            "close_failed"
        },
        result.as_ref().err().map(String::as_str),
    );
    result
}

fn confirmation_state(panes: &[Value], paths: &[String]) -> Result<(Vec<String>, bool), String> {
    let present = panes
        .iter()
        .map(|pane| {
            pane.get("pane_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| "Herdr close confirmation has an invalid pane_id".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let pane_at_checkout = panes.iter().any(|pane| {
        ["cwd", "foreground_cwd"].into_iter().any(|key| {
            pane.get(key)
                .and_then(Value::as_str)
                .is_some_and(|cwd| paths.iter().any(|path| path == cwd))
        })
    });
    Ok((present, pane_at_checkout))
}

fn open_worktree(
    connector: &dyn ApiConnector,
    path: &str,
    repository_root: &str,
    pane_id: Option<&str>,
) -> Result<(), String> {
    let panes = pane_id.into_iter().map(str::to_owned).collect::<Vec<_>>();
    trace(path, &panes, "open_requested", None);
    let result = match pane_id {
        Some(id) => control_request(connector, "pane.focus", json!({"pane_id":id})),
        None => control_request(
            connector,
            "worktree.open",
            json!({"cwd":repository_root,"path":path,"focus":true}),
        ),
    }
    .map(|_| ());
    trace(
        path,
        &panes,
        if result.is_ok() {
            "open_completed"
        } else {
            "open_failed"
        },
        result.as_ref().err().map(String::as_str),
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_herdr_client::{ApiError, ApiStream};
    use hide_platform::ipc::LocalStream;
    use std::collections::VecDeque;
    use std::io::{BufRead, BufReader, Write};
    use std::process::Command;

    #[test]
    fn a_tab_whose_saved_folder_is_gone_is_refused_with_that_reason() {
        let checkout = tempfile::tempdir().unwrap();
        let inside = checkout.path().join("sub");
        std::fs::create_dir(&inside).unwrap();
        let request = |cwd: &std::path::Path| CheckoutTabRequest {
            id: 1,
            agent_kind: Some("claude".into()),
            checkout_path: checkout.path().display().to_string(),
            label: "resume".into(),
            host: crate::checkout_owner::TabHost::Workspace("w1".into()),
            resume_scope: None,
            resume_reference: None,
            focus: false,
            cwd: cwd.display().to_string(),
        };
        let node = hide_node::Local::of_process();
        assert_eq!(folder_present(&node, &request(&inside)), Ok(()));
        assert_eq!(folder_present(&node, &request(checkout.path())), Ok(()));
        let gone = checkout.path().join("removed");
        assert_eq!(
            folder_present(&node, &request(&gone)),
            Err(format!(
                "The working folder {} no longer exists",
                gone.display()
            ))
        );
    }

    // The only fake is the external server's newline-delimited protocol.
    struct Server {
        replies: Mutex<VecDeque<Value>>,
        requests: Arc<Mutex<Vec<Value>>>,
    }
    impl ApiConnector for Server {
        fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError> {
            let reply = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected request");
            let requests = self.requests.clone();
            let (client, mut server) = LocalStream::pair().unwrap();
            thread::spawn(move || {
                let mut line = String::new();
                BufReader::new(&mut server).read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                let mut response = reply;
                response["id"] = request["id"].clone();
                requests.lock().unwrap().push(request);
                writeln!(server, "{response}").unwrap();
            });
            Ok(Box::new(client))
        }
    }
    fn server(replies: Vec<Value>) -> Server {
        Server {
            replies: Mutex::new(replies.into()),
            requests: Arc::new(Mutex::new(vec![])),
        }
    }
    fn snapshot(ids: &[&str]) -> Value {
        json!({"result":{"type":"session_snapshot","snapshot":{"panes":ids.iter().map(|id|json!({"pane_id":id})).collect::<Vec<_>>()}}})
    }
    fn workspace_created(path: &str, workspace_id: &str, pane_id: &str) -> Value {
        json!({"result":{
            "type":"workspace_created",
            "workspace":{"workspace_id":workspace_id,"label":"hide","number":1,"focused":true,"pane_count":1,"tab_count":1,"active_tab_id":format!("{workspace_id}:t1"),"agent_status":"unknown"},
            "tab":{"workspace_id":workspace_id,"tab_id":format!("{workspace_id}:t1"),"label":"hide codex","number":1,"focused":true,"pane_count":1,"agent_status":"unknown"},
            "root_pane":{"workspace_id":workspace_id,"tab_id":format!("{workspace_id}:t1"),"pane_id":pane_id, "terminal_id": "fixture-terminal","cwd":path,"foreground_cwd":path,"focused":true,"agent_status":"unknown","revision":0,"scroll":{"max_offset_from_bottom":0,"offset_from_bottom":0,"viewport_rows":40}}
        }})
    }
    fn worktree_created(path: &str, branch: &str) -> Value {
        let mut value = workspace_created(path, "w2", "w2:p1");
        let result = value["result"].as_object_mut().unwrap();
        result.insert("type".into(), json!("worktree_created"));
        result.insert(
            "worktree".into(),
            json!({
                "branch":branch,"is_bare":false,"is_detached":false,
                "is_linked_worktree":true,"is_prunable":false,"label":"repo",
                "open_workspace_id":"w2","path":path
            }),
        );
        value
    }
    fn worktree_list(path: &str, branch: &str) -> Value {
        json!({"result":{"type":"worktree_list","source":{
            "repo_key":"/fixture/repo/.git","repo_name":"repo","repo_root":"/fixture/repo",
            "source_checkout_path":"/fixture/repo","source_workspace_id":"w1"
        },"worktrees":[{
            "branch":branch,"is_bare":false,"is_detached":false,
            "is_linked_worktree":true,"is_prunable":false,"label":"repo",
            "open_workspace_id":"w2","path":path
        }]}})
    }
    struct ScriptedGit {
        replies: Mutex<VecDeque<Result<String, String>>>,
        calls: Mutex<Vec<Vec<String>>>,
    }
    impl ScriptedGit {
        fn new(replies: Vec<Result<&str, &str>>) -> Self {
            Self {
                replies: Mutex::new(
                    replies
                        .into_iter()
                        .map(|result| result.map(str::to_owned).map_err(str::to_owned))
                        .collect(),
                ),
                calls: Mutex::new(Vec::new()),
            }
        }
    }
    impl GitCommands for ScriptedGit {
        fn run(&self, _cwd: &str, command: GitCommand) -> Result<String, String> {
            self.calls.lock().unwrap().push(command.args().unwrap());
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected git call")
        }
    }
    fn task(branch: &str) -> WorktreeTaskRequest {
        WorktreeTaskRequest {
            id: 1,
            repository_root: "/fixture/repo".into(),
            branch: branch.into(),
            base_branch: Some("main".into()),
            agent_kind: None,
            focus: true,
            purpose: None,
            issue: None,
        }
    }

    fn purpose_request(purpose: &str) -> PurposeTaskRequest {
        PurposeTaskRequest {
            id: 7,
            checkout_id: "checkout:feature".into(),
            repository_root: "/fixture/repo".into(),
            branch: Some("feature".into()),
            session_workspace_id: Some("w7".into()),
            purpose: purpose.into(),
        }
    }

    #[test]
    fn issue_writer_uses_the_same_token_first_boundary_and_clears_both_values() {
        let server = server(vec![
            json!({"result":{"type":"ok"}}),
            json!({"result":{"type":"ok"}}),
        ]);
        let git = ScriptedGit::new(vec![Ok(""), Ok("")]);
        write_workspace_metadata(
            &server,
            &git,
            &purpose_request("acme/project#42"),
            "issue",
            BranchConfigKey::Issue,
        )
        .unwrap();
        write_workspace_metadata(
            &server,
            &git,
            &purpose_request(""),
            "issue",
            BranchConfigKey::Issue,
        )
        .unwrap();
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests[0]["params"]["tokens"]["issue"], "acme/project#42");
        assert!(requests[1]["params"]["tokens"]["issue"].is_null());
        let calls = git.calls.lock().unwrap();
        assert_eq!(
            calls[0],
            ["config", "--", "branch.feature.issue", "acme/project#42"]
        );
        assert_eq!(
            calls[1],
            ["config", "--unset-all", "--", "branch.feature.issue"]
        );
    }

    #[test]
    fn purpose_writes_the_workspace_token_before_the_branch_description() {
        let server = server(vec![json!({"result":{"type":"ok"}})]);
        let git = ScriptedGit::new(vec![Ok("")]);

        assert_eq!(
            write_purpose(&server, &git, &purpose_request("Ship checkout row D")).unwrap(),
            PurposeTaskOutcome::Saved {
                purpose: "Ship checkout row D".into(),
                token_written: true,
            }
        );

        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["method"], "workspace.report_metadata");
        assert_eq!(requests[0]["params"]["workspace_id"], "w7");
        assert_eq!(requests[0]["params"]["source"], "hide");
        assert_eq!(
            requests[0]["params"]["tokens"]["purpose"],
            "Ship checkout row D"
        );
        assert_eq!(
            git.calls.lock().unwrap().as_slice(),
            [vec![
                "config".to_owned(),
                "--".to_owned(),
                "branch.feature.description".to_owned(),
                "Ship checkout row D".to_owned(),
            ]]
        );
    }

    #[test]
    fn issue_cleanup_distinguishes_branch_switch_detach_removal_and_unregister() {
        let (mut mirror, receiver) = PurposeMirror::recording();
        let mut project = WorkspaceSnapshot {
            agent_presence: Default::default(),
            agent_scope: Default::default(),
            home_issues: Default::default(),
            pull_requests: Vec::new(),
            tasks: Default::default(),
            id: "project".to_owned(),
            label: "Fixture".to_owned(),
            path: "/fixture/repo".to_owned(),
            remote_target_id: None,
            expanded: true,
            device_id: "local".to_owned(),
            repo_name: "repo".to_owned(),
            is_git: true,
            default_branch: Some("main".to_owned()),
            branches: vec!["main".to_owned(), "topic/quoted".to_owned()],
            registered: true,
            temporary: false,
            session_workspace_ids: vec!["w-purpose".to_owned()],
            last_activity_unix_ms: None,
            pinned: false,
            is_home: false,
            checkouts: vec![crate::model::CheckoutSnapshot {
                id: "checkout".to_owned(),
                workspace_id: "project".to_owned(),
                label: "topic/quoted".to_owned(),
                path: "/fixture/repo/worktrees/topic".to_owned(),
                branch: Some("topic/quoted".to_owned()),
                exists: true,
                is_worktree: true,
                ..Default::default()
            }],
            inactive_checkouts: Default::default(),
            session_folds: Default::default(),
            removal: Default::default(),
            disk: Default::default(),
            cleanup: None,
        };
        mirror.sync(&[], std::slice::from_ref(&project), &HashMap::new());
        project.checkouts[0].branch = Some("other".into());
        mirror.sync(&[], std::slice::from_ref(&project), &HashMap::new());
        project.checkouts[0].branch = None;
        mirror.sync(&[], std::slice::from_ref(&project), &HashMap::new());
        assert!(receiver.try_recv().is_err());
        project.checkouts[0].branch = Some("other".into());
        mirror.sync(&[], std::slice::from_ref(&project), &HashMap::new());
        mirror.sync(&[], &[], &HashMap::new());
        assert!(receiver.try_recv().is_err(), "unregister is not deletion");
        mirror.sync(&[], std::slice::from_ref(&project), &HashMap::new());
        project.checkouts[0].branch = None;
        mirror.sync(&[], std::slice::from_ref(&project), &HashMap::new());
        mirror.sync(&[], std::slice::from_ref(&project), &HashMap::new());
        assert!(receiver.try_recv().is_err(), "detaching is not deletion");
        project.checkouts.clear();
        let space = workspace::SessionSpace {
            id: "w-purpose".into(),
            label: "Fixture".into(),
            cwds: vec!["/fixture/repo/worktrees/topic".into()],
            purpose: None,
        };
        mirror.sync(&[space], &[project], &HashMap::new());
        let PurposeMirrorMessage::RemoveIssue {
            branch,
            workspace_ids,
            ..
        } = receiver
            .try_recv()
            .expect("detached removal still owns cleanup")
        else {
            panic!("expected cleanup")
        };
        assert_eq!(branch, "other");
        assert_eq!(workspace_ids, vec!["w-purpose"]);
        assert!(receiver.try_recv().is_err());
    }
    #[test]
    fn issue_rollback_reports_only_an_uncertain_token_as_unconfirmed() {
        for failed_rollback in [false, true] {
            let rollback = if failed_rollback {
                json!({"error":{"code":"unavailable","message":"rollback refused"}})
            } else {
                json!({"result":{"type":"ok"}})
            };
            let server = server(vec![json!({"result":{"type":"ok"}}), rollback]);
            let git = ScriptedGit::new(vec![Err("git locked")]);
            let error = write_issue_metadata(
                &server,
                &git,
                &purpose_request("acme/project#3"),
                Some("acme/project#1"),
            )
            .unwrap_err();
            assert_eq!(error.unconfirmed_token, failed_rollback);
            assert_eq!(
                server.requests.lock().unwrap()[1]["params"]["tokens"]["issue"],
                "acme/project#1"
            );
        }
    }

    #[test]
    fn rejected_workspace_token_never_writes_git() {
        let server = server(vec![json!({
            "error":{"code":"unavailable","message":"injected token refusal"}
        })]);
        let git = ScriptedGit::new(vec![]);

        let error = write_purpose(&server, &git, &purpose_request("Keep input")).unwrap_err();

        assert!(error.contains("injected token refusal"));
        assert!(git.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn git_failure_after_a_workspace_token_is_a_partial_save() {
        let server = server(vec![json!({"result":{"type":"ok"}})]);
        let git = ScriptedGit::new(vec![Err("injected git lock")]);

        assert_eq!(
            write_purpose(&server, &git, &purpose_request("Visible live")).unwrap(),
            PurposeTaskOutcome::GitFailed {
                purpose: "Visible live".into(),
                token_written: true,
                detail: "injected git lock".into(),
            }
        );
    }

    #[test]
    fn git_failure_without_a_live_workspace_reports_no_partial_save() {
        let server = server(vec![]);
        let git = ScriptedGit::new(vec![Err("injected git lock")]);
        let mut request = purpose_request("Keep input");
        request.session_workspace_id = None;

        assert_eq!(
            write_purpose(&server, &git, &request).unwrap(),
            PurposeTaskOutcome::GitFailed {
                purpose: "Keep input".into(),
                token_written: false,
                detail: "injected git lock".into(),
            }
        );
        assert!(server.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn clearing_a_purpose_clears_the_token_and_branch_description() {
        let server = server(vec![json!({"result":{"type":"ok"}})]);
        let git = ScriptedGit::new(vec![Ok("")]);

        assert!(matches!(
            write_purpose(&server, &git, &purpose_request("")),
            Ok(PurposeTaskOutcome::Saved { purpose, .. }) if purpose.is_empty()
        ));

        let requests = server.requests.lock().unwrap();
        assert!(requests[0]["params"]["tokens"]["purpose"].is_null());
        assert_eq!(
            git.calls.lock().unwrap().as_slice(),
            [vec![
                "config".to_owned(),
                "--unset-all".to_owned(),
                "--".to_owned(),
                "branch.feature.description".to_owned(),
            ]]
        );
    }

    #[test]
    fn creation_compensates_a_token_when_the_git_mirror_fails() {
        let server = server(vec![
            json!({"result":{"type":"ok"}}),
            json!({"result":{"type":"ok"}}),
        ]);
        let git = ScriptedGit::new(vec![Err("injected git lock")]);

        assert_eq!(
            write_created_purpose(&server, &git, &purpose_request("Use the fallback")),
            Some(CreatedPurposeFailure {
                detail: "injected git lock".to_owned(),
                unconfirmed_token: None,
            })
        );

        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0]["params"]["tokens"]["purpose"],
            "Use the fallback"
        );
        assert!(requests[1]["params"]["tokens"]["purpose"].is_null());
    }

    #[test]
    fn creation_marks_a_token_unconfirmed_when_git_and_cleanup_both_fail() {
        let server = server(vec![
            json!({"result":{"type":"ok"}}),
            json!({"error":{"code":"unavailable","message":"injected cleanup refusal"}}),
        ]);
        let git = ScriptedGit::new(vec![Err("injected git lock")]);

        assert_eq!(
            write_created_purpose(&server, &git, &purpose_request("Use the fallback")),
            Some(CreatedPurposeFailure {
                detail: "injected git lock; Herdr token cleanup failed: workspace.report_metadata failed: unavailable: injected cleanup refusal".to_owned(),
                unconfirmed_token: Some("Use the fallback".to_owned()),
            })
        );
        assert_eq!(server.requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn purpose_mirror_emits_initial_changes_and_clear_for_the_exact_branch() {
        let (mut mirror, receiver) = PurposeMirror::recording();
        let mut space = workspace::SessionSpace {
            id: "w-purpose".to_owned(),
            label: "Fixture".to_owned(),
            cwds: vec!["/fixture/repo/worktrees/topic".to_owned()],
            purpose: Some("Initial purpose".to_owned()),
        };
        let checkout = WorkspaceSnapshot {
            agent_presence: Default::default(),
            agent_scope: Default::default(),
            home_issues: Default::default(),
            pull_requests: Vec::new(),
            tasks: Default::default(),
            id: "project".to_owned(),
            label: "Fixture".to_owned(),
            path: "/fixture/repo".to_owned(),
            remote_target_id: None,
            expanded: true,
            device_id: "local".to_owned(),
            repo_name: "repo".to_owned(),
            is_git: true,
            default_branch: Some("main".to_owned()),
            branches: vec!["main".to_owned(), "topic/quoted".to_owned()],
            registered: true,
            temporary: false,
            session_workspace_ids: vec!["w-purpose".to_owned()],
            last_activity_unix_ms: None,
            pinned: false,
            is_home: false,
            checkouts: vec![crate::model::CheckoutSnapshot {
                id: "checkout".to_owned(),
                workspace_id: "project".to_owned(),
                label: "topic/quoted".to_owned(),
                path: "/fixture/repo/worktrees/topic".to_owned(),
                branch: Some("topic/quoted".to_owned()),
                exists: true,
                is_worktree: true,
                ..Default::default()
            }],
            inactive_checkouts: Default::default(),
            session_folds: Default::default(),
            removal: Default::default(),
            disk: Default::default(),
            cleanup: None,
        };

        let suppressed = HashMap::from([(
            workspace::comparison_by_names(Path::new("/fixture/repo/worktrees/topic")),
            "Initial purpose".to_owned(),
        )]);
        mirror.sync(
            std::slice::from_ref(&space),
            std::slice::from_ref(&checkout),
            &suppressed,
        );
        assert!(
            receiver.try_recv().is_err(),
            "an unconfirmed creation token is never mirrored back into Git"
        );

        mirror.sync(
            std::slice::from_ref(&space),
            std::slice::from_ref(&checkout),
            &HashMap::new(),
        );
        let PurposeMirrorMessage::Write(initial) = receiver.recv().unwrap() else {
            panic!("initial purpose is a write")
        };
        assert_eq!(initial.workspace_id, "w-purpose");
        assert_eq!(initial.repository_root, "/fixture/repo");
        assert_eq!(initial.branch, "topic/quoted");
        assert_eq!(initial.purpose.as_deref(), Some("Initial purpose"));

        mirror.sync(
            std::slice::from_ref(&space),
            std::slice::from_ref(&checkout),
            &HashMap::new(),
        );
        assert!(
            receiver.try_recv().is_err(),
            "an unchanged token is not rewritten"
        );

        space.purpose = Some("Changed purpose".to_owned());
        mirror.sync(
            std::slice::from_ref(&space),
            std::slice::from_ref(&checkout),
            &HashMap::new(),
        );
        let PurposeMirrorMessage::Write(changed) = receiver.recv().unwrap() else {
            panic!("changed purpose is a write")
        };
        assert_eq!(changed.purpose.as_deref(), Some("Changed purpose"));

        space.purpose = None;
        mirror.sync(
            std::slice::from_ref(&space),
            std::slice::from_ref(&checkout),
            &HashMap::new(),
        );
        let PurposeMirrorMessage::Write(cleared) = receiver.recv().unwrap() else {
            panic!("cleared purpose is a write")
        };
        assert_eq!(cleared.branch, "topic/quoted");
        assert!(cleared.purpose.is_none());
    }

    #[test]
    fn purpose_mirror_uses_the_projects_authoritative_workspace_only() {
        let (mut mirror, receiver) = PurposeMirror::recording();
        let spaces = vec![
            workspace::SessionSpace {
                id: "w-outer".to_owned(),
                label: "Outer".to_owned(),
                cwds: vec!["/fixture/repo/worktrees/topic".to_owned()],
                purpose: Some("Outer purpose".to_owned()),
            },
            workspace::SessionSpace {
                id: "w-nested".to_owned(),
                label: "Nested".to_owned(),
                cwds: vec!["/fixture/repo/worktrees/topic/nested".to_owned()],
                purpose: Some("Nested purpose".to_owned()),
            },
            workspace::SessionSpace {
                id: "w-authority".to_owned(),
                label: "Outer second".to_owned(),
                cwds: vec!["/fixture/repo/worktrees/topic".to_owned()],
                purpose: Some("Authoritative purpose".to_owned()),
            },
        ];
        let project = WorkspaceSnapshot {
            agent_presence: Default::default(),
            agent_scope: Default::default(),
            home_issues: Default::default(),
            pull_requests: Vec::new(),
            tasks: Default::default(),
            id: "outer".to_owned(),
            label: "Outer".to_owned(),
            path: "/fixture/repo".to_owned(),
            remote_target_id: None,
            expanded: true,
            device_id: "local".to_owned(),
            repo_name: "repo".to_owned(),
            is_git: true,
            default_branch: Some("main".to_owned()),
            branches: vec!["topic".to_owned()],
            registered: true,
            temporary: false,
            session_workspace_ids: vec!["w-outer".to_owned(), "w-authority".to_owned()],
            last_activity_unix_ms: None,
            pinned: false,
            is_home: false,
            checkouts: vec![crate::model::CheckoutSnapshot {
                id: "checkout".to_owned(),
                workspace_id: "outer".to_owned(),
                label: "topic".to_owned(),
                path: "/fixture/repo/worktrees/topic".to_owned(),
                branch: Some("topic".to_owned()),
                exists: true,
                is_worktree: true,
                ..Default::default()
            }],
            inactive_checkouts: Default::default(),
            session_folds: Default::default(),
            removal: Default::default(),
            disk: Default::default(),
            cleanup: None,
        };

        mirror.sync(&spaces, std::slice::from_ref(&project), &HashMap::new());

        let PurposeMirrorMessage::Write(write) = receiver.recv().unwrap() else {
            panic!("the authoritative purpose is a write")
        };
        assert_eq!(write.workspace_id, "w-authority");
        assert_eq!(write.purpose.as_deref(), Some("Authoritative purpose"));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn purpose_mirror_tracks_source_switches_by_checkout_and_restores_the_last_value() {
        let (mut mirror, receiver) = PurposeMirror::recording();
        let first = workspace::SessionSpace {
            id: "w-first".to_owned(),
            label: "First".to_owned(),
            cwds: vec!["/fixture/repo/worktrees/topic".to_owned()],
            purpose: Some("First purpose".to_owned()),
        };
        let later_without_token = workspace::SessionSpace {
            id: "w-later".to_owned(),
            label: "Later".to_owned(),
            cwds: vec!["/fixture/repo/worktrees/topic".to_owned()],
            purpose: None,
        };
        let mut project = WorkspaceSnapshot {
            agent_presence: Default::default(),
            agent_scope: Default::default(),
            home_issues: Default::default(),
            pull_requests: Vec::new(),
            tasks: Default::default(),
            id: "project".to_owned(),
            label: "Fixture".to_owned(),
            path: "/fixture/repo".to_owned(),
            remote_target_id: None,
            expanded: true,
            device_id: "local".to_owned(),
            repo_name: "repo".to_owned(),
            is_git: true,
            default_branch: Some("main".to_owned()),
            branches: vec!["topic".to_owned()],
            registered: true,
            temporary: false,
            session_workspace_ids: vec!["w-first".to_owned()],
            last_activity_unix_ms: None,
            pinned: false,
            is_home: false,
            checkouts: vec![crate::model::CheckoutSnapshot {
                id: "checkout".to_owned(),
                workspace_id: "project".to_owned(),
                label: "topic".to_owned(),
                path: "/fixture/repo/worktrees/topic".to_owned(),
                branch: Some("topic".to_owned()),
                exists: true,
                is_worktree: true,
                ..Default::default()
            }],
            inactive_checkouts: Default::default(),
            session_folds: Default::default(),
            removal: Default::default(),
            disk: Default::default(),
            cleanup: None,
        };

        mirror.sync(
            std::slice::from_ref(&first),
            std::slice::from_ref(&project),
            &HashMap::new(),
        );
        let PurposeMirrorMessage::Write(initial) = receiver.recv().unwrap() else {
            panic!("initial purpose is written")
        };
        assert_eq!(initial.workspace_id, "w-first");
        assert_eq!(initial.purpose.as_deref(), Some("First purpose"));

        project.session_workspace_ids.push("w-later".to_owned());
        mirror.sync(
            &[first.clone(), later_without_token],
            std::slice::from_ref(&project),
            &HashMap::new(),
        );
        let PurposeMirrorMessage::Write(cleared) = receiver.recv().unwrap() else {
            panic!("later explicit absence clears the mirror")
        };
        assert_eq!(cleared.workspace_id, "w-later");
        assert!(cleared.purpose.is_none());

        project.session_workspace_ids.pop();
        mirror.sync(
            std::slice::from_ref(&first),
            std::slice::from_ref(&project),
            &HashMap::new(),
        );
        let PurposeMirrorMessage::Write(restored) = receiver.recv().unwrap() else {
            panic!("removing the later workspace restores the earlier value")
        };
        assert_eq!(restored.workspace_id, "w-first");
        assert_eq!(restored.purpose.as_deref(), Some("First purpose"));
    }

    #[test]
    fn refusal_never_authorizes_removal_or_closes_the_next_pane() {
        let server = server(vec![
            json!({"error":{"code":"confirmation_required","message":"close refused"}}),
        ]);
        let result = close_checkout_panes(
            &server,
            &hide_node::Local::of_process(),
            &["/fixture/topic".into()],
            &["w1:p1".into(), "w1:p2".into()],
            ProcessWait::Skip,
            CONFIRM_TIMEOUT,
        );
        assert!(result.unwrap_err().contains("close refused"));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
    #[test]
    fn every_requested_pane_must_disappear_before_removal_is_ready() {
        let server = server(vec![
            json!({"result":{"type":"ok"}}),
            json!({"result":{"type":"ok"}}),
            snapshot(&["w1:p2", "w2:p1"]),
            snapshot(&["w2:p1"]),
        ]);
        close_checkout_panes(
            &server,
            &hide_node::Local::of_process(),
            &["/fixture/topic".into()],
            &["w1:p1".into(), "w1:p2".into()],
            ProcessWait::Skip,
            CONFIRM_TIMEOUT,
        )
        .unwrap();
        assert!(server.replies.lock().unwrap().is_empty());
        assert_eq!(
            server.requests.lock().unwrap()[1]["params"],
            json!({"pane_id":"w1:p2"})
        );
    }
    #[test]
    fn a_new_pane_at_the_checkout_blocks_removal_authorization() {
        let panes = vec![json!({
            "pane_id":"w2:p1",
            "cwd":"/fixture/topic",
            "foreground_cwd":"/fixture/topic"
        })];
        let (ids, at_checkout) = confirmation_state(&panes, &["/fixture/topic".into()]).unwrap();
        assert_eq!(ids, vec!["w2:p1"]);
        assert!(at_checkout);
    }
    #[test]
    fn malformed_confirmation_cannot_authorize_removal() {
        for invalid in [
            json!({}),
            json!({"type":"session_snapshot","snapshot":{}}),
            json!({"type":"session_snapshot","snapshot":{"panes":[{}]}}),
        ] {
            let server = server(vec![
                json!({"result":{"type":"ok"}}),
                json!({"result":invalid}),
            ]);
            assert!(
                close_checkout_panes(
                    &server,
                    &hide_node::Local::of_process(),
                    &["/fixture/topic".into()],
                    &["w1:p1".into()],
                    ProcessWait::Skip,
                    CONFIRM_TIMEOUT
                )
                .is_err()
            );
        }
    }
    fn process_info(pane: &str, shell: u32) -> Value {
        json!({"result":{"type":"pane_process_info","process_info":{
            "pane_id":pane,"shell_pid":shell,"foreground_process_group_id":shell,
            "foreground_processes":[{"pid":shell,"name":"sleep"}]}}})
    }

    /// A child that is ended however the test leaves.
    #[cfg(unix)]
    struct LongLived(std::process::Child);
    #[cfg(unix)]
    impl LongLived {
        fn start() -> Self {
            Self(Command::new("sleep").arg("30").spawn().unwrap())
        }
        fn end(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[cfg(unix)]
    impl Drop for LongLived {
        fn drop(&mut self) {
            self.end();
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_wait_goes_on_while_a_process_runs_and_ends_when_it_has() {
        let mut child = LongLived::start();
        let pid = child.0.id();
        let server = server(vec![process_info("w1:p1", pid)]);
        let mut held = pane_processes(&server, &hide_node::Local::of_process(), "w1:p1").unwrap();
        let mut pauses = 0;
        wait_for_processes_to_end(
            &hide_node::Local::of_process(),
            &mut held,
            Instant::now() + CONFIRM_TIMEOUT,
            |_| {
                pauses += 1;
                child.end();
            },
        )
        .unwrap();
        assert_eq!(
            pauses, 1,
            "it paused once while the child ran and not again"
        );
        assert!(held.is_empty());
    }

    /// A shell with a child of its own that Herdr does not report, and that
    /// child's pid. On Windows a pane's `cmd.exe` runs the agent through
    /// `powershell.exe`, and Herdr reports only the shell and the agent.
    #[cfg(unix)]
    struct ShellWithChild {
        shell: std::process::Child,
        child: u32,
    }
    #[cfg(unix)]
    impl ShellWithChild {
        fn start() -> Self {
            let mut shell = Command::new("sh")
                .args(["-c", "sleep 30 & echo $!; wait"])
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let mut line = String::new();
            BufReader::new(shell.stdout.take().unwrap())
                .read_line(&mut line)
                .unwrap();
            let child = line.trim().parse().unwrap();
            Self { shell, child }
        }
        fn end_shell(&mut self) {
            let _ = self.shell.kill();
            let _ = self.shell.wait();
        }
    }
    #[cfg(unix)]
    impl Drop for ShellWithChild {
        fn drop(&mut self) {
            self.end_shell();
            let _ = hide_platform::process::kill_tree(self.child);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_process_the_shell_started_holds_the_wait_after_the_shell_has_ended() {
        let mut tree = ShellWithChild::start();
        // Herdr names the shell alone, as its foreground process too.
        let server = server(vec![process_info("w1:p1", tree.shell.id())]);
        let node = hide_node::Local::of_process();
        let mut held = pane_processes(&server, &node, "w1:p1").unwrap();
        tree.end_shell();
        let error = wait_for_processes_to_end(&node, &mut held, Instant::now(), |_| {
            panic!("no time is left to wait")
        })
        .unwrap_err();
        // The child is `sh` until it has become `sleep`, which may be after
        // the shell printed its pid; the name is the other test's subject.
        assert!(error.contains(&format!("pid {})", tree.child)), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_process_still_running_at_the_deadline_fails_the_wait_and_names_it() {
        let child = LongLived::start();
        let pid = child.0.id();
        let server = server(vec![process_info("w1:p1", pid)]);
        let mut held = pane_processes(&server, &hide_node::Local::of_process(), "w1:p1").unwrap();
        assert_eq!(held.len(), 1);
        // A deadline that has passed still checks once before it fails.
        let error = wait_for_processes_to_end(
            &hide_node::Local::of_process(),
            &mut held,
            Instant::now(),
            |_| panic!("no time is left to wait"),
        )
        .unwrap_err();
        assert!(
            error.contains(&format!("still running (sleep pid {pid})")),
            "{error}"
        );
    }

    #[test]
    fn a_process_that_has_ended_is_not_waited_for() {
        let mut held = vec![PaneProcess {
            pid: u32::MAX - 1,
            started: 1,
            name: "gone".to_owned(),
        }];
        wait_for_processes_to_end(
            &hide_node::Local::of_process(),
            &mut held,
            Instant::now(),
            |_| panic!("nothing to wait for"),
        )
        .unwrap();
        assert!(held.is_empty());
    }

    #[test]
    fn a_pane_reports_each_running_process_once_and_never_the_systems_pids() {
        let me = std::process::id();
        for shell in [0, 1, u32::MAX - 1] {
            let server = server(vec![process_info("w1:p1", shell)]);
            assert!(
                pane_processes(&server, &hide_node::Local::of_process(), "w1:p1")
                    .unwrap()
                    .is_empty(),
                "pid {shell}"
            );
        }
        // The shell is also the foreground process, and is listed once. It
        // is a child of its own: the test process's pid would also have
        // every other running test's children under it (issue 746).
        let mut command = if cfg!(windows) {
            let mut command = Command::new("ping");
            command.args(["-n", "30", "127.0.0.1"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("30");
            command
        };
        let mut shell = command.stdout(std::process::Stdio::null()).spawn().unwrap();
        let pid = shell.id();
        let server = server(vec![process_info("w1:p1", pid)]);
        let held = pane_processes(&server, &hide_node::Local::of_process(), "w1:p1");
        let tree = hide_platform::process::descendants(pid);
        let _ = shell.kill();
        let _ = shell.wait();
        let held = held
            .unwrap()
            .iter()
            .map(|process| process.pid)
            .collect::<Vec<_>>();
        let tree = tree.unwrap();
        assert_eq!(held.iter().filter(|held| **held == pid).count(), 1);
        assert!(
            held.iter().all(|held| *held == pid || tree.contains(held)),
            "only the shell and what runs under it: {held:?}"
        );
        assert!(!held.contains(&me));
    }

    #[test]
    fn a_pane_whose_processes_cannot_be_read_closes_no_pane() {
        let server = server(vec![
            json!({"error":{"code":"pane_not_found","message":"no such pane"}}),
        ]);
        let error = close_checkout_panes(
            &server,
            &hide_node::Local::of_process(),
            &["/fixture/topic".into()],
            &["w1:p1".into(), "w1:p2".into()],
            ProcessWait::ForEnd,
            CONFIRM_TIMEOUT,
        )
        .unwrap_err();
        assert!(error.contains("pane.process_info failed"), "{error}");
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_wait_that_is_skipped_never_asks_for_processes() {
        let server = server(vec![json!({"result":{"type":"ok"}}), snapshot(&[])]);
        close_checkout_panes(
            &server,
            &hide_node::Local::of_process(),
            &["/fixture/topic".into()],
            &["w1:p1".into()],
            ProcessWait::Skip,
            CONFIRM_TIMEOUT,
        )
        .unwrap();
        assert_eq!(server.requests.lock().unwrap()[0]["method"], "pane.close");
    }

    #[test]
    fn confirmation_timeout_cannot_authorize_removal() {
        let server = server(vec![json!({"result":{"type":"ok"}}), snapshot(&["w1:p1"])]);
        assert!(
            close_checkout_panes(
                &server,
                &hide_node::Local::of_process(),
                &["/fixture/topic".into()],
                &["w1:p1".into()],
                ProcessWait::Skip,
                Duration::from_millis(20)
            )
            .unwrap_err()
            .contains("Timed out")
        );
    }
    #[test]
    fn open_uses_repository_context_and_existing_pane_focus_is_exact() {
        let server = server(vec![
            json!({"result":{"type":"ok"}}),
            json!({"result":{"type":"ok"}}),
        ]);
        open_worktree(&server, "/fixture/topic", "/fixture/main", None).unwrap();
        open_worktree(&server, "/fixture/topic", "/fixture/main", Some("w1:p2")).unwrap();
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests[0]["method"], "worktree.open");
        assert_eq!(
            requests[0]["params"],
            json!({"cwd":"/fixture/main","path":"/fixture/topic","focus":true})
        );
        assert_eq!(requests[1]["method"], "pane.focus");
        assert_eq!(requests[1]["params"], json!({"pane_id":"w1:p2"}));
    }
    #[test]
    fn open_preserves_the_server_rejection() {
        let server = server(vec![
            json!({"error":{"code":"not_found","message":"checkout no longer exists"}}),
        ]);
        assert!(
            open_worktree(&server, "/fixture/topic", "/fixture/main", None)
                .unwrap_err()
                .contains("checkout no longer exists")
        );
    }

    #[test]
    fn created_path_mismatch_is_rolled_back() {
        let missing = "/private/tmp/hide-created-path-missing";
        let server = server(vec![
            worktree_created(missing, "feature"),
            worktree_list(missing, "other"),
            json!({"result":{"type":"ok"}}),
            worktree_list(missing, "other"),
        ]);
        let error = create_worktree(
            &server,
            &hide_node::Local::of_process(),
            true,
            &task("feature"),
        )
        .unwrap_err();
        assert!(error.contains("rolled back"));
        assert!(!Path::new(missing).exists());
        let methods = server
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request["method"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            methods,
            [
                "worktree.create",
                "worktree.list",
                "worktree.remove",
                "worktree.list"
            ]
        );
    }

    /// B27: a host that cannot answer whether the created folder exists
    /// leaves the worktree as created; nothing is rolled back for it.
    #[test]
    fn an_unanswered_folder_check_keeps_the_created_worktree() {
        struct Busy;
        impl crate::node_access::NodeLink for Busy {
            fn call(
                &self,
                _call: hide_node_link::protocol::Call,
                _timeout: std::time::Duration,
            ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
                Err(crate::node_access::LinkError::Unknown(
                    "The device did not answer in time".to_owned(),
                ))
            }
        }
        let path = "/private/tmp/hide-created-unanswered";
        let server = server(vec![
            worktree_created(path, "feature"),
            worktree_list(path, "feature"),
        ]);
        let error = create_worktree(&server, &Busy, false, &task("feature")).unwrap_err();
        assert!(error.contains("it was kept"), "{error}");
        // No retry of the creation can succeed, so the next step is named.
        assert!(error.contains("start the agent in its pane"), "{error}");
        let methods = server
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request["method"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert!(
            !methods.iter().any(|method| method == "worktree.remove"),
            "{methods:?}"
        );
    }

    #[test]
    fn created_path_alias_is_not_a_mismatch() {
        let folder = tempfile::Builder::new()
            .prefix("hide-created-path-alias-")
            .tempdir_in("/tmp")
            .unwrap();
        let root = folder.path().to_path_buf();
        let response_path = root.to_string_lossy().into_owned();
        let listed_path = std::fs::canonicalize(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let server = server(vec![
            worktree_created(&response_path, "feature"),
            worktree_list(&listed_path, "feature"),
        ]);

        let outcome = create_worktree(
            &server,
            &hide_node::Local::of_process(),
            true,
            &task("feature"),
        )
        .unwrap();
        assert_eq!(outcome.path, response_path);
        assert_eq!(
            server
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|request| request["method"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["worktree.create", "worktree.list"]
        );
    }

    #[test]
    fn migrate_branch_blocked_when_dirty() {
        let git = ScriptedGit::new(vec![Ok(" M file")]);
        let server = server(vec![]);
        let error = migrate_branch(
            &server,
            &hide_node::Local::of_process(),
            &git,
            &task("feature"),
        )
        .unwrap_err();
        assert!(error.contains("uncommitted changes"));
        assert!(server.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn migrate_branch_moves_branch_to_worktree() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let path = root.to_string_lossy().into_owned();
        let server = server(vec![
            worktree_created(&path, "feature"),
            worktree_list(&path, "feature"),
        ]);
        let git = ScriptedGit::new(vec![Ok(""), Ok("feature"), Ok("")]);
        let outcome = migrate_branch(
            &server,
            &hide_node::Local::of_process(),
            &git,
            &task("feature"),
        )
        .unwrap();
        assert_eq!(outcome.path, path);
        assert_eq!(
            git.calls.lock().unwrap()[2],
            ["checkout", "--no-overwrite-ignore", "main", "--"]
        );
        let requests = server.requests.lock().unwrap();
        let create = requests
            .iter()
            .find(|request| request["method"] == "worktree.create")
            .unwrap();
        assert_eq!(
            create["params"]["focus"], false,
            "migration must not move Herdr focus"
        );
    }

    #[test]
    fn a_checkout_tab_is_focused_in_herdr_only_when_the_request_asks() {
        for focus in [true, false] {
            let server = server(vec![json!({"result":{
                "type":"tab_created",
                "tab":{"tab_id":"w9:t2","workspace_id":"w9","number":2,"label":"Tab 2","focused":focus,"pane_count":1,"agent_status":"idle"},
                "root_pane":{"pane_id":"w9:p2","terminal_id":"fixture-terminal","workspace_id":"w9","tab_id":"w9:t2","focused":focus,"agent_status":"idle","revision":1}
            }})]);
            let outcome = create_checkout_tab(
                &server,
                &CheckoutTabRequest {
                    id: 1,
                    agent_kind: None,
                    checkout_path: "/repo".into(),
                    label: "Tab 2".into(),
                    host: crate::checkout_owner::TabHost::Workspace("w9".into()),
                    focus,
                    resume_scope: None,
                    resume_reference: None,
                    cwd: "/repo".into(),
                },
                None,
            )
            .unwrap();
            assert_eq!(outcome.pane_id, "w9:p2");
            let requests = server.requests.lock().unwrap();
            assert_eq!(requests[0]["method"], "tab.create");
            assert_eq!(requests[0]["params"]["focus"], focus);
        }
    }

    #[test]
    fn migrate_branch_restores_on_failure() {
        let server = server(vec![
            json!({"error":{"code":"injected","message":"pane creation failed"}}),
        ]);
        let git = ScriptedGit::new(vec![Ok(""), Ok("feature"), Ok(""), Ok("")]);
        let error = migrate_branch(
            &server,
            &hide_node::Local::of_process(),
            &git,
            &task("feature"),
        )
        .unwrap_err();
        assert!(error.contains("restored the main worktree to feature"));
        assert_eq!(
            git.calls.lock().unwrap()[3],
            ["checkout", "--no-overwrite-ignore", "feature", "--"]
        );
        assert_eq!(
            server
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|request| request["method"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["worktree.create"]
        );
    }

    #[test]
    fn migrate_branch_leaves_panes_and_focus() {
        let git = ScriptedGit::new(vec![Ok(""), Ok("feature"), Ok(""), Ok("")]);
        let server = server(vec![
            json!({"error":{"code":"injected","message":"create failed"}}),
        ]);
        let _ = migrate_branch(
            &server,
            &hide_node::Local::of_process(),
            &git,
            &task("feature"),
        );
        assert!(
            server
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|request| request["method"] != "pane.focus"
                    && request["method"] != "pane.close")
        );
    }

    #[test]
    fn migrate_branch_does_not_remove_an_unowned_worktree_after_create_failure() {
        let server = server(vec![
            json!({"error":{"code":"injected","message":"create failed after another client used the branch"}}),
        ]);
        let git = ScriptedGit::new(vec![Ok(""), Ok("feature"), Ok(""), Ok("")]);

        let error = migrate_branch(
            &server,
            &hide_node::Local::of_process(),
            &git,
            &task("feature"),
        )
        .unwrap_err();

        assert!(error.contains("restored the main worktree to feature"));
        assert_eq!(
            server
                .requests
                .lock()
                .unwrap()
                .iter()
                .map(|request| request["method"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["worktree.create"],
            "an ambiguous create failure must not remove a worktree it cannot prove it owns"
        );
    }

    #[test]
    fn migrate_branch_checkout_failure_leaves_repo_untouched() {
        let git = ScriptedGit::new(vec![
            Ok(""),
            Ok("feature"),
            Err("injected checkout failure"),
        ]);
        let server = server(vec![]);
        let error = migrate_branch(
            &server,
            &hide_node::Local::of_process(),
            &git,
            &task("feature"),
        )
        .unwrap_err();
        assert!(error.contains("checkout base branch"));
        assert_eq!(git.calls.lock().unwrap().len(), 3);
        assert!(server.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn migrate_branch_refuses_before_overwriting_an_ignored_file() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let run = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["init", "-b", "main"]);
        run(&["config", "user.name", "Fixture"]);
        run(&["config", "user.email", "fixture@example.invalid"]);
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("ignored.txt"), "base bytes").unwrap();
        run(&["add", "ignored.txt"]);
        run(&["commit", "-m", "base"]);
        run(&["checkout", "-b", "feature"]);
        run(&["rm", "ignored.txt"]);
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        run(&["add", ".gitignore"]);
        run(&["commit", "-m", "ignore path"]);
        std::fs::write(root.join("ignored.txt"), "private ignored bytes").unwrap();

        let server = server(vec![]);
        let request = WorktreeTaskRequest {
            repository_root: root.to_string_lossy().into_owned(),
            branch: "feature".into(),
            base_branch: Some("main".into()),
            ..task("feature")
        };
        let node = hide_node::Local::of_process();
        let error = migrate_branch(&server, &node, &NodeGit(&node), &request).unwrap_err();

        assert!(error.contains("checkout base branch"));
        assert_eq!(
            std::fs::read_to_string(root.join("ignored.txt")).unwrap(),
            "private ignored bytes"
        );
        let branch = NodeGit(&node)
            .run(root.to_str().unwrap(), GitCommand::CurrentBranch)
            .unwrap();
        assert_eq!(branch, "feature");
        assert!(server.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn migrate_branch_restore_failure_reports_repo_state() {
        let server = server(vec![
            json!({"error":{"code":"injected","message":"create failed"}}),
        ]);
        let git = ScriptedGit::new(vec![
            Ok(""),
            Ok("feature"),
            Ok(""),
            Err("injected restore failure"),
            Ok("main"),
        ]);
        let error = migrate_branch(
            &server,
            &hide_node::Local::of_process(),
            &git,
            &task("feature"),
        )
        .unwrap_err();
        assert!(error.contains("main worktree is now on main"));
        assert!(error.contains("open the repository at /fixture/repo and check out feature"));
    }

    fn agent(status: &str) -> Value {
        json!({
            "pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "terminal_id": "term_1",
            "agent": "claude", "agent_status": status, "state_change_seq": 1,
            "focused": false, "interactive_ready": true, "revision": 0
        })
    }
    fn shell_ready() -> Value {
        json!({"result":{"type":"pane_process_info","process_info":{
            "pane_id":"w1:p1","shell_pid":4100,"foreground_process_group_id":4100,
            "foreground_processes":[{"pid":4100,"name":"zsh"}]
        }}})
    }
    fn requests_of(server: &Server) -> Vec<Value> {
        server.requests.lock().unwrap().clone()
    }

    fn methods_of(requests: &[Value]) -> Vec<&str> {
        requests
            .iter()
            .map(|request| request["method"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn a_native_archive_resume_never_launches_an_unconfirmed_or_superseded_owner() {
        for (kind, flag) in [("pi", "--session"), ("omp", "--resume")] {
            native_archive_resume_journey(kind, flag);
        }
    }

    fn native_archive_resume_journey(kind: &str, flag: &str) {
        let home = tempfile::tempdir().unwrap();
        let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
        let folder = crate::fixture::native_session_folder(home.path(), kind, &cwd);
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("native.jsonl");
        let header = format!(
            "{}\n",
            json!({"type":"session", "version":3, "id":"native-pi", "cwd":cwd})
        );
        std::fs::write(&path, &header).unwrap();
        std::fs::copy(&path, folder.join("duplicate.jsonl")).unwrap();
        let node = hide_node::Local::new(Some(home.path().to_path_buf()));
        let make = || PendingAgentStart {
            resume_reference: Some(crate::sidebar::SessionAgentSessionPayload {
                kind: "path".into(),
                value: path.display().to_string(),
            }),
            pane_id: "w1:p1".into(),
            kind: kind.into(),
            prompt: None,
            args: vec![flag.into(), "native-pi".into()],
            resume_scope: Some(hide_session::SessionReadScope {
                id: "native-pi".into(),
                cwd: cwd.display().to_string(),
            }),
            codex_daemon: Default::default(),
            shell: crate::agent_start::ShellStart::Plain,
            made_worktree: None,
        };
        let server = server(vec![]);
        assert_eq!(
            launch_with_prompt(&server, Some(&node), 7, make()),
            TaskAgentOutcome::Failed("session_route_unconfirmed".into())
        );
        let mut missing_scope = make();
        missing_scope.resume_scope = None;
        assert_eq!(
            launch_with_prompt(&server, Some(&node), 7, missing_scope),
            TaskAgentOutcome::Failed("session_resume_scope_required".into())
        );
        std::fs::remove_file(folder.join("duplicate.jsonl")).unwrap();
        let other_history = format!(
            "{header}{}\n",
            json!({"type":"message", "id":"other-message", "message":{"role":"assistant", "content":[{"type":"text", "text":"different history"}]}})
        );
        std::fs::write(folder.join("other.jsonl"), &other_history).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            launch_with_prompt(&server, Some(&node), 7, make()),
            TaskAgentOutcome::Failed("session_route_unconfirmed".into())
        );
        assert_eq!(
            std::fs::read_to_string(folder.join("other.jsonl")).unwrap(),
            other_history
        );
        std::fs::remove_file(folder.join("other.jsonl")).unwrap();
        std::fs::write(&path, header.replace("native-pi", "native-pi-prefix")).unwrap();
        assert_eq!(
            launch_with_prompt(&server, Some(&node), 7, make()),
            TaskAgentOutcome::Failed("session_route_unconfirmed".into())
        );
        std::fs::write(&path, &header).unwrap();
        assert_eq!(
            launch_with_prompt_checked(&server, Some(&node), Some(&node), 7, make(), &|| Err(
                "session_resume_intent_changed".into()
            )),
            TaskAgentOutcome::Failed("session_resume_intent_changed".into())
        );
        assert!(requests_of(&server).is_empty());
        assert_eq!(std::fs::read_to_string(path).unwrap(), header);
    }

    /// The installed CLI help fixes these expectations independently of the
    /// adapter. Every first prompt travels in argv, never as PTY input.
    #[test]
    fn a_first_prompt_travels_as_the_agents_own_argument() {
        // OpenCode takes no model, so its start names none.
        for (kind, prompt_flag, extra_roots, model) in [
            ("claude", "--", true, Some("fixture-model")),
            ("codex", "--", true, Some("fixture-model")),
            ("grok", "--", false, Some("fixture-model")),
            ("opencode", "--prompt", false, None),
            ("pi", "--", false, Some("fixture-model")),
            ("omp", "--", true, Some("fixture-model")),
            ("cursor", "--", true, Some("fixture-model")),
        ] {
            let server = server(vec![
                shell_ready(),
                json!({"result":{"type":"agent_started","argv":[],"agent":agent("working")}}),
            ]);
            let args = crate::runtime::agent_choice::agent_arguments(
                Some(kind),
                model,
                &["/work/my app".into()],
            );
            let outcome = launch_with_prompt(
                &server,
                None,
                7,
                PendingAgentStart {
                    resume_reference: None,
                    resume_scope: None,
                    pane_id: "w1:p1".into(),
                    kind: kind.into(),
                    prompt: Some("fix the tests".into()),
                    args,
                    codex_daemon: crate::codex_launch::CodexDaemon::Unsupported,
                    shell: crate::agent_start::ShellStart::Plain,
                    made_worktree: None,
                },
            );
            assert!(
                matches!(outcome, TaskAgentOutcome::Started),
                "{kind}: {outcome:?}"
            );
            let requests = requests_of(&server);
            assert_eq!(
                methods_of(&requests),
                ["pane.process_info", "agent.start"],
                "{kind}"
            );
            let mut expected: Vec<&str> = model.into_iter().flat_map(|m| ["--model", m]).collect();
            if extra_roots {
                expected.extend(["--add-dir", "/work/my app"]);
            }
            expected.extend([prompt_flag, "fix the tests"]);
            assert_eq!(requests[1]["params"]["args"], json!(expected), "{kind}");
            assert_eq!(requests[1]["params"]["kind"], kind);
        }
    }

    /// PRD overview-request-view D-20: a Codex that has the shared daemon is
    /// started without it, ahead of its first prompt.
    #[test]
    fn a_device_worktree_start_records_trust_on_the_pane_node_or_refuses_without_one() {
        let home = tempfile::tempdir().unwrap();
        let tree = home.path().join("tree");
        std::fs::create_dir_all(tree.join(".git")).unwrap();
        let wire = hide_platform::path::to_wire(&tree).unwrap();
        let resolved = hide_platform::fs::identity::canonical(&tree).unwrap();
        let node = hide_node::Local::new(Some(home.path().to_path_buf()));
        let make = || PendingAgentStart {
            resume_reference: None,
            resume_scope: None,
            pane_id: "w1:p1".into(),
            kind: "claude".into(),
            prompt: None,
            args: Vec::new(),
            codex_daemon: crate::codex_launch::CodexDaemon::Unsupported,
            shell: crate::agent_start::ShellStart::Plain,
            made_worktree: Some(wire.clone()),
        };
        // A device start: no agent node (its Herdr answers for its PATH), a
        // pane node that is the device's.
        let server = server(vec![
            shell_ready(),
            json!({"result":{"type":"agent_started","argv":[],"agent":agent("working")}}),
        ]);
        let outcome = launch_with_prompt_checked(&server, None, Some(&node), 7, make(), &|| Ok(()));
        assert!(matches!(outcome, TaskAgentOutcome::Started), "{outcome:?}");
        let written: Value =
            serde_json::from_slice(&std::fs::read(home.path().join(".claude.json")).unwrap())
                .unwrap();
        let key = resolved.to_str().unwrap().replace('\\', "/");
        assert_eq!(
            written["projects"][key.as_str()]["hasTrustDialogAccepted"],
            true
        );
        // With no pane node the start refuses before Herdr is asked.
        let server = self::server(vec![]);
        let TaskAgentOutcome::Failed(reason) =
            launch_with_prompt_checked(&server, None, None, 7, make(), &|| Ok(()))
        else {
            panic!("a start without the pane's machine went ahead")
        };
        assert!(reason.starts_with("agent_trust_unavailable"), "{reason}");
        assert!(requests_of(&server).is_empty());
    }

    #[test]
    fn an_unknown_codex_first_start_refuses_before_contacting_herdr() {
        let server = server(vec![]);
        let outcome = launch_with_prompt(
            &server,
            None,
            7,
            PendingAgentStart {
                resume_reference: None,
                resume_scope: None,
                pane_id: "w1:p1".into(),
                kind: "codex".into(),
                prompt: Some("fix the tests".into()),
                args: Vec::new(),
                codex_daemon: crate::codex_launch::CodexDaemon::Unknown,
                shell: crate::agent_start::ShellStart::Plain,
                made_worktree: None,
            },
        );
        let TaskAgentOutcome::Failed(reason) = outcome else {
            panic!("start succeeded")
        };
        assert_eq!(reason, crate::codex_launch::CAPABILITY_UNKNOWN);
        assert!(requests_of(&server).is_empty());
    }

    /// The chosen agent is looked up on the node that would run it, and one
    /// it lacks fails the start before Herdr types anything into the pane.
    #[test]
    fn an_agent_missing_from_its_node_fails_before_contacting_herdr() {
        struct Missing(Mutex<Vec<hide_node_link::protocol::Call>>);
        impl crate::node_access::NodeLink for Missing {
            fn call(
                &self,
                call: hide_node_link::protocol::Call,
                _timeout: std::time::Duration,
            ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
                self.0.lock().unwrap().push(call);
                Ok(crate::node_access::LinkAnswer::Parsed(json!(false)))
            }
        }
        let node = Missing(Mutex::default());
        let server = server(vec![]);
        let outcome = launch_with_prompt(
            &server,
            Some(&node),
            7,
            PendingAgentStart {
                resume_reference: None,
                resume_scope: None,
                pane_id: "w1:p1".into(),
                kind: "claude".into(),
                prompt: None,
                args: Vec::new(),
                codex_daemon: Default::default(),
                shell: crate::agent_start::ShellStart::Plain,
                made_worktree: None,
            },
        );
        assert_eq!(
            outcome,
            TaskAgentOutcome::Failed(
                "claude is not installed on the daemon's PATH. Install it, then retry.".into()
            )
        );
        assert_eq!(
            *node.0.lock().unwrap(),
            [hide_node_link::protocol::Call::AgentInstalled {
                name: "claude".into()
            }]
        );
        assert!(requests_of(&server).is_empty());
    }

    #[test]
    fn a_codex_with_the_shared_daemon_starts_without_it() {
        let server = server(vec![
            shell_ready(),
            json!({"result":{"type":"agent_started","argv":[],"agent":agent("working")}}),
        ]);
        let outcome = launch_with_prompt(
            &server,
            None,
            7,
            PendingAgentStart {
                resume_reference: None,
                resume_scope: None,
                pane_id: "w1:p1".into(),
                kind: "codex".into(),
                prompt: Some("fix the tests".into()),
                args: Vec::new(),
                codex_daemon: crate::codex_launch::CodexDaemon::Present,
                shell: crate::agent_start::ShellStart::Plain,
                made_worktree: None,
            },
        );
        assert!(matches!(outcome, TaskAgentOutcome::Started), "{outcome:?}");
        let requests = requests_of(&server);
        assert_eq!(
            requests[1]["params"]["args"],
            json!(["--no-daemon", "--", "fix the tests"])
        );
    }

    /// D-26, B31: a multi-line prompt still travels as the CLI's own
    /// argument, its line breaks as U+2028 and its tabs as spaces; nothing is
    /// typed into the pane, where a startup question would take it.
    #[test]
    fn a_multi_line_prompt_travels_as_one_argument_and_is_never_typed() {
        let server = server(vec![
            shell_ready(),
            json!({"result":{"type":"agent_started","argv":[],"agent":agent("idle")}}),
        ]);
        let outcome = launch_with_prompt(
            &server,
            None,
            7,
            PendingAgentStart {
                resume_reference: None,
                resume_scope: None,
                pane_id: "w1:p1".into(),
                kind: "claude".into(),
                prompt: Some("1\nfix the tests\r\n\tthen push".into()),
                args: vec!["--model".into(), "opus".into()],
                codex_daemon: Default::default(),
                shell: crate::agent_start::ShellStart::Plain,
                made_worktree: None,
            },
        );
        assert!(matches!(outcome, TaskAgentOutcome::Started), "{outcome:?}");
        let requests = requests_of(&server);
        assert_eq!(methods_of(&requests), ["pane.process_info", "agent.start"]);
        assert_eq!(
            requests[1]["params"]["args"],
            json!([
                "--model",
                "opus",
                "--",
                "1\u{2028}fix the tests\u{2028}    then push"
            ])
        );
    }

    /// B31: a prompt with a character that cannot be passed fails the start
    /// with the reason, and Herdr is not asked at all.
    #[test]
    fn a_prompt_with_another_control_character_fails_the_start_before_herdr() {
        let server = server(vec![]);
        let outcome = launch_with_prompt(
            &server,
            None,
            7,
            PendingAgentStart {
                resume_reference: None,
                resume_scope: None,
                pane_id: "w1:p1".into(),
                kind: "claude".into(),
                prompt: Some("fix\u{7}the bell".into()),
                args: Vec::new(),
                codex_daemon: Default::default(),
                shell: crate::agent_start::ShellStart::Plain,
                made_worktree: None,
            },
        );
        let TaskAgentOutcome::Failed(message) = outcome else {
            panic!("expected a failed start, got {outcome:?}");
        };
        assert!(message.contains("U+0007"), "{message}");
        assert!(requests_of(&server).is_empty());
    }

    /// A prompt over the cap fails the start before Herdr is asked, rather
    /// than failing inside the pane's shell where no one reads it.
    #[test]
    fn a_prompt_over_the_cap_fails_the_start_before_herdr() {
        let server = server(vec![]);
        let at_cap = "가".repeat(MAX_PROMPT_BYTES / 3);
        assert!(prompt_argument(&at_cap).is_ok());
        let outcome = launch_with_prompt(
            &server,
            None,
            7,
            PendingAgentStart {
                resume_reference: None,
                resume_scope: None,
                pane_id: "w1:p1".into(),
                kind: "claude".into(),
                prompt: Some(format!("{at_cap}a\n")),
                args: Vec::new(),
                codex_daemon: Default::default(),
                shell: crate::agent_start::ShellStart::Plain,
                made_worktree: None,
            },
        );
        let TaskAgentOutcome::Failed(message) = outcome else {
            panic!("expected a failed start, got {outcome:?}");
        };
        assert!(message.contains("64 KB"), "{message}");
        assert!(requests_of(&server).is_empty());
    }

    /// B31: a start Herdr refuses is a failed start with Herdr's reason; the
    /// agent is not started again without its prompt.
    #[test]
    fn a_refused_start_is_reported_and_not_retried_without_the_prompt() {
        let server = server(vec![
            shell_ready(),
            json!({"error":{"code":"invalid_agent_argument","message":"agent arguments cannot be encoded safely for the target shell"}}),
        ]);
        let outcome = launch_with_prompt(
            &server,
            None,
            7,
            PendingAgentStart {
                resume_reference: None,
                resume_scope: None,
                pane_id: "w1:p1".into(),
                kind: "claude".into(),
                prompt: Some("fix the tests".into()),
                args: Vec::new(),
                codex_daemon: Default::default(),
                shell: crate::agent_start::ShellStart::Plain,
                made_worktree: None,
            },
        );
        let TaskAgentOutcome::Failed(message) = outcome else {
            panic!("expected a failed start, got {outcome:?}");
        };
        assert!(message.contains("invalid_agent_argument"), "{message}");
        assert_eq!(
            methods_of(&requests_of(&server)),
            ["pane.process_info", "agent.start"]
        );
    }
}
