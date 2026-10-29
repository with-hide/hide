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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteStage {
    Alias,
    Ssh,
    Auth,
    Herdr,
    Protocol,
    Pty,
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
    /// A report with a device test's stages pending, in the order the row
    /// lists them: the connection, the sign-in, the helper's protocol, Herdr,
    /// the terminals and Git (PRD system-ssh-device-link B25). Each is read
    /// off the connection the device already has; the test dials nothing.
    pub fn new(operation_id: impl Into<String>, host: RemoteHostIdentity) -> Self {
        let stages = [
            RemoteStage::Ssh,
            RemoteStage::Auth,
            RemoteStage::Protocol,
            RemoteStage::Herdr,
            RemoteStage::Pty,
            RemoteStage::Git,
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
    /// The account, address, port and machine the node first ran on;
    /// bound on the first connection after consent and never rewritten by
    /// one, except that a consent bound before contract 4 gains the machine
    /// id it lacked.
    #[serde(default)]
    pub identity: Option<HostIdentity>,
}

/// Who a consent was given for: the account and address `ssh` reaches, and
/// the machine that answered there (PRD system-ssh-device-link D-09). The
/// host key is OpenSSH's to check against `known_hosts`, so a consent is no
/// longer bound to it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostIdentity {
    pub user: String,
    pub hostname: String,
    pub port: u16,
    /// The id the machine reports of itself (`hide_platform::host::machine_id`
    /// read on the device; a WSL distribution's install id as `wsl:<id>`).
    /// Absent in a consent bound before contract 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<String>,
    /// The host key a consent before contract 4 was bound to. Kept as it
    /// was, so a build before contract 4 still recognizes the device; a
    /// consent given since leaves it empty.
    #[serde(default)]
    pub host_key_sha256: String,
}

impl HostIdentity {
    pub fn describe(&self) -> String {
        match &self.machine_id {
            Some(machine) => format!(
                "{}@{}:{} (machine {machine})",
                self.user, self.hostname, self.port
            ),
            None => format!("{}@{}:{}", self.user, self.hostname, self.port),
        }
    }

    /// Whether `observed` is the device a consent bound to `self` covers: the
    /// same account, address and port, and the same machine (B16, B17). A
    /// consent from before contract 4 names no machine, only the host key it
    /// was given for, and is carried to the machine that answers (D-09). A
    /// record that names neither is bound to nothing and covers no device:
    /// it never stands for any machine at that address.
    pub fn covers(&self, observed: &HostIdentity) -> bool {
        let machine = match &self.machine_id {
            Some(machine) => observed.machine_id.as_ref() == Some(machine),
            None => !self.host_key_sha256.is_empty() && observed.machine_id.is_some(),
        };
        self.user == observed.user
            && self.hostname == observed.hostname
            && self.port == observed.port
            && machine
    }

    /// Whether `self` and `other` name the same account on the same machine:
    /// the machine ids when both have one, else the host key a consent from
    /// before contract 4 kept when both have one, else the same address. Two
    /// registrations for one account share its install, so a doubt reads as
    /// the same: removing one then leaves the other's kit in place.
    pub fn same_account(&self, other: &HostIdentity) -> bool {
        if self.user != other.user {
            return false;
        }
        match (&self.machine_id, &other.machine_id) {
            (Some(mine), Some(theirs)) => mine == theirs,
            _ if !self.host_key_sha256.is_empty() && !other.host_key_sha256.is_empty() => {
                self.host_key_sha256 == other.host_key_sha256
            }
            _ => self.hostname == other.hostname && self.port == other.port,
        }
    }
}

/// The scope the operator agrees to, versioned. A build that needs more than
/// this contract describes bumps it, and every device asks again (B51).
/// Contract 2 added the `hide` command installed beside the helper and its
/// link in the consented command folder. Contract 3 is the whole install kit
/// (PRD device-parity D-12): the hook entries besides the command
/// (it also held the labels plugin, which labels-in-hided retired, so the
/// scope only narrowed). Contract 4 binds the consent to the machine id the
/// device reports instead of its host key (PRD system-ssh-device-link D-09);
/// the scope is unchanged, so a contract-2 or -3 consent with the same
/// folders is carried to 4 on its next connection without asking (D-13).
pub const HOST_CONSENT_CONTRACT: u32 = 4;

/// The contracts a consent may be carried forward from without asking.
pub const HOST_CONSENT_CARRIED_FROM: [u32; 2] = [2, 3];

/// Where the node is installed on the device unless the daemon was started
/// with another root; `~` is the remote account's home.
pub const DEFAULT_HELPER_ROOT: &str = hide_kit::layout::HELPER_ROOT;

/// Where the device's `hide` command is linked unless the daemon was started
/// with another folder; `~` is the remote account's home.
pub const DEFAULT_CLI_DIR: &str = "~/.local/bin";

/// Whether a connection attempt has the operator behind it: it may start a
/// machine that is off, a WSL distribution that is not running (PRD
/// wsl-device D-11), and it may ask the operator on the device row (PRD
/// system-ssh-device-link D-06). Only the app's launch and the operator's
/// own `다시 연결` (or consent, or a fresh registration) may; a scheduled
/// retry, a read that needs the link and every background job may not. Such
/// an attempt on a machine that is off ends as [`EstablishError::Stopped`],
/// and one that needs a question ends as [`SshProblem::SignInNeeded`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Wake {
    Allowed,
    NotAllowed,
}

/// What a connection attempt waits on, as the device's row shows it (PRD
/// system-ssh-device-link B4-B12). The words are ssh's own; a secret
/// the operator types is never part of it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignInWait {
    /// ssh asks for something to type that is shown masked: a password, a
    /// key's passphrase (with the key's file, for the `ssh-add` line), a
    /// one-time code or a security key's PIN. `prompt` is ssh's sentence.
    Secret {
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key_file: Option<String>,
    },
    /// The server is not in `known_hosts`: its name and key fingerprint, for
    /// the operator to trust or not.
    HostKey { host: String, fingerprint: String },
    /// ssh asks yes or no.
    Confirm { prompt: String },
    /// A security key waits for a touch; it ends by itself.
    Touch { prompt: String },
    /// The sign-in waits for a page in the browser.
    Browser { url: String },
}

impl SignInWait {
    /// Whether the operator answers it on the row, as opposed to a wait that
    /// ends by itself.
    pub fn is_question(&self) -> bool {
        matches!(
            self,
            Self::Secret { .. } | Self::HostKey { .. } | Self::Confirm { .. }
        )
    }

    /// The `kind` the row and `hide device` read.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Secret { .. } => "secret",
            Self::HostKey { .. } => "host_key",
            Self::Confirm { .. } => "confirm",
            Self::Touch { .. } => "touch",
            Self::Browser { .. } => "browser",
        }
    }
}

/// Text the operator typed for ssh. It is handed to ssh and dropped: it has
/// no serialization, and its `Debug` shows nothing of it (B10).
pub struct Secret(String);

impl Secret {
    pub fn new(text: String) -> Self {
        Self(text)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(..)")
    }
}

/// The operator's answer to a question.
#[derive(Debug)]
pub enum Answer {
    /// What they typed for a secret.
    Text(Secret),
    /// They trusted the server or said yes.
    Accept,
}

/// Why a question got no answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The attempt is Hide's own, so nobody may be asked (D-06, B14).
    NotAllowed,
    /// The operator chose Cancel (B8).
    Cancelled,
    /// Nobody answered within the bound (D-05: five minutes).
    TimedOut,
    /// The attempt the question belonged to is over.
    Ended,
}

/// The device row's side of one connection attempt, which the core gives
/// the transport (PRD system-ssh-device-link D-05, D-06, D-10). Blocking;
/// called from the attempt's own threads, never under the runtime lock.
pub trait SignIn: Send + Sync {
    /// Shows a wait that takes no answer on the row, or clears it.
    fn show(&self, wait: Option<SignInWait>);
    /// Asks `question` on the row and waits for the operator, at most the
    /// core's bound; an attempt Hide started by itself is answered
    /// [`Refusal::NotAllowed`] at once.
    fn ask(&self, question: SignInWait) -> Result<Answer, Refusal>;
    /// The attempt's sign-in is over, signed in or not: whatever it still
    /// shows or asks goes, and a question still waiting is answered
    /// [`Refusal::Ended`]. A caller whose own state ends with the attempt,
    /// as the device row's does, has nothing to do.
    fn ended(&self) {}
    /// The screen that started the attempt went away or cancelled a wait
    /// that takes no answer (a browser check, a security key's touch): the
    /// attempt ends as if its question were cancelled (B8). The device row,
    /// which stays while its attempt runs, never withdraws.
    fn withdrawn(&self) -> bool {
        false
    }
}

/// What an attempt waits on as a screen shows it: the device row
/// (`DeviceSnapshot.sign_in`), a node window's strip (`core_link`) and the
/// core move dialog (`core_move`) read the same shape (PRD
/// system-ssh-device-link B6-B12, B33).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShownSignIn {
    /// The number an answer names, so an answer typed for a question ssh no
    /// longer asks never answers the next one (B7).
    pub question: u64,
    #[serde(flatten)]
    pub wait: SignInWait,
    /// For a key's passphrase: the command that adds the key to the agent
    /// of the machine ssh runs on, so it is typed once (B9).
    #[serde(default)]
    pub key_command: Option<String>,
}

impl ShownSignIn {
    pub fn new(question: u64, wait: SignInWait) -> Self {
        let key_command = match &wait {
            SignInWait::Secret {
                key_file: Some(file),
                ..
            } => Some(add_key_command(file)),
            _ => None,
        };
        Self {
            question,
            wait,
            key_command,
        }
    }
}

/// The command that adds a key to the agent of the machine ssh runs on, so
/// its passphrase is typed once rather than on every connection (D-07,
/// B9).
pub fn add_key_command(file: &str) -> String {
    let quoted = if file
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "/._-~+@:".contains(character))
    {
        file.to_owned()
    } else {
        format!("'{}'", file.replace('\'', r"'\''"))
    };
    if cfg!(target_os = "macos") {
        format!("ssh-add --apple-use-keychain {quoted}")
    } else {
        format!("ssh-add {quoted}")
    }
}

/// The operator's reply to a shown question, from whichever screen shows
/// it. Its `Debug` shows nothing typed (B10).
#[derive(Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum SignInReply {
    Text { text: String },
    Accept,
    Cancel,
}

impl SignInReply {
    /// The reply's kind, for a log line: never what was typed.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Accept => "accept",
            Self::Cancel => "cancel",
        }
    }

    /// The answer the waiting question takes.
    pub fn into_answer(self) -> Result<Answer, Refusal> {
        match self {
            Self::Text { text } => Ok(Answer::Text(Secret::new(text))),
            Self::Accept => Ok(Answer::Accept),
            Self::Cancel => Err(Refusal::Cancelled),
        }
    }
}

impl fmt::Debug for SignInReply {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Text { .. } => "Text(..)",
            Self::Accept => "Accept",
            Self::Cancel => "Cancel",
        })
    }
}

/// A [`SignIn`] that asks nobody and shows nothing: a test's, or a caller
/// that has no row.
pub struct NoSignIn;

impl SignIn for NoSignIn {
    fn show(&self, _wait: Option<SignInWait>) {}

    fn ask(&self, _question: SignInWait) -> Result<Answer, Refusal> {
        Err(Refusal::NotAllowed)
    }
}

/// Why `ssh` did not connect, decided in one place from what it said and
/// how it ended (`hide_node::ssh::openssh`); the row words each one with
/// what to do (PRD system-ssh-device-link B5, B7, B8, B13, B14, B18-B21).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshProblem {
    /// A question or a browser check was needed and nobody was asked, or
    /// the operator cancelled or left it unanswered.
    SignInNeeded,
    /// The browser check was not finished in time.
    BrowserTimeout,
    /// The server refused every way of signing in that was tried.
    SignInRefused,
    /// The server's access policy refused the account (a `tailscale: `
    /// banner).
    PolicyRefused,
    /// The server's key differs from the one `known_hosts` holds.
    HostKeyChanged,
    /// The alias's host name does not resolve.
    NameNotFound,
    /// The server could not be reached.
    Unreachable,
    /// The alias's proxy command or jump host failed.
    ProxyFailed,
    /// The machine is not one Hide can run on.
    Unsupported,
    /// This computer has no `ssh` program.
    SshMissing,
    /// This computer's OpenSSH is older than 8.4, which cannot hand Hide a
    /// question.
    OpenSshOld,
    /// An app update replaced this Hide's program since it started (macOS),
    /// so it cannot start `ssh`: only Hide opened again, which runs the new
    /// build, can.
    HideUpdated,
    /// None of the above; the row shows ssh's last line.
    Unclassified,
}

impl SshProblem {
    pub fn code(self) -> &'static str {
        match self {
            Self::SignInNeeded => "sign_in_needed",
            Self::BrowserTimeout => "browser_timeout",
            Self::SignInRefused => "sign_in_refused",
            Self::PolicyRefused => "policy_refused",
            Self::HostKeyChanged => "host_key_changed",
            Self::NameNotFound => "name_not_found",
            Self::Unreachable => "unreachable",
            Self::ProxyFailed => "proxy_failed",
            Self::Unsupported => "unsupported",
            Self::SshMissing => "ssh_missing",
            Self::OpenSshOld => "openssh_old",
            Self::HideUpdated => "hide_updated",
            Self::Unclassified => "unclassified",
        }
    }

    /// Whether Hide tries again by itself after the backoff: only what a
    /// network or a server coming back can change. Everything else waits for
    /// the operator, so a background retry never repeats a refused sign-in
    /// at the server.
    pub fn retried(self) -> bool {
        matches!(
            self,
            Self::NameNotFound | Self::Unreachable | Self::ProxyFailed | Self::Unclassified
        )
    }

    /// What the operator does about it, the one grouping the device row and
    /// `hide device` share: sign in again, check the server key in a
    /// terminal, open Hide again, or retry.
    pub fn action(self) -> FailureAction {
        match self {
            Self::SignInNeeded | Self::BrowserTimeout | Self::SignInRefused => {
                FailureAction::SignIn
            }
            Self::HostKeyChanged => FailureAction::CheckHostKey,
            Self::HideUpdated => FailureAction::ReopenHide,
            Self::PolicyRefused
            | Self::NameNotFound
            | Self::Unreachable
            | Self::ProxyFailed
            | Self::Unsupported
            | Self::SshMissing
            | Self::OpenSshOld
            | Self::Unclassified => FailureAction::Retry,
        }
    }
}

/// What the operator does about a failed ssh connection ([`SshProblem::action`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureAction {
    SignIn,
    CheckHostKey,
    /// Quit Hide and open it again: no retry of this Hide can succeed.
    ReopenHide,
    Retry,
}

impl FailureAction {
    pub fn code(self) -> &'static str {
        match self {
            Self::SignIn => "sign_in",
            Self::CheckHostKey => "check_host_key",
            Self::ReopenHide => "reopen_hide",
            Self::Retry => "retry",
        }
    }
}

/// A classified ssh failure: the class and the one line the row may show
/// with it (ssh's last line for an unclassified one), never a secret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SshFailure {
    pub problem: SshProblem,
    pub detail: String,
}

#[derive(Debug)]
pub enum EstablishError {
    Connect(RemoteError),
    /// `ssh` did not connect, for the classified reason.
    Ssh(SshFailure),
    /// The machine is off and this attempt may not start it ([`Wake`]):
    /// nothing was started, and only an attempt that may wake it, or the
    /// machine starting by itself, brings the link back.
    Stopped(String),
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
            Self::Ssh(failure) => formatter.write_str(&failure.detail),
            Self::IdentityChanged { bound, observed } => write!(
                formatter,
                "The device now answers as {}, not {} that the helper was allowed on; allow it again in Settings to continue",
                observed.describe(),
                bound.describe()
            ),
            Self::Stopped(reason)
            | Self::Unsupported(reason)
            | Self::Install(reason)
            | Self::Helper(reason) => formatter.write_str(reason),
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

/// One connection attempt as the core starts it: whether a machine that is
/// off may be started and the operator may be asked (`wake`, decided once
/// per attempt), the row the attempt's questions and waits go to, what ends
/// it from outside, and what hears the link's end.
pub struct Attempt {
    pub wake: Wake,
    pub sign_in: std::sync::Arc<dyn SignIn>,
    pub cancel: Cancel,
    pub on_close: OnClose,
}

impl Attempt {
    /// An attempt that asks nobody, for a caller with no device row.
    pub fn unattended(wake: Wake, on_close: OnClose) -> Self {
        Self {
            wake,
            sign_in: std::sync::Arc::new(NoSignIn),
            cancel: Cancel::default(),
            on_close,
        }
    }
}

/// One connection attempt's life, shared by the core that started it and
/// the transport that runs it (PRD system-ssh-device-link B26).
///
/// The core cancels the attempt of a device that was removed, whose consent
/// was withdrawn, or that a newer attempt replaced. While the transport owns
/// a process for the attempt it arms that process's kill ([`Self::arm`]),
/// which a cancel runs at once, so a write or a wait on the process ends
/// with it; every other wait (its turn, the sign-in) looks at the cancel.
/// The transport disarms the kill as it hands a working link to the core,
/// which closes that link its own way from then on.
///
/// Each part of the attempt that may still run holds a share ([`Self::hold`]):
/// the worker until it returns, and a process's reaper until the process is
/// reaped. The attempt has ended once no share is held, so the next attempt
/// for the device waits until nothing of this one runs.
#[derive(Clone, Default)]
pub struct Cancel(std::sync::Arc<CancelState>);

#[derive(Default)]
struct CancelState {
    cancelled: std::sync::atomic::AtomicBool,
    kill: std::sync::Mutex<Option<Box<dyn Fn() + Send>>>,
    held: std::sync::Mutex<usize>,
    released: std::sync::Condvar,
    ended: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl std::fmt::Debug for Cancel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Cancel")
            .field("cancelled", &self.is_cancelled())
            .field("ended", &self.ended())
            .finish()
    }
}

fn locked<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Cancel {
    /// Cancels the attempt and runs its armed kill, which only tells the
    /// process's owner to end it and never waits, so a caller may hold a
    /// lock.
    pub fn cancel(&self) {
        self.0
            .cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(kill) = locked(&self.0.kill).as_ref() {
            kill();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Arms what ends the attempt's process at once on a cancel, and runs it
    /// now when the attempt is cancelled already. `kill` may run more than
    /// once and must not block.
    pub fn arm(&self, kill: impl Fn() + Send + 'static) {
        let mut armed = locked(&self.0.kill);
        let kill = armed.insert(Box::new(kill));
        if self.is_cancelled() {
            kill();
        }
    }

    /// The attempt's process now carries a link the core owns: a cancel no
    /// longer ends it.
    pub fn disarm(&self) {
        locked(&self.0.kill).take();
    }

    /// A share of the attempt that runs until the returned [`Held`] drops.
    pub fn hold(&self) -> Held {
        *locked(&self.0.held) += 1;
        Held(self.clone())
    }

    /// Runs `then` once the attempt's last share is given back, on the
    /// thread that gives it back and outside every lock of the attempt, or
    /// now when nothing of it runs. It must not block.
    pub fn when_ended(&self, then: impl FnOnce() + Send + 'static) {
        let held = locked(&self.0.held);
        if *held > 0 {
            *locked(&self.0.ended) = Some(Box::new(then));
            return;
        }
        drop(held);
        then();
    }

    /// Whether nothing of the attempt runs any more.
    pub fn ended(&self) -> bool {
        *locked(&self.0.held) == 0
    }

    /// Waits up to `within` for the attempt to end; answers whether it did.
    pub fn wait_ended(&self, within: std::time::Duration) -> bool {
        let held = locked(&self.0.held);
        let (held, _) = self
            .0
            .released
            .wait_timeout_while(held, within, |held| *held > 0)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *held == 0
    }
}

/// A share of a [`Cancel`]'s attempt, given back when dropped.
pub struct Held(Cancel);

impl Held {
    /// What this share stood for may still run, and nothing can confirm it
    /// ended: the share is never given back, so the attempt never counts as
    /// ended and a wait for it ends at its own bound.
    pub fn outlived(self) {
        std::mem::forget(self);
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let state = &self.0.0;
        let mut held = locked(&state.held);
        *held -= 1;
        if *held > 0 {
            return;
        }
        // Taken while the count is held, so a `when_ended` racing this drop
        // either finds the attempt running and leaves its callback here, or
        // finds it ended and runs it itself.
        let then = locked(&state.ended).take();
        state.released.notify_all();
        drop(held);
        if let Some(then) = then {
            then();
        }
    }
}

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

/// One WSL distribution of this PC, as Add device lists it: read from the
/// account's registry, so listing never starts WSL (PRD wsl-device D-13).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WslDistribution {
    pub name: String,
    /// 1 or 2.
    pub wsl_version: u8,
    /// Its first-run user setup has not finished, or it is still being
    /// installed: it cannot be connected until it has (B28).
    pub setup_unfinished: bool,
    /// The distribution `wsl.exe` runs when none is named.
    pub default: bool,
}

/// What one WSL listing found, or why there is none: an `Err` is a failure
/// to read, never "no WSL" (B25); a PC without WSL lists none.
pub type WslListing = Result<Vec<WslDistribution>, String>;

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

    /// The transport for the device `node`, the WSL distribution `distro` of
    /// this PC (PRD wsl-device D-07); nothing starts until it is used.
    fn wsl_transport(
        &self,
        _node: &str,
        _distro: &str,
    ) -> Result<std::sync::Arc<dyn DialedTransport>, String> {
        Err("WSL distributions can be connected only from the Windows app".to_owned())
    }

    /// This PC's WSL distributions, for Add device. Blocking and bounded;
    /// it starts no `wsl.exe`.
    fn wsl_distributions(&self) -> WslListing {
        Ok(Vec::new())
    }
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
    /// run it off the runtime lock.
    fn establish(
        &self,
        consent: &HostConsent,
        retirement_projects: &[String],
        attempt: Attempt,
    ) -> Result<Established, EstablishError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_add_key_command_quotes_a_file_a_shell_would_split() {
        let plain = add_key_command("/home/al/.ssh/id_ed25519");
        let spaced = add_key_command("/Users/example/a b/.ssh/it's");
        if cfg!(target_os = "macos") {
            assert_eq!(
                plain,
                "ssh-add --apple-use-keychain /home/al/.ssh/id_ed25519"
            );
            assert_eq!(
                spaced,
                r"ssh-add --apple-use-keychain '/Users/example/a b/.ssh/it'\''s'"
            );
        } else {
            assert_eq!(plain, "ssh-add /home/al/.ssh/id_ed25519");
            assert_eq!(spaced, r"ssh-add '/Users/example/a b/.ssh/it'\''s'");
        }
    }

    /// A cancel runs the armed kill, also one armed after it, and none once
    /// disarmed; the attempt ends only once every share is given back.
    #[test]
    fn a_cancel_kills_what_is_armed_and_the_attempt_ends_with_its_last_share() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let kills = std::sync::Arc::new(AtomicUsize::new(0));
        let counting = |kills: &std::sync::Arc<AtomicUsize>| {
            let kills = std::sync::Arc::clone(kills);
            move || {
                kills.fetch_add(1, Ordering::SeqCst);
            }
        };
        let attempt = Cancel::default();
        assert!(attempt.ended());
        let worker = attempt.hold();
        let child = attempt.hold();
        attempt.arm(counting(&kills));
        attempt.cancel();
        assert_eq!(kills.load(Ordering::SeqCst), 1);
        drop(worker);
        assert!(!attempt.ended());
        assert!(!attempt.wait_ended(std::time::Duration::from_millis(10)));
        let waiting = {
            let attempt = attempt.clone();
            std::thread::spawn(move || attempt.wait_ended(std::time::Duration::from_secs(10)))
        };
        let (told, heard) = std::sync::mpsc::channel();
        attempt.when_ended(move || told.send(()).unwrap());
        assert!(heard.try_recv().is_err());
        drop(child);
        assert!(waiting.join().unwrap());
        heard
            .try_recv()
            .expect("the end was told when the last share went");
        let (told, heard) = std::sync::mpsc::channel();
        attempt.when_ended(move || told.send(()).unwrap());
        heard
            .try_recv()
            .expect("an attempt that ended is told at once");

        let late = Cancel::default();
        late.cancel();
        late.arm(counting(&kills));
        assert_eq!(kills.load(Ordering::SeqCst), 2);
        let handed_on = Cancel::default();
        handed_on.arm(counting(&kills));
        handed_on.disarm();
        handed_on.cancel();
        assert_eq!(kills.load(Ordering::SeqCst), 2);
    }

    fn identity() -> RemoteHostIdentity {
        RemoteHostIdentity {
            host_id: "ssh:mini".to_owned(),
            alias: "mini".to_owned(),
            hostname: "mini.example.test".to_owned(),
            port: 2200,
        }
    }

    /// D-09, B16, B17: a consent covers the same account, address and port;
    /// once bound to a machine, only that machine; one from before contract
    /// 4 names none and covers whichever answers.
    #[test]
    fn a_consent_covers_its_account_and_the_machine_it_was_bound_to() {
        let bound = HostIdentity {
            user: "me".to_owned(),
            hostname: "studio.local".to_owned(),
            port: 22,
            machine_id: Some("m1".to_owned()),
            host_key_sha256: String::new(),
        };
        let observed = |change: fn(&mut HostIdentity)| {
            let mut identity = bound.clone();
            change(&mut identity);
            identity
        };
        assert!(bound.covers(&bound));
        assert!(!bound.covers(&observed(|it| it.machine_id = Some("m2".to_owned()))));
        assert!(!bound.covers(&observed(|it| it.machine_id = None)));
        assert!(!bound.covers(&observed(|it| it.user = "other".to_owned())));
        assert!(!bound.covers(&observed(|it| it.hostname = "elsewhere".to_owned())));
        assert!(!bound.covers(&observed(|it| it.port = 2222)));
        let legacy = HostIdentity {
            machine_id: None,
            host_key_sha256: "SHA256:old".to_owned(),
            ..bound.clone()
        };
        assert!(legacy.covers(&observed(|it| it.machine_id = Some("m2".to_owned()))));
        assert!(!legacy.covers(&observed(|it| it.port = 2222)));
        // A machine that names no id is never one a consent covers, and a
        // record bound to nothing covers no machine at all (P1-5).
        assert!(!legacy.covers(&observed(|it| it.machine_id = None)));
        let unbound = HostIdentity {
            machine_id: None,
            host_key_sha256: String::new(),
            ..bound.clone()
        };
        assert!(!unbound.covers(&bound));
    }

    /// Two registrations reach one account's install when their consents
    /// name the same machine, or, with a consent from before contract 4 on
    /// either side, the same host key, or failing both, the same address.
    #[test]
    fn registrations_share_an_account_by_the_evidence_both_consents_carry() {
        let fresh = HostIdentity {
            user: "me".to_owned(),
            hostname: "studio.local".to_owned(),
            port: 22,
            machine_id: Some("m1".to_owned()),
            host_key_sha256: String::new(),
        };
        let carried = HostIdentity {
            host_key_sha256: "SHA256:old".to_owned(),
            ..fresh.clone()
        };
        let legacy = HostIdentity {
            machine_id: None,
            ..carried.clone()
        };
        assert!(fresh.same_account(&carried));
        assert!(carried.same_account(&legacy));
        assert!(
            legacy.same_account(&fresh),
            "no evidence apart: the address"
        );
        let elsewhere = HostIdentity {
            hostname: "other.local".to_owned(),
            ..legacy.clone()
        };
        assert!(!fresh.same_account(&elsewhere));
        assert!(!fresh.same_account(&HostIdentity {
            machine_id: Some("m2".to_owned()),
            ..fresh.clone()
        }));
        assert!(!legacy.same_account(&HostIdentity {
            host_key_sha256: "SHA256:new".to_owned(),
            ..legacy.clone()
        }));
        assert!(!fresh.same_account(&HostIdentity {
            user: "other".to_owned(),
            ..fresh.clone()
        }));
    }

    #[test]
    fn capability_report_has_staged_failure_and_visible_recovery() {
        let mut report = CapabilityReport::new("op-1", identity());
        report.fail(RemoteStage::Auth, "sign-in needed", true, true);
        assert!(matches!(
            report.connection,
            RemoteConnectionState::ActionRequired { .. }
        ));
        assert!(matches!(
            report.stage(RemoteStage::Auth).unwrap().state,
            CapabilityState::Failed {
                retryable: true,
                action_required: true
            }
        ));
        report.pass(RemoteStage::Auth, "retried after sign-in");
        report.connected();
        assert_eq!(report.connection, RemoteConnectionState::Connected);
    }

    #[test]
    fn capability_report_requires_every_stage_before_connected() {
        let mut report = CapabilityReport::new("op-1", identity());
        for stage in [
            RemoteStage::Ssh,
            RemoteStage::Auth,
            RemoteStage::Protocol,
            RemoteStage::Herdr,
            RemoteStage::Pty,
        ] {
            report.pass(stage, "ok");
        }
        assert!(!report.all_required_passed());
        report.pass(RemoteStage::Git, "ok");
        assert!(report.all_required_passed());
    }
}
