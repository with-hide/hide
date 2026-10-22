use std::io;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use herdr_core::workspace_control::{Action, Edge};
use hide_platform::process;

use crate::env::{self, Env};
use crate::spawn::spawn_owned;
use crate::state_file::{self, DaemonState};

#[derive(Debug, Eq, PartialEq)]
pub enum CommandKind {
    Delivery(herdr_core::delivery::Command),
    /// `hide agent spawn … --brief-file <path>`: the spawn, whose brief is
    /// read from `path` in the caller's account when it runs.
    AgentSpawnBriefFile {
        command: herdr_core::delivery::Command,
        path: String,
    },
    /// `hide mission …`: a Conductor's mission (PRD software-factory-conductor).
    Mission(crate::mission_cli::MissionCli),
    /// `hide agent catalog [--machine <id>]`.
    AgentCatalog {
        machine: Option<String>,
    },
    /// `hide ps [--all] [--here] [--json]`.
    Ps(crate::ps_cli::Options),
    Device(crate::device_cli::DeviceCli),
    Help,
    AgentSpawnHelp,
    /// `hide version [--json]`: this build's version, commit and contract.
    Version {
        json: bool,
    },
    /// `hide contract --json`: the contract of the commands tools call.
    Contract,
    Open,
    /// `hide connect`: `open` without the browser, answered as one JSON line
    /// for a host that loads the shell itself (the desktop app).
    Connect,
    Status {
        json: bool,
    },
    Stop,
    /// `hide runtime prepare`: the folder this package's runtime runs from,
    /// made first on Windows (`hide_kit::runtime`), as one JSON line for the
    /// desktop host.
    RuntimePrepare,
    Serve {
        keep_alive: bool,
    },
    Dev,
    BrowserOpen {
        target: String,
        wait: bool,
        request_id: Option<String>,
    },
    BrowserConnect {
        display_id: Option<String>,
    },
    /// `hide browser <verb> <display> ...`: read or act on a page.
    BrowserPage(crate::browser_page::Command),
    /// `hide browser help`: the agent guide for the page commands.
    BrowserHelp,
    /// `hide guide`: the agent's guide to Hide (`crate::guide`).
    Guide,
    WorkspaceBootstrap,
    /// `hide workspace memory`: a hook's Project Memory from the core, with
    /// the prompt on stdin (PRD core-host-node-move B14).
    WorkspaceMemory {
        event: String,
        runtime: String,
        session: String,
    },
    WorkspaceInfo,
    WorkspaceHookInfo,
    /// Private post-tool transport. Its bounded metadata is read from stdin:
    /// one observation per recipient, or per started agent with `--start`.
    ObservationHook {
        start: bool,
    },
    ViewList,
    ViewStatus {
        view_id: String,
    },
    WorkspaceAction {
        action: Action,
        request_id: Option<String>,
    },
    /// `hide links`: a read of the link record (PRD link-graph D-19).
    Links(herdr_core::links::query::LinksQuery),
    /// `hide memory show|search|add|forget|undo|placed|building|unbuilt|outdated|skipped|stale|release`
    /// (PRD project-memory-v2 D-03, D-11; PRD memory-stale-cards).
    Memory(herdr_core::memory_command::MemoryCommand),
}

const LINKS_USAGE: &str = "usage: hide links pr <number> | issue <number> | branch <name> | session <id> [--all-projects]";

const MEMORY_USAGE: &str = "usage: hide memory show <card> | search <words> | add <text> | forget <card> | undo <card> | placed <card> <path> | building <card> <branch> <path> | unbuilt <card> <reason> | outdated <card> <reason> | skipped <card> <reason> | stale <card> <reason> | release <card>";

const BROWSER_USAGE: &str = "usage: hide browser open <url-or-path> [--wait] [--request-id <id>] | connect [--display <id>] | help | <command> <display> ... (see hide browser help)";

/// The version this build reports: `HIDE_VERSION` when it was built (a
/// package passes the version it ships), else the crate's own.
pub const VERSION: &str = crate::VERSION;

/// This build's place in main's history (`HIDE_BUILD_ORDER`, its
/// first-parent commit count), which orders two builds of one release
/// (`build_order`); 0 for a build no package numbered.
pub const ORDER: u64 = count(env!("HIDE_BUILD_ORDER"));

/// A count `build.rs` checked is digits only.
const fn count(digits: &str) -> u64 {
    let digits = digits.as_bytes();
    let (mut value, mut index) = (0_u64, 0);
    while index < digits.len() {
        value = value * 10 + (digits[index] - b'0') as u64;
        index += 1;
    }
    value
}

/// The commit this build was made from, when it was built from a checkout.
pub const COMMIT: Option<&str> = match env!("HIDE_BUILD_COMMIT").as_bytes() {
    [] => None,
    _ => Some(env!("HIDE_BUILD_COMMIT")),
};

pub fn parse_args(args: &[String]) -> Result<CommandKind, String> {
    let mut iter = args.iter().skip(1);
    match iter.next().map(String::as_str) {
        Some("help" | "--help" | "-h") if iter.next().is_none() => Ok(CommandKind::Help),
        Some("--version") if iter.next().is_none() => Ok(CommandKind::Version { json: false }),
        Some("version") => match (iter.next().map(String::as_str), iter.next()) {
            (None, _) => Ok(CommandKind::Version { json: false }),
            (Some("--json"), None) => Ok(CommandKind::Version { json: true }),
            _ => Err("usage: hide version [--json]".to_owned()),
        },
        Some("guide") => match iter.next() {
            None => Ok(CommandKind::Guide),
            Some(_) => Err("usage: hide guide".to_owned()),
        },
        Some("contract") => match (iter.next().map(String::as_str), iter.next()) {
            (Some("--json"), None) => Ok(CommandKind::Contract),
            _ => Err("usage: hide contract --json".to_owned()),
        },
        None | Some("open") => Ok(CommandKind::Open),
        Some("connect") => Ok(CommandKind::Connect),
        Some("status") => {
            let json = iter.any(|arg| arg == "--json");
            Ok(CommandKind::Status { json })
        }
        Some("stop") => Ok(CommandKind::Stop),
        Some("runtime") => match (iter.next().map(String::as_str), iter.next()) {
            (Some("prepare"), None) => Ok(CommandKind::RuntimePrepare),
            _ => Err("usage: hide runtime prepare".to_owned()),
        },
        Some("serve") => {
            let keep_alive = iter.any(|arg| arg == "--keep-alive");
            Ok(CommandKind::Serve { keep_alive })
        }
        Some("dev") => Ok(CommandKind::Dev),
        Some("agent") if args[2..] == ["spawn", "--help"] => Ok(CommandKind::AgentSpawnHelp),
        Some("agent") if args.get(2).map(String::as_str) == Some("catalog") => {
            crate::cli_contract::admit(&args[1..])
                .map_err(|refusal| format!("{refusal}\n{}", crate::agent_cli::USAGE))?;
            parse_catalog(&args[3..])
        }
        Some("mission") if matches!(args.get(2).map(String::as_str), Some("help" | "--help")) => {
            crate::mission_cli::parse(args[2..].iter()).map(CommandKind::Mission)
        }
        Some("mission") => {
            crate::cli_contract::admit(&args[1..])
                .map_err(|refusal| format!("{refusal}\n{}", crate::mission_cli::USAGE))?;
            crate::mission_cli::parse(iter)
                .map(CommandKind::Mission)
                .map_err(|reason| format!("{reason}\n{}", crate::mission_cli::USAGE))
        }
        Some("agent") => {
            crate::cli_contract::admit(&args[1..])
                .map_err(|refusal| format!("{refusal}\n{}", crate::agent_cli::USAGE))?;
            crate::agent_cli::parse(iter).map(|parsed| match parsed.brief_file {
                Some(path) => CommandKind::AgentSpawnBriefFile {
                    command: parsed.command,
                    path,
                },
                None => CommandKind::Delivery(parsed.command),
            })
        }
        Some("ps") => {
            crate::cli_contract::admit(&args[1..])
                .map_err(|refusal| format!("{refusal}\n{}", crate::ps_cli::USAGE))?;
            crate::ps_cli::parse(iter).map(CommandKind::Ps)
        }
        Some("device") => {
            crate::cli_contract::admit(&args[1..])
                .map_err(|refusal| format!("{refusal}\n{}", crate::device_cli::USAGE))?;
            crate::device_cli::parse(iter).map(CommandKind::Device)
        }
        Some(topic @ ("request" | "inbox" | "watch")) => {
            crate::cli_contract::admit(&args[1..])
                .map_err(|refusal| format!("{refusal}\n{}", crate::delivery_cli::USAGE))?;
            crate::delivery_cli::parse(topic, iter).map(CommandKind::Delivery)
        }
        Some("browser") => parse_browser(iter),
        Some("observation") => match (iter.next().map(String::as_str), iter.next()) {
            (Some("--hook"), None) => Ok(CommandKind::ObservationHook { start: false }),
            (Some("--hook"), Some(flag)) if flag == "--start" && iter.next().is_none() => {
                Ok(CommandKind::ObservationHook { start: true })
            }
            _ => Err("usage: hide observation --hook [--start]".into()),
        },
        Some("workspace") => match iter.next().map(String::as_str) {
            Some("memory") => {
                let words: Vec<&str> = iter.map(String::as_str).collect();
                parse_workspace_memory(&words)
            }
            Some("bootstrap") if iter.next().is_none() => Ok(CommandKind::WorkspaceBootstrap),
            Some("info") => match (iter.next().map(String::as_str), iter.next()) {
                (None, None) => Ok(CommandKind::WorkspaceInfo),
                (Some("--hook"), None) => Ok(CommandKind::WorkspaceHookInfo),
                _ => Err("usage: hide workspace info [--hook]".to_owned()),
            },
            _ => Err("usage: hide workspace info".to_owned()),
        },
        Some("file") => parse_open(iter, true),
        Some("diff") => parse_open(iter, false),
        Some("view") => parse_view(iter),
        Some("links") => parse_links(iter),
        Some("memory") => parse_memory(iter),
        Some(other) => Err(format!("unknown command: {other}")),
    }
}

fn parse_catalog(args: &[String]) -> Result<CommandKind, String> {
    let args: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|arg| *arg != "--json")
        .collect();
    match args.as_slice() {
        [] => Ok(CommandKind::AgentCatalog { machine: None }),
        ["--machine", machine]
            if !machine.is_empty()
                && !machine.starts_with('-')
                && machine.len() <= 256
                && !machine.chars().any(char::is_control) =>
        {
            Ok(CommandKind::AgentCatalog {
                machine: Some((*machine).to_owned()),
            })
        }
        _ => Err("usage: hide agent catalog [--machine <device id>]".to_owned()),
    }
}

fn parse_links<'a>(iter: impl Iterator<Item = &'a String>) -> Result<CommandKind, String> {
    use herdr_core::links::query::{LinksQuery, QueryTarget};
    let mut all_projects = false;
    let mut words = Vec::new();
    for word in iter {
        if word == "--all-projects" && !all_projects {
            all_projects = true;
        } else if word.starts_with('-') {
            return Err(LINKS_USAGE.to_owned());
        } else {
            words.push(word.as_str());
        }
    }
    let number = |value: &str| {
        value
            .strip_prefix('#')
            .unwrap_or(value)
            .parse::<u64>()
            .ok()
            .filter(|number| *number > 0)
    };
    let target = match words.as_slice() {
        ["pr", value] => number(value).map(|number| QueryTarget::Pr { number }),
        ["issue", value] => number(value).map(|number| QueryTarget::Issue { number }),
        ["branch", name] => Some(QueryTarget::Branch {
            name: (*name).to_owned(),
        }),
        ["session", id] => Some(QueryTarget::Session {
            id: (*id).to_owned(),
        }),
        _ => None,
    }
    .filter(QueryTarget::valid)
    .ok_or(LINKS_USAGE)?;
    Ok(CommandKind::Links(LinksQuery {
        target,
        all_projects,
    }))
}

fn parse_memory<'a>(mut iter: impl Iterator<Item = &'a String>) -> Result<CommandKind, String> {
    use herdr_core::memory_command::MemoryCommand;
    let verb = iter.next().map(String::as_str);
    let words = iter.map(String::as_str).collect::<Vec<_>>();
    let rest = words.join(" ");
    let command = match (verb, words.as_slice()) {
        (Some("show"), [card]) => herdr_core::memory_command::parse_card(card)
            .map(|number| MemoryCommand::Show { number }),
        (Some("forget"), [card]) => herdr_core::memory_command::parse_card(card)
            .map(|number| MemoryCommand::Forget { number }),
        (Some("undo"), [card]) => herdr_core::memory_command::parse_card(card)
            .map(|number| MemoryCommand::Undo { number }),
        (Some("release"), [card]) => herdr_core::memory_command::parse_card(card)
            .map(|number| MemoryCommand::Release { number }),
        (Some("placed"), [card, path]) => {
            herdr_core::memory_command::parse_card(card).map(|number| MemoryCommand::Placed {
                number,
                path: (*path).to_owned(),
            })
        }
        (Some("building"), [card, branch, path]) => herdr_core::memory_command::parse_card(card)
            .map(|number| MemoryCommand::Building {
                number,
                branch: (*branch).to_owned(),
                path: (*path).to_owned(),
            }),
        (Some(verb @ ("outdated" | "skipped" | "unbuilt" | "stale")), [card, reason @ ..])
            if !reason.is_empty() =>
        {
            herdr_core::memory_command::parse_card(card).map(|number| {
                let reason = reason.join(" ");
                match verb {
                    "skipped" => MemoryCommand::Skipped { number, reason },
                    "unbuilt" => MemoryCommand::Unbuilt { number, reason },
                    "stale" => MemoryCommand::Stale { number, reason },
                    _ => MemoryCommand::Outdated { number, reason },
                }
            })
        }
        (Some("search"), [_, ..]) if !rest.trim().is_empty() => {
            Some(MemoryCommand::Search { words: rest })
        }
        (Some("add"), [_, ..]) if !rest.trim().is_empty() => {
            Some(MemoryCommand::Add { text: rest })
        }
        _ => None,
    };
    command
        .map(CommandKind::Memory)
        .ok_or_else(|| MEMORY_USAGE.to_owned())
}

fn parse_open<'a>(
    mut iter: impl Iterator<Item = &'a String>,
    file: bool,
) -> Result<CommandKind, String> {
    let usage = if file {
        "usage: hide file open <path> [--beside] [--request-id <id>]"
    } else {
        "usage: hide diff open <path> [--beside] [--request-id <id>]"
    };
    if iter.next().map(String::as_str) != Some("open") {
        return Err(usage.to_owned());
    }
    let path = iter
        .next()
        .filter(|path| !path.is_empty() && !path.starts_with('-'))
        .ok_or(usage)?;
    let (mut beside, mut request_id) = (false, None);
    while let Some(option) = iter.next() {
        match option.as_str() {
            "--beside" if !beside => beside = true,
            "--request-id" if request_id.is_none() => {
                request_id = Some(
                    iter.next()
                        .filter(|id| !id.is_empty() && !id.starts_with('-'))
                        .ok_or(usage)?
                        .clone(),
                );
            }
            _ => return Err(usage.to_owned()),
        }
    }
    let action = if file {
        Action::OpenFile {
            path: path.clone(),
            beside,
        }
    } else {
        Action::OpenDiff {
            path: path.clone(),
            beside,
        }
    };
    Ok(CommandKind::WorkspaceAction { action, request_id })
}

fn parse_view<'a>(mut iter: impl Iterator<Item = &'a String>) -> Result<CommandKind, String> {
    let usage = "usage: hide view list | status <view-id> | select <view-id> [--request-id <id>] | close <view-id> [--request-id <id>] | split <view-id> --area <area-id> --edge left|right|up|down [--request-id <id>] | move <view-id> --area <area-id> --index <n> [--request-id <id>]";
    let verb = iter.next().map(String::as_str).ok_or(usage)?;
    if verb == "list" {
        return if iter.next().is_none() {
            Ok(CommandKind::ViewList)
        } else {
            Err(usage.to_owned())
        };
    }
    if verb == "status" {
        return match (iter.next(), iter.next()) {
            (Some(view_id), None) if !view_id.is_empty() && !view_id.starts_with('-') => {
                Ok(CommandKind::ViewStatus {
                    view_id: view_id.clone(),
                })
            }
            _ => Err(usage.to_owned()),
        };
    }
    let view_id = iter
        .next()
        .filter(|id| !id.starts_with('-'))
        .ok_or(usage)?
        .clone();
    let mut area_id = None;
    let mut edge = None;
    let mut index = None;
    let mut request_id = None;
    while let Some(option) = iter.next() {
        let value = iter.next().ok_or(usage)?;
        if value.is_empty() || value.starts_with('-') {
            return Err(usage.to_owned());
        }
        match option.as_str() {
            "--area" if area_id.is_none() => area_id = Some(value.clone()),
            "--edge" if edge.is_none() => {
                edge = Some(match value.as_str() {
                    "left" => Edge::Left,
                    "right" => Edge::Right,
                    "up" => Edge::Up,
                    "down" => Edge::Down,
                    _ => return Err(usage.to_owned()),
                })
            }
            "--index" if index.is_none() => {
                index = Some(value.parse::<usize>().map_err(|_| usage)?)
            }
            "--request-id" if request_id.is_none() => request_id = Some(value.clone()),
            _ => return Err(usage.to_owned()),
        }
    }
    let action = match verb {
        "select" if area_id.is_none() && edge.is_none() && index.is_none() => Action::Select {
            view_id,
            expected_browser_area: None,
        },
        "close" if area_id.is_none() && edge.is_none() && index.is_none() => Action::Close {
            view_id,
            expected_browser_area: None,
        },
        "split" if index.is_none() => Action::Split {
            view_id,
            area_id: area_id.ok_or(usage)?,
            edge: edge.ok_or(usage)?,
        },
        "move" if edge.is_none() => Action::Move {
            view_id,
            area_id: area_id.ok_or(usage)?,
            index: index.ok_or(usage)?,
        },
        _ => return Err(usage.to_owned()),
    };
    Ok(CommandKind::WorkspaceAction { action, request_id })
}

fn parse_browser<'a>(mut iter: impl Iterator<Item = &'a String>) -> Result<CommandKind, String> {
    let verb = iter.next().map(String::as_str);
    if verb == Some("help") {
        return match iter.next() {
            None => Ok(CommandKind::BrowserHelp),
            Some(_) => Err(BROWSER_USAGE.to_owned()),
        };
    }
    if let Some(verb) = verb.filter(|verb| crate::browser_page::VERBS.contains(verb)) {
        return crate::browser_page::parse(verb, iter).map(CommandKind::BrowserPage);
    }
    if verb == Some("connect") {
        let display_id = match (iter.next().map(String::as_str), iter.next(), iter.next()) {
            (None, None, None) => None,
            (Some("--display"), Some(id), None) if !id.is_empty() && !id.starts_with('-') => {
                Some(id.clone())
            }
            _ => return Err(BROWSER_USAGE.to_owned()),
        };
        return Ok(CommandKind::BrowserConnect { display_id });
    }
    if verb != Some("open") {
        return Err(BROWSER_USAGE.to_owned());
    }
    let (mut target, mut wait, mut request_id) = (None, false, None);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--wait" if !wait => wait = true,
            "--request-id" if request_id.is_none() => {
                request_id = Some(
                    iter.next()
                        .filter(|id| !id.is_empty() && !id.starts_with('-'))
                        .ok_or(BROWSER_USAGE)?
                        .clone(),
                )
            }
            _ if target.is_none() && !arg.starts_with('-') => target = Some(arg.clone()),
            _ => return Err(BROWSER_USAGE.to_owned()),
        }
    }
    let target = target.ok_or_else(|| BROWSER_USAGE.to_owned())?;
    Ok(CommandKind::BrowserOpen {
        target,
        wait,
        request_id,
    })
}

pub fn run(kind: CommandKind) -> Result<(), String> {
    // Neither reads the environment nor needs a daemon.
    if let CommandKind::Version { json } = kind {
        let digest = crate::cli_contract::digest();
        if json {
            println!(
                "{}",
                serde_json::json!({"version": VERSION, "commit": COMMIT, "contract": digest})
            );
        } else {
            match COMMIT {
                Some(commit) => println!("hide {VERSION} {commit}"),
                None => println!("hide {VERSION}"),
            }
        }
        return Ok(());
    }
    if kind == CommandKind::Contract {
        println!("{}", crate::cli_contract::document());
        return Ok(());
    }

    if kind == CommandKind::AgentSpawnHelp {
        println!("{}", crate::agent_cli::SPAWN_HELP);
        return Ok(());
    }
    if kind == CommandKind::Help {
        println!("hide guide\nhide version [--json]\nhide contract --json");
        println!("{}", crate::delivery_cli::USAGE);
        println!("{}", crate::agent_cli::USAGE);
        println!("{}", crate::ps_cli::USAGE);
        println!("{}", crate::device_cli::USAGE);
        println!("{}", crate::mission_cli::USAGE);
        println!(
            "hide workspace info\nhide file open <path> [--beside] [--request-id <id>]\nhide diff open <path> [--beside] [--request-id <id>]\nhide browser open <url-or-path> [--wait] [--request-id <id>]\nhide browser connect [--display <id>]\nhide browser snapshot|click|fill|type|press|hover|drag|scroll|wait|screenshot|eval|console|network <display> ...\nhide browser help\nhide view list\nhide view status <view-id>\nhide view select <view-id> [--request-id <id>]\nhide view close <view-id> [--request-id <id>]\nhide view split <view-id> --area <area-id> --edge left|right|up|down [--request-id <id>]\nhide view move <view-id> --area <area-id> --index <n> [--request-id <id>]\nhide links pr <number> | issue <number> | branch <name> | session <id> [--all-projects]\nhide memory show <card> | search <words> | add <text> | forget <card> | undo <card> | placed <card> <path> | building <card> <branch> <path> | unbuilt <card> <reason> | outdated <card> <reason> | skipped <card> <reason> | stale <card> <reason> | release <card>\nEach Workspace command requires a live Hide renderer and a caller Hide can bind to a checkout, an attested Herdr pane or a shell inside a registered checkout; it never starts Hide. hide links needs only the caller and a running Hide, and reads the caller's Project unless --all-projects is given. hide memory reads, adds to and forgets from the caller's Project Memory, which the core keeps in one store for the Projects of its own machine and every registered linked device with a Memory reader. hide memory release ends a hold an agent's stale report put on a card; run in a Hide shell pane with no agent, it also lets that card's text be reported stale once more."
        );
        return Ok(());
    }
    // The guides need no daemon and no environment.
    if kind == CommandKind::Mission(crate::mission_cli::MissionCli::Help) {
        print!("{}", crate::mission_cli::HELP);
        return Ok(());
    }
    if kind == CommandKind::BrowserHelp {
        print!("{}", crate::browser_page::HELP);
        return Ok(());
    }
    if kind == CommandKind::Guide {
        print!("{}", crate::guide::text());
        return Ok(());
    }
    let env = match needed_keys(&kind).map_or_else(env::load, env::load_for) {
        Ok(env) => env,
        Err(_errors)
            if matches!(
                &kind,
                CommandKind::WorkspaceBootstrap
                    | CommandKind::WorkspaceMemory { .. }
                    | CommandKind::Delivery(_)
                    | CommandKind::AgentSpawnBriefFile { .. }
                    | CommandKind::Mission(_)
                    | CommandKind::AgentCatalog { .. }
                    | CommandKind::Ps(_)
                    | CommandKind::Device(_)
                    | CommandKind::WorkspaceInfo
                    | CommandKind::WorkspaceHookInfo
                    | CommandKind::ObservationHook { .. }
                    | CommandKind::ViewList
                    | CommandKind::ViewStatus { .. }
                    | CommandKind::BrowserOpen { .. }
                    | CommandKind::BrowserConnect { .. }
                    | CommandKind::BrowserPage(_)
                    | CommandKind::WorkspaceAction { .. }
                    | CommandKind::Links(_)
                    | CommandKind::Memory(_)
            ) =>
        {
            return workspace_refusal(
                "environment_invalid",
                "Check Hide environment settings and retry",
            );
        }
        Err(errors) => {
            return Err(errors
                .iter()
                .map(|error| format!("{}: {}", error.key, error.kind))
                .collect::<Vec<_>>()
                .join("\n"));
        }
    };
    match kind {
        CommandKind::Mission(command) => crate::mission_cli::run(&env, command),
        CommandKind::AgentCatalog { machine } => crate::mission_cli::run_catalog(&env, machine),
        CommandKind::Ps(options) => crate::ps_cli::run(&env, &options),
        CommandKind::Device(cli) => crate::device_cli::run(&env, &cli),
        CommandKind::Delivery(command) => {
            if matches!(&command, herdr_core::delivery::Command::Agents { .. }) {
                crate::agent_cli::run(&env, command)
            } else {
                crate::delivery_cli::run(&env, command)
            }
        }
        CommandKind::AgentSpawnBriefFile { command, path } => {
            let brief = crate::agent_cli::read_brief_file(&path)?;
            crate::agent_cli::run(&env, crate::agent_cli::with_brief(command, brief))
        }
        CommandKind::Help
        | CommandKind::AgentSpawnHelp
        | CommandKind::BrowserHelp
        | CommandKind::Guide
        | CommandKind::Version { .. }
        | CommandKind::Contract => unreachable!("handled above"),
        CommandKind::Open => open(&env),
        CommandKind::Connect => connect_json(&env),
        CommandKind::Status { json: true } => status_json(&env.state_dir),
        CommandKind::Status { json: false } => status(&env.state_dir, env.idle_secs),
        CommandKind::Stop => stop(&env.state_dir),
        CommandKind::RuntimePrepare => runtime_prepare(&env.home),
        CommandKind::Serve { keep_alive } => serve(env, keep_alive),
        CommandKind::Dev => dev(env),
        CommandKind::BrowserOpen {
            target,
            wait,
            request_id,
        } => browser_open(&env, &target, wait, request_id.as_deref()),
        CommandKind::BrowserConnect { display_id } => {
            let answer = browser_connect_value(&env, display_id.as_deref(), true)?;
            println!("{answer}");
            Ok(())
        }
        CommandKind::BrowserPage(command) => crate::browser_page::run(&env, command),
        CommandKind::WorkspaceBootstrap => {
            let reference = crate::workspace_cli::bootstrap(&env, false)?;
            println!("{}", serde_json::json!({"ok":true,"reference":reference}));
            Ok(())
        }
        CommandKind::WorkspaceInfo => workspace_query(&env, "info"),
        CommandKind::WorkspaceHookInfo => {
            let answer = crate::workspace_cli::request_hook(
                &env,
                hide_node_link::panes::HookCommand::WorkspaceInfo {},
            )?;
            println!("{answer}");
            Ok(())
        }
        CommandKind::ObservationHook { start } => {
            use hide_node_link::panes::{
                HookCommand, OBSERVATION_ITEM_LIMIT, OBSERVATION_RECIPIENT_LIMIT,
            };
            use std::io::Read;
            // One item per recipient, each already bounded; the array adds
            // its brackets and commas.
            let limit = (OBSERVATION_ITEM_LIMIT + 1) * OBSERVATION_RECIPIENT_LIMIT + 2;
            let mut bytes = Vec::new();
            std::io::stdin()
                .lock()
                .take((limit + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| "observation_input_unavailable")?;
            if bytes.len() > limit {
                return Err("observation_input_too_large".into());
            }
            let observations: Vec<hide_node_link::panes::DirectObservation> =
                serde_json::from_slice(&bytes).map_err(|_| "observation_input_invalid")?;
            if observations.is_empty() || observations.len() > OBSERVATION_RECIPIENT_LIMIT {
                return Err("observation_input_invalid".into());
            }
            // Each recipient is proven and recorded on its own; one refusal
            // does not withhold the others.
            let mut refusal = None;
            for observation in observations {
                let command = if start {
                    HookCommand::StartObservation { observation }
                } else {
                    HookCommand::Observation { observation }
                };
                let answer = crate::workspace_cli::request_hook(&env, command)?;
                if answer["ok"] != true && refusal.is_none() {
                    refusal = Some(answer);
                }
            }
            let answer = refusal.unwrap_or_else(|| {
                serde_json::json!({"type":"workspace_result","ok":true,"result":{"queued":true}})
            });
            println!("{answer}");
            if answer["ok"] == true {
                Ok(())
            } else {
                Err(answer["reason"]
                    .as_str()
                    .unwrap_or("observation_unavailable")
                    .into())
            }
        }
        CommandKind::WorkspaceMemory {
            event,
            runtime,
            session,
        } => {
            const NEXT: &str = "Continue without Project Memory";
            let prompt = crate::workspace_cli::read_memory_prompt(std::io::stdin().lock())
                .or_else(|reason| workspace_refusal(&reason, NEXT))?;
            let answer = crate::workspace_cli::request_memory_hook(
                &env,
                json_memory_hook_ask(&event, &runtime, &session, &prompt),
            )
            .or_else(|reason| workspace_refusal(&reason, NEXT))?;
            println!("{answer}");
            if answer["ok"] == true {
                Ok(())
            } else {
                Err(answer["reason"]
                    .as_str()
                    .unwrap_or("memory_unavailable")
                    .to_owned())
            }
        }
        CommandKind::ViewList => workspace_query(&env, "view_list"),
        CommandKind::ViewStatus { view_id } => view_status(&env, &view_id),
        CommandKind::WorkspaceAction { action, request_id } => {
            workspace_action(&env, action, request_id.as_deref())
        }
        CommandKind::Links(query) => links(&env, &query),
        CommandKind::Memory(command) => memory(&env, &command),
    }
}

/// Prints a `hide memory` answer as text an agent reads; a refusal prints
/// its reason and next action as one JSON line and exits non-zero (B8, B16).
fn memory(env: &Env, command: &herdr_core::memory_command::MemoryCommand) -> Result<(), String> {
    use herdr_core::memory_command::{MemoryCommand, next_action};
    if !command.valid() {
        let reason = match command {
            MemoryCommand::Add { .. } => "memory_text_too_long",
            MemoryCommand::Placed { .. } => "memory_path_not_found",
            MemoryCommand::Building { branch, .. }
                if !herdr_core::memory_command::branch_name(branch) =>
            {
                "memory_branch_not_found"
            }
            MemoryCommand::Building { .. } => "memory_building_path_not_found",
            MemoryCommand::Outdated { .. }
            | MemoryCommand::Skipped { .. }
            | MemoryCommand::Unbuilt { .. } => "memory_reason_invalid",
            MemoryCommand::Stale { .. } => "memory_stale_reason_invalid",
            _ => "invalid_request",
        };
        return workspace_refusal(reason, next_action(reason));
    }
    let mut credential = match crate::workspace_cli::Credential::acquire(env) {
        Ok(credential) => credential,
        Err(reason) => return query_refusal(&reason, bootstrap_next_action(&reason), true),
    };
    let answer = match crate::workspace_cli::request_memory(&mut credential, command) {
        Ok(answer) => answer,
        Err(reason) => return query_refusal(&reason, request_next_action(&reason), true),
    };
    if answer["ok"] != true {
        println!("{answer}");
        return Err(answer["reason"]
            .as_str()
            .unwrap_or("memory_request_failed")
            .to_owned());
    }
    print!("{}", memory_text(command, &answer["result"]));
    Ok(())
}

fn memory_text(
    command: &herdr_core::memory_command::MemoryCommand,
    result: &serde_json::Value,
) -> String {
    use herdr_core::memory_command::MemoryCommand;
    let text = |value: &serde_json::Value| value.as_str().unwrap_or_default().to_owned();
    match command {
        MemoryCommand::Show { .. } => {
            let mut lines = vec![
                format!(
                    "{} · {} · {} · {} sources, {} from the operator",
                    text(&result["card"]),
                    text(&result["kind"]),
                    text(&result["state"]).replace('_', " "),
                    result["sources"],
                    result["operator_sources"],
                ),
                text(&result["title"]),
                String::new(),
                text(&result["body"]),
            ];
            let cited = result["cited_paths"]
                .as_array()
                .map(|paths| paths.iter().map(text).collect::<Vec<_>>())
                .unwrap_or_default();
            if !cited.is_empty() {
                lines.push(format!("Files: {}", cited.join(", ")));
            }
            let evidence = result["evidence"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .map(|row| {
                            let value = text(&row["value"]);
                            let mark = match (row["present"].as_bool(), row["fingerprint"].as_str())
                            {
                                (Some(false), _) => " (not found)".to_owned(),
                                (_, Some(fingerprint)) => format!(
                                    " ({})",
                                    fingerprint.chars().take(15).collect::<String>()
                                ),
                                _ => String::new(),
                            };
                            format!("{} {value}{mark}", text(&row["kind"]))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if !evidence.is_empty() {
                lines.push(format!("Rests on: {}", evidence.join(", ")));
            }
            let hold = &result["hold"];
            if hold.is_object() {
                lines.push(format!(
                    "Held: an agent reported it stale at {}, so Hide does not give it until that conversation is analyzed or {}. Release: hide memory release {}",
                    text(&hold["reported_at"]),
                    text(&hold["expires_at"]),
                    text(&result["card"]),
                ));
                lines.push(format!(
                    "Reported because (the reporting agent's words, not checked): \"{}\"",
                    text(&hold["reason"])
                ));
            }
            if result["verifying"] == true {
                lines.push(
                    "Not verified: a file or name this card rests on changed, so check it against the repository before relying on it."
                        .to_owned(),
                );
            }
            let comparison = &result["document_comparison"];
            if comparison.is_object() {
                let document = text(&comparison["document"]);
                lines.push(match comparison["authority"].as_str() {
                    Some("card_newer") => {
                        format!("Document comparison: the operator's words are newer than {document}.")
                    }
                    Some("outdated") => format!(
                        "Document comparison: {document} is newer than the operator's words, so this card is outdated."
                    ),
                    _ => format!(
                        "Document comparison: {document} disagrees with the operator's words, and which is newer is not known."
                    ),
                });
                lines.push(format!(
                    "Document says: {}",
                    text(&comparison["document_text"])
                ));
                let session = comparison["source_session"]
                    .as_str()
                    .map(|session| format!(" (session {session})"))
                    .unwrap_or_default();
                lines.push(format!(
                    "Operator said{session}: {}",
                    text(&comparison["operator_text"])
                ));
            }
            lines.join("\n") + "\n"
        }
        MemoryCommand::Forget { .. } => {
            let card = text(&result["card"]);
            format!("Forgot {card}. Undo: hide memory undo {card}\n")
        }
        MemoryCommand::Undo { .. } => {
            let card = text(&result["card"]);
            match result["outcome"].as_str() {
                Some("removed") => format!("Removed {card}.\n"),
                _ => format!("Restored {card}.\n"),
            }
        }
        MemoryCommand::Stale { .. } => {
            let card = text(&result["card"]);
            format!(
                "Held {card}: Hide stops giving it until this conversation is analyzed, at the latest {}. Release: hide memory release {card}\n",
                text(&result["expires_at"]),
            )
        }
        MemoryCommand::Release { .. } => {
            let card = text(&result["card"]);
            match (result["report"].as_str(), result["caller"].as_str()) {
                (Some("given_back"), _) => format!(
                    "Released {card}; Hide gives it again, and it may be reported stale once more.\n"
                ),
                (Some("spent"), Some("checkout")) => format!(
                    "Released {card}; Hide gives it again. Hide cannot tell a terminal outside its panes from an agent, so this text is not reported stale again; a release run in a Hide shell pane with no agent gives the report back.\n"
                ),
                (Some("spent"), _) => format!(
                    "Released {card}; Hide gives it again. An agent released it, so this text is not reported stale again.\n"
                ),
                _ => format!("Released {card}; Hide gives it again.\n"),
            }
        }
        MemoryCommand::Placed { .. } => format!(
            "{} points at {} once its pull request merges.\n",
            text(&result["card"]),
            text(&result["path"])
        ),
        MemoryCommand::Building { .. } => format!(
            "{} goes into {} with the pull request of {}; this attempt ends without one.\n",
            text(&result["card"]),
            text(&result["path"]),
            text(&result["branch"])
        ),
        MemoryCommand::Unbuilt { .. } => format!(
            "{} returns to To ask as not built yet, with its reason; this attempt ends without a pull request.\n",
            text(&result["card"])
        ),
        MemoryCommand::Outdated { .. } => format!(
            "{} is marked outdated; Hide checks it again and this attempt ends without a pull request.\n",
            text(&result["card"])
        ),
        MemoryCommand::Skipped { .. } => format!(
            "{} returns to To ask with its reason. Other cards stay in the shared pull request; retry continues the same branch.\n",
            text(&result["card"])
        ),
        MemoryCommand::Search { .. } => {
            let cards = result["cards"].as_array().cloned().unwrap_or_default();
            if cards.is_empty() {
                return "No card matches.\n".to_owned();
            }
            cards
                .iter()
                .map(|card| format!("{}  {}\n", text(&card["card"]), text(&card["title"])))
                .collect()
        }
        MemoryCommand::Add { .. } => {
            let outcomes = result["outcomes"].as_array().cloned().unwrap_or_default();
            if outcomes.is_empty() {
                return "Nothing kept: the text holds nothing a card would.\n".to_owned();
            }
            outcomes
                .iter()
                .map(|outcome| {
                    let card = text(&outcome["card"]);
                    match outcome["outcome"].as_str() {
                        Some("added") => format!("Added {card}.\n"),
                        Some("already_added") => format!("Already added as {card}.\n"),
                        Some("same_as") => format!("{card} already says this; added it as a source.\n"),
                        Some("replaced") => format!("Replaced {card}.\n"),
                        Some("outranked") => format!(
                            "Not given: {card} rests on the operator's own words and stays in use; this text is kept in its history.\n"
                        ),
                        Some("secret") => "Not kept: it looked like a secret.\n".to_owned(),
                        Some("full") => "Not kept: this Project's Memory is full.\n".to_owned(),
                        _ => "Nothing kept: the text holds nothing a card would.\n".to_owned(),
                    }
                })
                .collect()
        }
    }
}

const WORKSPACE_MEMORY_USAGE: &str = "usage: hide workspace memory --event <SessionStart|UserPromptSubmit> --runtime <runtime> --session <id> < prompt";

fn parse_workspace_memory(words: &[&str]) -> Result<CommandKind, String> {
    let ["--event", event, "--runtime", runtime, "--session", session] = words else {
        return Err(WORKSPACE_MEMORY_USAGE.to_owned());
    };
    if !matches!(*event, "SessionStart" | "UserPromptSubmit")
        || runtime.is_empty()
        || runtime.chars().any(char::is_control)
        || !herdr_core::delivery::valid_session(session)
    {
        return Err(WORKSPACE_MEMORY_USAGE.to_owned());
    }
    Ok(CommandKind::WorkspaceMemory {
        event: (*event).to_owned(),
        runtime: (*runtime).to_owned(),
        session: (*session).to_owned(),
    })
}

/// The `memory_hook` request's own fields; the prompt goes only when there
/// is one.
fn json_memory_hook_ask(
    event: &str,
    runtime: &str,
    session: &str,
    prompt: &str,
) -> serde_json::Value {
    let mut ask = serde_json::json!({"event": event, "runtime": runtime, "session": session});
    if !prompt.is_empty() {
        ask["prompt"] = serde_json::json!(prompt);
    }
    ask
}

/// Prints the record's answer as one line; a refusal prints its reason and
/// next action and exits non-zero (B42, B43).
fn links(env: &Env, query: &herdr_core::links::query::LinksQuery) -> Result<(), String> {
    let mut credential = match crate::workspace_cli::Credential::acquire(env) {
        Ok(credential) => credential,
        Err(reason) => return query_refusal(&reason, bootstrap_next_action(&reason), true),
    };
    let answer = match crate::workspace_cli::request_links(&mut credential, query) {
        Ok(answer) => answer,
        Err(reason) => {
            return query_refusal(&reason, request_next_action(&reason), true);
        }
    };
    println!("{answer}");
    if answer["ok"] == true {
        Ok(())
    } else {
        Err(answer["reason"]
            .as_str()
            .unwrap_or("links_request_failed")
            .to_owned())
    }
}

fn workspace_action(env: &Env, action: Action, request_id: Option<&str>) -> Result<(), String> {
    let answer = workspace_action_value(env, action, request_id)?;
    println!("{answer}");
    Ok(())
}

fn workspace_action_value(
    env: &Env,
    action: Action,
    request_id: Option<&str>,
) -> Result<serde_json::Value, String> {
    let action = match action {
        Action::OpenFile { path, beside } => Action::OpenFile {
            path: match absolute_caller_path(&path) {
                Ok(path) => path,
                Err(reason) => {
                    return workspace_refusal(&reason, "Check the current directory and retry");
                }
            },
            beside,
        },
        Action::OpenDiff { path, beside } => Action::OpenDiff {
            path: match absolute_caller_path(&path) {
                Ok(path) => path,
                Err(reason) => {
                    return workspace_refusal(&reason, "Check the current directory and retry");
                }
            },
            beside,
        },
        other => other,
    };
    let request_id = match request_id {
        Some(id) if crate::workspace_cli::valid_request_id(id) => id.to_owned(),
        Some(_) => {
            return workspace_refusal(
                "invalid_request_id",
                "Use the request ID printed by an earlier hide command",
            );
        }
        None => match crate::workspace_cli::fresh_request_id() {
            Ok(id) => id,
            Err(reason) => return workspace_refusal(&reason, "Check the local runtime and retry"),
        },
    };
    let mut credential = match crate::workspace_cli::Credential::acquire(env) {
        Ok(credential) => credential,
        Err(reason) => return workspace_action_before_send_refusal(&request_id, &reason),
    };
    let answer = match crate::workspace_cli::request_action(&mut credential, action, &request_id) {
        Ok(answer) => answer,
        Err(reason) if bootstrap_refused(&reason) => {
            return workspace_action_before_send_refusal(&request_id, &reason);
        }
        Err(reason) => return workspace_action_refusal(&request_id, &reason),
    };
    if answer["ok"] == true {
        Ok(answer)
    } else {
        println!("{answer}");
        Err(answer["reason"]
            .as_str()
            .unwrap_or("workspace_action_failed")
            .to_owned())
    }
}

/// What the caller can do about a refused credential bootstrap. The daemon
/// answers the bootstrap with a reason only, so the CLI names the next step.
pub(crate) fn bootstrap_next_action(reason: &str) -> &'static str {
    match reason {
        "checkout_not_registered" => crate::pane_auth::CHECKOUT_NEXT_ACTION,
        "caller_unavailable" => "Retry from a live shell inside a registered project checkout",
        _ => "Open Hide, reconnect this pane, and retry the same command",
    }
}

/// A bootstrap refusal reaches a request when the reference `HIDE_CAP_REF`
/// named was gone and the bare bootstrap that replaces it was refused;
/// nothing was sent, and the bootstrap's next step still applies.
fn bootstrap_refused(reason: &str) -> bool {
    matches!(reason, "checkout_not_registered" | "caller_unavailable")
}

fn request_next_action(reason: &str) -> &'static str {
    if bootstrap_refused(reason) {
        bootstrap_next_action(reason)
    } else {
        "Check Hide status, reconnect the pane, and retry"
    }
}

fn workspace_action_before_send_refusal<T>(request_id: &str, reason: &str) -> Result<T, String> {
    println!(
        "{}",
        serde_json::json!({
            "ok": false,
            "request_id": request_id,
            "reason": reason,
            "applied": "not_applied",
            "next_action": bootstrap_next_action(reason),
        })
    );
    Err(reason.to_owned())
}

fn absolute_caller_path(path: &str) -> Result<String, String> {
    use std::path::{Component, PathBuf};
    let cwd = std::env::current_dir().map_err(|_| "cwd_unavailable".to_owned())?;
    let input = PathBuf::from(path);
    let mut normalized = PathBuf::new();
    for component in cwd.join(input).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    // A pane's cwd may be reached through a platform alias such as macOS
    // /var -> /private/var, while Herdr reports the checkout's real path.
    // Resolve the nearest existing ancestor so deleted diff paths still
    // retain their final components and the host can judge them in its root.
    let mut ancestor = normalized.as_path();
    let mut missing = Vec::new();
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| "path_unavailable".to_owned())?;
        missing.push(name.to_os_string());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| "path_unavailable".to_owned())?;
    }
    let mut resolved = hide_platform::fs::identity::canonical(ancestor)
        .map_err(|_| "path_unavailable".to_owned())?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    hide_platform::path::to_wire(&resolved).map_err(|_| "path_unavailable".to_owned())
}

fn workspace_action_refusal<T>(request_id: &str, reason: &str) -> Result<T, String> {
    let applied = if matches!(
        reason,
        "request_timeout" | "hide_unavailable" | "invalid_response"
    ) {
        "unknown"
    } else {
        "not_applied"
    };
    println!(
        "{}",
        serde_json::json!({
            "ok": false,
            "request_id": request_id,
            "reason": reason,
            "applied": applied,
            "next_action": format!("Reconnect Hide, inspect hide view list, or retry the same command with --request-id {request_id}"),
        })
    );
    Err(reason.to_owned())
}

fn workspace_query(env: &Env, query: &str) -> Result<(), String> {
    let answer = workspace_query_value(env, query, true)?;
    println!("{answer}");
    Ok(())
}

fn workspace_query_value(
    env: &Env,
    query: &str,
    report: bool,
) -> Result<serde_json::Value, String> {
    workspace_query_options(env, query, None, report)
}

fn browser_connect_value(
    env: &Env,
    display_id: Option<&str>,
    report: bool,
) -> Result<serde_json::Value, String> {
    workspace_query_options(env, "browser_connect", display_id, report)
}

fn workspace_query_options(
    env: &Env,
    query: &str,
    display_id: Option<&str>,
    report: bool,
) -> Result<serde_json::Value, String> {
    let mut credential = match crate::workspace_cli::Credential::acquire(env) {
        Ok(credential) => credential,
        Err(reason) => {
            return query_refusal(&reason, bootstrap_next_action(&reason), report);
        }
    };
    let requested = if query == "browser_connect" {
        crate::workspace_cli::browser_connect(&mut credential, display_id)
    } else {
        crate::workspace_cli::request(&mut credential, query)
    };
    let answer = match requested {
        Ok(answer) => answer,
        Err(reason) => {
            return query_refusal(&reason, request_next_action(&reason), report);
        }
    };
    if answer["ok"] == true {
        Ok(answer)
    } else {
        if report {
            println!("{answer}");
        }
        Err(answer["reason"]
            .as_str()
            .unwrap_or("workspace_request_failed")
            .to_owned())
    }
}

fn query_refusal<T>(reason: &str, next_action: &str, report: bool) -> Result<T, String> {
    if report {
        workspace_refusal(reason, next_action)
    } else {
        Err(reason.to_owned())
    }
}

fn workspace_refusal<T>(reason: &str, next_action: &str) -> Result<T, String> {
    println!(
        "{}",
        serde_json::json!({"ok":false,"reason":reason,"next_action":next_action})
    );
    Err(reason.to_owned())
}

fn view_status_value(env: &Env, view_id: &str, report: bool) -> Result<serde_json::Value, String> {
    let answer = workspace_query_value(env, "view_list", report)?;
    let view = answer["result"]["views"]
        .as_array()
        .and_then(|views| views.iter().find(|view| view["view_id"] == view_id));
    match view {
        Some(view) => {
            Ok(serde_json::json!({"ok":true,"context":answer["result"]["context"],"view":view}))
        }
        None => {
            query_refusal(
                "view_missing",
                "Run hide view list and choose a current View",
                report,
            )?;
            unreachable!()
        }
    }
}

fn view_status(env: &Env, view_id: &str) -> Result<(), String> {
    let answer = view_status_value(env, view_id, true)?;
    println!("{answer}");
    Ok(())
}

#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn browser_open(
    env: &Env,
    target: &str,
    wait: bool,
    request_id: Option<&str>,
) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|_| "cwd_unavailable".to_owned())?;
    let url = match crate::browser_cli::address(target, &cwd) {
        Ok(url) => url,
        Err(_) => {
            return workspace_refusal(
                "invalid_address",
                "Use an http, https, or readable checkout HTML address",
            );
        }
    };
    let mut answer = workspace_action_value(
        env,
        Action::OpenBrowser {
            url,
            area_id: None,
            new_target: false,
        },
        request_id,
    )?;
    let display_id = answer["result"]["view_id"]
        .as_str()
        .ok_or("invalid_response")?
        .to_owned();
    match browser_connect_value(env, Some(&display_id), false) {
        Ok(connection) => {
            for key in ["cdp_http_url", "browser_ws_url"] {
                answer["result"][key] = connection["result"][key].clone();
            }
        }
        Err(reason) => {
            // Opening already applied. Endpoint discovery cannot recast it as
            // a failed action and tempt a caller to create the page again.
            let next_action = if reason == "browser_control_elsewhere" {
                // The only window showing it is on another machine: its
                // endpoints are that machine's loopback, its page commands
                // reach it through Hide.
                format!("Run hide browser snapshot {display_id} and the other page commands")
            } else {
                format!(
                    "Run hide browser connect --display {display_id} after the desktop window reconnects"
                )
            };
            answer["result"]["browser_control"] = serde_json::json!({
                "state":"unavailable","reason":reason,"next_action":next_action
            });
        }
    }
    if !wait {
        println!("{answer}");
        return Ok(());
    }
    let view_id = answer["result"]["view_id"]
        .as_str()
        .ok_or("invalid_response")?;
    let requested_load = answer["result"]["load"]
        .as_u64()
        .ok_or("invalid_response")?;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let status = match view_status_value(env, view_id, false) {
            Ok(status) => status,
            Err(reason) => {
                println!(
                    "{}",
                    serde_json::json!({"ok":false,"request_id":answer["request_id"],"applied":"applied","reason":"page_status_unavailable","detail":reason,"view_id":view_id,"result":answer["result"],"next_action":format!("Run hide view status {view_id} or retry the same request ID")})
                );
                return Err("page_status_unavailable".to_owned());
            }
        };
        let page = &status["view"]["page"];
        if page["load"].as_u64() != Some(requested_load) {
            println!(
                "{}",
                serde_json::json!({"ok":false,"request_id":answer["request_id"],"applied":"applied","reason":"page_superseded","view_id":view_id,"page":page,"next_action":"Run hide view status or open the page again with a new request ID"})
            );
            return Err("page_superseded".to_owned());
        }
        match page["state"].as_str() {
            Some("loaded") => {
                println!(
                    "{}",
                    serde_json::json!({"ok":true,"request_id":answer["request_id"],"result":answer["result"],"page":page})
                );
                return Ok(());
            }
            Some("failed") => {
                println!(
                    "{}",
                    serde_json::json!({"ok":false,"request_id":answer["request_id"],"applied":"applied","reason":"page_failed","view_id":view_id,"page":page,"next_action":"Inspect the page failure and retry with a new request ID"})
                );
                return Err("page_failed".to_owned());
            }
            Some("disconnected" | "unsupported") => {
                println!(
                    "{}",
                    serde_json::json!({"ok":false,"request_id":answer["request_id"],"applied":"applied","reason":"page_unavailable","view_id":view_id,"page":page,"next_action":"Reconnect the Hide desktop window and run hide view status again"})
                );
                return Err("page_unavailable".to_owned());
            }
            Some("pending" | "loading") => {}
            _ => {
                println!(
                    "{}",
                    serde_json::json!({"ok":false,"request_id":answer["request_id"],"applied":"applied","reason":"page_status_unavailable","view_id":view_id,"result":answer["result"],"page":page,"next_action":format!("Run hide view status {view_id} or retry the same request ID")})
                );
                return Err("page_status_unavailable".to_owned());
            }
        }
        if std::time::Instant::now() >= deadline {
            println!(
                "{}",
                serde_json::json!({"ok":false,"request_id":answer["request_id"],"applied":"applied","reason":"page_wait_timeout","view_id":view_id,"page":page,"next_action":format!("Run hide view status {view_id} or retry the same request ID")})
            );
            return Err("page_wait_timeout".to_owned());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Why no daemon could be reached or started, in the categories a host shows.
#[derive(Debug, Eq, PartialEq)]
pub enum ConnectError {
    /// The daemon process could not start.
    StartFailed(String),
    /// A daemon was started but never answered its health check.
    NoResponse(String),
    /// A daemon of another build runs here and this `hide` is not the app's,
    /// so it neither replaces nor attaches to it.
    OtherBuild(String),
    /// The daemon stopped because a stored file could not be made this
    /// machine's (PRD core-host-node B2); the host names the file.
    StateRefused { file: String, detail: String },
    /// The core runs a newer build than this app's (PRD core-host-node-move
    /// B11): the core is left as it is, and the host shows both builds and
    /// asks for this app to be updated.
    CoreNewer(CoreBuilds),
    /// This connection's update of the core to this app's build failed and
    /// the core runs its previous build (B10): the host shows both builds,
    /// Retry (the next connection's one attempt) and updating on the core's
    /// machine.
    UpdateFailed(CoreBuilds),
}

/// The core's machine and the two builds `hide connect` names to the host,
/// as the node's `/health` gives them (`builds`).
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq)]
pub struct CoreBuilds {
    /// The core machine's name.
    pub machine: String,
    /// The core's build and this app's, as `build_order::Release::shown`.
    pub core: String,
    pub app: String,
}

impl ConnectError {
    fn reason(&self) -> &'static str {
        match self {
            ConnectError::StartFailed(_) => "start_failed",
            ConnectError::NoResponse(_) => "no_response",
            ConnectError::OtherBuild(_) => "other_build",
            ConnectError::StateRefused { .. } => "state_refused",
            ConnectError::CoreNewer(_) => "core_newer",
            ConnectError::UpdateFailed(_) => "update_failed",
        }
    }

    fn detail(&self) -> String {
        match self {
            ConnectError::StartFailed(detail)
            | ConnectError::NoResponse(detail)
            | ConnectError::OtherBuild(detail)
            | ConnectError::StateRefused { detail, .. } => detail.clone(),
            ConnectError::CoreNewer(newer) => format!(
                "the core on {} runs {}, newer than this app's {}",
                newer.machine, newer.core, newer.app
            ),
            ConnectError::UpdateFailed(failed) => format!(
                "the core on {} could not be updated to {} and runs {}",
                failed.machine, failed.app, failed.core
            ),
        }
    }
}

/// The one discovery path: the live daemon the state file names, or a new
/// one started and waited for. `open` and `connect` both run it, so a host
/// that loads the shell itself can never start a second daemon.
///
/// A live daemon of another build than the `hided` beside this CLI is
/// stopped and replaced (PRD labels-in-hided D-19) when this `hide` is the
/// app's own, the one in an app bundle's `Contents/Resources`: the app and
/// its daemon are always one build. Any other `hide` (a dev build, a copy)
/// refuses with the mismatch named and attaches to nothing, because the
/// installed app's daemon is the operator's. A daemon that will not stop
/// is a start failure. Only the daemon of this state folder is looked at,
/// so a dev or e2e daemon elsewhere is never replaced, and stopping it
/// leaves Herdr, its panes and their agents running.
///
/// The app's own `hide` first moves a Herdr server another Hide app
/// started onto this app's Herdr, its panes kept
/// ([`crate::herdr_handoff`]), so the daemon follows a server of the same
/// app.
///
/// The whole look-and-replace holds this state folder's connect lock, so
/// two connects never both stop a daemon and start their own.
///
/// A daemon in the node role is then waited for as [`await_core_link`]
/// says.
fn connect(env: &Env) -> Result<Healthy, ConnectError> {
    let healthy = find_or_start(env)?;
    await_core_link(&healthy.0)?;
    Ok(healthy)
}

/// A daemon and the `/health` it answered when it was found or started: its
/// role and node are read from that answer, never asked for again.
type Healthy = (DaemonState, serde_json::Value);

/// How long `hide connect` waits for a node-role daemon's first word from
/// its core.
const CORE_LINK_WITHIN: Duration = Duration::from_secs(20);

/// A daemon in the node role (PRD core-host-node-remote-core D-08, D-23)
/// attaches the host once its first attempt to link to its core ended: a
/// live link, a core it is updating to this build (PRD core-host-node-move
/// B10), a core that is not running or cannot be reached, which the
/// window shows as its reconnecting state, or a link the operator ended
/// from this machine's window, which the window offers to restore (B16).
/// This connection first gives a core update that failed on the last one
/// its attempt (B20). A core of
/// another build refuses the node, and the host shows the failure it shows
/// for a local daemon of another build; a newer core or a failed update is
/// named with both builds. Past [`CORE_LINK_WITHIN`] the host attaches and
/// the window shows the same reconnecting state. A daemon that is its own
/// core answers at once.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn await_core_link(state: &DaemonState) -> Result<(), ConnectError> {
    let deadline = std::time::Instant::now() + CORE_LINK_WITHIN;
    let mut connected = false;
    loop {
        let Ok(health) = health_json(state.port, HEALTH_REQUEST) else {
            return Ok(());
        };
        if health["role"] != "node" {
            return Ok(());
        }
        if !connected {
            connected = true;
            connect_again(state).map_err(ConnectError::NoResponse)?;
            continue;
        }
        if health["core_link"] == "connecting" && std::time::Instant::now() < deadline {
            std::thread::sleep(HEALTH_PAUSE);
            continue;
        }
        let builds = || {
            serde_json::from_value::<CoreBuilds>(health["builds"].clone()).map_err(|error| {
                ConnectError::StartFailed(format!("the node's health names no builds: {error}"))
            })
        };
        let named = |kind: &str, builds: &CoreBuilds| {
            eprintln!(
                "{}",
                serde_json::json!({
                    "component": "hide", "kind": kind, "pid": state.pid,
                    "core": builds.core, "app": builds.app,
                })
            );
        };
        if health["core_link_reason"] == "core_newer" {
            let newer = builds()?;
            named("core.newer_refused", &newer);
            return Err(ConnectError::CoreNewer(newer));
        }
        if health["core_link_reason"] == "update_failed" {
            let failed = builds()?;
            named("core.update_failed", &failed);
            return Err(ConnectError::UpdateFailed(failed));
        }
        if health["core_link_reason"] == "other_build" {
            eprintln!(
                "{}",
                serde_json::json!({
                    "component": "hide", "kind": "core.other_build_refused",
                    "pid": state.pid, "build": health["build"],
                })
            );
            return Err(ConnectError::OtherBuild(format!(
                "the core this machine's hided links to runs another build than build {}, and refused it",
                health["build"].as_str().unwrap_or("unknown"),
            )));
        }
        return Ok(());
    }
}

/// Tells the node daemon a window connects, which gives a core update
/// that failed on the last connection this one's attempt.
fn connect_again(state: &DaemonState) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(HEALTH_REQUEST))
        .proxy(None)
        .build()
        .into();
    agent
        .post(&format!(
            "http://127.0.0.1:{}/core-link/connect",
            state.port
        ))
        .header("Authorization", &format!("Bearer {}", state.token))
        .send_empty()
        .map(|_| ())
        .map_err(|error| format!("the node did not take this connection: {error}"))
}

/// The daemon of this state folder, replaced or started as [`connect`]
/// says.
fn find_or_start(env: &Env) -> Result<Healthy, ConnectError> {
    let build = crate::build_id::of_file(&daemon_binary().map_err(ConnectError::StartFailed)?)
        .map_err(ConnectError::StartFailed)?;
    #[cfg(unix)]
    move_legacy_state(env).map_err(ConnectError::StartFailed)?;
    let _serialized = state_file::lock_connect(&env.state_dir)
        .map_err(|error| ConnectError::StartFailed(format!("the connect lock: {error}")))?;
    // Judged by the file this `hide` is, not the link it was invoked
    // through: the kit's command resolves into its package, and a link that
    // only looks like a package path has no authority.
    let package = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .ok()
        .as_deref()
        .and_then(hide_kit::bundled_kit_dir);
    // Before the daemon, so a daemon this connect starts or keeps finds the
    // server it will follow already running this app's Herdr.
    if let Some(herdr) = package.as_deref().and_then(|kit| given_herdr(env, kit))
        && let Some(socket) = &env.herdr_socket_path
        && let Some(record) = crate::herdr_handoff::hand_off(&herdr, Path::new(socket)).record()
    {
        eprintln!("{record}");
    }
    if let Some((state, health)) = healthy_daemon(env) {
        let running = health.get("build").and_then(serde_json::Value::as_str);
        if running == Some(build.as_str()) {
            return Ok((state, health));
        }
        if package.is_none() {
            eprintln!(
                "{}",
                serde_json::json!({
                    "component": "hide", "kind": "daemon.other_build_refused",
                    "pid": state.pid, "running_build": running, "build": build,
                })
            );
            return Err(ConnectError::OtherBuild(format!(
                "a hided of another build is running (pid {}, build {}); this hide is not from a desktop package (build {build}) and neither replaces nor attaches to it",
                state.pid,
                running.unwrap_or("unknown"),
            )));
        }
        if let Some(program) = health["starter"]["program"].as_str() {
            let program = std::path::PathBuf::from(program);
            return update_started_core(env, (state, health), &build, &program);
        }
        eprintln!(
            "{}",
            serde_json::json!({
                "component": "hide", "kind": "daemon.replacing",
                "pid": state.pid, "running_build": running, "build": build,
            })
        );
        stop_daemon(&env.state_dir, &state).map_err(|detail| {
            eprintln!(
                "{}",
                serde_json::json!({"component": "hide", "kind": "daemon.replace_failed", "pid": state.pid, "message": detail})
            );
            ConnectError::StartFailed(detail)
        })?;
    }
    let spawned_at = unix_ms();
    let mut daemon = spawn_daemon(env, false).map_err(ConnectError::StartFailed)?;
    let started = wait_healthy(env, &mut daemon, spawned_at);
    // The daemon outlives this command; nothing waits for it.
    std::mem::forget(daemon);
    started
}

/// The Herdr the app in `kit` ships, when it is the one this connect was
/// given (`HERDR_BIN_PATH`): the host hands its own unless the operator
/// named another, whose server is theirs to keep.
fn given_herdr(env: &Env, kit: &Path) -> Option<std::path::PathBuf> {
    let shipped = kit.join(format!("herdr{}", std::env::consts::EXE_SUFFIX));
    let given = env.herdr_bin_path.as_deref()?;
    let same = hide_platform::fs::identity::same_file(given, &shipped).ok()?;
    same.then_some(shipped)
}

/// How long an updated core has to answer `/health` once its update said
/// it takes links.
const UPDATED_CORE_ANSWERS_WITHIN: Duration = Duration::from_secs(10);

/// A core of another build that its starter runs here, the core a move
/// placed on this machine (PRD core-host-node-move Q13), is never stopped
/// for this app's own: its login item would start that build again at the
/// next login beside this one. This app, when newer, updates it through
/// the updater a node runs (B10): this build goes into a version folder
/// beside the core's, and its own `hided core-update` replaces the core
/// through the starter, or goes back to the build it ran. Each connection
/// is one attempt (B20). An older app is told to update; a build neither
/// orders is refused as another build.
fn update_started_core(
    env: &Env,
    (state, health): Healthy,
    build: &str,
    program: &Path,
) -> Result<Healthy, ConnectError> {
    use crate::build_order::{Release, Standing};
    let core: Release = serde_json::from_value(health["release"].clone()).map_err(|error| {
        ConnectError::StartFailed(format!("the core's health names no release: {error}"))
    })?;
    let running = health["build"]
        .as_str()
        .ok_or_else(|| ConnectError::StartFailed("the core's health names no build".to_owned()))?;
    let ours = Release::of_this_build();
    let builds = CoreBuilds {
        machine: crate::host_name().ok_or_else(|| {
            ConnectError::StartFailed("this machine would not say its name".to_owned())
        })?,
        core: core.shown(),
        app: ours.shown(),
    };
    let log = |kind: &str, fields: serde_json::Value| {
        let mut record = serde_json::json!({
            "component": "hide", "kind": kind, "pid": state.pid,
            "core": builds.core, "app": builds.app,
        });
        if let (Some(record), serde_json::Value::Object(fields)) = (record.as_object_mut(), fields)
        {
            record.extend(fields);
        }
        eprintln!("{record}");
    };
    match crate::build_order::standing((build, &ours), (running, &core)) {
        Standing::Same => return Ok((state, health)),
        Standing::Older => {
            log("core.newer_refused", serde_json::json!({}));
            return Err(ConnectError::CoreNewer(builds));
        }
        Standing::Unordered => {
            log("core.other_build_refused", serde_json::json!({}));
            return Err(ConnectError::OtherBuild(format!(
                "the core its login item runs here is build {running}, which this app's build {build} neither updates nor replaces"
            )));
        }
        Standing::Newer => {}
    }
    let intent = crate::core_update::new_intent();
    log("core.update_started", serde_json::json!({"intent": intent}));
    let failed = |reason: String| {
        log(
            "core.update_failed",
            serde_json::json!({"intent": intent, "reason": reason}),
        );
        ConnectError::UpdateFailed(builds.clone())
    };
    // Only a build's own place is installed beside: a program anywhere else
    // names no install root.
    let root = hide_platform::path::to_wire(program)
        .map_err(|error| format!("{}: {error}", program.display()))
        .and_then(|wire| hide_kit::build_of(&wire))
        .and_then(|place| {
            hide_platform::path::from_wire(&place.root)
                .map_err(|error| format!("{}: {error}", place.root))
        })
        .map_err(failed)?;
    let packages = daemon_binary()
        .map(|hided| {
            hide_node::ssh::host::HelperPackages::new(hided.parent().map(Path::to_path_buf))
        })
        .map_err(failed)?;
    let new = packages.install_here(&root).map_err(failed)?;
    let mut command = std::process::Command::new(&new);
    command
        .arg("core-update")
        .arg("--state-dir")
        .arg(&env.state_dir)
        .arg("--previous")
        .arg(program)
        .args(["--intent", &intent]);
    // Its own process group, so a host that gives up on this connect and
    // kills it never cuts the update short.
    let finished = hide_platform::process::run_to_end(
        &mut command,
        crate::core_update::WITHIN,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|error| failed(format!("core-update: {error:?}")))?;
    let exit = finished
        .code
        .map_or_else(|| "on a signal".to_owned(), |code| code.to_string());
    crate::core_update::outcome(&finished.stdout, &exit, &finished.stderr).map_err(failed)?;
    log("core.updated", serde_json::json!({"intent": intent}));
    wait_for_build(env, build)
}

/// The daemon of this state folder once it runs `build` and answers.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_for_build(env: &Env, build: &str) -> Result<Healthy, ConnectError> {
    let deadline = std::time::Instant::now() + UPDATED_CORE_ANSWERS_WITHIN;
    loop {
        if let Some((state, health)) = healthy_daemon(env)
            && health["build"] == build
        {
            return Ok((state, health));
        }
        if std::time::Instant::now() >= deadline {
            return Err(ConnectError::NoResponse(format!(
                "the core updated to build {build} did not answer within {}s",
                UPDATED_CORE_ANSWERS_WITHIN.as_secs()
            )));
        }
        std::thread::sleep(HEALTH_PAUSE);
    }
}

/// Moves the legacy default state folder into place before anything looks
/// for a daemon (PRD hide-home-layout D-05). The daemon running from it is
/// the process its state names, proven to be that daemon ([`proven_daemon`]),
/// so a reused pid is never signalled.
#[cfg(unix)]
fn move_legacy_state(env: &Env) -> Result<(), String> {
    let Some(legacy) = env.legacy_state_dir.as_deref() else {
        return Ok(());
    };
    let moved = crate::state_move::move_legacy(legacy, &env.state_dir, |legacy| {
        let Some(state) = state_file::read_state(legacy)
            .ok()
            .flatten()
            .and_then(|state| {
                proven_daemon(state, |state| health_of(state, HEALTH_REQUEST).is_ok())
            })
        else {
            return Ok(None);
        };
        stop_daemon(legacy, &state)?;
        Ok(Some(state.pid))
    })
    .map_err(|detail| format!("the state folder could not be moved: {detail}"))?;
    if let crate::state_move::Moved::Moved { stopped_pid } = moved {
        eprintln!(
            "{}",
            serde_json::json!({
                "component": "hide", "kind": "state.moved",
                "from": legacy.display().to_string(),
                "to": env.state_dir.display().to_string(),
                "stopped_pid": stopped_pid,
            })
        );
    }
    Ok(())
}

/// `state` when its pid is still the daemon that recorded it. A state that
/// recorded its daemon's process start is proven by the kernel's answer
/// alone, so a daemon too busy to answer is still found; one an earlier
/// build wrote without a start (before v0.3.0) is judged by liveness only,
/// which a reused pid passes, so it is proven only when the daemon answers
/// as itself (`answers`), and an answer that does not come leaves it
/// unproven.
#[cfg(unix)]
fn proven_daemon(
    state: DaemonState,
    answers: impl FnOnce(&DaemonState) -> bool,
) -> Option<DaemonState> {
    let proven = still_the_daemon(&state) && (state.pid_started.is_some() || answers(&state));
    proven.then_some(state)
}

fn open(env: &Env) -> Result<(), String> {
    let (state, health) = connect(env).map_err(|error| error.detail())?;
    open_browser(&state, &health)
}

fn connect_json(env: &Env) -> Result<(), String> {
    let (line, result) =
        match connect(env).map(|(state, health)| attached_json(&state, &health, "ok")) {
            Ok(Ok(line)) => (line, Ok(())),
            // The node names a machine this build does not take, so where its
            // screens run is unknown: no address is handed out.
            Ok(Err(detail)) => (
                serde_json::json!({"ok": false, "reason": "no_response", "detail": detail}),
                Err(format!("no_response: {detail}")),
            ),
            Err(error) => (
                match &error {
                    ConnectError::StateRefused { file, .. } => serde_json::json!({
                        "ok": false,
                        "reason": error.reason(),
                        "detail": error.detail(),
                        "file": file,
                    }),
                    ConnectError::CoreNewer(builds) | ConnectError::UpdateFailed(builds) => {
                        serde_json::json!({
                            "ok": false,
                            "reason": error.reason(),
                            "detail": error.detail(),
                            "machine": builds.machine,
                            "core": builds.core,
                            "app": builds.app,
                        })
                    }
                    _ => serde_json::json!({
                        "ok": false,
                        "reason": error.reason(),
                        "detail": error.detail(),
                    }),
                },
                Err(format!("{}: {}", error.reason(), error.detail())),
            ),
        };
    println!("{line}");
    result
}

/// The attach-only probe: never starts a daemon.
/// The environment keys a command reads. `None` is every key: the command acts
/// on the daemon's whole configuration, as the daemon does, and refuses on any
/// invalid one. `stop` and `status` only find a daemon, so an unrelated key
/// (an opener, a port) must not decide whether a running daemon can be asked
/// about or stopped.
fn needed_keys(kind: &CommandKind) -> Option<&'static [&'static str]> {
    match kind {
        CommandKind::Stop | CommandKind::Status { json: true } => Some(env::STATE_FOLDER_KEYS),
        CommandKind::RuntimePrepare => Some(&[env::HOME]),
        CommandKind::Status { json: false } => Some(env::STATUS_KEYS),
        _ => None,
    }
}

fn status_json(state_dir: &Path) -> Result<(), String> {
    let line = match healthy_daemon_in(state_dir) {
        Some((state, health)) => attached_json(&state, &health, "running")?,
        None => serde_json::json!({ "running": false }),
    };
    println!("{line}");
    Ok(())
}

/// A live daemon as a host loads it. The URL carries the token in its hash,
/// exactly as `open` hands it to a browser.
fn attached_json(
    state: &DaemonState,
    health: &serde_json::Value,
    flag: &str,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        flag: true,
        "url": daemon_url(state, health)?,
        "port": state.port,
        "pid": state.pid,
    }))
}

fn daemon_url(state: &DaemonState, health: &serde_json::Value) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{}/#token={}", state.port, state.token);
    Ok(match screen_node(health)? {
        Some(node) => format!("{url}&node={node}"),
        None => url,
    })
}

/// The machine a node-role daemon's screens run on, which the page reads
/// from its hash to tell this machine's panes and checkouts from the
/// core's (PRD core-host-node-remote-core B13); `None` for a daemon that is
/// its own core, whose screens' machine is the core's. Read from the
/// `/health` the daemon answered when it was found, so no second request can
/// fail where the first answered. A node id this build does not take is an
/// error: a node's screen opened as the core's would draw the node's panes
/// as the core's.
fn screen_node(health: &serde_json::Value) -> Result<Option<String>, String> {
    if health["role"] != "node" {
        return Ok(None);
    }
    health["node"]
        .as_str()
        .filter(|node| {
            !node.is_empty()
                && node.len() <= 128
                && node
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
        .map(|node| Some(node.to_owned()))
        .ok_or_else(|| "the node daemon names a machine this build does not take".to_owned())
}

fn status(state_dir: &Path, idle_secs: u64) -> Result<(), String> {
    match healthy_daemon_in(state_dir).map(|(state, _)| state) {
        Some(state) => {
            let health = health_json(state.port, HEALTH_REQUEST)?;
            let clients = health
                .get("clients")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let idle = health
                .get("idle_remaining_secs")
                .and_then(|value| value.as_u64())
                .unwrap_or(idle_secs);
            println!(
                "pid {}  port {}  clients {}  idle {}s remaining",
                state.pid, state.port, clients, idle
            );
            Ok(())
        }
        None => {
            println!("hided is not running");
            Ok(())
        }
    }
}

/// Answers the folder the desktop host runs this package's daemon, CLI and
/// Herdr from, as one JSON line. A Windows package's is a copy outside its
/// install folder (PRD auto-update D-10), made here once per build, and the
/// copies of other builds nothing runs from are removed; what could not be
/// removed is reported to stderr as records the host logs (D-11). Elsewhere
/// it is the package's own resources folder. A `hide` outside a package has
/// no runtime to prepare.
fn runtime_prepare(home: &Path) -> Result<(), String> {
    let package = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .ok()
        .as_deref()
        .and_then(hide_kit::bundled_kit_dir);
    let Some(package) = package else {
        println!(
            "{}",
            serde_json::json!({ "ok": false, "reason": "not_packaged" })
        );
        return Err(
            "this hide is not from a desktop package, so it has no runtime to prepare".to_owned(),
        );
    };
    if !cfg!(windows) || hide_kit::runtime::is_runtime_copy(&package) {
        println!(
            "{}",
            serde_json::json!({ "ok": true, "dir": package, "copied": false })
        );
        return Ok(());
    }
    let daemon = package.join(format!("hided{}", std::env::consts::EXE_SUFFIX));
    let build = hide_kit::runtime::build_name(VERSION, &crate::build_id::of_file(&daemon)?)?;
    let prepared = hide_kit::runtime::prepare(&package, home, &build)?;
    let pruned = &prepared.pruned;
    for name in &pruned.in_use {
        eprintln!(
            "{}",
            serde_json::json!({ "component": "hide", "kind": "runtime.copy_in_use", "copy": name })
        );
    }
    for (name, detail) in &pruned.failed {
        eprintln!(
            "{}",
            serde_json::json!({ "component": "hide", "kind": "runtime.copy_remove_failed", "copy": name, "detail": detail })
        );
    }
    if let Some(count) = pruned.over_cap {
        eprintln!(
            "{}",
            serde_json::json!({ "component": "hide", "kind": "runtime.copies_over_cap", "count": count, "cap": hide_kit::runtime::COPIES_CAP })
        );
    }
    println!(
        "{}",
        serde_json::json!({ "ok": true, "dir": prepared.dir, "copied": prepared.copied, "removed": pruned.removed })
    );
    Ok(())
}

fn stop(state_dir: &Path) -> Result<(), String> {
    if let Some(state) = state_file::read_state(state_dir).map_err(|e| e.to_string())? {
        stop_daemon(state_dir, &state)?;
    }
    Ok(())
}

/// Whether `state`'s pid is still the daemon that recorded it. A pid another
/// process has reused (another account's, or a later daemon's) is not: it
/// reads as gone and is never signalled. A state with no recorded start comes
/// from an earlier build and is judged by liveness alone. A daemon that has
/// ended but that its starter has not reaped yet is gone too, although Linux
/// still lists its start time.
fn still_the_daemon(state: &DaemonState) -> bool {
    match state.pid_started {
        Some(_) => state.is_proven_running(),
        None => process::is_alive(state.pid),
    }
}

/// Ends the daemon `state` names: SIGTERM and five seconds for its graceful
/// stop, which ends its AI requests and provider processes, then SIGKILL.
/// An error only when it is still alive after that.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn stop_daemon(state_dir: &Path, state: &DaemonState) -> Result<(), String> {
    // A pid that is gone, or is another process now, has nothing to stop:
    // only the state is cleared.
    if !still_the_daemon(state) {
        state_file::forget_daemon(state_dir, state.pid);
        return Ok(());
    }
    // A damaged state's pid names no daemon (the platform layer refuses it),
    // so the state is only cleared.
    let asked = process::terminate(state.pid);
    if asked.is_err_and(|error| error.kind() == io::ErrorKind::InvalidInput) {
        state_file::forget_daemon(state_dir, state.pid);
        return Ok(());
    }
    for _ in 0..50 {
        if !still_the_daemon(state) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if still_the_daemon(state) {
        // The daemon leads its own group, so its provider processes go too.
        let _ = process::kill_tree(state.pid);
        for _ in 0..20 {
            if !still_the_daemon(state) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if still_the_daemon(state) {
        return Err(format!(
            "the running hided (pid {}) did not stop",
            state.pid
        ));
    }
    // Only the state of the daemon stopped here: another connect may have
    // started its own since, and its state and lock stay.
    state_file::forget_daemon(state_dir, state.pid);
    Ok(())
}

fn serve(mut env: Env, keep_alive: bool) -> Result<(), String> {
    env.keep_alive = keep_alive || env.keep_alive;
    // The daemon runs inside this CLI here; its build is the `hided` this
    // CLI ships beside, the one `hide connect` compares against.
    env.build = Some(crate::build_id::of_file(&daemon_binary()?)?);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("hide-serve")
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(crate::run_daemon(env))
}

fn dev(mut env: Env) -> Result<(), String> {
    env.keep_alive = true;
    if env.vite_origin.is_none() {
        env.vite_origin = Some("http://127.0.0.1:5173".into());
    }
    if env.bind.port() == 0 {
        env.bind = "127.0.0.1:9876".parse().expect("loopback");
    }
    println!(
        "HIDED_ORIGIN=http://127.0.0.1:{}  Vite origin {}",
        env.bind.port(),
        env.vite_origin.as_deref().unwrap_or("")
    );
    serve(env, true)
}

/// The `hided` beside this CLI's real file (`hided.exe` on Windows). A CLI
/// reached through a symlink (a desktop host's `HIDE_CLI_PATH`, a PATH entry)
/// resolves to the build it names; there is no other daemon to run, and
/// running this CLI in its place would start one more CLI per attempt, each
/// starting the next.
fn daemon_binary() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|error| error.to_string())?;
    exe.parent()
        .map(|dir| dir.join(format!("hided{}", std::env::consts::EXE_SUFFIX)))
        .filter(|path| path.is_file())
        .ok_or_else(|| format!("no hided beside {}", exe.display()))
}

fn spawn_daemon(env: &Env, keep_alive: bool) -> Result<std::process::Child, String> {
    // The daemon would refuse the same value after it is spawned; checking
    // here answers `start_failed` with the path instead of a ten-second
    // `no_response` that names nothing.
    if let Some(error) = env::herdr_bin_error(env) {
        return Err(error);
    }
    let mut command = Command::new(daemon_binary()?);
    command.env("HIDE_STATE_DIR", &env.state_dir);
    if keep_alive {
        command.env("HIDE_KEEP_ALIVE", "1");
    }
    if let Some(socket) = &env.herdr_socket_path {
        command.env("HERDR_SOCKET_PATH", socket);
    }
    if let Some(bin) = &env.herdr_bin_path {
        command.env("HERDR_BIN_PATH", bin);
    }
    if let Some(origin) = &env.vite_origin {
        command.env("HIDE_VITE_ORIGIN", origin);
    }
    spawn_owned(&mut command).map_err(|error| error.to_string())
}

/// How long `hide connect` waits, on the wall clock, for the daemon it
/// started to show progress: to answer `/health`, or to report on
/// `/starting` a step of its start past the last one seen. A start that
/// moves is waited for step by step, and one that stands still this long is
/// refused; the error names this same bound and the step it stood in.
const HEALTHY_WITHIN: Duration = Duration::from_secs(10);

/// The longest one `/health` request may take.
const HEALTH_REQUEST: Duration = Duration::from_secs(1);

/// The pause between two looks at a daemon that has not answered yet.
const HEALTH_PAUSE: Duration = Duration::from_millis(100);

/// Waits for the daemon `hide connect` started while it makes progress. One
/// that exits first ends the wait at once, with the reason it logged when it
/// refused its state folder (PRD core-host-node B2), since its stderr goes
/// nowhere.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_healthy(
    env: &Env,
    daemon: &mut std::process::Child,
    spawned_at: u64,
) -> Result<Healthy, ConnectError> {
    let pid = daemon.id();
    let waited = wait_started(
        HEALTHY_WITHIN,
        std::time::Instant::now,
        std::thread::sleep,
        || daemon.try_wait().ok().flatten(),
        |timeout| look(&env.state_dir, pid, timeout),
    );
    match waited {
        Ok(healthy) => Ok(healthy),
        Err(Waited::Timeout(detail)) => Err(ConnectError::NoResponse(detail)),
        Err(Waited::Exited(status)) => {
            let refusal = herdr_core::diagnostics::newest_record(
                &env.state_dir.join("core-state.json"),
                |record| refused_by(record, pid, spawned_at),
            );
            Err(match refusal {
                Some(record) => ConnectError::StateRefused {
                    file: record["file"].as_str().unwrap_or_default().to_owned(),
                    detail: record["reason"].as_str().unwrap_or_default().to_owned(),
                },
                None => ConnectError::StartFailed(format!(
                    "hided exited with {status} before it answered"
                )),
            })
        }
    }
}

/// The refusal this start's daemon logged: its pid, written no earlier
/// than it was spawned, so a record an earlier daemon with a reused pid
/// left in the same Logs file is not read as this one's.
fn refused_by(record: &serde_json::Value, pid: u32, spawned_at: u64) -> bool {
    record["kind"] == "node_migration.refused"
        && record["pid"].as_u64() == Some(u64::from(pid))
        && record["at_unix_ms"]
            .as_u64()
            .is_some_and(|at| at >= spawned_at)
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

enum Waited {
    Timeout(String),
    Exited(std::process::ExitStatus),
}

/// What one look at a started daemon that is not healthy yet saw: what
/// stood in the way, and the step of its start it reported, if it did.
struct Unready {
    detail: String,
    start: Option<StartStep>,
}

/// A step of a daemon's start, as its seat's `/starting` reports it; the
/// last is the role mounted, after which `/health` answers.
#[derive(Clone, Debug, PartialEq)]
struct StartStep {
    step: u64,
    stage: String,
    mounted: bool,
}

/// One look, within `timeout`, at the daemon `pid` this connect started.
/// Its seat answers `/starting` at once, while `/health` waits for the
/// role to mount, so a start still under way is read from `/starting`
/// alone, and `/health` is asked once a role is mounted, or of a seat that
/// does not answer for `pid` (no state written yet).
fn look(state_dir: &Path, pid: u32, timeout: Duration) -> Result<Healthy, Unready> {
    let asked = std::time::Instant::now();
    let start = start_of(state_dir, pid, timeout);
    if let Some(unmounted) = start.as_ref().filter(|start| !start.mounted) {
        return Err(Unready {
            detail: format!("step {} ({}) of its start", unmounted.step, unmounted.stage),
            start,
        });
    }
    probe_daemon(state_dir, timeout.saturating_sub(asked.elapsed()))
        .map_err(|detail| Unready { detail, start })
}

/// Waits until `probe` answers, while the daemon makes progress: each look
/// that reports a later step of its start gives it `within` again, and
/// `within` with no new step ends the wait. A start passes a fixed number of
/// steps, so the wait stays bounded however slowly each one runs.
fn wait_started<T>(
    within: Duration,
    now: impl Fn() -> std::time::Instant,
    mut pause: impl FnMut(Duration),
    mut exited: impl FnMut() -> Option<std::process::ExitStatus>,
    mut probe: impl FnMut(Duration) -> Result<T, Unready>,
) -> Result<T, Waited> {
    let mut deadline = now() + within;
    let mut reached: Option<u64> = None;
    loop {
        let left = deadline.saturating_duration_since(now());
        let seen = match probe(left.min(HEALTH_REQUEST)) {
            Ok(state) => return Ok(state),
            Err(seen) => seen,
        };
        if let Some(status) = exited() {
            return Err(Waited::Exited(status));
        }
        if let Some(step) = seen.start.map(|start| start.step)
            && reached.is_none_or(|reached| step > reached)
        {
            reached = Some(step);
            deadline = now() + within;
        }
        let left = deadline.saturating_duration_since(now());
        if left.is_zero() {
            let detail = &seen.detail;
            return Err(Waited::Timeout(if reached.is_some() {
                format!("hided made no progress for {within:?}; last waited on {detail}")
            } else {
                format!("hided did not become healthy within {within:?}; last waited on {detail}")
            }));
        }
        pause(left.min(HEALTH_PAUSE));
    }
}

/// The step the start of daemon `pid` has reached, as its seat answers
/// `/starting` within `timeout`; none when the state names another daemon or
/// the seat does not answer for `pid`.
fn start_of(state_dir: &Path, pid: u32, timeout: Duration) -> Option<StartStep> {
    let state = state_file::read_state(state_dir).ok()??;
    if state.pid != pid {
        return None;
    }
    let answer = loopback_json(state.port, "/starting", timeout).ok()?;
    if answer["pid"].as_u64() != Some(u64::from(pid)) {
        return None;
    }
    Some(StartStep {
        step: answer["step"].as_u64()?,
        stage: answer["stage"].as_str()?.to_owned(),
        mounted: answer["mounted"].as_bool()?,
    })
}

/// Looks at the started daemon until it answers or `within` has passed on
/// `now`'s clock. No request may outlast the time left, so the wait ends at
/// the bound it names, and the error says what was last seen, so a slow
/// start names its stage.
#[cfg(test)]
fn wait_healthy_within<T>(
    within: Duration,
    now: impl Fn() -> std::time::Instant,
    pause: impl FnMut(Duration),
    probe: impl FnMut(Duration) -> Result<T, Unready>,
) -> Result<T, String> {
    wait_started(within, now, pause, || None, probe).map_err(|waited| match waited {
        Waited::Timeout(detail) => detail,
        Waited::Exited(status) => format!("exited with {status}"),
    })
}

/// The live daemon of this state folder and what its `/health` answered.
/// The answer has to come from the process the state names: a pid reused
/// after a crash, with another daemon on the port, is a stale state that
/// `hide connect` must never signal.
fn healthy_daemon(env: &Env) -> Option<(DaemonState, serde_json::Value)> {
    healthy_daemon_in(&env.state_dir)
}

fn healthy_daemon_in(state_dir: &Path) -> Option<(DaemonState, serde_json::Value)> {
    probe_daemon(state_dir, HEALTH_REQUEST).ok()
}

/// The daemon `state_dir` names and its `/health`, asked within `timeout`,
/// or what stands in the way: no state yet, a pid that is not that daemon,
/// or a `/health` that failed or came from another process.
fn probe_daemon(
    state_dir: &Path,
    timeout: Duration,
) -> Result<(DaemonState, serde_json::Value), String> {
    let state = state_file::read_state(state_dir)
        .map_err(|error| format!("the daemon state, which could not be read: {error}"))?
        .ok_or_else(|| "the daemon to write its state".to_owned())?;
    if !still_the_daemon(&state) {
        return Err(format!(
            "pid {} the state names, which is not that daemon",
            state.pid
        ));
    }
    let health = health_of(&state, timeout)?;
    Ok((state, health))
}

/// The `/health` of the daemon `state` names, asked within `timeout`; an
/// answer for another pid is a failure.
fn health_of(state: &DaemonState, timeout: Duration) -> Result<serde_json::Value, String> {
    let health = health_json(state.port, timeout)
        .map_err(|error| format!("/health on port {}: {error}", state.port))?;
    let answered = health.get("pid").and_then(serde_json::Value::as_u64);
    if answered != Some(u64::from(state.pid)) {
        return Err(format!(
            "/health on port {}, which answered for pid {answered:?} rather than {}",
            state.port, state.pid
        ));
    }
    Ok(health)
}

/// The daemon's `/health`, asked in process: a forked `curl` exists only
/// where the system ships one at `/usr/bin/curl`, and Windows has none. The
/// request goes to this machine's loopback, so no proxy from the
/// environment may carry it, and a status other than 2xx is a failure.
fn health_json(port: u16, timeout: Duration) -> Result<serde_json::Value, String> {
    loopback_json(port, "/health", timeout)
}

/// The JSON a daemon on this machine's `port` answers at `path`.
fn loopback_json(port: u16, path: &str, timeout: Duration) -> Result<serde_json::Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .proxy(None)
        .build()
        .into();
    let body = agent
        .get(&format!("http://127.0.0.1:{port}{path}"))
        .call()
        .map_err(|error| error.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&body).map_err(|error| error.to_string())
}

fn open_browser(state: &DaemonState, health: &serde_json::Value) -> Result<(), String> {
    let url = daemon_url(state, health)?;
    Command::new("/usr/bin/open")
        .arg(&url)
        .status()
        .map_err(|error| error.to_string())?;
    println!("{url}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The address is decided from the health the daemon answered when it
    /// was found: a daemon that does not answer a second request still gets
    /// its own, and a node id the filter refuses gets none.
    #[test]
    fn a_daemon_s_address_comes_from_the_health_it_answered() {
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let state = DaemonState {
            pid: 1,
            port,
            token: "t".to_owned(),
            socket: None,
            started_at: String::new(),
            pid_started: None,
        };
        let core = serde_json::json!({"role": "core"});
        assert_eq!(
            daemon_url(&state, &core).unwrap(),
            format!("http://127.0.0.1:{port}/#token=t"),
            "no second request decides the address"
        );
        let node = serde_json::json!({"role": "node", "node": "mac-1"});
        assert_eq!(
            daemon_url(&state, &node).unwrap(),
            format!("http://127.0.0.1:{port}/#token=t&node=mac-1")
        );
        let refused = daemon_url(
            &state,
            &serde_json::json!({"role": "node", "node": "mac 1&x=y"}),
        )
        .expect_err("a node id the filter refuses");
        assert!(refused.contains("does not take"), "{refused}");
    }

    #[test]
    fn a_refusal_is_read_only_from_the_daemon_this_start_spawned() {
        let record = |pid: u32, at: u64| serde_json::json!({"kind": "node_migration.refused", "pid": pid, "at_unix_ms": at, "file": "/s/node.json"});
        assert!(refused_by(&record(42, 1_000), 42, 1_000));
        // Another daemon's, or one an earlier daemon with the same pid left.
        assert!(!refused_by(&record(43, 1_000), 42, 1_000));
        assert!(!refused_by(&record(42, 999), 42, 1_000));
        assert!(!refused_by(
            &serde_json::json!({"kind": "node_migration.refused", "pid": 42}),
            42,
            0
        ));
    }

    fn recorded(pid: u32, pid_started: Option<u64>) -> DaemonState {
        DaemonState {
            pid,
            port: 7001,
            token: "abc".into(),
            socket: None,
            started_at: "now".into(),
            pid_started,
        }
    }

    #[test]
    fn a_pid_reused_by_another_process_reads_as_gone_and_is_not_signalled() {
        // A live process stands in for the stranger that now holds the pid:
        // the state recorded a different start for it.
        let mut stranger = Command::new("sleep").arg("30").spawn().unwrap();
        let real_start = process::start_time(stranger.id()).unwrap();
        let replaced = recorded(stranger.id(), Some(real_start.wrapping_add(1)));
        assert!(!still_the_daemon(&replaced));

        let dir = tempfile::tempdir().unwrap();
        state_file::write_state(dir.path(), &replaced).unwrap();
        stop_daemon(dir.path(), &replaced).unwrap();
        assert!(
            stranger.try_wait().unwrap().is_none(),
            "the process that reused the pid was signalled"
        );
        assert!(
            state_file::read_state(dir.path()).unwrap().is_none(),
            "the stale state is cleared"
        );

        // The same process under the start the state recorded is the daemon.
        assert!(still_the_daemon(&recorded(stranger.id(), Some(real_start))));
        let _ = stranger.kill();
        let _ = stranger.wait();
    }

    #[cfg(unix)]
    #[test]
    fn a_legacy_daemon_that_recorded_its_start_is_proven_without_an_answer() {
        let me = std::process::id();
        let start = process::start_time(me).unwrap();
        let unasked = |_: &DaemonState| -> bool { panic!("a recorded start needs no answer") };
        assert!(
            proven_daemon(recorded(me, Some(start)), unasked).is_some(),
            "a daemon too busy to answer /health is still the daemon its start names"
        );
        assert!(
            proven_daemon(recorded(me, Some(start.wrapping_add(1))), unasked).is_none(),
            "a pid another process reused is not the daemon"
        );
        // A state from before v0.3.0 recorded no start: only an answer proves it.
        assert!(proven_daemon(recorded(me, None), |_| true).is_some());
        assert!(proven_daemon(recorded(me, None), |_| false).is_none());
    }

    #[test]
    fn a_daemon_that_never_answers_ends_the_wait_at_its_bound_and_names_what_it_waited_on() {
        let start = std::time::Instant::now();
        let clock = std::cell::Cell::new(start);
        let asked = std::cell::RefCell::new(Vec::new());
        let error = wait_healthy_within::<()>(
            HEALTHY_WITHIN,
            || clock.get(),
            |pause| clock.set(clock.get() + pause),
            // A `/health` that times out takes all the time it is given.
            |timeout| {
                let left = (start + HEALTHY_WITHIN).saturating_duration_since(clock.get());
                asked.borrow_mut().push((timeout, left));
                clock.set(clock.get() + timeout);
                Err(Unready {
                    detail: "/health on port 7001: timed out".to_owned(),
                    start: None,
                })
            },
        )
        .unwrap_err();
        assert_eq!(
            clock.get() - start,
            HEALTHY_WITHIN,
            "the wait ends at the bound it names"
        );
        assert!(
            asked
                .borrow()
                .iter()
                .all(|(timeout, left)| timeout <= left && *timeout <= HEALTH_REQUEST),
            "no request outlasts the time left: {:?}",
            asked.borrow()
        );
        assert_eq!(
            error,
            "hided did not become healthy within 10s; last waited on /health on port 7001: timed out"
        );
    }

    /// A start that reports a later step every few seconds, as a first
    /// start on a cold machine does, slower in all than the bound: it is
    /// waited for, since each step it reports is progress.
    #[test]
    fn a_start_that_moves_is_waited_for_past_the_bound() {
        let start = std::time::Instant::now();
        let clock = std::cell::Cell::new(start);
        let step_every = Duration::from_secs(6);
        let answered = wait_healthy_within(
            HEALTHY_WITHIN,
            || clock.get(),
            |pause| clock.set(clock.get() + pause),
            |timeout| {
                clock.set(clock.get() + timeout);
                let step = (clock.get() - start).as_secs() / step_every.as_secs();
                if step >= 5 {
                    return Ok(step);
                }
                Err(Unready {
                    detail: format!("step {step} of its start"),
                    start: Some(StartStep {
                        step,
                        stage: format!("stage {step}"),
                        mounted: false,
                    }),
                })
            },
        )
        .expect("a start that moves is waited for");
        assert_eq!(answered, 5);
        assert!(clock.get() - start > HEALTHY_WITHIN * 2);
    }

    /// A start that stands in one step is refused once it has stood there
    /// for the bound, and the error names the step.
    #[test]
    fn a_start_that_stands_still_is_refused_at_the_bound_naming_its_step() {
        let start = std::time::Instant::now();
        let clock = std::cell::Cell::new(start);
        let stuck_from = Duration::from_secs(4);
        let error = wait_healthy_within(
            HEALTHY_WITHIN,
            || clock.get(),
            |pause| clock.set(clock.get() + pause),
            |timeout| -> Result<(), Unready> {
                clock.set(clock.get() + timeout);
                let step = if clock.get() - start < stuck_from {
                    1
                } else {
                    3
                };
                Err(Unready {
                    detail: format!("step {step} (core.owner) of its start"),
                    start: Some(StartStep {
                        step,
                        stage: "core.owner".to_owned(),
                        mounted: false,
                    }),
                })
            },
        )
        .unwrap_err();
        let stood = clock.get() - start;
        assert!(
            stood >= stuck_from + HEALTHY_WITHIN
                && stood <= stuck_from + HEALTHY_WITHIN + HEALTH_REQUEST,
            "refused after {stood:?}"
        );
        assert_eq!(
            error,
            "hided made no progress for 10s; last waited on step 3 (core.owner) of its start"
        );
    }

    // The web e2e fixture runs `hide stop` on the daemon it started, and
    // cannot reap it while that runs.
    #[cfg(unix)]
    #[test]
    fn an_ended_daemon_its_starter_has_not_reaped_reads_as_stopped() {
        let mut ended = Command::new("sleep").arg("30").spawn().unwrap();
        let state = recorded(ended.id(), Some(process::start_time(ended.id()).unwrap()));
        let dir = tempfile::tempdir().unwrap();
        state_file::write_state(dir.path(), &state).unwrap();
        ended.kill().unwrap();
        stop_daemon(dir.path(), &state).unwrap();
        assert!(
            state_file::read_state(dir.path()).unwrap().is_none(),
            "the stopped daemon's state is cleared"
        );
        ended.wait().unwrap();
    }

    #[test]
    fn a_state_with_no_recorded_start_is_judged_by_liveness() {
        assert!(still_the_daemon(&recorded(std::process::id(), None)));
        assert!(!still_the_daemon(&recorded(0, None)));
    }

    #[test]
    fn spawn_alone_supports_subcommand_help_and_rejects_no_watch_before_delivery() {
        let parse = |args: &[&str]| {
            parse_args(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
        };
        assert_eq!(
            parse(&["hide", "agent", "spawn", "--help"]).unwrap(),
            CommandKind::AgentSpawnHelp
        );
        for verb in ["register", "list", "show", "end"] {
            assert!(
                parse(&["hide", "agent", verb, "--help"])
                    .unwrap_err()
                    .contains("Unknown flag")
            );
        }
        let error = parse(&["hide", "agent", "spawn", "--no-watch"]).unwrap_err();
        assert!(error.starts_with("Unknown flag for hide agent spawn: --no-watch\n"));
        assert!(error.contains(crate::agent_cli::USAGE));
    }

    #[test]
    fn parse_serve_keep_alive() {
        let args = ["hide", "serve", "--keep-alive"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        assert_eq!(
            parse_args(&args).unwrap(),
            CommandKind::Serve { keep_alive: true }
        );
    }

    #[test]
    fn workspace_memory_takes_an_event_runtime_and_session() {
        let parse = |args: &[&str]| {
            let mut words = vec!["hide", "workspace", "memory"];
            words.extend(args);
            parse_args(
                &words
                    .iter()
                    .map(|word| (*word).to_owned())
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            parse(&[
                "--event",
                "SessionStart",
                "--runtime",
                "codex",
                "--session",
                "s-1"
            ])
            .unwrap(),
            CommandKind::WorkspaceMemory {
                event: "SessionStart".into(),
                runtime: "codex".into(),
                session: "s-1".into(),
            }
        );
        assert_eq!(
            parse(&[
                "--event",
                "UserPromptSubmit",
                "--runtime",
                "pi",
                "--session",
                "/sessions/a.jsonl",
            ])
            .unwrap(),
            CommandKind::WorkspaceMemory {
                event: "UserPromptSubmit".into(),
                runtime: "pi".into(),
                session: "/sessions/a.jsonl".into(),
            }
        );
        for args in [
            vec![],
            vec!["--event", "Stop", "--runtime", "codex", "--session", "s-1"],
            vec![
                "--event",
                "SessionStart",
                "--runtime",
                "codex",
                "--session",
                "-x",
            ],
            vec![
                "--event",
                "SessionStart",
                "--runtime",
                "",
                "--session",
                "s-1",
            ],
            vec![
                "--runtime",
                "codex",
                "--event",
                "SessionStart",
                "--session",
                "s-1",
            ],
            vec![
                "--event",
                "SessionStart",
                "--runtime",
                "codex",
                "--session",
                "s-1",
                "--cwd",
                "/work/project",
            ],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn workspace_memory_reads_at_most_the_prompt_limit_and_keeps_whole_characters() {
        let limit = herdr_core::memory_hook::PROMPT_LIMIT_BYTES;
        let long = "가".repeat(limit);
        let read = crate::workspace_cli::read_memory_prompt(long.as_bytes()).unwrap();
        assert!(
            read.len() <= limit && long.starts_with(&read),
            "{}",
            read.len()
        );
        assert!(read.len() > limit - 3);
        assert_eq!(
            crate::workspace_cli::read_memory_prompt("keep going".as_bytes()).unwrap(),
            "keep going"
        );
    }

    #[test]
    fn parse_connect_and_status_json() {
        let parse = |line: &[&str]| {
            let args = line.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
            parse_args(&args).unwrap()
        };
        assert_eq!(parse(&["hide", "connect"]), CommandKind::Connect);
        assert_eq!(
            parse(&["hide", "status", "--json"]),
            CommandKind::Status { json: true }
        );
        assert_eq!(
            parse(&["hide", "status"]),
            CommandKind::Status { json: false }
        );
    }

    #[test]
    fn browser_connect_has_no_workspace_override() {
        let parse = |args: &[&str]| {
            parse_args(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
        };
        assert_eq!(
            parse(&["hide", "browser", "connect"]).unwrap(),
            CommandKind::BrowserConnect { display_id: None }
        );
        assert_eq!(
            parse(&["hide", "browser", "connect", "--display", "browser-a"]).unwrap(),
            CommandKind::BrowserConnect {
                display_id: Some("browser-a".into())
            }
        );
        for args in [
            vec!["hide", "browser", "connect", "--display"],
            vec!["hide", "browser", "connect", "--workspace", "other"],
            vec![
                "hide",
                "browser",
                "connect",
                "--display",
                "browser-a",
                "--reveal",
            ],
        ] {
            assert!(parse(&args).is_err());
        }
    }

    #[test]
    fn attached_json_carries_the_token_url() {
        let state = DaemonState {
            pid: 42,
            port: 7001,
            token: "abc".into(),
            socket: None,
            started_at: "now".into(),
            pid_started: None,
        };
        assert_eq!(
            attached_json(&state, &serde_json::json!({"role": "core"}), "ok").unwrap(),
            serde_json::json!({
                "ok": true,
                "url": "http://127.0.0.1:7001/#token=abc",
                "port": 7001,
                "pid": 42,
            })
        );
    }

    #[test]
    fn parse_browser_open() {
        let parse = |line: &[&str]| {
            let args = line.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
            parse_args(&args)
        };
        assert_eq!(
            parse(&["hide", "browser", "open", "index.html"]),
            Ok(CommandKind::BrowserOpen {
                target: "index.html".into(),
                wait: false,
                request_id: None,
            })
        );
        assert_eq!(
            parse(&[
                "hide",
                "browser",
                "open",
                "localhost:3000",
                "--wait",
                "--request-id",
                "1234567890000-abc",
            ]),
            Ok(CommandKind::BrowserOpen {
                target: "localhost:3000".into(),
                wait: true,
                request_id: Some("1234567890000-abc".into()),
            })
        );
        for bad in [
            &["hide", "browser"][..],
            &["hide", "browser", "close", "a"],
            &["hide", "browser", "open"],
            &["hide", "browser", "open", "a", "b"],
            &["hide", "browser", "open", "a", "--pane"],
            &["hide", "browser", "open", "a", "--reveal"],
            &[
                "hide", "browser", "open", "a", "--pane", "p1", "--pane", "p2",
            ],
        ] {
            assert_eq!(parse(bad), Err(BROWSER_USAGE.to_owned()), "{bad:?}");
        }
    }

    #[test]
    fn memory_reads_a_card_name_and_joins_words_and_text() {
        use herdr_core::memory_command::MemoryCommand;
        let parse = |line: &[&str]| {
            parse_args(&line.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
        };
        assert_eq!(
            parse(&["hide", "memory", "show", "m3"]),
            Ok(CommandKind::Memory(MemoryCommand::Show { number: 3 }))
        );
        assert_eq!(
            parse(&["hide", "memory", "search", "테스트", "nextest"]),
            Ok(CommandKind::Memory(MemoryCommand::Search {
                words: "테스트 nextest".into()
            }))
        );
        assert_eq!(
            parse(&["hide", "memory", "add", "Lint runs before tests"]),
            Ok(CommandKind::Memory(MemoryCommand::Add {
                text: "Lint runs before tests".into()
            }))
        );
        assert_eq!(
            parse(&["hide", "memory", "forget", "m5"]),
            Ok(CommandKind::Memory(MemoryCommand::Forget { number: 5 }))
        );
        assert_eq!(
            parse(&["hide", "memory", "undo", "m5"]),
            Ok(CommandKind::Memory(MemoryCommand::Undo { number: 5 }))
        );
        assert_eq!(
            parse(&["hide", "memory", "placed", "m5", "docs/TESTING.md"]),
            Ok(CommandKind::Memory(MemoryCommand::Placed {
                number: 5,
                path: "docs/TESTING.md".into()
            }))
        );
        assert_eq!(
            parse(&[
                "hide",
                "memory",
                "building",
                "m5",
                "feat/overview",
                "docs/UI_BEHAVIOR.md"
            ]),
            Ok(CommandKind::Memory(MemoryCommand::Building {
                number: 5,
                branch: "feat/overview".into(),
                path: "docs/UI_BEHAVIOR.md".into()
            }))
        );
        assert_eq!(
            parse(&[
                "hide", "memory", "unbuilt", "m5", "no", "branch", "builds", "it"
            ]),
            Ok(CommandKind::Memory(MemoryCommand::Unbuilt {
                number: 5,
                reason: "no branch builds it".into()
            }))
        );
        assert_eq!(
            parse(&[
                "hide", "memory", "stale", "m5", "commits", "are", "English", "now"
            ]),
            Ok(CommandKind::Memory(MemoryCommand::Stale {
                number: 5,
                reason: "commits are English now".into()
            }))
        );
        assert_eq!(
            parse(&["hide", "memory", "release", "m5"]),
            Ok(CommandKind::Memory(MemoryCommand::Release { number: 5 }))
        );
        for bad in [
            &["hide", "memory", "forget"][..],
            &["hide", "memory", "undo", "5x"],
            &["hide", "memory"],
            &["hide", "memory", "show", "m0"],
            &["hide", "memory", "show", "m1", "m2"],
            &["hide", "memory", "search"],
            &["hide", "memory", "add", " "],
            &["hide", "memory", "edit", "m1"],
            &["hide", "memory", "placed", "m1"],
            &["hide", "memory", "building", "m1", "feat/overview"],
            &["hide", "memory", "unbuilt", "m1"],
            &["hide", "memory", "stale", "m1"],
            &["hide", "memory", "release"],
            &["hide", "memory", "release", "m1", "m2"],
        ] {
            assert_eq!(parse(bad), Err(MEMORY_USAGE.to_owned()), "{bad:?}");
        }
    }

    #[test]
    fn memory_answers_read_as_text_with_the_files_a_card_cites() {
        use herdr_core::memory_command::MemoryCommand;
        let shown = memory_text(
            &MemoryCommand::Show { number: 3 },
            &serde_json::json!({
                "card": "m3", "title": "Tests run with nextest", "kind": "rule",
                "state": "active", "body": "Run tests with nextest.",
                "sources": 2, "operator_sources": 1,
                "cited_paths": ["docs/TESTING.md"],
            }),
        );
        assert_eq!(
            shown,
            "m3 · rule · active · 2 sources, 1 from the operator\nTests run with nextest\n\nRun tests with nextest.\nFiles: docs/TESTING.md\n"
        );
        let compared = memory_text(
            &MemoryCommand::Show { number: 9 },
            &serde_json::json!({
                "card": "m9", "title": "Backlog follows one source", "kind": "decision",
                "state": "active", "body": "Missed work offers 백로그로.",
                "sources": 2, "operator_sources": 2,
                "document_comparison": {
                    "authority": "card_newer", "document": "docs/AI_PROVIDERS.md",
                    "document_text": "An explicit `이슈로` press drafts an issue.",
                    "document_at_unix_ms": 1, "operator_text": "좋아",
                    "operator_at_unix_ms": 2, "source_session": "s-1",
                },
            }),
        );
        assert_eq!(
            compared,
            "m9 · decision · active · 2 sources, 2 from the operator\nBacklog follows one source\n\nMissed work offers 백로그로.\nDocument comparison: the operator's words are newer than docs/AI_PROVIDERS.md.\nDocument says: An explicit `이슈로` press drafts an issue.\nOperator said (session s-1): 좋아\n"
        );
        let search = MemoryCommand::Search { words: "x".into() };
        assert_eq!(
            memory_text(
                &MemoryCommand::Skipped {
                    number: 6,
                    reason: "Guide unavailable".into()
                },
                &serde_json::json!({"card":"m6","outcome":"skipped"})
            ),
            "m6 returns to To ask with its reason. Other cards stay in the shared pull request; retry continues the same branch.\n"
        );
        assert_eq!(
            memory_text(
                &MemoryCommand::Building {
                    number: 6,
                    branch: "feat/overview".into(),
                    path: "docs/UI_BEHAVIOR.md".into()
                },
                &serde_json::json!({"card":"m6","outcome":"building","branch":"feat/overview","path":"docs/UI_BEHAVIOR.md"})
            ),
            "m6 goes into docs/UI_BEHAVIOR.md with the pull request of feat/overview; this attempt ends without one.\n"
        );
        assert_eq!(
            memory_text(&search, &serde_json::json!({"cards": []})),
            "No card matches.\n"
        );
        let add = MemoryCommand::Add { text: "x".into() };
        assert_eq!(
            memory_text(
                &add,
                &serde_json::json!({"outcomes": [
                    {"outcome": "outranked", "card": "m1"},
                    {"outcome": "already_added", "card": "m2"},
                ]})
            ),
            "Not given: m1 rests on the operator's own words and stays in use; this text is kept in its history.\nAlready added as m2.\n"
        );
        assert_eq!(
            memory_text(
                &MemoryCommand::Forget { number: 5 },
                &serde_json::json!({"card": "m5", "outcome": "forgotten"})
            ),
            "Forgot m5. Undo: hide memory undo m5\n"
        );
        assert_eq!(
            memory_text(
                &MemoryCommand::Undo { number: 5 },
                &serde_json::json!({"card": "m5", "outcome": "removed"})
            ),
            "Removed m5.\n"
        );
        assert_eq!(
            memory_text(
                &MemoryCommand::Stale {
                    number: 5,
                    reason: "x".into()
                },
                &serde_json::json!({"card": "m5", "outcome": "held", "expires_at": "2026-10-24 09:00 UTC"})
            ),
            "Held m5: Hide stops giving it until this conversation is analyzed, at the latest 2026-10-24 09:00 UTC. Release: hide memory release m5\n"
        );
        let released = |caller: &str, report: &str| {
            memory_text(
                &MemoryCommand::Release { number: 5 },
                &serde_json::json!({"card": "m5", "outcome": "released", "caller": caller, "report": report}),
            )
        };
        assert_eq!(
            released("person", "given_back"),
            "Released m5; Hide gives it again, and it may be reported stale once more.\n"
        );
        assert_eq!(
            released("agent", "spent"),
            "Released m5; Hide gives it again. An agent released it, so this text is not reported stale again.\n"
        );
        assert!(
            released("checkout", "spent")
                .contains("a release run in a Hide shell pane with no agent gives the report back"),
            "a person at an outside terminal learns where a release gives the report back"
        );
        let shown = memory_text(
            &MemoryCommand::Show { number: 5 },
            &serde_json::json!({
                "card": "m5", "kind": "rule", "state": "active", "sources": 1,
                "operator_sources": 1, "title": "Commits", "body": "Write commits in Korean",
                "hold": {"reason": "English now", "reported_at": "2026-10-17 09:00 UTC", "expires_at": "2026-10-24 09:00 UTC"},
            }),
        );
        assert!(
            shown.contains("Held: an agent reported it stale at 2026-10-17 09:00 UTC"),
            "{shown}"
        );
        assert!(
            shown.contains(
                "Reported because (the reporting agent's words, not checked): \"English now\""
            ),
            "{shown}"
        );
    }

    #[test]
    fn links_reads_one_target_and_has_no_write_command() {
        use herdr_core::links::query::{LinksQuery, QueryTarget};
        let parse = |line: &[&str]| {
            parse_args(&line.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
        };
        assert_eq!(
            parse(&["hide", "links", "pr", "#317"]),
            Ok(CommandKind::Links(LinksQuery {
                target: QueryTarget::Pr { number: 317 },
                all_projects: false,
            }))
        );
        assert_eq!(
            parse(&["hide", "links", "--all-projects", "branch", "feat/x"]),
            Ok(CommandKind::Links(LinksQuery {
                target: QueryTarget::Branch {
                    name: "feat/x".into()
                },
                all_projects: true,
            }))
        );
        assert_eq!(
            parse(&["hide", "links", "session", "a1b2"]),
            Ok(CommandKind::Links(LinksQuery {
                target: QueryTarget::Session { id: "a1b2".into() },
                all_projects: false,
            }))
        );
        for bad in [
            &["hide", "links"][..],
            &["hide", "links", "pr", "0"],
            &["hide", "links", "pr", "abc"],
            &["hide", "links", "issue"],
            &["hide", "links", "add", "pr", "1"],
            &["hide", "links", "pr", "1", "--project", "other"],
            &[
                "hide",
                "links",
                "pr",
                "1",
                "--all-projects",
                "--all-projects",
            ],
        ] {
            assert_eq!(parse(bad), Err(LINKS_USAGE.to_owned()), "{bad:?}");
        }
    }

    #[test]
    fn view_commands_accept_one_view_and_no_workspace_override() {
        let parse = |line: &[&str]| {
            parse_args(&line.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
        };
        assert_eq!(
            parse(&[
                "hide",
                "view",
                "split",
                "d2",
                "--area",
                "a1",
                "--edge",
                "right",
                "--request-id",
                "1234567890000-abc"
            ]),
            Ok(CommandKind::WorkspaceAction {
                action: Action::Split {
                    view_id: "d2".into(),
                    area_id: "a1".into(),
                    edge: Edge::Right
                },
                request_id: Some("1234567890000-abc".into()),
            })
        );
        assert!(parse(&["hide", "view", "select", "d2", "--workspace", "other"]).is_err());
        assert!(parse(&["hide", "view", "move", "d2", "--area", "a1"]).is_err());
        assert!(
            parse(&[
                "hide", "view", "split", "d2", "--area", "a1", "--edge", "right", "--edge", "left"
            ])
            .is_err()
        );
    }

    #[test]
    fn file_and_diff_open_parse_scoped_paths_and_reject_target_overrides() {
        let parse = |line: &[&str]| {
            parse_args(&line.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
        };
        assert_eq!(
            parse(&["hide", "file", "open", "src/lib.rs", "--beside"]),
            Ok(CommandKind::WorkspaceAction {
                action: Action::OpenFile {
                    path: "src/lib.rs".into(),
                    beside: true,
                },
                request_id: None,
            })
        );
        assert_eq!(
            parse(&["hide", "diff", "open", "src/lib.rs"]),
            Ok(CommandKind::WorkspaceAction {
                action: Action::OpenDiff {
                    path: "src/lib.rs".into(),
                    beside: false,
                },
                request_id: None,
            })
        );
        assert!(parse(&["hide", "file", "open", "src/lib.rs", "--workspace", "other"]).is_err());
        assert!(parse(&["hide", "file", "open", "src/lib.rs", "--reveal"]).is_err());
        assert!(parse(&["hide", "diff", "open", "src/lib.rs", "--beside", "--beside"]).is_err());
    }

    #[test]
    fn parse_open_default() {
        let args = ["hide"].into_iter().map(String::from).collect::<Vec<_>>();
        assert_eq!(parse_args(&args).unwrap(), CommandKind::Open);
    }
}
