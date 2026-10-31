//! What a holder does while it lives: it keeps the pane's PTY and program,
//! reads everything the program writes into the terminal parser whether or
//! not a node is connected, serves one node connection at a time, and ends
//! the program's whole session before it ends itself.
//!
//! Threads, each with one job and no lock held across a blocking call:
//! the PTY reader (parses, answers the program's questions, and marks the
//! screen changed without ever waiting for a connection), the PTY writer (the
//! one writer of input and answers, in the order they came, behind bounded
//! queues), the program's waiter, the listener, one short-lived thread per
//! opening connection that reads its first message within its bound and
//! answers an ask, and a reader and a screen writer per accepted connection.
//! Every decision is made on the thread that runs [`Holder::run`], from one
//! event queue.
//!
//! The screen is pulled, not pushed: the PTY reader only marks it changed,
//! and the connection's writer draws the screen as it is when it can write,
//! at most once per [`MIN_FRAGMENT_INTERVAL`], as a fragment of what changed
//! since the last one it wrote. A slow node therefore gets fewer, larger
//! fragments and never a queue, and no connection means no drawing at all
//! (PRD engine-bundle-2-pane-holder D-7, D-9).

use std::collections::VecDeque;
use std::io::{Read, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use hide_holder_protocol::{
    Answer, Ask, Capability, ClientMessage, CloseReason, Closed, Fragment, FrameError, HELLO_LIMIT,
    HolderMessage, Opening as FirstMessage, Size, Unknown, Welcome, agree, read_client,
    read_opening, write_holder,
};
use hide_platform::ipc::{ListenerCloser, LocalListener, LocalStream, ShutdownHandle};
use hide_platform::process::{Session, SessionEnd};
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use serde_json::json;

use crate::Config;
use crate::encode::Encoder;
use crate::log::Log;
use crate::pane::{Pane, Wheel};

/// How long ending the pane's session may take before it is reported as
/// unconfirmed.
const TREE_END_LIMIT: Duration = Duration::from_secs(5);

/// How long the PTY's close and the drain of its last output may take, and,
/// after it, how long the connections still opening are given before each
/// is closed: one bound for all of them, never one per connection.
const DRAIN_LIMIT: Duration = Duration::from_secs(5);

/// How long a closing connection is given to take its last messages.
const FLUSH_LIMIT: Duration = Duration::from_secs(2);

/// How long one write to a connection may wait for the node to read.
const WRITE_LIMIT: Duration = Duration::from_secs(5);

/// The least time between two fragments a connection is sent, as Herdr
/// draws at most once per frame of a 60 Hz screen.
const MIN_FRAGMENT_INTERVAL: Duration = Duration::from_millis(16);

/// The most answer bytes waiting for the program to take them; past it an
/// answer is dropped and logged, and the program's output goes on being read
/// (PRD engine-bundle-2-pane-holder D-9, D-17).
const ANSWER_QUEUE_CAP: usize = 64 * 1024;

/// The most input bytes waiting for the program to take them; past it the
/// connection that sent them is closed (`input_queue_full`).
const INPUT_QUEUE_CAP: usize = 1024 * 1024;

/// The most connections reading their hello at once; one past it is closed
/// with `opening_full`.
const MAX_OPENING: usize = 8;

/// How long the listener gives one refusal of a connection past
/// `MAX_OPENING`, so a burst of them never holds up the accept.
const REFUSE_LIMIT: Duration = Duration::from_millis(250);

/// Debug builds only: a holder started with this variable set panics on its
/// PTY writer's thread at the first input, so a test sees what a panic does.
/// A release holder never reads it, and the pane's program never gets it.
const PANIC_ON_INPUT: &str = "HIDE_HOLDER_TEST_PANIC_ON_INPUT";

/// How much one read of the PTY takes.
const READ_CHUNK: usize = 64 * 1024;

/// What this holder can send.
const SENDS: [Capability; 1] = [Capability::Screen];

enum Event {
    /// A connection whose hello was accepted.
    Opened {
        stream: LocalStream,
        version: u16,
        sends: Vec<Capability>,
    },
    Message {
        generation: u64,
        message: ClientMessage,
    },
    /// A connection's stream ended or failed.
    Gone {
        generation: u64,
        error: FrameError,
    },
    /// A connection's screen could not be read from the parser.
    ScreenFailed {
        generation: u64,
        error: String,
    },
    /// An ask wants the terminal's foreground process group, which only
    /// the PTY knows.
    Foreground {
        reply: Sender<Result<u32, Unknown>>,
    },
    /// The PTY refused input a connection sent; later input from that
    /// connection is dropped, and another connection's is tried again.
    InputFailed {
        generation: u64,
        error: String,
    },
    ProgramExited {
        code: Option<i32>,
    },
    PtyClosed,
}

/// What a connection's writer has to do: messages to send first, then the
/// screen when it changed.
#[derive(Default)]
struct Outbox {
    state: Mutex<Pending>,
    wake: Condvar,
}

#[derive(Default)]
struct Pending {
    messages: VecDeque<HolderMessage>,
    changed: bool,
    full: bool,
    /// One more each time the screen changes.
    version: u64,
    /// Whether the writer waits for the screen to change. Only then does a
    /// change wake it: while it waits out the interval between fragments,
    /// a flood's every read would otherwise wake it for nothing (about
    /// 100,000 wakes for 100 MB on macOS, where a PTY read gives about
    /// 1 KiB).
    waits_for_change: bool,
}

impl Outbox {
    fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn send(&self, message: HolderMessage) {
        self.pending().messages.push_back(message);
        self.wake.notify_one();
    }

    fn changed(&self) {
        let mut pending = self.pending();
        pending.changed = true;
        pending.version += 1;
        let wake = std::mem::take(&mut pending.waits_for_change);
        drop(pending);
        if wake {
            self.wake.notify_one();
        }
    }

    /// The next screen is sent whole.
    fn redraw(&self) {
        self.pending().full = true;
        self.changed();
    }
}

/// The connection that holds the pane, as [`Holder::run`] sees it.
struct Connection {
    generation: u64,
    outbox: Arc<Outbox>,
    shutdown: ShutdownHandle,
    /// Says when the writer has written its last message.
    written: Receiver<()>,
}

impl Connection {
    /// Sends `closed` after what is queued and lets the writer end the
    /// stream; answers once it did or `FLUSH_LIMIT` passed.
    fn close(self, closed: Closed) {
        self.outbox.send(HolderMessage::Closed(closed));
        if self.written.recv_timeout(FLUSH_LIMIT).is_err() {
            self.shutdown.shutdown();
        }
    }
}

/// What the PTY writer writes, in the order it was handed over.
enum PtyWrite {
    /// A connection's input.
    Input { generation: u64, bytes: Vec<u8> },
    /// The holder's answers to the program's questions.
    Answers(Vec<u8>),
}

pub struct Holder {
    log: Log,
    socket: PathBuf,
    /// Says when the listener's thread has dropped it.
    listener_gone: Receiver<()>,
    closer: ListenerCloser,
    session: Session,
    master: Option<Box<dyn MasterPty + Send>>,
    program: u32,
    size: Size,
    pane: Arc<Mutex<Pane>>,
    events: Receiver<Event>,
    sender: Sender<Event>,
    /// The screen of the connection that holds the pane, which the PTY
    /// reader marks changed.
    outbox: Arc<Mutex<Option<Arc<Outbox>>>>,
    /// One more for each full fragment, across connections.
    epochs: Arc<AtomicU64>,
    writes: Sender<PtyWrite>,
    input_queued: Arc<AtomicUsize>,
    answers: Answers,
    current: Option<Connection>,
    generation: u64,
    pty_closed: bool,
    /// Set once the pane ends: what a connection that opens from then on is
    /// closed with.
    ending: Arc<Mutex<Option<Closed>>>,
    opening: Arc<Opening>,
}

impl Holder {
    /// Starts the program on a new PTY and listens at the socket. An error
    /// leaves nothing running.
    /// `job` names the job that holds the holder, for the log.
    pub fn start(config: &Config, log: Log, job: &str) -> Result<Self, String> {
        #[cfg(windows)]
        portable_pty::win::verify_runtime().map_err(|error| error.to_string())?;
        let mut session = Session::new().map_err(|error| format!("the pane's session: {error}"))?;
        let size = config.size;
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("the PTY could not be opened: {error}"))?;
        let mut command = CommandBuilder::from_argv(config.program.clone());
        command.cwd(&config.cwd);
        command.env_remove(hide_platform::process::LENT_KEY);
        let panic_on_input = cfg!(debug_assertions) && std::env::var_os(PANIC_ON_INPUT).is_some();
        command.env_remove(PANIC_ON_INPUT);
        #[cfg(unix)]
        let spawned = pair.slave.spawn_command(command);
        #[cfg(windows)]
        let spawned = pair.slave.spawn_command_in_job(command, session.job());
        let mut child = spawned.map_err(|error| format!("the program could not start: {error}"))?;
        let program = child.process_id().unwrap_or(0);
        if let Err(error) = session.adopt(program) {
            let _ = child.kill();
            return Err(format!("the program's session: {error}"));
        }
        // Started: from here a failure ends the session it started.
        let started = Started {
            session: Some(session),
            log: log.clone(),
        };
        drop(pair.slave);
        let master = pair.master;
        let reader = master
            .try_clone_reader()
            .map_err(|error| format!("the PTY's output: {error}"))?;
        let writer = master
            .take_writer()
            .map_err(|error| format!("the PTY's input: {error}"))?;
        let pane = Pane::new(size)?;
        let listener = LocalListener::bind(&config.socket).map_err(|error| {
            format!(
                "{} could not be listened on: {error}",
                config.socket.display()
            )
        })?;
        let closer = listener.closer();
        let session = started.keep();

        let (sender, events) = mpsc::channel();
        let pane = Arc::new(Mutex::new(pane));
        let outbox: Arc<Mutex<Option<Arc<Outbox>>>> = Arc::new(Mutex::new(None));
        let (writes, writes_rx) = mpsc::channel::<PtyWrite>();
        let input_queued = Arc::new(AtomicUsize::new(0));
        let answers = Answers {
            writes: writes.clone(),
            queued: Arc::new(AtomicUsize::new(0)),
            dropping: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            log: log.clone(),
        };

        spawn_named("pty-reader", {
            let pane = pane.clone();
            let outbox = outbox.clone();
            let sender = sender.clone();
            let answers = answers.clone();
            move || read_pty(reader, &pane, &outbox, &answers, &sender)
        });
        spawn_named("pty-writer", {
            let input_queued = input_queued.clone();
            let answers_queued = answers.queued.clone();
            let sender = sender.clone();
            move || {
                write_pty(
                    writer,
                    &writes_rx,
                    &input_queued,
                    &answers_queued,
                    &sender,
                    panic_on_input,
                )
            }
        });
        spawn_named("program-waiter", {
            let sender = sender.clone();
            move || {
                let code = match child.wait() {
                    Ok(status) if status.signal().is_none() => Some(status.exit_code() as i32),
                    _ => None,
                };
                let _ = sender.send(Event::ProgramExited { code });
            }
        });
        let ending = Arc::new(Mutex::new(None));
        let opening = Arc::new(Opening::default());
        let accept = AcceptSide {
            sender: sender.clone(),
            opening: opening.clone(),
            ending: ending.clone(),
            log: log.clone(),
            pane: pane.clone(),
            shell: program,
        };
        let (gone, listener_gone) = mpsc::channel();
        spawn_named("listener", move || {
            accept.serve(&listener);
            // Dropped here, once the accept has ended: its endpoint goes.
            drop(listener);
            let _ = gone.send(());
        });
        log.event(
            "holder.started",
            &[
                ("program", program.into()),
                ("rows", size.rows.into()),
                ("cols", size.cols.into()),
                ("job", job.into()),
            ],
        );
        Ok(Self {
            log,
            socket: config.socket.clone(),
            listener_gone,
            closer,
            session,
            master: Some(master),
            program,
            size,
            pane,
            events,
            sender,
            outbox,
            epochs: Arc::new(AtomicU64::new(0)),
            writes,
            input_queued,
            answers,
            current: None,
            generation: 0,
            pty_closed: false,
            ending,
            opening,
        })
    }

    pub fn program(&self) -> u32 {
        self.program
    }

    /// Serves until the pane ends, and answers the holder's exit status.
    pub fn run(mut self) -> ExitCode {
        loop {
            let Ok(event) = self.events.recv() else {
                return self.finish(CloseReason::HolderFailed, None, "the event queue ended");
            };
            match event {
                Event::Opened {
                    stream,
                    version,
                    sends,
                } => self.take_over(stream, version, sends),
                Event::Message {
                    generation,
                    message,
                } => {
                    // A connection that was taken over may still have sent
                    // something; it no longer holds the pane.
                    if self.current_generation() != Some(generation) {
                        continue;
                    }
                    match message {
                        ClientMessage::Input(bytes) => self.input(bytes),
                        ClientMessage::Paste(text) => self.paste(&text),
                        ClientMessage::Mouse(mouse) => self.mouse(&mouse),
                        ClientMessage::Scroll(scroll) => self.scroll(&scroll),
                        ClientMessage::Resize(size) => self.resize(size),
                        ClientMessage::Theme(theme) => self.theme(&theme),
                        ClientMessage::Redraw => {
                            if let Some(connection) = &self.current {
                                connection.outbox.redraw();
                            }
                        }
                        ClientMessage::Release => {
                            self.close_current(Closed::new(CloseReason::Released, "released"));
                        }
                        ClientMessage::Terminate => {
                            return self.finish(CloseReason::Terminated, None, "asked to end");
                        }
                    }
                }
                Event::Gone { generation, error } => {
                    if self.current_generation() == Some(generation) {
                        self.log.event(
                            "holder.connection_lost",
                            &[
                                ("generation", generation.into()),
                                ("reason", error.close_reason().name().into()),
                                ("error", error.to_string().into()),
                            ],
                        );
                        let reason = error.close_reason();
                        self.close_current(Closed::new(reason, error.to_string()));
                    }
                }
                Event::ScreenFailed { generation, error } => {
                    self.log.event(
                        "holder.screen_failed",
                        &[
                            ("generation", generation.into()),
                            ("error", error.as_str().into()),
                        ],
                    );
                    if self.current_generation() == Some(generation) {
                        self.close_current(Closed::new(
                            CloseReason::HolderFailed,
                            format!("the screen could not be read: {error}"),
                        ));
                    }
                }
                Event::Foreground { reply } => {
                    let _ = reply.send(self.foreground());
                }
                Event::InputFailed { generation, error } => {
                    self.log.event(
                        "holder.input_failed",
                        &[
                            ("generation", generation.into()),
                            ("error", error.as_str().into()),
                        ],
                    );
                    if self.current_generation() == Some(generation) {
                        self.close_current(Closed::new(
                            CloseReason::InputFailed,
                            format!("the pane's terminal refused input: {error}"),
                        ));
                    }
                }
                Event::ProgramExited { code } => {
                    return self.finish(CloseReason::ProgramExited, code, "the program exited");
                }
                Event::PtyClosed => self.pty_closed = true,
            }
        }
    }

    fn current_generation(&self) -> Option<u64> {
        self.current
            .as_ref()
            .map(|connection| connection.generation)
    }

    /// The opened connection holds the pane from now on: the one it replaces
    /// is told it was taken over, and nothing it sent from here on applies.
    /// Its first fragment is full.
    fn take_over(&mut self, stream: LocalStream, version: u16, sends: Vec<Capability>) {
        self.generation += 1;
        let generation = self.generation;
        if let Some(previous) = self.current.take() {
            self.publish(None);
            self.log.event(
                "holder.taken_over",
                &[
                    ("from", previous.generation.into()),
                    ("to", generation.into()),
                ],
            );
            previous.close(Closed::new(
                CloseReason::TakenOver,
                format!("connection {generation} took the pane over"),
            ));
        }
        let outbox = Arc::new(Outbox::default());
        let (written_tx, written) = mpsc::channel();
        let shutdown = stream.shutdown_handle();
        outbox.send(HolderMessage::Welcome(Welcome {
            version,
            generation,
            program: self.program,
            size: self.size,
            sends: sends.clone(),
        }));
        let writer_stream = stream.duplicate();
        if writer_stream.set_write_timeout(Some(WRITE_LIMIT)).is_err() {
            self.log.event(
                "holder.write_bound_unset",
                &[("generation", generation.into())],
            );
        }
        let screen = sends.contains(&Capability::Screen);
        if screen {
            outbox.redraw();
        }
        spawn_named("connection-writer", {
            let writer = ScreenWriter {
                generation,
                outbox: outbox.clone(),
                pane: screen.then(|| self.pane.clone()),
                epochs: self.epochs.clone(),
                events: self.sender.clone(),
            };
            let shutdown = stream.shutdown_handle();
            move || writer.serve(writer_stream, &shutdown, &written_tx)
        });
        spawn_named("connection-reader", {
            let sender = self.sender.clone();
            move || read_connection(stream, generation, &sender)
        });
        if screen {
            self.publish(Some(outbox.clone()));
        }
        self.current = Some(Connection {
            generation,
            outbox,
            shutdown,
            written,
        });
        self.log
            .event("holder.connected", &[("generation", generation.into())]);
    }

    fn publish(&self, outbox: Option<Arc<Outbox>>) {
        *self.outbox.lock().unwrap_or_else(PoisonError::into_inner) = outbox;
    }

    fn close_current(&mut self, closed: Closed) {
        if let Some(connection) = self.current.take() {
            self.publish(None);
            self.log.event(
                "holder.connection_closed",
                &[
                    ("generation", connection.generation.into()),
                    ("reason", closed.reason.name().into()),
                ],
            );
            connection.close(closed);
        }
    }

    fn pane(&self) -> std::sync::MutexGuard<'_, Pane> {
        self.pane.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The screen changed by something other than the program's output.
    fn screen_changed(&self) {
        if let Some(connection) = &self.current {
            connection.outbox.changed();
        }
    }

    /// Bytes for the program from the connection that holds the pane, which
    /// put the screen back at the bottom as typing does.
    fn input(&mut self, bytes: Vec<u8>) {
        let Some(generation) = self.current_generation() else {
            return;
        };
        if self.pane().scroll_to_bottom() {
            self.screen_changed();
        }
        let length = bytes.len();
        if self.input_queued.load(Ordering::SeqCst) + length > INPUT_QUEUE_CAP {
            self.log.event(
                "holder.input_queue_full",
                &[("queued", self.input_queued.load(Ordering::SeqCst).into())],
            );
            self.close_current(Closed::new(
                CloseReason::InputQueueFull,
                format!("more than {INPUT_QUEUE_CAP} input bytes waited"),
            ));
            return;
        }
        self.input_queued.fetch_add(length, Ordering::SeqCst);
        if self
            .writes
            .send(PtyWrite::Input { generation, bytes })
            .is_err()
        {
            // The writer's thread is gone, which only a panic does, and a
            // panic ends the holder.
            self.input_queued.fetch_sub(length, Ordering::SeqCst);
        }
    }

    fn paste(&mut self, text: &[u8]) {
        let encoded = self.pane().paste(text);
        match encoded {
            Ok(bytes) => self.input(bytes),
            Err(error) => self.meaning_failed("paste", &error),
        }
    }

    fn mouse(&mut self, mouse: &hide_holder_protocol::Mouse) {
        let encoded = self.pane().mouse(mouse);
        match encoded {
            Ok(Some(bytes)) => self.input(bytes),
            // The program tracks no mouse: nothing to send.
            Ok(None) => {}
            Err(error) => self.meaning_failed("mouse", &error),
        }
    }

    fn scroll(&mut self, scroll: &hide_holder_protocol::Scroll) {
        let wheel = self.pane().scroll(scroll);
        match wheel {
            Ok(Wheel::Input(bytes)) => self.input(bytes),
            Ok(Wheel::Moved) => self.screen_changed(),
            Ok(Wheel::Still) => {}
            Err(error) => self.meaning_failed("scroll", &error),
        }
    }

    /// The parser could not say what the pane's settings are, which no input
    /// of the node's causes: logged, and the request dropped.
    fn meaning_failed(&self, what: &str, error: &hide_vt::Error) {
        self.log.event(
            "holder.input_unencoded",
            &[("what", what.into()), ("error", error.to_string().into())],
        );
    }

    fn theme(&mut self, theme: &hide_holder_protocol::Theme) {
        let mut answers = Vec::new();
        let taken = self.pane().theme(theme, &mut answers);
        if let Err(error) = taken {
            self.log.event(
                "holder.theme_failed",
                &[("error", error.to_string().into())],
            );
        }
        self.answers.send(answers);
        self.screen_changed();
    }

    /// The terminal's foreground process group, for an ask.
    fn foreground(&self) -> Result<u32, Unknown> {
        #[cfg(unix)]
        {
            let group = self
                .master
                .as_ref()
                .and_then(|master| master.process_group_leader());
            match group {
                Some(group) if group > 0 => Ok(group as u32),
                _ => Err(Unknown::NoGroup),
            }
        }
        #[cfg(windows)]
        Err(Unknown::NotOnThisSystem)
    }

    fn resize(&mut self, size: hide_holder_protocol::Size) {
        let size = match Size::checked(size.rows, size.cols) {
            Ok(size) => size,
            Err(why) => {
                self.close_current(Closed::new(CloseReason::SizeInvalid, why));
                return;
            }
        };
        if let Some(master) = &self.master
            && let Err(error) = master.resize(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
        {
            self.resize_failed(
                "pty",
                format!(
                    "the pane's terminal refused {}x{}: {error}",
                    size.rows, size.cols
                ),
            );
            return;
        }
        // The terminal has the size now, whatever the parser answers.
        self.size = size;
        let mut answers = Vec::new();
        let resized = self.pane().resize(size, &mut answers);
        self.answers.send(answers);
        self.screen_changed();
        if let Err(error) = resized {
            self.resize_failed(
                "parser",
                format!(
                    "the holder's parser refused {}x{}: {error}",
                    size.rows, size.cols
                ),
            );
        }
    }

    fn resize_failed(&mut self, part: &str, detail: String) {
        self.log.event(
            "holder.resize_failed",
            &[("part", part.into()), ("error", detail.as_str().into())],
        );
        self.close_current(Closed::new(CloseReason::ResizeFailed, detail));
    }

    /// Closes a connection that opened while the pane ends with what ends
    /// it, so no connection is dropped without a reason, by `deadline`.
    fn turn_away(&self, stream: &LocalStream, deadline: Instant) {
        let closed = self
            .ending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| Closed::new(CloseReason::HolderFailed, "the pane is ending"));
        refuse(stream, closed, deadline, &self.log);
    }

    /// Ends the pane: its session first, then the PTY while its last output
    /// drains, then the connection with why, then the holder.
    fn finish(mut self, reason: CloseReason, exit_code: Option<i32>, why: &str) -> ExitCode {
        // From here a connection that opens is closed with why, by its own
        // thread or, when it was already queued, by this one.
        let mut ending = Closed::new(reason, format!("the pane is ending: {why}"));
        ending.exit_code = exit_code;
        *self.ending.lock().unwrap_or_else(PoisonError::into_inner) = Some(ending);
        self.log
            .event("holder.ending", &[("reason", reason.name().into())]);
        self.closer.close();
        let mut status = ExitCode::SUCCESS;
        let mut closed = match self.session.end(TREE_END_LIMIT) {
            Ok(SessionEnd::Ended) => Closed::new(reason, why),
            Ok(SessionEnd::Unconfirmed { left }) => {
                status = ExitCode::FAILURE;
                self.log.event(
                    "holder.tree_end_unconfirmed",
                    &[("left", json!(left)), ("limit_ms", 5000.into())],
                );
                let mut closed = Closed::new(
                    CloseReason::TreeEndUnconfirmed,
                    format!("{} of the pane's processes outlived its end", left.len()),
                );
                closed.left = left;
                closed
            }
            Err(error) => {
                status = ExitCode::FAILURE;
                self.log.event(
                    "holder.tree_end_failed",
                    &[("error", error.to_string().into())],
                );
                Closed::new(
                    CloseReason::HolderFailed,
                    format!("the pane's processes could not be ended: {error}"),
                )
            }
        };
        closed.exit_code = exit_code;
        if !self.close_pty() {
            status = ExitCode::FAILURE;
        }
        if let Some(connection) = self.current.take() {
            self.publish(None);
            connection.close(closed.clone());
        }
        if self.listener_gone.recv_timeout(DRAIN_LIMIT).is_err() {
            self.log.event("holder.listener_close_unconfirmed", &[]);
        }
        // Each opening connection reads its hello within its bound, then
        // closes itself or is queued; the queued ones are closed here, all
        // within the one bound.
        let deadline = Instant::now() + DRAIN_LIMIT;
        if !self.opening.wait_idle(deadline) {
            self.log.event("holder.opening_unconfirmed", &[]);
        }
        while let Ok(event) = self.events.try_recv() {
            if let Event::Opened { stream, .. } = event {
                self.turn_away(&stream, deadline);
            }
        }
        if hide_platform::ipc::is_endpoint(&self.socket).unwrap_or(false) {
            let _ = std::fs::remove_file(&self.socket);
        }
        self.log.event(
            "holder.ended",
            &[
                ("reason", closed.reason.name().into()),
                ("exit_code", json!(exit_code)),
            ],
        );
        status
    }

    /// Closes the PTY on a thread of its own while the reader goes on taking
    /// its output until the end, since a console closing with output nobody
    /// reads can wait for ever; answers whether both finished in time.
    fn close_pty(&mut self) -> bool {
        let Some(master) = self.master.take() else {
            return true;
        };
        let (closed_tx, closed) = mpsc::channel();
        spawn_named("pty-closer", move || {
            drop(master);
            let _ = closed_tx.send(());
        });
        let deadline = Instant::now() + DRAIN_LIMIT;
        let mut pty_closed = self.pty_closed;
        while !pty_closed {
            match self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(Event::PtyClosed) => pty_closed = true,
                Ok(Event::Opened { stream, .. }) => self.turn_away(&stream, deadline),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
        let closer_done = closed
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_ok();
        if !(pty_closed && closer_done) {
            self.log.event(
                "holder.pty_close_unconfirmed",
                &[
                    ("output_ended", pty_closed.into()),
                    ("console_closed", closer_done.into()),
                ],
            );
        }
        pty_closed && closer_done
    }
}

/// A session started but not yet handed to the holder: dropped on a failed
/// start, it ends what it started.
struct Started {
    session: Option<Session>,
    log: Log,
}

impl Started {
    fn keep(mut self) -> Session {
        self.session.take().expect("a started session is kept once")
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        if let Some(mut session) = self.session.take() {
            let ended = session.end(TREE_END_LIMIT);
            if !matches!(ended, Ok(SessionEnd::Ended)) {
                self.log.event(
                    "holder.start_cleanup_unconfirmed",
                    &[("end", format!("{ended:?}").into())],
                );
            }
        }
    }
}

fn spawn_named(name: &str, work: impl FnOnce() + Send + 'static) {
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
        .unwrap_or_else(|error| panic!("the holder could not start its {name} thread: {error}"));
}

/// Hands the program's answers to the PTY writer, past their bound
/// dropping them with a log line for each run of drops.
#[derive(Clone)]
struct Answers {
    writes: Sender<PtyWrite>,
    queued: Arc<AtomicUsize>,
    dropping: Arc<std::sync::atomic::AtomicBool>,
    log: Log,
}

impl Answers {
    fn send(&self, answers: Vec<u8>) {
        if answers.is_empty() {
            return;
        }
        let length = answers.len();
        if self.queued.load(Ordering::SeqCst) + length > ANSWER_QUEUE_CAP {
            if !self.dropping.swap(true, Ordering::SeqCst) {
                self.log.event(
                    "holder.answers_dropped",
                    &[
                        ("bytes", length.into()),
                        ("queued", self.queued.load(Ordering::SeqCst).into()),
                    ],
                );
            }
            return;
        }
        self.dropping.store(false, Ordering::SeqCst);
        self.queued.fetch_add(length, Ordering::SeqCst);
        if self.writes.send(PtyWrite::Answers(answers)).is_err() {
            self.queued.fetch_sub(length, Ordering::SeqCst);
        }
    }
}

/// Reads the PTY until it ends, whether or not a node is connected, so the
/// program never waits on a reader and its questions are answered.
fn read_pty(
    mut reader: Box<dyn Read + Send>,
    pane: &Mutex<Pane>,
    outbox: &Mutex<Option<Arc<Outbox>>>,
    answers: &Answers,
    events: &Sender<Event>,
) {
    let mut buffer = vec![0u8; READ_CHUNK];
    let mut answered = Vec::new();
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            // A Unix PTY answers EIO once no program holds its other end.
            Err(_) => break,
        };
        pane.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .feed(&buffer[..count], &mut answered);
        answers.send(std::mem::take(&mut answered));
        let current = outbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(current) = current {
            current.changed();
        }
    }
    let _ = events.send(Event::PtyClosed);
}

/// The one writer to the program: input and answers in the order they came.
/// Once the PTY refuses a connection's input, the rest it sent is dropped,
/// since that connection is closed; another connection's input is tried
/// again.
fn write_pty(
    mut writer: Box<dyn std::io::Write + Send>,
    writes: &Receiver<PtyWrite>,
    input_queued: &AtomicUsize,
    answers_queued: &AtomicUsize,
    events: &Sender<Event>,
    panic_on_input: bool,
) {
    let mut refused = None;
    for write in writes {
        match write {
            PtyWrite::Input { generation, bytes } => {
                assert!(
                    !panic_on_input,
                    "asked to panic on input ({PANIC_ON_INPUT})"
                );
                if refused != Some(generation)
                    && let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush())
                {
                    refused = Some(generation);
                    let _ = events.send(Event::InputFailed {
                        generation,
                        error: error.to_string(),
                    });
                }
                input_queued.fetch_sub(bytes.len(), Ordering::SeqCst);
            }
            PtyWrite::Answers(bytes) => {
                // A PTY that refuses answers refuses everything; the pane's
                // end follows, and nothing waits for them.
                let _ = writer.write_all(&bytes).and_then(|()| writer.flush());
                answers_queued.fetch_sub(bytes.len(), Ordering::SeqCst);
            }
        }
    }
}

/// A connection's writer: its messages in order, and the screen whenever it
/// changed and the connection can take it.
struct ScreenWriter {
    generation: u64,
    outbox: Arc<Outbox>,
    /// `None` when the node does not take the screen.
    pane: Option<Arc<Mutex<Pane>>>,
    epochs: Arc<AtomicU64>,
    events: Sender<Event>,
}

enum Job {
    Send(HolderMessage),
    Draw { full: bool },
}

impl ScreenWriter {
    fn serve(self, mut stream: LocalStream, shutdown: &ShutdownHandle, written: &Sender<()>) {
        let mut encoder = Encoder::default();
        let mut epoch = 0;
        let mut seq = 0;
        let mut next = Instant::now();
        // The screen version seen while the program held synchronized
        // output, and until when it may.
        let mut held: Option<(u64, Instant)> = None;
        loop {
            let job = self.next_job(next, held);
            let message = match job {
                Job::Send(message) => message,
                Job::Draw { full } => {
                    let Some(pane) = &self.pane else { continue };
                    let taken = {
                        let mut pane = pane.lock().unwrap_or_else(PoisonError::into_inner);
                        match pane.drawable_at() {
                            Some(until) if Instant::now() < until => Err(until),
                            _ => Ok(pane.take(full)),
                        }
                    };
                    let frame = match taken {
                        Err(until) => {
                            let mut pending = self.outbox.pending();
                            pending.changed = true;
                            pending.full |= full;
                            held = Some((pending.version, until));
                            continue;
                        }
                        Ok(Ok(frame)) => frame,
                        Ok(Err(error)) => {
                            let _ = self.events.send(Event::ScreenFailed {
                                generation: self.generation,
                                error: error.to_string(),
                            });
                            continue;
                        }
                    };
                    held = None;
                    let size = Size {
                        rows: frame.rows,
                        cols: frame.cols,
                    };
                    let encoded = encoder.encode(frame, full);
                    let mut sent = true;
                    for (index, bytes) in encoded.pieces.into_iter().enumerate() {
                        let full = encoded.full && index == 0;
                        if full {
                            epoch = self.epochs.fetch_add(1, Ordering::SeqCst) + 1;
                            seq = 0;
                        } else {
                            seq += 1;
                        }
                        let fragment = HolderMessage::Fragment(Fragment {
                            epoch,
                            seq,
                            full,
                            size,
                            bytes,
                        });
                        if write_holder(&mut stream, &fragment).is_err() {
                            sent = false;
                            break;
                        }
                    }
                    next = Instant::now() + MIN_FRAGMENT_INTERVAL;
                    if sent {
                        continue;
                    }
                    break;
                }
            };
            let last = matches!(message, HolderMessage::Closed(_));
            if write_holder(&mut stream, &message).is_err() || last {
                break;
            }
        }
        shutdown.shutdown();
        let _ = written.send(());
    }

    /// Waits for a message to send, or for the screen to have changed by the
    /// time it may be drawn again.
    fn next_job(&self, next: Instant, held: Option<(u64, Instant)>) -> Job {
        let mut pending = self.outbox.pending();
        loop {
            if let Some(message) = pending.messages.pop_front() {
                return Job::Send(message);
            }
            if pending.changed {
                let mut at = next;
                // Held for the program's synchronized output: a change may
                // be the end of it, so it wakes the writer.
                let mut waits_for_change = false;
                if let Some((version, until)) = held
                    && version == pending.version
                {
                    at = at.max(until);
                    waits_for_change = true;
                }
                let now = Instant::now();
                if now >= at {
                    pending.changed = false;
                    pending.waits_for_change = false;
                    return Job::Draw {
                        full: std::mem::take(&mut pending.full),
                    };
                }
                pending.waits_for_change = waits_for_change;
                pending = self
                    .outbox
                    .wake
                    .wait_timeout(pending, at - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
                continue;
            }
            pending.waits_for_change = true;
            pending = self
                .outbox
                .wake
                .wait(pending)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

fn read_connection(mut stream: LocalStream, generation: u64, events: &Sender<Event>) {
    loop {
        match read_client(&mut stream) {
            Ok(message) => {
                let ends = matches!(message, ClientMessage::Release | ClientMessage::Terminate);
                if events
                    .send(Event::Message {
                        generation,
                        message,
                    })
                    .is_err()
                    || ends
                {
                    return;
                }
            }
            Err(error) => {
                let _ = events.send(Event::Gone { generation, error });
                return;
            }
        }
    }
}

/// The listener's side: each connection reads its first message on a thread
/// of its own, so a slow or wrong one never holds up another or the one that
/// holds the pane, and an ask is answered there.
struct AcceptSide {
    sender: Sender<Event>,
    opening: Arc<Opening>,
    ending: Arc<Mutex<Option<Closed>>>,
    log: Log,
    pane: Arc<Mutex<Pane>>,
    shell: u32,
}

/// How many connections are reading their hello, which the pane's end waits
/// to reach none.
#[derive(Default)]
struct Opening {
    count: Mutex<usize>,
    idle: Condvar,
}

impl Opening {
    /// Counts one more, unless `MAX_OPENING` are already.
    fn enter(&self) -> bool {
        let mut count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        if *count >= MAX_OPENING {
            return false;
        }
        *count += 1;
        true
    }

    fn leave(&self) {
        let mut count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        *count -= 1;
        if *count == 0 {
            self.idle.notify_all();
        }
    }

    /// Waits until none is opening, or `deadline` passes.
    fn wait_idle(&self, deadline: Instant) -> bool {
        let mut count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        while *count > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            count = self
                .idle
                .wait_timeout(count, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

impl AcceptSide {
    fn serve(&self, listener: &LocalListener) {
        loop {
            let stream = match listener.accept() {
                Ok(stream) => stream,
                // The closer ends the accept; any other failure ends taking
                // connections too, and the pane runs on unreachable.
                Err(error) => {
                    if error.kind() != std::io::ErrorKind::Interrupted {
                        self.log.event(
                            "holder.accept_ended",
                            &[("error", error.to_string().into())],
                        );
                    }
                    return;
                }
            };
            if !self.opening.enter() {
                refuse(
                    &stream,
                    Closed::new(
                        CloseReason::OpeningFull,
                        format!("{MAX_OPENING} connections were opening already"),
                    ),
                    Instant::now() + REFUSE_LIMIT,
                    &self.log,
                );
                continue;
            }
            let side = Asked {
                sender: self.sender.clone(),
                ending: self.ending.clone(),
                log: self.log.clone(),
                pane: self.pane.clone(),
                shell: self.shell,
            };
            let opening = self.opening.clone();
            spawn_named("connection-opening", move || {
                side.open(stream);
                opening.leave();
            });
        }
    }
}

/// Closes a connection that does not hold the pane with why, by `deadline`:
/// a handle of its own, since the one that read the hello may be past its
/// deadline, and a handle past its deadline writes nothing.
fn refuse(stream: &LocalStream, closed: Closed, deadline: Instant, log: &Log) {
    log.event(
        "holder.connection_refused",
        &[
            ("reason", closed.reason.name().into()),
            ("detail", closed.detail.clone().into()),
        ],
    );
    let mut writer = stream.duplicate();
    writer.set_deadline(deadline);
    if let Err(error) = write_holder(&mut writer, &HolderMessage::Closed(closed)) {
        log.event(
            "holder.refusal_unsent",
            &[("error", error.to_string().into())],
        );
    }
}

/// What an opening connection's thread needs.
struct Asked {
    sender: Sender<Event>,
    ending: Arc<Mutex<Option<Closed>>>,
    log: Log,
    pane: Arc<Mutex<Pane>>,
    shell: u32,
}

impl Asked {
    fn ended(&self) -> Option<Closed> {
        self.ending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn open(&self, stream: LocalStream) {
        let log = &self.log;
        let mut bounded = stream.duplicate();
        bounded.set_deadline(Instant::now() + HELLO_LIMIT);
        if !stream.peer_is_this_account().unwrap_or(false) {
            let closed = Closed::new(CloseReason::HelloInvalid, "the peer is not this account");
            refuse(&stream, closed, Instant::now() + HELLO_LIMIT, log);
            return;
        }
        let hello = match read_opening(&mut bounded) {
            Ok(FirstMessage::Hello(hello)) => hello,
            Ok(FirstMessage::Ask(ask)) => return self.answer(&stream, ask),
            Err(error) => {
                let closed = Closed::new(hello_reason(&error), error.to_string());
                refuse(&stream, closed, Instant::now() + HELLO_LIMIT, log);
                return;
            }
        };
        let (version, sends) = match agree(&hello, &SENDS) {
            Ok(agreed) => agreed,
            Err(closed) => {
                refuse(&stream, closed, Instant::now() + HELLO_LIMIT, log);
                return;
            }
        };
        if let Some(closed) = self.ended() {
            refuse(&stream, closed, Instant::now() + HELLO_LIMIT, log);
            return;
        }
        // Queued once the pane has begun to end, it is closed by the end; a
        // queue already gone means the holder is past its end.
        let opened = Event::Opened {
            stream,
            version,
            sends,
        };
        if let Err(mpsc::SendError(Event::Opened { stream, .. })) = self.sender.send(opened) {
            let closed = self
                .ended()
                .unwrap_or_else(|| Closed::new(CloseReason::HolderFailed, "the holder has ended"));
            refuse(&stream, closed, Instant::now() + HELLO_LIMIT, log);
        }
    }

    /// Answers one ask and closes; the connection that holds the pane, and
    /// the screen's scroll position, stay as they were (D-11, D-18).
    fn answer(&self, stream: &LocalStream, ask: Ask) {
        let deadline = Instant::now() + HELLO_LIMIT;
        if let Some(closed) = self.ended() {
            refuse(stream, closed, deadline, &self.log);
            return;
        }
        if let Err(closed) = ask.check() {
            refuse(stream, closed, deadline, &self.log);
            return;
        }
        let answer = match ask {
            Ask::Read { lines } => self
                .pane
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .read(lines)
                .map_err(|error| format!("the screen could not be read: {error}")),
            Ask::Processes => self.processes(deadline),
        };
        match answer {
            Ok(answer) => {
                let mut writer = stream.duplicate();
                writer.set_deadline(deadline);
                if let Err(error) = write_holder(&mut writer, &HolderMessage::Answer(answer)) {
                    self.log.event(
                        "holder.answer_unsent",
                        &[("error", error.to_string().into())],
                    );
                }
            }
            Err(why) => refuse(
                stream,
                Closed::new(CloseReason::AskFailed, why),
                deadline,
                &self.log,
            ),
        }
    }

    fn processes(&self, deadline: Instant) -> Result<Answer, String> {
        let (reply, group) = mpsc::channel();
        self.sender
            .send(Event::Foreground { reply })
            .map_err(|_| "the holder has ended".to_owned())?;
        let group = group
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| "the pane's terminal did not say its foreground".to_owned())?;
        Ok(crate::ask::processes(self.shell, group))
    }
}

fn hello_reason(error: &FrameError) -> CloseReason {
    match error {
        FrameError::Io(error) if error.kind() == std::io::ErrorKind::TimedOut => {
            CloseReason::HelloTimeout
        }
        FrameError::TooLarge(_) => CloseReason::FrameTooLarge,
        _ => CloseReason::HelloInvalid,
    }
}
