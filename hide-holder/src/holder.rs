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
//! answers an ask, a reader and a screen writer per accepted connection, and
//! a reader and a facts writer for the one watch that follows the pane
//! ([`crate::watch`]). Every decision is made on the thread that runs
//! [`Holder::run`], from one event queue, except the program's input: a
//! connection's reader encodes every message that becomes input and hands
//! it to the PTY writer itself, in the order the connection sent it, so a
//! key wakes one thread fewer and nothing on the run thread holds it up
//! (PRD engine-bundle-3-core-owns D-12a).
//!
//! The screen is pulled, not pushed: the PTY reader only marks it changed,
//! and the connection's writer draws the screen as it is when it can write,
//! at most once per [`MIN_FRAGMENT_INTERVAL`], as a fragment of what changed
//! since the last one it wrote; the program's answer to input is drawn at
//! once ([`Pending::input`]). A slow node therefore gets fewer, larger
//! fragments and never a queue, and no connection means no drawing at all
//! (PRD engine-bundle-2-pane-holder D-7, D-9). A watch's facts are pulled
//! the same way.

use std::cell::Cell;
use std::collections::VecDeque;
use std::io::{Read, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use hide_holder_protocol::{
    Ask, Capability, ClientMessage, CloseReason, Closed, Fragment, FrameError, HELLO_LIMIT, Hello,
    HolderMessage, LineEditing, Notice, NoticeReason, Opening as FirstMessage, Size, SubmitPart,
    Unknown, Watching, Welcome, agree, read_client, read_opening, write_holder,
};
use hide_platform::ipc::{ListenerCloser, LocalListener, LocalStream, ShutdownHandle};
use hide_platform::process::{Session, SessionEnd};
use portable_pty::{CommandBuilder, MasterPty, PtySize};
use serde_json::json;

use crate::Config;
use crate::encode::Encoder;
use crate::log::Log;
use crate::pane::{Pane, Wheel};
use crate::record::Claim;
use crate::watch::{FactsWriter, LineEditingRead, SharedRules, Wake, WatchConnection, WatchReader};

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

/// The least time between two fragments a connection is sent for output the
/// program makes on its own, as Herdr draws at most once per frame of a
/// 60 Hz screen; the program's answer to input does not wait for it
/// ([`Pending::input`]).
const MIN_FRAGMENT_INTERVAL: Duration = Duration::from_millis(16);

/// How long a connection's screen stays still after a fragment before the
/// writer gives the allocator's freed pages back to the system, once, until
/// the next fragment ([`hide_platform::memory::return_freed`]).
const RETURN_FREED_AFTER: Duration = Duration::from_secs(1);

/// The program output, past what a screen or two of a redraw takes, after
/// which a connection's next return of freed pages is logged; a
/// connection's first return is logged too, and every other return is not,
/// so a pane that redraws a line every second or two does not fill its log.
const LOGGED_RETURN_BYTES: u64 = 1024 * 1024;

/// The most answer bytes waiting for the program to take them; past it the
/// answers are refused with a reason, logged and told to the connection
/// that accepts notices, and the program's output goes on being read (PRD
/// engine-bundle-2-pane-holder D-9, D-17).
const ANSWER_QUEUE_CAP: usize = 64 * 1024;

/// The most input bytes waiting for the program to take them; past it the
/// connection that sent them is closed (`input_queue_full`).
const INPUT_QUEUE_CAP: usize = 1024 * 1024;

/// The least a queued submission counts against [`INPUT_QUEUE_CAP`]: it
/// holds the writer for its gap whatever its size, so at most 256 wait,
/// and a connection that sends more is ended as one past the cap is.
const SUBMIT_COST: usize = 4 * 1024;

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

/// Debug builds only: a holder started with this variable naming a file
/// writes nothing to its program until that file exists, so a test fills
/// its input and answer queues to their bounds as a program that reads
/// nothing would, on every system alike (a Windows console takes all the
/// input it is given). A release holder never reads it, and the pane's
/// program never gets it.
const HOLD_WRITES: &str = "HIDE_HOLDER_TEST_HOLD_WRITES";

/// How much one read of the PTY takes.
const READ_CHUNK: usize = 64 * 1024;

/// What this holder can send, and what it reads beyond version 1's input
/// ([`Capability::Keys`]).
const SENDS: [Capability; 4] = [
    Capability::Screen,
    Capability::Notices,
    Capability::Keys,
    Capability::Submit,
];

/// What this holder sends on a watch.
const WATCH_SENDS: [Capability; 4] = [
    Capability::Facts,
    Capability::ProgramPackage,
    Capability::LineEditing,
    Capability::ForegroundUnread,
];

enum Event {
    /// A connection whose hello was accepted.
    Opened {
        stream: LocalStream,
        version: u16,
        sends: Vec<Capability>,
    },
    /// What a connection asks of the run thread: all it sends but the
    /// program's input, which its reader hands the PTY writer itself.
    Message {
        generation: u64,
        message: Control,
    },
    /// A connection's input passed [`INPUT_QUEUE_CAP`] waiting bytes.
    InputQueueFull {
        generation: u64,
        queued: usize,
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
    /// A watch whose opening was accepted.
    Watch {
        stream: LocalStream,
        version: u16,
        sends: Vec<Capability>,
    },
    /// A watch's node let go (`None`), or its stream ended or failed.
    WatchGone {
        generation: u64,
        error: Option<FrameError>,
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
    /// Something refused while the pane runs on, for the connection that
    /// holds it.
    Notice(Notice),
}

/// What a connection sends that is not the program's input.
enum Control {
    Resize(hide_holder_protocol::Size),
    Theme(hide_holder_protocol::Theme),
    Redraw,
    Release,
    Terminate,
}

/// What a connection's reader needs to turn the program's input into PTY
/// writes itself, off the run thread: every message that becomes input goes
/// to the one PTY writer in the order the connection sent it, encoded for
/// the pane's settings at that moment under the pane's lock (PRD
/// engine-bundle-3-core-owns D-12a).
#[derive(Clone)]
struct InputPath {
    /// The generation of the connection that holds the pane, 0 for none. A
    /// reader holds it from its check until its input is queued, and a
    /// takeover or a close changes it only between two of them, so input is
    /// queued exactly when the run thread would have taken it: what a
    /// connection sent before another took the pane over is written, what
    /// it sent after is not.
    holding: Arc<Mutex<u64>>,
    pane: Arc<Mutex<Pane>>,
    writes: Sender<PtyWrite>,
    queued: Arc<AtomicUsize>,
    events: Sender<Event>,
    log: Log,
}

/// What a reader does after one input message.
enum Taken {
    /// Read on.
    Next,
    /// The connection is to be closed: its input passed the cap.
    Full,
}

impl InputPath {
    fn holding(&self) -> MutexGuard<'_, u64> {
        self.holding.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn taken_over(&self, generation: u64) {
        *self.holding() = generation;
    }

    fn let_go(&self) {
        *self.holding() = 0;
    }

    /// Encodes `message` for the pane's settings now and hands its bytes to
    /// the PTY writer, putting the screen back at the bottom as typing does,
    /// when `generation` still holds the pane. The lock order is the
    /// generation, then the pane.
    fn take(&self, generation: u64, outbox: &Outbox, message: ClientMessage) -> Taken {
        let holding = self.holding();
        if *holding != generation {
            return Taken::Next;
        }
        let (write, moved) = {
            let mut pane = self.pane.lock().unwrap_or_else(PoisonError::into_inner);
            if let ClientMessage::Submit(submit) = &message {
                let part = |pane: &mut Pane, part: &SubmitPart| match part {
                    SubmitPart::Input(bytes) => Ok(bytes.clone()),
                    SubmitPart::Paste(text) => pane.paste(text),
                    SubmitPart::Keys(keys) => pane.keys(keys),
                };
                let encoded = submit
                    .text
                    .iter()
                    .try_fold(Vec::new(), |mut text, each| {
                        text.extend(part(&mut pane, each)?);
                        Ok::<_, hide_vt::Error>(text)
                    })
                    .and_then(|text| Ok((text, part(&mut pane, &submit.enter)?)));
                let (text, enter) = match encoded {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        drop(pane);
                        self.log.event(
                            "holder.input_unencoded",
                            &[
                                ("what", "submit".into()),
                                ("error", error.to_string().into()),
                            ],
                        );
                        return Taken::Next;
                    }
                };
                let moved = pane.scroll_to_bottom();
                (
                    PtyWrite::Submit {
                        generation,
                        text,
                        gap: submit.gap,
                        enter,
                    },
                    moved,
                )
            } else {
                let encoded = match &message {
                    ClientMessage::Input(_) => Ok(None),
                    ClientMessage::Paste(text) => pane.paste(text).map(Some),
                    ClientMessage::Mouse(mouse) => pane.mouse(mouse),
                    ClientMessage::Key(keys) => pane.keys(keys).map(Some),
                    ClientMessage::Scroll(scroll) => match pane.scroll(scroll) {
                        Ok(Wheel::Input(bytes)) => Ok(Some(bytes)),
                        // The screen moved, or did not: nothing for the program.
                        Ok(Wheel::Moved) => {
                            drop(pane);
                            outbox.changed();
                            return Taken::Next;
                        }
                        Ok(Wheel::Still) => return Taken::Next,
                        Err(error) => Err(error),
                    },
                    _ => unreachable!("a reader hands over only the program's input"),
                };
                let bytes = match (encoded, message) {
                    (Ok(None), ClientMessage::Input(bytes)) => bytes,
                    (Ok(Some(bytes)), _) => bytes,
                    // The program tracks no mouse: nothing to send.
                    (Ok(None), _) => return Taken::Next,
                    (Err(error), message) => {
                        drop(pane);
                        self.log.event(
                            "holder.input_unencoded",
                            &[
                                ("what", input_name(&message).into()),
                                ("error", error.to_string().into()),
                            ],
                        );
                        return Taken::Next;
                    }
                };
                let moved = pane.scroll_to_bottom();
                (PtyWrite::Input { generation, bytes }, moved)
            }
        };
        if moved {
            outbox.changed();
        }
        let length = write.len();
        // Held to the end, so the takeover waits for this input to be queued.
        let _holding = holding;
        let queued = self.queued.load(Ordering::SeqCst);
        if queued + length > INPUT_QUEUE_CAP {
            let _ = self
                .events
                .send(Event::InputQueueFull { generation, queued });
            return Taken::Full;
        }
        self.queued.fetch_add(length, Ordering::SeqCst);
        // Before the program can answer, so its answer is never taken for
        // a change before it.
        outbox.input();
        if self.writes.send(write).is_err() {
            // The writer's thread is gone, which only a panic does, and a
            // panic ends the holder.
            self.queued.fetch_sub(length, Ordering::SeqCst);
        }
        Taken::Next
    }
}

fn input_name(message: &ClientMessage) -> &'static str {
    match message {
        ClientMessage::Submit(_) => "submit",
        ClientMessage::Paste(_) => "paste",
        ClientMessage::Mouse(_) => "mouse",
        ClientMessage::Scroll(_) => "scroll",
        ClientMessage::Key(_) => "key",
        _ => "input",
    }
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
    /// The screen's version when input was handed to the program and no
    /// fragment has answered it yet: a change after it is the program's
    /// answer, such as a key's echo, and is drawn at once rather than after
    /// the interval between fragments, so an echo never waits on the
    /// fragment before it. One fragment more per input message at most;
    /// output the program makes on its own stays paced.
    input: Option<u64>,
}

impl Pending {
    /// When the changed screen may be drawn, and whether a change should
    /// wake the writer before then: once `next` passes, at once when the
    /// change answers input, and never before the end of the synchronized
    /// output the screen was `held` at, unless it changes again.
    fn draw_at(
        &self,
        next: Instant,
        held: Option<(u64, Instant)>,
        now: Instant,
    ) -> (Instant, bool) {
        let mut at = if self.answers_input() { now } else { next };
        // Held for the program's synchronized output: a change may be the
        // end of it, so it wakes the writer.
        let mut waits_for_change = false;
        if let Some((version, until)) = held
            && version == self.version
        {
            at = at.max(until);
            waits_for_change = true;
        }
        (at, waits_for_change)
    }

    fn answers_input(&self) -> bool {
        self.input.is_some_and(|input| input < self.version)
    }
}

impl Outbox {
    fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn send(&self, message: HolderMessage) {
        self.pending().messages.push_back(message);
        self.wake.notify_one();
    }

    /// Queues `notice`, folded into the one of its reason still waiting, so
    /// a writer that does not keep up holds at most one notice per reason
    /// however often answers are refused and resumed. The folded notice goes
    /// last, so the last one sent says what holds now.
    fn notice(&self, notice: Notice) {
        let mut pending = self.pending();
        let waiting = pending.messages.iter().position(
            |message| matches!(message, HolderMessage::Notice(queued) if queued.reason == notice.reason),
        );
        let notice = match waiting.and_then(|index| pending.messages.remove(index)) {
            Some(HolderMessage::Notice(queued)) => fold(queued, notice),
            _ => notice,
        };
        pending.messages.push_back(HolderMessage::Notice(notice));
        drop(pending);
        self.wake.notify_one();
    }

    fn changed(&self) {
        let mut pending = self.pending();
        pending.changed = true;
        pending.version += 1;
        // The program's answer to input is drawn at once, so it wakes a
        // writer waiting out the interval too.
        let wake = std::mem::take(&mut pending.waits_for_change) || pending.input.is_some();
        drop(pending);
        if wake {
            self.wake.notify_one();
        }
    }

    /// Input is being handed to the program: the next change is its answer
    /// ([`Pending::input`]).
    fn input(&self) {
        let mut pending = self.pending();
        let version = pending.version;
        pending.input.get_or_insert(version);
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
    /// Whether the node accepts [`Notice`]s.
    notices: bool,
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
    /// A connection's submission ([`hide_holder_protocol::Submit`]): `text`,
    /// then `enter` once `gap` has passed since `text` was written, with the
    /// input written after it waiting until then.
    Submit {
        generation: u64,
        text: Vec<u8>,
        gap: Duration,
        enter: Vec<u8>,
    },
    /// The holder's answers to the program's questions.
    Answers(Vec<u8>),
}

/// When this process began and when it had written itself into its record,
/// so `holder.started` says where its start went: the claim, the parser and
/// socket, the program's spawn and the record's `running`.
pub struct StartClock {
    pub begun: Instant,
    pub claimed: Duration,
}

/// `elapsed` in milliseconds to a tenth.
fn millis(elapsed: Duration) -> f64 {
    (elapsed.as_secs_f64() * 10_000.0).round() / 10.0
}

pub struct Holder {
    log: Log,
    socket: PathBuf,
    /// Says when the listener's thread has dropped it.
    listener_gone: Receiver<()>,
    closer: ListenerCloser,
    session: Session,
    master: PtyMaster,
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
    input: InputPath,
    answers: Answers,
    current: Option<Connection>,
    generation: u64,
    /// The watch that follows the pane, its writer, which the PTY reader
    /// marks changed, and the rules it reads the screen by.
    watch: Option<WatchConnection>,
    watch_generation: u64,
    watch_wake: Arc<Mutex<Option<Arc<Wake>>>>,
    rules: SharedRules,
    pty_closed: bool,
    /// Set once the pane ends: what a connection that opens from then on is
    /// closed with.
    ending: Arc<Mutex<Option<Closed>>>,
    opening: Arc<Opening>,
    /// The pane's record, which the holder writes why the pane ended into.
    record: Option<Claim>,
}

impl Holder {
    /// Starts the program on a new PTY and listens at the socket. An error
    /// leaves nothing running.
    /// `job` names the job that holds the holder, for the log; `record` is
    /// the pane's record, told the program and `running` in one write once
    /// the program runs, the socket listening before it starts.
    pub fn start(
        config: &Config,
        log: Log,
        job: &str,
        record: Option<Claim>,
        clock: &StartClock,
    ) -> Result<Self, String> {
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
        // The parser and the socket before the program, so that once the
        // program runs, one write of the record says it runs and the socket
        // listens; a failure from here to the spawn leaves no program, and
        // the dropped listener takes its endpoint with it.
        let pane = Pane::new(size, config.theme.as_ref())?;
        let listener = LocalListener::bind(&config.socket).map_err(|error| {
            format!(
                "{} could not be listened on: {error}",
                config.socket.display()
            )
        })?;
        let closer = listener.closer();
        let listening_at = clock.begun.elapsed();
        let mut command = CommandBuilder::from_argv(config.program.clone());
        command.cwd(&config.cwd);
        // The terminal the program writes to is the holder's, and nothing
        // else's: no Herdr pane, outer terminal or parent agent (D-9).
        crate::pane_env::apply(&mut command);
        command.env_remove(hide_platform::process::LENT_KEY);
        // The holder's own launch metadata, never its program's: a guarded
        // launch in the pane (Hide's hook helper) would take the holder's
        // owner descriptor for its own, fail to watch it and do nothing.
        for key in hide_platform::process::OWNER_LAUNCH_KEYS {
            command.env_remove(key);
        }
        let panic_on_input = cfg!(debug_assertions) && std::env::var_os(PANIC_ON_INPUT).is_some();
        command.env_remove(PANIC_ON_INPUT);
        let hold_writes = std::env::var_os(HOLD_WRITES)
            .filter(|_| cfg!(debug_assertions))
            .map(PathBuf::from);
        command.env_remove(HOLD_WRITES);
        #[cfg(unix)]
        let spawned = pair.slave.spawn_command(command);
        #[cfg(windows)]
        let spawned = pair.slave.spawn_command_in_job(command, session.job());
        let mut child = spawned.map_err(|error| format!("the program could not start: {error}"))?;
        let spawned_at = clock.begun.elapsed();
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
        // The program and `running` in one write of the record: the window
        // in which the program runs and the record does not name it is that
        // write, as it was when the program had a write of its own, and a
        // start writes its record once less (each write is flushed, and the
        // start of every new pane waited for them).
        if let Some(record) = &record {
            record
                .running(program)
                .map_err(|reason| format!("the pane's record: {reason}"))?;
        }
        let running_at = clock.begun.elapsed();
        drop(pair.slave);
        let master = pair.master;
        let reader = master
            .try_clone_reader()
            .map_err(|error| format!("the PTY's output: {error}"))?;
        let writer = master
            .take_writer()
            .map_err(|error| format!("the PTY's input: {error}"))?;
        let master = PtyMaster(Arc::new(Mutex::new(Some(master))));
        let session = started.keep();

        let (sender, events) = mpsc::channel();
        let pane = Arc::new(Mutex::new(pane));
        let outbox: Arc<Mutex<Option<Arc<Outbox>>>> = Arc::new(Mutex::new(None));
        let watch_wake: Arc<Mutex<Option<Arc<Wake>>>> = Arc::new(Mutex::new(None));
        let (writes, writes_rx) = mpsc::channel::<PtyWrite>();
        let input_queued = Arc::new(AtomicUsize::new(0));
        let answers = Answers {
            writes: writes.clone(),
            queued: Arc::new(AtomicUsize::new(0)),
            refused: Arc::new(Mutex::new(None)),
            events: sender.clone(),
            log: log.clone(),
        };

        spawn_named("pty-reader", {
            let pane = pane.clone();
            let marks = Marks {
                outbox: outbox.clone(),
                watch: watch_wake.clone(),
            };
            let sender = sender.clone();
            let answers = answers.clone();
            move || read_pty(reader, &pane, &marks, &answers, &sender)
        });
        let input = InputPath {
            holding: Arc::new(Mutex::new(0)),
            pane: pane.clone(),
            writes,
            queued: input_queued,
            events: sender.clone(),
            log: log.clone(),
        };
        spawn_named("pty-writer", {
            let input = input.clone();
            let answers_queued = answers.queued.clone();
            let sender = sender.clone();
            move || {
                write_pty(
                    writer,
                    &writes_rx,
                    &input,
                    &answers_queued,
                    &sender,
                    panic_on_input,
                    hold_writes.as_deref(),
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
            outbox: outbox.clone(),
            shell: program,
            master: master.clone(),
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
                ("claimed_ms", millis(clock.claimed).into()),
                ("listening_ms", millis(listening_at).into()),
                ("spawned_ms", millis(spawned_at).into()),
                ("running_ms", millis(running_at).into()),
            ],
        );
        Ok(Self {
            log,
            socket: config.socket.clone(),
            listener_gone,
            closer,
            session,
            master,
            program,
            size,
            pane,
            events,
            sender,
            outbox,
            epochs: Arc::new(AtomicU64::new(0)),
            input,
            answers,
            current: None,
            generation: 0,
            watch: None,
            watch_generation: 0,
            watch_wake,
            rules: SharedRules::default(),
            pty_closed: false,
            ending,
            opening,
            record,
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
                        Control::Resize(size) => self.resize(size),
                        Control::Theme(theme) => self.theme(&theme),
                        Control::Redraw => {
                            if let Some(connection) = &self.current {
                                connection.outbox.redraw();
                            }
                        }
                        Control::Release => {
                            self.close_current(Closed::new(CloseReason::Released, "released"));
                        }
                        Control::Terminate => {
                            return self.finish(CloseReason::Terminated, None, "asked to end");
                        }
                    }
                }
                Event::InputQueueFull { generation, queued } => {
                    if self.current_generation() == Some(generation) {
                        self.log
                            .event("holder.input_queue_full", &[("queued", queued.into())]);
                        self.close_current(Closed::new(
                            CloseReason::InputQueueFull,
                            format!("more than {INPUT_QUEUE_CAP} input bytes waited"),
                        ));
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
                Event::Watch {
                    stream,
                    version,
                    sends,
                } => self.watch(stream, version, sends),
                Event::WatchGone { generation, error } => {
                    if self.watch.as_ref().map(|watch| watch.generation) == Some(generation) {
                        let closed = match error {
                            None => Closed::new(CloseReason::Released, "released"),
                            Some(error) => {
                                self.log.event(
                                    "holder.watch_lost",
                                    &[
                                        ("generation", generation.into()),
                                        ("reason", error.close_reason().name().into()),
                                        ("error", error.to_string().into()),
                                    ],
                                );
                                Closed::new(error.close_reason(), error.to_string())
                            }
                        };
                        self.close_watch(closed);
                    }
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
                Event::Notice(notice) => {
                    if let Some(connection) = &self.current
                        && connection.notices
                    {
                        connection.outbox.notice(notice);
                    }
                }
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
        // Before anything of the new connection is read: what the one it
        // replaces sends from here on is dropped.
        self.input.taken_over(generation);
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
        // In this order, so no change falls between them: the PTY reader
        // marks this outbox changed from here on, the first screen is asked
        // for whole, and only then can the writer take it.
        let screen = sends.contains(&Capability::Screen);
        if screen {
            self.publish(Some(outbox.clone()));
            outbox.redraw();
        }
        spawn_named("connection-writer", {
            let writer = ScreenWriter {
                generation,
                outbox: outbox.clone(),
                pane: screen.then(|| self.pane.clone()),
                epochs: self.epochs.clone(),
                events: self.sender.clone(),
                log: self.log.clone(),
                returned: Cell::new(false),
                fed: Cell::new(0),
            };
            let shutdown = stream.shutdown_handle();
            move || writer.serve(writer_stream, &shutdown, &written_tx)
        });
        spawn_named("connection-reader", {
            let sender = self.sender.clone();
            let input = self.input.clone();
            let outbox = outbox.clone();
            move || read_connection(stream, generation, &sender, &input, &outbox)
        });
        self.current = Some(Connection {
            generation,
            outbox,
            notices: sends.contains(&Capability::Notices),
            shutdown,
            written,
        });
        self.log
            .event("holder.connected", &[("generation", generation.into())]);
    }

    fn publish(&self, outbox: Option<Arc<Outbox>>) {
        *self.outbox.lock().unwrap_or_else(PoisonError::into_inner) = outbox;
    }

    /// The opened watch follows the pane from now on: the one it replaces is
    /// told it was taken over, and its reader's late rules change nothing.
    /// Its first facts are sent whatever they are, and its node's first rule
    /// set is read as soon as it arrives.
    /// How a watch reads whether the terminal edits lines under the
    /// foreground program it names: from the master on Unix; on Windows from
    /// a helper attached to the pane's console, which the holder never
    /// attaches to itself (`console_input.rs`).
    fn line_editing(&self, master: PtyMaster) -> LineEditingRead {
        #[cfg(unix)]
        {
            Box::new(move |_program| master.line_editing())
        }
        #[cfg(windows)]
        {
            let _ = master;
            let mut console =
                crate::console_input::ConsoleLines::new(self.program, self.log.clone());
            Box::new(move |program| console.read(program))
        }
    }

    fn watch(&mut self, stream: LocalStream, version: u16, sends: Vec<Capability>) {
        self.watch_generation += 1;
        let generation = self.watch_generation;
        if let Some(previous) = self.watch.take() {
            self.publish_watch(None);
            self.log.event(
                "holder.watch_replaced",
                &[
                    ("from", previous.generation.into()),
                    ("to", generation.into()),
                ],
            );
            previous.close(
                Closed::new(
                    CloseReason::TakenOver,
                    format!("watch {generation} took the pane's facts over"),
                ),
                FLUSH_LIMIT,
            );
        }
        self.rules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .owner = generation;
        let wake = Arc::new(Wake::default());
        let program_package = sends.contains(&Capability::ProgramPackage);
        let line_editing = sends.contains(&Capability::LineEditing);
        let foreground_unread = sends.contains(&Capability::ForegroundUnread);
        wake.send(HolderMessage::Watching(Watching {
            version,
            rules_engine: hide_screen_rules::ENGINE_VERSION,
            sends,
        }));
        let writer_stream = stream.duplicate();
        if writer_stream.set_write_timeout(Some(WRITE_LIMIT)).is_err() {
            self.log
                .event("holder.write_bound_unset", &[("watch", generation.into())]);
        }
        // In this order, so no change falls between them: the PTY reader
        // marks this watch changed from here on, and its first facts are
        // owed.
        self.publish_watch(Some(wake.clone()));
        wake.owe();
        let (written_tx, written) = mpsc::channel();
        let shutdown = stream.shutdown_handle();
        spawn_named("watch-writer", {
            let master = self.master.clone();
            let writer = FactsWriter {
                generation,
                wake: wake.clone(),
                pane: self.pane.clone(),
                rules: self.rules.clone(),
                shell: self.program,
                program_package,
                group: Box::new({
                    let master = master.clone();
                    let shell = self.program;
                    move || master.group(shell)
                }),
                foreground_unread,
                line_editing: line_editing.then(|| self.line_editing(master.clone())),
                log: self.log.clone(),
            };
            let shutdown = stream.shutdown_handle();
            move || writer.serve(writer_stream, &shutdown, &written_tx)
        });
        spawn_named("watch-reader", {
            let reader = WatchReader {
                generation,
                wake: wake.clone(),
                pane: self.pane.clone(),
                rules: self.rules.clone(),
                log: self.log.clone(),
            };
            let sender = self.sender.clone();
            move || {
                reader.serve(stream, |ended| {
                    let _ = sender.send(Event::WatchGone {
                        generation,
                        error: ended.err(),
                    });
                });
            }
        });
        self.watch = Some(WatchConnection {
            generation,
            wake,
            shutdown,
            written,
        });
        self.log
            .event("holder.watch_opened", &[("generation", generation.into())]);
    }

    fn publish_watch(&self, wake: Option<Arc<Wake>>) {
        *self
            .watch_wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = wake;
    }

    fn close_watch(&mut self, closed: Closed) {
        if let Some(watch) = self.watch.take() {
            self.publish_watch(None);
            self.log.event(
                "holder.watch_closed",
                &[
                    ("generation", watch.generation.into()),
                    ("reason", closed.reason.name().into()),
                ],
            );
            watch.close(closed, FLUSH_LIMIT);
        }
    }

    /// What the pane's facts read may have changed by something other than
    /// the program's output.
    fn facts_changed(&self) {
        if let Some(watch) = &self.watch {
            watch.wake.changed();
        }
    }

    fn close_current(&mut self, closed: Closed) {
        if let Some(connection) = self.current.take() {
            self.input.let_go();
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

    fn resize(&mut self, size: hide_holder_protocol::Size) {
        let size = match Size::checked(size.rows, size.cols) {
            Ok(size) => size,
            Err(why) => {
                self.close_current(Closed::new(CloseReason::SizeInvalid, why));
                return;
            }
        };
        let refused = self.master.lock().as_ref().and_then(|master| {
            master
                .resize(PtySize {
                    rows: size.rows,
                    cols: size.cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .err()
        });
        if let Some(error) = refused {
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
        self.facts_changed();
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
        // Whether an end it could not confirm makes it exit with a failure.
        let mut failed = false;
        let mut closed = match self.session.end(TREE_END_LIMIT) {
            Ok(SessionEnd::Ended) => Closed::new(reason, why),
            Ok(SessionEnd::Unconfirmed { left }) => {
                failed = true;
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
                failed = true;
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
            failed = true;
        }
        // Before the connection hears it, so a node told the pane ended
        // reads the same end in the record.
        self.write_end(&closed);
        if let Some(connection) = self.current.take() {
            self.input.let_go();
            self.publish(None);
            connection.close(closed.clone());
        }
        if let Some(watch) = self.watch.take() {
            self.publish_watch(None);
            watch.close(closed.clone(), FLUSH_LIMIT);
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
            if let Event::Opened { stream, .. } | Event::Watch { stream, .. } = event {
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
                ("failed", failed.into()),
            ],
        );
        if failed {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }

    /// Writes why the pane ended into its record. One it cannot write is
    /// logged with the end it meant to write, and the next node settles the
    /// pane as one whose holder was lost.
    fn write_end(&self, closed: &Closed) {
        let Some(record) = &self.record else { return };
        let reason = hide_holder_protocol::record::EndReason::of_close(closed.reason);
        let mut end = hide_holder_protocol::record::End::new(reason, closed.detail.clone());
        end.exit_code = closed.exit_code;
        end.left = closed.left.clone();
        if let Err(error) = record.end(end) {
            self.log.event(
                crate::record::UNWRITTEN_EVENT,
                &[
                    ("reason", reason.name().into()),
                    ("incarnation", record.incarnation().into()),
                    ("exit_code", json!(closed.exit_code)),
                    ("left", json!(closed.left)),
                    ("error", error.as_str().into()),
                ],
            );
        }
    }

    /// Closes the PTY on a thread of its own while the reader goes on taking
    /// its output until the end, since a console closing with output nobody
    /// reads can wait for ever; answers whether both finished in time.
    fn close_pty(&mut self) -> bool {
        let Some(master) = self.master.lock().take() else {
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
                Ok(Event::Opened { stream, .. } | Event::Watch { stream, .. }) => {
                    self.turn_away(&stream, deadline);
                }
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

/// Hands the program's answers to the PTY writer. Past their bound the
/// answers of one parse (one read of the PTY, a resize or a theme) are
/// refused whole, never in part, and every parse after it is too until the
/// program has taken what waits: the run's first refusal and its end are
/// each logged with the reason and told to the connection that holds the
/// pane when it accepts notices.
#[derive(Clone)]
struct Answers {
    writes: Sender<PtyWrite>,
    queued: Arc<AtomicUsize>,
    /// The answers and bytes refused in this run of refusals, while one
    /// runs.
    refused: Arc<Mutex<Option<(u64, u64)>>>,
    events: Sender<Event>,
    log: Log,
}

/// `queued` and the `newer` notice of its reason as one: their counts and
/// bytes added, said in the newer one's words, a resumption's from its sum.
fn fold(queued: Notice, newer: Notice) -> Notice {
    let count = queued.count.saturating_add(newer.count);
    let bytes = queued.bytes.saturating_add(newer.bytes);
    let detail = match newer.reason {
        NoticeReason::AnswersResumed => resumed_detail(count, bytes),
        _ => newer.detail,
    };
    Notice::new(newer.reason, count, bytes, detail)
}

fn resumed_detail(count: u64, bytes: u64) -> String {
    format!("{count} answers ({bytes} bytes) were refused while the program did not read them")
}

impl Answers {
    fn send(&self, answers: Vec<u8>) {
        if answers.is_empty() {
            return;
        }
        let length = answers.len();
        let mut refused = self.refused.lock().unwrap_or_else(PoisonError::into_inner);
        let queued = self.queued.load(Ordering::SeqCst);
        if queued + length > ANSWER_QUEUE_CAP {
            match refused.as_mut() {
                Some((count, bytes)) => {
                    *count += 1;
                    *bytes += length as u64;
                }
                None => {
                    *refused = Some((1, length as u64));
                    self.log.event(
                        "holder.answers_refused",
                        &[
                            ("reason", "answer_queue_full".into()),
                            ("bytes", length.into()),
                            ("queued", queued.into()),
                            ("cap", ANSWER_QUEUE_CAP.into()),
                        ],
                    );
                    self.notify(Notice::new(
                        NoticeReason::AnswersRefused,
                        1,
                        length as u64,
                        format!(
                            "the program's answers waited past {ANSWER_QUEUE_CAP} bytes ({queued} queued); answers are refused until it reads them"
                        ),
                    ));
                }
            }
            return;
        }
        if let Some((count, bytes)) = refused.take() {
            self.log.event(
                "holder.answers_resumed",
                &[("refused", count.into()), ("refused_bytes", bytes.into())],
            );
            self.notify(Notice::new(
                NoticeReason::AnswersResumed,
                count,
                bytes,
                resumed_detail(count, bytes),
            ));
        }
        drop(refused);
        self.queued.fetch_add(length, Ordering::SeqCst);
        if self.writes.send(PtyWrite::Answers(answers)).is_err() {
            self.queued.fetch_sub(length, Ordering::SeqCst);
        }
    }

    /// Hands `notice` to the holder's loop, which gives it to the connection
    /// that holds the pane if that one accepts notices; with no loop left,
    /// the holder is ending and the log has it.
    fn notify(&self, notice: Notice) {
        let _ = self.events.send(Event::Notice(notice));
    }
}

/// The writers the PTY reader marks changed: the screen of the connection
/// that holds the pane, and the watch's facts.
struct Marks {
    outbox: Arc<Mutex<Option<Arc<Outbox>>>>,
    watch: Arc<Mutex<Option<Arc<Wake>>>>,
}

impl Marks {
    fn changed(&self) {
        let outbox = self
            .outbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(outbox) = outbox {
            outbox.changed();
        }
        let watch = self
            .watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(watch) = watch {
            watch.changed();
        }
    }
}

/// The PTY's controlling side: the loop resizes and closes it, and an ask
/// and a watch's writer read the terminal's foreground group from it.
#[derive(Clone)]
struct PtyMaster(Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>);

impl PtyMaster {
    fn lock(&self) -> MutexGuard<'_, Option<Box<dyn MasterPty + Send>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the terminal edits lines, as the platform reads the master
    /// this holder keeps (`hide_platform::process::master_line_input`):
    /// unknown when the terminal could not be asked, its pane having ended.
    #[cfg(unix)]
    fn line_editing(&self) -> LineEditing {
        use hide_platform::process::{LineInput, master_line_input};
        let guard = self.lock();
        let read = guard
            .as_ref()
            .and_then(|master| master.as_raw_fd())
            // SAFETY: the master stays open while its lock is held, for the
            // whole of the call.
            .map(|fd| master_line_input(unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) }));
        match read {
            Some(Ok(Some(LineInput::Lines { .. }))) => LineEditing::Edits,
            Some(Ok(Some(LineInput::Keys))) => LineEditing::ReadsKeys,
            _ => LineEditing::Unknown,
        }
    }

    /// The terminal's foreground process group, or why it is not known. On
    /// Windows, which names none for a console, the process that stands in
    /// for it under `shell`, the pane's program
    /// (`hide_platform::process::console_foreground`).
    fn group(&self, shell: u32) -> Result<u32, Unknown> {
        #[cfg(unix)]
        {
            let _ = shell;
            let group = self
                .lock()
                .as_ref()
                .and_then(|master| master.process_group_leader());
            match group {
                Some(group) if group > 0 => Ok(group as u32),
                _ => Err(Unknown::NoGroup),
            }
        }
        #[cfg(windows)]
        {
            let _ = self;
            hide_platform::process::console_foreground(shell).map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    Unknown::NoGroup
                } else {
                    Unknown::Unreadable
                }
            })
        }
    }
}

/// Reads the PTY until it ends, whether or not a node is connected, so the
/// program never waits on a reader and its questions are answered.
fn read_pty(
    mut reader: Box<dyn Read + Send>,
    pane: &Mutex<Pane>,
    marks: &Marks,
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
        marks.changed();
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
    input: &InputPath,
    answers_queued: &AtomicUsize,
    events: &Sender<Event>,
    panic_on_input: bool,
    hold_writes: Option<&std::path::Path>,
) {
    if let Some(release) = hold_writes {
        while !release.exists() {
            thread::park_timeout(Duration::from_millis(10));
        }
    }
    let mut refused = None;
    // Input that came while a submission waited for its gap, in order.
    let mut waiting: VecDeque<PtyWrite> = VecDeque::new();
    loop {
        let write = match waiting.pop_front() {
            Some(write) => write,
            None => match writes.recv() {
                Ok(write) => write,
                Err(_) => return,
            },
        };
        let length = write.len();
        match write {
            PtyWrite::Input { generation, bytes } => {
                assert!(
                    !panic_on_input,
                    "asked to panic on input ({PANIC_ON_INPUT})"
                );
                write_input(&mut *writer, events, generation, &bytes, &mut refused);
                input.queued.fetch_sub(length, Ordering::SeqCst);
            }
            PtyWrite::Submit {
                generation,
                text,
                gap,
                enter,
            } => {
                // Herdr's `agent.prompt`: the gap runs from the text's
                // flush, the program's answers go on meanwhile, and the
                // input after it waits until the submit was written.
                write_input(&mut *writer, events, generation, &text, &mut refused);
                // Read before the gap's start, so the submit is written no
                // sooner than a gap after this.
                let text_at = std::time::SystemTime::now();
                let due = Instant::now() + gap;
                while refused != Some(generation) {
                    let left = due.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    match writes.recv_timeout(left) {
                        Ok(PtyWrite::Answers(bytes)) => {
                            let _ = writer.write_all(&bytes).and_then(|()| writer.flush());
                            answers_queued.fetch_sub(bytes.len(), Ordering::SeqCst);
                        }
                        Ok(other) => waiting.push_back(other),
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                write_input(&mut *writer, events, generation, &enter, &mut refused);
                input.queued.fetch_sub(length, Ordering::SeqCst);
                // One line a submission, never one a key.
                input.log.event(
                    "holder.submitted",
                    &[
                        ("generation", generation.into()),
                        (
                            "text_at_unix_ms",
                            text_at
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |since| since.as_millis() as u64)
                                .into(),
                        ),
                        ("gap_ms", (gap.as_millis() as u64).into()),
                        ("written", (refused != Some(generation)).into()),
                    ],
                );
            }
            PtyWrite::Answers(bytes) => {
                // A PTY that refuses answers refuses everything; the pane's
                // end follows, and nothing waits for them.
                let _ = writer.write_all(&bytes).and_then(|()| writer.flush());
                answers_queued.fetch_sub(length, Ordering::SeqCst);
            }
        }
    }
}

/// Writes a connection's input unless the PTY already refused that
/// connection's, and says so when it refuses now.
fn write_input(
    writer: &mut (dyn std::io::Write + Send),
    events: &Sender<Event>,
    generation: u64,
    bytes: &[u8],
    refused: &mut Option<u64>,
) {
    if *refused != Some(generation)
        && let Err(error) = writer.write_all(bytes).and_then(|()| writer.flush())
    {
        *refused = Some(generation);
        let _ = events.send(Event::InputFailed {
            generation,
            error: error.to_string(),
        });
    }
}

impl PtyWrite {
    /// What it counts against its queue's cap.
    fn len(&self) -> usize {
        match self {
            Self::Input { bytes, .. } | Self::Answers(bytes) => bytes.len(),
            Self::Submit { text, enter, .. } => (text.len() + enter.len()).max(SUBMIT_COST),
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
    log: Log,
    /// Whether this connection's writer has given freed pages back yet, and
    /// the pane's output fed (`Pane::fed`) when it last did: which returns
    /// are logged (`LOGGED_RETURN_BYTES`).
    returned: Cell<bool>,
    fed: Cell<u64>,
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
        // Whether a fragment went out since the freed pages were last given
        // back: a flood leaves the allocator holding what it freed.
        let mut drew = false;
        loop {
            let job = self.next_job(next, held, &mut drew);
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
                    drew = true;
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
    /// time it may be drawn again. Once the screen has stayed still for
    /// `RETURN_FREED_AFTER` after a fragment (`drew`), it gives the freed
    /// pages back and waits with no bound; nothing wakes an idle writer.
    fn next_job(&self, next: Instant, held: Option<(u64, Instant)>, drew: &mut bool) -> Job {
        let mut pending = self.outbox.pending();
        loop {
            if let Some(message) = pending.messages.pop_front() {
                return Job::Send(message);
            }
            if pending.changed {
                let now = Instant::now();
                let (at, waits_for_change) = pending.draw_at(next, held, now);
                if now >= at {
                    pending.changed = false;
                    pending.waits_for_change = false;
                    if pending.answers_input() {
                        pending.input = None;
                    }
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
            if *drew {
                let (guard, waited) = self
                    .outbox
                    .wake
                    .wait_timeout(pending, RETURN_FREED_AFTER)
                    .unwrap_or_else(PoisonError::into_inner);
                pending = guard;
                if waited.timed_out() && !pending.changed && pending.messages.is_empty() {
                    *drew = false;
                    drop(pending);
                    self.return_freed();
                    pending = self.outbox.pending();
                }
                continue;
            }
            pending = self
                .outbox
                .wake
                .wait(pending)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn return_freed(&self) {
        match hide_platform::memory::return_freed() {
            Ok(()) => {
                let fed = self.pane.as_ref().map_or(0, |pane| {
                    pane.lock().unwrap_or_else(PoisonError::into_inner).fed()
                });
                let output = fed.saturating_sub(self.fed.replace(fed));
                if !self.returned.replace(true) || output >= LOGGED_RETURN_BYTES {
                    self.log.event(
                        "holder.memory_returned",
                        &[
                            ("generation", self.generation.into()),
                            ("output_bytes", output.into()),
                        ],
                    );
                }
            }
            // A C library with no such call (musl, which no build of this
            // repository targets) keeps what it freed.
            Err(error) if error.kind() == std::io::ErrorKind::Unsupported => {}
            Err(error) => self.log.event(
                "holder.memory_return_failed",
                &[("error", error.to_string().into())],
            ),
        }
    }
}

/// Reads a connection until it ends: the program's input goes to the PTY
/// writer from here, everything else to the run thread.
fn read_connection(
    mut stream: LocalStream,
    generation: u64,
    events: &Sender<Event>,
    input: &InputPath,
    outbox: &Outbox,
) {
    loop {
        match read_client(&mut stream) {
            Ok(message) => {
                let control = match message {
                    ClientMessage::Resize(size) => Control::Resize(size),
                    ClientMessage::Theme(theme) => Control::Theme(theme),
                    ClientMessage::Redraw => Control::Redraw,
                    ClientMessage::Release => Control::Release,
                    ClientMessage::Terminate => Control::Terminate,
                    input_message @ (ClientMessage::Input(_)
                    | ClientMessage::Paste(_)
                    | ClientMessage::Mouse(_)
                    | ClientMessage::Scroll(_)
                    | ClientMessage::Key(_)
                    | ClientMessage::Submit(_)) => {
                        match input.take(generation, outbox, input_message) {
                            Taken::Next => continue,
                            Taken::Full => return,
                        }
                    }
                };
                let ends = matches!(control, Control::Release | Control::Terminate);
                if events
                    .send(Event::Message {
                        generation,
                        message: control,
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
    /// The screen of the connection that holds the pane, which a scroll ask
    /// marks changed.
    outbox: Arc<Mutex<Option<Arc<Outbox>>>>,
    shell: u32,
    master: PtyMaster,
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
                outbox: self.outbox.clone(),
                shell: self.shell,
                master: self.master.clone(),
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
    outbox: Arc<Mutex<Option<Arc<Outbox>>>>,
    shell: u32,
    master: PtyMaster,
}

impl Asked {
    /// Marks the holding connection's screen changed, when a connection
    /// holds the pane.
    fn screen_changed(&self) {
        let outbox = self
            .outbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(outbox) = outbox {
            outbox.changed();
        }
    }

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
            Ok(FirstMessage::Watch(hello)) => return self.watch(stream, &hello),
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

    /// Hands an accepted watch to the holder's loop, which replaces any watch
    /// before it; a watch that does not take facts has nothing to follow.
    fn watch(&self, stream: LocalStream, hello: &Hello) {
        let log = &self.log;
        let (version, sends) = match agree(hello, &WATCH_SENDS) {
            Ok(agreed) => agreed,
            Err(closed) => {
                refuse(&stream, closed, Instant::now() + HELLO_LIMIT, log);
                return;
            }
        };
        if !sends.contains(&Capability::Facts) {
            let closed = Closed::new(CloseReason::HelloInvalid, "a watch takes facts");
            refuse(&stream, closed, Instant::now() + HELLO_LIMIT, log);
            return;
        }
        if let Some(closed) = self.ended() {
            refuse(&stream, closed, Instant::now() + HELLO_LIMIT, log);
            return;
        }
        let opened = Event::Watch {
            stream,
            version,
            sends,
        };
        if let Err(mpsc::SendError(Event::Watch { stream, .. })) = self.sender.send(opened) {
            let closed = self
                .ended()
                .unwrap_or_else(|| Closed::new(CloseReason::HolderFailed, "the holder has ended"));
            refuse(&stream, closed, Instant::now() + HELLO_LIMIT, log);
        }
    }

    /// Answers one ask and closes; the connection that holds the pane, and
    /// the screen's scroll position, stay as they were (D-11, D-18), except
    /// that a scroll-to moves the screen and sends the holding connection
    /// the moved screen.
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
            Ask::Screen => self
                .pane
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .read_screen()
                .map_err(|error| format!("the screen could not be read: {error}")),
            Ask::Rows { lines } => self
                .pane
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .read_rows(lines)
                .map_err(|error| format!("the screen could not be read: {error}")),
            Ask::Visible => self
                .pane
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .read_visible()
                .map_err(|error| format!("the screen could not be read: {error}")),
            Ask::Scroll => self
                .pane
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .scroll_position()
                .map_err(|error| format!("the scroll position could not be read: {error}")),
            Ask::ScrollTo { offset_from_bottom } => {
                let moved = self
                    .pane
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .scroll_to(offset_from_bottom);
                match moved {
                    Ok((answer, moved)) => {
                        if moved {
                            self.screen_changed();
                        }
                        Ok(answer)
                    }
                    Err(error) => Err(format!("the screen could not be scrolled: {error}")),
                }
            }
            Ask::Processes => Ok(crate::ask::processes(
                self.shell,
                self.master.group(self.shell),
                &self.log,
            )),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn notices(outbox: &Outbox) -> Vec<Notice> {
        outbox
            .pending()
            .messages
            .iter()
            .map(|message| match message {
                HolderMessage::Notice(notice) => notice.clone(),
                other => panic!("not a notice: {other:?}"),
            })
            .collect()
    }

    /// A writer that does not keep up while answers are refused and resumed
    /// a thousand times holds one notice of each reason, the counts added
    /// and the newest reason last.
    #[test]
    fn refusals_and_resumptions_a_writer_has_not_sent_fold_into_one_of_each() {
        let outbox = Outbox::default();
        for _ in 0..1000 {
            outbox.notice(Notice::new(NoticeReason::AnswersRefused, 1, 10, "refused"));
            outbox.notice(Notice::new(NoticeReason::AnswersResumed, 3, 30, "resumed"));
        }
        let held = notices(&outbox);
        assert_eq!(held.len(), 2, "{held:?}");
        assert_eq!(held[0].reason, NoticeReason::AnswersRefused);
        assert_eq!((held[0].count, held[0].bytes), (1000, 10_000));
        assert_eq!(held[1].reason, NoticeReason::AnswersResumed);
        assert_eq!((held[1].count, held[1].bytes), (3000, 30_000));
        assert!(
            held[1].detail.starts_with("3000 answers (30000 bytes)"),
            "{held:?}"
        );

        // A refusal after them goes last: the program refuses answers now.
        outbox.notice(Notice::new(NoticeReason::AnswersRefused, 1, 10, "refused"));
        let held = notices(&outbox);
        assert_eq!(held.len(), 2, "{held:?}");
        assert_eq!(held[1].reason, NoticeReason::AnswersRefused);
        assert_eq!(held[1].count, 1001);
    }

    /// The program's answer to input is drawn at once, however recently the
    /// last fragment went out; output before the input, or with none, waits
    /// out the interval, and synchronized output still holds an answer.
    #[test]
    fn a_change_after_input_is_drawn_at_once_and_other_output_is_paced() {
        let now = Instant::now();
        let next = now + Duration::from_secs(3600);
        let outbox = Outbox::default();

        outbox.changed();
        assert_eq!(outbox.pending().draw_at(next, None, now), (next, false));

        outbox.input();
        assert_eq!(
            outbox.pending().draw_at(next, None, now),
            (next, false),
            "the change before the input does not answer it"
        );
        outbox.changed();
        assert_eq!(outbox.pending().draw_at(next, None, now), (now, false));

        let pending = outbox.pending();
        let until = now + Duration::from_secs(1);
        assert_eq!(
            pending.draw_at(next, Some((pending.version, until)), now),
            (until, true)
        );
    }
}
