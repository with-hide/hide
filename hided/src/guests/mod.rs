//! Other machines' work on this machine (PRD device-connect-iroh D-61,
//! D-77): for each guest, a folder of its own, the guest's build placed in
//! it, and one node process of that build which this daemon starts,
//! supervises ([`node`]) and feeds from the guest's connection.
//!
//! Three kinds of stream reach here, each only under a grant the guest
//! holds: the management stream, on which the guest asks for its node and
//! hears where it stands; the node link, whose bytes this daemon carries
//! to the node's `link.sock` and back without reading them; and a file
//! stream that carries a build this machine does not have.
//!
//! A guest's node is its own build's process, so nothing here speaks to it
//! beyond the launch contract, and one guest's failure stays in that
//! guest's folder and process.

pub mod node;
pub mod tap;

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use hide_kit::layout;
use hide_node::ssh::host::{HelperPackages, guest_build};
use hide_node_link::address::guest_pane_id;
use hide_peer::protocol::build::{self, FileMessage};
use hide_peer::protocol::manage::{Manage, NodeStop};
use hide_peer::protocol::{PeerKey, StreamKind, frame, launch};
use hide_peer::{limits, wire};
use hide_platform::fs::{Access, atomic, private};
use hide_platform::ipc::{self, LocalStream};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Handle;
use tokio::sync::watch;

use self::node::{GuestNode, Phase};
use self::tap::GuestTap;
use crate::core::CoreHandle;
use crate::ending::Ending;
use crate::peer_host::{Incoming, PeerHost, Stream};

/// How long the daemon's end waits for each guest's node to be reaped.
const END_WITHIN: Duration = Duration::from_secs(launch::END_NOW_WAITS_SECS + 1);

/// How long a node that said it is ready is given to take its link.
const LINK_WITHIN: Duration = Duration::from_secs(5);

/// The bytes carried in one step between a guest and its node.
const CARRY: usize = 64 * 1024;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A map of guests is whole whatever panicked beside it.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn record(kind: &str, guest: &PeerKey, extra: serde_json::Value) {
    let mut record = json!({
        "component": "guests",
        "kind": kind,
        "guest": guest.short(),
    });
    if let (Some(record), Some(extra)) = (record.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            record.insert(key.clone(), value.clone());
        }
    }
    hide_core::diagnostic!(record);
}

/// Why a guest's node was not started, as its guest is told.
type Refusal = (NodeStop, String);

fn stopped((reason, detail): Refusal) -> Manage {
    Manage::NodeStopped {
        reason,
        retry: false,
        detail: Some(detail),
    }
}

fn told(phase: &Phase) -> Manage {
    match phase {
        Phase::Starting => preparing(),
        Phase::Ready(running) => Manage::NodeReady {
            build: running.build.clone(),
        },
        Phase::Stopped {
            reason,
            retry,
            detail,
        } => Manage::NodeStopped {
            reason: *reason,
            retry: *retry,
            detail: detail.clone(),
        },
    }
}

/// The platform a guest's build has to be for: this machine's.
fn preparing() -> Manage {
    Manage::NodePreparing {
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
    }
}

/// Where a guest's panes reach this machine's screens and its core.
pub struct Screens {
    pub core: Arc<CoreHandle>,
}

/// What this host remembers of a guest's node between its own starts
/// (`h/node.json`), so the guest's panes are drawn and typed into again
/// after this daemon restarts with the guest away (B16).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct NodeRecord {
    /// The build the node last ran from.
    build_id: String,
    /// The guest's product device id.
    device: String,
}

/// One guest on this machine: its node, and the tap this machine's screens
/// reach its panes by. Made together and dropped together.
#[derive(Clone)]
struct Hosted {
    node: Arc<GuestNode>,
    tap: Arc<GuestTap>,
}

pub struct Guests {
    state_dir: PathBuf,
    /// The account's home, which every guest's node is started with.
    home: PathBuf,
    /// This daemon's own programs: the build a guest of the same build
    /// runs, placed without a byte crossing the connection.
    packages: HelperPackages,
    /// The name of this daemon's own build for this machine, read once.
    own_build: OnceLock<Option<String>>,
    hosted: Mutex<HashMap<PeerKey, Hosted>>,
    /// None in a test of the node's launch alone, where no screen draws a
    /// guest's panes.
    screens: Option<Screens>,
    runtime: Handle,
}

/// The screens' hub for a daemon with none.
struct NoScreens;

impl hide_node::terminal::OutputSink for NoScreens {
    fn output(&self, _pane: &str, _bytes: &[u8], _full: bool) {}
    fn forget(&self, _pane: &str) {}
    fn clipboard(&self, _pane: &str, _text: &[u8]) {}
}

impl Guests {
    /// Called on the daemon's runtime, whose tasks follow each guest's
    /// node.
    pub fn new(
        state_dir: PathBuf,
        home: PathBuf,
        packages: HelperPackages,
        screens: Option<Screens>,
    ) -> Arc<Self> {
        Arc::new(Self {
            state_dir,
            home,
            packages,
            own_build: OnceLock::new(),
            hosted: Mutex::new(HashMap::new()),
            screens,
            runtime: Handle::current(),
        })
    }

    /// Every guest's panes on this machine, as each guest's node last
    /// said; a guest with none is not in the list.
    pub fn panes(&self) -> Vec<hide_core::guest_panes::GuestPanes> {
        let mut guests: Vec<_> = lock(&self.hosted)
            .iter()
            .map(|(key, hosted)| {
                let device = hosted.tap.device();
                hide_core::guest_panes::GuestPanes {
                    device_id: device.to_owned(),
                    key: key.to_hex(),
                    panes: hosted
                        .tap
                        .panes()
                        .into_iter()
                        .map(|pane| hide_core::guest_panes::GuestPane {
                            id: guest_pane_id(device, &pane.pane),
                            pane: pane.pane,
                            since_unix_ms: pane.since_unix_ms,
                        })
                        .collect(),
                }
            })
            .filter(|guest| !guest.panes.is_empty())
            .collect();
        guests.sort_by(|left, right| left.device_id.cmp(&right.device_id));
        guests
    }

    /// Tells the core the guests' panes, whole; the core keeps the list
    /// only when it differs. Called from a tap's own thread or a blocking
    /// one, never under a lock of this module.
    fn tell_core(&self) {
        let Some(screens) = &self.screens else { return };
        let event = json!({
            "schema_version": hide_core::SCHEMA_VERSION,
            "kind": "guest_panes",
            "payload": { "guests": self.panes() },
        });
        if let Err(error) = screens.core.dispatch(event.to_string().into_bytes()) {
            hide_core::diagnostic!(json!({
                "component": "guests",
                "kind": "guests.core_not_told",
                "reason": error,
            }));
        }
    }

    /// The guest's tap: it lists the panes the guest's `h/panes.json`
    /// holds, takes this machine's screens to them from now on, and is
    /// connected whenever `node` is ready.
    fn tap_of(self: &Arc<Self>, guest: &PeerKey, device: &str, node: &GuestNode) -> Arc<GuestTap> {
        let hub: Arc<dyn hide_node::terminal::OutputSink> = match &self.screens {
            Some(screens) => Arc::clone(&screens.core.outputs) as _,
            None => Arc::new(NoScreens),
        };
        let guests = Arc::downgrade(self);
        let tap = GuestTap::open(
            device,
            &guest.short(),
            layout::guest_panes(&self.state_dir, guest),
            hub,
            Box::new(move || {
                if let Some(guests) = guests.upgrade() {
                    guests.tell_core();
                }
            }),
        );
        if let Some(screens) = &self.screens {
            screens
                .core
                .terminals
                .install_guest(device, Arc::clone(&tap) as _);
        }
        let mut phase = node.phase();
        let record = layout::guest_node_record(&self.state_dir, guest);
        let following = Arc::clone(&tap);
        // Ends with the node: its phase has no more to say once it is
        // dropped.
        self.runtime.spawn(async move {
            loop {
                let now = phase.borrow_and_update().clone();
                match now {
                    Phase::Ready(running) => {
                        following.attach(&running.tap);
                        // What the next start of this daemon runs the
                        // guest's node from.
                        let remembered = NodeRecord {
                            build_id: running.build_id,
                            device: following.device().to_owned(),
                        };
                        let record = record.clone();
                        tokio::task::spawn_blocking(move || {
                            let written = serde_json::to_vec(&remembered)
                                .map_err(io::Error::other)
                                .and_then(|bytes| {
                                    atomic::write_file(&record, &bytes, Access::Private)
                                });
                            if let Err(error) = written {
                                hide_core::diagnostic!(json!({
                                    "component": "guests",
                                    "kind": "guests.node_not_remembered",
                                    "reason": error.to_string(),
                                }));
                            }
                        });
                    }
                    Phase::Starting | Phase::Stopped { .. } => following.detach(),
                }
                if phase.changed().await.is_err() {
                    break;
                }
            }
        });
        tap
    }

    /// The guests this machine hosted before this start: each one's panes
    /// are listed again from its `h/panes.json`, and a guest that still has
    /// panes gets its node back from the build it last ran, whether or not
    /// the guest is connected (B16). Blocking: it reads each guest's folder.
    pub fn resume(self: &Arc<Self>) {
        let Ok(folders) = std::fs::read_dir(layout::guests(&self.state_dir)) else {
            return;
        };
        for folder in folders.flatten().take(2 * limits::MAX_GRANTS) {
            let Some(guest) = std::fs::read_to_string(folder.path().join(launch::KEY_FILE))
                .ok()
                .and_then(|key| PeerKey::from_hex(key.trim()).ok())
                .filter(|key| folder.file_name().to_str() == Some(key.short().as_str()))
            else {
                continue;
            };
            let Some(remembered) =
                std::fs::read(layout::guest_node_record(&self.state_dir, &guest))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<NodeRecord>(&bytes).ok())
            else {
                continue;
            };
            let Ok(node) = self.node_for(&guest, &remembered.device) else {
                continue;
            };
            let has_panes = lock(&self.hosted)
                .get(&guest)
                .is_some_and(|hosted| !hosted.tap.panes().is_empty());
            let root = layout::guest_install_root(&self.state_dir, &guest);
            match guest_build::placed(&root, &remembered.build_id, std::env::consts::OS) {
                Some(program) if has_panes => node.start(program),
                _ => {}
            }
            record("guests.resumed", &guest, json!({"panes": has_panes}));
        }
        self.tell_core();
    }

    /// Takes the three kinds of stream a guest opens for its node, and ends
    /// every node with the daemon. Called once, before the endpoint opens.
    pub fn serve(self: &Arc<Self>, peers: &PeerHost, shutdown: Ending) {
        let guests = Arc::clone(self);
        peers.handle(StreamKind::Management, move |incoming| {
            Arc::clone(&guests).manage(incoming)
        });
        let guests = Arc::clone(self);
        peers.handle(StreamKind::NodeLink, move |incoming| {
            Arc::clone(&guests).link(incoming)
        });
        let guests = Arc::clone(self);
        peers.handle(StreamKind::File, move |incoming| {
            Arc::clone(&guests).file(incoming)
        });
        let guests = Arc::clone(self);
        tokio::task::spawn_blocking(move || guests.resume());
        let guests = Arc::clone(self);
        tokio::spawn(async move {
            shutdown.ended().await;
            let _ = tokio::task::spawn_blocking(move || guests.stop()).await;
        });
    }

    /// The node of `guest`, when this daemon was asked for one.
    pub fn node(&self, guest: &PeerKey) -> Option<Arc<GuestNode>> {
        lock(&self.hosted)
            .get(guest)
            .map(|hosted| Arc::clone(&hosted.node))
    }

    /// Ends every guest's node and waits, within a bound, until each is
    /// reaped. Their pane holders stay.
    pub fn stop(&self) {
        let hosted: Vec<_> = lock(&self.hosted).values().cloned().collect();
        for guest in &hosted {
            guest.node.stop();
        }
        let deadline = Instant::now() + END_WITHIN;
        for guest in &hosted {
            guest.node.stopped_by(deadline);
            guest.tap.detach();
        }
    }

    /// The guest's folder, made private with the guest's whole key in it.
    /// A folder that holds another key is another guest's whose key begins
    /// the same: it is refused, never shared.
    fn folder(&self, guest: &PeerKey) -> Result<(), Refusal> {
        let refused = |what: &str, error: &dyn std::fmt::Display| {
            (NodeStop::FolderRefused, format!("{what}: {error}"))
        };
        ipc::check_path(&launch::longest_endpoint(&self.state_dir)).map_err(|error| {
            refused(
                "this machine's state folder is too long a path for a guest's endpoints",
                &error,
            )
        })?;
        let dir = layout::guest_dir(&self.state_dir, guest);
        private::create_dir_all(&dir)
            .map_err(|error| refused("the guest's folder could not be made", &error))?;
        let key = layout::guest_key(&self.state_dir, guest);
        match std::fs::read_to_string(&key) {
            Ok(held) if held.trim() == guest.to_hex() => Ok(()),
            Ok(_) => Err((
                NodeStop::FolderRefused,
                "another guest whose key begins the same holds this guest's folder".to_owned(),
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                atomic::write_file(&key, guest.to_hex().as_bytes(), Access::Private)
                    .map(drop)
                    .map_err(|error| refused("the guest's key could not be saved", &error))
            }
            Err(error) => Err(refused("the guest's key could not be read", &error)),
        }
    }

    /// The name of this daemon's own build for this machine; `None` when
    /// it lacks a part a guest's node needs.
    fn own_build(&self) -> Option<&str> {
        self.own_build
            .get_or_init(|| {
                self.packages
                    .guest_build(std::env::consts::OS, std::env::consts::ARCH)
                    .ok()
                    .map(|own| own.id().to_owned())
            })
            .as_deref()
    }

    /// The node program of the guest's build `id`: already placed, or
    /// placed now from this daemon's own programs when they are that
    /// build. Blocking: it reads and writes whole programs.
    fn program(&self, guest: &PeerKey, id: &str) -> Result<PathBuf, Refusal> {
        if !build::is_id(id) {
            return Err((NodeStop::Other, format!("{id:?} names no build")));
        }
        self.folder(guest)?;
        let root = layout::guest_install_root(&self.state_dir, guest);
        if let Some(program) = guest_build::placed(&root, id, std::env::consts::OS) {
            return Ok(program);
        }
        if self.own_build() != Some(id) {
            return Err((
                NodeStop::BuildMissing,
                format!("build {id} is not on this machine"),
            ));
        }
        self.packages
            .guest_build(std::env::consts::OS, std::env::consts::ARCH)
            .map_err(|error| error.to_string())
            .and_then(|own| own.place(&root))
            .map_err(|detail| (NodeStop::FolderRefused, detail))
    }

    /// The guest's node and its tap, made on its first ask. `device` is
    /// the id the guest's panes go by on this machine: one that names no
    /// guest, or that another guest's panes already go by, is refused. The
    /// map holds one entry per guest that ever asked; at the cap the
    /// guests whose node is stopped and who have no pane here are dropped,
    /// and a guest past it is refused.
    fn node_for(
        self: &Arc<Self>,
        guest: &PeerKey,
        device: &str,
    ) -> Result<Arc<GuestNode>, Refusal> {
        let refused = |detail: String| Err((NodeStop::Other, detail));
        if !hide_core::guest_panes::device_names_a_guest(device) {
            return refused(format!("{device:?} is no device id this machine can show"));
        }
        let mut hosted = lock(&self.hosted);
        if let Some(known) = hosted.get(guest) {
            if known.tap.device() != device {
                return refused(format!(
                    "this machine knows this guest as {:?}, not {device:?}; pair the two again",
                    known.tap.device()
                ));
            }
            return Ok(Arc::clone(&known.node));
        }
        if hosted.values().any(|other| other.tap.device() == device) {
            return refused(format!(
                "another guest of this machine already goes by {device:?}"
            ));
        }
        if hosted.len() >= limits::MAX_GRANTS {
            let idle: Vec<PeerKey> = hosted
                .iter()
                .filter(|(_, known)| {
                    known.tap.panes().is_empty() && known.node.stopped_by(Instant::now())
                })
                .map(|(key, _)| *key)
                .collect();
            for key in idle {
                if let (Some(dropped), Some(screens)) = (hosted.remove(&key), &self.screens) {
                    screens.core.terminals.remove_guest(dropped.tap.device());
                }
            }
        }
        if hosted.len() >= limits::MAX_GRANTS {
            return refused(format!(
                "this machine already runs {} guests' nodes",
                limits::MAX_GRANTS
            ));
        }
        let node = GuestNode::new(
            *guest,
            layout::guest_node_side(&self.state_dir, guest),
            self.home.clone(),
        );
        let tap = self.tap_of(guest, device, &node);
        hosted.insert(
            *guest,
            Hosted {
                node: Arc::clone(&node),
                tap,
            },
        );
        Ok(node)
    }

    /// Starts the guest's node from its build `id`, placing the build when
    /// this machine has it.
    async fn start(
        self: &Arc<Self>,
        guest: PeerKey,
        device: &str,
        id: String,
    ) -> Result<Arc<GuestNode>, Refusal> {
        let guests = Arc::clone(self);
        let program = tokio::task::spawn_blocking(move || guests.program(&guest, &id))
            .await
            .map_err(|error| (NodeStop::Other, error.to_string()))??;
        let node = self.node_for(&guest, device)?;
        node.start(program);
        Ok(node)
    }

    /// One guest's management stream: its asks for its node, and where the
    /// node stands, told on every change from the first ask on.
    async fn manage(self: Arc<Self>, incoming: Incoming) {
        let guest = incoming.link.key();
        let device = incoming.link.their_hello().device.clone();
        let Stream {
            mut send, mut recv, ..
        } = incoming.stream;
        let mut phase: Option<watch::Receiver<Phase>> = None;
        loop {
            let answer = tokio::select! {
                message = wire::read_message(&mut recv, frame::MAX_MESSAGE) => {
                    let Ok(Some(body)) = message else { break };
                    let asked = match Manage::parse(&body) {
                        Ok(Some(asked)) => asked,
                        // A message of a later build.
                        Ok(None) => continue,
                        Err(_) => break,
                    };
                    match self.asked(guest, &device, asked).await {
                        Asked::Answer(answer) => Some(answer),
                        Asked::Watch(node) => {
                            // The node's standing now is the first answer.
                            let mut watched = node.phase();
                            watched.mark_changed();
                            phase = Some(watched);
                            None
                        }
                        Asked::Nothing => None,
                    }
                }
                changed = async {
                    match phase.as_mut() {
                        Some(phase) => phase.changed().await.map(|()| told(&phase.borrow_and_update())),
                        None => std::future::pending().await,
                    }
                } => match changed {
                    Ok(answer) => Some(answer),
                    Err(_) => break,
                },
            };
            if let Some(answer) = answer
                && wire::write_message(&mut send, &answer).await.is_err()
            {
                break;
            }
        }
    }

    async fn asked(self: &Arc<Self>, guest: PeerKey, device: &str, asked: Manage) -> Asked {
        match asked {
            Manage::NodeStart { build_id: None, .. } => Asked::Answer(preparing()),
            Manage::NodeStart {
                build,
                build_id: Some(id),
                ..
            } => match self.start(guest, device, id.clone()).await {
                Ok(node) => {
                    record(
                        "guests.node_asked",
                        &guest,
                        json!({"build": build, "build_id": id}),
                    );
                    Asked::Watch(node)
                }
                Err(refusal) => {
                    record(
                        "guests.node_refused",
                        &guest,
                        json!({"build": build, "build_id": id, "reason": refusal.1}),
                    );
                    Asked::Answer(stopped(refusal))
                }
            },
            Manage::NodeRetry => match self.node(&guest) {
                Some(node) if node.retry() => Asked::Watch(node),
                // This daemon was never asked for a build: the guest names
                // one, as it does on its first ask.
                _ => Asked::Answer(Manage::NodeStopped {
                    reason: NodeStop::Ended,
                    retry: false,
                    detail: None,
                }),
            },
            Manage::NodeEnd { mode } => {
                if let Some(node) = self.node(&guest) {
                    node.end(mode);
                }
                Asked::Nothing
            }
            // The event box's messages, and what only a host writes.
            Manage::Resume { .. }
            | Manage::Event(_)
            | Manage::EventAck { .. }
            | Manage::NodeReady { .. }
            | Manage::NodeStopped { .. }
            | Manage::NodePreparing { .. } => Asked::Nothing,
        }
    }

    /// One guest's node link: its bytes go to the guest's node and the
    /// node's come back, until either side ends. A guest whose node is not
    /// ready gets a closed stream; it opens the link when it hears ready.
    async fn link(self: Arc<Self>, incoming: Incoming) {
        let guest = incoming.link.key();
        let Some(running) = self.node(&guest).and_then(|node| node.running()) else {
            record("guests.link_without_node", &guest, json!({}));
            return;
        };
        let runtime = Handle::current();
        let carried =
            tokio::task::spawn_blocking(move || carry(incoming.stream, &running.link, &runtime))
                .await;
        let ended = match carried {
            Ok(Ok(())) => "ended".to_owned(),
            Ok(Err(error)) => error.to_string(),
            Err(error) => error.to_string(),
        };
        record("guests.link_ended", &guest, json!({"reason": ended}));
    }

    /// A build the guest uploads: the list of its files, then each file's
    /// bytes in that order. Every file is checked against the list before
    /// it is written, and the answer says whether the build is placed.
    async fn file(self: Arc<Self>, incoming: Incoming) {
        let guest = incoming.link.key();
        let Stream {
            mut send, mut recv, ..
        } = incoming.stream;
        let Ok(Some(body)) = wire::read_message(&mut recv, frame::MAX_MESSAGE).await else {
            return;
        };
        let (id, files) = match FileMessage::parse(&body) {
            Ok(Some(FileMessage::Build { id, files })) => (id, files),
            // What only a host writes, or a message of a later build.
            Ok(_) => {
                let _ = wire::write_message(
                    &mut send,
                    &FileMessage::Refused {
                        reason: "this build takes no such file message".to_owned(),
                    },
                )
                .await;
                return;
            }
            Err(_) => return,
        };
        let runtime = Handle::current();
        let guests = Arc::clone(&self);
        let named = id.clone();
        let placed = tokio::task::spawn_blocking(move || {
            guests.folder(&guest).map_err(|refusal| refusal.1)?;
            let root = layout::guest_install_root(&guests.state_dir, &guest);
            guest_build::place(&root, &named, &files, |file| {
                // `place` checked the list: a file is at most the cap.
                let mut bytes = vec![0u8; file.size as usize];
                runtime.block_on(async {
                    tokio::time::timeout(limits::WRITE_WITHIN, recv.read_exact(&mut bytes))
                        .await
                        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
                })?;
                Ok(bytes)
            })
        })
        .await
        .unwrap_or_else(|error| Err(error.to_string()));
        let answer = match placed {
            Ok(_) => {
                record("guests.build_placed", &guest, json!({"build_id": id}));
                FileMessage::BuildPlaced { id }
            }
            Err(reason) => {
                record(
                    "guests.build_refused",
                    &guest,
                    json!({"build_id": id, "reason": reason}),
                );
                FileMessage::Refused { reason }
            }
        };
        let _ = wire::write_message(&mut send, &answer).await;
        let _ = send.shutdown().await;
    }
}

enum Asked {
    Answer(Manage),
    /// The node's standing is told from now on.
    Watch(Arc<GuestNode>),
    Nothing,
}

/// Carries a guest's link stream to its node's endpoint and back until one
/// side ends, which ends the other. Blocking: one thread per direction,
/// this one and one it starts and joins.
fn carry(stream: Stream, endpoint: &Path, runtime: &Handle) -> io::Result<()> {
    let Stream {
        mut send, mut recv, ..
    } = stream;
    let node = LocalStream::connect_until(endpoint, Instant::now() + LINK_WITHIN)?;
    let mut to_node = node.duplicate();
    let mut from_node = node;
    let closer = from_node.shutdown_handle();
    // The node's bytes to the guest. It ends when the node closes, when the
    // guest takes no bytes in time, and when the other direction shuts the
    // node's stream down.
    let (ended, gone) = tokio::sync::oneshot::channel::<()>();
    let back = {
        let runtime = runtime.clone();
        std::thread::Builder::new()
            .name("guest-link".to_owned())
            .spawn(move || {
                let _ended = ended;
                let mut buffer = vec![0u8; CARRY];
                loop {
                    let read = match from_node.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => read,
                    };
                    let sent = runtime.block_on(async {
                        tokio::time::timeout(limits::WRITE_WITHIN, async {
                            send.write_all(&buffer[..read]).await?;
                            send.flush().await
                        })
                        .await
                    });
                    if !matches!(sent, Ok(Ok(()))) {
                        break;
                    }
                }
                let _ = runtime.block_on(send.shutdown());
            })?
    };
    // The guest's bytes to the node, on this thread, until the guest's
    // stream ends, the node takes no more, or the other direction ended.
    let mut gone = gone;
    let mut buffer = vec![0u8; CARRY];
    let result = loop {
        let read = runtime.block_on(async {
            tokio::select! {
                read = recv.read(&mut buffer) => Some(read),
                _ = &mut gone => None,
            }
        });
        match read {
            None | Some(Ok(0)) => break Ok(()),
            Some(Ok(read)) => {
                if let Err(error) = to_node
                    .write_all(&buffer[..read])
                    .and_then(|()| to_node.flush())
                {
                    break Err(error);
                }
            }
            Some(Err(error)) => break Err(error),
        }
    };
    closer.shutdown();
    let _ = back.join();
    result
}

/// A build whose node is a stand-in, for tests on both sides of a guest's
/// node.
#[cfg(all(test, unix))]
pub(crate) mod stand_in {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use hide_peer::protocol::launch;

    /// Lays out in `folder` the programs of "another build" whose node is a
    /// few lines of shell: all a host knows of a guest's node is the launch
    /// contract, which they keep. They say they are ready, name the link
    /// endpoint a test listens on, and end when told to or when their input
    /// ends.
    pub(crate) fn programs(folder: &Path) {
        let node = format!(
            "#!/bin/sh\nprintf '%s\\n' '{{\"guest_node\":{},\"build\":\"9.9.9\",\"build_id\":\"0000000000000000\",\"pid\":'\"$$\"',\"link\":\"{}\",\"tap\":\"{}\",\"tap_versions\":[{},{}],\"capabilities\":[]}}'\nwhile read -r line; do\n  case \"$line\" in *'\"end\"'*) exit 0 ;; esac\ndone\nexit 0\n",
            launch::GUEST_NODE,
            launch::LINK_ENDPOINT,
            launch::TAP_ENDPOINT,
            hide_peer::protocol::tap::OLDEST_TAP,
            hide_peer::protocol::tap::NEWEST_TAP,
        );
        for (name, bytes) in [
            ("hided", node.as_bytes()),
            ("hide", b"hide".as_slice()),
            ("hide-agent-hooks", b"hooks".as_slice()),
            ("hide-holder", b"holder".as_slice()),
        ] {
            let path = folder.join(name);
            std::fs::write(&path, bytes).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use hide_peer::protocol::build::BuildFile;
    use hide_peer::protocol::launch::EndMode;

    use super::*;
    use crate::peer_host::Link;
    use crate::peer_host::fixture::{Machine, paired, short_root, within};

    /// A host that takes guests, and one guest paired to it and connected.
    struct Pair {
        host: Machine,
        guest: Machine,
        link: Link,
        _home: tempfile::TempDir,
    }

    impl Pair {
        async fn new() -> Self {
            let host = Machine::at("windows-pc", short_root());
            let guest = Machine::new("macbook");
            let home = tempfile::tempdir().unwrap();
            // The host's own build has no programs: every build a guest
            // names is one this machine lacks.
            Guests::new(
                host.dir.path().to_path_buf(),
                home.path().to_path_buf(),
                HelperPackages::new(None, crate::cli::RELEASE),
                None,
            )
            .serve(&host.host, host.shutdown.clone());
            let link = paired(&host, &guest).await;
            Self {
                host,
                guest,
                link,
                _home: home,
            }
        }

        fn guest_key(&self) -> PeerKey {
            self.guest.host.peers().unwrap().key()
        }
    }

    async fn ask(stream: &mut Stream, message: &Manage) {
        wire::write_message(&mut stream.send, message)
            .await
            .unwrap();
    }

    async fn answer(stream: &mut Stream) -> Manage {
        let body = within(
            "a management message",
            wire::read_message(&mut stream.recv, frame::MAX_MESSAGE),
        )
        .await
        .unwrap()
        .expect("the management stream stays open");
        Manage::parse(&body)
            .unwrap()
            .expect("a message this build knows")
    }

    fn start(build_id: Option<&str>) -> Manage {
        Manage::NodeStart {
            build: "9.9.9".to_owned(),
            build_id: build_id.map(str::to_owned),
            release: None,
        }
    }

    /// Uploads `files` as the build `id`, each file's bytes from `bytes`,
    /// and answers what the host said.
    async fn upload(link: &Link, id: &str, files: &[BuildFile], bytes: &[&[u8]]) -> FileMessage {
        let mut stream = link.open(StreamKind::File).await.unwrap();
        wire::write_message(
            &mut stream.send,
            &FileMessage::Build {
                id: id.to_owned(),
                files: files.to_vec(),
            },
        )
        .await
        .unwrap();
        for bytes in bytes {
            stream.send.write_all(bytes).await.unwrap();
        }
        stream.send.flush().await.unwrap();
        let body = within(
            "the host's answer to the build",
            wire::read_message(&mut stream.recv, frame::MAX_MESSAGE),
        )
        .await
        .unwrap()
        .expect("an answer before the stream ends");
        FileMessage::parse(&body).unwrap().expect("a known answer")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_guest_hears_the_platform_to_build_for_and_a_build_that_is_not_its_list_is_refused() {
        let pair = Pair::new().await;
        let mut management = pair.link.open(StreamKind::Management).await.unwrap();

        ask(&mut management, &start(None)).await;
        assert_eq!(
            answer(&mut management).await,
            Manage::NodePreparing {
                os: std::env::consts::OS.to_owned(),
                arch: std::env::consts::ARCH.to_owned(),
            }
        );

        let files = vec![BuildFile {
            path: "hided".to_owned(),
            size: 4,
            sha256: "0".repeat(64),
            executable: true,
        }];
        let id = build::id_of(&files);
        ask(&mut management, &start(Some(&id))).await;
        assert!(
            matches!(
                answer(&mut management).await,
                Manage::NodeStopped {
                    reason: NodeStop::BuildMissing,
                    retry: false,
                    ..
                }
            ),
            "a build this machine lacks is asked of the guest"
        );

        // Four bytes that are not the file the list names.
        let refused = upload(&pair.link, &id, &files, &[b"node"]).await;
        assert!(
            matches!(refused, FileMessage::Refused { .. }),
            "{refused:?}"
        );
        let root = layout::guest_install_root(pair.host.dir.path(), &pair.guest_key());
        assert_eq!(
            guest_build::placed(&root, &id, std::env::consts::OS),
            None,
            "a build with a wrong file never counts as placed"
        );
        ask(&mut management, &start(Some(&id))).await;
        assert!(matches!(
            answer(&mut management).await,
            Manage::NodeStopped {
                reason: NodeStop::BuildMissing,
                ..
            }
        ));
    }

    fn key(byte: u8) -> PeerKey {
        PeerKey::from_hex(&format!("{byte:02x}").repeat(32)).unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_guests_panes_go_by_a_device_id_that_is_its_own_on_this_machine() {
        let state = short_root();
        let home = tempfile::tempdir().unwrap();
        let guests = Guests::new(
            state.path().to_path_buf(),
            home.path().to_path_buf(),
            HelperPackages::new(None, crate::cli::RELEASE),
            None,
        );
        let refused = |guest: u8, device: &str| {
            guests
                .node_for(&key(guest), device)
                .err()
                .map(|(reason, _)| reason)
        };
        assert_eq!(refused(1, "macbook"), None);
        assert_eq!(refused(1, "macbook"), None, "the same guest asks again");
        assert_eq!(
            refused(2, "macbook"),
            Some(NodeStop::Other),
            "another guest under the same id would share its panes' addresses"
        );
        assert_eq!(
            refused(1, "mini"),
            Some(NodeStop::Other),
            "a guest's panes keep the id they were given"
        );
        assert_eq!(refused(3, "mac:book"), Some(NodeStop::Other));
        assert_eq!(refused(3, ""), Some(NodeStop::Other));
        assert_eq!(refused(3, "mini"), None);
    }

    /// B16: the host's daemon starts again with the guest away.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_host_that_starts_again_lists_a_guests_panes_and_runs_its_node_with_the_guest_away() {
        use hide_peer::protocol::panes::{self, Pane, Panes};
        use hide_peer::protocol::tap::ToNode;

        let state = short_root();
        let home = tempfile::tempdir().unwrap();
        let guest = key(7);
        let programs = tempfile::tempdir().unwrap();
        stand_in::programs(programs.path());
        // What an earlier run of this daemon left: the guest's folder, its
        // build, the build its node ran from and the panes it had.
        let build = HelperPackages::new(Some(programs.path().to_path_buf()), crate::cli::RELEASE)
            .guest_build(std::env::consts::OS, std::env::consts::ARCH)
            .unwrap_or_else(|error| panic!("the stand-in build is whole: {error}"));
        let earlier = Guests::new(
            state.path().to_path_buf(),
            home.path().to_path_buf(),
            HelperPackages::new(None, crate::cli::RELEASE),
            None,
        );
        earlier.folder(&guest).unwrap();
        build
            .place(&layout::guest_install_root(state.path(), &guest))
            .unwrap();
        let write = |path: PathBuf, bytes: Vec<u8>| {
            private::create_dir_all(path.parent().unwrap()).unwrap();
            atomic::write_file(&path, &bytes, Access::Private).unwrap();
        };
        write(
            layout::guest_node_record(state.path(), &guest),
            serde_json::to_vec(&NodeRecord {
                build_id: build.id().to_owned(),
                device: "macbook".to_owned(),
            })
            .unwrap(),
        );
        write(
            layout::guest_panes(state.path(), &guest),
            serde_json::to_vec(&Panes {
                version: panes::VERSION,
                panes: vec![Pane {
                    pane: "w1:p1".to_owned(),
                    rows: 24,
                    cols: 80,
                    since_unix_ms: 5,
                }],
            })
            .unwrap(),
        );
        drop(earlier);
        // This test is the tap of the node the stand-in says is ready.
        let node_dir = layout::guest_node_side(state.path(), &guest);
        std::fs::create_dir_all(&node_dir).unwrap();
        let taps = ipc::LocalListener::bind(&node_dir.join(launch::TAP_ENDPOINT)).unwrap();

        let guests = Guests::new(
            state.path().to_path_buf(),
            home.path().to_path_buf(),
            HelperPackages::new(None, crate::cli::RELEASE),
            None,
        );
        {
            let guests = Arc::clone(&guests);
            tokio::task::spawn_blocking(move || guests.resume())
                .await
                .unwrap();
        }
        assert_eq!(
            guests.panes(),
            [hide_core::guest_panes::GuestPanes {
                device_id: "macbook".to_owned(),
                key: guest.to_hex(),
                panes: vec![hide_core::guest_panes::GuestPane {
                    id: "guest:macbook:pane:w1:p1".to_owned(),
                    pane: "w1:p1".to_owned(),
                    since_unix_ms: 5,
                }],
            }],
            "the panes show before the node is back"
        );
        let hello = within(
            "the tap of the node that was started again",
            tokio::task::spawn_blocking(move || {
                let mut node = taps.accept().unwrap();
                tap::tests::heard(&mut node)
            }),
        )
        .await
        .unwrap();
        assert!(matches!(hello, ToNode::Hello { .. }), "{hello:?}");
        assert!(
            guests.node(&guest).unwrap().running().is_some(),
            "the guest's node runs with no guest connected"
        );
        tokio::task::spawn_blocking(move || guests.stop())
            .await
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_guest_uploads_its_build_and_its_node_runs_from_it_carries_its_link_and_ends_when_asked()
     {
        let pair = Pair::new().await;
        let guest = pair.guest_key();
        let state = pair.host.dir.path();
        let programs = tempfile::tempdir().unwrap();
        stand_in::programs(programs.path());
        let build = HelperPackages::new(Some(programs.path().to_path_buf()), crate::cli::RELEASE)
            .guest_build(std::env::consts::OS, std::env::consts::ARCH)
            .unwrap_or_else(|error| panic!("the stand-in build is whole: {error}"));
        let bytes: Vec<&[u8]> = (0..build.files().len())
            .map(|index| build.bytes(index).unwrap())
            .collect();
        assert_eq!(
            upload(&pair.link, build.id(), build.files(), &bytes).await,
            FileMessage::BuildPlaced {
                id: build.id().to_owned()
            }
        );
        assert_eq!(
            std::fs::read_to_string(layout::guest_key(state, &guest)).unwrap(),
            guest.to_hex(),
            "the guest's folder holds its whole key"
        );

        // The stand-in names the node's link endpoint and this test is the
        // node behind it.
        let node_dir = layout::guest_node_side(state, &guest);
        std::fs::create_dir_all(&node_dir).unwrap();
        let listener = ipc::LocalListener::bind(&node_dir.join(launch::LINK_ENDPOINT)).unwrap();
        let node = tokio::task::spawn_blocking(move || {
            let mut stream = listener.accept().unwrap();
            let mut heard = [0u8; 5];
            stream.read_exact(&mut heard).unwrap();
            stream.write_all(b"world").unwrap();
            heard
        });

        let mut management = pair.link.open(StreamKind::Management).await.unwrap();
        ask(&mut management, &start(Some(build.id()))).await;
        let ready = loop {
            match answer(&mut management).await {
                Manage::NodePreparing { .. } => {}
                other => break other,
            }
        };
        assert_eq!(
            ready,
            Manage::NodeReady {
                build: "9.9.9".to_owned()
            }
        );

        let mut link = pair.link.open(StreamKind::NodeLink).await.unwrap();
        link.send.write_all(b"hello").await.unwrap();
        link.send.flush().await.unwrap();
        let mut back = [0u8; 5];
        within("the node's bytes", link.recv.read_exact(&mut back))
            .await
            .unwrap();
        assert_eq!(&back, b"world");
        assert_eq!(&within("the node's read", node).await.unwrap(), b"hello");
        // The node closed its end: the guest's stream ends with it.
        let mut rest = Vec::new();
        let ended = within("the link's end", link.recv.read_to_end(&mut rest)).await;
        assert!(matches!(ended, Ok(0) | Err(_)), "{ended:?}");

        ask(&mut management, &Manage::NodeEnd { mode: EndMode::Now }).await;
        assert_eq!(
            answer(&mut management).await,
            Manage::NodeStopped {
                reason: NodeStop::Ended,
                retry: false,
                detail: None,
            }
        );
    }
}
