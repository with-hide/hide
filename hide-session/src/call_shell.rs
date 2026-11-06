//! Which shell an agent's shell tool runs a command in, so a call is read in
//! that shell's syntax (`hide_shell::Syntax`): by the hooks before it runs
//! and by the session readers after a session file recorded it. The tool
//! decides it, not the shell of the pane the agent runs in, since an agent
//! starts a shell of its own for each call. This is the one place it is
//! decided; a reader reads the call in the syntax it is handed and never
//! guesses from the text (docs/agent-hooks.md, Which shell reads a call).

use hide_agent_adapter::HookDialect;
use hide_shell::Syntax;

/// The syntax of the shell `dialect`'s tool `tool` runs a command in on
/// this system. A session file is read on the machine whose agent wrote it,
/// so this system is the session's too.
pub fn syntax(dialect: HookDialect, tool: &str) -> Syntax {
    syntax_on(dialect, tool, cfg!(windows))
}

/// [`syntax`] on Windows (`windows`) or on macOS and Linux.
pub fn syntax_on(dialect: HookDialect, tool: &str, windows: bool) -> Syntax {
    match dialect {
        // Claude Code names the shell in the tool: `Bash` runs in Git Bash
        // on Windows (and is not registered where Git Bash is missing), and
        // `PowerShell` in PowerShell on every system (tools reference,
        // "PowerShell tool").
        HookDialect::ClaudeCode if tool == "PowerShell" => Syntax::PowerShell,
        HookDialect::ClaudeCode => Syntax::Posix,
        // Codex runs every command in the session's shell, PowerShell on
        // Windows (codex-rs shell_detect `default_user_shell`); Grok in pwsh or
        // Windows PowerShell; Cursor and agy are read the same way on
        // Windows, unconfirmed by a capture (docs/INSTALL.md).
        HookDialect::Codex | HookDialect::Grok | HookDialect::Cursor | HookDialect::Antigravity
            if windows =>
        {
            Syntax::PowerShell
        }
        // OpenCode's, Pi's and omp's bash tool runs a POSIX shell, Git Bash on
        // Windows.
        _ => Syntax::Posix,
    }
}

/// The name a syntax is marked with where a call travels between machines
/// (a thread's `_meta.hide.shell`).
pub fn name(syntax: Syntax) -> &'static str {
    match syntax {
        Syntax::Posix => "posix",
        Syntax::PowerShell => "powershell",
    }
}

/// The syntax a [`name`] marks; `None` for a name this build does not know.
pub fn from_name(name: &str) -> Option<Syntax> {
    match name {
        "posix" => Some(Syntax::Posix),
        "powershell" => Some(Syntax::PowerShell),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which shell each runtime's shell tool runs a command in, per system:
    /// Claude Code's tools reference (Bash in Git Bash, PowerShell tool),
    /// Codex's shell detection (PowerShell on Windows), Grok's hooks guide
    /// (pwsh or Windows PowerShell), and Cursor's and agy's read the same way.
    #[test]
    fn the_shell_syntax_is_the_one_the_tool_runs_on_its_system() {
        use HookDialect::*;
        let table = [
            (ClaudeCode, "Bash", Syntax::Posix, Syntax::Posix),
            (
                ClaudeCode,
                "PowerShell",
                Syntax::PowerShell,
                Syntax::PowerShell,
            ),
            (Codex, "Bash", Syntax::Posix, Syntax::PowerShell),
            (Codex, "exec_command", Syntax::Posix, Syntax::PowerShell),
            (
                Grok,
                "run_terminal_command",
                Syntax::Posix,
                Syntax::PowerShell,
            ),
            (Grok, "Bash", Syntax::Posix, Syntax::PowerShell),
            (Cursor, "Shell", Syntax::Posix, Syntax::PowerShell),
            (
                Antigravity,
                "run_command",
                Syntax::Posix,
                Syntax::PowerShell,
            ),
            (OpenCode, "bash", Syntax::Posix, Syntax::Posix),
            (Pi, "bash", Syntax::Posix, Syntax::Posix),
            (Omp, "bash", Syntax::Posix, Syntax::Posix),
        ];
        for (dialect, tool, elsewhere, windows) in table {
            assert_eq!(
                syntax_on(dialect, tool, false),
                elsewhere,
                "{dialect:?} {tool}"
            );
            assert_eq!(
                syntax_on(dialect, tool, true),
                windows,
                "{dialect:?} {tool} on Windows"
            );
            assert_eq!(
                syntax(dialect, tool),
                if cfg!(windows) { windows } else { elsewhere }
            );
        }
    }
}
