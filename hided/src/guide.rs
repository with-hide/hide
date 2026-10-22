//! `hide guide`: the agent's guide to Hide as a whole, beside `hide browser
//! help` and `hide mission help`; a skill or a hook only points at it. It
//! needs no daemon and no environment.
//!
//! The spawning and Herdr rules are the session-start text every agent is
//! given (`hide_agent_hooks::PURPOSE_CONTEXT`), put in at print time, so the
//! guide and the session note cannot say two different things.

const TEXT: &str = include_str!("../assets/guide.md");

/// Where the session-start rules go in [`TEXT`].
const SESSION_RULES: &str = "{session_rules}";

pub fn text() -> String {
    TEXT.replacen(SESSION_RULES, hide_agent_hooks::PURPOSE_CONTEXT, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_carries_the_session_start_rules_once() {
        assert_eq!(TEXT.matches(SESSION_RULES).count(), 1);
        let guide = text();
        assert_eq!(guide.matches(hide_agent_hooks::PURPOSE_CONTEXT).count(), 1);
    }

    #[test]
    fn the_guide_spells_hide_ps_as_the_command_parses_it() {
        assert!(TEXT.contains(crate::ps_cli::USAGE));
    }

    #[test]
    fn the_guide_spells_hide_device_as_the_command_parses_it() {
        assert!(TEXT.contains(crate::device_cli::USAGE));
    }
}
