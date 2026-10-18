//! The real Claude Code in a pane of the pinned Herdr, in a worktree of a
//! repository it was never told to trust: it starts at its prompt in the
//! worktree whose trust Hide recorded through the node (`trust_worktree`),
//! and asks its folder trust question in the repository's other worktree,
//! which shows the check reads the question where Claude asks it. Claude Code
//! shows the question before it needs a sign-in, so no account is needed.
//!
//! It needs the pinned Herdr in `HIDE_E2E_HERDR_BIN` and Claude Code in
//! `HIDE_TEST_CLAUDE_BIN`, so it is ignored by default; the nightly agent
//! trust contract installs the pinned Claude Code and runs it on every
//! system.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;

use super::real_herdr_tests::PrivateHerdr;
use super::*;

/// Claude Code 2.1.302's folder trust question.
const TRUST_QUESTION: &str = "Quick safety check";
/// The banner over Claude Code's prompt, which its trust question replaces.
const PROMPT_BANNER: &str = "Claude Code v";

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
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

/// A tab running `claude` in `folder`, in a workspace of its own.
fn claude_in(herdr: &PrivateHerdr, claude: &Path, folder: &Path, label: &str) -> String {
    let (workspace, _, _) = crate::wire::created_workspace(herdr.request(
        "workspace.create",
        json!({"cwd": folder, "label": label, "focus": false}),
    ))
    .unwrap();
    let env = [
        ("DISABLE_AUTOUPDATER", "1"),
        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let command = [claude.to_str().unwrap().to_owned()];
    let tab = crate::live::create_tab_in(
        &herdr.connector(),
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

#[test]
#[ignore = "needs the pinned Herdr (HIDE_E2E_HERDR_BIN) and Claude Code (HIDE_TEST_CLAUDE_BIN)"]
fn the_real_claude_starts_at_its_prompt_in_a_worktree_hide_trusted_and_asks_in_another() {
    let claude = PathBuf::from(
        std::env::var_os("HIDE_TEST_CLAUDE_BIN").expect("HIDE_TEST_CLAUDE_BIN names Claude Code"),
    );
    let herdr = PrivateHerdr::new(None);
    let home = herdr.home.clone();
    // An account where Claude Code has run before and trusts no folder.
    std::fs::write(
        home.join(".claude.json"),
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

    let node = hide_node::Local::new(Some(home.clone()));
    trust_worktree(
        Some(&node),
        "claude",
        &hide_platform::path::to_wire(&task).unwrap(),
    )
    .unwrap();

    let asked = claude_in(&herdr, &claude, &other, "other");
    herdr.wait_on(
        &asked,
        Duration::from_secs(90),
        "Claude Code asks its folder trust question in a worktree no one trusted",
        || herdr.screen(&asked).contains(TRUST_QUESTION),
    );
    let trusted = claude_in(&herdr, &claude, &task, "task");
    herdr.wait_on(
        &trusted,
        Duration::from_secs(90),
        "Claude Code starts at its prompt in the worktree Hide trusted",
        || herdr.screen(&trusted).contains(PROMPT_BANNER),
    );
    let screen = herdr.screen(&trusted);
    assert!(
        !screen.contains(TRUST_QUESTION),
        "Claude Code asked in the worktree Hide trusted:\n{screen}"
    );
}
