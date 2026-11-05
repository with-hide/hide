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
//! The `windows wsl` lane sets up what only a disposable runner may have and
//! runs both: the OpenSSH Server it installs, with a key it authorizes for
//! the runner's account (`HIDE_E2E_WINDOWS_SSH_DIR`: `client` and
//! `known_hosts`), and the WSL distribution its other walks own
//! (`HIDE_E2E_WSL_DISTRO`, its Linux kit in `HIDE_E2E_WSL_PACKAGES`). Both
//! need this build's CLI binaries (`scripts/verify-cargo.sh cli`) and the
//! pinned ConPTY runtime (`HIDE_HOLDER_CONPTY`).
#![cfg(windows)]

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
use hide_node_link::protocol::{Call, RootOpened, RootRef};
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
    hide_node::ssh::Connector::new(Some(packages.to_path_buf()))
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
/// `line` does not spell, so its echo is never taken for its output.
fn terminal(
    connected: &Established,
    sink: &Sink,
    pane: &str,
    cwd: &str,
    line: &[u8],
    answer: &str,
) -> LinkPanes {
    let panes = LinkPanes::new(
        Arc::clone(&connected.node.host),
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
        env: Vec::new(),
        rows: 24,
        cols: 80,
        notice: None,
    };
    let request = panes.request().unwrap();
    let reserved = panes.reserve(&start, None, request).unwrap();
    panes.start_reserved(&start, reserved, request).unwrap();
    panes.retire(request).unwrap();
    let terminals = connected
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
    terminals.key(KeyTarget::Pane(pane.to_owned()), line.to_vec(), 1);
    wait_for("the typed line's output", || {
        String::from_utf8_lossy(&sink.output.lock().unwrap())
            .contains(answer)
            .then_some(())
    });
    panes
}

/// The processes running a program file under `folder`: the device's node,
/// its holders and the ConPTY host each holder starts.
fn programs_running(folder: &Path) -> Vec<(PathBuf, Vec<u32>)> {
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
                    .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                if !pids.is_empty() {
                    running.push((path, pids));
                }
            }
        }
    }
    running
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

/// Ends `pane` and the link; once `quiet` answers, nothing of the device's
/// node or holders runs.
fn end(panes: LinkPanes, pane: &str, connected: Established, quiet: impl Fn() -> bool) {
    panes.terminate(pane).unwrap();
    drop(panes);
    connected.node.host.close("test ended");
    drop(connected);
    wait_for("the device's node and holders to end", || {
        quiet().then_some(())
    });
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
    let ssh = variable("HIDE_E2E_WINDOWS_SSH_DIR");
    let fixture = tempfile::Builder::new().prefix("wsd").tempdir().unwrap();
    let packages = fixture.path().join("packages");
    own_package(&packages);
    let local = fixture.path().join("local");
    std::fs::create_dir_all(local.join(".ssh")).unwrap();
    std::fs::write(
        local.join(".ssh").join("config"),
        format!(
            "Host windows-device\n  HostName 127.0.0.1\n  Port 22\n  User {}\n  IdentityFile {}\n  IdentitiesOnly yes\n  IdentityAgent none\n  UserKnownHostsFile {}\n  GlobalKnownHostsFile {}\n",
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

    let sink = Arc::new(Sink::default());
    let connector = connector(&packages, sink.clone())
        .with_device_home(Some(format!("~/.hide-dev/{run_name}/home")));
    let row = Arc::new(Row::default());
    let started = Instant::now();
    let connected = establish(&connector, &local, "windows-device", row.clone());
    println!(
        "windows device: connected in {:?}, upload={:?}",
        started.elapsed(),
        connected.upload
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
    let panes = terminal(
        &connected,
        &sink,
        "windows-device-pane",
        &project,
        b"Write-Output ('own-' + 6*7)\r",
        "own-42",
    );
    // The node and the holder ran from the fixture's device home.
    assert!(!programs_running(run.path()).is_empty());
    end(panes, "windows-device-pane", connected, || {
        programs_running(run.path()).is_empty()
    });
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
    // A key's public half holds no single quote.
    as_root(
        &distro,
        &format!(
            "pkill -KILL -u {SSH_USER} 2>/dev/null; userdel -r -f {SSH_USER} 2>/dev/null; useradd -m -s /bin/bash {SSH_USER} \
             && mkdir -p /run/sshd && ssh-keygen -A \
             && mkdir -m 700 /home/{SSH_USER}/.ssh && printf '%s\\n' '{}' > /home/{SSH_USER}/.ssh/authorized_keys \
             && chmod 600 /home/{SSH_USER}/.ssh/authorized_keys && chown -R {SSH_USER}: /home/{SSH_USER}/.ssh \
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
    let sshd = WslSshd::start(&distro);

    let sink = Arc::new(Sink::default());
    let connector = connector(&packages, sink.clone());
    let row = Row::answering(vec![Answer::Text(Secret::new(PASSPHRASE.to_owned()))]);
    let started = Instant::now();
    let connected = establish(&connector, &local, "linux-device", row.clone());
    println!(
        "linux device: connected in {:?}, upload={:?}, asked={:?}",
        started.elapsed(),
        connected.upload,
        row.asked()
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
    assert_eq!(home, format!("/home/{SSH_USER}"));
    let project = format!("{home}/project");
    files_and_git(&*connected.node.host, &project);

    let panes = terminal(
        &connected,
        &sink,
        "linux-device-pane",
        &project,
        b"printf 'ssh_%s\\n' answered\r",
        "ssh_answered",
    );
    end(panes, "linux-device-pane", connected, || {
        as_root(
            &distro,
            &format!("pgrep -u {SSH_USER} -f '[n]ode serve|[h]ide-holder' >/dev/null && echo running || echo quiet"),
        )
        .contains("quiet")
    });
    drop(sshd);
    as_root(
        &distro,
        &format!(
            "pkill -KILL -u {SSH_USER} 2>/dev/null; userdel -r -f {SSH_USER} 2>/dev/null; true"
        ),
    );
}
