//! This daemon on a screen machine whose core runs elsewhere (PRD
//! core-host-node-remote-core D-02, D-05, D-06, D-08): it starts no core.
//! It keeps its node's link to the core ([`NodeRole`]) and serves its own
//! screens on loopback exactly as a core's daemon does, `/ws` included, so
//! the screen and the desktop host cannot tell the difference.
//!
//! Each screen's core traffic (events, snapshots and deltas, answers) goes
//! to the core through one relay per screen on the link's SSH connection.
//! Terminals are drawn from this daemon's own hub: this machine's panes
//! from its own terminals without the link, every other pane from the one
//! terminals relay the node keeps to the core. So a key into one of this
//! machine's panes and its output never leave the machine.
//!
//! Each screen's relay is read as fast as its core sends, whatever the
//! screen takes: a screen that stalls holds at most 4 MiB of core frames,
//! and past that it is closed as fallen behind and reattaches (D-20, B17),
//! so it never stops the SSH connection every other screen, pane and the
//! link share.
//!
//! Keys and redraws for the core's panes wait, in order, while the
//! terminals relay is not there to take them (it opens after the link, and
//! again after it fails): at most 64 KiB, each at most 3 s, as the core
//! holds keys typed before a pane exists. A key past the bound is refused
//! to the screen that typed it; one that waited too long, was held for a
//! link that ended, or whose write was cut off is dropped and that screen
//! told, once per burst with the count and the cause; none is lost unsaid.
//!
//! While the link is down a screen is held: its socket stays open, every
//! frame it sends is dropped and counted, and nothing reaches it until the
//! link is back; a screen that was attached when the link ended is closed
//! once, so its next attempt is the held one (amendment 4 of the plan's
//! review). Keys typed meanwhile are never delivered later (B8).

mod screen;
mod terminals;

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::{SinkExt, StreamExt};
use hide_node::terminal::OutputSink;
use hide_node_link::device::{ShownSignIn, SignInReply};
use hide_node_link::terminal::{
    KeyTarget, MAX_PANE_ID_BYTES, TerminalDown, TerminalLine, TerminalNode, TerminalUp,
    decode_base64, device_pane_prefix, encode_base64,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc, watch};
use tokio_tungstenite::tungstenite;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::attachments::Attachments;
use crate::boundary::{Boundary, Refusal, Root};
use crate::browser_routes::{BrowserRoutes, RouteRequest};
use crate::node_browser::NodeBrowser;
use crate::node_pages::NodePages;
use crate::node_role::{LiveLink, NodeIdentity, NodeRole, Phase};
use crate::node_uploads::{Handled, ScreenUploads};
use crate::placement::Placement;
use crate::screen_event::{self, Kind};
use crate::server::{
    CloseReason, FIRST_FRAME_TIMEOUT, Handshake, RELAY_GRANT_HEADER, check_origin, refuse,
    terminal_key,
};
use crate::state_file::{MAX_CLIENTS, SCHEMA_VERSION};
use crate::terminal_hub::{HubClient, Resume, TerminalHub};

use screen::screen;
use terminals::{OwnPanes, ScreenTerminals, keep_terminals_relay};

/// The largest message a relay of the core may send this daemon.
const RELAY_MAX_MESSAGE: usize = 16 * 1024 * 1024;

/// What the node role's screen server holds.
#[derive(Clone)]
pub struct NodeState {
    pub token: Arc<String>,
    pub allowed_origins: Arc<HashSet<String>>,
    pub ui_dir: Option<PathBuf>,
    pub version: &'static str,
    pub build: Option<Arc<str>>,
    /// The seat this role is mounted on; `/health` names its instance.
    pub seat: crate::seat::SeatParts,
    pub hub: Arc<TerminalHub>,
    pub terminals: Arc<ScreenTerminals>,
    pub live: watch::Receiver<Option<Arc<LiveLink>>>,
    pub phase: Arc<dyn Fn() -> Phase + Send + Sync>,
    /// The phase as it changes, which a held screen is told.
    pub phases: watch::Receiver<Phase>,
    /// The SSH alias this machine dials its core by, which a held screen
    /// names until the core's hello names its machine.
    pub core_alias: Arc<str>,
    /// Gives a failed core update this connection's attempt
    /// (`NodeRole::connect_again`).
    pub connect_again: Arc<dyn Fn() + Send + Sync>,
    /// Ends or restores the link at the operator's word from this
    /// machine's window (B16), recorded in the placement first.
    pub link_control: Arc<dyn Fn(LinkAction) -> Result<(), String> + Send + Sync>,
    /// What the dial a window's Retry or connect started asks, which a held
    /// screen is told (PRD system-ssh-device-link B33).
    pub sign_ins: watch::Receiver<Option<ShownSignIn>>,
    /// A held screen's answer to it; one for a question no longer asked
    /// reaches nothing.
    pub answer_sign_in: Arc<dyn Fn(u64, SignInReply) -> bool + Send + Sync>,
    /// Screens held while the link is down: when the last one leaves, a
    /// question still waiting is cancelled, so the attempt ends rather than
    /// wait for a screen nobody sees (B8).
    pub held_screens: Arc<AtomicUsize>,
    pub clients: Arc<AtomicUsize>,
    pub connections: Arc<AtomicU64>,
    pub last_client_gone: Arc<Mutex<Instant>>,
    pub shutdown: crate::ending::Ending,
    /// Frames screens sent while the link was down, dropped.
    pub held_frames: Arc<AtomicU64>,
    /// This machine's paths a screen may read without the core: the
    /// checkouts the core opened on this node over the live link.
    pub boundary: Arc<Boundary>,
    /// The Browser View routes this machine's desktop host resolves
    /// (`node_pages`).
    pub browser_routes: Arc<BrowserRoutes>,
    /// Desktop windows attached now; with none, every route is closed.
    pub desktop_screens: Arc<AtomicUsize>,
    /// Files this machine's screens paste or drop, staged here
    /// (`node_uploads`).
    pub attachments: Arc<Attachments>,
    /// This machine's desktop windows' browser gateways (`node_browser`).
    pub browser: Arc<NodeBrowser>,
}

/// What the operator asks of the link from this machine's window (PRD
/// core-host-node-move B16): a `core_link` event {action}. `retry` dials
/// again after a failure only the operator could fix, as a window's
/// connect does.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LinkAction {
    Disconnect,
    Reconnect,
    Retry,
}

/// The node role, running: its link and its screen server.
pub struct NodeDaemon {
    pub state: NodeState,
    role: Arc<NodeRole>,
    relay: tokio::task::JoinHandle<()>,
    reaper: tokio::task::JoinHandle<()>,
    browser: tokio::task::JoinHandle<()>,
}

impl NodeDaemon {
    /// Starts the link to the core `placement` names and the terminals
    /// relay that follows it; the caller serves [`router`] with `state`.
    pub fn start(
        home: &std::path::Path,
        placement: Placement,
        identity: NodeIdentity,
        server: ServerParts,
        updates: Option<crate::node_role::Updates>,
    ) -> Result<Self, String> {
        let hub = TerminalHub::new();
        let own_prefix = device_pane_prefix(&identity.node);
        let screen_node = identity.node.clone();
        let core_node = placement.node.clone();
        let core_alias: Arc<str> = placement.alias.as_str().into();
        let boundary = Arc::new(Boundary::for_node(
            home,
            herdr_core::node::NodeId::parse(&identity.node)?,
        )?);
        let browser = NodeBrowser::new(server.browser_relay_port);
        let role = Arc::new(NodeRole::start_for_screens(
            home,
            placement,
            identity,
            Arc::new(OwnPanes {
                hub: Arc::clone(&hub),
                prefix: own_prefix.clone(),
            }),
            Some(Arc::clone(&browser)),
            Some(roots_follow(Arc::clone(&boundary))),
            updates,
        )?);
        let live = role.live();
        let follow = browser.spawn_follow(live.clone());
        let terminals = Arc::new(ScreenTerminals::new(own_prefix.clone(), live.clone()));
        let relay = tokio::spawn(keep_terminals_relay(
            live.clone(),
            Arc::clone(&hub),
            Arc::clone(&terminals),
        ));
        // The screen server reads the phase without keeping the role: the
        // role ends with this daemon, whatever the server still holds.
        let phase = {
            let role = Arc::downgrade(&role);
            Arc::new(move || {
                role.upgrade().map_or_else(
                    || Phase::Waiting {
                        reason: crate::node_role::LinkFailure::Stopping,
                    },
                    |role| role.phase(),
                )
            }) as Arc<dyn Fn() -> Phase + Send + Sync>
        };
        let connect_again = {
            let role = Arc::downgrade(&role);
            Arc::new(move || {
                if let Some(role) = role.upgrade() {
                    role.connect_again();
                }
            }) as Arc<dyn Fn() + Send + Sync>
        };
        let link_control = {
            let role = Arc::downgrade(&role);
            let state_dir = server.state_dir.clone();
            let own_node = screen_node.clone();
            Arc::new(move |action: LinkAction| {
                let role = role.upgrade().ok_or("the node is stopping")?;
                if action == LinkAction::Retry {
                    role.connect_again();
                    return Ok(());
                }
                let disconnected = action == LinkAction::Disconnect;
                // Recorded first, so a restart keeps what the operator chose.
                let recorded = crate::placement::update(&state_dir, &own_node, |placement| {
                    placement.disconnected = disconnected;
                })?;
                if !recorded {
                    return Err("the placement record is gone".to_owned());
                }
                if disconnected {
                    role.disconnect();
                } else {
                    role.reconnect();
                }
                Ok(())
            }) as Arc<dyn Fn(LinkAction) -> Result<(), String> + Send + Sync>
        };
        let phases = role.phases();
        let sign_ins = role.sign_ins();
        let answer_sign_in = {
            let role = Arc::downgrade(&role);
            Arc::new(move |question: u64, reply: SignInReply| {
                role.upgrade()
                    .is_some_and(|role| role.answer_sign_in(question, reply))
            }) as Arc<dyn Fn(u64, SignInReply) -> bool + Send + Sync>
        };
        let browser_routes = BrowserRoutes::new(Arc::new(NodePages::new(
            screen_node,
            core_node,
            live.clone(),
            Arc::clone(&boundary),
        )));
        let desktop_screens = Arc::new(AtomicUsize::new(0));
        let reaper =
            browser_routes.spawn_reaper(Arc::clone(&desktop_screens), server.shutdown.clone());
        Ok(Self {
            state: NodeState {
                token: server.token,
                allowed_origins: server.allowed_origins,
                ui_dir: server.ui_dir,
                version: server.version,
                build: server.build,
                seat: server.seat,
                hub,
                terminals,
                live,
                phase,
                phases,
                core_alias,
                connect_again,
                link_control,
                sign_ins,
                answer_sign_in,
                held_screens: Arc::new(AtomicUsize::new(0)),
                clients: Arc::new(AtomicUsize::new(0)),
                connections: Arc::new(AtomicU64::new(0)),
                last_client_gone: Arc::new(Mutex::new(Instant::now())),
                boundary,
                shutdown: server.shutdown,
                held_frames: Arc::new(AtomicU64::new(0)),
                browser_routes,
                desktop_screens,
                attachments: Arc::new(Attachments::new(&server.state_dir)),
                browser,
            },
            role,
            relay,
            reaper,
            browser: follow,
        })
    }

    /// The link's phase, for a caller that waits on it.
    pub fn role(&self) -> &NodeRole {
        &self.role
    }

    /// The link's phase, for a caller that waits on it off this task.
    pub fn role_handle(&self) -> Arc<NodeRole> {
        Arc::clone(&self.role)
    }
}

impl Drop for NodeDaemon {
    fn drop(&mut self) {
        self.relay.abort();
        self.reaper.abort();
        self.browser.abort();
    }
}

/// What the screen server is given by the daemon that starts it.
pub struct ServerParts {
    pub token: Arc<String>,
    pub allowed_origins: Arc<HashSet<String>>,
    pub ui_dir: Option<PathBuf>,
    pub version: &'static str,
    pub build: Option<Arc<str>>,
    pub seat: crate::seat::SeatParts,
    pub shutdown: crate::ending::Ending,
    /// This machine's state folder, where its screens' uploads are staged.
    pub state_dir: PathBuf,
    /// The loopback port the browser relay listens on alone
    /// ([`relay_router`]).
    pub browser_relay_port: u16,
}

pub fn router(state: NodeState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/core-link/connect", post(connect_again))
        .route("/ws", get(ws_upgrade))
        .route(
            "/browser-route",
            post(resolve_browser_route).delete(release_browser_route),
        )
        .route(
            "/browser-control",
            post(crate::node_browser::register)
                .delete(crate::node_browser::release)
                .layer(axum::extract::DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/browser-control/action",
            post(crate::node_browser::action)
                .layer(axum::extract::DefaultBodyLimit::max(16 * 1024)),
        )
        .route("/", get(static_asset))
        .route("/assets/{*path}", get(static_asset))
        .fallback(|| async { axum::http::StatusCode::NOT_FOUND })
        .with_state(state)
}

/// The browser relay's own listener: one route, a relay ticket, and
/// nothing else of this daemon (`node_browser`).
pub fn relay_router(state: NodeState) -> Router {
    Router::new()
        .route("/browser-relay/{ticket}", get(crate::node_browser::relay))
        .fallback(|| async { axum::http::StatusCode::NOT_FOUND })
        .with_state(state)
}

/// What `hide connect` reads, as a core's daemon answers it, and where the
/// link to the core stands.
async fn health(State(state): State<NodeState>) -> impl IntoResponse {
    let phase = (state.phase)();
    let (link, reason) = match &phase {
        Phase::Connecting => ("connecting", None),
        Phase::Live(_) => ("live", None),
        Phase::Updating { .. } => ("updating", None),
        Phase::Waiting { reason } => ("waiting", Some(reason.to_string())),
        Phase::Disconnected => ("disconnected", None),
    };
    // The core's machine and both builds, which the window names when this
    // app is the one to update (B11) or its update of the core failed (B10).
    let builds = match &phase {
        Phase::Waiting {
            reason:
                crate::node_role::LinkFailure::CoreNewer {
                    machine,
                    release: core,
                }
                | crate::node_role::LinkFailure::UpdateFailed { machine, core, .. },
        } => Some(json!({
            "machine": machine,
            "core": core.shown(),
            "app": crate::build_order::Release::of_this_build().shown(),
        })),
        _ => None,
    };
    axum::Json(json!({
        "pid": std::process::id(),
        "version": state.version,
        "build": state.build.as_deref(),
        "schema_version": SCHEMA_VERSION,
        "clients": state.clients.load(Ordering::SeqCst),
        "open_handlers_in_flight": 0,
        "idle_remaining_secs": Value::Null,
        "role": "node",
        "instance": state.seat.instance(),
        "node": state.boundary.node().as_str(),
        "core_link": link,
        "core_link_reason": reason,
        "release": crate::build_order::Release::of_this_build(),
        "builds": builds,
    }))
}

/// A window connects to this node (`hide connect`): a core update that
/// failed on the last connection gets this one's attempt (B10, B20).
async fn connect_again(State(state): State<NodeState>, headers: HeaderMap) -> StatusCode {
    if !crate::server::bearer_matches(&headers, &state.token) {
        return StatusCode::UNAUTHORIZED;
    }
    (state.connect_again)();
    StatusCode::NO_CONTENT
}

async fn resolve_browser_route(
    State(state): State<NodeState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<RouteRequest>,
) -> Response {
    if !crate::server::bearer_matches(&headers, &state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    crate::browser_routes::resolve_answer(&state.browser_routes, request).await
}

async fn release_browser_route(
    State(state): State<NodeState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<RouteRequest>,
) -> Response {
    if !crate::server::bearer_matches(&headers, &state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    crate::browser_routes::release_answer(&state.browser_routes, request).await
}

async fn static_asset(uri: Uri, State(state): State<NodeState>) -> Response {
    crate::server::ui_asset(state.ui_dir.as_deref(), uri.path()).await
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<NodeState>,
) -> Response {
    let origin = headers
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    // A screen of this machine only: the node role serves no phone and
    // nothing through `tailscale serve`.
    let proxied = crate::server::via_tailnet(&headers);
    ws.on_upgrade(move |socket| screen(socket, state, origin, proxied))
}

type Upstream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Opens a relay of `mode` to the core through the link's forward.
async fn open_relay(link: &LiveLink, mode: &str) -> Result<Upstream, String> {
    let address = SocketAddr::from(([127, 0, 0, 1], link.relay_port));
    let mut request = format!("ws://{address}/relay?mode={mode}")
        .into_client_request()
        .map_err(|error| error.to_string())?;
    request.headers_mut().insert(
        RELAY_GRANT_HEADER,
        link.accepted
            .relay_token
            .parse()
            .map_err(|_| "the relay grant is not a header value".to_owned())?,
    );
    let config = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(RELAY_MAX_MESSAGE))
        .max_frame_size(Some(RELAY_MAX_MESSAGE));
    let (socket, _) = tokio::time::timeout(
        Duration::from_secs(15),
        tokio_tungstenite::connect_async_with_config(request, Some(config), true),
    )
    .await
    .map_err(|_| "the relay did not open in time".to_owned())?
    .map_err(|error| error.to_string())?;
    Ok(socket)
}

/// Keeps `boundary`'s roots at the checkouts the core opened on this node
/// over the live link, as they change: every read of a screen, and every
/// page, is judged against the same roots, never set by the read itself.
fn roots_follow(boundary: Arc<Boundary>) -> crate::node_role::RootsChanged {
    Arc::new(move |roots: &[String]| {
        boundary.set_roots(
            roots
                .iter()
                .map(|root| Root::opened_on_node(root))
                .collect(),
        );
    })
}

/// A screen's read of a file of this machine, under a checkout the core
/// opened here (PRD core-host-node-remote-core B4, D-05): the file, opened
/// under its root, or the refusal. `None` for a read of another machine's
/// file, and for a path under no root the core opened here, which the core
/// answers as it answers any device's.
fn own_file(state: &NodeState, event: &Value) -> Option<crate::server::OpenedFile> {
    let device = event
        .pointer("/payload/device_id")
        .and_then(Value::as_str)?;
    if state.boundary.node() != device {
        return None;
    }
    let path = event.pointer("/payload/path").and_then(Value::as_str)?;
    match state.boundary.open_file(path) {
        Err(Refusal::OutsideCheckout) => None,
        opened => Some(opened),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The node's boundary holds the checkout roots the core opened over
    /// the link as the core opens them, and none once a link ends: a
    /// screen's read is judged against them and never sets them (arch
    /// review item 8).
    #[test]
    fn the_node_s_boundary_follows_the_roots_the_core_opens() {
        let home = tempfile::tempdir().unwrap();
        let home = hide_platform::fs::identity::canonical(home.path()).unwrap();
        let checkout = home.join("app");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(checkout.join("a.txt"), "a").unwrap();
        let boundary = Arc::new(
            Boundary::for_node(&home, herdr_core::node::NodeId::parse("test-node").unwrap())
                .unwrap(),
        );
        let follow = roots_follow(Arc::clone(&boundary));
        let roots = {
            let follow = Arc::clone(&follow);
            hide_host::serve::OpenedRoots::telling(move |roots| follow(roots))
        };
        let file = checkout.join("a.txt").to_string_lossy().into_owned();
        assert!(matches!(
            boundary.open_file(&file),
            Err(Refusal::OutsideCheckout)
        ));
        roots.record(&checkout.to_string_lossy());
        assert!(boundary.open_file(&file).is_ok(), "the opened root is read");
        // The link ended.
        follow(&[]);
        assert!(matches!(
            boundary.open_file(&file),
            Err(Refusal::OutsideCheckout)
        ));
    }
}
