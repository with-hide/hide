//! Desk's own Codex home (with-hide/hide-private#388): the `CODEX_HOME`
//! Desk's Codex runs with, so it reads none of the operator's `~/.codex`
//! (global `AGENTS.md`, skills, MCP servers, hooks, rules or sign-in), and
//! the configuration and rules Hide writes there before every start.
//!
//! Codex's read-only sandbox blocks loopback, so a `hide` command run inside
//! it never reaches hided. Codex runs a command its rules `allow` outside the
//! sandbox and without asking, so the rules name exactly the `hide` commands
//! Desk runs without a card ([`crate::permission::HIDE_WITHOUT_CARD`]); every
//! other command still runs in the sandbox or asks, and reaches Hide as a
//! permission request. A rule matches the start of a command only, which is
//! why a Desk spawn's words for the agent's own CLI are refused by the core
//! rather than asked for here.
//!
//! The sign-in is a file in that home, so Desk needs one Codex sign-in of
//! its own, which `hide desk login codex` makes with the adapter's own Codex.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::permission::HIDE_WITHOUT_CARD;

/// What `hide desk login codex` is called; the line Desk shows when Codex
/// needs signing in (B6).
pub const LOGIN_COMMAND: &str = "hide desk login codex";

/// Desk's Codex configuration. Commands run with Desk's own `PATH`, not the
/// login shell's or a snapshot of it, so the `hide` Desk put first is the
/// one that runs; no skill or project instruction file reaches the model.
const CONFIG: &str = "\
# Written by Hide for Desk's Codex before every start; edits are replaced.
allow_login_shell = false
project_doc_max_bytes = 0

[features]
shell_snapshot = false

[skills]
include_instructions = false

[skills.bundled]
enabled = false
";

/// The rules file Desk's Codex reads.
fn rules() -> String {
    let mut text = String::from(
        "# Written by Hide for Desk's Codex before every start; edits are replaced.\n\
         # The hide commands Desk runs without a card run outside the sandbox.\n",
    );
    for words in HIDE_WITHOUT_CARD {
        let pattern = std::iter::once("hide")
            .chain(words.iter().copied())
            .map(|word| format!("\"{word}\""))
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&format!(
            "prefix_rule(pattern = [{pattern}], decision = \"allow\")\n"
        ));
    }
    text
}

/// The files Desk writes into `home` before Codex starts: path, body and
/// whether it is a program.
pub fn files(home: &Path) -> Vec<(PathBuf, String, bool)> {
    vec![
        (home.join("config.toml"), CONFIG.to_owned(), false),
        (home.join("rules").join("hide-desk.rules"), rules(), false),
    ]
}

/// The variable that points Codex at Desk's home.
pub fn home_variable(home: &Path) -> (OsString, OsString) {
    ("CODEX_HOME".into(), home.into())
}

/// `codex login` run by the adapter's own Codex with Desk's home, with the
/// operator's own options for it (`--device-auth`, `--with-api-key`). The
/// caller gives it the terminal and creates `home` first.
pub fn login(node: &Path, package: &Path, home: &Path, options: &[String]) -> Command {
    let mut command = Command::new(node);
    command
        .arg(package.join(crate::AdapterId::Codex.pin().entry))
        .args(["cli", "login"])
        .args(options);
    let (name, value) = home_variable(home);
    command.env(name, value);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hide_command_desk_frees_is_a_rule_and_nothing_else_is() {
        let rules = rules();
        let lines: Vec<&str> = rules
            .lines()
            .filter(|line| line.starts_with("prefix_rule"))
            .collect();
        assert_eq!(lines.len(), HIDE_WITHOUT_CARD.len());
        assert!(lines.contains(
            &"prefix_rule(pattern = [\"hide\", \"agent\", \"list\"], decision = \"allow\")"
        ));
        assert!(
            lines
                .iter()
                .all(|line| line.ends_with("decision = \"allow\")"))
        );
        assert!(!rules.contains("[\"hide\", \"stop\"]"));
        assert!(!rules.contains("\"archive\""));
    }
}
