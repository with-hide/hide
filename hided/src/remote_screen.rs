//! Another machine's view of this core, and this machine's view of another
//! (PRD D-53, D-67, B17, B20, B63).
//!
//! On the host, a guest the operator granted opens streams of two kinds on
//! the one connection between the two machines, each carrying HTTP and
//! nothing else: no `/health`, no other route (D-67). A
//! [`StreamKind::Screen`] stream carries the view's socket (`/ws`), its
//! control and terminal frames. A [`StreamKind::ScreenBulk`] stream, behind
//! everything else, carries this daemon's page (`/`), its assets
//! (`/assets/*`) and the view's file bytes (`/ws/bulk`, written from their
//! own task), so neither a page load nor a file read holds the socket's
//! frames. The socket is a screen of the `remote` kind
//! (`event_class::Client::Remote`): it attaches with the grant's id and
//! generation in place of this machine's token, draws terminals under the
//! hub's remote limits (D-84) and sends only what the class table admits. A guest holds one view at a
//! time: a new one takes the place of the one before, and the views of all
//! guests are [`MAX_VIEWS`] at most, apart from this machine's own windows
//! (`MAX_CLIENTS`). Every stream runs under the grant and ends when it
//! changes.
//!
//! On the guest, the desktop app reaches the host through this daemon:
//! `/peer-screen/<device>/…?key=<host key>` (this machine's token) answers
//! the page and its assets from the host's bulk streams, keeping each asset
//! by the host's key and build,
//! and `/peer-screen/<device>/ws?key=<host key>` joins one socket to the
//! host's two streams. The desktop opens that socket only while the tile
//! shows. Each request names the key its page was opened for, and one whose
//! device has another host now is refused: a page one host's key loaded
//! never reaches a host that took the same device id after it.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use axum::Router;
use axum::body::Body;
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use hide_peer::protocol::hello::GrantState;
use hide_peer::protocol::{PeerKey, StreamKind};
use hide_peer::store::KnownState;
use serde_json::json;
use tokio::sync::{Notify, mpsc};
use tokio_tungstenite::tungstenite;

use crate::peer_host::{Incoming, Link, PeerHost};
use crate::server::AppState;

/// The views of every guest at once. Each guest holds at most one.
pub const MAX_VIEWS: usize = 4;
/// The header that ties a view's bulk stream to its screen stream.
const VIEW_HEADER: &str = "x-hide-view";
/// The longest asset a guest takes from a host.
const MAX_ASSET: usize = 32 * 1024 * 1024;
/// The assets a guest keeps of hosts' builds, in bytes; past it the oldest
/// go.
const ASSET_CACHE: usize = 48 * 1024 * 1024;
/// Page and asset requests a guest has open on a host at once; the view's
/// bulk socket takes another of the kind's streams, and a second view's
/// one more while it takes the first's place.
const ASSET_STREAMS: usize = 6;
/// The file reads one view runs at once; one more is answered `busy`.
pub const VIEW_READS: usize = 4;
/// How long a guest waits for a host's answer to open a view.
const OPEN_WITHIN: std::time::Duration = std::time::Duration::from_secs(15);
/// How long one page or asset may take to arrive whole: `MAX_ASSET` at
/// about 0.5 MiB/s, so a host that sends slowly or stops holds a request's
/// place no longer than this.
const ASSET_WITHIN: std::time::Duration = std::time::Duration::from_secs(60);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The guest a screen stream came from, as this host granted it: what its
/// view attaches with in place of this machine's token.
#[derive(Clone, Debug)]
pub struct Viewer {
    pub key: PeerKey,
    pub device: String,
    pub grant: String,
    pub generation: u64,
    /// The view the guest named, tying its bulk stream to its socket.
    pub view: String,
}

/// A view's place among the guests' views: it ends when a newer view of the
/// same guest takes it, and frees its place when dropped.
pub struct Seat {
    screens: Arc<RemoteScreens>,
    key: PeerKey,
    id: u64,
    pub viewer: Viewer,
    taken: Arc<Notify>,
}

impl Seat {
    /// Resolves when a newer view of the same guest took this one's place.
    pub async fn taken(&self) {
        self.taken.notified().await;
    }

    /// The sender of this view's bulk stream, when the guest opened one.
    pub fn bulk(&self) -> Option<mpsc::Sender<Message>> {
        lock(&self.screens.bulks)
            .get(&(self.key, self.viewer.view.clone()))
            .cloned()
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        let mut seats = lock(&self.screens.seats);
        if seats.get(&self.key).is_some_and(|held| held.0 == self.id) {
            seats.remove(&self.key);
        }
        self.screens.views.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Why a view was not seated.
#[derive(Debug, Eq, PartialEq)]
pub enum SeatRefused {
    Full,
}

/// One host's asset: the host by its key, then its build and the path. The
/// build and the path are what that host says, so two hosts never share an
/// entry: one machine's file is never served as another's page, which would
/// run in that machine's origin and on its socket.
#[derive(Clone, PartialEq, Eq, Hash)]
struct AssetKey {
    host: PeerKey,
    build: String,
    path: String,
}

#[derive(Default)]
struct AssetCache {
    entries: HashMap<AssetKey, (String, Bytes)>,
    order: VecDeque<AssetKey>,
    bytes: usize,
}

/// What an entry costs beyond its body and its names: the map's and the
/// queue's places for it, so an empty answer is never free.
const ENTRY_COST: usize = 256;

impl AssetCache {
    fn get(&self, key: &AssetKey) -> Option<(String, Bytes)> {
        self.entries.get(key).cloned()
    }

    /// What `key` with `kind` and `body` holds of the cache's bound.
    fn cost(key: &AssetKey, kind: &str, body: &Bytes) -> usize {
        body.len() + key.build.len() + key.path.len() + kind.len() + ENTRY_COST
    }

    fn put(&mut self, key: AssetKey, kind: String, body: Bytes) {
        let cost = Self::cost(&key, &kind, &body);
        if cost > ASSET_CACHE || self.entries.contains_key(&key) {
            return;
        }
        self.bytes += cost;
        self.entries.insert(key.clone(), (kind, body));
        self.order.push_back(key);
        while self.bytes > ASSET_CACHE {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some((kind, body)) = self.entries.remove(&oldest) {
                self.bytes -= Self::cost(&oldest, &kind, &body);
            }
        }
    }
}

/// Both sides of other machines' views.
pub struct RemoteScreens {
    /// The view each guest holds now, by its seat's id.
    seats: Mutex<HashMap<PeerKey, (u64, Arc<Notify>)>>,
    /// Every view seated now.
    views: AtomicUsize,
    next: AtomicU64,
    /// Each view's bulk stream, by its guest and the view it named.
    bulks: Mutex<HashMap<(PeerKey, String), mpsc::Sender<Message>>>,
    /// The daemon's state, set once it is built; a stream that comes before
    /// is closed and the guest opens it again.
    state: OnceLock<AppState>,
    /// The guest side's assets of hosts' builds.
    assets: Mutex<AssetCache>,
    /// The guest side's page and asset requests open now, per host: a host
    /// that answers slowly or not at all holds only its own places. A host
    /// with none open is dropped from the map at the next request.
    asset_places: Mutex<HashMap<PeerKey, Arc<tokio::sync::Semaphore>>>,
    /// The guest side's view of each host open now, by device and key: the
    /// view a page's route question names, so the host answers it only
    /// while that view lives.
    open_views: Mutex<HashMap<(String, PeerKey), String>>,
}

impl RemoteScreens {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            seats: Mutex::default(),
            views: AtomicUsize::new(0),
            next: AtomicU64::new(0),
            bulks: Mutex::default(),
            state: OnceLock::new(),
            assets: Mutex::default(),
            asset_places: Mutex::default(),
            open_views: Mutex::default(),
        })
    }

    /// How many views are seated now; they do not count against this
    /// machine's windows.
    /// A place for one page or asset request on `host`, waiting at most
    /// `OPEN_WITHIN` for one of its own to free.
    async fn asset_place(&self, host: PeerKey) -> Option<tokio::sync::OwnedSemaphorePermit> {
        let places = {
            let mut places = lock(&self.asset_places);
            // A host's places are kept while a request holds one: each
            // request holds the semaphore it took from.
            places.retain(|_, held| Arc::strong_count(held) > 1);
            Arc::clone(
                places
                    .entry(host)
                    .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(ASSET_STREAMS))),
            )
        };
        tokio::time::timeout(OPEN_WITHIN, places.acquire_owned())
            .await
            .ok()?
            .ok()
    }

    pub fn seated(&self) -> usize {
        self.views.load(Ordering::SeqCst)
    }

    /// Takes the two kinds of stream a guest opens for its view. Called once,
    /// before the endpoint opens.
    pub fn serve(self: &Arc<Self>, peers: &PeerHost) {
        for kind in [StreamKind::Screen, StreamKind::ScreenBulk] {
            let screens = Arc::clone(self);
            peers.handle(kind, move |incoming| {
                let screens = Arc::clone(&screens);
                async move { screens.stream(kind, incoming).await }
            });
        }
    }

    /// The daemon's state, once built.
    pub fn attach(&self, state: AppState) {
        let _ = self.state.set(state);
    }

    /// Seats a view of `viewer`'s guest, taking the place of the one it had.
    pub fn seat(self: &Arc<Self>, viewer: Viewer) -> Result<Seat, SeatRefused> {
        let mut seats = lock(&self.seats);
        let replacing = seats.remove(&viewer.key);
        let held = self.views.load(Ordering::SeqCst) - usize::from(replacing.is_some());
        if held >= MAX_VIEWS {
            if let Some(replacing) = replacing {
                seats.insert(viewer.key, replacing);
            }
            return Err(SeatRefused::Full);
        }
        if let Some((_, taken)) = replacing {
            taken.notify_one();
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let taken = Arc::new(Notify::new());
        seats.insert(viewer.key, (id, Arc::clone(&taken)));
        self.views.fetch_add(1, Ordering::SeqCst);
        Ok(Seat {
            screens: Arc::clone(self),
            key: viewer.key,
            id,
            viewer,
            taken,
        })
    }

    /// One stream a guest opened: HTTP to the view's routes, as the guest
    /// this host granted.
    async fn stream(self: Arc<Self>, kind: StreamKind, incoming: Incoming) {
        let Some(state) = self.state.get().cloned() else {
            return;
        };
        let key = incoming.link.key();
        let Some(grant) = state
            .peers
            .peers()
            .and_then(|peers| peers.grants(|grants, _| grants.current(&key).cloned()))
        else {
            return;
        };
        let viewer = Viewer {
            key,
            device: grant.device,
            grant: grant.id,
            generation: grant.generation,
            view: String::new(),
        };
        let route = ViewRoute {
            state,
            screens: Arc::clone(&self),
            viewer,
        };
        // The socket's frames on the screen kind; the page, its assets and
        // the file bytes behind them on the bulk kind (D-67).
        let router = match kind {
            StreamKind::ScreenBulk => Router::new()
                .route("/", get(view_asset))
                .route("/assets/{*path}", get(view_asset))
                .route("/ws/bulk", get(bulk_upgrade))
                .route("/page-route", axum::routing::post(page_route)),
            _ => Router::new().route("/ws", get(view_upgrade)),
        }
        .fallback(|| async { StatusCode::NOT_FOUND })
        .with_state(route);
        let io = tokio::io::join(incoming.stream.recv, incoming.stream.send);
        let service = hyper_util::service::TowerToHyperService::new(router);
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(hyper_util::rt::TokioIo::new(io), service)
            .with_upgrades()
            .await;
    }
}

#[derive(Clone)]
struct ViewRoute {
    state: AppState,
    screens: Arc<RemoteScreens>,
    viewer: Viewer,
}

fn view_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(VIEW_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|view| !view.is_empty() && view.len() <= 64)
        .map(str::to_owned)
}

async fn view_asset(uri: axum::http::Uri, State(route): State<ViewRoute>) -> Response {
    crate::server::ui_asset(route.state.ui_dir.as_deref(), uri.path()).await
}

/// What a guest asks before it loads one of this machine's browser displays
/// in its view: the display by its Workspace, its id and its load.
#[derive(serde::Deserialize)]
struct PageRouteAsk {
    device: String,
    checkout: String,
    view: String,
    load: u64,
}

/// The longest field of a route question.
const MAX_ROUTE_FIELD: usize = 8192;

fn refusal(status: StatusCode, reason: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        json!({"reason": reason}).to_string(),
    )
        .into_response()
}

/// `POST /page-route` on a view's bulk stream: where a browser display
/// the guest draws in its view of this machine loads from, decided by this
/// core as it decides it for its own window (`browser_route_source`). A
/// page of another of this machine's devices on its loopback or its files
/// is refused, since only that device reaches them; this machine's own
/// files are not served to a view yet; every other page loads its address,
/// and the answer says whether it is this machine's own, whose loopback the
/// page may reach through this machine.
async fn page_route(headers: HeaderMap, State(route): State<ViewRoute>, body: Bytes) -> Response {
    let live = view_of(&headers)
        .map(|view| lock(&route.screens.bulks).contains_key(&(route.viewer.key, view)));
    let core = Arc::clone(&route.state.core);
    answer_page_route(live, &body, move |ask| {
        let own = core.node().as_str().to_owned();
        core.browser_route_source(&ask.device, &ask.checkout, &ask.view, ask.load)
            .map(|source| (source, own))
    })
    .await
}

/// The core's answer to where a display loads from, with the core's own
/// node id.
type RouteSourceOf = Result<
    (
        Option<hide_core::workspace_control::BrowserRouteSource>,
        String,
    ),
    String,
>;

/// `/page-route`'s whole answer once the view the request named is judged
/// (`None`: it named none; `Some(false)`: not one of this guest's live
/// views): the question read and bounded, the core asked through
/// `source_of` on a blocking task, and its source decided.
async fn answer_page_route<F>(view_live: Option<bool>, body: &[u8], source_of: F) -> Response
where
    F: FnOnce(PageRouteAsk) -> RouteSourceOf + Send + 'static,
{
    match view_live {
        None => return StatusCode::BAD_REQUEST.into_response(),
        Some(false) => return refusal(StatusCode::GONE, "view_unavailable"),
        Some(true) => {}
    }
    let Ok(ask) = serde_json::from_slice::<PageRouteAsk>(body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if [&ask.device, &ask.checkout, &ask.view]
        .iter()
        .any(|field| field.is_empty() || field.len() > MAX_ROUTE_FIELD)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let load = ask.load;
    let (source, own_node) = match tokio::task::spawn_blocking(move || source_of(ask)).await {
        Ok(Ok(found)) => found,
        _ => return refusal(StatusCode::SERVICE_UNAVAILABLE, "core_unavailable"),
    };
    let Some(source) = source else {
        return refusal(StatusCode::GONE, "view_unavailable");
    };
    let own = source.device_id == own_node;
    let file = source
        .url
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"));
    if !own && (file || crate::browser_routes::loopback_page(&source.url)) {
        return refusal(StatusCode::FORBIDDEN, "remote_device_page_unavailable");
    }
    if file {
        return refusal(StatusCode::FORBIDDEN, "remote_file_unavailable");
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json!({"url": source.url, "source_url": source.url, "load": load, "own": own}).to_string(),
    )
        .into_response()
}

async fn view_upgrade(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(route): State<ViewRoute>,
) -> Response {
    let Some(view) = view_of(&headers) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let ViewRoute {
        state,
        screens,
        mut viewer,
    } = route;
    viewer.view = view;
    ws.on_upgrade(move |socket| crate::server::remote_client_loop(socket, state, screens, viewer))
}

async fn bulk_upgrade(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(route): State<ViewRoute>,
) -> Response {
    let Some(view) = view_of(&headers) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let key = route.viewer.key;
    let screens = route.screens;
    ws.on_upgrade(move |socket| async move {
        let (mut sink, mut source) = socket.split();
        let (sender, mut frames) = mpsc::channel::<Message>(2);
        {
            let mut bulks = lock(&screens.bulks);
            // One bulk per guest: a newer view's takes the place of the last.
            bulks.retain(|(guest, _), _| *guest != key);
            bulks.insert((key, view.clone()), sender);
        }
        loop {
            tokio::select! {
                frame = frames.recv() => {
                    let Some(frame) = frame else { break };
                    if sink.send(frame).await.is_err() {
                        break;
                    }
                }
                incoming = source.next() => {
                    // The guest writes nothing on it; its end ends the view's
                    // bulk.
                    if !matches!(incoming, Some(Ok(_))) {
                        break;
                    }
                }
            }
        }
        lock(&screens.bulks).remove(&(key, view));
    })
}

// The guest side.

/// Why a host's view could not be opened, as the desktop app reads it.
fn unavailable(reason: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "application/json")],
        json!({"reason": reason}).to_string(),
    )
        .into_response()
}

/// The key a request's page was opened for, its query's one `key=<hex>`;
/// `None` when it names none, or anything else.
fn asked_key(query: Option<&str>) -> Option<PeerKey> {
    PeerKey::from_hex(query?.strip_prefix("key=")?).ok()
}

/// The link to `device` while this computer may open it under `key`, with
/// the build it says it runs. A device whose host has another key now is
/// not paired under the one asked for, so a page the earlier host loaded
/// never reaches the later one (security pre-review S1).
fn host_link(
    state: &AppState,
    device: &str,
    key: &PeerKey,
) -> Result<(Link, String), &'static str> {
    host_link_on(&state.peers, device, key)
}

/// [`host_link`] on the endpoint itself, for what runs beside the daemon's
/// state (`screen_net`).
pub(crate) fn host_link_on(
    host: &PeerHost,
    device: &str,
    key: &PeerKey,
) -> Result<(Link, String), &'static str> {
    let peers = host.peers().ok_or("not_connected")?;
    let host = peers
        .hosts()
        .into_iter()
        .find(|host| host.device == device && host.key == *key)
        .ok_or("not_paired")?;
    match host.state {
        KnownState::Active => {}
        KnownState::PausedByMe => return Err("paused"),
        KnownState::Confirming => return Err("not_connected"),
        KnownState::RevokePending | KnownState::Unpaired => return Err("no_grant"),
    }
    let link = peers.link(&host.key).ok_or("not_connected")?;
    match link.granted() {
        GrantState::Active => {}
        GrantState::Paused => return Err("paused_there"),
        _ => return Err("no_grant"),
    }
    let build = link.their_hello().build.clone();
    Ok((link, build))
}

/// A stream of a host's view: its two halves, which end with the grant.
type ViewIo = tokio::io::Join<
    hide_peer::gate::Guarded<<crate::peer_host::Conn as hide_peer::PeerConn>::Recv>,
    hide_peer::gate::Guarded<<crate::peer_host::Conn as hide_peer::PeerConn>::Send>,
>;

async fn open_stream(link: &Link, kind: StreamKind) -> Result<ViewIo, &'static str> {
    let stream = tokio::time::timeout(OPEN_WITHIN, link.open(kind))
        .await
        .map_err(|_| "not_connected")?
        .map_err(|error| match error {
            hide_peer::peers::OpenError::NoGrant(_) => "no_grant",
            hide_peer::peers::OpenError::Refused(_) => "refused",
            _ => "not_connected",
        })?;
    Ok(tokio::io::join(stream.recv, stream.send))
}

/// `GET /peer-screen/<device>/?key=<host key>`: the host's page, for the
/// desktop app's view of it.
pub async fn guest_page(
    State(state): State<AppState>,
    Path(device): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    guest_file(state, device, String::new(), query, headers).await
}

/// `GET /peer-screen/<device>/<path>?key=<host key>`: one of the host's assets.
pub async fn guest_asset(
    State(state): State<AppState>,
    Path((device, path)): Path<(String, String)>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    guest_file(state, device, path, query, headers).await
}

async fn guest_file(
    state: AppState,
    device: String,
    path: String,
    query: Option<String>,
    headers: HeaderMap,
) -> Response {
    if !crate::server::bearer_matches(&headers, &state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let path = format!("/{path}");
    if path != "/" && path != "/index.html" && !path.starts_with("/assets/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(host) = asked_key(query.as_deref()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let (link, build) = match host_link(&state, &device, &host) {
        Ok(found) => found,
        Err(reason) => return unavailable(reason),
    };
    let screens = Arc::clone(&state.remote_screens);
    let key = AssetKey {
        host,
        build,
        path: path.clone(),
    };
    // Assets carry their content in their names; the page itself is read
    // each time, so a host's new build is drawn from its own page.
    let cached = path.starts_with("/assets/");
    if cached && let Some((kind, body)) = lock(&screens.assets).get(&key) {
        return asset_response(&kind, body);
    }
    let Some(_place) = screens.asset_place(host).await else {
        return unavailable("not_connected");
    };
    let io = match open_stream(&link, StreamKind::ScreenBulk).await {
        Ok(io) => io,
        Err(reason) => return unavailable(reason),
    };
    match fetch(io, &path).await {
        Ok((status, kind, body)) if status == StatusCode::OK => {
            if cached {
                lock(&screens.assets).put(key, kind.clone(), body.clone());
            }
            asset_response(&kind, body)
        }
        Ok((status, ..)) => status.into_response(),
        Err(()) => unavailable("not_connected"),
    }
}

fn asset_response(kind: &str, body: Bytes) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, kind)
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// One GET over a host's screen stream.
async fn fetch<IO>(io: IO, path: &str) -> Result<(StatusCode, String, Bytes), ()>
where
    IO: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use http_body_util::{BodyExt, Empty, Limited};
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(io))
            .await
            .map_err(drop)?;
    let driver = crate::owned_task::AbortOnDrop(
        tokio::spawn(async move {
            let _ = connection.await;
        })
        .abort_handle(),
    );
    let request = hyper::Request::get(path)
        .header(header::HOST, "hide")
        .body(Empty::<Bytes>::new())
        .map_err(drop)?;
    let response = tokio::time::timeout(OPEN_WITHIN, sender.send_request(request))
        .await
        .map_err(drop)?
        .map_err(drop)?;
    let status = response.status();
    let kind = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let body = tokio::time::timeout(
        ASSET_WITHIN,
        Limited::new(response.into_body(), MAX_ASSET).collect(),
    )
    .await
    .map_err(drop)?
    .map_err(drop)?
    .to_bytes();
    drop(driver);
    Ok((status, kind, body))
}

/// `GET /peer-screen/<device>/ws?key=<host key>`: the desktop app's socket
/// to the host's view, joined from the host's two streams: the socket's
/// frames and the file bytes beside them. It ends when either does, or the
/// desktop's.
pub async fn guest_socket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(device): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    if !crate::server::bearer_matches(&headers, &state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(host) = asked_key(query.as_deref()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let link = match host_link(&state, &device, &host) {
        Ok((link, _)) => link,
        Err(reason) => return unavailable(reason),
    };
    let screens = Arc::clone(&state.remote_screens);
    ws.on_upgrade(move |socket| join_view(socket, link, device, screens))
}

/// The close a desktop's view socket gets when the host's view could not be
/// opened or ended: the page opens it again on its own.
fn host_gone(reason: &str) -> Message {
    Message::Close(Some(axum::extract::ws::CloseFrame {
        code: 4010,
        reason: reason.to_owned().into(),
    }))
}

async fn join_view(mut socket: WebSocket, link: Link, device: String, screens: Arc<RemoteScreens>) {
    let mut view = [0_u8; 12];
    getrandom::getrandom(&mut view).expect("getrandom");
    let view = hex::encode(view);
    let named = (device.clone(), link.key());
    let opened = async {
        let bulk = open_view_socket(&link, StreamKind::ScreenBulk, "/ws/bulk", &view).await?;
        let screen = open_view_socket(&link, StreamKind::Screen, "/ws", &view).await?;
        Ok::<_, &'static str>((screen, bulk))
    }
    .await;
    let (screen, bulk) = match opened {
        Ok(opened) => opened,
        Err(reason) => {
            hide_core::diagnostic!(json!({
                "component": "remote_screen", "kind": "view.open_failed",
                "device": device, "reason": reason,
            }));
            let _ = socket.send(host_gone(reason)).await;
            return;
        }
    };
    lock(&screens.open_views).insert(named.clone(), view.clone());
    let _named = ViewNamed {
        screens: Arc::clone(&screens),
        named,
        view: view.clone(),
    };
    let (mut to_host, mut from_host) = screen.split();
    let (_bulk_sink, mut from_bulk) = bulk.split();
    let ended = loop {
        tokio::select! {
            frame = socket.recv() => match frame {
                Some(Ok(Message::Text(text))) => {
                    if to_host.send(tungstenite::Message::Text(text.as_str().into())).await.is_err() {
                        break "host_gone";
                    }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    if to_host.send(tungstenite::Message::Binary(bytes)).await.is_err() {
                        break "host_gone";
                    }
                }
                Some(Ok(_)) => {}
                _ => break "closed",
            },
            frame = from_host.next() => match frame.and_then(Result::ok).and_then(to_desktop) {
                Some(frame) => {
                    if socket.send(frame).await.is_err() {
                        break "closed";
                    }
                }
                None => break "host_gone",
            },
            frame = from_bulk.next() => match frame.and_then(Result::ok).and_then(to_desktop) {
                Some(frame) => {
                    if socket.send(frame).await.is_err() {
                        break "closed";
                    }
                }
                None => break "host_gone",
            },
        }
    };
    if ended == "host_gone" {
        let _ = socket.send(host_gone("host_gone")).await;
    }
}

/// The guest's record of a view it holds open, taken out as the view ends
/// unless a later view of the same host took its place.
struct ViewNamed {
    screens: Arc<RemoteScreens>,
    named: (String, PeerKey),
    view: String,
}

impl Drop for ViewNamed {
    fn drop(&mut self) {
        let mut views = lock(&self.screens.open_views);
        if views.get(&self.named) == Some(&self.view) {
            views.remove(&self.named);
        }
    }
}

/// One value of a query `a=1&b=2`.
fn query_value<'a>(query: Option<&'a str>, name: &str) -> Option<&'a str> {
    query?
        .split('&')
        .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

/// The answer the guest's desktop reads: JSON with the status the host gave.
fn json_answer(status: StatusCode, body: Bytes) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// `POST /peer-screen/<device>/page-route?key=<host key>`: the desktop
/// asks, before each load of a browser display it draws in that machine's
/// screen, where it loads from; the host's core answers through the view
/// open now, and nothing loads without its answer.
pub async fn guest_page_route(
    State(state): State<AppState>,
    Path(device): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !crate::server::bearer_matches(&headers, &state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(host) =
        query_value(query.as_deref(), "key").and_then(|key| PeerKey::from_hex(key).ok())
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if body.len() > 4 * MAX_ROUTE_FIELD {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let link = match host_link(&state, &device, &host) {
        Ok((link, _)) => link,
        Err(reason) => return unavailable(reason),
    };
    let screens = Arc::clone(&state.remote_screens);
    let Some(view) = lock(&screens.open_views).get(&(device, host)).cloned() else {
        return unavailable("not_shown");
    };
    let Some(_place) = screens.asset_place(host).await else {
        return unavailable("not_connected");
    };
    let io = match open_stream(&link, StreamKind::ScreenBulk).await {
        Ok(io) => io,
        Err(reason) => return unavailable(reason),
    };
    match ask(io, "/page-route", &view, body).await {
        Ok((status, answer)) => json_answer(status, answer),
        Err(()) => unavailable("not_connected"),
    }
}

/// `POST /peer-screen/<device>/net?key=<host key>&owner=<pid>`: the listener
/// the desktop process `owner` sets as the proxy of that machine's browser
/// pages (`screen_net`), with where requests through it stand; `DELETE`
/// lets it go.
pub async fn guest_net(
    method: axum::http::Method,
    State(state): State<AppState>,
    Path(device): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    if !crate::server::bearer_matches(&headers, &state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let key = query_value(query.as_deref(), "key").and_then(|key| PeerKey::from_hex(key).ok());
    let owner = query_value(query.as_deref(), "owner").and_then(|owner| owner.parse::<i32>().ok());
    let (Some(key), Some(owner)) = (key, owner.filter(|owner| *owner > 0)) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if method == axum::http::Method::DELETE {
        state.screen_net.release(&device, key, owner);
        return StatusCode::NO_CONTENT.into_response();
    }
    match state.screen_net.ensure(&device, key, owner).await {
        Ok(ensured) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            json!(ensured).to_string(),
        )
            .into_response(),
        Err("browser_owner_unavailable") => {
            refusal(StatusCode::BAD_REQUEST, "browser_owner_unavailable")
        }
        Err(reason) => unavailable(reason),
    }
}

/// One POST of JSON over a host's bulk stream, naming the view it asks for.
async fn ask<IO>(io: IO, path: &str, view: &str, body: Bytes) -> Result<(StatusCode, Bytes), ()>
where
    IO: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use http_body_util::{BodyExt, Full, Limited};
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(io))
            .await
            .map_err(drop)?;
    let driver = crate::owned_task::AbortOnDrop(
        tokio::spawn(async move {
            let _ = connection.await;
        })
        .abort_handle(),
    );
    let request = hyper::Request::post(path)
        .header(header::HOST, "hide")
        .header(header::CONTENT_TYPE, "application/json")
        .header(VIEW_HEADER, view)
        .body(Full::new(body))
        .map_err(drop)?;
    let response = tokio::time::timeout(OPEN_WITHIN, sender.send_request(request))
        .await
        .map_err(drop)?
        .map_err(drop)?;
    let status = response.status();
    let answer = tokio::time::timeout(
        OPEN_WITHIN,
        Limited::new(response.into_body(), 4 * MAX_ROUTE_FIELD).collect(),
    )
    .await
    .map_err(drop)?
    .map_err(drop)?
    .to_bytes();
    drop(driver);
    Ok((status, answer))
}

/// A host frame as the desktop's socket carries it; `None` at the host's
/// close.
fn to_desktop(frame: tungstenite::Message) -> Option<Message> {
    match frame {
        tungstenite::Message::Text(text) => Some(Message::Text(text.as_str().into())),
        tungstenite::Message::Binary(bytes) => Some(Message::Binary(bytes)),
        tungstenite::Message::Close(_) => None,
        _ => Some(Message::Ping(Bytes::new())),
    }
}

async fn open_view_socket(
    link: &Link,
    kind: StreamKind,
    path: &str,
    view: &str,
) -> Result<tokio_tungstenite::WebSocketStream<ViewIo>, &'static str> {
    use tungstenite::client::IntoClientRequest;
    let io = open_stream(link, kind).await?;
    let mut request = format!("ws://hide{path}")
        .into_client_request()
        .map_err(|_| "refused")?;
    request
        .headers_mut()
        .insert(VIEW_HEADER, view.parse().map_err(|_| "refused")?);
    let (socket, _) =
        tokio::time::timeout(OPEN_WITHIN, tokio_tungstenite::client_async(request, io))
            .await
            .map_err(|_| "not_connected")?
            .map_err(|_| "refused")?;
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The answer `/page-route` gives a question for `url`, the core naming
    /// `device` as the page's machine and `host` as its own.
    async fn routed(device: &str, url: &str) -> (StatusCode, serde_json::Value) {
        let body =
            json!({"device": "host", "checkout": "/web", "view": "d1", "load": 7}).to_string();
        let (device, url) = (device.to_owned(), url.to_owned());
        let answer = answer_page_route(Some(true), body.as_bytes(), move |ask| {
            assert_eq!(
                (ask.device.as_str(), ask.view.as_str(), ask.load),
                ("host", "d1", 7)
            );
            Ok((
                Some(hide_core::workspace_control::BrowserRouteSource {
                    device_id: device,
                    checkout_path: "/web".to_owned(),
                    url,
                    load: 7,
                }),
                "host".to_owned(),
            ))
        })
        .await;
        let status = answer.status();
        let bytes = http_body_util::BodyExt::collect(answer.into_body())
            .await
            .unwrap()
            .to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    // Where a display the guest draws loads from is the host core's answer:
    // a page of a third device on that device's loopback, the unspecified
    // address that reaches it included, or in its files is refused, since
    // only that device reaches it; the host's own file page is not served to
    // a screen yet; a public page of another device loads through the host as
    // not its own, and the host's own loopback page as its own.
    #[tokio::test]
    async fn a_screen_s_page_route_refuses_a_third_device_s_loopback_and_files() {
        for url in [
            "http://localhost:5173/",
            "http://127.0.0.1:3000/a",
            "http://[::]:8080/",
            "http://0.0.0.0:8080/",
            "file:///srv/device-c/report.html",
        ] {
            let (status, body) = routed("device-c", url).await;
            assert_eq!(
                (status, body["reason"].as_str()),
                (
                    StatusCode::FORBIDDEN,
                    Some("remote_device_page_unavailable")
                ),
                "{url}"
            );
        }
        let (status, body) = routed("host", "file:///srv/host/report.html").await;
        assert_eq!(
            (status, body["reason"].as_str()),
            (StatusCode::FORBIDDEN, Some("remote_file_unavailable"))
        );
        let (status, body) = routed("device-c", "https://example.com/").await;
        assert_eq!(
            (status, body["own"].as_bool(), body["url"].as_str()),
            (StatusCode::OK, Some(false), Some("https://example.com/"))
        );
        let (status, body) = routed("host", "http://localhost:5173/").await;
        assert_eq!(
            (status, body["own"].as_bool(), body["load"].as_u64()),
            (StatusCode::OK, Some(true), Some(7))
        );
    }

    // A view that is not this guest's live one is answered before the core
    // is asked, and a question out of shape is refused.
    #[tokio::test]
    async fn a_page_route_answers_only_a_live_view_of_its_guest() {
        let never = |_: PageRouteAsk| -> RouteSourceOf { panic!("the core is not asked") };
        assert_eq!(
            answer_page_route(Some(false), b"{}", never).await.status(),
            StatusCode::GONE
        );
        assert_eq!(
            answer_page_route(None, b"{}", never).await.status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            answer_page_route(Some(true), b"not json", never)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        let gone = answer_page_route(
            Some(true),
            br#"{"device":"h","checkout":"/w","view":"d1","load":1}"#,
            |_| Ok((None, "h".to_owned())),
        )
        .await;
        assert_eq!(gone.status(), StatusCode::GONE);
    }

    fn viewer(byte: u8) -> Viewer {
        Viewer {
            key: PeerKey::from_bytes([byte; 32]),
            device: format!("device-{byte}"),
            grant: format!("grant-{byte}"),
            generation: 1,
            view: format!("view-{byte}"),
        }
    }

    /// A guest holds one view: its next takes the place of the last, which
    /// is told so. All guests hold MAX_VIEWS at most, apart from this
    /// machine's windows (PRD D-67).
    #[tokio::test]
    async fn a_guest_holds_one_view_and_all_guests_a_few() {
        let screens = RemoteScreens::new();
        let first = screens.seat(viewer(1)).unwrap();
        let second = screens.seat(viewer(1)).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), first.taken())
            .await
            .expect("the first view is told its place was taken");
        drop(first);
        assert_eq!(screens.seated(), 1);
        let others: Vec<_> = (2..=4)
            .map(|byte| screens.seat(viewer(byte)).unwrap())
            .collect();
        assert_eq!(screens.seat(viewer(9)).err(), Some(SeatRefused::Full));
        // A guest already seated takes its own place even when all are held.
        let again = screens.seat(viewer(2)).unwrap();
        drop((second, others, again));
        assert_eq!(screens.seated(), 0);
    }

    fn asset(host: u8, build: &str, path: &str) -> AssetKey {
        AssetKey {
            host: PeerKey::from_bytes([host; 32]),
            build: build.into(),
            path: path.into(),
        }
    }

    #[test]
    fn the_asset_cache_keeps_a_build_s_assets_within_its_bound() {
        let mut cache = AssetCache::default();
        let big = Bytes::from(vec![0_u8; ASSET_CACHE / 2 + 1]);
        cache.put(
            asset(1, "b1", "/assets/a.js"),
            "text/javascript".into(),
            big.clone(),
        );
        cache.put(
            asset(1, "b1", "/assets/b.js"),
            "text/javascript".into(),
            big,
        );
        assert!(cache.bytes <= ASSET_CACHE);
        assert!(cache.get(&asset(1, "b1", "/assets/a.js")).is_none());
        assert!(cache.get(&asset(1, "b1", "/assets/b.js")).is_some());
        assert!(cache.get(&asset(1, "b2", "/assets/b.js")).is_none());
    }

    // A host that holds every place it has (a stalled answer) holds none of
    // another host's: that one's page still opens at once, and the stalled
    // host's next request gives up within its bound.
    #[tokio::test(start_paused = true)]
    async fn a_host_that_holds_its_places_holds_no_other_host_s() {
        let screens = RemoteScreens::new();
        let (stalled, other) = (PeerKey::from_bytes([1; 32]), PeerKey::from_bytes([2; 32]));
        let mut held = Vec::new();
        for _ in 0..ASSET_STREAMS {
            held.push(
                screens
                    .asset_place(stalled)
                    .await
                    .expect("a place of its own"),
            );
        }
        let started = tokio::time::Instant::now();
        assert!(screens.asset_place(other).await.is_some());
        assert_eq!(started.elapsed(), std::time::Duration::ZERO);
        assert!(screens.asset_place(stalled).await.is_none());
        assert_eq!(started.elapsed(), OPEN_WITHIN);
        drop(held);
        assert!(screens.asset_place(stalled).await.is_some());
    }

    // Empty answers under ever new names fill the bound too: what each
    // entry's names and places take counts, so the oldest go.
    #[test]
    fn empty_assets_under_new_names_stay_within_the_bound() {
        let mut cache = AssetCache::default();
        let limit = ASSET_CACHE / ENTRY_COST;
        for n in 0..limit + 100 {
            cache.put(
                asset(1, "b1", &format!("/assets/{n}.js")),
                "text/javascript".into(),
                Bytes::new(),
            );
        }
        assert!(cache.bytes <= ASSET_CACHE);
        assert!(cache.entries.len() < limit);
        assert_eq!(cache.entries.len(), cache.order.len());
        assert!(cache.get(&asset(1, "b1", "/assets/0.js")).is_none());
        assert!(
            cache
                .get(&asset(1, "b1", &format!("/assets/{}.js", limit + 99)))
                .is_some()
        );
    }

    // A paired machine that names another's build and path keeps its own
    // file: the other machine's page is served the other machine's bytes.
    #[test]
    fn two_hosts_saying_the_same_build_and_path_keep_their_own_bytes() {
        let mut cache = AssetCache::default();
        cache.put(
            asset(1, "b1", "/assets/app.js"),
            "text/javascript".into(),
            Bytes::from_static(b"one"),
        );
        cache.put(
            asset(2, "b1", "/assets/app.js"),
            "text/javascript".into(),
            Bytes::from_static(b"two"),
        );
        assert_eq!(
            cache
                .get(&asset(1, "b1", "/assets/app.js"))
                .map(|(_, body)| body),
            Some(Bytes::from_static(b"one"))
        );
        assert_eq!(
            cache
                .get(&asset(2, "b1", "/assets/app.js"))
                .map(|(_, body)| body),
            Some(Bytes::from_static(b"two"))
        );
        assert!(cache.get(&asset(3, "b1", "/assets/app.js")).is_none());
    }
}
