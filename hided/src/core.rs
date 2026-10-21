//! Owner-thread wrapper around `herdr_core::Core`.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use herdr_core::node_access::NodeLink;
use herdr_core::workspace_control::{
    Action, ActionMaterial, ActionPreparation, ActionResult, Context, Query, QueryResult, Refusal,
};
use herdr_core::{Core, CoreOptions};
use hide_node::terminal::device::DeviceSink;
use hide_node::terminal::router::Router;
use hide_node::terminal::{
    Attacher, LocalAttacher, Mode, OutputSink, ReportSink, RetryPolicy, Service, SessionParts,
};
use tokio::sync::broadcast;

use crate::terminal_hub::TerminalHub;

pub type SnapshotReply = herdr_core::SnapshotDeltaPayload;

type MoveSourceAnswer =
    Result<(herdr_core::MoveSource, serde_json::Value), herdr_core::MoveSourceRefusal>;
type ReleaseSourceAnswer =
    Result<(herdr_core::ReleaseSource, serde_json::Value), herdr_core::MoveSourceRefusal>;

enum Command {
    ObservationPrepare {
        device_id: String,
        pane_id: String,
        expected: Context,
        terminal: String,
        observation: hide_node_link::panes::DirectObservation,
        start: bool,
        deadline: std::time::Instant,
        reply: Sender<Result<herdr_core::direct_observation::PreparedObservation, String>>,
    },
    DeliveryHuman {
        reply: Sender<Result<herdr_core::delivery::worker::PreparedHuman, String>>,
    },
    ReleaseEffects {
        reply: Sender<bool>,
    },
    DeliveryPrepare {
        device_id: String,
        pane_id: String,
        expected: Context,
        hint: Option<String>,
        command: herdr_core::delivery::Command,
        reply: Sender<Result<herdr_core::delivery::worker::Prepared, String>>,
    },
    MemoryPrepare {
        device_id: String,
        pane_id: String,
        expected: Context,
        command: herdr_core::memory_command::MemoryCommand,
        reply: Sender<Result<herdr_core::memory_command::PreparedMemory, String>>,
    },
    Mission {
        device_id: String,
        pane_id: String,
        expected: Context,
        command: herdr_core::missions::MissionCommand,
        reply: Sender<Result<serde_json::Value, String>>,
    },
    AgentCatalog {
        device_id: String,
        machine: Option<String>,
        reply: Sender<Result<serde_json::Value, String>>,
    },
    SetFileRoots {
        roots: Vec<(std::path::PathBuf, std::fs::File)>,
        reply: Sender<Result<(), String>>,
    },
    Dispatch {
        event: Vec<u8>,
        reply: Sender<Result<(), String>>,
    },
    DeviceChannel {
        device_id: String,
        reply: Sender<Result<Arc<dyn NodeLink>, String>>,
    },
    WorkspaceRemoteRoutes {
        reply: Sender<Vec<herdr_core::WorkspaceRemoteRoute>>,
    },
    RemoteHerdrApi {
        device_id: String,
        reply: Sender<Option<Arc<dyn hide_herdr_client::ApiConnector>>>,
    },
    InputGate {
        reply: Sender<Arc<herdr_core::input_gate::PaneInputGate>>,
    },
    InboundRefusal {
        node: String,
        reply: Sender<Option<String>>,
    },
    LinkedNodes {
        reply: Sender<Vec<String>>,
    },
    ReleaseSource {
        node: String,
        reply: Sender<ReleaseSourceAnswer>,
    },
    MoveSource {
        device: String,
        reply: Sender<MoveSourceAnswer>,
    },
    InboundNode {
        node: String,
        label: String,
        arrived: herdr_core::remote::Arrived,
        reply: Sender<Result<(), String>>,
    },
    BrowserRouteSource {
        device_id: String,
        checkout_path: String,
        view_id: String,
        load: u64,
        reply: Sender<Option<herdr_core::workspace_control::BrowserRouteSource>>,
    },
    WorkspaceQuery {
        device_id: String,
        pane_id: String,
        query: Query,
        reply: Sender<Result<QueryResult, Refusal>>,
    },
    LinksScope {
        context: Context,
        reply: Sender<Option<herdr_core::links::query::Scope>>,
    },
    MemoryScope {
        context: Context,
        reply: Sender<Result<herdr_core::memory_hook::Scope, &'static str>>,
    },
    WorkspaceAction {
        device_id: String,
        pane_id: String,
        expected: Context,
        request_id: String,
        action: Action,
        material: Box<Result<Option<ActionMaterial>, Refusal>>,
        reply: Sender<Result<ActionResult, Refusal>>,
    },
    WorkspacePrepare {
        device_id: String,
        pane_id: String,
        expected: Context,
        request_id: String,
        action: Action,
        reply: Sender<Result<ActionPreparation, Refusal>>,
    },
    Snapshot {
        have_revision: u64,
        reply: Sender<Result<SnapshotReply, String>>,
    },
    Shutdown,
}

pub struct CoreHandle {
    /// The machine the core runs on, named in every key for this machine.
    node: herdr_core::node::NodeId,
    commands: Sender<Command>,
    pub notify: broadcast::Sender<()>,
    /// Every node's terminals: a screen's keys and views go here directly,
    /// never through the core (PRD core-host-node-terminal D-05).
    pub terminals: Arc<Router>,
    /// The output every node's terminals produce, as each screen reads it.
    pub hub: Arc<TerminalHub>,
    /// What every pane's output reaches: the hub, and the terminals relay
    /// of each linked node (`relay`).
    pub outputs: Arc<crate::relay::ScreenOutputs>,
    /// What a shutdown ends, taken out by the first one, so a second
    /// shutdown or the `Drop` after it finds nothing left to close.
    running: Mutex<Option<Running>>,
}

/// The core's owner thread and this machine's terminal service, which
/// `terminals` routes its panes to; the service is closed with the core,
/// since what else holds the router may outlive it.
struct Running {
    thread: JoinHandle<()>,
    local: Arc<Service>,
}

impl CoreHandle {
    #[allow(clippy::too_many_arguments)] // identity facts and deadline cross the authenticated boundary intact
    pub fn prepare_direct_observation(
        &self,
        device: &str,
        pane: &str,
        expected: &Context,
        terminal: String,
        observation: hide_node_link::panes::DirectObservation,
        start: bool,
        deadline: std::time::Instant,
    ) -> Result<herdr_core::direct_observation::PreparedObservation, String> {
        let (reply, result) = mpsc::channel();
        self.commands
            .send(Command::ObservationPrepare {
                device_id: device.to_owned(),
                pane_id: pane.to_owned(),
                expected: expected.clone(),
                terminal,
                observation,
                start,
                deadline,
                reply,
            })
            .map_err(|_| "observation_unavailable")?;
        let left = deadline
            .checked_duration_since(std::time::Instant::now())
            .ok_or("observation_expired")?;
        result
            .recv_timeout(left)
            .map_err(|_| "observation_expired")?
    }
    /// The machine the core runs on.
    pub fn node(&self) -> &herdr_core::node::NodeId {
        &self.node
    }

    pub fn prepare_delivery_human(
        &self,
    ) -> Result<herdr_core::delivery::worker::PreparedHuman, String> {
        let (reply, result) = mpsc::channel();
        self.commands
            .send(Command::DeliveryHuman { reply })
            .map_err(|_| "delivery_unavailable")?;
        result.recv().map_err(|_| "delivery_unavailable")?
    }

    /// The move's link committed this core: what it held while pending
    /// runs now. Returns whether this released the hold.
    pub fn release_effects(&self) -> Result<bool, String> {
        let (reply, result) = mpsc::channel();
        self.commands
            .send(Command::ReleaseEffects { reply })
            .map_err(|_| "core owner thread is gone".to_owned())?;
        result
            .recv()
            .map_err(|_| "core owner thread dropped the release reply".to_owned())
    }

    pub fn prepare_delivery(
        &self,
        device: &str,
        pane: &str,
        expected: &Context,
        hint: Option<String>,
        command: herdr_core::delivery::Command,
    ) -> Result<herdr_core::delivery::worker::Prepared, String> {
        let (reply, result) = mpsc::channel();
        self.commands
            .send(Command::DeliveryPrepare {
                device_id: device.to_owned(),
                pane_id: pane.to_owned(),
                expected: expected.clone(),
                hint,
                command,
                reply,
            })
            .map_err(|_| "delivery_unavailable".to_owned())?;
        result
            .recv()
            .map_err(|_| "delivery_unavailable".to_owned())?
    }

    /// Checks a `hide memory` caller on the owner thread; the command runs
    /// on the caller's blocking task.
    pub fn prepare_memory(
        &self,
        device: &str,
        pane: &str,
        expected: &Context,
        command: herdr_core::memory_command::MemoryCommand,
    ) -> Result<herdr_core::memory_command::PreparedMemory, String> {
        let (reply, result) = mpsc::channel();
        self.commands
            .send(Command::MemoryPrepare {
                device_id: device.to_owned(),
                pane_id: pane.to_owned(),
                expected: expected.clone(),
                command,
                reply,
            })
            .map_err(|_| "memory_unavailable".to_owned())?;
        result.recv().map_err(|_| "memory_unavailable".to_owned())?
    }

    /// Runs a Conductor's `hide mission` command on the owner thread.
    pub fn mission(
        &self,
        device: &str,
        pane: &str,
        expected: &Context,
        command: herdr_core::missions::MissionCommand,
    ) -> Result<serde_json::Value, String> {
        let (reply, result) = mpsc::channel();
        self.commands
            .send(Command::Mission {
                device_id: device.to_owned(),
                pane_id: pane.to_owned(),
                expected: expected.clone(),
                command,
                reply,
            })
            .map_err(|_| "mission_unavailable".to_owned())?;
        result
            .recv()
            .map_err(|_| "mission_unavailable".to_owned())?
    }

    /// `hide agent catalog`, read on the owner thread.
    pub fn agent_catalog(
        &self,
        device: &str,
        machine: Option<String>,
    ) -> Result<serde_json::Value, String> {
        let (reply, result) = mpsc::channel();
        self.commands
            .send(Command::AgentCatalog {
                device_id: device.to_owned(),
                machine,
                reply,
            })
            .map_err(|_| "catalog_unavailable".to_owned())?;
        result
            .recv()
            .map_err(|_| "catalog_unavailable".to_owned())?
    }

    pub fn set_file_roots(
        &self,
        roots: Vec<(std::path::PathBuf, std::fs::File)>,
    ) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::SetFileRoots { roots, reply })
            .map_err(|_| "core owner thread is gone".to_owned())?;
        rx.recv()
            .map_err(|_| "core owner thread dropped file-root reply".to_owned())?
    }

    /// `panes` is where each device's node link sends its panes' proofs and
    /// command streams (`node_panes`).
    pub fn spawn(
        options: CoreOptions,
        panes: hide_node::ssh::PaneEventsSlot,
    ) -> Result<Self, String> {
        let node = options.node_id.clone();
        let (command_tx, command_rx) = mpsc::channel::<Command>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let (notify_tx, _) = broadcast::channel(32);
        let notify_for_thread = notify_tx.clone();
        let (reports, terminal_reports) = herdr_core::terminal_reports::terminal_reports();
        let reports: Arc<dyn ReportSink> = Arc::new(reports);
        let hub = TerminalHub::new();
        let outputs = crate::relay::ScreenOutputs::new(Arc::clone(&hub));
        let local = Arc::new(
            Service::start(
                local_attacher(&options),
                Arc::clone(&outputs) as Arc<dyn OutputSink>,
                Arc::clone(&reports),
                RetryPolicy::Automatic,
            )
            .map_err(|error| format!("terminal service failed to start: {error}"))?,
        );
        let terminals = Arc::new(Router::new(
            Arc::clone(&local) as Arc<dyn hide_node_link::terminal::TerminalNode>,
            Arc::clone(&outputs) as Arc<dyn OutputSink>,
            reports,
        ));
        let routes = Arc::clone(&terminals);
        let thread = thread::Builder::new()
            .name("hided-core".into())
            .spawn(move || {
                owner_loop(
                    options,
                    panes,
                    (routes, terminal_reports),
                    command_rx,
                    ready_tx,
                    notify_for_thread,
                )
            })
            .map_err(|error| format!("core owner thread failed to start: {error}"))?;
        ready_rx
            .recv()
            .map_err(|_| "core owner thread exited before ready".to_owned())??;
        Ok(Self {
            node,
            commands: command_tx,
            notify: notify_tx,
            terminals,
            hub,
            outputs,
            running: Mutex::new(Some(Running { thread, local })),
        })
    }

    /// The core's input gate (`herdr_core::input_gate`), which a writer that
    /// does not go through a node's terminal asks at the write.
    pub fn input_gate(&self) -> Result<Arc<herdr_core::input_gate::PaneInputGate>, String> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::InputGate { reply })
            .map_err(|_| "core owner thread is gone".to_owned())?;
        rx.recv()
            .map_err(|_| "core owner thread dropped the input gate reply".to_owned())
    }

    pub fn dispatch(&self, event: Vec<u8>) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::Dispatch { event, reply })
            .map_err(|_| "core owner thread is gone".to_owned())?;
        rx.recv()
            .map_err(|_| "core owner thread dropped dispatch reply".to_owned())?
    }

    /// The link to the node `device_id` names; see `Core::node_link`.
    pub fn node_link(&self, device_id: &str) -> Result<Arc<dyn NodeLink>, String> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::DeviceChannel {
                device_id: device_id.to_owned(),
                reply,
            })
            .map_err(|_| "core owner thread is gone".to_owned())?;
        rx.recv()
            .map_err(|_| "core owner thread dropped device-channel reply".to_owned())?
    }

    pub fn workspace_remote_routes(&self) -> Result<Vec<herdr_core::WorkspaceRemoteRoute>, String> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::WorkspaceRemoteRoutes { reply })
            .map_err(|_| "core owner thread is gone".to_owned())?;
        rx.recv()
            .map_err(|_| "core owner thread dropped remote routes".to_owned())
    }

    /// The Herdr API connection the core holds for a connected SSH device;
    /// see `Core::remote_herdr_api`.
    pub fn remote_herdr_api(
        &self,
        device_id: &str,
    ) -> Result<Option<Arc<dyn hide_herdr_client::ApiConnector>>, String> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::RemoteHerdrApi {
                device_id: device_id.to_owned(),
                reply,
            })
            .map_err(|_| "core owner thread is gone".to_owned())?;
        rx.recv()
            .map_err(|_| "core owner thread dropped the device connection reply".to_owned())
    }

    /// The nodes that dialed this core and are connected now: the machines
    /// the operator works at (PRD core-host-node-remote-core D-18).
    pub fn linked_nodes(&self) -> Result<Vec<String>, String> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::LinkedNodes { reply })
            .map_err(|_| "core owner thread is gone".to_owned())?;
        rx.recv()
            .map_err(|_| "core owner thread dropped the linked nodes".to_owned())
    }

    /// What a move of the core to `device` is made from, and the device's
    /// label records (PRD core-host-node-move B4).
    pub fn release_source(&self, node: &str) -> Result<ReleaseSourceAnswer, String> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::ReleaseSource {
                node: node.to_owned(),
                reply,
            })
            .map_err(|_| "core owner thread is gone".to_owned())?;
        rx.recv()
            .map_err(|_| "core owner thread dropped the release source".to_owned())
    }

    pub fn move_source(&self, device: &str) -> Result<MoveSourceAnswer, String> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::MoveSource {
                device: device.to_owned(),
                reply,
            })
            .map_err(|_| "core owner thread is gone".to_owned())?;
        rx.recv()
            .map_err(|_| "core owner thread dropped the move source".to_owned())
    }

    /// Why a node dialing this core would be refused, before its link is
    /// started (PRD core-host-node-remote-core D-10).
    pub fn inbound_refusal(&self, node: &str) -> Option<String> {
        let (reply, rx) = mpsc::channel();
        if self
            .commands
            .send(Command::InboundRefusal {
                node: node.to_owned(),
                reply,
            })
            .is_err()
        {
            return Some("core_unavailable".to_owned());
        }
        rx.recv()
            .unwrap_or_else(|_| Some("core_unavailable".to_owned()))
    }

    /// Hands the core the link of a node that dialed it. A refusal drops
    /// the arrived node, whose transport closes the link.
    pub fn accept_inbound_node(
        &self,
        node: &str,
        label: &str,
        arrived: herdr_core::remote::Arrived,
    ) -> Result<(), String> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::InboundNode {
                node: node.to_owned(),
                label: label.to_owned(),
                arrived,
                reply,
            })
            .map_err(|_| "core_unavailable".to_owned())?;
        rx.recv().map_err(|_| "core_unavailable".to_owned())?
    }

    pub fn browser_route_source(
        &self,
        device_id: &str,
        checkout_path: &str,
        view_id: &str,
        load: u64,
    ) -> Result<Option<herdr_core::workspace_control::BrowserRouteSource>, String> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::BrowserRouteSource {
                device_id: device_id.to_owned(),
                checkout_path: checkout_path.to_owned(),
                view_id: view_id.to_owned(),
                load,
                reply,
            })
            .map_err(|_| "core owner thread is gone".to_owned())?;
        rx.recv()
            .map_err(|_| "core owner thread dropped browser route source".to_owned())
    }

    /// What a hook's Memory read for the pane in `context` is answered from,
    /// as the core knows it now; the node call and the store read run on the
    /// caller's blocking task. The owner's answer is waited for until the
    /// hook's `deadline` at most, the one its node call and store read share.
    pub fn memory_scope(
        &self,
        context: &Context,
        deadline: std::time::Instant,
    ) -> Result<herdr_core::memory_hook::Scope, &'static str> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::MemoryScope {
                context: context.clone(),
                reply,
            })
            .map_err(|_| "core_unavailable")?;
        match rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            Ok(scope) => scope,
            Err(mpsc::RecvTimeoutError::Timeout) => Err("core_deadline"),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err("core_unavailable"),
        }
    }

    /// The Projects a `hide links` read may answer from, as the core knows
    /// them now; the read itself runs on the caller's blocking task.
    pub fn links_scope(&self, context: &Context) -> Option<herdr_core::links::query::Scope> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::LinksScope {
                context: context.clone(),
                reply,
            })
            .ok()?;
        rx.recv().ok().flatten()
    }

    pub fn workspace_query(
        &self,
        device_id: &str,
        pane_id: &str,
        query: Query,
    ) -> Result<QueryResult, Refusal> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::WorkspaceQuery {
                device_id: device_id.to_owned(),
                pane_id: pane_id.to_owned(),
                query,
                reply,
            })
            .map_err(|_| Refusal {
                reason: "core_unavailable",
                next_action: "Reconnect Hide and retry",
            })?;
        rx.recv().map_err(|_| Refusal {
            reason: "core_unavailable",
            next_action: "Reconnect Hide and retry",
        })?
    }

    pub fn workspace_action(
        &self,
        device_id: &str,
        pane_id: &str,
        expected: &Context,
        request_id: &str,
        action: Action,
        material: Result<Option<ActionMaterial>, Refusal>,
    ) -> Result<ActionResult, Refusal> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::WorkspaceAction {
                device_id: device_id.to_owned(),
                pane_id: pane_id.to_owned(),
                expected: expected.clone(),
                request_id: request_id.to_owned(),
                action,
                material: Box::new(material),
                reply,
            })
            .map_err(|_| Refusal {
                reason: "core_unavailable",
                next_action: "Reconnect Hide and retry",
            })?;
        rx.recv().map_err(|_| Refusal {
            reason: "core_unavailable",
            next_action: "Reconnect Hide and retry",
        })?
    }

    pub fn workspace_prepare_action(
        &self,
        device_id: &str,
        pane_id: &str,
        expected: &Context,
        request_id: &str,
        action: &Action,
    ) -> Result<ActionPreparation, Refusal> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::WorkspacePrepare {
                device_id: device_id.to_owned(),
                pane_id: pane_id.to_owned(),
                expected: expected.clone(),
                request_id: request_id.to_owned(),
                action: action.clone(),
                reply,
            })
            .map_err(|_| Refusal {
                reason: "core_unavailable",
                next_action: "Reconnect Hide and retry",
            })?;
        rx.recv().map_err(|_| Refusal {
            reason: "core_unavailable",
            next_action: "Reconnect Hide and retry",
        })?
    }

    pub fn snapshot(&self, have_revision: u64) -> Result<SnapshotReply, String> {
        let (reply, rx) = mpsc::channel();
        self.commands
            .send(Command::Snapshot {
                have_revision,
                reply,
            })
            .map_err(|_| "core owner thread is gone".to_owned())?;
        rx.recv()
            .map_err(|_| "core owner thread dropped snapshot reply".to_owned())?
    }

    /// Ends the core once: the first call takes what it ends, so `Drop`
    /// after an explicit shutdown closes nothing again and never waits out
    /// a timed-out close a second time. A second caller waits under the lock
    /// until the first has finished.
    pub fn shutdown(&self) {
        let mut running = self
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(Running { thread, local }) = running.take() else {
            return;
        };
        let _ = self.commands.send(Command::Shutdown);
        let _ = thread.join();
        // The core that installs devices is gone: a device's terminals end,
        // and so do this machine's sessions and their attach children, now
        // rather than when the last holder of the router lets it go. A core
        // that stops for a move leaves its process a node, whose own
        // sessions would otherwise find each pane held and draw it only
        // read-only.
        self.terminals.remove_every_device();
        local.close();
    }
}

impl Drop for CoreHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// This machine's Herdr, through the official `herdr terminal session`
/// client. A daemon with no Herdr server has no panes of its own, and an
/// attach says so.
fn local_attacher(options: &CoreOptions) -> Box<dyn Attacher> {
    struct NoHerdr;
    impl Attacher for NoHerdr {
        fn open(&self, _: &str, _: Mode, _: u16, _: u16) -> Result<SessionParts, String> {
            Err("This daemon runs without a Herdr server".to_owned())
        }
    }
    match options.herdr_socket_path.as_deref() {
        Some(socket) => Box::new(LocalAttacher::new(
            options
                .herdr_bin_path
                .as_deref()
                .map(std::path::PathBuf::from),
            std::path::PathBuf::from(socket),
        )),
        None => Box::new(NoHerdr),
    }
}

fn owner_loop(
    options: CoreOptions,
    panes: hide_node::ssh::PaneEventsSlot,
    (terminals, terminal_reports): (Arc<Router>, herdr_core::terminal_reports::TerminalReports),
    commands: Receiver<Command>,
    ready: Sender<Result<(), String>>,
    notify: broadcast::Sender<()>,
) {
    // The core's own node answers for the home the core reads and writes
    // for: the configured one, else the process's, as the core decides.
    let home = options
        .home
        .as_ref()
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from));
    let mut own_node = hide_node::Local::new(home.clone());
    // What the node measured of each repository's worktrees outlives the
    // daemon, in the core's state folder.
    let state_dir = std::path::Path::new(&options.app_state_path)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    own_node = own_node
        .keeping_worktree_facts_in(hide_kit::layout::worktree_facts(&state_dir))
        .keeping_feedback_in(state_dir.clone());
    if let Some(socket) = options.herdr_socket_path.as_deref() {
        own_node = own_node.with_herdr_socket(std::path::PathBuf::from(socket));
    }
    // The install kit's parts ship beside this binary: in the app bundle, or
    // in the build folder a move or a core update placed in this account's
    // helper root, which a moved core's login item runs. A daemon anywhere
    // else, a development build among them, installs nothing (PRD
    // device-parity D-19).
    if let Ok(executable) = std::env::current_exe() {
        own_node = own_node.with_kit(hide_host::serve::KitPlace::of_program(
            &executable,
            home.as_deref(),
        ));
    }
    let own_herdr = options
        .herdr_socket_path
        .as_deref()
        .map(|socket| hide_node::herdr(std::path::Path::new(socket)));
    // The program builds devices run ship beside this binary; a daemon
    // anywhere else carries none, which leaves each device's node
    // `unsupported` with that reason.
    let devices = hide_node::ssh::Connector::new(
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)),
    )
    .with_herdr_program(
        options
            .herdr_bin_path
            .as_ref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("herdr")),
    )
    .with_helper_root(
        options
            .host_helper_root
            .clone()
            .unwrap_or_else(|| hide_node::ssh::host::DEFAULT_HELPER_ROOT.to_owned()),
    )
    .with_pane_events(panes)
    .with_terminals(Arc::clone(&terminals) as Arc<dyn DeviceSink>);
    let analytics = crate::env::product_analytics_token().and_then(|token| {
        match crate::product_analytics::Worker::start(home.as_deref(), &state_dir, token) {
            Ok(worker) => worker,
            Err(reason) => {
                herdr_core::diagnostic!(
                    serde_json::json!({ "component": "product_analytics", "kind": reason })
                );
                None
            }
        }
    });
    let Some(core) = Core::create_with_product_analytics(
        options,
        std::sync::Arc::new(own_node),
        own_herdr,
        std::sync::Arc::new(devices),
        terminals,
        terminal_reports,
        analytics.as_ref().map(|worker| {
            Arc::clone(&worker.client) as Arc<dyn herdr_core::product_analytics::Sink>
        }),
    ) else {
        let _ = ready.send(Err(
            "herdr-core create failed (check schema_version and paths)".to_owned(),
        ));
        return;
    };
    // After the core opened its Logs file: the SSH transport's records land
    // there too.
    hide_node::diagnostics::install(herdr_core::diagnostics::emit);
    core.on_change(move || {
        let _ = notify.send(());
    });
    let _ = ready.send(Ok(()));
    let mut _held_roots = hide_node::HeldRoots::default();
    while let Ok(command) = commands.recv() {
        match command {
            Command::ObservationPrepare {
                device_id,
                pane_id,
                expected,
                terminal,
                observation,
                start,
                deadline,
                reply,
            } => {
                let _ = reply.send(core.prepare_direct_observation(
                    &device_id,
                    &pane_id,
                    &expected,
                    &terminal,
                    observation,
                    start,
                    deadline,
                ));
            }
            Command::DeliveryHuman { reply } => {
                let _ = reply.send(core.prepare_delivery_human());
            }
            Command::ReleaseEffects { reply } => {
                let _ = reply.send(core.release_effects());
            }
            Command::MemoryPrepare {
                device_id,
                pane_id,
                expected,
                command,
                reply,
            } => {
                let _ = reply.send(core.prepare_memory(&device_id, &pane_id, &expected, command));
            }
            Command::Mission {
                device_id,
                pane_id,
                expected,
                command,
                reply,
            } => {
                let _ = reply.send(core.mission_command(&device_id, &pane_id, &expected, command));
            }
            Command::AgentCatalog {
                device_id,
                machine,
                reply,
            } => {
                let _ = reply.send(core.agent_catalog(&device_id, machine.as_deref()));
            }

            Command::DeliveryPrepare {
                device_id,
                pane_id,
                expected,
                hint,
                command,
                reply,
            } => {
                let _ = reply.send(core.prepare_delivery(
                    &device_id,
                    &pane_id,
                    &expected,
                    hint.as_deref(),
                    command,
                ));
            }
            Command::SetFileRoots { roots, reply } => {
                // This node holds the opened roots while the core pins their
                // identities; the previous set closes once it is replaced.
                let (held, identities) = hide_node::hold_roots(roots);
                core.set_file_roots(herdr_core::FileRoots::from_identities(identities));
                _held_roots = held;
                let _ = reply.send(Ok(()));
            }
            Command::Dispatch { event, reply } => {
                let _ = core.dispatch_bytes(&event);
                let _ = reply.send(Ok(()));
            }
            Command::DeviceChannel { device_id, reply } => {
                let _ = reply.send(core.node_link(&device_id));
            }
            Command::WorkspaceRemoteRoutes { reply } => {
                let _ = reply.send(core.workspace_remote_routes());
            }
            Command::RemoteHerdrApi { device_id, reply } => {
                let _ = reply.send(core.remote_herdr_api(&device_id));
            }
            Command::InputGate { reply } => {
                let _ = reply.send(core.input_gate());
            }
            Command::InboundRefusal { node, reply } => {
                let _ = reply.send(core.inbound_refusal(&node));
            }
            Command::LinkedNodes { reply } => {
                let _ = reply.send(core.linked_nodes());
            }
            Command::MoveSource { device, reply } => {
                let _ = reply.send(core.move_source(&device));
            }
            Command::ReleaseSource { node, reply } => {
                let _ = reply.send(core.release_source(&node));
            }
            Command::InboundNode {
                node,
                label,
                arrived,
                reply,
            } => {
                let _ = reply.send(core.accept_inbound_node(&node, &label, arrived));
            }
            Command::BrowserRouteSource {
                device_id,
                checkout_path,
                view_id,
                load,
                reply,
            } => {
                let _ = reply.send(core.browser_route_source(
                    &device_id,
                    &checkout_path,
                    &view_id,
                    load,
                ));
            }
            Command::WorkspaceQuery {
                device_id,
                pane_id,
                query,
                reply,
            } => {
                let _ = reply.send(core.workspace_control_query(&device_id, &pane_id, query));
            }
            Command::LinksScope { context, reply } => {
                let _ = reply.send(core.links_scope(&context));
            }
            Command::MemoryScope { context, reply } => {
                let _ = reply.send(core.memory_scope(&context));
            }
            Command::WorkspaceAction {
                device_id,
                pane_id,
                expected,
                request_id,
                action,
                material,
                reply,
            } => {
                let _ = reply.send(core.workspace_control_action(
                    &device_id,
                    &pane_id,
                    &expected,
                    &request_id,
                    action,
                    *material,
                ));
            }
            Command::WorkspacePrepare {
                device_id,
                pane_id,
                expected,
                request_id,
                action,
                reply,
            } => {
                let _ = reply.send(core.workspace_control_prepare_action(
                    &device_id,
                    &pane_id,
                    &expected,
                    &request_id,
                    &action,
                ));
            }
            Command::Snapshot {
                have_revision,
                reply,
            } => {
                let _ = reply.send(core.snapshot_delta_payload(have_revision));
            }
            Command::Shutdown => break,
        }
    }
    core.clear_on_change();
}

#[cfg(test)]
impl CoreHandle {
    /// A core with no Herdr and no registered checkout.
    pub(crate) fn bare(directory: &std::path::Path) -> Self {
        Self::spawn(
            herdr_core::CoreOptions {
                schema_version: crate::state_file::SCHEMA_VERSION,
                home: None,
                node_id: herdr_core::node::NodeId::parse("test-node").unwrap(),
                herdr_socket_path: None,
                herdr_bin_path: None,
                app_state_path: directory.join("core-state.json").display().to_string(),
                host_helper_root: None,
                host_cli_dir: None,
                workspace_views_path: None,
                shortcut_import_path: None,
                local_issues_path: None,
                effects_held: false,
                machine_name: None,
                account_state_folder: false,
            },
            Default::default(),
        )
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use hide_node::terminal::device::{DeviceTerminals, LineLink, LineRefused};
    use hide_node_link::terminal::{TerminalNode, TerminalRoutes};

    use super::*;

    /// A link that takes every line, and tells the test when its writer
    /// let go of it.
    struct Open {
        _released: mpsc::Sender<()>,
    }

    impl LineLink for Open {
        fn send_line(&self, _line: &[u8]) -> Result<(), LineRefused> {
            Ok(())
        }
        fn end(&self, _reason: &str) {}
    }

    /// Dropping the core's handle while a device's terminals are linked
    /// ends the terminal router, and with it this machine's terminal
    /// service, its sessions and their attach children: a device's node
    /// reports to the router that holds it, so the two kept each other.
    #[test]
    fn dropping_the_handle_with_a_device_linked_ends_its_terminals() {
        let directory = tempfile::tempdir().unwrap();
        let core = CoreHandle::bare(directory.path());
        let (link, released) = mpsc::channel();
        let device = DeviceTerminals::start(
            "mini",
            Arc::new(Open { _released: link }),
            Arc::clone(&core.terminals) as Arc<dyn DeviceSink>,
        )
        .unwrap();
        core.terminals
            .install_device("mini", Arc::new(device) as Arc<dyn TerminalNode>);
        let router = Arc::downgrade(&core.terminals);
        drop(core);
        // The link's writer ends once its device is removed.
        assert_eq!(
            released.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected),
            "the device's link writer outlived the core's handle"
        );
        let started = Instant::now();
        while router.upgrade().is_some() {
            assert!(
                started.elapsed() < Duration::from_secs(1),
                "the terminal router outlived the core's handle"
            );
            thread::yield_now();
        }
    }
}
