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
//! agent's own `<program>`, which still names it in a failure. A program
//! that cannot start ends the start here, with a line in the pane that names
//! the profile and the program, and the exit code a shell gives a command it
//! could not find (127) or run (126).
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
    let mut command = Command::new(program);
    command.args(&start.leading).args(rest);
    for (name, value) in &start.env {
        command.env(name, value);
    }
    Some(match hide_platform::process::hand_over(command) {
        Ok(code) => code,
        Err(error) => {
            let shown = program.to_string_lossy();
            eprintln!(
                "hide: {} could not start {shown}: {error}. This pane is a shell again.",
                start.profile
            );
            if error.kind() == std::io::ErrorKind::NotFound {
                NOT_FOUND
            } else {
                NOT_RUN
            }
        }
    })
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
