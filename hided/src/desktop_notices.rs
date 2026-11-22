//! The desktop app's agent notifications (PRD first-run-onboarding D-09,
//! B31-B38): the phone push's notification transitions
//! (`hide_core::agent_state::push::Transitions`) run a second time for the
//! desktop windows, independent of the Mobile setting.
//!
//! They judge the projection the mobile follower already reads off the core's
//! lock (`mobile::Mobile::follow`), so a burst serializes one snapshot however
//! many consumers there are, and they run only while a computer could show
//! one: a desktop window has the switch on, which each window says for
//! itself like any observation hint (`demand.rs`), or a computer that may
//! open this machine is connected. The first projection after that seeds
//! them, so nothing that was already waiting when the app opened is
//! announced (B33). A judgment that announces or clears something is one
//! `agent_notice` frame for the desktop windows; whether a notification
//! shows, and in what words, is the desktop host's and the window's to
//! decide (D-10, B39).
//!
//! Where a notice goes is the core's rule (`hide_core::notice_place`, PRD
//! device-connect-iroh D-60, D-78): this module says what it observed of
//! this machine's seat, holds the seat each computer that may open this
//! machine told of itself, and takes the answer. A new notice reaches the
//! windows only while this computer is the place, which is while someone
//! uses it; a computer that is on and not in use gets none (B59), and the
//! phones rest exactly while a computer is the place. While another
//! computer is the place the notice is handed to it on the `core` stream
//! (`peer_core`), and it says whether it showed it. One that says no, or
//! nothing within `notice_place::TAKEN_WITHIN`, is at no seat until it
//! tells its seat again, and the notice goes to the place that is next
//! then (B71).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use hide_core::agent_state::phone::{AgentKey, Projection};
use hide_core::agent_state::push::{Notice, NoticeState, Transitions};
use hide_core::notice_place::{self, COMPUTER_WINS_WITHIN, Peer, Place, Seat, TAKEN_WITHIN};
use hide_peer::protocol::core::{self as wire, CoreMessage};
use serde_json::{Value, json};
use tokio::sync::{Notify, broadcast, watch};

use crate::demand::ObservationDemand;

/// Frames a window may fall behind by before it misses one; a missed frame
/// costs one notification, never a wrong one.
const FRAME_BACKLOG: usize = 16;

/// The computers whose seat this machine holds at once: the connected ones
/// that may open it. One more is refused and recorded.
const MAX_COMPUTERS: usize = 16;

/// The notices handed to other computers this machine remembers at once,
/// waiting for their answer or shown there. Past it a new one is not
/// handed over and goes to the phones.
pub(crate) const MAX_HANDED: usize = 256;

/// A seat that reads as it did is told again only after this long: a
/// computer in use says so when input resumes after a pause and never per
/// input (D-60, the desktop host's five-second rule).
const SEAT_PAUSE: Duration = Duration::from_secs(5);

/// What a window words of a notice another machine handed over (the
/// `agent_notice` frame, `contracts/hided-ws.schema.json`): the status
/// codes it knows and the longest detail it takes. A code a later build of
/// that machine sends beyond them is left out and the window says the
/// section; a longer detail is cut.
const STATUS_CODES: [&str; 16] = [
    "error",
    "question",
    "approval",
    "working",
    "done",
    "idle",
    "unknown",
    "starting",
    "waiting",
    "unfinished",
    "stopped",
    "attached",
    "sleeping",
    "waking",
    "sleep_failed",
    "look",
];
const DETAIL_CHARS: usize = 80;

/// Where another computer is written to: its `core` stream.
pub trait Computers: Send + Sync {
    /// Whether `message` was handed to the stream of the computer under
    /// `key`; false when it has none now or the stream is full.
    fn tell(&self, key: &str, message: CoreMessage) -> bool;
}

pub struct DesktopNotices {
    /// The desktop windows whose switch is on.
    demand: ObservationDemand,
    wanted: AtomicBool,
    computer_use: Mutex<ComputerUse>,
    resting: AtomicBool,
    waiting: ObservationDemand,
    waiting_wanted: AtomicBool,
    transitions: Mutex<Transitions>,
    frames: broadcast::Sender<Arc<str>>,
    /// Set once, before a machine connects.
    computers: OnceLock<Arc<dyn Computers>>,
    /// Taken after `computer_use`, never before it.
    handed: Mutex<Handed>,
    /// Counts the changes of this computer's seat worth telling.
    seat_moves: watch::Sender<u64>,
    /// Wakes the follower that judges: a computer that may open this
    /// machine came or went, or a notice handed to one is the next place's.
    judge: Notify,
}

/// Only receipt times on this daemon, never a clock supplied by a phone or
/// by another computer. The existing one-second host poll cannot order
/// inputs within one second: the computer wins that interval so the
/// ambiguous case stays quiet.
#[derive(Default)]
struct ComputerUse {
    hosts: HashSet<u64>,
    computer: Option<Instant>,
    phone: Option<Instant>,
    /// What each computer that may open this machine last told of its
    /// seat, and when it arrived.
    peers: HashMap<String, (Seat, Instant)>,
    /// What this machine last told of its own seat, and when.
    told: Option<(SeatKind, Instant)>,
}

/// What of a seat decides where a notice goes: in use, shows the
/// operator's turn, and a phone acted after the last input.
type SeatKind = (bool, bool, bool);

impl ComputerUse {
    /// This machine's seat as the place rule reads it: lengths of time
    /// measured back from `now`, or from the latest receipt when that is
    /// later, so two receipts are never compared through a clamped zero.
    fn seat(&self, shows: bool, now: Instant) -> Seat {
        let now = [Some(now), self.computer, self.phone]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or(now);
        let ago = |at: Instant| now.saturating_duration_since(at);
        Seat {
            input: (!self.hosts.is_empty()).then(|| self.computer.map_or(Duration::ZERO, ago)),
            shows,
            phone: self.phone.map(ago),
        }
    }

    /// The places a notification made at `now` is tried at, with a phone
    /// counted as paired: whether one is, is the phone side's to know. What
    /// another computer told is as much older as the time since it arrived.
    fn places(&self, shows: bool, now: Instant) -> Vec<Place> {
        let peers: Vec<Peer> = self
            .peers
            .iter()
            .map(|(key, (seat, at))| {
                let since = now.saturating_duration_since(*at);
                Peer {
                    key: key.clone(),
                    seat: Seat {
                        input: seat.input.map(|input| input.saturating_add(since)),
                        shows: seat.shows,
                        phone: seat.phone.map(|phone| phone.saturating_add(since)),
                    },
                }
            })
            .collect();
        notice_place::places(self.seat(shows, now), &peers, true)
    }

    /// The computer under `key` is at no seat until it tells its seat again.
    fn unseat(&mut self, key: &str) {
        if let Some((seat, _)) = self.peers.get_mut(key) {
            seat.input = None;
        }
    }
}

/// The notices handed to other computers.
#[derive(Default)]
struct Handed {
    next: u64,
    /// Sent to a computer that has not said yet whether it showed it.
    waiting: HashMap<u64, Waiting>,
    /// Shown by a computer, by tag: the one to tell when the notice is over.
    shown: HashMap<String, String>,
}

struct Waiting {
    key: String,
    notice: Notice,
    due: Instant,
}

/// Where one judgment's notices went, apart from another computer.
#[derive(Default)]
struct Placed {
    here: Vec<Notice>,
    /// The roots whose notice is the phones' now.
    phone: Vec<AgentKey>,
}

impl Default for DesktopNotices {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopNotices {
    pub fn new() -> Self {
        Self {
            demand: ObservationDemand::default(),
            wanted: AtomicBool::new(false),
            computer_use: Mutex::default(),
            resting: AtomicBool::new(false),
            waiting: ObservationDemand::default(),
            waiting_wanted: AtomicBool::new(false),
            transitions: Mutex::default(),
            frames: broadcast::channel(FRAME_BACKLOG).0,
            computers: OnceLock::new(),
            handed: Mutex::default(),
            seat_moves: watch::channel(0).0,
            judge: Notify::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Transitions> {
        self.transitions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn handed(&self) -> std::sync::MutexGuard<'_, Handed> {
        self.handed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// One desktop window says whether its switch is on. True when the
    /// windows as a whole just turned it on, so the follower must wake.
    pub fn set(&self, connection: u64, wanted: bool) -> bool {
        self.demand.set(connection, wanted, |on| self.turned(on)) == Some(true)
    }

    /// A host reports a use transition, connection replay, or input resumed
    /// after a five-second pause. No per-keystroke work or snapshot publish.
    pub fn activity(&self, connection: u64, in_use: bool) {
        self.activity_at(connection, in_use, Instant::now());
    }

    pub(crate) fn activity_at(&self, connection: u64, in_use: bool, at: Instant) {
        let mut use_state = self.computer_use.lock().expect("computer use");
        if in_use {
            use_state.hosts.insert(connection);
            use_state.computer = Some(use_state.computer.map_or(at, |old| old.max(at)));
        } else {
            use_state.hosts.remove(&connection);
        }
        self.seat_changed(&mut use_state, at);
    }

    /// Called only for a newly accepted phone action, including a device pane.
    pub fn phone_input(&self, at: Instant) {
        let mut use_state = self.computer_use.lock().expect("computer use");
        use_state.phone = Some(use_state.phone.map_or(at, |old| old.max(at)));
        self.seat_changed(&mut use_state, at);
    }

    /// This machine's seat moved: the phones' rest follows, and the
    /// machines this computer may open are told when it reads differently
    /// or [`SEAT_PAUSE`] passed since they were.
    fn seat_changed(&self, use_state: &mut ComputerUse, now: Instant) {
        self.update_rest(use_state, now);
        let seat = use_state.seat(self.waiting_wanted.load(Ordering::SeqCst), now);
        let phone_later = match (seat.input, seat.phone) {
            (Some(input), Some(phone)) => input > phone + COMPUTER_WINS_WITHIN,
            (None, phone) => phone.is_some(),
            (Some(_), None) => false,
        };
        let kind = (seat.input.is_some(), seat.shows, phone_later);
        if use_state.told.is_some_and(|(told, at)| {
            told == kind && now.saturating_duration_since(at) < SEAT_PAUSE
        }) {
            return;
        }
        use_state.told = Some((kind, now));
        self.seat_moves.send_modify(|moves| *moves += 1);
    }

    /// The phones rest while a computer whose window shows the operator's
    /// turn is the place (`notice_place::phone_rests`).
    fn update_rest(&self, use_state: &ComputerUse, now: Instant) {
        let shows = self.waiting_wanted.load(Ordering::SeqCst);
        let resting = notice_place::phone_rests(&use_state.places(shows, now));
        if self.resting.swap(resting, Ordering::SeqCst) != resting {
            hide_core::diagnostic!(json!({
                "component": "mobile_push", "kind": "phone.rest_changed", "resting": resting,
            }));
        }
    }

    pub fn set_waiting(&self, connection: u64, wanted: bool) {
        self.waiting.set(connection, wanted, |on| {
            let mut use_state = self.computer_use.lock().expect("computer use");
            self.waiting_wanted.store(on, Ordering::SeqCst);
            self.seat_changed(&mut use_state, Instant::now());
        });
    }

    /// Whether a notification made now goes to this machine's windows: a
    /// window has notifications on and this computer is the place.
    fn takes_here(&self) -> bool {
        let use_state = self.computer_use.lock().expect("computer use");
        use_state.places(self.wanted(), Instant::now()).first() == Some(&Place::Here)
    }

    /// Resting must leave at least one desktop able to announce a person's turn.
    pub fn rests_phone(&self) -> bool {
        self.resting.load(Ordering::SeqCst)
    }

    /// A closed window takes its demand with it.
    pub fn release(&self, connection: u64) {
        self.demand.release(connection, |on| self.turned(on));
        self.activity(connection, false);
        self.set_waiting(connection, false);
    }

    /// The windows' switch turned. The transitions start over unless they
    /// already run for a computer that may open this machine.
    fn turned(&self, on: bool) {
        self.wanted.store(on, Ordering::SeqCst);
        if !self.has_computers() {
            self.lock().reset();
        }
        hide_core::diagnostic!(json!({
            "component": "desktop_notices", "kind": "desktop_notices.demand", "on": on,
        }));
    }

    pub fn wanted(&self) -> bool {
        self.wanted.load(Ordering::SeqCst)
    }

    /// Whether this machine holds the seat of a computer that may open it.
    pub(crate) fn has_computers(&self) -> bool {
        !self
            .computer_use
            .lock()
            .expect("computer use")
            .peers
            .is_empty()
    }

    /// Whether a computer could show a notice of this machine's work: a
    /// window here has the switch on, or a computer that may open this
    /// machine is connected. The follower reads the core while it holds.
    pub fn judging(&self) -> bool {
        self.wanted() || self.has_computers()
    }

    /// Whether the transitions run and no projection has seeded them since.
    pub fn seed_pending(&self) -> bool {
        self.judging() && !self.lock().seeded()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<str>> {
        self.frames.subscribe()
    }

    /// Completes when the follower has something new to judge.
    pub async fn judge_wanted(&self) {
        self.judge.notified().await;
    }

    /// Where the other computers are written to. Called once, at start.
    pub fn set_computers(&self, computers: Arc<dyn Computers>) {
        let _ = self.computers.set(computers);
    }

    fn tell(&self, key: &str, message: CoreMessage) -> bool {
        self.computers
            .get()
            .is_some_and(|computers| computers.tell(key, message))
    }

    /// This computer's seat as it tells the machines it may open.
    pub fn seat(&self) -> Seat {
        let use_state = self.computer_use.lock().expect("computer use");
        use_state.seat(self.waiting_wanted.load(Ordering::SeqCst), Instant::now())
    }

    /// Moves whenever [`Self::seat`] is worth telling again.
    pub fn seat_moves(&self) -> watch::Receiver<u64> {
        self.seat_moves.subscribe()
    }

    /// The computer under `key`, which may open this machine, told its seat.
    pub fn peer_seat(&self, key: &str, seat: Seat) {
        let now = Instant::now();
        let first = {
            let mut use_state = self.computer_use.lock().expect("computer use");
            if !use_state.peers.contains_key(key) && use_state.peers.len() >= MAX_COMPUTERS {
                hide_core::diagnostic!(json!({
                    "component": "desktop_notices", "kind": "desktop_notices.computers_capped",
                    "peer": key, "cap": MAX_COMPUTERS,
                }));
                return;
            }
            let first = use_state.peers.is_empty();
            use_state.peers.insert(key.to_owned(), (seat, now));
            self.update_rest(&use_state, now);
            first
        };
        if first {
            // The first projection after this seeds: what already waits is
            // not announced to a computer that just connected.
            if !self.wanted() {
                self.lock().reset();
            }
            self.judge.notify_one();
        }
    }

    /// The computer under `key` may open this machine no longer, or left:
    /// what it was sent and has not answered is the next place's now.
    pub fn peer_gone(&self, key: &str) {
        let now = Instant::now();
        let last = {
            let mut use_state = self.computer_use.lock().expect("computer use");
            if use_state.peers.remove(key).is_none() {
                return;
            }
            self.update_rest(&use_state, now);
            let mut handed = self.handed();
            for waiting in handed.waiting.values_mut() {
                if waiting.key == key {
                    waiting.due = now;
                }
            }
            handed.shown.retain(|_, computer| computer != key);
            use_state.peers.is_empty()
        };
        if last && !self.wanted() {
            self.lock().reset();
        }
        self.judge.notify_one();
    }

    /// Judges one projection. A change worth a notification goes to its one
    /// place: this machine's windows as one frame, another computer on its
    /// stream, or the phones, whose roots are returned. One that clears a
    /// notification shown before (B35) closes it wherever it was shown.
    pub fn observe(&self, projection: &Projection, at: Instant) -> Vec<AgentKey> {
        let (notices, seen) = self.lock().observe(projection);
        let mut placed = Placed::default();
        if !notices.is_empty() {
            let mut use_state = self.computer_use.lock().expect("computer use");
            let mut handed = self.handed();
            for notice in notices {
                self.place(&mut use_state, &mut handed, notice, at, &mut placed);
            }
            self.update_rest(&use_state, at);
        }
        self.over(&seen);
        self.frame(&placed.here, &seen);
        placed.phone
    }

    /// Sends `notice` to the one place it goes now (D-60), which in a build
    /// without connections between two Hides is always this machine's
    /// windows. The transition is spent either way: a notice this computer
    /// did not take is another place's, never shown here late. A computer
    /// that cannot be written to is at no seat, and the place that is next
    /// then is tried.
    fn place(
        &self,
        use_state: &mut ComputerUse,
        handed: &mut Handed,
        notice: Notice,
        at: Instant,
        placed: &mut Placed,
    ) {
        let tag = notice.key.tag();
        let record = |place: &str, peer: Option<&str>| {
            hide_core::diagnostic!(json!({
                "component": "desktop_notices", "kind": "desktop_notice.not_here",
                "agent": tag, "place": place, "peer": peer,
            }));
        };
        loop {
            let key = match use_state.places(self.wanted(), at).into_iter().next() {
                Some(Place::Here) => {
                    placed.here.push(notice);
                    return;
                }
                Some(Place::Peer(key)) => key,
                Some(Place::Phone) => {
                    record("phone", None);
                    placed.phone.push(notice.key);
                    return;
                }
                None => {
                    record("none", None);
                    return;
                }
            };
            let Some(message) = wire_notice(&notice) else {
                record("none", Some(&key));
                return;
            };
            if handed.waiting.len() + handed.shown.len() >= MAX_HANDED {
                hide_core::diagnostic!(json!({
                    "component": "desktop_notices", "kind": "desktop_notice.handed_capped",
                    "agent": tag, "cap": MAX_HANDED,
                }));
                placed.phone.push(notice.key);
                return;
            }
            let id = handed.next;
            handed.next += 1;
            if self.tell(
                &key,
                CoreMessage::Notice {
                    id,
                    notice: message,
                },
            ) {
                record("computer", Some(&key));
                handed.waiting.insert(
                    id,
                    Waiting {
                        key,
                        notice,
                        due: at + TAKEN_WITHIN,
                    },
                );
                return;
            }
            use_state.unseat(&key);
        }
    }

    /// The computer under `key` said whether it showed notice `id`. One
    /// that did not is the next place's at the follower's next pass.
    pub fn answered(&self, key: &str, id: u64, taken: bool) {
        {
            let mut handed = self.handed();
            if handed
                .waiting
                .get(&id)
                .is_none_or(|waiting| waiting.key != key)
            {
                return;
            }
            if taken {
                if let Some(waiting) = handed.waiting.remove(&id) {
                    handed.shown.insert(waiting.notice.key.tag(), waiting.key);
                }
                return;
            }
            if let Some(waiting) = handed.waiting.get_mut(&id) {
                waiting.due = Instant::now();
            }
        }
        self.judge.notify_one();
    }

    /// When the next handed notice stops waiting for its answer.
    pub fn next_due(&self) -> Option<Instant> {
        self.handed()
            .waiting
            .values()
            .map(|waiting| waiting.due)
            .min()
    }

    /// The handed notices whose computer did not say by `at` that it showed
    /// them (B71): that computer is at no seat until it tells its seat
    /// again, and each notice goes to the place that is next now. Returns
    /// the roots whose notice is the phones'.
    pub fn expire(&self, at: Instant) -> Vec<AgentKey> {
        let mut placed = Placed::default();
        {
            let mut use_state = self.computer_use.lock().expect("computer use");
            let mut handed = self.handed();
            let mut due: Vec<u64> = handed
                .waiting
                .iter()
                .filter(|(_, waiting)| waiting.due <= at)
                .map(|(id, _)| *id)
                .collect();
            if due.is_empty() {
                return Vec::new();
            }
            due.sort_unstable();
            for id in due {
                let Some(waiting) = handed.waiting.remove(&id) else {
                    continue;
                };
                hide_core::diagnostic!(json!({
                    "component": "desktop_notices", "kind": "desktop_notice.not_taken",
                    "agent": waiting.notice.key.tag(), "peer": waiting.key,
                }));
                use_state.unseat(&waiting.key);
                self.place(&mut use_state, &mut handed, waiting.notice, at, &mut placed);
            }
            self.update_rest(&use_state, at);
        }
        self.frame(&placed.here, &BTreeSet::new());
        placed.phone
    }

    /// The notices of `seen` are over: each computer that showed one, or
    /// was sent one, is told to close it.
    fn over(&self, seen: &BTreeSet<AgentKey>) {
        let mut closing: HashMap<String, Vec<String>> = HashMap::new();
        {
            let mut handed = self.handed();
            if handed.waiting.is_empty() && handed.shown.is_empty() {
                return;
            }
            for key in seen {
                let tag = key.tag();
                if let Some(computer) = handed.shown.remove(&tag) {
                    closing.entry(computer).or_default().push(tag.clone());
                }
                handed.waiting.retain(|_, waiting| {
                    if waiting.notice.key != *key {
                        return true;
                    }
                    closing
                        .entry(waiting.key.clone())
                        .or_default()
                        .push(tag.clone());
                    false
                });
            }
        }
        for (computer, tags) in closing {
            for tags in tags.chunks(wire::MAX_CLEAR_TAGS) {
                self.tell(
                    &computer,
                    CoreMessage::NoticeClear {
                        tags: tags.to_vec(),
                    },
                );
            }
        }
    }

    /// One `agent_notice` frame for this machine's windows.
    fn frame(&self, notices: &[Notice], seen: &BTreeSet<AgentKey>) {
        if notices.is_empty() && seen.is_empty() {
            return;
        }
        hide_core::diagnostic!(json!({
            "component": "desktop_notices", "kind": "desktop_notice.judged",
            "agents": notices.iter().map(|notice| notice.key.tag()).collect::<Vec<_>>(),
            "cleared": seen.iter().map(AgentKey::tag).collect::<Vec<_>>(),
        }));
        let frame = frame(notices, seen.iter());
        // No window listening is no notification, which is the answer.
        let _ = self.frames.send(Arc::from(frame.to_string()));
    }

    /// A notice the machine under `key`, which this computer may open,
    /// handed over. It is shown, and the answer is yes, only while someone
    /// is at this computer and a window here has notifications on.
    pub fn show(&self, key: &str, notice: &wire::Notice) -> bool {
        if !notice.valid() || !self.takes_here() {
            return false;
        }
        let tag = peer_tag(key, &notice.tag());
        // What this build's window cannot word is left out.
        let subject = notice
            .subject
            .clone()
            .and_then(hide_core::agent_state::push::known_subject);
        let status_code = notice
            .status_code
            .as_deref()
            .filter(|code| STATUS_CODES.contains(code));
        let detail = notice
            .detail
            .as_deref()
            .map(|detail| detail.chars().take(DETAIL_CHARS).collect::<String>());
        let frame = json!({
            "type": "agent_notice",
            "payload": {
                "notices": [{
                    "tag": tag,
                    "peer": key,
                    "device_id": notice.device_id,
                    "pane_id": notice.pane_id,
                    "title": notice.title,
                    "state": notice.state,
                    "status_code": status_code,
                    "detail": detail,
                    "subject": subject,
                    "place": notice.place,
                }],
                "clear": [],
            },
        });
        let shown = self.frames.send(Arc::from(frame.to_string())).is_ok();
        hide_core::diagnostic!(json!({
            "component": "desktop_notices", "kind": "desktop_notice.handed_in",
            "agent": tag, "shown": shown,
        }));
        shown
    }

    /// A phone named `phone` was paired with this machine: each connected
    /// computer that may open it is told once, whether or not someone is at
    /// it, so a pairing no one there made is seen (PRD device-connect-iroh
    /// D-70). They are the computers that told this machine their seat, at
    /// most [`MAX_COMPUTERS`]; one that is not connected now is not told.
    /// Returns how many were.
    pub fn phone_paired(&self, phone: &str) -> usize {
        let computers: Vec<String> = {
            let use_state = self.computer_use.lock().expect("computer use");
            use_state.peers.keys().cloned().collect()
        };
        let told = computers
            .iter()
            .filter(|key| {
                self.tell(
                    key,
                    CoreMessage::PhonePaired {
                        phone: phone.to_owned(),
                    },
                )
            })
            .count();
        hide_core::diagnostic!(json!({
            "component": "desktop_notices", "kind": "desktop_notices.phone_paired_told",
            "computers": computers.len(), "told": told,
        }));
        told
    }

    /// The machine under `key`, which this computer may open, says a phone
    /// named `phone` was paired with it (D-70). Every window here that takes
    /// notifications shows it, whoever is at this computer: it is the
    /// operator's one sign of a pairing they did not make.
    pub fn show_phone_paired(&self, key: &str, phone: &str) -> bool {
        if !wire::valid_phone_name(phone) {
            return false;
        }
        let frame = json!({
            "type": "agent_notice",
            "payload": {
                "notices": [],
                "clear": [],
                "phones": [{"tag": peer_tag(key, &format!("phone|{phone}")), "peer": key, "phone": phone}],
            },
        });
        let shown = self.frames.send(Arc::from(frame.to_string())).is_ok();
        hide_core::diagnostic!(json!({
            "component": "desktop_notices", "kind": "desktop_notice.phone_paired",
            "peer": key, "shown": shown,
        }));
        shown
    }

    /// The machine under `key` says the notices under `tags` are over.
    pub fn withdraw(&self, key: &str, tags: &[String]) {
        let clear: Vec<String> = tags
            .iter()
            .take(wire::MAX_CLEAR_TAGS)
            .filter(|tag| wire::valid_tag(tag))
            .map(|tag| peer_tag(key, tag))
            .collect();
        if clear.is_empty() {
            return;
        }
        let frame = json!({"type": "agent_notice", "payload": {"notices": [], "clear": clear}});
        let _ = self.frames.send(Arc::from(frame.to_string()));
    }
}

/// The tag a window knows another machine's notice by: that machine's own
/// tag behind its connection id, so two machines' tags never meet.
fn peer_tag(key: &str, tag: &str) -> String {
    format!("peer:{key}|{tag}")
}

/// `notice` as another computer is handed it, when it is one a computer
/// shows.
fn wire_notice(notice: &Notice) -> Option<wire::Notice> {
    matches!(notice.state, NoticeState::NeedsYou | NoticeState::Done).then(|| {
        wire::Notice {
            device_id: notice.key.device_id.clone(),
            pane_id: notice.key.pane_id.clone(),
            title: notice.title.clone(),
            state: notice.state.as_str().to_owned(),
            status_code: notice.status_code.clone(),
            detail: notice.detail.clone(),
            subject: notice
                .subject
                .as_ref()
                .and_then(|subject| serde_json::to_value(subject).ok()),
            place: notice.place.clone(),
        }
        .clipped()
    })
}

/// The `agent_notice` frame: data only, as the phone's push payload is; the
/// window words the state in the interface language.
fn frame<'a>(notices: &[Notice], clear: impl Iterator<Item = &'a AgentKey>) -> Value {
    json!({
        "type": "agent_notice",
        "payload": {
            "notices": notices.iter().map(|notice| json!({
                "tag": notice.key.tag(),
                "device_id": notice.key.device_id,
                "pane_id": notice.key.pane_id,
                "title": notice.title,
                "state": notice.state.as_str(),
                "status_code": notice.status_code,
                "detail": notice.detail,
                "subject": notice.subject,
                "place": notice.place,
            })).collect::<Vec<_>>(),
            "clear": clear.map(AgentKey::tag).collect::<Vec<_>>(),
        },
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn any_active_host_with_a_waiting_notice_switch_rests_phones_until_it_leaves() {
        let windows = super::DesktopNotices::default();
        windows.activity(1, true);
        assert!(
            !windows.rests_phone(),
            "an active host without notifications cannot replace the phone"
        );
        windows.set_waiting(1, true);
        windows.activity(2, true);
        windows.set_waiting(2, true);
        assert!(windows.rests_phone());
        windows.activity(1, false);
        assert!(windows.rests_phone(), "the second host is still in use");
        windows.release(2);
        assert!(!windows.rests_phone(), "a disconnected host is gone");
        windows.activity(1, true);
        assert!(windows.rests_phone());
        windows.release(1);
        assert!(
            !windows.rests_phone(),
            "no attached window means phone pushes"
        );
    }
    #[test]
    fn latest_accepted_device_wins_with_computer_ties_and_no_host_fallback() {
        let windows = super::DesktopNotices::default();
        let at = std::time::Instant::now();
        let second = std::time::Duration::from_secs(1);
        windows.set_waiting(1, true);
        windows.activity_at(1, true, at);
        assert!(windows.rests_phone());
        windows.phone_input(at + second * 2);
        assert!(!windows.rests_phone());
        windows.activity_at(1, true, at + second * 3);
        assert!(
            windows.rests_phone(),
            "resumed input while already in use wins"
        );
        windows.phone_input(at + second * 3 + second / 2);
        assert!(windows.rests_phone(), "within one poll the computer wins");
        windows.phone_input(at + second * 5);
        assert!(!windows.rests_phone());
        windows.activity_at(1, false, at + second * 6);
        assert!(
            !windows.rests_phone(),
            "lock, sleep or idle still permit pushes"
        );
        windows.activity_at(1, true, at + second * 7);
        assert!(windows.rests_phone(), "waking to input rests again");
        windows.set_waiting(1, false);
        assert!(
            !windows.rests_phone(),
            "no desktop Needs You means phone pushes"
        );
        windows.set_waiting(1, true);
        assert!(windows.rests_phone());
        windows.release(1);
        assert!(!windows.rests_phone());
        windows.phone_input(at + second * 8);
        assert!(
            !windows.rests_phone(),
            "no host keeps the existing fallback"
        );
        let restarted = super::DesktopNotices::default();
        restarted.set_waiting(1, true);
        restarted.activity_at(1, true, at + second * 9);
        assert!(restarted.rests_phone(), "restart forgets phone takeover");
    }

    use super::*;
    use hide_core::agent_state::phone::project;

    fn rest(group: &str, demand: &str) -> Value {
        json!({
            "navigator": {
                "agents": [{
                    "pane_id": "w1:p1", "group": group, "mark": "going",
                    "identity_label": "Add a dark mode switch", "agent_kind": "claude",
                    "changed_at_unix_ms": 1_790_000_000_000_u64, "status_code": "idle",
                    "demand": demand,
                }],
                "workspaces": [{"label": "my-app", "is_git": true, "checkouts": [
                    {"label": "main", "branch": "main", "tabs": [{"panes": [{"id": "w1:p1"}]}]},
                ]}],
                "devices": [{"id": "node", "label": null}],
            },
        })
    }

    fn judged(notices: &DesktopNotices, rest: &Value) -> Option<Value> {
        let mut frames = notices.subscribe();
        notices.observe(&project(rest, "node"), Instant::now());
        frames
            .try_recv()
            .ok()
            .map(|frame| serde_json::from_str(&frame).unwrap())
    }

    /// B33: the projection after the switch turns on only seeds; a root that
    /// newly needs the operator is one notice with the task and the project,
    /// the same state again is none, and the root turning Seen clears it.
    #[test]
    fn the_first_projection_seeds_and_a_new_wait_is_one_notice_cleared_when_seen() {
        let notices = DesktopNotices::default();
        assert!(notices.set(7, true));
        notices.activity(7, true);
        assert!(notices.wanted() && notices.seed_pending());
        assert_eq!(judged(&notices, &rest("needs_you", "approval")), None);
        assert!(!notices.seed_pending());
        assert_eq!(judged(&notices, &rest("working", "none")), None);

        let frame = judged(&notices, &rest("needs_you", "approval")).expect("a notice");
        assert_eq!(frame["type"], "agent_notice");
        let notice = &frame["payload"]["notices"][0];
        assert_eq!(notice["state"], "needs_you");
        assert_eq!(notice["title"], "Add a dark mode switch");
        assert_eq!(notice["place"], "my-app");
        assert_eq!(notice["pane_id"], "w1:p1");
        assert_eq!(judged(&notices, &rest("needs_you", "approval")), None);

        let cleared = judged(&notices, &rest("seen", "none")).expect("a clear");
        assert_eq!(cleared["payload"]["notices"], json!([]));
        assert_eq!(cleared["payload"]["clear"], json!([notice["tag"].clone()]));
    }

    /// The switch follows the last window that has it on, and turning it on
    /// again seeds afresh, so a wait that began while it was off is not
    /// announced late.
    #[test]
    fn the_switch_follows_its_windows_and_reseeds_when_it_turns_on_again() {
        let notices = DesktopNotices::default();
        assert!(notices.set(1, true));
        assert!(!notices.set(2, true), "a second window adds no transition");
        judged(&notices, &rest("working", "none"));
        notices.release(1);
        assert!(notices.wanted());
        notices.set(2, false);
        assert!(!notices.wanted());
        assert!(notices.set(2, true));
        assert_eq!(judged(&notices, &rest("needs_you", "question")), None);
    }

    /// status-model.md: the root's detail, not a child's line or a newly
    /// generated sentence, reaches both transports; copy changes alone are
    /// not state transitions and must not repeat a notification.
    #[test]
    fn each_notice_carries_the_roots_chosen_sentence_and_status() {
        for (group, demand, status, detail) in [
            (
                "needs_you",
                "question",
                "question",
                Some("배포할 환경을 골라 주세요"),
            ),
            ("needs_you", "approval", "approval", Some("계획 승인 대기")),
            ("needs_you", "error", "error", Some("로그인이 필요해요")),
            ("done", "none", "done", Some("배포 완료")),
            ("done", "none", "stopped", Some("테스트 세 개 남음")),
            ("needs_you", "approval", "approval", None),
        ] {
            let notices = DesktopNotices::default();
            notices.set(7, true);
            notices.activity(7, true);
            judged(&notices, &rest("working", "none"));
            let mut asking = rest(group, demand);
            asking["navigator"]["agents"][0]["status_code"] = json!(status);
            asking["navigator"]["agents"][0]["detail"] = json!(detail);
            let frame = judged(&notices, &asking).expect("a new notice");
            let notice = &frame["payload"]["notices"][0];
            assert_eq!(notice["status_code"], status);
            assert_eq!(notice["detail"], json!(detail));
            asking["navigator"]["agents"][0]["detail"] = json!("새 문장");
            assert_eq!(judged(&notices, &asking), None);
        }
    }

    /// PRD device-connect-iroh B59: a computer
    /// that is on and not in use gets no notice, the one it missed is not
    /// shown when someone returns, a later one is, and a phone that acted
    /// after the computer's last input is where the operator is until the
    /// next computer input.
    #[test]
    fn a_new_notice_reaches_the_windows_only_while_this_computer_is_in_use() {
        let notices = DesktopNotices::new();
        notices.set(7, true);
        judged(&notices, &rest("working", "none"));
        assert_eq!(
            judged(&notices, &rest("needs_you", "approval")),
            None,
            "no one is at this computer"
        );
        notices.activity(7, true);
        assert_eq!(
            judged(&notices, &rest("needs_you", "approval")),
            None,
            "the wait that began while away is not announced late"
        );
        judged(&notices, &rest("working", "none"));
        let frame = judged(&notices, &rest("needs_you", "question")).expect("a notice");
        let tag = frame["payload"]["notices"][0]["tag"].clone();

        // Away again: the notice shown before is still cleared.
        notices.activity(7, false);
        let cleared = judged(&notices, &rest("seen", "none")).expect("a clear");
        assert_eq!(cleared["payload"]["clear"], json!([tag]));

        let at = Instant::now();
        notices.activity_at(7, true, at);
        notices.phone_input(at + Duration::from_secs(3));
        judged(&notices, &rest("working", "none"));
        assert_eq!(
            judged(&notices, &rest("needs_you", "approval")),
            None,
            "the operator answered from a phone last"
        );
    }

    /// The other computers as this machine writes to them: what each was
    /// told, and the ones nothing can be written to.
    #[derive(Default)]
    struct Others {
        told: Mutex<Vec<(String, CoreMessage)>>,
        unreachable: Mutex<HashSet<String>>,
    }

    impl Computers for Others {
        fn tell(&self, key: &str, message: CoreMessage) -> bool {
            if self.unreachable.lock().unwrap().contains(key) {
                return false;
            }
            self.told.lock().unwrap().push((key.to_owned(), message));
            true
        }
    }

    impl Others {
        /// What was told since the last look, as `(computer, kind, id)`.
        fn taken(&self) -> Vec<(String, &'static str, u64)> {
            std::mem::take(&mut *self.told.lock().unwrap())
                .into_iter()
                .map(|(key, message)| match message {
                    CoreMessage::Notice { id, .. } => (key, "notice", id),
                    CoreMessage::NoticeClear { tags } => (key, "clear", tags.len() as u64),
                    _ => (key, "other", 0),
                })
                .collect()
        }
    }

    fn used(seconds_ago: u64) -> Seat {
        Seat {
            input: Some(Duration::from_secs(seconds_ago)),
            shows: true,
            phone: None,
        }
    }

    fn with_others() -> (DesktopNotices, Arc<Others>) {
        let (notices, others) = (DesktopNotices::new(), Arc::new(Others::default()));
        notices.set_computers(Arc::clone(&others) as Arc<dyn Computers>);
        (notices, others)
    }

    fn key() -> AgentKey {
        AgentKey {
            device_id: "node".into(),
            pane_id: "w1:p1".into(),
        }
    }

    /// PRD device-connect-iroh B59, B71: with no one at this machine a new
    /// notice goes to the one computer in use that may open it, the one
    /// used last; a computer that does not say within the limit that it
    /// showed it, or says no, is at no seat any more and the next place
    /// gets it, the phones last. No window here is needed for any of it.
    #[test]
    fn a_notice_goes_to_one_computer_in_use_and_on_to_the_next_place_when_it_is_not_taken() {
        let (notices, others) = with_others();
        assert!(!notices.judging());
        notices.peer_seat("macbook", used(2));
        notices.peer_seat("windows", used(30));
        assert!(notices.judging() && notices.seed_pending());
        assert!(notices.rests_phone(), "a computer is the place");
        let at = Instant::now();
        let observe = |rest: &Value, at| notices.observe(&project(rest, "node"), at);
        observe(&rest("working", "none"), at);

        assert_eq!(observe(&rest("needs_you", "approval"), at), []);
        assert_eq!(others.taken(), [("macbook".to_owned(), "notice", 0)]);
        assert_eq!(notices.next_due(), Some(at + TAKEN_WITHIN));

        // Another computer's answer is not this notice's.
        notices.answered("windows", 0, true);
        assert_eq!(
            notices.expire(at + TAKEN_WITHIN - Duration::from_secs(1)),
            []
        );
        assert_eq!(others.taken(), []);

        assert_eq!(notices.expire(at + TAKEN_WITHIN), []);
        assert_eq!(others.taken(), [("windows".to_owned(), "notice", 1)]);
        assert!(notices.rests_phone(), "the other computer is the place now");

        notices.answered("windows", 1, false);
        assert_eq!(
            notices.expire(at + TAKEN_WITHIN * 2),
            [key()],
            "no computer took it: the phones'"
        );
        assert_eq!(others.taken(), []);
        assert!(!notices.rests_phone(), "no computer is at a seat it told");
        assert_eq!(notices.next_due(), None);

        // The computer that did not answer is tried again once it tells
        // its seat, and one nothing can be written to is passed over.
        notices.peer_seat("macbook", used(0));
        others.unreachable.lock().unwrap().insert("macbook".into());
        observe(&rest("working", "none"), at);
        assert_eq!(observe(&rest("needs_you", "question"), at), [key()]);
        assert_eq!(others.taken(), []);
    }

    /// B71, B35: the computer that showed a notice is told when it is over;
    /// someone at this machine takes a notice before any other computer;
    /// and a computer that left is asked nothing, with what it had not
    /// answered going on at once.
    #[test]
    fn the_computer_that_showed_a_notice_closes_it_and_this_machine_in_use_comes_first() {
        let (notices, others) = with_others();
        notices.peer_seat("macbook", used(2));
        let at = Instant::now();
        let observe = |rest: &Value| notices.observe(&project(rest, "node"), at);
        observe(&rest("working", "none"));
        observe(&rest("needs_you", "approval"));
        assert_eq!(others.taken(), [("macbook".to_owned(), "notice", 0)]);
        notices.answered("macbook", 0, true);
        assert_eq!(notices.next_due(), None);
        observe(&rest("seen", "none"));
        assert_eq!(others.taken(), [("macbook".to_owned(), "clear", 1)]);

        // Someone sits down here: the next notice is this machine's own.
        notices.set(7, true);
        notices.activity(7, true);
        let mut frames = notices.subscribe();
        observe(&rest("working", "none"));
        observe(&rest("needs_you", "question"));
        assert!(frames.try_recv().is_ok());
        assert_eq!(others.taken(), []);

        // Away again, a notice waits on the other computer, which leaves.
        notices.release(7);
        observe(&rest("working", "none"));
        observe(&rest("needs_you", "approval"));
        assert_eq!(others.taken(), [("macbook".to_owned(), "notice", 1)]);
        notices.peer_gone("macbook");
        assert!(!notices.judging());
        assert_eq!(notices.expire(Instant::now()), [key()]);
        assert_eq!(others.taken(), []);
    }

    /// B71: a notice another machine hands over is shown only while someone
    /// is at this computer with notifications on, under a tag of that
    /// machine's own, and its close reaches the windows under the same tag.
    #[test]
    fn a_handed_notice_shows_only_while_someone_is_here() {
        let notices = DesktopNotices::new();
        let handed = wire::Notice {
            device_id: "node".into(),
            pane_id: "w1:p1".into(),
            title: "Fix login".into(),
            state: "needs_you".into(),
            status_code: Some("question".into()),
            detail: Some("Which branch?".into()),
            subject: Some(json!({"cause": "of a build this one cannot read"})),
            place: "shop-api".into(),
        };
        let mut frames = notices.subscribe();
        notices.set(1, true);
        assert!(!notices.show("pc", &handed), "no one is here");
        assert!(frames.try_recv().is_err());

        notices.activity(1, true);
        assert!(notices.show("pc", &handed));
        let frame: Value = serde_json::from_str(&frames.try_recv().unwrap()).unwrap();
        let shown = &frame["payload"]["notices"][0];
        assert_eq!(shown["tag"], "peer:pc|node|w1:p1");
        assert_eq!(shown["peer"], "pc");
        assert_eq!(shown["title"], "Fix login");
        assert_eq!(shown["detail"], "Which branch?");
        assert_eq!(shown["subject"], Value::Null);

        let mut wrong = handed.clone();
        wrong.state = "working".into();
        assert!(!notices.show("pc", &wrong), "not a notice a computer shows");

        // A later build's status code and a long sentence: the window gets
        // only what it words.
        let mut later = handed.clone();
        later.status_code = Some("paused_by_policy".into());
        later.detail = Some("가".repeat(DETAIL_CHARS + 20));
        assert!(notices.show("pc", &later));
        let frame: Value = serde_json::from_str(&frames.try_recv().unwrap()).unwrap();
        let shown = &frame["payload"]["notices"][0];
        assert_eq!(shown["status_code"], Value::Null);
        assert_eq!(shown["detail"], "가".repeat(DETAIL_CHARS));
        let schema: Value =
            serde_json::from_str(include_str!("../../contracts/hided-ws.schema.json")).unwrap();
        let item = &schema["$defs"]["agentNotice"]["properties"]["notices"]["items"]["properties"];
        let mut listed: Vec<&str> = item["status_code"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        listed.sort_unstable();
        let mut known = STATUS_CODES.to_vec();
        known.sort_unstable();
        assert_eq!(known, listed, "the codes the frame's contract lists");
        assert_eq!(item["detail"]["maxLength"], DETAIL_CHARS);

        notices.withdraw("pc", &["node|w1:p1".into(), String::new()]);
        let frame: Value = serde_json::from_str(&frames.try_recv().unwrap()).unwrap();
        assert_eq!(frame["payload"]["clear"], json!(["peer:pc|node|w1:p1"]));
    }

    /// D-60: this computer's seat is told when it reads differently, and
    /// again when input resumes after a pause, never per input.
    #[test]
    fn this_seat_is_told_when_it_changes_and_not_per_input() {
        let notices = DesktopNotices::new();
        let mut moves = notices.seat_moves();
        let mut told = || {
            let moved = moves.has_changed().unwrap();
            moves.borrow_and_update();
            moved
        };
        let at = Instant::now();
        let second = Duration::from_secs(1);
        notices.activity_at(1, true, at);
        assert!(told(), "someone sat down");
        notices.activity_at(1, true, at + second);
        notices.phone_input(at + second);
        assert!(!told(), "the seat reads as it did");
        notices.activity_at(1, true, at + second * 6);
        assert!(told(), "input resumed after a pause");
        notices.phone_input(at + second * 9);
        assert!(told(), "a phone acted after the last input");
        notices.phone_input(at + second * 10);
        assert!(!told());
        notices.activity_at(1, false, at + second * 11);
        assert!(told(), "no one is here");
        let seat = notices.seat();
        assert_eq!((seat.input, seat.shows), (None, false));
        assert!(seat.phone.is_some());
    }
}
