//! A node's holder panes, through hide-node's own code with the built
//! holder (PRD engine-bundle-2-pane-holder D-10, D-11, D-15, D-17, B1, B2,
//! B7, B9, B10): the development switch off opens Herdr and starts no
//! holder; on, a start shows the pane's first full screen, takes input and
//! converges when asked again; a node killed outright finds its holders
//! again when the next one starts, attaches 64 and keeps the rest running,
//! and reports the pane that ended meanwhile; a takeover ends the first
//! node's session for good; and the node speaks to a holder of the oldest
//! version it supports and leaves an older one running.
//!
//! The node is `hide_node::terminal::Service` with `holder::Holders`, as the
//! daemon builds it, in this test process, or in the `node` role when the
//! test kills it.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use hide_holder_protocol::record::{self, EndReason, Identity, PaneRecord, Phase, RECORD_FORMAT};
use hide_holder_protocol::{Answer, Size};
use hide_kit::layout;
use hide_kit::panes::{StartRequest, Started};
use hide_node::terminal::holder::{Holders, Switch, open_holder};
use hide_node::terminal::protocol::SessionEvent;
use hide_node::terminal::{
    Attacher, Mode, OutputSink, ReportSink, RetryPolicy, Service, SessionParts,
};
use hide_node_link::terminal::{
    GridSize, KeyTarget, TerminalControl, TerminalNode, TerminalReport,
};

use crate::fixture_v1;
use crate::role::GO;
use crate::support::{
    HANG_LIMIT, Home, PIDS_FILE, ROLE, Running, node_theme, print_ok, role_program, shell, source,
    wait_until,
};

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

/// The node's Herdr in these tests: it answers no attach, and says which
/// panes it was asked to open.
struct Herdr(Mutex<Sender<String>>);

impl Attacher for Herdr {
    fn open(&self, pane: &str, _: Mode, _: u16, _: u16) -> Result<SessionParts, String> {
        let _ = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(pane.to_owned());
        Err("this test has no Herdr".to_owned())
    }
}

/// A node over one state folder, as the daemon builds it.
struct Node {
    service: Service,
    holders: Option<Holders>,
    screens: Arc<Screens>,
    reports: Arc<Reports>,
    herdr_opened: Mutex<Receiver<String>>,
}

impl Node {
    fn start(switch: Switch, home: &Path, state: &Path) -> Self {
        let screens = Arc::new(Screens::default());
        let reports = Arc::new(Reports::default());
        let holders = Holders::open(
            switch,
            home,
            state,
            source(),
            Arc::clone(&reports) as Arc<dyn ReportSink>,
        )
        .unwrap();
        let (opened, herdr_opened) = channel();
        let herdr: Box<dyn Attacher> = Box::new(Herdr(Mutex::new(opened)));
        let attacher = match &holders {
            Some(holders) => holders.attacher(herdr),
            None => herdr,
        };
        let service = Service::start(
            attacher,
            Arc::clone(&screens) as Arc<dyn OutputSink>,
            Arc::clone(&reports) as Arc<dyn ReportSink>,
            RetryPolicy::Automatic,
        )
        .unwrap();
        Self {
            service,
            holders,
            screens,
            reports,
            herdr_opened: Mutex::new(herdr_opened),
        }
    }

    fn holders(&self) -> &Holders {
        self.holders.as_ref().expect("the switch is on")
    }

    fn start_pane(&self, request: &StartRequest) -> Started {
        self.holders()
            .start(&self.service, request)
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

/// B1: with the switch off a node opens its panes through Herdr and makes
/// no holder, version folder, socket or record.
#[test]
fn a_node_with_the_switch_off_opens_herdr_and_starts_no_holder() {
    let folder = Folder::new();
    let node = Node::start(Switch::Off, folder.home.path(), &folder.state);
    assert!(node.holders.is_none());
    node.service.control(TerminalControl::Attach {
        pane: "w1:p1".to_owned(),
        size: Some(SIZE),
        manual: false,
    });
    assert_eq!(
        node.herdr_opened
            .lock()
            .unwrap()
            .recv_timeout(HANG_LIMIT)
            .unwrap(),
        "w1:p1",
        "the pane opens through Herdr"
    );
    assert!(!layout::holders_root(folder.home.path()).exists());
    assert!(!layout::pane_records(&folder.state).exists());
    assert!(!layout::holder_panes(&folder.state).exists());
}

/// B2: a started pane shows its first full screen, takes the bytes typed
/// into it, answers the same start with the same pane, and ends whole.
#[test]
fn a_started_pane_shows_its_first_full_screen_takes_input_and_converges() {
    let folder = Folder::new();
    let node = Node::start(Switch::On, folder.home.path(), &folder.state);
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
    let ended = node.holders().terminate(&node.service, "s1").unwrap();
    assert_eq!(ended.phase, Phase::Ended);
    assert_eq!(ended.end.map(|end| end.reason), Some(EndReason::Terminated));
    assert!(!holder.runs());
}

/// A redraw, and a view at the grid the pane already runs at, each bring a
/// full frame from the holder, and what the program prints after either is
/// drawn: the node waits for a full frame after both, and a holder answers
/// a same-size resize with nothing, so each asks for the screen with a
/// redraw instead.
#[test]
fn a_redraw_on_a_holder_pane_draws_its_screen_again_and_what_follows() {
    let folder = Folder::new();
    let node = Node::start(Switch::On, folder.home.path(), &folder.state);
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
    node.holders().terminate(&node.service, "r1").unwrap();
}

/// The `node` role: starts every pane of the kill test, eight at once as a
/// node's starts come, says so once each attached pane drew its first full
/// screen, and lives until the test kills it.
pub fn node_role() {
    let home = PathBuf::from(std::env::var_os(NODE_HOME).unwrap());
    let state = PathBuf::from(std::env::var_os(NODE_STATE).unwrap());
    let go = std::env::var_os(GO).unwrap();
    let pids = std::env::var_os(PIDS_FILE).unwrap();
    let node = Node::start(Switch::On, &home, &state);
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
    let next = Node::start(Switch::On, folder.home.path(), &folder.state);
    let recovered = next.holders().recover(&next.service);
    assert!(recovered.complete, "{recovered:?}");
    let mut expected: Vec<String> = panes.iter().filter(|pane| *pane != ENDS).cloned().collect();
    expected.sort();
    let kept = expected.split_off(hide_node_link::terminal::MAX_ATTACHED_PANES);
    assert_eq!(recovered.attached, expected);
    assert_eq!(recovered.kept, kept);
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

/// B9: a bad first message leaves the node's connection alone; a second
/// node that says hello takes the pane over, the first node's session ends
/// `taken_over` and is not tried again, and what the first node types after
/// never reaches the program.
#[test]
fn a_second_node_takes_the_pane_over_and_the_first_does_not_try_again() {
    let folder = Folder::new();
    let node = Node::start(Switch::On, folder.home.path(), &folder.state);
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

    let second = open_holder(&socket, None).unwrap();
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
    let mut writer = second.writer.unwrap();
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

    let node = Node::start(Switch::On, folder.home.path(), &folder.state);
    let recovered = node.holders().recover(&node.service);
    assert_eq!(recovered.attached, vec!["f1".to_owned()], "{recovered:?}");
    node.screens.wait_full("f1", fixture_v1::SCREEN_TEXT);

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
    let ended = node.holders().terminate(&node.service, "f1").unwrap();
    assert_eq!(ended.phase, Phase::Ended);
    let logged = logged();
    assert!(logged.contains("terminate"), "{logged}");
    assert!(
        !logged.contains("unknown"),
        "the node sent what version 1 does not know:\n{logged}"
    );
    // Every hello offers the oldest version a node speaks to.
    assert!(
        logged
            .lines()
            .filter(|line| line.starts_with("hello"))
            .all(|line| line.starts_with("hello 1 ")),
        "{logged}"
    );
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

    let node = Node::start(Switch::On, folder.home.path(), &folder.state);
    let recovered = node.holders().recover(&node.service);
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
