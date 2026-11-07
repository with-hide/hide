//! A node's holder panes, through hide-node's own code with the built
//! holder (PRD engine-bundle-2-pane-holder D-10, D-11, D-15, D-17, B1, B2,
//! B7, B9, B10): a start shows the pane's first full screen, takes input and
//! converges when asked again; a node killed outright finds its holders
//! again when the next one starts, attaches 64 and keeps the rest running,
//! and reports the pane that ended meanwhile; a takeover ends the first
//! node's session for good; the node speaks to a holder of the oldest
//! version it supports and leaves an older one running; and it tells the
//! engine every pane's facts, read by the rules its holder can read (PRD
//! engine-bundle-3-core-owns D-18, D-21).
//!
//! The node is `hide_node::terminal::Service` with `holder::Holders`, as the
//! daemon builds it, in this test process, or in the `node` role when the
//! test kills it.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use hide_holder_protocol::record::{self, EndReason, Identity, PaneRecord, Phase, RECORD_FORMAT};
use hide_holder_protocol::{Answer, Size};
use hide_kit::layout;
use hide_kit::panes::{StartRequest, Started};
use hide_node::terminal::engine_panes::{LocalPanes, PaneShell};
use hide_node::terminal::holder::{HolderFolders, Holders, open_holder};
use hide_node::terminal::protocol::SessionEvent;
use hide_node::terminal::{Attacher, OutputSink, ReportSink, RetryPolicy, Service};
use hide_node_link::holders::{
    HolderError, HolderStart, NO_RULES_FOR_HOLDER, PaneFacts, PaneFactsSink, PaneHolders, RuleSet,
    ScreenRead, ScreenRules, ScreenState, UNSUPPORTED_HOLDER, WATCH_LOST,
};
use hide_node_link::terminal::{
    GridSize, KeyTarget, TerminalControl, TerminalNode, TerminalReport,
};

use crate::fixture_v1;
use crate::role::GO;
use crate::support::{
    HANG_LIMIT, Home, PIDS_FILE, ROLE, Running, SCRIPT, Script, node_theme, print_ok, role_program,
    shell, source, wait_until,
};

pub(crate) mod ownership;
mod rules_recovery;
mod transport_ownership;

/// The variables the `node` role reads.
const NODE_HOME: &str = "HIDE_HOLDER_TEST_NODE_HOME";
const NODE_STATE: &str = "HIDE_HOLDER_TEST_NODE_STATE";
const NODE_READY: &str = "HIDE_HOLDER_TEST_NODE_READY";

/// The panes the killed node starts: one more than a node attaches, one
/// that prints while no node is there, and one that ends then.
const SHELLS: usize = hide_node_link::terminal::MAX_ATTACHED_PANES;
const PRINTS: &str = "a-print";
const ENDS: &str = "a-end";
/// What `PRINTS` writes once the node is gone.
const WHILE_AWAY: &str = "while-away";

const SIZE: GridSize = GridSize { rows: 24, cols: 80 };

/// A frame's bytes and whether it was full.
type Frame = (Vec<u8>, bool);

/// The frames a node handed its screen, by pane, in order.
#[derive(Default)]
struct Screens {
    frames: Mutex<BTreeMap<String, Vec<Frame>>>,
    /// What the panes' programs wrote to the clipboard, by pane, in order.
    copied: Mutex<Vec<(String, Vec<u8>)>>,
    changed: Condvar,
}

impl OutputSink for Screens {
    fn output(&self, pane: &str, bytes: &[u8], full: bool) {
        self.lock()
            .entry(pane.to_owned())
            .or_default()
            .push((bytes.to_vec(), full));
        self.changed.notify_all();
    }

    fn forget(&self, _pane: &str) {}

    fn clipboard(&self, pane: &str, text: &[u8]) {
        self.copied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((pane.to_owned(), text.to_vec()));
        self.changed.notify_all();
    }
}

impl Screens {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Vec<Frame>>> {
        self.frames.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits for a full frame of `pane` whose text holds `text`, and
    /// answers its text.
    fn wait_full(&self, pane: &str, text: &str) -> String {
        let deadline = Instant::now() + HANG_LIMIT;
        let mut frames = self.lock();
        loop {
            if let Some(found) = frames
                .get(pane)
                .into_iter()
                .flatten()
                .filter(|(_, full)| *full)
                .map(|(bytes, _)| text_of(bytes))
                .find(|drawn| drawn.contains(text))
            {
                return found;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "pane {pane} never drew {text:?} in a full frame"
            );
            frames = self
                .changed
                .wait_timeout(frames, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// The first full frame of `pane`, waited for.
    fn first_full(&self, pane: &str) -> String {
        self.wait_full(pane, "")
    }

    fn drew(&self, pane: &str) -> bool {
        self.lock().contains_key(pane)
    }

    /// How many full frames `pane` drew.
    fn fulls(&self, pane: &str) -> usize {
        self.lock()
            .get(pane)
            .map_or(0, |frames| frames.iter().filter(|(_, full)| *full).count())
    }

    /// Waits for `pane` to draw `text` in the frames after its first
    /// `after`, read together, since one printed line may come in more than
    /// one frame.
    fn wait_drawn_after(&self, pane: &str, after: usize, text: &str) {
        let deadline = Instant::now() + HANG_LIMIT;
        let mut frames = self.lock();
        loop {
            let drawn: Vec<u8> = frames
                .get(pane)
                .into_iter()
                .flatten()
                .skip(after)
                .flat_map(|(bytes, _)| bytes.iter().copied())
                .collect();
            let drawn = text_of(&drawn);
            if drawn.contains(text) {
                return;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "pane {pane} never drew {text:?}; it drew:\n{drawn}"
            );
            frames = self
                .changed
                .wait_timeout(frames, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn count(&self, pane: &str) -> usize {
        self.lock().get(pane).map_or(0, Vec::len)
    }
}

/// The text a frame draws: its bytes less their escape sequences.
fn text_of(bytes: &[u8]) -> String {
    let mut text = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != 0x1b {
            if bytes[at] >= 0x20 {
                text.push(bytes[at]);
            }
            at += 1;
            continue;
        }
        at += 1;
        match bytes.get(at) {
            Some(b'[') => {
                at += 1;
                while at < bytes.len() && !(0x40..=0x7e).contains(&bytes[at]) {
                    at += 1;
                }
                at += 1;
            }
            Some(b']' | b'P' | b'_') => {
                while at < bytes.len() && bytes[at] != 0x07 && bytes[at] != 0x1b {
                    at += 1;
                }
                at += if bytes.get(at) == Some(&0x1b) { 2 } else { 1 };
            }
            _ => at += 1,
        }
    }
    String::from_utf8_lossy(&text).into_owned()
}

/// What a node told its core, in order.
#[derive(Default)]
struct Reports {
    told: Mutex<Vec<TerminalReport>>,
    changed: Condvar,
}

impl ReportSink for Reports {
    fn report(&self, report: TerminalReport) {
        self.told
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(report);
        self.changed.notify_all();
    }
}

impl Reports {
    /// Waits for `pane`'s state to be reported `state`, and answers it.
    fn wait_state(&self, pane: &str, state: &str) -> hide_node_link::terminal::PaneTerminalState {
        let deadline = Instant::now() + HANG_LIMIT;
        let mut told = self.told.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(found) = told.iter().find_map(|report| match report {
                TerminalReport::State {
                    pane: named,
                    state: found,
                } if named == pane && found.state == state => Some(found.clone()),
                _ => None,
            }) {
                return found;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "pane {pane} was never reported {state}");
            told = self
                .changed
                .wait_timeout(told, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn states(&self, pane: &str) -> Vec<String> {
        self.told
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter_map(|report| match report {
                TerminalReport::State { pane: named, state } if named == pane => {
                    Some(state.state.clone())
                }
                _ => None,
            })
            .collect()
    }
}

/// The facts a node told the engine's sink, in order.
#[derive(Default)]
struct Told {
    facts: Mutex<Vec<(String, PaneFacts)>>,
    changed: Condvar,
}

impl Told {
    fn sink(self: &Arc<Self>) -> PaneFactsSink {
        let told = Arc::clone(self);
        Arc::new(move |pane: &str, facts: &PaneFacts| {
            told.facts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((pane.to_owned(), facts.clone()));
            told.changed.notify_all();
        })
    }

    /// Waits until the last facts told of `pane` are ones for which
    /// `matches` holds, and answers them.
    fn wait(&self, pane: &str, what: &str, matches: impl Fn(&PaneFacts) -> bool) -> PaneFacts {
        let deadline = Instant::now() + HANG_LIMIT;
        let mut told = self.facts.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            let last = told
                .iter()
                .rev()
                .find(|(named, _)| named == pane)
                .map(|(_, facts)| facts.clone());
            if let Some(facts) = last.filter(|facts| matches(facts)) {
                return facts;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "pane {pane} was never told {what}; it was told {:?}",
                told.iter()
                    .filter(|(named, _)| named == pane)
                    .collect::<Vec<_>>()
            );
            told = self
                .changed
                .wait_timeout(told, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// How many facts were told of `pane`.
    fn count(&self, pane: &str) -> usize {
        self.facts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(named, _)| named == pane)
            .count()
    }
}

/// A node over one state folder, as the daemon builds it.
struct Node {
    service: Service,
    holders: Holders,
    owner: Option<hide_kit::panes::Owner>,
    screens: Arc<Screens>,
    reports: Arc<Reports>,
}

impl Node {
    fn start(home: &Path, state: &Path) -> Self {
        let screens = Arc::new(Screens::default());
        let reports = Arc::new(Reports::default());
        let holders = Holders::open(
            HolderFolders {
                home,
                state_dir: state,
            },
            source(),
            Arc::clone(&reports) as Arc<dyn ReportSink>,
        )
        .unwrap();
        let service = Service::start(
            holders.attacher(),
            Arc::clone(&screens) as Arc<dyn OutputSink>,
            Arc::clone(&reports) as Arc<dyn ReportSink>,
            RetryPolicy::Automatic,
        )
        .unwrap();
        let owner = Some(holders.acquire(Duration::from_secs(5)).unwrap());
        Self {
            service,
            holders,
            owner,
            screens,
            reports,
        }
    }

    fn holders(&self) -> &Holders {
        &self.holders
    }

    fn owner(&self) -> &hide_kit::panes::Owner {
        self.owner.as_ref().expect("the fixture owns its records")
    }

    fn start_pane(&self, request: &StartRequest) -> Started {
        self.holders()
            .start(self.owner(), &self.service, request)
            .unwrap_or_else(|refused| panic!("{} did not start: {refused}", request.pane))
    }

    fn type_into(&self, pane: &str, bytes: &[u8]) {
        self.service
            .key(KeyTarget::Pane(pane.to_owned()), bytes.to_vec(), 0);
    }

    /// The pane's recent text, read on a connection of its own.
    fn text(&self, pane: &str) -> String {
        match self.holders().read(pane, 200).unwrap() {
            Answer::Text { text, .. } => text,
            other => panic!("a read answered {other:?}"),
        }
    }
}

/// A private HOME and state folder. Dropping it ends every holder its
/// records name, and what each pane's program left, by pid and start time.
struct Folder {
    home: Home,
    state: PathBuf,
}

impl Folder {
    fn new() -> Self {
        let home = Home::new();
        let state = home.path().join("state");
        Self { home, state }
    }

    fn shell(&self, pane: &str) -> StartRequest {
        request(self.home.path(), pane, shell(), Vec::new())
    }

    fn record(&self, pane: &str) -> PaneRecord {
        record::read(&layout::pane_record(&self.state, pane))
            .unwrap()
            .expect("the pane has a record")
    }

    fn go(&self) -> PathBuf {
        self.home.path().join("go")
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let Ok(entries) = std::fs::read_dir(layout::pane_records(&self.state)) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(Some(record)) = record::read(&entry.path()) else {
                continue;
            };
            if let Some(holder) = record.holder
                && holder.runs()
            {
                let _ = hide_platform::process::kill_tree(holder.pid);
            }
            #[cfg(unix)]
            if let Some(program) = record.program
                && let Ok(mut session) =
                    hide_platform::process::Session::left_by(program.pid, program.started)
            {
                let _ = session.end(Duration::from_secs(5));
            }
        }
    }
}

/// A pane started in `cwd` at [`SIZE`] with the node's theme.
fn request(
    cwd: &Path,
    pane: &str,
    program: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
) -> StartRequest {
    StartRequest {
        pane: pane.to_owned(),
        program,
        cwd: cwd.to_owned(),
        env,
        size: Size {
            rows: SIZE.rows,
            cols: SIZE.cols,
        },
        theme: node_theme(),
    }
}

/// A line that prints `<word>-node` without that text in the line itself.
/// On Windows the caret keeps the typed line from holding it; a variable
/// would not do twice in one shell, since cmd expands `%B%` when it reads
/// the line, before its `set` runs, and prints the word set before.
/// Starts `start`'s pane as the core's own engine does: a request number,
/// the pane's reservation, its start, and the number retired unless the
/// answer was lost.
fn start_pane(holders: &dyn PaneHolders, start: &HolderStart) -> Result<(), HolderError> {
    let request = holders.request()?;
    let result = holders
        .reserve(start, None, request)
        .and_then(|reserved| holders.start_reserved(start, reserved, request));
    if !matches!(result, Err(HolderError::Unreachable { .. })) {
        holders.retire(request)?;
    }
    result
}

fn print_node(word: &str) -> Vec<u8> {
    if cfg!(windows) {
        format!("echo {word}^-node\r").into_bytes()
    } else {
        format!("B={word}; echo \"$B-node\"\r").into_bytes()
    }
}

/// A process this test started, ended by its own handle however the test
/// ends.
struct Spawned(Child);

impl Drop for Spawned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn role(role: &str, env: &[(&str, &OsString)]) -> Spawned {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "role::play", "--nocapture", "--test-threads=1"])
        .env(ROLE, role)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    Spawned(command.spawn().unwrap())
}

/// B2: a started pane shows its first full screen, takes the bytes typed
/// into it, answers the same start with the same pane, and ends whole.
#[test]
fn a_started_pane_shows_its_first_full_screen_takes_input_and_converges() {
    let folder = Folder::new();
    let node = Node::start(folder.home.path(), &folder.state);
    let request = folder.shell("s1");
    let Started::Started(first) = node.start_pane(&request) else {
        panic!("the first start found a pane");
    };
    node.screens.first_full("s1");
    node.type_into("s1", print_ok());
    wait_until("the shell's answer", || node.text("s1").contains("hide-ok"));

    let Started::Existing(again) = node.start_pane(&request) else {
        panic!("the same start asked again started a second holder");
    };
    assert_eq!(
        (again.incarnation, again.holder),
        (first.incarnation, first.holder)
    );

    let holder = first.holder.unwrap();
    let ended = node
        .holders()
        .terminate(node.owner(), &node.service, "s1")
        .unwrap();
    assert_eq!(ended.phase, Phase::Ended);
    assert_eq!(ended.end.map(|end| end.reason), Some(EndReason::Terminated));
    assert!(!holder.runs());
}

/// The saved layout, or a newly drawn pane, can attach before its holder's
/// start was registered. The node's engine already owns that pane: it
/// waits for the same holder start and draws its output.
#[test]
fn an_engine_pane_attaches_before_its_holder_start_is_registered() {
    let folder = Folder::new();
    let Node {
        service,
        holders,
        screens,
        reports,
        owner,
    } = Node::start(folder.home.path(), &folder.state);
    drop(owner);
    let service = Arc::new(service);
    let holders = Arc::new(holders);
    let engine = Arc::new(LocalPanes::new(
        Arc::clone(&holders),
        Arc::clone(&service),
        PaneShell::Fixed(shell()),
    ))
    .own(Duration::from_secs(5))
    .unwrap();
    let pane = "w1:p1";
    assert!(holders.record(pane).unwrap().is_none());
    assert!(
        holders.attacher().keeps_own_size(pane),
        "an engine pane belongs to its holder before a record or start exists"
    );
    service.control(TerminalControl::Attach {
        pane: pane.to_owned(),
        size: None,
        manual: false,
    });
    reports.wait_state(pane, "starting");
    start_pane(
        &*engine,
        &HolderStart {
            pane: pane.to_owned(),
            program: None,
            cwd: hide_platform::path::to_wire(folder.home.path()).unwrap(),
            env: Vec::new(),
            rows: SIZE.rows,
            cols: SIZE.cols,
            notice: None,
        },
    )
    .unwrap();
    screens.first_full(pane);
    service.key(KeyTarget::Pane(pane.to_owned()), print_ok().to_vec(), 0);
    screens.wait_drawn_after(pane, 0, "hide-ok");
    assert!(
        !reports
            .states(pane)
            .iter()
            .any(|state| state == "unavailable")
    );
    engine.terminate(pane).unwrap();
    service.close();
}

/// An unreadable neighbour stays quarantined while the node connection
/// recovers a live pane and admits a new, distinct pane.
#[test]
fn unreadable_records_do_not_reject_the_nodes_healthy_panes() {
    use hide_node_link::holders::PaneInput;
    use hide_node_link::link::NodeLink;
    use hide_node_link::link_panes::LinkPanes;

    let folder = Folder::new();
    let node = Node::start(folder.home.path(), &folder.state);
    node.start_pane(&folder.shell("w1:p1"));
    node.screens.first_full("w1:p1");
    let invalid = [
        ("w1:p2", b"{cut off".as_slice()),
        ("w1:p3", b"{\"format\":999}".as_slice()),
    ];
    for (pane, bytes) in invalid {
        std::fs::write(
            layout::pane_record(&folder.state, &hide_kit::panes::holder_pane(pane)),
            bytes,
        )
        .unwrap();
    }
    let Node {
        service,
        holders,
        owner,
        screens,
        ..
    } = node;
    drop(owner);
    let service = Arc::new(service);
    let local = Arc::new(
        hide_node::Local::new(Some(folder.home.path().to_owned())).with_panes(Arc::new(
            LocalPanes::new(
                Arc::new(holders),
                Arc::clone(&service),
                PaneShell::Fixed(shell()),
            ),
        )),
    );
    let linked = LinkPanes::new(Arc::clone(&local) as Arc<dyn NodeLink>, Arc::new(|_| {}));
    let found = linked.recovered(Duration::from_secs(5)).unwrap();
    for pane in ["w1:p1", "w1:p2", "w1:p3"] {
        assert!(
            found.present.contains(&pane.to_owned()),
            "{pane} was lost: {found:?}"
        );
    }
    linked
        .input("w1:p1", PaneInput::Bytes(print_ok().to_vec()))
        .unwrap();
    screens.wait_drawn_after("w1:p1", 0, "hide-ok");
    assert!(linked.read("w1:p1", 200).unwrap().0.contains("hide-ok"));
    let start = |pane: &str| HolderStart {
        pane: pane.to_owned(),
        program: None,
        cwd: hide_platform::path::to_wire(folder.home.path()).unwrap(),
        env: Vec::new(),
        rows: SIZE.rows,
        cols: SIZE.cols,
        notice: None,
    };
    start_pane(&linked, &start("w1:p4")).unwrap();
    screens.first_full("w1:p4");
    for (pane, bytes) in invalid {
        assert!(matches!(
            start_pane(&linked, &start(pane)),
            Err(HolderError::Refused { .. })
        ));
        assert_eq!(
            std::fs::read(layout::pane_record(
                &folder.state,
                &hide_kit::panes::holder_pane(pane)
            ))
            .unwrap(),
            bytes
        );
    }
    linked.terminate("w1:p4").unwrap();
    linked.terminate("w1:p1").unwrap();
    local.close("test finished");
    service.close();
}

/// A cancellation that finds an unreported holder death is still a close,
/// including a start whose cleanup was previously uncertain. Recovery must
/// not convert the cancelled intent into a replacement shell.
#[test]
fn cancelled_dead_holders_stay_closed_on_the_next_owner() {
    use hide_node_link::holders::HolderIntent;

    let folder = Folder::new();
    let mut node = Node::start(folder.home.path(), &folder.state);
    for (pane, phase) in [("p1", Phase::Running), ("p2", Phase::Ended)] {
        let intent = HolderIntent {
            pane: pane.to_owned(),
            digest: "a".repeat(64),
            cwd: hide_platform::path::to_wire(folder.home.path()).unwrap(),
        };
        let guard = node.owner().issue(pane, &intent.digest).unwrap();
        let reserved = node.holders().reserve(&guard, &intent, None).unwrap();
        node.holders()
            .start_reserved(&guard, &node.service, &intent, reserved, || {
                Ok(folder.shell(pane))
            })
            .unwrap();
        let mut abandoned = folder.record(pane);
        let holder = abandoned.holder.unwrap();
        hide_platform::process::kill_tree(holder.pid).unwrap();
        wait_until("the test holder to exit", || !holder.runs());
        // Model the durable record left when its final write never arrives.
        abandoned.phase = phase;
        abandoned.end = (phase == Phase::Ended).then(|| {
            record::End::new(
                EndReason::StartUnconfirmed,
                "the original start did not answer",
            )
        });
        {
            let _lock = record::lock(&layout::pane_records_lock(&folder.state), record::LOCK_WAIT)
                .unwrap()
                .unwrap();
            record::write(&layout::pane_record(&folder.state, pane), &abandoned).unwrap();
        }
        let original = abandoned.reservation.as_ref().unwrap();
        node.holders()
            .cancel(
                &guard,
                &node.service,
                &HolderIntent {
                    pane: pane.to_owned(),
                    digest: original.request.clone(),
                    cwd: original.cwd.clone(),
                },
            )
            .unwrap();
        let ended = folder.record(pane);
        assert_eq!(ended.phase, Phase::Ended);
        assert!(ended.cancelled());
        assert_eq!(ended.end.unwrap().reason, EndReason::Terminated);
        let panes =
            hide_kit::panes::Panes::open(folder.home.path(), &folder.state, |_| {}).unwrap();
        assert!(
            panes
                .start_reserved(
                    &guard,
                    pane,
                    abandoned.incarnation,
                    &original.request,
                    &source(),
                    || panic!("a cancelled start expanded its shell")
                )
                .is_err()
        );
    }
    drop(node.owner.take());
    node.owner = Some(node.holders().acquire(Duration::from_secs(5)).unwrap());
    node.holders().recover(node.owner(), &node.service);
    let recovered = node.holders().recovered(Duration::from_secs(5)).unwrap();
    assert_eq!(recovered.closed, vec!["p1", "p2"]);
    assert!(recovered.ended.is_empty(), "{recovered:?}");
    assert!(recovered.recoverable.is_empty(), "{recovered:?}");
    assert!(recovered.present.is_empty(), "{recovered:?}");
    node.service.close();
}

/// Reboot fences old process identities but cannot turn a completed pane
/// into a recoverable shell. Exercise the node's actual closed partition.
#[test]
fn completed_panes_from_another_boot_stay_closed() {
    let folder = Folder::new();
    let panes = hide_kit::panes::Panes::open(folder.home.path(), &folder.state, |_| {}).unwrap();
    let reasons = [
        EndReason::ProgramExited,
        EndReason::Terminated,
        EndReason::TreeEndUnconfirmed,
    ];
    for (i, reason) in reasons.into_iter().enumerate() {
        let pane = format!("p{i}");
        let record = PaneRecord {
            format: record::LEGACY_RECORD_FORMAT,
            pane: pane.clone(),
            owner: panes.owner().to_owned(),
            incarnation: i as u64 + 1,
            phase: Phase::Ended,
            protocol: hide_holder_protocol::PROTOCOL_VERSION,
            version_folder: String::new(),
            spec: String::new(),
            reservation: None,
            reserved_at_unix_ms: record::now_unix_ms(),
            boot: Some("test-earlier-boot".to_owned()),
            holder: None,
            program: None,
            end: Some(record::End::new(reason, "already completed")),
        };
        record::write(&layout::pane_record(&folder.state, &pane), &record).unwrap();
    }
    let node = Node::start(folder.home.path(), &folder.state);
    node.holders().recover(node.owner(), &node.service);
    let recovered = node.holders().recovered(Duration::from_secs(5)).unwrap();
    assert_eq!(recovered.closed, vec!["p0", "p1", "p2"]);
    assert!(recovered.ended.is_empty(), "{recovered:?}");
    assert!(recovered.recoverable.is_empty(), "{recovered:?}");
    assert!(recovered.present.is_empty(), "{recovered:?}");
    for (i, reason) in reasons.into_iter().enumerate() {
        assert_eq!(folder.record(&format!("p{i}")).end.unwrap().reason, reason);
    }
    node.service.close();
}

/// A redraw, and a view at the grid the pane already runs at, each bring a
/// full frame from the holder, and what the program prints after either is
/// drawn: the node waits for a full frame after both, and a holder answers
/// a same-size resize with nothing, so each asks for the screen with a
/// redraw instead.
#[test]
fn a_redraw_on_a_holder_pane_draws_its_screen_again_and_what_follows() {
    let folder = Folder::new();
    let node = Node::start(folder.home.path(), &folder.state);
    node.start_pane(&folder.shell("r1"));
    node.screens.first_full("r1");
    node.type_into("r1", print_ok());
    wait_until("the shell's answer", || node.text("r1").contains("hide-ok"));

    let asks: [(&str, &dyn Fn()); 2] = [
        ("redrawn", &|| node.service.redraw("r1")),
        ("viewed", &|| node.service.view("r1", SIZE, true)),
    ];
    for (word, ask) in asks {
        let fulls = node.screens.fulls("r1");
        ask();
        wait_until("a full frame after the ask", || {
            node.screens.fulls("r1") > fulls
        });
        let drawn = node.screens.count("r1");
        node.type_into("r1", &print_node(word));
        let printed = format!("{word}-node");
        let started = Instant::now();
        loop {
            let text = node.text("r1");
            if text.contains(&printed) {
                break;
            }
            assert!(
                started.elapsed() < HANG_LIMIT,
                "the program never printed {printed:?}; the pane holds:\n{text}"
            );
            #[allow(clippy::disallowed_methods)] // a read, not an event to wait on
            thread::sleep(Duration::from_millis(50));
        }
        node.screens.wait_drawn_after("r1", drawn, &printed);
    }
    node.holders()
        .terminate(node.owner(), &node.service, "r1")
        .unwrap();
}

/// The `node` role: starts every pane of the kill test, eight at once as a
/// node's starts come, says so once each attached pane drew its first full
/// screen, and lives until the test kills it.
pub fn node_role() {
    let home = PathBuf::from(std::env::var_os(NODE_HOME).unwrap());
    let state = PathBuf::from(std::env::var_os(NODE_STATE).unwrap());
    let go = std::env::var_os(GO).unwrap();
    let pids = std::env::var_os(PIDS_FILE).unwrap();
    let node = Node::start(&home, &state);
    let played = |pane: &str, role: &str| {
        request(
            &home,
            pane,
            role_program(role),
            vec![
                (ROLE.into(), role.into()),
                (GO.into(), go.clone()),
                (PIDS_FILE.into(), pids.clone()),
            ],
        )
    };
    let mut requests = vec![
        played(PRINTS, "print_when_told"),
        played(ENDS, "exit_when_told"),
    ];
    requests.extend(
        (0..SHELLS).map(|index| request(&home, &format!("p{index:02}"), shell(), Vec::new())),
    );
    let next = Mutex::new(requests.iter());
    thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                while let Some(request) = next.lock().unwrap().next() {
                    node.start_pane(request);
                }
            });
        }
    });
    wait_until("the attached panes' first screens", || {
        requests
            .iter()
            .filter(|request| node.screens.drew(&request.pane))
            .count()
            >= hide_node_link::terminal::MAX_ATTACHED_PANES
    });
    std::fs::write(std::env::var_os(NODE_READY).unwrap(), b"ready").unwrap();
    loop {
        thread::park();
    }
}

/// B5 and B7: a node killed outright leaves its holders running; the next
/// node finds them by their records, attaches the first 64 by name at the
/// screen each holds now (output while no node was there included), keeps
/// the 65th running unattached, and reports the pane that ended meanwhile
/// with its recorded reason. The programs are the processes they were.
#[test]
fn a_node_killed_outright_finds_its_holders_again_and_attaches_sixty_four() {
    let folder = Folder::new();
    let ready = folder.home.path().join("node-ready");
    let pids = folder.home.path().join("pids");
    let started_at = Instant::now();
    let node = role(
        "node",
        &[
            (NODE_HOME, &folder.home.path().into()),
            (NODE_STATE, &folder.state.clone().into()),
            (NODE_READY, &ready.clone().into()),
            (GO, &folder.go().into()),
            (PIDS_FILE, &pids.into()),
        ],
    );
    let started = Instant::now();
    while !ready.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(240),
            "the node never started its panes"
        );
        #[allow(clippy::disallowed_methods)]
        // the role writes a file, not an event this test can wait on
        thread::sleep(Duration::from_millis(50));
    }
    let start_took = started_at.elapsed();
    let panes: Vec<String> = std::iter::once(PRINTS.to_owned())
        .chain(std::iter::once(ENDS.to_owned()))
        .chain((0..SHELLS).map(|index| format!("p{index:02}")))
        .collect();
    let before: BTreeMap<String, (Option<Identity>, Option<Identity>)> = panes
        .iter()
        .map(|pane| {
            let record = folder.record(pane);
            (pane.clone(), (record.holder, record.program))
        })
        .collect();

    // Killed outright: no release, no close.
    drop(node);
    std::fs::write(folder.go(), b"go").unwrap();
    wait_until("the pane told to end to end", || {
        folder.record(ENDS).phase == Phase::Ended
    });

    let recovered_at = Instant::now();
    let next = Node::start(folder.home.path(), &folder.state);
    let told = Arc::new(Told::default());
    next.holders().watch_facts(told.sink());
    let recovered = next.holders().recover(next.owner(), &next.service);
    assert!(recovered.complete, "{recovered:?}");
    let mut expected: Vec<String> = panes.iter().filter(|pane| *pane != ENDS).cloned().collect();
    expected.sort();
    let kept = expected.split_off(hide_node_link::terminal::MAX_ATTACHED_PANES);
    assert_eq!(recovered.attached, expected);
    assert_eq!(recovered.kept, kept);
    // The pane past the attach cap, whose screen nobody holds, is followed
    // as the others are.
    told.wait(&kept[0], "its facts", |_| true);
    assert_eq!(
        recovered.ended,
        vec![(ENDS.to_owned(), Some(EndReason::ProgramExited))]
    );
    let ended = next.reports.wait_state(ENDS, "ended");
    assert_eq!(ended.exit_category.as_deref(), Some("program_exited"));
    assert_eq!(next.reports.states(ENDS), ["ended"], "reported once");

    for pane in &recovered.attached {
        next.screens.first_full(pane);
    }
    let recover_took = recovered_at.elapsed();
    next.screens.wait_full(PRINTS, WHILE_AWAY);
    for pane in &kept {
        let limited = next.reports.wait_state(pane, "unavailable");
        assert_eq!(limited.exit_category.as_deref(), Some("attach_limit"));
        assert!(!next.screens.drew(pane), "{pane} past the cap was attached");
        let (holder, _) = before[pane];
        assert!(
            holder.unwrap().runs(),
            "{pane} past the cap stopped running"
        );
    }
    for pane in panes.iter().filter(|pane| *pane != ENDS) {
        let record = folder.record(pane);
        assert_eq!(
            (record.holder, record.program),
            before[pane],
            "{pane}'s holder or program changed"
        );
        assert!(record.holder.unwrap().runs());
    }
    eprintln!(
        "{} panes: started and attached in {:?}, found again and 64 drawn in {:?}",
        panes.len(),
        start_took,
        recover_took
    );
}

/// Ends a holder this test found by its identity, however the test ends.
struct EndsHolder(Identity);

impl Drop for EndsHolder {
    fn drop(&mut self) {
        if self.0.runs() {
            let _ = hide_platform::process::kill_tree(self.0.pid);
        }
    }
}

/// PRD engine-bundle-3-core-owns B17 and D-8: a holder whose record went
/// with its deleted state folder is not ended by the next node, which finds
/// no record of it; it runs on and ends when its shell does.
#[test]
fn a_holder_whose_record_was_deleted_runs_on_until_its_shell_ends() {
    let folder = Folder::new();
    let node = Node::start(folder.home.path(), &folder.state);
    node.start_pane(&folder.shell("b17"));
    node.screens.first_full("b17");
    let record = folder.record("b17");
    let holder = EndsHolder(record.holder.expect("the holder wrote itself in"));
    let program = record.program.expect("the holder started its shell");
    drop(node);
    std::fs::remove_dir_all(&folder.state).unwrap();

    let next = Node::start(folder.home.path(), &folder.state);
    let recovered = next.holders().recover(next.owner(), &next.service);
    assert!(recovered.complete, "{recovered:?}");
    assert!(
        recovered.attached.is_empty() && recovered.ended.is_empty(),
        "the next node knows nothing of the pane: {recovered:?}"
    );
    assert!(
        holder.0.runs(),
        "the next node ended a holder it holds no record of"
    );

    hide_platform::process::kill_tree(program.pid).unwrap();
    wait_until("the holder to end with its shell", || !holder.0.runs());
}

/// B9: a bad first message leaves the node's connection alone; a second
/// node that says hello takes the pane over, the first node's session ends
/// `taken_over` and is not tried again, and what the first node types after
/// never reaches the program.
#[test]
fn a_second_node_takes_the_pane_over_and_the_first_does_not_try_again() {
    let folder = Folder::new();
    let node = Node::start(folder.home.path(), &folder.state);
    node.start_pane(&folder.shell("t1"));
    node.screens.first_full("t1");
    let socket = layout::holder_socket(&folder.state, "t1");

    let mut wrong =
        hide_platform::ipc::LocalStream::connect_until(&socket, Instant::now() + HANG_LIMIT)
            .unwrap();
    std::io::Write::write_all(&mut wrong, b"\x10\x00\x00\x00not a node at all").unwrap();
    let mut answer = Vec::new();
    let _ = std::io::Read::read_to_end(&mut wrong, &mut answer);
    node.type_into("t1", print_ok());
    wait_until("the first node's input after a wrong opening", || {
        node.text("t1").contains("hide-ok")
    });

    let second = open_holder("t1", &socket, None).unwrap();
    let taken = node.reports.wait_state("t1", "taken_over");
    assert_eq!(taken.exit_category.as_deref(), Some("taken_over"));
    assert_eq!(taken.retry_decision, "none");
    let mut events = second.events;
    let (heard, closes): (Sender<String>, Receiver<String>) = channel();
    thread::spawn(move || {
        loop {
            if let SessionEvent::Ended { category, .. } = events.next_event() {
                let _ = heard.send(category.to_owned());
                return;
            }
        }
    });

    node.type_into("t1", &print_node("first-late"));
    let mut writer = second.writer;
    writer.input(&print_node("second")).unwrap();
    wait_until("the second node's input", || {
        node.text("t1").contains("second-node")
    });
    assert!(!node.text("t1").contains("first-late"));
    // The service's first retry would come five seconds after a close it
    // tries again; a takeover is never one.
    assert!(
        closes.recv_timeout(Duration::from_secs(7)).is_err(),
        "the first node took the pane back"
    );
    assert_eq!(
        node.reports.states("t1").last().map(String::as_str),
        Some("taken_over")
    );
}

fn fixture_record(folder: &Folder, pane: &str, protocol: u16, holder: u32, version_folder: &str) {
    let holder = Identity::of(holder).expect("the fixture runs");
    let owner = hide_kit::panes::Panes::open(folder.home.path(), &folder.state, |_| {})
        .unwrap()
        .owner()
        .to_owned();
    let written = PaneRecord {
        format: RECORD_FORMAT,
        pane: pane.to_owned(),
        owner,
        incarnation: 1,
        phase: Phase::Running,
        protocol,
        version_folder: version_folder.to_owned(),
        spec: String::new(),
        reservation: None,
        reserved_at_unix_ms: record::now_unix_ms(),
        boot: hide_platform::process::boot_id().ok(),
        holder: Some(holder),
        program: Some(holder),
        end: None,
    };
    record::write(&layout::pane_record(&folder.state, pane), &written).unwrap();
}

/// B10: a holder of the oldest version a node speaks to, frozen in
/// `fixture_v1.rs`, is found by its record and attached; the node draws its
/// screen and sends it input and a size, reads it, and ends it, and sends
/// nothing that version does not know.
#[test]
fn a_node_works_a_holder_of_the_oldest_version_it_supports() {
    assert_eq!(
        hide_holder_protocol::OLDEST_HOLDER_VERSION,
        1,
        "freeze a fixture for the new oldest version"
    );
    let folder = Folder::new();
    hide_platform::fs::private::create_dir_all(&layout::holder_panes(&folder.state)).unwrap();
    let log = folder.home.path().join("fixture.log");
    let fixture = role(
        "holder_v1",
        &[
            (
                fixture_v1::SOCKET,
                &layout::holder_socket(&folder.state, "f1").into(),
            ),
            (fixture_v1::LOG, &log.clone().into()),
            (
                fixture_v1::RECORD,
                &layout::pane_record(&folder.state, "f1").into(),
            ),
            (
                fixture_v1::LOCK,
                &layout::pane_records_lock(&folder.state).into(),
            ),
        ],
    );
    let logged = || std::fs::read_to_string(&log).unwrap_or_default();
    wait_until("the fixture to listen", || logged().contains("listening"));
    fixture_record(&folder, "f1", 1, fixture.0.id(), "v1-fixture");

    let node = Node::start(folder.home.path(), &folder.state);
    let told = Arc::new(Told::default());
    node.holders().watch_facts(told.sink());
    let recovered = node.holders().recover(node.owner(), &node.service);
    assert_eq!(recovered.attached, vec!["f1".to_owned()], "{recovered:?}");
    node.screens.wait_full("f1", fixture_v1::SCREEN_TEXT);
    // A version 1 holder built before watches refuses the watch opening,
    // the one opening a node tries that such a holder does not know, and
    // the engine is told once that the pane's facts are not followed.
    told.wait("f1", "unsupported", |facts| {
        facts.screen
            == ScreenRead::Unknown {
                reason: UNSUPPORTED_HOLDER.to_owned(),
            }
    });

    node.type_into("f1", b"abc");
    wait_until("the input", || logged().contains("input 616263"));
    node.service.control(TerminalControl::Resize {
        pane: "f1".to_owned(),
        size: GridSize {
            rows: 30,
            cols: 100,
        },
    });
    wait_until("the resize", || logged().contains("resize 30 100"));
    let redraws = || logged().lines().filter(|line| *line == "redraw").count();
    let before = redraws();
    node.service.redraw("f1");
    wait_until("the redraw", || redraws() > before);
    match node.holders().read("f1", 10).unwrap() {
        Answer::Text { text, .. } => assert_eq!(text, fixture_v1::READ_TEXT),
        other => panic!("a read answered {other:?}"),
    }
    let ended = node
        .holders()
        .terminate(node.owner(), &node.service, "f1")
        .unwrap();
    assert_eq!(ended.phase, Phase::Ended);
    let logged = logged();
    assert!(logged.contains("terminate"), "{logged}");
    assert!(
        logged
            .lines()
            .filter(|line| line.starts_with("unknown"))
            .all(|line| line == "unknown 0x19"),
        "the node sent what version 1 does not know:\n{logged}"
    );
    assert_eq!(told.count("f1"), 1, "the pane was told more than once");
    // Every hello offers the oldest version a node speaks to.
    assert!(
        logged
            .lines()
            .filter(|line| line.starts_with("hello"))
            .all(|line| line.starts_with("hello 1 ")),
        "{logged}"
    );
}

/// D-18, D-21: a node tells the engine a pane's facts from the moment it
/// starts, gives its holder the first rule set its evaluator reads, says
/// when none does, takes the rules away, and tells a sink set later the last
/// facts at once.
#[test]
fn a_node_tells_its_panes_facts_read_by_the_rules_its_holder_reads() {
    let folder = Folder::new();
    let node = Node::start(folder.home.path(), &folder.state);
    let told = Arc::new(Told::default());
    node.holders().watch_facts(told.sink());
    let script = Script::new(&folder.home, vec![b"\x1b]2;job\x07busy\r\n".to_vec()]);
    node.start_pane(&request(
        folder.home.path(),
        "w1",
        role_program("script"),
        vec![
            (ROLE.into(), "script".into()),
            (SCRIPT.into(), script.folder.clone().into()),
        ],
    ));
    told.wait("w1", "the title", |facts| {
        facts.title.as_deref() == Some("job") && facts.screen == ScreenRead::NoRules
    });

    let set = |min_engine_version: u32, manifest: &str| RuleSet {
        source: "built-in".to_owned(),
        version: "1".to_owned(),
        min_engine_version,
        manifest: manifest.to_owned(),
    };
    let newer = set(
        hide_screen_rules::ENGINE_VERSION + 1,
        r#"{"version":"2","min_engine_version":99,"rules":[]}"#,
    );
    let busy = set(
        1,
        r#"{"version":"1","min_engine_version":1,"rules":[{"id":"busy","state":"working","contains":["busy"]}]}"#,
    );
    let rules = |sets: Vec<RuleSet>| ScreenRules {
        agent: "test".to_owned(),
        sets,
    };
    node.holders()
        .set_rules("w1", Some(rules(vec![newer.clone(), busy])))
        .unwrap();
    told.wait("w1", "the screen read by the set that fits", |facts| {
        facts.screen
            == ScreenRead::Read {
                state: ScreenState::Working,
                rule: Some("busy".to_owned()),
            }
    });
    node.holders()
        .set_rules("w1", Some(rules(vec![newer])))
        .unwrap();
    told.wait("w1", "no rules for its holder", |facts| {
        facts.screen
            == ScreenRead::Unknown {
                reason: NO_RULES_FOR_HOLDER.to_owned(),
            }
    });
    node.holders().set_rules("w1", None).unwrap();
    let last = told.wait("w1", "no rules", |facts| {
        facts.screen == ScreenRead::NoRules
    });
    assert!(matches!(
        node.holders().set_rules("unknown", None),
        Err(HolderError::NoPane { .. })
    ));

    let later = Arc::new(Told::default());
    node.holders().watch_facts(later.sink());
    assert_eq!(later.wait("w1", "the last facts", |_| true), last);
    node.holders()
        .terminate(node.owner(), &node.service, "w1")
        .unwrap();
}

/// B19: a pane whose watch ends some other way than with the pane (here
/// another watch takes it) is told once that its facts are no longer
/// followed, with why, and keeps what it last showed.
#[test]
fn a_pane_whose_watch_another_took_reads_unknown_with_why() {
    let folder = Folder::new();
    let node = Node::start(folder.home.path(), &folder.state);
    let told = Arc::new(Told::default());
    node.holders().watch_facts(told.sink());
    let script = Script::new(&folder.home, vec![b"\x1b]2;job\x07busy\r\n".to_vec()]);
    node.start_pane(&request(
        folder.home.path(),
        "l1",
        role_program("script"),
        vec![
            (ROLE.into(), "script".into()),
            (SCRIPT.into(), script.folder.clone().into()),
        ],
    ));
    told.wait("l1", "the title", |facts| {
        facts.title.as_deref() == Some("job")
    });

    let (_other, _) = hide_holder_protocol::watch(
        &layout::holder_socket(&folder.state, "l1"),
        vec![hide_holder_protocol::Capability::Facts],
        Instant::now() + HANG_LIMIT,
    )
    .unwrap();
    let lost = told.wait("l1", "its watch lost", |facts| {
        facts.screen
            == ScreenRead::Unknown {
                reason: WATCH_LOST.to_owned(),
            }
    });
    assert_eq!(lost.title.as_deref(), Some("job"));
    node.holders()
        .terminate(node.owner(), &node.service, "l1")
        .unwrap();
}

/// B10: a holder older than the oldest version a node speaks to is reported
/// `unsupported_holder_version`, never attached or ended, and its version
/// folder stays.
#[test]
fn a_holder_older_than_the_oldest_supported_is_reported_and_left_running() {
    let folder = Folder::new();
    let old_folder = layout::holders_root(folder.home.path()).join("v0-old");
    std::fs::create_dir_all(&old_folder).unwrap();
    let pids = folder.home.path().join("pids");
    let old = role(
        "exit_when_told",
        &[(GO, &folder.go().into()), (PIDS_FILE, &pids.into())],
    );
    let running = Running::of(old.0.id()).unwrap();
    hide_platform::fs::private::create_dir_all(&layout::pane_records(&folder.state)).unwrap();
    fixture_record(&folder, "o1", 0, old.0.id(), "v0-old");

    let node = Node::start(folder.home.path(), &folder.state);
    let recovered = node.holders().recover(node.owner(), &node.service);
    assert_eq!(
        recovered.unsupported,
        vec!["o1".to_owned()],
        "{recovered:?}"
    );
    assert!(recovered.attached.is_empty());
    let reported = node.reports.wait_state("o1", "unsupported");
    assert_eq!(
        reported.exit_category.as_deref(),
        Some("unsupported_holder_version")
    );
    assert!(running.is_running(), "the old holder was ended");
    assert!(old_folder.exists(), "the old version folder was removed");
    assert!(!node.screens.drew("o1"));
}

/// What the node logged to its diagnostic log while a test listened
/// ([`Listening`]), in order.
static DIAGNOSTICS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
static LISTENERS: AtomicUsize = AtomicUsize::new(0);

/// The node's sink in this test binary: every record still reaches the
/// error stream, as with no sink, so the other tests of the binary log as
/// before; it is kept only while a test listens.
fn keep_diagnostic(record: serde_json::Value) {
    eprintln!("{record}");
    if LISTENERS.load(Ordering::SeqCst) > 0 {
        DIAGNOSTICS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }
}

/// A test reading the node's diagnostic log; the records kept for it go
/// when the last listener does.
struct Listening;

impl Drop for Listening {
    fn drop(&mut self) {
        let mut kept = DIAGNOSTICS.lock().unwrap_or_else(PoisonError::into_inner);
        if LISTENERS.fetch_sub(1, Ordering::SeqCst) == 1 {
            kept.clear();
        }
    }
}

/// Makes the node's diagnostic log readable by [`wait_diagnostic`] until
/// the answer drops. The sink is the process's, installed once, and passes
/// every record on to the error stream; tests running at once share it, so
/// each matches only the records that name what it made.
fn diagnostics() -> Listening {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        assert!(
            hide_node::diagnostics::install(keep_diagnostic),
            "another sink took the node's diagnostic log"
        );
    });
    let _kept = DIAGNOSTICS.lock().unwrap_or_else(PoisonError::into_inner);
    LISTENERS.fetch_add(1, Ordering::SeqCst);
    Listening
}

/// Waits for the node to log a record of `kind` that `matches`, and answers
/// it.
fn wait_diagnostic(kind: &str, matches: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
    let mut found = None;
    wait_until(&format!("a {kind} diagnostic"), || {
        found = DIAGNOSTICS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|record| record["kind"] == kind && matches(record))
            .cloned();
        found.is_some()
    });
    found.unwrap()
}

fn folder_name(dir: &Path) -> String {
    dir.file_name().unwrap().to_string_lossy().into_owned()
}

/// B20 through a node's own start: a node with the switch on, started as the
/// daemon starts one, removes the version folder of an older build nothing
/// runs from once it has found its holders again, keeps this build's folder
/// and the older build's folder a pane still runs from (its holder too old
/// to speak to, left running), and logs what it did.
#[test]
fn a_starting_node_removes_the_old_version_folders_nothing_runs_from() {
    let _listening = diagnostics();
    let folder = Folder::new();
    let home = folder.home.path();
    let current = source();
    let current_name =
        hide_kit::holders::version_name(crate::support::current_version(), &current).unwrap();
    drop(
        folder
            .home
            .prepare(crate::support::current_version(), &current),
    );

    // An older build's folder a pane still runs from, by its record.
    let older = folder.home.prepare(0, &current);
    let older_name = folder_name(&older.dir);
    let holder =
        crate::support::start_ready(&older.program, &folder.home.socket("o"), &shell(), &[]);
    drop(older);
    hide_platform::fs::private::create_dir_all(&layout::pane_records(&folder.state)).unwrap();
    fixture_record(&folder, "o1", 0, holder.process.pid, &older_name);

    // An older build's folder nothing runs from.
    let unused_program = home.join("unused-holder");
    std::fs::write(&unused_program, b"an older holder").unwrap();
    let unused = hide_kit::holders::Source {
        program: unused_program,
        conpty: current.conpty.clone(),
    };
    let unused_dir = folder.home.prepare(0, &unused).dir.clone();
    let unused_name = folder_name(&unused_dir);

    let reports = Arc::new(Reports::default());
    let terminals = Holders::start_from(
        HolderFolders {
            home,
            state_dir: &folder.state,
        },
        current,
        Arc::new(Screens::default()) as Arc<dyn OutputSink>,
        Arc::clone(&reports) as Arc<dyn ReportSink>,
    )
    .unwrap();

    // Assembly starts no record mutation. The connection acquires ownership
    // and completes recovery before it prunes, just as the daemon's does.
    let _owned = Arc::new(LocalPanes::new(
        Arc::clone(&terminals.holders),
        Arc::clone(&terminals.service),
        PaneShell::Fixed(shell()),
    ))
    .own(HANG_LIMIT)
    .unwrap();

    // This test's prune: the one that names its unused folder.
    let pruned = wait_diagnostic("pane_holder.folders_pruned", |record| {
        record["removed"]
            .as_array()
            .is_some_and(|removed| removed.iter().any(|name| name == unused_name.as_str()))
    });
    assert_eq!(
        pruned["removed"],
        serde_json::json!([unused_name]),
        "{pruned}"
    );
    assert_eq!(
        pruned["in_use"],
        serde_json::json!([older_name]),
        "{pruned}"
    );
    assert_eq!(pruned["failed"], 0, "{pruned}");
    assert!(!unused_dir.exists(), "{unused_name} was kept");
    for name in [&current_name, &older_name] {
        let dir = layout::holder_dir(home, name);
        assert!(
            dir.join(hide_kit::holders::program_name()).is_file(),
            "{name} lost its holder"
        );
    }
    assert_eq!(
        reports
            .wait_state("o1", "unsupported")
            .exit_category
            .as_deref(),
        Some("unsupported_holder_version")
    );
    assert!(holder.process.is_running(), "the older holder was ended");
}

/// B14 through the node's own transport: a node accepts the holder's
/// notices, logs a refusal of the program's answers with the pane's id, and
/// its session of the pane goes on.
#[test]
fn a_node_logs_the_answers_its_holder_refused_with_the_pane_and_keeps_the_session() {
    let _listening = diagnostics();
    let home = Home::new();
    let mut burst = b"\x1b]11;?\x07".repeat(6000);
    burst.extend_from_slice(b"asked");
    let script = crate::support::Script::new(&home, vec![b"ready".to_vec(), burst]);
    let release: OsString = home.path().join("release").into();
    let holder = script.start_with(&home, &[(crate::support::HOLD_WRITES, &release)]);
    let transport = open_holder("q1", &holder.socket, None).unwrap();
    let mut events = transport.events;
    let (drawn, frames) = channel();
    thread::spawn(move || {
        loop {
            let event = events.next_event();
            let ended = matches!(event, SessionEvent::Ended { .. });
            if drawn.send(event).is_err() || ended {
                return;
            }
        }
    });
    script.go(1);
    let refused = wait_diagnostic("pane_holder.answers_refused", |record| {
        record["pane_id"] == "q1"
    });
    assert_eq!(refused["count"], 1, "{refused}");
    assert!(
        refused["message"].as_str().unwrap().contains("65536"),
        "{refused}"
    );
    // The session read past the notice: the burst's end is drawn, and the
    // session did not end.
    let mut text = Vec::new();
    let deadline = Instant::now() + HANG_LIMIT;
    while !text_of(&text).contains("asked") {
        let left = deadline.saturating_duration_since(Instant::now());
        match frames
            .recv_timeout(left)
            .expect("the burst's end was never drawn")
        {
            SessionEvent::Frame { bytes, .. } => text.extend_from_slice(&bytes),
            SessionEvent::Clipboard { .. } => {}
            SessionEvent::Ended { category, message } => {
                panic!("the session ended ({category}): {message}")
            }
        }
    }
}
