//! Desk's own Codex home (with-hide/hide-private#388): the `CODEX_HOME`
//! Desk's Codex runs with, so it reads none of the operator's `~/.codex`
//! (global `AGENTS.md`, skills, MCP servers, hooks, rules or sign-in), and
//! the configuration and rules Hide writes there before every start.
//!
//! Codex's read-only sandbox blocks loopback, so a `hide` command run inside
//! it never reaches hided. Codex reads its rules once per session, so a rule
//! that let a command out of the sandbox without asking would keep the
//! Settings › Desk › 권한 table of the session's start and skip Hide's
//! decision. The rules therefore allow nothing: a command that needs more
//! than the sandbox, a `hide` command included, asks to run outside it, and
//! that request reaches Hide, which answers it from the table as it is then
//! ([`crate::permission`]). Desk's instructions tell the agent to ask so for
//! each `hide` command; one run in the sandbox first fails there, and Codex
//! then asks. The file is still written before every start, so rules an
//! older Hide wrote there are gone.
//!
//! The sign-in is a file in that home, so Desk needs one Codex sign-in of
//! its own, which `hide desk login codex` makes with the adapter's own Codex.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// The rules file Desk's Codex reads: no rule, so nothing leaves the
/// sandbox without a request Hide answers.
const RULES: &str = "\
# Written by Hide for Desk's Codex before every start; edits are replaced.
# No command runs outside the sandbox without asking: Hide answers each
# request from Settings > Desk > Permissions.
";

/// The files Desk writes into `home` before Codex starts: path, body and
/// whether it is a program.
pub fn files(home: &Path) -> Vec<(PathBuf, String, bool)> {
    vec![
        (home.join("config.toml"), CONFIG.to_owned(), false),
        (
            home.join("rules").join("hide-desk.rules"),
            RULES.to_owned(),
            false,
        ),
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
    fn desk_s_codex_rules_let_no_command_out_of_the_sandbox_on_their_own() {
        let written = files(Path::new("/state/desk/codex"));
        let (_, rules, _) = written
            .iter()
            .find(|(path, ..)| path.ends_with("rules/hide-desk.rules"))
            .expect("the rules file is written");
        assert!(
            rules
                .lines()
                .all(|line| line.is_empty() || line.starts_with('#')),
            "{rules}"
        );
    }
}
