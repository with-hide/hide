//! Isolated process fixture for the actual SSH return-route journey.
//! Every candidate child has an owned process tree; counts and reads are capped.

pub mod renderer;
pub mod responsive;
use super::run::{Run, finish};

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use hide_platform::process::{CapturedOutput, OwnedChild};
use serde_json::{Value, json};

pub const READY_BOUND: Duration = Duration::from_secs(30);
const READ_CAP: usize = 1024 * 1024;

/// The prompt every fixture shell draws, so a test types into a pane only
/// once its shell is ready for keys.
pub const PROMPT: &str = "fixture-ready> ";

#[derive(Clone)]
pub struct Environment {
    pub home: PathBuf,
    values: BTreeMap<OsString, OsString>,
}

impl Environment {
    pub fn new(root: &Path, socket: &Path, bin: &Path, state_dir: Option<&Path>) -> Result<Self> {
        let home = root.join("home");
        for folder in [
            &home,
            &root.join("config"),
            &root.join("state"),
            &home.join("bin"),
            &home.join("project"),
        ] {
            fs::create_dir_all(folder)?;
        }
        fs::write(home.join(".zshrc"), format!("PS1='{PROMPT}'\n"))?;
        let path = format!(
            "{}:/usr/bin:/bin:/usr/sbin:/sbin",
            home.join("bin").display()
        );
        // A daemon finds `gh` and `git` on its login shell's `PATH`, and on
        // macOS that shell's `path_helper` puts the system's folders first,
        // where a runner keeps its own signed-out `gh` (issue
        // hide-private#201). The account's startup files put its own folders
        // first, as an operator's installs come first (`fixtureLoginShellPath`
        // in `web/e2e/platform-fixture.ts`).
        for file in [".zprofile", ".bash_profile", ".profile"] {
            fs::write(
                home.join(file),
                format!("PATH={}:\"$PATH\"\nexport PATH\n", quote(&path)),
            )?;
        }
        // This journey starts after retirement. Mark only that private
        // prerequisite complete so candidate kit reconciliation never reaches
        // the account's actual service manager; mailbox state is never seeded.
        let kit = hide_kit::kit_state_dir(&home);
        hide_platform::fs::private::create_dir_all(&kit)?;
        hide_platform::fs::atomic::write_file(
            &kit.join("coordination-retirement.json"),
            &serde_json::to_vec(
                &json!({"homes":[],"step":"complete","failure":null,"complete":true}),
            )?,
            hide_platform::fs::Access::Private,
        )?;
        fs::write(
            root.join("herdr.toml"),
            "[update]\nversion_check = false\nmanifest_check = false\n",
        )?;
        let mut values: BTreeMap<_, _> = std::env::vars_os()
            .filter(|(name, _)| {
                let name = name.to_string_lossy();
                ![
                    "HERDR_",
                    "HIDE_",
                    "HCOORD_",
                    "SASU_",
                    "ELECTRON_",
                    "SSH_",
                    "XDG_",
                    "CLAUDE_",
                    "CODEX_",
                    "OPENCODE_",
                    "GIT_",
                ]
                .iter()
                .any(|prefix| name.starts_with(prefix))
            })
            .collect();
        for (name, value) in [
            ("HOME", home.clone().into_os_string()),
            // The account's git reads only its own home: no operator signing
            // key or hook arrives through the machine's configuration.
            ("GIT_CONFIG_NOSYSTEM", "1".into()),
            ("SHELL", "/bin/zsh".into()),
            ("PATH", path.into()),
            ("HERDR_SOCKET_PATH", socket.as_os_str().to_owned()),
            ("HERDR_BIN_PATH", bin.as_os_str().to_owned()),
            (
                "HERDR_SESSION",
                format!("delivery-{}", root.file_name().unwrap().to_string_lossy()).into(),
            ),
            (
                "HERDR_CONFIG_PATH",
                root.join("herdr.toml").into_os_string(),
            ),
            ("XDG_CONFIG_HOME", root.join("config").into_os_string()),
            ("XDG_STATE_HOME", root.join("state").into_os_string()),
            ("HERDR_DISABLE_SOUND", "1".into()),
            ("skip_global_compinit", "1".into()),
        ] {
            values.insert(name.into(), value);
        }
        if let Some(state_dir) = state_dir {
            values.insert("HIDE_STATE_DIR".into(), state_dir.as_os_str().to_owned());
        }
        Ok(Self { home, values })
    }

    #[allow(clippy::disallowed_methods)] // env_clear: an SSH exec's environment, on Unix
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .envs(&self.values)
            .current_dir(&self.home);
        command
    }

    /// Every variable a session of the account gets.
    pub fn values(&self) -> Vec<(OsString, OsString)> {
        self.values
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }

    pub fn set(&mut self, name: &str, value: impl Into<OsString>) {
        self.values.insert(name.into(), value.into());
    }

    pub fn unset(&mut self, name: &str) {
        self.values.remove(OsStr::new(name));
    }
}

pub fn quote(value: impl AsRef<OsStr>) -> String {
    format!(
        "'{}'",
        value.as_ref().to_string_lossy().replace('\'', "'\\''")
    )
}

pub fn capture(mut command: Command) -> Result<CapturedOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    OwnedChild::spawn(&mut command)?
        .capture_until(Instant::now() + Duration::from_secs(10), READ_CAP)
        .map_err(|error| anyhow::anyhow!("private command capture: {error}"))
}

pub fn successful(command: Command) -> Result<Vec<u8>> {
    let answer = capture(command)?;
    ensure!(
        answer.status.success(),
        "private command failed: {}",
        String::from_utf8_lossy(&answer.stderr)
    );
    Ok(answer.stdout)
}

pub fn wait_for<T>(what: &str, observe: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    wait_within(what, READY_BOUND, observe)
}

pub fn wait_within<T>(
    what: &str,
    bound: Duration,
    mut observe: impl FnMut() -> Result<Option<T>>,
) -> Result<T> {
    let deadline = Instant::now() + bound;
    loop {
        if let Some(value) = observe()? {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for {what}");
        }
        // This is a state observation cadence, never a product action or
        // delay used as evidence of readiness.
        std::thread::park_timeout(Duration::from_millis(50));
    }
}

/// What the account's machine is running when the helper never became ready,
/// read-only, so a failed run shows whether the helper started at all. It
/// starts nothing: a second helper would change the state it reports.
fn helper_diagnostics(remote: &Environment) -> String {
    let mut processes = remote.command("/bin/ps");
    processes.args(["-eo", "pid,ppid,stat,etime,args"]);
    match capture(processes) {
        Ok(answer) => {
            let text = String::from_utf8_lossy(&answer.stdout);
            let shown: String = text.chars().take(8192).collect();
            format!("private helper never became ready; processes:\n{shown}")
        }
        Err(error) => format!("private helper never became ready; processes unavailable: {error}"),
    }
}

pub fn read(path: &Path) -> Result<Vec<u8>> {
    ensure!(
        fs::metadata(path)?.len() <= READ_CAP as u64,
        "private fixture read cap"
    );
    Ok(fs::read(path)?)
}

pub struct Herdr {
    pub environment: Environment,
    pub pane: String,
    pub root: PathBuf,
    binary: PathBuf,
    server: Option<OwnedChild>,
    sequence: u32,
}

impl Herdr {
    fn start(
        root: PathBuf,
        mut environment: Environment,
        binary: PathBuf,
        state: Option<&Path>,
    ) -> Result<Self> {
        if let Some(state) = state {
            environment.set("HIDE_STATE_DIR", state.as_os_str());
        }
        let log = File::create(root.join("herdr.log"))?;
        let mut command = environment.command(&binary);
        command
            .arg("server")
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        let server = OwnedChild::spawn(&mut command)?;
        let mut herdr = Self {
            root,
            environment,
            binary,
            server: Some(server),
            pane: String::new(),
            sequence: 0,
        };
        wait_for("private Herdr socket", || {
            let output = capture(herdr.command(&["api", "snapshot"]))?;
            let snapshot = serde_json::from_slice::<Value>(&output.stdout).ok();
            Ok((output.status.success()
                && snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.pointer("/result/snapshot").is_some()))
            .then_some(()))
        })?;
        let folder = herdr.environment.home.join("project");
        let workspace = herdr.run(&[
            "workspace",
            "create",
            "--label",
            "fixture",
            "--cwd",
            folder.to_str().context("fixture path UTF-8")?,
            "--focus",
        ])?;
        herdr.pane = workspace
            .pointer("/result/root_pane/pane_id")
            .and_then(Value::as_str)
            .context("private Herdr root pane")?
            .into();
        wait_for("private shell prompt", || {
            Ok(herdr.text()?.contains("fixture").then_some(()))
        })?;
        herdr.write(&[
            "agent",
            "start",
            if state.is_some() {
                "local-parent"
            } else {
                "remote-child"
            },
            "--kind",
            "claude",
            "--pane",
            &herdr.pane,
        ])?;
        wait_for("private provider command loop", || {
            Ok(herdr
                .text()?
                .contains("DELIVERY_FIXTURE_READY")
                .then_some(()))
        })?;
        herdr.write(&[
            "pane",
            "report-agent-session",
            &herdr.pane,
            "--source",
            "herdr:claude",
            "--agent",
            "claude",
            "--agent-session-id",
            if state.is_some() {
                "fixture-local-session"
            } else {
                "fixture-remote-session"
            },
            "--seq",
            "1",
        ])?;
        herdr.finished_session(if state.is_some() {
            "fixture-local-session"
        } else {
            "fixture-remote-session"
        })?;
        Ok(herdr)
    }

    /// The conversation file a provider keeps for `session`, holding one
    /// finished turn. The doorbell rings a Claude Code pane only once its
    /// session read says nothing waits for the operator, and the fixture's
    /// provider writes no conversation of its own.
    fn finished_session(&self, session: &str) -> Result<()> {
        let folder = self.environment.home.join(".claude/projects/-fixture");
        fs::create_dir_all(&folder)?;
        let record = |kind: &str, message: Value| {
            json!({"type": kind, "sessionId": session, "uuid": format!("{session}-{kind}"),
                "timestamp": "2026-01-01T00:00:00.000Z", "message": message})
            .to_string()
        };
        fs::write(
            folder.join(format!("{session}.jsonl")),
            [
                record("user", json!({"role": "user", "content": "start"})),
                record(
                    "assistant",
                    json!({"role": "assistant", "stop_reason": "end_turn",
                        "content": [{"type": "text", "text": "ready"}]}),
                ),
            ]
            .join("\n")
                + "\n",
        )?;
        Ok(())
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = self.environment.command(&self.binary);
        command.args(args);
        command
    }

    pub fn run(&self, args: &[&str]) -> Result<Value> {
        serde_json::from_slice(&successful(self.command(args))?)
            .context("private Herdr answer JSON")
    }

    fn write(&self, args: &[&str]) -> Result<()> {
        successful(self.command(args)).map(|_| ())
    }

    /// What the provider's own hook reports for a pane it runs in: the native
    /// session Hide waits for before it calls a spawned agent started. The
    /// fixture's provider is a shim that reports nothing by itself.
    pub fn report_session(&self, pane: &str, session: &str) -> Result<()> {
        self.finished_session(session)?;
        self.write(&[
            "pane",
            "report-agent-session",
            pane,
            "--source",
            "herdr:claude",
            "--agent",
            "claude",
            "--agent-session-id",
            session,
            "--seq",
            "1",
        ])
    }

    fn text(&self) -> Result<String> {
        Ok(String::from_utf8(successful(self.command(&[
            "pane", "read", &self.pane, "--source", "visible", "--format", "text",
        ]))?)?)
    }

    pub fn run_in_pane(&mut self, command: &str) -> Result<PaneOutput> {
        let pane = self.pane.clone();
        self.run_in(&pane, command)
    }

    /// Runs a command in another pane of this server whose provider is the
    /// fixture shim, such as the pane of an agent Hide spawned here.
    pub fn run_in(&mut self, pane: &str, command: &str) -> Result<PaneOutput> {
        ensure!(self.sequence < 32, "private pane command cap");
        self.sequence += 1;
        let base = self.root.join(format!("command-{}", self.sequence));
        let script = base.with_extension("sh");
        let out = base.with_extension("out");
        let err = base.with_extension("err");
        let status = base.with_extension("status");
        let timing = PathBuf::from(format!("{}.ms", script.display()));
        fs::write(
            &script,
            format!(
                "{command} > {} 2> {}\nprintf '%s' \"$?\" > {}\n",
                quote(&out),
                quote(&err),
                quote(&status)
            ),
        )?;
        self.write(&[
            "pane",
            "send-text",
            pane,
            &format!("RUN {}\n", script.display()),
        ])?;
        wait_for("attested pane command status", || {
            if status.exists() && timing.exists() {
                Ok(Some(()))
            } else {
                Ok(None)
            }
        })?;
        Ok(PaneOutput {
            stdout: String::from_utf8(read(&out)?)?,
            stderr: String::from_utf8(read(&err)?)?,
            status: String::from_utf8(read(&status)?)?.parse()?,
            elapsed: Duration::from_millis(String::from_utf8(read(&timing)?)?.trim().parse()?),
        })
    }

    fn stop(&mut self) -> Result<()> {
        if let Some(mut server) = self.server.take() {
            let stop = capture(self.command(&["server", "stop"]))?;
            ensure!(stop.status.success(), "private Herdr stop refused");
            wait_for("private Herdr confirmed exit", || {
                Ok(server.try_wait()?.map(|_| ()))
            })?;
        }
        Ok(())
    }
}

pub struct PaneOutput {
    pub stdout: String,
    pub stderr: String,
    pub status: u32,
    pub elapsed: Duration,
}

impl PaneOutput {
    pub fn json(&self) -> Result<Value> {
        ensure!(
            self.status == 0,
            "pane command failed: {} {}",
            self.stdout,
            self.stderr
        );
        let answer: Value = serde_json::from_str(&self.stdout).context("pane command JSON")?;
        ensure!(answer["ok"] == true, "pane CLI refused its command");
        answer.get("result").cloned().context("pane CLI result")
    }
}

pub struct Fixture {
    pub local: Herdr,
    pub remote: Herdr,
    pub ssh: super::sshd::Sshd,
    pub state: PathBuf,
    pub hide: PathBuf,
    pub hooks: PathBuf,
    source_cli: PathBuf,
    root: PathBuf,
    removed: bool,
    ipc: tempfile::TempDir,
    daemon: Option<OwnedChild>,
    port: u16,
    token: String,
}

impl Fixture {
    /// Runs one journey in a fresh fixture and ends it by how the journey
    /// went (`support::run::finish`): a passing journey leaves no run
    /// directory, a failing one keeps it for the lane to upload. The only way
    /// to start a fixture, so no test has to remember either half.
    pub fn journey(body: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        let mut fixture = Self::start(false)?;
        let journey = body(&mut fixture);
        finish(&mut fixture, journey)
    }

    pub fn measurement_journey(body: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        let mut fixture = Self::start(true)?;
        let journey = body(&mut fixture);
        finish(&mut fixture, journey)
    }

    fn start(measurement: bool) -> Result<Self> {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let bin = PathBuf::from(
            std::env::var_os("HIDE_E2E_HERDR_BIN")
                .context("set HIDE_E2E_HERDR_BIN to the pinned binary")?,
        );
        let source_cli = match std::env::var_os("HIDE_E2E_CLI_DIR") {
            Some(path) => PathBuf::from(path).canonicalize()?,
            None if measurement => bail!("measurement requires HIDE_E2E_CLI_DIR"),
            None => repository.join("target/debug"),
        };
        if measurement {
            ensure!(
                source_cli.ends_with("target/release"),
                "measurement requires worktree release binaries"
            );
        }
        for name in ["hide", "hided", "hide-agent-hooks"] {
            ensure!(
                source_cli.join(name).is_file(),
                "build this worktree's CLI binaries before remote_delivery"
            );
        }
        let artifacts = repository.join("agents/runs");
        fs::create_dir_all(&artifacts)?;
        let root = tempfile::Builder::new()
            .prefix("rd-")
            .tempdir_in(artifacts)?
            .keep();
        let ipc = tempfile::Builder::new().prefix("rd-").tempdir_in("/tmp")?;
        // The device's node binds its pane socket under the device's state
        // folder, which must stay short enough for a Unix socket path.
        let device_state = ipc.path().join("d");
        hide_platform::fs::private::create_dir_all(&device_state)?;
        let state = root.join("s");
        hide_platform::fs::private::create_dir_all(&state)?;
        let mut local_env =
            Environment::new(&root.join("l"), &ipc.path().join("l.sock"), &bin, None)?;
        let remote_env = Environment::new(
            &root.join("r"),
            &ipc.path().join("r.sock"),
            &bin,
            Some(&device_state),
        )?;
        // The device installs shipped executables, which carry no debug
        // data. Stage this candidate's code with the same property: hashing
        // and uploading a CI debug image can exhaust helper setup's bound.
        // Never strip the worktree's build output or another candidate.
        let cli = root.join("cli");
        fs::create_dir(&cli)?;
        for name in ["hide", "hided", "hide-agent-hooks"] {
            let staged = cli.join(name);
            fs::copy(source_cli.join(name), &staged)?;
            // Frozen build inputs may be read-only. Only this private copy
            // needs owner write access for strip and ad-hoc signing.
            hide_platform::fs::private::restrict_to_owner(&staged)?;
            let mut strip = local_env.command("/usr/bin/strip");
            strip.arg(&staged);
            successful(strip)?;
            #[cfg(target_os = "macos")]
            {
                let mut sign = local_env.command("/usr/bin/codesign");
                sign.args(["--force", "--sign", "-"]).arg(&staged);
                successful(sign)?;
            }
        }
        let manifest: Value =
            serde_json::from_slice(&read(&repository.join("contracts/herdr-bundle.json"))?)?;
        let mut version = local_env.command(&bin);
        version.arg("--version");
        ensure!(
            String::from_utf8(successful(version)?)?
                .contains(manifest["version"].as_str().context("pin version")?),
            "fixture Herdr does not match repository pin"
        );
        fs::write(root.join("provider.c"), SHIM)?;
        let mut compiler = local_env.command("/usr/bin/cc");
        compiler
            .arg("-O1")
            .arg(root.join("provider.c"))
            .arg("-o")
            .arg(local_env.home.join("bin/claude"));
        successful(compiler)?;
        fs::copy(
            local_env.home.join("bin/claude"),
            remote_env.home.join("bin/claude"),
        )?;
        // The ignored run directory lives inside this checkout. Give each
        // private project its own Git boundary so discovery cannot climb to
        // the real checkout outside the fixture account's home.
        for environment in [&local_env, &remote_env] {
            let mut command = environment.command("/usr/bin/git");
            command
                .args(["-c", "init.defaultBranch=main", "-C"])
                .arg(environment.home.join("project"))
                .args(["init", "-q"]);
            successful(command)?;
        }
        if measurement {
            for environment in [&local_env, &remote_env] {
                responsive::prepare_account(environment)?;
            }
        }
        // RemoteRead's production PATH is the account's .local/bin plus system
        // tools. This private account carries only this pinned Herdr there.
        fs::create_dir_all(remote_env.home.join(".local/bin"))?;
        std::os::unix::fs::symlink(&bin, remote_env.home.join(".local/bin/herdr"))?;
        fs::create_dir_all(local_env.home.join(".ssh"))?;
        let local = Herdr::start(root.join("l"), local_env.clone(), bin.clone(), Some(&state))?;
        let remote = Herdr::start(root.join("r"), remote_env.clone(), bin, None)?;
        let ssh = super::sshd::Sshd::start(&root.join("sshd"), &remote_env.values())?;
        fs::write(
            local_env.home.join(".ssh/config"),
            ssh.client_config("delivery-fixture"),
        )?;
        local_env.set("HIDE_STATE_DIR", state.as_os_str());
        local_env.set("HIDE_KEEP_ALIVE", "1");
        local_env.set("HIDE_OPEN_COMMAND", "/usr/bin/true");
        local_env.set(
            "HIDE_HOST_HELPER_ROOT",
            remote.environment.home.join("helper").into_os_string(),
        );
        local_env.set(
            "HIDE_HOST_CLI_DIR",
            remote.environment.home.join("bin").into_os_string(),
        );
        local_env.set(
            "HIDE_TAILSCALE_BIN",
            local_env.home.join("absent-tailscale").into_os_string(),
        );
        let log = File::create(root.join("hided.log"))?;
        let mut command = local_env.command(cli.join("hided"));
        command
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        let daemon = OwnedChild::spawn(&mut command)?;
        let daemon_state: Value = wait_for("private hided state", || {
            let path = state.join("hided.json");
            if !path.exists() {
                return Ok(None);
            }
            Ok(serde_json::from_slice(&read(&path)?).ok())
        })?;
        let fixture = Self {
            local,
            remote,
            ssh,
            state,
            hide: cli.join("hide"),
            hooks: cli.join("hide-agent-hooks"),
            source_cli,
            root,
            removed: false,
            ipc,
            daemon: Some(daemon),
            port: daemon_state["port"]
                .as_u64()
                .context("private daemon port")? as u16,
            token: daemon_state["token"]
                .as_str()
                .context("private daemon token")?
                .into(),
        };
        fixture.event("create_workspace", json!({"path":fixture.local.environment.home.join("project"),"label":"Local fixture","initialize_git":false}))?;
        fixture.event("register_device", json!({"id":"remote","label":"Remote fixture","ssh_alias":"delivery-fixture","herdr_socket_path":fixture.ipc.path().join("r.sock"),"host_consent":true}))?;
        wait_for("connected private SSH device", || {
            let snapshot = fixture.snapshot()?;
            Ok(snapshot
                .pointer("/status/remote")
                .and_then(Value::as_array)
                .is_some_and(|rows| {
                    rows.iter()
                        .any(|row| row["target_id"] == "remote" && row["state"] == "connected")
                })
                .then_some(()))
        })
        .with_context(|| fixture.device_state())?;
        wait_for("consented private helper", || {
            let snapshot = fixture.snapshot()?;
            if let Some(host) = snapshot
                .pointer("/navigator/devices")
                .and_then(Value::as_array)
                .and_then(|rows| rows.iter().find(|row| row["id"] == "remote"))
                .and_then(|row| row.get("host"))
                && matches!(
                    host["state"].as_str(),
                    Some("unavailable" | "unsupported" | "identity_changed")
                )
            {
                bail!("private helper setup failed: {}", host["message"]);
            }
            Ok(snapshot
                .pointer("/navigator/devices")
                .and_then(Value::as_array)
                .is_some_and(|rows| {
                    rows.iter().any(|row| {
                        row["id"] == "remote"
                            && row
                                .pointer("/host/state")
                                .is_some_and(|state| state == "ready")
                    })
                })
                .then_some(()))
        })
        .inspect_err(|_| eprintln!("{}", helper_diagnostics(&fixture.remote.environment)))?;
        fixture.event("create_workspace", json!({"device_id":"remote","path":fixture.remote.environment.home.join("project"),"label":"Remote fixture","initialize_git":false}))?;
        wait_for("registered private remote checkout", || {
            let snapshot = fixture.snapshot()?;
            Ok(snapshot
                .pointer("/status/remote")
                .and_then(Value::as_array)
                .and_then(|rows| rows.iter().find(|row| row["target_id"] == "remote"))
                .and_then(|remote| remote.pointer("/session/workspaces"))
                .and_then(Value::as_array)
                .is_some_and(|rows| {
                    rows.iter().any(|row| {
                        row["device_id"] == "remote"
                            && row["registered"] == true
                            && row["path"]
                                == fixture
                                    .remote
                                    .environment
                                    .home
                                    .join("project")
                                    .to_string_lossy()
                                    .as_ref()
                    })
                })
                .then_some(()))
        })?;
        fixture.wait_bridge(1)?;
        // The device's CLI is a new file, which macOS checks the first time
        // it starts, and a journey's hook runs it inside its budget; start it
        // once here (docs/TESTING.md, rule 8).
        let cli = fixture.remote.environment.home.join("bin/hide");
        wait_for("the device's installed CLI", || {
            Ok(cli.is_file().then_some(()))
        })?;
        let mut version = fixture.remote.environment.command(&cli);
        version.arg("--version");
        successful(version)?;
        Ok(fixture)
    }

    /// The device's project folder as a repository a worktree can branch from.
    pub fn commit_remote_project(&self) -> Result<PathBuf> {
        let project = self.remote.environment.home.join("project");
        let mut commit = self.remote.environment.command("/usr/bin/git");
        commit.arg("-C").arg(&project).args([
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "fixture",
        ]);
        successful(commit)?;
        Ok(project)
    }

    pub fn snapshot(&self) -> Result<Value> {
        Ok(renderer::Renderer::connect(self.port, &self.token)?
            .snapshot()
            .clone())
    }

    fn event(&self, kind: &str, payload: Value) -> Result<()> {
        renderer::Renderer::connect(self.port, &self.token)?.event(kind, payload)
    }

    /// What the core says about the device when it never connected: its
    /// last error, its connection rows and whether it was registered at all.
    fn device_state(&self) -> String {
        match self.snapshot() {
            Ok(snapshot) => format!(
                "last_error={} remote={} devices={}",
                snapshot
                    .pointer("/status/last_error")
                    .unwrap_or(&Value::Null),
                snapshot.pointer("/status/remote").unwrap_or(&Value::Null),
                snapshot
                    .pointer("/navigator/devices")
                    .unwrap_or(&Value::Null),
            ),
            Err(error) => format!("snapshot unreadable: {error:#}"),
        }
    }

    pub fn reconnect_device(&self) -> Result<()> {
        self.event("retry_connect", json!({"target_id":"remote"}))
    }

    pub fn ledger(&self) -> Result<herdr_core::delivery::ledger::Ledger> {
        let path = self.state.join("delivery-ledger.json");
        if !path.exists() {
            return Ok(Default::default());
        }
        Ok(serde_json::from_slice(&read(&path)?)?)
    }

    /// The line the daemon's own doorbell typed into the recipient's pane
    /// for letter `id`, once it has rung. The doorbell waits for the pane to
    /// be quiet for 30 seconds (`docs/delivery.md`), so this waits longer
    /// than a readiness check does.
    pub fn rung(&self, id: &str) -> Result<String> {
        wait_within(
            "the doorbell to ring for the letter",
            Duration::from_secs(90),
            || {
                Ok(self
                    .ledger()?
                    .letters
                    .into_iter()
                    .find(|letter| letter.id == id)
                    .and_then(|letter| letter.bell_line))
            },
        )
    }

    pub fn wait_bridge(&self, minimum: usize) -> Result<()> {
        wait_for("device node pane service ready", || {
            let path = self.state.join("Logs/core.jsonl");
            if !path.exists() {
                return Ok(None);
            }
            let count = String::from_utf8(read(&path)?)?
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .filter(|row| {
                    row["component"] == "remote_host" && row["kind"] == "host.panes_started"
                })
                .count();
            Ok((count >= minimum).then_some(()))
        })
    }
}

impl Run for Fixture {
    fn root(&self) -> &Path {
        &self.root
    }

    fn stop(&mut self) -> Result<()> {
        if let Some(mut daemon) = self.daemon.take() {
            daemon.kill_tree()?;
            wait_for("private hided confirmed exit", || {
                Ok(daemon.try_wait()?.map(|_| ()))
            })?;
        }
        self.ssh.stop()?;
        self.remote.stop()?;
        self.local.stop()?;
        fs::write(
            self.root.join("cleanup-confirmed"),
            "all owned process trees ended\n",
        )?;
        Ok(())
    }

    fn removed(&mut self) {
        self.removed = true;
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // `finish` confirmed every exit before it removed the folder.
        if self.removed {
            return;
        }
        if let Err(error) = self.stop() {
            eprintln!("private delivery fixture cleanup failed, evidence retained: {error}");
        }
    }
}

const SHIM: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <termios.h>
#include <unistd.h>
#include <sys/wait.h>
#include <time.h>
int main(int argc, char **argv) {
  if (argc > 1 && strcmp(argv[1], "--version") == 0) { puts("fixture provider"); return 0; }
  if (argc > 1 && strcmp(argv[1], "auth") == 0) { puts("{\"loggedIn\":false}"); return 0; }
  for (int i = 1; i < argc; i++) {
    if (strcmp(argv[i], "-p") == 0 || strcmp(argv[i], "--print") == 0) return 1;
  }
  struct termios t;
  if (tcgetattr(0, &t) == 0) {
    t.c_lflag &= ~(ICANON | ECHO | IEXTEN);
    t.c_cc[VMIN] = 1; t.c_cc[VTIME] = 0;
    tcsetattr(0, TCSANOW, &t);
  }
  puts("DELIVERY_FIXTURE_READY"); fflush(stdout);
  char line[8192]; size_t size = 0; char byte;
  while (read(0, &byte, 1) == 1) {
    if (byte == '\r' || byte == '\n') {
      line[size] = 0;
      if (strncmp(line, "RUN ", 4) == 0) {
        struct timespec started, ended;
        clock_gettime(CLOCK_MONOTONIC, &started);
        pid_t child = fork();
        if (child == 0) { execl("/bin/sh", "sh", line + 4, NULL); _exit(127); }
        if (child > 0) { int status; waitpid(child, &status, 0); }
        clock_gettime(CLOCK_MONOTONIC, &ended);
        /* The test takes the timing file's existence as the command's end, so
           it appears whole: fopen would show it empty before the write. */
        char timing[8192], staged[8200];
        snprintf(timing, sizeof timing, "%s.ms", line + 4);
        snprintf(staged, sizeof staged, "%s.tmp", timing);
        FILE *out = fopen(staged, "w");
        if (out) {
          fprintf(out, "%lld", (long long)(ended.tv_sec - started.tv_sec) * 1000LL + (ended.tv_nsec - started.tv_nsec) / 1000000LL);
          fclose(out);
          rename(staged, timing);
        }
      }
      size = 0;
    } else if (size + 1 < sizeof line) { line[size++] = byte; }
  }
  return 0;
}
"#;
