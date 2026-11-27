//! `hide guide`: the agent's guide to Hide as a whole, beside `hide browser
//! help` and `hide mission help`; a skill or a hook only points at it. It
//! needs no daemon and no environment.
//!
//! A session starts with a short note (`hide_agent_hooks::START_CONTEXT`)
//! that names the common commands and points here, so every other rule an
//! agent needs lives in one section of this guide, and `hide guide <topic>`
//! prints that section alone. Its first section is the full table of what
//! the operator asks and the command for each
//! (`hide_agent_hooks::intent_table!`), put in at print time, so the guide
//! and the `hide` skill cannot say two different things.

const TEXT: &str = include_str!("../assets/guide.md");

/// Where the intent table goes in [`TEXT`].
const INTENT_TABLE: &str = "{intent_table}";
/// Where the list of topics goes in [`TEXT`].
const TOPICS: &str = "{topics}";

pub const USAGE: &str = "usage: hide guide [<topic>]";

/// The whole guide, with the topics `hide guide <topic>` takes listed after
/// its opening.
pub fn text() -> String {
    let topics: Vec<String> = topics().iter().map(|key| format!("`{key}`")).collect();
    body().replacen(TOPICS, &topics.join(", "), 1)
}

/// The section `topic` names, heading included, or none when no section is
/// named so. A topic is a section's heading in any case, with or without
/// its leading "The" and its punctuation: `letters`, `agent-profiles`,
/// `operators-screen`, "Project Memory".
pub fn section(topic: &str) -> Option<String> {
    let wanted = key(topic);
    let body = body();
    sections(&body)
        .find(|(heading, _)| key(heading) == wanted)
        .map(|(_, section)| section.to_owned())
}

/// Every topic, in the guide's order.
pub fn topics() -> Vec<String> {
    sections(&body()).map(|(heading, _)| key(heading)).collect()
}

/// What `hide guide <topic>` answers for a topic no section is named by.
pub fn unknown_topic(topic: &str) -> String {
    format!(
        "no guide topic named {topic:?}; the topics are {}\n{USAGE}",
        topics().join(", ")
    )
}

fn body() -> String {
    TEXT.replacen(INTENT_TABLE, hide_agent_hooks::intent_table!(), 1)
}

/// Each `## ` section of `body` with its heading, up to the next one.
fn sections(body: &str) -> impl Iterator<Item = (&str, &str)> {
    let starts: Vec<usize> = body.match_indices("\n## ").map(|(at, _)| at + 1).collect();
    let ends: Vec<usize> = starts.iter().skip(1).copied().chain([body.len()]).collect();
    starts.into_iter().zip(ends).map(move |(start, end)| {
        let section = &body[start..end];
        let heading = section["## ".len()..].lines().next().unwrap_or_default();
        (heading, section)
    })
}

/// A heading or a typed topic as one key: lower case, apostrophes dropped,
/// words joined by `-`, and a leading "the" left out.
fn key(text: &str) -> String {
    let words: Vec<String> = text
        .to_lowercase()
        .replace(['\'', '’'], "")
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();
    let words = match words.first().map(String::as_str) {
        Some("the") if words.len() > 1 => &words[1..],
        _ => &words[..],
    };
    words.join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_carries_the_intent_table_once_and_lists_its_topics() {
        assert_eq!(TEXT.matches(INTENT_TABLE).count(), 1);
        assert_eq!(TEXT.matches(TOPICS).count(), 1);
        let guide = text();
        assert_eq!(guide.matches(hide_agent_hooks::intent_table!()).count(), 1);
        assert!(!guide.contains(INTENT_TABLE) && !guide.contains(TOPICS));
        assert!(guide.contains("`hide guide <topic>` prints one section of this guide; the topics are `what-the-operator-asks`, `before-the-first-command`, `letters`,"));
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

    /// Each section is a topic of its own, named by its heading in any case,
    /// with or without its "The" and punctuation, and printed alone.
    #[test]
    fn each_section_of_the_guide_is_a_topic_printed_alone() {
        assert_eq!(
            topics(),
            [
                "what-the-operator-asks",
                "before-the-first-command",
                "letters",
                "agents",
                "waiting",
                "agent-profiles",
                "panes",
                "removing-a-worktree",
                "automations",
                "devices",
                "bots",
                "operators-screen",
                "project-memory",
                "past-sessions",
                "where-to-go-next",
            ]
        );
        for topic in topics() {
            let section = section(&topic).unwrap();
            assert!(section.starts_with("## "), "{topic}");
            assert!(
                !section.contains("\n## "),
                "{topic} runs into the next section"
            );
            assert!(text().contains(&section), "{topic}");
        }
        let screen = section("operators-screen").unwrap();
        assert!(screen.starts_with("## The operator's screen\n"));
        for typed in [
            "The operator's screen",
            "the-operators-screen",
            "OPERATORS SCREEN",
        ] {
            assert_eq!(section(typed).as_deref(), Some(screen.as_str()), "{typed}");
        }
        assert_eq!(section("Project Memory"), section("project-memory"));
        assert_eq!(section("nothing-like-it"), None);
        assert!(unknown_topic("x").contains("letters, agents"));
        assert!(unknown_topic("x").ends_with(USAGE));
    }

    /// The session-start note names topics by the key `hide guide` takes.
    #[test]
    fn the_topics_the_session_start_names_are_topics_of_the_guide() {
        let start = hide_agent_hooks::START_CONTEXT;
        for topic in [
            "agents",
            "letters",
            "panes",
            "operators-screen",
            "automations",
            "devices",
        ] {
            assert!(start.contains(&format!("`{topic}`")), "{topic}");
            assert!(section(topic).is_some(), "{topic}");
        }
        assert!(start.contains("`hide guide agents`"));
    }

    /// Every rule the session start stopped carrying (it once held the
    /// whole intent table and the pane, letter, worktree and delegation
    /// rules) lives in the section where an agent looks for it.
    #[test]
    fn every_rule_the_session_start_dropped_lives_in_its_guide_section() {
        let rules: &[(&str, &[&str])] = &[
            (
                "what-the-operator-asks",
                &[hide_agent_hooks::intent_table!()],
            ),
            (
                "panes",
                &[
                    "`hide pane read <pane>",
                    "`hide pane send <pane>",
                    "`hide pane open --beside -- <command>`",
                    "`hide agent read <agent>",
                    "`remote:<device id>:pane:<id>`",
                    "`core:<device id>:pane:<id>`",
                    "`@name (pane <address> · task)`",
                    "a pane that is no registered agent takes `hide pane send` and no letter",
                    "so hand an agent work with `hide request send`",
                    "40 characters at most",
                    "`cd <worktree> && hide workspace purpose \"…\"`",
                ],
            ),
            (
                "letters",
                &["`--force` puts the letter into the recipient's input now"],
            ),
            (
                "agents",
                &[
                    "`hide agent spawn --parent here …`: it records that you answer for the child and starts an automatic watch on it",
                    "`hide agent spawn …` without `--parent`: it becomes an operator-owned root with no automatic watch",
                    "Both record you as origin and leave the operator's screen and keyboard focus unchanged",
                    "Without `--beside`, `hide agent spawn` opens the agent in a tab of its own",
                    "`hide agent spawn --beside …` starts it in a pane beside yours, in your own checkout only",
                    "Never start an agent with `hide pane open` or by typing its program into a pane",
                    "From the machine that runs Hide, start the agent on a connected device by adding `--machine <device id>`",
                    "`--repo` and `--path` are then that device's paths",
                    "`machine_not_permitted`",
                    "`hide agent ask-human auth <where>`",
                    "needs a timeout of its own",
                    "Read it before you start an agent",
                    "`hide agent show <id>`",
                    "`hide ps`",
                ],
            ),
            (
                "operators-screen",
                &[
                    "asked for or not, you open as soon as it is ready, without asking whether to",
                    "so it interrupts nothing",
                    "Never show the operator a result with the system opener (`open`, `xdg-open`, `start`, `Start-Process`, `Invoke-Item`)",
                    "for a file outside the checkout too",
                    "such as `aside`, not `browser open`",
                    "read it with `view status <view-id>`",
                ],
            ),
            (
                "automations",
                &[
                    "`--schedule \"once in 7d 09:00\"`",
                    "Not a cron job, an OS scheduler or a script of your own",
                    "shorter than five minutes",
                    "Run `hide automation help` first",
                ],
            ),
            (
                "where-to-go-next",
                &[
                    "first ask in one line: Shall I run this as a mission, or do it directly?",
                    "With none of these signals, work directly without asking",
                    "run `hide issue help` and follow it",
                ],
            ),
        ];
        for (topic, wanted) in rules {
            let section = section(topic).unwrap();
            for rule in *wanted {
                assert!(section.contains(rule), "{topic} lacks {rule}");
            }
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
            ("session start", hide_agent_hooks::START_CONTEXT.to_owned()),
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
