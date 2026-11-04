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

use super::gate::Gate;
use super::host::bootstrap::{self, HerdrChoice};
use super::host::{ATTEMPT_ENDED as ENDED, HelperPackages, PaneEventsSlot, PaneHook, TerminalHook};
use super::hosts::{Resolve, ssh_g};
use super::openssh::{SshProgram, Version};
use super::session::{self, Dial};
use super::*;
use hide_node_link::call_as;
use hide_node_link::device::{
    Attempt, DeviceConnector, DeviceTransport, DialedTransport, EstablishError, Established,
    HostConsent, HostIdentity, NodeReady, SshFailure, SshHostListing, SshProblem,
};
use hide_node_link::protocol::{Call, Hello};

/// Connection attempts that dial at once across every device (B27); the
/// rest wait their turn, shown as queued on their rows.
pub const MAX_DIALING: usize = 4;
/// How long a device waits for its turn before its attempt fails.
const TURN_WITHIN: Duration = Duration::from_secs(10 * 60);

fn ended() -> EstablishError {
    EstablishError::Helper(ENDED.to_owned())
}

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
    /// This PC's WSL, whose distributions are devices too (PRD wsl-device
    /// D-01); none where there is none.
    wsl: Option<host::wsl::WslAccess>,
    /// The home every SSH device's node runs with in place of its account's
    /// (`HIDE_DEV_DEVICE_HOME`), as [`parse_device_home`] read it.
    device_home: Option<String>,
}

/// The development variable that gives every SSH device's node a home of
/// its own in place of the device account's, so a candidate's live check on
/// a shared account never writes that account's `~/.hide`, agent settings or
/// device node state: `~/<path>` under the account's home, or an absolute
/// path as the device's own system spells it (`/srv/x`, `C:/x`). The install
/// root's `~/` is that home too.
pub const DEVICE_HOME_VARIABLE: &str = "HIDE_DEV_DEVICE_HOME";

/// [`DEVICE_HOME_VARIABLE`]'s value when it names a home a device can be
/// given: `~/` and a path below the home, or a wire-absolute path, on one
/// line with no empty, `.` or `..` name.
pub fn parse_device_home(value: &str) -> Result<String, String> {
    let names = match value.strip_prefix("~/") {
        Some(rest) => rest,
        None if hide_platform::path::is_wire_absolute(value) => value,
        None => {
            return Err(format!(
                "{DEVICE_HOME_VARIABLE} must start with ~/ or name an absolute path"
            ));
        }
    };
    let names = names.trim_start_matches('/');
    if value.chars().any(char::is_control)
        || names.is_empty()
        || names
            .split('/')
            .any(|name| name.is_empty() || name == "." || name == "..")
    {
        return Err(format!(
            "{DEVICE_HOME_VARIABLE} must be a plain path with no empty, `.` or `..` name"
        ));
    }
    Ok(value.to_owned())
}

/// The home `configured` names on a device whose account's home is `home`
/// and whose system is `os`: below that home for `~/`, else as given when
/// it is absolute on that system.
pub(super) fn resolve_device_home(
    configured: &str,
    home: &str,
    os: &str,
) -> Result<String, String> {
    if let Some(rest) = configured.strip_prefix("~/") {
        return Ok(format!("{}/{rest}", home.trim_end_matches('/')));
    }
    let absolute = if os == "windows" {
        !configured.starts_with('/') && hide_platform::path::is_wire_absolute(configured)
    } else {
        configured.starts_with('/')
    };
    if absolute {
        Ok(configured.to_owned())
    } else {
        Err(format!(
            "{DEVICE_HOME_VARIABLE} {configured:?} is not an absolute path on this {os} device"
        ))
    }
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
            wsl: None,
            device_home: None,
        }
    }

    /// Runs every SSH device's node with `home` in place of its account's
    /// home ([`DEVICE_HOME_VARIABLE`]); `None` leaves the account's.
    pub fn with_device_home(mut self, home: Option<String>) -> Self {
        self.device_home = home;
        self
    }

    /// Reaches WSL distributions through `access`: this PC's WSL, or a
    /// test's stand-in.
    pub fn with_wsl(mut self, access: host::wsl::WslAccess) -> Self {
        self.wsl = Some(access);
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
                super::destination::validate_session(&session)?;
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
        super::destination::Target::parse(alias)?;
        let (panes, terminals) = self.hooks(node);
        Ok(Arc::new(SshDevice {
            program: SshProgram::new(self.ssh.clone(), home),
            alias: alias.to_owned(),
            herdr,
            askpass: self.askpass.clone(),
            packages: self.packages.clone(),
            device_home: self.device_home.clone(),
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
}

/// One device reached over the system `ssh`.
pub struct SshDevice {
    program: SshProgram,
    alias: String,
    herdr: HerdrChoice,
    askpass: Option<PathBuf>,
    packages: HelperPackages,
    device_home: Option<String>,
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

/// The dials [`MAX_DIALING`] admits at once, shared by every device. A dial
/// waiting for its place reads connecting: the operator has nothing to do
/// about the wait (design principle 13).
static DIALING: Gate = Gate::new(MAX_DIALING);

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
        if !host::check_git(&linked.host, &mut report) {
            return report;
        }
        report.connected();
        report
    }

    fn establish(
        &self,
        consent: &HostConsent,
        _retirement_projects: &[String],
        attempt: Attempt,
    ) -> Result<Established, EstablishError> {
        let since = Instant::now();
        let target = self.target();
        let stage = |name: &str| host::establish_stage(&target, name, since);
        let Attempt {
            wake,
            sign_in,
            cancel,
            on_close,
        } = attempt;
        stage("resolve");
        let version = self.program.version().map_err(EstablishError::Ssh)?;
        let resolved = self
            .program
            .resolve(&self.alias)
            .map_err(EstablishError::Ssh)?;
        let cancelled = || cancel.is_cancelled();
        let _turn = DIALING
            .admit(Instant::now() + TURN_WITHIN, Some(&cancelled))
            .ok_or_else(|| {
                if cancel.is_cancelled() {
                    return ended();
                }
                EstablishError::Ssh(SshFailure {
                    problem: SshProblem::Unclassified,
                    detail: format!(
                        "{MAX_DIALING} other devices kept connecting for {} minutes",
                        TURN_WITHIN.as_secs() / 60
                    ),
                })
            })?;
        let (mut pipe, facts) = session::sign_in(
            &Dial {
                program: &self.program,
                alias: &self.alias,
                resolved: &resolved,
                version,
                askpass: self.askpass.as_deref(),
                target: &target,
            },
            Arc::clone(&sign_in),
            wake,
            &cancel,
            &stage,
        )?;
        // Until the helper starts, every way out ends and reaps the `ssh`
        // child before the attempt returns (B8, B26).
        let started = (|pipe: &mut session::SigningIn| -> Result<(HostIdentity, bootstrap::Installed), EstablishError> {
            let identity = HostIdentity {
                user: resolved.user.clone(),
                hostname: resolved.hostname.clone(),
                port: resolved.port,
                machine_id: Some(facts.machine_id.clone()),
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
            let payload = self.packages.payload_for(&facts.os, &facts.arch)?;
            let home = match &self.device_home {
                Some(configured) => Some(
                    resolve_device_home(configured, &facts.home, &facts.os)
                        .map_err(EstablishError::Install)?,
                ),
                None => None,
            };
            stage("install");
            let installed = bootstrap::install(
                &mut *pipe,
                &consent.helper_root,
                home.as_deref().unwrap_or(&facts.home),
                &payload,
            )?;
            stage("start");
            bootstrap::start(&mut *pipe, &installed.helper_relative, &self.herdr, home.as_deref())?;
            Ok((identity, installed))
        })(&mut pipe);
        let (identity, installed) = match started {
            Ok(started) => started,
            Err(error) => {
                pipe.link.kill();
                return Err(if cancel.is_cancelled() {
                    ended()
                } else {
                    error
                });
            }
        };
        let mut link = pipe.link;
        let rest = link.take_rest();
        let host = link.into_host(rest, self.panes.clone(), on_close);
        stage("hello");
        let hello: Hello = call_as(&host, Call::Hello, host::hello_timeout(installed.fresh))
            .map_err(|error| {
                host.close("the helper did not answer Hello");
                EstablishError::Helper(format!("The device helper did not start: {error}"))
            })?;
        if let Some(reason) = host::helper_protocol_refusal(&hello) {
            host.close("helper protocol mismatch");
            return Err(EstablishError::Helper(reason));
        }
        if cancel.is_cancelled() {
            host.close(ENDED);
            return Err(ended());
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
        // The device's panes run on the core's own engine, in pane holders
        // its node runs; the core runs its kit pass after ready.
        let kit = None;
        stage("holders");
        let terminals = host::start_holders_at(&host, self.terminals.clone());
        // The link is the core's from here, which closes it its own way: a
        // cancel no longer kills ssh. One that came before ends the link
        // here; one after it reaches the core, which drops this attempt's
        // result as stale and closes it.
        cancel.disarm();
        if cancel.is_cancelled() {
            host.close(ENDED);
            return Err(ended());
        }
        *lock_recover(&self.link) = Some(Linked {
            host: host.clone(),
            version,
            signed_in: format!("{}@{}:{}", resolved.user, resolved.hostname, resolved.port),
            protocol: hello.protocol,
            herdr_socket: None,
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
