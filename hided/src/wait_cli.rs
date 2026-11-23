//! `hide wait …` (PRD core-wait D-21): an agent's own wait on a pull request,
//! its CI, a branch, a mission or a mission's node, answered by the core
//! through the pane capability `hide mission` uses. The agent is put to
//! sleep at its next rest and woken with one Hide notice when the wait is
//! met, fails or passes its deadline.

use crate::env::Env;
use hide_core::missions::{MissionCommand, next_action};
use hide_core::waits::WaitInput;
use serde_json::json;

/// The guide `hide wait help` prints.
pub const HELP: &str = include_str!("../assets/wait/help.md");

pub const USAGE: &str = "hide wait help\nhide wait pr <number> [--ci] [--for <2h|3d>]\nhide wait branch <name> [--for <2h|3d>]\nhide wait mission <id> [--node <key>] [--for <2h|3d>]\nhide wait list\nhide wait cancel <wait-id>";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WaitCli {
    Help,
    Command(MissionCommand),
}

/// The words after `hide wait`.
pub fn parse<'a>(mut args: impl Iterator<Item = &'a String>) -> Result<WaitCli, String> {
    let Some(verb) = args.next() else {
        return Err(USAGE.to_owned());
    };
    let rest: Vec<&String> = args.collect();
    let mut positional = Vec::new();
    let mut ci = false;
    let mut for_span = None;
    let mut node = None;
    let mut at = 0;
    while at < rest.len() {
        let word = rest[at].as_str();
        match word {
            "--ci" if verb == "pr" => ci = true,
            "--for" | "--node" => {
                let value = rest
                    .get(at + 1)
                    .map(|value| value.as_str())
                    .filter(|value| !value.is_empty() && !value.starts_with('-'))
                    .ok_or_else(|| format!("{word} needs a value"))?;
                let slot = if word == "--for" {
                    &mut for_span
                } else {
                    &mut node
                };
                if slot.replace(value.to_owned()).is_some() {
                    return Err(format!("{word} is given twice"));
                }
                at += 1;
            }
            _ if word.starts_with('-') => {
                return Err(format!("unknown flag for hide wait {verb}: {word}"));
            }
            _ => positional.push(word.to_owned()),
        }
        at += 1;
    }
    if node.is_some() && verb != "mission" {
        return Err("--node goes with hide wait mission".to_owned());
    }
    if for_span.is_some() && matches!(verb.as_str(), "list" | "cancel" | "help" | "--help") {
        return Err(USAGE.to_owned());
    }
    if let Some(span) = &for_span
        && hide_core::waits::span_minutes(span).is_none()
    {
        return Err(
            "--for is minutes, hours or days between 1h and 30d, such as 2h or 3d".to_owned(),
        );
    }
    let one = |positional: &[String]| match positional {
        [value] => Ok(value.clone()),
        _ => Err(USAGE.to_owned()),
    };
    let command = match verb.as_str() {
        "help" | "--help" if positional.is_empty() => return Ok(WaitCli::Help),
        "pr" => {
            let number = one(&positional)?;
            let number = number
                .trim_start_matches(['#', '!'])
                .parse::<u32>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or("the pull request is its number, such as 1232")?;
            MissionCommand::Wait {
                wait: WaitInput {
                    pr: Some(number),
                    ci,
                    for_span,
                    ..WaitInput::default()
                },
            }
        }
        "branch" => MissionCommand::Wait {
            wait: WaitInput {
                branch: Some(one(&positional)?),
                for_span,
                ..WaitInput::default()
            },
        },
        "mission" => MissionCommand::Wait {
            wait: WaitInput {
                mission: Some(one(&positional)?),
                node,
                for_span,
                ..WaitInput::default()
            },
        },
        "list" if positional.is_empty() => MissionCommand::WaitList,
        "cancel" => MissionCommand::WaitCancel {
            id: one(&positional)?,
        },
        _ => return Err(USAGE.to_owned()),
    };
    let named = match &command {
        MissionCommand::Wait { wait } => wait.target().is_ok(),
        _ => true,
    };
    if !named || !command.valid() {
        return Err("a value is too long, holds control characters or names no target".to_owned());
    }
    Ok(WaitCli::Command(command))
}

pub fn run(env: &Env, cli: WaitCli) -> Result<(), String> {
    let command = match cli {
        WaitCli::Help => {
            print!("{HELP}");
            return Ok(());
        }
        WaitCli::Command(command) => command,
    };
    let answer = (|| {
        let mut credential = crate::workspace_cli::Credential::acquire(env)?;
        crate::workspace_cli::request_mission(&mut credential, &command)
    })();
    let (reason, next) = match answer {
        Ok(answer) if answer["ok"] == true => {
            println!("{}", json!({"ok":true,"result":answer["result"]}));
            return Ok(());
        }
        Ok(answer) => {
            let reason = answer["reason"]
                .as_str()
                .unwrap_or("mission_unavailable")
                .to_owned();
            let next = answer["next_action"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| next_action(&reason).to_owned());
            (reason, next)
        }
        Err(reason) => {
            let next = if reason == "request_timeout" {
                "The wait may have been made; run hide wait list before asking again".to_owned()
            } else if crate::cli::bootstrap_refused(&reason) {
                crate::cli::request_next_action(&reason).to_owned()
            } else {
                next_action(&reason).to_owned()
            };
            (reason, next)
        }
    };
    println!("{}", json!({"ok":false,"reason":reason,"next_action":next}));
    Err(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(words: &[&str]) -> Result<WaitCli, String> {
        let words: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
        parse(words.iter())
    }

    fn input(words: &[&str]) -> WaitInput {
        match parsed(words) {
            Ok(WaitCli::Command(MissionCommand::Wait { wait })) => wait,
            other => panic!("{words:?}: {other:?}"),
        }
    }

    #[test]
    fn each_target_reads_as_the_core_takes_it() {
        let pr = input(&["pr", "1232", "--ci", "--for", "3d"]);
        assert_eq!(
            (pr.pr, pr.ci, pr.for_span.as_deref()),
            (Some(1232), true, Some("3d"))
        );
        assert_eq!(input(&["pr", "#12"]).pr, Some(12));
        assert_eq!(
            input(&["branch", "feat/lib"]).branch.as_deref(),
            Some("feat/lib")
        );
        let node = input(&["mission", "mission-91", "--node", "gitlab-write"]);
        assert_eq!(
            (node.mission.as_deref(), node.node.as_deref()),
            (Some("mission-91"), Some("gitlab-write"))
        );
        assert_eq!(
            parsed(&["list"]),
            Ok(WaitCli::Command(MissionCommand::WaitList))
        );
        assert_eq!(
            parsed(&["cancel", "wait-3"]),
            Ok(WaitCli::Command(MissionCommand::WaitCancel {
                id: "wait-3".into()
            }))
        );
        assert_eq!(parsed(&["help"]), Ok(WaitCli::Help));
    }

    #[test]
    fn a_wait_that_is_not_one_is_refused_before_the_core() {
        for words in [
            &["pr", "abc"][..],
            &["pr", "0"],
            &["branch", "x", "--ci"],
            &["branch", "x", "--node", "n"],
            &["pr", "1", "--for", "5m"],
            &["pr", "1", "--for", "31d"],
            &["pr", "1", "--for"],
            &["mission", "mission-1", "mission-2"],
            &["list", "extra"],
            &["cancel"],
            &["later"],
        ] {
            assert!(parsed(words).is_err(), "{words:?}");
        }
    }
}
