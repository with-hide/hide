//! One hook request to the daemon: a proof-and-answer exchange over the local
//! bootstrap socket hided publishes, or a device's node socket, one JSON line
//! each way (docs/agent-hooks.md, What the hook returns and reports back).
//!
//! The hook speaks it itself, and `hide`'s hook commands speak it through
//! the same function, so the two never differ. The hook never starts `hide`
//! to ask: on Windows the system holds a program's start until its scan
//! ends, which for `hide` took a second on every start and eight the first
//! time a new copy ran, past Memory's whole budget and, three starts
//! together, past the agent's eight-second limit for the hook. A socket
//! costs no program start on any system.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hide_platform::fs::private;
use hide_platform::ipc::LocalStream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::report::HookStatusReport;

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
/// from acquiring dispatch, browser, or letter-confirmation authority.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookCommand {
    WorkspaceInfo {},
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
}

impl HookCommand {
    /// Only session guidance keeps a reference for the commands it advertises.
    pub fn one_shot(&self) -> bool {
        !matches!(self, Self::WorkspaceInfo {})
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

/// Connects to each node pane service that lives in `bridge_dir`, each
/// connect waiting no later than `deadline`, which then bounds the stream.
pub fn live_bridges(
    bridge_dir: &Path,
    deadline: Instant,
) -> Result<Vec<LocalStream>, &'static str> {
    let metadata = match std::fs::symlink_metadata(bridge_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
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
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("bridge_unavailable"),
    };
    let mut streams = Vec::new();
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
        let Ok(stream) = LocalStream::connect_until(&socket, deadline) else {
            continue;
        };
        streams.push(stream);
        if streams.len() > 16 {
            return Err("bridge_limit");
        }
    }
    Ok(streams)
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
/// in, and the reference it names (`HIDE_CAP_REF`), if any.
pub struct Caller<'a> {
    pub state_dir: &'a Path,
    pub pane_id: Option<&'a str>,
    pub named: Option<&'a Path>,
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
/// `ok` or not, with how long each step took. Choosing a route happens
/// before sending; neither a refusal nor a lost answer retries the command.
///
/// Each step that can fail has a reason of its own, which a hook records
/// (`docs/agent-hooks.md`, What a hook records when it fails): no daemon
/// published its socket (`hide_unavailable`), the record naming it is not
/// trusted (`invalid_bootstrap_socket_record`), nothing took the connection
/// there (`hide_unreachable`), the request was not written whole, so nothing
/// ran (`request_unsent`), no answer came before the deadline and the command
/// may have run (`request_timeout`), and the answer was not one bounded JSON
/// line for this request (`invalid_response`).
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
        let connect = |socket: &Path| {
            LocalStream::connect_until(socket, deadline)
                .map_err(|error| ("hide_unreachable", Some(error)))
        };
        let selected = match &named {
            Some((
                _,
                Reference {
                    route: Route::Node { socket },
                    ..
                },
            )) => connect(&node_socket(socket).ok_or("invalid_reference")?),
            _ => {
                let state = named
                    .as_ref()
                    .and_then(|(path, _)| path.parent().and_then(Path::parent))
                    .unwrap_or(caller.state_dir);
                recorded_socket_path(&bootstrap_socket_record(state))
                    .map_err(|refusal| (refusal.code(), None))
                    .and_then(|socket| connect(&socket))
            }
        };
        let mut stream = match selected {
            Ok(stream) => stream,
            // A device's node answers for a pane whose daemon is on another
            // machine; without one, the route's own failure stands.
            Err((reason, error)) => {
                let mut bridges = live_bridges(&workspace_bridges(caller.state_dir), deadline)?;
                if bridges.len() > 1 {
                    return Err("ambiguous_pane".into());
                }
                bridges.pop().ok_or_else(|| step(reason, error))?
            }
        };
        timing.connected = Some(started.elapsed());
        let nonce = random_hex::<16>()?;
        let one_shot = command.one_shot();
        let token = named.map(|(_, reference)| reference.token);
        let line = loop {
            let hook = HookRequest {
                request_id: request_id.to_owned(),
                command: command.clone(),
                token: token.clone(),
            };
            let request = json!({
                "pane_id": caller.pane_id.unwrap_or(""),
                "nonce": nonce,
                "one_shot": one_shot,
                "hook": hook,
            });
            let line = request.to_string();
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
        stream
            .write_all(format!("{line}\n").as_bytes())
            .map_err(|error| step("request_unsent", Some(error)))?;
        let line = read_line(
            |bytes| stream.read(bytes),
            || deadline.checked_duration_since(Instant::now()),
        )
        .map_err(|(reason, error)| step(reason, error))?;
        timing.answered = Some(started.elapsed());
        let answer: Value =
            serde_json::from_str(&line).map_err(|_| step("invalid_response", None))?;
        if answer["ok"] == true && answer["request_id"] != request_id {
            return Err(step("invalid_response", None));
        }
        Ok(answer)
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
}
