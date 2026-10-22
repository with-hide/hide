//! The one way a node starts `git` (PRD git-freshness D-11).
//!
//! Hide runs Git on its own: reads every two seconds, a background fetch of
//! every registered repository, a clone. None of these has a person at a
//! terminal, so no Git it starts may ask for anything: a credential, a host
//! key, a passphrase or a Credential Manager window would hold the run until
//! its deadline, or put a window in front of the operator for a fetch they
//! never asked for. Every invocation therefore starts here, with the same
//! environment on macOS, Linux and Windows:
//!
//! - `GIT_TERMINAL_PROMPT=0`: Git never reads a credential from a terminal;
//! - empty `GIT_ASKPASS` and `SSH_ASKPASS`, `SSH_ASKPASS_REQUIRE=never`: no
//!   askpass program is started, the operator's `core.askPass` included;
//! - `GCM_INTERACTIVE=never` and `credential.interactive=never`: Git
//!   Credential Manager answers from what it already holds or fails;
//! - `GIT_SSH_COMMAND` with `BatchMode=yes`: `ssh` never asks for a password
//!   or a host key, and a connection that stops answering is dropped;
//! - `--no-optional-locks`: a read never takes the index lock the operator's
//!   own Git may want;
//! - stdin is closed, and [`output`] ends the whole process tree at its
//!   deadline and refuses an answer past its output cap.
//!
//! A credential helper may still hold what it needs and answer at once,
//! which is how a fetch uses the operator's login without asking for it.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// The most one ordinary `git` invocation may take. A repository that cannot
/// answer in this time reports its status unavailable rather than holding
/// every other project's answer behind it; a status over evicted iCloud files
/// ran for minutes before this bound existed.
pub const DEADLINE: Duration = Duration::from_secs(15);

/// The most a command that changes the repository may take (a checkout, a
/// branch setting). It is long, because stopping a checkout part way leaves
/// the operator's tree half switched; it exists so a Git that hangs on a
/// hook or a filter cannot hold a node worker for good.
pub const CHANGE_DEADLINE: Duration = Duration::from_secs(10 * 60);

/// The most output one invocation may answer when its caller names no
/// smaller cap: the index of a very large repository fits, and a runaway
/// output is refused rather than held in memory.
pub const OUTPUT_LIMIT: u64 = 256 * 1024 * 1024;

/// `ssh` never asks for a password or a host key, gives up on a host that
/// does not answer, and drops a connection that stops answering.
pub const SSH_COMMAND: &str =
    "ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=15 -o ServerAliveCountMax=4";

/// `git` with the non-interactive environment and global options, and
/// nothing else: no working directory and no subcommand yet.
pub fn command() -> Command {
    let mut command = hide_platform::programs::command("git");
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("SSH_ASKPASS_REQUIRE", "never")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_SSH_COMMAND", SSH_COMMAND)
        .args(["-c", "credential.interactive=never", "--no-optional-locks"])
        .stdin(Stdio::null());
    command
}

/// [`command`] that runs in `cwd`.
pub fn at(cwd: &Path) -> Command {
    let mut command = command();
    command.arg("-C").arg(cwd);
    command
}

/// Runs `command` to completion within `deadline`, its stdout and stderr each
/// at most `limit` bytes ([`OUTPUT_LIMIT`] when `None`). `Ok(None)` is a run
/// that was stopped at its deadline; past the cap is an error.
pub fn output(
    command: &mut Command,
    deadline: Duration,
    limit: Option<u64>,
) -> std::io::Result<Option<std::process::Output>> {
    crate::worktrees::output_limited(command, deadline, Some(limit.unwrap_or(OUTPUT_LIMIT)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `git` a node runs starts at [`command`]: a second spelling of
    /// the program anywhere else in the crate's product code would start Git
    /// without the non-interactive environment.
    #[test]
    fn no_product_code_starts_git_outside_the_launcher() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stray = Vec::new();
        let mut folders = vec![source];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    folders.push(path);
                    continue;
                }
                if path.extension().is_none_or(|extension| extension != "rs")
                    || path.ends_with("git_run.rs")
                {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                // Test modules build their fixtures with a plain Git.
                let product = text.split("#[cfg(test)]").next().unwrap_or_default();
                for spelling in ["command(\"git\")", "Command::new(\"git\")"] {
                    if product.contains(spelling) {
                        stray.push(format!("{}: {spelling}", path.display()));
                    }
                }
            }
        }
        assert!(stray.is_empty(), "git started outside git_run: {stray:?}");
    }

    #[test]
    fn the_launcher_names_every_non_interactive_setting() {
        let command = at(Path::new("/repository"));
        let environment: std::collections::BTreeMap<_, _> = command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect();
        for (name, value) in [
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_ASKPASS", ""),
            ("SSH_ASKPASS", ""),
            ("SSH_ASKPASS_REQUIRE", "never"),
            ("GCM_INTERACTIVE", "never"),
            ("GIT_SSH_COMMAND", SSH_COMMAND),
        ] {
            assert_eq!(
                environment.get(name).cloned().flatten().as_deref(),
                Some(value),
                "{name}"
            );
        }
        let arguments: Vec<_> = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            arguments,
            [
                "-c",
                "credential.interactive=never",
                "--no-optional-locks",
                "-C",
                "/repository"
            ]
        );
    }
}
