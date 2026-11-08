//! The retirement of Hide's old Home folder (`hide_host::home::retire`,
//! `Call::HomeRetire`) through the helper process: what Hide's marker lists in
//! `~/hide` is removed, the operator's own entries stay, and a rerun changes
//! nothing more. Every test works in a temporary directory passed as the
//! account's home; nothing reads the real one.
//!
//! The per-entry rules (changed links, edited guides, an unreadable marker,
//! an interrupted run) are `hide_host::home`'s own tests, which run on every
//! system; this file proves the call on the wire.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use hide_host::home::{HomeRetired, MARKER_FILE};
use hide_host::protocol::{Call, Request};

/// A temporary account home holding an old `~/hide` an earlier build made.
struct Account {
    _dir: tempfile::TempDir,
    home: PathBuf,
}

impl Account {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        Self { _dir: dir, home }
    }

    fn hide(&self) -> PathBuf {
        self.home.join("hide")
    }

    /// The folder an earlier build made for one project `name`.
    fn old_home(&self, name: &str) -> PathBuf {
        let project = self.home.join("projects").join(name);
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("keep.txt"), "mine").unwrap();
        std::fs::create_dir_all(self.hide()).unwrap();
        std::os::unix::fs::symlink(&project, self.hide().join(name)).unwrap();
        std::fs::write(
            self.hide().join(MARKER_FILE),
            serde_json::json!({"version": 1, "links": {name: project}}).to_string(),
        )
        .unwrap();
        std::fs::write(self.hide().join("AGENTS.md"), "# Hide Home\n\n- a -> /a\n").unwrap();
        std::os::unix::fs::symlink("AGENTS.md", self.hide().join("CLAUDE.md")).unwrap();
        project
    }
}

#[test]
fn home_retire_round_trips_and_removes_only_what_hide_made() {
    let request = Request {
        id: 7,
        call: Call::HomeRetire { keep_empty: false },
    };
    let line = serde_json::to_string(&request).unwrap();
    assert_eq!(line, r#"{"id":7,"op":"home_retire","keep_empty":false}"#);
    assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), request);

    // The helper reads HOME itself; run it as a process so the test never
    // edits this one's environment.
    let account = Account::new();
    let project = account.old_home("alpha");
    std::fs::write(account.hide().join("notes.md"), "mine").unwrap();
    let answer = ask_helper(Some(&account.home), false);
    let retired: HomeRetired = serde_json::from_value(answer["ok"].clone()).unwrap();
    assert!(retired.marked && retired.kept, "{retired:?}");
    assert!(retired.changed.is_empty());
    let mut left: Vec<String> = std::fs::read_dir(account.hide())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, ["notes.md"]);
    assert_eq!(
        std::fs::read_to_string(project.join("keep.txt")).unwrap(),
        "mine"
    );

    // Again: nothing more to remove, the same folder state.
    let again: HomeRetired =
        serde_json::from_value(ask_helper(Some(&account.home), false)["ok"].clone()).unwrap();
    assert!(!again.marked && again.kept && again.removed.is_empty());
}

#[test]
fn an_old_home_with_nothing_else_in_it_is_removed_whole() {
    let account = Account::new();
    account.old_home("alpha");
    let answer = ask_helper(Some(&account.home), false);
    let retired: HomeRetired = serde_json::from_value(answer["ok"].clone()).unwrap();
    assert!(!retired.kept, "{retired:?}");
    assert!(!account.hide().exists());
}

#[test]
fn a_caller_that_keeps_the_folder_open_leaves_it_empty() {
    let account = Account::new();
    account.old_home("alpha");
    let answer = ask_helper(Some(&account.home), true);
    let retired: HomeRetired = serde_json::from_value(answer["ok"].clone()).unwrap();
    assert!(retired.kept);
    assert!(account.hide().is_dir());
    assert_eq!(std::fs::read_dir(account.hide()).unwrap().count(), 0);
}

#[test]
fn a_helper_without_a_usable_home_refuses() {
    let refused = ask_helper(None, false);
    assert_eq!(refused["error"]["code"], "unsupported");
    // A relative HOME would look under whatever folder the helper runs in.
    let relative = ask_helper(Some(Path::new("relative-home")), false);
    assert_eq!(relative["error"]["code"], "unsupported");
    assert!(!Path::new("relative-home").exists());
}

/// One `home_retire` request answered by `hided node serve`, whose HOME is
/// `home` or absent.
fn ask_helper(home: Option<&Path>, keep_empty: bool) -> serde_json::Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hided"));
    command
        .args(["node", "serve"])
        .env_remove("HOME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some(home) = home {
        command.env("HOME", home);
    }
    let mut child = command.spawn().unwrap();
    let request = serde_json::json!({"id": 1, "op": "home_retire", "keep_empty": keep_empty});
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{request}").unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    drop(stdin);
    child.wait().unwrap();
    serde_json::from_str(&line).unwrap()
}
