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
    /// The words that start the target's `hide`, for a caller that knows
    /// where it is: the node of a core moved before 1.2.0 runs the `hide`
    /// beside that core's program (`moved_core`), and a helper device's
    /// read and switch the one beside its node's ([`Reach::running`]).
    /// None reads the command folder.
    pub hide: Option<Vec<String>>,
}

impl Reach {
    /// This reach for a helper device whose node runs `node` there (its
    /// `hided`, where the connection that started it put it): every `hide`
    /// asked of the account is the one beside that program, this machine's
    /// own build, never the one the account links, which follows a build
    /// only once that build's kit has run there and may still name an
    /// older helper's (with-hide/hide#3007). A device with no node this
    /// core started there reads the command folder, as any account does.
    pub fn running(&self, node: Option<&str>) -> Reach {
        let mut reach = self.clone();
        if let Some(hide) = node.and_then(hide_beside) {
            reach.hide = Some(vec![hide]);
        }
        reach
    }
}

/// The `hide` in the folder of `program`, a path as two machines spell it to
/// each other (`/` between names on every system), with the `.exe` a
/// Windows program is found by. None for a program that names no folder,
/// which no install writes.
pub(crate) fn hide_beside(program: &str) -> Option<String> {
    let (folder, file) = program.rsplit_once('/')?;
    let name = if file.ends_with(".exe") {
        "hide.exe"
    } else {
        "hide"
    };
    Some(format!("{folder}/{name}"))
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
    /// Its Hide runs and refused the look, for a reason this build has no
    /// state of its own for: never read as an older Hide.
    Refused { code: String },
    /// Its Hide is installed and does not run: on a Mac, nobody is logged
    /// in.
    NotRunning { system: String },
    /// SSH did not sign in without asking, or the machine was not reached.
    NotReached,
}

/// Whether `alias` is a name `ssh` reads as a destination and never as an
/// option.
pub fn valid_alias(alias: &str) -> bool {
    hide_peer::store::valid_alias(alias)
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
pub(crate) struct Ssh<'a>(pub(crate) &'a Upstream);

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

/// The target's `hide` as the command folder names it: whether its path is
/// under the account's home (and so relative to it), and that path, in wire
/// spelling. The one rule [`program`] and [`carry_argv`] read, so the
/// command that paired is the command that carries.
fn folder_program(reach: &Reach, system: &str) -> (bool, String) {
    let folder = reach.command_folder.as_deref().unwrap_or(COMMAND_FOLDER);
    let (under_home, folder) = match folder.strip_prefix("~/") {
        Some(under) => (true, under),
        None => (false, folder),
    };
    let name = if system == "windows" {
        "hide.cmd"
    } else {
        "hide"
    };
    (
        under_home,
        format!("{}/{name}", folder.trim_end_matches('/')),
    )
}

/// The words that start the target's `hide` in a run step. A folder under
/// the account's home is read from that account's own `HOME` on a POSIX
/// system; a Windows session starts in the profile folder, which a path
/// under the home is relative to.
fn program(reach: &Reach, system: &str) -> Vec<String> {
    if let Some(hide) = &reach.hide {
        return hide.clone();
    }
    let (under_home, path) = folder_program(reach, system);
    if system == "windows" {
        return vec![path.replace('/', "\\")];
    }
    match under_home {
        true => vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "exec \"$HOME/$0\" \"$@\"".to_owned(),
            path,
        ],
        false => vec![path],
    }
}

/// The words that start the target's `hide device carry --ssh` in the start
/// script's place: the `hide` [`program`] runs, made absolute
/// from the account's `home` (wire spelling), since the script execs it
/// rather than running it through a shell. Named words that do not start
/// with an absolute program are read through the command folder instead.
pub(crate) fn carry_argv(reach: &Reach, home: &str, system: &str) -> Vec<String> {
    let mut argv = match reach.hide.as_deref() {
        Some(words @ [hide, ..]) if hide_platform::path::is_wire_absolute(hide) => words.to_vec(),
        _ => match folder_program(reach, system) {
            (true, path) => vec![format!("{}/{path}", home.trim_end_matches('/'))],
            (false, path) => vec![path],
        },
    };
    argv.extend(["device", "carry", "--ssh"].map(str::to_owned));
    argv
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

/// Whether the target said an app of its account opens its Hide
/// (`hide_kit::app::present`, in its look): `None` when its answer does not
/// say, as an older Hide's does not, or as one that could not read its
/// note does not.
fn app_of(said: &Said) -> Option<bool> {
    match said {
        Said::Answer(answer) => answer
            .get("look")
            .or_else(|| answer.get("value").and_then(|value| value.get("look")))
            .and_then(|look| look["app"].as_bool()),
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

/// Whether a refusal says the Hide there is older: its `hide` refused the
/// words (`usage`), its daemon the ask (`invalid_request`), or it is a 1.2.11
/// build, which kept connections between two Hides behind a switch and
/// refused every verb that pairs (`device_connect_unavailable`). Only that
/// says the Hide there is older.
fn unknown_there(answer: &Value) -> bool {
    matches!(
        answer["error"]["code"].as_str(),
        Some("usage" | "invalid_request" | "device_connect_unavailable")
    )
}

/// The code of a refusal that does not say the Hide there is older, logged
/// with the step it ended, and the reason a switch or a pairing gives.
fn refused_there(login: &dyn Login, step: &str, code: &str) -> &'static str {
    hide_core::diagnostic!(serde_json::json!({
        "component": "pairing",
        "kind": "ssh.refused_there",
        "step": step,
        "alias": login.name(),
        "code": code,
    }));
    "ssh_refused_there"
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
            None if unknown_there(&answer) => Looked::UpdateThere,
            None => Looked::Refused {
                code: answer["error"]["code"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            },
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
pub fn core_on(upstream: &Upstream, reach: &Reach) -> Result<Option<CoreThere>, String> {
    Ok(core_of(run(&Ssh(upstream), reach, &["look-ssh"])?))
}

fn core_of(said: Said) -> Option<CoreThere> {
    match said {
        Said::Answer(answer) if answer["ok"] == true => {
            serde_json::from_value(answer["value"]["look"].clone()).ok()
        }
        Said::Answer(_) | Said::NoProgram | Said::Unread => None,
    }
}

/// What a Host the operator chose in Add device said (B7): whether a Hide of
/// its own runs, the system its account runs, and for one that runs none,
/// why a switch could start none there, `None` when it could. Add device
/// and `hide device add` connect it as a server from this, or add it over
/// SSH (operator decision 2026-11-28).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostLook {
    pub looked: Looked,
    /// As `std::env::consts::OS` spells it; none for a login that did not
    /// sign in.
    pub system: Option<String>,
    /// For [`Looked::Absent`] alone: the code of why no Hide is started
    /// there.
    pub leaves: Option<&'static str>,
    /// For [`Looked::Ready`] alone: where the machine stands on the rule
    /// every switch passes, as a look reads it ([`may_move`]), so a Mac
    /// that does not log its account in by itself is said before the press
    /// (operator decision 2026-11-28); none when its `hide` did not say.
    pub condition: Option<Condition>,
}

/// Asks the machine `alias` names whether it can pair over SSH and, when it
/// runs no Hide, whether one can be started there, over one login that asks
/// nobody and changes nothing there. Blocks for as long as ssh takes to
/// sign in and the target to answer.
pub fn look(reach: &Reach, alias: &str) -> HostLook {
    let upstream = Upstream::new(SshTarget::new(&reach.home, alias));
    let login = Ssh(&upstream);
    let looked = host_look(&login, reach, look_on(&login, reach));
    upstream.close();
    looked
}

/// `looked`, with what the login it was read on says of the machine.
fn host_look(login: &dyn Login, reach: &Reach, looked: Looked) -> HostLook {
    if looked == Looked::NotReached {
        return HostLook {
            looked,
            system: None,
            leaves: None,
            condition: None,
        };
    }
    let system = login
        .system()
        .map_err(|error| not_reached(login, "system", &error))
        .ok();
    let leaves = match (&looked, system.as_deref()) {
        (Looked::Absent, Some(system)) => takes_no_hide(login, system),
        (Looked::Absent, None) => Some(login.channel().not_reached()),
        _ => None,
    };
    let condition = match looked {
        Looked::Ready => may_move(login, reach, Asked::Look)
            .ok()
            .map(|moved| moved.condition),
        _ => None,
    };
    HostLook {
        looked,
        system,
        leaves,
        condition,
    }
}

/// Why a switch could start no Hide on an account of `system` that runs
/// none, read before anything is put there: the rule [`switch_on`] and
/// [`may_move`] pass, as far as it is read without a `hide` there. Windows
/// is not switched, a Mac takes a Hide only from a Mac, and a Linux with no
/// systemd has nothing that starts it again. Whether a Linux account may
/// linger and whether a Mac has someone logged in are read by the switch
/// itself, through the `hide` its device helper brings.
fn takes_no_hide(login: &dyn Login, system: &str) -> Option<&'static str> {
    match system {
        "windows" => Some("windows"),
        "macos" => install_refusal(system, std::env::consts::OS, None),
        "linux" if runs_systemd(login) => None,
        "linux" => Some("no_systemd"),
        _ => Some("unknown"),
    }
}

/// [`look`], on a login.
pub(crate) fn look_on(login: &dyn Login, reach: &Reach) -> Looked {
    match run(login, reach, &["look-ssh"]) {
        Ok(said) => logged_look(login, look_of(said)),
        Err(error) => {
            not_reached(login, "look", &error);
            Looked::NotReached
        }
    }
}

/// `looked`, with a refusal the look did not know logged.
fn logged_look(login: &dyn Login, looked: Looked) -> Looked {
    if let Looked::Refused { code } = &looked {
        refused_there(login, "look", code);
    }
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
        grant: None,
    };
    let answer = accept_on(login, reach, &asked)?;
    paired(login, &answer["accepted"]).ok_or("ssh_update_there")
}

/// What one `accept-ssh` asks of a target.
pub(crate) struct Accept<'a> {
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

/// Runs the target's `accept-ssh` on `login` and answers the value it
/// printed.
pub(crate) fn accept_on(
    login: &dyn Login,
    reach: &Reach,
    asked: &Accept<'_>,
) -> Result<Value, &'static str> {
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
    if let Some(grant) = asked.grant {
        words.extend(["--grant", grant]);
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
            Some("device_unknown") => "device_unknown",
            Some("usage") => "ssh_update_there",
            _ => refused_there(
                login,
                "accept",
                answer["error"]["code"].as_str().unwrap_or_default(),
            ),
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

/// Where a device's machine stands on the one rule every switch passes
/// (operator decision 2026-11-24): what the button beside it says, whether
/// it may be pressed, and what `hide device switch` answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    /// A Mac that logs the account in by itself: its Hide is back after a
    /// restart with nobody there.
    AutoLoginOn,
    /// A Mac that does not: the switch runs, and after a restart someone
    /// logs in to that Mac once before it connects again. `how` is whether
    /// the setting can be turned on there, which FileVault rules out.
    AutoLoginOff { how: bool },
    /// A Mac nobody is logged in to: its Hide runs only in a login, so
    /// none can be started or reached there now.
    SignedOut,
    /// Linux: the account lingers.
    LingerOn,
    /// Linux: the account does not linger yet. The switch asks for it,
    /// and only then is it known whether the account may.
    LingerOff,
    /// Linux: the account may not turn linger on itself, so its Hide ends
    /// with its last session until an administrator runs `command`.
    LingerAdmin { command: String },
    /// A Linux, or a WSL distribution, with no systemd: nothing there
    /// starts its Hide again (B43).
    NoSystemd,
    /// Windows: no device there is switched in 1.3.0.
    Windows,
    /// The account keeps no login item (a HOME that is not the account's
    /// own), or its system is none this build knows.
    Unknown,
}

impl Condition {
    /// Whether a switch runs on it.
    pub fn may(&self) -> bool {
        matches!(
            self,
            Self::AutoLoginOn | Self::AutoLoginOff { .. } | Self::LingerOn | Self::LingerOff
        )
    }

    /// As the answer of a read and of a switch carries it.
    pub fn payload(&self) -> Value {
        use serde_json::json;
        match self {
            Self::AutoLoginOn => json!({ "kind": "auto_login_on" }),
            Self::AutoLoginOff { how } => json!({ "kind": "auto_login_off", "how": how }),
            Self::SignedOut => json!({ "kind": "signed_out" }),
            Self::LingerOn => json!({ "kind": "linger_on" }),
            Self::LingerOff => json!({ "kind": "linger_off" }),
            Self::LingerAdmin { command } => json!({ "kind": "linger_admin", "command": command }),
            Self::NoSystemd => json!({ "kind": "no_systemd" }),
            Self::Windows => json!({ "kind": "windows" }),
            Self::Unknown => json!({ "kind": "unknown" }),
        }
    }
}

/// What a device's machine said of the rule, and the account it said it
/// of.
#[derive(Debug, PartialEq, Eq)]
pub struct Move {
    pub condition: Condition,
    /// The account's login name there, which the screen's words name.
    pub user: Option<String>,
}

/// Whether a read may change the machine it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asked {
    /// A look: an opened row. It changes nothing there.
    Look,
    /// The operator asked for the switch: a Linux account that does not
    /// linger is asked to, which alone says whether it may.
    Switch,
}

/// The one rule every switch passes, read from the device's machine over
/// the login it is dialed by, which asks nobody. The target's own `hide`
/// says the facts (`hide always-on unattended`) and this reads them per
/// system: a Mac needs someone logged in now, and is warned about, not
/// refused, when it does not log the account in by itself; a Linux
/// account needs systemd and linger; Windows is not switched. A look
/// changes nothing there; a switch asks a Linux account that does not
/// linger to (`hide always-on linger`), and nothing else. A target whose
/// `hide` does not answer was not read, and the code says so.
pub(crate) fn may_move(
    login: &dyn Login,
    reach: &Reach,
    asked: Asked,
) -> Result<Move, &'static str> {
    // A command run in a distribution that is off starts it, which a look
    // nobody pressed for does not do (B43).
    if asked == Asked::Look
        && !login
            .on()
            .map_err(|error| not_reached(login, "on", &error))?
    {
        return Err(login.channel().not_reached());
    }
    let system = login
        .system()
        .map_err(|error| not_reached(login, "system", &error))?;
    if system == "windows" {
        return Ok(Move {
            condition: Condition::Windows,
            user: None,
        });
    }
    let said = match run_hide(login, reach, &["always-on", "unattended", "--json"]) {
        Ok(Said::Answer(answer)) if answer["ok"] == true => answer["value"].clone(),
        Ok(Said::NoProgram) => return Err("ssh_hide_absent"),
        Ok(Said::Unread) => return Err("ssh_update_there"),
        Ok(Said::Answer(answer)) if unknown_there(&answer) => return Err("ssh_update_there"),
        Ok(Said::Answer(answer)) => {
            let code = answer["error"]["code"].as_str().unwrap_or_default();
            return Err(refused_there(login, "unattended", code));
        }
        Err(error) => return Err(not_reached(login, "unattended", &error)),
    };
    let user = said["user"].as_str().map(str::to_owned);
    let mut condition = condition_of(&system, &said, || runs_systemd(login));
    if condition == Condition::LingerOff && asked == Asked::Switch {
        condition = match run_hide(login, reach, &["always-on", "linger", "--json"]) {
            Ok(Said::Answer(answer)) if answer["ok"] == true => lingers_now(&answer["value"]),
            Ok(_) => Condition::Unknown,
            Err(error) => return Err(not_reached(login, "linger", &error)),
        };
    }
    Ok(Move { condition, user })
}

/// The rule, on what the target said of itself. `systemd` is asked only
/// of a Linux account that does not linger.
fn condition_of(system: &str, said: &Value, systemd: impl FnOnce() -> bool) -> Condition {
    let unattended = said["unattended"] == true;
    let desktop = said["desktop"].as_bool();
    match system {
        "macos" => match desktop {
            None => Condition::Unknown,
            Some(false) => Condition::SignedOut,
            Some(true) if unattended => Condition::AutoLoginOn,
            Some(true) => Condition::AutoLoginOff {
                how: said["automatic_login_offered"] != false,
            },
        },
        "linux" if unattended => Condition::LingerOn,
        // Nothing in a system with no systemd starts its Hide again,
        // whatever its `hide` answered.
        "linux" if !systemd() => Condition::NoSystemd,
        "linux" if desktop.is_some() => Condition::LingerOff,
        _ => Condition::Unknown,
    }
}

/// What asking for linger came to (`hide always-on linger`).
fn lingers_now(value: &Value) -> Condition {
    match (value["linger"].as_str(), value["command"].as_str()) {
        (Some("on"), _) => Condition::LingerOn,
        (Some("needs_administrator"), Some(command)) => Condition::LingerAdmin {
            command: command.to_owned(),
        },
        _ => Condition::Unknown,
    }
}

/// [`may_move`] for the helper device `alias` names, over a login of its
/// own that asks nobody.
pub fn may_move_device(reach: &Reach, alias: &str, asked: Asked) -> Result<Move, &'static str> {
    let upstream = Upstream::new(SshTarget::new(&reach.home, alias));
    let moves = may_move(&Ssh(&upstream), reach, asked);
    upstream.close();
    moves
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

/// How far a helper device's switch got on the SSH login alone.
pub enum Switched {
    /// The target's Hide saved a grant for this machine, or holds one
    /// already (`paired` is none): the connection confirms it.
    Paired {
        paired: Option<Paired>,
        folder: Folder,
    },
    /// Another machine took the helper's folder first, named as the target
    /// knows it: nothing was saved, and this machine goes on with a folder
    /// of its own only when it runs no terminal there (D-75).
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
    /// when this machine asks for the helper's folder: not when it goes
    /// on with a folder of its own (D-75).
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
    /// Told each step as it begins, for the row that draws the switch:
    /// `starting`, `pairing`, then `projects` once the target admitted
    /// this machine.
    pub step: &'a (dyn Fn(&'static str) + Sync),
}

/// Looks at the target `alias` names and pairs with the Hide it runs, over
/// one SSH login that asks nobody: the read every switch starts with
/// ([`may_move_device`]) signed in without asking. A refusal is its reason
/// code, and nothing was saved on either machine.
pub fn switch(alias: &str, switch: &Switch<'_>) -> Result<Switched, &'static str> {
    let upstream = Upstream::new(SshTarget::new(&switch.reach.home, alias));
    let done = switch_on(&Ssh(&upstream), switch);
    upstream.close();
    done
}

/// [`switch`], on a login.
pub(crate) fn switch_on(login: &dyn Login, switch: &Switch<'_>) -> Result<Switched, &'static str> {
    let unreached = login.channel().not_reached();
    (switch.step)("starting");
    let (looked, desktop, app) = match run(login, switch.reach, &["look-ssh"]) {
        Ok(said) => {
            let (desktop, app) = (desktop_of(&said), app_of(&said));
            (logged_look(login, look_of(said)), desktop, app)
        }
        Err(error) => return Err(not_reached(login, "look", &error)),
    };
    // A Hide of the account's own that stopped and that no app there opens
    // is one a machine started over a login: a switch that ended before
    // its pairing leaves it with nothing to keep it running, and nobody
    // can open it there. This machine starts it again; one an app opens
    // is the app's to open, and one that did not say is left alone.
    let stopped = matches!(looked, Looked::NotRunning { .. }) && app == Some(false);
    // A target this machine would start a Hide on is asked nothing more
    // when it may not start one there: said before the operator is asked
    // to allow it, and before anything is started.
    if looked == Looked::Absent || stopped {
        let system = login
            .system()
            .map_err(|error| not_reached(login, "system", &error))?;
        if let Some(refusal) = install_refusal(&system, std::env::consts::OS, desktop) {
            return Err(refusal);
        }
    }
    // Starts the account's Hide and has it answer for the account.
    let start = || -> Result<(), &'static str> {
        start_core(login, switch.reach)?;
        match look_on(login, switch.reach) {
            Looked::Ready => Ok(()),
            Looked::NotReached => Err(unreached),
            _ => Err("ssh_core_not_started"),
        }
    };
    match looked {
        Looked::Ready => {}
        Looked::Absent if switch.install => {
            start()?;
        }
        // The operator allowed a Hide there when it was first started.
        Looked::NotRunning { .. } if stopped => {
            start()?;
        }
        Looked::Absent => {
            let system = login
                .system()
                .map_err(|error| not_reached(login, "system", &error))?;
            return Ok(Switched::InstallNeeded { system });
        }
        Looked::UpdateThere => return Err("ssh_update_there"),
        Looked::Refused { .. } => return Err("ssh_refused_there"),
        Looked::NotRunning { .. } => return Err("ssh_hide_not_running"),
        Looked::NotReached => return Err(unreached),
    }
    (switch.step)("pairing");
    let accepted = accept_on(
        login,
        switch.reach,
        &Accept {
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
                login,
                switch,
                Switched::Paired {
                    paired: None,
                    folder: Folder::Kept,
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
    Ok(admit(login, switch, Switched::Paired { paired, folder }))
}

/// The target answered the pairing `done`: the caller confirms it over the
/// connection, and the device's projects go to the machine's own Hide once
/// it admitted this machine (D-48), on this login and with a bound of
/// their own.
fn admit(login: &dyn Login, switch: &Switch<'_>, done: Switched) -> Switched {
    if (switch.admitted)(&done) && !switch.projects.is_empty() {
        (switch.step)("projects");
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
        Candidate::Relay(_) | Candidate::Wsl(_) | Candidate::Ssh(_) => None,
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
        // A Hide of this build that refused the look, as a Windows PC's did
        // when it could not place a caller over SSH, is no older Hide.
        assert_eq!(
            look_of(said(
                1,
                &line(json!({"ok":false,"error":{"code":"caller_origin_unavailable"}}))
            )),
            Looked::Refused {
                code: "caller_origin_unavailable".to_owned()
            }
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

    /// An account with no `hide` at all, as a scripted login: every `hide`
    /// is a command that is not there, and `/run/systemd/system` is there
    /// when `systemd` says so.
    struct Bare {
        system: Result<&'static str, ()>,
        systemd: bool,
    }

    impl Login for Bare {
        fn channel(&self) -> Channel {
            Channel::Ssh
        }

        fn name(&self) -> &str {
            "bare"
        }

        fn system(&self) -> Result<String, String> {
            self.system
                .map(str::to_owned)
                .map_err(|()| "lost".to_owned())
        }

        fn on(&self) -> Result<bool, String> {
            Ok(true)
        }

        fn run(&self, argv: &[String], _cap: usize, _within: Duration) -> Result<Ran, String> {
            let status = match argv.first().map(String::as_str) {
                Some("test") if self.systemd => 0,
                Some("test") => 1,
                _ => 127,
            };
            Ok(Ran {
                status,
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }

        fn first(&self, _own: &[Candidate]) -> Vec<Candidate> {
            Vec::new()
        }

        fn install(&self, _packages: &HelperPackages, _root: &str) -> Result<String, NotInstalled> {
            Err(NotInstalled::Failed("not scripted".to_owned()))
        }
    }

    /// Add device connects a Host with no Hide as a server only where the
    /// switch could start one (operator decision 2026-11-28): Windows, a
    /// Linux with no systemd and a Mac asked from another system are added
    /// over SSH with the reason, and a machine with a Hide of its own is
    /// asked nothing more.
    #[test]
    fn a_host_with_no_hide_says_whether_a_switch_could_start_one() {
        let reach = Reach {
            home: PathBuf::from("/nonexistent"),
            command_folder: None,
            hide: None,
        };
        let leaves = |system, systemd| {
            let login = Bare { system, systemd };
            let look = host_look(&login, &reach, look_on(&login, &reach));
            assert_eq!(look.looked, Looked::Absent);
            look.leaves
        };
        assert_eq!(leaves(Ok("linux"), true), None);
        assert_eq!(leaves(Ok("linux"), false), Some("no_systemd"));
        assert_eq!(leaves(Ok("windows"), true), Some("windows"));
        assert_eq!(leaves(Ok("freebsd"), true), Some("unknown"));
        let mac = install_refusal("macos", std::env::consts::OS, None);
        assert_eq!(leaves(Ok("macos"), true), mac);
        assert_eq!(
            host_look(
                &Bare {
                    system: Err(()),
                    systemd: true
                },
                &reach,
                Looked::Absent
            ),
            HostLook {
                looked: Looked::Absent,
                system: None,
                leaves: Some("ssh_not_reached"),
                condition: None,
            }
        );
        let ready = host_look(
            &Bare {
                system: Ok("macos"),
                systemd: false,
            },
            &reach,
            Looked::Ready,
        );
        assert_eq!(
            ready,
            HostLook {
                looked: Looked::Ready,
                system: Some("macos".to_owned()),
                leaves: None,
                condition: None,
            }
        );
    }

    /// An account whose Hide runs, as a scripted login: its look answers,
    /// and `hide always-on unattended` says `said`.
    struct Running {
        said: Value,
    }

    impl Login for Running {
        fn channel(&self) -> Channel {
            Channel::Ssh
        }

        fn name(&self) -> &str {
            "running"
        }

        fn system(&self) -> Result<String, String> {
            Ok("macos".to_owned())
        }

        fn on(&self) -> Result<bool, String> {
            Ok(true)
        }

        fn run(&self, argv: &[String], _cap: usize, _within: Duration) -> Result<Ran, String> {
            let words: Vec<&str> = argv
                .iter()
                .map(String::as_str)
                .skip_while(|word| !matches!(*word, "device" | "always-on"))
                .take(2)
                .collect();
            let answer = match words.as_slice() {
                ["device", "look-ssh"] => {
                    json!({"ok":true,"value":{"look":{"system":"macos","core":true}}})
                }
                ["always-on", "unattended"] => json!({"ok":true,"value":self.said}),
                other => return Err(format!("not scripted: {other:?}")),
            };
            Ok(Ran {
                status: 0,
                stdout: line(answer),
                stderr: Vec::new(),
            })
        }

        fn first(&self, _own: &[Candidate]) -> Vec<Candidate> {
            Vec::new()
        }

        fn install(&self, _packages: &HelperPackages, _root: &str) -> Result<String, NotInstalled> {
            Err(NotInstalled::Failed("not scripted".to_owned()))
        }
    }

    /// A Mac that runs Hide says before Add device's press whether it logs
    /// its account in by itself (operator decision 2026-11-28): one that
    /// does not, FileVault's included, needs a login after a restart.
    #[test]
    fn a_mac_running_hide_says_before_the_press_whether_it_logs_in_by_itself() {
        let reach = Reach {
            home: PathBuf::from("/nonexistent"),
            command_folder: None,
            hide: None,
        };
        let condition = |said: Value| {
            let login = Running { said };
            let look = host_look(&login, &reach, look_on(&login, &reach));
            assert_eq!(look.looked, Looked::Ready);
            look.condition
        };
        assert_eq!(
            condition(json!({"unattended": true, "desktop": true})),
            Some(Condition::AutoLoginOn)
        );
        assert_eq!(
            condition(json!({"unattended": false, "desktop": true})),
            Some(Condition::AutoLoginOff { how: true })
        );
        assert_eq!(
            condition(
                json!({"unattended": false, "desktop": true, "automatic_login_offered": false})
            ),
            Some(Condition::AutoLoginOff { how: false })
        );
    }

    /// A target account whose own Hide stopped, as a scripted login: it
    /// records the words each `hide` was given, and its Hide runs once a
    /// `hide connect` was asked.
    struct Stopped {
        system: &'static str,
        /// What its look says beside the system and the core.
        look: Value,
        asked: std::sync::Mutex<Vec<String>>,
    }

    impl Stopped {
        fn new(system: &'static str, look: Value) -> Self {
            Self {
                system,
                look,
                asked: std::sync::Mutex::default(),
            }
        }

        fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Login for Stopped {
        fn channel(&self) -> Channel {
            Channel::Ssh
        }

        fn name(&self) -> &str {
            "stopped"
        }

        fn system(&self) -> Result<String, String> {
            Ok(self.system.to_owned())
        }

        fn on(&self) -> Result<bool, String> {
            Ok(true)
        }

        fn run(&self, argv: &[String], _cap: usize, _within: Duration) -> Result<Ran, String> {
            let words: Vec<&str> = argv
                .iter()
                .map(String::as_str)
                .skip_while(|word| !matches!(*word, "device" | "connect" | "always-on"))
                .take(2)
                .collect();
            let mut asked = self.asked.lock().unwrap();
            let runs = asked.iter().any(|words| words == "connect");
            asked.push(words.join(" "));
            let mut look = json!({"system": self.system, "core": true});
            let (status, answer) = match words.as_slice() {
                ["connect"] => (0, json!({"ok":true})),
                ["device", "look-ssh"] if runs => (0, json!({"ok":true,"value":{"look":look}})),
                ["device", "look-ssh"] => {
                    for (key, value) in self.look.as_object().unwrap() {
                        look[key] = value.clone();
                    }
                    (
                        1,
                        json!({"ok":false,"error":{"code":"hide_unavailable"},"look":look}),
                    )
                }
                // Another machine holds the helper's folder: the switch
                // reached the pairing, which is all these tests follow.
                ["device", "accept-ssh"] => (
                    0,
                    json!({"ok":true,"value":{"helper":"held","by":"Studio"}}),
                ),
                other => return Err(format!("not scripted: {other:?}")),
            };
            Ok(Ran {
                status,
                stdout: line(answer),
                stderr: Vec::new(),
            })
        }

        fn first(&self, _own: &[Candidate]) -> Vec<Candidate> {
            Vec::new()
        }

        fn install(&self, _packages: &HelperPackages, _root: &str) -> Result<String, NotInstalled> {
            Err(NotInstalled::Failed("not scripted".to_owned()))
        }
    }

    /// One switch on `login` that the operator did not allow a first start
    /// for.
    fn switch_of(login: &Stopped) -> Result<Switched, &'static str> {
        let reach = Reach {
            home: PathBuf::from("/nonexistent"),
            command_folder: None,
            hide: None,
        };
        let me = Me {
            device: "this-device".to_owned(),
            name: "This Mac".to_owned(),
        };
        let key = PeerKey::from_bytes([7; 32]);
        switch_on(
            login,
            &Switch {
                reach: &reach,
                request: "req-1",
                me: &me,
                key: &key,
                helper_root: Some("~/.hide/device"),
                projects: &[],
                admitted: &|_| false,
                install: false,
                step: &|_| {},
            },
        )
    }

    /// A switch that ended after it started the target's Hide and before
    /// its pairing leaves a Hide nothing keeps running and nobody can open
    /// on a machine with no app. The next switch starts it again and goes
    /// on to the pairing; a Hide an app there opens is left to its app, as
    /// is one that does not say, and a Mac's is started only under the rule
    /// its first start is held to.
    #[test]
    fn a_switch_starts_again_a_stopped_hide_that_no_app_opens() {
        let stopped = Stopped::new("linux", json!({"app": false}));
        let done = switch_of(&stopped);
        assert!(
            matches!(&done, Ok(Switched::HeldBy(by)) if by == "Studio"),
            "{:?}",
            done.err()
        );
        assert_eq!(
            stopped.asked(),
            [
                "device look-ssh",
                "connect",
                "device look-ssh",
                "device accept-ssh"
            ]
        );

        for look in [json!({"app": true}), json!({})] {
            let with_app = Stopped::new("linux", look);
            let done = switch_of(&with_app);
            assert_eq!(done.err(), Some("ssh_hide_not_running"));
            assert_eq!(with_app.asked(), ["device look-ssh"]);
        }

        let mac = Stopped::new("macos", json!({"app": false, "desktop": false}));
        let done = switch_of(&mac);
        assert_eq!(
            done.err(),
            install_refusal("macos", std::env::consts::OS, Some(false))
        );
        assert_eq!(mac.asked(), ["device look-ssh"]);
    }

    /// The rule's table (operator decision 2026-11-24), on what a target
    /// says of itself: a Mac is refused only while nobody is logged in to
    /// it and warned about while it does not log the account in by itself,
    /// with the way to turn that on unless FileVault rules it out; a Linux
    /// account lingers, or is asked to by the switch, and a system with no
    /// systemd is refused whatever it said; an account with no login item
    /// and a system this build does not switch are not switched.
    #[test]
    fn the_rule_reads_each_system_by_its_own_facts() {
        let of = |system: &str, said: Value| condition_of(system, &said, || true);
        let mac = |unattended: bool, desktop: bool, offered: bool| json!({"unattended": unattended, "desktop": desktop, "automatic_login_offered": offered});
        let table = [
            (
                of("macos", mac(true, true, true)),
                Condition::AutoLoginOn,
                true,
            ),
            (
                of("macos", mac(false, true, true)),
                Condition::AutoLoginOff { how: true },
                true,
            ),
            (
                of("macos", mac(false, true, false)),
                Condition::AutoLoginOff { how: false },
                true,
            ),
            // Nobody is logged in: refused, also with automatic login on.
            (
                of("macos", mac(false, false, true)),
                Condition::SignedOut,
                false,
            ),
            (
                of("macos", mac(true, false, true)),
                Condition::SignedOut,
                false,
            ),
            (
                of("macos", json!({"unattended": false})),
                Condition::Unknown,
                false,
            ),
            (
                of("linux", json!({"unattended": true, "desktop": true})),
                Condition::LingerOn,
                true,
            ),
            (
                of("linux", json!({"unattended": false, "desktop": true})),
                Condition::LingerOff,
                true,
            ),
            (
                of("linux", json!({"unattended": false})),
                Condition::Unknown,
                false,
            ),
            (
                of("freebsd", json!({"unattended": true, "desktop": true})),
                Condition::Unknown,
                false,
            ),
        ];
        for (read, expected, may) in table {
            assert_eq!(read, expected);
            assert_eq!(read.may(), may, "{read:?}");
        }
        // No systemd is asked only of an account that does not linger.
        let none = |said: Value| condition_of("linux", &said, || false);
        assert_eq!(
            none(json!({"unattended": false, "desktop": true})),
            Condition::NoSystemd
        );
        assert_eq!(
            none(json!({"unattended": true, "desktop": true})),
            Condition::LingerOn
        );
        for refused in [
            Condition::NoSystemd,
            Condition::Windows,
            Condition::LingerAdmin {
                command: "sudo loginctl enable-linger hy".to_owned(),
            },
        ] {
            assert!(!refused.may(), "{refused:?}");
        }
        assert_eq!(
            lingers_now(&json!({"linger": "needs_administrator", "command": "sudo loginctl enable-linger hy"})).payload(),
            json!({"kind": "linger_admin", "command": "sudo loginctl enable-linger hy"})
        );
        assert_eq!(lingers_now(&json!({"linger": "on"})), Condition::LingerOn);
        assert_eq!(
            lingers_now(&json!({"linger": "needs_administrator"})),
            Condition::Unknown
        );
        assert_eq!(
            Condition::AutoLoginOff { how: false }.payload(),
            json!({"kind": "auto_login_off", "how": false})
        );
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

    /// A helper device's account where Hide's node of this build runs and
    /// the `hide` the account links is still a 1.2.11 helper's, as on the
    /// Mac mini of with-hide/hide#3007, whose kit never pointed the link at
    /// the newer build: it records which `hide` answered what, and its own
    /// Hide runs once a `hide connect` was asked of the node's.
    struct Linked {
        asked: std::sync::Mutex<Vec<(&'static str, String)>>,
    }

    /// Where Linked's node runs from, and the `hide` beside it.
    const NODE: &str = "/home/me/.hide/host-helper/0123456789abcdef/hided";
    const NODE_HIDE: &str = "/home/me/.hide/host-helper/0123456789abcdef/hide";

    impl Linked {
        fn asked(&self) -> Vec<(&'static str, String)> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Login for Linked {
        fn channel(&self) -> Channel {
            Channel::Ssh
        }

        fn name(&self) -> &str {
            "mini"
        }

        fn system(&self) -> Result<String, String> {
            Ok("linux".to_owned())
        }

        fn on(&self) -> Result<bool, String> {
            Ok(true)
        }

        fn run(&self, argv: &[String], _cap: usize, _within: Duration) -> Result<Ran, String> {
            let (who, words) = match argv {
                [hide, words @ ..] if hide == NODE_HIDE => ("node", words),
                [sh, _, _, link, words @ ..] if sh == "sh" && link == ".local/bin/hide" => {
                    ("link", words)
                }
                other => return Err(format!("not scripted: {other:?}")),
            };
            let words: Vec<&str> = words.iter().map(String::as_str).take(2).collect();
            let mut asked = self.asked.lock().unwrap();
            let runs = asked.contains(&("node", "connect".to_owned()));
            asked.push((who, words.join(" ")));
            let (status, answer) = match (who, words.as_slice()) {
                // 1.2.11 knows no `always-on unattended`, and prints its
                // usage; it keeps every verb that pairs behind its switch.
                ("link", ["always-on", ..]) => (2, None),
                ("link", ["device", ..]) => (
                    1,
                    Some(json!({"ok":false,"error":{
                        "code":"device_connect_unavailable",
                        "message":"This build does not offer connections between two Hides yet: add a machine by its SSH alias",
                    }})),
                ),
                ("node", ["always-on", "unattended"]) => {
                    (0, Some(json!({"ok":true,"value":{"unattended":true}})))
                }
                ("node", ["connect"]) => (0, Some(json!({"ok":true}))),
                ("node", ["device", "look-ssh"]) if runs => (
                    0,
                    Some(json!({"ok":true,"value":{"look":{"system":"linux","core":true}}})),
                ),
                ("node", ["device", "look-ssh"]) => (
                    1,
                    Some(json!({"ok":false,"error":{"code":"hide_unavailable"},
                        "look":{"system":"linux","core":false}})),
                ),
                // Another machine holds the helper's folder: the switch
                // reached the pairing, which is all this test follows.
                ("node", ["device", "accept-ssh"]) => (
                    0,
                    Some(json!({"ok":true,"value":{"helper":"held","by":"Studio"}})),
                ),
                other => return Err(format!("not scripted: {other:?}")),
            };
            Ok(Ran {
                status,
                stdout: answer.map(line).unwrap_or_default(),
                stderr: Vec::new(),
            })
        }

        fn first(&self, _own: &[Candidate]) -> Vec<Candidate> {
            Vec::new()
        }

        fn install(&self, _packages: &HelperPackages, _root: &str) -> Result<String, NotInstalled> {
            Err(NotInstalled::Failed("not scripted".to_owned()))
        }
    }

    /// A helper device's read and switch run the `hide` beside its node,
    /// this machine's own build, so a `hide` the account still links to an
    /// older helper neither refuses the switch nor starts that older build
    /// (with-hide/hide#3007); with no node of this machine's known there,
    /// the account's own `hide` answers, and a 1.2.11 one reads as a Hide
    /// to update, never as a refusal with nothing to do.
    #[test]
    fn a_helper_devices_switch_runs_the_hide_beside_its_node_not_the_linked_one() {
        let reach = Reach {
            home: PathBuf::from("/nonexistent"),
            command_folder: None,
            hide: None,
        };
        let me = Me {
            device: "this-device".to_owned(),
            name: "This Mac".to_owned(),
        };
        let key = PeerKey::from_bytes([7; 32]);
        let switched = |login: &Linked, reach: &Reach| {
            switch_on(
                login,
                &Switch {
                    reach,
                    request: "req-1",
                    me: &me,
                    key: &key,
                    helper_root: Some("~/.hide/host-helper"),
                    projects: &[],
                    admitted: &|_| false,
                    install: true,
                    step: &|_| {},
                },
            )
        };

        let node = Linked {
            asked: std::sync::Mutex::default(),
        };
        let running = reach.running(Some(NODE));
        assert_eq!(
            may_move(&node, &running, Asked::Switch),
            Ok(Move {
                condition: Condition::LingerOn,
                user: None,
            })
        );
        let done = switched(&node, &running);
        assert!(
            matches!(&done, Ok(Switched::HeldBy(by)) if by == "Studio"),
            "{:?}",
            done.err()
        );
        let asked: Vec<(&str, String)> = [
            "always-on unattended",
            "device look-ssh",
            "connect",
            "device look-ssh",
            "device accept-ssh",
        ]
        .into_iter()
        .map(|words| ("node", words.to_owned()))
        .collect();
        assert_eq!(node.asked(), asked);

        let linked = Linked {
            asked: std::sync::Mutex::default(),
        };
        let unknown = reach.running(None);
        assert_eq!(
            may_move(&linked, &unknown, Asked::Switch),
            Err("ssh_update_there")
        );
        assert_eq!(switched(&linked, &unknown).err(), Some("ssh_update_there"));
        assert_eq!(
            linked.asked(),
            [
                ("link", "always-on unattended".to_owned()),
                ("link", "device look-ssh".to_owned()),
            ]
        );
    }

    /// The `hide` beside a node is named as the node's own program is, so a
    /// Windows node's is the `.exe` beside it; a program with no folder
    /// names none.
    #[test]
    fn the_hide_beside_a_node_is_spelled_as_its_program_is() {
        assert_eq!(hide_beside(NODE).as_deref(), Some(NODE_HIDE));
        assert_eq!(
            hide_beside("C:/Users/me/.hide/host-helper/0123456789abcdef/hided.exe").as_deref(),
            Some("C:/Users/me/.hide/host-helper/0123456789abcdef/hide.exe")
        );
        assert_eq!(hide_beside("hided"), None);
        let reach = Reach {
            home: PathBuf::from("/Users/example"),
            command_folder: Some("/opt/hide/bin".to_owned()),
            hide: None,
        };
        assert_eq!(program(&reach.running(Some(NODE)), "linux"), [NODE_HIDE]);
        assert_eq!(
            program(&reach.running(None), "linux"),
            ["/opt/hide/bin/hide"]
        );
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
