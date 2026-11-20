//! `hide skill …` and `hide mcp …`: a machine's skills and MCP servers,
//! changed the way Settings › 스킬·MCP changes them (PRD skill-mcp-manager
//! B31-B33, D-19, D-29). The core plans and runs the change
//! (`hide_core::store_command`); this file only reads the words and prints
//! the answer.
//!
//! `add` and `update` wait until the operator answers the confirm Hide shows
//! in their 내 차례, and print what they chose; there is no option to skip
//! it. A key's value is never an argument: `--env` and `--header` take a
//! name, and the operator types its value on the confirm.

use std::time::Duration;

use hide_agent_hooks::mcp::{Reach, Remote};
use hide_core::store_command::{CONFIRM_WAIT, PASS_WAIT, StoreCommand};
use hide_kit::store::{ItemKind, SourceSpec};
use serde_json::json;

use crate::env::Env;

pub const SKILL_USAGE: &str = "hide skill list [--machine <device id>]\nhide skill show <name> [--machine <device id>]\nhide skill add <github address|folder> [--name <skill>]... [--machine <device id>]\nhide skill update <name> [--machine <device id>]\nhide skill on|off <name> [--agent <agent>] [--machine <device id>]\nhide skill remove <name> [--machine <device id>]\nadd and update wait until the operator answers the confirm in their 내 차례.";

pub const MCP_USAGE: &str = "hide mcp list [--machine <device id>]\nhide mcp show <name> [--machine <device id>]\nhide mcp add|update <name> [--env <NAME>]... -- <command> [<argument>...] [--machine <device id>]\nhide mcp add|update <name> --url <https address> [--sse] [--header <NAME>]... [--machine <device id>]\nhide mcp on|off <name> [--agent <agent>] [--machine <device id>]\nhide mcp remove <name> [--machine <device id>]\n--env and --header take a key's name only; the operator types its value on the confirm in their 내 차례.";

/// The answer comes a little after the core's own wait ends, which then
/// answers `store_unanswered` itself.
const SLACK: Duration = Duration::from_secs(15);

/// The command `args` (the words after `hide skill` or `hide mcp`) name.
pub fn parse(kind: ItemKind, args: &[String]) -> Result<StoreCommand, String> {
    let usage = match kind {
        ItemKind::Skill => SKILL_USAGE,
        ItemKind::Mcp => MCP_USAGE,
    };
    let verb = args.first().map(String::as_str).ok_or(usage)?;
    let mut words = Vec::new();
    let mut machine = None;
    let mut agent = None;
    let mut url = None;
    let mut sse = false;
    let mut names = Vec::new();
    let mut env = Vec::new();
    let mut headers = Vec::new();
    let mut program: Option<Vec<String>> = None;
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        let mut value = || {
            rest.next()
                .filter(|value| !value.starts_with("--"))
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        let once = |slot: &mut Option<String>, taken: String| {
            if slot.replace(taken).is_some() {
                return Err(format!("{arg} is given twice"));
            }
            Ok(())
        };
        match arg.as_str() {
            "--json" => {}
            "--machine" => once(&mut machine, value()?)?,
            "--agent" => once(&mut agent, value()?)?,
            "--name" if kind == ItemKind::Skill => names.push(value()?),
            "--url" if kind == ItemKind::Mcp => once(&mut url, value()?)?,
            "--sse" if kind == ItemKind::Mcp => sse = true,
            "--env" if kind == ItemKind::Mcp => env.push(key_name(arg, value()?)?),
            "--header" if kind == ItemKind::Mcp => headers.push(key_name(arg, value()?)?),
            "--" if kind == ItemKind::Mcp => {
                program = Some(rest.by_ref().cloned().collect());
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            word => words.push(word.to_owned()),
        }
    }
    let only = |allowed: bool| if allowed { Ok(()) } else { Err(usage.to_owned()) };
    let reach = || -> Result<Reach, String> {
        match (url.clone(), program.clone()) {
            (Some(url), None) if env.is_empty() => Ok(Reach::Remote {
                transport: if sse { Remote::Sse } else { Remote::Http },
                url,
                headers: headers.clone(),
            }),
            (None, Some(program)) if !sse && headers.is_empty() => {
                let mut program = program.into_iter();
                let command = program
                    .next()
                    .ok_or_else(|| "-- needs the server's command".to_owned())?;
                Ok(Reach::Local {
                    command,
                    args: program.collect(),
                    env: env.clone(),
                })
            }
            _ => Err(usage.to_owned()),
        }
    };
    let changes_reach = url.is_some() || program.is_some() || sse || !env.is_empty() || !headers.is_empty();
    let command = match (verb, words.as_slice()) {
        ("list", []) => {
            only(agent.is_none() && names.is_empty() && !changes_reach)?;
            StoreCommand::List { kind, machine }
        }
        ("show", [name]) => {
            only(agent.is_none() && names.is_empty() && !changes_reach)?;
            StoreCommand::Show {
                kind,
                name: name.clone(),
                machine,
            }
        }
        ("add", [source]) if kind == ItemKind::Skill => {
            only(agent.is_none())?;
            StoreCommand::Add {
                source: source_spec(source, machine.is_some()),
                names,
                machine,
            }
        }
        ("add", [name]) => {
            only(agent.is_none())?;
            StoreCommand::AddMcp {
                name: name.clone(),
                reach: reach()?,
                machine,
            }
        }
        ("update", [name]) if kind == ItemKind::Skill => {
            only(agent.is_none() && names.is_empty())?;
            StoreCommand::Update {
                name: name.clone(),
                machine,
            }
        }
        ("update", [name]) => {
            only(agent.is_none())?;
            StoreCommand::UpdateMcp {
                name: name.clone(),
                reach: reach()?,
                machine,
            }
        }
        (on @ ("on" | "off"), [name]) => {
            only(names.is_empty() && !changes_reach)?;
            StoreCommand::Set {
                kind,
                name: name.clone(),
                agent,
                on: on == "on",
                machine,
            }
        }
        ("remove", [name]) => {
            only(agent.is_none() && names.is_empty() && !changes_reach)?;
            StoreCommand::Remove {
                kind,
                name: name.clone(),
                machine,
            }
        }
        _ => return Err(usage.to_owned()),
    };
    if !command.valid() {
        return Err("an argument is empty, too long or holds a NUL".to_owned());
    }
    Ok(command)
}

/// A key's name, never its value: `NAME=value` is refused, since a key
/// travels only from the operator's own confirm (B33).
fn key_name(flag: &str, name: String) -> Result<String, String> {
    if name.contains('=') {
        return Err(format!(
            "{flag} takes a key's name only; the operator types its value on the confirm"
        ));
    }
    Ok(name)
}

/// What the source names: a folder of this machine (absolute, or relative
/// to the working folder, or one that is there), a folder spelled for
/// another machine, or a git address.
fn source_spec(source: &str, elsewhere: bool) -> SourceSpec {
    let path = std::path::Path::new(source);
    let spelled_as_path = source.starts_with('/')
        || source.starts_with("~/")
        || source.starts_with("./")
        || source.starts_with("../")
        || source.as_bytes().get(1..3) == Some(b":\\")
        || source.as_bytes().get(1..3) == Some(b":/");
    if elsewhere {
        return if spelled_as_path {
            SourceSpec::Folder {
                path: source.to_owned(),
            }
        } else {
            SourceSpec::Git {
                url: source.to_owned(),
            }
        };
    }
    let home = source
        .strip_prefix("~/")
        .and_then(|rest| hide_platform::host::home_dir().ok().map(|home| home.join(rest)));
    let local = home.unwrap_or_else(|| path.to_path_buf());
    if spelled_as_path || local.is_dir() {
        let absolute = std::path::absolute(&local).unwrap_or(local);
        return SourceSpec::Folder {
            path: absolute.to_string_lossy().into_owned(),
        };
    }
    SourceSpec::Git {
        url: source.to_owned(),
    }
}

pub fn run(env: &Env, command: StoreCommand) -> Result<(), String> {
    let wait = if command.confirms() {
        CONFIRM_WAIT
    } else {
        PASS_WAIT
    } + SLACK;
    if command.confirms() {
        eprintln!(
            "Waiting for the operator to answer the confirm in their 내 차례… If your tool stops waiting first, run the same command again: it waits on the same confirm."
        );
    }
    let answer = (|| {
        let mut credential = crate::workspace_cli::Credential::acquire(env)?;
        crate::workspace_cli::request_store(&mut credential, &command, wait)
    })();
    match answer {
        Ok(answer) if answer["ok"] == true => {
            println!("{}", json!({"ok":true,"value":answer["result"]}));
            Ok(())
        }
        Ok(answer) => {
            let code = answer["reason"]
                .as_str()
                .unwrap_or(hide_core::store_command::UNAVAILABLE)
                .to_owned();
            let message = answer["next_action"].as_str().unwrap_or(&code);
            let detail = answer["detail"].as_str();
            println!(
                "{}",
                json!({"ok":false,"error":{"code":code,"message":message,"detail":detail}})
            );
            Err(code)
        }
        Err(code) => {
            let message = if code == "request_timeout" {
                "The change may still be running; hide skill list or hide mcp list shows where it stands"
            } else {
                crate::cli::request_next_action(&code)
            };
            println!(
                "{}",
                json!({"ok":false,"error":{"code":code,"message":message}})
            );
            Err(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_owned).collect()
    }

    #[test]
    fn an_mcp_server_is_read_from_its_command_or_its_address_with_key_names_only() {
        assert_eq!(
            parse(ItemKind::Mcp, &words("add github --env GITHUB_TOKEN -- npx -y @modelcontextprotocol/server-github")),
            Ok(StoreCommand::AddMcp {
                name: "github".to_owned(),
                reach: Reach::Local {
                    command: "npx".to_owned(),
                    args: vec!["-y".to_owned(), "@modelcontextprotocol/server-github".to_owned()],
                    env: vec!["GITHUB_TOKEN".to_owned()],
                },
                machine: None,
            })
        );
        assert_eq!(
            parse(ItemKind::Mcp, &words("add linear --url https://mcp.linear.app/sse --sse --header Authorization")),
            Ok(StoreCommand::AddMcp {
                name: "linear".to_owned(),
                reach: Reach::Remote {
                    transport: Remote::Sse,
                    url: "https://mcp.linear.app/sse".to_owned(),
                    headers: vec!["Authorization".to_owned()],
                },
                machine: None,
            })
        );
    }

    #[test]
    fn a_key_value_is_refused_as_an_argument_and_no_option_skips_the_confirm() {
        assert!(parse(ItemKind::Mcp, &words("add github --env GITHUB_TOKEN=ghp_x -- npx server")).is_err());
        assert!(parse(ItemKind::Mcp, &words("add linear --url https://a --header Authorization=Bearer")).is_err());
        for skip in ["--yes", "--force", "--no-confirm", "--skip-confirm"] {
            assert_eq!(
                parse(ItemKind::Skill, &words(&format!("add someone/archify {skip}"))),
                Err(format!("unknown option {skip}")),
                "{skip}"
            );
        }
    }

    #[test]
    fn a_switch_names_its_agent_and_a_skill_source_reads_as_an_address_or_a_folder() {
        assert_eq!(
            parse(ItemKind::Skill, &words("off archify --agent antigravity")),
            Ok(StoreCommand::Set {
                kind: ItemKind::Skill,
                name: "archify".to_owned(),
                agent: Some("antigravity".to_owned()),
                on: false,
                machine: None,
            })
        );
        assert_eq!(
            parse(ItemKind::Skill, &words("add github.com/someone/archify --name archify --machine mini")),
            Ok(StoreCommand::Add {
                source: SourceSpec::Git {
                    url: "github.com/someone/archify".to_owned()
                },
                names: vec!["archify".to_owned()],
                machine: Some("mini".to_owned()),
            })
        );
        assert_eq!(
            parse(ItemKind::Skill, &words("add /home/me/skills --machine mini")),
            Ok(StoreCommand::Add {
                source: SourceSpec::Folder {
                    path: "/home/me/skills".to_owned()
                },
                names: Vec::new(),
                machine: Some("mini".to_owned()),
            })
        );
    }
}
