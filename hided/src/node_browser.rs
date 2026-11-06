//! Browser control on a screen machine whose core runs elsewhere (PRD
//! core-host-node-remote-core D-06, B4, B13, B15). The desktop window on
//! this machine registers its CDP gateway with this daemon, as it would
//! with a core's, and the registration never leaves the machine: the core
//! learns only that a window is here (its owner pid), over the link.
//!
//! Authority stays with the core. It checks a caller's credential, decides
//! the Workspace, area and display, and asks this daemon over the link for
//! the gateway's capability in that scope (`browser_gateway`). For a caller
//! on this machine the answer is the gateway's own loopback URLs, or a
//! one-shot relay ticket on this daemon, whose relay runs the caller's CDP
//! to the gateway here: none of it crosses the link. For a caller on
//! another machine the core reaches the same relay through a link stream.
//! The page actions the gateway asks for (open, close, select) go to the
//! core, which runs them as it runs its own window's.
//!
//! Everything handed out follows the link it was handed out over: when the
//! link ends, its tickets are dropped, its relays end as a lost gateway, and
//! the gateway is asked to revoke every capability it issued.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, watch};

use crate::backoff::Backoff;
use crate::browser_control::{BrowserControl, Failure, Registration};
use crate::node_daemon::NodeState;
use crate::node_role::LiveLink;
use crate::server::RELAY_GRANT_HEADER;

/// How long a ticket waits to be dialed.
const TICKET_LIFE: Duration = Duration::from_secs(10);
/// Relays this daemon runs at once, as many as a core runs, counting the
/// tickets handed out and not yet dialed: the refusal comes with the
/// question, not with the dial.
const MAX_RELAYS: usize = 4;
/// How often the windows are looked at again: one that exited without
/// releasing its registration leaves, and the core is told.
const OWNERS_EVERY: Duration = Duration::from_secs(5);
/// The largest answer the core sends a page action.
const MAX_ACTION_ANSWER: u64 = 64 * 1024;
const CORE_TIMEOUT: Duration = Duration::from_secs(12);
/// No link is live, or the last one's capabilities are still being revoked.
const NO_LINK: u64 = u64::MAX;

struct Ticket {
    browser_ws_url: String,
    display_id: String,
    generation: u64,
    issued: Instant,
    /// The relay it may run, held from issue to the relay's end.
    slot: OwnedSemaphorePermit,
}

/// This machine's desktop windows and the relays handed out for them.
pub struct NodeBrowser {
    control: BrowserControl,
    tickets: Mutex<HashMap<String, Ticket>>,
    relays: Arc<Semaphore>,
    /// The browser relay's own loopback port.
    port: u16,
    /// The link whose questions are answered now ([`NO_LINK`] while there
    /// is none, and while the last one's capabilities are revoked).
    generation: AtomicU64,
    /// A window registered or left: the core is told again.
    changed: Notify,
}

impl NodeBrowser {
    pub fn new(port: u16) -> Arc<Self> {
        Arc::new(Self {
            control: BrowserControl::default(),
            tickets: Mutex::new(HashMap::new()),
            relays: Arc::new(Semaphore::new(MAX_RELAYS)),
            port,
            generation: AtomicU64::new(NO_LINK),
            changed: Notify::new(),
        })
    }

    fn answering(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    /// The tickets waiting, past their life dropped, and the slot of one
    /// more relay, if one fits beside them and the running ones.
    fn room(&self, tickets: &mut HashMap<String, Ticket>) -> Option<OwnedSemaphorePermit> {
        tickets.retain(|_, ticket| ticket.issued.elapsed() < TICKET_LIFE);
        let slot = Arc::clone(&self.relays).try_acquire_owned().ok();
        if slot.is_none() {
            hide_core::diagnostic!(json!({
                "component": "node_browser",
                "kind": "relay.cap_reached",
                "cap": MAX_RELAYS,
            }));
        }
        slot
    }

    /// The core's question over the link of `generation`. Everything the
    /// question can be refused for is checked before the gateway is asked,
    /// and a capability is issued only on the link that is answered now.
    fn capability(&self, scope: &Value, relay: bool, generation: u64) -> Result<Value, String> {
        let failed = |(reason, _): Failure| reason.to_owned();
        let display_id = scope_valid(scope).ok_or("invalid_browser_scope")?;
        let unanswered = || "browser_control_unavailable".to_owned();
        if !self.answering(generation) {
            return Err(unanswered());
        }
        if relay {
            if display_id.is_none() {
                return Err("browser_display_missing".to_owned());
            }
            if self.room(&mut lock(&self.tickets)).is_none() {
                return Err("browser_relay_limit".to_owned());
            }
        }
        let (http, ws) = self.control.gateway_capability(scope).map_err(failed)?;
        // A link that ended while the gateway was asked may already have
        // had everything revoked, which this capability missed. One that
        // ends after this look is revoked after the capability was issued.
        if !self.answering(generation) {
            self.control.revoke_all();
            return Err(unanswered());
        }
        let Some(display_id) = display_id.filter(|_| relay) else {
            return Ok(json!({"cdp_http_url": http, "browser_ws_url": ws}));
        };
        let mut tickets = lock(&self.tickets);
        if !self.answering(generation) {
            return Err(unanswered());
        }
        // Another question took the last room meanwhile: this capability
        // is never handed out and goes with the link's revocation.
        let Some(slot) = self.room(&mut tickets) else {
            return Err("browser_relay_limit".to_owned());
        };
        let ticket = crate::state_file::new_token();
        tickets.insert(
            ticket.clone(),
            Ticket {
                browser_ws_url: ws,
                display_id,
                generation,
                issued: Instant::now(),
                slot,
            },
        );
        Ok(json!({
            "relay_url": format!("ws://127.0.0.1:{}/browser-relay/{ticket}", self.port)
        }))
    }

    /// Takes `ticket` for a dial on the link of `generation`.
    fn take(&self, ticket: &str, generation: Option<u64>) -> Option<Ticket> {
        let ticket = lock(&self.tickets).remove(ticket)?;
        (Some(ticket.generation) == generation && ticket.issued.elapsed() < TICKET_LIFE)
            .then_some(ticket)
    }

    /// Follows the link: tells each core that takes it which windows are
    /// here, again whenever that changes (a window registering, leaving, or
    /// exiting without leaving), and revokes everything handed out over a
    /// link once it is gone, before the next link is answered. An announce
    /// the core refuses is sent again after 2 s, doubling to a minute, and
    /// logged when its reason changes; a link that changes meanwhile is
    /// followed at once. A daemon that stops aborts this task: the desktop
    /// host revokes everything itself when its daemon goes.
    pub fn spawn_follow(
        self: &Arc<Self>,
        mut live: watch::Receiver<Option<Arc<LiveLink>>>,
    ) -> tokio::task::JoinHandle<()> {
        let browser = Arc::clone(self);
        tokio::spawn(async move {
            let mut current: Option<u64> = None;
            // The windows the current link's core was last told of.
            let mut told: Option<Vec<i32>> = None;
            // An announce that failed is sent again after a wait.
            let mut backoff = Backoff::default();
            let mut retry_at: Option<tokio::time::Instant> = None;
            let mut failure: Option<String> = None;
            loop {
                let link = live.borrow_and_update().clone();
                let generation = link.as_ref().map(|link| link.generation);
                if current != generation {
                    browser.generation.store(NO_LINK, Ordering::SeqCst);
                    if current.is_some() {
                        browser.ended().await;
                    }
                    current = generation;
                    told = None;
                    backoff.reset();
                    retry_at = None;
                    failure = None;
                    browser
                        .generation
                        .store(generation.unwrap_or(NO_LINK), Ordering::SeqCst);
                    continue;
                }
                let owners = browser.control.owners();
                let due = retry_at.is_none_or(|at| tokio::time::Instant::now() >= at);
                if let Some(link) = link.filter(|_| due && told.as_ref() != Some(&owners)) {
                    let sending = owners.clone();
                    let sent = tokio::select! {
                        sent = tokio::task::spawn_blocking(move || announce(&link, &sending)) => {
                            sent.unwrap_or_else(|_| Err("announce_failed".to_owned()))
                        }
                        changed = live.changed() => {
                            if changed.is_err() {
                                browser.ended().await;
                                return;
                            }
                            continue;
                        }
                    };
                    match sent {
                        Ok(()) => {
                            told = Some(owners);
                            backoff.reset();
                            retry_at = None;
                            failure = None;
                        }
                        Err(reason) => {
                            if failure.as_ref() != Some(&reason) {
                                hide_core::diagnostic!(json!({
                                    "component": "node_browser",
                                    "kind": "announce.failed",
                                    "generation": generation,
                                    "reason": reason,
                                }));
                                failure = Some(reason);
                            }
                            retry_at = Some(tokio::time::Instant::now() + backoff.failed());
                        }
                    }
                }
                let look = tokio::time::Instant::now() + OWNERS_EVERY;
                let next = retry_at.map_or(look, |at| at.min(look));
                tokio::select! {
                    changed = live.changed() => {
                        if changed.is_err() {
                            browser.ended().await;
                            return;
                        }
                    }
                    () = browser.changed.notified() => retry_at = None,
                    () = tokio::time::sleep_until(next) => {}
                }
            }
        })
    }

    /// The link is gone: its tickets go, and the gateway drops every
    /// capability; each relay ends on its own as a lost gateway.
    async fn ended(self: &Arc<Self>) {
        lock(&self.tickets).clear();
        let browser = Arc::clone(self);
        let _ = tokio::task::spawn_blocking(move || browser.control.revoke_all()).await;
    }
}

/// The browser gateway as the link of one generation serves it.
pub struct LinkBrowser {
    pub browser: Arc<NodeBrowser>,
    pub generation: u64,
}

impl hide_host::link_bridge::BrowserGateway for LinkBrowser {
    fn capability(&self, scope: &Value, relay: bool) -> Result<Value, String> {
        self.browser.capability(scope, relay, self.generation)
    }

    fn relay_address(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.browser.port))
    }
}

/// The scope the core decided, in the shape the gateway takes and nothing
/// else; its display, if it names one.
fn scope_valid(scope: &Value) -> Option<Option<String>> {
    let fields = scope.as_object()?;
    let text = |key: &str, cap: usize| {
        fields
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= cap)
    };
    text("workspace", 8 * 1024 + 256)?;
    text("area_id", 256)?;
    let display = match fields.get("display_id") {
        None => None,
        Some(_) => Some(text("display_id", 256)?.to_owned()),
    };
    fields
        .keys()
        .all(|key| matches!(key.as_str(), "workspace" | "area_id" | "display_id"))
        .then_some(display)
}

fn core_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(CORE_TIMEOUT))
        .max_redirects(0)
        .proxy(None)
        .http_status_as_error(false)
        .build()
        .into()
}

/// Tells the core over `link` which windows are here now.
fn announce(link: &LiveLink, owners: &[i32]) -> Result<(), String> {
    let response = core_agent()
        .post(format!(
            "http://127.0.0.1:{}/relay/browser-control",
            link.relay_port
        ))
        .header(RELAY_GRANT_HEADER, &link.accepted.relay_token)
        .header("Content-Type", "application/json")
        .send(json!({ "owners": owners }).to_string().as_bytes())
        .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("the core answered {}", response.status()))
    }
}

fn failure(status: StatusCode, (reason, next_action): Failure) -> Response {
    hide_core::diagnostic!(json!({
        "component": "node_browser",
        "kind": "request.refused",
        "reason": reason,
    }));
    (
        status,
        axum::Json(json!({"ok": false, "reason": reason, "next_action": next_action})),
    )
        .into_response()
}

/// The desktop window on this machine registers its gateway here.
pub async fn register(
    State(state): State<NodeState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<Registration>,
) -> Response {
    if !crate::server::bearer_matches(&headers, &state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.browser.control.register(request) {
        Ok(()) => {
            state.browser.changed.notify_one();
            StatusCode::NO_CONTENT.into_response()
        }
        Err(refused) => failure(StatusCode::CONFLICT, refused),
    }
}

pub async fn release(
    State(state): State<NodeState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<Registration>,
) -> Response {
    if !crate::server::bearer_matches(&headers, &state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.browser.control.release(&request) {
        Ok(()) => {
            state.browser.changed.notify_one();
            StatusCode::NO_CONTENT.into_response()
        }
        Err(refused) => failure(StatusCode::CONFLICT, refused),
    }
}

/// A page action the window's gateway asks for: the core runs it, as the
/// window registered here.
pub async fn action(
    State(state): State<NodeState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<crate::browser_control::BrowserAction>,
) -> Response {
    if !crate::server::bearer_matches(&headers, &state.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !request.valid() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if let Err(refused) = state.browser.control.registered(request.owner_pid) {
        return failure(StatusCode::CONFLICT, refused);
    }
    let Some(link) = state.live.borrow().clone() else {
        return failure(
            StatusCode::SERVICE_UNAVAILABLE,
            (
                "browser_control_unavailable",
                "Wait for this machine to reach its core and retry",
            ),
        );
    };
    let forwarded = tokio::task::spawn_blocking(move || {
        let body = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        let mut response = core_agent()
            .post(format!(
                "http://127.0.0.1:{}/relay/browser-control/action",
                link.relay_port
            ))
            .header(RELAY_GRANT_HEADER, &link.accepted.relay_token)
            .header("Content-Type", "application/json")
            .send(&body[..])
            .map_err(|error| error.to_string())?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_ACTION_ANSWER)
            .read_to_vec()
            .map_err(|error| error.to_string())?;
        Ok::<_, String>((status, body))
    })
    .await;
    match forwarded {
        Ok(Ok((status, body))) => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        // The core may have run it: the gateway reads this as uncertain.
        _ => StatusCode::BAD_GATEWAY.into_response(),
    }
}

/// A caller dials the relay a ticket names: the core handed it out over
/// the live link, for this caller only, moments ago. The gateway side opens
/// first, so its refusal (its own client cap) reaches the caller as a
/// status rather than a dropped socket.
pub async fn relay(
    upgrade: WebSocketUpgrade,
    Path(ticket): Path<String>,
    headers: HeaderMap,
    State(state): State<NodeState>,
) -> Response {
    // A process on this machine, never a page or `tailscale serve`.
    if headers.contains_key(axum::http::header::ORIGIN) || crate::server::via_tailnet(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let generation = state.live.borrow().as_ref().map(|link| link.generation);
    let Some(ticket) = state.browser.take(&ticket, generation) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let gateway = match crate::browser_relay::connect(&ticket.browser_ws_url).await {
        Ok(gateway) => gateway,
        Err((reason, _)) => {
            return if reason == "browser_control_busy" {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::BAD_GATEWAY
            }
            .into_response();
        }
    };
    let live = state.live.clone();
    let generation = ticket.generation;
    upgrade.on_upgrade(move |mut socket| async move {
        // The relay's slot, taken when its ticket was issued, goes as it ends.
        let _slot = ticket.slot;
        crate::browser_relay::pump(
            &mut socket,
            gateway,
            &ticket.display_id,
            crate::browser_relay::Way::Node,
            link_gone(live, generation),
        )
        .await;
    })
}

/// Resolves once the link of `generation` is gone.
async fn link_gone(mut live: watch::Receiver<Option<Arc<LiveLink>>>, generation: u64) {
    loop {
        if live
            .borrow_and_update()
            .as_ref()
            .map(|link| link.generation)
            != Some(generation)
        {
            return;
        }
        if live.changed().await.is_err() {
            return;
        }
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

    #[test]
    fn a_scope_is_taken_only_in_the_gateway_s_shape() {
        let scope = json!({"workspace": "local\u{0}/c", "area_id": "a", "display_id": "d"});
        assert_eq!(scope_valid(&scope), Some(Some("d".to_owned())));
        assert_eq!(
            scope_valid(&json!({"workspace": "local\u{0}/c", "area_id": "a"})),
            Some(None)
        );
        for refused in [
            json!({"workspace": "local\u{0}/c"}),
            json!({"workspace": "local\u{0}/c", "area_id": ""}),
            json!({"workspace": "local\u{0}/c", "area_id": "a", "display_id": 1}),
            json!({"workspace": "local\u{0}/c", "area_id": "a", "endpoint": "x"}),
            // A linked node never grants the operator's Comment lease.
            json!({"workspace": "local\u{0}/c", "area_id": "a", "display_id": "d", "purpose": "comment"}),
            json!("local"),
        ] {
            assert_eq!(scope_valid(&refused), None, "{refused}");
        }
    }

    #[test]
    fn a_ticket_is_taken_once_and_only_on_the_link_it_was_handed_out_over() {
        let browser = NodeBrowser::new(4242);
        let ticket = |browser: &NodeBrowser, generation| {
            lock(&browser.tickets).insert(
                format!("t{generation}"),
                Ticket {
                    browser_ws_url: "ws://127.0.0.1:1/cdp".to_owned(),
                    display_id: "d".to_owned(),
                    generation,
                    issued: Instant::now(),
                    slot: Arc::clone(&browser.relays).try_acquire_owned().unwrap(),
                },
            );
            format!("t{generation}")
        };
        let first = ticket(&browser, 1);
        assert!(browser.take(&first, Some(1)).is_some());
        assert!(browser.take(&first, Some(1)).is_none(), "one dial only");
        let second = ticket(&browser, 1);
        assert!(browser.take(&second, Some(2)).is_none(), "another link");
        assert!(
            browser.take(&second, Some(1)).is_none(),
            "gone after a refusal"
        );
        let third = ticket(&browser, 1);
        assert!(browser.take(&third, None).is_none(), "no link");
    }

    /// A question on a link that is not the one answered now (it ended, or
    /// the last one's capabilities are still being revoked) issues nothing;
    /// relays waiting to be dialed count against the relays that may run.
    #[test]
    fn nothing_is_issued_off_the_answered_link_or_past_the_relay_cap() {
        let browser = NodeBrowser::new(4242);
        let scope = json!({"workspace": "local\u{0}/c", "area_id": "a", "display_id": "d"});
        assert_eq!(
            browser.capability(&scope, true, 1).unwrap_err(),
            "browser_control_unavailable"
        );
        browser.generation.store(1, Ordering::SeqCst);
        assert_eq!(
            browser.capability(&scope, true, 2).unwrap_err(),
            "browser_control_unavailable"
        );
        let mut tickets = lock(&browser.tickets);
        for n in 0..MAX_RELAYS {
            tickets.insert(
                format!("t{n}"),
                Ticket {
                    browser_ws_url: String::new(),
                    display_id: "d".to_owned(),
                    generation: 1,
                    issued: Instant::now(),
                    slot: Arc::clone(&browser.relays).try_acquire_owned().unwrap(),
                },
            );
        }
        drop(tickets);
        assert_eq!(
            browser.capability(&scope, true, 1).unwrap_err(),
            "browser_relay_limit"
        );
        browser.take("t0", Some(1)).unwrap();
        // Room again; the gateway is asked next, and there is none.
        assert_eq!(
            browser.capability(&scope, true, 1).unwrap_err(),
            "browser_control_unavailable"
        );
    }

    #[test]
    fn a_relay_without_a_registered_window_is_refused_with_its_reason() {
        let browser = NodeBrowser::new(4242);
        browser.generation.store(1, Ordering::SeqCst);
        let scope = json!({"workspace": "local\u{0}/c", "area_id": "a", "display_id": "d"});
        assert_eq!(
            browser.capability(&scope, true, 1).unwrap_err(),
            "browser_control_unavailable"
        );
        assert_eq!(
            browser
                .capability(&json!({"workspace": "x"}), true, 1)
                .unwrap_err(),
            "invalid_browser_scope"
        );
    }
}
