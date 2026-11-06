//! A Windows core dials its devices over Windows' own OpenSSH
//! (with-hide/hide-private#501 N1), as the Windows app does: the core's
//! connector, `ssh.exe` from `System32\OpenSSH` and the build folder laid out
//! as the Windows package lays out its resources (`hided.exe`, `hide.exe`,
//! `hide-agent-hooks.exe`, `hide-holder.exe` and the holder's ConPTY runtime
//! in `holder-conpty`), from which the core finds its own programs.
//!
//! The first walk reaches this runner's own OpenSSH Server as a Windows
//! device; the second reaches an `sshd` inside the runner's WSL 2
//! distribution as a Linux device, with a key whose passphrase Hide's
//! askpass asks once. Each installs the device's node, runs a pane on the
//! core's own engine and types into its terminal, reads a file and the Git
//! changes of a project, and ends with nothing of the node left running.
//!
//! Each walk ends what it started on the device whichever way the test
//! ends ([`Walk`]): a holder outlives a lost link by design, so a walk that
//! failed partway would otherwise leave the device's node, holders and
//! shells running on the runner.
//!
//! The `windows wsl` lane sets up what only a disposable runner may have and
//! runs both: the OpenSSH Server it installs, with a key it authorizes for
//! the runner's account (`HIDE_E2E_WINDOWS_SSH_DIR`: `client`, `known_hosts`
//! and the server's `port`), and the WSL distribution its other walks own
//! (`HIDE_E2E_WSL_DISTRO`, its Linux kit in `HIDE_E2E_WSL_PACKAGES`). Both
//! need this build's CLI binaries (`scripts/verify-cargo.sh cli`) and the
//! pinned ConPTY runtime (`HIDE_HOLDER_CONPTY`).
#![cfg(windows)]

use std::collections::BTreeMap;
use std::io::Read;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hide_node_link::device::{
    Answer, Attempt, Cancel, DeviceConnector, Established, HOST_CONSENT_CONTRACT, HostConsent,
    Refusal, Secret, SignIn, SignInWait, Wake,
};
use hide_node_link::holders::{HolderStart, PaneHolders};
use hide_node_link::link_panes::LinkPanes;
use hide_node_link::protocol::{Call, KitAction, RootOpened, RootRef};
use hide_node_link::terminal::{GridSize, KeyTarget, TerminalControl, TerminalReport};
use hide_node_link::{NodeLink, call_as};
use serde_json::Value;

use super::wsl_device::as_root;

const TIMEOUT: Duration = Duration::from_secs(120);
/// Windows' own OpenSSH, which a person's PC has; the `ssh` first on a Git
/// Bash `PATH` is Git's.
const OPENSSH: &str = r"C:\Windows\System32\OpenSSH";
/// The account the Linux walk signs in as inside the distribution, apart
/// from the WSL walks' own.
const SSH_USER: &str = "hidessh";
/// The port the distribution's `sshd` listens on, which WSL forwards from
/// the runner's loopback.
const WSL_SSH_PORT: u16 = 47622;
const PASSPHRASE: &str = "fixture passphrase";
/// The programs a Windows package carries in its resources
/// (`desktop/scripts/package.mjs`).
const PROGRAMS: [&str; 4] = ["hided", "hide", "hide-agent-hooks", "hide-holder"];

#[derive(Default)]
struct Sink {
    output: Mutex<Vec<u8>>,
    reports: Mutex<Vec<TerminalReport>>,
}

impl hide_node::terminal::device::DeviceSink for Sink {
    fn output(&self, _: &str, _: &str, bytes: &[u8], _: bool) {
        self.output.lock().unwrap().extend_from_slice(bytes);
    }
    fn report(&self, _: &str, report: TerminalReport) {
        self.reports.lock().unwrap().push(report);
    }
}

/// The device row's side of an attempt: answers each question with the
/// next answer and keeps what it was asked.
#[derive(Default)]
struct Row {
    answers: Mutex<Vec<Answer>>,
    asked: Mutex<Vec<SignInWait>>,
}

impl Row {
    fn answering(answers: Vec<Answer>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into_iter().rev().collect()),
            ..Self::default()
        })
    }

    fn asked(&self) -> Vec<SignInWait> {
        self.asked.lock().unwrap().clone()
    }
}

impl SignIn for Row {
    fn show(&self, _: Option<SignInWait>) {}

    fn ask(&self, wait: SignInWait) -> Result<Answer, Refusal> {
        self.asked.lock().unwrap().push(wait);
        self.answers.lock().unwrap().pop().ok_or(Refusal::Cancelled)
    }
}

#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait_for<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(value) = ready() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn variable(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")))
}

/// A path as an ssh config spells it on Windows.
fn config_path(path: &Path) -> String {
    format!("\"{}\"", path.display().to_string().replace('\\', "/"))
}

/// Copies the folder `from` to `to`, files and folders alike.
fn copy_folder(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_folder(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The core's build folder in `folder`, laid out as the Windows package
/// lays out its resources: this build's programs under their `.exe` names
/// and the pinned ConPTY runtime in `holder-conpty`.
fn own_package(folder: &Path) {
    let built = Path::new(env!("CARGO_BIN_EXE_hided")).parent().unwrap();
    std::fs::create_dir_all(folder).unwrap();
    for name in PROGRAMS {
        let program = format!("{name}.exe");
        let source = built.join(&program);
        assert!(
            source.is_file(),
            "build this worktree's CLI binaries first (scripts/verify-cargo.sh cli): {}",
            source.display()
        );
        std::fs::copy(source, folder.join(program)).unwrap();
    }
    copy_folder(
        &variable("HIDE_HOLDER_CONPTY"),
        &folder.join(hide_kit::holders::CONPTY_SOURCE),
    );
}

/// The connector the Windows app builds over `packages`, with Windows' own
/// `ssh.exe` and the askpass `hide.exe` it finds there by itself.
fn connector(packages: &Path, sink: Arc<Sink>) -> hide_node::ssh::Connector {
    hide_node::ssh::Connector::new(Some(packages.to_path_buf()), hided::cli::RELEASE)
        .with_ssh_program(Path::new(OPENSSH).join("ssh.exe"))
        .with_terminals(sink)
        .with_pane_events(Arc::default())
}

fn establish(
    connector: &hide_node::ssh::Connector,
    local: &Path,
    alias: &str,
    sign_in: Arc<dyn SignIn>,
) -> Established {
    let transport = connector.transport(local, alias, alias).unwrap();
    transport
        .establish(
            &HostConsent {
                contract: HOST_CONSENT_CONTRACT,
                helper_root: "~/.hide/host-helper".to_owned(),
                cli_dir: Some("~/.local/bin".to_owned()),
                granted_at_unix_ms: 1,
                identity: None,
            },
            &[],
            Attempt {
                wake: Wake::Allowed,
                sign_in,
                cancel: Cancel::default(),
                on_close: Box::new(|_| {}),
            },
        )
        .unwrap_or_else(|error| panic!("the connection to {alias}: {error}"))
}

/// The files and Git of the project `project` (a wire path), which holds
/// `a.txt` committed as `hello` and changed since to `hello`, `more`.
fn files_and_git(link: &dyn NodeLink, project: &str) {
    let opened: RootOpened = call_as(
        link,
        Call::RootOpen {
            root: project.to_owned(),
        },
        TIMEOUT,
    )
    .unwrap_or_else(|error| panic!("open {project}: {error}"));
    let root = RootRef {
        path: project.to_owned(),
        identity: opened.identity,
    };
    let bytes: Value = call_as(
        link,
        Call::Bytes {
            root: root.clone(),
            path: "a.txt".to_owned(),
            offset: 0,
            length: 64,
        },
        TIMEOUT,
    )
    .unwrap();
    assert_eq!(bytes["data"], "aGVsbG8KbW9yZQo=", "{bytes}");
    let version: String = call_as(
        link,
        Call::Git {
            root: project.to_owned(),
            command: hide_node_link::git::GitCommand::Version,
        },
        TIMEOUT,
    )
    .unwrap();
    assert!(version.starts_with("git version"), "{version}");
    let changes: Value = call_as(
        link,
        Call::Changes {
            root,
            scope: String::new(),
            selected: Some("a.txt".to_owned()),
            committed: false,
            base: None,
            diffs: Vec::new(),
        },
        TIMEOUT,
    )
    .unwrap();
    assert!(changes.to_string().contains("a.txt"), "{changes}");
}

/// Starts the device's interactive shell as `pane` in `cwd` on the core's
/// own engine, types `line` into its terminal and waits for `answer`, which
/// `line` does not spell, so its echo is never taken for its output. The
/// pane is `walk`'s to end from its start on.
fn terminal(walk: &mut Walk<'_>, sink: &Sink, pane: &str, cwd: &str, line: &[u8], answer: &str) {
    start_pane(walk, sink, pane, cwd, Vec::new());
    typed(walk, sink, pane, line, |output| {
        output.contains(answer).then_some(())
    });
}

/// Starts the device's interactive shell as `pane` in `cwd`, with `env` over
/// the node's own variables, and waits until the core controls its terminal.
/// The pane is `walk`'s to end from its start on.
fn start_pane(walk: &mut Walk<'_>, sink: &Sink, pane: &str, cwd: &str, env: Vec<(String, String)>) {
    let panes = LinkPanes::new(
        Arc::clone(&walk.connected().node.host),
        Arc::new(|record| println!("pane record: {record}")),
    );
    let recovery = panes.recovered(Duration::from_secs(10)).unwrap();
    assert!(recovery.present.is_empty(), "{recovery:?}");
    // As the core's own engine starts a pane: a number, the pane's
    // reservation, its start, then the number retired.
    let start = HolderStart {
        pane: pane.to_owned(),
        program: None,
        cwd: cwd.to_owned(),
        env,
        rows: 24,
        cols: 80,
        notice: None,
    };
    let request = panes.request().unwrap();
    let reserved = panes.reserve(&start, None, request).unwrap();
    let (panes, _) = walk.pane.insert((panes, pane.to_owned()));
    panes.start_reserved(&start, reserved, request).unwrap();
    panes.retire(request).unwrap();
    let terminals = walk
        .connected()
        .node
        .terminals
        .as_ref()
        .expect("terminals ride the link");
    terminals.control(TerminalControl::Attach {
        pane: pane.to_owned(),
        size: Some(GridSize { rows: 24, cols: 80 }),
        manual: false,
    });
    wait_for("the pane's terminal", || {
        sink.reports
            .lock()
            .unwrap()
            .iter()
            .any(|report| matches!(report, TerminalReport::State { pane: reported, state } if reported == pane && state.state == "controlling"))
            .then_some(())
    });
}

/// Types `line` into `pane`'s terminal and waits until `until` finds what it
/// looks for in everything the terminals have shown.
fn typed<T>(
    walk: &Walk<'_>,
    sink: &Sink,
    pane: &str,
    line: &[u8],
    until: impl Fn(&str) -> Option<T>,
) -> T {
    let terminals = walk
        .connected()
        .node
        .terminals
        .as_ref()
        .expect("terminals ride the link");
    terminals.key(KeyTarget::Pane(pane.to_owned()), line.to_vec(), 1);
    wait_for("the typed line's output", || {
        until(&String::from_utf8_lossy(&sink.output.lock().unwrap()))
    })
}

/// The processes running a program file under `folder`: the device's node,
/// its holders and the ConPTY host each holder starts.
fn programs_running(folder: &Path) -> Result<Vec<u32>, String> {
    let mut running = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                folders.push(path);
            } else if path.extension().is_some_and(|extension| extension == "exe") {
                let pids = hide_platform::fs::identity::file_id(&path)
                    .and_then(|file| hide_platform::process::running(&file))
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                running.extend(pids);
            }
        }
    }
    Ok(running)
}

/// `git` in `project` with no setting of the account's, which must succeed.
fn git(project: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .current_dir(project)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(arguments)
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?}");
}

/// A key of the fixture's in `folder`, its passphrase `passphrase`, made by
/// Windows' own `ssh-keygen.exe`.
fn key(folder: &Path, name: &str, passphrase: &str) -> PathBuf {
    let key = folder.join(name);
    let status = Command::new(Path::new(OPENSSH).join("ssh-keygen.exe"))
        .args(["-q", "-t", "ed25519", "-C", name, "-N", passphrase, "-f"])
        .arg(&key)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    key
}

/// The device a walk reaches, as the test sees what runs there.
trait Device {
    /// What of the device's node, its holders and their shells runs, one
    /// line each; none once a walk has ended.
    fn running(&self) -> Result<Vec<String>, String>;
    /// Ends everything [`Device::running`] names.
    fn kill(&self) -> Result<(), String>;
}

/// What a walk started on its device, ended whichever way the test ends.
/// [`Walk::end`] ends the pane and the link and fails the test unless
/// nothing of the node runs on the device afterwards. A walk dropped
/// without it, because the test failed first, ends the same, then ends by
/// force what still runs after [`LEFT_AFTER`] and says what that was.
struct Walk<'a> {
    device: &'a dyn Device,
    connected: Option<Established>,
    pane: Option<(LinkPanes, String)>,
    ended: bool,
}

/// How long a dropped walk waits for the device to end its node, holders
/// and shells on their own.
const LEFT_AFTER: Duration = Duration::from_secs(30);

impl<'a> Walk<'a> {
    fn new(device: &'a dyn Device) -> Self {
        Self {
            device,
            connected: None,
            pane: None,
            ended: false,
        }
    }

    fn connect(&mut self, connected: Established) {
        self.connected = Some(connected);
    }

    fn connected(&self) -> &Established {
        self.connected.as_ref().expect("the walk is connected")
    }

    /// Ends the pane, then the link, the link even when the pane did not end.
    /// It looks at the device first, so what a holder started before the
    /// test's own look, such as a pane's shell, is known before it ends.
    fn close(&mut self) -> Result<(), String> {
        if let Err(error) = self.device.running() {
            eprintln!("what runs on the device before the walk ends is unknown: {error}");
        }
        let ended = match self.pane.take() {
            Some((panes, pane)) => panes
                .terminate(&pane)
                .map_err(|error| format!("{pane} did not end: {error}")),
            None => Ok(()),
        };
        if let Some(connected) = self.connected.take() {
            connected.node.host.close("test ended");
        }
        ended
    }

    fn end(mut self) {
        self.close().unwrap();
        wait_for("the device's node, holders and shells to end", || {
            self.device.running().unwrap().is_empty().then_some(())
        });
        self.ended = true;
    }

    /// What still runs on the device, after waiting up to `within` for none.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn left(&self, within: Duration) -> Result<Vec<String>, String> {
        let deadline = Instant::now() + within;
        loop {
            let running = self.device.running()?;
            if running.is_empty() || Instant::now() >= deadline {
                return Ok(running);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

impl Drop for Walk<'_> {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        if let Err(error) = self.close() {
            eprintln!("{error}");
        }
        match self.left(LEFT_AFTER) {
            Ok(left) if left.is_empty() => {}
            Ok(left) => {
                eprintln!("still running on the device, ended by force: {left:?}");
                if let Err(error) = self.device.kill() {
                    eprintln!("ending them failed: {error}");
                }
                match self.left(Duration::from_secs(10)) {
                    Ok(left) if left.is_empty() => {}
                    Ok(left) => eprintln!("left running on the device: {left:?}"),
                    Err(error) => eprintln!("what runs on the device is unknown: {error}"),
                }
            }
            Err(error) => eprintln!("what runs on the device is unknown: {error}"),
        }
        if !std::thread::panicking() {
            panic!("a walk must end with Walk::end");
        }
    }
}

/// The Windows device of the first walk, whose node, holders and ConPTY
/// hosts run from the fixture's device home. Each look also keeps every
/// process they started, such as a pane's shell, by its pid and start, so
/// one that outlives its holder is still seen.
struct WindowsDevice {
    home: PathBuf,
    seen: Mutex<BTreeMap<u32, u64>>,
}

impl WindowsDevice {
    fn new(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
            seen: Mutex::default(),
        }
    }

    /// The pids seen so far that still run the process first seen.
    fn alive(&self) -> Result<Vec<u32>, String> {
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for program in programs_running(&self.home)? {
            let started = hide_platform::process::descendants(program)
                .map_err(|error| format!("what {program} started: {error}"))?;
            for pid in std::iter::once(program).chain(started) {
                if let Ok(start) = hide_platform::process::start_time(pid) {
                    seen.entry(pid).or_insert(start);
                }
            }
        }
        Ok(seen
            .iter()
            .filter(|&(&pid, &start)| {
                hide_platform::process::start_time(pid).is_ok_and(|now| now == start)
            })
            .map(|(&pid, _)| pid)
            .collect())
    }
}

impl Device for WindowsDevice {
    fn running(&self) -> Result<Vec<String>, String> {
        Ok(self
            .alive()?
            .into_iter()
            .map(|pid| {
                let name = hide_platform::process::name_of(pid)
                    .unwrap_or_else(|error| format!("({error})"));
                format!("{pid} {name}")
            })
            .collect())
    }

    fn kill(&self) -> Result<(), String> {
        let failed: Vec<String> = self
            .alive()?
            .into_iter()
            .filter_map(|pid| {
                hide_platform::process::kill_tree(pid)
                    .err()
                    .map(|error| format!("{pid}: {error}"))
            })
            .collect();
        if failed.is_empty() {
            Ok(())
        } else {
            Err(failed.join(", "))
        }
    }
}

/// `script` as root in `distro`, its output when it succeeds; a failure is
/// the caller's to report, where [`as_root`] fails the test.
fn in_distro(distro: &str, script: &str) -> Result<String, String> {
    let output = Command::new("wsl.exe")
        .args(["-d", distro, "-u", "root", "-e", "sh", "-c", script])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("wsl.exe: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{script}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The Linux device of the second walk: the fixture's account, where the
/// node, its holders, a pane's shell and the link's ssh session run. The
/// account's systemd user manager, which logind ends on its own delay, is
/// not the node's.
struct LinuxDevice<'a> {
    distro: &'a str,
}

impl Device for LinuxDevice<'_> {
    fn running(&self) -> Result<Vec<String>, String> {
        let listed = in_distro(
            self.distro,
            &format!("pgrep -a -u {SSH_USER} -f 'node serve|hide-holder|bash|sshd' || true"),
        )?;
        Ok(listed.lines().map(str::to_owned).collect())
    }

    fn kill(&self) -> Result<(), String> {
        in_distro(self.distro, &format!("pkill -KILL -u {SSH_USER}; true")).map(drop)
    }
}

/// The fixture's account in the distribution, removed with everything it
/// runs whichever way the test ends.
struct Account<'a> {
    distro: &'a str,
}

impl Drop for Account<'_> {
    fn drop(&mut self) {
        if let Err(error) = in_distro(
            self.distro,
            &format!("pkill -KILL -u {SSH_USER}; userdel -r -f {SSH_USER} 2>/dev/null; true"),
        ) {
            eprintln!("the fixture's account was not removed: {error}");
        }
    }
}

/// The bytes of every file under `folder`: what a first connection uploads
/// when the folder holds the build for the device.
fn folder_bytes(folder: &Path) -> u64 {
    std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                folder_bytes(&entry.path())
            } else {
                entry.metadata().unwrap().len()
            }
        })
        .sum()
}

/// Prints what a first connection took to upload `bytes`: the files' own
/// size, what crossed the link for them and the time the core spent packing
/// them (with-hide/hide-private#582), and holds that both devices took the
/// build packed.
fn report_first(label: &str, took: Duration, connected: &Established, bytes: u64) {
    let mib = |bytes: u64| bytes as f64 / 1_048_576.0;
    let upload = &connected.upload;
    println!(
        "{label}: connected in {took:?}, upload={upload:?}, {:.1} MiB of files as {:.1} MiB on the link, packed in {} ms, {:.1} MiB/s of files overall",
        mib(upload.bytes),
        mib(upload.wire),
        upload.pack_ms,
        mib(upload.bytes) / took.as_secs_f64()
    );
    // The kit folder's size, which the runs before packing printed as the
    // upload's.
    println!("{label}: the kit folder holds {:.1} MiB", mib(bytes));
    assert!(
        upload.wire < upload.bytes,
        "the build was not sent packed: {upload:?}"
    );
}

/// Connects again once the first link ended. The device already holds this
/// build, so nothing is sent again, and what this connection takes is what
/// every later connection of the same build takes; it ends like the first.
fn reconnect(
    connector: &hide_node::ssh::Connector,
    local: &Path,
    alias: &str,
    sign_in: Arc<dyn SignIn>,
    device: &dyn Device,
) {
    let mut walk = Walk::new(device);
    let started = Instant::now();
    walk.connect(establish(connector, local, alias, sign_in));
    let connected = walk.connected();
    println!(
        "{alias}: reconnected in {:?}, upload={:?}",
        started.elapsed(),
        connected.upload
    );
    assert!(!connected.installed, "a reconnection installs nothing");
    assert_eq!(connected.upload.sent, 0, "{:?}", connected.upload);
    walk.end();
}

/// This runner's own OpenSSH Server as a Windows device: the core's build
/// folder, the core account's ssh config naming the server
/// `windows-device`, and a device home of the fixture's below the account's
/// (`HIDE_DEV_DEVICE_HOME`'s rule), so nothing reaches the runner account's
/// own Hide folders.
struct WindowsServer {
    _fixture: tempfile::TempDir,
    packages: PathBuf,
    local: PathBuf,
    run: tempfile::TempDir,
    run_name: String,
}

impl WindowsServer {
    fn new() -> Self {
        let ssh = variable("HIDE_E2E_WINDOWS_SSH_DIR");
        let fixture = tempfile::Builder::new().prefix("wsd").tempdir().unwrap();
        let packages = fixture.path().join("packages");
        own_package(&packages);
        let local = fixture.path().join("local");
        std::fs::create_dir_all(local.join(".ssh")).unwrap();
        std::fs::write(
            local.join(".ssh").join("config"),
            format!(
                "Host windows-device\n  HostName 127.0.0.1\n  Port {}\n  User {}\n  IdentityFile {}\n  IdentitiesOnly yes\n  IdentityAgent none\n  UserKnownHostsFile {}\n  GlobalKnownHostsFile {}\n",
                std::fs::read_to_string(ssh.join("port")).unwrap().trim(),
                std::env::var("USERNAME").unwrap(),
                config_path(&ssh.join("client")),
                config_path(&ssh.join("known_hosts")),
                config_path(&ssh.join("known_hosts")),
            ),
        )
        .unwrap();
        // The device's home of its own, below the account's home as the start
        // script requires; the start script makes `home` inside it.
        let profile = variable("USERPROFILE");
        let dev = profile.join(".hide-dev");
        std::fs::create_dir_all(&dev).unwrap();
        let run = tempfile::Builder::new()
            .prefix("wsd")
            .tempdir_in(&dev)
            .unwrap();
        let run_name = run.path().file_name().unwrap().to_str().unwrap().to_owned();
        Self {
            _fixture: fixture,
            packages,
            local,
            run,
            run_name,
        }
    }

    /// The connector the Windows app builds, reaching the device home.
    fn connector(&self, sink: Arc<Sink>) -> hide_node::ssh::Connector {
        connector(&self.packages, sink)
            .with_device_home(Some(format!("~/.hide-dev/{}/home", self.run_name)))
    }
}

/// c1: this runner's own OpenSSH Server as a Windows device of a Windows
/// core on its own engine. The connection installs the core's own
/// `hided.exe`, `hide-holder.exe` and `holder-conpty`, found beside it under
/// their Windows names, into a device home of the fixture's below the
/// account's (`HIDE_DEV_DEVICE_HOME`'s rule), so nothing reaches the
/// runner account's own Hide folders.
#[test]
#[ignore = "the runner's OpenSSH Server and a key it takes; the windows wsl lane sets them up"]
fn a_windows_core_connects_a_windows_device_over_its_own_openssh() {
    let server = WindowsServer::new();
    let run_name = server.run_name.clone();
    let local = server.local.clone();
    let device = WindowsDevice::new(server.run.path());

    let sink = Arc::new(Sink::default());
    let connector = server.connector(sink.clone());
    let row = Arc::new(Row::default());
    let mut walk = Walk::new(&device);
    let started = Instant::now();
    walk.connect(establish(&connector, &local, "windows-device", row.clone()));
    let connected = walk.connected();
    report_first(
        "windows-device",
        started.elapsed(),
        connected,
        folder_bytes(&server.packages),
    );
    assert!(row.asked().is_empty(), "{:?}", row.asked());
    assert!(
        connected.installed,
        "the first connection installs the node"
    );
    assert_eq!(connected.node.hello.os, "windows");
    assert!(
        connected.helper_path.ends_with("hided.exe"),
        "{}",
        connected.helper_path
    );
    assert!(
        connected.node.terminals.is_ok(),
        "the device's holders did not start: {:?}",
        connected.node.terminals.as_ref().err()
    );
    let home = connected
        .node
        .hello
        .home
        .clone()
        .expect("the node names its home");
    // Windows compares names without case.
    assert!(
        home.to_lowercase()
            .ends_with(&format!("/.hide-dev/{run_name}/home").to_lowercase()),
        "the node's home {home} is not the fixture's"
    );
    let native_home = hide_platform::path::from_wire(&home).unwrap();

    let project = native_home.join("project");
    std::fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "-q"]);
    std::fs::write(project.join("a.txt"), "hello\n").unwrap();
    git(&project, &["add", "a.txt"]);
    git(&project, &["commit", "-qm", "init"]);
    std::fs::write(project.join("a.txt"), "hello\nmore\n").unwrap();
    let project = hide_platform::path::to_wire(&project).unwrap();
    files_and_git(&*connected.node.host, &project);

    // The holder starts the integrated PowerShell, and Windows' console
    // ends a line with a bare CR.
    terminal(
        &mut walk,
        &sink,
        "windows-device-pane",
        &project,
        b"Write-Output ('own-' + 6*7)\r",
        "own-42",
    );
    // The node and the holder run from the fixture's device home; this
    // look also keeps the pane's shell.
    assert!(!device.running().unwrap().is_empty());
    walk.end();
    reconnect(
        &connector,
        &local,
        "windows-device",
        Arc::new(Row::default()),
        &device,
    );
}

/// The bytes of the desktop app's `hide.cmd` (`hide-kit/src/cli_windows.rs`),
/// which reach its `hide.exe` through the `.hide-kit` junction beside it.
const APP_SHIM: &[u8] = b"@echo off\r\nrem Hide kit command. Managed by Hide.\r\nsetlocal DisableDelayedExpansion\r\n\"%~dp0.hide-kit\\hide.exe\" %*\r\nexit /b %errorlevel%\r\n";

/// `FILE_ATTRIBUTE_REPARSE_POINT`: a junction, a symbolic link or any other
/// entry the system redirects through.
const REPARSE_POINT: u32 = 0x400;

/// Every reparse point under `folder`, which is not followed into.
fn reparse_points(folder: &Path) -> Vec<PathBuf> {
    use std::os::windows::fs::MetadataExt;
    let mut found = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            if metadata.file_attributes() & REPARSE_POINT != 0 {
                found.push(path);
            } else if metadata.is_dir() {
                folders.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Makes `link` a junction to `target` the way a standard user's program
/// makes one: `runas /trustlevel:0x20000` runs `mklink` with a token that
/// is no administrator's, so the redirection guard records its maker as
/// untrusted, as it does for the desktop app's own junction. `runas` returns
/// once it has started the command, so the junction is waited for.
fn untrusted_junction(link: &Path, target: &Path) {
    let (link_name, target_name) = (link.display().to_string(), target.display().to_string());
    assert!(
        !link_name.contains(' ') && !target_name.contains(' '),
        "runas takes one command line: {link_name} {target_name}"
    );
    let status = Command::new(r"C:\Windows\System32\runas.exe")
        .arg("/trustlevel:0x20000")
        .arg(format!("cmd.exe /d /c mklink /J {link_name} {target_name}"))
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "runas: {status}");
    wait_for("the standard user's junction", || {
        std::fs::symlink_metadata(link).is_ok().then_some(())
    });
}

/// The device's kit for the command folder `~/.local/bin`, as the core asks
/// for it once the operator allowed it; the report it answers.
fn kit_apply(walk: &Walk<'_>) -> Value {
    let report: Value = call_as(
        &*walk.connected().node.host,
        Call::Kit {
            action: KitAction::Apply,
            cli_dir: "~/.local/bin".to_owned(),
            retirement_projects: Vec::new(),
        },
        TIMEOUT,
    )
    .unwrap_or_else(|error| panic!("the device's kit: {error}"));
    assert!(report["refused"].is_null(), "{report}");
    report
}

/// The word after the last `hide-exit-` the terminals showed: what the
/// pane's `hide` line of the redirection guard walk answered.
fn hide_answer(output: &str) -> Option<String> {
    let (_, after) = output.rsplit_once("hide-exit-")?;
    let word: String = after
        .chars()
        .take_while(|letter| letter.is_ascii_alphanumeric() || *letter == '-')
        .collect();
    // The line is complete once a letter that is no part of the word follows.
    (after.len() > word.len()).then_some(word)
}

/// with-hide/hide-private#501, win-device-links: a Windows device reaches
/// its own installation's programs through no link. Windows' OpenSSH
/// installer turns the redirection guard on for `sshd.exe`
/// (`install-sshd.ps1`, and the lane does the same), and every process of an
/// ssh session then refuses a junction no administrator made, with "The path
/// cannot be traversed because it contains an untrusted mount point".
///
/// The desktop app's `hide` holds the account's command folder first, its
/// junction made by a standard user as the app makes it. The device's kit
/// leaves it, and a pane of the device whose `PATH` starts with that folder
/// still runs the device's own `hide`, from the helper root's `current`
/// folder; neither the helper root nor the command folder holds a reparse
/// point of the device's. Once the app's command is gone, the device's kit
/// takes the name, and its `hide.cmd` answers in a plain ssh shell there.
#[test]
#[ignore = "the runner's OpenSSH Server, its redirection guard and a key it takes; the windows wsl lane sets them up"]
fn a_windows_device_reaches_its_programs_through_no_link_under_the_redirection_guard() {
    let server = WindowsServer::new();
    let device = WindowsDevice::new(server.run.path());
    let sink = Arc::new(Sink::default());
    let connector = server.connector(sink.clone());
    let mut walk = Walk::new(&device);
    walk.connect(establish(
        &connector,
        &server.local,
        "windows-device",
        Arc::new(Row::default()),
    ));
    let wire_home = walk
        .connected()
        .node
        .hello
        .home
        .clone()
        .expect("the node names its home");
    let home = hide_platform::path::from_wire(&wire_home).unwrap();
    let cli = home.join(".local").join("bin");
    let root = hide_kit::layout::helper_root(&home);

    // The desktop app's command, as its kit writes it into the account's
    // command folder: the shim and a junction to the package's resources.
    let resources = home.join("app").join("resources");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::write(resources.join("app.asar"), b"").unwrap();
    std::fs::write(home.join("app").join("hide.exe"), b"").unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_BIN_EXE_hided")).with_file_name("hide.exe"),
        resources.join("hide.exe"),
    )
    .unwrap();
    std::fs::create_dir_all(&cli).unwrap();
    std::fs::write(cli.join("hide.cmd"), APP_SHIM).unwrap();
    let app_junction = cli.join(".hide-kit");
    untrusted_junction(&app_junction, &resources);

    let report = kit_apply(&walk);
    // The account's `PATH` starts with its command folder, as the installers
    // that use `~\.local\bin` leave it.
    let path = format!("{};{}", cli.display(), std::env::var("PATH").unwrap());
    start_pane(
        &mut walk,
        &sink,
        "windows-device-hide",
        &wire_home,
        vec![("PATH".to_owned(), path)],
    );
    // As Windows spells the path, with no `/` the layout's names carry.
    let own = PathBuf::from(
        root.join(hide_kit::CURRENT)
            .join("hide.exe")
            .display()
            .to_string()
            .replace('/', "\\"),
    );
    // The answer's words are split in the line, so its echo never reads as
    // the answer.
    let line = format!(
        "hide --version; Write-Output ('hide-' + 'exit-' + $LASTEXITCODE + '-' + ((Get-Command hide).Source -eq '{}'))\r",
        own.display()
    );
    let answer = typed(
        &walk,
        &sink,
        "windows-device-hide",
        line.as_bytes(),
        hide_answer,
    );
    assert_eq!(
        answer,
        "0-True",
        "the pane's hide is not the device's own {}: {}",
        own.display(),
        String::from_utf8_lossy(&sink.output.lock().unwrap())
    );
    assert_eq!(reparse_points(&root), Vec::<PathBuf>::new(), "{report}");
    assert_eq!(reparse_points(&cli), vec![app_junction.clone()], "{report}");

    // The app's command goes, as removing the app leaves the folder; the
    // device's kit then takes the name.
    hide_platform::fs::link::remove_link(&app_junction).unwrap();
    std::fs::remove_file(cli.join("hide.cmd")).unwrap();
    let report = kit_apply(&walk);
    assert_eq!(reparse_points(&cli), Vec::<PathBuf>::new(), "{report}");
    let shell = Command::new(Path::new(OPENSSH).join("ssh.exe"))
        .arg("-F")
        .arg(server.local.join(".ssh").join("config"))
        .args(["-o", "BatchMode=yes", "windows-device"])
        .arg(format!("{} --version", cli.join("hide.cmd").display()))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        shell.status.success(),
        "the device's hide.cmd in a plain ssh shell: {}{}",
        String::from_utf8_lossy(&shell.stdout),
        String::from_utf8_lossy(&shell.stderr)
    );
    walk.end();
}

/// `sshd` inside the distribution, started by this test and ended with it.
struct WslSshd {
    distro: String,
    child: hide_platform::process::OwnedChild,
}

impl WslSshd {
    /// The distribution's `sshd` in the foreground of a `wsl.exe` this test
    /// owns, so the distribution runs while it serves and it ends with the
    /// test; answers once the runner's loopback hears its greeting.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn start(distro: &str) -> Self {
        let mut command = Command::new("wsl.exe");
        command
            .args([
                "-d",
                distro,
                "-u",
                "root",
                "-e",
                "/usr/sbin/sshd",
                "-D",
                "-e",
            ])
            .args(["-p", &WSL_SSH_PORT.to_string()])
            .args([
                "-o",
                "ListenAddress=0.0.0.0",
                "-o",
                "PasswordAuthentication=no",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let sshd = Self {
            distro: distro.to_owned(),
            child: hide_platform::process::OwnedChild::spawn(&mut command).unwrap(),
        };
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let greeting =
                TcpStream::connect(("127.0.0.1", WSL_SSH_PORT)).and_then(|mut stream| {
                    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                    let mut first = [0; 4];
                    stream.read_exact(&mut first)?;
                    Ok(first)
                });
            if matches!(greeting, Ok(first) if &first == b"SSH-") {
                return sshd;
            }
            assert!(
                Instant::now() < deadline,
                "the distribution's sshd did not answer on 127.0.0.1:{WSL_SSH_PORT}: {greeting:?}"
            );
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

impl Drop for WslSshd {
    fn drop(&mut self) {
        if let Err(error) = self.child.kill_tree() {
            eprintln!("the distribution's sshd did not end: {error}");
        }
        let _ = self.child.wait();
        // Ending `wsl.exe` need not end what it ran inside the distribution.
        let ended = Command::new("wsl.exe")
            .args(["-d", &self.distro, "-u", "root", "-e", "pkill", "-f"])
            .arg(format!("sshd .*-p {WSL_SSH_PORT}"))
            .status();
        if let Err(error) = ended {
            eprintln!("the distribution's sshd was not asked to end: {error}");
        }
    }
}

/// c2: a Linux device of a Windows core over the system ssh: an `sshd` in
/// the runner's WSL 2 distribution, reached on the runner's loopback with a
/// key whose passphrase the device row is asked once, through the askpass
/// `hide.exe` the core finds beside its own programs. The Linux programs
/// come from the lane's kit under the developer folder's names.
#[test]
#[ignore = "an sshd in the runner's WSL 2 distribution; the windows wsl lane sets it up"]
fn a_windows_core_connects_a_linux_device_over_ssh_with_a_passphrase_asked_once() {
    let distro = std::env::var("HIDE_E2E_WSL_DISTRO").expect("set HIDE_E2E_WSL_DISTRO");
    let fixture = tempfile::Builder::new().prefix("wsl").tempdir().unwrap();
    let packages = fixture.path().join("packages");
    own_package(&packages);
    for entry in std::fs::read_dir(variable("HIDE_E2E_WSL_PACKAGES")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), packages.join(entry.file_name())).unwrap();
    }
    let key = key(fixture.path(), "locked", PASSPHRASE);
    let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let _account = Account { distro: &distro };
    // A key's public half holds no single quote.
    as_root(
        &distro,
        &format!(
            "pkill -KILL -u {SSH_USER} 2>/dev/null; userdel -r -f {SSH_USER} 2>/dev/null; useradd -m -s /bin/bash {SSH_USER} \
             && mkdir -p /run/sshd && ssh-keygen -A \
             && mkdir -m 700 ~{SSH_USER}/.ssh && printf '%s\\n' '{}' > ~{SSH_USER}/.ssh/authorized_keys \
             && chmod 600 ~{SSH_USER}/.ssh/authorized_keys && chown -R {SSH_USER}: ~{SSH_USER}/.ssh \
             && su - {SSH_USER} -c 'mkdir -p project && cd project && git init -q && printf \"hello\\n\" > a.txt && git add a.txt && git -c user.name=t -c user.email=t@example.invalid commit -qm init && printf \"more\\n\" >> a.txt'",
            public.trim()
        ),
    );
    let host_key = as_root(&distro, "cat /etc/ssh/ssh_host_ed25519_key.pub");
    let known_hosts = fixture.path().join("known_hosts");
    std::fs::write(
        &known_hosts,
        format!("[127.0.0.1]:{WSL_SSH_PORT} {}\n", host_key.trim()),
    )
    .unwrap();
    let local = fixture.path().join("local");
    std::fs::create_dir_all(local.join(".ssh")).unwrap();
    std::fs::write(
        local.join(".ssh").join("config"),
        format!(
            "Host linux-device\n  HostName 127.0.0.1\n  Port {WSL_SSH_PORT}\n  User {SSH_USER}\n  IdentityFile {}\n  IdentitiesOnly yes\n  IdentityAgent none\n  UserKnownHostsFile {}\n  GlobalKnownHostsFile {}\n",
            config_path(&key),
            config_path(&known_hosts),
            config_path(&known_hosts),
        ),
    )
    .unwrap();
    let _sshd = WslSshd::start(&distro);
    let device = LinuxDevice { distro: &distro };

    let sink = Arc::new(Sink::default());
    let connector = connector(&packages, sink.clone());
    let row = Row::answering(vec![Answer::Text(Secret::new(PASSPHRASE.to_owned()))]);
    let mut walk = Walk::new(&device);
    let started = Instant::now();
    walk.connect(establish(&connector, &local, "linux-device", row.clone()));
    let connected = walk.connected();
    report_first(
        "linux-device",
        started.elapsed(),
        connected,
        folder_bytes(&variable("HIDE_E2E_WSL_PACKAGES")),
    );
    let asked = row.asked();
    // ssh names the key as the config spells it, `/` between names.
    assert!(
        matches!(&asked[..], [SignInWait::Secret { key_file: Some(file), .. }] if Path::new(file).file_name() == key.file_name()),
        "the key's passphrase is asked once for the whole connection: {asked:?}"
    );
    assert!(
        connected.installed,
        "the first connection installs the node"
    );
    assert_eq!(connected.node.hello.os, "linux");
    assert!(
        connected.node.terminals.is_ok(),
        "the device's holders did not start: {:?}",
        connected.node.terminals.as_ref().err()
    );
    let home = connected
        .node
        .hello
        .home
        .clone()
        .expect("the node names its home");
    // The account's home as the distribution names it.
    let account_home = as_root(&distro, &format!("getent passwd {SSH_USER} | cut -d: -f6"));
    assert_eq!(home, account_home.trim());
    let project = format!("{home}/project");
    files_and_git(&*connected.node.host, &project);

    terminal(
        &mut walk,
        &sink,
        "linux-device-pane",
        &project,
        b"printf 'ssh_%s\\n' answered\r",
        "ssh_answered",
    );
    walk.end();
    // Each connection asks the passphrase once, the reconnection too.
    let again = Row::answering(vec![Answer::Text(Secret::new(PASSPHRASE.to_owned()))]);
    reconnect(&connector, &local, "linux-device", again.clone(), &device);
    assert_eq!(again.asked().len(), 1, "{:?}", again.asked());
}
