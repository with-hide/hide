//! `hide agent exec`, the command a pane's shell runs to start an agent
//! with a profile (PRD agent-profiles D-26; docs/ARCHITECTURE.md, Starting
//! an agent with a profile). It is not in the help: the core types it, with
//! the absolute path of the `hide` beside the pane's node, as the start line
//! of a profile's agent.
//!
//! ```text
//! hide agent exec --attempt <id> --profile <label> [--env NAME=VALUE]... [--program <file> [--leading <word>]...] -- <program> <argument>...
//! ```
//!
//! It adds the profile's variables to its own environment and runs the
//! agent's program in its place (`hide_platform::process::hand_over`), so the
//! variables reach that program alone and never the shell that typed the
//! line. `--program` runs the profile's program (its wrapper), with the
//! words `--leading` names before the agent's arguments, instead of the
//! agent's own `<program>`, which still names it in a failure.
//!
//! `--attempt` names the start the engine typed this line for, always the
//! first words after `agent exec`, so a bounded read of this process's
//! arguments names it (`hide_agent_hooks::hook_request::is_attempt_executor`).
//! It tells the pane's node of that start (`HookCommand::AgentExec`), and the
//! node takes the report only from this process as its kernel names it:
//! it checks the program as the system will start it
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

use hide_agent_hooks::hook_request::{AgentExecEvent, ExecProcess};

/// The exit code of a start whose program was not found.
const NOT_FOUND: i32 = 127;
/// The exit code of a start whose program could not run.
const NOT_RUN: i32 = 126;
/// The exit code of a start line this command could not read.
const MISUSED: i32 = 2;

/// What a start line asks for.
#[derive(Debug, PartialEq, Eq)]
struct Start {
    profile: String,
    env: Vec<(OsString, OsString)>,
    program: Option<OsString>,
    leading: Vec<OsString>,
    argv: Vec<OsString>,
}

/// Runs the start `arguments` (this process's own, its name first) ask for
/// and returns the exit code to end with, or `None` when they are not
/// `agent exec`. On Unix a start that succeeds never returns.
pub fn run(arguments: &[OsString]) -> Option<i32> {
    if arguments.get(1).map(OsString::as_os_str) != Some(OsStr::new("agent"))
        || arguments.get(2).map(OsString::as_os_str) != Some(OsStr::new("exec"))
    {
        return None;
    }
    let attempt = match attempt_of(&arguments[3..]) {
        Ok(attempt) => attempt,
        Err(why) => {
            eprintln!("hide: this agent start line cannot be read: {why}");
            return Some(MISUSED);
        }
    };
    let start = match parse(&arguments[5..]) {
        Ok(start) => start,
        Err(why) => {
            eprintln!("hide: this agent start line cannot be read: {why}");
            return Some(MISUSED);
        }
    };
    let (program, rest) = match &start.program {
        Some(program) => (program, &start.argv[1..]),
        None => (&start.argv[0], &start.argv[1..]),
    };
    let failure = |why: &dyn std::fmt::Display| {
        format!(
            "{} could not start {}: {why}",
            start.profile,
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
        .map(|(_, value)| value.clone())
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    if let Err((why, code)) = startable(program, &search) {
        let failure = failure(&why);
        report(
            attempt,
            AgentExecEvent::Refused {
                failure: failure.clone(),
            },
        );
        return Some(ended(&failure, code));
    }
    // The program starts only once the engine has taken this start: one it
    // has already answered for is not run late.
    if let Err(why) = ready(attempt) {
        return Some(ended(
            &failure(&format_args!("Hide did not take its start ({why})")),
            NOT_RUN,
        ));
    }
    let mut command = Command::new(program);
    command.args(&start.leading).args(rest);
    for (name, value) in &start.env {
        command.env(name, value);
    }
    Some(
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
        },
    )
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

/// Tells the pane's node `event` of `attempt`. A node that cannot be told
/// leaves the start's result to what the engine reads of this process.
fn report(attempt: &str, event: AgentExecEvent) {
    if let Ok(env) = crate::env::load() {
        let _ = crate::workspace_cli::report_agent_exec(&env, attempt, event);
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
        Err(answer["reason"]
            .as_str()
            .unwrap_or("refused")
            .chars()
            .take(64)
            .collect())
    }
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

fn parse(arguments: &[OsString]) -> Result<Start, String> {
    let mut profile = None;
    let mut env = Vec::new();
    let mut program = None;
    let mut leading = Vec::new();
    let mut words = arguments.iter();
    let argv = loop {
        let Some(word) = words.next() else {
            return Err("it names no program after --".to_owned());
        };
        let mut value = |flag: &str| {
            words
                .next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match word.to_str() {
            Some("--") => break words.cloned().collect::<Vec<_>>(),
            Some("--profile") => {
                profile = Some(value("--profile")?.to_string_lossy().into_owned());
            }
            Some("--env") => {
                let pair = value("--env")?;
                let text = pair.to_str().ok_or("an --env pair is not text")?.to_owned();
                let (name, value) = text
                    .split_once('=')
                    .filter(|(name, _)| !name.is_empty())
                    .ok_or("an --env pair is not NAME=VALUE")?;
                env.push((OsString::from(name), OsString::from(value)));
            }
            Some("--program") => program = Some(value("--program")?),
            Some("--leading") => leading.push(value("--leading")?),
            _ => return Err(format!("{} is not an option", word.to_string_lossy())),
        }
    };
    if argv.is_empty() || argv[0].is_empty() {
        return Err("it names no program after --".to_owned());
    }
    if program.is_none() && !leading.is_empty() {
        return Err("--leading goes with --program".to_owned());
    }
    Ok(Start {
        profile: profile.ok_or("it names no --profile")?,
        env,
        program,
        leading,
        argv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &[&str]) -> Vec<OsString> {
        line.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_start_line_reads_its_profile_variables_program_and_the_agents_words() {
        let start = parse(&words(&[
            "--profile",
            "Claude Code · work",
            "--env",
            "CLAUDE_CONFIG_DIR=/a/b=c/",
            "--env",
            "EMPTY=",
            "--program",
            "/bin/gw",
            "--leading",
            "--fast",
            "--",
            "claude",
            "--resume",
            "--",
        ]))
        .unwrap();
        assert_eq!(
            start,
            Start {
                profile: "Claude Code · work".to_owned(),
                env: vec![
                    ("CLAUDE_CONFIG_DIR".into(), "/a/b=c/".into()),
                    ("EMPTY".into(), "".into()),
                ],
                program: Some("/bin/gw".into()),
                leading: words(&["--fast"]),
                argv: words(&["claude", "--resume", "--"]),
            }
        );
    }

    #[test]
    fn a_start_line_without_its_profile_program_or_pair_is_refused() {
        for (line, why) in [
            (&["--", "claude"][..], "it names no --profile"),
            (&["--profile", "p"][..], "it names no program after --"),
            (
                &["--profile", "p", "--"][..],
                "it names no program after --",
            ),
            (
                &["--profile", "p", "--env", "=x", "--", "a"][..],
                "an --env pair is not NAME=VALUE",
            ),
            (
                &["--profile", "p", "--env", "NAME", "--", "a"][..],
                "an --env pair is not NAME=VALUE",
            ),
            (
                &["--profile", "p", "--leading", "x", "--", "a"][..],
                "--leading goes with --program",
            ),
            (
                &["--profile", "p", "--pane", "x", "--", "a"][..],
                "--pane is not an option",
            ),
            (&["--profile"][..], "--profile needs a value"),
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
    fn a_start_line_names_its_attempt_first() {
        let attempt = "1794175536904-d976a9485d217ccbe6a55d4704337678";
        assert_eq!(
            attempt_of(&words(&["--attempt", attempt, "--profile", "p"])),
            Ok(attempt)
        );
        for line in [
            &["--profile", "p", "--attempt", attempt][..],
            &["--attempt"][..],
            &["--attempt", "not-an-attempt"][..],
        ] {
            assert!(attempt_of(&words(line)).is_err(), "{line:?}");
        }
        assert_eq!(
            run(&words(&[
                "hide",
                "agent",
                "exec",
                "--profile",
                "p",
                "--",
                "a"
            ])),
            Some(MISUSED)
        );
    }
}
