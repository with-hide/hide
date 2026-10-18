//! `--no-daemon` on every Codex Hide starts (PRD overview-request-view D-20).
//!
//! A Codex attached to the shared app-server daemon runs its hooks in the
//! daemon's environment, so Herdr never learns that pane's session and Hide
//! cannot read it (openai/codex#48500). Every start Hide makes, a new agent,
//! a start from an issue or PR, a reopen, a fork and a wake, goes through
//! [`crate::wire::agent_start_params`], which takes the machine's answer here
//! and puts the flag first. Whether the machine's shared server is on does
//! not matter to the flag: the kit no longer turns the server off by itself
//! (PRD settings-cleanup D-14), only the operator's own request does, and a
//! machine where it was turned off earlier keeps it off.
//!
//! An older Codex has no daemon and refuses the flag, so the flag goes only
//! where the machine's kit read a Codex that has the daemon setting; a
//! machine whose kit has not answered yet refuses a start with
//! [`CAPABILITY_UNKNOWN`].
//!
//! A transition path: it goes away once openai/codex#48500 runs a daemon's
//! hooks in each window's environment.

use crate::model::KitSnapshot;

pub(crate) const NO_DAEMON: &str = "--no-daemon";

/// The refusal of a Codex start on a machine whose kit has not read its
/// Codex: a stable code, worded for the operator where it is shown (the
/// CLI's next action in `hided`, the screen's in the web shell's catalog).
pub(crate) const CAPABILITY_UNKNOWN: &str = "codex_capability_unknown";

/// What a machine's kit last read about its Codex's shared daemon.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum CodexDaemon {
    /// No answer, no Codex there, or a failed capability read.
    #[default]
    Unknown,
    /// The actual binary was read and has no shared daemon setting.
    Unsupported,
    /// That Codex has the shared daemon and the `--no-daemon` flag.
    Present,
}

impl CodexDaemon {
    /// The machine's answer from its kit's last report.
    pub(crate) fn from_kit(kit: &KitSnapshot) -> Self {
        if kit.unavailable.is_some() {
            return Self::Unknown;
        }
        match kit.codex_daemon {
            Some(true) => Self::Present,
            Some(false) => Self::Unsupported,
            None => Self::Unknown,
        }
    }
}

/// Whether a start of `kind` needs the machine's kit answer first: only a
/// start this module may put the flag on does.
pub(crate) fn needs_kit_answer(kind: &str) -> bool {
    hide_agent_adapter::adapter(kind).and_then(|row| row.start)
        == Some(hide_agent_adapter::LaunchDialect::Codex)
}

/// `args` for a start of `kind`, with `--no-daemon` first for a Codex that
/// has the daemon, once (the caller may already carry it).
pub(crate) fn start_arguments(
    kind: &str,
    daemon: CodexDaemon,
    args: Vec<String>,
) -> Result<Vec<String>, String> {
    if !needs_kit_answer(kind) {
        return Ok(args);
    }
    match daemon {
        CodexDaemon::Unknown => Err(CAPABILITY_UNKNOWN.to_owned()),
        CodexDaemon::Unsupported => Ok(args),
        CodexDaemon::Present
            if args
                .iter()
                .take_while(|arg| arg.as_str() != "--")
                .any(|arg| arg == NO_DAEMON) =>
        {
            Ok(args)
        }
        CodexDaemon::Present => Ok(std::iter::once(NO_DAEMON.to_owned()).chain(args).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit(codex_daemon: Option<bool>) -> KitSnapshot {
        KitSnapshot {
            codex_daemon,
            ..KitSnapshot::default()
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn a_codex_with_the_daemon_starts_without_it_on_every_kind_of_start() {
        let present = CodexDaemon::from_kit(&kit(Some(true)));
        assert_eq!(
            start_arguments("codex", present, strings(&["resume", "abc"])),
            Ok(strings(&["--no-daemon", "resume", "abc"]))
        );
        assert_eq!(
            start_arguments("codex", present, strings(&["--", "fix the bug"])),
            Ok(strings(&["--no-daemon", "--", "fix the bug"]))
        );
        assert_eq!(
            start_arguments("codex", present, Vec::new()),
            Ok(strings(&["--no-daemon"]))
        );
        // Never twice.
        assert_eq!(
            start_arguments("codex", present, strings(&["--no-daemon"])),
            Ok(strings(&["--no-daemon"]))
        );
    }

    #[test]
    fn only_a_confirmed_older_codex_gets_no_flag() {
        let mut older = kit(Some(false));
        assert_eq!(
            start_arguments(
                "codex",
                CodexDaemon::from_kit(&older),
                strings(&["resume", "a"])
            ),
            Ok(strings(&["resume", "a"]))
        );
        older.codex_daemon = Some(true);
        assert_eq!(
            start_arguments("codex", CodexDaemon::from_kit(&older), vec![]),
            Ok(strings(&["--no-daemon"]))
        );
        assert_eq!(
            start_arguments("claude", CodexDaemon::Unknown, strings(&["--resume", "a"])),
            Ok(strings(&["--resume", "a"]))
        );
    }

    #[test]
    fn an_unread_capability_or_an_unavailable_kit_refuses_the_common_start_boundary() {
        let mut unavailable = kit(Some(true));
        unavailable.unavailable = Some("no kit here".to_owned());
        for kit in [kit(None), unavailable, KitSnapshot::default()] {
            for args in [vec![], strings(&["resume", "a"]), strings(&["--no-daemon"])] {
                let error = crate::wire::agent_start_params(
                    "pane",
                    "agent",
                    "codex",
                    args,
                    CodexDaemon::from_kit(&kit),
                )
                .unwrap_err();
                assert_eq!(error, CAPABILITY_UNKNOWN);
            }
        }
    }

    #[test]
    fn a_prompt_that_mentions_the_flag_is_not_a_process_flag() {
        assert_eq!(
            start_arguments(
                "codex",
                CodexDaemon::Present,
                strings(&["--", "--no-daemon"])
            ),
            Ok(strings(&["--no-daemon", "--", "--no-daemon"]))
        );
    }
}
