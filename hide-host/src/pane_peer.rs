//! The pane bootstrap's words for process identity (an `i32` pid, a start as
//! an optional number) over `hide-platform`, which answers each question for
//! the system it runs on. hided and a device's node (`panes`) ask the
//! same questions through here.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hide_herdr_client::{ApiConnector, LocalSocketConnector, request_small_response_strict_until};
use hide_platform::process;
use serde_json::{Value, json};

const HERDR_TIMEOUT: Duration = Duration::from_secs(2);

pub use hide_node_link::panes::PaneIdentity;

/// Whether `pid` is `shell_pid` or one of its descendants.
pub fn descends_from(pid: i32, shell_pid: i32) -> bool {
    match (u32::try_from(pid), u32::try_from(shell_pid)) {
        (Ok(pid), Ok(shell)) => process::descends_from(pid, shell),
        _ => false,
    }
}

/// When `pid` started, comparable only with another start of the same
/// system; `None` once the process is gone.
pub fn process_start(pid: i32) -> Option<u64> {
    process::start_time(u32::try_from(pid).ok()?).ok()
}

/// The peer's current directory as the system reports it, or `None` when the
/// process is gone or refuses inspection.
pub fn process_cwd(pid: i32) -> Option<PathBuf> {
    process::cwd_of(u32::try_from(pid).ok()?).ok()
}

/// The identity of the pane `pane_id` of the Herdr listening at `socket`.
pub fn inspect(socket: &Path, pane_id: &str) -> Result<PaneIdentity, &'static str> {
    inspect_address(socket, pane_id).map(|(_, identity)| identity)
}

/// Resolves inherited caller aliases and proves the current address's shell.
pub fn inspect_address(
    socket: &Path,
    pane_id: &str,
) -> Result<(String, PaneIdentity), &'static str> {
    inspect_address_until(socket, pane_id, Instant::now() + HERDR_TIMEOUT * 3)
}

/// The current address Herdr associates with an inherited caller reference.
/// This is routing evidence only; callers must separately attest the shell.
pub fn current_address(
    connector: &dyn ApiConnector,
    pane_id: &str,
    deadline: Instant,
) -> Result<String, &'static str> {
    if pane_id.is_empty() || pane_id.len() > 256 {
        return Err("invalid_request");
    }
    let answer = request_small_response_strict_until(
        connector,
        "pane.current",
        json!({"caller_pane_id": pane_id}),
        stage_deadline(deadline)?,
    )
    .map_err(|_| "pane_unavailable")?;
    answer
        .pointer("/pane/pane_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .map(str::to_owned)
        .ok_or("pane_unavailable")
}

/// The same pane proof under one caller-owned deadline, shared by both reads.
pub fn inspect_until(
    socket: &Path,
    pane_id: &str,
    deadline: Instant,
) -> Result<PaneIdentity, &'static str> {
    inspect_address_until(socket, pane_id, deadline).map(|(_, identity)| identity)
}

fn inspect_address_until(
    socket: &Path,
    pane_id: &str,
    deadline: Instant,
) -> Result<(String, PaneIdentity), &'static str> {
    if !socket.is_absolute() || pane_id.is_empty() || pane_id.len() > 256 {
        return Err("invalid_request");
    }
    let connector = LocalSocketConnector::new(socket);
    let pane_id = current_address(&connector, pane_id, deadline)?;
    let process = request_small_response_strict_until(
        &connector,
        "pane.process_info",
        json!({"pane_id":pane_id}),
        stage_deadline(deadline)?,
    )
    .map_err(|_| "pane_unavailable")?;
    let shell = process
        .pointer("/process_info/shell_pid")
        .and_then(Value::as_u64)
        .filter(|pid| *pid > 0 && *pid <= i32::MAX as u64)
        .ok_or("pane_unavailable")? as i32;
    let started = process_start(shell).ok_or("pane_unavailable")?;
    let pane = request_small_response_strict_until(
        &connector,
        "pane.get",
        json!({"pane_id":pane_id}),
        stage_deadline(deadline)?,
    )
    .map_err(|_| "pane_unavailable")?;
    let terminal_id = pane
        .pointer("/pane/terminal_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("pane_unavailable")?
        .to_owned();
    Ok((
        pane_id,
        PaneIdentity {
            terminal_id,
            shell_pid: shell,
            shell_started: started,
        },
    ))
}

fn stage_deadline(deadline: Instant) -> Result<Instant, &'static str> {
    let now = Instant::now();
    if deadline <= now {
        return Err("pane_inspection_expired");
    }
    Ok(deadline.min(now + HERDR_TIMEOUT))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    #[test]
    fn an_inherited_alias_proves_the_current_terminal_and_shell() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("p.sock");
        let listener = hide_platform::ipc::LocalListener::bind(&socket).unwrap();
        let closer = listener.closer();
        let server = std::thread::spawn(move || {
            let mut seen = Vec::new();
            while let Ok(mut stream) = listener.accept() {
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut line = String::new();
                BufReader::new(&mut stream).read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                let method = request["method"].as_str().unwrap();
                let result = match method {
                    "pane.current" => {
                        assert_eq!(request["params"]["caller_pane_id"], "w1:p1");
                        json!({"pane":{"pane_id":"w2:p2"}})
                    }
                    "pane.process_info" => {
                        assert_eq!(request["params"]["pane_id"], "w2:p2");
                        json!({"process_info":{"shell_pid":std::process::id()}})
                    }
                    "pane.get" => {
                        assert_eq!(request["params"]["pane_id"], "w2:p2");
                        json!({"pane":{"terminal_id":"same-terminal"}})
                    }
                    _ => panic!("unexpected proof request"),
                };
                seen.push(method.to_owned());
                writeln!(stream, "{}", json!({"id":request["id"],"result":result})).unwrap();
            }
            seen
        });
        let answer = inspect_address(&socket, "w1:p1");
        closer.close();
        let seen = server.join().unwrap();
        let (address, identity) = answer.unwrap();
        assert_eq!(address, "w2:p2");
        assert_eq!(identity.terminal_id, "same-terminal");
        assert_eq!(identity.shell_pid, std::process::id() as i32);
        assert_eq!(
            Some(identity.shell_started),
            process_start(identity.shell_pid)
        );
        assert_eq!(seen, ["pane.current", "pane.process_info", "pane.get"]);
    }
}
