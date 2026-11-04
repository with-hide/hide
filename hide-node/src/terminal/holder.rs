//! A node's panes held by pane holders (PRD engine-bundle-2-pane-holder
//! D-10, D-11, D-15, D-17): starting and ending them, attaching their screens
//! through the terminal service, and, when the node starts, finding again
//! the holders that ran on while it was gone.
//!
//! [`Holders`] owns the state folder's pane records (`hide_kit::panes`) and
//! [`PaneAttacher`] opens a pane it knows by them over its holder's socket. A holder pane's screen reaches the [`super::OutputSink`] as the
//! fragments the holder draws, each delivered as a frame, and its input goes
//! out as bytes, resizes and wheels; a holder encodes those for its
//! program's own settings.
//!
//! The holder's connection is the one that holds its screen and input; a
//! second node that says hello takes it over, and this node's session ends
//! `taken_over`, which is never tried again on its own (B9). A holder that
//! speaks a version older than [`OLDEST_HOLDER_VERSION`] is reported
//! `unsupported_holder_version` and left running (B10). A device's node
//! runs holders when its core asks (`Call::HoldersStart`, PRD
//! engine-bundle-3-core-owns D-22); a Windows device's node runs inside the
//! job its SSH session made, which a holder leaves through `start_resident`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
#[cfg(test)]
use std::thread;
use std::time::{Duration, Instant};

use hide_holder_protocol::record::{EndReason, PaneRecord};
use hide_holder_protocol::{
    Answer, Ask, Capability, ClientMessage, CloseReason, Closed, HolderMessage, Key, MAX_INPUT,
    MAX_KEYS, MAX_PASTE, Modifiers, OLDEST_HOLDER_VERSION, OpenError, PROTOCOL_VERSION, Scroll,
    Size, read_holder, write_client,
};
use hide_kit::holders::{FOLDERS_CAP, Pruned, Source};
use hide_kit::panes::{
    CloseError, Judgment, Panes, RECONCILE_BUDGET, StartRefusal, StartRequest, Started, core_pane,
    holder_pane,
};
use hide_node_link::holders::{HolderIntent, HolderRecovery, HolderReservation, RecoverablePane};
use hide_node_link::terminal::{
    GridSize, MAX_ATTACHED_PANES, PaneTerminalState, ReportSink, TerminalControl, TerminalNode,
    TerminalReport,
};
use hide_platform::ipc::LocalStream;
use serde_json::json;

use super::Service;
use super::protocol::SessionEvent;
use super::session::{Attacher, Cleanup, OpenFailure, SessionEvents, SessionWriter, Transport};

mod watch;

use watch::Watches;
pub(super) use watch::too_old;

/// How long opening a holder's connection may take: its connect and its
/// hello's answer, a screen's or a watch's (`watch.rs`).
const OPEN_LIMIT: Duration = Duration::from_secs(5);

/// How long one question to a holder may take.
const ASK_LIMIT: Duration = Duration::from_secs(5);

/// How many reconciles a node's start runs before it reports what is left
/// for the next start. Each takes up panes for [`RECONCILE_BUDGET`], so this
/// bounds a start's reconcile to about nine minutes however many panes it
/// finds; one reconcile takes up every pane of a state folder unless their
/// holders are slow to end.
const RECONCILE_ROUNDS: usize = 8;

/// The panes this node opens through their holders, by pane id: those it
/// started or adopted and has not ended, and those whose holder it is
/// starting. Looked up under the service's lock, where it does nothing but
/// look up a name; only an open, on its own worker, waits on it.
#[derive(Default)]
struct Known {
    panes: Mutex<BTreeMap<String, Holder>>,
    /// Set before the core's own engine can publish any pane. An unseen
    /// pane then belongs to a holder whose start has not been registered
    /// yet, including a saved pane waiting for record recovery.
    owns_every_pane: AtomicBool,
    /// Told whenever a start under way ends, either way.
    settled: Condvar,
}

/// A pane's holder as the node knows it.
enum Holder {
    /// Being started: its socket may not listen yet.
    Starting,
    Running(PathBuf),
}

impl Known {
    /// The socket of `pane`'s holder, waiting until `deadline` for a start
    /// under way: a pane is drawn, and its view attaches, while its holder
    /// starts (the core's own engine answers a new tab or split first), and
    /// that attach is the holder's to answer. `None` for a pane no holder
    /// runs or starts before the core's own engine owns every pane; once it
    /// does, the node waits for even an unseen pane, since the layout may be
    /// drawn before its start is registered. A deadline is a holder failure.
    fn socket_once_started(
        &self,
        pane: &str,
        deadline: Instant,
    ) -> Result<Option<PathBuf>, String> {
        let mut panes = self.lock();
        loop {
            match panes.get(pane) {
                Some(Holder::Running(socket)) => return Ok(Some(socket.clone())),
                None if !self.owns_every_pane.load(Ordering::Acquire) => return Ok(None),
                None => {}
                Some(Holder::Starting) => {}
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| {
                    format!("Pane {pane}'s holder did not become ready before the start deadline")
                })?;
            panes = self
                .settled
                .wait_timeout(panes, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Whether `pane` is a holder pane: one that runs, or one being started.
    fn contains(&self, pane: &str) -> bool {
        self.owns_every_pane.load(Ordering::Acquire) || self.lock().contains_key(pane)
    }

    /// A start of `pane`'s holder begins; a pane that already runs keeps
    /// its socket, since a start asked again answers it.
    fn starting(&self, pane: &str) {
        self.lock()
            .entry(pane.to_owned())
            .or_insert(Holder::Starting);
    }

    /// A start of `pane` that left nothing running ends: an open waiting on
    /// it goes on without it.
    fn not_started(&self, pane: &str) {
        let mut panes = self.lock();
        if matches!(panes.get(pane), Some(Holder::Starting)) {
            panes.remove(pane);
        }
        drop(panes);
        self.settled.notify_all();
    }

    fn insert(&self, pane: &str, socket: PathBuf) {
        self.lock().insert(pane.to_owned(), Holder::Running(socket));
        self.settled.notify_all();
    }

    fn remove(&self, pane: &str) {
        self.lock().remove(pane);
        self.settled.notify_all();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Holder>> {
        self.panes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Who is told when a holder pane's program exits on its own.
#[derive(Default)]
struct Ends(Mutex<Option<hide_node_link::holders::PaneEnded>>);

impl Ends {
    fn tell(&self, pane: &str) {
        let watch = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(watch) = watch {
            watch(pane);
        }
    }
}

/// Opens a pane its node knows by its holder's record over the holder's
/// socket; a pane no holder runs has no session to open.
pub struct PaneAttacher {
    known: Arc<Known>,
    ends: Arc<Ends>,
}

impl Attacher for PaneAttacher {
    fn open_transport(
        &self,
        pane_id: &str,
        size: Option<GridSize>,
    ) -> Result<Transport, OpenFailure> {
        let deadline = Instant::now() + hide_kit::panes::START_LIMIT;
        match self
            .known
            .socket_once_started(pane_id, deadline)
            .map_err(OpenFailure::retried)?
        {
            Some(socket) => open_watched(pane_id, &socket, size, Some(Arc::clone(&self.ends))),
            None => Err(OpenFailure::retried(format!(
                "Pane {pane_id} has no holder on this node"
            ))),
        }
    }

    fn keeps_own_size(&self, pane_id: &str) -> bool {
        self.known.contains(pane_id)
    }
}

/// Opens the holder of `pane` at `socket` as a control session: says hello,
/// accepting its screen and its notices, and resizes it to `size` when that
/// is not the size it has. A holder holds one connection at a time, so this
/// takes the pane over from whoever held it.
pub fn open_holder(
    pane: &str,
    socket: &Path,
    size: Option<GridSize>,
) -> Result<Transport, OpenFailure> {
    open_watched(pane, socket, size, None)
}

/// [`open_holder`], telling `ends` when the pane's program exits.
fn open_watched(
    pane: &str,
    socket: &Path,
    size: Option<GridSize>,
    ends: Option<Arc<Ends>>,
) -> Result<Transport, OpenFailure> {
    let (stream, welcome) = hide_holder_protocol::open(
        socket,
        vec![
            Capability::Screen,
            Capability::Notices,
            Capability::Keys,
            Capability::Submit,
        ],
        Instant::now() + OPEN_LIMIT,
    )
    .map_err(open_failure)?;
    let reads_keys = welcome.sends.contains(&Capability::Keys);
    let submissions = if welcome.sends.contains(&Capability::Submit) {
        super::session::Submissions::Whole
    } else {
        super::session::Submissions::Refused
    };
    let found = GridSize {
        rows: welcome.size.rows,
        cols: welcome.size.cols,
    };
    let mut writer = HolderWriter {
        stream: stream.duplicate(),
        size: found,
    };
    if let Some(size) = size {
        writer.resize(size).map_err(|error| {
            OpenFailure::retried(format!("the holder could not be resized: {error}"))
        })?;
    }
    let shutdown = stream.shutdown_handle();
    Ok(Transport {
        events: Box::new(HolderEvents {
            pane: pane.to_owned(),
            stream,
            screen: None,
            ends,
        }),
        writer: Box::new(writer),
        reads_keys,
        submissions,
        // After the release has been written (or a second passed), a
        // reader still waiting on a holder that never answered it is let go.
        cleanup: Cleanup::Other(Box::new(move || shutdown.shutdown())),
        // The size it has now: the one asked for, else the one it had.
        size: Some(size.unwrap_or(found)),
    })
}

fn open_failure(error: OpenError) -> OpenFailure {
    match error {
        OpenError::Unsupported { version } => OpenFailure {
            category: "unsupported_holder_version",
            message: format!(
                "The pane's holder speaks version {version}, older than the oldest this build speaks to ({OLDEST_HOLDER_VERSION}); it is left running"
            ),
            retried: false,
        },
        // Only a refusal that says attaching again cannot help is final, as
        // for a close during the session (`super::final_close`).
        OpenError::Refused(closed) => OpenFailure {
            category: category(closed.reason),
            message: message(&closed),
            retried: super::final_close(category(closed.reason)).is_none(),
        },
        error => OpenFailure {
            category: "holder_unreachable",
            message: error.to_string(),
            retried: true,
        },
    }
}

/// The `exit_category` a holder's close reason names. Those the service
/// does not try again are listed in `super::final_close`.
pub fn category(reason: CloseReason) -> &'static str {
    match reason {
        CloseReason::UnsupportedVersion => "unsupported_holder_version",
        other => other.name(),
    }
}

fn message(closed: &Closed) -> String {
    match closed.reason {
        CloseReason::ProgramExited => match closed.exit_code {
            Some(code) => format!("The pane's program exited with code {code}"),
            None => "The pane's program exited".to_owned(),
        },
        CloseReason::Terminated => "The pane was ended".to_owned(),
        CloseReason::TakenOver => "Another node took this pane over".to_owned(),
        CloseReason::UnsupportedVersion => format!(
            "The pane's holder speaks version {}, which this build does not speak to; it is left running",
            closed.version
        ),
        reason => format!("The pane's holder closed the connection: {}", reason.name()),
    }
}

/// A holder connection's messages as session events: each fragment is a
/// frame at the size it was drawn, and its close is the session's end with
/// the reason the holder named. A notice, something the holder refused while
/// the pane runs on, goes to the diagnostic log and the session goes on;
/// the operator can do nothing about it, so nothing reaches the screen.
struct HolderEvents {
    pane: String,
    stream: LocalStream,
    /// The epoch and sequence of the last fragment.
    screen: Option<(u64, u64)>,
    /// Told when the pane's program exits on its own.
    ends: Option<Arc<Ends>>,
}

impl SessionEvents for HolderEvents {
    fn next_event(&mut self) -> SessionEvent {
        loop {
            if let Some(event) = self.next_message() {
                return event;
            }
        }
    }
}

impl HolderEvents {
    /// The session event the next message is, or `None` for a notice.
    fn next_message(&mut self) -> Option<SessionEvent> {
        Some(match read_holder(&mut self.stream) {
            Ok(HolderMessage::Notice(notice)) => {
                crate::diagnostic!(json!({
                    "component": "pane_holder",
                    "kind": format!("pane_holder.{}", notice.reason.name()),
                    "pane_id": self.pane,
                    "reason_code": notice.reason.code(),
                    "count": notice.count,
                    "bytes": notice.bytes,
                    "message": notice.detail,
                }));
                return None;
            }
            Ok(HolderMessage::Fragment(fragment)) => {
                // A full fragment opens an epoch at 0; every other one is the
                // next of its epoch. One out of order means the screen this
                // node draws is no longer the holder's.
                let follows = if fragment.full {
                    fragment.seq == 0 && self.screen.is_none_or(|(epoch, _)| fragment.epoch > epoch)
                } else {
                    self.screen == Some((fragment.epoch, fragment.seq.wrapping_sub(1)))
                };
                if !follows {
                    return Some(SessionEvent::Ended {
                        category: "protocol_error",
                        message: format!(
                            "The pane's holder sent fragment {}.{} after {:?}",
                            fragment.epoch, fragment.seq, self.screen
                        ),
                    });
                }
                self.screen = Some((fragment.epoch, fragment.seq));
                SessionEvent::Frame {
                    width: fragment.size.cols,
                    height: fragment.size.rows,
                    full: fragment.full,
                    bytes: fragment.bytes,
                }
            }
            Ok(HolderMessage::Closed(closed)) => {
                if closed.reason == CloseReason::ProgramExited
                    && let Some(ends) = &self.ends
                {
                    ends.tell(&self.pane);
                }
                SessionEvent::Ended {
                    category: category(closed.reason),
                    message: message(&closed),
                }
            }
            Ok(
                HolderMessage::Welcome(_)
                | HolderMessage::Answer(_)
                | HolderMessage::Watching(_)
                | HolderMessage::Facts(_),
            ) => SessionEvent::Ended {
                category: "protocol_error",
                message: "The pane's holder sent a welcome, an answer or facts on its screen"
                    .to_owned(),
            },
            Err(error) => SessionEvent::Ended {
                category: "transport_eof",
                message: format!("The pane's holder connection ended: {error}"),
            },
        })
    }
}

/// A holder connection's input side.
struct HolderWriter {
    stream: LocalStream,
    /// The size the holder has: a resize to it is not sent, since the holder
    /// resizes its PTY, and signals its program, on every one it is sent.
    size: GridSize,
}

impl SessionWriter for HolderWriter {
    fn submit(&mut self, submit: &hide_holder_protocol::Submit) -> std::io::Result<()> {
        write_client(&mut self.stream, &ClientMessage::Submit(submit.clone()))
    }

    fn input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        for chunk in bytes.chunks(MAX_INPUT) {
            write_client(&mut self.stream, &ClientMessage::Input(chunk.to_vec()))?;
        }
        Ok(())
    }

    /// A paste past one message's bound goes as several, each wrapped for
    /// the program's paste mode on its own.
    fn paste(&mut self, text: &[u8]) -> std::io::Result<()> {
        for chunk in text.chunks(MAX_PASTE) {
            write_client(&mut self.stream, &ClientMessage::Paste(chunk.to_vec()))?;
        }
        Ok(())
    }

    fn keys(&mut self, keys: &[Key]) -> std::io::Result<()> {
        for chunk in keys.chunks(MAX_KEYS) {
            write_client(&mut self.stream, &ClientMessage::Key(chunk.to_vec()))?;
        }
        Ok(())
    }

    fn resize(&mut self, size: GridSize) -> std::io::Result<()> {
        if size == self.size {
            return Ok(());
        }
        let checked = Size::checked(size.rows, size.cols).map_err(std::io::Error::other)?;
        write_client(&mut self.stream, &ClientMessage::Resize(checked))?;
        self.size = size;
        Ok(())
    }

    /// A holder draws its whole screen again when asked; a size it does not
    /// have yet brings a full screen by the resize.
    fn redraw(&mut self, size: GridSize) -> std::io::Result<()> {
        if size != self.size {
            return self.resize(size);
        }
        write_client(&mut self.stream, &ClientMessage::Redraw)
    }

    fn scroll(
        &mut self,
        lines: i32,
        column: Option<u16>,
        row: Option<u16>,
        modifiers: u8,
    ) -> std::io::Result<()> {
        let lines = i16::try_from(lines).unwrap_or(if lines > 0 { i16::MAX } else { i16::MIN });
        write_client(
            &mut self.stream,
            &ClientMessage::Scroll(Scroll {
                lines,
                cell: column.zip(row),
                // The screen's bits (shift 1, ctrl 2, alt 4, meta 8) are the
                // protocol's.
                modifiers: Modifiers(modifiers),
            }),
        )
    }

    fn release(&mut self) -> std::io::Result<()> {
        write_client(&mut self.stream, &ClientMessage::Release)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::Write::flush(&mut self.stream)
    }
}

/// What a node's start found of its holders and did about them.
#[derive(Debug, Default)]
pub struct Recovered {
    /// Panes whose holders run, attached to the service, in name order.
    pub attached: Vec<String>,
    /// Panes whose holders run, past [`MAX_ATTACHED_PANES`]: the service
    /// reports them `attach_limit`; they run on, and their screens are kept
    /// by their holders.
    pub kept: Vec<String>,
    /// Panes that ended, with the reason their records name.
    pub ended: Vec<(String, Option<EndReason>)>,
    /// Panes whose holders speak a version too old to speak to, left
    /// running.
    pub unsupported: Vec<String>,
    /// Panes and holders that could not be judged, with why; nothing was
    /// done to them.
    pub unjudged: Vec<(String, String)>,
    /// Whether everything found was taken up; when not, the next start
    /// takes up the rest.
    pub complete: bool,
}

/// A state folder's holder panes, as a node runs them.
pub struct Holders {
    panes: Panes,
    source: Source,
    home: PathBuf,
    state_dir: PathBuf,
    known: Arc<Known>,
    ends: Arc<Ends>,
    /// Each holder pane's facts, on a watch of its own (`watch`).
    watches: Arc<Watches>,
    reports: Arc<dyn ReportSink>,
    /// What the last [`Holders::recover`] found, for the core's restore to
    /// wait on ([`Holders::recovered`]).
    recovery: (Mutex<Option<HolderRecovery>>, Condvar),
}

/// What [`Holders::start_local`] starts: the node's terminal service and its
/// holder panes.
pub struct LocalTerminals {
    pub service: Arc<Service>,
    pub holders: Arc<Holders>,
}

/// Where a node's holder panes live: the home whose version folders the
/// holders are copied into, and the state folder that keeps their records.
#[derive(Clone, Copy, Debug)]
pub struct HolderFolders<'a> {
    pub home: &'a Path,
    pub state_dir: &'a Path,
}

impl Holders {
    /// One connection's ownership, retained by its adapter and admitted
    /// workers. Acquisition and crash repair happen off every runtime lock.
    pub fn acquire(&self, within: Duration) -> Result<hide_kit::panes::Owner, String> {
        self.panes.acquire(within)
    }
    /// These holders are the node's only pane engine. Tell the shared
    /// attach owner before the engine exposes a saved or newly drawn pane;
    /// recovery and start registration may both happen after its attach.
    pub(super) fn own_every_pane(&self) {
        self.known.owns_every_pane.store(true, Ordering::Release);
    }

    /// The holder panes of `folders.state_dir`, whose holders are copied
    /// from `source` into version folders under `folders.home`.
    pub fn open(
        folders: HolderFolders<'_>,
        source: Source,
        reports: Arc<dyn ReportSink>,
    ) -> Result<Self, String> {
        let panes = Panes::open(folders.home, folders.state_dir, crate::diagnostics::emit)?;
        Ok(Self {
            panes,
            source,
            home: folders.home.to_owned(),
            state_dir: folders.state_dir.to_owned(),
            known: Arc::default(),
            ends: Arc::default(),
            watches: Arc::default(),
            reports,
            recovery: (Mutex::new(None), Condvar::new()),
        })
    }

    /// The local node's holder program: the one beside this program, and on
    /// Windows the pinned ConPTY runtime in the folder beside it the app
    /// package puts it in ([`hide_kit::holders::CONPTY_SOURCE`],
    /// `scripts/fetch-conpty.py`).
    pub fn local_source() -> Result<Source, String> {
        let program = std::env::current_exe()
            .map_err(|error| format!("this program could not be found: {error}"))?;
        let dir = program
            .parent()
            .ok_or_else(|| format!("{} has no folder", program.display()))?;
        Ok(Source {
            program: dir.join(hide_kit::holders::program_name()),
            conpty: cfg!(windows).then(|| dir.join(hide_kit::holders::CONPTY_SOURCE)),
        })
    }

    /// The local node's terminal service as a daemon starts it: the state
    /// folder's holder panes, the holder from
    /// [`Holders::local_source`], and the holders that ran on while no node
    /// was there found again, and the version folders nothing runs from
    /// removed when its connection has acquired pane ownership. hided and
    /// the tools that measure a node (`hide-node/examples/holder_node.rs`)
    /// start it here, so what they measure is how the daemon is built.
    pub fn start_local(
        folders: HolderFolders<'_>,
        outputs: Arc<dyn super::OutputSink>,
        reports: Arc<dyn ReportSink>,
    ) -> Result<LocalTerminals, String> {
        Self::start_from(folders, Self::local_source()?, outputs, reports)
    }

    /// [`Holders::start_local`] with the holder program `source` names, for
    /// a test that starts a node as the daemon does with the holder it built.
    pub fn start_from(
        folders: HolderFolders<'_>,
        source: Source,
        outputs: Arc<dyn super::OutputSink>,
        reports: Arc<dyn ReportSink>,
    ) -> Result<LocalTerminals, String> {
        let holders = Arc::new(Self::open(folders, source, Arc::clone(&reports))?);
        let service = Arc::new(
            Service::start(
                holders.attacher(),
                outputs,
                reports,
                super::RetryPolicy::Automatic,
            )
            .map_err(|error| format!("terminal service failed to start: {error}"))?,
        );
        Ok(LocalTerminals { service, holders })
    }

    /// The attacher the node's terminal service opens panes with: each
    /// pane over its holder's socket.
    pub fn attacher(&self) -> Box<dyn Attacher> {
        Box::new(PaneAttacher {
            known: Arc::clone(&self.known),
            ends: Arc::clone(&self.ends),
        })
    }

    /// Starts `request`'s pane and attaches its screen to `service`, at the
    /// size the core named for it, else the size it was started at until a
    /// view draws it. The pane id is the start's key: asked again
    /// for a pane that lives, it answers that pane and starts no second
    /// holder. A pane past [`MAX_ATTACHED_PANES`] runs and is reported
    /// `attach_limit` by the service, as any pane over the cap is. The
    /// service and the reports name the pane by `request.pane`, the holder's
    /// files by [`holder_pane`] of it.
    pub fn start(
        &self,
        owner: &hide_kit::panes::Owner,
        service: &Service,
        request: &StartRequest,
    ) -> Result<Started, StartRefusal> {
        let holder = holder_pane(&request.pane);
        self.known.starting(&request.pane);
        let started = self.panes.start(
            owner,
            &StartRequest {
                pane: holder.clone(),
                ..request.clone()
            },
            &self.source,
        );
        self.attach_started(service, &request.pane, started)
    }

    pub fn reserve(
        &self,
        guard: &hide_kit::panes::Request<'_>,
        intent: &HolderIntent,
        previous: Option<u64>,
    ) -> Result<HolderReservation, StartRefusal> {
        self.panes
            .reserve(
                guard,
                &holder_pane(&intent.pane),
                &intent.digest,
                &intent.cwd,
                previous,
            )
            .map(|record| HolderReservation {
                incarnation: record.incarnation,
            })
    }

    pub fn start_reserved(
        &self,
        guard: &hide_kit::panes::Request<'_>,
        service: &Service,
        intent: &HolderIntent,
        reserved: HolderReservation,
        request: impl FnOnce() -> Result<StartRequest, StartRefusal>,
    ) -> Result<Started, StartRefusal> {
        self.known.starting(&intent.pane);
        let holder = holder_pane(&intent.pane);
        let result = self.panes.start_reserved(
            guard,
            &holder,
            reserved.incarnation,
            &intent.digest,
            &self.source,
            || {
                let mut request = request()?;
                request.pane = holder.clone();
                Ok(request)
            },
        );
        let attached = self.attach_started(service, &intent.pane, result)?;
        // Cancellation can finish while this worker is attaching an earlier
        // successful start. Observe storage after enqueueing Attach: either
        // this forget follows it, or the later cancel enqueues its own forget.
        // The admitted request guard stays alive through this cleanup.
        match self.panes.record(&holder) {
            Ok(Some(current))
                if current.incarnation == reserved.incarnation && !current.cancelled() =>
            {
                Ok(attached)
            }
            Ok(Some(current)) => {
                self.forget_screen(service, &intent.pane);
                Err(StartRefusal::Conflict(Box::new(current)))
            }
            other => {
                self.forget_screen(service, &intent.pane);
                Err(StartRefusal::Uncertain(format!(
                    "the attached reservation could not be confirmed: {other:?}"
                )))
            }
        }
    }

    pub fn cancel(
        &self,
        guard: &hide_kit::panes::Request<'_>,
        service: &Service,
        intent: &HolderIntent,
    ) -> Result<(), StartRefusal> {
        self.panes.cancel_reservation(
            guard,
            &holder_pane(&intent.pane),
            &intent.digest,
            &intent.cwd,
        )?;
        self.forget_screen(service, &intent.pane);
        Ok(())
    }

    fn forget_screen(&self, service: &Service, pane: &str) {
        service.control(TerminalControl::Closing {
            pane: pane.to_owned(),
            closing: true,
        });
        self.known.remove(pane);
        self.watches.forget(pane);
        service.control(TerminalControl::Forget {
            pane: pane.to_owned(),
        });
    }

    fn attach_started(
        &self,
        service: &Service,
        pane: &str,
        started: Result<Started, StartRefusal>,
    ) -> Result<Started, StartRefusal> {
        let started = match started {
            Ok(started) => started,
            Err(refusal) => {
                self.known.not_started(pane);
                return Err(refusal);
            }
        };
        let (Started::Started(record) | Started::Existing(record)) = &started;
        if record.end.is_some() {
            self.known.not_started(pane);
            self.report_ended(pane, record);
            return Ok(started);
        }
        let socket = hide_kit::layout::holder_socket(&self.state_dir, &holder_pane(pane));
        self.known.insert(pane, socket.clone());
        self.watches.watch(pane, socket);
        // No size: the start's is the holder's own, not a decision, and it
        // must not replace a size the core already named for the pane (a
        // daemon that starts again names the one it saved).
        service.control(TerminalControl::Attach {
            pane: pane.to_owned(),
            size: None,
            manual: false,
        });
        Ok(started)
    }

    /// Ends the pane: its program and everything it started, then its
    /// holder, within `hide_kit::panes::CLOSE_LIMIT`, and answers its record.
    /// The service's session of it ends as a close it asked for; the node
    /// forgets the pane once it ended.
    pub fn terminate(
        &self,
        owner: &hide_kit::panes::Owner,
        service: &Service,
        pane: &str,
    ) -> Result<PaneRecord, CloseError> {
        service.control(TerminalControl::Closing {
            pane: pane.to_owned(),
            closing: true,
        });
        let closed = self.panes.close(owner, &holder_pane(pane));
        if closed.is_ok() || matches!(closed, Err(CloseError::NoRecord)) {
            self.forget_screen(service, pane);
        } else {
            service.control(TerminalControl::Closing {
                pane: pane.to_owned(),
                closing: false,
            });
        }
        closed
    }

    /// The pane's last `lines` rows, on a connection of its own that leaves
    /// the screen connection and its scroll position as they are. A holder
    /// answers at most [`hide_holder_protocol::MAX_READ_LINES`] rows, so a
    /// longer ask (a pane search reads up to 10,000) reads that many, marked
    /// truncated when older rows exist, instead of being refused.
    pub fn read(&self, pane: &str, lines: u16) -> Result<Answer, OpenError> {
        let lines = lines.min(hide_holder_protocol::MAX_READ_LINES);
        self.ask(pane, Ask::Read { lines })
    }

    /// The rows the pane's screen shows now, or its last `lines` rows, with
    /// their styling as escape sequences, as [`Holders::read`] asks.
    pub fn read_styled(&self, pane: &str, rows: Option<u16>) -> Result<Answer, OpenError> {
        self.ask(
            pane,
            match rows {
                None => Ask::Screen,
                Some(lines) => Ask::Rows {
                    lines: lines.min(hide_holder_protocol::MAX_READ_LINES),
                },
            },
        )
    }

    /// The rows the pane's screen shows now, as text, as [`Holders::read`]
    /// asks.
    pub fn read_visible(&self, pane: &str) -> Result<Answer, OpenError> {
        self.ask(pane, Ask::Visible)
    }

    /// Where the pane's screen is in its scrollback, after moving it to `to`
    /// rows above the bottom when asked; the holder sends the connection
    /// that holds the screen the moved screen.
    pub fn scroll(&self, pane: &str, to: Option<u64>) -> Result<Answer, OpenError> {
        self.ask(
            pane,
            match to {
                None => Ask::Scroll,
                Some(offset_from_bottom) => Ask::ScrollTo { offset_from_bottom },
            },
        )
    }

    /// The pane's shell and the processes in its terminal's foreground, as
    /// [`Holders::read`] asks, by `within` when that is sooner than an ask's
    /// own bound.
    pub fn processes(&self, pane: &str, within: Duration) -> Result<Answer, OpenError> {
        self.ask_by(pane, Ask::Processes, Instant::now() + within.min(ASK_LIMIT))
    }

    /// The pane's record, read under no lock: a reader sees one whole
    /// record, as each change replaces it atomically.
    pub fn record(&self, pane: &str) -> Result<Option<PaneRecord>, String> {
        self.panes
            .record(&holder_pane(pane))
            .map_err(|why| why.to_string())
    }

    /// Asks in the version the pane's record says its holder speaks, so a
    /// newer node still reads an older holder.
    fn ask(&self, pane: &str, ask: Ask) -> Result<Answer, OpenError> {
        self.ask_by(pane, ask, Instant::now() + ASK_LIMIT)
    }

    /// [`Self::ask`], answered by `deadline`.
    fn ask_by(&self, pane: &str, ask: Ask, deadline: Instant) -> Result<Answer, OpenError> {
        let holder = holder_pane(pane);
        let record = self
            .panes
            .record(&holder)
            .map_err(|why| OpenError::Io(std::io::Error::other(why.to_string())))?
            .ok_or_else(|| {
                OpenError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("pane {pane} has no record"),
                ))
            })?;
        hide_holder_protocol::ask(
            &hide_kit::layout::holder_socket(&self.state_dir, &holder),
            ask,
            record.protocol,
            deadline,
        )
    }

    /// What a node does when it starts: reconciles the records, and the
    /// holders they name, until everything found is taken up (or
    /// [`RECONCILE_ROUNDS`] have run), asks the service to attach every holder
    /// that runs, by name, at the size each has, and reports each pane that
    /// ended while no node was there with the reason its record names. The
    /// service attaches the first [`MAX_ATTACHED_PANES`] and reports the rest
    /// `attach_limit`, as it does a start past its cap; they run on, their
    /// screens held by their holders. Each pane is reported once, by what the
    /// last round that took it up found.
    pub fn recover(&self, owner: &hide_kit::panes::Owner, service: &Service) -> Recovered {
        let mut recovered = Recovered::default();
        let mut judged: BTreeMap<String, Judgment> = BTreeMap::new();
        let mut failed = None;
        for round in 1..=RECONCILE_ROUNDS {
            let reconciled = self.panes.reconcile(owner, RECONCILE_BUDGET);
            let pending = reconciled.pending();
            recovered.complete = reconciled.is_complete();
            for (pane, judgment) in reconciled.panes {
                let waits = matches!(judgment, Judgment::Pending(_) | Judgment::Starting(_));
                // A pane a round took up keeps what it was found; one left
                // pending takes what a later round finds.
                let settled = judged.get(&pane).is_some_and(|earlier| {
                    !matches!(earlier, Judgment::Pending(_) | Judgment::Starting(_))
                });
                if !settled && !(waits && judged.contains_key(&pane)) {
                    judged.insert(pane, judgment);
                }
            }
            failed = reconciled.failed;
            if recovered.complete {
                break;
            }
            crate::diagnostic!(json!({
                "component": "pane_holder",
                "kind": "pane_holder.reconcile_pending",
                "round": round,
                "pending": pending,
            }));
        }
        let mut alive = Vec::new();
        let mut waiting = Vec::new();
        let mut recoverable = Vec::new();
        // A record is named by its holder id; the service, the reports and
        // the core name the pane by its own id.
        for (holder, judgment) in judged {
            let pane = core_pane(&holder);
            let interrupted = match &judgment {
                Judgment::Recoverable(record) => Some(record),
                Judgment::Ended(record) if !finished(record.end.as_ref().map(|end| end.reason)) => {
                    Some(record)
                }
                _ => None,
            };
            if let Some(record) = interrupted {
                recoverable.push(RecoverablePane {
                    pane: pane.clone(),
                    cwd: record.reservation.as_ref().map(|intent| intent.cwd.clone()),
                    incarnation: record.incarnation,
                });
            }
            match judgment {
                Judgment::Recoverable(_) => recovered.ended.push((pane, None)),
                Judgment::Alive(_) => alive.push((pane, holder)),
                Judgment::Ended(record) => {
                    self.report_ended(&pane, &record);
                    recovered
                        .ended
                        .push((pane, record.end.as_ref().map(|end| end.reason)));
                }
                Judgment::Unsupported(record) => {
                    self.report(
                        &pane,
                        "unsupported",
                        "unsupported_holder_version",
                        format!(
                            "The pane's holder speaks version {}, older than the oldest this build speaks to ({OLDEST_HOLDER_VERSION}); it is left running",
                            record.protocol
                        ),
                    );
                    self.watches.unsupported(&pane, None, record.protocol);
                    recovered.unsupported.push(pane);
                }
                Judgment::Pending(_) | Judgment::Starting(_) => waiting.push(pane),
                other => recovered.unjudged.push((pane, format!("{other:?}"))),
            }
        }
        if let Some(failed) = failed {
            recovered.unjudged.push((String::new(), failed));
        }
        let present = alive
            .iter()
            .map(|(pane, _)| pane.clone())
            .chain(recovered.unsupported.iter().cloned())
            .chain(waiting.iter().cloned())
            .chain(
                recovered
                    .unjudged
                    .iter()
                    .map(|(pane, _)| pane.clone())
                    .filter(|pane| !pane.is_empty()),
            )
            .collect();
        if !waiting.is_empty() {
            crate::diagnostic!(json!({
                "component": "pane_holder",
                "kind": "pane_holder.reconcile_unfinished",
                "rounds": RECONCILE_ROUNDS,
                "panes": waiting,
            }));
        }
        for (pane, holder) in alive {
            let socket = hide_kit::layout::holder_socket(&self.state_dir, &holder);
            self.known.insert(&pane, socket.clone());
            // Every pane that runs is followed, attached or kept past the
            // attach cap alike.
            self.watches.watch(&pane, socket);
            // No size: a holder keeps its own, and the screen that comes back
            // is the one it holds at this moment.
            service.control(TerminalControl::Attach {
                pane: pane.clone(),
                size: None,
                manual: false,
            });
            if recovered.attached.len() < MAX_ATTACHED_PANES {
                recovered.attached.push(pane);
            } else {
                recovered.kept.push(pane);
            }
        }
        crate::diagnostic!(json!({
            "component": "pane_holder",
            "kind": "pane_holder.recovered",
            "attached": recovered.attached.len(),
            "kept_unattached": recovered.kept.len(),
            "ended": recovered.ended.len(),
            "unsupported": recovered.unsupported.len(),
            "unjudged": recovered.unjudged.len(),
            "complete": recovered.complete,
        }));
        let (found, settled) = &self.recovery;
        let (closed, ended) = recovered
            .ended
            .iter()
            .partition::<Vec<_>, _>(|(_, reason)| finished(*reason));
        let panes = |ended: Vec<&(String, Option<EndReason>)>| {
            ended.into_iter().map(|(pane, _)| pane.clone()).collect()
        };
        *found.lock().unwrap_or_else(PoisonError::into_inner) = Some(HolderRecovery {
            owner: owner.incarnation(),
            closed: panes(closed),
            ended: panes(ended),
            present,
            recoverable,
        });
        settled.notify_all();
        recovered
    }

    /// What the last [`Holders::recover`] found, waiting up to `timeout` for
    /// the first one to finish; `None` when none finished by then.
    pub fn recovered(&self, timeout: Duration) -> Option<HolderRecovery> {
        let (found, settled) = &self.recovery;
        let found = found.lock().unwrap_or_else(PoisonError::into_inner);
        let (found, _) = settled
            .wait_timeout_while(found, timeout, |found| found.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        found.clone()
    }

    /// Tells `ended` the id of each pane whose program exits on its own
    /// while this node holds its screen, from now on.
    pub fn watch_ends(&self, ended: hide_node_link::holders::PaneEnded) {
        *self.ends.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(ended);
    }

    /// Tells `sink` each holder pane's facts from now on, and at once the
    /// last facts of every pane heard from before it.
    pub fn watch_facts(&self, sink: hide_node_link::holders::PaneFactsSink) {
        self.watches.set_sink(sink);
    }

    /// Keeps `rules` for `pane` and gives its holder the first set its
    /// evaluator reads, now when its watch is open and again on each watch
    /// opened later.
    pub fn set_rules(
        &self,
        pane: &str,
        rules: Option<hide_node_link::holders::ScreenRules>,
    ) -> Result<(), hide_node_link::holders::HolderError> {
        self.watches.set_rules(pane, rules)
    }

    /// How many of the state folder's panes may still run
    /// (`hide_kit::panes::Panes::live`): a daemon of the core's own engine
    /// lives while there is one (PRD engine-bundle-3-core-owns D-10).
    pub fn live_panes(&self) -> Result<usize, String> {
        self.panes.live()
    }

    /// Removes the version folders under the home that are not this
    /// build's and that nothing runs from (D-16, B20): a folder a holder or
    /// an OpenConsole still runs from stays, one a start has reserved is
    /// waited for, and the runtime copies live under another root
    /// (`hide_kit::holders::prune`). What could not be removed, and more
    /// than [`FOLDERS_CAP`] folders left, go to the diagnostic log; `None`
    /// when this build's own folder could not be named, and nothing is
    /// removed then.
    pub fn prune_folders(&self) -> Option<Pruned> {
        let current = match hide_kit::holders::version_name(PROTOCOL_VERSION, &self.source) {
            Ok(name) => name,
            Err(error) => {
                crate::diagnostic!(json!({
                    "component": "pane_holder",
                    "kind": "pane_holder.folders_not_pruned",
                    "message": format!("the holder's files could not be read: {error}"),
                }));
                return None;
            }
        };
        let pruned = hide_kit::holders::prune(&self.home, &[&current]);
        if pruned.busy {
            crate::diagnostic!(json!({
                "component": "pane_holder",
                "kind": "pane_holder.folders_busy",
                "message": "starts held the version folders for the whole wait; the next start prunes them",
            }));
        }
        for (folder, detail) in &pruned.failed {
            crate::diagnostic!(json!({
                "component": "pane_holder",
                "kind": "pane_holder.folder_remove_failed",
                "folder": folder,
                "message": detail,
            }));
        }
        if let Some(count) = pruned.over_cap {
            crate::diagnostic!(json!({
                "component": "pane_holder",
                "kind": "pane_holder.folders_over_cap",
                "count": count,
                "cap": FOLDERS_CAP,
            }));
        }
        crate::diagnostic!(json!({
            "component": "pane_holder",
            "kind": "pane_holder.folders_pruned",
            "removed": pruned.removed,
            "in_use": pruned.in_use,
            "failed": pruned.failed.len(),
        }));
        Some(pruned)
    }

    fn report_ended(&self, pane: &str, record: &PaneRecord) {
        let (category, message) = match &record.end {
            Some(end) => (
                end.reason.name(),
                match end.exit_code {
                    Some(code) => format!("The pane ended ({}, code {code})", end.reason.name()),
                    None => format!("The pane ended ({})", end.reason.name()),
                },
            ),
            None => ("ended", "The pane ended".to_owned()),
        };
        self.report(pane, "ended", category, message);
    }

    fn report(&self, pane: &str, state: &str, category: &str, message: String) {
        crate::diagnostic!(json!({
            "component": "pane_holder",
            "kind": "pane_holder.pane_reported",
            "pane_id": pane,
            "state": state,
            "exit_category": category,
        }));
        self.reports.report(TerminalReport::State {
            pane: pane.to_owned(),
            state: PaneTerminalState {
                state: state.to_owned(),
                generation: 0,
                attempt: 0,
                message: Some(message),
                exit_category: Some(category.to_owned()),
                retry_decision: "none".to_owned(),
                last_attempt_at_unix_ms: None,
            },
        });
    }
}

/// Whether a pane that ended with `reason` is finished, so it closes rather
/// than taking a new shell: its program exited or a node ended it, its
/// leftovers ended or not. A pane whose holder ended under it (the machine
/// restarted, the holder was lost or failed) or whose start never finished
/// is not, nor one whose record names no reason.
fn finished(reason: Option<EndReason>) -> bool {
    matches!(
        reason,
        Some(EndReason::ProgramExited | EndReason::Terminated | EndReason::TreeEndUnconfirmed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core's own engine draws a new tab or split before its holder
    /// starts, and the view attaches at once: that attach waits for the
    /// start and opens the holder. A pane whose start fails, or that no
    /// holder runs, is refused at once.
    #[test]
    fn an_attach_to_a_pane_whose_holder_is_starting_waits_for_it() {
        let known = Arc::new(Known::default());
        let attacher = PaneAttacher {
            known: Arc::clone(&known),
            ends: Arc::default(),
        };
        let size = Some(GridSize { rows: 24, cols: 80 });
        let missing = tempfile::tempdir().unwrap();

        known.starting("w1:p2");
        assert!(
            attacher.keeps_own_size("w1:p2"),
            "a starting pane is a holder pane"
        );
        // While the start runs the open waits, out to its deadline.
        let began = Instant::now();
        let wait = Duration::from_millis(30);
        assert!(known.socket_once_started("w1:p2", began + wait).is_err());
        assert!(
            began.elapsed() >= wait,
            "the lookup did not wait for the start"
        );
        // A start that ends wakes the open, which takes the holder's socket.
        let socket = missing.path().join("w1_p2.sock");
        thread::scope(|scope| {
            let waiting = scope.spawn(|| {
                known.socket_once_started("w1:p2", Instant::now() + Duration::from_secs(10))
            });
            known.insert("w1:p2", socket.clone());
            assert_eq!(waiting.join().unwrap().unwrap(), Some(socket.clone()));
        });
        // No holder listens there, so the open itself fails, at the holder.
        assert!(attacher.open_transport("w1:p2", size).is_err());

        known.starting("w1:p3");
        thread::scope(|scope| {
            let open = scope.spawn(|| attacher.open_transport("w1:p3", size));
            known.not_started("w1:p3");
            assert!(open.join().unwrap().is_err());
        });
        assert!(!attacher.keeps_own_size("w1:p3"));

        let refused = attacher
            .open_transport("w1:p9", size)
            .err()
            .expect("a pane no holder runs is refused");
        assert!(refused.retried, "{}", refused.message);
    }

    #[test]
    fn an_unregistered_pane_of_the_only_engine_fails_at_its_deadline() {
        let known = Known::default();
        assert_eq!(
            known.socket_once_started("w1:p1", Instant::now()).unwrap(),
            None
        );
        known.owns_every_pane.store(true, Ordering::Release);
        assert!(known.contains("w1:p1"));
        assert!(known.socket_once_started("w1:p1", Instant::now()).is_err());
        assert!(
            known.lock().is_empty(),
            "a waiting attach retains no pane entry"
        );
    }

    #[test]
    fn a_pane_its_program_or_a_node_ended_is_finished_and_one_its_holder_lost_is_not() {
        for (reason, closes) in [
            (Some(EndReason::ProgramExited), true),
            (Some(EndReason::Terminated), true),
            (Some(EndReason::TreeEndUnconfirmed), true),
            (Some(EndReason::HolderFailed), false),
            (Some(EndReason::HolderLost), false),
            (Some(EndReason::MachineRestarted), false),
            (Some(EndReason::StartFailed), false),
            (Some(EndReason::StartUnconfirmed), false),
            (Some(EndReason::StartInterrupted), false),
            (None, false),
        ] {
            assert_eq!(finished(reason), closes, "{reason:?}");
        }
    }

    #[test]
    fn holder_closes_that_attaching_again_cannot_mend_are_never_retried() {
        for (reason, state) in [
            (CloseReason::ProgramExited, Some("ended")),
            (CloseReason::Terminated, Some("ended")),
            (CloseReason::TakenOver, Some("taken_over")),
            (CloseReason::UnsupportedVersion, Some("unsupported")),
            (CloseReason::ProtocolError, None),
            (CloseReason::HolderFailed, None),
            (CloseReason::InputQueueFull, None),
            (CloseReason::Released, None),
        ] {
            assert_eq!(
                super::super::final_close(category(reason)),
                state,
                "{reason:?}"
            );
        }
    }
}
