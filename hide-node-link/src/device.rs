//! What the core and a node share about a registered SSH device: the stage
//! a device operation failed at, its diagnostic, and the capability report
//! of a connection test (PRD core-host-node D-21). The SSH transport that
//! produces them is the node's (`hide_node::ssh`); the core only reads them.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The prefixes a host-key refusal starts with, so a caller can tell a
/// changed key from a missing one without parsing the rest of the sentence.
pub const HOST_KEY_CHANGED: &str = "host key changed";
pub const HOST_KEY_UNKNOWN: &str = "host key unknown";

/// Herdr's named-session grammar, before a route is stored or dispatched.
pub fn valid_herdr_session(session: &str) -> bool {
    hide_platform::host::valid_herdr_session(session)
}

/// Whether `alias` is one SSH config token Hide may hand to `ssh`: never
/// empty, an option, a path or more than one word.
pub fn ssh_alias_is_one_token(alias: &str) -> bool {
    !(alias.is_empty()
        || alias.chars().any(char::is_whitespace)
        || alias.bytes().any(|byte| byte.is_ascii_control())
        || alias.contains('/')
        || alias.starts_with('-'))
}

/// Which trust or sign-in step a connection failure names, read off the
/// words this module wrote into it: a changed host key, an unknown one, or a
/// refused authentication (PRD S5.5 B38). Any other failure is `None`.
pub fn connection_problem(message: &str) -> Option<&'static str> {
    if message.contains(HOST_KEY_CHANGED) {
        Some("host_key_changed")
    } else if message.contains(HOST_KEY_UNKNOWN) {
        Some("host_key_unknown")
    } else if message.contains("stage=auth ") {
        Some("authentication")
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteStage {
    Alias,
    Ssh,
    Auth,
    Herdr,
    Protocol,
    Pty,
    Sftp,
    Git,
    Tunnel,
    Reconnect,
    Cleanup,
}

impl fmt::Display for RemoteStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Alias => "alias",
            Self::Ssh => "ssh",
            Self::Auth => "auth",
            Self::Herdr => "herdr",
            Self::Protocol => "protocol",
            Self::Pty => "pty",
            Self::Sftp => "sftp",
            Self::Git => "git",
            Self::Tunnel => "tunnel",
            Self::Reconnect => "reconnect",
            Self::Cleanup => "cleanup",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteDiagnostic {
    pub operation_id: String,
    pub target: String,
    pub stage: RemoteStage,
    pub retryable: bool,
    pub action_required: bool,
    pub reason: String,
}

impl fmt::Display for RemoteDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "stage={} target={} operation={} retryable={} action_required={} cause={}",
            self.stage,
            self.target,
            self.operation_id,
            self.retryable,
            self.action_required,
            self.reason
        )
    }
}

/// The operation of a dial that never reached the machine's SSH server: the
/// connection was refused, reset or not answered before the server presented
/// its host key. Every later step (the host key, the sign-in, a channel) has
/// its own.
pub const DIAL_OPERATION: &str = "remote-dial";

/// The operation of a dial whose server presented a host key known_hosts
/// refused (changed, unknown, or unreadable): the operator has to act.
pub const HOST_KEY_OPERATION: &str = "remote-host-key";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteError {
    diagnostic: RemoteDiagnostic,
}

impl RemoteError {
    pub fn new(
        operation_id: impl Into<String>,
        target: impl Into<String>,
        stage: RemoteStage,
        reason: impl Into<String>,
        retryable: bool,
        action_required: bool,
    ) -> Self {
        Self {
            diagnostic: RemoteDiagnostic {
                operation_id: operation_id.into(),
                target: target.into(),
                stage,
                retryable,
                action_required,
                reason: reason.into(),
            },
        }
    }

    pub fn diagnostic(&self) -> &RemoteDiagnostic {
        &self.diagnostic
    }

    pub fn stage(&self) -> RemoteStage {
        self.diagnostic.stage
    }

    /// Whether the connection never reached the machine's SSH server, so a
    /// move of this machine or of the network may change the answer.
    pub fn never_reached_server(&self) -> bool {
        self.diagnostic.operation_id == DIAL_OPERATION
    }

    /// Whether a move of this machine or of the network may change the
    /// answer: a dial that never reached the server, or a failure on the way
    /// that may be retried and asks nothing of the operator (a reset or a
    /// timeout in the key exchange, a channel that failed or would not
    /// open). A host key, a sign-in or an alias the operator must fix answers
    /// the same from any network.
    pub fn a_move_can_change(&self) -> bool {
        self.never_reached_server()
            || (self.diagnostic.retryable && !self.diagnostic.action_required)
    }
}

impl fmt::Display for RemoteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.diagnostic.fmt(formatter)
    }
}

impl std::error::Error for RemoteError {}

pub type RemoteResult<T> = Result<T, RemoteError>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteHostIdentity {
    pub host_id: String,
    pub alias: String,
    pub hostname: String,
    pub port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RemoteConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting { attempt: u32 },
    Stale { reason: String },
    Failed { reason: String },
    ActionRequired { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CapabilityState {
    Pending,
    Passed,
    Failed {
        retryable: bool,
        action_required: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapabilityResult {
    pub stage: RemoteStage,
    pub state: CapabilityState,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapabilityReport {
    pub operation_id: String,
    pub host: RemoteHostIdentity,
    pub connection: RemoteConnectionState,
    pub stages: Vec<CapabilityResult>,
}

impl CapabilityReport {
    pub fn new(operation_id: impl Into<String>, host: RemoteHostIdentity) -> Self {
        let stages = [
            RemoteStage::Ssh,
            RemoteStage::Auth,
            RemoteStage::Herdr,
            RemoteStage::Protocol,
            RemoteStage::Pty,
            RemoteStage::Sftp,
            RemoteStage::Git,
            RemoteStage::Tunnel,
        ]
        .into_iter()
        .map(|stage| CapabilityResult {
            stage,
            state: CapabilityState::Pending,
            detail: String::new(),
        })
        .collect();
        Self {
            operation_id: operation_id.into(),
            host,
            connection: RemoteConnectionState::Connecting,
            stages,
        }
    }

    pub fn stage(&self, stage: RemoteStage) -> Option<&CapabilityResult> {
        self.stages.iter().find(|result| result.stage == stage)
    }

    pub fn pass(&mut self, stage: RemoteStage, detail: impl Into<String>) {
        if let Some(result) = self.stages.iter_mut().find(|result| result.stage == stage) {
            result.state = CapabilityState::Passed;
            result.detail = detail.into();
        } else {
            self.stages.push(CapabilityResult {
                stage,
                state: CapabilityState::Passed,
                detail: detail.into(),
            });
        }
    }

    pub fn fail(
        &mut self,
        stage: RemoteStage,
        detail: impl Into<String>,
        retryable: bool,
        action_required: bool,
    ) {
        if let Some(result) = self.stages.iter_mut().find(|result| result.stage == stage) {
            result.state = CapabilityState::Failed {
                retryable,
                action_required,
            };
            result.detail = detail.into();
        } else {
            self.stages.push(CapabilityResult {
                stage,
                state: CapabilityState::Failed {
                    retryable,
                    action_required,
                },
                detail: detail.into(),
            });
        }
        self.connection = if action_required {
            RemoteConnectionState::ActionRequired {
                reason: format!("{stage} capability requires an explicit user action"),
            }
        } else {
            RemoteConnectionState::Failed {
                reason: format!("{stage} capability failed"),
            }
        };
    }

    pub fn connected(&mut self) {
        self.connection = RemoteConnectionState::Connected;
    }

    pub fn all_required_passed(&self) -> bool {
        self.stages
            .iter()
            .all(|result| matches!(result.state, CapabilityState::Passed))
    }
}

/// A socket path a device registration may name: absolute, one line.
pub fn valid_remote_socket_path(path: &str) -> bool {
    path.starts_with('/') && path.len() > 1 && !path.bytes().any(|byte| byte.is_ascii_control())
}

/// What the operator allowed on a device: install and update Hide's node
/// and its `hide` command under `helper_root`, link `hide` in `cli_dir` when
/// that name is free or already Hide's, run the node only for the life of an
/// SSH connection, and perform file and Git work inside registered
/// checkouts, with trash moves and worktree removals still confirmed one by
/// one. `contract` names that scope; a build whose scope differs asks again,
/// and so does a device that answers with another identity than the one the
/// consent was first used on.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostConsent {
    pub contract: u32,
    pub helper_root: String,
    /// Absent in a consent given before contract 2, which never covered it.
    #[serde(default)]
    pub cli_dir: Option<String>,
    pub granted_at_unix_ms: u64,
    /// The account, address and host key the node first ran on; bound on
    /// the first connection after consent and never rewritten by one.
    #[serde(default)]
    pub identity: Option<HostIdentity>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostIdentity {
    pub user: String,
    pub hostname: String,
    pub port: u16,
    pub host_key_sha256: String,
}

impl HostIdentity {
    pub fn describe(&self) -> String {
        format!(
            "{}@{}:{} ({})",
            self.user, self.hostname, self.port, self.host_key_sha256
        )
    }
}

/// The scope the operator agrees to, versioned. A build that needs more than
/// this contract describes bumps it, and every device asks again (B51).
/// Contract 2 added the `hide` command installed beside the helper and its
/// link in the consented command folder. Contract 3 is the whole install kit
/// (PRD device-parity D-12): the hook entries besides the command
/// (it also held the labels plugin, which labels-in-hided retired, so the
/// scope only narrowed); a contract-2 consent with the same folders is
/// carried to 3 on its next connection without asking (D-13).
pub const HOST_CONSENT_CONTRACT: u32 = 3;

/// The contract a consent may be carried forward from without asking.
pub const HOST_CONSENT_CARRIED_FROM: u32 = 2;

/// Where the node is installed on the device unless the daemon was started
/// with another root; `~` is the remote account's home.
pub const DEFAULT_HELPER_ROOT: &str = hide_kit::layout::HELPER_ROOT;

/// Where the device's `hide` command is linked unless the daemon was started
/// with another folder; `~` is the remote account's home.
pub const DEFAULT_CLI_DIR: &str = "~/.local/bin";

#[derive(Debug)]
pub enum EstablishError {
    Connect(RemoteError),
    /// The device answering the alias is not the one consent was given for.
    IdentityChanged {
        bound: Box<HostIdentity>,
        observed: Box<HostIdentity>,
    },
    /// This build cannot serve the device; nothing was installed.
    Unsupported(String),
    /// Acquiring the release kit for a known foreign platform failed.
    Download {
        os: String,
        arch: String,
        reason: String,
    },
    Install(String),
    Helper(String),
}

impl fmt::Display for EstablishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(error) => write!(formatter, "{}", error.diagnostic().reason),
            Self::IdentityChanged { bound, observed } => write!(
                formatter,
                "The device now answers as {}, not {} that the helper was allowed on; allow it again in Settings to continue",
                observed.describe(),
                bound.describe()
            ),
            Self::Unsupported(reason) | Self::Install(reason) | Self::Helper(reason) => {
                formatter.write_str(reason)
            }
            Self::Download { reason, .. } => formatter.write_str(reason),
        }
    }
}

/// A device's node answering on its link, however the link opened.
pub struct NodeReady {
    pub host: std::sync::Arc<dyn crate::NodeLink>,
    pub hello: crate::protocol::Hello,
    /// The device's terminals inside this link, when its node started its
    /// terminal service, or why it has none (its node speaks an older
    /// protocol, or its Herdr could not be found).
    pub terminals: Result<std::sync::Arc<dyn crate::terminal::TerminalNode>, String>,
}

/// A device the core dialed, its node started and answering: the node, the
/// account and host key the dial reached, and what the install did.
pub struct Established {
    pub node: NodeReady,
    pub identity: HostIdentity,
    /// The node was installed or replaced on this connection.
    pub installed: bool,
    pub helper_path: String,
    /// How the build's files reached the device on this connection.
    pub upload: Upload,
    /// The automatic kit pass completed before starting Herdr-dependent
    /// services when Herdr was unreadable. A normal connection leaves it
    /// to the core after helper readiness.
    pub kit: Option<Result<Box<hide_kit::KitReport>, String>>,
}

/// Hears why a link ended, once.
pub type OnClose = Box<dyn FnOnce(String) + Send + 'static>;

/// A node that dialed the core, as the core takes it (PRD
/// core-host-node-remote-core D-04, D-10): its link is up and its node
/// answering, so the core never dials it, installs nothing on it and asks
/// no consent of it.
pub struct Arrived {
    pub transport: std::sync::Arc<dyn DeviceTransport>,
    pub node: NodeReady,
    /// Takes the handler the link's end is told to; told at once when the
    /// link already ended.
    pub hear_close: Box<dyn FnOnce(OnClose) + Send + 'static>,
}

/// What one connection's install did with the build's files: how many it
/// sent, how many were already there, and the kit parts left out with why.
/// The kit on the device reports a left-out part on the device's row; this
/// is the diagnostic detail.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Upload {
    pub sent: usize,
    pub reused: usize,
    pub missing: Vec<String>,
}

/// Where an alias leads, as `ssh -G` resolved it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SshAddress {
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl SshAddress {
    /// `user@host:port`, the form the dialog shows and compares.
    pub fn display(&self) -> String {
        if self.host.contains(':') {
            format!("{}@[{}]:{}", self.user, self.host, self.port)
        } else {
            format!("{}@{}:{}", self.user, self.host, self.port)
        }
    }
}

/// Why an alias has no address; the dialog shows a short line per code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SshHostProblem {
    /// The `ssh` program was not found.
    SshMissing,
    /// `ssh -G` ended without a usable answer (a bad option in the config).
    SshFailed,
    /// `ssh -G` did not answer within the resolve deadline.
    TimedOut,
}

impl SshHostProblem {
    pub fn code(self) -> &'static str {
        match self {
            Self::SshMissing => "ssh_missing",
            Self::SshFailed => "ssh_failed",
            Self::TimedOut => "timed_out",
        }
    }
}

/// One listed alias and where it leads.
#[derive(Debug, Eq, PartialEq)]
pub struct SshHostEntry {
    pub alias: String,
    pub address: Result<SshAddress, SshHostProblem>,
}

/// What one listing found: the config's concrete aliases with their
/// addresses, and the address of every registered alias, whether or not the
/// config still names it, so two aliases of one machine can be told apart
/// from two machines.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct SshHostListing {
    pub entries: Vec<SshHostEntry>,
    pub registered: Vec<(String, SshAddress)>,
    pub truncated: bool,
}

/// Saved client configuration, never evidence of a reachable server.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HerdrProfile {
    pub id: String,
    pub label: String,
    pub target: String,
    pub session: String,
    pub enabled: bool,
}

#[derive(Debug, Default)]
pub struct HerdrProfileListing {
    pub profiles: Vec<HerdrProfile>,
    /// Config-only resolution for profile and already registered targets.
    pub endpoints: std::collections::BTreeMap<String, Result<String, String>>,
    pub truncated: bool,
}

/// Opens the transport to a registered device. The node that holds the SSH
/// configuration and keys implements it (`hide_node::ssh`); the core asks it
/// by alias and never reads `~/.ssh` itself (PRD core-host-node D-21).
pub trait DeviceConnector: Send + Sync {
    /// The transport for the device `node` behind `alias`, resolved through
    /// the SSH configuration of the account at `home`; nothing connects until
    /// it is used. A refusal is the sentence the device row shows.
    fn transport(
        &self,
        home: &std::path::Path,
        node: &str,
        alias: &str,
        herdr_socket: Option<String>,
    ) -> Result<std::sync::Arc<dyn DialedTransport>, String>;

    /// An explicit session must never silently use the default server.
    fn transport_session(
        &self,
        home: &std::path::Path,
        node: &str,
        target: &str,
        herdr_socket: Option<String>,
        session: Option<String>,
    ) -> Result<std::sync::Arc<dyn DialedTransport>, String> {
        if session.is_some() {
            return Err("This connector cannot select a saved Herdr session".to_owned());
        }
        self.transport(home, node, target, herdr_socket)
    }

    /// Reads local client profiles only, without connecting to any target.
    fn herdr_profiles(
        &self,
        _home: &std::path::Path,
        _registered: &[String],
        _stop: &std::sync::atomic::AtomicBool,
    ) -> Result<HerdrProfileListing, String> {
        Err(
            "Saved Herdr machines are unavailable on this node. Retry after reconnecting"
                .to_owned(),
        )
    }

    /// The concrete Host aliases of the SSH configuration of the account at
    /// `home` and where each leads, with the address of each of
    /// `registered`, for Add device; it never connects. Blocking and
    /// bounded; raising `stop` ends it early.
    fn ssh_hosts(
        &self,
        home: &std::path::Path,
        registered: &[String],
        stop: &std::sync::atomic::AtomicBool,
    ) -> SshHostListing;
}

/// Decodes the Herdr snapshot a connection test fetched, given the device's
/// host id and the socket it answered on, and answers the protocol revision
/// it reports. The decode is the core's, because the snapshot's shape is its
/// Herdr contract; the fetch is the transport's.
pub type SnapshotCheck<'a> =
    &'a (dyn Fn(&serde_json::Value, &str, &str) -> RemoteResult<u32> + Sync);

/// One registered device as the core reaches it, whichever way its link
/// opened: its Herdr API over that link.
pub trait DeviceTransport: Send + Sync {
    /// The device's Herdr API, over this transport.
    fn herdr_api_connector(&self) -> std::sync::Arc<dyn hide_herdr_client::ApiConnector>;
    /// The Herdr version the device reported last, if it has.
    fn cached_herdr_version(&self) -> Option<String>;
    /// The exact socket already resolved for this transport, without I/O.
    fn cached_herdr_socket(&self) -> Option<String> {
        None
    }
    /// The concrete transport, for the shell that built it and reaches parts
    /// the core does not use (browser and return-route forwards).
    fn into_any(self: std::sync::Arc<Self>) -> std::sync::Arc<dyn std::any::Any + Send + Sync>;
}

/// A device the core dials over SSH: it tests the connection, and installs
/// and starts the device's node. What the core then does on the device goes
/// through that node's link.
pub trait DialedTransport: DeviceTransport {
    /// Probes each stage of a connection for the operator's test; `check`
    /// decodes the Herdr stage's snapshot.
    fn capability_test(&self, operation_id: &str, check: SnapshotCheck<'_>) -> CapabilityReport;
    /// Connects, checks `consent` against the device that answered, installs
    /// this build's node when the device lacks it, and starts it. Blocking;
    /// run it off the runtime lock. `on_close` hears why the link ended.
    fn establish(
        &self,
        consent: &HostConsent,
        retirement_projects: &[String],
        on_close: OnClose,
    ) -> Result<Established, EstablishError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> RemoteHostIdentity {
        RemoteHostIdentity {
            host_id: "ssh:mini".to_owned(),
            alias: "mini".to_owned(),
            hostname: "mini.example.test".to_owned(),
            port: 2200,
        }
    }

    #[test]
    fn capability_report_has_staged_failure_and_visible_recovery() {
        let mut report = CapabilityReport::new("op-1", identity());
        report.fail(RemoteStage::Tunnel, "forward denied", true, true);
        assert!(matches!(
            report.connection,
            RemoteConnectionState::ActionRequired { .. }
        ));
        assert!(matches!(
            report.stage(RemoteStage::Tunnel).unwrap().state,
            CapabilityState::Failed {
                retryable: true,
                action_required: true
            }
        ));
        report.pass(RemoteStage::Tunnel, "retried after approval");
        report.connected();
        assert_eq!(report.connection, RemoteConnectionState::Connected);
    }

    #[test]
    fn capability_report_requires_every_stage_before_connected() {
        let mut report = CapabilityReport::new("op-1", identity());
        for stage in [
            RemoteStage::Ssh,
            RemoteStage::Auth,
            RemoteStage::Herdr,
            RemoteStage::Protocol,
            RemoteStage::Pty,
            RemoteStage::Sftp,
            RemoteStage::Git,
        ] {
            report.pass(stage, "ok");
        }
        assert!(!report.all_required_passed());
        report.pass(RemoteStage::Tunnel, "ok");
        assert!(report.all_required_passed());
    }
}
