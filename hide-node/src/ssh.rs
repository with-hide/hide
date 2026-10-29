//! Remote host, transport, and projection boundaries for the IDE.
//!
//! The module intentionally keeps the product contract separate from the SSH
//! implementation. Alias import and state projection are deterministic and
//! testable without a network. The russh adapter is the only production
//! transport; no OpenSSH subprocess is used by this path.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use russh::client::{self, Handle, Handler, Msg};
use russh::keys::{
    PrivateKeyWithHashAlg, PublicKeyOrCertificate, agent::client::AgentClient,
    check_known_hosts_path,
};
use russh::{Channel, ChannelMsg, Disconnect, Pty};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{Semaphore, oneshot};

use hide_herdr_client::{ApiConnector, ApiError, ApiStream, ConnectionShutdown};

mod device;
pub mod host;
pub mod hosts;
mod identities;
pub mod profiles;
pub mod transfer;
pub mod upstream;

pub use device::{Connector, SshDevice};
pub use host::{LinkStream, LinkWriter, OpenError, PaneEvents, PaneEventsSlot, RemoteHost};
pub use identities::IdentityFile;
use identities::{AgentState, KeyFile, Offer, Plan};

use hide_node_link::device::{
    CapabilityReport, HOST_KEY_CHANGED, HOST_KEY_UNKNOWN, RemoteError, RemoteHostIdentity,
    RemoteResult, RemoteStage, SnapshotCheck, valid_remote_socket_path,
};
const SSH_OPERATION_TIMEOUT: Duration = Duration::from_secs(15);
/// Session channels (an exec, a shell or a subsystem) a device's connection
/// keeps open at once. OpenSSH's sshd allows ten per connection
/// (`MaxSessions`), and the node's link holds one for as long as it lives; a
/// forward's channels are not sessions and are not counted.
const MAX_SESSION_CHANNELS: usize = 8;

// Russh spawns its session task after receiving the server's SSH banner but
// before returning a Handle. If the caller cancels during key exchange, its
// connect future drops without aborting that task. Shutdown of this duplicate
// socket closes the task's stream even when no Handle was returned.
struct ConnectingSocket(Option<std::net::TcpStream>);

impl ConnectingSocket {
    fn release(&mut self) {
        self.0.take();
    }
}

impl Drop for ConnectingSocket {
    fn drop(&mut self) {
        if let Some(socket) = self.0.take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}
const DEFAULT_REMOTE_TERM: &str = "xterm-256color";

/// An environment name this module reads, with what its absence does.
struct EnvironmentRead {
    key: &'static str,
    #[allow(dead_code, reason = "the declaration documents the contract")]
    missing_behavior: &'static str,
}

/// Environment names read by this module. Values remain in the process
/// environment and are never copied into diagnostics or remote commands.
const REMOTE_PROCESS_ENVIRONMENT: &[EnvironmentRead] = &[
    EnvironmentRead {
        key: "HOME",
        missing_behavior: "required for a home-relative IdentityAgent, which fails visibly without it",
    },
    EnvironmentRead {
        key: "SSH_AUTH_SOCK",
        missing_behavior: "optional: agent authentication uses the ssh config IdentityAgent, and reports an explicit action-required failure when neither is set",
    },
];

fn remote_error(
    operation_id: &str,
    target: &str,
    stage: RemoteStage,
    error: impl fmt::Display,
    retryable: bool,
    action_required: bool,
) -> RemoteError {
    RemoteError::new(
        operation_id,
        target,
        stage,
        error.to_string(),
        retryable,
        action_required,
    )
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SshAlias {
    /// Stable local identity based on the alias, not a resolved IP address.
    pub host_id: String,
    pub alias: String,
    pub hostname: String,
    pub user: String,
    pub port: u16,
    /// The key files sign-in reads, in order: every `IdentityFile` the ssh
    /// config gives this alias, or OpenSSH's default files in `~/.ssh` when
    /// it gives none (`IdentityFile none` leaves it empty).
    pub identity_files: Vec<IdentityFile>,
    /// `IdentitiesOnly yes`: the agent is used only for one of
    /// `identity_files`, never for its other keys.
    pub identities_only: bool,
    /// Where agent authentication looks for its socket, resolved from the SSH
    /// config the same way OpenSSH resolves it.
    pub agent_socket: AgentSocket,
    pub known_hosts_file: PathBuf,
}

/// The agent socket an alias authenticates through.
///
/// OpenSSH resolves this from `IdentityAgent` and only falls back to
/// `SSH_AUTH_SOCK` when no directive matched. Reading `SSH_AUTH_SOCK` alone
/// reaches whichever agent the launching process happened to carry, which for a
/// Finder launch is the empty launchd agent rather than the one the user
/// configured.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AgentSocket {
    /// No `IdentityAgent` matched the alias, so `SSH_AUTH_SOCK` decides.
    Environment,
    /// `IdentityAgent none`: this host authenticates without an agent.
    Disabled,
    /// `IdentityAgent <path>`: this socket, whatever the environment holds.
    Path(PathBuf),
}

impl AgentSocket {
    /// Names the source in diagnostics. The socket path itself never appears,
    /// because it is a routing value the environment contract keeps out of
    /// diagnostics.
    fn source(&self) -> &'static str {
        match self {
            Self::Environment => "SSH_AUTH_SOCK",
            Self::Disabled => "IdentityAgent none",
            Self::Path(_) => "the ssh config IdentityAgent",
        }
    }
}

impl SshAlias {
    /// A saved target may be an SSH alias, `user@host`, or an SSH URI.
    /// Resolve configuration without making a connection or running ssh.
    pub fn from_target_file(path: &Path, target: &str) -> RemoteResult<Self> {
        let contents = profiles::ssh_config(path).map_err(|error| alias_error(target, error))?;
        Self::from_target_contents(target, &contents, ssh_dir(target, path)?)
    }

    fn from_target_contents(target: &str, contents: &str, ssh_dir: &Path) -> RemoteResult<Self> {
        let parsed = profiles::Target::parse(target).map_err(|error| alias_error(target, error))?;
        let config = russh_config::parse(contents, &parsed.host)
            .map_err(|error| alias_error(target, error.to_string()))?;
        if config
            .host_config
            .proxy_command
            .as_deref()
            .is_some_and(|value| value != "none")
            || config
                .host_config
                .proxy_jump
                .as_deref()
                .is_some_and(|value| value != "none")
        {
            return Err(alias_error(
                target,
                "This SSH target requires a proxy route the device connector cannot use",
            ));
        }
        let mut alias = Self::from_config_contents(&parsed.host, contents, ssh_dir)?;
        if let Some(user) = parsed.user {
            alias.user = user;
        }
        if let Some(port) = parsed.port {
            alias.port = port;
        }
        alias.alias = target.to_owned();
        alias.host_id = format!("ssh:{target}");
        Ok(alias)
    }
    /// Resolves `alias` from an ssh config's text. `ssh_dir` is the folder the
    /// config belongs to (the account's `~/.ssh`): its `known_hosts` and
    /// OpenSSH's default key files are read there, and `~` in an
    /// `IdentityFile` is the folder above it.
    pub fn from_config_contents(alias: &str, contents: &str, ssh_dir: &Path) -> RemoteResult<Self> {
        validate_alias(alias)?;
        let config = russh_config::parse(contents, alias).map_err(|error| {
            remote_error(
                "ssh-alias-import",
                alias,
                RemoteStage::Alias,
                error,
                false,
                true,
            )
        })?;
        let hostname = config.host().trim().to_owned();
        if hostname.is_empty() || hostname.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(remote_error(
                "ssh-alias-import",
                alias,
                RemoteStage::Alias,
                "Host entry does not resolve a hostname",
                false,
                true,
            ));
        }
        let user = config.user().trim().to_owned();
        if user.is_empty() || user.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(remote_error(
                "ssh-alias-import",
                alias,
                RemoteStage::Alias,
                "Host entry does not resolve a usable user",
                false,
                true,
            ));
        }
        let directives = AuthDirectives::scan(alias, contents);
        let agent_socket = match directives.identity_agent {
            Some(value) => resolve_identity_agent(alias, value)?,
            None => AgentSocket::Environment,
        };
        let identities_only = match directives.identities_only {
            None => false,
            Some(value) if value.eq_ignore_ascii_case("yes") || value.eq_ignore_ascii_case("true") => true,
            Some(value) if value.eq_ignore_ascii_case("no") || value.eq_ignore_ascii_case("false") => false,
            Some(_) => return Err(alias_error(alias, "IdentitiesOnly must be yes or no")),
        };
        // OpenSSH adds its default files only when no IdentityFile matched,
        // and reads none at all for `IdentityFile none`.
        let identity_files = if directives.identity_files.is_empty() {
            identities::default_identity_files(ssh_dir)
        } else {
            let home = ssh_dir.parent().ok_or_else(|| {
                alias_error(alias, "the ssh config's folder has no home folder above it")
            })?;
            directives
                .identity_files
                .iter()
                .filter(|value| !value.trim_matches('"').eq_ignore_ascii_case("none"))
                .map(|value| {
                    identities::expand_identity_file(value, home)
                        .map_err(|error| alias_error(alias, error))
                })
                .collect::<RemoteResult<Vec<_>>>()?
        };
        Ok(Self {
            host_id: format!("ssh:{alias}"),
            alias: alias.to_owned(),
            hostname,
            user,
            port: config.port(),
            identity_files,
            identities_only,
            agent_socket,
            known_hosts_file: ssh_dir.join("known_hosts"),
        })
    }

    pub fn from_config_file(path: &Path, alias: &str) -> RemoteResult<Self> {
        let contents = std::fs::read_to_string(path).map_err(|error| {
            remote_error(
                "ssh-alias-import",
                &path.display().to_string(),
                RemoteStage::Alias,
                error,
                true,
                false,
            )
        })?;
        // The account the config belongs to keeps its hosts and keys beside
        // it, as OpenSSH's own defaults (`~/.ssh/known_hosts`, `~/.ssh/id_*`)
        // do for that home.
        Self::from_config_contents(alias, &contents, ssh_dir(alias, path)?)
    }

    pub fn identity(&self) -> RemoteHostIdentity {
        RemoteHostIdentity {
            host_id: self.host_id.clone(),
            alias: self.alias.clone(),
            hostname: self.hostname.clone(),
            port: self.port,
        }
    }

    pub fn target(&self) -> String {
        format!("{}@{}:{}", self.user, self.hostname, self.port)
    }
}

pub fn import_ssh_aliases_from_str(contents: &str, ssh_dir: &Path) -> RemoteResult<Vec<SshAlias>> {
    let mut aliases = scan_alias_names(contents);
    aliases.sort();
    aliases.dedup();
    aliases
        .into_iter()
        .map(|alias| SshAlias::from_config_contents(&alias, contents, ssh_dir))
        .collect()
}

fn scan_alias_names(contents: &str) -> Vec<String> {
    let mut names = BTreeSet::new();
    for line in contents.lines() {
        let line = line.trim();
        let Some((keyword, values)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if !keyword.eq_ignore_ascii_case("host") {
            continue;
        }
        for value in values.split_whitespace() {
            if value.is_empty()
                || value.starts_with('!')
                || value.contains('*')
                || value.contains('?')
            {
                continue;
            }
            names.insert(value.to_owned());
        }
    }
    names.into_iter().collect()
}

/// The folder an ssh config file sits in, where its account keeps
/// `known_hosts` and its keys.
fn ssh_dir<'a>(alias: &str, config: &'a Path) -> RemoteResult<&'a Path> {
    config
        .parent()
        .ok_or_else(|| alias_error(alias, "the ssh config path names no folder"))
}

/// The directives that decide how one alias signs in, as written.
///
/// `russh_config` drops `IdentityAgent` and `IdentitiesOnly` and expands `~`
/// in `IdentityFile` against the process's home rather than the account's, so
/// the `Host` block matching that already decides hostname and user is
/// repeated here. `Match` blocks are ignored, exactly as they are for every
/// other directive this module reads.
#[derive(Default)]
struct AuthDirectives<'a> {
    identity_agent: Option<&'a str>,
    identities_only: Option<&'a str>,
    /// Every matching line, in order: `IdentityFile` adds up across blocks.
    identity_files: Vec<&'a str>,
}

impl<'a> AuthDirectives<'a> {
    fn scan(alias: &str, contents: &'a str) -> Self {
        let mut found = Self::default();
        let mut applies = false;
        for line in contents.lines() {
            let Some((keyword, value)) = split_config_line(line) else {
                continue;
            };
            if keyword.eq_ignore_ascii_case("host") {
                applies = host_patterns_match(alias, value);
            } else if keyword.eq_ignore_ascii_case("match") {
                applies = false;
            } else if applies {
                match keyword.to_ascii_lowercase().as_str() {
                    "identityfile" => found.identity_files.push(value),
                    // OpenSSH keeps the first value it obtains for a keyword.
                    "identityagent" => {
                        found.identity_agent.get_or_insert(value);
                    }
                    "identitiesonly" => {
                        found.identities_only.get_or_insert(value);
                    }
                    _ => {}
                }
            }
        }
        found
    }
}

fn split_config_line(line: &str) -> Option<(&str, &str)> {
    let line = line.split('#').next().unwrap_or_default().trim();
    let index = line.find(|character: char| character.is_whitespace() || character == '=')?;
    let (keyword, remainder) = line.split_at(index);
    let value = remainder
        .trim_start_matches(|character: char| character.is_whitespace() || character == '=')
        .trim_end();
    (!value.is_empty()).then_some((keyword, value))
}

/// OpenSSH `Host` matching: one positive pattern has to match and no negated
/// pattern may.
fn host_patterns_match(alias: &str, patterns: &str) -> bool {
    let mut matched = false;
    for pattern in patterns.split_whitespace() {
        if let Some(negated) = pattern.strip_prefix('!') {
            if matches_host_pattern(alias, negated) {
                return false;
            }
        } else if matches_host_pattern(alias, pattern) {
            matched = true;
        }
    }
    matched
}

fn matches_host_pattern(candidate: &str, pattern: &str) -> bool {
    let candidate: Vec<char> = candidate.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    // Iterative wildcard match: `star` remembers the last `*` so a failed tail
    // can retry one character later without recursing.
    let (mut c, mut p) = (0usize, 0usize);
    let (mut star, mut retry) = (None, 0usize);
    while c < candidate.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == candidate[c]) {
            c += 1;
            p += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            retry = c;
            p += 1;
        } else if let Some(star) = star {
            p = star + 1;
            retry += 1;
            c = retry;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|character| *character == '*')
}

fn resolve_identity_agent(alias: &str, value: &str) -> RemoteResult<AgentSocket> {
    let value = value.trim_matches('"');
    if value.eq_ignore_ascii_case("none") {
        return Ok(AgentSocket::Disabled);
    }
    if value == "SSH_AUTH_SOCK" || value == "$SSH_AUTH_SOCK" {
        return Ok(AgentSocket::Environment);
    }
    if value.starts_with('$') {
        return Err(alias_error(
            alias,
            "IdentityAgent names an environment variable outside the declared remote environment contract",
        ));
    }
    let socket = expand_identity_agent_path(alias, value)?;
    if !socket.is_absolute() {
        return Err(alias_error(
            alias,
            "IdentityAgent must resolve to an absolute path to the agent's socket or pipe",
        ));
    }
    Ok(AgentSocket::Path(socket))
}

fn expand_identity_agent_path(alias: &str, value: &str) -> RemoteResult<PathBuf> {
    if let Some(rest) = value.strip_prefix("~/").or(value.strip_prefix("%d/")) {
        let home = read_remote_environment("HOME")
            .map_err(|error| alias_error(alias, error))?
            .map(PathBuf::from)
            .ok_or_else(|| {
                alias_error(
                    alias,
                    "IdentityAgent is home-relative and HOME is not set, so it cannot be resolved",
                )
            })?;
        return Ok(home.join(expand_percent_escapes(alias, rest)?));
    }
    Ok(PathBuf::from(expand_percent_escapes(alias, value)?))
}

fn expand_percent_escapes(alias: &str, value: &str) -> RemoteResult<String> {
    let mut expanded = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            expanded.push(character);
            continue;
        }
        match characters.next() {
            Some('%') => expanded.push('%'),
            _ => {
                return Err(alias_error(
                    alias,
                    "IdentityAgent carries a percent token this shell does not expand",
                ));
            }
        }
    }
    Ok(expanded)
}

fn alias_error(alias: &str, cause: impl fmt::Display) -> RemoteError {
    remote_error(
        "ssh-alias-import",
        alias,
        RemoteStage::Alias,
        cause,
        false,
        true,
    )
}

fn validate_alias(alias: &str) -> RemoteResult<()> {
    if !hide_node_link::device::ssh_alias_is_one_token(alias) {
        return Err(remote_error(
            "ssh-alias-import",
            alias,
            RemoteStage::Alias,
            "alias must be one non-empty SSH config token",
            false,
            true,
        ));
    }
    Ok(())
}

fn read_remote_environment(key: &str) -> io::Result<Option<OsString>> {
    if !REMOTE_PROCESS_ENVIRONMENT
        .iter()
        .any(|contract| contract.key == key)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unregistered remote environment key {key}"),
        ));
    }
    Ok(std::env::var_os(key))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RemoteReadCommand {
    GitStatus {
        root: String,
    },
    /// `herdr status server --json`: the host's own answer to where its
    /// server socket is and whether the server is up. `socket` is the socket
    /// the device registration names, when it names one; otherwise the host's
    /// default server answers.
    HerdrServerStatus {
        socket: Option<String>,
        /// The helper root whose pinned Herdr copy is asked first (PRD
        /// device-herdr-update D-10), when the build brought one there.
        helper_root: Option<String>,
    },
    /// A saved profile's exact named server. Explicit CLI session selection
    /// wins over any environment the remote account happens to carry.
    HerdrSessionStatus {
        session: String,
        helper_root: Option<String>,
    },
}

/// Where a non-login SSH exec finds `herdr` on the remote host: the
/// installer's user prefix, Homebrew, and the system paths. A login shell
/// would print its banners into the JSON, so the PATH is spelled out instead.
const REMOTE_HERDR_PATH: &str = "$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin";

/// The probe's `PATH`: the pinned Herdr copy under `helper_root` first,
/// when the root is spelled plainly enough to sit inside double quotes, and
/// [`REMOTE_HERDR_PATH`] after it, so a device the build brought no Herdr
/// to answers from its own.
fn remote_herdr_path(helper_root: Option<&str>) -> String {
    let pinned = helper_root.and_then(|root| {
        let plain = |rest: &str| {
            !rest.is_empty()
                && rest
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte))
        };
        let spelled = match root.strip_prefix("~/") {
            Some(rest) if plain(rest) => format!("$HOME/{rest}"),
            None if root.starts_with('/') && plain(root) => root.to_owned(),
            _ => return None,
        };
        Some(format!(
            "{}/{}/{}",
            spelled.trim_end_matches('/'),
            hide_kit::device_herdr::HERDR_DIR,
            hide_kit::CURRENT
        ))
    });
    match pinned {
        Some(pinned) => format!("{pinned}:{REMOTE_HERDR_PATH}"),
        None => REMOTE_HERDR_PATH.to_owned(),
    }
}

impl RemoteReadCommand {
    fn operation_id(&self) -> &'static str {
        match self {
            Self::GitStatus { .. } => "remote-git-status",
            Self::HerdrServerStatus { .. } | Self::HerdrSessionStatus { .. } => {
                "remote-herdr-status"
            }
        }
    }

    fn stage(&self) -> RemoteStage {
        match self {
            Self::GitStatus { .. } => RemoteStage::Git,
            Self::HerdrServerStatus { .. } | Self::HerdrSessionStatus { .. } => RemoteStage::Herdr,
        }
    }

    fn command_line(&self) -> RemoteResult<String> {
        match self {
            Self::HerdrSessionStatus {
                session,
                helper_root,
            } => {
                profiles::validate_session(session).map_err(|error| alias_error(session, error))?;
                Ok(format!(
                    "PATH=\"{}\" herdr --session {} status server --json",
                    remote_herdr_path(helper_root.as_deref()),
                    shell_quote(session)
                ))
            }
            Self::HerdrServerStatus {
                socket: None,
                helper_root,
            } => Ok(format!(
                "PATH=\"{}\" herdr status server --json",
                remote_herdr_path(helper_root.as_deref())
            )),
            Self::HerdrServerStatus {
                socket: Some(socket),
                helper_root,
            } => {
                if !valid_remote_socket_path(socket) {
                    return Err(remote_error(
                        self.operation_id(),
                        socket,
                        self.stage(),
                        "the device's Herdr socket must be an absolute single-line path",
                        false,
                        false,
                    ));
                }
                Ok(format!(
                    "HERDR_SOCKET_PATH={} PATH=\"{}\" herdr status server --json",
                    shell_quote(socket),
                    remote_herdr_path(helper_root.as_deref())
                ))
            }
            Self::GitStatus { root } => {
                if !root.starts_with('/')
                    || root
                        .bytes()
                        .any(|byte| matches!(byte, b'\n' | b'\r' | b'\0'))
                {
                    return Err(remote_error(
                        self.operation_id(),
                        root,
                        self.stage(),
                        "Git root must be an absolute single-line path",
                        false,
                        true,
                    ));
                }
                Ok(format!(
                    "git -C {} status --short --porcelain=v1",
                    shell_quote(root)
                ))
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteCommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_status: u32,
}

/// The fields of `herdr status server --json` the IDE reads. The CLI is the
/// boundary here, not the socket schema: the socket cannot be reached before
/// this answer says where it is.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct RemoteHerdrServerStatus {
    pub running: bool,
    pub socket: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub protocol: Option<u32>,
}

/// The remembered socket is one owned `String`, so a panic while it was held
/// cannot have left it half-written; the value is taken as it is.
fn lock_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Exit status a POSIX shell reports when the command is not on its PATH.
const EXIT_COMMAND_NOT_FOUND: u32 = 127;

/// Reads the server status the remote host printed, and turns each way it
/// can fail into the action the operator takes on that host.
pub fn parse_herdr_server_status(
    host_id: &str,
    output: &RemoteCommandOutput,
) -> RemoteResult<RemoteHerdrServerStatus> {
    let operation_id = RemoteReadCommand::HerdrServerStatus {
        socket: None,
        helper_root: None,
    }
    .operation_id();
    if output.exit_status == EXIT_COMMAND_NOT_FOUND {
        return Err(remote_error(
            operation_id,
            host_id,
            RemoteStage::Herdr,
            format!(
                "Herdr was not found on {host_id}. Allow and install Hide's helper to install the Herdr Hide brings."
            ),
            false,
            true,
        ));
    }
    if output.exit_status != 0 {
        return Err(remote_error(
            operation_id,
            host_id,
            RemoteStage::Herdr,
            format!(
                "herdr status server --json exited {} on {host_id}: {}",
                output.exit_status,
                redact_output(&output.stderr)
            ),
            true,
            false,
        ));
    }
    let status: RemoteHerdrServerStatus =
        serde_json::from_str(output.stdout.trim()).map_err(|error| {
            remote_error(
                operation_id,
                host_id,
                RemoteStage::Herdr,
                format!("herdr status server --json on {host_id} was not readable: {error}"),
                false,
                true,
            )
        })?;
    if !hide_platform::path::is_wire_absolute(&status.socket)
        || status.socket.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(remote_error(
            operation_id,
            host_id,
            RemoteStage::Herdr,
            "remote Herdr socket path must be absolute and single-line",
            false,
            true,
        ));
    }
    Ok(status)
}

/// One device client's Tokio runtime. Its clones travel with channels,
/// forwards and file routes into hided's own async tasks, so the last one can
/// be dropped inside another runtime, where the blocking shutdown of a plain
/// drop panics; there it shuts down in the background instead.
struct RemoteRuntime(Option<Runtime>);

impl std::ops::Deref for RemoteRuntime {
    type Target = Runtime;

    fn deref(&self) -> &Runtime {
        self.0
            .as_ref()
            .expect("a remote runtime is taken only when it drops")
    }
}

impl Drop for RemoteRuntime {
    fn drop(&mut self) {
        let Some(runtime) = self.0.take() else {
            return;
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            runtime.shutdown_background();
        }
    }
}

#[derive(Clone)]
pub struct RusshRemoteClient {
    host: SshAlias,
    runtime: Arc<RemoteRuntime>,
    /// The answer `herdr status server --json` reported on the host. It is
    /// asked for on first use and forgotten when a socket open fails, so a
    /// server restarted under another path or version is found again on the
    /// next attempt instead of failing forever on the remembered one.
    herdr_status: Arc<Mutex<Option<RemoteHerdrServerStatus>>>,
    /// The Herdr socket the device registration names, for a host whose
    /// server does not listen at its default path.
    herdr_socket: Option<String>,
    herdr_session: Option<String>,
    /// The helper root Hide installs under on the device, whose pinned Herdr
    /// the status read asks first.
    helper_root: Option<String>,
    /// The device's one SSH connection, shared by every clone.
    connection: Arc<Connection>,
}

/// The device's one SSH connection (PRD core-host-node b4): the node's link,
/// the Herdr API channels, attachment SFTP, the Herdr status read and the
/// browser forwards all open their channels on it. The first use dials it,
/// and a use after it closed dials it again; the last clone of the device's
/// client ends it. Terminals and the capability test still dial their own.
struct Connection {
    runtime: Arc<RemoteRuntime>,
    session: tokio::sync::Mutex<Option<Arc<Handle<KnownHostHandler>>>>,
    /// Hears the session in `session` end, set with it under its lock.
    ended: Mutex<Option<tokio::sync::watch::Receiver<bool>>>,
    /// The host key the last dial accepted, the identity consent binds.
    observed_key: Arc<Mutex<Option<String>>>,
    sessions: Arc<Semaphore>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(session) = self.session.get_mut().take() {
            self.runtime.spawn(async move {
                let _ = session
                    .disconnect(Disconnect::ByApplication, "device closed", "en")
                    .await;
            });
        }
    }
}

#[derive(Clone)]
pub(crate) struct RusshApiConnector {
    connection: Arc<RusshApiConnection>,
}

impl fmt::Debug for RusshApiConnector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RusshApiConnector")
            .field("host_id", &self.connection.client.host.host_id)
            .finish()
    }
}

struct RusshApiConnection {
    client: RusshRemoteClient,
}

async fn bounded_ssh_operation<T>(
    operation: impl std::future::Future<Output = Result<T, russh::Error>>,
) -> Result<T, String> {
    tokio::time::timeout(SSH_OPERATION_TIMEOUT, operation)
        .await
        .map_err(|_| "SSH operation timed out".to_owned())?
        .map_err(|error| error.to_string())
}

impl RusshApiConnection {
    fn open_stream_until(
        &self,
        socket_path: String,
        deadline: std::time::Instant,
    ) -> Result<russh::ChannelStream<Msg>, ApiError> {
        let client = &self.client;
        client.runtime.block_on(async {
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
                // Only an existing session can serve passive proof. Dialing
                // could synchronously read account keys/known_hosts, which
                // an async timer cannot interrupt.
                let session = client
                    .connection
                    .session
                    .lock()
                    .await
                    .as_ref()
                    .filter(|session| !session.is_closed())
                    .cloned()
                    .ok_or_else(|| {
                        ApiError::Transport(
                            "SSH session is unavailable for a deadline-bound request".to_owned(),
                        )
                    })?;
                session
                    .channel_open_direct_streamlocal(socket_path)
                    .await
                    .map(|channel| channel.into_stream())
                    .map_err(|error| ApiError::Transport(error.to_string()))
            })
            .await
            .map_err(|_| ApiError::Transport("SSH connection deadline expired".to_owned()))?
        })
    }

    fn open_stream(&self, socket_path: &str) -> Result<russh::ChannelStream<Msg>, ApiError> {
        let client = &self.client;
        client.runtime.block_on(async {
            let session = client
                .shared_session()
                .await
                .map_err(|error| ApiError::Transport(error.to_string()))?;
            match bounded_ssh_operation(
                session.channel_open_direct_streamlocal(socket_path.to_owned()),
            )
            .await
            {
                Ok(channel) => Ok(channel.into_stream()),
                Err(error) => {
                    client.forget_session(&session).await;
                    client.forget_herdr_socket();
                    Err(ApiError::Transport(format!(
                        "remote Herdr socket open failed for {}: {error}",
                        client.host.host_id
                    )))
                }
            }
        })
    }
}

struct RusshApiShutdown {
    stopped: Arc<AtomicBool>,
}

impl ConnectionShutdown for RusshApiShutdown {
    fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
    }
}

struct RusshApiStream {
    runtime: Arc<RemoteRuntime>,
    _connection: Arc<RusshApiConnection>,
    stream: Option<russh::ChannelStream<Msg>>,
    stopped: Arc<AtomicBool>,
    read_timeout: Mutex<Option<Duration>>,
    write_timeout: Mutex<Option<Duration>>,
}

impl RusshApiStream {
    fn timeout(slot: &Mutex<Option<Duration>>, operation: &str) -> io::Result<Option<Duration>> {
        slot.lock().map(|timeout| *timeout).map_err(|_| {
            io::Error::other(format!(
                "remote Herdr {operation} timeout state is poisoned"
            ))
        })
    }

    fn stream(&mut self) -> io::Result<&mut russh::ChannelStream<Msg>> {
        self.stream.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "remote Herdr socket stream is closed",
            )
        })
    }
}

impl Read for RusshApiStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let deadline =
            Self::timeout(&self.read_timeout, "read")?.map(|value| Instant::now() + value);
        loop {
            if self.stopped.load(Ordering::Acquire) {
                return Ok(0);
            }
            let wait = match deadline {
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "remote Herdr socket read timed out",
                        ));
                    }
                    remaining.min(Duration::from_millis(100))
                }
                None => Duration::from_millis(100),
            };
            let runtime = Arc::clone(&self.runtime);
            let stream = self.stream()?;
            match runtime.block_on(async { tokio::time::timeout(wait, stream.read(buffer)).await })
            {
                Ok(result) => return result,
                Err(_) => continue,
            }
        }
    }
}

impl Write for RusshApiStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "remote Herdr socket stream is closed",
            ));
        }
        let timeout = Self::timeout(&self.write_timeout, "write")?.unwrap_or(SSH_OPERATION_TIMEOUT);
        let runtime = Arc::clone(&self.runtime);
        let stream = self.stream()?;
        runtime
            .block_on(async { tokio::time::timeout(timeout, stream.write(buffer)).await })
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "remote Herdr socket write timed out",
                )
            })?
    }

    fn flush(&mut self) -> io::Result<()> {
        let timeout = Self::timeout(&self.write_timeout, "write")?.unwrap_or(SSH_OPERATION_TIMEOUT);
        let runtime = Arc::clone(&self.runtime);
        let stream = self.stream()?;
        runtime
            .block_on(async { tokio::time::timeout(timeout, stream.flush()).await })
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "remote Herdr socket flush timed out",
                )
            })?
    }
}

impl ApiStream for RusshApiStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), ApiError> {
        *self.read_timeout.lock().map_err(|_| {
            ApiError::Transport("remote Herdr read timeout state is poisoned".to_owned())
        })? = timeout;
        Ok(())
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> Result<(), ApiError> {
        *self.write_timeout.lock().map_err(|_| {
            ApiError::Transport("remote Herdr write timeout state is poisoned".to_owned())
        })? = timeout;
        Ok(())
    }

    fn read_line_with_timeout(&mut self, timeout: Duration) -> Result<String, ApiError> {
        let previous = Self::timeout(&self.read_timeout, "read")
            .map_err(|error| ApiError::Transport(error.to_string()))?;
        let deadline = Instant::now() + timeout;
        let mut bytes = Vec::new();
        let result = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break Err(ApiError::Transport(
                    "subscription acknowledgement timed out".to_owned(),
                ));
            }
            self.set_read_timeout(Some(remaining))?;
            let mut byte = [0_u8; 1];
            match self.read(&mut byte) {
                Ok(0) => {
                    break Err(ApiError::Transport(
                        "subscription acknowledgement reached EOF".to_owned(),
                    ));
                }
                Ok(_) => {
                    bytes.push(byte[0]);
                    if byte[0] == b'\n' {
                        break String::from_utf8(bytes).map_err(|error| {
                            ApiError::Malformed(format!(
                                "subscription acknowledgement was not UTF-8: {error}"
                            ))
                        });
                    }
                    if bytes.len() > 64 * 1024 {
                        break Err(ApiError::Malformed(
                            "subscription acknowledgement exceeds 64 KiB".to_owned(),
                        ));
                    }
                }
                Err(error) => break Err(ApiError::Transport(error.to_string())),
            }
        };
        self.set_read_timeout(previous)?;
        result
    }

    fn shutdown_handle(&self) -> Result<Box<dyn ConnectionShutdown>, ApiError> {
        Ok(Box::new(RusshApiShutdown {
            stopped: Arc::clone(&self.stopped),
        }))
    }
}

impl Drop for RusshApiStream {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let stream = self.stream.take();
        self.runtime.block_on(async { drop(stream) });
    }
}

impl ApiConnector for RusshApiConnector {
    fn connect_until(&self, deadline: std::time::Instant) -> Result<Box<dyn ApiStream>, ApiError> {
        let socket_path = self
            .connection
            .client
            .cached_herdr_socket()
            .ok_or_else(|| {
                ApiError::Transport(
                    "Herdr socket is unavailable for a deadline-bound request".to_owned(),
                )
            })?;
        let stream = self.connection.open_stream_until(socket_path, deadline)?;
        Ok(Box::new(RusshApiStream {
            runtime: Arc::clone(&self.connection.client.runtime),
            _connection: Arc::clone(&self.connection),
            stream: Some(stream),
            stopped: Arc::new(AtomicBool::new(false)),
            read_timeout: Mutex::new(None),
            write_timeout: Mutex::new(None),
        }))
    }

    fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError> {
        let socket_path = self
            .connection
            .client
            .herdr_socket_path()
            .map_err(|error| ApiError::Transport(error.to_string()))?;
        let stream = self.connection.open_stream(&socket_path)?;
        Ok(Box::new(RusshApiStream {
            runtime: Arc::clone(&self.connection.client.runtime),
            _connection: Arc::clone(&self.connection),
            stream: Some(stream),
            stopped: Arc::new(AtomicBool::new(false)),
            read_timeout: Mutex::new(None),
            write_timeout: Mutex::new(None),
        }))
    }
}

impl fmt::Debug for RusshRemoteClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RusshRemoteClient")
            .field("host_id", &self.host.host_id)
            .field("hostname", &self.host.hostname)
            .field("port", &self.host.port)
            .finish_non_exhaustive()
    }
}

impl RusshRemoteClient {
    pub fn new(host: SshAlias) -> RemoteResult<Self> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|error| {
                remote_error(
                    "remote-runtime",
                    &host.host_id,
                    RemoteStage::Ssh,
                    error,
                    false,
                    true,
                )
            })?;
        let runtime = Arc::new(RemoteRuntime(Some(runtime)));
        Ok(Self {
            host,
            connection: Arc::new(Connection {
                runtime: Arc::clone(&runtime),
                session: tokio::sync::Mutex::new(None),
                ended: Mutex::new(None),
                observed_key: Arc::new(Mutex::new(None)),
                sessions: Arc::new(Semaphore::new(MAX_SESSION_CHANNELS)),
            }),
            runtime,
            herdr_status: Arc::new(Mutex::new(None)),
            herdr_socket: None,
            herdr_session: None,
            helper_root: None,
        })
    }

    /// The device's connection, dialed when there is none or the last one
    /// closed.
    async fn shared_session(&self) -> RemoteResult<Arc<Handle<KnownHostHandler>>> {
        self.shared_session_heard()
            .await
            .map(|(session, _ended)| session)
    }

    /// The device's connection, as [`Self::shared_session`], with what
    /// hears that session end: work on one of its channels that the server
    /// no longer answers ends on that notification, not on a timer.
    async fn shared_session_heard(
        &self,
    ) -> RemoteResult<(
        Arc<Handle<KnownHostHandler>>,
        tokio::sync::watch::Receiver<bool>,
    )> {
        let mut slot = self.connection.session.lock().await;
        if let Some(session) = slot.as_ref().filter(|session| !session.is_closed())
            && let Some(ended) = lock_recover(&self.connection.ended).clone()
        {
            return Ok((Arc::clone(session), ended));
        }
        let (end, ended) = tokio::sync::watch::channel(false);
        let handler = KnownHostHandler::new(&self.host)
            .with_observed_key(Arc::clone(&self.connection.observed_key))
            .with_end(end);
        let session = Arc::new(self.connect(handler).await?);
        *slot = Some(Arc::clone(&session));
        *lock_recover(&self.connection.ended) = Some(ended.clone());
        Ok((session, ended))
    }

    /// Drops `session` as the device's connection when a channel could not
    /// be opened on it, so the next use dials again; a newer connection
    /// another use already made is kept.
    async fn forget_session(&self, session: &Arc<Handle<KnownHostHandler>>) {
        let mut slot = self.connection.session.lock().await;
        if slot.as_ref().is_some_and(|held| Arc::ptr_eq(held, session)) {
            *slot = None;
            drop(slot);
            let _ = bounded_ssh_operation(session.disconnect(
                Disconnect::ByApplication,
                "channel open failed",
                "en",
            ))
            .await;
        }
    }

    /// Ends the device's connection now, whatever still holds it; the next
    /// use dials again.
    pub(crate) fn disconnect(&self) {
        let connection = Arc::clone(&self.connection);
        let ended = async move {
            let session = connection.session.lock().await.take();
            if let Some(session) = session {
                let _ = bounded_ssh_operation(session.disconnect(
                    Disconnect::ByApplication,
                    "node closed",
                    "en",
                ))
                .await;
            }
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            self.runtime.spawn(ended);
        } else {
            self.runtime.block_on(ended);
        }
    }

    /// The host key the connection's last dial accepted.
    fn observed_host_key(&self) -> Option<String> {
        lock_recover(&self.connection.observed_key).clone()
    }

    /// Admission for one session channel on the device's connection, held
    /// while the channel is open. Past [`MAX_SESSION_CHANNELS`] a use waits
    /// for one to close, at most the SSH operation bound, and then fails.
    async fn session_channel(
        &self,
        operation_id: &str,
        stage: RemoteStage,
    ) -> RemoteResult<tokio::sync::OwnedSemaphorePermit> {
        self.session_channel_within(operation_id, stage, SSH_OPERATION_TIMEOUT)
            .await
    }

    async fn session_channel_within(
        &self,
        operation_id: &str,
        stage: RemoteStage,
        wait: Duration,
    ) -> RemoteResult<tokio::sync::OwnedSemaphorePermit> {
        match tokio::time::timeout(wait, Arc::clone(&self.connection.sessions).acquire_owned())
            .await
        {
            Ok(Ok(permit)) => Ok(permit),
            _ => {
                crate::diagnostic!(json!({
                    "component": "remote",
                    "kind": "connection.sessions_full",
                    "target": self.host.host_id,
                    "operation": operation_id,
                    "limit": MAX_SESSION_CHANNELS,
                }));
                Err(remote_error(
                    operation_id,
                    &self.host.host_id,
                    stage,
                    format!(
                        "the device's SSH connection already has {MAX_SESSION_CHANNELS} sessions open"
                    ),
                    true,
                    false,
                ))
            }
        }
    }

    /// Talks to the Herdr server at `socket` on the host instead of the one
    /// at its default path.
    pub fn with_herdr_socket(mut self, socket: Option<String>) -> Self {
        self.herdr_socket = socket;
        self
    }

    pub fn with_herdr_session(mut self, session: Option<String>) -> Self {
        self.herdr_session = session;
        self
    }

    /// Asks the pinned Herdr under `root` on the host first, when there is
    /// one (PRD device-herdr-update D-10).
    pub fn with_helper_root(mut self, root: Option<String>) -> Self {
        self.helper_root = root;
        self
    }

    pub fn host(&self) -> &SshAlias {
        &self.host
    }

    /// The remote Herdr socket, asked of the host on the first call and
    /// remembered until [`Self::forget_herdr_socket`].
    pub fn herdr_socket_path(&self) -> RemoteResult<String> {
        if let Some(status) = lock_recover(&self.herdr_status).clone() {
            return Ok(status.socket);
        }
        let command = match &self.herdr_session {
            Some(session) => RemoteReadCommand::HerdrSessionStatus {
                session: session.clone(),
                helper_root: self.helper_root.clone(),
            },
            None => RemoteReadCommand::HerdrServerStatus {
                socket: self.herdr_socket.clone(),
                helper_root: self.helper_root.clone(),
            },
        };
        let output = self.exec_read_only(command)?;
        let status = parse_herdr_server_status(&self.host.host_id, &output)?;
        let socket = status.socket.clone();
        *lock_recover(&self.herdr_status) = Some(status);
        Ok(socket)
    }

    pub(crate) fn cached_herdr_socket(&self) -> Option<String> {
        lock_recover(&self.herdr_status)
            .as_ref()
            .map(|status| status.socket.clone())
    }

    pub(crate) fn cached_herdr_version(&self) -> Option<String> {
        lock_recover(&self.herdr_status)
            .as_ref()
            .and_then(|status| status.version.clone())
    }

    pub(crate) fn herdr_session_readable(&self) -> bool {
        lock_recover(&self.herdr_status)
            .as_ref()
            .is_some_and(|status| {
                status.running
                    && status.protocol.map(u64::from)
                        == Some(hide_herdr_client::HERDR_PROTOCOL_REVISION)
            })
    }

    pub(crate) fn forget_herdr_socket(&self) {
        *lock_recover(&self.herdr_status) = None;
    }

    pub(crate) fn herdr_api_connector(&self) -> RusshApiConnector {
        RusshApiConnector {
            connection: Arc::new(RusshApiConnection {
                client: self.clone(),
            }),
        }
    }

    pub fn staged_capability_test(
        &self,
        operation_id: &str,
        check_snapshot: SnapshotCheck<'_>,
    ) -> CapabilityReport {
        let mut report = CapabilityReport::new(operation_id, self.host.identity());
        let connection = self
            .runtime
            .block_on(self.connect(KnownHostHandler::new(&self.host)));
        match connection {
            Ok(session) => {
                report.pass(
                    RemoteStage::Ssh,
                    "TCP SSH handshake and known_hosts verification passed",
                );
                report.pass(
                    RemoteStage::Auth,
                    "configured key or SSH agent authentication passed",
                );
                let _ = self.runtime.block_on(session.disconnect(
                    Disconnect::ByApplication,
                    "capability probe complete",
                    "en",
                ));
            }
            // Authentication runs after the handshake and the known_hosts
            // check, so a refused sign-in is the auth stage's failure alone,
            // never read as a host that could not be reached (B38).
            Err(error) if error.stage() == RemoteStage::Auth => {
                report.pass(
                    RemoteStage::Ssh,
                    "TCP SSH handshake and known_hosts verification passed",
                );
                report.fail(
                    RemoteStage::Auth,
                    error.to_string(),
                    error.diagnostic().retryable,
                    error.diagnostic().action_required,
                );
                return report;
            }
            Err(error) => {
                report.fail(
                    RemoteStage::Ssh,
                    error.to_string(),
                    error.diagnostic().retryable,
                    error.diagnostic().action_required,
                );
                if error.stage() != RemoteStage::Ssh {
                    report.fail(
                        error.stage(),
                        error.to_string(),
                        error.diagnostic().retryable,
                        error.diagnostic().action_required,
                    );
                }
                return report;
            }
        }

        // A stale remembered socket would make the probe answer for a server
        // that has since moved; the probe asks the host afresh.
        self.forget_herdr_socket();
        match self.fetch_herdr_snapshot(check_snapshot) {
            Ok(protocol) => {
                report.pass(
                    RemoteStage::Herdr,
                    "official Herdr Socket API snapshot responded",
                );
                report.pass(RemoteStage::Protocol, format!("Herdr protocol={protocol}"));
            }
            Err(error) => {
                report.fail(
                    error.stage(),
                    error.to_string(),
                    error.diagnostic().retryable,
                    error.diagnostic().action_required,
                );
                if error.stage() != RemoteStage::Protocol {
                    report.fail(
                        RemoteStage::Protocol,
                        error.to_string(),
                        error.diagnostic().retryable,
                        error.diagnostic().action_required,
                    );
                }
            }
        }

        match self.probe_pty() {
            Ok(()) => report.pass(
                RemoteStage::Pty,
                "remote PTY allocation and teardown passed",
            ),
            Err(error) => report.fail(
                RemoteStage::Pty,
                error.to_string(),
                error.diagnostic().retryable,
                error.diagnostic().action_required,
            ),
        }

        match self.probe_sftp() {
            Ok(path) => report.pass(RemoteStage::Sftp, format!("canonical root {path}")),
            Err(error) => report.fail(
                error.stage(),
                error.to_string(),
                error.diagnostic().retryable,
                error.diagnostic().action_required,
            ),
        }

        match self.exec_read_only(RemoteReadCommand::GitStatus {
            root: "/".to_owned(),
        }) {
            Ok(output) if output.exit_status == 0 || output.exit_status == 128 => {
                report.pass(
                    RemoteStage::Git,
                    "read-only git status command reached remote host",
                );
            }
            Ok(output) => report.fail(
                RemoteStage::Git,
                format!(
                    "exit={} stderr={}",
                    output.exit_status,
                    redact_output(&output.stderr)
                ),
                true,
                false,
            ),
            Err(error) => report.fail(
                error.stage(),
                error.to_string(),
                error.diagnostic().retryable,
                error.diagnostic().action_required,
            ),
        }

        // The tunnel stage is not probed: opening a forward changes the
        // device's forwarding state, which a test must not do.
        report.fail(
            RemoteStage::Tunnel,
            "tunnel probe is explicit because it changes remote forwarding state",
            false,
            true,
        );
        if report.all_required_passed() {
            report.connected();
        }
        report
    }

    fn probe_pty(&self) -> RemoteResult<()> {
        let session = self
            .runtime
            .block_on(self.connect(KnownHostHandler::new(&self.host)))?;
        let result = self.runtime.block_on(async {
            let channel = session.channel_open_session().await.map_err(|error| {
                remote_error(
                    "remote-pty-probe",
                    &self.host.host_id,
                    RemoteStage::Pty,
                    error,
                    true,
                    false,
                )
            })?;
            channel
                .request_pty(
                    true,
                    DEFAULT_REMOTE_TERM,
                    120,
                    40,
                    0,
                    0,
                    &[] as &[(Pty, u32)],
                )
                .await
                .map_err(|error| {
                    remote_error(
                        "remote-pty-probe",
                        &self.host.host_id,
                        RemoteStage::Pty,
                        error,
                        true,
                        false,
                    )
                })?;
            channel.close().await.map_err(|error| {
                remote_error(
                    "remote-pty-probe",
                    &self.host.host_id,
                    RemoteStage::Pty,
                    error,
                    true,
                    false,
                )
            })?;
            Ok::<(), RemoteError>(())
        });
        let disconnect = self.runtime.block_on(session.disconnect(
            Disconnect::ByApplication,
            "PTY capability probe complete",
            "en",
        ));
        let disconnect = disconnect.map_err(|error| {
            remote_error(
                "remote-pty-probe",
                &self.host.host_id,
                RemoteStage::Cleanup,
                error,
                true,
                false,
            )
        });
        match (result, disconnect) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(primary), Ok(())) => Err(primary),
            (Ok(()), Err(cleanup)) => Err(cleanup),
            (Err(primary), Err(cleanup)) => Err(remote_error(
                "remote-pty-probe",
                &self.host.host_id,
                primary.stage(),
                format!("{}; cleanup failed: {cleanup}", primary.diagnostic().reason),
                primary.diagnostic().retryable,
                primary.diagnostic().action_required,
            )),
        }
    }

    pub fn exec_read_only(&self, command: RemoteReadCommand) -> RemoteResult<RemoteCommandOutput> {
        let operation_id = command.operation_id();
        let stage = command.stage();
        let command_line = command.command_line()?;
        self.runtime.block_on(async {
            let _permit = self.session_channel(operation_id, stage).await?;
            let session = self.shared_session().await?;
            tokio::time::timeout(
                SSH_OPERATION_TIMEOUT,
                execute_channel(
                    &session,
                    &command_line,
                    operation_id,
                    &self.host.host_id,
                    stage,
                ),
            )
            .await
            .map_err(|_| {
                remote_error(
                    operation_id,
                    &self.host.host_id,
                    stage,
                    "remote read timed out",
                    true,
                    false,
                )
            })?
        })
    }

    /// Asks the device's Herdr for its snapshot and hands it to `check`,
    /// which decodes it; the answer is the protocol revision it reports.
    fn fetch_herdr_snapshot(&self, check: SnapshotCheck<'_>) -> RemoteResult<u32> {
        let socket_path = self.herdr_socket_path()?;
        let connector = self.herdr_api_connector();
        let response = hide_herdr_client::request_with_connector(
            &connector,
            "session.snapshot",
            json!({}),
            SSH_OPERATION_TIMEOUT,
        )
        .map_err(|error| self.remote_snapshot_error(error))?;
        // Herdr names no host in its snapshot; the one Hide reached the
        // socket through is the identity the envelope carries.
        check(&response, &self.host.host_id, &socket_path)
    }

    fn remote_snapshot_error(&self, error: ApiError) -> RemoteError {
        let (stage, retryable, action_required) = match &error {
            ApiError::Malformed(_) => (RemoteStage::Protocol, false, true),
            ApiError::NotRunning(_) | ApiError::Transport(_) | ApiError::Remote { .. } => {
                (RemoteStage::Herdr, true, false)
            }
        };
        remote_error(
            "remote-herdr-snapshot",
            &self.host.host_id,
            stage,
            error,
            retryable,
            action_required,
        )
    }

    fn probe_sftp(&self) -> RemoteResult<String> {
        let session = self
            .runtime
            .block_on(self.connect(KnownHostHandler::new(&self.host)))?;
        let operation = self.runtime.block_on(async {
            let channel = session.channel_open_session().await.map_err(|error| {
                remote_error(
                    "remote-sftp-probe",
                    &self.host.host_id,
                    RemoteStage::Sftp,
                    error,
                    true,
                    false,
                )
            })?;
            channel
                .request_subsystem(true, "sftp")
                .await
                .map_err(|error| {
                    remote_error(
                        "remote-sftp-probe",
                        &self.host.host_id,
                        RemoteStage::Sftp,
                        error,
                        true,
                        false,
                    )
                })?;
            let sftp = SftpSession::new(channel.into_stream())
                .await
                .map_err(|error| {
                    remote_error(
                        "remote-sftp-probe",
                        &self.host.host_id,
                        RemoteStage::Sftp,
                        error,
                        true,
                        false,
                    )
                })?;
            let result = sftp.canonicalize(".").await.map_err(|error| {
                remote_error(
                    "remote-sftp-probe",
                    &self.host.host_id,
                    RemoteStage::Sftp,
                    error,
                    true,
                    false,
                )
            })?;
            let cleanup = sftp.close().await.map_err(|error| {
                remote_error(
                    "remote-sftp-probe",
                    &self.host.host_id,
                    RemoteStage::Cleanup,
                    error,
                    true,
                    false,
                )
            });
            combine_cleanup("remote-sftp-probe", &self.host.host_id, Ok(result), cleanup)
        });
        let disconnect = self
            .runtime
            .block_on(session.disconnect(Disconnect::ByApplication, "SFTP probe complete", "en"))
            .map_err(|error| {
                remote_error(
                    "remote-sftp-probe",
                    &self.host.host_id,
                    RemoteStage::Cleanup,
                    error,
                    true,
                    false,
                )
            });
        combine_cleanup(
            "remote-sftp-probe",
            &self.host.host_id,
            operation,
            disconnect,
        )
    }

    async fn connect(&self, handler: KnownHostHandler) -> RemoteResult<Handle<KnownHostHandler>> {
        let config = client::Config {
            inactivity_timeout: Some(SSH_OPERATION_TIMEOUT),
            keepalive_interval: Some(Duration::from_secs(5)),
            ..client::Config::default()
        };
        // Whether the server presented its host key: a connection that ends
        // before it (refused, reset before the SSH greeting, or no answer in
        // time) is a dial that never reached an SSH server, as the node's
        // port probe, which reads the greeting, also counts it.
        let presented = Arc::clone(&handler.presented);
        let rejected = Arc::clone(&handler.rejected);
        let reached_operation = || {
            if presented.load(std::sync::atomic::Ordering::SeqCst) {
                "remote-connect"
            } else {
                hide_node_link::device::DIAL_OPERATION
            }
        };
        tokio::time::timeout(SSH_OPERATION_TIMEOUT, async {
            let socket = TcpStream::connect((self.host.hostname.as_str(), self.host.port))
                .await
                .map_err(|error| {
                    remote_error(
                        hide_node_link::device::DIAL_OPERATION,
                        &self.host.host_id,
                        RemoteStage::Ssh,
                        error,
                        true,
                        false,
                    )
                })?;
            if config.nodelay {
                socket.set_nodelay(true).map_err(|error| {
                    remote_error(
                        "remote-connect",
                        &self.host.host_id,
                        RemoteStage::Ssh,
                        error,
                        true,
                        false,
                    )
                })?;
            }
            let socket = socket.into_std().map_err(|error| {
                remote_error(
                    "remote-connect",
                    &self.host.host_id,
                    RemoteStage::Ssh,
                    error,
                    true,
                    false,
                )
            })?;
            let shutdown = socket.try_clone().map_err(|error| {
                remote_error(
                    "remote-connect",
                    &self.host.host_id,
                    RemoteStage::Ssh,
                    error,
                    true,
                    false,
                )
            })?;
            let mut connecting = ConnectingSocket(Some(shutdown));
            let socket = TcpStream::from_std(socket).map_err(|error| {
                remote_error(
                    "remote-connect",
                    &self.host.host_id,
                    RemoteStage::Ssh,
                    error,
                    true,
                    false,
                )
            })?;
            let mut session = client::connect_stream(Arc::new(config), socket, handler)
                .await
                .map_err(|error| {
                    if rejected.load(std::sync::atomic::Ordering::SeqCst) {
                        remote_error(
                            hide_node_link::device::HOST_KEY_OPERATION,
                            &self.host.host_id,
                            RemoteStage::Ssh,
                            error,
                            false,
                            true,
                        )
                    } else {
                        remote_error(
                            reached_operation(),
                            &self.host.host_id,
                            RemoteStage::Ssh,
                            error,
                            true,
                            false,
                        )
                    }
                })?;
            authenticate(&mut session, &self.host).await?;
            connecting.release();
            Ok(session)
        })
        .await
        .map_err(|_| {
            let operation = reached_operation();
            let reason = if operation == hide_node_link::device::DIAL_OPERATION {
                "no SSH server answered in time"
            } else {
                "SSH connection or authentication timed out"
            };
            remote_error(
                operation,
                &self.host.host_id,
                RemoteStage::Ssh,
                reason,
                true,
                false,
            )
        })?
    }

    /// A browser view on this Mac reaches a loopback server on the device.
    /// One owner holds the listener; dropping it disconnects every channel.
    pub fn start_local_workspace_forward(
        &self,
        remote: SocketAddr,
        alternate: Option<SocketAddr>,
        preserve_numeric_host: bool,
        mut canceled: oneshot::Receiver<()>,
    ) -> RemoteResult<RemoteLocalForward> {
        if !remote.ip().is_loopback()
            || remote.port() == 0
            || alternate.is_some_and(|address| {
                !address.ip().is_loopback() || address.port() != remote.port()
            })
        {
            return Err(remote_error(
                "workspace-browser-forward",
                &self.host.host_id,
                RemoteStage::Tunnel,
                "remote endpoint must be loopback with a nonzero port",
                false,
                true,
            ));
        }
        // Keep numeric loopback hosts unchanged for HTTPS certificate checks.
        // A localhost URL can resolve to either family, so reserve both local
        // addresses at the same port before publishing the route.
        let local_ip = if preserve_numeric_host {
            remote.ip()
        } else if remote.is_ipv6() {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        // A pending route must not reserve a local port before SSH connects:
        // its View can close while the remote handshake is still waiting.
        // The route's channels open on the device's one connection, which
        // the route never ends.
        let session = self.runtime.block_on(async {
            tokio::select! {
                _ = &mut canceled => Err(remote_error(
                    "workspace-browser-forward", &self.host.host_id, RemoteStage::Tunnel,
                    "browser route was canceled", true, false,
                )),
                result = self.shared_session() => result,
            }
        })?;
        if !matches!(
            canceled.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ) {
            return Err(remote_error(
                "workspace-browser-forward",
                &self.host.host_id,
                RemoteStage::Tunnel,
                "browser route was canceled",
                true,
                false,
            ));
        }
        let listeners = self.runtime.block_on(async {
            for _ in 0..16 {
                let primary = TcpListener::bind(SocketAddr::new(local_ip, 0)).await?;
                let primary_addr = primary.local_addr()?;
                if alternate.is_none() {
                    return Ok((primary, None, primary_addr));
                }
                match TcpListener::bind(SocketAddr::new(
                    IpAddr::V6(Ipv6Addr::LOCALHOST),
                    primary_addr.port(),
                ))
                .await
                {
                    Ok(ipv6) => return Ok((primary, Some(ipv6), primary_addr)),
                    Err(error) if error.kind() == io::ErrorKind::AddrInUse => continue,
                    Err(error) => return Err(error),
                }
            }
            Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "could not reserve both localhost address families",
            ))
        });
        let (listener, ipv6_listener, local_addr) = listeners.map_err(|error| {
            remote_error(
                "workspace-browser-forward",
                &self.host.host_id,
                RemoteStage::Tunnel,
                error,
                true,
                false,
            )
        })?;
        let port = local_addr.port();
        let session_task = Arc::clone(&session);
        let failure = Arc::new(Mutex::new(None));
        let failure_task = Arc::clone(&failure);
        let task = self.runtime.spawn(async move {
            const MAX_CONNECTIONS: usize = 16;
            let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
            let mut transfers = tokio::task::JoinSet::new();
            loop {
                let accepted = tokio::select! {
                    accepted = listener.accept() => accepted,
                    accepted = async {
                        match &ipv6_listener {
                            Some(listener) => listener.accept().await,
                            None => std::future::pending().await,
                        }
                    } => accepted,
                    completed = transfers.join_next(), if !transfers.is_empty() => {
                        if let Some(Err(error)) = completed
                            && let Ok(mut slot) = failure_task.lock() {
                                *slot = Some(error.to_string());
                            }
                        continue;
                    },
                };
                let (mut local, _) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        if let Ok(mut slot) = failure_task.lock() {
                            *slot = Some(error.to_string());
                        }
                        break;
                    }
                };
                let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                    let _ = local.shutdown().await;
                    continue;
                };
                let channel = tokio::time::timeout(SSH_OPERATION_TIMEOUT, async {
                    let first = session_task
                        .channel_open_direct_tcpip(
                            remote.ip().to_string(),
                            u32::from(remote.port()),
                            local_ip.to_string(),
                            u32::from(port),
                        )
                        .await;
                    match (first, alternate) {
                        (Err(_), Some(alternate)) => {
                            session_task
                                .channel_open_direct_tcpip(
                                    alternate.ip().to_string(),
                                    u32::from(alternate.port()),
                                    local_ip.to_string(),
                                    u32::from(port),
                                )
                                .await
                        }
                        (answer, _) => answer,
                    }
                })
                .await
                .map_err(|_| "SSH channel open timed out".to_owned())
                .and_then(|answer| answer.map_err(|error| error.to_string()));
                match channel {
                    Ok(channel) => {
                        transfers.spawn(async move {
                            let _permit = permit;
                            let mut stream = channel.into_stream();
                            let _ = tokio::io::copy_bidirectional(&mut local, &mut stream).await;
                        });
                    }
                    Err(error) => {
                        if let Ok(mut slot) = failure_task.lock() {
                            *slot = Some(error.to_string());
                        }
                        let _ = local.shutdown().await;
                    }
                }
            }
            transfers.abort_all();
        });
        Ok(RemoteLocalForward {
            runtime: Arc::clone(&self.runtime),
            local_addr,
            task: Mutex::new(Some(task)),
            failure,
        })
    }
}

/// Signs in the way `ssh <alias>` would from the operator's terminal: the
/// keys of [`identities::Plan`], in OpenSSH's order, until the server accepts
/// one. A refusal says what was offered and, for a key Hide could not use
/// itself, how to give it to the agent.
async fn authenticate(session: &mut Handle<KnownHostHandler>, host: &SshAlias) -> RemoteResult<()> {
    let failed = |error: &dyn fmt::Display, retryable: bool| {
        remote_error(
            "remote-auth",
            &host.host_id,
            RemoteStage::Auth,
            error,
            retryable,
            true,
        )
    };
    let reads: Vec<KeyFile> = host
        .identity_files
        .iter()
        .map(|file| KeyFile::read(&file.path))
        .collect();

    let source = host.agent_socket.source();
    let socket = match &host.agent_socket {
        AgentSocket::Disabled => None,
        AgentSocket::Path(socket) => Some(socket.clone().into_os_string()),
        AgentSocket::Environment => {
            read_remote_environment("SSH_AUTH_SOCK").map_err(|error| failed(&error, false))?
        }
    };
    // An agent that does not answer is passed over, as OpenSSH passes it
    // over, so the key files still get their turn.
    let mut agent = None;
    let mut agent_keys = Vec::new();
    let agent_state = match socket {
        None if host.agent_socket == AgentSocket::Disabled => AgentState::Disabled,
        None => AgentState::NotSet,
        Some(socket) => {
            // OpenSSH's agent listens on a Unix socket, and on Windows on a
            // named pipe (`\\.\pipe\openssh-ssh-agent`), which the same
            // setting names.
            #[cfg(unix)]
            let connected = AgentClient::connect_uds(socket).await;
            #[cfg(windows)]
            let connected = AgentClient::connect_named_pipe(socket).await;
            match connected {
                Ok(mut client) => match client.request_identities().await {
                    Ok(identities) => {
                        agent_keys = identities
                            .iter()
                            .map(|identity| identity.public_key().into_owned())
                            .collect();
                        agent = Some(client);
                        AgentState::Held(agent_keys.len())
                    }
                    Err(_) => AgentState::Unreachable,
                },
                Err(_) => AgentState::Unreachable,
            }
        }
    };

    let plan = Plan::new(&agent_keys, &reads, host.identities_only);
    if !plan.offers.is_empty() {
        // Without the server's preferred hash an RSA identity is signed as
        // ssh-rsa (SHA-1), which every current sshd refuses.
        let hash = session
            .best_supported_rsa_hash()
            .await
            .map_err(|error| failed(&error, true))?
            .flatten();
        for offer in &plan.offers {
            let result = match (*offer, agent.as_mut()) {
                (Offer::Agent(index), Some(agent)) => {
                    session
                        .authenticate_publickey_with(
                            host.user.clone(),
                            agent_keys[index].clone(),
                            hash,
                            agent,
                        )
                        .await
                        .map_err(|error| failed(&error, true))?
                }
                (Offer::Agent(_), None) => continue,
                (Offer::File(index), _) => {
                    let KeyFile::Usable(key) = &reads[index] else {
                        continue;
                    };
                    session
                        .authenticate_publickey(
                            host.user.clone(),
                            PrivateKeyWithHashAlg::new(Arc::new(key.clone()), hash),
                        )
                        .await
                        .map_err(|error| failed(&error, true))?
                }
            };
            if result.success() {
                return Ok(());
            }
        }
    }
    // Nothing offered is the operator's to fix before a retry can help; keys
    // the server refused may be authorized there in the meantime.
    let retryable = !plan.offers.is_empty();
    Err(failed(
        &identities::refusal(agent_state, source, &plan, &host.identity_files, &reads),
        retryable,
    ))
}

async fn execute_channel(
    session: &Handle<KnownHostHandler>,
    command: &str,
    operation_id: &str,
    target: &str,
    stage: RemoteStage,
) -> RemoteResult<RemoteCommandOutput> {
    let mut channel = session
        .channel_open_session()
        .await
        .map_err(|error| remote_error(operation_id, target, stage, error, true, false))?;
    channel
        .exec(true, command)
        .await
        .map_err(|error| remote_error(operation_id, target, stage, error, true, false))?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;
    while let Some(message) = channel.wait().await {
        match message {
            ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
            ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
            ChannelMsg::ExitStatus {
                exit_status: status,
            } => exit_status = Some(status),
            ChannelMsg::Close => break,
            _ => {}
        }
    }
    Ok(RemoteCommandOutput {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        exit_status: exit_status.ok_or_else(|| {
            remote_error(
                operation_id,
                target,
                stage,
                "remote command closed without an exit status",
                true,
                false,
            )
        })?,
    })
}

#[derive(Clone, Debug)]
struct KnownHostHandler {
    host: String,
    port: u16,
    known_hosts_file: PathBuf,
    /// Receives the SHA-256 fingerprint of a host key known_hosts accepted,
    /// the identity a device's helper consent is bound to.
    observed_key: Option<Arc<Mutex<Option<String>>>>,
    /// Set once the server presented its host key: the dial reached an SSH
    /// server, whatever is decided of the key.
    presented: Arc<std::sync::atomic::AtomicBool>,
    /// Set when known_hosts refused the key the server presented.
    rejected: Arc<std::sync::atomic::AtomicBool>,
    /// Told when the session ends: the session owns its handler and drops
    /// it then, whether the server disconnected or the connection failed.
    end: Option<tokio::sync::watch::Sender<bool>>,
}

impl Drop for KnownHostHandler {
    fn drop(&mut self) {
        if let Some(end) = &self.end {
            end.send_replace(true);
        }
    }
}

impl KnownHostHandler {
    fn new(host: &SshAlias) -> Self {
        Self {
            host: host.hostname.clone(),
            port: host.port,
            known_hosts_file: host.known_hosts_file.clone(),
            observed_key: None,
            presented: Arc::default(),
            rejected: Arc::default(),
            end: None,
        }
    }

    fn with_end(mut self, end: tokio::sync::watch::Sender<bool>) -> Self {
        self.end = Some(end);
        self
    }

    fn with_observed_key(mut self, observed_key: Arc<Mutex<Option<String>>>) -> Self {
        self.observed_key = Some(observed_key);
        self
    }
}

impl Handler for KnownHostHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        self.presented
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let checked = self.check_key(server_public_key);
        if checked.is_err() {
            self.rejected
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        checked
    }
}

impl KnownHostHandler {
    fn check_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, anyhow::Error> {
        let public_key = server_public_key.public_key();
        // A changed key and an unknown one need different actions from the
        // operator, so they are named differently (PRD S5.5 B38); neither is
        // ever accepted here.
        let trusted = match check_known_hosts_path(
            &self.host,
            self.port,
            &public_key,
            &self.known_hosts_file,
        ) {
            Ok(trusted) => trusted,
            Err(russh::keys::Error::KeyChanged { line }) => {
                return Err(anyhow!(
                    "{HOST_KEY_CHANGED}: the host key for {}:{} differs from known_hosts line {line}; verify the device before updating known_hosts",
                    self.host,
                    self.port
                ));
            }
            Err(error) => return Err(anyhow!("known_hosts verification failed: {error}")),
        };
        if !trusted {
            return Err(anyhow!(
                "{HOST_KEY_UNKNOWN}: server key is not present in known_hosts for {}:{}; connect once with ssh to review and record it",
                self.host,
                self.port
            ));
        }
        if let Some(observed) = self.observed_key.as_ref() {
            *lock_recover(observed) = Some(
                public_key
                    .fingerprint(russh::keys::HashAlg::Sha256)
                    .to_string(),
            );
        }
        Ok(true)
    }
}

pub struct RemoteLocalForward {
    runtime: Arc<RemoteRuntime>,
    local_addr: SocketAddr,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    failure: Arc<Mutex<Option<String>>>,
}

impl RemoteLocalForward {
    pub fn port(&self) -> u16 {
        self.local_addr.port()
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
    pub fn failure(&self) -> Option<String> {
        self.failure.lock().ok().and_then(|slot| slot.clone())
    }
    pub fn close(&self) {
        let Some(task) = self.task.lock().ok().and_then(|mut task| task.take()) else {
            return;
        };
        // The accept task can be awaiting SSH channel confirmation rather
        // than the listener. Abort it and its owned transfer set immediately;
        // each transfer's channel closes as it is dropped.
        task.abort();
        let local_addr = self.local_addr;
        let cleanup = async move {
            if tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .is_err()
            {
                eprintln!(
                    "{}",
                    json!({"component":"remote","kind":"workspace_forward.task_close_timeout","local_addr":local_addr.to_string()})
                );
            }
        };
        if tokio::runtime::Handle::try_current().is_ok() {
            self.runtime.spawn(cleanup);
        } else {
            self.runtime.block_on(cleanup);
        }
    }
}

impl Drop for RemoteLocalForward {
    fn drop(&mut self) {
        self.close();
    }
}

/// `value` as one word of a POSIX shell command line.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn redact_output(value: &str) -> String {
    format!(
        "redacted_output bytes={} lines={}",
        value.len(),
        value.lines().count()
    )
}

fn combine_cleanup<T>(
    operation_id: &str,
    target: &str,
    result: RemoteResult<T>,
    cleanup: RemoteResult<()>,
) -> RemoteResult<T> {
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(primary), Err(cleanup)) => Err(remote_error(
            operation_id,
            target,
            primary.stage(),
            format!(
                "{}; cleanup failed: {}",
                primary.diagnostic().reason,
                cleanup.diagnostic().reason
            ),
            primary.diagnostic().retryable,
            primary.diagnostic().action_required,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_node_link::device::connection_problem;

    #[test]
    fn ssh_protocol_wait_has_an_absolute_deadline() {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        let result = runtime.block_on(bounded_ssh_operation(std::future::pending::<
            Result<(), russh::Error>,
        >()));
        assert_eq!(result.unwrap_err(), "SSH operation timed out");
    }

    #[test]
    fn a_proof_connection_wait_ends_at_its_deadline_without_dialing() {
        let client = RusshRemoteClient::new(host()).unwrap();
        let connection = Arc::clone(&client.connection);
        let (locked, ready) = std::sync::mpsc::channel();
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let holding = client.runtime.spawn(async move {
            let _held = connection.session.lock().await;
            locked.send(()).unwrap();
            // A broken deadline still lets the fixture finish and fail.
            let _ = tokio::time::timeout(Duration::from_secs(1), released).await;
        });
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        let connector = client.herdr_api_connector();
        let answer = connector.connection.open_stream_until(
            "/fixture/herdr.sock".to_owned(),
            Instant::now() + Duration::from_millis(20),
        );
        let _ = release.send(());
        client.runtime.block_on(holding).unwrap();
        assert!(
            matches!(answer, Err(ApiError::Transport(reason)) if reason.contains("deadline expired"))
        );
        assert!(
            client
                .runtime
                .block_on(client.connection.session.lock())
                .is_none()
        );
    }

    fn host() -> SshAlias {
        SshAlias::from_config_contents(
            "mini",
            "Host mini\n  HostName mini.example.test\n  User example\n  Port 2200\n  IdentityFile ~/.ssh/id_ed25519\n",
            &fixture_ssh_dir(),
        )
        .unwrap()
    }

    /// Letter-720: past eight session channels on a device's connection, a
    /// use waits for one to close and, once its wait ends, fails with a
    /// record, so the connection never asks sshd for more than it allows.
    #[test]
    fn a_session_channel_past_the_cap_fails_with_a_record() {
        static RECORDS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
        assert!(crate::diagnostics::install(|record| {
            RECORDS.lock().unwrap().push(record);
        }));
        let client = RusshRemoteClient::new(host()).unwrap();
        let wait = Duration::from_millis(50);
        client.runtime.block_on(async {
            let mut held = Vec::new();
            for _ in 0..MAX_SESSION_CHANNELS {
                held.push(
                    client
                        .session_channel_within("remote-read", RemoteStage::Ssh, wait)
                        .await
                        .unwrap(),
                );
            }
            let refused = client
                .session_channel_within("attachment-stage", RemoteStage::Sftp, wait)
                .await
                .unwrap_err();
            assert_eq!(
                refused.diagnostic().reason,
                "the device's SSH connection already has 8 sessions open"
            );
            drop(held.pop());
            held.push(
                client
                    .session_channel_within("attachment-stage", RemoteStage::Sftp, wait)
                    .await
                    .unwrap(),
            );
        });
        // Other tests in the same process may write their own records.
        let records: Vec<_> = RECORDS
            .lock()
            .unwrap()
            .iter()
            .filter(|record| record["kind"] == "connection.sessions_full")
            .cloned()
            .collect();
        assert_eq!(
            records,
            [json!({
                "component": "remote",
                "kind": "connection.sessions_full",
                "target": "ssh:mini",
                "operation": "attachment-stage",
                "limit": 8,
            })]
        );
    }

    /// hided's reaper drops a removed device's last forward on its own runtime.
    #[test]
    fn a_client_released_inside_another_runtime_shuts_down_quietly() {
        let client = RusshRemoteClient::new(host()).unwrap();
        let forward_share = Arc::clone(&client.runtime);
        drop(client);
        Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async move { drop(forward_share) });
    }

    /// Russh starts a socket-owning task after the SSH banner but before it
    /// returns a Handle. Canceling in that interval must still close the peer.
    #[test]
    fn repeated_post_banner_cancellation_closes_each_socket() {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let client = runtime.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let alias = SshAlias {
                host_id: "ssh:kex-stall".to_owned(),
                alias: "kex-stall".to_owned(),
                hostname: "127.0.0.1".to_owned(),
                user: "example".to_owned(),
                port,
                identity_files: Vec::new(),
                identities_only: false,
                agent_socket: AgentSocket::Disabled,
                known_hosts_file: PathBuf::from("/dev/null"),
            };
            let client = Arc::new(RusshRemoteClient::new(alias).unwrap());
            for attempt in 0..6 {
                let connecting = Arc::clone(&client);
                let handler = KnownHostHandler::new(&client.host);
                let task = tokio::spawn(async move { connecting.connect(handler).await });
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(2), listener.accept())
                        .await
                        .expect("client connects to fake SSH peer")
                        .unwrap();
                let mut byte = [0; 1];
                let mut banner_end = false;
                for _ in 0..255 {
                    socket.read_exact(&mut byte).await.unwrap();
                    if byte[0] == b'\n' {
                        banner_end = true;
                        break;
                    }
                }
                assert!(banner_end, "client sends an SSH identification");
                socket.write_all(b"SSH-2.0-kex-stall\r\n").await.unwrap();
                socket.read_exact(&mut byte).await.unwrap();
                task.abort();
                let _ = task.await;
                tokio::time::timeout(Duration::from_secs(2), async {
                    loop {
                        match socket.read(&mut byte).await {
                            Ok(0) => break,
                            Ok(_) => {}
                            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => break,
                            Err(error) => panic!("unexpected peer read failure: {error}"),
                        }
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("cancellation {attempt} left its SSH socket open"));
            }
            client
        });
        drop(client);
    }

    /// A changed host key, an unknown one and a refused sign-in each need a
    /// different action, so the device row names which one it was (B38): the
    /// words come from the real known_hosts check, not a copy of them.
    #[test]
    fn a_changed_or_unknown_host_key_and_a_refused_sign_in_are_told_apart() {
        let offered = russh::keys::PublicKey::from_openssh(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBmTEAgbvSH51RTwhPKbL+uBW92zVlMr81wfUEJlRNkr",
        )
        .unwrap();
        let recorded =
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHDBmiUzbqzzahLz/nn+wP/Sotw5klGvW4QbnvZKoHbG";
        let directory = tempfile::tempdir().unwrap();
        let answer = |known_hosts: &str| {
            let file = directory.path().join("known_hosts");
            std::fs::write(&file, known_hosts).unwrap();
            let mut alias = host();
            alias.known_hosts_file = file;
            let mut handler = KnownHostHandler::new(&alias);
            let key = PublicKeyOrCertificate::PublicKey {
                key: offered.clone(),
                hash_alg: None,
            };
            Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(handler.check_server_key(&key))
                .map_err(|error| error.to_string())
        };

        let changed = answer(&format!("[mini.example.test]:2200 {recorded}\n")).unwrap_err();
        assert_eq!(
            connection_problem(&changed),
            Some("host_key_changed"),
            "{changed}"
        );
        let unknown = answer("").unwrap_err();
        assert_eq!(
            connection_problem(&unknown),
            Some("host_key_unknown"),
            "{unknown}"
        );
        let trusted = answer(&format!(
            "[mini.example.test]:2200 {}\n",
            offered.to_openssh().unwrap()
        ));
        assert_eq!(trusted, Ok(true));

        let refused = remote_error(
            "remote-auth",
            "mini",
            RemoteStage::Auth,
            "the ssh config sets IdentityAgent none and no IdentityFile authenticated",
            false,
            true,
        );
        assert_eq!(
            connection_problem(&refused.to_string()),
            Some("authentication")
        );
        let unreachable = remote_error(
            "remote-connect",
            "mini",
            RemoteStage::Ssh,
            "Connection refused",
            true,
            false,
        );
        assert_eq!(connection_problem(&unreachable.to_string()), None);
    }

    fn status_output(exit_status: u32, stdout: &str) -> RemoteCommandOutput {
        RemoteCommandOutput {
            stdout: stdout.to_owned(),
            stderr: String::new(),
            exit_status,
        }
    }

    /// The shape `herdr status server --json` prints on 0.9.1, with the
    /// socket under the remote user's own home.
    #[test]
    fn herdr_server_status_names_the_socket_the_host_reported() {
        let status = parse_herdr_server_status(
            "mini",
            &status_output(
                0,
                r#"{"status":"running","running":true,"version":"0.9.1","protocol":22,"capabilities":{"live_handoff":true},"compatible":true,"endpoint_compatible":true,"socket":"/Users/example/.config/herdr/herdr.sock","session":null,"restart_needed":false,"server_binary_stale":false}"#,
            ),
        )
        .expect("running server status parses");
        assert_eq!(status.socket, "/Users/example/.config/herdr/herdr.sock");
        assert_eq!(status.version.as_deref(), Some("0.9.1"));
        assert_eq!(status.protocol, Some(22));
    }

    #[test]
    fn a_stopped_server_still_names_the_socket_selected_by_the_remote_environment() {
        let status = parse_herdr_server_status(
            "mini",
            &status_output(0, r#"{"running":false,"socket":"/srv/herdr/work.sock"}"#),
        )
        .expect("a known socket is independent of whether a server runs there");
        assert!(!status.running);
        assert_eq!(status.socket, "/srv/herdr/work.sock");
    }

    #[test]
    fn herdr_server_status_failures_name_what_to_do_on_the_host() {
        let not_installed = parse_herdr_server_status("mini", &status_output(127, ""))
            .expect_err("a missing binary is a failure");
        assert!(not_installed.diagnostic().action_required);
        assert!(!not_installed.diagnostic().retryable);
        assert!(not_installed.to_string().contains("not found on mini"));
        assert!(
            not_installed
                .to_string()
                .contains("Allow and install Hide's helper")
        );

        let unreadable = parse_herdr_server_status("mini", &status_output(0, "usage: herdr"))
            .expect_err("non-JSON output is a failure");
        assert!(unreadable.to_string().contains("not readable"));

        let relative = parse_herdr_server_status(
            "mini",
            &status_output(0, r#"{"running":true,"socket":"herdr.sock"}"#),
        )
        .expect_err("a relative socket is refused");
        assert!(relative.to_string().contains("absolute"));
    }

    #[test]
    fn herdr_server_status_command_runs_without_a_login_shell() {
        let command = RemoteReadCommand::HerdrServerStatus {
            socket: None,
            helper_root: None,
        }
        .command_line()
        .expect("status command");
        assert_eq!(
            command,
            "PATH=\"$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin\" herdr status server --json"
        );
    }

    #[test]
    fn the_status_read_asks_the_pinned_herdr_under_the_helper_root_first() {
        let command = |root: &str| {
            RemoteReadCommand::HerdrServerStatus {
                socket: None,
                helper_root: Some(root.to_owned()),
            }
            .command_line()
            .unwrap()
        };
        assert_eq!(
            command("~/.hide/host-helper"),
            "PATH=\"$HOME/.hide/host-helper/herdr/current:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin\" herdr status server --json"
        );
        assert_eq!(
            command("/srv/hide/helper/"),
            "PATH=\"/srv/hide/helper/herdr/current:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin\" herdr status server --json"
        );
        // A root that would need quoting is not spelled into the shell line.
        for odd in ["~/a\"b", "/tmp/$(x)", "relative", "/tmp/a b"] {
            assert!(
                command(odd).starts_with("PATH=\"$HOME/.local/bin:"),
                "{odd}"
            );
        }
    }

    #[test]
    fn a_registered_herdr_socket_is_asked_for_by_name_and_quoted() {
        let command = RemoteReadCommand::HerdrServerStatus {
            socket: Some("/tmp/hide verify/herdr.sock".to_owned()),
            helper_root: None,
        }
        .command_line()
        .expect("status command");
        assert_eq!(
            command,
            "HERDR_SOCKET_PATH='/tmp/hide verify/herdr.sock' PATH=\"$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin\" herdr status server --json"
        );
        for invalid in ["relative.sock", "/tmp/a\nb.sock", "/"] {
            assert!(
                RemoteReadCommand::HerdrServerStatus {
                    socket: Some(invalid.to_owned()),
                    helper_root: None,
                }
                .command_line()
                .is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn alias_import_ignores_wildcard_and_negated_entries() {
        let aliases = import_ssh_aliases_from_str(
            "Host *\n  User default\nHost mini staging\n  HostName 127.0.0.1\nHost !staging\n  User nobody\nHost *.internal\n  HostName ignored\n",
            &fixture_ssh_dir(),
        )
        .unwrap();
        assert_eq!(
            aliases
                .iter()
                .map(|alias| alias.alias.as_str())
                .collect::<Vec<_>>(),
            ["mini", "staging"]
        );
        assert_eq!(aliases[0].host_id, "ssh:mini");
        assert_eq!(aliases[0].port, 22);
    }

    /// The account's `~/.ssh` in alias fixtures, spelled the way this
    /// machine spells an absolute path (a drive path on Windows).
    fn fixture_ssh_dir() -> PathBuf {
        std::env::temp_dir().join("hide-remote-home").join(".ssh")
    }

    /// Every `IdentityFile` that matches the alias is kept, in config order
    /// across blocks, with `~` read as the account's home; only when none
    /// matches do OpenSSH's defaults stand in, and `IdentityFile none`
    /// leaves no file at all. Paths are kept, never key contents.
    #[test]
    fn alias_import_keeps_every_identity_file_or_openssh_defaults() {
        let ssh_dir = fixture_ssh_dir();
        let home = ssh_dir.parent().unwrap();
        let named = SshAlias::from_config_contents(
            "mini",
            "Host mini\n  HostName mini.example.test\n  User example\n  IdentityFile ~/.ssh/work\n  IdentityFile=\"%d/keys/second\"\n\nHost *\n  IdentityFile ~/.ssh/everywhere\n  IdentitiesOnly yes\n",
            &ssh_dir,
        )
        .unwrap();
        assert_eq!(
            named
                .identity_files
                .iter()
                .map(|file| (file.path.clone(), file.shown.as_str(), file.default))
                .collect::<Vec<_>>(),
            [
                (home.join(".ssh/work"), "~/.ssh/work", false),
                (home.join("keys/second"), "%d/keys/second", false),
                (home.join(".ssh/everywhere"), "~/.ssh/everywhere", false),
            ]
        );
        assert!(named.identities_only);
        assert_eq!(named.known_hosts_file, ssh_dir.join("known_hosts"));
        let encoded = serde_json::to_string(&named).unwrap();
        assert!(!encoded.contains("PRIVATE KEY"));

        let unnamed = SshAlias::from_config_contents(
            "mini",
            "Host mini\n  HostName mini.example.test\n  User example\n",
            &ssh_dir,
        )
        .unwrap();
        assert_eq!(
            unnamed
                .identity_files
                .iter()
                .map(|file| (file.path.clone(), file.shown.as_str(), file.default))
                .collect::<Vec<_>>(),
            [
                (ssh_dir.join("id_rsa"), "~/.ssh/id_rsa", true),
                (ssh_dir.join("id_ecdsa"), "~/.ssh/id_ecdsa", true),
                (ssh_dir.join("id_ecdsa_sk"), "~/.ssh/id_ecdsa_sk", true),
                (ssh_dir.join("id_ed25519"), "~/.ssh/id_ed25519", true),
                (ssh_dir.join("id_ed25519_sk"), "~/.ssh/id_ed25519_sk", true),
            ]
        );
        assert!(!unnamed.identities_only);

        let none = SshAlias::from_config_contents(
            "mini",
            "Host mini\n  HostName mini.example.test\n  User example\n  IdentityFile none\n",
            &ssh_dir,
        )
        .unwrap();
        assert!(none.identity_files.is_empty());
    }

    /// A key file name Hide would have to guess at is refused when the alias
    /// is read, rather than becoming a key that silently never loads.
    #[test]
    fn an_identity_file_hide_cannot_expand_fails_visibly() {
        for (line, reason) in [
            ("IdentityFile ~/.ssh/%h", "percent token"),
            ("IdentityFile ${KEYS}/id", "environment variable"),
            ("IdentityFile keys/id", "absolute path"),
            ("IdentitiesOnly maybe", "IdentitiesOnly must be yes or no"),
        ] {
            let error = SshAlias::from_config_contents(
                "mini",
                &format!("Host mini\n  HostName mini.example.test\n  User example\n  {line}\n"),
                &fixture_ssh_dir(),
            )
            .unwrap_err();
            assert_eq!(error.stage(), RemoteStage::Alias);
            assert!(error.to_string().contains(reason), "{line}: {error}");
        }
    }

    /// An agent socket is a path on this machine, so the fixtures spell it
    /// the way this machine does (a drive path on Windows).
    fn local_agent(name: &str) -> PathBuf {
        std::env::temp_dir().join("hide-remote-home").join(name)
    }

    #[test]
    fn wildcard_identity_agent_reaches_the_alias_it_covers() {
        let agent = local_agent("agent.sock");
        let alias = SshAlias::from_config_contents(
            "mini",
            &format!(
                "Host *\n  IdentityAgent {}\n\nHost mini\n  HostName mini.example.test\n  User example\n",
                agent.display()
            ),
            &fixture_ssh_dir(),
        )
        .unwrap();
        assert_eq!(alias.agent_socket, AgentSocket::Path(agent));
    }

    #[test]
    fn the_first_matching_identity_agent_wins() {
        let first = local_agent("first.sock");
        let alias = SshAlias::from_config_contents(
            "mini",
            &format!(
                "Host mini\n  HostName mini.example.test\n  User example\n  IdentityAgent {}\n\nHost *\n  IdentityAgent {}\n",
                first.display(),
                local_agent("second.sock").display()
            ),
            &fixture_ssh_dir(),
        )
        .unwrap();
        assert_eq!(alias.agent_socket, AgentSocket::Path(first));
    }

    #[test]
    fn an_identity_agent_for_another_host_does_not_reach_this_alias() {
        let alias = SshAlias::from_config_contents(
            "mini",
            "Host github.com\n  IdentityAgent /private/tmp/hide-remote-home/github.sock\n\nHost mini\n  HostName mini.example.test\n  User example\n",
            &fixture_ssh_dir(),
        )
        .unwrap();
        assert_eq!(alias.agent_socket, AgentSocket::Environment);
    }

    #[test]
    fn identity_agent_none_and_the_environment_spelling_are_distinct() {
        let disabled = SshAlias::from_config_contents(
            "mini",
            "Host mini\n  HostName mini.example.test\n  User example\n  IdentityAgent none\n",
            &fixture_ssh_dir(),
        )
        .unwrap();
        assert_eq!(disabled.agent_socket, AgentSocket::Disabled);
        let environment = SshAlias::from_config_contents(
            "mini",
            "Host mini\n  HostName mini.example.test\n  User example\n  IdentityAgent SSH_AUTH_SOCK\n",
            &fixture_ssh_dir(),
        )
        .unwrap();
        assert_eq!(environment.agent_socket, AgentSocket::Environment);
    }

    #[test]
    fn an_identity_agent_this_shell_cannot_resolve_fails_visibly() {
        let error = SshAlias::from_config_contents(
            "mini",
            "Host mini\n  HostName mini.example.test\n  User example\n  IdentityAgent $HIDE_AGENT_SOCK\n",
            &fixture_ssh_dir(),
        )
        .unwrap_err();
        assert_eq!(error.stage(), RemoteStage::Alias);
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("action_required=true"), "{diagnostic}");
        assert!(
            diagnostic.contains("outside the declared remote environment contract"),
            "{diagnostic}"
        );
    }

    #[test]
    fn a_negated_host_pattern_keeps_its_identity_agent_away() {
        let alias = SshAlias::from_config_contents(
            "mini",
            "Host * !mini\n  IdentityAgent /private/tmp/hide-remote-home/agent.sock\n\nHost mini\n  HostName mini.example.test\n  User example\n",
            &fixture_ssh_dir(),
        )
        .unwrap();
        assert_eq!(alias.agent_socket, AgentSocket::Environment);
    }

    #[test]
    fn read_only_git_command_quotes_absolute_root() {
        let command = RemoteReadCommand::GitStatus {
            root: "/tmp/a b".to_owned(),
        }
        .command_line()
        .unwrap();
        assert_eq!(command, "git -C '/tmp/a b' status --short --porcelain=v1");
        assert!(
            RemoteReadCommand::GitStatus {
                root: "relative".to_owned()
            }
            .command_line()
            .is_err()
        );
        assert!(
            RemoteReadCommand::GitStatus {
                root: "/tmp/bad\0path".to_owned()
            }
            .command_line()
            .is_err()
        );
    }
}

#[cfg(test)]
mod sign_in_tests;
