//! Pane-scoped Workspace CLI transport. Bootstrap attests the caller over a
//! local socket; commands and results use hided's authenticated `/ws`.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use herdr_core::workspace_control::Action;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;

use hide_platform::fs::private;
use hide_platform::ipc::LocalStream;

use crate::env::Env;
use crate::pane_auth::{Reference, Route};
use crate::state_file::SCHEMA_VERSION;

const TIMEOUT: Duration = Duration::from_secs(10);
/// Longer than the daemon's own Factory answer limit, so the daemon's reason
/// arrives rather than a local timeout.
const FACTORY_TIMEOUT: Duration = Duration::from_secs(110);
const REMOTE_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(25);
/// A spawn on another device: its checks, worktree, tab and agent start are
/// each a round trip over SSH.
const REMOTE_SPAWN_TIMEOUT: Duration = Duration::from_secs(60);

/// An auto-bootstrapped direct CLI owns its reference even if transport fails.
struct OneShotReference(PathBuf);

impl Drop for OneShotReference {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_file(self.0.with_extension("claimed"));
    }
}

/// The credential one command runs with: the reference `HIDE_CAP_REF` names,
/// or a one-shot reference the command bootstrapped and removes when it ends.
///
/// A named reference outlives nothing it was issued for: when it has expired
/// or the daemon revoked it, the command bootstraps a one-shot credential
/// exactly as a bare `hide` does, once, so the fallback passes the same
/// caller attestation and carries no authority a bare command would not get.
pub struct Credential<'a> {
    env: &'a Env,
    path: PathBuf,
    /// The reference came from `HIDE_CAP_REF` and has not been replaced.
    named: bool,
    _one_shot: Option<OneShotReference>,
}

impl<'a> Credential<'a> {
    pub fn acquire(env: &'a Env) -> Result<Self, String> {
        match std::env::var(crate::env::HIDE_CAP_REF) {
            Ok(value) if !value.is_empty() => Ok(Self::named(env, PathBuf::from(value))),
            Ok(_) => Err("invalid_reference".to_owned()),
            Err(_) => Self::one_shot(env),
        }
    }

    /// The credential a `HIDE_CAP_REF` naming `path` gives.
    pub fn named(env: &'a Env, path: PathBuf) -> Self {
        Self {
            env,
            path,
            named: true,
            _one_shot: None,
        }
    }

    fn one_shot(env: &'a Env) -> Result<Self, String> {
        let path = bootstrap(env, true)?;
        Ok(Self {
            env,
            path: path.clone(),
            named: false,
            _one_shot: Some(OneShotReference(path)),
        })
    }

    /// What a request on this credential does once its answer is in: a
    /// named reference is persistent and is claimed, a one-shot one ends
    /// with its connection.
    fn after_answer(&self) -> AfterAnswer {
        if self.named {
            AfterAnswer::Claim
        } else {
            AfterAnswer::Close
        }
    }

    /// Replaces a named reference the daemon no longer holds with a bare
    /// one-shot bootstrap. False when `reason` does not say the reference is
    /// gone or the credential was already bootstrapped, so a request retries
    /// once.
    fn renew(&mut self, reason: &str) -> Result<bool, String> {
        if !self.named || !matches!(reason, "credential_expired" | UNREACHABLE) {
            return Ok(false);
        }
        *self = Self::one_shot(self.env)?;
        Ok(true)
    }

    /// Runs one request, and again with a bare bootstrap when the named
    /// reference is gone. A gone reference is found before the daemon runs
    /// the command (the file, the connect, the handshake, the daemon's
    /// credential check) or, as a lost claim acknowledgement, after a read;
    /// a lost claim after an action keeps its result, so the second run
    /// never repeats an applied action.
    fn run<T>(
        &mut self,
        mut request: impl FnMut(&Path, AfterAnswer) -> Result<T, String>,
    ) -> Result<T, String> {
        match request(&self.path, self.after_answer()) {
            Err(reason) if self.renew(&reason)? => request(&self.path, self.after_answer()),
            outcome => outcome,
        }
        .map_err(reported)
    }
}

/// What a request does with its credential once the daemon answered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AfterAnswer {
    /// Claims it: a persistent reference (`HIDE_CAP_REF`) outlives its
    /// unclaimed period only once a command acknowledged an answer with it.
    Claim,
    /// Closes the connection: the daemon revokes a one-shot reference's
    /// token when that connection ends and the command removes its file, so
    /// a claim would be one more round trip for a token about to go.
    Close,
}

/// No daemon listens where the reference points, before anything was sent:
/// the daemon that issued it stopped without removing it. Reported as
/// `hide_unavailable`, the reason a caller already acts on, except by a
/// hook's request ([`request_hook`]), whose reasons name the step that failed.
const UNREACHABLE: &str = "hide_unreachable";

fn reported(reason: String) -> String {
    if reason == UNREACHABLE {
        "hide_unavailable".to_owned()
    } else {
        reason
    }
}

pub fn bootstrap(env: &Env, one_shot: bool) -> Result<PathBuf, String> {
    // A caller with no pane id (a daemon first started from a plain terminal)
    // still bootstraps: hided binds it to the registered checkout holding
    // its cwd, or refuses with the reason the caller can act on.
    let pane_id = env.pane_id.as_deref().unwrap_or("");
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(|_| "reference_unavailable".to_owned())?;
    let request = json!({"pane_id":pane_id,"nonce":hex::encode(nonce),"one_shot":one_shot});
    match bootstrap_local(env, &request) {
        Ok(path) => Ok(path),
        Err(local_reason) => match bootstrap_remote(env, &request) {
            Ok(Some(path)) => Ok(path),
            Ok(None) => Err(local_reason),
            Err(remote_reason) => Err(remote_reason),
        },
    }
}

fn bootstrap_local(env: &Env, request: &Value) -> Result<PathBuf, String> {
    let socket = crate::pane_auth::bootstrap_socket_path(&env.state_dir)?;
    let mut stream = LocalStream::connect_until(&socket, Instant::now() + TIMEOUT)
        .map_err(|_| "hide_unavailable".to_owned())?;
    writeln!(stream, "{request}").map_err(|_| "hide_unavailable".to_owned())?;
    read_bootstrap_answer(&mut stream)
}

fn read_bootstrap_answer(stream: &mut impl Read) -> Result<PathBuf, String> {
    let mut line = String::new();
    BufReader::new(stream)
        .take(4096)
        .read_line(&mut line)
        .map_err(|_| "hide_unavailable".to_owned())?;
    let answer: Value = serde_json::from_str(&line).map_err(|_| "hide_unavailable".to_owned())?;
    if answer["ok"] != true {
        return Err(answer["reason"]
            .as_str()
            .unwrap_or("bootstrap_refused")
            .to_owned());
    }
    answer["reference"]
        .as_str()
        .and_then(|wire| hide_platform::path::from_wire(wire).ok())
        .ok_or_else(|| "reference_unavailable".to_owned())
}

/// Asks each node pane service on this device (`hide_host::panes`), one per
/// daemon that has this device open, for this pane's reference.
fn bootstrap_remote(env: &Env, request: &Value) -> Result<Option<PathBuf>, String> {
    bootstrap_bridges(
        &hide_kit::layout::workspace_bridges(&env.state_dir),
        request,
    )
}

/// Connects to each node pane service that lives in `bridge_dir`, each
/// connect waiting no later than `deadline`, which then bounds the stream.
fn live_bridges(bridge_dir: &Path, deadline: Instant) -> Result<Vec<LocalStream>, String> {
    let bridge_dir = bridge_dir.to_path_buf();
    let metadata = match fs::symlink_metadata(&bridge_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("bridge_unavailable".to_owned()),
    };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || !private::owned_by_current_user(&bridge_dir).unwrap_or(false)
        || !private::is_private(&bridge_dir).unwrap_or(false)
    {
        return Err("bridge_unavailable".to_owned());
    }
    let entries = match fs::read_dir(&bridge_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("bridge_unavailable".to_owned()),
    };
    let mut streams = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| "bridge_unavailable".to_owned())?;
        if !entry.file_name().to_string_lossy().starts_with("bridge-") {
            continue;
        }
        // A socket on Unix, the listener's marker file on Windows; the
        // connect tells a live listener from what a dead one left.
        let socket = entry.path().join("bootstrap.sock");
        let Ok(metadata) = fs::symlink_metadata(&socket) else {
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
        let seen = streams.len();
        if seen > 16 {
            return Err("bridge_limit".to_owned());
        }
    }
    Ok(streams)
}

fn bootstrap_bridges(bridge_dir: &Path, request: &Value) -> Result<Option<PathBuf>, String> {
    let mut success = None;
    let mut reason = None;
    for mut stream in live_bridges(bridge_dir, Instant::now() + REMOTE_BOOTSTRAP_TIMEOUT)? {
        // Each bridge has the whole time: one answers after the one before.
        stream.set_deadline(Instant::now() + REMOTE_BOOTSTRAP_TIMEOUT);
        if writeln!(stream, "{request}").is_err() {
            reason = Some("bridge_unavailable".to_owned());
            continue;
        }
        match read_bootstrap_answer(&mut stream) {
            Ok(path) if path.starts_with(bridge_dir) => {
                if success.is_some() {
                    // A bridge may return a persistent reference already held
                    // by a live pane. Ambiguity cannot revoke either holder.
                    return Err("ambiguous_pane".to_owned());
                }
                success = Some(path);
            }
            Ok(_) => reason = Some("invalid_reference".to_owned()),
            Err(error) => reason = Some(error),
        }
    }
    if success.is_some() {
        Ok(success)
    } else if let Some(reason) = reason {
        Err(reason)
    } else {
        Ok(None)
    }
}

fn read_reference(path: &Path) -> Result<Reference, String> {
    if !path.is_absolute() {
        return Err("invalid_reference".to_owned());
    }
    // The opened file is this account's own regular file and never what a
    // link at the path leads to, so replacing the path with a link between a
    // check and the read cannot make this helper read an unrelated file.
    let file = private::open_own_file(path, false).map_err(|_| "credential_expired".to_owned())?;
    let metadata = file
        .metadata()
        .map_err(|_| "credential_expired".to_owned())?;
    if !private::is_private(path).unwrap_or(false) || metadata.len() > 4096 {
        return Err("invalid_reference".to_owned());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| "credential_expired".to_owned())?;
    if bytes.len() > 4096 {
        return Err("invalid_reference".to_owned());
    }
    // References are written without a flush, so a power loss can leave one
    // empty: it holds no credential, and reads as gone so a named reference
    // renews rather than failing every command.
    if bytes.is_empty() {
        return Err("credential_expired".to_owned());
    }
    let reference: Reference =
        serde_json::from_slice(&bytes).map_err(|_| "invalid_reference".to_owned())?;
    if reference.token.len() != 64 || !reference.token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid_reference".to_owned());
    }
    let valid = match &reference.route {
        Route::Daemon { port, origin_port } => *port != 0 && *origin_port != 0,
        Route::Node { socket } => node_socket(socket).is_some(),
    };
    if !valid {
        return Err("invalid_reference".to_owned());
    }
    Ok(reference)
}

pub fn request(credential: &mut Credential, query: &str) -> Result<Value, String> {
    credential.run(|path, after| request_query(path, query, None, after))
}

pub fn browser_connect(
    credential: &mut Credential,
    display_id: Option<&str>,
) -> Result<Value, String> {
    credential.run(|path, after| request_query(path, "browser_connect", display_id, after))
}

fn request_query(
    path: &Path,
    query: &str,
    display_id: Option<&str>,
    after: AfterAnswer,
) -> Result<Value, String> {
    let reference = read_reference(path)?;
    let request_id = fresh_request_id()?;
    run_exchange(
        path,
        &reference,
        json!({"type":"workspace_query","request_id":request_id,"query":query,"display_id":display_id}),
        &request_id,
        false,
        after,
    )
}

/// `hide links`: a read, answered without a shell.
pub fn request_links(
    credential: &mut Credential,
    query: &herdr_core::links::query::LinksQuery,
) -> Result<Value, String> {
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange(
            path,
            &reference,
            json!({"type":"links","request_id":request_id,"query":query}),
            &request_id,
            false,
            after,
        )
    })
}

/// `hide memory`: answered without a shell. An `add` asks the provider, so
/// it has the minute B8 gives it; it writes nothing after its own deadline,
/// and running it again adds nothing twice.
pub fn request_memory(
    credential: &mut Credential,
    command: &herdr_core::memory_command::MemoryCommand,
) -> Result<Value, String> {
    let (action, timeout) = match command {
        herdr_core::memory_command::MemoryCommand::Add { .. } => {
            (true, herdr_core::memory_command::ADD_TIMEOUT)
        }
        herdr_core::memory_command::MemoryCommand::Show { .. }
        | herdr_core::memory_command::MemoryCommand::Forget { .. }
        | herdr_core::memory_command::MemoryCommand::Undo { .. }
        | herdr_core::memory_command::MemoryCommand::Placed { .. }
        | herdr_core::memory_command::MemoryCommand::Building { .. }
        | herdr_core::memory_command::MemoryCommand::Unbuilt { .. }
        | herdr_core::memory_command::MemoryCommand::Outdated { .. }
        | herdr_core::memory_command::MemoryCommand::Skipped { .. }
        | herdr_core::memory_command::MemoryCommand::Stale { .. }
        | herdr_core::memory_command::MemoryCommand::Release { .. } => (true, TIMEOUT),
        herdr_core::memory_command::MemoryCommand::Search { .. } => (false, TIMEOUT),
    };
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange_within(
            path,
            &reference,
            json!({"type":"memory","request_id":request_id,"command":command}),
            &request_id,
            action,
            timeout,
            after,
        )
    })
}

/// Losing the final capability claim cannot turn durable success into retry.
pub fn request_delivery(
    credential: &mut Credential,
    command: herdr_core::delivery::Command,
    hint: Option<&str>,
) -> Result<Value, String> {
    // A spawn on another device makes several calls over SSH, so its answer
    // has longer than a local command's; running it again after a timeout
    // returns the same agent either way.
    let timeout = match &command {
        herdr_core::delivery::Command::Agents {
            command:
                herdr_core::coordination::Command::Spawn {
                    machine: Some(_), ..
                },
        } => REMOTE_SPAWN_TIMEOUT,
        _ => TIMEOUT,
    };
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange_within(
            path,
            &reference,
            json!({
                "type":"delivery", "request_id":request_id, "command":&command, "caller_pane":hint
            }),
            &request_id,
            true,
            timeout,
            after,
        )
    })
}

/// A `hide factory` command. `add` may wait for its intake review, so the
/// answer has longer than a Workspace request to arrive.
pub fn request_factory(
    credential: &mut Credential,
    command: &hide_factory::Command,
    hint: Option<&str>,
) -> Result<Value, String> {
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange_within(
            path,
            &reference,
            json!({
                "type":"factory", "request_id":request_id, "command":command, "caller_pane":hint
            }),
            &request_id,
            true,
            FACTORY_TIMEOUT,
            after,
        )
    })
}

/// One question check, with no automatic replay on an expired credential.
pub fn request_factory_question_guard(
    credential: &Credential,
    session: &str,
    agent_runtime: &str,
) -> Result<Value, String> {
    let path = &credential.path;
    let reference = read_reference(path)?;
    let request_id = fresh_request_id()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "request_unavailable".to_owned())?;
    runtime.block_on(async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let (value, mut socket) = tokio::time::timeout_at(
            deadline,
            exchange_response_with_limit(
                &reference,
                json!({"type":"factory_question_guard", "request_id":request_id,
                "session":session, "runtime":agent_runtime}),
                &request_id,
                Some(16 * 1024),
            ),
        )
        .await
        .map_err(|_| "factory_guard_expired".to_owned())??;
        if credential.after_answer() == AfterAnswer::Claim {
            tokio::time::timeout_at(deadline, claim(&mut socket, path))
                .await
                .map_err(|_| "factory_guard_expired".to_owned())??;
        }
        Ok(value)
    })
}

/// How long `hide workspace memory` waits for the core's answer: the hook
/// that runs it waits 1,250 ms in all, process start included.
const MEMORY_WITHIN: Duration = Duration::from_millis(1_200);
/// The prompt a hook piped in: at most
/// `herdr_core::memory_hook::PROMPT_LIMIT_BYTES`, its valid UTF-8 start.
pub fn read_memory_prompt(input: impl std::io::Read) -> Result<String, String> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    input
        .take(herdr_core::memory_hook::PROMPT_LIMIT_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "prompt_unreadable".to_owned())?;
    Ok(match String::from_utf8(bytes) {
        Ok(prompt) => prompt,
        Err(error) => {
            let valid = error.utf8_error().valid_up_to();
            let mut bytes = error.into_bytes();
            bytes.truncate(valid);
            String::from_utf8(bytes).map_err(|_| "prompt_unreadable".to_owned())?
        }
    })
}

/// A hook's Project Memory from the core (`hide workspace memory`). A prompt
/// whose escaping would carry the request past what the daemon reads is cut
/// until it fits.
pub fn request_memory_hook(env: &Env, mut ask: Value) -> Result<Value, String> {
    ask["kind"] = json!("memory");
    let command = serde_json::from_value(ask).map_err(|_| "invalid_request".to_owned())?;
    request_hook(env, command)
}

/// Hook reads always use the bootstrap service. Choosing a route happens
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
pub fn request_hook(
    env: &Env,
    command: hide_node_link::panes::HookCommand,
) -> Result<Value, String> {
    let within = if matches!(command, hide_node_link::panes::HookCommand::Memory { .. }) {
        MEMORY_WITHIN
    } else if matches!(
        command,
        hide_node_link::panes::HookCommand::Observation { .. }
    ) {
        hide_node_link::panes::OBSERVATION_SEND_BUDGET
    } else {
        TIMEOUT
    };
    hook_request(
        env,
        std::env::var(crate::env::HIDE_CAP_REF).ok(),
        command,
        Instant::now() + within,
    )
}

/// [`request_hook`] with the reference `HIDE_CAP_REF` names, if any, and the
/// request's deadline.
fn hook_request(
    env: &Env,
    named: Option<String>,
    mut command: hide_node_link::panes::HookCommand,
    deadline: Instant,
) -> Result<Value, String> {
    use hide_node_link::panes::{HOOK_REQUEST_LIMIT, HookCommand, HookRequest};
    let request_id = fresh_request_id()?;
    let failed = |reason: &'static str, error: Option<&std::io::Error>| {
        herdr_core::diagnostic!(json!({
            "component": "workspace_cli",
            "kind": "hook.request_failed",
            "request_id": request_id,
            "pane_id": env.pane_id,
            "reason": reason,
            "io_kind": error.map(|error| format!("{:?}", error.kind())),
        }));
        reason.to_owned()
    };
    let named = match named {
        Some(path) if path.is_empty() => return Err("invalid_reference".to_owned()),
        Some(path) => match read_reference(Path::new(&path)) {
            Ok(reference) => Some((PathBuf::from(path), reference)),
            Err(reason) if reason == "credential_expired" => {
                herdr_core::diagnostic!(json!({
                    "component": "workspace_cli",
                    "kind": "hook.reference_missing",
                    "request_id": request_id,
                    "pane_id": env.pane_id,
                }));
                None
            }
            Err(reason) => return Err(reason),
        },
        None => None,
    };
    let connect = |socket: &Path| {
        LocalStream::connect_until(socket, deadline).map_err(|error| (UNREACHABLE, Some(error)))
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
                .unwrap_or(&env.state_dir);
            hide_node::pane_proof::recorded_socket_path(&crate::pane_auth::bootstrap_socket_record(
                state,
            ))
            .map_err(|refusal| (refusal.code(), None))
            .and_then(|socket| connect(&socket))
        }
    };
    let mut stream = match selected {
        Ok(stream) => stream,
        // A device's node answers for a pane whose daemon is on another
        // machine; without one, the route's own failure stands.
        Err((reason, error)) => {
            let mut bridges = live_bridges(
                &hide_kit::layout::workspace_bridges(&env.state_dir),
                deadline,
            )?;
            if bridges.len() > 1 {
                return Err("ambiguous_pane".to_owned());
            }
            bridges
                .pop()
                .ok_or_else(|| failed(reason, error.as_ref()))?
        }
    };
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(|_| "reference_unavailable".to_owned())?;
    let token = named.map(|(_, reference)| reference.token);
    let line = loop {
        let hook = HookRequest {
            request_id: request_id.clone(),
            command: command.clone(),
            token: token.clone(),
        };
        let request = json!({"pane_id":env.pane_id.as_deref().unwrap_or(""),"nonce":hex::encode(nonce),"one_shot":command.one_shot(),"hook":hook});
        let line = request.to_string();
        if line.len() < HOOK_REQUEST_LIMIT {
            break line;
        }
        let HookCommand::Memory { prompt, .. } = &mut command else {
            return Err("invalid_request".to_owned());
        };
        if prompt.is_empty() {
            return Err("memory_request_too_large".to_owned());
        }
        *prompt = herdr_core::memory_hook::prefix(prompt, prompt.len() / 2).to_owned();
    };
    stream
        .write_all(format!("{line}\n").as_bytes())
        .map_err(|error| failed("request_unsent", Some(&error)))?;
    let line = read_hook_line(
        |bytes| stream.read(bytes),
        || deadline.checked_duration_since(Instant::now()),
    )
    .map_err(|(reason, error)| failed(reason, error.as_ref()))?;
    let answer: Value =
        serde_json::from_str(&line).map_err(|_| failed("invalid_response", None))?;
    if answer["ok"] == true && answer["request_id"] != request_id {
        return Err(failed("invalid_response", None));
    }
    Ok(answer)
}

/// Reads the answer's one line. The stream's deadline bounds each read, and
/// the time left is checked again after each, so a peer that trickles bytes
/// cannot keep the request past its deadline; the clock is supplied so that
/// is tested without sleeping or a scheduler speed assertion. A read that
/// fails says why with the system's answer.
fn read_hook_line(
    mut read: impl FnMut(&mut [u8]) -> std::io::Result<usize>,
    mut remaining: impl FnMut() -> Option<Duration>,
) -> Result<String, (&'static str, Option<std::io::Error>)> {
    use hide_node_link::panes::HOOK_RESPONSE_LIMIT;
    let timed_out = || ("request_timeout", None);
    let mut answer = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        remaining()
            .filter(|left| !left.is_zero())
            .ok_or_else(timed_out)?;
        let read = read(&mut chunk).map_err(|error| {
            let reason = if error.kind() == std::io::ErrorKind::TimedOut {
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

/// The relay socket for one `hide browser` page command: the same scoped
/// handshake as every Workspace request and a claim whatever the credential,
/// since the daemon starts the relay only for a claimed one, after which the
/// socket carries CDP frames to the caller's display. A refusal keeps the daemon's
/// reason and next action.
/// The relay socket, whether the display is its area's selected View, and the
/// address the core keeps for it.
pub(crate) async fn browser_relay(
    credential: &mut Credential<'_>,
    display_id: &str,
) -> Result<(WorkspaceSocket, bool, Option<String>), (String, Option<String>)> {
    let renewed = |reason: String| {
        let next_action = crate::cli::bootstrap_next_action(&reason).to_owned();
        (reason, Some(next_action))
    };
    match relay_once(&credential.path, display_id).await {
        Err((reason, _)) if credential.renew(&reason).map_err(renewed)? => {
            relay_once(&credential.path, display_id).await
        }
        outcome => outcome,
    }
    .map_err(|(reason, next_action)| (reported(reason), next_action))
}

async fn relay_once(
    path: &Path,
    display_id: &str,
) -> Result<(WorkspaceSocket, bool, Option<String>), (String, Option<String>)> {
    let reference = read_reference(path).map_err(|reason| (reason, None))?;
    let request_id = fresh_request_id().map_err(|reason| (reason, None))?;
    let payload = json!({"type":"browser_relay","request_id":request_id,"display_id":display_id});
    let (answer, mut socket) =
        tokio::time::timeout(TIMEOUT, exchange_response(&reference, payload, &request_id))
            .await
            .map_err(|_| ("request_timeout".to_owned(), None))?
            .map_err(|reason| (reason, None))?;
    if answer["ok"] != true {
        return Err((
            answer["reason"]
                .as_str()
                .unwrap_or("browser_relay_refused")
                .to_owned(),
            answer["next_action"].as_str().map(str::to_owned),
        ));
    }
    tokio::time::timeout(Duration::from_secs(2), claim(&mut socket, path))
        .await
        .map_err(|_| ("credential_expired".to_owned(), None))?
        .map_err(|reason| (reason, None))?;
    let (selected, source) = relay_facts(&answer["result"]);
    // The display is shown on this machine, whose daemon relays it: the
    // CDP runs here and never crosses to the core.
    if let Some(url) = answer["result"]["relay_url"].as_str() {
        let _ = socket.close(None).await;
        return Ok((dial_relay(url).await?, selected, source));
    }
    Ok((socket, selected, source))
}

/// Whether the display is its area's selected View and the address the core
/// keeps for it, from the daemon's relay answer.
pub(crate) fn relay_facts(result: &serde_json::Value) -> (bool, Option<String>) {
    (
        result["selected"] == true,
        result["source_url"].as_str().map(str::to_owned),
    )
}

/// Dials this machine's daemon at the one-shot relay URL the core handed
/// out; a refused dial keeps its reason.
async fn dial_relay(url: &str) -> Result<WorkspaceSocket, (String, Option<String>)> {
    let refused = |(reason, next_action): crate::browser_control::Failure| {
        (reason.to_owned(), Some(next_action.to_owned()))
    };
    let unavailable = (
        "browser_control_unavailable",
        "Reconnect the Hide desktop app and retry",
    );
    if !crate::browser_control::relay_url_valid(url) {
        return Err(refused(unavailable));
    }
    let port = url
        .parse::<axum::http::Uri>()
        .ok()
        .and_then(|uri| uri.port_u16())
        .ok_or_else(|| refused(unavailable))?;
    let request = url
        .into_client_request()
        .map_err(|_| refused(unavailable))?;
    let stream = tokio::time::timeout(TIMEOUT, tokio::net::TcpStream::connect(("127.0.0.1", port)))
        .await
        .map_err(|_| refused(unavailable))?
        .map_err(|_| refused(unavailable))?;
    let transport: Pin<Box<dyn Transport>> = Box::pin(stream);
    match tokio::time::timeout(TIMEOUT, tokio_tungstenite::client_async(request, transport)).await {
        Ok(Ok((socket, _))) => Ok(socket),
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) => Err(refused(
            crate::browser_relay::refused(response.status().as_u16()),
        )),
        _ => Err(refused(unavailable)),
    }
}

pub fn request_action(
    credential: &mut Credential,
    action: Action,
    request_id: &str,
) -> Result<Value, String> {
    if !valid_request_id(request_id) {
        return Err("invalid_request_id".to_owned());
    }
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        run_exchange(
            path,
            &reference,
            json!({"type":"workspace_action","request_id":request_id,"command":&action}),
            request_id,
            true,
            after,
        )
    })
}

pub fn valid_request_id(id: &str) -> bool {
    let Some((timestamp, suffix)) = id.split_once('-') else {
        return false;
    };
    id.len() <= 64
        && !suffix.is_empty()
        && timestamp.parse::<u64>().is_ok()
        && suffix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn fresh_request_id() -> Result<String, String> {
    let mut id = [0u8; 16];
    getrandom::getrandom(&mut id).map_err(|_| "request_unavailable".to_owned())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "request_unavailable".to_owned())?
        .as_millis();
    Ok(format!("{now}-{}", hex::encode(id)))
}

fn run_exchange(
    path: &Path,
    reference: &Reference,
    payload: Value,
    request_id: &str,
    action: bool,
    after: AfterAnswer,
) -> Result<Value, String> {
    run_exchange_within(path, reference, payload, request_id, action, TIMEOUT, after)
}

fn run_exchange_within(
    path: &Path,
    reference: &Reference,
    payload: Value,
    request_id: &str,
    action: bool,
    timeout: Duration,
    after: AfterAnswer,
) -> Result<Value, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "request_unavailable".to_owned())?;
    let (value, mut socket) = runtime.block_on(async {
        tokio::time::timeout(timeout, exchange_response(reference, payload, request_id))
            .await
            .map_err(|_| "request_timeout".to_owned())?
    })?;
    // The answer is in; a one-shot credential's connection ends here.
    if after == AfterAnswer::Close {
        return Ok(value);
    }
    let claimed = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), claim(&mut socket, path))
            .await
            .map_err(|_| "credential_expired".to_owned())?
    });
    if let Err(reason) = claimed {
        eprintln!(
            "{}",
            json!({"component":"workspace_cli","kind":"claim.failed","request_id":request_id,"reason":reason})
        );
        if !action {
            return Err(reason);
        }
    }
    Ok(value)
}

/// The bytes under a Workspace socket: a loopback connection, or a stream
/// through a device's node.
pub(crate) trait Transport: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Transport for T {}

pub(crate) type WorkspaceSocket = tokio_tungstenite::WebSocketStream<Pin<Box<dyn Transport>>>;

async fn connect(reference: &Reference) -> Result<WorkspaceSocket, String> {
    connect_with_limit(reference, None).await
}

async fn connect_with_limit(
    reference: &Reference,
    response_limit: Option<usize>,
) -> Result<WorkspaceSocket, String> {
    let (request, transport): (_, Pin<Box<dyn Transport>>) = match &reference.route {
        Route::Daemon { port, origin_port } => {
            let mut request = format!("ws://127.0.0.1:{port}/ws")
                .into_client_request()
                .map_err(|_| "hide_unavailable".to_owned())?;
            request.headers_mut().insert(
                ORIGIN,
                format!("http://127.0.0.1:{origin_port}")
                    .parse()
                    .map_err(|_| "hide_unavailable".to_owned())?,
            );
            let stream = tokio::net::TcpStream::connect(("127.0.0.1", *port))
                .await
                .map_err(|_| UNREACHABLE.to_owned())?;
            (request, Box::pin(stream))
        }
        Route::Node { socket } => {
            let socket = node_socket(socket).ok_or_else(|| "invalid_reference".to_owned())?;
            // The link's route answers only `/ws`, and only for a credential
            // the same node vouched for over the same link.
            let request = "ws://node/ws"
                .into_client_request()
                .map_err(|_| "hide_unavailable".to_owned())?;
            (
                request,
                Box::pin(node_stream(&socket, Some(&reference.token))?),
            )
        }
    };
    let config = response_limit.map(|limit| {
        tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(limit))
            .max_frame_size(Some(limit))
    });
    let (socket, _) = tokio_tungstenite::client_async_with_config(request, transport, config)
        .await
        .map_err(|_| UNREACHABLE.to_owned())?;
    Ok(socket)
}

/// The node socket a device reference names: an absolute path on this
/// machine, or nothing.
fn node_socket(wire: &str) -> Option<PathBuf> {
    hide_platform::path::from_wire(wire)
        .ok()
        .filter(|path| path.is_absolute())
}

/// Opens a stream through this device's node to the daemon behind it, as an
/// async stream. The node's local stream blocks, so two threads move its
/// bytes through an in-memory pipe; each ends when its side does, and the
/// first to end closes the node's stream, which ends the other.
fn node_stream(socket: &Path, token: Option<&str>) -> Result<tokio::io::DuplexStream, String> {
    let stream = LocalStream::connect(socket).map_err(|_| UNREACHABLE.to_owned())?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(|_| "hide_unavailable".to_owned())?;
    stream
        .set_read_timeout(None)
        .map_err(|_| "hide_unavailable".to_owned())?;
    let mut writer = stream.duplicate();
    writeln!(writer, "{}", json!({"stream": true, "token": token}))
        .map_err(|_| UNREACHABLE.to_owned())?;
    let (ours, theirs) = tokio::io::duplex(hide_node_link::panes::MAX_CHUNK);
    let (mut outgoing, mut incoming) = tokio::io::split(theirs);
    let runtime = tokio::runtime::Handle::current();
    let reading = runtime.clone();
    let reader_shutdown = stream.shutdown_handle();
    let mut reader = stream;
    std::thread::Builder::new()
        .name("hide-node-stream-read".to_owned())
        .spawn(move || {
            let mut buffer = vec![0_u8; hide_node_link::panes::MAX_CHUNK];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if reading
                            .block_on(incoming.write_all(&buffer[..read]))
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            reader_shutdown.shutdown();
            let _ = reading.block_on(incoming.shutdown());
        })
        .map_err(|_| "hide_unavailable".to_owned())?;
    let writer_shutdown = writer.shutdown_handle();
    std::thread::Builder::new()
        .name("hide-node-stream-write".to_owned())
        .spawn(move || {
            let mut buffer = vec![0_u8; hide_node_link::panes::MAX_CHUNK];
            loop {
                match runtime.block_on(outgoing.read(&mut buffer)) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if writer.write_all(&buffer[..read]).is_err() {
                            break;
                        }
                    }
                }
            }
            writer_shutdown.shutdown();
        })
        .map_err(|_| "hide_unavailable".to_owned())?;
    Ok(ours)
}

async fn exchange_response(
    reference: &Reference,
    payload: Value,
    request_id: &str,
) -> Result<(Value, WorkspaceSocket), String> {
    exchange_response_with_limit(reference, payload, request_id, None).await
}

async fn exchange_response_with_limit(
    reference: &Reference,
    payload: Value,
    request_id: &str,
    response_limit: Option<usize>,
) -> Result<(Value, WorkspaceSocket), String> {
    let mut socket = match response_limit {
        Some(limit) => connect_with_limit(reference, Some(limit)).await?,
        None => connect(reference).await?,
    };
    socket
        .send(Message::Text(
            json!({"token":reference.token,"schema_version":SCHEMA_VERSION})
                .to_string()
                .into(),
        ))
        .await
        .map_err(|_| "hide_unavailable".to_owned())?;
    socket
        .send(Message::Text(payload.to_string().into()))
        .await
        .map_err(|_| "hide_unavailable".to_owned())?;
    match socket.next().await {
        Some(Ok(Message::Text(text))) => {
            let value: Value =
                serde_json::from_str(&text).map_err(|_| "invalid_response".to_owned())?;
            if value["type"] != "workspace_result" || value["request_id"] != request_id {
                return Err("invalid_response".to_owned());
            }
            // The daemon checks the credential before it runs the command.
            if value["ok"] == false && value["reason"] == "credential_expired" {
                return Err("credential_expired".to_owned());
            }
            Ok((value, socket))
        }
        // A token the daemon does not hold: it expired, was revoked, or was
        // issued by a daemon that has since stopped.
        Some(Ok(Message::Close(Some(frame))))
            if u16::from(frame.code) == crate::server::CloseReason::InvalidToken.code() =>
        {
            Err("credential_expired".to_owned())
        }
        Some(Ok(Message::Close(_))) => Err("credential_rejected".to_owned()),
        _ => Err("hide_unavailable".to_owned()),
    }
}

async fn claim(socket: &mut WorkspaceSocket, path: &Path) -> Result<(), String> {
    socket
        .send(Message::Text(
            json!({"type":"workspace_claim"}).to_string().into(),
        ))
        .await
        .map_err(|_| "hide_unavailable".to_owned())?;
    match socket.next().await {
        Some(Ok(Message::Text(reply)))
            if serde_json::from_str::<Value>(&reply)
                .ok()
                .is_some_and(|answer| answer["type"] == "workspace_claimed") => {}
        _ => return Err("credential_expired".to_owned()),
    }
    let marker = path.with_extension("claimed");
    match private::create_new_file(&marker) {
        Ok(_) => Ok(()),
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists
                && private::open_own_file(&marker, false).is_ok()
                && private::is_private(&marker).unwrap_or(false) =>
        {
            Ok(())
        }
        Err(_) => Err("reference_unavailable".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};

    #[test]
    fn a_trickling_hook_response_cannot_renew_the_request_deadline() {
        use std::cell::Cell;
        let elapsed = Cell::new(Duration::ZERO);
        let reads = Cell::new(0);
        let answer = read_hook_line(
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
            (
                vec![b' '; hide_node_link::panes::HOOK_RESPONSE_LIMIT + 1],
                Err("invalid_response"),
            ),
        ] {
            let mut peer = std::io::Cursor::new(bytes);
            let answer = read_hook_line(
                |chunk| std::io::Read::read(&mut peer, chunk),
                || Some(Duration::from_secs(10)),
            );
            assert_eq!(answer.map_err(|(reason, _)| reason), expected);
        }
        // A read the deadline ended is a timeout; one the stream ended is no
        // answer, and each keeps what the system said.
        for (kind, expected) in [
            (std::io::ErrorKind::TimedOut, "request_timeout"),
            (std::io::ErrorKind::ConnectionReset, "invalid_response"),
        ] {
            let answer = read_hook_line(|_| Err(kind.into()), || Some(Duration::from_secs(10)));
            assert_eq!(
                answer.map_err(|(reason, error)| (reason, error.map(|error| error.kind()))),
                Err((expected, Some(kind)))
            );
        }
    }

    /// A bootstrap listener recorded in `state` as a running daemon records
    /// it. Its one connection reads a request line, hands it to the test and
    /// then does what `respond` does with the stream.
    fn hook_listener(
        state: &Path,
        respond: impl FnOnce(&Value, &mut LocalStream) + Send + 'static,
    ) -> (mpsc::Receiver<Value>, std::thread::JoinHandle<()>) {
        private::create_dir_all(&state.join("pane-capabilities")).unwrap();
        let (listener, _) = crate::pane_auth::bind(state).unwrap();
        let (heard, hears) = mpsc::channel();
        let served = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let _ = heard.send(request.clone());
            respond(&request, &mut stream);
        });
        (hears, served)
    }

    /// The gap Windows hooks fell through (letter-7785): a hook's request
    /// crosses a real local stream to the bootstrap listener and reads its
    /// answer. Memory's prompt makes the request longer than a Windows pipe's
    /// 512-byte buffer, so its write waits on the listener's reads.
    #[test]
    fn a_hook_request_reaches_the_bootstrap_listener_and_reads_its_answer() {
        let state = short_dir();
        let (hears, served) = hook_listener(state.path(), |request, stream| {
            let answer =
                json!({"ok": true, "request_id": request["hook"]["request_id"], "result": {}});
            writeln!(stream, "{answer}").unwrap();
        });
        let prompt = "x".repeat(8 * 1024);
        let command = hide_node_link::panes::HookCommand::Memory {
            event: "UserPromptSubmit".to_owned(),
            runtime: "claude".to_owned(),
            session: "session".to_owned(),
            prompt: prompt.clone(),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let answer = hook_request(&env_at(state.path()), None, command, deadline).unwrap();
        let request = hears.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["request_id"], request["hook"]["request_id"]);
        assert_eq!(request["hook"]["command"]["prompt"], prompt.as_str());
        served.join().unwrap();
    }

    /// Writes a bootstrap record in `state` naming `socket`, as a daemon
    /// would, without a listener behind it.
    fn record_socket(state: &Path, socket: &Path) {
        let record = crate::pane_auth::bootstrap_socket_record(state);
        private::create_dir_all(record.parent().unwrap()).unwrap();
        private::create_new_file(&record)
            .unwrap()
            .write_all(socket.to_str().unwrap().as_bytes())
            .unwrap();
    }

    /// Each step a hook's request can stop at answers its own reason, which
    /// the hook records under a cause of its own.
    #[test]
    fn each_step_a_hook_request_stops_at_has_its_own_reason_and_cause() {
        let ask = |state: &Path, within: Duration| {
            let command = hide_node_link::panes::HookCommand::WorkspaceInfo {};
            let deadline = Instant::now() + within;
            let reason = hook_request(&env_at(state), None, command, deadline).unwrap_err();
            let cause = hide_agent_hooks::delivery::refusal_cause(Some(&reason));
            (reason, cause)
        };
        let within = Duration::from_secs(10);
        let step = |reason: &str, cause| (reason.to_owned(), cause);

        // No daemon published its socket.
        let state = short_dir();
        assert_eq!(
            ask(state.path(), within),
            step("hide_unavailable", "unavailable")
        );

        // The record names a folder that is gone.
        let state = short_dir();
        record_socket(state.path(), &state.path().join("gone").join("b.sock"));
        assert_eq!(
            ask(state.path(), within),
            step("invalid_bootstrap_socket_record", "record")
        );

        // The record names a private folder where nothing listens.
        let state = short_dir();
        let folder = state.path().join("sockets");
        private::create_dir(&folder).unwrap();
        record_socket(state.path(), &folder.join("b.sock"));
        assert_eq!(
            ask(state.path(), within),
            step("hide_unreachable", "connect")
        );

        // The listener reads the request and ends without an answer.
        let state = short_dir();
        let (_, served) = hook_listener(state.path(), |_, _| {});
        assert_eq!(
            ask(state.path(), within),
            step("invalid_response", "answer")
        );
        served.join().unwrap();

        // The listener reads the request and never answers.
        let state = short_dir();
        let (release, released) = mpsc::channel::<()>();
        let (_, served) = hook_listener(state.path(), move |_, _| {
            // Bounded only so a request that never times out fails the test.
            let _ = released.recv_timeout(Duration::from_secs(20));
        });
        assert_eq!(
            ask(state.path(), Duration::from_millis(300)),
            step("request_timeout", "timeout")
        );
        drop(release);
        served.join().unwrap();
    }

    // Mode bits widen the file for the refusal.
    #[cfg(unix)]
    #[test]
    fn reference_reader_refuses_symlink_and_open_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reference.json");
        fs::write(&path, br#"{"token":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","port":12345,"origin_port":12345}"#).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&path, permissions.clone()).unwrap();
        assert!(matches!(
            read_reference(&path).unwrap().route,
            Route::Daemon {
                port: 12345,
                origin_port: 12345
            }
        ));
        let link = directory.path().join("link.json");
        symlink(&path, &link).unwrap();
        assert!(matches!(read_reference(&link), Err(reason) if reason == "credential_expired"));
        permissions.set_mode(0o644);
        fs::set_permissions(&path, permissions).unwrap();
        assert!(matches!(read_reference(&path), Err(reason) if reason == "invalid_reference"));
    }

    #[cfg(unix)]
    #[test]
    fn an_empty_reference_reads_as_gone() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reference.json");
        fs::write(&path, b"").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(read_reference(&path), Err(reason) if reason == "credential_expired"));
    }

    /// What a node writes up its link, one line at a time.
    struct Lines(Vec<u8>, mpsc::Sender<String>);

    impl Write for Lines {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.extend_from_slice(bytes);
            while let Some(end) = self.0.iter().position(|byte| *byte == b'\n') {
                let line = self.0.drain(..=end).collect::<Vec<_>>();
                let _ = self
                    .1
                    .send(String::from_utf8_lossy(&line).trim().to_owned());
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A Unix socket path is limited to about a hundred bytes, and the node
    /// binds two folders below this one, so a long TMPDIR overflows it.
    fn short_dir() -> tempfile::TempDir {
        if cfg!(unix) {
            tempfile::Builder::new().prefix("hb").tempdir_in("/tmp")
        } else {
            tempfile::Builder::new().prefix("hb").tempdir()
        }
        .unwrap()
    }

    fn next_event(lines: &mpsc::Receiver<String>) -> Value {
        serde_json::from_str(&lines.recv_timeout(Duration::from_secs(10)).unwrap()).unwrap()
    }

    /// A device's node and a pane's `hide` meet over the system's local
    /// stream: the node sees the caller's pid and asks the pane's Herdr about
    /// it, and stopping the node removes it from this device.
    #[test]
    fn a_device_node_answers_this_device_until_it_stops() {
        let directory = short_dir();
        let bridges = directory.path().join("bridges");
        let herdr = directory.path().join("no-herdr.sock");
        let (sent, _lines) = mpsc::channel();
        let output = std::sync::Mutex::new(Lines(Vec::new(), sent));
        let panes = hide_host::panes::Panes::new();
        let request = json!({"pane_id": "w1:p1", "nonce": "0".repeat(32)});
        std::thread::scope(|scope| {
            let started = panes
                .start(scope, &output, &bridges, herdr.to_str().unwrap())
                .unwrap();
            // Both ends read and write the wire spelling, which on Windows is
            // the only one `from_wire` reads.
            assert!(
                node_socket(&started.socket).unwrap().starts_with(&bridges),
                "{}",
                started.socket
            );
            // No Herdr answers at the socket the node was given, so it cannot
            // read the pane; it got that far only by seeing who called.
            assert_eq!(
                bootstrap_bridges(&bridges, &request),
                Err("pane_unavailable".to_owned())
            );
            panes.stop();
        });
        panes.remove_folder();
        assert_eq!(bootstrap_bridges(&bridges, &request), Ok(None));
    }

    /// A command's bytes cross the node both ways, the first of them sent
    /// with the line that opens the stream, and the core closing the stream
    /// ends it for the command.
    #[test]
    fn a_command_stream_crosses_the_node_both_ways() {
        use base64::Engine as _;
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let directory = short_dir();
        let bridges = directory.path().join("bridges");
        let herdr = directory.path().join("no-herdr.sock");
        let (sent, lines) = mpsc::channel();
        let output = std::sync::Mutex::new(Lines(Vec::new(), sent));
        let panes = hide_host::panes::Panes::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        std::thread::scope(|scope| {
            let started = panes
                .start(scope, &output, &bridges, herdr.to_str().unwrap())
                .unwrap();
            let socket = node_socket(&started.socket).unwrap();
            runtime.block_on(async {
                let mut stream = node_stream(&socket, None).unwrap();
                stream.write_all(b"ping").await.unwrap();
                let opened = next_event(&lines);
                assert_eq!(opened["event"], "stream_open");
                let id = opened["stream"].as_u64().unwrap();
                let data = next_event(&lines);
                assert_eq!(
                    data,
                    json!({"event":"stream_data","stream":id,"data":encode(b"ping")})
                );
                panes.write_stream(id, &encode(b"pong")).unwrap();
                let mut answer = [0_u8; 4];
                stream.read_exact(&mut answer).await.unwrap();
                assert_eq!(&answer, b"pong");
                panes.close_stream(id).unwrap();
                let mut rest = Vec::new();
                stream.read_to_end(&mut rest).await.unwrap();
                assert!(rest.is_empty());
                assert_eq!(
                    next_event(&lines),
                    json!({"event":"stream_closed","stream":id})
                );
            });
            panes.stop();
        });
        panes.remove_folder();
    }

    /// The environment a `hide` whose state folder is `directory` runs with.
    fn env_at(directory: &Path) -> Env {
        Env {
            home: directory.to_path_buf(),
            herdr_socket_path: None,
            herdr_bin_path: None,
            login_shell: None,
            state_dir: directory.to_path_buf(),
            legacy_state_dir: None,
            account_state_folder: false,
            keep_alive: true,
            vite_origin: None,
            bind: "127.0.0.1:0".parse().unwrap(),
            idle_secs: 600,
            build: None,
            starter_program: None,
            open_command: None,
            host_helper_root: None,
            host_cli_dir: None,
            pane_id: None,
            tailscale_bin: Some(directory.join("no-tailscale")),
            search_path: None,
        }
    }

    /// Waits, at most a hang guard's while, until `done` holds.
    #[allow(clippy::disallowed_methods)] // polls a state the daemon changes on its own task, bounded by a deadline
    fn until(mut done: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !done() {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }

    /// A one-shot command against a running daemon sends no claim and
    /// writes no claimed marker, and the daemon revokes its token once the
    /// connection ends; a named reference's command still claims. This
    /// process stands in for the pane's shell, so every credential stays
    /// alive until the daemon or the command ends it.
    #[test]
    fn a_one_shot_command_is_not_claimed_and_its_token_ends_with_its_connection() {
        let directory = tempfile::tempdir().unwrap();
        let env = env_at(directory.path());
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let running = runtime
            .block_on(crate::start_daemon(env.clone()))
            .expect("start daemon");
        let registry = Arc::clone(&running.pane_capabilities);
        let shell = i32::try_from(std::process::id()).unwrap();
        let attestation = crate::pane_auth::Attestation::pane(
            "w1:p1",
            herdr_core::workspace_control::Context {
                device_id: "local".to_owned(),
                workspace_id: "workspace".to_owned(),
                checkout_id: "checkout".to_owned(),
                checkout_path: "/checkout".to_owned(),
            },
            shell,
        );
        let holder = (shell, hide_node::pane_proof::process_start(shell).unwrap());
        let issue = |nonce: char, holder: Option<(i32, u64)>| {
            let (path, _) = registry
                .issue(
                    &attestation,
                    &nonce.to_string().repeat(32),
                    running.port,
                    holder,
                )
                .unwrap();
            let token = read_reference(&path).unwrap().token;
            (path, token)
        };
        let one_shot = |path: &Path| Credential {
            env: &env,
            path: path.to_path_buf(),
            named: false,
            _one_shot: Some(OneShotReference(path.to_path_buf())),
        };
        // This daemon has no Herdr to read the pane from, so each command is
        // answered with that refusal; what follows an answer is the same.
        let answered = |credential: &mut Credential| {
            let answer = request(credential, "info").unwrap();
            assert_eq!(answer["reason"], "pane_unavailable", "{answer}");
        };

        // A one-shot credential as a bare command bootstraps it: the daemon
        // ends the token with the connection.
        let (path, token) = issue('1', Some(holder));
        let mut credential = one_shot(&path);
        answered(&mut credential);
        assert!(
            until(|| registry.get(&token).is_none()),
            "the daemon kept a one-shot token past its connection"
        );
        drop(credential);
        assert!(!path.exists());

        // The same command on a token the daemon would keep: it sends no
        // claim, so neither side records one.
        let (path, token) = issue('2', None);
        let mut credential = one_shot(&path);
        answered(&mut credential);
        assert!(!path.with_extension("claimed").exists());
        assert_eq!(registry.claimed(&token), Some(false));
        drop(credential);

        // A named reference is persistent, and its command claims it.
        let (path, token) = issue('3', None);
        let mut credential = Credential::named(&env, path.clone());
        answered(&mut credential);
        assert!(path.with_extension("claimed").is_file());
        assert_eq!(registry.claimed(&token), Some(true));

        running.stop();
        drop(running);
        drop(runtime);
    }

    #[test]
    fn action_result_survives_a_lost_claim_ack() {
        let (sender, receiver) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                sender.send(listener.local_addr().unwrap().port()).unwrap();
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let _ = socket.next().await;
                let _ = socket.next().await;
                socket
                    .send(Message::Text(
                        json!({
                            "type":"workspace_result","request_id":"1-retry","ok":true,
                            "result":{"view_id":"view-1"}
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
                socket.close(None).await.unwrap();
            });
        });
        let port = receiver.recv().unwrap();
        let reference = Reference {
            token: "a".repeat(64),
            route: Route::Daemon {
                port,
                origin_port: port,
            },
        };
        let directory = tempfile::tempdir().unwrap();
        let answer = run_exchange(
            &directory.path().join("reference.json"),
            &reference,
            json!({"type":"workspace_action","request_id":"1-retry"}),
            "1-retry",
            true,
            AfterAnswer::Claim,
        )
        .unwrap();
        assert_eq!(answer["result"]["view_id"], "view-1");
        server.join().unwrap();
    }
}
