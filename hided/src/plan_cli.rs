//! `hide plan …` (PRD hide-plan D-27): an agent's commands over its own
//! session's plan, answered by the core without a shell through the pane
//! capability a delivery command uses, on this machine or a connected
//! device's (D-21).
//!
//! Six verbs and four flags. Each answer the core gives for the caller's own
//! plan carries the summary the agent's hook keeps, and the CLI writes it
//! into the pane's record at once, so a plan put up for review stops the
//! next write in the same turn and an approval lets it through (D-30).

use crate::env::Env;
use herdr_core::plans::{PlanCommand, PlanInput, StepStatus, next_action};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The guide `hide plan help` prints, which the `hide-plan` skill points at.
pub const HELP: &str = include_str!("../assets/plan/help.md");

pub const USAGE: &str = "hide plan help\nhide plan set (--file <plan.json> | --json <plan>) [--ask [why]] [--reason <text>]\nhide plan node <key> [--status planned|working|done|dropped] [--line <text>]\nhide plan approve [<agent>]\nhide plan drop [<agent>]\nhide plan show [<agent>]";

/// The largest plan file read from disk.
const FILE_LIMIT: u64 = 96 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanCli {
    Help,
    Command(PlanCommand),
    /// A plan still to be read from a file, at run time.
    SetFile {
        path: String,
        ask: bool,
        why: Option<String>,
        reason: Option<String>,
    },
}

fn value<'a>(args: &'a [String], at: usize, flag: &str) -> Result<&'a str, String> {
    let value = args
        .get(at + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} needs a value"))?;
    if value.starts_with("--") || value.is_empty() || value.chars().any(|c| c.is_control()) {
        return Err(format!("{flag} needs a value"));
    }
    Ok(value)
}

/// A command line's positionals and valued options; `ask` is `Some` when
/// `--ask` was given, with its reason when one followed.
struct Flags {
    positional: Vec<String>,
    values: BTreeMap<String, String>,
    ask: Option<Option<String>>,
}

/// `--ask` takes its reason only when one follows that is not another flag.
fn flags(args: &[String], valued: &[&str]) -> Result<Flags, String> {
    let mut positional = Vec::new();
    let mut values = BTreeMap::new();
    let mut ask = None;
    let mut at = 0;
    while at < args.len() {
        let arg = args[at].as_str();
        if arg == "--ask" && valued.contains(&"--ask") {
            if ask.is_some() {
                return Err("--ask is given twice".into());
            }
            match args.get(at + 1) {
                Some(why) if !why.starts_with("--") => {
                    ask = Some(Some(value(args, at, arg)?.to_owned()));
                    at += 2;
                }
                _ => {
                    ask = Some(None);
                    at += 1;
                }
            }
            continue;
        }
        if valued.contains(&arg) {
            let text = value(args, at, arg)?;
            if values.insert(arg.to_owned(), text.to_owned()).is_some() {
                return Err(format!("{arg} is given twice"));
            }
            at += 2;
        } else if arg.starts_with('-') {
            return Err(format!("Unknown flag: {arg}"));
        } else {
            positional.push(arg.to_owned());
            at += 1;
        }
    }
    Ok(Flags {
        positional,
        values,
        ask,
    })
}

fn agent(positional: Vec<String>) -> Result<Option<String>, String> {
    match positional.as_slice() {
        [] => Ok(None),
        [agent] => Ok(Some(agent.clone())),
        _ => Err(USAGE.to_owned()),
    }
}

fn plan_input(text: &str) -> Result<PlanInput, String> {
    serde_json::from_str(text).map_err(|error| format!("the plan is not a plan: {error}"))
}

/// Parses the words after `hide plan`.
pub fn parse<'a>(args: impl Iterator<Item = &'a String>) -> Result<PlanCli, String> {
    let args: Vec<String> = args.cloned().collect();
    let (verb, rest) = args.split_first().ok_or(USAGE)?;
    let command = match verb.as_str() {
        "help" | "--help" => {
            return if rest.is_empty() {
                Ok(PlanCli::Help)
            } else {
                Err(USAGE.to_owned())
            };
        }
        "set" => {
            let Flags {
                positional,
                values,
                ask,
            } = flags(rest, &["--file", "--json", "--ask", "--reason"])?;
            if !positional.is_empty() {
                return Err(USAGE.to_owned());
            }
            let why = ask.clone().flatten();
            let reason = values.get("--reason").cloned();
            match (values.get("--file"), values.get("--json")) {
                (Some(path), None) => {
                    return Ok(PlanCli::SetFile {
                        path: path.clone(),
                        ask: ask.is_some(),
                        why,
                        reason,
                    });
                }
                (None, Some(text)) => PlanCommand::Set {
                    plan: plan_input(text)?,
                    ask: ask.is_some(),
                    why,
                    reason,
                },
                _ => return Err("give the plan once, with --file or --json".into()),
            }
        }
        "node" => {
            let Flags {
                positional, values, ..
            } = flags(rest, &["--status", "--line"])?;
            let [key] = positional.as_slice() else {
                return Err(USAGE.to_owned());
            };
            let status = values
                .get("--status")
                .map(|status| {
                    serde_json::from_value::<StepStatus>(json!(status)).map_err(|_| {
                        format!("--status takes planned, working, done or dropped, not {status}")
                    })
                })
                .transpose()?;
            let line = values.get("--line").cloned();
            if status.is_none() && line.is_none() {
                return Err("give --status, --line or both".into());
            }
            PlanCommand::Node {
                key: key.clone(),
                status,
                line,
            }
        }
        "approve" | "drop" | "show" => {
            let positional = flags(rest, &[])?.positional;
            let agent = agent(positional)?;
            match verb.as_str() {
                "approve" => PlanCommand::Approve { agent },
                "drop" => PlanCommand::Drop { agent },
                _ => PlanCommand::Show { agent },
            }
        }
        _ => return Err(USAGE.to_owned()),
    };
    if !command.valid() {
        return Err("a key or text is invalid, or a value is too long".to_owned());
    }
    Ok(PlanCli::Command(command))
}

fn read_file(path: &str) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|error| format!("{path}: {error}"))?;
    let mut text = String::new();
    file.take(FILE_LIMIT + 1)
        .read_to_string(&mut text)
        .map_err(|error| format!("{path}: {error}"))?;
    if text.len() as u64 > FILE_LIMIT {
        return Err(format!("{path} is larger than {FILE_LIMIT} bytes"));
    }
    Ok(text)
}

fn refuse(reason: &str, next: &str) -> Result<(), String> {
    println!("{}", json!({"ok":false,"reason":reason,"next_action":next}));
    Err(reason.to_owned())
}

/// Writes the answer's summary into this pane's record, so the hook's next
/// refusal and hint read it (D-30). A failed write is said on stderr and
/// the command still answers: the next prompt writes the record again.
fn keep_context(result: &Value) {
    let (Some(context), Some(session)) = (
        result.get("context").filter(|context| !context.is_null()),
        result["session"]["session_id"].as_str(),
    ) else {
        return;
    };
    let Ok(context) =
        serde_json::from_value::<hide_agent_hooks::plan_record::PlanContext>(context.clone())
    else {
        return;
    };
    let Some(pane) = std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|pane| !pane.is_empty())
    else {
        return;
    };
    let written = hide_platform::host::home_dir()
        .and_then(|home| hide_agent_hooks::plan_record::write(&home, &pane, session, &context));
    if let Err(error) = written {
        eprintln!("hide plan: the pane's plan record was not written: {error}");
    }
}

/// Runs a parsed `hide plan` command and prints its answer as one JSON
/// line: `{"ok":true,"result":…}`, or the refusal's reason and next action.
pub fn run(env: &Env, command: PlanCli) -> Result<(), String> {
    let command = match command {
        PlanCli::Help => {
            print!("{HELP}");
            return Ok(());
        }
        PlanCli::Command(command) => command,
        PlanCli::SetFile {
            path,
            ask,
            why,
            reason,
        } => match read_file(&path).and_then(|text| plan_input(&text)) {
            Ok(plan) => PlanCommand::Set {
                plan,
                ask,
                why,
                reason,
            },
            Err(message) => return refuse("plan_invalid", &message),
        },
    };
    if !command.valid() {
        return refuse("plan_invalid", next_action("plan_invalid"));
    }
    let answer = (|| {
        let mut credential = crate::workspace_cli::Credential::acquire(env)?;
        crate::workspace_cli::request_plan(&mut credential, &command)
    })();
    match answer {
        Ok(answer) if answer["ok"] == true => {
            keep_context(&answer["result"]);
            println!("{}", json!({"ok":true,"result":answer["result"]}));
            Ok(())
        }
        Ok(answer) => {
            let reason = answer["reason"]
                .as_str()
                .unwrap_or("plan_unavailable")
                .to_owned();
            let next = answer["next_action"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| next_action(&reason).to_owned());
            println!("{}", json!({"ok":false,"reason":reason,"next_action":next}));
            Err(reason)
        }
        Err(reason) => {
            let next = if reason == "request_timeout" {
                "The command may have been applied; run hide plan show before running it again"
            } else {
                next_action(&reason)
            };
            println!("{}", json!({"ok":false,"reason":reason,"next_action":next}));
            Err(reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &[&str]) -> Vec<String> {
        line.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn ask_takes_its_reason_only_when_one_follows() {
        let bare = words(&["set", "--file", "p.json", "--ask"]);
        assert_eq!(
            parse(bare.iter()).unwrap(),
            PlanCli::SetFile {
                path: "p.json".into(),
                ask: true,
                why: None,
                reason: None
            }
        );
        let why = words(&["set", "--ask", "scope moved", "--file", "p.json"]);
        assert_eq!(
            parse(why.iter()).unwrap(),
            PlanCli::SetFile {
                path: "p.json".into(),
                ask: true,
                why: Some("scope moved".into()),
                reason: None
            }
        );
        let plain = words(&[
            "set",
            "--json",
            r#"{"nodes":[{"key":"a","title":"A","depends_on":[],"status":"planned"}]}"#,
        ]);
        assert!(matches!(
            parse(plain.iter()).unwrap(),
            PlanCli::Command(PlanCommand::Set { ask: false, .. })
        ));
        assert!(parse(words(&["set"]).iter()).is_err());
    }

    #[test]
    fn a_node_change_names_a_status_or_a_line() {
        assert!(matches!(
            parse(words(&["node", "core", "--status", "done"]).iter()).unwrap(),
            PlanCli::Command(PlanCommand::Node {
                status: Some(StepStatus::Done),
                line: None,
                ..
            })
        ));
        assert!(parse(words(&["node", "core"]).iter()).is_err());
        assert!(parse(words(&["node", "core", "--status", "finished"]).iter()).is_err());
        assert!(parse(words(&["node", "bad key", "--line", "x"]).iter()).is_err());
    }

    #[test]
    fn approve_drop_and_show_take_at_most_one_agent() {
        assert_eq!(
            parse(words(&["approve"]).iter()).unwrap(),
            PlanCli::Command(PlanCommand::Approve { agent: None })
        );
        assert_eq!(
            parse(words(&["show", "agent-7"]).iter()).unwrap(),
            PlanCli::Command(PlanCommand::Show {
                agent: Some("agent-7".into())
            })
        );
        assert!(parse(words(&["drop", "a", "b"]).iter()).is_err());
    }
}
