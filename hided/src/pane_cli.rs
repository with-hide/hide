//! `hide pane read|send|open`, `hide agent read` and `hide workspace
//! purpose`: an agent's reads of and input to the panes of its machine, and
//! its checkout's purpose, through Hide rather than Herdr (PRD
//! engine-bundle-1 D-5). The daemon runs them (`herdr_core::pane_command`).

use herdr_core::pane_command::{DEFAULT_LINES, PaneCommand};
use serde_json::json;

use crate::env::Env;

pub const USAGE: &str = "hide pane read <pane> [--lines <n>] [--json]\nhide pane send <pane> <text> [--enter]\nhide pane open --beside [--focus] -- <command>\nhide agent read <agent> [--lines <n>] [--json]\nhide workspace purpose <text>";

/// One parsed command, and whether its read is printed as the answer line.
#[derive(Debug, Eq, PartialEq)]
pub struct PaneCli {
    pub command: PaneCommand,
    pub json: bool,
}

/// `--lines <n>`: any decimal number, which the command's own limit then
/// judges, so a number past it is refused with its reason.
fn lines(value: Option<&String>) -> Result<u32, String> {
    let value = value.ok_or_else(|| USAGE.to_owned())?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(USAGE.to_owned());
    }
    Ok(value
        .parse::<u64>()
        .map_or(u32::MAX, |lines| u32::try_from(lines).unwrap_or(u32::MAX)))
}

fn word(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('-') && !value.chars().any(char::is_control)
}

/// A read of `pane read` or `agent read`: the subject, then `--lines` and
/// `--json` in any order.
fn read(args: &[String], subject: impl Fn(String, u32) -> PaneCommand) -> Result<PaneCli, String> {
    let [target, rest @ ..] = args else {
        return Err(USAGE.to_owned());
    };
    if !word(target) {
        return Err(USAGE.to_owned());
    }
    let mut count = None;
    let mut json = false;
    let mut rest = rest.iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--lines" if count.is_none() => count = Some(lines(rest.next())?),
            "--json" if !json => json = true,
            _ => return Err(USAGE.to_owned()),
        }
    }
    Ok(PaneCli {
        command: subject(target.clone(), count.unwrap_or(DEFAULT_LINES)),
        json,
    })
}

/// `hide pane <verb> …`, the words after `pane`.
pub fn parse(args: &[String]) -> Result<PaneCli, String> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(USAGE.to_owned());
    };
    match verb.as_str() {
        "read" => read(rest, |pane, lines| PaneCommand::Read { pane, lines }),
        "send" => {
            let (pane, text, enter) = match rest {
                [pane, text] => (pane, text, false),
                [pane, text, flag] if flag == "--enter" => (pane, text, true),
                _ => return Err(USAGE.to_owned()),
            };
            if !word(pane) || text.trim().is_empty() {
                return Err(USAGE.to_owned());
            }
            Ok(PaneCli {
                command: PaneCommand::Send {
                    pane: pane.clone(),
                    text: text.clone(),
                    enter,
                },
                json: false,
            })
        }
        "open" => {
            let split = rest
                .iter()
                .position(|word| word == "--")
                .ok_or_else(|| USAGE.to_owned())?;
            let (flags, command) = (&rest[..split], &rest[split + 1..]);
            let mut beside = false;
            let mut focus = false;
            for flag in flags {
                match flag.as_str() {
                    "--beside" if !beside => beside = true,
                    "--focus" if !focus => focus = true,
                    _ => return Err(USAGE.to_owned()),
                }
            }
            if !beside || command.is_empty() {
                return Err(USAGE.to_owned());
            }
            Ok(PaneCli {
                command: PaneCommand::Open {
                    command: command.join(" "),
                    focus,
                },
                json: false,
            })
        }
        _ => Err(USAGE.to_owned()),
    }
}

/// `hide agent read …`, the words after `read`.
pub fn parse_agent_read(args: &[String]) -> Result<PaneCli, String> {
    read(args, |agent, lines| PaneCommand::AgentRead { agent, lines })
}

/// `hide workspace purpose <text>`, the words after `purpose`.
pub fn parse_purpose(args: &[String]) -> Result<PaneCli, String> {
    match args {
        [text] if !text.starts_with("--") && !text.trim().is_empty() => Ok(PaneCli {
            command: PaneCommand::Purpose { text: text.clone() },
            json: false,
        }),
        _ => Err(USAGE.to_owned()),
    }
}

/// Runs the command and prints its answer: a read's text, or one line,
/// `{"ok":true,"value":…}` or `{"ok":false,"error":{"code","message"}}` with
/// the step to take, and a non-zero exit for a refusal.
pub fn run(env: &Env, cli: PaneCli) -> Result<(), String> {
    let refused = |code: String, message: &str| {
        println!(
            "{}",
            json!({"ok":false,"error":{"code":code,"message":message}})
        );
        Err(code)
    };
    if let Err(code) = cli.command.check() {
        let message = herdr_core::pane_command::next_action(code).unwrap_or_default();
        return refused(code.to_owned(), message);
    }
    let raw = matches!(
        cli.command,
        PaneCommand::Read { .. } | PaneCommand::AgentRead { .. }
    ) && !cli.json;
    let answer = crate::workspace_cli::Credential::acquire(env).and_then(|mut credential| {
        crate::workspace_cli::request_pane(&mut credential, &cli.command)
    });
    match answer {
        Ok(answer) if answer["ok"] == true => {
            let value = &answer["result"];
            if raw {
                let text = value["text"].as_str().unwrap_or_default();
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
            } else {
                println!("{}", json!({"ok":true,"value":value}));
            }
            Ok(())
        }
        Ok(answer) => {
            let code = answer["reason"]
                .as_str()
                .unwrap_or("pane_unavailable")
                .to_owned();
            let message = answer["next_action"]
                .as_str()
                .or_else(|| herdr_core::pane_command::next_action(&code))
                .unwrap_or("Retry; the reason is in Hide's diagnostic log")
                .to_owned();
            refused(code, &message)
        }
        Err(code) => {
            let message = crate::cli::request_next_action(&code);
            refused(code, message)
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
    fn a_read_defaults_to_120_lines_and_takes_its_flags_in_any_order() {
        assert_eq!(
            parse(&words(&["read", "w1:p2"])).unwrap(),
            PaneCli {
                command: PaneCommand::Read {
                    pane: "w1:p2".into(),
                    lines: 120
                },
                json: false
            }
        );
        assert_eq!(
            parse_agent_read(&words(&["child", "--json", "--lines", "40"])).unwrap(),
            PaneCli {
                command: PaneCommand::AgentRead {
                    agent: "child".into(),
                    lines: 40
                },
                json: true
            }
        );
        // A number past the limit parses, and the command's check refuses it
        // with its reason rather than the usage.
        let past = parse(&words(&["read", "w1:p2", "--lines", "99999999999"])).unwrap();
        assert_eq!(past.command.check(), Err("lines_out_of_range"));
        assert!(parse(&words(&["read", "w1:p2", "--lines", "-1"])).is_err());
    }

    #[test]
    fn send_presses_enter_only_when_asked() {
        let sent = |line: &[&str]| match parse(&words(line)).unwrap().command {
            PaneCommand::Send { enter, .. } => enter,
            other => panic!("{other:?}"),
        };
        assert!(!sent(&["send", "w1:p2", "ls -la"]));
        assert!(sent(&["send", "w1:p2", "ls -la", "--enter"]));
        assert!(parse(&words(&["send", "w1:p2"])).is_err());
    }

    #[test]
    fn open_needs_beside_and_runs_the_words_after_the_separator_as_one_line() {
        assert_eq!(
            parse(&words(&["open", "--beside", "--", "npm", "run", "dev"]))
                .unwrap()
                .command,
            PaneCommand::Open {
                command: "npm run dev".into(),
                focus: false
            }
        );
        assert_eq!(
            parse(&words(&[
                "open",
                "--focus",
                "--beside",
                "--",
                "tail -f log"
            ]))
            .unwrap()
            .command,
            PaneCommand::Open {
                command: "tail -f log".into(),
                focus: true
            }
        );
        assert!(parse(&words(&["open", "--", "ls"])).is_err());
        assert!(parse(&words(&["open", "--beside"])).is_err());
    }
}
