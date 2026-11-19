//! Pairing this machine with other Hides (PRD device-connect-iroh B1-B7,
//! B24-B27, B50-B52; D-26, D-72).
//!
//! `hide_peer::pairing` owns how two machines pair. This module is where
//! this daemon's operator reaches it, and where what it settles goes:
//!
//! - an [`Ask`] is one thing the operator does: a window of this machine
//!   sends it as a `pairing_*` event, a terminal as a `hide device` verb
//!   over the bootstrap socket. Sharing this machine, pasting a link,
//!   allowing a request, turning a direction back on, naming an address and
//!   asking to be let in are the operator's alone: an agent in a pane, an
//!   external tool and a window of another machine are refused (B52).
//! - the pairings under way (the link, the words, who waits) go to this
//!   machine's own windows as a `pairing` frame, and nowhere else: neither
//!   into the core's snapshot, which a paired machine may read, nor into a
//!   log.
//! - who opens this machine, whom it opens and whether the two are
//!   connected go to the core as one list in one event (D-72).
//!
//! The endpoint is `peer_host`'s. Asking for a status opens nothing; sharing
//! and adding do (D-83).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use hide_core::peer_machines::{
    PeerMachine, PeerOpenedBy, PeerOpens, PeerPath, PeerReason, PeerUpdate,
};
use hide_peer::pairing::{AcceptError, JoinError, Me, Pairings, RequestError, View};
use hide_peer::protocol::hello::{Behind, GrantState};
use hide_peer::protocol::{Candidate, PathKind, PeerKey, StreamKind};
use hide_peer::store::{GrantStatus, KnownState, StoreError, Via};
use hide_peer::words::WordList;
use hide_peer::{PeerEvent, PeerTransport, RelayState};
use hide_peer_iroh::IrohTransport;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{OnceCell, Semaphore, watch};
use tokio::time::Instant;

use crate::core::CoreHandle;
use crate::ending::Ending;
use crate::pairing_ssh;
use crate::peer_host::{HostPeers, Incoming, PeerHost};

type Service = Pairings<IrohTransport>;

/// How long a share a terminal holds lives after the terminal last asked
/// for its status: a closed terminal ends its offer (security spec 3.4).
const HELD_FOR: Duration = Duration::from_secs(10);

/// How long a share waits for the relay to name this machine before its
/// link goes out with the addresses it has.
const RELAY_WITHIN: Duration = Duration::from_secs(3);

/// How long the relay's answer to "why are you not reachable" is reused.
const RELAY_ANSWER_FOR: Duration = Duration::from_secs(30);

/// The most addresses kept for one host.
const MAX_ADDRESSES: usize = 8;

/// Which direction of a pair an ask is about.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// This machine opens the other.
    Outgoing,
    /// The other opens this machine.
    Incoming,
    Both,
}

/// One thing the operator does about pairing.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case", deny_unknown_fields)]
pub enum Ask {
    /// What a terminal reads while it waits: the pairings under way and
    /// the paired machines. `hold` keeps the share that terminal started.
    Status {
        #[serde(default)]
        hold: bool,
    },
    Share {
        /// The language the confirmation words are read in.
        #[serde(default)]
        language: Option<String>,
        /// The asker keeps asking for the status; the offer ends when it
        /// stops.
        #[serde(default)]
        held: bool,
    },
    EndShare,
    Join {
        link: String,
    },
    Compare {
        same: bool,
    },
    DismissJoin,
    Answer {
        ask: u64,
        allow: bool,
    },
    Pause {
        device: String,
        #[serde(default)]
        direction: Option<Direction>,
    },
    Resume {
        device: String,
        #[serde(default)]
        direction: Option<Direction>,
    },
    Remove {
        device: String,
        #[serde(default)]
        direction: Option<Direction>,
    },
    Request {
        device: String,
    },
    DismissRequest {
        device: String,
    },
    /// Dials a machine that is not connected again now.
    Retry {
        device: String,
    },
    /// Names an address to reach a host at first.
    Address {
        device: String,
        address: String,
    },
    /// Asks a machine this one reaches over SSH whether it can pair that
    /// way; nothing changes there (B7).
    LookSsh {
        alias: String,
    },
    /// Pairs with a machine reached over SSH whose own Hide runs: that Hide
    /// saves a pending grant and this machine confirms it over the
    /// connection (security spec 15). Nothing is installed there.
    PairSsh {
        alias: String,
    },
    /// The target's side of a pairing over SSH or `wsl.exe` (security spec
    /// 15, 17).
    AcceptSsh {
        request: String,
        key: String,
        device: String,
        name: String,
        #[serde(default)]
        wsl: bool,
    },
}

impl Ask {
    /// Whether only the operator may ask it, at a window or a terminal of
    /// this machine (B52).
    fn operators(&self) -> bool {
        match self {
            Self::Share { .. }
            | Self::EndShare
            | Self::Join { .. }
            | Self::Compare { .. }
            | Self::Answer { .. }
            | Self::Resume { .. }
            | Self::Request { .. }
            | Self::Address { .. }
            | Self::LookSsh { .. }
            | Self::PairSsh { .. }
            | Self::AcceptSsh { .. } => true,
            Self::Status { .. }
            | Self::DismissJoin
            | Self::Pause { .. }
            | Self::Remove { .. }
            | Self::DismissRequest { .. }
            | Self::Retry { .. } => false,
        }
    }
}

/// Who asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    /// A window of this machine, or a terminal of it outside Hide's panes.
    Operator,
    /// A program in a Hide pane: it may look, stop and remove, and nothing
    /// that opens this machine or reaches another (B52).
    Pane,
    /// A screen that is not this machine's own window (another machine's
    /// view of it, a phone): it is refused every ask, and only it hears so
    /// (B52).
    OtherScreen,
}

/// What a caller does about a refusal `code`.
pub fn next_action(code: &str) -> &'static str {
    match code {
        "device_from_pane" => {
            "Ask the operator to run it in a terminal outside Hide's panes, or to use Settings > Devices"
        }
        "device_from_other_screen" => "Do it on that machine's own Hide window",
        "link_invalid" => {
            "Copy the whole link from the other machine's Settings > Devices and try again"
        }
        "link_newer" => "Update Hide on this machine, then add the link again",
        "link_own" => "This link is this machine's own; paste it on the other machine",
        "device_already_connected" => "Run hide device list: the machine is already connected",
        "device_unknown" => "Run hide device list for the connected machines",
        "device_ambiguous" => "Name the machine by its id from hide device list",
        "device_direction_needed" => {
            "Both machines open each other: add --outgoing, --incoming or --both"
        }
        "device_not_a_guest" => {
            "That machine does not open this one, so there is no connection to ask over: share a link from it instead"
        }
        "request_waiting" => "The earlier request still waits for its answer on the other machine",
        "ask_gone" => "The request is over; nothing was changed",
        "address_invalid" => "Give the address as ip:port",
        "peer_unavailable" => {
            "This machine could not open its connection; the reason is in Hide's diagnostic log"
        }
        "device_limit" => {
            "Too many machines are connected: remove one that is not used, then try again"
        }
        "device_not_saved" => "Hide could not save the change; the reason is in its diagnostic log",
        "pairing_ended" => {
            "The link ended and nothing was connected: share a new link and start again"
        }
        "pairing_no_offer" => {
            "The link is used, expired or replaced: share a new link on the other machine and add that one"
        }
        "pairing_declined" => "The other machine's operator said no; nothing was connected",
        "pairing_host_full" => {
            "The other machine has too many machines connected: remove one there, then share a new link"
        }
        "pairing_full" => {
            "This machine is connected to too many machines: remove one, then add the link again"
        }
        "pairing_unanswered" => {
            "Nobody answered in two minutes: share a new link and answer on both machines"
        }
        "pairing_unreachable" => {
            "The other machine could not be reached, or the connection ended midway: share a new link and try again"
        }
        "pairing_update_needed" => {
            "The two Hides are too far apart to connect: update the older one, then share a new link"
        }
        "ssh_hide_absent" => {
            "That machine has no Hide of its own: install Hide there and add it by a link it shares"
        }
        "ssh_update_there" => "Update Hide on that machine, then add it again",
        "ssh_hide_not_running" => {
            "Hide is not running on that machine: log in there and open Hide, then add it again"
        }
        "ssh_not_reached" => {
            "ssh did not sign in without asking: check ssh <alias> in a terminal, or add the machine by a link it shares"
        }
        "ssh_busy" => "Other SSH checks are still running; try again in a moment",
        "device_own" => "That is this machine itself",
        _ => "Retry; the reason is in Hide's diagnostic log",
    }
}

pub(crate) fn refused(code: &str) -> Value {
    json!({ "ok": false, "reason": code, "next_action": next_action(code) })
}

fn lock<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    // Small maps of plain values: whole whatever panicked beside them.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One paired machine, with what an ask needs to act on it.
struct Paired {
    machine: PeerMachine,
    key: PeerKey,
}

struct RelayAnswer {
    state: RelayState,
    at: Instant,
}

pub struct Pairing {
    host: Arc<PeerHost>,
    core: Arc<CoreHandle>,
    me: Me,
    service: OnceCell<Service>,
    frame: watch::Sender<Arc<Value>>,
    /// Counts the asks that want the paired machines read again.
    look: watch::Sender<u64>,
    /// Which machines cannot connect until one side updates.
    updates: Mutex<HashMap<PeerKey, PeerUpdate>>,
    relay: Mutex<Option<RelayAnswer>>,
    /// When the terminal that holds the open share last asked.
    held: Mutex<Option<Instant>>,
    /// How a target's `hide` is reached over SSH.
    ssh: pairing_ssh::Reach,
    /// The SSH dials under way: each is one `ssh` this daemon owns.
    ssh_dials: Arc<Semaphore>,
    /// The request each alias's pairing runs under, so one that ssh lost
    /// midway is asked again as the same request (security spec 15).
    ssh_requests: Mutex<Vec<SshRequest>>,
    shutdown: Ending,
}

/// The most `ssh` this daemon runs at once to look at or pair with a
/// machine; an ask past it is refused, not queued.
const SSH_DIALS: usize = 4;

/// How long a pairing's request is asked again as itself: as long as the
/// target keeps its pending grant.
const SSH_REQUEST_KEPT: Duration = Duration::from_secs(10 * 60);

/// The most requests remembered; the oldest is dropped first.
const SSH_REQUESTS: usize = 16;

struct SshRequest {
    alias: String,
    id: String,
    made: Instant,
}

impl Pairing {
    /// Hands the pairing and revoke streams' handlers to `host`. Nothing
    /// opens: the service starts with the endpoint.
    pub fn new(
        host: Arc<PeerHost>,
        core: Arc<CoreHandle>,
        me: Me,
        ssh: pairing_ssh::Reach,
        shutdown: Ending,
    ) -> Arc<Self> {
        let pairing = Arc::new(Self {
            host: Arc::clone(&host),
            core,
            me,
            service: OnceCell::new(),
            frame: watch::channel(Arc::new(json!(View::default()))).0,
            look: watch::channel(0).0,
            updates: Mutex::new(HashMap::new()),
            relay: Mutex::new(None),
            held: Mutex::new(None),
            ssh,
            ssh_dials: Arc::new(Semaphore::new(SSH_DIALS)),
            ssh_requests: Mutex::new(Vec::new()),
            shutdown,
        });
        for kind in [StreamKind::Pairing, StreamKind::Revoke] {
            let pairing = Arc::clone(&pairing);
            host.handle(kind, move |incoming: Incoming| {
                let pairing = Arc::clone(&pairing);
                async move {
                    // A stream arrived, so the endpoint is open.
                    if let Ok(service) = pairing.service().await {
                        service.serve(incoming);
                    }
                }
            });
        }
        pairing
    }

    /// The `pairing` frame this machine's own windows draw.
    pub fn subscribe_frame(&self) -> watch::Receiver<Arc<Value>> {
        self.frame.subscribe()
    }

    /// Starts the service on an endpoint the daemon's start opened: every
    /// known host is dialed, and what was not settled goes on.
    pub async fn started(self: &Arc<Self>) {
        if let Err(code) = self.service().await {
            hide_core::diagnostic!(json!({
                "component": "pairing",
                "kind": "pairing.not_started",
                "reason": code,
            }));
        }
    }

    /// The service, which opens the endpoint when it is not open yet.
    async fn service(self: &Arc<Self>) -> Result<Service, &'static str> {
        self.service
            .get_or_try_init(|| async {
                let peers = self.host.open().await.map_err(|error| {
                    hide_core::diagnostic!(json!({
                        "component": "pairing",
                        "kind": "pairing.endpoint_unavailable",
                        "reason": error.to_string(),
                    }));
                    "peer_unavailable"
                })?;
                let service = Pairings::start(peers.clone(), self.me.clone()).map_err(|error| {
                    hide_core::diagnostic!(json!({
                        "component": "pairing",
                        "kind": "pairing.service_unavailable",
                        "reason": error.to_string(),
                    }));
                    "peer_unavailable"
                })?;
                tokio::spawn(Arc::clone(self).follow(service.clone(), peers));
                Ok(service)
            })
            .await
            .cloned()
    }

    /// Republishes the frame and the paired machines whenever either may
    /// have changed, until the daemon ends.
    async fn follow(self: Arc<Self>, service: Service, peers: HostPeers) {
        let mut view = service.view();
        let mut saved = service.saved();
        let mut look = self.look.subscribe();
        let mut events = peers.events();
        let mut told: Option<Vec<PeerMachine>> = None;
        loop {
            let frame = json!(*view.borrow_and_update());
            self.frame.send_if_modified(|shown| {
                if **shown == frame {
                    return false;
                }
                *shown = Arc::new(frame);
                true
            });
            saved.borrow_and_update();
            look.borrow_and_update();
            let machines: Vec<PeerMachine> = self
                .paired(&peers)
                .await
                .into_iter()
                .map(|paired| paired.machine)
                .collect();
            if told.as_ref() != Some(&machines) {
                self.paired_changed(&machines).await;
                told = Some(machines);
            }
            tokio::select! {
                () = self.shutdown.ended() => {
                    service.close();
                    return;
                }
                changed = view.changed() => if changed.is_err() { return },
                changed = saved.changed() => if changed.is_err() { return },
                changed = look.changed() => if changed.is_err() { return },
                event = events.recv() => match event {
                    Ok(PeerEvent::UpdateNeeded { key, behind, .. }) => {
                        let update = match behind {
                            Behind::Ours | Behind::Either => PeerUpdate::Here,
                            Behind::Theirs => PeerUpdate::There,
                        };
                        lock(&self.updates).insert(key, update);
                    }
                    Ok(PeerEvent::Connected { key, .. }) => {
                        lock(&self.updates).remove(&key);
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
            }
        }
    }

    /// Runs once when the pairing starts and then whenever the machines this
    /// one is paired with changed: a machine paired, confirmed, paused,
    /// resumed or removed in either direction, connected or lost. Whatever
    /// follows "is this machine paired with anything" belongs here; today
    /// that is the core's list (D-72).
    async fn paired_changed(&self, machines: &[PeerMachine]) {
        let event = json!({
            "schema_version": hide_core::SCHEMA_VERSION,
            "kind": "peer_machines",
            "payload": { "machines": machines },
        });
        let core = Arc::clone(&self.core);
        let told =
            tokio::task::spawn_blocking(move || core.dispatch(event.to_string().into_bytes()))
                .await
                .map_err(|error| error.to_string())
                .and_then(|told| told);
        if let Err(error) = told {
            hide_core::diagnostic!(json!({
                "component": "pairing",
                "kind": "pairing.core_not_told",
                "reason": error,
            }));
        }
    }

    /// What the relay says of itself, asked only to explain a host that is
    /// not connected, and at most once in [`RELAY_ANSWER_FOR`].
    async fn relay_state(&self, peers: &HostPeers) -> RelayState {
        if let Some(answer) = lock(&self.relay).as_ref()
            && answer.at.elapsed() < RELAY_ANSWER_FOR
        {
            return answer.state;
        }
        let state = peers.transport().relay().await;
        *lock(&self.relay) = Some(RelayAnswer {
            state,
            at: Instant::now(),
        });
        state
    }

    /// Every machine this one is paired with, in either direction, by name.
    async fn paired(&self, peers: &HostPeers) -> Vec<Paired> {
        let hosts = peers.hosts();
        let guests = peers.grants(|grants, now| {
            grants
                .list(now)
                .filter(|grant| grant.state != GrantStatus::Pending)
                .cloned()
                .collect::<Vec<_>>()
        });
        let updates = lock(&self.updates).clone();
        let mut paired: Vec<Paired> = Vec::new();
        let mut away = false;
        for host in &hosts {
            let link = peers.link(&host.key);
            let opens = match host.state {
                KnownState::Confirming => PeerOpens::Confirming,
                KnownState::PausedByMe => PeerOpens::Paused,
                KnownState::RevokePending => PeerOpens::Removing,
                KnownState::Unpaired => PeerOpens::Unpaired,
                KnownState::Active => match link.as_ref().map(|link| link.granted()) {
                    Some(GrantState::Paused) => PeerOpens::PausedThere,
                    _ => PeerOpens::Active,
                },
            };
            away |= link.is_none() && host.state.dialed();
            paired.push(Paired {
                key: host.key,
                machine: PeerMachine {
                    key: host.key.to_hex(),
                    device_id: host.device.clone(),
                    name: host.name.clone(),
                    opens: Some(opens),
                    opened_by: None,
                    connected: link.is_some(),
                    path: link.map(|link| path_of(link.path())),
                    reason: None,
                    update: updates.get(&host.key).copied(),
                },
            });
        }
        for guest in &guests {
            let opened_by = Some(match guest.state {
                GrantStatus::Paused => PeerOpenedBy::Paused,
                _ => PeerOpenedBy::Active,
            });
            if let Some(known) = paired.iter_mut().find(|paired| paired.key == guest.key) {
                known.machine.opened_by = opened_by;
                continue;
            }
            let link = peers.link(&guest.key);
            paired.push(Paired {
                key: guest.key,
                machine: PeerMachine {
                    key: guest.key.to_hex(),
                    device_id: guest.device.clone(),
                    name: guest.name.clone(),
                    opens: None,
                    opened_by,
                    connected: link.is_some(),
                    path: link.map(|link| path_of(link.path())),
                    reason: None,
                    update: updates.get(&guest.key).copied(),
                },
            });
        }
        // Why a host this machine dials is not connected: the relay's own
        // answer when it has one (B34), else nothing more than "not
        // connected".
        if away {
            let reason = match self.relay_state(peers).await {
                RelayState::MonthlyLimit => Some(PeerReason::RelayMonthlyLimit),
                RelayState::Unreachable => Some(PeerReason::RelayUnreachable),
                RelayState::Attached | RelayState::None => None,
            };
            for paired in &mut paired {
                if !paired.machine.connected && paired.machine.opens.is_some() {
                    paired.machine.reason = reason;
                }
            }
        }
        paired.sort_by(|a, b| {
            (a.machine.name.to_lowercase(), &a.machine.key)
                .cmp(&(b.machine.name.to_lowercase(), &b.machine.key))
        });
        paired
    }

    /// The paired machine `text` names: its device id, its name, or the
    /// start of its connection id.
    async fn named(&self, peers: &HostPeers, text: &str) -> Result<Paired, &'static str> {
        let text = text.trim();
        let mut paired = self.paired(peers).await;
        if let Some(at) = paired
            .iter()
            .position(|paired| paired.machine.device_id == text || paired.machine.key == text)
        {
            return Ok(paired.swap_remove(at));
        }
        let mut matching: Vec<usize> = paired
            .iter()
            .enumerate()
            .filter(|(_, paired)| {
                paired.machine.name.eq_ignore_ascii_case(text)
                    || (text.len() >= 8 && paired.machine.key.starts_with(text))
            })
            .map(|(at, _)| at)
            .collect();
        match (matching.pop(), matching.is_empty()) {
            (Some(at), true) => Ok(paired.swap_remove(at)),
            (Some(_), false) => Err("device_ambiguous"),
            (None, _) => Err("device_unknown"),
        }
    }

    /// Answers one ask. The answer is `{"ok": true, ...}` or a refusal with
    /// its code and what to do about it.
    pub async fn ask(self: &Arc<Self>, ask: Ask, caller: Caller) -> Value {
        match caller {
            Caller::Operator => {}
            Caller::Pane if !ask.operators() => {}
            Caller::Pane => return refused("device_from_pane"),
            Caller::OtherScreen => return refused("device_from_other_screen"),
        }
        match self.run(ask).await {
            Ok(mut answer) => {
                answer["ok"] = json!(true);
                answer
            }
            Err(code) => refused(code),
        }
    }

    async fn run(self: &Arc<Self>, ask: Ask) -> Result<Value, &'static str> {
        // Reading opens nothing: a machine paired with nothing has nothing
        // to say and no endpoint to open for it.
        if let Ask::Status { hold } = ask {
            let Some(service) = self.service.get() else {
                return Ok(json!({ "pairing": View::default(), "machines": [] }));
            };
            if hold && let Some(held) = lock(&self.held).as_mut() {
                *held = Instant::now();
            }
            let peers = self.host.peers().ok_or("peer_unavailable")?;
            // Each machine as `hide device list` answers it.
            let machines: Vec<Value> = self
                .paired(&peers)
                .await
                .iter()
                .map(|paired| hide_core::peer_machines::answer(&paired.machine))
                .collect();
            return Ok(json!({ "pairing": *service.view().borrow(), "machines": machines }));
        }
        // Looking at a machine over SSH opens nothing here and changes
        // nothing there.
        if let Ask::LookSsh { alias } = &ask {
            let (reach, target) = (self.ssh.clone(), alias.clone());
            let looked = self
                .ssh_dial(alias, move || pairing_ssh::look(&reach, &target))
                .await?;
            return Ok(json!({ "alias": alias, "looked": looked }));
        }
        // Sharing this machine, joining another and either side of an SSH
        // pairing open the endpoint (D-83). Nothing else does: with no
        // endpoint there is no machine to act on and no ask to answer.
        let service = match &ask {
            Ask::Share { .. } | Ask::Join { .. } | Ask::PairSsh { .. } | Ask::AcceptSsh { .. } => {
                self.service().await?
            }
            Ask::EndShare | Ask::DismissJoin => match self.service.get() {
                Some(service) => service.clone(),
                None => return Ok(json!({})),
            },
            Ask::Compare { .. } | Ask::Answer { .. } => {
                self.service.get().cloned().ok_or("ask_gone")?
            }
            _ => self.service.get().cloned().ok_or("device_unknown")?,
        };
        let peers = self.host.peers().ok_or("peer_unavailable")?;
        let store = |error: StoreError| match error {
            StoreError::Unknown => "device_unknown",
            StoreError::Full => "device_limit",
            StoreError::Io(error) => {
                hide_core::diagnostic!(json!({
                    "component": "pairing",
                    "kind": "pairing.not_saved",
                    "reason": error.to_string(),
                }));
                "device_not_saved"
            }
        };
        match ask {
            Ask::Status { .. } | Ask::LookSsh { .. } => unreachable!("answered above"),
            Ask::PairSsh { alias } => {
                let request = self.ssh_request(&alias)?;
                let (reach, me, key) = (self.ssh.clone(), self.me.clone(), peers.key());
                let target = alias.clone();
                let paired = self
                    .ssh_dial(&alias, move || {
                        pairing_ssh::accept(&reach, &target, &request, &me, &key)
                    })
                    .await??;
                let answer = json!({
                    "device_id": paired.accepted.device,
                    "name": paired.accepted.name,
                });
                service
                    .adopt(paired.accepted, paired.first)
                    .map_err(|error| match error {
                        AcceptError::Invalid => "device_own",
                        AcceptError::Already => "device_already_connected",
                        AcceptError::Store(error) => store(error),
                    })?;
                // Adopted: a later pairing of this alias is a new request.
                lock(&self.ssh_requests).retain(|kept| kept.alias != alias);
                Ok(answer)
            }
            Ask::Share { language, held } => {
                self.relay_named(&peers).await;
                let list = WordList::for_language(language.as_deref().unwrap_or_default());
                let offer = service.share(list).map_err(|error| {
                    hide_core::diagnostic!(json!({
                        "component": "pairing",
                        "kind": "pairing.share_failed",
                        "reason": error.to_string(),
                    }));
                    "peer_unavailable"
                })?;
                *lock(&self.held) = held.then(Instant::now);
                if held {
                    tokio::spawn(Arc::clone(self).hold(service.clone()));
                }
                Ok(json!({ "link": offer.link, "expires_unix_ms": offer.expires_unix_ms }))
            }
            Ask::EndShare => {
                *lock(&self.held) = None;
                service.end_offer();
                Ok(json!({}))
            }
            Ask::Join { link } => {
                service.join(&link).map_err(|error| match error {
                    JoinError::Link(hide_peer::link::LinkError::Newer) => "link_newer",
                    JoinError::Link(hide_peer::link::LinkError::NotALink) => "link_invalid",
                    JoinError::Own => "link_own",
                    JoinError::Already => "device_already_connected",
                })?;
                Ok(json!({}))
            }
            Ask::Compare { same } => service.compare(same).then(|| json!({})).ok_or("ask_gone"),
            Ask::DismissJoin => {
                service.dismiss_join();
                Ok(json!({}))
            }
            Ask::Answer { ask, allow } => service
                .answer(ask, allow)
                .then(|| json!({}))
                .ok_or("ask_gone"),
            Ask::Pause { device, direction } => {
                let paired = self.named(&peers, &device).await?;
                let (outgoing, incoming) = directions(&paired.machine, direction)?;
                if outgoing {
                    service.pause_host(&paired.key).map_err(store)?;
                }
                if incoming {
                    service.pause_guest(&paired.key).map_err(store)?;
                }
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::Resume { device, direction } => {
                let paired = self.named(&peers, &device).await?;
                let (outgoing, incoming) = directions(&paired.machine, direction)?;
                if outgoing {
                    service.resume_host(&paired.key).map_err(store)?;
                }
                if incoming {
                    service.resume_guest(&paired.key).map_err(store)?;
                }
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::Remove { device, direction } => {
                let paired = self.named(&peers, &device).await?;
                let (outgoing, incoming) = directions(&paired.machine, direction)?;
                if outgoing {
                    service.remove_host(&paired.key).map_err(store)?;
                }
                if incoming {
                    service.remove_guest(&paired.key).map_err(store)?;
                }
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::Request { device } => {
                let paired = self.named(&peers, &device).await?;
                service.request(paired.key).map_err(|error| match error {
                    RequestError::NotAGuest => "device_not_a_guest",
                    RequestError::Already => "device_already_connected",
                    RequestError::Waiting => "request_waiting",
                })?;
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::DismissRequest { device } => {
                let paired = self.named(&peers, &device).await?;
                service.dismiss_request(&paired.key);
                Ok(json!({}))
            }
            Ask::Retry { device } => {
                let paired = self.named(&peers, &device).await?;
                *lock(&self.relay) = None;
                peers.network_changed();
                self.look.send_modify(|count| *count += 1);
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::Address { device, address } => {
                let paired = self.named(&peers, &device).await?;
                let address: SocketAddr = address.trim().parse().map_err(|_| "address_invalid")?;
                let saved = peers.known(|known| {
                    let mut host = known.of(&paired.key).cloned().ok_or(StoreError::Unknown)?;
                    let first = Candidate::Ip(address);
                    host.candidates.retain(|candidate| *candidate != first);
                    host.candidates.insert(0, first);
                    host.candidates.truncate(MAX_ADDRESSES);
                    known.put(host)
                });
                saved.map_err(store)?;
                peers.network_changed();
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::AcceptSsh {
                request,
                key,
                device,
                name,
                wsl,
            } => {
                let guest = PeerKey::from_hex(&key).map_err(|_| "invalid_request")?;
                self.relay_named(&peers).await;
                let via = if wsl { Via::Wsl } else { Via::Ssh };
                let accepted = service
                    .accept_ssh(&request, guest, &device, &name, via)
                    .map_err(|error| match error {
                        AcceptError::Invalid => "invalid_request",
                        AcceptError::Already => "device_already_connected",
                        AcceptError::Store(error) => store(error),
                    })?;
                Ok(json!({ "accepted": accepted }))
            }
        }
    }

    /// Runs one `ssh` step for `alias` off the runtime, when fewer than
    /// [`SSH_DIALS`] run; the step's `ssh` ends with it.
    async fn ssh_dial<T: Send + 'static>(
        &self,
        alias: &str,
        step: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, &'static str> {
        if !pairing_ssh::valid_alias(alias) {
            return Err("invalid_request");
        }
        let dial = Arc::clone(&self.ssh_dials)
            .try_acquire_owned()
            .map_err(|_| "ssh_busy")?;
        tokio::task::spawn_blocking(move || {
            let _dial = dial;
            step()
        })
        .await
        .map_err(|_| "ssh_not_reached")
    }

    /// The request `alias`'s pairing runs under: the one it ran under in
    /// the last ten minutes, or a new one.
    fn ssh_request(&self, alias: &str) -> Result<String, &'static str> {
        let mut requests = lock(&self.ssh_requests);
        requests.retain(|kept| kept.made.elapsed() < SSH_REQUEST_KEPT);
        if let Some(kept) = requests.iter().find(|kept| kept.alias == alias) {
            return Ok(kept.id.clone());
        }
        let mut id = [0u8; 16];
        getrandom::getrandom(&mut id).map_err(|_| "peer_unavailable")?;
        let id = hex::encode(id);
        if requests.len() == SSH_REQUESTS {
            requests.remove(0);
        }
        requests.push(SshRequest {
            alias: alias.to_owned(),
            id: id.clone(),
            made: Instant::now(),
        });
        Ok(id)
    }

    /// Waits, briefly, until the relay named this machine, so a link or an
    /// SSH answer made right after the endpoint opened says where it is.
    async fn relay_named(&self, peers: &HostPeers) {
        let named = |peers: &HostPeers| {
            peers
                .transport()
                .candidates()
                .iter()
                .any(|candidate| matches!(candidate, Candidate::Relay(_)))
        };
        if named(peers) || peers.transport().relay().await == RelayState::None {
            return;
        }
        let over = Instant::now() + RELAY_WITHIN;
        while !named(peers) && Instant::now() < over {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Ends a share whose terminal stopped asking for its status.
    async fn hold(self: Arc<Self>, service: Service) {
        loop {
            tokio::select! {
                () = self.shutdown.ended() => return,
                () = tokio::time::sleep(HELD_FOR / 4) => {}
            }
            let last = *lock(&self.held);
            let Some(last) = last else {
                return;
            };
            let waiting = {
                let view = service.view();
                let view = view.borrow();
                view.offer.is_some() || !view.asks.is_empty()
            };
            if !waiting {
                *lock(&self.held) = None;
                return;
            }
            if last.elapsed() >= HELD_FOR {
                *lock(&self.held) = None;
                service.end_offer();
                return;
            }
        }
    }

    /// A `pairing_*` event of a window. `own` is whether the window is one
    /// of this machine's, not another machine's view of it.
    pub async fn window_event(self: &Arc<Self>, event: &Value, own: bool) -> Value {
        let kind = event
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut payload = event.get("payload").cloned().unwrap_or_else(|| json!({}));
        let request_id = payload
            .as_object_mut()
            .and_then(|payload| payload.remove("request_id"));
        let verb = kind.strip_prefix("pairing_").unwrap_or_default();
        let ask = payload.as_object_mut().and_then(|payload| {
            payload.insert("verb".to_owned(), json!(verb));
            serde_json::from_value::<Ask>(Value::Object(payload.clone())).ok()
        });
        let mut answer = match ask {
            // A window never holds a share by asking, and never answers
            // for a target of an SSH pairing.
            Some(Ask::Status { .. } | Ask::AcceptSsh { .. }) | None => refused("invalid_request"),
            Some(ask) => {
                let caller = if own {
                    Caller::Operator
                } else {
                    Caller::OtherScreen
                };
                self.ask(ask, caller).await
            }
        };
        answer["request_id"] = request_id.unwrap_or(Value::Null);
        answer["verb"] = json!(verb);
        json!({ "type": "pairing_result", "payload": answer })
    }
}

fn path_of(path: PathKind) -> PeerPath {
    match path {
        PathKind::Direct => PeerPath::Direct,
        PathKind::Relay => PeerPath::Relay,
        PathKind::Local => PeerPath::Local,
    }
}

/// Which directions of `machine` an ask is about: the one it names, or the
/// only one there is. `(outgoing, incoming)`.
fn directions(
    machine: &PeerMachine,
    asked: Option<Direction>,
) -> Result<(bool, bool), &'static str> {
    let (opens, opened_by) = (machine.opens.is_some(), machine.opened_by.is_some());
    match asked {
        Some(Direction::Outgoing) if opens => Ok((true, false)),
        Some(Direction::Incoming) if opened_by => Ok((false, true)),
        Some(Direction::Both) => Ok((opens, opened_by)),
        Some(_) => Err("device_unknown"),
        None if opens && opened_by => Err("device_direction_needed"),
        None => Ok((opens, opened_by)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(opens: bool, opened_by: bool) -> PeerMachine {
        PeerMachine {
            key: "ab".repeat(32),
            device_id: "windows-pc".into(),
            name: "Windows PC".into(),
            opens: opens.then_some(PeerOpens::Active),
            opened_by: opened_by.then_some(PeerOpenedBy::Active),
            connected: false,
            path: None,
            reason: None,
            update: None,
        }
    }

    #[test]
    fn an_ask_names_its_direction_only_when_the_pair_has_two() {
        // One direction: it is the one meant.
        assert_eq!(directions(&machine(true, false), None), Ok((true, false)));
        assert_eq!(directions(&machine(false, true), None), Ok((false, true)));
        // Two: the operator says which, so "pause" never stops more than
        // was meant.
        assert_eq!(
            directions(&machine(true, true), None),
            Err("device_direction_needed")
        );
        assert_eq!(
            directions(&machine(true, true), Some(Direction::Incoming)),
            Ok((false, true))
        );
        assert_eq!(
            directions(&machine(true, true), Some(Direction::Both)),
            Ok((true, true))
        );
        // A direction the pair does not have.
        assert_eq!(
            directions(&machine(true, false), Some(Direction::Incoming)),
            Err("device_unknown")
        );
    }

    #[test]
    fn what_only_the_operator_may_ask_is_refused_to_every_other_caller() {
        let operators = [
            json!({"verb": "share"}),
            json!({"verb": "end_share"}),
            json!({"verb": "join", "link": "hide://pair#v=1"}),
            json!({"verb": "compare", "same": true}),
            json!({"verb": "answer", "ask": 1, "allow": true}),
            json!({"verb": "resume", "device": "pc"}),
            json!({"verb": "request", "device": "pc"}),
            json!({"verb": "address", "device": "pc", "address": "10.0.0.2:7842"}),
            json!({"verb": "accept_ssh", "request": "r", "key": "k", "device": "d", "name": "n"}),
        ];
        for ask in operators {
            let parsed: Ask = serde_json::from_value(ask.clone()).unwrap();
            assert!(parsed.operators(), "{ask}");
        }
        // Stopping and looking take nothing an agent could gain by.
        let anyone = [
            json!({"verb": "status"}),
            json!({"verb": "pause", "device": "pc"}),
            json!({"verb": "remove", "device": "pc", "direction": "incoming"}),
            json!({"verb": "retry", "device": "pc"}),
            json!({"verb": "dismiss_join"}),
            json!({"verb": "dismiss_request", "device": "pc"}),
        ];
        for ask in anyone {
            let parsed: Ask = serde_json::from_value(ask.clone()).unwrap();
            assert!(!parsed.operators(), "{ask}");
        }
        // An ask with a field no build wrote is not guessed at.
        assert!(serde_json::from_value::<Ask>(json!({"verb": "share", "as": "root"})).is_err());
        for code in [
            "device_from_pane",
            "device_from_other_screen",
            "link_invalid",
            "link_newer",
            "link_own",
            "device_already_connected",
            "device_unknown",
            "device_ambiguous",
            "device_direction_needed",
            "device_not_a_guest",
            "request_waiting",
            "ask_gone",
            "address_invalid",
            "peer_unavailable",
            "device_limit",
            "device_not_saved",
            "pairing_ended",
            "pairing_no_offer",
            "pairing_declined",
            "pairing_host_full",
            "pairing_full",
            "pairing_unanswered",
            "pairing_unreachable",
            "pairing_update_needed",
        ] {
            assert!(!next_action(code).starts_with("Retry;"), "{code}");
        }
    }
}
