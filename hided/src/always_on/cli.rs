//! `hide always-on`: what the desktop app and the uninstallers ask of a
//! machine's login item and its Windows firewall rules (PRD
//! device-connect-iroh D-38, D-41, B30, B32).
//!
//! - `status [--json]`: whether the folder is paired, its login item and
//!   marker are in place, its UDP port, and its firewall rules.
//! - `remove`: the login item, its marker and the install root's builds go;
//!   an uninstaller runs it after `hide stop`.
//! - `firewall ask-allow`: the desktop app's one ask for the rules of the
//!   folder's UDP port (chosen once): Windows's permission prompt runs
//!   `firewall allow --port <n>` elevated, and one line says whether the
//!   operator allowed it.
//! - `firewall allow --port <n>` and `firewall remove`: the rules, run by an
//!   administrator.
//! - `firewall ask-remove`: the uninstaller's removal of the rules, which
//!   asks Windows to run `firewall remove` elevated when any rule is there,
//!   and fails, saying the rules were left, when the operator declines.

use std::path::Path;

use serde_json::json;

pub const USAGE: &str = "usage: hide always-on status [--json]
       hide always-on remove
       hide always-on firewall ask-allow
       hide always-on firewall allow --port <n> [--retire <n>]
       hide always-on firewall remove
       hide always-on firewall ask-remove";

#[derive(Debug, Eq, PartialEq)]
pub enum Command {
    Status { json: bool },
    Remove,
    FirewallAskAllow,
    FirewallAllow { port: u16, retired: Option<u16> },
    FirewallRemove,
    FirewallAskRemove,
}

/// The words after `hide always-on`.
pub fn parse(words: &[String]) -> Result<Command, String> {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["status"] => Ok(Command::Status { json: false }),
        ["status", "--json"] => Ok(Command::Status { json: true }),
        ["remove"] => Ok(Command::Remove),
        ["firewall", "ask-allow"] => Ok(Command::FirewallAskAllow),
        ["firewall", "allow", "--port", port] => port
            .parse()
            .map(|port| Command::FirewallAllow {
                port,
                retired: None,
            })
            .map_err(|_| USAGE.to_owned()),
        ["firewall", "allow", "--port", port, "--retire", retired] => {
            match (port.parse(), retired.parse()) {
                (Ok(port), Ok(retired)) => Ok(Command::FirewallAllow {
                    port,
                    retired: Some(retired),
                }),
                _ => Err(USAGE.to_owned()),
            }
        }
        ["firewall", "remove"] => Ok(Command::FirewallRemove),
        ["firewall", "ask-remove"] => Ok(Command::FirewallAskRemove),
        _ => Err(USAGE.to_owned()),
    }
}

pub fn run(command: &Command, home: &Path, state_dir: &Path) -> Result<(), String> {
    match command {
        Command::Status { json } => {
            let status = status(home, state_dir)?;
            if *json {
                println!("{status}");
            } else {
                println!(
                    "paired: {}\nlogin item: {}\nkeep-alive marker: {}\nfirewall: {}",
                    status["paired"],
                    status["login_item"],
                    status["keep_alive"],
                    status["firewall"]
                );
            }
            Ok(())
        }
        Command::Remove => remove(home, state_dir),
        Command::FirewallAskAllow => {
            println!("{}", json!({ "answer": ask_to_allow(state_dir)? }));
            Ok(())
        }
        Command::FirewallAllow { port, retired } => {
            hide_platform::firewall::allow(*port, *retired).map_err(|error| error.to_string())
        }
        Command::FirewallRemove => {
            hide_platform::firewall::remove_all().map_err(|error| error.to_string())
        }
        Command::FirewallAskRemove => ask_to_remove_rules(),
    }
}

/// The rules for the folder's port, through Windows's permission prompt
/// unless they are in place: `allowed`, `declined` or `failed`, or
/// `not_used` outside Windows.
fn ask_to_allow(state_dir: &Path) -> Result<&'static str, String> {
    use hide_platform::firewall::{self, Elevated};
    let port = hide_peer::port::of(state_dir).map_err(|error| error.to_string())?;
    match firewall::allowed(port) {
        Ok(true) => return Ok("allowed"),
        Ok(false) => {}
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => return Ok("not_used"),
        Err(error) => return Err(error.to_string()),
    }
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    // The rule for a port the folder held before its port was taken goes
    // with this allow (`hide_peer::port::replace`).
    let retired = hide_peer::port::replaced(state_dir).map_err(|error| error.to_string())?;
    let port = port.to_string();
    let retired_word = retired.map(|retired| retired.to_string());
    let mut arguments = vec!["always-on", "firewall", "allow", "--port", &port];
    if let Some(retired) = &retired_word {
        arguments.extend(["--retire", retired]);
    }
    let answer = firewall::run_elevated(&program, &arguments).map_err(|error| error.to_string())?;
    if answer == Elevated::Done {
        hide_peer::port::retired(state_dir).map_err(|error| error.to_string())?;
    }
    Ok(match answer {
        Elevated::Done => "allowed",
        Elevated::Declined => "declined",
        Elevated::Failed(code) => {
            hide_core::diagnostic!(json!({
                "component": "always_on",
                "kind": "firewall.allow_failed",
                "exit": code,
            }));
            "failed"
        }
    })
}

/// The rules go through Windows's permission prompt when any are there; a
/// declined prompt leaves them, which the uninstaller says in one line.
fn ask_to_remove_rules() -> Result<(), String> {
    use hide_platform::firewall::{self, Elevated};
    match firewall::any() {
        Ok(false) => return Ok(()),
        Ok(true) => {}
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => return Ok(()),
        Err(error) => return Err(error.to_string()),
    }
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    match firewall::run_elevated(&program, &["always-on", "firewall", "remove"])
        .map_err(|error| error.to_string())?
    {
        Elevated::Done => Ok(()),
        Elevated::Declined => Err(format!(
            "Hide's firewall rules were left in place: remove the rule group {} in Windows Defender Firewall's inbound rules",
            firewall::GROUP
        )),
        Elevated::Failed(code) => Err(format!(
            "Hide's firewall rules were left in place: their removal exited {code:?}"
        )),
    }
}

fn status(home: &Path, state_dir: &Path) -> Result<serde_json::Value, String> {
    let agents = super::agents_for(home);
    let port = hide_peer::port::read(&hide_kit::layout::peer_port(state_dir))
        .map_err(|error| error.to_string())?;
    let firewall = match port.map(hide_platform::firewall::allowed) {
        None => "no_port".to_owned(),
        Some(Ok(true)) => "allowed".to_owned(),
        Some(Ok(false)) => "missing".to_owned(),
        Some(Err(error)) if error.kind() == std::io::ErrorKind::Unsupported => {
            "not_used".to_owned()
        }
        Some(Err(error)) => return Err(error.to_string()),
    };
    Ok(json!({
        "paired": super::paired(state_dir)?,
        "login_item": crate::login_item::installed(&agents, home, state_dir)?,
        "keep_alive": crate::login_item::marker(state_dir).exists(),
        "udp_port": port,
        "firewall": firewall,
    }))
}

/// The login item and its marker, then the install root's builds; the core
/// they ran was stopped first (`hide stop`), so nothing runs from them.
fn remove(home: &Path, state_dir: &Path) -> Result<(), String> {
    let agents = super::agents_for(home);
    crate::login_item::remove(&agents, home, state_dir)?;
    let root = hide_kit::layout::core_root(home);
    match std::fs::remove_dir_all(&root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{}: {error}", root.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn each_verb_takes_only_its_words() {
        assert_eq!(
            parse(&words(&["status", "--json"])),
            Ok(Command::Status { json: true })
        );
        assert_eq!(
            parse(&words(&["firewall", "allow", "--port", "51234"])),
            Ok(Command::FirewallAllow {
                port: 51234,
                retired: None
            })
        );
        assert_eq!(
            parse(&words(&[
                "firewall", "allow", "--port", "51234", "--retire", "50000"
            ])),
            Ok(Command::FirewallAllow {
                port: 51234,
                retired: Some(50000)
            })
        );
        for refused in [
            &["firewall", "allow"][..],
            &["firewall", "allow", "--port", "x"],
            &["firewall", "allow", "--port", "51234", "--retire", "x"],
            &["remove", "--all"],
            &[],
        ] {
            assert_eq!(parse(&words(refused)), Err(USAGE.to_owned()), "{refused:?}");
        }
    }

    /// `remove` takes the item, its marker and the builds; a folder that
    /// never paired has nothing to take and answers the same.
    #[test]
    fn remove_takes_the_marker_and_the_builds() {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        remove(home.path(), &state).unwrap();
        crate::login_item::keep(&state).unwrap();
        let build = hide_kit::layout::core_root(home.path()).join("0123456789abcdef");
        std::fs::create_dir_all(&build).unwrap();
        remove(home.path(), &state).unwrap();
        assert!(!crate::login_item::marker(&state).exists());
        assert!(!hide_kit::layout::core_root(home.path()).exists());
    }
}
