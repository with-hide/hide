//! File tabs on an SSH device (PRD S5.5 B8-B15, B34): the read runs on a
//! worker and lands in the checkout that asked, and a save is checked
//! against the revision the draft was based on, runs one at a time, and is
//! read back rather than resent when its answer is lost.
//!
//! The device is a double of the helper connection behind the same
//! `NodeLink` boundary: it answers with the helper's own dispatch, so the
//! file work is real, and it can hold a request or lose an answer.

use super::*;
use crate::model::{ViewDisplayState, ViewNodeSnapshot};
use crate::node_access::{LinkAnswer, LinkError, NodeLink};
use crate::view_layout::{DisplayKind, Edge};
use hide_node_link::protocol::Call;
use serde_json::Value;
use std::sync::Condvar;

const DEVICE: &str = "device-a";
const WORKSPACE: &str = "remote:device-a:workspace:1";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Answer {
    Normally,
    /// The write happens and its answer never arrives.
    LoseAfterEffect,
    /// The connection drops before the write reaches the device.
    LoseBeforeEffect,
    /// The write happens, its answer is lost, and the link stays down.
    LoseAfterEffectThenDrop,
    /// Every request fails without being sent: the device is unreachable.
    Unreachable,
    /// The next request fails as unreachable, on a connection that has
    /// since been replaced; the ones after it are answered.
    UnreachableOnce,
}

#[derive(Default)]
struct Gate {
    held: bool,
    /// Files whose reads wait, by their path under the root, so a test can
    /// land two reads in the order a burst can.
    held_reads: HashSet<String>,
    waiting: usize,
}

pub(super) struct FakeDevice {
    answer: Mutex<Answer>,
    gate: Mutex<Gate>,
    released: Condvar,
    saves: Mutex<Vec<String>>,
    /// The files read through this device, by their path.
    reads: Mutex<Vec<String>>,
    pins: Mutex<HashMap<String, hide_node_link::RootIdentity>>,
    closes: Mutex<Vec<&'static str>>,
}

impl FakeDevice {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(Answer::Normally),
            gate: Mutex::new(Gate::default()),
            released: Condvar::new(),
            saves: Mutex::new(Vec::new()),
            reads: Mutex::new(Vec::new()),
            pins: Mutex::new(HashMap::new()),
            closes: Mutex::new(Vec::new()),
        })
    }

    fn answer(&self, answer: Answer) {
        *self.answer.lock().unwrap() = answer;
    }

    pub(super) fn hold(&self) {
        self.gate.lock().unwrap().held = true;
    }

    pub(super) fn release(&self) {
        self.gate.lock().unwrap().held = false;
        self.released.notify_all();
    }

    fn hold_read(&self, relative: &str) {
        self.gate
            .lock()
            .unwrap()
            .held_reads
            .insert(relative.to_owned());
    }

    fn release_read(&self, relative: &str) {
        self.gate.lock().unwrap().held_reads.remove(relative);
        self.released.notify_all();
    }

    fn waiting(&self) -> usize {
        self.gate.lock().unwrap().waiting
    }

    fn saves(&self) -> Vec<String> {
        self.saves.lock().unwrap().clone()
    }

    fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }
}

impl NodeLink for FakeDevice {
    fn close(&self, _reason: &str) {
        self.closes.lock().unwrap().push("close");
    }

    fn close_when_idle(&self, _reason: &str) {
        self.closes.lock().unwrap().push("when_idle");
    }

    fn call(&self, call: Call, timeout: Duration) -> Result<LinkAnswer, LinkError> {
        {
            let read = match &call {
                Call::OpenDocument { path, .. } => Some(path.as_str()),
                _ => None,
            };
            if let Some(read) = read {
                self.reads.lock().unwrap().push(read.to_owned());
            }
            let mut gate = self.gate.lock().unwrap();
            gate.waiting += 1;
            while gate.held || read.is_some_and(|read| gate.held_reads.contains(read)) {
                gate = self.released.wait(gate).unwrap();
            }
            gate.waiting -= 1;
        }
        let answer = *self.answer.lock().unwrap();
        let is_save = matches!(call, Call::Save { .. });
        if let Call::Save { contents, .. } = &call {
            self.saves.lock().unwrap().push(contents.clone());
        }
        match answer {
            Answer::Unreachable => Err(LinkError::NotConnected(
                "The device helper is not connected".to_owned(),
            )),
            Answer::UnreachableOnce => {
                *self.answer.lock().unwrap() = Answer::Normally;
                Err(LinkError::NotConnected(
                    "The device helper is not connected".to_owned(),
                ))
            }
            Answer::LoseBeforeEffect if is_save => Err(LinkError::Unknown(
                "The connection ended before the device answered".to_owned(),
            )),
            Answer::LoseAfterEffectThenDrop if is_save => {
                let _ = hide_node::Local::of_process().call(call, timeout);
                *self.answer.lock().unwrap() = Answer::Unreachable;
                Err(LinkError::Unknown(
                    "The connection ended before the device answered".to_owned(),
                ))
            }
            Answer::LoseAfterEffect if is_save => {
                let _ = hide_node::Local::of_process().call(call, timeout);
                Err(LinkError::Unknown(
                    "The connection ended before the device answered".to_owned(),
                ))
            }
            _ => hide_node::Local::of_process().call(call, timeout),
        }
    }

    fn pinned(&self, root: &str) -> Option<hide_node_link::RootIdentity> {
        self.pins.lock().unwrap().get(root).copied()
    }

    fn pin(&self, root: &str, identity: Option<hide_node_link::RootIdentity>) {
        let mut pins = self.pins.lock().unwrap();
        match identity {
            Some(identity) => pins.insert(root.to_owned(), identity),
            None => pins.remove(root),
        };
    }
}

struct Fixture {
    root: PathBuf,
    /// The device checkout, keyed by its folder as the catalog keys it.
    checkout: String,
    shared: SharedRuntime,
    device: Arc<FakeDevice>,
}

impl Fixture {
    fn new() -> Self {
        Self::with(|runtime| runtime)
    }

    /// The fixture with `setup` applied to the runtime before its workers
    /// start.
    fn with(setup: impl FnOnce(Runtime) -> Runtime) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("a.txt"), "old\n").unwrap();
        let mut runtime = runtime();
        let checkout_id = crate::device_catalog::checkout_id(DEVICE, &root.to_string_lossy());
        let mut remote = workspace(
            WORKSPACE,
            "Remote",
            &root.to_string_lossy(),
            vec![checkout(
                WORKSPACE,
                &checkout_id,
                &root.to_string_lossy(),
                None,
            )],
        );
        remote.device_id = DEVICE.to_owned();
        remote.remote_target_id = Some(DEVICE.to_owned());
        runtime.snapshot.navigator.workspaces = vec![remote];
        runtime.snapshot.navigator.focused_workspace_id = Some(WORKSPACE.to_owned());
        runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
        let device = FakeDevice::new();
        runtime.device_hosts.insert(
            DEVICE.to_owned(),
            hosts::DeviceHost::new(
                hosts::HostPhase::Ready {
                    host: device.clone(),
                    platform: "macos aarch64".to_owned(),
                    helper_path: Some("/fake/hided".to_owned()),
                    windows_drives: None,
                },
                1,
            ),
        );
        // The checkout is written by the runtime's workers, so it is the
        // runtime's and goes after they have let go.
        runtime.test_dirs.push(dir);
        let shared = SharedRuntime::new(setup(runtime));
        shared
            .lock()
            .unwrap()
            .install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
        Self {
            root,
            checkout: checkout_id,
            shared,
            device,
        }
    }

    /// The checkout as a node that dials this core holds it: in the node's
    /// Herdr session rather than the navigator, in front there while the
    /// node is the device in front.
    fn in_node_session(self) -> Self {
        {
            let mut runtime = self.shared.lock().unwrap();
            let project = runtime.snapshot.navigator.workspaces.remove(0);
            let mut session = super::device_catalog::session(vec![project]);
            session.focused_workspace_id = Some(WORKSPACE.to_owned());
            session.focused_checkout_id = Some(self.checkout.clone());
            let mut node = super::device_catalog::connected_status(DEVICE);
            node.session = Some(session);
            runtime.snapshot.status.remote.push(node);
            let mut row = runtime.snapshot.navigator.devices[0].clone();
            row.id = DEVICE.to_owned();
            row.kind = "remote".to_owned();
            runtime.snapshot.navigator.devices.push(row);
            runtime.snapshot.navigator.focused_device_id = Some(DEVICE.to_owned());
            runtime.snapshot.navigator.focused_workspace_id = None;
            runtime.snapshot.navigator.focused_checkout_id = None;
        }
        self
    }

    /// A folder the runtime writes into on its workers. It is the runtime's,
    /// so it goes after they have let go, never under a save still running.
    fn runtime_dir(&self, prefix: &str) -> PathBuf {
        let dir = scratch_dir(prefix);
        let path = dir.path().to_path_buf();
        self.shared.lock().unwrap().test_dirs.push(dir);
        path
    }

    fn path(&self, name: &str) -> String {
        self.root.join(name).to_string_lossy().into_owned()
    }

    fn dispatch(&self, kind: &str, payload: Value) {
        let event =
            serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload});
        self.shared
            .lock()
            .unwrap()
            .dispatch_json(&serde_json::to_vec(&event).unwrap());
    }

    fn open(&self, name: &str) {
        self.dispatch(
            "file_open",
            serde_json::json!({"path": self.path(name), "workspace_id": WORKSPACE, "checkout_id": &self.checkout}),
        );
    }

    fn save(&self, name: &str, contents: &str) {
        self.dispatch(
            "file_save",
            serde_json::json!({
                "tab_id": Runtime::file_tab_id(WORKSPACE, &self.checkout, &self.path(name)),
                "path": self.path(name),
                "contents_utf8": contents,
            }),
        );
    }

    fn document(&self, name: &str) -> Option<EditorDocumentSnapshot> {
        let tab_id = Runtime::file_tab_id(WORKSPACE, &self.checkout, &self.path(name));
        self.shared
            .lock()
            .unwrap()
            .editor_documents
            .get(&tab_id)
            .map(|document| EditorDocumentSnapshot::clone(document))
    }

    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn wait(&self, what: &str, mut ready: impl FnMut(&Runtime) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if ready(&self.shared.lock().unwrap()) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_for_document(
        &self,
        name: &str,
        what: &str,
        ready: impl Fn(&EditorDocumentSnapshot) -> bool,
    ) {
        let tab_id = Runtime::file_tab_id(WORKSPACE, &self.checkout, &self.path(name));
        self.wait(what, |runtime| {
            runtime
                .editor_documents
                .get(&tab_id)
                .is_some_and(|document| ready(document))
        });
    }

    fn last_error(&self) -> Option<String> {
        self.shared
            .lock()
            .unwrap()
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.clone())
    }

    fn open_and_wait(&self, name: &str) -> EditorDocumentSnapshot {
        self.open(name);
        self.wait_for_document(name, "the device read", |_| true);
        self.document(name).unwrap()
    }
}

#[test]
fn a_device_file_is_read_on_a_worker_and_shows_as_a_tab_when_it_arrives() {
    let f = Fixture::new();
    f.device.hold();
    f.open("a.txt");
    {
        let runtime = f.shared.lock().unwrap();
        assert!(
            runtime.snapshot.editor.tabs.is_empty(),
            "no tab before the answer"
        );
        assert_eq!(runtime.snapshot.editor.opening.len(), 1);
        assert_eq!(runtime.snapshot.editor.opening[0].path, f.path("a.txt"));
    }
    f.device.release();
    f.wait("the tab", |runtime| {
        runtime.snapshot.editor.document.is_some()
    });
    let runtime = f.shared.lock().unwrap();
    assert!(runtime.snapshot.editor.opening.is_empty());
    let document = runtime.snapshot.editor.document.as_ref().unwrap();
    assert_eq!(document.contents_utf8.as_deref(), Some("old\n"));
    assert_eq!(
        document.revision.as_deref(),
        Some(hide_node_link::document::revision_of(b"old\n").as_str())
    );
}

/// A terminal link to a file of a node's own checkout, which a window on
/// that node prints: the checkout is in the node's session, so the reveal
/// finds it there, reads the file through the node and moves no focus of the
/// core's, since the node's Herdr owns that checkout's focus.
#[test]
fn a_file_of_a_nodes_checkout_in_front_is_revealed_through_the_node() {
    let f = Fixture::new().in_node_session();
    f.dispatch("reveal_path", node_reveal(&f, "a.txt"));
    f.wait("the revealed file", |runtime| {
        runtime.snapshot.editor.document.is_some()
    });
    let runtime = f.shared.lock().unwrap();
    assert_eq!(runtime.snapshot.status.last_error, None);
    assert_eq!(
        runtime.snapshot.ui_state.selected_path.as_deref(),
        Some(f.path("a.txt").as_str())
    );
    assert!(runtime.snapshot.ui_state.right_panel_visible);
    assert_eq!(runtime.snapshot.navigator.focused_checkout_id, None);
    assert!(
        f.device.reads().iter().any(|read| read.ends_with("a.txt")),
        "read through the node: {:?}",
        f.device.reads()
    );
}

/// Group-tabs B28 makes checkout navigation core-owned: an empty checkout
/// can read its file immediately without a Herdr workspace or focus request.
#[test]
fn a_file_of_a_nodes_checkout_behind_is_revealed_without_a_herdr_focus_ack() {
    let (f, requests, _held) = behind_reveal("documents-node-reveal");
    f.wait("the device file read", |_| {
        f.device.reads().iter().any(|read| read.ends_with("a.txt"))
    });
    {
        let mut runtime = f.shared.lock().unwrap();
        assert_eq!(runtime.snapshot.status.last_error, None);
        assert_eq!(
            runtime.snapshot.navigator.focused_device_id.as_deref(),
            Some(DEVICE)
        );
        assert!(
            revealing(&f, &runtime),
            "the core choice starts its file read"
        );
        let other = other_checkout(&runtime);
        land(&mut runtime, &other);
        assert_eq!(
            runtime.front_checkout().map(|(_, checkout)| checkout),
            Some(f.checkout.as_str()),
            "stale Herdr focus cannot cancel the chosen checkout"
        );
    }
    assert!(
        requests.lock().unwrap().is_empty(),
        "an empty checkout needs no PTY or focus mutation"
    );
    f.device.release_read("a.txt");
    f.wait("the revealed file", |runtime| {
        runtime.snapshot.ui_state.selected_path.as_deref() == Some(f.path("a.txt").as_str())
    });
    assert_eq!(f.last_error(), None);
}

/// The read stays asynchronous after core navigation. A newer explicit
/// checkout choice keeps the late result from taking the current screen.
#[test]
fn a_core_chosen_remote_reveal_read_does_not_replace_a_newer_checkout_choice() {
    let (f, _, _held) = behind_reveal("documents-node-reveal-new-choice");
    f.wait("the held file read", |_| {
        f.device.reads().iter().any(|read| read.ends_with("a.txt"))
    });
    let other;
    {
        let mut runtime = f.shared.lock().unwrap();
        other = other_checkout(&runtime);
        runtime.request_remote_control(RemoteControlPayload {
            target_id: DEVICE.to_owned(),
            request_id: "later".to_owned(),
            report_pane_focus_outcome: false,
            focus_device: true,
            request: RemoteControlRequest::FocusWorkspace {
                workspace_id: OTHER_WORKSPACE.to_owned(),
                checkout_id: Some(other.clone()),
            },
        });
        runtime.sync_workspace_view();
        assert_eq!(
            runtime.front_checkout().map(|(_, checkout)| checkout),
            Some(other.as_str())
        );
    }
    f.device.release_read("a.txt");
    f.wait("the old read to settle", |runtime| {
        runtime.snapshot.editor.opening.is_empty()
    });
    let runtime = f.shared.lock().unwrap();
    assert_eq!(
        runtime.front_checkout().map(|(_, checkout)| checkout),
        Some(other.as_str())
    );
    assert_ne!(
        runtime.snapshot.ui_state.selected_path.as_deref(),
        Some(f.path("a.txt").as_str())
    );
    assert!(runtime.snapshot.editor.document.is_none());
}

/// Device removal still retires an in-flight read; changing checkout
/// navigation must not weaken the existing late-answer cancellation boundary.
#[test]
fn a_core_chosen_remote_reveal_read_is_dropped_when_its_device_is_removed() {
    let (f, _, _held) = behind_reveal("documents-node-reveal-removed");
    f.wait("the held file read", |_| {
        f.device.reads().iter().any(|read| read.ends_with("a.txt"))
    });
    {
        let mut runtime = f.shared.lock().unwrap();
        runtime.retire_device_editor_tabs(DEVICE);
        runtime.forget_device_views(DEVICE);
        assert!(runtime.snapshot.editor.opening.is_empty());
    }
    f.device.release_read("a.txt");
    f.wait("the late reveal answer to be dropped", |runtime| {
        diagnostic_count(runtime, "file.open_dropped") == 1
    });
    let runtime = f.shared.lock().unwrap();
    assert!(runtime.snapshot.editor.tabs.is_empty());
    assert!(runtime.snapshot.editor.document.is_none());
}

/// A failed assertion also releases the boundary gate before the runtime
/// joins its workers, so the test reports the failure rather than hanging.
struct HeldRevealRead(Arc<FakeDevice>);
impl Drop for HeldRevealRead {
    fn drop(&mut self) {
        self.0.release_read("a.txt");
    }
}

const OTHER_WORKSPACE: &str = "remote:device-a:workspace:2";

/// A node fixture with two empty checkouts and a clicked file whose actual
/// device read is held at the NodeLink boundary.
fn behind_reveal(views: &str) -> (Fixture, Arc<Mutex<Vec<Value>>>, HeldRevealRead) {
    let f = Fixture::with(|runtime| super::workspace_view::with_new_views(runtime, views))
        .in_node_session();
    let requests = Arc::new(Mutex::new(Vec::new()));
    {
        let mut runtime = f.shared.lock().unwrap();
        runtime.install_remote_control(RemoteControlContext::new(
            DEVICE,
            Arc::new(super::device_catalog::RecordingHerdr {
                requests: requests.clone(),
            }),
            Weak::new(),
            crate::handle::ChangeNotifier::noop(),
        ));
        let other = f.root.join("other").to_string_lossy().into_owned();
        let other_id = crate::device_catalog::checkout_id(DEVICE, &other);
        let mut project = workspace(
            OTHER_WORKSPACE,
            "Other",
            &other,
            vec![checkout(OTHER_WORKSPACE, &other_id, &other, None)],
        );
        project.device_id = DEVICE.to_owned();
        project.remote_target_id = Some(DEVICE.to_owned());
        let session = runtime.snapshot.status.remote[0].session.as_mut().unwrap();
        session.workspaces.push(project);
        session.focused_workspace_id = None;
        session.focused_checkout_id = None;
        runtime.snapshot.navigator.focused_device_id = Some(crate::node::TEST_NODE.to_owned());
    }
    f.device.hold_read("a.txt");
    let held = HeldRevealRead(f.device.clone());
    f.dispatch("reveal_path", node_reveal(&f, "a.txt"));
    (f, requests, held)
}

fn other_checkout(runtime: &Runtime) -> String {
    runtime.snapshot.status.remote[0]
        .session
        .as_ref()
        .unwrap()
        .workspaces[1]
        .checkouts[0]
        .id
        .clone()
}

/// The node's Herdr moved its front to `checkout`, and the sync ran.
fn land(runtime: &mut Runtime, checkout: &str) {
    let session = runtime.snapshot.status.remote[0].session.as_mut().unwrap();
    let workspace = session
        .workspaces
        .iter()
        .find(|workspace| {
            workspace
                .checkouts
                .iter()
                .any(|candidate| candidate.id == checkout)
        })
        .unwrap()
        .id
        .clone();
    session.focused_workspace_id = Some(workspace);
    session.focused_checkout_id = Some(checkout.to_owned());
    runtime.sync_workspace_view();
}

/// Whether a reveal of a.txt has started: its read is out or it landed.
fn revealing(f: &Fixture, runtime: &Runtime) -> bool {
    !runtime.snapshot.editor.opening.is_empty()
        || runtime.snapshot.ui_state.selected_path.as_deref() == Some(f.path("a.txt").as_str())
}

fn node_reveal(f: &Fixture, name: &str) -> Value {
    serde_json::json!({
        "path": f.path(name), "workspace_id": WORKSPACE, "checkout_id": &f.checkout,
        "is_directory": false, "device_id": DEVICE,
    })
}

/// A read still running when its device is removed opens no tab when its
/// answer lands, and the log says the answer was dropped.
#[test]
fn a_device_read_that_lands_after_the_device_was_removed_opens_nothing() {
    let f = Fixture::new();
    f.device.hold();
    f.open("a.txt");
    {
        let mut runtime = f.shared.lock().unwrap();
        runtime.retire_device_editor_tabs(DEVICE);
        assert!(runtime.snapshot.editor.opening.is_empty());
    }
    f.device.release();
    f.wait("the late answer to be dropped", |runtime| {
        diagnostic_count(runtime, "file.open_dropped") == 1
    });
    let runtime = f.shared.lock().unwrap();
    assert!(runtime.snapshot.editor.tabs.is_empty());
    assert!(runtime.snapshot.editor.document.is_none());
}

/// B34: the answer belongs to the checkout that asked. The operator moved
/// on meanwhile, so the tab joins that checkout without taking the screen.
#[test]
fn a_device_read_that_arrives_after_the_operator_moved_on_does_not_take_the_screen() {
    let f = Fixture::new();
    f.device.hold();
    f.open("a.txt");
    f.shared
        .lock()
        .unwrap()
        .snapshot
        .navigator
        .focused_checkout_id = None;
    f.device.release();
    f.wait("the tab", |runtime| {
        !runtime.snapshot.editor.tabs.is_empty()
    });
    let runtime = f.shared.lock().unwrap();
    assert_eq!(runtime.snapshot.editor.tabs[0].checkout_id, f.checkout);
    assert_eq!(runtime.snapshot.editor.active_tab_id, None);
    assert!(runtime.snapshot.editor.document.is_none());
}

#[test]
fn a_device_save_writes_the_draft_and_moves_the_revision() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    f.save("a.txt", "new\n");
    f.wait_for_document("a.txt", "the save", |document| !document.dirty);
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "new\n"
    );
    let document = f.document("a.txt").unwrap();
    assert_eq!(
        document.revision.as_deref(),
        Some(hide_node_link::document::revision_of(b"new\n").as_str())
    );
    assert_eq!(document.save, None);
}

/// B13: another writer changed the file; the save is refused, the file and
/// the draft both survive, and Keep Editing makes the next save replace
/// that version on purpose.
#[test]
fn a_device_save_over_a_changed_file_is_a_conflict_until_the_operator_keeps_editing() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    std::fs::write(f.root.join("a.txt"), "theirs\n").unwrap();
    f.save("a.txt", "mine\n");
    f.wait_for_document("a.txt", "the conflict", |document| {
        document.conflict.is_some()
    });
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "theirs\n"
    );
    let document = f.document("a.txt").unwrap();
    assert!(document.dirty);
    assert_eq!(document.contents_utf8.as_deref(), Some("mine\n"));
    let conflict = document.conflict.unwrap();
    assert_eq!(
        conflict.disk_revision.as_deref(),
        Some(hide_node_link::document::revision_of(b"theirs\n").as_str())
    );
    assert_eq!(f.last_error().as_deref(), Some("file.save_conflict"));

    f.dispatch(
        "file_conflict",
        serde_json::json!({"action": "keep_editing"}),
    );
    f.save("a.txt", "mine\n");
    f.wait_for_document("a.txt", "the save over it", |document| !document.dirty);
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "mine\n"
    );
}

/// B14: the write landed but its answer was lost. The file is read back and
/// the save is recognised as done; nothing is sent twice.
#[test]
fn a_save_whose_answer_was_lost_after_it_landed_is_read_back_as_saved() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    f.device.answer(Answer::LoseAfterEffect);
    f.save("a.txt", "new\n");
    f.wait_for_document("a.txt", "the read-back", |document| {
        document.save.is_none() && !document.dirty
    });
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "new\n"
    );
    assert_eq!(f.device.saves(), vec!["new\n".to_owned()], "never resent");
    assert_eq!(
        diagnostic_count(&f.shared.lock().unwrap(), "file.save_settled_saved"),
        1
    );
}

/// B14: the connection dropped before the write. The read-back finds the
/// old file, so the draft is kept, the tab says the save has not reached the
/// file yet (it may still), nothing is resent, and Retry saves.
#[test]
fn a_save_whose_answer_was_lost_before_it_landed_is_read_back_as_not_saved_yet() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    f.device.answer(Answer::LoseBeforeEffect);
    f.save("a.txt", "new\n");
    f.wait("the verdict", |runtime| {
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str())
            == Some("file.save_not_applied")
    });
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "old\n"
    );
    let document = f.document("a.txt").unwrap();
    assert!(document.dirty);
    assert_eq!(document.contents_utf8.as_deref(), Some("new\n"));
    assert_eq!(
        document.save.map(|save| save.state).as_deref(),
        Some("not_applied"),
        "the tab says the save has not reached the file"
    );
    assert_eq!(f.device.saves().len(), 1, "never resent");

    f.device.answer(Answer::Normally);
    f.save("a.txt", "new\n");
    f.wait_for_document("a.txt", "the retried save", |document| !document.dirty);
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "new\n"
    );
}

/// B14: while the device cannot be reached the result stays unknown, a new
/// save is refused rather than sent, and the read-back runs when the helper
/// is back.
#[test]
fn an_unknown_save_waits_for_the_device_and_blocks_the_next_save_until_read_back() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    f.device.answer(Answer::LoseAfterEffectThenDrop);
    f.save("a.txt", "new\n");
    f.wait_for_document("a.txt", "the refused read-back", |document| {
        document.save.as_ref().is_some_and(|save| {
            save.state == "unknown" && save.message.as_deref().unwrap_or("").contains("waiting")
        })
    });

    // The refused save asks for a read-back, which is still on its way to
    // the old connection when the helper comes back.
    f.device.hold();
    f.save("a.txt", "newer\n");
    assert_eq!(f.last_error().as_deref(), Some("file.save_unsettled"));
    f.wait("the read-back to reach the device", |_| {
        f.device.waiting() == 1
    });
    f.shared.lock().unwrap().settle_device_saves(DEVICE);
    f.device.answer(Answer::UnreachableOnce);
    f.device.release();
    assert_eq!(
        f.device.saves(),
        vec!["new\n".to_owned()],
        "the second save was not sent"
    );
    f.wait_for_document("a.txt", "the read-back after reconnect", |document| {
        document.save.is_none()
    });
    let document = f.document("a.txt").unwrap();
    assert_eq!(
        document.revision.as_deref(),
        Some(hide_node_link::document::revision_of(b"new\n").as_str())
    );
    assert!(document.dirty, "the newer draft is still unsaved");
    assert_eq!(document.contents_utf8.as_deref(), Some("newer\n"));
}

/// D-15: one save per document runs; drafts that arrive meanwhile collapse
/// to the newest, which is sent against the revision the first one wrote.
#[test]
fn drafts_saved_while_a_save_runs_collapse_to_the_newest() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    f.device.hold();
    f.save("a.txt", "first\n");
    f.wait("the first save to reach the device", |_| {
        f.device.waiting() == 1
    });
    f.save("a.txt", "second\n");
    f.save("a.txt", "third\n");
    assert_eq!(
        f.document("a.txt")
            .unwrap()
            .save
            .map(|save| save.state)
            .as_deref(),
        Some("saving")
    );
    f.device.release();
    f.wait_for_document("a.txt", "the last save", |document| {
        !document.dirty && document.save.is_none()
    });
    assert_eq!(
        f.device.saves(),
        vec!["first\n".to_owned(), "third\n".to_owned()]
    );
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "third\n"
    );
}

/// B52: without a helper (no consent, or revoked) nothing is read or sent,
/// and the reason names what to do.
#[test]
fn a_device_without_a_helper_reads_and_sends_nothing() {
    let f = Fixture::new();
    f.shared.lock().unwrap().device_hosts.clear();
    f.open("a.txt");
    let runtime = f.shared.lock().unwrap();
    assert!(runtime.snapshot.editor.tabs.is_empty());
    assert!(runtime.snapshot.editor.opening.is_empty());
    let error = runtime.snapshot.status.last_error.as_ref().unwrap();
    assert_eq!(error.kind, "file.open_failed");
    assert!(f.device.saves().is_empty());
}

impl Fixture {
    fn set_phase(&self, phase: hosts::HostPhase) {
        self.shared
            .lock()
            .unwrap()
            .device_hosts
            .get_mut(DEVICE)
            .unwrap()
            .phase = phase;
    }

    fn ready(&self) -> hosts::HostPhase {
        hosts::HostPhase::Ready {
            host: self.device.clone(),
            platform: "macos aarch64".to_owned(),
            helper_path: Some("/fake/hided".to_owned()),
            windows_drives: None,
        }
    }

    /// The document the shell draws, not the core's own copy.
    fn shown(&self) -> EditorDocumentSnapshot {
        self.shared
            .lock()
            .unwrap()
            .snapshot
            .editor
            .document
            .clone()
            .unwrap()
    }
}

/// A save asked for while the helper is still connecting waits for it
/// rather than failing, keeps only the newest draft, and goes out once the
/// helper is ready; the tab shows the draft and why it has not saved yet.
#[test]
fn a_save_while_the_helper_connects_waits_and_goes_out_when_it_is_ready() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    f.set_phase(hosts::HostPhase::Connecting);
    f.save("a.txt", "first\n");
    f.save("a.txt", "second\n");
    let shown = f.shown();
    assert!(shown.dirty);
    assert_eq!(shown.contents_utf8.as_deref(), Some("second\n"));
    assert_eq!(
        shown.save.map(|save| save.state).as_deref(),
        Some("waiting")
    );
    assert!(
        f.device.saves().is_empty(),
        "nothing is sent while connecting"
    );

    f.set_phase(f.ready());
    f.shared.lock().unwrap().settle_device_saves(DEVICE);
    f.wait_for_document("a.txt", "the waiting save", |document| {
        !document.dirty && document.save.is_none()
    });
    assert_eq!(f.device.saves(), vec!["second\n".to_owned()]);
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "second\n"
    );
}

/// The helper never came: the waiting save is dropped unsent, and the draft
/// stays in the tab with the reason.
#[test]
fn a_save_waiting_for_a_helper_that_fails_is_not_sent_and_keeps_the_draft() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    f.set_phase(hosts::HostPhase::Connecting);
    f.save("a.txt", "mine\n");
    f.shared
        .lock()
        .unwrap()
        .release_held_saves(DEVICE, "The SSH connection timed out");
    f.set_phase(hosts::HostPhase::Unavailable("timed out".to_owned()));
    let shown = f.shown();
    assert!(shown.dirty);
    assert_eq!(shown.contents_utf8.as_deref(), Some("mine\n"));
    assert_eq!(
        shown.save.map(|save| save.state).as_deref(),
        Some("refused")
    );
    assert_eq!(f.last_error().as_deref(), Some("file.save_unavailable"));
    f.shared.lock().unwrap().settle_device_saves(DEVICE);

    // A save refused outright still shows the draft it was given.
    f.save("a.txt", "again\n");
    let shown = f.shown();
    assert!(shown.dirty);
    assert_eq!(shown.contents_utf8.as_deref(), Some("again\n"));
    assert_eq!(f.last_error().as_deref(), Some("file.save_unavailable"));
    assert!(f.device.saves().is_empty());
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "old\n"
    );
}

/// B52: withdrawing consent while a save waits for the helper drops that save
/// unsent, so it cannot go out on its own if consent is given again.
#[test]
fn withdrawing_consent_drops_a_save_waiting_for_the_helper() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    f.set_phase(hosts::HostPhase::Connecting);
    f.save("a.txt", "mine\n");
    {
        let mut runtime = f.shared.lock().unwrap();
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: DEVICE.to_owned(),
                label: DEVICE.to_owned(),
                origin: crate::model::LinkOrigin::Dialed {
                    ssh_alias: DEVICE.to_owned(),
                },
                herdr_socket_path: None,
                herdr_session: None,
                host_consent: None,
            });
        runtime.set_host_consent(DEVICE, false);
    }
    let shown = f.shown();
    assert!(shown.dirty);
    assert_eq!(shown.contents_utf8.as_deref(), Some("mine\n"));
    assert_eq!(
        shown.save.map(|save| (
            save.state,
            save.message
                .unwrap_or_default()
                .contains("no longer allowed")
        )),
        Some(("refused".to_owned(), true)),
        "the save no longer waits and the tab says why"
    );
    assert_eq!(f.last_error().as_deref(), Some("file.save_unavailable"));
    f.set_phase(f.ready());
    f.shared.lock().unwrap().settle_device_saves(DEVICE);
    assert!(
        f.device.saves().is_empty(),
        "nothing goes out once allowed again"
    );
}

/// B52: a save already running when consent is withdrawn lands with its
/// real result; the connection is closed only once it is idle, and no new
/// save is sent.
#[test]
fn withdrawing_consent_lets_a_running_save_land() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    f.device.hold();
    f.save("a.txt", "mine\n");
    {
        let mut runtime = f.shared.lock().unwrap();
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: DEVICE.to_owned(),
                label: DEVICE.to_owned(),
                origin: crate::model::LinkOrigin::Dialed {
                    ssh_alias: DEVICE.to_owned(),
                },
                herdr_socket_path: None,
                herdr_session: None,
                host_consent: None,
            });
        runtime.set_host_consent(DEVICE, false);
    }
    assert_eq!(*f.device.closes.lock().unwrap(), vec!["when_idle"]);
    f.device.release();
    f.wait_for_document("a.txt", "the running save", |document| {
        !document.dirty && document.save.is_none()
    });
    assert_eq!(f.device.saves(), vec!["mine\n".to_owned()]);
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "mine\n"
    );
    f.save("a.txt", "after\n");
    assert_eq!(f.device.saves().len(), 1, "nothing new is sent");
}

/// B47: this machine's disk is read on a worker too, so a slow volume holds
/// no runtime lock. An unrelated action applies while the read waits, and a
/// reveal of that file moves the screen only when the read lands.
#[test]
fn a_slow_local_read_blocks_nothing_and_a_reveal_moves_only_when_it_lands() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("nested")).unwrap();
    std::fs::write(root.join("nested/b.txt"), "local\n").unwrap();
    let root_text = root.to_string_lossy().into_owned();
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "workspace:local",
        "Local",
        &root_text,
        vec![checkout(
            "workspace:local",
            "checkout:local",
            &root_text,
            None,
        )],
    )];
    runtime.snapshot.navigator.focused_workspace_id = Some("workspace:local".to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some("checkout:local".to_owned());
    runtime.snapshot.navigator.root_path = Some(root_text.clone());
    runtime.snapshot.ui_state.right_panel_visible = false;
    let disk = FakeDevice::new();
    runtime.own_node = disk.clone();
    let shared = SharedRuntime::new(runtime);
    shared
        .lock()
        .unwrap()
        .install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
    let dispatch = |event: Vec<u8>| shared.lock().unwrap().dispatch_json(&event);
    let file = root.join("nested/b.txt");

    disk.hold();
    dispatch(reveal_event(
        "workspace:local",
        "checkout:local",
        &file,
        false,
    ));
    wait_for("the read to reach the disk", || disk.waiting() > 0);
    dispatch(
        serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "ui_state_update",
            "payload": {
                "expanded_paths": [],
                "collapsed_checkout_ids": [],
                "selected_path": null,
                "selected_pane_id": null,
                "shortcut_bindings": {}
            }
        }))
        .unwrap(),
    );
    {
        let runtime = shared.lock().unwrap();
        assert!(!runtime.snapshot.ui_state.right_panel_visible);
        assert_eq!(runtime.snapshot.ui_state.selected_path, None);
        assert!(runtime.snapshot.editor.tabs.is_empty());
        assert_eq!(runtime.snapshot.editor.opening.len(), 1);
    }

    disk.release();
    wait(&shared, "the local read to land", |runtime| {
        runtime.snapshot.editor.document.is_some()
    });
    let runtime = shared.lock().unwrap();
    let file_text = file.to_string_lossy().into_owned();
    assert!(runtime.snapshot.ui_state.right_panel_visible);
    assert_eq!(
        runtime.snapshot.ui_state.selected_path.as_deref(),
        Some(file_text.as_str())
    );
    assert!(
        runtime
            .snapshot
            .ui_state
            .expanded_paths
            .contains(&root.join("nested").to_string_lossy().into_owned())
    );
    let document = runtime.snapshot.editor.document.as_ref().unwrap();
    assert_eq!(document.contents_utf8.as_deref(), Some("local\n"));
}

/// A device file tab is drawn in that device checkout's strip, after the
/// host's Herdr tabs, and a session sync that brings only Herdr's tabs keeps
/// it there without announcing a change on every poll.
#[test]
fn a_device_file_tab_joins_the_device_strip_and_survives_session_syncs() {
    let f = Fixture::new();
    let session = {
        let mut runtime = f.shared.lock().unwrap();
        let remote = runtime.snapshot.navigator.workspaces.remove(0);
        let session = RemoteSessionSnapshot {
            workspaces: vec![remote],
            agents: Vec::new(),
            active_tab_ids: Default::default(),
            focused_workspace_id: Some(WORKSPACE.to_owned()),
            focused_checkout_id: Some(f.checkout.clone()),
            focused_tab_id: None,
            focused_pane_id: None,
            pane_layouts: Vec::new(),
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
            herdr_version: None,
        };
        runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
            target_id: DEVICE.to_owned(),
            state: "connected".to_owned(),
            message: None,
            herdr_version: None,
            session: Some(session.clone()),
            files: RemoteFileListSnapshot::idle(),
            catalog: Default::default(),
            unreachable_since_unix_ms: None,
        });
        runtime.snapshot.navigator.focused_device_id = Some(DEVICE.to_owned());
        runtime.snapshot.navigator.focused_workspace_id = None;
        runtime.snapshot.navigator.focused_checkout_id = None;
        session
    };
    f.open_and_wait("a.txt");
    let tab_id = Runtime::file_tab_id(WORKSPACE, &f.checkout, &f.path("a.txt"));
    let strip_ids = |runtime: &Runtime| {
        runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .unwrap()
            .workspaces[0]
            .checkouts[0]
            .strip
            .iter()
            .map(|entry| (entry.kind, entry.source_id.clone()))
            .collect::<Vec<_>>()
    };
    let mut runtime = f.shared.lock().unwrap();
    assert_eq!(
        strip_ids(&runtime),
        vec![(StripTabKind::File, tab_id.clone())]
    );
    assert_eq!(
        runtime.snapshot.editor.active_tab_id.as_deref(),
        Some(tab_id.as_str())
    );

    runtime.ingest_remote_session(DEVICE, Ok(session.clone()));
    assert_eq!(
        strip_ids(&runtime),
        vec![(StripTabKind::File, tab_id.clone())]
    );
    assert!(
        !runtime.ingest_remote_session(DEVICE, Ok(session)),
        "an unchanged host session is not a change"
    );
}

/// B16, B18, B34: an Explorer change on a device's checkout is made by that
/// device's host, off the lock, and an open tab of the item follows it so its
/// next save lands on the new path; a change the tree asked for on another
/// device than the one in front is refused and changes nothing.
#[test]
fn a_device_explorer_change_runs_on_its_host_and_the_open_tab_follows_it() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    let root = f.root.to_string_lossy().into_owned();

    f.dispatch(
        "file_create",
        serde_json::json!({"root": root, "parent": root, "name": "planted.txt"}),
    );
    f.wait("the refusal", |runtime| {
        runtime
            .snapshot
            .explorer_operation
            .as_ref()
            .is_some_and(|operation| operation.phase == "failed")
    });
    assert!(
        !f.root.join("planted.txt").exists(),
        "the tree that asked is not the one in front"
    );

    f.dispatch(
        "path_rename",
        serde_json::json!({"root": root, "path": f.path("a.txt"), "name": "b.txt", "device_id": DEVICE}),
    );
    f.wait("the rename", |runtime| {
        runtime
            .snapshot
            .explorer_operation
            .as_ref()
            .is_some_and(|operation| operation.phase == "finished")
    });
    assert!(f.root.join("b.txt").is_file() && !f.root.join("a.txt").exists());
    let tab_id = Runtime::file_tab_id(WORKSPACE, &f.checkout, &f.path("a.txt"));
    assert_eq!(
        f.shared.lock().unwrap().document_places[&tab_id]
            .relative
            .as_str(),
        "b.txt"
    );
    f.dispatch(
        "file_save",
        serde_json::json!({"tab_id": tab_id, "path": f.path("b.txt"), "contents_utf8": "new\n"}),
    );
    f.wait("the save", |runtime| {
        runtime
            .editor_documents
            .get(&tab_id)
            .is_some_and(|document| {
                document.revision.as_deref()
                    == Some(hide_node_link::document::revision_of(b"new\n").as_str())
            })
    });
    assert_eq!(
        std::fs::read_to_string(f.root.join("b.txt")).unwrap(),
        "new\n"
    );
    assert!(
        !f.root.join("a.txt").exists(),
        "the save did not recreate the old path"
    );
}

/// A device folder removed and made again at the same path after it was
/// listed is a different folder: a change or an open names the root the
/// listing pinned, so it is refused there instead of landing in the new one.
#[test]
fn a_device_folder_replaced_after_it_was_listed_takes_no_change_or_open() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap().join("checkout");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.txt"), "first").unwrap();
    let device = FakeDevice::new();
    let root_text = root.to_string_lossy().into_owned();
    crate::node_access::list_folder(device.as_ref(), &root_text, "").unwrap();

    std::fs::rename(&root, dir.path().join("moved")).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.txt"), "second").unwrap();
    let place = files::DocumentRoot {
        device_id: DEVICE.to_owned(),
        path: root_text.clone(),
        identity: None,
    };
    let rename = files::ExplorerOperation::rename(
        &root_text,
        &root.join("a.txt").to_string_lossy(),
        "b.txt",
    )
    .unwrap();
    assert!(files::apply_explorer_operation(device.as_ref(), &place, &rename).is_err());
    assert!(root.join("a.txt").is_file(), "the new folder is untouched");

    let refused = files::open_document(
        device.as_ref(),
        &place,
        &root.join("a.txt").to_string_lossy(),
    );
    assert!(refused.is_err());
    // The refused open unpinned the root; the operator's next open adopts
    // the folder now at that path.
    let (document, _) = files::open_document(
        device.as_ref(),
        &place,
        &root.join("a.txt").to_string_lossy(),
    )
    .unwrap();
    assert_eq!(document.contents_utf8.as_deref(), Some("second"));
}

/// Reload and a save both decide which text and revision the tab holds, so
/// one waits for the other instead of taking its state away: a reload
/// landing mid-save would leave the save's revision on the reloaded text,
/// and the next save would overwrite the file without a conflict.
#[test]
fn reload_and_save_do_not_overtake_each_other_on_one_tab() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    f.device.hold();
    f.save("a.txt", "saved\n");
    f.wait("the save to be sent", |_| f.device.waiting() == 1);
    f.dispatch("file_conflict", serde_json::json!({"action": "reload"}));
    assert_eq!(f.last_error().as_deref(), Some("file.reload_busy"));
    f.device.release();
    f.wait_for_document("a.txt", "the save", |document| !document.dirty);
    assert_eq!(
        f.document("a.txt").unwrap().revision.as_deref(),
        Some(hide_node_link::document::revision_of(b"saved\n").as_str())
    );

    std::fs::write(f.root.join("a.txt"), "outside\n").unwrap();
    f.device.hold();
    f.dispatch("file_conflict", serde_json::json!({"action": "reload"}));
    f.wait("the reload to be sent", |_| f.device.waiting() == 1);
    f.save("a.txt", "typed during reload\n");
    assert_eq!(f.last_error().as_deref(), Some("file.save_during_reload"));
    assert!(
        f.device
            .saves()
            .iter()
            .all(|sent| sent != "typed during reload\n")
    );
    f.device.release();
    f.wait_for_document("a.txt", "the reload", |document| {
        document.contents_utf8.as_deref() == Some("outside\n")
    });
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "outside\n"
    );
}

/// B19, B22: a device checkout's History is read by that device's helper,
/// and an answer for the same path on another device is never shown: moving
/// to this machine's checkout at that path drops the device's list in the
/// same frame, and a device read that lands afterwards is refused.
#[test]
fn a_device_checkouts_history_comes_from_its_helper_and_stays_with_its_device() {
    let f = Fixture::new();
    let git = |arguments: &[&str]| {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&f.root)
                .args(arguments)
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "-q"]);
    let mut runtime = f.shared.lock().unwrap();
    runtime.snapshot.ui_state.right_panel_visible = true;
    runtime.snapshot.ui_state.right_panel_section = RightPanelSection::Changes;
    runtime.sync_changes_root_path();
    let request = runtime.changes_request().unwrap();
    assert_eq!(request.root.device_id, DEVICE);
    let device_answer = || crate::changes::ChangesAnswer {
        detail: None,
        key: Some(request.key()),
        selection: request.selection(),
        changes: crate::changes::read(&request).0,
    };
    let listed = device_answer();
    assert_eq!(listed.changes.unavailable_reason, None);
    assert_eq!(listed.changes.entries[0].relative_path, "a.txt");
    assert!(runtime.ingest_changes(listed));

    let local_id = "workspace:local-same-path";
    let local_checkout = "checkout:local-same-path";
    let path = f.root.to_string_lossy().into_owned();
    runtime.snapshot.navigator.workspaces.push(workspace(
        local_id,
        "Local",
        &path,
        vec![checkout(local_id, local_checkout, &path, None)],
    ));
    runtime.snapshot.navigator.focused_workspace_id = Some(local_id.to_owned());
    runtime.snapshot.navigator.focused_checkout_id = Some(local_checkout.to_owned());
    runtime.sync_changes_root_path();
    assert_eq!(
        runtime.snapshot.navigator.changes_root_path.as_deref(),
        Some(path.as_str())
    );
    assert_eq!(
        runtime.snapshot.changes,
        crate::model::ChangesSnapshot::default()
    );
    assert!(!runtime.ingest_changes(device_answer()));
    assert_eq!(
        runtime.snapshot.changes,
        crate::model::ChangesSnapshot::default()
    );
}

/// B23, B32, B34: a device's closed file is reopened from that device and
/// through its host, never from this machine, and a reopen on the device does
/// not take this machine's newer close off its stack.
#[test]
fn a_closed_device_file_reopens_only_on_its_device_and_leaves_this_machines_close() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    let tab_id = Runtime::file_tab_id(WORKSPACE, &f.checkout, &f.path("a.txt"));
    f.dispatch("file_close", serde_json::json!({"tab_id": tab_id}));
    {
        let mut runtime = f.shared.lock().unwrap();
        assert!(runtime.snapshot.editor.tabs.is_empty());
        // This machine closed a file after the device's.
        runtime.push_recent_closed(super::closed_file("local-close", "/repo/local.txt"));
        runtime.snapshot.navigator.devices.push(DeviceSnapshot {
            agent_scope: Default::default(),
            id: DEVICE.to_owned(),
            label: Some("Device".to_owned()),
            kind: "remote".to_owned(),
            state: "connected".to_owned(),
            message: None,
            problem: None,
            sign_in: None,
            ssh_alias: Some(DEVICE.to_owned()),
            wsl_distro: None,
            herdr_socket_path: None,
            agent_count: 0,
            test: None,
            host: Default::default(),
            kit: Default::default(),
            machine_name: None,
            dials_in: false,
            herdr_behind: None,
        });
        assert_eq!(runtime.snapshot.recent_closed.count, 1);
        assert_eq!(
            runtime.snapshot.recent_closed.top_label.as_deref(),
            Some("local.txt"),
            "this machine in front offers only its own close"
        );
    }

    f.dispatch("focus_device", serde_json::json!({"device_id": DEVICE}));
    {
        let runtime = f.shared.lock().unwrap();
        assert_eq!(runtime.snapshot.recent_closed.count, 1);
        assert_eq!(
            runtime.snapshot.recent_closed.top_label.as_deref(),
            Some("a.txt")
        );
        assert!(runtime.snapshot.recent_closed.can_reopen);
    }
    f.dispatch("reopen_closed", serde_json::json!({}));
    f.wait("the device file back in its tab", |runtime| {
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.id == tab_id)
    });
    let runtime = f.shared.lock().unwrap();
    assert_eq!(
        runtime.editor_documents[&tab_id].contents_utf8.as_deref(),
        Some("old\n"),
        "read back through the device's host"
    );
    assert_eq!(runtime.snapshot.recent_closed.count, 0);
    assert_eq!(runtime.recent_closed.len(), 1);
    assert_eq!(runtime.recent_closed[0].label(), "local.txt");
}

/// The front Workspace's displays in tree order, as the shell's next read
/// shows them: what each shows, whether it can, and why not.
fn view_displays(runtime: &mut Runtime) -> Vec<(DisplayKind, ViewDisplayState, Option<String>)> {
    runtime.sync_workspace_view();
    let mut shown = Vec::new();
    let mut nodes = vec![
        runtime
            .snapshot
            .workspace_view
            .as_ref()
            .expect("the device Workspace is in front")
            .layout
            .root
            .clone(),
    ];
    while let Some(node) = nodes.pop() {
        match node {
            ViewNodeSnapshot::Area(area) => shown.extend(
                area.displays
                    .into_iter()
                    .map(|display| (display.kind, display.state, display.reason)),
            ),
            ViewNodeSnapshot::Split(split) => nodes.extend([*split.second, *split.first]),
        }
    }
    shown
}

/// The device as a restart finds it: registered as "studio" with its helper
/// allowed, its catalog listed, its helper still connecting.
fn studio_connecting(runtime: &mut Runtime) {
    let host_consent = Some(runtime.new_host_consent());
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: DEVICE.to_owned(),
            label: "studio".to_owned(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: DEVICE.to_owned(),
            },
            herdr_socket_path: None,
            herdr_session: None,
            host_consent,
        });
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: DEVICE.to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: None,
        session: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: crate::model::DeviceCatalogSnapshot {
            state: "ready".to_owned(),
            ..Default::default()
        },
        unreachable_since_unix_ms: None,
    });
    runtime.device_hosts.get_mut(DEVICE).unwrap().phase = hosts::HostPhase::Connecting;
}

fn helper_ready(runtime: &mut Runtime, device: &Arc<FakeDevice>) {
    runtime.device_hosts.get_mut(DEVICE).unwrap().phase = hosts::HostPhase::Ready {
        host: device.clone(),
        platform: "macos aarch64".to_owned(),
        helper_path: Some("/fake/hided".to_owned()),
        windows_drives: None,
    };
}

/// S6 B20, B21, S7 contract 5, B16: after a restart a device Workspace's
/// displays wait for its helper and then its catalog, saying which, rather
/// than coming back unavailable while it connects; a helper that cannot be
/// used gives the device's own reason and its displays keep waiting, and
/// Retry on one says what it waits for; once its files can be read each
/// display binds to its document. Removing the device removes its displays
/// with its tabs.
#[test]
fn a_device_workspaces_displays_wait_for_its_helper_after_a_restart() {
    let f = Fixture::new();
    let root = f.root.to_string_lossy().into_owned();
    let views_dir = f.runtime_dir("hide-document-views-");
    let file_display;
    {
        let mut runtime = f.shared.lock().unwrap();
        studio_connecting(&mut runtime);
        let mut store =
            WorkspaceViewStore::open(views_dir.join("views.json"), Default::default()).0;
        // The file's diff, and the file beside it in use.
        let layout = &mut store.views.entry(DEVICE, &root).layout;
        let diff = layout.new_display(&f.path("a.txt"), DisplayKind::Diff, Some(false), false);
        layout.insert("a1", diff, 1).unwrap();
        let file = layout.new_display(&f.path("a.txt"), DisplayKind::File, None, false);
        file_display = file.id.clone();
        layout.split_new("a1", Edge::Right, file, 2).unwrap();
        runtime.workspace_views = Some(store);
        let connecting = Some("Waiting for studio to connect".to_owned());
        assert_eq!(
            view_displays(&mut runtime),
            vec![
                (
                    DisplayKind::Diff,
                    ViewDisplayState::Waiting,
                    connecting.clone()
                ),
                (
                    DisplayKind::File,
                    ViewDisplayState::Waiting,
                    connecting.clone()
                ),
            ]
        );
        assert!(
            runtime.snapshot.editor.tabs.is_empty(),
            "nothing is restored, or marked unavailable, while the helper connects"
        );
        let retry = serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "view_layout",
            "payload": {
                "workspace": {"device_id": DEVICE, "path": root},
                "action": "retry",
                "display_id": file_display,
            },
        });
        runtime.dispatch_json(&serde_json::to_vec(&retry).unwrap());
        let refused = runtime.snapshot.status.last_error.clone().unwrap();
        assert_eq!(
            (refused.kind.as_str(), Some(refused.message)),
            ("view_layout.waiting", connecting)
        );

        let stale_helper = Some(
            "The device helper speaks protocol 8, this Hide needs 10; the helper this Hide carries does not match it, so rebuild or reinstall Hide"
                .to_owned(),
        );
        runtime.device_hosts.get_mut(DEVICE).unwrap().phase =
            hosts::HostPhase::Unavailable(stale_helper.clone().unwrap());
        assert_eq!(
            view_displays(&mut runtime),
            vec![
                (
                    DisplayKind::Diff,
                    ViewDisplayState::Waiting,
                    stale_helper.clone()
                ),
                (
                    DisplayKind::File,
                    ViewDisplayState::Waiting,
                    stale_helper.clone()
                ),
            ],
            "a refused helper is the device's own reason, and the displays wait for it"
        );
        runtime.dispatch_json(&serde_json::to_vec(&retry).unwrap());
        let refused = runtime.snapshot.status.last_error.clone().unwrap();
        assert_eq!(
            (refused.kind.as_str(), Some(refused.message)),
            ("view_layout.waiting", stale_helper)
        );
        assert!(runtime.snapshot.editor.tabs.is_empty());

        helper_ready(&mut runtime, &f.device);
        runtime.snapshot.status.remote[0].catalog.state = "resolving".to_owned();
        assert!(
            !runtime.restore_front_when_ready(),
            "the device's checkouts are not in their Projects yet"
        );
        let listing = Some("Waiting for studio to list its projects".to_owned());
        assert_eq!(view_displays(&mut runtime)[1].2, listing);
        runtime.snapshot.status.remote[0].catalog.state = "ready".to_owned();
        f.device.hold();
        assert!(runtime.restore_front_when_ready());
        assert_eq!(
            view_displays(&mut runtime),
            vec![
                (DisplayKind::Diff, ViewDisplayState::Open, None),
                (DisplayKind::File, ViewDisplayState::Opening, None),
            ],
            "a diff shows at once and the file is being read"
        );
    }
    f.device.release();
    f.wait_for_document("a.txt", "the restored device file", |_| true);
    let mut runtime = f.shared.lock().unwrap();
    assert_eq!(
        view_displays(&mut runtime),
        vec![
            (DisplayKind::Diff, ViewDisplayState::Open, None),
            (DisplayKind::File, ViewDisplayState::Open, None),
        ]
    );
    let remove = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "remove_device",
        "payload": {"device_id": DEVICE},
    });
    assert!(runtime.dispatch_json(&serde_json::to_vec(&remove).unwrap()));
    assert!(runtime.snapshot.editor.tabs.is_empty());
    assert!(
        runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .views
            .get(DEVICE, &root)
            .is_none()
    );
    assert!(
        runtime
            .snapshot
            .workspace_view
            .as_ref()
            .is_none_or(|view| view.layout.display_count == 0),
        "no display of the removed device is left on screen"
    );
}

/// S7 contract 5, the S6 rule: a Workspace writes nothing into the views
/// file until its restore has landed, so quitting while its displays wait
/// for the device, or while their files are still being read, leaves the
/// stored layout as it was, byte for byte.
#[test]
fn a_quit_mid_restore_keeps_the_stored_layout_byte_for_byte() {
    let f = Fixture::new();
    std::fs::write(f.root.join("b.txt"), "b\n").unwrap();
    let views_dir = f.runtime_dir("hide-document-views-");
    let file = views_dir.join("workspace-views.json");
    // The last session: a.txt's diff and a preview of b.txt in one area,
    // and a.txt in the area beside it, which was in use.
    let mut views = crate::workspace_views::WorkspaceViews::default();
    let entry = views.entry(DEVICE, &f.root.to_string_lossy());
    entry.show_documents();
    let layout = &mut entry.layout;
    let diff = layout.new_display(&f.path("a.txt"), DisplayKind::Diff, Some(false), false);
    layout.insert("a1", diff, 1).unwrap();
    let preview = layout.new_display(&f.path("b.txt"), DisplayKind::File, None, true);
    layout.insert("a1", preview, 2).unwrap();
    let in_use = layout.new_display(&f.path("a.txt"), DisplayKind::File, None, false);
    layout.split_new("a1", Edge::Right, in_use, 3).unwrap();
    if let crate::view_layout::Node::Split(split) = &mut layout.root {
        split.ratio = 0.3;
    }
    crate::workspace_views::save(&file, &views).unwrap();
    let stored = std::fs::read_to_string(&file).unwrap();
    // Quitting joins the file's writer, as dropping the core does.
    let quit = || {
        let writer = f.shared.lock().unwrap().take_workspace_views_save_worker();
        if let Some(writer) = writer {
            writer.join().unwrap();
        }
        std::fs::read_to_string(&file).unwrap()
    };

    {
        let mut runtime = f.shared.lock().unwrap();
        studio_connecting(&mut runtime);
        runtime.workspace_views =
            Some(WorkspaceViewStore::open(file.clone(), Default::default()).0);
        let connecting = Some("Waiting for studio to connect".to_owned());
        assert_eq!(
            view_displays(&mut runtime),
            vec![
                (
                    DisplayKind::Diff,
                    ViewDisplayState::Waiting,
                    connecting.clone()
                ),
                (
                    DisplayKind::File,
                    ViewDisplayState::Waiting,
                    connecting.clone()
                ),
                (DisplayKind::File, ViewDisplayState::Waiting, connecting),
            ]
        );
    }
    assert_eq!(quit(), stored, "nothing is written while the displays wait");

    f.device.hold();
    {
        let mut runtime = f.shared.lock().unwrap();
        helper_ready(&mut runtime, &f.device);
        assert_eq!(
            view_displays(&mut runtime),
            vec![
                (DisplayKind::Diff, ViewDisplayState::Open, None),
                (DisplayKind::File, ViewDisplayState::Opening, None),
                (DisplayKind::File, ViewDisplayState::Opening, None),
            ],
            "the diff shows at once and both files are being read"
        );
    }
    assert_eq!(
        quit(),
        stored,
        "nothing is written while the files are read"
    );
    f.device.release();
    f.wait_for_document("a.txt", "the restored file in use", |_| true);
    f.wait_for_document("b.txt", "the restored preview", |_| true);
}

/// S7 contract 4.2, D-04: reads land on workers, so two can land before
/// anything reconciles the Views between them. A single click whose read
/// lands just after the restore read of the preview it replaces takes that
/// preview's place, and the restored document goes as a replaced preview
/// does, rather than coming back pinned beside the new one.
#[test]
fn a_preview_open_landing_right_after_its_restore_read_replaces_that_preview() {
    let f = Fixture::new();
    std::fs::write(f.root.join("b.txt"), "b\n").unwrap();
    let views_dir = f.runtime_dir("hide-document-views-");
    let file = views_dir.join("workspace-views.json");
    let mut views = crate::workspace_views::WorkspaceViews::default();
    let layout = &mut views.entry(DEVICE, &f.root.to_string_lossy()).layout;
    let restored = layout.new_display(&f.path("b.txt"), DisplayKind::File, None, true);
    layout.insert("a1", restored, 1).unwrap();
    crate::workspace_views::save(&file, &views).unwrap();
    f.device.hold_read("b.txt");
    f.device.hold_read("a.txt");
    {
        let mut runtime = f.shared.lock().unwrap();
        studio_connecting(&mut runtime);
        runtime.workspace_views = Some(WorkspaceViewStore::open(file, Default::default()).0);
        helper_ready(&mut runtime, &f.device);
        assert_eq!(
            view_displays(&mut runtime),
            vec![(DisplayKind::File, ViewDisplayState::Opening, None)]
        );
    }
    f.dispatch(
        "file_open",
        serde_json::json!({
            "path": f.path("a.txt"), "workspace_id": WORKSPACE, "checkout_id": &f.checkout,
            "preview": true,
        }),
    );

    f.device.release_read("b.txt");
    f.wait_for_document("b.txt", "the restore read", |_| true);
    f.device.release_read("a.txt");
    f.wait_for_document("a.txt", "the single click's read", |_| true);

    let mut runtime = f.shared.lock().unwrap();
    runtime.sync_workspace_view();
    let view = runtime.snapshot.workspace_view.clone().unwrap();
    let ViewNodeSnapshot::Area(area) = view.layout.root else {
        panic!("one area");
    };
    let shown: Vec<_> = area
        .displays
        .iter()
        .map(|display| (display.label.as_str(), display.preview))
        .collect();
    assert_eq!(shown, vec![("a.txt", true)]);
    let tabs: Vec<_> = runtime
        .snapshot
        .editor
        .tabs
        .iter()
        .map(|tab| tab.label.as_str())
        .collect();
    assert_eq!(tabs, vec!["a.txt"], "the replaced preview's document goes");
}

/// S7 B2, D-04: a double-click reaches the core as two single clicks and the
/// double-click itself, three opens of one file. The file is still being read
/// when the later ones arrive, so the pin must not be dropped as a repeat of
/// the read already running: the document lands pinned, not as a preview.
#[test]
fn a_double_click_during_the_first_clicks_read_pins_the_document() {
    let f = Fixture::new();
    let views_dir = f.runtime_dir("hide-document-views-");
    {
        let mut runtime = f.shared.lock().unwrap();
        studio_connecting(&mut runtime);
        runtime.workspace_views = Some(
            WorkspaceViewStore::open(views_dir.join("workspace-views.json"), Default::default()).0,
        );
        helper_ready(&mut runtime, &f.device);
    }
    f.device.hold_read("a.txt");
    let click = |preview: bool| {
        f.dispatch(
            "file_open",
            serde_json::json!({
                "path": f.path("a.txt"), "workspace_id": WORKSPACE, "checkout_id": &f.checkout,
                "preview": preview,
            }),
        );
    };
    click(true);
    click(true);
    click(false);
    f.device.release_read("a.txt");
    f.wait_for_document("a.txt", "the read", |_| true);

    let mut runtime = f.shared.lock().unwrap();
    runtime.sync_workspace_view();
    let view = runtime.snapshot.workspace_view.clone().unwrap();
    let ViewNodeSnapshot::Area(area) = view.layout.root else {
        panic!("one area");
    };
    let shown: Vec<_> = area
        .displays
        .iter()
        .map(|display| (display.label.as_str(), display.preview))
        .collect();
    assert_eq!(shown, vec![("a.txt", false)]);
    let tabs: Vec<_> = runtime
        .snapshot
        .editor
        .tabs
        .iter()
        .map(|tab| (tab.label.as_str(), tab.preview))
        .collect();
    assert_eq!(tabs, vec![("a.txt", false)]);
}

/// D10/B18: GUI opens show their result even when the empty column was
/// closed while the node read was outstanding.
#[test]
fn a_gui_file_read_opens_file_views_when_its_document_lands() {
    for preview in [true, false] {
        let f = Fixture::new();
        let views_dir = f.runtime_dir("hide-gui-read-");
        {
            let mut runtime = f.shared.lock().unwrap();
            studio_connecting(&mut runtime);
            helper_ready(&mut runtime, &f.device);
            runtime.workspace_views =
                Some(WorkspaceViewStore::open(views_dir.join("views.json"), Default::default()).0);
        }
        f.device.hold_read("a.txt");
        f.dispatch(
            "file_open",
            serde_json::json!({
                "path": f.path("a.txt"), "workspace_id": WORKSPACE,
                "checkout_id": f.checkout, "preview": preview,
            }),
        );
        f.wait("the held node read", |_| f.device.waiting() > 0);
        f.device.release_read("a.txt");
        f.wait_for_document("a.txt", "the GUI read", |_| true);
        let mut runtime = f.shared.lock().unwrap();
        runtime.sync_workspace_view();
        let view = runtime.snapshot.workspace_view.as_ref().unwrap();
        assert!(
            view.open && view.documents,
            "a successful GUI open must be visible"
        );
        assert_eq!(view.layout.display_count, 1);
        assert_eq!(runtime.visible_view_documents().len(), 1);
    }
}

#[test]
fn a_gui_file_read_cannot_recreate_its_closed_agent_tab() {
    let f = Fixture::new();
    let views_dir = f.runtime_dir("hide-gui-closed-read-");
    let key = (DEVICE.to_owned(), f.root.to_string_lossy().into_owned());
    {
        let mut runtime = f.shared.lock().unwrap();
        studio_connecting(&mut runtime);
        helper_ready(&mut runtime, &f.device);
        let checkout = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0];
        checkout.tabs = ["tab-a", "tab-b"]
            .into_iter()
            .map(|id| {
                let mut held = tab(WORKSPACE, &f.checkout, None);
                held.id = Some(id.to_owned());
                held
            })
            .collect();
        checkout.active_tab_id = Some("tab-a".to_owned());
        runtime.workspace_views =
            Some(WorkspaceViewStore::open(views_dir.join("views.json"), Default::default()).0);
        runtime
            .reconcile_agent_topology(
                &key,
                &[("tab-a".to_owned(), false), ("tab-b".to_owned(), false)],
            )
            .unwrap();
        runtime.sync_workspace_view();
    }
    f.device.hold_read("a.txt");
    f.open("a.txt");
    f.wait("the held node read", |_| f.device.waiting() > 0);
    let before = {
        let mut runtime = f.shared.lock().unwrap();
        let checkout = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0];
        checkout
            .tabs
            .retain(|tab| tab.id.as_deref() != Some("tab-a"));
        checkout.active_tab_id = Some("tab-b".to_owned());
        runtime
            .reconcile_agent_topology(&key, &[("tab-b".to_owned(), false)])
            .unwrap();
        runtime.sync_workspace_view();
        runtime.prune_view_callers(
            &(DEVICE.to_owned(), f.root.to_string_lossy().into_owned()),
            &["tab-b"],
        );
        runtime.sync_workspace_view();
        runtime.snapshot.workspace_view.clone()
    };
    f.device.release_read("a.txt");
    f.wait("the late answer to be dropped", |runtime| {
        diagnostic_count(runtime, "file.open_dropped") == 1
    });
    let mut runtime = f.shared.lock().unwrap();
    runtime.sync_workspace_view();
    assert_eq!(runtime.snapshot.workspace_view, before);
    assert!(runtime.snapshot.editor.tabs.is_empty());
    assert!(runtime.snapshot.editor.opening.is_empty());
}

/// B1/B18: coalescing file I/O preserves each GUI destination, including
/// B's request when A closes before the shared read answers.
#[test]
fn a_coalesced_gui_read_serves_each_live_requesting_tab() {
    for close_a in [false, true] {
        let f = Fixture::new();
        let views_dir = f.runtime_dir("hide-gui-shared-read-");
        {
            let mut runtime = f.shared.lock().unwrap();
            studio_connecting(&mut runtime);
            helper_ready(&mut runtime, &f.device);
            let checkout = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0];
            checkout.tabs = ["tab-a", "tab-b"]
                .into_iter()
                .map(|id| {
                    let mut held = tab(WORKSPACE, &f.checkout, None);
                    held.id = Some(id.to_owned());
                    held
                })
                .collect();
            checkout.active_tab_id = Some("tab-a".to_owned());
            runtime.workspace_views =
                Some(WorkspaceViewStore::open(views_dir.join("views.json"), Default::default()).0);
            runtime.sync_workspace_view();
        }
        f.device.hold_read("a.txt");
        f.open("a.txt");
        f.wait("the held node read", |_| f.device.waiting() > 0);
        {
            let mut runtime = f.shared.lock().unwrap();
            runtime.snapshot.navigator.workspaces[0].checkouts[0].active_tab_id =
                Some("tab-b".to_owned());
            runtime.sync_workspace_view();
        }
        f.open("a.txt");
        if close_a {
            let mut runtime = f.shared.lock().unwrap();
            runtime.snapshot.navigator.workspaces[0].checkouts[0]
                .tabs
                .retain(|tab| tab.id.as_deref() != Some("tab-a"));
            runtime.prune_view_callers(
                &(DEVICE.to_owned(), f.root.to_string_lossy().into_owned()),
                &["tab-b"],
            );
        }
        f.device.release_read("a.txt");
        f.wait("the shared read completion", |runtime| {
            runtime.snapshot.editor.opening.is_empty()
        });
        let mut runtime = f.shared.lock().unwrap();
        runtime.sync_workspace_view();
        let view = runtime.snapshot.workspace_view.as_ref().unwrap();
        assert!(
            view.open && view.documents,
            "B's explicit GUI open must become visible (A closed: {close_a})"
        );
        assert_eq!(view.layout.display_count, 1);
        assert_eq!(runtime.snapshot.editor.tabs.len(), 1, "one shared document");
    }
}

/// B1/B10: a second open may supply the shared buffer while the GUI read is
/// held; the GUI placement still lands without overwriting its dirty draft.
#[test]
fn a_gui_read_places_an_existing_dirty_buffer_in_its_requesting_tab() {
    let f = Fixture::new();
    let views_dir = f.runtime_dir("hide-gui-existing-read-");
    let root = {
        let mut runtime = f.shared.lock().unwrap();
        studio_connecting(&mut runtime);
        helper_ready(&mut runtime, &f.device);
        let checkout = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0];
        checkout.tabs = ["tab-a", "tab-b"]
            .into_iter()
            .map(|id| {
                let mut held = tab(WORKSPACE, &f.checkout, None);
                held.id = Some(id.to_owned());
                held
            })
            .collect();
        checkout.active_tab_id = Some("tab-a".to_owned());
        runtime.workspace_views =
            Some(WorkspaceViewStore::open(views_dir.join("views.json"), Default::default()).0);
        runtime.sync_workspace_view();
        runtime.document_root(WORKSPACE, &f.checkout).unwrap()
    };
    f.device.hold_read("a.txt");
    f.open("a.txt");
    f.wait("the held node read", |_| f.device.waiting() > 0);
    let (document, place) =
        files::open_document(FakeDevice::new().as_ref(), &root, &f.path("a.txt")).unwrap();
    {
        let mut runtime = f.shared.lock().unwrap();
        runtime.snapshot.navigator.workspaces[0].checkouts[0].active_tab_id =
            Some("tab-b".to_owned());
        runtime.sync_workspace_view();
        let tab_id = runtime.new_file_tab_id(WORKSPACE, &f.checkout, &f.path("a.txt"));
        runtime.insert_file_tab(
            super::super::editor::PreparedFileTab::Read {
                tab_id,
                document: Box::new(document),
                place,
            },
            WORKSPACE,
            &f.checkout,
            &f.path("a.txt"),
            false,
        );
    }
    let tab_id = Runtime::file_tab_id(WORKSPACE, &f.checkout, &f.path("a.txt"));
    f.dispatch(
        "file_draft",
        serde_json::json!({"tab_id": tab_id, "contents_utf8": "shared unsaved draft\n"}),
    );
    f.device.release_read("a.txt");
    f.wait("the held GUI completion", |runtime| {
        runtime.snapshot.editor.opening.is_empty()
    });
    let mut runtime = f.shared.lock().unwrap();
    runtime.sync_workspace_view();
    assert_eq!(
        runtime.editor_documents[&tab_id].contents_utf8.as_deref(),
        Some("shared unsaved draft\n")
    );
    assert!(
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .unwrap()
            .dirty
    );
    runtime.snapshot.navigator.workspaces[0].checkouts[0].active_tab_id = Some("tab-a".to_owned());
    runtime.sync_workspace_view();
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert!(
        view.open && view.documents,
        "A's GUI placement must survive an existing buffer"
    );
    assert_eq!(view.layout.display_count, 1);
}

/// B10/B18: two stale New tab destinations refuse independently, without
/// reopening an empty column or keeping an unreferenced document behind.
#[test]
fn a_coalesced_gui_read_cannot_replace_closed_new_tabs() {
    let f = Fixture::new();
    let views_dir = f.runtime_dir("hide-gui-refused-read-");
    {
        let mut runtime = f.shared.lock().unwrap();
        studio_connecting(&mut runtime);
        helper_ready(&mut runtime, &f.device);
        runtime.workspace_views =
            Some(WorkspaceViewStore::open(views_dir.join("views.json"), Default::default()).0);
        runtime.sync_workspace_view();
    }
    f.device.hold_read("a.txt");
    let mut ids = Vec::new();
    for _ in 0..2 {
        f.dispatch(
            "browser_open",
            serde_json::json!({"url": "", "workspace_id": WORKSPACE, "checkout_id": f.checkout}),
        );
        let id = {
            let mut runtime = f.shared.lock().unwrap();
            runtime.sync_workspace_view();
            let ViewNodeSnapshot::Area(area) = &runtime
                .snapshot
                .workspace_view
                .as_ref()
                .unwrap()
                .layout
                .root
            else {
                panic!("one area")
            };
            area.active.clone().unwrap()
        };
        ids.push(id);
        f.open("a.txt");
    }
    f.wait("the held node read", |_| f.device.waiting() > 0);
    for id in ids {
        f.dispatch("view_layout", serde_json::json!({"workspace": {"device_id": DEVICE, "path": f.root.to_string_lossy()}, "action": "close", "display_id": id}));
    }
    f.device.release_read("a.txt");
    f.wait("the refused GUI completion", |runtime| {
        runtime.snapshot.editor.opening.is_empty()
    });
    let mut runtime = f.shared.lock().unwrap();
    runtime.sync_workspace_view();
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert_eq!(view.layout.display_count, 0);
    assert!(runtime.snapshot.editor.tabs.is_empty());
    assert!(runtime.editor_documents.is_empty());
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "view_layout.unknown_display"
    );
}

/// B15/D12: New tab churn cannot turn one held file read into an unbounded
/// queue of GUI destinations, even though each replaces an existing slot:
/// the checkout's cap counts the destinations still waiting for the read.
#[test]
fn a_coalesced_gui_read_refuses_a_destination_past_the_views_cap() {
    let f = Fixture::new();
    let views_dir = f.runtime_dir("hide-gui-bounded-read-");
    {
        let mut runtime = f.shared.lock().unwrap();
        studio_connecting(&mut runtime);
        helper_ready(&mut runtime, &f.device);
        runtime.workspace_views =
            Some(WorkspaceViewStore::open(views_dir.join("views.json"), Default::default()).0);
        runtime.sync_workspace_view();
    }
    f.device.hold_read("a.txt");
    for n in 1..=crate::view_layout::MAX_VIEW_DISPLAYS + 1 {
        f.dispatch(
            "browser_open",
            serde_json::json!({"url": "", "workspace_id": WORKSPACE, "checkout_id": f.checkout}),
        );
        let id = {
            let mut runtime = f.shared.lock().unwrap();
            runtime.sync_workspace_view();
            let ViewNodeSnapshot::Area(area) = &runtime
                .snapshot
                .workspace_view
                .as_ref()
                .unwrap()
                .layout
                .root
            else {
                panic!("one area")
            };
            area.active.clone().unwrap()
        };
        f.open("a.txt");
        {
            let runtime = f.shared.lock().unwrap();
            if n <= crate::view_layout::MAX_VIEW_DISPLAYS {
                assert!(
                    runtime.snapshot.status.last_error.is_none(),
                    "destination {n} is admitted"
                );
            } else {
                assert_eq!(
                    runtime.snapshot.status.last_error.as_ref().unwrap().kind,
                    "view_layout.display_limit"
                );
            }
        }
        f.dispatch("view_layout", serde_json::json!({"workspace": {"device_id": DEVICE, "path": f.root.to_string_lossy()}, "action": "close", "display_id": id}));
    }
    f.device.release_read("a.txt");
    f.wait("the bounded read completion", |runtime| {
        runtime.snapshot.editor.opening.is_empty()
    });
    let runtime = f.shared.lock().unwrap();
    assert!(runtime.snapshot.editor.tabs.is_empty());
    assert!(runtime.editor_documents.is_empty());
}

/// B1/B11/B22: retiring B's clean buffer cannot cancel A's still-admitted
/// GUI destination, even when B closed the last actual display of the file.
#[test]
fn a_gui_read_survives_another_tabs_clean_buffer_retirement() {
    let f = Fixture::new();
    let views_dir = f.runtime_dir("hide-gui-retired-read-");
    let root = {
        let mut runtime = f.shared.lock().unwrap();
        studio_connecting(&mut runtime);
        helper_ready(&mut runtime, &f.device);
        let checkout = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0];
        checkout.tabs = ["tab-a", "tab-b"]
            .into_iter()
            .map(|id| {
                let mut held = tab(WORKSPACE, &f.checkout, None);
                held.id = Some(id.to_owned());
                held
            })
            .collect();
        checkout.active_tab_id = Some("tab-a".to_owned());
        runtime.workspace_views =
            Some(WorkspaceViewStore::open(views_dir.join("views.json"), Default::default()).0);
        runtime.sync_workspace_view();
        runtime.document_root(WORKSPACE, &f.checkout).unwrap()
    };
    f.device.hold_read("a.txt");
    f.open("a.txt");
    f.wait("the held node read", |_| f.device.waiting() > 0);
    let (document, place) =
        files::open_document(FakeDevice::new().as_ref(), &root, &f.path("a.txt")).unwrap();
    {
        let mut runtime = f.shared.lock().unwrap();
        runtime.snapshot.navigator.workspaces[0].checkouts[0].active_tab_id =
            Some("tab-b".to_owned());
        runtime.sync_workspace_view();
        let tab_id = runtime.new_file_tab_id(WORKSPACE, &f.checkout, &f.path("a.txt"));
        runtime.insert_file_tab(
            super::super::editor::PreparedFileTab::Read {
                tab_id,
                document: Box::new(document),
                place,
            },
            WORKSPACE,
            &f.checkout,
            &f.path("a.txt"),
            false,
        );
    }
    f.open("a.txt");
    let id = {
        let mut runtime = f.shared.lock().unwrap();
        runtime.sync_workspace_view();
        let ViewNodeSnapshot::Area(area) = &runtime
            .snapshot
            .workspace_view
            .as_ref()
            .unwrap()
            .layout
            .root
        else {
            panic!("one area")
        };
        area.active.clone().unwrap()
    };
    f.dispatch("view_layout", serde_json::json!({"workspace": {"device_id": DEVICE, "path": f.root.to_string_lossy()}, "action": "close", "display_id": id}));
    f.device.release_read("a.txt");
    f.wait("the original GUI read completion", |runtime| {
        runtime.snapshot.editor.opening.is_empty()
    });
    let mut runtime = f.shared.lock().unwrap();
    runtime.sync_workspace_view();
    runtime.snapshot.navigator.workspaces[0].checkouts[0].active_tab_id = Some("tab-a".to_owned());
    runtime.sync_workspace_view();
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    assert!(
        view.open && view.documents,
        "A still owns its accepted GUI open"
    );
    assert_eq!(view.layout.display_count, 1);
    assert_eq!(runtime.snapshot.editor.tabs.len(), 1);
}

/// S7 B23: this machine and a device can hold a checkout at the same path,
/// and each is its own Workspace: the file opened in each is two documents,
/// each shown by its own Workspace's view, so a draft in one leaves the
/// other as it was.
#[test]
fn the_same_path_on_two_devices_is_two_documents_in_two_views() {
    let f = Fixture::new();
    let path = f.root.to_string_lossy().into_owned();
    let (local_id, local_checkout) = ("workspace:local-same-path", "checkout:local-same-path");
    let views_dir = f.runtime_dir("hide-document-views-");
    {
        let mut runtime = f.shared.lock().unwrap();
        studio_connecting(&mut runtime);
        helper_ready(&mut runtime, &f.device);
        runtime.snapshot.navigator.workspaces.push(workspace(
            local_id,
            "Local",
            &path,
            vec![checkout(local_id, local_checkout, &path, None)],
        ));
        let opened = std::fs::File::open(&f.root).unwrap();
        runtime.set_file_roots(crate::files::FileRoots::from_identities(
            hide_node::hold_roots(vec![(f.root.clone(), opened)]).1,
        ));
        runtime.workspace_views =
            Some(WorkspaceViewStore::open(views_dir.join("views.json"), Default::default()).0);
        runtime.sync_workspace_view();
    }
    let device_tab = Runtime::file_tab_id(WORKSPACE, &f.checkout, &f.path("a.txt"));
    let local_tab = Runtime::file_tab_id(local_id, local_checkout, &f.path("a.txt"));
    f.open("a.txt");
    f.wait("the device's document", |runtime| {
        runtime.editor_documents.contains_key(&device_tab)
    });
    {
        let mut runtime = f.shared.lock().unwrap();
        runtime.snapshot.navigator.focused_workspace_id = Some(local_id.to_owned());
        runtime.snapshot.navigator.focused_checkout_id = Some(local_checkout.to_owned());
        runtime.sync_workspace_view();
    }
    f.dispatch(
        "file_open",
        serde_json::json!({"path": f.path("a.txt"), "workspace_id": local_id, "checkout_id": local_checkout}),
    );
    f.wait("this machine's document", |runtime| {
        runtime.editor_documents.contains_key(&local_tab)
    });
    assert_ne!(device_tab, local_tab);

    f.dispatch(
        "file_draft",
        serde_json::json!({"tab_id": local_tab, "contents_utf8": "local draft\n"}),
    );
    let mut runtime = f.shared.lock().unwrap();
    runtime.sync_workspace_view();
    let bound = |device: &str| {
        runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .views
            .get(device, &path)
            .map(|view| {
                view.layout
                    .displays()
                    .map(|display| display.tab_id.clone())
                    .collect::<Vec<_>>()
            })
    };
    assert_eq!(bound(DEVICE), Some(vec![Some(device_tab.clone())]));
    assert_eq!(
        bound(crate::node::TEST_NODE),
        Some(vec![Some(local_tab.clone())])
    );
    let dirty = |tab_id: &str| {
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .map(|tab| tab.dirty)
    };
    assert_eq!(
        (dirty(&local_tab), dirty(&device_tab)),
        (Some(true), Some(false))
    );
    assert_eq!(
        runtime.editor_documents[&device_tab]
            .contents_utf8
            .as_deref(),
        Some("old\n")
    );
}

/// S7 contract 3, B16: a view whose file could not be read shows `opening`
/// while Retry reads it again, not its old failure with Retry still offered,
/// and `open` once the read lands.
#[test]
fn a_retried_view_shows_opening_while_its_read_is_in_flight() {
    let f = Fixture::new();
    let root = f.root.to_string_lossy().into_owned();
    let views_dir = f.runtime_dir("hide-document-views-");
    let file = views_dir.join("workspace-views.json");
    let mut views = crate::workspace_views::WorkspaceViews::default();
    let layout = &mut views.entry(DEVICE, &root).layout;
    let restored = layout.new_display(&f.path("gone.txt"), DisplayKind::File, None, false);
    let restored_id = restored.id.clone();
    layout.insert("a1", restored, 1).unwrap();
    crate::workspace_views::save(&file, &views).unwrap();
    {
        let mut runtime = f.shared.lock().unwrap();
        studio_connecting(&mut runtime);
        runtime.workspace_views = Some(WorkspaceViewStore::open(file, Default::default()).0);
        helper_ready(&mut runtime, &f.device);
        runtime.sync_workspace_view();
    }
    f.wait("the restore read to fail", |runtime| {
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.unavailable_reason.is_some())
    });
    let state = || {
        let mut runtime = f.shared.lock().unwrap();
        let shown = view_displays(&mut runtime);
        let [(_, state, reason)] = shown.as_slice() else {
            panic!("one view: {shown:?}");
        };
        (*state, reason.is_some())
    };
    assert_eq!(state(), (ViewDisplayState::Unavailable, true));

    std::fs::write(f.root.join("gone.txt"), "back\n").unwrap();
    f.device.hold_read("gone.txt");
    f.dispatch(
        "view_layout",
        serde_json::json!({
            "workspace": {"device_id": DEVICE, "path": root},
            "action": "retry", "display_id": restored_id,
        }),
    );
    assert_eq!(state(), (ViewDisplayState::Opening, false));

    f.device.release_read("gone.txt");
    f.wait_for_document("gone.txt", "the retried read", |_| true);
    assert_eq!(state(), (ViewDisplayState::Open, false));
}

impl Fixture {
    fn explorer(&self, kind: &str, mut payload: Value) {
        payload["root"] = Value::from(self.root_text());
        payload["device_id"] = Value::from(DEVICE);
        self.dispatch(kind, payload);
    }

    /// Waits until the device holds the running change, so what is asked
    /// next arrives while it is still working.
    fn wait_held(&self) {
        self.wait("the change to reach the device", |_| {
            self.device.waiting() > 0
        });
    }

    fn settled(&self, kind: &str, path: &str, phase: &str) {
        self.wait(&format!("{kind} of {path} to be {phase}"), |runtime| {
            runtime
                .snapshot
                .explorer_operation
                .as_ref()
                .is_some_and(|operation| {
                    operation.kind == kind && operation.path == path && operation.phase == phase
                })
        });
    }

    fn root_text(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    fn diagnostics(&self, kind: &str) -> usize {
        let runtime = self.shared.lock().unwrap();
        runtime
            .snapshot
            .status
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.kind == kind)
            .count()
    }
}

/// An Explorer change asked for while another one runs waits for it and
/// then runs, in order: a folder trashed right after a file was dragged into
/// it goes once the move lands, where it used to be refused as busy with
/// nothing logged and nothing on screen (#630).
#[test]
fn a_trash_asked_while_a_move_runs_waits_for_the_move_and_then_runs() {
    let f = Fixture::new();
    let (_, folder_name) = crate::files::tests::unique_trash_names();
    let folder = f.root.join(&folder_name);
    std::fs::create_dir(&folder).unwrap();

    f.device.hold();
    f.explorer(
        "path_move",
        serde_json::json!({"path": f.path("a.txt"), "destination": f.path(&folder_name)}),
    );
    f.wait_held();
    f.explorer(
        "path_trash",
        serde_json::json!({"path": f.path(&folder_name), "select_after": f.root_text()}),
    );
    f.device.release();

    f.settled("path_trash", &f.path(&folder_name), "finished");
    assert!(
        !folder.exists(),
        "the folder went, with the moved file in it"
    );
    assert!(!f.root.join("a.txt").exists());
    crate::files::tests::remove_from_trash(&[&folder_name]);
}

/// A copy lands beside its source under the host's `copy` name, selects what
/// exists now rather than the name the page expected, and leaves the open tab
/// of its source where it was: a copy adds an item and moves none.
#[test]
fn a_copy_takes_the_host_chosen_name_and_moves_no_open_tab() {
    let f = Fixture::new();
    f.open_and_wait("a.txt");
    f.explorer(
        "path_copy",
        serde_json::json!({"path": f.path("a.txt"), "destination": f.root_text()}),
    );
    f.settled("path_copy", &f.path("a.txt"), "finished");
    assert_eq!(
        std::fs::read_to_string(f.root.join("a copy.txt")).unwrap(),
        "old\n"
    );
    let runtime = f.shared.lock().unwrap();
    let slot = runtime.snapshot.explorer_operation.as_ref().unwrap();
    assert_eq!(slot.destination, f.path("a copy.txt"));
    assert_eq!(
        runtime.snapshot.ui_state.selected_path.as_deref(),
        Some(f.path("a copy.txt").as_str())
    );
    assert!(
        runtime
            .snapshot
            .editor
            .tabs
            .iter()
            .all(|tab| tab.path == f.path("a.txt")),
        "the source's tab still shows the source"
    );
}

/// A copy that cannot be made says so at the folder it was pasted into and
/// leaves nothing behind.
#[test]
fn a_copy_into_a_missing_folder_fails_where_it_was_pasted_and_adds_nothing() {
    let f = Fixture::new();
    f.explorer(
        "path_copy",
        serde_json::json!({"path": f.path("a.txt"), "destination": f.path("nowhere")}),
    );
    f.settled("path_copy", &f.path("a.txt"), "failed");
    let runtime = f.shared.lock().unwrap();
    let slot = runtime.snapshot.explorer_operation.as_ref().unwrap();
    assert!(slot.failure.is_some());
    drop(runtime);
    let names: Vec<_> = std::fs::read_dir(&f.root)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "only a.txt is there");
}

/// A drop writes the staged files into the folder under free names and
/// selects the first item; the staged bytes are read through the core's own
/// node, the machine hided staged them on.
#[test]
fn a_drop_is_written_from_its_staged_files_under_free_names() {
    let f = Fixture::new();
    let staging = tempfile::tempdir().unwrap();
    let staged = staging.path().join("one.txt");
    std::fs::write(&staged, "dropped").unwrap();
    f.explorer(
        "path_import",
        serde_json::json!({
            "destination": f.root_text(),
            "items": [
                {"path": "a.txt", "directory": false, "staged": staged.to_string_lossy()},
                {"path": "dir", "directory": true},
            ],
        }),
    );
    f.wait("the drop to finish", |runtime| {
        runtime
            .snapshot
            .explorer_operation
            .as_ref()
            .is_some_and(|operation| {
                operation.kind == "path_import" && operation.phase == "finished"
            })
    });
    assert_eq!(
        std::fs::read_to_string(f.root.join("a copy.txt")).unwrap(),
        "dropped"
    );
    assert_eq!(
        std::fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "old\n"
    );
    assert!(f.root.join("dir").is_dir());
    let runtime = f.shared.lock().unwrap();
    assert_eq!(
        runtime.snapshot.ui_state.selected_path.as_deref(),
        Some(f.path("a copy.txt").as_str())
    );
}

/// At most four changes wait behind the running one; the newest past that is
/// refused and logged, and the four run in the order they were asked.
#[test]
fn explorer_changes_past_four_waiting_are_refused_and_logged() {
    let f = Fixture::new();
    f.device.hold();
    f.explorer(
        "dir_create",
        serde_json::json!({"parent": f.root_text(), "name": "d0"}),
    );
    f.wait_held();
    for name in ["d1", "d2", "d3", "d4", "d5"] {
        f.explorer(
            "dir_create",
            serde_json::json!({"parent": f.root_text(), "name": name}),
        );
    }
    assert_eq!(
        f.diagnostics("explorer.busy"),
        1,
        "the fifth waiting change is refused"
    );
    f.device.release();

    f.settled("dir_create", &f.path("d4"), "finished");
    for name in ["d0", "d1", "d2", "d3", "d4"] {
        assert!(f.root.join(name).is_dir(), "{name} was made");
    }
    assert!(
        !f.root.join("d5").exists(),
        "the refused change made nothing"
    );
}

/// A waiting change is planned again when its turn comes: one whose item
/// went while it waited fails at its row like any change, and the changes
/// behind it are dropped with one diagnostic rather than run past a failure
/// the operator has not seen.
#[test]
fn a_waiting_change_whose_item_went_fails_at_its_row_and_drops_the_rest() {
    let f = Fixture::new();
    std::fs::write(f.root.join("b.txt"), "b\n").unwrap();
    f.device.hold();
    f.explorer(
        "dir_create",
        serde_json::json!({"parent": f.root_text(), "name": "d0"}),
    );
    f.wait_held();
    f.explorer(
        "path_rename",
        serde_json::json!({"path": f.path("b.txt"), "name": "c.txt"}),
    );
    f.explorer(
        "dir_create",
        serde_json::json!({"parent": f.root_text(), "name": "d1"}),
    );
    std::fs::remove_file(f.root.join("b.txt")).unwrap();
    f.device.release();

    f.settled("path_rename", &f.path("b.txt"), "failed");
    let message = f
        .shared
        .lock()
        .unwrap()
        .snapshot
        .explorer_operation
        .as_ref()
        .unwrap()
        .failure
        .clone();
    assert!(message.is_some(), "the failure says why");
    assert!(f.root.join("d0").is_dir());
    assert!(!f.root.join("c.txt").exists());
    assert!(!f.root.join("d1").exists(), "nothing runs past the failure");
    assert_eq!(f.diagnostics("explorer.queue_dropped"), 1);
}

/// Waiting changes belong to the checkout that was in front when they were
/// asked; once another one is in front they are dropped with one
/// diagnostic, and the running change still lands.
#[test]
fn waiting_explorer_changes_are_dropped_when_the_checkout_in_front_changes() {
    let f = Fixture::new();
    f.device.hold();
    f.explorer(
        "dir_create",
        serde_json::json!({"parent": f.root_text(), "name": "d0"}),
    );
    f.wait_held();
    f.explorer(
        "dir_create",
        serde_json::json!({"parent": f.root_text(), "name": "d1"}),
    );
    f.shared
        .lock()
        .unwrap()
        .snapshot
        .navigator
        .focused_checkout_id = Some("checkout:elsewhere".to_owned());
    f.device.release();

    f.settled("dir_create", &f.path("d0"), "finished");
    f.wait("the waiting change to be dropped", |runtime| {
        runtime
            .snapshot
            .status
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == "explorer.queue_dropped")
    });
    assert!(f.root.join("d0").is_dir(), "the running change landed");
    assert!(
        !f.root.join("d1").exists(),
        "the dropped change made nothing"
    );
    assert_eq!(f.diagnostics("explorer.queue_dropped"), 1);
}
