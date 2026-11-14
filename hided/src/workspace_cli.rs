//! Pane-scoped Workspace CLI transport. Bootstrap attests the caller over a
//! local socket; commands and results use hided's authenticated `/ws`.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use hide_core::workspace_control::Action;
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
use hide_agent_hooks::hook_request::install::{self, AccessAsk, Ask, PaneRequest};
use hide_agent_hooks::hook_request::node_socket;

const TIMEOUT: Duration = Duration::from_secs(10);
/// Longer than the daemon's own Factory answer limit, so the daemon's reason
/// arrives rather than a local timeout.
const REMOTE_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(25);
/// The catalog's own wait for a model-list read is 10 seconds; the exchange
/// outlives it.
const CATALOG_TIMEOUT: Duration = Duration::from_secs(20);
/// A profile change reads the list, then installs or takes out Hide's
/// pieces in its account (`hide_core::agent_profiles`, its node's wait).
const PROFILE_TIMEOUT: Duration = Duration::from_secs(150);

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
            // Outside Hide's panes, a connected external tool runs as its
            // connection (B18); any other caller as before.
            Err(_) => match crate::access_cli::tool_key(env)? {
                Some(key) => Self::tool(env, key),
                None => Self::one_shot(env),
            },
        }
    }

    /// A one-shot credential for the external tool whose key is `key`.
    fn tool(env: &'a Env, key: String) -> Result<Self, String> {
        let path = bootstrap_tool(&crate::access_cli::state_dir(env), key)?;
        Ok(Self {
            env,
            path: path.clone(),
            named: false,
            _one_shot: Some(OneShotReference(path)),
        })
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
    // The pane's installation goes with it, so a daemon of another one
    // refuses it rather than bind it by its folder (`pane_auth`).
    let request = PaneRequest {
        pane_id: pane_id.to_owned(),
        nonce: hex::encode(nonce),
        one_shot,
        hook: None,
        install: env.install.clone(),
        access: None,
    };
    let request = serde_json::to_value(&request).map_err(|_| "reference_unavailable".to_owned())?;
    let install = env.install.as_deref();
    in_pane_order(
        install::order(install),
        || bootstrap_local(env, &request, install),
        || bootstrap_remote(env, &request, install),
    )
}

/// Asks this account's daemon (`local`) and the device bridges (`remote`) for
/// the pane in the order its installation calls for ([`install::order`]).
/// A pane that asks the daemon first, and hears both refuse, reports the
/// bridge's reason: a daemon in the node role opens no bootstrap socket, so
/// its node is the one that can say why.
fn in_pane_order(
    order: [Ask; 2],
    local: impl FnOnce() -> Result<PathBuf, String>,
    remote: impl FnOnce() -> Result<Option<PathBuf>, String>,
) -> Result<PathBuf, String> {
    if order[0] == Ask::Bridges {
        return match remote() {
            Ok(Some(path)) => Ok(path),
            Ok(None) => local(),
            Err(remote_reason) => local().map_err(|_| remote_reason),
        };
    }
    match local() {
        Ok(path) => Ok(path),
        Err(local_reason) => match remote() {
            Ok(Some(path)) => Ok(path),
            Ok(None) => Err(local_reason),
            Err(remote_reason) => Err(remote_reason),
        },
    }
}

/// An external tool's one-shot credential, proved by its key on the
/// bootstrap socket of the daemon in `state_dir` (PRD hide-external-control
/// B18): never a device bridge, since a tool runs on the core's machine.
fn bootstrap_tool(state_dir: &Path, key: String) -> Result<PathBuf, String> {
    let request = access_request(AccessAsk::Use { key }, true)?;
    let mut stream = connect_bootstrap(state_dir)?;
    writeln!(stream, "{request}").map_err(|_| "hide_unavailable".to_owned())?;
    read_bootstrap_answer(&mut stream, None)
}

/// `hide access request|status|leave`: one ask on the bootstrap socket of the
/// daemon in `state_dir`, answered with the core's `access` value, or a
/// refusal's code and the daemon's next action when it gave one.
pub fn access_bootstrap(
    state_dir: &Path,
    ask: AccessAsk,
) -> Result<Value, (String, Option<String>)> {
    let unavailable = |code: String| (code, None);
    let request = access_request(ask, false).map_err(unavailable)?;
    let mut stream = connect_bootstrap(state_dir).map_err(unavailable)?;
    writeln!(stream, "{request}").map_err(|_| unavailable("hide_unavailable".to_owned()))?;
    let mut line = String::new();
    BufReader::new(&mut stream)
        .take(4096)
        .read_line(&mut line)
        .map_err(|_| unavailable("hide_unavailable".to_owned()))?;
    let answer: Value =
        serde_json::from_str(&line).map_err(|_| unavailable("hide_unavailable".to_owned()))?;
    if answer["ok"] == true {
        return Ok(answer["access"].clone());
    }
    Err((
        answer["reason"]
            .as_str()
            .unwrap_or("bootstrap_refused")
            .to_owned(),
        answer["next_action"].as_str().map(str::to_owned),
    ))
}

fn access_request(ask: AccessAsk, one_shot: bool) -> Result<Value, String> {
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(|_| "reference_unavailable".to_owned())?;
    serde_json::to_value(PaneRequest {
        pane_id: String::new(),
        nonce: hex::encode(nonce),
        one_shot,
        hook: None,
        install: None,
        access: Some(ask),
    })
    .map_err(|_| "reference_unavailable".to_owned())
}

fn connect_bootstrap(state_dir: &Path) -> Result<LocalStream, String> {
    let socket = crate::pane_auth::bootstrap_socket_path(state_dir)?;
    LocalStream::connect_until(&socket, Instant::now() + TIMEOUT)
        .map_err(|_| "hide_unavailable".to_owned())
}

fn bootstrap_local(env: &Env, request: &Value, install: Option<&str>) -> Result<PathBuf, String> {
    let socket = crate::pane_auth::bootstrap_socket_path(&env.state_dir)?;
    let mut stream = LocalStream::connect_until(&socket, Instant::now() + TIMEOUT)
        .map_err(|_| "hide_unavailable".to_owned())?;
    writeln!(stream, "{request}").map_err(|_| "hide_unavailable".to_owned())?;
    read_bootstrap_answer(&mut stream, install)
}

/// Reads one bootstrap answer. For a request that named an installation
/// (`install`), a reference counts only when the answer names that same
/// installation ([`install::confirm`]).
fn read_bootstrap_answer(stream: &mut impl Read, install: Option<&str>) -> Result<PathBuf, String> {
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
    install::confirm(&answer, install).map_err(str::to_owned)?;
    answer["reference"]
        .as_str()
        .and_then(|wire| hide_platform::path::from_wire(wire).ok())
        .ok_or_else(|| "reference_unavailable".to_owned())
}

/// Asks each node pane service on this device (`hide_host::panes`), one per
/// daemon that has this device open, for this pane's reference.
fn bootstrap_remote(
    env: &Env,
    request: &Value,
    install: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    bootstrap_bridges(
        &hide_kit::layout::workspace_bridges(&env.state_dir),
        request,
        install,
    )
}

/// [`hide_agent_hooks::hook_request::live_bridges`] that may answer for a
/// pane naming `install`, its reason owned.
fn live_bridges(
    bridge_dir: &Path,
    deadline: Instant,
    install: Option<&str>,
) -> Result<Vec<LocalStream>, String> {
    hide_agent_hooks::hook_request::live_bridges(bridge_dir, deadline, install)
        .map(|bridges| bridges.streams)
        .map_err(str::to_owned)
}

/// Asks each bridge that may answer for a pane naming `install` in turn
/// ([`install::settle_bridges`]): one whose answer does not confirm
/// `install` is not this pane's node, so it neither succeeds nor makes the
/// pane ambiguous.
fn bootstrap_bridges(
    bridge_dir: &Path,
    request: &Value,
    install: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let answers = live_bridges(
        bridge_dir,
        Instant::now() + REMOTE_BOOTSTRAP_TIMEOUT,
        install,
    )?
    .into_iter()
    .map(|mut stream| {
        // Each bridge has the whole time: one answers after the one before.
        stream.set_deadline(Instant::now() + REMOTE_BOOTSTRAP_TIMEOUT);
        writeln!(stream, "{request}").map_err(|_| "bridge_unavailable".to_owned())?;
        match read_bootstrap_answer(&mut stream, install)? {
            path if path.starts_with(bridge_dir) => Ok(path),
            _ => Err("invalid_reference".to_owned()),
        }
    });
    install::settle_bridges(answers)
}

/// [`hide_agent_hooks::hook_request::read_reference`], its reason owned.
fn read_reference(path: &Path) -> Result<Reference, String> {
    hide_agent_hooks::hook_request::read_reference(path).map_err(str::to_owned)
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
    query: &hide_core::links::query::LinksQuery,
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

/// `hide session`: a read of the Project's sessions, answered without a
/// shell. A `read` may wait for a device's node for the minute D-14 gives
/// it, and the exchange waits a little past that for the refusal.
pub fn request_session(
    credential: &mut Credential,
    command: &hide_core::links::session_command::SessionCommand,
) -> Result<Value, String> {
    let timeout = match command {
        hide_core::links::session_command::SessionCommand::Read { .. } => {
            hide_core::links::session_command::READ_TIMEOUT + Duration::from_secs(15)
        }
        _ => Duration::from_secs(30),
    };
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange_within(
            path,
            &reference,
            json!({"type":"session","request_id":request_id,"command":command}),
            &request_id,
            false,
            timeout,
            after,
        )
    })
}

/// `hide memory`: answered without a shell. An `add` asks the provider, so
/// it has the minute B8 gives it; it writes nothing after its own deadline,
/// and running it again adds nothing twice.
pub fn request_memory(
    credential: &mut Credential,
    command: &hide_core::memory_command::MemoryCommand,
) -> Result<Value, String> {
    let (action, timeout) = match command {
        hide_core::memory_command::MemoryCommand::Add { .. } => {
            (true, hide_core::memory_command::ADD_TIMEOUT)
        }
        hide_core::memory_command::MemoryCommand::Show { .. }
        | hide_core::memory_command::MemoryCommand::Forget { .. }
        | hide_core::memory_command::MemoryCommand::Undo { .. }
        | hide_core::memory_command::MemoryCommand::Placed { .. }
        | hide_core::memory_command::MemoryCommand::Building { .. }
        | hide_core::memory_command::MemoryCommand::Unbuilt { .. }
        | hide_core::memory_command::MemoryCommand::Outdated { .. }
        | hide_core::memory_command::MemoryCommand::Skipped { .. }
        | hide_core::memory_command::MemoryCommand::Stale { .. }
        | hide_core::memory_command::MemoryCommand::Release { .. }
        | hide_core::memory_command::MemoryCommand::Recheck { .. } => (true, TIMEOUT),
        hide_core::memory_command::MemoryCommand::Search { .. } => (false, TIMEOUT),
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

/// `hide mission`: a Conductor's command over its mission, answered by
/// the core without a shell. Running it again converges: a plan or node
/// change repeated changes nothing more.
pub fn request_mission(
    credential: &mut Credential,
    command: &hide_core::missions::MissionCommand,
) -> Result<Value, String> {
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange(
            path,
            &reference,
            json!({"type":"mission","request_id":request_id,"command":command}),
            &request_id,
            true,
            after,
        )
    })
}

/// `hide task`: a command over the caller's session's own plan, answered by
/// the core without a shell. Running it again converges: a repeated change
/// changes nothing more.
pub fn request_plan(
    credential: &mut Credential,
    command: &hide_core::plans::PlanCommand,
) -> Result<Value, String> {
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange(
            path,
            &reference,
            json!({"type":"plan","request_id":request_id,"command":command}),
            &request_id,
            true,
            after,
        )
    })
}

/// `hide agent catalog`: read from what the core holds, after the daemon's
/// wait of up to 10 seconds for one read of the model lists, so the
/// exchange is given longer than an ordinary read.
pub fn request_agent_catalog(
    credential: &mut Credential,
    machine: Option<&str>,
) -> Result<Value, String> {
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange_within(
            path,
            &reference,
            json!({"type":"agent_catalog","request_id":request_id,"machine":machine}),
            &request_id,
            false,
            CATALOG_TIMEOUT,
            after,
        )
    })
}

/// `hide agent profile`: a change of a machine's agent profiles, which
/// installs or takes out Hide's pieces in the account it touches. Never sent
/// again on its own: a lost answer is read back with `list`.
pub fn request_agent_profile(
    credential: &mut Credential,
    command: &hide_core::agent_profiles::ProfileCommand,
) -> Result<Value, String> {
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange_within(
            path,
            &reference,
            json!({"type":"agent_profile","request_id":request_id,"command":command}),
            &request_id,
            true,
            PROFILE_TIMEOUT,
            after,
        )
    })
}

/// `hide device`: the core's own device registrations. A `list` is a read;
/// an `add` or `remove` run again after a lost answer is refused as a
/// duplicate or an unknown device, never applied twice.
pub fn request_device(
    credential: &mut Credential,
    command: &hide_core::device_command::DeviceCommand,
) -> Result<Value, String> {
    let action = !matches!(command, hide_core::device_command::DeviceCommand::List);
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange(
            path,
            &reference,
            json!({"type":"device","request_id":request_id,"command":command}),
            &request_id,
            action,
            after,
        )
    })
}

/// How long `hide <path>` waits for its answer: past the daemon's own wait
/// for the registration, so the daemon's reason arrives rather than a local
/// timeout.
const PROJECT_OPEN_TIMEOUT: Duration = Duration::from_secs(40);

/// `hide <path>` from a pane, a device's included: the folder is on the
/// credential's machine. Running it again only brings the project forward.
pub fn request_project_open(credential: &mut Credential, path: &str) -> Result<Value, String> {
    credential.run(|reference_path, after| {
        let reference = read_reference(reference_path)?;
        let request_id = fresh_request_id()?;
        run_exchange_within(
            reference_path,
            &reference,
            json!({"type":"project_open","request_id":request_id,"path":path}),
            &request_id,
            true,
            PROJECT_OPEN_TIMEOUT,
            after,
        )
    })
}

/// `hide <path>` from a shell in no pane on a device: asked of the node
/// pane service of this device's link, which proves the caller only as a
/// process of its own account and sends the open up with no credential
/// (`hide_host::panes`). No live service means the device is not connected;
/// several mean several cores have it open, and which one is meant is not
/// guessed.
/// No node of this account answers: this machine is no connected device.
pub const NOT_LINKED: &str = "device_not_linked";

pub fn request_project_open_on_bridge(env: &Env, path: &str) -> Result<Value, String> {
    project_open_on_bridge(&hide_kit::layout::workspace_bridges(&env.state_dir), path)
}

fn project_open_on_bridge(bridge_dir: &Path, path: &str) -> Result<Value, String> {
    // A plain shell names no installation, so every live node may be meant.
    let mut bridges = live_bridges(bridge_dir, Instant::now() + TIMEOUT, None)?;
    let mut stream = match bridges.len() {
        0 => return Err(NOT_LINKED.to_owned()),
        1 => bridges.remove(0),
        _ => return Err("bridge_ambiguous".to_owned()),
    };
    // The node answers within its own two bounds: the core's acceptance,
    // then the open's outcome.
    stream.set_deadline(
        Instant::now()
            + hide_node_link::panes::PROJECT_OPEN_ACCEPT
            + hide_node_link::panes::PROJECT_OPEN_ANSWER
            + TIMEOUT,
    );
    let request_id = fresh_request_id()?;
    let request = json!({"project_open": {"path": path, "request_id": request_id}});
    writeln!(stream, "{request}").map_err(|_| "bridge_unavailable".to_owned())?;
    let mut line = String::new();
    BufReader::new(stream)
        .take(hide_node_link::panes::HOOK_RESPONSE_LIMIT as u64)
        .read_line(&mut line)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                "request_timeout".to_owned()
            } else {
                "bridge_unavailable".to_owned()
            }
        })?;
    serde_json::from_str(&line).map_err(|_| "bridge_unavailable".to_owned())
}

/// `hide <path>` on the core's own machine, with the daemon's token from the
/// state file this account owns: one `/ws` connection whose handshake names
/// `client_kind: "cli"` carries one request and its answer, and is never a
/// screen (`hided/src/project_open.rs`).
pub fn request_project_open_with_token(
    port: u16,
    token: &str,
    path: &str,
) -> Result<Value, String> {
    let request_id = fresh_request_id()?;
    request_with_token(
        port,
        token,
        json!({"type":"project_open","request_id":request_id,"path":path}),
        &request_id,
        PROJECT_OPEN_TIMEOUT,
    )
}

/// `hide import herdr` on the core's own machine, with the daemon's token,
/// as `hide <path>` asks (PRD engine-bundle-4-herdr-removal B17): what this
/// machine's Herdr save offers, and with `yes` the move itself.
pub fn request_herdr_import_with_token(port: u16, token: &str, yes: bool) -> Result<Value, String> {
    let request_id = fresh_request_id()?;
    request_with_token(
        port,
        token,
        json!({"type":"herdr_import","request_id":request_id,"yes":yes}),
        &request_id,
        TIMEOUT,
    )
}

/// One `request` named `request_id` and its answer over one `/ws`
/// connection whose handshake names `client_kind: "cli"`, with the daemon's
/// token from the state file this account owns.
fn request_with_token(
    port: u16,
    token: &str,
    request: Value,
    request_id: &str,
    within: Duration,
) -> Result<Value, String> {
    let reference = Reference {
        token: token.to_owned(),
        route: Route::Daemon {
            port,
            origin_port: port,
        },
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "request_unavailable".to_owned())?;
    runtime.block_on(async {
        tokio::time::timeout(within, async {
            let mut socket = connect(&reference).await.map_err(reported)?;
            socket
                .send(Message::Text(
                    json!({"token":token,"schema_version":SCHEMA_VERSION,"client_kind":"cli"})
                        .to_string()
                        .into(),
                ))
                .await
                .map_err(|_| "hide_unavailable".to_owned())?;
            socket
                .send(Message::Text(request.to_string().into()))
                .await
                .map_err(|_| "hide_unavailable".to_owned())?;
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    let value: Value =
                        serde_json::from_str(&text).map_err(|_| "invalid_response".to_owned())?;
                    if value["type"] != "workspace_result" || value["request_id"] != request_id {
                        return Err("invalid_response".to_owned());
                    }
                    Ok(value)
                }
                // The state file named a daemon that holds another token.
                Some(Ok(Message::Close(_))) => Err("credential_rejected".to_owned()),
                _ => Err("hide_unavailable".to_owned()),
            }
        })
        .await
        .map_err(|_| "request_timeout".to_owned())?
    })
}

/// `hide access list`: read by a connected tool from what the core holds.
pub fn request_access_list(credential: &mut Credential) -> Result<Value, String> {
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange(
            path,
            &reference,
            json!({"type":"access_list","request_id":request_id}),
            &request_id,
            false,
            after,
        )
    })
}

/// `hide ps`: read from what the core holds.
pub fn request_ps(credential: &mut Credential, all: bool) -> Result<Value, String> {
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange(
            path,
            &reference,
            json!({"type":"ps","request_id":request_id,"all":all}),
            &request_id,
            false,
            after,
        )
    })
}

/// `hide pane`, `hide agent read` and `hide workspace purpose`: the core
/// admits the caller's pane and runs the command through its engine, with
/// no shell needed.
pub fn request_pane(
    credential: &mut Credential,
    command: &hide_core::pane_command::PaneCommand,
) -> Result<Value, String> {
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange(
            path,
            &reference,
            json!({"type":"pane","request_id":request_id,"command":command}),
            &request_id,
            false,
            after,
        )
    })
}

/// `hide workspace remove`: the core removes the worktree as the delete
/// dialog does and answers once the removal has ended, which can take as
/// long as the host's recheck and removal; the daemon's own wait ends first.
pub fn request_worktree_remove(
    credential: &mut Credential,
    command: &hide_core::worktree_command::WorktreeRemove,
) -> Result<Value, String> {
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange_within(
            path,
            &reference,
            json!({"type":"worktree_remove","request_id":request_id,"command":command}),
            &request_id,
            true,
            hide_core::worktree_command::REMOVE_WAIT + Duration::from_secs(20),
            after,
        )
    })
}

/// `hide agent sleep` (`agent_sleep`) and `hide agent archive`
/// (`agent_archive`): the core's own action on a registered agent.
pub fn request_agent_action(
    credential: &mut Credential,
    action: &str,
    id: &str,
    actor: Option<&str>,
) -> Result<Value, String> {
    credential.run(|path, after| {
        let reference = read_reference(path)?;
        let request_id = fresh_request_id()?;
        run_exchange(
            path,
            &reference,
            json!({"type":action,"request_id":request_id,"id":id,"actor":actor}),
            &request_id,
            false,
            after,
        )
    })
}

/// Losing the final capability claim cannot turn durable success into retry.
pub fn request_delivery(
    credential: &mut Credential,
    command: hide_core::delivery::Command,
    hint: Option<&str>,
) -> Result<Value, String> {
    // A spawn creates a worktree, starts an agent and hands it its brief,
    // which takes longer than a local command, and one on another device
    // makes several calls over SSH besides; running it again after a timeout
    // returns the same agent either way.
    // The spawn answers within its own answer wait, which starts after the
    // wait for a kit pass on the child's machine and ends its new shell's
    // wait too, so the CLI waits both and an ordinary command's wait for the
    // answer to come back.
    let timeout = match &command {
        hide_core::delivery::Command::Agents {
            command: hide_core::coordination::Command::Spawn { machine, .. },
        } => {
            TIMEOUT
                + if machine.is_some() {
                    hide_core::coordination::DEVICE_SPAWN_ANSWER_WAIT
                } else {
                    hide_core::coordination::SPAWN_ANSWER_WAIT
                }
                + hide_core::coordination::HOOK_PASS_WAIT
        }
        // An end closes its agent's pane after its record ended.
        hide_core::delivery::Command::Agents {
            command: hide_core::coordination::Command::End { .. },
        } => TIMEOUT + hide_core::coordination::END_ANSWER_WAIT,
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

/// How long `hide workspace memory` waits for the core's answer, inside the
/// 1,250 ms a hook gives Memory (`hide_agent_hooks::memory::MEMORY_BUDGET`).
const MEMORY_WITHIN: Duration = Duration::from_millis(1_200);
/// The prompt a hook piped in: at most
/// `hide_core::memory_hook::PROMPT_LIMIT_BYTES`, its valid UTF-8 start.
pub fn read_memory_prompt(input: impl std::io::Read) -> Result<String, String> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    input
        .take(hide_core::memory_hook::PROMPT_LIMIT_BYTES as u64)
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

/// A hook's read through the bootstrap service
/// (`hide_agent_hooks::hook_request::send`, which the hook itself also
/// speaks), with the request's deadline. A step that failed is written to the
/// diagnostic log as `hook.request_failed` with the system's error kind.
pub fn request_hook(
    env: &Env,
    command: hide_node_link::panes::HookCommand,
) -> Result<Value, String> {
    let within = if matches!(command, hide_node_link::panes::HookCommand::Memory { .. }) {
        MEMORY_WITHIN
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

/// How long `hide agent exec` waits for its node to answer a report: the
/// agent waits on it, and a `Ready` that is not answered starts nothing.
const AGENT_EXEC_WITHIN: Duration = Duration::from_secs(10);

/// `hide agent exec`'s report of the profile's start `attempt` the pane's
/// shell ran (`HookCommand::AgentExec`), within [`AGENT_EXEC_WITHIN`].
pub fn report_agent_exec(
    env: &Env,
    attempt: &str,
    event: hide_node_link::panes::AgentExecEvent,
) -> Result<Value, String> {
    hook_request(
        env,
        std::env::var(crate::env::HIDE_CAP_REF).ok(),
        hide_node_link::panes::HookCommand::AgentExec {
            attempt: attempt.to_owned(),
            event,
        },
        Instant::now() + AGENT_EXEC_WITHIN,
    )
}

/// [`request_hook`] with the reference `HIDE_CAP_REF` names, if any, and the
/// request's deadline.
fn hook_request(
    env: &Env,
    named: Option<String>,
    command: hide_node_link::panes::HookCommand,
    deadline: Instant,
) -> Result<Value, String> {
    use hide_agent_hooks::hook_request::{Caller, send};
    let request_id = fresh_request_id()?;
    let caller = Caller {
        state_dir: &env.state_dir,
        pane_id: env.pane_id.as_deref(),
        install: env.install.as_deref(),
        named: named.as_deref().map(Path::new),
    };
    let sent = send(&caller, &request_id, command, deadline);
    if sent.reference_missing {
        hide_core::diagnostic!(json!({
            "component": "workspace_cli",
            "kind": "hook.reference_missing",
            "request_id": request_id,
            "pane_id": env.pane_id,
        }));
    }
    sent.answer.map_err(|failure| {
        if failure.step {
            hide_core::diagnostic!(json!({
                "component": "workspace_cli",
                "kind": "hook.request_failed",
                "request_id": request_id,
                "pane_id": env.pane_id,
                "reason": failure.reason,
                "io_kind": failure.error.as_ref().map(|error| format!("{:?}", error.kind())),
            }));
        }
        failure.reason.to_owned()
    })
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

/// Tells the core this caller read `view_id` (PRD unseen-agent-views D-22).
/// The read has already answered, so a failure here leaves the caller's
/// answer as it is and is said once on stderr, the CLI's log.
pub fn note_view_read(env: &Env, view_id: &str) {
    let outcome = Credential::acquire(env).and_then(|mut credential| {
        fresh_request_id().and_then(|request_id| {
            request_action(
                &mut credential,
                Action::Read {
                    view_id: view_id.to_owned(),
                },
                &request_id,
            )
        })
    });
    let refused = match &outcome {
        Ok(answer) if answer["ok"] == true => return,
        Ok(answer) => answer["reason"].as_str().unwrap_or("refused").to_owned(),
        Err(reason) => reason.clone(),
    };
    eprintln!(
        "{}",
        json!({
            "component": "hide",
            "kind": "view_read.unsent",
            "view": view_id,
            "reason": refused,
        })
    );
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
    hide_agent_hooks::hook_request::fresh_request_id().map_err(str::to_owned)
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
    use hide_agent_hooks::hook_request::install::{INSTALL_UNCONFIRMED, OTHER_INSTALL};
    use std::sync::{Arc, mpsc};

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

    /// A daemon or node a hook may ask: it answers every request it is
    /// handed with `answer` until it is closed, and says which requests it
    /// was handed. A connection that sends nothing, as a bridge's does when
    /// the hook connects to it and asks another, is not a request.
    struct Asked {
        closer: hide_platform::ipc::ListenerCloser,
        served: std::thread::JoinHandle<Vec<Value>>,
    }

    impl Asked {
        fn serve<L>(listener: L, answer: impl Fn(&Value) -> Value + Send + 'static) -> Self
        where
            L: std::ops::Deref<Target = hide_platform::ipc::LocalListener> + Send + 'static,
        {
            let closer = listener.closer();
            let served = std::thread::spawn(move || {
                let mut asked = Vec::new();
                while let Ok(mut stream) = listener.accept() {
                    let mut line = String::new();
                    let _ = BufReader::new(&mut stream).read_line(&mut line);
                    let Ok(request) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    let _ = writeln!(stream, "{}", answer(&request));
                    asked.push(request);
                }
                asked
            });
            Self { closer, served }
        }

        /// This state folder's daemon.
        fn daemon(state: &Path, answer: impl Fn(&Value) -> Value + Send + 'static) -> Self {
            private::create_dir_all(&state.join("pane-capabilities")).unwrap();
            let (listener, _) = crate::pane_auth::bind(state).unwrap();
            Self::serve(listener, answer)
        }

        /// A node pane service in this state folder's bridges.
        fn bridge(state: &Path, answer: impl Fn(&Value) -> Value + Send + 'static) -> Self {
            let bridges = hide_kit::layout::workspace_bridges(state);
            private::create_dir_all(&bridges).unwrap();
            let folder = bridges.join("bridge-node");
            private::create_dir(&folder).unwrap();
            let listener =
                hide_platform::ipc::LocalListener::bind(&folder.join("bootstrap.sock")).unwrap();
            Self::serve(Box::new(listener), answer)
        }

        /// Stops answering and returns what it was asked.
        fn close(self) -> Vec<Value> {
            self.closer.close();
            self.served.join().unwrap()
        }
    }

    /// The answer of a daemon or node that ran the hook's request, naming
    /// `install`, or no installation at all as an earlier build does.
    fn ran(install: Option<&str>) -> impl Fn(&Value) -> Value + Send + 'static {
        let install = install.map(str::to_owned);
        move |request| {
            let mut answer =
                json!({"ok": true, "request_id": request["hook"]["request_id"], "result": {}});
            if let Some(install) = &install {
                answer["install"] = json!(install);
            }
            answer
        }
    }

    /// A daemon or node of another installation than the pane's.
    fn refuses_other_install(_: &Value) -> Value {
        json!({"ok": false, "reason": OTHER_INSTALL})
    }

    /// A hook's request from `env`, and the cause a hook records for its
    /// failure.
    fn ask_hook(env: &Env) -> Result<Value, (String, &'static str)> {
        let command = hide_node_link::panes::HookCommand::WorkspaceInfo {};
        hook_request(env, None, command, Instant::now() + Duration::from_secs(10)).map_err(
            |reason| {
                let cause = hide_agent_hooks::delivery::refusal_cause(Some(&reason));
                (reason, cause)
            },
        )
    }

    /// A hook's request names its pane's installation and takes an answer
    /// only when the daemon names the same one (with-hide/hide-private#575).
    /// An earlier build's daemon names none and binds the pane by its folder,
    /// having already run the command, so its answer is refused and the
    /// command is not sent again to the node beside it; neither is one
    /// naming another installation. A pane that names none takes the answer
    /// as before.
    #[test]
    fn a_hook_takes_an_answer_only_from_its_panes_installation() {
        let app = hide_agent_hooks::route::APP_INSTALL;
        let ask = |pane: Option<&str>, answering: Option<&str>| {
            let state = short_dir();
            let daemon = Asked::daemon(state.path(), ran(answering));
            let node = Asked::bridge(state.path(), ran(pane));
            let mut env = env_at(state.path());
            env.install = pane.map(str::to_owned);
            let answer = ask_hook(&env);
            let asked = daemon.close();
            assert_eq!(asked.len(), 1);
            assert_eq!(asked[0]["install"], json!(pane));
            assert_eq!(node.close(), Vec::<Value>::new(), "asked once only");
            answer.map(|answer| answer["ok"] == true)
        };
        let unconfirmed = Err((INSTALL_UNCONFIRMED.to_owned(), "unconfirmed"));
        assert_eq!(ask(Some(app), None), unconfirmed);
        assert_eq!(ask(Some(app), Some("root:/helper")), unconfirmed);
        assert_eq!(ask(Some(app), Some(app)), Ok(true));
        assert_eq!(ask(None, None), Ok(true));
    }

    /// A node pane service in this state folder's bridges, in its own folder
    /// `name`, that declares `declared` as a current node does, or nothing
    /// as an earlier one.
    fn declared_bridge(
        state: &Path,
        name: &str,
        declared: Option<Option<&str>>,
        answer: impl Fn(&Value) -> Value + Send + 'static,
    ) -> (PathBuf, Asked) {
        let bridges = hide_kit::layout::workspace_bridges(state);
        private::create_dir_all(&bridges).unwrap();
        let folder = bridges.join(name);
        private::create_dir(&folder).unwrap();
        if let Some(install) = declared {
            install::declare_bridge(&folder, install).unwrap();
        }
        let listener =
            hide_platform::ipc::LocalListener::bind(&folder.join("bootstrap.sock")).unwrap();
        (folder, Asked::serve(Box::new(listener), answer))
    }

    /// Two nodes in one device's bridges folder, the pane's own and another
    /// installation's: the pane's hook and its `hide` both take the own
    /// node's answer, and neither ever asks the other node
    /// (with-hide/hide-private#622, group C).
    #[test]
    fn a_hook_and_hide_answer_alike_beside_another_installations_node() {
        let state = short_dir();
        let pane = hide_agent_hooks::route::install_of_root(&state.path().join("helper"));
        let bridges = hide_kit::layout::workspace_bridges(state.path());
        let reference = bridges.join("bridge-own").join("ref.json");
        let wire = hide_platform::path::to_wire(&reference).unwrap();
        let run = ran(Some(&pane));
        let own_install = pane.clone();
        let (_, own) = declared_bridge(
            state.path(),
            "bridge-own",
            Some(Some(&pane)),
            move |request| {
                if request["hook"].is_object() {
                    run(request)
                } else {
                    json!({"ok": true, "reference": wire, "install": own_install})
                }
            },
        );
        let (_, other) = declared_bridge(
            state.path(),
            "bridge-other",
            Some(Some(hide_agent_hooks::route::APP_INSTALL)),
            refuses_other_install,
        );
        let mut env = env_at(state.path());
        env.install = Some(pane.clone());
        assert_eq!(ask_hook(&env).map(|answer| answer["ok"] == true), Ok(true));
        assert_eq!(bootstrap(&env, true), Ok(reference));
        let asked = own.close();
        assert_eq!(asked.len(), 2, "the own node is asked by each: {asked:?}");
        assert!(
            asked
                .iter()
                .all(|request| request["install"] == json!(pane))
        );
        assert_eq!(
            other.close(),
            Vec::<Value>::new(),
            "another installation's node is never asked"
        );
    }

    /// A hook's command lands on at most one node: with two that may each be
    /// the pane's, the pane's own by its declaration and one from before
    /// nodes declared theirs, the hook asks neither and says why.
    #[test]
    fn a_hooks_command_reaches_at_most_one_node() {
        let state = short_dir();
        let pane = hide_agent_hooks::route::install_of_root(&state.path().join("helper"));
        let (_, own) = declared_bridge(
            state.path(),
            "bridge-own",
            Some(Some(&pane)),
            ran(Some(&pane)),
        );
        let (_, earlier) = declared_bridge(state.path(), "bridge-earlier", None, ran(Some(&pane)));
        let daemon = Asked::daemon(state.path(), refuses_other_install);
        let mut env = env_at(state.path());
        env.install = Some(pane);
        assert_eq!(
            ask_hook(&env),
            Err((install::AMBIGUOUS_PANE.to_owned(), "refused"))
        );
        assert_eq!(own.close(), Vec::<Value>::new());
        assert_eq!(earlier.close(), Vec::<Value>::new());
        assert_eq!(daemon.close(), Vec::<Value>::new());
    }

    /// A device helper's pane is its node's, so its hook asks the node
    /// before this account's daemon, which would only refuse it as another
    /// installation's. A route moves on only past a refusal of another
    /// installation, which ran nothing, and when only such refusals answer,
    /// that is the hook's reason.
    #[test]
    fn a_hook_asks_its_panes_own_installation_first() {
        // The device pane's own node answers, and the daemon is never asked.
        let state = short_dir();
        let pane = hide_agent_hooks::route::install_of_root(&state.path().join("helper"));
        let daemon = Asked::daemon(state.path(), refuses_other_install);
        let node = Asked::bridge(state.path(), ran(Some(&pane)));
        let mut env = env_at(state.path());
        env.install = Some(pane.clone());
        assert_eq!(ask_hook(&env).map(|answer| answer["ok"] == true), Ok(true));
        assert_eq!(node.close().len(), 1);
        assert_eq!(daemon.close(), Vec::<Value>::new());

        // With no node of its own, only the daemon's refusal answers.
        let state = short_dir();
        let daemon = Asked::daemon(state.path(), refuses_other_install);
        let mut env = env_at(state.path());
        env.install = Some(pane.clone());
        assert_eq!(
            ask_hook(&env),
            Err((OTHER_INSTALL.to_owned(), "other_install"))
        );
        assert_eq!(daemon.close().len(), 1);

        // An app pane whose state folder's daemon is another installation's
        // goes on to the node, which is the pane's own.
        let state = short_dir();
        let app = hide_agent_hooks::route::APP_INSTALL;
        let daemon = Asked::daemon(state.path(), refuses_other_install);
        let node = Asked::bridge(state.path(), ran(Some(app)));
        let mut env = env_at(state.path());
        env.install = Some(app.to_owned());
        assert_eq!(ask_hook(&env).map(|answer| answer["ok"] == true), Ok(true));
        assert_eq!(daemon.close().len(), 1);
        assert_eq!(node.close().len(), 1);
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
    /// stream: the node sees the caller's pid and asks the pane's holder about
    /// it, and stopping the node removes it from this device.
    #[test]
    fn a_device_node_answers_this_device_until_it_stops() {
        let directory = short_dir();
        let bridges = directory.path().join("bridges");
        let (sent, _lines) = mpsc::channel();
        let output = std::sync::Mutex::new(Lines(Vec::new(), sent));
        let panes = hide_host::panes::Panes::new();
        let request = json!({"pane_id": "w1:p1", "nonce": "0".repeat(32)});
        // The node is stopped before anything is asserted, so a failed
        // assertion cannot leave the scope waiting on its thread.
        let (socket, answer) = std::thread::scope(|scope| {
            let started = panes
                .start(
                    scope,
                    &output,
                    &bridges,
                    hide_host::pane_holders::PaneSlot::default(),
                )
                .unwrap();
            let answer = bootstrap_bridges(&bridges, &request, None);
            panes.stop();
            (started.socket, answer)
        });
        // Both ends read and write the wire spelling, which on Windows is the
        // only one `from_wire` reads.
        assert!(
            node_socket(&socket).unwrap().starts_with(&bridges),
            "{socket}"
        );
        // The node was given no pane holders, so it cannot read the pane; it
        // got that far only by seeing who called.
        assert_eq!(answer, Err("bridge_unavailable".to_owned()));
        panes.remove_folder();
        assert_eq!(bootstrap_bridges(&bridges, &request, None), Ok(None));
    }

    /// A device helper's pane whose own node does not answer is asked of
    /// this account's daemon. A daemon from before answers named their
    /// installation ignores the request's `install` and binds the pane by its
    /// folder, so its reference is refused (with-hide/hide-private#497), as is
    /// one naming another installation; an answer naming the pane's own is
    /// taken.
    #[test]
    fn a_pane_takes_a_reference_only_from_its_own_installation() {
        // `answering` names the answer's installation from the pane's.
        let ask = |answering: fn(&str) -> Option<String>| {
            let state = short_dir();
            let reference = state.path().join("pane-capabilities").join("ref.json");
            let pane = hide_agent_hooks::route::install_of_root(&state.path().join("helper"));
            let mut answer = json!({
                "ok": true,
                "reference": hide_platform::path::to_wire(&reference).unwrap(),
            });
            if let Some(install) = answering(&pane) {
                answer["install"] = json!(install);
            }
            let (hears, served) = hook_listener(state.path(), move |_, stream| {
                writeln!(stream, "{answer}").unwrap();
            });
            let mut env = env_at(state.path());
            env.install = Some(pane.clone());
            let outcome = bootstrap(&env, true);
            let request = hears.recv_timeout(Duration::from_secs(10)).unwrap();
            served.join().unwrap();
            assert_eq!(request["install"], pane.as_str());
            outcome.map(|taken| assert_eq!(taken, reference))
        };
        let unconfirmed = Err(INSTALL_UNCONFIRMED.to_owned());
        assert_eq!(ask(|_| None), unconfirmed);
        assert_eq!(
            ask(|_| Some(hide_agent_hooks::route::APP_INSTALL.to_owned())),
            unconfirmed
        );
        assert_eq!(ask(|pane| Some(pane.to_owned())), Ok(()));
    }

    /// A request that names no installation, from a pane no installation
    /// started, takes the answer as before, naming one or not.
    #[test]
    fn an_answer_is_checked_only_for_the_installation_the_request_named() {
        let read = |answer: Value, install: Option<&str>| {
            read_bootstrap_answer(&mut format!("{answer}\n").as_bytes(), install)
        };
        let state = short_dir();
        let reference = state.path().join("pane-capabilities").join("ref.json");
        let wire = hide_platform::path::to_wire(&reference).unwrap();
        assert_eq!(
            read(json!({"ok": true, "reference": wire}), None),
            Ok(reference.clone())
        );
        assert_eq!(
            read(
                json!({"ok": true, "reference": wire, "install": "app"}),
                None
            ),
            Ok(reference.clone())
        );
        assert_eq!(
            read(
                json!({"ok": true, "reference": wire, "install": null}),
                Some("app")
            ),
            Err(INSTALL_UNCONFIRMED.to_owned())
        );
        // A refusal keeps its own reason whatever installation it names.
        assert_eq!(
            read(json!({"ok": false, "reason": "other_install"}), Some("app")),
            Err("other_install".to_owned())
        );
    }

    /// Two nodes in one bridges folder, one of them another installation's
    /// that proves a caller by its folder: only the node that names the
    /// pane's installation answers for it, and the other neither succeeds
    /// nor makes the pane ambiguous.
    #[test]
    fn a_bridge_of_another_installation_does_not_answer_for_the_pane() {
        let directory = short_dir();
        let bridges = directory.path().join("bridges");
        private::create_dir_all(&bridges).unwrap();
        let pane = "root:/helper";
        let node = |name: &str, install: Option<&str>| {
            let folder = bridges.join(name);
            private::create_dir(&folder).unwrap();
            let reference = folder.join("ref.json");
            let mut answer = json!({
                "ok": true,
                "reference": hide_platform::path::to_wire(&reference).unwrap(),
            });
            if let Some(install) = install {
                answer["install"] = json!(install);
            }
            let listener =
                hide_platform::ipc::LocalListener::bind(&folder.join("bootstrap.sock")).unwrap();
            let served = std::thread::spawn(move || {
                let mut stream = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&mut stream).read_line(&mut line).unwrap();
                writeln!(stream, "{answer}").unwrap();
            });
            (reference, served)
        };
        let (own, own_served) = node("bridge-own", Some(pane));
        let (_, app_served) = node("bridge-app", Some(hide_agent_hooks::route::APP_INSTALL));
        let request = json!({"pane_id": "w1:p1", "nonce": "0".repeat(32), "install": pane});
        let answer = bootstrap_bridges(&bridges, &request, Some(pane));
        own_served.join().unwrap();
        app_served.join().unwrap();
        assert_eq!(answer, Ok(Some(own)));

        // With only the other installation's node, the pane has no answer.
        let (_, app_served) = node("bridge-older", None);
        fs::remove_dir_all(bridges.join("bridge-own")).unwrap();
        fs::remove_dir_all(bridges.join("bridge-app")).unwrap();
        let answer = bootstrap_bridges(&bridges, &request, Some(pane));
        app_served.join().unwrap();
        assert_eq!(answer, Err(INSTALL_UNCONFIRMED.to_owned()));
    }

    /// During the update gap the desktop app's pane runs a newer `hide` than
    /// this account's daemon, which answers without naming its installation,
    /// and a current device node in the same bridges folder refuses the pane
    /// as another installation's. The pane hears its own daemon's reason, with
    /// its advice to reopen Hide, not the other node's.
    #[test]
    fn an_older_daemons_reason_outlasts_another_installations_refusal() {
        let state = short_dir();
        let reference = state.path().join("pane-capabilities").join("ref.json");
        let answer = json!({
            "ok": true,
            "reference": hide_platform::path::to_wire(&reference).unwrap(),
        });
        let (_, served) = hook_listener(state.path(), move |_, stream| {
            writeln!(stream, "{answer}").unwrap();
        });
        let bridges = hide_kit::layout::workspace_bridges(state.path());
        private::create_dir_all(&bridges).unwrap();
        let node = bridges.join("bridge-device");
        private::create_dir(&node).unwrap();
        let listener =
            hide_platform::ipc::LocalListener::bind(&node.join("bootstrap.sock")).unwrap();
        let refused = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            let answer = json!({"ok": false, "reason": OTHER_INSTALL});
            writeln!(stream, "{answer}").unwrap();
        });
        let mut env = env_at(state.path());
        env.install = Some(hide_agent_hooks::route::APP_INSTALL.to_owned());
        let outcome = bootstrap(&env, true);
        served.join().unwrap();
        refused.join().unwrap();
        assert_eq!(outcome, Err(INSTALL_UNCONFIRMED.to_owned()));
    }

    /// The desktop app's daemon in the node role, attached to another
    /// machine's core, opens no bootstrap socket, so its app pane's `hide`
    /// finds no daemon and asks the node, whose refusal is the one the
    /// caller can act on, not the missing daemon's `hide_unavailable`.
    #[test]
    fn an_app_pane_without_a_daemon_hears_its_nodes_refusal() {
        let state = short_dir();
        let bridges = hide_kit::layout::workspace_bridges(state.path());
        private::create_dir_all(&bridges).unwrap();
        let node = bridges.join("bridge-node");
        private::create_dir(&node).unwrap();
        let listener =
            hide_platform::ipc::LocalListener::bind(&node.join("bootstrap.sock")).unwrap();
        let refused = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            let answer = json!({"ok": false, "reason": "checkout_not_registered"});
            writeln!(stream, "{answer}").unwrap();
        });
        let mut env = env_at(state.path());
        env.install = Some(hide_agent_hooks::route::APP_INSTALL.to_owned());
        let outcome = bootstrap(&env, true);
        refused.join().unwrap();
        assert_eq!(outcome, Err("checkout_not_registered".to_owned()));
    }

    /// A device helper's pane reaches its own node through the bridges even
    /// while this account's daemon would answer for it; the desktop app's
    /// pane, and a pane that names no installation, reach the daemon first.
    #[test]
    fn a_panes_hide_reaches_the_installation_that_started_it() {
        let daemon = || Ok(PathBuf::from("daemon"));
        let bridge = || Ok(Some(PathBuf::from("bridge")));
        let device = install::order(Some("root:/helper"));
        let app = install::order(Some(hide_agent_hooks::route::APP_INSTALL));
        assert_eq!(in_pane_order(device, daemon, bridge), Ok("bridge".into()));
        assert_eq!(in_pane_order(app, daemon, bridge), Ok("daemon".into()));
        // A node that does not hold the pane leaves it to the daemon.
        assert_eq!(
            in_pane_order(device, daemon, || Ok(None)),
            Ok("daemon".into())
        );
        // Refused by both, a device pane says why its node refused.
        assert_eq!(
            in_pane_order(
                device,
                || Err("hide_unavailable".to_owned()),
                || Err("pane_unavailable".to_owned())
            ),
            Err("pane_unavailable".to_owned())
        );
    }

    /// A plain shell's `hide <path>` on a device asks the one node of its
    /// link and reads the core's answer; with that node stopped, or a second
    /// core's node beside it, it is refused with a reason at once.
    #[test]
    fn a_plain_shell_opens_through_its_device_s_one_node() {
        use hide_node_link::panes::ProofAnswer;
        let directory = short_dir();
        let bridges = directory.path().join("bridges");
        let (sent, lines) = mpsc::channel();
        let output = std::sync::Mutex::new(Lines(Vec::new(), sent));
        let panes = hide_host::panes::Panes::new();
        let other = hide_host::panes::Panes::new();
        let answer = json!({"type":"workspace_result","ok":true,"result":{"label":"app","path":"/home/me/app","added":true,"window":true}});
        std::thread::scope(|scope| {
            panes
                .start(
                    scope,
                    &output,
                    &bridges,
                    hide_host::pane_holders::PaneSlot::default(),
                )
                .unwrap();
            let panes = &panes;
            let answered = answer.clone();
            let core = scope.spawn(move || {
                let event = next_event(&lines);
                assert_eq!(event["event"], "project_open");
                assert_eq!(event["path"], "/home/me/app");
                let request = event["request"].as_u64().unwrap();
                panes
                    .answer_proof(request, ProofAnswer::ProjectOpenAccepted)
                    .unwrap();
                panes
                    .answer_proof(request, ProofAnswer::ProjectOpen { answer: answered })
                    .unwrap();
            });
            assert_eq!(
                project_open_on_bridge(&bridges, "/home/me/app"),
                Ok(answer.clone())
            );
            core.join().unwrap();
            other
                .start(
                    scope,
                    &output,
                    &bridges,
                    hide_host::pane_holders::PaneSlot::default(),
                )
                .unwrap();
            assert_eq!(
                project_open_on_bridge(&bridges, "/home/me/app"),
                Err("bridge_ambiguous".to_owned())
            );
            other.stop();
            panes.stop();
        });
        other.remove_folder();
        // A folder a killed node left still holds its socket's path, which
        // no listener answers any more.
        panes.remove_folder();
        let dead = bridges.join("bridge-dead");
        private::create_dir_all(&dead).unwrap();
        private::create_new_file(&dead.join("bootstrap.sock")).unwrap();
        let asked = Instant::now();
        assert_eq!(
            project_open_on_bridge(&bridges, "/home/me/app"),
            Err(NOT_LINKED.to_owned())
        );
        assert!(asked.elapsed() < Duration::from_secs(2));
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
        let (sent, lines) = mpsc::channel();
        let output = std::sync::Mutex::new(Lines(Vec::new(), sent));
        let panes = hide_host::panes::Panes::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        std::thread::scope(|scope| {
            let started = panes
                .start(
                    scope,
                    &output,
                    &bridges,
                    hide_host::pane_holders::PaneSlot::default(),
                )
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
            install: None,
            access: None,
            tailscale_bin: Some(directory.join("no-tailscale")),
            relay_url: "ws://127.0.0.1:9/ws".to_owned(),
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

    /// A one-shot command against a running daemon writes no claimed
    /// marker, and the daemon revokes its token once the connection ends; a
    /// command for a pane the engine cannot prove is refused and its
    /// credential revoked before any claim. This process stands in for the
    /// pane's shell, so every credential stays alive until the daemon or the
    /// command ends it. A named reference's claim needs a pane the engine
    /// runs, and the device contract proves it
    /// (`workspace_guidance_gets_one_reusable_reference_and_rejects_writes`).
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
            hide_core::workspace_control::Context {
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
        // The daemon's engine runs no pane `w1:p1`, so each command is
        // refused with that and its credential revoked.
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

        // The same command on a token the daemon would keep: the refusal
        // revokes the credential, so neither side records a claim.
        let (path, token) = issue('2', None);
        let mut credential = one_shot(&path);
        answered(&mut credential);
        assert!(!path.with_extension("claimed").exists());
        assert_eq!(registry.claimed(&token), None);
        drop(credential);

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
