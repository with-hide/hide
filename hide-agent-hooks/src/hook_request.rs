//! One hook request to the daemon: a proof-and-answer exchange over the local
//! bootstrap socket hided publishes, or a device's node socket, one JSON line
//! each way (docs/agent-hooks.md, What the hook returns and reports back).
//!
//! The hook speaks it itself, and `hide`'s hook commands speak it through
//! the same function, so the two never differ. The hook never starts `hide`,
//! to ask or to confirm: on Windows the system holds a program's start until
//! its scan ends, which for `hide` took a second on every start and eight to
//! twenty-five the first time a new copy ran, inside a call no deadline can
//! end, while the starting program already held the agent's output pipe, so
//! the agent's eight-second limit for the hook ran out and it dropped the
//! hook's answer (with-hide/hide-private#663). A socket costs no program
//! start on any system.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hide_platform::fs::private;
use hide_platform::ipc::LocalStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::report::HookStatusReport;

pub mod install;

/// Where a runtime hook can hand letters to its model.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PullPoint {
    #[default]
    Prompt,
    Tool,
    TurnEnd,
}

/// The only commands admitted by a hook's proof-and-answer exchange.
/// Keeping this separate from Workspace commands prevents this transport
/// from acquiring dispatch or browser authority. Its one write of a letter is
/// the receipt for letters the same pane's pull handed over, which the core
/// takes only from their recipient's own pane and session, as it does from
/// `hide inbox --confirm`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookCommand {
    WorkspaceInfo {},
    /// Whether the caller sits in a checkout Hide has registered: the proof
    /// that admits the request is the answer, as it is for `hide workspace
    /// bootstrap`, and nothing outlives it.
    Registration {},
    LetterPull {
        prompt_digest: Option<String>,
        session: Option<String>,
        #[serde(default)]
        point: PullPoint,
        /// Set by a caller that confirms a whole letter bundle; a device kit
        /// older than bundles leaves it out and is handed its letters as
        /// before (`docs/delivery.md`, One bundle per handover).
        #[serde(default)]
        bundle: bool,
    },
    /// The letters a pull handed over reached the agent: the hook printed
    /// them and its output was flushed.
    LetterConfirm {
        ids: Vec<String>,
    },
    Memory {
        event: String,
        runtime: String,
        session: String,
        #[serde(default)]
        prompt: String,
    },
    /// What the pane's agent hook observed: its turn and its subagent
    /// counts, for the core's status judgement (PRD engine-bundle-1 D-7).
    /// Metadata only; it never dispatches input or letters.
    Status {
        report: HookStatusReport,
    },
    /// What `hide agent exec` asks or says of the profile's start it runs
    /// (PRD agent-profiles B22), for the start `attempt` names
    /// ([`AGENT_EXEC_ATTEMPT`] on its line). The service that takes it
    /// answers only a caller whose own arguments are that attempt's
    /// executor, and hands on that caller's process ([`ExecProcess`]); the
    /// engine decides what came of the start from a read of that process,
    /// never from this report alone. Metadata only, like `Status`; a
    /// [`AgentExecEvent::Profile`] is answered with an [`ExecProfile`].
    AgentExec {
        attempt: String,
        event: AgentExecEvent,
    },
}

/// What `hide agent exec` asks or tells of the start it runs.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentExecEvent {
    /// It asks how its profile runs, before it checks any program: its line
    /// names only its attempt and the agent's own words, and the engine
    /// keeps which profile, at which version, the start was checked with.
    /// Answered only while that attempt runs and before its `Ready`, with
    /// the profile as its machine has it now ([`ExecProfile`]).
    Profile,
    /// It is about to hand the pane to the program, and does only when
    /// this report is answered ok: an attempt that has ended is refused, so
    /// its line does not run the program after its start was answered.
    Ready,
    /// Its check before any start refused the program: `failure` names the
    /// profile and the program.
    Refused { failure: String },
    /// Starting the program failed after the check (`exec` on Unix;
    /// creating the process, its job or its resume on Windows).
    Failed { failure: String },
    /// Windows, where the executor stays the program's parent: the program
    /// runs as `child`, in the executor's job and resumed. Unix sends none,
    /// since `exec` leaves nothing of the executor to send it.
    Spawned { child: ExecProcess },
}

impl AgentExecEvent {
    fn valid(&self) -> bool {
        match self {
            Self::Profile | Self::Ready | Self::Spawned { .. } => true,
            Self::Refused { failure } | Self::Failed { failure } => {
                failure.len() <= AGENT_EXEC_FAILURE_LIMIT
            }
        }
    }
}

/// The longest sentence an [`AgentExecEvent`] failure carries.
pub const AGENT_EXEC_FAILURE_LIMIT: usize = 1024;

/// The answer to an [`AgentExecEvent::Profile`]: how the start runs, as the
/// profile's machine has it now at the version its start was checked with,
/// or why it does not run, which the executor reports as `Refused` and
/// starts nothing on.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ExecProfile {
    Ready(ExecStart),
    Refused { failure: String },
}

/// How a profile's start runs on its machine: what its line once carried.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ExecStart {
    /// The agent and the profile as the operator reads them
    /// (`Claude · work`), which the start's failures name.
    pub label: String,
    /// The variables the start sets: the account's, then the profile's own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<(String, String)>,
    /// The profile's own program, run in place of the agent's, with the
    /// words `leading` names before the agent's arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leading: Vec<String>,
}

/// The flag of `hide agent exec` that names its start's attempt: the first
/// words after `agent exec`, so a bounded read of its arguments always has
/// it ([`is_attempt_executor`]).
pub const AGENT_EXEC_ATTEMPT: &str = "--attempt";

/// A process as the system keeps it apart from any later one with the same
/// pid: the pid and when it started (`hide_platform::process::start_time`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ExecProcess {
    pub pid: u32,
    pub started: u64,
}

/// Whether `attempt` is one an engine gave a start ([`fresh_attempt`]'s
/// shape), which bounds what a report may name. An executor of another
/// shape's build refuses this one's line before it runs anything, as this
/// one refuses that one's.
pub fn valid_attempt(attempt: &str) -> bool {
    attempt.len() == ATTEMPT_BYTES * 2
        && attempt
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The random bytes of an attempt: it tells one start of a pane from the
/// others, beside the kernel's word for the process that names it, and is
/// never a credential.
const ATTEMPT_BYTES: usize = 8;

/// A fresh attempt for a profile's start: [`ATTEMPT_BYTES`] random bytes
/// in lowercase hex, short enough to read on the start line.
pub fn fresh_attempt() -> Result<String, &'static str> {
    random_hex::<ATTEMPT_BYTES>()
}

/// Whether `argv`, a process's arguments with its program first, is the
/// `hide agent exec` of `attempt`.
pub fn is_attempt_executor(argv: &[String], attempt: &str) -> bool {
    matches!(
        argv,
        [_, agent, exec, flag, named, ..]
            if agent == "agent" && exec == "exec" && flag == AGENT_EXEC_ATTEMPT && named == attempt
    )
}

/// The process `peer`, which sent a report of `attempt`, as the system
/// says it is now: refused `attempt_mismatch` unless its arguments are that
/// attempt's executor, read between two reads of its start so a pid that
/// changed hands in between is not taken for it.
pub fn executor_process(peer: u32, attempt: &str) -> Result<ExecProcess, &'static str> {
    use hide_platform::process;
    let started = process::start_time(peer).map_err(|_| "caller_unavailable")?;
    let argv = process::argv_of(peer).map_err(|_| "caller_unavailable")?;
    if process::start_time(peer).ok() != Some(started) {
        return Err("caller_unavailable");
    }
    if !is_attempt_executor(&argv, attempt) {
        return Err("attempt_mismatch");
    }
    Ok(ExecProcess { pid: peer, started })
}

/// What a fresh read of a start's process says of it: the program ran,
/// nothing is known yet, or the process the start ran as is gone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecSeen {
    Ran,
    Waiting,
    Ended,
}

/// What a fresh read (`read`: when the process started and its arguments,
/// `None` when no such process is in the pane) of the executor's own pid
/// says: on Unix the program runs in the executor's process, so a read of
/// that same process (its start unchanged) whose arguments are no longer
/// the attempt's executor is one taken after `exec` replaced it. Empty
/// arguments, which a process in the middle of `exec` reads as, are
/// nothing yet.
pub fn seen_executor(
    read: Option<(u64, &[String])>,
    executor: ExecProcess,
    attempt: &str,
) -> ExecSeen {
    match read {
        None => ExecSeen::Ended,
        Some((started, _)) if started != executor.started => ExecSeen::Ended,
        Some((_, argv)) if argv.is_empty() || is_attempt_executor(argv, attempt) => {
            ExecSeen::Waiting
        }
        Some(_) => ExecSeen::Ran,
    }
}

/// What a fresh read (`read`: when the process started and its parent)
/// of the child a Windows executor said it runs (`child`) says: that
/// process, still the executor's child.
pub fn seen_child(
    read: Option<(u64, Option<u32>)>,
    child: ExecProcess,
    executor: ExecProcess,
) -> ExecSeen {
    match read {
        Some((started, parent)) if started == child.started && parent == Some(executor.pid) => {
            ExecSeen::Ran
        }
        _ => ExecSeen::Ended,
    }
}

impl HookCommand {
    /// Only session guidance keeps a reference for the commands it advertises.
    pub fn one_shot(&self) -> bool {
        !matches!(self, Self::WorkspaceInfo {})
    }

    /// Whether an [`Self::AgentExec`] names a well-formed attempt and
    /// carries a bounded failure; every other command is.
    pub fn agent_exec_valid(&self) -> bool {
        match self {
            Self::AgentExec { attempt, event } => valid_attempt(attempt) && event.valid(),
            _ => true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HookRequest {
    pub request_id: String,
    pub command: HookCommand,
    /// A named reference constrains the fresh proof; it never substitutes
    /// for the node's kernel check of the caller.
    pub token: Option<String>,
}

pub const HOOK_REQUEST_LIMIT: usize = 16 * 1024;
pub const HOOK_RESPONSE_LIMIT: usize = 64 * 1024;

/// What a reference file holds: the credential, and the way to the daemon
/// that issued it.
#[derive(Serialize, Deserialize)]
pub struct Reference {
    pub token: String,
    #[serde(flatten)]
    pub route: Route,
}

/// How a command reaches the daemon that issued its credential: this
/// machine's loopback port, or, on a device, the node socket whose link to
/// that daemon carries the command (`hide_host::panes`).
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Route {
    Daemon { port: u16, origin_port: u16 },
    Node { socket: String },
}

/// The reference file at `path`, read only when it is this account's own
/// private regular file; a missing or empty one is `credential_expired`.
pub fn read_reference(path: &Path) -> Result<Reference, &'static str> {
    if !path.is_absolute() {
        return Err("invalid_reference");
    }
    // The opened file is this account's own regular file and never what a
    // link at the path leads to, so replacing the path with a link between a
    // check and the read cannot make this helper read an unrelated file.
    let file = private::open_own_file(path, false).map_err(|_| "credential_expired")?;
    let metadata = file.metadata().map_err(|_| "credential_expired")?;
    if !private::is_private(path).unwrap_or(false) || metadata.len() > 4096 {
        return Err("invalid_reference");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| "credential_expired")?;
    if bytes.len() > 4096 {
        return Err("invalid_reference");
    }
    // References are written without a flush, so a power loss can leave one
    // empty: it holds no credential, and reads as gone so a named reference
    // renews rather than failing every command.
    if bytes.is_empty() {
        return Err("credential_expired");
    }
    let reference: Reference = serde_json::from_slice(&bytes).map_err(|_| "invalid_reference")?;
    if reference.token.len() != 64 || !reference.token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid_reference");
    }
    let valid = match &reference.route {
        Route::Daemon { port, origin_port } => *port != 0 && *origin_port != 0,
        Route::Node { socket } => node_socket(socket).is_some(),
    };
    if !valid {
        return Err("invalid_reference");
    }
    Ok(reference)
}

/// The node socket a device reference names: an absolute path on this
/// machine, or nothing.
pub fn node_socket(wire: &str) -> Option<PathBuf> {
    hide_platform::path::from_wire(wire)
        .ok()
        .filter(|path| path.is_absolute())
}

/// The longest bootstrap socket path the record may hold: a short `/tmp`
/// path on Unix, the account's temporary folder on Windows.
const BOOTSTRAP_RECORD_CAP: u64 = 1024;

/// The record the daemon's bootstrap listener names its socket in
/// (`hide_node::pane_proof::bind`).
pub fn bootstrap_socket_record(state_dir: &Path) -> PathBuf {
    state_dir.join("pane-capabilities/bootstrap-socket")
}

/// Why a socket record is not followed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordRefusal {
    /// There is no record this account can open: nothing published one.
    Missing,
    /// The record or the socket's folder is not this account's own and
    /// private, or the record is not a path.
    Untrusted,
    /// The folder the record names is gone: the process that wrote it ended
    /// without removing it.
    Stale,
}

impl RecordRefusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::Missing => "hide_unavailable",
            Self::Untrusted | Self::Stale => "invalid_bootstrap_socket_record",
        }
    }
}

impl std::fmt::Display for RecordRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

/// The socket a record written by `hide_node::pane_proof::bind_recorded`
/// names, trusted only when the record and the socket's folder are this
/// account's own and private.
pub fn recorded_socket_path(record: &Path) -> Result<PathBuf, RecordRefusal> {
    let file = private::open_own_file(record, false).map_err(|_| RecordRefusal::Missing)?;
    let metadata = file.metadata().map_err(|_| RecordRefusal::Missing)?;
    if !private::is_private(record).unwrap_or(false) || metadata.len() > BOOTSTRAP_RECORD_CAP {
        return Err(RecordRefusal::Untrusted);
    }
    let mut bytes = Vec::new();
    file.take(BOOTSTRAP_RECORD_CAP)
        .read_to_end(&mut bytes)
        .map_err(|_| RecordRefusal::Untrusted)?;
    let path = PathBuf::from(String::from_utf8(bytes).map_err(|_| RecordRefusal::Untrusted)?);
    let directory = path.parent().ok_or(RecordRefusal::Untrusted)?;
    let metadata = std::fs::symlink_metadata(directory).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            RecordRefusal::Stale
        } else {
            RecordRefusal::Untrusted
        }
    })?;
    if !path.is_absolute()
        || !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || !private::owned_by_current_user(directory).unwrap_or(false)
        || !private::is_private(directory).unwrap_or(false)
    {
        return Err(RecordRefusal::Untrusted);
    }
    Ok(path)
}

/// The folder a device's helper and the `hide` beside it meet in, under the
/// state folder (`hide_kit::layout::workspace_bridges` names it for every
/// other caller).
pub fn workspace_bridges(state_dir: &Path) -> PathBuf {
    state_dir.join("workspace-bridges")
}

/// The node pane services in a bridges folder that may answer for a pane
/// ([`install::may_answer`]), connected, and whether a live one was left out
/// because it declares another installation than the pane's.
pub struct LiveBridges {
    pub streams: Vec<LocalStream>,
    pub others: bool,
}

/// Connects to each node pane service that lives in `bridge_dir` and may
/// answer for a pane naming `install`, each connect waiting no later than
/// `deadline`, which then bounds the stream. A service that declares another
/// installation is never connected to.
pub fn live_bridges(
    bridge_dir: &Path,
    deadline: Instant,
    install: Option<&str>,
) -> Result<LiveBridges, &'static str> {
    let metadata = match std::fs::symlink_metadata(bridge_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(LiveBridges {
                streams: Vec::new(),
                others: false,
            });
        }
        Err(_) => return Err("bridge_unavailable"),
    };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || !private::owned_by_current_user(bridge_dir).unwrap_or(false)
        || !private::is_private(bridge_dir).unwrap_or(false)
    {
        return Err("bridge_unavailable");
    }
    let entries = match std::fs::read_dir(bridge_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(LiveBridges {
                streams: Vec::new(),
                others: false,
            });
        }
        Err(_) => return Err("bridge_unavailable"),
    };
    let mut streams = Vec::new();
    let mut others = false;
    for entry in entries {
        let entry = entry.map_err(|_| "bridge_unavailable")?;
        if !entry.file_name().to_string_lossy().starts_with("bridge-") {
            continue;
        }
        // A socket on Unix, the listener's marker file on Windows; the
        // connect tells a live listener from what a dead one left.
        let socket = entry.path().join("bootstrap.sock");
        let Ok(metadata) = std::fs::symlink_metadata(&socket) else {
            continue;
        };
        if metadata.is_dir()
            || metadata.file_type().is_symlink()
            || !private::owned_by_current_user(&socket).unwrap_or(false)
        {
            continue;
        }
        if !install::may_answer(&install::bridge_declared(&entry.path()), install) {
            others = true;
            continue;
        }
        let Ok(stream) = LocalStream::connect_until(&socket, deadline) else {
            continue;
        };
        streams.push(stream);
        if streams.len() > 16 {
            return Err("bridge_limit");
        }
    }
    Ok(LiveBridges { streams, others })
}

/// A fresh request id: the time and 16 random bytes.
pub fn fresh_request_id() -> Result<String, &'static str> {
    let id = random_hex::<16>()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "request_unavailable")?
        .as_millis();
    Ok(format!("{now}-{id}"))
}

fn random_hex<const N: usize>() -> Result<String, &'static str> {
    let mut bytes = [0u8; N];
    getrandom::getrandom(&mut bytes).map_err(|_| "request_unavailable")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Who asks: the daemon's state folder, the pane the caller says it runs
/// in, the installation that pane names (`HIDE_INSTALL`), and the reference
/// it names (`HIDE_CAP_REF`), if any.
pub struct Caller<'a> {
    pub state_dir: &'a Path,
    pub pane_id: Option<&'a str>,
    pub install: Option<&'a str>,
    pub named: Option<&'a Path>,
}

/// One way to a daemon or node that may answer for the pane, in the order
/// [`send`] tries them.
enum Way<'a> {
    /// The node socket the pane's reference names.
    Node(PathBuf),
    /// The bootstrap socket the daemon of this state folder recorded.
    Daemon(&'a Path),
    /// This device's node pane service, when exactly one may be the pane's
    /// ([`install::single_bridge`]).
    Bridges,
}

/// Why a request got no answer: the step's reason, the system's error when
/// one said why, and whether the exchange had begun (a step of it failed,
/// rather than the request being refused before anything was chosen).
#[derive(Debug)]
pub struct Failure {
    pub reason: &'static str,
    pub error: Option<io::Error>,
    pub step: bool,
}

impl From<&'static str> for Failure {
    fn from(reason: &'static str) -> Self {
        Self {
            reason,
            error: None,
            step: false,
        }
    }
}

fn step(reason: &'static str, error: Option<io::Error>) -> Failure {
    Failure {
        reason,
        error,
        step: true,
    }
}

/// How long a request's steps took, from its start: when the stream was
/// connected and when the answer line was in, each `None` when the request
/// stopped before it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Timing {
    pub connected: Option<Duration>,
    pub answered: Option<Duration>,
}

/// What became of one request: the daemon's answer or why there is none,
/// how long its steps took, and whether the reference the caller named was
/// gone, so the request went on with the fresh proof alone.
#[derive(Debug)]
pub struct Sent {
    pub answer: Result<Value, Failure>,
    pub timing: Timing,
    pub reference_missing: bool,
}

/// Sends `command` as request `request_id` and returns the daemon's answer,
/// `ok` or not, with how long each step took.
///
/// The request goes to the route the pane's reference names, else to this
/// account's daemon and this device's node in the order the pane's
/// installation calls for ([`install::order`]). Of the device's nodes, it
/// goes to the one that may be the pane's ([`install::single_bridge`]): a
/// node declaring another installation is never asked, and with more than
/// one left the command goes to none. It moves on to
/// the next only when nothing ran: nothing took the connection, or a daemon
/// or node of another installation refused it ([`install::OTHER_INSTALL`]).
/// Any other answer is the request's, and neither a refusal nor a lost answer
/// retries the command. An `ok` answer counts only when it names the pane's
/// installation ([`install::confirm`]): one from an earlier build, which ran
/// the command without knowing whose pane it was, is
/// [`install::INSTALL_UNCONFIRMED`] and is not sent again elsewhere.
///
/// Each step that can fail has a reason of its own, which a hook records
/// (`docs/agent-hooks.md`, What a hook records when it fails): no daemon
/// published its socket (`hide_unavailable`), the record naming it is not
/// trusted (`invalid_bootstrap_socket_record`), nothing took the connection
/// there (`hide_unreachable`), the request was not written whole, so nothing
/// ran (`request_unsent`), no answer came before the deadline and the command
/// may have run (`request_timeout`), and the answer was not one bounded JSON
/// line for this request (`invalid_response`). When no route reached the
/// pane's own installation, the first route's failure stands, and
/// `other_install` only when every route that answered was another's.
pub fn send(
    caller: &Caller<'_>,
    request_id: &str,
    mut command: HookCommand,
    deadline: Instant,
) -> Sent {
    let started = Instant::now();
    let mut timing = Timing::default();
    let mut reference_missing = false;
    let answer = (|| {
        let named = match caller.named {
            Some(path) if path.as_os_str().is_empty() => return Err("invalid_reference".into()),
            Some(path) => match read_reference(path) {
                Ok(reference) => Some((path, reference)),
                // A reference that is gone falls back to the fresh proof.
                Err("credential_expired") => {
                    reference_missing = true;
                    None
                }
                Err(reason) => return Err(reason.into()),
            },
            None => None,
        };
        let ways = match &named {
            Some((
                _,
                Reference {
                    route: Route::Node { socket },
                    ..
                },
            )) => [
                Way::Node(node_socket(socket).ok_or("invalid_reference")?),
                Way::Bridges,
            ],
            // The daemon that issued the reference, whose state folder holds
            // it two levels down.
            Some(_) => [
                Way::Daemon(
                    caller
                        .named
                        .and_then(Path::parent)
                        .and_then(Path::parent)
                        .unwrap_or(caller.state_dir),
                ),
                Way::Bridges,
            ],
            None => install::order(caller.install).map(|ask| match ask {
                install::Ask::Daemon => Way::Daemon(caller.state_dir),
                install::Ask::Bridges => Way::Bridges,
            }),
        };
        let nonce = random_hex::<16>()?;
        let one_shot = command.one_shot();
        let token = named.map(|(_, reference)| reference.token);
        let line = loop {
            let hook = HookRequest {
                request_id: request_id.to_owned(),
                command: command.clone(),
                token: token.clone(),
            };
            let request = install::PaneRequest {
                access: None,
                pane_id: caller.pane_id.unwrap_or("").to_owned(),
                nonce: nonce.clone(),
                one_shot,
                hook: Some(hook),
                install: caller.install.map(str::to_owned),
            };
            let line = serde_json::to_string(&request).map_err(|_| "invalid_request")?;
            if line.len() < HOOK_REQUEST_LIMIT {
                break line;
            }
            // Only Memory carries text of the caller's: its prompt is cut
            // until the request fits.
            let HookCommand::Memory { prompt, .. } = &mut command else {
                return Err("invalid_request".into());
            };
            if prompt.is_empty() {
                return Err("memory_request_too_large".into());
            }
            *prompt = hide_memory::hook::prefix(prompt, prompt.len() / 2).to_owned();
        };
        let connect = |socket: &Path| {
            LocalStream::connect_until(socket, deadline)
                .map_err(|error| ("hide_unreachable", Some(error)))
        };
        let mut unreached = None;
        let mut other_install = false;
        for way in ways {
            let selected = match way {
                Way::Node(socket) => connect(&socket),
                Way::Daemon(state) => recorded_socket_path(&bootstrap_socket_record(state))
                    .map_err(|refusal| (refusal.code(), None))
                    .and_then(|socket| connect(&socket)),
                Way::Bridges => {
                    let bridges = live_bridges(
                        &workspace_bridges(caller.state_dir),
                        deadline,
                        caller.install,
                    )?;
                    other_install |= bridges.others;
                    match install::single_bridge(bridges.streams)? {
                        Some(stream) => Ok(stream),
                        None => continue,
                    }
                }
            };
            let mut stream = match selected {
                Ok(stream) => stream,
                Err((reason, error)) => {
                    unreached.get_or_insert_with(|| step(reason, error));
                    continue;
                }
            };
            timing.connected = Some(started.elapsed());
            stream
                .write_all(format!("{line}\n").as_bytes())
                .map_err(|error| step("request_unsent", Some(error)))?;
            let reply = read_line(
                |bytes| stream.read(bytes),
                || deadline.checked_duration_since(Instant::now()),
            )
            .map_err(|(reason, error)| step(reason, error))?;
            timing.answered = Some(started.elapsed());
            let answer: Value =
                serde_json::from_str(&reply).map_err(|_| step("invalid_response", None))?;
            if answer["ok"] != true {
                if answer["reason"] == install::OTHER_INSTALL {
                    other_install = true;
                    continue;
                }
                return Ok(answer);
            }
            if answer["request_id"] != request_id {
                return Err(step("invalid_response", None));
            }
            install::confirm(&answer, caller.install)?;
            return Ok(answer);
        }
        Err(unreached.unwrap_or_else(|| {
            if other_install {
                install::OTHER_INSTALL.into()
            } else {
                step("hide_unavailable", None)
            }
        }))
    })();
    Sent {
        answer,
        timing,
        reference_missing,
    }
}

/// Reads the answer's one line. The stream's deadline bounds each read, and
/// the time left is checked again after each, so a peer that trickles bytes
/// cannot keep the request past its deadline; the clock is supplied so that
/// is tested without sleeping or a scheduler speed assertion. A read that
/// fails says why with the system's answer.
pub fn read_line(
    mut read: impl FnMut(&mut [u8]) -> io::Result<usize>,
    mut remaining: impl FnMut() -> Option<Duration>,
) -> Result<String, (&'static str, Option<io::Error>)> {
    let timed_out = || ("request_timeout", None);
    let mut answer = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        remaining()
            .filter(|left| !left.is_zero())
            .ok_or_else(timed_out)?;
        let read = read(&mut chunk).map_err(|error| {
            let reason = if error.kind() == io::ErrorKind::TimedOut {
                "request_timeout"
            } else {
                "invalid_response"
            };
            (reason, Some(error))
        })?;
        remaining()
            .filter(|left| !left.is_zero())
            .ok_or_else(timed_out)?;
        if read == 0 {
            return Err(("invalid_response", None));
        }
        let end = chunk[..read]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|at| at + 1);
        answer.extend_from_slice(&chunk[..end.unwrap_or(read)]);
        if answer.len() > HOOK_RESPONSE_LIMIT {
            return Err(("invalid_response", None));
        }
        if end.is_some() {
            return String::from_utf8(answer).map_err(|_| ("invalid_response", None));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state folder no daemon published a socket in, and no device's node
    /// beside it: the request ends with that step's reason, before anything
    /// was sent, and says it never connected.
    #[test]
    fn a_request_with_no_daemon_to_ask_ends_at_once() {
        let state = tempfile::tempdir().unwrap();
        let caller = Caller {
            state_dir: state.path(),
            pane_id: None,
            install: None,
            named: None,
        };
        let sent = send(
            &caller,
            "1-request",
            HookCommand::WorkspaceInfo {},
            Instant::now() + Duration::from_secs(10),
        );
        let failure = sent.answer.unwrap_err();
        assert_eq!((failure.reason, failure.step), ("hide_unavailable", true));
        assert_eq!(sent.timing, Timing::default());
    }

    #[test]
    fn a_trickling_hook_response_cannot_renew_the_request_deadline() {
        use std::cell::Cell;
        let elapsed = Cell::new(Duration::ZERO);
        let reads = Cell::new(0);
        let answer = read_line(
            |bytes| {
                reads.set(reads.get() + 1);
                elapsed.set(elapsed.get() + Duration::from_secs(4));
                bytes[0] = b' ';
                Ok(1)
            },
            || Duration::from_secs(10).checked_sub(elapsed.get()),
        );
        assert_eq!(answer.map_err(|(reason, _)| reason), Err("request_timeout"));
        // At 4 s, 8 s and 12 s: the third byte came past the 10 s deadline.
        assert_eq!(reads.get(), 3);
    }

    #[test]
    fn a_hook_response_requires_a_bounded_complete_line() {
        for (bytes, expected) in [
            (b"{}\n".to_vec(), Ok("{}\n".to_owned())),
            (b"{}".to_vec(), Err("invalid_response")),
            (vec![b' '; HOOK_RESPONSE_LIMIT + 1], Err("invalid_response")),
        ] {
            let mut peer = std::io::Cursor::new(bytes);
            let answer = read_line(
                |chunk| std::io::Read::read(&mut peer, chunk),
                || Some(Duration::from_secs(10)),
            );
            assert_eq!(answer.map_err(|(reason, _)| reason), expected);
        }
        // A read the deadline ended is a timeout; one the stream ended is no
        // answer, and each keeps what the system said.
        for (kind, expected) in [
            (io::ErrorKind::TimedOut, "request_timeout"),
            (io::ErrorKind::ConnectionReset, "invalid_response"),
        ] {
            let answer = read_line(|_| Err(kind.into()), || Some(Duration::from_secs(10)));
            assert_eq!(
                answer.map_err(|(reason, error)| (reason, error.map(|error| error.kind()))),
                Err((expected, Some(kind)))
            );
        }
    }

    /// A report names only an attempt an engine could have given, so its
    /// size is bounded, and a sender is that attempt's executor only when
    /// its own first words after `agent exec` name it.
    #[test]
    fn a_report_names_an_attempt_of_the_engines_shape_and_its_executor_names_it_first() {
        let attempt = fresh_attempt().unwrap();
        assert!(valid_attempt(&attempt));
        assert_ne!(fresh_attempt().unwrap(), attempt);
        for refused in [
            "",
            "0123456789abcde",
            "0123456789abcdef0",
            "0123456789ABCDEF",
            "0123456789abcdeg",
            // The shape a build before it typed, which it never reads.
            "1794175536904-d976a9485d217ccbe6a55d4704337678",
        ] {
            assert!(!valid_attempt(refused), "{refused}");
        }
        let line = |words: &[&str]| {
            words
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        let executor = line(&[
            "/bin/hide",
            "agent",
            "exec",
            "--attempt",
            &attempt,
            "--",
            "claude",
        ]);
        assert!(is_attempt_executor(&executor, &attempt));
        assert!(!is_attempt_executor(&executor, "0123456789abcdef"));
        // The attempt named anywhere but first is not this executor's.
        let later = line(&["/bin/hide", "agent", "exec", "--", "--attempt", &attempt]);
        assert!(!is_attempt_executor(&later, &attempt));
        assert!(!is_attempt_executor(&line(&["/bin/cat", "-"]), &attempt));
        assert!(!is_attempt_executor(&[], &attempt));
    }

    /// A process whose arguments are not the attempt's executor is refused
    /// as its sender: this test's own, here.
    #[test]
    fn a_report_from_a_process_that_is_not_its_executor_is_refused() {
        let attempt = fresh_attempt().unwrap();
        assert_eq!(
            executor_process(std::process::id(), &attempt),
            Err("attempt_mismatch")
        );
    }

    /// What a fresh read of the executor's pid says: the program, once the
    /// same process (same start) has other arguments than the executor's;
    /// nothing yet while it is the executor or in the middle of `exec`; and
    /// gone when the pid is not in the pane or names a later process.
    #[test]
    fn a_read_of_the_executors_pid_counts_only_the_same_process_with_other_arguments() {
        let attempt = fresh_attempt().unwrap();
        let executor = ExecProcess {
            pid: 40,
            started: 7,
        };
        let words = |words: &[&str]| {
            words
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        let own = words(&["/bin/hide", "agent", "exec", "--attempt", &attempt]);
        let program = words(&["claude", "--resume"]);
        let none: &[String] = &[];
        assert_eq!(
            seen_executor(Some((7, program.as_slice())), executor, &attempt),
            ExecSeen::Ran
        );
        assert_eq!(
            seen_executor(Some((7, own.as_slice())), executor, &attempt),
            ExecSeen::Waiting
        );
        assert_eq!(
            seen_executor(Some((7, none)), executor, &attempt),
            ExecSeen::Waiting
        );
        // The pid under another start is another process, whatever it runs.
        assert_eq!(
            seen_executor(Some((8, program.as_slice())), executor, &attempt),
            ExecSeen::Ended
        );
        assert_eq!(seen_executor(None, executor, &attempt), ExecSeen::Ended);
    }

    /// Windows: the child the executor reported runs only while a read of
    /// it finds the same process, still the executor's child.
    #[test]
    fn a_read_of_a_windows_child_counts_only_the_same_process_under_its_executor() {
        let executor = ExecProcess {
            pid: 40,
            started: 7,
        };
        let child = ExecProcess {
            pid: 41,
            started: 9,
        };
        assert_eq!(
            seen_child(Some((9, Some(40))), child, executor),
            ExecSeen::Ran
        );
        assert_eq!(
            seen_child(Some((10, Some(40))), child, executor),
            ExecSeen::Ended
        );
        assert_eq!(
            seen_child(Some((9, Some(39))), child, executor),
            ExecSeen::Ended
        );
        assert_eq!(
            seen_child(Some((9, None)), child, executor),
            ExecSeen::Ended
        );
        assert_eq!(seen_child(None, child, executor), ExecSeen::Ended);
    }
}
