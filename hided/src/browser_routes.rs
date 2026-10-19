//! Native Browser View routes into a consented SSH device. A route belongs
//! to one core-owned View and load stamp; it never falls back to this Mac.
//!
//! The same registry serves a node whose core runs on another machine
//! (`node_pages`): there the View's source comes from the core over the
//! link, and a page of the core's machine reaches its loopback through the
//! link's own SSH connection. [`PageSources`] is what each side supplies.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use herdr_core::node_access::{self, NodeLink};
use herdr_core::workspace_control::BrowserRouteSource;
use hide_node::ssh::upstream::Upstream;
use hide_node::ssh::{RemoteLocalForward, RusshRemoteClient};
use hide_node_link::device::RemoteResult;
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Semaphore, oneshot, watch};

use crate::browser_assets::{self, Asset};
use crate::core::CoreHandle;
use crate::state_file::new_token;
use hide_node::pane_proof::process_start;

const MAX_ROUTES: usize = 12;
const MAX_ROUTE_BUILDS: usize = 4;
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FILE_REQUESTS: usize = 8;

/// Where a registry's pages come from, on the machine it runs on. Both
/// calls block, so the registry makes them off the async runtime.
pub trait PageSources: Send + Sync + 'static {
    /// The machine whose pages the desktop loads as they are.
    fn own_node(&self) -> &str;
    /// The page a core-owned View shows at `load`, or `None` when that View
    /// or load is no longer current.
    fn source(
        &self,
        device: &str,
        checkout: &str,
        view: &str,
        load: u64,
    ) -> Result<Option<BrowserRouteSource>, &'static str>;
    /// The way to `device`'s loopback and files.
    fn way(&self, device: &str) -> Result<Way, &'static str>;
}

/// How a page of another machine is reached from this one.
pub enum Way {
    /// A device this core reached over SSH: its loopback through that
    /// connection, its files over its link.
    Device {
        client: Arc<RusshRemoteClient>,
        files: Arc<dyn NodeLink>,
    },
    /// The core's machine, from a node whose core runs there: its loopback
    /// through the link's SSH connection. Its files are not read from here,
    /// so its `file:` pages are refused.
    Core(Arc<Upstream>),
}

impl Way {
    fn forward(
        &self,
        remote: SocketAddr,
        alternate: Option<SocketAddr>,
        preserve_numeric_host: bool,
        canceled: oneshot::Receiver<()>,
    ) -> RemoteResult<RemoteLocalForward> {
        match self {
            Self::Device { client, .. } => client.start_local_workspace_forward(
                remote,
                alternate,
                preserve_numeric_host,
                canceled,
            ),
            Self::Core(upstream) => {
                upstream.page_forward(remote, alternate, preserve_numeric_host, canceled)
            }
        }
    }
}

impl PageSources for CoreHandle {
    fn own_node(&self) -> &str {
        self.node().as_str()
    }

    fn source(
        &self,
        device: &str,
        checkout: &str,
        view: &str,
        load: u64,
    ) -> Result<Option<BrowserRouteSource>, &'static str> {
        self.browser_route_source(device, checkout, view, load)
            .map_err(|_| "core_unavailable")
    }

    fn way(&self, device: &str) -> Result<Way, &'static str> {
        let route = self
            .workspace_remote_routes()
            .map_err(|_| "core_unavailable")?
            .into_iter()
            .find(|route| route.device_id == device)
            .ok_or("host_unavailable")?;
        Ok(Way::Device {
            client: ssh_client(&route).ok_or("host_unavailable")?,
            files: route.channel,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct Key {
    owner_pid: i32,
    device: String,
    checkout: String,
    view: String,
}

enum RouteKind {
    Http(RemoteLocalForward),
    File(oneshot::Sender<()>),
}

struct Route {
    owner_started: u64,
    source: BrowserRouteSource,
    url: String,
    kind: RouteKind,
}

type RouteResult = Result<Resolved, &'static str>;

enum RouteEntry {
    Pending {
        owner_started: u64,
        source: BrowserRouteSource,
        result: watch::Sender<Option<RouteResult>>,
        cancel: oneshot::Sender<()>,
    },
    Ready(Route),
}

impl RouteEntry {
    fn source(&self) -> &BrowserRouteSource {
        match self {
            Self::Pending { source, .. } => source,
            Self::Ready(route) => &route.source,
        }
    }

    fn owner_started(&self) -> u64 {
        match self {
            Self::Pending { owner_started, .. } => *owner_started,
            Self::Ready(route) => route.owner_started,
        }
    }

    fn close(self) {
        match self {
            Self::Pending { result, cancel, .. } => {
                let _ = cancel.send(());
                result.send_replace(Some(Err("view_unavailable")));
            }
            Self::Ready(route) => route.close(),
        }
    }
}

impl Route {
    fn close(self) {
        match self.kind {
            RouteKind::Http(forward) => forward.close(),
            RouteKind::File(stop) => {
                let _ = stop.send(());
            }
        }
    }
}

#[derive(Clone, Serialize)]
pub struct Resolved {
    pub url: String,
    pub source_url: String,
    pub load: u64,
}

pub struct BrowserRoutes {
    pages: Arc<dyn PageSources>,
    routes: Mutex<HashMap<Key, RouteEntry>>,
    build_slots: Arc<Semaphore>,
}

impl BrowserRoutes {
    pub fn new(pages: Arc<dyn PageSources>) -> Arc<Self> {
        Arc::new(Self {
            pages,
            routes: Mutex::new(HashMap::new()),
            build_slots: Arc::new(Semaphore::new(MAX_ROUTE_BUILDS)),
        })
    }

    pub async fn resolve(
        self: &Arc<Self>,
        device: String,
        checkout: String,
        view: String,
        load: u64,
        owner_pid: i32,
    ) -> Result<Resolved, &'static str> {
        let owner_started = process_start(owner_pid).ok_or("browser_owner_unavailable")?;
        let key = Key {
            owner_pid,
            device,
            checkout,
            view,
        };
        let pages = Arc::clone(&self.pages);
        let query = key.clone();
        let source = tokio::task::spawn_blocking(move || {
            pages.source(&query.device, &query.checkout, &query.view, load)
        })
        .await
        .map_err(|_| "core_unavailable")??
        .ok_or("view_unavailable")?;
        if self.pages.own_node() == key.device {
            return Ok(Resolved {
                url: source.url.clone(),
                source_url: source.url,
                load,
            });
        }
        let mut routes = self.routes.lock().await;
        if let Some(entry) = routes.get(&key)
            && entry.source() == &source
            && entry.owner_started() == owner_started
        {
            match entry {
                RouteEntry::Ready(route) => {
                    return Ok(Resolved {
                        url: route.url.clone(),
                        source_url: source.url,
                        load,
                    });
                }
                RouteEntry::Pending { result, .. } => {
                    let waiting = result.subscribe();
                    drop(routes);
                    return wait_for_route(waiting).await;
                }
            }
        }
        if routes.len() >= MAX_ROUTES && !routes.contains_key(&key) {
            return Err("browser_route_limit");
        }
        let build_permit = Arc::clone(&self.build_slots)
            .try_acquire_owned()
            .map_err(|_| "browser_route_build_limit")?;
        let old = routes.remove(&key);
        let (result, waiting) = watch::channel(None);
        let (cancel, canceled) = oneshot::channel();
        routes.insert(
            key.clone(),
            RouteEntry::Pending {
                owner_started,
                source: source.clone(),
                result: result.clone(),
                cancel,
            },
        );
        drop(routes);
        if let Some(old) = old {
            old.close();
        }
        let routes = Arc::clone(self);
        tokio::spawn(async move {
            let _build_permit = build_permit;
            let built = routes
                .build_route(&key, &source, owner_started, canceled)
                .await;
            routes
                .finish_route(key, source, owner_started, result, built)
                .await;
        });
        wait_for_route(waiting).await
    }

    async fn build_route(
        &self,
        key: &Key,
        source: &BrowserRouteSource,
        owner_started: u64,
        canceled: oneshot::Receiver<()>,
    ) -> Result<Option<Route>, &'static str> {
        let pages = Arc::clone(&self.pages);
        let device = key.device.clone();
        let way = tokio::task::spawn_blocking(move || pages.way(&device))
            .await
            .map_err(|_| "core_unavailable")??;
        let (url, kind) = if crate::file_url::is_file_url(&source.url) {
            let Way::Device { files, .. } = &way else {
                return Err("file_page_unavailable");
            };
            let (path, suffix) =
                crate::file_url::file_path(&source.url).ok_or("invalid_file_url")?;
            // The device's paths in the wire spelling, related by names
            // alone whatever system the device runs.
            let relative = hide_platform::path::wire_relative(&source.checkout_path, &path)
                .ok()
                .filter(|relative| !relative.is_root())
                .ok_or("outside_checkout")?
                .into_string();
            let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
                .await
                .map_err(|_| "route_bind_failed")?;
            let port = listener
                .local_addr()
                .map_err(|_| "route_bind_failed")?
                .port();
            let secret = new_token();
            let state = FileRoute {
                pages: Arc::clone(&self.pages),
                key: key.clone(),
                owner_started,
                source: source.clone(),
                channel: Arc::clone(files),
                secret: secret.clone(),
                entry: relative.clone(),
                allowed: Arc::new(Mutex::new(HashMap::new())),
                requests: Arc::new(Semaphore::new(MAX_FILE_REQUESTS)),
            };
            let app = Router::new().fallback(get(serve_file)).with_state(state);
            let (stop, done) = oneshot::channel();
            tokio::spawn(async move {
                let result = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = done.await;
                    })
                    .await;
                if let Err(error) = result {
                    eprintln!(
                        "{}",
                        serde_json::json!({"component":"browser_routes","kind":"file_server.failed","reason":error.to_string()})
                    );
                }
            });
            let escaped = relative
                .split('/')
                .map(|part| utf8_percent_encode(part, NON_ALPHANUMERIC).to_string())
                .collect::<Vec<_>>()
                .join("/");
            (
                format!("http://127.0.0.1:{port}/{secret}/{escaped}{suffix}"),
                RouteKind::File(stop),
            )
        } else if let Some((scheme, remote_port, host, tail)) = loopback_target(&source.url) {
            let remote_ip = crate::browser_cli::loopback_ip(&host).ok_or("invalid_loopback")?;
            let remote = SocketAddr::new(remote_ip, remote_port);
            let alternate = (host == "localhost").then_some(SocketAddr::new(
                IpAddr::V6(Ipv6Addr::LOCALHOST),
                remote_port,
            ));
            let preserve_numeric_host =
                scheme == "https" && host != "localhost" && host != "0.0.0.0";
            let forward = tokio::task::spawn_blocking(move || {
                way.forward(remote, alternate, preserve_numeric_host, canceled)
            })
            .await
            .map_err(|_| "route_failed")?
            .map_err(|error| {
                eprintln!(
                    "{}",
                    serde_json::json!({"component":"browser_routes","kind":"forward.failed","reason":error.to_string()})
                );
                "route_failed"
            })?;
            let url = forwarded_url(&scheme, &host, forward.local_addr(), &tail);
            (url, RouteKind::Http(forward))
        } else {
            return Ok(None);
        };
        Ok(Some(Route {
            owner_started,
            source: source.clone(),
            url,
            kind,
        }))
    }

    async fn finish_route(
        &self,
        key: Key,
        source: BrowserRouteSource,
        owner_started: u64,
        result: watch::Sender<Option<RouteResult>>,
        built: Result<Option<Route>, &'static str>,
    ) {
        let current = if built.is_ok() && process_start(key.owner_pid) == Some(owner_started) {
            let pages = Arc::clone(&self.pages);
            let query = key.clone();
            let load = source.load;
            tokio::task::spawn_blocking(move || {
                pages
                    .source(&query.device, &query.checkout, &query.view, load)
                    .ok()
                    .flatten()
            })
            .await
            .ok()
            .flatten()
            .as_ref()
                == Some(&source)
        } else {
            false
        };
        let mut routes = self.routes.lock().await;
        let reserved = matches!(routes.get(&key), Some(RouteEntry::Pending { result: pending, .. }) if pending.same_channel(&result));
        if !reserved {
            drop(routes);
            if let Ok(Some(route)) = built {
                route.close();
            }
            eprintln!(
                "{}",
                serde_json::json!({"component":"browser_routes","kind":"route.discarded","reason":"reservation_superseded","device_id":source.device_id,"load":source.load})
            );
            return;
        }
        let outcome = match (current, built) {
            (true, Ok(Some(route))) => {
                let resolved = Resolved {
                    url: route.url.clone(),
                    source_url: source.url.clone(),
                    load: source.load,
                };
                routes.insert(key, RouteEntry::Ready(route));
                eprintln!(
                    "{}",
                    serde_json::json!({"component":"browser_routes","kind":"route.ready","device_id":source.device_id,"load":source.load})
                );
                Ok(resolved)
            }
            (true, Ok(None)) => {
                routes.remove(&key);
                Ok(Resolved {
                    url: source.url.clone(),
                    source_url: source.url,
                    load: source.load,
                })
            }
            (false, Ok(Some(route))) => {
                routes.remove(&key);
                drop(routes);
                route.close();
                result.send_replace(Some(Err("view_unavailable")));
                return;
            }
            (_, Err(reason)) => {
                routes.remove(&key);
                Err(reason)
            }
            (false, Ok(None)) => {
                routes.remove(&key);
                Err("view_unavailable")
            }
        };
        drop(routes);
        result.send_replace(Some(outcome));
    }

    pub async fn release(
        &self,
        device: &str,
        checkout: &str,
        view: &str,
        load: u64,
        owner_pid: i32,
    ) {
        let key = Key {
            owner_pid,
            device: device.to_owned(),
            checkout: checkout.to_owned(),
            view: view.to_owned(),
        };
        let mut routes = self.routes.lock().await;
        let removed = if routes
            .get(&key)
            .is_some_and(|entry| entry.source().load == load)
        {
            routes.remove(&key)
        } else {
            None
        };
        drop(routes);
        if let Some(entry) = removed {
            entry.close();
        }
    }

    /// Closes every route whose owner, View or desktop is gone, every 2 s
    /// until `shutdown`; the caller may also end it by aborting the task.
    pub fn spawn_reaper(
        self: &Arc<Self>,
        desktop_renderers: Arc<AtomicUsize>,
        shutdown: crate::ending::Ending,
    ) -> tokio::task::JoinHandle<()> {
        let routes = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            loop {
                tokio::select! { () = shutdown.ended() => break, _ = tick.tick() => {} }
                let candidates: Vec<_> = routes
                    .routes
                    .lock()
                    .await
                    .iter()
                    .map(|(key, entry)| {
                        (key.clone(), entry.source().clone(), entry.owner_started())
                    })
                    .collect();
                for (key, source, owner_started) in candidates {
                    let load = source.load;
                    let stale = if desktop_renderers.load(Ordering::SeqCst) == 0
                        || process_start(key.owner_pid) != Some(owner_started)
                    {
                        true
                    } else {
                        let pages = Arc::clone(&routes.pages);
                        let key_for_query = key.clone();
                        tokio::task::spawn_blocking(move || {
                            pages
                                .source(
                                    &key_for_query.device,
                                    &key_for_query.checkout,
                                    &key_for_query.view,
                                    source.load,
                                )
                                .ok()
                                .flatten()
                                .is_none()
                        })
                        .await
                        .unwrap_or(true)
                    };
                    if stale {
                        routes
                            .release(&key.device, &key.checkout, &key.view, load, key.owner_pid)
                            .await;
                    }
                }
            }
            let remaining: Vec<_> = routes
                .routes
                .lock()
                .await
                .drain()
                .map(|(_, entry)| entry)
                .collect();
            for entry in remaining {
                entry.close();
            }
        })
    }
}

/// What the desktop host sends to resolve or release a View's route.
#[derive(Deserialize)]
pub(crate) struct RouteRequest {
    device_id: String,
    checkout_path: String,
    id: String,
    load: u64,
    owner_pid: i32,
}

impl RouteRequest {
    fn valid(&self) -> bool {
        !self.device_id.is_empty()
            && self.device_id.len() <= 256
            && hide_platform::path::is_wire_absolute(&self.checkout_path)
            && self.checkout_path.len() <= 8192
            && !self.id.is_empty()
            && self.id.len() <= 256
            && self.owner_pid > 0
    }
}

/// The answer to an authorized `POST /browser-route`.
pub(crate) async fn resolve_answer(routes: &Arc<BrowserRoutes>, request: RouteRequest) -> Response {
    if !request.valid() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match routes
        .resolve(
            request.device_id,
            request.checkout_path,
            request.id,
            request.load,
            request.owner_pid,
        )
        .await
    {
        Ok(route) => axum::Json(route).into_response(),
        Err(reason) => {
            eprintln!(
                "{}",
                serde_json::json!({"component":"browser_routes","kind":"resolve.refused","reason":reason})
            );
            (
                StatusCode::CONFLICT,
                axum::Json(serde_json::json!({"reason":reason})),
            )
                .into_response()
        }
    }
}

/// The answer to an authorized `DELETE /browser-route`.
pub(crate) async fn release_answer(routes: &BrowserRoutes, request: RouteRequest) -> Response {
    if !request.valid() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    routes
        .release(
            &request.device_id,
            &request.checkout_path,
            &request.id,
            request.load,
            request.owner_pid,
        )
        .await;
    StatusCode::NO_CONTENT.into_response()
}

async fn wait_for_route(mut result: watch::Receiver<Option<RouteResult>>) -> RouteResult {
    loop {
        if let Some(outcome) = result.borrow_and_update().clone() {
            return outcome;
        }
        result.changed().await.map_err(|_| "route_failed")?;
    }
}

fn loopback_target(raw: &str) -> Option<(String, u16, String, String)> {
    let (without_fragment, fragment) = raw
        .split_once('#')
        .map_or((raw, ""), |(head, tail)| (head, tail));
    let uri: Uri = without_fragment.parse().ok()?;
    let scheme = uri.scheme_str()?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = uri.authority()?;
    let host = authority.host().to_ascii_lowercase();
    crate::browser_cli::loopback_ip(&host)?;
    let port = authority
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });
    let local_host = match host.as_str() {
        "0.0.0.0" => "127.0.0.1",
        "localhost." => "localhost",
        "::1" => "[::1]",
        _ => &host,
    };
    let tail = uri.path_and_query().map_or("/", |value| value.as_str());
    let tail = if fragment.is_empty() {
        tail.to_owned()
    } else {
        format!("{tail}#{fragment}")
    };
    Some((scheme.to_owned(), port, local_host.to_owned(), tail))
}

fn forwarded_url(scheme: &str, host: &str, local_addr: SocketAddr, tail: &str) -> String {
    // HTTPS validates the URL host, so keep the source name or numeric IP.
    // The forward binds that numeric loopback IP, and reserves both families
    // when localhost may resolve to either one.
    let address = if scheme == "https" && host != "0.0.0.0" {
        format!("{host}:{}", local_addr.port())
    } else {
        local_addr.to_string()
    };
    format!("{scheme}://{address}{tail}")
}

#[derive(Clone)]
struct FileRoute {
    pages: Arc<dyn PageSources>,
    key: Key,
    owner_started: u64,
    source: BrowserRouteSource,
    channel: Arc<dyn NodeLink>,
    secret: String,
    entry: String,
    allowed: Arc<Mutex<HashMap<String, Asset>>>,
    requests: Arc<Semaphore>,
}

async fn serve_file(State(route): State<FileRoute>, uri: Uri) -> Response {
    let Some(path) = uri.path().strip_prefix(&format!("/{}/", route.secret)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if path.len() > 8192 {
        return StatusCode::URI_TOO_LONG.into_response();
    }
    let Ok(decoded) = percent_decode_str(path).decode_utf8() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if hide_platform::path::RelPath::parse(&decoded).is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let is_entry = decoded == route.entry;
    let asset = if is_entry {
        None
    } else {
        route.allowed.lock().await.get(decoded.as_ref()).copied()
    };
    if !is_entry && asset.is_none() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(permit) = route.requests.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let check = Arc::clone(&route.pages);
    let source = route.source.clone();
    let key = route.key.clone();
    let owner_started = route.owner_started;
    let channel = Arc::clone(&route.channel);
    let decoded = decoded.into_owned();
    let entry = route.entry.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if process_start(key.owner_pid) != Some(owner_started) {
            return Err("browser_owner_unavailable");
        }
        if check
            .source(&key.device, &key.checkout, &key.view, source.load)
            .ok()
            .flatten()
            .is_none()
        {
            return Err("view_unavailable");
        }
        let mut bytes = Vec::new();
        let mut stamp = None;
        loop {
            let range = node_access::read_bytes(
                channel.as_ref(),
                &key.checkout,
                &decoded,
                bytes.len() as u64,
                hide_host::bytes::MAX_RANGE,
            )
            .map_err(|_| "file_unavailable")?;
            if range.total > MAX_FILE_BYTES {
                return Err("file_too_large");
            }
            if stamp.as_ref().is_some_and(|old| old != &range.file) {
                return Err("file_changed");
            }
            stamp = Some(range.file.clone());
            let chunk = range.bytes().map_err(|_| "file_unavailable")?;
            if chunk.is_empty() && (bytes.len() as u64) < range.total {
                return Err("file_unavailable");
            }
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 >= range.total {
                break;
            }
        }
        let additions = if is_entry {
            browser_assets::html_assets(&entry, &bytes)
        } else if asset == Some(Asset::Css) {
            browser_assets::css_assets(&decoded, &bytes)
        } else {
            HashMap::new()
        };
        Ok::<_, &'static str>((bytes, additions))
    })
    .await;
    match result {
        Ok(Ok((bytes, additions))) => {
            let mut allowed = route.allowed.lock().await;
            if !is_entry
                && allowed.len()
                    + additions
                        .keys()
                        .filter(|path| !allowed.contains_key(*path))
                        .count()
                    > browser_assets::MAX_ASSETS
            {
                return StatusCode::TOO_MANY_REQUESTS.into_response();
            }
            if is_entry {
                if additions.len() > browser_assets::MAX_ASSETS {
                    return StatusCode::TOO_MANY_REQUESTS.into_response();
                }
                *allowed = additions;
            } else {
                allowed.extend(additions);
            }
            drop(allowed);
            Response::builder()
            .status(StatusCode::OK)
            .header(
                "content-type",
                crate::server::mime_for(Path::new(&decoded_path_for_mime(uri.path()))),
            )
            .header("x-content-type-options", "nosniff")
            .header("content-security-policy", "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'none'; form-action 'none'; base-uri 'none'; object-src 'none'; frame-src 'none'")
            .body(Body::from(bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Ok(Err(reason)) => {
            eprintln!(
                "{}",
                serde_json::json!({"component":"browser_routes","kind":"file.refused","reason":reason})
            );
            match reason {
                "view_unavailable" => StatusCode::GONE,
                "file_too_large" => StatusCode::PAYLOAD_TOO_LARGE,
                "file_changed" => StatusCode::CONFLICT,
                _ => StatusCode::FORBIDDEN,
            }
            .into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn decoded_path_for_mime(path: &str) -> String {
    percent_decode_str(path).decode_utf8_lossy().into_owned()
}

/// The SSH client behind a route's device transport, for the loopback
/// forward this daemon opens itself. Every device this daemon builds is an
/// `SshDevice`, so `None` means a transport this daemon did not build.
fn ssh_client(
    route: &herdr_core::WorkspaceRemoteRoute,
) -> Option<Arc<hide_node::ssh::RusshRemoteClient>> {
    Arc::clone(&route.transport)
        .into_any()
        .downcast::<hide_node::ssh::SshDevice>()
        .ok()
        .and_then(|device| device.client().cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_admitted_loopback_address_has_an_ssh_target() {
        for address in [
            "http://localhost:5173/",
            "http://localhost.:5173/",
            "http://127.0.0.1:5173/",
            "http://127.0.0.2:5173/",
            "http://[::1]:5173/",
            "http://[0:0:0:0:0:0:0:1]:5173/",
            "http://[::ffff:127.0.0.1]:5173/",
        ] {
            let (_, port, host, _) = loopback_target(address).expect(address);
            assert_eq!(port, 5173);
            assert!(crate::browser_cli::loopback_ip(&host).is_some());
        }
        assert_eq!(
            crate::browser_cli::loopback_ip("[::ffff:127.0.0.1]"),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
        );
    }

    #[test]
    fn forwarded_https_retains_localhost_certificate_name_and_scheme() {
        let (scheme, _, host, tail) =
            loopback_target("HTTPS://localhost:8443/secure?q=1").expect("admitted loopback URL");
        assert_eq!(
            forwarded_url(&scheme, &host, "127.0.0.1:49152".parse().unwrap(), &tail),
            "https://localhost:49152/secure?q=1"
        );
        assert_eq!(
            forwarded_url(
                "https",
                "127.0.0.2",
                "127.0.0.2:49152".parse().unwrap(),
                "/"
            ),
            "https://127.0.0.2:49152/"
        );
    }
}
