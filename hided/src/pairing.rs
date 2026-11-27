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

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use hide_core::peer_machines::{
    PeerMachine, PeerOpenedBy, PeerOpens, PeerPath, PeerReason, PeerSsh, PeerSshBlocked, PeerUpdate,
};
use hide_peer::pairing::{AcceptError, Accepted, JoinError, Me, Pairings, RequestError, View};
use hide_peer::proof::Proof;
use hide_peer::protocol::hello::{Behind, GrantState};
use hide_peer::protocol::{Candidate, PathKind, PeerKey, StreamKind};
use hide_peer::store::{Grant, GrantStatus, Host, KnownState, Login, StoreError, Via};
use hide_peer::words::WordList;
use hide_peer::{PeerEvent, PeerTransport, RelayState};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{OnceCell, Semaphore, watch};
use tokio::time::Instant;

use crate::core::CoreHandle;
use crate::ending::Ending;
use crate::guests::{HelperBack, HelperFolder};
use crate::pairing_ssh;
use crate::pairing_wsl;
use crate::peer_host::{HostPeers, Incoming, Link, PeerHost};
use crate::started_cores::{Failure, Raised};

type Service = Pairings<crate::peer_host::Transport>;

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

/// Puts the address an operator gave first among a host's, and drops the
/// last ones past [`MAX_ADDRESSES`]. A distribution's name is not an
/// address someone gave: it is the only way this PC reaches that Hide, so
/// it is never the one dropped.
fn put_first(candidates: &mut Vec<Candidate>, first: Candidate) {
    candidates.retain(|candidate| *candidate != first);
    candidates.insert(0, first);
    let distribution = |candidate: &Candidate| matches!(candidate, Candidate::Wsl(_));
    let mut room =
        MAX_ADDRESSES.saturating_sub(candidates.iter().filter(|c| distribution(c)).count());
    candidates.retain(|candidate| {
        distribution(candidate) || {
            let kept = room > 0;
            room = room.saturating_sub(1);
            kept
        }
    });
}

/// What carries a carrier that arrives on the bootstrap socket, when it is
/// not a distribution's `wsl.exe`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CarryOver {
    Ssh,
}

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
    /// Asks a machine this one opens, whose Hide is too old to connect,
    /// to update itself (B22).
    Update {
        device: String,
    },
    /// Names an address to reach a host at first.
    Address {
        device: String,
        address: String,
    },
    /// Gives a paired machine the name this machine shows it under: on
    /// the rail, in its tabs and in Settings › Devices. Only this machine's
    /// records change; the machine keeps calling itself what it did.
    Rename {
        device: String,
        name: String,
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
    /// [`Ask::LookSsh`] for a WSL distribution of this PC, asked through
    /// `wsl.exe` (D-27); it may start a distribution that is off, which the
    /// operator's choice of it in Add device allows (B43).
    LookWsl {
        distro: String,
    },
    /// [`Ask::PairSsh`] for a WSL distribution of this PC (security spec
    /// 17): its own Hide saves a pending grant, this PC confirms it over
    /// the connection `wsl.exe` carries, and nothing is installed there.
    PairWsl {
        distro: String,
    },
    /// The stream this ask came on becomes the carrier of a connection to
    /// this machine's core: what `hide device carry` asks, run by the PC
    /// of the distribution this core runs in (D-27, D-62), or with `over`
    /// SSH by a paired machine that signed in to this account
    /// (mission-107), which this daemon then echoes. Answered by
    /// [`Pairing::carrier`], never as a value.
    Carry {
        #[serde(default)]
        over: Option<CarryOver>,
    },
    /// Reads where a device stands on the rule every switch passes
    /// (`pairing_ssh::may_move`), over the login it is reached by, and
    /// changes nothing there: what an opened row shows beside its button.
    /// `device` is a helper device, or a machine a helper device was
    /// switched to, read over the login it was switched through.
    LookSwitch {
        device: String,
    },
    /// Switches the helper device this machine dials as `alias` to the
    /// connection of the Hide on its machine (D-49, B36): that Hide is
    /// paired over the device's SSH login and, once the connection is
    /// confirmed, the device goes on over it.
    Switch {
        device: String,
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
        /// The asking machine gives this one a grant too, and this machine
        /// becomes its guest without a question: only a machine this core
        /// was moved off before 1.2.0, which dials it as a node, is taken
        /// so, since the move was the consent (D-46).
        #[serde(default)]
        grant: Option<String>,
    },
    /// The device helper this account runs for a machine reached over SSH
    /// asks, as it starts, for the folder a switch handed to a machine that
    /// is no longer paired here (D-49): the way back from a switch is to
    /// remove the machine and add this one again over SSH. Answered by
    /// [`Guests::release_helper`].
    ReleaseHelper,
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
            | Self::Update { .. }
            | Self::Address { .. }
            | Self::LookSsh { .. }
            | Self::LookSwitch { .. }
            | Self::PairSsh { .. }
            | Self::LookWsl { .. }
            | Self::PairWsl { .. }
            | Self::Carry { .. }
            | Self::Switch { .. }
            | Self::AcceptSsh { .. }
            | Self::ReleaseHelper => true,
            Self::Status { .. }
            | Self::DismissJoin
            | Self::Pause { .. }
            | Self::Remove { .. }
            | Self::DismissRequest { .. }
            | Self::Rename { .. }
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
        "name_invalid" => "Give a name of one to 64 bytes, on one line, that is not only spaces",
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
            "That machine has no Hide of its own yet: add it over SSH, and once it is connected, hide device switch <device> --install puts Hide there"
        }
        "ssh_update_there" => "Update Hide on that machine, then add it again",
        "ssh_refused_there" => {
            "Hide on that machine refused the request that came over SSH: this machine's diagnostic log names its reason; try again, or add the machine by a link it shares"
        }
        "ssh_hide_not_running" => {
            "Hide is not running on that machine: log in there and open Hide, then add it again"
        }
        "ssh_not_reached" => {
            "ssh did not sign in without asking: check ssh <alias> in a terminal, or add the machine by a link it shares"
        }
        "ssh_busy" => "Other SSH checks are still running; try again in a moment",
        "wsl_not_reached" => {
            "WSL did not run Hide in that distribution: open the distribution once in a terminal, then try again"
        }
        "wsl_unavailable" => "This computer has no WSL to reach a distribution through",
        "ssh_mac_from_mac_only" => {
            "A Mac takes a Hide only from a Mac: install Hide on that Mac and add it by a link it shares"
        }
        "ssh_mac_signed_out" => {
            "Nobody is logged in to that Mac: log in there, then try the switch again"
        }
        "switch_not_connected" => {
            "The two machines were paired and the new connection did not open in time; the device is used as before. Try the switch again"
        }
        "switch_not_moved" => {
            "The two machines were paired and the device did not take the new connection; it is used as before. Try the switch again"
        }
        "switch_cut" => {
            "The login to that machine ended during the switch; the device is used as before. Try the switch again"
        }
        "switch_running" => "This device's switch is already running",
        "ssh_core_not_started" => {
            "Hide could not be started on that machine; the device is used as before. Try the switch again, or open Hide there"
        }
        "switch_unsupported" => {
            "Only a device this Hide dials over SSH and has connected to once, or a WSL distribution of this PC, can be switched"
        }
        "device_own" => "That is this machine itself",
        "update_not_asked" => {
            "Only a machine this one opens, with that direction on, can be asked to update: update Hide on that machine itself"
        }
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

/// What one switch was asked with.
#[derive(Clone, Copy)]
struct SwitchAsked {
    /// Go on without the helper's folder, which another machine took first
    /// (D-75), and so without the terminals the helper ran: only for a
    /// device that lists none of this machine's.
    own_folder: bool,
    /// Start a Hide on a machine that runs none of its own (D-45).
    install: bool,
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
    /// The build this daemon's environment pins (`Env::build`), as its
    /// node link's hello reads it.
    build: Option<String>,
    /// What keeps this machine's core on once it is paired.
    always_on: crate::always_on::Follower,
    me: Me,
    service: OnceCell<Service>,
    frame: watch::Sender<Arc<Value>>,
    /// Counts the asks that want the paired machines read again.
    look: watch::Sender<u64>,
    /// Which machines cannot connect until one side updates, and whether
    /// that side was asked to (B22), until the two connect.
    updates: Mutex<HashMap<PeerKey, PeerUpdate>>,
    /// Where bringing the Hides this machine switched to its build stands.
    raises: Mutex<Raises>,
    /// Held by the one raise that runs: each copies a build to a machine
    /// and restarts its core (B23).
    raising: tokio::sync::Mutex<()>,
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
    switching: Mutex<HashSet<String>>,
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

/// What a switch's steps answer once the read that decided the switch
/// signed in without asking: a login that does not carry a step from then
/// on was cut midway, which the next press carries, never a login that
/// asks for a password, which only that read can say.
fn cut_midway(code: &'static str) -> &'static str {
    match code {
        "ssh_not_reached" | "wsl_not_reached" => "switch_cut",
        code => code,
    }
}

/// The name a distribution's pairing request is kept under beside the SSH
/// aliases': no alias holds a space.
fn wsl_request_key(distro: &str) -> String {
    format!("wsl {distro}")
}

/// The most sessions one add of an alias makes (mission-107, 2.1).
const ADD_SESSIONS: usize = 3;

/// What the next session of an add does.
#[derive(Clone, Copy)]
enum AddNext {
    /// Reads which machine the alias reaches, and pairs it unless it is a
    /// known host.
    Look,
    /// Checks a known host's key over the alias.
    Check(PeerKey),
    /// Pairs, as a new machine.
    Pair,
}

/// What a pairing of an add came to.
enum Paired2 {
    New(Value),
    /// The key answered is a known host's.
    Known(PeerKey),
}

/// What a check of an alias found.
enum Checked {
    Proved,
    OtherKey(PeerKey),
    /// Nothing recorded, for this reason (`"ssh"` of the answer).
    Said(&'static str),
    /// The session did not sign in: the code a pairing answers.
    NotReached(&'static str),
}

impl Checked {
    fn of_session(error: &hide_node::ssh::session::SessionError) -> Self {
        use hide_node::ssh::session::SessionError;
        match error {
            SessionError::Refused(reason) if reason.ends_with("is missing") => {
                Self::Said("hide_absent")
            }
            SessionError::Ssh(_) | SessionError::Lost(_) => Self::NotReached("ssh_not_reached"),
            SessionError::Refused(_) | SessionError::Local(_) => Self::Said("not_proved"),
        }
    }
}

/// One session of an add: its `ssh`, ended when the session is dropped.
struct AddLogin {
    upstream: Arc<hide_node::ssh::upstream::Upstream>,
    reach: pairing_ssh::Reach,
}

impl AddLogin {
    fn new(reach: &pairing_ssh::Reach, alias: &str) -> Self {
        Self {
            upstream: Arc::new(hide_node::ssh::upstream::Upstream::new(
                hide_node::ssh::session::SshTarget::new(&reach.home, alias),
            )),
            reach: reach.clone(),
        }
    }

    /// One blocking step on the session, off the runtime.
    async fn step<T: Send + 'static>(
        &self,
        step: impl FnOnce(&hide_node::ssh::upstream::Upstream, &pairing_ssh::Reach) -> T
        + Send
        + 'static,
    ) -> Result<T, &'static str> {
        let (upstream, reach) = (Arc::clone(&self.upstream), self.reach.clone());
        tokio::task::spawn_blocking(move || step(&upstream, &reach))
            .await
            .map_err(|_| "ssh_not_reached")
    }

    fn close(&self) {
        self.upstream.close();
    }
}

impl Drop for AddLogin {
    fn drop(&mut self) {
        self.close();
    }
}

/// What a far `hide device carry` said as it refused, read from its last
/// line once it wrote one, briefly.
async fn far_said(said: &Arc<Mutex<Vec<u8>>>) -> Option<&'static str> {
    let over = Instant::now() + Duration::from_millis(500);
    loop {
        let line = {
            let said = lock(said);
            String::from_utf8_lossy(&said)
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .map(|line| line.trim().to_owned())
        };
        let reason = line.as_deref().and_then(crate::peer_ssh::far_refusal);
        if reason.is_some() || Instant::now() >= over {
            return reason;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The code a pairing answers for a record that could not be saved.
fn store_code(error: StoreError) -> &'static str {
    match error {
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
    }
}

struct SshRequest {
    alias: String,
    id: String,
    made: Instant,
}

/// What a machine's row says after another refused first exchange, given
/// what it said: which side is behind, and that the side was asked to
/// update when it was asked before and is still the one behind.
fn still_needed(said: Option<PeerUpdate>, behind: Behind) -> PeerUpdate {
    match behind {
        Behind::Theirs if said == Some(PeerUpdate::ThereAsked) => PeerUpdate::ThereAsked,
        Behind::Theirs => PeerUpdate::There,
        Behind::Ours | Behind::Either if said == Some(PeerUpdate::HereAsked) => {
            PeerUpdate::HereAsked
        }
        Behind::Ours | Behind::Either => PeerUpdate::Here,
    }
}

/// The Hides this machine switched on other machines, each brought to this
/// machine's build once per build it is seen to run (B23,
/// `crate::started_cores`).
#[derive(Default)]
struct Raises {
    /// A raise that waits or runs, or how the last one failed: what the
    /// machine's row shows in place of who has to update.
    shown: HashMap<PeerKey, PeerUpdate>,
    /// The build each machine named when its raise was last begun.
    tried: HashMap<PeerKey, String>,
    /// The raise that waits or runs for each machine, by the number it was
    /// begun under: only that raise's end is written.
    running: HashMap<PeerKey, u64>,
    begun: u64,
}

impl Raises {
    /// Begins a raise of `key`'s machine, which names the build `theirs`,
    /// and answers its number; none when one runs for it already, or when
    /// that build was tried and nobody `asked` again.
    fn begin(&mut self, key: PeerKey, theirs: &str, asked: bool) -> Option<u64> {
        if self.running.contains_key(&key)
            || (!asked && self.tried.get(&key).map(String::as_str) == Some(theirs))
        {
            return None;
        }
        self.begun += 1;
        self.running.insert(key, self.begun);
        self.tried.insert(key, theirs.to_owned());
        self.shown.insert(key, PeerUpdate::Updating);
        Some(self.begun)
    }

    /// Writes how raise `attempt` of `key`'s machine ended. A raise that is
    /// no longer that machine's, because the machine was taken off while it
    /// ran, writes nothing and answers false.
    fn finish(&mut self, key: PeerKey, attempt: u64, raised: Result<Raised, Failure>) -> bool {
        if self.running.get(&key) != Some(&attempt) {
            return false;
        }
        self.running.remove(&key);
        match raised {
            Ok(_) => self.shown.remove(&key),
            Err(failure) => self.shown.insert(key, raise_failure(failure)),
        };
        true
    }

    /// Keeps only what belongs to a machine among `paired`: what a raise
    /// left of a machine goes with the machine.
    fn keep(&mut self, paired: &std::collections::HashSet<PeerKey>) {
        self.shown.retain(|key, _| paired.contains(key));
        self.tried.retain(|key, _| paired.contains(key));
        self.running.retain(|key, _| paired.contains(key));
    }
}

/// A failed raise as its row names it.
fn raise_failure(failure: Failure) -> PeerUpdate {
    match failure {
        Failure::NotReached => PeerUpdate::UpdateNotReached,
        Failure::RolledBack => PeerUpdate::UpdateRolledBack,
        Failure::Busy => PeerUpdate::UpdateBusy,
        Failure::Failed => PeerUpdate::UpdateFailed,
    }
}

impl Pairing {
    /// Hands the pairing and revoke streams' handlers to `host`. Nothing
    /// opens: the service starts with the endpoint.
    pub fn new(
        host: Arc<PeerHost>,
        core: Arc<CoreHandle>,
        build: Option<String>,
        always_on: crate::always_on::Follower,
        me: Me,
        ssh: pairing_ssh::Reach,
        shutdown: Ending,
    ) -> Arc<Self> {
        let pairing = Arc::new(Self {
            host: Arc::clone(&host),
            core,
            build,
            always_on,
            me,
            service: OnceCell::new(),
            frame: watch::channel(Arc::new(json!(View::default()))).0,
            look: watch::channel(0).0,
            updates: Mutex::new(HashMap::new()),
            raises: Mutex::default(),
            raising: tokio::sync::Mutex::default(),
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
            Ok((guests, hosts)) => rows(&hosts, &guests, |_| None, &HashMap::new(), |_| None).0,
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
        // What changes a row's SSH route, on a machine that dials over SSH.
        let routes = self.host.ssh().cloned();
        let mut routed = match &routes {
            Some(routes) => routes.changed(),
            None => watch::channel(0).1,
        };
        loop {
            // A host forgotten, or its alias changed or turned off, takes
            // its route with it.
            if let Some(routes) = &routes {
                let kept: Vec<(PeerKey, Option<String>)> = peers
                    .hosts()
                    .iter()
                    .map(|host| (host.key, host.ssh_alias().map(str::to_owned)))
                    .collect();
                routes.hosts(&kept);
            }
            routed.borrow_and_update();
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
                    if told
                        .as_deref()
                        .is_some_and(|before| pairing_finished(before, &machines))
                    {
                        self.core
                            .note_feature_used(hide_core::product_analytics::Feature::HidesPaired);
                    }
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
                changed = routed.changed() => if changed.is_err() { return },
                event = events.recv() => match event {
                    Ok(PeerEvent::UpdateNeeded { key, behind, build, .. }) => {
                        let asked = {
                            let mut updates = lock(&self.updates);
                            let update = still_needed(updates.get(&key).copied(), behind);
                            updates.insert(key, update);
                            update == PeerUpdate::ThereAsked
                        };
                        // An ask is said once per exchange: until the two
                        // connect, each dial of the machine says it again,
                        // so a Hide restarted there still hears it. A
                        // machine this one no longer opens is asked nothing.
                        if asked && peers.ask_update(&key).is_err() {
                            lock(&self.updates).insert(key, PeerUpdate::There);
                        }
                        // A Hide this machine switched is brought to its
                        // build instead of waiting for someone there.
                        if behind != Behind::Ours {
                            self.raise_started(&peers, key, &build, false);
                        }
                    }
                    // The app here is told through the machine's row: its
                    // own window updates and restarts (B22).
                    // Only where this machine is the one behind: an ask on
                    // a connection that lives has nothing to update for.
                    Ok(PeerEvent::UpdateRequested { key }) => {
                        let mut updates = lock(&self.updates);
                        if updates.get(&key) == Some(&PeerUpdate::Here) {
                            updates.insert(key, PeerUpdate::HereAsked);
                        }
                    }
                    Ok(PeerEvent::Connected { key, .. }) => {
                        lock(&self.updates).remove(&key);
                        if let Some(link) = peers.link(&key) {
                            let build = link.their_hello().build.clone();
                            // A route parked for that machine's update is
                            // tried again once it names another build.
                            if let Some(routes) = &routes {
                                routes.heard_build(&key, &build);
                            }
                            self.raise_started(&peers, key, &build, false);
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
            }
        }
    }

    /// Brings the Hide this machine reaches through a login on `key`'s
    /// machine to this build, when that Hide names an older one (`theirs`):
    /// once per build it names, and again when the operator asks (`asked`).
    /// A machine this one never switched through a login, and one that runs
    /// this build or a newer release, are left alone, and so is one that
    /// answers that its app updates it or that nothing would start its core
    /// again: its row shows nothing of the raise. One raise runs at a time.
    fn raise_started(self: &Arc<Self>, peers: &HostPeers, key: PeerKey, theirs: &str, asked: bool) {
        let Some(login) = peers.host(&key).and_then(|host| host.started) else {
            return;
        };
        let ours = crate::build_order::Release::of_this_build().version;
        let attempt = {
            let mut raises = lock(&self.raises);
            if !crate::build_order::may_be_older(&ours, theirs) {
                // It caught up, by this machine's raise or another's.
                if raises.shown.remove(&key).is_some() {
                    self.look.send_modify(|count| *count += 1);
                }
                return;
            }
            let Some(attempt) = raises.begin(key, theirs, asked) else {
                return;
            };
            attempt
        };
        self.look.send_modify(|count| *count += 1);
        let (pairing, peers) = (Arc::clone(self), peers.clone());
        tokio::spawn(async move {
            let raised = pairing.raise(login).await;
            let written = lock(&pairing.raises).finish(key, attempt, raised);
            if raised == Ok(Raised::Updated) {
                // The core there restarted: its connection is made again
                // now, not at the next redial.
                peers.network_changed();
            }
            if written {
                pairing.look.send_modify(|count| *count += 1);
            }
        });
    }

    /// One raise through `login`, after the ones begun before it.
    async fn raise(&self, login: Login) -> Result<Raised, Failure> {
        let _one = tokio::select! {
            () = self.shutdown.ended() => return Err(Failure::Failed),
            one = self.raising.lock() => one,
        };
        let dial = Arc::clone(&self.ssh_dials)
            .acquire_owned()
            .await
            .map_err(|_| Failure::Failed)?;
        let (reach, wsl) = (self.ssh.clone(), self.host.wsl().cloned());
        tokio::task::spawn_blocking(move || {
            let _dial = dial;
            // This build's programs for every system it carries, beside
            // this one.
            let packages = hide_node::ssh::host::HelperPackages::new(
                std::env::current_exe()
                    .ok()
                    .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)),
                crate::cli::RELEASE,
            );
            match &login {
                Login::Ssh(alias) => pairing_ssh::raise(&reach, alias, &packages),
                Login::Wsl(distro) => match &wsl {
                    Some(carriers) => pairing_wsl::raise(carriers.access(), distro, &packages),
                    None => Err(Failure::NotReached),
                },
            }
        })
        .await
        .unwrap_or(Err(Failure::Failed))
    }

    /// Runs once when the pairing starts and then whenever the machines this
    /// one is paired with changed: a machine paired, confirmed, paused,
    /// resumed or removed in either direction, connected or lost. Whatever
    /// follows "is this machine paired with anything" belongs here: the
    /// core's list (D-72) and the login item that keeps a paired core on
    /// (D-41).
    async fn paired_changed(&self, machines: &[PeerMachine]) {
        self.always_on.peers_changed();
        // What a raise left of a machine goes with the machine.
        {
            let paired: std::collections::HashSet<PeerKey> = machines
                .iter()
                .filter_map(|machine| PeerKey::from_hex(&machine.key).ok())
                .collect();
            lock(&self.raises).keep(&paired);
        }
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
        let mut updates = lock(&self.updates).clone();
        // A raise under way or failed is what its row says: this machine
        // is who updates that Hide.
        updates.extend(lock(&self.raises).shown.clone());
        let routes = self.host.ssh();
        let (mut paired, away) = rows(
            &hosts,
            &guests,
            |key| peers.link(key),
            &updates,
            |key| routes.and_then(|routes| routes.blocked(key)),
        );
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
    /// its code and what to do about it.
    pub async fn ask(self: &Arc<Self>, ask: Ask, caller: Caller) -> Value {
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
        // Adding an alias a paired machine is reached by answers its outcome
        // beside a refusal (mission-107, 2.1).
        let answered = match ask {
            Ask::PairSsh { alias } => self.pair_ssh(&alias).await,
            ask => self.run(ask, caller).await.map_err(|code| (code, None)),
        };
        match answered {
            Ok(mut answer) => {
                answer["ok"] = json!(true);
                answer
            }
            Err((code, ssh)) => {
                let mut answer = refused(code);
                if let Some(ssh) = ssh {
                    answer["ssh"] = json!(ssh);
                }
                answer
            }
        }
    }

    /// What this core says of itself to a machine that reads it before it
    /// asks anything (`hide device look-ssh`): its node, and its build and
    /// release as its node link's hello names them (`attach::CoreHello`).
    async fn itself(&self) -> Value {
        let pinned = self.build.clone();
        // The hash of the executable, read once: never on a runtime thread.
        let build =
            tokio::task::spawn_blocking(move || crate::build_id::of_daemon(pinned.as_deref()))
                .await
                .ok()
                .and_then(Result::ok);
        json!({
            "node": self.core.node().as_str(),
            "build": build,
            "release": crate::build_order::Release::of_this_build(),
        })
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
            let core = self.itself().await;
            let Some(service) = self.service.get() else {
                let machines = answers(self.recorded().await);
                return Ok(json!({
                    "pairing": View::default(), "machines": machines, "core": core,
                }));
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
            return Ok(json!({ "pairing": pairing, "machines": machines, "core": core }));
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
        if let Ask::LookSwitch { device } = &ask {
            let (device, via, node) = self.read_through(device).await?;
            let moves = self
                .may_move(&via, node.as_deref(), pairing_ssh::Asked::Look)
                .await?;
            return Ok(json!({
                "device_id": device,
                "condition": moves.condition.payload(),
                "user": moves.user,
            }));
        }
        if let Ask::ReleaseHelper = ask {
            return self.release_helper().await;
        }
        if let Ask::LookWsl { distro } = &ask {
            let (carriers, reach) = (self.wsl()?, self.ssh.clone());
            let target = distro.clone();
            let looked = self
                .wsl_call(distro, move || {
                    pairing_wsl::look(carriers.access(), &reach, &target)
                })
                .await?;
            return Ok(json!({ "distro": distro, "looked": looked }));
        }
        // Sharing this machine, joining another and either side of an SSH
        // pairing open the endpoint (D-83). Nothing else does: with no
        // endpoint there is no machine to act on and no ask to answer.
        let service = match &ask {
            Ask::Share { .. }
            | Ask::Join { .. }
            | Ask::PairWsl { .. }
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
            Ask::Status { .. }
            | Ask::LookSsh { .. }
            | Ask::LookWsl { .. }
            | Ask::LookSwitch { .. }
            | Ask::ReleaseHelper => {
                unreachable!("answered above")
            }
            // A carrier is a stream, which only the bootstrap socket hands
            // over (`Pairing::carrier`); asked as a value it is no request.
            Ask::Carry { .. } => Err("invalid_request"),
            Ask::PairWsl { distro } => {
                let carriers = self.wsl()?;
                let requested = wsl_request_key(&distro);
                let request = self.ssh_request(&requested)?;
                let (reach, me, key) = (self.ssh.clone(), self.me.clone(), peers.key());
                let target = distro.clone();
                let asked = Arc::clone(&carriers);
                let paired = self
                    .wsl_call(&distro, move || {
                        pairing_wsl::accept(asked.access(), &reach, &target, &request, &me, &key)
                    })
                    .await??;
                let answer = json!({
                    "device_id": paired.accepted.device,
                    "name": paired.accepted.name,
                });
                // The operator pressed: the dial that confirms the pairing
                // may start the distribution again if it went off meanwhile.
                carriers.may_wake(&distro);
                // A distribution is reached through `wsl.exe` alone.
                service.adopt(paired.accepted, paired.first, None).map_err(
                    |error| match error {
                        AcceptError::Invalid => "device_own",
                        AcceptError::Already => "device_already_connected",
                        AcceptError::Store(error) => store(error),
                    },
                )?;
                lock(&self.ssh_requests).retain(|kept| kept.alias != requested);
                Ok(answer)
            }
            Ask::PairSsh { .. } => unreachable!("answered by `pair_ssh`"),
            Ask::Switch { device, install } => {
                // The device as the core knows it: a terminal may name it
                // by its alias.
                let (core, named) = (Arc::clone(&self.core), device);
                let source = tokio::task::spawn_blocking(move || core.device_switch_source(&named))
                    .await
                    .map_err(|_| "hide_unavailable")??;
                let device = source.device_id.clone();
                // One switch of a device at a time: a second ask while
                // the first runs changes nothing.
                if !lock(&self.switching).insert(device.clone()) {
                    return Err("switch_running");
                }
                let answer = self.switch_asked(&service, &peers, &source, install).await;
                // The one who asked reads which device its answer is for.
                let answer = answer.map(|mut answer| {
                    answer["device_id"] = json!(device);
                    answer
                });
                // The switch is over, however it ended: the core draws no
                // step of it any more, and a device that was not switched
                // is left as it was.
                self.switch_step(&device, "ended", json!({})).await;
                lock(&self.switching).remove(&device);
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
                // A route parked for the operator is tried again too.
                if let Some(routes) = self.host.ssh() {
                    routes.retry(&paired.key);
                }
                lock(&self.relay).answer = None;
                peers.network_changed();
                // A raise that failed is the operator's to ask again.
                let failed = {
                    let raises = lock(&self.raises);
                    raises
                        .shown
                        .get(&paired.key)
                        .filter(|shown| **shown != PeerUpdate::Updating)
                        .and_then(|_| raises.tried.get(&paired.key).cloned())
                };
                if let Some(theirs) = failed {
                    self.raise_started(&peers, paired.key, &theirs, true);
                }
                self.look.send_modify(|count| *count += 1);
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::Update { device } => {
                let paired = self.named(&peers, &device)?;
                // Asked only of a machine whose Hide is the older one and
                // that this machine opens: the ask rides the first
                // exchange of the dial made now, the one thing the two
                // builds share.
                let older = matches!(
                    lock(&self.updates).get(&paired.key),
                    Some(PeerUpdate::There | PeerUpdate::ThereAsked)
                );
                if !older || peers.ask_update(&paired.key).is_err() {
                    return Err("update_not_asked");
                }
                lock(&self.updates).insert(paired.key, PeerUpdate::ThereAsked);
                peers.network_changed();
                self.look.send_modify(|count| *count += 1);
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::Address { device, address } => {
                let paired = self.named(&peers, &device)?;
                let address: SocketAddr = address.trim().parse().map_err(|_| "address_invalid")?;
                let saved = peers.known(|known| {
                    let mut host = known.of(&paired.key).cloned().ok_or(StoreError::Unknown)?;
                    put_first(&mut host.candidates, Candidate::Ip(address));
                    known.put(host)
                });
                saved.map_err(store)?;
                peers.network_changed();
                Ok(json!({ "device_id": paired.machine.device_id }))
            }
            Ask::Rename { device, name } => {
                let chosen = chosen_name(&name)?;
                let paired = self.named(&peers, &device)?;
                if !peers.rename(&paired.key, chosen).map_err(store)? {
                    return Err("device_unknown");
                }
                self.look.send_modify(|count| *count += 1);
                let shown = self
                    .paired(&peers)
                    .into_iter()
                    .find(|now| now.key == paired.key)
                    .map_or(paired.machine.name, |now| now.machine.name);
                Ok(json!({ "device_id": paired.machine.device_id, "name": shown }))
            }
            Ask::AcceptSsh {
                request,
                key,
                device,
                name,
                wsl,
                helper,
                grant,
            } => {
                let guest = PeerKey::from_hex(&key).map_err(|_| "invalid_request")?;
                // A machine that gives its grant in the same breath is
                // taken as a host only when this core already runs its
                // work: it dials this core as a node, or was taken before.
                if grant.is_some() {
                    let state = self
                        .core
                        .state_dir()
                        .join(hide_core::node_migration::CORE_STATE);
                    let dials =
                        tokio::task::spawn_blocking(move || hide_core::dialing_nodes(&state))
                            .await
                            .map_err(|_| "hide_unavailable")?
                            .map_err(|_| "hide_unavailable")?
                            .contains(&device);
                    let taken = self
                        .paired(&peers)
                        .iter()
                        .any(|paired| paired.key == guest && paired.machine.opens.is_some());
                    if !dials && !taken {
                        return Err("device_unknown");
                    }
                }
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
                    Ok(accepted) => Some(accepted),
                    // A machine that is paired already asks for the folder
                    // alone.
                    Err(AcceptError::Already) if helper.is_some() => None,
                    Err(AcceptError::Invalid) => return Err("invalid_request"),
                    Err(AcceptError::Already) => return Err("device_already_connected"),
                    Err(AcceptError::Store(error)) => return Err(store(error)),
                };
                if let Some(grant) = grant {
                    // The move was the consent both ways: this machine's
                    // grant for the asker is settled here, not at a confirm
                    // over a connection the two may not make before a
                    // pending grant runs out.
                    let given = accepted.as_ref().ok_or("device_already_connected")?;
                    peers.confirm(&guest, &given.grant).map_err(store)?;
                    let host = Accepted {
                        key: guest,
                        device: device.clone(),
                        name: name.clone(),
                        grant,
                        candidates: Vec::new(),
                    };
                    match service.adopt(host, Vec::new(), None) {
                        Ok(()) => {}
                        Err(AcceptError::Invalid) => return Err("invalid_request"),
                        Err(AcceptError::Already) => return Err("device_already_connected"),
                        Err(AcceptError::Store(error)) => return Err(store(error)),
                    }
                    return Ok(json!({ "accepted": accepted, "adopted": true }));
                }
                let accepted = accepted.map_or(Value::Null, |accepted| json!(accepted));
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

    /// One switch the operator asked for (operator decision 2026-11-24:
    /// a device is switched only when they ask). The machine is read first,
    /// and one the rule refuses answers `left` with where it stands, for
    /// the one line of whoever asked; this is the read that asks a Linux
    /// account that does not linger to. A device another computer switched
    /// first (D-75) goes on with a folder of this machine's own, which
    /// takes up none of the terminals it runs there, so only when it lists
    /// none: one that lists any, or was not read, answers `held` with that
    /// computer's name and the count. No switch closes a terminal.
    async fn switch_asked(
        self: &Arc<Self>,
        service: &Service,
        peers: &HostPeers,
        source: &hide_core::DeviceSwitchSource,
        install: bool,
    ) -> Result<Value, &'static str> {
        // The operator asked: a distribution that is off may be started
        // for the read, the switch and the dial that follows it (B43).
        if let hide_core::SwitchVia::Wsl { distro } = &source.via {
            self.wsl()?.may_wake(distro);
        }
        let device = source.device_id.as_str();
        self.switch_step(device, "checking", json!({})).await;
        let moves = self
            .may_move(
                &source.via,
                source.node_program.as_deref(),
                pairing_ssh::Asked::Switch,
            )
            .await?;
        let condition = moves.condition.payload();
        if !moves.condition.may() {
            return Ok(json!({ "switch": "left", "condition": condition, "user": moves.user }));
        }
        let asked = |own_folder| SwitchAsked {
            own_folder,
            install,
        };
        let mut answer = self
            .switch(service, peers, source, asked(false))
            .await
            .map_err(cut_midway)?;
        if answer["switch"] == "held" {
            let (core, listed) = (Arc::clone(&self.core), device.to_owned());
            let open = tokio::task::spawn_blocking(move || core.device_terminals(&listed))
                .await
                .map_err(|_| "hide_unavailable")?;
            if open != Some(0) {
                // How many this computer runs there, when the core read
                // them: the ones to close before the switch is asked again.
                answer["terminals"] = json!(open);
                return Ok(answer);
            }
            answer = self
                .switch(service, peers, source, asked(true))
                .await
                .map_err(cut_midway)?;
        }
        // What the one who asked is told of the machine it switched: a
        // Mac that does not log in by itself is switched with that said.
        answer["condition"] = condition;
        answer["user"] = json!(moves.user);
        Ok(answer)
    }

    /// The steps of [`Ask::Switch`]. Whatever stops it before the
    /// connection is confirmed leaves the device a helper device: the core
    /// changes its link only when the paired machine's grant is active.
    async fn switch(
        self: &Arc<Self>,
        service: &Service,
        peers: &HostPeers,
        source: &hide_core::DeviceSwitchSource,
        how: SwitchAsked,
    ) -> Result<Value, &'static str> {
        let SwitchAsked {
            own_folder,
            install,
        } = how;
        let device = source.device_id.as_str();
        // The machine keeps the name its device had here, not the one it
        // calls itself: the operator switched the device they know.
        let label = hide_peer::link::clean_name(&source.label);
        // The request a switch runs under is its channel's own, so one
        // that stopped midway is asked again as the same request.
        let requested = match &source.via {
            hide_core::SwitchVia::Ssh { alias, .. } => alias.clone(),
            hide_core::SwitchVia::Wsl { distro } => wsl_request_key(distro),
        };
        let request = self.ssh_request(&requested)?;
        let reach = self.ssh.running(source.node_program.as_deref());
        let (me, key) = (self.me.clone(), peers.key());
        let helper_root = source.helper_root.clone();
        let projects = source.projects.clone();
        let runtime = tokio::runtime::Handle::current();
        // The login the machine is reached through, which its record keeps
        // for bringing its Hide to this machine's build (B23).
        let through = match &source.via {
            hide_core::SwitchVia::Ssh { alias, .. } => Login::Ssh(alias.clone()),
            hide_core::SwitchVia::Wsl { distro } => Login::Wsl(distro.clone()),
        };
        // What follows the target's answer runs on the thread that holds
        // the login, before it ends: the machine's own Hide is handed the
        // device's projects only once it admitted this machine.
        let landed: Arc<Mutex<Option<Result<Value, &'static str>>>> = Arc::default();
        // Each step is told to the core as it begins, from that thread.
        let step = {
            let (this, device, runtime) = (Arc::clone(self), device.to_owned(), runtime.clone());
            move |step: &'static str| runtime.block_on(this.switch_step(&device, step, json!({})))
        };
        let admitted = {
            let (this, service, peers) = (Arc::clone(self), service.clone(), peers.clone());
            let (device, requested, landed) =
                (device.to_owned(), requested.clone(), Arc::clone(&landed));
            let label = label.clone();
            move |switched: &pairing_ssh::Switched| {
                let done = runtime.block_on(this.admit_switch(
                    &service,
                    &peers,
                    &device,
                    label.as_deref(),
                    &through,
                    &requested,
                    own_folder,
                    switched,
                ));
                let active = done.is_ok();
                *lock(&landed) = Some(done);
                active
            }
        };
        // What one switch asks, whichever channel carries it, from the
        // thread its login runs on.
        let asked = {
            move |run: &dyn Fn(
                &pairing_ssh::Switch<'_>,
            ) -> Result<pairing_ssh::Switched, &'static str>| {
                run(&pairing_ssh::Switch {
                    reach: &reach,
                    request: &request,
                    me: &me,
                    key: &key,
                    helper_root: (!own_folder).then_some(helper_root.as_str()),
                    projects: &projects,
                    admitted: &admitted,
                    install,
                    step: &step,
                })
            }
        };
        let switched = match &source.via {
            hide_core::SwitchVia::Ssh { alias } => {
                let target = alias.clone();
                self.ssh_dial(alias, move || {
                    asked(&|switch| pairing_ssh::switch(&target, switch))
                })
                .await??
            }
            hide_core::SwitchVia::Wsl { distro } => {
                let (carriers, target) = (self.wsl()?, distro.clone());
                self.wsl_call(distro, move || {
                    asked(&|switch| pairing_wsl::switch(carriers.access(), &target, switch))
                })
                .await??
            }
        };
        match switched {
            // That computer's name came over a wire and is printed.
            pairing_ssh::Switched::HeldBy(by) => Ok(json!({
                "switch": "held",
                "by": hide_peer::link::clean_name(&by),
            })),
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
    /// active. `label` is the device's name here, which the pairing keeps;
    /// `through` is the login the machine was reached through, and
    /// `requested` the key its request is kept under.
    #[allow(clippy::too_many_arguments)]
    async fn admit_switch(
        self: &Arc<Self>,
        service: &Service,
        peers: &HostPeers,
        device: &str,
        label: Option<&str>,
        through: &Login,
        requested: &str,
        own_folder: bool,
        switched: &pairing_ssh::Switched,
    ) -> Result<Value, &'static str> {
        let pairing_ssh::Switched::Paired { paired, folder } = switched else {
            return Err("switch_not_connected");
        };
        // The machine the device is: the one that just answered, or the
        // one this machine is paired with already, under the device's id
        // or, for a distribution, by the name this PC reaches it by (its
        // own Hide names itself by an id of its own).
        let known_as = |paired: &Paired| match through {
            Login::Ssh(_) => paired.machine.device_id == device,
            Login::Wsl(distro) => peers
                .host(&paired.key)
                .is_some_and(|host| host.candidates.contains(&Candidate::Wsl(distro.clone()))),
        };
        let host = match paired {
            Some(paired) => paired.accepted.key,
            None => self
                .paired(peers)
                .into_iter()
                .find(|paired| known_as(paired) && paired.machine.opens.is_some())
                .map(|paired| paired.key)
                .ok_or("device_unknown")?,
        };
        let connecting = json!({ "key": host.to_hex(), "own_folder": own_folder });
        self.switch_step(device, "connecting", connecting).await;
        if let Some(paired) = paired.clone() {
            // A switched host's route is the login it was switched through
            // (`Host::started`), read as its alias.
            match service.adopt(paired.accepted, paired.first, None) {
                // Adopted before, by a switch that stopped after it.
                Ok(()) | Err(AcceptError::Already) => {}
                Err(AcceptError::Invalid) => return Err("device_own"),
                Err(AcceptError::Store(StoreError::Full)) => return Err("device_limit"),
                Err(AcceptError::Store(_)) => return Err("device_not_saved"),
            }
            lock(&self.ssh_requests).retain(|kept| kept.alias != requested);
        }
        // A Hide reached through a login is this machine's to keep at its
        // build through that login (B23), whichever switch started it: the
        // record is the machine's, not this run's, and an account that has
        // the app answers a raise that its app updates it. The pairing
        // stands without the record, so a store that refuses it is only
        // logged.
        // The machine is shown under the name its device had here, not the
        // one it calls itself: the operator switched the device they know
        // (B36). It is their chosen name, which the machine's own does not
        // replace.
        let saved = peers.known(|known| {
            let mut record = known.of(&host).cloned().ok_or(StoreError::Unknown)?;
            if !started_through(&mut record, through, label) {
                return Ok(());
            }
            known.put(record)
        });
        if let Err(error) = saved {
            hide_core::diagnostic!(json!({
                "component": "pairing",
                "kind": "pairing.started_not_saved",
                "reason": format!("{error:?}"),
            }));
        }
        // Done is the device holding that machine's connection, which
        // the core gives it once the grant is active and may refuse (a
        // terminal opened there meanwhile, the save failed): a pairing
        // alone is no switched device, and its machine is handed nothing.
        let over = Instant::now() + SWITCH_CONNECTS_WITHIN;
        loop {
            let (core, named, key) = (Arc::clone(&self.core), device.to_owned(), host.to_hex());
            let landed =
                tokio::task::spawn_blocking(move || core.device_switch_landed(&named, &key))
                    .await
                    .map_err(|_| "hide_unavailable")?;
            match landed {
                Some(true) => return Ok(json!({ "switch": "done", "folder": folder })),
                Some(false) => return Err("switch_not_moved"),
                None => {}
            }
            if Instant::now() >= over || self.shutdown.has_ended() {
                return Err("switch_not_connected");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Where the machine reached through `via` stands on the rule every
    /// switch passes (`pairing_ssh::may_move`), read over a login of its
    /// own that asks nobody, with the `hide` beside its device's node,
    /// `node`, when one runs there ([`pairing_ssh::Reach::running`]).
    async fn may_move(
        &self,
        via: &hide_core::SwitchVia,
        node: Option<&str>,
        asked: pairing_ssh::Asked,
    ) -> Result<pairing_ssh::Move, &'static str> {
        let reach = self.ssh.running(node);
        match via {
            hide_core::SwitchVia::Ssh { alias, .. } => {
                let target = alias.clone();
                self.ssh_dial(alias, move || {
                    pairing_ssh::may_move_device(&reach, &target, asked)
                })
                .await?
            }
            hide_core::SwitchVia::Wsl { distro } => {
                let (carriers, target) = (self.wsl()?, distro.clone());
                self.wsl_call(distro, move || {
                    pairing_wsl::may_move(carriers.access(), &reach, &target, asked)
                })
                .await?
            }
        }
    }

    /// The login `device` is read through, its id, and the program its
    /// node runs there: a helper device's own, as the core knows them, or,
    /// for a machine a helper device was switched to, the login it was
    /// switched through, which its record keeps, with no node of this
    /// computer's there. A machine paired any other way has none.
    async fn read_through(
        &self,
        device: &str,
    ) -> Result<(String, hide_core::SwitchVia, Option<String>), &'static str> {
        let (core, named) = (Arc::clone(&self.core), device.to_owned());
        let source = tokio::task::spawn_blocking(move || core.device_switch_source(&named))
            .await
            .map_err(|_| "hide_unavailable")?;
        match source {
            Ok(source) => Ok((source.device_id, source.via, source.node_program)),
            Err(refusal) => {
                // Read from the records, so a machine is read whether or
                // not the endpoint is open.
                let (_, hosts) = self.host.recorded().await.map_err(|_| refusal)?;
                let started = hosts
                    .into_iter()
                    .find(|host| host.device == device)
                    .and_then(|host| host.started)
                    .ok_or(refusal)?;
                let via = match started {
                    Login::Ssh(alias) => hide_core::SwitchVia::Ssh { alias },
                    Login::Wsl(distro) => hide_core::SwitchVia::Wsl { distro },
                };
                Ok((device.to_owned(), via, None))
            }
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

    /// [`Ask::ReleaseHelper`]: gives the device helper's folder back from a
    /// machine that holds no grant here any more, or names the paired
    /// machine that still holds it. It opens no endpoint: the machine that
    /// took the folder may have been this one's last pairing.
    async fn release_helper(self: &Arc<Self>) -> Result<Value, &'static str> {
        let Some(guests) = self.guests.get().cloned() else {
            return Ok(json!({ "helper": "free" }));
        };
        let _one = self.helper_asks.lock().await;
        // A record that cannot be read gives nothing back: the machine it
        // names may still be paired.
        let paired = match self.host.peers() {
            Some(peers) => self.paired(&peers),
            None => {
                let (guests, hosts) = self.host.recorded().await.map_err(|_| "hide_unavailable")?;
                rows(&hosts, &guests, |_| None, &HashMap::new(), |_| None).0
            }
        };
        let granted: Vec<PeerKey> = paired
            .iter()
            .filter(|paired| paired.machine.opened_by.is_some())
            .map(|paired| paired.key)
            .collect();
        let back =
            tokio::task::spawn_blocking(move || guests.release_helper(|key| granted.contains(key)))
                .await
                .map_err(|_| "hide_unavailable")?;
        Ok(match back {
            HelperBack::Free => json!({ "helper": "free" }),
            HelperBack::Released => json!({ "helper": "released" }),
            HelperBack::HeldBy(holder) => {
                let by = paired
                    .into_iter()
                    .find(|paired| paired.key == holder)
                    .map(|paired| paired.machine.name)
                    .unwrap_or_default();
                json!({ "helper": "held", "by": by })
            }
        })
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

    /// Adds the machine `alias` reaches over SSH (B7): its own Hide pairs
    /// over the login, which installs nothing there, and the alias is kept
    /// as the route the connection takes when no direct path opens
    /// (mission-107). A machine already paired gets the alias only once a
    /// carrier over it proved that machine's key (2.1): the device id the
    /// look answers and the key the accept answers only choose which key is
    /// checked. At most three sessions, one after another, on one of the
    /// operator's places, each check bounded by `limits::CHECK_WITHIN`.
    /// A refusal is its code, with what became of the alias when the
    /// machine was paired already.
    async fn pair_ssh(
        self: &Arc<Self>,
        alias: &str,
    ) -> Result<Value, (&'static str, Option<&'static str>)> {
        if !pairing_ssh::valid_alias(alias) {
            return Err(("invalid_request", None));
        }
        let service = self.service().await.map_err(|code| (code, None))?;
        let peers = self.host.peers().ok_or(("peer_unavailable", None))?;
        // One place of the operator's for the whole add, held across its
        // blocking steps and its checks, and given back however it ends.
        let _place = Arc::clone(&self.ssh_dials)
            .try_acquire_owned()
            .map_err(|_| ("ssh_busy", None))?;
        let already = |ssh: &'static str| Err(("device_already_connected", Some(ssh)));
        let mut next = AddNext::Look;
        for _ in 0..ADD_SESSIONS {
            let login = AddLogin::new(&self.ssh, alias);
            let key = match next {
                AddNext::Check(key) => key,
                AddNext::Look => {
                    let looked = login
                        .step(pairing_ssh::core_on)
                        .await
                        .map_err(|code| (code, None))?;
                    let node = looked.ok().flatten().map(|core| core.node);
                    let known = node.and_then(|node| {
                        peers
                            .hosts()
                            .into_iter()
                            .find(|host| host.device == node)
                            .map(|host| host.key)
                    });
                    match known {
                        Some(key) => key,
                        None => match self.pair_on(&service, &peers, &login, alias).await? {
                            Paired2::New(answer) => return Ok(answer),
                            Paired2::Known(key) => key,
                        },
                    }
                }
                AddNext::Pair => match self.pair_on(&service, &peers, &login, alias).await? {
                    Paired2::New(answer) => return Ok(answer),
                    Paired2::Known(key) => key,
                },
            };
            match self.check_alias(&peers, &login, key).await {
                Checked::Proved => {
                    let recorded = peers.known(|known| known.record_ssh(&key, alias));
                    recorded.map_err(|error| (store_code(error), None))?;
                    self.look.send_modify(|count| *count += 1);
                    return already("recorded");
                }
                // Another known key: checked against itself next; nothing is
                // recorded from a check of another key.
                Checked::OtherKey(other) if peers.host(&other).is_some() => {
                    next = AddNext::Check(other);
                }
                // A machine set up again under a new key pairs as a new
                // machine; its earlier record goes as one device is one host.
                Checked::OtherKey(_) => next = AddNext::Pair,
                Checked::Said(ssh) => return already(ssh),
                Checked::NotReached(code) => return Err((code, None)),
            }
        }
        already("not_proved")
    }

    /// Today's SSH pairing on `login`'s session: `accept-ssh`, and the
    /// answer adopted with the alias kept as the host's route. A host
    /// already known under the key it answered with is that key, to check.
    async fn pair_on(
        &self,
        service: &Service,
        peers: &HostPeers,
        login: &AddLogin,
        alias: &str,
    ) -> Result<Paired2, (&'static str, Option<&'static str>)> {
        let request = self.ssh_request(alias).map_err(|code| (code, None))?;
        let (me, key) = (self.me.clone(), peers.key());
        let paired = login
            .step(move |upstream, reach| {
                pairing_ssh::pair_on(&pairing_ssh::Ssh(upstream), reach, &request, &me, &key)
            })
            .await
            .map_err(|code| (code, None))?
            .map_err(|code| (code, None))?;
        let answer = json!({
            "device_id": paired.accepted.device,
            "name": paired.accepted.name,
        });
        let known = paired.accepted.key;
        match service.adopt(paired.accepted, paired.first, Some(alias.to_owned())) {
            Ok(()) => {
                // Adopted: a later pairing of this alias is a new request.
                lock(&self.ssh_requests).retain(|kept| kept.alias != alias);
                Ok(Paired2::New(answer))
            }
            Err(AcceptError::Already) => Ok(Paired2::Known(known)),
            Err(AcceptError::Invalid) => Err(("device_own", None)),
            Err(AcceptError::Store(error)) => Err((store_code(error), None)),
        }
    }

    /// Runs `hide device carry --ssh` on `login`'s session and the key
    /// proof against `key` on it, within `limits::CHECK_WITHIN`: nothing is
    /// admitted, no route hears it, and the session's `ssh` ends however it
    /// ends.
    async fn check_alias(&self, peers: &HostPeers, login: &AddLogin, key: PeerKey) -> Checked {
        let checking = async {
            let channel = login
                .step(|upstream, reach| {
                    upstream.carry(|home, system| pairing_ssh::carry_argv(reach, home, system))
                })
                .await;
            let channel = match channel {
                Err(code) => return Checked::NotReached(code),
                Ok(Err(error)) => return Checked::of_session(&error),
                Ok(Ok(channel)) => channel,
            };
            let said = Arc::clone(&channel.stderr);
            let carrier = hide_peer::mux::Carrier::from(channel.stream);
            let pipe = peers.transport().pipe();
            match pipe
                .check(carrier, &key, hide_peer::limits::CHECK_WITHIN)
                .await
            {
                Ok(Proof::Proved) => Checked::Proved,
                Ok(Proof::OtherKey(other)) => Checked::OtherKey(other),
                Ok(Proof::NotProved) => Checked::Said("not_proved"),
                Err(_) => Checked::Said(far_said(&said).await.unwrap_or("not_proved")),
            }
        };
        match tokio::time::timeout(hide_peer::limits::CHECK_WITHIN, checking).await {
            Ok(checked) => checked,
            Err(_) => {
                login.close();
                Checked::Said("check_timed_out")
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
        self.login_step(step, "ssh_not_reached").await
    }

    /// Runs one `wsl.exe` step for `distro` off the runtime, counted with
    /// the `ssh` steps; the step's `wsl.exe` ends with it.
    async fn wsl_call<T: Send + 'static>(
        &self,
        distro: &str,
        step: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, &'static str> {
        if !hide_platform::wsl::valid_distribution_name(distro) {
            return Err("invalid_request");
        }
        self.login_step(step, "wsl_not_reached").await
    }

    /// One step that runs a login's program, off the runtime; an ask past
    /// [`SSH_DIALS`] of them is refused, not queued.
    async fn login_step<T: Send + 'static>(
        &self,
        step: impl FnOnce() -> T + Send + 'static,
        lost: &'static str,
    ) -> Result<T, &'static str> {
        let dial = Arc::clone(&self.ssh_dials)
            .try_acquire_owned()
            .map_err(|_| "ssh_busy")?;
        tokio::task::spawn_blocking(move || {
            let _dial = dial;
            step()
        })
        .await
        .map_err(|_| lost)
    }

    /// What reaches this PC's WSL distributions; refused on a machine that
    /// reaches none.
    fn wsl(&self) -> Result<Arc<pairing_wsl::Carriers>, &'static str> {
        self.host.wsl().cloned().ok_or("wsl_unavailable")
    }

    /// The endpoint a carrier that arrived on the bootstrap socket is
    /// handed to (`Ask::Carry`): opened for it, as for either side of a
    /// pairing, and only for the operator's own account outside a pane.
    pub async fn carrier(self: &Arc<Self>, caller: Caller) -> Result<HostPeers, &'static str> {
        if caller != Caller::Operator {
            return Err("device_from_pane");
        }
        self.service().await?;
        self.host.peers().ok_or("peer_unavailable")
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
            Some(Ask::Status { .. } | Ask::AcceptSsh { .. } | Ask::ReleaseHelper) | None => {
                refused("invalid_request")
            }
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
/// The name an operator's rename keeps for a machine: none for an empty
/// one, which takes the machine back to the name it gives itself, else the
/// name, held to the rule every machine's own name is (`valid_name`).
fn chosen_name(name: &str) -> Result<Option<&str>, &'static str> {
    match name.trim() {
        "" => Ok(None),
        name if hide_peer::link::valid_name(name) => Ok(Some(name)),
        _ => Err("name_invalid"),
    }
}

/// Records that `through` started the machine's Hide, and keeps `label`,
/// the name its device had here, as the operator's chosen name unless one
/// was chosen already (B36). False when the record did not change.
fn started_through(record: &mut Host, through: &Login, label: Option<&str>) -> bool {
    let label = label
        .map(str::trim)
        .filter(|label| hide_peer::link::valid_name(label));
    let name = record.chosen_name.is_none() && label.is_some();
    if record.started.as_ref() == Some(through) && !name {
        return false;
    }
    record.started = Some(through.clone());
    if name {
        record.chosen_name = label.map(str::to_owned);
    }
    true
}

fn rows(
    hosts: &[Host],
    guests: &[Grant],
    link: impl Fn(&PeerKey) -> Option<Link>,
    updates: &HashMap<PeerKey, PeerUpdate>,
    ssh: impl Fn(&PeerKey) -> Option<PeerSshBlocked>,
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
                name: host.shown_name().to_owned(),
                named: host.chosen_name.is_some(),
                opens: Some(opens),
                opened_by: None,
                connected: link.is_some(),
                path: link.map(|link| path_of(link.path())),
                reason: None,
                update: updates.get(&host.key).copied(),
                // The distribution this PC reaches it through, when it is
                // one: the same record `admit_switch` reads.
                wsl: host
                    .candidates
                    .iter()
                    .find_map(|candidate| match candidate {
                        Candidate::Wsl(distro) => Some(distro.clone()),
                        _ => None,
                    }),
                // The core's to say, of its own devices.
                helper_device: None,
                // Only a host this machine keeps has a route, and the row
                // says only why one waits for the operator.
                ssh: ssh(&host.key).map(|blocked| PeerSsh { blocked }),
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
                name: guest.shown_name().to_owned(),
                named: guest.chosen_name.is_some(),
                opens: None,
                opened_by,
                connected: link.is_some(),
                path: link.map(|link| path_of(link.path())),
                reason: None,
                update: updates.get(&guest.key).copied(),
                wsl: None,
                helper_device: None,
                ssh: None,
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
/// Whether a pairing finished between `before` and `after`, the machines
/// told before and now: a machine opens this one, or is opened by it, where
/// before it was not listed or held neither direction yet (a confirm under
/// way, a pairing the other machine had dropped). A resume, a reconnect and
/// the daemon's start (nothing told before) finish none, so the usage
/// statistics count a pairing once, on each machine.
fn pairing_finished(before: &[PeerMachine], after: &[PeerMachine]) -> bool {
    let holds = |machine: &PeerMachine| {
        machine.opened_by.is_some()
            || matches!(
                machine.opens,
                Some(PeerOpens::Active | PeerOpens::Paused | PeerOpens::PausedThere)
            )
    };
    after
        .iter()
        .filter(|machine| holds(machine))
        .any(|machine| {
            !before
                .iter()
                .any(|earlier| earlier.key == machine.key && holds(earlier))
        })
}

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
        PathKind::Ssh => PeerPath::Ssh,
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
            named: false,
            opens: opens.then_some(PeerOpens::Active),
            opened_by: opened_by.then_some(PeerOpenedBy::Active),
            connected: false,
            path: None,
            reason: None,
            update: None,
            wsl: None,
            helper_device: None,
            ssh: None,
        }
    }

    /// The usage statistics count a pairing when it finishes, on the side
    /// that allowed it and on the side that joined, and never for a confirm
    /// still under way, a resume or a reconnect.
    #[test]
    fn a_pairing_finishes_once_and_a_resume_or_reconnect_finishes_none() {
        let confirming = PeerMachine {
            opens: Some(PeerOpens::Confirming),
            ..machine(false, false)
        };
        assert!(pairing_finished(&[], &[machine(false, true)]));
        assert!(!pairing_finished(&[], std::slice::from_ref(&confirming)));
        assert!(pairing_finished(&[confirming], &[machine(true, false)]));
        let paused = PeerMachine {
            opens: Some(PeerOpens::Paused),
            ..machine(false, false)
        };
        assert!(!pairing_finished(&[paused], &[machine(true, false)]));
        let connected = PeerMachine {
            connected: true,
            ..machine(true, false)
        };
        assert!(!pairing_finished(&[machine(true, false)], &[connected]));
        assert!(!pairing_finished(&[machine(true, true)], &[]));
    }

    fn paired(key: u8, device: &str, name: &str) -> Paired {
        let key = PeerKey::from_bytes([key; 32]);
        Paired {
            key,
            machine: PeerMachine {
                key: key.to_hex(),
                device_id: device.into(),
                name: name.into(),
                named: false,
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
            chosen_name: None,
            grant: "grant".into(),
            state,
            candidates: Vec::new(),
            started: None,
            ssh: None,
        };
        // A host that dropped the pairing is listed and is nothing to dial.
        let (dropped, away) = rows(
            &[host(1, KnownState::Unpaired)],
            &[],
            |_| None,
            &HashMap::new(),
            |_| None,
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
            |_| None,
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
            json!({"verb": "rename", "device": "pc", "name": "회사 PC"}),
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
    fn an_update_that_was_asked_for_stays_asked_while_the_same_side_is_behind() {
        use PeerUpdate::{Here, HereAsked, There, ThereAsked};
        for (said, behind, says) in [
            (None, Behind::Theirs, There),
            (None, Behind::Ours, Here),
            (None, Behind::Either, Here),
            (Some(ThereAsked), Behind::Theirs, ThereAsked),
            (Some(HereAsked), Behind::Ours, HereAsked),
            (Some(HereAsked), Behind::Either, HereAsked),
            // The other side became the older one: nothing was asked of it.
            (Some(ThereAsked), Behind::Ours, Here),
            (Some(HereAsked), Behind::Theirs, There),
        ] {
            assert_eq!(still_needed(said, behind), says, "{said:?} {behind:?}");
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
            // Another machine's app restarts on it (B22).
            json!({"verb": "update", "device": "pc"}),
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
            // A name is this machine's word for the other one.
            json!({"verb": "rename", "device": "pc", "name": "회사 PC"}),
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
            "update_not_asked",
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

    /// A login that ends after the read that decided a switch, as when its
    /// sshd is cut while the target accepts the pairing, answers the cut
    /// the next press carries, never the login that asks for a password
    /// the read alone can say; every other refusal is kept as it came.
    #[test]
    fn a_login_cut_during_a_switch_is_not_a_login_that_asks() {
        assert_eq!(cut_midway("ssh_not_reached"), "switch_cut");
        assert_eq!(cut_midway("wsl_not_reached"), "switch_cut");
        for code in [
            "ssh_core_not_started",
            "switch_not_connected",
            "ssh_mac_signed_out",
        ] {
            assert_eq!(cut_midway(code), code);
        }
        assert!(next_action("switch_cut").contains("Try the switch again"));
    }

    /// One raise runs for a machine at a time, a build already tried is
    /// tried again only when asked, and a raise whose machine was taken off
    /// while it ran leaves nothing of that machine behind when it ends.
    #[test]
    fn a_raise_that_ends_after_its_machine_was_taken_off_writes_nothing() {
        let (kept, gone) = (PeerKey::from_bytes([1; 32]), PeerKey::from_bytes([2; 32]));
        let mut raises = Raises::default();
        let first = raises.begin(kept, "1.2.9", false).unwrap();
        assert_eq!(raises.begin(kept, "1.2.9", true), None, "one already runs");
        assert!(raises.finish(kept, first, Err(Failure::Busy)));
        assert_eq!(raises.shown.get(&kept), Some(&PeerUpdate::UpdateBusy));
        assert_eq!(raises.begin(kept, "1.2.9", false), None, "tried, not asked");
        let again = raises.begin(kept, "1.2.9", true).unwrap();
        assert!(raises.finish(kept, again, Ok(Raised::Updated)));
        assert_eq!(raises.shown.get(&kept), None);

        let late = raises.begin(gone, "1.2.9", false).unwrap();
        raises.keep(&std::collections::HashSet::from([kept]));
        assert!(!raises.finish(gone, late, Err(Failure::Failed)));
        // Paired again while the first raise still ran: only the new
        // raise's end is the machine's.
        let new = raises.begin(gone, "1.2.9", false).unwrap();
        assert!(!raises.finish(gone, late, Err(Failure::Failed)));
        assert_eq!(raises.shown.get(&gone), Some(&PeerUpdate::Updating));
        assert!(raises.finish(gone, new, Ok(Raised::Current)));
        raises.keep(&std::collections::HashSet::from([kept]));
        assert!(raises.shown.is_empty() && raises.running.is_empty());
        assert_eq!(raises.tried.len(), 1);
    }

    /// An address an operator gives goes first and the last ones past the
    /// cap go, but never a distribution's name: it is the only way this PC
    /// reaches that Hide, and `hide device address` used often would have
    /// cut it off.
    #[test]
    fn a_given_address_never_pushes_a_distribution_s_name_out() {
        let address = |last: u8| Candidate::Ip(SocketAddr::from(([10, 0, 0, last], 7000)));
        let distribution = Candidate::Wsl("Ubuntu".to_owned());
        let mut candidates = vec![distribution.clone()];
        for last in 1..=MAX_ADDRESSES as u8 + 2 {
            put_first(&mut candidates, address(last));
        }
        assert_eq!(candidates.len(), MAX_ADDRESSES);
        assert_eq!(candidates[0], address(MAX_ADDRESSES as u8 + 2));
        assert_eq!(candidates.last(), Some(&distribution));
        // The same address again moves to the front and adds nothing.
        put_first(&mut candidates, address(5));
        assert_eq!(candidates.len(), MAX_ADDRESSES);
        assert_eq!(candidates[0], address(5));
        assert!(candidates.contains(&distribution));
    }

    /// A rename keeps the name the operator typed as theirs, refuses what
    /// no machine's name may hold, and an empty one takes the chosen name
    /// back.
    #[test]
    fn a_rename_keeps_a_name_a_machine_could_have_and_an_empty_one_takes_it_back() {
        assert_eq!(chosen_name(" 회사 PC "), Ok(Some("회사 PC")));
        assert_eq!(chosen_name(""), Ok(None));
        assert_eq!(chosen_name("   "), Ok(None));
        for refused in ["a\nb", "\u{202e}cp.exe", &"가".repeat(22)] {
            assert_eq!(chosen_name(refused), Err("name_invalid"), "{refused:?}");
        }
    }

    fn host(key: u8, chosen_name: Option<&str>) -> Host {
        Host {
            key: PeerKey::from_bytes([key; 32]),
            device: format!("device-{key}"),
            name: format!("Machine {key}"),
            chosen_name: chosen_name.map(str::to_owned),
            grant: "grant".into(),
            state: KnownState::Active,
            candidates: Vec::new(),
            started: None,
            ssh: None,
        }
    }

    /// B36: a switched machine is shown under the name its device had
    /// here, as the operator's chosen name, and a name they chose already
    /// is never replaced by it.
    #[test]
    fn a_switch_keeps_the_devices_name_as_the_chosen_one_unless_one_was_chosen() {
        let ssh = Login::Ssh("mini".into());
        let mut record = host(1, None);
        assert!(started_through(&mut record, &ssh, Some("빌드 서버")));
        assert_eq!(record.started, Some(ssh.clone()));
        assert_eq!(record.chosen_name.as_deref(), Some("빌드 서버"));
        assert_eq!(record.name, "Machine 1", "the machine's own name stays");
        // The same switch again changes nothing.
        assert!(!started_through(&mut record, &ssh, Some("다른 이름")));
        assert_eq!(record.chosen_name.as_deref(), Some("빌드 서버"));

        let mut chosen = host(2, Some("거실 Mac mini"));
        assert!(started_through(&mut chosen, &ssh, Some("mini")));
        assert_eq!(chosen.chosen_name.as_deref(), Some("거실 Mac mini"));

        for label in [None, Some("\u{202e}mini"), Some("  ")] {
            let mut record = host(3, None);
            assert!(started_through(&mut record, &ssh, label));
            assert_eq!(record.chosen_name, None, "{label:?}");
        }
    }

    /// A row shows the chosen name while there is one and says so, and the
    /// machine's own otherwise.
    #[test]
    fn a_row_shows_the_chosen_name_and_says_it_was_chosen() {
        let (rows, _) = rows(
            &[host(1, Some("거실 Mac mini")), host(2, None)],
            &[],
            |_| None,
            &HashMap::new(),
            |_| None,
        );
        let mut shown: Vec<_> = rows
            .iter()
            .map(|row| (row.machine.name.as_str(), row.machine.named))
            .collect();
        shown.sort();
        assert_eq!(shown, [("Machine 2", false), ("거실 Mac mini", true)]);
    }
}
