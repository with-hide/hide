//! A device's node: `hided node serve` answering the `hide_host` contract
//! over one SSH exec channel (PRD S5.5 D-05, D-20, D-23).
//!
//! Nothing here runs without the operator's consent for that device, recorded
//! on its registration and bound to the SSH identity the helper was first
//! allowed on. The helper is installed (or replaced by a newer build) under the
//! consented install root, started with `node serve` on an exec channel of a
//! dedicated SSH connection, and ends when that connection does: there is no
//! daemon, no listening socket and no background install.
//!
//! Admission is bounded per device: four requests run and thirty-two wait; a
//! request past that is refused as busy, never dropped. A request that was
//! sent and got no answer, because the connection ended or the deadline
//! passed, is `Unknown`: it may have taken effect, and the caller settles it
//! by reading the target again rather than by resending it (B14, B33).

use super::*;
#[path = "inbound.rs"]
pub mod inbound;
#[path = "link_streams.rs"]
mod link_streams;
#[path = "retirement.rs"]
mod retirement;
use futures_util::{StreamExt, TryStreamExt, stream};
use hide_node_link::HostError;
pub use hide_node_link::LinkError;
use hide_node_link::device::{HostConsent, HostIdentity};
use hide_node_link::panes::{NodeEvent, PanesStarted};
use hide_node_link::protocol::{Call, Hello, PROTOCOL_VERSION, Request};
use hide_node_link::{LinkAnswer, NodeLink, call_as};
pub use link_streams::{
    LinkHerdrConnector, LinkStream, LinkWriter, MAX_HERDR_PENDING, OpenError, StreamCloser,
};
use russh_sftp::client::{RawSftpSession, error::Error as SftpError};
use russh_sftp::protocol::{FileAttributes, StatusCode};
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
/// The pinned Herdr the package carries beside the helper, which a device
/// of the same system gets in a folder of its own (PRD device-herdr-update
/// D-08, `hide_kit::device_herdr`).
const HERDR_NAME: &str = "herdr";
/// Bounds on a kit folder read into memory for upload (engineering rule
/// 15); a folder past them is left out and the kit names the missing part.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a draining connection waits for its admitted requests before it
/// closes anyway. An admitted request is written and answered by its one
/// deadline, and the longest call timeout is 120 s (an index walk, a
/// worktree removal), so this outlasts it.
const DRAIN_BOUND: Duration = Duration::from_secs(250);
/// How long a cancel waits to be written; the call it stops ends on its
/// own timeout whatever happens to the cancel.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(5);
const HELLO_TIMEOUT: Duration = Duration::from_secs(15);

/// The helper builds this daemon carries, one per device platform.
#[derive(Clone, Debug, Default)]
pub struct HelperPackages {
    directory: Option<PathBuf>,
}

impl HelperPackages {
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self { directory }
    }

    /// The build for `os`/`arch` (Rust's names: `macos`, `aarch64`). A
    /// `hided-<os>-<arch>` file wins; an unsuffixed `hided` is accepted only
    /// for this daemon's own platform, which is what the package and a
    /// development build carry: the daemon's own program.
    pub fn find(&self, os: &str, arch: &str) -> Result<PathBuf, String> {
        self.find_named(HELPER_NAME, os, arch).map_err(|()| {
            format!(
                "This Hide build does not include the device helper for {} {arch}; files and Git on this device stay unavailable until a build that packages it is installed",
                platform_label(os)
            )
        })
    }

    /// Everything this build puts on a device of `os`/`arch`: the helper,
    /// which the device cannot do without, and the kit's parts, each found by
    /// the same rule as the helper. A part this build does not carry is left out and named, and
    /// the kit on the device reports it as missing from the build.
    pub(super) fn payload(&self, os: &str, arch: &str) -> Result<Payload, EstablishError> {
        let helper = self.find(os, arch).map_err(EstablishError::Unsupported)?;
        let helper = read_package(HELPER_NAME, &helper).map_err(EstablishError::Install)?;
        let mut payload = Payload {
            files: vec![helper],
            herdr: None,
            missing: Vec::new(),
        };
        for name in [CLI_NAME, HOOKS_NAME] {
            match self.find_named(name, os, arch) {
                Ok(path) => match read_package(name, &path) {
                    Ok(package) => payload.files.push(package),
                    Err(reason) => payload.missing.push(reason),
                },
                Err(()) => payload.missing.push(format!(
                    "This Hide build does not include {name} for {} {arch}",
                    platform_label(os)
                )),
            }
        }
        // A build without Herdr for this system brings none, and the device
        // keeps its own (D-08); the build folder names the copy it brings,
        // so a new pin is a new build folder.
        if let Ok(path) = self.find_named(HERDR_NAME, os, arch) {
            match read_package(hide_kit::device_herdr::HERDR_PROGRAM, &path) {
                Ok(herdr) => {
                    let pin = format!("{}\n", herdr_copy_name(&herdr));
                    payload.files.push(Package {
                        relative: hide_kit::device_herdr::HERDR_PIN.to_owned(),
                        digest: hex_digest(pin.as_bytes()),
                        bytes: pin.into_bytes(),
                        executable: false,
                    });
                    payload.herdr = Some(herdr);
                }
                Err(reason) => payload.missing.push(reason),
            }
        }
        Ok(payload)
    }

    /// A file or a folder named `name`, by the helper's rule.
    fn find_named(&self, name: &str, os: &str, arch: &str) -> Result<PathBuf, ()> {
        let directory = self.directory.as_ref().ok_or(())?;
        let named = directory.join(format!("{name}-{os}-{arch}"));
        if named.exists() {
            return Ok(named);
        }
        let own = directory.join(name);
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

/// The folder name of a pinned Herdr copy on a device: a prefix of its
/// digest, so each pin has its own folder and builds share it.
fn herdr_copy_name(herdr: &Package) -> &str {
    &herdr.digest[..16]
}

/// What a build puts on one device.
pub(super) struct Payload {
    /// The helper first, then the kit's parts.
    files: Vec<Package>,
    /// The pinned Herdr, which goes into `<root>/herdr/<copy>/` rather than
    /// the build folder, so a build that changes only the helper finds it
    /// there (D-08).
    herdr: Option<Package>,
    /// Why a kit part is not among them.
    missing: Vec<String>,
}

impl Payload {
    /// The payload for the machine running the core, which runs its own
    /// app's Herdr: a device's copy and the pin naming it are not its to
    /// carry.
    fn without_herdr(mut self) -> Self {
        self.herdr = None;
        self.files
            .retain(|file| file.relative != hide_kit::device_herdr::HERDR_PIN);
        self
    }

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
/// A system without mode bits has nothing to give it, so the package is
/// refused there rather than installed with a guessed mode.
fn read_package(relative: &str, path: &Path) -> Result<Package, String> {
    use std::io::Read;
    let unreadable = |error: std::io::Error| {
        format!("The package {} could not be read: {error}", path.display())
    };
    let mut file = std::fs::File::open(path).map_err(unreadable)?;
    if !file.metadata().map_err(unreadable)?.is_file() {
        return Err(format!("The package {} is not a file", path.display()));
    }
    let mode = hide_platform::fs::permissions::Permissions::of(&file)
        .map_err(unreadable)?
        .unix_mode()
        .ok_or_else(|| {
            format!(
                "The package {} has no file mode on this system to install it with",
                path.display()
            )
        })?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(unreadable)?;
    Ok(Package {
        relative: relative.to_owned(),
        digest: hex_digest(&bytes),
        bytes,
        executable: mode & 0o111 != 0,
    })
}

fn platform_label(os: &str) -> &str {
    match os {
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    }
}

/// `uname -s -m` in Rust's platform names.
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
/// sent, because nothing went out for it (B52). A link has two: one for
/// machine calls and one for link control (`Call::is_control`).
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
        } else {
            &self.gate
        }
    }

    /// Admits nothing more on either lane; the first reason given is kept.
    fn stop_gates(&self, reason: &str) {
        self.gate.stop(reason);
        self.control_gate.stop(reason);
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

/// Connects, checks consent against the device that answered, installs the
/// helper when the device lacks this build's, and starts it. Blocking; run
/// it off the runtime lock.
pub fn establish(
    client: &RusshRemoteClient,
    packages: &HelperPackages,
    consent: &HostConsent,
    retirement_projects: &[String],
    panes: Option<PaneHook>,
    terminals: Option<TerminalHook>,
    on_close: Box<dyn FnOnce(String) + Send + 'static>,
) -> Result<Established, EstablishError> {
    let since = Instant::now();
    let target = client.host.host_id.as_str();
    establish_stage(target, "connect", since);
    let session = client
        .runtime
        .block_on(async {
            tokio::time::timeout(SSH_OPERATION_TIMEOUT, client.shared_session())
                .await
                .unwrap_or_else(|_| {
                    Err(remote_error(
                        "remote-host-connect",
                        &client.host.host_id,
                        RemoteStage::Ssh,
                        "the SSH connection timed out",
                        true,
                        false,
                    ))
                })
        })
        .map_err(EstablishError::Connect)?;
    let identity = HostIdentity {
        user: client.host.user.clone(),
        hostname: client.host.hostname.clone(),
        port: client.host.port,
        host_key_sha256: client.observed_host_key().unwrap_or_default(),
    };
    if identity.host_key_sha256.is_empty() {
        return Err(EstablishError::Helper(
            "The device's host key was not observed, so consent cannot be checked".to_owned(),
        ));
    }
    if let Some(bound) = consent.identity.as_ref()
        && bound != &identity
    {
        return Err(EstablishError::IdentityChanged {
            bound: Box::new(bound.clone()),
            observed: Box::new(identity),
        });
    }
    let result = client.runtime.block_on(async {
        tokio::time::timeout(
            INSTALL_TIMEOUT,
            start_helper(
                &session,
                client,
                packages,
                &consent.helper_root,
                retirement_projects,
                since,
            ),
        )
        .await
        .unwrap_or_else(|_| {
            Err(EstablishError::Install(
                "Installing the device helper timed out; nothing was started".to_owned(),
            ))
        })
    });
    // A failed setup leaves the device's connection to the channels that
    // share it; what it opened closed with it.
    let Started {
        channel,
        session_channel,
        installed,
        helper_path,
        upload,
    } = result?;
    let host = spawn_host(
        client,
        Arc::clone(&client.connection),
        channel,
        session_channel,
        panes.clone(),
        on_close,
    );
    establish_stage(target, "hello", since);
    let hello: Hello = call_as(&host, Call::Hello, HELLO_TIMEOUT).map_err(|error| {
        EstablishError::Helper(format!("The device helper did not start: {error}"))
    })?;
    if let Some(reason) = helper_protocol_refusal(&hello) {
        host.close("helper protocol mismatch");
        return Err(EstablishError::Helper(reason));
    }
    let readers = hello.readers();
    for reason in readers.diagnostics() {
        crate::diagnostic!(json!({
            "component": "remote_host",
            "kind": "host.reader_advertisement_refused",
            "target": target,
            "reason": reason,
        }));
    }
    host.take_readers(hello.protocol, readers)
        .map_err(EstablishError::Helper)?;
    if panes.is_some() {
        establish_stage(target, "panes", since);
        start_panes(client, &host);
    }
    let terminals = match terminals {
        None => Err("this process takes no device terminals".to_owned()),
        Some(hook) => {
            establish_stage(target, "terminals", since);
            // The audited 24 link keeps its legacy features only: its node
            // has no terminal service, so it is never asked to start one.
            if hello.protocol == PROTOCOL_VERSION {
                start_terminals(client, &host, hook)
            } else {
                let reason = format!(
                    "the device's helper speaks protocol {}, which carries no terminals",
                    hello.protocol
                );
                crate::diagnostic!(json!({
                    "component": "remote_host",
                    "kind": "host.terminals_unstarted",
                    "target": target,
                    "reason": reason,
                }));
                Err(reason)
            }
        }
    };
    establish_stage(target, "ready", since);
    Ok(Established {
        node: NodeReady {
            host: Arc::new(host),
            hello,
            terminals,
        },
        identity,
        installed,
        helper_path,
        upload,
    })
}

/// Starts the node's terminal service for the device's Herdr and the writer
/// that carries the core's side of it, so the device's panes' terminals
/// flow inside this link (PRD core-host-node-terminal D-10). A device whose
/// Herdr cannot be found has none; its panes read unavailable and the
/// reason goes to the log.
fn start_terminals(
    client: &RusshRemoteClient,
    host: &RemoteHost,
    hook: TerminalHook,
) -> Result<Arc<dyn hide_node_link::terminal::TerminalNode>, String> {
    start_terminals_at(
        client
            .herdr_socket_path()
            .map_err(|error| error.to_string()),
        host,
        hook,
    )
}

/// [`start_terminals`] for the node's Herdr at `herdr_socket`, as the node
/// names it.
pub(super) fn start_terminals_at(
    herdr_socket: Result<String, String>,
    host: &RemoteHost,
    hook: TerminalHook,
) -> Result<Arc<dyn hide_node_link::terminal::TerminalNode>, String> {
    let started = herdr_socket
        .and_then(|herdr_socket| {
            host.call(Call::TerminalsStart { herdr_socket }, HELLO_TIMEOUT)
                .map_err(|error| error.to_string())
        })
        .and_then(|_| {
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
                "kind": "host.terminals_started",
                "target": host.inner.target,
                "link": terminals.link(),
            }));
            Ok(Arc::new(terminals))
        }
        Err(reason) => {
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "host.terminals_unstarted",
                "target": host.inner.target,
                "reason": reason,
            }));
            Err(reason)
        }
    }
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
fn establish_stage(target: &str, stage: &str, since: Instant) {
    crate::diagnostic!(json!({
        "component": "remote_host",
        "kind": "host.establish_stage",
        "target": target,
        "stage": stage,
        "elapsed_ms": since.elapsed().as_millis() as u64,
    }));
}

/// Starts the node's pane service for the device's Herdr, so its panes'
/// `hide` reaches the core over this link. A device whose Herdr cannot be
/// found keeps its files and Git; its panes' `hide` answers that the node is
/// unavailable, and the reason goes to the log.
fn start_panes(client: &RusshRemoteClient, host: &RemoteHost) {
    start_panes_at(
        client
            .herdr_socket_path()
            .map_err(|error| error.to_string()),
        host,
    );
}

/// [`start_panes`] for the node's Herdr at `herdr_socket`, as the node
/// names it.
pub(super) fn start_panes_at(herdr_socket: Result<String, String>, host: &RemoteHost) {
    let started = herdr_socket.and_then(|herdr_socket| {
        call_as::<PanesStarted>(host, Call::PanesStart { herdr_socket }, HELLO_TIMEOUT)
            .map_err(|error| error.to_string())
    });
    match started {
        Ok(_) => crate::diagnostic!(json!({
            "component": "remote_host",
            "kind": "host.panes_started",
            "target": host.inner.target,
        })),
        Err(reason) => crate::diagnostic!(json!({
            "component": "remote_host",
            "kind": "host.panes_unstarted",
            "target": host.inner.target,
            "reason": reason,
        })),
    }
}

/// Protocol24 has an audited transition contract with a frozen reader set.
/// Every other mismatch is refused before use; ordinary digest-based payload
/// replacement still installs the helper carried by this build.
fn helper_protocol_refusal(hello: &Hello) -> Option<String> {
    (!matches!(hello.protocol, 24 | PROTOCOL_VERSION)).then(|| {
        format!(
            "The device helper speaks protocol {}, this Hide needs {PROTOCOL_VERSION}; the helper this Hide carries does not match it, so rebuild or reinstall Hide",
            hello.protocol
        )
    })
}

/// A started helper: its channel and that channel's place among the
/// connection's sessions, and what the install did.
struct Started {
    channel: Channel<Msg>,
    session_channel: tokio::sync::OwnedSemaphorePermit,
    installed: bool,
    helper_path: String,
    upload: Upload,
}

async fn start_helper(
    session: &Handle<KnownHostHandler>,
    client: &RusshRemoteClient,
    packages: &HelperPackages,
    helper_root: &str,
    retirement_projects: &[String],
    since: Instant,
) -> Result<Started, EstablishError> {
    let target = client.host.host_id.clone();
    establish_stage(&target, "platform", since);
    let admit = |operation: &'static str, stage: RemoteStage| async move {
        client
            .session_channel(operation, stage)
            .await
            .map_err(EstablishError::Connect)
    };
    let permit = admit("remote-host-platform", RemoteStage::Sftp).await?;
    let uname = execute_channel(
        session,
        "uname -s -m",
        "remote-host-platform",
        &target,
        RemoteStage::Sftp,
    )
    .await
    .map_err(EstablishError::Connect)?;
    if uname.exit_status != 0 {
        return Err(EstablishError::Unsupported(format!(
            "The device could not report its platform: {}",
            uname.stderr.trim()
        )));
    }
    drop(permit);
    let (os, arch) = platform_of(uname.stdout.trim()).map_err(EstablishError::Unsupported)?;
    let payload = packages.payload(&os, &arch)?;
    // The helper inherits this SSH exec environment. Read its path overrides
    // before uploading any candidate; older helpers need no new operation.
    establish_stage(&target, "environment", since);
    let permit = admit("remote-retirement-environment", RemoteStage::Sftp).await?;
    let environment = execute_channel(
        session,
        r#"[ "${#HCOORD_HOME}" -le 4096 ] && [ "${#HIDE_STATE_DIR}" -le 4096 ] && [ "${#XDG_STATE_HOME}" -le 4096 ] || exit 65; printf '%s\n' "${HCOORD_HOME-}" "${HIDE_STATE_DIR-}" "${XDG_STATE_HOME-}""#,
        "remote-retirement-environment",
        &target,
        RemoteStage::Sftp,
    )
    .await
    .map_err(EstablishError::Connect)?;
    if environment.exit_status != 0 {
        return Err(EstablishError::Install("The device retirement paths could not be inspected; check its SSH environment and retry; no helper was uploaded".into()));
    }
    drop(permit);
    let locations = retirement::Locations::from_environment(&environment.stdout)?;

    establish_stage(&target, "install", since);
    let permit = admit("remote-host-install", RemoteStage::Sftp).await?;
    let channel = session.channel_open_session().await.map_err(|error| {
        EstablishError::Install(format!("The SFTP channel could not be opened: {error}"))
    })?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| {
            EstablishError::Install(format!("SFTP is not available on the device: {error}"))
        })?;
    let raw = RawSftpSession::new(channel.into_stream());
    raw.set_timeout(30);
    let installed = install(
        &raw,
        helper_root,
        &payload,
        retirement_projects,
        &locations,
        |phase| establish_stage(&target, phase, since),
    )
    .await;
    let _ = raw.close_session();
    drop(raw);
    drop(permit);
    let installed = installed?;
    let (helper_path, fresh) = (installed.helper_path, installed.fresh);

    establish_stage(&target, "start", since);
    let session_channel = admit("remote-host-start", RemoteStage::Ssh).await?;
    let channel = session.channel_open_session().await.map_err(|error| {
        EstablishError::Helper(format!("The helper channel could not be opened: {error}"))
    })?;
    channel
        .exec(true, format!("{} node serve", shell_quote(&helper_path)))
        .await
        .map_err(|error| {
            EstablishError::Helper(format!("The helper could not be started: {error}"))
        })?;
    Ok(Started {
        channel,
        session_channel,
        installed: fresh,
        helper_path,
        upload: installed.upload,
    })
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sftp_failure(what: &str, error: impl fmt::Display) -> EstablishError {
    EstablishError::Install(format!("{what}: {error}"))
}

/// Where an install left things.
pub(super) struct Installed {
    pub(super) helper_path: String,
    /// Whether the helper itself was written on this connection.
    fresh: bool,
    upload: Upload,
}

/// Puts this build's helper and kit parts in `<root>/<version>/`, where the
/// version is a prefix of the digest over all of them, keeping the folder
/// layout the kit reads. The
/// helper is reused only after checking its bytes, because Hide runs it; any
/// other file is reused when a file of its size that only the account can
/// change is already there, because only a verified upload is ever renamed
/// into that private folder and reading every part back on each connection
/// would delay every reconnect. So an install that stopped part way resumes
/// with the files it had not sent (B18). A kit file that cannot be placed
/// never stops the helper: the kit on the device reports that part. The
/// helper removes older builds once it has pointed `current` at this one
/// (`hide_host::kit`).
async fn install(
    raw: &RawSftpSession,
    helper_root: &str,
    payload: &Payload,
    retirement_projects: &[String],
    locations: &retirement::Locations,
    stage: impl Fn(&str),
) -> Result<Installed, EstablishError> {
    stage("install_init");
    raw.init()
        .await
        .map_err(|error| sftp_failure("SFTP did not start", error))?;
    stage("install_root");
    let (home, owner, root) = private_root_spelling(raw, helper_root).await?;
    // No folders, staging files or current link have changed at this point.
    stage("install_preflight");
    tokio::time::timeout(
        SSH_OPERATION_TIMEOUT,
        retirement::preflight(raw, &home, owner, retirement_projects, locations),
    ).await.map_err(|_| EstablishError::Install(
        "The device retirement preflight timed out; inspect its run and request state and retry; no helper was uploaded".into()
    ))??;
    stage("install_payload");
    place_payload(raw, &home, owner, &root, payload).await
}

/// Puts this build in a version folder under `helper_root` on a machine
/// that already runs a build of Hide's, without the device's first-install
/// checks: a core update (PRD core-host-node-move B10) on the machine
/// running the core, whose helper root the move already made. Answers where
/// the build's `hided` is. `raw` has started.
pub(super) async fn install_build(
    raw: &RawSftpSession,
    packages: &HelperPackages,
    os: &str,
    arch: &str,
    helper_root: &str,
) -> Result<Installed, EstablishError> {
    let payload = packages.payload(os, arch)?.without_herdr();
    if !payload.missing.is_empty() {
        return Err(EstablishError::Install(format!(
            "This build cannot update a core without all of its parts: {}",
            payload.missing.join("; ")
        )));
    }
    let (home, owner, root) = private_root_spelling(raw, helper_root).await?;
    place_payload(raw, &home, owner, &root, &payload).await
}

impl HelperPackages {
    /// [`install_build`] on this machine: this build in its version folder
    /// under `root`, the folder a remote install of it would name, for a
    /// core update run where the core runs (PRD core-host-node-move Q13).
    /// A file already there is kept only when its bytes are this build's;
    /// any other is replaced whole. Answers where the build's `hided` is.
    pub fn install_here(&self, root: &Path) -> Result<PathBuf, String> {
        use hide_platform::fs::atomic::{self, DurableAccess};
        use hide_platform::fs::private;
        let payload = self
            .payload(std::env::consts::OS, std::env::consts::ARCH)
            .map_err(|error| error.to_string())?
            .without_herdr();
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
        Ok(version_dir.join(HELPER_NAME))
    }
}

/// The account's home as SFTP reports it, its owner, and `helper_root`
/// spelled under it, checked to be a plain path.
async fn private_root_spelling(
    raw: &RawSftpSession,
    helper_root: &str,
) -> Result<(String, u32, String), EstablishError> {
    let home = raw
        .realpath(".")
        .await
        .map_err(|error| sftp_failure("SFTP did not report the home directory", error))?
        .files
        .first()
        .map(|entry| entry.filename.clone())
        .ok_or_else(|| {
            EstablishError::Install("SFTP did not report the home directory".to_owned())
        })?;
    if !home.starts_with('/') || home.chars().any(char::is_control) {
        return Err(EstablishError::Install(
            "SFTP reported an unusable home directory".to_owned(),
        ));
    }
    let owner = raw
        .lstat(&home)
        .await
        .map_err(|error| sftp_failure("The home directory could not be inspected", error))?
        .attrs
        .uid
        .ok_or_else(|| {
            EstablishError::Install("SFTP did not report the home directory's owner".to_owned())
        })?;
    let root = match helper_root.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", home.trim_end_matches('/')),
        None if helper_root.starts_with('/') => helper_root.to_owned(),
        None => {
            return Err(EstablishError::Install(format!(
                "The helper install root {helper_root:?} must be absolute or start with ~/"
            )));
        }
    };
    if root.split('/').any(|part| part == ".." || part == ".") || root.chars().any(char::is_control)
    {
        return Err(EstablishError::Install(
            "The helper install root is not a plain path".to_owned(),
        ));
    }
    Ok((home, owner, root))
}

/// Puts `payload` in its version folder under `root` (see [`install`]).
async fn place_payload(
    raw: &RawSftpSession,
    home: &str,
    owner: u32,
    root: &str,
    payload: &Payload,
) -> Result<Installed, EstablishError> {
    // Everything from here goes through the path the check resolved, whose
    // every folder was checked and none is a link, so a link on the spelled
    // path cannot be swapped between the check and the launch.
    let root = ensure_private_dirs(raw, home, root, owner).await?;
    let version = payload.version();
    let version_dir = format!("{root}/{}", &version[..16]);
    ensure_private_dir(raw, &version_dir, owner).await?;
    let mut upload = Upload {
        missing: payload.missing.clone(),
        ..Upload::default()
    };
    let (helper, parts) = payload
        .files
        .split_first()
        .ok_or_else(|| EstablishError::Install("The build carries no helper".to_owned()))?;
    let final_path = format!("{version_dir}/{}", helper.relative);
    let reusable = match raw.lstat(&final_path).await {
        Ok(existing) => {
            private_file(&existing.attrs, owner)
                && existing.attrs.size == Some(helper.bytes.len() as u64)
                && remote_digest(raw, &final_path, helper.bytes.len(), "helper")
                    .await
                    .ok()
                    .as_deref()
                    == Some(helper.digest.as_str())
        }
        Err(_) => false,
    };
    let (helper_path, fresh) = if reusable {
        upload.reused += 1;
        (final_path, false)
    } else {
        upload.sent += 1;
        (
            place_file(raw, &version_dir, helper, owner, "helper").await?,
            true,
        )
    };
    let mut folders = std::collections::HashSet::new();
    for part in parts {
        let placed = async {
            // Each folder on the way is made private before anything is put
            // in it, parents first.
            let mut folder = version_dir.clone();
            if let Some((parents, _)) = part.relative.rsplit_once('/') {
                for name in parents.split('/') {
                    folder = format!("{folder}/{name}");
                    if folders.insert(folder.clone()) {
                        ensure_private_dir(raw, &folder, owner).await?;
                    }
                }
            }
            let path = format!("{version_dir}/{}", part.relative);
            match raw.lstat(&path).await {
                Ok(existing)
                    if private_file(&existing.attrs, owner)
                        && existing.attrs.size == Some(part.bytes.len() as u64) =>
                {
                    Ok(false)
                }
                _ => place_file(raw, &version_dir, part, owner, &part.relative)
                    .await
                    .map(|_| true),
            }
        }
        .await;
        match placed {
            Ok(true) => upload.sent += 1,
            Ok(false) => upload.reused += 1,
            Err(error) => upload.missing.push(error.to_string()),
        }
    }
    if let Some(herdr) = &payload.herdr {
        match place_herdr(raw, &root, herdr, owner).await {
            Ok(true) => upload.sent += 1,
            Ok(false) => upload.reused += 1,
            Err(error) => upload.missing.push(error.to_string()),
        }
    }
    Ok(Installed {
        helper_path,
        fresh,
        upload,
    })
}

/// Puts the pinned Herdr in `<root>/herdr/<copy>/herdr`, by the kit parts'
/// rule: a private file of its size already there is the verified upload
/// an earlier connection renamed into place. Answers whether it was sent.
async fn place_herdr(
    raw: &RawSftpSession,
    root: &str,
    herdr: &Package,
    owner: u32,
) -> Result<bool, EstablishError> {
    let folder = format!("{root}/{}", hide_kit::device_herdr::HERDR_DIR);
    ensure_private_dir(raw, &folder, owner).await?;
    let copy = format!("{folder}/{}", herdr_copy_name(herdr));
    ensure_private_dir(raw, &copy, owner).await?;
    match raw.lstat(&format!("{copy}/{}", herdr.relative)).await {
        Ok(existing)
            if private_file(&existing.attrs, owner)
                && existing.attrs.size == Some(herdr.bytes.len() as u64) =>
        {
            Ok(false)
        }
        _ => place_file(raw, &copy, herdr, owner, "pinned Herdr")
            .await
            .map(|_| true),
    }
}

/// Numbers each upload this process stages, so two connections to one
/// account (two registrations of the same machine) never share a staging
/// name.
static NEXT_UPLOAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

use super::transfer::{TRANSFER_CHUNK, TRANSFERS_IN_FLIGHT};

/// Staging names this process made whose upload ended before it removed
/// them: an install that times out drops its upload mid-write, where no
/// `await` can run. The next upload into the same folder removes them. Only
/// this process's own names are ever listed, and the list is capped.
static ABANDONED_STAGING: Mutex<Vec<String>> = Mutex::new(Vec::new());
const ABANDONED_STAGING_CAP: usize = 64;

/// A staging name in use; dropped before `finish`, it is listed as abandoned.
struct Staging {
    path: String,
    finished: bool,
}

impl Staging {
    fn finish(mut self) {
        self.finished = true;
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let mut abandoned = lock_recover(&ABANDONED_STAGING);
        if abandoned.len() == ABANDONED_STAGING_CAP {
            abandoned.remove(0);
        }
        abandoned.push(std::mem::take(&mut self.path));
    }
}

/// Removes the staging files this process abandoned in `folder`.
async fn remove_abandoned_staging(raw: &RawSftpSession, folder: &str) {
    let here: Vec<String> = {
        let mut abandoned = lock_recover(&ABANDONED_STAGING);
        let (here, elsewhere) = std::mem::take(&mut *abandoned)
            .into_iter()
            .partition(|path| path.rsplit_once('/').map(|(parent, _)| parent) == Some(folder));
        *abandoned = elsewhere;
        here
    };
    for path in here {
        let _ = raw.remove(&path).await;
    }
}

/// Whether `path` already holds exactly `package`'s bytes.
async fn holds(raw: &RawSftpSession, path: &str, package: &Package, what: &str) -> bool {
    matches!(
        remote_digest(raw, path, package.bytes.len(), what).await,
        Ok(found) if found == package.digest
    )
}

/// Uploads `package` beside its final name, checks the uploaded bytes, and
/// renames it into place, so the final name only ever holds a whole,
/// verified copy. Another connection to the same account can place the same
/// build at the same time: a final name that already holds these bytes is
/// kept rather than replaced.
async fn place_file(
    raw: &RawSftpSession,
    version_dir: &str,
    package: &Package,
    owner: u32,
    what: &str,
) -> Result<String, EstablishError> {
    let upload_id = NEXT_UPLOAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let since = Instant::now();
    let stage = |phase: &str| {
        crate::diagnostic!(json!({
            "component": "remote_host",
            "kind": "host.upload_stage",
            "upload_id": upload_id,
            "package": package.relative,
            "bytes": package.bytes.len(),
            "phase": phase,
            "elapsed_ms": since.elapsed().as_millis() as u64,
        }));
    };
    stage("open");
    let final_path = format!("{version_dir}/{}", package.relative);
    let (folder, name) = final_path
        .rsplit_once('/')
        .unwrap_or((version_dir, package.relative.as_str()));
    let staging = format!(
        "{folder}/.upload-{name}-{}-{}",
        std::process::id(),
        upload_id
    );
    remove_abandoned_staging(raw, folder).await;
    // Only a process that ended with this same pid could have left it.
    let _ = raw.remove(&staging).await;
    let staged = Staging {
        path: staging.clone(),
        finished: false,
    };
    let handle = raw
        .open(
            &staging,
            OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            FileAttributes {
                permissions: Some(if package.executable { 0o700 } else { 0o600 }),
                ..FileAttributes::empty()
            },
        )
        .await
        .map_err(|error| sftp_failure(&format!("The {what} could not be uploaded"), error))?
        .handle;
    // The read-back below checks the whole file.
    stage("write");
    let written = super::transfer::write_bytes(raw, &handle, &package.bytes, |error| {
        sftp_failure(&format!("The {what} upload failed"), error)
    })
    .await;
    stage("close");
    let closed = raw.close(handle).await;
    let verified = match (written, closed) {
        (Ok(()), Ok(_)) => {
            stage("read_back");
            match remote_digest(raw, &staging, package.bytes.len(), what).await {
                Ok(uploaded) if uploaded == package.digest => Ok(()),
                Ok(_) => Err(EstablishError::Install(format!(
                    "The uploaded {what} did not match the package; it was not used"
                ))),
                Err(error) => Err(error),
            }
        }
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(sftp_failure(
            &format!("The {what} upload did not finish"),
            error,
        )),
    };
    if let Err(error) = verified {
        let _ = raw.remove(&staging).await;
        staged.finish();
        return Err(error);
    }
    stage("place");
    if holds(raw, &final_path, package, what).await {
        let _ = raw.remove(&staging).await;
        staged.finish();
    } else {
        let _ = raw.remove(&final_path).await;
        let renamed = raw.rename(&staging, &final_path).await;
        if renamed.is_err() {
            let _ = raw.remove(&staging).await;
        }
        staged.finish();
        if let Err(error) = renamed
            && !holds(raw, &final_path, package, what).await
        {
            return Err(sftp_failure(
                &format!("The {what} could not be put in place"),
                error,
            ));
        }
    }
    let placed = raw.lstat(&final_path).await.map_err(|error| {
        sftp_failure(
            &format!("The installed {what} could not be inspected"),
            error,
        )
    })?;
    if !private_file(&placed.attrs, owner) {
        let _ = raw.remove(&final_path).await;
        return Err(EstablishError::Install(format!(
            "{final_path} was not left a file only the account can change, so the {what} was not used"
        )));
    }
    stage("complete");
    Ok(final_path)
}

/// A helper file the account owns and no group or other account can write.
/// Its folder is already private, so this refuses what a device's own
/// defaults could leave behind, not what another account could swap in.
fn private_file(attrs: &FileAttributes, owner: u32) -> bool {
    attrs.is_regular()
        && attrs.uid == Some(owner)
        && attrs.permissions.is_some_and(|mode| mode & 0o022 == 0)
}

/// Where the private walk to the helper root starts, and the names it
/// creates from there: below home it starts at home, which it never touches,
/// and anywhere else at `/`. Both are the device's paths, so they are read
/// as `/`-spelled names whatever system the core runs on: empty names (`//`,
/// a trailing `/`) are dropped as the device drops them, and `/home/al` is
/// not a prefix of `/home/alice`.
fn private_walk(
    home: &str,
    root: &str,
) -> Result<(String, String), hide_platform::path::PathError> {
    let plain = |path: &str| {
        let names: Vec<&str> = path.split('/').filter(|name| !name.is_empty()).collect();
        format!("/{}", names.join("/"))
    };
    let (home, root) = (plain(home), plain(root));
    match hide_platform::path::wire_relative(&home, &root) {
        Ok(below) => Ok((home.trim_end_matches('/').to_owned(), below.to_string())),
        Err(hide_platform::path::PathError::Outside) => Ok((String::new(), root)),
        Err(error) => Err(error),
    }
}

/// Creates and checks the helper root, and answers the real path it resolves
/// to, which is the one the helper is installed under and started from.
async fn ensure_private_dirs(
    raw: &RawSftpSession,
    home: &str,
    root: &str,
    owner: u32,
) -> Result<String, EstablishError> {
    let (mut current, relative) = private_walk(home, root).map_err(|error| {
        EstablishError::Install(format!(
            "The helper install root is not a plain path: {error}"
        ))
    })?;
    for part in relative.split('/').filter(|part| !part.is_empty()) {
        current.push('/');
        current.push_str(part);
        // An existing ancestor may be a link (`/tmp` on macOS), but the link
        // and the folder it leads to must both be ones no other account can
        // change, or that account could swap the helper between its digest
        // check and its launch (OpenSSH's rule for key files). The root
        // itself is checked without following links below.
        match raw.lstat(&current).await {
            Ok(link) if link.attrs.is_symlink() && !owned_by(&link.attrs, owner) => {
                return Err(EstablishError::Install(format!(
                    "{current} is a link another account owns, so the helper was not installed"
                )));
            }
            Ok(_) | Err(SftpError::Status(_)) => {}
            Err(error) => {
                return Err(sftp_failure(
                    "The helper folder could not be inspected",
                    error,
                ));
            }
        }
        match raw.stat(&current).await {
            Ok(attrs) if attrs.attrs.is_dir() => validate_ancestor(&attrs.attrs, owner, &current)?,
            Ok(_) => {
                return Err(EstablishError::Install(format!(
                    "{current} exists and is not a folder, so the helper was not installed"
                )));
            }
            Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
                make_private_dir(raw, &current).await?;
            }
            Err(error) => {
                return Err(sftp_failure(
                    "The helper folder could not be inspected",
                    error,
                ));
            }
        }
    }
    let attrs = raw
        .lstat(root)
        .await
        .map_err(|error| sftp_failure("The helper folder could not be inspected", error))?
        .attrs;
    validate_private(&attrs, owner, root)?;
    // The walk above checked each folder as spelled, and home not at all.
    // The folder the root really is is checked again from `/`, home and its
    // parents included, so a linked ancestor whose target sits under a
    // folder another account can change is refused as well (OpenSSH's
    // `safe_path` rule for key files, which a device that signs in with a
    // key already passes).
    let resolved = raw
        .realpath(root)
        .await
        .map_err(|error| sftp_failure("The helper folder could not be resolved", error))?
        .files
        .first()
        .map(|entry| entry.filename.clone())
        .ok_or_else(|| {
            EstablishError::Install("SFTP did not resolve the helper folder".to_owned())
        })?;
    for ancestor in ancestors(&resolved)? {
        let attrs = raw
            .lstat(&ancestor)
            .await
            .map_err(|error| sftp_failure("The helper folder could not be inspected", error))?
            .attrs;
        if !attrs.is_dir() {
            return Err(EstablishError::Install(format!(
                "{ancestor} is not a folder on the resolved helper path, so the helper was not installed"
            )));
        }
        validate_ancestor(&attrs, owner, &ancestor)?;
    }
    let attrs = raw
        .lstat(&resolved)
        .await
        .map_err(|error| sftp_failure("The helper folder could not be inspected", error))?
        .attrs;
    validate_private(&attrs, owner, &resolved)?;
    Ok(resolved)
}

/// Every folder above `resolved`, from `/`, for an absolute path SFTP
/// resolved; anything else is refused rather than guessed at.
fn ancestors(resolved: &str) -> Result<Vec<String>, EstablishError> {
    if !resolved.starts_with('/')
        || resolved.chars().any(char::is_control)
        || resolved.split('/').any(|part| part == ".." || part == ".")
    {
        return Err(EstablishError::Install(format!(
            "SFTP resolved the helper folder to an unusable path {resolved:?}"
        )));
    }
    let parts: Vec<&str> = resolved
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let mut folders = vec!["/".to_owned()];
    let mut current = String::new();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        current.push('/');
        current.push_str(part);
        folders.push(current.clone());
    }
    Ok(folders)
}

/// Makes `path` a 0700 folder. Another connection to the same account (two
/// registrations of one machine connecting together) can make it first: a
/// folder that is there when `mkdir` fails is taken as that one, and the
/// caller checks it as it would its own.
async fn make_private_dir(raw: &RawSftpSession, path: &str) -> Result<(), EstablishError> {
    let made = raw
        .mkdir(
            path,
            FileAttributes {
                permissions: Some(0o700),
                ..FileAttributes::empty()
            },
        )
        .await;
    match made {
        Ok(_) => Ok(()),
        Err(error) => match raw.lstat(path).await {
            Ok(found) if found.attrs.is_dir() => Ok(()),
            _ => Err(sftp_failure(
                "The helper folder could not be created",
                error,
            )),
        },
    }
}

async fn ensure_private_dir(
    raw: &RawSftpSession,
    path: &str,
    owner: u32,
) -> Result<(), EstablishError> {
    match raw.lstat(path).await {
        Ok(attrs) => validate_private(&attrs.attrs, owner, path),
        Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
            make_private_dir(raw, path).await?;
            let attrs = raw
                .lstat(path)
                .await
                .map_err(|error| sftp_failure("The helper folder could not be inspected", error))?;
            validate_private(&attrs.attrs, owner, path)
        }
        Err(error) => Err(sftp_failure(
            "The helper folder could not be inspected",
            error,
        )),
    }
}

fn owned_by(attrs: &FileAttributes, owner: u32) -> bool {
    attrs.uid == Some(owner) || attrs.uid == Some(0)
}

/// A folder on the way to the helper root: owned by the account or by root,
/// and writable by no group or other account unless it is a root-owned sticky
/// folder such as `/tmp`, where others cannot rename what the account made.
/// A folder whose mode was not reported is refused, not assumed safe. Group
/// write is refused even for a group only the account is in (a `umask 002`
/// home on some Linux systems), as OpenSSH's default rule does; the refusal
/// names the fix.
fn validate_ancestor(attrs: &FileAttributes, owner: u32, path: &str) -> Result<(), EstablishError> {
    let Some(mode) = attrs.permissions else {
        return Err(EstablishError::Install(format!(
            "The device did not report who can change {path}, so the helper was not installed under it"
        )));
    };
    let shared = mode & 0o022 != 0;
    let root_sticky = attrs.uid == Some(0) && mode & 0o1000 != 0;
    if owned_by(attrs, owner) && (!shared || root_sticky) {
        return Ok(());
    }
    Err(EstablishError::Install(if owned_by(attrs, owner) {
        format!(
            "{path} can be changed by its group or by other accounts, so the helper was not installed under it; remove that write access on the device (chmod go-w {path}) or choose another install root"
        )
    } else {
        format!(
            "{path} belongs to another account, so the helper was not installed under it; choose another install root"
        )
    }))
}

fn validate_private(attrs: &FileAttributes, owner: u32, path: &str) -> Result<(), EstablishError> {
    if !attrs.is_dir()
        || attrs.uid != Some(owner)
        || attrs.permissions.is_none_or(|mode| mode & 0o022 != 0)
    {
        return Err(EstablishError::Install(format!(
            "{path} must be a folder owned by the account and not writable by others; the helper was not installed"
        )));
    }
    Ok(())
}

async fn remote_digest(
    raw: &RawSftpSession,
    path: &str,
    length: usize,
    what: &str,
) -> Result<String, EstablishError> {
    let handle = raw
        .open(path, OpenFlags::READ, FileAttributes::empty())
        .await
        .map_err(|error| sftp_failure(&format!("The {what} could not be read back"), error))?
        .handle;
    let mut hasher = Sha256::new();
    let mut offset = 0usize;
    // Several reads in flight, hashed in file order. A file that ends early
    // answers nothing more and is incomplete below.
    let read_range = |start: usize, end: usize| {
        let handle = handle.clone();
        async move {
            super::transfer::read_range(raw, &handle, start as u64, end as u64)
                .await
                .map_err(|error| sftp_failure(&format!("The {what} could not be read back"), error))
        }
    };
    let result = stream::iter((0..length).step_by(TRANSFER_CHUNK))
        .map(|start| read_range(start, (start + TRANSFER_CHUNK).min(length)))
        .buffered(TRANSFERS_IN_FLIGHT)
        .try_for_each(|bytes| {
            offset += bytes.len();
            hasher.update(&bytes);
            futures_util::future::ready(Ok(()))
        })
        .await;
    let _ = raw.close(handle).await;
    result?;
    if offset != length {
        return Err(EstablishError::Install(format!(
            "The {what} on the device is incomplete"
        )));
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Starts the link over `channel`. Its reader keeps `connection`, the
/// device's connection the channel runs on, until the link ends: a device
/// removed while a call is still out (its kit coming off) drops its own hold
/// on the connection, which closes only when its last holder goes, and the
/// call still gets its answer.
fn spawn_host(
    client: &RusshRemoteClient,
    connection: Arc<Connection>,
    channel: Channel<Msg>,
    session_channel: tokio::sync::OwnedSemaphorePermit,
    panes: Option<PaneHook>,
    on_close: Box<dyn FnOnce(String) + Send + 'static>,
) -> RemoteHost {
    let writer: Pin<Box<dyn AsyncWrite + Send>> = Box::pin(channel.make_writer());
    start_reader(
        client.host.host_id.clone(),
        Arc::clone(&client.runtime),
        Some(session_channel),
        writer,
        LinkSource::Channel {
            channel,
            _connection: connection,
        },
        panes,
        on_close,
    )
}

/// Where a link's lines come from: a device helper's SSH exec channel, or
/// the bytes of a node that dialed this core, as its attach role hands them
/// over a local stream (PRD core-host-node-remote-core D-04, D-10).
enum LinkSource {
    Channel {
        channel: Channel<Msg>,
        _connection: Arc<Connection>,
    },
    Stream(tokio::sync::mpsc::Receiver<Vec<u8>>),
}

/// What one read of a link's source brought.
enum Arrival {
    Data(Vec<u8>),
    Stderr(Vec<u8>),
    Ended(String),
}

impl LinkSource {
    async fn next(&mut self) -> Arrival {
        match self {
            LinkSource::Channel { channel, .. } => loop {
                match channel.wait().await {
                    Some(ChannelMsg::Data { data }) => return Arrival::Data(data.to_vec()),
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        return Arrival::Stderr(data.to_vec());
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        return Arrival::Ended(format!(
                            "the device helper exited with status {exit_status}"
                        ));
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        return Arrival::Ended(
                            "the connection to the device helper closed".to_owned(),
                        );
                    }
                    Some(_) => {}
                }
            },
            LinkSource::Stream(chunks) => match chunks.recv().await {
                Some(chunk) => Arrival::Data(chunk),
                None => Arrival::Ended("the node's attach stream closed".to_owned()),
            },
        }
    }

    /// Ends what the link read from, so its far end reads the end of its
    /// input and exits.
    async fn finish(self) {
        match self {
            LinkSource::Channel { channel, .. } => {
                let _ = channel.eof().await;
                let _ = channel.close().await;
            }
            LinkSource::Stream(chunks) => drop(chunks),
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
    stream: Arc<std::sync::Mutex<hide_platform::ipc::LocalStream>>,
    writing: Option<tokio::task::JoinHandle<std::io::Result<usize>>>,
}

impl BlockingWriter {
    fn new(stream: hide_platform::ipc::LocalStream) -> Self {
        Self {
            stream: Arc::new(std::sync::Mutex::new(stream)),
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
                std::io::Write::write_all(&mut *stream, &bytes)?;
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
    use std::sync::atomic::AtomicBool;

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

    fn folder(uid: u32, mode: u32) -> FileAttributes {
        FileAttributes {
            uid: Some(uid),
            permissions: Some(0o040000 | mode),
            ..FileAttributes::empty()
        }
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

    /// A helper file is reused or started only as the account's own file no
    /// group or other account can write, and never when its mode is unknown.
    #[test]
    fn a_helper_file_others_can_write_is_not_reused_or_started() {
        let me = 501;
        let file = |uid: u32, mode: Option<u32>| FileAttributes {
            uid: Some(uid),
            permissions: mode.map(|mode| 0o100000 | mode),
            ..FileAttributes::empty()
        };
        assert!(private_file(&file(me, Some(0o700)), me));
        assert!(!private_file(&file(me, Some(0o720)), me));
        assert!(!private_file(&file(me, Some(0o702)), me));
        assert!(!private_file(&file(502, Some(0o700)), me));
        assert!(!private_file(&file(me, None), me));
        assert!(!private_file(&folder(me, 0o700), me));
    }

    /// The folders on the way to the helper root follow OpenSSH's rule: the
    /// account's or root's, and shared-writable only as a root sticky folder.
    #[test]
    fn a_helper_root_ancestor_another_account_can_change_is_refused() {
        let me = 501;
        assert!(validate_ancestor(&folder(me, 0o700), me, "/tmp/mine").is_ok());
        assert!(validate_ancestor(&folder(0, 0o755), me, "/usr").is_ok());
        assert!(validate_ancestor(&folder(0, 0o1777), me, "/private/tmp").is_ok());
        assert!(validate_ancestor(&folder(502, 0o755), me, "/tmp/theirs").is_err());
        assert!(validate_ancestor(&folder(me, 0o777), me, "/tmp/open").is_err());
        assert!(validate_ancestor(&folder(0, 0o777), me, "/shared").is_err());
        // A mode the device did not report is refused, even for root's.
        let unreported = FileAttributes {
            uid: Some(0),
            permissions: None,
            ..FileAttributes::empty()
        };
        assert!(validate_ancestor(&unreported, me, "/").is_err());
        // A group-writable home folder names the fix.
        let refusal = validate_ancestor(&folder(me, 0o775), me, "/home/me/.local")
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("chmod go-w /home/me/.local"), "{refusal}");
        // The resolved path is checked from `/`, home and its parents too.
        assert_eq!(
            ancestors("/Users/example/.hide/host-helper").unwrap(),
            ["/", "/Users", "/Users/example", "/Users/example/.hide"]
        );
        assert_eq!(ancestors("/helper").unwrap(), ["/"]);
        assert!(ancestors("relative/helper").is_err());
        assert!(ancestors("/a/../b").is_err());
        assert!(owned_by(&folder(0, 0o755), me) && !owned_by(&folder(502, 0o755), me));
        let walk = |home: &str, root: &str| private_walk(home, root).unwrap();
        let pair = |start: &str, below: &str| (start.to_owned(), below.to_owned());
        assert_eq!(
            walk("/home/alice", "/home/alice/.hide/helper"),
            pair("/home/alice", ".hide/helper")
        );
        assert_eq!(walk("/home/alice", "/home/alice"), pair("/home/alice", ""));
        assert_eq!(walk("/", "/helper"), pair("", "helper"));
        // Spelled with empty names, it is the same folder and the same walk.
        assert_eq!(
            walk("/home/alice/", "/home/alice//.hide/helper/"),
            pair("/home/alice", ".hide/helper")
        );
        // Outside home, and beside it by a shared prefix, it walks from `/`.
        assert_eq!(walk("/home/alice", "/opt/hide"), pair("", "/opt/hide"));
        assert_eq!(walk("/home/al", "/home/alice/x"), pair("", "/home/alice/x"));
        assert!(private_walk("/home/alice", "/home/alice/../bob").is_err());
    }

    #[test]
    fn device_platforms_map_to_package_names() {
        assert_eq!(
            platform_of("Darwin arm64").unwrap(),
            ("macos".to_owned(), "aarch64".to_owned())
        );
        assert_eq!(
            platform_of("Darwin x86_64\n").unwrap(),
            ("macos".to_owned(), "x86_64".to_owned())
        );
        assert_eq!(
            platform_of("Linux aarch64").unwrap(),
            ("linux".to_owned(), "aarch64".to_owned())
        );
        assert!(platform_of("FreeBSD amd64").is_err());
        assert!(platform_of("").is_err());
    }

    #[test]
    fn a_package_for_another_platform_is_never_substituted() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join(HELPER_NAME), b"own").unwrap();
        let packages = HelperPackages::new(Some(directory.path().to_path_buf()));
        let own = packages
            .find(std::env::consts::OS, std::env::consts::ARCH)
            .unwrap();
        assert_eq!(own, directory.path().join(HELPER_NAME));
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

    #[cfg(unix)]
    fn write(path: &Path, bytes: &[u8], mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
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

    /// The pinned Herdr goes to a device of the package's own system only,
    /// in a folder of its own that the build folder names, so a new pin is
    /// a new build and a helper-only change finds the copy in place.
    #[cfg(unix)]
    #[test]
    fn a_device_of_the_packages_own_system_gets_the_pinned_herdr_beside_its_build() {
        let directory = tempfile::tempdir().unwrap();
        let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
        write(&directory.path().join(HELPER_NAME), b"helper", 0o755);
        let packages = HelperPackages::new(Some(directory.path().to_path_buf()));
        let without = packages.payload(os, arch).unwrap();
        assert!(without.herdr.is_none());

        write(&directory.path().join(HERDR_NAME), b"pinned herdr", 0o755);
        let with = packages.payload(os, arch).unwrap();
        let herdr = with.herdr.as_ref().unwrap();
        assert_eq!(herdr.relative, "herdr");
        assert!(herdr.executable);
        let pin = with
            .files
            .iter()
            .find(|file| file.relative == "herdr.pin")
            .unwrap();
        assert_eq!(pin.bytes, format!("{}\n", &herdr.digest[..16]).into_bytes());
        assert!(!with.files.iter().any(|file| file.relative == "herdr"));
        assert_ne!(with.version(), without.version());
        // The machine running the core keeps its own app's Herdr, and its
        // build is the one it would be without a pin.
        let core = packages.payload(os, arch).unwrap().without_herdr();
        assert!(core.herdr.is_none());
        assert_eq!(core.version(), without.version());

        write(
            &directory.path().join(HERDR_NAME),
            b"next pinned herdr",
            0o755,
        );
        assert_ne!(
            packages.payload(os, arch).unwrap().version(),
            with.version()
        );

        // A device of another processor gets its own helper and no Herdr.
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
        assert!(other.herdr.is_none());
        assert!(!other.files.iter().any(|file| file.relative == "herdr.pin"));
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

    /// A writable in-memory SFTP account for the upload tests: each write
    /// lands at its own offset, a read answers at most `read_cap` bytes, and
    /// while `slow` is set every write waits, so an install can be cut off
    /// mid-upload.
    #[derive(Clone, Default)]
    struct Uploads {
        files: Arc<Mutex<std::collections::BTreeMap<String, Vec<u8>>>>,
        slow: Arc<AtomicBool>,
    }

    struct UploadAccount {
        uploads: Uploads,
        read_cap: usize,
    }

    const UPLOAD_OWNER: u32 = 501;

    fn ok_status(id: u32) -> russh_sftp::protocol::Status {
        russh_sftp::protocol::Status {
            id,
            status_code: StatusCode::Ok,
            error_message: String::new(),
            language_tag: String::new(),
        }
    }

    impl russh_sftp::server::Handler for UploadAccount {
        type Error = StatusCode;
        fn unimplemented(&self) -> Self::Error {
            StatusCode::OpUnsupported
        }
        async fn open(
            &mut self,
            id: u32,
            filename: String,
            flags: OpenFlags,
            _attrs: FileAttributes,
        ) -> Result<russh_sftp::protocol::Handle, Self::Error> {
            let mut files = lock_recover(&self.uploads.files);
            if flags.contains(OpenFlags::CREATE) {
                if flags.contains(OpenFlags::EXCLUDE) && files.contains_key(&filename) {
                    return Err(StatusCode::Failure);
                }
                files.entry(filename.clone()).or_default();
            } else if !files.contains_key(&filename) {
                return Err(StatusCode::NoSuchFile);
            }
            Ok(russh_sftp::protocol::Handle {
                id,
                handle: filename,
            })
        }
        async fn write(
            &mut self,
            id: u32,
            handle: String,
            offset: u64,
            data: Vec<u8>,
        ) -> Result<russh_sftp::protocol::Status, Self::Error> {
            if self.uploads.slow.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let mut files = lock_recover(&self.uploads.files);
            let file = files.get_mut(&handle).ok_or(StatusCode::NoSuchFile)?;
            let end = offset as usize + data.len();
            if file.len() < end {
                file.resize(end, 0);
            }
            file[offset as usize..end].copy_from_slice(&data);
            Ok(ok_status(id))
        }
        async fn read(
            &mut self,
            id: u32,
            handle: String,
            offset: u64,
            len: u32,
        ) -> Result<russh_sftp::protocol::Data, Self::Error> {
            let files = lock_recover(&self.uploads.files);
            let bytes = files.get(&handle).ok_or(StatusCode::NoSuchFile)?;
            let offset = offset as usize;
            if offset >= bytes.len() {
                return Err(StatusCode::Eof);
            }
            let end = bytes.len().min(offset + (len as usize).min(self.read_cap));
            Ok(russh_sftp::protocol::Data {
                id,
                data: bytes[offset..end].to_vec(),
            })
        }
        async fn close(
            &mut self,
            id: u32,
            _handle: String,
        ) -> Result<russh_sftp::protocol::Status, Self::Error> {
            Ok(ok_status(id))
        }
        async fn lstat(
            &mut self,
            id: u32,
            path: String,
        ) -> Result<russh_sftp::protocol::Attrs, Self::Error> {
            let files = lock_recover(&self.uploads.files);
            let bytes = files.get(&path).ok_or(StatusCode::NoSuchFile)?;
            Ok(russh_sftp::protocol::Attrs {
                id,
                attrs: FileAttributes {
                    uid: Some(UPLOAD_OWNER),
                    size: Some(bytes.len() as u64),
                    permissions: Some(0o100700),
                    ..FileAttributes::empty()
                },
            })
        }
        async fn remove(
            &mut self,
            id: u32,
            filename: String,
        ) -> Result<russh_sftp::protocol::Status, Self::Error> {
            lock_recover(&self.uploads.files)
                .remove(&filename)
                .ok_or(StatusCode::NoSuchFile)?;
            Ok(ok_status(id))
        }
        async fn rename(
            &mut self,
            id: u32,
            oldpath: String,
            newpath: String,
        ) -> Result<russh_sftp::protocol::Status, Self::Error> {
            let mut files = lock_recover(&self.uploads.files);
            let bytes = files.remove(&oldpath).ok_or(StatusCode::NoSuchFile)?;
            files.insert(newpath, bytes);
            Ok(ok_status(id))
        }
    }

    fn package(relative: &str, length: usize) -> Package {
        let bytes: Vec<u8> = (0..length).map(|index| (index * 31 % 251) as u8).collect();
        let digest = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Package {
            relative: relative.to_owned(),
            bytes,
            digest,
            executable: true,
        }
    }

    /// Runs `body` against a fresh in-memory account.
    fn with_account<T>(
        uploads: &Uploads,
        read_cap: usize,
        body: impl AsyncFnOnce(&RawSftpSession) -> T,
    ) -> T {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (client, server) = tokio::io::duplex(1024 * 1024);
            let account = UploadAccount {
                uploads: uploads.clone(),
                read_cap,
            };
            let task = tokio::spawn(russh_sftp::server::run(server, account));
            let raw = RawSftpSession::new(client);
            raw.init().await.unwrap();
            let result = body(&raw).await;
            let _ = raw.close_session();
            drop(raw);
            let _ = task.await;
            result
        })
    }

    fn staging_names(uploads: &Uploads) -> Vec<String> {
        lock_recover(&uploads.files)
            .keys()
            .filter(|path| path.contains("/.upload-"))
            .cloned()
            .collect()
    }

    /// The helper goes up many writes at a time and is read back many reads
    /// at a time, short reads included; what lands is the package, byte for
    /// byte, under its final name, with no staging file left.
    #[test]
    fn a_pipelined_upload_places_the_package_byte_for_byte() {
        let uploads = Uploads::default();
        let helper = package("hided", TRANSFER_CHUNK * TRANSFERS_IN_FLIGHT * 3 + 1234);
        let placed = with_account(&uploads, 10_000, async |raw| {
            place_file(raw, "/pipelined/v1", &helper, UPLOAD_OWNER, "helper").await
        })
        .unwrap();
        assert_eq!(placed, "/pipelined/v1/hided");
        assert_eq!(
            lock_recover(&uploads.files).get("/pipelined/v1/hided"),
            Some(&helper.bytes)
        );
        assert!(staging_names(&uploads).is_empty());
    }

    /// An install that times out mid-upload drops the upload where nothing
    /// can be awaited; its staging file is gone after the next upload into
    /// the same folder, and nothing else there is touched.
    #[test]
    fn a_timed_out_upload_leaves_no_staging_file_after_the_next_attempt() {
        let uploads = Uploads::default();
        lock_recover(&uploads.files).insert("/timed-out/v1/other".into(), b"kept".to_vec());
        let helper = package("hided", TRANSFER_CHUNK * TRANSFERS_IN_FLIGHT * 4);
        uploads.slow.store(true, Ordering::SeqCst);
        let first = with_account(&uploads, usize::MAX, async |raw| {
            tokio::time::timeout(
                Duration::from_millis(30),
                place_file(raw, "/timed-out/v1", &helper, UPLOAD_OWNER, "helper"),
            )
            .await
        });
        assert!(first.is_err(), "the first attempt was cut off");
        assert_eq!(staging_names(&uploads).len(), 1, "it left its staging file");

        uploads.slow.store(false, Ordering::SeqCst);
        with_account(&uploads, usize::MAX, async |raw| {
            place_file(raw, "/timed-out/v1", &helper, UPLOAD_OWNER, "helper").await
        })
        .unwrap();
        assert!(staging_names(&uploads).is_empty());
        let files = lock_recover(&uploads.files);
        assert_eq!(files.get("/timed-out/v1/hided"), Some(&helper.bytes));
        assert_eq!(
            files.get("/timed-out/v1/other").map(Vec::as_slice),
            Some(&b"kept"[..])
        );
    }
}

/// Runs against a real device: `HERDR_TEST_SSH_ALIAS` names it and
/// `HERDR_TEST_REMOTE_FIXTURE` is a disposable folder on it holding
/// `checkout/a.txt` with `old`. The helper is installed under the fixture,
/// never under the account's own install root.
#[cfg(test)]
mod probe {
    use super::*;
    use hide_node_link::RootIdentity;
    use hide_node_link::protocol::{RootOpened, RootRef};

    #[test]
    #[ignore = "needs an authorized SSH device and a disposable fixture"]
    fn remote_host_open_save_conflict_and_close_probe() {
        let alias_name = std::env::var("HERDR_TEST_SSH_ALIAS").expect("configured SSH alias");
        let fixture = std::env::var("HERDR_TEST_REMOTE_FIXTURE").expect("remote fixture folder");
        let helper_dir =
            std::env::var("HERDR_TEST_HELPER_DIR").expect("local helper package folder");
        let home = std::env::var_os("HOME").expect("HOME");
        let alias =
            SshAlias::from_config_file(&PathBuf::from(home).join(".ssh/config"), &alias_name)
                .expect("SSH alias resolves");
        let client = RusshRemoteClient::new(alias).expect("client");
        let consent = HostConsent {
            contract: HOST_CONSENT_CONTRACT,
            helper_root: format!("{fixture}/helper"),
            cli_dir: Some(format!("{fixture}/bin")),
            granted_at_unix_ms: 1,
            identity: None,
        };
        let packages = HelperPackages::new(Some(PathBuf::from(helper_dir)));
        let (seen, closed) = std::sync::mpsc::channel::<String>();
        let established = establish(
            &client,
            &packages,
            &consent,
            &[],
            None,
            None,
            Box::new(move |reason| {
                let _ = seen.send(reason);
            }),
        )
        .expect("helper established");
        eprintln!(
            "probe: identity={} installed={} path={} platform={} {}",
            established.identity.describe(),
            established.installed,
            established.helper_path,
            established.node.hello.os,
            established.node.hello.arch
        );
        let host = established.node.host;
        let timeout = Duration::from_secs(20);
        let root_path = format!("{fixture}/checkout");
        let opened: RootOpened = call_as(
            &*host,
            Call::RootOpen {
                root: root_path.clone(),
            },
            timeout,
        )
        .expect("root opens");
        let root = RootRef {
            path: root_path,
            identity: opened.identity,
        };
        let listing: hide_node_link::list::Listing = call_as(
            &*host,
            Call::List {
                root: root.clone(),
                path: String::new(),
            },
            timeout,
        )
        .expect("listing");
        assert!(
            listing.entries.iter().any(|entry| entry.name == "a.txt"),
            "{listing:?}"
        );
        let document: hide_node_link::document::Document = call_as(
            &*host,
            Call::OpenDocument {
                root: root.clone(),
                path: "a.txt".to_owned(),
            },
            timeout,
        )
        .expect("document");
        assert_eq!(document.contents.as_deref(), Some("old\n"));
        let revision = document.revision.expect("editable revision");
        let saved: hide_node_link::save::Saved = call_as(
            &*host,
            Call::Save {
                root: root.clone(),
                path: "a.txt".to_owned(),
                contents: "saved from the probe\n".to_owned(),
                expected_revision: revision.clone(),
            },
            timeout,
        )
        .expect("save");
        assert_ne!(saved.revision, revision);
        match host.call(
            Call::Save {
                root: root.clone(),
                path: "a.txt".to_owned(),
                contents: "stale draft\n".to_owned(),
                expected_revision: revision,
            },
            timeout,
        ) {
            Err(LinkError::Refused(error)) => {
                assert_eq!(error.code, hide_node_link::ErrorCode::Conflict);
                assert_eq!(
                    error.actual_revision.as_deref(),
                    Some(saved.revision.as_str())
                );
            }
            other => panic!("a stale save must be a conflict: {other:?}"),
        }
        match host.call(
            Call::List {
                root: RootRef {
                    path: root.path.clone(),
                    identity: RootIdentity {
                        device: 0,
                        inode: 0,
                    },
                },
                path: String::new(),
            },
            timeout,
        ) {
            Err(LinkError::Refused(error)) => {
                assert_eq!(error.code, hide_node_link::ErrorCode::RootReplaced)
            }
            other => panic!("a wrong root identity must be refused: {other:?}"),
        }
        match host.call(
            Call::OpenDocument {
                root: root.clone(),
                path: "../escape.txt".to_owned(),
            },
            timeout,
        ) {
            Err(LinkError::Refused(error)) => {
                assert_eq!(error.code, hide_node_link::ErrorCode::InvalidPath)
            }
            other => panic!("a traversal must be refused: {other:?}"),
        }
        host.close("probe finished");
        match host.call(Call::Hello, timeout) {
            Err(LinkError::NotConnected(_)) => {}
            other => panic!("a closed host takes no request: {other:?}"),
        }
        eprintln!(
            "probe: close observed = {:?}",
            closed.recv_timeout(Duration::from_secs(10)).ok()
        );
    }
}
