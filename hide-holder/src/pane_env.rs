// Derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr, commit
// 2ecf19a), src/pane.rs: `apply_pane_terminal_env` and the parent-agent
// variables `apply_pane_launch_env` removes. Copyright 2025 herdr
// contributors. Licensed under the Apache License, Version 2.0; see NOTICE at
// the repository root.
// Changed by Hide: the terminal says it is Hide's (`TERM_PROGRAM=hide`, no
// version) and that it shows hyperlinks (`FORCE_HYPERLINK=1`), every
// `HERDR_*` variable is removed rather than set, and fish's reflow variable
// is not set.

//! The environment a pane's program starts with: the terminal the holder
//! emulates, and nothing that names another terminal, another pane or the
//! agent that started the daemon (PRD engine-bundle-3-core-owns D-9). The
//! pane's own identity, `HIDE_PANE_ID`, is the starting node's to add, since
//! the node names the pane by the core's id.

use portable_pty::CommandBuilder;

/// The terminal the holder emulates, an xterm with 256 colors and true color.
const TERM: &str = "xterm-256color";
const COLORTERM: &str = "truecolor";
/// What a program reads to know which terminal runs it.
const TERM_PROGRAM: &str = "hide";
/// That the terminal shows OSC 8 hyperlinks, which Hide's does: the
/// `supports-hyperlinks` convention, which Claude Code reads (it knows no
/// `TERM_PROGRAM=hide`). An agent that marks its links tells the shell where
/// a URL it broke across rows ends (`encode::LINK_END_OSC`).
const FORCE_HYPERLINK: &str = "1";

/// Handles of the outer terminal that started the daemon, never of this pane.
const OUTER_TERMINAL: [&str; 13] = [
    "TERM_PROGRAM_VERSION",
    "ITERM_SESSION_ID",
    "LC_TERMINAL",
    "LC_TERMINAL_VERSION",
    "WEZTERM_PANE",
    "KITTY_WINDOW_ID",
    "WT_SESSION",
    "TMUX",
    "TMUX_PANE",
    "STY",
    "ZELLIJ",
    "ZELLIJ_SESSION_NAME",
    "ZELLIJ_PANE_ID",
];

/// Variables that make a program a child of the agent that started the
/// daemon; a new pane is nobody's child agent.
const PARENT_AGENT: [&str; 6] = [
    "CODEX_THREAD_ID",
    "OMPCODE",
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_TOKEN",
];

/// The prefix of Herdr's pane identity (`HERDR_PANE_ID`, `HERDR_SOCKET_PATH`
/// and the rest): a pane on this engine is no Herdr pane, and a program that
/// read one would act on the Herdr that started the daemon.
const HERDR_PREFIX: &str = "HERDR_";

/// Sets `command`'s terminal and removes what names another terminal, pane
/// or agent, whatever the environment it was started from held (an app
/// opened from the Dock has no `TERM`).
pub fn apply(command: &mut CommandBuilder) {
    command.env("TERM", TERM);
    command.env("COLORTERM", COLORTERM);
    command.env("TERM_PROGRAM", TERM_PROGRAM);
    command.env("FORCE_HYPERLINK", FORCE_HYPERLINK);
    let herdr: Vec<String> = command
        .iter_full_env_as_str()
        .map(|(key, _)| key)
        .filter(|key| key.to_ascii_uppercase().starts_with(HERDR_PREFIX))
        .map(str::to_owned)
        .collect();
    for key in herdr
        .iter()
        .map(String::as_str)
        .chain(OUTER_TERMINAL)
        .chain(PARENT_AGENT)
    {
        command.env_remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_says_it_runs_in_hide_and_names_no_herdr_pane_or_outer_terminal() {
        let mut command = CommandBuilder::new("sh");
        command.env("HERDR_PANE_ID", "w1:p1");
        command.env("HERDR_SOCKET_PATH", "/tmp/herdr.sock");
        command.env("herdr_env", "1");
        command.env("TERM_PROGRAM", "iTerm.app");
        command.env("TERM_PROGRAM_VERSION", "3.5");
        command.env("TMUX", "/tmp/tmux");
        command.env("CLAUDECODE", "1");
        command.env("HIDE_PANE_ID", "w1:p2");
        command.env("PATH", "/usr/bin");

        apply(&mut command);

        let env: std::collections::BTreeMap<_, _> = command.iter_full_env_as_str().collect();
        assert_eq!(env.get("TERM_PROGRAM"), Some(&"hide"));
        assert_eq!(env.get("TERM"), Some(&"xterm-256color"));
        assert_eq!(env.get("COLORTERM"), Some(&"truecolor"));
        assert_eq!(env.get("FORCE_HYPERLINK"), Some(&"1"));
        assert!(
            env.keys()
                .all(|key| !key.to_ascii_uppercase().starts_with("HERDR_")),
            "{env:?}"
        );
        for gone in ["TERM_PROGRAM_VERSION", "TMUX", "CLAUDECODE"] {
            assert!(!env.contains_key(gone), "{gone} stays in {env:?}");
        }
        assert_eq!(env.get("HIDE_PANE_ID"), Some(&"w1:p2"));
        assert_eq!(env.get("PATH"), Some(&"/usr/bin"));
    }
}
