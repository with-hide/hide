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

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_node_link::protocol::{OPENCODE_FORK_TIME, OpenCodeFork};
use hide_node_link::{ErrorCode, HostError};
use hide_platform::process::{RunFailure, run_to_end};
use hide_session::opencode::ForkRoute;

/// What a reason adds when OpenCode's server may have made the fork although
/// the node never read its id: the session stays in OpenCode's own list.
const MAY_EXIST: &str = "a fork may exist in OpenCode's session list";

/// Forks the root session `source` that OpenCode recorded in `cwd`.
pub(crate) fn fork(
    home: &Path,
    source: &str,
    cwd: &Path,
    stop: &AtomicBool,
) -> Result<OpenCodeFork, HostError> {
    // Found and run on the account's own search, as every program a node
    // starts by name is, so a CLI installed as a script finds its interpreter.
    fork_with(home, source, cwd, stop, || {
        hide_platform::programs::command("opencode")
    })
}

/// [`fork`] with the `opencode` command it runs, made only for a session in
/// 2.x's tables.
fn fork_with(
    home: &Path,
    source: &str,
    cwd: &Path,
    stop: &AtomicBool,
    opencode: impl FnOnce() -> Command,
) -> Result<OpenCodeFork, HostError> {
    let checkout = cwd
        .to_str()
        .ok_or_else(|| HostError::new(ErrorCode::InvalidPath, "session_route_cwd_unconfirmed"))?;
    match hide_session::opencode::fork_route(home, source, checkout)
        .map_err(|reason| HostError::new(ErrorCode::Io, reason))?
    {
        ForkRoute::Flag => Ok(OpenCodeFork::Flag),
        ForkRoute::Server => fork_on_server(opencode(), source, cwd, OPENCODE_FORK_TIME, stop),
    }
}

/// Runs `opencode api` to fork `source` on OpenCode's server. The CLI runs
/// with the node's own environment, the one `fork_route` read OpenCode's
/// database with (its data folder may be `XDG_DATA_HOME`). Once it has
/// started, a run that ends without an id it can read says a fork may exist.
fn fork_on_server(
    mut command: Command,
    source: &str,
    cwd: &Path,
    deadline: Duration,
    stop: &AtomicBool,
) -> Result<OpenCodeFork, HostError> {
    command
        .args(["api", "POST", &format!("/api/session/{source}/fork")])
        .args(["--data", "{}"])
        .current_dir(cwd);
    let finished = run_to_end(&mut command, deadline, stop).map_err(|failure| {
        HostError::new(
            ErrorCode::Io,
            match failure {
                RunFailure::TimedOut => format!("opencode_fork_timed_out: {MAY_EXIST}"),
                RunFailure::Stopped => format!("opencode_fork_stopped: {MAY_EXIST}"),
                RunFailure::Start(error) => format!("opencode_fork_not_started: {error}"),
                RunFailure::Wait(error) => {
                    format!("opencode_fork_unconfirmed: {error}; {MAY_EXIST}")
                }
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
    let malformed = || {
        HostError::new(
            ErrorCode::Io,
            format!("opencode_fork_answer_malformed: {MAY_EXIST}"),
        )
    };
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
                "opencode_fork_answer_malformed: a fork may exist in OpenCode's session list",
                "{answer:?}"
            );
        }
    }

    /// The root session both recorded OpenCode fixtures hold.
    #[cfg(unix)]
    const ROOT: &str = "ses_0a1b2c3d4e5f60718293a4b5c6";

    /// An OpenCode database in `home` from a recorded fixture, its sessions
    /// moved to `checkout`.
    #[cfg(unix)]
    fn database(home: &Path, fixture: &str, sessions: &str, checkout: &Path) {
        let folder = home.join(".local/share/opencode");
        std::fs::create_dir_all(&folder).unwrap();
        let database = rusqlite::Connection::open(folder.join("opencode.db")).unwrap();
        database.execute_batch(fixture).unwrap();
        database
            .execute(
                &format!("UPDATE {sessions} SET directory = ?1"),
                [checkout.to_str().unwrap()],
            )
            .unwrap();
    }

    #[cfg(unix)]
    fn opencode(folder: &Path, body: &str) -> std::path::PathBuf {
        let program = folder.join("opencode");
        crate::executable_fixture::write_executable(&program, &format!("#!/bin/sh\n{body}\n"));
        program
    }

    /// A session in 2.0.29's tables is forked by the `opencode` the node finds
    /// by name on its search, asked once in the checkout with the session's
    /// own fork route, and its answer is the pane's id; a session in
    /// 1.18.30's tables keeps its flag and runs no `opencode` at all.
    #[cfg(unix)]
    #[test]
    fn a_2x_session_is_forked_by_the_opencode_on_the_search_and_a_1x_one_keeps_its_flag() {
        let home = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let checkout = hide_platform::fs::identity::canonical(folder.path()).unwrap();
        let bin = checkout.join("bin");
        std::fs::create_dir(&bin).unwrap();
        let asked = checkout.join("asked");
        opencode(
            &bin,
            &format!(
                "printf '%s|' \"$PWD\" \"$@\" >> '{}'\nprintf '{{\"data\":{{\"id\":\"ses_fork\"}}}}'",
                asked.display()
            ),
        );
        database(
            home.path(),
            include_str!("../../hide-session/tests/fixtures/adapters/opencode-2.0.29/opencode.sql"),
            "session_v2",
            &checkout,
        );
        let answer = fork_with(
            home.path(),
            ROOT,
            &checkout,
            &AtomicBool::new(false),
            || hide_platform::programs::command_on(Some(bin.as_os_str()), "opencode"),
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
                "{}|api|POST|/api/session/{ROOT}/fork|--data|{{}}|",
                checkout.display()
            )
        );

        let one = tempfile::tempdir().unwrap();
        database(
            one.path(),
            include_str!(
                "../../hide-session/tests/fixtures/adapters/opencode-1.18.30/opencode.sql"
            ),
            "session",
            &checkout,
        );
        assert_eq!(
            fork_with(one.path(), ROOT, &checkout, &AtomicBool::new(false), || {
                panic!("a 1.x session runs no opencode")
            }),
            Ok(OpenCodeFork::Flag)
        );
    }

    /// A server that refuses ends the fork with its reason; a CLI that never
    /// answers ends it with a reason that says the server may have made one.
    /// No id is made up.
    #[cfg(unix)]
    #[test]
    fn a_refused_or_unanswered_fork_is_a_reason_and_no_session() {
        let folder = tempfile::tempdir().unwrap();
        let refusing = opencode(
            folder.path(),
            "printf '{\"error\":\"not found\"}'\necho 'HTTP 404 Not Found' >&2\nexit 1",
        );
        let refused = fork_on_server(
            Command::new(&refusing),
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
            Command::new(&hanging),
            "ses_root",
            folder.path(),
            Duration::from_millis(200),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(
            timed_out.message,
            "opencode_fork_timed_out: a fork may exist in OpenCode's session list"
        );
    }
}
