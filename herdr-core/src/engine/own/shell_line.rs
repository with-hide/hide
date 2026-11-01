// Derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit 2ecf19a):
// `shell_quote` of src/platform/macos.rs and src/platform/linux.rs,
// `interactive_unix_shell_command` and `quote_powershell_arg` of
// src/platform/mod.rs, `powershell_agent_script` and
// `quote_windows_command_line_arg` of src/platform/windows.rs,
// `validate_resume_argv` of src/agent_resume.rs, and
// `stripped_terminal_title` of src/terminal/title.rs.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: one module for the line the own engine types into a
// pane's shell; the shell is told by its name, PowerShell's or a POSIX
// shell's, with no cmd.exe form, since the own engine's panes run one of
// those; a resume line can say, when the agent ended with a failure, how it
// ended; the Windows title prefix is dropped on every system.

//! The line the core's own engine types into a pane's shell to start an
//! agent (PRD engine-bundle-3-core-owns D-4, D-16): the agent's program and
//! its arguments, each quoted for the shell that reads it, as Herdr's
//! `agent.start` types it.

/// The shell a line is typed into.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Shell {
    Posix,
    PowerShell,
}

impl Shell {
    /// The shell a pane's program name says, or this system's own when the
    /// name is unknown: the own engine starts PowerShell on Windows and a
    /// POSIX shell elsewhere.
    pub(super) fn of(name: Option<&str>) -> Self {
        let powershell = |name: &str| {
            let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
            let name = name.trim_start_matches('-').to_ascii_lowercase();
            let name = name.strip_suffix(".exe").unwrap_or(&name);
            matches!(name, "pwsh" | "powershell")
        };
        match name {
            Some(name) if powershell(name) => Self::PowerShell,
            Some(_) => Self::Posix,
            None if cfg!(windows) => Self::PowerShell,
            None => Self::Posix,
        }
    }
}

/// `argv` as one command line `shell` reads, with no Enter.
pub(super) fn command(argv: &[String], shell: Shell) -> Option<String> {
    let (program, args) = argv.split_first()?;
    Some(match shell {
        Shell::Posix => std::iter::once(program)
            .chain(args)
            .map(|word| posix_quote(word))
            .collect::<Vec<_>>()
            .join(" "),
        Shell::PowerShell => powershell_script(program, args),
    })
}

/// A resume of `argv` typed into `shell`, which says, when the agent ends
/// with a failure, that this pane is a shell now and how the agent ended:
/// true both of a resume that could not open the conversation and of an
/// agent that ran and then ended with a failure (PRD D-7, B13). An agent that
/// ends cleanly prints nothing.
pub(super) fn resume_command(argv: &[String], agent: &str, shell: Shell) -> Option<String> {
    let (program, args) = argv.split_first()?;
    let agent: String = agent
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
        .collect();
    Some(match shell {
        Shell::Posix => format!(
            "{} || printf 'hide: %s ended with exit code %s after its resume; this pane is a shell now.\\n' {} \"$?\"",
            command(argv, shell)?,
            posix_quote(&agent)
        ),
        // `Start-Process` itself succeeds whatever its program's exit code,
        // so the code is read from the process it hands back.
        Shell::PowerShell => {
            let (script, process) = powershell_parts(program, args);
            format!(
                // The name is a string literal here, never a bare word,
                // which an expression would run as a command; the filter
                // above leaves no apostrophe in it.
                "if({script}){{& {program} {powershell_args}; $hideExit = $LASTEXITCODE}}else{{$hideExit = ({process} -PassThru).ExitCode}}; if ($hideExit) {{ Write-Host ('hide: {agent} ended with exit code ' + $hideExit + ' after its resume; this pane is a shell now.') }}",
                program = powershell_quote(program),
                powershell_args = powershell_args(args),
            )
        }
    })
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
    format!("'{}'", value.replace('\'', "'\\''"))
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
    format!("'{}'", value.replace('\'', "''"))
}

/// A Windows command line argument, for `Start-Process -ArgumentList`.
fn windows_quote(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|ch| matches!(ch, ' ' | '\t' | '\n' | '\x0b' | '"'))
    {
        return value.to_owned();
    }
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }
        if ch == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
        } else {
            quoted.push_str(&"\\".repeat(backslashes));
        }
        backslashes = 0;
        quoted.push(ch);
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
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
        .map(|arg| windows_quote(arg))
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

/// The most words a resume types.
const MAX_RESUME_ARGS: usize = 64;
/// The most bytes a resume's words hold in all.
const MAX_RESUME_ARGV_BYTES: usize = 8 * 1024;

/// Whether `argv` may be typed as a resume: a plain command name first
/// (shells disagree on how to run a quoted path), no control character and
/// no apostrophe, within Herdr's bounds.
pub(super) fn check_resume(argv: &[String]) -> Result<(), String> {
    let Some(command) = argv.first() else {
        return Err("the resume names no program".to_owned());
    };
    if argv.len() > MAX_RESUME_ARGS {
        return Err(format!("the resume has more than {MAX_RESUME_ARGS} words"));
    }
    if argv.iter().map(String::len).sum::<usize>() > MAX_RESUME_ARGV_BYTES {
        return Err(format!(
            "the resume is longer than {MAX_RESUME_ARGV_BYTES} bytes"
        ));
    }
    if argv.iter().any(|arg| arg.chars().any(char::is_control)) {
        return Err("the resume holds a control character".to_owned());
    }
    if argv.iter().any(|arg| arg.contains('\'')) {
        return Err("the resume holds an apostrophe".to_owned());
    }
    let plain = !command.is_empty()
        && !command.starts_with('-')
        && command
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if !plain {
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
            command(&argv, Shell::Posix).as_deref(),
            Some("pi '' 'two words' 'a'\\''b' '$HOME' 'semi;colon' @options")
        );
        assert_eq!(
            command(&argv, Shell::PowerShell).unwrap(),
            "if((Get-Command pi -ErrorAction SilentlyContinue).CommandType -eq 'ExternalScript'){& pi '' 'two words' 'a''b' '$HOME' 'semi;colon' '@options'}else{Start-Process -FilePath pi -ArgumentList '\"\" \"two words\" a''b $HOME semi;colon @options' -NoNewWindow -Wait}"
        );
        assert_eq!(
            command(&words(&["claude"]), Shell::PowerShell).unwrap(),
            "& claude"
        );
    }

    #[test]
    fn the_shell_is_told_by_its_name_and_else_by_the_system() {
        assert_eq!(Shell::of(Some("-zsh")), Shell::Posix);
        assert_eq!(Shell::of(Some("pwsh.exe")), Shell::PowerShell);
        assert_eq!(Shell::of(Some("/usr/local/bin/pwsh")), Shell::PowerShell);
        let fallback = if cfg!(windows) {
            Shell::PowerShell
        } else {
            Shell::Posix
        };
        assert_eq!(Shell::of(None), fallback);
    }

    #[test]
    fn a_resume_line_says_how_the_agent_ended_only_when_it_failed() {
        let line = resume_command(
            &words(&["claude", "--resume", "abc"]),
            "Claude Code",
            Shell::Posix,
        )
        .unwrap();
        assert_eq!(
            line,
            "claude --resume abc || printf 'hide: %s ended with exit code %s after its resume; this pane is a shell now.\\n' 'Claude Code' \"$?\""
        );
        let line = resume_command(
            &words(&["claude", "--resume", "abc"]),
            "it's",
            Shell::PowerShell,
        )
        .unwrap();
        assert_eq!(
            line,
            "if((Get-Command claude -ErrorAction SilentlyContinue).CommandType -eq 'ExternalScript'){& claude '--resume' abc; $hideExit = $LASTEXITCODE}else{$hideExit = (Start-Process -FilePath claude -ArgumentList '--resume abc' -NoNewWindow -Wait -PassThru).ExitCode}; if ($hideExit) { Write-Host ('hide: its ended with exit code ' + $hideExit + ' after its resume; this pane is a shell now.') }"
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
        let many = vec!["x".to_owned(); MAX_RESUME_ARGS + 1];
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
