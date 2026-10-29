//! One connection over the system `ssh` that Hide starts and owns, signed in
//! through Hide's askpass (PRD system-ssh-device-link D-02, D-05, B33).
//!
//! A device's link ([`super::SshDevice`]) and the connection to a core on
//! another machine ([`Session`]) start the same way: `ssh <alias>` runs the
//! start script ([`super::host::bootstrap`]), its questions go to the
//! attempt's [`SignIn`] through a desk ([`super::askpass`]), what it prints
//! while it signs in is read for a browser check and kept for the one
//! classification of a run that did not connect ([`openssh::classify`]),
//! and the script's probe is the sign-in's end. A device then installs and
//! starts its helper; a session takes steps until its owner ends it or
//! hands its pipes to the attach role.

use super::host::bootstrap::{self, Facts, Pipe, ProbeError, StepError, Stop};
use super::host::{ATTEMPT_ENDED as ENDED, ChildLink, ChildOutput, HelperPackages};
use super::openssh::{self, Resolved, SignInLine, SshProgram, Version};
use super::transfer::FileCopy;
use super::*;
use hide_node_link::device::{
    Cancel, EstablishError, NoSignIn, Refusal, SignIn, SignInWait, SshFailure, SshProblem, Wake,
};

pub use super::host::bootstrap::Ran;

/// How long ssh has to sign in and reach the start script, not counting the
/// time a question or a browser check waits for the operator: a server that
/// takes longer is not answering.
pub(super) const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a browser check waits (D-10).
const BROWSER_WITHIN: Duration = Duration::from_secs(5 * 60);
/// How long an attempt nobody may be asked for keeps going after a sign-in
/// link appeared: an ordinary banner's link is followed by the connection at
/// once, and a check that holds the sign-in waits for the operator (B14).
const UNATTENDED_BROWSER_GRACE: Duration = Duration::from_secs(15);
/// The lines of ssh's error output kept for the classification and the log.
const STDERR_TAIL: usize = 64;

/// The start script's pipe on an `ssh` that may still be signing in: what
/// ssh prints on its error output meanwhile is read for a browser check
/// (D-10) and kept for the classification, and the time a question or a
/// check waits for the operator does not count against the sign-in's own
/// clock.
pub(super) struct SigningIn {
    pub(super) link: ChildLink,
    /// Open until the connection reaches the device's shell.
    pub(super) desk: Option<askpass::Desk>,
    sign_in: Arc<dyn SignIn>,
    wake: Wake,
    cancel: Cancel,
    pub(super) stderr: std::collections::VecDeque<String>,
    signed_in: bool,
    /// When a sign-in link was shown on the row (or, for an attempt nobody
    /// may be asked for, seen).
    browser: Option<Instant>,
    pub(super) browser_expired: bool,
    /// Time the operator held, added to every deadline given.
    pub(super) held: Duration,
    tick: Instant,
}

impl SigningIn {
    fn operator_turn(&self) -> bool {
        self.desk.as_ref().is_some_and(askpass::Desk::operator_turn)
            || (self.browser.is_some() && self.wake == Wake::Allowed)
    }

    fn error_line(&mut self, line: String) {
        if !self.signed_in
            && self.browser.is_none()
            && let SignInLine::Browser(url) = openssh::sign_in_line(&line)
        {
            self.browser = Some(Instant::now());
            if self.wake == Wake::Allowed {
                self.sign_in.show(Some(SignInWait::Browser { url }));
            }
        }
        if self.stderr.len() == STDERR_TAIL {
            self.stderr.pop_front();
        }
        self.stderr.push_back(line);
    }

    /// The sign-in is over: the connection reached the device's shell.
    fn reached_shell(&mut self) {
        if !self.signed_in {
            self.signed_in = true;
            if self.browser.take().is_some() && self.wake == Wake::Allowed {
                self.sign_in.show(None);
            }
        }
    }

    /// Whether a browser check ran out of its time.
    fn browser_over(&mut self) -> bool {
        let Some(shown) = self.browser else {
            return false;
        };
        let within = if self.wake == Wake::Allowed {
            BROWSER_WITHIN
        } else {
            UNATTENDED_BROWSER_GRACE
        };
        if shown.elapsed() < within {
            return false;
        }
        self.browser_expired = true;
        if self.wake == Wake::Allowed {
            self.sign_in.show(None);
        }
        true
    }
}

impl Pipe for SigningIn {
    fn send(&mut self, bytes: &[u8], until: Instant) -> Result<(), String> {
        if self.cancel.is_cancelled() {
            return Err(ENDED.to_owned());
        }
        self.link
            .send(bytes, until + self.held)
            .map_err(|error| error.to_string())
    }

    fn line(&mut self, until: Instant) -> Result<Vec<u8>, Stop> {
        loop {
            if self.cancel.is_cancelled() {
                return Err(Stop::Ended(ENDED.to_owned()));
            }
            // B8: the first question the operator cancelled or left
            // unanswered ends the attempt; ssh would only ask again or try
            // its next way of signing in.
            if !self.signed_in
                && self
                    .desk
                    .as_ref()
                    .is_some_and(|desk| desk.refusal().is_some())
            {
                return Err(Stop::Ended("a sign-in question got no answer".to_owned()));
            }
            let now = Instant::now();
            if !self.signed_in && self.operator_turn() {
                self.held += now.saturating_duration_since(self.tick);
            }
            self.tick = now;
            if !self.signed_in && self.browser_over() {
                return Err(Stop::Ended(
                    "the check in the browser was not finished in time".to_owned(),
                ));
            }
            let deadline = until + self.held;
            if now >= deadline {
                return Err(Stop::Quiet);
            }
            // A second at a time, so the operator's time is counted as it
            // passes.
            match self.link.next(deadline.min(now + Duration::from_secs(1))) {
                ChildOutput::Line(line) => {
                    self.reached_shell();
                    return Ok(line);
                }
                ChildOutput::Error(line) => self.error_line(line),
                ChildOutput::Quiet => {}
                ChildOutput::Ended(reason) => return Err(Stop::Ended(reason)),
            }
        }
    }

    fn bytes(&mut self, max: usize, until: Instant) -> Result<Vec<u8>, Stop> {
        if self.cancel.is_cancelled() {
            return Err(Stop::Ended(ENDED.to_owned()));
        }
        self.link.bytes(max, until + self.held)
    }
}

/// What one sign-in dials: the alias as `ssh -G` resolved it for this
/// attempt, and the `hide` that answers ssh's questions.
pub(super) struct Dial<'a> {
    pub program: &'a SshProgram,
    pub alias: &'a str,
    pub resolved: &'a Resolved,
    pub version: Version,
    pub askpass: Option<&'a Path>,
    /// Names the connection in the log.
    pub target: &'a str,
}

/// Starts `ssh` for the alias and reads the start script's probe through the
/// sign-in: the attempt's questions go to `sign_in` (none when this `ssh`
/// cannot hand one to Hide, B21), and a run that did not reach the script is
/// classified once, with ssh's raw words in the log. Every way out but the
/// probe's answer ends and reaps the `ssh` child before it returns (B8,
/// B26); from the probe on, the caller owns it through the pipe, and `cancel`
/// ends it at once until the caller disarms it.
pub(super) fn sign_in(
    dial: &Dial<'_>,
    sign_in: Arc<dyn SignIn>,
    wake: Wake,
    cancel: &Cancel,
    stage: &dyn Fn(&str),
) -> Result<(SigningIn, Facts), EstablishError> {
    let version = dial.version;
    // B21: an ssh that cannot hand a question to Hide gets none to hand.
    let desk = match (dial.askpass, version.asks_through_askpass()) {
        (Some(program), true) => {
            if !program.is_file() {
                return Err(EstablishError::Helper(format!(
                    "Hide's sign-in helper {} is missing; reinstall Hide",
                    program.display()
                )));
            }
            // ssh expands `~` and `%d` to the user database's home,
            // whatever HOME it was given.
            let key_files = hide_platform::user_agents::account_home()
                .ok()
                .and_then(|home| dial.resolved.identity_files(&home, dial.alias));
            Some(
                askpass::Desk::open(&std::env::temp_dir(), Arc::clone(&sign_in), key_files)
                    .map_err(|error| {
                        EstablishError::Helper(format!("The sign-in desk could not open: {error}"))
                    })?,
            )
        }
        _ => None,
    };
    let mut command = dial.program.command();
    // With no desk, nothing can hear a question, so ssh asks none, of a
    // terminal included.
    if desk.is_none() {
        command.args(["-o", "BatchMode=yes"]);
    }
    command.args(dial.program.link_args(
        dial.alias,
        dial.resolved,
        version,
        bootstrap::REMOTE_COMMAND,
    ));
    match (&desk, dial.askpass) {
        (Some(desk), Some(program)) => {
            command.envs(desk.environment(program));
        }
        _ => {
            command.env("SSH_ASKPASS_REQUIRE", "never");
        }
    }
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    stage("dial");
    // Tied: ssh still signing in reads nothing that would tell it this
    // process died, so its group ends with this process either way.
    let child = match hide_platform::process::OwnedChild::spawn_tied(&mut command) {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(EstablishError::Ssh(openssh::missing(dial.program.path())));
        }
        Err(error) => {
            return Err(EstablishError::Helper(format!(
                "ssh could not start: {error}"
            )));
        }
    };
    // From here a cancel ends ssh at once, and the attempt has ended only
    // once ssh is reaped.
    let link = ChildLink::start(dial.target, child, Some(cancel))
        .map_err(|error| EstablishError::Helper(format!("ssh's link could not start: {error}")))?;
    let mut pipe = SigningIn {
        link,
        desk,
        sign_in,
        wake,
        cancel: cancel.clone(),
        stderr: std::collections::VecDeque::new(),
        signed_in: false,
        browser: None,
        browser_expired: false,
        held: Duration::ZERO,
        tick: Instant::now(),
    };
    stage("sign_in");
    let facts = match bootstrap::probe(&mut pipe, Instant::now() + SIGN_IN_TIMEOUT) {
        Ok(facts) => facts,
        Err(ProbeError::Unsupported(reason)) => {
            pipe.link.kill();
            return Err(EstablishError::Unsupported(reason));
        }
        Err(ProbeError::Stopped(stop)) => {
            pipe.link.kill();
            if cancel.is_cancelled() {
                return Err(EstablishError::Helper(ENDED.to_owned()));
            }
            let stderr: Vec<String> = pipe.stderr.iter().cloned().collect();
            let mut failure = openssh::classify(&openssh::Ending {
                stderr: &stderr,
                refusal: pipe
                    .desk
                    .as_ref()
                    .and_then(askpass::Desk::refusal)
                    .or_else(|| {
                        (pipe.browser_expired && wake == Wake::NotAllowed)
                            .then_some(Refusal::NotAllowed)
                    }),
                proxied: dial.resolved.proxied(),
                prompts_unavailable: !version.asks_through_askpass(),
                browser_waited: pipe.browser_expired,
            });
            if failure.problem == SshProblem::Unclassified && stderr.is_empty() {
                failure.detail = match stop {
                    Stop::Quiet => {
                        format!("ssh did not sign in within {} s", SIGN_IN_TIMEOUT.as_secs())
                    }
                    Stop::Ended(reason) => reason,
                };
            }
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "host.ssh_failed",
                "target": dial.target,
                "problem": failure.problem.code(),
                "stderr_tail": stderr,
            }));
            return Err(EstablishError::Ssh(failure));
        }
    };
    // Signed in: nothing more is asked on this connection, and the
    // operator's time counted toward the sign-in alone; the steps' deadlines
    // are their own.
    pipe.desk = None;
    pipe.held = Duration::ZERO;
    Ok((pipe, facts))
}

/// Why a connection to another machine failed.
#[derive(Clone, Debug)]
pub enum SessionError {
    /// `ssh` did not connect or sign in, as [`openssh::classify`] read it.
    Ssh(SshFailure),
    /// The connection ended, or stopped answering, after it signed in.
    Lost(String),
    /// The machine refused the step, or is not one this build can serve.
    Refused(String),
    /// A file on this machine could not be read or written.
    Local(String),
}

impl SessionError {
    /// Whether the connection never reached the machine's SSH server: its
    /// name did not resolve, or nothing answered there.
    pub fn never_reached_server(&self) -> bool {
        matches!(
            self,
            Self::Ssh(SshFailure {
                problem: SshProblem::NameNotFound | SshProblem::Unreachable,
                ..
            })
        )
    }

    /// Whether a move of this machine or of the network may change the
    /// answer: a server not reached, a proxy that failed on the way, a
    /// connection lost after it signed in, or an ending ssh did not explain.
    /// A sign-in, a server key or an alias the operator must fix answers
    /// the same from any network.
    pub fn a_move_can_change(&self) -> bool {
        match self {
            Self::Ssh(failure) => matches!(
                failure.problem,
                SshProblem::NameNotFound
                    | SshProblem::Unreachable
                    | SshProblem::ProxyFailed
                    | SshProblem::Unclassified
            ),
            Self::Lost(_) => true,
            Self::Refused(_) | Self::Local(_) => false,
        }
    }

    fn of_establish(error: EstablishError) -> Self {
        match error {
            EstablishError::Ssh(failure) => Self::Ssh(failure),
            EstablishError::Helper(reason) if reason == ENDED => Self::Lost(reason),
            other => Self::Refused(other.to_string()),
        }
    }
}

impl From<StepError> for SessionError {
    fn from(error: StepError) -> Self {
        match error {
            StepError::Lost(reason) => Self::Lost(reason),
            StepError::Refused(reason) => Self::Refused(reason),
            StepError::Local(reason) => Self::Local(reason),
        }
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ssh(failure) => {
                write!(formatter, "{}: {}", failure.problem.code(), failure.detail)
            }
            Self::Lost(reason) => write!(formatter, "lost: {reason}"),
            Self::Refused(reason) => formatter.write_str(reason),
            Self::Local(reason) => write!(formatter, "local: {reason}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// The machine a [`Session`] reaches: an alias of the account's SSH
/// configuration, the `ssh` that reads it, and the `hide` that answers its
/// questions.
#[derive(Clone, Debug)]
pub struct SshTarget {
    program: SshProgram,
    alias: String,
    askpass: Option<PathBuf>,
}

impl SshTarget {
    /// `alias` as the account at `home` names it, through the `ssh` on
    /// `PATH`, with the `hide` beside this program hearing its questions; a
    /// program with no `hide` beside it, such as a test, has ssh ask none.
    pub fn new(home: &Path, alias: &str) -> Self {
        let askpass = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.parent()
                    .map(|folder| folder.join(format!("hide{}", std::env::consts::EXE_SUFFIX)))
            })
            .filter(|program| program.is_file());
        Self {
            program: SshProgram::new(PathBuf::from("ssh"), home),
            alias: alias.to_owned(),
            askpass,
        }
    }

    pub fn alias(&self) -> &str {
        &self.alias
    }

    /// What `ssh -G` resolves the alias to now.
    pub fn resolve(&self) -> Result<Resolved, SshFailure> {
        self.program.resolve(&self.alias)
    }
}

/// A connection to another machine through the system `ssh`, its start
/// script waiting for steps: the connection a node keeps to its core's
/// machine, and a core move's to the machine on the other side (PRD
/// system-ssh-device-link B33). One sign-in, then steps one at a time, until
/// the session is dropped, which ends its `ssh`, or [`Self::attach`] hands
/// its pipes to the attach role.
///
/// Nobody is asked anything yet: a question, an unknown server key or a
/// check in the browser ends the sign-in as `sign_in_needed`, as an attempt
/// Hide made by itself does (D-06); keys, the agent and Tailscale SSH sign
/// in.
pub struct Session {
    pipe: SigningIn,
    facts: Facts,
    target: String,
    /// A step lost the connection or left its input and output out of
    /// step: no later step is sent.
    broken: bool,
}

impl Session {
    /// Signs in to `target`'s alias, as `ssh -G` resolved it (`resolved`);
    /// `cancel` ends the `ssh` at once, from the dial to the session's end.
    pub fn open(
        target: &SshTarget,
        resolved: &Resolved,
        cancel: &Cancel,
    ) -> Result<Self, SessionError> {
        let name = format!("ssh:{}", target.alias);
        let version = target.program.version().map_err(SessionError::Ssh)?;
        let (pipe, facts) = sign_in(
            &Dial {
                program: &target.program,
                alias: &target.alias,
                resolved,
                version,
                askpass: target.askpass.as_deref(),
                target: &name,
            },
            Arc::new(NoSignIn),
            Wake::NotAllowed,
            cancel,
            &|_| {},
        )
        .map_err(SessionError::of_establish)?;
        Ok(Self {
            pipe,
            facts,
            target: name,
            broken: false,
        })
    }

    /// The account and platform the machine answered with.
    pub fn home(&self) -> &str {
        &self.facts.home
    }

    /// Whether a step lost this session, so no later step may use it.
    pub fn broken(&self) -> bool {
        self.broken
    }

    fn usable(&self) -> Result<(), SessionError> {
        if self.broken {
            return Err(SessionError::Lost(
                "an earlier step lost this connection".to_owned(),
            ));
        }
        Ok(())
    }

    fn checked<T>(&mut self, result: Result<T, StepError>) -> Result<T, SessionError> {
        if let Err(error) = &result
            && !matches!(error, StepError::Refused(_))
        {
            self.broken = true;
        }
        result.map_err(SessionError::from)
    }

    /// Puts this build's programs in their version folder under `root` on
    /// the machine, sending only what it lacks (a core update, PRD
    /// core-host-node-move B10). Answers the path of the build's `hided`
    /// there.
    pub fn install_build(
        &mut self,
        packages: &HelperPackages,
        root: &str,
    ) -> Result<String, SessionError> {
        self.usable()?;
        let payload = packages
            .payload(&self.facts.os, &self.facts.arch)
            .map_err(|error| SessionError::Refused(error.to_string()))?;
        let installed = bootstrap::install_build(&mut self.pipe, root, &self.facts.home, payload);
        // The install reads its own failure into words; whether the
        // connection is still in step is not among them.
        if installed.is_err() {
            self.broken = true;
        }
        installed.map_err(|error| SessionError::Refused(error.to_string()))
    }

    /// Runs `argv` on the machine with no input and answers its exit status
    /// and output; output past `cap` bytes, or a command still running after
    /// `within`, is a failure, and the command is not waited for.
    pub fn run(
        &mut self,
        argv: &[String],
        cap: usize,
        within: Duration,
    ) -> Result<Ran, SessionError> {
        self.usable()?;
        let ran = bootstrap::run(&mut self.pipe, argv, cap, Instant::now() + within);
        self.checked(ran)
    }

    /// Copies `file` to the machine, streamed and renamed into place whole
    /// ([`super::transfer::upload`]); `sent` hears the bytes as they go.
    pub fn upload(
        &mut self,
        file: &FileCopy,
        sent: &(dyn Fn(u64) + Sync),
    ) -> Result<(), SessionError> {
        self.usable()?;
        let copied = super::transfer::upload(&mut self.pipe, file, sent);
        self.checked(copied)
    }

    /// Copies `file` from the machine into its place below `into` here,
    /// whole ([`super::transfer::download`]); `received` hears the bytes as
    /// they arrive.
    pub fn download(
        &mut self,
        into: &Path,
        file: &FileCopy,
        received: &(dyn Fn(u64) + Sync),
    ) -> Result<(), SessionError> {
        self.usable()?;
        let copied = super::transfer::download(&mut self.pipe, into, file, received);
        self.checked(copied)
    }

    /// Starts `program attach` on the machine in place of the start script
    /// and answers its standard input and output as one local stream; its
    /// standard error, and ssh's, go to the stream's `stderr`.
    pub fn attach(
        mut self,
        program: &str,
        state_dir: Option<&str>,
    ) -> Result<AttachChannel, SessionError> {
        self.usable()?;
        let started = bootstrap::attach(&mut self.pipe, program, state_dir);
        self.checked(started)?;
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let stream = self
            .pipe
            .link
            .into_stream(Arc::clone(&stderr))
            .map_err(|error| SessionError::Local(format!("the attach stream: {error}")))?;
        crate::diagnostic!(json!({
            "component": "remote_host",
            "kind": "session.attached",
            "target": self.target,
        }));
        Ok(AttachChannel { stream, stderr })
    }
}

/// The attach role's standard input and output, as a local stream: what is
/// written to it reaches the attach role, and what the attach role writes is
/// read from it. The stream ends when the connection does, and the
/// connection ends when the stream is closed.
pub struct AttachChannel {
    pub stream: hide_platform::ipc::LocalStream,
    /// What the attach role and ssh wrote on their error output, for the
    /// diagnostic when the stream ends before a handshake.
    pub stderr: Arc<Mutex<Vec<u8>>>,
}
