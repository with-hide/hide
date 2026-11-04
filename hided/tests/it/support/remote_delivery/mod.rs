//! Isolated process fixture for the actual SSH return-route journey, on the
//! core's own engine: the daemon's node and the device's node each start
//! their panes' holders, and the fixture starts each machine's stand-in
//! agent as the start panel does. Every candidate child has an owned process
//! tree, every holder is ended by its record; counts and reads are capped.

pub mod renderer;
use super::run::{Run, finish};

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use hide_platform::process::{CapturedOutput, OwnedChild, Session, SessionEnd};
use serde_json::{Value, json};

pub const READY_BOUND: Duration = Duration::from_secs(30);
const READ_CAP: usize = 1024 * 1024;

/// How long a holder the fixture ended may take to go.
const HOLDER_END: Duration = Duration::from_secs(10);

/// The prompt every fixture shell draws, so a test types into a pane only
/// once its shell is ready for keys.
pub const PROMPT: &str = "fixture-ready> ";

/// The device's id, as the fixture registers it.
pub const DEVICE: &str = "remote";

#[derive(Clone)]
pub struct Environment {
    pub home: PathBuf,
    values: BTreeMap<OsString, OsString>,
}

impl Environment {
    pub fn new(root: &Path, state_dir: Option<&Path>) -> Result<Self> {
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
        // Nothing of the session that runs the tests reaches the account:
        // a pane's identity, the agent it runs and its tools are the
        // account's own.
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
            ("XDG_CONFIG_HOME", root.join("config").into_os_string()),
            ("XDG_STATE_HOME", root.join("state").into_os_string()),
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

/// The daemon's screen connection, as the web shell's: one typed event in,
/// one snapshot out.
#[derive(Clone)]
pub struct Daemon {
    pub port: u16,
    pub token: String,
}

impl Daemon {
    pub fn snapshot(&self) -> Result<Value> {
        Ok(renderer::Renderer::connect(self.port, &self.token)?
            .snapshot()
            .clone())
    }

    pub fn event(&self, kind: &str, payload: Value) -> Result<()> {
        renderer::Renderer::connect(self.port, &self.token)?.event(kind, payload)
    }

    /// Sizes `pane` as a pane view does once it is drawn, for a pane no page
    /// shows (`viewPane` in `web/e2e/daemon-actions.ts`).
    fn view(&self, pane: &str) -> Result<()> {
        self.event(
            "terminal_viewport",
            json!({"pane_id": pane, "cols": 120, "rows": 40, "new_view": true}),
        )?;
        self.event(
            "terminal_resize",
            json!({"pane_id": pane, "cols": 120, "rows": 40}),
        )
    }
}

/// The pane of the agent on `session` in a snapshot, wherever the snapshot
/// nests its row.
pub fn pane_on_session(value: &Value, session: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if map.get("session_id").and_then(Value::as_str) == Some(session)
                && map.get("agent_kind").is_some_and(|kind| !kind.is_null())
                && let Some(pane) = map.get("pane_id").and_then(Value::as_str)
            {
                return Some(pane.to_owned());
            }
            map.values()
                .find_map(|value| pane_on_session(value, session))
        }
        Value::Array(rows) => rows
            .iter()
            .find_map(|value| pane_on_session(value, session)),
        _ => None,
    }
}

/// The file the stand-in agent in the pane its machine calls `pane` writes
/// once it reads its terminal (`SHIM`), in the account's home.
fn ready_marker(home: &Path, pane: &str) -> PathBuf {
    home.join(".fixture-ready").join(pane.replace(':', "-"))
}

/// The file that hands the stand-in agent in the pane its machine calls
/// `pane` the script to run next (`SHIM`), in the account's home.
fn run_order(home: &Path, pane: &str) -> PathBuf {
    home.join(".fixture-run").join(pane.replace(':', "-"))
}

/// The panes of an account whose stand-in reads its terminal, as the
/// account's machine calls them.
pub fn ready_panes(home: &Path) -> Result<Vec<String>> {
    let folder = home.join(".fixture-ready");
    if !folder.exists() {
        return Ok(Vec::new());
    }
    let mut panes = Vec::new();
    for entry in fs::read_dir(folder)? {
        panes.push(fs::read_to_string(entry?.path())?);
    }
    panes.sort();
    Ok(panes)
}

/// The conversation file a provider keeps for `session`, holding one
/// finished turn. The doorbell rings a Claude Code pane only once its
/// session read says nothing waits for the operator, and the fixture's
/// provider writes no conversation of its own.
fn finished_session(home: &Path, session: &str) -> Result<()> {
    let folder = home.join(".claude/projects/-fixture");
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

/// One machine's account and the stand-in agent the fixture started in its
/// project, registered under its name from inside its pane.
pub struct Agent {
    pub environment: Environment,
    /// The agent's pane as the core names it, which the daemon's events take.
    pub pane: String,
    /// The agent's registration.
    pub registration: String,
    /// The account's `hide` and hook helper.
    pub hide: PathBuf,
    pub hooks: PathBuf,
    /// The device the account is on; `None` is the daemon's own machine.
    device: Option<&'static str>,
    daemon: Daemon,
    root: PathBuf,
    sequence: u32,
}

impl Agent {
    /// Starts the stand-in on `session` in the account's project, as the
    /// start panel resumes a session, and registers it as `name`.
    #[allow(clippy::too_many_arguments)]
    fn start(
        daemon: &Daemon,
        root: PathBuf,
        environment: Environment,
        device: Option<&'static str>,
        hide: PathBuf,
        hooks: PathBuf,
        session: &str,
        name: &str,
        host_scope: &str,
    ) -> Result<Self> {
        finished_session(&environment.home, session)?;
        let mut start = json!({
            "checkout_path": environment.home.join("project"),
            "provider": "claude",
            "resume_session_id": session,
            "request_id": format!("fixture-{session}"),
        });
        if let Some(device) = device {
            start["device_id"] = device.into();
        }
        daemon.event("agent_start_in_checkout", start)?;
        // The engine learns the pane's session from the start's `--resume`.
        let pane = wait_for("the fixture agent on its session", || {
            Ok(pane_on_session(&daemon.snapshot()?, session))
        })?;
        let mut agent = Self {
            environment,
            pane,
            registration: String::new(),
            hide,
            hooks,
            device,
            daemon: daemon.clone(),
            root,
            sequence: 0,
        };
        agent.view(&agent.pane.clone())?;
        let registered = agent
            .run_in_pane(&format!(
                "{} agent register --host-scope {} --session {session} \
                 --instance \"$HIDE_PANE_ID\" --name {name} --pane \"$HIDE_PANE_ID\"",
                quote(&agent.hide),
                quote(host_scope)
            ))?
            .value()?;
        agent.registration = registered["id"]
            .as_str()
            .context("the fixture agent's registration")?
            .to_owned();
        Ok(agent)
    }

    /// The pane the core calls `pane` as the account's machine calls it.
    pub fn own_pane<'a>(&self, pane: &'a str) -> Result<&'a str> {
        match self.device {
            None => Ok(pane),
            Some(device) => pane
                .strip_prefix(&format!("remote:{device}:pane:"))
                .with_context(|| format!("{pane} is not a pane of {device}")),
        }
    }

    /// The pane the account's machine calls `pane` as the core calls it.
    pub fn core_pane(&self, pane: &str) -> String {
        match self.device {
            None => pane.to_owned(),
            Some(device) => format!("remote:{device}:pane:{pane}"),
        }
    }

    /// Sizes `pane` and waits until its stand-in reads its terminal.
    fn view(&self, pane: &str) -> Result<()> {
        self.daemon.view(pane)?;
        let marker = ready_marker(&self.environment.home, self.own_pane(pane)?);
        wait_for("the stand-in reading its terminal", || {
            Ok(marker.exists().then_some(()))
        })
    }

    /// What the provider's own hook reports for a pane it runs in: the native
    /// session Hide waits for before it calls a spawned agent started. The
    /// fixture's provider is a shim that reports nothing by itself, so its
    /// SessionStart hook runs from inside the pane.
    pub fn report_session(&mut self, pane: &str, session: &str) -> Result<()> {
        self.view(pane)?;
        finished_session(&self.environment.home, session)?;
        let started = json!({
            "session_id": session,
            "hook_event_name": "SessionStart",
            "source": "startup",
            "cwd": self.environment.home.join("project"),
        });
        let reported = self.run_in(
            pane,
            &format!(
                "printf '%s' {} | {} hook --runtime claude-code --event SessionStart",
                quote(started.to_string()),
                quote(&self.hooks)
            ),
        )?;
        ensure!(
            reported.status == 0,
            "the SessionStart hook failed: {}",
            reported.stderr
        );
        Ok(())
    }

    pub fn run_in_pane(&mut self, command: &str) -> Result<PaneOutput> {
        let pane = self.pane.clone();
        self.run_in(&pane, command)
    }

    /// Runs a command in a pane of this account whose provider is the
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
        // The script reaches the stand-in by a file of its pane, not by the
        // pane's keys: a key is the operator's input, which stands as a
        // draft in front of every later letter until the agent's prompt hook
        // says it was sent, and a fixture command is no one's prompt.
        let order = run_order(&self.environment.home, self.own_pane(pane)?);
        hide_platform::fs::private::create_dir_all(order.parent().context("run order folder")?)?;
        let staged = order.with_extension("tmp");
        fs::write(&staged, script.display().to_string())?;
        fs::rename(&staged, &order)?;
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

    /// The `value` of a `hide agent` answer, which does not use the envelope
    /// of the mailbox commands.
    pub fn value(&self) -> Result<Value> {
        ensure!(
            self.status == 0,
            "agent command failed: {} {}",
            self.stdout,
            self.stderr
        );
        let answer: Value = serde_json::from_str(&self.stdout).context("agent command JSON")?;
        ensure!(answer["ok"] == true, "agent command refused: {answer}");
        answer.get("value").cloned().context("agent command value")
    }
}

/// Ends every pane holder `state_dir` records: a holder outlives the node
/// that started it by design, so the fixture that started the node owns it.
/// Each pane's program session is ended by the pid and start time its record
/// names, and its holder ends with it; a holder still running after
/// [`HOLDER_END`] fails the stop.
fn end_holders(state_dir: &Path) -> Result<()> {
    let folder = hide_kit::layout::pane_records(state_dir);
    if !folder.exists() {
        return Ok(());
    }
    let identity = |record: &Value, key: &str| -> Option<(u32, u64)> {
        let found = record.get(key)?;
        Some((
            u32::try_from(found.get("pid")?.as_u64()?).ok()?,
            found.get("started")?.as_u64()?,
        ))
    };
    let runs = |(pid, started): (u32, u64)| {
        hide_platform::process::is_alive(pid)
            && hide_platform::process::start_time(pid).ok() == Some(started)
    };
    let mut holders = Vec::new();
    for entry in fs::read_dir(&folder)? {
        let path = entry?.path();
        if path.extension() != Some(OsStr::new("json")) {
            continue;
        }
        let record: Value = serde_json::from_slice(&read(&path)?)
            .with_context(|| format!("holder record {}", path.display()))?;
        if let Some((pid, started)) = identity(&record, "program").filter(|program| runs(*program))
        {
            match Session::left_by(pid, started).and_then(|mut session| session.end(HOLDER_END))? {
                SessionEnd::Ended => {}
                SessionEnd::Unconfirmed { left } => {
                    bail!("the pane program of {} left {left:?}", path.display())
                }
            }
        }
        holders.extend(identity(&record, "holder").map(|holder| (path, holder)));
    }
    for (path, holder) in holders {
        wait_within("the fixture's holders to end", HOLDER_END, || {
            Ok((!runs(holder)).then_some(()))
        })
        .with_context(|| format!("the holder of {} still runs", path.display()))?;
    }
    Ok(())
}

pub struct Fixture {
    pub local: Agent,
    pub remote: Agent,
    pub ssh: super::sshd::Sshd,
    pub state: PathBuf,
    pub hide: PathBuf,
    pub hooks: PathBuf,
    root: PathBuf,
    removed: bool,
    /// The device's state folder, short enough for its sockets.
    ipc: tempfile::TempDir,
    daemon: Option<OwnedChild>,
    screen: Daemon,
}

impl Fixture {
    /// Runs one journey in a fresh fixture and ends it by how the journey
    /// went (`support::run::finish`): a passing journey leaves no run
    /// directory, a failing one keeps it for the lane to upload. The only way
    /// to start a fixture, so no test has to remember either half.
    pub fn journey(body: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        let mut fixture = Self::start()?;
        let journey = body(&mut fixture);
        finish(&mut fixture, journey)
    }

    fn start() -> Result<Self> {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let source_cli = repository.join("target/debug");
        // A node starts each pane's holder from beside its own program.
        const SHIPPED: [&str; 4] = ["hide", "hided", "hide-agent-hooks", "hide-holder"];
        for name in SHIPPED {
            ensure!(
                source_cli.join(name).is_file(),
                "build this worktree's CLI binaries before remote_delivery (scripts/verify-cargo.sh cli)"
            );
        }
        let artifacts = repository.join("agents/runs");
        fs::create_dir_all(&artifacts)?;
        let root = tempfile::Builder::new()
            .prefix("rd-")
            .tempdir_in(artifacts)?
            .keep();
        let ipc = tempfile::Builder::new().prefix("rd-").tempdir_in("/tmp")?;
        // The device's node binds its pane socket and its holders' sockets
        // under the device's state folder, which must stay short enough for
        // a Unix socket path; so must the daemon's own.
        let device_state = ipc.path().join("d");
        hide_platform::fs::private::create_dir_all(&device_state)?;
        let state = ipc.path().join("s");
        hide_platform::fs::private::create_dir_all(&state)?;
        let mut local_env = Environment::new(&root.join("l"), None)?;
        let remote_env = Environment::new(&root.join("r"), Some(&device_state))?;
        // The device installs shipped executables, which carry no debug
        // data. Stage this candidate's code with the same property: hashing
        // and uploading a CI debug image can exhaust helper setup's bound.
        // Never strip the worktree's build output or another candidate.
        let cli = root.join("cli");
        fs::create_dir(&cli)?;
        for name in SHIPPED {
            super::shipped::place(&source_cli.join(name), &cli.join(name), &local_env)?;
        }
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
        fs::create_dir_all(local_env.home.join(".ssh"))?;
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
            remote_env.home.join("helper").into_os_string(),
        );
        local_env.set(
            "HIDE_HOST_CLI_DIR",
            remote_env.home.join("bin").into_os_string(),
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
        let mut daemon = Some(OwnedChild::spawn(&mut command)?);
        // Until the fixture holds them, a failed start ends what it started.
        let started = (|| -> Result<(Daemon, Agent, Agent)> {
            let daemon_state: Value = wait_for("private hided state", || {
                let path = state.join("hided.json");
                if !path.exists() {
                    return Ok(None);
                }
                Ok(serde_json::from_slice(&read(&path)?).ok())
            })?;
            let screen = Daemon {
                port: daemon_state["port"]
                    .as_u64()
                    .context("private daemon port")? as u16,
                token: daemon_state["token"]
                    .as_str()
                    .context("private daemon token")?
                    .into(),
            };
            Self::connect_device(&screen, &state, &remote_env)?;
            Self::add_local_project(&screen, &local_env.home.join("project"))?;
            let local = Agent::start(
                &screen,
                root.join("l"),
                local_env.clone(),
                None,
                cli.join("hide"),
                cli.join("hide-agent-hooks"),
                "fixture-local-session",
                "local-parent",
                // The core's own engine is named by its folder in the
                // daemon's state (`hide_kit::layout::engine_dir`).
                &state.join("engine").to_string_lossy(),
            )?;
            let remote_hide = remote_env.home.join("bin/hide");
            let remote_hooks = fs::canonicalize(&remote_hide)?
                .parent()
                .context("installed private CLI directory")?
                .join("hide-agent-hooks");
            ensure!(
                remote_hooks.is_file(),
                "private remote kit lacks its hook sibling"
            );
            let remote = Agent::start(
                &screen,
                root.join("r"),
                remote_env.clone(),
                Some(DEVICE),
                remote_hide,
                remote_hooks,
                "fixture-remote-session",
                "remote-child",
                // A device's agents are scoped by the device.
                DEVICE,
            )?;
            Ok((screen, local, remote))
        })();
        match started {
            Ok((screen, local, remote)) => Ok(Self {
                local,
                remote,
                ssh,
                state,
                hide: cli.join("hide"),
                hooks: cli.join("hide-agent-hooks"),
                root,
                removed: false,
                ipc,
                daemon: daemon.take(),
                screen,
            }),
            Err(error) => {
                let mut ssh = ssh;
                let stopped = Self::end(&mut daemon, &mut ssh, &[&state, &device_state]);
                eprintln!("the fixture's run directory: {}", root.display());
                Err(match stopped {
                    Ok(()) => error,
                    Err(stop) => error.context(format!("and its stop failed: {stop:#}")),
                })
            }
        }
    }

    /// Registers the device and its project and waits for its node's pane
    /// service and installed CLI.
    /// Adds the account's project on this machine as Add project does, and
    /// waits until the navigator lists it, so the local agent starts in a
    /// checkout the core knows.
    fn add_local_project(screen: &Daemon, project: &Path) -> Result<()> {
        let path = project.to_string_lossy().into_owned();
        screen.event(
            "create_workspace",
            json!({"path":path,"label":"Local fixture","initialize_git":false}),
        )?;
        wait_for("registered private local checkout", || {
            let snapshot = screen.snapshot()?;
            let listed = |row: &Value| row["path"] == path.as_str();
            Ok(snapshot
                .pointer("/navigator/workspaces")
                .and_then(Value::as_array)
                .is_some_and(|rows| {
                    rows.iter().any(|row| {
                        listed(row)
                            || row["checkouts"]
                                .as_array()
                                .is_some_and(|checkouts| checkouts.iter().any(listed))
                    })
                })
                .then_some(()))
        })
    }

    fn connect_device(screen: &Daemon, state: &Path, remote: &Environment) -> Result<()> {
        let device_state = || match screen.snapshot() {
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
        };
        screen.event("register_device", json!({"id":DEVICE,"label":"Remote fixture","ssh_alias":"delivery-fixture","host_consent":true}))?;
        wait_for("connected private SSH device", || {
            let snapshot = screen.snapshot()?;
            Ok(snapshot
                .pointer("/status/remote")
                .and_then(Value::as_array)
                .is_some_and(|rows| {
                    rows.iter()
                        .any(|row| row["target_id"] == DEVICE && row["state"] == "connected")
                })
                .then_some(()))
        })
        .with_context(device_state)?;
        wait_for("consented private helper", || {
            let snapshot = screen.snapshot()?;
            if let Some(host) = snapshot
                .pointer("/navigator/devices")
                .and_then(Value::as_array)
                .and_then(|rows| rows.iter().find(|row| row["id"] == DEVICE))
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
                        row["id"] == DEVICE
                            && row
                                .pointer("/host/state")
                                .is_some_and(|state| state == "ready")
                    })
                })
                .then_some(()))
        })
        .inspect_err(|_| eprintln!("{}", helper_diagnostics(remote)))?;
        let project = remote.home.join("project");
        screen.event("create_workspace", json!({"device_id":DEVICE,"path":project,"label":"Remote fixture","initialize_git":false}))?;
        wait_for("registered private remote checkout", || {
            let snapshot = screen.snapshot()?;
            Ok(snapshot
                .pointer("/status/remote")
                .and_then(Value::as_array)
                .and_then(|rows| rows.iter().find(|row| row["target_id"] == DEVICE))
                .and_then(|remote| remote.pointer("/session/workspaces"))
                .and_then(Value::as_array)
                .is_some_and(|rows| {
                    rows.iter().any(|row| {
                        row["device_id"] == DEVICE
                            && row["registered"] == true
                            && row["path"] == project.to_string_lossy().as_ref()
                    })
                })
                .then_some(()))
        })?;
        wait_bridge(state, 1)?;
        // The device's CLI is a new file, which macOS checks the first time
        // it starts, and a journey's hook runs it inside its budget; start it
        // once here (docs/TESTING.md, rule 8).
        let cli = remote.home.join("bin/hide");
        wait_for("the device's installed CLI", || {
            Ok(cli.is_file().then_some(()))
        })?;
        let mut version = remote.command(&cli);
        version.arg("--version");
        successful(version)?;
        Ok(())
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
        self.screen.snapshot()
    }

    fn event(&self, kind: &str, payload: Value) -> Result<()> {
        self.screen.event(kind, payload)
    }

    pub fn reconnect_device(&self) -> Result<()> {
        self.event("retry_connect", json!({"target_id":DEVICE}))
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
        wait_bridge(&self.state, minimum)
    }

    /// Ends the daemon, the device's sshd and every holder either node
    /// started, confirming each exit.
    fn end(
        daemon: &mut Option<OwnedChild>,
        ssh: &mut super::sshd::Sshd,
        states: &[&Path],
    ) -> Result<()> {
        if let Some(mut daemon) = daemon.take() {
            daemon.kill_tree()?;
            wait_for("private hided confirmed exit", || {
                Ok(daemon.try_wait()?.map(|_| ()))
            })?;
        }
        ssh.stop()?;
        for state in states {
            end_holders(state)?;
        }
        Ok(())
    }
}

/// Waits until the daemon at `state` logged `minimum` starts of a device
/// node's pane service.
fn wait_bridge(state: &Path, minimum: usize) -> Result<()> {
    wait_for("device node pane service ready", || {
        let path = state.join("Logs/core.jsonl");
        if !path.exists() {
            return Ok(None);
        }
        let count = String::from_utf8(read(&path)?)?
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|row| {
                row["component"] == "remote_host" && row["kind"] == "host.holders_started"
            })
            .count();
        Ok((count >= minimum).then_some(()))
    })
}

impl Run for Fixture {
    fn root(&self) -> &Path {
        &self.root
    }

    fn stop(&mut self) -> Result<()> {
        let device_state = self.ipc.path().join("d");
        Self::end(
            &mut self.daemon,
            &mut self.ssh,
            &[&self.state, &device_state],
        )?;
        // The state folders sit under /tmp for their sockets' sake; their
        // records stay with the run, which a failed journey keeps.
        for (state, kept) in [(&self.state, "logs-local"), (&device_state, "logs-device")] {
            let logs = state.join("Logs");
            if !logs.is_dir() {
                continue;
            }
            fs::create_dir_all(self.root.join(kept))?;
            for entry in fs::read_dir(&logs)? {
                let path = entry?.path();
                if path.is_file() {
                    fs::copy(&path, self.root.join(kept).join(path.file_name().unwrap()))?;
                }
            }
        }
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

/// The stand-in agent: it reads its terminal raw, draws Claude Code's empty
/// composer again after each Enter, marks itself ready under
/// `$HOME/.fixture-ready` by the pane its machine names (`HIDE_PANE_ID`), and
/// runs each script `$HOME/.fixture-run` hands that pane (`run_order`),
/// writing the script's run time beside it once the script ended.
const SHIM: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <termios.h>
#include <poll.h>
#include <unistd.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <time.h>
static void composer(void) {
  fputs("\r\n────────────────\r\n❯ \r\n────────────────\r\n", stdout);
  fflush(stdout);
}
/* Written whole by a rename, so a reader never sees it half made. */
static void ready(void) {
  const char *home = getenv("HOME"), *pane = getenv("HIDE_PANE_ID");
  if (!home || !pane || !*pane) return;
  char folder[4096], name[4096], staged[4200];
  snprintf(folder, sizeof folder, "%s/.fixture-ready", home);
  mkdir(folder, 0700);
  snprintf(name, sizeof name, "%s/", folder);
  size_t at = strlen(name);
  for (const char *c = pane; *c && at + 1 < sizeof name; c++) name[at++] = *c == ':' ? '-' : *c;
  name[at] = 0;
  snprintf(staged, sizeof staged, "%s.tmp", name);
  FILE *out = fopen(staged, "w");
  if (!out) return;
  fputs(pane, out);
  fclose(out);
  rename(staged, name);
}
/* The script named in this pane's run order, if one waits: taken, run, and
   its run time written beside it whole, since the test takes the timing
   file's existence as the command's end. */
static void run(void) {
  const char *home = getenv("HOME"), *pane = getenv("HIDE_PANE_ID");
  if (!home || !pane || !*pane) return;
  char order[4096];
  snprintf(order, sizeof order, "%s/.fixture-run/", home);
  size_t at = strlen(order);
  for (const char *c = pane; *c && at + 1 < sizeof order; c++) order[at++] = *c == ':' ? '-' : *c;
  order[at] = 0;
  FILE *in = fopen(order, "r");
  if (!in) return;
  char script[4096];
  size_t size = fread(script, 1, sizeof script - 1, in);
  fclose(in);
  unlink(order);
  script[size] = 0;
  struct timespec started, ended;
  clock_gettime(CLOCK_MONOTONIC, &started);
  pid_t child = fork();
  if (child == 0) { execl("/bin/sh", "sh", script, NULL); _exit(127); }
  if (child > 0) { int status; waitpid(child, &status, 0); }
  clock_gettime(CLOCK_MONOTONIC, &ended);
  char timing[4200], staged[4300];
  snprintf(timing, sizeof timing, "%s.ms", script);
  snprintf(staged, sizeof staged, "%s.tmp", timing);
  FILE *out = fopen(staged, "w");
  if (!out) return;
  fprintf(out, "%lld", (long long)(ended.tv_sec - started.tv_sec) * 1000LL + (ended.tv_nsec - started.tv_nsec) / 1000000LL);
  fclose(out);
  rename(staged, timing);
}
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
  composer();
  ready();
  /* Keys are read as they come, and the run order is looked at every 20 ms;
     the terminal closing ends the stand-in. */
  struct pollfd input = { .fd = 0, .events = POLLIN };
  char byte;
  for (;;) {
    int woke = poll(&input, 1, 20);
    if (woke > 0) {
      if (read(0, &byte, 1) != 1) return 0;
      if (byte == '\r' || byte == '\n') composer();
    }
    run();
  }
}
"#;
