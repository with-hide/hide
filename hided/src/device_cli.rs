//! `hide device list|add|remove|retry`: the SSH devices of Settings ›
//! Devices, for an agent the operator asked to add, connect or remove one.
//! The core answers (`herdr_core::device_command`); this file reads the
//! words, waits on a retry's outcome when asked, and prints the answer, as
//! one JSON line with `--json` and as lines for a person without.
use crate::env::Env;
use herdr_core::device_command::DeviceCommand;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub const USAGE: &str = "hide device list [--json]\nhide device add <ssh alias> [--name <name>] [--socket <path>] [--json]\nhide device remove <device id> [--json]\nhide device retry <device id> [--wait] [--json]";

/// How long `retry --wait` waits for an attempt to connect, reach the
/// operator's turn or fail: past ssh connecting and a first install of
/// Hide's helper over a slow link. What waits on the operator returns at
/// once, so this bounds only the attempt's own work.
const WAIT_BOUND: Duration = Duration::from_secs(300);
const WAIT_POLL: Duration = Duration::from_millis(500);

#[derive(Debug, Eq, PartialEq)]
pub struct DeviceCli {
    pub command: DeviceCommand,
    pub json: bool,
    /// `retry --wait`: answer the attempt's outcome, not its start.
    pub wait: bool,
}

pub fn parse<'a>(mut args: impl Iterator<Item = &'a String>) -> Result<DeviceCli, String> {
    let verb = args.next().map(String::as_str);
    let mut json = false;
    let mut wait = false;
    let mut name = None;
    let mut socket = None;
    let mut words = Vec::new();
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "--json" if !json => {
                json = true;
                continue;
            }
            "--wait" if verb == Some("retry") && !wait => {
                wait = true;
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
        (Some("retry"), [device_id]) => DeviceCommand::Retry {
            device_id: device_id.clone(),
        },
        _ => return Err(USAGE.to_owned()),
    };
    if !command.valid() {
        return Err(USAGE.to_owned());
    }
    Ok(DeviceCli {
        command,
        json,
        wait,
    })
}

pub fn run(env: &Env, cli: &DeviceCli) -> Result<(), String> {
    let answer = (|| {
        let mut value = ask(env, &cli.command)?;
        if let (true, DeviceCommand::Retry { device_id }) = (cli.wait, &cli.command) {
            value["device"] = outcome(env, device_id, value["device"].clone())?;
        }
        Ok(value)
    })();
    let (code, message, device) = match answer {
        Ok(value) => {
            if cli.json {
                println!("{}", json!({"ok":true,"value":value}));
            } else {
                print!("{}", draw(&cli.command, &value));
            }
            return Ok(());
        }
        Err(Refused::Answer { code, message }) => (code, message, None),
        Err(Refused::Unreachable(code)) => {
            let message = crate::cli::request_next_action(&code).to_owned();
            (code, message, None)
        }
        Err(Refused::WaitedOut(device)) => (
            "device_wait_timeout".to_owned(),
            format!(
                "The attempt is still connecting after {} seconds; run hide device retry {} --wait again to keep waiting",
                WAIT_BOUND.as_secs(),
                device["id"].as_str().unwrap_or_default()
            ),
            Some(device),
        ),
    };
    let mut line = json!({"ok":false,"error":{"code":code,"message":message}});
    if let Some(device) = device {
        line["device"] = device;
    }
    println!("{line}");
    Err(code)
}

enum Refused {
    /// The core's refusal, with what the caller does about it.
    Answer { code: String, message: String },
    /// No answer arrived, for this reason code.
    Unreachable(String),
    /// `retry --wait` reached its bound while the attempt still ran; the
    /// device as last read.
    WaitedOut(Value),
}

/// One request on its own credential: a bare `hide`'s one-shot credential
/// ends with its answer, so each read of a wait takes a new one.
fn ask(env: &Env, command: &DeviceCommand) -> Result<Value, Refused> {
    let answer = crate::workspace_cli::Credential::acquire(env)
        .and_then(|mut credential| crate::workspace_cli::request_device(&mut credential, command))
        .map_err(Refused::Unreachable)?;
    if answer["ok"] == true {
        return Ok(answer["result"].clone());
    }
    let code = answer["reason"]
        .as_str()
        .unwrap_or("device_unavailable")
        .to_owned();
    let message = answer["next_action"].as_str().unwrap_or(&code).to_owned();
    Err(Refused::Answer { code, message })
}

/// Reads the device until its attempt has an outcome: connected, the
/// operator's turn or failed (PRD system-ssh-device-link B30).
#[allow(clippy::disallowed_methods)] // polls a state the core changes on its own worker, bounded by WAIT_BOUND
fn outcome(env: &Env, device_id: &str, mut device: Value) -> Result<Value, Refused> {
    let deadline = Instant::now() + WAIT_BOUND;
    while device["state"] == "connecting" {
        if Instant::now() >= deadline {
            return Err(Refused::WaitedOut(device));
        }
        std::thread::sleep(WAIT_POLL);
        let list = ask(env, &DeviceCommand::List)?;
        device = list["devices"]
            .as_array()
            .and_then(|devices| devices.iter().find(|device| device["id"] == device_id))
            .cloned()
            // Removed while the attempt ran.
            .ok_or_else(|| Refused::Answer {
                code: "device_unknown".to_owned(),
                message: herdr_core::device_command::next_action("device_unknown").to_owned(),
            })?;
    }
    Ok(device)
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
        DeviceCommand::Retry { .. } => format!(
            "{}{}",
            if value["started"] == true {
                "Started an attempt: "
            } else {
                "No new attempt: "
            },
            device_line(&value["device"])
        ),
    }
}

fn device_line(device: &Value) -> String {
    let text = |key: &str| device[key].as_str().unwrap_or_default();
    let mut line = format!(
        "{}  {}  ssh {}  {}\n",
        text("id"),
        text("label"),
        text("ssh_alias"),
        text("state").replace('_', " "),
    );
    let waits_on = &device["waits_on"];
    if let Some(kind) = waits_on["kind"].as_str() {
        line.push_str(&format!("  waits on {}", kind.replace('_', " ")));
        if let Some(input) = waits_on["input"].as_str() {
            line.push_str(&format!(" ({input})"));
        }
        line.push('\n');
    }
    let failure = &device["failure"];
    if let Some(action) = failure["action"].as_str() {
        line.push_str(&format!("  failed: {}", action.replace('_', " ")));
        if let Some(cause) = failure["cause"].as_str() {
            line.push_str(&format!(" ({cause})"));
        }
        if let Some(detail) = failure["detail"].as_str() {
            line.push_str(&format!(" - {detail}"));
        }
        line.push('\n');
    }
    if let Some(next) = device["next_action"].as_str() {
        line.push_str(&format!("  next: {next}\n"));
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
                wait: false,
            })
        );
        assert!(parse_words(&["add"]).is_err());
        assert!(parse_words(&["add", "a", "b"]).is_err());
        assert!(parse_words(&["add", "a", "--name"]).is_err());
        assert!(parse_words(&["list", "--name", "x"]).is_err());
        assert!(parse_words(&["remove", "studio", "--socket", "/x"]).is_err());
    }

    #[test]
    fn retry_takes_one_device_id_and_waits_only_when_asked() {
        let retry = |wait| {
            Ok(DeviceCli {
                command: DeviceCommand::Retry {
                    device_id: "studio".into(),
                },
                json: true,
                wait,
            })
        };
        assert_eq!(parse_words(&["retry", "studio", "--json"]), retry(false));
        assert_eq!(
            parse_words(&["retry", "--wait", "studio", "--json"]),
            retry(true)
        );
        assert!(parse_words(&["retry"]).is_err());
        assert!(parse_words(&["retry", "studio", "--wait", "--wait"]).is_err());
        assert!(parse_words(&["list", "--wait"]).is_err());
        // Nothing answers what the operator answers on the row.
        for verb in ["allow", "trust", "answer", "sign-in"] {
            assert!(parse_words(&[verb, "studio"]).is_err(), "{verb}");
        }
    }

    /// A sign-in failure carries no ssh line (the core leaves it out), so
    /// the lines for a person show only its action, cause and next step.
    #[test]
    fn a_sign_in_failure_shows_its_action_without_a_detail() {
        let line = device_line(&json!({
            "id": "studio", "label": "Studio", "ssh_alias": "studio", "state": "failed",
            "waits_on": null,
            "failure": {"action": "sign_in", "cause": "sign_in_refused", "detail": null},
            "next_action": "Run hide device retry studio --wait: it signs in again and may ask the operator on Studio's row in Settings > Devices",
        }));
        assert_eq!(
            line,
            "studio  Studio  ssh studio  failed\n  failed: sign in (sign_in_refused)\n  next: Run hide device retry studio --wait: it signs in again and may ask the operator on Studio's row in Settings > Devices\n"
        );
    }

    #[test]
    fn a_device_says_what_it_waits_on_and_what_to_do() {
        let line = device_line(&json!({
            "id": "studio", "label": "Studio", "ssh_alias": "studio", "state": "operator_turn",
            "waits_on": {"kind": "sign_in", "input": "secret"}, "failure": null,
            "next_action": "Ask the operator to type the password or code ssh asks for on Studio's row in Settings > Devices",
        }));
        assert_eq!(
            line,
            "studio  Studio  ssh studio  operator turn\n  waits on sign in (secret)\n  next: Ask the operator to type the password or code ssh asks for on Studio's row in Settings > Devices\n"
        );
        let line = device_line(&json!({
            "id": "studio", "label": "Studio", "ssh_alias": "studio", "state": "failed",
            "waits_on": null,
            "failure": {"action": "retry", "cause": "unclassified", "detail": "kex_exchange_identification: Connection closed"},
            "next_action": "Fix what detail names, or ask the operator to, then run hide device retry studio",
        }));
        assert_eq!(
            line,
            "studio  Studio  ssh studio  failed\n  failed: retry (unclassified) - kex_exchange_identification: Connection closed\n  next: Fix what detail names, or ask the operator to, then run hide device retry studio\n"
        );
    }
}
