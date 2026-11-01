// Partly derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit
// 2ecf19a), src/detect/mod.rs: `script_arg_agent_name`, `flag_matches`,
// `option_takes_value`, `is_python_runtime` and `path_basename`.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: a runtime's script is named by its file name without its
// extension instead of being matched against Herdr's agent table, deno's
// `run` is passed over, and the shell, cmd and PowerShell forms are not taken.

//! The program in a pane's foreground, as an agent definition's
//! `executables` recognize it (PRD engine-bundle-3-core-owns D-21): the
//! leader of the terminal's foreground group, or its first member once the
//! leader has ended, when the group is not the pane's own program's, named
//! by its file, or by the script a runtime (node, bun, deno, python) runs. The system is asked once per process: a process is
//! known again by its pid and the name the system gives it, which an `exec`
//! changes.

use hide_holder_protocol::{MAX_ARGS, MAX_ARGV_BYTES, Program, Unknown};

/// The last program found, by its pid and the system's name for it.
#[derive(Default)]
pub struct Foreground {
    known: Option<((u32, String), Program)>,
}

impl Foreground {
    /// The program in the foreground of a terminal whose group is `group`,
    /// or none while `shell`, the pane's program, holds it or the system
    /// cannot say.
    ///
    /// It is read each time the pane's facts are, up to 60 times a second
    /// while the program writes, so it asks the system one name in the
    /// usual case: the pane's program leads its own session and group, so
    /// its group is the shell's, and any other group's leader is its
    /// program. Only a group whose leader has ended is listed, which on
    /// Linux reads the whole process table.
    pub fn program(&mut self, shell: u32, group: Result<u32, Unknown>) -> Option<Program> {
        let group = group.ok()?;
        if group == shell {
            return None;
        }
        let key = match hide_platform::process::name_of(group) {
            Ok(name) => (group, name),
            Err(_) => {
                let mut members = hide_platform::process::group_members(group).ok()?;
                if members.contains(&shell) {
                    return None;
                }
                members.sort_unstable();
                let pid = *members.first()?;
                (pid, hide_platform::process::name_of(pid).ok()?)
            }
        };
        let pid = key.0;
        if let Some((known, program)) = &self.known
            && *known == key
        {
            return Some(program.clone());
        }
        // A system that keeps no arguments (Windows) or a process gone
        // since leaves them empty.
        let argv = bounded(hide_platform::process::argv_of(pid).unwrap_or_default());
        let program = Program {
            pid,
            name: program_name(&key.1, &argv),
            argv,
        };
        self.known = Some((key, program.clone()));
        Some(program)
    }
}

/// The first [`MAX_ARGS`] arguments that fit in [`MAX_ARGV_BYTES`].
fn bounded(argv: Vec<String>) -> Vec<String> {
    let mut total = 0;
    argv.into_iter()
        .take(MAX_ARGS)
        .take_while(|arg| {
            total += arg.len();
            total <= MAX_ARGV_BYTES
        })
        .collect()
}

/// The name a program is recognized by: its first argument's file name, or
/// the system's name for it when it has none, without `.exe`; for a runtime
/// that runs a script, the script's file name without its extension.
pub fn program_name(system_name: &str, argv: &[String]) -> String {
    let program = argv
        .first()
        .map(String::as_str)
        .filter(|first| !first.is_empty())
        .unwrap_or(system_name);
    let name = without_exe(path_basename(program));
    let runtime = name.to_lowercase();
    let script = match runtime.as_str() {
        "node" | "bun" => script_arg(argv, &["-e", "--eval", "-p", "--print"], &[], false),
        "deno" => script_arg(argv, &["-e", "--eval"], &[], true),
        runtime if is_python_runtime(runtime) => script_arg(argv, &["-c"], &["-m"], false),
        _ => None,
    };
    match script {
        Some(script) => without_extension(path_basename(script)).to_owned(),
        None => name.to_owned(),
    }
}

/// The script a runtime's arguments name: the first that is not an option
/// (or the one after `--`), none when the runtime runs code given inline
/// or a module. `subcommand` passes over a leading `run`, as deno takes it.
fn script_arg<'a>(
    argv: &'a [String],
    eval_flags: &[&str],
    module_flags: &[&str],
    subcommand: bool,
) -> Option<&'a str> {
    let mut args = argv.iter().skip(1);
    let mut first = true;
    while let Some(arg) = args.next() {
        if arg == "--" {
            return args
                .next()
                .map(String::as_str)
                .filter(|arg| !arg.is_empty());
        }
        if flag_matches(arg, eval_flags) || flag_matches(arg, module_flags) {
            return None;
        }
        if arg.starts_with('-') {
            if option_takes_value(arg) {
                let _ = args.next();
            }
            continue;
        }
        if subcommand && first && arg == "run" {
            first = false;
            continue;
        }
        let token = arg.trim_matches(|c| matches!(c, '"' | '\''));
        return (!token.is_empty()).then_some(token);
    }
    None
}

fn flag_matches(arg: &str, flags: &[&str]) -> bool {
    flags
        .iter()
        .any(|flag| arg == *flag || short_flag_payload(arg, flag) || long_flag_value(arg, flag))
}

fn short_flag_payload(arg: &str, flag: &str) -> bool {
    flag.starts_with('-')
        && !flag.starts_with("--")
        && arg.starts_with(flag)
        && arg.len() > flag.len()
}

fn long_flag_value(arg: &str, flag: &str) -> bool {
    flag.starts_with("--")
        && arg
            .strip_prefix(flag)
            .is_some_and(|rest| rest.starts_with('='))
}

fn option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "-r" | "--require"
            | "--loader"
            | "--import"
            | "--experimental-loader"
            | "--inspect-port"
            | "-W"
            | "-X"
            | "-S"
            | "-L"
            | "-o"
    )
}

fn is_python_runtime(name: &str) -> bool {
    name == "python"
        || name.strip_prefix("python").is_some_and(|version| {
            !version.is_empty()
                && version
                    .split('.')
                    .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
        })
}

fn path_basename(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|component| !component.is_empty())
        .unwrap_or(path)
}

fn without_exe(name: &str) -> &str {
    match name.len().checked_sub(4) {
        Some(cut) if name.is_char_boundary(cut) && name[cut..].eq_ignore_ascii_case(".exe") => {
            &name[..cut]
        }
        _ => name,
    }
}

fn without_extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(system: &str, argv: &[&str]) -> String {
        let argv: Vec<String> = argv.iter().map(|arg| (*arg).to_owned()).collect();
        program_name(system, &argv)
    }

    #[test]
    fn a_program_is_named_by_its_file_and_a_runtime_by_the_script_it_runs() {
        assert_eq!(name("claude", &["/Users/me/.local/bin/claude"]), "claude");
        assert_eq!(name("codex.exe", &[]), "codex");
        assert_eq!(name("CODEX.EXE", &["C:\\bin\\Codex.EXE"]), "Codex");
        assert_eq!(name("node", &["node", "/Users/me/.local/bin/pi"]), "pi");
        assert_eq!(
            name(
                "node",
                &["node", "--require", "x", "/opt/qwen/cli.js", "--tui"]
            ),
            "cli"
        );
        assert_eq!(name("bun", &["bun", "--", "./agent.ts"]), "agent");
        assert_eq!(name("deno", &["deno", "run", "-A", "/x/tool.ts"]), "tool");
        assert_eq!(
            name("python3.12", &["python3.12", "/srv/hermes.py"]),
            "hermes"
        );
        assert_eq!(name("python3", &["python3", "-m", "aider"]), "python3");
        assert_eq!(name("node", &["node", "-e", "console.log(1)"]), "node");
        assert_eq!(name("node", &["node"]), "node");
        assert_eq!(name("pythonx", &["pythonx", "script.py"]), "pythonx");
    }

    #[test]
    fn arguments_keep_the_first_that_fit_their_bounds() {
        let many: Vec<String> = (0..MAX_ARGS + 10).map(|i| i.to_string()).collect();
        assert_eq!(bounded(many).len(), MAX_ARGS);
        let wide = vec!["a".repeat(MAX_ARGV_BYTES - 1), "bb".to_owned()];
        assert_eq!(bounded(wide).len(), 1);
    }
}
