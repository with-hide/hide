//! A paired machine's core and an app update, on every system (PRD
//! device-connect-iroh B32, D-41): the core its login item runs from a build
//! in the install root keeps running when the app is replaced, and the next
//! `hide connect` of the newer app puts its build beside the core's and
//! updates the core to it through `hided core-update`, or brings the
//! previous build back when the new one does not start.
//!
//! The login item is the fixture starter here (`HIDE_CORE_STARTER=fixture`
//! in a fixture HOME), which runs the core with the item's own command,
//! `hided core-login`, and never reaches the account's real login items.
//! Every process runs on a private home and state folder with none of the
//! Hide variables of the run that started the test.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const VERSION: &str = "1.2.10";
/// The build order of the core's build and of the app's newer one.
const OLD: u32 = 1;
const NEW: u32 = 2;
/// A folder name under the install root that only the old build uses.
const OLD_VERSION_FOLDER: &str = "00000000000000a1";

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// One journey's machine: a fixture HOME, its state folder, the install
/// root a login item runs its builds from, and the newer app.
struct Machine {
    _dir: tempfile::TempDir,
    home: PathBuf,
    state: PathBuf,
    core_root: PathBuf,
    /// The newer app's `hide`, inside its package.
    app_cli: PathBuf,
    /// The newer app's `hided`, whose bytes are the new build.
    app_daemon: PathBuf,
    /// The core's build: `<core_root>/<OLD_VERSION_FOLDER>/hided`.
    old_daemon: PathBuf,
    old_core: Option<Child>,
}

impl Machine {
    fn new() -> Self {
        let dir = crate::fixtures::fixture_root("hu");
        let root = hide_platform::fs::identity::canonical(dir.path()).unwrap();
        let home = root.join("home");
        let state = root.join("state");
        crate::fixtures::fixture_home(&home);
        let core_root = hide_kit::layout::core_root(&home);
        let resources = package(&root.join("app"));
        let built = Path::new(env!("CARGO_BIN_EXE_hided"));
        let app_cli = resources.join(exe("hide"));
        let app_daemon = resources.join(exe("hided"));
        std::fs::copy(built.with_file_name(exe("hide")), &app_cli).unwrap();
        std::fs::copy(built, &app_daemon).unwrap();
        // The parts a build carries besides its programs; nothing here runs
        // a pane, so their bytes are stand-ins.
        for name in ["hide-agent-hooks", "hide-holder"] {
            stand_in(&resources.join(exe(name)));
        }
        if cfg!(windows) {
            for (path, _) in conpty_runtime() {
                let file = resources.join(&path);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, path.as_bytes()).unwrap();
            }
        }
        let old_daemon = core_root.join(OLD_VERSION_FOLDER).join(exe("hided"));
        other_build(built, &old_daemon);
        Self {
            _dir: dir,
            home,
            state,
            core_root,
            app_cli,
            app_daemon,
            old_daemon,
            old_core: None,
        }
    }

    /// `program` with only this machine's places and the fixture's
    /// switches, for a build of `order`.
    fn command(&self, program: &Path, order: u32) -> Command {
        let mut command = Command::new(program);
        for (key, _) in std::env::vars_os() {
            let upper = key.to_string_lossy().to_uppercase();
            if upper.starts_with("HIDE_") {
                command.env_remove(key);
            }
        }
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("APPDATA", self.home.join("AppData").join("Roaming"))
            .env("LOCALAPPDATA", self.home.join("AppData").join("Local"))
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env_remove("XDG_STATE_HOME")
            .env("HIDE_STATE_DIR", &self.state)
            .env("HIDE_TAILSCALE_BIN", self.home.join("no-tailscale"))
            .env("HIDE_CORE_STARTER", "fixture")
            .env("HIDE_BUILD_VERSION_OVERRIDE", VERSION)
            .env("HIDE_BUILD_ORDER_OVERRIDE", order.to_string())
            .stdin(Stdio::null());
        command
    }

    /// The core the login item starts at login: the old build's `hided
    /// core-login`, with the item's marker in place.
    fn log_in(&mut self) -> Value {
        self.log_in_as(OLD)
    }

    /// [`Self::log_in`], with the core's build of `order`.
    fn log_in_as(&mut self, order: u32) -> Value {
        std::fs::create_dir_all(&self.state).unwrap();
        std::fs::write(hide_kit::layout::keep_alive(&self.state), "").unwrap();
        let log = std::fs::File::create(self.home.join("old-core.log")).unwrap();
        let child = self
            .command(&self.old_daemon, order)
            .arg("core-login")
            .arg("--state-dir")
            .arg(&self.state)
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .expect("the old build's core starts");
        self.old_core = Some(child);
        let old = build_of(&self.old_daemon);
        self.health_until(|health| health["build"] == old.as_str())
    }

    /// `hide connect` of the newer app: whether it succeeded, and its line.
    fn connect(&self) -> (bool, Value) {
        let mut connect = self.command(&self.app_cli, NEW);
        connect.arg("connect");
        self.answer(connect)
    }

    /// What the running core answers the app's `hide workspace info` from a
    /// pane of `install`, as a pane names its installation: its refusal
    /// reason, none for an answer.
    fn workspace_info_as(&self, install: &str) -> Option<String> {
        let mut info = self.command(&self.app_cli, NEW);
        info.env(hide_agent_hooks::route::INSTALL_VARIABLE, install)
            .args(["workspace", "info"])
            .current_dir(&self.home);
        let (_, line) = self.answer(info);
        line["reason"].as_str().map(str::to_owned)
    }

    /// The raise a paired machine runs with the build it put here (B23):
    /// `program core-update` with no build named, as a build of `order`.
    fn raise(&self, program: &Path, order: u32) -> Value {
        let mut raise = self.command(program, order);
        raise
            .arg("core-update")
            .arg("--state-dir")
            .arg(&self.state)
            .args(["--intent", "update-1"]);
        let (ok, line) = self.answer(raise);
        assert!(ok, "a raise answers with its outcome: {line}");
        line
    }

    /// Runs `command` to its end: whether it succeeded, and its line.
    fn answer(&self, mut command: Command) -> (bool, Value) {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the command starts");
        // Read to the end on a thread of its own: a daemon that inherited
        // the output would hold it open for its whole life on Windows.
        let mut stdout = child.stdout.take().unwrap();
        let (done, heard) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes);
            let _ = done.send(bytes);
        });
        let bytes = heard
            .recv_timeout(Duration::from_secs(240))
            .expect("the command answers within the update's bound");
        let ok = child.wait().unwrap().success();
        let line = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "the command printed no JSON line ({error}): {}",
                String::from_utf8_lossy(&bytes)
            )
        });
        (ok, line)
    }

    /// The running core's `/health` once `until` holds of it.
    #[allow(clippy::disallowed_methods)] // a polling helper: it reads another process's answers until a deadline
    fn health_until(&self, until: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut last = Value::Null;
        loop {
            if let Some(health) = health(&self.state) {
                if until(&health) {
                    return health;
                }
                last = health;
            }
            assert!(
                Instant::now() < deadline,
                "the core did not answer as expected: {last}; its log: {}",
                std::fs::read_to_string(self.home.join("old-core.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(100)); // wait-ok: another process's /health naming the build expected
        }
    }

    /// The version folders under the install root.
    fn builds(&self) -> Vec<String> {
        let mut builds: Vec<String> = std::fs::read_dir(&self.core_root)
            .unwrap()
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| hide_kit::is_build_name(name))
            .collect();
        builds.sort();
        builds
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = self
            .command(&self.app_cli, NEW)
            .arg("stop")
            .stdout(Stdio::null())
            .status();
        if let Some(mut core) = self.old_core.take() {
            let _ = core.kill();
            let _ = core.wait();
        }
    }
}

/// The package folder an app's `hide` counts as the app's own in
/// (`hide_kit::bundled_kit_dir`), made under `app`.
fn package(app: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        let resources = app.join("Hide.app").join("Contents").join("Resources");
        std::fs::create_dir_all(&resources).unwrap();
        resources
    } else {
        let resources = app.join("resources");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(resources.join("app.asar"), "").unwrap();
        stand_in(&app.join(exe("hide")));
        resources
    }
}

fn stand_in(path: &Path) {
    std::fs::write(path, path.to_string_lossy().as_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// The holder's ConPTY runtime files the Windows kit names
/// (`contracts/device-kits.json`).
fn conpty_runtime() -> Vec<(String, bool)> {
    let contract: Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../contracts/device-kits.json"),
        )
        .unwrap(),
    )
    .unwrap();
    contract["kits"]["windows-x86_64"]["runtime"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(path, program)| (path.clone(), program.as_bool().unwrap()))
        .collect()
}

/// A copy of `built` at `at` that is another build of it: on macOS re-signed
/// under another identifier, as a package of another commit is; elsewhere
/// with bytes after its image, which its loader never reads.
fn other_build(built: &Path, at: &Path) {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::copy(built, at).unwrap();
    if cfg!(target_os = "macos") {
        let signed = Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-", "--identifier"])
            .arg("dev.hide.always-on-old-build")
            .arg(at)
            .status()
            .unwrap();
        assert!(signed.success(), "codesign could not re-sign the copy");
    } else {
        let mut file = std::fs::OpenOptions::new().append(true).open(at).unwrap();
        file.write_all(b"\0hide always-on old build\0").unwrap();
    }
}

/// A build as the daemon names it: the SHA-256 of its file.
fn build_of(program: &Path) -> String {
    hided::build_id::of_file(program).unwrap()
}

/// The core's `/health`, when one answers on the state folder's port.
fn health(state: &Path) -> Option<Value> {
    let recorded: Value =
        serde_json::from_slice(&std::fs::read(state.join("hided.json")).ok()?).ok()?;
    let port = recorded["port"].as_u64()?;
    let mut stream = TcpStream::connect(("127.0.0.1", port as u16)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    let response = String::from_utf8(response).ok()?;
    let (_, body) = response.split_once("\r\n\r\n")?;
    serde_json::from_str(body).ok()
}

fn program_of(health: &Value) -> PathBuf {
    PathBuf::from(
        health["starter"]["program"]
            .as_str()
            .unwrap_or_else(|| panic!("the core names no starter program: {health}")),
    )
}

fn same_file(left: &Path, right: &Path) -> bool {
    hide_platform::fs::identity::canonical(left).unwrap()
        == hide_platform::fs::identity::canonical(right).unwrap()
}

#[test]
fn a_newer_app_updates_the_core_its_login_item_runs_and_keeps_the_previous_build() {
    let mut machine = Machine::new();
    let old = machine.log_in();
    assert!(same_file(&program_of(&old), &machine.old_daemon), "{old}");
    let old_pid = machine.old_core.as_ref().unwrap().id();

    let (ok, line) = machine.connect();

    assert!(ok, "the newer app's connect updates the core: {line}");
    let new_build = build_of(&machine.app_daemon);
    let updated = machine.health_until(|health| health["build"] == new_build.as_str());
    let program = program_of(&updated);
    assert!(
        program.starts_with(&machine.core_root) && !same_file(&program, &machine.old_daemon),
        "the core runs the new build from the install root: {}",
        program.display()
    );
    assert_eq!(line["pid"], updated["pid"], "{line}");
    assert!(
        !hide_platform::process::is_alive(old_pid),
        "the old build's core ended"
    );
    // The previous build is kept for a way back, and no more.
    assert_eq!(machine.builds().len(), 2, "{:?}", machine.builds());
    assert!(
        hide_kit::layout::keep_alive(&machine.state).is_file(),
        "the item's marker stays"
    );
    // The core the item runs from a copy of the app's build is the app's:
    // a pane the app's own daemon started reaches it, and none of the
    // install root's own does (with-hide/hide#2876).
    assert_ne!(
        machine.workspace_info_as(hide_agent_hooks::route::APP_INSTALL),
        Some("other_install".to_owned())
    );
    assert_eq!(
        machine.workspace_info_as(&hide_kit::layout::install_of_root(&machine.core_root)),
        Some("other_install".to_owned())
    );
}

/// The app the operator puts on a machine is the build its login item
/// runs, as it is with no item: an app older than the core, or one of a
/// build neither orders, replaces the core with its own build rather than
/// asking the operator to update the app it just installed.
#[test]
fn an_app_older_than_the_core_its_login_item_runs_puts_its_own_build_there() {
    let mut machine = Machine::new();
    machine.log_in_as(NEW + 1);

    let (ok, line) = machine.connect();

    assert!(ok, "the older app's connect replaces the core: {line}");
    let app_build = build_of(&machine.app_daemon);
    let replaced = machine.health_until(|health| health["build"] == app_build.as_str());
    assert!(program_of(&replaced).starts_with(&machine.core_root), "{replaced}");
    assert_ne!(
        machine.workspace_info_as(hide_agent_hooks::route::APP_INSTALL),
        Some("other_install".to_owned())
    );
}

/// B23: a machine that started this account's Hide over SSH or `wsl.exe`
/// puts its newer build here and runs it as a raise. The core its starter
/// runs is replaced by that build, placed in the install root; the same
/// build asked again finds nothing to do, and an older build leaves the
/// newer core as it is.
#[test]
fn a_paired_machine_s_newer_build_raises_the_core_and_an_older_one_leaves_it() {
    let mut machine = Machine::new();
    machine.log_in();
    let old_pid = machine.old_core.as_ref().unwrap().id();

    let raised = machine.raise(&machine.app_daemon, NEW);

    assert_eq!(raised["kind"], "updated", "{raised}");
    let new_build = build_of(&machine.app_daemon);
    let updated = machine.health_until(|health| health["build"] == new_build.as_str());
    let program = program_of(&updated);
    assert!(
        program.starts_with(&machine.core_root) && !same_file(&program, &machine.old_daemon),
        "the core runs the new build from the install root: {}",
        program.display()
    );
    assert_eq!(raised["pid"], updated["pid"], "{raised}");
    assert!(
        !hide_platform::process::is_alive(old_pid),
        "the old build's core ended"
    );
    assert_eq!(machine.builds().len(), 2, "{:?}", machine.builds());

    let again = machine.raise(&machine.app_daemon, NEW);
    assert_eq!(again["kind"], "current", "{again}");
    let lower = machine.raise(&machine.old_daemon, OLD);
    assert_eq!(lower["kind"], "kept", "{lower}");
    let after = machine.health_until(|health| health["build"] == new_build.as_str());
    assert_eq!(after["pid"], updated["pid"], "neither restarted the core");
}

/// B23: once the app has been opened on a machine whose Hide another
/// machine started, the app updates that core, so a paired machine's build
/// newer than the app's raises nothing: a core raised past the app would
/// refuse the app's own connection. The app's word is its own `hide
/// connect`'s, whoever holds the account's `hide` command, here the device
/// helper that started the Hide.
#[test]
fn a_core_its_app_opened_is_left_to_the_app_by_a_paired_machine_s_newer_build() {
    let mut machine = Machine::new();
    machine.log_in();
    #[cfg(unix)]
    {
        let helper = hide_kit::layout::helper_root(&machine.home)
            .join("current")
            .join("hide");
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        stand_in(&helper);
        let bin = machine.home.join(".local").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(&helper, bin.join("hide")).unwrap();
    }

    let (ok, line) = machine.connect();

    assert!(ok, "the app opens and updates the core: {line}");
    let app_build = build_of(&machine.app_daemon);
    let opened = machine.health_until(|health| health["build"] == app_build.as_str());
    // Another build of a later order than the app's, as a paired machine
    // on a newer main would put here.
    let raised = machine.raise(&machine.old_daemon, NEW + 1);
    assert_eq!(raised["kind"], "left_to_app", "{raised}");
    let after = machine.health_until(|health| health["build"] == app_build.as_str());
    assert_eq!(after["pid"], opened["pid"], "the core was not restarted");
    assert_eq!(machine.builds().len(), 2, "{:?}", machine.builds());
}

#[test]
fn a_new_build_that_does_not_start_leaves_the_previous_core_running() {
    let mut machine = Machine::new();
    machine.log_in();
    std::fs::write(
        machine.home.join(".hide-e2e-fail-build"),
        build_of(&machine.app_daemon),
    )
    .unwrap();

    let (ok, line) = machine.connect();

    assert!(!ok, "{line}");
    assert_eq!(line["reason"], "update_failed", "{line}");
    let old = build_of(&machine.old_daemon);
    let back = machine.health_until(|health| health["build"] == old.as_str());
    assert!(
        same_file(&program_of(&back), &machine.old_daemon),
        "the previous build runs again: {back}"
    );
    assert!(hide_kit::layout::keep_alive(&machine.state).is_file());
}
