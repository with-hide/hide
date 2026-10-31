//! A node's panes held by pane holders (PRD engine-bundle-2-pane-holder
//! D-10, D-11, D-15, D-17): starting and ending them, attaching their screens
//! through the terminal service beside the node's Herdr sessions, and, when
//! the node starts, finding again the holders that ran on while it was gone.
//!
//! Nothing here runs unless the development switch [`SWITCH`] is on: with it
//! off a node opens Herdr sessions only, as it always has, and starts,
//! adopts and reads no holder (B1). With it on, [`Holders`] owns the state
//! folder's pane records (`hide_kit::panes`) and [`PaneAttacher`] opens a
//! pane it knows by them over its holder's socket and every other pane over
//! Herdr. A holder pane's screen reaches the [`super::OutputSink`] as the
//! fragments the holder draws, each delivered as the frame a Herdr session
//! delivers, and its input goes out as the bytes, resizes and wheels a Herdr
//! session takes; a holder encodes those for its program's own settings.
//!
//! The holder's connection is the one that holds its screen and input; a
//! second node that says hello takes it over, and this node's session ends
//! `taken_over`, which is never tried again on its own (B9). A holder that
//! speaks a version older than [`OLDEST_HOLDER_VERSION`] is reported
//! `unsupported_holder_version` and left running (B10). Only the local node
//! runs holders in this bundle: a device's node runs under a job its
//! `OwnedChild` start made, which a holder cannot break away from on
//! Windows, and devices carry no holder yet ([`device_switch`]).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use hide_holder_protocol::record::{EndReason, PaneRecord};
use hide_holder_protocol::{
    Answer, Ask, Capability, ClientMessage, CloseReason, Closed, HolderMessage, MAX_INPUT,
    Modifiers, OLDEST_HOLDER_VERSION, OpenError, Scroll, Size, read_holder, write_client,
};
use hide_kit::holders::Source;
use hide_kit::panes::{
    CloseError, Judgment, OrphanOutcome, Panes, RECONCILE_BUDGET, StartRefusal, StartRequest,
    Started,
};
use hide_node_link::terminal::{
    GridSize, MAX_ATTACHED_PANES, PaneTerminalState, ReportSink, TerminalControl, TerminalNode,
    TerminalReport,
};
use hide_platform::ipc::LocalStream;
use serde_json::json;

use super::Service;
use super::protocol::{Mode, SessionEvent};
use super::session::{
    Attacher, Cleanup, OpenFailure, SessionEvents, SessionParts, SessionWriter, Transport,
};

/// The development switch that lets a node run pane holders.
///
/// - Requirement: optional; it is read once, when a node's terminal service
///   starts.
/// - Shape: unset, `0` (off) or `1` (on). Any other value fails the node's
///   start with the values it takes, rather than being read as off.
/// - Fallback: off.
/// - When it is missing: the node opens every pane through its Herdr's
///   terminal sessions and starts, adopts and reads no pane holder; the app
///   and the daemon behave as they always have. Nobody but a developer of
///   the next engine sets it: the app shows no holder pane in this bundle.
pub const SWITCH: &str = "HIDE_DEV_PANE_HOLDER";

/// Whether [`SWITCH`] is on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Switch {
    Off,
    On,
}

impl Switch {
    /// The switch as this process's environment sets it.
    pub fn from_env() -> Result<Self, String> {
        Self::parse(std::env::var_os(SWITCH).as_deref())
    }

    /// The switch a value of [`SWITCH`] sets.
    pub fn parse(value: Option<&OsStr>) -> Result<Self, String> {
        match value.map(|value| value.to_str()) {
            None => Ok(Self::Off),
            Some(Some("0")) => Ok(Self::Off),
            Some(Some("1")) => Ok(Self::On),
            Some(value) => Err(format!(
                "{SWITCH} is {}; it takes 1 (run pane holders) or 0 (do not), or is left unset",
                value.map_or_else(|| "not text".to_owned(), |value| format!("{value:?}"))
            )),
        }
    }
}

/// A device's node never runs holders in this bundle. With the switch on
/// (or set to a value it does not take) it says so in the diagnostic log
/// and opens its panes through Herdr: a device's node runs inside the job
/// object its `OwnedChild` start made, which on Windows allows no breakaway,
/// so a holder could not outlive it, and a device carries no holder until
/// its kit does.
pub fn device_switch() {
    let switch = Switch::from_env();
    if switch != Ok(Switch::Off) {
        crate::diagnostic!(json!({
            "component": "pane_holder",
            "kind": "pane_holder.device_node_unsupported",
            "switch": switch.as_ref().map_or_else(|why| why.clone(), |_| "on".to_owned()),
            "message": "a device's node does not run pane holders in this bundle; its panes open through Herdr",
        }));
    }
}

/// How long opening a holder's connection may take: its connect and its
/// hello's answer.
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
/// started or adopted and has not ended. Read under the service's lock, so
/// it does nothing but look up a name.
#[derive(Default)]
struct Known(Mutex<BTreeMap<String, PathBuf>>);

impl Known {
    fn socket(&self, pane: &str) -> Option<PathBuf> {
        self.lock().get(pane).cloned()
    }

    fn insert(&self, pane: &str, socket: PathBuf) {
        self.lock().insert(pane.to_owned(), socket);
    }

    fn remove(&self, pane: &str) {
        self.lock().remove(pane);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, PathBuf>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Opens a pane its node knows by its holder's record over the holder's
/// socket, and every other pane through `herdr`.
pub struct PaneAttacher {
    herdr: Box<dyn Attacher>,
    known: Arc<Known>,
}

impl Attacher for PaneAttacher {
    fn open(
        &self,
        pane_id: &str,
        mode: Mode,
        rows: u16,
        cols: u16,
    ) -> Result<SessionParts, String> {
        self.herdr.open(pane_id, mode, rows, cols)
    }

    fn open_transport(
        &self,
        pane_id: &str,
        mode: Mode,
        size: Option<GridSize>,
    ) -> Result<Transport, OpenFailure> {
        match self.known.socket(pane_id) {
            Some(socket) => open_holder(&socket, size),
            None => self.herdr.open_transport(pane_id, mode, size),
        }
    }

    fn keeps_own_size(&self, pane_id: &str) -> bool {
        self.known.socket(pane_id).is_some() || self.herdr.keeps_own_size(pane_id)
    }
}

/// Opens the holder at `socket` as a control session: says hello, accepting
/// its screen, and resizes it to `size` when that is not the size it has.
/// A holder holds one connection at a time, so this takes the pane over
/// from whoever held it.
pub fn open_holder(socket: &Path, size: Option<GridSize>) -> Result<Transport, OpenFailure> {
    let (stream, welcome) = hide_holder_protocol::open(
        socket,
        vec![Capability::Screen],
        Instant::now() + OPEN_LIMIT,
    )
    .map_err(open_failure)?;
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
            stream,
            screen: None,
        }),
        writer: Some(Box::new(writer)),
        kept: None,
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
        OpenError::Refused(closed) => OpenFailure {
            category: category(closed.reason),
            message: message(&closed),
            retried: closed.reason == CloseReason::OpeningFull,
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
/// the reason the holder named.
struct HolderEvents {
    stream: LocalStream,
    /// The epoch and sequence of the last fragment.
    screen: Option<(u64, u64)>,
}

impl SessionEvents for HolderEvents {
    fn next_event(&mut self) -> SessionEvent {
        match read_holder(&mut self.stream) {
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
                    return SessionEvent::Ended {
                        category: "protocol_error",
                        message: format!(
                            "The pane's holder sent fragment {}.{} after {:?}",
                            fragment.epoch, fragment.seq, self.screen
                        ),
                    };
                }
                self.screen = Some((fragment.epoch, fragment.seq));
                SessionEvent::Frame {
                    width: fragment.size.cols,
                    height: fragment.size.rows,
                    full: fragment.full,
                    bytes: fragment.bytes,
                }
            }
            Ok(HolderMessage::Closed(closed)) => SessionEvent::Ended {
                category: category(closed.reason),
                message: message(&closed),
            },
            Ok(HolderMessage::Welcome(_) | HolderMessage::Answer(_)) => SessionEvent::Ended {
                category: "protocol_error",
                message: "The pane's holder sent a welcome or an answer on its screen".to_owned(),
            },
            Err(error) => SessionEvent::Ended {
                category: "transport_eof",
                message: format!("The pane's holder connection ended: {error}"),
            },
        }
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
    fn input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        for chunk in bytes.chunks(MAX_INPUT) {
            write_client(&mut self.stream, &ClientMessage::Input(chunk.to_vec()))?;
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
    /// Panes whose holders run, left unattached past [`MAX_ATTACHED_PANES`]:
    /// they run on, and their screens are kept by their holders.
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

/// A state folder's holder panes, as a node with [`SWITCH`] on runs them.
pub struct Holders {
    panes: Panes,
    source: Source,
    state_dir: PathBuf,
    known: Arc<Known>,
    reports: Arc<dyn ReportSink>,
}

impl Holders {
    /// The holder panes of `state_dir`, whose holders are copied from
    /// `source` into version folders under `home`, when `switch` is on;
    /// `None` when it is off, and then nothing of holders is opened.
    pub fn open(
        switch: Switch,
        home: &Path,
        state_dir: &Path,
        source: Source,
        reports: Arc<dyn ReportSink>,
    ) -> Result<Option<Self>, String> {
        if switch == Switch::Off {
            return Ok(None);
        }
        let panes = Panes::open(home, state_dir, crate::diagnostics::emit)?;
        Ok(Some(Self {
            panes,
            source,
            state_dir: state_dir.to_owned(),
            known: Arc::default(),
            reports,
        }))
    }

    /// The local node's holder program: the one beside this program, and on
    /// Windows the pinned ConPTY runtime beside the node's Herdr.
    pub fn local_source(herdr_bin: Option<&Path>) -> Result<Source, String> {
        let program = std::env::current_exe()
            .map_err(|error| format!("this program could not be found: {error}"))?;
        let dir = program
            .parent()
            .ok_or_else(|| format!("{} has no folder", program.display()))?;
        let conpty = if cfg!(windows) {
            let herdr = herdr_bin.ok_or(
                "the pinned ConPTY runtime is found beside Herdr, and no Herdr is configured",
            )?;
            Some(
                herdr
                    .parent()
                    .ok_or_else(|| format!("{} has no folder", herdr.display()))?
                    .join(hide_kit::holders::CONPTY),
            )
        } else {
            None
        };
        Ok(Source {
            program: dir.join(hide_kit::holders::program_name()),
            conpty,
        })
    }

    /// The attacher the node's terminal service opens panes with: holder
    /// panes over their sockets, the rest through `herdr`.
    pub fn attacher(&self, herdr: Box<dyn Attacher>) -> Box<dyn Attacher> {
        Box::new(PaneAttacher {
            herdr,
            known: Arc::clone(&self.known),
        })
    }

    /// Starts `request`'s pane and attaches its screen to `service`, at the
    /// size it was started at. The pane id is the start's key: asked again
    /// for a pane that lives, it answers that pane and starts no second
    /// holder. A pane past [`MAX_ATTACHED_PANES`] runs and is reported
    /// `attach_limit` by the service, as any pane over the cap is.
    pub fn start(
        &self,
        service: &Service,
        request: &StartRequest,
    ) -> Result<Started, StartRefusal> {
        let started = self.panes.start(request, &self.source)?;
        let (Started::Started(record) | Started::Existing(record)) = &started;
        if record.end.is_some() {
            self.report_ended(record);
            return Ok(started);
        }
        self.known.insert(
            &request.pane,
            hide_kit::layout::holder_socket(&self.state_dir, &request.pane),
        );
        service.control(TerminalControl::Attach {
            pane: request.pane.clone(),
            size: Some(GridSize {
                rows: request.size.rows,
                cols: request.size.cols,
            }),
            manual: false,
        });
        Ok(started)
    }

    /// Ends the pane: its program and everything it started, then its
    /// holder, within `hide_kit::panes::CLOSE_LIMIT`, and answers its record.
    /// The service's session of it ends as a close it asked for; the node
    /// forgets the pane once it ended.
    pub fn terminate(&self, service: &Service, pane: &str) -> Result<PaneRecord, CloseError> {
        service.control(TerminalControl::Closing {
            pane: pane.to_owned(),
            closing: true,
        });
        let closed = self.panes.close(pane);
        if closed.is_ok() || matches!(closed, Err(CloseError::NoRecord)) {
            self.known.remove(pane);
            service.control(TerminalControl::Forget {
                pane: pane.to_owned(),
            });
        } else {
            service.control(TerminalControl::Closing {
                pane: pane.to_owned(),
                closing: false,
            });
        }
        closed
    }

    /// The pane's last `lines` rows, on a connection of its own that leaves
    /// the screen connection and its scroll position as they are.
    pub fn read(&self, pane: &str, lines: u16) -> Result<Answer, OpenError> {
        self.ask(pane, Ask::Read { lines })
    }

    /// The pane's shell and the processes in its terminal's foreground, as
    /// [`Holders::read`] asks.
    pub fn processes(&self, pane: &str) -> Result<Answer, OpenError> {
        self.ask(pane, Ask::Processes)
    }

    fn ask(&self, pane: &str, ask: Ask) -> Result<Answer, OpenError> {
        hide_holder_protocol::ask(
            &hide_kit::layout::holder_socket(&self.state_dir, pane),
            ask,
            Instant::now() + ASK_LIMIT,
        )
    }

    /// What a node does when it starts: reconciles the records with the
    /// holders that run until everything found is taken up (or
    /// [`RECONCILE_ROUNDS`] have run), attaches the first
    /// [`MAX_ATTACHED_PANES`] holders that run, by name, at the size each
    /// has, and keeps the rest running unattached, and reports each pane
    /// that ended while no node was there with the reason its record names.
    pub fn recover(&self, service: &Service) -> Recovered {
        let mut recovered = Recovered::default();
        let mut alive = Vec::new();
        for round in 1..=RECONCILE_ROUNDS {
            let reconciled = self.panes.reconcile(RECONCILE_BUDGET);
            let pending = reconciled.pending();
            recovered.complete = reconciled.is_complete();
            for (pane, judgment) in reconciled.panes {
                match judgment {
                    Judgment::Alive(_) => alive.push(pane),
                    Judgment::Ended(record) => {
                        self.report_ended(&record);
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
                        recovered.unsupported.push(pane);
                    }
                    // Taken up again by the next round.
                    Judgment::Pending(_) | Judgment::Starting(_) => {}
                    other => recovered.unjudged.push((pane, format!("{other:?}"))),
                }
            }
            for orphan in reconciled.orphans {
                if let OrphanOutcome::Kept(why) = orphan.outcome {
                    recovered
                        .unjudged
                        .push((orphan.pane.unwrap_or_default(), why));
                }
            }
            if let Some(failed) = reconciled.failed {
                recovered.unjudged.push((String::new(), failed));
            }
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
        alive.sort();
        alive.dedup();
        for pane in alive {
            self.known.insert(
                &pane,
                hide_kit::layout::holder_socket(&self.state_dir, &pane),
            );
            if recovered.attached.len() < MAX_ATTACHED_PANES {
                // No size: a holder keeps its own, and the screen that comes
                // back is the one it holds at this moment.
                service.control(TerminalControl::Attach {
                    pane: pane.clone(),
                    size: None,
                    manual: false,
                });
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
        recovered
    }

    /// [`Holders::recover`] on a thread of its own, so a node's start does
    /// not wait on its holders.
    pub fn recover_in_background(self: Arc<Self>, service: Arc<Service>) {
        if let Err(error) = thread::Builder::new()
            .name("hide-node-pane-holder-recover".into())
            .spawn(move || {
                self.recover(&service);
            })
        {
            crate::diagnostic!(json!({
                "component": "pane_holder",
                "kind": "pane_holder.recover_not_started",
                "message": error.to_string(),
            }));
        }
    }

    fn report_ended(&self, record: &PaneRecord) {
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
        self.report(&record.pane, "ended", category, message);
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
                mode: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_is_off_unless_set_to_one_and_refuses_other_values_naming_what_it_takes() {
        assert_eq!(Switch::parse(None), Ok(Switch::Off));
        assert_eq!(Switch::parse(Some(OsStr::new("0"))), Ok(Switch::Off));
        assert_eq!(Switch::parse(Some(OsStr::new("1"))), Ok(Switch::On));
        for value in ["", "true", "yes", " 1", "2"] {
            let refused = Switch::parse(Some(OsStr::new(value))).unwrap_err();
            assert!(
                refused.contains(SWITCH) && refused.contains("1 (run pane holders) or 0"),
                "{refused}"
            );
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
