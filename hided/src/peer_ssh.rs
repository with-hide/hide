//! The SSH path of the connection two Hides keep (mission-107; security
//! spec, carriers): when no direct path opens, a host this machine keeps is
//! reached through the system `ssh` to an alias that signs in asking
//! nothing, and `hide device carry --ssh` there, whose input and output are
//! the carrier `hide_peer::pipe` runs the connection on. The machine-key
//! proof on the carrier decides who the far end is, as on a `wsl.exe`
//! carrier; the login is never trusted to say it.
//!
//! [`SshCarriers`] makes those carriers and keeps each host's route: ready
//! (its last attempt proved, or none was made since this daemon started),
//! waiting after a failure Hide retries by itself, due once that wait ran
//! out, or parked until the operator acts. Which failures are retried and
//! what the operator does about the rest are `SshProblem::retried` and
//! `SshProblem::action`, the one rule a device's row reads too. The state
//! lives in memory, so a daemon's start tries each route once (D6).
//!
//! Every sign-in it makes runs in one of [`limits::MAX_CARRIER_SIGN_INS`]
//! places of its own, never one of the operator's, and asks nobody (R1);
//! each attempt carries a stop that ends its `ssh` wherever it is, which
//! the route keeps beside itself, so forgetting a host or changing its alias
//! ends what still runs for it.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;

use hide_core::peer_machines::PeerSshBlocked;
use hide_node::ssh::session::{AttachChannel, SessionError, SshTarget};
use hide_node::ssh::upstream::Upstream;
use hide_node_link::device::{FailureAction, SshProblem};
use hide_peer::Route;
use hide_peer::limits;
use hide_peer::mux::Carrier;
use hide_peer::pipe::Routes;
use hide_peer::proof::Proof;
use hide_peer::protocol::PeerKey;
use hide_peer::store::StoreError;
use serde_json::{Value, json};
use tokio::sync::{Semaphore, watch};
use tokio::time::Instant;

use crate::pairing_ssh::{self, Reach};

fn lock<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    // Plain values and handles: whole whatever panicked beside them.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How long a carrier that ended before its proof waits for the last line
/// its far command wrote, to read why it refused.
const SAID_WITHIN: Duration = Duration::from_millis(500);

/// The most of a far line kept in the log.
const SAID_CAP: usize = 512;

/// What signs in to an alias, asking nobody, and starts `hide device carry
/// --ssh` in the start script's place. The system `ssh` in the daemon; a
/// stand-in in a test.
pub trait Signer: Send + Sync + 'static {
    /// Blocking. Hands `stop` what ends the attempt, before anything else
    /// that could wait.
    fn carry(&self, alias: &str, stop: &Stop) -> Result<AttachChannel, SessionError>;
}

/// The `ssh` a carrier's alias is dialed with: the account's own, forwarding
/// nothing of it (R7).
pub(crate) fn carrier_target(reach: &Reach, alias: &str) -> SshTarget {
    SshTarget::new(&reach.home, alias).forwarding_nothing()
}

/// The system `ssh`, as a device link signs in.
struct SystemSsh {
    reach: Reach,
}

impl Signer for SystemSsh {
    fn carry(&self, alias: &str, stop: &Stop) -> Result<AttachChannel, SessionError> {
        let upstream = Arc::new(Upstream::new(carrier_target(&self.reach, alias)));
        // R1: nobody pressed for it, so a question, an unknown server key or
        // a browser check ends it as a sign-in needed.
        upstream.may_ask(None);
        let ends = Arc::clone(&upstream);
        // Before the step, the sign-in's cancel; after it, the same cancel
        // still ends the `ssh` the carrier runs on.
        stop.set(move || ends.close());
        upstream.carry(|home, system| pairing_ssh::carry_argv(&self.reach, home, system))
    }
}

/// What ends one attempt wherever it is. Set once by the signer; a stop
/// asked for before it was set runs as it is set.
#[derive(Default)]
pub struct Stop {
    stopped: AtomicBool,
    end: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl Stop {
    pub fn set(&self, end: impl Fn() + Send + Sync + 'static) {
        *lock(&self.end) = Some(Box::new(end));
        if self.stopped.load(Ordering::SeqCst) {
            self.stop();
        }
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(end) = lock(&self.end).as_ref() {
            end();
        }
    }
}

/// Ends an attempt whose future was dropped before it had a carrier.
struct StopOnDrop(Option<Arc<Stop>>);

impl StopOnDrop {
    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            stop.stop();
        }
    }
}

/// What one attempt found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// The far end proved the host's key.
    Proved,
    /// The far end proved another key (R3).
    OtherKey,
    /// Nothing was proved (a signature that did not verify, or a proof the
    /// far end refused).
    NotProved,
    /// `ssh` did not sign in, as `openssh::classify` read it.
    Problem(SshProblem),
    /// The far account's Hide does not run, and is never started (R8).
    NotRunning,
    /// The far `hide` is too old to carry, or there is none.
    UpdateThere,
    /// The connection ended or stopped answering before a proof.
    Lost,
    /// The attempt reached its own bound with no outcome.
    TimedOut,
}

impl Outcome {
    fn of_session(error: &SessionError) -> Self {
        match error {
            SessionError::Ssh(failure) => Self::Problem(failure.problem),
            // The carry step's own refusal: the program it names is not
            // there.
            SessionError::Refused(reason) if reason.ends_with("is missing") => Self::UpdateThere,
            SessionError::Refused(_) => Self::Problem(SshProblem::Unsupported),
            SessionError::Lost(_) | SessionError::Local(_) => Self::Lost,
        }
    }

    fn of_proof(proof: Proof) -> Self {
        match proof {
            Proof::Proved => Self::Proved,
            Proof::OtherKey(_) => Self::OtherKey,
            Proof::NotProved => Self::NotProved,
        }
    }

    /// What a far `hide device carry` said as it refused, on its last line:
    /// `{"ok":false,"reason":…}` from this build, a usage line from one that
    /// knows no `--ssh`.
    fn of_far_line(line: &str) -> Option<Self> {
        if let Ok(said) = serde_json::from_str::<Value>(line) {
            return match said["reason"].as_str()? {
                "hide_not_running" => Some(Self::NotRunning),
                "ssh_update_there" | "hide_unavailable" => Some(Self::UpdateThere),
                _ => None,
            };
        }
        line.starts_with("hide device ")
            .then_some(Self::UpdateThere)
    }

    fn code(self) -> &'static str {
        match self {
            Self::Proved => "proved",
            Self::OtherKey => "other_key",
            Self::NotProved => "not_proved",
            Self::Problem(problem) => problem.code(),
            Self::NotRunning => "hide_not_running",
            Self::UpdateThere => "update_there",
            Self::Lost => "lost",
            Self::TimedOut => "timed_out",
        }
    }
}

/// The answer an add gives for a far `hide device carry` that refused, read
/// from its last line: `hide_not_running`, or `update_there` for one too old
/// to carry or that answered a usage line.
pub(crate) fn far_refusal(line: &str) -> Option<&'static str> {
    match Outcome::of_far_line(line)? {
        Outcome::NotRunning => Some("hide_not_running"),
        Outcome::UpdateThere => Some("update_there"),
        _ => None,
    }
}

fn blocked_of(action: FailureAction) -> PeerSshBlocked {
    match action {
        FailureAction::SignIn => PeerSshBlocked::SignIn,
        FailureAction::CheckHostKey => PeerSshBlocked::CheckHostKey,
        FailureAction::ReopenHide => PeerSshBlocked::ReopenHide,
        FailureAction::Retry => PeerSshBlocked::Retry,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ready,
    Waiting { until: Instant },
    Due,
    Parked(PeerSshBlocked),
}

/// One known host's SSH route.
struct Entry {
    alias: String,
    /// Taken from one counter that only rises: an attempt's outcome changes
    /// the entry only while the entry is still the one it was made under.
    epoch: u64,
    state: State,
    /// The wait the next failure starts.
    wait: Duration,
    /// The stops of the attempts that run for it.
    attempts: Vec<Weak<Stop>>,
    /// The build the host's last hello named, and the one it named when
    /// the route parked for its update.
    heard: Option<String>,
    parked_on: Option<String>,
}

impl Entry {
    fn new(alias: &str, epoch: u64) -> Self {
        Self {
            alias: alias.to_owned(),
            epoch,
            state: State::Ready,
            wait: limits::CLIMB_FIRST,
            attempts: Vec::new(),
            heard: None,
            parked_on: None,
        }
    }

    fn stop_attempts(&mut self) {
        for stop in self.attempts.drain(..).filter_map(|stop| stop.upgrade()) {
            stop.stop();
        }
    }

    /// The route waits, its wait doubling to [`limits::CLIMB_MOST`]. A
    /// parked route stays parked.
    fn wait(&mut self) {
        if matches!(self.state, State::Parked(_)) {
            return;
        }
        self.state = State::Waiting {
            until: Instant::now() + self.wait,
        };
        self.wait = (self.wait * 2).min(limits::CLIMB_MOST);
    }

    fn park(&mut self, blocked: PeerSshBlocked) {
        self.state = State::Parked(blocked);
        if blocked == PeerSshBlocked::UpdateThere {
            self.parked_on = self.heard.clone();
        }
    }

    /// The state now, a wait that ran out read as due.
    fn now(&mut self) -> State {
        if let State::Waiting { until } = self.state
            && Instant::now() >= until
        {
            self.state = State::Due;
        }
        self.state
    }
}

type TurnOff = Box<dyn Fn(&PeerKey, &str) -> Result<bool, StoreError> + Send + Sync>;

/// The SSH carriers of this daemon's connections, and each known host's
/// route.
pub struct SshCarriers {
    signer: Arc<dyn Signer>,
    /// The background sign-ins that run at once, apart from the operator's.
    places: Arc<Semaphore>,
    routes: Mutex<HashMap<PeerKey, Entry>>,
    epochs: AtomicU64,
    /// Saves a host's route as off, by compare-and-set on its alias: the
    /// peer records', handed in once they are open.
    turn_off: OnceLock<TurnOff>,
    /// Counts changes a row shows.
    changed: watch::Sender<u64>,
}

impl SshCarriers {
    /// The daemon's, signing in with the system `ssh` as `reach`'s account.
    pub fn new(reach: Reach) -> Arc<Self> {
        Self::with_signer(Arc::new(SystemSsh { reach }))
    }

    pub fn with_signer(signer: Arc<dyn Signer>) -> Arc<Self> {
        Arc::new(Self {
            signer,
            places: Arc::new(Semaphore::new(limits::MAX_CARRIER_SIGN_INS)),
            routes: Mutex::default(),
            epochs: AtomicU64::new(0),
            turn_off: OnceLock::new(),
            changed: watch::channel(0).0,
        })
    }

    /// Where a route turned off is saved: once, when the peer records open.
    pub fn store_in(
        &self,
        turn_off: impl Fn(&PeerKey, &str) -> Result<bool, StoreError> + Send + Sync + 'static,
    ) {
        let _ = self.turn_off.set(Box::new(turn_off));
    }

    /// Counts the changes a machine's row shows.
    pub fn changed(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn touched(&self) {
        self.changed.send_modify(|count| *count += 1);
    }

    fn epoch(&self) -> u64 {
        self.epochs.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// The entry of `key`'s route through `alias`: a new one, under a new
    /// epoch, when there was none or it named another alias, whose attempts
    /// then end.
    fn entry<'a>(
        &self,
        routes: &'a mut HashMap<PeerKey, Entry>,
        key: &PeerKey,
        alias: &str,
    ) -> &'a mut Entry {
        let fresh = routes.get(key).is_none_or(|entry| entry.alias != alias);
        if fresh {
            if let Some(mut earlier) = routes.remove(key) {
                earlier.stop_attempts();
            }
            routes.insert(*key, Entry::new(alias, self.epoch()));
        }
        routes.get_mut(key).expect("inserted above")
    }

    /// What `key`'s row says of its route: why it waits for the operator,
    /// when it does.
    pub fn blocked(&self, key: &PeerKey) -> Option<PeerSshBlocked> {
        match lock(&self.routes).get(key)?.state {
            State::Parked(blocked) => Some(blocked),
            State::Ready | State::Waiting { .. } | State::Due => None,
        }
    }

    /// The operator asked to retry the machine (`Ask::Retry`): its route is
    /// tried again by the next dial.
    pub fn retry(&self, key: &PeerKey) {
        let mut routes = lock(&self.routes);
        if let Some(entry) = routes.get_mut(key)
            && entry.state != State::Ready
        {
            entry.state = State::Ready;
            entry.wait = limits::CLIMB_FIRST;
            drop(routes);
            self.touched();
        }
    }

    /// The app opened, when a sign-in may ask: a route parked for one is
    /// tried again.
    pub fn app_opened(&self) {
        let mut lifted = false;
        for entry in lock(&self.routes).values_mut() {
            if entry.state == State::Parked(PeerSshBlocked::SignIn) {
                entry.state = State::Ready;
                lifted = true;
            }
        }
        if lifted {
            self.touched();
        }
    }

    /// `key`'s hello named `build`: a route parked for that machine's update
    /// is tried again once it names another build than when it parked.
    pub fn heard_build(&self, key: &PeerKey, build: &str) {
        let mut routes = lock(&self.routes);
        let Some(entry) = routes.get_mut(key) else {
            return;
        };
        entry.heard = Some(build.to_owned());
        if entry.state != State::Parked(PeerSshBlocked::UpdateThere) {
            return;
        }
        match entry.parked_on.as_deref() {
            None => entry.parked_on = Some(build.to_owned()),
            Some(parked_on) if parked_on != build => {
                entry.state = State::Ready;
                entry.parked_on = None;
                drop(routes);
                self.touched();
            }
            Some(_) => {}
        }
    }

    /// The hosts this machine keeps and the alias of each: a route whose
    /// host was forgotten, or whose alias changed or was turned off, goes,
    /// with what still runs for it.
    pub fn hosts(&self, kept: &[(PeerKey, Option<String>)]) {
        let mut routes = lock(&self.routes);
        let before = routes.len();
        routes.retain(|key, entry| {
            let still = kept
                .iter()
                .any(|(kept, alias)| kept == key && alias.as_deref() == Some(&entry.alias));
            if !still {
                entry.stop_attempts();
            }
            still
        });
        if routes.len() != before {
            drop(routes);
            self.touched();
        }
    }

    /// Makes the carrier to `key`'s machine through `alias`: refused at once,
    /// with no `ssh`, while the route waits or is parked; otherwise one
    /// sign-in in a place of the carriers', which ends at the caller's
    /// bound (`limits::CARRIER_WITHIN`, the attempt's own) when the future
    /// is dropped.
    pub fn carrier(
        self: &Arc<Self>,
        key: PeerKey,
        alias: String,
    ) -> impl Future<Output = io::Result<Carrier>> + Send + 'static + use<> {
        let carriers = Arc::clone(self);
        async move {
            let stop = Arc::new(Stop::default());
            let epoch = {
                let mut routes = lock(&carriers.routes);
                let entry = carriers.entry(&mut routes, &key, &alias);
                match entry.now() {
                    State::Ready | State::Due => {}
                    State::Waiting { .. } => {
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            "the SSH route waits after a failure",
                        ));
                    }
                    State::Parked(blocked) => {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            format!("the SSH route is parked ({blocked:?})"),
                        ));
                    }
                }
                entry.attempts.retain(|stop| stop.strong_count() > 0);
                entry.attempts.push(Arc::downgrade(&stop));
                entry.epoch
            };
            let attempt = Arc::new(Attempt {
                carriers: Arc::downgrade(&carriers),
                key,
                alias: alias.clone(),
                epoch,
                started: Instant::now(),
                settled: AtomicBool::new(false),
                said: AtomicBool::new(false),
                stop: Arc::clone(&stop),
            });
            // Ends the sign-in when this future is dropped, wherever it is.
            let guard = StopOnDrop(Some(Arc::clone(&stop)));
            let place = Arc::clone(&carriers.places)
                .acquire_owned()
                .await
                .map_err(io::Error::other)?;
            let (signer, signing) = (Arc::clone(&carriers.signer), Arc::clone(&attempt));
            let signed = tokio::task::spawn_blocking(move || {
                let _place = place;
                signer.carry(&signing.alias, &signing.stop)
            })
            .await
            .map_err(io::Error::other)?;
            match signed {
                Ok(channel) => {
                    guard.disarm();
                    Ok(carrier_of(channel, attempt))
                }
                Err(error) => {
                    attempt.settle(Outcome::of_session(&error));
                    Err(io::Error::other(error.to_string()))
                }
            }
        }
    }

    /// What attempt `epoch` of `key`'s route through `alias` found: it
    /// changes the route only while the route is still that one.
    fn settled(&self, key: &PeerKey, alias: &str, epoch: u64, outcome: Outcome) {
        hide_core::diagnostic!(json!({
            "component": "pairing",
            "kind": "ssh.carrier_dial",
            "peer": key.short(),
            "alias": alias,
            "class": outcome.code(),
        }));
        if outcome == Outcome::OtherKey {
            return self.refused_key(key, alias, epoch);
        }
        let mut routes = lock(&self.routes);
        let Some(entry) = routes.get_mut(key).filter(|entry| entry.epoch == epoch) else {
            return;
        };
        let before = entry.state;
        match outcome {
            Outcome::Proved => {
                entry.state = State::Ready;
                entry.wait = limits::CLIMB_FIRST;
            }
            Outcome::OtherKey => unreachable!("answered above"),
            Outcome::NotProved => entry.park(PeerSshBlocked::Retry),
            Outcome::Problem(problem) if problem.retried() => entry.wait(),
            Outcome::Problem(problem) => entry.park(blocked_of(problem.action())),
            Outcome::NotRunning => {
                entry.wait = limits::CLIMB_MOST;
                entry.wait();
            }
            Outcome::UpdateThere => entry.park(PeerSshBlocked::UpdateThere),
            Outcome::Lost | Outcome::TimedOut => entry.wait(),
        }
        let after = entry.state;
        drop(routes);
        if let State::Parked(blocked) = after
            && before != after
        {
            hide_core::diagnostic!(json!({
                "component": "pairing",
                "kind": "peer.ssh_parked",
                "peer": key.short(),
                "blocked": blocked,
            }));
        }
        if before != after {
            self.touched();
        }
    }

    /// The alias reached a Hide that proved another key: the route is turned
    /// off and saved, by compare-and-set on the alias the attempt dialed, so
    /// an attempt older than the route writes nothing (R3). A save that
    /// fails parks the route, so the row says it and nothing is dialed until
    /// a retry or a restart tries the save again.
    fn refused_key(&self, key: &PeerKey, alias: &str, epoch: u64) {
        let current = lock(&self.routes)
            .get(key)
            .is_some_and(|entry| entry.epoch == epoch);
        if !current {
            return;
        }
        hide_core::diagnostic!(json!({
            "component": "pairing",
            "kind": "peer.ssh_refused_key",
            "peer": key.short(),
        }));
        let saved = match self.turn_off.get() {
            Some(turn_off) => turn_off(key, alias),
            None => Err(StoreError::Io(io::Error::other(
                "the peer records are not open",
            ))),
        };
        let mut routes = lock(&self.routes);
        let Some(entry) = routes.get_mut(key).filter(|entry| entry.epoch == epoch) else {
            return;
        };
        match saved {
            // Off: the host names no alias any more, and a new one recorded
            // later starts a new route.
            Ok(true) => {
                routes.remove(key);
            }
            // The route changed since the attempt: a newer alias stays.
            Ok(false) => {}
            Err(error) => {
                entry.park(PeerSshBlocked::NotSaved);
                hide_core::diagnostic!(json!({
                    "component": "pairing",
                    "kind": "peer.ssh_off_unsaved",
                    "peer": key.short(),
                    "reason": error.to_string(),
                }));
            }
        }
        drop(routes);
        self.touched();
    }
}

impl Routes for SshCarriers {
    fn route(&self, key: &PeerKey, alias: &str) -> Route {
        let mut routes = lock(&self.routes);
        match self.entry(&mut routes, key, alias).now() {
            State::Ready => Route::Ready,
            State::Due => Route::Due,
            State::Waiting { .. } | State::Parked(_) => Route::Held,
        }
    }

    fn fell(&self, key: &PeerKey) {
        if let Some(entry) = lock(&self.routes).get_mut(key) {
            entry.wait();
        }
    }

    fn network_changed(&self) {
        for entry in lock(&self.routes).values_mut() {
            if matches!(entry.state, State::Waiting { .. } | State::Due) {
                entry.state = State::Due;
                entry.wait = limits::CLIMB_FIRST;
            }
        }
    }
}

/// One attempt, held by its sign-in and then by its carrier. It settles its
/// route once: by the proof, by why the sign-in or the far command refused,
/// or, dropped at or past its own bound with nothing else said, as a wait.
struct Attempt {
    carriers: Weak<SshCarriers>,
    key: PeerKey,
    alias: String,
    epoch: u64,
    started: Instant,
    settled: AtomicBool,
    /// The far command's last line was read once.
    said: AtomicBool,
    stop: Arc<Stop>,
}

impl Attempt {
    fn settle(&self, outcome: Outcome) {
        if self.settled.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(carriers) = self.carriers.upgrade() {
            carriers.settled(&self.key, &self.alias, self.epoch, outcome);
        }
    }

    /// The carrier ended: what the far command and `ssh` wrote last goes to
    /// the log, and a far refusal settles an attempt nothing settled yet.
    /// Off the thread that ended it, which is the connection's.
    fn carrier_ended(self: &Arc<Self>, said: &Arc<Mutex<Vec<u8>>>) {
        if self.said.swap(true, Ordering::AcqRel) {
            return;
        }
        let (attempt, said) = (Arc::clone(self), Arc::clone(said));
        let _ = std::thread::Builder::new()
            .name("peer-ssh-said".to_owned())
            .spawn(move || {
                let over = std::time::Instant::now() + SAID_WITHIN;
                let last = loop {
                    let last = last_line(&lock(&said));
                    let refusal = last.as_deref().and_then(Outcome::of_far_line);
                    if refusal.is_some()
                        || attempt.settled.load(Ordering::Acquire)
                        || std::time::Instant::now() >= over
                    {
                        break last;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                };
                if let Some(line) = &last {
                    let line = hide_node::ssh::openssh::kept_line(line);
                    hide_core::diagnostic!(json!({
                        "component": "pairing",
                        "kind": "ssh.carrier_said",
                        "peer": attempt.key.short(),
                        "message": line.chars().take(SAID_CAP).collect::<String>(),
                    }));
                }
                if let Some(refusal) = last.as_deref().and_then(Outcome::of_far_line) {
                    attempt.settle(refusal);
                }
            });
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        // Its own bound ended it with nothing said: the route waits, so the
        // climb does not sign in again at the next grace. One dropped
        // sooner (another path was chosen, the daemon stops) changes
        // nothing.
        if !self.settled.load(Ordering::Acquire) && self.started.elapsed() >= limits::CARRIER_WITHIN
        {
            self.settle(Outcome::TimedOut);
        }
    }
}

/// The last line with words in `said`.
fn last_line(said: &[u8]) -> Option<String> {
    String::from_utf8_lossy(said)
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_owned())
}

/// The carrier an attach channel is: its end ends `ssh` and reads what the
/// far command said; the proof's outcome settles the attempt.
fn carrier_of(channel: AttachChannel, attempt: Arc<Attempt>) -> Carrier {
    let AttachChannel { stream, stderr } = channel;
    let read = stream.duplicate();
    let end = stream.shutdown_handle();
    let told = Arc::clone(&attempt);
    Carrier::new(read, stream, move || {
        end.shutdown();
        attempt.stop.stop();
        attempt.carrier_ended(&stderr);
    })
    .told(move |proof| told.settle(Outcome::of_proof(proof)))
}

#[cfg(test)]
#[path = "peer_ssh_tests.rs"]
mod tests;
