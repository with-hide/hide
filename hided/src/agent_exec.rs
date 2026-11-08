//! `hide agent exec`, the command a pane's shell runs to start an agent
//! with a profile (PRD agent-profiles D-26; docs/ARCHITECTURE.md, Starting
//! an agent with a profile). It is not in the help: the core types it, with
//! the absolute path of the `hide` beside the pane's node, as the start line
//! of a profile's agent.
//!
//! ```text
//! hide agent exec --profile <label> [--env NAME=VALUE]... [--program <file> [--leading <word>]...] -- <program> <argument>...
//! ```
//!
//! It adds the profile's variables to its own environment and runs the
//! agent's program in its place (`hide_platform::process::hand_over`), so the
//! variables reach that program alone and never the shell that typed the
//! line. `--program` runs the profile's program (its wrapper), with the
//! words `--leading` names before the agent's arguments, instead of the
//! agent's own `<program>`, which still names it in a failure.
//!
//! It tells the pane's node what came of the start (`HookCommand::AgentExec`),
//! which the start that typed the line waits for (PRD agent-profiles B22): it
//! checks the program as the system will start it
//! (`hide_platform::programs::runnable`) and reports the hand-over as it
//! happens, or why the program cannot start. A program that cannot start
//! ends the start here too, with a line in the pane that names the profile
//! and the program, and the exit code a shell gives a command it could not
//! find (127) or run (126).
//!
//! It runs before anything else in `hide` (`bin/hide.rs`): before the pane's
//! own installation is handed the call (`pane_install::hand_off`), which
//! would leave a second process between the shell and the agent.

use std::ffi::{OsStr, OsString};
use std::process::Command;

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
    let start = match parse(&arguments[3..]) {
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
    let failed = |why: &dyn std::fmt::Display, code| {
        let failure = format!(
            "{} could not start {}: {why}",
            start.profile,
            program.to_string_lossy()
        );
        report(Some(failure.clone()));
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
        return Some(failed(&why, code));
    }
    let mut command = Command::new(program);
    command.args(&start.leading).args(rest);
    for (name, value) in &start.env {
        command.env(name, value);
    }
    Some(
        match hide_platform::process::hand_over(command, || report(None)) {
            Ok(code) => code,
            Err(error) => {
                let code = if error.kind() == std::io::ErrorKind::NotFound {
                    NOT_FOUND
                } else {
                    NOT_RUN
                };
                failed(&error, code)
            }
        },
    )
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

/// Tells the pane's node what came of the start (`None`: the program has
/// the pane now). A node that cannot be told leaves the start to its wait.
fn report(failure: Option<String>) {
    if let Ok(env) = crate::env::load() {
        let _ = crate::workspace_cli::report_agent_exec(&env, failure);
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
}
