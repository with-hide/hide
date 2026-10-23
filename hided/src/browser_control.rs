//! App-lifetime browser control registration. Only the authenticated desktop
//! registers a loopback gateway; a pane receives a checkout-scoped capability.
//!
//! The core also keeps the gateways a linked node's daemon announced for the
//! desktop windows on its machine (PRD core-host-node-remote-core B4, B13,
//! B15), and chooses for each caller: the window on the caller's own
//! machine, else the core's own window, else the one node window there is.
//! Several of a kind refuse as ambiguous. A node's gateway is reached only
//! through its link, and is forgotten when the link ends.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::Uri;
use herdr_core::workspace_control::QueryResult;
use hide_node::pane_proof::process_start;
use hide_node::ssh::RemoteHost;
use hide_node_link::protocol::Call;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

const MAX_HOSTS: usize = 4;
const MAX_REQUESTS: usize = 8;
/// One relay per `hide browser` command in flight. A relay holds one of the
/// daemon's eight client connections for its whole life, so half stay free
/// for the shell and other commands, and the gateway's eight CDP clients for
/// external tools.
const MAX_RELAYS: usize = 4;
const MAX_ANSWER_BYTES: u64 = 16 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
/// How long a node has to answer for its gateway: the gateway's own
/// request, and the link around it.
const NODE_TIMEOUT: Duration = Duration::from_secs(10);
/// Linked nodes whose windows the core keeps at once.
const MAX_NODES: usize = 16;

pub type Failure = (&'static str, &'static str);

/// The answer to a caller whose window's capability URLs are on another
/// machine's loopback: its page commands reach the window through Hide.
const ELSEWHERE: Failure = (
    "browser_control_elsewhere",
    "Use hide browser page commands, which reach that window through Hide",
);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    pub owner_pid: i32,
    pub endpoint: String,
    pub token: String,
}

#[derive(Clone)]
struct Host {
    started: u64,
    endpoint: String,
    token: String,
    caller_key: String,
}

/// The desktop windows a linked node's daemon announced, by owner pid, each
/// with the caller key its page actions run as.
struct NodeWindows {
    link: RemoteHost,
    owners: Vec<(i32, String)>,
}

pub struct BrowserControl {
    hosts: Mutex<HashMap<i32, Host>>,
    nodes: Mutex<HashMap<String, NodeWindows>>,
    slots: Arc<Semaphore>,
    relays: Arc<Semaphore>,
}

impl Default for BrowserControl {
    fn default() -> Self {
        Self {
            hosts: Mutex::new(HashMap::new()),
            nodes: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(MAX_REQUESTS)),
            relays: Arc::new(Semaphore::new(MAX_RELAYS)),
        }
    }
}

/// The window a caller's browser connection goes to.
enum Window {
    /// The window on the caller's own machine, a linked node, reached by
    /// asking that node over its link.
    Own(RemoteHost),
    /// The core's own window; a capability URL of it is the core machine's
    /// loopback, so only a caller on that machine is handed one.
    Core(Host),
    /// The one window there is, on another linked node.
    Elsewhere(RemoteHost),
}

/// Where a `hide browser` page command's relay runs.
pub enum Relay {
    /// From the core to its own window's gateway, at this capability URL.
    Core(String),
    /// On the caller's own machine: the node's one-shot relay URL, which
    /// the caller dials itself.
    Own(String),
    /// From the core through `link` to that node's relay URL.
    Through { link: RemoteHost, relay_url: String },
}

fn unavailable() -> Failure {
    (
        "browser_control_unavailable",
        "Reconnect the Hide desktop app and retry",
    )
}

fn supported_address(address: &str) -> Result<(), Failure> {
    if address == "about:blank"
        || address.parse::<Uri>().is_ok_and(|uri| {
            matches!(uri.scheme_str(), Some("http" | "https")) && uri.host().is_some()
        })
    {
        return Ok(());
    }
    Err((
        "browser_address_unsupported",
        "Use an http or https browser display; native file pages do not support CDP",
    ))
}

fn live(hosts: &mut HashMap<i32, Host>) {
    hosts.retain(|pid, host| process_start(*pid) == Some(host.started));
}

/// The windows of nodes whose link still lives. One whose link ended keeps
/// its entry, unseen, until its node announces again over a new link (so
/// each window keeps its caller key, and a retried action stays one
/// action) or room is needed for another node.
fn linked(nodes: &HashMap<String, NodeWindows>) -> impl Iterator<Item = (&String, &NodeWindows)> {
    nodes
        .iter()
        .filter(|(_, windows)| windows.link.closed_reason().is_none())
}

fn ambiguous() -> Failure {
    (
        "browser_control_ambiguous",
        "Keep one desktop window attached to this daemon and retry",
    )
}

/// A reason a node's daemon refused with, as this daemon would have: the
/// node answers one of these codes as the whole message, and anything else
/// reads as an unavailable window.
fn node_failure(message: &str) -> Failure {
    [
        unavailable(),
        ambiguous(),
        (
            "browser_control_busy",
            "Wait for an earlier browser request to finish and retry",
        ),
        (
            "browser_relay_limit",
            "Wait for another hide browser command to finish and retry",
        ),
        (
            "browser_display_missing",
            "Run hide view list and choose a browser display",
        ),
    ]
    .into_iter()
    .find(|(reason, _)| *reason == message)
    .unwrap_or_else(unavailable)
}

/// The window's machine did not answer over its link.
fn unreachable_window() -> Failure {
    (
        "browser_window_unreachable",
        "Retry once the machine that shows this page is reachable again",
    )
}

/// Numeric IPv4 loopback only, with no credentials, path, query or redirect.
/// This is the private registration URL, never a public capability URL.
fn endpoint_valid(endpoint: &str) -> bool {
    let Ok(uri) = endpoint.parse::<Uri>() else {
        return false;
    };
    uri.scheme_str() == Some("http")
        && uri.host() == Some("127.0.0.1")
        && uri.port_u16().is_some_and(|port| port != 0)
        && uri
            .authority()
            .is_some_and(|authority| !authority.as_str().contains('@'))
        && matches!(
            uri.path_and_query().map(|value| value.as_str()),
            None | Some("/")
        )
}

impl BrowserControl {
    pub fn register(&self, registration: Registration) -> Result<(), Failure> {
        if !endpoint_valid(&registration.endpoint)
            || registration.token.len() < 32
            || registration.token.len() > 128
            || !registration
                .token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err((
                "invalid_browser_control",
                "Use the desktop app's loopback gateway",
            ));
        }
        let started = process_start(registration.owner_pid).ok_or_else(unavailable)?;
        let endpoint = registration.endpoint.trim_end_matches('/');
        let mut hosts = self.hosts.lock().map_err(|_| unavailable())?;
        live(&mut hosts);
        if let Some(host) = hosts.get(&registration.owner_pid) {
            if host.started == started
                && host.endpoint == endpoint
                && host.token == registration.token
            {
                return Ok(());
            }
            return Err((
                "browser_control_conflict",
                "Reconnect this desktop window before registering its gateway",
            ));
        }
        if hosts.len() >= MAX_HOSTS {
            return Err((
                "browser_control_limit",
                "Close an unused Hide desktop window and retry",
            ));
        }
        let caller_key = crate::state_file::new_token();
        hosts.insert(
            registration.owner_pid,
            Host {
                started,
                endpoint: endpoint.to_owned(),
                token: registration.token,
                caller_key,
            },
        );
        Ok(())
    }

    pub fn release(&self, registration: &Registration) -> Result<(), Failure> {
        let mut hosts = self.hosts.lock().map_err(|_| unavailable())?;
        // A stale app cannot remove a later app's registration after PID reuse.
        if hosts.get(&registration.owner_pid).is_some_and(|host| {
            host.endpoint == registration.endpoint.trim_end_matches('/')
                && crate::server::token_matches(&registration.token, &host.token)
        }) {
            hosts.remove(&registration.owner_pid);
        }
        Ok(())
    }

    pub fn caller(&self, pid: i32, checkout_path: &str) -> Result<String, Failure> {
        let mut hosts = self.hosts.lock().map_err(|_| unavailable())?;
        live(&mut hosts);
        let host = hosts.get(&pid).ok_or_else(unavailable)?;
        Ok(herdr_core::workspace_control::gateway_caller_id(
            &host.caller_key,
            checkout_path,
        ))
    }

    pub fn acquire(&self) -> Result<tokio::sync::OwnedSemaphorePermit, Failure> {
        self.slots.clone().try_acquire_owned().map_err(|_| {
            (
                "browser_control_busy",
                "Wait for an earlier browser request to finish and retry",
            )
        })
    }

    /// Held for a relay's whole life, so a crossed cap refuses the next one
    /// with its reason instead of queueing it.
    pub fn acquire_relay(&self) -> Result<tokio::sync::OwnedSemaphorePermit, Failure> {
        self.relays.clone().try_acquire_owned().map_err(|_| {
            (
                "browser_relay_limit",
                "Wait for another hide browser command to finish and retry",
            )
        })
    }

    /// The owner pids of the windows registered here and still running.
    pub fn owners(&self) -> Vec<i32> {
        let Ok(mut hosts) = self.hosts.lock() else {
            return Vec::new();
        };
        live(&mut hosts);
        let mut owners: Vec<i32> = hosts.keys().copied().collect();
        owners.sort_unstable();
        owners
    }

    /// Whether `pid` registered a window here that still runs.
    pub fn registered(&self, pid: i32) -> Result<(), Failure> {
        let mut hosts = self.hosts.lock().map_err(|_| unavailable())?;
        live(&mut hosts);
        hosts
            .contains_key(&pid)
            .then_some(())
            .ok_or_else(unavailable)
    }

    /// The windows a linked node's daemon has now, as it announced them
    /// over `link`; a window it announced before keeps its caller key.
    pub fn announce(&self, node: &str, link: RemoteHost, owners: Vec<i32>) -> Result<(), Failure> {
        if owners.len() > MAX_HOSTS || owners.iter().any(|pid| *pid <= 0) {
            return Err((
                "invalid_browser_control",
                "Use the desktop app's loopback gateway",
            ));
        }
        let mut nodes = self.nodes.lock().map_err(|_| unavailable())?;
        let before = nodes
            .remove(node)
            .map(|windows| windows.owners)
            .unwrap_or_default();
        if owners.is_empty() {
            return Ok(());
        }
        if nodes.len() >= MAX_NODES {
            nodes.retain(|_, windows| windows.link.closed_reason().is_none());
        }
        if nodes.len() >= MAX_NODES {
            return Err((
                "browser_control_limit",
                "Close an unused Hide desktop window and retry",
            ));
        }
        let owners = owners
            .into_iter()
            .map(|pid| {
                let key = before
                    .iter()
                    .find(|(known, _)| *known == pid)
                    .map_or_else(crate::state_file::new_token, |(_, key)| key.clone());
                (pid, key)
            })
            .collect();
        nodes.insert(node.to_owned(), NodeWindows { link, owners });
        Ok(())
    }

    /// The caller a linked node's window acts as on `checkout_path`.
    pub fn node_caller(
        &self,
        node: &str,
        pid: i32,
        checkout_path: &str,
    ) -> Result<String, Failure> {
        let nodes = self.nodes.lock().map_err(|_| unavailable())?;
        let (_, key) = linked(&nodes)
            .find(|(name, _)| *name == node)
            .and_then(|(_, windows)| windows.owners.iter().find(|(owner, _)| *owner == pid))
            .ok_or_else(unavailable)?;
        Ok(herdr_core::workspace_control::gateway_caller_id(
            key,
            checkout_path,
        ))
    }

    /// The one window registered here, if any; several are ambiguous.
    fn own_host(&self) -> Result<Option<Host>, Failure> {
        let mut hosts = self.hosts.lock().map_err(|_| unavailable())?;
        live(&mut hosts);
        match hosts.len() {
            0 => Ok(None),
            1 => Ok(hosts.values().next().cloned()),
            _ => Err(ambiguous()),
        }
    }

    /// The link of `node`'s one window, if it announced any.
    fn node_window(&self, node: &str) -> Result<Option<RemoteHost>, Failure> {
        let nodes = self.nodes.lock().map_err(|_| unavailable())?;
        match linked(&nodes)
            .find(|(name, _)| *name == node)
            .map(|(_, windows)| windows)
        {
            None => Ok(None),
            Some(windows) if windows.owners.len() == 1 => Ok(Some(windows.link.clone())),
            Some(_) => Err(ambiguous()),
        }
    }

    /// The link of the one node window there is, if any.
    fn only_node_window(&self) -> Result<Option<RemoteHost>, Failure> {
        let nodes = self.nodes.lock().map_err(|_| unavailable())?;
        let mut windows = linked(&nodes).map(|(_, windows)| windows);
        match (windows.next(), windows.next()) {
            (None, _) => Ok(None),
            (Some(only), None) if only.owners.len() == 1 => Ok(Some(only.link.clone())),
            _ => Err(ambiguous()),
        }
    }

    /// The gateway capability `scope` names, from the window registered
    /// here. Its URLs stay on this machine's loopback.
    pub fn gateway_capability(&self, scope: &Value) -> Result<(String, String), Failure> {
        let host = self.own_host()?.ok_or_else(unavailable)?;
        ask_gateway(&host, scope)
    }

    /// The capability for the operator's Comment mode on `scope`'s display,
    /// which the gateway grants for a local `file:` page too, and for the
    /// comment session's own commands only. It stays in hided: no result
    /// built for a pane (`connect`, `relay`) carries it, and the scope a
    /// pane's request makes (`scope_of`) never names a purpose.
    pub fn comment_capability(&self, scope: &Value) -> Result<(String, String), Failure> {
        let host = self.own_host()?.ok_or_else(unavailable)?;
        let mut scope = scope.clone();
        scope["purpose"] = json!("comment");
        ask_gateway(&host, &scope)
    }

    /// Asks every window registered here to drop each capability it handed
    /// out, the link they were handed out over having ended: all at once,
    /// each asked again a few times before its failure is logged. The
    /// desktop host's own lease (it revokes everything when its daemon
    /// changes or goes) is the backstop for one that never answers.
    #[allow(clippy::disallowed_methods)] // a production retry wait, not test code
    pub fn revoke_all(&self) {
        let hosts: Vec<(i32, Host)> = match self.hosts.lock() {
            Ok(mut hosts) => {
                live(&mut hosts);
                hosts
                    .iter()
                    .map(|(pid, host)| (*pid, host.clone()))
                    .collect()
            }
            Err(_) => return,
        };
        std::thread::scope(|scope| {
            for (pid, host) in &hosts {
                scope.spawn(move || {
                    let mut failure = String::new();
                    for attempt in 0..REVOKE_ATTEMPTS {
                        if attempt > 0 {
                            std::thread::sleep(REVOKE_RETRY);
                        }
                        let revoked = revoke_agent()
                            .post(format!("{}/revoke", host.endpoint))
                            .header("Authorization", format!("Bearer {}", host.token))
                            .send_empty();
                        match revoked {
                            Ok(_) => return,
                            Err(error) => failure = error.to_string(),
                        }
                    }
                    herdr_core::diagnostic!(json!({
                        "component": "browser_control",
                        "kind": "revoke.failed",
                        "owner_pid": pid,
                        "attempts": REVOKE_ATTEMPTS,
                        "reason": failure,
                    }));
                });
            }
        });
    }

    /// The window `caller_node`'s caller reaches: the one on its own
    /// machine, else the core's own, else the one node window there is.
    fn window(&self, caller_node: Option<&str>) -> Result<Window, Failure> {
        if let Some(link) = caller_node
            .map(|node| self.node_window(node))
            .transpose()?
            .flatten()
        {
            return Ok(Window::Own(link));
        }
        if let Some(host) = self.own_host()? {
            return Ok(Window::Core(host));
        }
        self.only_node_window()?
            .map(Window::Elsewhere)
            .ok_or_else(unavailable)
    }

    /// `hide browser connect` and `open`: the capability URLs for
    /// `display_id` (or the active area) of `source`'s checkout, for a
    /// caller that reached the core through `caller_node`'s link if it
    /// names one.
    pub fn connect(
        &self,
        source: &QueryResult,
        display_id: Option<&str>,
        caller_node: Option<&str>,
    ) -> Result<Value, Failure> {
        let _slot = self.acquire()?;
        let (area_id, display_id, scope) = scope_of(source, display_id)?;
        let (http, ws) = match self.window(caller_node)? {
            Window::Own(link) => ask_node(&link, &scope)?,
            // A caller that came over a link is on another machine than the
            // core, so the core's own window answers it by relay only; the
            // gateway is not asked, because its URLs are the core's loopback.
            Window::Core(_) if caller_node.is_some() => return Err(ELSEWHERE),
            Window::Core(host) => ask_gateway(&host, &scope)?,
            // Its URLs are on another machine's loopback.
            Window::Elsewhere(_) => return Err(ELSEWHERE),
        };
        // Return only the bounded public contract, never a registration token
        // or arbitrary fields the desktop sent back.
        Ok(
            json!({"context":source.context,"area_id":area_id,"display_id":display_id,
            "cdp_http_url":http,"browser_ws_url":ws}),
        )
    }

    /// A `hide browser` page command's relay to `display_id` of `source`'s
    /// checkout.
    pub fn relay(
        &self,
        source: &QueryResult,
        display_id: &str,
        caller_node: Option<&str>,
    ) -> Result<Relay, Failure> {
        let _slot = self.acquire()?;
        let (_, _, scope) = scope_of(source, Some(display_id))?;
        Ok(match self.window(caller_node)? {
            Window::Own(link) => Relay::Own(ask_node_relay(&link, &scope)?),
            Window::Core(host) => Relay::Core(ask_gateway(&host, &scope)?.1),
            Window::Elsewhere(link) => Relay::Through {
                relay_url: ask_node_relay(&link, &scope)?,
                link,
            },
        })
    }
}

/// The area, display and gateway scope of a connection to `display_id`, or
/// to `source`'s active area.
fn scope_of(
    source: &QueryResult,
    display_id: Option<&str>,
) -> Result<(String, Option<String>, Value), Failure> {
    let (area_id, display_id) = connection_scope(source, display_id)?;
    let workspace = format!(
        "{}\0{}",
        source.context.device_id, source.context.checkout_path
    );
    let mut scope = json!({"workspace":workspace,"area_id":area_id});
    if let Some(id) = &display_id {
        scope["display_id"] = json!(id);
    }
    Ok((area_id, display_id, scope))
}

/// A revoke is asked again rather than waited on.
const REVOKE_ATTEMPTS: u32 = 3;
const REVOKE_RETRY: Duration = Duration::from_millis(500);

fn revoke_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        .max_redirects(0)
        .proxy(None)
        .build()
        .into()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .max_redirects(0)
        .proxy(None)
        .build()
        .into()
}

/// The capability a registered gateway hands out for `scope`.
fn ask_gateway(host: &Host, scope: &Value) -> Result<(String, String), Failure> {
    let body = scope.to_string();
    let mut response = agent()
        .post(format!("{}/connect", host.endpoint))
        .header("Authorization", format!("Bearer {}", host.token))
        .header("Content-Type", "application/json")
        .send(body.as_bytes())
        .map_err(|_| unavailable())?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER_BYTES)
        .read_to_vec()
        .map_err(|_| unavailable())?;
    let answer: Value = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    let answer = answer.get("result").unwrap_or(&answer);
    let http = answer["cdp_http_url"].as_str().ok_or_else(unavailable)?;
    let ws = answer["browser_ws_url"].as_str().ok_or_else(unavailable)?;
    if !capability_url_valid(http, "http", &host.endpoint)
        || !capability_url_valid(ws, "ws", &host.endpoint)
        || http.contains(&host.token)
        || ws.contains(&host.token)
    {
        return Err(unavailable());
    }
    Ok((http.to_owned(), ws.to_owned()))
}

/// Asks a linked node's daemon about its window over `link`.
fn ask_link(link: &RemoteHost, scope: &Value, relay: bool) -> Result<Value, Failure> {
    let answer = link
        .call(
            Call::BrowserGateway {
                scope: scope.clone(),
                relay,
            },
            NODE_TIMEOUT,
        )
        .map_err(|error| {
            herdr_core::diagnostic!(json!({
                "component": "browser_control",
                "kind": "node_gateway.refused",
                "node": link.target(),
                "reason": error.to_string().chars().take(160).collect::<String>(),
            }));
            match error {
                hide_node_link::LinkError::Refused(refusal) => node_failure(&refusal.message),
                _ => unreachable_window(),
            }
        })?;
    match answer {
        hide_node_link::LinkAnswer::Parsed(value) => Ok(value),
        hide_node_link::LinkAnswer::Raw(raw) => {
            serde_json::from_str(raw.get()).map_err(|_| unavailable())
        }
    }
}

/// A linked node's gateway capability, on that node's loopback.
fn ask_node(link: &RemoteHost, scope: &Value) -> Result<(String, String), Failure> {
    let answer = ask_link(link, scope, false)?;
    let http = answer["cdp_http_url"].as_str().ok_or_else(unavailable)?;
    let ws = answer["browser_ws_url"].as_str().ok_or_else(unavailable)?;
    let endpoint = loopback_origin(http).ok_or_else(unavailable)?;
    if !capability_url_valid(http, "http", &endpoint) || !capability_url_valid(ws, "ws", &endpoint)
    {
        return Err(unavailable());
    }
    Ok((http.to_owned(), ws.to_owned()))
}

/// A linked node's one-shot relay URL, on that node's loopback.
fn ask_node_relay(link: &RemoteHost, scope: &Value) -> Result<String, Failure> {
    let answer = ask_link(link, scope, true)?;
    let url = answer["relay_url"].as_str().ok_or_else(unavailable)?;
    relay_url_valid(url)
        .then(|| url.to_owned())
        .ok_or_else(unavailable)
}

/// `http://127.0.0.1:<port>` of a URL on this machine's numeric loopback.
fn loopback_origin(url: &str) -> Option<String> {
    let uri = url.parse::<Uri>().ok()?;
    (uri.host() == Some("127.0.0.1") && !uri.authority()?.as_str().contains('@'))
        .then(|| {
            uri.port_u16()
                .map(|port| format!("http://127.0.0.1:{port}"))
        })
        .flatten()
}

/// A node's relay URL: its daemon's loopback port and one ticket.
pub fn relay_url_valid(url: &str) -> bool {
    let Ok(uri) = url.parse::<Uri>() else {
        return false;
    };
    uri.scheme_str() == Some("ws")
        && uri.host() == Some("127.0.0.1")
        && uri.port_u16().is_some_and(|port| port != 0)
        && uri
            .authority()
            .is_some_and(|authority| !authority.as_str().contains('@'))
        && uri.query().is_none()
        && uri
            .path()
            .strip_prefix("/browser-relay/")
            .is_some_and(|ticket| {
                ticket.len() >= 32
                    && ticket.len() <= 128
                    && ticket.bytes().all(|b| b.is_ascii_alphanumeric())
            })
}

fn capability_url_valid(value: &str, scheme: &str, endpoint: &str) -> bool {
    let Ok(uri) = value.parse::<Uri>() else {
        return false;
    };
    let Ok(base) = endpoint.parse::<Uri>() else {
        return false;
    };
    value.len() <= 2048
        && uri.scheme_str() == Some(scheme)
        && uri.authority() == base.authority()
        && uri.query().is_none()
        && uri.path().len() > 32
        && !uri.path().contains("..")
}

fn connection_scope(
    source: &QueryResult,
    display_id: Option<&str>,
) -> Result<(String, Option<String>), Failure> {
    let views = source.views.as_deref().ok_or_else(unavailable)?;
    if let Some(id) = display_id {
        let view = views
            .iter()
            .find(|view| view.view_id == id && (view.kind == "browser" || view.kind == "external"))
            .ok_or((
                "browser_display_missing",
                "Run hide view list and choose a browser display",
            ))?;
        // An outside file's page is the operator's click alone (PRD
        // outside-path-in-hide B18): refused as a checkout's file page is.
        if view.kind == "external" {
            return Err((
                "browser_address_unsupported",
                "Use an http or https browser display; native file pages do not support CDP",
            ));
        }
        supported_address(&view.target)?;
        return Ok((view.area_id.clone(), Some(view.view_id.clone())));
    }
    let view = views.iter().find(|view| view.active_area).ok_or((
        "view_area_missing",
        "Open this checkout's View area and retry",
    ))?;
    Ok((view.area_id.clone(), None))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserAction {
    pub owner_pid: i32,
    pub device_id: String,
    pub checkout_path: String,
    pub area_id: String,
    pub action: String,
    pub url: Option<String>,
    pub display_id: Option<String>,
    pub request_id: String,
}

impl BrowserAction {
    pub fn valid(&self) -> bool {
        self.owner_pid > 0
            && !self.device_id.is_empty()
            && self.device_id.len() <= 256
            && hide_platform::path::is_wire_absolute(&self.checkout_path)
            && self.checkout_path.len() <= 8192
            && !self.area_id.is_empty()
            && self.area_id.len() <= 256
            && crate::workspace_cli::valid_request_id(&self.request_id)
    }

    pub fn command(
        &self,
        source: &QueryResult,
    ) -> Result<herdr_core::workspace_control::Action, Failure> {
        use herdr_core::workspace_control::Action;
        if source.context.device_id != self.device_id
            || source.context.checkout_path != self.checkout_path
        {
            return Err((
                "workspace_changed",
                "Reconnect the browser capability and retry",
            ));
        }
        match self.action.as_str() {
            "open" if self.display_id.is_none() => {
                let url = self
                    .url
                    .clone()
                    .filter(|url| !url.is_empty())
                    .ok_or(("invalid_address", "Use an http or https address"))?;
                supported_address(&url)?;
                Ok(Action::OpenBrowser {
                    url,
                    area_id: Some(self.area_id.clone()),
                    new_target: true,
                })
            }
            "close" | "select" if self.url.is_none() => {
                let id = self.display_id.as_deref().ok_or((
                    "browser_display_missing",
                    "Choose a current browser display",
                ))?;
                Ok(if self.action == "close" {
                    Action::Close {
                        view_id: id.to_owned(),
                        expected_browser_area: Some(self.area_id.clone()),
                    }
                } else {
                    Action::Select {
                        view_id: id.to_owned(),
                        expected_browser_area: Some(self.area_id.clone()),
                    }
                })
            }
            _ => Err((
                "invalid_browser_action",
                "Use a scoped browser open, close, or select action",
            )),
        }
    }

    /// Check a fresh intent after the core has checked its retry record.
    /// A completed close can return its receipt after its page is gone.
    pub fn validate_scope(&self, source: &QueryResult) -> Result<(), Failure> {
        let views = source.views.as_deref().ok_or_else(unavailable)?;
        if !views.iter().any(|view| view.area_id == self.area_id) {
            return Err(("view_area_missing", "Reconnect from an existing View area"));
        }
        if let Some(id) = self.display_id.as_deref()
            && !views.iter().any(|view| {
                view.view_id == id && view.area_id == self.area_id && view.kind == "browser"
            })
        {
            return Err((
                "browser_display_missing",
                "Reconnect the browser capability and retry",
            ));
        }
        if let Some(id) = self.display_id.as_deref()
            && let Some(view) = views.iter().find(|view| view.view_id == id)
        {
            supported_address(&view.target)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> QueryResult {
        QueryResult {
            context: herdr_core::workspace_control::Context {
                device_id: "local".into(),
                workspace_id: "workspace".into(),
                checkout_id: "checkout".into(),
                checkout_path: "/checkout".into(),
            },
            capabilities: vec![],
            views: Some(vec![herdr_core::workspace_control::View {
                area_id: "area-a".into(),
                view_id: "browser-a".into(),
                kind: "browser",
                target: "about:blank".into(),
                selected: true,
                active_area: true,
                page: None,
            }]),
        }
    }

    #[test]
    fn display_lookup_never_accepts_a_terminal_or_foreign_display() {
        let mut source = source();
        assert_eq!(
            connection_scope(&source, Some("browser-a")).unwrap(),
            ("area-a".into(), Some("browser-a".into()))
        );
        assert!(connection_scope(&source, Some("other-checkout-browser")).is_err());
        source.views.as_mut().unwrap()[0].kind = "terminal";
        assert!(connection_scope(&source, Some("browser-a")).is_err());
        assert_eq!(
            connection_scope(&source, None).unwrap(),
            ("area-a".into(), None)
        );
    }

    #[test]
    fn action_refuses_wrong_checkout_area_and_non_browser_display() {
        let source = source();
        let mut action = BrowserAction {
            owner_pid: std::process::id() as i32,
            device_id: "local".into(),
            checkout_path: "/checkout".into(),
            area_id: "area-a".into(),
            action: "close".into(),
            url: None,
            display_id: Some("browser-a".into()),
            request_id: "100-close".into(),
        };
        assert!(action.command(&source).is_ok());
        assert!(
            matches!(action.command(&source).unwrap(), herdr_core::workspace_control::Action::Close { expected_browser_area: Some(area), .. } if area == "area-a")
        );
        assert!(action.validate_scope(&source).is_ok());
        action.checkout_path = "/foreign".into();
        assert_eq!(action.command(&source).unwrap_err().0, "workspace_changed");
        action.checkout_path = "/checkout".into();
        action.area_id = "other-area".into();
        assert_eq!(
            action.validate_scope(&source).unwrap_err().0,
            "view_area_missing"
        );
        action.area_id = "area-a".into();
        action.display_id = Some("shell-renderer".into());
        assert_eq!(
            action.validate_scope(&source).unwrap_err().0,
            "browser_display_missing"
        );
    }

    #[test]
    fn requests_have_one_shared_admission_cap() {
        let registry = BrowserControl::default();
        let slots: Vec<_> = (0..MAX_REQUESTS)
            .map(|_| registry.acquire().unwrap())
            .collect();
        assert_eq!(registry.acquire().unwrap_err().0, "browser_control_busy");
        drop(slots);
        assert!(registry.acquire().is_ok());
    }

    #[test]
    fn relays_have_their_own_cap_and_free_their_slot_when_they_end() {
        let registry = BrowserControl::default();
        let relays: Vec<_> = (0..MAX_RELAYS)
            .map(|_| registry.acquire_relay().unwrap())
            .collect();
        assert_eq!(
            registry.acquire_relay().unwrap_err().0,
            "browser_relay_limit"
        );
        // Discovery and actions keep their own admission while relays are full.
        assert!(registry.acquire().is_ok());
        drop(relays);
        assert!(registry.acquire_relay().is_ok());
    }

    /// A scripted gateway that answers `/connect` with a capability for
    /// itself and keeps each request body it was asked with.
    fn scripted_gateway() -> (String, Arc<Mutex<Vec<Value>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&bodies);
        let base = endpoint.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut raw = Vec::new();
                let mut chunk = [0_u8; 1024];
                let body = loop {
                    let read = stream.read(&mut chunk).unwrap_or(0);
                    raw.extend_from_slice(&chunk[..read]);
                    let text = String::from_utf8_lossy(&raw).into_owned();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|n| n.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if body.len() >= length || read == 0 {
                            break body.to_owned();
                        }
                    }
                };
                seen.lock()
                    .unwrap()
                    .push(serde_json::from_str(&body).unwrap());
                let path = "a".repeat(40);
                let answer = json!({
                    "cdp_http_url": format!("{base}/cdp/{path}"),
                    "browser_ws_url": format!("{}/cdp/{path}/devtools/browser", base.replace("http:", "ws:")),
                })
                .to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                );
            }
        });
        (endpoint, bodies)
    }

    #[test]
    fn a_caller_on_another_machine_is_not_handed_the_cores_own_window_urls() {
        let (endpoint, bodies) = scripted_gateway();
        let registry = BrowserControl::default();
        registry
            .register(Registration {
                owner_pid: std::process::id() as i32,
                endpoint,
                token: "t".repeat(40),
            })
            .unwrap();
        let source = source();
        // A pane on a device reached the core over that device's link: the
        // core's window is on another machine, so discovery answers
        // `elsewhere` without asking the gateway, and the relay still works.
        assert_eq!(
            registry
                .connect(&source, Some("browser-a"), Some("device"))
                .unwrap_err()
                .0,
            "browser_control_elsewhere"
        );
        assert!(bodies.lock().unwrap().is_empty());
        assert!(matches!(
            registry.relay(&source, "browser-a", Some("device")),
            Ok(Relay::Core(_))
        ));
        // A pane on the core's own machine still gets the URLs.
        assert!(registry.connect(&source, Some("browser-a"), None).is_ok());
    }

    #[test]
    fn only_the_comment_capability_names_a_purpose() {
        let (endpoint, bodies) = scripted_gateway();
        let registry = BrowserControl::default();
        registry
            .register(Registration {
                owner_pid: std::process::id() as i32,
                endpoint,
                token: "t".repeat(40),
            })
            .unwrap();
        let source = source();
        // A pane's discovery and its page command's relay.
        registry.connect(&source, Some("browser-a"), None).unwrap();
        registry.relay(&source, "browser-a", None).unwrap();
        // The operator's Comment mode, on a display a pane could never drive.
        let scope = scope_of(&source, Some("browser-a")).unwrap().2;
        registry.comment_capability(&scope).unwrap();

        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 3);
        for pane in &bodies[..2] {
            assert!(pane.get("purpose").is_none(), "{pane}");
        }
        assert_eq!(bodies[2]["purpose"], "comment");
        assert_eq!(bodies[2]["display_id"], "browser-a");
    }

    #[test]
    fn native_file_pages_are_explicitly_unsupported_for_cdp() {
        let mut source = source();
        source.views.as_mut().unwrap()[0].target = "file:///checkout/index.html".into();
        assert_eq!(
            connection_scope(&source, Some("browser-a")).unwrap_err().0,
            "browser_address_unsupported"
        );
        // The area capability is still useful for opening HTTP pages beside it.
        assert!(connection_scope(&source, None).is_ok());
        // An outside file's tab, whose target the core leaves empty, is refused the same way.
        let view = &mut source.views.as_mut().unwrap()[0];
        view.kind = "external";
        view.target = String::new();
        assert_eq!(
            connection_scope(&source, Some("browser-a")).unwrap_err().0,
            "browser_address_unsupported"
        );
        for address in [
            "file:///checkout/index.html",
            "/checkout/index.html",
            "data:text/html,private",
            "javascript:alert(1)",
        ] {
            assert_eq!(
                supported_address(address).unwrap_err().0,
                "browser_address_unsupported"
            );
        }
        for address in [
            "http://127.0.0.1:3000/",
            "https://example.test/page",
            "about:blank",
        ] {
            assert!(supported_address(address).is_ok(), "{address}");
        }
    }

    #[test]
    fn registration_cannot_turn_the_daemon_into_an_external_proxy() {
        assert!(endpoint_valid("http://127.0.0.1:9322"));
        for url in [
            "http://localhost:9322",
            "http://example.com:9322",
            "https://127.0.0.1:9322",
            "http://127.0.0.1:0",
            "http://user@127.0.0.1:9322",
            "http://127.0.0.1:9322/secret",
            "http://127.0.0.1:9322/?to=external",
        ] {
            assert!(!endpoint_valid(url), "{url}");
        }
    }

    /// A node's relay URL is its daemon's loopback and one ticket, nothing
    /// a caller could be sent elsewhere with.
    #[test]
    fn a_node_relay_url_is_only_a_loopback_ticket() {
        let ticket = "a".repeat(43);
        assert!(relay_url_valid(&format!(
            "ws://127.0.0.1:4242/browser-relay/{ticket}"
        )));
        for url in [
            format!("ws://localhost:4242/browser-relay/{ticket}"),
            format!("ws://10.0.0.2:4242/browser-relay/{ticket}"),
            format!("wss://127.0.0.1:4242/browser-relay/{ticket}"),
            format!("ws://127.0.0.1:4242/cdp/{ticket}"),
            format!("ws://127.0.0.1:4242/browser-relay/{ticket}?to=x"),
            format!("ws://user@127.0.0.1:4242/browser-relay/{ticket}"),
            "ws://127.0.0.1:4242/browser-relay/short".to_owned(),
            format!("ws://127.0.0.1:4242/browser-relay/{}", "a/".repeat(20)),
        ] {
            assert!(!relay_url_valid(&url), "{url}");
        }
        assert_eq!(
            loopback_origin("http://127.0.0.1:9322/cdp/x"),
            Some("http://127.0.0.1:9322".to_owned())
        );
        assert_eq!(loopback_origin("http://example.test:9322/cdp/x"), None);
    }

    #[test]
    fn returned_capability_stays_on_the_registered_port() {
        let base = "http://127.0.0.1:9322";
        assert!(capability_url_valid(
            "http://127.0.0.1:9322/c/012345678901234567890123456789012345",
            "http",
            base
        ));
        assert!(!capability_url_valid(
            "http://127.0.0.1:9323/c/012345678901234567890123456789012345",
            "http",
            base
        ));
        assert!(!capability_url_valid(
            "http://external:9322/c/012345678901234567890123456789012345",
            "http",
            base
        ));
    }

    #[test]
    fn registration_is_idempotent_and_stale_release_does_not_remove_it() {
        let registry = BrowserControl::default();
        let pid = std::process::id() as i32;
        let make = |token: &str| Registration {
            owner_pid: pid,
            endpoint: "http://127.0.0.1:9322".into(),
            token: token.into(),
        };
        let token = "0123456789abcdef0123456789abcdef";
        registry.register(make(token)).unwrap();
        registry.register(make(token)).unwrap();
        let mut with_slash = make(token);
        with_slash.endpoint.push('/');
        registry.register(with_slash).unwrap();
        registry
            .release(&make("ffffffffffffffffffffffffffffffff"))
            .unwrap();
        assert!(registry.caller(pid, "/checkout").is_ok());
        registry.release(&make(token)).unwrap();
        assert!(registry.caller(pid, "/checkout").is_err());
    }
}
