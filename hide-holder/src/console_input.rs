//! Whether a Windows pane's console edits lines, asked of a helper attached
//! to it (`hide_platform::process::console_input`), never by the holder
//! itself: a process attached to the console is ended by the console's close
//! whatever it answers, and the holder holds the pane's program and its
//! conversation.
//!
//! The helper is this program started again in its helper mode
//! (`--console-input <shell pid>`, `main.rs`). It is started on the watch's
//! first recheck of a program, at least [`crate::watch::LINE_EDITING_RECHECK`]
//! after the program took the pane's foreground, so a short command never
//! starts one, and it is asked on rechecks only; the facts in between carry
//! the last answer. It lives until the console first reads keys under the
//! program, for at most [`crate::watch::LINE_EDITING_RECHECK_FOR`], one per
//! pane. After that the program is read as it was last read until another
//! program takes the foreground: a program that once read keys is not
//! waiting to start, and a helper kept for every running agent would be a
//! process per pane.

use std::process::Command;
use std::time::Instant;

use hide_holder_protocol::LineEditing;
use hide_platform::process::console_input::ConsoleInput;

use crate::log::Log;
use crate::watch::{LINE_EDITING_RECHECK_FOR, LineAsk};

/// The argument that starts this program as the helper.
pub const HELPER_ARGUMENT: &str = "--console-input";

/// How many helpers one program's wait may start: one, and one more when the
/// first ended or stopped answering. A console that ends every helper is
/// read as not known from then on.
const STARTS_PER_PROGRAM: u32 = 2;

/// The console reads of one watch, for the program in the pane's foreground.
pub struct ConsoleLines {
    shell: u32,
    log: Log,
    program: Option<Program>,
}

struct Program {
    pid: u32,
    since: Instant,
    helper: Option<ConsoleInput>,
    starts: u32,
    /// The last answer, and the one given once the program read keys, the
    /// wait passed its bound or the helper could not be had.
    last: LineEditing,
    settled: bool,
}

impl ConsoleLines {
    pub fn new(shell: u32, log: Log) -> Self {
        Self {
            shell,
            log,
            program: None,
        }
    }

    /// Whether the console edits lines under `program`, the pane's foreground
    /// program (`None` while the shell holds it, which is read as not known,
    /// as before: only a started program's reading keys is waited on). Only
    /// a [`LineAsk::Recheck`] asks the console; a program is not known until
    /// the first one has.
    pub fn read(&mut self, program: Option<u32>, ask: LineAsk) -> LineEditing {
        let Some(pid) = program else {
            self.program = None;
            return LineEditing::Unknown;
        };
        let (shell, log) = (self.shell, &self.log);
        let current = match &mut self.program {
            Some(current) if current.pid == pid => current,
            slot => slot.insert(Program {
                pid,
                since: Instant::now(),
                helper: None,
                starts: 0,
                last: LineEditing::Unknown,
                settled: false,
            }),
        };
        if current.settled || ask == LineAsk::Facts {
            return current.last;
        }
        if current.since.elapsed() >= LINE_EDITING_RECHECK_FOR {
            // Past the start's deadline a program still editing lines is
            // not frozen as editing them: it is not known from then on, as
            // a console that cannot be read is.
            unread(log, pid, "wait_bound", None);
            return current.settle(LineEditing::Unknown);
        }
        let helper = match &mut current.helper {
            Some(helper) => helper,
            None if current.starts >= STARTS_PER_PROGRAM => {
                unread(log, pid, "helper_starts_spent", None);
                return current.settle(LineEditing::Unknown);
            }
            None => {
                current.starts += 1;
                match helper_command(shell)
                    .and_then(|mut command| ConsoleInput::start(&mut command))
                {
                    Ok(helper) => current.helper.insert(helper),
                    Err(error) => {
                        unread(log, pid, "helper_not_started", Some(&error));
                        return current.settle(LineEditing::Unknown);
                    }
                }
            }
        };
        match helper.edits_lines() {
            Ok(true) => {
                current.last = LineEditing::Edits;
                current.last
            }
            Ok(false) => current.settle(LineEditing::ReadsKeys),
            Err(error) => {
                // The next recheck starts another helper, within the bound.
                current.helper = None;
                unread(log, pid, "helper_unanswered", Some(&error));
                current.last
            }
        }
    }
}

/// This program started again as the helper for the console `shell` runs
/// in: the holder's own file in its version folder, so the helper is the
/// same build.
fn helper_command(shell: u32) -> std::io::Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command.arg(HELPER_ARGUMENT).arg(shell.to_string());
    Ok(command)
}

fn unread(log: &Log, program: u32, reason: &str, error: Option<&std::io::Error>) {
    let mut fields: Vec<(&str, serde_json::Value)> = vec![
        ("program", u64::from(program).into()),
        ("reason", reason.into()),
    ];
    if let Some(error) = error {
        fields.push(("error", error.to_string().into()));
    }
    log.event("holder.console_input_unread", &fields);
}

impl Program {
    /// Ends the wait on this program with `answer`, letting the helper go,
    /// and answers it.
    fn settle(&mut self, answer: LineEditing) -> LineEditing {
        self.last = answer;
        self.settled = true;
        self.helper = None;
        answer
    }
}
