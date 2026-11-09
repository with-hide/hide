//! `hide guide`: the agent's guide to Hide as a whole, beside `hide browser
//! help` and `hide mission help`; a skill or a hook only points at it. It
//! needs no daemon and no environment.
//!
//! The spawning and pane rules are the session-start text every agent is
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
        // B9: the top-level guide routes planning to missions; workers keep
        // their node steps in the mission guide.
        assert!(!guide.contains("hide task help"));
        assert!(guide.contains("run `hide mission help`"));
        assert!(guide.contains("A worker or reviewer of a mission node"));
    }

    #[test]
    fn the_guide_spells_hide_ps_as_the_command_parses_it() {
        assert!(TEXT.contains(crate::ps_cli::USAGE));
    }

    #[test]
    fn the_guide_spells_hide_session_as_the_command_parses_it() {
        assert!(TEXT.contains(crate::session_cli::USAGE));
    }

    #[test]
    fn the_guide_spells_hide_agent_profile_as_the_command_parses_it() {
        assert!(TEXT.contains(crate::profile_cli::USAGE));
    }

    #[test]
    fn the_guide_spells_hide_device_as_the_command_parses_it() {
        assert!(TEXT.contains(crate::device_cli::USAGE));
    }

    #[test]
    fn the_guide_spells_hide_pane_as_the_command_parses_it() {
        assert!(TEXT.contains(crate::pane_cli::USAGE));
    }

    /// The guide teaches a step only a person takes by the command that
    /// declares it, and parses as the CLI does (PRD ask-human B27).
    #[test]
    fn the_guide_teaches_ask_human_in_place_of_leaving_a_login() {
        assert!(!TEXT.contains("Leave a login to the operator."));
        for example in [
            "hide agent ask-human auth https://github.com/login",
            "hide agent ask-human --clear",
        ] {
            assert!(TEXT.contains(example), "{example}");
            let args: Vec<String> = example.split(' ').skip(1).map(str::to_owned).collect();
            crate::cli_contract::admit(&args).unwrap();
            crate::agent_cli::parse(args[1..].iter()).unwrap();
        }
    }

    #[test]
    fn the_guide_spells_hide_workspace_remove_as_the_command_parses_it() {
        assert!(TEXT.contains(crate::worktree_cli::USAGE));
    }

    /// What an agent is taught at its session's start, by `hide guide` and
    /// by `hide mission help`.
    fn taught() -> Vec<(&'static str, String)> {
        vec![
            (
                "session start",
                hide_agent_hooks::PURPOSE_CONTEXT.to_owned(),
            ),
            ("hide guide", text()),
            ("hide mission help", crate::mission_cli::HELP.to_owned()),
        ]
    }

    /// PRD engine-bundle-1 D-6, B1: an agent is taught Hide's commands only,
    /// never Herdr's program, its variables or its commands.
    #[test]
    fn what_an_agent_is_taught_names_no_herdr() {
        for (name, text) in taught() {
            assert!(!text.to_lowercase().contains("herdr"), "{name} names Herdr");
        }
    }

    /// Every `hide` command of the contract's topics that an agent is taught
    /// is a command of the contract, so a guide cannot point at one that
    /// does not exist.
    #[test]
    fn every_contract_command_an_agent_is_taught_is_in_the_contract() {
        let mut checked = 0;
        for (name, text) in taught() {
            for (at, _) in text.match_indices("hide ") {
                let words: Vec<&str> = text[at + "hide ".len()..]
                    .split(|c: char| c.is_whitespace() || c == '`' || c == ',')
                    .take(3)
                    .take_while(|word| {
                        !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c == '-')
                    })
                    .collect();
                let Some(topic) = words.first() else { continue };
                if !crate::cli_contract::topic(topic) && *topic != "workspace" {
                    continue;
                }
                if words.get(1) == Some(&"help")
                    || (*topic == "workspace"
                        && !matches!(words.get(1), Some(&"purpose" | &"remove")))
                {
                    continue;
                }
                let args: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
                // A topic named on its own (`the hide mission commands`)
                // names no command.
                if args.len() == 1 && !crate::cli_contract::names(&args) {
                    continue;
                }
                assert!(
                    crate::cli_contract::names(&args),
                    "{name} teaches hide {}, which the contract does not name",
                    words.join(" ")
                );
                checked += 1;
            }
        }
        assert!(checked > 40, "only {checked} commands were checked");
    }
}
