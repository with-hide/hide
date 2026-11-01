//! Hands input to a running agent through the agent's own way in, on the
//! machine whose pane runs it (`Call::Deliver`; `docs/delivery.md`, The
//! input ladder).
//!
//! Each way in answers whether it sent the input whole, sent nothing, or
//! began and lost its answer, so the core never sends again what may have
//! arrived (engineering principle 11). The text never reaches a log or an
//! error: an answer's reason names only what failed.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_agent_hooks::runtime::AgentRuntime;
use hide_node_link::input::{Delivery, InputChannel};
use hide_platform::process::{RunFailure, run_to_end};

/// How long `codex queue` may take: it answers in about 0.3 s (Codex
/// 0.166.0, measured 2026-11-01); this bounds one that does not.
const QUEUE_DEADLINE: Duration = Duration::from_secs(10);

/// Hands `text` to the agent behind `channel`, for the account whose home
/// is `home`.
pub fn deliver(home: &Path, channel: &InputChannel, text: &str, stop: &AtomicBool) -> Delivery {
    match channel {
        InputChannel::CodexQueue { thread } => codex_queue(home, thread, text, stop),
    }
}

/// `codex queue --thread <thread> --message=<text>`: Codex's own durable
/// queue for the session, which the running TUI takes as a turn once it
/// rests with an empty composer. Exit 0 is Codex's answer that it queued
/// the message, not that the session took it. The message is one argument
/// (`--message=`), so a text that begins with `-` is never read as an
/// option.
fn codex_queue(home: &Path, thread: &str, text: &str, stop: &AtomicBool) -> Delivery {
    let Some(codex) = hide_platform::programs::find_cli("codex") else {
        return Delivery::Unavailable {
            reason: "codex_not_found".into(),
        };
    };
    let mut command = Command::new(codex.path());
    hide_platform::process::restrict_to_login_environment(&mut command);
    command
        .args(["queue", "--thread", thread])
        .arg(format!("--message={text}"))
        .env("HOME", home)
        .env("CODEX_HOME", AgentRuntime::Codex.home_directory(home))
        .env("PATH", codex.search());
    match run_to_end(&mut command, QUEUE_DEADLINE, stop) {
        Ok(finished) if finished.succeeded() => Delivery::Sent,
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
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    line.chars().take(200).collect()
}
