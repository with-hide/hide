//! Whether an agent Hide starts asks before each tool call: the operator's
//! choice per agent and per machine in Settings › Agents (권한), and the
//! arguments it becomes on every start and restart Hide makes of that agent
//! there (⌘N, a new worktree, 맡기기, the phone, `hide agent spawn`, a wake,
//! a fork and a reopen, one typed by hand included).
//!
//! The choice is read when the start is made, never kept with the pane: a
//! restart follows the setting as it is now. Hide never edits the agent's own
//! configuration files; an agent the operator types into a terminal follows
//! that configuration alone.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The most bytes of options the operator may type for one agent.
pub(crate) const OPTIONS_LIMIT: usize = 512;
/// The most words those options may split into.
pub(crate) const OPTIONS_WORD_LIMIT: usize = 32;

/// One agent's choice on one machine. A missing entry is `Full`, the
/// default for every operator, so only a choice that differs is saved.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AgentPermission {
    /// The agent's declared full-access option (`hide_agent_adapter::FullAccess`).
    #[default]
    Full,
    /// No option: the CLI asks as its own configuration says.
    Ask,
    /// Options the operator typed, put after the program as given: `text`
    /// as typed, which the row shows again, and `options`, its words.
    Custom { text: String, options: Vec<String> },
}

/// Each agent's choice on one machine, by adapter id.
pub type AgentPermissions = BTreeMap<String, AgentPermission>;

impl AgentPermission {
    /// The choice for typed `text`: its words as a shell reads them, quoting
    /// removed, refused when it is anything but a list of words (a second
    /// command, a redirection, a substitution or an unclosed quote) or past
    /// the limits.
    pub(crate) fn custom(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.len() > OPTIONS_LIMIT {
            return Err(format!("options are longer than {OPTIONS_LIMIT} bytes"));
        }
        if text.chars().any(char::is_control) {
            return Err("options hold a control character".into());
        }
        let mut line = hide_shell::shell_line(text);
        if !text.is_empty() && line.commands.is_empty() {
            return Err("options end inside a quote".into());
        }
        if line.commands.len() > 1
            || !line.targets.is_empty()
            || !line.substitutions.is_empty()
            || line.fallback
        {
            return Err(
                "options are words only, with no second command, redirection or substitution"
                    .into(),
            );
        }
        let options = line.commands.pop().unwrap_or_default();
        if options.len() > OPTIONS_WORD_LIMIT {
            return Err(format!("options are more than {OPTIONS_WORD_LIMIT} words"));
        }
        Ok(Self::Custom {
            text: text.to_owned(),
            options,
        })
    }
}

/// The arguments a start of a `kind` agent adds for `permission`, which go
/// where the model and effort go: after a resume or fork selector
/// (`codex resume <id>` and `codex fork <id>` take the same option as a new
/// start, Codex 0.166.0's help) and before the native arguments and a first
/// prompt. None for a kind Hide does not start.
pub(crate) fn arguments(kind: &str, permission: &AgentPermission) -> Vec<String> {
    match permission {
        AgentPermission::Full => full_access(kind)
            .iter()
            .flat_map(|option| option.iter().map(|word| (*word).to_owned()))
            .collect(),
        AgentPermission::Ask => Vec::new(),
        AgentPermission::Custom { options, .. } => options.clone(),
    }
}

/// 매번 물어보기 for every agent Hide starts: what a machine's choices are
/// when they cannot be read, so an unreadable file never widens one to full
/// access.
pub(crate) fn every_agent_asks() -> AgentPermissions {
    hide_agent_adapter::ADAPTERS
        .iter()
        .filter(|row| row.start.is_some())
        .map(|row| (row.id.to_owned(), AgentPermission::Ask))
        .collect()
}

/// Each option a full-access start of a `kind` agent adds, as its words with
/// the option's name first; none for a kind Hide does not start.
pub(crate) fn full_access(kind: &str) -> &'static [&'static [&'static str]] {
    hide_agent_adapter::adapter(kind)
        .and_then(|row| row.start)
        .map_or(&[], |start| start.options().full_access.options())
}

/// `kind`'s choice in a machine's `permissions`.
pub(crate) fn of(permissions: &AgentPermissions, kind: &str) -> AgentPermission {
    hide_agent_adapter::adapter(kind)
        .and_then(|row| permissions.get(row.id))
        .cloned()
        .unwrap_or_default()
}

/// Claude Code's full-access words as 2.1.316 needs them (its help, and its
/// warning seen in an empty HOME on 2026-11-01): what tests expect a start to
/// carry, written here once rather than read from the adapter.
#[cfg(test)]
pub(crate) const CLAUDE_FULL_ACCESS: [&str; 3] = [
    "--dangerously-skip-permissions",
    "--settings",
    r#"{"skipDangerousModePermissionPrompt":true}"#,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn full_access_adds_each_agents_own_option_and_ask_adds_nothing() {
        // Each CLI's `--help`, read 2026-11-01.
        for (kind, expected) in [
            ("claude", words(&CLAUDE_FULL_ACCESS)),
            (
                "codex",
                words(&["--dangerously-bypass-approvals-and-sandbox"]),
            ),
            ("grok", words(&["--always-approve"])),
            ("opencode", words(&["--auto"])),
            ("omp", words(&["--auto-approve"])),
            ("agy", words(&["--dangerously-skip-permissions"])),
            ("pi", Vec::new()),
            ("terminal", Vec::new()),
        ] {
            assert_eq!(arguments(kind, &AgentPermission::Full), expected, "{kind}");
            assert_eq!(arguments(kind, &AgentPermission::Ask), Vec::<String>::new());
        }
    }

    #[test]
    fn typed_options_are_split_as_a_shell_reads_them_and_passed_as_given() {
        let permission = AgentPermission::custom(
            "  --permission-mode acceptEdits --append-system-prompt 'be brief'  ",
        )
        .unwrap();
        assert_eq!(
            arguments("claude", &permission),
            words(&[
                "--permission-mode",
                "acceptEdits",
                "--append-system-prompt",
                "be brief"
            ])
        );
        assert_eq!(
            permission,
            AgentPermission::Custom {
                text: "--permission-mode acceptEdits --append-system-prompt 'be brief'".into(),
                options: words(&[
                    "--permission-mode",
                    "acceptEdits",
                    "--append-system-prompt",
                    "be brief"
                ]),
            }
        );
        // A `$` stays literal: Herdr quotes every argument it types.
        assert_eq!(
            arguments("codex", &AgentPermission::custom("-c x=$HOME").unwrap()),
            words(&["-c", "x=$HOME"])
        );
        assert_eq!(
            arguments("claude", &AgentPermission::custom("").unwrap()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn typed_options_that_are_not_only_words_are_refused() {
        for text in [
            "--yolo; rm -rf ~",
            "--a && --b",
            "--a > out",
            "--a $(whoami)",
            "--a 'unclosed",
            "--a\u{7}",
        ] {
            assert!(AgentPermission::custom(text).is_err(), "{text}");
        }
        assert!(AgentPermission::custom(&"x".repeat(OPTIONS_LIMIT + 1)).is_err());
        assert!(AgentPermission::custom(&"-a ".repeat(OPTIONS_WORD_LIMIT + 1)).is_err());
    }

    #[test]
    fn a_machine_without_a_saved_choice_gives_every_agent_full_access() {
        let mut permissions = AgentPermissions::new();
        assert_eq!(of(&permissions, "claude"), AgentPermission::Full);
        permissions.insert("codex".into(), AgentPermission::Ask);
        // A kind is found by any of its adapter's spellings.
        assert_eq!(of(&permissions, "codex"), AgentPermission::Ask);
        assert_eq!(of(&permissions, "claude-code"), AgentPermission::Full);
    }

    #[test]
    fn a_saved_choice_reads_back_as_it_was_written() {
        let permissions: AgentPermissions = serde_json::from_value(serde_json::json!({
            "claude-code": {"mode": "ask"},
            "codex": {"mode": "custom", "text": "--full-auto", "options": ["--full-auto"]},
        }))
        .unwrap();
        assert_eq!(of(&permissions, "claude"), AgentPermission::Ask);
        assert_eq!(
            arguments("codex", &of(&permissions, "codex")),
            words(&["--full-auto"])
        );
    }
}
