//! A device's node: `hided node serve` answering the `hide_host` contract
//! over one link, the standard streams of the child that reached the device
//! (PRD S5.5 D-05, D-20, D-23; system-ssh-device-link D-02).
//!
//! Nothing here runs without the operator's consent for that device, recorded
//! on its registration and bound to the identity the helper was first allowed
//! on. The helper is installed (or replaced by a newer build) under the
//! consented install root by the start script on that same child
//! ([`bootstrap`]), started there with `node serve`, and ends when the child
//! does: there is no daemon, no listening socket and no background install.
//!
//! Admission is bounded per device: four requests run and thirty-two wait; a
//! request past that is refused as busy, never dropped. A request that was
//! sent and got no answer, because the connection ended or the deadline
//! passed, is `Unknown`: it may have taken effect, and the caller settles it
//! by reading the target again rather than by resending it (B14, B33).

use super::*;
#[path = "bootstrap.rs"]
pub(super) mod bootstrap;
#[path = "device_bundles.rs"]
mod device_bundles;
#[path = "inbound.rs"]
pub mod inbound;
#[path = "link_streams.rs"]
mod link_streams;
#[path = "wsl.rs"]
pub mod wsl;
use hide_node_link::HostError;
pub use hide_node_link::LinkError;
use hide_node_link::panes::{NodeEvent, PanesStarted};
use hide_node_link::protocol::{Call, Hello, PROTOCOL_VERSION, Request};
use hide_node_link::{LinkAnswer, NodeLink, call_as};
pub use link_streams::{LinkStream, LinkWriter, OpenError, StreamCloser};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Condvar;
use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::sync::mpsc;

/// The longest answer line read from a device helper. The largest answer is
/// a 16 MiB document, which JSON escaping can grow by up to six times, so
/// this bounds memory without refusing any answer the protocol can produce.
const MAX_ANSWER_BYTES: usize = 128 * 1024 * 1024;

/// One answer line as the helper sends it (`hide_node_link::protocol::Response`),
/// with the result borrowed as JSON text rather than built into a `Value`.
#[derive(serde::Deserialize)]
struct AnswerLine<'a> {
    id: u64,
    // `Option` alone reads a `null` result as absent; a unit answer is `null`.
    #[serde(borrow, default, deserialize_with = "present")]
    ok: Option<&'a RawValue>,
    #[serde(default)]
    error: Option<HostError>,
}

/// What the reader hands a waiting call: the result's text or the refusal.
type Answered = Result<Box<RawValue>, HostError>;

/// One report line (`hide_node_link::protocol::Progress`), its report kept as
/// text.
#[derive(serde::Deserialize)]
struct ProgressLine<'a> {
    progress: u64,
    #[serde(borrow)]
    report: &'a RawValue,
}

/// Reports one call may have waiting for its caller; a call that reads them
/// slower than they arrive is stopped with an error (engineering rule 15).
pub const MAX_WAITING_REPORTS: usize = 256;

/// What the reader hands a waiting call.
enum Delivery {
    Report(Box<RawValue>),
    Answer(Answered),
    /// More than [`MAX_WAITING_REPORTS`] reports waited unread.
    Overflow,
}

/// A call waiting for its answer, and, when it hears reports, how many wait
/// unread.
struct Waiting {
    sender: mpsc::Sender<Delivery>,
    reports: Option<Arc<AtomicUsize>>,
    /// The call reports until told to stop (`Call::runs_until_stopped`), so
    /// a draining link stops it rather than wait for its answer.
    until_stopped: bool,
}

fn present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<&'de RawValue>, D::Error> {
    serde::Deserialize::deserialize(deserializer).map(Some)
}

pub use hide_node_link::device::{
    DEFAULT_CLI_DIR, DEFAULT_HELPER_ROOT, EstablishError, Established, HOST_CONSENT_CARRIED_FROM,
    HOST_CONSENT_CONTRACT, NodeReady, Upload,
};

pub const MAX_RUNNING: usize = hide_host::serve::CONCURRENCY;
/// Link control requests ([`Call::is_control`]) running at once, beside
/// the machine calls: the node answers them on workers of their own.
pub const MAX_CONTROL_RUNNING: usize = hide_host::serve::CONTROL_CONCURRENCY;
/// Background Git work ([`Call::is_background`]) running at once, on a lane
/// of its own at both ends.
pub const MAX_BACKGROUND_RUNNING: usize = hide_host::serve::BACKGROUND_CONCURRENCY;
/// The core's engine asking the node's pane holders ([`Call::is_pane`])
/// running at once, on a lane of their own at both ends.
pub const MAX_PANE_RUNNING: usize = hide_host::serve::PANE_CONCURRENCY;
pub const MAX_QUEUED: usize = 32;

/// This program, which a device runs in its node role (`hided node serve`,
/// PRD core-host-node D-02).
const HELPER_NAME: &str = "hided";
/// The pane-side Workspace CLI, the same `hide` this daemon ships, so a
/// device's panes can reach this Hide through their return route.
const CLI_NAME: &str = "hide";
/// The rest of the install kit (`hide_kit`), under the names the kit reads
/// in its folder: the hook helper.
const HOOKS_NAME: &str = "hide-agent-hooks";
/// The pane holder (`hide-holder`), which a device of the core's own engine
/// runs its panes in, beside the helper in the same build folder, as the
/// app carries it beside `hided` (PRD engine-bundle-3-core-owns D-22).
const HOLDER_NAME: &str = "hide-holder";

/// A program's file name on a device of `os`: Windows finds a program by its
/// `.exe` suffix.
fn device_program(name: &str, os: &str) -> String {
    if os == "windows" {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// How long a draining connection waits for its admitted requests before it
/// closes anyway. An admitted request is written and answered by its one
/// deadline, and the longest call timeout is 120 s (an index walk, a
/// worktree removal), so this outlasts it.
const DRAIN_BOUND: Duration = Duration::from_secs(250);
/// How long a cancel waits to be written; the call it stops ends on its
/// own timeout whatever happens to the cancel.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const HELLO_TIMEOUT: Duration = Duration::from_secs(15);
/// How long the first Hello waits when the helper program was written by
/// this very connection's install: a program's first start after it is
/// written can take tens of seconds while the system scans it (Windows
/// Defender on a new `.exe`; on a real Windows 11 PC on 2026-11-04 the
/// first Hello passed [`HELLO_TIMEOUT`] once, and the row showed the helper
/// unavailable until the retry), while a helper the device already held
/// starts well inside [`HELLO_TIMEOUT`].
pub(super) const FIRST_HELLO_TIMEOUT: Duration = Duration::from_secs(60);

/// The bound for a connection's first Hello: [`FIRST_HELLO_TIMEOUT`] when
/// its install wrote the helper (`fresh`), [`HELLO_TIMEOUT`] otherwise.
pub(super) fn hello_timeout(fresh: bool) -> Duration {
    if fresh {
        FIRST_HELLO_TIMEOUT
    } else {
        HELLO_TIMEOUT
    }
}

/// A device test's Git stage: the node runs `git --version` over the link
/// the device has, read-only and with no login of its own (PRD
/// system-ssh-device-link B25). Answers whether it passed.
pub(super) fn check_git(
    link: &dyn hide_node_link::NodeLink,
    report: &mut hide_node_link::device::CapabilityReport,
) -> bool {
    use hide_node_link::device::RemoteStage;
    let asked = call_as::<String>(
        link,
        Call::Git {
            root: "/".to_owned(),
            command: hide_node_link::git::GitCommand::Version,
        },
        Duration::from_secs(10),
    );
    match asked {
        Ok(version) => {
            report.pass(RemoteStage::Git, version);
            true
        }
        Err(error) => {
            report.fail(
                RemoteStage::Git,
                format!("Git did not answer on the device: {error}"),
                true,
                true,
            );
            false
        }
    }
}

/// Local helpers, plus a manifest for device bundles fetched on connection.
#[derive(Clone, Debug, Default)]
pub struct HelperPackages {
    directory: Option<PathBuf>,
}

impl HelperPackages {
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self { directory }
    }

    /// The build for `os`/`arch` (Rust's names: `macos`, `aarch64`). A
    /// `hided-<os>-<arch>` file wins; the daemon's own program (`hided`,
    /// `hided.exe` on Windows) is accepted only for this daemon's own
    /// platform, which is what the package and a development build carry.
    pub fn find(&self, os: &str, arch: &str) -> Result<PathBuf, String> {
        self.find_program(HELPER_NAME, os, arch).map_err(|()| {
            format!(
                "This Hide build does not include the device helper for {} {arch}; files and Git on this device stay unavailable until a build that packages it is installed",
                platform_label(os)
            )
        })
    }

    /// What a device of `os`/`arch` gets: [`HelperPackages::payload`] with
    /// the pane holder beside the helper, which runs the device's panes for
    /// the core's own engine.
    pub(super) fn payload_for(&self, os: &str, arch: &str) -> Result<Payload, EstablishError> {
        // The holder goes beside the helper; a build that does not carry
        // the holder for the device's system connects nothing rather than a
        // node that could run no pane.
        let (mut payload, bundled) = self.kit(os, arch)?;
        // A release's device kit carries the holder for a device of
        // another system; a build's own system, or a developer's
        // folder, has it beside the helper.
        let holder = match bundled {
            Some(holder) => holder,
            None => {
                let holder = self.find_program(HOLDER_NAME, os, arch).map_err(|()| {
                    EstablishError::Unsupported(format!(
                        "This Hide build does not include the pane holder for {} {arch}; the core's own engine cannot run this device's panes",
                        platform_label(os)
                    ))
                })?;
                read_package(&device_program(HOLDER_NAME, os), &holder)
                    .map_err(EstablishError::Install)?
            }
        };
        payload.files.push(holder);
        // A Windows holder loads the pinned ConPTY runtime from the
        // folder beside the node's program (`hide_kit::holders`).
        if os == "windows" {
            let conpty = self
                .find_folder(hide_kit::holders::CONPTY_SOURCE, os, arch)
                .map_err(|()| {
                    EstablishError::Unsupported(format!(
                        "This Hide build does not include the pane holder's ConPTY runtime for {} {arch}; the core's own engine cannot run this device's panes",
                        platform_label(os)
                    ))
                })?;
            payload.files.extend(
                read_folder(hide_kit::holders::CONPTY_SOURCE, &conpty)
                    .map_err(EstablishError::Install)?,
            );
        }
        Ok(payload)
    }

    /// The programs this build puts on a device of `os`/`arch` besides the
    /// pane holder: the helper, which the device cannot do without, and the
    /// kit's parts, each found by the same rule as the helper. A part this
    /// build does not carry is left out and named, and the kit on the device
    /// reports it as missing from the build.
    pub(super) fn payload(&self, os: &str, arch: &str) -> Result<Payload, EstablishError> {
        self.kit(os, arch).map(|(payload, _)| payload)
    }

    /// [`HelperPackages::payload`], and the pane holder when it came with a
    /// release's device kit; a build's own files leave the holder unread
    /// until a device asks for it.
    fn kit(&self, os: &str, arch: &str) -> Result<(Payload, Option<Package>), EstablishError> {
        // Existing native and developer-provided artifacts keep their lookup
        // rule. Only a missing foreign helper goes to the release cache. This
        // runs on the connection worker, never under the runtime mutex, and
        // the bounded synchronous request cannot outlive that worker.
        if self.find_program(HELPER_NAME, os, arch).is_err()
            && (os != std::env::consts::OS || arch != std::env::consts::ARCH)
            && device_bundles::supported(os, arch)
        {
            return device_bundles::payload(self.directory.as_deref(), os, arch)
                .map(|(payload, holder)| (payload, Some(holder)))
                .map_err(|reason| EstablishError::Download {
                    os: os.to_owned(),
                    arch: arch.to_owned(),
                    reason,
                });
        }
        let helper = self.find(os, arch).map_err(EstablishError::Unsupported)?;
        let helper = read_package(&device_program(HELPER_NAME, os), &helper)
            .map_err(EstablishError::Install)?;
        let mut payload = Payload {
            files: vec![helper],
            missing: Vec::new(),
        };
        for name in [CLI_NAME, HOOKS_NAME] {
            match self.find_program(name, os, arch) {
                Ok(path) => match read_package(&device_program(name, os), &path) {
                    Ok(package) => payload.files.push(package),
                    Err(reason) => payload.missing.push(reason),
                },
                Err(()) => payload.missing.push(format!(
                    "This Hide build does not include {name} for {} {arch}",
                    platform_label(os)
                )),
            }
        }
        Ok((payload, None))
    }

    /// The program `name` for `os`/`arch`, by the helper's rule: this build's
    /// own copy carries its system's program name (`hided.exe` on Windows),
    /// as the package and a development build name it.
    fn find_program(&self, name: &str, os: &str, arch: &str) -> Result<PathBuf, ()> {
        self.find_named(name, &device_program(name, os), os, arch)
    }

    /// The folder `name` for `os`/`arch`, by the helper's rule: a folder has
    /// the same name on every system (`holder-conpty`).
    fn find_folder(&self, name: &str, os: &str, arch: &str) -> Result<PathBuf, ()> {
        self.find_named(name, name, os, arch)
    }

    /// A developer's `<name>-<os>-<arch>` first; for this daemon's own
    /// platform only, then the file or folder `own` beside it.
    fn find_named(&self, name: &str, own: &str, os: &str, arch: &str) -> Result<PathBuf, ()> {
        let directory = self.directory.as_ref().ok_or(())?;
        let named = directory.join(format!("{name}-{os}-{arch}"));
        if named.exists() {
            return Ok(named);
        }
        let own = directory.join(own);
        if os == std::env::consts::OS && arch == std::env::consts::ARCH && own.exists() {
            return Ok(own);
        }
        Err(())
    }
}

/// One file of a build, as it goes into the device's version folder.
struct Package {
    /// Its path under the version folder, `/`-separated.
    relative: String,
    bytes: Vec<u8>,
    digest: String,
    executable: bool,
}

/// What a build puts on one device.
pub(super) struct Payload {
    /// The helper first, then the kit's parts.
    files: Vec<Package>,
    /// Why a kit part is not among them.
    missing: Vec<String>,
}

impl Payload {
    /// The build's version: a digest over every file's path, mode and bytes,
    /// so any change to any part is a new folder on the device.
    fn version(&self) -> String {
        let mut entries = self
            .files
            .iter()
            .map(|file| {
                format!(
                    "{}\0{}\0{}\n",
                    file.relative,
                    if file.executable { "x" } else { "-" },
                    file.digest
                )
            })
            .collect::<Vec<_>>();
        entries.sort();
        hex_digest(entries.concat().as_bytes())
    }
}

/// A package keeps the mode bits it has here, which the device's copy gets.
/// A system without mode bits (Windows, which carries the Linux device kit
/// for its WSL distributions) has nothing to give it: there a package is
/// executable exactly when it is one of the kit's programs, by the kit's
/// contract, never by a guess from the file.
fn read_package(relative: &str, path: &Path) -> Result<Package, String> {
    use std::io::Read;
    let unreadable = |error: std::io::Error| {
        format!("The package {} could not be read: {error}", path.display())
    };
    let mut file = std::fs::File::open(path).map_err(unreadable)?;
    if !file.metadata().map_err(unreadable)?.is_file() {
        return Err(format!("The package {} is not a file", path.display()));
    }
    let executable = match hide_platform::fs::permissions::Permissions::of(&file)
        .map_err(unreadable)?
        .unix_mode()
    {
        Some(mode) => mode & 0o111 != 0,
        None => program_by_contract(relative).ok_or_else(|| {
            format!(
                "The package {} has no file mode on this system to install it with",
                path.display()
            )
        })?,
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(unreadable)?;
    Ok(Package {
        relative: relative.to_owned(),
        digest: hex_digest(&bytes),
        bytes,
        executable,
    })
}

/// Whether the kit part at `relative` is a program, for a system whose
/// files carry no mode; `None` for a name the kit does not define. A Windows
/// device's programs carry their `.exe`, and the ConPTY runtime's programs
/// are its `.exe` files.
fn program_by_contract(relative: &str) -> Option<bool> {
    if let Some(runtime) = relative
        .strip_prefix(hide_kit::holders::CONPTY_SOURCE)
        .and_then(|rest| rest.strip_prefix('/'))
    {
        return Some(runtime.ends_with(".exe"));
    }
    match relative.strip_suffix(".exe").unwrap_or(relative) {
        HELPER_NAME | CLI_NAME | HOOKS_NAME | HOLDER_NAME => Some(true),
        _ => None,
    }
}

/// Every file under the folder `dir`, as packages under `relative`, in name
/// order; anything but a file or a folder in it is refused.
fn read_folder(relative: &str, dir: &Path) -> Result<Vec<Package>, String> {
    let unreadable =
        |error: std::io::Error| format!("The package {} could not be read: {error}", dir.display());
    let mut entries = std::fs::read_dir(dir)
        .map_err(unreadable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(unreadable)?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    let mut packages = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            format!(
                "The package {} names a file that is not UTF-8",
                dir.display()
            )
        })?;
        let child = format!("{relative}/{name}");
        let kind = entry.file_type().map_err(unreadable)?;
        if kind.is_dir() {
            packages.extend(read_folder(&child, &entry.path())?);
        } else if kind.is_file() {
            packages.push(read_package(&child, &entry.path())?);
        } else {
            return Err(format!(
                "The package {} is not a file or a folder",
                entry.path().display()
            ));
        }
    }
    Ok(packages)
}

fn platform_label(os: &str) -> &str {
    match os {
        "macos" => "macOS",
        "linux" => "Linux",
        "windows" => "Windows",
        other => other,
    }
}

/// `uname -s -m` in Rust's platform names; a Windows device's PowerShell
/// script answers `Windows` and its processor as Windows names it (`amd64`,
/// `arm64`).
pub(super) fn platform_of(uname: &str) -> Result<(String, String), String> {
    let mut parts = uname.split_whitespace();
    let (Some(system), Some(machine)) = (parts.next(), parts.next()) else {
        return Err(format!(
            "The device reported an unreadable platform: {uname:?}"
        ));
    };
    let os = match system {
        "Darwin" => "macos",
        "Linux" => "linux",
        "Windows" => "windows",
        other => {
            return Err(format!(
                "The device runs {other}, which the helper does not support"
            ));
        }
    };
    let arch = match machine {
        "arm64" | "aarch64" => "aarch64",
        "x86_64" | "amd64" => "x86_64",
        other => {
            return Err(format!(
                "The device's processor {other} is not supported by the helper"
            ));
        }
    };
    if os == "macos" && arch == "x86_64" {
        return Err(
            "Intel Mac devices are not supported; Hide ships for Apple Silicon Macs only"
                .to_owned(),
        );
    }
    Ok((os.to_owned(), arch.to_owned()))
}

/// What a device's node sends without being asked: its panes' credential
/// proofs and `hide` command streams (`hide_node_link::panes`). hided hears
/// them; the core never does. Both calls come from the link's reader, on the
/// transport's runtime: an implementation hands the work to its own thread
/// and returns, and never calls the link from inside them.
pub trait PaneEvents: Send + Sync {
    /// `event` arrived on `link`, the link of the device `node`.
    fn event(&self, node: &str, link: &RemoteHost, event: NodeEvent);
    /// `link` ended: nothing it vouched for holds any more (B18).
    fn closed(&self, node: &str, link: &RemoteHost);
}

/// Where a connector's links send their pane events, filled once hided's
/// server exists; events before that find it empty and are refused.
pub type PaneEventsSlot = Arc<std::sync::OnceLock<Arc<dyn PaneEvents>>>;

/// Which device a link serves and where its pane events go.
#[derive(Clone)]
pub struct PaneHook {
    pub node: String,
    pub events: PaneEventsSlot,
}

/// Which device a link serves and where its terminals' output and reports
/// go on this side.
#[derive(Clone)]
pub struct TerminalHook {
    pub node: String,
    pub sink: Arc<dyn crate::terminal::device::DeviceSink>,
}

/// A live helper connection. Cloning shares it; the SSH connection ends when
/// the last clone is dropped or [`RemoteHost::close`] is called.
#[derive(Clone)]
pub struct RemoteHost {
    inner: Arc<Inner>,
}

impl fmt::Debug for RemoteHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteHost")
            .field("target", &self.inner.target)
            .finish_non_exhaustive()
    }
}

/// Admission to one helper connection: four running and thirty-two waiting
/// requests (PRD S5.5 D-15). Once the connection is draining or closed it
/// admits nothing new, and a request still waiting is refused rather than
/// sent, because nothing went out for it (B52). A link has three: one for
/// machine calls, one for link control (`Call::is_control`), one for
/// background Git work (`Call::is_background`) and one for the core's engine
/// asking the node's pane holders (`Call::is_pane`).
struct Gate {
    state: Mutex<Admission>,
    changed: Condvar,
    running: usize,
}

struct Admission {
    running: usize,
    queued: usize,
    stopped: Option<String>,
}

impl Gate {
    fn new(running: usize) -> Self {
        Self {
            state: Mutex::new(Admission {
                running: 0,
                queued: 0,
                stopped: None,
            }),
            changed: Condvar::new(),
            running,
        }
    }

    /// Admits one request, waiting for a slot until `deadline` at most: the
    /// call's own, which its write and its answer share.
    fn admit(&self, deadline: Instant) -> Result<(), LinkError> {
        let mut admission = lock_recover(&self.state);
        if let Some(reason) = &admission.stopped {
            return Err(LinkError::NotConnected(reason.clone()));
        }
        if admission.running < self.running {
            admission.running += 1;
            return Ok(());
        }
        if admission.queued >= MAX_QUEUED {
            return Err(LinkError::Busy);
        }
        admission.queued += 1;
        loop {
            if let Some(reason) = &admission.stopped {
                let reason = reason.clone();
                admission.queued -= 1;
                drop(admission);
                self.changed.notify_all();
                return Err(LinkError::NotConnected(reason));
            }
            if admission.running < self.running {
                break;
            }
            let now = Instant::now();
            if now >= deadline {
                admission.queued -= 1;
                return Err(LinkError::Busy);
            }
            admission = self
                .changed
                .wait_timeout(admission, deadline - now)
                .map(|(guard, _)| guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner().0);
        }
        admission.queued -= 1;
        admission.running += 1;
        Ok(())
    }

    fn release(&self) {
        lock_recover(&self.state).running -= 1;
        self.changed.notify_all();
    }

    /// Why nothing more is admitted, once it is not.
    fn stopped(&self) -> Option<String> {
        lock_recover(&self.state).stopped.clone()
    }

    /// Admits nothing more; the first reason given is the one kept.
    fn stop(&self, reason: &str) {
        let mut admission = lock_recover(&self.state);
        if admission.stopped.is_none() {
            admission.stopped = Some(reason.to_owned());
        }
        drop(admission);
        self.changed.notify_all();
    }

    /// Waits until no admitted or waiting request remains, or `deadline`.
    fn wait_idle(&self, deadline: Instant) {
        let mut admission = lock_recover(&self.state);
        while admission.running > 0 || admission.queued > 0 {
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            admission = self
                .changed
                .wait_timeout(admission, deadline - now)
                .map(|(guard, _)| guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner().0);
        }
    }
}

struct Inner {
    target: String,
    runtime: Arc<RemoteRuntime>,
    /// Wakes the reader to end the helper's channel: on [`RemoteHost::close`]
    /// and when the last clone is dropped. The device's connection stays,
    /// since other channels share it.
    closing: Arc<tokio::sync::Notify>,
    /// The link's place among the connection's session channels; a link
    /// over a local stream takes none.
    _session_channel: Option<tokio::sync::OwnedSemaphorePermit>,
    writer: tokio::sync::Mutex<Pin<Box<dyn AsyncWrite + Send>>>,
    pending: Mutex<HashMap<u64, Waiting>>,
    /// Why the connection ended, once it has; waiters subscribe to it
    /// rather than ask again ([`RemoteHost::closed`]).
    closed: tokio::sync::watch::Sender<Option<String>>,
    gate: Gate,
    /// Admission for link control, which never waits behind machine calls.
    control_gate: Gate,
    /// Admission for background Git work, which never takes a machine
    /// call's place.
    background_gate: Gate,
    /// Admission for the pane holders' requests, which never wait behind
    /// machine calls nor take their place.
    pane_gate: Gate,
    next_id: AtomicU64,
    /// Checkout roots this connection has opened, pinned to the directory
    /// they named then.
    roots: Mutex<HashMap<String, hide_node_link::RootIdentity>>,
    /// Where the node's terminal lines go once its terminal service started
    /// on this link (`crate::terminal::device`).
    terminals: std::sync::OnceLock<crate::terminal::device::LineHandler>,
    readers: std::sync::OnceLock<hide_node_link::sessions::ReaderFeatures>,
    /// The protocol the node's Hello named.
    protocol: std::sync::OnceLock<u32>,
    /// The streams to the node's own Herdr and its daemon's browser relay
    /// open on this link, for a node that dialed its core (`link_streams`).
    streams: link_streams::LinkStreams,
    /// The link runs over the attach role's local stream: a node that dialed
    /// this core, not a device it dialed.
    dialed_by_node: bool,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.closing.notify_one();
    }
}

impl Inner {
    fn gate_for(&self, call: &Call) -> &Gate {
        if call.is_control() {
            &self.control_gate
        } else if call.is_background() {
            &self.background_gate
        } else if call.is_pane() {
            &self.pane_gate
        } else {
            &self.gate
        }
    }

    /// Admits nothing more on any lane; the first reason given is kept.
    fn stop_gates(&self, reason: &str) {
        self.gate.stop(reason);
        self.control_gate.stop(reason);
        self.background_gate.stop(reason);
        self.pane_gate.stop(reason);
    }
}

impl RemoteHost {
    pub fn target(&self) -> &str {
        &self.inner.target
    }

    /// Whether `other` is this same connection, not a later one to the
    /// same device.
    pub fn same_link(&self, other: &RemoteHost) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// A number that names this connection while it lives, for keying what
    /// belongs to it.
    pub fn identity(&self) -> usize {
        Arc::as_ptr(&self.inner) as usize
    }

    /// A link with no device behind it: it has an identity and can be
    /// closed, and every call on it times out. For tests of what a link's
    /// identity decides, in this crate and in hided; a feature that compiled
    /// it only for them built hide-node and every crate above it twice.
    #[doc(hidden)]
    pub fn detached(target: &str) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a test runtime");
        let writer: Pin<Box<dyn AsyncWrite + Send>> = Box::pin(tokio::io::sink());
        RemoteHost {
            inner: Arc::new(Inner {
                target: target.to_owned(),
                runtime: Arc::new(RemoteRuntime(Some(runtime))),
                closing: Arc::new(tokio::sync::Notify::new()),
                _session_channel: None,
                writer: tokio::sync::Mutex::new(writer),
                pending: Mutex::new(HashMap::new()),
                closed: tokio::sync::watch::Sender::new(None),
                gate: Gate::new(MAX_RUNNING),
                control_gate: Gate::new(MAX_CONTROL_RUNNING),
                background_gate: Gate::new(MAX_BACKGROUND_RUNNING),
                pane_gate: Gate::new(MAX_PANE_RUNNING),
                next_id: AtomicU64::new(1),
                roots: Mutex::new(HashMap::new()),
                terminals: std::sync::OnceLock::new(),
                readers: std::sync::OnceLock::new(),
                protocol: std::sync::OnceLock::new(),
                streams: Default::default(),
                dialed_by_node: false,
            }),
        }
    }

    /// Whether a node dialed this core for this link (PRD
    /// core-host-node-remote-core D-07): the screen machine's node, which
    /// vouches for checkout callers as well as pane callers.
    pub fn dialed_by_node(&self) -> bool {
        self.inner.dialed_by_node
    }

    /// Why the connection ended, once it has.
    pub fn closed_reason(&self) -> Option<String> {
        self.inner.closed.borrow().clone()
    }

    /// Resolves with why the connection ended, as soon as it has. Holds no
    /// clone of the link, so waiting never keeps it alive.
    pub fn closed(&self) -> impl std::future::Future<Output = String> + Send + 'static {
        let mut closed = self.inner.closed.subscribe();
        async move {
            match closed.wait_for(Option::is_some).await {
                Ok(reason) => reason.clone().unwrap_or_default(),
                // Every clone of the link was dropped.
                Err(_) => "the link was dropped".to_owned(),
            }
        }
    }

    /// Ends the link; the helper exits when its channel closes. Requests
    /// still waiting for an answer become `Unknown`.
    pub fn close(&self, reason: &str) {
        mark_closed(&self.inner, reason.to_owned());
        self.inner.closing.notify_one();
    }

    /// Sends one request and waits at most `timeout` for its answer. Must
    /// not be called from inside an async context. The call has one
    /// deadline: waiting for a slot, waiting for the writer, the write and
    /// the answer all end by it, so a caller that gave up at `timeout` leaves
    /// no work of this call behind.
    pub fn call(&self, call: Call, timeout: Duration) -> Result<LinkAnswer, LinkError> {
        hide_node_link::link::check_reader_call(self, &call)?;
        let deadline = Instant::now() + timeout;
        let gate = self.inner.gate_for(&call);
        gate.admit(deadline)?;
        let result = self.send_and_wait(call, deadline, None);
        gate.release();
        result
    }

    /// [`RemoteHost::call`] for a call whose node reports before it
    /// answers: each report reaches `progress`, and one answered with false,
    /// or the timeout passing, asks the node to stop the call.
    pub fn call_with_progress(
        &self,
        call: Call,
        timeout: Duration,
        progress: &mut dyn FnMut(serde_json::Value) -> bool,
    ) -> Result<LinkAnswer, LinkError> {
        hide_node_link::link::check_reader_call(self, &call)?;
        let deadline = Instant::now() + timeout;
        let gate = self.inner.gate_for(&call);
        gate.admit(deadline)?;
        let result = self.send_and_wait(call, deadline, Some(progress));
        gate.release();
        result
    }

    fn send_and_wait(
        &self,
        call: Call,
        deadline: Instant,
        mut progress: Option<&mut dyn FnMut(serde_json::Value) -> bool>,
    ) -> Result<LinkAnswer, LinkError> {
        let inner = &self.inner;
        let id = inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        let reports = progress.is_some().then(Arc::default);
        lock_recover(&inner.pending).insert(
            id,
            Waiting {
                sender,
                reports: reports.clone(),
                until_stopped: call.runs_until_stopped(),
            },
        );
        self.send(id, call, deadline)?;
        let mut stopping = false;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(left) {
                Ok(Delivery::Report(raw)) => {
                    if let Some(waiting) = &reports {
                        waiting.fetch_sub(1, Ordering::AcqRel);
                    }
                    // Reports already on their way after a stop are not the
                    // caller's: it said it hears no more.
                    if stopping {
                        continue;
                    }
                    let go_on = match progress.as_mut() {
                        Some(progress) => serde_json::from_str(raw.get())
                            .map(&mut **progress)
                            .unwrap_or(false),
                        None => true,
                    };
                    if !go_on && !stopping {
                        stopping = true;
                        self.cancel(id);
                    }
                }
                Ok(Delivery::Answer(Ok(raw))) => return Ok(LinkAnswer::Raw(raw)),
                Ok(Delivery::Answer(Err(error))) => return Err(LinkError::Refused(error)),
                Ok(Delivery::Overflow) => {
                    self.cancel(id);
                    return Err(LinkError::Unknown(format!(
                        "The device reported faster than its {MAX_WAITING_REPORTS} waiting reports were read; the call was stopped"
                    )));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    lock_recover(&inner.pending).remove(&id);
                    if progress.is_some() {
                        self.cancel(id);
                    }
                    return Err(LinkError::Unknown(
                        "The device did not answer in time; the result is unknown and nothing was resent"
                            .to_owned(),
                    ));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(LinkError::Unknown(format!(
                        "The connection to the device ended before it answered ({}); the result is unknown and nothing was resent",
                        self.closed_reason()
                            .unwrap_or_else(|| "no reason reported".to_owned())
                    )));
                }
            }
        }
    }

    /// Asks the node to stop the reporting call `request`; its answer still
    /// ends the call. Nobody waits for the cancel's own answer.
    fn cancel(&self, request: u64) {
        let inner = &self.inner;
        let id = inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, _unheard) = mpsc::channel();
        lock_recover(&inner.pending).insert(
            id,
            Waiting {
                sender,
                reports: None,
                until_stopped: false,
            },
        );
        if let Err(error) = self.send(
            id,
            Call::Cancel { request },
            Instant::now() + CANCEL_TIMEOUT,
        ) {
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "host.cancel_unsent",
                "target": inner.target,
                "request": request,
                "error": error.to_string(),
            }));
        }
    }

    /// Writes request `id`, already waiting in `pending`; on any failure it
    /// is taken out again.
    fn send(&self, id: u64, call: Call, deadline: Instant) -> Result<(), LinkError> {
        let inner = &self.inner;
        if let Err(error) = hide_node_link::link::check_reader_call(self, &call) {
            lock_recover(&inner.pending).remove(&id);
            return Err(error);
        }
        // A cancel adds no effect on the device and is how a draining link
        // ends the reporting calls it waits for.
        let cancels = matches!(call, Call::Cancel { .. });
        let encoded = serde_json::to_vec(&Request { id, call });
        let mut line = match encoded {
            Ok(line) => line,
            Err(error) => {
                lock_recover(&inner.pending).remove(&id);
                return Err(LinkError::NotConnected(format!(
                    "The request could not be encoded: {error}"
                )));
            }
        };
        line.push(b'\n');
        let written = self.write_line(&line, deadline, cancels);
        if written.is_err() {
            lock_recover(&inner.pending).remove(&id);
        }
        written
    }

    /// The protocol the node's Hello named, once it has greeted the core.
    pub fn protocol(&self) -> Option<u32> {
        self.inner.protocol.get().copied()
    }

    /// Takes the protocol and reader facts the node's Hello advertised;
    /// once per link.
    pub(super) fn take_readers(
        &self,
        protocol: u32,
        readers: hide_node_link::sessions::ReaderFeatures,
    ) -> Result<(), String> {
        let _ = self.inner.protocol.set(protocol);
        self.inner
            .readers
            .set(readers)
            .map_err(|_| "The node's reader facts were already established".to_owned())
    }

    /// Hands what the node's terminal service sends on this link to
    /// `terminals`; a link takes one handler for its life.
    pub fn take_terminal_lines(&self, terminals: crate::terminal::device::LineHandler) -> bool {
        self.inner.terminals.set(terminals).is_ok()
    }

    /// Writes one whole line, which carries its newline, by `deadline`. A
    /// line that was started and not finished ends the connection, since the
    /// node would read the next line as its tail.
    fn write_line(&self, line: &[u8], deadline: Instant, cancels: bool) -> Result<(), LinkError> {
        let inner = &self.inner;
        if let Some(reason) = self.closed_reason() {
            return Err(LinkError::NotConnected(reason));
        }
        // Waiting behind another request's write sends nothing, so running
        // out of time there leaves the connection as it was; only a write
        // that started and did not finish can have sent part of a line.
        let deadline = tokio::time::Instant::from_std(deadline);
        let written = inner.runtime.block_on(async {
            let Ok(mut writer) = tokio::time::timeout_at(deadline, inner.writer.lock()).await
            else {
                return Err(LinkError::Busy);
            };
            // The writer came free as the deadline passed: nothing was sent,
            // and a write begun now could only be cut short.
            if tokio::time::Instant::now() >= deadline {
                return Err(LinkError::Busy);
            }
            // Admitted before the connection began to drain, but not sent:
            // it is refused now rather than sent after consent was withdrawn.
            if let Some(reason) = inner.gate.stopped().filter(|_| !cancels) {
                return Err(LinkError::NotConnected(reason));
            }
            Ok(tokio::time::timeout_at(deadline, async {
                writer.write_all(line).await?;
                writer.flush().await
            })
            .await)
        })?;
        match written {
            Ok(Ok(())) => Ok(()),
            // A write that failed or timed out may have sent part of the
            // line, and the next request would be read as its tail, so the
            // connection ends here; the next use reconnects.
            Ok(Err(error)) => {
                self.close("a request could not be written to the device helper");
                Err(LinkError::Unknown(format!(
                    "The connection to the device failed while the request was sent ({error}); its result is unknown"
                )))
            }
            Err(_) => {
                self.close("a request was not accepted by the device helper in time");
                Err(LinkError::Unknown(
                    "The device did not accept the request in time; its result is unknown"
                        .to_owned(),
                ))
            }
        }
    }
}

/// How long one terminal line may wait for the link's writer, after which
/// it is written again, and how long its write may take before the link is
/// ended as stalled.
const TERMINAL_LINE_TIMEOUT: Duration = Duration::from_secs(30);

impl crate::terminal::device::LineLink for RemoteHost {
    fn send_line(&self, line: &[u8]) -> Result<(), crate::terminal::device::LineRefused> {
        use crate::terminal::device::LineRefused;
        self.write_line(line, Instant::now() + TERMINAL_LINE_TIMEOUT, false)
            .map_err(|error| match error {
                LinkError::Busy => LineRefused::Busy,
                error => LineRefused::Ended(error.to_string()),
            })
    }

    fn end(&self, reason: &str) {
        self.close(reason);
    }
}

impl NodeLink for RemoteHost {
    fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
        if self.closed_reason().is_some() || self.inner.gate.stopped().is_some() {
            return None;
        }
        self.inner.readers.get()
    }

    fn predates_current_protocol(&self) -> bool {
        self.inner
            .protocol
            .get()
            .is_some_and(|protocol| *protocol < PROTOCOL_VERSION)
    }

    fn call(&self, call: Call, timeout: Duration) -> Result<LinkAnswer, LinkError> {
        RemoteHost::call(self, call, timeout)
    }

    fn call_with_progress(
        &self,
        call: Call,
        timeout: Duration,
        progress: &mut dyn FnMut(serde_json::Value) -> bool,
    ) -> Result<LinkAnswer, LinkError> {
        RemoteHost::call_with_progress(self, call, timeout, progress)
    }

    fn close_when_idle(&self, reason: &str) {
        let host = RemoteHost {
            inner: Arc::clone(&self.inner),
        };
        // Nothing new goes out from here, whoever still holds the channel;
        // a request waiting for a slot is refused, since nothing was sent.
        self.inner.stop_gates(reason);
        let drained = reason.to_owned();
        let spawned = std::thread::Builder::new()
            .name("remote-host-drain".into())
            .spawn(move || {
                // A Git watch runs until it is told to stop, so the drain
                // stops it rather than wait out its bound; a clone or any
                // other call with an effect settles to its own result.
                let reporting = lock_recover(&host.inner.pending)
                    .iter()
                    .filter(|(_, waiting)| waiting.until_stopped)
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                for request in reporting {
                    host.cancel(request);
                }
                // Admitted requests are bounded by their own timeouts; this
                // bound only keeps a wedged count from holding the link open.
                let deadline = Instant::now() + DRAIN_BOUND;
                host.inner.gate.wait_idle(deadline);
                host.inner.control_gate.wait_idle(deadline);
                host.inner.background_gate.wait_idle(deadline);
                host.inner.pane_gate.wait_idle(deadline);
                host.close(&drained);
            });
        if let Err(error) = spawned {
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "host.drain_unstarted",
                "target": self.inner.target,
                "error": error.to_string(),
            }));
            self.close(reason);
        }
    }

    fn closed_reason(&self) -> Option<String> {
        RemoteHost::closed_reason(self)
    }

    fn as_any(&self) -> Option<&(dyn std::any::Any + Send + Sync)> {
        Some(self)
    }

    fn close(&self, reason: &str) {
        RemoteHost::close(self, reason)
    }

    fn pinned(&self, root: &str) -> Option<hide_node_link::RootIdentity> {
        lock_recover(&self.inner.roots).get(root).copied()
    }

    fn pin(&self, root: &str, identity: Option<hide_node_link::RootIdentity>) {
        let mut roots = lock_recover(&self.inner.roots);
        match identity {
            Some(identity) => {
                roots.insert(root.to_owned(), identity);
            }
            None => {
                roots.remove(root);
            }
        }
    }
}

fn mark_closed(inner: &Inner, reason: String) {
    inner.closed.send_if_modified(|closed| {
        if closed.is_some() {
            return false;
        }
        *closed = Some(reason.clone());
        true
    });
    // Dropping the senders wakes every waiting request as disconnected.
    lock_recover(&inner.pending).clear();
    inner.streams.end_all();
    inner.stop_gates(&reason);
}

/// Why a line the device helper sent, whole or still arriving, is longer
/// than any the protocol allows: a terminal line past
/// [`hide_node_link::terminal::MAX_TERMINAL_LINE_BYTES`], an answer past
/// [`MAX_ANSWER_BYTES`]. The helper's lines are untrusted input, so such a
/// line ends the connection rather than this process's memory.
fn overlong_line(line: &[u8]) -> Option<String> {
    use hide_node_link::terminal::{MAX_TERMINAL_LINE_BYTES, TERMINAL_LINE_PREFIX};
    if line.starts_with(TERMINAL_LINE_PREFIX) {
        return (line.len() > MAX_TERMINAL_LINE_BYTES).then(|| {
            format!(
                "the device helper sent a terminal line longer than {} MiB",
                MAX_TERMINAL_LINE_BYTES / (1024 * 1024)
            )
        });
    }
    (line.len() > MAX_ANSWER_BYTES).then(|| {
        format!(
            "the device helper sent an answer longer than {} MiB",
            MAX_ANSWER_BYTES / (1024 * 1024)
        )
    })
}

/// B30: each step a device's connection reaches, so a connection that
/// stalls names the step it stalled in.
pub(super) fn establish_stage(target: &str, stage: &str, since: Instant) {
    crate::diagnostic!(json!({
        "component": "remote_host",
        "kind": "host.establish_stage",
        "target": target,
        "stage": stage,
        "elapsed_ms": since.elapsed().as_millis() as u64,
    }));
}

/// Starts the node's pane holders, the terminal service that shows them and
/// the pane service its panes' `hide` is proved on (`Call::HoldersStart`),
/// and the writer that carries the core's side of the terminals, for a
/// device whose panes the core's own engine runs (PRD
/// engine-bundle-3-core-owns D-22). A node that cannot start them has no
/// terminals, and the reason goes to the log.
pub(super) fn start_holders_at(
    host: &RemoteHost,
    hook: Option<TerminalHook>,
) -> Result<Arc<dyn hide_node_link::terminal::TerminalNode>, String> {
    let started = call_as::<PanesStarted>(host, Call::HoldersStart, HELLO_TIMEOUT)
        .map_err(|error| error.to_string())
        .and_then(|_| {
            let hook = hook.ok_or_else(|| "this process takes no device terminals".to_owned())?;
            crate::terminal::device::DeviceTerminals::start(
                &hook.node,
                Arc::new(host.clone()),
                hook.sink,
            )
            .map_err(|error| format!("the terminal writer could not start: {error}"))
        });
    match started {
        Ok(terminals) => {
            host.take_terminal_lines(terminals.inbound());
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "host.holders_started",
                "target": host.inner.target,
                "link": terminals.link(),
            }));
            Ok(Arc::new(terminals))
        }
        Err(reason) => {
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "host.holders_unstarted",
                "target": host.inner.target,
                "reason": reason,
            }));
            Err(reason)
        }
    }
}

/// Protocol24 has an audited transition contract with a frozen reader set.
/// Every other mismatch is refused before use; ordinary digest-based payload
/// replacement still installs the helper carried by this build.
pub(super) fn helper_protocol_refusal(hello: &Hello) -> Option<String> {
    (!matches!(hello.protocol, 24 | PROTOCOL_VERSION)).then(|| {
        format!(
            "The device helper speaks protocol {}, this Hide needs {PROTOCOL_VERSION}; the helper this Hide carries does not match it, so rebuild or reinstall Hide",
            hello.protocol
        )
    })
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl HelperPackages {
    /// A core update's install (`bootstrap::install_build`) on this
    /// machine: this build in its version folder
    /// under `root`, the folder a remote install of it would name, for a
    /// core update run where the core runs (PRD core-host-node-move Q13).
    /// A file already there is kept only when its bytes are this build's;
    /// any other is replaced whole. Answers where the build's `hided` is
    /// (`hided.exe` on Windows).
    pub fn install_here(&self, root: &Path) -> Result<PathBuf, String> {
        use hide_platform::fs::atomic::{self, DurableAccess};
        use hide_platform::fs::private;
        let payload = self
            .payload(std::env::consts::OS, std::env::consts::ARCH)
            .map_err(|error| error.to_string())?;
        if !payload.missing.is_empty() {
            return Err(format!(
                "This build cannot update a core without all of its parts: {}",
                payload.missing.join("; ")
            ));
        }
        let version_dir = root.join(&payload.version()[..16]);
        private::create_dir_all(&version_dir).map_err(|error| {
            format!(
                "The build folder {} could not be made: {error}",
                version_dir.display()
            )
        })?;
        for file in &payload.files {
            let path = version_dir.join(&file.relative);
            if std::fs::read(&path).is_ok_and(|bytes| hex_digest(&bytes) == file.digest) {
                continue;
            }
            let access = if file.executable {
                DurableAccess::PrivateExecutable
            } else {
                DurableAccess::Private
            };
            atomic::write_file_durable(&path, &file.bytes, access)
                .map_err(|error| format!("{} could not be written: {error}", path.display()))?;
        }
        Ok(version_dir.join(device_program(HELPER_NAME, std::env::consts::OS)))
    }
}

/// Where a link's lines come from: the pipes of the child that runs a
/// device's helper, or the bytes of a node that dialed this core, as its
/// attach role hands them over a local stream (PRD core-host-node-remote-core
/// D-04, D-10).
enum LinkSource {
    Stream(tokio::sync::mpsc::Receiver<Vec<u8>>),
    /// The stdout and stderr of a child this process owns, and what ends
    /// it ([`ChildLink`]): what its start script left unread comes first.
    Child {
        pending: Option<Vec<u8>>,
        stderr: Option<Vec<u8>>,
        ended: Option<String>,
        arrivals: tokio::sync::mpsc::Receiver<Arrival>,
        ending: ChildEnding,
    },
}

/// What one read of a link's source brought.
enum Arrival {
    Data(Vec<u8>),
    Stderr(Vec<u8>),
    /// A child's standard error reached its end.
    StderrClosed,
    Ended(String),
}

impl LinkSource {
    async fn next(&mut self) -> Arrival {
        match self {
            LinkSource::Stream(chunks) => match chunks.recv().await {
                Some(chunk) => Arrival::Data(chunk),
                None => Arrival::Ended("the node's attach stream closed".to_owned()),
            },
            LinkSource::Child {
                pending,
                stderr,
                ended,
                arrivals,
                ..
            } => {
                if let Some(data) = pending.take() {
                    return Arrival::Data(data);
                }
                if let Some(data) = stderr.take() {
                    return Arrival::Stderr(data);
                }
                if let Some(reason) = ended.take() {
                    return Arrival::Ended(reason);
                }
                arrivals
                    .recv()
                    .await
                    .unwrap_or_else(|| Arrival::Ended("the helper's output closed".to_owned()))
            }
        }
    }

    /// Ends what the link read from, so its far end reads the end of its
    /// input and exits.
    async fn finish(self) {
        match self {
            LinkSource::Stream(chunks) => drop(chunks),
            LinkSource::Child {
                arrivals, ending, ..
            } => {
                drop(arrivals);
                ending.end();
            }
        }
    }
}

/// A link that runs the same reader, calls and pane events as a device's
/// over a local byte stream: the attach role's, for a node that dialed this
/// core (PRD core-host-node-remote-core D-10). Its identity, credentials and
/// revocation are those of any link, whoever dialed. `target` names it in the
/// log; `stream` is closed when the link ends.
pub fn over_local_stream(
    target: &str,
    stream: hide_platform::ipc::LocalStream,
    panes: Option<PaneHook>,
    on_close: Box<dyn FnOnce(String) + Send + 'static>,
) -> std::io::Result<RemoteHost> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("node-link")
        .enable_all()
        .build()?;
    let runtime = Arc::new(RemoteRuntime(Some(runtime)));
    let shutdown = stream.shutdown_handle();
    let mut reading = stream.duplicate();
    let (chunks, arrivals) = tokio::sync::mpsc::channel::<Vec<u8>>(LOCAL_READ_CHUNKS);
    std::thread::Builder::new()
        .name("node-link-read".into())
        .spawn(move || {
            let mut buffer = vec![0u8; LOCAL_READ_BYTES];
            loop {
                match std::io::Read::read(&mut reading, &mut buffer) {
                    // A signal that cut the read short is no end of the link.
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Ok(0) | Err(_) => return,
                    Ok(read) => {
                        if chunks.blocking_send(buffer[..read].to_vec()).is_err() {
                            return;
                        }
                    }
                }
            }
        })?;
    let writer: Pin<Box<dyn AsyncWrite + Send>> = Box::pin(BlockingWriter::new(stream));
    // The read thread ends with the stream, whichever side ended it. The
    // stream is shut down as this closure goes, run or not: a link dropped
    // right after its close (a refused node) tears its runtime down before
    // the reader runs it, and the peer would otherwise never read the end.
    let shutdown = ShutdownOnDrop(shutdown);
    let ended = Box::new(move |reason: String| {
        drop(shutdown);
        on_close(reason);
    });
    Ok(start_reader(
        target.to_owned(),
        runtime,
        None,
        writer,
        LinkSource::Stream(arrivals),
        panes,
        ended,
    ))
}

/// How long a child whose input closed has to exit by itself before it is
/// ended: the helper reads the end of its input and returns at once, so
/// this is only the bound on one that does not.
const CHILD_EXIT_GRACE: Duration = Duration::from_secs(5);

/// A link over the stdin and stdout of a child this process started and
/// owns: the `wsl.exe` that runs a WSL distribution's helper (PRD wsl-device
/// D-07). See [`ChildLink`].
pub fn over_child(
    target: &str,
    child: hide_platform::process::OwnedChild,
    panes: Option<PaneHook>,
    on_close: Box<dyn FnOnce(String) + Send + 'static>,
) -> std::io::Result<RemoteHost> {
    Ok(ChildLink::start(target, child, None)?.into_host(Vec::new(), panes, on_close))
}

/// What one look at a [`ChildLink`] before it carries the node brought.
pub(super) enum ChildOutput {
    /// A whole line of its standard output, without the line end.
    Line(Vec<u8>),
    /// A whole line of its standard error, without the line end.
    Error(String),
    /// Nothing within the time given.
    Quiet,
    /// Its standard output ended.
    Ended(String),
}

/// The stdin, stdout and stderr of a child this process started and owns,
/// as one link: the `ssh` that runs a device's start script and then its
/// helper (PRD system-ssh-device-link D-02), or the `wsl.exe` that does the
/// same in a WSL distribution (PRD wsl-device D-07). The start script reads
/// and answers on it first ([`Self::next`], [`Self::send`]); then the same
/// pipes become the node's link ([`Self::into_host`]), which runs the same
/// reader, calls and pane events as any link, and keeps the child's standard
/// error as its diagnostic tail.
///
/// The link owns the child (engineering rule 14): when it ends, for any
/// reason, the child's input closes, so the helper reads its end and exits,
/// and a child still running after [`CHILD_EXIT_GRACE`] is ended with its
/// tree. A child that exits ends the link. Started for a connection attempt,
/// the child is that attempt's until it is reaped: its reaper holds a share
/// of the attempt, and the attempt's cancel ends it at once until the
/// transport disarms it (`Cancel`).
pub(super) struct ChildLink {
    target: String,
    runtime: Arc<RemoteRuntime>,
    arrivals: tokio::sync::mpsc::Receiver<Arrival>,
    input: Arc<Mutex<Option<Box<dyn std::io::Write + Send>>>>,
    ending: ChildEnding,
    /// Standard output read past the last whole line.
    stdout: Vec<u8>,
    /// Standard error read past the last whole line.
    stderr: Vec<u8>,
    stdout_ended: Option<String>,
    stderr_ended: bool,
}

/// The most of one line a child may write before the start script hands
/// over: its lines are short, and the bytes before it (a login file's
/// greeting) are skipped line by line.
const MAX_START_LINE: usize = 64 * 1024;

impl ChildLink {
    pub(super) fn start(
        target: &str,
        mut child: hide_platform::process::OwnedChild,
        attempt: Option<&hide_node_link::device::Cancel>,
    ) -> std::io::Result<Self> {
        let missing =
            |what: &str| std::io::Error::other(format!("the child's {what} is not piped"));
        let stdin = child.take_stdin().ok_or_else(|| missing("input"))?;
        let mut stdout = child.take_stdout().ok_or_else(|| missing("output"))?;
        let mut stderr = child.take_stderr().ok_or_else(|| missing("error output"))?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("node-link")
            .enable_all()
            .build()?;
        let runtime = Arc::new(RemoteRuntime(Some(runtime)));
        let (arrive, arrivals) = tokio::sync::mpsc::channel::<Arrival>(LOCAL_READ_CHUNKS);
        let stderr_arrive = arrive.clone();
        std::thread::Builder::new()
            .name("node-link-read".into())
            .spawn(move || {
                let mut buffer = vec![0u8; LOCAL_READ_BYTES];
                let ended = loop {
                    match std::io::Read::read(&mut stdout, &mut buffer) {
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Ok(0) => break "the helper's output closed".to_owned(),
                        Err(error) => break format!("the helper's output failed: {error}"),
                        Ok(read) => {
                            if arrive
                                .blocking_send(Arrival::Data(buffer[..read].to_vec()))
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                };
                let _ = arrive.blocking_send(Arrival::Ended(ended));
            })?;
        std::thread::Builder::new()
            .name("node-link-stderr".into())
            .spawn(move || {
                let mut buffer = vec![0u8; 4096];
                loop {
                    match std::io::Read::read(&mut stderr, &mut buffer) {
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            // A full queue drops diagnostic bytes, never a
                            // line of the link's own output.
                            if let Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) =
                                stderr_arrive.try_send(Arrival::Stderr(buffer[..read].to_vec()))
                            {
                                return;
                            }
                        }
                    }
                }
                let _ = stderr_arrive.try_send(Arrival::StderrClosed);
            })?;
        let input: Arc<Mutex<Option<Box<dyn std::io::Write + Send>>>> =
            Arc::new(Mutex::new(Some(Box::new(stdin))));
        let ending = ChildEnding::start(
            target,
            child,
            Arc::clone(&input),
            attempt.map(hide_node_link::device::Cancel::hold),
        )?;
        if let Some(attempt) = attempt {
            attempt.arm(ending.killer());
        }
        Ok(Self {
            target: target.to_owned(),
            runtime,
            arrivals,
            input,
            ending,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_ended: None,
            stderr_ended: false,
        })
    }

    /// The next whole line the child wrote on either output, or `Quiet` once
    /// `until` passes first.
    pub(super) fn next(&mut self, until: Instant) -> ChildOutput {
        loop {
            if let Some(line) = take_line(&mut self.stderr) {
                return ChildOutput::Error(String::from_utf8_lossy(&line).into_owned());
            }
            if let Some(line) = take_line(&mut self.stdout) {
                return ChildOutput::Line(line);
            }
            if self.stdout.len() > MAX_START_LINE {
                return ChildOutput::Ended(format!(
                    "the device wrote a line longer than {} KiB before its helper started",
                    MAX_START_LINE / 1024
                ));
            }
            if self.stderr.len() > MAX_START_LINE {
                let line = std::mem::take(&mut self.stderr);
                return ChildOutput::Error(String::from_utf8_lossy(&line).into_owned());
            }
            if let Some(reason) = self.stdout_ended.clone() {
                // What stderr still holds is read before the end is told,
                // so a failure is classified on all of what ssh said.
                if !self.stderr_ended {
                    let left = until.saturating_duration_since(Instant::now());
                    let bound = left.min(Duration::from_secs(2));
                    match self.recv(bound) {
                        Some(()) => continue,
                        None => self.stderr_ended = true,
                    }
                }
                if !self.stderr.is_empty() {
                    let line = std::mem::take(&mut self.stderr);
                    return ChildOutput::Error(String::from_utf8_lossy(&line).into_owned());
                }
                return ChildOutput::Ended(reason);
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return ChildOutput::Quiet;
            }
            if self.recv(left).is_none() {
                return ChildOutput::Quiet;
            }
        }
    }

    /// At most `max` bytes of the child's standard output, at least one, for
    /// a step that answers in counted bytes. What it writes on its error
    /// output meanwhile waits for [`Self::next`], the latest
    /// [`MAX_START_LINE`] bytes of it.
    pub(super) fn bytes(&mut self, max: usize, until: Instant) -> Result<Vec<u8>, bootstrap::Stop> {
        loop {
            if !self.stdout.is_empty() {
                let taken = self.stdout.len().min(max);
                return Ok(self.stdout.drain(..taken).collect());
            }
            if let Some(reason) = self.stdout_ended.clone() {
                return Err(bootstrap::Stop::Ended(reason));
            }
            if self.stderr.len() > MAX_START_LINE {
                let over = self.stderr.len() - MAX_START_LINE;
                self.stderr.drain(..over);
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() || self.recv(left).is_none() {
                return Err(bootstrap::Stop::Quiet);
            }
        }
    }

    /// Takes one arrival within `within` into the buffers; `None` when none
    /// came.
    fn recv(&mut self, within: Duration) -> Option<()> {
        let arrivals = &mut self.arrivals;
        let arrival = self
            .runtime
            .block_on(async { tokio::time::timeout(within, arrivals.recv()).await.ok() })?;
        match arrival {
            Some(Arrival::Data(data)) => self.stdout.extend_from_slice(&data),
            Some(Arrival::Stderr(data)) => self.stderr.extend_from_slice(&data),
            Some(Arrival::StderrClosed) => self.stderr_ended = true,
            Some(Arrival::Ended(reason)) => self.stdout_ended = Some(reason),
            None => {
                self.stdout_ended = Some("the helper's output closed".to_owned());
                self.stderr_ended = true;
            }
        }
        Some(())
    }

    /// Ends the child and its tree at once and returns once it is reaped, so
    /// an attempt that stops before its helper starts leaves nothing running.
    pub(super) fn kill(&mut self) {
        self.ending.kill();
    }

    /// Writes `bytes` to the child's input. A write the child does not take
    /// by `until` ends the child, which ends the write.
    pub(super) fn send(&mut self, bytes: &[u8], until: Instant) -> std::io::Result<()> {
        self.ending.arm(Some(until));
        let written = {
            let mut input = lock_recover(&self.input);
            match input.as_mut() {
                Some(input) => input.write_all(bytes).and_then(|()| input.flush()),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "the child's input closed",
                )),
            }
        };
        self.ending.arm(None);
        written
    }

    /// The node the child now runs: `pending` is what it already wrote past
    /// the start script's last line.
    pub(super) fn into_host(
        self,
        pending: Vec<u8>,
        panes: Option<PaneHook>,
        on_close: Box<dyn FnOnce(String) + Send + 'static>,
    ) -> RemoteHost {
        let Self {
            target,
            runtime,
            arrivals,
            input,
            ending,
            mut stdout,
            stderr,
            stdout_ended,
            ..
        } = self;
        let mut first = pending;
        first.append(&mut stdout);
        let writer: Pin<Box<dyn AsyncWrite + Send>> = Box::pin(BlockingWriter {
            stream: input,
            writing: None,
        });
        start_reader(
            target,
            runtime,
            None,
            writer,
            LinkSource::Child {
                pending: (!first.is_empty()).then_some(first),
                stderr: (!stderr.is_empty()).then_some(stderr),
                ended: stdout_ended,
                arrivals,
                ending,
            },
            panes,
            on_close,
        )
    }

    /// Takes what the child wrote past the start script's last line, for
    /// [`Self::into_host`].
    pub(super) fn take_rest(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.stdout)
    }

    /// The child's input and output as one local stream, for a peer this
    /// process speaks to itself (the attach role on a core's machine, PRD
    /// system-ssh-device-link B33): what is written to the stream reaches
    /// the child's input, and what the child writes is read from it, past
    /// the start script's last line. Its error output goes to `stderr`, the
    /// first [`CHILD_STDERR_KEPT`] bytes, for the diagnostic of a stream
    /// that ended early.
    ///
    /// The child's output ending ends the stream, and the stream's far end
    /// closing ends the child: its input closes, and its tree is ended after
    /// [`CHILD_EXIT_GRACE`], as for a link.
    pub(super) fn into_stream(
        self,
        stderr: Arc<Mutex<Vec<u8>>>,
    ) -> std::io::Result<hide_platform::ipc::LocalStream> {
        let Self {
            mut arrivals,
            input,
            ending,
            stdout,
            stderr: said,
            stdout_ended,
            ..
        } = self;
        let keep = move |bytes: &[u8]| {
            let mut kept = lock_recover(&stderr);
            let room = CHILD_STDERR_KEPT.saturating_sub(kept.len());
            kept.extend_from_slice(&bytes[..bytes.len().min(room)]);
        };
        keep(&said);
        let (ours, theirs) = hide_platform::ipc::LocalStream::pair()?;
        let ending = Arc::new(Mutex::new(Some(ending)));
        let end = {
            let ending = Arc::clone(&ending);
            move || {
                if let Some(ending) = lock_recover(&ending).take() {
                    ending.end();
                }
            }
        };
        let mut down = theirs.duplicate();
        let shutdown = theirs.shutdown_handle();
        let down_end = end.clone();
        std::thread::Builder::new()
            .name("child-stream-down".into())
            .spawn(move || {
                let mut written = stdout.is_empty() || down.write_all(&stdout).is_ok();
                if stdout_ended.is_some() {
                    written = false;
                }
                while written {
                    match arrivals.blocking_recv() {
                        Some(Arrival::Data(data)) => written = down.write_all(&data).is_ok(),
                        Some(Arrival::Stderr(data)) => keep(&data),
                        Some(Arrival::StderrClosed) => {}
                        Some(Arrival::Ended(_)) | None => break,
                    }
                }
                shutdown.shutdown();
                down_end();
            })?;
        let mut up = theirs;
        std::thread::Builder::new()
            .name("child-stream-up".into())
            .spawn(move || {
                let mut buffer = vec![0_u8; LOCAL_READ_BYTES];
                loop {
                    let read = match std::io::Read::read(&mut up, &mut buffer) {
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Ok(0) | Err(_) => break,
                        Ok(read) => read,
                    };
                    let sent = match lock_recover(&input).as_mut() {
                        Some(input) => input
                            .write_all(&buffer[..read])
                            .and_then(|()| input.flush())
                            .is_ok(),
                        None => false,
                    };
                    if !sent {
                        break;
                    }
                }
                up.shutdown_handle().shutdown();
                end();
            })?;
        Ok(ours)
    }
}

/// The most of a child's error output [`ChildLink::into_stream`] keeps.
pub(super) const CHILD_STDERR_KEPT: usize = 4096;

/// A start script's pipe on a child that asks for nothing while it starts
/// (the `wsl.exe` of a distribution): its error lines are kept, the last
/// few, for the failure they explain.
pub(super) struct UnaskedStart {
    pub(super) link: ChildLink,
    errors: std::collections::VecDeque<String>,
    cancel: hide_node_link::device::Cancel,
}

/// The error lines an [`UnaskedStart`] keeps.
const UNASKED_ERROR_TAIL: usize = 8;

impl UnaskedStart {
    pub(super) fn new(link: ChildLink, cancel: hide_node_link::device::Cancel) -> Self {
        Self {
            link,
            errors: std::collections::VecDeque::new(),
            cancel,
        }
    }

    /// The last line the child wrote on its error output that says anything.
    pub(super) fn last_error(&self) -> Option<&str> {
        self.errors
            .iter()
            .rev()
            .map(|line| line.trim())
            .find(|line| !line.is_empty())
    }
}

impl bootstrap::Pipe for UnaskedStart {
    fn send(&mut self, bytes: &[u8], until: Instant) -> Result<(), String> {
        if self.cancel.is_cancelled() {
            return Err(ATTEMPT_ENDED.to_owned());
        }
        self.link
            .send(bytes, until)
            .map_err(|error| error.to_string())
    }

    fn line(&mut self, until: Instant) -> Result<Vec<u8>, bootstrap::Stop> {
        loop {
            if self.cancel.is_cancelled() {
                return Err(bootstrap::Stop::Ended(ATTEMPT_ENDED.to_owned()));
            }
            // A second at a time, so a cancel is seen while it waits.
            let now = Instant::now();
            if now >= until {
                return Err(bootstrap::Stop::Quiet);
            }
            match self.link.next(until.min(now + Duration::from_secs(1))) {
                ChildOutput::Line(line) => return Ok(line),
                ChildOutput::Error(line) => {
                    if self.errors.len() == UNASKED_ERROR_TAIL {
                        self.errors.pop_front();
                    }
                    self.errors.push_back(line);
                }
                ChildOutput::Quiet => {}
                ChildOutput::Ended(reason) => return Err(bootstrap::Stop::Ended(reason)),
            }
        }
    }

    fn bytes(&mut self, max: usize, until: Instant) -> Result<Vec<u8>, bootstrap::Stop> {
        if self.cancel.is_cancelled() {
            return Err(bootstrap::Stop::Ended(ATTEMPT_ENDED.to_owned()));
        }
        self.link.bytes(max, until)
    }
}

/// Why an attempt the core cancelled stopped.
pub(super) const ATTEMPT_ENDED: &str = "The connection attempt was ended";

/// Takes the first whole line out of `buffer`, without its end.
fn take_line(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let end = buffer.iter().position(|byte| *byte == b'\n')?;
    let mut line: Vec<u8> = buffer.drain(..=end).collect();
    line.pop();
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Some(line)
}

/// What ends the child of a [`ChildLink`]: told once, from the link's end or
/// its drop, a thread closes the child's input and waits out
/// [`CHILD_EXIT_GRACE`] for it before ending its tree. The clock starts when
/// it is told: a write blocked on a child that stopped reading holds the
/// input until the child ends, so the input closes once no write holds it,
/// and ending the tree is what frees such a write. While a start script's
/// write is armed with a deadline, passing it ends the tree at once.
struct ChildEnding {
    /// Shared only with the killer an attempt's cancel holds, as a weak
    /// reference, so the link's end is still every sender gone.
    told: Option<Arc<mpsc::SyncSender<Told>>>,
    /// Set by the killer: the thread ends the child at its next look, also
    /// when the queue was too full to take the word.
    killed: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

enum Told {
    End,
    /// End the child at once, without its grace.
    Kill,
    Arm(Option<Instant>),
}

impl ChildEnding {
    fn start(
        target: &str,
        mut child: hide_platform::process::OwnedChild,
        input: Arc<Mutex<Option<Box<dyn std::io::Write + Send>>>>,
        held: Option<hide_node_link::device::Held>,
    ) -> std::io::Result<Self> {
        let (told, heard) = mpsc::sync_channel::<Told>(4);
        let killed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let target = target.to_owned();
        let thread_killed = Arc::clone(&killed);
        let thread = std::thread::Builder::new()
            .name("node-link-child".into())
            .spawn(move || {
                let killed = || thread_killed.load(std::sync::atomic::Ordering::SeqCst);
                // Told, every sender gone, or an armed write's deadline
                // passed: either way the link is over.
                let mut armed: Option<Instant> = None;
                let mut kill = false;
                let late = loop {
                    if killed() {
                        kill = true;
                        break false;
                    }
                    let heard = match armed {
                        None => heard
                            .recv()
                            .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                        Some(until) => {
                            heard.recv_timeout(until.saturating_duration_since(Instant::now()))
                        }
                    };
                    match heard {
                        Ok(Told::Arm(until)) => armed = until,
                        Ok(Told::Kill) => {
                            kill = true;
                            break false;
                        }
                        Ok(Told::End) | Err(mpsc::RecvTimeoutError::Disconnected) => break false,
                        Err(mpsc::RecvTimeoutError::Timeout) => break true,
                    }
                };
                let pid = child.id();
                let since = Instant::now();
                let mut input_open = true;
                let mut status = None;
                while !late && !kill && !killed() && since.elapsed() < CHILD_EXIT_GRACE {
                    if input_open {
                        input_open = !close_unless_writing(&input);
                    }
                    match child.try_wait() {
                        Ok(Some(exited)) => {
                            status = Some(exited);
                            break;
                        }
                        Ok(None) => std::thread::park_timeout(Duration::from_millis(50)),
                        Err(_) => break,
                    }
                }
                let killed = status.is_none();
                // A tied child's status comes only with its whole tree
                // confirmed gone; an error here is an end nothing confirmed.
                let ended = if killed {
                    child.kill_tree().and_then(|_| child.wait()).map(Some)
                } else {
                    Ok(status)
                };
                if input_open {
                    close_unless_writing(&input);
                }
                match ended {
                    Ok(status) => {
                        crate::diagnostic!(json!({
                            "component": "remote_host",
                            "kind": "host.child_ended",
                            "target": target,
                            "pid": pid,
                            "code": status.and_then(|status| status.code()),
                            "killed": killed,
                            "write_deadline_passed": late,
                            "after_ms": since.elapsed().as_millis() as u64,
                        }));
                        // The attempt this child was started for has
                        // nothing of it left running only now.
                        drop(held);
                    }
                    Err(error) => {
                        crate::diagnostic!(json!({
                            "component": "remote_host",
                            "kind": "host.child_end_unconfirmed",
                            "target": target,
                            "pid": pid,
                            "killed": killed,
                            "error": error.to_string(),
                            "after_ms": since.elapsed().as_millis() as u64,
                        }));
                        // Something of the child's tree may still run, so
                        // its attempt never counts as ended.
                        if let Some(held) = held {
                            held.outlived();
                        }
                    }
                }
            })?;
        Ok(Self {
            told: Some(Arc::new(told)),
            killed,
            thread: Some(thread),
        })
    }

    /// What an attempt's cancel runs: tells the thread to end the child now,
    /// without waiting, and keeps nothing of the link alive.
    fn killer(&self) -> impl Fn() + Send + 'static {
        let told = self.told.as_ref().map(Arc::downgrade);
        let killed = Arc::clone(&self.killed);
        move || {
            killed.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(told) = told.as_ref().and_then(std::sync::Weak::upgrade) {
                let _ = told.try_send(Told::Kill);
            }
        }
    }

    /// Ends the child and its tree now and returns once it is reaped.
    fn kill(&mut self) {
        self.killed.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(told) = self.told.take() {
            let _ = told.send(Told::Kill);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    fn arm(&self, until: Option<Instant>) {
        if let Some(told) = &self.told {
            let _ = told.send(Told::Arm(until));
        }
    }

    fn end(mut self) {
        if let Some(told) = self.told.take() {
            let _ = told.try_send(Told::End);
        }
    }
}

/// Closes a child link's input unless a write holds it; answers whether it
/// is closed.
fn close_unless_writing(input: &Mutex<Option<Box<dyn std::io::Write + Send>>>) -> bool {
    match input.try_lock() {
        Ok(mut held) => {
            held.take();
            true
        }
        Err(std::sync::TryLockError::Poisoned(poisoned)) => {
            poisoned.into_inner().take();
            true
        }
        Err(std::sync::TryLockError::WouldBlock) => false,
    }
}

/// Shuts a local link's stream down when it goes.
struct ShutdownOnDrop(hide_platform::ipc::ShutdownHandle);

impl Drop for ShutdownOnDrop {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

/// Chunks a local link's read thread holds before the reader takes them; a
/// reader that falls behind holds the thread, never memory.
const LOCAL_READ_CHUNKS: usize = 16;
const LOCAL_READ_BYTES: usize = 64 * 1024;

/// A blocking local stream as the link's writer: each write runs on the
/// runtime's blocking pool, so a stalled peer holds a pool thread, never the
/// runtime, and the link's own write timeout still ends the wait.
struct BlockingWriter {
    /// Empty once the link ended its child's input ([`over_child`]).
    stream: Arc<std::sync::Mutex<Option<Box<dyn std::io::Write + Send>>>>,
    writing: Option<tokio::task::JoinHandle<std::io::Result<usize>>>,
}

impl BlockingWriter {
    fn new(stream: hide_platform::ipc::LocalStream) -> Self {
        Self {
            stream: Arc::new(std::sync::Mutex::new(Some(Box::new(stream)))),
            writing: None,
        }
    }
}

impl AsyncWrite for BlockingWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        use std::task::Poll;
        if self.writing.is_none() {
            let stream = Arc::clone(&self.stream);
            let bytes = buffer.to_vec();
            self.writing = Some(tokio::task::spawn_blocking(move || {
                let mut stream = lock_recover(&stream);
                let stream = stream.as_mut().ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the link's input closed")
                })?;
                std::io::Write::write_all(stream, &bytes)?;
                Ok(bytes.len())
            }));
        }
        let writing = self.writing.as_mut().expect("a write in flight");
        match Pin::new(writing).poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                self.writing = None;
                Poll::Ready(result.unwrap_or_else(|error| Err(std::io::Error::other(error))))
            }
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/// The link's reader, the same for every source: lines are framed, sorted
/// into terminal lines, pane events, reports and answers, and the first
/// reason to stop ends the link.
fn start_reader(
    target: String,
    runtime: Arc<RemoteRuntime>,
    session_channel: Option<tokio::sync::OwnedSemaphorePermit>,
    writer: Pin<Box<dyn AsyncWrite + Send>>,
    mut source: LinkSource,
    panes: Option<PaneHook>,
    on_close: Box<dyn FnOnce(String) + Send + 'static>,
) -> RemoteHost {
    let closing = Arc::new(tokio::sync::Notify::new());
    let dialed_by_node = matches!(source, LinkSource::Stream(_));
    let inner = Arc::new(Inner {
        target,
        runtime: Arc::clone(&runtime),
        closing: Arc::clone(&closing),
        _session_channel: session_channel,
        writer: tokio::sync::Mutex::new(writer),
        pending: Mutex::new(HashMap::new()),
        closed: tokio::sync::watch::Sender::new(None),
        gate: Gate::new(MAX_RUNNING),
        control_gate: Gate::new(MAX_CONTROL_RUNNING),
        background_gate: Gate::new(MAX_BACKGROUND_RUNNING),
        pane_gate: Gate::new(MAX_PANE_RUNNING),
        next_id: AtomicU64::new(1),
        roots: Mutex::new(HashMap::new()),
        terminals: std::sync::OnceLock::new(),
        readers: std::sync::OnceLock::new(),
        protocol: std::sync::OnceLock::new(),
        streams: Default::default(),
        dialed_by_node,
    });
    let reader = Arc::downgrade(&inner);
    runtime.spawn(async move {
        let mut buffer: Vec<u8> = Vec::new();
        // How far `buffer` has been searched for a line end, so a large
        // answer arriving in many chunks is scanned once.
        let mut scanned = 0;
        let mut stderr: Vec<u8> = Vec::new();
        let reason = 'read: loop {
            let arrival = tokio::select! {
                arrival = source.next() => arrival,
                () = closing.notified() => break "this Hide closed the link".to_owned(),
            };
            match arrival {
                Arrival::Data(data) => {
                    buffer.extend_from_slice(&data);
                    while let Some(offset) = buffer[scanned..].iter().position(|byte| *byte == b'\n') {
                        let end = scanned + offset;
                        scanned = 0;
                        let line: Vec<u8> = buffer.drain(..=end).collect();
                        let Some(inner) = reader.upgrade() else { return };
                        if let Some(reason) = overlong_line(&line) {
                            break 'read reason;
                        }
                        deliver_line(&inner, panes.as_ref(), &line);
                    }
                    scanned = buffer.len();
                    if let Some(reason) = overlong_line(&buffer) {
                        break reason;
                    }
                }
                Arrival::Stderr(data) => {
                    stderr.extend_from_slice(&data);
                    if stderr.len() > 4096 {
                        stderr.drain(..stderr.len() - 4096);
                    }
                }
                Arrival::StderrClosed => {}
                Arrival::Ended(reason) => break reason,
            }
        };
        // Whatever ended the loop, the source ends with it, so the node
        // reads the end of its input and exits.
        source.finish().await;
        if let Some(inner) = reader.upgrade() {
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "host.closed",
                "target": inner.target,
                "reason": reason,
                "stderr_tail": String::from_utf8_lossy(&stderr).chars().rev().take(400).collect::<String>().chars().rev().collect::<String>(),
            }));
            mark_closed(&inner, reason.clone());
            if let Some(hook) = &panes
                && let Some(events) = hook.events.get()
            {
                events.closed(&hook.node, &RemoteHost { inner });
            }
        }
        on_close(reason);
    });
    RemoteHost { inner }
}

/// Sorts one whole line from the node: a terminal line, a pane event, a
/// call's report, or an answer.
fn deliver_line(inner: &Arc<Inner>, panes: Option<&PaneHook>, line: &[u8]) {
    if line.starts_with(hide_node_link::terminal::TERMINAL_LINE_PREFIX) {
        match inner.terminals.get() {
            Some(terminals) => terminals(line),
            None => crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "host.terminal_line_unheard",
                "target": inner.target,
                "bytes": line.len(),
            })),
        }
        return;
    }
    if line.starts_with(b"{\"event\":\"ping\"}") {
        return;
    }
    if line.starts_with(b"{\"event\":\"link_data\"")
        || line.starts_with(b"{\"event\":\"link_closed\"")
    {
        deliver_link(inner, line);
        return;
    }
    if line.starts_with(b"{\"event\":") {
        deliver_event(inner, panes, line);
        return;
    }
    if line.starts_with(b"{\"progress\":") {
        deliver_report(inner, line);
        return;
    }
    // The answer is only tokenized here and kept as its own text; one nobody
    // waits for is never copied.
    match serde_json::from_slice::<AnswerLine<'_>>(line) {
        Ok(answer) => {
            let outcome = match (answer.ok, answer.error) {
                (Some(raw), None) => Some(Ok(raw)),
                (None, Some(error)) => Some(Err(error)),
                _ => None,
            };
            let sender = lock_recover(&inner.pending)
                .remove(&answer.id)
                .map(|waiting| waiting.sender);
            match (sender, outcome) {
                (Some(sender), Some(outcome)) => {
                    let _ = sender.send(Delivery::Answer(outcome.map(ToOwned::to_owned)));
                }
                (sender, _) => crate::diagnostic!(json!({
                    "component": "remote_host",
                    "kind": "host.answer_unmatched",
                    "target": inner.target,
                    "id": answer.id,
                    "awaited": sender.is_some(),
                    "bytes": line.len(),
                })),
            }
        }
        // serde's own text can quote the value it could not read, which may
        // be file contents, so only its class and position are logged (S5.5
        // B48).
        Err(error) => crate::diagnostic!(json!({
            "component": "remote_host",
            "kind": "host.answer_unreadable",
            "target": inner.target,
            "class": format!("{:?}", error.classify()),
            "line": error.line(),
            "column": error.column(),
            "bytes": line.len(),
        })),
    }
}

/// Hands one event line to hided's pane events. A node that sends events
/// on a link nobody listens to, or a line that is not one, is logged and
/// dropped; the requests on the link go on.
/// Hands a report to the call that hears it. A call that does not hear
/// reports, or one gone, drops it; one with [`MAX_WAITING_REPORTS`] unread
/// is told it overflowed and hears nothing more.
fn deliver_report(inner: &Arc<Inner>, line: &[u8]) {
    let report = match serde_json::from_slice::<ProgressLine<'_>>(line) {
        Ok(report) => report,
        Err(error) => {
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "host.report_unreadable",
                "target": inner.target,
                "class": format!("{:?}", error.classify()),
                "bytes": line.len(),
            }));
            return;
        }
    };
    let mut pending = lock_recover(&inner.pending);
    let Some(waiting) = pending.get(&report.progress) else {
        return;
    };
    let Some(unread) = &waiting.reports else {
        return;
    };
    if unread.fetch_add(1, Ordering::AcqRel) >= MAX_WAITING_REPORTS {
        if let Some(waiting) = pending.remove(&report.progress) {
            let _ = waiting.sender.send(Delivery::Overflow);
        }
        crate::diagnostic!(json!({
            "component": "remote_host",
            "kind": "host.reports_overflowed",
            "target": inner.target,
            "request": report.progress,
            "cap": MAX_WAITING_REPORTS,
        }));
        return;
    }
    let _ = waiting
        .sender
        .send(Delivery::Report(report.report.to_owned()));
}

/// Hands what the node read from a stream's end to the stream the core
/// opened (`link_streams`); these never reach hided's pane events.
fn deliver_link(inner: &Arc<Inner>, line: &[u8]) {
    match serde_json::from_slice::<NodeEvent>(line) {
        Ok(NodeEvent::LinkData { stream, data }) => {
            inner.streams.data(&inner.target, stream, &data);
        }
        Ok(NodeEvent::LinkClosed { stream, reason }) => {
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "link_stream.closed_by_node",
                "target": inner.target,
                "stream": stream,
                "end": inner.streams.end_of(stream),
                "reason": reason.chars().take(64).collect::<String>(),
            }));
            inner.streams.end(stream, "the node ended the stream");
        }
        Ok(_) | Err(_) => crate::diagnostic!(json!({
            "component": "remote_host",
            "kind": "host.event_unreadable",
            "target": inner.target,
            "bytes": line.len(),
        })),
    }
}

fn deliver_event(inner: &Arc<Inner>, panes: Option<&PaneHook>, line: &[u8]) {
    let event = match serde_json::from_slice::<NodeEvent>(line) {
        Ok(event) => event,
        Err(error) => {
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "host.event_unreadable",
                "target": inner.target,
                "class": format!("{:?}", error.classify()),
                "bytes": line.len(),
            }));
            return;
        }
    };
    match panes.and_then(|hook| Some((hook, hook.events.get()?))) {
        Some((hook, events)) => events.event(
            &hook.node,
            &RemoteHost {
                inner: Arc::clone(inner),
            },
            event,
        ),
        None => crate::diagnostic!(json!({
            "component": "remote_host",
            "kind": "host.event_unheard",
            "target": inner.target,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first Hello after the connection's install wrote the helper
    /// waits long enough for a scanned first start, past the bound a helper
    /// the device already held gets; it is a fixed bound, not a retry.
    #[test]
    fn a_freshly_installed_helper_gets_a_longer_first_hello() {
        assert_eq!(hello_timeout(false), Duration::from_secs(15));
        assert_eq!(hello_timeout(true), Duration::from_secs(60));
        assert!(hello_timeout(true) > hello_timeout(false));
    }

    /// A device's terminal line past its cap ends the link, and an answer
    /// keeps its own, larger cap: a device's node is another machine's
    /// program.
    /// Waiting for a link's end is told the reason as soon as the link
    /// closes, and that the link is gone once every clone was dropped,
    /// with no look again in between.
    #[test]
    fn a_link_s_end_wakes_its_waiters_at_once() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("a runtime");
        let link = RemoteHost::detached("ssh:ending");
        let closed = link.closed();
        link.close("this Hide closed the link");
        let reason = runtime
            .block_on(async { tokio::time::timeout(Duration::from_millis(100), closed).await })
            .expect("woken at once");
        assert_eq!(reason, "this Hide closed the link");
        let dropped = RemoteHost::detached("ssh:dropped");
        let closed = dropped.closed();
        drop(dropped);
        let reason = runtime
            .block_on(async { tokio::time::timeout(Duration::from_millis(100), closed).await })
            .expect("woken at once");
        assert_eq!(reason, "the link was dropped");
    }

    #[test]
    fn a_terminal_line_past_its_cap_ends_the_link_and_an_answer_keeps_its_own() {
        use hide_node_link::terminal::{MAX_TERMINAL_LINE_BYTES, TERMINAL_LINE_PREFIX};
        let mut line = TERMINAL_LINE_PREFIX.to_vec();
        line.resize(MAX_TERMINAL_LINE_BYTES, b'a');
        assert_eq!(overlong_line(&line), None);
        line.push(b'a');
        assert!(
            overlong_line(&line)
                .is_some_and(|reason| reason.contains("terminal line longer than 8 MiB"))
        );
        let answer = vec![b'a'; MAX_TERMINAL_LINE_BYTES + 1];
        assert_eq!(overlong_line(&answer), None);
    }

    #[derive(Default)]
    struct Heard(Mutex<Vec<(String, NodeEvent)>>);

    impl PaneEvents for Heard {
        fn event(&self, node: &str, _link: &RemoteHost, event: NodeEvent) {
            lock_recover(&self.0).push((node.to_owned(), event));
        }

        fn closed(&self, _node: &str, _link: &RemoteHost) {}
    }

    /// A call that leaves reports unread past the cap hears that it
    /// overflowed and nothing more, and a call that hears no reports drops
    /// them; neither grows without bound.
    #[test]
    fn reports_past_the_cap_end_the_call_as_overflowed() {
        let link = RemoteHost::detached("ssh:reports");
        let (sender, receiver) = mpsc::channel();
        let unread = Arc::new(AtomicUsize::new(MAX_WAITING_REPORTS - 1));
        lock_recover(&link.inner.pending).insert(
            7,
            Waiting {
                sender,
                reports: Some(Arc::clone(&unread)),
                until_stopped: false,
            },
        );
        let line = br#"{"progress":7,"report":{"n":1}}"#;
        deliver_report(&link.inner, line);
        assert!(matches!(receiver.try_recv(), Ok(Delivery::Report(_))));
        deliver_report(&link.inner, line);
        assert!(matches!(receiver.try_recv(), Ok(Delivery::Overflow)));
        assert!(!lock_recover(&link.inner.pending).contains_key(&7));
        deliver_report(&link.inner, line);
        assert!(receiver.try_recv().is_err(), "nothing after the overflow");

        let (sender, receiver) = mpsc::channel();
        lock_recover(&link.inner.pending).insert(
            8,
            Waiting {
                sender,
                reports: None,
                until_stopped: false,
            },
        );
        deliver_report(&link.inner, br#"{"progress":8,"report":null}"#);
        assert!(
            receiver.try_recv().is_err(),
            "a plain call hears no reports"
        );
    }

    /// Letter-720: a node cannot speak for another node's panes. The node an
    /// event belongs to is the link it came up, never a field of the line,
    /// so a line naming another node reaches the core as this link's.
    #[test]
    fn a_link_s_events_carry_its_own_node_whatever_the_line_says() {
        let link = RemoteHost::detached("ssh:device-a");
        let heard = Arc::new(Heard::default());
        let slot: PaneEventsSlot = Arc::default();
        let _ = slot.set(Arc::clone(&heard) as Arc<dyn PaneEvents>);
        let hook = PaneHook {
            node: "node-a".to_owned(),
            events: slot,
        };
        let line = json!({
            "event": "pane_proof", "request": 7, "pane_id": "w1:p1",
            "terminal_id": "t1", "shell_pid": 42, "shell_started": 9,
            "nonce": "n", "one_shot": false,
            "node": "node-b", "device_id": "node-b",
        });
        deliver_event(&link.inner, Some(&hook), line.to_string().as_bytes());
        let heard = lock_recover(&heard.0);
        assert_eq!(heard.len(), 1);
        assert_eq!(heard[0].0, "node-a");
        assert!(matches!(&heard[0].1, NodeEvent::PaneProof { pane_id, .. } if pane_id == "w1:p1"));
    }

    /// A link over a local stream, the one a node that dialed the core
    /// arrives on (PRD core-host-node-remote-core D-10), answers calls,
    /// carries the node's events as its own, and ends with its stream: the
    /// node closing it ends the link, and the link closing ends the stream.
    #[test]
    fn a_link_over_a_local_stream_answers_and_ends_with_its_stream() {
        use std::io::{BufRead, Write};
        let (core_end, node_end) = hide_platform::ipc::LocalStream::pair().expect("a stream pair");
        let heard = Arc::new(Heard::default());
        let slot: PaneEventsSlot = Arc::default();
        let _ = slot.set(Arc::clone(&heard) as Arc<dyn PaneEvents>);
        let (closed, ended) = mpsc::channel();
        let link = over_local_stream(
            "inbound:node-a",
            core_end,
            Some(PaneHook {
                node: "node-a".to_owned(),
                events: slot,
            }),
            Box::new(move |reason| {
                let _ = closed.send(reason);
            }),
        )
        .expect("a local link");
        // The core vouches for a checkout caller only on such a link.
        assert!(link.dialed_by_node());
        assert!(!RemoteHost::detached("device").dialed_by_node());
        let node = std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(node_end.duplicate());
            let mut writer = node_end;
            let mut line = String::new();
            reader.read_line(&mut line).expect("a request");
            let request: serde_json::Value = serde_json::from_str(&line).expect("a request line");
            let event = json!({"event": "revoke", "token": "t"});
            writeln!(writer, "{event}").expect("an event");
            let answer = json!({"id": request["id"], "ok": {"echo": request["call"]["call"]}});
            writeln!(writer, "{answer}").expect("an answer");
            // The node ends its side; the link reads the end.
            drop(writer);
            drop(reader);
        });
        let answer = link
            .call(Call::HookDiagnosis, Duration::from_secs(10))
            .expect("an answer");
        let LinkAnswer::Raw(raw) = answer else {
            panic!("a raw answer")
        };
        assert!(raw.get().contains("echo"), "{}", raw.get());
        node.join().expect("the node thread");
        let reason = ended
            .recv_timeout(Duration::from_secs(10))
            .expect("the link ends with its stream");
        assert!(reason.contains("attach stream closed"), "{reason}");
        assert!(link.closed_reason().is_some());
        let heard = lock_recover(&heard.0);
        assert_eq!(heard.len(), 1);
        assert_eq!(heard[0].0, "node-a");
        assert!(matches!(&heard[0].1, NodeEvent::Revoke { token } if token == "t"));
    }

    /// Remote audit R2: a call has one deadline. A call that waited for a
    /// slot, or for the writer behind another request, has only what is left
    /// of it for its answer, so its work ends when its caller gives up, not a
    /// whole timeout after it was written.
    #[test]
    #[allow(clippy::disallowed_methods)] // the held slot and the held writer are the subject: each is let go late, as on a busy link
    fn a_call_s_waits_and_its_answer_share_one_deadline() {
        use std::io::BufRead;
        const TIMEOUT: Duration = Duration::from_secs(2);
        const HELD: Duration = Duration::from_secs(1);
        let (core_end, node_end) = hide_platform::ipc::LocalStream::pair().expect("a stream pair");
        let link = over_local_stream("inbound:node-a", core_end, None, Box::new(|_| {}))
            .expect("a local link");
        // The node reads the two requests and answers neither, holding its
        // end until the test is done with the link.
        let (requests, written) = mpsc::channel();
        let (finished, done) = mpsc::channel::<()>();
        let node = std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(node_end);
            for _ in 0..2 {
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok_and(|read| read > 0) {
                    let _ = requests.send(line);
                }
            }
            let _ = done.recv();
        });
        let project = || Call::Project {
            path: "/repo".to_owned(),
        };
        let call = |link: &RemoteHost| {
            let link = link.clone();
            std::thread::spawn(move || link.call(project(), TIMEOUT))
        };
        let unanswered = |result: Result<LinkAnswer, LinkError>| matches!(result, Err(LinkError::Unknown(reason)) if reason.contains("did not answer in time"));

        // Every slot is taken, and one comes free late.
        let gate = link.inner.gate_for(&project());
        for _ in 0..MAX_RUNNING {
            gate.admit(Instant::now() + Duration::from_secs(60))
                .expect("a slot");
        }
        let caller = call(&link);
        queued(gate, "the call never waited for a slot");
        std::thread::sleep(HELD);
        let freed = Instant::now();
        gate.release();
        assert!(unanswered(caller.join().unwrap()));
        let returned = Instant::now();
        written
            .recv_timeout(Duration::from_secs(10))
            .expect("the call was written once a slot came free");
        // Written after `freed`, a call whose answer had a timeout of its
        // own would still be waiting here.
        assert!(
            returned < freed + TIMEOUT,
            "the slot's wait was spent twice"
        );
        for _ in 1..MAX_RUNNING {
            gate.release();
        }

        // The writer is taken by another request's line, and comes free late.
        let other_line = link.inner.runtime.block_on(link.inner.writer.lock());
        let caller = call(&link);
        std::thread::sleep(HELD);
        let freed = Instant::now();
        drop(other_line);
        assert!(unanswered(caller.join().unwrap()));
        let returned = Instant::now();
        written
            .recv_timeout(Duration::from_secs(10))
            .expect("the call was written once the writer came free");
        assert!(
            returned < freed + TIMEOUT,
            "the writer's wait was spent twice"
        );

        assert!(
            link.closed_reason().is_none(),
            "an unanswered call keeps the link"
        );
        drop(finished);
        node.join().expect("the node thread");
    }

    /// Waits until a request waits for a slot. Queueing wakes no one, so
    /// the count is the only state to watch.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn queued(gate: &Gate, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while lock_recover(&gate.state).queued == 0 {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// B52: withdrawing consent stops the channel admitting anything new,
    /// whoever still holds it, and refuses a request waiting for a slot
    /// unsent, while the requests already admitted run to their answers and
    /// the connection waits for them before it closes.
    #[test]
    #[allow(clippy::disallowed_methods)] // a window in which the idle wait must not end: no state reports an event that has not happened
    fn a_draining_connection_refuses_new_and_waiting_requests_and_waits_for_running_ones() {
        let gate = Arc::new(Gate::new(MAX_RUNNING));
        for _ in 0..MAX_RUNNING {
            assert!(gate.admit(Instant::now() + Duration::from_secs(1)).is_ok());
        }
        let waiting = {
            let gate = Arc::clone(&gate);
            std::thread::spawn(move || gate.admit(Instant::now() + Duration::from_secs(60)))
        };
        queued(&gate, "the request never waited");

        let stopped = Instant::now();
        gate.stop("consent revoked");
        assert!(matches!(
            waiting.join().unwrap(),
            Err(LinkError::NotConnected(reason)) if reason == "consent revoked"
        ));
        assert!(stopped.elapsed() < Duration::from_secs(10));
        assert!(matches!(
            gate.admit(Instant::now() + Duration::from_secs(1)),
            Err(LinkError::NotConnected(_))
        ));

        let (started, waiting_idle) = std::sync::mpsc::channel();
        let idle = {
            let gate = Arc::clone(&gate);
            std::thread::spawn(move || {
                started.send(()).unwrap();
                gate.wait_idle(Instant::now() + Duration::from_secs(60))
            })
        };
        // The window covers only the wait itself, not the thread's start.
        waiting_idle.recv_timeout(Duration::from_secs(10)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!idle.is_finished(), "closed while admitted requests ran");
        for _ in 0..MAX_RUNNING {
            gate.release();
        }
        idle.join().unwrap();
        let admission = lock_recover(&gate.state);
        assert_eq!((admission.running, admission.queued), (0, 0));
    }

    /// Only the audited protocol24 transition is admitted alongside the
    /// current protocol; earlier and unknown future contracts are refused.
    #[test]
    fn only_the_audited_previous_helper_protocol_is_admitted() {
        let hello = |protocol| Hello {
            protocol,
            version: "0.0.0".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            home: None,
            machine_identity: hide_node_link::protocol::MachineIdentity::Unavailable {
                reason: "fixture identity is unavailable".to_owned(),
            },
            reader_features: None,
            windows_drives: None,
        };
        let refusal = helper_protocol_refusal(&hello(23)).expect("refused");
        assert_eq!(
            refusal,
            format!(
                "The device helper speaks protocol {}, this Hide needs {PROTOCOL_VERSION}; the helper this Hide carries does not match it, so rebuild or reinstall Hide",
                23
            )
        );
        assert_eq!(helper_protocol_refusal(&hello(PROTOCOL_VERSION)), None);
        assert_eq!(helper_protocol_refusal(&hello(24)), None);
        // The helper of the build before session_work_turn.
        assert!(helper_protocol_refusal(&hello(32)).is_some());
        assert!(helper_protocol_refusal(&hello(PROTOCOL_VERSION - 1)).is_some());
        assert!(helper_protocol_refusal(&hello(PROTOCOL_VERSION + 1)).is_some());
    }

    /// An answer line keeps its result as the helper's own text, `null`
    /// included, and a refusal as the helper's error; nothing else reads as
    /// an answer.
    #[test]
    fn an_answer_line_keeps_its_result_as_text() {
        let line: AnswerLine = serde_json::from_str(r#"{"id":7,"ok":null}"#).unwrap();
        assert_eq!((line.id, line.ok.map(RawValue::get)), (7, Some("null")));
        let line: AnswerLine =
            serde_json::from_str(r#"{"id":8,"ok":{"paths":["a"],"truncated":false}}"#).unwrap();
        let answer = LinkAnswer::Raw(line.ok.unwrap().to_owned());
        let decoded = match answer {
            LinkAnswer::Raw(raw) => {
                serde_json::from_str::<hide_node_link::index::Walked>(raw.get())
            }
            LinkAnswer::Parsed(_) => unreachable!(),
        }
        .unwrap();
        assert_eq!(decoded.paths, vec!["a".to_owned()]);
        let line: AnswerLine =
            serde_json::from_str(r#"{"id":9,"error":{"code":"invalid_path","message":"no"}}"#)
                .unwrap();
        assert!(line.ok.is_none());
        assert_eq!(
            line.error.unwrap().code,
            hide_node_link::ErrorCode::InvalidPath
        );
        assert!(serde_json::from_str::<AnswerLine>(r#"{"ok":1}"#).is_err());
    }

    #[test]
    fn device_platforms_map_to_package_names() {
        assert_eq!(
            platform_of("Darwin arm64").unwrap(),
            ("macos".to_owned(), "aarch64".to_owned())
        );
        assert_eq!(
            platform_of("Darwin x86_64\n").unwrap_err(),
            "Intel Mac devices are not supported; Hide ships for Apple Silicon Macs only"
        );
        assert!(platform_of("Darwin amd64").is_err());
        assert_eq!(
            platform_of("Linux x86_64").unwrap(),
            ("linux".to_owned(), "x86_64".to_owned())
        );
        assert_eq!(
            platform_of("Linux aarch64").unwrap(),
            ("linux".to_owned(), "aarch64".to_owned())
        );
        assert!(platform_of("FreeBSD amd64").is_err());
        assert!(platform_of("").is_err());
        // A Windows device's PowerShell script names its processor as
        // Windows does.
        assert_eq!(
            platform_of("Windows amd64").unwrap(),
            ("windows".to_owned(), "x86_64".to_owned())
        );
        assert_eq!(
            platform_of("Windows arm64").unwrap(),
            ("windows".to_owned(), "aarch64".to_owned())
        );
    }

    #[test]
    fn a_package_for_another_platform_is_never_substituted() {
        let directory = tempfile::tempdir().unwrap();
        let own_name = device_program(HELPER_NAME, std::env::consts::OS);
        std::fs::write(directory.path().join(&own_name), b"own").unwrap();
        let packages = HelperPackages::new(Some(directory.path().to_path_buf()));
        let own = packages
            .find(std::env::consts::OS, std::env::consts::ARCH)
            .unwrap();
        assert_eq!(own, directory.path().join(own_name));
        let other_arch = if std::env::consts::ARCH == "x86_64" {
            "aarch64"
        } else {
            "x86_64"
        };
        let refused = packages.find("macos", other_arch).unwrap_err();
        assert!(
            refused.contains("does not include the device helper"),
            "{refused}"
        );
        std::fs::write(
            directory
                .path()
                .join(format!("{HELPER_NAME}-macos-{other_arch}")),
            b"x",
        )
        .unwrap();
        assert!(packages.find("macos", other_arch).is_ok());
    }

    /// Writes `bytes` at `path` with `mode` where files carry one; a Windows
    /// file has none, and its package is a program by the kit's contract.
    fn write(path: &Path, bytes: &[u8], mode: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        #[cfg(not(unix))]
        let _ = mode;
    }

    /// A folder laid out as this system's package lays out its resources
    /// (`desktop/scripts/package.mjs`, and the Windows runtime copy made from
    /// it): each program under its system's name, `hided.exe` on Windows, and
    /// on Windows the holder's ConPTY runtime in the folder `holder-conpty`.
    fn own_package(folder: &Path) {
        for name in [HELPER_NAME, CLI_NAME, HOOKS_NAME, HOLDER_NAME] {
            write(
                &folder.join(device_program(name, std::env::consts::OS)),
                name.as_bytes(),
                0o755,
            );
        }
        if cfg!(windows) {
            let conpty = folder.join(hide_kit::holders::CONPTY_SOURCE);
            write(&conpty.join("conpty.dll"), b"dll", 0o644);
            write(&conpty.join("x64/OpenConsole.exe"), b"console", 0o755);
        }
    }

    /// A core gives a device of its own system the programs its own package
    /// holds, each by its system's program name, and on Windows the ConPTY
    /// folder by its one name (with-hide/hide-private#501 N1: a Windows core
    /// looked for `hided` beside its `hided.exe` and refused its own system).
    #[test]
    fn a_core_finds_its_own_programs_by_their_exe_name_and_its_conpty_folder_bare() {
        let directory = tempfile::tempdir().unwrap();
        own_package(directory.path());
        let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
        let packages = HelperPackages::new(Some(directory.path().to_path_buf()));
        assert_eq!(
            packages.find(os, arch).unwrap(),
            directory.path().join(device_program(HELPER_NAME, os))
        );
        let payload = packages.payload_for(os, arch).unwrap();
        assert!(payload.missing.is_empty(), "{:?}", payload.missing);
        let mut expected = [HELPER_NAME, CLI_NAME, HOOKS_NAME, HOLDER_NAME]
            .map(|name| (device_program(name, os), true))
            .to_vec();
        if cfg!(windows) {
            expected.extend([
                ("holder-conpty/conpty.dll".to_owned(), false),
                ("holder-conpty/x64/OpenConsole.exe".to_owned(), true),
            ]);
        }
        expected.sort();
        assert_eq!(relative(&payload), expected);
    }

    /// Windows starts a program by its `.exe` name, so a file that has none
    /// is never taken for the core's own program there.
    #[cfg(windows)]
    #[test]
    fn a_windows_core_never_takes_a_program_without_its_exe_name() {
        let directory = tempfile::tempdir().unwrap();
        for name in [HELPER_NAME, HOLDER_NAME] {
            write(&directory.path().join(name), name.as_bytes(), 0o755);
        }
        let packages = HelperPackages::new(Some(directory.path().to_path_buf()));
        let refused = packages.find("windows", "x86_64").unwrap_err();
        assert!(
            refused.contains("does not include the device helper"),
            "{refused}"
        );
    }

    /// A core update installs this build in its version folder and answers
    /// the `hided` it wrote there, under its system's program name, which
    /// is what the update then starts.
    #[test]
    fn a_core_update_answers_the_hided_it_installed_under_its_exe_name() {
        let directory = tempfile::tempdir().unwrap();
        own_package(directory.path());
        let root = tempfile::tempdir().unwrap();
        let packages = HelperPackages::new(Some(directory.path().to_path_buf()));
        let installed = packages.install_here(root.path()).unwrap();
        assert_eq!(
            installed.file_name().and_then(|name| name.to_str()),
            Some(device_program(HELPER_NAME, std::env::consts::OS).as_str())
        );
        assert_eq!(std::fs::read(&installed).unwrap(), HELPER_NAME.as_bytes());
    }

    fn relative(payload: &Payload) -> Vec<(String, bool)> {
        let mut files = payload
            .files
            .iter()
            .map(|file| (file.relative.clone(), file.executable))
            .collect::<Vec<_>>();
        files.sort();
        files
    }

    /// The kit's parts follow the helper's rule, each on its own: a build
    /// that lacks one still installs the helper and names what it left out.
    // A payload is made only where files have modes (`read_package`).
    #[cfg(unix)]
    #[test]
    fn a_device_payload_carries_every_kit_part_the_build_has() {
        let directory = tempfile::tempdir().unwrap();
        let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
        write(&directory.path().join(HELPER_NAME), b"helper", 0o755);
        let packages = HelperPackages::new(Some(directory.path().to_path_buf()));
        let bare = packages.payload(os, arch).unwrap();
        assert_eq!(bare.files[0].relative, HELPER_NAME);
        assert_eq!(bare.files.len(), 1);
        assert_eq!(bare.missing.len(), 2, "{:?}", bare.missing);

        write(&directory.path().join(CLI_NAME), b"cli", 0o755);
        write(&directory.path().join(HOOKS_NAME), b"hooks", 0o755);
        let full = packages.payload(os, arch).unwrap();
        assert!(full.missing.is_empty(), "{:?}", full.missing);
        assert_eq!(
            relative(&full),
            vec![
                ("hide".to_owned(), true),
                ("hide-agent-hooks".to_owned(), true),
                ("hided".to_owned(), true),
            ]
        );
        assert_ne!(full.version(), bare.version());

        // Another processor gets its own helper,
        // never this machine's binaries.
        let other_arch = if arch == "x86_64" {
            "aarch64"
        } else {
            "x86_64"
        };
        write(
            &directory
                .path()
                .join(format!("{HELPER_NAME}-{os}-{other_arch}")),
            b"other",
            0o755,
        );
        let other = packages.payload(os, other_arch).unwrap();
        assert_eq!(relative(&other), vec![("hided".to_owned(), true),]);
        assert_eq!(other.missing.len(), 2, "{:?}", other.missing);
    }

    /// A device gets the holder beside its helper; a build without the
    /// holder connects nothing.
    #[cfg(unix)]
    #[test]
    fn a_device_gets_the_holder_beside_its_helper() {
        let directory = tempfile::tempdir().unwrap();
        let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
        write(&directory.path().join(HELPER_NAME), b"helper", 0o755);
        let packages = HelperPackages::new(Some(directory.path().to_path_buf()));
        assert!(matches!(
            packages.payload_for(os, arch),
            Err(EstablishError::Unsupported(_))
        ));

        write(&directory.path().join(HOLDER_NAME), b"holder", 0o755);
        let own = packages.payload_for(os, arch).unwrap();
        assert!(own.files.iter().any(|file| file.relative == HOLDER_NAME));
    }

    /// A Windows device gets its programs under their `.exe` names, the
    /// holder and the ConPTY runtime the holder loads.
    #[cfg(unix)]
    #[test]
    fn a_windows_device_gets_its_programs_and_the_conpty_runtime() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path();
        for name in [HELPER_NAME, CLI_NAME, HOOKS_NAME, HOLDER_NAME] {
            write(
                &folder.join(format!("{name}-windows-x86_64")),
                name.as_bytes(),
                0o755,
            );
        }
        let packages = HelperPackages::new(Some(folder.to_path_buf()));
        assert!(
            matches!(
                packages.payload_for("windows", "x86_64"),
                Err(EstablishError::Unsupported(reason)) if reason.contains("ConPTY")
            ),
            "a holder without its ConPTY runtime runs no pane"
        );

        let conpty = folder.join(format!(
            "{}-windows-x86_64",
            hide_kit::holders::CONPTY_SOURCE
        ));
        std::fs::create_dir_all(conpty.join("x64")).unwrap();
        write(&conpty.join("conpty.dll"), b"dll", 0o644);
        write(&conpty.join("x64/OpenConsole.exe"), b"console", 0o755);
        let own = packages.payload_for("windows", "x86_64").unwrap();
        assert_eq!(
            own.files
                .iter()
                .map(|file| (file.relative.as_str(), file.executable))
                .collect::<Vec<_>>(),
            [
                ("hided.exe", true),
                ("hide.exe", true),
                ("hide-agent-hooks.exe", true),
                ("hide-holder.exe", true),
                ("holder-conpty/conpty.dll", false),
                ("holder-conpty/x64/OpenConsole.exe", true),
            ]
        );
        assert_eq!(program_by_contract("hide-holder.exe"), Some(true));
        assert_eq!(program_by_contract("holder-conpty/conpty.dll"), Some(false));
        assert_eq!(
            program_by_contract("holder-conpty/x64/OpenConsole.exe"),
            Some(true)
        );
    }

    /// Any change to any part is a new version folder on the device, so a
    /// hook never runs a mix of two builds.
    #[cfg(unix)]
    #[test]
    fn the_payload_version_follows_every_file_and_mode() {
        let directory = tempfile::tempdir().unwrap();
        let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
        write(&directory.path().join(HELPER_NAME), b"helper", 0o755);
        write(&directory.path().join(HOOKS_NAME), b"one", 0o644);
        let packages = HelperPackages::new(Some(directory.path().to_path_buf()));
        let first = packages.payload(os, arch).unwrap().version();
        assert_eq!(packages.payload(os, arch).unwrap().version(), first);
        write(&directory.path().join(HOOKS_NAME), b"two", 0o644);
        let second = packages.payload(os, arch).unwrap().version();
        assert_ne!(second, first);
        write(&directory.path().join(HOOKS_NAME), b"two", 0o755);
        assert_ne!(packages.payload(os, arch).unwrap().version(), second);
    }
}
