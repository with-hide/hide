//! Two machines on one host, for the node that dials its core (PRD
//! core-host-node-remote-core): the core machine is a private account behind
//! the fixture's own SSH server, with its own Herdr, state folder and core;
//! the screen machine is a second private account with its own Herdr. The
//! core runs under a fixture machine id, so the two are two nodes.
//!
//! Every process here is owned by the fixture and ended by it; nothing
//! reaches the operator's Herdr, state or SSH.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, ensure};
use hide_platform::process::OwnedChild;
use serde_json::{Value, json};

use super::remote_delivery::renderer::Renderer;
use super::remote_delivery::{Environment, capture, read, successful, wait_for};
use super::run::{Run, finish};
use super::sshd::Sshd;

/// The core machine's node id.
pub const CORE_NODE: &str = "fixture-core-machine";
pub const ALIAS: &str = "core-fixture";

/// A private Herdr server of one account.
pub struct Herdr {
    pub environment: Environment,
    pub socket: PathBuf,
    binary: PathBuf,
    server: Option<OwnedChild>,
}

impl Herdr {
    /// Makes `folder` a repository with this account's git, so the
    /// operator's git configuration never reaches the fixture.
    pub fn git_init(&self, folder: &Path) -> Result<()> {
        fs::create_dir_all(folder)?;
        let mut init = self.environment.command("/usr/bin/git");
        init.args(["init", "-q"]).current_dir(folder);
        successful(init)?;
        Ok(())
    }

    pub fn start(
        root: &Path,
        environment: Environment,
        socket: PathBuf,
        binary: &Path,
    ) -> Result<Self> {
        let log = File::create(root.join("herdr.log"))?;
        let mut command = environment.command(binary);
        command
            .arg("server")
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        let server = OwnedChild::spawn(&mut command)?;
        let herdr = Self {
            environment,
            socket,
            binary: binary.to_owned(),
            server: Some(server),
        };
        wait_for("private Herdr socket", || {
            let mut command = herdr.environment.command(&herdr.binary);
            command.args(["api", "snapshot"]);
            let output = capture(command)?;
            Ok(output.status.success().then_some(()))
        })?;
        Ok(herdr)
    }

    /// Runs `herdr <args>` against this server and answers its JSON.
    pub fn run(&self, args: &[&str]) -> Result<Value> {
        let mut command = self.environment.command(&self.binary);
        command.args(args);
        serde_json::from_slice(&successful(command)?).context("private Herdr answer JSON")
    }

    /// Runs `herdr <args>` against this server, for a command that answers
    /// nothing.
    pub fn write(&self, args: &[&str]) -> Result<()> {
        let mut command = self.environment.command(&self.binary);
        command.args(args);
        successful(command).map(|_| ())
    }

    /// A Herdr workspace at `folder`, and its root pane.
    pub fn workspace_at(&self, folder: &Path) -> Result<String> {
        let created = self.run(&[
            "workspace",
            "create",
            "--label",
            "fixture",
            "--cwd",
            folder.to_str().context("fixture path UTF-8")?,
            "--focus",
        ])?;
        Ok(created
            .pointer("/result/root_pane/pane_id")
            .and_then(Value::as_str)
            .context("private Herdr root pane")?
            .to_owned())
    }

    pub fn stop(&mut self) -> Result<()> {
        if let Some(mut server) = self.server.take() {
            let mut command = self.environment.command(&self.binary);
            command.args(["server", "stop"]);
            let stopped = capture(command)?;
            if !stopped.status.success() {
                server.kill_tree()?;
            }
            wait_for("private Herdr confirmed exit", || {
                Ok(server.try_wait()?.map(|_| ()))
            })?;
        }
        Ok(())
    }
}

pub struct Fixture {
    /// The core machine: its account, Herdr and state folder.
    pub core: Herdr,
    pub core_state: PathBuf,
    /// The screen machine: its account and Herdr.
    pub screen: Herdr,
    pub ssh: Sshd,
    pub hided: PathBuf,
    pub root: PathBuf,
    removed: bool,
    _ipc: tempfile::TempDir,
    daemon: Option<OwnedChild>,
    /// The screen machine's hided in the node role, once started.
    node: Option<OwnedChild>,
    port: u16,
    token: String,
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
        let bin = PathBuf::from(
            std::env::var_os("HIDE_E2E_HERDR_BIN")
                .context("set HIDE_E2E_HERDR_BIN to the pinned binary")?,
        );
        let cli = std::env::var_os("HIDE_E2E_CLI_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| repository.join("target/debug"));
        let hided = cli.join("hided");
        ensure!(
            hided.is_file(),
            "build this worktree's hided before remote_core"
        );
        let artifacts = repository.join("agents/runs");
        fs::create_dir_all(&artifacts)?;
        let root = tempfile::Builder::new()
            .prefix("rc-")
            .tempdir_in(artifacts)?
            .keep();
        // Sockets and the core's state folder stay short enough for a Unix
        // socket path.
        let ipc = tempfile::Builder::new().prefix("rc-").tempdir_in("/tmp")?;
        let core_state = ipc.path().join("c");
        hide_platform::fs::private::create_dir_all(&core_state)?;
        let core_socket = ipc.path().join("c.sock");
        let screen_socket = ipc.path().join("s.sock");
        let mut core_env = Environment::new(&root.join("c"), &core_socket, &bin, None)?;
        let screen_env = Environment::new(&root.join("s"), &screen_socket, &bin, None)?;
        core_env.set("HIDE_MACHINE_ID", CORE_NODE);
        core_env.set("HIDE_STATE_DIR", core_state.as_os_str());
        core_env.set("HIDE_KEEP_ALIVE", "1");
        core_env.set("HIDE_OPEN_COMMAND", "/usr/bin/true");
        core_env.set(
            "HIDE_TAILSCALE_BIN",
            core_env.home.join("absent-tailscale").into_os_string(),
        );
        for environment in [&core_env, &screen_env] {
            let mut command = environment.command("/usr/bin/git");
            command
                .args(["-c", "init.defaultBranch=main", "-C"])
                .arg(environment.home.join("project"))
                .args(["init", "-q"]);
            successful(command)?;
        }
        let ssh_dir = screen_env.home.join(".ssh");
        fs::create_dir_all(&ssh_dir)?;
        let core = Herdr::start(&root.join("c"), core_env.clone(), core_socket, &bin)?;
        let screen = Herdr::start(&root.join("s"), screen_env, screen_socket, &bin)?;
        // The core machine is the system's own `sshd`, whose sessions run
        // in the core account's environment.
        let ssh = Sshd::start(&root.join("sshd"), &core_env.values())?;
        fs::write(ssh_dir.join("config"), ssh.client_config(ALIAS))?;
        let mut fixture = Self {
            core,
            core_state,
            screen,
            ssh,
            hided,
            root,
            removed: false,
            _ipc: ipc,
            daemon: None,
            node: None,
            port: 0,
            token: String::new(),
        };
        fixture.start_core()?;
        Ok(fixture)
    }

    /// Starts the core machine's hided on its state folder and waits for
    /// its attach socket. A core killed before leaves its state and its
    /// attach socket behind: the state is removed so this start's is the
    /// one waited for, and the socket is left for the new core to replace.
    pub fn start_core(&mut self) -> Result<()> {
        ensure!(self.daemon.is_none(), "the core's hided already runs");
        let state = self.core_state.join("hided.json");
        let socket = self.core_state.join("node-attach-socket");
        // A core started again finds its predecessor's record still there;
        // every bind records a new token, so a changed record is this core's.
        let stale = fs::read(&socket).ok();
        match fs::remove_file(&state) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        let log = File::options()
            .create(true)
            .append(true)
            .open(self.root.join("core-hided.log"))?;
        let mut command = self.core.environment.command(&self.hided);
        command
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        self.daemon = Some(OwnedChild::spawn(&mut command)?);
        let daemon_state: Value = wait_for("private core hided state", || {
            if !state.exists() {
                return Ok(None);
            }
            Ok(serde_json::from_slice(&read(&state)?).ok())
        })?;
        self.port = daemon_state["port"].as_u64().context("core port")? as u16;
        self.token = daemon_state["token"]
            .as_str()
            .context("core token")?
            .to_owned();
        wait_for("core attach socket", || {
            let record = fs::read(&socket).ok();
            Ok((record.is_some() && record != stale).then_some(()))
        })?;
        Ok(())
    }

    /// Records the core machine as this screen machine's core, as the move
    /// will (layer 5).
    pub fn placement(&self) -> hided::placement::Placement {
        hided::placement::Placement {
            alias: ALIAS.to_owned(),
            node: CORE_NODE.to_owned(),
            program: self.hided.display().to_string(),
            state_dir: Some(self.core_state.display().to_string()),
            move_intent: None,
            disconnected: false,
        }
    }

    pub fn screen_home(&self) -> &Path {
        &self.screen.environment.home
    }

    /// Starts the screen machine's hided with the core machine recorded as
    /// its core: it runs in the node role. Answers its port and token.
    pub fn start_node(&mut self) -> Result<(u16, String)> {
        self.start_node_with(&self.hided.clone())
    }

    /// The screen machine's state folder, which records its core.
    pub fn node_state(&self) -> PathBuf {
        self._ipc.path().join("s")
    }

    /// `program` run as the screen machine's account, on its state folder.
    pub fn screen_command(&self, program: &Path) -> std::process::Command {
        let mut environment = self.screen.environment.clone();
        environment.set("HIDE_STATE_DIR", self.node_state().as_os_str());
        environment.set("HIDE_OPEN_COMMAND", "/usr/bin/true");
        environment.command(program)
    }

    /// `program` run as the core machine's account, on the core's state
    /// folder.
    pub fn core_command(&self, program: &Path) -> std::process::Command {
        let mut environment = self.core.environment.clone();
        environment.set("HIDE_STATE_DIR", self.core_state.as_os_str());
        environment.set("HIDE_OPEN_COMMAND", "/usr/bin/true");
        environment.command(program)
    }

    /// Sends the screen machine's hided `signal` and waits for it to end.
    pub fn signal_node(&mut self, signal: i32) -> Result<()> {
        let mut node = self.node.take().context("no node hided runs")?;
        let pid = i32::try_from(node.id())?;
        // SAFETY: the pid is the fixture's own unreaped child.
        ensure!(
            unsafe { libc::kill(pid, signal) } == 0,
            "the node was not signalled"
        );
        wait_for("private node hided confirmed exit", || {
            Ok(node.try_wait()?.map(|_| ()))
        })?;
        Ok(())
    }

    /// Sends the screen machine's hided `signal`, leaving it the fixture's.
    pub fn signal_running_node(&self, signal: i32) -> Result<()> {
        let node = self.node.as_ref().context("no node hided runs")?;
        let pid = i32::try_from(node.id())?;
        // SAFETY: the pid is the fixture's own unreaped child.
        ensure!(
            unsafe { libc::kill(pid, signal) } == 0,
            "the node was not signalled"
        );
        Ok(())
    }

    /// The attach role processes running for the core's state folder, as
    /// the process table lists them.
    pub fn attach_processes(&self) -> Result<Vec<String>> {
        let mut command = std::process::Command::new("/bin/ps");
        command.args(["-axo", "pid=,command="]);
        let listed = String::from_utf8(successful(command)?)?;
        let wanted = format!("attach --state-dir {}", self.core_state.display());
        Ok(listed
            .lines()
            .filter(|line| line.contains(&wanted) && !line.contains("/bin/ps"))
            .map(str::to_owned)
            .collect())
    }

    /// Ends the core machine's hided: its machine still answers SSH, and
    /// its attach socket's record stays behind.
    pub fn stop_core(&mut self) -> Result<()> {
        if let Some(mut daemon) = self.daemon.take() {
            daemon.kill_tree()?;
            wait_for("private core hided confirmed exit", || {
                Ok(daemon.try_wait()?.map(|_| ()))
            })?;
        }
        Ok(())
    }

    /// [`Fixture::start_node`] with the `hided` at `hided`.
    pub fn start_node_with(&mut self, hided: &Path) -> Result<(u16, String)> {
        let state = self.node_state();
        hide_platform::fs::private::create_dir_all(&state)?;
        let placement = self.placement();
        let record = hided::placement::record_path(&state);
        fs::write(
            &record,
            serde_json::to_vec(&json!({
                "alias": placement.alias,
                "node": placement.node,
                "program": placement.program,
                "state_dir": placement.state_dir,
            }))?,
        )?;
        hide_platform::fs::private::restrict_to_owner(&record)?;
        self.spawn_node(hided)
    }

    /// Starts the screen machine's hided again on the record its last run
    /// left, after [`Fixture::signal_node`].
    pub fn restart_node(&mut self) -> Result<(u16, String)> {
        ensure!(self.node.is_none(), "the node hided still runs");
        self.spawn_node(&self.hided.clone())
    }

    fn spawn_node(&mut self, hided: &Path) -> Result<(u16, String)> {
        let state = self.node_state();
        // A node killed before it could clean up leaves its state behind;
        // this start's state is the one waited for.
        match fs::remove_file(state.join("hided.json")) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        let mut environment = self.screen.environment.clone();
        environment.set("HIDE_STATE_DIR", state.as_os_str());
        environment.set("HIDE_KEEP_ALIVE", "1");
        environment.set("HIDE_OPEN_COMMAND", "/usr/bin/true");
        let log = File::create(self.root.join("node-hided.log"))?;
        let mut command = environment.command(hided);
        command
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        self.node = Some(OwnedChild::spawn(&mut command)?);
        let daemon_state: Value = wait_for("private node hided state", || {
            let path = state.join("hided.json");
            if !path.exists() {
                return Ok(None);
            }
            Ok(serde_json::from_slice(&read(&path)?).ok())
        })?;
        let port = daemon_state["port"].as_u64().context("node port")? as u16;
        let token = daemon_state["token"]
            .as_str()
            .context("node token")?
            .to_owned();
        Ok((port, token))
    }

    /// The core's diagnostic rows of `component` and `kind`, as written now.
    pub fn core_log(&self, component: &str, kind: &str) -> Result<Vec<Value>> {
        log_rows(&self.core_state.join("Logs/core.jsonl"), component, kind)
    }

    /// The node role's diagnostic rows of `component` and `kind`.
    pub fn node_log(&self, component: &str, kind: &str) -> Result<Vec<Value>> {
        log_rows(&self._ipc.path().join("s/Logs/core.jsonl"), component, kind)
    }

    /// The core's loopback port and screen token.
    pub fn core_screen(&self) -> (u16, String) {
        (self.port, self.token.clone())
    }

    pub fn core_home(&self) -> &Path {
        &self.core.environment.home
    }

    pub fn snapshot(&self) -> Result<Value> {
        Ok(Renderer::connect(self.port, &self.token)?
            .snapshot()
            .clone())
    }

    pub fn event(&self, kind: &str, payload: Value) -> Result<()> {
        Renderer::connect(self.port, &self.token)?.event(kind, payload)
    }

    /// The core's device row for `node`, once it is in the snapshot.
    pub fn device(&self, node: &str) -> Result<Option<Value>> {
        Ok(self
            .snapshot()?
            .pointer("/navigator/devices")
            .and_then(Value::as_array)
            .and_then(|rows| rows.iter().find(|row| row["id"] == node))
            .cloned())
    }

    /// The core's records of `kind` once `ready` holds for them: a daemon's
    /// log writer writes a record after the answer the test already read.
    pub fn core_log_until(
        &self,
        component: &str,
        kind: &str,
        ready: impl Fn(&[Value]) -> bool,
    ) -> Result<Vec<Value>> {
        log_until(
            &self.core_state.join("Logs/core.jsonl"),
            component,
            kind,
            ready,
        )
    }

    /// What the core logged of its nodes' links, in order, for a failure's
    /// message: its log lives in a folder the fixture removes as it ends.
    pub fn core_link_records(&self) -> Vec<String> {
        let path = self.core_state.join("Logs/core.jsonl");
        let Ok(text) = read(&path) else {
            return vec![format!("no log at {}", path.display())];
        };
        String::from_utf8_lossy(&text)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|row| row["component"] == "node_link" || row["component"] == "remote_host")
            .map(|row| {
                format!(
                    "{} {}",
                    row["kind"],
                    row["reason"]
                        .as_str()
                        .or(row["error"].as_str())
                        .unwrap_or_default()
                )
            })
            .collect()
    }

    /// Whether `text` is in anything either daemon logged: its diagnostic
    /// log or its standard error.
    pub fn logs_mention(&self, text: &str) -> Result<bool> {
        for path in [
            self.core_state.join("Logs/core.jsonl"),
            self._ipc.path().join("s/Logs/core.jsonl"),
            self.root.join("core-hided.log"),
            self.root.join("node-hided.log"),
        ] {
            if path.exists() && String::from_utf8_lossy(&read(&path)?).contains(text) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn create_workspace_on(&self, node: &str, path: &Path) -> Result<()> {
        self.event(
            "create_workspace",
            json!({"device_id": node, "path": path, "label": "Screen fixture", "initialize_git": false}),
        )
    }
}

impl Run for Fixture {
    fn root(&self) -> &Path {
        &self.root
    }

    fn stop(&mut self) -> Result<()> {
        if let Some(mut node) = self.node.take() {
            // A journey that stopped the node (`signal_running_node`) and
            // failed before it resumed it would leave a process that cannot
            // act on its termination.
            if let Ok(pid) = i32::try_from(node.id()) {
                // SAFETY: the pid is the fixture's own unreaped child.
                unsafe { libc::kill(pid, libc::SIGCONT) };
            }
            node.kill_tree()?;
            wait_for("private node hided confirmed exit", || {
                Ok(node.try_wait()?.map(|_| ()))
            })?;
        }
        self.stop_core()?;
        self.ssh.stop()?;
        self.core.stop()?;
        self.screen.stop()?;
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
            eprintln!("private remote core fixture cleanup failed, evidence retained: {error}");
        }
    }
}

fn log_until(
    path: &Path,
    component: &str,
    kind: &str,
    ready: impl Fn(&[Value]) -> bool,
) -> Result<Vec<Value>> {
    let mut last = Vec::new();
    wait_for(&format!("{component} {kind} records"), || {
        let rows = log_rows(path, component, kind)?;
        if ready(&rows) {
            return Ok(Some(rows));
        }
        last = rows;
        Ok(None)
    })
    .with_context(|| format!("the records read: {last:?}"))
}

fn log_rows(path: &Path, component: &str, kind: &str) -> Result<Vec<Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8(read(path)?)?
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|row| row["component"] == component && row["kind"] == kind)
        .collect())
}
