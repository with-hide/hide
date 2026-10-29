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
//! A one-shot `wsl.exe` runs only on a connection attempt's worker, off the
//! runtime lock: the running check, the account probe and the install. Each
//! has a deadline, at most [`MAX_CALLS`] run at once in this process, and
//! one that does not answer in time is a failed attempt, never "no WSL"
//! (B25). Before any of them, the distribution's registry entry, read
//! without starting WSL, says whether it exists, which WSL runs it and
//! whether its first-run setup finished; and only an attempt that may wake
//! a distribution ([`Wake`]) runs anything in one that is off (D-11).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hide_herdr_client::ApiError;

use hide_node_link::call_as;
use hide_node_link::device::{
    CapabilityReport, DeviceTransport, DialedTransport, EstablishError, Established, HostConsent,
    HostIdentity, NodeReady, OnClose, RemoteHostIdentity, RemoteStage, SnapshotCheck, Upload, Wake,
    WslDistribution, WslListing,
};
use hide_node_link::protocol::{Call, Hello, KitAction};
use hide_platform::wsl::Distribution;
use serde_json::json;

use super::{
    DEFAULT_CLI_DIR, HELLO_TIMEOUT, HelperPackages, Package, PaneHook, Payload, RemoteHost,
    TerminalHook, helper_protocol_refusal, lock_recover, platform_of,
};

/// One-shot `wsl.exe` calls this process runs at once, across every
/// distribution; a call that finds none free within [`CALL_ADMISSION`] is
/// a failed attempt (engineering rule 15). `wsl.exe` serializes much of its
/// work in one service, so more would only queue there, where no deadline
/// reaches.
pub const MAX_CALLS: usize = 4;
const CALL_ADMISSION: Duration = Duration::from_secs(10);

/// How long `wsl.exe --list --running` may take.
const RUNNING_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the account probe may take: it may be what starts the
/// distribution, and a cold start of WSL's virtual machine takes seconds.
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the whole install may take, every file together.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(180);
/// The most a probe or an install step may write back.
const CALL_OUTPUT_LIMIT: usize = 64 * 1024;
/// How long the bridged Herdr has to answer before the kit is asked to
/// start it.
const HERDR_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the kit pass a connection runs before its services may take.
const KIT_TIMEOUT: Duration = Duration::from_secs(180);

/// The account probe, as the distribution's default user: who it is, its
/// home, its platform, and whether it can check an upload's digest. One
/// line, so no Windows argument rule has a newline to read.
const PROBE_SCRIPT: &str = "id -un; id -u; printf '%s\\n' \"$HOME\"; uname -s -m; command -v sha256sum >/dev/null && echo sha256sum || echo none";

/// Prints the relative path of each `<path> <digest>` pair under `$1` that
/// already holds those bytes as a file the account owns, so a connection
/// sends only what is missing and an install that stopped part way resumes
/// with what it had not sent (B26).
const HELD_SCRIPT: &str = "set -u; root=$1; shift; held() { [ -f \"$1\" ] && [ ! -L \"$1\" ] && [ -O \"$1\" ] || return 1; set -- \"$2\" $(sha256sum < \"$1\"); [ \"$1\" = \"$2\" ]; }; while [ $# -ge 2 ]; do if held \"$root/$1\" \"$2\"; then printf '%s\\n' \"$1\"; fi; shift 2; done";

/// Puts its standard input at `$1/$2` with mode `$4`: written beside the
/// final name in a folder only the account can enter, checked against the
/// digest `$3`, and renamed into place, so the final name only ever holds a
/// whole, verified copy and an interrupted upload leaves nothing in use.
const PLACE_SCRIPT: &str = "set -eu; umask 077; root=$1; final=$1/$2; digest=$3; mode=$4; dir=${final%/*}; mkdir -p -- \"$dir\"; chmod -- 700 \"$root\"; tmp=$dir/.upload-${final##*/}-$$; trap 'rm -f -- \"$tmp\"' EXIT; cat > \"$tmp\"; set -- $(sha256sum < \"$tmp\"); [ \"$1\" = \"$digest\" ] || { echo 'the uploaded file did not match the package' >&2; exit 66; }; chmod -- \"$mode\" \"$tmp\"; mv -f -- \"$tmp\" \"$final\"; trap - EXIT";

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

    /// `wsl.exe -d <distro> [-u <user>] --cd ~ -e <command…>`.
    fn command(&self, user: Option<&str>, command: &[&str]) -> Command {
        let mut wsl = Command::new(&self.access.program);
        wsl.args(["-d", &self.distro]);
        if let Some(user) = user {
            wsl.args(["-u", user]);
        }
        // The helper and its scripts start in the Linux home, not in the
        // Windows folder this process runs in, which WSL would mount.
        wsl.args(["--cd", "~", "-e"]).args(command);
        wsl
    }

    /// Runs one `wsl.exe` to its end within `deadline`, with `input` on its
    /// standard input; answers its standard output. Admitted among
    /// [`MAX_CALLS`].
    fn call(
        &self,
        what: &str,
        user: Option<&str>,
        command: &[&str],
        input: Option<Vec<u8>>,
        deadline: Instant,
    ) -> Result<String, String> {
        let _admitted = CALLS
            .admit(Instant::now() + CALL_ADMISSION)
            .ok_or_else(|| {
                format!(
                    "WSL is busy with {MAX_CALLS} other calls from Hide; {what} was not started"
                )
            })?;
        let mut command = self.command(user, command);
        command
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = hide_platform::process::OwnedChild::spawn(&mut command)
            .map_err(|error| format!("wsl.exe could not start for {what}: {error}"))?;
        // The input goes from its own thread, so a child that writes while
        // it reads never waits on this one. It ends when the pipe closes.
        let writing = match (input, child.take_stdin()) {
            (Some(bytes), Some(mut stdin)) => Some(
                std::thread::Builder::new()
                    .name("wsl-call-input".into())
                    .spawn(move || stdin.write_all(&bytes).and_then(|()| stdin.flush()))
                    .map_err(|error| format!("{what} could not send its input: {error}"))?,
            ),
            _ => None,
        };
        let captured = child.capture_until(deadline, CALL_OUTPUT_LIMIT);
        if let Some(writing) = writing {
            // The child has ended, so the pipe is closed and the write over.
            let _ = writing.join();
        }
        let output = captured.map_err(|error| match error.kind {
            hide_platform::process::CaptureFailureKind::Deadline => {
                format!("WSL did not answer {what} in time")
            }
            _ => format!("{what} failed: {error}"),
        })?;
        if !output.status.success() {
            let stderr = hide_platform::wsl::decode(&output.stderr);
            let line = stderr.lines().rev().find(|line| !line.trim().is_empty());
            return Err(format!(
                "{what} failed ({}){}",
                output.status,
                line.map(|line| format!(": {}", line.trim()))
                    .unwrap_or_default()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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

    fn probe(&self) -> Result<Probe, EstablishError> {
        let answer = self
            .call(
                "the account probe",
                None,
                &["sh", "-c", PROBE_SCRIPT],
                None,
                Instant::now() + PROBE_TIMEOUT,
            )
            .map_err(EstablishError::Helper)?;
        Probe::read(&answer).map_err(EstablishError::Unsupported)
    }

    /// Puts `payload` in its version folder under `root`, sending only the
    /// files that are not already there whole (see [`HELD_SCRIPT`]).
    fn install(
        &self,
        user: &str,
        root: &str,
        payload: &Payload,
    ) -> Result<(String, bool, Upload), EstablishError> {
        let deadline = Instant::now() + INSTALL_TIMEOUT;
        let version_dir = format!("{root}/{}", &payload.version()[..16]);
        let mut files: Vec<(String, &Package)> = payload
            .files
            .iter()
            .map(|file| {
                (
                    format!("{}/{}", &payload.version()[..16], file.relative),
                    file,
                )
            })
            .collect();
        if let Some(herdr) = &payload.herdr {
            files.push((
                format!(
                    "{}/{}/{}",
                    hide_kit::device_herdr::HERDR_DIR,
                    super::herdr_copy_name(herdr),
                    herdr.relative
                ),
                herdr,
            ));
        }
        let mut held_args = vec!["sh", "-c", HELD_SCRIPT, "sh", root];
        for (relative, file) in &files {
            held_args.push(relative);
            held_args.push(&file.digest);
        }
        let held = self
            .call("the install check", Some(user), &held_args, None, deadline)
            .map_err(EstablishError::Install)?;
        let held: std::collections::BTreeSet<&str> = held.lines().collect();
        let mut upload = Upload {
            missing: payload.missing.clone(),
            ..Upload::default()
        };
        let mut fresh = false;
        for (index, (relative, file)) in files.iter().enumerate() {
            if held.contains(relative.as_str()) {
                upload.reused += 1;
                continue;
            }
            let mode = if file.executable { "700" } else { "600" };
            let placed = self.call(
                &format!("the upload of {}", file.relative),
                Some(user),
                &[
                    "sh",
                    "-c",
                    PLACE_SCRIPT,
                    "sh",
                    root,
                    relative,
                    &file.digest,
                    mode,
                ],
                Some(file.bytes.clone()),
                deadline,
            );
            match placed {
                Ok(_) => {
                    upload.sent += 1;
                    fresh |= index == 0;
                }
                // The helper, a chosen Herdr and its pin must arrive before
                // the helper starts; an optional kit part the kit reports.
                Err(reason)
                    if index == 0
                        || file.relative == hide_kit::device_herdr::HERDR_PIN
                        || file.relative == hide_kit::device_herdr::HERDR_PROGRAM =>
                {
                    return Err(EstablishError::Install(reason));
                }
                Err(reason) => upload.missing.push(reason),
            }
        }
        let helper = payload
            .files
            .first()
            .ok_or_else(|| EstablishError::Install("The build carries no helper".to_owned()))?;
        Ok((format!("{version_dir}/{}", helper.relative), fresh, upload))
    }
}

/// What the account probe answered.
#[derive(Debug, Eq, PartialEq)]
struct Probe {
    user: String,
    home: String,
    os: String,
    arch: String,
}

impl Probe {
    fn read(answer: &str) -> Result<Self, String> {
        let lines: Vec<&str> = answer.lines().map(str::trim_end).collect();
        let [user, _uid, home, uname, digest] = lines[..] else {
            return Err(format!(
                "The distribution answered an unreadable probe: {answer:?}"
            ));
        };
        if !hide_platform::wsl::valid_user_name(user) {
            return Err(format!(
                "The distribution's default user {user:?} is not a name Hide can run as"
            ));
        }
        if !home.starts_with('/') || home.chars().any(char::is_control) {
            return Err("The distribution's default user has no usable home folder".to_owned());
        }
        if digest != "sha256sum" {
            return Err(
                "The distribution has no sha256sum, which Hide checks its install with".to_owned(),
            );
        }
        let (os, arch) = platform_of(uname)?;
        if os != "linux" {
            return Err(format!("The distribution reported {os}, not Linux"));
        }
        Ok(Self {
            user: user.to_owned(),
            home: home.to_owned(),
            os,
            arch,
        })
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
        host_key_sha256: format!("wsl:{}", distribution.id),
    }
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

    /// A distribution's link is its only connection: `wsl.exe` and the
    /// helper inside it end with it (PRD wsl-device B27).
    fn close(&self) {
        if let Some(link) = lock_recover(&self.link).as_ref() {
            link.close("the core let go of the distribution");
        }
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
        for stage in [
            RemoteStage::Pty,
            RemoteStage::Sftp,
            RemoteStage::Git,
            RemoteStage::Tunnel,
        ] {
            report.pass(stage, "through the node link");
        }
        report.connected();
        report
    }

    fn establish(
        &self,
        consent: &HostConsent,
        _retirement_projects: &[String],
        wake: Wake,
        on_close: OnClose,
    ) -> Result<Established, EstablishError> {
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
        stage("probe");
        let probe = self.probe()?;
        let identity = identity_of(&distribution, &probe.user);
        if let Some(bound) = consent.identity.as_ref()
            && bound != &identity
        {
            return Err(EstablishError::IdentityChanged {
                bound: Box::new(bound.clone()),
                observed: Box::new(identity),
            });
        }
        let payload = self.packages.payload(&probe.os, &probe.arch)?;
        let root = match consent.helper_root.strip_prefix("~/") {
            Some(rest) => format!("{}/{rest}", probe.home.trim_end_matches('/')),
            None if consent.helper_root.starts_with('/') => consent.helper_root.clone(),
            None => {
                return Err(EstablishError::Install(format!(
                    "The helper install root {:?} must be absolute or start with ~/",
                    consent.helper_root
                )));
            }
        };
        if root.split('/').any(|part| part == ".." || part == ".")
            || root.chars().any(char::is_control)
        {
            return Err(EstablishError::Install(
                "The helper install root is not a plain path".to_owned(),
            ));
        }
        stage("install");
        let (helper_path, installed, upload) = self.install(&probe.user, &root, &payload)?;
        stage("start");
        let mut command = self.command(
            Some(&probe.user),
            &[&helper_path, "node", "serve", "--own-herdr"],
        );
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = hide_platform::process::OwnedChild::spawn(&mut command).map_err(|error| {
            EstablishError::Helper(format!("wsl.exe could not start the helper: {error}"))
        })?;
        let host =
            super::over_child(&target, child, self.panes.clone(), on_close).map_err(|error| {
                EstablishError::Helper(format!("The helper's link could not start: {error}"))
            })?;
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
        stage("ready");
        Ok(Established {
            node: NodeReady {
                host: Arc::new(host),
                hello,
                terminals,
            },
            identity,
            installed,
            helper_path,
            upload,
            kit,
        })
    }
}

#[cfg(all(test, unix))]
#[path = "wsl_tests.rs"]
mod tests;
