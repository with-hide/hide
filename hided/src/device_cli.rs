//! `hide device list|add|remove`: the SSH devices of Settings › Devices,
//! for an agent the operator asked to add or remove one. The core answers
//! (`herdr_core::device_command`); this file reads the words and prints the
//! answer, as one JSON line with `--json` and as lines for a person without.
use crate::env::Env;
use herdr_core::device_command::DeviceCommand;
use serde_json::{Value, json};

pub const USAGE: &str = "hide device list [--json]\nhide device add <ssh alias> [--name <name>] [--socket <path>] [--json]\nhide device remove <device id> [--json]";

#[derive(Debug, Eq, PartialEq)]
pub struct DeviceCli {
    pub command: DeviceCommand,
    pub json: bool,
}

pub fn parse<'a>(mut args: impl Iterator<Item = &'a String>) -> Result<DeviceCli, String> {
    let verb = args.next().map(String::as_str);
    let mut json = false;
    let mut name = None;
    let mut socket = None;
    let mut words = Vec::new();
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "--json" if !json => {
                json = true;
                continue;
            }
            "--name" if verb == Some("add") && name.is_none() => &mut name,
            "--socket" if verb == Some("add") && socket.is_none() => &mut socket,
            word if !word.starts_with('-') => {
                words.push(word.to_owned());
                continue;
            }
            _ => return Err(USAGE.to_owned()),
        };
        match args.next() {
            Some(value) if !value.starts_with('-') => *slot = Some(value.clone()),
            _ => return Err(USAGE.to_owned()),
        }
    }
    let command = match (verb, words.as_slice()) {
        (Some("list"), []) => DeviceCommand::List,
        (Some("add"), [alias]) => DeviceCommand::Add {
            ssh_alias: alias.clone(),
            name,
            herdr_socket_path: socket,
        },
        (Some("remove"), [device_id]) => DeviceCommand::Remove {
            device_id: device_id.clone(),
        },
        _ => return Err(USAGE.to_owned()),
    };
    if !command.valid() {
        return Err(USAGE.to_owned());
    }
    Ok(DeviceCli { command, json })
}

pub fn run(env: &Env, cli: &DeviceCli) -> Result<(), String> {
    let answer = (|| {
        let mut credential = crate::workspace_cli::Credential::acquire(env)?;
        crate::workspace_cli::request_device(&mut credential, &cli.command)
    })();
    let (code, message) = match answer {
        Ok(answer) if answer["ok"] == true => {
            let value = &answer["result"];
            if cli.json {
                println!("{}", json!({"ok":true,"value":value}));
            } else {
                print!("{}", draw(&cli.command, value));
            }
            return Ok(());
        }
        Ok(answer) => {
            let code = answer["reason"]
                .as_str()
                .unwrap_or("device_unavailable")
                .to_owned();
            let message = answer["next_action"].as_str().unwrap_or(&code).to_owned();
            (code, message)
        }
        Err(code) => (code.clone(), code),
    };
    println!(
        "{}",
        json!({"ok":false,"error":{"code":code,"message":message}})
    );
    Err(code)
}

/// The answer as lines for a person.
fn draw(command: &DeviceCommand, value: &Value) -> String {
    match command {
        DeviceCommand::List => {
            let devices = value["devices"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if devices.is_empty() {
                return "No SSH device is registered.\n".to_owned();
            }
            devices.iter().map(device_line).collect()
        }
        DeviceCommand::Add { .. } => format!(
            "Added {}{}\n",
            device_line(&value["device"]),
            value["next_action"].as_str().unwrap_or_default()
        ),
        DeviceCommand::Remove { device_id } => format!(
            "Removed {device_id}. Its panes, agents and files on the device keep running.\n"
        ),
    }
}

fn device_line(device: &Value) -> String {
    let text = |key: &str| device[key].as_str().unwrap_or_default();
    let helper = &device["helper"];
    let helper_state = match (helper["consent"].as_str(), helper["state"].as_str()) {
        (Some("none" | "outdated"), _) => "waits for Allow and install in Settings › Devices",
        (_, Some(state)) => state,
        _ => "unknown",
    };
    let mut line = format!(
        "{}  {}  ssh {}  {}  helper {helper_state}\n",
        text("id"),
        text("label"),
        text("ssh_alias"),
        text("state"),
    );
    if let Some(message) = device["message"].as_str() {
        line.push_str(&format!("  {message}\n"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str]) -> Result<DeviceCli, String> {
        let args: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
        parse(args.iter())
    }

    #[test]
    fn add_takes_an_alias_with_an_optional_name_and_socket() {
        assert_eq!(
            parse_words(&[
                "add",
                "studio",
                "--name",
                "Studio",
                "--socket",
                "/tmp/h.sock"
            ]),
            Ok(DeviceCli {
                command: DeviceCommand::Add {
                    ssh_alias: "studio".into(),
                    name: Some("Studio".into()),
                    herdr_socket_path: Some("/tmp/h.sock".into()),
                },
                json: false,
            })
        );
        assert!(parse_words(&["add"]).is_err());
        assert!(parse_words(&["add", "a", "b"]).is_err());
        assert!(parse_words(&["add", "a", "--name"]).is_err());
        assert!(parse_words(&["list", "--name", "x"]).is_err());
        assert!(parse_words(&["remove", "studio", "--socket", "/x"]).is_err());
    }

    #[test]
    fn a_device_without_the_helper_says_where_to_allow_it() {
        let line = device_line(&json!({
            "id": "studio", "label": "Studio", "ssh_alias": "studio", "state": "unavailable",
            "message": null, "helper": {"consent": "none", "state": "not_allowed", "message": null},
        }));
        assert_eq!(
            line,
            "studio  Studio  ssh studio  unavailable  helper waits for Allow and install in Settings › Devices\n"
        );
    }
}
