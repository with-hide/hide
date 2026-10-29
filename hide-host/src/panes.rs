//! A device's panes on its node link (PRD core-host-node B18, B19). The node
//! listens on an owner-only local stream (`hide_platform::ipc`: a Unix socket,
//! a named pipe on Windows) in a fresh `bridge-*` folder of the account's
//! `workspace-bridges` folder, where a pane's `hide` finds it; several cores
//! may serve one device, so each node keeps its own folder.
//!
//! A caller asks either for a credential or for a stream. A credential
//! request is proved with this machine's kernel (the pid the system reports
//! for the stream's other end, descended from the pane's shell) and sent up
//! the link as [`NodeEvent::PaneProof`]; the core answers down, and the node
//! writes the reference file the caller reads. On the screen machine's node
//! a caller in no pane is proved by its working directory instead
//! ([`NodeEvent::CheckoutProof`]). A stream carries one `hide`
//! command's bytes up and down the same link, and opens with the caller the
//! node proved when it holds a one-shot credential issued here
//! ([`StreamCaller`]). Everything here lives only as
//! long as the link: the node's input closing ends the listener, every
//! stream and every reference with it.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread::Scope;
use std::time::{Duration, Instant};

use base64::Engine as _;
use hide_node_link::panes::{
    MAX_CHUNK, MAX_STREAMS, NodeEvent, PROJECT_OPEN_ACCEPT, PROJECT_OPEN_ANSWER,
    PROJECT_OPENS_PER_MINUTE, PaneIdentity, PanesStarted, ProofAnswer, StreamCaller,
};
use hide_platform::fs::private;
use hide_platform::ipc::{ListenerCloser, LocalListener, LocalStream, ShutdownHandle};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{ErrorCode, HostError, HostResult};
use crate::pane_peer;

const CLIENT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_CONTROL_LINE: u64 = hide_node_link::panes::HOOK_REQUEST_LIMIT as u64;
const UNCLAIMED_LIFETIME: Duration = Duration::from_secs(30);
const REFERENCE_LIFETIME: Duration = Duration::from_secs(8 * 60 * 60);
const SWEEP_INTERVAL: Duration = Duration::from_secs(5);
/// Callers served at once; more wait in the system's backlog.
const MAX_CLIENTS: usize = 16;
/// How old a `bridge-*` folder whose socket does not answer must be before
/// a starting service removes it.
const DEAD_FOLDER_AGE: Duration = Duration::from_secs(60);
/// How long a write to a stream's caller may block before the stream ends.
const STREAM_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

struct IssuedReference {
    token: String,
    created: Instant,
    holder: Option<(i32, u64)>,
    /// The pane's shell and its start, for a pane caller's reference, which
    /// ends with the shell; a checkout caller's has none.
    shell: Option<(i32, u64)>,
    /// The pane a pane caller's reference was proved for.
    pane_id: Option<String>,
}

/// What the node proved about a caller.
enum Proved {
    Pane(PaneIdentity),
    /// The wire spelling of the caller's canonical working directory.
    Checkout(String),
}

/// A credential request, as a pane's `hide` sends it.
#[derive(Deserialize)]
struct Bootstrap {
    pane_id: String,
    nonce: String,
    #[serde(default)]
    one_shot: bool,
    #[serde(default)]
    hook: Option<hide_node_link::panes::HookRequest>,
}

#[derive(Clone)]
enum BootstrapAnswer {
    Reference { path: PathBuf, info: Option<Value> },
    Hook(Value),
}

struct Started {
    herdr_socket: PathBuf,
    socket_wire: String,
    folder: PathBuf,
    closer: ListenerCloser,
}

/// The pane service of one node link. Created idle with the serve loop and
/// started by the core's `panes_start`.
pub struct Panes {
    started: OnceLock<Started>,
    start_lock: Mutex<()>,
    stopped: AtomicBool,
    issued: Mutex<HashMap<PathBuf, IssuedReference>>,
    proofs: Mutex<HashMap<u64, mpsc::SyncSender<ProofAnswer>>>,
    streams: Mutex<HashMap<u64, StreamEnd>>,
    clients: AtomicUsize,
    next: AtomicU64,
    checkout_callers: bool,
    opens: Mutex<Opens>,
}

/// The plain shells' `hide <path>` on this link: one in flight, and when
/// the last [`PROJECT_OPENS_PER_MINUTE`] started.
#[derive(Default)]
struct Opens {
    in_flight: bool,
    started: std::collections::VecDeque<Instant>,
}

/// An open in flight, until it is dropped.
struct OpenSlot<'a>(&'a Mutex<Opens>);

impl Drop for OpenSlot<'_> {
    fn drop(&mut self) {
        lock(self.0).in_flight = false;
    }
}

/// A `hide <path>` from a plain shell, as its `hide` sends it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectOpenRequest {
    path: String,
    request_id: String,
}

struct StreamEnd {
    writer: LocalStream,
    shutdown: ShutdownHandle,
}

impl Default for Panes {
    fn default() -> Self {
        Self::new()
    }
}

impl Panes {
    pub fn new() -> Self {
        Self {
            started: OnceLock::new(),
            start_lock: Mutex::new(()),
            stopped: AtomicBool::new(false),
            issued: Mutex::new(HashMap::new()),
            proofs: Mutex::new(HashMap::new()),
            streams: Mutex::new(HashMap::new()),
            clients: AtomicUsize::new(0),
            next: AtomicU64::new(1),
            checkout_callers: false,
            opens: Mutex::new(Opens::default()),
        }
    }

    /// Proves a caller in no pane by its working directory, as
    /// [`crate::serve::Services::checkout_callers`] says.
    pub fn with_checkout_callers(mut self, checkout_callers: bool) -> Self {
        self.checkout_callers = checkout_callers;
        self
    }

    /// Binds the listener in a new folder under `bridges` and starts serving
    /// it on `scope`. Asked again for the same Herdr, answers the same
    /// socket; another Herdr is refused, since a node serves one.
    pub fn start<'scope, 'env, W: Write + Send>(
        &'scope self,
        scope: &'scope Scope<'scope, 'env>,
        output: &'scope Mutex<W>,
        bridges: &Path,
        herdr_socket: &str,
    ) -> HostResult<PanesStarted> {
        let herdr_socket = PathBuf::from(herdr_socket);
        if !herdr_socket.is_absolute() {
            return Err(HostError::new(
                ErrorCode::InvalidPath,
                "The Herdr socket is not an absolute path",
            ));
        }
        let _starting = self
            .start_lock
            .lock()
            .map_err(|_| HostError::new(ErrorCode::Io, "The pane service lock was poisoned"))?;
        if let Some(started) = self.started.get() {
            return if started.herdr_socket == herdr_socket {
                Ok(PanesStarted {
                    socket: started.socket_wire.clone(),
                })
            } else {
                Err(HostError::new(
                    ErrorCode::Conflict,
                    "This node already serves the panes of another Herdr server",
                ))
            };
        }
        if self.stopped.load(Ordering::Acquire) {
            return Err(HostError::new(ErrorCode::Io, "The node is stopping"));
        }
        let io = |error: io::Error| HostError::new(ErrorCode::Io, error.to_string());
        validate_dir(bridges).map_err(io)?;
        remove_dead_folders(bridges);
        let folder = tempfile::Builder::new()
            .prefix("bridge-")
            .tempdir_in(bridges)
            .map_err(io)?
            .keep();
        let socket = folder.join("bootstrap.sock");
        let bound = (|| {
            private::restrict_to_owner(&folder)?;
            let wire = hide_platform::path::to_wire(&socket)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            let listener = LocalListener::bind(&socket)?;
            Ok::<_, io::Error>((wire, listener))
        })();
        let (socket_wire, listener) = match bound {
            Ok(bound) => bound,
            Err(error) => {
                let _ = fs::remove_dir_all(&folder);
                return Err(io(error));
            }
        };
        let started = Started {
            herdr_socket,
            socket_wire: socket_wire.clone(),
            folder,
            closer: listener.closer(),
        };
        if self.started.set(started).is_err() {
            unreachable!("only the start lock's holder sets the service");
        }
        scope.spawn(move || self.accept_loop(scope, output, listener));
        Ok(PanesStarted {
            socket: socket_wire,
        })
    }

    /// The Herdr server this service serves, once started.
    pub fn herdr_socket(&self) -> Option<String> {
        self.started
            .get()
            .map(|started| started.herdr_socket.display().to_string())
    }

    /// Ends the listener, every stream and every waiting proof, and removes
    /// the folder with the references in it. The serve loop calls it when
    /// its input ends.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        // Waits out a start in progress, so its listener is closed too.
        drop(self.start_lock.lock());
        if let Some(started) = self.started.get() {
            started.closer.close();
        }
        for (_, end) in lock(&self.streams).drain() {
            end.shutdown.shutdown();
        }
        lock(&self.proofs).clear();
    }

    /// Removes the service's folder; called once every thread of it ended.
    pub fn remove_folder(&self) {
        if let Some(started) = self.started.get() {
            let _ = fs::remove_dir_all(&started.folder);
        }
    }

    /// Hands the core's answer to the request waiting for it; one nobody
    /// waits for any more (its caller left, or it timed out) is dropped.
    pub fn answer_proof(&self, request: u64, answer: ProofAnswer) -> HostResult<Value> {
        let persistent_info = matches!(answer, ProofAnswer::WorkspaceInfo { .. });
        let mut proofs = lock(&self.proofs);
        // An accepted open waits on for its outcome under the same request.
        let waiting = if answer == ProofAnswer::ProjectOpenAccepted {
            proofs.get(&request).cloned()
        } else {
            proofs.remove(&request)
        };
        drop(proofs);
        let accepted = waiting.is_some_and(|waiting| waiting.try_send(answer).is_ok());
        // A late guidance answer must not claim a reference no caller got.
        Ok(if persistent_info {
            json!(accepted)
        } else {
            Value::Null
        })
    }

    /// `pane_id`'s identity now, from the Herdr this service was started for.
    pub fn inspect(&self, pane_id: &str) -> HostResult<PaneIdentity> {
        let started = self.started.get().ok_or_else(|| {
            HostError::new(ErrorCode::Unsupported, "The pane service is not started")
        })?;
        pane_peer::inspect(&started.herdr_socket, pane_id)
            .map_err(|reason| HostError::new(ErrorCode::NotFound, reason))
    }

    /// Writes base64 `data` to `stream`'s caller. A caller that does not
    /// take it in time loses its stream.
    pub fn write_stream(&self, stream: u64, data: &str) -> HostResult<Value> {
        if data.len() > MAX_CHUNK.div_ceil(3) * 4 {
            return Err(HostError::new(
                ErrorCode::TooLarge,
                "A stream chunk is larger than the link carries",
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| {
                HostError::new(ErrorCode::InvalidRequest, "A stream chunk is not base64")
            })?;
        let mut writer = {
            let streams = lock(&self.streams);
            let end = streams
                .get(&stream)
                .ok_or_else(|| HostError::new(ErrorCode::NotFound, "The stream has ended"))?;
            end.writer.duplicate()
        };
        if let Err(error) = writer.write_all(&bytes) {
            self.end_stream(stream);
            return Err(HostError::new(ErrorCode::Io, error.to_string()));
        }
        Ok(Value::Null)
    }

    /// Ends `stream` from the core's side; its reader then reports it closed.
    pub fn close_stream(&self, stream: u64) -> HostResult<Value> {
        self.end_stream(stream);
        Ok(Value::Null)
    }

    fn end_stream(&self, stream: u64) {
        if let Some(end) = lock(&self.streams).remove(&stream) {
            end.shutdown.shutdown();
        }
    }

    fn accept_loop<'scope, 'env, W: Write + Send>(
        &'scope self,
        scope: &'scope Scope<'scope, 'env>,
        output: &'scope Mutex<W>,
        listener: LocalListener,
    ) {
        // Bounded like the pane bootstrap's queue: a flood waits in the
        // system's backlog, not as open streams here.
        let (accepted, arrivals) = mpsc::sync_channel(MAX_CLIENTS);
        scope.spawn(move || {
            loop {
                let arrival = listener.accept();
                let failed = arrival.is_err();
                if accepted.send(arrival).is_err() || failed {
                    return;
                }
            }
        });
        let mut last_sweep = Instant::now();
        loop {
            if last_sweep.elapsed() >= SWEEP_INTERVAL {
                if self.sweep_references(output).is_err() {
                    return;
                }
                last_sweep = Instant::now();
            }
            let wait = SWEEP_INTERVAL.saturating_sub(last_sweep.elapsed());
            match arrivals.recv_timeout(wait) {
                Ok(Ok(mut stream)) => {
                    if self.clients.fetch_add(1, Ordering::AcqRel) >= MAX_CLIENTS {
                        self.clients.fetch_sub(1, Ordering::AcqRel);
                        let _ = answer_client(&mut stream, Err("bridge_busy".to_owned()));
                        report_refusal(output, None, "bridge_busy");
                        continue;
                    }
                    scope.spawn(move || {
                        self.serve_client(stream, output);
                        self.clients.fetch_sub(1, Ordering::AcqRel);
                    });
                }
                Ok(Err(_)) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            if self.stopped.load(Ordering::Acquire) {
                return;
            }
        }
    }

    fn serve_client(&self, mut stream: LocalStream, output: &Mutex<impl Write>) {
        let first = (|| {
            stream
                .set_read_timeout(Some(CLIENT_TIMEOUT))
                .map_err(|_| "bridge_unavailable")?;
            stream
                .set_write_timeout(Some(CLIENT_TIMEOUT))
                .map_err(|_| "bridge_unavailable")?;
            // A byte at a time: a stream's first bytes follow its line at
            // once, and a buffered reader would keep them from the stream.
            let mut line = Vec::new();
            let mut byte = [0_u8; 1];
            while line.last() != Some(&b'\n') {
                if line.len() as u64 >= MAX_CONTROL_LINE
                    || stream.read(&mut byte).map_err(|_| "invalid_request")? == 0
                {
                    return Err("invalid_request");
                }
                line.push(byte[0]);
            }
            serde_json::from_slice::<Value>(&line).map_err(|_| "invalid_request")
        })();
        match first {
            Ok(request) if request.get("stream") == Some(&Value::Bool(true)) => {
                self.serve_stream(stream, request.get("token").and_then(Value::as_str), output)
            }
            Ok(request) if request.get("project_open").is_some() => {
                self.serve_project_open(stream, request, output)
            }
            Ok(request) => self.serve_bootstrap(stream, request, output),
            Err(reason) => {
                let _ = answer_client(&mut stream, Err(reason.to_owned()));
                report_refusal(output, None, reason);
            }
        }
    }

    fn serve_bootstrap(&self, mut stream: LocalStream, request: Value, output: &Mutex<impl Write>) {
        // The pane the caller names, as far as a record should carry it.
        let named = request
            .get("pane_id")
            .and_then(Value::as_str)
            .filter(|pane_id| pane_id.len() <= 256)
            .map(str::to_owned);
        let Some(started) = self.started.get() else {
            let _ = answer_client(&mut stream, Err("bridge_unavailable".to_owned()));
            report_refusal(output, named, "bridge_unavailable");
            return;
        };
        let mut core_refused = false;
        let mut issued_token = None;
        let mut token_new = false;
        let mut created_reference = None;
        let result: Result<BootstrapAnswer, String> = (|| {
            let peer = stream
                .peer_pid()
                .ok()
                .and_then(|pid| i32::try_from(pid).ok())
                .ok_or("caller_unavailable")?;
            let request: Bootstrap =
                serde_json::from_value(request).map_err(|_| "invalid_request")?;
            if request.nonce.len() != 32
                || !request.nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err("invalid_nonce".to_owned());
            }
            if request
                .hook
                .as_ref()
                .is_some_and(|hook| request.one_shot != hook.command.one_shot())
            {
                return Err("invalid_request".to_owned());
            }
            let pane =
                pane_peer::inspect(&started.herdr_socket, &request.pane_id).and_then(|identity| {
                    if pane_peer::descends_from(peer, identity.shell_pid) {
                        Ok(identity)
                    } else {
                        Err("caller_not_in_pane")
                    }
                });
            let proved = match pane {
                Ok(identity) => Proved::Pane(identity),
                Err(_) if self.checkout_callers => {
                    Proved::Checkout(caller_directory(peer).ok_or("caller_unavailable")?)
                }
                Err(reason) => return Err(reason.to_owned()),
            };
            let holder = if request.one_shot {
                Some((
                    peer,
                    pane_peer::process_start(peer).ok_or("caller_unavailable")?,
                ))
            } else {
                None
            };
            self.sweep_references(output)
                .map_err(|_| "bridge_unavailable")?;
            let id = self.next.fetch_add(1, Ordering::Relaxed);
            let (waiting, answer) = mpsc::sync_channel(1);
            lock(&self.proofs).insert(id, waiting);
            let event = match &proved {
                Proved::Pane(identity) => NodeEvent::PaneProof {
                    request: id,
                    pane_id: request.pane_id.clone(),
                    identity: identity.clone(),
                    nonce: request.nonce.clone(),
                    one_shot: request.one_shot,
                    hook: request.hook.clone(),
                },
                Proved::Checkout(path) => NodeEvent::CheckoutProof {
                    request: id,
                    path: path.clone(),
                    nonce: request.nonce.clone(),
                    one_shot: request.one_shot,
                    hook: request.hook.clone(),
                },
            };
            let sent = write_event(output, &event);
            let answer = sent
                .ok()
                .and_then(|()| answer.recv_timeout(CLIENT_TIMEOUT).ok());
            lock(&self.proofs).remove(&id);
            let mut info = None;
            let token = match answer.ok_or("bridge_unavailable")? {
                ProofAnswer::Hook { answer }
                    if request.hook.is_some() && (request.one_shot || answer["ok"] != true) =>
                {
                    if answer.to_string().len() + 1 > hide_node_link::panes::HOOK_RESPONSE_LIMIT {
                        return Err("invalid_response".to_owned());
                    }
                    return Ok(BootstrapAnswer::Hook(answer));
                }
                ProofAnswer::Hook { .. } => return Err("invalid_response".to_owned()),
                ProofAnswer::WorkspaceInfo {
                    token,
                    issued_new,
                    answer,
                } => {
                    token_new = issued_new;
                    issued_token = Some(token.clone());
                    if !request
                        .hook
                        .as_ref()
                        .is_some_and(|hook| !hook.command.one_shot())
                        || answer["ok"] != true
                        || answer.to_string().len() + 1 > hide_node_link::panes::HOOK_RESPONSE_LIMIT
                        || token.len() != 64
                        || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        return Err("invalid_response".to_owned());
                    }
                    info = Some(answer);
                    token
                }
                ProofAnswer::Refused { reason, .. } => {
                    core_refused = true;
                    return Err(reason);
                }
                // An open's answers never pair a credential request.
                ProofAnswer::ProjectOpenAccepted | ProofAnswer::ProjectOpen { .. } => {
                    return Err("invalid_response".to_owned());
                }
                ProofAnswer::Issued { token, issued_new } => {
                    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        return Err("invalid_response".to_owned());
                    }
                    token_new = issued_new;
                    if request.hook.is_some() {
                        issued_token = Some(token);
                        return Err("invalid_response".to_owned());
                    }
                    token
                }
            };
            issued_token = Some(token.clone());
            let mut issued = lock(&self.issued);
            if !request.one_shot
                && let Some((path, _)) = issued.iter().find(|(path, reference)| {
                    reference.token == token && path.is_file() && reference.holder.is_none()
                })
            {
                if info.is_some() && !path.with_extension("claimed").is_file() {
                    private::create_new_file(&path.with_extension("claimed"))
                        .map_err(|_| "reference_unavailable")?;
                }
                return Ok(BootstrapAnswer::Reference {
                    path: path.clone(),
                    info,
                });
            }
            let path = started.folder.join(format!("{}.json", request.nonce));
            let mut file = private::create_new_file(&path).map_err(|_| "reference_unavailable")?;
            // Not flushed to disk: the reference lives no longer than the
            // token it names, which no restart keeps, and only this
            // machine reads it.
            if write!(
                file,
                "{}",
                json!({"token": token, "socket": started.socket_wire})
            )
            .is_err()
            {
                let _ = fs::remove_file(&path);
                return Err("reference_unavailable".to_owned());
            }
            issued.insert(
                path.clone(),
                IssuedReference {
                    token,
                    created: Instant::now(),
                    holder,
                    shell: match &proved {
                        Proved::Pane(identity) => {
                            Some((identity.shell_pid, identity.shell_started))
                        }
                        Proved::Checkout(_) => None,
                    },
                    pane_id: match &proved {
                        Proved::Pane(_) => Some(request.pane_id.clone()),
                        Proved::Checkout(_) => None,
                    },
                },
            );
            created_reference = Some(path.clone());
            if info.is_some() {
                private::create_new_file(&path.with_extension("claimed"))
                    .map_err(|_| "reference_unavailable")?;
            }
            Ok(BootstrapAnswer::Reference { path, info })
        })();
        let delivered = answer_client(&mut stream, result.clone()).is_ok();
        // The core records the refusals it answered itself.
        if let Err(reason) = &result
            && !core_refused
        {
            report_refusal(output, named, reason);
        }
        if !delivered || result.is_err() {
            if let Some(path) = created_reference {
                let _ = fs::remove_file(&path);
                let _ = fs::remove_file(path.with_extension("claimed"));
                lock(&self.issued).remove(&path);
            }
            if token_new && let Some(token) = issued_token {
                let _ = write_event(output, &NodeEvent::Revoke { token });
            }
        }
    }

    /// `hide <path>` from a shell in no pane: proved only as a process of
    /// this node's own account, which the system names for the stream's
    /// other end, and answered with nothing but what the core's open says.
    /// No credential is issued and no stream opened, so the caller gains no
    /// other command.
    fn serve_project_open(
        &self,
        mut stream: LocalStream,
        request: Value,
        output: &Mutex<impl Write>,
    ) {
        let result = self.project_open(&stream, request, output);
        let answer = match result {
            Ok(answer) => answer,
            Err(reason) => {
                report_refusal(output, None, &reason);
                json!({
                    "type": "workspace_result", "ok": false, "reason": reason,
                    "next_action": hide_node_link::panes::project_open_next_action(&reason),
                })
            }
        };
        let _ = writeln!(stream, "{answer}");
    }

    fn project_open(
        &self,
        stream: &LocalStream,
        request: Value,
        output: &Mutex<impl Write>,
    ) -> Result<Value, String> {
        if !stream.peer_is_this_account().unwrap_or(false) {
            return Err("caller_not_this_account".to_owned());
        }
        let open: ProjectOpenRequest = request
            .get("project_open")
            .cloned()
            .and_then(|open| serde_json::from_value(open).ok())
            .ok_or("invalid_request")?;
        let printable = |text: &str, limit: usize| {
            !text.is_empty() && text.len() <= limit && !text.chars().any(char::is_control)
        };
        if !printable(&open.path, 4096)
            || !hide_platform::path::is_wire_absolute(&open.path)
            || !printable(&open.request_id, 128)
        {
            return Err("invalid_request".to_owned());
        }
        if self.started.get().is_none() || self.stopped.load(Ordering::Acquire) {
            return Err("bridge_unavailable".to_owned());
        }
        let _slot = self.claim_open()?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        // Room for the acceptance and the outcome.
        let (waiting, answers) = mpsc::sync_channel(2);
        lock(&self.proofs).insert(id, waiting);
        let event = NodeEvent::ProjectOpen {
            request: id,
            path: open.path,
            request_id: open.request_id,
        };
        let outcome = write_event(output, &event)
            .ok()
            .and_then(|()| answers.recv_timeout(PROJECT_OPEN_ACCEPT).ok())
            .and_then(|first| match first {
                ProofAnswer::ProjectOpenAccepted => answers.recv_timeout(PROJECT_OPEN_ANSWER).ok(),
                other => Some(other),
            });
        lock(&self.proofs).remove(&id);
        match outcome.ok_or("bridge_unavailable")? {
            ProofAnswer::ProjectOpen { answer } => Ok(answer),
            ProofAnswer::Refused { reason, .. } => Err(reason),
            _ => Err("invalid_response".to_owned()),
        }
    }

    /// Takes the one open this link runs at a time, within the minute's
    /// bound.
    fn claim_open(&self) -> Result<OpenSlot<'_>, String> {
        let mut opens = lock(&self.opens);
        if opens.in_flight {
            return Err("project_open_busy".to_owned());
        }
        let now = Instant::now();
        while opens
            .started
            .front()
            .is_some_and(|started| now.duration_since(*started) >= Duration::from_secs(60))
        {
            opens.started.pop_front();
        }
        if opens.started.len() >= PROJECT_OPENS_PER_MINUTE {
            return Err("project_open_rate_limited".to_owned());
        }
        opens.started.push_back(now);
        opens.in_flight = true;
        Ok(OpenSlot(&self.opens))
    }

    fn serve_stream(&self, stream: LocalStream, token: Option<&str>, output: &Mutex<impl Write>) {
        if stream.set_read_timeout(None).is_err() {
            return;
        }
        // The caller's writes are paced by the core; a caller that stops
        // reading loses its stream rather than holding a node worker.
        if stream
            .set_write_timeout(Some(STREAM_WRITE_TIMEOUT))
            .is_err()
        {
            return;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        {
            let mut streams = lock(&self.streams);
            if self.stopped.load(Ordering::Acquire) {
                return;
            }
            if streams.len() >= MAX_STREAMS {
                drop(streams);
                report_refusal(output, None, "streams_full");
                return;
            }
            streams.insert(
                id,
                StreamEnd {
                    writer: stream.duplicate(),
                    shutdown: stream.shutdown_handle(),
                },
            );
        }
        let caller = self.stream_caller(&stream, token);
        let mut reader = stream;
        if write_event(output, &NodeEvent::StreamOpen { stream: id, caller }).is_ok() {
            let mut buffer = vec![0_u8; MAX_CHUNK];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        let data =
                            base64::engine::general_purpose::STANDARD.encode(&buffer[..read]);
                        if write_event(output, &NodeEvent::StreamData { stream: id, data }).is_err()
                        {
                            break;
                        }
                    }
                }
            }
        }
        self.end_stream(id);
        let _ = write_event(output, &NodeEvent::StreamClosed { stream: id });
    }

    /// What the node proves about the process that opened a stream: the
    /// holder of a credential issued here for a pane, still
    /// descended from that pane's shell as its Herdr answers now, the same
    /// checks a bootstrap makes. `None` when the peer holds no such
    /// credential or the pane no longer answers for it; the core then asks
    /// `Call::PaneInspect` for the stream's command, as it always did.
    fn stream_caller(&self, stream: &LocalStream, token: Option<&str>) -> Option<StreamCaller> {
        let started = self.started.get()?;
        let peer = i32::try_from(stream.peer_pid().ok()?).ok()?;
        let holder = (peer, pane_peer::process_start(peer)?);
        let pane_id = lock(&self.issued)
            .values()
            .find(|reference| match token {
                // The token only selects a reference this node issued. It
                // never replaces the kernel peer and current pane proof.
                Some(token) => {
                    reference.token == token
                        && reference.holder.is_none_or(|issued| issued == holder)
                }
                // Existing clients send no token; their one-shot holder
                // remains enough to select their own reference.
                None => reference.holder == Some(holder),
            })?
            .pane_id
            .clone()?;
        let identity = pane_peer::inspect(&started.herdr_socket, &pane_id).ok()?;
        pane_peer::descends_from(peer, identity.shell_pid)
            .then_some(StreamCaller { pane_id, identity })
    }

    fn sweep_references(&self, output: &Mutex<impl Write>) -> io::Result<()> {
        let mut revoked = Vec::new();
        {
            let mut issued = lock(&self.issued);
            let stale = issued
                .iter()
                .filter_map(|(path, reference)| {
                    let present = path.is_file();
                    let expired = reference.created.elapsed() >= REFERENCE_LIFETIME
                        || (!path.with_extension("claimed").is_file()
                            && reference.created.elapsed() >= UNCLAIMED_LIFETIME);
                    let holder_gone = reference.holder.is_some_and(|(pid, started)| {
                        pane_peer::process_start(pid) != Some(started)
                    });
                    let pane_gone = reference.shell.is_some_and(|(pid, started)| {
                        pane_peer::process_start(pid) != Some(started)
                    });
                    (!present || expired || holder_gone || pane_gone).then_some(path.clone())
                })
                .collect::<Vec<_>>();
            for path in stale {
                remove_if_present(&path)?;
                remove_if_present(&path.with_extension("claimed"))?;
                if let Some(reference) = issued.remove(&path) {
                    revoked.push(reference.token);
                }
            }
        }
        for token in revoked {
            write_event(output, &NodeEvent::Revoke { token })?;
        }
        Ok(())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The folder a node's bootstrap folders go in: this account's own, never a
/// link, and private, as is every folder above it the node makes: it sits in
/// the state folder, which a core moved here keeps its letters in.
fn validate_dir(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bridge directory is not absolute",
        ));
    }
    private::own_dir(path).map(drop)
}

/// Removes the folders of pane services that ended without removing their
/// own: a node that was killed, or the workspace bridge an older Hide ran.
/// A folder is dead when its socket no longer answers; one younger than a
/// minute may belong to a service still binding, so it is left.
fn remove_dead_folders(bridges: &Path) {
    let Ok(entries) = fs::read_dir(bridges) else {
        return;
    };
    for entry in entries.flatten() {
        let folder = entry.path();
        let young = fs::symlink_metadata(&folder)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_none_or(|age| age < DEAD_FOLDER_AGE);
        if !entry.file_name().to_string_lossy().starts_with("bridge-")
            || young
            || !entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            continue;
        }
        let socket = folder.join("bootstrap.sock");
        if fs::symlink_metadata(&socket).is_err() || LocalStream::connect(&socket).is_err() {
            let _ = fs::remove_dir_all(&folder);
        }
    }
}

/// The wire spelling of `peer`'s canonical working directory, as the kernel
/// reports it.
fn caller_directory(peer: i32) -> Option<String> {
    let cwd = pane_peer::process_cwd(peer)?;
    let canonical = hide_platform::fs::identity::canonical(&cwd).ok()?;
    hide_platform::path::to_wire(&canonical).ok()
}

/// One event line on the link's output, under the same lock as the answers,
/// so a line is never cut by another.
pub(crate) fn write_event(output: &Mutex<impl Write>, event: &NodeEvent) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(event).map_err(io::Error::other)?;
    bytes.push(b'\n');
    let mut output = output
        .lock()
        .map_err(|_| io::Error::other("node output lock poisoned"))?;
    output.write_all(&bytes)?;
    output.flush()
}

/// Tells the core this node turned a caller away, so the refusal is recorded
/// with the node it happened on.
fn report_refusal(output: &Mutex<impl Write>, pane_id: Option<String>, reason: &str) {
    let _ = write_event(
        output,
        &NodeEvent::Refused {
            pane_id,
            reason: reason.to_owned(),
        },
    );
}

/// A reference with no wire spelling is not answered at all, so its caller
/// reads the end of the stream and the issuer withdraws the reference.
fn answer_client(
    stream: &mut LocalStream,
    result: Result<BootstrapAnswer, String>,
) -> io::Result<()> {
    let answer = match result {
        Ok(BootstrapAnswer::Reference { path, info }) => {
            let reference = hide_platform::path::to_wire(&path).map_err(io::Error::other)?;
            let mut answer = info.unwrap_or_else(|| json!({"ok":true}));
            answer["reference"] = json!(reference);
            if answer.to_string().len() + 1 > hide_node_link::panes::HOOK_RESPONSE_LIMIT {
                return Err(io::Error::other("hook response too large"));
            }
            answer
        }
        Ok(BootstrapAnswer::Hook(answer)) => answer,
        Err(reason) => {
            json!({"ok":false,"reason":reason,"next_action":hide_node_link::panes::refusal_next_action(&reason)})
        }
    };
    writeln!(stream, "{answer}")
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

// A folder's age is set through its handle, which only Unix opens.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// On a machine Hide has not written to yet, the bridge folder leaves no
    /// folder others can read: a core moved here later refuses a state
    /// folder that is not private, and reads no letters.
    #[test]
    fn the_bridge_folder_makes_every_folder_above_it_private() {
        let home = tempfile::tempdir().unwrap();
        let state = hide_kit::layout::default_state_dir(home.path());
        let bridges = hide_kit::layout::workspace_bridges(&state);
        validate_dir(&bridges).unwrap();
        for made in [hide_kit::layout::hide_home(home.path()), state, bridges] {
            assert!(private::is_private(&made).unwrap(), "{}", made.display());
        }
    }

    #[test]
    fn every_device_bootstrap_refusal_includes_recovery_advice() {
        use std::io::BufRead;
        for (reason, advice) in [
            ("pane_changed", "Reconnect the pane and retry"),
            ("remote_unavailable", "Reconnect the pane and retry"),
            ("credential_expired", "Reconnect the pane and retry"),
            ("invalid_request", "Reconnect the pane and retry"),
            (
                "checkout_not_registered",
                "Run the command from a shell inside a registered project checkout, or reconnect Hide, and retry",
            ),
        ] {
            let (mut writer, reader) = LocalStream::pair().unwrap();
            answer_client(&mut writer, Err(reason.to_owned())).unwrap();
            let mut line = String::new();
            std::io::BufReader::new(reader)
                .read_line(&mut line)
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&line).unwrap(),
                json!({"ok":false,"reason":reason,"next_action":advice})
            );
        }
    }

    /// A link's output as the core reads it: one event per line, handed to
    /// the test as it is written.
    struct Events(mpsc::Sender<Value>);

    impl Write for Events {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            for line in bytes
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
            {
                let _ = self
                    .0
                    .send(serde_json::from_slice(line).map_err(io::Error::other)?);
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A started pane service in a short `/tmp` folder (a socket path has a
    /// length bound), its events, and the socket a plain shell connects to.
    fn with_service(body: impl FnOnce(&Panes, &mpsc::Receiver<Value>, &Path)) {
        let root = tempfile::Builder::new()
            .prefix("hp")
            .tempdir_in("/tmp")
            .unwrap();
        let bridges = root.path().join("b");
        let (sender, events) = mpsc::channel();
        let output = Mutex::new(Events(sender));
        let panes = Panes::new();
        std::thread::scope(|scope| {
            let started = panes
                .start(scope, &output, &bridges, "/tmp/hide-test-herdr.sock")
                .unwrap();
            let socket = hide_platform::path::from_wire(&started.socket).unwrap();
            body(&panes, &events, &socket);
            panes.stop();
        });
        panes.remove_folder();
    }

    /// What a plain shell's `hide <path>` reads back for `request`.
    fn ask(socket: &Path, request: Value) -> std::thread::JoinHandle<Value> {
        let socket = socket.to_path_buf();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let mut stream = LocalStream::connect(&socket).unwrap();
            writeln!(stream, "{request}").unwrap();
            let mut line = String::new();
            std::io::BufReader::new(stream)
                .read_line(&mut line)
                .unwrap();
            serde_json::from_str(&line).unwrap()
        })
    }

    fn open(path: &str) -> Value {
        json!({"project_open": {"path": path, "request_id": "1-open"}})
    }

    /// The next event of `kind`, past the refusals and pings beside it.
    fn next_event(events: &mpsc::Receiver<Value>, kind: &str) -> Value {
        loop {
            let event = events.recv_timeout(Duration::from_secs(10)).unwrap();
            if event["event"] == kind {
                return event;
            }
        }
    }

    /// A shell of the node's own account, in no pane, gets exactly the
    /// core's answer to one open, and the node vouches for nothing else.
    #[test]
    fn a_plain_shell_open_goes_up_once_and_reads_the_core_s_answer() {
        with_service(|panes, events, socket| {
            let caller = ask(socket, open("/home/me/app"));
            let event = next_event(events, "project_open");
            assert_eq!(event["path"], "/home/me/app");
            assert_eq!(event["request_id"], "1-open");
            let request = event["request"].as_u64().unwrap();
            panes
                .answer_proof(request, ProofAnswer::ProjectOpenAccepted)
                .unwrap();
            let answer =
                json!({"type": "workspace_result", "ok": true, "result": {"label": "app"}});
            panes
                .answer_proof(
                    request,
                    ProofAnswer::ProjectOpen {
                        answer: answer.clone(),
                    },
                )
                .unwrap();
            assert_eq!(caller.join().unwrap(), answer);
            assert!(
                lock(&panes.issued).is_empty(),
                "no credential is issued for an open"
            );
        });
    }

    /// A link that ended without the node noticing never takes the open:
    /// the caller hears to connect the device within the acceptance bound.
    #[test]
    fn an_open_no_core_takes_answers_connect_first_within_its_bound() {
        with_service(|_, events, socket| {
            let asked = Instant::now();
            let caller = ask(socket, open("/home/me/app"));
            next_event(events, "project_open");
            let answer = caller.join().unwrap();
            assert_eq!(answer["ok"], false);
            assert_eq!(answer["reason"], "bridge_unavailable");
            assert_eq!(
                answer["next_action"],
                "Connect this device in Hide first, then run it again"
            );
            assert!(asked.elapsed() < PROJECT_OPEN_ACCEPT + Duration::from_secs(3));
        });
    }

    /// A relative or oversized path, or an unknown field, is refused by the
    /// node before anything goes up.
    #[test]
    fn a_malformed_open_never_reaches_the_core() {
        with_service(|_, events, socket| {
            for request in [
                open("app"),
                open(&format!("/{}", "a".repeat(4096))),
                json!({"project_open": {"path": "/home/me/app", "request_id": "1", "pane_id": "p"}}),
            ] {
                let answer = ask(socket, request).join().unwrap();
                assert_eq!(answer["reason"], "invalid_request", "{answer}");
            }
            let refusal = next_event(events, "refused");
            assert_eq!(refusal["reason"], "invalid_request");
            assert!(
                events
                    .try_iter()
                    .all(|event| event["event"] != "project_open"),
                "nothing went up"
            );
        });
    }

    /// One open runs at a time, and six start in a minute at most.
    #[test]
    fn opens_run_one_at_a_time_and_six_a_minute() {
        let panes = Panes::new();
        let running = panes.claim_open().unwrap();
        assert_eq!(panes.claim_open().err().unwrap(), "project_open_busy");
        drop(running);
        for _ in 1..PROJECT_OPENS_PER_MINUTE {
            drop(panes.claim_open().unwrap());
        }
        assert_eq!(
            panes.claim_open().err().unwrap(),
            "project_open_rate_limited"
        );
        // A minute later the oldest has left the window.
        if let Some(started) = lock(&panes.opens).started.front_mut() {
            *started -= Duration::from_secs(61);
        }
        drop(panes.claim_open().unwrap());
    }

    /// A process of another account is refused by the kernel's own answer
    /// about the stream's other end, never by a field it sent. The bridge
    /// folder is private, so the test opens its own folder and socket to
    /// that account first; `nobody` connects with the system's Python.
    #[test]
    #[ignore = "needs passwordless sudo to run a process as another account; the remote mailbox lane runs it"]
    fn another_account_s_open_is_refused_by_the_kernel_s_peer_check() {
        use std::os::unix::fs::PermissionsExt;
        with_service(|_, events, socket| {
            for (path, mode) in [
                (
                    socket.parent().unwrap().parent().unwrap().parent().unwrap(),
                    0o711,
                ),
                (socket.parent().unwrap().parent().unwrap(), 0o711),
                (socket.parent().unwrap(), 0o711),
                (socket, 0o777),
            ] {
                fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
            }
            let script = "import socket, sys\n\
                s = socket.socket(socket.AF_UNIX)\n\
                s.connect(sys.argv[1])\n\
                s.sendall(sys.argv[2].encode() + b'\\n')\n\
                sys.stdout.write(s.makefile().readline())\n";
            let output = std::process::Command::new("sudo")
                .args(["-n", "-u", "nobody", "python3", "-c", script])
                .arg(socket)
                .arg(open("/home/me/app").to_string())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(answer["reason"], "caller_not_this_account", "{answer}");
            assert_eq!(
                next_event(events, "refused")["reason"],
                "caller_not_this_account"
            );
            assert!(
                events
                    .try_iter()
                    .all(|event| event["event"] != "project_open")
            );
        });
    }

    /// A starting service removes what a killed node or an older Hide's
    /// bridge left, and never a folder whose socket answers or one another
    /// service may still be binding.
    #[test]
    fn a_start_removes_only_dead_folders() {
        let root = tempfile::Builder::new()
            .prefix("hp")
            .tempdir_in("/tmp")
            .unwrap();
        let bridges = root.path();
        let old = std::time::SystemTime::now() - 2 * DEAD_FOLDER_AGE;
        let folder = |name: &str| {
            let path = bridges.join(name);
            fs::create_dir(&path).unwrap();
            path
        };
        let age = |path: &Path| fs::File::open(path).unwrap().set_modified(old).unwrap();
        let dead = folder("bridge-dead");
        fs::write(dead.join("bootstrap.sock"), b"").unwrap();
        let empty = folder("bridge-empty");
        let live = folder("bridge-live");
        let _listener = LocalListener::bind(&live.join("bootstrap.sock")).unwrap();
        let young = folder("bridge-young");
        let other = folder("other");
        for path in [&dead, &empty, &live, &other] {
            age(path);
        }
        remove_dead_folders(bridges);
        assert!(!dead.exists());
        assert!(!empty.exists());
        assert!(live.exists());
        assert!(young.exists());
        assert!(other.exists());
    }
}
