//! `hide agent profile …`: a machine's agent profiles, changed the way
//! Settings › Agents changes them (PRD agent-profiles D-05, D-33, B11). The
//! core plans and checks the change (`hide_core::agent_profiles`); this file
//! only reads the words and prints the answer.

use hide_core::agent_profiles::ProfileCommand;
use serde_json::json;

use crate::env::Env;

pub const USAGE: &str = "hide agent profile list [--kind <kind>] [--machine <device id>]\nhide agent profile add <kind> <name> [--account <folder|name>] [--program <program>] [--env KEY=VALUE]... [--default] [--machine <device id>]\nhide agent profile edit <kind> <name> [--rename <name>] [--program <program> | --no-program] [--env KEY=VALUE]... [--unset-env KEY]... [--machine <device id>]\nhide agent profile default <kind> <name> [--machine <device id>]\nhide agent profile remove <kind> <name> [--machine <device id>]\nhide agent profile reinstall <kind> <name> [--machine <device id>]";

/// The command `args` (the words after `hide agent profile`) name. The
/// contract has already admitted their shape (`cli_contract::admit`).
pub fn parse(args: &[String]) -> Result<ProfileCommand, String> {
    let verb = args.first().ok_or(USAGE)?.as_str();
    let mut words = Vec::new();
    let mut one = std::collections::BTreeMap::<&str, String>::new();
    let mut env = Vec::new();
    let mut unset_env = Vec::new();
    let mut default = false;
    let mut no_program = false;
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        let mut value = || {
            rest.next()
                .filter(|value| !value.starts_with("--"))
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--json" => {}
            "--default" => default = true,
            "--no-program" => no_program = true,
            "--env" => env.push(value()?),
            "--unset-env" => unset_env.push(value()?),
            flag @ ("--kind" | "--machine" | "--account" | "--program" | "--rename") => {
                let taken = value()?;
                if one.insert(flag, taken).is_some() {
                    return Err(format!("{flag} is given twice"));
                }
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            word => words.push(word.to_owned()),
        }
    }
    if no_program && one.contains_key("--program") {
        return Err("--program and --no-program cannot be given together".to_owned());
    }
    let machine = one.remove("--machine");
    let command = match (verb, words.as_slice()) {
        ("list", []) => ProfileCommand::List {
            machine,
            kind: one.remove("--kind"),
        },
        ("add", [kind, name]) => ProfileCommand::Add {
            machine,
            kind: kind.clone(),
            name: name.clone(),
            account: one.remove("--account"),
            program: one.remove("--program"),
            env,
            default,
        },
        ("edit", [kind, name]) => ProfileCommand::Edit {
            machine,
            kind: kind.clone(),
            name: name.clone(),
            rename: one.remove("--rename"),
            program: one.remove("--program"),
            no_program,
            env,
            unset_env,
        },
        ("default", [kind, name]) => ProfileCommand::Default {
            machine,
            kind: kind.clone(),
            name: name.clone(),
        },
        ("remove", [kind, name]) => ProfileCommand::Remove {
            machine,
            kind: kind.clone(),
            name: name.clone(),
        },
        ("reinstall", [kind, name]) => ProfileCommand::Reinstall {
            machine,
            kind: kind.clone(),
            name: name.clone(),
        },
        _ => return Err(USAGE.to_owned()),
    };
    if !one.is_empty() {
        return Err(USAGE.to_owned());
    }
    if !command.valid() {
        return Err("an argument is too long or holds a NUL".to_owned());
    }
    Ok(command)
}

pub fn run(env: &Env, command: ProfileCommand) -> Result<(), String> {
    let answer = (|| {
        let mut credential = crate::workspace_cli::Credential::acquire(env)?;
        crate::workspace_cli::request_agent_profile(&mut credential, &command)
    })();
    match answer {
        Ok(answer) if answer["ok"] == true => {
            println!("{}", json!({"ok":true,"value":answer["result"]}));
            Ok(())
        }
        Ok(answer) => {
            let code = answer["reason"]
                .as_str()
                .unwrap_or(hide_core::agent_profiles::UNANSWERED)
                .to_owned();
            let message = answer["next_action"].as_str().unwrap_or(&code);
            println!(
                "{}",
                json!({"ok":false,"error":{"code":code,"message":message}})
            );
            Err(code)
        }
        Err(code) => {
            let message = if code == "request_timeout" {
                "The change may still be running; hide agent profile list shows where it stands"
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

    fn parsed(args: &[&str]) -> Result<ProfileCommand, String> {
        parse(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn each_verb_reads_its_agent_name_and_options() {
        assert_eq!(
            parsed(&[
                "add",
                "claude",
                "work",
                "--account",
                "~/accounts/work",
                "--env",
                "A=1",
                "--env",
                "B=2",
                "--default",
            ]),
            Ok(ProfileCommand::Add {
                machine: None,
                kind: "claude".into(),
                name: "work".into(),
                account: Some("~/accounts/work".into()),
                program: None,
                env: vec!["A=1".into(), "B=2".into()],
                default: true,
            })
        );
        assert_eq!(
            parsed(&[
                "edit",
                "codex",
                "team",
                "--rename",
                "crew",
                "--no-program",
                "--unset-env",
                "A",
            ]),
            Ok(ProfileCommand::Edit {
                machine: None,
                kind: "codex".into(),
                name: "team".into(),
                rename: Some("crew".into()),
                program: None,
                no_program: true,
                env: Vec::new(),
                unset_env: vec!["A".into()],
            })
        );
        assert_eq!(
            parsed(&["list", "--kind", "omp", "--machine", "mini"]),
            Ok(ProfileCommand::List {
                machine: Some("mini".into()),
                kind: Some("omp".into()),
            })
        );
        assert!(parsed(&["remove", "claude"]).is_err());
        assert!(parsed(&["add", "claude", "a", "--account", "x", "--account", "y"]).is_err());
        assert!(parsed(&["rename", "claude", "a"]).is_err());
    }
}
