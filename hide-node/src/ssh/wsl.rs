//! A WSL distribution of this PC as a device (PRD wsl-device D-01, D-07).
//!
//! The core reaches it as it reaches any device it dials: its node is this
//! build's Linux `hided`, installed on consent under the Linux account's
//! helper root, answering the node contract. What differs is the pipe: one
//! `wsl.exe -d <distro> -u <user> -e <hided> node serve --own-herdr` child
//! per distribution, owned by its link ([`super::over_child`]), whose stdin
//! and stdout carry the link. No `wsl.exe` runs per request. The device's
//! Herdr stays inside WSL and is reached through the link's own streams, and
//! its files and Git are the node's, by their Linux paths, never through a
//! `\\wsl$` share (D-15).
//!
//! The link's child first runs the start script every dialed device runs
//! ([`super::bootstrap`], PRD system-ssh-device-link D-02): the account
//! probe, the install and the helper's start go over its own pipes, so a
//! connection starts one `wsl.exe` besides the running check. Both run on
//! a connection attempt's worker, off the runtime lock, each with a
//! deadline; at most [`MAX_CALLS`] run at once in this process, and one
//! that does not answer in time is a failed attempt, never "no WSL" (B25).
//! Before any of them, the distribution's registry entry, read without
//! starting WSL, says whether it exists, which WSL runs it and whether its
//! first-run setup finished; and only an attempt that may wake a
//! distribution ([`Wake`]) runs anything in one that is off (D-11).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hide_herdr_client::ApiError;

use hide_node_link::call_as;
use hide_node_link::device::{
    Attempt, CapabilityReport, DeviceTransport, DialedTransport, EstablishError, Established,
    HostConsent, HostIdentity, NodeReady, RemoteHostIdentity, RemoteStage, SnapshotCheck, Wake,
    WslDistribution, WslListing,
};
use hide_node_link::protocol::{Call, Hello, KitAction};
use hide_platform::wsl::Distribution;
use serde_json::json;

use super::{
    ChildLink, DEFAULT_CLI_DIR, HELLO_TIMEOUT, HelperPackages, PaneHook, RemoteHost, TerminalHook,
    UnaskedStart, bootstrap, helper_protocol_refusal, lock_recover,
};

/// `wsl.exe` calls this process runs at once, across every distribution: a
/// running check, or a link's child until its helper starts. A call that
/// finds none free within [`CALL_ADMISSION`] is a failed attempt
/// (engineering rule 15). `wsl.exe` serializes much of its work in one
/// service, so more would only queue there, where no deadline reaches.
pub const MAX_CALLS: usize = 4;
const CALL_ADMISSION: Duration = Duration::from_secs(10);

/// How long `wsl.exe --list --running` may take.
const RUNNING_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the account probe may take: it may be what starts the
/// distribution, and a cold start of WSL's virtual machine takes seconds.
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the bridged Herdr has to answer before the kit is asked to
/// start it.
const HERDR_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the kit pass a connection runs before its services may take.
const KIT_TIMEOUT: Duration = Duration::from_secs(180);

/// How this process reaches WSL: the `wsl.exe` it runs and where it reads
/// the distributions; a test's stand-in for both on a system without WSL.
#[derive(Clone)]
pub struct WslAccess {
    program: PathBuf,
    distributions: Arc<dyn Fn() -> std::io::Result<Vec<Distribution>> + Send + Sync>,
}

impl WslAccess {
    /// This PC's WSL: `wsl.exe` in the system folder, the distributions in
    /// the account's registry. `None` where there is none (any system but
    /// Windows, or a Windows without WSL).
    pub fn of_this_pc() -> Option<Self> {
        let program = hide_platform::wsl::program().ok()?;
        Some(Self {
            program,
            distributions: Arc::new(hide_platform::wsl::distributions),
        })
    }

    /// `program` in place of `wsl.exe`, and `distributions` in place of the
    /// registry.
    pub fn with(
        program: PathBuf,
        distributions: Arc<dyn Fn() -> std::io::Result<Vec<Distribution>> + Send + Sync>,
    ) -> Self {
        Self {
            program,
            distributions,
        }
    }

    /// The distributions Add device lists; starts no process.
    pub fn listing(&self) -> WslListing {
        let found = (self.distributions)()
            .map_err(|error| format!("This PC's WSL distributions could not be read: {error}"))?;
        Ok(found
            .into_iter()
            .filter(|found| hide_platform::wsl::valid_distribution_name(&found.name))
            .map(|found| WslDistribution {
                setup_unfinished: !found.installed || found.first_run_pending,
                wsl_version: if found.wsl_version == 2 { 2 } else { 1 },
                default: found.default,
                name: found.name,
            })
            .collect())
    }

    fn distribution(&self, name: &str) -> Result<Distribution, EstablishError> {
        let found = (self.distributions)().map_err(|error| {
            EstablishError::Helper(format!(
                "This PC's WSL distributions could not be read: {error}"
            ))
        })?;
        found
            .into_iter()
            .find(|found| found.name == name)
            .ok_or_else(|| {
                EstablishError::Unsupported(format!(
                    "The WSL distribution {name} is not installed on this PC"
                ))
            })
    }
}

/// One WSL distribution reached through `wsl.exe`.
pub struct WslDevice {
    distro: String,
    access: WslAccess,
    packages: HelperPackages,
    panes: Option<PaneHook>,
    terminals: Option<TerminalHook>,
    /// The link the latest attempt started, which carries the device's
    /// Herdr; reaching Herdr never starts `wsl.exe`.
    link: Arc<Mutex<Option<RemoteHost>>>,
    /// The Herdr socket inside WSL the node bridges, as its Hello said.
    herdr_socket: Arc<Mutex<Option<String>>>,
}

impl WslDevice {
    pub(crate) fn new(
        distro: &str,
        access: WslAccess,
        packages: HelperPackages,
        panes: Option<PaneHook>,
        terminals: Option<TerminalHook>,
    ) -> Result<Self, String> {
        if !hide_platform::wsl::valid_distribution_name(distro) {
            return Err(format!("{distro:?} is not a WSL distribution name"));
        }
        Ok(Self {
            distro: distro.to_owned(),
            access,
            packages,
            panes,
            terminals,
            link: Arc::default(),
            herdr_socket: Arc::default(),
        })
    }

    fn target(&self) -> String {
        format!("wsl:{}", self.distro)
    }

    /// `wsl.exe -d <distro> --cd ~ -e <command…>`, as the distribution's
    /// default user.
    fn command(&self, command: &[&str]) -> Command {
        let mut wsl = Command::new(&self.access.program);
        // The helper and its scripts start in the Linux home, not in the
        // Windows folder this process runs in, which WSL would mount.
        wsl.args(["-d", &self.distro, "--cd", "~", "-e"])
            .args(command)
            // wsl.exe's own messages in UTF-8, as the link reads them.
            .env("WSL_UTF8", "1");
        wsl
    }

    /// Whether the distribution runs now; asking starts nothing.
    fn running(&self) -> Result<bool, String> {
        let _admitted = CALLS
            .admit(Instant::now() + CALL_ADMISSION)
            .ok_or_else(|| {
                format!(
                    "WSL is busy with {MAX_CALLS} other calls from Hide; its state was not read"
                )
            })?;
        hide_platform::wsl::running_from(&self.access.program, RUNNING_TIMEOUT)
            .map(|running| running.contains(&self.distro))
            .map_err(|error| error.to_string())
    }

    /// Why the start script stopped before the helper started, with the
    /// last thing `wsl.exe` said.
    fn stopped(pipe: &UnaskedStart, stop: bootstrap::Stop) -> String {
        let reason = match stop {
            bootstrap::Stop::Quiet => "WSL did not answer the account probe in time".to_owned(),
            bootstrap::Stop::Ended(reason) => format!("WSL ended the connection: {reason}"),
        };
        match pipe.last_error() {
            Some(line) => format!("{reason}: {line}"),
            None => reason,
        }
    }
}

/// [`MAX_CALLS`], shared by every distribution of this process.
static CALLS: CallGate = CallGate {
    running: Mutex::new(0),
    freed: std::sync::Condvar::new(),
};

struct CallGate {
    running: Mutex<usize>,
    freed: std::sync::Condvar,
}

/// One admitted call; its end frees the gate's slot.
struct CallSlot(&'static CallGate);

impl CallGate {
    fn admit(&'static self, deadline: Instant) -> Option<CallSlot> {
        let mut running = lock_recover(&self.running);
        while *running >= MAX_CALLS {
            let left = deadline.checked_duration_since(Instant::now())?;
            running = self
                .freed
                .wait_timeout(running, left)
                .map(|(guard, _)| guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner().0);
        }
        *running += 1;
        Some(CallSlot(self))
    }
}

impl Drop for CallSlot {
    fn drop(&mut self) {
        *lock_recover(&self.0.running) -= 1;
        self.0.freed.notify_one();
    }
}

/// The identity a consent binds on a distribution: its default Linux user,
/// its name and the id WSL gave it when it was installed, so installing it
/// again or changing its default user asks for consent again (B28).
fn identity_of(distribution: &Distribution, user: &str) -> HostIdentity {
    HostIdentity {
        user: user.to_owned(),
        hostname: distribution.name.clone(),
        port: 0,
        machine_id: Some(format!("wsl:{}", distribution.id)),
        host_key_sha256: String::new(),
    }
}

/// Whether a consent bound to `bound` covers the distribution that answered
/// as `observed`. A consent bound before contract 4 named the install id in
/// its host key field, so it still names the install it was given for.
fn covers(bound: &HostIdentity, observed: &HostIdentity) -> bool {
    let machine = bound
        .machine_id
        .as_deref()
        .or_else(|| Some(bound.host_key_sha256.as_str()).filter(|key| key.starts_with("wsl:")));
    bound.user == observed.user
        && bound.hostname == observed.hostname
        && bound.port == observed.port
        && machine.is_some()
        && machine == observed.machine_id.as_deref()
}

impl DeviceTransport for WslDevice {
    fn herdr_api_connector(&self) -> Arc<dyn hide_herdr_client::ApiConnector> {
        Arc::new(WslHerdr(Arc::clone(&self.link)))
    }

    fn cached_herdr_version(&self) -> Option<String> {
        None
    }

    fn cached_herdr_socket(&self) -> Option<String> {
        lock_recover(&self.herdr_socket).clone()
    }
}

/// The device's Herdr through its current link; no link, no Herdr, and
/// nothing is started to find one.
struct WslHerdr(Arc<Mutex<Option<RemoteHost>>>);

impl hide_herdr_client::ApiConnector for WslHerdr {
    fn connect(&self) -> Result<Box<dyn hide_herdr_client::ApiStream>, ApiError> {
        let link = lock_recover(&self.0).clone().ok_or_else(|| {
            ApiError::Transport("The WSL distribution's helper is not connected".to_owned())
        })?;
        link.herdr_connector().connect()
    }
}

impl DialedTransport for WslDevice {
    fn capability_test(&self, operation_id: &str, check: SnapshotCheck<'_>) -> CapabilityReport {
        let mut report = CapabilityReport::new(
            operation_id,
            RemoteHostIdentity {
                host_id: self.target(),
                alias: self.distro.clone(),
                hostname: self.distro.clone(),
                port: 0,
            },
        );
        // The test reads what is connected; it never starts WSL.
        match self.access.distribution(&self.distro) {
            Ok(found) if found.wsl_version == 2 => {
                report.pass(RemoteStage::Ssh, "WSL 2");
            }
            Ok(_) => {
                report.fail(
                    RemoteStage::Ssh,
                    "The distribution runs on WSL 1",
                    false,
                    true,
                );
                return report;
            }
            Err(error) => {
                report.fail(RemoteStage::Ssh, error.to_string(), false, true);
                return report;
            }
        }
        report.pass(RemoteStage::Auth, "this PC's account");
        let link = lock_recover(&self.link).clone();
        let Some(link) = link.filter(|link| link.closed_reason().is_none()) else {
            report.fail(
                RemoteStage::Herdr,
                "The distribution's helper is not connected",
                true,
                false,
            );
            return report;
        };
        let socket = self.cached_herdr_socket().unwrap_or_default();
        let snapshot = hide_herdr_client::request_with_connector(
            &link.herdr_connector(),
            "session.snapshot",
            json!({}),
            Duration::from_secs(10),
        );
        match snapshot
            .map_err(|error| error.to_string())
            .and_then(|value| {
                check(&value, &self.target(), &socket).map_err(|error| error.to_string())
            }) {
            Ok(protocol) => {
                report.pass(RemoteStage::Herdr, socket.clone());
                report.pass(RemoteStage::Protocol, format!("protocol {protocol}"));
            }
            Err(reason) => {
                report.fail(RemoteStage::Herdr, reason, true, false);
                return report;
            }
        }
        report.pass(RemoteStage::Pty, "through the node link");
        if !super::check_git(&link, &mut report) {
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
        let Attempt {
            wake,
            cancel,
            on_close,
            ..
        } = attempt;
        let since = Instant::now();
        let target = self.target();
        let stage = |name: &str| super::establish_stage(&target, name, since);
        stage("distribution");
        let distribution = self.access.distribution(&self.distro)?;
        if distribution.wsl_version != 2 {
            return Err(EstablishError::Unsupported(format!(
                "{} runs on WSL 1; convert it with `wsl --set-version {} 2` to connect it",
                self.distro, self.distro
            )));
        }
        if !distribution.installed || distribution.first_run_pending {
            return Err(EstablishError::Unsupported(format!(
                "Finish setting up {} first: open it once and create its user",
                self.distro
            )));
        }
        if wake == Wake::NotAllowed {
            stage("running");
            if !self.running().map_err(EstablishError::Helper)? {
                return Err(EstablishError::Stopped(format!(
                    "{} is off; it starts again when Hide opens or you reconnect it",
                    self.distro
                )));
            }
        }
        // The slot is held until the helper has started in place of the
        // start script.
        let admitted = CALLS
            .admit(Instant::now() + CALL_ADMISSION)
            .ok_or_else(|| {
                EstablishError::Helper(format!(
                    "WSL is busy with {MAX_CALLS} other calls from Hide; the connection was not started"
                ))
            })?;
        stage("probe");
        let mut command = self.command(&bootstrap::WRAPPER);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = hide_platform::process::OwnedChild::spawn(&mut command)
            .map_err(|error| EstablishError::Helper(format!("wsl.exe could not start: {error}")))?;
        let link = ChildLink::start(&target, child, Some(&cancel)).map_err(|error| {
            EstablishError::Helper(format!("The distribution's link could not start: {error}"))
        })?;
        let mut pipe = UnaskedStart::new(link, cancel.clone());
        // Until the helper starts, every way out ends and reaps `wsl.exe`
        // before the attempt returns (PRD system-ssh-device-link B26).
        let started = (|pipe: &mut UnaskedStart| -> Result<(bootstrap::Installed, HostIdentity), EstablishError> {
            let facts = match bootstrap::probe(&mut *pipe, Instant::now() + PROBE_TIMEOUT) {
                Ok(facts) => facts,
                Err(bootstrap::ProbeError::Unsupported(reason)) => {
                    return Err(EstablishError::Unsupported(reason));
                }
                Err(bootstrap::ProbeError::Stopped(stop)) => {
                    return Err(EstablishError::Helper(Self::stopped(pipe, stop)));
                }
            };
            if facts.os != "linux" {
                return Err(EstablishError::Unsupported(format!(
                    "The distribution reported {}, not Linux",
                    facts.os
                )));
            }
            let identity = identity_of(&distribution, &facts.user);
            if let Some(bound) = consent.identity.as_ref()
                && !covers(bound, &identity)
            {
                return Err(EstablishError::IdentityChanged {
                    bound: Box::new(bound.clone()),
                    observed: Box::new(identity),
                });
            }
            let payload = self.packages.payload(&facts.os, &facts.arch)?;
            stage("install");
            let installed =
                bootstrap::install(&mut *pipe, &consent.helper_root, &facts.home, &payload)?;
            stage("start");
            bootstrap::start(
                &mut *pipe,
                &installed.helper_relative,
                &bootstrap::HerdrChoice::Account,
            )?;
            Ok((installed, identity))
        })(&mut pipe);
        let (installed, identity) = match started {
            Ok(started) => started,
            Err(error) => {
                pipe.link.kill();
                return Err(if cancel.is_cancelled() {
                    EstablishError::Helper(super::ATTEMPT_ENDED.to_owned())
                } else {
                    error
                });
            }
        };
        drop(admitted);
        let host = pipe
            .link
            .into_host(Vec::new(), self.panes.clone(), on_close);
        stage("hello");
        let hello: Hello = call_as(&host, Call::Hello, HELLO_TIMEOUT).map_err(|error| {
            host.close("the helper did not answer Hello");
            EstablishError::Helper(format!("The WSL helper did not start: {error}"))
        })?;
        if let Some(reason) = helper_protocol_refusal(&hello) {
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
            .ok_or_else(|| "the WSL helper bridges no Herdr socket".to_owned());
        *lock_recover(&self.herdr_socket) = socket.clone().ok();
        *lock_recover(&self.link) = Some(host.clone());
        // Only a Herdr that does not answer needs the kit before the
        // services start; otherwise the core runs its pass after ready.
        let herdr_answers = hide_herdr_client::request_with_connector(
            &host.herdr_connector(),
            "session.snapshot",
            json!({}),
            HERDR_CHECK_TIMEOUT,
        )
        .is_ok();
        let kit = (!herdr_answers).then(|| {
            stage("kit");
            call_as::<hide_kit::KitReport>(
                &host,
                Call::Kit {
                    action: KitAction::Apply,
                    cli_dir: consent
                        .cli_dir
                        .clone()
                        .unwrap_or_else(|| DEFAULT_CLI_DIR.to_owned()),
                    herdr_socket: socket.clone().ok(),
                    preserve_herdr_server: false,
                    retirement_projects: Vec::new(),
                },
                KIT_TIMEOUT,
            )
            .map(Box::new)
            .map_err(|error| error.to_string())
        });
        if self.panes.is_some() {
            stage("panes");
            super::start_panes_at(socket.clone(), &host);
        }
        let terminals = match self.terminals.clone() {
            None => Err("this process takes no device terminals".to_owned()),
            Some(hook) => {
                stage("terminals");
                super::start_terminals_at(socket, &host, hook)
            }
        };
        // The link is the core's from here (see the SSH device's).
        cancel.disarm();
        if cancel.is_cancelled() {
            host.close(super::ATTEMPT_ENDED);
            return Err(EstablishError::Helper(super::ATTEMPT_ENDED.to_owned()));
        }
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

#[cfg(all(test, unix))]
#[path = "wsl_tests.rs"]
mod tests;
