//! Desk's own Codex home (with-hide/hide-private#388): the `CODEX_HOME`
//! Desk's Codex runs with, so it reads none of the operator's `~/.codex`
//! setup (global `AGENTS.md`, skills, MCP servers, hooks or rules), and the
//! configuration and rules Hide writes there before every start.
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
//! The sign-in stays the operator's own (PRD Desk D-81): the home's
//! `auth.json` is a link to the operator's, made again before every start,
//! as Hide AI's Codex homes do. Codex writes a refreshed token through such
//! a link to its target (its credential file module: "Symlinks retain their
//! target"; a hard link it refuses), so both read the same sign-in, and
//! Hide never reads or copies it. A file an earlier Desk-only sign-in left
//! there is replaced by the link. A link to a file needs a privilege on a
//! Windows account without Developer Mode, where Desk's Codex then does not
//! start.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

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

/// The operator's own Codex sign-in: `auth.json` in the folder their Codex
/// uses, the account's `CODEX_HOME` or else `~/.codex` under `home`.
pub fn operator_sign_in(home: &Path, codex_home: Option<&std::ffi::OsStr>) -> PathBuf {
    codex_home
        .filter(|folder| !folder.is_empty())
        .map_or_else(|| home.join(".codex"), PathBuf::from)
        .join("auth.json")
}

/// The link Desk makes before Codex starts, `(target, link)`: Desk's home's
/// `auth.json` leads to the operator's sign-in.
pub fn sign_in_link(operator: &Path, home: &Path) -> (PathBuf, PathBuf) {
    (operator.to_path_buf(), home.join("auth.json"))
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

    #[test]
    fn the_sign_in_is_the_operator_s_from_their_codex_folder() {
        let home = Path::new("/home/me");
        assert_eq!(
            operator_sign_in(home, None),
            home.join(".codex").join("auth.json")
        );
        assert_eq!(
            operator_sign_in(home, Some(std::ffi::OsStr::new("/elsewhere/codex"))),
            Path::new("/elsewhere/codex").join("auth.json")
        );
        assert_eq!(
            operator_sign_in(home, Some(std::ffi::OsStr::new(""))),
            home.join(".codex").join("auth.json")
        );
    }
}
