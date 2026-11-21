//! Which pane a process runs in, as its environment names it (PRD
//! engine-bundle-3-core-owns D-9): the one rule the `hide` CLI, the hook
//! helper and the daemon share, so each asks the core about the same pane.
//!
//! A pane names itself `HIDE_PANE_ID`, which its holder's node sets. The
//! name is only a claim: the daemon proves it against the pane's record and
//! the caller's process before it means anything (`hided::pane_auth`).

/// The variable a pane names itself by.
pub const HIDE_PANE_ID: &str = "HIDE_PANE_ID";

/// The pane this process's environment names; an empty value names none.
pub fn from_env() -> Option<String> {
    named_by(|key| std::env::var(key).ok())
}

/// The pane `get` names, by the same rule as [`from_env`].
pub fn named_by(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    get(HIDE_PANE_ID).filter(|value| !value.is_empty())
}
