//! Hands input to a running agent through the agent's own way in, on the
//! machine whose pane runs it (`Call::Deliver`; `docs/delivery.md`, The
//! input ladder).
//!
//! Each way in answers whether it sent the input whole, sent nothing, or
//! began and lost its answer, so the core never sends again what may have
//! arrived (engineering principle 11). The text and the token never reach a
//! log or an error: an answer's reason names only what failed.

use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use hide_agent_hooks::runtime::AgentRuntime;
use hide_node_link::input::{Delivery, InputChannel};
use hide_platform::process::{RunFailure, run_to_end};

/// How long `codex queue` may take: it answers in about 0.3 s (Codex
/// 0.166.0, measured 2026-11-01); this bounds one that does not.
const QUEUE_DEADLINE: Duration = Duration::from_secs(10);

/// How long a write to a Claude Code session's socket may take: the session
/// reads it at once, and nothing is read back.
const SOCKET_DEADLINE: Duration = Duration::from_secs(3);

/// Hands `text` to the agent behind `channel`, for the account whose home
/// is `home`.
pub fn deliver(home: &Path, channel: &InputChannel, text: &str, stop: &AtomicBool) -> Delivery {
    match channel {
        InputChannel::CodexQueue { thread } => codex_queue(home, thread, text, stop),
        InputChannel::ClaudeMessaging { socket, token } => {
            claude_messaging(Path::new(socket), token, text)
        }
    }
}

/// `codex queue --thread <thread> --message <text>`: Codex's own durable
/// queue for the session, which the running TUI takes as a turn. Exit 0 is
/// Codex's answer that it queued the message.
fn codex_queue(home: &Path, thread: &str, text: &str, stop: &AtomicBool) -> Delivery {
    let Some(codex) = hide_platform::programs::find_cli("codex") else {
        return Delivery::Unavailable {
            reason: "codex_not_found".into(),
        };
    };
    let mut command = Command::new(codex.path());
    hide_platform::process::restrict_to_login_environment(&mut command);
    command
        .args(["queue", "--thread", thread, "--message", text])
        .env("HOME", home)
        .env("CODEX_HOME", AgentRuntime::Codex.home_directory(home))
        .env("PATH", codex.search());
    match run_to_end(&mut command, QUEUE_DEADLINE, stop) {
        Ok(finished) if finished.succeeded() => Delivery::Received,
        // Codex refused before queueing: an unknown thread ("no rollout
        // found") or an older Codex without the command.
        Ok(finished) => Delivery::Unavailable {
            reason: format!(
                "codex queue exited {}: {}",
                finished
                    .code
                    .map_or_else(|| "by a signal".to_owned(), |code| code.to_string()),
                first_line(&finished.stderr)
            ),
        },
        Err(RunFailure::Start(error)) => Delivery::Unavailable {
            reason: format!("codex queue could not start: {}", error.kind()),
        },
        Err(RunFailure::Wait(error)) => Delivery::Unknown {
            reason: format!("codex queue could not be waited on: {}", error.kind()),
        },
        Err(RunFailure::TimedOut) => Delivery::Unknown {
            reason: "codex queue did not answer in time".into(),
        },
        Err(RunFailure::Stopped) => Delivery::Unknown {
            reason: "codex queue was stopped because Hide is quitting".into(),
        },
    }
}

/// The first line of a CLI's error output, cut to a log line's length.
fn first_line(text: &str) -> String {
    let line = text.lines().find(|line| !line.trim().is_empty()).unwrap_or("");
    line.chars().take(200).collect()
}

/// Writes the session's two lines, `auth` with the token its hook received
/// and the `user` message, to its messaging socket. The socket answers
/// nothing; the prompt hook's digest is the record (Claude Code 2.1.316,
/// measured 2026-11-01). A Windows session's socket is a named pipe no run
/// has measured, so there the rung is unavailable and the next one takes
/// the input (`docs/INSTALL.md`, What each system supports).
fn claude_messaging(socket: &Path, token: &str, text: &str) -> Delivery {
    if !hide_agent_adapter::AgentApi::ClaudeMessaging
        .systems()
        .include_this_one()
    {
        return Delivery::Unavailable {
            reason: "unsupported_system".into(),
        };
    }
    let lines = format!(
        "{}\n{}\n",
        serde_json::json!({"type": "auth", "token": token}),
        serde_json::json!({"type": "user", "message": {"role": "user", "content": text}}),
    );
    let deadline = Instant::now() + SOCKET_DEADLINE;
    let mut stream = match hide_platform::ipc::LocalStream::connect_until(socket, deadline) {
        Ok(stream) => stream,
        Err(error) => {
            return Delivery::Unavailable {
                reason: format!("the session's socket could not be reached: {}", error.kind()),
            };
        }
    };
    match stream.write_all(lines.as_bytes()).and_then(|()| stream.flush()) {
        Ok(()) => Delivery::Sent,
        Err(error) => Delivery::Unknown {
            reason: format!("the write to the session's socket failed: {}", error.kind()),
        },
    }
}
