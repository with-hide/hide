//! Native Browser View routes into a connected device. A route belongs to
//! one core-owned View and load stamp; it never falls back to this Mac. A
//! device's page and files ride the device's node link, whichever link the
//! device has (`page_forward`).
//!
//! A route is for one owner: a desktop process of this machine, or another
//! machine's view of this one (`remote_screen`), whose window draws this
//! machine's displays and reaches a route through this machine's proxy
//! (`screen_net`). A view's routes also serve this machine's own files,
//! which its window cannot read, and live only while the view does.
//!
//! The same registry serves a node whose core runs on another machine
//! (`node_pages`): there the View's source comes from the core over the
//! link, and a page of the core's machine reaches its loopback through the
//! node's connection to its core. [`PageSources`] is what each side
//! supplies.

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
use hide_core::node_access::{self, NodeLink};
use hide_core::workspace_control::BrowserRouteSource;
use hide_node::ssh::RemoteHost;
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Semaphore, oneshot, watch};

use crate::browser_assets::{self, Asset};
use crate::core::CoreHandle;
use crate::page_forward::{PageForward, PageLink};
use crate::state_file::new_token;
use hide_node::pane_proof::process_start;
use hide_peer::mux::Mux;
use hide_peer::protocol::PeerKey;

const MAX_ROUTES: usize = 12;
const MAX_ROUTE_BUILDS: usize = 4;
/// The routes one guest's view holds at once, apart from the desktop's
/// `MAX_ROUTES`, so a guest never takes this machine's own window's places.
const MAX_SCREEN_ROUTES: usize = 8;
/// The routes guests' views build at once, apart from the desktop's.
const MAX_SCREEN_ROUTE_BUILDS: usize = 4;
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
    /// A connected device: its loopback and its files through its node
    /// link, whichever link that is.
    Device(RemoteHost),
    /// The core's machine, from a node whose core runs there: its loopback
    /// through the node's connection to its core. Its files are not read
    /// from here, so its `file:` pages are refused.
    Core(Arc<Mux>),
    /// This machine's own node: its files, for a view of another machine,
    /// which cannot read them.
    Own(Arc<dyn NodeLink>),
}

/// Who a route is for.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Owner {
    /// A desktop process of this machine, by its pid.
    Process(i32),
    /// Another machine's view of this one: the guest's key and the view it
    /// named (`remote_screen`).
    Screen { guest: PeerKey, view: String },
}

/// The views other machines hold of this one now (`remote_screen`).
pub trait ScreenViews: Send + Sync + 'static {
    /// Whether `guest`'s view `view` is still open.
    fn live(&self, guest: &PeerKey, view: &str) -> bool;
}

/// Whether a route's owner is still the one it was made for: the process,
/// started when it was (`started`), or the view, still open.
fn owner_alive(screens: Option<&dyn ScreenViews>, owner: &Owner, started: u64) -> bool {
    match owner {
        Owner::Process(pid) => process_start(*pid) == Some(started),
        Owner::Screen { guest, view } => screens.is_some_and(|screens| screens.live(guest, view)),
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
        if device == self.node().as_str() {
            return self
                .node_link(device)
                .map(Way::Own)
                .map_err(|_| "core_unavailable");
        }
        let route = self
            .workspace_remote_routes()
            .map_err(|_| "core_unavailable")?
            .into_iter()
            .find(|route| route.device_id == device)
            .ok_or("host_unavailable")?;
        // Every device link this daemon builds is a `RemoteHost`; one it
        // cannot carry a stream over is not a way to the device's pages.
        let link = route
            .channel
            .as_any()
            .and_then(|link| link.downcast_ref::<RemoteHost>())
            .ok_or("host_unavailable")?;
        Ok(Way::Device(link.clone()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct Key {
    owner: Owner,
    device: String,
    checkout: String,
    view: String,
}

enum RouteKind {
    /// A page of a device or of the core's machine.
    Page(PageForward),
    File(oneshot::Sender<()>),
}

/// `owner_started` is when a process owner started; a view's is 0, its id
/// being the whole of it.
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

    /// Whether the route still reaches its page: one over a node link that
    /// has ended reaches nothing.
    fn live(&self) -> bool {
        match self {
            Self::Pending { .. } => true,
            Self::Ready(route) => match &route.kind {
                RouteKind::Page(forward) => forward.live(),
                RouteKind::File(_) => true,
            },
        }
    }

    fn close(self) {
        match self {
            Self::Pending { result, .. } => {
                result.send_replace(Some(Err("view_unavailable")));
            }
            Self::Ready(route) => route.close(),
        }
    }
}

impl Route {
    fn close(self) {
        match self.kind {
            RouteKind::Page(forward) => drop(forward),
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
    /// A `file:` page's folder of its own files: the root of the checkout
    /// that holds it (`BrowserRouteSource::checkout_path`), which the desktop
    /// host confines a local page's requests to and reads a route's
    /// addresses against.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

impl Resolved {
    /// The answer for `source` loaded from `url`, this machine's own address
    /// or a route's.
    fn of(source: &BrowserRouteSource, url: String) -> Self {
        Self {
            url,
            source_url: source.url.clone(),
            load: source.load,
            root: hide_platform::path::is_file_url(&source.url)
                .then(|| source.checkout_path.clone()),
        }
    }
}

pub struct BrowserRoutes {
    pages: Arc<dyn PageSources>,
    /// Other machines' views of this one, which own their own routes; none
    /// on a node whose core runs elsewhere.
    screens: Option<Arc<dyn ScreenViews>>,
    routes: Mutex<HashMap<Key, RouteEntry>>,
    build_slots: Arc<Semaphore>,
    screen_build_slots: Arc<Semaphore>,
}

impl BrowserRoutes {
    pub fn new(pages: Arc<dyn PageSources>, screens: Option<Arc<dyn ScreenViews>>) -> Arc<Self> {
        Arc::new(Self {
            pages,
            screens,
            routes: Mutex::new(HashMap::new()),
            build_slots: Arc::new(Semaphore::new(MAX_ROUTE_BUILDS)),
            screen_build_slots: Arc::new(Semaphore::new(MAX_SCREEN_ROUTE_BUILDS)),
        })
    }

    fn alive(&self, owner: &Owner, started: u64) -> bool {
        owner_alive(self.screens.as_deref(), owner, started)
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
            owner: Owner::Process(owner_pid),
            device,
            checkout,
            view,
        };
        self.resolve_as(key, load, owner_started)
            .await
            .map(|(resolved, _)| resolved)
    }

    /// Where display `display` loads from in `guest`'s view `view` of this
    /// machine (`remote_screen`, `/page-route`), and whether it is this
    /// machine's own page, whose loopback it may reach. A web page of this
    /// machine loads its address; one of its files, which that machine
    /// cannot read, loads from a route of this machine's loopback the view
    /// owns; a device's file page from a route as this machine's window
    /// loads it. A device's loopback page is refused
    /// (`remote_device_page_unavailable`): that view keeps no login storage
    /// apart for a device's pages.
    pub async fn resolve_for_screen(
        self: &Arc<Self>,
        device: String,
        checkout: String,
        display: String,
        load: u64,
        guest: PeerKey,
        view: String,
    ) -> Result<(Resolved, bool), &'static str> {
        let owner = Owner::Screen { guest, view };
        if !self.alive(&owner, 0) {
            return Err("view_unavailable");
        }
        let key = Key {
            owner,
            device,
            checkout,
            view: display,
        };
        self.resolve_as(key, load, 0).await
    }

    async fn resolve_as(
        self: &Arc<Self>,
        key: Key,
        load: u64,
        owner_started: u64,
    ) -> Result<(Resolved, bool), &'static str> {
        let pages = Arc::clone(&self.pages);
        let query = key.clone();
        let source = tokio::task::spawn_blocking(move || {
            pages.source(&query.device, &query.checkout, &query.view, load)
        })
        .await
        .map_err(|_| "core_unavailable")??
        .ok_or("view_unavailable")?;
        // The page loads on the machine its source names, which for a page
        // of Desk's desk is the session's that opened it, not the desk's.
        let own = self.pages.own_node() == source.device_id;
        let screen = matches!(key.owner, Owner::Screen { .. });
        if screen {
            // A view's web page loads its address through this machine's
            // proxy; only a file, this machine's or a device's, needs a
            // route here.
            if !hide_platform::path::is_file_url(&source.url) {
                if !own && loopback_page(&source.url) {
                    return Err("remote_device_page_unavailable");
                }
                return Ok((Resolved::of(&source, source.url.clone()), own));
            }
        } else if own {
            return Ok((Resolved::of(&source, source.url.clone()), true));
        }
        let mut routes = self.routes.lock().await;
        if let Some(entry) = routes.get(&key)
            && entry.source() == &source
            && entry.owner_started() == owner_started
            && entry.live()
        {
            match entry {
                RouteEntry::Ready(route) => {
                    return Ok((Resolved::of(&source, route.url.clone()), own));
                }
                RouteEntry::Pending { result, .. } => {
                    let waiting = result.subscribe();
                    drop(routes);
                    return wait_for_route(waiting)
                        .await
                        .map(|resolved| (resolved, own));
                }
            }
        }
        // A guest holds one view at a time: its routes of an earlier one
        // reach nothing that view can load any more.
        let ended: Vec<RouteEntry> = if let Owner::Screen { guest, view } = &key.owner {
            let earlier: Vec<Key> = routes
                .keys()
                .filter(|held| matches!(&held.owner, Owner::Screen { guest: of, view: seen } if of == guest && seen != view))
                .cloned()
                .collect();
            earlier
                .iter()
                .filter_map(|held| routes.remove(held))
                .collect()
        } else {
            Vec::new()
        };
        for entry in ended {
            entry.close();
        }
        if !routes.contains_key(&key) && full(&routes, &key.owner) {
            return Err("browser_route_limit");
        }
        let slots = if screen {
            &self.screen_build_slots
        } else {
            &self.build_slots
        };
        let build_permit = Arc::clone(slots)
            .try_acquire_owned()
            .map_err(|_| "browser_route_build_limit")?;
        let old = routes.remove(&key);
        let (result, waiting) = watch::channel(None);
        routes.insert(
            key.clone(),
            RouteEntry::Pending {
                owner_started,
                source: source.clone(),
                result: result.clone(),
            },
        );
        drop(routes);
        if let Some(old) = old {
            old.close();
        }
        let routes = Arc::clone(self);
        tokio::spawn(async move {
            let _build_permit = build_permit;
            let built = routes.build_route(&key, &source, owner_started).await;
            routes
                .finish_route(key, source, owner_started, result, built)
                .await;
        });
        wait_for_route(waiting)
            .await
            .map(|resolved| (resolved, own))
    }

    async fn build_route(
        &self,
        key: &Key,
        source: &BrowserRouteSource,
        owner_started: u64,
    ) -> Result<Option<Route>, &'static str> {
        let pages = Arc::clone(&self.pages);
        let device = source.device_id.clone();
        let way = tokio::task::spawn_blocking(move || pages.way(&device))
            .await
            .map_err(|_| "core_unavailable")??;
        let (url, kind) = if hide_platform::path::is_file_url(&source.url) {
            let channel: Arc<dyn NodeLink> = match &way {
                Way::Device(link) => Arc::new(link.clone()),
                Way::Own(link) => Arc::clone(link),
                Way::Core(_) => return Err("file_page_unavailable"),
            };
            let (path, suffix) =
                hide_platform::path::from_file_url(&source.url).ok_or("invalid_file_url")?;
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
                screens: self.screens.clone(),
                key: key.clone(),
                owner_started,
                source: source.clone(),
                channel,
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
                    hide_core::diagnostic!(
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
            let link = match way {
                Way::Device(link) => PageLink::Device(link),
                Way::Core(mux) => PageLink::Core(mux),
                // This machine's own loopback page loads its address.
                Way::Own(_) => return Ok(None),
            };
            let forwarded = PageForward::start(link, remote, alternate, preserve_numeric_host)
                .await
                .map(|forward| (forward.local_addr(), RouteKind::Page(forward)))
                .map_err(|error| error.to_string());
            let (local_addr, kind) = forwarded.map_err(|reason| {
                hide_core::diagnostic!(serde_json::json!({"component":"browser_routes","kind":"forward.failed","reason":reason}));
                "route_failed"
            })?;
            let url = forwarded_url(&scheme, &host, local_addr, &tail);
            (url, kind)
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
        let current = if built.is_ok() && self.alive(&key.owner, owner_started) {
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
            hide_core::diagnostic!(
                serde_json::json!({"component":"browser_routes","kind":"route.discarded","reason":"reservation_superseded","device_id":source.device_id,"load":source.load})
            );
            return;
        }
        let outcome = match (current, built) {
            (true, Ok(Some(route))) => {
                let resolved = Resolved::of(&source, route.url.clone());
                routes.insert(key, RouteEntry::Ready(route));
                hide_core::diagnostic!(
                    serde_json::json!({"component":"browser_routes","kind":"route.ready","device_id":source.device_id,"load":source.load})
                );
                Ok(resolved)
            }
            (true, Ok(None)) => {
                routes.remove(&key);
                Ok(Resolved::of(&source, source.url.clone()))
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
            owner: Owner::Process(owner_pid),
            device: device.to_owned(),
            checkout: checkout.to_owned(),
            view: view.to_owned(),
        };
        self.release_key(&key, load).await;
    }

    /// Lets go of the route `guest`'s view `view` holds for display
    /// `display` at `load`: its page in that view closed.
    pub async fn release_for_screen(
        &self,
        device: &str,
        checkout: &str,
        display: &str,
        load: u64,
        guest: PeerKey,
        view: &str,
    ) {
        let key = Key {
            owner: Owner::Screen {
                guest,
                view: view.to_owned(),
            },
            device: device.to_owned(),
            checkout: checkout.to_owned(),
            view: display.to_owned(),
        };
        self.release_key(&key, load).await;
    }

    async fn release_key(&self, key: &Key, load: u64) {
        let mut routes = self.routes.lock().await;
        let removed = if routes
            .get(key)
            .is_some_and(|entry| entry.source().load == load)
        {
            routes.remove(key)
        } else {
            None
        };
        drop(routes);
        if let Some(entry) = removed {
            entry.close();
        }
    }

    /// Closes every route whose owner, View, desktop or node link is gone,
    /// every 2 s until `shutdown`; the caller may also end it by aborting
    /// the task. A view's routes need no desktop of this machine: they end
    /// with the view.
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
                        (
                            key.clone(),
                            entry.source().clone(),
                            entry.owner_started(),
                            entry.live(),
                        )
                    })
                    .collect();
                for (key, source, owner_started, live) in candidates {
                    let load = source.load;
                    let desktop_gone = matches!(key.owner, Owner::Process(_))
                        && desktop_renderers.load(Ordering::SeqCst) == 0;
                    let stale = if !live || desktop_gone || !routes.alive(&key.owner, owner_started)
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
                        routes.release_key(&key, load).await;
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

/// Whether `path` names a place a page of a View can stand: a checkout's
/// absolute path, or Desk's own desk, the one View tree no checkout holds,
/// keyed by Desk's caller name (PRD desk-own-desk D-01).
pub(crate) fn is_page_checkout(path: &str) -> bool {
    hide_platform::path::is_wire_absolute(path) || path == hide_core::workspace_control::DESK_CALLER
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
            && is_page_checkout(&self.checkout_path)
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
            hide_core::diagnostic!(
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

/// Whether `owner` holds as many routes as it may: this machine's desktops
/// `MAX_ROUTES` together, and each guest's view `MAX_SCREEN_ROUTES` of its
/// own.
fn full(routes: &HashMap<Key, RouteEntry>, owner: &Owner) -> bool {
    match owner {
        Owner::Process(_) => {
            routes
                .keys()
                .filter(|held| matches!(held.owner, Owner::Process(_)))
                .count()
                >= MAX_ROUTES
        }
        Owner::Screen { guest, .. } => {
            routes
                .keys()
                .filter(
                    |held| matches!(&held.owner, Owner::Screen { guest: of, .. } if of == guest),
                )
                .count()
                >= MAX_SCREEN_ROUTES
        }
    }
}

async fn wait_for_route(mut result: watch::Receiver<Option<RouteResult>>) -> RouteResult {
    loop {
        if let Some(outcome) = result.borrow_and_update().clone() {
            return outcome;
        }
        result.changed().await.map_err(|_| "route_failed")?;
    }
}

/// Whether `raw` is a page on its machine's loopback, as a device's page
/// route reads one: a remote screen's route answer refuses another device's.
pub(crate) fn loopback_page(raw: &str) -> bool {
    loopback_target(raw).is_some()
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
        "0.0.0.0" | "[::ffff:0.0.0.0]" | "[::ffff:0:0]" => "127.0.0.1",
        "::" | "[::]" => "[::1]",
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
    screens: Option<Arc<dyn ScreenViews>>,
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
    let screens = route.screens.clone();
    let source = route.source.clone();
    let key = route.key.clone();
    let owner_started = route.owner_started;
    let channel = Arc::clone(&route.channel);
    let decoded = decoded.into_owned();
    let entry = route.entry.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if !owner_alive(screens.as_deref(), &key.owner, owner_started) {
            // A view that ended has nothing more of this machine's.
            return Err(match key.owner {
                Owner::Process(_) => "browser_owner_unavailable",
                Owner::Screen { .. } => "view_unavailable",
            });
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
            // The page's own checkout, which holds its file even when
            // another checkout's Workspace shows it.
            let range = node_access::read_bytes(
                channel.as_ref(),
                &source.checkout_path,
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
            hide_core::diagnostic!(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn route_request(checkout_path: &str) -> RouteRequest {
        RouteRequest {
            device_id: "node-1".to_owned(),
            checkout_path: checkout_path.to_owned(),
            id: "view-1".to_owned(),
            load: 1,
            owner_pid: 42,
        }
    }

    #[test]
    fn a_page_of_desks_own_desk_is_a_route_the_host_may_ask_for() {
        assert!(is_page_checkout("hide:desk"));
        assert!(route_request("hide:desk").valid());
        assert!(route_request("/work/repo").valid());
        assert!(!route_request("hide:other").valid());
        assert!(!route_request("relative/repo").valid());
    }

    #[test]
    fn every_admitted_loopback_address_has_a_loopback_target() {
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

    /// A machine whose own node is `host`, whose one display is `source`,
    /// read off disk through this process's own node.
    struct OneDisplay {
        source: std::sync::Mutex<Option<BrowserRouteSource>>,
    }

    impl PageSources for OneDisplay {
        fn own_node(&self) -> &str {
            "host"
        }

        fn source(
            &self,
            _device: &str,
            _checkout: &str,
            _view: &str,
            load: u64,
        ) -> Result<Option<BrowserRouteSource>, &'static str> {
            let source = self.source.lock().unwrap().clone();
            Ok(source.filter(|source| source.load == load))
        }

        fn way(&self, device: &str) -> Result<Way, &'static str> {
            if device == "host" {
                Ok(Way::Own(Arc::new(hide_node::Local::of_process())))
            } else {
                Err("host_unavailable")
            }
        }
    }

    /// The views open now, by view id.
    #[derive(Default)]
    struct Views(std::sync::Mutex<std::collections::HashSet<String>>);

    impl ScreenViews for Views {
        fn live(&self, _guest: &PeerKey, view: &str) -> bool {
            self.0.lock().unwrap().contains(view)
        }
    }

    fn guest(byte: u8) -> PeerKey {
        PeerKey::from_bytes([byte; 32])
    }

    fn display(device: &str, checkout: &str, url: &str, load: u64) -> BrowserRouteSource {
        BrowserRouteSource {
            device_id: device.to_owned(),
            checkout_path: checkout.to_owned(),
            url: url.to_owned(),
            load,
        }
    }

    /// A registry over `source`, with the views `open`.
    fn registry(
        source: BrowserRouteSource,
        open: &[&str],
    ) -> (Arc<BrowserRoutes>, Arc<OneDisplay>, Arc<Views>) {
        let pages = Arc::new(OneDisplay {
            source: std::sync::Mutex::new(Some(source)),
        });
        let views = Arc::new(Views::default());
        views
            .0
            .lock()
            .unwrap()
            .extend(open.iter().map(|view| (*view).to_owned()));
        let routes = BrowserRoutes::new(
            Arc::clone(&pages) as Arc<dyn PageSources>,
            Some(Arc::clone(&views) as Arc<dyn ScreenViews>),
        );
        (routes, pages, views)
    }

    /// The status a GET of `url`'s `path` (the route's own prefix and on)
    /// answers, with the body and its policy header.
    async fn fetch(url: &str, path: &str) -> (u16, Vec<u8>, Option<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let address = url
            .strip_prefix("http://")
            .and_then(|rest| rest.split('/').next())
            .expect("an http route");
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).await.unwrap();
        let end = answer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("a head");
        let head = String::from_utf8_lossy(&answer[..end]).into_owned();
        let status = head
            .split(' ')
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap();
        let policy = head
            .lines()
            .find_map(|line| line.strip_prefix("content-security-policy: "))
            .map(str::to_owned);
        (status, answer[end + 4..].to_vec(), policy)
    }

    /// The route's own prefix, `/<secret>/`.
    fn prefix(url: &str) -> String {
        let path = url.split_once("://").unwrap().1;
        let secret = path.split('/').nth(1).unwrap();
        format!("/{secret}/")
    }

    // docs/BROWSER_DISPLAYS.md, Its checkout files: a file page of this
    // machine that another machine's view draws loads from a route on this
    // machine's loopback the view owns, read through this machine's own node
    // from the checkout that holds it: the page and the files its HTML and
    // CSS declare, under the same policy as a device's file page. A file the
    // page does not declare (`.env` beside it), a path out of the checkout,
    // and a declared file that is a link out of it are refused; once the
    // view ends the route answers nothing more (TF5).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_view_s_file_route_serves_its_page_and_declared_files_alone() {
        let place = tempfile::tempdir().unwrap();
        let checkout = place.path().join("repo");
        std::fs::create_dir_all(checkout.join("img")).unwrap();
        std::fs::write(
            checkout.join("report.html"),
            r#"<link rel="stylesheet" href="style.css"><img src="img/chart.png"><img src="leak.png">"#,
        )
        .unwrap();
        std::fs::write(
            checkout.join("style.css"),
            "body { background: url(img/bg.png) }",
        )
        .unwrap();
        std::fs::write(checkout.join("img/chart.png"), b"png").unwrap();
        std::fs::write(checkout.join("img/bg.png"), b"bg").unwrap();
        std::fs::write(checkout.join(".env"), "SECRET=1").unwrap();
        std::fs::write(place.path().join("outside.txt"), "outside").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(place.path().join("outside.txt"), checkout.join("leak.png"))
            .unwrap();
        let root = hide_platform::path::to_wire_lossy(&checkout);
        let page = hide_platform::path::to_file_url(&format!("{root}/report.html"), "");
        let (routes, pages, views) = registry(display("host", &root, &page, 3), &["v1"]);

        let (resolved, own) = routes
            .resolve_for_screen(
                "host".into(),
                root.clone(),
                "d1".into(),
                3,
                guest(1),
                "v1".into(),
            )
            .await
            .expect("a route for the view");
        assert!(own, "this machine's own file page");
        assert!(
            resolved.url.starts_with("http://127.0.0.1:"),
            "{}",
            resolved.url
        );
        assert!(resolved.url.ends_with("/report%2Ehtml"), "{}", resolved.url);
        assert_eq!(resolved.source_url, page);
        assert_eq!(resolved.root.as_deref(), Some(root.as_str()));
        let at = prefix(&resolved.url);

        let (status, body, policy) = fetch(&resolved.url, &format!("{at}report.html")).await;
        assert_eq!(status, 200);
        assert!(body.starts_with(b"<link"));
        assert!(policy.is_some_and(|policy| policy.contains("connect-src 'none'")));
        assert_eq!(fetch(&resolved.url, &format!("{at}style.css")).await.0, 200);
        assert_eq!(
            fetch(&resolved.url, &format!("{at}img/chart.png")).await.0,
            200
        );
        // Declared by the stylesheet, once it was read.
        assert_eq!(
            fetch(&resolved.url, &format!("{at}img/bg.png")).await.0,
            200
        );
        assert_eq!(fetch(&resolved.url, &format!("{at}.env")).await.0, 403);
        assert_eq!(
            fetch(&resolved.url, &format!("{at}../outside.txt")).await.0,
            403
        );
        assert_eq!(
            fetch(&resolved.url, &format!("{at}%2e%2e/outside.txt"))
                .await
                .0,
            403
        );
        assert_eq!(
            fetch(&resolved.url, "/report.html").await.0,
            404,
            "no secret, no page"
        );
        #[cfg(unix)]
        assert_eq!(
            fetch(&resolved.url, &format!("{at}leak.png")).await.0,
            403,
            "a declared link out of the checkout"
        );

        // The View's next load: the route answers nothing of the last.
        pages.source.lock().unwrap().as_mut().unwrap().load = 4;
        assert_eq!(
            fetch(&resolved.url, &format!("{at}report.html")).await.0,
            410
        );
        pages.source.lock().unwrap().as_mut().unwrap().load = 3;
        assert_eq!(
            fetch(&resolved.url, &format!("{at}report.html")).await.0,
            200
        );
        // The view ends (its tile hid): nothing more, and no new route.
        views.0.lock().unwrap().clear();
        assert_eq!(
            fetch(&resolved.url, &format!("{at}report.html")).await.0,
            410
        );
        assert_eq!(
            routes
                .resolve_for_screen("host".into(), root, "d1".into(), 3, guest(1), "v1".into())
                .await
                .err(),
            Some("view_unavailable")
        );
    }

    // A view's pages: this machine's web pages load their address as its
    // own; a device's loopback page is refused, since the view keeps no
    // login storage apart for a device's pages; a file outside the checkout
    // that names it gets no route. A guest's next view takes its earlier
    // view's routes with it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_view_s_pages_load_from_this_machine_or_are_refused() {
        let place = tempfile::tempdir().unwrap();
        let root = hide_platform::path::to_wire_lossy(place.path());
        let (routes, pages, views) = registry(
            display("host", &root, "http://localhost:5173/", 1),
            &["v1", "v2"],
        );
        let ask = |load: u64, view: &str| {
            routes.resolve_for_screen(
                "host".into(),
                root.clone(),
                "d1".into(),
                load,
                guest(1),
                view.into(),
            )
        };
        let (resolved, own) = ask(1, "v1").await.unwrap();
        assert_eq!(
            (resolved.url.as_str(), own),
            ("http://localhost:5173/", true)
        );

        *pages.source.lock().unwrap() =
            Some(display("device-c", &root, "http://127.0.0.1:3000/", 2));
        assert_eq!(
            ask(2, "v1").await.err(),
            Some("remote_device_page_unavailable")
        );
        *pages.source.lock().unwrap() = Some(display("device-c", &root, "https://example.com/", 3));
        let (resolved, own) = ask(3, "v1").await.unwrap();
        assert_eq!(
            (resolved.url.as_str(), own),
            ("https://example.com/", false)
        );

        let outside = hide_platform::path::to_file_url("/etc/hosts", "");
        *pages.source.lock().unwrap() = Some(display("host", &root, &outside, 4));
        assert_eq!(ask(4, "v1").await.err(), Some("outside_checkout"));

        std::fs::write(place.path().join("a.html"), "a").unwrap();
        let page = hide_platform::path::to_file_url(&format!("{root}/a.html"), "");
        *pages.source.lock().unwrap() = Some(display("host", &root, &page, 5));
        let (first, _) = ask(5, "v1").await.unwrap();
        assert_eq!(
            fetch(&first.url, &format!("{}a.html", prefix(&first.url)))
                .await
                .0,
            200
        );
        let (second, _) = ask(5, "v2").await.unwrap();
        assert_ne!(first.url, second.url);
        assert_eq!(
            fetch(&second.url, &format!("{}a.html", prefix(&second.url)))
                .await
                .0,
            200
        );
        views.0.lock().unwrap().remove("v1");
        assert_eq!(
            routes.routes.lock().await.len(),
            1,
            "the guest's earlier view's route went when its next view asked"
        );
    }

    // TF4: a guest's view holds eight routes of its own and this machine's
    // desktops twelve, so neither takes the other's places, nor one guest
    // another's.
    #[test]
    fn eight_routes_of_a_view_leave_the_desktop_s_twelve() {
        let source = display("device-c", "/w", "file:///w/a.html", 1);
        let entry = || {
            let (result, _) = watch::channel(None);
            RouteEntry::Pending {
                owner_started: 0,
                source: source.clone(),
                result,
            }
        };
        let key = |owner: Owner, n: usize| Key {
            owner,
            device: "device-c".into(),
            checkout: "/w".into(),
            view: format!("d{n}"),
        };
        let screen = |byte: u8| Owner::Screen {
            guest: guest(byte),
            view: "v".into(),
        };
        let mut routes = HashMap::new();
        for n in 0..MAX_SCREEN_ROUTES {
            routes.insert(key(screen(1), n), entry());
        }
        assert!(full(&routes, &screen(1)));
        assert!(!full(&routes, &screen(2)));
        assert!(!full(&routes, &Owner::Process(7)));
        for n in 0..MAX_ROUTES {
            routes.insert(key(Owner::Process(7), n), entry());
        }
        assert!(full(&routes, &Owner::Process(8)));
        assert!(!full(&routes, &screen(2)));
    }
}
