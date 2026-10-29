//! The device transport behind `hide_node_link::device`: the node that holds
//! the account's SSH configuration and keys reaches each registered device
//! with the system `ssh`, and the core sees only the traits.
//!
//! A device is one `ssh <alias>` this process starts and owns per connection
//! (PRD system-ssh-device-link D-02): OpenSSH reads the alias exactly as a
//! terminal does (keys, the agent, ProxyJump, certificates, Tailscale SSH),
//! asks what it must ask through Hide's askpass, which puts it on the device
//! row ([`super::askpass`]), and the same input and output carry the start
//! script, the install and then the helper's link ([`super::host::bootstrap`]).
//! Herdr, files, Git, terminals and pages are that link's streams and calls,
//! so a connection signs in once.

use super::host::bootstrap::{self, HerdrChoice, Pipe, ProbeError, Stop};
use super::host::{ChildLink, ChildOutput, HelperPackages, PaneEventsSlot, PaneHook, TerminalHook};
use super::hosts::{Resolve, ssh_g};
use super::openssh::{self, SignInLine, SshProgram, Version};
use super::*;
use hide_node_link::call_as;
use hide_node_link::device::{
    Attempt, DeviceConnector, DeviceTransport, DialedTransport, EstablishError, Established,
    HostConsent, HostIdentity, NodeReady, SignIn, SignInWait, SshFailure, SshHostListing,
    SshProblem, Wake,
};
use hide_node_link::protocol::{Call, Hello, KitAction};

/// Connection attempts that dial at once across every device (B27); the
/// rest wait their turn, shown as queued on their rows.
pub const MAX_DIALING: usize = 4;
/// How long a device waits for its turn before its attempt fails.
const TURN_WITHIN: Duration = Duration::from_secs(10 * 60);
/// How long ssh has to sign in and reach the start script, not counting the
/// time a question or a browser check waits for the operator: a server that
/// takes longer is not answering.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a browser check waits (D-10).
const BROWSER_WITHIN: Duration = Duration::from_secs(5 * 60);
/// How long an attempt nobody may be asked for keeps going after a sign-in
/// link appeared: an ordinary banner's link is followed by the connection at
/// once, and a check that holds the sign-in waits for the operator (B14).
const UNATTENDED_BROWSER_GRACE: Duration = Duration::from_secs(15);
/// The lines of ssh's error output kept for the classification and the log.
const STDERR_TAIL: usize = 64;
/// How long the helper's Herdr has to answer before the kit is asked to
/// start it.
const HERDR_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the kit pass a connection runs before its services may take.
const KIT_TIMEOUT: Duration = Duration::from_secs(180);

/// Opens the transport to a device by alias. It carries this build's
/// device packages, which a transport installs on a device that lacks them.
pub struct Connector {
    packages: HelperPackages,
    /// The `ssh` devices are reached with, and the `hide` that answers its
    /// questions: both beside this program.
    ssh: PathBuf,
    askpass: Option<PathBuf>,
    /// What resolves an alias for Add device's list: `ssh -G`, or a test's
    /// answers.
    resolve: Resolve,
    /// Where each device's pane events go; none for a connector whose
    /// devices' panes do not reach this process.
    panes: Option<PaneEventsSlot>,
    /// Where each device's terminals' output and reports go; none for a
    /// connector whose devices' terminals do not reach this process.
    terminals: Option<Arc<dyn crate::terminal::device::DeviceSink>>,
    herdr_program: PathBuf,
    /// This PC's WSL, whose distributions are devices too (PRD wsl-device
    /// D-01); none where there is none.
    wsl: Option<host::wsl::WslAccess>,
}

impl Connector {
    /// `helper_dir` is the folder holding the builds devices run and the
    /// `hide` that answers ssh's questions; `None` leaves every device's
    /// node `unsupported` with that reason.
    pub fn new(helper_dir: Option<PathBuf>) -> Self {
        let askpass = helper_dir
            .as_ref()
            .map(|dir| dir.join(format!("hide{}", std::env::consts::EXE_SUFFIX)));
        Self {
            packages: HelperPackages::new(helper_dir),
            ssh: PathBuf::from("ssh"),
            askpass,
            resolve: ssh_g(PathBuf::from("ssh")),
            panes: None,
            terminals: None,
            herdr_program: PathBuf::from("herdr"),
            wsl: None,
        }
    }

    /// Reaches WSL distributions through `access`: this PC's WSL, or a
    /// test's stand-in.
    pub fn with_wsl(mut self, access: host::wsl::WslAccess) -> Self {
        self.wsl = Some(access);
        self
    }

    pub fn with_herdr_program(mut self, program: PathBuf) -> Self {
        self.herdr_program = program;
        self
    }

    /// Each device's panes' terminals flow inside its node link, and their
    /// output and reports go to `sink`.
    pub fn with_terminals(mut self, sink: Arc<dyn crate::terminal::device::DeviceSink>) -> Self {
        self.terminals = Some(sink);
        self
    }

    /// Each device's node serves its panes' `hide` over its link, and what
    /// they send goes to `events`.
    pub fn with_pane_events(mut self, events: PaneEventsSlot) -> Self {
        self.panes = Some(events);
        self
    }

    /// Reaches devices and resolves aliases with `ssh` instead of the `ssh`
    /// on `PATH`.
    pub fn with_ssh_program(mut self, ssh: PathBuf) -> Self {
        self.resolve = ssh_g(ssh.clone());
        self.ssh = ssh;
        self
    }

    /// Answers ssh's questions with `askpass` instead of the `hide` beside
    /// the helpers.
    pub fn with_askpass_program(mut self, askpass: PathBuf) -> Self {
        self.askpass = Some(askpass);
        self
    }

    /// Resolves aliases for Add device's list with `resolve`, so a test
    /// decides each answer and no child races the `ssh -G` deadline.
    pub fn with_resolve(mut self, resolve: Resolve) -> Self {
        self.resolve = resolve;
        self
    }

    fn hooks(&self, node: &str) -> (Option<PaneHook>, Option<TerminalHook>) {
        (
            self.panes.clone().map(|events| PaneHook {
                node: node.to_owned(),
                events,
            }),
            self.terminals.clone().map(|sink| TerminalHook {
                node: node.to_owned(),
                sink,
            }),
        )
    }
}

impl DeviceConnector for Connector {
    fn transport(
        &self,
        home: &Path,
        node: &str,
        alias: &str,
        herdr_socket: Option<String>,
    ) -> Result<Arc<dyn DialedTransport>, String> {
        self.transport_session(home, node, alias, herdr_socket, None)
    }

    fn transport_session(
        &self,
        home: &Path,
        node: &str,
        alias: &str,
        herdr_socket: Option<String>,
        session: Option<String>,
    ) -> Result<Arc<dyn DialedTransport>, String> {
        let herdr = match (session, herdr_socket) {
            (Some(_), Some(_)) => {
                return Err(
                    "A saved session cannot also select a different Herdr socket".to_owned(),
                );
            }
            (Some(session), None) => {
                super::profiles::validate_session(&session)?;
                HerdrChoice::Session(session)
            }
            (None, Some(socket)) => {
                if !hide_node_link::device::valid_remote_socket_path(&socket) {
                    return Err(
                        "The device's Herdr socket must be an absolute single-line path".to_owned(),
                    );
                }
                HerdrChoice::Socket(socket)
            }
            (None, None) => HerdrChoice::Account,
        };
        super::profiles::Target::parse(alias)?;
        let (panes, terminals) = self.hooks(node);
        Ok(Arc::new(SshDevice {
            program: SshProgram::new(self.ssh.clone(), home),
            alias: alias.to_owned(),
            herdr,
            askpass: self.askpass.clone(),
            packages: self.packages.clone(),
            panes,
            terminals,
            link: Arc::default(),
        }))
    }

    fn wsl_transport(&self, node: &str, distro: &str) -> Result<Arc<dyn DialedTransport>, String> {
        let access = self.wsl.clone().ok_or_else(|| {
            "WSL distributions can be connected only from the Windows app".to_owned()
        })?;
        let (panes, terminals) = self.hooks(node);
        Ok(Arc::new(host::wsl::WslDevice::new(
            distro,
            access,
            self.packages.clone(),
            panes,
            terminals,
        )?))
    }

    fn wsl_distributions(&self) -> hide_node_link::device::WslListing {
        self.wsl
            .as_ref()
            .map_or_else(|| Ok(Vec::new()), host::wsl::WslAccess::listing)
    }

    fn ssh_hosts(&self, home: &Path, registered: &[String], stop: &AtomicBool) -> SshHostListing {
        super::hosts::list(&self.resolve, home, registered, stop)
    }

    fn herdr_profiles(
        &self,
        home: &Path,
        registered: &[String],
        stop: &AtomicBool,
    ) -> Result<hide_node_link::device::HerdrProfileListing, String> {
        super::profiles::list(&self.herdr_program, home, registered, stop)
    }
}

/// One device reached over the system `ssh`.
pub struct SshDevice {
    program: SshProgram,
    alias: String,
    herdr: HerdrChoice,
    askpass: Option<PathBuf>,
    packages: HelperPackages,
    panes: Option<PaneHook>,
    terminals: Option<TerminalHook>,
    /// The link the latest attempt started, which carries the device's
    /// Herdr; reaching Herdr never starts `ssh`.
    link: Arc<Mutex<Option<Linked>>>,
}

/// What the current link answered when it started, for the device's test.
#[derive(Clone)]
struct Linked {
    host: host::RemoteHost,
    version: Version,
    signed_in: String,
    protocol: u32,
    herdr_socket: Option<String>,
    terminals: Result<(), String>,
}

impl SshDevice {
    fn target(&self) -> String {
        format!("ssh:{}", self.alias)
    }

    fn current(&self) -> Option<Linked> {
        lock_recover(&self.link)
            .clone()
            .filter(|linked| linked.host.closed_reason().is_none())
    }
}

impl DeviceTransport for SshDevice {
    fn herdr_api_connector(&self) -> Arc<dyn ApiConnector> {
        Arc::new(LinkedHerdr(Arc::clone(&self.link)))
    }

    fn cached_herdr_version(&self) -> Option<String> {
        None
    }

    fn cached_herdr_socket(&self) -> Option<String> {
        lock_recover(&self.link)
            .as_ref()
            .and_then(|linked| linked.herdr_socket.clone())
    }
}

/// The device's Herdr through its current link; no link, no Herdr, and
/// nothing is started to find one.
struct LinkedHerdr(Arc<Mutex<Option<Linked>>>);

impl ApiConnector for LinkedHerdr {
    fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError> {
        let host = lock_recover(&self.0)
            .as_ref()
            .map(|linked| linked.host.clone())
            .ok_or_else(|| {
                ApiError::Transport("The device's helper is not connected".to_owned())
            })?;
        host.herdr_connector().connect()
    }
}

/// The turns [`MAX_DIALING`] gives out, shared by every device.
static DIALING: Turns = Turns {
    taken: Mutex::new(0),
    freed: std::sync::Condvar::new(),
};

struct Turns {
    taken: Mutex<usize>,
    freed: std::sync::Condvar,
}

/// One attempt's turn; its end frees it.
struct Turn(&'static Turns);

impl Turns {
    /// Waits for a turn, showing the row queued meanwhile.
    fn take(&'static self, sign_in: &dyn SignIn) -> Option<Turn> {
        let deadline = Instant::now() + TURN_WITHIN;
        let mut taken = lock_recover(&self.taken);
        let mut shown = false;
        while *taken >= MAX_DIALING {
            if !shown {
                sign_in.show(Some(SignInWait::Queued));
                shown = true;
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            taken = self
                .freed
                .wait_timeout(taken, left)
                .map(|(guard, _)| guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner().0);
        }
        *taken += 1;
        drop(taken);
        if shown {
            sign_in.show(None);
        }
        Some(Turn(self))
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        *lock_recover(&self.0.taken) -= 1;
        self.0.freed.notify_one();
    }
}

/// The start script's pipe on an `ssh` that may still be signing in: what
/// ssh prints on its error output meanwhile is read for a browser check
/// (D-10) and kept for the classification, and the time a question or a
/// check waits for the operator does not count against the sign-in's own
/// clock.
struct SigningIn<'a> {
    link: ChildLink,
    /// Open until the connection reaches the device's shell.
    desk: Option<askpass::Desk>,
    sign_in: &'a dyn SignIn,
    wake: Wake,
    stderr: std::collections::VecDeque<String>,
    signed_in: bool,
    /// When a sign-in link was shown on the row (or, for an attempt nobody
    /// may be asked for, seen).
    browser: Option<Instant>,
    browser_expired: bool,
    /// Time the operator held, added to every deadline given.
    held: Duration,
    tick: Instant,
}

impl SigningIn<'_> {
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

impl Pipe for SigningIn<'_> {
    fn send(&mut self, bytes: &[u8], until: Instant) -> Result<(), String> {
        self.link
            .send(bytes, until + self.held)
            .map_err(|error| error.to_string())
    }

    fn line(&mut self, until: Instant) -> Result<Vec<u8>, Stop> {
        loop {
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
}

impl DialedTransport for SshDevice {
    fn capability_test(&self, operation_id: &str, check: SnapshotCheck<'_>) -> CapabilityReport {
        let mut report = CapabilityReport::new(
            operation_id,
            RemoteHostIdentity {
                host_id: self.target(),
                alias: self.alias.clone(),
                hostname: self.alias.clone(),
                port: 0,
            },
        );
        // The test reads the connection the device has; it never dials, so
        // it never asks for a sign-in of its own (B25).
        let Some(linked) = self.current() else {
            report.fail(
                RemoteStage::Ssh,
                "The device is not connected; Retry connects it",
                true,
                false,
            );
            return report;
        };
        report.pass(
            RemoteStage::Ssh,
            format!("OpenSSH {}.{}", linked.version.major, linked.version.minor),
        );
        report.pass(RemoteStage::Auth, linked.signed_in.clone());
        report.pass(
            RemoteStage::Protocol,
            format!("protocol {}", linked.protocol),
        );
        let socket = linked.herdr_socket.clone().unwrap_or_default();
        let snapshot = hide_herdr_client::request_with_connector(
            &linked.host.herdr_connector(),
            "session.snapshot",
            json!({}),
            Duration::from_secs(10),
        );
        match snapshot
            .map_err(|error| error.to_string())
            .and_then(|value| {
                check(&value, &self.target(), &socket).map_err(|error| error.to_string())
            }) {
            Ok(_) => report.pass(RemoteStage::Herdr, socket),
            Err(reason) => {
                report.fail(RemoteStage::Herdr, reason, true, false);
                return report;
            }
        }
        match &linked.terminals {
            Ok(()) => report.pass(RemoteStage::Pty, "through the node link"),
            Err(reason) => {
                report.fail(RemoteStage::Pty, reason.clone(), true, false);
                return report;
            }
        }
        report.pass(RemoteStage::Git, "through the node link");
        report.connected();
        report
    }

    fn establish(
        &self,
        consent: &HostConsent,
        retirement_projects: &[String],
        attempt: Attempt,
    ) -> Result<Established, EstablishError> {
        let since = Instant::now();
        let target = self.target();
        let stage = |name: &str| host::establish_stage(&target, name, since);
        let Attempt {
            wake,
            sign_in,
            on_close,
        } = attempt;
        stage("resolve");
        let version = self.program.version().map_err(EstablishError::Ssh)?;
        let resolved = self
            .program
            .resolve(&self.alias)
            .map_err(EstablishError::Ssh)?;
        let _turn = DIALING.take(&*sign_in).ok_or_else(|| {
            EstablishError::Ssh(SshFailure {
                problem: SshProblem::Unclassified,
                detail: format!(
                    "{MAX_DIALING} other devices kept connecting for {} minutes",
                    TURN_WITHIN.as_secs() / 60
                ),
            })
        })?;
        // B21: an ssh that cannot hand a question to Hide gets none to hand.
        let desk = match (&self.askpass, version.asks_through_askpass()) {
            (Some(program), true) => {
                if !program.is_file() {
                    return Err(EstablishError::Helper(format!(
                        "Hide's sign-in helper {} is missing; reinstall Hide",
                        program.display()
                    )));
                }
                Some(
                    askpass::Desk::open(&std::env::temp_dir(), Arc::clone(&sign_in)).map_err(
                        |error| {
                            EstablishError::Helper(format!(
                                "The sign-in desk could not open: {error}"
                            ))
                        },
                    )?,
                )
            }
            _ => None,
        };
        let mut command = self.program.command();
        command.args(self.program.link_args(
            &self.alias,
            &resolved,
            version,
            bootstrap::REMOTE_COMMAND,
        ));
        match (&desk, &self.askpass) {
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
        let child = match hide_platform::process::OwnedChild::spawn(&mut command) {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(EstablishError::Ssh(openssh::missing(self.program.path())));
            }
            Err(error) => {
                return Err(EstablishError::Helper(format!(
                    "ssh could not start: {error}"
                )));
            }
        };
        let link = ChildLink::start(&target, child).map_err(|error| {
            EstablishError::Helper(format!("ssh's link could not start: {error}"))
        })?;
        let mut pipe = SigningIn {
            link,
            desk,
            sign_in: &*sign_in,
            wake,
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
                return Err(EstablishError::Unsupported(reason));
            }
            Err(ProbeError::Stopped(stop)) => {
                let stderr: Vec<String> = pipe.stderr.iter().cloned().collect();
                let mut failure =
                    openssh::classify(&openssh::Ending {
                        stderr: &stderr,
                        refusal: pipe.desk.as_ref().and_then(askpass::Desk::refusal).or_else(
                            || {
                                (pipe.browser_expired && wake == Wake::NotAllowed)
                                    .then_some(hide_node_link::device::Refusal::NotAllowed)
                            },
                        ),
                        proxied: resolved.proxied(),
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
                    "target": target,
                    "problem": failure.problem.code(),
                    "stderr_tail": stderr,
                }));
                return Err(EstablishError::Ssh(failure));
            }
        };
        // Signed in: nothing more is asked on this connection.
        pipe.desk = None;
        let identity = HostIdentity {
            user: resolved.user.clone(),
            hostname: resolved.hostname.clone(),
            port: resolved.port,
            machine_id: facts.machine_id.clone(),
            host_key_sha256: String::new(),
        };
        if let Some(bound) = consent.identity.as_ref()
            && !bound.covers(&identity)
        {
            return Err(EstablishError::IdentityChanged {
                bound: Box::new(bound.clone()),
                observed: Box::new(identity),
            });
        }
        let payload = self.packages.payload(&facts.os, &facts.arch)?;
        stage("install");
        let installed = bootstrap::install(&mut pipe, &consent.helper_root, &facts.home, &payload)?;
        stage("start");
        bootstrap::start(&mut pipe, &installed.helper_relative, &self.herdr)?;
        let SigningIn { mut link, .. } = pipe;
        let rest = link.take_rest();
        let host = link.into_host(rest, self.panes.clone(), on_close);
        stage("hello");
        let hello: Hello = call_as(&host, Call::Hello, host::HELLO_TIMEOUT).map_err(|error| {
            host.close("the helper did not answer Hello");
            EstablishError::Helper(format!("The device helper did not start: {error}"))
        })?;
        if let Some(reason) = host::helper_protocol_refusal(&hello) {
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
        let socket = hello
            .herdr_socket
            .clone()
            .ok_or_else(|| "the device's helper bridges no Herdr socket".to_owned());
        // Only a Herdr that does not answer needs the kit before the services
        // start; otherwise the core runs its pass after ready. A named
        // session is attached only, never started by a connection's kit.
        let unanswered = self.herdr == HerdrChoice::Account
            && hide_herdr_client::request_with_connector(
                &host.herdr_connector(),
                "session.snapshot",
                json!({}),
                HERDR_CHECK_TIMEOUT,
            )
            .is_err();
        let kit = unanswered.then(|| {
            stage("kit");
            call_as::<hide_kit::KitReport>(
                &host,
                Call::Kit {
                    action: KitAction::Apply,
                    cli_dir: consent
                        .cli_dir
                        .clone()
                        .unwrap_or_else(|| host::DEFAULT_CLI_DIR.to_owned()),
                    herdr_socket: socket.clone().ok(),
                    preserve_herdr_server: false,
                    retirement_projects: retirement_projects.to_vec(),
                },
                KIT_TIMEOUT,
            )
            .map(Box::new)
            .map_err(|error| error.to_string())
        });
        if self.panes.is_some() {
            stage("panes");
            host::start_panes_at(socket.clone(), &host);
        }
        let terminals = match self.terminals.clone() {
            None => Err("this process takes no device terminals".to_owned()),
            Some(hook) => {
                stage("terminals");
                host::start_terminals_at(socket.clone(), &host, hook)
            }
        };
        *lock_recover(&self.link) = Some(Linked {
            host: host.clone(),
            version,
            signed_in: format!("{}@{}:{}", resolved.user, resolved.hostname, resolved.port),
            protocol: hello.protocol,
            herdr_socket: socket.ok(),
            terminals: terminals.as_ref().map(|_| ()).map_err(Clone::clone),
        });
        stage("ready");
        Ok(Established {
            node: NodeReady {
                host: Arc::new(host),
                hello,
                terminals,
            },
            identity,
            installed: installed.fresh,
            helper_path: installed.helper_path,
            upload: installed.upload,
            kit,
        })
    }
}
