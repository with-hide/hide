//! How a node on another machine reaches this core (PRD
//! core-host-node-remote-core D-04, D-07, D-10).
//!
//! The node dials this machine over SSH and runs `hided attach` here. That
//! attach role starts no core: it finds the core already running on this
//! machine through the attach socket the core records in its state folder,
//! and pipes the node's SSH channel to it byte for byte. With no core to
//! find it answers `no_core` and exits.
//!
//! The attach socket sits in a fresh owner-only folder, so only processes
//! of this account reach it; they are inside the core's trust boundary
//! already (they can read its screen token). On it, the core and the node
//! first trade one line each: the core names its node and build, the node
//! names its own and its Herdr socket. A different build, this machine's own
//! node id, a device this core dials, or a node whose earlier link stands is
//! refused with a reason line. Otherwise the core answers with the relay
//! grant for the node's screens and the stream becomes the node's link.
//!
//! The attach role ends when its node's channel ends, when the core's socket
//! closes, or when its node has sent nothing for
//! [`hide_node_link::panes::HEARTBEAT_LIMIT`], so neither a half-open SSH
//! connection nor a stopped core leaves it running (B20).

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hide_node::pane_proof::{BootstrapListener, RecordRefusal};
use hide_node::ssh::RemoteHost;
use hide_node::ssh::host::inbound::{self, InboundNode};
use hide_node::terminal::device::DeviceSink;
use hide_platform::ipc::LocalStream;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::core::CoreHandle;
use crate::relay::RelayRequests;
use crate::state_file::new_token;

/// The longest handshake line either side reads.
const MAX_HANDSHAKE_LINE: u64 = 4096;
/// How long the handshake may take before the attach socket gives up on a
/// node.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// Nodes the core takes the handshake of at once; more wait in the
/// listener's backlog.
const MAX_ATTACHING: usize = 4;
/// Grants held at once: one per linked node and a few being handed out.
const MAX_GRANTS: usize = 16;
/// How long a relay waits for a grant handed out moments ago to be bound to
/// its link: the node's Hello and the core's acceptance, with room.
const GRANT_BIND_WAIT: Duration = Duration::from_secs(10);
/// How long a node's earlier link has to answer a greeting when the node
/// dials again, before the new link replaces it (B9's 10 s holds with it).
const STALE_PROBE: Duration = Duration::from_secs(3);
/// How long a grant handed to a node may wait for its link to be taken.
const UNBOUND_GRANT_LIFETIME: Duration = Duration::from_secs(60);
/// The wait after an accept that failed, doubling to [`ACCEPT_RETRY_MAX`].
const ACCEPT_RETRY_FIRST: Duration = Duration::from_millis(100);
const ACCEPT_RETRY_MAX: Duration = Duration::from_secs(5);

/// Why the attach role reached no core on its machine; the node reports it
/// as the reason it waits.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachOutcome {
    /// No core recorded its attach socket here: none runs, and the attach
    /// role starts none (D-07).
    NoCore,
    /// The record names a socket no core answers on: the core that wrote
    /// it ended without removing it, or is not answering.
    CoreNotAnswering,
    /// The record or its socket's folder is not this account's own and
    /// private, so it is not followed.
    RecordUntrusted,
}

impl AttachOutcome {
    pub fn code(self) -> &'static str {
        match self {
            Self::NoCore => "no_core",
            Self::CoreNotAnswering => "core_not_answering",
            Self::RecordUntrusted => "record_untrusted",
        }
    }
}

/// One handshake line, as the attach role, the core and the node write it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Line {
    /// From the attach role: why it reached no core on its machine.
    Attach(AttachOutcome),
    /// From the core: who it is.
    Core(CoreHello),
    /// From the node: who it is.
    Node(NodeHello),
    /// From the core: the link is taken, and how the node's screens reach
    /// the core's screen traffic.
    Accepted(Accepted),
    /// From the core: the link is refused, and why.
    Refused(Refusal),
    /// From `hided core-move release` on the core's own machine: stop the
    /// core for a move back to `target` (PRD core-host-node-move B6).
    Release(ReleaseRequest),
    /// From the core: the stop is recorded and the core stops now.
    Released,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReleaseRequest {
    pub intent: String,
    pub target: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CoreHello {
    pub node: String,
    pub build: String,
    pub protocol: u32,
    /// The core build's version and order (`build_order`); a core before
    /// this field reads as unordered.
    #[serde(default)]
    pub release: crate::build_order::Release,
    /// The core machine's name, which a node's page names when it asks
    /// for this app to be updated (B11).
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NodeHello {
    pub node: String,
    pub label: String,
    pub build: String,
    pub herdr_socket: String,
    /// The node build's version and order (`build_order`).
    #[serde(default)]
    pub release: crate::build_order::Release,
    /// The core move whose commit this link is, while the node's placement
    /// names one (PRD core-host-node-move amendment 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_intent: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Accepted {
    /// The core's loopback port on its own machine.
    pub port: u16,
    /// The grant the node's screen relay presents; it ends with the link.
    pub relay_token: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Refusal {
    pub reason: String,
}

/// Reads one handshake line of at most [`MAX_HANDSHAKE_LINE`] bytes.
pub fn read_line(reader: &mut impl BufRead) -> Result<Line, HandshakeError> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_HANDSHAKE_LINE + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|error| {
            HandshakeError::Lost(format!("the handshake could not be read: {error}"))
        })?;
    if bytes.is_empty() {
        return Err(HandshakeError::Lost(
            "the other side closed before the handshake".to_owned(),
        ));
    }
    if bytes.len() as u64 > MAX_HANDSHAKE_LINE || bytes.last() != Some(&b'\n') {
        return Err(HandshakeError::Unreadable(
            "the handshake line is too long".to_owned(),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        HandshakeError::Unreadable(format!(
            "the handshake line is not one this build reads ({:?})",
            error.classify()
        ))
    })
}

/// Why a handshake line was not read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HandshakeError {
    /// The connection ended, failed or went quiet past its deadline.
    Lost(String),
    /// A line arrived that this build does not read.
    Unreadable(String),
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lost(message) | Self::Unreadable(message) => formatter.write_str(message),
        }
    }
}

/// Writes one handshake line.
pub fn write_line(writer: &mut impl Write, line: &Line) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(line).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .and_then(|()| writer.flush())
        .map_err(|error| format!("the handshake could not be written: {error}"))
}

/// Where the core records its attach socket.
pub fn attach_record(state_dir: &Path) -> PathBuf {
    state_dir.join("node-attach-socket")
}

/// Binds the core's attach socket and records it in `state_dir`.
pub fn bind(state_dir: &Path) -> Result<(BootstrapListener, PathBuf), String> {
    hide_node::pane_proof::bind_recorded(
        &attach_record(state_dir),
        "hide-attach",
        "a.sock",
        new_token,
    )
}

/// The screen relay grants this core handed to linked nodes, each bound to
/// the link it was handed out on and ending with it (D-10).
pub struct RelayGrants {
    grants: Mutex<HashMap<String, Grant>>,
    /// Moves on every bind and revoke, waking whoever waits for a grant to
    /// be bound ([`RelayGrants::admit`]).
    changed: tokio::sync::watch::Sender<u64>,
}

impl Default for RelayGrants {
    fn default() -> Self {
        Self {
            grants: Mutex::default(),
            changed: tokio::sync::watch::Sender::new(0),
        }
    }
}

struct Grant {
    node: String,
    /// When it was handed out; one never bound to a link expires.
    issued: Instant,
    /// The link the grant belongs to, once the link is up.
    link: Option<RemoteHost>,
    /// What the node's screens wait on this core for, over every relay.
    requests: Arc<RelayRequests>,
}

/// A relay a grant admitted: the node, its link, and what its screens wait
/// on.
pub struct Admitted {
    pub node: String,
    pub link: RemoteHost,
    pub requests: Arc<RelayRequests>,
}

impl RelayGrants {
    /// A new grant for `node`, not yet bound to its link; `None` at the cap.
    fn issue(&self, node: &str) -> Option<String> {
        let mut grants = lock(&self.grants);
        grants.retain(|_, grant| match &grant.link {
            Some(link) => link.closed_reason().is_none(),
            None => grant.issued.elapsed() < UNBOUND_GRANT_LIFETIME,
        });
        if grants.len() >= MAX_GRANTS {
            return None;
        }
        let token = new_token();
        grants.insert(
            token.clone(),
            Grant {
                node: node.to_owned(),
                issued: Instant::now(),
                link: None,
                requests: Arc::default(),
            },
        );
        Some(token)
    }

    fn bind(&self, token: &str, link: RemoteHost) {
        if let Some(grant) = lock(&self.grants).get_mut(token) {
            grant.link = Some(link);
        }
        self.changed.send_modify(|count| *count += 1);
    }

    fn revoke(&self, token: &str) {
        lock(&self.grants).remove(token);
        self.changed.send_modify(|count| *count += 1);
    }

    /// The node and link `token` was granted on, waiting up to
    /// [`GRANT_BIND_WAIT`] for a grant handed out moments ago to be bound:
    /// the node's screens may ask before the core finished taking its link.
    pub async fn admit(&self, token: &str) -> Option<Admitted> {
        let deadline = tokio::time::Instant::now() + GRANT_BIND_WAIT;
        // Subscribed before the look, so a bind between the two still wakes it.
        let mut changed = self.changed.subscribe();
        loop {
            {
                let grants = lock(&self.grants);
                let grant = grants.get(token)?;
                if grant.link.is_some() {
                    drop(grants);
                    return self.valid(token);
                }
            }
            match tokio::time::timeout_at(deadline, changed.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => return None,
            }
        }
    }

    /// The node and link `token` was granted on, while that link lives.
    pub fn valid(&self, token: &str) -> Option<Admitted> {
        let grants = lock(&self.grants);
        let grant = grants.get(token)?;
        let link = grant.link.as_ref()?;
        link.closed_reason().is_none().then(|| Admitted {
            node: grant.node.clone(),
            link: link.clone(),
            requests: Arc::clone(&grant.requests),
        })
    }
}

/// What the core's attach socket needs to take a node's link.
pub struct AttachService {
    pub core: Arc<CoreHandle>,
    /// This core's build, which a node must match (D-23); `None` reads it
    /// from this executable when the first node attaches.
    pub build: Option<String>,
    /// The core's loopback port, handed to a linked node's screen relay.
    pub port: u16,
    pub panes: hide_node::ssh::PaneEventsSlot,
    pub terminals: Arc<dyn DeviceSink>,
    pub grants: Arc<RelayGrants>,
    pub attaching: Attaching,
    /// Closed while a move's copy waits for its commit.
    pub move_gate: Arc<crate::core_move::gate::MoveGate>,
    /// The process's move, which a release asks to stop the core.
    pub moves: Arc<crate::core_move::control::MoveControl>,
}

/// How long the core may take to check and record a release.
const RELEASE_ANSWER_WITHIN: Duration = Duration::from_secs(20);

/// The nodes whose link is between the core's admission and its taking the
/// link as the node's. A second attach of one meanwhile is refused as
/// already linked, as it would be a moment later; without this both pass
/// the admission, and one of them is told it was accepted and then dropped.
#[derive(Default)]
pub struct Attaching(Mutex<HashSet<String>>);

impl Attaching {
    /// Holds `node` until the guard goes, or `None` while another holds it.
    fn hold(&self, node: &str) -> Option<AttachingNode<'_>> {
        lock(&self.0)
            .insert(node.to_owned())
            .then(|| AttachingNode(self, node.to_owned()))
    }
}

struct AttachingNode<'a>(&'a Attaching, String);

impl Drop for AttachingNode<'_> {
    fn drop(&mut self) {
        lock(&self.0.0).remove(&self.1);
    }
}

/// Takes nodes' links on `listener` until `shutdown`.
pub async fn serve(
    listener: BootstrapListener,
    service: Arc<AttachService>,
    shutdown: crate::ending::Ending,
) {
    // Closed however this future ends, a drop included, so the accept
    // thread never outlives it.
    let closing = CloseOnDrop {
        closer: listener.closer(),
        closed: Arc::new(AtomicBool::new(false)),
    };
    let (accepted, mut arrivals) = tokio::sync::mpsc::channel(MAX_ATTACHING);
    let accepting = {
        let closed = Arc::clone(&closing.closed);
        std::thread::Builder::new()
            .name("node-attach-accept".to_owned())
            .spawn(move || accept_nodes(&listener, &accepted, &closed))
    };
    if let Err(error) = accepting {
        attach_stopped(&error.to_string());
        return;
    }
    let limit = Arc::new(tokio::sync::Semaphore::new(MAX_ATTACHING));
    loop {
        let arrival = tokio::select! {
            arrival = arrivals.recv() => arrival,
            () = shutdown.ended() => break,
        };
        let Some(stream) = arrival else {
            break;
        };
        // The folder admits only this account already; a peer the system
        // cannot name, or names as another account, is not taken either.
        if !stream.peer_is_this_account().unwrap_or(false) {
            herdr_core::diagnostic!(json!({
                "component": "node_link",
                "kind": "attach.refused",
                "reason": "other_account",
            }));
            continue;
        }
        let Ok(permit) = Arc::clone(&limit).try_acquire_owned() else {
            herdr_core::diagnostic!(json!({
                "component": "node_link",
                "kind": "attach.busy",
                "cap": MAX_ATTACHING,
            }));
            continue;
        };
        let service = Arc::clone(&service);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            take_link(stream, &service);
        });
    }
}

/// Accepts nodes until the listener is closed. An accept that fails (out of
/// descriptors, a peer gone before it was taken) is logged and tried again
/// after a bounded wait, so one failure never ends the attach service while
/// its record still tells nodes a core runs here.
#[allow(clippy::disallowed_methods)] // a production wait between accepts, not test code
fn accept_nodes(
    listener: &BootstrapListener,
    accepted: &tokio::sync::mpsc::Sender<LocalStream>,
    closed: &AtomicBool,
) {
    let mut wait = ACCEPT_RETRY_FIRST;
    loop {
        match listener.accept() {
            Ok(stream) => {
                wait = ACCEPT_RETRY_FIRST;
                if accepted.blocking_send(stream).is_err() {
                    return;
                }
            }
            Err(_) if closed.load(Ordering::SeqCst) => return,
            Err(error) => {
                herdr_core::diagnostic!(json!({
                    "component": "node_link",
                    "kind": "attach.accept_failed",
                    "message": error.to_string(),
                    "retry_ms": wait.as_millis() as u64,
                }));
                std::thread::sleep(wait);
                wait = (wait * 2).min(ACCEPT_RETRY_MAX);
            }
        }
    }
}

struct CloseOnDrop {
    closer: hide_platform::ipc::ListenerCloser,
    /// Set before the close, so the accept thread tells its own stop from an
    /// accept that failed.
    closed: Arc<AtomicBool>,
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        self.closer.close();
    }
}

fn attach_stopped(message: &str) {
    herdr_core::diagnostic!(json!({
        "component": "node_link",
        "kind": "attach.stopped",
        "message": message,
    }));
}

impl AttachService {
    fn build(&self) -> Result<String, String> {
        static OWN: std::sync::OnceLock<Result<String, String>> = std::sync::OnceLock::new();
        match &self.build {
            Some(build) => Ok(build.clone()),
            None => OWN.get_or_init(crate::build_id::of_current_exe).clone(),
        }
    }
}

/// The handshake with one node and, when it is taken, its link.
fn take_link(stream: LocalStream, service: &AttachService) {
    let started = Instant::now();
    let build = match service.build() {
        Ok(build) => build,
        Err(message) => {
            attach_failed("", &message);
            return;
        }
    };
    let _ = stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
    let mut writer = stream.duplicate();
    let mut reader = BufReader::new(stream.duplicate());
    let core_node = service.core.node().as_str().to_owned();
    let release = crate::build_order::Release::of_this_build();
    let hello = Line::Core(CoreHello {
        node: core_node.clone(),
        build: build.clone(),
        protocol: hide_node_link::protocol::PROTOCOL_VERSION,
        release: release.clone(),
        label: crate::host_name().unwrap_or_else(|| core_node.clone()),
    });
    if let Err(message) = write_line(&mut writer, &hello) {
        attach_failed("", &message);
        return;
    }
    let node = match read_line(&mut reader) {
        Ok(Line::Node(node)) => node,
        // Only this account reaches the socket, so a release comes from the
        // core's own machine (`hided core-move release`).
        Ok(Line::Release(request)) => {
            let answer =
                service
                    .moves
                    .release(&request.intent, &request.target, RELEASE_ANSWER_WITHIN);
            herdr_core::diagnostic!(json!({
                "component": "core_move",
                "kind": "release.asked",
                "intent": request.intent,
                "target": request.target,
                "refused": answer.as_ref().err(),
            }));
            let line = match answer {
                Ok(()) => Line::Released,
                Err(reason) => Line::Refused(Refusal { reason }),
            };
            let _ = write_line(&mut writer, &line);
            return;
        }
        Ok(_) => {
            attach_failed("", "the node sent another line than its own");
            return;
        }
        Err(error) => {
            attach_failed("", &error.to_string());
            return;
        }
    };
    let refuse = |writer: &mut LocalStream, reason: &str| {
        herdr_core::diagnostic!(json!({
            "component": "node_link",
            "kind": "attach.refused",
            "node": node.node,
            "reason": reason,
            "node_build": node.build,
            "core_build": build,
            "node_release": node.release,
            "core_release": release,
        }));
        let _ = write_line(
            writer,
            &Line::Refused(Refusal {
                reason: reason.to_owned(),
            }),
        );
    };
    // The node decides the same before it says hello; this is the core's
    // own rule (D-10): only the same build links, and an older node is told
    // the core is newer so its app is the one updated. A newer node updates
    // this core over SSH before it dials, never across builds.
    match crate::build_order::standing((&node.build, &node.release), (&build, &release)) {
        crate::build_order::Standing::Same => {}
        crate::build_order::Standing::Older => {
            refuse(&mut writer, "core_newer");
            return;
        }
        crate::build_order::Standing::Newer | crate::build_order::Standing::Unordered => {
            refuse(&mut writer, "other_build");
            return;
        }
    }
    let Some(_attaching) = service.attaching.hold(&node.node) else {
        refuse(&mut writer, "already_linked");
        return;
    };
    if let Some(reason) = standing_refusal(service, &node.node) {
        refuse(&mut writer, &reason);
        return;
    }
    let Some(relay_token) = service.grants.issue(&node.node) else {
        refuse(&mut writer, "grants_full");
        return;
    };
    // The commit of a pending move, recorded before the node hears it: a
    // node that never reads the answer finds the move committed when it
    // asks.
    if let Err(reason) = service
        .move_gate
        .admit(&node.node, node.move_intent.as_deref())
    {
        service.grants.revoke(&relay_token);
        refuse(&mut writer, reason);
        return;
    }
    let accepted = Line::Accepted(Accepted {
        port: service.port,
        relay_token: relay_token.clone(),
    });
    if let Err(message) = write_line(&mut writer, &accepted) {
        service.grants.revoke(&relay_token);
        attach_failed(&node.node, &message);
        return;
    }
    // From here the stream carries the node's link and its screens'
    // streams (`link_mux`); nothing of the handshake is left unread, since
    // the node writes nothing more until it reads the acceptance.
    drop(reader);
    drop(writer);
    let _ = stream.set_read_timeout(None);
    let stream = match crate::link_mux::serve(stream) {
        Ok(link) => link,
        Err(error) => {
            service.grants.revoke(&relay_token);
            attach_failed(&node.node, &error.to_string());
            return;
        }
    };
    let established = inbound::establish(
        InboundNode {
            node: node.node.clone(),
            label: node.label.clone(),
            herdr_socket: node.herdr_socket.clone(),
        },
        stream,
        Some(hide_node::ssh::host::PaneHook {
            node: node.node.clone(),
            events: Arc::clone(&service.panes),
        }),
        Some(hide_node::ssh::host::TerminalHook {
            node: node.node.clone(),
            sink: Arc::clone(&service.terminals),
        }),
    );
    let (link, arrived) = match established {
        Ok(established) => established,
        Err(message) => {
            service.grants.revoke(&relay_token);
            attach_failed(&node.node, &message);
            return;
        }
    };
    service.grants.bind(&relay_token, link.clone());
    // The grant goes with its link, so a link that ended holds nothing of
    // its connection in the table.
    {
        let grants = Arc::clone(&service.grants);
        let closed = link.closed();
        let token = relay_token.clone();
        tokio::runtime::Handle::current().spawn(async move {
            closed.await;
            grants.revoke(&token);
        });
    }
    let link = link.identity();
    match service
        .core
        .accept_inbound_node(&node.node, &node.label, arrived)
    {
        Ok(()) => herdr_core::diagnostic!(json!({
            "component": "node_link",
            "kind": "attach.linked",
            "node": node.node,
            "link": link,
            "elapsed_ms": started.elapsed().as_millis() as u64,
        })),
        // The transport was dropped with the refusal, which closed the link.
        Err(reason) => {
            service.grants.revoke(&relay_token);
            herdr_core::diagnostic!(json!({
                "component": "node_link",
                "kind": "attach.refused",
                "node": node.node,
                "reason": reason,
            }));
        }
    }
}

/// Why a link from `node` is refused now. A node that dials again while
/// its earlier link still stands has most often lost that link without
/// either end seeing it close (a network change, D-09): the earlier link is
/// greeted, and one that does not answer within [`STALE_PROBE`] is ended so
/// this one takes its place, rather than wait out the attach role's silence
/// limit. An earlier link that answers keeps its place.
fn standing_refusal(service: &AttachService, node: &str) -> Option<String> {
    let reason = service.core.inbound_refusal(node)?;
    if reason != "already_linked" {
        return Some(reason);
    }
    // A link still being established is not superseded: its own attempt
    // decides it within its handshake.
    let Ok(earlier) = service.core.node_link(node) else {
        return Some(reason);
    };
    match earlier.call(hide_node_link::protocol::Call::Hello, STALE_PROBE) {
        Ok(_) => Some(reason),
        Err(error) => {
            herdr_core::diagnostic!(json!({
                "component": "node_link",
                "kind": "attach.superseded",
                "node": node,
                "error": error.to_string(),
            }));
            earlier.close("the node dialed again and this link did not answer");
            service.core.inbound_refusal(node)
        }
    }
}

fn attach_failed(node: &str, message: &str) {
    herdr_core::diagnostic!(json!({
        "component": "node_link",
        "kind": "attach.failed",
        "node": node,
        "message": message,
    }));
}

/// `hided attach [--state-dir <dir>]`: the attach role an SSH connection
/// from a node runs on the core's machine. Its standard input and output are
/// the node's channel.
pub fn run(args: &[OsString]) -> Result<(), String> {
    let state_dir = match args {
        [] => {
            let home = hide_platform::host::home_dir()
                .map_err(|error| format!("the attach role has no home folder: {error}"))?;
            hide_kit::layout::state_dir_from_process(&home)
        }
        [flag, dir] if flag == "--state-dir" => {
            let dir = PathBuf::from(dir);
            if !dir.is_absolute() {
                return Err("--state-dir must be an absolute path".to_owned());
            }
            dir
        }
        _ => return Err("usage: hided attach [--state-dir <absolute dir>]".to_owned()),
    };
    match find_core(&state_dir) {
        Ok(core) => pipe(core),
        // No core answers here: the attach role starts none (D-07) and says
        // why.
        Err(outcome) => {
            let mut stdout = std::io::stdout().lock();
            write_line(&mut stdout, &Line::Attach(outcome))
        }
    }
}

/// Asks the core running on `state_dir` to stop for the move back
/// `intent` to `target`; answers once the core recorded the stop, or its
/// refusal.
pub fn release(state_dir: &Path, intent: &str, target: &str) -> Result<(), String> {
    let stream = find_core(state_dir)
        .map_err(|outcome| format!("no core to release: {}", outcome.code()))?;
    let _ = stream.set_read_timeout(Some(RELEASE_ANSWER_WITHIN + HANDSHAKE_TIMEOUT));
    let mut writer = stream.duplicate();
    let mut reader = BufReader::new(stream);
    match read_line(&mut reader).map_err(|error| error.to_string())? {
        Line::Core(_) => {}
        _ => return Err("the core sent another line than its own".to_owned()),
    }
    write_line(
        &mut writer,
        &Line::Release(ReleaseRequest {
            intent: intent.to_owned(),
            target: target.to_owned(),
        }),
    )?;
    match read_line(&mut reader).map_err(|error| error.to_string())? {
        Line::Released => Ok(()),
        Line::Refused(refusal) => Err(refusal.reason),
        _ => Err("the core answered another line than the release's".to_owned()),
    }
}

/// The core's attach socket on this machine, as the core in `state_dir`
/// recorded it. A node that is not this account's own login cannot reach
/// the record: the state folder and the socket's folder are owner-only.
fn find_core(state_dir: &Path) -> Result<LocalStream, AttachOutcome> {
    let socket = hide_node::pane_proof::recorded_socket_path(&attach_record(state_dir)).map_err(
        |refusal| match refusal {
            RecordRefusal::Missing => AttachOutcome::NoCore,
            RecordRefusal::Stale => AttachOutcome::CoreNotAnswering,
            RecordRefusal::Untrusted => AttachOutcome::RecordUntrusted,
        },
    )?;
    LocalStream::connect(&socket).map_err(|_| AttachOutcome::CoreNotAnswering)
}

/// Pipes standard input to the core and the core to standard output until
/// either ends or the node falls silent, then exits the process, so no
/// direction is left waiting on the other.
fn pipe(core: LocalStream) -> ! {
    // Milliseconds since `started` on the monotonic clock, so a step of the
    // wall clock neither ends a healthy link nor hides a silent one.
    let started = Instant::now();
    let now_ms = move || started.elapsed().as_millis() as u64;
    let heard = Arc::new(AtomicU64::new(now_ms()));
    let (done, ended) = std::sync::mpsc::channel::<&'static str>();
    let mut to_core = core.duplicate();
    {
        let heard = Arc::clone(&heard);
        let done = done.clone();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut buffer = vec![0_u8; 64 * 1024];
            let reason = loop {
                match stdin.read(&mut buffer) {
                    Ok(0) => break "node_closed",
                    Ok(read) => {
                        heard.store(now_ms(), Ordering::Relaxed);
                        if to_core.write_all(&buffer[..read]).is_err() {
                            break "core_closed";
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break "node_closed",
                }
            };
            let _ = done.send(reason);
        });
    }
    {
        let done = done.clone();
        let mut from_core = core.duplicate();
        std::thread::spawn(move || {
            let mut stdout = std::io::stdout().lock();
            let mut buffer = vec![0_u8; 64 * 1024];
            let reason = loop {
                match from_core.read(&mut buffer) {
                    Ok(0) => break "core_closed",
                    Ok(read) => {
                        if stdout
                            .write_all(&buffer[..read])
                            .and_then(|()| stdout.flush())
                            .is_err()
                        {
                            break "node_closed";
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break "core_closed",
                }
            };
            let _ = done.send(reason);
        });
    }
    let limit = hide_node_link::panes::HEARTBEAT_LIMIT.as_millis() as u64;
    let reason = loop {
        match ended.recv_timeout(Duration::from_secs(1)) {
            Ok(reason) => break reason,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if now_ms().saturating_sub(heard.load(Ordering::Relaxed)) > limit {
                    break "node_silent";
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break "ended",
        }
    };
    core.shutdown_handle().shutdown();
    eprintln!(
        "{}",
        json!({"component": "node_link", "kind": "attach.ended", "reason": reason})
    );
    std::process::exit(0)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One node is attached by one link at a time: a second attach of it
    /// is refused until the first is taken or fails, and another node is
    /// never held up by it.
    #[test]
    fn a_node_is_attached_by_one_link_at_a_time() {
        let attaching = Attaching::default();
        let first = attaching.hold("node-a").expect("the first attach");
        assert!(attaching.hold("node-a").is_none());
        assert!(attaching.hold("node-b").is_some());
        drop(first);
        assert!(attaching.hold("node-a").is_some());
    }

    /// The attach role tells the node why it reached no core: none
    /// recorded, a recorded core that is gone, or a record this account
    /// does not keep private, which it never follows.
    #[test]
    fn the_attach_role_says_why_it_reached_no_core() {
        let state = tempfile::tempdir().unwrap();
        let outcome = |state: &Path| find_core(state).err();
        assert_eq!(outcome(state.path()), Some(AttachOutcome::NoCore));

        let (listener, _) = bind(state.path()).unwrap();
        assert_eq!(outcome(state.path()), None, "the recorded core answers");
        let record = attach_record(state.path());
        // A record others can read is refused; Windows keeps no mode bits.
        #[cfg(unix)]
        {
            let private = std::fs::metadata(&record).unwrap().permissions();
            let mut shared = private.clone();
            std::os::unix::fs::PermissionsExt::set_mode(&mut shared, 0o644);
            std::fs::set_permissions(&record, shared).unwrap();
            assert_eq!(outcome(state.path()), Some(AttachOutcome::RecordUntrusted));
            std::fs::set_permissions(&record, private).unwrap();
        }

        // The core ended without removing its record.
        drop(listener);
        assert!(record.exists());
        assert_eq!(outcome(state.path()), Some(AttachOutcome::CoreNotAnswering));
    }
}
