//! Forking an OpenCode session for a pane to open (NodeLink `opencode_fork`).
//!
//! OpenCode 1.x forks a session itself when its interactive command is given
//! `--fork`. 2.x's interactive command has no `--fork` (2.0.29,
//! `packages/cli/src/commands/commands.ts`: the root command takes
//! `--session`, `--continue` and `--prompt`; only `mini` and `run` take
//! `--fork`), so the node asks 2.x's server to make the fork, through
//! `opencode api`, the CLI's own request to the server the account's
//! interactive OpenCode uses (`session.fork`, `POST /api/session/<id>/fork`),
//! and answers the new session's id for the pane to open. Which of the two a
//! session takes is decided by the schema of the database holding it
//! (`hide_session::opencode::fork_route`), the way every OpenCode read is.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_node_link::protocol::{OPENCODE_FORK_TIME, OpenCodeFork};
use hide_node_link::{ErrorCode, HostError};
use hide_platform::process::{RunFailure, run_to_end};
use hide_session::opencode::ForkRoute;

/// Forks the root session `source` that OpenCode recorded in `cwd`.
pub(crate) fn fork(
    home: &Path,
    source: &str,
    cwd: &Path,
    stop: &AtomicBool,
) -> Result<OpenCodeFork, HostError> {
    let checkout = cwd
        .to_str()
        .ok_or_else(|| HostError::new(ErrorCode::InvalidPath, "session_route_cwd_unconfirmed"))?;
    match hide_session::opencode::fork_route(home, source, checkout)
        .map_err(|reason| HostError::new(ErrorCode::Io, reason))?
    {
        ForkRoute::Flag => Ok(OpenCodeFork::Flag),
        ForkRoute::Server => {
            let program = hide_ai::resolve_binary(Path::new("opencode"))
                .ok_or_else(|| HostError::new(ErrorCode::Unsupported, "opencode_not_installed"))?;
            let path = hide_platform::programs::account_path().ok_or_else(|| {
                HostError::new(ErrorCode::Unsupported, "opencode_path_unavailable")
            })?;
            fork_on_server(&program, &path, source, cwd, OPENCODE_FORK_TIME, stop)
        }
    }
}

/// Runs `opencode api` to fork `source` on OpenCode's server. The CLI runs
/// with the node's own environment, the one `fork_route` read OpenCode's
/// database with (its data folder may be `XDG_DATA_HOME`), and the `PATH` the
/// program was found on, so a CLI installed as a script finds its interpreter.
fn fork_on_server(
    program: &Path,
    path: &OsStr,
    source: &str,
    cwd: &Path,
    deadline: Duration,
    stop: &AtomicBool,
) -> Result<OpenCodeFork, HostError> {
    let mut command = Command::new(program);
    command
        .args(["api", "POST", &format!("/api/session/{source}/fork")])
        .args(["--data", "{}"])
        .current_dir(cwd)
        .env("PATH", path);
    let finished = run_to_end(&mut command, deadline, stop).map_err(|failure| {
        HostError::new(
            ErrorCode::Io,
            match failure {
                RunFailure::TimedOut => "opencode_fork_timed_out".to_owned(),
                RunFailure::Stopped => "opencode_fork_stopped".to_owned(),
                RunFailure::Start(error) => format!("opencode_fork_not_started: {error}"),
                RunFailure::Wait(error) => format!("opencode_fork_unconfirmed: {error}"),
            },
        )
    })?;
    if !finished.succeeded() {
        return Err(HostError::new(
            ErrorCode::Io,
            format!("opencode_fork_refused: {}", finished.last_error_line()),
        ));
    }
    forked(&finished.stdout).map(|id| OpenCodeFork::Forked { id })
}

/// The new session's id in the server's answer, `{"data": {"id": ...}}`
/// (2.0.29 `packages/client`, `session.fork`), held to the rule every
/// OpenCode id is, so it cannot reach the pane's argument list as a flag.
fn forked(answer: &str) -> Result<String, HostError> {
    let malformed = || HostError::new(ErrorCode::Io, "opencode_fork_answer_malformed");
    let value: serde_json::Value = serde_json::from_str(answer.trim()).map_err(|_| malformed())?;
    value["data"]["id"]
        .as_str()
        .filter(|id| hide_session::valid_native_id(id))
        .map(str::to_owned)
        .ok_or_else(malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_new_session_is_read_from_the_servers_answer_and_held_to_the_id_rule() {
        assert_eq!(
            forked("{\"data\":{\"id\":\"ses_fork\",\"parentID\":\"ses_root\"}}\n").unwrap(),
            "ses_fork"
        );
        for answer in [
            "",
            "not json",
            "{\"data\":{}}",
            "{\"id\":\"ses_fork\"}",
            "{\"data\":{\"id\":\"--dangerously-skip-permissions\"}}",
            "{\"data\":{\"id\":\"..\"}}",
        ] {
            assert_eq!(
                forked(answer).unwrap_err().message,
                "opencode_fork_answer_malformed",
                "{answer:?}"
            );
        }
    }

    #[cfg(unix)]
    fn opencode(folder: &Path, body: &str) -> std::path::PathBuf {
        let program = folder.join("opencode");
        crate::executable_fixture::write_executable(&program, &format!("#!/bin/sh\n{body}\n"));
        program
    }

    /// The fork is the server's: the CLI is asked once, in the checkout,
    /// with the session's own fork route, and its answer is the pane's id.
    #[cfg(unix)]
    #[test]
    fn opencode_2_forks_the_session_through_its_servers_own_route() {
        let folder = tempfile::tempdir().unwrap();
        let checkout = hide_platform::fs::identity::canonical(folder.path()).unwrap();
        let asked = checkout.join("asked");
        let program = opencode(
            &checkout,
            &format!(
                "printf '%s|' \"$PWD\" \"$@\" >> '{}'\nprintf '{{\"data\":{{\"id\":\"ses_fork\"}}}}'",
                asked.display()
            ),
        );
        let answer = fork_on_server(
            &program,
            OsStr::new("/usr/bin:/bin"),
            "ses_root",
            &checkout,
            OPENCODE_FORK_TIME,
            &AtomicBool::new(false),
        )
        .unwrap();

        assert_eq!(
            answer,
            OpenCodeFork::Forked {
                id: "ses_fork".into()
            }
        );
        assert_eq!(
            std::fs::read_to_string(&asked).unwrap(),
            format!(
                "{}|api|POST|/api/session/ses_root/fork|--data|{{}}|",
                checkout.display()
            )
        );
    }

    /// A server that refuses, or a CLI that never answers, ends the fork with
    /// its reason; no id is made up.
    #[cfg(unix)]
    #[test]
    fn a_refused_or_unanswered_fork_is_a_reason_and_no_session() {
        let folder = tempfile::tempdir().unwrap();
        let refusing = opencode(
            folder.path(),
            "printf '{\"error\":\"not found\"}'\necho 'HTTP 404 Not Found' >&2\nexit 1",
        );
        let refused = fork_on_server(
            &refusing,
            OsStr::new("/usr/bin:/bin"),
            "ses_root",
            folder.path(),
            OPENCODE_FORK_TIME,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(refused.message, "opencode_fork_refused: HTTP 404 Not Found");

        let silent = folder.path().join("silent");
        std::fs::create_dir(&silent).unwrap();
        let hanging = opencode(&silent, "exec sleep 30");
        let timed_out = fork_on_server(
            &hanging,
            OsStr::new("/usr/bin:/bin"),
            "ses_root",
            folder.path(),
            Duration::from_millis(200),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(timed_out.message, "opencode_fork_timed_out");
    }
}
