//! Moving the Herdr server another Hide app started onto this app's Herdr,
//! with its panes and their processes kept (Herdr's `server.live_handoff`).
//!
//! The server an app starts outlives the app, so once a newer app replaces
//! it the server goes on running the old app's Herdr from wherever the old
//! app was put: renamed aside or in the Trash. Every pane runs under that
//! program until the server ends, and the system names the old app when a
//! process in a pane asks for a permission. A packaged `hide connect`
//! therefore asks the server on its socket which program it runs, and when
//! that is another Hide app's Herdr it has the server hand its panes to this
//! app's Herdr. Herdr's own clients, the daemon's terminal attaches among
//! them, are told to reconnect.
//!
//! A server the operator runs from their own Herdr is never handed over, a
//! server newer than this app's Herdr is not given to an older one, and
//! Herdr hands over on Unix only. A handoff that does not happen leaves the
//! server as it was, as every earlier build did, so each outcome is a log
//! record and nothing the connect fails on.
//!
//! A device's node keeps a rule of its own ([`device_update`], PRD
//! device-herdr-update): the server on the device's registered socket is
//! handed to the pinned copy the build brought whenever it runs an older
//! stable release of Herdr (D-04, D-06). When no server answers, the node
//! starts that copy and gives it to the account, detached from both the
//! node and SSH (device-herdr-bootstrap D2). A failed handoff is tried once
//! per server process and pin (D-13).
//!
//! This machine's server the operator runs from their own older Herdr is
//! moved by the core as soon as a look offers it, once per server (operator
//! decision 2026-10-23, replacing PRD herdr-takeover D-14): [`LocalTakeover`]
//! is what the core asks, and it judges the server by the same
//! [`device_step`] a device's node does (D-08). A failed move is remembered
//! by server process and pin in one file of this machine's state folder
//! (D-23), so the step opens on its failure after a restart, Hide does not
//! move that server again on its own, and a press always tries again.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use herdr_core::herdr_takeover::{
    FailureCause, HerdrTakeover, Look, Moved, Offer, OfferKind, PathHerdr, PathUpdated, Versions,
};
use hide_kit::device_herdr::{BehindCause, HerdrUpdate};
use hide_platform::fs::identity::file_id;
use hide_platform::ipc::LocalStream;
use hide_platform::process::{Image, image_of};

/// How long the server has to hand its panes over. Forty panes with
/// scrollback took about a second (2026-10-14); `hide connect` itself has
/// 25 s from the host.
const HANDOFF_WITHIN: Duration = Duration::from_secs(15);

/// How long a `ping` or this app's Herdr naming its version may take.
const ASK_WITHIN: Duration = Duration::from_secs(5);

/// How long Homebrew may take to upgrade the operator's `herdr`: it updates
/// its own index first, and downloads the release.
const BREW_WITHIN: Duration = Duration::from_secs(300);

/// How long a copy of this app's Herdr over the operator's may take.
const COPY_WITHIN: Duration = Duration::from_secs(30);

/// What one look at the server came to.
#[derive(Debug, Eq, PartialEq)]
pub enum Outcome {
    /// No server answers on the socket.
    NoServer,
    /// The server runs this app's Herdr.
    Ours,
    /// The server runs a Herdr no Hide app holds: the operator's own.
    NotHides { program: PathBuf },
    /// The system cannot say which program the server runs, or the server
    /// cannot hand its panes over.
    Unsupported { reason: String },
    /// The server's Herdr is newer than this app's, or the two releases do
    /// not order, so the server keeps its own.
    Kept { server: String, ours: String },
    /// The panes now belong to a server running this app's Herdr.
    HandedOff {
        from_pid: u32,
        to_pid: u32,
        from_program: PathBuf,
        elapsed_ms: u64,
    },
    /// Something along the way failed; the server stays as it was unless
    /// the stage is `confirm`.
    Failed { stage: &'static str, detail: String },
}

impl Outcome {
    /// The log record, or `None` for the two answers every connect gives
    /// once nothing is left to move.
    pub fn record(&self) -> Option<serde_json::Value> {
        use serde_json::json;
        let record = match self {
            Outcome::NoServer | Outcome::Ours => return None,
            Outcome::NotHides { program } => {
                json!({"outcome": "not_hides", "program": program.display().to_string()})
            }
            Outcome::Unsupported { reason } => json!({"outcome": "unsupported", "reason": reason}),
            Outcome::Kept { server, ours } => {
                json!({"outcome": "kept", "server_version": server, "version": ours})
            }
            Outcome::HandedOff {
                from_pid,
                to_pid,
                from_program,
                elapsed_ms,
            } => json!({
                "outcome": "handed_off", "from_pid": from_pid, "to_pid": to_pid,
                "from_program": from_program.display().to_string(), "elapsed_ms": elapsed_ms,
            }),
            Outcome::Failed { stage, detail } => {
                json!({"outcome": "failed", "stage": stage, "message": detail})
            }
        };
        let mut record = record;
        record["component"] = "hide".into();
        record["kind"] = "herdr.handoff".into();
        Some(record)
    }
}

/// Moves the server on `socket` onto `ours`, this app's Herdr, when another
/// Hide app's Herdr runs it.
pub fn hand_off(ours: &Path, socket: &Path) -> Outcome {
    let ours_file = match file_id(ours) {
        Ok(file) => file,
        Err(error) => return failed("ours", error),
    };
    let (from_pid, image) = match program_on(socket) {
        Ok(Some(found)) => found,
        Ok(None) => return Outcome::NoServer,
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            return Outcome::Unsupported {
                reason: error.to_string(),
            };
        }
        Err(error) => return failed("program", error),
    };
    if image.file == ours_file {
        return Outcome::Ours;
    }
    if !image
        .path
        .parent()
        .is_some_and(hide_kit::holds_package_runtime)
    {
        return Outcome::NotHides {
            program: image.path,
        };
    }
    let pong = match ask(socket, "ping", herdr_core::wire::ping_params())
        .and_then(herdr_core::wire::server_pong)
    {
        Ok(pong) => pong,
        Err(detail) => {
            return Outcome::Failed {
                stage: "ping",
                detail,
            };
        }
    };
    if !pong.live_handoff {
        return Outcome::Unsupported {
            reason: format!("Herdr {} cannot hand its panes over here", pong.version),
        };
    }
    let version = match version_of(ours) {
        Ok(version) => version,
        Err(detail) => {
            return Outcome::Failed {
                stage: "version",
                detail,
            };
        }
    };
    if !takes_over(&version, &pong.version) {
        return Outcome::Kept {
            server: pong.version,
            ours: version,
        };
    }
    match hand_over(ours, &ours_file, &version, socket) {
        Ok(Handed { to_pid, elapsed_ms }) => Outcome::HandedOff {
            from_pid,
            to_pid,
            from_program: image.path,
            elapsed_ms,
        },
        Err((stage, detail)) => Outcome::Failed { stage, detail },
    }
}

/// The Herdr the desktop package in `kit` ships, when it is the one this
/// process was given (`HERDR_BIN_PATH`): the host hands its own unless the
/// operator named another, whose server is theirs to keep.
pub fn shipped_herdr(kit: &Path, given: Option<&Path>) -> Option<PathBuf> {
    let shipped = kit.join(format!("herdr{}", std::env::consts::EXE_SUFFIX));
    let same = hide_platform::fs::identity::same_file(given?, &shipped).ok()?;
    same.then_some(shipped)
}

/// This machine's Herdr server against the Herdr this app ships, for the
/// core to offer and, on the operator's press, to move (PRD
/// herdr-takeover).
pub struct LocalTakeover {
    ours: PathBuf,
    socket: PathBuf,
    record: PathBuf,
    /// The version `ours` names for itself, read once.
    version: std::sync::OnceLock<String>,
    /// The operator's own `herdr`; `None` finds it as their terminal would.
    operator_herdr: Option<PathBuf>,
}

/// Who runs the server on the socket.
#[derive(Debug, Eq, PartialEq)]
enum Owner {
    Ours,
    /// Another Hide app's Herdr: `hide connect` moves it on its own (B15).
    AnotherApp,
    Operator,
    /// The system does not say which program a process runs (Windows), so
    /// only the versions decide (D-27).
    Unknown,
}

/// The step that could not read the server, and why.
type Unread = (&'static str, String);

impl LocalTakeover {
    /// The takeover of a daemon that runs from a desktop package with that
    /// package's own Herdr on `socket`; `None` for any other daemon (a
    /// development build, a node a move placed, a Herdr the operator named),
    /// which then offers nothing.
    pub fn for_daemon(
        herdr: Option<&Path>,
        socket: Option<&Path>,
        state_dir: &Path,
    ) -> Option<Self> {
        let kit = std::env::current_exe()
            .and_then(|path| path.canonicalize())
            .ok()
            .as_deref()
            .and_then(hide_kit::bundled_kit_dir)?;
        Some(Self::new(
            shipped_herdr(&kit, herdr)?,
            socket?.to_path_buf(),
            hide_kit::layout::herdr_takeover_record(state_dir),
        ))
    }

    pub fn new(ours: PathBuf, socket: PathBuf, record: PathBuf) -> Self {
        Self {
            ours,
            socket,
            record,
            version: std::sync::OnceLock::new(),
            operator_herdr: None,
        }
    }

    /// Reads `herdr` as the operator's own rather than the one their
    /// terminal finds (an isolated test's).
    pub fn with_operator_herdr(mut self, herdr: PathBuf) -> Self {
        self.operator_herdr = Some(herdr);
        self
    }

    /// The moves that failed before. A record that cannot be read counts as
    /// none and is replaced by the next failure: it decides whether the step
    /// opens on its failure, and the core keeps its own memory of the server
    /// it moved on its own, so a damaged file must not hide the offer.
    fn attempts(&self) -> Attempts {
        Attempts::read(&self.record).unwrap_or_else(|detail| {
            herdr_core::diagnostic!(serde_json::json!({
                "component": "herdr",
                "kind": "herdr.handoff",
                "outcome": "record_unreadable",
                "detail": detail,
            }));
            Attempts::default()
        })
    }

    fn version(&self) -> Result<String, String> {
        if let Some(version) = self.version.get() {
            return Ok(version.clone());
        }
        let version = version_of(&self.ours)?;
        Ok(self.version.get_or_init(|| version).clone())
    }

    /// The server on the socket, who runs it, and what it is offered; the
    /// pid is `None` where the system cannot name a socket's peer.
    fn judge(&self) -> Result<Option<(Option<u32>, Offer)>, Unread> {
        let ours_file = file_id(&self.ours).map_err(|error| ("ours", error.to_string()))?;
        let (pid, owner) = match server_on(&self.socket) {
            Ok(None) => return Ok(None),
            Ok(Some((pid, Ok(image)))) => {
                let owner = if image.file == ours_file {
                    Owner::Ours
                } else if image
                    .path
                    .parent()
                    .is_some_and(hide_kit::holds_package_runtime)
                {
                    Owner::AnotherApp
                } else {
                    Owner::Operator
                };
                (pid, owner)
            }
            Ok(Some((pid, Err(error)))) if error.kind() == io::ErrorKind::Unsupported => {
                (pid, Owner::Unknown)
            }
            Ok(Some((_, Err(error)))) | Err(error) => return Err(("program", error.to_string())),
        };
        if matches!(owner, Owner::Ours | Owner::AnotherApp) {
            return Ok(None);
        }
        let pong = ask(&self.socket, "ping", herdr_core::wire::ping_params())
            .and_then(herdr_core::wire::server_pong)
            .map_err(|detail| ("ping", detail))?;
        let version = self.version().map_err(|detail| ("version", detail))?;
        let server_protocol = u64::from(pong.protocol);
        let kind = match device_step(&version, &pong.version, pong.live_handoff) {
            DeviceStep::Current => return Ok(None),
            DeviceStep::Kept if server_protocol > hide_herdr_client::HERDR_PROTOCOL_REVISION => {
                OfferKind::UpdateHide
            }
            DeviceStep::Kept => return Ok(None),
            DeviceStep::HandOver => OfferKind::Move,
            DeviceStep::CannotHandOff => OfferKind::Restart,
        };
        let failed_before = match (kind, pid) {
            (OfferKind::Move, Some(pid)) => {
                let attempt = Attempt::of(&self.socket, pid, &self.ours)
                    .map_err(|error| ("program", error.to_string()))?;
                self.attempts()
                    .current(&version)
                    .failed_attempt(&attempt)
                    .map(|failed| failed.cause.unwrap_or(FailureCause::Refused))
            }
            _ => None,
        };
        Ok(Some((
            pid,
            Offer {
                kind,
                server_pid: pid,
                server_version: pong.version,
                server_protocol,
                version,
                failed_before,
            },
        )))
    }

    /// The operator's own `herdr`, the one their terminal finds, as it names
    /// itself: `herdr status client --json` from 0.6.2 carries its protocol,
    /// and an older release answers only `--version`, which leaves the
    /// protocol unknown and so behind this build's.
    fn path_herdr(&self) -> Option<PathHerdr> {
        let herdr = self.operator_herdr()?;
        let (version, protocol) = match status_client(&herdr) {
            Ok(answer) => (
                answer["version"].as_str()?.to_owned(),
                answer["protocol"].as_u64(),
            ),
            Err(_) => (named_version(&herdr)?, None),
        };
        Some(PathHerdr {
            update: PathUpdate::of(&self.ours, &herdr).command(),
            version,
            protocol,
        })
    }

    /// The operator's own `herdr`, as their terminal finds it.
    fn operator_herdr(&self) -> Option<PathBuf> {
        match &self.operator_herdr {
            Some(herdr) => Some(herdr.clone()),
            None => hide_platform::programs::find_cli("herdr")
                .map(hide_platform::programs::Program::into_path),
        }
    }
}

/// How the operator's `herdr` is brought to this app's Herdr without
/// touching a running server: Homebrew's own upgrade where Homebrew
/// installed it, which refuses any other update, and otherwise a copy of
/// this app's Herdr over the file the link points at, which writes a new
/// file, so a server running the old one goes on. Hide runs it after a move
/// (operator decision 2026-10-23), except a copy that needs an administrator,
/// which it only shows (letter-13293).
#[derive(Debug, Eq, PartialEq)]
enum PathUpdate {
    /// `brew` is the Homebrew of the prefix that holds the file.
    Brew { brew: PathBuf },
    Copy {
        ours: PathBuf,
        file: PathBuf,
        needs_admin: bool,
    },
}

impl PathUpdate {
    fn of(ours: &Path, herdr: &Path) -> Self {
        let file = std::fs::canonicalize(herdr).unwrap_or_else(|_| herdr.to_owned());
        if let Some(prefix) = homebrew_prefix(&file) {
            return Self::Brew {
                brew: prefix.join("bin").join("brew"),
            };
        }
        let needs_admin = file.parent().is_some_and(|dir| {
            !hide_platform::fs::permissions::account_can_add_to(dir).unwrap_or(true)
        });
        Self::Copy {
            ours: ours.to_owned(),
            file,
            needs_admin,
        }
    }

    /// The command as the operator would type it in their terminal.
    fn command(&self) -> String {
        match self {
            Self::Brew { .. } => "brew upgrade herdr".to_owned(),
            Self::Copy {
                ours,
                file,
                needs_admin,
            } => copy_command(ours, file, *needs_admin),
        }
    }

    /// Whether Hide runs it: never a copy that asks for an administrator,
    /// and never on a system with no move to follow (Windows).
    fn runs(&self) -> bool {
        match self {
            Self::Brew { .. } => true,
            Self::Copy { needs_admin, .. } => cfg!(unix) && !needs_admin,
        }
    }

    /// Runs it, when it [`runs`](Self::runs), as a program with its
    /// arguments rather than through a shell.
    fn run(&self) -> Result<(), String> {
        if !self.runs() {
            return Err("it is left to the operator".to_owned());
        }
        let (mut command, within) = match self {
            Self::Brew { brew } => {
                // A prefix without its own `brew` is found as the terminal would.
                let program = if brew.is_file() {
                    brew.clone()
                } else {
                    PathBuf::from("brew")
                };
                let mut command = hide_platform::programs::command(program);
                command.args(["upgrade", "herdr"]);
                (command, BREW_WITHIN)
            }
            Self::Copy { ours, file, .. } => {
                let mut command = hide_platform::programs::command("install");
                command.args(["-m", "755"]).arg(ours).arg(file);
                (command, COPY_WITHIN)
            }
        };
        let finished = hide_platform::process::run_to_end(
            &mut command,
            within,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .map_err(|error| format!("{}: {error:?}", self.command()))?;
        if !finished.succeeded() {
            return Err(format!(
                "{} exited {:?}: {}",
                self.command(),
                finished.code,
                finished.last_error_line()
            ));
        }
        Ok(())
    }
}

/// The prefix of a file Homebrew put in its cellar
/// (`<prefix>/Cellar/herdr/<version>/bin`).
fn homebrew_prefix(file: &Path) -> Option<&Path> {
    file.ancestors().find_map(|folder| {
        let cellar = folder.parent()?;
        (folder.file_name()? == "herdr" && cellar.file_name()? == "Cellar")
            .then_some(())
            .and(cellar.parent())
    })
}

#[cfg(unix)]
fn copy_command(ours: &Path, file: &Path, needs_admin: bool) -> String {
    let quote = |path: &Path| format!("'{}'", path.display().to_string().replace('\'', r"'\''"));
    format!(
        "{}install -m 755 {} {}",
        if needs_admin { "sudo " } else { "" },
        quote(ours),
        quote(file)
    )
}

/// PowerShell has no `sudo`; a folder this account cannot write asks for an
/// administrator PowerShell, which `docs/INSTALL.md` names as a gap.
#[cfg(windows)]
fn copy_command(ours: &Path, file: &Path, _needs_admin: bool) -> String {
    let quote = |path: &Path| format!("'{}'", path.display().to_string().replace('\'', "''"));
    format!(
        "Copy-Item -Force -LiteralPath {} -Destination {}",
        quote(ours),
        quote(file)
    )
}

/// The takeover of a daemon that cannot move a server, one not run from a
/// desktop package with that package's own Herdr: its look finds nothing,
/// so a Herdr step an earlier daemon saved goes on to the agents.
pub struct NoTakeover;

impl HerdrTakeover for NoTakeover {
    fn look(&self) -> Look {
        Look::Nothing
    }

    fn versions(&self) -> Versions {
        Versions::default()
    }

    fn hand_over(&self) -> Moved {
        Moved::Failed {
            cause: FailureCause::Unreadable,
            stage: "unavailable".into(),
            detail: "this daemon does not run from a desktop package with its own Herdr".into(),
        }
    }

    fn update_path_herdr(&self) -> PathUpdated {
        PathUpdated::Gone
    }
}

impl HerdrTakeover for LocalTakeover {
    fn update_path_herdr(&self) -> PathUpdated {
        let Some(herdr) = self.operator_herdr() else {
            return PathUpdated::Gone;
        };
        let update = PathUpdate::of(&self.ours, &herdr);
        let ran = update.runs().then(|| update.command());
        let outcome = update.run();
        match self.path_herdr() {
            None => PathUpdated::Gone,
            Some(now) if !now.behind() => PathUpdated::Current {
                version: now.version,
            },
            Some(now) => PathUpdated::Behind {
                detail: match outcome {
                    Ok(()) => format!(
                        "herdr {} still speaks protocol {:?}",
                        now.version, now.protocol
                    ),
                    Err(detail) => detail,
                },
                version: now.version,
                ran,
            },
        }
    }

    fn versions(&self) -> Versions {
        Versions {
            hide: self.version().ok(),
            path_herdr: self.path_herdr(),
        }
    }

    fn look(&self) -> Look {
        match self.judge() {
            Ok(None) => Look::Nothing,
            Ok(Some((_, offer))) => Look::Offer(offer),
            Err((stage, detail)) => Look::Unreadable {
                stage: stage.to_owned(),
                detail,
            },
        }
    }

    fn hand_over(&self) -> Moved {
        let unreadable = |stage: &str, detail: String| Moved::Failed {
            cause: FailureCause::Unreadable,
            stage: stage.to_owned(),
            detail,
        };
        // Judged again: the server may have changed since the offer.
        let (pid, offer) = match self.judge() {
            Ok(Some((Some(pid), offer))) if offer.kind == OfferKind::Move => (pid, offer),
            Ok(_) => {
                return unreadable("look", "the server is no longer one to move".to_owned());
            }
            Err((stage, detail)) => return unreadable(stage, detail),
        };
        let ours_file = match file_id(&self.ours) {
            Ok(file) => file,
            Err(error) => return unreadable("ours", error.to_string()),
        };
        match hand_over(&self.ours, &ours_file, &offer.version, &self.socket) {
            Ok(Handed { to_pid, elapsed_ms }) => Moved::Done {
                from_pid: pid,
                to_pid,
                elapsed_ms,
                path_herdr_behind: self
                    .path_herdr()
                    .filter(|herdr| {
                        herdr.protocol.is_none_or(|protocol| {
                            protocol < hide_herdr_client::HERDR_PROTOCOL_REVISION
                        })
                    })
                    .map(|herdr| herdr.version),
            },
            Err((stage, detail)) => {
                let cause = match stage {
                    "handoff" => FailureCause::Refused,
                    "confirm" => FailureCause::NotConfirmed,
                    _ => FailureCause::Unreadable,
                };
                let reason = format!("{stage}: {detail}");
                let kept = Attempt::of(&self.socket, pid, &self.ours)
                    .map_err(|error| error.to_string())
                    .and_then(|attempt| {
                        self.attempts()
                            .current(&offer.version)
                            .add_with_cause(
                                attempt,
                                reason.clone(),
                                Some(cause),
                                Some(offer.version.clone()),
                            )
                            .write(&self.record)
                    });
                Moved::Failed {
                    cause,
                    stage: stage.to_owned(),
                    detail: match kept {
                        Ok(()) => reason,
                        Err(error) => format!("{reason}; the attempt was not recorded: {error}"),
                    },
                }
            }
        }
    }
}

/// The server answering on `socket`, by its pid where the system names a
/// socket's peer, and the program it runs where the system says.
#[allow(clippy::type_complexity)]
fn server_on(socket: &Path) -> io::Result<Option<(Option<u32>, io::Result<Image>)>> {
    let stream = match LocalStream::connect(socket) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    match stream.peer_pid() {
        Ok(pid) => Ok(Some((Some(pid), image_of(pid)))),
        Err(error) if error.kind() == io::ErrorKind::Unsupported => Ok(Some((None, Err(error)))),
        Err(error) => Err(error),
    }
}

/// A server that now runs the Herdr it was handed to.
struct Handed {
    to_pid: u32,
    elapsed_ms: u64,
}

/// Has the server on `socket` hand its panes to `ours`, whose file is
/// `ours_file` and whose version is `version`, and confirms the server on
/// the socket runs it afterwards. A failure names its stage; the server
/// stays as it was unless the stage is `confirm`.
fn hand_over(
    ours: &Path,
    ours_file: &hide_platform::fs::identity::FileId,
    version: &str,
    socket: &Path,
) -> Result<Handed, (&'static str, String)> {
    let started = Instant::now();
    ours.to_str()
        .ok_or_else(|| format!("{} is not a path Herdr can take", ours.display()))
        .and_then(|ours| herdr_core::wire::live_handoff_params(ours, version))
        .and_then(|params| {
            hide_herdr_client::request_with_timeout(
                socket,
                "server.live_handoff",
                params,
                HANDOFF_WITHIN,
            )
            .map_err(|error| error.to_string())
        })
        .and_then(herdr_core::wire::live_handoff_done)
        .map_err(|detail| ("handoff", detail))?;
    match program_on(socket) {
        Ok(Some((to_pid, now))) if now.file == *ours_file => Ok(Handed {
            to_pid,
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        }),
        Ok(Some((pid, now))) => Err((
            "confirm",
            format!("the server (pid {pid}) runs {}", now.path.display()),
        )),
        Ok(None) => Err(("confirm", "no server answers after the handoff".to_owned())),
        Err(error) => Err(("confirm", error.to_string())),
    }
}

/// What a device's node does after its kit has applied: the server on the
/// device's registered `socket` is handed to `pinned`, the copy of Herdr the
/// build brought, when it runs an older stable release, or started when
/// none answers; `record` keeps the
/// handoffs that failed, so each is tried once per server process and pin.
/// Nothing here fails the kit pass: every outcome is an answer.
pub fn device_update(pinned: &Path, socket: &Path, record: &Path) -> HerdrUpdate {
    let failed = |stage: &str, reason: String, version: Option<&str>, server: Option<&str>| {
        HerdrUpdate::Failed {
            stage: stage.to_owned(),
            reason,
            version: version.map(str::to_owned),
            server_version: server.map(str::to_owned),
        }
    };
    let pinned_file = match file_id(pinned) {
        Ok(file) => file,
        Err(error) => return failed("pinned", error.to_string(), None, None),
    };
    let version = match version_of(pinned) {
        Ok(version) => version,
        Err(reason) => return failed("version", reason, None, None),
    };
    let from_pid = match program_on(socket) {
        Ok(Some((pid, _))) => pid,
        Ok(None) => {
            return match start_device_server(pinned, &pinned_file, socket) {
                Ok(pid) => HerdrUpdate::Started { version, pid },
                Err(reason) => failed("start", reason, Some(&version), None),
            };
        }
        Err(error) => return failed("program", error.to_string(), Some(&version), None),
    };
    let pong = match ask(socket, "ping", herdr_core::wire::ping_params())
        .and_then(herdr_core::wire::server_pong)
    {
        Ok(pong) => pong,
        Err(reason) => return failed("ping", reason, Some(&version), None),
    };
    let server_version = pong.version;
    match device_step(&version, &server_version, pong.live_handoff) {
        DeviceStep::Current => return HerdrUpdate::Current { version },
        DeviceStep::Kept => {
            return HerdrUpdate::Kept {
                server_version,
                version,
            };
        }
        DeviceStep::CannotHandOff => {
            return HerdrUpdate::Behind {
                reason: format!("Herdr {server_version} cannot hand its panes over"),
                server_version,
                version,
                cause: BehindCause::CannotHandOff,
            };
        }
        DeviceStep::HandOver => {}
    }
    let known =
        |stage: &str, reason: String| failed(stage, reason, Some(&version), Some(&server_version));
    let attempt = match Attempt::of(socket, from_pid, pinned) {
        Ok(attempt) => attempt,
        Err(error) => return known("program", error.to_string()),
    };
    let attempts = match Attempts::read(record) {
        Ok(attempts) => attempts,
        Err(reason) => return known("record", reason),
    };
    if let Some(earlier) = attempts.failed(&attempt) {
        return HerdrUpdate::Behind {
            server_version,
            version,
            cause: BehindCause::HandoffFailed,
            reason: format!("an earlier handoff of this server failed: {earlier}"),
        };
    }
    match hand_over(pinned, &pinned_file, &version, socket) {
        Ok(Handed { to_pid, elapsed_ms }) => HerdrUpdate::HandedOff {
            server_version,
            version,
            from_pid,
            to_pid,
            elapsed_ms,
        },
        Err((stage, detail)) => {
            let reason = format!("{stage}: {detail}");
            let kept = attempts.add(attempt, reason.clone()).write(record);
            HerdrUpdate::Behind {
                server_version,
                version,
                cause: BehindCause::HandoffFailed,
                reason: match kept {
                    Ok(()) => reason,
                    Err(error) => format!("{reason}; the attempt was not recorded: {error}"),
                },
            }
        }
    }
}

/// The short-lived starter exits before this returns: the server is then
/// reparented outside the node's process tree as well as its process group.
/// SSH cleanup of the node cannot end the account's server.
fn start_device_server(
    pinned: &Path,
    pinned_file: &hide_platform::fs::identity::FileId,
    socket: &Path,
) -> Result<u32, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut command = hide_platform::programs::command(executable);
    command
        .arg("node")
        .arg("start-herdr")
        .arg(pinned)
        .arg(socket);
    let finished = hide_platform::process::run_to_end(
        &mut command,
        HANDOFF_WITHIN + ASK_WITHIN,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|error| format!("the Herdr starter failed: {error:?}"))?;
    if !finished.succeeded() {
        return Err(format!(
            "the Herdr starter exited {:?}: {}",
            finished.code,
            finished.stderr.trim()
        ));
    }
    let pid: u32 = finished
        .stdout
        .trim()
        .parse()
        .map_err(|error| format!("the Herdr starter returned no pid: {error}"))?;
    match program_on(socket).map_err(|error| error.to_string())? {
        Some((answering, image)) if answering == pid && image.file == *pinned_file => Ok(pid),
        _ => Err("the started Herdr no longer answers on its socket".to_owned()),
    }
}

/// Internal `hided node start-herdr` entrypoint. Its only successful exit
/// gives the server to the account; all failed starts still own and end
/// their child. It holds no SSH descriptor and starts at most one server.
#[cfg(unix)]
#[allow(clippy::disallowed_methods)] // bounded readiness polling, not a readiness delay
pub(crate) fn start_detached_device_server(pinned: &Path, socket: &Path) -> Result<u32, String> {
    use std::process::Stdio;

    let pinned_file = file_id(pinned).map_err(|error| error.to_string())?;
    let mut command = hide_platform::programs::command(pinned);
    command
        .arg("server")
        .env("HERDR_SOCKET_PATH", socket)
        .env_remove("HERDR_SESSION")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_platform::process::detach(&mut command).map_err(|error| error.to_string())?;
    let mut child = hide_platform::process::OwnedChild::spawn(&mut command)
        .map_err(|error| format!("starting the pinned Herdr: {error}"))?;
    let pid = child.id();
    let deadline = Instant::now() + HANDOFF_WITHIN;
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Err(format!(
                "the pinned Herdr exited before answering: {status}"
            ));
        }
        if let Some((answering, image)) = program_on(socket).map_err(|error| error.to_string())? {
            if answering != pid || image.file != pinned_file {
                return Err("another server took the socket while Herdr was starting".to_owned());
            }
            // A listening socket alone is not an answering server.
            if hide_herdr_client::request_with_timeout(
                socket,
                "ping",
                herdr_core::wire::ping_params()?,
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(ASK_WITHIN),
            )
            .map_err(|error| error.to_string())
            .and_then(herdr_core::wire::server_pong)
            .is_ok()
            {
                child.release();
                return Ok(pid);
            }
        }
        if Instant::now() >= deadline {
            return Err("the pinned Herdr did not answer before the startup deadline".to_owned());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(not(unix))]
pub(crate) fn start_detached_device_server(_pinned: &Path, _socket: &Path) -> Result<u32, String> {
    Err("starting a device Herdr server requires macOS or Linux".to_owned())
}

/// What a device's server is due, from its Herdr's version, the pin's and
/// whether the server can hand its panes over.
#[derive(Debug, Eq, PartialEq)]
enum DeviceStep {
    /// The server already runs the pinned release, from whichever file.
    Current,
    /// A newer release, or one the pin does not order against: left alone.
    Kept,
    /// An older release that cannot hand its panes over.
    CannotHandOff,
    HandOver,
}

fn device_step(pinned: &str, server: &str, live_handoff: bool) -> DeviceStep {
    if pinned == server {
        return DeviceStep::Current;
    }
    match (release(pinned), release(server)) {
        (Some(pinned), Some(server)) if pinned > server => {
            if live_handoff {
                DeviceStep::HandOver
            } else {
                DeviceStep::CannotHandOff
            }
        }
        _ => DeviceStep::Kept,
    }
}

/// The most failed handoffs a device keeps (engineering rule 15): one per
/// server process, and a device runs only a few servers.
const ATTEMPTS_KEPT: usize = 16;

/// One handoff of one server process to one pinned copy.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
struct Attempt {
    socket: PathBuf,
    pid: u32,
    /// The process's start, so a pid a restarted server reuses is a new
    /// server (D-13).
    started: u64,
    pinned: PathBuf,
}

impl Attempt {
    fn of(socket: &Path, pid: u32, pinned: &Path) -> io::Result<Self> {
        Ok(Self {
            socket: socket.to_path_buf(),
            pid,
            started: hide_platform::process::start_time(pid)?,
            pinned: pinned.to_path_buf(),
        })
    }
}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
struct Attempts {
    failed: Vec<FailedAttempt>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct FailedAttempt {
    #[serde(flatten)]
    attempt: Attempt,
    reason: String,
    /// What the screen names, for a move this machine's operator asked for;
    /// a device's record has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cause: Option<FailureCause>,
    /// The release of the app's Herdr it was moving to: an app update keeps
    /// the program's path, so the release is what makes a new pin new.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
}

impl Attempts {
    /// The record at `path`, empty when there is none yet.
    fn read(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("{} is unreadable: {error}", path.display())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("{} could not be read: {error}", path.display())),
        }
    }

    /// Why `attempt` failed before, when it did.
    fn failed(&self, attempt: &Attempt) -> Option<&str> {
        self.failed_attempt(attempt)
            .map(|failed| failed.reason.as_str())
    }

    fn failed_attempt(&self, attempt: &Attempt) -> Option<&FailedAttempt> {
        self.failed.iter().find(|failed| failed.attempt == *attempt)
    }

    /// This machine's record without the moves whose server process has
    /// ended or whose app Herdr is no longer `version` (D-23).
    fn current(mut self, version: &str) -> Self {
        self.failed.retain(|failed| {
            failed.version.as_deref() == Some(version)
                && hide_platform::process::start_time(failed.attempt.pid).ok()
                    == Some(failed.attempt.started)
        });
        self
    }

    /// The record with `attempt` added and only the newest attempts kept.
    fn add(self, attempt: Attempt, reason: String) -> Self {
        self.add_with_cause(attempt, reason, None, None)
    }

    /// [`Attempts::add`] naming what the screen says of it; the same
    /// attempt failing again replaces its line.
    fn add_with_cause(
        mut self,
        attempt: Attempt,
        reason: String,
        cause: Option<FailureCause>,
        version: Option<String>,
    ) -> Self {
        self.failed.retain(|failed| failed.attempt != attempt);
        self.failed.push(FailedAttempt {
            attempt,
            reason,
            cause,
            version,
        });
        let over = self.failed.len().saturating_sub(ATTEMPTS_KEPT);
        self.failed.drain(..over);
        self
    }

    fn write(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        hide_platform::fs::atomic::write_file(path, &bytes, hide_platform::fs::Access::Private)
            .map(|_| ())
            .map_err(|error| format!("{}: {error}", path.display()))
    }
}

fn failed(stage: &'static str, error: io::Error) -> Outcome {
    Outcome::Failed {
        stage,
        detail: error.to_string(),
    }
}

/// The server answering on `socket` and the program it runs, `None` when
/// nothing answers there.
fn program_on(socket: &Path) -> io::Result<Option<(u32, Image)>> {
    let stream = match LocalStream::connect(socket) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let pid = stream.peer_pid()?;
    image_of(pid).map(|image| Some((pid, image)))
}

fn ask(
    socket: &Path,
    method: &str,
    params: Result<serde_json::Value, String>,
) -> Result<serde_json::Value, String> {
    hide_herdr_client::request_with_timeout(socket, method, params?, ASK_WITHIN)
        .map_err(|error| format!("{method}: {error}"))
}

/// The version this app's Herdr names for itself.
fn version_of(herdr: &Path) -> Result<String, String> {
    status_client(herdr)?["version"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "herdr status client named no version".to_owned())
}

/// What a Herdr program says of itself (`herdr status client --json`).
fn status_client(herdr: &Path) -> Result<serde_json::Value, String> {
    let mut command = hide_platform::programs::command(herdr);
    command.args(["status", "client", "--json"]);
    let finished = hide_platform::process::run_to_end(
        &mut command,
        ASK_WITHIN,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|error| format!("herdr status client: {error:?}"))?;
    if !finished.succeeded() {
        return Err(format!(
            "herdr status client exited {:?}: {}",
            finished.code,
            finished.stderr.trim()
        ));
    }
    serde_json::from_str(finished.stdout.trim())
        .map_err(|error| format!("herdr status client answered no JSON: {error}"))
}

/// The release a Herdr too old for `status client` names (`herdr 0.5.12`).
fn named_version(herdr: &Path) -> Option<String> {
    let mut command = hide_platform::programs::command(herdr);
    command.arg("--version");
    let finished = hide_platform::process::run_to_end(
        &mut command,
        ASK_WITHIN,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .ok()
    .filter(|finished| finished.succeeded())?;
    finished
        .stdout
        .trim()
        .strip_prefix("herdr ")
        .map(str::to_owned)
}

/// Whether this app's Herdr may take the server's panes: the same release,
/// or a later one. A newer server is not handed back to an older Herdr,
/// which may not read what it would be handed, and releases that do not
/// order (a preview beside a release) are left as they are.
fn takes_over(ours: &str, server: &str) -> bool {
    if ours == server {
        return true;
    }
    match (release(ours), release(server)) {
        (Some(ours), Some(server)) => ours > server,
        _ => false,
    }
}

fn release(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let release = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(release)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_same_or_a_later_release_takes_a_servers_panes() {
        for (ours, server, takes) in [
            ("0.9.3", "0.9.3", true),
            ("0.9.4", "0.9.3", true),
            ("0.10.0", "0.9.12", true),
            ("1.0.0", "0.9.3", true),
            ("0.9.3", "0.9.4", false),
            ("0.9.3", "0.10.0", false),
            ("0.9.4-preview.1", "0.9.3", false),
            ("0.9.3", "0.9.4-preview.1", false),
            ("0.9", "0.9.3", false),
        ] {
            assert_eq!(takes_over(ours, server), takes, "{ours} over {server}");
        }
    }

    #[test]
    fn a_device_server_is_handed_over_only_from_an_older_release_that_can() {
        for (pinned, server, live_handoff, step) in [
            ("0.9.3", "0.9.3", true, DeviceStep::Current),
            ("0.9.3", "0.9.3", false, DeviceStep::Current),
            ("0.9.3", "0.9.1", true, DeviceStep::HandOver),
            ("0.10.0", "0.9.12", true, DeviceStep::HandOver),
            ("0.9.3", "0.9.1", false, DeviceStep::CannotHandOff),
            ("0.9.3", "0.9.4", true, DeviceStep::Kept),
            ("0.9.3", "0.9.4", false, DeviceStep::Kept),
            ("0.9.3", "0.9.4-preview.1", true, DeviceStep::Kept),
            ("0.9.4-preview.1", "0.9.3", true, DeviceStep::Kept),
        ] {
            assert_eq!(
                device_step(pinned, server, live_handoff),
                step,
                "{server} under pin {pinned}, live handoff {live_handoff}"
            );
        }
    }

    #[test]
    fn a_failed_handoff_is_remembered_for_that_server_process_and_pin_only() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("handoff.json");
        let attempt = |pid, started, pinned: &str| Attempt {
            socket: PathBuf::from("/home/me/.config/herdr/herdr.sock"),
            pid,
            started,
            pinned: PathBuf::from(pinned),
        };
        let first = attempt(41, 7, "/root/herdr/aaaa/herdr");
        Attempts::read(&record)
            .unwrap()
            .add(first.clone(), "handoff: refused".to_owned())
            .write(&record)
            .unwrap();

        let read = Attempts::read(&record).unwrap();
        assert_eq!(read.failed(&first), Some("handoff: refused"));
        // A restarted server, even on the same pid, and a new pin are new.
        assert_eq!(read.failed(&attempt(41, 8, "/root/herdr/aaaa/herdr")), None);
        assert_eq!(read.failed(&attempt(41, 7, "/root/herdr/bbbb/herdr")), None);

        let mut many = read;
        for pid in 0..40 {
            many = many.add(attempt(pid, 1, "/root/herdr/aaaa/herdr"), "x".to_owned());
        }
        assert_eq!(many.failed.len(), ATTEMPTS_KEPT);
        assert_eq!(many.failed(&first), None, "the oldest attempts go first");
    }

    /// D-23: this machine's record keeps a failed move only while its server
    /// process runs and the app's Herdr is the release it was moving to.
    #[test]
    fn a_failed_move_is_forgotten_once_its_server_ends_or_the_pin_moves() {
        let me = std::process::id();
        let live = Attempt {
            socket: PathBuf::from("/tmp/herdr.sock"),
            pid: me,
            started: hide_platform::process::start_time(me).unwrap(),
            pinned: PathBuf::from("/Applications/hide.app/Contents/Resources/herdr"),
        };
        let ended = Attempt {
            started: live.started.wrapping_add(1),
            ..live.clone()
        };
        let record = Attempts::default()
            .add_with_cause(
                ended.clone(),
                "handoff: refused".into(),
                Some(FailureCause::Refused),
                Some("0.9.5".into()),
            )
            .add_with_cause(
                live.clone(),
                "confirm: other".into(),
                Some(FailureCause::NotConfirmed),
                Some("0.9.5".into()),
            )
            .add_with_cause(
                live.clone(),
                "handoff: refused again".into(),
                Some(FailureCause::Refused),
                Some("0.9.5".into()),
            );
        assert_eq!(record.failed.len(), 2, "the same attempt keeps one line");
        let current = record.current("0.9.5");
        assert!(current.failed_attempt(&ended).is_none());
        assert_eq!(
            current
                .failed_attempt(&live)
                .and_then(|failed| failed.cause),
            Some(FailureCause::Refused)
        );
        assert!(current.current("0.9.6").failed.is_empty());
    }

    /// This machine's move record only says whether the step opens on its
    /// failure, so a damaged one hides nothing and the next failure
    /// replaces it; a device's record, which gates its updater, refuses.
    #[test]
    fn a_damaged_move_record_counts_as_no_failure_and_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("herdr-handoff.json");
        std::fs::write(&record, b"{not json").unwrap();
        let takeover = LocalTakeover::new(
            dir.path().join("herdr"),
            dir.path().join("herdr.sock"),
            record.clone(),
        );
        assert!(takeover.attempts().failed.is_empty());
        let me = std::process::id();
        let attempt = Attempt {
            socket: dir.path().join("herdr.sock"),
            pid: me,
            started: hide_platform::process::start_time(me).unwrap(),
            pinned: dir.path().join("herdr"),
        };
        takeover
            .attempts()
            .add_with_cause(
                attempt.clone(),
                "handoff: refused".into(),
                Some(FailureCause::Refused),
                Some("0.9.5".into()),
            )
            .write(&record)
            .unwrap();
        assert!(takeover.attempts().failed_attempt(&attempt).is_some());
    }

    #[test]
    fn an_unreadable_record_is_refused_rather_than_read_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("handoff.json");
        std::fs::write(&record, b"not json").unwrap();
        assert!(Attempts::read(&record).is_err());
    }

    #[test]
    fn a_homebrew_herdr_is_upgraded_by_homebrew() {
        let dir = tempfile::tempdir().unwrap();
        let keg = dir.path().join("Cellar/herdr/0.7.5/bin");
        std::fs::create_dir_all(&keg).unwrap();
        std::fs::write(keg.join("herdr"), b"").unwrap();
        let update = PathUpdate::of(
            Path::new("/Applications/hide.app/herdr"),
            &keg.join("herdr"),
        );
        assert_eq!(update.command(), "brew upgrade herdr");
        assert!(update.runs());
        assert_eq!(
            update,
            PathUpdate::Brew {
                brew: std::fs::canonicalize(dir.path()).unwrap().join("bin/brew")
            },
            "the Homebrew that installed it upgrades it"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_herdr_of_ones_own_is_replaced_by_a_copy_and_a_locked_folder_asks_for_sudo() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("it's bin");
        std::fs::create_dir(&own).unwrap();
        std::fs::write(own.join("herdr"), b"").unwrap();
        let file = std::fs::canonicalize(own.join("herdr")).unwrap();
        let ours = Path::new("/Applications/hide.app/Contents/Resources/herdr");
        let open = PathUpdate::of(ours, &own.join("herdr"));
        assert_eq!(
            open.command(),
            format!(
                "install -m 755 '{}' '{}'",
                ours.display(),
                file.display().to_string().replace('\'', r"'\''")
            )
        );
        assert!(open.runs());
        std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o555)).unwrap();
        let locked = PathUpdate::of(ours, &own.join("herdr"));
        std::fs::set_permissions(&own, std::fs::Permissions::from_mode(0o755)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert!(
                locked.command().starts_with("sudo install -m 755 "),
                "{}",
                locked.command()
            );
            assert!(!locked.runs(), "a command that needs sudo is never run");
            assert!(locked.run().is_err());
        }
    }

    /// A copy the account may make replaces the operator's `herdr` with a new
    /// file of this app's, executable, and leaves the old one to a process
    /// still running it.
    #[cfg(unix)]
    #[test]
    fn a_copy_hide_runs_replaces_the_file_with_a_new_one() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let ours = dir.path().join("ours");
        std::fs::write(&ours, b"new herdr").unwrap();
        let theirs = dir.path().join("theirs");
        std::fs::write(&theirs, b"old herdr").unwrap();
        let before = std::fs::metadata(&theirs).unwrap().ino();
        PathUpdate::of(&ours, &theirs).run().unwrap();
        assert_eq!(std::fs::read(&theirs).unwrap(), b"new herdr");
        let after = std::fs::metadata(&theirs).unwrap();
        assert_eq!(after.permissions().mode() & 0o777, 0o755);
        assert_ne!(
            after.ino(),
            before,
            "a new file, not the running one rewritten"
        );
    }
}
