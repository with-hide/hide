use super::*;
use hide_node_link::terminal::{GridSize, PaneTerminalState, TerminalControl, TerminalReport};
use std::time::Duration;

use crate::fake_herdr::FakeHerdr;

mod agent_areas;
#[path = "tests/agent_connection.rs"]
mod agent_connection;
#[path = "tests/agent_features.rs"]
mod agent_features;
#[path = "tests/agent_scopes.rs"]
mod agent_scopes;
#[path = "tests/agent_sleep.rs"]
mod agent_sleep;
#[path = "tests/agent_update.rs"]
mod agent_update;
#[path = "tests/agents_settings_remote.rs"]
mod agents_settings_remote;
#[path = "tests/appearance.rs"]
mod appearance;
#[path = "tests/backlog.rs"]
mod backlog;
#[path = "tests/beside_spawn.rs"]
mod beside_spawn;
#[path = "tests/birth_cwd.rs"]
mod birth_cwd;
mod checkout_git;
#[path = "tests/control_order.rs"]
mod control_order;
#[path = "tests/detail.rs"]
mod detail;
#[path = "tests/device_catalog.rs"]
mod device_catalog;
#[path = "tests/device_kit.rs"]
mod device_kit;
#[path = "tests/device_worktrees.rs"]
mod device_worktrees;
#[path = "tests/devices.rs"]
mod devices;
#[path = "tests/disk_sweep.rs"]
mod disk_sweep;
#[path = "tests/documents.rs"]
mod documents;
#[path = "tests/editor_preview.rs"]
mod editor_preview;
#[path = "tests/editor_reopen.rs"]
mod editor_reopen;
mod fetch;
#[path = "tests/first_run.rs"]
mod first_run;
#[path = "tests/fork_registration.rs"]
mod fork_registration;
#[path = "tests/github_reads.rs"]
mod github_reads;
#[path = "../worktree_control/group_start_tests.rs"]
mod group_start_admission;
mod group_tabs;
mod group_tabs_remote;
mod guardrails;
#[path = "tests/herdr_takeover.rs"]
mod herdr_takeover;
#[path = "tests/home.rs"]
mod home;
#[path = "tests/inbound.rs"]
mod inbound;
#[path = "tests/issues.rs"]
mod issues;
#[path = "tests/key_routing.rs"]
mod key_routing;
#[path = "tests/labels.rs"]
mod labels;
#[path = "tests/launch_choice.rs"]
mod launch_choice;
#[path = "tests/lineage.rs"]
mod lineage;
#[path = "tests/links.rs"]
mod links;
#[path = "tests/memory.rs"]
mod memory;
#[path = "tests/missions.rs"]
mod missions;
#[path = "tests/named_tab.rs"]
mod named_tab;
#[path = "tests/node_agent_areas.rs"]
mod node_agent_areas;
#[path = "tests/node_issues.rs"]
mod node_issues;
#[path = "tests/observed_start.rs"]
mod observed_start;
#[path = "tests/operator_focus.rs"]
mod operator_focus;
mod overview_graph;
mod pane_geometry;
pub(super) mod pane_move;
mod plans;
#[path = "tests/project_settings.rs"]
mod project_settings;
#[path = "tests/projects.rs"]
mod projects;
mod ps;
#[path = "tests/pull_requests.rs"]
mod pull_requests;
#[path = "tests/recent_checkouts.rs"]
mod recent_checkouts;
#[path = "tests/recent_panes.rs"]
mod recent_panes;
#[path = "tests/remote_spawn.rs"]
mod remote_spawn;
#[path = "tests/repository_clone.rs"]
mod repository_clone;
#[path = "tests/session_archive.rs"]
mod session_archive;
#[path = "tests/session_navigation.rs"]
mod session_navigation;
#[path = "tests/session_state.rs"]
mod session_state;
#[path = "tests/shortcut_import.rs"]
mod shortcut_import;
#[path = "tests/snapshot_delta.rs"]
mod snapshot_delta;
#[cfg(unix)]
#[path = "tests/ssh_hosts.rs"]
mod ssh_hosts_list;
mod start_geometry;
mod suggestions;
#[path = "tests/terminal.rs"]
mod terminal;
#[path = "tests/tree_close.rs"]
mod tree_close;
#[path = "tests/tree_reopen.rs"]
mod tree_reopen;
mod ui_state_focus;
#[path = "tests/usage_limit.rs"]
mod usage_limit;
#[path = "tests/view_areas.rs"]
mod view_areas;
mod view_bookmarks;
#[path = "tests/workspace_control.rs"]
mod workspace_control;
#[path = "tests/workspace_view.rs"]
mod workspace_view;
mod wsl_devices;

/// The `tab_list` Herdr answers a `tab.move` with, carrying the tabs in
/// their new order. Only the order is read here; the other fields are
/// what the pinned schema requires of a tab.
fn tab_list(ids: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "type": "tab_list",
        "tabs": ids.iter().enumerate().map(|(index, tab_id)| serde_json::json!({
            "tab_id": tab_id,
            "workspace_id": tab_id.split(':').next().unwrap_or_default(),
            "number": index + 1,
            "label": "fixture",
            "focused": false,
            "pane_count": 1,
            "agent_status": "idle"
        })).collect::<Vec<_>>()
    })
}

fn no_worktrees() -> crate::model::WorktreeCatalogSnapshot {
    crate::model::WorktreeCatalogSnapshot::default()
}

/// A settled worktree with nothing in the way, ready for the Remove
/// button's rules to be applied to it.
fn settled_worktree(badge: crate::model::PullRequestBadge) -> CheckoutSnapshot {
    let pull_request = crate::model::PullRequestSnapshot {
        closing_issues: Default::default(),
        title: "Fixture pull request".into(),
        checks: crate::model::PullRequestChecks::Unknown,
        number: 7,
        head_branch: "feature".to_owned(),
        base_branch: "main".to_owned(),
        url: "https://example.invalid/pull/7".to_owned(),
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
    };
    CheckoutSnapshot {
        agent_scope: Default::default(),
        id: "checkout-feature".to_owned(),
        workspace_id: "workspace-1".to_owned(),
        label: "feature".to_owned(),
        path: "/tmp/hide/feature".to_owned(),
        branch: Some("feature".to_owned()),
        is_worktree: true,
        exists: true,
        pull_request: Some(pull_request.clone()),
        worktree: Some(crate::model::WorktreeSnapshot {
            path: "/tmp/hide/feature".to_owned(),
            branch: Some("feature".to_owned()),
            pull_request: Some(pull_request),
            deletion_gate: crate::model::WorktreeDeletionGateSnapshot {
                can_delete_branch: true,
                ..crate::model::WorktreeDeletionGateSnapshot::default()
            },
            ..crate::model::WorktreeSnapshot::default()
        }),
        ..CheckoutSnapshot::default()
    }
}

fn card_for(
    runtime: &mut Runtime,
    checkout: CheckoutSnapshot,
) -> crate::model::CheckoutCardSnapshot {
    let checkout_id = checkout.id.clone();
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "workspace-1",
        "hide",
        "/tmp/hide",
        vec![checkout],
    )];
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id);
    runtime.refresh_card();
    runtime.snapshot.card.clone()
}

use crate::live::SessionFetchError;
use crate::model::{
    CheckoutSnapshot, DeviceSnapshot, PaneLayoutNodeSnapshot, PaneLayoutSnapshot, PaneSnapshot,
    RemoteSessionSnapshot, RemoteStatusSnapshot, TabSnapshot, TerminalPaneSnapshot,
    WorkspaceRegistration, WorkspaceSnapshot,
};
use crate::model::{MAX_PANE_TEXT_SCALE, MIN_PANE_TEXT_SCALE};
use crate::sidebar::SessionSnapshotPayload;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_RUNTIME_STATE_ID: AtomicU64 = AtomicU64::new(0);

/// Waits for the core's workers to bring `shared` to a state `ready` accepts;
/// a timeout names `what` with the operation and the last error.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait(shared: &Arc<Mutex<Runtime>>, what: &str, ready: impl Fn(&Runtime) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !ready(&shared.lock().unwrap()) {
        if std::time::Instant::now() >= deadline {
            let runtime = shared.lock().unwrap();
            panic!(
                "timed out waiting for {what}: {:?} {:?}",
                runtime.snapshot.task_operation, runtime.snapshot.status.last_error
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Waits for `ready`, for a state outside the runtime (a file, a fake).
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
pub(super) fn wait_for(what: &str, ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Every node's terminals as a runtime under test reaches them: each control
/// is recorded and nothing answers, so a test reports what a node would.
#[derive(Default)]
pub(super) struct RecordedTerminals {
    controls: Mutex<Vec<TerminalControl>>,
    devices: Mutex<Vec<String>>,
}

impl RecordedTerminals {
    /// The controls sent since the last take.
    pub(super) fn take(&self) -> Vec<TerminalControl> {
        std::mem::take(&mut *self.controls.lock().unwrap())
    }

    /// The panes asked to attach since the last take, and whether each was
    /// the operator's Reconnect.
    pub(super) fn attaches(&self) -> Vec<(String, bool)> {
        self.take()
            .into_iter()
            .filter_map(|control| match control {
                TerminalControl::Attach { pane, manual, .. } => Some((pane, manual)),
                _ => None,
            })
            .collect()
    }

    /// `install:<device>` and `remove:<device>`, in order.
    pub(super) fn devices(&self) -> Vec<String> {
        self.devices.lock().unwrap().clone()
    }
}

impl hide_node_link::terminal::TerminalNode for RecordedTerminals {
    fn control(&self, control: TerminalControl) {
        self.controls.lock().unwrap().push(control);
    }

    fn key(&self, _: hide_node_link::terminal::KeyTarget, _: Vec<u8>, _: u64) {
        unreachable!("keys reach the node beside the screen, never through the core");
    }

    fn view(&self, _: &str, _: GridSize, _: bool) {
        unreachable!("views reach the node beside the screen, never through the core");
    }

    fn redraw(&self, _: &str) {
        unreachable!("redraws reach the node beside the screen, never through the core");
    }
}

impl hide_node_link::terminal::TerminalRoutes for RecordedTerminals {
    fn install_device(&self, device: &str, _: Arc<dyn hide_node_link::terminal::TerminalNode>) {
        self.devices
            .lock()
            .unwrap()
            .push(format!("install:{device}"));
    }

    fn remove_device(&self, device: &str) {
        self.devices
            .lock()
            .unwrap()
            .push(format!("remove:{device}"));
    }

    fn terminals_unstarted(&self, device: &str, _: &str) {
        self.devices
            .lock()
            .unwrap()
            .push(format!("unstarted:{device}"));
    }
}

/// Gives `runtime` recorded terminal routes.
pub(super) fn record_terminals(runtime: &mut Runtime) -> Arc<RecordedTerminals> {
    let terminals = Arc::new(RecordedTerminals::default());
    runtime.set_terminals(Arc::clone(&terminals) as _);
    terminals
}

/// A pane's state as its node reports it.
pub(super) fn terminal_state(state: &str, generation: u64) -> PaneTerminalState {
    PaneTerminalState {
        state: state.to_owned(),
        mode: match state {
            "controlling" => Some("control".to_owned()),
            "observing" => Some("observe".to_owned()),
            _ => None,
        },
        generation,
        attempt: 1,
        message: None,
        exit_category: None,
        retry_decision: "none".to_owned(),
        last_attempt_at_unix_ms: None,
    }
}

/// What a pane's node reports for keys the operator typed into it on the
/// screen; whether they submitted is the node's to decide
/// (`hide-node` `terminal::input::submits`).
pub(super) fn typed_into(runtime: &mut Runtime, pane: &str, submitted: bool) -> bool {
    runtime.ingest_terminal_reports(vec![TerminalReport::Input {
        pane: pane.to_owned(),
        at_unix_ms: unix_milliseconds(),
        submitted,
        focus: true,
    }])
}

/// Reports `state` for `pane` the way its node would.
pub(super) fn report_terminal(runtime: &mut Runtime, pane: &str, state: PaneTerminalState) -> bool {
    runtime.ingest_terminal_reports(vec![TerminalReport::State {
        pane: pane.to_owned(),
        state,
    }])
}

pub(super) fn context_payload() -> SessionSnapshotPayload {
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [
            {"pane_id":"w1:p1", "agent":"codex", "agent_status":"working", "state_change_seq":10,
             "cwd":"/tmp/hide-context-alpha", "tokens":{"activity":"1788871000000"}},
            {"pane_id":"w2:p1", "agent":"codex", "agent_status":"working", "state_change_seq":20,
             "cwd":"/tmp/hide-context-zeta", "tokens":{"activity":"1788872000000"},
             "agent_session":{"kind":"id", "value":"context-session"}, "spawned_from_pane_id":"w1:p1"}
        ],
        "workspaces":[{"workspace_id":"w1","label":"Alpha"},{"workspace_id":"w2","label":"Zeta"}],
        "tabs":[{"workspace_id":"w1","tab_id":"w1:t1","label":"1"},{"workspace_id":"w2","tab_id":"w2:t1","label":"1"}],
        "panes":[{"pane_id":"w1:p1","cwd":"/tmp/hide-context-alpha"},{"pane_id":"w2:p1","cwd":"/tmp/hide-context-zeta"}],
        "layouts":[
            {"workspace_id":"w1","tab_id":"w1:t1","zoomed":false,"area":{"x":0,"y":0,"width":80,"height":24},
             "focused_pane_id":"w1:p1","panes":[{"pane_id":"w1:p1","rect":{"x":0,"y":0,"width":80,"height":24}}],"splits":[]},
            {"workspace_id":"w2","tab_id":"w2:t1","zoomed":false,"area":{"x":0,"y":0,"width":80,"height":24},
             "focused_pane_id":"w2:p1","panes":[{"pane_id":"w2:p1","rect":{"x":0,"y":0,"width":80,"height":24}}],"splits":[]}
        ]
    })).unwrap()
}

/// A new folder under the system temp folder, removed when it is dropped.
///
/// It is never named by the pid: nextest starts every test in a process of
/// its own, so a name made of the pid and a counter of that process is one an
/// earlier test process may have left a file under, and a runtime started
/// there loads that state instead of the defaults the test assumes.
pub(super) fn scratch_dir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("a new scratch folder")
}

/// Hands the runtime's folders to the test, which keeps them past the
/// runtime: a restart test drops the runtime and starts another on its files.
pub(super) fn hold_dirs(runtime: &mut Runtime) -> Vec<tempfile::TempDir> {
    std::mem::take(&mut runtime.test_dirs)
}

/// A runtime shared with the workers it starts, as hided shares it.
///
/// The runtime owns the folders made for it, so they go when it is dropped,
/// and a shared runtime is dropped by whichever holder lets go last. A worker
/// keeps its hold for a moment after its last step; when that outlasted the
/// test, the test process exited before the worker let go, the runtime was
/// never dropped, and its folders stayed in the temp folder. Dropping this
/// takes the runtime back on the test's own thread once every worker has let
/// go (a worker only upgrades its weak reference, which fails after that), so
/// the folders go with the test.
pub(super) struct SharedRuntime(Option<Arc<Mutex<Runtime>>>);

impl SharedRuntime {
    pub(super) fn new(runtime: Runtime) -> Self {
        Self(Some(Arc::new(Mutex::new(runtime))))
    }

    pub(super) fn weak(&self) -> std::sync::Weak<Mutex<Runtime>> {
        Arc::downgrade(&**self)
    }
}

impl std::ops::Deref for SharedRuntime {
    type Target = Arc<Mutex<Runtime>>;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("the runtime is shared until drop")
    }
}

impl Drop for SharedRuntime {
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn drop(&mut self) {
        let Some(mut shared) = self.0.take() else {
            return;
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            match Arc::try_unwrap(shared) {
                Ok(runtime) => {
                    drop(runtime);
                    return;
                }
                Err(held) if std::time::Instant::now() < deadline => {
                    shared = held;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) if std::thread::panicking() => return,
                Err(_) => panic!(
                    "a worker still held the runtime 10 s after the test ended, so its folders would outlive the test"
                ),
            }
        }
    }
}

pub(super) fn runtime() -> Runtime {
    let state = scratch_dir("herdr-core-runtime-");
    let options = CoreOptions {
        schema_version: SCHEMA_VERSION,
        home: None,
        node_id: crate::node::test_node(),
        herdr_socket_path: Some("/tmp/herdr-core-pet-runtime.sock".to_owned()),
        herdr_bin_path: None,
        app_state_path: state
            .path()
            .join("state.json")
            .to_string_lossy()
            .into_owned(),
        host_helper_root: None,
        host_cli_dir: None,
        workspace_views_path: None,
        shortcut_import_path: None,
        local_issues_path: None,
        effects_held: false,
        machine_name: None,
        account_state_folder: false,
    };
    let mut runtime = Runtime::new(
        options,
        environment::EnvironmentReport {
            statuses: Vec::new(),
            home_path: None,
            codex_home: None,
        },
        std::sync::Arc::new(hide_node::Local::of_process()),
        crate::node::test_devices(),
    );
    runtime.test_dirs.push(state);
    runtime
}

/// A runtime with a live context pointed at a socket that does not exist.
///
/// Pane focus needs a live connection to be dispatched at all, and the
/// worker it spawns fails on its own without touching this runtime, so a
/// test can drive the real focus event rather than a shortcut into the
/// read record.
pub(super) fn live_runtime() -> Runtime {
    let mut runtime = runtime();
    let socket_path = std::env::temp_dir()
        .join(format!(
            "herdr-core-read-record-{}-{}.sock",
            std::process::id(),
            NEXT_RUNTIME_STATE_ID.fetch_add(1, Ordering::Relaxed)
        ))
        .to_string_lossy()
        .into_owned();
    runtime.live = Some(live::LiveContext {
        socket_path: socket_path.clone().into(),
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(hide_herdr_client::LocalSocketConnector::new(&socket_path)),
        node: Arc::new(hide_node::Local::of_process()),
    });
    runtime
}

/// The event the shell sends when the operator clicks an agent row, a
/// pane, or picks one from the switcher.
fn operator_focus_event(pane_id: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "focus_pane",
        "payload": {"pane_id": pane_id, "origin": "operator"}
    }))
    .expect("focus pane event")
}

/// The event the shell sends once on launch to put the terminal back on
/// the pane the last session ended on.
fn restore_focus_event(pane_id: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "focus_pane",
        "payload": {"pane_id": pane_id, "origin": "restore"}
    }))
    .expect("restore focus event")
}

/// The panes the sidebar still shows as unread, in snapshot order.
fn unread_panes(runtime: &Runtime) -> Vec<String> {
    let mut panes = runtime
        .snapshot
        .navigator
        .agents
        .iter()
        .filter(|agent| agent.unread)
        .map(|agent| agent.pane_id.clone())
        .collect::<Vec<_>>();
    panes.sort();
    panes
}

/// Three finished panes side by side in one tab, the shape the operator
/// reported. `focused` is the pane Herdr names as that tab's focus, which
/// is a tab-scoped verdict Hide's read axis must not follow.
fn finished_tab_payload(panes: &[(&str, u64)], focused: &str) -> SessionSnapshotPayload {
    let agents = panes
        .iter()
        .map(|(pane_id, seq)| {
            serde_json::json!({
                "pane_id": pane_id,
                "workspace_label": "Fixture",
                "agent": "codex",
                "agent_status": "done",
                "state_change_seq": seq,
                "tokens": {"status_done_new": "\u{25cf}", "activity": "0000000000001"}
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(panes.len(), 3, "the fixture is three panes side by side");
    let layout_panes = panes
        .iter()
        .enumerate()
        .map(|(index, (pane_id, _))| {
            serde_json::json!({
                "pane_id": pane_id,
                "rect": {"x": index * 30, "y": 0, "width": 30, "height": 24}
            })
        })
        .collect::<Vec<_>>();
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": agents,
        "focused_workspace_id": "w1",
        "focused_pane_id": focused,
        "workspaces": [{"workspace_id": "w1", "label": "fixture", "active_tab_id": "t1"}],
        "tabs": [{"workspace_id": "w1", "tab_id": "t1", "label": ""}],
        "layouts": [{
            "workspace_id": "w1", "tab_id": "t1", "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 90, "height": 24},
            "focused_pane_id": focused,
            "panes": layout_panes,
            "splits": [
                {"direction": "right", "ratio": 0.333_333_34,
                 "rect": {"x": 0, "y": 0, "width": 90, "height": 24}},
                {"direction": "right", "ratio": 0.5,
                 "rect": {"x": 30, "y": 0, "width": 60, "height": 24}}
            ]
        }]
    }))
    .expect("session payload")
}

fn working_payload() -> SessionSnapshotPayload {
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [{
            "pane_id": "w1:p1",
            "workspace_label": "Fixture",
            "agent": "codex",
            "agent_status": "working",
            "tokens": {"status_working": "\u{25cf}", "activity": "0000000000001"}
        }],
        "tabs": [{"workspace_id": "w1", "tab_id": "t1", "label": ""}],
        "layouts": [{
            "workspace_id": "w1", "tab_id": "t1", "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": "w1:p1",
            "panes": [{"pane_id": "w1:p1",
                       "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        }]
    }))
    .expect("session payload")
}

fn pane(id: &str, cwd: &str) -> PaneSnapshot {
    PaneSnapshot {
        start_claim: None,
        terminal_id: None,
        failed_terminal_id: None,
        id: id.to_owned(),
        herdr_label: None,
        terminal_title: None,
        cwd: cwd.to_owned(),
        status_code: crate::model::AgentStatusCode::Attached,
        requires_close_confirmation: false,
        requires_close_status_check: false,
        identity_label: None,
        activity_at_unix_ms: None,
        fork: PaneForkSnapshot::default(),
        ports: Vec::new(),
        servers: Vec::new(),
        children: None,
        lineage_path: Vec::new(),
        sleep: None,
        sleep_action: None,
    }
}

fn tab(workspace_id: &str, checkout_id: &str, pane: Option<PaneSnapshot>) -> TabSnapshot {
    TabSnapshot {
        agent: None,
        naming: crate::model::TabNaming {
            focused_pane_id: pane
                .as_ref()
                .map(|pane| pane.id.clone())
                .unwrap_or_default(),
            number: 1,
            raw: "Session".into(),
            automatic: "Tab 1".into(),
            ..Default::default()
        },
        id: Some(format!("{checkout_id}:tab")),
        workspace_id: Some(workspace_id.to_owned()),
        checkout_id: Some(checkout_id.to_owned()),
        label: Some("Session".to_owned()),
        empty: pane.is_none(),
        delegated: false,
        panes: pane.into_iter().collect(),
    }
}

fn checkout(
    workspace_id: &str,
    checkout_id: &str,
    path: &str,
    pane: Option<PaneSnapshot>,
) -> CheckoutSnapshot {
    CheckoutSnapshot {
        agent_scope: Default::default(),
        next_tab_label: crate::model::next_tab_label(std::iter::empty()),
        id: checkout_id.to_owned(),
        workspace_id: workspace_id.to_owned(),
        label: checkout_id.to_owned(),
        path: path.to_owned(),
        branch: None,
        is_worktree: false,
        exists: true,
        temporary: false,
        has_panes: pane.is_some(),
        tabs: pane
            .clone()
            .map(|pane| vec![tab(workspace_id, checkout_id, Some(pane))])
            .unwrap_or_default(),
        active_tab_id: pane.map(|_| format!("{checkout_id}:tab")),
        strip: Vec::new(),
        ..CheckoutSnapshot::default()
    }
}

fn workspace(
    id: &str,
    label: &str,
    path: &str,
    checkouts: Vec<CheckoutSnapshot>,
) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        agent_presence: Default::default(),
        agent_scope: Default::default(),
        home_issues: Default::default(),
        pull_requests: Vec::new(),
        tasks: Default::default(),
        id: id.to_owned(),
        label: label.to_owned(),
        path: path.to_owned(),
        remote_target_id: None,
        expanded: true,
        device_id: crate::node::TEST_NODE.to_owned(),
        repo_name: label.to_owned(),
        is_git: false,
        default_branch: None,
        branches: Vec::new(),
        registered: true,
        temporary: false,
        session_workspace_ids: Vec::new(),
        last_activity_unix_ms: None,
        checkouts,
        pinned: false,
        is_home: false,
        inactive_checkouts: Default::default(),
        session_folds: Default::default(),
        removal: Default::default(),
        disk: Default::default(),
        cleanup: None,
    }
}

/// One Herdr workspace whose tabs each hold one pane inside
/// `checkout_path`. `tab_order` is the order Herdr reports its tabs in and
/// `layout_order` is the order the layouts arrive in, so a test can hand
/// the two orders apart and see which one the navigator follows.
fn tab_order_payload(
    checkout_path: &str,
    tab_order: &[&str],
    layout_order: &[&str],
    active_tab_id: &str,
) -> SessionSnapshotPayload {
    let tabs = tab_order
        .iter()
        .map(|tab_id| {
            serde_json::json!({
                "workspace_id": "w-order", "tab_id": tab_id, "label": "",
                "number": tab_id.rsplit(":t").next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(1)
            })
        })
        .collect::<Vec<_>>();
    let panes = layout_order
        .iter()
        .map(|tab_id| serde_json::json!({"pane_id": format!("{tab_id}:p"), "cwd": checkout_path}))
        .collect::<Vec<_>>();
    let layouts = layout_order
        .iter()
        .map(|tab_id| {
            serde_json::json!({
                "workspace_id": "w-order",
                "tab_id": tab_id,
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": format!("{tab_id}:p"),
                "panes": [{
                    "pane_id": format!("{tab_id}:p"),
                    "rect": {"x": 0, "y": 0, "width": 80, "height": 24}
                }],
                "splits": []
            })
        })
        .collect::<Vec<_>>();
    // Herdr's keyboard is in the workspace, on the active tab's pane,
    // which is what a live `session.snapshot` reports.
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "focused_workspace_id": "w-order",
        "focused_pane_id": format!("{active_tab_id}:p"),
        "workspaces": [{
            "workspace_id": "w-order",
            "label": "order",
            "active_tab_id": active_tab_id
        }],
        "tabs": tabs,
        "panes": panes,
        "layouts": layouts
    }))
    .expect("ordered session payload")
}

/// Two or more Herdr workspaces whose tabs all sit in `checkout_path`, so
/// the path-keyed navigator folds them into one checkout. Each entry is
/// `(workspace_id, tab_ids, active_tab_id)`; `focused_workspace_id` is
/// the one holding Herdr's keyboard, on its active tab's pane. Panes are
/// named `<tab>:p`.
fn split_checkout_payload(
    checkout_path: &str,
    workspaces: &[(&str, &[&str], &str)],
    focused_workspace_id: &str,
) -> SessionSnapshotPayload {
    let mut sessions = Vec::new();
    let mut tabs = Vec::new();
    let mut panes = Vec::new();
    let mut layouts = Vec::new();
    for (workspace_id, tab_ids, active_tab_id) in workspaces {
        sessions.push(serde_json::json!({
            "workspace_id": workspace_id,
            "label": workspace_id,
            "active_tab_id": active_tab_id
        }));
        for tab_id in tab_ids.iter() {
            tabs.push(serde_json::json!({
                "workspace_id": workspace_id, "tab_id": tab_id, "label": ""
            }));
            panes.push(serde_json::json!({
                "pane_id": format!("{tab_id}:p"), "cwd": checkout_path
            }));
            layouts.push(serde_json::json!({
                "workspace_id": workspace_id,
                "tab_id": tab_id,
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": format!("{tab_id}:p"),
                "panes": [{
                    "pane_id": format!("{tab_id}:p"),
                    "rect": {"x": 0, "y": 0, "width": 80, "height": 24}
                }],
                "splits": []
            }));
        }
    }
    let focused_pane_id = workspaces
        .iter()
        .find(|(workspace_id, _, _)| *workspace_id == focused_workspace_id)
        .map(|(_, _, active_tab_id)| format!("{active_tab_id}:p"))
        .expect("the focused workspace is one of the listed workspaces");
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "focused_workspace_id": focused_workspace_id,
        "focused_pane_id": focused_pane_id,
        "workspaces": sessions,
        "tabs": tabs,
        "panes": panes,
        "layouts": layouts
    }))
    .expect("split checkout session payload")
}

/// A runtime with one registered checkout focused, ready to ingest
/// [`tab_order_payload`]. Returns the checkout id the navigator gave it.
fn tab_order_runtime(checkout_path: &str) -> (Runtime, String) {
    let mut runtime = runtime();
    runtime.snapshot.ui_state.workspace_registrations = vec![WorkspaceRegistration {
        primary_checkout_id: None,
        id: "workspace:order".to_owned(),
        label: "order".to_owned(),
        path: checkout_path.to_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    }];
    runtime.rebuild_catalog();
    let checkout_id = workspace::checkout_id_for_path("workspace:order", Path::new(checkout_path));
    runtime.snapshot.navigator.focused_workspace_id = Some("workspace:order".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
    // A selection the operator made, so the first live session keeps it
    // instead of retiring it with the restore hint.
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.navigator.root_path = Some(checkout_path.to_owned());
    runtime.reset_terminal_projection(None);
    (runtime, checkout_id)
}

fn ordered_tab_ids(runtime: &Runtime, checkout_id: &str) -> Vec<String> {
    runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .find(|checkout| checkout.id == checkout_id)
        .expect("the registered checkout")
        .tabs
        .iter()
        .map(|tab| tab.id.clone().expect("a Herdr tab always has an id"))
        .collect()
}

/// The panes the terminal projection is holding open, in snapshot order.
fn projected_pane_ids(runtime: &Runtime) -> Vec<String> {
    runtime
        .snapshot()
        .terminal
        .panes
        .iter()
        .map(|pane| pane.pane_id.clone())
        .collect()
}

fn checkout_active_tab_id(runtime: &Runtime, checkout_id: &str) -> Option<String> {
    runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .find(|checkout| checkout.id == checkout_id)
        .expect("the registered checkout")
        .active_tab_id
        .clone()
}

/// A [`tab_order_runtime`] with a live Herdr context, so a view-state
/// notification actually leaves and a wait is armed.
fn live_tab_order_runtime(checkout_path: &str) -> (Runtime, String) {
    let (mut runtime, checkout_id) = tab_order_runtime(checkout_path);
    let socket_path = std::env::temp_dir()
        .join(format!(
            "herdr-core-view-authority-{}-{}.sock",
            std::process::id(),
            NEXT_RUNTIME_STATE_ID.fetch_add(1, Ordering::Relaxed)
        ))
        .to_string_lossy()
        .into_owned();
    runtime.live = Some(live::LiveContext {
        socket_path: socket_path.clone().into(),
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(hide_herdr_client::LocalSocketConnector::new(&socket_path)),
        node: Arc::new(hide_node::Local::of_process()),
    });
    (runtime, checkout_id)
}

fn focus_tab_event(checkout_id: &str, tab_id: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "focus_tab",
        "payload": {
            "workspace_id": "workspace:order",
            "checkout_id": checkout_id,
            "tab_id": tab_id
        }
    }))
    .expect("focus tab event")
}

fn correlated_pane_focus_event(pane_id: &str, request_id: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "focus_pane",
        "payload": {
            "pane_id": pane_id,
            "origin": "operator",
            "request_id": request_id
        }
    }))
    .expect("correlated pane focus event")
}

/// Herdr's event stream as the replica publishes it: each session carries
/// the tab moves Herdr has made, so a test states which moves a session
/// comes after instead of leaving the core to infer them from its state.
struct HerdrMoves(crate::sidebar::SessionTabMoves);

impl HerdrMoves {
    fn new() -> Self {
        Self(crate::sidebar::SessionTabMoves::new(1))
    }

    /// The stream of a new replica, built after a reconnect: its count
    /// starts again.
    fn reconnected(&self) -> Self {
        Self(crate::sidebar::SessionTabMoves::new(self.0.generation + 1))
    }

    /// `payload` as the session published after Herdr moved to each of
    /// `moves`, in order. Like the replica, it names the last move as the
    /// session's focus event unless the test set one.
    fn after(
        &mut self,
        moves: &[&str],
        mut payload: SessionSnapshotPayload,
    ) -> SessionSnapshotPayload {
        for tab_id in moves {
            self.0.record((*tab_id).to_owned());
        }
        if let Some(last) = moves.last()
            && payload.tab_focus.is_none()
        {
            let workspace_id = payload
                .tabs
                .iter()
                .find(|tab| tab.tab_id == *last)
                .map(|tab| tab.workspace_id.clone())
                .expect("a move to a tab the session lists");
            payload.tab_focus = Some(crate::sidebar::SessionTabFocus {
                generation: self.0.generation,
                workspace_id,
                tab_id: (*last).to_owned(),
                revision: self.0.applied,
                creation: false,
            });
        }
        payload.tab_moves = Some(self.0.clone());
        payload
    }
}

/// The tabs of the moves sent to Herdr and not yet answered, oldest first.
fn sent_tab_moves(runtime: &Runtime) -> Vec<String> {
    runtime
        .tab_focus_requests
        .iter()
        .map(|request| request.tab_id.clone())
        .collect()
}

fn newest_sent_at(runtime: &Runtime) -> u64 {
    runtime
        .tab_focus_requests
        .last()
        .expect("a tab move is in flight")
        .requested_at_unix_ms
}

fn diagnostic_count(runtime: &Runtime, kind: &str) -> usize {
    runtime
        .snapshot()
        .status
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == kind)
        .count()
}

fn strip_ids(runtime: &Runtime, checkout_id: &str) -> Vec<String> {
    runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .find(|checkout| checkout.id == checkout_id)
        .expect("the registered checkout")
        .strip
        .iter()
        .map(|entry| entry.id.clone())
        .collect()
}

fn strip_labels(runtime: &Runtime, checkout_id: &str) -> Vec<String> {
    runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .find(|checkout| checkout.id == checkout_id)
        .expect("the registered checkout")
        .strip
        .iter()
        .map(|entry| entry.label.clone())
        .collect()
}

/// A registered checkout the strip tests can drive.
///
/// The temp root is a symlink on macOS and the catalog keys checkouts by
/// the real path, so the fixture uses that. It is also made its own
/// repository, because a directory inside another repository is
/// catalogued under that repository's root instead. The checkout is a
/// folder inside the scratch folder rather than the scratch folder itself,
/// so a second checkout, a worktree or a link a test makes beside it is
/// inside the scratch folder too and goes with it.
fn strip_checkout(name: &str) -> (Runtime, String, PathBuf) {
    let folder = scratch_dir(&format!("hide-strip-{name}-"));
    let directory = folder
        .path()
        .canonicalize()
        .expect("a real scratch path")
        .join(name);
    std::fs::create_dir(&directory).expect("the checkout folder");
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&directory)
            .status()
            .expect("git init runs")
            .success()
    );
    std::fs::write(directory.join("notes.md"), "notes\n").expect("fixture file");
    let (mut runtime, checkout_id) = tab_order_runtime(&directory.to_string_lossy());
    runtime.test_dirs.push(folder);
    (runtime, checkout_id, directory)
}

/// #400: a tab Hide created belongs to the checkout it was created for while
/// its pane still reports the cwd it was born with, whether the session or
/// Herdr's acknowledgment arrives first; a tab made outside Hide with the same
/// cwd follows its pane, and a created tab's record leaves with the tab.
#[test]
fn a_created_tab_joins_its_requested_checkout_while_an_external_tab_follows_its_pane_cwd() {
    let (mut runtime, checkout_id, directory) = strip_checkout("created-membership");
    let checkout_path = directory.to_string_lossy().into_owned();
    // A folder of the test's own beside the checkout, not one holding it: a
    // pane in the checkout lies inside every row of a folder that holds it,
    // and which of those rows it joins is not what this test is about.
    let birth = directory.with_file_name("birth");
    std::fs::create_dir(&birth).expect("the birth folder");
    let birth_cwd = birth.to_string_lossy().into_owned();
    // Every tab but the first reports the birth cwd, outside the checkout.
    let payload = |tabs: &[&str]| {
        let mut payload = tab_order_payload(&checkout_path, tabs, tabs, "w-order:t1");
        for pane in &mut payload.panes {
            if pane.pane_id != "w-order:t1:p" {
                pane.cwd = Some(birth_cwd.clone());
            }
        }
        payload
    };
    let acknowledge = |runtime: &mut Runtime, tab_id: &str| {
        runtime.ingest_local_control_result(
            RemoteControlAction::CreateTab {
                workspace_id: "w-order".to_owned(),
                cwd: checkout_path.clone(),
                label: "new".to_owned(),
                area_id: None,
                group_id: None,
                admission_id: None,
            },
            Ok(RemoteControlOutcome::Acknowledged {
                created_tab_id: Some(tab_id.to_owned()),
                created_pane_id: Some(format!("{tab_id}:p")),
            }),
            1,
        );
    };
    let checkout_path_of = |runtime: &Runtime, tab_id: &str| {
        runtime
            .snapshot()
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .find(|checkout| {
                checkout
                    .tabs
                    .iter()
                    .any(|tab| tab.id.as_deref() == Some(tab_id))
            })
            .map(|checkout| checkout.path.clone())
    };

    // Acknowledged before the session carries the tab, which the answer
    // draws (PRD instant-pane-topology D-05).
    acknowledge(&mut runtime, "w-order:t2");
    assert!(runtime.take_republish_request());
    runtime.ingest_session(Ok(payload(&["w-order:t1", "w-order:t2", "w-order:t3"])));
    assert_eq!(
        ordered_tab_ids(&runtime, &checkout_id),
        ["w-order:t1", "w-order:t2"]
    );
    assert_eq!(
        checkout_path_of(&runtime, "w-order:t3").as_deref(),
        Some(birth_cwd.as_str()),
        "a tab made outside Hide is placed by its pane cwd"
    );
    assert!(!runtime.take_republish_request());

    // The session placed the tab before the acknowledgment arrived, so the
    // acknowledgment asks for one more publish, which moves it.
    let all = ["w-order:t1", "w-order:t2", "w-order:t3", "w-order:t4"];
    runtime.ingest_session(Ok(payload(&all)));
    assert_eq!(
        checkout_path_of(&runtime, "w-order:t4").as_deref(),
        Some(birth_cwd.as_str())
    );
    acknowledge(&mut runtime, "w-order:t4");
    assert!(runtime.take_republish_request());
    runtime.ingest_session(Ok(payload(&all)));
    assert_eq!(
        ordered_tab_ids(&runtime, &checkout_id),
        ["w-order:t1", "w-order:t2", "w-order:t4"]
    );

    // A closed tab takes its record along: a later tab under the same id is
    // one Hide did not create.
    runtime.ingest_session(Ok(payload(&["w-order:t1", "w-order:t3", "w-order:t4"])));
    runtime.ingest_session(Ok(payload(&all)));
    assert_eq!(
        ordered_tab_ids(&runtime, &checkout_id),
        ["w-order:t1", "w-order:t4"]
    );

    // Losing Herdr drops every record, because the next server numbers its
    // own tabs.
    runtime.ingest_session(Err(SessionFetchError::SocketMissing(
        "herdr went away".to_owned(),
    )));
    runtime.ingest_session(Ok(payload(&all)));
    assert_eq!(ordered_tab_ids(&runtime, &checkout_id), ["w-order:t1"]);
}

fn reorder_tab(runtime: &mut Runtime, checkout_id: &str, entry_id: &str, to_index: usize) -> bool {
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "reorder_tab",
        "payload": {
            "workspace_id": "workspace:order",
            "checkout_id": checkout_id,
            "tab_id": entry_id,
            "to_index": to_index
        }
    }))
    .expect("reorder tab event");
    runtime.dispatch_json(&event)
}

fn open_file(runtime: &mut Runtime, checkout_id: &str, path: &Path) {
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "file_open",
        "payload": {
            "path": path.to_string_lossy(),
            "workspace_id": "workspace:order",
            "checkout_id": checkout_id
        }
    }))
    .expect("file open event");
    assert!(runtime.dispatch_json(&event));
}

/// A repository with one linked worktree, both holding panes of the same
/// Herdr workspace, which is how a workspace comes to span two checkouts.
/// Returns the runtime, the repository's checkout id, and both directories.
fn split_workspace_checkouts(name: &str) -> (Runtime, String, PathBuf, PathBuf) {
    let folder = scratch_dir(&format!("hide-split-{name}-"));
    let root = folder.path().canonicalize().expect("a real fixture path");
    let repository = root.join("repo");
    std::fs::create_dir_all(&repository).expect("repository directory");
    let git = |arguments: &[&str], directory: &Path| {
        assert!(
            std::process::Command::new("git")
                // The fixture owns its identity, and it must not reach for
                // the operator's signing key: a commit the fixture makes
                // failed whenever the signing agent was not answering,
                // which made this suite fail for a reason that had nothing
                // to do with the code under test.
                .args(["-c", "commit.gpgsign=false"])
                .args(arguments)
                .current_dir(directory)
                .env("GIT_AUTHOR_NAME", "fixture")
                .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
                .env("GIT_COMMITTER_NAME", "fixture")
                .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()
                .expect("git runs")
                .success(),
            "git {arguments:?}"
        );
    };
    git(&["init", "-q", "-b", "main"], &repository);
    std::fs::write(repository.join("notes.md"), "notes\n").expect("fixture file");
    git(&["add", "notes.md"], &repository);
    git(&["commit", "-qm", "notes"], &repository);
    // A linked worktree is a second checkout of the same project, so the
    // catalog gives it its own row under one project.
    let worktree = root.join("feature");
    git(
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            &worktree.to_string_lossy(),
        ],
        &repository,
    );
    let (mut runtime, checkout_id) = tab_order_runtime(&repository.to_string_lossy());
    runtime.test_dirs.push(folder);
    (runtime, checkout_id, repository, worktree)
}

/// A session payload for one Herdr workspace whose tabs are split across
/// two directories, in Herdr's own order.
fn split_workspace_payload(
    tab_order: &[(&str, &str)],
    active_tab_id: &str,
) -> SessionSnapshotPayload {
    let tabs = tab_order
        .iter()
        .map(|(tab_id, _)| {
            serde_json::json!({"workspace_id": "w-order", "tab_id": tab_id, "label": "",
                "number": tab_id.rsplit(":t").next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(1)})
        })
        .collect::<Vec<_>>();
    let panes = tab_order
        .iter()
        .map(|(tab_id, cwd)| serde_json::json!({"pane_id": format!("{tab_id}:p"), "cwd": cwd}))
        .collect::<Vec<_>>();
    let layouts = tab_order
        .iter()
        .map(|(tab_id, _)| {
            serde_json::json!({
                "workspace_id": "w-order",
                "tab_id": tab_id,
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": format!("{tab_id}:p"),
                "panes": [{
                    "pane_id": format!("{tab_id}:p"),
                    "rect": {"x": 0, "y": 0, "width": 80, "height": 24}
                }],
                "splits": []
            })
        })
        .collect::<Vec<_>>();
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [],
        "workspaces": [{
            "workspace_id": "w-order",
            "label": "order",
            "active_tab_id": active_tab_id
        }],
        "tabs": tabs,
        "panes": panes,
        "layouts": layouts
    }))
    .expect("split session payload")
}

/// Every source file of a shell in this repository, with its path relative
/// to the repository, for the structure tests that keep a defect the core
/// once fixed from coming back on the other side of the wire.
fn shell_sources(relative_root: &str, extensions: &[&str]) -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative_root);
    let mut files = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .unwrap_or_else(|_| panic!("{} is readable", directory.display()))
        {
            let path = entry.expect("a source entry").path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            if !extensions.contains(&extension) || name.contains(".test.") {
                continue;
            }
            let relative = path.strip_prefix(&root).expect("under the root");
            files.push((
                format!("{relative_root}/{}", relative.display()),
                std::fs::read_to_string(&path).expect("a readable source"),
            ));
        }
    }
    files.sort();
    files
}

/// The web shell's sources, tests excluded.
fn web_sources() -> Vec<(String, String)> {
    shell_sources("web/src", &["ts", "tsx"])
}

/// Two registered checkouts with the first focused, so a reveal into the
/// second has a checkout switch to make. The second holds
/// `deep/nested/leaf/target.txt`, which is the shape SC1 and SC2 describe.
fn reveal_runtime() -> (Runtime, PathBuf, String, PathBuf, String) {
    let mut roots = Vec::new();
    let mut folders = Vec::new();
    for name in ["one", "two"] {
        let folder = scratch_dir(&format!("hide-reveal-{name}-"));
        std::fs::create_dir_all(folder.path().join("deep/nested/leaf")).expect("fixture tree");
        let directory = folder.path().canonicalize().expect("a real checkout path");
        folders.push(folder);
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&directory)
                .status()
                .expect("git init runs")
                .success()
        );
        std::fs::write(directory.join("deep/nested/leaf/target.txt"), "found\n")
            .expect("fixture file");
        roots.push(directory);
    }
    let mut runtime = runtime();
    runtime.test_dirs.extend(folders);
    runtime.snapshot.ui_state.workspace_registrations = roots
        .iter()
        .enumerate()
        .map(|(index, path)| WorkspaceRegistration {
            primary_checkout_id: None,
            id: format!("workspace:{index}"),
            label: format!("workspace {index}"),
            path: path.to_string_lossy().into_owned(),
            device_id: crate::node::TEST_NODE.to_owned(),
            pinned: false,
            home: false,
        })
        .collect();
    runtime.rebuild_catalog();
    let ids = roots
        .iter()
        .enumerate()
        .map(|(index, path)| workspace::checkout_id_for_path(&format!("workspace:{index}"), path))
        .collect::<Vec<_>>();
    runtime.snapshot.navigator.focused_workspace_id = Some("workspace:0".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some(ids[0].clone());
    runtime.snapshot.ui_state.focused_checkout_id = Some(ids[0].clone());
    runtime.snapshot.navigator.root_path = Some(roots[0].to_string_lossy().into_owned());
    // The panel starts hidden on a section that is not the tree, which is
    // the state SC1 names: the reveal has to open it and switch it.
    runtime.snapshot.ui_state.right_panel_visible = false;
    runtime.snapshot.ui_state.right_panel_section = RightPanelSection::Changes;
    let (first, second) = (roots[0].clone(), roots[1].clone());
    (runtime, first, ids[0].clone(), second, ids[1].clone())
}

fn reveal_event(workspace_id: &str, checkout_id: &str, path: &Path, is_directory: bool) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "reveal_path",
        "payload": {
            "path": path.to_string_lossy(),
            "workspace_id": workspace_id,
            "checkout_id": checkout_id,
            "is_directory": is_directory
        }
    }))
    .expect("reveal event")
}

fn explorer_runtime() -> (Runtime, PathBuf) {
    let folder = scratch_dir("hide-explorer-runtime-");
    let root = folder.path().canonicalize().expect("a real root");
    std::fs::create_dir_all(root.join("src/nested")).expect("fixture tree");
    std::fs::write(root.join("src/lib.rs"), "lib\n").expect("fixture file");
    let mut runtime = runtime();
    runtime.test_dirs.push(folder);
    focus_local_checkout(&mut runtime, &root);
    (runtime, root)
}

/// Registers `root` as this machine's checkout and puts it in front, the
/// way a click on it in the sidebar leaves the navigator.
fn focus_local_checkout(runtime: &mut Runtime, root: &Path) {
    runtime.snapshot.ui_state.workspace_registrations = vec![WorkspaceRegistration {
        primary_checkout_id: None,
        id: "workspace:0".to_owned(),
        label: "workspace 0".to_owned(),
        path: root.to_string_lossy().into_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        pinned: false,
        home: false,
    }];
    runtime.rebuild_catalog();
    let checkout_id = workspace::checkout_id_for_path("workspace:0", root);
    runtime.snapshot.navigator.focused_workspace_id = Some("workspace:0".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout_id);
    runtime.snapshot.navigator.root_path = Some(root.to_string_lossy().into_owned());
}

fn explorer_event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": kind,
        "payload": payload
    }))
    .expect("explorer event")
}

fn closed_file(key: &str, path: &str) -> ClosedItem {
    ClosedItem::File {
        key: key.to_owned(),
        device_id: crate::node::TEST_NODE.to_owned(),
        workspace_id: "workspace:0".to_owned(),
        checkout_id: "checkout:0".to_owned(),
        checkout_path: "/repo".to_owned(),
        path: path.to_owned(),
        label: Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path)
            .to_owned(),
    }
}

fn close_capture_request(key: &str) -> live::CloseCaptureRequest {
    live::CloseCaptureRequest {
        failed_terminals: Vec::new(),
        retain_for_reopen: true,
        key: key.to_owned(),
        connection_generation: 0,
        context: ClosedContext {
            workspace_id: "workspace:0".to_owned(),
            workspace_label: "Fixture".to_owned(),
            workspace_ids_before_close: vec!["workspace:0".to_owned()],
            tab_ids_before_close: vec![format!("tab:{key}")],
            pane_ids_before_close: vec![],
            checkout_id: "checkout:0".to_owned(),
            checkout_path: "/repo".to_owned(),
            tab_id: format!("tab:{key}"),
            tab_label: key.to_owned(),
            tab_index: 0,
            agent_area: None,
            agent_group: None,
            replacement_shell: false,
            replacement_in_tab: false,
            tab_name: Default::default(),
        },
        panes: vec![],
        target: live::CloseCaptureTarget::Tab {
            tab_id: format!("tab:{key}"),
        },
    }
}

#[test]
fn a_device_that_is_not_connected_lends_no_herdr_api_to_a_phone() {
    // The phone's detail reads this as "the device is not connected" (mobile-companion B28).
    let runtime = runtime();
    assert!(runtime.remote_herdr_api("ssh-mini").is_none());
}

#[test]
fn start_claims_survive_local_projection_without_becoming_public_titles() {
    let claim = crate::start_claim::label(&"a".repeat(64));
    let mut payload = context_payload();
    assert!(
        payload.panes[0].start_claim.is_none(),
        "older payloads still decode"
    );
    for carried_separately in [false, true] {
        payload.panes[0].start_claim = carried_separately.then(|| claim.clone());
        payload.panes[0].label = (!carried_separately).then(|| claim.clone());
        payload.panes[0].terminal_title = Some(claim.clone());
        let panes = project_layout_panes(&payload.layouts[0], &payload, &[], &[], "/repo");
        let pane = &panes[0];
        assert_eq!(pane.start_claim.as_deref(), Some(claim.as_str()));
        assert_eq!(pane.herdr_label, None);
        assert_eq!(pane.terminal_title, None);
        let public = serde_json::to_string(pane).unwrap();
        assert!(!public.contains(&claim));
        assert!(!public.contains("start_claim"));
    }

    payload.panes[0].label = Some(" 작업 메모 ".into());
    payload.panes[0].terminal_title = Some("hide-start:v1:human notes".into());
    let panes = project_layout_panes(&payload.layouts[0], &payload, &[], &[], "/repo");
    assert_eq!(panes[0].herdr_label.as_deref(), Some(" 작업 메모 "));
    assert_eq!(
        panes[0].terminal_title.as_deref(),
        Some("hide-start:v1:human notes")
    );
    assert_eq!(panes[0].start_claim.as_deref(), Some(claim.as_str()));
}
