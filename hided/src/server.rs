use std::collections::HashSet;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::attachments::{self, Attachments};
use crate::boundary::{self, Boundary, Listing, Refusal, Unregistrable};
use crate::core::CoreHandle;
use crate::index::{IndexAnswer, IndexService};
use crate::pane_auth::Registry;
use crate::screen_event::{self, Kind};
use crate::state_file::{MAX_CLIENTS, SCHEMA_VERSION};
use crate::terminal_hub::Resume;
use crate::watch::WatchService;
use base64::Engine as _;
use hide_node::opener::OpenHandler;
use hide_node_link::terminal::{GridSize, KeyTarget, MAX_PANE_ID_BYTES, ScreenInput, TerminalNode};

const FALLBACK_INDEX: &str = include_str!("../fallback-ui/index.html");

/// The web shell a release binary carries; empty in a debug build, which
/// reads `web/dist` from disk instead (`build.rs`).
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/ui_embed.rs"));
}

/// Whether this binary carries the web shell (a release build).
pub fn has_embedded_ui() -> bool {
    !embedded::FILES.is_empty()
}

/// The embedded file for a request path: `/` is `index.html`, anything else
/// is an exact relative path, so a traversal segment never matches a key.
pub fn embedded_file(path: &str) -> Option<(&'static str, &'static [u8])> {
    let relative = path.trim_start_matches('/');
    let name = if relative.is_empty() {
        "index.html"
    } else {
        relative
    };
    embedded::FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(file, bytes)| (*file, *bytes))
}

#[derive(Clone)]
pub struct AppState {
    pub core: Arc<CoreHandle>,
    pub boundary: Arc<Boundary>,
    /// Follows the core's checkout roots into `boundary`; a refused event
    /// catches it up before it is answered.
    pub roots: Arc<crate::RootFollower>,
    /// The daemon's one watch service; every client subscribes to its frames.
    pub watch: Arc<WatchService>,
    /// The ⌘P index cache, one lazy index per registered checkout.
    pub index: Arc<IndexService>,
    /// Staged dropped files on their way to the core's attachment directory.
    pub attachments: Arc<Attachments>,
    /// One bounded, daemon-owned path for OS file associations.
    pub opener: OpenHandler,
    pub token: Arc<String>,
    pub pane_capabilities: Arc<Registry>,
    pub browser_routes: Arc<crate::browser_routes::BrowserRoutes>,
    pub browser_control: Arc<crate::browser_control::BrowserControl>,
    /// The operator's Comment mode on browser Views (PRD
    /// view-opener-feedback B9).
    pub browser_comments: Arc<crate::browser_page::comment::CommentSessions>,
    pub allowed_origins: Arc<HashSet<String>>,
    /// Every connection now, another machine's views among them: while any
    /// is, the daemon stays and its readers run.
    pub clients: Arc<AtomicUsize>,
    /// This machine's own connections (windows, tools, phones), the ones
    /// `MAX_CLIENTS` counts ([`LocalSeat`]); another machine's views have
    /// seats of their own (`remote_screen`).
    pub local_seats: Arc<AtomicUsize>,
    /// Authenticated shell windows, distinct from CLI and non-rendering clients.
    pub renderers: Arc<AtomicUsize>,
    pub desktop_renderers: Arc<AtomicUsize>,
    /// What draws this machine's browser displays: its desktop windows and
    /// every other machine's view whose desktop app draws them there
    /// (`draws_pages`). An agent's `hide browser open` needs one; the
    /// gateway, the page commands and the route reaper still need a desktop
    /// window, since a view gives no page control.
    pub page_renderers: Arc<AtomicUsize>,
    /// Numbers connections so a stage can be released with its connection.
    pub connections: Arc<AtomicU64>,
    pub last_client_gone: Arc<Mutex<Instant>>,
    pub keep_alive: bool,
    pub idle_secs: u64,
    /// This daemon's build, reported on `/health` for `hide connect`.
    pub build: Option<Arc<str>>,
    /// The program this core's starter runs, reported on `/health` so a
    /// `hide connect` on this machine updates the core rather than stopping
    /// it (PRD core-host-node-move Q13); `None` when no starter runs it.
    pub starter_program: Option<Arc<str>>,
    /// The program this daemon is, reported on `/health` so a raise of a
    /// core no starter started knows which build to keep for its way back
    /// (PRD device-connect-iroh B23).
    pub program: Option<Arc<str>>,
    /// Serializes a renderer count change with the `ui_attached` event it
    /// sends, so two windows coming and going at once cannot leave the core
    /// told the opposite of the final count.
    pub renderer_transitions: Arc<Mutex<()>>,
    pub shutdown: crate::ending::Ending,
    /// Turns true once, when this role ends (`RunningDaemon` dropped): every
    /// socket of the role is closed with 1012 `role_ended`, so a window
    /// reconnects to the role mounted next on the same seat rather than wait
    /// on a core that has stopped (PRD core-host-node-move B4).
    pub role_ended: tokio::sync::watch::Receiver<bool>,
    pub ui_dir: Option<PathBuf>,
    pub version: &'static str,
    /// The seat this role is mounted on; `/health` names its instance, so a
    /// client that registered with an earlier role registers again.
    pub seat: crate::seat::SeatParts,
    /// Which connections are looking at the Settings agents tab; the daemon
    /// owns the core's one observation flag on their behalf.
    pub demand: Arc<crate::demand::ObservationDemand>,
    /// Which connections show a start surface (the start panel, a start
    /// dialog's model menu, a phone's start sheet): the provider catalog is
    /// read while any does, without the Settings-only hook diagnosis (PRD
    /// home-device-rail D-18).
    pub start_demand: Arc<crate::demand::ObservationDemand>,
    /// Which connections show the Overview's request view: while any does,
    /// the core re-reads the pull requests whose checks are running (PRD
    /// overview-request-view D-32).
    pub request_view_demand: Arc<crate::demand::ObservationDemand>,
    /// Which windows hold the keyboard and draw the Workspace screen: the
    /// core counts a View in front of the operator as seen only while one
    /// does (PRD unseen-agent-views D-07, B6, B8).
    pub window_focus_demand: Arc<crate::demand::FocusDemand>,
    /// Which connections show which backlog item's origin: the core keeps
    /// an origin until the last of them closes it or leaves.
    pub origin_holds: Arc<crate::demand::OriginHolds>,
    /// Which windows show the core's session thread; the last to let go or
    /// leave closes it.
    pub thread_holds: Arc<crate::demand::ThreadHolds>,
    /// What Settings > About reads about this daemon, sent once after
    /// a handshake. No token, no environment beyond the paths it names.
    pub daemon_info: Arc<Value>,
    /// Settings › Devices and the phones it pairs (`mobile/`).
    pub mobile: Arc<crate::mobile::Mobile>,
    /// This machine's endpoint for other Hides (`peer_host`).
    pub peers: Arc<crate::peer_host::PeerHost>,
    /// Pairing this machine with other Hides, for Settings > Devices
    /// (`pairing`).
    pub pairing: Arc<crate::pairing::Pairing>,
    /// Other machines' work on this one (`guests`): a window closes a
    /// guest's pane through it.
    pub guests: Arc<crate::guests::Guests>,
    /// The desktop windows' agent notifications (`desktop_notices.rs`).
    pub desktop_notices: Arc<crate::desktop_notices::DesktopNotices>,
    /// The grants this core handed linked nodes' screen relays (`relay`).
    pub relay_grants: Arc<crate::attach::RelayGrants>,
    /// Other machines' views of this core, and this machine's views of
    /// theirs (`remote_screen`).
    pub remote_screens: Arc<crate::remote_screen::RemoteScreens>,
    /// The way a paired machine's browser pages drawn here leave through
    /// it, and its half for this machine's pages drawn there (`screen_net`).
    pub screen_net: Arc<crate::screen_net::ScreenNet>,
    /// Windows of this machine open now (`OwnScreen`).
    pub own_screens: Arc<Mutex<usize>>,
    /// Which screen each pane's terminal size follows (`pane_sizes`).
    pub pane_sizes: Arc<crate::pane_sizes::PaneSizes>,
    /// What the pane bootstrap's and the node attach socket's accept threads
    /// do, for `/health`.
    pub listeners: Arc<crate::accepting::CoreListeners>,
    /// A `hide <path>` that brought a project in front asks this machine's
    /// windows to come forward too, showing it (`project_open`): each sends
    /// its screen a `window_call` frame with this payload.
    pub window_calls: tokio::sync::broadcast::Sender<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Handshake {
    pub(crate) token: String,
    pub(crate) schema_version: u32,
    pub(crate) client_kind: Option<String>,
    pub(crate) have_revision: Option<u64>,
    /// Absent keeps the schema-2 complete-rest transition contract.
    pub(crate) snapshot_format: Option<String>,
    /// The `core_instance` of the daemon frame before the snapshot the
    /// client's revision came from. A revision counts only on the core that
    /// numbered it: a core moved, rolled back or updated numbers its own.
    pub(crate) have_core: Option<String>,
    /// The last `terminal` frame's cursor the client applied, which resumes
    /// every pane's output from the terminal hub that gave it.
    pub(crate) have_terminal_sequence: Option<u64>,
    /// The epoch of the hub that gave the cursor.
    pub(crate) have_terminal_epoch: Option<String>,
    /// Another machine's view of this one whose desktop app draws this
    /// machine's browser displays in it (`page_renderers`); a window of this
    /// machine says nothing, and an older guest's view never says it.
    pub(crate) draws_pages: Option<bool>,
}

impl Handshake {
    fn supported_snapshot_format(&self) -> bool {
        self.snapshot_format
            .as_deref()
            .is_none_or(|format| format == "semantic-rest-v1")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseReason {
    InvalidToken,
    OriginNotAllowed,
    SchemaMismatch,
    ClientLimit,
}

impl CloseReason {
    pub fn code(self) -> u16 {
        match self {
            Self::InvalidToken => 4001,
            Self::OriginNotAllowed => 4002,
            Self::SchemaMismatch => 4003,
            Self::ClientLimit => 4004,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::InvalidToken => "invalid_token",
            Self::OriginNotAllowed => "origin_not_allowed",
            Self::SchemaMismatch => "schema_mismatch",
            Self::ClientLimit => "client_limit",
        }
    }
}

/// The close a socket gets when its role ends; the page reconnects on it.
fn role_ended_frame() -> CloseFrame {
    CloseFrame {
        code: 1012,
        reason: "role_ended".into(),
    }
}

mod event_class;
mod relay_routes;

use event_class::Client;

pub use relay_routes::RELAY_GRANT_HEADER;
use relay_routes::{
    RELAY_TAKES, RelayScreen, relay_attachment, relay_browser_control,
    relay_browser_control_action, relay_browser_source, relay_busy, relay_upgrade,
};

pub fn router(state: AppState) -> Router {
    let browser_control_limit = axum::extract::DefaultBodyLimit::max(16 * 1024);
    Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_upgrade))
        .route("/relay", get(relay_upgrade))
        .route("/relay/browser-source", post(relay_browser_source))
        .route(
            "/relay/browser-control",
            post(relay_browser_control).layer(browser_control_limit),
        )
        .route(
            "/relay/browser-control/action",
            post(relay_browser_control_action).layer(browser_control_limit),
        )
        .route(
            "/browser-route",
            post(resolve_browser_route).delete(release_browser_route),
        )
        .route("/hide-ai-wake", post(hide_ai_wake))
        .route(
            "/browser-control",
            post(register_browser_control)
                .delete(release_browser_control)
                .layer(browser_control_limit),
        )
        .route(
            "/browser-control/action",
            post(browser_control_action).layer(browser_control_limit),
        )
        .route("/", get(static_asset))
        .route("/assets/{*path}", get(static_asset))
        // The desktop app's view of a paired machine (`remote_screen`).
        .route(
            "/peer-screen/{device}/ws",
            get(crate::remote_screen::guest_socket),
        )
        .route(
            "/peer-screen/{device}/net",
            post(crate::remote_screen::guest_net).delete(crate::remote_screen::guest_net),
        )
        .route(
            "/peer-screen/{device}/page-route",
            post(crate::remote_screen::guest_page_route),
        )
        .route(
            "/peer-screen/{device}/",
            get(crate::remote_screen::guest_page),
        )
        .route(
            "/peer-screen/{device}/{*path}",
            get(crate::remote_screen::guest_asset),
        )
        .route("/m", get(mobile_asset))
        .route("/m/", get(mobile_asset))
        .route("/m/{*path}", get(mobile_asset))
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(axum::middleware::from_fn(tailnet_gate))
        .with_state(state)
}

/// Whether a request came through `tailscale serve` rather than from this
/// Mac. serve forwards the tailnet Host and adds X-Forwarded-For, while every
/// request, proxied or not, reaches hided from 127.0.0.1; a tailnet caller
/// that is not a browser may also send any Origin it likes (PRD D-16).
pub fn via_tailnet(headers: &HeaderMap) -> bool {
    if headers.contains_key("x-forwarded-for") || headers.contains_key("tailscale-user-login") {
        return true;
    }
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let name = match host.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    };
    !matches!(name, "127.0.0.1" | "localhost" | "[::1]")
}

/// A request from the tailnet reaches only the phone app's static shell,
/// its hashed assets and `/ws`, where only a phone handshake is accepted.
async fn tailnet_gate(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if via_tailnet(request.headers()) {
        let path = request.uri().path();
        let allowed = path == "/ws"
            || path == "/m"
            || path.starts_with("/m/")
            || path.starts_with("/assets/");
        if !allowed {
            return StatusCode::NOT_FOUND.into_response();
        }
    }
    next.run(request).await
}

/// Whether `headers` carry this daemon's token, as the desktop host sends
/// it for a route or a browser control.
pub(crate) fn bearer_matches(headers: &HeaderMap, token: &str) -> bool {
    let offered = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("");
    token_matches(offered, token)
}

fn browser_route_authorized(headers: &HeaderMap, state: &AppState) -> bool {
    bearer_matches(headers, &state.token)
}

async fn resolve_browser_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<crate::browser_routes::RouteRequest>,
) -> Response {
    if !browser_route_authorized(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    crate::browser_routes::resolve_answer(&state.browser_routes, request).await
}

/// The desktop host's word that the app opened, the machine woke or the
/// screen unlocked (PRD hide-ai-suggestions B6, D-34): one core event, which
/// asks the day's suggestion runs to start a few minutes later.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HideAiWake {
    cause: String,
}

async fn hide_ai_wake(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<HideAiWake>,
) -> Response {
    if !bearer_matches(&headers, &state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !matches!(request.cause.as_str(), "app_open" | "resume" | "unlock") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let event = json!({
        "schema_version": hide_core::SCHEMA_VERSION,
        "kind": "hide_ai_wake",
        "payload": {"cause": request.cause},
    });
    let core = Arc::clone(&state.core);
    match tokio::task::spawn_blocking(move || core.dispatch(event.to_string().into_bytes())).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

async fn release_browser_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<crate::browser_routes::RouteRequest>,
) -> Response {
    if !browser_route_authorized(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    crate::browser_routes::release_answer(&state.browser_routes, request).await
}

fn browser_control_failure((reason, next_action): crate::browser_control::Failure) -> Response {
    hide_core::diagnostic!(
        json!({"component":"browser_control","kind":"request.refused","reason":reason})
    );
    (
        StatusCode::CONFLICT,
        axum::Json(json!({"ok":false,"reason":reason,"next_action":next_action})),
    )
        .into_response()
}

async fn register_browser_control(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<crate::browser_control::Registration>,
) -> Response {
    if !browser_route_authorized(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.browser_control.register(request) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(failure) => browser_control_failure(failure),
    }
}

async fn release_browser_control(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<crate::browser_control::Registration>,
) -> Response {
    if !browser_route_authorized(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.browser_control.release(&request) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(failure) => browser_control_failure(failure),
    }
}

/// A desktop gateway never writes View state itself. Its authenticated,
/// scoped page action uses the same prepare/read/commit boundary as hide CLI.
async fn browser_control_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<crate::browser_control::BrowserAction>,
) -> Response {
    if !browser_route_authorized(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    run_browser_action(state, request, |control, request| {
        control.caller(request.owner_pid, &request.checkout_path)
    })
    .await
}

/// A window's page action, run as the caller `caller` names for it.
async fn run_browser_action(
    state: AppState,
    request: crate::browser_control::BrowserAction,
    caller: impl FnOnce(
        &crate::browser_control::BrowserControl,
        &crate::browser_control::BrowserAction,
    ) -> Result<String, crate::browser_control::Failure>
    + Send
    + 'static,
) -> Response {
    use hide_core::workspace_control::{ActionPreparation, Query};
    if !request.valid() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let slot = match state.browser_control.acquire() {
        Ok(slot) => slot,
        Err(failure) => return browser_control_failure(failure),
    };
    let result = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        if state.desktop_renderers.load(Ordering::SeqCst) == 0 {
            return Err(("browser_unsupported", "Open the Hide desktop app and retry"));
        }
        let caller = caller(&state.browser_control, &request)?;
        let source = state
            .core
            .workspace_query(&request.device_id, &caller, Query::ViewList)
            .map_err(|refusal| (refusal.reason, refusal.next_action))?;
        let action = request.command(&source)?;
        let preparation = state
            .core
            .workspace_prepare_action(
                &request.device_id,
                &caller,
                &source.context,
                &request.request_id,
                &action,
            )
            .map_err(|refusal| (refusal.reason, refusal.next_action))?;
        let material = match preparation {
            ActionPreparation::Cached(result) => {
                return result.map_err(|refusal| (refusal.reason, refusal.next_action));
            }
            ActionPreparation::Ready => {
                request.validate_scope(&source)?;
                Ok(None)
            }
            ActionPreparation::Read(read) => {
                request.validate_scope(&source)?;
                read.read().map(Some)
            }
        };
        state
            .core
            .workspace_action(
                &request.device_id,
                &caller,
                &source.context,
                &request.request_id,
                action,
                material,
            )
            .map_err(|refusal| (refusal.reason, refusal.next_action))
    })
    .await;
    match result {
        Ok(Ok(result)) => axum::Json(json!({"ok":true,"result":result})).into_response(),
        Ok(Err(failure)) => browser_control_failure(failure),
        Err(_) => browser_control_failure((
            "browser_control_unavailable",
            "Reconnect the desktop app and retry",
        )),
    }
}

fn idle_remaining_secs(state: &AppState) -> Option<u64> {
    if state.keep_alive {
        return None;
    }
    if state.clients.load(Ordering::SeqCst) > 0 {
        return Some(state.idle_secs);
    }
    let gone = *state.last_client_gone.lock().expect("client timestamp");
    Some(state.idle_secs.saturating_sub(gone.elapsed().as_secs()))
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    axum::Json(json!({
        "pid": std::process::id(),
        "role": "core",
        "version": state.version,
        "release": crate::build_order::Release::of_this_build(),
        "build": state.build.as_deref(),
        "starter": state.starter_program.as_deref().map(|program| json!({"program": program})),
        "program": state.program.as_deref(),
        "schema_version": SCHEMA_VERSION,
        "clients": state.clients.load(Ordering::SeqCst),
        // What draws this machine's browser displays now: its desktop
        // windows and the views of it whose app draws pages (`draws_pages`).
        "page_renderers": state.page_renderers.load(Ordering::SeqCst),
        "open_handlers_in_flight": state.opener.in_flight(),
        "idle_remaining_secs": idle_remaining_secs(&state),
        "instance": state.seat.instance(),
        "listeners": state.listeners.report(),
    }))
}

fn confined_file(root: &std::path::Path, relative: &str) -> Option<std::path::PathBuf> {
    if relative.split(['/', '\\']).any(|segment| segment == "..") {
        return None;
    }
    let candidate = if relative.is_empty() {
        root.join("index.html")
    } else {
        root.join(relative)
    };
    let root = root.canonicalize().ok()?;
    let file = candidate.canonicalize().ok()?;
    file.starts_with(&root).then_some(file)
}

async fn static_asset(uri: Uri, State(state): State<AppState>) -> Response {
    ui_asset(state.ui_dir.as_deref(), uri.path()).await
}

/// The web shell's file at `path`: embedded in a release build, read from
/// the confined `ui_dir` in a debug build, and the fallback page at `/`.
pub(crate) async fn ui_asset(ui_dir: Option<&Path>, path: &str) -> Response {
    if has_embedded_ui() {
        return match embedded_file(path) {
            Some((name, bytes)) => Response::builder()
                .status(StatusCode::OK)
                .header("content-type", mime_for(std::path::Path::new(name)))
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }
    if let Some(dir) = ui_dir {
        let relative = path.trim_start_matches('/');
        if let Some(candidate) = confined_file(dir, relative)
            && candidate.is_file()
        {
            return match tokio::fs::read(&candidate).await {
                Ok(bytes) => Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", mime_for(&candidate))
                    .body(Body::from(bytes))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            };
        }
        if (relative.is_empty() || !relative.contains('.'))
            && let Some(index) = confined_file(dir, "index.html")
            && let Ok(bytes) = tokio::fs::read(index).await
        {
            return Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/html; charset=utf-8")
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }
    if path == "/" {
        return Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/html; charset=utf-8")
            .body(Body::from(FALLBACK_INDEX))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    StatusCode::NOT_FOUND.into_response()
}

/// One file of the built web app by its path under `web/dist`: embedded in
/// a release build, read from the confined UI folder in a debug build.
async fn ui_file(state: &AppState, relative: &str) -> Option<Response> {
    let (content_type, bytes): (&'static str, Vec<u8>) = if has_embedded_ui() {
        let (name, bytes) = embedded_file(relative)?;
        (mime_for(std::path::Path::new(name)), bytes.to_vec())
    } else {
        let candidate = confined_file(state.ui_dir.as_deref()?, relative)?;
        if !candidate.is_file() {
            return None;
        }
        (
            mime_for(&candidate),
            tokio::fs::read(&candidate).await.ok()?,
        )
    };
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", content_type);
    // The service worker and the manifest are read fresh so a new build
    // replaces the cached app (PRD D-16).
    if relative.ends_with("sw.js")
        || relative.ends_with(".webmanifest")
        || relative.ends_with(".html")
    {
        response = response.header("cache-control", "no-cache");
    }
    Some(
        response
            .body(Body::from(bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    )
}

/// The phone app (PRD D-16): `/m/` is `mobile.html` and `/m/<file>` is
/// `m/<file>` (manifest, service worker, icons); its scripts and styles are
/// the shared `/assets`. Runtime data never comes from here, only over `/ws`.
async fn mobile_asset(uri: Uri, State(state): State<AppState>) -> Response {
    let path = uri.path();
    if path == "/m" {
        return Response::builder()
            .status(StatusCode::PERMANENT_REDIRECT)
            .header(header::LOCATION, "/m/")
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    let rest = path.trim_start_matches("/m/");
    let relative = if rest.is_empty() {
        "mobile.html".to_owned()
    } else {
        format!("m/{rest}")
    };
    ui_file(&state, &relative)
        .await
        .unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    let origin = headers
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(512)
        .collect::<String>();
    let proxied = via_tailnet(&headers);
    // A phone's frames are small; a tailnet caller may not buffer more.
    let ws = if proxied {
        ws.max_message_size(TAILNET_MAX_MESSAGE)
            .max_frame_size(TAILNET_MAX_MESSAGE)
    } else {
        ws
    };
    ws.on_upgrade(move |socket| client_loop(socket, state, origin, user_agent, proxied))
}

/// Serves one device pane's command stream (`node_panes`): HTTP over the
/// bytes the device's node relays up its link, with one route, `/ws`, which
/// takes only a credential `node` vouched for over `link`. A browser never
/// reaches it, so it has no Origin to check, and the shell's token, a phone
/// and a credential of this machine or of another device are refused.
/// `attested` is the caller the node proved when the stream opened, which
/// the command's check reads in place of asking the node again.
pub async fn serve_link_connection(
    state: AppState,
    node: String,
    link: hide_node::ssh::RemoteHost,
    attested: Option<crate::pane_auth::StreamAttestation>,
    io: tokio::io::DuplexStream,
) {
    let route = LinkRoute {
        state,
        node,
        link,
        attested: attested.map(Arc::new),
    };
    let router = Router::new()
        .route("/ws", get(link_ws_upgrade))
        .with_state(route);
    let service = hyper_util::service::TowerToHyperService::new(router);
    let _ = hyper::server::conn::http1::Builder::new()
        .serve_connection(hyper_util::rt::TokioIo::new(io), service)
        .with_upgrades()
        .await;
}

#[derive(Clone)]
struct LinkRoute {
    state: AppState,
    node: String,
    link: hide_node::ssh::RemoteHost,
    attested: Option<Arc<crate::pane_auth::StreamAttestation>>,
}

async fn link_ws_upgrade(ws: WebSocketUpgrade, State(route): State<LinkRoute>) -> Response {
    ws.on_upgrade(move |socket| link_client_loop(socket, route))
}

async fn link_client_loop(mut socket: WebSocket, route: LinkRoute) {
    let LinkRoute {
        state,
        node,
        link,
        attested,
    } = route;
    let handshake = match tokio::time::timeout(FIRST_FRAME_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Handshake>(&text).ok(),
        _ => None,
    };
    let Some(handshake) = handshake else {
        refuse(&mut socket, CloseReason::InvalidToken, None).await;
        return;
    };
    if handshake.schema_version != SCHEMA_VERSION || !handshake.supported_snapshot_format() {
        refuse(&mut socket, CloseReason::SchemaMismatch, None).await;
        return;
    }
    match state
        .pane_capabilities
        .vouched(&handshake.token, &node, &link)
    {
        Some(capability) => {
            let connection = state.connections.fetch_add(1, Ordering::SeqCst);
            scoped_client_loop(
                socket,
                state,
                connection,
                handshake.token,
                capability.one_shot,
                Some((node, link)),
                attested,
            )
            .await;
        }
        _ => refuse(&mut socket, CloseReason::InvalidToken, None).await,
    }
}

/// The largest message a WebSocket that came through `tailscale serve` may send.
const TAILNET_MAX_MESSAGE: usize = 64 * 1024;
/// How long a new WebSocket may wait before its first frame.
pub(crate) const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn check_origin(
    origin: Option<&str>,
    allowed: &HashSet<String>,
) -> Result<(), CloseReason> {
    let Some(origin) = origin else {
        return Err(CloseReason::OriginNotAllowed);
    };
    if allowed.iter().any(|allowed| allowed == origin) {
        Ok(())
    } else {
        Err(CloseReason::OriginNotAllowed)
    }
}

async fn client_loop(
    mut socket: WebSocket,
    state: AppState,
    origin: Option<String>,
    user_agent: String,
    proxied: bool,
) {
    // This Mac's tailnet address is an allowed Origin only while Mobile is
    // exposed, and only for a phone: the shell token is never accepted from
    // it, nor from anything that came through `tailscale serve` (PRD D-16).
    // A proxied request is never loopback, whatever Origin it claims: a
    // client outside a browser can write any Origin, so through serve only
    // the Mac's tailnet Origin counts, and only while Mobile is exposed.
    let loopback = !proxied && check_origin(origin.as_deref(), &state.allowed_origins).is_ok();
    let tailnet = !loopback && state.mobile.origin_allowed(origin.as_deref());
    if !loopback && !tailnet {
        refuse(&mut socket, CloseReason::OriginNotAllowed, None).await;
        return;
    }
    // A window that reconnects after the role ended, before the next role is
    // mounted, is sent on at once.
    let ended = *state.role_ended.borrow();
    if ended {
        let _ = socket.send(Message::Close(Some(role_ended_frame()))).await;
        return;
    }
    let connection = state.connections.fetch_add(1, Ordering::SeqCst);
    let first = match tokio::time::timeout(FIRST_FRAME_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        _ => {
            refuse(&mut socket, CloseReason::InvalidToken, None).await;
            return;
        }
    };
    let first_value = serde_json::from_str::<Value>(&first).unwrap_or(Value::Null);
    if crate::mobile::phone::is_phone(&first_value) {
        let _seat = match LocalSeat::take(&state) {
            Ok(seat) => seat,
            Err(count) => {
                refuse(&mut socket, CloseReason::ClientLimit, Some(count)).await;
                return;
            }
        };
        state.clients.fetch_add(1, Ordering::SeqCst);
        crate::mobile::phone::serve(
            socket,
            first_value,
            Arc::clone(&state.mobile),
            connection,
            user_agent,
        )
        .await;
        client_gone(&state, connection, false, false);
        return;
    }
    if !loopback {
        refuse(&mut socket, CloseReason::OriginNotAllowed, None).await;
        return;
    }
    let handshake: Handshake = match serde_json::from_value(first_value) {
        Ok(value) => value,
        Err(_) => {
            refuse(&mut socket, CloseReason::InvalidToken, None).await;
            return;
        }
    };
    if handshake.schema_version != SCHEMA_VERSION || !handshake.supported_snapshot_format() {
        refuse(&mut socket, CloseReason::SchemaMismatch, None).await;
        return;
    }
    if !token_matches(&handshake.token, &state.token) {
        if let Some(capability) = state.pane_capabilities.get(&handshake.token) {
            scoped_client_loop(
                socket,
                state,
                connection,
                handshake.token,
                capability.one_shot,
                None,
                None,
            )
            .await;
        } else {
            refuse(&mut socket, CloseReason::InvalidToken, None).await;
        }
        return;
    }
    // `hide <path>` on this machine: one request, never a screen.
    if handshake.client_kind.as_deref() == Some("cli") {
        crate::project_open::operator_request(socket, state, connection).await;
        return;
    }
    screen_loop(socket, state, connection, handshake, None, None).await;
}

/// Another machine's view of this core, over its screen stream (PRD D-53,
/// D-67): it attaches with the grant its stream runs under in place of this
/// machine's token, which its page never holds, and takes a view's seat,
/// apart from this machine's windows.
pub(crate) async fn remote_client_loop(
    mut socket: WebSocket,
    state: AppState,
    screens: Arc<crate::remote_screen::RemoteScreens>,
    viewer: crate::remote_screen::Viewer,
) {
    let connection = state.connections.fetch_add(1, Ordering::SeqCst);
    let handshake = match tokio::time::timeout(FIRST_FRAME_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Handshake>(&text).ok(),
        _ => None,
    };
    let Some(handshake) = handshake else {
        refuse(&mut socket, CloseReason::InvalidToken, None).await;
        return;
    };
    if handshake.schema_version != SCHEMA_VERSION || !handshake.supported_snapshot_format() {
        refuse(&mut socket, CloseReason::SchemaMismatch, None).await;
        return;
    }
    let seat = match screens.seat(viewer) {
        Ok(seat) => seat,
        Err(crate::remote_screen::SeatRefused::Full) => {
            refuse(
                &mut socket,
                CloseReason::ClientLimit,
                Some(screens.seated() + 1),
            )
            .await;
            return;
        }
    };
    hide_core::diagnostic!(json!({
        "component": "remote_screen", "kind": "view.opened", "connection": connection,
        "device": seat.viewer.device, "grant": seat.viewer.grant,
        "generation": seat.viewer.generation,
    }));
    screen_loop(socket, state, connection, handshake, None, Some(seat)).await;
}

/// The `mobile` frame for a screen. A screen another machine shows of this
/// one gets no pairing code: a phone pairs only from this machine's own
/// window or terminal, and only removing a phone is open to that screen (PRD
/// device-connect-iroh D-53, D-70).
fn mobile_frame(payload: &Value, own_screen: bool) -> Value {
    let mut payload = payload.clone();
    if !own_screen && let Some(fields) = payload.as_object_mut() {
        fields.insert("qr".to_owned(), Value::Null);
        fields.insert("code_expires_at_ms".to_owned(), Value::Null);
    }
    json!({"type": "mobile", "payload": payload})
}

/// Whether a `mobile_*` event from a screen goes on to Mobile. A screen
/// another machine shows of this one may send only the kinds the class table
/// opens to any window (`event_class`, the one list: removing a phone,
/// retrying the relay, setting when phones are notified, watching the tab);
/// everything else is refused here, at the connection (PRD
/// device-connect-iroh D-53, D-70): making or ending a code, and Mobile's
/// switch and method, which decide whether a phone can pair and end the code
/// a person at this machine is pairing with. Its watching of Settings >
/// Mobile is not counted, since the last watcher leaving ends that code too
/// (`Mobile::release`). A kind the table does not open is refused, so a new
/// control is this machine's own until it is let in.
fn mobile_event_reaches(kind: &str, own_screen: bool) -> Result<bool, String> {
    if own_screen {
        return Ok(true);
    }
    if kind == "mobile_observe" {
        return Ok(false);
    }
    match event_class::class_of(kind) {
        Some(event_class::Class::Window) => Ok(true),
        _ => Err(format!(
            "{kind} is for this machine's own screen: a screen of another machine removes phones but never pairs one"
        )),
    }
}

/// One screen's session after its handshake: the snapshot stream, the
/// answers to its events and, for a screen of this machine, its terminals.
/// A linked node's screen (`relay`) draws terminals from its own node's hub
/// and ends with the node's link.
async fn screen_loop(
    mut socket: WebSocket,
    state: AppState,
    connection: u64,
    handshake: Handshake,
    relay: Option<RelayScreen>,
    seat: Option<crate::remote_screen::Seat>,
) {
    let remote = seat.is_some();
    // Another machine's views have their own seats (`remote_screen`) and
    // never take a place of this machine's windows.
    let _local = if remote {
        None
    } else {
        match LocalSeat::take(&state) {
            Ok(local) => Some(local),
            Err(count) => {
                refuse(&mut socket, CloseReason::ClientLimit, Some(count)).await;
                return;
            }
        }
    };
    let previous = state.clients.fetch_add(1, Ordering::SeqCst);
    if previous == 0 {
        state.roots.resume();
    }
    // A view draws like a window whatever its page says it is; it is never
    // this machine's desktop window.
    let renderer = remote || matches!(handshake.client_kind.as_deref(), Some("web" | "desktop"));
    let desktop = !remote && handshake.client_kind.as_deref() == Some("desktop");
    // A window of this machine: what lets a machine in or brings a project
    // forward reaches only such a window.
    let own = renderer && relay.is_none() && !remote;
    let client = match (remote, renderer) {
        (true, _) => Client::Remote,
        (false, true) => Client::Window { desktop },
        (false, false) => Client::Tool,
    };
    if renderer {
        renderer_count(&state, true);
    }
    if desktop {
        state.desktop_renderers.fetch_add(1, Ordering::SeqCst);
    }
    let _draws_pages = (desktop || (remote && handshake.draws_pages == Some(true)))
        .then(|| PageRenderer::arrive(&state.page_renderers));
    // A window of this machine draws linked nodes' panes from this hub, so
    // they send their output up while one is open (`Router::mirror`).
    let _own_screen = (renderer && relay.is_none()).then(|| OwnScreen::arrive(&state));
    // A reconnecting client resumes from the cursor it last applied, so the
    // first frame carries only what changed while it was away; a fresh client
    // (cursor 0) gets the whole state.
    let own_core = state
        .daemon_info
        .get("core_instance")
        .and_then(Value::as_str);
    let mut have_revision = match handshake.have_core.as_deref() {
        Some(core) if Some(core) != own_core => 0,
        _ => handshake.have_revision.unwrap_or(0),
    };
    let granular = handshake.snapshot_format.as_deref() == Some("semantic-rest-v1");
    let mut notify = state.core.notify.subscribe();
    // A change in a watched folder is announced on this socket beside the
    // snapshot stream; the client re-reads the one folder it names (B2).
    let mut directory_changes = state.watch.subscribe();
    // Answers a device's helper gives for this client alone (the Explorer's
    // listing of a device checkout) arrive here from their own tasks, so a
    // slow device never holds up this socket's snapshot stream.
    let (device_frames_tx, mut device_frames) = tokio::sync::mpsc::channel::<String>(16);
    // Device file reads for this client, each in its own task: their frames
    // come back through `device_bytes`, at most two ranges ahead of the
    // socket, and every read ends with the connection.
    let (device_bytes_tx, mut device_bytes) = tokio::sync::mpsc::channel::<Message>(2);
    // The operator's comments and Sends from this screen, in the order it
    // made them, off the socket loop; started by the first one.
    let mut feedback: Option<FeedbackQueue> = None;
    // This screen's search across every project, and its asks of agents,
    // each one at a time, off the socket loop.
    let file_search = OneAtATime::<Option<Value>>::default();
    let screen_asks = OneAtATime::<ScreenAsks>::default();
    let mut device_reads: std::collections::VecDeque<DeviceRead> =
        std::collections::VecDeque::new();
    let daemon = json!({"type": "daemon", "payload": state.daemon_info.as_ref()});
    if socket
        .send(Message::Text(daemon.to_string().into()))
        .await
        .is_err()
    {
        client_gone(&state, connection, renderer, desktop);
        return;
    }
    let mut window_calls = state.window_calls.subscribe();
    // A desktop window shows the agent notifications (`agent_notice`); a
    // browser tab never does (PRD first-run-onboarding D-14), so it keeps no
    // receiver that would hold frames it never reads.
    let mut agent_notices = desktop.then(|| state.desktop_notices.subscribe());
    // Settings › Devices reads one `mobile` frame, sent now and on each change.
    let mut mobile_frames = state.mobile.subscribe_frame();
    // A linked node's screen and another machine's view are both a screen
    // another machine shows of this one (D-53, D-70).
    let own_screen = relay.is_none() && !remote;
    // Where this screen's choices of the operator's look are made: a view's
    // count only while its machine shows it (`remote_screen`, the core's
    // `remote_views`), and a tool's are no choice.
    let look_screen = client.look_screen(seat.as_ref().map(|seat| seat.viewer.key.to_hex()));
    if renderer {
        let frame = mobile_frame(&mobile_frames.borrow_and_update(), own_screen);
        if socket
            .send(Message::Text(frame.to_string().into()))
            .await
            .is_err()
        {
            client_gone(&state, connection, renderer, desktop);
            return;
        }
    }
    // Settings > Devices reads the pairings under way from one `pairing`
    // frame. It carries a link that lets a machine in and the words two
    // people compare, so only this machine's own windows get it: never a
    // phone, and never a screen another machine shows of this one.
    let mut pairing_frames = state.pairing.subscribe_frame();
    if own {
        let frame =
            json!({"type": "pairing", "payload": pairing_frames.borrow_and_update().as_ref()});
        if socket
            .send(Message::Text(frame.to_string().into()))
            .await
            .is_err()
        {
            client_gone(&state, connection, renderer, desktop);
            return;
        }
    }
    let Ok(first) = send_snapshot(&mut socket, &state, &mut have_revision, granular).await else {
        client_gone(&state, connection, renderer, desktop);
        return;
    };
    // Terminal output reaches the client beside its snapshots, from the hub
    // (PRD core-host-node-terminal D-11). A client that kept its terminals
    // across a reconnect resumes every pane from its cursor, or is drawn
    // again whole when it names none; one that got a whole snapshot draws
    // each pane from the full frame its view asks for.
    let terminals = relay.is_none().then(|| {
        let resume = match (
            first,
            handshake.have_terminal_sequence,
            handshake.have_terminal_epoch.clone(),
        ) {
            (FrameKind::Snapshot, _, _) => Resume::Fresh,
            // A view's frame may stop inside a chunk (D-84), so it is drawn
            // again whole rather than resumed from a cursor.
            (FrameKind::Delta, _, _) if remote => Resume::Redraw,
            (FrameKind::Delta, Some(cursor), Some(epoch)) => Resume::After { epoch, cursor },
            // A cursor without the hub it came from is not trusted.
            (FrameKind::Delta, _, _) => Resume::Redraw,
        };
        let limits = match remote {
            true => crate::terminal_hub::Limits::REMOTE,
            false => crate::terminal_hub::Limits::LOCAL,
        };
        state.core.hub.connect_with(resume, limits)
    });
    // A view's file reads, each in its own task writing to the view's bulk
    // stream; every read ends with the connection.
    let mut view_reads: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let link = relay.as_ref().map(|relay| relay.link.clone());
    let mut role_ended = state.role_ended.clone();
    let mut ended_with_role = false;
    loop {
        tokio::select! {
            () = async { let _ = role_ended.wait_for(|ended| *ended).await; } => {
                ended_with_role = true;
                break;
            }
            // A newer view of the same guest took this one's place.
            () = async {
                match &seat {
                    Some(seat) => seat.taken().await,
                    None => std::future::pending().await,
                }
            } => break,
            changed = notify.recv() => {
                match changed {
                    Ok(()) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
                match send_snapshot(&mut socket, &state, &mut have_revision, granular).await {
                    // A whole snapshot resets the client's terminals too.
                    Ok(FrameKind::Snapshot) => {
                        if let Some(terminals) = &terminals {
                            terminals.restart();
                        }
                    }
                    Ok(FrameKind::Delta) => {}
                    Err(()) => break,
                }
            }
            () = async {
                match &link {
                    Some(link) => {
                        link.closed().await;
                    }
                    None => std::future::pending().await,
                }
            } => break,
            () = async {
                match &terminals {
                    Some(terminals) => terminals.ready().await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(terminals) = &terminals else { continue };
                let (frame, redraws) = terminals.take();
                for pane in redraws {
                    state.core.terminals.redraw(&pane);
                }
                if let Some(frame) = frame
                    && socket.send(Message::Text(frame.text.into())).await.is_err()
                {
                    break;
                }
                // Only a window writes a program's copy to its clipboard.
                let copies = terminals.take_clipboard();
                let mut gone = false;
                for copied in copies.iter().filter(|_| renderer) {
                    if socket.send(Message::Text(copied.as_ref().into())).await.is_err() {
                        gone = true;
                        break;
                    }
                }
                if gone {
                    break;
                }
            }
            changed = directory_changes.recv() => {
                match changed {
                    Ok(frame) => {
                        let value = serde_json::from_str::<Value>(&frame).ok();
                        let field = |name: &str| {
                            value
                                .as_ref()
                                .and_then(|value| value.pointer(&format!("/payload/{name}")))
                                .and_then(Value::as_str)
                        };
                        // A folder or file on this machine is re-checked against
                        // its checkout roots; a device's names a path there,
                        // which only that device's catalog roots can vouch for.
                        let admitted = match (field("path"), field("device_id")) {
                            (Some(path), Some(device)) if !state.boundary.names_this_node(Some(device)) => {
                                state.boundary.is_under_device_root(device, path)
                            }
                            (Some(path), _) => state.boundary.resolve_target(path).is_ok(),
                            (None, _) => false,
                        };
                        if !admitted {
                            continue;
                        }
                        if socket.send(Message::Text(frame.into())).await.is_err() {
                            break;
                        }
                    }
                    // A client that fell behind on folder changes keeps its
                    // snapshot stream; the tree re-reads on the next change.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
                }
            }
            // `state` holds the sender, so the channel never closes under
            // this loop; a window that lagged behind calls skips to the
            // newest, which is the project the operator asked for last.
            call = window_calls.recv(), if own => {
                let Ok(payload) = call else { continue };
                let frame = json!({"type": "window_call", "payload": payload}).to_string();
                if socket.send(Message::Text(frame.into())).await.is_err() {
                    break;
                }
            }
            notice = async {
                match agent_notices.as_mut() {
                    Some(notices) => notices.recv().await,
                    None => std::future::pending().await,
                }
            }, if desktop => {
                match notice {
                    Ok(frame) => {
                        if socket.send(Message::Text(frame.as_ref().into())).await.is_err() {
                            break;
                        }
                    }
                    // A window that fell behind misses those notifications;
                    // the next change is announced as usual.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        hide_core::diagnostic!(json!({
                            "component": "desktop_notices", "kind": "agent_notice.lagged",
                            "connection": connection, "missed": missed,
                        }));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
                }
            }
            changed = mobile_frames.changed(), if renderer => {
                if changed.is_err() {
                    break;
                }
                let frame = mobile_frame(&mobile_frames.borrow_and_update(), own_screen);
                if socket.send(Message::Text(frame.to_string().into())).await.is_err() {
                    break;
                }
            }
            changed = pairing_frames.changed(), if own => {
                if changed.is_err() {
                    break;
                }
                let frame = json!({"type": "pairing", "payload": pairing_frames.borrow_and_update().as_ref()});
                if socket.send(Message::Text(frame.to_string().into())).await.is_err() {
                    break;
                }
            }
            Some(frame) = device_bytes.recv() => {
                if socket.send(frame).await.is_err() {
                    break;
                }
            }
            Some(frame) = device_frames.recv() => {
                if socket.send(Message::Text(frame.into())).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let routed = relay
                            .as_ref()
                            .and_then(|relay| Some((relay, screen_event::read(&text, RELAY_TAKES)?)));
                        if let Some((relay, routed)) = routed {
                            match routed.kind {
                                Kind::TerminalInput => {
                                    screen_input(&state, connection, &payload_str(&routed.event, "pane_id"));
                                }
                                _ => {
                                    let refused = relay_attachment(&state, connection, &relay.node, &routed.event, &text);
                                    if let Some(refused) = refused
                                        && socket.send(Message::Text(refused.to_string().into())).await.is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                            continue;
                        }
                        // A view's keys bring their pane's output first (D-84).
                        if remote
                            && let Some(terminals) = &terminals
                            && let Some(routed) = screen_event::read(&text, &[Kind::Key])
                            && let Ok((KeyTarget::Pane(pane), _)) = terminal_key(&routed.event)
                        {
                            terminals.keyed(&pane);
                        }
                        match handle_client_text(&state, &text, connection, client, own_screen, &look_screen) {
                            // A view's file bytes go on its bulk stream, from
                            // their own task (D-67).
                            Ok(ClientAction::FileBytes(event)) if remote => {
                                view_reads.retain(|read| !read.is_finished());
                                let bulk = seat.as_ref().and_then(crate::remote_screen::Seat::bulk);
                                let mut answer = None;
                                match (bulk, event_device(&state.boundary, &event)) {
                                    (None, _) => answer = Some(file_bytes_error(&payload_str(&event, "request_id"), &payload_str(&event, "path"), "bulk_missing")),
                                    (Some(bulk), Some(device)) => {
                                        answer = start_device_read(&state, &mut device_reads, bulk, (device, None), event);
                                    }
                                    (Some(_), None) if view_reads.len() >= crate::remote_screen::VIEW_READS => {
                                        answer = Some(file_bytes_error(&payload_str(&event, "request_id"), &payload_str(&event, "path"), "busy"));
                                    }
                                    (Some(bulk), None) => view_reads.push(start_view_read(&state, bulk, event)),
                                }
                                if let Some(frame) = answer
                                    && socket.send(frame).await.is_err()
                                {
                                    break;
                                }
                            }
                            Ok(ClientAction::FileBytes(event)) => {
                                if let Some(device) = event_device(&state.boundary, &event) {
                                    let slot = relay.as_ref().map(RelayScreen::slot);
                                    if matches!(slot, Some(None)) {
                                        let refused = file_bytes_error(&payload_str(&event, "request_id"), &payload_str(&event, "path"), "relay_busy");
                                        if socket.send(refused).await.is_err() {
                                            break;
                                        }
                                        continue;
                                    }
                                    let superseded = start_device_read(&state, &mut device_reads, device_bytes_tx.clone(), (device, slot.flatten()), event);
                                    // Sent here rather than through `device_bytes`,
                                    // which this loop drains and could be full.
                                    if let Some(frame) = superseded
                                        && socket.send(frame).await.is_err()
                                    {
                                        break;
                                    }
                                } else if send_file_bytes(&mut socket, &state.boundary, &state.roots, &event).await.is_err() {
                                    break;
                                }
                            }
                            Ok(ClientAction::Pairing(event)) => {
                                // Answered off this loop, which keeps the
                                // terminals moving while a share waits for
                                // the relay; the answer is this client's.
                                let pairing = Arc::clone(&state.pairing);
                                let answers = device_frames_tx.clone();
                                tokio::spawn(async move {
                                    let answer = pairing.window_event(&event, own).await;
                                    let _ = answers.send(answer.to_string()).await;
                                });
                            }
                            Ok(ClientAction::BrowserComment(event)) => {
                                // A linked node's screen shows pages its own
                                // machine's window holds, which this daemon's
                                // gateway does not reach (docs/INSTALL.md).
                                let refused = if relay.is_some() {
                                    Some("comment_unavailable_here".to_owned())
                                } else {
                                    let context = crate::browser_page::comment::Context {
                                        control: Arc::clone(&state.browser_control),
                                    };
                                    match state.browser_comments.handle(context, connection, &event, device_frames_tx.clone()) {
                                        Ok(Some(added)) => {
                                            let job = FeedbackJob { event: added.event, png: added.png };
                                            if let Some(refusal) = queue_feedback(&state, &mut feedback, job, &device_frames_tx)
                                                && socket.send(Message::Text(refusal.into())).await.is_err()
                                            {
                                                break;
                                            }
                                            None
                                        }
                                        Ok(None) => None,
                                        Err(reason) => Some(reason),
                                    }
                                };
                                if let Some(reason) = refused {
                                    let frame = json!({"type": "browser_comment", "payload": {
                                        "workspace": event.pointer("/payload/workspace"),
                                        "display_id": event.pointer("/payload/display_id"), "on": false, "reason": reason,
                                    }});
                                    if socket.send(Message::Text(frame.to_string().into())).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Ok(ClientAction::DeviceListing(event)) => {
                                let slot = relay.as_ref().map(RelayScreen::slot);
                                if matches!(slot, Some(None)) {
                                    if socket.send(relay_busy()).await.is_err() {
                                        break;
                                    }
                                    continue;
                                }
                                spawn_device_listing(&state, event, device_frames_tx.clone(), slot.flatten());
                            }
                            Ok(ClientAction::NodeAsk(event)) => {
                                let slot = relay.as_ref().map(RelayScreen::slot);
                                if matches!(slot, Some(None)) {
                                    if socket.send(relay_busy()).await.is_err() {
                                        break;
                                    }
                                    continue;
                                }
                                spawn_node_ask(&state, event, device_frames_tx.clone(), slot.flatten());
                            }
                            Ok(ClientAction::ScreenAsk(event)) => {
                                screen_asks.ask(&state, device_frames_tx.clone(), |asks| asks.add(event), screen_ask);
                            }
                            Ok(ClientAction::FileIndexAll(event)) => {
                                file_search.ask(&state, device_frames_tx.clone(), |waiting| { *waiting = Some(event); Ok(()) }, file_index_all);
                            }
                            Ok(ClientAction::Automation(event)) => {
                                crate::automation_screen::spawn(&state.core, event, look_screen.clone(), device_frames_tx.clone());
                            }
                            Ok(ClientAction::Access(event)) => {
                                crate::access_screen::spawn(&state.core, event, device_frames_tx.clone());
                            }
                            Ok(ClientAction::Feedback(event)) => {
                                let job = FeedbackJob { event, png: None };
                                if let Some(refusal) = queue_feedback(&state, &mut feedback, job, &device_frames_tx)
                                    && socket.send(Message::Text(refusal.into())).await.is_err()
                                {
                                    break;
                                }
                            }
                            outcome => {
                                let replies = match outcome {
                                    Ok(ClientAction::Replies(frames)) => frames,
                                    Err(error) => vec![Message::Text(
                                        json!({"type":"error","payload":{},"message": error}).to_string().into(),
                                    )],
                                    _ => unreachable!(),
                                };
                                let mut failed = false;
                                for frame in replies {
                                    if socket.send(frame).await.is_err() { failed = true; break; }
                                }
                                if failed { break; }
                            }
                        }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        if let Some((request_id, reason)) = state.attachments.receive(connection, &bytes)
                            && socket
                                .send(Message::Text(
                                    attachment_refused(&request_id, reason).to_string().into(),
                                ))
                                .await
                                .is_err()
                        {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
    for read in device_reads {
        read.task.abort();
    }
    for read in view_reads {
        read.abort();
    }
    if ended_with_role {
        let _ = socket.send(Message::Close(Some(role_ended_frame()))).await;
    }
    client_gone(&state, connection, renderer, desktop);
}

/// `invalid_agent_name`'s next action, naming the engine's limit
/// (`hide_core::AGENT_NAME_LIMIT`).
static INVALID_AGENT_NAME: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "The --name was refused: an agent name is 1 to {} characters of lowercase letters, digits, - and _, starting with a lowercase letter; spawn again with a name that fits",
        hide_core::AGENT_NAME_LIMIT
    )
});

/// What a refused delivery or agent command asks of its caller.
pub(crate) fn delivery_next_action(code: &str) -> &'static str {
    match hide_core::coordination::split_refusal(code).0 {
        // A spawn that hands a plan step, or a letter that would wake a
        // plan's sleeping child, is refused as the plan words it (PRD
        // hide-plan-delegate B5, B6).
        code if code.starts_with("plan_") => hide_core::plans::next_action(code),
        // A spawn under a mission node that may not start yet (PRD
        // node-presets D-06) is refused as the mission words it for a plan too.
        code @ ("mission_node_held" | "mission_gate_before_start") => {
            hide_core::missions::next_action(code)
        }
        "agent_pane_required" => {
            "Run the command inside a pane where the agent is running; an agent on Codex's shared daemon needs a session started with --no-daemon"
        }
        "machine_unknown" => {
            "Use the device id from hide workspace info or hide agent list with --machine, then retry"
        }
        // `hide agent ask-human` (PRD ask-human B2, B3).
        "human_kind_unknown" => {
            "hide agent ask-human takes auth (a sign-in, a re-authentication, an unlock) or permission (an app connection, an OS permission); for a decision or anything else, send your parent a letter with hide request send"
        }
        "human_place_required" => {
            "Name where the person acts: the page's address (github.com/login), the app (1Password) or the setting (System Settings > Privacy & Security > Camera)"
        }
        "human_place_too_long" => {
            "Name the place in at most 300 characters: the page's address, the app or the setting"
        }
        "machine_unavailable" => {
            "Check that the device is connected in Settings > Devices and its helper is allowed, then run the same command again"
        }
        "repository_unavailable" => {
            "Pass --repo as the path of a Git repository on that device, with --branch, then run the command again; a plain folder, not a Git repository, takes no --branch once it is added as a project on that device"
        }
        // A spawn with no branch works in a plain folder itself
        // (with-hide/hide#2479).
        "spawn_repo_required" => {
            "This is a Git project, or a folder no project of that machine holds: for a Git repository pass --repo <its path> and --branch <a branch>, and the agent works in its own worktree on that branch; a plain folder, not a Git repository, is added as a project first, and then an agent starts in it without --branch"
        }
        "spawn_project_loading" => {
            "Hide is still reading that project's folder; run the same command again in a few seconds"
        }
        "spawn_folder_unreadable" => {
            "Hide could not read that project's folder: it is gone, or the Git repository it points at cannot be read. Check the folder; for a Git repository pass --repo and --branch, and for a plain folder that moved, add it as a project again"
        }
        "agent_not_installed" => {
            "Install the agent CLI on the device so it is on the device's PATH, then run the command again"
        }
        "spawn_busy" => "The first request is still running; wait, then run the same command again",
        "workspace_closing" => {
            "The folder's last terminal is still closing; wait a moment, then run the same command again with the same intent"
        }
        "holder_cap" => {
            "That machine already runs the most panes Hide keeps open there (the detail names the machine and the count); end finished agents on it with hide agent end <id>, which closes their panes, then run the same command again with the same intent"
        }
        "pane_close_failed" => {
            "The registration ended, but the engine did not close its pane; run the same command again to close it"
        }
        "composer_not_empty" => {
            "Someone is typing in the recipient's input, so nothing was typed or recorded; send it again without --force, and it is delivered once they submit"
        }
        "composer_unconfirmed" => {
            "Hide could not read the recipient's input as empty (a menu or dialog may be open), so nothing was typed or recorded; send it again without --force"
        }
        "force_target_asleep" => {
            "The recipient is asleep; send it again without --force, and a request or block wakes it"
        }
        "machine_not_permitted" => hide_core::access::MACHINE_NOT_PERMITTED_NEXT,
        "recipient_not_agent" => {
            "A letter to another machine goes to an agent, not a pane; find the agent with hide agent list --machine <device> and send to core:<device>:agent:<id>"
        }
        "sender_not_registered" => {
            "Another machine answers an agent Hide knows, and Hide could read no agent session from this pane yet; run the command again from the agent's own turn once hide agent show here names it"
        }
        // A letter neither sent by the caller nor to it (nor one its mission
        // node lets it read), or one gone from the ledger: the same refusal
        // as for an id that never was, so it says no more than that.
        "letter_unavailable" => {
            "No letter with that id was sent by you or to you, or it has left the ledger; ask the agent that named it to send you its body in a letter of its own"
        }
        "intent_conflict" => {
            "Use a new --intent for each new spawn; spawn again with this intent only to repeat the same spawn"
        }
        "desk_spawn_args" => {
            "Desk passes no words to an agent's own CLI after `--`; choose the model and effort with --model and --effort and run the spawn again without `--`, or ask the operator to start that session themselves"
        }
        "invalid_launch_choice" => {
            "Pass a --model that is a model name, and an --effort the agent's CLI takes (Claude Code: low, medium, high, xhigh, max; Codex: minimal, low, medium, high, xhigh), then run the command again"
        }
        "worktree_unconfirmed" => {
            "The engine had not confirmed the new worktree in time and may still finish it; run the same command again with the same intent"
        }
        "agent_not_started" => {
            "The agent's start ended in its pane without the agent and was not typed again; check that pane for why, then spawn with a new intent"
        }
        "shell_starting" => {
            "The new pane's shell had not finished starting while the spawn could wait, so nothing was typed and Hide closed the tab it opened; find what the account's shell startup files wait on (start a login shell in a terminal of your own), then run the same command again with the same intent"
        }
        "pane_busy" => {
            "The new pane's shell kept another program running for the start's 30 s wait, so nothing was typed; let that program finish or end it in that pane, then run the same command again with the same intent"
        }
        "native_identity_unavailable" => {
            "The agent started and has not reported its session yet (Codex and OpenCode 2 report theirs at their first turn); Hide registers it with its parent and watch when it does, and the same command with the same intent then answers it. When error.spawn.cause is question, its screen asks a question of its own, such as whether to trust its folder: answer it in its pane, or ask a person to with hide agent ask-human permission <pane>, then run the same command again"
        }
        "identity_waits_full" => {
            "Too many agents Hide started are waiting to report their session; give one of them a first prompt or close its pane, then run the same command again"
        }
        "session_too_long" => {
            "The agent names its session by a file path longer than 4096 bytes; spawn it with a new intent in a checkout with a shorter path"
        }
        "target_ambiguous" => {
            "More than one agent answers to that name, on different machines; name it by its registration ID from hide agent list (agent-N), or by the pane's full address remote:<device>:pane:<id>"
        }
        // `hide agent show` and `hide agent end` named no registration and
        // no spawn: the same key answers the same until it names one.
        "agent_unavailable" => {
            "Hide knows no agent by that: take its id from hide agent list (agent-N); hide agent show also takes a spawn's id (spawn-N), the agent's name or its pane"
        }
        "target_unavailable" => {
            "No agent answers to that; a bare pane ID names a pane on your own machine. Find the recipient in hide agent list and send to its name or registration ID"
        }
        "no_parent" => {
            "You have no parent: no agent answers for you, so your work is the operator's to read; send to a named agent from hide agent list instead"
        }
        // The spawn that started the caller has not bound it yet and is
        // still within its answer time (`coordination::registering`).
        "participant_registering" => {
            "Hide is still registering you from the spawn that started you, which takes a few seconds; run the same command again shortly with the same intent"
        }
        "mission_cap_reached" => {
            "The mission's workers are at its cap; wait for one to finish and end it with hide agent end <id>, then run the same command again with the same intent"
        }
        "mission_brief_missing" => {
            "A mission node's worker starts on its brief: spawn again with --brief-file <path> (or --brief <text>) holding what the node is to do"
        }
        "invalid_brief" => "The brief must be text of at most 16 KiB that is not blank",
        "invalid_agent_name" => INVALID_AGENT_NAME.as_str(),
        "brief_not_submitted" => {
            "The agent started but did not take its brief within 30 seconds, and nothing was typed; error.spawn.cause says why: question (its screen asks a question; answer it in its pane), not_ready, unread, or agent_ended. Then run the same command again with the same intent, which submits the brief, starting the agent once more only if it ended"
        }
        "mission_not_started" => {
            "The operator has not approved the mission's plan; show it to them and run hide mission start once they approve"
        }
        "mission_machine_node_refused" => {
            "On another machine than the project's, a mission runs only a pull-request change node's worker and its checkers; leave out --machine, or name the project's machine, for this node's worker"
        }
        "mission_other_repository" => {
            "A mission node's worker works in the mission's project only, and --repo names another repository (the detail names both); spawn again with --repo naming the mission's project folder or a worktree of it"
        }
        "mission_machine_unavailable" => {
            "That machine is not connected; reconnect it in Settings > Devices and run the same command again, or leave out --machine to start the worker on the project's machine"
        }
        "mission_projects_loading" => {
            "Hide is still reading that machine's projects; run the same command again in a few seconds"
        }
        "mission_machine_no_repository" => {
            "Hide knows no project of the mission's GitHub repository on that machine: register a checkout of it there as a project, or wait while GitHub is still being read for that project or for the mission's own (its pull requests show once it is), then run the same command again with --repo naming it; or leave out --machine"
        }
        // A spawn whose agent would start without Hide's hook on its
        // machine (`runtime::start_hook`): nothing was made or typed, and the
        // same command continues once the operator has done this.
        "agent_hook_off" => {
            "The agent is switched off for that machine; ask the operator to switch it on in Settings > Agents for that machine, then run the same command again with the same intent"
        }
        "agent_not_set_up" => {
            "The agent has not made its own folder on that machine yet, so Hide cannot put its hook in; ask the operator to start the agent there once and sign in, then run the same command again with the same intent"
        }
        "agent_hook_untrusted" => {
            "Codex on that machine has not trusted Hide's hooks; ask the operator to start codex there once and choose Trust all and continue on its Hooks need review screen, then run the same command again with the same intent"
        }
        "agent_hook_missing" => {
            "Hide's hook for the agent is not in place on that machine; ask the operator to press Reinstall for that machine in Settings > Devices (This Mac for this machine), then run the same command again with the same intent"
        }
        "agent_hook_unconfirmed" => {
            "Hide's check of the agent's hook on that machine did not finish in time; run the same command again with the same intent"
        }
        "agent_hook_unchecked" => {
            "Hide could not check the agent's hook on that machine; ask the operator to open Settings > Devices for that machine (This Mac for this machine), which says what stopped the check and offers Try again or Check again, then run the same command again with the same intent"
        }
        "codex_capability_unknown" => {
            "Hide has not read whether that machine's Codex runs a shared server; ask the operator to press Check again in Settings > Agents for that machine, then run the same command again with the same intent"
        }
        "beside_other_checkout" => {
            "--beside opens an agent only in the checkout you work in; pass that checkout's --repo and --branch, or run the command again without --beside to open it in a new tab"
        }
        "beside_other_machine" => {
            "--beside opens an agent only on your own machine; leave out --machine, or run the command again without --beside to open it in a new tab"
        }
        "beside_too_narrow" => {
            "Your pane is too narrow to split: the agent's input box would not be readable beside you, so nothing was opened; run the command again without --beside to open the agent in a new tab"
        }
        "beside_unmeasured" => {
            "The panes of your tab are not drawn yet, so nothing was opened; run the command again without --beside to open the agent in a new tab"
        }
        "beside_tab_busy" => {
            "Your tab is still busy with the splits and moves ahead of this one, so nothing was opened; run the command again without --beside to open the agent in a new tab"
        }
        "beside_needs_agent_pane" => {
            "--beside opens the agent beside the pane that calls it; run the command from an agent pane Hide knows, or without --beside to open it in a new tab"
        }
        "beside_with_parent" => {
            "An agent opened with --beside is the operator's to watch and has no parent; leave out --parent to open it beside you, or leave out --beside to supervise it in a new tab"
        }
        // A spawn's agent profile (docs/ARCHITECTURE.md, Starting an agent
        // with a profile): only an unanswered machine is worth the same
        // intent again as it is.
        "profile_gone" => {
            "That machine no longer has the profile; spawn with another --profile, or without --profile for the agent's default there"
        }
        "profiles_unsupported" => {
            "That machine's Hide predates agent profiles; update Hide there, or spawn without --profile"
        }
        "profile_refused" => {
            "The profile cannot start its program on that machine (the detail says why); fix that program or the profile, or spawn with another --profile, since the same intent fails the same way until then"
        }
        "profile_unanswered" => {
            "That machine did not answer for the profile; check that it is connected, then run the same command again with the same intent"
        }
        "profile_unconfirmed" => {
            "The agent's start was typed but Hide could not confirm its program runs; look at the agent's pane before starting it again, since it may be running"
        }
        "launch_records_full" => {
            "Hide keeps as many records of how its agents started as it can, so an agent with a profile does not start; close agents you no longer need, then run the same command again"
        }
        "beside_pane_unavailable" => {
            "The engine could not split your pane (it closed, or the engine refused the split), so nothing was opened; run the same command again from your agent pane, or with a new intent without --beside to open it in a new tab"
        }
        _ => "Check the current agent pane and retry the same intent",
    }
}

/// Whether a page command of the CLI may act on `view` (a phone's picture
/// is a read and does not ask, `mobile/page_shot.rs`). A View on a desk does only as
/// its area's selected View, so one behind another tab is refused before
/// anything is sent; a page on no desk is on no screen for an action to be
/// drawn over, so every one of them does (PRD lane-desk-pair-view B32).
pub(crate) fn takes_input(view: &hide_core::workspace_control::View) -> bool {
    view.selected || view.area_id == hide_core::workspace_views::SHELF_AREA
}

/// What the core knows of a display a `hide browser` command relays to:
/// whether it takes input (`takes_input`), and `source_url`, the address the
/// core keeps for it. A device's page loads from a loopback route on this
/// machine, so the page reports that address and the CLI names this one
/// instead. `None` when the caller's checkout, whose Views alone `views`
/// lists, has no such display.
pub(crate) fn relay_facts(
    views: &[hide_core::workspace_control::View],
    display_id: &str,
) -> Option<Value> {
    let view = views.iter().find(|view| view.view_id == display_id)?;
    Some(json!({"selected": takes_input(view), "source_url": view.target}))
}

/// The relay's answer to the CLI: `relay_facts` under the display's id.
pub(crate) fn relay_ready(display_id: &str, facts: &Value) -> Value {
    json!({"display_id": display_id, "selected": facts["selected"], "source_url": facts["source_url"]})
}

/// The reason and next action of a refused command. A refusal that names the
/// devices the caller could have used says so in its next action, and its
/// reason stays the code.
fn refusal_answer(reason: String, next_action: &'static str) -> (String, String) {
    // An external tool's refusal says what the tool does next, whichever
    // owner refused it (PRD hide-external-control B20-B23).
    if let Some(next_action) = crate::access::next_action(&reason) {
        return (reason, next_action.to_owned());
    }
    match hide_core::coordination::split_refusal(&reason) {
        (code @ "shell_starting", detail) => {
            let (seconds, screen) =
                hide_core::coordination::shell_starting_detail(detail.unwrap_or_default());
            let waited = seconds.map_or_else(String::new, |seconds| {
                format!(". The shell was waited for {seconds} s")
            });
            let screen = screen.map_or_else(String::new, |screen| {
                format!(". Its screen ends with: {screen}")
            });
            (code.to_owned(), format!("{next_action}{waited}{screen}"))
        }
        (code @ "mission_cap_reached", Some(cap)) => (
            code.to_owned(),
            format!("{next_action}; the mission runs at most {cap} workers at once"),
        ),
        (code @ "mission_conducted", Some(name)) => (
            code.to_owned(),
            format!("{next_action}; {name} conducts it now"),
        ),
        // A spawn or a send that stopped partway names what it left as a
        // field of the answer (`refusal_field`), not in the sentence.
        (code, Some(_)) if refusal_field(&reason).is_some() => {
            (code.to_owned(), next_action.to_owned())
        }
        // Only an unknown machine's detail lists the devices to name.
        (code @ "machine_unknown", Some("")) => (
            code.to_owned(),
            "No other device is connected; connect one in Settings > Devices, then retry"
                .to_owned(),
        ),
        (code @ "machine_unknown", Some(connected)) => (
            code.to_owned(),
            format!("Use one of the connected device ids with --machine: {connected}"),
        ),
        // Every other detail names the step, node, field or evidence to fix:
        // a plan's (PRD hide-plan B1, B2; hide-plan-delegate B5, B10-B13) and
        // a mission's (PRD mission-map-git-criteria B1, B9, B10).
        (code, Some(detail)) => (code.to_owned(), format!("{next_action} ({detail})")),
        (_, None) => (reason, next_action.to_owned()),
    }
}

/// What a refusal of work that stopped partway left, as a field of the
/// answer: a spawn's `spawn` (`{"id","pane"}`) and a pane send's `sent`
/// (`{"text_sent","keys_sent","enter_sent"}`).
fn refusal_field(reason: &str) -> Option<(&'static str, Value)> {
    let (code, detail) = hide_core::coordination::split_refusal(reason);
    let name = if hide_core::coordination::INCOMPLETE_SPAWN.contains(&code) {
        "spawn"
    } else if hide_core::pane_command::PARTIAL.contains(&code) {
        "sent"
    } else {
        return None;
    };
    Some((name, serde_json::from_str(detail?).ok()?))
}

fn valid_caller_hint(value: &serde_json::Value) -> bool {
    value.get("caller_pane").is_none_or(|hint| {
        hint.is_null()
            || hint.as_str().is_some_and(|hint| {
                !hint.is_empty() && hint.len() <= 256 && !hint.chars().any(char::is_control)
            })
    })
}

enum ScopedRequest {
    Query(hide_core::workspace_control::Query),
    Action(hide_core::workspace_control::Action),
    BrowserConnect(Option<String>),
    /// `hide browser` page commands: the same checks as a display
    /// connection, then this socket carries the CDP frames.
    BrowserRelay(String),
    Delivery(Box<hide_core::delivery::Command>, Option<String>),
    /// `hide links`: a read of the link record, which needs no shell.
    Links(hide_core::links::query::LinksQuery),
    /// `hide session list|record|read`: a read of the Project's sessions,
    /// which needs no shell.
    Session(hide_core::links::session_command::SessionCommand),
    /// `hide memory`: Project Memory's own store, which needs no shell.
    Memory(hide_core::memory_command::MemoryCommand),
    /// `hide workspace memory`: a hook's Project Memory, which needs no
    /// shell.
    MemoryHook(hide_core::memory_hook::Ask),
    /// `hide mission`: a Conductor's mission record, the core's own.
    Mission(Box<hide_core::missions::MissionCommand>),
    /// `hide task`: the caller's session's own plan, the core's record.
    Plan(hide_core::plans::PlanCommand),
    /// `hide automation`: the operator's automations, the core's book
    /// (`automation_cli`).
    Automation(Box<hide_core::automations::Call>),
    /// `hide session fold-ask`: one of the caller's session's open questions,
    /// folded with its reason in the link record.
    FoldAsk {
        id: String,
        reason: String,
    },
    /// `hide agent catalog`: what a machine's agents take, read from the
    /// core's own reports.
    AgentCatalog(Option<String>),
    /// `hide agent profile`: planned on the owner thread, run off it.
    AgentProfile(hide_core::agent_profiles::ProfileCommand),
    /// `hide skill` / `hide mcp`: planned on the owner thread, waited on off
    /// it, an install or an update until the operator answers its confirm.
    Store(hide_core::store_command::StoreCommand),
    /// `hide ps [--all]`: the live panes and agents, read from the core's
    /// own state.
    Ps {
        all: bool,
    },
    /// `hide agent sleep <id> [--actor <id>]`: the runtime's own end of a
    /// registered agent.
    AgentSleep {
        id: String,
        actor: Option<String>,
    },
    /// `hide agent archive <id> [--actor <id>]`: the screen's `세션 보관`
    /// for a registered agent, admitted by the core.
    AgentArchive {
        id: String,
        actor: Option<String>,
    },
    /// `hide device`: the core's own device registrations.
    Device(hide_core::device_command::DeviceCommand),
    /// `hide pane`, `hide agent read` and `hide workspace purpose`: run
    /// through the core's engine, which needs no shell.
    Pane(hide_core::pane_command::PaneCommand),
    /// `hide <path>`: a folder of the caller's machine opened as a project
    /// (`project_open`).
    ProjectOpen(String),
    /// `hide workspace remove`: the delete dialog's removal of a worktree,
    /// run by the core, which needs no shell.
    WorktreeRemove(hide_core::worktree_command::WorktreeRemove),
    /// `hide access list`: an external tool reads the tools connected and
    /// the requests waiting (PRD hide-external-control B17).
    AccessList,
}

impl ScopedRequest {
    /// A request a shell must be open to answer. Delivery and the link
    /// record are the daemon's own and answer without one (B43).
    fn needs_renderer(&self) -> bool {
        !matches!(
            self,
            Self::Delivery(..)
                | Self::Links(..)
                | Self::Session(..)
                | Self::Memory(..)
                | Self::MemoryHook(..)
                | Self::Mission(..)
                | Self::Plan(..)
                | Self::Automation(..)
                | Self::FoldAsk { .. }
                | Self::AgentCatalog(..)
                | Self::AgentProfile(..)
                | Self::Store(..)
                | Self::Ps { .. }
                | Self::AgentSleep { .. }
                | Self::AgentArchive { .. }
                | Self::Device(..)
                | Self::Pane(..)
                | Self::ProjectOpen(..)
                | Self::WorktreeRemove(..)
                | Self::AccessList
        )
    }

    /// What a bot's mode must allow for this command (PRD
    /// hide-external-control B20-B23, mission-87). A command family the core
    /// also runs for a bot takes its need from the core's one table
    /// (`hide_core::access`), which the core judges again with what the
    /// command names; the rest are this boundary's alone
    /// (`docs/ARCHITECTURE.md`, Bots, has every row).
    fn access_need(&self) -> hide_core::access::Need {
        use hide_core::access::{self, Need};
        match self {
            Self::Delivery(command, _) => access::delivery_need(command),
            Self::Pane(command) => access::pane_need(command),
            Self::Mission(command) => access::mission_need(command),
            Self::Device(command) => access::device_need(command),
            // The core lists an own-mode bot's own work only, and holds the
            // agent the rest name to it.
            Self::Ps { .. } | Self::AgentSleep { .. } | Self::AgentArchive { .. } => Need::Work,
            // Which agents and models a machine offers is no one's work,
            // and a spawn into any project on any device takes it.
            Self::AgentCatalog(..) => Need::Work,
            // The operator's screen and the project's records (Views, files,
            // pages, links, sessions, worktrees), the bots and requests, the
            // skills and MCP servers, the agents' profiles, the automations
            // book and a new project are no one bot's work.
            Self::Query(..)
            | Self::Action(..)
            | Self::BrowserConnect(..)
            | Self::BrowserRelay(..)
            | Self::Links(..)
            | Self::Session(..)
            | Self::WorktreeRemove(..)
            | Self::AccessList
            | Self::Store(..)
            | Self::AgentProfile(..)
            | Self::Automation(..)
            | Self::ProjectOpen(..) => Need::All,
            Self::Plan(..) | Self::FoldAsk { .. } | Self::Memory(..) | Self::MemoryHook(..) => {
                Need::Agent
            }
        }
    }
}

/// Judges `command` by the mode its caller runs in: a bot's own, and the
/// whole Hide for Desk (`Capability::reach`); a pane's or checkout's has none
/// and is judged by the core where the command names its caller.
fn admit_command(
    cap: &crate::pane_auth::Capability,
    command: &ScopedRequest,
) -> Result<(), (String, &'static str)> {
    match cap.reach() {
        Some(mode) => hide_core::access::admit(mode, command.access_need()).map_err(|reason| {
            (
                reason.to_owned(),
                crate::pane_auth::refusal_next_action(reason),
            )
        }),
        None => Ok(()),
    }
}

/// A pane capability can submit only Workspace commands. It never receives a
/// snapshot, file bytes, or the shell's unrestricted dispatch channel.
async fn scoped_client_loop(
    mut socket: WebSocket,
    state: AppState,
    connection: u64,
    token: String,
    one_shot: bool,
    source: Option<(String, hide_node::ssh::RemoteHost)>,
    attested: Option<Arc<crate::pane_auth::StreamAttestation>>,
) {
    let _local = match LocalSeat::take(&state) {
        Ok(local) => local,
        Err(count) => {
            refuse(&mut socket, CloseReason::ClientLimit, Some(count)).await;
            return;
        }
    };
    state.clients.fetch_add(1, Ordering::SeqCst);
    let incoming = tokio::time::timeout(Duration::from_secs(10), socket.recv()).await;
    // Set only for an admitted `browser_relay`: its slot, its display and the
    // gateway socket it pumps once the answer and the claim are through.
    let mut relay = None;
    let mut relay_way = crate::browser_relay::Way::Core;
    let mut relay_for: Option<String> = None;
    let mut relay_slot = None;
    // Where an admitted relay runs, which its answer leaves here: the
    // answer itself is the same JSON every scoped request gives.
    let chosen: Arc<std::sync::Mutex<Option<crate::browser_control::Relay>>> = Arc::default();
    let response = match incoming {
        Ok(Some(Ok(Message::Text(text)))) if text.len() <= 128 * 1024 => {
            match serde_json::from_str::<Value>(&text) {
                Ok(value) => {
                    let request_id = value["request_id"].as_str().unwrap_or("");
                    let command = match value["type"].as_str() {
                        Some("workspace_query") => match value["query"].as_str() {
                            Some("info") => Some(ScopedRequest::Query(
                                hide_core::workspace_control::Query::Info,
                            )),
                            Some("view_list") => Some(ScopedRequest::Query(
                                hide_core::workspace_control::Query::ViewList,
                            )),
                            Some("browser_connect") => match value.get("display_id") {
                                None | Some(Value::Null) => {
                                    Some(ScopedRequest::BrowserConnect(None))
                                }
                                Some(Value::String(id)) if !id.is_empty() && id.len() <= 256 => {
                                    Some(ScopedRequest::BrowserConnect(Some(id.clone())))
                                }
                                _ => None,
                            },
                            _ => None,
                        },
                        Some("browser_relay") => match value.get("display_id") {
                            Some(Value::String(id)) if !id.is_empty() && id.len() <= 256 => {
                                Some(ScopedRequest::BrowserRelay(id.clone()))
                            }
                            _ => None,
                        },
                        Some("workspace_action") => {
                            serde_json::from_value::<hide_core::workspace_control::Action>(
                                value["command"].clone(),
                            )
                            .ok()
                            .map(ScopedRequest::Action)
                        }
                        Some("links") => serde_json::from_value::<
                            hide_core::links::query::LinksQuery,
                        >(value["query"].clone())
                        .ok()
                        .filter(|query| query.target.valid())
                        .map(ScopedRequest::Links),
                        Some("session") => serde_json::from_value::<
                            hide_core::links::session_command::SessionCommand,
                        >(value["command"].clone())
                        .ok()
                        .map(ScopedRequest::Session),
                        Some("delivery") => {
                            hide_core::delivery::Command::from_wire(value["command"].clone())
                                .ok()
                                .filter(|_| valid_caller_hint(&value))
                                .map(|command| {
                                    ScopedRequest::Delivery(
                                        Box::new(command),
                                        value["caller_pane"].as_str().map(str::to_owned),
                                    )
                                })
                        }
                        Some("memory") => serde_json::from_value::<
                            hide_core::memory_command::MemoryCommand,
                        >(value["command"].clone())
                        .ok()
                        .filter(hide_core::memory_command::MemoryCommand::valid)
                        .map(ScopedRequest::Memory),
                        Some("memory_hook") => hide_core::memory_hook::Ask::from_wire(&value)
                            .map(ScopedRequest::MemoryHook),
                        Some("mission") => serde_json::from_value::<
                            hide_core::missions::MissionCommand,
                        >(value["command"].clone())
                        .ok()
                        .filter(hide_core::missions::MissionCommand::valid)
                        .map(|command| ScopedRequest::Mission(Box::new(command))),
                        Some("plan") => serde_json::from_value::<hide_core::plans::PlanCommand>(
                            value["command"].clone(),
                        )
                        .ok()
                        .filter(hide_core::plans::PlanCommand::valid)
                        .map(ScopedRequest::Plan),
                        Some("automation") => crate::automation_cli::from_wire(&value["call"])
                            .map(|call| ScopedRequest::Automation(Box::new(call))),
                        Some("fold_ask") => {
                            match (value["id"].as_str(), value["reason"].as_str()) {
                                (Some(id), Some(reason))
                                    if id.len() <= 64 && reason.len() <= 1024 =>
                                {
                                    Some(ScopedRequest::FoldAsk {
                                        id: id.to_owned(),
                                        reason: reason.to_owned(),
                                    })
                                }
                                _ => None,
                            }
                        }
                        Some("agent_profile") => serde_json::from_value::<
                            hide_core::agent_profiles::ProfileCommand,
                        >(value["command"].clone())
                        .ok()
                        .filter(hide_core::agent_profiles::ProfileCommand::valid)
                        .map(ScopedRequest::AgentProfile),
                        Some("store") => serde_json::from_value::<
                            hide_core::store_command::StoreCommand,
                        >(value["command"].clone())
                        .ok()
                        .filter(hide_core::store_command::StoreCommand::valid)
                        .map(ScopedRequest::Store),
                        Some("agent_catalog") => match value.get("machine") {
                            None | Some(Value::Null) => Some(ScopedRequest::AgentCatalog(None)),
                            Some(Value::String(id)) if !id.is_empty() && id.len() <= 256 => {
                                Some(ScopedRequest::AgentCatalog(Some(id.clone())))
                            }
                            _ => None,
                        },
                        Some("ps") => match value.get("all") {
                            None | Some(Value::Bool(false)) => {
                                Some(ScopedRequest::Ps { all: false })
                            }
                            Some(Value::Bool(true)) => Some(ScopedRequest::Ps { all: true }),
                            _ => None,
                        },
                        Some(kind @ ("agent_sleep" | "agent_archive")) => {
                            let text = |value: &Value| -> Option<String> {
                                value
                                    .as_str()
                                    .filter(|text| {
                                        !text.is_empty()
                                            && text.len() <= 256
                                            && !text.chars().any(char::is_control)
                                    })
                                    .map(str::to_owned)
                            };
                            match (text(&value["id"]), value.get("actor")) {
                                (Some(id), None | Some(Value::Null)) => Some((id, None)),
                                (Some(id), Some(actor)) => {
                                    text(actor).map(|actor| (id, Some(actor)))
                                }
                                _ => None,
                            }
                            .map(|(id, actor)| {
                                if kind == "agent_sleep" {
                                    ScopedRequest::AgentSleep { id, actor }
                                } else {
                                    ScopedRequest::AgentArchive { id, actor }
                                }
                            })
                        }
                        Some("device") => serde_json::from_value::<
                            hide_core::device_command::DeviceCommand,
                        >(value["command"].clone())
                        .ok()
                        .filter(hide_core::device_command::DeviceCommand::valid)
                        .map(ScopedRequest::Device),
                        Some("pane") => serde_json::from_value::<
                            hide_core::pane_command::PaneCommand,
                        >(value["command"].clone())
                        .ok()
                        .map(ScopedRequest::Pane),
                        Some("worktree_remove") => {
                            serde_json::from_value::<hide_core::worktree_command::WorktreeRemove>(
                                value["command"].clone(),
                            )
                            .ok()
                            .filter(hide_core::worktree_command::WorktreeRemove::valid)
                            .map(ScopedRequest::WorktreeRemove)
                        }
                        Some("project_open") => value["path"]
                            .as_str()
                            .filter(|path| hide_core::project_open::valid_path(path))
                            .map(|path| ScopedRequest::ProjectOpen(path.to_owned())),
                        Some("access_list") => Some(ScopedRequest::AccessList),
                        _ => None,
                    };
                    // A plan carries its whole node list, and an automation
                    // its task, so a mission, plan or automation command may
                    // use the frame's full size like a letter.
                    if command.is_none()
                        || (value["type"] != "delivery"
                            && value["type"] != "mission"
                            && value["type"] != "plan"
                            && value["type"] != "automation"
                            && value["type"] != "pane"
                            && text.len() > 16 * 1024)
                        || request_id.is_empty()
                        || request_id.len() > 64
                        || !request_id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    {
                        json!({"type":"workspace_result","ok":false,"reason":"invalid_request","next_action":"Check hide command arguments and retry"})
                    } else {
                        let request_id = request_id.to_owned();
                        let Some(command) = command else {
                            unreachable!("validated above")
                        };
                        let relay_display = match &command {
                            ScopedRequest::BrowserRelay(id) => Some(id.clone()),
                            _ => None,
                        };
                        relay_for.clone_from(&relay_display);
                        relay_slot = relay_display
                            .as_ref()
                            .map(|_| state.browser_control.acquire_relay());
                        let core = Arc::clone(&state.core);
                        let registry = Arc::clone(&state.pane_capabilities);
                        let renderers = Arc::clone(&state.renderers);
                        let desktop_renderers = Arc::clone(&state.desktop_renderers);
                        let page_renderers = Arc::clone(&state.page_renderers);
                        let browser_control = Arc::clone(&state.browser_control);
                        let query_token = token.clone();
                        let command_request_id = request_id.clone();
                        let caller_node = source.as_ref().map(|(node, _)| node.clone());
                        let command_relay = Arc::clone(&chosen);
                        let roots = Arc::clone(&state.roots);
                        let window_calls = state.window_calls.clone();
                        let outcome = if let Some(Err((reason, next_action))) = &relay_slot {
                            Ok(Err(((*reason).to_owned(), *next_action)))
                        } else {
                            tokio::task::spawn_blocking(move || {
                            let cap = registry.validate(&query_token, Some(core.engine()), &core, attested.as_deref())
                                .map_err(|reason| (reason.to_owned(), crate::pane_auth::refusal_next_action(reason)))?;
                            // A bot runs what its mode allows, and Desk what
                            // the whole Hide allows, judged here for every
                            // command (B20-B23, mission-87 D3).
                            admit_command(&cap, &command)?;
                            // Desk's file open waits in the core for a window
                            // showing Desk, so it needs none now.
                            let desk = cap.pane_id == hide_core::workspace_control::DESK_CALLER;
                            if command.needs_renderer() && !desk && renderers.load(Ordering::SeqCst) == 0 {
                                return Err((
                                    "renderer_unavailable".to_owned(),
                                    "Open Hide's web or desktop shell and retry",
                                ));
                            }
                            let result = match command {
                                ScopedRequest::BrowserConnect(display_id) => {
                                    if desktop_renderers.load(Ordering::SeqCst) == 0 {
                                        return Err(("browser_unsupported".to_owned(), "Open the Hide desktop app and retry"));
                                    }
                                    let result = core.workspace_query(&cap.context.device_id, &cap.pane_id,
                                        hide_core::workspace_control::Query::ViewList)
                                        .map_err(|refusal| (refusal.reason.to_owned(), refusal.next_action))?;
                                    if result.context != cap.context {
                                        let (reason, next_action) = cap.changed_refusal();
                                        return Err((reason.to_owned(), next_action));
                                    }
                                    browser_control.connect(&result, display_id.as_deref(), caller_node.as_deref())
                                        .map_err(|(reason, next_action)| (reason.to_owned(), next_action))?
                                }
                                ScopedRequest::BrowserRelay(display_id) => {
                                    if desktop_renderers.load(Ordering::SeqCst) == 0 {
                                        return Err(("browser_unsupported".to_owned(), "Open the Hide desktop app and retry"));
                                    }
                                    let result = core.workspace_query(&cap.context.device_id, &cap.pane_id,
                                        hide_core::workspace_control::Query::ViewList)
                                        .map_err(|refusal| (refusal.reason.to_owned(), refusal.next_action))?;
                                    if result.context != cap.context {
                                        let (reason, next_action) = cap.changed_refusal();
                                        return Err((reason.to_owned(), next_action));
                                    }
                                    let relay = browser_control.relay(&result, &display_id, caller_node.as_deref())
                                        .map_err(|(reason, next_action)| (reason.to_owned(), next_action))?;
                                    *command_relay.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(relay);
                                    relay_facts(result.views.as_deref().unwrap_or(&[]), &display_id)
                                        .ok_or(("display_missing".to_owned(), "Run hide view list and retry"))?
                                }
                                ScopedRequest::Delivery(command, hint) => {
                                    crate::hook_request::delivery(&core, &cap, *command, hint)?
                                }
                                ScopedRequest::AccessList => {
                                    if cap.access_mode().is_none() {
                                        return Err((
                                            "access_not_connected".to_owned(),
                                            crate::pane_auth::refusal_next_action("access_not_connected"),
                                        ));
                                    }
                                    core.access(hide_core::access::Call::List).map_err(|code| {
                                        let next = crate::pane_auth::refusal_next_action(&code);
                                        (code, next)
                                    })?
                                }
                                ScopedRequest::MemoryHook(ask) => {
                                    crate::hook_request::memory(&core, &cap, &ask)?
                                }
                                ScopedRequest::Links(query) => {
                                    let scope = core.links_scope(&cap.context).ok_or((
                                        "links_unavailable".to_owned(),
                                        "Retry; the reason is in Hide's diagnostic log",
                                    ))?;
                                    let own = core.node_link(&scope.local_device).map_err(|_| (
                                        "links_unavailable".to_owned(),
                                        "Retry; the reason is in Hide's diagnostic log",
                                    ))?;
                                    hide_core::links::query::run(&scope, &query, own.as_ref())
                                        .map_err(|(reason, next_action)| (reason.to_owned(), next_action))?
                                }
                                ScopedRequest::Session(command) => {
                                    let scope = core.session_scope(&cap.context).ok_or((
                                        "links_unavailable".to_owned(),
                                        "Retry; the reason is in Hide's diagnostic log",
                                    ))?;
                                    hide_core::links::session_command::run(&scope, &command, core.as_ref())?
                                }
                                ScopedRequest::Memory(command) => {
                                    core.prepare_memory(&cap.context.device_id, &cap.pane_id, &cap.context, command)
                                        .and_then(hide_core::memory_command::PreparedMemory::run)
                                        .map_err(|code| {
                                            let next = hide_core::memory_command::next_action(&code);
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::Mission(command) => {
                                    // A take hands the former Conductor's
                                    // children to the caller's registration,
                                    // so the caller has one first (B43).
                                    if matches!(*command, hide_core::missions::MissionCommand::Take { .. }) {
                                        crate::hook_request::delivery(
                                            &core,
                                            &cap,
                                            hide_core::delivery::Command::Agents {
                                                command: hide_core::coordination::Command::RegisterSelf {
                                                    project: Some(cap.context.checkout_path.clone()),
                                                },
                                            },
                                            None,
                                        )?;
                                    }
                                    core.mission(&cap.context.device_id, &cap.pane_id, &cap.context, *command)
                                        .map_err(|code| {
                                            let next = hide_core::missions::next_action(
                                                hide_core::coordination::split_refusal(&code).0,
                                            );
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::Plan(command) => {
                                    core.plan(&cap.context.device_id, &cap.pane_id, &cap.context, command)
                                        .map_err(|code| {
                                            let next = hide_core::plans::next_action(
                                                hide_core::coordination::split_refusal(&code).0,
                                            );
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::Automation(call) => {
                                    let from_device = cap.context.device_id != core.node().as_str();
                                    crate::automation_cli::answer(&core, *call, from_device)?
                                }
                                ScopedRequest::FoldAsk { id, reason } => {
                                    core.fold_ask_scope(&cap.context.device_id, &cap.pane_id, &cap.context)
                                        .and_then(|scope| scope.run(&id, &reason))
                                        .map_err(|code| {
                                            let next = hide_core::links::agent_asks::next_action(&code);
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::AgentProfile(command) => {
                                    core.prepare_agent_profile(&cap.context.device_id, command)
                                        .and_then(hide_core::agent_profiles::Prepared::run)
                                        .map_err(|code| {
                                            let next = hide_core::agent_profiles::next_action(
                                                hide_core::coordination::split_refusal(&code).0,
                                            );
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::Store(command) => {
                                    core.prepare_store(&cap.context.device_id, &cap.pane_id, command)
                                        .and_then(hide_core::store_command::Prepared::run)
                                        .map_err(|code| {
                                            let next = hide_core::store_command::next_action(
                                                hide_core::coordination::split_refusal(&code).0,
                                            );
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::AgentCatalog(machine) => {
                                    core.agent_catalog(&cap.context.device_id, machine)
                                        .map_err(|code| {
                                            let next = crate::server::delivery_next_action(&code);
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::Ps { all } => {
                                    core.ps(&cap.context.device_id, &cap.pane_id, all)
                                        .map_err(|code| {
                                            let next = crate::server::delivery_next_action(&code);
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::AgentSleep { id, actor } => {
                                    core.agent_sleep(&cap.context.device_id, &cap.pane_id, &cap.context, id, actor)
                                        .map_err(|code| {
                                            let next = hide_core::coordination::sleep_next_action(&code);
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::AgentArchive { id, actor } => {
                                    core.agent_archive(&cap.context.device_id, &cap.pane_id, &cap.context, id, actor)
                                        .map_err(|code| {
                                            let next = hide_core::coordination::archive_next_action(&code);
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::ProjectOpen(path) => crate::project_open::run(
                                    &core,
                                                                        &roots,
                                    &window_calls,
                                    renderers.load(Ordering::SeqCst),
                                    &cap.context.device_id,
                                    &path,
                                    &command_request_id,
                                )?,
                                ScopedRequest::WorktreeRemove(command) => {
                                    core.worktree_remove(&cap.context.device_id, &cap.pane_id, &cap.context, command)
                                        .map_err(|reason| {
                                            let code = hide_core::coordination::split_refusal(&reason).0;
                                            let next = hide_core::worktree_command::next_action(code)
                                                .unwrap_or_else(|| crate::server::delivery_next_action(code));
                                            (reason, next)
                                        })?
                                }
                                ScopedRequest::Pane(command) => {
                                    let refusal = |reason: String| {
                                        let code = hide_core::coordination::split_refusal(&reason).0;
                                        let next = hide_core::pane_command::next_action(code)
                                            .unwrap_or_else(|| crate::server::delivery_next_action(code));
                                        (reason, next)
                                    };
                                    core.prepare_pane(&cap.context.device_id, &cap.pane_id, &cap.context, command)
                                        .and_then(hide_core::pane_command::Prepared::run)
                                        .map_err(refusal)?
                                }
                                ScopedRequest::Device(command) => {
                                    core.device_command(&cap.context.device_id, command)
                                        .map_err(|code| {
                                            let next = hide_core::device_command::next_action(&code);
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::Query(query) => {
                                    let mut result = core
                                        .workspace_query(
                                            &cap.context.device_id,
                                            &cap.pane_id,
                                            query,
                                        )
                                        .map_err(|refusal| (refusal.reason.to_owned(), refusal.next_action))?;
                                    if result.context != cap.context {
                                        let (reason, next_action) = cap.changed_refusal();
                                        return Err((reason.to_owned(), next_action));
                                    }
                                    scope_page_query(
                                        &mut result,
                                        desktop_renderers.load(Ordering::SeqCst) > 0,
                                        page_renderers.load(Ordering::SeqCst) > 0,
                                    );
                                    serde_json::to_value(result).map_err(|_| {
                                        ("result_encoding_failed".to_owned(), "Reconnect Hide and retry")
                                    })?
                                }
                                ScopedRequest::Action(action) => {
                                    // A file outside every checkout of this
                                    // machine is the window's to grant; the
                                    // core gets the grant alone. Desk's tab
                                    // panel shows no outside file, and its
                                    // path may be a device's, so the core
                                    // places or refuses it.
                                    let boundary = roots.boundary();
                                    let outside = (caller_node.is_none()
                                        && !desk
                                        && boundary.names_this_node(Some(&cap.context.device_id)))
                                    .then(|| {
                                        crate::outside_open::outside_target(&action, boundary, || {
                                            crate::server::roots_current(&roots)
                                        })
                                    })
                                    .flatten();
                                    let action = match outside {
                                        Some((path, page)) => {
                                            if desktop_renderers.load(Ordering::SeqCst) == 0 {
                                                return Err((
                                                    "outside_file_no_window".to_owned(),
                                                    "Open the Hide desktop app on this computer and retry",
                                                ));
                                            }
                                            crate::outside_open::grant(&browser_control, &path, page)
                                                .map_err(|(reason, next_action)| (reason.to_owned(), next_action))?
                                        }
                                        None => action,
                                    };
                                    if matches!(action, hide_core::workspace_control::Action::OpenBrowser { .. }) && page_renderers.load(Ordering::SeqCst) == 0 {
                                        return Err(("browser_unsupported".to_owned(), NO_PAGE_RENDERER));
                                    }
                                    let preparation = core
                                        .workspace_prepare_action(
                                            &cap.context.device_id,
                                            &cap.pane_id,
                                            &cap.context,
                                            &command_request_id,
                                            &action,
                                        )
                                        .map_err(|refusal| (refusal.reason.to_owned(), refusal.next_action))?;
                                    let material = match preparation {
                                        hide_core::workspace_control::ActionPreparation::Cached(result) => {
                                            let result = result.map_err(|refusal| (refusal.reason.to_owned(), refusal.next_action))?;
                                            return serde_json::to_value(result).map_err(|_| {
                                                ("result_encoding_failed".to_owned(), "Reconnect Hide and retry")
                                            });
                                        }
                                        hide_core::workspace_control::ActionPreparation::Ready => Ok(None),
                                        hide_core::workspace_control::ActionPreparation::Read(source) => {
                                            source.read().map(Some)
                                        }
                                    };
                                    let result = core
                                        .workspace_action(
                                            &cap.context.device_id,
                                            &cap.pane_id,
                                            &cap.context,
                                            &command_request_id,
                                            action,
                                            material,
                                        )
                                        .map_err(|refusal| (refusal.reason.to_owned(), refusal.next_action))?;
                                    if result.context != cap.context {
                                        let (reason, next_action) = cap.changed_refusal();
                                        return Err((reason.to_owned(), next_action));
                                    }
                                    serde_json::to_value(result).map_err(|_| {
                                        ("result_encoding_failed".to_owned(), "Reconnect Hide and retry")
                                    })?
                                }
                            };
                            Ok::<_, (String, &str)>(result)
                        })
                        .await
                        };
                        // The relay's gateway side opens before the answer,
                        // so a refused upgrade is a reason, not a dropped
                        // socket; its URL stays in this process. A window on
                        // the caller's own machine is relayed there: the
                        // caller dials its node's relay URL itself.
                        let outcome = match (outcome, &relay_display) {
                            (Ok(Ok(result)), Some(display_id)) => {
                                use crate::browser_control::Relay;
                                let mut ready = relay_ready(display_id, &result);
                                let chosen = chosen
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                                    .take();
                                let opened = match chosen {
                                    Some(Relay::Core(url)) => {
                                        crate::browser_relay::connect(&url).await.map(Some)
                                    }
                                    Some(Relay::Through { link, relay_url }) => {
                                        relay_way = crate::browser_relay::Way::Link;
                                        crate::browser_relay::connect_through(link, &relay_url)
                                            .await
                                            .map(Some)
                                    }
                                    Some(Relay::Own(relay_url)) => {
                                        ready["relay_url"] = json!(relay_url);
                                        Ok(None)
                                    }
                                    None => Err((
                                        "browser_control_unavailable",
                                        "Reconnect the Hide desktop app and retry",
                                    )),
                                };
                                match opened {
                                    Ok(gateway) => {
                                        relay = gateway;
                                        Ok(Ok(ready))
                                    }
                                    Err((reason, next_action)) => {
                                        Ok(Err((reason.to_owned(), next_action)))
                                    }
                                }
                            }
                            (outcome, _) => outcome,
                        };
                        match outcome {
                            Ok(Ok(result)) => {
                                json!({"type":"workspace_result","request_id":request_id,"ok":true,"result":result})
                            }
                            Ok(Err((reason, next_action))) => {
                                let field = refusal_field(&reason);
                                let (reason, next_action) = refusal_answer(reason, next_action);
                                let mut answer = json!({"type":"workspace_result","request_id":request_id,"ok":false,"reason":reason,"next_action":next_action});
                                if let Some((name, value)) = field {
                                    answer[name] = value;
                                }
                                answer
                            }
                            Err(_) => {
                                json!({"type":"workspace_result","request_id":request_id,"ok":false,"reason":"query_unavailable","next_action":"Retry after reconnecting Hide"})
                            }
                        }
                    }
                }
                Err(_) => {
                    json!({"type":"workspace_result","ok":false,"reason":"invalid_request","next_action":"Check the command arguments and retry"})
                }
            }
        }
        _ => {
            json!({"type":"workspace_result","ok":false,"reason":"request_timeout","next_action":"Check Hide status and retry"})
        }
    };
    if socket
        .send(Message::Text(response.to_string().into()))
        .await
        .is_ok()
    {
        let claim = tokio::time::timeout(Duration::from_secs(2), socket.recv()).await;
        let mut claimed = false;
        if matches!(claim, Ok(Some(Ok(Message::Text(text)))) if text == r#"{"type":"workspace_claim"}"#)
        {
            let answer = match state.pane_capabilities.claim(&token) {
                Ok(()) => {
                    claimed = true;
                    json!({"type":"workspace_claimed"})
                }
                Err(reason) => json!({"type":"workspace_claim_refused","reason":reason}),
            };
            let _ = socket.send(Message::Text(answer.to_string().into())).await;
        }
        // A relay starts only for a caller that completed the claim, and
        // holds its slot until the pump ends.
        if let (Some(gateway), true) = (relay.take(), claimed) {
            crate::browser_relay::pump(
                &mut socket,
                gateway,
                relay_for.as_deref().unwrap_or(""),
                relay_way,
                std::future::pending(),
            )
            .await;
        }
        drop(relay_slot);
    }
    let _ = socket.send(Message::Close(None)).await;
    if one_shot {
        state.pane_capabilities.revoke(&token);
    }
    client_gone(&state, connection, false, false);
}

/// How many device file reads one client runs at once. A viewer asks for one
/// file; a read beyond this is most likely for a view the page has left, so
/// the oldest read is ended to make room and answered as `superseded`, so
/// whatever waits on it settles (D-15).
const DEVICE_READS_PER_CLIENT: usize = 2;

struct DeviceRead {
    request_id: String,
    path: String,
    task: tokio::task::AbortHandle,
}

/// Starts a device read and returns the error frame for the read it ended to
/// make room, if any.
/// `device` comes with the slot a linked node's screen holds for the read.
fn start_device_read(
    state: &AppState,
    reads: &mut std::collections::VecDeque<DeviceRead>,
    frames: tokio::sync::mpsc::Sender<Message>,
    (device, slot): (String, Option<crate::relay::RelaySlot>),
    event: Value,
) -> Option<Message> {
    reads.retain(|read| !read.task.is_finished());
    let mut superseded = None;
    if reads.len() >= DEVICE_READS_PER_CLIENT
        && let Some(oldest) = reads.pop_front()
    {
        oldest.task.abort();
        hide_core::diagnostic!(json!({
            "component": "hided", "kind": "device.file_bytes_ended",
            "device": device,
            "request_id": oldest.request_id.chars().take(LOGGED_PATH_CAP).collect::<String>(),
            "reason": "a newer read for this client took its place",
        }));
        superseded = Some(file_bytes_error(
            &oldest.request_id,
            &oldest.path,
            "superseded",
        ));
    }
    // Kept to answer this read by its own id if a newer one ends it.
    let (request_id, path) = (
        payload_str(&event, "request_id"),
        payload_str(&event, "path"),
    );
    let read = stream_device_file_bytes(
        frames,
        Arc::clone(&state.core),
        Arc::clone(&state.boundary),
        Arc::clone(&state.roots),
        device,
        event,
    );
    let task = tokio::spawn(async move {
        let _slot = slot;
        read.await
    });
    reads.push_back(DeviceRead {
        request_id,
        path,
        task: task.abort_handle(),
    });
    superseded
}

/// Reads a file of this machine for another machine's view, onto the view's
/// bulk stream, apart from the socket's frames (PRD D-67).
fn start_view_read(
    state: &AppState,
    mut bulk: tokio::sync::mpsc::Sender<Message>,
    event: Value,
) -> tokio::task::JoinHandle<()> {
    let state = state.clone();
    tokio::spawn(async move {
        let _ = send_file_bytes(&mut bulk, &state.boundary, &state.roots, &event).await;
    })
}

/// Constant in the token's length, so a byte-by-byte mismatch does not leak
/// how much of the token a caller guessed.
pub(crate) fn token_matches(offered: &str, expected: &str) -> bool {
    use subtle::ConstantTimeEq;
    offered.len() == expected.len() && offered.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// Forwards a client event to the core, or answers it here.
///
/// An empty answer means the event went to the core, which answers through
/// the snapshot stream. Anything the daemon answers itself is one or more
/// frames for this client alone: a directory listing, a path refusal, or the
/// binary frames of a `file_bytes` read.
enum ClientAction {
    FileBytes(Value),
    /// Something the operator does about pairing (`pairing_*`), which the
    /// daemon's pairing answers for this client alone.
    Pairing(Value),
    /// A `file_list` for a checkout on an SSH device, answered by its helper.
    DeviceListing(Value),
    /// Add a project's question for a machine's node: a folder under its
    /// home (`home_folder_list`), the projects worth adding there
    /// (`importable_projects`), or where a clone or a new project would
    /// land on a device (`clone_target`, `project_target`).
    NodeAsk(Value),
    /// A window's call on the automations that waits for the core's answer
    /// (`automation_screen`).
    Automation(Value),
    /// Settings › Bots' call whose answer carries a made bot's code
    /// (`access_screen`), from this machine's own window only.
    Access(Value),
    /// Comment mode on a browser View, answered when its page is ready.
    BrowserComment(Value),
    /// The operator's comment or Send, which the screen's feedback queue
    /// hands the core in order.
    Feedback(Value),
    /// The palette's search across every project (`file_index_all`),
    /// answered off the socket loop by `OneAtATime`.
    FileIndexAll(Value),
    /// An agent's ask read (`pane_ask`) or answered (`pane_choice`) from a
    /// window as a phone does (`mobile::screen_ask`), off the socket loop.
    ScreenAsk(Value),
    Replies(Vec<Message>),
}

/// The file events a checkout on an SSH device answers through that
/// device's helper. This machine's checkout roots and home say nothing about
/// another machine's paths, so these never meet the local boundary: the core
/// finds the checkout and its device in its own catalog, and the helper
/// confines the path to the checkout root it opened, or, for a registration,
/// to that device's own home (`hide_host::register`).
const DEVICE_FILE_EVENTS: [&str; 13] = [
    "create_workspace",
    "desk_view_open",
    "clone_repository",
    "file_list",
    "file_open",
    "reveal_path",
    "file_save",
    "file_create",
    "dir_create",
    "path_rename",
    "path_move",
    "path_copy",
    "path_trash",
];

/// The SSH device a file event names, or `None` for this machine. Any id
/// other than this node's, the legacy `local` included, is a device's and is
/// never read through this machine's boundary.
fn event_device(boundary: &Boundary, event: &Value) -> Option<String> {
    let device = event.pointer("/payload/device_id").and_then(Value::as_str);
    (!boundary.names_this_node(device)).then(|| device.unwrap_or_default().to_owned())
}

fn handle_client_text(
    state: &AppState,
    text: &str,
    connection: u64,
    client: Client,
    own_screen: bool,
    look_screen: &Option<hide_core::LookScreen>,
) -> Result<ClientAction, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("client json: {error}"))?;
    let mut event = if value.get("schema_version").is_some() && value.get("kind").is_some() {
        value
    } else {
        return Err("expected a core event {schema_version, kind, payload}".to_owned());
    };
    // Who may send the kind is the class table's alone (PRD D-67); what
    // follows only routes an admitted event to its handler.
    let kind = event
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Err(reason) = event_class::admit(kind, client) {
        event_class::log_refusal(kind, reason, client, connection);
        return Ok(ClientAction::Replies(vec![event_class::refusal(
            kind, reason,
        )]));
    }
    if event.get("kind").and_then(Value::as_str) == Some("file_bytes") {
        return Ok(ClientAction::FileBytes(event));
    }
    if event.get("kind").and_then(Value::as_str) == Some("file_index") {
        return Ok(ClientAction::Replies(handle_file_index(state, &event)));
    }
    if matches!(
        event.get("kind").and_then(Value::as_str),
        Some("pane_ask" | "pane_choice")
    ) {
        return Ok(ClientAction::ScreenAsk(event));
    }
    if event.get("kind").and_then(Value::as_str) == Some("file_index_all") {
        return Ok(ClientAction::FileIndexAll(event));
    }
    match event.get("kind").and_then(Value::as_str) {
        Some("home_folder_list" | "importable_projects") => {
            return Ok(ClientAction::NodeAsk(event));
        }
        Some(crate::automation_screen::KIND) => return Ok(ClientAction::Automation(event)),
        // A made bot's code lets a program in, so, like a pairing link, it
        // reaches this machine's own window alone: a linked node's screen
        // is a window too, but another machine's.
        Some("access_call") if !own_screen => {
            return Err(
                "access_call is for this machine's own window: another machine's screen lists the bots but makes none"
                    .to_owned(),
            );
        }
        Some("access_call") => return Ok(ClientAction::Access(event)),
        Some("project_target" | "clone_target")
            if event_device(&state.boundary, &event).is_some() =>
        {
            return Ok(ClientAction::NodeAsk(event));
        }
        Some("project_target") => {
            return Ok(ClientAction::Replies(vec![Message::Text(
                project_target(&state.boundary, &event).to_string().into(),
            )]));
        }
        Some("clone_target") => {
            return Ok(ClientAction::Replies(vec![Message::Text(
                clone_target(&state.boundary, &event).to_string().into(),
            )]));
        }
        Some("attachment_stage") => {
            return Ok(ClientAction::Replies(handle_attachment_stage(
                state, &event, connection,
            )));
        }
        Some("attachment_commit") => {
            screen_input(state, connection, &payload_str(&event, "pane_id"));
            return Ok(ClientAction::Replies(handle_attachment_commit(
                state, &event,
            )));
        }
        Some("thread_input") => {
            return Ok(ClientAction::Replies(handle_thread_input(state, &event)));
        }
        Some("desk_send") => {
            return Ok(ClientAction::Replies(handle_desk_send(state, &event)));
        }
        Some("path_import") => {
            return Ok(ClientAction::Replies(handle_path_import(state, &event)));
        }
        Some("attachment_cancel") => {
            state
                .attachments
                .discard(&payload_str(&event, "request_id"));
            return Ok(ClientAction::Replies(Vec::new()));
        }
        Some("ai_settings") => {
            return handle_ai_settings(state, event, connection);
        }
        // Keys and views go to the pane's node beside the core, which hears
        // only what the node reports of them (PRD core-host-node-terminal
        // D-05).
        Some("key") => {
            let (target, input) = terminal_key(&event)?;
            if let KeyTarget::Pane(pane) = &target {
                screen_input(state, connection, pane);
            }
            state.core.terminals.type_into(target, input, unix_ms_now());
            return Ok(ClientAction::Replies(Vec::new()));
        }
        Some("terminal_mouse") => {
            screen_input(state, connection, &payload_str(&event, "pane_id"));
        }
        // A screen's grid goes on to the core only while the pane follows
        // that screen (`pane_sizes`).
        Some("terminal_resize") => {
            let (pane, size) = terminal_grid(&event)?;
            let floor = event.pointer("/payload/floor").cloned();
            if state
                .pane_sizes
                .resized_measured(connection, &pane, size, floor)
                .is_none()
            {
                return Ok(ClientAction::Replies(Vec::new()));
            }
        }
        Some("terminal_viewport") => {
            let (pane, size, new_view) = terminal_view(&event)?;
            let shown = state.pane_sizes.view(connection, &pane, size);
            state.core.terminals.view(&pane, shown, new_view);
            return Ok(ClientAction::Replies(Vec::new()));
        }
        // A desktop window says whether its notifications are on; the daemon
        // judges agent changes for it only then (`desktop_notices.rs`).
        Some("desktop_activity") => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Activity {
                in_use: bool,
            }
            let activity: Activity = serde_json::from_value(event["payload"].clone())
                .map_err(|_| "desktop_activity.in_use must be a boolean".to_owned())?;
            state.desktop_notices.activity(connection, activity.in_use);
            state.mobile.wake_follower();
            return Ok(ClientAction::Replies(Vec::new()));
        }
        Some("desktop_notices") => {
            let Some(wanted) = event
                .get("payload")
                .and_then(|payload| payload.get("wanted"))
                .and_then(Value::as_bool)
            else {
                return Err("desktop_notices.wanted must be a boolean".to_owned());
            };
            let waiting = event["payload"]
                .get("needs_you")
                .map(|value| {
                    value
                        .as_bool()
                        .ok_or("desktop_notices.needs_you must be a boolean")
                })
                .transpose()?
                .unwrap_or(wanted);
            state
                .desktop_notices
                .set_waiting(connection, wanted && waiting);
            state.desktop_notices.set(connection, wanted);
            state.mobile.wake_follower();
            return Ok(ClientAction::Replies(Vec::new()));
        }
        // An item's origin is read for each window that shows it and closed
        // in the core only when the last such window lets it go.
        Some("backlog_origin") => {
            let owner = origin_identity(&event)?;
            state.origin_holds.hold(connection, &owner)?;
        }
        Some("backlog_origin_close") => {
            let owner = origin_identity(&event)?;
            state.origin_holds.let_go(connection, &owner, |key| {
                close_backlog_origin(&state.core, connection, key)
            });
            return Ok(ClientAction::Replies(Vec::new()));
        }
        // A window that has the keyboard, and the screen it draws: the
        // Workspace or Desk's desk (a window that names none draws the
        // Workspace); the core hears the screen of the window that took the
        // keyboard last, and a window that leaves without saying so lets go
        // of its focus with its connection.
        Some("window_focus") => {
            let payload = event.get("payload");
            let Some(focused) = payload
                .and_then(|payload| payload.get("focused"))
                .and_then(Value::as_bool)
            else {
                return Err("window_focus.focused must be a boolean".to_owned());
            };
            let screen = match payload.and_then(|payload| payload.get("screen")) {
                None => crate::demand::FocusScreen::Workspace,
                Some(screen) => screen
                    .as_str()
                    .and_then(crate::demand::FocusScreen::parse)
                    .ok_or_else(|| "window_focus.screen must be workspace or desk".to_owned())?,
            };
            state
                .window_focus_demand
                .set(connection, focused.then_some(screen), |now| {
                    dispatch_window_focus(&state.core, connection, now)
                });
            return Ok(ClientAction::Replies(Vec::new()));
        }
        Some("request_view") => {
            let Some(observing) = event
                .get("payload")
                .and_then(|payload| payload.get("observing"))
                .and_then(Value::as_bool)
            else {
                return Err("request_view.observing must be a boolean".to_owned());
            };
            Demand::RequestView
                .of(state)
                .set(connection, observing, |aggregate| {
                    dispatch_observation(&state.core, Demand::RequestView, connection, aggregate)
                });
            return Ok(ClientAction::Replies(Vec::new()));
        }
        // The operator's comments and their Send write a letter in the
        // operator's name, so only a window sends them (PRD
        // view-opener-feedback D-17, the `Window` class).
        Some("view_comment" | "view_feedback_send") => return Ok(ClientAction::Feedback(event)),
        Some("browser_comment") => return Ok(ClientAction::BrowserComment(event)),
        // The one thing a window asks of a guest's pane beside reading,
        // typing and its size (PRD device-connect-iroh D-74, B10): a window
        // that may type into it ends it, after its confirm (the `Window`
        // class), and hears whether it did.
        Some("guest_pane_close") => {
            let answer = guest_close_answer(&event, |pane| state.guests.close_pane(pane))
                .map_err(str::to_owned)?;
            return Ok(ClientAction::Replies(
                answer
                    .map(|frame| Message::Text(frame.to_string().into()))
                    .into_iter()
                    .collect(),
            ));
        }
        Some(kind) if kind.starts_with("pairing_") => {
            return Ok(ClientAction::Pairing(event));
        }
        Some(kind) if kind.starts_with("mobile_") => {
            if mobile_event_reaches(kind, own_screen)? {
                state.mobile.handle_event(connection, &event)?;
            }
            return Ok(ClientAction::Replies(Vec::new()));
        }
        // A window that shows the core's session thread holds it; the
        // thread closes when the last hold is let go or its window leaves
        // without a close (a reload, a crash, a lost socket), so its file
        // looks do not outlive the screens. A tool draws no thread and holds
        // none.
        Some(kind @ ("session_thread_open" | "session_thread_close")) => {
            let window = (client != Client::Tool).then_some(connection);
            let bytes =
                serde_json::to_vec(&event).map_err(|error| format!("event encode: {error}"))?;
            let dispatch = || state.core.dispatch(bytes);
            if kind == "session_thread_open" {
                let pane = event
                    .pointer("/payload/pane_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                state.thread_holds.open(window, pane, dispatch)?;
            } else {
                state.thread_holds.close(window, dispatch)?;
            }
            return Ok(ClientAction::Replies(Vec::new()));
        }
        Some("open_external") => {
            // This token-authenticated socket can be reached through an SSH
            // tunnel. The browser cannot prove it is on the daemon's host,
            // so it has no authority to start the host's OS handler.
            let path = payload_str(&event, "path");
            return Ok(ClientAction::Replies(vec![Message::Text(
                json!({"type":"open_external_result", "payload":{
                    "path":path, "ok":false, "reason":"untrusted_client"
                }})
                .to_string()
                .into(),
            )]));
        }
        _ => {}
    }
    let kind = event.get("kind").and_then(Value::as_str).unwrap_or("");
    if DEVICE_FILE_EVENTS.contains(&kind) && event_device(&state.boundary, &event).is_some() {
        if kind == "file_list" {
            return Ok(ClientAction::DeviceListing(event));
        }
        let bytes = serde_json::to_vec(&event).map_err(|error| format!("event encode: {error}"))?;
        return state
            .core
            .dispatch_from(bytes, look_screen.clone())
            .map(|()| ClientAction::Replies(Vec::new()));
    }
    if let Some(reply) = admit_event(&state.boundary, &mut event, || roots_current(&state.roots)) {
        return Ok(ClientAction::Replies(vec![Message::Text(
            reply.to_string().into(),
        )]));
    }
    let bytes = serde_json::to_vec(&event).map_err(|error| format!("event encode: {error}"))?;
    state
        .core
        .dispatch_from(bytes, look_screen.clone())
        .map(|()| ClientAction::Replies(Vec::new()))
}

/// One of the operator's comments or Sends, with the screenshot a page's
/// pick took for it.
struct FeedbackJob {
    event: Value,
    png: Option<Vec<u8>>,
}

impl FeedbackJob {
    /// Whether it carries a screenshot: a page's pick, or a screen's crop.
    fn shot(&self) -> bool {
        self.png.is_some() || self.event.pointer("/payload/shot_png").is_some()
    }
}

/// Comments and Sends a screen may have waiting. A View moves its line
/// comments as one relocate each, at most 50 per View, so a burst of them
/// fits.
const FEEDBACK_QUEUE: usize = 256;
/// Of those, the ones with a screenshot, each of up to 16 MiB.
const FEEDBACK_SHOTS: usize = 4;

/// The screen's feedback queue: what it holds, in the order the screen made
/// it, and how many of those carry a screenshot.
struct FeedbackQueue {
    jobs: tokio::sync::mpsc::Sender<FeedbackJob>,
    shots: Arc<std::sync::atomic::AtomicUsize>,
}

impl FeedbackQueue {
    fn new() -> (Self, tokio::sync::mpsc::Receiver<FeedbackJob>) {
        let (jobs, receiver) = tokio::sync::mpsc::channel(FEEDBACK_QUEUE);
        let shots = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (Self { jobs, shots }, receiver)
    }

    /// Takes `job` behind those waiting, or refuses it: the queue is full, or
    /// it holds as many screenshots as it may.
    fn push(&self, job: FeedbackJob) -> Result<(), ()> {
        use std::sync::atomic::Ordering;
        let shot = job.shot();
        if shot && self.shots.fetch_add(1, Ordering::SeqCst) >= FEEDBACK_SHOTS {
            self.shots.fetch_sub(1, Ordering::SeqCst);
            return Err(());
        }
        self.jobs.try_send(job).map_err(|_| {
            if shot {
                self.shots.fetch_sub(1, Ordering::SeqCst);
            }
        })
    }
}

/// Hands one comment or Send to the screen's feedback queue, which starts
/// with the first. A full queue is refused, and the answer says so.
fn queue_feedback(
    state: &AppState,
    feedback: &mut Option<FeedbackQueue>,
    job: FeedbackJob,
    frames: &tokio::sync::mpsc::Sender<String>,
) -> Option<String> {
    let queue = feedback.get_or_insert_with(|| spawn_feedback(state, frames.clone()));
    queue.push(job).err().map(|()| {
        json!({"type": "error", "payload": {"kind": "view_feedback", "reason": "busy"},
            "message": "Too many comments are still being saved; try again."})
        .to_string()
    })
}

/// The screen's feedback queue: one task that stages each comment's
/// screenshot (a decode of up to 16 MiB, a staging scan and an fsync) on a
/// blocking thread and then hands it to the core, one after another, so a
/// Send made after a comment reaches the core after it. It ends with the
/// screen, when the socket loop drops its sender. A refusal reaches that
/// screen as an error.
fn spawn_feedback(state: &AppState, frames: tokio::sync::mpsc::Sender<String>) -> FeedbackQueue {
    let (queue, mut jobs) = FeedbackQueue::new();
    let shots = Arc::clone(&queue.shots);
    let attachments = Arc::clone(&state.attachments);
    let core = Arc::clone(&state.core);
    let boundary = Arc::clone(&state.boundary);
    let roots = Arc::clone(&state.roots);
    tokio::spawn(async move {
        while let Some(job) = jobs.recv().await {
            let shot = job.shot();
            let (attachments, core, boundary, roots) = (
                Arc::clone(&attachments),
                Arc::clone(&core),
                Arc::clone(&boundary),
                Arc::clone(&roots),
            );
            let outcome = tokio::task::spawn_blocking(move || {
                let FeedbackJob { mut event, png } = job;
                match png {
                    Some(png) => {
                        stage_page_shot(&attachments, core.node().as_str(), &mut event, &png)
                    }
                    None if event.pointer("/payload/shot_png").is_some() => {
                        stage_comment_shot(&attachments, core.node().as_str(), &mut event)?
                    }
                    None => {}
                }
                if let Some(reply) = admit_event(&boundary, &mut event, || roots_current(&roots)) {
                    return Ok(Some(reply.to_string()));
                }
                let bytes =
                    serde_json::to_vec(&event).map_err(|error| format!("event encode: {error}"))?;
                core.dispatch(bytes).map(|()| None)
            })
            .await
            .unwrap_or_else(|error| Err(format!("comment failed: {error}")));
            if shot {
                shots.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
            let frame = match outcome {
                Ok(None) => continue,
                Ok(Some(reply)) => reply,
                Err(error) => json!({"type": "error", "payload": {}, "message": error}).to_string(),
            };
            if frames.send(frame).await.is_err() {
                break;
            }
        }
    });
    queue
}

/// A page comment's screenshot, taken as its place was picked: staged, and
/// its name put on the add. A page that gave none, or one that could not be
/// staged, leaves the comment without one, as the diagnostic log says.
fn stage_page_shot(
    attachments: &crate::attachments::Attachments,
    node: &str,
    event: &mut Value,
    png: &[u8],
) {
    match attachments.stage_view_shot(png) {
        Ok(name) => event["payload"]["shot"] = json!({"node": node, "name": name}),
        Err(error) => hide_core::diagnostic!(json!({
            "component": "browser_comment",
            "kind": "shot.stage_failed",
            "display_id": event.pointer("/payload/display_id"),
            "error": error,
        })),
    }
}

/// Replaces a `view_comment` add's `shot_png` (base64) with the staged
/// screenshot's node and name.
fn stage_comment_shot(
    attachments: &crate::attachments::Attachments,
    node: &str,
    event: &mut Value,
) -> Result<(), String> {
    use base64::Engine;
    let payload = event
        .get_mut("payload")
        .and_then(Value::as_object_mut)
        .ok_or("view_comment has no payload")?;
    let encoded = payload
        .remove("shot_png")
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or("view_comment.shot_png must be base64 text")?;
    if encoded.len() > 22 * 1024 * 1024 {
        return Err("The screenshot is larger than 16 MiB.".to_owned());
    }
    let png = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "view_comment.shot_png is not base64".to_owned())?;
    let name = attachments.stage_view_shot(&png)?;
    payload.insert("shot".to_owned(), json!({"node": node, "name": name}));
    Ok(())
}

/// Lists one folder of a device checkout on a blocking task and hands the
/// answer to the client's socket loop. A client gone by then drops it.
fn spawn_device_listing(
    state: &AppState,
    event: Value,
    frames: tokio::sync::mpsc::Sender<String>,
    slot: Option<crate::relay::RelaySlot>,
) {
    let core = Arc::clone(&state.core);
    let boundary = Arc::clone(&state.boundary);
    let roots = Arc::clone(&state.roots);
    tokio::spawn(async move {
        let _slot = slot;
        let frame = tokio::task::spawn_blocking(move || device_listing(&core, &boundary, &roots, &event))
            .await
            .unwrap_or_else(|error| {
                json!({"type": "error", "payload": {}, "message": format!("device listing failed: {error}")})
            });
        let _ = frames.send(frame.to_string()).await;
    });
}

/// One folder of a checkout on an SSH device. The root has to be a checkout
/// the core's catalog carries for that device; the folder is spelled under it
/// and the helper refuses anything that leaves it.
fn device_listing(
    core: &CoreHandle,
    boundary: &Boundary,
    roots: &crate::RootFollower,
    event: &Value,
) -> Value {
    let device = payload_str(event, "device_id");
    let root = payload_str(event, "root");
    let raw = payload_str(event, "path");
    if !device_root_known(boundary, roots, &device, &root) {
        return refused("file_list", &root, Refusal::OutsideCheckout);
    }
    let folder = if raw.is_empty() { root.clone() } else { raw };
    // Both are the device's paths in the wire spelling, related by names
    // alone whatever system the device runs.
    let relative = match hide_platform::path::wire_relative(&root, &folder) {
        Ok(relative) => relative.into_string(),
        Err(_) => return refused("file_list", &folder, Refusal::OutsideCheckout),
    };
    // `reason` is the host's refusal code, which the Explorer words; the
    // sentence beside it is the log's.
    let unavailable = |code: &str, reason: Option<&str>, message: String| {
        hide_core::diagnostic!(json!({
            "component": "hided",
            "kind": "device.listing_unavailable",
            "device": device,
            "code": code,
            "reason": reason,
            "message": message,
        }));
        json!({"type": "directory_unavailable", "payload": {
            "kind": "file_list", "device_id": device, "root_path": folder, "code": code, "reason": reason,
        }})
    };
    let channel = match core.node_link(&device) {
        Ok(channel) => channel,
        Err(message) => return unavailable("not_ready", None, message),
    };
    use hide_core::node_access::LinkError;
    match hide_core::node_access::list_folder(channel.as_ref(), &root, &relative) {
        Ok(listing) => {
            let base = folder.trim_end_matches('/');
            let entries: Vec<Value> = listing
                .entries
                .into_iter()
                .map(|entry| {
                    json!(crate::boundary::Entry {
                        path: format!("{base}/{}", entry.name),
                        name: entry.name,
                        is_directory: entry.is_directory,
                        inode: Some(entry.inode),
                    })
                })
                .collect();
            json!({"type": "directory_list", "payload": {
                "kind": "file_list", "device_id": device, "root_path": folder,
                "entries": entries, "truncated": listing.truncated,
            }})
        }
        Err(LinkError::NotConnected(message)) => unavailable("not_ready", None, message),
        Err(error @ LinkError::Busy) => unavailable("busy", None, error.to_string()),
        Err(LinkError::Unknown(message)) => unavailable("unknown", None, message),
        Err(LinkError::Refused(error)) => {
            unavailable("refused", Some(error.code.as_str()), error.message)
        }
    }
}

/// The reason a node ask's frame carries for a refusal: the node's own code
/// for a path it judged, `busy` for a link already full, and `disconnected`
/// for a machine that cannot be reached (`contracts/hided-ws.schema.json`,
/// `nodeAskReason`).
fn node_ask_reason(error: &hide_node_link::LinkError) -> &'static str {
    match error {
        hide_node_link::LinkError::Refused(refused) => match refused.code {
            hide_node_link::ErrorCode::OutsideRoot => "outside_home",
            hide_node_link::ErrorCode::NotFound => "not_found",
            hide_node_link::ErrorCode::NotADirectory => "not_a_directory",
            hide_node_link::ErrorCode::InvalidPath => "invalid_path",
            hide_node_link::ErrorCode::AlreadyExists => "already_exists",
            _ => "unreadable",
        },
        hide_node_link::LinkError::Busy => "busy",
        _ => "disconnected",
    }
}

/// Asks a machine's node Add a project's question on a blocking task and
/// hands the answer to the client's socket loop; a client gone by then drops
/// it. The answer is never forwarded to the core and changes nothing on disk.
fn spawn_node_ask(
    state: &AppState,
    event: Value,
    frames: tokio::sync::mpsc::Sender<String>,
    slot: Option<crate::relay::RelaySlot>,
) {
    let core = Arc::clone(&state.core);
    let boundary = Arc::clone(&state.boundary);
    tokio::spawn(async move {
        let _slot = slot;
        let frame = tokio::task::spawn_blocking(move || node_ask(&core, &boundary, &event))
            .await
            .unwrap_or_else(|error| {
                json!({"type": "error", "payload": {}, "message": format!("node ask failed: {error}")})
            });
        let _ = frames.send(frame.to_string()).await;
    });
}

/// How long a node may take to list a folder or judge a new folder's place.
const NODE_ASK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// How long a node may take to find the projects worth adding: it walks up
/// to `SESSION_DISCOVERY_LIMIT` session entries and judges up to
/// `JUDGED_FOLDER_CAP` folders, and answers by `IMPORTABLE_SEARCH_BUDGET`,
/// which this leaves room above for the link.
const IMPORTABLE_ASK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Add a project's question for a machine's node, answered as the frame the
/// dialog reads (PRD device-add-project D-03, D-07, D-08):
///
/// - `home_folder_list` {device_id, path}: one folder under that machine's
///   home (`Call::HomeList`), this machine's included, answered as a
///   `home_folder_list` frame carrying `listing` or `reason`;
/// - `importable_projects` {device_id}: the projects worth adding on that
///   machine (`Call::ImportableProjects`), its registrations left out,
///   answered as an `importable_projects` frame carrying `projects` or
///   `reason`, each source the node could not read logged as
///   `add_project.import_source_dropped`;
/// - a device's `clone_target` and `project_target`: where the clone or new
///   project would land under that device's home (`Call::NewFolderTarget`),
///   answered in the same frames this machine's `$HOME` line answers with.
///
/// `reason` is a code the dialog words: the node's refusal (`outside_home`,
/// `not_found`, `not_a_directory`, `invalid_path`, `already_exists`,
/// `unreadable`), or `disconnected` when the machine cannot be asked; the
/// node's sentence goes to the log.
fn node_ask(core: &CoreHandle, boundary: &Boundary, event: &Value) -> Value {
    use hide_core::node_access::LinkError;
    let kind = event.get("kind").and_then(Value::as_str).unwrap_or("");
    let device = Some(payload_str(event, "device_id"))
        .filter(|device| !boundary.names_this_node(Some(device)))
        .unwrap_or_else(|| boundary.node().as_str().to_owned());
    let reason = |error: LinkError| -> &'static str {
        let (code, message) = match &error {
            LinkError::Refused(refused) => (Some(refused.code), refused.message.clone()),
            other => (None, other.to_string()),
        };
        hide_core::diagnostic!(json!({
            "component": "hided", "kind": "add_project.node_refused", "event": kind,
            "device": device, "code": code.map(|code| code.as_str()), "message": message,
        }));
        node_ask_reason(&error)
    };
    let link = core.node_link(&device);
    match kind {
        "home_folder_list" => {
            let request = payload_str(event, "path");
            let answer = link.map_err(LinkError::NotConnected).and_then(|link| {
                hide_core::node_access::call_as::<hide_node_link::browse::HomeListing>(
                    link.as_ref(),
                    hide_node_link::protocol::Call::HomeList {
                        path: request.clone(),
                    },
                    NODE_ASK_TIMEOUT,
                )
            });
            let (listing, refused) = match answer {
                Ok(listing) => (Some(listing), None),
                Err(error) => (None, Some(reason(error))),
            };
            json!({"type": "home_folder_list", "payload": {
                "device_id": device, "request": request, "listing": listing, "reason": refused,
            }})
        }
        "importable_projects" => {
            let answer = link.map_err(LinkError::NotConnected).and_then(|link| {
                // The core's registrations there are left out before the
                // node keeps its newest twenty.
                let exclude = core
                    .registered_paths(&device)
                    .map_err(LinkError::NotConnected)?;
                hide_core::node_access::call_as::<hide_node_link::importable::ImportableProjects>(
                    link.as_ref(),
                    hide_node_link::protocol::Call::ImportableProjects { exclude },
                    IMPORTABLE_ASK_TIMEOUT,
                )
            });
            let (projects, refused) = match answer {
                Ok(found) => {
                    // A source the node could not read is the log's, never
                    // the screen's (PRD device-add-project D-12, B27).
                    for dropped in &found.dropped {
                        hide_core::diagnostic!(json!({
                            "component": "hided", "kind": "add_project.import_source_dropped",
                            "device": device, "source": dropped.source, "reason": dropped.reason,
                        }));
                    }
                    (Some(found.projects), None)
                }
                Err(error) => (None, Some(reason(error))),
            };
            json!({"type": "importable_projects", "payload": {
                "device_id": device, "projects": projects, "reason": refused,
            }})
        }
        _ => {
            let parent = payload_str(event, "parent");
            let name = payload_str(event, "name");
            let project = kind == "project_target";
            let answer = link.map_err(LinkError::NotConnected).and_then(|link| {
                hide_core::node_access::call_as::<hide_node_link::browse::NewFolderTarget>(
                    link.as_ref(),
                    hide_node_link::protocol::Call::NewFolderTarget {
                        parent: parent.clone(),
                        name: name.clone(),
                        kind: if project {
                            hide_node_link::browse::NewFolderKind::Project
                        } else {
                            hide_node_link::browse::NewFolderKind::Clone
                        },
                    },
                    NODE_ASK_TIMEOUT,
                )
            });
            let (target, refused) = match answer {
                Ok(target) => (Some(target), None),
                Err(error) => (None, Some(reason(error))),
            };
            let path = target.as_ref().and_then(|target| target.path.clone());
            if project {
                json!({"type": "project_target", "payload": {
                    "device_id": device, "parent": parent, "name": name,
                    "parent_path": target.as_ref().map(|target| target.parent.clone()),
                    "parent_label": target.as_ref().map(|target| target.parent_label.clone()),
                    "path": path, "leftover": target.as_ref().is_some_and(|target| target.leftover),
                    "reason": refused,
                }})
            } else {
                json!({"type": "clone_target", "payload": {
                    "device_id": device, "parent": parent, "name": name, "path": path, "reason": refused,
                }})
            }
        }
    }
}

/// Splits an `ai_settings` event: each observation hint (`observing` for the
/// Settings agents tab, `start_observing` for a start surface) is this
/// connection's demand and reaches the core only when its aggregate changes;
/// a provider or model choice goes to the core as it came.
fn handle_ai_settings(
    state: &AppState,
    mut event: Value,
    connection: u64,
) -> Result<ClientAction, String> {
    for demand in [Demand::Settings, Demand::Start] {
        let hint = event
            .get_mut("payload")
            .and_then(Value::as_object_mut)
            .and_then(|payload| payload.remove(demand.field()));
        match hint {
            Some(Value::Bool(observing)) => {
                demand.of(state).set(connection, observing, |aggregate| {
                    dispatch_observation(&state.core, demand, connection, aggregate)
                });
            }
            Some(Value::Null) | None => {}
            Some(_) => return Err(format!("ai_settings.{} must be a boolean", demand.field())),
        }
    }
    let carries_choice = event
        .get("payload")
        .and_then(Value::as_object)
        .is_some_and(|payload| !payload.is_empty());
    if carries_choice {
        let bytes = serde_json::to_vec(&event).map_err(|error| format!("event encode: {error}"))?;
        state.core.dispatch(bytes)?;
    }
    Ok(ClientAction::Replies(Vec::new()))
}

/// The two screens whose look costs the core a provider probe.
#[derive(Clone, Copy)]
pub(crate) enum Demand {
    /// The Settings agents tab: the catalog and the hook diagnosis.
    Settings,
    /// A start surface: the catalog only.
    Start,
    /// The Overview's request view: the running checks' re-read.
    RequestView,
}

impl Demand {
    fn field(self) -> &'static str {
        match self {
            Self::Settings => "observing",
            Self::Start => "start_observing",
            Self::RequestView => "request_view",
        }
    }

    pub(crate) fn of(self, state: &AppState) -> &crate::demand::ObservationDemand {
        match self {
            Self::Settings => &state.demand,
            Self::Start => &state.start_demand,
            Self::RequestView => &state.request_view_demand,
        }
    }
}

/// Runs under the demand lock (`ObservationDemand::set`), so it only logs
/// and hands the event to the core's channel.
/// Tells the core no window shows this owner's origin any more.
fn close_backlog_origin(
    core: &CoreHandle,
    connection: u64,
    owner: &hide_core::links::ledger::BacklogOriginIdentity,
) {
    let event = json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "backlog_origin_close",
        "payload": owner,
    });
    if let Err(error) = core.dispatch(event.to_string().into_bytes()) {
        hide_core::diagnostic!(
            json!({"component": "hided", "kind": "backlog.origin_release_failed", "connection": connection, "message": error})
        );
    }
}

fn origin_identity(
    event: &Value,
) -> Result<hide_core::links::ledger::BacklogOriginIdentity, String> {
    let owner: hide_core::links::ledger::BacklogOriginIdentity = serde_json::from_value(
        event
            .get("payload")
            .cloned()
            .ok_or_else(|| "backlog_origin.payload is required".to_owned())?,
    )
    .map_err(|error| format!("backlog_origin: {error}"))?;
    owner.validate()?;
    Ok(owner)
}

pub(crate) fn dispatch_observation(
    core: &CoreHandle,
    demand: Demand,
    connection: u64,
    observing: bool,
) {
    hide_core::diagnostic!(json!({
        "component": "hided",
        "kind": "settings.observation",
        "connection": connection,
        "demand": demand.field(),
        "observing": observing,
    }));
    let event = match demand {
        Demand::RequestView => json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "request_view",
            "payload": {"observing": observing},
        }),
        Demand::Settings | Demand::Start => json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "ai_settings",
            "payload": {demand.field(): observing},
        }),
    };
    if let Err(error) = core.dispatch(event.to_string().into_bytes()) {
        hide_core::diagnostic!(json!({
            "component": "hided",
            "kind": "settings.observation_failed",
            "connection": connection,
            "message": error,
        }));
    }
}

/// Tells the core which screen the window holding the keyboard draws, or
/// that none does (`window_focus`). Runs under the focus demand's lock, so it
/// only logs and hands the event to the core's channel.
fn dispatch_window_focus(
    core: &CoreHandle,
    connection: u64,
    screen: Option<crate::demand::FocusScreen>,
) {
    let screen = screen.map(crate::demand::FocusScreen::as_str);
    hide_core::diagnostic!(json!({
        "component": "hided",
        "kind": "window_focus.observation",
        "connection": connection,
        "screen": screen,
    }));
    let event = json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "window_focus",
        "payload": {"focused": screen.is_some(), "screen": screen.unwrap_or("workspace")},
    });
    if let Err(error) = core.dispatch(event.to_string().into_bytes()) {
        hide_core::diagnostic!(json!({
            "component": "hided",
            "kind": "window_focus.observation_failed",
            "connection": connection,
            "message": error,
        }));
    }
}

/// Opens a checkout file with the host OS handler, or with the program
/// `HIDE_OPEN_COMMAND` names (PRD S3 D-12). The path passes the same
/// checkout-root boundary as every other Explorer path, so the handler can
/// only ever be pointed at a regular file inside a registered root.
#[allow(dead_code)] // Retained for a future transport that can prove local ownership.
fn handle_open_external(state: &AppState, event: &Value) -> Vec<Message> {
    let path = payload_str(event, "path");
    let real = match state.boundary.resolve_file(&path) {
        Ok((real, _)) => real,
        Err(refusal) => {
            return vec![Message::Text(
                refused("open_external", &path, refusal).to_string().into(),
            )];
        }
    };
    // The shell reveals an executable, an application bundle or an installer
    // rather than opening it, and this frame is a page's request rather than
    // the operator's own click, so the same rule holds here (D-12).
    if let Err(reason) = openable(&real) {
        hide_core::diagnostic!(json!({
            "component": "hided",
            "kind": "open.external.refused",
            "reason": reason,
        }));
        return vec![Message::Text(
            json!({
                "type": "open_external_result",
                "payload": {"path": hide_platform::path::to_wire_lossy(&real), "ok": false, "reason": reason},
            })
            .to_string()
            .into(),
        )];
    }
    let (ok, reason) = match state.opener.launch(&real) {
        Ok(()) => (true, Value::Null),
        Err(reason) => (false, Value::String(reason.to_owned())),
    };
    hide_core::diagnostic!(json!({
        "component": "hided",
        "kind": "open.external",
        "ok": ok,
        "open_handlers_in_flight": state.opener.in_flight(),
    }));
    vec![Message::Text(
        json!({
            "type": "open_external_result",
            "payload": {"path": hide_platform::path::to_wire_lossy(&real), "ok": ok, "reason": reason},
        })
        .to_string()
        .into(),
    )]
}

/// The extensions whose registered handler runs, installs or executes what it
/// opens rather than showing it: a launcher, a terminal script, an installer,
/// a package, a script interpreter's file. A page that can write inside a
/// checkout could otherwise name one and have the operator's own machine start
/// it, which is the line the shell draws for executable paths.
const EXECUTING_EXTENSIONS: &[&str] = &[
    // macOS bundles, packages, profiles and terminal scripts
    "app",
    "pkg",
    "mpkg",
    "dmg",
    "mobileconfig",
    "terminal",
    "term",
    "command",
    "tool",
    "workflow",
    "scpt",
    "scptd",
    // locators: the handler hands the target to something else, Terminal for
    // an ssh:// URL among them
    "webloc",
    "url",
    "inetloc",
    "fileloc",
    // shell and interpreter scripts whose handler runs them on open
    "sh",
    "bash",
    "zsh",
    "csh",
    "fish",
    "ksh",
    "py",
    "pyw",
    "pl",
    "rb",
    "php",
    "lua",
    "jar",
    "class",
    "appimage",
    "run",
    "desktop",
    "service",
    // Windows executables, script hosts and package installers
    "exe",
    "com",
    "scr",
    "pif",
    "bat",
    "cmd",
    "msi",
    "msp",
    "lnk",
    "ps1",
    "psm1",
    "psd1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "hta",
    "jnlp",
    "msc",
    "application",
    "appref-ms",
    "appx",
    "msix",
    "appinstaller",
    // shared libraries
    "dylib",
    "so",
];

/// Whether the host handler may be pointed at this file. An application
/// bundle, an installer, a script a handler would run, anything with an
/// execute bit, or a file whose own header says it is an executable is
/// refused, because a page must not be able to start a program by naming a
/// checkout file (D-12).
fn openable(path: &Path) -> Result<(), &'static str> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if EXECUTING_EXTENSIONS.contains(&extension.as_str()) {
        return Err("not_openable");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(path).map_err(|_| "not_found")?;
        if metadata.permissions().mode() & 0o111 != 0 {
            return Err("not_openable");
        }
    }
    let mut file = fs::File::open(path).map_err(|_| "not_found")?;
    if is_executable_header(&mut file)? {
        return Err("not_openable");
    }
    Ok(())
}

/// Whether the file's own header says it is a program: Mach-O (thin or fat,
/// either byte order), ELF, or the DOS/PE MZ family. A real MZ executable
/// either carries the zero fields its header format requires or the loader
/// signature its header points at, while a document that merely begins with
/// those letters has neither.
fn is_executable_header(file: &mut fs::File) -> Result<bool, &'static str> {
    use std::io::{Read, Seek, SeekFrom};
    let mut head = [0u8; 512];
    let read = file.read(&mut head).map_err(|_| "not_found")?;
    let head = &head[..read];
    if head.len() < 4 {
        return Ok(false);
    }
    let magic4 = [head[0], head[1], head[2], head[3]];
    if matches!(
        magic4,
        // Mach-O 32/64 and their byte-swapped forms, 32 and 64-bit fat
        [0xFE, 0xED, 0xFA, 0xCE]
            | [0xFE, 0xED, 0xFA, 0xCF]
            | [0xCE, 0xFA, 0xED, 0xFE]
            | [0xCF, 0xFA, 0xED, 0xFE]
            | [0xCA, 0xFE, 0xBA, 0xBE]
            | [0xBE, 0xBA, 0xFE, 0xCA]
            | [0xCA, 0xFE, 0xBA, 0xBF]
            | [0xBF, 0xBA, 0xFE, 0xCA]
            // ELF
            | [0x7F, b'E', b'L', b'F']
    ) {
        return Ok(true);
    }
    if head[0] != b'M' || head[1] != b'Z' {
        return Ok(false);
    }
    // Every real MZ family executable has zero fields in its header; a text
    // document that begins with those two letters has none.
    if head.contains(&0) {
        return Ok(true);
    }
    // `e_lfanew` says where the loader signature is. A header that points at
    // PE/NE/LE/LX is a program whatever its extension claims.
    if head.len() < 0x40 {
        return Ok(false);
    }
    let at = u64::from(u32::from_le_bytes([
        head[0x3C], head[0x3D], head[0x3E], head[0x3F],
    ]));
    if at < 0x40 || file.seek(SeekFrom::Start(at)).is_err() {
        return Ok(false);
    }
    let mut signature = [0u8; 4];
    if file.read_exact(&mut signature).is_err() {
        return Ok(false);
    }
    Ok(matches!(
        &signature,
        b"PE\0\0" | b"NE\0\0" | b"LE\0\0" | b"LX\0\0" | b"W4\0\0" | b"DL\0\0"
    ))
}

/// One line of a refused attachment: the request and why nothing was staged.
fn attachment_refused(request_id: &str, reason: &str) -> Value {
    json!({"type": "attachment_refused", "payload": {"request_id": request_id, "reason": reason}})
}

/// A session thread's input box (PRD session-docs-and-assistant B29): the
/// files it staged are committed here, where they were staged, and the core
/// is told their paths with the words, which it types into the thread's
/// pane. A refusal before the core is answered as an attachment's is, by
/// the input's request id; the core's own answer is in the thread section.
fn handle_thread_input(state: &AppState, event: &Value) -> Vec<Message> {
    let request_id = payload_str(event, "request_id");
    let refuse = |reason: &str| {
        vec![Message::Text(
            attachment_refused(&request_id, reason).to_string().into(),
        )]
    };
    if !attachments::valid_request_id(&request_id) {
        return refuse("invalid_request_id");
    }
    let (Some(generation), Some(text)) = (
        event.pointer("/payload/generation").and_then(Value::as_u64),
        event.pointer("/payload/text").and_then(Value::as_str),
    ) else {
        return refuse("invalid_item");
    };
    let stages: Vec<String> = event
        .pointer("/payload/stages")
        .and_then(Value::as_array)
        .map(|stages| {
            stages
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let paths = if stages.is_empty() {
        Vec::new()
    } else {
        match state.attachments.commit(&stages, false) {
            Ok(paths) => paths,
            Err(reason) => return refuse(reason),
        }
    };
    let input = json!({
        "schema_version": 2,
        "kind": "session_thread_input",
        "payload": {
            "generation": generation,
            "pane_id": payload_str(event, "pane_id"),
            "request_id": request_id,
            "text": text,
            "paths": paths,
        },
    });
    if let Err(error) = state
        .core
        .dispatch(serde_json::to_vec(&input).unwrap_or_default())
    {
        log_snapshot_failure("thread_input", &error);
        return refuse("forward_failed");
    }
    Vec::new()
}

/// A message to Desk's agent (PRD desk-2-ui B41): the files its composer
/// staged are committed here, where they were staged, and the core is told
/// their names, sizes and paths with the words. hided is the only maker of
/// `files`, so the payload is rebuilt from the fields a window may send and
/// a window can never name a path. A refusal before the core is answered as
/// an attachment's is, by the message's request id.
fn handle_desk_send(state: &AppState, event: &Value) -> Vec<Message> {
    let request_id = payload_str(event, "request_id");
    let refuse = |reason: &str| {
        vec![Message::Text(
            attachment_refused(&request_id, reason).to_string().into(),
        )]
    };
    let stages: Vec<String> = event
        .pointer("/payload/stages")
        .and_then(Value::as_array)
        .map(|stages| {
            stages
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let files = if stages.is_empty() {
        Vec::new()
    } else {
        if !attachments::valid_request_id(&request_id) {
            return refuse("invalid_request_id");
        }
        // The files Desk still has to send stay whatever its folder's cap says.
        let in_use = match state.core.desk_files_in_use() {
            Ok(in_use) => in_use,
            Err(error) => {
                log_snapshot_failure("desk_files_in_use", &error);
                return refuse("forward_failed");
            }
        };
        match state.attachments.commit_named(&stages, &in_use) {
            Ok(files) => files,
            Err(reason) => return refuse(reason),
        }
    };
    let send = desk_send_for_core(event, &files);
    if let Err(error) = state
        .core
        .dispatch(serde_json::to_vec(&send).unwrap_or_default())
    {
        log_snapshot_failure("desk_send", &error);
        return refuse("forward_failed");
    }
    Vec::new()
}

/// The core's `desk_send` for a window's: the words, mentions and `again`
/// it sent, and `files` from hided's own commit only, so a path a window
/// wrote into the event never reaches Desk's agent.
fn desk_send_for_core(event: &Value, files: &[attachments::CommittedFile]) -> Value {
    let mut payload = json!({
        "text": event.pointer("/payload/text").cloned().unwrap_or(Value::Null),
        "mentions": event.pointer("/payload/mentions").cloned().unwrap_or_else(|| json!([])),
        "files": files,
    });
    if let Some(again) = event.pointer("/payload/again").and_then(Value::as_u64) {
        payload["again"] = Value::from(again);
    }
    json!({"schema_version": 2, "kind": "desk_send", "payload": payload})
}

/// Opens one staged upload the client will send bytes for. The caps are
/// checked here so a too-large file is refused before its bytes arrive.
fn handle_attachment_stage(state: &AppState, event: &Value, connection: u64) -> Vec<Message> {
    let request_id = payload_str(event, "request_id");
    let name = payload_str(event, "name");
    let size = event
        .pointer("/payload/size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let clipboard = event
        .pointer("/payload/clipboard")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match state
        .attachments
        .begin(connection, &request_id, &name, size, clipboard)
    {
        Ok(()) => Vec::new(),
        Err(reason) => vec![Message::Text(
            attachment_refused(&request_id, reason).to_string().into(),
        )],
    }
}

/// Turns staged uploads into the one `terminal_attachment` event the core
/// takes for a batch, and reports clipboard readiness, because hided
/// staged the file the core is waiting for.
fn handle_attachment_commit(state: &AppState, event: &Value) -> Vec<Message> {
    let request_id = payload_str(event, "request_id");
    let pane_id = payload_str(event, "pane_id");
    if pane_id.is_empty() {
        return vec![Message::Text(
            attachment_refused(&request_id, "no_pane")
                .to_string()
                .into(),
        )];
    }
    let clipboard = event
        .pointer("/payload/clipboard")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let stages: Vec<String> = event
        .pointer("/payload/stages")
        .and_then(Value::as_array)
        .map(|stages| {
            stages
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    // A clipboard paste names no paths: the core reads the image at the path
    // its own request id derives, so the commit must be the stage that wrote
    // it and nothing else.
    if clipboard && (stages.len() != 1 || stages.first().is_none_or(|stage| *stage != request_id)) {
        return vec![Message::Text(
            attachment_refused(&request_id, "invalid_request_id")
                .to_string()
                .into(),
        )];
    }
    if !attachments::valid_request_id(&request_id) {
        return vec![Message::Text(
            attachment_refused(&request_id, "invalid_request_id")
                .to_string()
                .into(),
        )];
    }
    let paths = match state.attachments.commit(&stages, clipboard) {
        Ok(paths) => paths,
        Err(reason) => {
            return vec![Message::Text(
                attachment_refused(&request_id, reason).to_string().into(),
            )];
        }
    };
    let attachment = json!({
        "schema_version": 2,
        "kind": "terminal_attachment",
        "payload": {
            "request_id": request_id,
            "pane_id": pane_id,
            "clipboard": clipboard,
            "paths": paths,
        },
    });
    if let Err(error) = state
        .core
        .dispatch(serde_json::to_vec(&attachment).unwrap_or_default())
    {
        log_snapshot_failure("attachment", &error);
        return vec![Message::Text(
            attachment_refused(&request_id, "forward_failed")
                .to_string()
                .into(),
        )];
    }
    if clipboard {
        let ready = json!({
            "schema_version": 2,
            "kind": "terminal_attachment_ready",
            "payload": {"request_id": request_id, "pane_id": pane_id, "error": Value::Null},
        });
        if let Err(error) = state
            .core
            .dispatch(serde_json::to_vec(&ready).unwrap_or_default())
        {
            log_snapshot_failure("attachment", &error);
        }
    }
    Vec::new()
}

/// A window's drop from the file manager: the window uploaded each file's
/// bytes as an `attachment_stage`, and this event names where the items land
/// and which upload is which file. hided is the only maker of the core's
/// `path_import` event: it keeps the destination, the shape of each item and
/// the staged path hided itself wrote for an upload, and forwards nothing the
/// window sent as it came, so a window cannot name a file for the core to read.
fn handle_path_import(state: &AppState, event: &Value) -> Vec<Message> {
    /// Bytes of one item's path below the destination folder.
    const IMPORT_PATH_MAX_BYTES: usize = 4096;
    use hide_node_link::mutate::IMPORT_MAX_ITEMS;
    let request_id = payload_str(event, "request_id");
    let refuse = |reason: &str| {
        vec![Message::Text(
            attachment_refused(&request_id, reason).to_string().into(),
        )]
    };
    let items: Vec<(String, bool, Option<String>)> = event
        .pointer("/payload/items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    (
                        item.get("path")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        item.get("directory")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        item.get("stage").and_then(Value::as_str).map(str::to_owned),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if items.is_empty() || items.len() > IMPORT_MAX_ITEMS {
        return refuse("too_many_files");
    }
    // A name is one component of a path the page chose; the host judges it
    // again against the system it writes on.
    let shaped = items.iter().all(|(path, directory, stage)| {
        !path.is_empty()
            && path.len() <= IMPORT_PATH_MAX_BYTES
            && path.split('/').all(boundary::valid_name)
            && (*directory == stage.is_none())
    });
    if !shaped {
        return refuse("invalid_item");
    }
    let stages: Vec<String> = items
        .iter()
        .filter_map(|(_, _, stage)| stage.clone())
        .collect();
    let mut core_event = json!({
        "schema_version": 2,
        "kind": "path_import",
        "payload": {
            "root": payload_str(event, "root"),
            "destination": payload_str(event, "destination"),
        },
    });
    if let Some(device) = event.pointer("/payload/device_id") {
        core_event["payload"]["device_id"] = device.clone();
    }
    // This machine's checkout is judged against its registered roots; a
    // device's paths are its helper's to confine.
    if event_device(&state.boundary, &core_event).is_none()
        && let Some(refusal) = explorer_rooted_path(
            &state.boundary,
            &mut core_event,
            "path_import",
            "destination",
        )
    {
        for stage in &stages {
            state.attachments.discard(stage);
        }
        return vec![Message::Text(refusal.to_string().into())];
    }
    let paths = if stages.is_empty() {
        Vec::new()
    } else {
        match state.attachments.commit_dropped(&stages) {
            Ok(paths) => paths,
            Err(reason) => {
                for stage in &stages {
                    state.attachments.discard(stage);
                }
                return refuse(reason);
            }
        }
    };
    let mut staged = paths.into_iter();
    core_event["payload"]["items"] = Value::Array(
        items
            .iter()
            .map(|(path, directory, stage)| {
                json!({
                    "path": path,
                    "directory": directory,
                    "staged": stage.as_ref().and_then(|_| staged.next()),
                })
            })
            .collect(),
    );
    if let Err(error) = state
        .core
        .dispatch(serde_json::to_vec(&core_event).unwrap_or_default())
    {
        log_snapshot_failure("path_import", &error);
        return refuse("forward_failed");
    }
    Vec::new()
}

/// A `file_index` query: the root is checked against the registered checkouts
/// and the daemon answers from its per-root index. The first query for a root
/// starts the walk and answers `indexing: true`; the next one has the list.
fn handle_file_index(state: &AppState, event: &Value) -> Vec<Message> {
    let root = payload_str(event, "root");
    let query = payload_str(event, "query");
    let answer = match event_device(&state.boundary, event) {
        Some(device) => device_file_index(state, &device, &root, &query),
        None => local_file_index(state, &root, &query),
    };
    vec![Message::Text(answer.to_string().into())]
}

/// What one screen has asked a [`OneAtATime`] worker and not had answered.
trait Waiting: Default + Send + 'static {
    /// The next request to answer, if any.
    fn next(&mut self) -> Option<Value>;
    /// Whether a request waits.
    fn any(&self) -> bool;
}

/// The search keeps only the newest query.
impl Waiting for Option<Value> {
    fn next(&mut self) -> Option<Value> {
        self.take()
    }

    fn any(&self) -> bool {
        self.is_some()
    }
}

/// A screen's asks of agents (`mobile::screen_ask`, PRD desk-2-ui B14): its
/// choices in the order pressed, each answered, and the newest `pane_ask` of
/// each pane, since an older one would only be answered with what the newer
/// one reads. A choice goes first: it is what the operator pressed.
#[derive(Default)]
struct ScreenAsks {
    choices: std::collections::VecDeque<Value>,
    asks: Vec<Value>,
}

/// Choices a screen may have waiting; one past it is answered `busy`.
const SCREEN_CHOICES_WAITING: usize = 8;
/// Panes whose ask a screen may have waiting; past it the oldest gives way,
/// as the newer read answers what the window shows now.
const SCREEN_ASKS_WAITING: usize = 16;

impl ScreenAsks {
    /// Adds `event`, or answers it at once when a choice finds the line full.
    fn add(&mut self, event: Value) -> Result<(), Value> {
        if event.get("kind").and_then(Value::as_str) == Some("pane_choice") {
            if self.choices.len() >= SCREEN_CHOICES_WAITING {
                return Err(json!({"type": "pane_choice_result", "payload": {
                    "request_id": event.pointer("/payload/request_id"), "ok": false, "reason": "busy",
                }}));
            }
            self.choices.push_back(event);
            return Ok(());
        }
        let pane = |event: &Value| {
            (
                event.pointer("/payload/device_id").cloned(),
                event.pointer("/payload/pane_id").cloned(),
            )
        };
        self.asks.retain(|waiting| pane(waiting) != pane(&event));
        if self.asks.len() >= SCREEN_ASKS_WAITING {
            self.asks.remove(0);
        }
        self.asks.push(event);
        Ok(())
    }
}

impl Waiting for ScreenAsks {
    fn next(&mut self) -> Option<Value> {
        self.choices
            .pop_front()
            .or_else(|| (!self.asks.is_empty()).then(|| self.asks.remove(0)))
    }

    fn any(&self) -> bool {
        !self.choices.is_empty() || !self.asks.is_empty()
    }
}

/// A screen's `pane_ask` or `pane_choice`, answered on the blocking pool.
fn screen_ask(state: &AppState, event: &Value) -> Value {
    let payload = event.get("payload").cloned().unwrap_or(Value::Null);
    match event.get("kind").and_then(Value::as_str) {
        Some("pane_choice") => crate::mobile::screen_ask::choose(&state.mobile, &payload),
        _ => crate::mobile::screen_ask::ask_frame(&state.mobile, &payload),
    }
}

/// One screen's requests that read every project or every agent, answered
/// one at a time on a blocking worker and never on the socket loop: the
/// search across every project (`file_index_all`, PRD desk-2-ui B38, D-23),
/// which ranks up to 50,000 paths per project, and an agent's ask read or
/// answered from a window (`pane_ask`, `pane_choice`), each a core snapshot
/// and a projection of every agent. What waits is `W`'s to bound: a query
/// typed while one runs replaces the one waiting, so a fast typist starts at
/// most one more and the answers end on the newest query.
struct OneAtATime<W> {
    waiting: Arc<std::sync::Mutex<W>>,
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl<W> Clone for OneAtATime<W> {
    fn clone(&self) -> Self {
        Self {
            waiting: Arc::clone(&self.waiting),
            running: Arc::clone(&self.running),
        }
    }
}

impl<W: Waiting> Default for OneAtATime<W> {
    fn default() -> Self {
        Self {
            waiting: Arc::default(),
            running: Arc::default(),
        }
    }
}

impl<W: Waiting> OneAtATime<W> {
    fn waiting(&self) -> std::sync::MutexGuard<'_, W> {
        self.waiting
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Puts a request in line with `put`, which may answer it at once
    /// instead, and starts the worker unless it runs; `answer` makes each
    /// request's frame.
    fn ask(
        &self,
        state: &AppState,
        frames: tokio::sync::mpsc::Sender<String>,
        put: impl FnOnce(&mut W) -> Result<(), Value>,
        answer: fn(&AppState, &Value) -> Value,
    ) {
        use std::sync::atomic::Ordering::SeqCst;
        if let Err(frame) = put(&mut self.waiting()) {
            tokio::spawn(async move {
                let _ = frames.send(frame.to_string()).await;
            });
            return;
        }
        if self.running.swap(true, SeqCst) {
            return;
        }
        let (worker, state) = (self.clone(), state.clone());
        tokio::spawn(async move {
            loop {
                let Some(event) = worker.waiting().next() else {
                    worker.running.store(false, SeqCst);
                    // A request that arrived between the take and the store
                    // finds the worker still running and leaves it here.
                    if worker.waiting().any() && !worker.running.swap(true, SeqCst) {
                        continue;
                    }
                    return;
                };
                let state = state.clone();
                let frame = tokio::task::spawn_blocking(move || answer(&state, &event))
                    .await
                    .unwrap_or_else(|error| {
                        json!({"type": "error", "payload": {}, "message": format!("request failed: {error}")})
                    });
                if frames.send(frame.to_string()).await.is_err() {
                    worker.running.store(false, SeqCst);
                    return;
                }
            }
        });
    }
}

/// The `file_index_all_result` frame for one query: each project's root (one
/// per project, `IndexService::set_projects`) asked of its own index, at most
/// `SEARCH_WALKS_MAX` walks at once, merged and capped at `RESULT_LIMIT`, and
/// only the rows kept resolved to the paths a screen opens. `prefer`, the
/// project the screen shows first, keeps up to half of them.
fn file_index_all(state: &AppState, event: &Value) -> Value {
    let query = payload_str(event, "query");
    let prefer = event.pointer("/payload/prefer").and_then(|prefer| {
        Some((
            prefer.get("device_id")?.as_str()?.to_owned(),
            prefer.get("root_path")?.as_str()?.to_owned(),
        ))
    });
    let node = state.boundary.node().as_str().to_owned();
    let searched = state.index.search(
        &query,
        prefer.as_ref(),
        crate::index::RESULT_LIMIT,
        |(device, root)| {
            if *device == node {
                local_walk(state, root)
            } else {
                device_walk(state, device, root)
            }
        },
    );
    let files: Vec<Value> = searched
        .hits
        .iter()
        .filter_map(|hit| {
            let (device, root) = &hit.project;
            let relative = hide_platform::path::RelPath::parse(&hit.relative_path).ok()?;
            let path = hide_platform::path::wire_join(root, &relative);
            // This machine's file is answered only while it is still inside
            // its checkout; a device's helper confines its own.
            if *device == node && state.boundary.resolve_target(&path).is_err() {
                return None;
            }
            Some(json!({
                "device_id": device,
                "root_path": root,
                "path": path,
                "relative_path": hit.relative_path,
            }))
        })
        .collect();
    json!({"type": "file_index_all_result", "payload": {
        "query": query,
        "files": files,
        "more": searched.more,
        "index_truncated": searched.index_truncated,
        "indexing": searched.indexing,
        "unavailable": searched.unavailable.iter().map(|((device, root), code)| json!({
            "device_id": device,
            "root_path": root,
            "code": code,
        })).collect::<Vec<_>>(),
    }})
}

/// The walk of this machine's checkout `root`, opened on the walk's own
/// thread; a root the boundary no longer holds fails as `outside_checkout`.
fn local_walk(state: &AppState, root: &str) -> crate::index::WalkFn {
    let (boundary, core, root) = (
        Arc::clone(&state.boundary),
        Arc::clone(&state.core),
        root.to_owned(),
    );
    Box::new(move || {
        let (known, opened) = boundary
            .open_directory(Path::new(&root), &root)
            .map_err(|refusal| refusal.code().to_owned())?;
        let node = boundary.node().as_str().to_owned();
        let others: Vec<PathBuf> = core
            .registered_paths(&node)?
            .iter()
            .filter_map(|path| hide_host::root::native_absolute(path))
            .collect();
        Ok(hide_host::index::walk(
            &cap_std::fs::Dir::from_std_file(opened),
            &known,
            Some(boundary.home()),
            &others,
        ))
    })
}

/// The walk of a device's checkout `root`, by that device's helper; a
/// failure answers the code the palette words and logs the helper's sentence.
fn device_walk(state: &AppState, device: &str, root: &str) -> crate::index::WalkFn {
    let core = Arc::clone(&state.core);
    let (walk_device, walk_root) = (device.to_owned(), root.to_owned());
    Box::new(move || {
        let log = |code: &str, message: String| {
            hide_core::diagnostic!(json!({
                "component": "hided",
                "kind": "device.index_unavailable",
                "device": walk_device,
                "code": code,
                "message": message,
            }));
            code.to_owned()
        };
        let channel = core
            .node_link(&walk_device)
            .map_err(|message| log("link.not_connected", message))?;
        let others = core
            .registered_paths(&walk_device)
            .map_err(|message| log("core.unavailable", message))?;
        hide_core::node_access::index_root(channel.as_ref(), &walk_root, others).map_err(|error| {
            log(
                &hide_core::node_access::link_failure_code(&error),
                error.to_string(),
            )
        })
    })
}

/// A `file_index` query for a checkout of this machine: the root is checked
/// against the registered checkouts and the daemon answers from its index.
fn local_file_index(state: &AppState, root: &str, query: &str) -> Value {
    let found = match state.boundary.open_directory(Path::new(root), root) {
        Err(_) if roots_current(&state.roots) => {
            state.boundary.open_directory(Path::new(root), root)
        }
        found => found,
    };
    let Ok((known, opened)) = found else {
        return refused("file_index", root, Refusal::OutsideCheckout);
    };
    let root_path = hide_platform::path::to_wire_lossy(&known);
    let walk_root = known.clone();
    let (core, node, home) = (
        Arc::clone(&state.core),
        state.boundary.node().as_str().to_owned(),
        state.boundary.home().to_path_buf(),
    );
    let answer = state.index.query(
        state.boundary.node().as_str(),
        &root_path,
        query,
        move || {
            // The other projects of this machine, which the home project's
            // walk leaves to their own (PRD home-folder-project D-05); asked
            // here, on the walk's worker, once per walk.
            let others: Vec<PathBuf> = core
                .registered_paths(&node)?
                .iter()
                .filter_map(|path| hide_host::root::native_absolute(path))
                .collect();
            Ok(hide_host::index::walk(
                &cap_std::fs::Dir::from_std_file(opened),
                &walk_root,
                Some(&home),
                &others,
            ))
        },
    );
    index_result(
        state.boundary.node().as_str(),
        &root_path,
        query,
        answer,
        |relative| {
            let relative = hide_platform::path::RelPath::parse(relative).ok()?;
            let path = hide_platform::path::wire_join(&root_path, &relative);
            state.boundary.resolve_target(&path).ok().map(|_| path)
        },
    )
}

/// The `file_index_result` frame for one answer. `path_of` gives an entry's
/// absolute path on its device, or `None` to leave an entry out.
fn index_result(
    device: &str,
    root: &str,
    query: &str,
    answer: IndexAnswer,
    path_of: impl Fn(&str) -> Option<String>,
) -> Value {
    let (files, truncated, indexing, unavailable) = match answer {
        IndexAnswer::Indexing => (Vec::new(), false, true, None),
        IndexAnswer::Ready {
            entries,
            truncated,
            refreshing,
        } => (
            entries
                .iter()
                .filter_map(|relative| {
                    path_of(relative).map(|path| json!({"path": path, "relative_path": relative}))
                })
                .collect(),
            truncated,
            refreshing,
            None,
        ),
        IndexAnswer::Failed(message) => (Vec::new(), false, false, Some(message)),
    };
    json!({"type": "file_index_result", "payload": {
        "device_id": device,
        "root_path": root,
        "query": query,
        "files": files,
        "truncated": truncated,
        "indexing": indexing,
        "unavailable": unavailable,
    }})
}

/// A `file_index` query for a checkout on an SSH device. The root has to be
/// one the core's catalog carries for that device; the walk is the device's
/// helper's, reached on the index worker, and its paths are that device's.
fn device_file_index(state: &AppState, device: &str, root: &str, query: &str) -> Value {
    if !device_root_known(&state.boundary, &state.roots, device, root) {
        return refused("file_index", root, Refusal::OutsideCheckout);
    }
    // A walk that failed answers the code the palette words; the helper's
    // sentence goes to the log.
    let answer = state
        .index
        .query(device, root, query, device_walk(state, device, root));
    index_result(device, root, query, answer, |relative| {
        hide_platform::path::RelPath::parse(relative)
            .ok()
            .map(|relative| hide_platform::path::wire_join(root, &relative))
    })
}

/// Bytes one binary frame carries; a read streams in frames this size so a
/// large file never becomes one unbounded message on the shared socket.
const BYTES_CHUNK: u64 = 4 * 1024 * 1024;

/// The header of a binary frame: a 4-byte big-endian length, the header JSON,
/// then the bytes. The header carries the request it answers, the offset the
/// bytes start at, the file's total size, and whether this frame ends the
/// read, so a client can assemble a range without a second round trip.
fn bytes_frame(header: &Value, bytes: &[u8]) -> Vec<u8> {
    let json = header.to_string();
    let mut out = Vec::with_capacity(4 + json.len() + bytes.len());
    out.extend_from_slice(&(json.len() as u32).to_be_bytes());
    out.extend_from_slice(json.as_bytes());
    out.extend_from_slice(bytes);
    out
}

/// Splits a binary frame `bytes_frame` made into its header and its bytes;
/// `None` when it is not one.
pub(crate) fn split_bytes_frame(frame: &[u8]) -> Option<(Value, &[u8])> {
    let length = u32::from_be_bytes(frame.get(..4)?.try_into().ok()?) as usize;
    let header = serde_json::from_slice(frame.get(4..4 + length)?).ok()?;
    Some((header, &frame[4 + length..]))
}

/// A read the daemon could not serve as bytes: the file is past the read cap,
/// or the disk refused it. The client shows one line and offers no retry.
fn file_bytes_error(request_id: &str, path: &str, reason: &str) -> Message {
    hide_core::diagnostic!(json!({
        "component": "hided",
        "kind": "file_bytes.failed",
        "reason": reason,
        "path": path.chars().take(LOGGED_PATH_CAP).collect::<String>(),
    }));
    Message::Text(
        json!({
            "type": "file_bytes_error",
            "payload": {"request_id": request_id, "path": path, "reason": reason},
        })
        .to_string()
        .into(),
    )
}

/// The boundary's refusal for a device's answer about a path: a file that is
/// not there, or a path its root does not hold. Anything else is a read that
/// failed.
pub(crate) fn device_refusal(error: &hide_core::node_access::LinkError) -> Option<Refusal> {
    use hide_node_link::ErrorCode;
    let hide_core::node_access::LinkError::Refused(error) = error else {
        return None;
    };
    match error.code {
        ErrorCode::NotFound => Some(Refusal::NotFound),
        ErrorCode::OutsideRoot | ErrorCode::InvalidPath => Some(Refusal::OutsideCheckout),
        _ => None,
    }
}

/// A `file_bytes` read of a file on an SSH device. The root has to be a
/// checkout the core's catalog carries for that device and the path is spelled
/// under it; the device's helper confines the read and answers one bounded
/// range per call, so this keeps one range in memory as the local read does.
/// A file that changed between ranges ends the read as `read_failed` rather
/// than joining two files' bytes.
///
/// Each range is a helper round trip that can take seconds, so the read runs
/// in its own task and hands its frames to the client loop through a bounded
/// channel: the socket keeps carrying typing and snapshots meanwhile, and a
/// full channel holds the read back instead of buffering it.
pub(crate) async fn stream_device_file_bytes(
    frames: tokio::sync::mpsc::Sender<Message>,
    core: Arc<CoreHandle>,
    boundary: Arc<Boundary>,
    roots: Arc<crate::RootFollower>,
    device: String,
    event: Value,
) -> Result<(), ()> {
    let (device, event) = (device.as_str(), &event);
    let request_id = payload_str(event, "request_id");
    let path = payload_str(event, "path");
    let root = payload_str(event, "root");
    let offset = event
        .pointer("/payload/offset")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let length = event.pointer("/payload/length").and_then(Value::as_u64);
    let relative = hide_platform::path::wire_relative(&root, &path)
        .ok()
        .filter(|relative| !relative.is_root())
        .map(hide_platform::path::RelPath::into_string);
    let Some(relative) = relative.filter(|_| device_root_known(&boundary, &roots, device, &root))
    else {
        return frames
            .send(Message::Text(
                refused("file_bytes", &path, Refusal::OutsideCheckout)
                    .to_string()
                    .into(),
            ))
            .await
            .map_err(|_| ());
    };
    if length.is_some_and(|length| length > boundary::MAX_FILE_BYTES) {
        return frames
            .send(file_bytes_error(&request_id, &path, "too_large"))
            .await
            .map_err(|_| ());
    }
    let mut cursor = offset;
    let mut end: Option<u64> = None;
    let mut first: Option<hide_host::bytes::FileStamp> = None;
    let mut total: Option<u64> = None;
    loop {
        let wanted = end.map_or(hide_host::bytes::MAX_RANGE, |end| {
            end.saturating_sub(cursor).min(hide_host::bytes::MAX_RANGE)
        });
        let core = Arc::clone(&core);
        let (read_device, read_root, read_relative) =
            (device.to_owned(), root.clone(), relative.clone());
        let range = tokio::task::spawn_blocking(move || {
            let channel = core
                .node_link(&read_device)
                .map_err(|message| (None, message))?;
            hide_core::node_access::read_bytes(
                channel.as_ref(),
                &read_root,
                &read_relative,
                cursor,
                wanted,
            )
            .map_err(|error| (device_refusal(&error), error.to_string()))
        })
        .await
        .map_err(|error| (None, error.to_string()))
        .and_then(|result| result);
        let range = match range {
            Ok(range) => range,
            // The device judged the path as this Mac's boundary would: the
            // same refusal, whichever machine holds the file.
            Err((Some(refusal), _)) => {
                return frames
                    .send(Message::Text(
                        refused("file_bytes", &path, refusal).to_string().into(),
                    ))
                    .await
                    .map_err(|_| ());
            }
            Err((None, message)) => {
                hide_core::diagnostic!(json!({
                    "component": "hided",
                    "kind": "device.file_bytes_failed",
                    "device": device,
                    "message": message,
                }));
                return frames
                    .send(file_bytes_error(&request_id, &path, "read_failed"))
                    .await
                    .map_err(|_| ());
            }
        };
        let end_now = *end.get_or_insert_with(|| {
            let wanted = length.unwrap_or_else(|| range.total.saturating_sub(range.offset));
            range.offset.saturating_add(wanted).min(range.total)
        });
        if end_now - range.offset.min(end_now) > boundary::MAX_FILE_BYTES {
            return frames
                .send(file_bytes_error(&request_id, &path, "too_large"))
                .await
                .map_err(|_| ());
        }
        if first.get_or_insert_with(|| range.file.clone()) != &range.file {
            return frames
                .send(file_bytes_error(&request_id, &path, "read_failed"))
                .await
                .map_err(|_| ());
        }
        let expected_total = *total.get_or_insert(range.total);
        let Ok(bytes) = range.bytes() else {
            return frames
                .send(file_bytes_error(&request_id, &path, "read_failed"))
                .await
                .map_err(|_| ());
        };
        let asked = AskedRange {
            offset: cursor,
            length: wanted,
            end: end_now,
            total: expected_total,
        };
        let Some((taken, eof)) = asked.accept(range.offset, range.total, bytes.len()) else {
            return frames
                .send(file_bytes_error(&request_id, &path, "read_failed"))
                .await
                .map_err(|_| ());
        };
        let header = json!({
            "type": "file_bytes",
            "request_id": request_id,
            "path": path,
            "offset": range.offset,
            "total": range.total,
            "eof": eof,
        });
        frames
            .send(Message::Binary(
                bytes_frame(&header, &bytes[..taken]).into(),
            ))
            .await
            .map_err(|_| ())?;
        cursor = range.offset + taken as u64;
        if eof {
            return Ok(());
        }
    }
}

/// One range a device read asked its helper for.
struct AskedRange {
    offset: u64,
    length: u64,
    /// Where the whole read ends.
    end: u64,
    /// The file size the read started with.
    total: u64,
}

impl AskedRange {
    /// The helper's answer is untrusted input: it has to be the range asked
    /// for, of the same file size, no longer than asked, and short only where
    /// the read ends, or a misbehaving helper could stretch one read into
    /// millions of round trips or end it early as if the file were shorter.
    /// Returns how many of the answered bytes to send and whether they end
    /// the read; `None` fails the read.
    fn accept(&self, offset: u64, total: u64, length: usize) -> Option<(usize, bool)> {
        if offset != self.offset || total != self.total || length as u64 > self.length {
            return None;
        }
        let taken = length.min(self.end.saturating_sub(offset) as usize);
        let eof = offset + taken as u64 >= self.end;
        if !eof && (length as u64) < self.length {
            return None;
        }
        Some((taken, eof))
    }
}

/// Where the frames of a `file_bytes` read go: a client's socket, or the
/// channel a phone's read collects them from (`mobile::looks`), so one reader
/// of a file answers both and the phone's reads confine a path by the same
/// boundary as the desktop's.
pub(crate) trait FrameSink: Send {
    fn deliver(
        &mut self,
        message: Message,
    ) -> impl std::future::Future<Output = Result<(), ()>> + Send;
}

impl FrameSink for WebSocket {
    async fn deliver(&mut self, message: Message) -> Result<(), ()> {
        self.send(message).await.map_err(|_| ())
    }
}

impl FrameSink for tokio::sync::mpsc::Sender<Message> {
    async fn deliver(&mut self, message: Message) -> Result<(), ()> {
        self.send(message).await.map_err(|_| ())
    }
}

/// A `file_bytes` read keeps only one bounded chunk in memory. Sending each
/// frame before reading the next gives the socket backpressure even when all
/// eight authenticated clients ask for the maximum range together.
pub(crate) async fn send_file_bytes(
    socket: &mut impl FrameSink,
    boundary: &Boundary,
    roots: &crate::RootFollower,
    event: &Value,
) -> Result<(), ()> {
    let path = payload_str(event, "path");
    let opened = match boundary.open_file(&path) {
        Err(Refusal::OutsideCheckout) if roots_current(roots) => boundary.open_file(&path),
        opened => opened,
    };
    send_opened_file_bytes(socket, event, opened).await
}

/// A file a boundary opened for a read: its real path, the open file and its
/// size, or why it was refused.
pub(crate) type OpenedFile = Result<(PathBuf, std::fs::File, u64), Refusal>;

/// Answers a `file_bytes` event with the file `opened` gave for its path, or
/// with the refusal: the same frames whichever daemon read the file.
pub(crate) async fn send_opened_file_bytes(
    socket: &mut impl FrameSink,
    event: &Value,
    opened: OpenedFile,
) -> Result<(), ()> {
    let request_id = payload_str(event, "request_id");
    let path = payload_str(event, "path");
    let offset = event
        .pointer("/payload/offset")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let length = event.pointer("/payload/length").and_then(Value::as_u64);
    let (real, file, total) = match opened {
        Ok(source) => source,
        Err(refusal) => {
            return socket
                .deliver(Message::Text(
                    refused("file_bytes", &path, refusal).to_string().into(),
                ))
                .await;
        }
    };
    let wanted = length.unwrap_or_else(|| total.saturating_sub(offset.min(total)));
    if wanted > boundary::MAX_FILE_BYTES {
        return socket
            .deliver(file_bytes_error(&request_id, &path, "too_large"))
            .await;
    }
    let mut file = tokio::fs::File::from_std(file);
    let start = offset.min(total);
    if start > 0 && file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return socket
            .deliver(file_bytes_error(&request_id, &path, "read_failed"))
            .await;
    }
    let end = start.saturating_add(wanted).min(total);
    let displayed = hide_platform::path::to_wire_lossy(&real);
    let mut cursor = start;
    loop {
        let take = (end - cursor).min(BYTES_CHUNK) as usize;
        let mut buffer = vec![0u8; take];
        let read = match file.read(&mut buffer).await {
            Ok(read) => read,
            Err(_) => {
                return socket
                    .deliver(file_bytes_error(&request_id, &path, "read_failed"))
                    .await;
            }
        };
        buffer.truncate(read);
        // A file that shrank between the size read and this read answers 0
        // bytes; that ends the stream rather than spinning on it.
        let eof = read == 0 || cursor + read as u64 >= end;
        let header = json!({
            "type": "file_bytes",
            "request_id": request_id,
            "path": displayed,
            "offset": cursor,
            "total": total,
            "eof": eof,
        });
        socket
            .deliver(Message::Binary(bytes_frame(&header, &buffer).into()))
            .await?;
        cursor += read as u64;
        if eof {
            return Ok(());
        }
    }
}

/// The one place a client's path is checked before the core sees it (PRD S2
/// B10, S3 D-01 and B11). Two lines run here, one per flow, and every path
/// that passes is rewritten to the spelling that was checked.
///
/// The registration line reads `$HOME`: a `create_workspace` whose path does
/// not resolve under home, or is written outside it without the operator's
/// confirmation of that path (`confirmed_outside_home`), or belongs to a
/// project outside home that is not that folder (the frame's `confirm` names
/// the project's root), is answered with a `path_refused` frame and never
/// reaches the core. Create new project and
/// Clone from URL take no confirmation. A `remote_file_list` names a path on a remote machine,
/// which this boundary knows nothing about, and is forwarded as it came.
///
/// The Explorer line reads the registered checkout roots: every path an
/// explorer event carries is checked against the root the event named, and a
/// path outside it is refused as `outside_checkout`. `file_save` is the one
/// exception and is checked without being rewritten, because the core compares
/// the path it stored with the one it is handed.
///
/// `file_list` is the Explorer's listing: it names the root and the folder under
/// it, and the children are answered here as a `directory_list` frame, because
/// the core has no event for reading a directory and the Explorer shows files
/// the core's own listing never carries.
///
/// The shell's own attachment events are hided's to send (a web client stages
/// bytes through `attachment_*` instead), so a client that sends one is
/// answered with an error frame and it never reaches the core: those events
/// name arbitrary paths for the core to read. Every other kind passes
/// untouched to the core.
fn apply_boundary(boundary: &Boundary, event: &mut Value) -> Option<Value> {
    let kind = event.get("kind").and_then(Value::as_str)?.to_owned();
    match kind.as_str() {
        "create_workspace" if event.pointer("/payload/new_folder") == Some(&Value::Bool(true)) => {
            new_project(boundary, event, &kind)
        }
        "create_workspace" => register(boundary, event, &kind).map(|mut frame| {
            // An add sent by request id reads its refusal by that id, as it
            // reads the core's (`create_workspace.request_id`).
            if let (Some(id), Some(payload)) = (
                event.pointer("/payload/request_id").and_then(Value::as_str),
                frame.get_mut("payload").and_then(Value::as_object_mut),
            ) {
                payload.insert("request_id".to_owned(), Value::from(id));
            }
            frame
        }),
        "clone_repository" => clone_admit(boundary, event, &kind),
        "file_list" => explorer_listing(boundary, event, &kind),
        "file_open" | "reveal_path" => explorer_open(boundary, event, &kind),
        // A file Desk's desk opens is checked as an open is.
        "desk_view_open"
            if event.pointer("/payload/kind").and_then(Value::as_str) == Some("file") =>
        {
            explorer_open(boundary, event, &kind)
        }
        "file_save" => explorer_save(boundary, event, &kind),
        "file_create" | "dir_create" => explorer_create(boundary, event, &kind),
        "path_rename" => explorer_rename(boundary, event, &kind),
        "path_move" | "path_copy" => explorer_move(boundary, event, &kind),
        "path_trash" => explorer_trash(boundary, event, &kind),
        "browser_open" | "browser_state" => browser_url(boundary, event, &kind),
        "view_layout"
            if event.pointer("/payload/action").and_then(Value::as_str) == Some("navigate") =>
        {
            browser_url(boundary, event, &kind)
        }
        // The shell's attachment events and a thread's input name files the
        // operator's machine shows it; hided is their only producer for a
        // web client (which stages bytes instead), so a client that sends
        // one is naming an arbitrary path and never reaches the core.
        "terminal_attachment"
        | "terminal_attachment_ready"
        | "terminal_attachment_action"
        | "session_thread_input" => rejected(kind.as_str()),
        _ => None,
    }
}

/// Runs the boundary on a client event. The roots follow the core on their own
/// reads, so an event can name a checkout this client read in a snapshot the
/// boundary has not applied yet: after a launch, a checkout the core learns
/// from the engine's first sync reaches both on separate reads. A refusal as
/// outside every checkout therefore brings the roots current (`catch_up`) and
/// runs the check once more before it is answered. The first check's
/// `path.refused` line stays in the log, followed by `boundary.roots_caught_up`
/// when the second one let the event through.
fn admit_event(
    boundary: &Boundary,
    event: &mut Value,
    catch_up: impl FnOnce() -> bool,
) -> Option<Value> {
    let reply = apply_boundary(boundary, event);
    if !reply.as_ref().is_some_and(refused_outside_checkout) || !catch_up() {
        return reply;
    }
    let again = apply_boundary(boundary, event);
    if !again.as_ref().is_some_and(refused_outside_checkout) {
        hide_core::diagnostic!(json!({
            "component": "hided",
            "kind": "boundary.roots_caught_up",
            "event": event.get("kind").and_then(Value::as_str).unwrap_or(""),
        }));
    }
    again
}

/// Whether a boundary answer refused a path as outside every checkout.
fn refused_outside_checkout(frame: &Value) -> bool {
    frame.get("type").and_then(Value::as_str) == Some("path_refused")
        && frame.pointer("/payload/reason").and_then(Value::as_str)
            == Some(Refusal::OutsideCheckout.code())
}

/// Brings the roots current with the core for a check that found no root:
/// true once they are, whether this read or one it waited for applied them,
/// so the check is worth running again. False only when the core is gone.
pub(crate) fn roots_current(roots: &crate::RootFollower) -> bool {
    match roots.catch_up() {
        Ok(_) => true,
        Err(error) => {
            crate::roots_failed(&error);
            false
        }
    }
}

/// Whether `root` is a checkout the core's catalog carries for `device`,
/// bringing the roots current once before saying no (see `admit_event`).
pub(crate) fn device_root_known(
    boundary: &Boundary,
    roots: &crate::RootFollower,
    device: &str,
    root: &str,
) -> bool {
    boundary.is_device_root(device, root)
        || (roots_current(roots) && boundary.is_device_root(device, root))
}

/// A `key` event's target, a pane or a creation request (exactly one), and
/// its bytes.
/// A `key` event: the pane or creation it is typed into, and what was typed.
/// `paste: true` marks text the screen pasted, its line breaks carriage
/// returns, which the pane's holder wraps for the program's paste mode
/// (with-hide/hide issue 2952); a daemon before it reads the same event as keys.
pub(crate) fn terminal_key(event: &Value) -> Result<(KeyTarget, ScreenInput), String> {
    let field = |name: &str| {
        event
            .pointer(&format!("/payload/{name}"))
            .and_then(Value::as_str)
    };
    let named = |id: &str| !id.is_empty() && id.len() <= MAX_PANE_ID_BYTES;
    let target = match (field("pane_id"), field("pending_request")) {
        (Some(pane), None) if named(pane) => KeyTarget::Pane(pane.to_owned()),
        (None, Some(request)) if named(request) => KeyTarget::Request(request.to_owned()),
        _ => {
            return Err(format!(
                "key needs exactly one of pane_id and pending_request, of at most {MAX_PANE_ID_BYTES} bytes"
            ));
        }
    };
    let bytes = field("bytes_base64")
        .ok_or_else(|| "key.bytes_base64 must be a string".to_owned())
        .and_then(|data| {
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|error| format!("key.bytes_base64: {error}"))
        })?;
    let paste = match event.pointer("/payload/paste") {
        None => false,
        Some(paste) => paste
            .as_bool()
            .ok_or_else(|| "key.paste must be a boolean".to_owned())?,
    };
    Ok((
        target,
        if paste {
            ScreenInput::Paste(bytes)
        } else {
            ScreenInput::Keys(bytes)
        },
    ))
}

/// A `terminal_viewport` event: the pane, the grid its view draws at, and
/// whether the view has nothing drawn yet.
pub(crate) fn terminal_view(event: &Value) -> Result<(String, GridSize, bool), String> {
    let pane = payload_str(event, "pane_id");
    let dimension = |name: &str| {
        event
            .pointer(&format!("/payload/{name}"))
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| *value > 0)
    };
    let (Some(rows), Some(cols)) = (dimension("rows"), dimension("cols")) else {
        return Err("terminal_viewport needs positive rows and cols".to_owned());
    };
    if pane.is_empty() || pane.len() > MAX_PANE_ID_BYTES {
        return Err(format!(
            "terminal_viewport.pane_id must be a string of at most {MAX_PANE_ID_BYTES} bytes"
        ));
    }
    let new_view = event
        .pointer("/payload/new_view")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok((pane, GridSize { rows, cols }, new_view))
}

/// A `terminal_resize` event's pane and grid; the core refuses a grid that
/// is not positive, so one is passed on as it came.
fn terminal_grid(event: &Value) -> Result<(String, GridSize), String> {
    let pane = payload_str(event, "pane_id");
    let dimension = |name: &str| {
        event
            .pointer(&format!("/payload/{name}"))
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
    };
    match (dimension("rows"), dimension("cols")) {
        (Some(rows), Some(cols)) if !pane.is_empty() => Ok((pane, GridSize { rows, cols })),
        _ => Err("terminal_resize needs a pane_id, rows and cols".to_owned()),
    }
}

/// `connection` sent `pane` input: the pane takes that screen's grid when
/// it now follows it (`pane_sizes`).
fn screen_input(state: &AppState, connection: u64, pane: &str) {
    if pane.is_empty() {
        return;
    }
    if let Some(size) = state.pane_sizes.input(connection, pane) {
        send_grid(state, pane, size);
    }
}

/// Views and sizes `pane` at `size`, as the screen that now sizes it would:
/// its node expects frames at the new grid before the core's resize brings
/// them.
fn send_grid(state: &AppState, pane: &str, size: GridSize) {
    state.core.terminals.view(pane, size, false);
    let mut payload = json!({"pane_id": pane, "cols": size.cols, "rows": size.rows});
    // The measure the new screen took to fit this grid goes on with it.
    if let Some(floor) = state.pane_sizes.applied_floor(pane) {
        payload["floor"] = floor;
    }
    let event = json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "terminal_resize",
        "payload": payload,
    });
    if let Err(error) = state.core.dispatch(event.to_string().into_bytes()) {
        hide_core::diagnostic!(json!({
            "component": "pane_sizes",
            "kind": "pane_sizes.dispatch_failed",
            "pane_id": pane,
            "message": error,
        }));
    }
}

pub(crate) fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// What hided answers `event`, a window's ask that `close` ends a guest's
/// pane. The call a window makes now, `{request_id, call: {pane_id}}`, is
/// answered with the `guest_pane_close_result` frame under its
/// `request_id`: `ok`, or the code it was refused with. A page loaded
/// before this machine updated still sends `{pane_id}` alone, and keeps
/// the answer it was built for: nothing when the pane closed, the error
/// frame (the `Err`) when it did not.
fn guest_close_answer(
    event: &Value,
    close: impl FnOnce(&str) -> Result<(), &'static str>,
) -> Result<Option<Value>, &'static str> {
    if event.pointer("/payload/call").is_none()
        && let Some(pane) = event.pointer("/payload/pane_id").and_then(Value::as_str)
    {
        return close(pane).map(|()| None);
    }
    let request_id = payload_str(event, "request_id");
    let pane = event
        .pointer("/payload/call/pane_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let answer = if request_id.is_empty() || pane.is_empty() {
        Err("guest_close_invalid")
    } else {
        close(pane)
    };
    let payload = match answer {
        Ok(()) => json!({"request_id": request_id, "ok": true}),
        Err(code) => json!({"request_id": request_id, "ok": false, "code": code}),
    };
    Ok(Some(
        json!({"type": "guest_pane_close_result", "payload": payload}),
    ))
}

/// The path a client sent, as written; a field the event omits reads as empty
/// and is refused like any other empty path.
fn payload_str(event: &Value, field: &str) -> String {
    event
        .pointer(&format!("/payload/{field}"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

/// The frame a client-sent shell-only event is answered with: the event is
/// never forwarded, so a path it carries cannot reach the core.
fn rejected(kind: &str) -> Option<Value> {
    hide_core::diagnostic!(json!({
        "component": "hided",
        "kind": "client.rejected",
        "event": kind,
        "reason": "daemon_only_event",
    }));
    Some(json!({
        "type": "error",
        "payload": {},
        "message": format!("{kind} is sent by the daemon, not by a client"),
    }))
}

/// The frame a refused path is answered with, and the one log line it leaves.
fn refused(kind: &str, path: &str, refusal: Refusal) -> Value {
    log_path_refusal(kind, path, refusal);
    json!({
        "type": "path_refused",
        "payload": {"kind": kind, "path": path, "reason": refusal.code()},
    })
}

/// A `create_workspace` for an existing folder: its path is rewritten to the
/// one the boundary registers, or refused; a question about the project the
/// folder belongs to names that project's root as `confirm`, which the
/// operator confirms in place of the folder asked about.
fn register(boundary: &Boundary, event: &mut Value, kind: &str) -> Option<Value> {
    let raw = payload_str(event, "path");
    let confirmed = event
        .pointer("/payload/confirmed_outside_home")
        .and_then(Value::as_str)
        .map(str::to_owned);
    match boundary.resolve_registration(&raw, confirmed.as_deref()) {
        Ok(real) => {
            event["payload"]["path"] = Value::String(hide_platform::path::to_wire_lossy(&real));
            None
        }
        Err(Unregistrable::Refused(refusal)) => Some(refused(kind, &raw, refusal)),
        Err(Unregistrable::ConfirmProject(root)) => {
            let mut frame = refused(kind, &raw, Refusal::NeedsConfirmation);
            frame["payload"]["confirm"] = Value::String(hide_platform::path::to_wire_lossy(&root));
            Some(frame)
        }
        Err(Unregistrable::ConfirmHome) => {
            let mut frame = refused(kind, &raw, Refusal::NeedsConfirmation);
            frame["payload"]["home"] = Value::Bool(true);
            Some(frame)
        }
    }
}

/// Checks one path field and rewrites it to the spelling that was checked.
fn rewrite(
    event: &mut Value,
    kind: &str,
    field: &str,
    resolve: impl Fn(&str) -> Result<PathBuf, Refusal>,
) -> Option<Value> {
    let raw = payload_str(event, field);
    match resolve(&raw) {
        Ok(real) => {
            event["payload"][field] = Value::String(hide_platform::path::to_wire_lossy(&real));
            None
        }
        Err(refusal) => Some(refused(kind, &raw, refusal)),
    }
}

/// A clone's `parent` is checked on the `$HOME` line with the folder `name`
/// will create, and rewritten to the canonical spelling; a refusal names the
/// folder as `parent/name`.
fn clone_admit(boundary: &Boundary, event: &mut Value, kind: &str) -> Option<Value> {
    let parent = payload_str(event, "parent");
    let name = payload_str(event, "name");
    match boundary.resolve_clone_target(&parent, &name) {
        Ok(real) => {
            event["payload"]["parent"] = Value::String(hide_platform::path::to_wire_lossy(&real));
            None
        }
        Err(refusal) => Some(refused(
            kind,
            &format!("{}/{name}", parent.trim_end_matches('/')),
            refusal,
        )),
    }
}

/// Add a project asks whether a clone could land at `parent/name` while the
/// operator types, so Clone is offered only for a folder that is free. The
/// answer is this frame and nothing reaches the core; a refusal is not
/// logged, because it is a question, not an attempt.
fn clone_target(boundary: &Boundary, event: &Value) -> Value {
    let parent = payload_str(event, "parent");
    let name = payload_str(event, "name");
    let answer = boundary.resolve_clone_target(&parent, &name);
    json!({
        "type": "clone_target",
        "payload": {
            "device_id": boundary.node().as_str(),
            "parent": parent,
            "name": name,
            "path": answer.as_ref().ok().map(|real| hide_platform::path::to_wire_lossy(&real.join(&name))),
            "reason": answer.err().map(Refusal::code),
        },
    })
}

/// A `create_workspace` for Create new project names a folder that does not
/// exist yet, so its path is judged as a parent on the `$HOME` line and one
/// folder name, and forwarded as the canonical path the folder will take.
fn new_project(boundary: &Boundary, event: &mut Value, kind: &str) -> Option<Value> {
    rewrite(event, kind, "path", |raw| {
        let (parent, name) = raw.rsplit_once('/').ok_or(Refusal::InvalidPath)?;
        let parent = if parent.is_empty() { "/" } else { parent };
        boundary
            .resolve_new_project(parent, name)
            .map(|project| project.path)
    })
}

/// A `project_target` asks where Create new project would make `name` in
/// `parent`, before anything is sent: the dialog shows the folder and the
/// full path as the operator types, and whatever the create itself would be
/// refused for. It is answered on the same line the create is checked on,
/// never forwarded, and changes nothing on disk. An empty name asks about
/// the parent alone.
fn project_target(boundary: &Boundary, event: &Value) -> Value {
    let parent = payload_str(event, "parent");
    let name = payload_str(event, "name");
    let mut payload = json!({
        "device_id": boundary.node().as_str(), "parent": parent, "name": name,
        "parent_path": null, "parent_label": null, "path": null, "leftover": false, "reason": null,
    });
    let real_parent = match boundary.resolve_dir(&parent) {
        Ok(real) => real,
        Err(refusal) => {
            payload["reason"] = Value::String(refusal.code().to_owned());
            return json!({"type": "project_target", "payload": payload});
        }
    };
    payload["parent_path"] = Value::String(hide_platform::path::to_wire_lossy(&real_parent));
    payload["parent_label"] = Value::String(boundary.home_label(&real_parent));
    if !name.is_empty() {
        match boundary.resolve_new_project(&parent, &name) {
            Ok(project) => {
                payload["path"] = Value::String(hide_platform::path::to_wire_lossy(&project.path));
                payload["leftover"] = Value::Bool(project.leftover);
            }
            Err(refusal) => payload["reason"] = Value::String(refusal.code().to_owned()),
        }
    }
    json!({"type": "project_target", "payload": payload})
}

/// A `file_list` names the checkout root and the folder under it, and is
/// answered here: the Explorer reads folders lazily, so the frame carries one
/// folder's children and the client asks again as the operator expands.
fn explorer_listing(boundary: &Boundary, event: &mut Value, kind: &str) -> Option<Value> {
    let root = payload_str(event, "root");
    let Some(known) = boundary.known_root(&root) else {
        return Some(refused(kind, &root, Refusal::OutsideCheckout));
    };
    let raw = payload_str(event, "path");
    match boundary.list_children(&known, &raw) {
        Ok(listing) => Some(listing_frame(kind, listing)),
        Err(refusal) => Some(refused(kind, &raw, refusal)),
    }
}

/// The frame a listing is answered with. `kind` is the event that asked, so a
/// client routes the answer to the flow that requested it: the Explorer keeps
/// one listing per folder it has expanded.
fn listing_frame(kind: &str, listing: Listing) -> Value {
    let mut frame = json!({"type": "directory_list", "payload": listing});
    frame["payload"]["kind"] = Value::String(kind.to_owned());
    frame
}

/// An open or a reveal names the checkout the path belongs to, so the path is
/// checked against that checkout's root. A pair this daemon holds no root for
/// falls back to any root; the core answers the unknown checkout itself.
fn explorer_open(boundary: &Boundary, event: &mut Value, kind: &str) -> Option<Value> {
    let workspace_id = payload_str(event, "workspace_id");
    let checkout_id = payload_str(event, "checkout_id");
    rewrite(event, kind, "path", |raw| {
        boundary.resolve_checkout(&workspace_id, &checkout_id, raw)
    })
}

/// A browser display's address. A `file:` URL names a local file, so its
/// path has to be under a registered checkout like any path a client sends,
/// and it is written back as the URL of the path that was checked. Any other
/// address is the core's to judge.
fn browser_url(boundary: &Boundary, event: &mut Value, kind: &str) -> Option<Value> {
    let raw = payload_str(event, "url");
    if !hide_platform::path::is_file_url(&raw) {
        return None;
    }
    if !boundary.names_this_node(
        event
            .pointer("/payload/workspace/device_id")
            .and_then(Value::as_str),
    ) {
        // The core and the consented device host own this remote path. This
        // Mac's checkout boundary cannot resolve a path on that device.
        return None;
    }
    let Some((path, suffix)) = hide_platform::path::from_file_url(&raw) else {
        return Some(refused(kind, &raw, Refusal::InvalidPath));
    };
    let checked = if kind == "browser_state" {
        boundary.resolve_browser_report(&path)
    } else {
        boundary.resolve_target(&path)
    };
    match checked {
        Ok(real) => {
            let url = hide_platform::path::to_file_url(
                &hide_platform::path::to_wire_lossy(&real),
                suffix,
            );
            event["payload"]["url"] = Value::String(url);
            None
        }
        Err(refusal) => Some(refused(kind, &path, refusal)),
    }
}

/// A save carries a path the client already opened, and the core compares it
/// with the path it stored, so it is checked and left as it came.
fn explorer_save(boundary: &Boundary, event: &mut Value, kind: &str) -> Option<Value> {
    let raw = payload_str(event, "path");
    if boundary.is_under_root(&raw) {
        return None;
    }
    Some(refused(kind, &raw, Refusal::OutsideCheckout))
}

/// The root an explorer change names and one path under it: the root has to be
/// one of the registered ones, and both are rewritten to the spelling that was
/// checked.
fn explorer_rooted_path(
    boundary: &Boundary,
    event: &mut Value,
    kind: &str,
    field: &str,
) -> Option<Value> {
    let root = payload_str(event, "root");
    let Some(known) = boundary.known_root(&root) else {
        return Some(refused(kind, &root, Refusal::OutsideCheckout));
    };
    let raw = payload_str(event, field);
    let real = match boundary.resolve_below(&known, &raw) {
        Ok(real) => real,
        Err(refusal) => return Some(refused(kind, &raw, refusal)),
    };
    event["payload"]["root"] = Value::String(hide_platform::path::to_wire_lossy(&known));
    event["payload"][field] = Value::String(hide_platform::path::to_wire_lossy(&real));
    None
}

/// A creation names the folder it lands in and the name it takes.
fn explorer_create(boundary: &Boundary, event: &mut Value, kind: &str) -> Option<Value> {
    let name = payload_str(event, "name");
    if !boundary::valid_name(&name) {
        return Some(refused(kind, &name, Refusal::InvalidPath));
    }
    explorer_rooted_path(boundary, event, kind, "parent")
}

/// A rename names the item and the name it takes.
fn explorer_rename(boundary: &Boundary, event: &mut Value, kind: &str) -> Option<Value> {
    let name = payload_str(event, "name");
    if !boundary::valid_name(&name) {
        return Some(refused(kind, &name, Refusal::InvalidPath));
    }
    explorer_rooted_path(boundary, event, kind, "path")
}

/// A move names the item and the folder it lands in; both are under the root.
fn explorer_move(boundary: &Boundary, event: &mut Value, kind: &str) -> Option<Value> {
    if let Some(frame) = explorer_rooted_path(boundary, event, kind, "path") {
        return Some(frame);
    }
    let root = payload_str(event, "root");
    rewrite(event, kind, "destination", |raw| {
        boundary.resolve_in_root(&root, raw)
    })
}

/// A trash names the item and the row the tree selects once it is gone; both
/// are under the root, and the core still refuses a selection inside the item.
fn explorer_trash(boundary: &Boundary, event: &mut Value, kind: &str) -> Option<Value> {
    if let Some(frame) = explorer_rooted_path(boundary, event, kind, "path") {
        return Some(frame);
    }
    let root = payload_str(event, "root");
    rewrite(event, kind, "select_after", |raw| {
        boundary.resolve_in_root(&root, raw)
    })
}

/// Characters of a refused path the log keeps; the path is client input, so
/// the log line is capped rather than grown with it.
const LOGGED_PATH_CAP: usize = 256;

fn log_path_refusal(kind: &str, path: &str, refusal: Refusal) {
    let logged: String = path.chars().take(LOGGED_PATH_CAP).collect();
    hide_core::diagnostic!(json!({
        "component": "hided",
        "kind": "path.refused",
        "event": kind,
        "reason": refusal.code(),
        "path": logged,
        "path_truncated": logged.len() < path.len(),
    }));
}

/// Which frame a delta read turned into, decided by the cursors alone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameKind {
    /// Self-contained: the client replaces everything it holds.
    Snapshot,
    /// Applies on top of what the client already holds.
    Delta,
}

/// Classifies the frame a read from `have_revision` produced.
///
/// `None` means the core could not serve the cursor the client sent: the
/// client's revision is ahead of the core's (the daemon restarted), which
/// the core answers as a fresh reader. The client state is not one a delta
/// can be applied to, and the caller re-reads from zero and sends a
/// snapshot.
pub fn classify_frame(have_revision: u64, payload_revision: Option<u64>) -> Option<FrameKind> {
    if have_revision == 0 {
        return Some(FrameKind::Snapshot);
    }
    if payload_revision.is_some_and(|revision| revision < have_revision) {
        return None;
    }
    Some(FrameKind::Delta)
}

async fn read_delta(
    state: &AppState,
    have_revision: u64,
) -> Result<hide_core::SnapshotDeltaPayload, ()> {
    let core = Arc::clone(&state.core);
    tokio::task::spawn_blocking(move || core.snapshot(have_revision))
        .await
        .map_err(|error| log_snapshot_failure("reader", &error.to_string()))?
        .map_err(|error| log_snapshot_failure("core", &error))
}

/// A frame the daemon could not produce ends the client's loop, so the
/// client sees a bare close; this record is the only trace of why.
fn log_snapshot_failure(stage: &str, message: &str) {
    hide_core::diagnostic!(json!({
        "component": "hided",
        "kind": "ws.snapshot_failed",
        "stage": stage,
        "message": message,
    }));
}

async fn send_snapshot(
    socket: &mut WebSocket,
    state: &AppState,
    have_revision: &mut u64,
    granular: bool,
) -> Result<FrameKind, ()> {
    let mut payload = read_delta(state, *have_revision).await?;
    let kind = match classify_frame(*have_revision, Some(payload.revision)) {
        Some(kind) => kind,
        None => {
            payload = read_delta(state, 0).await?;
            FrameKind::Snapshot
        }
    };
    let revision = payload.revision;
    // One encoding, on a transport worker, after typed ownership has left
    // both Runtime and the dispatch owner thread.
    let envelope = tokio::task::spawn_blocking(move || {
        #[derive(serde::Serialize)]
        struct Envelope<'a> {
            #[serde(rename = "type")]
            kind: &'static str,
            payload: hide_core::SnapshotDeltaWire<'a>,
        }
        serde_json::to_string(&Envelope {
            kind: match kind {
                FrameKind::Snapshot => "snapshot",
                FrameKind::Delta => "delta",
            },
            payload: if granular {
                hide_core::SnapshotDeltaWire::granular(&payload)
            } else {
                hide_core::SnapshotDeltaWire::borrow(&payload)
            },
        })
    })
    .await
    .map_err(|error| log_snapshot_failure("encoder", &error.to_string()))?
    .map_err(|error| log_snapshot_failure("encode", &error.to_string()))?;
    socket
        .send(Message::Text(envelope.into()))
        .await
        .map_err(|_| ())?;
    *have_revision = revision;
    Ok(kind)
}

/// Counts a window in or out, and tells the core when the first arrives or
/// the last leaves (PRD labels-in-hided B29).
fn renderer_count(state: &AppState, arrived: bool) {
    let _transition = state
        .renderer_transitions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let previous = if arrived {
        state.renderers.fetch_add(1, Ordering::SeqCst)
    } else {
        state.renderers.fetch_sub(1, Ordering::SeqCst)
    };
    match (arrived, previous) {
        (true, 0) => renderers_drawn(state, true),
        (false, 1) => renderers_drawn(state, false),
        _ => {}
    }
}

/// Tells every owner of screen-only reads whether a renderer draws: the core
/// (`ui_attached`) and the Explorer watch.
pub(crate) fn renderers_drawn(state: &AppState, attached: bool) {
    state.watch.set_drawn(attached);
    dispatch_ui_attached(&state.core, attached);
}

/// Closes the session thread whose last window left without closing it.
fn close_session_thread(core: &CoreHandle, connection: u64) {
    let event = json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "session_thread_close",
        "payload": {},
    });
    let failed = core.dispatch(event.to_string().into_bytes()).err();
    hide_core::diagnostic!(
        json!({"component": "hided", "kind": "session_thread.closed_with_connection", "connection": connection, "failed": failed})
    );
}

/// Tells the core whether any window draws its snapshot.
fn dispatch_ui_attached(core: &CoreHandle, attached: bool) {
    let event = json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "ui_attached",
        "payload": { "attached": attached },
    });
    if let Err(error) = core.dispatch(event.to_string().into_bytes()) {
        hide_core::diagnostic!(
            json!({"component": "hided", "kind": "ui_attached.dispatch_failed", "attached": attached, "message": error})
        );
    }
}

/// A window of this machine, counted while it is open; the first to open
/// and the last to close tell every node whether one draws its panes.
/// One of this machine's `MAX_CLIENTS` places, held for a connection's life
/// and let go when it drops, on every path out of its loop.
pub(crate) struct LocalSeat(Arc<AtomicUsize>);

impl LocalSeat {
    /// A place, or the count it would have made when all are taken.
    pub(crate) fn take(state: &AppState) -> Result<Self, usize> {
        let previous = state.local_seats.fetch_add(1, Ordering::SeqCst);
        if previous >= MAX_CLIENTS {
            state.local_seats.fetch_sub(1, Ordering::SeqCst);
            return Err(previous + 1);
        }
        Ok(Self(Arc::clone(&state.local_seats)))
    }
}

impl Drop for LocalSeat {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct OwnScreen {
    count: Arc<Mutex<usize>>,
    terminals: Arc<hide_node::terminal::router::Router>,
}

impl OwnScreen {
    fn arrive(state: &AppState) -> Self {
        let mut count = state
            .own_screens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *count += 1;
        if *count == 1 {
            state.core.terminals.mirror(true);
        }
        Self {
            count: Arc::clone(&state.own_screens),
            terminals: Arc::clone(&state.core.terminals),
        }
    }
}

impl Drop for OwnScreen {
    fn drop(&mut self) {
        let mut count = self
            .count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.terminals.mirror(false);
        }
    }
}

/// One screen that draws this machine's browser pages, counted while it
/// lives.
struct PageRenderer(Arc<AtomicUsize>);

impl PageRenderer {
    fn arrive(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(count))
    }
}

impl Drop for PageRenderer {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A pane's query as what draws this machine's pages can answer it: with
/// no desktop window here no page command is offered, since only a window
/// gives one; a view of another machine that draws the pages
/// (`page_renderers`) still opens one and reports its state, so the states
/// are kept while one draws them and read as unsupported or disconnected
/// only while none does.
pub(crate) fn scope_page_query(
    result: &mut hide_core::workspace_control::QueryResult,
    desktop: bool,
    drawn: bool,
) {
    if !desktop {
        result.capabilities.retain(|capability| {
            !capability.starts_with("browser.")
                || (drawn && matches!(*capability, "browser.open" | "browser.status"))
        });
    }
    if !drawn && let Some(views) = &mut result.views {
        for view in views {
            if let Some(page) = &mut view.page {
                page.state = if page.state == "pending" {
                    "unsupported"
                } else {
                    "disconnected"
                };
                page.failure = None;
            }
        }
    }
}

/// What an agent is told when nothing on this machine or another one could
/// draw the page it opens.
const NO_PAGE_RENDERER: &str = "Open the Hide desktop app here, or show this computer's screen in Hide on another computer, and retry";

pub(crate) fn client_gone(state: &AppState, connection: u64, renderer: bool, desktop: bool) {
    if renderer {
        renderer_count(state, false);
    }
    if desktop {
        state.desktop_renderers.fetch_sub(1, Ordering::SeqCst);
    }
    state.attachments.release(connection);
    state.browser_comments.release(connection);
    state.mobile.release(connection);
    state.desktop_notices.release(connection);
    state.mobile.wake_follower();
    for (pane, size) in state.pane_sizes.left(connection) {
        send_grid(state, &pane, size);
    }
    for demand in [Demand::Settings, Demand::Start, Demand::RequestView] {
        demand.of(state).release(connection, |observing| {
            dispatch_observation(&state.core, demand, connection, observing)
        });
    }
    state.window_focus_demand.release(connection, |now| {
        dispatch_window_focus(&state.core, connection, now)
    });
    state.origin_holds.release(connection, |key| {
        close_backlog_origin(&state.core, connection, key)
    });
    state
        .thread_holds
        .release(connection, || close_session_thread(&state.core, connection));
    let remaining = state
        .clients
        .fetch_sub(1, Ordering::SeqCst)
        .saturating_sub(1);
    if remaining == 0 {
        *state.last_client_gone.lock().expect("client timestamp") = Instant::now();
    }
}

/// How long a refusal's closing handshake may take.
const REFUSE_LIMIT: Duration = Duration::from_secs(10);

/// Sends the refusal's close frame and reads what the client sent until its
/// own close or the end of its stream. A client may already have sent its
/// request behind the handshake; dropping the socket with that unread resets
/// the connection, and a reset can discard the close frame before the client
/// reads it, which Windows does (issue 785), so the client would read a lost
/// daemon instead of the refusal.
pub(crate) async fn refuse(socket: &mut WebSocket, reason: CloseReason, extra: Option<usize>) {
    log_refusal(reason, extra);
    // Bounded: a stalled tailnet socket must not hold this task open.
    let _ = tokio::time::timeout(REFUSE_LIMIT, async {
        let close = Message::Close(Some(CloseFrame {
            code: reason.code(),
            reason: reason.name().into(),
        }));
        if socket.send(close).await.is_err() {
            return;
        }
        while let Some(Ok(message)) = socket.recv().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
        }
    })
    .await;
}

fn log_refusal(reason: CloseReason, extra: Option<usize>) {
    hide_core::diagnostic!(json!({
        "component": "hided",
        "kind": "ws.refused",
        "reason": reason.name(),
        "code": reason.code(),
        "clients": extra,
    }));
}

/// Mounts the core's routes on `seat` and starts the clock that ends the
/// daemon once no screen is left; the role aborts the clock when it ends.
pub fn mount(seat: &crate::seat::SeatParts, state: AppState) -> tokio::task::JoinHandle<()> {
    let shutdown = state.shutdown.clone();
    let mobile = Arc::clone(&state.mobile);
    let core = Arc::clone(&state.core);
    let idle_task = watch_idle(
        Idle {
            idle_secs: state.idle_secs,
            keep_alive: state.keep_alive,
            clients: Arc::clone(&state.clients),
            last_client_gone: Arc::clone(&state.last_client_gone),
            shutdown,
            // The core's own engine is this process's, so it answers while
            // the daemon runs and keeps nothing alive; its panes do (PRD
            // engine-bundle-3-core-owns D-10).
            live_work: {
                let holders = Arc::clone(state.core.pane_holders());
                live_work(
                    move || holders.live_panes(),
                    state.core.state_dir().to_path_buf(),
                    crate::always_on::PairedProbe::default(),
                )
            },
        },
        // Mobile on with a phone paired: the phone may come back at any
        // time and push has to keep going (PRD D-10), and an automation on
        // has to run at its time (PRD automations D-19). The idle clock
        // starts over when both end.
        move || mobile.keep_alive() || core.automations_on(),
    );
    seat.mount(router(state));
    idle_task
}

/// What keeps an idle daemon: its holder panes that may still run
/// (`live_panes`) and one for a pairing with another machine, which `probe`
/// answers from the state folder's files only when they changed.
fn live_work(
    live_panes: impl Fn() -> Result<usize, String> + Send + Sync + 'static,
    state_dir: PathBuf,
    probe: crate::always_on::PairedProbe,
) -> Arc<dyn Fn() -> Result<usize, String> + Send + Sync> {
    let probe = Mutex::new(probe);
    Arc::new(move || {
        let paired = probe.lock().expect("paired probe").paired(&state_dir)?;
        Ok(live_panes()? + usize::from(paired))
    })
}

/// What decides when a daemon with no screen ends.
pub(crate) struct Idle {
    pub idle_secs: u64,
    pub keep_alive: bool,
    pub clients: Arc<AtomicUsize>,
    pub last_client_gone: Arc<Mutex<Instant>>,
    pub shutdown: crate::ending::Ending,
    /// How many things that keep the daemon may still run: this machine's
    /// holder panes (`hide_kit::panes::Panes::live`), and for a core, its
    /// pairing with another machine (`always_on::paired`), whose peers reach
    /// it with no window open (PRD device-connect-iroh D-41).
    pub live_work: Arc<dyn Fn() -> Result<usize, String> + Send + Sync>,
}

/// Ends the daemon once it had no screen for `idle_secs` while `held` was
/// false and no pane of this machine's holders could still run; never with
/// `keep_alive`. Aborted by its
/// owner when the daemon stops another way.
pub(crate) fn watch_idle(
    idle: Idle,
    held: impl Fn() -> bool + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if idle.keep_alive {
            return;
        }
        let mut panes_probed: Option<Instant> = None;
        let mut panes_live = false;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if idle.clients.load(Ordering::SeqCst) != 0 {
                continue;
            }
            if held() {
                *idle.last_client_gone.lock().expect("client timestamp") = Instant::now();
                continue;
            }
            // The core's own engine's panes run in holders that outlive the
            // daemon, and the daemon is what keeps their letters and states
            // going, so it lives while one may still run. A record that
            // cannot be read is counted as running (`Panes::live`), and a
            // failed count keeps the daemon: ending it would leave panes
            // with no core.
            if panes_probed.is_none_or(|at| at.elapsed() >= PANES_PROBE_INTERVAL) {
                panes_probed = Some(Instant::now());
                let live = Arc::clone(&idle.live_work);
                panes_live = tokio::task::spawn_blocking(move || panes_running(&*live))
                    .await
                    .unwrap_or(true);
            }
            if panes_live {
                *idle.last_client_gone.lock().expect("client timestamp") = Instant::now();
                continue;
            }
            let gone = *idle.last_client_gone.lock().expect("client timestamp");
            if gone.elapsed() >= Duration::from_secs(idle.idle_secs) {
                idle.shutdown.end();
                return;
            }
        }
    })
}

/// How often an idle daemon counts its holder panes.
const PANES_PROBE_INTERVAL: Duration = Duration::from_secs(5);

/// Whether a holder pane of this machine may still run.
fn panes_running(live: &(dyn Fn() -> Result<usize, String> + Send + Sync)) -> bool {
    match live() {
        Ok(count) => count > 0,
        Err(message) => {
            hide_core::diagnostic!(json!({
                "component": "hided",
                "kind": "idle.panes_uncounted",
                "message": message,
            }));
            true
        }
    }
}

pub async fn bind(addr: SocketAddr) -> Result<tokio::net::TcpListener, String> {
    tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| format!("bind {addr} failed: {error}"))
}

pub(crate) fn mime_for(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        // Vite emits a lazy-loaded ES module (the pdf.js worker among them) as
        // `.mjs`; a module script is refused unless its type is JavaScript.
        Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("webmanifest") => "application/manifest+json",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

pub fn allowed_origins(port: u16, vite: Option<&str>) -> HashSet<String> {
    let mut set = HashSet::new();
    set.insert(format!("http://127.0.0.1:{port}"));
    set.insert(format!("http://localhost:{port}"));
    if let Some(vite) = vite {
        set.insert(vite.trim_end_matches('/').to_owned());
    }
    set
}

#[cfg(test)]
mod tests {
    /// A pane's query result with one page loaded and one pending.
    fn page_query() -> hide_core::workspace_control::QueryResult {
        use hide_core::workspace_control::{BrowserPage, Context, QueryResult, View};
        let view = |id: &str, state: &'static str| View {
            area_id: "a1".to_owned(),
            view_id: id.to_owned(),
            kind: "browser",
            target: format!("https://{id}.example/"),
            selected: false,
            active_area: false,
            page: Some(BrowserPage {
                load: 1,
                state,
                failure: Some("refused".to_owned()),
            }),
            openers: Vec::new(),
            operator: false,
            unseen: false,
        };
        QueryResult {
            context: Context {
                device_id: "mini".to_owned(),
                workspace_id: "w".to_owned(),
                checkout_id: "c".to_owned(),
                checkout_path: "/web".to_owned(),
            },
            capabilities: vec![
                "view.list",
                "browser.open",
                "browser.status",
                "browser.snapshot",
                "browser.click",
            ],
            views: Some(vec![view("d1", "loaded"), view("d2", "pending")]),
        }
    }

    // With no desktop window here but another machine's view drawing this
    // machine's pages, an agent may open one and read its state, never drive
    // it; with neither, nothing of a page is offered and its state reads as
    // nothing draws it.
    #[test]
    fn a_view_that_draws_pages_keeps_open_and_state_and_drops_the_page_commands() {
        let mut drawn = page_query();
        scope_page_query(&mut drawn, false, true);
        assert_eq!(
            drawn.capabilities,
            ["view.list", "browser.open", "browser.status"]
        );
        let pages: Vec<_> = drawn
            .views
            .unwrap()
            .into_iter()
            .map(|view| view.page.unwrap())
            .collect();
        assert_eq!((pages[0].state, pages[1].state), ("loaded", "pending"));
        assert_eq!(pages[0].failure.as_deref(), Some("refused"));

        let mut none = page_query();
        scope_page_query(&mut none, false, false);
        assert_eq!(none.capabilities, ["view.list"]);
        let pages: Vec<_> = none
            .views
            .unwrap()
            .into_iter()
            .map(|view| view.page.unwrap())
            .collect();
        assert_eq!(
            (pages[0].state, pages[1].state),
            ("disconnected", "unsupported")
        );
        assert!(pages.iter().all(|page| page.failure.is_none()));

        let mut window = page_query();
        scope_page_query(&mut window, true, true);
        assert_eq!(window, page_query());
    }

    /// Desk is judged at the command boundary as a bot of the whole Hide is:
    /// it reads, and what is the operator's alone or means an agent in a pane
    /// is refused with the table's codes. A capability that is no bot's or
    /// Desk's has no mode and is judged by the core.
    #[test]
    fn desk_is_judged_at_the_command_boundary_as_a_bot_of_the_whole_hide() {
        use super::{ScopedRequest, admit_command};
        let directory = tempfile::tempdir().unwrap();
        let registry = crate::pane_auth::Registry::new(directory.path()).unwrap();
        let path = registry
            .issue_desk(
                "0123456789abcdef0123456789abcdef",
                1,
                hide_core::desk::desk_context("local"),
            )
            .unwrap();
        let pid = std::process::id() as i32;
        registry
            .bind_desk(
                &path,
                pid,
                hide_node::pane_proof::process_start(pid).unwrap(),
            )
            .unwrap();
        let token = serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap())
            .unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned();
        let desk = registry.get(&token).unwrap();

        let reads =
            ScopedRequest::Session(hide_core::links::session_command::SessionCommand::List {
                since_ms: None,
                limit: None,
            });
        assert_eq!(admit_command(&desk, &reads), Ok(()));
        let adds = ScopedRequest::Device(hide_core::device_command::DeviceCommand::Add {
            ssh_alias: "box".into(),
            name: None,
        });
        assert_eq!(
            admit_command(&desk, &adds).unwrap_err().0,
            "access_operator_only"
        );
        let memory = ScopedRequest::Memory(hide_core::memory_command::MemoryCommand::Search {
            words: "x".into(),
        });
        assert_eq!(
            admit_command(&desk, &memory).unwrap_err().0,
            "access_needs_agent"
        );
    }

    /// This machine's eight places are taken in one place, `LocalSeat`
    /// (PRD device-connect-iroh D-67): a way in that compared a count of
    /// its own with `MAX_CLIENTS`, as the phone's, a pane's command and
    /// `hide <path>` once did with every connection, would let another
    /// machine's views take this machine's places. A node daemon serves no
    /// views and keeps its own count.
    #[test]
    fn only_a_local_seat_reads_the_client_limit() {
        fn sources(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    sources(&path, found);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    found.push(path);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        sources(&root, &mut files);
        // Spelled apart so this test's own lines are not among them.
        let limit = ["MAX_", "CLIENTS"].concat();
        let mut readers = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for line in text.lines() {
                if line.contains(&limit) && line.contains(">=") {
                    let name = file
                        .strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    readers.push(name);
                }
            }
        }
        readers.sort();
        assert_eq!(
            readers,
            ["node_daemon/screen.rs", "server.rs"],
            "take a LocalSeat instead"
        );
    }

    /// PRD lane-desk-pair-view B32: every page on no desk takes an agent's
    /// input, a desk page behind another tab of its area does not, and a
    /// display outside the caller's checkout, whose Views the list never
    /// holds, is missing rather than driven.
    #[test]
    fn a_page_on_no_desk_takes_input_and_another_checkouts_page_is_missing() {
        let view = |area: &str, id: &str, selected: bool| hide_core::workspace_control::View {
            area_id: area.into(),
            view_id: id.into(),
            kind: "browser",
            target: format!("https://{id}.test/"),
            selected,
            active_area: false,
            page: None,
            openers: Vec::new(),
            operator: false,
            unseen: true,
        };
        let shelf = hide_core::workspace_views::SHELF_AREA;
        let views = vec![
            view(shelf, "front-of-shelf", true),
            view(shelf, "behind-on-shelf", false),
            view("a1", "on-desk", true),
            view("a1", "behind-on-desk", false),
        ];
        let takes_input =
            |id: &str| super::relay_facts(&views, id).map(|facts| facts["selected"] == true);
        assert_eq!(takes_input("front-of-shelf"), Some(true));
        assert_eq!(takes_input("behind-on-shelf"), Some(true));
        assert_eq!(takes_input("on-desk"), Some(true));
        assert_eq!(takes_input("behind-on-desk"), Some(false));
        assert_eq!(takes_input("another-checkouts-page"), None);
    }

    /// A daemon with no screen and `live` holder panes, ending after one
    /// idle second.
    fn idle(live: Result<usize, String>) -> crate::ending::Ending {
        let shutdown = crate::ending::Ending::new();
        super::watch_idle(
            super::Idle {
                idle_secs: 1,
                keep_alive: false,
                clients: Arc::new(AtomicUsize::new(0)),
                last_client_gone: Arc::new(Mutex::new(Instant::now())),
                shutdown: shutdown.clone(),
                live_work: Arc::new(move || live.clone()),
            },
            || false,
        );
        shutdown
    }

    #[tokio::test]
    async fn a_daemon_outlives_the_idle_window_while_a_holder_pane_may_run() {
        let running = idle(Ok(1));
        let uncounted = idle(Err("the engine folder was unreadable".to_owned()));
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(!running.has_ended());
        assert!(!uncounted.has_ended(), "a count that failed keeps it alive");
    }

    /// What keeps an idle daemon is its panes plus one for a pairing, and
    /// the pairing is asked of the folder's files only when they changed.
    #[test]
    fn the_idle_probe_counts_panes_and_a_pairing_and_reads_the_folder_once() {
        use std::sync::atomic::AtomicUsize;
        static READS: AtomicUsize = AtomicUsize::new(0);
        let state = tempfile::tempdir().unwrap();
        let work = super::live_work(
            || Ok(2),
            state.path().to_path_buf(),
            crate::always_on::PairedProbe::reading(|dir| {
                READS.fetch_add(1, Ordering::SeqCst);
                crate::always_on::paired(dir)
            }),
        );
        for _ in 0..3 {
            assert_eq!(work(), Ok(2));
        }
        assert_eq!(
            READS.load(Ordering::SeqCst),
            1,
            "an unpaired folder is read once"
        );

        let guest = hide_peer::protocol::PeerKey::from_bytes([7; 32]);
        let now = 1_800_000_000_000;
        let mut grants =
            hide_peer::store::Grants::open(hide_kit::layout::peer_grants(state.path())).unwrap();
        let grant = grants
            .grant_pending(guest, "d1", "Laptop", hide_peer::store::Via::Link, now)
            .unwrap();
        grants.confirm(&guest, &grant.id, now).unwrap();
        for _ in 0..3 {
            assert_eq!(work(), Ok(3), "a confirmed pairing counts one more");
        }
        assert_eq!(
            READS.load(Ordering::SeqCst),
            2,
            "a new pairing is read once"
        );
    }

    #[tokio::test]
    async fn a_daemon_with_no_screen_and_no_pane_ends_after_the_idle_window() {
        let ended = idle(Ok(0));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ended.has_ended() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("the daemon ends");
    }

    #[test]
    fn a_screens_feedback_keeps_its_order_and_holds_at_most_four_screenshots() {
        let (queue, mut jobs) = super::FeedbackQueue::new();
        let job = |kind: &str, shot: bool| super::FeedbackJob {
            event: json!({"kind": kind}),
            png: shot.then(Vec::new),
        };
        for _ in 0..super::FEEDBACK_SHOTS {
            assert!(queue.push(job("add", true)).is_ok());
        }
        assert!(
            queue.push(job("add", true)).is_err(),
            "a fifth screenshot waits for none"
        );
        // A burst of relocates behind them still fits.
        for _ in 0..50 {
            assert!(queue.push(job("relocate", false)).is_ok());
        }
        assert!(queue.push(job("send", false)).is_ok());
        let kinds: Vec<String> = std::iter::from_fn(|| jobs.try_recv().ok())
            .map(|job| job.event["kind"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(kinds.len(), super::FEEDBACK_SHOTS + 51);
        assert_eq!(
            kinds.last().map(String::as_str),
            Some("send"),
            "the Send comes after every comment"
        );
        for _ in 0..super::FEEDBACK_QUEUE {
            assert!(queue.push(job("relocate", false)).is_ok());
        }
        assert!(
            queue.push(job("relocate", false)).is_err(),
            "a full queue refuses"
        );
    }

    use super::*;

    /// A Desk message's files reach the core only from hided's own commit:
    /// a path a window wrote into `files` is dropped, and the words,
    /// mentions and `다시 시도`'s message go on as sent (PRD desk-2-ui B41).
    #[test]
    fn a_desk_send_carries_only_the_files_hided_committed() {
        let event = json!({"kind": "desk_send", "payload": {
            "text": "look", "mentions": [{"kind": "session", "id": "p1", "name": "a"}],
            "files": [{"name": "passwd", "bytes": 1, "path": "/etc/passwd"}], "again": 4,
        }});
        assert_eq!(
            desk_send_for_core(&event, &[]),
            json!({"schema_version": 2, "kind": "desk_send", "payload": {
                "text": "look", "mentions": [{"kind": "session", "id": "p1", "name": "a"}],
                "files": [], "again": 4,
            }})
        );
        let staged = attachments::CommittedFile {
            name: "shot.png".to_owned(),
            bytes: 3,
            path: "/state/attachments/r-0-shot.png".to_owned(),
        };
        assert_eq!(
            desk_send_for_core(&event, &[staged])["payload"]["files"],
            json!([{"name": "shot.png", "bytes": 3, "path": "/state/attachments/r-0-shot.png"}])
        );
    }

    /// A window's close of a guest's pane is answered in the shape it was
    /// asked in: the call a page makes now hears its result frame, and a
    /// page loaded before the update, sending `{pane_id}` alone, closes the
    /// pane as it did and hears an error frame only when it did not (B10).
    #[test]
    fn a_guest_pane_close_is_answered_in_the_shape_it_was_asked() {
        let gone = |_: &str| Err("pane_unavailable");
        let call = json!({"kind": "guest_pane_close", "payload": {"request_id": "c1", "call": {"pane_id": "guest:mb:pane:p1"}}});
        let mut closed = Vec::new();
        assert_eq!(
            guest_close_answer(&call, |pane| {
                closed.push(pane.to_owned());
                Ok(())
            }),
            Ok(Some(
                json!({"type": "guest_pane_close_result", "payload": {"request_id": "c1", "ok": true}})
            ))
        );
        assert_eq!(
            guest_close_answer(&call, gone),
            Ok(Some(
                json!({"type": "guest_pane_close_result", "payload": {"request_id": "c1", "ok": false, "code": "pane_unavailable"}})
            ))
        );
        let earlier =
            json!({"kind": "guest_pane_close", "payload": {"pane_id": "guest:mb:pane:p2"}});
        assert_eq!(
            guest_close_answer(&earlier, |pane| {
                closed.push(pane.to_owned());
                Ok(())
            }),
            Ok(None)
        );
        assert_eq!(guest_close_answer(&earlier, gone), Err("pane_unavailable"));
        assert_eq!(closed, ["guest:mb:pane:p1", "guest:mb:pane:p2"]);
        let neither = json!({"kind": "guest_pane_close", "payload": {"request_id": "c2"}});
        assert_eq!(
            guest_close_answer(&neither, |_| unreachable!("no pane is named")),
            Ok(Some(
                json!({"type": "guest_pane_close_result", "payload": {"request_id": "c2", "ok": false, "code": "guest_close_invalid"}})
            ))
        );
    }

    /// A pane id a screen sends straight to a node is refused past the
    /// longest a device pane's id can be, so a client cannot fill a node's
    /// capped pane entries with ids of any length.
    #[test]
    fn a_key_or_view_naming_a_pane_id_past_its_length_is_refused() {
        let longest = format!("remote:{}:pane:w1:p1", "d".repeat(256));
        let too_long = format!("w1:{}", "p".repeat(600));
        let key = |pane: &str, field: &str| {
            terminal_key(&json!({"type": "key", "payload": {field: pane, "bytes_base64": "eA=="}}))
        };
        let view = |pane: &str| {
            terminal_view(&json!({"type": "terminal_viewport", "payload": {
                "pane_id": pane, "rows": 24, "cols": 80,
            }}))
        };
        assert!(key(&longest, "pane_id").is_ok());
        assert!(view(&longest).is_ok());
        assert!(key(&too_long, "pane_id").is_err());
        assert!(key(&too_long, "pending_request").is_err());
        assert!(view(&too_long).is_err());
    }

    /// with-hide/hide issue 2952: a screen marks the text it pasted, which goes on
    /// as a paste for the pane's holder to wrap; an unmarked key, or one a
    /// page built before the mark sends, is keys, and a mark that is no
    /// boolean is refused rather than guessed.
    #[test]
    fn a_key_marked_paste_is_a_paste_and_an_unmarked_one_is_keys() {
        let read = |payload: Value| terminal_key(&json!({"type": "key", "payload": payload}));
        assert_eq!(
            read(json!({"pane_id": "w1:p1", "bytes_base64": "YQ1i", "paste": true})),
            Ok((
                KeyTarget::Pane("w1:p1".into()),
                ScreenInput::Paste(b"a\rb".to_vec())
            ))
        );
        for payload in [
            json!({"pane_id": "w1:p1", "bytes_base64": "YQ1i"}),
            json!({"pane_id": "w1:p1", "bytes_base64": "YQ1i", "paste": false}),
        ] {
            assert_eq!(
                read(payload),
                Ok((
                    KeyTarget::Pane("w1:p1".into()),
                    ScreenInput::Keys(b"a\rb".to_vec())
                ))
            );
        }
        assert!(read(json!({"pane_id": "w1:p1", "bytes_base64": "YQ1i", "paste": "yes"})).is_err());
    }

    /// A spawn or a pane send that stopped partway answers its code and its
    /// own next step, with what it left (the spawn and pane, or what landed)
    /// as a field the caller reads, not a sentence.
    #[test]
    fn work_that_stopped_partway_names_what_it_left_as_a_field() {
        let reason =
            "native_identity_unavailable\u{1f}{\"id\":\"spawn-4\",\"pane\":\"w3:p2\"}".to_owned();
        assert_eq!(
            refusal_field(&reason),
            Some(("spawn", json!({"id": "spawn-4", "pane": "w3:p2"})))
        );
        let next = delivery_next_action("native_identity_unavailable");
        assert_eq!(
            refusal_answer(reason, next),
            ("native_identity_unavailable".to_owned(), next.to_owned())
        );
        assert_eq!(refusal_field("native_identity_unavailable"), None);
        assert_eq!(refusal_field("machine_unknown\u{1f}mini"), None);
        let sent = r#"{"text_sent":true,"keys_sent":[],"enter_sent":false}"#;
        let partial = format!("operator_typing\u{1f}{sent}");
        assert_eq!(
            refusal_field(&partial),
            Some(("sent", serde_json::from_str(sent).unwrap()))
        );
        let next = hide_core::pane_command::next_action("operator_typing").unwrap();
        assert_eq!(
            refusal_answer(partial, next),
            ("operator_typing".to_owned(), next.to_owned())
        );
    }

    /// A spawn under a mission node held or behind its start gate says what
    /// the mission owes, not to retry the same spawn (PRD node-presets D-06).
    #[test]
    fn a_spawn_the_mission_refuses_says_what_the_mission_owes() {
        for code in ["mission_node_held", "mission_gate_before_start"] {
            let next = delivery_next_action(&format!("{code}\u{1f}ship"));
            assert_eq!(next, hide_core::missions::next_action(code));
            assert!(!next.contains("retry the same intent"), "{next}");
        }
    }

    /// A refusal that lists the devices the caller could have named answers
    /// with that list; one without a list keeps the static advice.
    #[test]
    fn a_device_refusal_names_what_the_caller_can_do() {
        let reason = "machine_unknown\u{1f}mini, studio".to_owned();
        let (code, next) = refusal_answer(reason, delivery_next_action("machine_unknown"));
        assert_eq!(code, "machine_unknown");
        assert!(next.ends_with("--machine: mini, studio"), "{next}");
        let (_, none) = refusal_answer(
            "machine_unknown\u{1f}".to_owned(),
            delivery_next_action("machine_unknown"),
        );
        assert!(none.contains("No other device is connected"), "{none}");
        let (code, next) = refusal_answer(
            "shell_starting\u{1f}30\u{1f}[oh-my-zsh] Would you like to update? [Y/n]".to_owned(),
            delivery_next_action("shell_starting"),
        );
        assert_eq!(code, "shell_starting");
        assert!(
            next.ends_with(
                "The shell was waited for 30 s. Its screen ends with: [oh-my-zsh] Would you like to update? [Y/n]"
            ),
            "{next}"
        );
        // A shell that showed nothing still says how long it was waited for.
        let (_, blank) = refusal_answer(
            "shell_starting\u{1f}30\u{1f}".to_owned(),
            delivery_next_action("shell_starting"),
        );
        assert!(blank.ends_with("The shell was waited for 30 s"), "{blank}");
        for code in [
            "spawn_busy",
            "workspace_closing",
            "machine_not_permitted",
            "recipient_not_agent",
            "sender_not_registered",
            "machine_unavailable",
            "repository_unavailable",
            "spawn_repo_required",
            "spawn_project_loading",
            "spawn_folder_unreadable",
            "agent_not_installed",
            "intent_conflict",
            "agent_not_started",
            "worktree_unconfirmed",
            "shell_starting",
            "pane_busy",
            "native_identity_unavailable",
            "identity_waits_full",
            "session_too_long",
            "target_ambiguous",
            "target_unavailable",
            "no_parent",
            "participant_registering",
            "codex_capability_unknown",
            "agent_hook_off",
            "agent_not_set_up",
            "agent_hook_untrusted",
            "agent_hook_missing",
            "agent_hook_unconfirmed",
            "agent_hook_unchecked",
            "beside_other_checkout",
            "beside_other_machine",
            "beside_needs_agent_pane",
            "beside_too_narrow",
            "beside_unmeasured",
            "beside_tab_busy",
            "beside_with_parent",
            "beside_pane_unavailable",
        ] {
            let (reason, next) = refusal_answer(code.to_owned(), delivery_next_action(code));
            assert_eq!(reason, code);
            assert_ne!(next, delivery_next_action("other"), "{code}");
        }
    }

    /// A plan's refusal answers as the plan words it, with the step, field or
    /// evidence it names, from `hide task` and from a spawn or a letter
    /// alike, never as a device list (PRD hide-plan-delegate B5, B10, B24).
    #[test]
    fn a_plan_refusal_answers_the_plans_next_action_and_what_it_names() {
        for (reason, words) in [
            ("plan_evidence_missing\u{1f}pr --pr", "--pr <number>"),
            ("plan_pr_not_yours\u{1f}pr #12", "name the pull request"),
            ("plan_pr_unconfirmed\u{1f}pr #12", "ask GitHub"),
            (
                "plan_file_outside\u{1f}spec ../spec.md",
                "inside this session's checkout",
            ),
            ("plan_file_missing\u{1f}spec docs/spec.md", "write it"),
            (
                "plan_file_unconfirmed\u{1f}spec docs/spec.md",
                "run the same command again",
            ),
            ("plan_report_missing\u{1f}web", "report letter"),
            (
                "plan_order_refused\u{1f}web waits on spec",
                "Finish or drop",
            ),
            ("plan_node_finished\u{1f}web", "already done or dropped"),
            ("plan_node_unknown\u{1f}nope", "hide task show"),
            ("plan_delegate_full\u{1f}3", "do this step yourself"),
        ] {
            let (code, detail) = hide_core::coordination::split_refusal(reason);
            let detail = detail.unwrap();
            for next in [
                hide_core::plans::next_action(code),
                delivery_next_action(reason),
            ] {
                let (answered, text) = refusal_answer(reason.to_owned(), next);
                assert_eq!(answered, code);
                assert!(text.contains(words), "{code}: {text}");
                assert!(text.ends_with(&format!("({detail})")), "{code}: {text}");
                assert!(!text.contains("--machine"), "{code}: {text}");
            }
        }
        // A spawn's refusals that carry nothing more read as the plan's.
        for code in [
            "plan_missing",
            "plan_review_pending",
            "plan_delegate_needs_parent",
            "plan_delegate_not_permitted",
            "plan_unavailable",
            "plan_session_unknown",
        ] {
            let (answered, text) = refusal_answer(code.to_owned(), delivery_next_action(code));
            assert_eq!(answered, code);
            assert_eq!(text, hide_core::plans::next_action(code));
            assert_ne!(text, hide_core::plans::next_action("other"), "{code}");
        }
    }

    /// A caller refused for lacking an agent pane is told where to run the
    /// command, and one that named no agent how agents are named; every
    /// other delivery refusal keeps the retry advice.
    #[test]
    fn a_missing_agent_pane_names_where_to_run_the_command() {
        let next = delivery_next_action("agent_pane_required");
        assert!(next.contains("pane where the agent is running"), "{next}");
        assert!(next.contains("--no-daemon"), "{next}");
        // Asking again about a key that names no agent answers the same
        // (with-hide/hide#2299).
        assert_eq!(
            delivery_next_action("agent_unavailable"),
            "Hide knows no agent by that: take its id from hide agent list (agent-N); hide agent show also takes a spawn's id (spawn-N), the agent's name or its pane"
        );
        assert_eq!(
            delivery_next_action("caller_identity_conflict"),
            "Check the current agent pane and retry the same intent"
        );
    }

    /// with-hide/hide#2634: a spawn refused for its agent name says the name
    /// is the problem and the limit it broke, so the caller shortens the name
    /// instead of retrying the same one.
    #[test]
    fn a_spawn_refused_for_its_name_names_the_name_and_its_limit() {
        let next = delivery_next_action("invalid_agent_name");
        assert!(next.contains("--name"), "{next}");
        assert!(
            next.contains(&format!("1 to {} characters", hide_core::AGENT_NAME_LIMIT)),
            "{next}"
        );
        assert!(!next.contains("retry the same intent"), "{next}");
    }

    /// A device range is accepted only as the range asked for: same offset
    /// and file size, never longer, and short only where the read ends
    /// (PRD S5.5 B39, B43).
    #[test]
    fn a_device_range_answer_is_the_range_asked_for_or_the_read_fails() {
        const MIB: u64 = 1024 * 1024;
        let asked = AskedRange {
            offset: 0,
            length: 4 * MIB,
            end: 10 * MIB,
            total: 10 * MIB,
        };
        let full = (4 * MIB) as usize;
        assert_eq!(asked.accept(0, 10 * MIB, full), Some((full, false)));
        // One byte per range would take ten million round trips.
        assert_eq!(asked.accept(0, 10 * MIB, 1), None);
        // An empty answer before the end is not the end of the file.
        assert_eq!(asked.accept(0, 10 * MIB, 0), None);
        assert_eq!(asked.accept(0, 10 * MIB, full + 1), None);
        assert_eq!(asked.accept(1, 10 * MIB, full), None);
        assert_eq!(asked.accept(0, 11 * MIB, full), None);

        // The last range is short because the read ends there.
        let last = AskedRange {
            offset: 8 * MIB,
            length: 2 * MIB,
            end: 10 * MIB,
            total: 10 * MIB,
        };
        assert_eq!(
            last.accept(8 * MIB, 10 * MIB, (2 * MIB) as usize),
            Some(((2 * MIB) as usize, true))
        );
        // A read of an empty file ends at once.
        let empty = AskedRange {
            offset: 0,
            length: 4 * MIB,
            end: 0,
            total: 0,
        };
        assert_eq!(empty.accept(0, 0, 0), Some((0, true)));
        // A read that asked for less than the file ends at its own end.
        let part = AskedRange {
            offset: 0,
            length: 4 * MIB,
            end: MIB,
            total: 10 * MIB,
        };
        assert_eq!(part.accept(0, 10 * MIB, full), Some((MIB as usize, true)));
    }

    #[test]
    fn a_fresh_client_gets_a_snapshot_and_a_resumed_one_a_delta() {
        assert_eq!(classify_frame(0, Some(7)), Some(FrameKind::Snapshot));
        assert_eq!(classify_frame(7, Some(7)), Some(FrameKind::Delta));
        assert_eq!(classify_frame(5, Some(7)), Some(FrameKind::Delta));
    }

    #[test]
    fn a_gap_makes_the_server_start_over() {
        assert_eq!(classify_frame(9, Some(7)), None, "revision from the future");
    }

    #[test]
    fn the_host_handler_never_gets_a_program() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("notes.txt");
        std::fs::write(&plain, "x").unwrap();
        assert_eq!(openable(&plain), Ok(()), "an ordinary file opens");
        assert_eq!(
            openable(&dir.path().join("huge.md")),
            Err("not_found"),
            "a file that vanished between the boundary check and the handler is refused"
        );

        // A handler that runs what it opens, on any platform, is refused by
        // name; a plain text file with the same body is not.
        for name in [
            "run.sh",
            "run.tool",
            "app.app",
            "installer.pkg",
            "image.dmg",
            "term.terminal",
            "session.term",
            "job.command",
            "link.webloc",
            "script.py",
            "thing.jar",
            "run.exe",
            "run.bat",
            "run.ps1",
            "run.vbs",
            "notes.js",
            "notes.jse",
            "launch.jnlp",
            "snapin.msc",
            "setup.application",
            "ref.appref-ms",
            "pkg.appx",
            "pkg.msix",
            "pkg.appinstaller",
            "lib.dylib",
            "agent.desktop",
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, "x").unwrap();
            assert_eq!(openable(&path), Err("not_openable"), "{name}");
        }
        for name in [
            "notes.txt",
            "readme.md",
            "data.json",
            "clip.mp4",
            "bundle.ts",
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, "x").unwrap();
            assert_eq!(openable(&path), Ok(()), "{name}");
        }

        // An execute bit refuses a file with no telling extension.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let runnable = dir.path().join("server");
            std::fs::write(&runnable, "x").unwrap();
            std::fs::set_permissions(&runnable, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(openable(&runnable), Err("not_openable"), "execute bit");
        }

        // A program renamed to a document extension is refused by its header.
        let renamed = dir.path().join("notes.pdf");
        std::fs::write(&renamed, [0xCF, 0xFA, 0xED, 0xFE, 0, 0, 0, 0]).unwrap();
        assert_eq!(openable(&renamed), Err("not_openable"), "Mach-O header");
        let elf = dir.path().join("notes.png");
        std::fs::write(&elf, [0x7F, b'E', b'L', b'F', 0, 0, 0, 0]).unwrap();
        assert_eq!(openable(&elf), Err("not_openable"), "ELF header");
        let fat64 = dir.path().join("notes.md");
        std::fs::write(&fat64, [0xCA, 0xFE, 0xBA, 0xBF, 0, 0, 0, 0]).unwrap();
        assert_eq!(openable(&fat64), Err("not_openable"), "64-bit fat Mach-O");
        // A DOS/PE executable is refused whether its size field reads as text
        // or not: a real one carries the zero fields its header requires.
        let mut pe = b"MZ".to_vec();
        pe.extend_from_slice(b"A\0\x03\x00\x00\x00\x00\x00");
        pe.resize(0x3C, b' ');
        pe.extend_from_slice(&0x80u32.to_le_bytes());
        pe.resize(0x80, b' ');
        pe.extend_from_slice(b"PE\0\0");
        let pe_path = dir.path().join("notes.dat");
        std::fs::write(&pe_path, &pe).unwrap();
        assert_eq!(openable(&pe_path), Err("not_openable"), "PE header");
        let stub = dir.path().join("notes.txt");
        std::fs::write(&stub, b"MZ\x90\x00\x00").unwrap();
        assert_eq!(openable(&stub), Err("not_openable"), "MZ stub");
        // A document that merely starts with the same two letters opens.
        let text = dir.path().join("notes.md");
        std::fs::write(&text, b"MZ is a codec\n").unwrap();
        assert_eq!(openable(&text), Ok(()), "MZ letters in text");
        // And a PE whose header has no zero field at all is still caught by
        // the loader signature its `e_lfanew` points at.
        let at = 0x0101_0101u32;
        let mut crafted = b"MZ".to_vec();
        crafted.extend_from_slice(&[b'A'; 0x3A]);
        crafted.extend_from_slice(&at.to_le_bytes());
        crafted.resize(512, b'A');
        assert!(
            !crafted.contains(&0),
            "the crafted header carries no zero byte"
        );
        let crafted_path = dir.path().join("huge.md");
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&crafted_path).unwrap();
            file.write_all(&crafted).unwrap();
            file.write_all(&vec![b'A'; at as usize - crafted.len()])
                .unwrap();
            file.write_all(b"PE\0\0").unwrap();
        }
        assert_eq!(
            openable(&crafted_path),
            Err("not_openable"),
            "PE by e_lfanew"
        );
    }

    /// B43: `hide links`, `hide memory`, a hook's Memory and delivery answer
    /// with no shell open; every Workspace command still needs one.
    #[test]
    fn only_the_daemons_own_reads_answer_without_a_shell() {
        use hide_core::links::query::{LinksQuery, QueryTarget};
        let links = ScopedRequest::Links(LinksQuery {
            target: QueryTarget::Pr {
                number: 1,
                host: None,
                repository: None,
            },
            all_projects: false,
        });
        assert!(!links.needs_renderer());
        assert!(
            !ScopedRequest::Session(hide_core::links::session_command::SessionCommand::Record {
                id: "s-1".into()
            })
            .needs_renderer()
        );
        assert!(
            !ScopedRequest::Memory(hide_core::memory_command::MemoryCommand::Show { number: 1 })
                .needs_renderer()
        );
        let hook = hide_core::memory_hook::Ask::from_wire(&json!({
            "event": "SessionStart", "runtime": "codex", "session": "s-1",
        }))
        .unwrap();
        assert!(!ScopedRequest::MemoryHook(hook).needs_renderer());
        assert!(ScopedRequest::Query(hide_core::workspace_control::Query::Info).needs_renderer());
        assert!(ScopedRequest::BrowserConnect(None).needs_renderer());
        // C25: the rest of the daemon's own answers, a tool's list among them.
        for request in [
            ScopedRequest::Ps { all: false },
            ScopedRequest::AccessList,
            ScopedRequest::AgentCatalog(None),
            ScopedRequest::AgentSleep {
                id: "agent-1".into(),
                actor: None,
            },
            ScopedRequest::AgentArchive {
                id: "agent-1".into(),
                actor: None,
            },
            ScopedRequest::Device(hide_core::device_command::DeviceCommand::List),
            ScopedRequest::ProjectOpen("/srv/project".into()),
            ScopedRequest::Pane(hide_core::pane_command::PaneCommand::Read {
                pane: "w1:p1".into(),
                lines: 10,
            }),
        ] {
            assert!(!request.needs_renderer());
        }
        assert!(
            ScopedRequest::Action(hide_core::workspace_control::Action::OpenFile {
                path: "/srv/a".into(),
            })
            .needs_renderer()
        );
    }

    #[test]
    fn token_compare_needs_the_whole_token() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("ab", "abc"));
        assert!(!token_matches("abd", "abc"));
        assert!(!token_matches("", "abc"));
    }

    /// After a launch, a checkout the core learns from the engine's first sync
    /// reaches a client's snapshot and the boundary on separate reads. A
    /// listing that names it before the boundary applied it is answered once
    /// the roots are current, instead of leaving the Explorer empty on
    /// `outside_checkout`; a root the core does not carry is still refused,
    /// and a refusal for any other reason reads nothing.
    #[test]
    fn an_event_naming_a_root_the_boundary_has_not_applied_yet_is_checked_again_once_the_roots_are_current()
     {
        let home = tempfile::tempdir().unwrap();
        let checkout = home.path().join("checkout");
        std::fs::create_dir(&checkout).unwrap();
        std::fs::write(checkout.join("f65.txt"), "file 65\n").unwrap();
        let boundary = Boundary::new(home.path()).unwrap();
        let root = checkout.display().to_string();

        let mut reads = 0;
        let mut event = json!({"schema_version": 2, "kind": "file_list", "payload": {"root": root, "path": root}});
        let answer = admit_event(&boundary, &mut event, || {
            reads += 1;
            boundary.set_roots(vec![crate::boundary::Root {
                workspace_id: "w1".to_owned(),
                checkout_id: "c1".to_owned(),
                path: checkout.clone(),
            }]);
            true
        })
        .expect("a listing is answered here");
        assert_eq!(reads, 1);
        assert_eq!(answer["type"], "directory_list", "{answer}");
        assert!(answer.to_string().contains("f65.txt"), "{answer}");

        // A root the core does not carry either is refused after one read.
        let other = home.path().join("elsewhere");
        std::fs::create_dir(&other).unwrap();
        let other = other.display().to_string();
        let mut reads = 0;
        let mut event = json!({"schema_version": 2, "kind": "file_list", "payload": {"root": other, "path": other}});
        let answer = admit_event(&boundary, &mut event, || {
            reads += 1;
            true
        })
        .expect("refused");
        assert_eq!(reads, 1);
        assert!(refused_outside_checkout(&answer), "{answer}");

        // A name the boundary refuses for itself never waits on the core.
        let mut event = json!({"schema_version": 2, "kind": "file_create", "payload": {"root": root, "parent": root, "name": "../out"}});
        let answer =
            admit_event(&boundary, &mut event, || panic!("no root was missing")).expect("refused");
        assert_eq!(
            answer["payload"]["reason"],
            Refusal::InvalidPath.code(),
            "{answer}"
        );
    }

    /// A browser display's `file:` address is a path like any other: under a
    /// registered checkout it reaches the core as the URL of the checked
    /// path; outside one, or naming another host, it never does. A web
    /// address is not the boundary's to judge, and a layout action other than
    /// navigate carries no address.
    #[test]
    fn a_file_address_reaches_the_core_only_under_a_checkout() {
        let home = tempfile::tempdir().unwrap();
        let checkout = home.path().join("check out");
        std::fs::create_dir(&checkout).unwrap();
        std::fs::write(checkout.join("보고서.html"), "<p>ok</p>").unwrap();
        std::fs::write(home.path().join("secret.html"), "no").unwrap();
        let boundary = Boundary::new(home.path()).unwrap();
        boundary.set_roots(vec![crate::boundary::Root {
            workspace_id: "w1".to_owned(),
            checkout_id: "c1".to_owned(),
            path: checkout.clone(),
        }]);
        // Sent as `localhost` with the file name escaped in lower case; it
        // reaches the core as the one spelling of the path that was checked.
        let checked = hide_platform::path::to_file_url(
            &checkout.join("보고서.html").display().to_string(),
            "#top",
        );
        let inside = checked
            .replacen("file://", "file://localhost", 1)
            .replace("%EB%B3%B4", "%eb%b3%b4");

        for kind in ["browser_open", "browser_state"] {
            let mut event = json!({"schema_version": 2, "kind": kind, "payload": {"url": inside}});
            assert_eq!(apply_boundary(&boundary, &mut event), None, "{kind}");
            assert_eq!(event["payload"]["url"], checked, "{kind}");
        }
        let mut event = json!({"schema_version": 2, "kind": "view_layout", "payload": {"action": "navigate", "display_id": "d1", "url": inside}});
        assert_eq!(apply_boundary(&boundary, &mut event), None);
        assert_eq!(event["payload"]["url"], checked);

        let outside = hide_platform::path::to_file_url(
            &home.path().join("secret.html").display().to_string(),
            "",
        );
        for (url, reason) in [
            (outside.as_str(), Refusal::OutsideCheckout),
            ("file://server/share/a.html", Refusal::InvalidPath),
        ] {
            let mut event =
                json!({"schema_version": 2, "kind": "browser_open", "payload": {"url": url}});
            let answer = apply_boundary(&boundary, &mut event).expect("refused");
            assert_eq!(answer["type"], "path_refused", "{answer}");
            assert_eq!(answer["payload"]["reason"], reason.code(), "{answer}");
        }
        let mut event = json!({"schema_version": 2, "kind": "view_layout", "payload": {"action": "navigate", "display_id": "d1", "url": outside}});
        assert!(apply_boundary(&boundary, &mut event).is_some());

        for mut event in [
            json!({"schema_version": 2, "kind": "browser_open", "payload": {"url": "https://example.com/a"}}),
            json!({"schema_version": 2, "kind": "view_layout", "payload": {"action": "close", "display_id": "d1", "url": outside}}),
        ] {
            let before = event.clone();
            assert_eq!(apply_boundary(&boundary, &mut event), None);
            assert_eq!(event, before);
        }
    }

    #[test]
    fn only_this_nodes_id_or_none_reads_through_this_machines_boundary() {
        let home = tempfile::tempdir().unwrap();
        let boundary = Boundary::new(home.path()).unwrap();
        let event = |device: Value| json!({"kind": "file_open", "payload": {"device_id": device}});
        assert_eq!(event_device(&boundary, &event(Value::Null)), None);
        assert_eq!(event_device(&boundary, &event(json!(""))), None);
        assert_eq!(
            event_device(&boundary, &event(json!(boundary.node().as_str()))),
            None
        );
        // The id this machine had before node ids is a device's now, and a
        // device path never resolves through this machine's checkouts.
        assert_eq!(
            event_device(&boundary, &event(json!("local"))).as_deref(),
            Some("local")
        );
        assert_eq!(
            event_device(&boundary, &event(json!("mini"))).as_deref(),
            Some("mini")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_browser_file_report_keeps_its_load_through_a_registered_root_alias() {
        let home = tempfile::tempdir().unwrap();
        let physical = home.path().join("physical");
        let alias = home.path().join("alias");
        std::fs::create_dir(&physical).unwrap();
        std::os::unix::fs::symlink(&physical, &alias).unwrap();
        let checkout = alias.join("checkout");
        std::fs::create_dir(&checkout).unwrap();
        std::fs::write(checkout.join("manual.html"), "<title>Manual file</title>").unwrap();
        let boundary = Boundary::new(home.path()).unwrap();
        boundary.set_roots(vec![
            crate::boundary::Root {
                workspace_id: "w0".to_owned(),
                checkout_id: "c0".to_owned(),
                path: alias.clone(),
            },
            crate::boundary::Root {
                workspace_id: "w1".to_owned(),
                checkout_id: "c1".to_owned(),
                path: checkout.clone(),
            },
        ]);
        let native = checkout.join("manual.html").canonicalize().unwrap();
        let native_url = hide_platform::path::to_file_url(&native.display().to_string(), "#loaded");
        let checked_url = hide_platform::path::to_file_url(
            &checkout.join("manual.html").display().to_string(),
            "#loaded",
        );
        let mut report = json!({"schema_version": 2, "kind": "browser_state", "payload": {
            "workspace": {"device_id": boundary.node().as_str(), "path": checkout},
            "display_id": "d1", "url": native_url, "title": "Manual file",
            "load": 7, "loading": false, "failure": null, "present": true,
        }});
        assert_eq!(apply_boundary(&boundary, &mut report), None);
        assert_eq!(report["payload"]["url"], checked_url);
        assert_eq!(report["payload"]["load"], 7);
        assert_eq!(report["payload"]["loading"], false);

        // Report spelling support grants no new file-open authority.
        let mut open = json!({"schema_version": 2, "kind": "browser_open", "payload": {
            "url": native_url,
        }});
        assert!(apply_boundary(&boundary, &mut open).is_some());

        // An unrelated path still cannot become a browser load report.
        std::fs::write(home.path().join("secret.html"), "outside").unwrap();
        let outside = hide_platform::path::to_file_url(
            &home.path().join("secret.html").display().to_string(),
            "",
        );
        std::os::unix::fs::symlink(
            home.path().join("secret.html"),
            checkout.join("escape.html"),
        )
        .unwrap();
        let escape = hide_platform::path::to_file_url(
            &native
                .parent()
                .unwrap()
                .join("escape.html")
                .display()
                .to_string(),
            "",
        );
        for url in [outside, escape] {
            report["payload"]["url"] = Value::String(url);
            let answer = apply_boundary(&boundary, &mut report).expect("outside file refused");
            assert_eq!(answer["payload"]["reason"], Refusal::OutsideCheckout.code());
        }

        // Resolving a native spelling cannot revive a replaced root, even
        // when its still-current parent is registered too.
        std::fs::rename(&checkout, alias.join("retired")).unwrap();
        std::fs::create_dir(&checkout).unwrap();
        std::fs::write(checkout.join("manual.html"), "replacement").unwrap();
        report["payload"]["url"] = Value::String(native_url);
        let answer = apply_boundary(&boundary, &mut report).expect("replaced root refused");
        assert_eq!(answer["payload"]["reason"], Refusal::OutsideCheckout.code());
    }

    /// A clone's folder is checked on the `$HOME` line before the core sees
    /// the event, and a `clone_target` question is answered on the same line.
    #[test]
    fn clone_events_meet_the_home_line() {
        let home = tempfile::tempdir().unwrap();
        let projects = home.path().join("projects");
        std::fs::create_dir_all(projects.join("taken")).unwrap();
        let boundary = Boundary::new(home.path()).unwrap();
        let real = projects.canonicalize().unwrap();

        let mut event = json!({"schema_version": 2, "kind": "clone_repository", "payload": {
            "url": "https://example.com/org/fresh.git", "parent": "~/projects/", "name": "fresh"
        }});
        assert_eq!(apply_boundary(&boundary, &mut event), None);
        assert_eq!(event["payload"]["parent"], real.display().to_string());
        assert_eq!(event["payload"]["url"], "https://example.com/org/fresh.git");

        let mut event = json!({"schema_version": 2, "kind": "clone_repository", "payload": {
            "url": "https://example.com/org/taken.git", "parent": "~/projects", "name": "taken"
        }});
        let answer = apply_boundary(&boundary, &mut event).expect("refused");
        assert_eq!(answer["type"], "path_refused", "{answer}");
        assert_eq!(answer["payload"]["kind"], "clone_repository");
        assert_eq!(answer["payload"]["path"], "~/projects/taken");
        assert_eq!(answer["payload"]["reason"], "already_exists");

        let answer = clone_target(
            &boundary,
            &json!({"schema_version": 2, "kind": "clone_target", "payload": {"parent": "~/projects", "name": "fresh"}}),
        );
        assert_eq!(answer["type"], "clone_target");
        assert_eq!(
            answer["payload"]["path"],
            real.join("fresh").display().to_string()
        );
        assert_eq!(answer["payload"]["reason"], Value::Null);

        // A clone never lands outside home, whatever else the event carries
        // (PRD wsl-device B17).
        let outside = tempfile::tempdir().unwrap();
        let mut event = json!({"schema_version": 2, "kind": "clone_repository", "payload": {
            "url": "https://example.com/org/fresh.git",
            "parent": outside.path().display().to_string(), "name": "fresh",
            "confirmed_outside_home": outside.path().display().to_string(),
        }});
        let answer = apply_boundary(&boundary, &mut event).expect("refused");
        assert_eq!(answer["payload"]["reason"], "outside_home", "{answer}");

        for (parent, name, reason) in [
            ("~/projects", "taken", "already_exists"),
            ("/", "fresh", "outside_home"),
            ("~/missing", "fresh", "not_found"),
            ("~/projects", "../fresh", "invalid_path"),
        ] {
            let answer = clone_target(
                &boundary,
                &json!({"schema_version": 2, "kind": "clone_target", "payload": {"parent": parent, "name": name}}),
            );
            assert_eq!(answer["payload"]["reason"], reason, "{answer}");
            assert_eq!(answer["payload"]["path"], Value::Null, "{answer}");
        }
    }

    /// Create new project is judged on the `$HOME` line before anything is
    /// made: the probe answers the parent, the full path, and what the create
    /// would be refused for, and the create itself is forwarded as the
    /// canonical path only when the folder is free or a leftover of its own.
    #[test]
    fn a_new_project_is_checked_on_the_home_line_before_its_folder_is_made() {
        let home = tempfile::tempdir().unwrap();
        let projects = home.path().join("projects");
        std::fs::create_dir_all(projects.join("taken")).unwrap();
        std::fs::write(projects.join("taken/notes.md"), "mine\n").unwrap();
        std::fs::create_dir_all(projects.join("leftover/.git")).unwrap();
        let boundary = Boundary::new(home.path()).unwrap();
        let real = boundary.home().join("projects");
        let probe = |parent: &str, name: &str| {
            project_target(
                &boundary,
                &json!({"schema_version": 2, "kind": "project_target", "payload": {"parent": parent, "name": name}}),
            )["payload"]
                .clone()
        };

        let answer = probe("~", "");
        assert_eq!(answer["parent_label"], "~", "{answer}");
        assert_eq!(answer["parent_path"], boundary.home().display().to_string());
        assert_eq!(answer["path"], Value::Null);
        assert_eq!(answer["reason"], Value::Null);

        let answer = probe(&real.display().to_string(), "fresh");
        assert_eq!(answer["parent_label"], "~/projects", "{answer}");
        assert_eq!(answer["path"], real.join("fresh").display().to_string());
        assert_eq!(
            (answer["reason"].clone(), answer["leftover"].clone()),
            (Value::Null, Value::Bool(false))
        );
        assert_eq!(probe("~/projects", "leftover")["leftover"], true);
        for (parent, name, reason) in [
            ("~/projects", "taken", "already_exists"),
            ("~/projects", "a/b", "invalid_path"),
            ("~/projects", "..", "invalid_path"),
            ("~/projects/missing", "x", "not_found"),
            ("/", "x", "outside_home"),
        ] {
            assert_eq!(probe(parent, name)["reason"], reason, "{parent} {name}");
        }
        assert!(!real.join("fresh").exists(), "a probe makes nothing");

        let create = |path: &str| {
            json!({"schema_version": 2, "kind": "create_workspace",
                "payload": {"path": path, "label": "x", "initialize_git": true, "new_folder": true}})
        };
        let mut event = create("~/projects/fresh");
        assert_eq!(apply_boundary(&boundary, &mut event), None);
        assert_eq!(
            event["payload"]["path"],
            real.join("fresh").display().to_string()
        );
        let mut event = create("~/projects/leftover");
        assert_eq!(
            apply_boundary(&boundary, &mut event),
            None,
            "a retry continues into its leftover"
        );
        for (path, reason) in [
            ("~/projects/taken", "already_exists"),
            ("/tmp/elsewhere", "outside_home"),
            ("~/projects/", "invalid_path"),
        ] {
            let mut event = create(path);
            // Creating a folder outside home is refused even when the request
            // says the operator confirmed it (PRD wsl-device B17).
            event["payload"]["confirmed_outside_home"] = Value::String(path.to_owned());
            let answer = apply_boundary(&boundary, &mut event).expect("refused");
            assert_eq!(answer["type"], "path_refused", "{answer}");
            assert_eq!(answer["payload"]["reason"], reason, "{path}");
            assert_eq!(answer["payload"]["path"], path);
        }
        assert_eq!(
            std::fs::read_to_string(projects.join("taken/notes.md")).unwrap(),
            "mine\n"
        );
    }

    /// PRD device-add-project B25: a refused add that named a request id
    /// carries it back, so the importable list reads the refusal as that
    /// row's own; one that named none carries none.
    #[test]
    fn a_refused_add_carries_the_request_id_it_was_sent_with() {
        let home = tempfile::tempdir().unwrap();
        let boundary = Boundary::new(home.path()).unwrap();
        let gone = home.path().join("gone").display().to_string();
        let mut event = json!({"schema_version": 2, "kind": "create_workspace",
            "payload": {"path": gone, "label": "gone", "initialize_git": false, "request_id": "import:7"}});
        let answer = apply_boundary(&boundary, &mut event).expect("refused");
        assert_eq!(answer["type"], "path_refused", "{answer}");
        assert_eq!(answer["payload"]["request_id"], "import:7");
        let mut event = json!({"schema_version": 2, "kind": "create_workspace",
            "payload": {"path": gone, "label": "gone", "initialize_git": false}});
        let answer = apply_boundary(&boundary, &mut event).expect("refused");
        assert_eq!(answer["payload"].get("request_id"), None);
    }

    /// PRD wsl-device B15, B18: a folder outside home reaches the core only
    /// when the event carries the operator's confirmation of that path; the
    /// refusal before it says so and reads nothing there.
    #[test]
    fn a_folder_outside_home_is_forwarded_only_with_its_confirmation() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join("app")).unwrap();
        let boundary = Boundary::new(home.path()).unwrap();
        let path = outside.path().join("app").display().to_string();
        let create = |confirmed: Option<&str>| {
            let mut payload = json!({"path": path, "label": "app", "initialize_git": false});
            if let Some(confirmed) = confirmed {
                payload["confirmed_outside_home"] = Value::String(confirmed.to_owned());
            }
            json!({"schema_version": 2, "kind": "create_workspace", "payload": payload})
        };
        let other = outside.path().join("other").display().to_string();
        for confirmed in [None, Some(other.as_str())] {
            let mut event = create(confirmed);
            let answer = apply_boundary(&boundary, &mut event).expect("refused");
            assert_eq!(answer["type"], "path_refused", "{answer}");
            assert_eq!(answer["payload"]["reason"], "needs_confirmation");
            assert_eq!(answer["payload"]["path"], path.as_str());
        }
        let mut event = create(Some(&path));
        assert_eq!(apply_boundary(&boundary, &mut event), None);
        assert_eq!(
            event["payload"]["path"],
            outside
                .path()
                .join("app")
                .canonicalize()
                .unwrap()
                .display()
                .to_string()
        );

        // A folder inside a repository there is answered with the question
        // about the repository, which the frame names as `confirm`.
        let app = outside.path().join("app");
        std::fs::create_dir_all(app.join(".git")).unwrap();
        std::fs::write(app.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir_all(app.join("src")).unwrap();
        let src = app.join("src").display().to_string();
        let mut event = json!({"schema_version": 2, "kind": "create_workspace", "payload": {
            "path": src, "label": "src", "initialize_git": false, "confirmed_outside_home": src,
        }});
        let answer = apply_boundary(&boundary, &mut event).expect("asked");
        assert_eq!(
            answer["payload"]["reason"], "needs_confirmation",
            "{answer}"
        );
        assert_eq!(answer["payload"]["path"], src.as_str());
        assert_eq!(
            answer["payload"]["confirm"],
            hide_platform::path::to_wire_lossy(
                &hide_platform::fs::identity::canonical(&app).unwrap()
            )
        );
    }

    /// A screen another machine shows of this one lists the phones and can
    /// remove one, and never receives a live pairing code (PRD
    /// device-connect-iroh D-53, D-70).
    #[test]
    fn a_screen_of_another_machine_gets_no_pairing_code() {
        let payload = json!({"qr": "https://app.withhide.dev/#pair=x", "code_expires_at_ms": 9, "phones": [{"id": "a"}]});
        let own = mobile_frame(&payload, true);
        assert_eq!(own["payload"], payload);
        let other = mobile_frame(&payload, false);
        assert_eq!(other["payload"]["qr"], Value::Null);
        assert_eq!(other["payload"]["code_expires_at_ms"], Value::Null);
        assert_eq!(other["payload"]["phones"], payload["phones"]);
    }

    /// The same screen cannot make or end a code by asking, nor by turning
    /// Mobile off or changing its method, which end it too: the connection
    /// refuses them, removing a phone still goes through, and its watching of
    /// Settings › Devices never ends the code a person at this machine shows
    /// (PRD device-connect-iroh D-53, D-70).
    #[test]
    fn a_screen_of_another_machine_cannot_make_or_end_a_pairing_code() {
        for kind in [
            "mobile_show_code",
            "mobile_hide_code",
            "mobile_enable",
            "mobile_method",
            "mobile_rename",
            "mobile_unknown",
        ] {
            assert!(mobile_event_reaches(kind, false).is_err(), "{kind}");
            assert_eq!(mobile_event_reaches(kind, true), Ok(true), "{kind}");
        }
        assert_eq!(mobile_event_reaches("mobile_observe", false), Ok(false));
        assert_eq!(mobile_event_reaches("mobile_observe", true), Ok(true));
        for kind in ["mobile_revoke", "mobile_retry", "mobile_push_mode"] {
            assert_eq!(mobile_event_reaches(kind, false), Ok(true), "{kind}");
        }
    }

    /// Add a project's folder list words a node's refusal by the code the
    /// node judged with, and a machine it cannot reach as disconnected, so a
    /// device's `outside_home` reads as this machine's does (PRD
    /// device-add-project B12, B31).
    #[test]
    fn a_node_ask_refusal_carries_the_reason_the_dialog_reads() {
        use hide_node_link::{ErrorCode, HostError, LinkError};
        let refused = |code| node_ask_reason(&LinkError::Refused(HostError::new(code, "refused")));
        assert_eq!(refused(ErrorCode::OutsideRoot), "outside_home");
        assert_eq!(refused(ErrorCode::NotFound), "not_found");
        assert_eq!(refused(ErrorCode::NotADirectory), "not_a_directory");
        assert_eq!(refused(ErrorCode::InvalidPath), "invalid_path");
        assert_eq!(refused(ErrorCode::AlreadyExists), "already_exists");
        assert_eq!(refused(ErrorCode::Io), "unreadable");
        assert_eq!(node_ask_reason(&LinkError::Busy), "busy");
        assert_eq!(
            node_ask_reason(&LinkError::NotConnected("gone".into())),
            "disconnected"
        );
        assert_eq!(
            node_ask_reason(&LinkError::Unknown("lost".into())),
            "disconnected"
        );
    }
}
