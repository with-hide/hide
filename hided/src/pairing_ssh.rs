//! Pairing with a machine this one reaches over SSH (PRD device-connect-iroh
//! B7, B50, D-22; security spec 15): the two commands this machine runs on
//! the target through the system `ssh`, on the session `hide_node`'s
//! upstream signs in, and how their answers read.
//!
//! `hide device look-ssh` changes nothing there and says whether a Hide of
//! the target's own runs; `hide device accept-ssh` has that Hide save a
//! pending grant for this machine and answer what this machine confirms
//! over the connection. Nothing is installed: a target with no Hide of its
//! own is answered as such.
//!
//! A dial here asks nobody. A login that needs a password, a passphrase, a
//! code, an unknown host key or a key touch ends as not reached, and the
//! operator pairs by a link instead (B7): keys, the agent and Tailscale SSH
//! still sign in.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use hide_node::ssh::session::{SessionError, SshTarget};
use hide_node::ssh::upstream::Upstream;
use hide_peer::pairing::{Accepted, Me};
use hide_peer::protocol::{Candidate, PeerKey};
use serde::Serialize;
use serde_json::Value;

/// The folder an account links its `hide` command in when none is named.
const COMMAND_FOLDER: &str = "~/.local/bin";

/// The most of an answer read: an accepted pairing with its addresses is
/// well under it.
const ANSWER_CAP: usize = 64 * 1024;

/// How long the target's `hide` may take to answer once ssh signed in: its
/// own ask of its daemon waits at most thirty seconds.
const ANSWER_WITHIN: Duration = Duration::from_secs(40);

/// The addresses of the SSH login tried before the target's own (D-22).
const LOGIN_ADDRESSES: usize = 2;

/// How this machine reaches a target's `hide` over SSH.
#[derive(Clone, Debug)]
pub struct Reach {
    /// The account whose SSH configuration names the alias.
    pub home: PathBuf,
    /// The folder the target's account links `hide` in
    /// (`HIDE_HOST_CLI_DIR`); none is `~/.local/bin`.
    pub command_folder: Option<String>,
}

/// What a target said about pairing over SSH, before anything is asked of
/// it (B7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Looked {
    /// Its own Hide runs: pairing only connects, and installs nothing.
    Ready,
    /// It has no Hide of its own.
    Absent,
    /// Its Hide is too old to pair over SSH.
    UpdateThere,
    /// Its Hide is installed and does not run: on a Mac, nobody is logged
    /// in.
    NotRunning { system: String },
    /// SSH did not sign in without asking, or the machine was not reached.
    NotReached,
}

/// Whether `alias` is a name `ssh` reads as a destination and never as an
/// option.
pub fn valid_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias.len() <= 255
        && !alias.starts_with('-')
        && !alias
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

/// The words that start the target's `hide` in a run step. A folder under
/// the account's home is read from that account's own `HOME` on a POSIX
/// system; a Windows session starts in the profile folder, which a path
/// under the home is relative to.
fn program(reach: &Reach, system: &str) -> Vec<String> {
    let folder = reach.command_folder.as_deref().unwrap_or(COMMAND_FOLDER);
    let under_home = folder.strip_prefix("~/");
    if system == "windows" {
        let folder = under_home.unwrap_or(folder).replace('/', "\\");
        return vec![format!("{folder}\\hide.cmd")];
    }
    match under_home {
        Some(folder) => vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "exec \"$HOME/$0\" \"$@\"".to_owned(),
            format!("{folder}/hide"),
        ],
        None => vec![format!("{folder}/hide")],
    }
}

/// How one command on the target ended.
enum Said {
    /// Its answer line.
    Answer(Value),
    /// No `hide` is there to run.
    NoProgram,
    /// A `hide` ran and answered nothing this build reads: an older one.
    Unread,
}

fn said(status: i32, stdout: &[u8]) -> Said {
    // 127 is a command that is not there and 126 one that cannot be run, on
    // a POSIX shell and from the Windows start script alike.
    if status == 126 || status == 127 {
        return Said::NoProgram;
    }
    let text = String::from_utf8_lossy(stdout);
    let line = text.lines().rev().find(|line| !line.trim().is_empty());
    match line.and_then(|line| serde_json::from_str::<Value>(line).ok()) {
        Some(answer) if answer["ok"].is_boolean() => Said::Answer(answer),
        _ => Said::Unread,
    }
}

/// Runs `hide device <words>` on the target, on `upstream`'s session.
fn run(upstream: &Upstream, reach: &Reach, words: &[&str]) -> Result<Said, SessionError> {
    let system = upstream.system()?;
    let mut argv = program(reach, &system);
    argv.push("device".to_owned());
    argv.extend(words.iter().map(|word| (*word).to_owned()));
    let ran = upstream.run(&argv, ANSWER_CAP, ANSWER_WITHIN)?;
    Ok(said(ran.status, &ran.stdout))
}

fn look_of(said: Said) -> Looked {
    match said {
        Said::NoProgram => Looked::Absent,
        Said::Unread => Looked::UpdateThere,
        Said::Answer(answer) if answer["ok"] == true => Looked::Ready,
        Said::Answer(answer) => match answer.get("look") {
            // A `hide` of a device helper answers for an account that never
            // ran a Hide of its own.
            Some(look) if look["core"] == false => Looked::Absent,
            Some(look) => Looked::NotRunning {
                system: look["system"].as_str().unwrap_or_default().to_owned(),
            },
            // A refusal with no look is an older Hide's answer to a command
            // it does not know.
            None => Looked::UpdateThere,
        },
    }
}

/// Asks the machine `alias` names whether it can pair over SSH. Blocks for
/// as long as ssh takes to sign in and the target to answer.
pub fn look(reach: &Reach, alias: &str) -> Looked {
    let upstream = Upstream::new(SshTarget::new(&reach.home, alias));
    let looked = match run(&upstream, reach, &["look-ssh"]) {
        Ok(said) => look_of(said),
        Err(error) => {
            log(alias, "look", &error);
            Looked::NotReached
        }
    };
    upstream.close();
    looked
}

/// What the target answered to a pairing, and the addresses of the SSH
/// login to try before its own.
pub struct Paired {
    pub accepted: Accepted,
    pub first: Vec<Candidate>,
}

/// Has the target's Hide save a pending grant for this machine (`me`,
/// `key`) under `request`, and answers what it said. The same request run
/// again is answered the same there. A refusal is its reason code.
pub fn accept(
    reach: &Reach,
    alias: &str,
    request: &str,
    me: &Me,
    key: &PeerKey,
) -> Result<Paired, &'static str> {
    let upstream = Upstream::new(SshTarget::new(&reach.home, alias));
    let key = key.to_hex();
    let ran = run(
        &upstream,
        reach,
        &[
            "accept-ssh",
            "--request",
            request,
            "--key",
            &key,
            "--device",
            &me.device,
            "--name",
            &me.name,
        ],
    );
    let login = upstream.direct_host();
    upstream.close();
    let answer = match ran {
        Ok(Said::Answer(answer)) => answer,
        Ok(Said::NoProgram) => return Err("ssh_hide_absent"),
        Ok(Said::Unread) => return Err("ssh_update_there"),
        Err(error) => {
            log(alias, "accept", &error);
            return Err("ssh_not_reached");
        }
    };
    if answer["ok"] != true {
        return Err(match answer["error"]["code"].as_str() {
            Some("device_already_connected") => "device_already_connected",
            Some("device_limit") => "device_limit",
            Some("hide_unavailable" | "request_timeout") => "ssh_hide_not_running",
            Some("invalid_request") => "invalid_request",
            _ => "ssh_update_there",
        });
    }
    let accepted: Accepted = serde_json::from_value(answer["value"]["accepted"].clone())
        .map_err(|_| "ssh_update_there")?;
    let first = login
        .map(|host| login_addresses(&host, &accepted.candidates))
        .unwrap_or_default();
    Ok(Paired { accepted, first })
}

/// The addresses the SSH login reached, at the port the target's endpoint
/// listens on: the first direct candidates of a machine paired over SSH
/// (D-22). None when the target named no port of its own.
fn login_addresses(host: &str, own: &[Candidate]) -> Vec<Candidate> {
    let Some(port) = own.iter().find_map(|candidate| match candidate {
        Candidate::Ip(address) => Some(address.port()),
        Candidate::Relay(_) => None,
    }) else {
        return Vec::new();
    };
    let Ok(found) = (host, port).to_socket_addrs() else {
        return Vec::new();
    };
    found
        .take(LOGIN_ADDRESSES)
        .map(|address: SocketAddr| Candidate::Ip(address))
        .collect()
}

fn log(alias: &str, step: &str, error: &SessionError) {
    hide_core::diagnostic!(serde_json::json!({
        "component": "pairing",
        "kind": "ssh.not_reached",
        "step": step,
        "alias": alias,
        "reason": error.to_string(),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(value: Value) -> Vec<u8> {
        format!("{value}\n").into_bytes()
    }

    /// Each way a target answers the look reads as the one state its
    /// confirm shows (B7).
    #[test]
    fn a_look_reads_each_answer_as_its_state() {
        assert_eq!(look_of(said(127, b"")), Looked::Absent);
        assert_eq!(look_of(said(126, b"")), Looked::Absent);
        assert_eq!(
            look_of(said(
                0,
                &line(json!({"ok":true,"value":{"look":{"system":"macos","core":true}}}))
            )),
            Looked::Ready
        );
        // A Hide that is installed and does not run.
        assert_eq!(
            look_of(said(
                1,
                &line(
                    json!({"ok":false,"error":{"code":"hide_unavailable"},"look":{"system":"macos","core":true}})
                )
            )),
            Looked::NotRunning {
                system: "macos".to_owned()
            }
        );
        // A device helper's `hide`, on an account with no Hide of its own.
        assert_eq!(
            look_of(said(
                1,
                &line(
                    json!({"ok":false,"error":{"code":"hide_unavailable"},"look":{"system":"linux","core":false}})
                )
            )),
            Looked::Absent
        );
        // An older Hide: a usage line, or a refusal that names no look.
        assert_eq!(
            look_of(said(2, b"usage: hide device list\n")),
            Looked::UpdateThere
        );
        assert_eq!(
            look_of(said(1, &line(json!({"ok":false,"error":{"code":"usage"}})))),
            Looked::UpdateThere
        );
    }

    /// The command folder is spelled for the system that runs it, from the
    /// account's home.
    #[test]
    fn the_targets_hide_is_spelled_for_its_system() {
        let default = Reach {
            home: PathBuf::from("/Users/example"),
            command_folder: None,
        };
        assert_eq!(
            program(&default, "macos"),
            ["sh", "-c", "exec \"$HOME/$0\" \"$@\"", ".local/bin/hide"]
        );
        assert_eq!(program(&default, "windows"), [".local\\bin\\hide.cmd"]);
        let named = Reach {
            home: PathBuf::from("/Users/example"),
            command_folder: Some("/opt/hide/bin".to_owned()),
        };
        assert_eq!(program(&named, "linux"), ["/opt/hide/bin/hide"]);
    }

    /// The SSH login's address is tried at the target endpoint's own port,
    /// and a target that named none gets no guess.
    #[test]
    fn the_login_address_takes_the_targets_port() {
        let own = [
            Candidate::Relay("https://relay.example".to_owned()),
            Candidate::Ip("10.0.0.7:7842".parse().unwrap()),
        ];
        assert_eq!(
            login_addresses("127.0.0.1", &own),
            vec![Candidate::Ip("127.0.0.1:7842".parse().unwrap())]
        );
        assert_eq!(login_addresses("127.0.0.1", &own[..1]), Vec::new());
    }

    #[test]
    fn an_alias_ssh_would_read_as_an_option_is_refused() {
        assert!(valid_alias("studio"));
        assert!(valid_alias("build.box-2"));
        assert!(!valid_alias("-oProxyCommand=x"));
        assert!(!valid_alias("two words"));
        assert!(!valid_alias(""));
    }
}
