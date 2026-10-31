//! What a holder does while it lives: it keeps the pane's PTY and program,
//! reads everything the program writes into the terminal parser whether or
//! not a node is connected, serves one node connection at a time, and ends
//! the program's whole session before it ends itself.
//!
//! Threads, each with one job and no lock held across a blocking call:
//! the PTY reader (parses, and hands output to the connection's bounded
//! queue without ever waiting for it), the PTY writer (the one writer of
//! input, behind a bounded queue), the program's waiter, the listener, one
//! short-lived thread per opening connection that reads its hello within its
//! bound, and a reader and a writer per accepted connection. Every decision
//! is made on the thread that runs [`Holder::run`], from one event queue.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use hide_holder_protocol::{
    Capability, ClientMessage, CloseReason, Closed, FrameError, HELLO_LIMIT, HolderMessage, Size,
    Welcome, agree, read_client, read_hello, write_holder,
};
use hide_platform::ipc::{ListenerCloser, LocalListener, LocalStream, ShutdownHandle};
use hide_platform::process::{Session, SessionEnd};
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use serde_json::json;

use crate::Config;
use crate::log::Log;

/// How long ending the pane's session may take before it is reported as
/// unconfirmed.
const TREE_END_LIMIT: Duration = Duration::from_secs(5);

/// How long the PTY's close and the drain of its last output may take.
const DRAIN_LIMIT: Duration = Duration::from_secs(5);

/// How long a closing connection is given to take its last messages.
const FLUSH_LIMIT: Duration = Duration::from_secs(2);

/// How long one write to a connection may wait for the node to read.
const WRITE_LIMIT: Duration = Duration::from_secs(5);

/// The most output bytes waiting for a slow connection; past it the
/// connection is closed (`output_queue_full`) and the program runs on.
const OUTPUT_QUEUE_CAP: usize = 4 * 1024 * 1024;

/// The most input bytes waiting for the program to take them; past it the
/// connection that sent them is closed (`input_queue_full`).
const INPUT_QUEUE_CAP: usize = 1024 * 1024;

/// The most connections reading their hello at once; one past it is closed
/// unanswered.
const MAX_OPENING: usize = 8;

/// The parser's scrollback budget, libghostty's approximate bytes (PRD
/// engine-bundle-2-pane-holder D-17).
const SCROLLBACK_BYTES: usize = 2 * 1024 * 1024;

/// How much one read of the PTY takes.
const READ_CHUNK: usize = 64 * 1024;

/// What this holder can send.
const SENDS: [Capability; 1] = [Capability::RawOutput];

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
    /// A connection fell past the output bound.
    OutputOverflow {
        generation: u64,
    },
    ProgramExited {
        code: Option<i32>,
    },
    PtyClosed,
}

/// The connection that holds the pane, as the PTY reader sees it.
struct Outbound {
    generation: u64,
    raw: bool,
    overflowed: bool,
    sender: Sender<HolderMessage>,
    queued: Arc<AtomicUsize>,
}

/// The connection that holds the pane, as [`Holder::run`] sees it.
struct Connection {
    generation: u64,
    sender: Sender<HolderMessage>,
    shutdown: ShutdownHandle,
    /// Says when the writer has written its last message.
    written: Receiver<()>,
}

impl Connection {
    /// Sends `closed` after what is queued and lets the writer end the
    /// stream; answers once it did or `FLUSH_LIMIT` passed.
    fn close(self, closed: Closed) {
        let _ = self.sender.send(HolderMessage::Closed(closed));
        drop(self.sender);
        if self.written.recv_timeout(FLUSH_LIMIT).is_err() {
            self.shutdown.shutdown();
        }
    }
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
    terminal: Arc<Mutex<hide_vt::Terminal>>,
    events: Receiver<Event>,
    sender: Sender<Event>,
    outbound: Arc<Mutex<Option<Outbound>>>,
    input: Sender<Vec<u8>>,
    input_queued: Arc<AtomicUsize>,
    current: Option<Connection>,
    generation: u64,
    pty_closed: bool,
}

impl Holder {
    /// Starts the program on a new PTY and listens at the socket. An error
    /// leaves nothing running.
    pub fn start(config: &Config, log: Log) -> Result<Self, String> {
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
        if let Some(cwd) = &config.cwd {
            command.cwd(cwd);
        }
        command.env_remove(hide_platform::process::LENT_KEY);
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
        let terminal = hide_vt::Terminal::new(size.cols, size.rows, SCROLLBACK_BYTES)
            .map_err(|error| format!("the terminal parser: {error}"))?;
        let listener = LocalListener::bind(&config.socket).map_err(|error| {
            format!(
                "{} could not be listened on: {error}",
                config.socket.display()
            )
        })?;
        let closer = listener.closer();
        let session = started.keep();

        let (sender, events) = mpsc::channel();
        let terminal = Arc::new(Mutex::new(terminal));
        let outbound: Arc<Mutex<Option<Outbound>>> = Arc::new(Mutex::new(None));
        let (input, input_rx) = mpsc::channel::<Vec<u8>>();
        let input_queued = Arc::new(AtomicUsize::new(0));

        spawn_named("pty-reader", {
            let terminal = terminal.clone();
            let outbound = outbound.clone();
            let sender = sender.clone();
            move || read_pty(reader, &terminal, &outbound, &sender)
        });
        spawn_named("pty-writer", {
            let queued = input_queued.clone();
            let log = log.clone();
            move || write_pty(writer, &input_rx, &queued, &log)
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
        let accept = AcceptSide {
            sender: sender.clone(),
            opening: Arc::new(AtomicUsize::new(0)),
            log: log.clone(),
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
            terminal,
            events,
            sender,
            outbound,
            input,
            input_queued,
            current: None,
            generation: 0,
            pty_closed: false,
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
                        ClientMessage::Resize(size) => self.resize(size),
                        ClientMessage::Release => {
                            self.close_current(Closed::new(CloseReason::Released, "released"));
                        }
                        ClientMessage::Terminate => {
                            return self.finish(CloseReason::Terminated, None, "asked to end");
                        }
                        ClientMessage::Hello(_) => self.close_current(Closed::new(
                            CloseReason::ProtocolError,
                            "a second hello",
                        )),
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
                Event::OutputOverflow { generation } => {
                    if self.current_generation() == Some(generation) {
                        self.log.event(
                            "holder.output_queue_full",
                            &[("generation", generation.into())],
                        );
                        self.close_current(Closed::new(
                            CloseReason::OutputQueueFull,
                            format!("more than {OUTPUT_QUEUE_CAP} output bytes waited"),
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
        let (sender, outgoing) = mpsc::channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let (written_tx, written) = mpsc::channel();
        let shutdown = stream.shutdown_handle();
        let _ = sender.send(HolderMessage::Welcome(Welcome {
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
        spawn_named("connection-writer", {
            let queued = queued.clone();
            let shutdown = stream.shutdown_handle();
            move || write_connection(writer_stream, &outgoing, &queued, &shutdown, &written_tx)
        });
        spawn_named("connection-reader", {
            let sender = self.sender.clone();
            move || read_connection(stream, generation, &sender)
        });
        self.publish(Some(Outbound {
            generation,
            raw: sends.contains(&Capability::RawOutput),
            overflowed: false,
            sender: sender.clone(),
            queued,
        }));
        self.current = Some(Connection {
            generation,
            sender,
            shutdown,
            written,
        });
        self.log
            .event("holder.connected", &[("generation", generation.into())]);
    }

    fn publish(&self, outbound: Option<Outbound>) {
        *self.outbound.lock().unwrap_or_else(PoisonError::into_inner) = outbound;
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

    fn input(&mut self, bytes: Vec<u8>) {
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
        if self.input.send(bytes).is_err() {
            self.input_queued.fetch_sub(length, Ordering::SeqCst);
        }
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
            self.log.event(
                "holder.resize_failed",
                &[("error", error.to_string().into())],
            );
            return;
        }
        let resized = self
            .terminal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .resize(size.cols, size.rows, 0, 0);
        if let Err(error) = resized {
            self.log.event(
                "holder.parser_resize_failed",
                &[("error", error.to_string().into())],
            );
        }
        self.size = size;
    }

    /// Ends the pane: its session first, then the PTY while its last output
    /// drains, then the connection with why, then the holder.
    fn finish(mut self, reason: CloseReason, exit_code: Option<i32>, why: &str) -> ExitCode {
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

/// Reads the PTY until it ends, whether or not a node is connected, so the
/// program never waits on a reader.
fn read_pty(
    mut reader: Box<dyn Read + Send>,
    terminal: &Mutex<hide_vt::Terminal>,
    outbound: &Mutex<Option<Outbound>>,
    events: &Sender<Event>,
) {
    let mut buffer = vec![0u8; READ_CHUNK];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            // A Unix PTY answers EIO once no program holds its other end.
            Err(_) => break,
        };
        let bytes = &buffer[..count];
        terminal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .write(bytes);
        let mut outbound = outbound.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(out) = outbound.as_mut()
            && out.raw
            && !out.overflowed
        {
            if out.queued.load(Ordering::SeqCst) + count > OUTPUT_QUEUE_CAP {
                out.overflowed = true;
                let _ = events.send(Event::OutputOverflow {
                    generation: out.generation,
                });
            } else {
                out.queued.fetch_add(count, Ordering::SeqCst);
                let _ = out.sender.send(HolderMessage::Output(bytes.to_vec()));
            }
        }
    }
    let _ = events.send(Event::PtyClosed);
}

/// The one writer of the program's input.
fn write_pty(
    mut writer: Box<dyn Write + Send>,
    input: &Receiver<Vec<u8>>,
    queued: &AtomicUsize,
    log: &Log,
) {
    let mut failed = false;
    for bytes in input {
        if !failed && let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
            failed = true;
            log.event(
                "holder.input_failed",
                &[("error", error.to_string().into())],
            );
        }
        queued.fetch_sub(bytes.len(), Ordering::SeqCst);
    }
}

fn write_connection(
    mut stream: LocalStream,
    outgoing: &Receiver<HolderMessage>,
    queued: &AtomicUsize,
    shutdown: &ShutdownHandle,
    written: &Sender<()>,
) {
    for message in outgoing {
        let output = match &message {
            HolderMessage::Output(bytes) => bytes.len(),
            _ => 0,
        };
        let last = matches!(message, HolderMessage::Closed(_));
        let sent = write_holder(&mut stream, &message);
        queued.fetch_sub(output, Ordering::SeqCst);
        if sent.is_err() || last {
            break;
        }
    }
    shutdown.shutdown();
    let _ = written.send(());
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

/// The listener's side: each connection reads its hello on a thread of its
/// own, so a slow or wrong one never holds up another or the one that holds
/// the pane.
struct AcceptSide {
    sender: Sender<Event>,
    opening: Arc<AtomicUsize>,
    log: Log,
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
            if self.opening.fetch_add(1, Ordering::SeqCst) >= MAX_OPENING {
                self.opening.fetch_sub(1, Ordering::SeqCst);
                self.log.event("holder.opening_full", &[]);
                continue;
            }
            let sender = self.sender.clone();
            let opening = self.opening.clone();
            let log = self.log.clone();
            spawn_named("connection-opening", move || {
                open_connection(stream, &sender, &log);
                opening.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }
}

fn open_connection(stream: LocalStream, events: &Sender<Event>, log: &Log) {
    let mut bounded = stream.duplicate();
    bounded.set_deadline(Instant::now() + HELLO_LIMIT);
    // The refusal gets a deadline of its own: the hello's may have passed,
    // and a handle past its deadline writes nothing.
    let refuse = |closed: Closed| {
        log.event(
            "holder.connection_refused",
            &[
                ("reason", closed.reason.name().into()),
                ("detail", closed.detail.clone().into()),
            ],
        );
        let mut writer = stream.duplicate();
        writer.set_deadline(Instant::now() + HELLO_LIMIT);
        let _ = write_holder(&mut writer, &HolderMessage::Closed(closed));
    };
    if !stream.peer_is_this_account().unwrap_or(false) {
        refuse(Closed::new(
            CloseReason::HelloInvalid,
            "the peer is not this account",
        ));
        return;
    }
    let hello = match read_hello(&mut bounded) {
        Ok(hello) => hello,
        Err(error) => {
            refuse(Closed::new(hello_reason(&error), error.to_string()));
            return;
        }
    };
    match agree(&hello, &SENDS) {
        Ok((version, sends)) => {
            let _ = events.send(Event::Opened {
                stream,
                version,
                sends,
            });
        }
        Err(closed) => refuse(closed),
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
