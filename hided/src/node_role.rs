//! This daemon in the node role (PRD core-host-node-remote-core D-02, D-04,
//! D-08, D-09): the core runs on another machine, which the placement record
//! names, and this daemon keeps one link to it. It dials the core's machine
//! over SSH, runs the attach role there, trades the handshake, and answers
//! the core's calls on that channel exactly as a device the core dialed
//! answers them, with its own Herdr reached through the link.
//!
//! While the core cannot be reached the node waits and tries again, two
//! seconds after the first failure and doubling to a minute; a link that
//! lived a while starts the wait over. A watch thread looks every two
//! seconds for what makes the wait wrong (D-09): when the machine slept or
//! its network addresses changed, a node waiting on what a move may change
//! (the core's machine unreachable, a connection that failed on the way, a
//! link that was lost) tries at once, a node the core refused or that has
//! something to fix keeps its wait, and a live link must answer an SSH ping
//! within three seconds or is dropped and dialed again; while the core's
//! machine is unreachable, its SSH port is probed and the node tries at once
//! when the port answers again. Dropping the role ends the link, the SSH
//! connection and both threads.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use hide_node::ssh::SshAlias;
use hide_node::ssh::upstream::Upstream;
use hide_node::terminal::OutputSink;
use hide_node::terminal::device::NodeTerminals;
use hide_platform::ipc::{LocalStream, ShutdownHandle};
use serde_json::json;
use tokio::sync::watch;

use crate::attach::{self, Accepted, AttachOutcome, Line, NodeHello};
use crate::backoff::Backoff;
use crate::placement::Placement;

/// A link that lived this long starts the wait over.
const SETTLED: Duration = Duration::from_secs(30);
/// How long the core's machine has to answer the handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// How often the watch looks at the clocks, the network and, while the
/// core's machine is unreachable, its SSH port.
const WATCH_EVERY: Duration = Duration::from_secs(2);
/// The wall clock moving this much further than the monotonic clock, which
/// stops while the machine sleeps, between two looks means it slept.
const SLEPT: Duration = Duration::from_secs(5);
/// How long a live link's connection has to answer a ping after a wake.
const ANSWER_WITHIN: Duration = Duration::from_secs(3);
/// How long one probe of the core machine's SSH port may take in all:
/// resolving its name, taking the connection and greeting it.
const PROBE_WITHIN: Duration = Duration::from_secs(1);

/// Who this node is, as it tells its core.
#[derive(Clone, Debug)]
pub struct NodeIdentity {
    pub node: String,
    pub label: String,
    pub build: String,
    /// This build's version and order, which the core's are compared with.
    pub release: crate::build_order::Release,
    /// This machine's own Herdr, which the core reaches only through the
    /// link.
    pub herdr_socket: PathBuf,
    /// The Herdr binary this machine's panes attach with.
    pub herdr_bin: PathBuf,
    /// Where this node keeps what it measured of each repository's
    /// worktrees across links and restarts; none keeps them for one link.
    pub worktree_facts: Option<PathBuf>,
}

/// A link the core took, as the node's screens reach the core through it.
pub struct LiveLink {
    /// Counts links since the role started: a screen that opened on one
    /// link ends with it.
    pub generation: u64,
    pub accepted: Accepted,
    /// The loopback port on this machine whose connections reach the
    /// core's port through the link's SSH connection.
    pub relay_port: u16,
    /// This node's terminals for the link's life: the node's screens send
    /// its own panes' keys here, without the link.
    pub terminals: Arc<NodeTerminals>,
    /// The link's SSH connection, which a page of the core's machine
    /// reaches the core's loopback through (`node_pages`).
    pub upstream: Arc<Upstream>,
}

/// Where the node's link is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Phase {
    /// Dialing the core's machine and trading the handshake.
    Connecting,
    /// The core took the link.
    Live(Accepted),
    /// The core runs an older build, and this node is updating it to its
    /// own (B10); `machine` is the core machine's name as its hello gave
    /// it.
    Updating {
        machine: String,
        core: crate::build_order::Release,
    },
    /// The last attempt failed or the link ended; the next one waits.
    Waiting { reason: LinkFailure },
    /// The operator ended the link from this machine's window (PRD
    /// core-host-node-move B16): nothing dials until they reconnect, and the
    /// placement record keeps the choice across restarts.
    Disconnected,
}

/// Why the node's last attempt failed or its link ended. Its text is the
/// `core_link_reason` the node's health reports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkFailure {
    /// The core's machine could not be reached over SSH: the watch probes
    /// its SSH port and tries at once when it answers again.
    Unreachable(String),
    /// The core's machine answered, and its attach role reached no core.
    Attach(AttachOutcome),
    /// The core's machine answers as another core than the placement names.
    WrongCore(String),
    /// The core refused the node, with its reason (`other_build`,
    /// `own_node`, ...).
    Refused(String),
    /// The core runs a newer build than this node's (B11): the core is left
    /// as it is and this machine's app is the one to update. `machine` is
    /// the core machine's name as its hello gave it.
    CoreNewer {
        machine: String,
        release: crate::build_order::Release,
    },
    /// The core runs an older build than this node's: the node updates it
    /// to its own once per connection (B10, B20), and links once it runs
    /// this build. `machine` is the core machine's name as its hello gave
    /// it.
    CoreOlder {
        machine: String,
        release: crate::build_order::Release,
    },
    /// The core was updated to this node's build, which `program` names on
    /// its machine: the next dial goes at once.
    CoreUpdated { program: String },
    /// This connection's update of the core failed, and the core on
    /// `machine` runs the build it ran before (`core`): its reason. The
    /// next connection tries once more ([`NodeRole::connect_again`]).
    UpdateFailed {
        reason: String,
        machine: String,
        core: crate::build_order::Release,
    },
    /// The connection to the core's machine failed after its SSH server
    /// answered and before the core took the link: a reset or timeout in the
    /// key exchange, a channel that failed, or the handshake cut off or left
    /// unanswered. A move may change it.
    Transport(String),
    /// Anything else on the way to a link, which no move changes (an alias,
    /// a host key or a sign-in to fix, an answer this build does not read):
    /// its reason.
    Ended(String),
    /// A link the core had taken ended: its reason.
    Lost(String),
    /// The role is stopping.
    Stopping,
}

impl LinkFailure {
    /// Whether a sleep or a network move may change this answer, so the
    /// wait is cut short when one is seen: a dial that never reached the
    /// core's machine, a connection that failed on the way, and a link that
    /// was lost. A refusal, an attach outcome, a wrong core, or what the
    /// operator must fix answers the same from any network.
    fn a_move_can_change(&self) -> bool {
        matches!(
            self,
            Self::Unreachable(_) | Self::Transport(_) | Self::Lost(_)
        )
    }
}

impl Phase {
    /// The phase as a held screen is told it (`core_link` frame), with the
    /// core machine's name when its hello gave it and `alias`, the SSH alias
    /// this machine dials it by, which a screen that has not drawn the core
    /// yet names instead of an empty machine; `None` while live, when the
    /// core draws the screen itself.
    pub fn link_frame(&self, alias: &str) -> Option<serde_json::Value> {
        let (phase, machine) = match self {
            Self::Live(_) => return None,
            Self::Connecting => ("connecting", None),
            Self::Updating { machine, .. } => ("updating", Some(machine)),
            Self::Disconnected => ("disconnected", None),
            Self::Waiting { reason } => (
                "waiting",
                match reason {
                    LinkFailure::CoreNewer { machine, .. }
                    | LinkFailure::CoreOlder { machine, .. }
                    | LinkFailure::UpdateFailed { machine, .. } => Some(machine),
                    _ => None,
                },
            ),
        };
        Some(
            json!({"type": "core_link", "payload": {"phase": phase, "machine": machine, "alias": alias}}),
        )
    }
}

impl std::fmt::Display for LinkFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(error) => write!(formatter, "unreachable: {error}"),
            Self::Attach(outcome) => formatter.write_str(outcome.code()),
            Self::WrongCore(node) => write!(formatter, "wrong_core: the machine answers as {node}"),
            Self::Refused(reason)
            | Self::Transport(reason)
            | Self::Ended(reason)
            | Self::Lost(reason) => formatter.write_str(reason),
            Self::CoreNewer { .. } => formatter.write_str("core_newer"),
            Self::CoreOlder { .. } => formatter.write_str("core_older"),
            Self::CoreUpdated { .. } => formatter.write_str("core_updated"),
            Self::UpdateFailed { .. } => formatter.write_str("update_failed"),
            Self::Stopping => formatter.write_str("stopping"),
        }
    }
}

struct State {
    phase: Phase,
    stopping: bool,
    /// Set by [`NodeRole::wake`]: the next attempt does not wait.
    woken: bool,
    /// The live link's stream, ended to end the link.
    link: Option<ShutdownHandle>,
    /// This connection's one update of the core (B20).
    update: Update,
    /// The operator ended the link (B16); the link thread parks.
    disconnected: bool,
}

/// Where a connection's one update of the core stands (B10, B20).
#[derive(Clone, Debug, Eq, PartialEq)]
enum Update {
    /// The connection may update a core that runs an older build.
    Untried,
    /// The core reported this build running and no link has gone live
    /// since: a core still older is this update not holding.
    Unproven,
    /// Spent, with its reason: the core is refused as older until the next
    /// connection.
    Failed(String),
}

/// Why a connection updates its core no more.
#[derive(Debug, Eq, PartialEq)]
struct Spent {
    reason: String,
    /// It was spent by this attempt, not before it.
    now: bool,
}

/// The reason an update spends itself when it did not hold.
const UPDATE_NOT_KEPT: &str = "not_kept: the core runs an older build again after its update";

impl Update {
    /// A core older than this build asks for an update: `Ok` when this
    /// connection may run one. An update that did not hold until a live
    /// link is spent here rather than run again, so a core that keeps going
    /// back is updated once per connection, not once per dial.
    fn attempt(&mut self) -> Result<(), Spent> {
        match self {
            Self::Untried => Ok(()),
            Self::Unproven => {
                *self = Self::Failed(UPDATE_NOT_KEPT.to_owned());
                Err(Spent {
                    reason: UPDATE_NOT_KEPT.to_owned(),
                    now: true,
                })
            }
            Self::Failed(reason) => Err(Spent {
                reason: reason.clone(),
                now: false,
            }),
        }
    }

    /// A link went live: an update before it held, and a core older again
    /// later is a new reason to update.
    fn held(&mut self) {
        if *self == Self::Unproven {
            *self = Self::Untried;
        }
    }
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    upstream: Mutex<Option<Arc<Upstream>>>,
    live: watch::Sender<Option<Arc<LiveLink>>>,
    /// The phase as it changes, for the screens the node holds while the
    /// core cannot draw them.
    phases: watch::Sender<Phase>,
    /// Where the node's own panes' output goes besides the link.
    screen: Arc<dyn OutputSink>,
    /// This machine's desktop windows' browser gateways, which the core
    /// asks over each link (`node_browser`).
    browser: Option<Arc<crate::node_browser::NodeBrowser>>,
    /// Told the checkout roots the core opened on this node over the live
    /// link each time they change, and none when a link starts: the
    /// node's screens read files under them without the core.
    roots_changed: Option<RootsChanged>,
    /// How this node updates an older core to its build; none leaves an
    /// older core refused as another build.
    updates: Option<Updates>,
}

/// What a node needs to update its core to this build (PRD
/// core-host-node-move B10): the folder this build's programs ship in, and
/// the node's state folder, whose placement record names the program the
/// core's machine runs.
#[derive(Clone, Debug)]
pub struct Updates {
    pub packages: PathBuf,
    pub state_dir: PathBuf,
}

/// Hears the checkout roots the core opened on this node over the live link.
pub type RootsChanged = Arc<dyn Fn(&[String]) + Send + Sync>;

/// The node role's link to its core, held for as long as the daemon runs.
pub struct NodeRole {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    watch: Option<JoinHandle<()>>,
}

impl NodeRole {
    /// Starts keeping the link to the core `placement` names; `home` is
    /// the account whose `~/.ssh/config` names its alias.
    /// Its own panes' output reaches `screen`, the node's screens.
    pub fn start(
        home: &Path,
        placement: Placement,
        identity: NodeIdentity,
        screen: Arc<dyn OutputSink>,
    ) -> Result<Self, String> {
        Self::start_for_screens(home, placement, identity, screen, None, None, None)
    }

    /// [`NodeRole::start`] for a daemon whose screens work on this machine:
    /// its links also answer the core for this machine's desktop windows
    /// (`browser`), and the roots the core opens on each are told to
    /// `roots_changed`.
    pub fn start_for_screens(
        home: &Path,
        placement: Placement,
        identity: NodeIdentity,
        screen: Arc<dyn OutputSink>,
        browser: Option<Arc<crate::node_browser::NodeBrowser>>,
        roots_changed: Option<RootsChanged>,
        updates: Option<Updates>,
    ) -> Result<Self, String> {
        // A node the operator disconnected stays so from its first look.
        let first = if placement.disconnected {
            Phase::Disconnected
        } else {
            Phase::Connecting
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                phase: first.clone(),
                stopping: false,
                woken: false,
                link: None,
                update: Update::Untried,
                disconnected: placement.disconnected,
            }),
            changed: Condvar::new(),
            upstream: Mutex::new(None),
            live: watch::Sender::new(None),
            phases: watch::Sender::new(first),
            screen,
            browser,
            roots_changed,
            updates,
        });
        let config = home.join(".ssh/config");
        let thread = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("node-role-link".to_owned())
                .spawn(move || keep_linked(&shared, &config, &placement, &identity))
                .map_err(|error| format!("the node link thread could not start: {error}"))?
        };
        let mut role = Self {
            shared,
            thread: Some(thread),
            watch: None,
        };
        // Dropping `role` on a failure here ends the link thread.
        let shared = Arc::clone(&role.shared);
        role.watch = Some(
            std::thread::Builder::new()
                .name("node-role-watch".to_owned())
                .spawn(move || watch(&shared))
                .map_err(|error| format!("the node watch thread could not start: {error}"))?,
        );
        Ok(role)
    }

    pub fn phase(&self) -> Phase {
        lock(&self.shared.state).phase.clone()
    }

    /// The live link, as it changes: `None` while there is none.
    pub fn live(&self) -> watch::Receiver<Option<Arc<LiveLink>>> {
        self.shared.live.subscribe()
    }

    /// The phase, as it changes.
    pub fn phases(&self) -> watch::Receiver<Phase> {
        self.shared.phases.subscribe()
    }

    /// The operator ends the link from this machine's window (B16): the
    /// live link closes now and nothing dials until [`NodeRole::reconnect`].
    /// The core refuses this machine's panes, files and pages from then on
    /// as it does for any node whose link ended.
    pub fn disconnect(&self) {
        let mut state = lock(&self.shared.state);
        state.disconnected = true;
        if let Some(link) = state.link.take() {
            link.shutdown();
        }
        drop(state);
        let upstream = lock(&self.shared.upstream).clone();
        if let Some(upstream) = upstream {
            upstream.close();
        }
        self.shared.changed.notify_all();
    }

    /// The operator links this machine to its core again: the next attempt
    /// goes at once.
    pub fn reconnect(&self) {
        let mut state = lock(&self.shared.state);
        if !std::mem::take(&mut state.disconnected) {
            return;
        }
        // A new connection: a core update that failed gets its attempt.
        state.update = Update::Untried;
        state.woken = true;
        drop(state);
        self.shared.changed.notify_all();
    }

    /// Waits up to `timeout` for the phase to satisfy `done`, and answers
    /// the phase it ended on.
    pub fn wait_for(&self, timeout: Duration, done: impl Fn(&Phase) -> bool) -> Phase {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.shared.state);
        while !done(&state.phase) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        state.phase.clone()
    }

    /// The machine woke or its network changed: a link that may be dead is
    /// dropped and the next attempt goes at once.
    pub fn wake(&self) {
        wake(&self.shared);
    }

    /// A window connects (`hide connect`, a launch or its Retry): a core
    /// update that failed on the last connection gets this one's attempt,
    /// which starts at once (B10, B20). Anything else is left as it is.
    pub fn connect_again(&self) {
        let mut state = lock(&self.shared.state);
        // The operator ended the link; only their reconnect dials again.
        if state.disconnected || !matches!(state.update, Update::Failed(_)) {
            return;
        }
        state.update = Update::Untried;
        state.phase = Phase::Connecting;
        self.shared.phases.send_replace(Phase::Connecting);
        drop(state);
        wake(&self.shared);
    }
}

fn wake(shared: &Shared) {
    let mut state = lock(&shared.state);
    state.woken = true;
    if let Some(link) = state.link.take() {
        link.shutdown();
    }
    drop(state);
    // Closed outside the slot's lock: a close waits for a dial in progress,
    // which must not hold the next attempt or the role's end behind it.
    let upstream = lock(&shared.upstream).clone();
    if let Some(upstream) = upstream {
        upstream.close();
    }
    shared.changed.notify_all();
}

impl Drop for NodeRole {
    fn drop(&mut self) {
        {
            let mut state = lock(&self.shared.state);
            state.stopping = true;
            if let Some(link) = state.link.take() {
                link.shutdown();
            }
        }
        let upstream = lock(&self.shared.upstream).take();
        if let Some(upstream) = upstream {
            upstream.close();
        }
        self.shared.changed.notify_all();
        for thread in [self.thread.take(), self.watch.take()]
            .into_iter()
            .flatten()
        {
            let _ = thread.join();
        }
    }
}

fn keep_linked(shared: &Shared, config: &Path, placement: &Placement, identity: &NodeIdentity) {
    let mut placement = placement.clone();
    let mut backoff = Backoff::default();
    let mut generation = 0;
    loop {
        if parked(shared) {
            return;
        }
        generation += 1;
        if set_phase(shared, Phase::Connecting) {
            return;
        }
        let started = Instant::now();
        let reason = link_once(shared, config, &placement, identity, generation);
        // The core now runs this build: the next dial reaches it at once.
        if let LinkFailure::CoreUpdated { program } = reason {
            placement.program = program;
            backoff.reset();
            continue;
        }
        // The operator ended the link: the role parks at the loop's top.
        if lock(&shared.state).disconnected {
            backoff.reset();
            continue;
        }
        shared.live.send_replace(None);
        if started.elapsed() >= SETTLED {
            backoff.reset();
        }
        let wait = backoff.failed();
        herdr_core::diagnostic!(json!({
            "component": "node_role",
            "kind": "link.ended",
            "core": placement.node,
            "reason": reason.to_string(),
            "lived_ms": started.elapsed().as_millis() as u64,
            "retry_ms": wait.as_millis() as u64,
        }));
        if set_phase(shared, Phase::Waiting { reason }) {
            return;
        }
        let deadline = Instant::now() + wait;
        let mut state = lock(&shared.state);
        while !state.stopping && !state.woken && !state.disconnected {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            state = shared
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        if state.stopping {
            return;
        }
        // Woken, the wait starts over.
        if std::mem::take(&mut state.woken) {
            backoff.reset();
        }
    }
}

/// Looks every [`WATCH_EVERY`] for a sleep, a network change, or the core
/// machine's SSH port coming back, until the role stops.
fn watch(shared: &Shared) {
    let mut look = Look::new(SystemTime::now(), Instant::now(), network_addresses());
    let mut port = PortProbe::default();
    loop {
        {
            let state = lock(&shared.state);
            let (state, _) = shared
                .changed
                .wait_timeout_while(state, WATCH_EVERY, |state| !state.stopping)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.stopping {
                return;
            }
        }
        let moved = look.moved(SystemTime::now(), Instant::now(), network_addresses());
        let phase = lock(&shared.state).phase.clone();
        let upstream = lock(&shared.upstream).clone();
        if let Some(moved) = moved {
            port.moved();
            let answered = match (&phase, &upstream) {
                (Phase::Live(_), Some(upstream)) => Some(upstream.alive(ANSWER_WITHIN)),
                _ => None,
            };
            herdr_core::diagnostic!(json!({
                "component": "node_role",
                "kind": "machine.moved",
                "moved": moved,
                "live_link_answered": answered,
            }));
            if wakes_on_move(&phase, answered) {
                wake(shared);
            }
            continue;
        }
        let came_back = match &upstream {
            Some(upstream) => port.look(&phase, || upstream.reachable(PROBE_WITHIN)),
            None => port.look(&phase, || false),
        };
        if came_back {
            herdr_core::diagnostic!(json!({
                "component": "node_role",
                "kind": "core.reachable_again",
            }));
            wake(shared);
        }
    }
}

/// Whether a sleep or network move seen in `phase` starts the next attempt
/// now: a live link that no longer answers (`answered`), or a wait on a
/// failure a move may change. A refused node keeps its backoff.
fn wakes_on_move(phase: &Phase, answered: Option<bool>) -> bool {
    match phase {
        Phase::Connecting | Phase::Updating { .. } | Phase::Disconnected => false,
        Phase::Live(_) => answered != Some(true),
        Phase::Waiting { reason } => reason.a_move_can_change(),
    }
}

/// Whether the watch probes the core machine's SSH port: only while the
/// node waits after a dial that could not reach the machine, and not again
/// in that outage once the port answered. A port that answers while the dial
/// still fails says the failure is past the port, and the wait's own retry
/// tries it again.
fn probes_port(phase: &Phase, port: &PortWatch) -> bool {
    matches!(
        phase,
        Phase::Waiting {
            reason: LinkFailure::Unreachable(_)
        }
    ) && port.last != Some(true)
}

fn network_addresses() -> Option<BTreeSet<IpAddr>> {
    hide_platform::host::network_addresses().ok()
}

/// What the watch saw at its last look.
struct Look {
    wall: SystemTime,
    monotonic: Instant,
    addresses: Option<BTreeSet<IpAddr>>,
}

impl Look {
    fn new(wall: SystemTime, monotonic: Instant, addresses: Option<BTreeSet<IpAddr>>) -> Self {
        Self {
            wall,
            monotonic,
            addresses: addresses.map(routable),
        }
    }

    /// Takes a new look, and answers what moved since the last one:
    /// `"slept"` when the wall clock ran [`SLEPT`] past the monotonic one,
    /// `"network"` when the address set changed. An address set that could
    /// not be read says nothing. Link-local addresses (IPv4 169.254/16,
    /// IPv6 fe80::/10) do not count: macOS gives and takes them on its own
    /// (awdl0, llw0, utun), and no route to a core machine named by an SSH
    /// alias moves with them.
    fn moved(
        &mut self,
        wall: SystemTime,
        monotonic: Instant,
        addresses: Option<BTreeSet<IpAddr>>,
    ) -> Option<&'static str> {
        let walked = wall.duration_since(self.wall).unwrap_or_default();
        let ran = monotonic.saturating_duration_since(self.monotonic);
        let slept = walked.saturating_sub(ran) >= SLEPT;
        let addresses = addresses.map(routable);
        let network = match (&self.addresses, &addresses) {
            (Some(before), Some(now)) => before != now,
            _ => false,
        };
        self.wall = wall;
        self.monotonic = monotonic;
        if addresses.is_some() {
            self.addresses = addresses;
        }
        if slept {
            Some("slept")
        } else if network {
            Some("network")
        } else {
            None
        }
    }
}

/// The addresses a move of the network shows in: all but link-local ones.
fn routable(addresses: BTreeSet<IpAddr>) -> BTreeSet<IpAddr> {
    addresses
        .into_iter()
        .filter(|address| match address {
            IpAddr::V4(v4) => !v4.is_link_local(),
            IpAddr::V6(v6) => !v6.is_unicast_link_local(),
        })
        .collect()
}

/// The watch's probes of the core machine's SSH port, look by look, over
/// one outage: the unreachable waits and the dials between them. What the
/// port said holds for the whole outage, so a port silent in one wait and
/// answering at the first look of the next still wakes the node, and a port
/// that answers is probed once, not every other look. It starts over when
/// the outage ends (a link, or a failure past the port) or the machine
/// moves.
#[derive(Default)]
struct PortProbe {
    port: PortWatch,
}

impl PortProbe {
    /// One look in `phase`: probes when [`probes_port`] says so, and answers
    /// whether the port came back.
    fn look(&mut self, phase: &Phase, probe: impl FnOnce() -> bool) -> bool {
        let outage = matches!(
            phase,
            Phase::Connecting
                | Phase::Waiting {
                    reason: LinkFailure::Unreachable(_)
                }
        );
        if !outage {
            self.port = PortWatch::default();
            return false;
        }
        probes_port(phase, &self.port) && self.port.came_back(probe())
    }

    /// A sleep or a network move: what the port said before says nothing now.
    fn moved(&mut self) {
        self.port = PortWatch::default();
    }
}

/// The core machine's SSH port across the probes of one unreachable wait.
#[derive(Default)]
struct PortWatch {
    last: Option<bool>,
}

impl PortWatch {
    /// Records a probe; `true` when the port answers after a probe it did
    /// not. A port that answers from the first probe on says nothing: the
    /// dial failed on something a new attempt at once would not change.
    fn came_back(&mut self, answers: bool) -> bool {
        let came_back = self.last == Some(false) && answers;
        self.last = Some(answers);
        came_back
    }
}

/// Sets the phase; `true` when the role is stopping.
fn set_phase(shared: &Shared, phase: Phase) -> bool {
    let mut state = lock(&shared.state);
    if let Phase::Live(_) = phase {
        state.update.held();
    }
    if state.phase != phase {
        state.phase = phase.clone();
        shared.phases.send_replace(phase);
        shared.changed.notify_all();
    }
    state.stopping
}

/// Waits while the operator keeps the link ended (B16); `true` when the
/// role is stopping.
fn parked(shared: &Shared) -> bool {
    {
        let state = lock(&shared.state);
        if !state.disconnected {
            return state.stopping;
        }
    }
    shared.live.send_replace(None);
    if set_phase(shared, Phase::Disconnected) {
        return true;
    }
    herdr_core::diagnostic!(json!({"component": "node_role", "kind": "link.disconnected"}));
    let mut state = lock(&shared.state);
    while state.disconnected && !state.stopping {
        state = shared
            .changed
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    state.woken = false;
    state.stopping
}

/// One link: the handshake, then the core's calls until the link ends.
/// Answers why it ended.
fn link_once(
    shared: &Shared,
    config: &Path,
    placement: &Placement,
    identity: &NodeIdentity,
    generation: u64,
) -> LinkFailure {
    match try_link(shared, config, placement, identity, generation) {
        Ok(ended) | Err(ended) => ended,
    }
}

fn try_link(
    shared: &Shared,
    config: &Path,
    placement: &Placement,
    identity: &NodeIdentity,
    generation: u64,
) -> Result<LinkFailure, LinkFailure> {
    // The alias is read again for every attempt, so a fix the operator
    // makes to it reaches the next dial; a connection to an alias that
    // changed is ended and a new one made.
    let alias = SshAlias::from_config_file(config, &placement.alias)
        .map_err(|error| LinkFailure::Ended(format!("ssh_alias: {}", error.diagnostic().reason)))?;
    let upstream = {
        let mut slot = lock(&shared.upstream);
        match slot.as_ref() {
            Some(upstream) if *upstream.alias() == alias => Arc::clone(upstream),
            _ => {
                let upstream = Arc::new(
                    Upstream::new(alias)
                        .map_err(|error| LinkFailure::Ended(format!("ssh: {error}")))?,
                );
                let replaced = slot.replace(Arc::clone(&upstream));
                drop(slot);
                if let Some(replaced) = replaced {
                    replaced.close();
                }
                upstream
            }
        }
    };
    let channel = upstream
        .attach(&placement.program, placement.state_dir.as_deref())
        .map_err(attach_failure)?;
    let stream = channel.stream;
    {
        let mut state = lock(&shared.state);
        if state.stopping {
            return Err(LinkFailure::Stopping);
        }
        // Ended by the operator while it was dialled: it is never served.
        if state.disconnected {
            return Err(LinkFailure::Ended("disconnected".to_owned()));
        }
        state.link = Some(stream.shutdown_handle());
    }
    let ended = serve_link(
        &stream,
        placement,
        identity,
        shared,
        &channel.stderr,
        &upstream,
        generation,
    );
    lock(&shared.state).link = None;
    stream.shutdown_handle().shutdown();
    match ended {
        Err(LinkFailure::CoreOlder { machine, release }) => Err(update_core(
            shared, &upstream, placement, identity, machine, release,
        )),
        ended => ended,
    }
}

/// Updates the core, which runs an older build, to this node's (PRD
/// core-host-node-move B10): this build goes into a version folder beside
/// the core's on its machine, and its own `hided core-update` replaces the
/// core and goes back to the previous build when the new one does not take
/// links. Once per connection (B20): a failed update is not tried again
/// until the next one, and the core is then refused as older; an update the
/// core reported done is failed too when the core answers as older again
/// before a link went live.
fn update_core(
    shared: &Shared,
    upstream: &Upstream,
    placement: &Placement,
    identity: &NodeIdentity,
    machine: String,
    core: crate::build_order::Release,
) -> LinkFailure {
    let Some(updates) = shared.updates.as_ref() else {
        return LinkFailure::Refused("other_build".to_owned());
    };
    // Spent: refused as older until the next connection.
    let spent = lock(&shared.state).update.attempt();
    if let Err(Spent { reason, now }) = spent {
        if now {
            herdr_core::diagnostic!(json!({
                "component": "node_role",
                "kind": "update.failed",
                "core": placement.node,
                "reason": reason,
            }));
        }
        return LinkFailure::UpdateFailed {
            reason,
            machine,
            core,
        };
    }
    let updating = Phase::Updating {
        machine: machine.clone(),
        core: core.clone(),
    };
    if set_phase(shared, updating) {
        return LinkFailure::Stopping;
    }
    let intent = crate::core_update::new_intent();
    let log = |kind: &str, fields: serde_json::Value| {
        let mut record = json!({
            "component": "node_role",
            "kind": kind,
            "intent": intent,
            "core": placement.node,
        });
        if let (Some(record), serde_json::Value::Object(fields)) = (record.as_object_mut(), fields)
        {
            record.extend(fields);
        }
        herdr_core::diagnostic!(record);
    };
    let failed = |reason: String| {
        lock(&shared.state).update = Update::Failed(reason.clone());
        log("update.failed", json!({"reason": reason}));
        LinkFailure::UpdateFailed {
            reason,
            machine: machine.clone(),
            core: core.clone(),
        }
    };
    log("update.started", json!({"core_release": core}));
    let root = match hide_kit::build_of(&placement.program) {
        Ok(build) => build.root,
        Err(reason) => return failed(reason),
    };
    let packages = hide_node::ssh::host::HelperPackages::new(Some(updates.packages.clone()));
    let program = match upstream.install_build(&packages, &root) {
        Ok(program) => program,
        Err(reason) => return failed(format!("upload: {reason}")),
    };
    log("update.uploaded", json!({"program": program}));
    let mut command = format!(
        "{} core-update --previous {} --intent {intent}",
        hide_node::ssh::shell_quote(&program),
        hide_node::ssh::shell_quote(&placement.program),
    );
    if let Some(state_dir) = &placement.state_dir {
        command.push_str(&format!(
            " --state-dir {}",
            hide_node::ssh::shell_quote(state_dir)
        ));
    }
    let output = match upstream.exec(
        "core-update",
        &command,
        crate::core_update::OUTPUT_CAP,
        crate::core_update::WITHIN,
    ) {
        Ok(output) => output,
        Err(error) => return failed(format!("core-update: {error}")),
    };
    if let Err(reason) = crate::core_update::outcome(
        &output.stdout,
        &output.exit_status.to_string(),
        &output.stderr,
    ) {
        return failed(reason);
    }
    // The node's record names the program its next dials run there.
    let recorded = crate::placement::update(&updates.state_dir, &identity.node, |record| {
        record.program = program.clone();
    });
    let unrecorded = match recorded {
        Ok(true) => None,
        Ok(false) => Some("the placement record is gone".to_owned()),
        Err(reason) => Some(reason),
    };
    if let Some(reason) = unrecorded {
        // The core runs this build; the record keeps naming the previous
        // build's program, which the update kept.
        log("update.record_failed", json!({"reason": reason}));
    }
    log("update.done", json!({"program": program}));
    lock(&shared.state).update = Update::Unproven;
    LinkFailure::CoreUpdated { program }
}

/// Why a dial for the attach role failed. Only a dial that never reached
/// the machine's SSH server is unreachable, and only it is probed; one that
/// failed on the way after the server answered is a transport failure a move
/// may change; a host key, a sign-in or an alias is the operator's to fix.
fn attach_failure(error: hide_node_link::device::RemoteError) -> LinkFailure {
    if error.never_reached_server() {
        LinkFailure::Unreachable(error.to_string())
    } else if error.a_move_can_change() {
        LinkFailure::Transport(format!("ssh: {error}"))
    } else {
        LinkFailure::Ended(format!("ssh: {error}"))
    }
}

/// Why the screens' forward over the link could not be set up: what a move
/// may change (the connection, a local port) is a transport failure, what
/// the operator must fix is not.
fn forward_failure(error: hide_node_link::device::RemoteError) -> LinkFailure {
    let reason = format!("relay_forward: {error}");
    if error.a_move_can_change() {
        LinkFailure::Transport(reason)
    } else {
        LinkFailure::Ended(reason)
    }
}

/// Why the handshake on the attach channel failed: cut off or unanswered is
/// the connection's failure, an unreadable line is not.
fn handshake_failure(error: attach::HandshakeError) -> LinkFailure {
    match error {
        attach::HandshakeError::Lost(message) => LinkFailure::Transport(message),
        attach::HandshakeError::Unreadable(message) => LinkFailure::Ended(message),
    }
}

fn serve_link(
    stream: &LocalStream,
    placement: &Placement,
    identity: &NodeIdentity,
    shared: &Shared,
    stderr: &Mutex<Vec<u8>>,
    upstream: &Arc<Upstream>,
    generation: u64,
) -> Result<LinkFailure, LinkFailure> {
    let ended = LinkFailure::Ended;
    let mut reader = BufReader::new(stream.duplicate());
    let mut writer = stream.duplicate();
    reader
        .get_ref()
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|error| ended(error.to_string()))?;
    // What the attach role said on its standard error, beside why its
    // channel gave no handshake.
    let silent = |error: attach::HandshakeError| {
        let said = String::from_utf8_lossy(&lock(stderr)).trim().to_owned();
        handshake_failure(if said.is_empty() {
            error
        } else {
            match error {
                attach::HandshakeError::Lost(message) => {
                    attach::HandshakeError::Lost(format!("{message} ({said})"))
                }
                attach::HandshakeError::Unreadable(message) => {
                    attach::HandshakeError::Unreadable(format!("{message} ({said})"))
                }
            }
        })
    };
    match attach::read_line(&mut reader).map_err(silent)? {
        Line::Attach(outcome) => return Err(LinkFailure::Attach(outcome)),
        Line::Core(core) if core.node != placement.node => {
            return Err(LinkFailure::WrongCore(core.node));
        }
        Line::Core(core) => {
            let standing = crate::build_order::standing(
                (&identity.build, &identity.release),
                (&core.build, &core.release),
            );
            let refused = match standing {
                crate::build_order::Standing::Same => None,
                crate::build_order::Standing::Older => Some(LinkFailure::CoreNewer {
                    machine: core.label.clone(),
                    release: core.release.clone(),
                }),
                crate::build_order::Standing::Newer => Some(LinkFailure::CoreOlder {
                    machine: core.label.clone(),
                    release: core.release.clone(),
                }),
                crate::build_order::Standing::Unordered => {
                    Some(LinkFailure::Refused("other_build".to_owned()))
                }
            };
            if let Some(refused) = refused {
                herdr_core::diagnostic!(json!({
                    "component": "node_role",
                    "kind": "link.build_refused",
                    "core": placement.node,
                    "reason": refused.to_string(),
                    "node_build": identity.build,
                    "core_build": core.build,
                    "node_release": identity.release,
                    "core_release": core.release,
                }));
                return Err(refused);
            }
        }
        _ => {
            return Err(ended(
                "the core's machine answered another line than its core's".to_owned(),
            ));
        }
    }
    attach::write_line(
        &mut writer,
        &Line::Node(NodeHello {
            node: identity.node.clone(),
            label: identity.label.clone(),
            build: identity.build.clone(),
            herdr_socket: identity.herdr_socket.display().to_string(),
            release: identity.release.clone(),
            move_intent: placement.move_intent.clone(),
        }),
    )
    .map_err(LinkFailure::Transport)?;
    let accepted = match attach::read_line(&mut reader).map_err(handshake_failure)? {
        Line::Accepted(accepted) => accepted,
        Line::Refused(refusal) => return Err(LinkFailure::Refused(refusal.reason)),
        _ => {
            return Err(ended(
                "the core answered another line than its answer".to_owned(),
            ));
        }
    };
    reader
        .get_ref()
        .set_read_timeout(None)
        .map_err(|error| ended(error.to_string()))?;
    herdr_core::diagnostic!(json!({
        "component": "node_role",
        "kind": "link.accepted",
        "core": placement.node,
        "port": accepted.port,
    }));
    // The screens' way to the core: held for the link's life, and ended
    // with it.
    let forward = upstream.forward(accepted.port).map_err(forward_failure)?;
    let terminals = Arc::new(NodeTerminals::for_screen(
        Arc::clone(&shared.screen),
        identity.herdr_bin.clone(),
    ));
    let roots = match &shared.roots_changed {
        Some(changed) => {
            changed(&[]);
            let changed = Arc::clone(changed);
            hide_host::serve::OpenedRoots::telling(move |roots| changed(roots))
        }
        None => hide_host::serve::OpenedRoots::telling(|_| {}),
    };
    if set_phase(shared, Phase::Live(accepted.clone())) {
        return Err(LinkFailure::Stopping);
    }
    shared.live.send_replace(Some(Arc::new(LiveLink {
        generation,
        accepted,
        relay_port: forward.port(),
        terminals: Arc::clone(&terminals),
        upstream: Arc::clone(upstream),
    })));
    let browser = shared
        .browser
        .as_ref()
        .map(|browser| crate::node_browser::LinkBrowser {
            browser: Arc::clone(browser),
            generation,
        });
    let link_end = serve(
        reader,
        writer,
        identity,
        &terminals,
        &roots,
        browser
            .as_ref()
            .map(|browser| browser as &dyn hide_host::link_bridge::BrowserGateway),
    );
    shared.live.send_replace(None);
    if let Some(changed) = &shared.roots_changed {
        changed(&[]);
    }
    drop(forward);
    Ok(LinkFailure::Lost(match link_end {
        Ok(reason) | Err(reason) => reason,
    }))
}

/// Answers the core's calls until the link ends. The reader keeps what it
/// buffered past the handshake: the core's first call.
fn serve(
    reader: impl BufRead,
    writer: LocalStream,
    identity: &NodeIdentity,
    terminals: &NodeTerminals,
    roots: &hide_host::serve::OpenedRoots,
    browser: Option<&dyn hide_host::link_bridge::BrowserGateway>,
) -> Result<String, String> {
    hide_host::serve::serve_with(
        reader,
        writer,
        hide_host::serve::Services {
            terminals: Some(terminals),
            herdr_socket: Some(identity.herdr_socket.clone()),
            heartbeat: true,
            checkout_callers: true,
            opened_roots: Some(roots),
            browser,
            factory: true,
            worktree_facts: identity.worktree_facts.clone(),
            // The core keeps this machine's kit through the link, from the
            // app or the build this node runs (a development build
            // installs nothing).
            kit: std::env::current_exe().map_or(hide_host::serve::KitPlace::Standalone, |exe| {
                hide_host::serve::KitPlace::of_program(&exe)
            }),
        },
    )
    .map(|()| "link_closed".to_owned())
    .map_err(|error| format!("link_failed: {error}"))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addresses(last: u8) -> Option<BTreeSet<IpAddr>> {
        Some(BTreeSet::from([IpAddr::from([192, 168, 1, last])]))
    }

    /// A core that goes back to an older build after its update is updated
    /// once per connection, not once per dial: the second ask before a live
    /// link is the failed answer, and only a live link or a new connection
    /// gives the next update its try.
    #[test]
    fn an_update_that_does_not_hold_until_a_live_link_is_spent() {
        let mut update = Update::Untried;
        assert_eq!(update.attempt(), Ok(()));
        update = Update::Unproven;
        assert_eq!(
            update.attempt(),
            Err(Spent {
                reason: UPDATE_NOT_KEPT.to_owned(),
                now: true,
            })
        );
        assert_eq!(
            update.attempt(),
            Err(Spent {
                reason: UPDATE_NOT_KEPT.to_owned(),
                now: false,
            })
        );
        // A failure stays spent across a live link; a new connection ends it.
        update.held();
        assert!(update.attempt().is_err());

        let mut update = Update::Unproven;
        update.held();
        assert_eq!(update.attempt(), Ok(()));
    }

    #[test]
    fn a_sleep_or_a_new_address_set_is_seen_and_an_ordinary_look_is_not() {
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let monotonic = Instant::now();
        let mut look = Look::new(wall, monotonic, addresses(2));
        let step = WATCH_EVERY;
        assert_eq!(
            look.moved(wall + step, monotonic + step, addresses(2)),
            None
        );
        // The monotonic clock stood still for a minute the wall clock ran.
        let wall = wall + step + Duration::from_secs(60);
        let monotonic = monotonic + step * 2;
        assert_eq!(look.moved(wall, monotonic, addresses(2)), Some("slept"));
        let (wall, monotonic) = (wall + step, monotonic + step);
        assert_eq!(look.moved(wall, monotonic, addresses(3)), Some("network"));
        // An unreadable set says nothing and keeps the last one.
        let (wall, monotonic) = (wall + step, monotonic + step);
        assert_eq!(look.moved(wall, monotonic, None), None);
        let (wall, monotonic) = (wall + step, monotonic + step);
        assert_eq!(look.moved(wall, monotonic, addresses(3)), None);
        // A wall clock set back is not a sleep.
        assert_eq!(
            look.moved(
                wall - Duration::from_secs(3600),
                monotonic + step,
                addresses(3)
            ),
            None
        );
    }

    /// A screen opened while the link is down has drawn no core: the frame
    /// gives it the alias the node dials, and no machine name the core's
    /// hello has not given.
    #[test]
    fn a_held_screen_is_told_the_alias_the_core_is_dialed_by() {
        let frame = Phase::Waiting {
            reason: LinkFailure::Unreachable("connection refused".to_owned()),
        }
        .link_frame("mini")
        .unwrap();
        assert_eq!(
            frame["payload"],
            json!({"phase": "waiting", "machine": null, "alias": "mini"})
        );
        let frame = Phase::Disconnected.link_frame("mini").unwrap();
        assert_eq!(frame["payload"]["alias"], "mini");
    }

    /// The port is probed for a dial that could not reach the core's
    /// machine, whatever its text, and never for another failure whose text
    /// happens to read the same.
    #[test]
    fn only_an_unreachable_dial_probes_the_core_machine_s_port() {
        let waiting = |reason| Phase::Waiting { reason };
        let silent = PortWatch::default();
        assert!(probes_port(
            &waiting(LinkFailure::Unreachable("connection refused".to_owned())),
            &silent
        ));
        for reason in [
            LinkFailure::Refused("unreachable_by_policy".to_owned()),
            LinkFailure::Ended("unreachable: the link dropped".to_owned()),
            LinkFailure::Attach(AttachOutcome::NoCore),
            LinkFailure::Stopping,
        ] {
            assert!(
                !probes_port(&waiting(reason.clone()), &silent),
                "{reason:?}"
            );
        }
        let answered = PortWatch { last: Some(true) };
        assert!(!probes_port(
            &waiting(LinkFailure::Unreachable("connection refused".to_owned())),
            &answered
        ));
    }

    /// A move starts the next attempt at once only where it may change the
    /// answer: a refused node keeps its backoff however the network moves.
    #[test]
    fn a_sleep_or_network_move_cuts_short_only_a_wait_a_move_can_change() {
        let waiting = |reason| Phase::Waiting { reason };
        for reason in [
            LinkFailure::Unreachable("connection refused".to_owned()),
            LinkFailure::Lost("link_failed: closed".to_owned()),
        ] {
            assert!(wakes_on_move(&waiting(reason.clone()), None), "{reason:?}");
        }
        for reason in [
            LinkFailure::Refused("other_build".to_owned()),
            LinkFailure::Refused("nodes_full".to_owned()),
            LinkFailure::Refused("dialed_device".to_owned()),
            LinkFailure::Attach(AttachOutcome::NoCore),
            LinkFailure::WrongCore("another".to_owned()),
            LinkFailure::Ended("ssh: host_key_changed".to_owned()),
        ] {
            assert!(!wakes_on_move(&waiting(reason.clone()), None), "{reason:?}");
        }
        assert!(!wakes_on_move(&Phase::Connecting, None));
    }

    /// Only a connection that never reached the core machine's SSH server
    /// is unreachable, so only it is probed.
    #[test]
    fn only_a_dial_that_never_reached_ssh_is_unreachable() {
        for (error, _) in dial_outcomes() {
            let failure = attach_failure(error.clone());
            assert_eq!(
                matches!(failure, LinkFailure::Unreachable(_)),
                error.never_reached_server(),
                "{error}"
            );
            if !error.never_reached_server() {
                assert!(!failure.to_string().starts_with("unreachable"), "{failure}");
            }
        }
    }

    /// Dial failures as the SSH client writes them, and whether a move of
    /// the network may change each.
    fn dial_outcomes() -> Vec<(hide_node_link::device::RemoteError, bool)> {
        use hide_node_link::device::{
            DIAL_OPERATION, HOST_KEY_OPERATION, RemoteError, RemoteStage,
        };
        let error = |operation: &str, stage, reason: &str, retryable, action_required| {
            RemoteError::new(operation, "core", stage, reason, retryable, action_required)
        };
        vec![
            (
                error(
                    DIAL_OPERATION,
                    RemoteStage::Ssh,
                    "Connection refused",
                    true,
                    false,
                ),
                true,
            ),
            (
                error(
                    "remote-connect",
                    RemoteStage::Ssh,
                    "Connection reset by peer",
                    true,
                    false,
                ),
                true,
            ),
            (
                error(
                    "remote-connect",
                    RemoteStage::Ssh,
                    "SSH connection or authentication timed out",
                    true,
                    false,
                ),
                true,
            ),
            (
                error(
                    "node-attach",
                    RemoteStage::Ssh,
                    "channel open failure",
                    true,
                    false,
                ),
                true,
            ),
            (
                error(
                    HOST_KEY_OPERATION,
                    RemoteStage::Ssh,
                    "host key changed: ...",
                    false,
                    true,
                ),
                false,
            ),
            (
                error(
                    "remote-auth",
                    RemoteStage::Auth,
                    "no identity authenticated",
                    false,
                    true,
                ),
                false,
            ),
            (
                error(
                    "ssh-alias-import",
                    RemoteStage::Alias,
                    "unknown alias",
                    false,
                    true,
                ),
                false,
            ),
        ]
    }

    /// The screens' forward is judged as the dial is: a connection or port
    /// failure wakes on a move, a forward the operator must fix does not.
    #[test]
    fn a_forward_that_must_be_fixed_does_not_wake_on_a_move() {
        use hide_node_link::device::{RemoteError, RemoteStage};
        let forward = |reason: &str, retryable, action_required| {
            forward_failure(RemoteError::new(
                "workspace-browser-forward",
                "core",
                RemoteStage::Tunnel,
                reason,
                retryable,
                action_required,
            ))
        };
        let waiting = |reason| Phase::Waiting { reason };
        assert!(wakes_on_move(
            &waiting(forward("could not reserve a port", true, false)),
            None
        ));
        assert!(!wakes_on_move(
            &waiting(forward("remote endpoint must be loopback", false, true)),
            None
        ));
    }

    /// A move wakes a wait on what the network may have caused, and never
    /// one on what the operator or the core must change (D-09).
    #[test]
    fn a_move_wakes_a_transport_failure_but_not_what_must_be_fixed() {
        let waiting = |reason| Phase::Waiting { reason };
        for (error, wakes) in dial_outcomes() {
            assert_eq!(
                wakes_on_move(&waiting(attach_failure(error.clone())), None),
                wakes,
                "{error}"
            );
        }
        for (error, wakes) in [
            (
                attach::HandshakeError::Lost(
                    "the other side closed before the handshake".to_owned(),
                ),
                true,
            ),
            (
                attach::HandshakeError::Lost(
                    "the handshake could not be read: timed out".to_owned(),
                ),
                true,
            ),
            (
                attach::HandshakeError::Unreadable("the handshake line is too long".to_owned()),
                false,
            ),
        ] {
            assert_eq!(
                wakes_on_move(&waiting(handshake_failure(error.clone())), None),
                wakes,
                "{error}"
            );
        }
        for reason in [
            "other_build",
            "nodes_full",
            "dialed_device",
            "own_node",
            "already_linked",
        ] {
            assert!(!wakes_on_move(
                &waiting(LinkFailure::Refused(reason.to_owned())),
                None
            ));
        }
    }

    /// Link-local addresses come and go on their own; they are no move.
    #[test]
    fn link_local_addresses_coming_and_going_are_no_network_move() {
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let monotonic = Instant::now();
        let with = |extra: &[IpAddr]| {
            let mut set = addresses(2).unwrap();
            set.extend(extra.iter().copied());
            Some(set)
        };
        let mut look = Look::new(wall, monotonic, with(&[]));
        let step = WATCH_EVERY;
        let awdl: IpAddr = "fe80::1c2d:3eff:fe4f:5a6b".parse().unwrap();
        let auto: IpAddr = "169.254.10.20".parse().unwrap();
        assert_eq!(
            look.moved(wall + step, monotonic + step, with(&[awdl, auto])),
            None
        );
        let routable: IpAddr = "2001:db8::5".parse().unwrap();
        assert_eq!(
            look.moved(wall + step * 2, monotonic + step * 2, with(&[routable])),
            Some("network")
        );
    }

    /// Through the watch's looks: a port that answers during an unreachable
    /// outage is probed once and then left alone until the outage ends or
    /// the machine moves; a silent port is probed each look and wakes the
    /// node once when it answers.
    #[test]
    fn the_watch_probes_an_answering_port_once_per_outage() {
        let unreachable = Phase::Waiting {
            reason: LinkFailure::Unreachable("connection reset".to_owned()),
        };
        let mut port = PortProbe::default();
        let mut probes = 0;
        for _ in 0..10 {
            assert!(!port.look(&unreachable, || {
                probes += 1;
                true
            }));
        }
        assert_eq!(probes, 1, "an answering port is probed once per outage");

        port.moved();
        let mut probes = 0;
        for _ in 0..3 {
            port.look(&unreachable, || {
                probes += 1;
                true
            });
        }
        assert_eq!(probes, 1, "a move starts the probes over");

        // A link that lived ends the outage; the next one starts over.
        let lost = Phase::Waiting {
            reason: LinkFailure::Lost("link_closed".to_owned()),
        };
        assert!(!port.look(&lost, || unreachable!("not probed after a lost link")));
        let mut answers = [false, false, true, true].into_iter();
        let woke: Vec<bool> = (0..4)
            .map(|_| port.look(&unreachable, || answers.next().unwrap()))
            .collect();
        assert_eq!(
            woke,
            [false, false, true, false],
            "a new outage, a port that comes back"
        );
    }

    /// One unreachable wait after another is one outage: a port silent in
    /// the first wait and answering at the first look of the next wakes the
    /// node, however soon after the retry SSH came back.
    #[test]
    fn a_port_that_comes_back_between_two_unreachable_waits_wakes_the_node() {
        let unreachable = |cause: &str| Phase::Waiting {
            reason: LinkFailure::Unreachable(cause.to_owned()),
        };
        let mut port = PortProbe::default();
        assert!(!port.look(&unreachable("reset"), || false));
        assert!(!port.look(&Phase::Connecting, || unreachable!(
            "not probed while connecting"
        )));
        assert!(
            port.look(&unreachable("disconnected"), || true),
            "the port came back during the outage"
        );
    }

    #[test]
    fn only_a_port_that_was_silent_and_answers_again_wakes_the_node() {
        let mut port = PortWatch::default();
        assert!(!port.came_back(true), "a port answering from the start");
        assert!(!port.came_back(true));
        let mut port = PortWatch::default();
        assert!(!port.came_back(false));
        assert!(!port.came_back(false));
        assert!(port.came_back(true));
        assert!(!port.came_back(true), "once per return");
    }
}
