// Derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit 2ecf19a):
// `shell_quote` of src/platform/macos.rs and src/platform/linux.rs,
// `interactive_unix_shell_command` and `quote_powershell_arg` of
// src/platform/mod.rs, `powershell_agent_script` of
// src/platform/windows.rs, `validate_resume_argv` of src/agent_resume.rs, and
// `stripped_terminal_title` of src/terminal/title.rs.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: one module for the line the own engine types into a
// pane's shell; the shell is told by its name, PowerShell's or a POSIX
// shell's, and cmd.exe is given no line, since the own engine's panes run
// PowerShell on Windows; a resume line says, when the agent ended with a
// failure, that it did, in a form zsh, bash, fish and PowerShell all read;
// the Windows title prefix is dropped on every system; the Windows command
// line's quoting moved to hide-shell's `quote::windows_argument`.

//! The line the core's own engine types into a pane's shell to start an
//! agent (PRD engine-bundle-3-core-owns D-4, D-16): the agent's program and
//! its arguments, each quoted for the shell that reads it, as Herdr's
//! `agent.start` types it.

use hide_shell::Syntax;

/// The shell a pane's program name says, or, when the name is unknown or
/// could not be read, the own engine's shell on the pane's machine:
/// PowerShell when `windows` (the machine whose holder runs the pane, a
/// device's system and not the core's) and a POSIX shell elsewhere. cmd.exe
/// reads neither line, so it has none: a pane a caller made run it takes no
/// start.
pub(super) fn shell_of(name: Option<&str>, windows: bool) -> Option<Syntax> {
    match name.filter(|name| !name.is_empty()) {
        Some(name) => Syntax::of_program(name),
        None if windows => Some(Syntax::PowerShell),
        None => Some(Syntax::Posix),
    }
}

/// `argv` as one command line `shell` reads, with no Enter.
pub(super) fn command(argv: &[String], shell: Syntax) -> Option<String> {
    let (program, args) = argv.split_first()?;
    Some(match shell {
        Syntax::Posix => std::iter::once(program)
            .chain(args)
            .map(|word| posix_quote(word))
            .collect::<Vec<_>>()
            .join(" "),
        Syntax::PowerShell => powershell_script(program, args),
    })
}

/// A resume of `argv` typed into `shell`, which says, when the agent ends
/// with a failure, that it did and that this pane is a shell now: true both
/// of a resume that could not open the conversation and of an agent that ran
/// and then ended with a failure (PRD D-7, B13). An agent that ends cleanly
/// prints nothing. The POSIX line names no exit code, which zsh and bash
/// spell `$?` and fish refuses to read, so one line serves all three.
pub(super) fn resume_command(argv: &[String], agent: &str, shell: Syntax) -> Option<String> {
    let (program, args) = argv.split_first()?;
    let agent: String = agent
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
        .collect();
    Some(match shell {
        Syntax::Posix => format!(
            "{} || printf 'hide: %s ended with a failure after its resume; this pane is a shell now.\\n' {}",
            command(argv, shell)?,
            posix_quote(&agent)
        ),
        // `Start-Process` itself succeeds whatever its program's exit code,
        // so the code is read from the process it hands back.
        Syntax::PowerShell => {
            let (script, process) = powershell_parts(program, args);
            format!(
                // The name is a string literal here, never a bare word,
                // which an expression would run as a command; the filter
                // above leaves no apostrophe in it.
                "if({script}){{& {program} {powershell_args}; $hideExit = $LASTEXITCODE}}else{{$hideExit = ({process} -PassThru).ExitCode}}; if ($hideExit) {{ Write-Host 'hide: {agent} ended with a failure after its resume; this pane is a shell now.' }}",
                program = powershell_quote(program),
                powershell_args = powershell_args(args),
            )
        }
    })
}

/// The longest of the lines `line` writes for each shell a pane may run,
/// Enter included: a start or a resume is bounded as the shell it meets will
/// type it. `None` when no shell takes it.
pub(super) fn longest(line: impl Fn(Syntax) -> Option<String>) -> Option<usize> {
    [Syntax::Posix, Syntax::PowerShell]
        .into_iter()
        .filter_map(line)
        .map(|line| line.len() + 1)
        .max()
}

fn posix_quote(value: &str) -> String {
    if !value.is_empty()
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    '@' | '%' | '_' | '+' | '=' | ':' | ',' | '.' | '/' | '-'
                )
        })
    {
        return value.to_owned();
    }
    Syntax::Posix.single_quoted(value)
}

fn powershell_quote(value: &str) -> String {
    if !value.is_empty()
        && !value.starts_with('-')
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'+' | b'=')
        })
    {
        return value.to_owned();
    }
    Syntax::PowerShell.single_quoted(value)
}

/// Herdr's PowerShell start: a script the program names runs in the shell,
/// anything else as one process on the pane's console, its arguments handed
/// over as one Windows command line, which PowerShell 5.1's own passing of
/// native arguments would mangle.
fn powershell_script(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        return format!("& {}", powershell_quote(program));
    }
    let (script, process) = powershell_parts(program, args);
    format!(
        "if({script}){{& {} {}}}else{{{process}}}",
        powershell_quote(program),
        powershell_args(args),
    )
}

/// The test whether `program` is a PowerShell script, and the process start
/// of anything else.
fn powershell_parts(program: &str, args: &[String]) -> (String, String) {
    let command_line = args
        .iter()
        .map(|arg| hide_shell::quote::windows_argument(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let program = powershell_quote(program);
    let process = if args.is_empty() {
        format!("Start-Process -FilePath {program} -NoNewWindow -Wait")
    } else {
        format!(
            "Start-Process -FilePath {program} -ArgumentList {} -NoNewWindow -Wait",
            powershell_quote(&command_line)
        )
    };
    (
        format!(
            "(Get-Command {program} -ErrorAction SilentlyContinue).CommandType -eq 'ExternalScript'"
        ),
        process,
    )
}

fn powershell_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| powershell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `argv` may be typed as a resume: a plain command name first, no
/// control character and no apostrophe, within Herdr's bounds; the rules a
/// definition's words are read with (`hide_agent_adapter::definition`).
pub(super) fn check_resume(argv: &[String]) -> Result<(), String> {
    use hide_agent_adapter::definition::{
        LINE_BYTES, LINE_WORDS, is_plain_command, is_typable_word,
    };
    let Some(command) = argv.first() else {
        return Err("the resume names no program".to_owned());
    };
    if argv.len() > LINE_WORDS {
        return Err(format!("the resume has more than {LINE_WORDS} words"));
    }
    if argv.iter().map(String::len).sum::<usize>() > LINE_BYTES {
        return Err(format!("the resume is longer than {LINE_BYTES} bytes"));
    }
    if let Some(word) = argv.iter().find(|arg| !is_typable_word(arg)) {
        return Err(if word.contains('\'') {
            "the resume holds an apostrophe".to_owned()
        } else {
            "the resume holds a control character".to_owned()
        });
    }
    if !is_plain_command(command) {
        return Err("the resume starts with a path, not a program's name".to_owned());
    }
    Ok(())
}

/// The glyphs Claude Code puts before its title while it works.
const CLAUDE_ACTIVITY_GLYPHS: &str = "·✢✳✶✻✽◐◓◑◒";

/// A terminal title without the one activity glyph an agent puts before it,
/// as Herdr strips it; `None` for an empty title.
pub(super) fn stripped_title(title: &str) -> Option<String> {
    let title = title
        .strip_prefix("Administrator: ")
        .unwrap_or(title)
        .trim();
    let first = title.chars().next()?;
    let after_first = &title[first.len_utf8()..];
    let recognized =
        matches!(first, '\u{2800}'..='\u{28ff}') || CLAUDE_ACTIVITY_GLYPHS.contains(first);
    let stripped = if recognized
        && (after_first.is_empty() || after_first.chars().next().is_some_and(char::is_whitespace))
    {
        after_first.trim()
    } else {
        title
    };
    (!stripped.is_empty()).then(|| stripped.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn a_line_quotes_each_word_for_a_posix_shell_and_for_powershell() {
        let argv = words(&[
            "pi",
            "",
            "two words",
            "a'b",
            "$HOME",
            "semi;colon",
            "@options",
        ]);
        assert_eq!(
            command(&argv, Syntax::Posix).as_deref(),
            Some("pi '' 'two words' 'a'\\''b' '$HOME' 'semi;colon' @options")
        );
        assert_eq!(
            command(&argv, Syntax::PowerShell).unwrap(),
            "if((Get-Command pi -ErrorAction SilentlyContinue).CommandType -eq 'ExternalScript'){& pi '' 'two words' 'a''b' '$HOME' 'semi;colon' '@options'}else{Start-Process -FilePath pi -ArgumentList '\"\" \"two words\" a''b $HOME semi;colon @options' -NoNewWindow -Wait}"
        );
        assert_eq!(
            command(&words(&["claude"]), Syntax::PowerShell).unwrap(),
            "& claude"
        );
    }

    #[test]
    fn the_shell_is_told_by_its_name_and_else_by_the_panes_machine() {
        for windows in [false, true] {
            assert_eq!(shell_of(Some("-zsh"), windows), Some(Syntax::Posix));
            assert_eq!(shell_of(Some("fish"), windows), Some(Syntax::Posix));
            assert_eq!(
                shell_of(Some("pwsh.exe"), windows),
                Some(Syntax::PowerShell)
            );
            assert_eq!(
                shell_of(Some("/usr/local/bin/pwsh"), windows),
                Some(Syntax::PowerShell)
            );
            assert_eq!(
                shell_of(Some("C:\\Windows\\System32\\cmd.exe"), windows),
                None
            );
        }
        // A name not known, or one the holder could not read (an empty
        // name), is the pane's machine's shell, whatever the core runs on:
        // a Mac core types PowerShell's line into a Windows device's pane.
        for name in [None, Some("")] {
            assert_eq!(shell_of(name, true), Some(Syntax::PowerShell));
            assert_eq!(shell_of(name, false), Some(Syntax::Posix));
        }
    }

    #[test]
    fn a_resume_line_says_the_agent_failed_only_when_it_did() {
        let line = resume_command(
            &words(&["claude", "--resume", "abc"]),
            "Claude Code",
            Syntax::Posix,
        )
        .unwrap();
        assert_eq!(
            line,
            "claude --resume abc || printf 'hide: %s ended with a failure after its resume; this pane is a shell now.\\n' 'Claude Code'"
        );
        let line = resume_command(
            &words(&["claude", "--resume", "abc"]),
            "it's",
            Syntax::PowerShell,
        )
        .unwrap();
        assert_eq!(
            line,
            "if((Get-Command claude -ErrorAction SilentlyContinue).CommandType -eq 'ExternalScript'){& claude '--resume' abc; $hideExit = $LASTEXITCODE}else{$hideExit = (Start-Process -FilePath claude -ArgumentList '--resume abc' -NoNewWindow -Wait -PassThru).ExitCode}; if ($hideExit) { Write-Host 'hide: its ended with a failure after its resume; this pane is a shell now.' }"
        );
    }

    #[test]
    fn a_resume_must_start_with_a_plain_program_within_bounds() {
        assert!(check_resume(&words(&["claude", "--resume", "id-1"])).is_ok());
        for refused in [
            words(&[]),
            words(&["/usr/bin/claude"]),
            words(&["-x"]),
            words(&["claude", "a'b"]),
            words(&["claude", "line\nbreak"]),
        ] {
            assert!(check_resume(&refused).is_err(), "{refused:?}");
        }
        let many = vec!["x".to_owned(); hide_agent_adapter::definition::LINE_WORDS + 1];
        assert!(check_resume(&many).is_err());
    }

    #[test]
    fn a_title_loses_one_leading_activity_glyph_only() {
        assert_eq!(stripped_title("⠋ task").as_deref(), Some("task"));
        assert_eq!(stripped_title("◐ task").as_deref(), Some("task"));
        assert_eq!(stripped_title("✳task").as_deref(), Some("✳task"));
        assert_eq!(stripped_title("  plain  ").as_deref(), Some("plain"));
        assert_eq!(stripped_title("⠋"), None);
        assert_eq!(stripped_title(""), None);
    }
}

/// The resume line run by the shells themselves, which only they can say
/// they read: fish refuses a line zsh and bash take (`$?`), and a pane whose
/// line its shell refuses is never resumed. They need the shells, named in
/// `HIDE_TEST_ZSH_BIN`, `HIDE_TEST_BASH_BIN` and `HIDE_TEST_FISH_BIN`, so they
/// are ignored by default; the rust lane's shell step runs them with
/// `--ignored`.
#[cfg(all(test, unix))]
mod real_shell_tests {
    use std::process::{Command, Stdio};

    use super::*;

    /// What the shell named in `variable` writes running `argv`'s resume.
    #[allow(clippy::disallowed_methods)] // env_clear: the shell starts with no files and no environment of the test's
    fn resumed(variable: &str, argv: &[&str]) -> String {
        let shell =
            std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} names the shell"));
        let home = tempfile::tempdir().unwrap();
        let argv: Vec<String> = argv.iter().map(|word| (*word).to_owned()).collect();
        let line = resume_command(&argv, "Claude Code", Syntax::Posix).unwrap();
        let output = Command::new(shell)
            .arg("-c")
            .arg(&line)
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.stderr.is_empty(),
            "{variable} refused {line}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    /// An agent that fails says so after what it wrote, with its words
    /// quoted as the shell read them; one that ends cleanly adds nothing.
    fn resume_line_runs_in(variable: &str) {
        assert_eq!(
            resumed(
                variable,
                &["sh", "-c", "printf '%s\\n' \"$0\"; exit 3", "it's a 'word'"]
            ),
            "it's a 'word'\nhide: Claude Code ended with a failure after its resume; this pane is a shell now.\n"
        );
        assert_eq!(resumed(variable, &["sh", "-c", "exit 0"]), "");
    }

    #[test]
    #[ignore = "needs zsh: HIDE_TEST_ZSH_BIN"]
    fn zsh_runs_the_resume_line_and_says_a_failure() {
        resume_line_runs_in("HIDE_TEST_ZSH_BIN");
    }

    #[test]
    #[ignore = "needs bash: HIDE_TEST_BASH_BIN"]
    fn bash_runs_the_resume_line_and_says_a_failure() {
        resume_line_runs_in("HIDE_TEST_BASH_BIN");
    }

    #[test]
    #[ignore = "needs fish: HIDE_TEST_FISH_BIN"]
    fn fish_runs_the_resume_line_and_says_a_failure() {
        resume_line_runs_in("HIDE_TEST_FISH_BIN");
    }
}
