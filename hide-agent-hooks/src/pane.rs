//! Which pane a process runs in, as its environment names it (PRD
//! engine-bundle-3-core-owns D-9): the one rule the `hide` CLI, the hook
//! helper and the daemon share, so each asks the core about the same pane.
//!
//! A pane on the core's own engine names itself `HIDE_PANE_ID`, which its
//! holder's node sets and no Herdr pane carries; a Herdr pane names itself
//! `HERDR_PANE_ID`. The name is only a claim: the daemon proves it against
//! the pane's record and the caller's process before it means anything
//! (`hided::pane_auth`).

/// The variable a pane on the core's own engine names itself by.
pub const HIDE_PANE_ID: &str = "HIDE_PANE_ID";
/// The variable a Herdr pane names itself by, read after [`HIDE_PANE_ID`].
pub const HERDR_PANE_ID: &str = "HERDR_PANE_ID";

/// The pane this process's environment names, `HIDE_PANE_ID` first; an
/// empty value names none.
pub fn from_env() -> Option<String> {
    named_by(|key| std::env::var(key).ok())
}

/// The pane `get` names, by the same rule as [`from_env`].
pub fn named_by(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    [HIDE_PANE_ID, HERDR_PANE_ID]
        .into_iter()
        .find_map(|key| get(key).filter(|value| !value.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn a_hide_pane_is_named_before_a_herdr_pane_and_an_empty_name_is_none() {
        assert_eq!(
            named_by(env(&[
                ("HERDR_PANE_ID", "w1:p1"),
                ("HIDE_PANE_ID", "w2:p3")
            ])),
            Some("w2:p3".to_owned())
        );
        assert_eq!(
            named_by(env(&[("HERDR_PANE_ID", "w1:p1"), ("HIDE_PANE_ID", "")])),
            Some("w1:p1".to_owned())
        );
        assert_eq!(named_by(env(&[("HERDR_PANE_ID", "")])), None);
        assert_eq!(named_by(env(&[])), None);
    }
}
