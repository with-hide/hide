//! The projection of the engine's session, read whole on each wake.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::engine::{EngineError, SharedEngine, Wait};
use crate::handle::ChangeNotifier;
use crate::live::{LiveContext, SessionFetchError};
use crate::model::{
    CheckoutSnapshot, PaneSnapshot, RemotePaneLayoutFrame, RemotePaneLayoutSnapshot,
    RemoteSessionSnapshot, StripTabSnapshot, TabSnapshot, WorkspaceRegistration, WorkspaceSnapshot,
};
use crate::runtime::Runtime;
use crate::sidebar::{
    SessionAgentPayload, SessionLayoutPayload, SessionPanePayload, SessionSnapshotPayload,
    SessionTabPayload, SessionWorkspacePayload,
};
use crate::workspace;

const SYNC_REQUEST_TIMEOUT: Duration = Duration::from_secs(1);
const REMOTE_SYNC_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const ASYNC_OPERATION_TICK_INTERVAL: Duration = Duration::from_millis(250);
const CATALOG_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const RECONNECT_INITIAL_DELAY: Duration = Duration::from_millis(100);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(5);

/// A workspace catalog built outside the runtime mutex, together with the
/// registrations it came from so Runtime can reject a stale computation.
pub struct PrecomputedCatalog {
    pub registrations: Vec<WorkspaceRegistration>,
    pub worktrees: crate::model::WorktreeCatalogSnapshot,
    pub workspaces: Vec<WorkspaceSnapshot>,
    /// What the core's own node said about every path the catalog and the
    /// reconcile read, so neither reads a folder under the runtime lock.
    pub paths: Arc<workspace::PathIndex>,
}

#[cfg(test)]
impl PrecomputedCatalog {
    /// The catalog the sync coordinator would hand the runtime for
    /// `payload`, with its paths answered by the node in this process.
    pub fn here(
        registrations: Vec<WorkspaceRegistration>,
        payload: &crate::sidebar::SessionSnapshotPayload,
        worktrees: &crate::model::WorktreeCatalogSnapshot,
    ) -> Self {
        let node = crate::node::test_node();
        let mut wanted = workspace::PathIndex::wanted(&node, &registrations, &[], worktrees);
        wanted.extend(Runtime::session_paths(payload));
        let paths = workspace::paths_here(wanted);
        let spaces = Runtime::session_spaces(payload, &paths);
        Self {
            worktrees: worktrees.clone(),
            workspaces: workspace::build_catalog(&node, &registrations, &spaces, worktrees, &paths),
            registrations,
            paths: Arc::new(paths),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SessionSyncTarget {
    Local { socket_path: PathBuf },
    Remote { target_id: String, label: String },
}

#[derive(Clone)]
pub(crate) struct SessionSyncContext {
    target: SessionSyncTarget,
    /// The link to the node this target's machine work goes to, for a
    /// target the core reads that work from (PRD core-host-node D-21).
    node: Option<Arc<dyn crate::node_access::NodeLink>>,
    /// The link a node that dials this core brought, which its machine's
    /// readers ask (PRD core-host-node-remote-core B13); `None` for this
    /// machine, whose readers ask `node`, and for an SSH device.
    inbound: Option<Arc<dyn crate::node_access::NodeLink>>,
    /// This machine's own node, where `gh` runs with the operator's login,
    /// for every device: its pull requests are read here by repository name
    /// (PRD core-host-node-remote-core D-16), whether its node dials this
    /// core or this core dials it.
    login: Option<Arc<dyn crate::node_access::NodeLink>>,
    engine: SharedEngine,
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
}

impl SessionSyncContext {
    pub(crate) fn local(
        context: &LiveContext,
        node: Arc<dyn crate::node_access::NodeLink>,
    ) -> Self {
        Self {
            target: SessionSyncTarget::Local {
                socket_path: context.socket_path.clone(),
            },
            node: Some(node),
            inbound: None,
            login: None,
            engine: Arc::clone(&context.engine),
            runtime: context.runtime.clone(),
            notifier: context.notifier.clone(),
        }
    }

    /// A device's coordinator; `login` is this machine's own node, where the
    /// device's pull requests are read.
    pub(crate) fn remote(
        target_id: impl Into<String>,
        label: impl Into<String>,
        engine: SharedEngine,
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
        login: Arc<dyn crate::node_access::NodeLink>,
    ) -> Self {
        Self {
            target: SessionSyncTarget::Remote {
                target_id: target_id.into(),
                label: label.into(),
            },
            node: None,
            inbound: None,
            login: Some(login),
            engine,
            runtime,
            notifier,
        }
    }

    /// A device whose node dials this core: its machine is read through
    /// `link`, as this machine is through its own node.
    pub(crate) fn dialed_in(mut self, link: Arc<dyn crate::node_access::NodeLink>) -> Self {
        self.inbound = Some(link);
        self
    }

    fn log_target(&self) -> &str {
        match &self.target {
            SessionSyncTarget::Local { .. } => "local",
            SessionSyncTarget::Remote { target_id, .. } => target_id,
        }
    }

    /// The machine this target's agents are recorded under in the delivery
    /// ledger: the core's own node for the local target, the device id for a
    /// remote one. `log_target` names the target in diagnostics and is no
    /// ledger key, because the local target's records carry the node id and
    /// never `"local"` (issue 772).
    fn ledger_machine(&self) -> Result<String, String> {
        match &self.target {
            SessionSyncTarget::Local { .. } => {
                let runtime = self
                    .runtime
                    .upgrade()
                    .ok_or_else(|| "the runtime has ended".to_owned())?;
                let guard = runtime
                    .lock()
                    .map_err(|_| "the runtime lock is poisoned".to_owned())?;
                Ok(guard.node().to_string())
            }
            SessionSyncTarget::Remote { target_id, .. } => Ok(target_id.clone()),
        }
    }

    fn is_local(&self) -> bool {
        matches!(self.target, SessionSyncTarget::Local { .. })
    }

    fn request_timeout(&self) -> Duration {
        match self.target {
            SessionSyncTarget::Local { .. } => SYNC_REQUEST_TIMEOUT,
            SessionSyncTarget::Remote { .. } => REMOTE_SYNC_REQUEST_TIMEOUT,
        }
    }

    /// The target the runtime keeps this connection's read view under:
    /// none for this machine's server, the device's id for a device's.
    fn read_target(&self) -> Option<&str> {
        match &self.target {
            SessionSyncTarget::Local { .. } => None,
            SessionSyncTarget::Remote { target_id, .. } => Some(target_id),
        }
    }

    fn node(&self) -> Option<&Arc<dyn crate::node_access::NodeLink>> {
        self.node.as_ref()
    }

    /// The node `gh` runs on for this target's pull requests: this
    /// machine's own, for this machine and for every device.
    fn login(&self) -> Option<&Arc<dyn crate::node_access::NodeLink>> {
        match &self.target {
            SessionSyncTarget::Local { .. } => self.node.as_ref(),
            SessionSyncTarget::Remote { .. } => self.login.as_ref(),
        }
    }

    /// The node this target's own machine is read through: this machine's
    /// node, or the link of a node that dials this core.
    fn machine(&self) -> Option<&Arc<dyn crate::node_access::NodeLink>> {
        match &self.target {
            SessionSyncTarget::Local { .. } => self.node.as_ref(),
            SessionSyncTarget::Remote { .. } => self.inbound.as_ref(),
        }
    }
}

pub(crate) struct CatalogCache {
    registrations: Vec<WorkspaceRegistration>,
    spaces: Vec<workspace::SessionSpace>,
    /// The worktrees the cached catalog was built from. Without this a new
    /// worktree, a merge, or a commit would never reach the sidebar: the
    /// registrations and spaces would compare equal and the stale rows would
    /// be republished unchanged.
    worktrees: crate::model::WorktreeCatalogSnapshot,
    workspaces: Vec<WorkspaceSnapshot>,
    /// The paths `paths` answered, so a publish asking the same ones within
    /// `CATALOG_REFRESH_INTERVAL` reuses the answer; empty after a failed ask.
    paths: Arc<workspace::PathIndex>,
    built_at: Instant,
    /// The unconfirmed created purposes the purpose mirror last synced this
    /// catalog with; `None` until it has. A publish that changed neither,
    /// such as an agent status change, has no purpose to mirror (PRD
    /// instant-pane-topology D-18).
    purposes_synced: Option<HashMap<String, String>>,
}

#[derive(Default)]
struct CatalogState {
    cache: Option<CatalogCache>,
    reader: Option<path_facts::PathReader>,
    connection: u64,
    wake: Option<RepublishWaker>,
}

impl CatalogState {
    fn as_ref(&self) -> Option<&CatalogCache> {
        self.cache.as_ref()
    }
    fn as_mut(&mut self) -> Option<&mut CatalogCache> {
        self.cache.as_mut()
    }
}

pub(crate) struct SessionSyncHandle {
    sender: Sender<CoordinatorMessage>,
    republish: Arc<RepublishFlags>,
    worker: Option<JoinHandle<()>>,
}

impl SessionSyncHandle {
    pub(crate) fn republish_waker(&self) -> RepublishWaker {
        RepublishWaker::asking(self.sender.clone(), Arc::clone(&self.republish))
    }
}

impl Drop for SessionSyncHandle {
    fn drop(&mut self) {
        let _ = self.sender.send(CoordinatorMessage::Stop);
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            crate::diagnostic!(json!({
                "component": "session_sync",
                "kind": "coordinator.join_failed",
            }));
        }
    }
}

pub(crate) enum CoordinatorMessage {
    Stop,
    /// The label worker has a read or an analysis result to take.
    Labels,
    /// The runtime drew something ahead of Herdr, or took it back, and the
    /// session has to be read again with it (`RepublishWaker`).
    Republish,
    /// Read the session again after a multi-step topology transaction.
    RefreshTopology,
}

/// What the coordinator owes the wakes it has not taken yet.
#[derive(Default)]
pub(crate) struct RepublishFlags {
    /// A `Republish` is in the mailbox, so a later wake sends none.
    pending: AtomicBool,
    /// A wake whose reason the engine's session does not carry asked for the
    /// next publish (`RepublishWaker::asking`); without one, a read equal to
    /// the last one published is not published again.
    asked: AtomicBool,
}

impl RepublishFlags {
    /// Takes the wakes so far, before the session is read, so a wake that
    /// lands during the publish asks for the next one rather than being
    /// lost; answers whether one of them asked for a publish.
    fn take(&self) -> bool {
        self.pending.store(false, Ordering::Release);
        self.asked.swap(false, Ordering::AcqRel)
    }
}

/// Wakes the local coordinator to publish its session again at once. The
/// engine wakes it when its session changed; the core when it has drawn a
/// created tab or a predicted pane geometry ahead of Herdr, or taken one
/// back, and the overlay rides the next ingest (PRD instant-pane-topology
/// D-05, D-07), or when anything else it lays on a publish moved. Wakes that
/// arrive before the coordinator takes the first are one publish, so a burst
/// cannot queue publishes.
#[derive(Clone)]
pub(crate) struct RepublishWaker {
    sender: Sender<CoordinatorMessage>,
    flags: Arc<RepublishFlags>,
    /// This wake's reason is not in the engine's session, so the publish it
    /// asks for runs even when the session read equals the last one.
    asks: bool,
}

impl RepublishWaker {
    /// A waker for the core's own reasons: it is always published.
    fn asking(sender: Sender<CoordinatorMessage>, flags: Arc<RepublishFlags>) -> Self {
        Self {
            sender,
            flags,
            asks: true,
        }
    }

    /// The engine's waker: its session changed, and a read equal to the last
    /// one published (a change the session does not show) publishes nothing.
    fn engine(sender: Sender<CoordinatorMessage>, flags: Arc<RepublishFlags>) -> Self {
        Self {
            sender,
            flags,
            asks: false,
        }
    }

    pub(crate) fn refresh_topology(&self) -> bool {
        self.sender
            .send(CoordinatorMessage::RefreshTopology)
            .is_ok()
    }
    pub(crate) fn wake(&self) {
        if self.asks {
            self.flags.asked.store(true, Ordering::Release);
        }
        if !self.flags.pending.swap(true, Ordering::AcqRel) {
            let _ = self.sender.send(CoordinatorMessage::Republish);
        }
    }
}

mod coordinator;
mod path_facts;
mod projection;
mod pulled;

pub(crate) use coordinator::spawn;
pub(crate) use projection::{
    ProjectedAgent, ProjectedPane, ProjectedTab, ProjectedWorkspace, ProjectedWorktree,
    ProjectionState,
};

/// A worker's authoritative read uses the same device projection as the replica.
pub(crate) fn project_remote_read(
    state: ProjectionState,
    device: &str,
) -> Result<(SessionSnapshotPayload, RemoteSessionSnapshot), SessionFetchError> {
    let payload = state.project();
    let (remote, _) = state.project_remote(device, payload.clone())?;
    Ok((payload, remote))
}
