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
//! - `ssh` with `BatchMode=yes` ([`remote`]): `ssh` never asks for a password
//!   or a host key, and a connection that stops answering is dropped; the
//!   operator's own ssh command keeps its key and options;
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

macro_rules! ssh_options {
    () => {
        "-o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=15 -o ServerAliveCountMax=4"
    };
}

/// `ssh` never asks for a password or a host key, gives up on a host that
/// does not answer, and drops a connection that stops answering. Added after
/// the operator's own options, so an option they set wins.
pub const SSH_OPTIONS: &str = ssh_options!();

/// The ssh command when the operator names none.
pub const SSH_COMMAND: &str = concat!("ssh ", ssh_options!());

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

/// [`at`] for a run that reaches a remote (a fetch, a clone). The ssh it
/// starts is the operator's, in Git's own order: `GIT_SSH_COMMAND`, then
/// `core.sshCommand` as Git reads it in `cwd`, then `GIT_SSH`, then `ssh`; when
/// that command is `ssh`, [`SSH_OPTIONS`] follow the operator's own options.
/// Another program (a wrapper, `plink`) runs as configured, with no prompt
/// to answer it, since stdin is closed and every askpass is empty.
pub fn remote(cwd: &Path) -> Command {
    let mut command = at(cwd);
    let resolved = ssh_command(
        std::env::var("GIT_SSH_COMMAND").ok(),
        || configured_ssh_command(cwd),
        std::env::var("GIT_SSH").ok(),
    );
    match resolved {
        Some(ssh) => command.env("GIT_SSH_COMMAND", ssh),
        // `GIT_SSH` is not ssh: Git starts it itself, with its own variant.
        None => command.env_remove("GIT_SSH_COMMAND"),
    };
    command
}

/// The `GIT_SSH_COMMAND` a remote run gets; `None` leaves Git to start the
/// operator's `GIT_SSH` program.
fn ssh_command(
    environment: Option<String>,
    config: impl FnOnce() -> Option<String>,
    git_ssh: Option<String>,
) -> Option<String> {
    let named = |value: Option<String>| value.filter(|value| !value.trim().is_empty());
    let command = match named(environment).or_else(|| named(config())) {
        Some(command) => command,
        None => match named(git_ssh) {
            Some(program) if is_ssh(&program) => {
                return Some(format!(
                    "'{}' {SSH_OPTIONS}",
                    program.replace('\'', "'\\''")
                ));
            }
            Some(_) => return None,
            None => return Some(SSH_COMMAND.to_owned()),
        },
    };
    Some(if is_ssh(program_of(&command)) {
        format!("{command} {SSH_OPTIONS}")
    } else {
        command
    })
}

/// The program a shell command line starts: its first word, read whole when
/// it is quoted, as `"C:/Program Files/OpenSSH/ssh.exe" -i key` is.
fn program_of(command: &str) -> &str {
    let command = command.trim_start();
    match command.chars().next() {
        Some(quote @ ('"' | '\'')) => command[1..].split(quote).next().unwrap_or_default(),
        _ => command.split_whitespace().next().unwrap_or_default(),
    }
}

/// Whether a program, by its file name, is OpenSSH's client.
fn is_ssh(program: &str) -> bool {
    let program = program.trim_matches(['\'', '"']);
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let name = name.to_ascii_lowercase();
    name == "ssh" || name == "ssh.exe"
}

/// `core.sshCommand` as Git reads it in `cwd`: every scope and include.
fn configured_ssh_command(cwd: &Path) -> Option<String> {
    let output = output(
        at(cwd).args(["config", "--get", "core.sshCommand"]),
        Duration::from_secs(5),
        Some(64 * 1024),
    )
    .ok()??;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The Git on this machine, as `(major, minor)`, read once per process;
/// `None` when it cannot be read.
pub fn version() -> Option<(u32, u32)> {
    static VERSION: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
    *VERSION.get_or_init(|| {
        let output = output(command().arg("--version"), DEADLINE, Some(4 * 1024)).ok()??;
        parse_version(&String::from_utf8_lossy(&output.stdout))
    })
}

/// `git version 2.39.5 (Apple Git-154)` or `git version 2.45.1.windows.1`.
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text
        .trim()
        .strip_prefix("git version ")?
        .split(['.', ' '])
        .map(str::parse::<u32>);
    Some((parts.next()?.ok()?, parts.next()?.ok()?))
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
    fn the_operators_ssh_command_keeps_its_options_and_never_asks() {
        let none = || None;
        assert_eq!(ssh_command(None, none, None).as_deref(), Some(SSH_COMMAND));
        assert_eq!(
            ssh_command(None, || Some("ssh -i ~/.ssh/work_key".into()), None).as_deref(),
            Some(format!("ssh -i ~/.ssh/work_key {SSH_OPTIONS}").as_str())
        );
        // The environment comes before the configuration, as in Git.
        assert_eq!(
            ssh_command(
                Some("/usr/bin/ssh -F /cfg".into()),
                || panic!("the configuration is not read"),
                None
            )
            .as_deref(),
            Some(format!("/usr/bin/ssh -F /cfg {SSH_OPTIONS}").as_str())
        );
        // A wrapper takes no ssh options; it runs as written.
        assert_eq!(
            ssh_command(None, || Some("/opt/bin/ssh-wrap --vault".into()), None).as_deref(),
            Some("/opt/bin/ssh-wrap --vault")
        );
        assert_eq!(
            ssh_command(
                None,
                none,
                Some("C:\\Program Files\\OpenSSH\\ssh.exe".into())
            )
            .as_deref(),
            Some(format!("'C:\\Program Files\\OpenSSH\\ssh.exe' {SSH_OPTIONS}").as_str())
        );
        assert_eq!(
            ssh_command(None, none, Some("C:\\PuTTY\\plink.exe".into())),
            None
        );
        // Windows spells a program under Program Files in quotes.
        let quoted = "\"C:/Program Files/OpenSSH/ssh.exe\" -i C:/keys/work";
        assert_eq!(
            ssh_command(None, || Some(quoted.into()), None).as_deref(),
            Some(format!("{quoted} {SSH_OPTIONS}").as_str())
        );
        assert_eq!(
            ssh_command(None, || Some("'/opt/my tools/wrap' --vault".into()), None).as_deref(),
            Some("'/opt/my tools/wrap' --vault")
        );
    }

    #[test]
    fn the_configured_ssh_command_is_read_from_the_repository() {
        let folder = tempfile::tempdir().unwrap();
        let git = |arguments: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(folder.path())
                    .args(arguments)
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&["init", "--quiet"]);
        git(&["config", "core.sshCommand", "ssh -i /keys/work"]);
        assert_eq!(
            configured_ssh_command(folder.path()).as_deref(),
            Some("ssh -i /keys/work")
        );
        let environment = remote(folder.path())
            .get_envs()
            .find(|(name, _)| *name == "GIT_SSH_COMMAND")
            .and_then(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()));
        if std::env::var_os("GIT_SSH_COMMAND").is_none() {
            assert_eq!(
                environment.as_deref(),
                Some(format!("ssh -i /keys/work {SSH_OPTIONS}").as_str())
            );
        }
    }

    #[test]
    fn git_versions_read_on_every_system() {
        assert_eq!(
            parse_version("git version 2.39.5 (Apple Git-154)\n"),
            Some((2, 39))
        );
        assert_eq!(
            parse_version("git version 2.45.1.windows.1\n"),
            Some((2, 45))
        );
        assert_eq!(parse_version("git version 2.25.1\n"), Some((2, 25)));
        assert_eq!(parse_version("hub version 2.14"), None);
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
