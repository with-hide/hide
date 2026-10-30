//! Opening a checkout's owner workspace and making its tabs through the
//! engine: a newly registered project's workspace, a tab in an owner that is
//! open or one opened for it, and a tab replaced by one that runs a command.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::engine::Engine;
use serde_json::json;

use crate::checkout_owner::OwnerOpen;
use crate::live::{
    CONTROL_WAIT, ControlFailure, LiveContext, SessionFetchError, fetch_session, mutation_failure,
};
use crate::model::{WorkspaceRegistration, WorkspaceSnapshot};
use crate::node_access::call_as;
use crate::runtime::Runtime;
use crate::sidebar::SessionSnapshotPayload;
use crate::workspace;
use hide_node_link::project::ProjectCreated;
use hide_node_link::protocol::Call;

pub struct WorkspaceCreationOutcome {
    pub registration: WorkspaceRegistration,
    pub base_registrations: Vec<WorkspaceRegistration>,
    pub registrations: Vec<WorkspaceRegistration>,
    pub workspaces: Vec<WorkspaceSnapshot>,
    /// What the node said of the paths `workspaces` was built from.
    pub paths: Arc<workspace::PathIndex>,
    pub session: SessionSnapshotPayload,
    pub created_pane_id: Option<String>,
    pub git_init_error: Option<String>,
}

/// Why a project's registration failed: the `last_error` kind the shell words
/// the failure by, and the sentence the diagnostic log keeps (Herdr's protocol
/// answer, a socket path, an io error), which no screen draws.
#[derive(Debug)]
pub struct WorkspaceCreationFailure {
    pub kind: &'static str,
    pub message: String,
}

/// The node's answer to `ProjectCreate`, when it is the folder hided judged:
/// the event carries the real path the boundary checked, so another answer
/// means a link changed under it since, and that folder is not registered.
fn the_folder_checked(
    created: ProjectCreated,
    path: &str,
) -> Result<ProjectCreated, WorkspaceCreationFailure> {
    // Compared as paths, so `/` and `\` spell one folder on Windows.
    if std::path::Path::new(&created.path) == std::path::Path::new(path) {
        return Ok(created);
    }
    Err(WorkspaceCreationFailure::failed(format!(
        "{path} now leads to {}, which is not the folder that was checked; add it again",
        created.path
    )))
}

impl WorkspaceCreationFailure {
    /// A failure the operator meets as "the project could not be added".
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            kind: "workspace.create_failed",
            message: message.into(),
        }
    }

    /// The node refused to make or read the folder: a missing folder and a
    /// taken name are the operator's to fix, anything else is the generic kind.
    fn from_project_create(error: crate::node_access::LinkError) -> Self {
        let kind = match &error {
            crate::node_access::LinkError::Refused(refused) => match refused.code {
                hide_node_link::ErrorCode::NotFound => "workspace.create_refused.not_found",
                hide_node_link::ErrorCode::AlreadyExists => {
                    "workspace.create_refused.already_exists"
                }
                _ => "workspace.create_failed",
            },
            _ => "workspace.create_failed",
        };
        Self {
            kind,
            message: error.to_string(),
        }
    }

    /// Herdr could not be read: not running or unreachable is the connection's
    /// state, which the shell words as Herdr not connected yet. The read
    /// `after` the project's workspace was opened leaves whether it was added
    /// unknown, so it is worded as unconfirmed rather than as a plain retry.
    fn from_session_read(step: &str, error: &SessionFetchError) -> Self {
        let kind = match error {
            SessionFetchError::SocketMissing(_) | SessionFetchError::Unreachable(_)
                if step == "after" =>
            {
                "workspace.create_unconfirmed"
            }
            SessionFetchError::SocketMissing(_) | SessionFetchError::Unreachable(_) => {
                "workspace.control_unavailable"
            }
            _ => "workspace.create_failed",
        };
        Self {
            kind,
            message: format!(
                "session.snapshot {step} workspace creation failed: {}",
                error.message()
            ),
        }
    }

    /// Herdr's answer to opening the project's first workspace: a lost answer
    /// leaves whether it was made unknown, which must not read as a refusal a
    /// plain retry would repeat.
    fn from_owner(error: &ControlFailure) -> Self {
        Self {
            kind: if error.is_ambiguous() {
                "workspace.create_unconfirmed"
            } else {
                "workspace.create_failed"
            },
            message: error.message().to_owned(),
        }
    }
}

pub fn spawn_workspace_creation(
    context: LiveContext,
    node: crate::node::NodeId,
    path: String,
    label: String,
    initialize_git: bool,
    new_folder: bool,
    base_registrations: Vec<WorkspaceRegistration>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-workspace-create".to_owned())
        .spawn(move || {
            let started = Instant::now();
            let request_path = path.clone();
            // The catalog this worker builds replaces the navigator's, so it
            // must carry the worktree rows the reader has already found.
            // Building it from an empty catalog would drop every worktree
            // without a pane until the next read.
            let worktrees = context
                .runtime
                .upgrade()
                .and_then(|runtime| {
                    let read = runtime.lock().ok().map(|guard| guard.worktree_catalog());
                    drop(runtime);
                    read
                })
                .unwrap_or_default();
            // The node makes the folder and the repository, and answers the
            // real path the project is registered by. hided judged the path
            // this event carries as already real, so an answer that differs
            // means a link changed under it since: that folder was never
            // judged, and is not registered.
            let made: Result<ProjectCreated, WorkspaceCreationFailure> = call_as(
                context.node.as_ref(),
                Call::ProjectCreate {
                    path: path.clone(),
                    new_folder,
                    initialize_git,
                },
                PROJECT_CREATE_TIMEOUT,
            )
            .map_err(WorkspaceCreationFailure::from_project_create)
            .and_then(|created| the_folder_checked(created, &path));
            let result = made
                .and_then(|created| {
                    workspace::registration(&created.path, &label, node.as_str())
                        .map(|registration| (registration, created.git_error))
                        .map_err(WorkspaceCreationFailure::failed)
                })
                .and_then(|(registration, git_init_error)| {
                    let mut registrations = base_registrations.clone();
                    if !registrations
                        .iter()
                        .any(|existing| existing.id == registration.id)
                    {
                        registrations.push(registration.clone());
                    }
                    let ask = |session: &SessionSnapshotPayload| {
                        let mut wanted =
                            workspace::PathIndex::wanted(&node, &registrations, &[], &worktrees);
                        wanted.extend(Runtime::session_paths(session));
                        workspace::PathIndex::ask(context.node.as_ref(), wanted)
                            .map_err(WorkspaceCreationFailure::failed)
                    };
                    let before = fetch_session(context.engine.as_ref()).map_err(|error| {
                        WorkspaceCreationFailure::from_session_read("before", &error)
                    })?;
                    let before_paths = ask(&before)?;
                    let before_spaces = Runtime::session_spaces(&before, &before_paths);
                    let before_catalog = workspace::build_catalog(
                        &node,
                        &registrations,
                        &before_spaces,
                        &worktrees,
                        &before_paths,
                    );
                    let needs_herdr_workspace = before_catalog
                        .iter()
                        .any(|workspace| workspace.id == registration.id);
                    // The project's first workspace is its checkout's owner
                    // (PRD checkout-workspace-binding B12), never a plain
                    // workspace a later tab would pass over.
                    let created = needs_herdr_workspace
                        .then(|| {
                            ensure_owner(
                                context.engine.as_ref(),
                                &registered_owner(
                                    &node,
                                    &before_paths,
                                    &registration.path,
                                    &registration.label,
                                ),
                                true,
                                Default::default(),
                            )
                            .map_err(|error| WorkspaceCreationFailure::from_owner(&error))
                        })
                        .transpose()?;
                    let session = if created.is_some() {
                        fetch_session(context.engine.as_ref()).map_err(|error| {
                            WorkspaceCreationFailure::from_session_read("after", &error)
                        })?
                    } else {
                        before
                    };
                    let paths = if created.is_some() {
                        ask(&session)?
                    } else {
                        before_paths
                    };
                    let spaces = Runtime::session_spaces(&session, &paths);
                    let workspaces = workspace::build_catalog(
                        &node,
                        &registrations,
                        &spaces,
                        &worktrees,
                        &paths,
                    );
                    Ok(WorkspaceCreationOutcome {
                        registration,
                        base_registrations,
                        registrations,
                        workspaces,
                        paths: Arc::new(paths),
                        session,
                        created_pane_id: created
                            .and_then(|(_, first_tab)| first_tab.map(|(_, pane_id, _)| pane_id)),
                        git_init_error,
                    })
                });
            let elapsed_ms = started.elapsed().as_millis();
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => guard.ingest_workspace_creation(&request_path, result, elapsed_ms),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("workspace creation worker could not be started: {error}"))
}

/// The owner a newly registered project's checkout gets: a Git checkout is
/// opened from its repository's main worktree, a plain folder is created
/// and marked at the registered path, the same path the catalog keys it by.
fn registered_owner(
    node: &crate::node::NodeId,
    paths: &workspace::PathIndex,
    path: &str,
    label: &str,
) -> OwnerOpen {
    match paths.place(path) {
        Some(place) => {
            OwnerOpen::for_checkout(node.as_str(), &place.root, &place.main_root, true, label)
        }
        None => OwnerOpen::for_checkout(node.as_str(), path, path, false, label).on_node(),
    }
}

/// How long the node may take to make a project's folder and repository.
const PROJECT_CREATE_TIMEOUT: Duration = Duration::from_secs(30);

/// A tab Hide created in a checkout's owner Herdr workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OwnedTab {
    /// Label returned by creation, only while retaining that exact first tab.
    pub(crate) initial_label: Option<String>,
    pub(crate) workspace_id: String,
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
    /// Whether the pane was started the way it was asked for, its `env` and
    /// its command: a workspace's first tab that `worktree.open` made has
    /// neither, and one `workspace.create` made has no command.
    pub(crate) launched: bool,
}

/// Plain-folder owners are found and made one at a time, so two requests
/// racing for a folder with no owner converge on one workspace; a Git
/// checkout converges through `worktree.open` itself (`already_open`). The
/// lock spans every folder and device rather than one folder: an open here
/// happens once per folder per Herdr server life (D-11), and each call it
/// holds across is bounded by its own timeout, so the rare wait this costs
/// an unrelated folder is not worth a keyed lock registry.
static FOLDER_OWNER_OPEN: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Opens a checkout's owner and creates one tab in it (PRD
/// checkout-workspace-binding B2, B12, B13): a newly opened owner's first tab
/// is the new tab, and an owner that was already open gets a new tab. A
/// refusal fails the request; nothing is created in any other workspace (B3).
/// With a `command`, the tab's pane runs it (`layout.apply`); a newly opened
/// owner's first tab, which Herdr starts with its own shell, is replaced by
/// one that does (PRD agent-start-shell-ready D-10).
pub(crate) fn open_owner_tab(
    engine: &dyn Engine,
    owner: &OwnerOpen,
    cwd: &str,
    label: &str,
    focus: bool,
    env: std::collections::BTreeMap<String, String>,
    command: Option<&[String]>,
) -> Result<OwnedTab, ControlFailure> {
    let (workspace_id, first_tab) = ensure_owner(engine, owner, focus, env.clone())?;
    let owned = match first_tab {
        Some((tab_id, pane_id, initial_label)) if command.is_some() => replace_tab(
            engine,
            OwnedTab {
                initial_label,
                workspace_id,
                tab_id,
                pane_id,
                launched: false,
            },
            cwd,
            label,
            focus,
            env,
            command,
        ),
        Some((tab_id, pane_id, initial_label)) => {
            // The owner's first tab is the new tab, so it takes the label the
            // request carries, as `tab.create` would give it; the tab is real
            // either way.
            if !label.trim().is_empty()
                && let Err(error) = engine
                    .rename_tab(CONTROL_WAIT, &tab_id, label)
                    .map_err(mutation_failure("tab.rename"))
            {
                crate::diagnostic!(json!({
                    "component": "checkout_owner",
                    "kind": "owner.tab_label_failed",
                    "workspace_id": workspace_id,
                    "tab_id": tab_id,
                    "message": error.message(),
                }));
            }
            OwnedTab {
                initial_label,
                workspace_id,
                tab_id,
                pane_id,
                launched: matches!(owner, OwnerOpen::Folder { .. }),
            }
        }
        None => create_tab_in(engine, &workspace_id, cwd, label, focus, env, command)?,
    };
    crate::diagnostic!(json!({
        "component": "checkout_owner",
        "kind": "owner.tab_created",
        "path": owner.path(),
        "method": owner.kind(),
        "workspace_id": owned.workspace_id,
        "tab_id": owned.tab_id,
    }));
    Ok(owned)
}

/// The checkout's owner workspace, opened when none is open, and the tab and
/// pane Herdr made with it when this call opened it (`None` when it was
/// already open). `env` reaches the first pane only where Herdr takes it
/// (`workspace.create`); `worktree.open` takes none.
type FirstTab = (String, String, Option<String>);

pub(crate) fn ensure_owner(
    engine: &dyn Engine,
    owner: &OwnerOpen,
    focus: bool,
    env: std::collections::BTreeMap<String, String>,
) -> Result<(String, Option<FirstTab>), ControlFailure> {
    match owner {
        OwnerOpen::Worktree {
            path,
            repository_root,
            label,
        } => {
            let opened = engine
                .open_worktree(CONTROL_WAIT, path, repository_root, focus)
                .map_err(mutation_failure("worktree.open"))?;
            if opened.already_open {
                return Ok((opened.workspace_id, None));
            }
            // Only a workspace Herdr opened for this request takes the
            // checkout's name (D-14); the tab is real either way.
            if !label.trim().is_empty()
                && opened.label != *label
                && let Err(error) = engine
                    .rename_workspace(CONTROL_WAIT, &opened.workspace_id, label)
                    .map_err(mutation_failure("workspace.rename"))
            {
                crate::diagnostic!(json!({
                    "component": "checkout_owner",
                    "kind": "owner.label_failed",
                    "workspace_id": opened.workspace_id,
                    "path": path,
                    "message": error.message(),
                }));
            }
            Ok((
                opened.workspace_id,
                Some((opened.tab_id, opened.pane_id, Some(opened.tab_label))),
            ))
        }
        OwnerOpen::Folder {
            path, label, mark, ..
        } => {
            let _serialized = FOLDER_OWNER_OPEN
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let listed = engine
                .workspace_tokens(CONTROL_WAIT, crate::checkout_owner::OWNER_TOKEN)
                .map_err(|error| ControlFailure::Definite(error.within("workspace.list failed")))?;
            if let Some((workspace_id, _)) = listed
                .into_iter()
                .find(|(_, value)| value.as_deref().is_some_and(|value| owner.marks(value)))
            {
                return Ok((workspace_id, None));
            }
            let (workspace_id, tab_id, pane_id) = engine
                .create_workspace(CONTROL_WAIT, path, label, focus, env)
                .map_err(mutation_failure("workspace.create"))?;
            // The tab is real either way; without its mark only the next tab
            // in this folder makes another owner.
            if let Err(error) = engine
                .report_workspace_token(
                    CONTROL_WAIT,
                    &workspace_id,
                    crate::checkout_owner::OWNER_TOKEN,
                    Some(mark),
                )
                .map_err(mutation_failure("workspace.report_metadata"))
            {
                crate::diagnostic!(json!({
                    "component": "checkout_owner",
                    "kind": "owner.mark_failed",
                    "workspace_id": workspace_id,
                    "path": path,
                    "message": error.message(),
                }));
            }
            Ok((workspace_id, Some((tab_id, pane_id, None))))
        }
    }
}

/// A new tab in `workspace_id` whose pane is started with `env`, running
/// `command` when there is one (`layout.apply`) and Herdr's own shell
/// otherwise (`tab.create`).
pub(crate) fn create_tab_in(
    engine: &dyn Engine,
    workspace_id: &str,
    cwd: &str,
    label: &str,
    focus: bool,
    env: std::collections::BTreeMap<String, String>,
    command: Option<&[String]>,
) -> Result<OwnedTab, ControlFailure> {
    let (tab_id, pane_id) = match command {
        Some(command) => engine
            .launch_tab(
                CONTROL_WAIT,
                workspace_id,
                None,
                label,
                focus,
                cwd,
                env,
                command,
            )
            .map_err(mutation_failure("layout.apply")),
        None => engine
            .create_tab(CONTROL_WAIT, workspace_id, cwd, label, focus, env)
            .map_err(mutation_failure("tab.create")),
    }?;
    Ok(OwnedTab {
        initial_label: None,
        workspace_id: workspace_id.to_owned(),
        tab_id,
        pane_id,
        launched: true,
    })
}

/// `tab`, a workspace's first tab Herdr started with its own shell, replaced
/// by one whose pane runs `command` with `env` (`layout.apply` with the
/// tab's id): `worktree.create`, `worktree.open` and `workspace.create` take
/// no command. Nothing is shown of it, since the workspace was made for this
/// start. When the replacement fails, `tab` itself is answered, not
/// launched, and the failure is logged.
#[allow(clippy::too_many_arguments)] // the tab to replace and what its pane runs
pub(crate) fn replace_tab(
    engine: &dyn Engine,
    tab: OwnedTab,
    cwd: &str,
    label: &str,
    focus: bool,
    env: std::collections::BTreeMap<String, String>,
    command: Option<&[String]>,
) -> OwnedTab {
    let Some(command) = command else {
        return tab;
    };
    let replaced = engine
        .launch_tab(
            CONTROL_WAIT,
            &tab.workspace_id,
            Some(&tab.tab_id),
            label,
            focus,
            cwd,
            env,
            command,
        )
        .map_err(mutation_failure("layout.apply"));
    crate::diagnostic!(json!({
        "component": "agent_start",
        "kind": "launched_tab.replaced",
        "workspace_id": tab.workspace_id,
        "replaced": tab.tab_id,
        "tab_id": replaced.as_ref().ok().map(|(tab_id, _)| tab_id),
        "reason": replaced.as_ref().err().map(ControlFailure::message),
    }));
    match replaced {
        Ok((tab_id, pane_id)) => OwnedTab {
            initial_label: None,
            workspace_id: tab.workspace_id,
            tab_id,
            pane_id,
            launched: true,
        },
        Err(_) => tab,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::sync::Mutex;

    use super::*;
    use crate::fake_herdr::FakeHerdr;
    use crate::live::{RemoteControlAction, RemoteControlOutcome, execute_remote_control};

    /// PRD wsl-device B15, B17: the folder registered is the one hided
    /// judged; a link swapped under the path between the check and the
    /// node's answer registers nothing.
    #[test]
    fn a_folder_that_moved_since_its_check_is_not_registered() {
        let created = |path: &str| ProjectCreated {
            path: path.to_owned(),
            git_error: None,
        };
        assert!(the_folder_checked(created("/srv/app"), "/srv/app").is_ok());
        let moved = the_folder_checked(created("/etc"), "/srv/app").unwrap_err();
        assert_eq!(moved.kind, "workspace.create_failed");
        assert!(moved.message.contains("/etc"), "{}", moved.message);
    }

    #[test]
    fn a_failed_registration_is_named_by_what_the_operator_can_do_about_it() {
        let refused = |code| {
            WorkspaceCreationFailure::from_project_create(crate::node_access::LinkError::Refused(
                hide_node_link::HostError::new(code, "Workspace path does not exist: /x"),
            ))
            .kind
        };
        assert_eq!(
            refused(hide_node_link::ErrorCode::NotFound),
            "workspace.create_refused.not_found"
        );
        assert_eq!(
            refused(hide_node_link::ErrorCode::AlreadyExists),
            "workspace.create_refused.already_exists"
        );
        assert_eq!(
            refused(hide_node_link::ErrorCode::Io),
            "workspace.create_failed"
        );
        let missing =
            SessionFetchError::SocketMissing("Herdr socket file does not exist at /x.sock".into());
        let read = WorkspaceCreationFailure::from_session_read("before", &missing);
        assert_eq!(read.kind, "workspace.control_unavailable");
        // The sentence stays for the diagnostic log.
        assert!(read.message.contains("/x.sock"));
        assert_eq!(
            WorkspaceCreationFailure::from_session_read("after", &missing).kind,
            "workspace.create_unconfirmed"
        );
        let malformed = SessionFetchError::Malformed("bad".into());
        assert_eq!(
            WorkspaceCreationFailure::from_session_read("after", &malformed).kind,
            "workspace.create_failed"
        );
        let lost = ControlFailure::Ambiguous("worktree.open result is unknown: timed out".into());
        assert_eq!(
            WorkspaceCreationFailure::from_owner(&lost).kind,
            "workspace.create_unconfirmed"
        );
        let refused =
            ControlFailure::Definite("worktree.open was refused: invalid_params: nope".into());
        assert_eq!(
            WorkspaceCreationFailure::from_owner(&refused).kind,
            "workspace.create_failed"
        );
    }

    #[test]
    fn a_registered_project_gets_its_checkouts_owner_not_a_plain_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let folder = root.join("notes");
        let repo = root.join("repo");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let folder = folder.to_string_lossy().into_owned();
        let repo = repo.to_string_lossy().into_owned();

        let paths = workspace::paths_here([folder.clone(), repo.clone()]);
        assert_eq!(
            registered_owner(&crate::node::test_node(), &paths, &folder, "Notes"),
            OwnerOpen::for_checkout(crate::node::TEST_NODE, &folder, &folder, false, "Notes")
                .on_node()
        );
        assert_eq!(
            registered_owner(&crate::node::test_node(), &paths, &repo, "Repo"),
            OwnerOpen::Worktree {
                path: repo.clone(),
                repository_root: repo.clone(),
                label: "Repo".to_owned(),
            }
        );
    }

    #[test]
    fn local_tab_control_opens_the_checkouts_bound_workspace_when_none_is_open() {
        let herdr = FakeHerdr::start("owner-open-worktree", |method, _| match method {
            "worktree.open" => json!({
                "type": "worktree_opened",
                "already_open": false,
                "workspace": {"workspace_id": "w9", "number": 9, "label": "repo-feat", "focused": true, "pane_count": 1, "tab_count": 1, "active_tab_id": "w9:t1", "agent_status": "idle"},
                "worktree": {"path": "/repo-feat", "is_bare": false, "is_detached": false, "is_prunable": false, "is_linked_worktree": true, "label": "repo-feat"},
                "tab": {"tab_id": "w9:t1", "workspace_id": "w9", "number": 1, "label": "1", "focused": true, "pane_count": 1, "agent_status": "idle"},
                "root_pane": {"pane_id": "w9:p1", "terminal_id": "fixture-terminal", "workspace_id": "w9", "tab_id": "w9:t1", "focused": true, "agent_status": "idle", "revision": 1}
            }),
            "tab.rename" => json!({
                "type": "tab_info",
                "tab": {"tab_id": "w9:t1", "workspace_id": "w9", "number": 1, "label": "Tab 2", "focused": true, "pane_count": 1, "agent_status": "idle"}
            }),
            "workspace.rename" => json!({
                "type": "workspace_info",
                "workspace": {"workspace_id": "w9", "number": 9, "label": "feat", "focused": true, "pane_count": 1, "tab_count": 1, "active_tab_id": "w9:t1", "agent_status": "idle"}
            }),
            other => panic!("unexpected {other}"),
        });

        let outcome = execute_remote_control(
            herdr.engine().as_ref(),
            &RemoteControlAction::OpenOwner {
                owner: OwnerOpen::Worktree {
                    path: "/repo-feat".to_owned(),
                    repository_root: "/repo".to_owned(),
                    label: "feat".to_owned(),
                },
                cwd: "/repo-feat".to_owned(),
                label: "Tab 2".to_owned(),
                area_id: None,
                group_id: None,
                admission_id: None,
            },
        )
        .expect("owner open");

        // A newly opened owner's first tab is the new tab.
        assert!(matches!(
            outcome,
            RemoteControlOutcome::Acknowledged {
                created_tab_id: Some(ref tab_id),
                created_pane_id: Some(ref pane_id),
            } if tab_id == "w9:t1" && pane_id == "w9:p1"
        ));
        // Herdr opens a linked worktree only from its repository's main
        // worktree, and a label on the open would rename a workspace that was
        // already there, so the new one is named after it opens (D-14).
        assert_eq!(
            herdr.calls(),
            [
                (
                    "worktree.open".to_owned(),
                    json!({"cwd": "/repo", "path": "/repo-feat", "focus": true})
                ),
                (
                    "workspace.rename".to_owned(),
                    json!({"workspace_id": "w9", "label": "feat"})
                ),
                (
                    "tab.rename".to_owned(),
                    json!({"tab_id": "w9:t1", "label": "Tab 2"})
                ),
            ]
        );
    }

    #[test]
    fn a_second_request_for_an_open_owner_adds_a_tab_there_instead_of_another_workspace() {
        for focus in [true, false] {
            let herdr = FakeHerdr::start("owner-already-open", |method, _| match method {
                "worktree.open" => json!({
                    "type": "worktree_opened",
                    "already_open": true,
                    "workspace": {"workspace_id": "w9", "number": 9, "label": "hide", "focused": true, "pane_count": 1, "tab_count": 1, "active_tab_id": "w9:t1", "agent_status": "idle"},
                    "worktree": {"path": "/repo", "is_bare": false, "is_detached": false, "is_prunable": false, "is_linked_worktree": false, "label": "hide"},
                    "tab": {"tab_id": "w9:t1", "workspace_id": "w9", "number": 1, "label": "1", "focused": true, "pane_count": 1, "agent_status": "idle"},
                    "root_pane": {"pane_id": "w9:p1", "terminal_id": "fixture-terminal", "workspace_id": "w9", "tab_id": "w9:t1", "focused": true, "agent_status": "idle", "revision": 1}
                }),
                "tab.create" => json!({
                    "type": "tab_created",
                    "tab": {"tab_id": "w9:t2", "workspace_id": "w9", "number": 2, "label": "Tab 2", "focused": true, "pane_count": 1, "agent_status": "idle"},
                    "root_pane": {"pane_id": "w9:p2", "terminal_id": "fixture-terminal-2", "workspace_id": "w9", "tab_id": "w9:t2", "focused": true, "agent_status": "idle", "revision": 1}
                }),
                other => panic!("unexpected {other}"),
            });

            let tab = open_owner_tab(
                herdr.engine().as_ref(),
                &OwnerOpen::Worktree {
                    path: "/repo".to_owned(),
                    repository_root: "/repo".to_owned(),
                    label: "hide".to_owned(),
                },
                "/repo",
                "Tab 2",
                focus,
                Default::default(),
                None,
            )
            .expect("owner tab");

            assert_eq!(
                tab,
                OwnedTab {
                    initial_label: None,
                    workspace_id: "w9".to_owned(),
                    tab_id: "w9:t2".to_owned(),
                    pane_id: "w9:p2".to_owned(),
                    launched: true,
                }
            );
            assert_eq!(herdr.methods(), ["worktree.open", "tab.create"]);
            assert_eq!(herdr.calls()[0].1["focus"], focus);
            assert_eq!(herdr.calls()[1].1["focus"], focus);
        }
    }

    #[test]
    fn a_refused_owner_open_fails_the_request_and_creates_nothing_elsewhere() {
        let herdr = FakeHerdr::start_with_errors("owner-refused", |method, _| match method {
            "worktree.open" => Err(("not_found".to_owned(), "no such checkout".to_owned())),
            other => panic!("unexpected {other}"),
        });

        let failure = open_owner_tab(
            herdr.engine().as_ref(),
            &OwnerOpen::Worktree {
                path: "/gone".to_owned(),
                repository_root: "/gone".to_owned(),
                label: "gone".to_owned(),
            },
            "/gone",
            "Tab 1",
            true,
            Default::default(),
            None,
        )
        .expect_err("a refused open fails");

        assert!(!failure.is_ambiguous());
        assert!(failure.message().contains("no such checkout"));
        assert_eq!(herdr.methods(), ["worktree.open"]);
    }

    #[test]
    fn a_plain_folder_reuses_its_marked_owner_and_marks_a_new_one_only_when_none_is_live() {
        for focus in [true, false] {
            let mark = crate::checkout_owner::owner_mark(crate::node::TEST_NODE, "/notes");
            let listed_mark = mark.clone();
            let marked = Arc::new(Mutex::new(false));
            let marked_in_server = marked.clone();
            let herdr = FakeHerdr::start("owner-folder", move |method, params| match method {
                "workspace.list" => {
                    let tokens = if *marked_in_server.lock().unwrap() {
                        json!({"hide_owner": listed_mark})
                    } else {
                        json!({})
                    };
                    json!({"type": "workspace_list", "workspaces": [
                        {"workspace_id": "w1", "number": 1, "label": "other", "focused": false, "pane_count": 1, "tab_count": 1, "active_tab_id": "w1:t1", "agent_status": "idle"},
                        {"workspace_id": "w4", "number": 4, "label": "notes", "focused": false, "pane_count": 1, "tab_count": 1, "active_tab_id": "w4:t1", "agent_status": "idle", "tokens": tokens}
                    ]})
                }
                "workspace.create" => json!({
                    "type": "workspace_created",
                    "workspace": {"workspace_id": "w4", "number": 4, "label": "notes", "focused": true, "pane_count": 1, "tab_count": 1, "active_tab_id": "w4:t1", "agent_status": "idle"},
                    "tab": {"tab_id": "w4:t1", "workspace_id": "w4", "number": 1, "label": "1", "focused": true, "pane_count": 1, "agent_status": "idle"},
                    "root_pane": {"pane_id": "w4:p1", "terminal_id": "fixture-terminal", "workspace_id": "w4", "tab_id": "w4:t1", "focused": true, "agent_status": "idle", "revision": 1}
                }),
                "workspace.report_metadata" => {
                    assert_eq!(params["tokens"]["hide_owner"], json!(listed_mark));
                    *marked_in_server.lock().unwrap() = true;
                    json!({"type": "ok"})
                }
                "tab.rename" => json!({
                    "type": "tab_info",
                    "tab": {"tab_id": "w4:t1", "workspace_id": "w4", "number": 1, "label": "Tab 1", "focused": true, "pane_count": 1, "agent_status": "idle"}
                }),
                "tab.create" => json!({
                    "type": "tab_created",
                    "tab": {"tab_id": "w4:t2", "workspace_id": "w4", "number": 2, "label": "Tab 2", "focused": true, "pane_count": 1, "agent_status": "idle"},
                    "root_pane": {"pane_id": "w4:p2", "terminal_id": "fixture-terminal-2", "workspace_id": "w4", "tab_id": "w4:t2", "focused": true, "agent_status": "idle", "revision": 1}
                }),
                other => panic!("unexpected {other}"),
            });
            let owner = OwnerOpen::Folder {
                path: "/notes".to_owned(),
                label: "notes".to_owned(),
                mark,
                legacy_mark: None,
            };

            let first = open_owner_tab(
                herdr.engine().as_ref(),
                &owner,
                "/notes",
                "Tab 1",
                focus,
                Default::default(),
                None,
            )
            .expect("first tab");
            let second = open_owner_tab(
                herdr.engine().as_ref(),
                &owner,
                "/notes",
                "Tab 2",
                focus,
                Default::default(),
                None,
            )
            .expect("second tab");

            assert_eq!(
                (first.workspace_id.as_str(), first.tab_id.as_str()),
                ("w4", "w4:t1")
            );
            assert_eq!(
                (second.workspace_id.as_str(), second.tab_id.as_str()),
                ("w4", "w4:t2")
            );
            assert!(*marked.lock().unwrap());
            assert_eq!(herdr.calls()[1].1["focus"], focus);
            assert_eq!(herdr.calls()[5].1["focus"], focus);
            assert_eq!(
                herdr.methods(),
                [
                    "workspace.list",
                    "workspace.create",
                    "workspace.report_metadata",
                    "tab.rename",
                    "workspace.list",
                    "tab.create"
                ]
            );
        }
    }
}
