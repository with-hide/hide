//! Reading an agent's own commands before the operator talks to it, so `/`
//! lists them at once in Desk's composer and in a session's input (mission-106
//! command-lists). Every agent Desk speaks with sends its
//! `available_commands_update` as its session opens, with no prompt (each
//! measured on 2026-11-27: Claude, Codex, Grok, OpenCode, Pi, omp and agy, in
//! one to three seconds once warm): so a read starts the agent's ACP process
//! as Desk would, opens a session, waits a bounded time for the list and
//! ends the whole tree, on every path. Nothing is prompted, so no model is
//! asked and the process gets no capability ([`crate::adapter::Paths`]).
//!
//! The session the read opened is deleted again where the agent offers it
//! (`sessionCapabilities.delete`: Claude Code, Codex, OpenCode and Pi as
//! measured), so it does not stay in the agent's list of conversations to
//! resume; omp, Grok and agy offer only `close`, which keeps it, so each of
//! their reads leaves one empty session there.
//!
//! What decides that a kept list is stale is [`mark`]: the program that
//! answers and the skills folder the agent's definition names, which is
//! what an agent's version and the operator's skills change.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::AdapterId;
use crate::adapter::Launch;
use crate::commands::{self, Commands};
use crate::host::Host;
use crate::rpc::{CallError, Incoming};
use crate::session;

/// How long one read may take.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bounds {
    /// From the start of the process to its list: a cold start of agy took
    /// 38 s to open its session on this Mac.
    pub deadline: Duration,
    /// From the session's answer to its list: every agent measured sent it
    /// within a tenth of a second.
    pub list_wait: Duration,
    /// After a list, for another one (Grok sends its list twice as it
    /// opens); the longest one heard is the read's.
    pub settle: Duration,
    /// For the agent's answer to deleting the session the read opened.
    pub forget: Duration,
}

impl Bounds {
    pub const DEFAULT: Self = Self {
        deadline: Duration::from_secs(60),
        list_wait: Duration::from_secs(10),
        settle: Duration::from_secs(1),
        forget: Duration::from_secs(2),
    };
}

/// How often a read looks at its stop flag while it waits.
const POLL: Duration = Duration::from_millis(200);

/// Why a read brought no list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Unread {
    /// The agent refused the session until the operator signs in
    /// ([`session::signs_in`]).
    SignIn,
    /// The session opened and the agent sent no list, or an empty one.
    NoList,
    /// The process did not start, or its session did not open; the detail
    /// is for the log.
    Failed(String),
    /// The caller stopped it (Desk switched off, hided ending).
    Stopped,
}

/// What the read's process sent that the read uses; everything else the
/// agent sends is dropped as it arrives, so nothing queues past a few
/// messages whatever the agent writes.
enum Heard {
    Reply(u64, Result<Value, CallError>),
    Commands(Value),
    SignedOut,
    Asked(Value),
    Closed(String),
}

const INITIALIZE: u64 = 1;
const NEW_SESSION: u64 = 2;
const DELETE_SESSION: u64 = 3;

/// Reads `adapter`'s commands from a process `launch` starts, in `cwd`,
/// within `bounds`. The process and everything it started end before this
/// returns, on every path; `stop` ends the read within a fifth of a second.
pub fn read(
    adapter: AdapterId,
    launch: &Launch,
    cwd: &Path,
    stop: &AtomicBool,
    bounds: Bounds,
) -> Result<Commands, Unread> {
    let (sender, heard) = mpsc::channel();
    let sender = Mutex::new(sender);
    let sink: crate::rpc::Sink = Arc::new(move |incoming: Incoming| {
        let heard = match incoming {
            Incoming::Reply { tag, result } => Heard::Reply(tag, result),
            Incoming::Notification { method, params } => {
                if session::sign_in_status(&method, &params) == Some(false) {
                    Heard::SignedOut
                } else if method == "session/update"
                    && params
                        .pointer("/update/sessionUpdate")
                        .and_then(Value::as_str)
                        == Some("available_commands_update")
                {
                    Heard::Commands(params["update"].clone())
                } else {
                    return;
                }
            }
            Incoming::Request { id, .. } => Heard::Asked(id),
            Incoming::Closed { why } => Heard::Closed(why),
        };
        let _ = sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .send(heard);
    });
    let host = Host::start(launch, sink)
        .map_err(|error| Unread::Failed(format!("the process did not start: {error}")))?;
    let result = listen(&host, &heard, cwd, stop, bounds);
    let tail = host.stderr_tail();
    let ended = host.end();
    let read = match result {
        Ok(update) => commands::read(adapter, &update),
        Err(Unread::Failed(detail)) if !tail.is_empty() => {
            return Err(Unread::Failed(format!("{detail}: {tail}")));
        }
        Err(unread) => return Err(unread),
    };
    if let Err(error) = ended {
        return Err(Unread::Failed(format!("the process did not end: {error}")));
    }
    // A list with nothing in it, or nothing a menu can show, is no list.
    if read.commands.is_empty() {
        return Err(Unread::NoList);
    }
    Ok(read)
}

/// Opens the session and waits for the list, within `bounds`.
fn listen(
    host: &Host,
    heard: &mpsc::Receiver<Heard>,
    cwd: &Path,
    stop: &AtomicBool,
    bounds: Bounds,
) -> Result<Value, Unread> {
    let connection = host.connection();
    let mut deadline = Instant::now() + bounds.deadline;
    let mut list: Option<Value> = None;
    let mut opened: Option<Option<String>> = None;
    let mut can_delete = false;
    let mut signed_out = false;
    let refused = |step: &str, error: CallError, signed_out: bool| {
        if session::signs_in(&error, signed_out) {
            Unread::SignIn
        } else {
            Unread::Failed(format!("{step}: {error}"))
        }
    };
    connection
        .send("initialize", session::initialize_params(), INITIALIZE)
        .map_err(|error| Unread::Failed(format!("initialize: {error}")))?;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Err(Unread::Stopped);
        }
        let now = Instant::now();
        if now >= deadline {
            return match (opened, list) {
                (Some(session), Some(list)) => {
                    if can_delete && let Some(session) = session {
                        forget(connection, heard, &session, stop, bounds.forget);
                    }
                    Ok(list)
                }
                (Some(_), None) => Err(Unread::NoList),
                (None, _) => Err(Unread::Failed("no session within the deadline".into())),
            };
        }
        let heard = match heard.recv_timeout(POLL.min(deadline - now)) {
            Ok(heard) => heard,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(Unread::Failed("the stream ended".into()));
            }
        };
        match heard {
            Heard::Reply(INITIALIZE, Ok(answer)) => {
                can_delete = answer
                    .pointer("/agentCapabilities/sessionCapabilities/delete")
                    .is_some();
                connection
                    .send("session/new", session::new_session_params(cwd), NEW_SESSION)
                    .map_err(|error| Unread::Failed(format!("session/new: {error}")))?;
            }
            Heard::Reply(INITIALIZE, Err(error)) => {
                return Err(refused("initialize", error, signed_out));
            }
            Heard::Reply(NEW_SESSION, Ok(answer)) => {
                opened = Some(
                    answer
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                );
                let wait = if list.is_some() {
                    bounds.settle
                } else {
                    bounds.list_wait
                };
                deadline = deadline.min(Instant::now() + wait);
            }
            Heard::Reply(NEW_SESSION, Err(error)) => {
                return Err(refused("session/new", error, signed_out));
            }
            Heard::Reply(..) => {}
            // The longest list heard is the read's; one that comes before
            // the session's answer waits for it.
            Heard::Commands(update) => {
                let length = |update: &Value| {
                    update
                        .get("availableCommands")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len)
                };
                if list
                    .as_ref()
                    .is_none_or(|kept| length(&update) >= length(kept))
                {
                    list = Some(update);
                }
                if opened.is_some() {
                    deadline = deadline.min(Instant::now() + bounds.settle);
                }
            }
            Heard::SignedOut => signed_out = true,
            Heard::Asked(id) => {
                let _ = connection.respond_error(&id, -32601, "Desk offers no client methods");
            }
            Heard::Closed(why) => {
                return match (opened, list) {
                    (Some(_), Some(list)) => Ok(list),
                    _ => Err(Unread::Failed(format!("closed: {why}"))),
                };
            }
        }
    }
}

/// Deletes the session the read opened and waits for the agent's answer,
/// within `wait`: a session left behind is the agent's empty conversation
/// in its own list, so a refusal changes nothing the read brought.
fn forget(
    connection: &crate::rpc::Connection,
    heard: &mpsc::Receiver<Heard>,
    session: &str,
    stop: &AtomicBool,
    wait: Duration,
) {
    if connection
        .send(
            "session/delete",
            serde_json::json!({"sessionId": session}),
            DELETE_SESSION,
        )
        .is_err()
    {
        return;
    }
    let deadline = Instant::now() + wait;
    while !stop.load(Ordering::SeqCst) {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        match heard.recv_timeout(POLL.min(deadline - now)) {
            Ok(Heard::Reply(DELETE_SESSION, _) | Heard::Closed(_))
            | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Ok(Heard::Asked(id)) => {
                let _ = connection.respond_error(&id, -32601, "Desk offers no client methods");
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Entries of one skills folder a mark looks at; past them the rest of the
/// folder changes nothing until the daily read.
const MAX_MARKED_SKILLS: usize = 1024;

/// What `adapter`'s list depends on, as a line that changes when it does:
/// the adapter Hide pins, the agent's own program `cli` (its resolved file,
/// size and change time, which an update replaces), and each skill in
/// `skills`, the folder the agent's definition names (`[account.paths]
/// skills`), by name and change time. Only file facts are read: no process
/// starts.
pub fn mark(adapter: AdapterId, cli: Option<&Path>, skills: Option<&Path>) -> String {
    let mut line = String::new();
    if let Some(pin) = adapter.pin() {
        line.push_str(&format!("pin {}@{}\n", pin.package, pin.version));
    }
    if let Some(cli) = cli {
        line.push_str(&format!("cli {}\n", file_facts(cli)));
    }
    if let Some(skills) = skills {
        line.push_str(&format!("skills {}\n", skills.display()));
        let mut entries: Vec<(String, std::path::PathBuf)> = std::fs::read_dir(skills)
            .into_iter()
            .flatten()
            .flatten()
            .take(MAX_MARKED_SKILLS)
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                )
            })
            .collect();
        entries.sort();
        for (name, path) in entries {
            line.push_str(&format!(
                "skill {name} {} {}\n",
                modified(&path),
                modified(&path.join("SKILL.md"))
            ));
        }
    }
    line
}

/// A program's resolved file with its size and change time; a link an
/// update moves (Homebrew's `Cellar/<version>`) changes the first.
fn file_facts(path: &Path) -> String {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let size = std::fs::metadata(&real).map_or(0, |facts| facts.len());
    format!("{} {size} {}", real.display(), modified(&real))
}

/// When `path` (followed through links) last changed, in milliseconds since
/// the epoch; `-` when it cannot be read.
fn modified(path: &Path) -> String {
    std::fs::metadata(path)
        .and_then(|facts| facts.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(|| "-".into(), |since| since.as_millis().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_changes_with_the_program_and_with_a_skill_added_or_edited() {
        let home = tempfile::tempdir().unwrap();
        let skills = home.path().join("skills");
        std::fs::create_dir_all(skills.join("one")).unwrap();
        std::fs::write(skills.join("one/SKILL.md"), "one").unwrap();
        let cli = home.path().join("opencode");
        std::fs::write(&cli, "v1").unwrap();
        let first = mark(AdapterId::OPENCODE, Some(&cli), Some(&skills));
        assert!(first.contains("skill one "), "{first}");
        assert_eq!(mark(AdapterId::OPENCODE, Some(&cli), Some(&skills)), first);

        std::fs::create_dir_all(skills.join("two")).unwrap();
        let added = mark(AdapterId::OPENCODE, Some(&cli), Some(&skills));
        assert_ne!(added, first);

        std::fs::write(&cli, "version two").unwrap();
        let updated = mark(AdapterId::OPENCODE, Some(&cli), Some(&skills));
        assert_ne!(updated, added);

        // An agent spoken to through a pinned adapter carries the pin.
        let claude = mark(AdapterId::CLAUDE, None, None);
        assert!(claude.starts_with("pin @agentclientprotocol/claude-agent-acp@"));
    }
}
