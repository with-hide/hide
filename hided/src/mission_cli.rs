//! `hide mission …` and `hide agent catalog` (PRD software-factory-conductor
//! D-23, D-35): a Conductor's commands over its mission record, and the
//! catalog of what a machine's agents take. Both are answered by the core
//! without a shell, through the pane capability a delivery command uses.

use crate::env::Env;
use herdr_core::missions::{
    Checks, MissionCommand, MissionEnd, MissionScope, NodePatch, NodeStatus, PlanInput, Verdict,
    next_action,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The guide `hide mission help` prints, which the `hide` skill's guide points
/// at: how a Conductor works with the operator and what Hide holds it to.
pub const HELP: &str = include_str!("../assets/mission/help.md");

pub const USAGE: &str = "hide mission help\nhide mission create --title <title> [--scope pr|merge] [--cap <n>] [--github <account>]\nhide mission plan [--mission <id>] (--file <plan.json> | --json <plan>)\nhide mission node <key> [--mission <id>] [--status <status>] [--agent-id <id> | --no-agent-id] [--pr <n> | --no-pr] [--base-pr <n> | --no-base-pr] [--branch <name> | --no-branch] [--attempts <n>] [--line <text> | --no-line] [--change <text> | --no-change] [--review <text> | --no-review] [--commit <sha> | --no-commit] [--verdict pass|fail | --no-verdict] [--checks <passed>/<total> | --no-checks]\nhide mission node <key> [--mission <id>] [--reviewer <agent-id>] [--verifier <agent-id>]\nhide mission record <node> <line> [--mission <id>] (--pass | --fail) --evidence <text> [--commit <sha>] [--rerun]\nhide mission skip <node> <line> [--mission <id>] --reason <text> [--decision <key>]\nhide mission defect <node> [--mission <id>] --found-by <node> --line <text>\nhide mission start [--mission <id>]\nhide mission decide [--mission <id>] (--line <text> [--why <text>] [--option <text>]... [--recommend <n>] [--node <key>] | --clear)\nhide mission delay [--mission <id>] (--line <text> | --clear)\nhide mission close [--mission <id>] --as done|merge_wait|cancelled [--report <text> | --report-file <path>]\nhide mission reopen [--mission <id>]\nhide mission take <id>\nhide mission show [--mission <id>]\nhide mission list\nhide mission settings";

/// The largest plan or report file read from disk; the daemon refuses a
/// frame past its own limit anyway.
const FILE_LIMIT: u64 = 96 * 1024;

pub const STATUSES: &[&str] = &[
    "planned", "working", "waiting", "review", "held", "done", "failed", "dropped",
];

// Parsed once per command line and never stored, so its size is not worth
// a box.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MissionCli {
    Help,
    Command(MissionCommand),
    /// A plan or report still to be read from a file, at run time.
    PlanFile {
        mission: Option<String>,
        path: String,
    },
    CloseFile {
        mission: Option<String>,
        end: MissionEnd,
        path: String,
    },
}

/// Each verb's flags, those that take a value and then the switches: the
/// only ones its parser reads, and the ones the CLI contract names for it,
/// which `cli_contract`'s tests hold to this table (`--option`, read first,
/// is the one given more than once).
pub const FLAGS: &[(&str, &[&str], &[&str])] = &[
    ("create", &["--title", "--scope", "--cap", "--github"], &[]),
    ("plan", &["--mission", "--file", "--json"], &[]),
    (
        "node",
        &[
            "--mission",
            "--status",
            "--agent-id",
            "--pr",
            "--base-pr",
            "--branch",
            "--attempts",
            "--line",
            "--change",
            "--review",
            "--commit",
            "--verdict",
            "--checks",
            "--reviewer",
            "--verifier",
        ],
        &[
            "--no-agent-id",
            "--no-pr",
            "--no-base-pr",
            "--no-branch",
            "--no-line",
            "--no-change",
            "--no-review",
            "--no-commit",
            "--no-verdict",
            "--no-checks",
        ],
    ),
    (
        "record",
        &["--mission", "--evidence", "--commit"],
        &["--pass", "--fail", "--rerun"],
    ),
    ("skip", &["--mission", "--reason", "--decision"], &[]),
    ("defect", &["--mission", "--found-by", "--line"], &[]),
    (
        "decide",
        &[
            "--mission",
            "--line",
            "--why",
            "--option",
            "--recommend",
            "--node",
        ],
        &["--clear"],
    ),
    ("delay", &["--mission", "--line"], &["--clear"]),
    (
        "close",
        &["--mission", "--as", "--report", "--report-file"],
        &[],
    ),
    ("start", &["--mission"], &[]),
    ("reopen", &["--mission"], &[]),
    ("show", &["--mission"], &[]),
    ("take", &[], &[]),
    ("list", &[], &[]),
    ("settings", &[], &[]),
];

/// A command line's positionals, its valued options and its switches set.
type Flags = (Vec<String>, BTreeMap<String, String>, Vec<String>);

/// `verb`'s command line read by its row of `FLAGS`.
fn flags<'a>(args: impl Iterator<Item = &'a String>, verb: &str) -> Result<Flags, String> {
    let (_, valued, switches) = FLAGS
        .iter()
        .find(|(name, ..)| *name == verb)
        .ok_or_else(|| USAGE.to_owned())?;
    read_flags(args, valued, switches)
}

fn read_flags<'a>(
    args: impl Iterator<Item = &'a String>,
    valued: &[&str],
    switches: &[&str],
) -> Result<Flags, String> {
    let mut positional = Vec::new();
    let mut values = BTreeMap::new();
    let mut set = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let arg = arg.as_str();
        if arg == "--json" && !valued.contains(&"--json") {
            continue;
        }
        if valued.contains(&arg) {
            let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
            if value.starts_with("--")
                || value.is_empty()
                || value.chars().any(|c| c.is_control() && c != '\n')
            {
                return Err(format!("{arg} needs a value"));
            }
            if values.insert(arg.to_owned(), value.clone()).is_some() {
                return Err(format!("{arg} is given twice"));
            }
        } else if switches.contains(&arg) {
            if set.contains(&arg.to_owned()) {
                return Err(format!("{arg} is given twice"));
            }
            set.push(arg.to_owned());
        } else if arg.starts_with('-') {
            return Err(format!("unknown flag for hide mission: {arg}"));
        } else {
            positional.push(arg.to_owned());
        }
    }
    Ok((positional, values, set))
}

fn key(value: &str) -> Result<String, String> {
    if !value.starts_with('-') && herdr_core::missions::valid_key(value) {
        Ok(value.to_owned())
    } else {
        Err(format!("{value} is not a mission or node key"))
    }
}

fn number(flag: &str, value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| format!("{flag} takes a pull request number"))
}

fn one<'a>(
    values: &'a BTreeMap<String, String>,
    set: &[String],
    value: &str,
    clear: &str,
) -> Result<Option<Option<&'a str>>, String> {
    match (values.get(value), set.iter().any(|flag| flag == clear)) {
        (Some(_), true) => Err(format!("{value} and {clear} cannot be combined")),
        (Some(text), false) => Ok(Some(Some(text.as_str()))),
        (None, true) => Ok(Some(None)),
        (None, false) => Ok(None),
    }
}

/// `<passed>/<total>`, as a verify node's checks are recorded (B33).
fn checks(value: &str) -> Result<Checks, String> {
    value
        .split_once('/')
        .and_then(|(passed, total)| {
            Some(Checks {
                passed: passed.trim().parse().ok()?,
                total: total.trim().parse().ok()?,
            })
        })
        .ok_or_else(|| "--checks takes <passed>/<total>, such as 6/6".to_owned())
}

fn commit(flag: &str, value: &str) -> Result<String, String> {
    if herdr_core::missions::valid_commit(value) {
        Ok(value.to_owned())
    } else {
        Err(format!("{flag} takes a commit: 7 to 64 hexadecimal digits"))
    }
}

fn mission_of(values: &BTreeMap<String, String>) -> Result<Option<String>, String> {
    values.get("--mission").map(|id| key(id)).transpose()
}

/// Parses the words after `hide mission`.
pub fn parse<'a>(mut args: impl Iterator<Item = &'a String>) -> Result<MissionCli, String> {
    let verb = args.next().ok_or(USAGE)?.as_str();
    let command = match verb {
        "help" | "--help" => {
            if args.next().is_some() {
                return Err(USAGE.to_owned());
            }
            return Ok(MissionCli::Help);
        }
        "create" => {
            let (positional, values, _) = flags(args, "create")?;
            if !positional.is_empty() {
                return Err(USAGE.to_owned());
            }
            let title = values.get("--title").ok_or("--title is required")?.clone();
            let scope = values
                .get("--scope")
                .map(|scope| match scope.as_str() {
                    "pr" => Ok(MissionScope::Pr),
                    "merge" => Ok(MissionScope::Merge),
                    _ => Err("--scope is pr or merge".to_owned()),
                })
                .transpose()?;
            let cap = values
                .get("--cap")
                .map(|cap| {
                    cap.parse::<u8>()
                        .map_err(|_| "--cap takes a number from 1 to 16".to_owned())
                })
                .transpose()?;
            MissionCommand::Create {
                title,
                scope,
                cap,
                github: values.get("--github").cloned(),
            }
        }
        "plan" => {
            let (positional, values, _) = flags(args, "plan")?;
            if !positional.is_empty() {
                return Err(USAGE.to_owned());
            }
            let mission = mission_of(&values)?;
            match (values.get("--file"), values.get("--json")) {
                (Some(path), None) => {
                    return Ok(MissionCli::PlanFile {
                        mission,
                        path: path.clone(),
                    });
                }
                (None, Some(text)) => MissionCommand::Plan {
                    mission,
                    plan: plan_input(text)?,
                },
                _ => return Err("give the plan with --file or --json".to_owned()),
            }
        }
        "node" => {
            let (positional, values, set) = flags(args, "node")?;
            let [node] = positional.as_slice() else {
                return Err(USAGE.to_owned());
            };
            let status = values
                .get("--status")
                .map(|status| {
                    serde_json::from_value::<NodeStatus>(json!(status))
                        .map_err(|_| format!("--status is one of {}", STATUSES.join(", ")))
                })
                .transpose()?;
            let text = |value: &str, clear: &str| -> Result<Option<Option<String>>, String> {
                Ok(one(&values, &set, value, clear)?.map(|value| value.map(str::to_owned)))
            };
            let pr = |flag: &str, clear: &str| -> Result<Option<Option<u32>>, String> {
                one(&values, &set, flag, clear)?
                    .map(|value| value.map(|value| number(flag, value)).transpose())
                    .transpose()
            };
            let agent_id = text("--agent-id", "--no-agent-id")?;
            if agent_id
                .as_ref()
                .and_then(|id| id.as_deref())
                .is_some_and(|id| id.starts_with('-'))
            {
                return Err("--agent-id takes a registration id".to_owned());
            }
            let patch = NodePatch {
                status,
                agent_id,
                pr: pr("--pr", "--no-pr")?,
                base_pr: pr("--base-pr", "--no-base-pr")?,
                branch: text("--branch", "--no-branch")?,
                attempts: values
                    .get("--attempts")
                    .map(|value| {
                        value
                            .parse::<u8>()
                            .map_err(|_| "--attempts takes a number".to_owned())
                    })
                    .transpose()?,
                line: text("--line", "--no-line")?,
                change: text("--change", "--no-change")?,
                review: text("--review", "--no-review")?,
                worker: None,
                commit: one(&values, &set, "--commit", "--no-commit")?
                    .map(|value| value.map(|value| commit("--commit", value)).transpose())
                    .transpose()?,
                verdict: one(&values, &set, "--verdict", "--no-verdict")?
                    .map(|value| {
                        value
                            .map(|value| match value {
                                "pass" => Ok(Verdict::Pass),
                                "fail" => Ok(Verdict::Fail),
                                _ => Err("--verdict is pass or fail".to_owned()),
                            })
                            .transpose()
                    })
                    .transpose()?,
                checks: one(&values, &set, "--checks", "--no-checks")?
                    .map(|value| value.map(checks).transpose())
                    .transpose()?,
                reviewer: values.get("--reviewer").map(|id| key(id)).transpose()?,
                verifier: values.get("--verifier").map(|id| key(id)).transpose()?,
            };
            MissionCommand::Node {
                mission: mission_of(&values)?,
                node: key(node)?,
                patch,
            }
        }
        "record" => {
            let (positional, values, set) = flags(args, "record")?;
            let [node, line] = positional.as_slice() else {
                return Err(USAGE.to_owned());
            };
            let pass = match (
                set.iter().any(|flag| flag == "--pass"),
                set.iter().any(|flag| flag == "--fail"),
            ) {
                (true, false) => true,
                (false, true) => false,
                _ => return Err("give --pass or --fail".to_owned()),
            };
            MissionCommand::Record {
                mission: mission_of(&values)?,
                node: key(node)?,
                line: key(line)?,
                pass,
                evidence: values
                    .get("--evidence")
                    .filter(|evidence| !evidence.trim().is_empty())
                    .cloned()
                    .ok_or_else(|| {
                        "give --evidence with the one line the result rests on".to_owned()
                    })?,
                commit: values
                    .get("--commit")
                    .map(|value| commit("--commit", value))
                    .transpose()?,
                rerun: set.iter().any(|flag| flag == "--rerun"),
            }
        }
        "skip" => {
            let (positional, values, _) = flags(args, "skip")?;
            let [node, line] = positional.as_slice() else {
                return Err(USAGE.to_owned());
            };
            MissionCommand::Skip {
                mission: mission_of(&values)?,
                node: key(node)?,
                line: key(line)?,
                reason: values
                    .get("--reason")
                    .ok_or("--reason is required")?
                    .clone(),
                decision: values
                    .get("--decision")
                    .map(|value| {
                        value
                            .parse::<u64>()
                            .ok()
                            .filter(|value| *value > 0)
                            .ok_or_else(|| "--decision takes the question's key".to_owned())
                    })
                    .transpose()?,
            }
        }
        "defect" => {
            let (positional, values, _) = flags(args, "defect")?;
            let [node] = positional.as_slice() else {
                return Err(USAGE.to_owned());
            };
            MissionCommand::Defect {
                mission: mission_of(&values)?,
                node: key(node)?,
                found_by: key(values.get("--found-by").ok_or("--found-by is required")?)?,
                line: values.get("--line").ok_or("--line is required")?.clone(),
            }
        }
        "decide" => {
            // `--option` is the one flag given more than once, in order.
            let args: Vec<&String> = args.collect();
            let mut options = Vec::new();
            let mut rest = Vec::new();
            let mut words = args.into_iter();
            while let Some(arg) = words.next() {
                if arg == "--option" {
                    let value = words.next().ok_or("--option needs a value")?;
                    if value.starts_with("--") || value.trim().is_empty() {
                        return Err("--option needs a value".to_owned());
                    }
                    options.push(value.clone());
                } else {
                    rest.push(arg);
                }
            }
            let (positional, values, set) = flags(rest.into_iter(), "decide")?;
            if !positional.is_empty() {
                return Err(USAGE.to_owned());
            }
            let line = one(&values, &set, "--line", "--clear")?
                .ok_or("give --line <text> or --clear")?
                .map(str::to_owned);
            let recommend = values
                .get("--recommend")
                .map(|value| {
                    value
                        .parse::<u8>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(|| "--recommend takes an option's number, from 1".to_owned())
                })
                .transpose()?;
            MissionCommand::Decide {
                mission: mission_of(&values)?,
                line,
                why: values.get("--why").cloned(),
                options,
                recommend,
                node: values.get("--node").map(|node| key(node)).transpose()?,
            }
        }
        "delay" => {
            let (positional, values, set) = flags(args, "delay")?;
            if !positional.is_empty() {
                return Err(USAGE.to_owned());
            }
            let line = one(&values, &set, "--line", "--clear")?
                .ok_or("give --line <text> or --clear")?
                .map(str::to_owned);
            MissionCommand::Delay {
                mission: mission_of(&values)?,
                line,
            }
        }
        "close" => {
            let (positional, values, _) = flags(args, "close")?;
            if !positional.is_empty() {
                return Err(USAGE.to_owned());
            }
            let end = match values.get("--as").map(String::as_str) {
                Some("done") => MissionEnd::Done,
                Some("merge_wait") => MissionEnd::MergeWait,
                Some("cancelled") => MissionEnd::Cancelled,
                _ => return Err("--as is done, merge_wait or cancelled".to_owned()),
            };
            let mission = mission_of(&values)?;
            match (values.get("--report"), values.get("--report-file")) {
                (Some(_), Some(_)) => {
                    return Err("--report and --report-file cannot be combined".to_owned());
                }
                (None, Some(path)) => {
                    return Ok(MissionCli::CloseFile {
                        mission,
                        end,
                        path: path.clone(),
                    });
                }
                (report, None) => MissionCommand::Close {
                    mission,
                    end,
                    report: report.cloned(),
                },
            }
        }
        verb @ ("start" | "reopen" | "show") => {
            let (positional, values, _) = flags(args, verb)?;
            if !positional.is_empty() {
                return Err(USAGE.to_owned());
            }
            let mission = mission_of(&values)?;
            match verb {
                "start" => MissionCommand::Start { mission },
                "reopen" => MissionCommand::Reopen { mission },
                _ => MissionCommand::Show { mission },
            }
        }
        "take" => {
            let (positional, _, _) = flags(args, "take")?;
            let [mission] = positional.as_slice() else {
                return Err(USAGE.to_owned());
            };
            MissionCommand::Take {
                mission: key(mission)?,
            }
        }
        verb @ ("list" | "settings") => {
            let (positional, _, _) = flags(args, verb)?;
            if !positional.is_empty() {
                return Err(USAGE.to_owned());
            }
            if verb == "list" {
                MissionCommand::List
            } else {
                MissionCommand::Settings
            }
        }
        _ => return Err(USAGE.to_owned()),
    };
    if !command.valid() {
        return Err("a value is too long or holds control characters".to_owned());
    }
    Ok(MissionCli::Command(command))
}

fn plan_input(text: &str) -> Result<PlanInput, String> {
    serde_json::from_str(text)
        .map_err(|error| format!("the plan is not valid mission plan JSON: {error}"))
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

/// Runs a parsed `hide mission` command and prints its answer as one JSON
/// line: `{"ok":true,"result":…}`, or the refusal's reason and next action.
pub fn run(env: &Env, command: MissionCli) -> Result<(), String> {
    let command = match command {
        MissionCli::Help => {
            print!("{HELP}");
            return Ok(());
        }
        MissionCli::Command(command) => command,
        MissionCli::PlanFile { mission, path } => {
            let plan = match read_file(&path).and_then(|text| plan_input(&text)) {
                Ok(plan) => plan,
                Err(message) => return refuse("mission_plan_invalid", &message),
            };
            MissionCommand::Plan { mission, plan }
        }
        MissionCli::CloseFile { mission, end, path } => match read_file(&path) {
            Ok(report) => MissionCommand::Close {
                mission,
                end,
                report: Some(report),
            },
            Err(message) => return refuse("mission_text_invalid", &message),
        },
    };
    if !command.valid() {
        return refuse("mission_text_invalid", next_action("mission_text_invalid"));
    }
    let answer = (|| {
        let mut credential = crate::workspace_cli::Credential::acquire(env)?;
        crate::workspace_cli::request_mission(&mut credential, &command)
    })();
    print_answer(answer)
}

/// `hide agent catalog [--machine <id>]`, printed in the agent topic's
/// envelope.
pub fn run_catalog(env: &Env, machine: Option<String>) -> Result<(), String> {
    let answer = (|| {
        let mut credential = crate::workspace_cli::Credential::acquire(env)?;
        crate::workspace_cli::request_agent_catalog(&mut credential, machine.as_deref())
    })();
    match answer {
        Ok(answer) if answer["ok"] == true => {
            println!("{}", json!({"ok":true,"value":answer["result"]}));
            Ok(())
        }
        Ok(answer) => {
            let code = answer["reason"]
                .as_str()
                .unwrap_or("catalog_unavailable")
                .to_owned();
            let message = answer["next_action"].as_str().unwrap_or(&code);
            println!(
                "{}",
                json!({"ok":false,"error":{"code":code,"message":message}})
            );
            Err(code)
        }
        Err(code) => {
            println!(
                "{}",
                json!({"ok":false,"error":{"code":code,"message":crate::cli::request_next_action(&code)}})
            );
            Err(code)
        }
    }
}

fn print_answer(answer: Result<Value, String>) -> Result<(), String> {
    match answer {
        Ok(answer) if answer["ok"] == true => {
            println!("{}", json!({"ok":true,"result":answer["result"]}));
            Ok(())
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
            println!("{}", json!({"ok":false,"reason":reason,"next_action":next}));
            Err(reason)
        }
        Err(reason) => {
            let next = if reason == "request_timeout" {
                "The command may have been applied; run hide mission show before running it again"
            } else if crate::cli::bootstrap_refused(&reason) {
                crate::cli::request_next_action(&reason)
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
    fn a_node_change_keeps_what_it_does_not_name_and_clears_what_it_negates() {
        let args = words(&[
            "node",
            "a",
            "--status",
            "working",
            "--pr",
            "12",
            "--no-line",
        ]);
        let MissionCli::Command(MissionCommand::Node { node, patch, .. }) =
            parse(args.iter()).unwrap()
        else {
            panic!("a node command");
        };
        assert_eq!(node, "a");
        assert_eq!(patch.status, Some(NodeStatus::Working));
        assert_eq!(patch.pr, Some(Some(12)));
        assert_eq!(patch.line, Some(None));
        assert_eq!(patch.branch, None);
        let both = words(&["node", "a", "--line", "x", "--no-line"]);
        assert!(parse(both.iter()).is_err());
        let zero = words(&["node", "a", "--pr", "0"]);
        assert!(parse(zero.iter()).is_err());
    }

    #[test]
    fn a_plan_is_given_once_and_read_as_a_mission_plan() {
        let neither = words(&["plan"]);
        assert!(parse(neither.iter()).is_err());
        let bad = words(&["plan", "--json", "{\"nodes\": 3}"]);
        assert!(parse(bad.iter()).is_err());
        let good = words(&[
            "plan",
            "--json",
            "{\"nodes\": [{\"key\":\"a\",\"title\":\"A\",\"kind\":\"research\"}]}",
        ]);
        assert!(matches!(
            parse(good.iter()),
            Ok(MissionCli::Command(MissionCommand::Plan { .. }))
        ));
        let file = words(&["plan", "--mission", "mission-2", "--file", "plan.json"]);
        assert_eq!(
            parse(file.iter()).unwrap(),
            MissionCli::PlanFile {
                mission: Some("mission-2".into()),
                path: "plan.json".into()
            }
        );
    }

    #[test]
    fn decide_needs_a_line_or_clear_and_close_needs_its_end() {
        assert!(parse(words(&["decide"]).iter()).is_err());
        assert!(matches!(
            parse(words(&["decide", "--clear"]).iter()),
            Ok(MissionCli::Command(MissionCommand::Decide {
                line: None,
                ..
            }))
        ));
        // A decision's choices keep their order; the recommendation names one by number.
        assert_eq!(
            parse(
                words(&[
                    "decide",
                    "--line",
                    "Which base?",
                    "--why",
                    "Two PRs touch it",
                    "--option",
                    "main",
                    "--option",
                    "#1503",
                    "--recommend",
                    "2",
                    "--node",
                    "ui",
                ])
                .iter()
            )
            .unwrap(),
            MissionCli::Command(MissionCommand::Decide {
                mission: None,
                line: Some("Which base?".into()),
                why: Some("Two PRs touch it".into()),
                options: vec!["main".into(), "#1503".into()],
                recommend: Some(2),
                node: Some("ui".into()),
            })
        );
        assert!(parse(words(&["decide", "--line", "Q", "--recommend", "0"]).iter()).is_err());
        assert!(parse(words(&["decide", "--line", "Q", "--option"]).iter()).is_err());
        assert!(parse(words(&["close"]).iter()).is_err());
        assert!(matches!(
            parse(words(&["close", "--as", "cancelled"]).iter()),
            Ok(MissionCli::Command(MissionCommand::Close {
                end: MissionEnd::Cancelled,
                ..
            }))
        ));
    }

    #[test]
    fn record_takes_one_result_and_skip_needs_its_reason() {
        let Ok(MissionCli::Command(MissionCommand::Record {
            node,
            line,
            pass,
            commit,
            rerun,
            ..
        })) = parse(
            words(&[
                "record",
                "core",
                "check",
                "--pass",
                "--evidence",
                "cargo test: 12 passed",
                "--commit",
                "abc1234",
                "--rerun",
            ])
            .iter(),
        )
        else {
            panic!("record did not parse");
        };
        assert_eq!(
            (node.as_str(), line.as_str(), pass, commit.as_deref(), rerun),
            ("core", "check", true, Some("abc1234"), true)
        );
        assert!(parse(words(&["record", "core", "c1"]).iter()).is_err());
        // A worker's line and a checker's alike carry the line they rest on.
        for line in ["c1", "review"] {
            let refused = |evidence: &[&str]| {
                let mut argv = vec!["record", "core", line, "--fail"];
                argv.extend_from_slice(evidence);
                parse(words(&argv).iter()).unwrap_err()
            };
            assert!(refused(&[]).contains("--evidence"));
            assert!(refused(&["--evidence", "   "]).contains("--evidence"));
            assert!(refused(&["--evidence"]).contains("--evidence"));
        }
        assert!(
            parse(
                words(&[
                    "record",
                    "core",
                    "c1",
                    "--pass",
                    "--fail",
                    "--evidence",
                    "e"
                ])
                .iter()
            )
            .is_err()
        );
        assert!(
            parse(
                words(&[
                    "record",
                    "core",
                    "c1",
                    "--pass",
                    "--evidence",
                    "e",
                    "--commit",
                    "xyz"
                ])
                .iter()
            )
            .unwrap_err()
            .contains("--commit takes a commit")
        );
        assert!(parse(words(&["skip", "core", "c1"]).iter()).is_err());
        assert!(matches!(
            parse(
                words(&[
                    "skip",
                    "core",
                    "ci",
                    "--reason",
                    "flaky on main",
                    "--decision",
                    "3"
                ])
                .iter()
            ),
            Ok(MissionCli::Command(MissionCommand::Skip {
                decision: Some(3),
                ..
            }))
        ));
        assert!(matches!(
            parse(
                words(&[
                    "defect",
                    "core",
                    "--found-by",
                    "screen",
                    "--line",
                    "drops v1 nodes"
                ])
                .iter()
            ),
            Ok(MissionCli::Command(MissionCommand::Defect { .. }))
        ));
        let Ok(MissionCli::Command(MissionCommand::Node { patch, .. })) =
            parse(words(&["node", "core", "--reviewer", "agent-7"]).iter())
        else {
            panic!("node did not parse");
        };
        assert_eq!(patch.reviewer.as_deref(), Some("agent-7"));
    }

    #[test]
    fn node_records_a_commit_a_verdict_and_checks() {
        let Ok(MissionCli::Command(MissionCommand::Node { patch, .. })) = parse(
            words(&[
                "node",
                "check",
                "--verdict",
                "fail",
                "--checks",
                "4/6",
                "--no-commit",
            ])
            .iter(),
        ) else {
            panic!("node did not parse");
        };
        assert_eq!(patch.verdict, Some(Some(Verdict::Fail)));
        assert_eq!(
            patch.checks,
            Some(Some(Checks {
                passed: 4,
                total: 6
            }))
        );
        assert_eq!(patch.commit, Some(None));
        assert!(parse(words(&["node", "check", "--checks", "six"]).iter()).is_err());
        assert!(parse(words(&["node", "check", "--verdict", "maybe"]).iter()).is_err());
    }
}
