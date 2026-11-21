//! One pane's terminal session as the service holds it: the transport it
//! was opened over, its reader and its writer. A pane's holder
//! (`super::holder`) is the transport; its frames are what its reader and
//! writer speak. Dropping a session releases it and ends only what its
//! transport started (D-20).

use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{SendError, Sender, TryRecvError, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hide_holder_protocol::Key;
use hide_node_link::terminal::GridSize;
use serde_json::json;

use super::echo_timing::EchoClock;
use super::protocol::SessionEvent;

/// Opens a pane's session: a pane's holder in production
/// (`super::holder::PaneAttacher`), a stand-in in tests.
pub trait Attacher: Send + Sync {
    /// Opens the pane's session over the transport it is held by. `size` is
    /// `None` only for a pane [`Attacher::keeps_own_size`] says keeps its
    /// own, which is attached at the size it has.
    fn open_transport(
        &self,
        pane_id: &str,
        size: Option<GridSize>,
    ) -> Result<Transport, OpenFailure>;

    /// Whether the pane keeps its own size and attaches at it with no size
    /// from a view.
    fn keeps_own_size(&self, _pane_id: &str) -> bool {
        false
    }

    /// Whether a holder this node started or adopted runs the pane, or is
    /// being started for it. Unlike [`Attacher::keeps_own_size`], never true
    /// of a pane only because the holders are the node's one engine.
    fn holds(&self, _pane_id: &str) -> bool {
        false
    }
}

/// Why a session could not be opened, and whether trying again can help.
#[derive(Debug)]
pub struct OpenFailure {
    /// The `exit_category` the pane's state reports.
    pub category: &'static str,
    pub message: String,
    /// Whether the service tries again on its own; a pane that is over, or
    /// held by a holder this build cannot speak to, is not tried again.
    pub retried: bool,
}

impl OpenFailure {
    pub fn retried(message: String) -> Self {
        Self {
            category: "spawn_failed",
            message,
            retried: true,
        }
    }
}

/// Reads a session's events, one at a time, until it closes. The stream's
/// end and a read failure are the session closing, with the reason.
pub trait SessionEvents: Send {
    fn next_event(&mut self) -> SessionEvent;
}

/// Writes a session's input in its transport's form. Each call writes and
/// flushes what it was given, so it has reached the transport when it
/// returns.
pub trait SessionWriter: Send {
    fn input(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Pasted text, which the transport wraps for the pane's paste mode.
    fn paste(&mut self, text: &[u8]) -> io::Result<()>;
    /// Named keys, which the transport encodes for the pane's keyboard
    /// modes; only a session whose [`Transport::reads_keys`] is sent them.
    fn keys(&mut self, keys: &[Key]) -> io::Result<()>;
    fn resize(&mut self, size: GridSize) -> io::Result<()>;
    /// Asks for the whole screen again at `size`, the size the session
    /// runs at: the next frame is full.
    fn redraw(&mut self, size: GridSize) -> io::Result<()>;
    /// A wheel of `lines` rows, positive up, never zero, over the zero-based
    /// cell when the screen said which.
    fn scroll(
        &mut self,
        lines: i32,
        column: Option<u16>,
        row: Option<u16>,
        modifiers: u8,
    ) -> io::Result<()>;
    /// A pointer event over a zero-based cell, which the transport reports
    /// in the program's mouse mode or drops when it tracks none.
    fn mouse(&mut self, mouse: &hide_holder_protocol::Mouse) -> io::Result<()>;
    /// Lets go of the session; the pane runs on.
    fn release(&mut self) -> io::Result<()>;
    /// Answers once everything written before has reached the transport.
    fn flush(&mut self) -> io::Result<()>;
    /// One submission, which the
    /// transport keeps whole at the program's terminal; only a session whose
    /// [`Transport::submissions`] is [`Submissions::Whole`] is sent one.
    fn submit(&mut self, submit: &hide_holder_protocol::Submit) -> io::Result<()>;
}

/// How a transport takes one submission ([`SessionWriter::submit`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Submissions {
    /// Whole, its gap kept at the program's terminal: a holder that names
    /// `Capability::Submit`.
    Whole,
    /// Refused: a holder built before `Capability::Submit`. A submission for
    /// it is refused with nothing sent, and a fork's released keys are
    /// dropped and reported, since typed together a submit among them can
    /// land inside the program's paste.
    Refused,
}

/// An opened session as the service holds it.
pub struct Transport {
    pub events: Box<dyn SessionEvents>,
    pub writer: Box<dyn SessionWriter>,
    pub cleanup: Cleanup,
    /// The pane's size as the transport found it, when it says.
    pub size: Option<GridSize>,
    /// Whether the writer encodes named keys ([`SessionWriter::keys`]): a
    /// holder that names `Capability::Keys`.
    pub reads_keys: bool,
    pub submissions: Submissions,
}

impl Transport {
    /// Ends an open that lost its generation, before another open may begin.
    pub(super) fn close(self) {
        let Self {
            events,
            writer,
            cleanup,
            ..
        } = self;
        run_cleanup(cleanup);
        drop((events, writer));
    }
}

/// The attach children a service started and has not reaped yet, each
/// counted from before it starts until its reaper has seen it exit, so a
/// close can wait for the last one.
#[derive(Default)]
pub(super) struct Children {
    live: Mutex<HashSet<String>>,
    gone: Condvar,
}

impl Children {
    pub(super) fn hold(
        self: &Arc<Self>,
        pane: &str,
        ended: Box<dyn FnOnce() + Send>,
    ) -> Option<ChildHold> {
        let mut live = self.lock();
        if live.contains(pane) || live.len() >= hide_node_link::terminal::MAX_ATTACHED_PANES {
            return None;
        }
        live.insert(pane.to_owned());
        Some(ChildHold {
            owner: Arc::clone(self),
            pane: pane.to_owned(),
            ended: Some(ended),
        })
    }

    pub(super) fn contains(&self, pane: &str) -> bool {
        self.lock().contains(pane)
    }

    pub(super) fn occupied_with<'a>(&self, pending: impl Iterator<Item = &'a str>) -> usize {
        let live = self.lock();
        live.len() + pending.filter(|pane| !live.contains(*pane)).count()
    }

    /// Waits until every child counted has been reaped or `within` passes,
    /// and answers how many were still live.
    pub(super) fn wait_gone(&self, within: Duration) -> usize {
        let deadline = Instant::now() + within;
        let mut live = self.lock();
        while !live.is_empty() {
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
        live.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// One attach child counted live; dropped once it has been reaped.
pub(super) struct ChildHold {
    owner: Arc<Children>,
    pane: String,
    ended: Option<Box<dyn FnOnce() + Send>>,
}

impl Drop for ChildHold {
    fn drop(&mut self) {
        self.owner.lock().remove(&self.pane);
        self.owner.gone.notify_all();
        if let Some(ended) = self.ended.take() {
            ended();
        }
    }
}

/// What ends a session's transport once it is released.
pub enum Cleanup {
    Other(Box<dyn FnOnce() + Send>),
    None,
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
    Mouse(hide_holder_protocol::Mouse),
    /// Bytes for the pane, counted against [`MAX_UNWRITTEN_INPUT_BYTES`] by
    /// their length.
    Input {
        bytes: Vec<u8>,
        written: Option<Written>,
    },
    /// Pasted text, counted by its length.
    Paste(Vec<u8>),
    /// Named keys, each counted as one byte.
    Keys(Vec<Key>),
    /// One submission, counted by its text and submit.
    Submit(Box<hide_holder_protocol::Submit>),
    Engine {
        input: super::input::Typed,
        command: super::command::Command,
    },
    Resize(GridSize),
    Redraw(GridSize),
    /// Writes nothing; tells `written` once what was sent before it was
    /// written.
    Barrier(Written),
    Release {
        acknowledged: Sender<()>,
    },
}

impl WriterCommand {
    fn input(bytes: &[u8]) -> Self {
        Self::Input {
            bytes: bytes.to_vec(),
            written: None,
        }
    }

    fn cost(&self) -> usize {
        match self {
            Self::Scroll { .. } | Self::Mouse(_) | Self::Resize(_) | Self::Redraw(_) => {
                CONTROL_LINE_COST
            }
            Self::Input { bytes, .. } | Self::Paste(bytes) => bytes.len(),
            Self::Keys(keys) => keys.len(),
            Self::Submit(submit) => super::input::submit_len(submit),
            Self::Engine { input, .. } => input.len(),
            Self::Barrier(_) | Self::Release { .. } => 0,
        }
    }

    /// The command never reached the writer. Its sender hears the refusal
    /// itself, so a paste's [`Written`] is told nothing: it would run under
    /// the service's lock, which its own callback takes.
    fn refused(self, refusal: WriteRefused) -> Result<(), WriteRefused> {
        match self {
            Self::Input {
                written: Some(written),
                ..
            }
            | Self::Barrier(written) => written.disarm(),
            Self::Engine { command, .. } => command.refused(match &refusal {
                WriteRefused::Full => "Pane input exceeded the writer backlog limit",
                WriteRefused::Failed(message) | WriteRefused::Unsupported(message) => message,
            }),
            _ => {}
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
    /// The transport does not take what was sent, which is not sent.
    Unsupported(String),
}

impl WriteRefused {
    pub(super) fn message(&self, pane: &str) -> String {
        match self {
            Self::Full => format!(
                "Pane {pane} is not taking input; {} KiB already wait unwritten, so this input was not sent",
                MAX_UNWRITTEN_INPUT_BYTES / 1024
            ),
            Self::Failed(message) | Self::Unsupported(message) => message.clone(),
        }
    }
}

/// How a session's writer thread ended, or that it still runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WriterEnd {
    Running,
    /// Everything handed to it was written, or it was let go: a release, or
    /// its session dropped.
    Settled,
    /// A write failed, so what reached the pane is not known.
    Failed,
}

#[derive(Debug)]
struct WriterState {
    end: Mutex<WriterEnd>,
    moved: Condvar,
}

/// A session's writer as its pane's node remembers it after the session is
/// gone: a released or replaced session's writer keeps writing what was
/// handed to it before its release (with-hide/hide-private#646).
#[derive(Clone, Debug)]
pub(super) struct WriterLife(Arc<WriterState>);

impl WriterLife {
    pub(super) fn new() -> Self {
        Self(Arc::new(WriterState {
            end: Mutex::new(WriterEnd::Running),
            moved: Condvar::new(),
        }))
    }

    /// How its writer ended, or that it still runs, read once: a caller
    /// decides from this one value, never from two reads a writer's end can
    /// fall between.
    pub(super) fn end(&self) -> WriterEnd {
        *self.0.end.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether `self` and `other` are the same session's writer.
    pub(super) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Waits until its writer has ended, up to `deadline`: whether it ended
    /// having written everything handed to it.
    pub(super) fn settled_by(&self, deadline: Instant) -> Result<(), &'static str> {
        let mut end = self.0.end.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            match *end {
                WriterEnd::Settled => return Ok(()),
                WriterEnd::Failed => return Err("failed"),
                WriterEnd::Running => {}
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Err("still writing");
            };
            end = self
                .0
                .moved
                .wait_timeout(end, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

#[cfg(test)]
impl WriterLife {
    /// Ends it as its writer thread would, `failed` or having written
    /// everything.
    pub(super) fn end_as(&self, failed: bool) {
        drop(Ends {
            life: self.clone(),
            failed,
        });
    }
}

/// Marks its writer ended when the writer thread leaves, on every way out.
struct Ends {
    life: WriterLife,
    failed: bool,
}

impl Drop for Ends {
    fn drop(&mut self) {
        *self.life.0.end.lock().unwrap_or_else(|e| e.into_inner()) = if self.failed {
            WriterEnd::Failed
        } else {
            WriterEnd::Settled
        };
        self.life.0.moved.notify_all();
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
    writer: Option<Sender<WriterCommand>>,
    life: WriterLife,
    unwritten: Arc<Unwritten>,
    cleanup: Cleanup,
    /// Its child counted live until the reaper has seen it exit.
    child: Option<ChildHold>,
    pane_id: String,
    /// [`Transport::reads_keys`].
    reads_keys: bool,
    submissions: Submissions,
    /// Times input to the next frame; its writer stamps, its reader closes.
    echo: Arc<EchoClock>,
}

impl Session {
    /// Starts the writer for `transport` and returns the session and its
    /// events, which the caller starts reading once the session is
    /// installed. A writer that fails says so through `on_write_failure`.
    pub(super) fn start(
        pane_id: &str,
        generation: u64,
        transport: Transport,
        on_write_failure: Box<dyn Fn(String) + Send>,
    ) -> Result<(Self, Box<dyn SessionEvents>), String> {
        let Transport {
            events,
            writer,
            cleanup,
            size: _,
            reads_keys,
            submissions,
        } = transport;
        let unwritten = Arc::new(Unwritten::default());
        let echo = Arc::new(EchoClock::new(pane_id));
        let life = WriterLife::new();
        let writer = match spawn_session_writer(
            pane_id,
            writer,
            life.clone(),
            Arc::clone(&unwritten),
            Arc::clone(&echo),
            on_write_failure,
        ) {
            Ok(writer) => writer,
            Err(error) => {
                run_cleanup(cleanup);
                return Err(error);
            }
        };
        Ok((
            Self {
                generation,
                writer: Some(writer),
                life,
                unwritten,
                cleanup,
                child: None,
                pane_id: pane_id.to_owned(),
                reads_keys,
                submissions,
                echo,
            },
            events,
        ))
    }

    /// Its writer, which outlives it until what was handed to it is written.
    pub(super) fn life(&self) -> WriterLife {
        self.life.clone()
    }

    /// The clock its reader closes at each frame.
    pub(super) fn echo(&self) -> Arc<EchoClock> {
        Arc::clone(&self.echo)
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

    pub(super) fn command(
        &self,
        input: super::input::Typed,
        command: super::command::Command,
    ) -> Result<(), WriteRefused> {
        if let Err(refused) = self.takes(&input) {
            command.refused(&refused.message(&self.pane_id));
            return Err(refused);
        }
        self.send(WriterCommand::Engine { input, command })
    }

    pub(super) fn write(&self, bytes: &[u8]) -> Result<(), WriteRefused> {
        self.send(WriterCommand::input(bytes))
    }

    /// Pasted text, which the transport wraps for the pane's paste mode.
    pub(super) fn paste(&self, text: Vec<u8>) -> Result<(), WriteRefused> {
        self.send(WriterCommand::Paste(text))
    }

    /// Named keys, which the transport encodes for the pane's keyboard
    /// modes; refused, with nothing sent, by a transport that does not read
    /// them (a holder built before named keys, PRD engine-bundle-3-core-owns
    /// D-21).
    pub(super) fn keys(&self, keys: Vec<Key>) -> Result<(), WriteRefused> {
        if !self.reads_keys {
            return Err(WriteRefused::Unsupported(format!(
                "Pane {}'s holder was built before named keys; they were not sent",
                self.pane_id
            )));
        }
        self.send(WriterCommand::Keys(keys))
    }

    /// One submission ([`SessionWriter::submit`]); refused, with nothing
    /// sent, by a transport that does not take it whole.
    pub(super) fn submit(
        &self,
        submit: Box<hide_holder_protocol::Submit>,
    ) -> Result<(), WriteRefused> {
        let submit = super::input::Typed::Submit(submit);
        self.takes(&submit)?;
        let super::input::Typed::Submit(submit) = submit else {
            unreachable!("built above");
        };
        self.send(WriterCommand::Submit(submit))
    }

    /// How the transport takes a submission.
    pub(super) fn submissions(&self) -> Submissions {
        self.submissions
    }

    /// Refuses input the transport cannot take: named keys a holder built
    /// before them would be sent, or a submission it does not take whole.
    fn takes(&self, input: &super::input::Typed) -> Result<(), WriteRefused> {
        if input.names_keys() && !self.reads_keys {
            return Err(WriteRefused::Unsupported(format!(
                "Pane {}'s holder was built before named keys; input was not sent",
                self.pane_id
            )));
        }
        if matches!(input, super::input::Typed::Submit(_)) && self.submissions != Submissions::Whole
        {
            return Err(WriteRefused::Unsupported(format!(
                "Pane {}'s terminal takes no submission; input was not sent",
                self.pane_id
            )));
        }
        Ok(())
    }

    /// [`Session::write`], telling `written` once the bytes reached the
    /// session's pipe or could not; an `Err` answer tells it nothing.
    pub(super) fn write_then(&self, bytes: &[u8], written: Written) -> Result<(), WriteRefused> {
        self.send(WriterCommand::Input {
            bytes: bytes.to_vec(),
            written: Some(written),
        })
    }

    /// A side-channel write may start after earlier controller writes have
    /// reached the pipe. This barrier emits no terminal protocol or input.
    pub(super) fn after_writes(&self, written: Written) -> Result<(), WriteRefused> {
        self.send(WriterCommand::Barrier(written))
    }

    pub(super) fn resize(&self, rows: u16, cols: u16) -> Result<(), WriteRefused> {
        if rows == 0 || cols == 0 {
            return Err(WriteRefused::Failed(
                "terminal dimensions must be positive".to_owned(),
            ));
        }
        self.send(WriterCommand::Resize(GridSize { rows, cols }))
    }

    /// Asks the transport for the whole screen again at the size the
    /// session runs at.
    pub(super) fn redraw(&self, rows: u16, cols: u16) -> Result<(), WriteRefused> {
        if rows == 0 || cols == 0 {
            return Err(WriteRefused::Failed(
                "terminal dimensions must be positive".to_owned(),
            ));
        }
        self.send(WriterCommand::Redraw(GridSize { rows, cols }))
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

    pub(super) fn mouse(&self, mouse: hide_holder_protocol::Mouse) -> Result<(), WriteRefused> {
        self.send(WriterCommand::Mouse(mouse))
    }
}

fn spawn_session_writer(
    pane_id: &str,
    mut writer: Box<dyn SessionWriter>,
    life: WriterLife,
    unwritten: Arc<Unwritten>,
    echo: Arc<EchoClock>,
    on_failure: Box<dyn Fn(String) + Send>,
) -> Result<Sender<WriterCommand>, String> {
    let (sender, receiver) = channel::<WriterCommand>();
    thread::Builder::new()
        .name(format!("hide-node-terminal-writer-{pane_id}"))
        .spawn(move || {
            let mut ends = Ends {
                life,
                failed: false,
            };
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
                let (result, acknowledgement, written) = match command {
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
                        if lines == 0 {
                            unwritten.bytes.fetch_sub(cost, Ordering::AcqRel);
                            if disconnected {
                                return;
                            }
                            continue;
                        }
                        (writer.scroll(lines, column, row, modifiers), None, None)
                    }
                    WriterCommand::Input { bytes, written } => {
                        echo.input();
                        (writer.input(&bytes), None, written)
                    }
                    WriterCommand::Engine { input, command } => {
                        if !command.begin() {
                            unwritten.bytes.fetch_sub(cost, Ordering::AcqRel);
                            continue;
                        }
                        echo.input();
                        let result = match input {
                            super::input::Typed::Bytes(bytes) => writer.input(&bytes),
                            super::input::Typed::Paste(text) => writer.paste(&text),
                            super::input::Typed::Keys(keys) => writer.keys(&keys),
                            super::input::Typed::Submit(submit) => writer.submit(&submit),
                        };
                        let failed = result.as_ref().err().map(|error| error.to_string());
                        command.finish(result);
                        (
                            failed.map_or(Ok(()), |message| Err(io::Error::other(message))),
                            None,
                            None,
                        )
                    }
                    WriterCommand::Submit(submit) => {
                        echo.input();
                        (writer.submit(&submit), None, None)
                    }
                    WriterCommand::Paste(text) => {
                        echo.input();
                        (writer.paste(&text), None, None)
                    }
                    WriterCommand::Keys(keys) => {
                        echo.input();
                        (writer.keys(&keys), None, None)
                    }
                    WriterCommand::Mouse(mouse) => (writer.mouse(&mouse), None, None),
                    WriterCommand::Resize(size) => (writer.resize(size), None, None),
                    WriterCommand::Redraw(size) => (writer.redraw(size), None, None),
                    WriterCommand::Barrier(written) => (writer.flush(), None, Some(written)),
                    WriterCommand::Release { acknowledged } => {
                        (writer.release(), Some(acknowledged), None)
                    }
                };
                unwritten.bytes.fetch_sub(cost, Ordering::AcqRel);
                if let Some(written) = written {
                    written.finish(result.is_ok());
                }
                if let Some(acknowledgement) = acknowledgement {
                    // A release is the session letting go; a transport that
                    // already left has nothing to be told and no failure to
                    // report.
                    let _ = acknowledgement.send(());
                    return;
                }
                if let Err(error) = result {
                    ends.failed = true;
                    on_failure(format!("terminal control write failed: {error}"));
                    return;
                }
            }
        })
        .map_err(|error| format!("terminal control writer could not be started: {error}"))?;
    Ok(sender)
}

/// Reads `events` until the session ends, handing each event to `on_event`,
/// which answers whether to keep reading. The stream's end, a malformed
/// message and a read failure are each the session closing, with the reason.
pub(super) fn spawn_reader(
    pane_id: &str,
    mut events: Box<dyn SessionEvents>,
    mut on_event: impl FnMut(SessionEvent) -> bool + Send + 'static,
) -> Result<(), String> {
    thread::Builder::new()
        .name(format!("hide-node-terminal-{pane_id}"))
        .spawn(move || {
            loop {
                let event = events.next_event();
                let closed = matches!(event, SessionEvent::Ended { .. });
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
        let pane_id = self.pane_id.clone();
        // Fields drop in order, so a reaper that cannot start still ends the
        // child before it is counted gone.
        let reaping = Arc::new(Mutex::new(Some(Reaping { cleanup, child })));
        let owned = Arc::clone(&reaping);
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
                let Reaping { cleanup, child } = owned
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                    .expect("one reaper");
                run_cleanup(cleanup);
                drop(child);
            })
        {
            // Every Session is retired outside the service lock. If no
            // reaper can start, this caller still owns and completes cleanup.
            if let Some(Reaping { cleanup, child }) =
                reaping.lock().unwrap_or_else(|e| e.into_inner()).take()
            {
                run_cleanup(cleanup);
                drop(child);
            }
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

fn run_cleanup(cleanup: Cleanup) {
    match cleanup {
        Cleanup::Other(shutdown) => shutdown(),
        Cleanup::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{FakeEvents, FakeWriter};
    use super::*;
    use std::io::Write;
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

    /// Starts the writer thread over a stand-in writer into `stdin`.
    fn spawn_writer(
        pane_id: &str,
        stdin: Box<dyn Write + Send>,
        unwritten: Arc<Unwritten>,
        on_failure: Box<dyn Fn(String) + Send>,
    ) -> Result<Sender<WriterCommand>, String> {
        spawn_session_writer(
            pane_id,
            Box::new(FakeWriter(stdin)),
            WriterLife::new(),
            unwritten,
            Arc::new(EchoClock::new(pane_id)),
            on_failure,
        )
    }

    /// A transport whose writer writes into `stdin` and whose events end at
    /// once.
    fn stand_in(stdin: Box<dyn Write + Send>) -> Transport {
        let (_, events) = channel::<SessionEvent>();
        Transport {
            events: Box::new(FakeEvents(events)),
            writer: Box::new(FakeWriter(stdin)),
            cleanup: Cleanup::None,
            size: None,
            reads_keys: false,
            submissions: Submissions::Refused,
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
        writer.send(WriterCommand::input(b"x")).unwrap();
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
            stand_in(Box::new(Stalled {
                go: Arc::clone(&go),
                taken: Arc::clone(&taken),
            })),
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
            stand_in(Box::new(Stalled {
                go: Arc::clone(&go),
                taken: Arc::default(),
            })),
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
        writer.send(WriterCommand::input(b"x")).unwrap();
        let _ = writer.send(WriterCommand::input(b"y"));
        failures
            .recv_timeout(Duration::from_secs(1))
            .expect("the failure is reported");
        // The writer ended with the first failure, closing its end.
        assert!(failures.recv_timeout(Duration::from_secs(1)).is_err());
    }
}
