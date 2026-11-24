//! One guest's node process as this machine's hided runs it (PRD
//! device-connect-iroh D-77 (6)): started from the guest's own build in
//! the guest's folder, read on its standard streams, started again when it
//! crashes and left stopped when it crashes too often.
//!
//! The process is another build's, so everything this file knows of it is
//! `hide_peer::protocol::launch`: its arguments, its ready line, the lines
//! after it and its exit codes. One thread supervises one guest's node for
//! as long as that node is wanted; the node itself ends when this daemon
//! does, because its input closes, and its pane holders stay.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

use hide_peer::protocol::PeerKey;
use hide_peer::protocol::build;
use hide_peer::protocol::launch::{self, EndMode, FromNode, Position, Ready, ToNode};
use hide_peer::protocol::manage::NodeStop;
use hide_peer::protocol::tap;
use hide_platform::process::OwnedChild;
use serde_json::json;
use tokio::sync::watch;

/// Starts a guest's node as this daemon's child: it ends with this daemon,
/// as every child it starts does unless that child is a pane holder, which
/// leaves (D-77 (6)). The node's work is to start the guest's holders
/// ([`hide_platform::process::start_resident`]), so on Windows its job lets
/// a resident leave; a job that let none leave refused every holder, and no
/// terminal a paired computer opened on a Windows host ever started.
pub fn start(command: &mut Command) -> std::io::Result<OwnedChild> {
    OwnedChild::spawn_letting_residents_leave(command)
}

/// Where a guest's node stands, as its guest is told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Not running, why, and whether this host starts it again by itself.
    Stopped {
        reason: NodeStop,
        retry: bool,
        detail: Option<String>,
    },
    /// Being started.
    Starting,
    /// Running and bound: its link and tap endpoints take connections.
    Ready(Running),
}

/// What a running node said of itself in its ready line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub build: String,
    pub build_id: String,
    pub pid: u32,
    pub link: PathBuf,
    pub tap: PathBuf,
    pub capabilities: Vec<String>,
}

impl Phase {
    fn ended() -> Self {
        Self::Stopped {
            reason: NodeStop::Ended,
            retry: false,
            detail: None,
        }
    }
}

/// How many of a node's events wait for the guest's stream to take them
/// before the node's output is left unread, which holds the node's next
/// event in its box.
const EVENTS_WAITING: usize = 64;

/// The guest's management stream that asked for the node's box: where the
/// node's events go until that stream ends or another asks.
struct Listener {
    id: u64,
    events: tokio::sync::mpsc::Sender<serde_json::Value>,
}

/// How long a crashed node stays down before it is started again.
const CRASH_RESTART_AFTER: Duration = Duration::from_secs(1);

/// How one run of the node process ended.
#[derive(Debug, PartialEq, Eq)]
enum Exit {
    /// As asked, with its last pane, or because it was told to make way
    /// for another build.
    Ended,
    UpdateNeeded(String),
    FolderRefused(String),
    FolderHeld,
    /// It asked to be started again, for a holder it lost.
    Restart,
    Crashed(String),
}

/// Reads a node's exit code as the launch contract names it. A code the
/// contract does not name, and an end by a signal, is a crash.
fn exit_of(code: Option<i32>) -> Exit {
    match code {
        Some(launch::EXIT_ENDED) => Exit::Ended,
        Some(launch::EXIT_UNKNOWN_WORD) => Exit::UpdateNeeded(
            "the guest's build does not know how this host starts a node".to_owned(),
        ),
        Some(launch::EXIT_FOLDER_REFUSED) => {
            Exit::FolderRefused("the node could not make its folder or an endpoint".to_owned())
        }
        Some(launch::EXIT_FOLDER_HELD) => Exit::FolderHeld,
        Some(launch::EXIT_RESTART) => Exit::Restart,
        Some(code) => Exit::Crashed(format!("the node exited with code {code}")),
        None => Exit::Crashed("the node was ended by a signal".to_owned()),
    }
}

/// What follows a run of the node that ended as `exit` at `now`: the phase
/// its guest is told, and how long until it is started again, `None` to
/// leave it stopped. `held` counts the starts in a row that found another
/// node in the folder.
fn follows(
    exit: Exit,
    held: &mut u32,
    crashes: &mut VecDeque<Instant>,
    now: Instant,
) -> (Phase, Option<Duration>) {
    let stopped = |reason, retry, detail: String| Phase::Stopped {
        reason,
        retry,
        detail: Some(detail),
    };
    match exit {
        Exit::Ended => (Phase::ended(), None),
        Exit::UpdateNeeded(detail) => (stopped(NodeStop::UpdateNeeded, false, detail), None),
        Exit::FolderRefused(detail) => (stopped(NodeStop::FolderRefused, false, detail), None),
        Exit::FolderHeld if *held < launch::FOLDER_HELD_RETRIES => {
            *held += 1;
            (
                Phase::Starting,
                Some(Duration::from_secs(launch::FOLDER_HELD_RETRY_SECS)),
            )
        }
        // The node's own word that it is to be started again is no crash
        // to tell its guest of, but it is counted with them: a build that
        // asks at every start is stopped like one that crashes.
        Exit::Restart if !crashed_too_often(crashes, now) => {
            *held = 0;
            (Phase::Starting, Some(Duration::ZERO))
        }
        Exit::Restart => (
            stopped(
                NodeStop::Crashed,
                false,
                "the node asked to be started again too often".to_owned(),
            ),
            None,
        ),
        exit => {
            let detail = match exit {
                Exit::Crashed(detail) => detail,
                _ => "another node still holds the guest's folder".to_owned(),
            };
            *held = 0;
            match crashed_too_often(crashes, now) {
                true => (stopped(NodeStop::Crashed, false, detail), None),
                false => (
                    stopped(NodeStop::Crashed, true, detail),
                    Some(CRASH_RESTART_AFTER),
                ),
            }
        }
    }
}

/// Records a crash at `now` and answers whether the node crashed more
/// often than it is started again for: more than [`launch::CRASHES`] times
/// within [`launch::CRASH_WINDOW_SECS`] (B70).
fn crashed_too_often(crashes: &mut VecDeque<Instant>, now: Instant) -> bool {
    let window = Duration::from_secs(launch::CRASH_WINDOW_SECS);
    crashes.push_back(now);
    while crashes
        .front()
        .is_some_and(|earliest| now.duration_since(*earliest) > window)
    {
        crashes.pop_front();
    }
    crashes.len() > launch::CRASHES
}

/// The ready line read as this host may use it: a version this build
/// speaks, a tap it speaks, and endpoints that are names inside the node's
/// own folder, never a path out of it.
fn admit(ready: Ready, node_dir: &Path) -> Result<Running, Exit> {
    let named = |name: &str| {
        !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != ".."
    };
    if ready.guest_node != launch::GUEST_NODE
        || ready.tap_versions[0] > tap::NEWEST_TAP
        || ready.tap_versions[1] < tap::OLDEST_TAP
    {
        return Err(Exit::UpdateNeeded(format!(
            "the node speaks launch contract {} and tap {:?}; this host speaks {} and tap [{}, {}]",
            ready.guest_node,
            ready.tap_versions,
            launch::GUEST_NODE,
            tap::OLDEST_TAP,
            tap::NEWEST_TAP
        )));
    }
    if !named(&ready.link) || !named(&ready.tap) {
        return Err(Exit::Crashed(
            "the node named an endpoint outside its folder".to_owned(),
        ));
    }
    Ok(Running {
        build: ready.build,
        build_id: ready.build_id,
        pid: ready.pid,
        link: node_dir.join(&ready.link),
        tap: node_dir.join(&ready.tap),
        capabilities: ready.capabilities,
    })
}

/// One whole line of at most [`launch::MAX_LINE`] bytes, without its end;
/// `None` at the end of the stream, and for a line that is longer, after
/// which nothing more is read.
fn read_line(reader: &mut impl BufRead) -> Option<String> {
    let mut line = Vec::new();
    let read = reader
        .by_ref()
        .take(launch::MAX_LINE as u64)
        .read_until(b'\n', &mut line)
        .ok()?;
    if read == 0 || line.pop() != Some(b'\n') {
        return None;
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    String::from_utf8(line).ok()
}

#[derive(Default)]
struct Control {
    /// The running process's input: a line written here reaches it, and
    /// dropping it tells the node its host is gone.
    input: Option<ChildStdin>,
    /// A supervisor thread runs for this guest.
    supervised: bool,
    /// The program the supervisor runs, or starts again after a crash.
    current: Option<PathBuf>,
    /// The program the supervisor starts next, in place of `current`.
    next: Option<PathBuf>,
    /// The program last asked for, which a retry starts again.
    last: Option<PathBuf>,
    /// The daemon is ending: nothing is started again.
    stopping: bool,
    /// When the node crashed, within the window crashes are counted in.
    crashes: VecDeque<Instant>,
    listener: Option<Listener>,
    /// The listeners there have been, which names each.
    listeners: u64,
}

pub struct GuestNode {
    guest: PeerKey,
    /// The node's own state folder, `<state>/g/<key>/n`.
    node_dir: PathBuf,
    /// The account's home, as this daemon read it at its start.
    home: PathBuf,
    /// The state folder the node's process is told, for the node of the
    /// device helper's folder: this daemon's own. The helper ran under the
    /// login this daemon runs under, so the panes it started ask in this
    /// folder, and a node given the login's variables alone would look for
    /// them in the account's default one (D-49). That rests on one
    /// assumption: the SSH login the helper was started over resolved the
    /// same state folder as this daemon, which holds for the default one;
    /// a login whose `XDG_STATE_HOME` differs from the daemon's leaves the
    /// carried-on panes' hooks asking elsewhere. A guest's own node names
    /// its folder itself and is told none.
    process_state: Option<PathBuf>,
    phase: watch::Sender<Phase>,
    control: Mutex<Control>,
    wake: Condvar,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A supervisor that panicked left values that are still what they say.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl GuestNode {
    pub fn new(
        guest: PeerKey,
        node_dir: PathBuf,
        home: PathBuf,
        process_state: Option<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            guest,
            node_dir,
            home,
            process_state,
            phase: watch::Sender::new(Phase::ended()),
            control: Mutex::default(),
            wake: Condvar::new(),
        })
    }

    pub fn phase(&self) -> watch::Receiver<Phase> {
        self.phase.subscribe()
    }

    /// The node as it runs now, when it does.
    pub fn running(&self) -> Option<Running> {
        match &*self.phase.borrow() {
            Phase::Ready(running) => Some(running.clone()),
            _ => None,
        }
    }

    fn record(&self, kind: &str, extra: serde_json::Value) {
        let mut record = json!({
            "component": "guest_node_host",
            "kind": kind,
            "guest": self.guest.short(),
        });
        if let (Some(record), Some(extra)) = (record.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                record.insert(key.clone(), value.clone());
            }
        }
        hide_core::diagnostic!(record);
    }

    /// Runs the guest's node from `program`, the node program of one build
    /// in the guest's install root. A node of that program that runs, is
    /// being started or is about to be started again is left as it is; a
    /// node of another build is told its host is gone, which leaves its
    /// panes to their holders, and this build's node takes the folder after
    /// it (D-77 (2)).
    pub fn start(self: &Arc<Self>, program: PathBuf) {
        let mut control = lock(&self.control);
        if control.stopping {
            return;
        }
        control.last = Some(program.clone());
        if control.supervised {
            if control.next.as_ref().or(control.current.as_ref()) != Some(&program) {
                control.next = Some(program);
                control.input = None;
                drop(control);
                self.wake.notify_all();
            }
            return;
        }
        control.supervised = true;
        control.next = Some(program);
        self.phase.send_replace(Phase::Starting);
        let node = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name(format!("guest-{}", self.guest.short()))
            .spawn(move || node.supervise());
        if let Err(error) = spawned {
            control.supervised = false;
            control.next = None;
            self.phase.send_replace(Phase::Stopped {
                reason: NodeStop::Other,
                retry: false,
                detail: Some(format!("the node's supervisor could not start: {error}")),
            });
        }
    }

    /// Starts the running node again from the same build: it is told its
    /// host is gone, which leaves its panes to their holders, and the node
    /// that follows takes them. Nothing happens to a node that does not
    /// run.
    pub fn restart(&self) {
        let mut control = lock(&self.control);
        if control.stopping || !control.supervised || control.next.is_some() {
            return;
        }
        control.next = control.current.clone();
        control.input = None;
        drop(control);
        self.wake.notify_all();
    }

    /// The guest asked for its node again after this host stopped starting
    /// it (B70): the crashes counted so far are forgotten and the build
    /// last asked for is started. `false` when this daemon was never asked
    /// for a build, so the guest has to name one.
    pub fn retry(self: &Arc<Self>) -> bool {
        let last = {
            let mut control = lock(&self.control);
            control.crashes.clear();
            control.last.clone()
        };
        match last {
            Some(program) => {
                self.start(program);
                true
            }
            None => false,
        }
    }

    /// Tells the running node to end, as its guest asked.
    pub fn end(&self, mode: EndMode) {
        self.tell(&ToNode::End(mode));
    }

    /// Writes the running node one line; a node that is not running, or
    /// whose input is gone, hears nothing, and its exit says so.
    pub fn tell(&self, line: &ToNode) {
        let Ok(mut text) = serde_json::to_string(line) else {
            return;
        };
        text.push('\n');
        let mut control = lock(&self.control);
        let written = control.input.as_mut().is_some_and(|input| {
            input
                .write_all(text.as_bytes())
                .and_then(|()| input.flush())
                .is_ok()
        });
        if !written {
            control.input = None;
        }
    }

    /// The guest's core is connected and recorded the node's box as far as
    /// `position`: the node sends what follows, and each event comes out
    /// of the receiver as the management message it is. `None` for a node
    /// that is not running or keeps no box. A stream that asked earlier
    /// hears no more.
    pub fn resume(
        &self,
        position: Position,
    ) -> Option<(u64, tokio::sync::mpsc::Receiver<serde_json::Value>)> {
        if !self.running().is_some_and(|running| {
            running
                .capabilities
                .iter()
                .any(|named| named == launch::CAP_EVENTS)
        }) {
            return None;
        }
        let (events, heard) = tokio::sync::mpsc::channel(EVENTS_WAITING);
        let id = {
            let mut control = lock(&self.control);
            control.listeners += 1;
            let id = control.listeners;
            control.listener = Some(Listener { id, events });
            id
        };
        // The listener is in place first: the node answers with events.
        self.tell(&ToNode::Resume(position));
        Some((id, heard))
    }

    /// The guest's core recorded the node's box through `position`.
    pub fn ack(&self, position: Position) {
        self.tell(&ToNode::Ack(position));
    }

    /// The stream that asked for the box as listener `id` ended: the node
    /// keeps its events until the next one asks. A later listener is left
    /// as it is.
    pub fn away(&self, id: u64) {
        let was = {
            let mut control = lock(&self.control);
            let was = control
                .listener
                .as_ref()
                .is_some_and(|listener| listener.id == id);
            if was {
                control.listener = None;
            }
            was
        };
        if was {
            self.tell(&ToNode::Away);
        }
    }

    /// Hands one event line of the node to the stream that asked for its
    /// box, as a management message. The line is read as its fields, so
    /// one this build does not know goes with it. This waits while the
    /// stream is behind, which leaves the node's output unread and its
    /// next event in its box; a stream that ended is told to the node.
    fn relay(&self, line: &str) {
        let Ok(serde_json::Value::Object(mut said)) = serde_json::from_str(line) else {
            return;
        };
        let Some(serde_json::Value::Object(mut event)) = said.remove("event") else {
            return;
        };
        event.insert("t".to_owned(), "event".into());
        let listener = lock(&self.control)
            .listener
            .as_ref()
            .map(|listener| (listener.id, listener.events.clone()));
        // An event no stream asked for stays in the node's box: the guest
        // names its place again when it connects.
        let Some((id, events)) = listener else {
            return;
        };
        if events
            .blocking_send(serde_json::Value::Object(event))
            .is_err()
        {
            self.away(id);
        }
    }

    /// The daemon is ending: the node hears its host go and exits, and its
    /// holders keep its panes.
    pub fn stop(&self) {
        let mut control = lock(&self.control);
        control.stopping = true;
        control.next = None;
        control.input = None;
        drop(control);
        self.wake.notify_all();
    }

    /// Waits until `deadline` for the supervisor to have reaped the node
    /// after [`GuestNode::stop`]; whether it has.
    pub fn stopped_by(&self, deadline: Instant) -> bool {
        let control = lock(&self.control);
        let wait = deadline.saturating_duration_since(Instant::now());
        let (control, _) = self
            .wake
            .wait_timeout_while(control, wait, |control| control.supervised)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !control.supervised
    }

    /// Waits `wait` unless the daemon ends or another build is asked for
    /// first.
    fn pause(&self, wait: Duration) {
        let control = lock(&self.control);
        drop(
            self.wake
                .wait_timeout_while(control, wait, |control| {
                    !control.stopping && control.next.is_none()
                })
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }

    /// The supervisor's leave: nothing runs and nothing is started again
    /// until the next [`GuestNode::start`], which sees that under the same
    /// lock.
    fn leave(&self, control: &mut Control, phase: Phase) {
        control.supervised = false;
        control.current = None;
        control.next = None;
        self.phase.send_replace(phase);
        self.wake.notify_all();
    }

    fn supervise(self: Arc<Self>) {
        let mut held = 0u32;
        loop {
            let program = {
                let mut control = lock(&self.control);
                if let Some(next) = control.next.take() {
                    control.current = Some(next);
                    held = 0;
                }
                match (control.current.clone(), control.stopping) {
                    (Some(program), false) => program,
                    _ => return self.leave(&mut control, Phase::ended()),
                }
            };
            self.phase.send_replace(Phase::Starting);
            let exit = self.run(&program);
            self.record("guest_node.exited", json!({"exit": format!("{exit:?}")}));
            let mut control = lock(&self.control);
            // Another build was asked for while this one ran, or the daemon
            // ends: this end is the make-way, whatever it says.
            let (phase, again) = if control.next.is_some() || control.stopping {
                (Phase::Starting, Some(Duration::ZERO))
            } else {
                follows(exit, &mut held, &mut control.crashes, Instant::now())
            };
            match again {
                Some(wait) => {
                    self.phase.send_replace(phase);
                    drop(control);
                    if !wait.is_zero() {
                        self.pause(wait);
                    }
                }
                None => return self.leave(&mut control, phase),
            }
        }
    }

    /// The node's command: `program` with the login's variables, this
    /// node's folders, and, when this daemon's panes answer macOS's consent
    /// as the app (`as_app`), the word that the node's do too.
    fn command(&self, program: &Path, as_app: bool) -> Command {
        let mut command = Command::new(program);
        // The node is another build's Hide: it gets the login's variables
        // and none of this daemon's own.
        hide_platform::process::restrict_to_login_environment(&mut command);
        // The login's shell is the login's too: the node's kit asks it where
        // the account keeps its programs, as a device's node does under the
        // `sshd` that names it.
        if let Ok(shell) = hide_platform::host::default_shell() {
            command.env("SHELL", shell);
        }
        if let Some(state) = &self.process_state {
            command.env("HIDE_STATE_DIR", state);
        }
        // No app is above a node this daemon starts when none is above the
        // daemon: its panes answer macOS's consent as this daemon's do.
        if as_app {
            command.env(crate::env::HIDE_PANES_ANSWER_AS_APP, "1");
        }
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .args(launch::ARGUMENTS)
            .arg(&self.node_dir);
        command
    }

    /// One run of the node: started, read until its output ends, reaped.
    fn run(&self, program: &Path) -> Exit {
        let mut command = self.command(
            program,
            hide_node::terminal::holder::answers_consent_as_app(),
        );
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // The node starts the guest's holders, whose programs are the
        // operator's: it starts with the open-file limit this daemon started
        // with, raises its own, and gives its holders that one back.
        hide_platform::process::keep_starting_open_file_limit(&mut command);
        let mut child = match start(&mut command) {
            Ok(child) => child,
            Err(error) => {
                return Exit::Crashed(format!(
                    "{} could not be started: {error}",
                    program.display()
                ));
            }
        };
        {
            // Another build was asked for, or the daemon began to end,
            // since this run was chosen: its input is let go here, which
            // tells this node its host is gone, as the ask told the one
            // before it. Kept, it would run until something else ended it.
            let input = child.take_stdin();
            let mut control = lock(&self.control);
            if control.next.is_none() && !control.stopping {
                control.input = input;
            }
        }
        let (lines, heard) = mpsc::channel::<String>();
        let readers = [
            child.take_stdout().map(|output| {
                std::thread::spawn(move || {
                    let mut output = BufReader::new(output);
                    while let Some(line) = read_line(&mut output) {
                        if lines.send(line).is_err() {
                            break;
                        }
                    }
                })
            }),
            child.take_stderr().map(|errors| {
                let guest = self.guest.short();
                std::thread::spawn(move || {
                    let mut errors = BufReader::new(errors);
                    while let Some(line) = read_line(&mut errors) {
                        // The node's own records, kept under the guest's
                        // name; anything else it wrote is kept as text.
                        let line = serde_json::from_str::<serde_json::Value>(&line)
                            .unwrap_or(serde_json::Value::String(line));
                        hide_core::diagnostic!(json!({
                            "component": "guest_node",
                            "guest": guest,
                            "node": line,
                        }));
                    }
                })
            }),
        ];
        let built = program
            .parent()
            .and_then(|folder| folder.file_name())
            .and_then(|name| name.to_str())
            .filter(|name| build::is_id(name));
        let exit = self.serve(&mut child, &heard, built);
        // Whatever ended the run, the process and what it left in its tree
        // are gone before the next one starts; its holders are not in it.
        // The stream that asked for its box hears no more of it, and asks
        // the node that follows.
        {
            let mut control = lock(&self.control);
            control.input = None;
            control.listener = None;
        }
        let reaped = match &exit {
            Some(_) => {
                let _ = child.kill_tree();
                child.wait()
            }
            None => child.wait(),
        };
        for reader in readers.into_iter().flatten() {
            let _ = reader.join();
        }
        match (exit, reaped) {
            (Some(exit), _) => exit,
            (None, Ok(status)) => exit_of(status.code()),
            (None, Err(error)) => {
                Exit::Crashed(format!("the node's end could not be read: {error}"))
            }
        }
    }

    /// Reads the node's lines until its output ends. `Some` when this host
    /// ends the node itself, with why.
    fn serve(
        &self,
        child: &mut OwnedChild,
        heard: &mpsc::Receiver<String>,
        built: Option<&str>,
    ) -> Option<Exit> {
        let ready = match heard.recv_timeout(Duration::from_secs(launch::READY_WITHIN_SECS)) {
            Ok(line) => line,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Some(Exit::Crashed(format!(
                    "the node wrote no ready line within {} s",
                    launch::READY_WITHIN_SECS
                )));
            }
            // It exited before it was ready: its exit code says why.
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        };
        let running = match serde_json::from_str::<Ready>(&ready) {
            Ok(ready) => match admit(ready, &self.node_dir) {
                Ok(running) => running,
                Err(exit) => return Some(exit),
            },
            Err(error) => {
                return Some(Exit::Crashed(format!(
                    "the node's first line is no ready line: {error}"
                )));
            }
        };
        if running.pid != child.id() {
            self.record(
                "guest_node.pid_differs",
                json!({"said": running.pid, "started": child.id()}),
            );
        }
        self.record(
            "guest_node.ready",
            json!({"build": running.build, "build_id": running.build_id, "pid": child.id()}),
        );
        self.phase.send_replace(Phase::Ready(Running {
            // The build is the folder this host started it from, whatever
            // the node says; a node started from anywhere else is taken at
            // its word.
            build_id: built.map_or(running.build_id.clone(), str::to_owned),
            ..running
        }));
        // While the node says it is idle it is ended after the contract's
        // wait; anything it says first starts the wait again.
        let mut idle = false;
        loop {
            let line = match idle {
                true => match heard.recv_timeout(Duration::from_secs(launch::IDLE_END_SECS)) {
                    Ok(line) => line,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        self.record("guest_node.idle_end", json!({}));
                        self.end(EndMode::Now);
                        idle = false;
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return None,
                },
                false => match heard.recv() {
                    Ok(line) => line,
                    Err(_) => return None,
                },
            };
            match FromNode::parse(&line) {
                Some(FromNode::Idle(now)) => idle = now,
                Some(FromNode::Event(_)) => self.relay(&line),
                // A report the box had no room for. This build's node
                // keeps only what a pane sends, and answers that pane.
                Some(FromNode::BoxFull(_)) => {}
                // A line of a later build.
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A guest's node has no app above it when its daemon has none: it is
    /// told by one variable, and only then. The node is started with the
    /// login's variables, so a daemon that was told never hands the word on
    /// by inheritance.
    #[test]
    fn a_node_is_told_its_panes_answer_as_the_app_only_by_a_daemon_whose_own_do() {
        let node = GuestNode::new(
            PeerKey::from_hex(&"07".repeat(32)).unwrap(),
            PathBuf::from("node"),
            PathBuf::from("home"),
            None,
        );
        let told = |as_app: bool| {
            node.command(Path::new("hided"), as_app)
                .get_envs()
                .find(|(key, _)| *key == crate::env::HIDE_PANES_ANSWER_AS_APP)
                .map(|(_, value)| value.map(|value| value.to_os_string()))
        };
        assert_eq!(told(true), Some(Some("1".into())));
        // Not set, or taken out with the rest of this daemon's variables.
        assert!(
            matches!(told(false), None | Some(None)),
            "{:?}",
            told(false)
        );
    }

    #[test]
    fn a_node_is_left_stopped_after_its_sixth_crash_in_ten_minutes_and_not_for_older_ones() {
        let start = Instant::now();
        let at = |secs: u64| start + Duration::from_secs(secs);
        let mut crashes = VecDeque::new();
        for crash in 0..launch::CRASHES as u64 {
            assert!(!crashed_too_often(&mut crashes, at(crash)), "crash {crash}");
        }
        assert!(crashed_too_often(&mut crashes, at(launch::CRASHES as u64)));

        // The same count spread over more than the window never stops it.
        let mut spread = VecDeque::new();
        for crash in 0..20 {
            let when = at(crash * (launch::CRASH_WINDOW_SECS / launch::CRASHES as u64 + 1));
            assert!(!crashed_too_often(&mut spread, when), "crash {crash}");
        }
    }

    /// D-77 (6): a node that ends to be started again, for a holder it
    /// lost, is started at once with no crash told to its guest; the ends
    /// are counted with the crashes, so one that asks without end stops.
    #[test]
    fn a_node_that_asks_to_be_started_again_is_started_at_once_and_counted_with_the_crashes() {
        let now = Instant::now();
        let (mut held, mut crashes) = (3, VecDeque::new());
        for ask in 0..launch::CRASHES {
            let (phase, again) = follows(Exit::Restart, &mut held, &mut crashes, now);
            assert!(matches!(phase, Phase::Starting), "ask {ask}: {phase:?}");
            assert_eq!(again, Some(Duration::ZERO), "ask {ask}");
        }
        assert_eq!(held, 0);
        let (phase, again) = follows(Exit::Restart, &mut held, &mut crashes, now);
        assert!(
            matches!(
                phase,
                Phase::Stopped {
                    reason: NodeStop::Crashed,
                    retry: false,
                    ..
                }
            ),
            "{phase:?}"
        );
        assert_eq!(again, None);

        // A crash is told with its retry and waits a moment first.
        let (phase, again) = follows(
            Exit::Crashed("boom".to_owned()),
            &mut 0,
            &mut VecDeque::new(),
            now,
        );
        assert!(
            matches!(
                phase,
                Phase::Stopped {
                    reason: NodeStop::Crashed,
                    retry: true,
                    ..
                }
            ),
            "{phase:?}"
        );
        assert_eq!(again, Some(CRASH_RESTART_AFTER));
    }

    #[test]
    fn an_exit_code_reads_as_the_contract_names_it() {
        assert_eq!(exit_of(Some(0)), Exit::Ended);
        assert!(matches!(exit_of(Some(64)), Exit::UpdateNeeded(_)));
        assert!(matches!(exit_of(Some(73)), Exit::FolderRefused(_)));
        assert_eq!(exit_of(Some(75)), Exit::FolderHeld);
        assert_eq!(exit_of(Some(71)), Exit::Restart);
        for crash in [Some(70), Some(1), Some(101), None] {
            assert!(matches!(exit_of(crash), Exit::Crashed(_)), "{crash:?}");
        }
    }

    #[test]
    fn a_ready_line_is_admitted_only_with_endpoints_inside_the_nodes_folder() {
        let ready = |link: &str, guest_node| Ready {
            guest_node,
            build: "1.3.0".to_owned(),
            build_id: "0123456789abcdef".to_owned(),
            pid: 7,
            link: link.to_owned(),
            tap: launch::TAP_ENDPOINT.to_owned(),
            tap_versions: [tap::OLDEST_TAP, tap::NEWEST_TAP],
            capabilities: Vec::new(),
        };
        let dir = Path::new("/s/g/0123456789abcdef/n");
        let running = admit(ready(launch::LINK_ENDPOINT, launch::GUEST_NODE), dir).unwrap();
        assert_eq!(running.link, dir.join(launch::LINK_ENDPOINT));
        for outside in ["../link.sock", "/tmp/link.sock", "a/b", "", ".."] {
            assert!(
                matches!(
                    admit(ready(outside, launch::GUEST_NODE), dir),
                    Err(Exit::Crashed(_))
                ),
                "{outside:?}"
            );
        }
        assert!(matches!(
            admit(ready(launch::LINK_ENDPOINT, launch::GUEST_NODE + 1), dir),
            Err(Exit::UpdateNeeded(_))
        ));
    }

    #[test]
    fn a_line_longer_than_the_contract_allows_ends_the_reading() {
        let mut short = BufReader::new(&b"{\"idle\":true}\r\nrest"[..]);
        assert_eq!(read_line(&mut short).as_deref(), Some("{\"idle\":true}"));
        assert_eq!(
            read_line(&mut short),
            None,
            "a line with no end is not a line"
        );
        let long = vec![b'x'; launch::MAX_LINE + 1];
        assert_eq!(read_line(&mut BufReader::new(&long[..])), None);
    }
}
