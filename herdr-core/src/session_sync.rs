//! Event-driven projection of Herdr's authoritative session state.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::wire::{self, parse_subscription_line};

use crate::handle::ChangeNotifier;
use crate::live::{LiveContext, SessionFetchError};
use crate::model::{
    CheckoutSnapshot, PaneSnapshot, RemotePaneLayoutFrame, RemotePaneLayoutSnapshot,
    RemoteSessionSnapshot, StripTabSnapshot, TabSnapshot, WorkspaceRegistration, WorkspaceSnapshot,
};
use crate::runtime::{HerdrArrival, Runtime};
use crate::sidebar::{
    SessionAgentPayload, SessionLayoutPayload, SessionPanePayload, SessionSnapshotPayload,
    SessionTabPayload, SessionWorkspacePayload,
};
use crate::workspace;
use hide_herdr_client::{self, ApiConnector, ApiError};

const SYNC_REQUEST_TIMEOUT: Duration = Duration::from_secs(1);
/// How long after the bootstrap snapshot arrives an event read off the
/// stream is still reconciled against it rather than applied strictly
/// (`ApplyMode`). A line the reader thread had already pulled off the
/// socket before the snapshot answered is stamped earlier than this, so
/// the grace only has to cover the reader being scheduled late.
const RECONCILE_GRACE: Duration = Duration::from_secs(1);
const AGENT_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
/// The least time between two agent lists asked for because the session
/// moved while one was out; a steady event stream re-asks at this pace.
const AGENT_REASK_SPACING: Duration = Duration::from_millis(250);
const ASYNC_OPERATION_TICK_INTERVAL: Duration = Duration::from_millis(250);
/// Herdr events kept between two publishes for the operation stage records;
/// a burst longer than this keeps its newest events.
const ARRIVAL_BUFFER_LIMIT: usize = 256;
/// How many `workspace.get` reads a workspace waiting for its replacement
/// active tab gets, one per event that leaves it waiting and one per
/// operation tick, before the replica is declared stale and rebuilt from a
/// fresh snapshot. Eight ticks is two seconds, longer than any event the
/// read can have run ahead of takes to arrive.
const ACTIVE_TAB_READ_ATTEMPT_LIMIT: u32 = 8;
const CATALOG_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const RECONNECT_INITIAL_DELAY: Duration = Duration::from_millis(100);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(5);

const TOPOLOGY_SUBSCRIPTIONS: &[&str] = &[
    "workspace.created",
    "workspace.updated",
    "workspace.metadata_updated",
    "workspace.renamed",
    "workspace.moved",
    "workspace.reordered",
    "workspace.closed",
    "workspace.focused",
    "worktree.created",
    "worktree.opened",
    "worktree.removed",
    "tab.created",
    "tab.closed",
    "tab.focused",
    "tab.renamed",
    "tab.moved",
    "pane.created",
    "pane.closed",
    "pane.updated",
    "pane.focused",
    "pane.moved",
    "pane.exited",
    "pane.agent_detected",
    "layout.updated",
];

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
        wanted.extend(Runtime::session_cwds(payload));
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
    /// for a node that dials this core: its pull requests are read here by
    /// repository name (PRD core-host-node-remote-core D-16).
    login: Option<Arc<dyn crate::node_access::NodeLink>>,
    api_connector: Arc<dyn ApiConnector>,
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
            api_connector: Arc::clone(&context.api_connector),
            runtime: context.runtime.clone(),
            notifier: context.notifier.clone(),
        }
    }

    pub(crate) fn remote(
        target_id: impl Into<String>,
        label: impl Into<String>,
        api_connector: Arc<dyn ApiConnector>,
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
    ) -> Self {
        Self {
            target: SessionSyncTarget::Remote {
                target_id: target_id.into(),
                label: label.into(),
            },
            node: None,
            inbound: None,
            login: None,
            api_connector,
            runtime,
            notifier,
        }
    }

    /// A device whose node dials this core: its machine is read through
    /// `link`, as this machine is through its own node.
    pub(crate) fn dialed_in(
        mut self,
        link: Arc<dyn crate::node_access::NodeLink>,
        login: Arc<dyn crate::node_access::NodeLink>,
    ) -> Self {
        self.inbound = Some(link);
        self.login = Some(login);
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
    /// machine's own, for this machine and for a node that dials this core.
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
    /// The last publish was held for a pane whose cwd Herdr had not
    /// confirmed and nothing could stand in for, so the worker's answer has
    /// to publish even when it changes no cwd; a publish that ran clears it
    /// (PRD grid-ahead-of-herdr D-12).
    held_for_cwd: bool,
    /// The cwd the last publish showed for each pane whose own cwd Herdr
    /// had not confirmed, a sibling's standing in. An answer that leaves the
    /// pane at a different cwd than the screen shows has to publish even
    /// when it changes nothing in the replica. At most
    /// `UNCONFIRMED_CWD_LIMIT` panes wait, so at most that many are kept.
    stand_ins: Vec<(String, String)>,
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
    republish_pending: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl SessionSyncHandle {
    pub(crate) fn republish_waker(&self) -> RepublishWaker {
        RepublishWaker {
            sender: self.sender.clone(),
            pending: Arc::clone(&self.republish_pending),
        }
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
    SubscriptionLine {
        generation: u64,
        line: String,
        /// When the reader pulled the line off the socket, which is what
        /// places it before or after the bootstrap snapshot.
        received_at: Instant,
    },
    SubscriptionEnded {
        generation: u64,
        message: String,
    },
    /// The label worker has a read or an analysis result to take.
    Labels,
    /// The runtime drew something ahead of Herdr, or took it back, and the
    /// session has to be read again with it (`RepublishWaker`).
    Republish,
    /// A socket read the sync-read worker made for the subscription of
    /// `generation` (`sync_reads.rs`).
    SyncRead {
        generation: u64,
        answer: sync_reads::SyncReadAnswer,
    },
}

/// Wakes the local coordinator to publish its session again at once: the
/// core has drawn a created tab or a predicted pane geometry ahead of Herdr,
/// or taken one back, and the overlay rides the next ingest (PRD
/// instant-pane-topology D-05, D-07). Wakes that arrive before the
/// coordinator takes the first are one publish, so a burst cannot queue
/// publishes.
#[derive(Clone)]
pub(crate) struct RepublishWaker {
    sender: Sender<CoordinatorMessage>,
    pending: Arc<AtomicBool>,
}

impl RepublishWaker {
    pub(crate) fn wake(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            let _ = self.sender.send(CoordinatorMessage::Republish);
        }
    }
}

pub(crate) struct ActiveSubscription {
    generation: u64,
    shutdown: Box<dyn hide_herdr_client::ConnectionShutdown>,
    worker: Option<JoinHandle<()>>,
}

impl ActiveSubscription {
    fn stop(mut self) {
        self.shutdown.shutdown();
        // A reader is blocked in its socket read. Waiting for it here made a
        // disconnect hold the coordinator until the peer happened to close
        // its copy, so the reconnect deadline was no longer meaningful.
        // Shutdown wakes the socket; the detached reader's next message is
        // ignored by its generation check, and its weak runtime reference
        // keeps the stop path independent of the old connection.
        let _ = self.worker.take();
    }
}

mod coordinator;
mod launch_settle;
pub(crate) use launch_settle::record_start;
mod path_facts;
mod process_info;
mod projection;
mod replica;
mod subscription;
mod sync_reads;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use coordinator::agent_tick_needs_publish;
pub(crate) use coordinator::spawn;
pub(crate) use projection::{
    ProjectedAgent, ProjectedPane, ProjectedTab, ProjectedWorkspace, ProjectedWorktree,
    ProjectionState, project_snapshot,
};
#[cfg(test)]
pub(crate) use replica::SNAPSHOT_FIELDS_THE_REPLICA_READS;
pub(crate) use replica::{ApplyMode, PaneMove, ReplicaEvent, SessionReplica, SubscriptionLine};
pub(crate) use replica::{remote_pane_id, remote_tab_id};
#[cfg(test)]
pub(crate) use subscription::connect_failure_from_api;
pub(crate) use subscription::{
    Connected, connect, fetch_agent_launch, fetch_agents, fetch_pane_cwd,
    fetch_workspace_active_tab, log_sync_failure, stop_subscription,
};
use sync_reads::{SyncRead, SyncReadAnswer, SyncReader};
