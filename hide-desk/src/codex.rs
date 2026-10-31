//! Desk's own Codex home (with-hide/hide-private#388): the `CODEX_HOME`
//! Desk's Codex runs with, so it reads none of the operator's `~/.codex`
//! setup (global `AGENTS.md`, skills, MCP servers, hooks or rules), and the
//! configuration and rules Hide writes there before every start.
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

use crate::permission::HIDE_WITHOUT_CARD;

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
