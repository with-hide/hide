//! Two machines on one host for a core move (PRD core-host-node-move): the
//! source, whose core moves, and the target, a device the source's core
//! dials over the fixture's own SSH server. Each is a private account under
//! /tmp with its own Herdr, state folder and fixture machine id, and its
//! HOME is declared a fixture HOME, so the target's steps start its core as
//! a detached process rather than through launchd.
//!
//! Every process here is owned by the fixture and ended by it, the target
//! core the move started included; nothing reaches the operator's Herdr,
//! state, SSH or login items.
#![allow(dead_code)]

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use hide_platform::process::OwnedChild;
use serde_json::{Value, json};

use super::remote_core::Herdr;
use super::remote_delivery::renderer::Renderer;
use super::remote_delivery::{Environment, READY_BOUND, read, successful, wait_for, wait_within};
use super::run::Run;
use super::sshd::Sshd;

/// The target account's `head`: the system's, except that once
/// `.cut-upload` holds a count, the counted `head -c` that brings it to zero
/// kills its process group, which is the start script's session.
const CUTTING_HEAD: &str = r#"#!/bin/sh
arm="$HOME/.cut-upload"
if [ "${1:-}" = -c ] && [ -f "$arm" ]; then
  left=$(cat "$arm")
  if [ "$left" -le 1 ]; then rm -f "$arm"; kill -KILL 0; fi
  echo $((left - 1)) > "$arm"
fi
exec /usr/bin/head "$@"
"#;

pub const SOURCE_NODE: &str = "fixture-move-source";
pub const TARGET_NODE: &str = "fixture-move-target";
/// The SSH alias the source reaches the target by.
pub const ALIAS: &str = "mini";
/// The target's registration id in the source's core: unlike its alias, as
/// a second device registered under a taken id gets, so a move back that
/// names the device by its alias instead of its id shows.
pub const DEVICE_ID: &str = "mini-2";
/// How long a resumed move's node waits for its link before it asks the
/// peer (`LINK_WITHIN` in `hided/src/supervisor.rs`).
const LINK_WITHIN: Duration = Duration::from_secs(30);

/// The stand-ins of the target's checks: each passes until a `.fail` file
/// beside it says otherwise.
const STAND_INS: [(&str, &str); 3] = [
    ("gh", r#"[ -e "$0.fail" ] && exit 1; exit 0"#),
    ("launchctl", r#"[ -e "$0.fail" ] && exit 113; exit 0"#),
    (
        "pmset",
        r#"if [ -e "$0.fail" ]; then printf 'AC Power:\n sleep 10\n'; else printf 'AC Power:\n sleep 0\n'; fi"#,
    ),
];
/// Each machine's own `gh`, first on its `PATH` and signed in, as the
/// operator's are: a repository whose `origin` names no GitHub repository
/// answers as `gh` does there, so its issues read Local by themselves; the
/// fixture has no GitHub to answer anything else.
const GH: &str = r#"[ "$1 $2" = "auth status" ] && exit 0
if [ -z "$GH_REPO" ] && ! git remote get-url origin 2>/dev/null | grep -q github.com; then
  echo "none of the git remotes configured for this repository point to a known GitHub host" >&2
  exit 1
fi
echo "the fixture's gh has no GitHub to ask" >&2
exit 1"#;
const CLAUDE: &str = r#"if [ "$1 $2" = "auth status" ]; then
  if [ -e "$0.fail" ]; then echo '{"loggedIn":false}'; else echo '{"loggedIn":true}'; fi
  exit 0
fi
exit 2"#;

fn stand_in(path: &Path, script: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, format!("#!/bin/sh\n{script}\n"))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

pub struct Machine {
    pub herdr: Herdr,
    pub state: PathBuf,
}

impl Machine {
    pub fn home(&self) -> &Path {
        &self.herdr.environment.home
    }

    pub fn project(&self) -> PathBuf {
        self.home().join("project")
    }

    /// Where this machine keeps Hide AI's settings, outside its state
    /// folder.
    pub fn ai_settings(&self) -> PathBuf {
        hide_platform::host::state_dir_under(self.home()).join("hide/ai.json")
    }

    /// The daemon of this machine's state folder, as it recorded itself.
    pub fn daemon(&self) -> Result<Option<Value>> {
        let path = self.state.join("hided.json");
        if !path.exists() {
            return Ok(None);
        }
        Ok(serde_json::from_slice(&read(&path)?).ok())
    }

    /// The `/health` of the daemon this machine's state folder records.
    pub fn health(&self) -> Result<Value> {
        let state = self.daemon()?.context("no daemon recorded")?;
        let port = state["port"].as_u64().context("daemon port")?;
        let answer = ureq::get(&format!("http://127.0.0.1:{port}/health"))
            .call()
            .context("health")?
            .into_body()
            .read_to_string()?;
        Ok(serde_json::from_str(&answer)?)
    }

    pub fn record(&self, name: &str) -> Result<Option<Value>> {
        let path = self.state.join(name);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&read(&path)?)?))
    }
}

/// How the target's state folder is spelled.
enum Reach {
    Direct,
    ThroughLink,
    LinkedFolder,
}

pub struct Fixture {
    pub source: Machine,
    pub target: Machine,
    pub ssh: Sshd,
    pub cli: PathBuf,
    pub root: PathBuf,
    removed: bool,
    daemon: Option<OwnedChild>,
    port: u16,
    token: String,
}

fn machine_env(root: &Path, bin: &Path, node: &str) -> Result<(Environment, PathBuf, PathBuf)> {
    let socket = root.join("h.sock");
    let state = root.join("s");
    hide_platform::fs::private::create_dir_all(&state)?;
    let mut environment = Environment::new(root, &socket, bin, Some(&state))?;
    fs::write(environment.home.join(hided::env::FIXTURE_HOME_MARKER), "")?;
    environment.set("HIDE_MACHINE_ID", node);
    environment.set("HIDE_KEEP_ALIVE", "1");
    environment.set("HIDE_OPEN_COMMAND", "/usr/bin/true");
    environment.set(
        "HIDE_TAILSCALE_BIN",
        environment.home.join("absent-tailscale").into_os_string(),
    );
    let mut init = environment.command("/usr/bin/git");
    init.args(["-c", "init.defaultBranch=main", "-C"])
        .arg(environment.home.join("project"))
        .args(["init", "-q"]);
    successful(init)?;
    Ok((environment, socket, state))
}

impl Fixture {
    pub fn start() -> Result<Self> {
        Self::start_with(Reach::Direct)
    }

    /// A fixture whose target names its state folder through a link, as a
    /// machine whose home is reached through one does (`/tmp` on macOS,
    /// `/home` on a Linux with its homes elsewhere).
    pub fn start_through_link() -> Result<Self> {
        Self::start_with(Reach::ThroughLink)
    }

    /// A fixture whose target's state folder is itself a link, which the
    /// moved core cannot keep private.
    pub fn start_with_linked_state_folder() -> Result<Self> {
        Self::start_with(Reach::LinkedFolder)
    }

    fn start_with(reach: Reach) -> Result<Self> {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let bin = PathBuf::from(
            std::env::var_os("HIDE_E2E_HERDR_BIN")
                .context("set HIDE_E2E_HERDR_BIN to the pinned binary")?,
        );
        let source_cli = std::env::var_os("HIDE_E2E_CLI_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| repository.join("target/debug"));
        for name in ["hide", "hided", "hide-agent-hooks"] {
            ensure!(
                source_cli.join(name).is_file(),
                "build this worktree's CLI binaries before core_move"
            );
        }
        // Under the system's temporary folder, which the fixture starter
        // requires of a HOME, and short enough for a socket path.
        let root = tempfile::Builder::new()
            .prefix("hcm-")
            .tempdir_in("/tmp")?
            .keep();
        let root = hide_platform::fs::identity::canonical(&root)?;
        // A start that fails before the fixture exists keeps its root with
        // no `finish` or drop to shed it (`support::run::shed_programs`).
        let started = (|| -> Result<Self> {
            let (mut source_env, source_socket, source_state) =
                machine_env(&root.join("m"), &bin, SOURCE_NODE)?;
            let (mut target_env, target_socket, mut target_state) =
                machine_env(&root.join("c"), &bin, TARGET_NODE)?;
            match reach {
                Reach::Direct => {}
                Reach::ThroughLink => {
                    std::os::unix::fs::symlink(root.join("c"), root.join("via"))?;
                    target_state = root.join("via/s");
                    target_env.set("HIDE_STATE_DIR", target_state.as_os_str());
                }
                Reach::LinkedFolder => {
                    std::os::unix::fs::symlink(&target_state, root.join("c/linked"))?;
                    target_state = root.join("c/linked");
                    target_env.set("HIDE_STATE_DIR", target_state.as_os_str());
                }
            }
            target_env.set("HIDE_CORE_STARTER", "fixture");
            // The target's reads find `herdr` where a real install puts it.
            fs::create_dir_all(target_env.home.join(".local/bin"))?;
            std::os::unix::fs::symlink(&bin, target_env.home.join(".local/bin/herdr"))?;
            // The start script's `head`, which a file sent to the target
            // passes through, ends the connection on the count
            // `cut_upload_at` arms ([`Fixture::cut_upload_at`]).
            let head = target_env.home.join("bin/head");
            fs::write(&head, CUTTING_HEAD)?;
            fs::set_permissions(&head, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
            // The move's checks on the target run stand-ins for the machine's
            // own `gh`, session and power settings, and the agent Hide AI asks
            // answers there as signed in; `fail_checks` turns each against the
            // move.
            let preflight = root.join("c/preflight");
            fs::create_dir(&preflight)?;
            for (name, script) in STAND_INS {
                stand_in(&preflight.join(name), script)?;
            }
            // The agent answers as signed in on both machines: the target's for
            // a move there, this machine's for a move back.
            for environment in [&source_env, &target_env] {
                fs::create_dir_all(environment.home.join(".local/bin"))?;
                stand_in(&environment.home.join(".local/bin/claude"), CLAUDE)?;
                fs::create_dir_all(environment.home.join("bin"))?;
                stand_in(&environment.home.join("bin/gh"), GH)?;
            }
            // Each runs once here, so the system's first look at a new program
            // (macOS checks each one once, machine-wide in turn) is not inside a
            // check's bound.
            for (environment, program) in [&source_env, &target_env]
                .into_iter()
                .map(|environment| (environment, environment.home.join("bin/gh")))
            {
                let mut command = environment.command(&program);
                command.args(["auth", "status"]);
                successful(command)?;
            }
            for (program, args) in [
                (preflight.join("gh"), &[][..]),
                (preflight.join("launchctl"), &[]),
                (preflight.join("pmset"), &[]),
                (
                    target_env.home.join(".local/bin/claude"),
                    &["auth", "status"],
                ),
            ] {
                let mut command = target_env.command(&program);
                command.args(args);
                successful(command)?;
            }
            let mut command = source_env.command(source_env.home.join(".local/bin/claude"));
            command.args(["auth", "status"]);
            successful(command)?;
            target_env.set("HIDE_PREFLIGHT_PROGRAMS", preflight.into_os_string());
            let settings =
                hide_platform::host::state_dir_under(&source_env.home).join("hide/ai.json");
            fs::create_dir_all(settings.parent().context("settings folder")?)?;
            fs::write(&settings, r#"{"provider":"claude"}"#)?;
            // A shipped build carries no debug data; the helper upload stays
            // inside its bound with a copy of the same kind.
            let cli = root.join("cli");
            fs::create_dir(&cli)?;
            for name in ["hide", "hided", "hide-agent-hooks"] {
                let staged = cli.join(name);
                fs::copy(source_cli.join(name), &staged)?;
                hide_platform::fs::private::restrict_to_owner(&staged)?;
                let mut strip = source_env.command("/usr/bin/strip");
                strip.arg(&staged);
                successful(strip)?;
                #[cfg(target_os = "macos")]
                {
                    let mut sign = source_env.command("/usr/bin/codesign");
                    sign.args(["--force", "--sign", "-"]).arg(&staged);
                    successful(sign)?;
                }
            }
            // The target already holds this build, as a device that connected
            // before does, so its helper is ready within `READY_BOUND`: sending
            // it took 18 to 29 s of the 30 s on GitHub's Linux runner, writing
            // and reading back each of `hided` and `hide` (about 140 MB each in a
            // debug build), against about 1 s for the rest of the connection. The
            // connection still checks the helper's bytes before it starts it, and
            // a test's other build is still sent. The device install itself is
            // `remote_delivery`'s and `device_bootstrap`'s to prove.
            hide_node::ssh::host::HelperPackages::new(Some(cli.clone()))
                .install_here(&target_env.home.join("helper"))
                .map_err(anyhow::Error::msg)?;
            let ssh_dir = source_env.home.join(".ssh");
            fs::create_dir_all(&ssh_dir)?;
            let source_herdr = Herdr::start(
                &root.join("m"),
                source_env.clone(),
                source_socket.clone(),
                &bin,
            )?;
            let target_herdr = Herdr::start(
                &root.join("c"),
                target_env.clone(),
                target_socket.clone(),
                &bin,
            )?;
            // The target machine is the system's own `sshd`, whose sessions
            // run in the target account's environment.
            let ssh = Sshd::start(&root.join("sshd"), &target_env.values())?;
            fs::write(ssh_dir.join("config"), ssh.client_config(ALIAS))?;
            source_env.set(
                "HIDE_HOST_HELPER_ROOT",
                target_env.home.join("helper").into_os_string(),
            );
            source_env.set(
                "HIDE_HOST_CLI_DIR",
                target_env.home.join("bin").into_os_string(),
            );
            let mut fixture = Self {
                source: Machine {
                    herdr: source_herdr,
                    state: source_state,
                },
                target: Machine {
                    herdr: target_herdr,
                    state: target_state,
                },
                ssh,
                cli,
                root: root.clone(),
                removed: false,
                daemon: None,
                port: 0,
                token: String::new(),
            };
            fixture.source.herdr.environment = source_env;
            fixture.start_source()?;
            fixture.event(
            "create_workspace",
            json!({"path": fixture.source.project(), "label": "Source fixture", "initialize_git": false}),
        )?;
            fixture.event(
            "register_device",
            json!({"id": DEVICE_ID, "label": "Target fixture", "ssh_alias": ALIAS, "herdr_socket_path": target_socket, "host_consent": true}),
        )?;
            fixture.device_ready()?;
            fixture.event(
            "create_workspace",
            json!({"device_id": DEVICE_ID, "path": fixture.target.project(), "label": "Target fixture", "initialize_git": false}),
        )?;
            wait_for("both machines' projects registered", || {
                let projects = fixture.projects()?;
                Ok((projects.len() == 2).then_some(()))
            })?;
            // Saved too: a journey that kills the source gives its save thread
            // no last write.
            let saved = fixture.source.state.join("core-state.json");
            wait_for("both machines' projects saved", || {
                let state: Value = serde_json::from_slice(&read(&saved)?)?;
                Ok((state
                    .pointer("/workspace_registrations")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    == Some(2))
                .then_some(()))
            })?;
            Ok(fixture)
        })();
        if let Err(error) = &started {
            if let Err(shed) = super::run::shed_programs(&root) {
                eprintln!(
                    "the kept run {} still holds programs: {shed:#}",
                    root.display()
                );
            }
            eprintln!(
                "core move fixture start failed, evidence kept at {}: {error:#}",
                root.display()
            );
        }
        started
    }

    /// Makes every stand-in check on the target fail: `gh` signed out, no
    /// desktop session, sleep on power, and Hide AI's agent signed out.
    pub fn fail_checks(&self) -> Result<()> {
        let preflight = self.root.join("c/preflight");
        for (name, _) in STAND_INS {
            fs::write(preflight.join(format!("{name}.fail")), "")?;
        }
        fs::write(self.target.home().join(".local/bin/claude.fail"), "")?;
        Ok(())
    }

    /// Signs this machine's agent out, which a move back's check of the
    /// core's Hide AI finds.
    pub fn sign_out_here(&self) -> Result<()> {
        fs::write(self.source.home().join(".local/bin/claude.fail"), "")?;
        Ok(())
    }

    /// Waits until the source's core holds a ready link to the target and
    /// reads the target's Herdr session over it, which is what a move's
    /// connection check asks for: the session rides the helper's link, so
    /// it connects only once the link is ready.
    pub fn device_ready(&self) -> Result<()> {
        let mut last_host = None;
        wait_for("the target's helper ready", || {
            let snapshot = self.snapshot()?;
            let host = snapshot
                .pointer("/navigator/devices")
                .and_then(Value::as_array)
                .and_then(|rows| rows.iter().find(|row| row["id"] == DEVICE_ID))
                .and_then(|row| row.get("host"))
                .cloned();
            last_host = host.clone();
            if let Some(host) = &host
                && matches!(
                    host["state"].as_str(),
                    Some("unavailable" | "unsupported" | "identity_changed")
                )
            {
                bail!("the target's helper failed: {}", host["message"]);
            }
            let session = snapshot
                .pointer("/status/remote")
                .and_then(Value::as_array)
                .is_some_and(|rows| {
                    rows.iter()
                        .any(|row| row["target_id"] == DEVICE_ID && row["state"] == "connected")
                });
            Ok((session && host.is_some_and(|host| host["state"] == "ready")).then_some(()))
        })
        .with_context(|| format!("last target helper state: {last_host:?}"))
    }

    /// The target's device helper: the `hided` the source runs a move's
    /// steps with there.
    pub fn helper_program(&self) -> Result<PathBuf> {
        let snapshot = self.snapshot()?;
        let program = snapshot
            .pointer("/navigator/devices")
            .and_then(Value::as_array)
            .and_then(|rows| rows.iter().find(|row| row["id"] == DEVICE_ID))
            .and_then(|row| row.pointer("/host/helper_path"))
            .and_then(Value::as_str)
            .context("the target's helper path")?;
        Ok(PathBuf::from(program))
    }

    /// Puts `script` in front of `program` on the target: it runs first,
    /// with the program itself at `$0.real`, and whatever it does not
    /// answer goes on to that program unchanged. A process already running
    /// keeps the file it started from.
    pub fn stand_in_peer(&self, program: &Path, script: &str) -> Result<()> {
        let real = PathBuf::from(format!("{}.real", program.display()));
        fs::copy(program, &real)?;
        let next = PathBuf::from(format!("{}.stand-in", program.display()));
        stand_in(&next, &format!("{script}\nexec \"$0.real\" \"$@\""))?;
        fs::rename(&next, program)?;
        Ok(())
    }

    /// Starts the source machine's hided, which runs its core.
    pub fn start_source(&mut self) -> Result<()> {
        self.start_source_with(&self.cli.clone(), &[])
    }

    /// A copy of the CLI whose `hided` is another build than the fixture's:
    /// one byte past the program's end, which its signature does not cover.
    pub fn other_build(&self, name: &str) -> Result<PathBuf> {
        let cli = self.root.join(name);
        fs::create_dir(&cli)?;
        for program in ["hide", "hided", "hide-agent-hooks"] {
            fs::copy(self.cli.join(program), cli.join(program))?;
        }
        let mut bytes = fs::read(cli.join("hided"))?;
        bytes.push(0);
        fs::write(cli.join("hided"), bytes)?;
        Ok(cli)
    }

    /// [`Fixture::start_source`] with the `hided` in `cli`, and `extra` in
    /// its environment, such as the release a fixture has it present.
    pub fn start_source_with(&mut self, cli: &Path, extra: &[(&str, &str)]) -> Result<()> {
        ensure!(self.daemon.is_none(), "the source's hided already runs");
        match fs::remove_file(self.source.state.join("hided.json")) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        let log = File::options()
            .create(true)
            .append(true)
            .open(self.root.join("source-hided.log"))?;
        let mut environment = self.source.herdr.environment.clone();
        for (key, value) in extra {
            environment.set(key, value);
        }
        let mut command = environment.command(cli.join("hided"));
        command
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        self.daemon = Some(OwnedChild::spawn(&mut command)?);
        let state: Value = wait_for("the source's hided state", || self.source.daemon())?;
        self.port = state["port"].as_u64().context("source port")? as u16;
        self.token = state["token"].as_str().context("source token")?.to_owned();
        Ok(())
    }

    /// What `hide connect --json` answers the source machine's host, as the
    /// `hide` in `cli` asks it.
    pub fn connect_with(&self, cli: &Path) -> Result<Value> {
        let mut command = self.source.herdr.environment.command(cli.join("hide"));
        command.args(["connect", "--json"]);
        let output = command.output()?;
        serde_json::from_slice(&output.stdout).with_context(|| {
            format!(
                "hide connect answered {:?} ({})",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    /// The builds in `cli` laid out as an app bundle's resources, which a
    /// `hide` takes as the app's own.
    pub fn bundle(&self, name: &str, cli: &Path) -> Result<PathBuf> {
        let resources = self
            .root
            .join(format!("{name}.app"))
            .join("Contents")
            .join("Resources");
        fs::create_dir_all(&resources)?;
        for program in ["hide", "hided", "hide-agent-hooks"] {
            fs::copy(cli.join(program), resources.join(program))?;
        }
        Ok(resources)
    }

    /// What `hide connect --json` answers on the target machine, where the
    /// moved core runs, as the `hide` in `cli` asks it with `extra` in its
    /// environment.
    pub fn connect_on_target(&self, cli: &Path, extra: &[(&str, &str)]) -> Result<Value> {
        let mut environment = self.target.herdr.environment.clone();
        for (key, value) in extra {
            environment.set(key, value);
        }
        let mut command = environment.command(cli.join("hide"));
        command.args(["connect", "--json"]);
        let output = command.output()?;
        serde_json::from_slice(&output.stdout).with_context(|| {
            format!(
                "hide connect answered {:?} ({})",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    /// The source's window address: it stays the same through a move.
    pub fn window(&self) -> (u16, String) {
        (self.port, self.token.clone())
    }

    pub fn snapshot(&self) -> Result<Value> {
        Ok(Renderer::connect(self.port, &self.token)?
            .snapshot()
            .clone())
    }

    pub fn event(&self, kind: &str, payload: Value) -> Result<()> {
        Renderer::connect(self.port, &self.token)?.event(kind, payload)
    }

    /// A window on whatever role the source's address mounts, the move's
    /// screen or a node holding its screen included.
    pub fn held_window(&self) -> Result<Renderer> {
        Renderer::connect_held(self.port, &self.token)
    }

    /// The registered projects the source's window shows, as (machine,
    /// path), in order.
    pub fn projects(&self) -> Result<Vec<(String, String)>> {
        let snapshot = self.snapshot()?;
        let mut projects: Vec<(String, String)> = snapshot
            .pointer("/ui_state/workspace_registrations")
            .and_then(Value::as_array)
            .context("registrations")?
            .iter()
            .map(|row| {
                (
                    row["device_id"].as_str().unwrap_or_default().to_owned(),
                    row["path"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        projects.sort();
        Ok(projects)
    }

    /// The source's health, as a window's host reads it.
    pub fn health(&self) -> Result<Value> {
        let answer = ureq::get(&format!("http://127.0.0.1:{}/health", self.port))
            .call()
            .context("source health")?
            .into_body()
            .read_to_string()?;
        Ok(serde_json::from_str(&answer)?)
    }

    /// Waits for the source's move journal to reach `phase`.
    pub fn journal_until(&self, phase: &str) -> Result<Value> {
        wait_for(&format!("the move journal at {phase}"), || {
            let journal = self.source.record("core-move.json")?;
            if let Some(journal) = &journal
                && journal["phase"]["phase"] == "rolled_back"
                && phase != "rolled_back"
            {
                bail!("the move rolled back: {journal}");
            }
            Ok(journal.filter(|journal| journal["phase"]["phase"] == phase))
        })
    }

    /// Waits until the forward move is done for the window too: the
    /// supervisor writes the journal's `done` before the move's view leaves
    /// `linking`, and a request in between is refused as `move_busy`. A
    /// window hears the current view when it connects.
    pub fn forward_done(&self) -> Result<Value> {
        let journal = self.journal_until("done")?;
        self.held_window()?
            .core_move_until(READY_BOUND, |view| view["state"] == "done")
            .context("the window's move view at done")?;
        Ok(journal)
    }

    /// Ends the source's hided at once, as a crash or a power cut would.
    pub fn kill_source(&mut self) -> Result<()> {
        let mut daemon = self
            .daemon
            .take()
            .context("the source's hided is not running")?;
        daemon.kill_tree()?;
        wait_for("the source's hided confirmed exit", || {
            Ok(daemon.try_wait()?.map(|_| ()))
        })
    }

    /// Asks the source's hided to stop, as `hide stop` does.
    pub fn stop_source(&self) -> Result<()> {
        let daemon = self
            .daemon
            .as_ref()
            .context("the source's hided is not running")?;
        hide_platform::process::terminate(daemon.id())?;
        Ok(())
    }

    /// Waits until the source's hided ends on its own, within `bound`, and
    /// answers how it ended.
    pub fn source_ended_within(&mut self, bound: Duration) -> Result<std::process::ExitStatus> {
        let daemon = self
            .daemon
            .as_mut()
            .context("the source's hided is not running")?;
        let status =
            super::remote_delivery::wait_within("the source's hided ended", bound, || {
                Ok(daemon.try_wait()?)
            })?;
        self.daemon = None;
        Ok(status)
    }

    /// The role the source's window reaches: `core`, `node` or `moving`.
    pub fn role(&self) -> Result<String> {
        Ok(self.health()?["role"]
            .as_str()
            .context("health role")?
            .to_owned())
    }

    /// Waits until `machine`'s core logs a record of `kind`, and answers it.
    pub fn logged(&self, machine: &Machine, kind: &str) -> Result<Value> {
        self.logged_within(machine, kind, READY_BOUND)
    }

    /// The same, for a record a resumed move writes only once its node
    /// gave up waiting for the link, which takes the supervisor's own
    /// bound before this one starts.
    pub fn logged_after_link_wait(&self, machine: &Machine, kind: &str) -> Result<Value> {
        self.logged_within(machine, kind, LINK_WITHIN + READY_BOUND)
    }

    fn logged_within(&self, machine: &Machine, kind: &str, bound: Duration) -> Result<Value> {
        let path = machine.state.join("Logs/core.jsonl");
        wait_within(&format!("a {kind} record"), bound, || {
            let Ok(bytes) = read(&path) else {
                return Ok(None);
            };
            Ok(String::from_utf8_lossy(&bytes)
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|record| record["kind"] == kind))
        })
    }

    /// The Herdr pane the source's project opened with, as the source's
    /// Herdr lists it.
    pub fn source_pane(&self) -> Result<String> {
        let project = self.source.project().display().to_string();
        wait_for("the source project's pane", || {
            let snapshot = self.source.herdr.run(&["api", "snapshot"])?;
            let mut panes = Vec::new();
            panes_under(&snapshot, &mut panes);
            Ok(panes
                .into_iter()
                .find(|pane| pane["cwd"] == project.as_str())
                .and_then(|pane| pane["pane_id"].as_str().map(str::to_owned)))
        })
    }

    /// The `herdr terminal session` children of the source's hided for the
    /// Herdr pane `pane`, as the process table lists them.
    pub fn source_sessions(&self, pane: &str) -> Result<Vec<String>> {
        let parent = self
            .daemon
            .as_ref()
            .context("the source's hided is not running")?
            .id()
            .to_string();
        let mut command = std::process::Command::new("/bin/ps");
        command.args(["-axo", "ppid=,command="]);
        let listed = String::from_utf8(successful(command)?)?;
        Ok(listed
            .lines()
            .filter_map(|line| {
                let (ppid, command) = line.trim_start().split_once(' ')?;
                (ppid == parent
                    && command.contains("terminal session")
                    && command.split_whitespace().any(|word| word == pane))
                .then(|| command.trim().to_owned())
            })
            .collect())
    }

    /// The pid of the core the move started on the target, if one runs.
    pub fn target_core(&self) -> Result<Option<u32>> {
        Ok(self.target.daemon()?.and_then(|state| {
            let pid = state["pid"].as_u64()? as u32;
            hide_platform::process::is_alive(pid).then_some(pid)
        }))
    }

    /// Ends the connection to the target as the `nth` file sent from now
    /// starts to arrive there, as a connection that dropped: the session's
    /// whole process group ends, the server stays online, and the move can
    /// dial again.
    pub fn cut_upload_at(&self, nth: usize) -> Result<()> {
        fs::write(self.target.home().join(".cut-upload"), nth.to_string())?;
        Ok(())
    }

    /// Holds the target's handover record lock, as another change of it
    /// would: every step and core that changes the record waits for it.
    pub fn hold_target_handover(&self) -> Result<hide_platform::fs::lock::Lock> {
        let file = hide_platform::fs::private::open_or_create_file(
            &self.target.state.join("core-handover.lock"),
        )?;
        match hide_platform::fs::lock::lock_file(
            file,
            hide_platform::fs::lock::Mode::Exclusive,
            std::time::Duration::from_secs(5),
            &|| false,
        )? {
            hide_platform::fs::lock::Waited::Locked(lock) => Ok(lock),
            _ => bail!("the target's handover lock stayed held"),
        }
    }

    /// Runs the target's hided as its login item would after a login or a
    /// keep-alive restart, and answers how it ended.
    pub fn start_target_hided(&self) -> Result<std::process::ExitStatus> {
        let log = File::options()
            .create(true)
            .append(true)
            .open(self.root.join("target-hided.log"))?;
        let mut command = self
            .target
            .herdr
            .environment
            .command(self.cli.join("hided"));
        command
            .arg("core-login")
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        let mut child = OwnedChild::spawn(&mut command)?;
        wait_for("the target's hided ended", || Ok(child.try_wait()?))
    }

    /// The core logged on `machine`, for a failure's message.
    pub fn log_tail(&self, machine: &Machine) -> String {
        let path = machine.state.join("Logs/core.jsonl");
        let text = read(&path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let text = text.unwrap_or_else(|error| format!("no log at {}: {error}", path.display()));
        text.lines().rev().take(40).collect::<Vec<_>>().join("\n")
    }
}

/// Every object under `value` that names a pane and its folder.
fn panes_under(value: &Value, panes: &mut Vec<Value>) {
    match value {
        Value::Object(fields) => {
            if fields.contains_key("pane_id") && fields.contains_key("cwd") {
                panes.push(value.clone());
            }
            for field in fields.values() {
                panes_under(field, panes);
            }
        }
        Value::Array(items) => {
            for item in items {
                panes_under(item, panes);
            }
        }
        _ => {}
    }
}

impl Run for Fixture {
    fn root(&self) -> &Path {
        &self.root
    }

    fn stop(&mut self) -> Result<()> {
        if let Some(mut daemon) = self.daemon.take() {
            daemon.kill_tree()?;
            wait_for("the source's hided confirmed exit", || {
                Ok(daemon.try_wait()?.map(|_| ()))
            })?;
        }
        // The target's core is the move's, started detached by its step.
        if let Some(pid) = self.target_core()? {
            hide_platform::process::kill_tree(pid)?;
            wait_for("the target's core confirmed exit", || {
                Ok((!hide_platform::process::is_alive(pid)).then_some(()))
            })?;
        }
        self.ssh.stop()?;
        self.target.herdr.stop()?;
        self.source.herdr.stop()?;
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
            eprintln!(
                "core move fixture cleanup failed, evidence kept at {}: {error}",
                self.root.display()
            );
        }
        // A start that failed keeps its run without `finish`, so its
        // programs go here (`support::run::shed_programs`).
        if let Err(error) = super::run::shed_programs(&self.root) {
            eprintln!(
                "the kept run {} still holds programs: {error:#}",
                self.root.display()
            );
        }
    }
}
