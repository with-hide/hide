//! `hide agent exec`, the command a pane's shell runs to start an agent
//! with a profile (PRD agent-profiles D-26; docs/ARCHITECTURE.md, Starting
//! an agent with a profile). It is not in the help: the core types it, with
//! the absolute path of the `hide` beside the pane's node, as the start line
//! of a profile's agent.
//!
//! ```text
//! hide agent exec --attempt <id> -- <program> <argument>...
//! ```
//!
//! The line names the start's attempt and the agent's own words, nothing of
//! its profile: it first asks the pane's node how the profile runs
//! (`AgentExecEvent::Profile`), answered from the profile as its machine has
//! it now, at the version the start was checked with
//! (`hide_core::launch::exec_start`). It adds the profile's variables to its
//! own environment and runs the agent's program in its place
//! (`hide_platform::process::hand_over`), so the variables reach that
//! program alone and never the shell that typed the line or its history. A
//! profile with a program of its own (its wrapper) runs that program, with
//! its leading words before the agent's arguments, instead of the agent's
//! `<program>`, which still names it in a failure.
//!
//! `--attempt` names the start the engine typed this line for, always the
//! first words after `agent exec`, so a bounded read of this process's
//! arguments names it (`hide_agent_hooks::hook_request::is_attempt_executor`),
//! and its shape is this build's (`valid_attempt`): a line of another
//! build's shape is refused here before anything is asked or run.
//! It tells the pane's node of that start (`HookCommand::AgentExec`), and the
//! node takes the report only from this process as its kernel names it:
//! a profile its node refuses now (gone, being removed, changed since its
//! start was checked) is reported as the check's refusal; it checks the
//! program as the system will start it
//! (`hide_platform::programs::runnable`) and reports a refusal; it reports
//! `Ready` and starts the program only when that report is answered ok, so
//! a start that has ended does not run late; it reports a start that failed
//! after the check; and on Windows, where it stays the program's parent, it
//! reports the program's process once that program is in its job and
//! resumed. It never reports that the program runs: the engine reads that
//! of the process itself (docs/ARCHITECTURE.md, Starting an agent with a
//! profile). A program that does not start ends the start here too, with a
//! line in the pane that names the profile and the program, and the exit
//! code a shell gives a command it could not find (127) or run (126).
//!
//! It runs before anything else in `hide` (`bin/hide.rs`): before the pane's
//! own installation is handed the call (`pane_install::hand_off`), which
//! would leave a second process between the shell and the agent.

use std::ffi::{OsStr, OsString};
use std::process::Command;

use hide_agent_hooks::hook_request::{
    AGENT_EXEC_FAILURE_LIMIT, AgentExecEvent, ExecProcess, ExecProfile, ExecStart,
};

/// The exit code of a start whose program was not found.
const NOT_FOUND: i32 = 127;
/// The exit code of a start whose program could not run, or whose profile
/// its node refused.
const NOT_RUN: i32 = 126;
/// The exit code of a start line this command could not read.
const MISUSED: i32 = 2;

/// Runs the start `arguments` (this process's own, its name first) ask for
/// and returns the exit code to end with, or `None` when they are not
/// `agent exec`. On Unix a start that succeeds never returns.
pub fn run(arguments: &[OsString]) -> Option<i32> {
    if arguments.get(1).map(OsString::as_os_str) != Some(OsStr::new("agent"))
        || arguments.get(2).map(OsString::as_os_str) != Some(OsStr::new("exec"))
    {
        return None;
    }
    let misused = |why: String| {
        eprintln!("hide: this agent start line cannot be read: {why}");
        Some(MISUSED)
    };
    let attempt = match attempt_of(&arguments[3..]) {
        Ok(attempt) => attempt,
        Err(why) => return misused(why),
    };
    let argv = match parse(&arguments[5..]) {
        Ok(argv) => argv,
        Err(why) => return misused(why),
    };
    let start = match profile(attempt) {
        Ok(ExecProfile::Ready(start)) => start,
        Ok(ExecProfile::Refused { failure }) => {
            report(
                attempt,
                AgentExecEvent::Refused {
                    failure: failure.clone(),
                },
            );
            eprintln!("hide: {failure}. This pane is a shell again.");
            return Some(NOT_RUN);
        }
        Err(why) => {
            eprintln!(
                "hide: Hide did not take this agent's start ({why}). This pane is a shell again."
            );
            return Some(NOT_RUN);
        }
    };
    Some(run_start(attempt, &start, &argv))
}

/// Runs `argv`, the agent's own words, as `start` says, for `attempt`.
fn run_start(attempt: &str, start: &ExecStart, argv: &[OsString]) -> i32 {
    let program = start
        .program
        .as_ref()
        .map_or_else(|| argv[0].clone(), OsString::from);
    let rest = &argv[1..];
    let failure = |why: &dyn std::fmt::Display| {
        format!(
            "{} could not start {}: {why}",
            start.label,
            program.to_string_lossy()
        )
    };
    let ended = |failure: &str, code| {
        eprintln!("hide: {failure}. This pane is a shell again.");
        code
    };
    let search = start
        .env
        .iter()
        .rev()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| OsString::from(value))
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    if let Err((why, code)) = startable(&program, &search) {
        let failure = failure(&why);
        report(
            attempt,
            AgentExecEvent::Refused {
                failure: failure.clone(),
            },
        );
        return ended(&failure, code);
    }
    // The program starts only once the engine has taken this start: one it
    // has already answered for is not run late.
    if let Err(why) = ready(attempt) {
        return ended(
            &failure(&format_args!("Hide did not take its start ({why})")),
            NOT_RUN,
        );
    }
    let mut command = Command::new(&program);
    command.args(&start.leading).args(rest);
    for (name, value) in &start.env {
        command.env(name, value);
    }
    match hide_platform::process::hand_over(command, |child| spawned(attempt, child)) {
        Ok(code) => code,
        Err(error) => {
            let code = if error.kind() == std::io::ErrorKind::NotFound {
                NOT_FOUND
            } else {
                NOT_RUN
            };
            let failure = failure(&error);
            report(
                attempt,
                AgentExecEvent::Failed {
                    failure: failure.clone(),
                },
            );
            ended(&failure, code)
        }
    }
}

/// The attempt the first words after `agent exec` name
/// ([`hide_agent_hooks::hook_request::AGENT_EXEC_ATTEMPT`]).
fn attempt_of(arguments: &[OsString]) -> Result<&str, String> {
    use hide_agent_hooks::hook_request::{AGENT_EXEC_ATTEMPT, valid_attempt};
    match arguments {
        [flag, attempt, ..] if flag == AGENT_EXEC_ATTEMPT => attempt
            .to_str()
            .filter(|attempt| valid_attempt(attempt))
            .ok_or_else(|| format!("{AGENT_EXEC_ATTEMPT} names no start")),
        _ => Err(format!("it names no {AGENT_EXEC_ATTEMPT} first")),
    }
}

/// Whether `program` starts with `search` as its `PATH`, as the system
/// will start it: a name found there, or a path, that this account can run
/// (`hide_platform::programs::runnable`). Why not, and the exit code a shell
/// gives it, otherwise.
fn startable(program: &OsStr, search: &OsStr) -> Result<(), (String, i32)> {
    let named = std::path::Path::new(program);
    if named.components().count() > 1 {
        return hide_platform::programs::runnable(named, Some(search)).map_err(|why| {
            let code = if named.exists() { NOT_RUN } else { NOT_FOUND };
            (why, code)
        });
    }
    let name = program
        .to_str()
        .ok_or_else(|| ("its name is not text".to_owned(), NOT_FOUND))?;
    let found = hide_platform::programs::Program::find(search, name)
        .ok_or_else(|| (format!("{name} is not on this pane's PATH"), NOT_FOUND))?;
    hide_platform::programs::runnable(found.path(), Some(search)).map_err(|why| (why, NOT_RUN))
}

/// Asks the pane's node how the profile of `attempt` runs now: its answer,
/// or why the node did not take the ask (a start that ended, or a node that
/// did not answer).
fn profile(attempt: &str) -> Result<ExecProfile, String> {
    let env =
        crate::env::load().map_err(|_| "its pane's Hide settings are unreadable".to_owned())?;
    let answer = crate::workspace_cli::report_agent_exec(&env, attempt, AgentExecEvent::Profile)?;
    if answer["ok"] != true {
        return Err(refusal(&answer));
    }
    serde_json::from_value(answer["result"].clone())
        .map_err(|_| "its answer cannot be read".to_owned())
}

/// Tells the pane's node `event` of `attempt`. A node that cannot be told
/// leaves the start's result to what the engine reads of this process.
fn report(attempt: &str, event: AgentExecEvent) {
    if let Ok(env) = crate::env::load() {
        let _ = crate::workspace_cli::report_agent_exec(&env, attempt, bounded(event));
    }
}

/// `event` with its failure cut to what a report carries
/// ([`AGENT_EXEC_FAILURE_LIMIT`]), so a long program path never costs the
/// report itself.
fn bounded(event: AgentExecEvent) -> AgentExecEvent {
    let cut = |failure: String| {
        if failure.len() <= AGENT_EXEC_FAILURE_LIMIT {
            return failure;
        }
        let mut end = AGENT_EXEC_FAILURE_LIMIT - '…'.len_utf8();
        while !failure.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &failure[..end])
    };
    match event {
        AgentExecEvent::Refused { failure } => AgentExecEvent::Refused {
            failure: cut(failure),
        },
        AgentExecEvent::Failed { failure } => AgentExecEvent::Failed {
            failure: cut(failure),
        },
        event => event,
    }
}

/// Reports that the program is about to start, and answers whether the
/// engine took it: why not otherwise, its refusal or a node that did not
/// answer.
fn ready(attempt: &str) -> Result<(), String> {
    let env =
        crate::env::load().map_err(|_| "its pane's Hide settings are unreadable".to_owned())?;
    let answer = crate::workspace_cli::report_agent_exec(&env, attempt, AgentExecEvent::Ready)?;
    if answer["ok"] == true {
        Ok(())
    } else {
        Err(refusal(&answer))
    }
}

/// A refused answer's reason, bounded for the pane's line.
fn refusal(answer: &serde_json::Value) -> String {
    answer["reason"]
        .as_str()
        .unwrap_or("refused")
        .chars()
        .take(64)
        .collect()
}

/// Windows: reports the program's process, now in this process's job and
/// resumed, so the engine can read that it runs.
fn spawned(attempt: &str, child: u32) {
    if let Ok(started) = hide_platform::process::start_time(child) {
        report(
            attempt,
            AgentExecEvent::Spawned {
                child: ExecProcess {
                    pid: child,
                    started,
                },
            },
        );
    }
}

/// The agent's own words after `--`, the program first.
fn parse(arguments: &[OsString]) -> Result<Vec<OsString>, String> {
    match arguments.split_first() {
        Some((first, argv)) if first == "--" => match argv.first() {
            Some(program) if !program.is_empty() => Ok(argv.to_vec()),
            _ => Err("it names no program after --".to_owned()),
        },
        Some((word, _)) => Err(format!("{} is not an option", word.to_string_lossy())),
        None => Err("it names no program after --".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATTEMPT: &str = "9e4eedfa8bf06be0";

    fn words(line: &[&str]) -> Vec<OsString> {
        line.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_start_line_names_its_attempt_and_the_agents_words_alone() {
        assert_eq!(
            parse(&words(&["--", "claude", "--resume", "--"])).unwrap(),
            words(&["claude", "--resume", "--"])
        );
        for (line, why) in [
            (&[][..], "it names no program after --"),
            (&["--"][..], "it names no program after --"),
            (&["--", ""][..], "it names no program after --"),
            (
                &["--profile", "p", "--", "a"][..],
                "--profile is not an option",
            ),
            (&["--env", "A=b", "--", "a"][..], "--env is not an option"),
        ] {
            assert_eq!(parse(&words(line)).unwrap_err(), why, "{line:?}");
        }
    }

    #[test]
    fn only_agent_exec_is_taken() {
        assert_eq!(run(&words(&["hide", "agent", "list"])), None);
        assert_eq!(run(&words(&["hide", "exec"])), None);
        assert_eq!(run(&words(&["hide"])), None);
        assert_eq!(run(&words(&["hide", "agent", "exec"])), Some(MISUSED));
    }

    #[test]
    fn a_start_line_names_its_attempt_first_in_this_builds_shape() {
        assert_eq!(
            attempt_of(&words(&["--attempt", ATTEMPT, "--", "claude"])),
            Ok(ATTEMPT)
        );
        for line in [
            &["--", "claude", "--attempt", ATTEMPT][..],
            &["--attempt"][..],
            &["--attempt", "not-an-attempt"][..],
            // An earlier build's line: refused before anything is asked.
            &[
                "--attempt",
                "1794175536904-d976a9485d217ccbe6a55d4704337678",
                "--profile",
                "Claude · b",
            ][..],
        ] {
            assert!(attempt_of(&words(line)).is_err(), "{line:?}");
        }
        assert_eq!(
            run(&words(&["hide", "agent", "exec", "--", "a"])),
            Some(MISUSED)
        );
    }

    #[test]
    fn a_long_failure_is_cut_to_what_a_report_carries() {
        let long = "é".repeat(AGENT_EXEC_FAILURE_LIMIT);
        let AgentExecEvent::Refused { failure } =
            bounded(AgentExecEvent::Refused { failure: long })
        else {
            unreachable!()
        };
        assert!(failure.len() <= AGENT_EXEC_FAILURE_LIMIT);
        assert!(failure.ends_with('…'));
        let short = AgentExecEvent::Failed {
            failure: "x".to_owned(),
        };
        assert_eq!(bounded(short.clone()), short);
    }
}
