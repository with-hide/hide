//! `hide device`: the machines this one is connected to, for the operator at
//! a terminal and for an agent the operator asked.
//!
//! Two owners answer. Pairing with another Hide (`share`, `add <link>`,
//! `allow`, `pause`, `resume`, `request`, `address`, and `remove` or `retry`
//! of a paired machine) is this machine's own daemon's (`crate::pairing`),
//! asked over the bootstrap socket; sharing, adding a link, allowing,
//! resuming, asking back and naming an address are the operator's alone and
//! refused in a Hide pane (PRD device-connect-iroh B52). The SSH devices
//! (`list`, `add <ssh alias>`, and `remove` or `retry` of one) are the
//! core's (`hide_core::device_command`).
//!
//! This file reads the words, asks, waits where a command waits (a share
//! for its guest, a link for its words, a retry for its outcome), and
//! prints the answer: one JSON line with `--json`, lines for a person
//! without.
use crate::env::Env;
use hide_core::device_command::DeviceCommand;
use serde_json::{Value, json};
use std::io::Write as _;
use std::time::{Duration, Instant};

/// Every verb a person types, which `hide --help` and the agent guide list.
pub const USAGE: &str = "hide device list [--json]\nhide device share [--json]\nhide device add <link or ssh alias> [--name <name>] [--json]\nhide device allow [--json]\nhide device pause <device> [--outgoing|--incoming|--both] [--json]\nhide device resume <device> [--outgoing|--incoming|--both] [--json]\nhide device remove <device> [--outgoing|--incoming|--both] [--json]\nhide device request <device> [--json]\nhide device update <device> [--json]\nhide device address <device> <ip:port> [--json]\nhide device rename <device> <name> [--json]\nhide device retry <device> [--wait] [--json]\nhide device switch <device> [--install] [--json]";

/// How long `retry --wait` waits for an attempt to connect, reach the
/// operator's turn or fail: past ssh connecting and a first install of
/// Hide's helper over a slow link. What waits on the operator returns at
/// once, so this bounds only the attempt's own work.
const WAIT_BOUND: Duration = Duration::from_secs(300);
const WAIT_POLL: Duration = Duration::from_millis(500);

/// How long `switch` waits for its answer: the sum of its parts, each
/// bounded where it runs, and half a minute over. A login that asks the
/// operator on the device's row can outlast it; the switch then goes on in
/// the daemon and the row shows where it stands.
const SWITCH_BOUND: Duration = Duration::from_secs(
    crate::pairing_ssh::SWITCH_SSH_WITHIN.as_secs()
        + crate::pairing::SWITCH_CONNECTS_WITHIN.as_secs()
        + 30,
);

/// How long `projects-ssh` works: under what the machine that runs it over
/// SSH waits for (`pairing_ssh::PROJECTS_WITHIN`).
const PROJECTS_BOUND: Duration = Duration::from_secs(45);

/// How long a share or a pasted link is followed: past the link's nine
/// minutes and the two an answer may take.
const PAIRING_BOUND: Duration = Duration::from_secs(12 * 60);

/// How a pairing link starts (`hide_peer::link`).
const LINK_PREFIX: &str = "hide://";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Outgoing,
    Incoming,
    Both,
}

impl Direction {
    fn name(self) -> &'static str {
        match self {
            Self::Outgoing => "outgoing",
            Self::Incoming => "incoming",
            Self::Both => "both",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum Verb {
    /// The core's own: the list, and adding an SSH device.
    Core(DeviceCommand),
    Share,
    Join {
        link: String,
    },
    Allow,
    Pause {
        device: String,
        direction: Option<Direction>,
    },
    Resume {
        device: String,
        direction: Option<Direction>,
    },
    /// A paired machine's direction, or an SSH device.
    Remove {
        device: String,
        direction: Option<Direction>,
    },
    Request {
        device: String,
    },
    /// Asks a paired machine whose Hide is too old to connect to update.
    Update {
        device: String,
    },
    Address {
        device: String,
        address: String,
    },
    /// A paired machine dialed again, or an SSH device's attempt.
    Retry {
        device: String,
    },
    /// The name this machine shows a paired machine or a device under.
    Rename {
        device: String,
        name: String,
    },
    /// An SSH device switched to the connection of the Hide on its machine
    /// (D-49). `install` starts a Hide on a machine that runs none of its
    /// own (D-45).
    Switch {
        device: String,
        install: bool,
    },
    /// What a machine about to pair over SSH runs on its target first: it
    /// changes nothing and says whether a Hide of the target's own runs
    /// (B7); not a command a person types.
    LookSsh,
    /// What a machine pairing over SSH or `wsl.exe` runs on its target
    /// (security spec 15, 17); not a command a person types.
    AcceptSsh {
        request: String,
        key: String,
        device: String,
        name: String,
        wsl: bool,
        /// The root of the device helper the caller dialed on this account,
        /// whose folder and terminals it asks for with the grant (D-49).
        helper: Option<String>,
        /// The grant the caller gives this machine, which then becomes its
        /// guest: taken only from the machine this core was moved off
        /// before 1.2.0 (D-46).
        grant: Option<String>,
    },
    /// What a machine that switched its device here runs once this machine
    /// admitted it (security spec 15); not a command a person types. The
    /// folders of this account it had registered as projects on its device
    /// here, which this machine's own Hide then lists too (D-48).
    ProjectsSsh {
        projects: Vec<String>,
    },
    /// What the PC of a WSL distribution runs in it through `wsl.exe` to
    /// reach its core (D-27): this command's input and output become the
    /// carrier of the connection between the two; not a command a person
    /// types.
    Carry,
}

#[derive(Debug, Eq, PartialEq)]
pub struct DeviceCli {
    pub verb: Verb,
    pub json: bool,
    /// `retry --wait`: answer the attempt's outcome, not its start.
    pub wait: bool,
}

pub fn parse<'a>(mut args: impl Iterator<Item = &'a String>) -> Result<DeviceCli, String> {
    let verb = args.next().map(String::as_str);
    let accept = verb == Some("accept-ssh");
    let hands = verb == Some("projects-ssh");
    let directed = matches!(verb, Some("pause" | "resume" | "remove"));
    let mut json = false;
    let mut wait = false;
    let mut wsl = false;
    let mut install = false;
    let mut helper = None;
    let mut projects = Vec::new();
    let mut grant = None;
    let mut direction = None;
    let (mut name, mut request, mut key, mut device) = (None, None, None, None);
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
            "--install" if verb == Some("switch") && !install => {
                install = true;
                continue;
            }
            "--wsl" if accept && !wsl => {
                wsl = true;
                continue;
            }
            "--helper" if accept && helper.is_none() => &mut helper,
            "--grant" if accept && grant.is_none() => &mut grant,
            "--project" if hands && projects.len() < hide_core::SWITCHED_PROJECTS => {
                match args.next() {
                    Some(value) if !value.starts_with('-') => projects.push(value.clone()),
                    _ => return Err(USAGE.to_owned()),
                }
                continue;
            }
            "--outgoing" | "--incoming" | "--both" if directed && direction.is_none() => {
                direction = Some(match arg.as_str() {
                    "--outgoing" => Direction::Outgoing,
                    "--incoming" => Direction::Incoming,
                    _ => Direction::Both,
                });
                continue;
            }
            "--name" if (verb == Some("add") || accept) && name.is_none() => &mut name,
            "--request" if accept && request.is_none() => &mut request,
            "--key" if accept && key.is_none() => &mut key,
            "--device" if accept && device.is_none() => &mut device,
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
    // A machine is named by its id, its name or the start of its
    // connection id: one word a shell passes whole.
    let named = |word: &String| {
        (!word.trim().is_empty() && word.len() <= 256 && !word.chars().any(char::is_control))
            .then(|| word.clone())
            .ok_or_else(|| USAGE.to_owned())
    };
    let verb = match (verb, words.as_slice()) {
        (Some("list"), []) => Verb::Core(DeviceCommand::List),
        (Some("share"), []) => Verb::Share,
        (Some("allow"), []) => Verb::Allow,
        (Some("add"), [link]) if link.starts_with(LINK_PREFIX) => {
            // A name is the SSH device's; a link's machine says its own.
            if name.is_some() || link.len() > hide_peer::limits::MAX_LINK_BYTES {
                return Err(USAGE.to_owned());
            }
            Verb::Join { link: link.clone() }
        }
        (Some("add"), [alias]) => Verb::Core(DeviceCommand::Add {
            ssh_alias: alias.clone(),
            name,
        }),
        (Some("pause"), [device]) => Verb::Pause {
            device: named(device)?,
            direction,
        },
        (Some("resume"), [device]) => Verb::Resume {
            device: named(device)?,
            direction,
        },
        (Some("remove"), [device]) => Verb::Remove {
            device: named(device)?,
            direction,
        },
        (Some("request"), [device]) => Verb::Request {
            device: named(device)?,
        },
        (Some("update"), [device]) => Verb::Update {
            device: named(device)?,
        },
        (Some("address"), [device, address]) => {
            if address.parse::<std::net::SocketAddr>().is_err() {
                return Err(USAGE.to_owned());
            }
            Verb::Address {
                device: named(device)?,
                address: address.clone(),
            }
        }
        (Some("retry"), [device]) => Verb::Retry {
            device: named(device)?,
        },
        // An empty name takes a paired machine back to the one it gives itself.
        (Some("rename"), [device, name]) => Verb::Rename {
            device: named(device)?,
            name: match name.trim() {
                "" => String::new(),
                _ => hide_core::device_command::device_name(name)
                    .ok_or_else(|| USAGE.to_owned())?
                    .to_owned(),
            },
        },
        (Some("switch"), [device]) => Verb::Switch {
            device: named(device)?,
            install,
        },
        (Some("look-ssh"), []) if name.is_none() => Verb::LookSsh,
        (Some("carry"), []) if name.is_none() && !json => Verb::Carry,
        (Some("accept-ssh"), []) => match (request, key, device, name) {
            (Some(request), Some(key), Some(device), Some(name)) => Verb::AcceptSsh {
                request,
                key,
                device,
                name,
                wsl,
                helper,
                grant,
            },
            _ => return Err(USAGE.to_owned()),
        },
        (Some("projects-ssh"), []) if name.is_none() && !projects.is_empty() => {
            Verb::ProjectsSsh { projects }
        }
        _ => return Err(USAGE.to_owned()),
    };
    if let Verb::Core(command) = &verb
        && !command.valid()
    {
        return Err(USAGE.to_owned());
    }
    Ok(DeviceCli { verb, json, wait })
}

pub fn run(env: &Env, cli: &DeviceCli) -> Result<(), String> {
    // A carrier's output is the connection's bytes: nothing else is ever
    // printed there, and a refusal is the code this command ends with.
    if cli.verb == Verb::Carry {
        return carry(env);
    }
    let (code, message, device) = match answer(env, cli) {
        Ok(Printed::Done) => return Ok(()),
        Ok(Printed::Value(value, lines)) => {
            if cli.json {
                println!("{}", json!({"ok":true,"value":value}));
            } else {
                print!("{lines}");
            }
            return Ok(());
        }
        Err(Refused::Answer { code, message }) => (code, message, None),
        Err(Refused::Looked { code, look }) => {
            let message = crate::cli::request_next_action(&code).to_owned();
            println!(
                "{}",
                json!({"ok":false,"error":{"code":code,"message":message},"look":look})
            );
            return Err(code);
        }
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

/// What a verb leaves to print.
enum Printed {
    /// The answer, and its lines for a person.
    Value(Value, String),
    /// The verb printed as it went.
    Done,
}

fn answer(env: &Env, cli: &DeviceCli) -> Result<Printed, Refused> {
    let direction = |direction: &Option<Direction>| direction.map(Direction::name);
    match &cli.verb {
        Verb::Core(command @ DeviceCommand::Add { ssh_alias, .. }) => {
            add_alias(env, command, ssh_alias, cli.wait)
        }
        Verb::Core(command) => core(env, command, cli.wait),
        Verb::Share => share(env, cli.json),
        Verb::Join { link } => join(env, link, cli.json),
        Verb::Allow => allow(env),
        Verb::Pause {
            device,
            direction: to,
        } => {
            let value = pairing(
                env,
                json!({"verb":"pause","device":device,"direction":direction(to)}),
            )?;
            Ok(Printed::Value(
                value,
                format!("Paused {device}. Its work keeps running.\n"),
            ))
        }
        Verb::Resume {
            device,
            direction: to,
        } => {
            operators(env)?;
            let value = pairing(
                env,
                json!({"verb":"resume","device":device,"direction":direction(to)}),
            )?;
            Ok(Printed::Value(value, format!("Resumed {device}.\n")))
        }
        Verb::Remove {
            device,
            direction: to,
        } => {
            let paired = pairing(
                env,
                json!({"verb":"remove","device":device,"direction":direction(to)}),
            );
            match paired {
                Ok(value) => Ok(Printed::Value(
                    value,
                    format!(
                        "Removed {device}. What already ran there and what was read cannot be undone; its work keeps running.\n"
                    ),
                )),
                // No paired machine of that name: an SSH device, which has
                // no direction to name.
                Err(refused) if refused.unknown() && to.is_none() => core(
                    env,
                    &DeviceCommand::Remove {
                        device_id: device.clone(),
                    },
                    false,
                ),
                Err(refused) => Err(refused),
            }
        }
        Verb::Request { device } => {
            operators(env)?;
            let value = pairing(env, json!({"verb":"request","device":device}))?;
            Ok(Printed::Value(
                value,
                format!(
                    "Asked {device} to let this machine open it. Its operator answers there; hide device list shows what came of it.\n"
                ),
            ))
        }
        Verb::Update { device } => {
            operators(env)?;
            let value = pairing(env, json!({"verb":"update","device":device}))?;
            Ok(Printed::Value(
                value,
                format!(
                    "Asked {device} to update its Hide. It installs the official release and connects again; hide device list shows when it has.\n"
                ),
            ))
        }
        Verb::Address { device, address } => {
            operators(env)?;
            let value = pairing(
                env,
                json!({"verb":"address","device":device,"address":address}),
            )?;
            Ok(Printed::Value(
                value,
                format!("{device} is tried at {address} first.\n"),
            ))
        }
        Verb::Rename { device, name } => {
            let paired = pairing(env, json!({"verb":"rename","device":device,"name":name}));
            match paired {
                Ok(value) => {
                    let shown = value["name"].as_str().unwrap_or(name).to_owned();
                    Ok(Printed::Value(
                        value,
                        format!("{device} is shown as {shown} on this machine.\n"),
                    ))
                }
                // No paired machine of that name: an SSH or WSL device.
                Err(refused) if refused.unknown() => core(
                    env,
                    &DeviceCommand::Rename {
                        device_id: device.clone(),
                        name: name.clone(),
                    },
                    false,
                ),
                Err(refused) => Err(refused),
            }
        }
        Verb::Retry { device } => match pairing(env, json!({"verb":"retry","device":device})) {
            Ok(value) => Ok(Printed::Value(
                value,
                format!("Dialing {device} again; hide device list shows whether it connected.\n"),
            )),
            Err(refused) if refused.unknown() => core(
                env,
                &DeviceCommand::Retry {
                    device_id: device.clone(),
                },
                cli.wait,
            ),
            Err(refused) => Err(refused),
        },
        Verb::Switch { device, install } => {
            operators(env)?;
            let ask = json!({"verb":"switch","device":device,"install":install});
            let mut answer = crate::workspace_cli::pairing_ask_within(env, ask, SWITCH_BOUND)
                .map_err(Refused::Unreachable)?;
            if answer["ok"] != true {
                let code = answer["reason"]
                    .as_str()
                    .unwrap_or("device_unavailable")
                    .to_owned();
                let message = answer["next_action"].as_str().unwrap_or(&code).to_owned();
                return Err(Refused::Answer { code, message });
            }
            if let Some(fields) = answer.as_object_mut() {
                fields.remove("ok");
            }
            let lines = match answer["switch"].as_str() {
                Some("held") => format!(
                    "{} switched {device} first, and its terminals there carry on. Once every terminal this machine runs there is closed, Hide switches {device} by itself if it is still a device Hide moves.\n",
                    answer["by"].as_str().unwrap_or("Another computer"),
                ),
                Some("left") => left_line(device, &answer["needs"]),
                Some("install_needed") => format!(
                    "{device} runs no Hide of its own. Run hide device switch {device} --install to start one there from this machine's Hide program, which is already installed in that account, or install Hide there yourself and run the switch again.\n"
                ),
                _ => format!(
                    "Switched {device} to the Hide on its machine. Its running terminals carry on.\n"
                ),
            };
            Ok(Printed::Value(answer, lines))
        }
        Verb::Carry => unreachable!("run answers it before anything is printed"),
        Verb::LookSsh => {
            // Read by the machine that ran it, never by a person.
            let look = |core: bool| {
                let mut look = json!({"system": std::env::consts::OS, "core": core});
                // An account with no Hide running says whether its desktop
                // has someone logged in, which a machine reads before it
                // starts one here (D-45, B7); left out when this system
                // does not say.
                let desktop = (!core)
                    .then(|| {
                        crate::core_install::checks::desktop_session(
                            &env.home,
                            &std::sync::atomic::AtomicBool::new(false),
                        )
                    })
                    .flatten();
                if let Some(desktop) = desktop {
                    look["desktop"] = json!(desktop);
                }
                look
            };
            match pairing(env, json!({"verb":"status"})) {
                Ok(status) => {
                    let mut look = look(true);
                    // What the running core says of itself, which a node
                    // reads before it asks to be split from it
                    // (`pairing_ssh::core_on`); a core before 1.2.0 says
                    // nothing of it.
                    if let Some(core) = status["core"].as_object() {
                        for said in ["node", "build", "release"] {
                            look[said] = core.get(said).cloned().unwrap_or(Value::Null);
                        }
                    }
                    println!("{}", json!({"ok":true,"value":{"look":look}}));
                    Ok(Printed::Done)
                }
                // No daemon answered: a Hide of this account's own that is
                // not running, or an account that never ran one (a device
                // helper's `hide`), told apart by the daemon's own lock.
                Err(Refused::Unreachable(code)) => Err(Refused::Looked {
                    code,
                    look: look(hide_kit::layout::daemon_lock(&env.state_dir).exists()),
                }),
                Err(refused) => Err(refused),
            }
        }
        Verb::AcceptSsh {
            request,
            key,
            device,
            name,
            wsl,
            helper,
            grant,
        } => {
            let value = pairing(
                env,
                json!({
                    "verb":"accept_ssh","request":request,"key":key,
                    "device":device,"name":name,"wsl":wsl,"helper":helper,
                    "grant":grant,
                }),
            )?;
            // Read by the machine that ran it, never by a person.
            println!("{}", json!({"ok":true,"value":value}));
            Ok(Printed::Done)
        }
        Verb::ProjectsSsh { projects } => {
            let added = add_projects(env, projects, Instant::now() + PROJECTS_BOUND);
            // Read by the machine that ran it, never by a person.
            println!("{}", json!({"ok":true,"value":{"projects":added}}));
            Ok(Printed::Done)
        }
    }
}

/// Registers each of `projects`, folders of this account, as a project of
/// this machine's own Hide, and answers how many it now lists. One that is
/// gone, outside the account's home or refused is left out and logged. It
/// ends at `over`, each ask waiting for what is left of it, and a daemon
/// that does not answer one is not asked for the rest: the SSH login that
/// waits on this has a bound of its own (`pairing_ssh`), and the pairing
/// it follows stands whatever is listed.
fn add_projects(env: &Env, projects: &[String], over: Instant) -> usize {
    let Ok(Some(state)) = crate::state_file::read_state(&env.state_dir) else {
        return 0;
    };
    let not_added = |reason: &str| {
        eprintln!(
            "{}",
            json!({"component":"hide","kind":"device.project_not_added","reason":reason})
        );
    };
    let mut added = 0;
    for path in projects {
        let left = over.saturating_duration_since(Instant::now());
        if left.is_zero() {
            not_added("out_of_time");
            break;
        }
        match crate::workspace_cli::request_project_add_with_token(
            state.port,
            &state.token,
            path,
            left,
        ) {
            Ok(answer) if answer["ok"] == true => added += 1,
            Ok(answer) => not_added(answer["reason"].as_str().unwrap_or("refused")),
            Err(reason) => {
                not_added(&reason);
                break;
            }
        }
    }
    added
}

/// Joins this command's input and output to this account's core, as the
/// carrier of the connection the caller's core makes to it (D-27). It ends
/// when either end closes.
///
/// A core this account ran before and that is not running is started
/// first, as a login starts it: a distribution with no systemd has nothing
/// else that does, so its core lives while the distribution runs. An
/// account that never ran a core is left as it is.
fn carry(env: &Env) -> Result<(), String> {
    use std::io::Read as _;
    if env.pane_id.is_some() || env.access.is_some() {
        return Err("device_from_pane".to_owned());
    }
    use crate::workspace_cli::NoCarrier;
    let stream = match crate::workspace_cli::carrier(env) {
        Ok(stream) => stream,
        Err(NoCarrier::NoDaemon(_))
            if hide_kit::layout::daemon_lock(&env.state_dir).exists()
                && crate::cli::core_started(env) =>
        {
            crate::workspace_cli::carrier(env).map_err(NoCarrier::code)?
        }
        Err(refused) => return Err(refused.code()),
    };
    let mut to_core = stream.duplicate();
    let end = stream.shutdown_handle();
    // The caller's bytes to the core. When the caller's end closes the
    // core's is ended, which ends the other direction below.
    std::thread::Builder::new()
        .name("carry-in".to_owned())
        .spawn(move || {
            let _ = std::io::copy(&mut std::io::stdin().lock(), &mut to_core);
            end.shutdown();
        })
        .map_err(|_| "hide_unavailable".to_owned())?;
    let mut from_core = stream;
    let mut output = std::io::stdout().lock();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        match from_core.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let written = output.write_all(&buffer[..read]);
                if written.and_then(|()| output.flush()).is_err() {
                    break;
                }
            }
        }
    }
    // The thread that still waits for the caller's input ends with this
    // process.
    Ok(())
}

enum Refused {
    /// A refusal, with what the caller does about it.
    Answer { code: String, message: String },
    /// No answer arrived, for this reason code.
    Unreachable(String),
    /// `look-ssh` reached no daemon: the reason code, and what the command
    /// itself could say of this account.
    Looked { code: String, look: Value },
    /// `retry --wait` reached its bound while the attempt still ran; the
    /// device as last read.
    WaitedOut(Value),
}

impl Refused {
    fn code(code: &str) -> Self {
        Self::Answer {
            code: code.to_owned(),
            message: crate::pairing::next_action(code).to_owned(),
        }
    }

    /// Whether the daemon's pairing knows no machine of that name.
    fn unknown(&self) -> bool {
        matches!(self, Self::Answer { code, .. } if code == "device_unknown")
    }
}

/// Refuses what only the operator may ask before anything is sent, where
/// this command's own environment says it runs in a pane or as an external
/// tool; the daemon refuses a pane again by what the kernel says of the
/// caller (B52).
fn operators(env: &Env) -> Result<(), Refused> {
    if env.pane_id.is_some() {
        return Err(Refused::code("device_from_pane"));
    }
    // A bot, named or the one key this home keeps, in either mode.
    if crate::access_cli::tool_key(env)
        .map_err(|code| Refused::code(&code))?
        .is_some()
    {
        return Err(Refused::code(hide_core::access::OPERATOR_ONLY));
    }
    Ok(())
}

/// Adds a machine by its SSH alias (B7). One whose own Hide runs is paired
/// over SSH, which installs nothing there; any other is the core's SSH
/// device, as before, with a line when its Hide could pair once it is
/// updated or running. A command the daemon refuses to look for (a pane's)
/// goes to the core unchanged.
fn add_alias(
    env: &Env,
    command: &DeviceCommand,
    alias: &str,
    wait: bool,
) -> Result<Printed, Refused> {
    let looked = match operators(env) {
        Ok(()) => pairing(env, json!({"verb":"look_ssh","alias":alias})).ok(),
        Err(_) => None,
    };
    match looked
        .as_ref()
        .and_then(|looked| looked["looked"]["state"].as_str())
    {
        Some("ready") => {
            let mut value = pairing(env, json!({"verb":"pair_ssh","alias":alias}))?;
            value["paired"] = json!("ssh");
            let name = value["name"].as_str().unwrap_or(alias).to_owned();
            return Ok(Printed::Value(
                value,
                format!(
                    "{alias} already runs Hide: connected to {name}, and nothing was installed there. hide device list shows it.\n"
                ),
            ));
        }
        Some("update_there") => eprintln!(
            "Hide on {alias} is too old to connect as a Hide: update it there to connect that way. Adding it as an SSH device."
        ),
        Some("refused") => eprintln!(
            "Hide on {alias} refused the request that came over SSH (this machine's diagnostic log names its reason): connecting as a Hide needs it to answer. Adding it as an SSH device."
        ),
        Some("not_running") => eprintln!(
            "Hide is installed on {alias} and is not running: log in there and open Hide to connect as a Hide. Adding it as an SSH device."
        ),
        _ => {}
    }
    // The line says how an SSH device becomes its machine's own Hide: by
    // itself when that machine can take it, which the command reads now.
    match core(env, command, wait)? {
        Printed::Value(value, lines) => Ok(Printed::Value(
            value,
            format!(
                "{lines}{alias} is used over SSH. Hide connects the two as Hides by itself once the Hide on that machine would be back after a restart with nobody there; hide device switch {alias} reads that now.\n"
            ),
        )),
        printed => Ok(printed),
    }
}

/// One ask of this machine's daemon about pairing.
fn pairing(env: &Env, ask: Value) -> Result<Value, Refused> {
    let mut answer = crate::workspace_cli::pairing_ask(env, ask).map_err(Refused::Unreachable)?;
    if answer["ok"] == true {
        if let Some(fields) = answer.as_object_mut() {
            fields.remove("ok");
        }
        return Ok(answer);
    }
    let code = answer["reason"]
        .as_str()
        .unwrap_or("device_unavailable")
        .to_owned();
    let message = answer["next_action"].as_str().unwrap_or(&code).to_owned();
    Err(Refused::Answer { code, message })
}

/// A yes or no the operator types at this terminal. Anything but a yes,
/// a closed input included, is a no.
fn yes(question: &str) -> bool {
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Runs `question`, which waits for the operator at this terminal, and
/// calls `hold` every `every` until it is answered. The share a command
/// holds would otherwise end while its own question waits; when the
/// terminal goes away both stop, and the share ends as the daemon rules.
fn held_while<T>(
    every: Duration,
    mut hold: impl FnMut() + Send,
    question: impl FnOnce() -> T,
) -> T {
    let (answered, waits) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            while waits.recv_timeout(every) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                hold();
            }
        });
        let answer = question();
        // Dropped on a panic too, which ends the holder.
        drop(answered);
        answer
    })
}

/// The language this terminal reads, for the words two people compare.
fn language() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

fn words_of(value: &Value) -> Option<String> {
    let words: Vec<&str> = value.as_array()?.iter().filter_map(Value::as_str).collect();
    (!words.is_empty()).then(|| words.join("  "))
}

/// What a machine that asks would be allowed (D-76), and the question.
fn allowed(env: &Env, ask: &Value) -> Result<bool, Refused> {
    let name = ask["name"].as_str().unwrap_or_default();
    eprintln!(
        "{name} asks to open this machine. It could run commands and agents here and see the screen, and its Hide program and agent hooks are installed in this account."
    );
    if let Some(words) = words_of(&ask["words"]) {
        eprintln!("Words: {words}");
        eprintln!("Allow only if {name} shows the same four words in the same order.");
    }
    let allow = yes("Allow?");
    match pairing(env, json!({"verb":"answer","ask":ask["id"],"allow":allow})) {
        Ok(_) => Ok(allow),
        // It ended while the question was on the screen.
        Err(Refused::Answer { code, .. }) if code == "ask_gone" => {
            eprintln!("That request is over; nothing was allowed.");
            Ok(false)
        }
        Err(refused) => Err(refused),
    }
}

/// `hide device share` (B1): prints a link, asks about the machine that
/// pastes it, and waits until that machine is let in or the link ends. The
/// link ends with this command.
#[allow(clippy::disallowed_methods)] // polls the daemon's pairing, bounded by PAIRING_BOUND
fn share(env: &Env, json_line: bool) -> Result<Printed, Refused> {
    operators(env)?;
    let shared = pairing(
        env,
        json!({"verb":"share","language":language(),"held":true}),
    )?;
    let link = shared["link"].as_str().unwrap_or_default();
    if json_line {
        println!("{}", json!({"ok":true,"value":shared}));
    } else {
        println!(
            "Paste this link on the other machine: Settings > Devices > Add a device, or hide device add <link>.\nIt works once, for nine minutes, and ends when this command does.\n\n{link}\n"
        );
    }
    let _ = std::io::stdout().flush();
    let deadline = Instant::now() + PAIRING_BOUND;
    let mut answered: Vec<Value> = Vec::new();
    let mut let_in: Option<(String, Instant)> = None;
    loop {
        let status = pairing(env, json!({"verb":"status","hold":true}))?;
        let asks = status["pairing"]["asks"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for ask in asks.iter().filter(|ask| ask["kind"] == "link") {
            if answered.contains(&ask["id"]) {
                continue;
            }
            answered.push(ask["id"].clone());
            // The question may wait longer than the share lives unheld.
            let hold = || {
                let _ = pairing(env, json!({"verb":"status","hold":true}));
            };
            if held_while(WAIT_POLL, hold, || allowed(env, ask))? {
                let key = ask["key"].as_str().unwrap_or_default().to_owned();
                let_in = Some((key, Instant::now()));
                eprintln!("Waiting for the other machine to confirm...");
            }
        }
        if let Some((key, since)) = &let_in {
            let machine = status["machines"].as_array().and_then(|machines| {
                machines.iter().find(|machine| {
                    machine["connection_id"] == **key && !machine["opened_by"].is_null()
                })
            });
            if let Some(machine) = machine {
                eprintln!(
                    "Connected: {} opens this machine.",
                    machine["name"].as_str().unwrap_or_default()
                );
                return Ok(Printed::Done);
            }
            // The other operator's two minutes, and the confirm's way back.
            if since.elapsed() > Duration::from_secs(150) {
                return Err(Refused::code("pairing_ended"));
            }
        } else if status["pairing"]["offer"].is_null() && asks.is_empty() {
            return Err(Refused::code("pairing_ended"));
        }
        if Instant::now() >= deadline {
            let _ = pairing(env, json!({"verb":"end_share"}));
            return Err(Refused::code("pairing_ended"));
        }
        std::thread::sleep(WAIT_POLL);
    }
}

/// `hide device add <link>` (B2): shows the words and settles on the
/// operator's answer.
#[allow(clippy::disallowed_methods)] // polls the daemon's pairing, bounded by PAIRING_BOUND
fn join(env: &Env, link: &str, json_line: bool) -> Result<Printed, Refused> {
    operators(env)?;
    pairing(env, json!({"verb":"join","link":link}))?;
    let deadline = Instant::now() + PAIRING_BOUND;
    let mut compared = false;
    loop {
        let status = pairing(env, json!({"verb":"status"}))?;
        let join = &status["pairing"]["join"];
        let name = join["name"]
            .as_str()
            .unwrap_or("the other machine")
            .to_owned();
        match join["phase"].as_str() {
            Some("comparing") if !compared => {
                compared = true;
                if let Some(words) = words_of(&join["words"]) {
                    eprintln!("Words: {words}");
                }
                let same = yes(&format!(
                    "Does {name} show the same four words in the same order?"
                ));
                match pairing(env, json!({"verb":"compare","same":same})) {
                    Ok(_) if same => eprintln!("Waiting for {name} to allow it..."),
                    Ok(_) => {}
                    Err(Refused::Answer { code, .. }) if code == "ask_gone" => {}
                    Err(refused) => return Err(refused),
                }
            }
            Some(phase @ ("paired" | "waiting")) => {
                let _ = pairing(env, json!({"verb":"dismiss_join"}));
                let lines = if phase == "paired" {
                    format!("Connected: this machine opens {name}.\n")
                } else {
                    format!(
                        "Saved. {name} has not confirmed yet; Hide keeps trying and hide device list shows when it is connected.\n"
                    )
                };
                let value = json!({"host": join["host"], "name": name, "phase": phase});
                if json_line {
                    println!("{}", json!({"ok":true,"value":value}));
                    return Ok(Printed::Done);
                }
                return Ok(Printed::Value(value, lines));
            }
            Some("failed") => {
                let failure = join["failure"].as_str().unwrap_or("unreachable");
                let _ = pairing(env, json!({"verb":"dismiss_join"}));
                return Err(Refused::code(&format!("pairing_{failure}")));
            }
            Some(_) => {}
            None => return Err(Refused::code("pairing_ended")),
        }
        if Instant::now() >= deadline {
            let _ = pairing(env, json!({"verb":"dismiss_join"}));
            return Err(Refused::code("pairing_ended"));
        }
        std::thread::sleep(WAIT_POLL);
    }
}

/// `hide device allow` (B50): answers the machines that wait for this
/// machine's operator, on a machine with no window to answer in.
fn allow(env: &Env) -> Result<Printed, Refused> {
    operators(env)?;
    let status = pairing(env, json!({"verb":"status"}))?;
    let asks = status["pairing"]["asks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut answers = Vec::new();
    for ask in &asks {
        let allow = allowed(env, ask)?;
        answers.push(json!({"device_id": ask["device"], "name": ask["name"], "allowed": allow}));
    }
    let lines = if answers.is_empty() {
        "No machine is asking.\n".to_owned()
    } else {
        answers
            .iter()
            .map(|answer| {
                format!(
                    "{} {}\n",
                    if answer["allowed"] == true {
                        "Allowed"
                    } else {
                        "Refused"
                    },
                    answer["name"].as_str().unwrap_or_default()
                )
            })
            .collect()
    };
    Ok(Printed::Value(json!({"answered": answers}), lines))
}

/// The machines a terminal with no pane and no project of its own still
/// lists (B24): the paired ones, which this machine's daemon answers
/// whoever asks at it. The SSH devices are the core's to list.
fn paired_only(env: &Env) -> Option<Value> {
    let status = pairing(env, json!({"verb":"status"})).ok()?;
    Some(json!({"devices": [], "peers": status["machines"]}))
}

/// A command the core answers, with a retry's outcome when asked.
fn core(env: &Env, command: &DeviceCommand, wait: bool) -> Result<Printed, Refused> {
    let mut value = match (ask(env, command), command) {
        (Err(Refused::Unreachable(code)), DeviceCommand::List) => {
            paired_only(env).ok_or(Refused::Unreachable(code))?
        }
        (answer, _) => answer?,
    };
    if let (true, DeviceCommand::Retry { .. }) = (wait, command) {
        value["device"] = outcome(env, value["device"].clone())?;
    }
    let lines = draw(command, &value);
    Ok(Printed::Value(value, lines))
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
/// A device added by an alias takes its machine's id on its first
/// connection, so it is followed by its alias as well as its id.
fn outcome(env: &Env, mut device: Value) -> Result<Value, Refused> {
    let deadline = Instant::now() + WAIT_BOUND;
    let id = device["id"].clone();
    let alias = device["ssh_alias"].clone();
    let same = |row: &&Value| row["id"] == id || (alias.is_string() && row["ssh_alias"] == alias);
    while device["state"] == "connecting" {
        if Instant::now() >= deadline {
            return Err(Refused::WaitedOut(device));
        }
        std::thread::sleep(WAIT_POLL);
        let list = ask(env, &DeviceCommand::List)?;
        device = list["devices"]
            .as_array()
            .and_then(|devices| devices.iter().find(same))
            .cloned()
            // Removed while the attempt ran.
            .ok_or_else(|| Refused::Answer {
                code: "device_unknown".to_owned(),
                message: hide_core::device_command::next_action("device_unknown").to_owned(),
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
            let peers = value["peers"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            let guests = value["guest_panes"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if devices.is_empty() && peers.is_empty() && guests.is_empty() {
                return "No machine is connected.\n".to_owned();
            }
            let paired = |guest: &Value| {
                peers
                    .iter()
                    .any(|peer| peer["connection_id"] == guest["connection_id"])
            };
            peers
                .iter()
                // A machine that is still a helper device of this computer
                // is that device's line below, and has none of its own
                // (`PeerMachine::helper_device`).
                .filter(|peer| peer["helper_device"].is_null())
                .map(|peer| {
                    let panes: String = guests
                        .iter()
                        .filter(|guest| guest["connection_id"] == peer["connection_id"])
                        .map(guest_lines)
                        .collect();
                    peer_line(peer) + &panes
                })
                // A machine that was taken off and left panes running here.
                .chain(guests.iter().filter(|guest| !paired(guest)).map(|guest| {
                    format!(
                        "{}  no longer paired\n{}",
                        guest["id"].as_str().unwrap_or_default(),
                        guest_lines(guest)
                    )
                }))
                // A paired machine's device is that machine's line above,
                // as Settings lists it (`listedDevices` in the web shell):
                // one machine, one line, whatever id each goes by. A device
                // that was switched to its machine's own Hide is such a
                // device; one the core still dials itself keeps its line,
                // which is its machine's only one while a move that paired
                // the two has not taken the device (the filter above).
                .chain(
                    devices
                        .iter()
                        .filter(|device| device["connection_id"].is_null())
                        .map(device_line),
                )
                .collect()
        }
        DeviceCommand::Add { .. } => format!(
            "Added {}{}\n",
            device_line(&value["device"]),
            value["next_action"].as_str().unwrap_or_default()
        ),
        DeviceCommand::Remove { device_id } => format!(
            "Removed {device_id}. Its panes, agents and files on the device keep running.\n"
        ),
        DeviceCommand::Rename { device_id, name } => {
            format!("{device_id} is shown as {} on this machine.\n", name.trim())
        }
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

/// Why a switch left `device` on SSH, and the one thing that changes it
/// (operator decision 2026-11-24): the rule Hide's own move follows, said
/// to the one who asked at a terminal. `needs` is what the machine said it
/// needs (`pairing_ssh::Needs::payload`), null where nothing set there
/// would move it or it could not be read.
fn left_line(device: &str, needs: &Value) -> String {
    let stays = format!(
        "{device} is not switched and is used over SSH as before: the Hide on its machine would not be back after a restart with nobody there."
    );
    let then = format!("then run hide device switch {device} again");
    match (needs["kind"].as_str(), needs["command"].as_str()) {
        (Some("auto_login"), _) => {
            format!("{stays} Turn on automatic login for that account on the Mac, {then}.\n")
        }
        (Some("linger"), Some(command)) => {
            format!("{stays} Run {command} on that machine, {then}.\n")
        }
        _ => format!("{stays}\n"),
    }
}

/// A paired machine: who opens whom, whether the two are connected, and the
/// path the connection takes now in one word (D-86).
fn peer_line(peer: &Value) -> String {
    let text = |key: &str| peer[key].as_str().unwrap_or_default();
    let mut line = format!(
        "{}  {}  {}\n",
        text("id"),
        text("name"),
        match (peer["connected"] == true, text("path")) {
            (true, "direct") => "connected · direct",
            (true, "tailscale") => "connected · Tailscale",
            (true, "relay") => "connected · relay",
            // A WSL distribution and its PC: one machine, no path to name.
            (true, _) => "connected",
            (false, _) => "not connected",
        },
    );
    if let Some(opens) = peer["opens"].as_str() {
        line.push_str(&format!(
            "  this machine opens it: {}\n",
            opens.replace('_', " ")
        ));
    }
    if let Some(opened_by) = peer["opened_by"].as_str() {
        line.push_str(&format!("  it opens this machine: {opened_by}\n"));
    }
    if let Some(reason) = peer["reason"].as_str() {
        line.push_str(&format!("  reason: {}\n", reason.replace('_', " ")));
    }
    match peer["update"].as_str() {
        Some("here") => line.push_str("  update needed: this machine\n"),
        Some("here_asked") => line.push_str(
            "  update needed: this machine, and that machine asked for it; open Hide here to update\n",
        ),
        Some("there") => line.push_str(
            "  update needed: that machine; hide device update asks it to update when this machine opens it\n",
        ),
        Some("there_asked") => {
            line.push_str("  update needed: that machine, which this machine asked to update\n")
        }
        // The Hide this machine started there, kept at this machine's
        // build (B23).
        Some("updating") => line.push_str("  its Hide is being updated to this machine's build\n"),
        Some(failed) if failed.starts_with("update_") => line.push_str(&format!(
            "  its Hide was not updated to this machine's build ({}); hide device retry asks again\n",
            match failed {
                "update_not_reached" => "this machine's login did not reach it",
                "update_rolled_back" => "the new build did not start there, and the one before runs",
                "update_busy" => "another update runs there now",
                _ => "the diagnostic log says why",
            }
        )),
        _ => {}
    }
    line
}

/// The panes of a machine's work on this one (PRD device-connect-iroh D-77
/// (5)), each by the address `hide pane send` takes.
fn guest_lines(guest: &Value) -> String {
    guest["panes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pane| pane["id"].as_str())
        .map(|pane| format!("  its pane here: {pane}\n"))
        .collect()
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

    /// D-86, B39: a connected machine's line names the path its connection
    /// takes now in one word; one that is not connected names none.
    #[test]
    fn a_connected_machine_s_line_names_its_path() {
        let first = |peer: Value| peer_line(&peer).lines().next().unwrap().to_owned();
        let peer = |connected: bool, path: Value| json!({"id": "m1", "name": "Mac mini", "connected": connected, "path": path});
        assert_eq!(
            first(peer(true, json!("direct"))),
            "m1  Mac mini  connected · direct"
        );
        assert_eq!(
            first(peer(true, json!("tailscale"))),
            "m1  Mac mini  connected · Tailscale"
        );
        assert_eq!(
            first(peer(true, json!("relay"))),
            "m1  Mac mini  connected · relay"
        );
        assert_eq!(
            first(peer(false, Value::Null)),
            "m1  Mac mini  not connected"
        );
    }

    /// `projects-ssh` takes each folder the asking machine had registered
    /// as a project there (D-48), up to as many as a core adds at once; a
    /// project is a folder, never a flag, and no other verb takes one: the
    /// pairing's `accept-ssh` answers within one SSH answer's bound.
    #[test]
    fn projects_ssh_takes_the_projects_the_asking_machine_registered() {
        let parsed = parse_words(&[
            "projects-ssh",
            "--project",
            "/home/me/app",
            "--project",
            "/home/me/site",
        ])
        .unwrap();
        assert!(
            matches!(&parsed.verb, Verb::ProjectsSsh { projects } if projects == &["/home/me/app", "/home/me/site"]),
            "{parsed:?}"
        );
        assert!(parse_words(&["projects-ssh"]).is_err());
        assert!(parse_words(&["projects-ssh", "--project"]).is_err());
        assert!(parse_words(&["projects-ssh", "--project", "--json"]).is_err());
        assert!(parse_words(&["list", "--project", "/home/me/app"]).is_err());
        let accept = [
            "accept-ssh",
            "--request",
            "r1",
            "--key",
            "k1",
            "--device",
            "d1",
            "--name",
            "Mac",
        ];
        assert!(parse_words(&accept).is_ok());
        let mut with_project = accept.to_vec();
        with_project.extend(["--project", "/home/me/app"]);
        assert!(parse_words(&with_project).is_err());
        let mut many = vec!["projects-ssh"];
        for _ in 0..=hide_core::SWITCHED_PROJECTS {
            many.extend(["--project", "/home/me/app"]);
        }
        assert!(parse_words(&many).is_err());
    }

    /// `carry` is run by a machine and writes a connection's bytes: it
    /// takes no word, and no `--json` that would promise an answer line.
    #[test]
    fn carry_takes_nothing_and_answers_no_json() {
        assert_eq!(parse_words(&["carry"]).unwrap().verb, Verb::Carry);
        assert!(parse_words(&["carry", "Ubuntu"]).is_err());
        assert!(parse_words(&["carry", "--json"]).is_err());
        assert!(parse_words(&["carry", "--wait"]).is_err());
    }

    #[test]
    fn a_share_is_held_for_as_long_as_its_question_waits() {
        use std::sync::mpsc::{TryRecvError, channel};
        let (held, holds) = channel();
        let mut count = 0;
        let hold = move || {
            count += 1;
            let _ = held.send(count);
        };
        // The operator answers only after the share was held three times,
        // however long that takes: holding never waits for the answer.
        let answer = held_while(Duration::from_millis(1), hold, || {
            for _ in 0..3 {
                holds
                    .recv_timeout(Duration::from_secs(30))
                    .expect("held while the question waits");
            }
            "y"
        });
        assert_eq!(answer, "y");
        // Answered: the holder ended with the question and holds no more.
        while holds.try_recv().is_ok() {}
        assert_eq!(holds.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    fn rename_takes_a_machine_and_one_trimmed_name() {
        assert_eq!(
            parse_words(&["rename", "win", " 회사 PC "]),
            Ok(DeviceCli {
                verb: Verb::Rename {
                    device: "win".into(),
                    name: "회사 PC".into(),
                },
                json: false,
                wait: false,
            })
        );
        assert!(parse_words(&["rename", "win"]).is_err());
        // An empty name asks for the machine's own name back.
        assert!(matches!(
            parse_words(&["rename", "win", "  "]),
            Ok(DeviceCli { verb: Verb::Rename { name, .. }, .. }) if name.is_empty()
        ));
        assert!(parse_words(&["rename", "win", "a", "b"]).is_err());
    }

    #[test]
    fn add_takes_an_alias_with_an_optional_name() {
        assert_eq!(
            parse_words(&["add", "studio", "--name", "Studio"]),
            Ok(DeviceCli {
                verb: Verb::Core(DeviceCommand::Add {
                    ssh_alias: "studio".into(),
                    name: Some("Studio".into()),
                }),
                json: false,
                wait: false,
            })
        );
        assert!(parse_words(&["add"]).is_err());
        assert!(parse_words(&["add", "a", "b"]).is_err());
        assert!(parse_words(&["add", "a", "--name"]).is_err());
        assert!(parse_words(&["list", "--name", "x"]).is_err());
        assert!(parse_words(&["add", "studio", "--socket", "/x"]).is_err());
    }

    /// The operator's decision of 2026-11-24: a device that moved to its
    /// machine's own Hide has one row. The core still answers its device,
    /// which serves its panes, and the list draws the paired machine alone,
    /// whatever id each goes by (a distribution's own Hide names itself by
    /// another). A device the core still dials itself keeps its line with
    /// what it waits on, and that line is its machine's only one when a
    /// move paired the two and the device did not take the connection.
    #[test]
    fn the_list_draws_a_paired_machines_device_once_and_a_dialed_one_on_its_own_line() {
        let text = draw(
            &DeviceCommand::List,
            &json!({
                "devices": [
                    {"id": "wsl-ubuntu-1a2b", "label": "WSL Ubuntu", "ssh_alias": null,
                        "connection_id": "aa", "state": "connected"},
                    {"id": "build-id", "label": "build-box", "ssh_alias": "build",
                        "connection_id": null, "state": "waiting_for_you",
                        "waits_on": {"kind": "sign_in", "input": "secret"},
                        "next_action": "Answer on the device's row"},
                ],
                "peers": [
                    {"id": "distribution-own-id", "name": "WSL Ubuntu", "connection_id": "aa",
                        "connected": true, "path": "direct", "opens": "active"},
                    {"id": "build-id", "name": "build-box", "connection_id": "bb",
                        "connected": false, "path": null, "opens": "active",
                        "helper_device": "build-id"},
                ],
                "guest_panes": [],
            }),
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("build-box"))
                .count(),
            1,
            "{text}"
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("WSL Ubuntu"))
                .count(),
            1,
            "{text}"
        );
        assert!(
            lines[0].starts_with("distribution-own-id  WSL Ubuntu"),
            "{text}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("build-id  build-box  ssh build")),
            "{text}"
        );
        assert!(text.contains("waits on sign in (secret)"), "{text}");
    }

    #[test]
    fn retry_takes_one_device_id_and_waits_only_when_asked() {
        let retry = |wait| {
            Ok(DeviceCli {
                verb: Verb::Retry {
                    device: "studio".into(),
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
        for verb in ["trust", "answer", "sign-in"] {
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

    /// A paired machine's work here is listed under it by the address the
    /// operator types into it at, and work a machine left after it was
    /// taken off is listed too, under its id.
    #[test]
    fn the_list_names_each_pane_of_another_machines_work_here_under_that_machine() {
        let pane = |id: &str| json!({"id": id, "pane": "h1:p1", "since_unix_ms": 1});
        let text = draw(
            &DeviceCommand::List,
            &json!({
                "devices": [],
                "peers": [
                    {"id": "macbook", "name": "MacBook", "connection_id": "aa", "connected": true,
                        "opened_by": "active"},
                    {"id": "studio", "name": "Studio", "connection_id": "bb", "connected": false},
                ],
                "guest_panes": [
                    {"id": "macbook", "connection_id": "aa", "panes": [
                        pane("guest:macbook:pane:h1:p1"), pane("guest:macbook:pane:h1:p2")]},
                    {"id": "old-pc", "connection_id": "cc",
                        "panes": [pane("guest:old-pc:pane:h1:p1")]},
                ],
            }),
        );
        assert_eq!(
            text,
            "macbook  MacBook  connected\n  it opens this machine: active\n  its pane here: guest:macbook:pane:h1:p1\n  its pane here: guest:macbook:pane:h1:p2\nstudio  Studio  not connected\nold-pc  no longer paired\n  its pane here: guest:old-pc:pane:h1:p1\n"
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
