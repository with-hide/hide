//! The real Claude Code in a pane of an engine, in a worktree of a
//! repository it was never told to trust: it starts at its prompt in the
//! worktree whose trust Hide recorded through the node (`trust_worktree`),
//! and asks its folder trust question in the repository's other worktree,
//! which shows the check reads the question where Claude asks it. Claude Code
//! shows the question before it needs a sign-in, so no account is needed.
//!
//! It runs once on each engine: the pinned Herdr in a private server
//! (`HIDE_E2E_HERDR_BIN`), and the core's own engine over holders started
//! from the built holder (`HIDE_TEST_HOLDER_BIN`, and `HIDE_HOLDER_CONPTY` on
//! Windows). Both need Claude Code in `HIDE_TEST_CLAUDE_BIN`, so they are
//! ignored by default; the nightly agent trust contract installs the pinned
//! Claude Code and runs both on every system.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use super::real_herdr_tests::PrivateHerdr;
use super::*;
use crate::engine::{Engine, Wait};

/// Claude Code 2.1.302's folder trust question.
const TRUST_QUESTION: &str = "Quick safety check";
/// The banner over Claude Code's prompt, which its trust question replaces.
const PROMPT_BANNER: &str = "Claude Code v";
const WAIT: Wait = Wait::Each(Duration::from_secs(10));

fn git(cwd: &Path, args: &[&str]) {
    // The account's git settings, such as commit signing, are not the
    // fixture's.
    let output = Command::new("git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
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
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A tab running `claude` in `folder`, in a workspace of its own; `env`
/// reaches the pane's program beside what the engine gives it.
fn claude_in(
    engine: &dyn Engine,
    env: &BTreeMap<String, String>,
    claude: &Path,
    folder: &Path,
    label: &str,
) -> String {
    let (workspace, _, _) = engine
        .create_workspace(WAIT, folder.to_str().unwrap(), label, false, env.clone())
        .unwrap_or_else(|error| panic!("{label}: the workspace is made: {error:?}"));
    let mut env = env.clone();
    env.insert("DISABLE_AUTOUPDATER".into(), "1".into());
    env.insert(
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
        "1".into(),
    );
    let command = [claude.to_str().unwrap().to_owned()];
    let tab = crate::checkout_owner::open::create_tab_in(
        engine,
        &workspace,
        folder.to_str().unwrap(),
        label,
        false,
        env,
        Some(command.as_slice()),
    )
    .unwrap_or_else(|failure| panic!("{label}: {}", failure.message()));
    tab.pane_id
}

/// What `pane` shows now, without the styling between banner words.
fn screen(engine: &dyn Engine, pane: &str) -> String {
    engine
        .read_pane(WAIT, pane, "visible", 120)
        .map(|read| read.text)
        .unwrap_or_else(|error| format!("<unread: {error:?}>"))
}

/// Waits until `pane` shows `text`, failing with what it shows.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait_for(engine: &dyn Engine, pane: &str, text: &str, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    while !screen(engine, pane).contains(text) {
        assert!(
            Instant::now() < deadline,
            "{what}; pane {pane} shows:\n{}",
            screen(engine, pane)
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The check on `engine`, whose panes receive only private config roots:
/// Claude Code asks in the worktree nobody trusted and starts
/// at its prompt in the one Hide trusted.
fn claude_reads_the_trust_hide_recorded(engine: &dyn Engine, home: &Path) {
    let claude = PathBuf::from(
        std::env::var_os("HIDE_TEST_CLAUDE_BIN").expect("HIDE_TEST_CLAUDE_BIN names Claude Code"),
    );
    // Move every native config lookup with HOME, including an inherited
    // Claude override and its keychain namespace. The same private .claude
    // folder holds the legacy global config both Claude and trust_worktree
    // read (hide-agent-hooks Runtime::trust_path).
    let config = home.join(".claude");
    std::fs::create_dir_all(&config).unwrap();
    let env = [
        ("HOME", home.to_owned()),
        ("USERPROFILE", home.to_owned()),
        ("CLAUDE_CONFIG_DIR", config.clone()),
        ("CLAUDE_SECURESTORAGE_CONFIG_DIR", config.clone()),
        ("APPDATA", home.join("AppData/Roaming")),
        ("LOCALAPPDATA", home.join("AppData/Local")),
        ("XDG_CONFIG_HOME", home.join(".config")),
        ("XDG_STATE_HOME", home.join(".local/state")),
        ("XDG_DATA_HOME", home.join(".local/share")),
        ("XDG_CACHE_HOME", home.join(".cache")),
    ]
    .into_iter()
    .map(|(key, folder)| (key.to_owned(), folder.to_string_lossy().into_owned()))
    .collect();
    // An account where Claude Code has run before and trusts no folder.
    std::fs::write(
        config.join(".config.json"),
        r#"{"hasCompletedOnboarding": true, "hasSeenAutoDefaultNotice": true, "theme": "dark", "numStartups": 3}"#,
    )
    .unwrap();
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

    let node = hide_node::Local::new(Some(home.to_owned()));
    trust_worktree(
        Some(&node),
        "claude",
        &hide_platform::path::to_wire(&task).unwrap(),
    )
    .unwrap();

    // The node must have written the file the real CLI will read, not the
    // default global file outside CLAUDE_CONFIG_DIR.
    let trusted_config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(config.join(".config.json")).unwrap()).unwrap();
    let projects = trusted_config["projects"].as_object().unwrap();
    assert_eq!(projects.len(), 1);
    let (folder, trust) = projects.iter().next().unwrap();
    assert_eq!(
        std::fs::canonicalize(folder).unwrap(),
        std::fs::canonicalize(&task).unwrap()
    );
    assert_eq!(trust["hasTrustDialogAccepted"], true);
    assert!(!home.join(".claude.json").exists());

    let asked = claude_in(engine, &env, &claude, &other, "other");
    wait_for(
        engine,
        &asked,
        TRUST_QUESTION,
        "Claude Code asks its folder trust question in a worktree no one trusted",
    );
    let trusted = claude_in(engine, &env, &claude, &task, "task");
    wait_for(
        engine,
        &trusted,
        PROMPT_BANNER,
        "Claude Code starts at its prompt in the worktree Hide trusted",
    );
    let screen = screen(engine, &trusted);
    assert!(
        !screen.contains(TRUST_QUESTION),
        "Claude Code asked in the worktree Hide trusted:\n{screen}"
    );
}

#[test]
#[ignore = "needs the pinned Herdr (HIDE_E2E_HERDR_BIN) and Claude Code (HIDE_TEST_CLAUDE_BIN)"]
fn the_real_claude_starts_at_its_prompt_in_a_worktree_hide_trusted_and_asks_in_another() {
    // The server runs in its private home, which its panes inherit.
    let herdr = PrivateHerdr::new(None);
    let engine = herdr.engine();
    claude_reads_the_trust_hide_recorded(engine.as_ref(), &herdr.home);
}

#[test]
#[ignore = "needs the built pane holder (HIDE_TEST_HOLDER_BIN) and Claude Code (HIDE_TEST_CLAUDE_BIN)"]
fn on_the_own_engine_the_real_claude_starts_at_its_prompt_in_a_worktree_hide_trusted_and_asks_in_another()
 {
    struct Running(crate::engine::contract::BuiltHolders);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.stop();
        }
    }
    let running = Running(crate::engine::contract::BuiltHolders::start());
    // A holder's pane runs with the test's environment, so the private home
    // is named to the pane's program as Herdr's server names its own.
    let home = Path::new(&running.0.folder).join("home");
    std::fs::create_dir_all(&home).unwrap();
    claude_reads_the_trust_hide_recorded(&running.0.engine, &home);
}
