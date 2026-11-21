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
//! A look or a pairing the Add dialog starts asks nobody. A login that needs
//! a password, a passphrase, a code, an unknown host key or a key touch
//! ends as not reached, and the operator pairs by a link instead (B7): keys,
//! the agent and Tailscale SSH still sign in. A helper device's switch
//! ([`switch`]) is the operator's press on that device's row, so its one
//! login asks there as the device's own connection does (D-45).

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

/// How long the target's `projects-ssh` may take to answer: its own work
/// ends before that (`device_cli::PROJECTS_BOUND`).
const PROJECTS_WITHIN: Duration = Duration::from_secs(60);

/// The most a switch's commands on the target wait for in all, apart from
/// the login's own questions and the wait for the new connection: two
/// looks, the start, the pairing, what the started Hide needs, and the
/// projects.
pub const SWITCH_SSH_WITHIN: Duration =
    Duration::from_secs(5 * ANSWER_WITHIN.as_secs() + PROJECTS_WITHIN.as_secs());

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
    /// The words that start the target's `hide`, for a caller that knows
    /// where it is: the node of a core moved before 1.2.0 runs the `hide`
    /// beside that core's program (`moved_core`). None reads the command
    /// folder.
    pub hide: Option<Vec<String>>,
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
    if let Some(hide) = &reach.hide {
        return hide.clone();
    }
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
    let mut device = vec!["device"];
    device.extend(words);
    run_hide(upstream, reach, &device)
}

/// Runs `hide <words>` on the target, on `upstream`'s session.
fn run_hide(upstream: &Upstream, reach: &Reach, words: &[&str]) -> Result<Said, SessionError> {
    run_hide_within(upstream, reach, words, ANSWER_WITHIN)
}

/// Runs `hide <words>` on the target, on `upstream`'s session, waiting
/// `within` for its answer.
fn run_hide_within(
    upstream: &Upstream,
    reach: &Reach,
    words: &[&str],
    within: Duration,
) -> Result<Said, SessionError> {
    let system = upstream.system()?;
    let mut argv = program(reach, &system);
    argv.extend(words.iter().map(|word| (*word).to_owned()));
    let ran = upstream.run(&argv, ANSWER_CAP, within)?;
    Ok(said(ran.status, &ran.stdout))
}

/// Whether the target said nobody is logged in to its desktop
/// (`core_install::checks::desktop_session`, in its look): `None` when its
/// answer does not say, as an older Hide's or an older helper's does not.
fn desktop_of(said: &Said) -> Option<bool> {
    match said {
        Said::Answer(answer) => answer
            .get("look")
            .or_else(|| answer.get("value").and_then(|value| value.get("look")))
            .and_then(|look| look["desktop"].as_bool()),
        Said::NoProgram | Said::Unread => None,
    }
}

/// Why this machine starts no Hide on a target that runs `target` (PRD
/// device-connect-iroh D-45, B7): a Mac takes one only from a Mac, and only
/// while someone is logged in to its desktop, where its login item runs.
/// A target that did not say (`desktop` is `None`) is not refused.
fn install_refusal(target: &str, from: &str, desktop: Option<bool>) -> Option<&'static str> {
    if target != "macos" {
        return None;
    }
    if from != "macos" {
        return Some("ssh_mac_from_mac_only");
    }
    (desktop == Some(false)).then_some("ssh_mac_signed_out")
}

/// Starts a Hide of the target account's own from the `hide` this
/// machine's device helper linked there, which runs the build beside it
/// (D-45): the account then has a core of this machine's build on its own
/// state folder, and that core keeps itself running from a login item once
/// it is paired (`always_on`). What it printed is its own window's to
/// read, and is dropped.
fn start_core(upstream: &Upstream, reach: &Reach, alias: &str) -> Result<(), &'static str> {
    match run_hide(upstream, reach, &["connect"]) {
        Ok(Said::Answer(answer)) if answer["ok"] == true => Ok(()),
        Ok(Said::NoProgram) => Err("ssh_hide_absent"),
        Ok(_) => Err("ssh_core_not_started"),
        Err(error) => {
            log(alias, "start", &error);
            Err("ssh_not_reached")
        }
    }
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

/// The core a target runs, as it says of itself in its look: its node, and
/// its build and release as its node link's hello would name them.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq)]
pub struct CoreThere {
    pub node: String,
    pub build: String,
    #[serde(default)]
    pub release: crate::build_order::Release,
}

/// Reads the core the target runs, on `upstream`'s session, and changes
/// nothing there. `None` from a target that says nothing of it: a Hide
/// before 1.2.0, one that is not running, or none.
pub fn core_on(upstream: &Upstream, reach: &Reach) -> Result<Option<CoreThere>, SessionError> {
    Ok(core_of(run(upstream, reach, &["look-ssh"])?))
}

fn core_of(said: Said) -> Option<CoreThere> {
    match said {
        Said::Answer(answer) if answer["ok"] == true => {
            serde_json::from_value(answer["value"]["look"].clone()).ok()
        }
        Said::Answer(_) | Said::NoProgram | Said::Unread => None,
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
#[derive(Clone)]
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
    let asked = Accept {
        alias,
        request,
        me,
        key,
        helper: None,
        grant: None,
    };
    let answer = accept_on(&upstream, reach, &asked);
    let login = upstream.direct_host();
    upstream.close();
    paired(&answer?["accepted"], login).ok_or("ssh_update_there")
}

/// What one `accept-ssh` asks of the target `alias` names.
pub(crate) struct Accept<'a> {
    pub(crate) alias: &'a str,
    pub(crate) request: &'a str,
    pub(crate) me: &'a Me,
    pub(crate) key: &'a PeerKey,
    /// The root this machine's device helper is installed under there,
    /// when it also asks for that helper's folder (D-49).
    pub(crate) helper: Option<&'a str>,
    /// The grant this machine gives the target, which then becomes this
    /// machine's guest too: only the core this machine dialed as a node
    /// takes one (D-46).
    pub(crate) grant: Option<&'a str>,
}

/// Runs the target's `accept-ssh` on `upstream`'s session and answers the
/// value it printed.
pub(crate) fn accept_on(
    upstream: &Upstream,
    reach: &Reach,
    asked: &Accept<'_>,
) -> Result<Value, &'static str> {
    let (alias, me) = (asked.alias, asked.me);
    let key = asked.key.to_hex();
    let mut words = vec![
        "accept-ssh",
        "--request",
        asked.request,
        "--key",
        &key,
        "--device",
        &me.device,
        "--name",
        &me.name,
    ];
    if let Some(root) = asked.helper {
        words.extend(["--helper", root]);
    }
    if let Some(grant) = asked.grant {
        words.extend(["--grant", grant]);
    }
    let mut answer = match run(upstream, reach, &words) {
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
            Some("device_unknown") => "device_unknown",
            _ => "ssh_update_there",
        });
    }
    Ok(answer["value"].take())
}

/// The pairing a target's answer carries, with the addresses its SSH login
/// reached.
fn paired(accepted: &Value, login: Option<String>) -> Option<Paired> {
    let accepted: Accepted = serde_json::from_value(accepted.clone()).ok()?;
    let first = login
        .map(|host| login_addresses(&host, &accepted.candidates))
        .unwrap_or_default();
    Some(Paired { accepted, first })
}

/// What became of the folder of the device helper this machine dialed on
/// the target (D-49).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Folder {
    /// This machine's node goes on in it: its terminals continue.
    Taken,
    /// The target has no helper folder.
    Absent,
    /// This machine's work there has a folder of its own.
    Kept,
}

/// What a Hide started on a machine over SSH needs from the operator to
/// stay reachable with no one at that machine (PRD device-connect-iroh
/// B30).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Needs {
    /// A Mac's core starts at a login, so after a restart it is back only
    /// with automatic login on.
    AutoLogin,
    /// Linux: the account may not turn linger on itself, so the core ends
    /// with its last session until an administrator runs `command`.
    Linger { command: String },
}

impl Needs {
    /// As the core's `device_switch` event carries it.
    pub fn payload(&self) -> Value {
        match self {
            Self::AutoLogin => serde_json::json!({ "kind": "auto_login" }),
            Self::Linger { command } => {
                serde_json::json!({ "kind": "linger", "command": command })
            }
        }
    }
}

/// What the Hide just started on the target needs: read from the system it
/// runs on, and on Linux from its own `hide always-on linger`, which turns
/// linger on when the account may. A target that does not answer needs
/// nothing this machine can name; the switch goes on either way.
fn needs_of(upstream: &Upstream, reach: &Reach, alias: &str) -> Option<Needs> {
    match upstream.system().ok()?.as_str() {
        "macos" => Some(Needs::AutoLogin),
        "linux" => match run_hide(upstream, reach, &["always-on", "linger", "--json"]) {
            Ok(Said::Answer(answer)) => linger_needs(&answer),
            Ok(_) => None,
            Err(error) => {
                log(alias, "linger", &error);
                None
            }
        },
        _ => None,
    }
}

/// The command an administrator runs, when the target's answer names one.
fn linger_needs(answer: &Value) -> Option<Needs> {
    let value = &answer["value"];
    (answer["ok"] == true && value["linger"] == "needs_administrator")
        .then(|| value["command"].as_str())
        .flatten()
        .map(|command| Needs::Linger {
            command: command.to_owned(),
        })
}

/// How far a helper device's switch got on the SSH login alone.
pub enum Switched {
    /// The target's Hide saved a grant for this machine, or holds one
    /// already (`paired` is none): the connection confirms it.
    Paired {
        paired: Option<Paired>,
        folder: Folder,
        /// What the Hide this switch started there still needs from the
        /// operator to stay reachable; none for a Hide that ran already.
        needs: Option<Needs>,
    },
    /// Another machine took the helper's folder first, named as the target
    /// knows it: nothing was saved, and the operator is asked before this
    /// machine's terminals there are given up (D-75).
    HeldBy(String),
    /// The target has no Hide of its own: one is installed only when the
    /// operator allows it (D-45).
    InstallNeeded { system: String },
}

/// One switch of the helper device this machine dials as `alias`.
pub struct Switch<'a> {
    pub reach: &'a Reach,
    pub alias: &'a str,
    pub request: &'a str,
    pub me: &'a Me,
    pub key: &'a PeerKey,
    /// The root this machine's helper is installed under on the target,
    /// when this machine asks for the helper's folder: not after its
    /// operator agreed to go on without it (D-75).
    pub helper_root: Option<&'a str>,
    /// The folders this machine registered as projects on the device,
    /// which the machine's own Hide lists from then on (D-48).
    pub projects: &'a [String],
    /// Called once with the pairing the target answered, on the same SSH
    /// login: whether the target's grant for this machine became active.
    /// Only then are the projects handed over, so a switch that never
    /// lands registers nothing there.
    pub admitted: &'a (dyn Fn(&Switched) -> bool + Sync),
    /// Where the login's questions go: the device's row.
    pub press: Option<std::sync::Arc<dyn hide_node_link::device::SignIn>>,
    /// The operator allowed a Hide to be started on a target that runs
    /// none of its own (D-45); without it such a target is only named.
    pub install: bool,
    /// Called once when that start begins, for the row's step.
    pub installing: &'a (dyn Fn() + Sync),
}

/// Looks at the target and pairs with the Hide it runs, over one SSH login
/// that may ask the operator on the device's row. A refusal is its reason
/// code, and nothing was saved on either machine.
pub fn switch(switch: &Switch<'_>) -> Result<Switched, &'static str> {
    let upstream = Upstream::new(SshTarget::new(&switch.reach.home, switch.alias));
    upstream.may_ask(switch.press.clone());
    let done = switch_on(&upstream, switch);
    upstream.close();
    done
}

fn switch_on(upstream: &Upstream, switch: &Switch<'_>) -> Result<Switched, &'static str> {
    let (looked, desktop) = match run(upstream, switch.reach, &["look-ssh"]) {
        Ok(said) => {
            let desktop = desktop_of(&said);
            (look_of(said), desktop)
        }
        Err(error) => {
            log(switch.alias, "look", &error);
            return Err("ssh_not_reached");
        }
    };
    let mut started = false;
    // A target with no Hide of its own is asked nothing more when this
    // machine may not start one there: said before the operator is asked
    // to allow it, and before anything is started.
    if looked == Looked::Absent {
        let system = upstream.system().map_err(|_| "ssh_not_reached")?;
        if let Some(refusal) = install_refusal(&system, std::env::consts::OS, desktop) {
            return Err(refusal);
        }
    }
    match looked {
        Looked::Ready => {}
        Looked::Absent if switch.install => {
            (switch.installing)();
            start_core(upstream, switch.reach, switch.alias)?;
            // The core that was started answers for the account now.
            match run(upstream, switch.reach, &["look-ssh"]).map(look_of) {
                Ok(Looked::Ready) => started = true,
                Ok(_) => return Err("ssh_core_not_started"),
                Err(error) => {
                    log(switch.alias, "look", &error);
                    return Err("ssh_not_reached");
                }
            }
        }
        Looked::Absent => {
            let system = upstream.system().map_err(|_| "ssh_not_reached")?;
            return Ok(Switched::InstallNeeded { system });
        }
        Looked::UpdateThere => return Err("ssh_update_there"),
        Looked::NotRunning { .. } => return Err("ssh_hide_not_running"),
        Looked::NotReached => return Err("ssh_not_reached"),
    }
    let accepted = accept_on(
        upstream,
        switch.reach,
        &Accept {
            alias: switch.alias,
            request: switch.request,
            me: switch.me,
            key: switch.key,
            helper: switch.helper_root,
            grant: None,
        },
    );
    let answer = match accepted {
        // Paired already, and going on without the helper's folder: there
        // is nothing to save on the target.
        Err("device_already_connected") if switch.helper_root.is_none() => {
            return Ok(admit(
                upstream,
                switch,
                Switched::Paired {
                    paired: None,
                    folder: Folder::Kept,
                    needs: None,
                },
            ));
        }
        answer => answer?,
    };
    let folder = match answer["helper"].as_str() {
        Some("held") => {
            let by = answer["by"].as_str().unwrap_or_default().to_owned();
            return Ok(Switched::HeldBy(by));
        }
        Some("taken") => Folder::Taken,
        Some("absent") => Folder::Absent,
        Some("kept") => Folder::Kept,
        // Asked for no folder, and so told of none.
        None if switch.helper_root.is_none() => Folder::Kept,
        _ => return Err("ssh_update_there"),
    };
    let paired = match &answer["accepted"] {
        Value::Null if switch.helper_root.is_some() => None,
        accepted => Some(paired(accepted, upstream.direct_host()).ok_or("ssh_update_there")?),
    };
    let needs = started
        .then(|| needs_of(upstream, switch.reach, switch.alias))
        .flatten();
    Ok(admit(
        upstream,
        switch,
        Switched::Paired {
            paired,
            folder,
            needs,
        },
    ))
}

/// The target answered the pairing `done`: the caller confirms it over the
/// connection, and the device's projects go to the machine's own Hide once
/// it admitted this machine (D-48), on this login and with a bound of
/// their own.
fn admit(upstream: &Upstream, switch: &Switch<'_>, done: Switched) -> Switched {
    if (switch.admitted)(&done) && !switch.projects.is_empty() {
        hand_projects(upstream, switch);
    }
    done
}

/// Has the target's own Hide list the folders this machine had registered
/// as projects on its device there (D-48). A project the target left out,
/// or an answer that never came, stays the device's here and is none of its
/// own Hide's: the switch stands either way, so it is only logged.
fn hand_projects(upstream: &Upstream, switch: &Switch<'_>) {
    let mut words = vec!["device", "projects-ssh"];
    for project in switch.projects {
        words.extend(["--project", project.as_str()]);
    }
    let listed = match run_hide_within(upstream, switch.reach, &words, PROJECTS_WITHIN) {
        Ok(Said::Answer(answer)) if answer["ok"] == true => {
            answer["value"]["projects"].as_u64().unwrap_or(0)
        }
        Ok(_) => 0,
        Err(error) => {
            log(switch.alias, "projects", &error);
            0
        }
    };
    if listed < switch.projects.len() as u64 {
        hide_core::diagnostic!(serde_json::json!({
            "component": "pairing",
            "kind": "ssh.projects_left_out",
            "alias": switch.alias,
            "asked": switch.projects.len(),
            "listed": listed,
        }));
    }
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

    /// A running core's look names it, node and build, which is what a node
    /// reads before it asks to be split from it; a look that names none (a
    /// Hide before 1.2.0, one that is not running, none at all) reads as no
    /// core to split from.
    #[test]
    fn a_look_names_the_running_core_and_only_a_running_one() {
        let release = crate::build_order::Release {
            version: "1.2.11".to_owned(),
            order: 1043,
            commit: None,
        };
        let named = json!({"ok":true,"value":{"look":{
            "system":"macos","core":true,"node":"studio","build":"b1","release":release,
        }}});
        assert_eq!(
            core_of(said(0, &line(named))),
            Some(CoreThere {
                node: "studio".to_owned(),
                build: "b1".to_owned(),
                release,
            })
        );
        for unnamed in [
            said(
                0,
                &line(json!({"ok":true,"value":{"look":{"system":"macos","core":true}}})),
            ),
            said(
                0,
                &line(json!({"ok":true,"value":{"look":{
                    "system":"macos","core":true,"node":"studio","build":null,"release":null,
                }}})),
            ),
            said(
                1,
                &line(json!({
                    "ok":false,"error":{"code":"hide_unavailable"},
                    "look":{"system":"macos","core":true},
                })),
            ),
            said(2, b"usage: hide device list\n"),
            said(127, b""),
        ] {
            assert_eq!(core_of(unnamed), None);
        }
    }

    /// A Mac takes a Hide over SSH only from a Mac, and only while someone
    /// is logged in to its desktop (PRD device-connect-iroh D-45, B7); a
    /// target that does not say who is logged in, as an older helper's
    /// `hide` does not, is not refused for it, and no other system is asked.
    #[test]
    fn a_mac_target_takes_a_hide_only_from_a_mac_with_someone_logged_in() {
        let absent = |desktop: Value| {
            let mut look = json!({"system":"macos","core":false});
            if !desktop.is_null() {
                look["desktop"] = desktop;
            }
            said(
                1,
                &line(json!({"ok":false,"error":{"code":"hide_unavailable"},"look":look})),
            )
        };
        assert_eq!(desktop_of(&absent(json!(false))), Some(false));
        assert_eq!(desktop_of(&absent(json!(true))), Some(true));
        assert_eq!(desktop_of(&absent(Value::Null)), None);
        assert_eq!(desktop_of(&said(127, b"")), None);

        assert_eq!(
            install_refusal("macos", "linux", Some(true)),
            Some("ssh_mac_from_mac_only")
        );
        assert_eq!(
            install_refusal("macos", "windows", None),
            Some("ssh_mac_from_mac_only")
        );
        assert_eq!(
            install_refusal("macos", "macos", Some(false)),
            Some("ssh_mac_signed_out")
        );
        assert_eq!(install_refusal("macos", "macos", Some(true)), None);
        assert_eq!(install_refusal("macos", "macos", None), None);
        for target in ["linux", "windows"] {
            for from in ["macos", "linux", "windows"] {
                assert_eq!(install_refusal(target, from, Some(false)), None);
            }
        }
    }

    /// Only an answer that names the command an administrator runs is a
    /// need: an account that lingers, a system that uses no linger and an
    /// answer this build cannot read need nothing shown (B30).
    #[test]
    fn linger_is_needed_only_with_the_command_that_turns_it_on() {
        let answered = |value: Value| linger_needs(&json!({"ok": true, "value": value}));
        assert_eq!(
            answered(json!({
                "linger": "needs_administrator",
                "command": "sudo loginctl enable-linger hy",
            })),
            Some(Needs::Linger {
                command: "sudo loginctl enable-linger hy".to_owned()
            })
        );
        assert_eq!(answered(json!({"linger": "on"})), None);
        assert_eq!(answered(json!({"linger": "not_used"})), None);
        assert_eq!(answered(json!({"linger": "needs_administrator"})), None);
        assert_eq!(
            linger_needs(&json!({"ok": false, "error": {"code": "usage"}})),
            None
        );
        assert_eq!(Needs::AutoLogin.payload(), json!({"kind": "auto_login"}));
    }

    /// The command folder is spelled for the system that runs it, from the
    /// account's home.
    #[test]
    fn the_targets_hide_is_spelled_for_its_system() {
        let default = Reach {
            home: PathBuf::from("/Users/example"),
            command_folder: None,
            hide: None,
        };
        assert_eq!(
            program(&default, "macos"),
            ["sh", "-c", "exec \"$HOME/$0\" \"$@\"", ".local/bin/hide"]
        );
        assert_eq!(program(&default, "windows"), [".local\\bin\\hide.cmd"]);
        let named = Reach {
            home: PathBuf::from("/Users/example"),
            command_folder: Some("/opt/hide/bin".to_owned()),
            hide: None,
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
