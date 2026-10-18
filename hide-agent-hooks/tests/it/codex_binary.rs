//! The real Codex reads the folder trust Hide records for a worktree, and
//! stops reading it once Hide takes it out. Codex's own decision is read
//! without signing in: `config/read` from a folder answers that folder's
//! project layer (its `.codex/config.toml`) with a `disabledReason` while
//! Codex does not trust the folder (codex-cli 0.163.0). The nightly agent
//! trust contract installs the pinned Codex and names it in
//! `HIDE_CODEX_BINARY`.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use hide_agent_hooks::folder_trust::{self, Agent, Tools};
use serde_json::{Value, json};

/// One `codex app-server` over stdio, ended when dropped.
struct AppServer {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<Value>,
    next: u64,
}

impl AppServer {
    fn start(codex: &Path, home: &Path) -> Self {
        let mut child = Command::new(codex)
            .arg("app-server")
            .current_dir(home)
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("CODEX_HOME", home.join(".codex"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("codex app-server starts");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(message) = serde_json::from_str(&line)
                    && sender.send(message).is_err()
                {
                    return;
                }
            }
        });
        let mut server = Self {
            child,
            stdin,
            lines,
            next: 0,
        };
        server.request(
            "initialize",
            json!({"clientInfo": {"name": "hide-contract", "version": "0"}}),
        );
        server.send(&json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}));
        server
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let message = self
                .lines
                .recv_timeout(Duration::from_secs(30))
                .unwrap_or_else(|_| panic!("codex answers {method}"));
            if message.get("id").and_then(Value::as_u64) == Some(id)
                && message.get("method").is_none()
            {
                assert!(message.get("error").is_none(), "{method}: {message}");
                return message["result"].clone();
            }
        }
    }

    /// Why Codex does not load `folder`'s own settings, or `None` when it
    /// trusts the folder and loads them.
    fn project_layer_disabled(&mut self, folder: &Path) -> Option<String> {
        let answer = self.request("config/read", json!({"cwd": folder, "includeLayers": true}));
        let layers = answer["layers"]
            .as_array()
            .expect("config/read lists layers");
        let project = layers
            .iter()
            .find(|layer| layer["name"]["type"] == "project")
            .unwrap_or_else(|| panic!("codex reads {}'s .codex: {answer}", folder.display()));
        project["disabledReason"].as_str().map(str::to_owned)
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(cwd)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}

/// A repository and two linked worktrees of it, each with settings of its
/// own for Codex to load or not.
fn repository(home: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let main = home.join("repo");
    std::fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "--quiet"]);
    git(
        &main,
        &[
            "commit",
            "--quiet",
            "--allow-empty",
            "--no-gpg-sign",
            "-m",
            "fixture",
        ],
    );
    let (task, other) = (home.join("repo-task"), home.join("repo-other"));
    for (tree, branch) in [(&task, "task"), (&other, "other")] {
        git(
            &main,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                branch,
                tree.to_str().unwrap(),
            ],
        );
    }
    for tree in [&main, &task, &other] {
        std::fs::create_dir_all(tree.join(".codex")).unwrap();
        std::fs::write(tree.join(".codex/config.toml"), "model = \"fixture\"\n").unwrap();
    }
    (main, task, other)
}

#[test]
#[ignore = "runs the real codex named by HIDE_CODEX_BINARY in a throwaway home; the agent trust contract installs it"]
fn the_real_codex_reads_the_worktree_trust_hide_records_until_hide_takes_it_out() {
    let codex = PathBuf::from(
        std::env::var_os("HIDE_CODEX_BINARY")
            .expect("HIDE_CODEX_BINARY names the codex this test runs"),
    );
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".codex")).unwrap();
    let (main, task, other) = repository(home.path());
    let stop = AtomicBool::new(false);
    let tools = Tools {
        codex: Some(&codex),
        stop: &stop,
    };

    let mut server = AppServer::start(&codex, home.path());
    assert!(
        server.project_layer_disabled(&task).is_some(),
        "a worktree no one trusted is not trusted yet"
    );
    drop(server);

    folder_trust::trust_worktree(Agent::Codex, home.path(), &task, tools).unwrap();
    let config = std::fs::read_to_string(home.path().join(".codex/config.toml")).unwrap();
    let mut server = AppServer::start(&codex, home.path());
    assert_eq!(
        server.project_layer_disabled(&task),
        None,
        "codex trusts the worktree Hide trusted; config.toml:\n{config}"
    );
    assert!(
        server.project_layer_disabled(&other).is_some(),
        "the repository's other worktree is not trusted with it; config.toml:\n{config}"
    );
    let main = hide_platform::fs::identity::canonical(&main).unwrap();
    assert!(
        server.project_layer_disabled(&main).is_some(),
        "the main checkout stays the operator's; config.toml:\n{config}"
    );
    drop(server);

    assert_eq!(
        folder_trust::forget(Agent::Codex, home.path(), &task, tools).unwrap(),
        1
    );
    let mut server = AppServer::start(&codex, home.path());
    assert!(server.project_layer_disabled(&task).is_some());
}
