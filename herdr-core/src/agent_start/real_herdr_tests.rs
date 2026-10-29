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
/// The start of the prompt the user's files give a Unix shell.
#[cfg(unix)]
const PROMPT: &str = "fixture";

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

/// `claude` on unix: records the folder it started in and its arguments,
/// then runs as a long-lived process of that name, which Herdr reads as the
/// agent. Linux names a process after its file, macOS after its first
/// argument. Build a native stand-in: a system sleep may be a multicall
/// binary that refuses to run under the agent's name.
#[cfg(unix)]
fn unix_agent(herdr: &PrivateHerdr) {
    let agent = herdr.home.join("agent");
    std::fs::create_dir_all(&agent).unwrap();
    let source = agent.join("agent.c");
    std::fs::write(
        &source,
        "#include <unistd.h>\nint main(int argc, char **argv) { (void)argv; if (argc > 1) sleep(600); return 0; }\n",
    )
    .unwrap();
    let compiled = Command::new("cc")
        .arg(&source)
        .arg("-o")
        .arg(agent.join("claude"))
        .output()
        .expect("the native agent fixture needs a C compiler");
    assert!(
        compiled.status.success(),
        "compile the native agent fixture: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    // Validate this file before a start's deadline; no arguments exits.
    assert!(
        Command::new(agent.join("claude"))
            .status()
            .expect("start the native agent fixture")
            .success()
    );
    let script = herdr.home.join("bin/claude");
    std::fs::write(
        &script,
        "#!/bin/bash\nprintf '%s\\n' \"${PWD##*/} $*\" >> \"$HOME/claude-args\"\nexec -a claude \"$HOME/agent/claude\" 600\n",
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

/// What the user's own startup files hold for `shell`: the prompt, then,
/// when `ask`, the question and the answer recorded with the folder it was
/// asked in.
#[cfg(unix)]
fn user_files(herdr: &PrivateHerdr, shell: &str, ask: bool) {
    let (file, prompt, question) = match shell {
        "zsh" => (
            ".zshrc",
            format!("PS1='{PROMPT} %# '\n"),
            format!(
                "read -k 1 'answer?{QUESTION} '\nprint -r -- \"${{PWD:t}} answered [$answer]\" >> ~/answers\n"
            ),
        ),
        "bash" => (
            ".bashrc",
            format!("PS1='{PROMPT}$ '\n"),
            format!(
                "read -n 1 -p '{QUESTION} ' answer; echo\nprintf '%s\\n' \"${{PWD##*/}} answered [$answer]\" >> ~/answers\n"
            ),
        ),
        "fish" => (
            // Herdr gives its panes its own XDG_CONFIG_HOME.
            "xdg-config/fish/config.fish",
            format!("function fish_prompt; echo -n '{PROMPT}> '; end\n"),
            format!(
                "read -n 1 -P '{QUESTION} ' answer\necho (basename $PWD)\" answered [$answer]\" >> ~/answers\n"
            ),
        ),
        _ => unreachable!(),
    };
    let text = if ask { prompt + &question } else { prompt };
    let path = herdr.home.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
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
    // this folder exists.
    std::fs::create_dir_all(herdr.home.join(".local/share/fish/generated_completions")).unwrap();
}

/// The Windows PowerShell profile asks the question when `ask`; with none,
/// PowerShell's own prompt shows. The profile lives in the Documents folder,
/// which follows `USERPROFILE`, so with the server's home it is the test's
/// own file, not the account's. Windows names no Documents folder that does
/// not exist, and PowerShell then has no profile.
#[cfg(windows)]
fn user_files(herdr: &PrivateHerdr, _shell: &str, ask: bool) {
    if !ask {
        return;
    }
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
        PaneLine::Fresh,
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
    user_files(&herdr, shell, true);
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

/// What zsh showed in a sleeping Codex's shell line (with-hide/hide-private#37):
/// the tail of the sleep's second Ctrl+C, which Codex had quit before reading
/// and which Herdr had encoded in the keyboard protocol Codex turned on
/// (`ESC[99;5u`). Typed behind it, the wake ran `9;5ucodex ... resume <id>`.
const STRAY: &str = "9;5u";

/// The scenario every shell runs: text an ended agent left in its shell's
/// line is there when the wake types into that pane, and the resume line
/// still runs, with its own arguments, as the agent the wake names.
fn wake_after_stray_text(shell: &str, variable: &str) {
    let herdr = PrivateHerdr::new(std::env::var(variable).ok());
    #[cfg(unix)]
    unix_agent(&herdr);
    #[cfg(windows)]
    windows_agent(&herdr);
    user_files(&herdr, shell, false);
    let created = herdr.request(
        "workspace.create",
        json!({"cwd": herdr.home, "label": "agents", "focus": false}),
    );
    let workspace = created["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Herdr's own shell, the server's `SHELL` on unix, as an agent's pane is
    // once the agent has ended.
    let pane = herdr.tab(&workspace, "woken", None);
    let prompt = if cfg!(windows) { "PS " } else { PROMPT };
    herdr.wait_on(&pane, Duration::from_secs(30), "the prompt shows", || {
        herdr.screen(&pane).contains(prompt)
    });
    herdr.request(
        "pane.send_text",
        wire::pane_send_text_params(&pane, STRAY).unwrap(),
    );
    herdr.wait_on(
        &pane,
        Duration::from_secs(30),
        "the shell's line holds the stray text",
        || herdr.screen(&pane).contains(STRAY),
    );

    let woke = crate::agent_sleep_herdr::start_agent(
        &herdr.connector(),
        &Node {
            home: herdr.home.clone(),
            shell: None,
        },
        &crate::agent_sleep_herdr::WakeRequest {
            pane_id: pane.clone(),
            kind: "claude".into(),
            name: "woken".into(),
            mode: crate::agent_sleep::WakeMode::Resume,
            args: vec!["--resume".into(), "slept-session".into()],
            cwd: None,
            session_id: Some("slept-session".into()),
            codex_daemon: crate::codex_launch::CodexDaemon::Unknown,
        },
    );
    assert_eq!(woke, crate::agent_sleep_herdr::WakeOutcome::Started);
    herdr.wait_on(
        &pane,
        Duration::from_secs(30),
        "the woken agent shows",
        || agent_pane(&herdr, "woken").as_deref() == Some(pane.as_str()),
    );
    assert_eq!(herdr.file("claude-args"), "woken --resume slept-session\n");
}

#[cfg(unix)]
#[test]
#[ignore = "needs the pinned Herdr and zsh: HIDE_E2E_HERDR_BIN, HIDE_TEST_ZSH_BIN"]
fn a_zsh_wake_runs_its_resume_line_past_text_the_ended_agent_left_in_the_shell() {
    wake_after_stray_text("zsh", "HIDE_TEST_ZSH_BIN");
}

#[cfg(unix)]
#[test]
#[ignore = "needs the pinned Herdr and bash: HIDE_E2E_HERDR_BIN, HIDE_TEST_BASH_BIN"]
fn a_bash_wake_runs_its_resume_line_past_text_the_ended_agent_left_in_the_shell() {
    wake_after_stray_text("bash", "HIDE_TEST_BASH_BIN");
}

#[cfg(unix)]
#[test]
#[ignore = "needs the pinned Herdr and fish: HIDE_E2E_HERDR_BIN, HIDE_TEST_FISH_BIN"]
fn a_fish_wake_runs_its_resume_line_past_text_the_ended_agent_left_in_the_shell() {
    wake_after_stray_text("fish", "HIDE_TEST_FISH_BIN");
}

#[cfg(windows)]
#[test]
#[ignore = "needs the pinned Herdr: HIDE_E2E_HERDR_BIN"]
fn a_windows_powershell_wake_runs_its_resume_line_past_text_the_ended_agent_left_in_the_shell() {
    wake_after_stray_text("powershell", "HIDE_TEST_POWERSHELL_BIN");
}
