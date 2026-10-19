//! One official Herdr terminal session: the `herdr terminal session` child
//! this node starts for a pane, its reader and its writer. Control is
//! writable; observe is concurrent and read-only. Dropping a session tells
//! Herdr it is released and ends only this client process (D-20).

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{SendError, Sender, TryRecvError, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hide_platform::process::OwnedChild;
use serde_json::json;

use super::protocol::{self, Mode, SessionEvent};

/// Opens a pane's session; the node's own Herdr in production, a stand-in
/// in tests.
pub trait Attacher: Send + Sync {
    fn open(&self, pane_id: &str, mode: Mode, rows: u16, cols: u16)
    -> Result<SessionParts, String>;
}

/// The attach children a service started and has not reaped yet, each
/// counted from before it starts until its reaper has seen it exit, so a
/// close can wait for the last one.
#[derive(Default)]
pub(super) struct Children {
    live: Mutex<usize>,
    gone: Condvar,
}

impl Children {
    pub(super) fn hold(self: &Arc<Self>) -> ChildHold {
        *self.lock() += 1;
        ChildHold(Arc::clone(self))
    }

    /// Waits until every child counted has been reaped or `within` passes,
    /// and answers how many were still live.
    pub(super) fn wait_gone(&self, within: Duration) -> usize {
        let deadline = Instant::now() + within;
        let mut live = self.lock();
        while *live > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            live = self
                .gone
                .wait_timeout(live, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        *live
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, usize> {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// One attach child counted live; dropped once it has been reaped.
pub(super) struct ChildHold(Arc<Children>);

impl Drop for ChildHold {
    fn drop(&mut self) {
        let mut live = self.0.lock();
        *live = live.saturating_sub(1);
        if *live == 0 {
            self.0.gone.notify_all();
        }
    }
}

/// What an open session hands the node: its output, its input in control
/// mode, and what ends it.
pub struct SessionParts {
    pub reader: Box<dyn Read + Send>,
    pub writer: Option<Box<dyn Write + Send>>,
    pub cleanup: Cleanup,
}

pub enum Cleanup {
    Child(OwnedChild),
    Other(Box<dyn FnOnce() + Send>),
    None,
}

/// The node's Herdr, reached by the CLI `herdr terminal session` client.
pub struct LocalAttacher {
    herdr_bin: Option<PathBuf>,
    socket_path: PathBuf,
}

impl LocalAttacher {
    pub fn new(herdr_bin: Option<PathBuf>, socket_path: PathBuf) -> Self {
        Self {
            herdr_bin,
            socket_path,
        }
    }
}

impl Attacher for LocalAttacher {
    fn open(
        &self,
        pane_id: &str,
        mode: Mode,
        rows: u16,
        cols: u16,
    ) -> Result<SessionParts, String> {
        let Some(herdr_bin) = self.herdr_bin.as_ref() else {
            return Err(
                "herdr binary was not found; install herdr or set its path in the app options"
                    .to_owned(),
            );
        };
        let mut command = Command::new(herdr_bin);
        // Both modes get a stdin pipe: an observer never writes to it, but
        // its end closing when this process dies, however it dies, is what
        // ends the child (B20).
        command
            .args(protocol::session_arguments(mode, pane_id, rows, cols))
            .env("HERDR_SOCKET_PATH", &self.socket_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = OwnedChild::spawn(&mut command).map_err(|error| {
            format!(
                "herdr terminal session {} could not be spawned: {error}",
                mode.as_str()
            )
        })?;
        let stdin = child
            .take_stdin()
            .ok_or_else(|| "terminal session stdin was not piped".to_owned())?;
        let reader = child
            .take_stdout()
            .ok_or_else(|| "terminal session stdout was not piped".to_owned())?;
        let writer: Option<Box<dyn Write + Send>> = match mode {
            Mode::Control => Some(Box::new(stdin)),
            // Kept open, never written, for the child's whole life.
            Mode::Observe => Some(Box::new(ObserverStdin { _stdin: stdin })),
        };
        Ok(SessionParts {
            reader: Box::new(reader),
            writer,
            cleanup: Cleanup::Child(child),
        })
    }
}

/// An observer's stdin: held so its end closes with this process, refused
/// for any write.
struct ObserverStdin {
    _stdin: std::process::ChildStdin,
}

impl Write for ObserverStdin {
    fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("an observer takes no input"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Input a control session's writer holds unwritten at most: the cap D-18
/// sets on one pane's unsent keys, which a device's link keeps too. A write
/// into an empty queue is always taken, so no single paste is refused for
/// its size here.
pub(super) const MAX_UNWRITTEN_INPUT_BYTES: usize = hide_node_link::terminal::MAX_UNSENT_KEY_BYTES;
/// What a wheel or a resize counts against that cap.
const CONTROL_LINE_COST: usize = 64;

/// Told whether a write reached the session's pipe, once: by the writer
/// after the write, or as not written when the writer ends first.
pub(super) struct Written(Option<Box<dyn FnOnce(bool) + Send>>);

impl Written {
    pub(super) fn new(done: impl FnOnce(bool) + Send + 'static) -> Self {
        Self(Some(Box::new(done)))
    }

    fn finish(mut self, written: bool) {
        if let Some(done) = self.0.take() {
            done(written);
        }
    }

    /// The write never reached the writer; its sender says so itself.
    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for Written {
    fn drop(&mut self) {
        if let Some(done) = self.0.take() {
            done(false);
        }
    }
}

pub(super) enum WriterCommand {
    Scroll {
        lines: i32,
        column: Option<u16>,
        row: Option<u16>,
        modifiers: u8,
    },
    Line {
        line: String,
        /// What it counts against [`MAX_UNWRITTEN_INPUT_BYTES`].
        cost: usize,
        written: Option<Written>,
    },
    Release {
        acknowledged: Sender<()>,
    },
}

impl WriterCommand {
    fn line(line: String, cost: usize) -> Self {
        Self::Line {
            line,
            cost,
            written: None,
        }
    }

    fn cost(&self) -> usize {
        match self {
            Self::Scroll { .. } => CONTROL_LINE_COST,
            Self::Line { cost, .. } => *cost,
            Self::Release { .. } => 0,
        }
    }

    /// The command never reached the writer. Its sender hears the refusal
    /// itself, so a paste's [`Written`] is told nothing: it would run under
    /// the service's lock, which its own callback takes.
    fn refused(self, refusal: WriteRefused) -> Result<(), WriteRefused> {
        if let Self::Line {
            written: Some(written),
            ..
        } = self
        {
            written.disarm();
        }
        Err(refusal)
    }
}

/// Why a session took no input.
#[derive(Debug)]
pub(super) enum WriteRefused {
    /// The writer holds its cap of input its pipe has not taken: the
    /// session's other end stopped reading. Everything sent until the
    /// writer drains is refused too, so the pane never gets the tail of a
    /// refused paste.
    Full,
    Failed(String),
}

impl WriteRefused {
    pub(super) fn message(&self, pane: &str) -> String {
        match self {
            Self::Full => format!(
                "Pane {pane} is not taking input; {} KiB already wait unwritten, so this input was not sent",
                MAX_UNWRITTEN_INPUT_BYTES / 1024
            ),
            Self::Failed(message) => message.clone(),
        }
    }
}

/// The writer's backlog, shared by the session that adds to it and the
/// writer thread that drains it.
#[derive(Default)]
struct Unwritten {
    bytes: AtomicUsize,
    /// The cap was crossed and the backlog has not drained since.
    overflowed: AtomicBool,
}

/// A session the node holds for a pane.
pub(super) struct Session {
    pub(super) generation: u64,
    pub(super) mode: Mode,
    writer: Option<Sender<WriterCommand>>,
    unwritten: Arc<Unwritten>,
    /// An observer's kept stdin, closed with the session.
    _observer_stdin: Option<Box<dyn Write + Send>>,
    cleanup: Cleanup,
    /// Its child counted live until the reaper has seen it exit.
    child: Option<ChildHold>,
    pane_id: String,
}

impl Session {
    /// Starts the writer for `parts` and returns the session and its reader,
    /// which the caller starts once the session is installed. A writer that
    /// fails says so through `on_write_failure`.
    pub(super) fn start(
        pane_id: &str,
        generation: u64,
        mode: Mode,
        parts: SessionParts,
        on_write_failure: Box<dyn Fn(String) + Send>,
    ) -> Result<(Self, Box<dyn Read + Send>), String> {
        let SessionParts {
            reader,
            writer,
            cleanup,
        } = parts;
        let unwritten = Arc::new(Unwritten::default());
        let (writer, observer_stdin) = match (mode, writer) {
            (Mode::Control, Some(stdin)) => (
                Some(spawn_writer(
                    pane_id,
                    stdin,
                    Arc::clone(&unwritten),
                    on_write_failure,
                )?),
                None,
            ),
            (Mode::Control, None) => {
                run_cleanup(cleanup, pane_id);
                return Err("terminal control stream has no writer".to_owned());
            }
            (Mode::Observe, stdin) => (None, stdin),
        };
        Ok((
            Self {
                generation,
                mode,
                writer,
                unwritten,
                _observer_stdin: observer_stdin,
                cleanup,
                child: None,
                pane_id: pane_id.to_owned(),
            },
            reader,
        ))
    }

    /// Counts this session's child live until it has been reaped.
    pub(super) fn counted(mut self, child: ChildHold) -> Self {
        self.child = Some(child);
        self
    }

    /// Hands `command` to the writer within its cap. Only the service, under
    /// its lock, sends, so the check and the count cannot interleave.
    fn send(&self, command: WriterCommand) -> Result<(), WriteRefused> {
        let Some(writer) = self.writer.as_ref() else {
            return command.refused(WriteRefused::Failed(format!(
                "Pane {} is read-only because another client owns terminal control",
                self.pane_id
            )));
        };
        let cost = command.cost();
        let waiting = self.unwritten.bytes.load(Ordering::Acquire);
        if waiting == 0 {
            self.unwritten.overflowed.store(false, Ordering::Release);
        }
        if self.unwritten.overflowed.load(Ordering::Acquire)
            || (waiting > 0 && waiting + cost > MAX_UNWRITTEN_INPUT_BYTES)
        {
            self.unwritten.overflowed.store(true, Ordering::Release);
            return command.refused(WriteRefused::Full);
        }
        self.unwritten.bytes.fetch_add(cost, Ordering::AcqRel);
        writer.send(command).or_else(|SendError(command)| {
            self.unwritten.bytes.fetch_sub(cost, Ordering::AcqRel);
            command.refused(WriteRefused::Failed(
                "terminal control input channel is closed".to_owned(),
            ))
        })
    }

    pub(super) fn write(&self, bytes: &[u8]) -> Result<(), WriteRefused> {
        self.send(WriterCommand::line(
            protocol::input_line(bytes),
            bytes.len(),
        ))
    }

    /// [`Session::write`], telling `written` once the bytes reached the
    /// session's pipe or could not; an `Err` answer tells it nothing.
    pub(super) fn write_then(&self, bytes: &[u8], written: Written) -> Result<(), WriteRefused> {
        self.send(WriterCommand::Line {
            line: protocol::input_line(bytes),
            cost: bytes.len(),
            written: Some(written),
        })
    }

    /// A side-channel write may start after earlier controller writes have
    /// reached the pipe. This barrier emits no terminal protocol or input.
    pub(super) fn after_writes(&self, written: Written) -> Result<(), WriteRefused> {
        self.send(WriterCommand::Line {
            line: String::new(),
            cost: 0,
            written: Some(written),
        })
    }

    pub(super) fn resize(&self, rows: u16, cols: u16) -> Result<(), WriteRefused> {
        let line = protocol::resize_line(rows, cols).map_err(WriteRefused::Failed)?;
        self.send(WriterCommand::line(line, CONTROL_LINE_COST))
    }

    pub(super) fn scroll(
        &self,
        lines: i32,
        column: Option<u16>,
        row: Option<u16>,
        modifiers: u8,
    ) -> Result<(), WriteRefused> {
        self.send(WriterCommand::Scroll {
            lines,
            column,
            row,
            modifiers,
        })
    }
}

fn spawn_writer(
    pane_id: &str,
    mut stdin: Box<dyn Write + Send>,
    unwritten: Arc<Unwritten>,
    on_failure: Box<dyn Fn(String) + Send>,
) -> Result<Sender<WriterCommand>, String> {
    let (sender, receiver) = channel::<WriterCommand>();
    thread::Builder::new()
        .name(format!("hide-node-terminal-writer-{pane_id}"))
        .spawn(move || {
            let mut carried = None;
            loop {
                let command = match carried.take() {
                    Some(command) => command,
                    None => match receiver.recv() {
                        Ok(command) => command,
                        Err(_) => return,
                    },
                };
                // What this turn writes leaves the backlog once it is written
                // or has failed.
                let mut cost = command.cost();
                let (line, acknowledgement, written) = match command {
                    WriterCommand::Scroll {
                        mut lines,
                        mut column,
                        mut row,
                        mut modifiers,
                    } => {
                        // The screen has already turned precise trackpad
                        // movement into whole rows. Only wheels waiting in
                        // the channel right now are combined; nothing here
                        // waits for a frame or a timer.
                        let mut disconnected = false;
                        loop {
                            match receiver.try_recv() {
                                Ok(WriterCommand::Scroll {
                                    lines: next,
                                    column: next_column,
                                    row: next_row,
                                    modifiers: next_modifiers,
                                }) => {
                                    cost += CONTROL_LINE_COST;
                                    lines = lines.saturating_add(next);
                                    column = next_column;
                                    row = next_row;
                                    modifiers = next_modifiers;
                                }
                                Ok(command) => {
                                    carried = Some(command);
                                    break;
                                }
                                Err(TryRecvError::Empty) => break,
                                Err(TryRecvError::Disconnected) => {
                                    disconnected = true;
                                    break;
                                }
                            }
                        }
                        match protocol::scroll_line(lines, column, row, modifiers) {
                            Some(line) => (line, None, None),
                            None => {
                                unwritten.bytes.fetch_sub(cost, Ordering::AcqRel);
                                if disconnected {
                                    return;
                                }
                                continue;
                            }
                        }
                    }
                    WriterCommand::Line { line, written, .. } => (line, None, written),
                    WriterCommand::Release { acknowledged } => {
                        (protocol::release_line(), Some(acknowledged), None)
                    }
                };
                let result = stdin
                    .write_all(line.as_bytes())
                    .and_then(|()| stdin.flush());
                unwritten.bytes.fetch_sub(cost, Ordering::AcqRel);
                if let Some(written) = written {
                    written.finish(result.is_ok());
                }
                if let Some(acknowledgement) = acknowledgement {
                    // A release is the session letting go; a child that
                    // already left has nothing to be told and no failure
                    // to report.
                    let _ = acknowledgement.send(());
                    return;
                }
                if let Err(error) = result {
                    on_failure(format!("terminal control write failed: {error}"));
                    return;
                }
            }
        })
        .map_err(|error| format!("terminal control writer could not be started: {error}"))?;
    Ok(sender)
}

/// Reads `reader` until the session ends, handing each event to `on_event`,
/// which answers whether to keep reading. The stream's end, a malformed line
/// and a read failure are each the session closing, with the reason.
pub(super) fn spawn_reader(
    pane_id: &str,
    mode: Mode,
    reader: Box<dyn Read + Send>,
    mut on_event: impl FnMut(SessionEvent) -> bool + Send + 'static,
) -> Result<(), String> {
    thread::Builder::new()
        .name(format!("hide-node-terminal-{}-{pane_id}", mode.as_str()))
        .spawn(move || {
            let mut lines = BufReader::new(reader).lines();
            loop {
                let event = match lines.next() {
                    None => SessionEvent::Closed { reason: None },
                    Some(Ok(line)) => match protocol::parse_line(&line) {
                        Ok(event) => event,
                        Err(message) => SessionEvent::Closed {
                            reason: Some(message),
                        },
                    },
                    Some(Err(error)) => SessionEvent::Closed {
                        reason: Some(format!("terminal session stream failed: {error}")),
                    },
                };
                let closed = matches!(event, SessionEvent::Closed { .. });
                if !on_event(event) || closed {
                    return;
                }
            }
        })
        .map(|_| ())
        .map_err(|error| format!("terminal session reader could not be started: {error}"))
}

impl Drop for Session {
    fn drop(&mut self) {
        let acknowledgement = self.writer.take().and_then(|writer| {
            let (acknowledged, acknowledgement) = channel();
            writer
                .send(WriterCommand::Release { acknowledged })
                .ok()
                .map(|()| acknowledgement)
        });
        let cleanup = std::mem::replace(&mut self.cleanup, Cleanup::None);
        // Dropped here, or after the reaper's cleanup: a child is counted
        // gone only once it has exited.
        let child = self.child.take();
        if matches!(cleanup, Cleanup::None) {
            return;
        }
        let pane_id = self.pane_id.clone();
        // Fields drop in order, so a reaper that cannot start still ends the
        // child before it is counted gone.
        let reaping = Reaping { cleanup, child };
        if let Err(error) = thread::Builder::new()
            .name(format!("hide-node-terminal-reaper-{pane_id}"))
            .spawn(move || {
                if let Some(acknowledgement) = acknowledgement
                    && acknowledgement
                        .recv_timeout(Duration::from_secs(1))
                        .is_err()
                {
                    crate::diagnostic!(json!({
                        "component": "terminal_session",
                        "kind": "terminal.release_unacknowledged",
                        "pane_id": pane_id,
                    }));
                }
                let Reaping { cleanup, child } = reaping;
                run_cleanup(cleanup, &pane_id);
                drop(child);
            })
        {
            crate::diagnostic!(json!({
                "component": "terminal_session",
                "kind": "terminal.session_reaper_spawn_failed",
                "message": error.to_string(),
            }));
        }
    }
}

/// What a session's reaper ends, in the order it ends it.
struct Reaping {
    cleanup: Cleanup,
    child: Option<ChildHold>,
}

fn run_cleanup(cleanup: Cleanup, pane_id: &str) {
    match cleanup {
        Cleanup::Child(mut child) => reap_child(&mut child, pane_id),
        Cleanup::Other(shutdown) => shutdown(),
        Cleanup::None => {}
    }
}

#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn reap_child(child: &mut OwnedChild, pane_id: &str) {
    for _ in 0..20 {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                crate::diagnostic!(json!({
                    "component": "terminal_session",
                    "kind": "terminal.session_status_failed",
                    "pane_id": pane_id,
                    "message": error.to_string(),
                }));
                break;
            }
        }
    }
    if let Err(error) = child.kill_tree() {
        crate::diagnostic!(json!({
            "component": "terminal_session",
            "kind": "terminal.session_kill_failed",
            "pane_id": pane_id,
            "message": error.to_string(),
        }));
    }
    if let Err(error) = child.wait() {
        crate::diagnostic!(json!({
            "component": "terminal_session",
            "kind": "terminal.session_wait_failed",
            "pane_id": pane_id,
            "message": error.to_string(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::mpsc::Receiver;

    /// A pipe end that hands each flushed batch of lines to the test.
    struct Sink {
        pending: Vec<u8>,
        flushed: Sender<Vec<serde_json::Value>>,
    }

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.pending.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            let lines = std::mem::take(&mut self.pending);
            let lines = String::from_utf8(lines)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            let _ = self.flushed.send(lines);
            Ok(())
        }
    }

    fn writer() -> (Sender<WriterCommand>, Receiver<Vec<serde_json::Value>>) {
        let (flushed, received) = channel();
        let writer = spawn_writer(
            "fixture:p1",
            Box::new(Sink {
                pending: Vec::new(),
                flushed,
            }),
            Arc::default(),
            Box::new(|_| {}),
        )
        .unwrap();
        (writer, received)
    }

    fn wheel(lines: i32, column: u16) -> WriterCommand {
        WriterCommand::Scroll {
            lines,
            column: Some(column),
            row: Some(12),
            modifiers: 2,
        }
    }

    #[test]
    fn first_wheel_reaches_the_pipe_without_waiting_for_a_frame() {
        let (writer, received) = writer();
        writer.send(wheel(3, 24)).unwrap();
        let lines = received.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(lines[0]["type"], "terminal.scroll");
        assert_eq!(lines[0]["lines"], 3);
    }

    #[test]
    fn next_wheel_reaches_the_pipe_without_waiting_for_a_terminal_frame() {
        let (writer, received) = writer();
        writer.send(wheel(3, 24)).unwrap();
        received.recv_timeout(Duration::from_secs(1)).unwrap();
        writer.send(wheel(2, 25)).unwrap();
        let lines = received
            .recv_timeout(Duration::from_secs(1))
            .expect("a later wheel must not wait for an unrelated terminal frame");
        assert_eq!(lines[0]["lines"], 2);
        assert_eq!(lines[0]["column"], 25);
    }

    #[test]
    fn keyboard_input_keeps_its_order_after_scroll_input() {
        let (writer, received) = writer();
        writer.send(wheel(3, 24)).unwrap();
        received.recv_timeout(Duration::from_secs(1)).unwrap();
        writer.send(wheel(2, 25)).unwrap();
        writer
            .send(WriterCommand::line(protocol::input_line(b"x"), 1))
            .unwrap();
        let scroll = received.recv_timeout(Duration::from_secs(1)).unwrap();
        let input = received.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(scroll[0]["type"], "terminal.scroll");
        assert_eq!(input[0]["type"], "terminal.input");
    }

    /// A pipe end whose reader stopped: every write waits until the test
    /// lets the reader go on.
    struct Stalled {
        go: Arc<(Mutex<bool>, std::sync::Condvar)>,
        taken: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for Stalled {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let (going, resumed) = &*self.go;
            let mut going = going.lock().unwrap();
            while !*going {
                going = resumed.wait(going).unwrap();
            }
            self.taken.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Principle 15 on the local path: a session whose reader stopped holds
    /// at most its cap of input, refuses the rest until it drains, and tells
    /// a paste it was written only once the pipe took it.
    #[test]
    fn a_stalled_session_holds_at_most_its_cap_and_says_written_only_after_the_write() {
        let go = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let taken = Arc::new(Mutex::new(Vec::new()));
        let (session, _reader) = Session::start(
            "fixture:p1",
            1,
            Mode::Control,
            SessionParts {
                reader: Box::new(std::io::empty()),
                writer: Some(Box::new(Stalled {
                    go: Arc::clone(&go),
                    taken: Arc::clone(&taken),
                })),
                cleanup: Cleanup::None,
            },
            Box::new(|_| {}),
        )
        .unwrap();
        let (told, written) = channel();
        session
            .write_then(b"paste", Written::new(move |ok| told.send(ok).unwrap()))
            .unwrap();
        // The pipe has not taken the paste, so nobody hears it was written.
        assert!(written.recv_timeout(Duration::from_millis(100)).is_err());
        let key = [b'k'; 1024];
        let mut accepted = 0;
        let refused = loop {
            match session.write(&key) {
                Ok(()) => accepted += 1,
                Err(refused) => break refused,
            }
            assert!(
                accepted <= MAX_UNWRITTEN_INPUT_BYTES / key.len(),
                "never refused"
            );
        };
        assert!(matches!(refused, WriteRefused::Full));
        assert!(session.unwritten.bytes.load(Ordering::Acquire) <= MAX_UNWRITTEN_INPUT_BYTES);
        // Refused until the backlog drains, however small the input.
        assert!(matches!(session.write(b"x"), Err(WriteRefused::Full)));
        *go.0.lock().unwrap() = true;
        go.1.notify_all();
        assert!(written.recv_timeout(Duration::from_secs(5)).unwrap());
        let started = std::time::Instant::now();
        while session.unwritten.bytes.load(Ordering::Acquire) > 0 {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the backlog never drained"
            );
            thread::yield_now();
        }
        session.write(b"after").unwrap();
        drop(session);
    }

    /// A refused paste tells its caller, never its callback: the callback
    /// takes the service's lock, which the caller holds while it writes. The
    /// write runs on its own thread so a deadlock fails the test instead of
    /// hanging it.
    fn refused_without_its_callback(session: Session) {
        let held = Arc::new(Mutex::new(()));
        let (called, calls) = channel();
        let (answered, answer) = channel();
        let lock = Arc::clone(&held);
        thread::spawn(move || {
            let _service = held.lock().unwrap();
            let refused = session
                .write_then(
                    b"'/tmp/a.png' ",
                    Written::new(move |written| {
                        let _service = lock.lock().unwrap();
                        let _ = called.send(written);
                    }),
                )
                .is_err();
            let _ = answered.send(refused);
            session
        });
        assert!(
            answer
                .recv_timeout(Duration::from_secs(5))
                .expect("the refused paste deadlocked on its own callback"),
            "the paste was refused"
        );
        assert!(calls.try_recv().is_err(), "the callback heard a refusal");
    }

    #[test]
    fn a_paste_refused_by_a_full_backlog_does_not_call_back_under_the_lock() {
        let go = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let (session, _reader) = Session::start(
            "fixture:p1",
            1,
            Mode::Control,
            SessionParts {
                reader: Box::new(std::io::empty()),
                writer: Some(Box::new(Stalled {
                    go: Arc::clone(&go),
                    taken: Arc::default(),
                })),
                cleanup: Cleanup::None,
            },
            Box::new(|_| {}),
        )
        .unwrap();
        session.write(b"x").unwrap();
        assert!(matches!(
            session.write(&vec![b'k'; MAX_UNWRITTEN_INPUT_BYTES]),
            Err(WriteRefused::Full)
        ));
        refused_without_its_callback(session);
        *go.0.lock().unwrap() = true;
        go.1.notify_all();
    }

    #[test]
    fn a_paste_into_a_read_only_session_does_not_call_back_under_the_lock() {
        let (session, _reader) = Session::start(
            "fixture:p1",
            1,
            Mode::Observe,
            SessionParts {
                reader: Box::new(std::io::empty()),
                writer: None,
                cleanup: Cleanup::None,
            },
            Box::new(|_| {}),
        )
        .unwrap();
        refused_without_its_callback(session);
    }

    #[test]
    fn a_failed_write_is_reported_once() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("gone"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let (failed, failures) = channel();
        let failed = Mutex::new(failed);
        let writer = spawn_writer(
            "fixture:p1",
            Box::new(Broken),
            Arc::default(),
            Box::new(move |message| {
                let _ = failed.lock().unwrap().send(message);
            }),
        )
        .unwrap();
        writer
            .send(WriterCommand::line(protocol::input_line(b"x"), 1))
            .unwrap();
        let _ = writer.send(WriterCommand::line(protocol::input_line(b"y"), 1));
        failures
            .recv_timeout(Duration::from_secs(1))
            .expect("the failure is reported");
        // The writer ended with the first failure, closing its end.
        assert!(failures.recv_timeout(Duration::from_secs(1)).is_err());
    }
}
