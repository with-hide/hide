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
//!
//! The same two commands and the same switch run in a WSL distribution of
//! this PC through `wsl.exe` (`crate::pairing_wsl`, D-27): what differs is
//! the channel that carries them ([`Login`]), never what is asked or how an
//! answer reads.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use hide_node::ssh::host::HelperPackages;
use hide_node::ssh::session::{Ran, SessionError, SshTarget};
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

/// The channel a target's account is reached on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Ssh,
    /// `wsl.exe`, into a distribution of this PC.
    Wsl,
}

impl Channel {
    /// The code of "this channel did not reach the account".
    fn not_reached(self) -> &'static str {
        match self {
            Self::Ssh => "ssh_not_reached",
            Self::Wsl => "wsl_not_reached",
        }
    }
}

/// An account on a target that a channel already signed in to runs this
/// machine's commands in: an SSH session, or `wsl.exe` into a distribution.
pub trait Login {
    fn channel(&self) -> Channel;

    /// What the diagnostic log names the target by.
    fn name(&self) -> &str;

    /// The system the account runs on, as `std::env::consts::OS` spells it.
    fn system(&self) -> Result<String, String>;

    /// Whether the target is on, asked without starting it. A command run
    /// in a WSL distribution that is off starts it, so work nobody pressed
    /// for asks this first (B43); a machine reached over SSH is on whenever
    /// it is reached.
    fn on(&self) -> Result<bool, String>;

    /// Runs `argv` there to its end and answers its exit code and at most
    /// `cap` bytes of what it printed.
    fn run(&self, argv: &[String], cap: usize, within: Duration) -> Result<Ran, String>;

    /// The addresses to try before the ones the target named (`own`): what
    /// this channel knows of where the target is.
    fn first(&self, own: &[Candidate]) -> Vec<Candidate>;

    /// Puts this build's programs in their version folder under `root`
    /// there, sending only what the account lacks, and answers where the
    /// build's `hided` is (B23).
    fn install(&self, packages: &HelperPackages, root: &str) -> Result<String, NotInstalled>;
}

/// Why this build was not put on a target.
#[derive(Debug)]
pub enum NotInstalled {
    /// The channel did not reach the account.
    NotReached(String),
    /// The account was reached and the build did not go in.
    Failed(String),
}

/// The SSH session `upstream` signs in, as a [`Login`].
struct Ssh<'a>(&'a Upstream);

impl Login for Ssh<'_> {
    fn channel(&self) -> Channel {
        Channel::Ssh
    }

    fn on(&self) -> Result<bool, String> {
        Ok(true)
    }

    fn name(&self) -> &str {
        self.0.alias()
    }

    fn system(&self) -> Result<String, String> {
        self.0.system().map_err(|error| error.to_string())
    }

    fn run(&self, argv: &[String], cap: usize, within: Duration) -> Result<Ran, String> {
        self.0
            .run(argv, cap, within)
            .map_err(|error| error.to_string())
    }

    fn first(&self, own: &[Candidate]) -> Vec<Candidate> {
        self.0
            .direct_host()
            .map(|host| login_addresses(&host, own))
            .unwrap_or_default()
    }

    fn install(&self, packages: &HelperPackages, root: &str) -> Result<String, NotInstalled> {
        self.0
            .install_build(packages, root)
            .map_err(|error| match error {
                SessionError::Ssh(_) | SessionError::Lost(_) => {
                    NotInstalled::NotReached(error.to_string())
                }
                SessionError::Refused(_) | SessionError::Local(_) => {
                    NotInstalled::Failed(error.to_string())
                }
            })
    }
}

/// Brings the Hide of the target `alias` names, which this machine
/// switched, to this build (`crate::started_cores`), over one SSH login
/// that asks nobody: it runs with no operator at the row.
pub fn raise(
    reach: &Reach,
    alias: &str,
    packages: &HelperPackages,
) -> Result<crate::started_cores::Raised, crate::started_cores::Failure> {
    let upstream = Upstream::new(SshTarget::new(&reach.home, alias));
    let raised = crate::started_cores::raise_on(&Ssh(&upstream), packages);
    upstream.close();
    raised
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

/// Runs `hide device <words>` on the target, on `login`.
fn run(login: &dyn Login, reach: &Reach, words: &[&str]) -> Result<Said, String> {
    let mut device = vec!["device"];
    device.extend(words);
    run_hide(login, reach, &device)
}

/// The words that run `hide <words>` in an account of `system`.
pub(crate) fn hide_argv(reach: &Reach, system: &str, words: &[&str]) -> Vec<String> {
    let mut argv = program(reach, system);
    argv.extend(words.iter().map(|word| (*word).to_owned()));
    argv
}

/// Runs `hide <words>` on the target, on `login`.
fn run_hide(login: &dyn Login, reach: &Reach, words: &[&str]) -> Result<Said, String> {
    run_hide_within(login, reach, words, ANSWER_WITHIN)
}

/// Runs `hide <words>` on the target, on `login`, waiting `within` for its
/// answer.
fn run_hide_within(
    login: &dyn Login,
    reach: &Reach,
    words: &[&str],
    within: Duration,
) -> Result<Said, String> {
    let argv = hide_argv(reach, &login.system()?, words);
    let ran = login.run(&argv, ANSWER_CAP, within)?;
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
pub(crate) fn start_core(login: &dyn Login, reach: &Reach) -> Result<(), &'static str> {
    match run_hide(login, reach, &["connect"]) {
        Ok(Said::Answer(answer)) if answer["ok"] == true => Ok(()),
        Ok(Said::NoProgram) => Err("ssh_hide_absent"),
        Ok(_) => Err("ssh_core_not_started"),
        Err(error) => Err(not_reached(login, "start", &error)),
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

/// Asks the machine `alias` names whether it can pair over SSH. Blocks for
/// as long as ssh takes to sign in and the target to answer.
pub fn look(reach: &Reach, alias: &str) -> Looked {
    let upstream = Upstream::new(SshTarget::new(&reach.home, alias));
    let looked = look_on(&Ssh(&upstream), reach);
    upstream.close();
    looked
}

/// [`look`], on a login.
pub(crate) fn look_on(login: &dyn Login, reach: &Reach) -> Looked {
    match run(login, reach, &["look-ssh"]) {
        Ok(said) => look_of(said),
        Err(error) => {
            not_reached(login, "look", &error);
            Looked::NotReached
        }
    }
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
    let paired = pair_on(&Ssh(&upstream), reach, request, me, key);
    upstream.close();
    paired
}

/// [`accept`], on a login.
pub(crate) fn pair_on(
    login: &dyn Login,
    reach: &Reach,
    request: &str,
    me: &Me,
    key: &PeerKey,
) -> Result<Paired, &'static str> {
    let asked = Accept {
        request,
        me,
        key,
        helper: None,
    };
    let answer = accept_on(login, reach, &asked)?;
    paired(login, &answer["accepted"]).ok_or("ssh_update_there")
}

/// What one `accept-ssh` asks of a target.
struct Accept<'a> {
    request: &'a str,
    me: &'a Me,
    key: &'a PeerKey,
    /// The root this machine's device helper is installed under there,
    /// when it also asks for that helper's folder (D-49).
    helper: Option<&'a str>,
}

/// Runs the target's `accept-ssh` on `login` and answers the value it
/// printed.
fn accept_on(login: &dyn Login, reach: &Reach, asked: &Accept<'_>) -> Result<Value, &'static str> {
    let me = asked.me;
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
    // The target records which channel its grant was asked over.
    if login.channel() == Channel::Wsl {
        words.push("--wsl");
    }
    let mut answer = match run(login, reach, &words) {
        Ok(Said::Answer(answer)) => answer,
        Ok(Said::NoProgram) => return Err("ssh_hide_absent"),
        Ok(Said::Unread) => return Err("ssh_update_there"),
        Err(error) => return Err(not_reached(login, "accept", &error)),
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
    Ok(answer["value"].take())
}

/// The pairing a target's answer carries, with the addresses its login
/// knows the target by.
fn paired(login: &dyn Login, accepted: &Value) -> Option<Paired> {
    let accepted: Accepted = serde_json::from_value(accepted.clone()).ok()?;
    let first = login.first(&accepted.candidates);
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
    /// A WSL distribution with no systemd: nothing there starts its core,
    /// which lives while the distribution runs, so after Windows restarts
    /// the operator opens the distribution again (B43).
    OpenDistribution,
}

impl Needs {
    /// As the core's `device_switch` event carries it.
    pub fn payload(&self) -> Value {
        match self {
            Self::AutoLogin => serde_json::json!({ "kind": "auto_login" }),
            Self::Linger { command } => {
                serde_json::json!({ "kind": "linger", "command": command })
            }
            Self::OpenDistribution => serde_json::json!({ "kind": "open_distribution" }),
        }
    }
}

/// What the target's Hide needs. A WSL distribution with no systemd says
/// so whoever started its Hide, since nothing there starts it again. For a
/// Hide this switch `started` the rest is read from the system it runs on,
/// and on Linux from its own `hide always-on linger`, which turns linger
/// on when the account may. A target that does not answer needs nothing
/// this machine can name; the switch goes on either way.
fn needs_of(login: &dyn Login, reach: &Reach, started: bool) -> Option<Needs> {
    if login.channel() == Channel::Wsl && !runs_systemd(login) {
        return Some(Needs::OpenDistribution);
    }
    if !started {
        return None;
    }
    match login.system().ok()?.as_str() {
        "macos" => Some(Needs::AutoLogin),
        "linux" => match run_hide(login, reach, &["always-on", "linger", "--json"]) {
            Ok(Said::Answer(answer)) => linger_needs(&answer),
            Ok(_) => None,
            Err(error) => {
                not_reached(login, "linger", &error);
                None
            }
        },
        _ => None,
    }
}

/// Whether the target was started by systemd, by the folder systemd itself
/// names as the sign of it (`sd_booted`). A target that does not answer is
/// read as one that was: the line a missing systemd shows is said only of
/// a distribution known to have none.
pub(crate) fn runs_systemd(login: &dyn Login) -> bool {
    let argv = ["test", "-d", "/run/systemd/system"].map(str::to_owned);
    // `test` prints nothing; the cap is the least a run may be given.
    match login.run(&argv, 1, ANSWER_WITHIN) {
        Ok(ran) => ran.status == 0,
        Err(error) => {
            not_reached(login, "systemd", &error);
            true
        }
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

/// One switch of a helper device this machine dials.
pub struct Switch<'a> {
    pub reach: &'a Reach,
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
    /// Called once with the pairing the target answered, on the same
    /// login: whether the target's grant for this machine became active.
    /// Only then are the projects handed over, so a switch that never
    /// lands registers nothing there.
    pub admitted: &'a (dyn Fn(&Switched) -> bool + Sync),
    /// The operator allowed a Hide to be started on a target that runs
    /// none of its own (D-45); without it such a target is only named.
    pub install: bool,
    /// Called once when that start begins, for the row's step.
    pub installing: &'a (dyn Fn() + Sync),
}

/// Looks at the target `alias` names and pairs with the Hide it runs, over
/// one SSH login whose questions go to `press`, the device's row. A refusal
/// is its reason code, and nothing was saved on either machine.
pub fn switch(
    alias: &str,
    press: Option<std::sync::Arc<dyn hide_node_link::device::SignIn>>,
    switch: &Switch<'_>,
) -> Result<Switched, &'static str> {
    let upstream = Upstream::new(SshTarget::new(&switch.reach.home, alias));
    upstream.may_ask(press);
    let done = switch_on(&Ssh(&upstream), switch);
    upstream.close();
    done
}

/// [`switch`], on a login.
pub(crate) fn switch_on(login: &dyn Login, switch: &Switch<'_>) -> Result<Switched, &'static str> {
    let unreached = login.channel().not_reached();
    let (looked, desktop) = match run(login, switch.reach, &["look-ssh"]) {
        Ok(said) => {
            let desktop = desktop_of(&said);
            (look_of(said), desktop)
        }
        Err(error) => return Err(not_reached(login, "look", &error)),
    };
    let mut started = false;
    // A target with no Hide of its own is asked nothing more when this
    // machine may not start one there: said before the operator is asked
    // to allow it, and before anything is started.
    if looked == Looked::Absent {
        let system = login
            .system()
            .map_err(|error| not_reached(login, "system", &error))?;
        if let Some(refusal) = install_refusal(&system, std::env::consts::OS, desktop) {
            return Err(refusal);
        }
    }
    match looked {
        Looked::Ready => {}
        Looked::Absent if switch.install => {
            (switch.installing)();
            start_core(login, switch.reach)?;
            // The core that was started answers for the account now.
            match look_on(login, switch.reach) {
                Looked::Ready => started = true,
                Looked::NotReached => return Err(unreached),
                _ => return Err("ssh_core_not_started"),
            }
        }
        Looked::Absent => {
            let system = login
                .system()
                .map_err(|error| not_reached(login, "system", &error))?;
            return Ok(Switched::InstallNeeded { system });
        }
        Looked::UpdateThere => return Err("ssh_update_there"),
        Looked::NotRunning { .. } => return Err("ssh_hide_not_running"),
        Looked::NotReached => return Err(unreached),
    }
    let accepted = accept_on(
        login,
        switch.reach,
        &Accept {
            request: switch.request,
            me: switch.me,
            key: switch.key,
            helper: switch.helper_root,
        },
    );
    let answer = match accepted {
        // Paired already, and going on without the helper's folder: there
        // is nothing to save on the target.
        Err("device_already_connected") if switch.helper_root.is_none() => {
            return Ok(admit(
                login,
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
        accepted => Some(paired(login, accepted).ok_or("ssh_update_there")?),
    };
    let needs = needs_of(login, switch.reach, started);
    Ok(admit(
        login,
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
fn admit(login: &dyn Login, switch: &Switch<'_>, done: Switched) -> Switched {
    if (switch.admitted)(&done) && !switch.projects.is_empty() {
        hand_projects(login, switch);
    }
    done
}

/// Has the target's own Hide list the folders this machine had registered
/// as projects on its device there (D-48). A project the target left out,
/// or an answer that never came, stays the device's here and is none of its
/// own Hide's: the switch stands either way, so it is only logged.
fn hand_projects(login: &dyn Login, switch: &Switch<'_>) {
    let mut words = vec!["device", "projects-ssh"];
    for project in switch.projects {
        words.extend(["--project", project.as_str()]);
    }
    let listed = match run_hide_within(login, switch.reach, &words, PROJECTS_WITHIN) {
        Ok(Said::Answer(answer)) if answer["ok"] == true => {
            answer["value"]["projects"].as_u64().unwrap_or(0)
        }
        Ok(_) => 0,
        Err(error) => {
            not_reached(login, "projects", &error);
            0
        }
    };
    if listed < switch.projects.len() as u64 {
        hide_core::diagnostic!(serde_json::json!({
            "component": "pairing",
            "kind": "ssh.projects_left_out",
            "alias": login.name(),
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
        Candidate::Relay(_) | Candidate::Wsl(_) => None,
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

/// Records that `login` did not carry `step`, and answers the code of it.
fn not_reached(login: &dyn Login, step: &str, error: &str) -> &'static str {
    let channel = login.channel();
    hide_core::diagnostic!(serde_json::json!({
        "component": "pairing",
        "kind": match channel {
            Channel::Ssh => "ssh.not_reached",
            Channel::Wsl => "wsl.not_reached",
        },
        "step": step,
        "alias": login.name(),
        "reason": error,
    }));
    channel.not_reached()
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
        assert_eq!(
            Needs::OpenDistribution.payload(),
            json!({"kind": "open_distribution"})
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
