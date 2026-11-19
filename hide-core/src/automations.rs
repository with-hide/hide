//! Automations (PRD automations): an agent started on a schedule with the
//! same task, by the core of the machine that runs it (D-21).
//!
//! This module decides and remembers and makes no call: the book of
//! automations and each one's run records, what the clock asks of an
//! automation when its time comes, the run's first prompt, and the file the
//! book is kept in. `runtime/automations.rs` connects it to the runtime: the
//! events, the tick, the starts and the snapshot section.
//!
//! A run is one agent in one terminal (D-03, D-13). An automation runs one
//! run at a time (D-14); a time that came while the machine slept or the
//! core was off runs once when it is back, under one line that counts what
//! was missed (D-15). Records are capped per automation (D-18, engineering
//! 15): past the cap the oldest finished one goes, never a run that is open.

pub(crate) mod schedule;

use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

pub use schedule::{Clock, Meaning, Schedule, Weekday};

/// Records kept per automation (D-18): runs, skips and missed lines.
pub const RECORD_CAP: usize = 50;
/// Automations one core keeps; a create past it is refused (engineering 15).
pub const AUTOMATION_CAP: usize = 200;
/// Attempts one run makes before it ends failed (D-17).
pub const MAX_ATTEMPTS: u32 = 3;
/// The file is refused past this size rather than read.
pub(crate) const READ_CAP: u64 = 16 * 1024 * 1024;
const SCHEMA_VERSION: u32 = 1;
const NAME_CAP: usize = 80;
const TASK_CAP: usize = 16 * 1024;
/// The last answer a run keeps for the next one: one paragraph (D-10).
const ANSWER_CAP: usize = 2_000;
/// How late a time may be noticed and still run as itself: the core's tick
/// runs every 250 ms, so a later notice means the machine slept or the core
/// was off, and the time runs as a missed one (D-15).
pub const LATE_MS: u64 = 2 * 60 * 1000;
/// Times counted for one missed line at most; a core off for longer still
/// writes one line, and the count says at least this.
const MISSED_COUNT_CAP: u32 = 10_000;

/// The agent, model and effort a run starts with (D-07), as the agent start
/// names them.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentChoice {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// Where a run works (D-08): the automation's own folder under
/// `~/hide/automations` (`default`), or a project's folder the operator
/// chose, and then, in a Git project, a new worktree for each run when asked.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Folder {
    pub path: String,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub worktree: bool,
}

/// Why an automation is not running on schedule. Only the operator's own
/// pause is quiet; the others are failures a person fixes, which the
/// automation raises on its own Needs You line (D-12, B23).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    Operator,
    LoginExpired,
    FolderMissing,
    AgentMissing,
}

impl PauseReason {
    /// Whether the automation stopped itself for a person to fix: its own
    /// Needs You line (D-12).
    pub fn asks(self) -> bool {
        self != Self::Operator
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pause {
    pub reason: PauseReason,
    pub at_unix_ms: u64,
}

/// What started a run (B19, B20).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Scheduled,
    Manual,
    Missed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Its agent is being started (an attempt's start, or the next attempt's).
    Starting,
    Running,
    Succeeded,
    /// Every attempt failed (D-17), or a failure a person fixes stopped it
    /// before its agent ran.
    Failed,
    /// A person ended it (B16).
    Stopped,
}

impl RunState {
    pub fn active(self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }
}

/// The agent's conversation, which a finished run is opened again by
/// (D-13): the provider's own session id.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conversation {
    pub kind: String,
    pub id: String,
}

/// A finished run's conversation opened again in a terminal (B17): it does
/// not count as running and holds the record from the cap (B28).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Opened {
    /// The number of this opening, which the start's intent names.
    pub number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub number: u64,
    pub trigger: Trigger,
    /// The time it ran for; none for a manual run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_unix_ms: Option<u64>,
    pub started_at_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_unix_ms: Option<u64>,
    pub state: RunState,
    /// The attempt running now or last (D-17), from 1.
    pub attempt: u32,
    /// Why the last attempt failed or the run ended, as a code the screen
    /// words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The agent's coordination record, which names its pane, and its
    /// conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<Conversation>,
    /// The agent's last answer when it finished the task (D-10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    /// The branch of the run's own worktree, when the automation makes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The run's terminal still waits to be closed: a finished run's closes
    /// once nothing in it waits on the operator (B15).
    #[serde(default)]
    pub closing: bool,
    #[serde(default)]
    pub seen: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened: Option<Opened>,
    /// The conversation could not be opened again (B18).
    #[serde(default)]
    pub conversation_missing: bool,
}

impl Run {
    pub fn finished(&self) -> bool {
        !self.state.active()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkipReason {
    /// The automation's run was still going (D-14).
    PreviousRunning,
    /// The agent's account read 100% before its reset (D-16).
    UsageLimit { agent: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissedReason {
    /// The core was not running.
    CoreOff,
    /// The machine slept.
    Asleep,
}

// Nearly every record is a run, so boxing it would only add an allocation.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Entry {
    Run(Run),
    Skipped {
        at_unix_ms: u64,
        reason: SkipReason,
    },
    /// The times that passed while nothing could run them, one line (D-15).
    Missed {
        first_unix_ms: u64,
        last_unix_ms: u64,
        count: u32,
        reason: MissedReason,
    },
}

impl Entry {
    fn run(&self) -> Option<&Run> {
        match self {
            Self::Run(run) => Some(run),
            _ => None,
        }
    }

    /// A record the cap never drops: a run still going, or one whose
    /// conversation is open (B28).
    fn held(&self) -> bool {
        self.run()
            .is_some_and(|run| run.state.active() || run.opened.is_some() || run.closing)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Automation {
    pub id: String,
    pub name: String,
    pub task: String,
    pub schedule: Schedule,
    pub agent: AgentChoice,
    /// On the machine of the core that keeps it (D-21): the book names no
    /// machine, so it moves with the core like the rest of its state.
    pub folder: Folder,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<Pause>,
    pub created_at_unix_ms: u64,
    /// The next time it runs; none while it is paused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_due_unix_ms: Option<u64>,
    /// Oldest first.
    #[serde(default)]
    pub records: Vec<Entry>,
    #[serde(default)]
    pub next_run_number: u64,
}

/// What the clock asks of an automation whose time came.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Due {
    /// The time the run is for: the last one that passed.
    pub due_unix_ms: u64,
    /// The line of times that passed unrun, when this is not one time
    /// noticed on time.
    pub missed: Option<Entry>,
}

/// How the core came to notice a time (D-15).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Notice {
    /// The first look since the core started.
    Started,
    /// A look after a long gap between ticks: the machine slept.
    AfterGap,
    /// An ordinary tick.
    OnTick,
}

impl Automation {
    pub fn runs(&self) -> impl DoubleEndedIterator<Item = &Run> {
        self.records.iter().filter_map(Entry::run)
    }

    pub fn run_mut(&mut self, number: u64) -> Option<&mut Run> {
        self.records.iter_mut().find_map(|entry| match entry {
            Entry::Run(run) if run.number == number => Some(run),
            _ => None,
        })
    }

    /// The run going now (D-14): one at most.
    pub fn active_run(&self) -> Option<&Run> {
        self.runs().find(|run| run.state.active())
    }

    pub fn on(&self) -> bool {
        self.paused.is_none()
    }

    /// Finished runs the operator has not looked at (B1, B14).
    pub fn unseen(&self) -> usize {
        self.runs()
            .filter(|run| run.finished() && !run.seen)
            .count()
    }

    /// Takes the time that came, if one did by `now`, and moves the next one
    /// past `now`. One time noticed on time runs as itself; anything else,
    /// several times, or one noticed late, is a missed line and one run for
    /// all of them (D-15).
    pub fn take_due(&mut self, now: u64, zone: &TimeZone, notice: Notice) -> Option<Due> {
        let first = self.next_due_unix_ms.filter(|due| *due <= now)?;
        let mut last = first;
        let mut count = 1u32;
        while count < MISSED_COUNT_CAP {
            match self.schedule.next_after(last, zone) {
                Some(next) if next <= now => {
                    last = next;
                    count += 1;
                }
                _ => break,
            }
        }
        self.next_due_unix_ms = self.schedule.next_after(now, zone);
        let on_time = count == 1 && now.saturating_sub(first) <= LATE_MS;
        let missed = (!on_time).then_some(Entry::Missed {
            first_unix_ms: first,
            last_unix_ms: last,
            count,
            reason: match notice {
                Notice::Started => MissedReason::CoreOff,
                Notice::AfterGap | Notice::OnTick => MissedReason::Asleep,
            },
        });
        Some(Due {
            due_unix_ms: last,
            missed,
        })
    }

    /// Adds a record, dropping the oldest ones past the cap that nothing
    /// holds (B28).
    pub fn push(&mut self, entry: Entry) {
        self.records.push(entry);
        while self.records.len() > RECORD_CAP {
            let Some(oldest) = self.records.iter().position(|entry| !entry.held()) else {
                break;
            };
            self.records.remove(oldest);
        }
    }

    /// A new run, its number taken.
    pub fn new_run(&mut self, trigger: Trigger, due: Option<u64>, now: u64) -> u64 {
        self.next_run_number += 1;
        let number = self.next_run_number;
        self.push(Entry::Run(Run {
            number,
            trigger,
            due_unix_ms: due,
            started_at_unix_ms: now,
            ended_at_unix_ms: None,
            state: RunState::Starting,
            attempt: 1,
            reason: None,
            agent_id: None,
            conversation: None,
            answer: None,
            branch: None,
            closing: false,
            seen: false,
            opened: None,
            conversation_missing: false,
        }));
        number
    }

    /// The run's first prompt (D-10, B13): the task, then the last finished
    /// run's time and result and, when it finished the task, its last answer.
    pub fn prompt(&self, before: u64, zone: &TimeZone) -> String {
        let last = self
            .runs()
            .rev()
            .find(|run| run.number < before && run.finished());
        let Some(last) = last else {
            return self.task.clone();
        };
        let when = local_minute(last.started_at_unix_ms, zone);
        let result = match last.state {
            RunState::Succeeded => "성공".to_owned(),
            RunState::Failed => format!("실패 · {}번 시도", last.attempt),
            RunState::Stopped => "멈춤 · 사람이 끝냄".to_owned(),
            RunState::Starting | RunState::Running => unreachable!("a finished run"),
        };
        let mut prompt = format!("{}\n\n[지난 회차 · {when} · {result}]", self.task);
        if let Some(answer) = last
            .answer
            .as_deref()
            .filter(|_| last.state == RunState::Succeeded)
        {
            prompt.push('\n');
            prompt.push_str(answer);
        }
        prompt
    }

    /// The intent of an attempt's start: the same for the same attempt, so a
    /// start asked again, by a restarted core or a second click, is the
    /// spawn it was (B36, engineering 11).
    pub fn attempt_intent(&self, run: u64, attempt: u32) -> String {
        format!("automation-{}-r{run}-a{attempt}", self.id)
    }

    pub fn open_intent(&self, run: u64, number: u32) -> String {
        format!("automation-{}-r{run}-o{number}", self.id)
    }

    /// The branch of a run's own worktree.
    pub fn run_branch(&self, run: u64) -> String {
        format!("automation/{}-r{run}", self.id)
    }
}

/// A wall time to the minute in `zone`, as the next run's prompt writes it.
pub fn local_minute(unix_ms: u64, zone: &TimeZone) -> String {
    i64::try_from(unix_ms)
        .ok()
        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
        .map(|at| {
            at.to_zoned(zone.clone())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// The first paragraph of an answer, cut to the cap (D-10).
pub fn answer_paragraph(text: &str) -> Option<String> {
    let paragraph = text.trim().split("\n\n").next().unwrap_or_default().trim();
    if paragraph.is_empty() {
        return None;
    }
    Some(paragraph.chars().take(ANSWER_CAP).collect())
}

/// What the form or the CLI sends to make an automation (B4, B9).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub name: String,
    pub task: String,
    pub schedule: Schedule,
    pub agent: AgentChoice,
    /// A project's folder on this machine; none works in the automation's
    /// own folder (D-08).
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub worktree: bool,
    /// Make it and start a manual run (B8).
    #[serde(default)]
    pub run_now: bool,
}

/// What an edit changes (B26); a field it leaves out stays.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Changes {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub schedule: Option<Schedule>,
    #[serde(default)]
    pub agent: Option<AgentChoice>,
    /// `Some(None)` moves it to its own folder.
    #[serde(default, with = "double_option")]
    pub folder: Option<Option<String>>,
    #[serde(default)]
    pub worktree: Option<bool>,
}

mod double_option {
    use serde::{Deserialize, Deserializer};
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Option<String>>, D::Error> {
        Option::<String>::deserialize(deserializer).map(Some)
    }
}

/// One call on the automations, from the CLI (`hide automation`, answered)
/// or a window's event (B10). `id` is an automation's id or its exact name.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case", deny_unknown_fields)]
pub enum Call {
    Add(Draft),
    Edit {
        id: String,
        #[serde(flatten)]
        changes: Changes,
    },
    Pause {
        id: String,
    },
    Resume {
        id: String,
    },
    /// Run now (B19).
    Run {
        id: String,
    },
    Remove {
        id: String,
    },
    /// End a run going now (B16).
    RunStop {
        id: String,
        run: u64,
    },
    /// Open a finished run's conversation again (B17).
    RunOpen {
        id: String,
        run: u64,
    },
    /// The operator looked at the automation's runs (B11).
    Seen {
        id: String,
    },
    List,
    Show {
        id: String,
    },
    /// What a schedule reads as and when it runs next, for the form (B6).
    Preview {
        schedule: Schedule,
    },
}

/// The book: every automation of this core, and the agent last chosen,
/// which the form starts from (D-07).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stored {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub automations: Vec<Automation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_choice: Option<AgentChoice>,
}

impl Stored {
    pub fn find(&self, key: &str) -> Option<&Automation> {
        self.automations
            .iter()
            .find(|automation| automation.id == key)
            .or_else(|| {
                let mut named = self
                    .automations
                    .iter()
                    .filter(|automation| automation.name == key);
                named.next().filter(|_| named.next().is_none())
            })
    }

    pub fn position(&self, key: &str) -> Option<usize> {
        let id = self.find(key)?.id.clone();
        self.automations
            .iter()
            .position(|automation| automation.id == id)
    }

    /// Whether any automation is on, which keeps the daemon from its idle
    /// exit (D-19).
    pub fn any_on(&self) -> bool {
        self.automations.iter().any(Automation::on)
    }
}

/// The `automations` snapshot section (PRD automations D-02, D-20): every
/// automation as the table, the detail and the phone draw it, its records
/// newest first, and what the form starts from. Its own revisioned section.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AutomationsSnapshot {
    pub items: Vec<AutomationSnapshot>,
    /// Finished runs not looked at, over every automation: the sidebar row's
    /// number (B1).
    pub unseen: usize,
    /// The agent last chosen, the form's default (D-07).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_choice: Option<AgentChoice>,
    /// The zone the schedules are read in, by its IANA name (B6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    /// `~/hide/automations` on this machine, which the form names under its
    /// default folder (B5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own_folder_root: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AutomationSnapshot {
    pub id: String,
    pub name: String,
    pub task: String,
    /// As the operator chose it, which the form opens on.
    pub schedule: Schedule,
    /// The choice it reads as (B6).
    pub reads_as: Schedule,
    /// None while it is paused.
    pub next_run_at_unix_ms: Option<u64>,
    pub agent: AgentChoice,
    pub folder: Folder,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused: Option<Pause>,
    /// A run goes now: [지금 실행] is refused (B19).
    pub running: bool,
    pub unseen: usize,
    /// Newest first.
    pub records: Vec<RecordSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordSnapshot {
    Run(RunSnapshot),
    Skipped {
        at_unix_ms: u64,
        reason: SkipReason,
    },
    Missed {
        first_unix_ms: u64,
        last_unix_ms: u64,
        count: u32,
        reason: MissedReason,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RunSnapshot {
    pub number: u64,
    pub trigger: Trigger,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_unix_ms: Option<u64>,
    pub started_at_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at_unix_ms: Option<u64>,
    pub state: RunState,
    pub attempt: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The terminal the screen shows for the run: its agent's pane while it
    /// runs or its terminal is still open, else the pane of its
    /// conversation opened again (B16, B17).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    /// Its conversation is open again: "이어서 대화 중" (B17).
    pub opened: bool,
    /// Its conversation can be opened again.
    pub resumable: bool,
    pub conversation_missing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub seen: bool,
}

impl AutomationsSnapshot {
    /// The section of `book`, each run's terminal named by `run_pane` and
    /// each opened conversation's by `opened_pane`.
    pub fn of(
        book: &Stored,
        zone: &TimeZone,
        own_folder_root: Option<String>,
        run_pane: impl Fn(&Run) -> Option<String>,
        opened_pane: impl Fn(&Opened) -> Option<String>,
    ) -> Self {
        let items: Vec<AutomationSnapshot> = book
            .automations
            .iter()
            .map(|automation| AutomationSnapshot {
                id: automation.id.clone(),
                name: automation.name.clone(),
                task: automation.task.clone(),
                schedule: automation.schedule.clone(),
                reads_as: automation.schedule.meaning().reads_as,
                next_run_at_unix_ms: automation.next_due_unix_ms,
                agent: automation.agent.clone(),
                folder: automation.folder.clone(),
                paused: automation.paused,
                running: automation.active_run().is_some(),
                unseen: automation.unseen(),
                records: automation
                    .records
                    .iter()
                    .rev()
                    .map(|entry| match entry {
                        Entry::Run(run) => RecordSnapshot::Run(RunSnapshot {
                            number: run.number,
                            trigger: run.trigger,
                            due_unix_ms: run.due_unix_ms,
                            started_at_unix_ms: run.started_at_unix_ms,
                            ended_at_unix_ms: run.ended_at_unix_ms,
                            state: run.state,
                            attempt: run.attempt,
                            reason: run.reason.clone(),
                            pane_id: run_pane(run)
                                .or_else(|| run.opened.as_ref().and_then(&opened_pane)),
                            opened: run.opened.is_some(),
                            resumable: run.finished() && run.conversation.is_some(),
                            conversation_missing: run.conversation_missing,
                            branch: run.branch.clone(),
                            seen: run.seen,
                        }),
                        Entry::Skipped { at_unix_ms, reason } => RecordSnapshot::Skipped {
                            at_unix_ms: *at_unix_ms,
                            reason: reason.clone(),
                        },
                        Entry::Missed {
                            first_unix_ms,
                            last_unix_ms,
                            count,
                            reason,
                        } => RecordSnapshot::Missed {
                            first_unix_ms: *first_unix_ms,
                            last_unix_ms: *last_unix_ms,
                            count: *count,
                            reason: *reason,
                        },
                    })
                    .collect(),
            })
            .collect();
        Self {
            unseen: items.iter().map(|item| item.unseen).sum(),
            items,
            last_choice: book.last_choice.clone(),
            zone: zone.iana_name().map(str::to_owned),
            own_folder_root,
        }
    }
}

pub(crate) fn encode(stored: &Stored) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(stored).map_err(|error| error.to_string())
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Stored, String> {
    let mut stored: Stored = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if stored.schema_version > SCHEMA_VERSION {
        return Err(format!(
            "schema {} is newer than this build reads",
            stored.schema_version
        ));
    }
    stored.schema_version = SCHEMA_VERSION;
    stored.automations.truncate(AUTOMATION_CAP);
    for automation in &mut stored.automations {
        let excess = automation.records.len().saturating_sub(RECORD_CAP);
        automation.records.drain(..excess);
    }
    Ok(stored)
}

/// The checked name, task and agent of a draft or an edit; the reason code
/// when one is refused (B7, B10).
pub fn checked_name(name: &str) -> Result<String, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("automation_name_missing");
    }
    if name.chars().count() > NAME_CAP || name.chars().any(char::is_control) {
        return Err("automation_name_invalid");
    }
    Ok(name.to_owned())
}

pub fn checked_task(task: &str) -> Result<String, &'static str> {
    let task = task.trim();
    if task.is_empty() {
        return Err("automation_task_missing");
    }
    if task.len() > TASK_CAP {
        return Err("automation_task_too_long");
    }
    Ok(task.to_owned())
}

pub fn checked_agent(agent: &AgentChoice) -> Result<AgentChoice, &'static str> {
    let kind = hide_agent_adapter::start_kind(&agent.kind).ok_or("automation_agent_unknown")?;
    let field = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                if value.len() > 128 || value.chars().any(char::is_control) {
                    Err("automation_agent_unknown")
                } else {
                    Ok(value.to_owned())
                }
            })
            .transpose()
    };
    Ok(AgentChoice {
        kind: kind.to_owned(),
        profile: field(&agent.profile)?,
        model: field(&agent.model)?,
        effort: field(&agent.effort)?,
    })
}

/// A short id unlike every id in `taken`, from the time it was made.
pub fn new_id(now: u64, taken: &[&str]) -> String {
    let mut seed = now;
    loop {
        let id = base36(seed % 36u64.pow(6));
        let id = format!("{id:0>6}");
        if !taken.contains(&id.as_str()) {
            return id;
        }
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    }
}

fn base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
        if value == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// The automation's own folder's name, `<name>-<id>` (D-08): the name with
/// what no system takes in a folder name left out, so two automations of the
/// same name never share one.
pub fn own_folder_name(name: &str, id: &str) -> String {
    let mut slug = String::new();
    for character in name.trim().chars() {
        if character.is_whitespace() {
            if !slug.ends_with('-') {
                slug.push('-');
            }
        } else if !character.is_control() && !"<>:\"/\\|?*.".contains(character) {
            slug.push(character);
        }
        if slug.chars().count() >= 40 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        id.to_owned()
    } else {
        format!("{slug}-{id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u64 = 3_600_000;

    fn seoul() -> TimeZone {
        TimeZone::get("Asia/Seoul").unwrap()
    }

    fn at(text: &str) -> u64 {
        let local: jiff::civil::DateTime = text.parse().unwrap();
        u64::try_from(
            seoul()
                .to_ambiguous_zoned(local)
                .compatible()
                .unwrap()
                .timestamp()
                .as_millisecond(),
        )
        .unwrap()
    }

    fn automation(schedule: Schedule, now: u64) -> Automation {
        Automation {
            id: "abc123".into(),
            name: "이슈 정리".into(),
            task: "새 이슈를 정리해줘".into(),
            next_due_unix_ms: schedule.next_after(now, &seoul()),
            schedule,
            agent: AgentChoice {
                kind: "claude".into(),
                ..AgentChoice::default()
            },
            folder: Folder {
                path: "/home/me/hide/automations/x".into(),
                default: true,
                worktree: false,
            },
            paused: None,
            created_at_unix_ms: now,
            records: Vec::new(),
            next_run_number: 0,
        }
    }

    fn daily(time: &str) -> Schedule {
        Schedule::Daily {
            time: Clock::parse(time).unwrap(),
        }
    }

    fn hourly() -> Schedule {
        Schedule::Cron {
            expr: "0 * * * *".into(),
        }
    }

    /// B12: the clock moving past a time hands it over once, on time, and
    /// moves the next time a day on.
    #[test]
    fn a_time_noticed_on_time_runs_as_itself_once() {
        let mut automation = automation(daily("09:00"), at("2026-11-19T08:00"));
        let zone = seoul();
        assert_eq!(
            automation.take_due(at("2026-11-19T08:59"), &zone, Notice::OnTick),
            None
        );
        let due = automation
            .take_due(at("2026-11-19T09:00:01"), &zone, Notice::OnTick)
            .unwrap();
        assert_eq!(due.due_unix_ms, at("2026-11-19T09:00"));
        assert_eq!(due.missed, None);
        assert_eq!(automation.next_due_unix_ms, Some(at("2026-11-20T09:00")));
        assert_eq!(
            automation.take_due(at("2026-11-19T09:00:02"), &zone, Notice::OnTick),
            None,
            "the same time is not handed over twice"
        );
    }

    /// B20, D-15: a core off from yesterday 10:00 to today 08:30 with an
    /// hourly automation writes one line for 23 times and runs once.
    #[test]
    fn times_missed_while_the_core_was_off_are_one_line_and_one_run() {
        let mut automation = automation(hourly(), at("2026-11-18T09:30"));
        let due = automation
            .take_due(at("2026-11-19T08:30"), &seoul(), Notice::Started)
            .unwrap();
        assert_eq!(due.due_unix_ms, at("2026-11-19T08:00"));
        assert_eq!(
            due.missed,
            Some(Entry::Missed {
                first_unix_ms: at("2026-11-18T10:00"),
                last_unix_ms: at("2026-11-19T08:00"),
                count: 23,
                reason: MissedReason::CoreOff,
            })
        );
        assert_eq!(automation.next_due_unix_ms, Some(at("2026-11-19T09:00")));
    }

    /// One time noticed long after it came, as after a sleep, is missed too.
    #[test]
    fn one_time_noticed_late_after_a_sleep_is_a_missed_line() {
        let mut automation = automation(daily("09:00"), at("2026-11-19T08:00"));
        let due = automation
            .take_due(at("2026-11-19T11:00"), &seoul(), Notice::AfterGap)
            .unwrap();
        assert_eq!(
            due.missed,
            Some(Entry::Missed {
                first_unix_ms: at("2026-11-19T09:00"),
                last_unix_ms: at("2026-11-19T09:00"),
                count: 1,
                reason: MissedReason::Asleep,
            })
        );
    }

    /// B28: past 50 records the oldest finished one goes; a run still
    /// going and a run whose conversation is open stay.
    #[test]
    fn the_record_cap_drops_the_oldest_finished_record_and_keeps_open_runs() {
        let now = at("2026-11-19T08:00");
        let mut automation = automation(hourly(), now);
        let open = automation.new_run(Trigger::Manual, None, now);
        let run = automation.run_mut(open).unwrap();
        run.state = RunState::Succeeded;
        run.opened = Some(Opened::default());
        for index in 0..60 {
            automation.push(Entry::Skipped {
                at_unix_ms: now + index * HOUR,
                reason: SkipReason::PreviousRunning,
            });
        }
        assert_eq!(automation.records.len(), RECORD_CAP);
        assert!(automation.run_mut(open).is_some(), "an open run is kept");
        assert_eq!(
            automation.records[1],
            Entry::Skipped {
                at_unix_ms: now + 11 * HOUR,
                reason: SkipReason::PreviousRunning,
            },
            "the oldest skips went first"
        );
    }

    /// B13: the first run gets the task alone; the next one the last run's
    /// time, result and the answer it finished with.
    #[test]
    fn a_runs_prompt_carries_the_last_runs_time_result_and_answer() {
        let zone = seoul();
        let now = at("2026-11-19T09:00");
        let mut automation = automation(daily("09:00"), now);
        let first = automation.new_run(Trigger::Scheduled, Some(now), now);
        assert_eq!(automation.prompt(first, &zone), "새 이슈를 정리해줘");
        let run = automation.run_mut(first).unwrap();
        run.state = RunState::Succeeded;
        run.answer = answer_paragraph("새 이슈 3개를 정리했어요.\n\n자세한 내용은 아래.");
        let second = automation.new_run(Trigger::Scheduled, Some(now + 24 * HOUR), now);
        assert_eq!(
            automation.prompt(second, &zone),
            "새 이슈를 정리해줘\n\n[지난 회차 · 2026-11-19 09:00 · 성공]\n새 이슈 3개를 정리했어요."
        );
        let run = automation.run_mut(second).unwrap();
        run.state = RunState::Failed;
        run.attempt = 3;
        let third = automation.new_run(Trigger::Manual, None, now);
        assert!(
            automation
                .prompt(third, &zone)
                .ends_with("[지난 회차 · 2026-11-19 09:00 · 실패 · 3번 시도]"),
            "a failed run passes on no answer"
        );
    }

    #[test]
    fn a_finished_run_counts_unseen_until_looked_at() {
        let now = at("2026-11-19T09:00");
        let mut automation = automation(daily("09:00"), now);
        let running = automation.new_run(Trigger::Scheduled, Some(now), now);
        assert_eq!(automation.unseen(), 0, "a run going is not unseen");
        automation.run_mut(running).unwrap().state = RunState::Succeeded;
        assert_eq!(automation.unseen(), 1);
        automation.run_mut(running).unwrap().seen = true;
        assert_eq!(automation.unseen(), 0);
    }

    #[test]
    fn an_own_folder_name_keeps_the_name_and_drops_what_no_system_takes() {
        assert_eq!(own_folder_name("이슈 정리", "a1b2c3"), "이슈-정리-a1b2c3");
        assert_eq!(own_folder_name("a/b:c?", "a1b2c3"), "abc-a1b2c3");
        assert_eq!(own_folder_name("  ", "a1b2c3"), "a1b2c3");
        assert_eq!(own_folder_name("..", "a1b2c3"), "a1b2c3");
    }

    #[test]
    fn a_book_written_by_a_newer_build_is_refused_and_records_are_capped() {
        assert!(decode(br#"{"schema_version": 9}"#).is_err());
        let now = at("2026-11-19T09:00");
        let mut automation = automation(daily("09:00"), now);
        automation.records = (0..70)
            .map(|index| Entry::Skipped {
                at_unix_ms: index,
                reason: SkipReason::PreviousRunning,
            })
            .collect();
        let stored = Stored {
            schema_version: 1,
            automations: vec![automation],
            last_choice: None,
        };
        let read = decode(&encode(&stored).unwrap()).unwrap();
        assert_eq!(read.automations[0].records.len(), RECORD_CAP);
    }
}
