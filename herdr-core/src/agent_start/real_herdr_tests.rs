//! The start against the pinned Herdr itself, in a tab Hide makes the way
//! the product does (`shell_launch`, then `live::create_tab_in`, which runs
//! the launch through `layout.apply`), whose shell's own startup files ask a
//! one-key question first, as oh-my-zsh's update prompt does with
//! `read -k 1`. In the incident this replays, the start typed into that
//! question: its first letter answered it and the shell ran the rest.
//!
//! Each shell is its own test and needs its binary in its variable
//! (`HIDE_TEST_<SHELL>_BIN`) and the pinned Herdr in `HIDE_E2E_HERDR_BIN`,
//! so they are ignored by default. On Windows the question is in Windows
//! PowerShell's profile under the test's own home.
//!
//! A second scenario sleeps a stand-in Codex CLI in its pane and wakes it
//! there, as `agent_sleep_herdr` does (`sleep_and_wake`).
//!
//! The server is private: its own socket, session, config and home.

#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use hide_herdr_client::LocalSocketConnector;
use hide_host::shell_integration::UserShell;
use serde_json::json;

use super::*;

const QUESTION: &str = "Update the framework now? [Y/n]";

/// B8: the seed tab must remain discoverable when shell integration is absent.
#[test]
#[ignore = "needs the pinned Herdr: HIDE_E2E_HERDR_BIN"]
fn a_new_worktrees_seed_tab_is_named_from_creation_proof_without_shell_replacement() {
    let herdr = PrivateHerdr::new(None);
    let repo = herdr.home.join("repository");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "--no-gpg-sign",
            "-m",
            "fixture",
        ],
    ] {
        assert!(
            Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let created = herdr.request(
        "worktree.create",
        json!({
            "cwd": repo, "path": herdr.home.join("task"), "branch":"task",
            "label":"hide:spawn-fixture", "focus":false, "trust_repository":true,
        }),
    );
    assert_eq!(created["workspace"]["label"], "hide:spawn-fixture");
    // The receipt uses the wire boundary's path spelling, just as the
    // executor does; Herdr's native Windows response uses backslashes.
    let placement = wire::created_worktree(created.clone()).unwrap();
    // Pinned Herdr calls the seed "1", not the workspace's marker.
    let record: crate::coordination::SpawnRecord = serde_json::from_value(json!({
        "id":"spawn-fixture", "parent":"agent-1", "intent":"seed", "name":"Seed task",
        "kind":"claude", "repo":repo, "branch":"task", "requested_path":null, "args":[],
        "path":placement.path, "pane":placement.pane_id,
        "terminal":created["root_pane"]["terminal_id"], "child":null,
        "seed_tab":{"id":placement.tab_id, "label":placement.tab_label},
    }))
    .unwrap();
    let work = crate::coordination::naming::Work::of(&record).unwrap();
    let names = crate::coordination::naming::run(&herdr.connector(), &work);
    assert_eq!(
        names.workspace,
        crate::coordination::naming::Outcome::Applied
    );
    assert_eq!(names.tab, crate::coordination::naming::Outcome::Applied);
    let observed = wire::snapshot_response(herdr.request("session.snapshot", json!({}))).unwrap();
    assert_eq!(
        observed
            .tabs
            .iter()
            .find(|tab| tab.tab_id == created["tab"]["tab_id"])
            .unwrap()
            .label,
        "Seed task"
    );
}

/// The pane's node: it writes the integration for `shell` as the product's
/// node does.
struct Node {
    home: PathBuf,
    shell: Option<String>,
}

impl NodeLink for Node {
    fn call(
        &self,
        call: Call,
        _timeout: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        let Call::ShellIntegration = call else {
            panic!("unexpected {call:?}");
        };
        let user = UserShell {
            shell: self.shell.clone(),
            ..Default::default()
        };
        let launch = hide_host::shell_integration::install(&self.home, &user).unwrap();
        Ok(crate::node_access::LinkAnswer::Parsed(
            serde_json::to_value(launch).unwrap(),
        ))
    }
}

/// A private pinned Herdr server: its own socket, session, config and home
/// (also `real_agent_trust_tests`).
pub(super) struct PrivateHerdr {
    shell: Option<String>,
    bin: PathBuf,
    pub(super) home: PathBuf,
    socket: PathBuf,
    config: PathBuf,
    log: PathBuf,
    server: Option<Child>,
    _root: tempfile::TempDir,
}

impl PrivateHerdr {
    /// A server whose own shell, on unix, is `shell`.
    pub(super) fn new(shell: Option<String>) -> Self {
        let bin = PathBuf::from(
            std::env::var_os("HIDE_E2E_HERDR_BIN")
                .expect("HIDE_E2E_HERDR_BIN names the pinned herdr"),
        );
        let root = if cfg!(unix) {
            // A Unix socket path is limited to about a hundred bytes.
            tempfile::Builder::new()
                .prefix("ha")
                .tempdir_in("/tmp")
                .unwrap()
        } else {
            tempfile::tempdir().unwrap()
        };
        let home = root.path().join("home");
        std::fs::create_dir_all(home.join("bin")).unwrap();
        let config = root.path().join("herdr-config.toml");
        std::fs::write(
            &config,
            "[update]\nversion_check = false\nmanifest_check = false\n",
        )
        .unwrap();
        let mut herdr = Self {
            shell,
            bin,
            socket: root.path().join("h.sock"),
            log: root.path().join("server.log"),
            home,
            config,
            server: None,
            _root: root,
        };
        herdr.start();
        herdr
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.bin);
        for (key, _) in std::env::vars_os() {
            let key = key.to_string_lossy().to_uppercase();
            if key.starts_with("HERDR_") || key.starts_with("HIDE_") {
                command.env_remove(key);
            }
        }
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("TERM", "xterm-256color")
            .env("XDG_CONFIG_HOME", self.home.join("xdg-config"))
            .env("XDG_STATE_HOME", self.home.join("xdg-state"))
            .env("HERDR_SESSION", "hide-argv-shell")
            .env("HERDR_SOCKET_PATH", &self.socket)
            .env("HERDR_CONFIG_PATH", &self.config)
            .env("HERDR_DISABLE_SOUND", "1");
        if cfg!(unix)
            && let Some(shell) = &self.shell
        {
            command.env("SHELL", shell);
        }
        command
    }

    fn start(&mut self) {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .unwrap();
        self.server = Some(
            self.command()
                .arg("server")
                .stdin(Stdio::null())
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .expect("herdr server starts"),
        );
        self.wait_until(
            Duration::from_secs(60),
            || {
                hide_herdr_client::request_with_timeout(
                    &self.socket,
                    "ping",
                    json!({}),
                    Duration::from_secs(2),
                )
                .is_ok()
            },
            || format!("the server answers ping; log:\n{}", self.server_log()),
        );
    }

    fn stop(&mut self) {
        let _ = self.command().args(["server", "stop"]).output();
        if let Some(mut server) = self.server.take() {
            self.wait_until(
                Duration::from_secs(30),
                || server.try_wait().is_ok_and(|status| status.is_some()),
                || "the server stops".into(),
            );
        }
    }

    fn server_log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    pub(super) fn connector(&self) -> LocalSocketConnector {
        LocalSocketConnector::new(&self.socket)
    }

    pub(super) fn request(&self, method: &str, params: Value) -> Value {
        request_with_connector(&self.connector(), method, params, READ_TIMEOUT)
            .unwrap_or_else(|error| panic!("{method}: {error}"))
    }

    pub(super) fn screen(&self, pane: &str) -> String {
        let params = wire::pane_read_params(pane, "visible", 40).unwrap();
        wire::pane_text(self.request("pane.read", params))
            .unwrap()
            .text
    }

    fn processes(&self, pane: &str) -> Value {
        self.request("pane.process_info", json!({"pane_id": pane}))
    }

    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.home.join(name))
            .unwrap_or_default()
            .replace("\r\n", "\n")
    }

    /// A new unfocused tab in `workspace`, made as the product makes an
    /// agent's tab: running `launch` when there is one, Herdr's own shell
    /// otherwise.
    fn tab(&self, workspace: &str, label: &str, launch: Option<&ShellLaunch>) -> String {
        let cwd = self.home.join(label);
        std::fs::create_dir_all(&cwd).unwrap();
        let mut env = launch.map(|launch| launch.env.clone()).unwrap_or_default();
        env.insert("PATH".into(), pane_path(self));
        let tab = crate::live::create_tab_in(
            &self.connector(),
            workspace,
            cwd.to_str().unwrap(),
            label,
            false,
            env,
            launch.map(|launch| launch.command.as_slice()),
        )
        .unwrap_or_else(|failure| panic!("{label}: {}", failure.message()));
        assert!(tab.launched);
        tab.pane_id
    }

    /// A workspace Herdr made with its own first tab, as `worktree.create`
    /// does, whose tab is then replaced by one running `launch`: the only
    /// tab left in it.
    fn replaced_first_tab(&self, label: &str, launch: &ShellLaunch) -> String {
        let cwd = self.home.join(label);
        std::fs::create_dir_all(&cwd).unwrap();
        let (workspace_id, tab_id, pane_id) = crate::wire::created_workspace(self.request(
            "workspace.create",
            json!({"cwd": cwd, "label": label, "focus": false}),
        ))
        .unwrap();
        let mut env = launch.env.clone();
        env.insert("PATH".into(), pane_path(self));
        let tab = crate::live::replace_tab(
            &self.connector(),
            crate::live::OwnedTab {
                initial_label: None,
                workspace_id: workspace_id.clone(),
                tab_id: tab_id.clone(),
                pane_id,
                launched: false,
            },
            cwd.to_str().unwrap(),
            label,
            false,
            env,
            Some(&launch.command),
        );
        assert!(tab.launched, "{label}: the first tab was replaced");
        let tabs = self.request("tab.list", json!({"workspace_id": workspace_id}));
        let tabs = tabs["tabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 1, "{label}: one tab is left: {tabs:?}");
        assert_eq!(tabs[0]["tab_id"].as_str(), Some(tab.tab_id.as_str()));
        assert_ne!(tab.tab_id, tab_id, "{label}: Herdr's own tab is gone");
        tab.pane_id
    }

    pub(super) fn wait_on(
        &self,
        pane: &str,
        within: Duration,
        what: &str,
        done: impl FnMut() -> bool,
    ) {
        self.wait_until(within, done, || {
            format!(
                "{what}; pane {pane} shows:\n{}\nprocesses: {}",
                self.screen(pane),
                self.processes(pane)
            )
        });
    }

    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn wait_until(
        &self,
        within: Duration,
        mut done: impl FnMut() -> bool,
        failure: impl FnOnce() -> String,
    ) {
        let deadline = Instant::now() + within;
        while !done() {
            if Instant::now() >= deadline {
                panic!("{}", failure());
            }
            thread::sleep(POLL_INTERVAL);
        }
    }
}

impl Drop for PrivateHerdr {
    fn drop(&mut self) {
        let _ = self.command().args(["server", "stop"]).output();
        if let Some(server) = self.server.as_mut() {
            let _ = server.kill();
            let _ = server.wait();
        }
    }
}

/// The pane's PATH: the stand-in agent first.
fn pane_path(herdr: &PrivateHerdr) -> String {
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(
        std::iter::once(herdr.home.join("bin")).chain(std::env::split_paths(&inherited)),
    )
    .unwrap()
    .into_string()
    .unwrap()
}

/// `claude` on unix: a long-lived process (`native_agent`).
#[cfg(unix)]
fn unix_agent(herdr: &PrivateHerdr) {
    native_agent(
        herdr,
        "claude",
        "#include <unistd.h>\nint main(int argc, char **argv) { (void)argv; if (argc > 1) sleep(600); return 0; }\n",
    );
}

/// The agent `name` on unix: records the folder it started in and its
/// arguments in `<name>-args`, then runs `source`, compiled, as a process of
/// that name with one argument, which Herdr reads as the agent; with none
/// it exits. Linux names a process after its file, macOS after its first
/// argument. Build a native stand-in: a system sleep may be a multicall
/// binary that refuses to run under the agent's name.
#[cfg(unix)]
fn native_agent(herdr: &PrivateHerdr, name: &str, source: &str) {
    let agent = herdr.home.join("agent");
    std::fs::create_dir_all(&agent).unwrap();
    let file = agent.join(format!("{name}.c"));
    std::fs::write(&file, source).unwrap();
    let compiled = Command::new("cc")
        .arg(&file)
        .arg("-o")
        .arg(agent.join(name))
        .output()
        .expect("the native agent fixture needs a C compiler");
    assert!(
        compiled.status.success(),
        "compile the native agent fixture: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    // Validate this file before a start's deadline; no arguments exits.
    assert!(
        Command::new(agent.join(name))
            .status()
            .expect("start the native agent fixture")
            .success()
    );
    let script = herdr.home.join("bin").join(name);
    std::fs::write(
        &script,
        format!(
            "#!/bin/bash\nprintf '%s\\n' \"${{PWD##*/}} $*\" >> \"$HOME/{name}-args\"\nexec -a {name} \"$HOME/agent/{name}\" run\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

/// Appends a line to a file in the fixture home, again while another pane
/// holds the file: Windows refuses a second writer, and two panes record at
/// once.
#[cfg(windows)]
const RECORD: &str = "function Record($file, $line) { for ($i = 0; $i -lt 50; $i++) { try { Add-Content -ErrorAction Stop -Path \"$env:USERPROFILE\\$file\" -Value $line; return } catch { Start-Sleep -Milliseconds 100 } }; throw \"$file stayed locked\" }\r\n";

/// Herdr starts Windows agents through PowerShell: `claude.ps1` is found as
/// a script, records its folder and arguments, and runs a copy of
/// `timeout.exe` named `claude.exe`, which Herdr reads as the agent.
#[cfg(windows)]
fn windows_agent(herdr: &PrivateHerdr) {
    let agent = herdr.home.join("agent");
    std::fs::create_dir_all(&agent).unwrap();
    let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    std::fs::copy(
        Path::new(&system).join(r"System32\timeout.exe"),
        agent.join("claude.exe"),
    )
    .unwrap();
    std::fs::write(
        herdr.home.join(r"bin\claude.ps1"),
        format!(
            "{RECORD}Record 'claude-args' ((Split-Path -Leaf (Get-Location).Path) + ' ' + ($args -join ' '))\r\n& \"$env:USERPROFILE\\agent\\claude.exe\" /t 600 /nobreak\r\n"
        ),
    )
    .unwrap();
}

/// What the user's own startup files hold for `shell`: the question, and
/// the answer recorded with the folder it was asked in.
#[cfg(unix)]
fn user_files(herdr: &PrivateHerdr, shell: &str) {
    let question = match shell {
        "zsh" => format!(
            "read -k 1 'answer?{QUESTION} '\nprint -r -- \"${{PWD:t}} answered [$answer]\" >> ~/answers\n"
        ),
        "bash" => format!(
            "read -n 1 -p '{QUESTION} ' answer; echo\nprintf '%s\\n' \"${{PWD##*/}} answered [$answer]\" >> ~/answers\n"
        ),
        "fish" => format!(
            "read -n 1 -P '{QUESTION} ' answer\necho (basename $PWD)\" answered [$answer]\" >> ~/answers\n"
        ),
        _ => unreachable!(),
    };
    let path = prompt_files(herdr, shell);
    let prompt = std::fs::read_to_string(&path).unwrap();
    std::fs::write(path, prompt + &question).unwrap();
}

/// The user's own startup files for `shell` with its prompt alone; returns
/// the file the prompt is in.
#[cfg(unix)]
fn prompt_files(herdr: &PrivateHerdr, shell: &str) -> PathBuf {
    let (file, text) = match shell {
        "zsh" => (".zshrc", "PS1='fixture %# '\n"),
        "bash" => (".bashrc", "PS1='fixture$ '\n"),
        // Herdr gives its panes its own XDG_CONFIG_HOME.
        "fish" => (
            "xdg-config/fish/config.fish",
            "function fish_prompt; echo -n 'fixture> '; end\n",
        ),
        _ => unreachable!(),
    };
    let path = herdr.home.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    // Ubuntu's /etc/zsh/zshrc runs compinit, which asks a question of its
    // own about folders a CI runner leaves group-writable; Ubuntu's own
    // setting skips it, so only this test's question is asked.
    std::fs::write(herdr.home.join(".zshenv"), "skip_global_compinit=1\n").unwrap();
    // A login bash reads ~/.bashrc only through the login file, as most do.
    std::fs::write(
        herdr.home.join(".bash_profile"),
        "[ -r ~/.bashrc ] && . ~/.bashrc\n",
    )
    .unwrap();
    // fish generates completions from man pages in the background, in the
    // shell's own foreground group, the first time it reads input unless
    // this folder exists: in its data folder up to fish 3, in its cache
    // folder from fish 4 (4.9.3 ran it for about 3 s after its first prompt).
    std::fs::create_dir_all(herdr.home.join(".local/share/fish/generated_completions")).unwrap();
    std::fs::create_dir_all(herdr.home.join(".cache/fish/generated_completions")).unwrap();
    path
}

/// The Windows PowerShell profile asks the question. The profile lives in
/// the Documents folder, which follows `USERPROFILE`, so with the server's
/// home it is the test's own file, not the account's. Windows names no
/// Documents folder that does not exist, and PowerShell then has no profile.
#[cfg(windows)]
fn user_files(herdr: &PrivateHerdr, _shell: &str) {
    std::fs::create_dir_all(herdr.home.join("Documents")).unwrap();
    let profile = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", "$PROFILE.CurrentUserCurrentHost"])
        .env("USERPROFILE", &herdr.home)
        .env("HOME", &herdr.home)
        .output()
        .unwrap();
    let profile = PathBuf::from(String::from_utf8(profile.stdout).unwrap().trim());
    assert!(
        profile.starts_with(&herdr.home),
        "the profile {} is the account's, not the test's",
        profile.display()
    );
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(
        &profile,
        format!(
            "{RECORD}Write-Host -NoNewline '{QUESTION} '\r\n$key = [Console]::ReadKey($true)\r\nRecord 'answers' ((Split-Path -Leaf (Get-Location).Path) + ' answered [' + $key.KeyChar + ']')\r\n"
        ),
    )
    .unwrap();
}

fn start(
    herdr: &PrivateHerdr,
    pane: &str,
    name: &str,
    shell: ShellStart,
    wait: Duration,
) -> Result<Value, StartError> {
    let params = wire::agent_start_params(
        pane,
        name,
        "claude",
        vec!["--".into(), format!("promote {name}")],
        crate::codex_launch::CodexDaemon::Unknown,
    )
    .unwrap();
    start_within(
        &herdr.connector(),
        None,
        name,
        pane,
        shell,
        params,
        Duration::from_secs(10),
        wait,
        Duration::ZERO,
        &|| Ok(()),
    )
}

/// The pane the agent Herdr knows by `name` runs in, once it has shown.
fn agent_pane(herdr: &PrivateHerdr, name: &str) -> Option<String> {
    let answer = request_with_connector(
        &herdr.connector(),
        "agent.get",
        wire::agent_target_params(name).unwrap(),
        READ_TIMEOUT,
    )
    .ok()?;
    let agent = &answer["agent"];
    // Herdr names the start's kind at once; the agent has shown only once
    // its launch is no longer pending.
    if agent["launch_pending"] == Value::Bool(true) {
        return None;
    }
    agent["agent"].as_str()?;
    agent["pane_id"].as_str().map(str::to_owned)
}

/// The scenario every shell runs. Two tabs back to back, each held by the
/// question with nothing typed; the operator's tab keeps the focus. The
/// incident, where Herdr's own shell takes a start into the question. The
/// questions answered, each start runs its own prompt under its own name.
/// After a server restart the tabs come back with their labels.
fn startup_question(shell: &str, variable: &str) {
    let mut herdr = PrivateHerdr::new(std::env::var(variable).ok());
    #[cfg(unix)]
    unix_agent(&herdr);
    #[cfg(windows)]
    windows_agent(&herdr);
    user_files(&herdr, shell);
    let node = Node {
        home: herdr.home.clone(),
        shell: std::env::var(variable).ok(),
    };
    let launch = shell_launch(Some(&node), "real-herdr").expect("the shell is integrated");
    let created = herdr.request(
        "workspace.create",
        json!({"cwd": herdr.home, "label": "agents", "focus": false}),
    );
    let workspace = created["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let operator_tab = created["tab"]["tab_id"].as_str().unwrap().to_owned();

    // m43 starts as a new worktree's agent does, in the first tab of a
    // workspace of its own; m72 as an agent tab in an existing one.
    let first = herdr.replaced_first_tab("m43", &launch);
    let second = herdr.tab(&workspace, "m72", Some(&launch));
    for pane in [&first, &second] {
        herdr.wait_on(
            pane,
            Duration::from_secs(30),
            "the question is asked",
            || herdr.screen(pane).contains(QUESTION),
        );
    }
    for (pane, name) in [(&first, "m43"), (&second, "m72")] {
        let answer = start(
            &herdr,
            pane,
            name,
            ShellStart::of_tab(true, Some(&launch)),
            Duration::from_secs(2),
        );
        let Err(StartError::ShellStarting(line)) = answer else {
            panic!(
                "{name}: a shell held by its question is not a start, but {answer:?}; pane shows:\n{}\nprocesses: {}",
                herdr.screen(pane),
                herdr.processes(pane)
            );
        };
        assert!(
            line.as_deref().is_some_and(|line| line.contains(QUESTION)),
            "{line:?}"
        );
    }
    assert_eq!(herdr.file("answers"), "", "no start answered the question");
    assert_eq!(herdr.file("claude-args"), "", "nothing was typed");
    let workspace_now = herdr.request("workspace.get", json!({"workspace_id": workspace}));
    assert_eq!(
        workspace_now["workspace"]["active_tab_id"].as_str(),
        Some(operator_tab.as_str()),
        "an unfocused tab leaves the operator's tab active"
    );

    // The incident: a start that only waits for the shell to hold the
    // terminal types into the question, whose one key is the command's first
    // letter, and the shell runs the rest. Herdr's own shell is the server's
    // `SHELL`, the test's shell, on unix.
    #[cfg(unix)]
    {
        let plain = herdr.tab(&workspace, "plain", None);
        herdr.wait_on(
            &plain,
            Duration::from_secs(30),
            "the question is asked",
            || herdr.screen(&plain).contains(QUESTION),
        );
        start(
            &herdr,
            &plain,
            "plain",
            ShellStart::Plain,
            Duration::from_secs(2),
        )
        .expect("Herdr types the start");
        herdr.wait_on(
            &plain,
            Duration::from_secs(30),
            "the question is answered",
            || herdr.file("answers").contains("plain answered [c]"),
        );
        assert_eq!(herdr.file("claude-args"), "", "no agent ran");
    }

    // The operator answers each question; the same starts then launch, each
    // with its own prompt, under its own name, in its own tab.
    let answered = || {
        herdr
            .file("answers")
            .lines()
            .filter(|line| !line.starts_with("plain"))
            .count()
    };
    for (count, pane) in [&first, &second].into_iter().enumerate() {
        herdr.request(
            "pane.send_text",
            wire::pane_send_text_params(pane, "n").unwrap(),
        );
        herdr.wait_on(
            pane,
            Duration::from_secs(30),
            "the question is answered",
            || answered() == count + 1,
        );
    }
    // Windows PowerShell keeps Herdr's own prompt wrapper: the pane's folder
    // follows a `cd`, as in Herdr's own PowerShell panes.
    #[cfg(windows)]
    {
        let sub = herdr.home.join("m43").join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        herdr.request(
            "pane.send_text",
            wire::pane_send_text_params(&first, "cd sub\r").unwrap(),
        );
        herdr.wait_on(
            &first,
            Duration::from_secs(30),
            "the pane's folder follows",
            || {
                let pane = herdr.request("pane.get", json!({"pane_id": first}));
                pane["pane"]["cwd"]
                    .as_str()
                    .is_some_and(|cwd| cwd.trim_end_matches('\\').ends_with("sub"))
            },
        );
    }
    for (pane, name) in [(&first, "m43"), (&second, "m72")] {
        start(
            &herdr,
            pane,
            name,
            ShellStart::of_tab(true, Some(&launch)),
            Duration::from_secs(30),
        )
        .unwrap_or_else(|error| panic!("{name}: {error:?}"));
    }
    for (pane, name) in [(&first, "m43"), (&second, "m72")] {
        herdr.wait_on(pane, Duration::from_secs(30), "the agent shows", || {
            agent_pane(&herdr, name).as_deref() == Some(pane.as_str())
        });
    }
    let mut answers: Vec<_> = herdr
        .file("answers")
        .lines()
        .filter(|line| !line.starts_with("plain"))
        .map(str::to_owned)
        .collect();
    answers.sort();
    assert_eq!(answers, ["m43 answered [n]", "m72 answered [n]"]);
    let mut args: Vec<_> = herdr
        .file("claude-args")
        .lines()
        .map(|line| line.replace("-- ", ""))
        .collect();
    args.sort();
    let first_folder = if cfg!(windows) { "sub" } else { "m43" };
    let mut expected = [
        format!("{first_folder} promote m43"),
        "m72 promote m72".to_owned(),
    ];
    expected.sort();
    assert_eq!(args, expected);

    // A server restart brings the tabs back with their labels, running
    // Herdr's own shell; the launch is not run again.
    herdr.stop();
    herdr.start();
    let tabs = herdr.request("tab.list", json!({}));
    let labels: Vec<_> = tabs["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tab| tab["label"].as_str())
        .collect();
    assert!(labels.contains(&"m43") && labels.contains(&"m72"), "{tabs}");
}

#[cfg(unix)]
#[test]
#[ignore = "needs the pinned Herdr and zsh: HIDE_E2E_HERDR_BIN, HIDE_TEST_ZSH_BIN"]
fn a_zsh_startup_question_is_never_answered_by_a_start_and_each_tab_runs_its_own_prompt() {
    startup_question("zsh", "HIDE_TEST_ZSH_BIN");
}

#[cfg(unix)]
#[test]
#[ignore = "needs the pinned Herdr and bash: HIDE_E2E_HERDR_BIN, HIDE_TEST_BASH_BIN"]
fn a_bash_startup_question_is_never_answered_by_a_start_and_each_tab_runs_its_own_prompt() {
    startup_question("bash", "HIDE_TEST_BASH_BIN");
}

#[cfg(unix)]
#[test]
#[ignore = "needs the pinned Herdr and fish: HIDE_E2E_HERDR_BIN, HIDE_TEST_FISH_BIN"]
fn a_fish_startup_question_is_never_answered_by_a_start_and_each_tab_runs_its_own_prompt() {
    startup_question("fish", "HIDE_TEST_FISH_BIN");
}

#[cfg(windows)]
#[test]
#[ignore = "needs the pinned Herdr: HIDE_E2E_HERDR_BIN"]
fn a_windows_powershell_startup_question_is_never_answered_by_a_start_and_each_tab_runs_its_own_prompt()
 {
    startup_question("powershell", "HIDE_TEST_POWERSHELL_BIN");
}

/// Codex CLI 0.165.0 as a sleep meets it: it turns the kitty keyboard
/// protocol on, quits on the first Ctrl+C into its empty composer and reads
/// no input while it shuts down. It reads one byte at a time up to that
/// Ctrl+C, so a second press queued with it stays unread. Then it hands the
/// terminal back with the protocol off and the line settings it found put
/// back without discarding what it never read, so a key sent meanwhile is
/// left for the shell, encoded in the protocol. It hands back only once
/// the test opens the `codex-release` FIFO in its home (`Release`); the
/// alarm is its hang guard.
#[cfg(unix)]
const CODEX: &str = r#"#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <termios.h>
#include <unistd.h>

static void say(const char *text) { (void)!write(1, text, strlen(text)); }

int main(int argc, char **argv) {
    (void)argv;
    if (argc < 2) return 0;
    char release[4096];
    snprintf(release, sizeof release, "%s/codex-release", getenv("HOME"));
    struct termios found, raw;
    tcgetattr(0, &found);
    raw = found;
    cfmakeraw(&raw);
    tcsetattr(0, TCSANOW, &raw);
    say("\033[>7u\xe2\x80\xba \033[2mAsk Codex to do anything\033[0m\r\n");
    const char press[] = "\033[99;5u";
    size_t matched = 0;
    for (;;) {
        char byte;
        if (read(0, &byte, 1) != 1) return 1;
        if (byte == 3) break;
        matched = byte == press[matched] ? matched + 1 : byte == press[0];
        if (matched == sizeof press - 1) break;
    }
    say("Shutting down...\r\n");
    alarm(60);
    char byte;
    int gate = open(release, O_RDONLY);
    if (gate < 0 || read(gate, &byte, 1) != 1) return 1;
    say("\033[<u");
    tcsetattr(0, TCSANOW, &found);
    return 0;
}
"#;

/// What the stand-in Codex CLI shows while it shuts down.
#[cfg(unix)]
const QUITTING: &str = "Shutting down...";

/// Lets the stand-in Codex CLI hand its terminal back at what the sleep has
/// observed, never at a time: a screen read after the first press that
/// shows it quitting, which the sleep acts on, or a second press answered,
/// whose key Herdr has then encoded and queued for the terminal the
/// stand-in no longer reads.
#[cfg(unix)]
struct Release {
    fifo: PathBuf,
    presses: std::sync::atomic::AtomicUsize,
    released: std::sync::atomic::AtomicBool,
}

#[cfg(unix)]
impl Release {
    /// Herdr answered `method` with `answer`; called before the sleep reads
    /// the answer.
    fn answered(&self, method: &str, answer: &str) {
        use std::sync::atomic::Ordering;
        let observed = match method {
            "pane.send_keys" => self.presses.fetch_add(1, Ordering::SeqCst) == 1,
            "pane.read" => self.presses.load(Ordering::SeqCst) >= 1 && answer.contains(QUITTING),
            _ => false,
        };
        if observed && !self.released.swap(true, Ordering::SeqCst) {
            self.open();
        }
    }

    /// Opens the FIFO for writing, which ends the stand-in's wait, once it
    /// is waiting there; it is by the time either boundary is reached, and
    /// the deadline is the hang guard.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn open(&self) {
        use std::os::unix::fs::OpenOptionsExt;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&self.fifo)
            {
                Ok(mut fifo) => {
                    std::io::Write::write_all(&mut fifo, b"r").unwrap();
                    return;
                }
                // No reader yet.
                Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {}
                Err(error) => panic!("open the stand-in's release: {error}"),
            }
            assert!(
                Instant::now() < deadline,
                "the stand-in never waited for its release"
            );
            thread::sleep(POLL_INTERVAL);
        }
    }
}

/// Herdr's socket, with each answer shown to `Release` before the caller
/// reads it.
#[cfg(unix)]
struct Watched {
    socket: LocalSocketConnector,
    release: std::sync::Arc<Release>,
}

#[cfg(unix)]
impl ApiConnector for Watched {
    fn connect(&self) -> Result<Box<dyn hide_herdr_client::ApiStream>, ApiError> {
        Ok(Box::new(WatchedStream {
            stream: self.socket.connect()?,
            release: std::sync::Arc::clone(&self.release),
            request: Vec::new(),
            answer: Vec::new(),
            told: false,
        }))
    }
}

/// One request's stream: Herdr answers one line and closes it.
#[cfg(unix)]
struct WatchedStream {
    stream: Box<dyn hide_herdr_client::ApiStream>,
    release: std::sync::Arc<Release>,
    request: Vec<u8>,
    answer: Vec<u8>,
    told: bool,
}

#[cfg(unix)]
impl WatchedStream {
    fn tell(&mut self) {
        if self.told {
            return;
        }
        self.told = true;
        let request: Value = serde_json::from_slice(&self.request).unwrap_or_default();
        self.release.answered(
            request["method"].as_str().unwrap_or_default(),
            &String::from_utf8_lossy(&self.answer),
        );
    }
}

#[cfg(unix)]
impl std::io::Write for WatchedStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = self.stream.write(bytes)?;
        self.request.extend_from_slice(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

#[cfg(unix)]
impl std::io::Read for WatchedStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.stream.read(buffer)?;
        self.answer.extend_from_slice(&buffer[..read]);
        if self.answer.contains(&b'\n') || read == 0 {
            self.tell();
        }
        Ok(read)
    }
}

#[cfg(unix)]
impl hide_herdr_client::ApiStream for WatchedStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), ApiError> {
        self.stream.set_read_timeout(timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> Result<(), ApiError> {
        self.stream.set_write_timeout(timeout)
    }

    fn read_line_with_timeout(&mut self, timeout: Duration) -> Result<String, ApiError> {
        let line = self.stream.read_line_with_timeout(timeout)?;
        self.answer.extend_from_slice(line.as_bytes());
        self.tell();
        Ok(line)
    }

    fn shutdown_handle(&self) -> Result<Box<dyn hide_herdr_client::ConnectionShutdown>, ApiError> {
        self.stream.shutdown_handle()
    }
}

/// The scenario every Unix shell runs: Hide sleeps an idle Codex CLI in its
/// pane (`agent_sleep_herdr::end_agent`) and wakes it there, and the resume
/// line runs with its own arguments and nothing before them. The sleep's
/// second Ctrl+C, sent at a fixed 150 ms, reached the shell as `ESC[99;5u`,
/// and the wake ran `9;5ucodex resume <id>` (with-hide/hide-private#37).
#[cfg(unix)]
fn sleep_and_wake(shell: &str, variable: &str) {
    let herdr = PrivateHerdr::new(std::env::var(variable).ok());
    native_agent(&herdr, "codex", CODEX);
    prompt_files(&herdr, shell);
    let fifo = herdr.home.join("codex-release");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo runs")
            .success()
    );
    let created = herdr.request(
        "workspace.create",
        json!({"cwd": herdr.home, "label": "agents", "focus": false}),
    );
    let workspace = created["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let pane = herdr.tab(&workspace, "slept", None);
    start_within(
        &herdr.connector(),
        None,
        "slept",
        &pane,
        ShellStart::Plain,
        wire::agent_start_params(
            &pane,
            "slept",
            "codex",
            Vec::new(),
            crate::codex_launch::CodexDaemon::Unsupported,
        )
        .unwrap(),
        Duration::from_secs(10),
        Duration::from_secs(30),
        Duration::ZERO,
        &|| Ok(()),
    )
    .unwrap_or_else(|error| panic!("the agent starts: {error:?}"));
    herdr.wait_on(
        &pane,
        Duration::from_secs(30),
        "Herdr reads the agent idle",
        || {
            request_with_connector(
                &herdr.connector(),
                "agent.get",
                wire::agent_target_params(&pane).unwrap(),
                READ_TIMEOUT,
            )
            .ok()
            .and_then(|answer| wire::agent_state(answer).ok())
            .is_some_and(|state| state.status == "idle")
        },
    );
    // The node ends an agent only when its own quit failed, which fails
    // here (`Node` answers nothing else).
    let node = Node {
        home: herdr.home.clone(),
        shell: None,
    };
    let release = std::sync::Arc::new(Release {
        fifo,
        presses: Default::default(),
        released: Default::default(),
    });
    let watched = Watched {
        socket: herdr.connector(),
        release: std::sync::Arc::clone(&release),
    };
    assert!(
        crate::agent_sleep_herdr::end_agent(&watched, &node, &pane, "codex").is_ok(),
        "the agent quit; pane shows:\n{}",
        herdr.screen(&pane)
    );
    assert!(
        release.released.load(std::sync::atomic::Ordering::SeqCst),
        "the stand-in quit only on its release"
    );
    let woke = crate::agent_sleep_herdr::start_agent(
        &herdr.connector(),
        &node,
        &crate::agent_sleep_herdr::WakeRequest {
            pane_id: pane.clone(),
            kind: "codex".into(),
            name: "slept".into(),
            mode: crate::agent_sleep::WakeMode::Resume,
            args: vec!["resume".into(), "slept-session".into()],
            cwd: None,
            session_id: Some("slept-session".into()),
            codex_daemon: crate::codex_launch::CodexDaemon::Unsupported,
        },
    );
    assert_eq!(woke, crate::agent_sleep_herdr::WakeOutcome::Started);
    herdr.wait_on(
        &pane,
        Duration::from_secs(30),
        "the resume line runs",
        || herdr.file("codex-args").lines().count() == 2,
    );
    assert_eq!(
        herdr.file("codex-args"),
        "slept \nslept resume slept-session\n"
    );
}

#[cfg(unix)]
#[test]
#[ignore = "needs the pinned Herdr and zsh: HIDE_E2E_HERDR_BIN, HIDE_TEST_ZSH_BIN"]
fn a_codex_slept_and_woken_in_zsh_runs_its_resume_line_with_nothing_before_it() {
    sleep_and_wake("zsh", "HIDE_TEST_ZSH_BIN");
}

#[cfg(unix)]
#[test]
#[ignore = "needs the pinned Herdr and bash: HIDE_E2E_HERDR_BIN, HIDE_TEST_BASH_BIN"]
fn a_codex_slept_and_woken_in_bash_runs_its_resume_line_with_nothing_before_it() {
    sleep_and_wake("bash", "HIDE_TEST_BASH_BIN");
}

#[cfg(unix)]
#[test]
#[ignore = "needs the pinned Herdr and fish: HIDE_E2E_HERDR_BIN, HIDE_TEST_FISH_BIN"]
fn a_codex_slept_and_woken_in_fish_runs_its_resume_line_with_nothing_before_it() {
    sleep_and_wake("fish", "HIDE_TEST_FISH_BIN");
}
