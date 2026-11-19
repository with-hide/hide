//! `hide automation …` (PRD automations B9, B10): an agent makes, reads and
//! changes the operator's automations when asked in words ("매일 아침 9시에
//! 이슈 정리해줘"), through the pane capability every agent command uses.
//!
//! The core owns the automations and answers each call
//! (`hide_core::automations::Call`); this file reads the words into a call,
//! and on the daemon's side (`answer`) words the core's answer for an agent:
//! what the schedule reads as, the next time as the core's machine's wall
//! clock, and, for a refusal or an automation stopped for a person, what to
//! tell the operator to do. The window's form opens the same calls by its own
//! events; nothing here is the form's.

use hide_core::automations::{Call, Clock, PauseReason, Schedule, Weekday};
use serde_json::{Value, json};

use crate::env::Env;

/// The guide `hide automation help` prints.
pub const HELP: &str = include_str!("../assets/automation/help.md");

pub const USAGE: &str = "hide automation help\nhide automation add --name <name> --task <text> --schedule <when> --kind <kind> [--profile <name>] [--model <model>] [--effort <effort>] [--folder <path> [--worktree]] [--run-now]\nhide automation list\nhide automation show <id|name>\nhide automation run <id|name>\nhide automation pause <id|name>\nhide automation resume <id|name>\nhide automation edit <id|name> [--name <name>] [--task <text>] [--schedule <when>] [--kind <kind> [--profile <name>] [--model <model>] [--effort <effort>]] [--folder <path> | --own-folder] [--worktree | --no-worktree]\nhide automation remove <id|name>\nhide automation preview --schedule <when>";

/// The forms `--schedule` takes, as a refusal and the guide name them.
const SCHEDULE_FORMS: &str = "Give --schedule as one of: \"daily 09:00\", \"weekdays 09:00\", \"weekly mon,thu 09:00\", \"every 15m\" (every 5, 10, 15 or 30 minutes from the hour), \"every 3h 09:00\" (every 1 to 12 hours from that time) or \"cron <minute> <hour> <day> <month> <weekday>\", such as \"cron 0 9 * * 1-5\"; times are HH:MM on a 24-hour clock in the zone of the machine that runs Hide";

/// The flags each verb reads, valued and switches: the contract's table
/// names the same (`cli_contract`, held by its test).
pub const FLAGS: &[(&str, &[&str], &[&str])] = &[
    (
        "add",
        &[
            "--name",
            "--task",
            "--schedule",
            "--kind",
            "--profile",
            "--model",
            "--effort",
            "--folder",
        ],
        &["--worktree", "--run-now"],
    ),
    ("list", &[], &[]),
    ("show", &[], &[]),
    ("run", &[], &[]),
    ("pause", &[], &[]),
    ("resume", &[], &[]),
    (
        "edit",
        &[
            "--name",
            "--task",
            "--schedule",
            "--kind",
            "--profile",
            "--model",
            "--effort",
            "--folder",
        ],
        &["--own-folder", "--worktree", "--no-worktree"],
    ),
    ("remove", &[], &[]),
    ("preview", &["--schedule"], &[]),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutomationCli {
    Help,
    Command(Box<Request>),
}

/// The agent, model and effort a run starts with, named as `hide agent
/// spawn` names them (D-07).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Agent {
    pub kind: String,
    pub profile: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// A parsed command. The schedule and the folder are read when it runs, so
/// what is not one answers its reason on the command's JSON line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    Add {
        name: String,
        task: String,
        schedule: String,
        agent: Agent,
        folder: Option<String>,
        worktree: bool,
        run_now: bool,
    },
    Edit {
        id: String,
        name: Option<String>,
        task: Option<String>,
        schedule: Option<String>,
        agent: Option<Agent>,
        /// `Some(None)`: back to its own folder (`--own-folder`).
        folder: Option<Option<String>>,
        worktree: Option<bool>,
    },
    List,
    Show {
        id: String,
    },
    Run {
        id: String,
    },
    Pause {
        id: String,
    },
    Resume {
        id: String,
    },
    Remove {
        id: String,
    },
    Preview {
        schedule: String,
    },
}

/// Text as the contract's `text` admits it.
fn text(value: &str) -> bool {
    !value.is_empty() && !value.starts_with("--") && !value.chars().any(char::is_control)
}

/// An id or name as the contract's `key` admits it.
fn key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('-')
        && !value.chars().any(char::is_control)
}

/// A task: not blank, and it may run over lines. One longer than the
/// contract's `body` admits is refused when the command runs, with the
/// core's own reason (`automation_task_too_long`).
fn body(value: &str) -> bool {
    !value.trim().is_empty()
}

/// Parses the words after `hide automation`.
pub fn parse(args: &[String]) -> Result<AutomationCli, String> {
    let (verb, rest) = args.split_first().ok_or(USAGE)?;
    if matches!(verb.as_str(), "help" | "--help") {
        return if rest.is_empty() {
            Ok(AutomationCli::Help)
        } else {
            Err(USAGE.to_owned())
        };
    }
    let (_, valued, switches) = FLAGS.iter().find(|(name, ..)| *name == verb).ok_or(USAGE)?;
    let mut values = std::collections::BTreeMap::<&str, String>::new();
    let mut on = std::collections::BTreeSet::<&str>::new();
    let mut positional = Vec::new();
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let word = word.as_str();
        if let Some(flag) = valued.iter().find(|flag| **flag == word) {
            let value = words
                .next()
                .ok_or_else(|| format!("{flag} needs a value"))?;
            let admitted = match *flag {
                "--task" => body(value),
                _ => text(value),
            };
            if !admitted {
                return Err(format!("{flag} needs a value"));
            }
            if values.insert(flag, value.clone()).is_some() {
                return Err(format!("{flag} is given twice"));
            }
        } else if let Some(flag) = switches.iter().find(|flag| **flag == word) {
            if !on.insert(flag) {
                return Err(format!("{flag} is given twice"));
            }
        } else if word.starts_with('-') {
            return Err(format!("unknown flag for hide automation {verb}: {word}"));
        } else {
            positional.push(word.to_owned());
        }
    }
    let mut take = |flag: &str| values.remove(flag);
    let agent = |kind: Option<String>,
                 profile: Option<String>,
                 model: Option<String>,
                 effort: Option<String>|
     -> Result<Option<Agent>, String> {
        match kind {
            Some(kind) if key(&kind) => Ok(Some(Agent {
                kind,
                profile,
                model,
                effort,
            })),
            Some(_) => Err("--kind needs an agent kind, such as claude or codex".to_owned()),
            None if profile.is_some() || model.is_some() || effort.is_some() => {
                Err("--profile, --model and --effort go with --kind".to_owned())
            }
            None => Ok(None),
        }
    };
    let id = |positional: &[String]| match positional {
        [id] if key(id) => Ok(id.clone()),
        _ => Err(format!(
            "hide automation {verb} takes one automation id or name"
        )),
    };
    let request = match verb.as_str() {
        "add" => {
            if !positional.is_empty() {
                return Err(USAGE.to_owned());
            }
            let (Some(name), Some(task), Some(schedule)) =
                (take("--name"), take("--task"), take("--schedule"))
            else {
                return Err("hide automation add needs --name, --task and --schedule".to_owned());
            };
            let Some(agent) = agent(
                take("--kind"),
                take("--profile"),
                take("--model"),
                take("--effort"),
            )?
            else {
                return Err(
                    "hide automation add needs --kind, the agent each run starts".to_owned(),
                );
            };
            let folder = take("--folder");
            let worktree = on.contains("--worktree");
            if worktree && folder.is_none() {
                return Err("--worktree goes with --folder naming a Git project".to_owned());
            }
            Request::Add {
                name,
                task,
                schedule,
                agent,
                folder,
                worktree,
                run_now: on.contains("--run-now"),
            }
        }
        "edit" => {
            let id = id(&positional)?;
            let agent = agent(
                take("--kind"),
                take("--profile"),
                take("--model"),
                take("--effort"),
            )?;
            let folder = match (take("--folder"), on.contains("--own-folder")) {
                (Some(_), true) => return Err("give --folder or --own-folder, not both".to_owned()),
                (Some(path), false) => Some(Some(path)),
                (None, true) => Some(None),
                (None, false) => None,
            };
            let worktree = match (on.contains("--worktree"), on.contains("--no-worktree")) {
                (true, true) => return Err("give --worktree or --no-worktree, not both".to_owned()),
                (true, false) => Some(true),
                (false, true) => Some(false),
                (false, false) => None,
            };
            let (name, task, schedule) = (take("--name"), take("--task"), take("--schedule"));
            if name.is_none()
                && task.is_none()
                && schedule.is_none()
                && agent.is_none()
                && folder.is_none()
                && worktree.is_none()
            {
                return Err("hide automation edit needs at least one change".to_owned());
            }
            Request::Edit {
                id,
                name,
                task,
                schedule,
                agent,
                folder,
                worktree,
            }
        }
        "preview" => {
            if !positional.is_empty() {
                return Err(USAGE.to_owned());
            }
            Request::Preview {
                schedule: take("--schedule").ok_or("hide automation preview needs --schedule")?,
            }
        }
        "list" if positional.is_empty() => Request::List,
        "show" => Request::Show {
            id: id(&positional)?,
        },
        "run" => Request::Run {
            id: id(&positional)?,
        },
        "pause" => Request::Pause {
            id: id(&positional)?,
        },
        "resume" => Request::Resume {
            id: id(&positional)?,
        },
        "remove" => Request::Remove {
            id: id(&positional)?,
        },
        _ => return Err(USAGE.to_owned()),
    };
    Ok(AutomationCli::Command(Box::new(request)))
}

/// Reads a `--schedule` value into the core's schedule (B6). The words are
/// read here; whether the schedule may run (five minutes apart at least, a
/// cron line that names a real time, a period the form offers) is the
/// core's check, the one the form makes too.
pub fn read_schedule(text: &str) -> Result<Schedule, &'static str> {
    let text = text.trim();
    let (form, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let rest = rest.trim();
    let time = |word: &str| Clock::parse(word);
    match form.to_ascii_lowercase().as_str() {
        "cron" if !rest.is_empty() => Ok(Schedule::Cron {
            expr: rest.to_owned(),
        }),
        "daily" => Ok(Schedule::Daily { time: time(rest)? }),
        "weekdays" => Ok(Schedule::Weekdays { time: time(rest)? }),
        "weekly" => {
            // "weekly 09:00" names no day: the core answers why.
            let (days, at) = match rest.rsplit_once(char::is_whitespace) {
                Some((days, at)) => (days.trim(), at),
                None => ("", rest),
            };
            let days = days
                .split(',')
                .map(str::trim)
                .filter(|day| !day.is_empty())
                .map(|day| {
                    serde_json::from_value::<Weekday>(json!(day.to_ascii_lowercase()))
                        .map_err(|_| "schedule_unreadable")
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Schedule::Weekly {
                days,
                time: time(at)?,
            })
        }
        "every" => {
            // "every 15m" counts from each hour's start and takes no time;
            // a count the form does not offer is the core's to refuse, and
            // one too large to hold is refused the same way here.
            if let Some(minutes) = rest
                .strip_suffix('m')
                .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            {
                let minutes = minutes
                    .parse::<u8>()
                    .map_err(|_| "schedule_minutes_out_of_range")?;
                return Ok(Schedule::EveryMinutes { minutes });
            }
            let (hours, at) = rest
                .split_once(char::is_whitespace)
                .ok_or("schedule_unreadable")?;
            let hours = hours
                .strip_suffix('h')
                .and_then(|hours| hours.parse::<u8>().ok())
                .ok_or("schedule_unreadable")?;
            Ok(Schedule::EveryHours {
                hours,
                time: time(at.trim())?,
            })
        }
        _ => Err("schedule_unreadable"),
    }
}

/// What the agent tells the operator about a refusal or an automation that
/// stopped itself for a person (B10, B23): one sentence a person can act on.
pub fn next_action(code: &str) -> &'static str {
    match code {
        "schedule_unreadable" => SCHEDULE_FORMS,
        "schedule_too_often" => {
            "An automation runs at most every five minutes: give a cron line whose times are five minutes apart or more, such as \"cron */5 * * * *\", or \"every 5m\""
        }
        "schedule_minutes_out_of_range" => {
            "Every N minutes takes 5, 10, 15 or 30 minutes, such as \"every 15m\"; for longer use \"every 1h 00:00\", and for another period a cron line"
        }
        "schedule_cron_invalid" => {
            "Give five cron fields, minute hour day month weekday, with numbers, *, ranges, lists and steps, such as \"cron 0 9 * * 1-5\" for weekdays at 09:00"
        }
        "schedule_never" => {
            "That cron line names no time that comes within five years, such as February 30th; give a date that exists"
        }
        "schedule_time_invalid" => "Give the time as HH:MM on a 24-hour clock, such as 09:00",
        "schedule_days_missing" => {
            "Name at least one day for a weekly schedule, such as \"weekly mon,thu 09:00\""
        }
        "schedule_hours_out_of_range" => {
            "Every N hours takes 1 to 12 hours, such as \"every 3h 09:00\"; for once a day use \"daily 09:00\""
        }
        "automation_unknown" => {
            "No automation has that id or name; run hide automation list and name one by its id or exact name"
        }
        "automation_name_missing" => "Give the automation a name with --name",
        "automation_name_invalid" => {
            "Give a name of at most 80 characters with no line breaks or control characters"
        }
        "automation_name_taken" => {
            "Another automation has that name; choose another, or change that one with hide automation edit"
        }
        "automation_task_missing" => "Give the task each run is handed with --task",
        "automation_task_too_long" => {
            "The task is longer than 16 KiB; shorten it, or have it point at a file in the automation's folder"
        }
        "automation_agent_unknown" => {
            "Name an agent Hide starts with --kind (hide agent catalog lists them), and a model, effort and profile as hide agent spawn takes them"
        }
        "automation_folder_on_device" => {
            "An automation runs on the machine that runs Hide, not on this device, so it cannot work in this device's folder; leave out --folder to work in the automation's own folder there"
        }
        "automation_folder_unknown" => {
            "--folder must name a project's root folder as Hide has it open on the machine that runs Hide, not a folder inside one; open the folder in Hide first (hide <path>), or leave --folder out to work in the automation's own folder"
        }
        "automation_worktree_needs_git" => {
            "A new worktree per run needs a Git project; name one with --folder, or leave out --worktree"
        }
        "automation_cap" => {
            "Hide keeps 100 automations at most; remove one the operator no longer needs with hide automation remove, then add this one"
        }
        "automation_home_unknown" => {
            "Hide could not find this machine's home folder for the automation's own folder; give --folder with a project folder, and the reason is in Hide's diagnostic log"
        }
        "automation_running" => {
            "A run of this automation is going now; wait until it ends (hide automation show <id>) before running it again"
        }
        "automation_changed_unread" => {
            "The change was made, but Hide could not read the automation back; run hide automation list to see where it stands before changing it again"
        }
        "automations_unavailable" | "automations_store" => {
            "Hide could not open its automations; run the same command again, and if it lasts, the reason is in Hide's diagnostic log"
        }
        "automation_paused" => {
            "The operator paused it; it runs again from its next time after hide automation resume <id>"
        }
        "automation_login_expired" => {
            "The agent's sign-in on this machine ended: ask the operator to sign in to that agent's CLI again, then run hide automation resume <id>"
        }
        "automation_folder_missing" => {
            "The automation's folder is gone: make the folder again, or point it at another with hide automation edit <id> --folder <path>, then run hide automation resume <id>"
        }
        "automation_agent_missing" => {
            "The agent is not installed or not set up for Hide on this machine: ask the operator to install it or allow Hide's hooks for it, or choose another with hide automation edit <id> --kind <kind>, then run hide automation resume <id>"
        }
        "automation_folder_detached" => {
            "The project's folder has no branch out: check a branch out there, or give each run its own worktree with hide automation edit <id> --worktree, then run hide automation resume <id>"
        }
        "automation_machine_changed" => {
            "It was made on another machine: run hide automation resume <id> to run it on this one from its next time"
        }
        "automation_folder_in_repository" => {
            "Its own folder lies inside a Git repository: give it a project folder with hide automation edit <id> --folder <path>, then run hide automation resume <id>"
        }
        other => crate::cli::request_next_action(other),
    }
}

/// The folder `raw` names, as the core spells a project's path.
fn wire_path(raw: &str) -> Result<String, &'static str> {
    crate::cli::wire_folder(raw).map_err(|_| "automation_folder_unknown")
}

/// A task the core would refuse for its length, refused before it is sent
/// with the core's own reason.
fn checked_task(task: &str) -> Result<&str, &'static str> {
    if task.trim().len() > hide_core::delivery::BODY_LIMIT {
        return Err("automation_task_too_long");
    }
    Ok(task)
}

fn agent_json(agent: &Agent) -> Value {
    json!({
        "kind": agent.kind,
        "profile": agent.profile,
        "model": agent.model,
        "effort": agent.effort,
    })
}

/// The core's call for a parsed command, or the reason it is not one.
fn call_json(request: &Request) -> Result<Value, &'static str> {
    let schedule = |text: &str| read_schedule(text).map(|schedule| json!(schedule));
    let folder = |path: &Option<String>| path.as_deref().map(wire_path).transpose();
    Ok(match request {
        Request::Add {
            name,
            task,
            schedule: when,
            agent,
            folder: path,
            worktree,
            run_now,
        } => json!({
            "call": "add",
            "name": name,
            "task": checked_task(task)?,
            "schedule": schedule(when)?,
            "agent": agent_json(agent),
            "folder": folder(path)?,
            "worktree": worktree,
            "run_now": run_now,
        }),
        Request::Edit {
            id,
            name,
            task,
            schedule: when,
            agent,
            folder: path,
            worktree,
        } => {
            let mut call = json!({"call": "edit", "id": id});
            if let Some(name) = name {
                call["name"] = json!(name);
            }
            if let Some(task) = task {
                call["task"] = json!(checked_task(task)?);
            }
            if let Some(when) = when {
                call["schedule"] = schedule(when)?;
            }
            if let Some(agent) = agent {
                call["agent"] = agent_json(agent);
            }
            // `null` moves it back to its own folder.
            if let Some(path) = path {
                call["folder"] = json!(folder(path)?);
            }
            if let Some(worktree) = worktree {
                call["worktree"] = json!(worktree);
            }
            call
        }
        Request::List => json!({"call": "list"}),
        Request::Show { id } => json!({"call": "show", "id": id}),
        Request::Run { id } => json!({"call": "run", "id": id}),
        Request::Pause { id } => json!({"call": "pause", "id": id}),
        Request::Resume { id } => json!({"call": "resume", "id": id}),
        Request::Remove { id } => json!({"call": "remove", "id": id}),
        Request::Preview { schedule: when } => {
            json!({"call": "preview", "schedule": schedule(when)?})
        }
    })
}

fn refused(code: &str, message: &str) -> Result<(), String> {
    println!("{}", crate::cli::refusal_line(code, message, &Value::Null));
    Err(code.to_owned())
}

/// Runs a parsed command and prints one line, `{"ok":true,"value":…}` or
/// `{"ok":false,"error":{"code","message"}}` with what to do, and a non-zero
/// exit for a refusal.
pub fn run(env: &Env, cli: AutomationCli) -> Result<(), String> {
    let request = match cli {
        AutomationCli::Help => {
            print!("{HELP}");
            return Ok(());
        }
        AutomationCli::Command(request) => *request,
    };
    let call = match call_json(&request) {
        Ok(call) => call,
        Err(code) => return refused(code, next_action(code)),
    };
    let reads = matches!(
        request,
        Request::List | Request::Show { .. } | Request::Preview { .. }
    );
    let answer = crate::workspace_cli::Credential::acquire(env).and_then(|mut credential| {
        crate::workspace_cli::request_automation(&mut credential, &call, !reads)
    });
    match answer {
        Ok(answer) if answer["ok"] == true => {
            println!("{}", json!({"ok": true, "value": answer["result"]}));
            Ok(())
        }
        Ok(answer) => {
            let code = answer["reason"]
                .as_str()
                .unwrap_or("automations_unavailable")
                .to_owned();
            let message = answer["next_action"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| next_action(&code).to_owned());
            refused(&code, &message)
        }
        Err(code) => {
            // An answer that did not arrive may still have been applied.
            let message = if code == "request_timeout" && !reads {
                "The change may have been applied; run hide automation list before running it again"
            } else {
                crate::cli::request_next_action(&code)
            };
            refused(&code, message)
        }
    }
}

/// The calls a pane may make: the commands above. A window's own events
/// (a run's stop, its conversation opened again, what the screen shows,
/// what the operator has seen) are not an agent's.
pub fn from_wire(call: &Value) -> Option<Call> {
    let call = serde_json::from_value::<Call>(call.clone()).ok()?;
    matches!(
        call,
        Call::Add(_)
            | Call::Edit { .. }
            | Call::Pause { .. }
            | Call::Resume { .. }
            | Call::Run { .. }
            | Call::Remove { .. }
            | Call::List
            | Call::Show { .. }
            | Call::Preview { .. }
    )
    .then_some(call)
}

/// Whether the call only reads, which an external tool's View allows.
pub fn reads(call: &Call) -> bool {
    matches!(call, Call::List | Call::Show { .. } | Call::Preview { .. })
}

/// The wall clock of the machine whose core reads the schedules: the
/// daemon runs there, and the core reads the same system zone.
struct Clockface {
    zone: jiff::tz::TimeZone,
}

impl Clockface {
    fn here() -> Self {
        Self {
            zone: jiff::tz::TimeZone::system(),
        }
    }

    fn name(&self) -> Option<&str> {
        self.zone.iana_name()
    }

    /// `2026-11-20 09:00` for a time in unix milliseconds, null for none.
    fn at(&self, unix_ms: &Value) -> Value {
        unix_ms
            .as_u64()
            .map(|ms| json!(hide_core::automations::local_minute(ms, &self.zone)))
            .unwrap_or(Value::Null)
    }
}

/// A schedule in plain words: `weekdays at 09:00`.
fn schedule_words(schedule: &Value) -> Option<String> {
    let schedule = serde_json::from_value::<Schedule>(schedule.clone()).ok()?;
    let clock = |time: &Clock| format!("{:02}:{:02}", time.hour, time.minute);
    Some(match &schedule {
        Schedule::Daily { time } => format!("every day at {}", clock(time)),
        Schedule::Weekdays { time } => format!("weekdays at {}", clock(time)),
        Schedule::Weekly { days, time } => {
            let days: Vec<String> = days
                .iter()
                .filter_map(|day| serde_json::to_value(day).ok())
                .filter_map(|day| day.as_str().map(str::to_owned))
                .collect();
            format!("every {} at {}", days.join(", "), clock(time))
        }
        Schedule::EveryMinutes { minutes } => format!("every {minutes} minutes"),
        Schedule::EveryHours { hours, time } => {
            format!("every {hours} hours from {}", clock(time))
        }
        Schedule::Cron { expr } => format!("cron {expr}"),
    })
}

/// The pause's code, as the core names the reason.
fn pause_code(paused: &Value) -> Option<&'static str> {
    serde_json::from_value::<PauseReason>(paused["reason"].clone())
        .ok()
        .map(PauseReason::code)
}

/// A record with its time on the wall clock beside its milliseconds.
fn record(clock: &Clockface, mut record: Value) -> Value {
    for (field, local) in [
        ("started_at_unix_ms", "started_at"),
        ("ended_at_unix_ms", "ended_at"),
        ("at_unix_ms", "at"),
        ("first_unix_ms", "first"),
        ("last_unix_ms", "last"),
    ] {
        if record.get(field).is_some() {
            record[local] = clock.at(&record[field]);
        }
    }
    record
}

/// An automation as the core's section has it, with what an agent tells the
/// operator: what its schedule reads as, its next time on the machine's wall
/// clock, and, stopped for a person, what to fix (B23).
fn automation(clock: &Clockface, mut item: Value, records: bool) -> Value {
    item["reads_as_text"] = json!(schedule_words(&item["reads_as"]));
    item["next_run_at"] = clock.at(&item["next_run_at_unix_ms"]);
    item["zone"] = json!(clock.name());
    if let Some(code) = item.get("paused").and_then(pause_code) {
        item["fix"] = json!(next_action(code));
    }
    let entries = item["records"].as_array().cloned().unwrap_or_default();
    if records {
        item["records"] = json!(
            entries
                .into_iter()
                .map(|entry| record(clock, entry))
                .collect::<Vec<_>>()
        );
    } else {
        // A list names each automation's newest record only; `show` gives
        // them all.
        item.as_object_mut().map(|item| item.remove("records"));
        item["last_record"] = entries
            .into_iter()
            .next()
            .map(|entry| record(clock, entry))
            .unwrap_or(Value::Null);
    }
    item
}

type Refusal = (String, &'static str);

fn refusal(code: String) -> Refusal {
    let next = next_action(&code);
    (code, next)
}

/// What an answer to a pane of a connected device adds: the automation
/// runs where Hide's core runs, not on the device (D-21).
pub const ON_CORE_MACHINE: &str = "This automation runs on the machine that runs Hide, not on this device: its folder and its runs are there.";

/// A device's pane names a folder of the device, which the core's machine
/// does not have: an automation runs on the core's machine (D-21), so a
/// device's folder is refused rather than read there.
fn device_folder(call: &Call) -> bool {
    match call {
        Call::Add(draft) => draft.folder.is_some(),
        Call::Edit { changes, .. } => matches!(changes.folder, Some(Some(_))),
        _ => false,
    }
}

/// The daemon's answer to a pane's call: the core's own, then the
/// automation as it stands after a change, worded for an agent (B9, B10).
/// `from_device` is a pane of a connected device rather than of the core's
/// machine.
pub fn answer(
    core: &crate::core::CoreHandle,
    call: Call,
    from_device: bool,
) -> Result<Value, Refusal> {
    if from_device && device_folder(&call) {
        return Err(refusal("automation_folder_on_device".to_owned()));
    }
    let clock = Clockface::here();
    let show = |id: &str| {
        core.automation(Call::Show { id: id.to_owned() })
            .map(|item| automation(&clock, item, true))
            .map_err(refusal)
    };
    match call {
        Call::List => {
            let listed = core.automation(Call::List).map_err(refusal)?;
            let items: Vec<Value> = listed["items"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|item| automation(&clock, item, false))
                .collect();
            Ok(json!({"items": items, "zone": clock.name()}))
        }
        Call::Show { id } => Ok(json!({"automation": show(&id)?})),
        Call::Preview { schedule, name } => {
            let mut preview = core
                .automation(Call::Preview { schedule, name })
                .map_err(refusal)?;
            preview["reads_as_text"] = json!(schedule_words(&preview["reads_as"]));
            preview["next_run_at"] = clock.at(&preview["next_run_at_unix_ms"]);
            Ok(preview)
        }
        Call::Remove { id } => {
            // Named before it goes, so the answer says which one went.
            let name = show(&id)?["name"].clone();
            let removed = core.automation(Call::Remove { id }).map_err(refusal)?;
            Ok(json!({"removed": {
                "id": removed["id"],
                "name": name,
                "records": removed["records"],
            }}))
        }
        change => {
            let changed = core.automation(change).map_err(refusal)?;
            let Some(id) = changed["id"].as_str() else {
                return Err(refusal("automations_unavailable".to_owned()));
            };
            // Read back as the section has it. The change is made either
            // way; a read back that fails says so rather than answer half.
            let read_back =
                show(id).map_err(|_| refusal("automation_changed_unread".to_owned()))?;
            let mut answer = json!({"automation": read_back});
            if from_device {
                answer["note"] = json!(ON_CORE_MACHINE);
            }
            if let Some(run) = changed.get("run") {
                answer["run"] = run.clone();
            }
            // A run asked for that could not start for a person's reason:
            // the automation stopped, and what to fix is the answer's.
            if let Some(code) = changed["paused"].as_str() {
                answer["paused"] = json!({"code": code, "fix": next_action(code)});
            }
            Ok(answer)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &[&str]) -> Vec<String> {
        line.iter().map(|word| (*word).to_owned()).collect()
    }

    fn request(line: &[&str]) -> Request {
        match parse(&words(line)).unwrap() {
            AutomationCli::Command(request) => *request,
            AutomationCli::Help => panic!("help"),
        }
    }

    #[test]
    fn each_schedule_form_reads_as_the_core_choice() {
        let nine = Clock { hour: 9, minute: 0 };
        assert_eq!(
            read_schedule("daily 09:00"),
            Ok(Schedule::Daily { time: nine })
        );
        assert_eq!(
            read_schedule("Weekdays 09:00"),
            Ok(Schedule::Weekdays { time: nine })
        );
        assert_eq!(
            read_schedule("weekly mon, Thu 09:00"),
            Ok(Schedule::Weekly {
                days: vec![Weekday::Mon, Weekday::Thu],
                time: nine
            })
        );
        assert_eq!(
            read_schedule("every 3h 09:00"),
            Ok(Schedule::EveryHours {
                hours: 3,
                time: nine
            })
        );
        assert_eq!(
            read_schedule("every 15m"),
            Ok(Schedule::EveryMinutes { minutes: 15 })
        );
        assert_eq!(
            read_schedule("cron 0 9 * * 1-5"),
            Ok(Schedule::Cron {
                expr: "0 9 * * 1-5".into()
            })
        );
        // What the words cannot say is refused here; what may not run is
        // the core's to refuse, with its own reason.
        assert_eq!(read_schedule("daily 9am"), Err("schedule_time_invalid"));
        assert_eq!(read_schedule("hourly"), Err("schedule_unreadable"));
        assert_eq!(
            read_schedule("weekly someday 09:00"),
            Err("schedule_unreadable")
        );
        assert_eq!(read_schedule("every 3 09:00"), Err("schedule_unreadable"));
        // Every N minutes counts from the hour and takes no time.
        assert_eq!(read_schedule("every 15m 09:00"), Err("schedule_unreadable"));
        assert_eq!(read_schedule("every 1 5m"), Err("schedule_unreadable"));
        assert_eq!(read_schedule("every m"), Err("schedule_unreadable"));
        assert_eq!(
            read_schedule("every 900m"),
            Err("schedule_minutes_out_of_range")
        );
        assert_eq!(read_schedule("cron"), Err("schedule_unreadable"));
        assert_eq!(
            read_schedule("weekly 09:00"),
            Ok(Schedule::Weekly {
                days: vec![],
                time: nine
            })
        );
        assert_eq!(
            read_schedule("every 30h 09:00"),
            Ok(Schedule::EveryHours {
                hours: 30,
                time: nine
            })
        );
    }

    #[test]
    fn add_takes_the_agent_as_spawn_names_it() {
        assert_eq!(
            request(&[
                "add",
                "--name",
                "이슈 정리",
                "--task",
                "새 이슈를 정리해\n라벨도 붙여",
                "--schedule",
                "daily 09:00",
                "--kind",
                "claude",
                "--model",
                "opus",
                "--effort",
                "high",
            ]),
            Request::Add {
                name: "이슈 정리".into(),
                task: "새 이슈를 정리해\n라벨도 붙여".into(),
                schedule: "daily 09:00".into(),
                agent: Agent {
                    kind: "claude".into(),
                    profile: None,
                    model: Some("opus".into()),
                    effort: Some("high".into()),
                },
                folder: None,
                worktree: false,
                run_now: false,
            }
        );
        let base = [
            "add",
            "--name",
            "n",
            "--task",
            "t",
            "--schedule",
            "daily 09:00",
        ];
        assert!(parse(&words(&base)).is_err(), "no --kind");
        let mut worktree = base.to_vec();
        worktree.extend(["--kind", "codex", "--worktree"]);
        assert!(parse(&words(&worktree)).is_err(), "--worktree alone");
        let mut model_only = base.to_vec();
        model_only.extend(["--model", "opus"]);
        assert!(
            parse(&words(&model_only)).is_err(),
            "--model without --kind"
        );
    }

    #[test]
    fn edit_names_what_changes_and_nothing_else() {
        assert_eq!(
            request(&["edit", "k3x9a2", "--own-folder", "--no-worktree"]),
            Request::Edit {
                id: "k3x9a2".into(),
                name: None,
                task: None,
                schedule: None,
                agent: None,
                folder: Some(None),
                worktree: Some(false),
            }
        );
        assert!(parse(&words(&["edit", "k3x9a2"])).is_err());
        assert!(parse(&words(&["edit", "a", "--folder", "/p", "--own-folder"])).is_err());
        assert!(parse(&words(&["edit", "a", "--worktree", "--no-worktree"])).is_err());
        assert!(parse(&words(&["show"])).is_err());
        assert!(parse(&words(&["show", "a", "b"])).is_err());
        assert!(parse(&words(&["run", "-a"])).is_err());
        assert!(parse(&words(&["list", "a"])).is_err());
        assert!(parse(&words(&["pause", "a", "--now"])).is_err());
    }

    #[test]
    fn an_edit_sends_only_its_changes_and_null_for_its_own_folder() {
        let call = call_json(&request(&["edit", "a", "--own-folder"])).unwrap();
        assert_eq!(call, json!({"call": "edit", "id": "a", "folder": null}));
        let parsed = from_wire(&call).unwrap();
        let Call::Edit { changes, .. } = parsed else {
            panic!("an edit")
        };
        assert_eq!(changes.folder, Some(None));
        assert!(changes.name.is_none() && changes.schedule.is_none());
    }

    #[test]
    fn every_command_sends_a_call_the_core_reads() {
        for line in [
            &[
                "add",
                "--name",
                "n",
                "--task",
                "t",
                "--schedule",
                "weekly mon 09:00",
                "--kind",
                "claude",
                "--run-now",
            ][..],
            &[
                "edit",
                "n",
                "--schedule",
                "cron 0 9 * * 1-5",
                "--kind",
                "codex",
                "--effort",
                "high",
            ],
            &["list"],
            &["show", "n"],
            &["run", "n"],
            &["pause", "n"],
            &["resume", "n"],
            &["remove", "n"],
            &["preview", "--schedule", "every 2h 08:30"],
        ] {
            let call = call_json(&request(line)).unwrap();
            assert!(from_wire(&call).is_some(), "{line:?}: {call}");
        }
        // A window's own events are not a pane's.
        for event in [
            json!({"call": "seen", "id": "n"}),
            json!({"call": "run_stop", "id": "n", "run": 1}),
            json!({"call": "run_open", "id": "n", "run": 1}),
            json!({"call": "run_viewing", "id": "n", "run": 1, "on": true}),
        ] {
            assert!(from_wire(&event).is_none(), "{event}");
        }
    }

    #[test]
    fn a_schedule_that_cannot_be_read_says_how_to_write_one() {
        let refused = call_json(&request(&["preview", "--schedule", "every morning"]));
        assert_eq!(refused, Err("schedule_unreadable"));
        assert!(next_action("schedule_unreadable").contains("\"cron 0 9 * * 1-5\""));
        // Every reason the core gives for a schedule has its own sentence.
        for code in [
            "schedule_too_often",
            "schedule_minutes_out_of_range",
            "schedule_cron_invalid",
            "schedule_never",
            "schedule_time_invalid",
            "schedule_days_missing",
            "schedule_hours_out_of_range",
        ] {
            assert_ne!(
                next_action(code),
                crate::cli::request_next_action(code),
                "{code}"
            );
        }
    }

    #[test]
    fn the_schedule_reads_in_plain_words() {
        assert_eq!(
            schedule_words(&json!({"kind": "weekly", "days": ["mon", "thu"], "time": "09:00"})),
            Some("every mon, thu at 09:00".into())
        );
        assert_eq!(
            schedule_words(&json!({"kind": "every_hours", "hours": 3, "time": "09:30"})),
            Some("every 3 hours from 09:30".into())
        );
        assert_eq!(
            pause_code(&json!({"reason": "login_expired"})),
            Some("automation_login_expired")
        );
        assert_eq!(pause_code(&json!({"reason": "someday"})), None);
    }

    /// Every reason an automation stops for, and every refusal of an
    /// automation as written, has its own sentence, not the fallback for a
    /// daemon that could not be reached.
    #[test]
    fn every_stop_and_refusal_has_its_own_sentence() {
        use PauseReason::*;
        let reasons = [
            Operator,
            LoginExpired,
            FolderMissing,
            AgentMissing,
            FolderDetached,
            MachineChanged,
            FolderInRepository,
        ];
        // A reason the core adds fails to compile here until it is listed.
        for reason in reasons {
            match reason {
                Operator | LoginExpired | FolderMissing | AgentMissing | FolderDetached
                | MachineChanged | FolderInRepository => {}
            }
        }
        let codes = reasons.iter().map(|reason| reason.code()).chain(
            crate::cli_contract::AUTOMATION_DRAFT_REFUSALS
                .iter()
                .copied(),
        );
        for code in codes {
            assert_ne!(
                next_action(code),
                crate::cli::request_next_action(code),
                "{code} has no sentence of its own"
            );
        }
    }

    /// From a device's pane, a folder is the device's, so it is refused;
    /// its own folder, or a move back to it, is not (D-21).
    #[test]
    fn a_device_pane_names_no_folder() {
        let call = |line: &[&str]| from_wire(&call_json(&request(line)).unwrap()).unwrap();
        let add = [
            "add",
            "--name",
            "n",
            "--task",
            "t",
            "--schedule",
            "daily 09:00",
            "--kind",
            "claude",
        ];
        assert!(!device_folder(&call(&add)));
        let mut with_folder = add.to_vec();
        with_folder.extend(["--folder", "/"]);
        assert!(device_folder(&call(&with_folder)));
        assert!(device_folder(&call(&["edit", "n", "--folder", "/"])));
        assert!(!device_folder(&call(&["edit", "n", "--own-folder"])));
        assert!(!device_folder(&call(&["edit", "n", "--name", "m"])));
    }

    #[test]
    fn a_task_past_the_cap_is_refused_with_the_core_reason() {
        let long = "x".repeat(hide_core::delivery::BODY_LIMIT + 1);
        let line = [
            "add",
            "--name",
            "n",
            "--task",
            long.as_str(),
            "--schedule",
            "daily 09:00",
            "--kind",
            "claude",
        ];
        assert_eq!(call_json(&request(&line)), Err("automation_task_too_long"));
    }
}
