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
//! What decides that a kept list is stale is [`mark`]: the program that
//! answers and the skills folders the agent's definition names, which is
//! what an agent's version and the operator's skills change.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use hide_agent_adapter::{AccountPath, NodeEnv, Place, Profile};
use serde_json::Value;

use crate::AdapterId;
use crate::adapter::Launch;
use crate::commands::{self, Commands};
use crate::host::Host;
use crate::rpc::{CallError, Incoming};
use crate::session::{self, AUTH_REQUIRED};

/// The longest one read runs, from the start of the process to its list: a
/// cold start of agy took 38 s to open its session on this Mac.
pub const READ_DEADLINE: Duration = Duration::from_secs(60);
/// How long a read waits for the list once the session opened: every agent
/// measured sent it within a tenth of a second.
pub const LIST_WAIT: Duration = Duration::from_secs(10);
/// How often a read looks at its stop flag while it waits.
const POLL: Duration = Duration::from_millis(200);

/// Why a read brought no list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Unread {
    /// The agent refused the session until the operator signs in.
    SignIn,
    /// The session opened and the agent sent no list in [`LIST_WAIT`].
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

/// Reads `adapter`'s commands from a process `launch` starts, in `cwd`.
/// The process and everything it started end before this returns, on every
/// path; `stop` ends the read within [`POLL`].
pub fn read(
    adapter: AdapterId,
    launch: &Launch,
    cwd: &Path,
    stop: &AtomicBool,
) -> Result<Commands, Unread> {
    read_within(adapter, launch, cwd, stop, READ_DEADLINE, LIST_WAIT)
}

/// [`read`] within `deadline` from the start and `list_wait` after the
/// session opened.
fn read_within(
    adapter: AdapterId,
    launch: &Launch,
    cwd: &Path,
    stop: &AtomicBool,
    deadline: Duration,
    list_wait: Duration,
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
    let result = listen(&host, &heard, cwd, stop, deadline, list_wait);
    let tail = host.stderr_tail();
    let ended = host.end();
    match result {
        Ok(update) => Ok(commands::read(adapter, &update)),
        Err(Unread::Failed(detail)) if !tail.is_empty() => {
            Err(Unread::Failed(format!("{detail}: {tail}")))
        }
        Err(unread) => {
            // A tree the system could not confirm ended is worth the log
            // only when nothing else went wrong.
            if let (Unread::NoList, Err(error)) = (&unread, ended) {
                return Err(Unread::Failed(format!("the process did not end: {error}")));
            }
            Err(unread)
        }
    }
}

/// Opens the session and waits for the list, within `deadline`.
fn listen(
    host: &Host,
    heard: &mpsc::Receiver<Heard>,
    cwd: &Path,
    stop: &AtomicBool,
    deadline: Duration,
    list_wait: Duration,
) -> Result<Value, Unread> {
    let connection = host.connection();
    let mut deadline = Instant::now() + deadline;
    let mut list = None;
    let mut opened = false;
    let mut signed_out = false;
    let refused = |step: &str, error: CallError, signed_out: bool| match error {
        CallError::Rejected(rejected) if rejected.code == AUTH_REQUIRED || signed_out => {
            Unread::SignIn
        }
        other => Unread::Failed(format!("{step}: {other}")),
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
            return Err(if opened {
                Unread::NoList
            } else {
                Unread::Failed("no session within the deadline".into())
            });
        }
        let heard = match heard.recv_timeout(POLL.min(deadline - now)) {
            Ok(heard) => heard,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(Unread::Failed("the stream ended".into()));
            }
        };
        match heard {
            Heard::Reply(INITIALIZE, Ok(_)) => connection
                .send("session/new", session::new_session_params(cwd), NEW_SESSION)
                .map_err(|error| Unread::Failed(format!("session/new: {error}")))?,
            Heard::Reply(INITIALIZE, Err(error)) => {
                return Err(refused("initialize", error, signed_out));
            }
            Heard::Reply(NEW_SESSION, Ok(_)) => {
                if let Some(list) = list {
                    return Ok(list);
                }
                opened = true;
                deadline = deadline.min(Instant::now() + list_wait);
            }
            Heard::Reply(NEW_SESSION, Err(error)) => {
                return Err(refused("session/new", error, signed_out));
            }
            Heard::Reply(..) => {}
            // Kept until the session's answer: an agent may send it first.
            Heard::Commands(update) if opened => return Ok(update),
            Heard::Commands(update) => list = Some(update),
            Heard::SignedOut => signed_out = true,
            Heard::Asked(id) => {
                let _ = connection.respond_error(&id, -32601, "Desk offers no client methods");
            }
            Heard::Closed(why) => return Err(Unread::Failed(format!("closed: {why}"))),
        }
    }
}

/// Entries of one skills folder a mark looks at; past them the rest of the
/// folder changes nothing until the daily read.
const MAX_MARKED_SKILLS: usize = 1024;

/// What `adapter`'s list depends on, as a line that changes when it does:
/// the adapter Hide pins, the agent's own program (its resolved file, size
/// and change time, which an update replaces), and each skill in the skills
/// folder the agent's definition names (`[account.paths] skills`, the
/// shared `~/.agents/skills` for `"shared"`), by name and change time.
/// `cli` is the program Desk would start; `home` and `variable` are the
/// account's, as a new terminal has them. Only file facts are read: no
/// process starts.
pub fn mark(
    adapter: AdapterId,
    cli: Option<&Path>,
    home: Option<&Path>,
    variable: impl Fn(&str) -> Option<OsString>,
) -> String {
    let mut line = String::new();
    if let Some(pin) = adapter.pin() {
        line.push_str(&format!("pin {}@{}\n", pin.package, pin.version));
    }
    if let Some(cli) = cli {
        line.push_str(&format!("cli {}\n", file_facts(cli)));
    }
    let catalog = adapter.catalog();
    let account = home.and_then(|home| {
        catalog
            .resolve(
                &Profile::builtin_default(catalog.id),
                &NodeEnv::new(Some(home.to_path_buf()))
                    .with_account_variables(catalog.account_rule(), variable),
            )
            .ok()
    });
    let skills: Option<PathBuf> = match account.as_ref().map(|a| a.place(AccountPath::Skills)) {
        Some(Place::At(path)) => Some(path.clone()),
        Some(Place::Shared) => home.map(|home| home.join(".agents").join("skills")),
        Some(Place::Absent) | None => None,
    };
    if let Some(skills) = skills {
        line.push_str(&format!("skills {}\n", skills.display()));
        let mut entries: Vec<(String, PathBuf)> = std::fs::read_dir(&skills)
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
    use std::io::{BufRead, Write};

    /// The variable that runs this test binary as a fake agent, and the
    /// file it writes its pid to.
    const ROLE: &str = "HIDE_DESK_READ_AHEAD_AGENT";
    const PID_FILE: &str = "HIDE_DESK_READ_AHEAD_PID";

    /// A fake ACP agent: this binary run again with [`ROLE`] set. `list`
    /// sends its list after the session opens, `early` before it answers,
    /// `sign_in` refuses the session, `silent` opens it and sends none,
    /// `stuck` answers nothing. Run as a test without it, it returns at once.
    #[test]
    fn fake_agent() {
        let Ok(role) = std::env::var(ROLE) else {
            return;
        };
        if let Ok(file) = std::env::var(PID_FILE) {
            std::fs::write(file, std::process::id().to_string()).unwrap();
        }
        let out = std::io::stdout();
        let say = |message: Value| {
            let mut out = out.lock();
            writeln!(out, "{message}").unwrap();
            out.flush().unwrap();
        };
        let list = serde_json::json!({"jsonrpc": "2.0", "method": "session/update", "params": {
            "sessionId": "s", "update": {"sessionUpdate": "available_commands_update",
                "availableCommands": [{"name": "compact", "description": "Compact"}, {"name": "logout", "description": "Out"}]}}});
        for line in std::io::stdin().lock().lines() {
            let Ok(message) = serde_json::from_str::<Value>(&line.unwrap()) else {
                continue;
            };
            let id = message["id"].clone();
            match (role.as_str(), message["method"].as_str()) {
                ("stuck", _) => {}
                (_, Some("initialize")) => {
                    say(
                        serde_json::json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": 1}}),
                    );
                }
                ("sign_in", Some("session/new")) => {
                    say(serde_json::json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": AUTH_REQUIRED, "message": "Authentication required"}}))
                }
                (role, Some("session/new")) => {
                    if role == "early" {
                        say(list.clone());
                    }
                    say(
                        serde_json::json!({"jsonrpc": "2.0", "id": id, "result": {"sessionId": "s"}}),
                    );
                    if role == "list" {
                        say(list.clone());
                    }
                }
                _ => {}
            }
        }
    }

    /// Reads as the fake agent `role` in `folder`, and the pid it ran as,
    /// which it writes to `folder/pid` as it starts.
    fn read_in(folder: &Path, role: &str, stop: &AtomicBool) -> (Result<Commands, Unread>, u32) {
        let pid_file = folder.join("pid");
        let launch = Launch {
            program: std::env::current_exe().unwrap(),
            args: [
                "--exact",
                "read_ahead::tests::fake_agent",
                "--nocapture",
                "-q",
                "--test-threads=1",
            ]
            .map(OsString::from)
            .to_vec(),
            env: vec![
                (ROLE.into(), role.into()),
                (PID_FILE.into(), pid_file.clone().into()),
            ],
            cwd: folder.to_owned(),
        };
        let result = read_within(
            AdapterId::CODEX,
            &launch,
            folder,
            stop,
            Duration::from_secs(20),
            Duration::from_millis(500),
        );
        let pid = std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|pid| pid.parse().ok())
            .unwrap_or(0);
        (result, pid)
    }

    fn read_as(role: &str, stop: &AtomicBool) -> (Result<Commands, Unread>, u32) {
        let folder = tempfile::tempdir().unwrap();
        read_in(folder.path(), role, stop)
    }

    /// A read brings the list the agent sends as its session opens, before
    /// or after its answer, with what the definition leaves out gone, and
    /// says why it brought none; the process is gone after every read.
    #[test]
    fn a_read_brings_the_agent_s_list_or_why_not_and_ends_its_process_every_time() {
        let go = AtomicBool::new(false);
        for role in ["list", "early"] {
            let (read, pid) = read_as(role, &go);
            let names: Vec<String> = read
                .unwrap()
                .commands
                .into_iter()
                .map(|command| command.name)
                .collect();
            // Codex's definition leaves `logout` out.
            assert_eq!(names, ["/compact"], "{role}");
            assert!(pid != 0 && !hide_platform::process::is_alive(pid), "{role}");
        }
        let (read, pid) = read_as("sign_in", &go);
        assert_eq!(read.unwrap_err(), Unread::SignIn);
        assert!(!hide_platform::process::is_alive(pid));
        let (read, pid) = read_as("silent", &go);
        assert_eq!(read.unwrap_err(), Unread::NoList);
        assert!(!hide_platform::process::is_alive(pid));
    }

    /// A read the caller stops ends at once with its process, however long
    /// the agent would have kept it waiting.
    #[test]
    fn a_stopped_read_ends_its_process_within_a_poll() {
        let folder = tempfile::tempdir().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopper = {
            let stop = Arc::clone(&stop);
            let pid_file = folder.path().join("pid");
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(15);
                while !pid_file.exists() {
                    assert!(Instant::now() < deadline, "the fake agent never started");
                    #[allow(clippy::disallowed_methods)]
                    std::thread::sleep(Duration::from_millis(50)); // wait-ok: the next poll of the fake agent's pid file
                }
                stop.store(true, Ordering::SeqCst);
            })
        };
        let started = Instant::now();
        let (read, pid) = read_in(folder.path(), "stuck", &stop);
        stopper.join().unwrap();
        assert_eq!(read.unwrap_err(), Unread::Stopped);
        assert!(started.elapsed() < Duration::from_secs(15));
        assert!(pid != 0 && !hide_platform::process::is_alive(pid));
    }

    #[test]
    fn a_mark_changes_with_the_program_and_with_a_skill_added_or_edited() {
        let home = tempfile::tempdir().unwrap();
        let skills = home.path().join(".agents/skills");
        std::fs::create_dir_all(skills.join("one")).unwrap();
        std::fs::write(skills.join("one/SKILL.md"), "one").unwrap();
        let cli = home.path().join("codex-cli");
        std::fs::write(&cli, "v1").unwrap();
        let none = |_: &str| None;
        // OpenCode reads the shared folder (`skills = "shared"`).
        let first = mark(AdapterId::OPENCODE, Some(&cli), Some(home.path()), none);
        assert!(first.contains("skill one "), "{first}");
        assert_eq!(
            mark(AdapterId::OPENCODE, Some(&cli), Some(home.path()), none),
            first
        );

        std::fs::create_dir_all(skills.join("two")).unwrap();
        let added = mark(AdapterId::OPENCODE, Some(&cli), Some(home.path()), none);
        assert_ne!(added, first);

        std::fs::write(&cli, "version two").unwrap();
        let updated = mark(AdapterId::OPENCODE, Some(&cli), Some(home.path()), none);
        assert_ne!(updated, added);

        // Claude's folder is its own, under `CLAUDE_CONFIG_DIR` when set.
        let config = home.path().join("claude-config");
        std::fs::create_dir_all(config.join("skills/mine")).unwrap();
        let claude = mark(AdapterId::CLAUDE, None, Some(home.path()), |name: &str| {
            (name == "CLAUDE_CONFIG_DIR").then(|| config.clone().into_os_string())
        });
        assert!(claude.contains("skill mine "), "{claude}");
        assert!(claude.starts_with("pin @agentclientprotocol/claude-agent-acp@"));
    }
}
