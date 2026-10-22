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
use crate::boundary::{self, Boundary, Listing, Refusal};
use crate::core::CoreHandle;
use crate::index::{IndexAnswer, IndexService};
use crate::pane_auth::Registry;
use crate::screen_event::{self, Kind};
use crate::state_file::{MAX_CLIENTS, SCHEMA_VERSION};
use crate::terminal_hub::Resume;
use crate::watch::WatchService;
use base64::Engine as _;
use hide_node::opener::OpenHandler;
use hide_node_link::terminal::{GridSize, KeyTarget, MAX_PANE_ID_BYTES, TerminalNode};

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
    pub herdr_socket: Option<PathBuf>,
    pub allowed_origins: Arc<HashSet<String>>,
    pub clients: Arc<AtomicUsize>,
    /// Authenticated shell windows, distinct from CLI and non-rendering clients.
    pub renderers: Arc<AtomicUsize>,
    pub desktop_renderers: Arc<AtomicUsize>,
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
    /// Closed while this core runs on a move's copy that has not committed:
    /// it takes no move of its own then, and dials nothing to check one.
    pub move_gate: Arc<crate::core_move::gate::MoveGate>,
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
    /// Which connections show which backlog item's origin: the core keeps
    /// an origin until the last of them closes it or leaves.
    pub origin_holds: Arc<crate::demand::OriginHolds>,
    /// What Settings > About reads about this daemon, sent once after
    /// a handshake. No token, no environment beyond the paths it names.
    pub daemon_info: Arc<Value>,
    /// Settings > Mobile and the phones it pairs (`mobile/`).
    pub mobile: Arc<crate::mobile::Mobile>,
    /// The desktop windows' agent notifications (`desktop_notices.rs`).
    pub desktop_notices: Arc<crate::desktop_notices::DesktopNotices>,
    /// The grants this core handed linked nodes' screen relays (`relay`).
    pub relay_grants: Arc<crate::attach::RelayGrants>,
    /// Windows of this machine open now (`OwnScreen`).
    pub own_screens: Arc<Mutex<usize>>,
    /// Which screen each pane's terminal size follows (`pane_sizes`).
    pub pane_sizes: Arc<crate::pane_sizes::PaneSizes>,
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

mod relay_routes;

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
    eprintln!(
        "{}",
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
    use herdr_core::workspace_control::{ActionPreparation, Query};
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
        "schema_version": SCHEMA_VERSION,
        "clients": state.clients.load(Ordering::SeqCst),
        "open_handlers_in_flight": state.opener.in_flight(),
        "idle_remaining_secs": idle_remaining_secs(&state),
        "instance": state.seat.instance(),
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
        let previous = state.clients.fetch_add(1, Ordering::SeqCst);
        if previous >= MAX_CLIENTS {
            state.clients.fetch_sub(1, Ordering::SeqCst);
            refuse(&mut socket, CloseReason::ClientLimit, Some(previous + 1)).await;
            return;
        }
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
    screen_loop(socket, state, connection, handshake, None).await;
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
) {
    let previous = state.clients.fetch_add(1, Ordering::SeqCst);
    if previous >= MAX_CLIENTS {
        state.clients.fetch_sub(1, Ordering::SeqCst);
        refuse(&mut socket, CloseReason::ClientLimit, Some(previous + 1)).await;
        return;
    }
    if previous == 0 {
        state.roots.resume();
    }
    let renderer = matches!(handshake.client_kind.as_deref(), Some("web" | "desktop"));
    let desktop = handshake.client_kind.as_deref() == Some("desktop");
    if renderer {
        renderer_count(&state, true);
    }
    if desktop {
        state.desktop_renderers.fetch_add(1, Ordering::SeqCst);
    }
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
    // A move of the core is the process's, beside the core's state.
    let mut move_frames = state.seat.moves.subscribe();
    if renderer && relay.is_none() {
        let frame = crate::core_move::control::frame(&move_frames.borrow_and_update());
        if socket.send(Message::Text(frame.into())).await.is_err() {
            client_gone(&state, connection, renderer, desktop);
            return;
        }
    }
    // A desktop window shows the agent notifications (`agent_notice`); a
    // browser tab never does (PRD first-run-onboarding D-14), so it keeps no
    // receiver that would hold frames it never reads.
    let mut agent_notices = desktop.then(|| state.desktop_notices.subscribe());
    // Settings > Mobile reads one `mobile` frame, sent now and on each change.
    let mut mobile_frames = state.mobile.subscribe_frame();
    if renderer {
        let frame =
            json!({"type": "mobile", "payload": mobile_frames.borrow_and_update().as_ref()});
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
        state.core.hub.connect(
            match (
                first,
                handshake.have_terminal_sequence,
                handshake.have_terminal_epoch.clone(),
            ) {
                (FrameKind::Snapshot, _, _) => Resume::Fresh,
                (FrameKind::Delta, Some(cursor), Some(epoch)) => Resume::After { epoch, cursor },
                // A cursor without the hub it came from is not trusted.
                (FrameKind::Delta, _, _) => Resume::Redraw,
            },
        )
    });
    let link = relay.as_ref().map(|relay| relay.link.clone());
    let mut role_ended = state.role_ended.clone();
    let mut ended_with_role = false;
    loop {
        tokio::select! {
            () = async { let _ = role_ended.wait_for(|ended| *ended).await; } => {
                ended_with_role = true;
                break;
            }
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
                        // A folder on this machine is re-checked against its
                        // checkout roots; a device's folder names a path there,
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
            changed = move_frames.changed(), if renderer && relay.is_none() => {
                if changed.is_err() {
                    break;
                }
                let frame = crate::core_move::control::frame(&move_frames.borrow_and_update());
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
                        herdr_core::diagnostic!(json!({
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
                let frame = json!({"type": "mobile", "payload": mobile_frames.borrow_and_update().as_ref()});
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
                        match handle_client_text(&state, &text, connection, desktop, renderer) {
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
                            Ok(ClientAction::CoreMove(event)) => {
                                // A node's window moves the core from its own
                                // machine's hided, never through this one.
                                let answer = if relay.is_some() {
                                    Err("move_not_here")
                                } else if !state.move_gate.is_open() {
                                    Err("core_pending")
                                } else {
                                    state.seat.moves.request_event(&event)
                                };
                                if let Err(reason) = answer {
                                    let frame = crate::core_move::control::refusal_frame(reason);
                                    if socket.send(Message::Text(frame.into())).await.is_err() {
                                        break;
                                    }
                                }
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
    if ended_with_role {
        let _ = socket.send(Message::Close(Some(role_ended_frame()))).await;
    }
    client_gone(&state, connection, renderer, desktop);
}

/// What a refused delivery or agent command asks of its caller.
pub(crate) fn delivery_next_action(code: &str) -> &'static str {
    match herdr_core::coordination::split_refusal(code).0 {
        "agent_pane_required" => {
            "Run the command inside a pane where the agent is running; an agent on Codex's shared daemon needs a session started with --no-daemon"
        }
        "machine_unknown" => {
            "Use the device id from hide workspace info or hide agent list with --machine, then retry"
        }
        "machine_unavailable" => {
            "Check that the device is connected in Settings > Devices and its helper is allowed, then run the same command again"
        }
        "repository_unavailable" => {
            "Pass --repo as the repository's path on that device, then run the command again"
        }
        "agent_not_installed" => {
            "Install the agent CLI on the device so it is on the device's PATH, then run the command again"
        }
        "spawn_busy" => "The first request is still running; wait, then run the same command again",
        "machine_not_permitted" => {
            "Run the command from an agent on the machine that runs Hide, or leave out --machine"
        }
        "intent_conflict" => {
            "Use a new intent, or repeat the request this intent was first used for"
        }
        "invalid_launch_choice" => {
            "Pass a --model that is a model name, and an --effort the agent's CLI takes (Claude Code: low, medium, high, xhigh, max; Codex: minimal, low, medium, high, xhigh), then run the command again"
        }
        "worktree_unconfirmed" => {
            "Herdr had not confirmed the new worktree in time and may still finish it; run the same command again with the same intent"
        }
        "agent_not_started" => {
            "The agent's start ended in its pane without the agent and was not typed again; check that pane for why, then spawn with a new intent"
        }
        "shell_starting" => {
            "The new pane's shell had not finished starting; answer what its startup files ask in that pane, then spawn with a new intent"
        }
        "pane_busy" => {
            "The new pane's shell kept another program running for the start's 30 s wait, so nothing was typed; let that program finish or end it in that pane, then run the same command again with the same intent"
        }
        "native_identity_unavailable" => {
            "The agent started and has not reported its session yet (Codex and OpenCode 2 report theirs at their first turn); Hide registers it with its parent and watch when it does, and the same command with the same intent then answers it"
        }
        "identity_waits_full" => {
            "Too many agents Hide started are waiting to report their session; give one of them a first prompt or close its pane, then run the same command again"
        }
        "session_too_long" => {
            "The agent names its session by a file path longer than 4096 bytes; spawn it with a new intent in a checkout with a shorter path"
        }
        "target_ambiguous" => {
            "More than one agent answers to that name, on different machines; send to its registration ID from hide agent list (agent-N), or to the pane's full address remote:<device>:pane:<id>"
        }
        "target_unavailable" => {
            "No agent answers to that; a bare pane ID names a pane on your own machine. Find the recipient in hide agent list and send to its name or registration ID"
        }
        "mission_cap_reached" => {
            "The mission's workers are at its cap; wait for one to finish and end it with hide agent end <id>, then run the same command again with the same intent"
        }
        "mission_brief_missing" => {
            "A mission node's worker starts on its brief: spawn again with --brief-file <path> (or --brief <text>) holding what the node is to do"
        }
        "invalid_brief" => "The brief must be text of at most 16 KiB that is not blank",
        "brief_not_submitted" => {
            "The agent started but did not take its brief within 30 seconds; look at its pane, then run the same command again with the same intent, which submits the brief without starting the agent again"
        }
        "mission_not_started" => {
            "The operator has not approved the mission's plan; show it to them and run hide mission start once they approve"
        }
        "mission_machine_refused" => {
            "A mission's workers run on the machine its project is on; leave out --machine or name that machine"
        }
        "mission_candidate_refused" => {
            "The worker is none of the operator's candidates in Settings > Conductor; run hide mission settings and start it as one of them (its agent, its fixed model or the CLI's default, its fixed effort)"
        }
        "codex_capability_unknown" => {
            "Hide has not read whether that machine's Codex runs a shared server; ask the operator to press Check again in Settings > Agents for that machine, then run the same command again with the same intent"
        }
        _ => "Check the current agent pane and retry the same intent",
    }
}

/// What the core knows of a display a `hide browser` command relays to:
/// whether it is its area's selected View (the only one shown, so the CLI
/// refuses input to any other before sending it) and `source_url`, the
/// address the core keeps for it. A device's page loads from a loopback route
/// on this machine, so the page reports that address and the CLI names this
/// one instead. `None` when the checkout has no such display.
pub(crate) fn relay_facts(
    views: &[herdr_core::workspace_control::View],
    display_id: &str,
) -> Option<Value> {
    let view = views.iter().find(|view| view.view_id == display_id)?;
    Some(json!({"selected": view.selected, "source_url": view.target}))
}

/// The relay's answer to the CLI: `relay_facts` under the display's id.
pub(crate) fn relay_ready(display_id: &str, facts: &Value) -> Value {
    json!({"display_id": display_id, "selected": facts["selected"], "source_url": facts["source_url"]})
}

/// The reason and next action of a refused command. A refusal that names the
/// devices the caller could have used says so in its next action, and its
/// reason stays the code.
fn refusal_answer(reason: String, next_action: &'static str) -> (String, String) {
    match herdr_core::coordination::split_refusal(&reason) {
        (code @ "shell_starting", Some(screen)) if !screen.is_empty() => (
            code.to_owned(),
            format!("{next_action}. Its screen ends with: {screen}"),
        ),
        (code @ "shell_starting", _) => (code.to_owned(), next_action.to_owned()),
        (code @ "mission_cap_reached", Some(cap)) => (
            code.to_owned(),
            format!("{next_action}; the mission runs at most {cap} workers at once"),
        ),
        (code @ "mission_conducted", Some(name)) => (
            code.to_owned(),
            format!("{next_action}; {name} conducts it now"),
        ),
        // A plan's refusal names the step or the field to fix (PRD hide-plan
        // B1, B2).
        (
            code @ ("plan_invalid" | "plan_order_refused" | "plan_too_large" | "plan_node_unknown"),
            Some(detail),
        ) => (code.to_owned(), format!("{next_action} ({detail})")),
        (code, Some("")) => (
            code.to_owned(),
            "No other device is connected; connect one in Settings > Devices, then retry"
                .to_owned(),
        ),
        (code, Some(connected)) => (
            code.to_owned(),
            format!("Use one of the connected device ids with --machine: {connected}"),
        ),
        (_, None) => (reason, next_action.to_owned()),
    }
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
    Query(herdr_core::workspace_control::Query),
    Action(herdr_core::workspace_control::Action),
    BrowserConnect(Option<String>),
    /// `hide browser` page commands: the same checks as a display
    /// connection, then this socket carries the CDP frames.
    BrowserRelay(String),
    Delivery(Box<herdr_core::delivery::Command>, Option<String>),
    /// `hide links`: a read of the link record, which needs no shell.
    Links(herdr_core::links::query::LinksQuery),
    /// `hide memory`: Project Memory's own store, which needs no shell.
    Memory(herdr_core::memory_command::MemoryCommand),
    /// `hide workspace memory`: a hook's Project Memory, which needs no
    /// shell.
    MemoryHook(herdr_core::memory_hook::Ask),
    /// `hide mission`: a Conductor's mission record, the core's own.
    Mission(herdr_core::missions::MissionCommand),
    /// `hide plan`: the caller's session's own plan, the core's record.
    Plan(herdr_core::plans::PlanCommand),
    /// `hide agent catalog`: what a machine's agents take, read from the
    /// core's own reports.
    AgentCatalog(Option<String>),
    /// `hide ps [--all]`: the live panes and agents, read from the core's
    /// own state.
    Ps {
        all: bool,
    },
    /// `hide device`: the core's own device registrations.
    Device(herdr_core::device_command::DeviceCommand),
}

impl ScopedRequest {
    /// A request a shell must be open to answer. Delivery and the link
    /// record are the daemon's own and answer without one (B43).
    fn needs_renderer(&self) -> bool {
        !matches!(
            self,
            Self::Delivery(..)
                | Self::Links(..)
                | Self::Memory(..)
                | Self::MemoryHook(..)
                | Self::Mission(..)
                | Self::Plan(..)
                | Self::AgentCatalog(..)
                | Self::Ps { .. }
                | Self::Device(..)
        )
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
    let previous = state.clients.fetch_add(1, Ordering::SeqCst);
    if previous >= MAX_CLIENTS {
        state.clients.fetch_sub(1, Ordering::SeqCst);
        refuse(&mut socket, CloseReason::ClientLimit, Some(previous + 1)).await;
        return;
    }
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
                                herdr_core::workspace_control::Query::Info,
                            )),
                            Some("view_list") => Some(ScopedRequest::Query(
                                herdr_core::workspace_control::Query::ViewList,
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
                            serde_json::from_value::<herdr_core::workspace_control::Action>(
                                value["command"].clone(),
                            )
                            .ok()
                            .map(ScopedRequest::Action)
                        }
                        Some("links") => serde_json::from_value::<
                            herdr_core::links::query::LinksQuery,
                        >(value["query"].clone())
                        .ok()
                        .filter(|query| query.target.valid())
                        .map(ScopedRequest::Links),
                        Some("delivery") => {
                            herdr_core::delivery::Command::from_wire(value["command"].clone())
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
                            herdr_core::memory_command::MemoryCommand,
                        >(value["command"].clone())
                        .ok()
                        .filter(herdr_core::memory_command::MemoryCommand::valid)
                        .map(ScopedRequest::Memory),
                        Some("memory_hook") => herdr_core::memory_hook::Ask::from_wire(&value)
                            .map(ScopedRequest::MemoryHook),
                        Some("mission") => serde_json::from_value::<
                            herdr_core::missions::MissionCommand,
                        >(value["command"].clone())
                        .ok()
                        .filter(herdr_core::missions::MissionCommand::valid)
                        .map(ScopedRequest::Mission),
                        Some("plan") => serde_json::from_value::<herdr_core::plans::PlanCommand>(
                            value["command"].clone(),
                        )
                        .ok()
                        .filter(herdr_core::plans::PlanCommand::valid)
                        .map(ScopedRequest::Plan),
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
                        Some("device") => serde_json::from_value::<
                            herdr_core::device_command::DeviceCommand,
                        >(value["command"].clone())
                        .ok()
                        .filter(herdr_core::device_command::DeviceCommand::valid)
                        .map(ScopedRequest::Device),
                        _ => None,
                    };
                    // A plan carries its whole node list, so a mission
                    // command may use the frame's full size like a letter.
                    if command.is_none()
                        || (value["type"] != "delivery"
                            && value["type"] != "mission"
                            && value["type"] != "plan"
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
                        let browser_control = Arc::clone(&state.browser_control);
                        let herdr_socket = state.herdr_socket.clone();
                        let query_token = token.clone();
                        let command_request_id = request_id.clone();
                        let caller_node = source.as_ref().map(|(node, _)| node.clone());
                        let command_relay = Arc::clone(&chosen);
                        let outcome = if let Some(Err((reason, next_action))) = &relay_slot {
                            Ok(Err(((*reason).to_owned(), *next_action)))
                        } else {
                            tokio::task::spawn_blocking(move || {
                            let cap = registry.validate(&query_token, herdr_socket.as_deref(), &core, attested.as_deref())
                                .map_err(|reason| (reason.to_owned(), crate::pane_auth::refusal_next_action(reason)))?;
                            if command.needs_renderer() && renderers.load(Ordering::SeqCst) == 0 {
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
                                        herdr_core::workspace_control::Query::ViewList)
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
                                        herdr_core::workspace_control::Query::ViewList)
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
                                    herdr_core::links::query::run(&scope, &query, own.as_ref())
                                        .map_err(|(reason, next_action)| (reason.to_owned(), next_action))?
                                }
                                ScopedRequest::Memory(command) => {
                                    core.prepare_memory(&cap.context.device_id, &cap.pane_id, &cap.context, command)
                                        .and_then(herdr_core::memory_command::PreparedMemory::run)
                                        .map_err(|code| {
                                            let next = herdr_core::memory_command::next_action(&code);
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::Mission(command) => {
                                    core.mission(&cap.context.device_id, &cap.pane_id, &cap.context, command)
                                        .map_err(|code| {
                                            let next = herdr_core::missions::next_action(
                                                herdr_core::coordination::split_refusal(&code).0,
                                            );
                                            (code, next)
                                        })?
                                }
                                ScopedRequest::Plan(command) => {
                                    core.plan(&cap.context.device_id, &cap.pane_id, &cap.context, command)
                                        .map_err(|code| {
                                            let next = herdr_core::plans::next_action(
                                                herdr_core::coordination::split_refusal(&code).0,
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
                                ScopedRequest::Device(command) => {
                                    core.device_command(&cap.context.device_id, command)
                                        .map_err(|code| {
                                            let next = herdr_core::device_command::next_action(&code);
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
                                    if desktop_renderers.load(Ordering::SeqCst) == 0 {
                                        result.capabilities.retain(|capability| !capability.starts_with("browser."));
                                        if let Some(views) = &mut result.views {
                                            for view in views {
                                                if let Some(page) = &mut view.page {
                                                    page.state = if page.state == "pending" { "unsupported" } else { "disconnected" };
                                                    page.failure = None;
                                                }
                                            }
                                        }
                                    }
                                    serde_json::to_value(result).map_err(|_| {
                                        ("result_encoding_failed".to_owned(), "Reconnect Hide and retry")
                                    })?
                                }
                                ScopedRequest::Action(action) => {
                                    if matches!(action, herdr_core::workspace_control::Action::OpenBrowser { .. }) && desktop_renderers.load(Ordering::SeqCst) == 0 {
                                        return Err(("browser_unsupported".to_owned(), "Open the Hide desktop app and retry"));
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
                                        herdr_core::workspace_control::ActionPreparation::Cached(result) => {
                                            let result = result.map_err(|refusal| (refusal.reason.to_owned(), refusal.next_action))?;
                                            return serde_json::to_value(result).map_err(|_| {
                                                ("result_encoding_failed".to_owned(), "Reconnect Hide and retry")
                                            });
                                        }
                                        herdr_core::workspace_control::ActionPreparation::Ready => Ok(None),
                                        herdr_core::workspace_control::ActionPreparation::Read(source) => {
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
                                let (reason, next_action) = refusal_answer(reason, next_action);
                                json!({"type":"workspace_result","request_id":request_id,"ok":false,"reason":reason,"next_action":next_action})
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
        eprintln!(
            "{}",
            json!({
                "component": "hided", "kind": "device.file_bytes_ended",
                "device": device,
                "request_id": oldest.request_id.chars().take(LOGGED_PATH_CAP).collect::<String>(),
                "reason": "a newer read for this client took its place",
            })
        );
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
    /// A move of the core, which this process's supervisor takes.
    CoreMove(Value),
    /// A `file_list` for a checkout on an SSH device, answered by its helper.
    DeviceListing(Value),
    /// Comment mode on a browser View, answered when its page is ready.
    BrowserComment(Value),
    /// The operator's comment or Send, which the screen's feedback queue
    /// hands the core in order.
    Feedback(Value),
    Replies(Vec<Message>),
}

/// The file events a checkout on an SSH device answers through that
/// device's helper. This machine's checkout roots and home say nothing about
/// another machine's paths, so these never meet the local boundary: the core
/// finds the checkout and its device in its own catalog, and the helper
/// confines the path to the checkout root it opened, or, for a registration,
/// to that device's own home (`hide_host::register`).
const DEVICE_FILE_EVENTS: [&str; 11] = [
    "create_workspace",
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
    desktop: bool,
    renderer: bool,
) -> Result<ClientAction, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("client json: {error}"))?;
    let mut event = if value.get("schema_version").is_some() && value.get("kind").is_some() {
        value
    } else {
        return Err("expected a core event {schema_version, kind, payload}".to_owned());
    };
    if event.get("kind").and_then(Value::as_str) == Some("file_bytes") {
        return Ok(ClientAction::FileBytes(event));
    }
    if event.get("kind").and_then(Value::as_str) == Some("core_move") {
        return Ok(ClientAction::CoreMove(event));
    }
    if event.get("kind").and_then(Value::as_str) == Some("file_index") {
        return Ok(ClientAction::Replies(handle_file_index(state, &event)));
    }
    match event.get("kind").and_then(Value::as_str) {
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
            let (target, bytes) = terminal_key(&event)?;
            if let KeyTarget::Pane(pane) = &target {
                screen_input(state, connection, pane);
            }
            state.core.terminals.key(target, bytes, unix_ms_now());
            return Ok(ClientAction::Replies(Vec::new()));
        }
        Some("terminal_click") => {
            screen_input(state, connection, &payload_str(&event, "pane_id"));
        }
        // A screen's grid goes on to the core only while the pane follows
        // that screen (`pane_sizes`).
        Some("terminal_resize") => {
            let (pane, size) = terminal_grid(&event)?;
            if state.pane_sizes.resized(connection, &pane, size).is_none() {
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
        Some("desktop_notices") => {
            if !desktop {
                return Err("desktop_notices is a desktop window's".to_owned());
            }
            let Some(wanted) = event
                .get("payload")
                .and_then(|payload| payload.get("wanted"))
                .and_then(Value::as_bool)
            else {
                return Err("desktop_notices.wanted must be a boolean".to_owned());
            };
            if state.desktop_notices.set(connection, wanted) {
                state.mobile.wake_follower();
            }
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
        // The operator's comments and their Send, which writes a letter in
        // the operator's name, belong to a window the operator works in
        // (PRD view-opener-feedback D-17); a pane's scoped connection has no
        // such event at all.
        Some("view_comment" | "view_feedback_send") if !renderer => {
            return Err("View feedback is an operator window's".to_owned());
        }
        Some("view_comment" | "view_feedback_send") => return Ok(ClientAction::Feedback(event)),
        Some("browser_comment") => {
            if !renderer {
                return Err("View feedback is an operator window's".to_owned());
            }
            return Ok(ClientAction::BrowserComment(event));
        }
        // The daemon owns this flag from its renderer count; a window that
        // sent it would pause the readers every other window draws.
        Some("ui_attached") => return Err("ui_attached is sent by the daemon only".to_owned()),
        Some(kind) if kind.starts_with("mobile_") => {
            state.mobile.handle_event(connection, &event)?;
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
            .dispatch(bytes)
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
        .dispatch(bytes)
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
        Err(error) => herdr_core::diagnostic!(json!({
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
    let unavailable = |code: &str, message: String| {
        eprintln!(
            "{}",
            json!({
                "component": "hided",
                "kind": "device.listing_unavailable",
                "device": device,
                "code": code,
                "message": message,
            })
        );
        json!({"type": "directory_unavailable", "payload": {
            "kind": "file_list", "device_id": device, "root_path": folder, "code": code, "message": message,
        }})
    };
    let channel = match core.node_link(&device) {
        Ok(channel) => channel,
        Err(message) => return unavailable("not_ready", message),
    };
    use herdr_core::node_access::LinkError;
    match herdr_core::node_access::list_folder(channel.as_ref(), &root, &relative) {
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
        Err(LinkError::NotConnected(message)) => unavailable("not_ready", message),
        Err(error @ LinkError::Busy) => unavailable("busy", error.to_string()),
        Err(LinkError::Unknown(message)) => unavailable("unknown", message),
        Err(LinkError::Refused(error)) => unavailable("refused", error.message),
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
    owner: &herdr_core::links::ledger::BacklogOriginIdentity,
) {
    let event = json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "backlog_origin_close",
        "payload": owner,
    });
    if let Err(error) = core.dispatch(event.to_string().into_bytes()) {
        eprintln!(
            "{}",
            json!({"component": "hided", "kind": "backlog.origin_release_failed", "connection": connection, "message": error})
        );
    }
}

fn origin_identity(
    event: &Value,
) -> Result<herdr_core::links::ledger::BacklogOriginIdentity, String> {
    let owner: herdr_core::links::ledger::BacklogOriginIdentity = serde_json::from_value(
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
    eprintln!(
        "{}",
        json!({
            "component": "hided",
            "kind": "settings.observation",
            "connection": connection,
            "demand": demand.field(),
            "observing": observing,
        })
    );
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
        eprintln!(
            "{}",
            json!({
                "component": "hided",
                "kind": "settings.observation_failed",
                "connection": connection,
                "message": error,
            })
        );
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
        eprintln!(
            "{}",
            json!({
                "component": "hided",
                "kind": "open.external.refused",
                "reason": reason,
            })
        );
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
    eprintln!(
        "{}",
        json!({
            "component": "hided",
            "kind": "open.external",
            "ok": ok,
            "open_handlers_in_flight": state.opener.in_flight(),
        })
    );
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
    let bracketed = event
        .pointer("/payload/bracketed_paste")
        .and_then(Value::as_bool)
        .unwrap_or(true);
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
            "bracketed_paste": bracketed,
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
    if let Some(device) = event_device(&state.boundary, event) {
        return vec![Message::Text(
            device_file_index(state, &device, &root, &query)
                .to_string()
                .into(),
        )];
    }
    let found = match state.boundary.open_directory(Path::new(&root), &root) {
        Err(_) if roots_current(&state.roots) => {
            state.boundary.open_directory(Path::new(&root), &root)
        }
        found => found,
    };
    let Ok((known, opened)) = found else {
        return vec![Message::Text(
            refused("file_index", &root, Refusal::OutsideCheckout)
                .to_string()
                .into(),
        )];
    };
    let root_path = hide_platform::path::to_wire_lossy(&known);
    let walk_root = known.clone();
    let answer = state.index.query(
        state.boundary.node().as_str(),
        &root_path,
        &query,
        move || {
            Ok(hide_host::index::walk(
                &cap_std::fs::Dir::from_std_file(opened),
                &walk_root,
            ))
        },
    );
    vec![Message::Text(
        index_result(
            state.boundary.node().as_str(),
            &root_path,
            &query,
            answer,
            |relative| {
                let relative = hide_platform::path::RelPath::parse(relative).ok()?;
                let path = hide_platform::path::wire_join(&root_path, &relative);
                state.boundary.resolve_target(&path).ok().map(|_| path)
            },
        )
        .to_string()
        .into(),
    )]
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
    let core = Arc::clone(&state.core);
    let (walk_device, walk_root) = (device.to_owned(), root.to_owned());
    let answer = state.index.query(device, root, query, move || {
        let channel = core.node_link(&walk_device)?;
        herdr_core::node_access::index_root(channel.as_ref(), &walk_root).map_err(|error| {
            eprintln!(
                "{}",
                json!({
                    "component": "hided",
                    "kind": "device.index_unavailable",
                    "device": walk_device,
                    "message": error.to_string(),
                })
            );
            error.to_string()
        })
    });
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

/// A read the daemon could not serve as bytes: the file is past the read cap,
/// or the disk refused it. The client shows one line and offers no retry.
fn file_bytes_error(request_id: &str, path: &str, reason: &str) -> Message {
    eprintln!(
        "{}",
        json!({
            "component": "hided",
            "kind": "file_bytes.failed",
            "reason": reason,
            "path": path.chars().take(LOGGED_PATH_CAP).collect::<String>(),
        })
    );
    Message::Text(
        json!({
            "type": "file_bytes_error",
            "payload": {"request_id": request_id, "path": path, "reason": reason},
        })
        .to_string()
        .into(),
    )
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
async fn stream_device_file_bytes(
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
            let channel = core.node_link(&read_device)?;
            herdr_core::node_access::read_bytes(
                channel.as_ref(),
                &read_root,
                &read_relative,
                cursor,
                wanted,
            )
            .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result);
        let range = match range {
            Ok(range) => range,
            Err(message) => {
                eprintln!(
                    "{}",
                    json!({
                        "component": "hided",
                        "kind": "device.file_bytes_failed",
                        "device": device,
                        "message": message,
                    })
                );
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

/// A `file_bytes` read keeps only one bounded chunk in memory. Sending each
/// frame before reading the next gives the socket backpressure even when all
/// eight authenticated clients ask for the maximum range together.
async fn send_file_bytes(
    socket: &mut WebSocket,
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
    socket: &mut WebSocket,
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
                .send(Message::Text(
                    refused("file_bytes", &path, refusal).to_string().into(),
                ))
                .await
                .map_err(|_| ());
        }
    };
    let wanted = length.unwrap_or_else(|| total.saturating_sub(offset.min(total)));
    if wanted > boundary::MAX_FILE_BYTES {
        return socket
            .send(file_bytes_error(&request_id, &path, "too_large"))
            .await
            .map_err(|_| ());
    }
    let mut file = tokio::fs::File::from_std(file);
    let start = offset.min(total);
    if start > 0 && file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return socket
            .send(file_bytes_error(&request_id, &path, "read_failed"))
            .await
            .map_err(|_| ());
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
                    .send(file_bytes_error(&request_id, &path, "read_failed"))
                    .await
                    .map_err(|_| ());
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
            .send(Message::Binary(bytes_frame(&header, &buffer).into()))
            .await
            .map_err(|_| ())?;
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
/// not resolve under home is answered with a `path_refused` frame and never
/// reaches the core. A `remote_file_list` names a path on a remote machine,
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
        "create_workspace" => rewrite(event, &kind, "path", |raw| boundary.resolve_workspace(raw)),
        "clone_repository" => clone_admit(boundary, event, &kind),
        "file_list" => explorer_listing(boundary, event, &kind),
        "file_open" | "reveal_path" => explorer_open(boundary, event, &kind),
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
        // The shell's attachment events name files the operator's machine
        // shows it; hided is their only producer for a web client (which
        // stages bytes instead), so a client that sends one is naming an
        // arbitrary path and never reaches the core.
        "terminal_attachment" | "terminal_attachment_ready" | "terminal_attachment_action" => {
            rejected(kind.as_str())
        }
        _ => None,
    }
}

/// Runs the boundary on a client event. The roots follow the core on their own
/// reads, so an event can name a checkout this client read in a snapshot the
/// boundary has not applied yet: after a launch, a checkout the core learns
/// from Herdr's first sync reaches both on separate reads. A refusal as
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
        eprintln!(
            "{}",
            json!({
                "component": "hided",
                "kind": "boundary.roots_caught_up",
                "event": event.get("kind").and_then(Value::as_str).unwrap_or(""),
            })
        );
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
fn roots_current(roots: &crate::RootFollower) -> bool {
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
fn device_root_known(
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
pub(crate) fn terminal_key(event: &Value) -> Result<(KeyTarget, Vec<u8>), String> {
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
    Ok((target, bytes))
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
    let event = json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "terminal_resize",
        "payload": {"pane_id": pane, "cols": size.cols, "rows": size.rows},
    });
    if let Err(error) = state.core.dispatch(event.to_string().into_bytes()) {
        herdr_core::diagnostic!(json!({
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
    eprintln!(
        "{}",
        json!({
            "component": "hided",
            "kind": "client.rejected",
            "event": kind,
            "reason": "daemon_only_event",
        })
    );
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
        "parent": parent, "name": name,
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
    if !crate::file_url::is_file_url(&raw) {
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
    let Some((path, suffix)) = crate::file_url::file_path(&raw) else {
        return Some(refused(kind, &raw, Refusal::InvalidPath));
    };
    let checked = if kind == "browser_state" {
        boundary.resolve_browser_report(&path)
    } else {
        boundary.resolve_target(&path)
    };
    match checked {
        Ok(real) => {
            let url = crate::file_url::file_url(&hide_platform::path::to_wire_lossy(&real), suffix);
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
    eprintln!(
        "{}",
        json!({
            "component": "hided",
            "kind": "path.refused",
            "event": kind,
            "reason": refusal.code(),
            "path": logged,
            "path_truncated": logged.len() < path.len(),
        })
    );
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
) -> Result<herdr_core::SnapshotDeltaPayload, ()> {
    let core = Arc::clone(&state.core);
    tokio::task::spawn_blocking(move || core.snapshot(have_revision))
        .await
        .map_err(|error| log_snapshot_failure("reader", &error.to_string()))?
        .map_err(|error| log_snapshot_failure("core", &error))
}

/// A frame the daemon could not produce ends the client's loop, so the
/// client sees a bare close; this record is the only trace of why.
fn log_snapshot_failure(stage: &str, message: &str) {
    eprintln!(
        "{}",
        json!({
            "component": "hided",
            "kind": "ws.snapshot_failed",
            "stage": stage,
            "message": message,
        })
    );
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
            payload: herdr_core::SnapshotDeltaWire<'a>,
        }
        serde_json::to_string(&Envelope {
            kind: match kind {
                FrameKind::Snapshot => "snapshot",
                FrameKind::Delta => "delta",
            },
            payload: if granular {
                herdr_core::SnapshotDeltaWire::granular(&payload)
            } else {
                herdr_core::SnapshotDeltaWire::borrow(&payload)
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

/// Tells the core whether any window draws its snapshot.
fn dispatch_ui_attached(core: &CoreHandle, attached: bool) {
    let event = json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "ui_attached",
        "payload": { "attached": attached },
    });
    if let Err(error) = core.dispatch(event.to_string().into_bytes()) {
        eprintln!(
            "{}",
            json!({"component": "hided", "kind": "ui_attached.dispatch_failed", "attached": attached, "message": error})
        );
    }
}

/// A window of this machine, counted while it is open; the first to open
/// and the last to close tell every node whether one draws its panes.
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

fn client_gone(state: &AppState, connection: u64, renderer: bool, desktop: bool) {
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
    for (pane, size) in state.pane_sizes.left(connection) {
        send_grid(state, &pane, size);
    }
    for demand in [Demand::Settings, Demand::Start, Demand::RequestView] {
        demand.of(state).release(connection, |observing| {
            dispatch_observation(&state.core, demand, connection, observing)
        });
    }
    state.origin_holds.release(connection, |key| {
        close_backlog_origin(&state.core, connection, key)
    });
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
    eprintln!(
        "{}",
        json!({
            "component": "hided",
            "kind": "ws.refused",
            "reason": reason.name(),
            "code": reason.code(),
            "clients": extra,
        })
    );
}

/// Mounts the core's routes on `seat` and starts the clock that ends the
/// daemon once no screen is left; the role aborts the clock when it ends.
pub fn mount(seat: &crate::seat::SeatParts, state: AppState) -> tokio::task::JoinHandle<()> {
    let shutdown = state.shutdown.clone();
    let mobile = Arc::clone(&state.mobile);
    let idle_task = watch_idle(
        Idle {
            idle_secs: state.idle_secs,
            keep_alive: state.keep_alive,
            clients: Arc::clone(&state.clients),
            last_client_gone: Arc::clone(&state.last_client_gone),
            shutdown,
            herdr_socket: state.herdr_socket.clone(),
        },
        // Mobile on with a phone paired: the phone may come back at any
        // time and push has to keep going (PRD D-10). The idle clock starts
        // over when that ends.
        move || mobile.keep_alive(),
    );
    seat.mount(router(state));
    idle_task
}

/// What decides when a daemon with no screen ends.
pub(crate) struct Idle {
    pub idle_secs: u64,
    pub keep_alive: bool,
    pub clients: Arc<AtomicUsize>,
    pub last_client_gone: Arc<Mutex<Instant>>,
    pub shutdown: crate::ending::Ending,
    pub herdr_socket: Option<PathBuf>,
}

/// Ends the daemon once it had no screen for `idle_secs` while `held` was
/// false and its Herdr server could not be reached; never with
/// `keep_alive`. Aborted by its owner when the daemon stops another way.
pub(crate) fn watch_idle(
    idle: Idle,
    held: impl Fn() -> bool + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if idle.keep_alive {
            return;
        }
        let mut herdr_probed: Option<Instant> = None;
        let mut herdr_up = false;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if idle.clients.load(Ordering::SeqCst) != 0 {
                continue;
            }
            if held() {
                *idle.last_client_gone.lock().expect("client timestamp") = Instant::now();
                continue;
            }
            // The daemon lives as long as its Herdr server: labels keep
            // being made with no window open (PRD labels-in-hided D-18), and
            // a screen machine's agents keep reaching their core through its
            // node (PRD core-host-node-remote-core D-08). The idle clock runs
            // only while the server is unreachable, so a restart or a live
            // handoff, which brings it back within the idle window, never
            // ends the daemon.
            if let Some(socket) = idle.herdr_socket.clone()
                && herdr_probed.is_none_or(|at| at.elapsed() >= HERDR_PROBE_INTERVAL)
            {
                herdr_probed = Some(Instant::now());
                herdr_up = tokio::task::spawn_blocking(move || herdr_reachable(&socket))
                    .await
                    .unwrap_or(false);
            }
            if herdr_up {
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

/// How often an idle daemon asks whether its Herdr server is still there.
const HERDR_PROBE_INTERVAL: Duration = Duration::from_secs(30);
const HERDR_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Whether the Herdr server at `socket` answers; a refusal is an answer.
pub(crate) fn herdr_reachable(socket: &std::path::Path) -> bool {
    matches!(
        hide_herdr_client::request_with_timeout(socket, "ping", json!({}), HERDR_PROBE_TIMEOUT),
        Ok(_) | Err(hide_herdr_client::ApiError::Remote { .. })
    )
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
            "shell_starting\u{1f}[oh-my-zsh] Would you like to update? [Y/n]".to_owned(),
            delivery_next_action("shell_starting"),
        );
        assert_eq!(code, "shell_starting");
        assert!(
            next.ends_with("[oh-my-zsh] Would you like to update? [Y/n]"),
            "{next}"
        );
        for code in [
            "spawn_busy",
            "machine_not_permitted",
            "machine_unavailable",
            "repository_unavailable",
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
            "codex_capability_unknown",
        ] {
            let (reason, next) = refusal_answer(code.to_owned(), delivery_next_action(code));
            assert_eq!(reason, code);
            assert_ne!(next, delivery_next_action("other"), "{code}");
        }
    }

    /// A caller refused for lacking an agent pane is told where to run the
    /// command; every other delivery refusal keeps the retry advice.
    #[test]
    fn a_missing_agent_pane_names_where_to_run_the_command() {
        let next = delivery_next_action("agent_pane_required");
        assert!(next.contains("pane where the agent is running"), "{next}");
        assert!(next.contains("--no-daemon"), "{next}");
        assert_eq!(
            delivery_next_action("caller_identity_conflict"),
            "Check the current agent pane and retry the same intent"
        );
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
        use herdr_core::links::query::{LinksQuery, QueryTarget};
        let links = ScopedRequest::Links(LinksQuery {
            target: QueryTarget::Pr { number: 1 },
            all_projects: false,
        });
        assert!(!links.needs_renderer());
        assert!(
            !ScopedRequest::Memory(herdr_core::memory_command::MemoryCommand::Show { number: 1 })
                .needs_renderer()
        );
        let hook = herdr_core::memory_hook::Ask::from_wire(&json!({
            "event": "SessionStart", "runtime": "codex", "session": "s-1",
        }))
        .unwrap();
        assert!(!ScopedRequest::MemoryHook(hook).needs_renderer());
        assert!(ScopedRequest::Query(herdr_core::workspace_control::Query::Info).needs_renderer());
        assert!(ScopedRequest::BrowserConnect(None).needs_renderer());
    }

    #[test]
    fn token_compare_needs_the_whole_token() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("ab", "abc"));
        assert!(!token_matches("abd", "abc"));
        assert!(!token_matches("", "abc"));
    }

    /// After a launch, a checkout the core learns from Herdr's first sync
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
        let checked =
            crate::file_url::file_url(&checkout.join("보고서.html").display().to_string(), "#top");
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

        let outside =
            crate::file_url::file_url(&home.path().join("secret.html").display().to_string(), "");
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
        let native_url = crate::file_url::file_url(&native.display().to_string(), "#loaded");
        let checked_url = crate::file_url::file_url(
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
        let outside =
            crate::file_url::file_url(&home.path().join("secret.html").display().to_string(), "");
        std::os::unix::fs::symlink(
            home.path().join("secret.html"),
            checkout.join("escape.html"),
        )
        .unwrap();
        let escape = crate::file_url::file_url(
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
}
