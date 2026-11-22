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
//!   machine's own windows as a `pairing` frame and to the operator's
//!   terminal as its `status`, and nowhere else: neither to an agent in a
//!   pane, which reads the machines only, nor into the core's snapshot,
//!   which a paired machine may read, nor into a log.
//! - who opens this machine, whom it opens and whether the two are
//!   connected go to the core as one list in one event (D-72).
//!
//! The endpoint is `peer_host`'s. Asking for a status opens nothing; sharing,
//! adding and acting on a machine the records name do (D-83).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use hide_core::peer_machines::{
    PeerMachine, PeerOpenedBy, PeerOpens, PeerPath, PeerReason, PeerUpdate,
};
use hide_peer::pairing::{AcceptError, JoinError, Me, Pairings, RequestError, View};
use hide_peer::protocol::hello::{Behind, GrantState};
use hide_peer::protocol::{Candidate, PathKind, PeerKey, StreamKind};
use hide_peer::store::{Grant, GrantStatus, Host, KnownState, StoreError, Via};
use hide_peer::words::WordList;
use hide_peer::{PeerEvent, PeerTransport, RelayState};
use hide_peer_iroh::IrohTransport;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{OnceCell, Semaphore, watch};
use tokio::time::Instant;

use crate::core::CoreHandle;
use crate::ending::Ending;
use crate::guests::HelperFolder;
use crate::pairing_ssh;
use crate::peer_host::{HostPeers, Incoming, Link, PeerHost};

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
        /// With the direction that takes a guest off this machine: end the
        /// work it runs here at once instead of leaving it to run (B26).
        #[serde(default)]
        end_work: bool,
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
    /// Switches the helper device this machine dials as `alias` to the
    /// connection of the Hide on its machine (D-49, B36): that Hide is
    /// paired over the device's SSH login and, once the connection is
    /// confirmed, the device goes on over it. `own_folder` goes on without
    /// the helper's folder, which another machine took first (D-75).
    Switch {
        device: String,
        #[serde(default)]
        own_folder: bool,
        /// The operator allowed a Hide to be started on a machine that
        /// runs none of its own (D-45).
        #[serde(default)]
        install: bool,
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
        /// The asking machine dialed a device helper on this account,
        /// installed under this root, and asks for that helper's folder
        /// and terminals with the grant (D-49). When another machine took
        /// them first nothing is saved and the answer names that machine
        /// (D-75).
        #[serde(default)]
        helper: Option<String>,
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
            | Self::Switch { .. }
            | Self::AcceptSsh { .. } => true,
            Self::Status { .. }
            | Self::DismissJoin
            | Self::Pause { .. }
            | Self::Remove { .. }
            | Self::DismissRequest { .. }
            | Self::Retry { .. } => false,
        }
    }

    /// What a bot needs of its mode to ask it (mission-87): reading the
    /// machines and dialing one again are the whole Hide's, as `hide device
    /// list` and `retry` are; pairing, stopping and removing a machine are
    /// the operator's alone.
    fn bot_need(&self) -> hide_core::access::Need {
        match self {
            Self::Status { .. } | Self::Retry { .. } => hide_core::access::Need::All,
            _ => hide_core::access::Need::Operator,
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
    /// A bot, in the mode its key proved at the bootstrap socket: what
    /// `Ask::bot_need` says of its mode (mission-87).
    Bot(hide_core::access::Mode),
}

/// The refusal of every ask in a build that does not offer connections
/// between two Hides (`env::Env::device_connect` off).
pub const DEVICE_CONNECT_OFF: &str = "device_connect_unavailable";

/// What a caller does about a refusal `code`.
pub fn next_action(code: &str) -> &'static str {
    match code {
        DEVICE_CONNECT_OFF => {
            "This build does not offer connections between two Hides yet: add a machine by its SSH alias"
        }
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
        "device_ambiguous" => "Name the machine by its connection_id from hide device list --json",
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
        "ssh_mac_from_mac_only" => {
            "A Mac takes a Hide only from a Mac: install Hide on that Mac and add it by a link it shares"
        }
        "ssh_mac_signed_out" => {
            "Nobody is logged in to that Mac: log in there, then try the switch again"
        }
        "switch_not_connected" => {
            "The two machines were paired and the new connection did not open in time; the device is used as before. Try the switch again"
        }
        "switch_running" => "This device's switch is already running",
        "ssh_core_not_started" => {
            "Hide could not be started on that machine; the device is used as before. Try the switch again, or open Hide there"
        }
        "switch_unsupported" => {
            "Only a device this Hide dials over SSH and has connected to once can be switched"
        }
        "device_own" => "That is this machine itself",
        code => crate::access::next_action(code)
            .unwrap_or("Retry; the reason is in Hide's diagnostic log"),
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

/// How far a switch got with its target, for a device that was added for
/// that switch (B41): such a device is taken off again only when the target
/// was asked and nothing was started there. A switch that never reached
/// its SSH login (too many run, [`SSH_DIALS`]) said nothing about the
/// target, and one that started a Hide there or had a grant saved there
/// left something the device still reaches.
#[derive(Default)]
struct SwitchReach(std::sync::atomic::AtomicU8);

impl SwitchReach {
    const ASKED: u8 = 1;
    const STARTED: u8 = 2;

    /// The SSH login to the target runs.
    fn asked(&self) {
        self.0
            .fetch_max(Self::ASKED, std::sync::atomic::Ordering::AcqRel);
    }

    /// A Hide is being started on the target, or it answered a pairing.
    fn started(&self) {
        self.0
            .fetch_max(Self::STARTED, std::sync::atomic::Ordering::AcqRel);
    }

    fn clean(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire) == Self::ASKED
    }
}

struct RelayAnswer {
    state: RelayState,
    at: Instant,
}

/// What the relay last said of itself, and whether it is being asked now.
#[derive(Default)]
struct Relay {
    answer: Option<RelayAnswer>,
    asking: bool,
}

pub struct Pairing {
    host: Arc<PeerHost>,
    core: Arc<CoreHandle>,
    /// What keeps this machine's core on once it is paired.
    always_on: crate::always_on::Follower,
    me: Me,
    service: OnceCell<Service>,
    frame: watch::Sender<Arc<Value>>,
    /// Counts the asks that want the paired machines read again.
    look: watch::Sender<u64>,
    /// Which machines cannot connect until one side updates.
    updates: Mutex<HashMap<PeerKey, PeerUpdate>>,
    relay: Mutex<Relay>,
    /// The machines the core was last told, and the turn to tell it.
    told: tokio::sync::Mutex<Option<Vec<PeerMachine>>>,
    /// What runs the guests' work on this machine, handed in at the
    /// daemon's start: it follows who is a guest here.
    guests: OnceLock<Arc<crate::guests::Guests>>,
    /// When the terminal that holds the open share last asked.
    held: Mutex<Option<Instant>>,
    /// How a target's `hide` is reached over SSH.
    ssh: pairing_ssh::Reach,
    /// The SSH dials under way: each is one `ssh` this daemon owns.
    ssh_dials: Arc<Semaphore>,
    /// The request each alias's pairing runs under, so one that ssh lost
    /// midway is asked again as the same request (security spec 15).
    ssh_requests: Mutex<Vec<SshRequest>>,
    /// Held by an SSH pairing that asks for the device helper's folder,
    /// from its look at who holds the folder to its record of having taken
    /// it: two machines that ask at once are answered one after the other.
    helper_asks: tokio::sync::Mutex<()>,
    /// The devices whose switch runs now.
    switching: Mutex<std::collections::HashSet<String>>,
    shutdown: Ending,
}

/// The most `ssh` this daemon runs at once to look at or pair with a
/// machine; an ask past it is refused, not queued. A switch holds its one
/// for as long as its login is open: through the wait for the new
/// connection ([`SWITCH_CONNECTS_WITHIN`]) when the device has projects to
/// hand over after it.
const SSH_DIALS: usize = 4;

/// How long a pairing's request is asked again as itself: as long as the
/// target keeps its pending grant.
const SSH_REQUEST_KEPT: Duration = Duration::from_secs(10 * 60);

/// How long a switch waits for the connection to confirm the pairing it
/// just made: the dial, the hello and the confirmation, through a relay
/// when no direct path opens.
pub(crate) const SWITCH_CONNECTS_WITHIN: Duration = Duration::from_secs(90);

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
        always_on: crate::always_on::Follower,
        me: Me,
        ssh: pairing_ssh::Reach,
        shutdown: Ending,
    ) -> Arc<Self> {
        let pairing = Arc::new(Self {
            host: Arc::clone(&host),
            core,
            always_on,
            me,
            service: OnceCell::new(),
            frame: watch::channel(Arc::new(json!(View::default()))).0,
            look: watch::channel(0).0,
            updates: Mutex::new(HashMap::new()),
            relay: Mutex::new(Relay::default()),
            told: tokio::sync::Mutex::new(None),
            guests: OnceLock::new(),
            held: Mutex::new(None),
            ssh,
            ssh_dials: Arc::new(Semaphore::new(SSH_DIALS)),
            ssh_requests: Mutex::new(Vec::new()),
            helper_asks: tokio::sync::Mutex::default(),
            switching: Mutex::default(),
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

    /// What runs the guests' work on this machine. Given once, at the
    /// daemon's start, before the endpoint opens.
    pub fn follow_guests(&self, guests: Arc<crate::guests::Guests>) {
        let _ = self.guests.set(guests);
    }

    /// The daemon's start opened no endpoint: the core hears the machines
    /// the records still name, unless the service started meanwhile and
    /// told it.
    pub async fn rested(&self) {
        let machines: Vec<PeerMachine> = self
            .recorded()
            .await
            .into_iter()
            .map(|paired| paired.machine)
            .collect();
        let mut told = self.told.lock().await;
        if told.is_none() && !machines.is_empty() {
            self.paired_changed(&machines).await;
            *told = Some(machines);
        }
    }

    /// The machines the records name while the endpoint is not open: a
    /// machine whose every host dropped the pairing opens none at its start
    /// and still lists them, to be removed or paired again.
    async fn recorded(&self) -> Vec<Paired> {
        match self.host.recorded().await {
            Ok((guests, hosts)) => rows(&hosts, &guests, |_| None, &HashMap::new()).0,
            Err(error) => {
                hide_core::diagnostic!(json!({
                    "component": "pairing",
                    "kind": "pairing.records_unread",
                    "reason": error.to_string(),
                }));
                Vec::new()
            }
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
                .into_iter()
                .map(|paired| paired.machine)
                .collect();
            {
                let mut told = self.told.lock().await;
                if told.as_ref() != Some(&machines) {
                    self.paired_changed(&machines).await;
                    *told = Some(machines);
                }
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
    /// follows "is this machine paired with anything" belongs here: the
    /// core's list (D-72) and the login item that keeps a paired core on
    /// (D-41).
    async fn paired_changed(&self, machines: &[PeerMachine]) {
        self.always_on.peers_changed();
        // A guest's work here follows its grant: one that holds none any
        // more was taken off this machine (`Guests::granted`).
        if let Some(guests) = self.guests.get() {
            guests.granted(
                machines
                    .iter()
                    .filter(|machine| machine.opened_by.is_some())
                    .filter_map(|machine| PeerKey::from_hex(&machine.key).ok())
                    .collect(),
            );
        }
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

    /// What the relay last said of itself, to explain a host that is not
    /// connected. The relay is never asked here: an answer older than
    /// [`RELAY_ANSWER_FOR`], or none yet, starts the one task that asks and
    /// has the machines read again, so neither the follow loop nor an ask
    /// waits on the relay.
    fn relay_state(self: &Arc<Self>, peers: &HostPeers) -> RelayState {
        let mut relay = lock(&self.relay);
        let fresh = relay
            .answer
            .as_ref()
            .is_some_and(|answer| answer.at.elapsed() < RELAY_ANSWER_FOR);
        if !fresh && !relay.asking {
            relay.asking = true;
            let (pairing, peers) = (Arc::clone(self), peers.clone());
            tokio::spawn(async move {
                let state = tokio::select! {
                    () = pairing.shutdown.ended() => None,
                    state = peers.transport().relay() => Some(state),
                };
                {
                    let mut relay = lock(&pairing.relay);
                    relay.asking = false;
                    let Some(state) = state else { return };
                    relay.answer = Some(RelayAnswer {
                        state,
                        at: Instant::now(),
                    });
                }
                pairing.look.send_modify(|count| *count += 1);
            });
        }
        relay
            .answer
            .as_ref()
            .map_or(RelayState::None, |answer| answer.state)
    }

    /// Every machine this one is paired with, in either direction, by name.
    fn paired(self: &Arc<Self>, peers: &HostPeers) -> Vec<Paired> {
        let hosts = peers.hosts();
        let guests = peers.grants(|grants, now| grants.list(now).cloned().collect::<Vec<_>>());
        let updates = lock(&self.updates).clone();
        let (mut paired, away) = rows(&hosts, &guests, |key| peers.link(key), &updates);
        // Why a host this machine dials is not connected: the relay's own
        // answer when it has one (B34), else nothing more than "not
        // connected".
        if away {
            let reason = match self.relay_state(peers) {
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
        paired
    }

    /// The paired machine `text` names.
    fn named(self: &Arc<Self>, peers: &HostPeers, text: &str) -> Result<Paired, &'static str> {
        name_one(self.paired(peers), text)
    }

    /// Answers one ask. The answer is `{"ok": true, ...}` or a refusal with
    /// its code and what to do about it. A build that does not offer
    /// connections between two Hides refuses every ask and does nothing
    /// (`env::Env::device_connect`).
    pub async fn ask(self: &Arc<Self>, ask: Ask, caller: Caller) -> Value {
        if !self.host.device_connect() {
            return refused(DEVICE_CONNECT_OFF);
        }
        match caller {
            Caller::Operator => {}
            Caller::Pane if !ask.operators() => {}
            Caller::Pane => return refused("device_from_pane"),
            Caller::OtherScreen => return refused("device_from_other_screen"),
            Caller::Bot(mode) => {
                if let Err(code) = hide_core::access::admit(mode, ask.bot_need()) {
                    return refused(code);
                }
            }
        }
        match self.run(ask, caller).await {
            Ok(mut answer) => {
                answer["ok"] = json!(true);
                answer
            }
            Err(code) => refused(code),
        }
    }

    async fn run(self: &Arc<Self>, ask: Ask, caller: Caller) -> Result<Value, &'static str> {
        // Reading opens nothing: a machine paired with nothing has nothing
        // to say and no endpoint to open for it.
        if let Ask::Status { hold } = ask {
            // Each machine as `hide device list` answers it.
            let answers = |paired: Vec<Paired>| -> Vec<Value> {
                paired
                    .iter()
                    .map(|paired| hide_core::peer_machines::answer(&paired.machine))
                    .collect()
            };
            let Some(service) = self.service.get() else {
                let machines = answers(self.recorded().await);
                return Ok(json!({ "pairing": View::default(), "machines": machines }));
            };
            // The pairings under way, with their link and words, are the
            // operator's: an agent in a pane reads the machines and holds
            // no share.
            let operator = caller == Caller::Operator;
            if operator
                && hold
                && let Some(held) = lock(&self.held).as_mut()
            {
                *held = Instant::now();
            }
            let peers = self.host.peers().ok_or("peer_unavailable")?;
            let machines = answers(self.paired(&peers));
            let pairing = match operator {
                true => json!(*service.view().borrow()),
                false => json!(View::default()),
            };
            return Ok(json!({ "pairing": pairing, "machines": machines }));
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
            Ask::Share { .. }
            | Ask::Join { .. }
            | Ask::PairSsh { .. }
            | Ask::Switch { .. }
            | Ask::AcceptSsh { .. } => self.service().await?,
            Ask::EndShare | Ask::DismissJoin => match self.service.get() {
                Some(service) => service.clone(),
                None => return Ok(json!({})),
            },
            Ask::Compare { .. } | Ask::Answer { .. } => {
                self.service.get().cloned().ok_or("ask_gone")?
            }
            _ => match self.service.get() {
                Some(service) => service.clone(),
                // The endpoint is shut and the records still name a machine
                // (every host dropped the pairing): acting on one opens it.
                None if !self.recorded().await.is_empty() => self.service().await?,
                None => return Err("device_unknown"),
            },
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
            Ask::Switch {
                device,
                own_folder,
                install,
            } => {
                // The device as the core knows it: a terminal may name it
                // by its alias.
                let (core, named) = (Arc::clone(&self.core), device);
                let source = tokio::task::spawn_blocking(move || core.device_switch_source(&named))
                    .await
                    .map_err(|_| "hide_unavailable")??;
                let device = source.device_id.clone();
                // One switch of a device at a time: a second press while
                // the first runs changes nothing.
                if !lock(&self.switching).insert(device.clone()) {
                    return Err("switch_running");
                }
                let far = Arc::new(SwitchReach::default());
                // The window that pressed reads which device its answer is for.
                let answer = self
                    .switch(&service, &peers, source, own_folder, install, &far)
                    .await
                    .map(|mut answer| {
                        answer["device_id"] = json!(device);
                        answer
                    });
                lock(&self.switching).remove(&device);
                match &answer {
                    // The operator is asked something first: no step shows.
                    Ok(answer) if answer["switch"] != "done" => {
                        self.switch_step(&device, "ended", json!({})).await;
                    }
                    Ok(_) => {}
                    Err(code) => {
                        self.switch_step(
                            &device,
                            "failed",
                            json!({ "reason": code, "clean": far.clean() }),
                        )
                        .await;
                    }
                }
                answer
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
                    let (pairing, service) = (Arc::clone(self), service.clone());
                    tokio::spawn(async move {
                        let waiting = || {
                            let view = service.view();
                            let view = view.borrow();
                            view.offer.is_some() || !view.asks.is_empty()
                        };
                        hold(&pairing.shutdown, &pairing.held, waiting, || {
                            service.end_offer()
                        })
                        .await;
                    });
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
                let paired = self.named(&peers, &device)?;
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
                let paired = self.named(&peers, &device)?;
                let (outgoing, incoming) = directions(&paired.machine, direction)?;
                if outgoing {
                    service.resume_host(&paired.key).map_err(store)?;
                }
                if incoming {
                    service.resume_guest(&paired.key).map_err(store)?;
                }
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::Remove {
                device,
                direction,
                end_work,
            } => {
                let paired = self.named(&peers, &device)?;
                let (outgoing, incoming) = directions(&paired.machine, direction)?;
                if outgoing {
                    service.remove_host(&paired.key).map_err(store)?;
                }
                if incoming {
                    service.remove_guest(&paired.key).map_err(store)?;
                    // The guest is cut off first: its work is ended with
                    // no way back in for it.
                    if end_work && let Some(guests) = self.guests.get() {
                        guests.end_work(paired.key);
                    }
                }
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::Request { device } => {
                let paired = self.named(&peers, &device)?;
                service.request(paired.key).map_err(|error| match error {
                    RequestError::NotAGuest => "device_not_a_guest",
                    RequestError::Already => "device_already_connected",
                    RequestError::Waiting => "request_waiting",
                })?;
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::DismissRequest { device } => {
                let paired = self.named(&peers, &device)?;
                service.dismiss_request(&paired.key);
                Ok(json!({}))
            }
            Ask::Retry { device } => {
                let paired = self.named(&peers, &device)?;
                lock(&self.relay).answer = None;
                peers.network_changed();
                self.look.send_modify(|count| *count += 1);
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::Address { device, address } => {
                let paired = self.named(&peers, &device)?;
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
                helper,
            } => {
                let guest = PeerKey::from_hex(&key).map_err(|_| "invalid_request")?;
                self.relay_named(&peers).await;
                let via = if wsl { Via::Wsl } else { Via::Ssh };
                let _one = match helper {
                    Some(_) => Some(self.helper_asks.lock().await),
                    None => None,
                };
                // Who holds the helper's folder is read before the grant is
                // saved: a machine that came second saves nothing until its
                // operator said yes to losing the terminals (D-75).
                if let Some(root) = &helper
                    && let HelperFolder::HeldBy(holder) =
                        self.helper(&peers, guest, root, false).await
                {
                    let by = self
                        .paired(&peers)
                        .into_iter()
                        .find(|paired| paired.key == holder)
                        .map(|paired| paired.machine.name)
                        .unwrap_or_default();
                    return Ok(json!({ "helper": "held", "by": by }));
                }
                let accepted = match service.accept_ssh(&request, guest, &device, &name, via) {
                    Ok(accepted) => json!(accepted),
                    // A machine that is paired already asks for the folder
                    // alone.
                    Err(AcceptError::Already) if helper.is_some() => Value::Null,
                    Err(AcceptError::Invalid) => return Err("invalid_request"),
                    Err(AcceptError::Already) => return Err("device_already_connected"),
                    Err(AcceptError::Store(error)) => return Err(store(error)),
                };
                let Some(root) = helper else {
                    return Ok(json!({ "accepted": accepted }));
                };
                let folder = match self.helper(&peers, guest, &root, true).await {
                    HelperFolder::Taken => "taken",
                    HelperFolder::Absent => "absent",
                    // Read as free a moment ago, under the same lock.
                    HelperFolder::Kept | HelperFolder::HeldBy(_) => "kept",
                };
                Ok(json!({ "accepted": accepted, "helper": folder }))
            }
        }
    }

    /// The steps of [`Ask::Switch`], each told to the core as it starts so
    /// the device's row draws it. Whatever stops it before the connection
    /// is confirmed leaves the device a helper device: the core changes
    /// its link only when the paired machine's grant is active.
    async fn switch(
        self: &Arc<Self>,
        service: &Service,
        peers: &HostPeers,
        source: hide_core::DeviceSwitchSource,
        own_folder: bool,
        install: bool,
        far: &Arc<SwitchReach>,
    ) -> Result<Value, &'static str> {
        let device = source.device_id.as_str();
        let alias = source.ssh_alias.as_str();
        self.switch_step(device, "checking", json!({})).await;
        let request = self.ssh_request(alias)?;
        let (reach, me, key) = (self.ssh.clone(), self.me.clone(), peers.key());
        let target = alias.to_owned();
        let (helper_root, press) = (source.helper_root.clone(), source.sign_in.clone());
        let projects = source.projects.clone();
        // The install's step is told from the thread that runs ssh.
        let (steps, runtime) = (Arc::clone(self), tokio::runtime::Handle::current());
        let installed = device.to_owned();
        let installing = {
            let (runtime, far) = (runtime.clone(), Arc::clone(far));
            move || {
                far.started();
                runtime.block_on(steps.switch_step(&installed, "installing", json!({})));
            }
        };
        // What follows the target's answer runs on the thread that holds
        // the SSH login, before its session ends: the machine's own Hide is
        // handed the device's projects only once it admitted this machine.
        let landed: Arc<Mutex<Option<Result<Value, &'static str>>>> = Arc::default();
        let admitted = {
            let (this, service, peers) = (Arc::clone(self), service.clone(), peers.clone());
            let (device, alias, landed) =
                (device.to_owned(), alias.to_owned(), Arc::clone(&landed));
            let far = Arc::clone(far);
            move |switched: &pairing_ssh::Switched| {
                // The target answered a pairing: it holds a grant now.
                far.started();
                let done = runtime
                    .block_on(this.admit_switch(&service, &peers, &device, &alias, switched));
                let active = done.is_ok();
                *lock(&landed) = Some(done);
                active
            }
        };
        let asked = Arc::clone(far);
        let switched = self
            .ssh_dial(alias, move || {
                asked.asked();
                pairing_ssh::switch(&pairing_ssh::Switch {
                    reach: &reach,
                    alias: &target,
                    request: &request,
                    me: &me,
                    key: &key,
                    helper_root: (!own_folder).then_some(helper_root.as_str()),
                    projects: &projects,
                    admitted: &admitted,
                    press,
                    install,
                    installing: &installing,
                })
            })
            .await??;
        match switched {
            pairing_ssh::Switched::HeldBy(by) => Ok(json!({ "switch": "held", "by": by })),
            pairing_ssh::Switched::InstallNeeded { system } => {
                Ok(json!({ "switch": "install_needed", "system": system }))
            }
            pairing_ssh::Switched::Paired { .. } => {
                lock(&landed).take().unwrap_or(Err("switch_not_connected"))
            }
        }
    }

    /// What follows the target's answer to a switch: the core is told the
    /// machine the device becomes, the pairing is saved, and the connection
    /// confirms the grant. The core takes the device over the moment it is
    /// active.
    async fn admit_switch(
        self: &Arc<Self>,
        service: &Service,
        peers: &HostPeers,
        device: &str,
        alias: &str,
        switched: &pairing_ssh::Switched,
    ) -> Result<Value, &'static str> {
        let pairing_ssh::Switched::Paired {
            paired,
            folder,
            needs,
        } = switched
        else {
            return Err("switch_not_connected");
        };
        // The machine the device is: the one that just answered, or the
        // one this machine is paired with already under the device's id.
        let host = match paired {
            Some(paired) => paired.accepted.key,
            None => self
                .paired(peers)
                .into_iter()
                .find(|paired| paired.machine.device_id == device && paired.machine.opens.is_some())
                .map(|paired| paired.key)
                .ok_or("device_unknown")?,
        };
        // What the Hide started there still needs goes with the step, and
        // onto the device's row once it is that machine's (B30).
        let mut connecting = json!({ "key": host.to_hex() });
        if let Some(needs) = needs {
            connecting["needs"] = needs.payload();
        }
        self.switch_step(device, "connecting", connecting).await;
        if let Some(paired) = paired.clone() {
            match service.adopt(paired.accepted, paired.first) {
                // Adopted before, by a switch that stopped after it.
                Ok(()) | Err(AcceptError::Already) => {}
                Err(AcceptError::Invalid) => return Err("device_own"),
                Err(AcceptError::Store(StoreError::Full)) => return Err("device_limit"),
                Err(AcceptError::Store(_)) => return Err("device_not_saved"),
            }
            lock(&self.ssh_requests).retain(|kept| kept.alias != alias);
        }
        let over = Instant::now() + SWITCH_CONNECTS_WITHIN;
        loop {
            let active = self.paired(peers).into_iter().any(|paired| {
                paired.key == host && paired.machine.opens == Some(PeerOpens::Active)
            });
            if active {
                return Ok(json!({ "switch": "done", "folder": folder }));
            }
            if Instant::now() >= over || self.shutdown.has_ended() {
                return Err("switch_not_connected");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Tells the core one step of `device`'s switch (`device_switch`).
    async fn switch_step(&self, device: &str, step: &str, mut more: Value) {
        more["device_id"] = json!(device);
        more["step"] = json!(step);
        let event = json!({
            "schema_version": hide_core::SCHEMA_VERSION,
            "kind": "device_switch",
            "payload": more,
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
                "kind": "pairing.switch_not_told",
                "device": device,
                "step": step,
                "reason": error,
            }));
        }
    }

    /// Who holds the device helper's folder on this machine, as `guest`
    /// finds it; with `take`, `guest` takes it when it is free.
    async fn helper(
        self: &Arc<Self>,
        peers: &HostPeers,
        guest: PeerKey,
        root: &str,
        take: bool,
    ) -> HelperFolder {
        let Some(guests) = self.guests.get().cloned() else {
            return HelperFolder::Absent;
        };
        let granted: Vec<PeerKey> = self
            .paired(peers)
            .into_iter()
            .filter(|paired| paired.machine.opened_by.is_some())
            .map(|paired| paired.key)
            .collect();
        let root = root.to_owned();
        tokio::task::spawn_blocking(move || {
            let paired = |key: &PeerKey| granted.contains(key);
            match take {
                true => guests.take_helper(&guest, &root, paired),
                false => guests.helper_for(&guest, &root, paired),
            }
        })
        .await
        .unwrap_or(HelperFolder::Kept)
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

/// Ends a share whose terminal stopped asking for its status. `held` is when
/// the terminal last asked, `waiting` whether the share still has a link or
/// a question, and `end` ends it. A terminal that keeps asking, as it does
/// while its own question waits for the operator, keeps the share for as
/// long as it asks.
async fn hold(
    shutdown: &Ending,
    held: &Mutex<Option<Instant>>,
    waiting: impl Fn() -> bool,
    end: impl FnOnce(),
) {
    loop {
        tokio::select! {
            () = shutdown.ended() => return,
            () = tokio::time::sleep(HELD_FOR / 4) => {}
        }
        let Some(last) = *lock(held) else {
            return;
        };
        if !waiting() {
            *lock(held) = None;
            return;
        }
        if last.elapsed() >= HELD_FOR {
            *lock(held) = None;
            end();
            return;
        }
    }
}

/// One row per machine `hosts` and `guests` name, by name, and whether a
/// host this machine dials has no connection. `link` is the machine's
/// connection, when it has one.
fn rows(
    hosts: &[Host],
    guests: &[Grant],
    link: impl Fn(&PeerKey) -> Option<Link>,
    updates: &HashMap<PeerKey, PeerUpdate>,
) -> (Vec<Paired>, bool) {
    let mut paired: Vec<Paired> = Vec::new();
    let mut away = false;
    for host in hosts {
        let link = link(&host.key);
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
    // A pending grant is a pairing under way, not a paired machine.
    for guest in guests
        .iter()
        .filter(|guest| guest.state != GrantStatus::Pending)
    {
        let opened_by = Some(match guest.state {
            GrantStatus::Paused => PeerOpenedBy::Paused,
            _ => PeerOpenedBy::Active,
        });
        if let Some(known) = paired.iter_mut().find(|paired| paired.key == guest.key) {
            known.machine.opened_by = opened_by;
            continue;
        }
        let link = link(&guest.key);
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
    paired.sort_by(|a, b| {
        (a.machine.name.to_lowercase(), &a.machine.key)
            .cmp(&(b.machine.name.to_lowercase(), &b.machine.key))
    });
    (paired, away)
}

/// The one of `paired` that `text` names: by its connection id, its device
/// id, its name, or the start of its connection id. A device id is what a
/// machine says of itself, so two machines can say the same one; then only
/// the connection id, which each proved, names one of them.
fn name_one(mut paired: Vec<Paired>, text: &str) -> Result<Paired, &'static str> {
    let text = text.trim();
    let matching = |paired: &[Paired], named: &dyn Fn(&PeerMachine) -> bool| -> Vec<usize> {
        paired
            .iter()
            .enumerate()
            .filter(|(_, paired)| named(&paired.machine))
            .map(|(at, _)| at)
            .collect()
    };
    let found = [
        matching(&paired, &|machine| machine.key == text),
        matching(&paired, &|machine| machine.device_id == text),
        matching(&paired, &|machine| {
            machine.name.eq_ignore_ascii_case(text)
                || (text.len() >= 8 && machine.key.starts_with(text))
        }),
    ];
    match found
        .iter()
        .find(|found| !found.is_empty())
        .map(Vec::as_slice)
    {
        Some([at]) => Ok(paired.swap_remove(*at)),
        Some(_) => Err("device_ambiguous"),
        None => Err("device_unknown"),
    }
}

fn path_of(path: PathKind) -> PeerPath {
    match path {
        PathKind::Direct => PeerPath::Direct,
        PathKind::Tailscale => PeerPath::Tailscale,
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

    fn paired(key: u8, device: &str, name: &str) -> Paired {
        let key = PeerKey::from_bytes([key; 32]);
        Paired {
            key,
            machine: PeerMachine {
                key: key.to_hex(),
                device_id: device.into(),
                name: name.into(),
                ..machine(true, false)
            },
        }
    }

    #[test]
    fn two_machines_that_say_one_device_id_are_named_only_by_their_connection_ids() {
        let both = || {
            vec![
                paired(1, "windows-pc", "Office"),
                paired(2, "windows-pc", "Imposter"),
                paired(3, "macbook", "MacBook"),
            ]
        };
        let named = |text: &str| name_one(both(), text).map(|paired| paired.key);
        // The shared device id names neither, whichever was paired first.
        assert_eq!(named("windows-pc"), Err("device_ambiguous"));
        // Each connection id, which the machine proved, names its own row.
        for key in [1, 2] {
            let key = PeerKey::from_bytes([key; 32]);
            assert_eq!(named(&key.to_hex()), Ok(key));
            assert_eq!(named(&key.to_hex()[..8]), Ok(key));
        }
        // A device id only one machine says still names it, and so does a
        // name.
        assert_eq!(named("macbook"), Ok(PeerKey::from_bytes([3; 32])));
        assert_eq!(named(" imposter "), Ok(PeerKey::from_bytes([2; 32])));
        assert_eq!(named("nobody"), Err("device_unknown"));
    }

    #[test]
    fn a_machine_the_records_name_has_its_row_without_a_connection() {
        let host = |key: u8, state| Host {
            key: PeerKey::from_bytes([key; 32]),
            device: format!("device-{key}"),
            name: format!("Machine {key}"),
            grant: "grant".into(),
            state,
            candidates: Vec::new(),
        };
        // A host that dropped the pairing is listed and is nothing to dial.
        let (dropped, away) = rows(
            &[host(1, KnownState::Unpaired)],
            &[],
            |_| None,
            &HashMap::new(),
        );
        assert_eq!(dropped[0].machine.opens, Some(PeerOpens::Unpaired));
        assert!(!dropped[0].machine.connected);
        assert!(!away);
        // A host this machine still dials, and has no connection to, is.
        let (_, away) = rows(
            &[host(2, KnownState::Active)],
            &[],
            |_| None,
            &HashMap::new(),
        );
        assert!(away);
    }

    #[tokio::test(start_paused = true)]
    async fn a_held_share_lives_while_its_terminal_asks_and_ends_when_it_stops() {
        let shutdown = Ending::new();
        let held = Arc::new(Mutex::new(Some(Instant::now())));
        let ended = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let holder = tokio::spawn({
            let (shutdown, held, ended) = (shutdown.clone(), Arc::clone(&held), Arc::clone(&ended));
            async move {
                let end = || ended.store(true, std::sync::atomic::Ordering::SeqCst);
                hold(&shutdown, &held, || true, end).await;
            }
        });
        // The terminal asks twice a second, as `hide device share` does
        // while its allow question waits: three times what an unheld share
        // lives, and the share is still there.
        let asked = Instant::now();
        while asked.elapsed() < HELD_FOR * 3 {
            tokio::time::advance(Duration::from_millis(500)).await;
            *lock(&held) = Some(Instant::now());
        }
        assert!(!ended.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!holder.is_finished());
        // The terminal goes away: the share ends by the next look past
        // HELD_FOR.
        let last = Instant::now();
        tokio::time::timeout(HELD_FOR * 2, holder)
            .await
            .expect("the share ended")
            .unwrap();
        assert!(ended.load(std::sync::atomic::Ordering::SeqCst));
        assert!(last.elapsed() <= HELD_FOR + HELD_FOR / 4);
        assert_eq!(*lock(&held), None);
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

    /// mission-87: a bot reads the machines and dials one again in the
    /// whole Hide's mode, and pairs, stops or removes none in either.
    #[test]
    fn a_bot_reads_and_retries_machines_and_pairs_or_removes_none() {
        use hide_core::access::{Mode, NOT_OWN_WORK, OPERATOR_ONLY, admit};
        let ask = |value: Value| serde_json::from_value::<Ask>(value).unwrap();
        for read in [
            json!({"verb": "status"}),
            json!({"verb": "retry", "device": "pc"}),
        ] {
            assert_eq!(
                admit(Mode::All, ask(read.clone()).bot_need()),
                Ok(()),
                "{read}"
            );
            assert_eq!(
                admit(Mode::Own, ask(read.clone()).bot_need()),
                Err(NOT_OWN_WORK),
                "{read}"
            );
        }
        for operators in [
            json!({"verb": "pause", "device": "pc"}),
            json!({"verb": "remove", "device": "pc"}),
            json!({"verb": "resume", "device": "pc"}),
            json!({"verb": "share"}),
            json!({"verb": "join", "link": "hide://pair#v=1"}),
        ] {
            for mode in [Mode::All, Mode::Own] {
                assert_eq!(
                    admit(mode, ask(operators.clone()).bot_need()),
                    Err(OPERATOR_ONLY),
                    "{operators}"
                );
            }
        }
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

    /// Every refusal the contract names for `hide device switch` says what
    /// to do about it: a reason the screen has words for and the terminal
    /// answers with "Retry" is one decision kept in one place only.
    #[test]
    fn every_refusal_of_a_switch_says_what_to_do() {
        let switch = crate::cli_contract::COMMANDS
            .iter()
            .find(|spec| spec.words == ["device", "switch"])
            .expect("the contract names the switch");
        for code in ["ssh_mac_from_mac_only", "ssh_mac_signed_out", "ssh_busy"] {
            assert!(switch.refusals.contains(&code), "{code}");
        }
        for code in switch.refusals {
            assert!(!next_action(code).starts_with("Retry;"), "{code}");
        }
    }

    /// B41: a device added for its switch is taken off again only when the
    /// target was asked and nothing was started there. A switch that never
    /// got its SSH login said nothing about the target, so the device stays
    /// with its reason and Try again.
    #[test]
    fn a_switch_is_clean_only_when_its_target_was_asked_and_nothing_started_there() {
        let far = SwitchReach::default();
        assert!(!far.clean(), "no SSH login ran");
        far.asked();
        assert!(far.clean());
        far.started();
        assert!(!far.clean());
        far.asked();
        assert!(!far.clean(), "what was started there stays started");
    }
}
