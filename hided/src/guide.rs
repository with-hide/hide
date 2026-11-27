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
        // A pane pasted from another machine is read by its full address.
        assert!(guide.contains("remote:<device id>:pane:<id>"));
        // B9: the top-level guide routes planning to missions; workers keep
        // their node steps in the mission guide.
        assert!(!guide.contains("hide task help"));
        assert!(guide.contains("run `hide mission help`"));
        assert!(guide.contains("A worker or reviewer of a mission node"));
        assert!(guide.contains("run `hide issue help` and follow it"));
        for rule in [
            "explicitly asks for, invokes with `/hide-mission`",
            "without asking which way to work again",
            "first ask in one line: Shall I run this as a mission, or do it directly?",
            "Wait for the operator's answer and follow it",
            "With none of these signals, work directly without asking",
        ] {
            assert!(guide.contains(rule), "{rule}");
        }
    }

    #[test]
    fn the_guide_spells_hide_ps_as_the_command_parses_it() {
        assert!(TEXT.contains(crate::ps_cli::USAGE));
    }

    #[test]
    fn the_guide_spells_hide_session_as_the_command_parses_it() {
        assert!(TEXT.contains(crate::session_cli::USAGE));
    }

    /// An agent asked for a repeated task finds the automation command and
    /// its guide (PRD automations B9).
    #[test]
    fn the_guide_routes_a_repeated_task_to_an_automation() {
        let guide = text();
        assert!(guide.contains("## Automations"));
        assert!(guide.contains("Run `hide automation help` first"));
        assert!(guide.contains(
            "hide automation add --name <name> --task <text> --schedule <when> --kind <kind>"
        ));
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
    fn the_guide_spells_hide_access_as_the_command_parses_it() {
        assert!(TEXT.contains(crate::access_cli::USAGE));
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

    /// The guide names the fold of a settled question as the CLI parses it.
    #[test]
    fn the_guide_teaches_folding_a_settled_question() {
        let example = "hide session fold-ask <id> --reason \"<what settled it>\"";
        assert!(TEXT.contains(example), "{example}");
        let args: Vec<String> = ["session", "fold-ask", "missed-3-0", "--reason", "settled"]
            .map(str::to_owned)
            .to_vec();
        crate::cli_contract::admit(&args).unwrap();
        crate::session_cli::parse_fold_ask(&args[2..]).unwrap();
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
            ("hide issue help", crate::issue_cli::HELP.to_owned()),
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

    /// Every agent reads the same help whatever language its operator
    /// uses, and answers the operator in the language they write in, so
    /// what an agent is taught is English (docs/LOCALIZATION.md, A message
    /// for an agent is not screen copy): an example or a screen name in it
    /// is written in its English form.
    #[test]
    fn what_an_agent_is_taught_is_english() {
        let helps = [
            ("hide automation help", crate::automation_cli::HELP),
            ("hide task help", crate::plan_cli::HELP),
            ("hide wait help", crate::wait_cli::HELP),
            ("hide browser help", crate::browser_page::HELP),
        ];
        let taught = taught()
            .into_iter()
            .chain(helps.map(|(name, text)| (name, text.to_owned())));
        for (name, text) in taught {
            let other: Vec<&str> = text
                .lines()
                .filter(|line| line.chars().any(korean_chinese_or_japanese))
                .collect();
            assert!(
                other.is_empty(),
                "{name} is not English:\n{}",
                other.join("\n")
            );
        }
    }

    fn korean_chinese_or_japanese(c: char) -> bool {
        matches!(
            c,
            '\u{1100}'..='\u{11ff}'
                | '\u{3040}'..='\u{30ff}'
                | '\u{3130}'..='\u{318f}'
                | '\u{3400}'..='\u{4dbf}'
                | '\u{4e00}'..='\u{9fff}'
                | '\u{ac00}'..='\u{d7a3}'
        )
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
