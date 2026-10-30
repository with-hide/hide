//! Persistent participant identities and resumable child creation. The delivery
//! store is the sole writer; native I/O is performed by request workers.

mod executor;
pub(crate) mod hook_tokens;
pub(crate) mod identity_wait;
pub(crate) mod lineage;
pub(crate) mod naming;
pub(crate) mod started;

use crate::delivery::answer::{AgentView, Connection, RegisterCheck, Runtime};
use crate::delivery::{Actor, ledger::Ledger, watch};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Separates a refusal code from the detail its next action names. A control
/// character, because no code and no sentence of a refusal holds one.
const DETAIL: char = '\u{1f}';

/// A refusal code with the detail its next action names (the device ids a
/// caller could have used, or the last line of a shell that had not finished
/// starting); the daemon splits them again with
/// [`split_refusal`] and answers the code alone.
pub(crate) fn refusal(code: &str, detail: &str) -> String {
    format!("{code}{DETAIL}{detail}")
}

/// The code of a refusal and the detail it carries, if any.
pub fn split_refusal(refusal: &str) -> (&str, Option<&str>) {
    // Only the refusals that build a detail carry one, so text from a device
    // that happens to hold the separator is never read as a next action.
    match refusal.split_once(DETAIL) {
        Some((
            code @ ("machine_unknown"
            | crate::agent_start::SHELL_STARTING
            | "card_not_processed"
            | "mission_cap_reached"
            | "mission_conducted"
            | "mission_criteria_missing"
            | "mission_criteria_too_many"
            | "mission_criteria_unmet"
            | "mission_criteria_protected"
            | "mission_checkers_invalid"
            | "mission_checker_is_worker"
            | "mission_external_invalid"
            | "mission_nodes_unfinished"
            | "plan_invalid"
            | "plan_order_refused"
            | "plan_too_large"
            | "plan_node_unknown"),
            detail,
        )) => (code, Some(detail)),
        _ => (refusal, None),
    }
}

pub(crate) const AGENT_LIMIT: usize = 2048;
pub(crate) const SPAWN_LIMIT: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Register {
        check: bool,
        /// The caller's own machine when given; absent means the machine
        /// the caller's pane capability already names.
        machine: Option<String>,
        host_scope: String,
        session: String,
        instance: String,
        name: String,
        pane: String,
        parent: Option<String>,
        project: Option<String>,
    },
    List,
    Show {
        id: String,
    },
    End {
        id: String,
        actor: Option<String>,
    },
    Spawn {
        parent: Option<String>,
        /// The device the child is created on; absent means the caller's own.
        /// `repo` and `path` are that device's paths.
        machine: Option<String>,
        name: String,
        intent: String,
        kind: String,
        repo: String,
        branch: String,
        path: Option<String>,
        /// The agent's own arguments for this start only.
        args: Vec<String>,
        /// The model and effort the child starts with, which every restart
        /// Hide makes of it (a wake, a fork, a reopen) starts it with again.
        #[serde(
            default,
            skip_serializing_if = "hide_agent_adapter::LaunchChoice::is_default"
        )]
        launch: hide_agent_adapter::LaunchChoice,
        /// What Hide asked the child to do, for the screen to name it. Only
        /// Hide's own identity may give one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<SpawnTask>,
        /// The child's first prompt, which Hide submits once the agent takes
        /// input and records as a delivered letter under the spawn's intent,
        /// so the child starts on it without a person typing. Part of the
        /// intent like `args`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        brief: Option<String>,
        /// The child's pane opens beside the caller's, in the caller's tab,
        /// instead of in a tab of its own; only for the caller's own checkout
        /// on the caller's own device. Part of the intent like `brief`.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        beside: bool,
        /// The mission this spawn starts a worker of, and its cap (PRD
        /// software-factory-conductor D-45). Only the core sets it, from the
        /// caller's mission; no command names it.
        #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
        mission: Option<crate::runtime::missions::MissionAdmission>,
    },
}

/// How long a spawn waits for its agent to take input before the brief is
/// submitted: the TUI of a started agent takes seconds to draw its prompt,
/// and a trust question at startup holds it longer.
pub(crate) const BRIEF_READY_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long a caller waits for a spawn's answer: the bounded steps a spawn
/// takes one after another (its worktree, the shell reaching its prompt for
/// the start, the agent taking input, the brief, the session showing), and
/// a margin for the store's writes and a device's SSH round trips between
/// them. A caller that waits less hears `request_timeout` from a spawn that
/// is only slow, while the spawn goes on without it.
pub const SPAWN_ANSWER_WITHIN: std::time::Duration = std::time::Duration::from_secs(
    executor::WORKTREE_CREATE_TIMEOUT.as_secs()
        + crate::agent_start::SHELL_WAIT.as_secs()
        + BRIEF_READY_WAIT.as_secs()
        + executor::BRIEF_PROMPT_TIMEOUT.as_secs()
        + executor::IDENTITY_WAIT.as_secs()
        + 15,
);

/// How long a mission's spawn with no child yet counts as a worker being
/// made. A start takes seconds to a minute or two; one still childless past
/// this failed and was left (a retry under a new intent is its own spawn),
/// so it stops holding a place under the cap.
pub(crate) const MISSION_RESERVATION_HOLD_MS: u64 = 10 * 60 * 1000;

/// The workers a mission has now (D-29): spawns made for it whose child has
/// not ended, and those still being made, which will be one.
pub(crate) fn mission_workers(ledger: &Ledger, mission: &str, now: u64) -> usize {
    ledger
        .spawns
        .iter()
        .filter(|spawn| spawn.mission.as_deref() == Some(mission))
        .filter(|spawn| match &spawn.child {
            None => {
                !spawn.completed
                    && now.saturating_sub(spawn.reserved_at_unix_ms) < MISSION_RESERVATION_HOLD_MS
            }
            Some(child) => ledger
                .agents
                .iter()
                .any(|agent| &agent.id == child && !agent.ended),
        })
        .count()
}

/// The earliest time after `now` a mission's worker still being made stops
/// counting against its cap, so a count read now holds until then.
pub(crate) fn mission_reservations_end(ledger: &Ledger, now: u64) -> u64 {
    ledger
        .spawns
        .iter()
        .filter(|spawn| spawn.mission.is_some() && spawn.child.is_none() && !spawn.completed)
        .map(|spawn| {
            spawn
                .reserved_at_unix_ms
                .saturating_add(MISSION_RESERVATION_HOLD_MS)
        })
        .filter(|end| *end > now)
        .min()
        .unwrap_or(u64::MAX)
}

/// The serialized value of a Memory card's promotion into the repository.
pub const MEMORY_PROMOTE_TASK: &str = "memory_promote";
/// The serialized value of a 제안 line's application.
pub const SUGGESTION_APPLY_TASK: &str = "suggestion_apply";

/// What Hide started an agent for. Every kind Hide knows names the screen that
/// owns its progress, its home, in one exhaustive match, so a kind added
/// without a home does not compile (PRD hide-ai-work-home D-10, B22).
///
/// The ledger outlives a build: a kind another build wrote reads as
/// `Unknown` and never fails the ledger's load (D-19). The serialized value
/// of a known kind is its code, unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskKind {
    /// A Memory card's promotion into the repository.
    MemoryPromote,
    /// A 제안 line's fix, as a pull request or an issue (PRD
    /// hide-ai-suggestions B11-B14).
    SuggestionApply,
    /// A kind this build has no home for, with the value as written.
    Unknown(String),
}

/// The screen that owns what a Hide-started agent does: it shows the
/// progress, opens the agent's pane and cleans up after the work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskHome {
    /// The Hide AI tool's 제안 tab, on the row the task's target names: a
    /// notebook line (`s12`), or a Memory card on its way into the
    /// repository's documents (`m72`), which the tab took over from the
    /// Memory tab (PRD hide-ai-suggestions D-02).
    Suggestions,
}

impl TaskKind {
    pub fn from_code(code: &str) -> Self {
        match code {
            MEMORY_PROMOTE_TASK => Self::MemoryPromote,
            SUGGESTION_APPLY_TASK => Self::SuggestionApply,
            // A code this build does not know is shown and logged, never
            // acted on, so only a short printable form of it is kept.
            other => Self::Unknown(
                other
                    .chars()
                    .filter(|character| !character.is_control())
                    .take(64)
                    .collect(),
            ),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::MemoryPromote => MEMORY_PROMOTE_TASK,
            Self::SuggestionApply => SUGGESTION_APPLY_TASK,
            Self::Unknown(code) => code,
        }
    }

    /// The kind's home; none for a kind this build does not know, whose
    /// agent is then an operator's row (D-19).
    pub fn home(&self) -> Option<TaskHome> {
        match self {
            Self::MemoryPromote | Self::SuggestionApply => Some(TaskHome::Suggestions),
            Self::Unknown(_) => None,
        }
    }
}

impl Serialize for TaskKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TaskKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|code| Self::from_code(&code))
    }
}

/// The work Hide started an agent for, as codes the screen words in the
/// operator's language (`memory_promote` and the card `m72`), never as a
/// sentence. `target` is the thing the work is about, when it has one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnTask {
    pub kind: TaskKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "AgentRecordOnDisk", into = "AgentRecordOnDisk")]
pub struct AgentRecord {
    pub id: String,
    /// The name the agent registered under; none when it never had one of its
    /// own. A pane id is never a name (`AgentRecord::forget_pane_name`). On
    /// disk the field is always a string, so a build before this one reads the
    /// ledger: an unnamed record is written under its registration id.
    pub name: Option<String>,
    pub machine: String,
    pub host_scope: String,
    pub native_machine: String,
    pub session: String,
    pub instance: String,
    /// Herdr's terminal incarnation, retained when a move changes the public pane id.
    pub terminal_id: Option<String>,
    pub pane: String,
    pub parent: Option<String>,
    /// Provenance only for handoffs and forks; delegated provenance is the
    /// parent. A fork's origin is the source it was forked from, which its
    /// pane's fork mark names (`lineage::FORK_KEYS`).
    #[serde(default)]
    pub origin: Option<String>,
    pub project: Option<String>,
    pub actor: Actor,
    pub ended: bool,
    /// Made by Fork agent: the conversation is another execution's, resumed
    /// as a new session. The registration owns the fact, so it follows the
    /// registration to the pane that wakes it. A fork made before forks were
    /// the operator's roots has a parent and carries no mark.
    #[serde(default)]
    pub forked: bool,
}

/// `AgentRecord` as the ledger file spells it, with the wire name in `name`.
#[derive(Serialize, Deserialize)]
struct AgentRecordOnDisk {
    id: String,
    name: String,
    machine: String,
    host_scope: String,
    native_machine: String,
    session: String,
    instance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_id: Option<String>,
    pane: String,
    parent: Option<String>,
    #[serde(default)]
    origin: Option<String>,
    project: Option<String>,
    actor: Actor,
    ended: bool,
    #[serde(default)]
    forked: bool,
}

impl From<AgentRecord> for AgentRecordOnDisk {
    fn from(record: AgentRecord) -> Self {
        Self {
            name: record.wire_name().to_owned(),
            id: record.id,
            machine: record.machine,
            host_scope: record.host_scope,
            native_machine: record.native_machine,
            session: record.session,
            instance: record.instance,
            terminal_id: record.terminal_id,
            pane: record.pane,
            parent: record.parent,
            origin: record.origin,
            project: record.project,
            actor: record.actor,
            ended: record.ended,
            forked: record.forked,
        }
    }
}

/// A name equal to the record's own id or to its pane is no name: the first is
/// how an unnamed record is written, the second how an earlier build wrote a
/// parent Herdr never named.
impl From<AgentRecordOnDisk> for AgentRecord {
    fn from(disk: AgentRecordOnDisk) -> Self {
        let mut record = Self {
            name: Some(disk.name),
            id: disk.id,
            machine: disk.machine,
            host_scope: disk.host_scope,
            native_machine: disk.native_machine,
            session: disk.session,
            instance: disk.instance,
            terminal_id: disk.terminal_id,
            pane: disk.pane,
            parent: disk.parent,
            origin: disk.origin,
            project: disk.project,
            actor: disk.actor,
            ended: disk.ended,
            forked: disk.forked,
        };
        record.forget_pane_name();
        record
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnMode {
    #[default]
    Delegation,
    Handoff,
}

impl SpawnMode {
    fn responsibility(self, caller: &str) -> Option<String> {
        (self == Self::Delegation).then(|| caller.to_owned())
    }

    fn origin(self, caller: &str) -> Option<String> {
        (self == Self::Handoff).then(|| caller.to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_tab: Option<naming::SeedTab>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub names: Option<naming::Names>,
    pub id: String,
    /// The caller authorizes the receipt, including a parentless handoff.
    pub parent: String,
    #[serde(default)]
    pub mode: SpawnMode,
    /// The device the child lives on when it is not the caller's own. It is
    /// part of the intent: the same intent never reaches two devices.
    #[serde(default)]
    pub machine: Option<String>,
    pub intent: String,
    pub name: String,
    pub kind: String,
    pub repo: String,
    pub branch: String,
    pub requested_path: Option<String>,
    pub args: Vec<String>,
    /// Part of the intent like `args`; a record written before it existed
    /// has the CLI's defaults.
    #[serde(
        default,
        skip_serializing_if = "hide_agent_adapter::LaunchChoice::is_default"
    )]
    pub launch: hide_agent_adapter::LaunchChoice,
    pub path: Option<String>,
    pub pane: Option<String>,
    /// The Herdr terminal `pane` ran when the spawn recorded it. A pane id
    /// can name another pane later, so only this proves the pane is the
    /// spawn's own; a record written before it existed has none.
    #[serde(default)]
    pub terminal: Option<String>,
    /// How many starts Herdr took for this spawn's agent in `pane`: the first
    /// one, and at most one more when the first ended at the shell with no
    /// agent (`executor::spawn`).
    #[serde(default)]
    pub typed: u32,
    /// The shell `pane` was launched with Hide's integration in, by program
    /// name, so every start in it waits for that shell's first prompt; none
    /// when the pane runs Herdr's own shell.
    #[serde(default)]
    pub shell: Option<String>,
    pub child: Option<String>,
    // Completion and the initial watch are one durable store transaction.
    // Later report/end/watch-stop events never invalidate this receipt.
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub auto_watch_id: Option<String>,
    /// What Hide started the child for; absent for every other spawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<SpawnTask>,
    /// The mission the child is a worker of, counted against its cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission: Option<String>,
    /// The child's first prompt (`Command::Spawn::brief`), part of the
    /// intent; the letter made of it when the child binds carries the same
    /// text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    /// Whether the brief was submitted to the agent, so a retry after it
    /// never submits it twice (engineering #11).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub briefed: bool,
    /// The child's pane was split beside the caller's (`Command::Spawn::
    /// beside`), part of the intent. Delegated-child relocation leaves the
    /// child of such a spawn in the caller's tab.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub beside: bool,
    /// When the spawn was reserved; a record written before it existed has
    /// 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reserved_at_unix_ms: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

pub(crate) enum Mutation {
    Register {
        record: AgentRecord,
        check: bool,
    },
    End {
        id: String,
        actor: Option<String>,
    },
    Reserve {
        parent: String,
        command: Command,
    },
    /// The spawn's brief was submitted to its agent.
    Briefed {
        id: String,
    },
    Advance {
        seed_tab: Option<naming::SeedTab>,
        id: String,
        path: Option<String>,
        pane: Option<String>,
        /// The terminal `pane` runs, recorded with it.
        terminal: Option<String>,
        /// The integrated shell `pane` runs, recorded with it.
        shell: Option<String>,
        child: Option<String>,
    },
    /// Herdr took a start of the spawn's agent in its pane.
    Typed {
        id: String,
    },
    BindChild {
        id: String,
        record: AgentRecord,
    },
    Complete {
        id: String,
    },
    /// The agent Herdr took a start of has not shown its session yet: the
    /// registration waits for it (`identity_wait`).
    AwaitIdentity {
        wait: identity_wait::IdentityWait,
    },
    /// An agent the caller started through Herdr directly, seen after the
    /// fact by a passive observation: the operator's root with the caller as
    /// its origin. Only the core's observation path builds one
    /// (`executor::link_started`).
    RegisterStarted {
        record: AgentRecord,
    },
    /// A session took `mission` over (PRD software-factory-conductor B43):
    /// its workers, their starts and the watches on them become those of
    /// the registration `conductor`. Only Hide's own identity asks it, from
    /// the mission's takeover.
    AdoptMissionWorkers {
        mission: String,
        conductor: String,
    },
}

impl AgentRecord {
    /// The name this agent has on the wire and in the ledger: the name it
    /// registered under, else its registration id, which is stable and which
    /// `hide request send` and `--parent` already resolve. Never a pane id and
    /// never a title, so a letter's envelope names its sender the way the
    /// label worker later names that sender's work (`wire_name`).
    pub fn wire_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    /// A record written while a pane id was taken for a name, or under its own
    /// registration id, reads as having none. Reading does it, so no migration
    /// is needed: the next save writes the record in its current shape.
    fn forget_pane_name(&mut self) {
        // A Factory's or Hide's own identity is named by its code-owned name,
        // which is also its pane.
        if !self.actor.code_owned()
            && self.name.as_deref().is_some_and(|name| {
                name == self.pane
                    || name == self.actor.pane_id
                    || (!self.id.is_empty() && name == self.id)
            })
        {
            self.name = None;
        }
    }
}

/// The one name `actor` goes by in the ledger and on the wire: its live
/// registration's `wire_name`, else, for a pane that never registered, the
/// only identifier Herdr gave it. The writing side (a letter's or a watch's
/// endpoints) and the reading side (`parent_name` in the lineage facts) both
/// take it from here, so the envelope header and the name it is later matched
/// against cannot drift.
pub(crate) fn wire_name(ledger: &Ledger, actor: &Actor) -> String {
    ledger
        .agents
        .iter()
        .rev()
        .find(|record| !record.ended && record.actor.same_identity(actor))
        .map_or_else(
            || actor.name.clone(),
            |record| record.wire_name().to_owned(),
        )
}

/// `actor` as the ledger stores it: under its `wire_name`.
pub(crate) fn named(ledger: &Ledger, actor: &Actor) -> Actor {
    Actor {
        name: wire_name(ledger, actor),
        ..actor.clone()
    }
}

pub fn resolve_actor<'a>(ledger: &'a Ledger, key: &str) -> Option<&'a Actor> {
    resolve_by(ledger, |record| {
        record.id == key
            || record.name.as_deref() == Some(key)
            || record.actor.pane_id == key
            || record.pane == key
    })
}

/// The live registration whose registration id or name is `key`, never one
/// that only sits in a pane called `key`: a pane id is per machine, so a
/// caller that names a pane resolves it against its own machine's panes.
pub(crate) fn resolve_registered<'a>(ledger: &'a Ledger, key: &str) -> Option<&'a Actor> {
    resolve_by(ledger, |record| {
        record.id == key || record.name.as_deref() == Some(key)
    })
}

fn resolve_by(ledger: &Ledger, is: impl Fn(&AgentRecord) -> bool) -> Option<&Actor> {
    let mut found = ledger
        .agents
        .iter()
        .filter(|record| !record.ended && is(record));
    let first = found.next()?;
    found.next().is_none().then_some(&first.actor)
}

/// The id `hide agent show` and `--parent` take for the caller itself.
pub const HERE: &str = "here";

/// The caller's live registrations: the records not ended whose actor is
/// the caller's attested pane, device and session. `--parent here` takes
/// the first; `here` refuses more than one.
pub(crate) fn live_self<'a>(
    ledger: &'a Ledger,
    caller: &'a Actor,
) -> impl Iterator<Item = &'a AgentRecord> {
    ledger
        .agents
        .iter()
        .filter(move |record| !record.ended && record.actor.same_identity(caller))
}

/// The caller's own registration, by the same match as `--parent here`,
/// refusing two. It never registers. A caller with no such record learns
/// which of the ways it has none: its record ended, its pane's live record
/// belongs to another session (the pane's agent session changed), or there
/// is none.
pub(crate) fn here<'a>(
    ledger: &'a Ledger,
    caller: &'a Actor,
) -> Result<&'a AgentRecord, &'static str> {
    let mut live = live_self(ledger, caller);
    match (live.next(), live.next()) {
        (Some(record), None) => return Ok(record),
        (Some(_), Some(_)) => return Err("ambiguous_participant"),
        _ => {}
    }
    if ledger
        .agents
        .iter()
        .any(|record| record.ended && record.actor.same_identity(caller))
    {
        Err("participant_ended")
    } else if ledger.agents.iter().any(|record| {
        !record.ended
            && record.actor.pane_id == caller.pane_id
            && record.actor.device_id == caller.device_id
    }) {
        Err("participant_session_changed")
    } else {
        Err("participant_unavailable")
    }
}

pub(crate) fn view(record: &AgentRecord, ledger: &Ledger) -> Value {
    json!(agent_view(record, ledger))
}

pub(crate) fn agent_view(record: &AgentRecord, ledger: &Ledger) -> AgentView {
    AgentView {
        names: ledger
            .spawns
            .iter()
            .find(|spawn| spawn.child.as_deref() == Some(&record.id))
            .and_then(|spawn| spawn.names.clone()),
        id: record.id.clone(),
        name: record.wire_name().to_owned(),
        machine: record.machine.clone(),
        host_scope: record.host_scope.clone(),
        session: record.session.clone(),
        instance: record.instance.clone(),
        pane: record.pane.clone(),
        parent: record.parent.clone(),
        origin: record.origin.clone().or_else(|| record.parent.clone()),
        project: record.project.clone(),
        runtime: if record.ended {
            Runtime::Ended
        } else {
            Runtime::Running
        },
        connection: if record.ended {
            Connection::Disconnected
        } else {
            Connection::Connected
        },
        registered: !record.ended,
        watch: ledger
            .watches
            .iter()
            .find(|watch| watch.target.same_identity(&record.actor))
            .cloned(),
    }
}

fn key(value: &str) -> bool {
    crate::delivery::valid_key(value)
}

/// Whether a registration can keep `value` as its pane's native session, as
/// Herdr reports it: an id, or for Pi and omp the session file's path, which a
/// deep checkout makes longer than a key. Herdr keeps a path of up to
/// `SESSION_LIMIT` bytes, and so does the ledger. The one rule for a
/// registration's session, asked when the record is made and again when the
/// ledger is saved or read.
pub(crate) fn kept_session(value: &str) -> Result<(), &'static str> {
    if value.len() > crate::delivery::SESSION_LIMIT {
        return Err(SESSION_TOO_LONG);
    }
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err("invalid_registration");
    }
    Ok(())
}

/// The refusal of a registration whose pane reports a session past what the
/// ledger keeps, given when the record is made rather than when the whole
/// ledger fails to save.
pub(crate) const SESSION_TOO_LONG: &str = "session_too_long";

pub(crate) fn validate_records(ledger: &Ledger) -> Result<(), String> {
    if ledger.agents.len() > AGENT_LIMIT || ledger.spawns.len() > SPAWN_LIMIT {
        return Err("capacity".into());
    }
    let mut ids = std::collections::HashSet::new();
    for record in &ledger.agents {
        if !ids.insert(record.id.as_str())
            || !key(&record.id)
            || !record.id.starts_with("agent-")
            || record.name.as_deref().is_some_and(|name| !key(name))
            || record
                .terminal_id
                .as_deref()
                .is_some_and(|terminal| !key(terminal))
            || ![
                &record.machine,
                &record.host_scope,
                &record.native_machine,
                &record.instance,
                &record.pane,
            ]
            .into_iter()
            .all(|s| key(s))
            || kept_session(&record.session).is_err()
            || !record.actor.valid()
            || record.actor.device_id != record.machine
            || record.actor.session != crate::wire::session_digest(&record.session)
            || record.origin.as_ref().is_some_and(|origin| {
                record.parent.is_some()
                    || origin == &record.id
                    || !ledger.agents.iter().any(|record| &record.id == origin)
            })
            || record.parent.as_ref().is_some_and(|parent| {
                parent == &record.id || !ledger.agents.iter().any(|record| &record.id == parent)
            })
        {
            return Err("ledger_unavailable".into());
        }
    }
    for record in &ledger.spawns {
        if !ids.insert(record.id.as_str())
            || !key(&record.id)
            || !record.id.starts_with("spawn-")
            || ![
                &record.parent,
                &record.intent,
                &record.name,
                &record.kind,
                &record.repo,
                &record.branch,
            ]
            .into_iter()
            .all(|s| key(s))
            || record
                .machine
                .as_deref()
                .is_some_and(|machine| !key(machine))
            || !ledger
                .agents
                .iter()
                .any(|parent| parent.id == record.parent)
            || record.child.as_ref().is_some_and(|id| {
                !ledger.agents.iter().any(|child| {
                    &child.id == id
                        && child.parent == record.mode.responsibility(&record.parent)
                        && child.origin == record.mode.origin(&record.parent)
                        && record.pane.as_ref() == Some(&child.pane)
                        && Some(child.machine.as_str()) == spawn_machine(ledger, record)
                })
            })
            || (record.completed
                && (record.path.is_none() || record.pane.is_none() || record.child.is_none()))
            || record.auto_watch_id.as_ref().is_some_and(|id| {
                !record.completed
                    || record.mode == SpawnMode::Handoff
                    || !key(id)
                    || !id.starts_with("watch-")
            })
            || record.args.len() > 128
            || record.args.iter().map(String::len).sum::<usize>() > 8192
            || record.task.as_ref().is_some_and(|task| {
                record.mode != SpawnMode::Handoff
                    || task.target.as_deref().is_some_and(|target| !key(target))
            })
        {
            return Err("ledger_unavailable".into());
        }
    }
    if ledger.identity_waits.len() > identity_wait::LIMIT {
        return Err("capacity".into());
    }
    for wait in &ledger.identity_waits {
        if !ids.insert(wait.id.as_str())
            || !wait.id.starts_with("wait-")
            || ![&wait.id, &wait.machine, &wait.pane, &wait.name, &wait.kind]
                .into_iter()
                .all(|s| key(s))
            || !match &wait.owed {
                identity_wait::Owed::Spawn { spawn } => {
                    ledger.spawns.iter().any(|record| &record.id == spawn)
                }
                identity_wait::Owed::Fork { origin } => {
                    ledger.agents.iter().any(|record| &record.id == origin)
                }
            }
        {
            return Err("ledger_unavailable".into());
        }
    }
    Ok(())
}

pub(crate) fn apply(
    ledger: &mut Ledger,
    caller: &Actor,
    mutation: &Mutation,
    now: u64,
) -> Result<Value, String> {
    match mutation {
        Mutation::Register { record, check } => {
            validate_registration_name(caller, record)?;
            let callers_own = |id: &String| {
                ledger
                    .agents
                    .iter()
                    .any(|p| &p.id == id && !p.ended && p.actor.same_identity(caller))
            };
            // A fork is the operator's root, so it has no parent; the source
            // that asked for it registers it and is named as its origin.
            let forked_by_caller = record.forked
                && record.parent.is_none()
                && record.origin.as_ref().is_some_and(callers_own);
            if record.parent.as_ref().is_some_and(|id| !callers_own(id))
                || (record.parent.is_none()
                    && !record.actor.same_identity(caller)
                    && !forked_by_caller)
            {
                return Err("parent_authority_required".into());
            }
            // Registration has no provenance input but a fork's, which only
            // the core's own Fork agent sets (`executor::link_fork`). A
            // handoff's own retry keeps its stored origin; binding a spawn
            // still compares the complete immutable identity through the
            // strict insertion path.
            let mut registration = record.clone();
            if registration.origin.is_none()
                && let Some(existing) = ledger
                    .agents
                    .iter()
                    .find(|existing| !existing.ended && existing.actor.same_identity(&record.actor))
            {
                registration.origin = existing.origin.clone();
            }
            insert_record(ledger, &registration, *check)
        }
        Mutation::End { id, actor } => {
            let record = ledger
                .agents
                .iter()
                .find(|record| &record.id == id)
                .ok_or("agent_unavailable")?;
            let parent = record
                .parent
                .as_ref()
                .and_then(|id| resolve_actor(ledger, id));
            if !record.actor.same_identity(caller)
                && !parent.is_some_and(|parent| parent.same_identity(caller))
            {
                return Err("parent_authority_required".into());
            }
            if actor.as_ref().is_some_and(|id| {
                !resolve_actor(ledger, id).is_some_and(|actor| actor.same_identity(caller))
            }) {
                return Err("actor_identity_conflict".into());
            }
            end_record(ledger, id, now);
            Ok(view(
                ledger
                    .agents
                    .iter()
                    .find(|record| &record.id == id)
                    .ok_or("agent_unavailable")?,
                ledger,
            ))
        }
        Mutation::Reserve { parent, command } => {
            let Command::Spawn {
                machine,
                name,
                intent,
                kind,
                repo,
                branch,
                path,
                args,
                launch,
                task,
                brief,
                beside,
                mission,
                ..
            } = command
            else {
                return Err("invalid_spawn".into());
            };
            if brief.as_ref().is_some_and(|brief| {
                brief.trim().is_empty() || brief.len() > crate::delivery::BODY_LIMIT
            }) {
                return Err("invalid_brief".into());
            }
            let mode = if matches!(
                command,
                Command::Spawn {
                    parent: Some(_),
                    ..
                }
            ) {
                SpawnMode::Delegation
            } else {
                SpawnMode::Handoff
            };
            if !resolve_actor(ledger, parent).is_some_and(|parent| parent.same_identity(caller)) {
                return Err("parent_authority_required".into());
            }
            // The screen names a task as Hide's own work, so only Hide's own
            // identity may give one, and only on a handoff.
            if task.as_ref().is_some_and(|task| {
                mode != SpawnMode::Handoff
                    || !caller.hide_owned()
                    || task.target.as_deref().is_some_and(|target| !key(target))
            }) {
                return Err("task_reserved".into());
            }
            // A new start needs a home to show its progress in; only an
            // old record may carry a kind this build does not know (D-19).
            if task.as_ref().is_some_and(|task| task.kind.home().is_none()) {
                return Err("task_kind_unknown".into());
            }
            // The mission's Conductor now may have taken it over from the
            // one that started its workers, as a session that registered
            // only to start this: its workers become its own first, so this
            // start finds the worker it already has (B43).
            if let Some(admission) = mission {
                adopt_mission_workers(ledger, &admission.mission, parent)?;
            }
            // A mission's start asked again after it stopped holding its place
            // takes one again under the cap, or waits, so a start that comes
            // back late never runs one worker past it (engineering #15).
            if let Some(admission) = mission
                && let Some(record) = ledger
                    .spawns
                    .iter()
                    .find(|record| &record.parent == parent && &record.intent == intent)
                && record.mission.as_deref() == Some(admission.mission.as_str())
                && record.child.is_none()
                && !record.completed
                && now.saturating_sub(record.reserved_at_unix_ms) >= MISSION_RESERVATION_HOLD_MS
                && mission_workers(ledger, &admission.mission, now) >= usize::from(admission.cap)
            {
                return Err(refusal("mission_cap_reached", &admission.cap.to_string()));
            }
            if let Some(record) = ledger
                .spawns
                .iter_mut()
                .find(|record| &record.parent == parent && &record.intent == intent)
            {
                // A spawn reserved before tasks existed carries none, and the
                // same intent asked again with one is the same spawn: it
                // takes the task up, so a start in flight across an upgrade
                // converges instead of failing (engineering #11).
                let task_agrees = record.task == *task || record.task.is_none();
                if record.machine != *machine
                    || record.name != *name
                    || hide_agent_adapter::canonical_kind(&record.kind)
                        != hide_agent_adapter::canonical_kind(kind)
                    || record.repo != *repo
                    || record.branch != *branch
                    || record.requested_path != *path
                    || record.mode != mode
                    || record.args != *args
                    || record.launch != *launch
                    || record.brief != *brief
                    || record.beside != *beside
                    || !task_agrees
                {
                    return Err("intent_conflict".into());
                }
                if record.task.is_none() {
                    record.task.clone_from(task);
                }
                if record.mission.is_some() && record.child.is_none() && !record.completed {
                    record.reserved_at_unix_ms = now;
                }
                return Ok(json!(record));
            }
            // A new worker past its mission's cap waits for one to end; the
            // same intent asked again above still answers the spawn it made.
            if let Some(admission) = mission
                && mission_workers(ledger, &admission.mission, now) >= usize::from(admission.cap)
            {
                return Err(refusal("mission_cap_reached", &admission.cap.to_string()));
            }
            if ledger.spawns.len() >= SPAWN_LIMIT {
                return Err("capacity".into());
            }
            let record = SpawnRecord {
                seed_tab: None,
                names: Some(naming::Names::default()),
                id: allocate(ledger, "spawn")?,
                parent: parent.clone(),
                mode,
                machine: machine.clone(),
                intent: intent.clone(),
                name: name.clone(),
                kind: kind.clone(),
                repo: repo.clone(),
                branch: branch.clone(),
                requested_path: path.clone(),
                args: args.clone(),
                launch: launch.clone(),
                path: None,
                pane: None,
                terminal: None,
                typed: 0,
                shell: None,
                child: None,
                completed: false,
                auto_watch_id: None,
                task: task.clone(),
                mission: mission.as_ref().map(|admission| admission.mission.clone()),
                brief: brief.clone(),
                briefed: false,
                beside: *beside,
                reserved_at_unix_ms: now,
            };
            ledger.spawns.push(record.clone());
            Ok(json!(record))
        }
        Mutation::Briefed { id } => {
            let parent = ledger
                .spawns
                .iter()
                .find(|record| &record.id == id)
                .ok_or("spawn_unavailable")?
                .parent
                .clone();
            if !resolve_actor(ledger, &parent).is_some_and(|parent| parent.same_identity(caller)) {
                return Err("parent_authority_required".into());
            }
            let record = ledger
                .spawns
                .iter_mut()
                .find(|record| &record.id == id)
                .ok_or("spawn_unavailable")?;
            if record.pane.is_none() || record.typed == 0 || record.brief.is_none() {
                return Err("spawn_incomplete".into());
            }
            record.briefed = true;
            Ok(json!(record))
        }
        Mutation::Advance {
            seed_tab,
            id,
            path,
            pane,
            terminal,
            shell,
            child,
        } => {
            let parent = ledger
                .spawns
                .iter()
                .find(|record| &record.id == id)
                .ok_or("spawn_unavailable")?
                .parent
                .clone();
            if !resolve_actor(ledger, &parent).is_some_and(|parent| parent.same_identity(caller)) {
                return Err("parent_authority_required".into());
            }
            let record = ledger
                .spawns
                .iter_mut()
                .find(|record| &record.id == id)
                .ok_or("spawn_unavailable")?;
            if [
                (record.path.as_ref(), path.as_ref()),
                (record.pane.as_ref(), pane.as_ref()),
                (record.terminal.as_ref(), terminal.as_ref()),
                (record.child.as_ref(), child.as_ref()),
            ]
            .into_iter()
            .any(|(stored, next)| {
                stored
                    .zip(next)
                    .is_some_and(|(stored, next)| stored != next)
            }) {
                return Err("intent_conflict".into());
            }
            if [
                (record.path.as_ref(), path.as_ref()),
                (record.pane.as_ref(), pane.as_ref()),
                (record.terminal.as_ref(), terminal.as_ref()),
            ]
            .into_iter()
            .any(|(previous, next)| next.is_some() && previous != next)
            {
                ledger.spawn_placement_revision = ledger
                    .spawn_placement_revision
                    .checked_add(1)
                    .ok_or("capacity")?;
            }
            if let Some(seed) = seed_tab {
                if record.seed_tab.as_ref().is_some_and(|old| old != seed) {
                    return Err("intent_conflict".into());
                }
                record.seed_tab = Some(seed.clone());
            }
            // A step forward keeps a mission's start holding its place.
            record.reserved_at_unix_ms = now;
            if path.is_some() {
                record.path = path.clone()
            };
            if pane.is_some() {
                record.pane = pane.clone()
            };
            if terminal.is_some() {
                record.terminal = terminal.clone()
            };
            if shell.is_some() {
                record.shell = shell.clone()
            };
            if child.is_some() {
                record.child = child.clone()
            };
            Ok(json!(record))
        }
        Mutation::Typed { id } => {
            let parent = ledger
                .spawns
                .iter()
                .find(|record| &record.id == id)
                .ok_or("spawn_unavailable")?
                .parent
                .clone();
            if !resolve_actor(ledger, &parent).is_some_and(|parent| parent.same_identity(caller)) {
                return Err("parent_authority_required".into());
            }
            let record = ledger
                .spawns
                .iter_mut()
                .find(|record| &record.id == id)
                .ok_or("spawn_unavailable")?;
            if record.pane.is_none() || record.child.is_some() {
                return Err("spawn_incomplete".into());
            }
            record.typed = record.typed.saturating_add(1);
            Ok(json!(record))
        }
        Mutation::BindChild { id, record } => {
            let spawn = ledger
                .spawns
                .iter()
                .find(|spawn| &spawn.id == id)
                .ok_or("spawn_unavailable")?
                .clone();
            if !resolve_actor(ledger, &spawn.parent)
                .is_some_and(|parent| parent.same_identity(caller))
            {
                return Err("parent_authority_required".into());
            }
            if record.parent != spawn.mode.responsibility(&spawn.parent)
                || record.origin != spawn.mode.origin(&spawn.parent)
                || Some(record.machine.as_str()) != spawn_machine(ledger, &spawn)
                || spawn.pane.as_ref() != Some(&record.pane)
                || record.name.as_deref() != Some(spawn.name.as_str())
                || hide_agent_adapter::canonical_kind(&record.actor.kind)
                    != hide_agent_adapter::canonical_kind(&spawn.kind)
                || record.project != spawn.path
            {
                return Err("child_identity_changed".into());
            }
            validate_registration_name(caller, record)?;
            let existing = if let Some(child) = &spawn.child {
                Some(
                    ledger
                        .agents
                        .iter()
                        .find(|record| &record.id == child)
                        .ok_or("child_unavailable")?,
                )
            } else {
                // Recovery may find a registration installed before an older
                // interrupted binding. Its end state remains authoritative.
                ledger
                    .agents
                    .iter()
                    .find(|old| old.actor.same_identity(&record.actor))
            };
            let child = if let Some(existing) = existing {
                if !existing.actor.same_identity(&record.actor)
                    || existing.parent != record.parent
                    || existing.origin != record.origin
                    || existing.name != record.name
                {
                    return Err("child_identity_changed".into());
                }
                existing.id.clone()
            } else {
                insert_record(ledger, record, false)?["id"]
                    .as_str()
                    .ok_or("child_unavailable")?
                    .to_owned()
            };
            // Registering and binding the child share the worker's one commit,
            // so a retry cannot lose the ended child's identity between them.
            let spawn = ledger
                .spawns
                .iter_mut()
                .find(|spawn| &spawn.id == id)
                .ok_or("spawn_unavailable")?;
            let first_bind = spawn.child.is_none();
            spawn.child = Some(child);
            let spawn = spawn.clone();
            // The brief the agent already read is the first letter under the
            // spawn's intent, delivered: the ledger and the wake-up line
            // carry what the child was started on, and a letter the parent
            // sends later under the same intent is this one.
            if first_bind && let Some(brief) = spawn.brief.as_deref().filter(|_| spawn.briefed) {
                let sender = resolve_actor(ledger, &spawn.parent)
                    .ok_or("parent_unavailable")?
                    .clone();
                let letter = crate::delivery::mailbox::send(
                    ledger,
                    &sender,
                    &record.actor,
                    &spawn.intent,
                    brief,
                    "request",
                    None,
                    now,
                )?;
                let id = letter.id;
                let letter = ledger
                    .letters
                    .iter_mut()
                    .find(|letter| letter.id == id)
                    .ok_or("letter_unavailable")?;
                letter.state = crate::delivery::ledger::State::Delivered;
                letter.hook_confirmed = Some(true);
                letter.bell_sent = true;
            }
            Ok(json!(spawn))
        }
        Mutation::Complete { id } => {
            let spawn = ledger
                .spawns
                .iter()
                .find(|spawn| &spawn.id == id)
                .ok_or("spawn_unavailable")?
                .clone();
            let parent = resolve_actor(ledger, &spawn.parent)
                .ok_or("parent_unavailable")?
                .clone();
            if !parent.same_identity(caller) {
                return Err("parent_authority_required".into());
            }
            if spawn.completed {
                return Ok(json!(spawn));
            }
            if spawn.path.is_none() || spawn.pane.is_none() {
                return Err("spawn_incomplete".into());
            }
            let child = spawn
                .child
                .as_ref()
                .and_then(|id| ledger.agents.iter().find(|child| &child.id == id))
                .ok_or("child_unavailable")?
                .clone();
            // A child can finish while its immediate token write is still
            // returning. Confirmed completion remains authoritative even
            // when the initial watch has not been installed yet.
            let reported_complete = ledger.letters.iter().any(|letter| {
                letter.kind == "report"
                    && letter.sender.same_identity(&child.actor)
                    && letter.recipient.same_identity(&parent)
                    && letter.intake_confirmed()
            });
            let auto_watch_id =
                if spawn.mode == SpawnMode::Delegation && !child.ended && !reported_complete {
                    Some(watch::start(ledger, &parent, &child.actor, now)?.id)
                } else {
                    None
                };
            let spawn = ledger
                .spawns
                .iter_mut()
                .find(|spawn| &spawn.id == id)
                .ok_or("spawn_unavailable")?;
            spawn.completed = true;
            spawn.auto_watch_id = auto_watch_id;
            Ok(json!(spawn))
        }
        Mutation::AwaitIdentity { wait } => identity_wait::reserve(ledger, caller, wait),
        Mutation::RegisterStarted { record } => {
            // The caller vouches only for an agent other than itself, as its
            // origin, never as its parent: nothing watches the start.
            let callers_own = record.origin.as_ref().is_some_and(|id| {
                ledger
                    .agents
                    .iter()
                    .any(|p| &p.id == id && !p.ended && p.actor.same_identity(caller))
            });
            if !callers_own
                || record.parent.is_some()
                || record.forked
                || record.actor.same_identity(caller)
            {
                return Err("parent_authority_required".into());
            }
            validate_registration_name(caller, record)?;
            insert_record(ledger, record, false)
        }
        Mutation::AdoptMissionWorkers { mission, conductor } => {
            if !caller.hide_owned() {
                return Err("parent_authority_required".into());
            }
            let moved = adopt_mission_workers(ledger, mission, conductor)?;
            Ok(json!({"moved": moved}))
        }
    }
}

/// Makes the registration `conductor`, which conducts `mission` now, the
/// parent of each of the mission's worker starts another registration made,
/// with the worker's own record (its parent, or its origin when it was
/// started without one) and the watches on it, so its retries, reports and
/// warnings are the new Conductor's (PRD software-factory-conductor B43).
/// A start it already asked under the same intent keeps its own record.
fn adopt_mission_workers(
    ledger: &mut Ledger,
    mission: &str,
    conductor: &str,
) -> Result<usize, String> {
    let heir = ledger
        .agents
        .iter()
        .find(|record| record.id == conductor && !record.ended)
        .ok_or("agent_unavailable")?
        .actor
        .clone();
    let heir = named(ledger, &heir);
    let mut moved = 0;
    for index in 0..ledger.spawns.len() {
        let spawn = &ledger.spawns[index];
        if spawn.mission.as_deref() != Some(mission) || spawn.parent == conductor {
            continue;
        }
        if ledger
            .spawns
            .iter()
            .any(|other| other.parent == conductor && other.intent == spawn.intent)
        {
            continue;
        }
        let former = spawn.parent.clone();
        let former_actor = ledger
            .agents
            .iter()
            .find(|record| record.id == former)
            .map(|record| record.actor.clone());
        let child = spawn.child.clone();
        let auto_watch = spawn.auto_watch_id.clone();
        ledger.spawns[index].parent = conductor.to_owned();
        moved += 1;
        let Some(child) = child else { continue };
        let mut child_actor = None;
        if let Some(record) = ledger.agents.iter_mut().find(|record| record.id == child) {
            if record.parent.as_ref() == Some(&former) {
                record.parent = Some(conductor.to_owned());
            }
            if record.origin.as_ref() == Some(&former) {
                record.origin = Some(conductor.to_owned());
            }
            child_actor = Some(record.actor.clone());
        }
        for watch in &mut ledger.watches {
            let on_worker = auto_watch.as_ref() == Some(&watch.id)
                || (child_actor
                    .as_ref()
                    .is_some_and(|actor| actor.same_identity(&watch.target))
                    && former_actor
                        .as_ref()
                        .is_some_and(|actor| actor.same_identity(&watch.parent)));
            if on_worker && !watch.parent.same_identity(&heir) {
                watch.generation = watch.generation.saturating_add(1);
                watch.parent = heir.clone();
            }
        }
    }
    if moved > 0 {
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.workers_adopted",
            "mission": mission, "conductor": conductor, "count": moved,
        }));
    }
    Ok(moved)
}

/// The device a spawn's child lives on: the one the caller named, else the
/// caller's own, which its parent registration names.
fn spawn_machine<'a>(ledger: &'a Ledger, spawn: &'a SpawnRecord) -> Option<&'a str> {
    spawn.machine.as_deref().or_else(|| {
        ledger
            .agents
            .iter()
            .find(|parent| parent.id == spawn.parent)
            .map(|parent| parent.machine.as_str())
    })
}

fn validate_registration_name(caller: &Actor, record: &AgentRecord) -> Result<(), String> {
    // Only a Factory registers under its own reserved name (D-14).
    if (record.actor.code_owned() && !record.actor.same_identity(caller))
        || (!record.actor.code_owned()
            && (record
                .name
                .as_deref()
                .is_some_and(crate::delivery::reserved_name)
                || crate::delivery::reserved_name(&record.pane)))
    {
        return Err("reserved_name".into());
    }

    Ok(())
}

fn insert_record(ledger: &mut Ledger, record: &AgentRecord, check: bool) -> Result<Value, String> {
    if let Some(existing) = ledger
        .agents
        .iter()
        .find(|existing| !existing.ended && existing.actor.same_identity(&record.actor))
    {
        if existing.parent != record.parent
            || existing.origin != record.origin
            || existing.name != record.name
        {
            return Err("registration_conflict".into());
        }
        return Ok(view(existing, ledger));
    }
    if ledger.agents.iter().any(|existing| {
        !existing.ended
            && existing.machine == record.machine
            && existing.host_scope == record.host_scope
            && record.name.is_some()
            && existing.name == record.name
    }) {
        return Err("name_in_use".into());
    }
    if check {
        return Ok(json!(RegisterCheck {
            registered: false,
            name: record.name.clone().ok_or("invalid_registration")?,
            pane: record.pane.clone(),
        }));
    }
    if ledger.agents.len() >= AGENT_LIMIT {
        return Err("capacity".into());
    }
    let mut record = record.clone();
    record.id = allocate(ledger, "agent")?;
    ledger.agents.push(record.clone());
    Ok(view(&record, ledger))
}

/// Ends a registration, the watches on it and the answer waits of the letters
/// it sent or received, in one transition. Returns whether it was live.
fn end_record(ledger: &mut Ledger, id: &str, now: u64) -> bool {
    let Some(record) = ledger.agents.iter_mut().find(|record| record.id == id) else {
        return false;
    };
    let live = !std::mem::replace(&mut record.ended, true);
    let target = record.actor.clone();
    ledger
        .watches
        .retain(|watch| !watch.target.same_identity(&target));
    if live {
        ledger.end_answer_waits_of(&target, now);
    }
    live
}

/// Why the core ended a registration whose pane Herdr no longer has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaneGone {
    /// The pane was in the host's previous in-sync read and is not in this
    /// one: Herdr closed it, or moved it to another tab under a new id.
    Left,
    /// The registration was made before this read's snapshot was asked for,
    /// and the snapshot lacks its pane.
    Absent,
}

impl PaneGone {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Left => "pane_left",
            Self::Absent => "pane_absent",
        }
    }
}

/// One host's Herdr panes as its in-sync replica last published them. Only a
/// replica that bootstrapped from a fresh `session.snapshot` and has applied
/// every event since publishes, so a pane missing from it is closed; a
/// replica that lost events or a host that is unreachable publishes nothing,
/// and a stale read only ever lists more panes than exist.
pub(crate) struct PaneRead {
    /// The Herdr socket a local read came from; a remote device has one Herdr.
    pub(crate) host_scope: Option<String>,
    /// The ledger's `next_id` before the snapshot was asked for: a
    /// registration below it existed, with its pane, before that snapshot.
    /// `None` until a snapshot is asked for with the ledger available.
    pub(crate) floor: Option<u64>,
    /// `None` until the snapshot's first publish.
    pub(crate) panes: Option<std::collections::HashSet<String>>,
}

/// The live registrations on `device` whose panes the new read lacks, given
/// the panes of the read before it. A registration made after the snapshot
/// was asked for, whose pane no read has listed yet, is left alone: its pane
/// may be one the replica has not heard about.
pub(crate) fn gone_registrations(
    ledger: &Ledger,
    device: &str,
    read: &PaneRead,
    previous: Option<&std::collections::HashSet<String>>,
) -> Vec<(String, PaneGone)> {
    let Some(panes) = &read.panes else {
        return Vec::new();
    };
    ledger
        .agents
        .iter()
        .filter(|record| {
            !record.ended
                && !record.actor.code_owned()
                && record.actor.device_id == device
                && read
                    .host_scope
                    .as_ref()
                    .is_none_or(|scope| *scope == record.host_scope)
                && !panes.contains(&record.pane)
        })
        .filter_map(|record| {
            if previous.is_some_and(|previous| previous.contains(&record.pane)) {
                return Some((record.id.clone(), PaneGone::Left));
            }
            let sequence = record
                .id
                .strip_prefix("agent-")
                .and_then(|value| value.parse::<u64>().ok())?;
            read.floor
                .is_some_and(|floor| sequence < floor)
                .then(|| (record.id.clone(), PaneGone::Absent))
        })
        .collect()
}

/// Ends each registration the core found gone that is still live, with the
/// watches on it. Returns the ones it ended.
pub(crate) fn end_gone(
    ledger: &mut Ledger,
    gone: &std::collections::BTreeMap<String, PaneGone>,
    now: u64,
) -> Vec<(AgentRecord, PaneGone)> {
    let mut ended = Vec::new();
    for (id, reason) in gone {
        if ledger
            .agents
            .iter()
            .any(|record| &record.id == id && !record.ended)
            && end_record(ledger, id, now)
            && let Some(record) = ledger.agents.iter().find(|record| &record.id == id)
        {
            ended.push((record.clone(), *reason));
        }
    }
    ended
}

/// A sleeping session's registration continued by the pane that woke it.
/// The core started the wake and saved which registration the sleep held,
/// so the registration, its letters and its watch move to the new pane's
/// execution rather than ending with the old pane and being made again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rebind {
    /// The execution the registration was held by when its pane closed.
    pub(crate) from: Actor,
    /// The woken pane's execution, as Herdr reports it now.
    pub(crate) to: Actor,
    /// Herdr's own id of the woken pane, which a local actor spells the same.
    pub(crate) pane: String,
    pub(crate) terminal_id: Option<String>,
}

/// Points each registration at the pane that woke it, with the letters,
/// watches and spawn receipt that address it. Only a live registration still
/// held by `from` moves, and never onto a pane another registration holds.
/// Returns the registrations it moved.
pub(crate) fn rebind(
    ledger: &mut Ledger,
    rebinds: &std::collections::BTreeMap<String, Rebind>,
) -> Vec<(String, Actor)> {
    let mut moved = Vec::new();
    for (id, rebind) in rebinds {
        let held = ledger.agents.iter().any(|record| {
            &record.id == id && !record.ended && record.actor.same_identity(&rebind.from)
        });
        let taken = ledger.agents.iter().any(|record| {
            &record.id != id
                && !record.ended
                && (record.actor.same_identity(&rebind.to) || record.pane == rebind.pane)
        });
        if !held || taken {
            continue;
        }
        let Some(record) = ledger.agents.iter_mut().find(|record| &record.id == id) else {
            continue;
        };
        if record.instance == record.pane {
            record.instance = rebind.pane.clone();
        }
        let old_pane = std::mem::replace(&mut record.pane, rebind.pane.clone());
        record.terminal_id.clone_from(&rebind.terminal_id);
        record.actor = rebind.to.clone();
        for letter in &mut ledger.letters {
            for party in [&mut letter.sender, &mut letter.recipient] {
                if party.same_identity(&rebind.from) {
                    *party = rebind.to.clone();
                }
            }
            if let Some(warning) = letter.watch_warning.as_mut()
                && warning.target.same_identity(&rebind.from)
            {
                warning.target = rebind.to.clone();
            }
        }
        for watch in &mut ledger.watches {
            for party in [&mut watch.parent, &mut watch.target] {
                if party.same_identity(&rebind.from) {
                    *party = rebind.to.clone();
                }
            }
        }
        for spawn in &mut ledger.spawns {
            if spawn.child.as_ref() == Some(id) && spawn.pane.as_deref() == Some(old_pane.as_str())
            {
                spawn.pane = Some(rebind.pane.clone());
            }
        }
        moved.push((id.clone(), rebind.from.clone()));
    }
    moved
}

fn allocate(ledger: &mut Ledger, prefix: &str) -> Result<String, String> {
    let id = ledger.next_id;
    ledger.next_id = id.checked_add(1).ok_or("capacity")?;
    Ok(format!("{prefix}-{id}"))
}

#[cfg(test)]
pub(crate) use executor::{SPAWN, SPAWN_TURN};
pub(crate) use executor::{link_fork, link_started, register_code_owned, run};

#[cfg(test)]
mod tests {
    use super::*;
    fn record(pane: &str, session: &str, parent: Option<String>) -> AgentRecord {
        AgentRecord {
            id: String::new(),
            // A pane id is no name, so a fixture's agent has its own.
            name: Some(format!("{pane}-agent")),
            machine: "local".into(),
            host_scope: "fixture-scope".into(),
            native_machine: "fixture-machine".into(),
            session: session.into(),
            instance: format!("terminal-{pane}"),
            pane: pane.into(),
            parent,
            origin: None,
            project: None,
            actor: Actor {
                pane_id: pane.into(),
                name: pane.into(),
                kind: "codex".into(),
                device_id: "local".into(),
                session: crate::wire::session_digest(session),
            },
            ended: false,
            terminal_id: None,
            forked: false,
        }
    }
    fn register(ledger: &mut Ledger, record: AgentRecord, caller: &Actor) -> String {
        apply(
            ledger,
            caller,
            &Mutation::Register {
                record,
                check: false,
            },
            1,
        )
        .unwrap()["id"]
            .as_str()
            .unwrap()
            .into()
    }
    /// A registration an earlier build made for a parent Herdr never named
    /// holds its pane id as its name. Reading the ledger gives it none, so
    /// it is called by its registration id and by its screen title, never by
    /// the pane, and a name that is not a pane id is left alone.
    #[test]
    fn a_legacy_record_holding_a_pane_id_as_its_name_reads_as_unnamed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("delivery-ledger.json");
        let mut ledger = Ledger::default();
        let (legacy, named) = (
            {
                let mut legacy = record("w9J:p9R", "legacy-native", None);
                legacy.name = Some("w9J:p9R".into());
                legacy
            },
            {
                let mut named = record("w9J:p9S", "named-native", None);
                named.name = Some("lead".into());
                named
            },
        );
        let caller = legacy.actor.clone();
        let legacy_id = register_as(&mut ledger, legacy, &caller).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        // The shape an earlier build wrote.
        assert_eq!(ledger.agents[0].name.as_deref(), Some("w9J:p9R"));
        let caller = named.actor.clone();
        let named_id = register_as(&mut ledger, named, &caller).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        crate::delivery::ledger::save(&path, &ledger).unwrap();
        let loaded = crate::delivery::ledger::load(&path).unwrap();
        let by_id = |id: &str| loaded.agents.iter().find(|record| record.id == id).unwrap();
        assert_eq!(by_id(&legacy_id).name, None);
        assert_eq!(by_id(&legacy_id).wire_name(), legacy_id);
        assert_eq!(by_id(&named_id).wire_name(), "lead");
        assert_eq!(
            wire_name(&loaded, &by_id(&legacy_id).actor),
            legacy_id,
            "the writing side names it the same way"
        );
        assert!(
            resolve_actor(&loaded, "w9J:p9R").is_some(),
            "its pane still addresses it"
        );
    }

    /// A build before this one reads the ledger this one writes: `name` is a
    /// string on every record, the registration id for one with no name.
    #[test]
    fn an_unnamed_record_is_written_in_the_shape_an_older_build_reads() {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct PreviousRecord {
            id: String,
            name: String,
            machine: String,
            host_scope: String,
            native_machine: String,
            session: String,
            instance: String,
            pane: String,
            parent: Option<String>,
            #[serde(default)]
            origin: Option<String>,
            project: Option<String>,
            actor: Actor,
            ended: bool,
            #[serde(default)]
            forked: bool,
        }
        let mut unnamed = record("lead", "lead-native", None);
        unnamed.id = "agent-7".into();
        unnamed.name = None;
        let mut named = record("worker", "worker-native", None);
        named.id = "agent-8".into();
        named.name = Some("worker-agent".into());
        for (record, written) in [(unnamed, "agent-7"), (named, "worker-agent")] {
            let json = serde_json::to_value(&record).unwrap();
            assert_eq!(json["name"], written);
            let previous: PreviousRecord = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(previous.name, written);
            assert_eq!(serde_json::from_value::<AgentRecord>(json).unwrap(), record);
        }
    }

    fn command(intent: &str) -> Command {
        Command::Spawn {
            mission: None,
            parent: Some("here".into()),
            machine: None,
            name: "worker".into(),
            intent: intent.into(),
            kind: "codex".into(),
            repo: "/fixture".into(),
            branch: "topic".into(),
            path: None,
            args: vec!["--model".into(), "fixture model".into()],
            launch: Default::default(),
            task: None,
            brief: None,
            beside: false,
        }
    }
    fn register_as(
        ledger: &mut Ledger,
        record: AgentRecord,
        caller: &Actor,
    ) -> Result<Value, String> {
        apply(
            ledger,
            caller,
            &Mutation::Register {
                record,
                check: false,
            },
            1,
        )
    }

    // PRD fork-operator-root B1, B19: the source registers its fork as the
    // operator's root, and `hide agent show` names the source as its origin.
    #[test]
    fn a_fork_registers_as_a_root_whose_origin_is_its_source() {
        let mut ledger = Ledger::default();
        let source = record("pane-a", "session-a", None);
        let other = record("pane-b", "session-b", None);
        let source_id = register(&mut ledger, source.clone(), &source.actor);
        register(&mut ledger, other.clone(), &other.actor);
        let fork = || {
            let mut fork = record("pane-c", "session-c", None);
            fork.origin = Some(source_id.clone());
            fork.forked = true;
            fork
        };
        // Only the source itself vouches for its fork.
        let before = ledger.clone();
        assert_eq!(
            register_as(&mut ledger, fork(), &other.actor).unwrap_err(),
            "parent_authority_required"
        );
        assert_eq!(ledger, before);
        let id = register(&mut ledger, fork(), &source.actor);
        let fork = ledger.agents.iter().find(|record| record.id == id).unwrap();
        let shown = view(fork, &ledger);
        assert!(shown["parent"].is_null());
        assert_eq!(shown["origin"], source_id);
        // Without the fork's provenance, the source is not the new pane.
        let mut stranger = record("pane-d", "session-d", None);
        stranger.origin = Some(source_id.clone());
        assert_eq!(
            register_as(&mut ledger, stranger, &source.actor).unwrap_err(),
            "parent_authority_required"
        );
    }

    // PRD direct-prompt-observation-widen B14, B17: an agent the caller
    // started through Herdr directly is the operator's root with the caller
    // as its origin, never its child; nobody else vouches for it.
    #[test]
    fn an_observed_start_registers_a_root_whose_origin_is_its_starter() {
        let mut ledger = Ledger::default();
        let starter = record("pane-a", "session-a", None);
        let other = record("pane-b", "session-b", None);
        let starter_id = register(&mut ledger, starter.clone(), &starter.actor);
        let other_id = register(&mut ledger, other.clone(), &other.actor);
        let started = |origin: &str| {
            let mut child = record("pane-c", "session-c", None);
            child.origin = Some(origin.into());
            child
        };
        let start = |ledger: &mut Ledger, record: AgentRecord, caller: &Actor| {
            apply(ledger, caller, &Mutation::RegisterStarted { record }, 1)
        };
        let before = ledger.clone();
        for (record, caller) in [
            (started(&starter_id), &other.actor),
            (started(&other_id), &starter.actor),
            (
                {
                    let mut child = started(&starter_id);
                    child.parent = Some(starter_id.clone());
                    child.origin = None;
                    child
                },
                &starter.actor,
            ),
            (
                {
                    let mut own = starter.clone();
                    own.origin = Some(starter_id.clone());
                    own
                },
                &starter.actor,
            ),
        ] {
            assert_eq!(
                start(&mut ledger, record, caller).unwrap_err(),
                "parent_authority_required"
            );
        }
        assert_eq!(ledger, before);
        let shown = start(&mut ledger, started(&starter_id), &starter.actor).unwrap();
        assert!(shown["parent"].is_null());
        assert_eq!(shown["origin"], starter_id);
        assert!(ledger.watches.is_empty());
        // The same observation again converges on the one record.
        let again = start(&mut ledger, started(&starter_id), &starter.actor).unwrap();
        assert_eq!(again["id"], shown["id"]);
        assert!(validate_records(&ledger).is_ok());
    }

    /// Both ways a registration ends, `hide agent end` and the core finding
    /// its pane gone, end the answer waits of the letters it sent or received.
    #[test]
    fn a_registration_that_ends_ends_the_answer_waits_of_its_letters() {
        for by_pane in [false, true] {
            let mut ledger = Ledger::default();
            let (a, b, c) = (
                record("pane-a", "session-a", None),
                record("pane-b", "session-b", None),
                record("pane-c", "session-c", None),
            );
            register(&mut ledger, a.clone(), &a.actor);
            let b_id = register(&mut ledger, b.clone(), &b.actor);
            register(&mut ledger, c.clone(), &c.actor);
            let ask = |ledger: &mut Ledger, from: &Actor, to: &Actor, intent: &str| {
                let id = crate::delivery::mailbox::send(
                    ledger, from, to, intent, "body", "request", None, 2,
                )
                .unwrap()
                .id;
                // The recipient took it in; a letter still awaiting intake
                // is left to the delivery deadline.
                crate::delivery::mailbox::apply(
                    ledger,
                    to,
                    None,
                    &crate::delivery::Command::Confirm {
                        ids: vec![id.clone()],
                    },
                    2,
                )
                .unwrap();
                id
            };
            let to_b = ask(&mut ledger, &a.actor, &b.actor, "to-b");
            let from_b = ask(&mut ledger, &b.actor, &c.actor, "from-b");
            let elsewhere = ask(&mut ledger, &a.actor, &c.actor, "elsewhere");
            if by_pane {
                let gone = [(b_id, PaneGone::Left)].into();
                assert_eq!(end_gone(&mut ledger, &gone, 10).len(), 1);
            } else {
                apply(
                    &mut ledger,
                    &b.actor,
                    &Mutation::End {
                        id: b_id,
                        actor: None,
                    },
                    10,
                )
                .unwrap();
            }
            let wait = |id: &str| {
                let letter = ledger.letters.iter().find(|l| l.id == id).unwrap();
                (letter.waiting_answer, letter.answer_wait_ended)
            };
            let ended = (
                false,
                Some(crate::delivery::ledger::AnswerWaitEnd::PartyEnded),
            );
            assert_eq!(wait(&to_b), ended, "by_pane={by_pane}");
            assert_eq!(wait(&from_b), ended, "by_pane={by_pane}");
            assert_eq!(wait(&elsewhere), (true, None), "by_pane={by_pane}");
        }
    }

    /// Repeating `hide agent end` on a registration that already ended must not
    /// end the waits of the live registration that took its pane and session.
    #[test]
    fn ending_an_already_ended_registration_again_leaves_the_new_ones_waits() {
        let mut ledger = Ledger::default();
        let (a, b) = (
            record("pane-a", "session-a", None),
            record("pane-b", "session-b", None),
        );
        register(&mut ledger, a.clone(), &a.actor);
        let old = register(&mut ledger, b.clone(), &b.actor);
        let end = |ledger: &mut Ledger, id: &str, now: u64| {
            apply(
                ledger,
                &b.actor,
                &Mutation::End {
                    id: id.into(),
                    actor: None,
                },
                now,
            )
            .unwrap()
        };
        end(&mut ledger, &old, 3);
        let renewed = register(&mut ledger, b.clone(), &b.actor);
        assert_ne!(renewed, old);
        let id = crate::delivery::mailbox::send(
            &mut ledger,
            &a.actor,
            &b.actor,
            "ask",
            "body",
            "request",
            None,
            4,
        )
        .unwrap()
        .id;
        crate::delivery::mailbox::apply(
            &mut ledger,
            &b.actor,
            None,
            &crate::delivery::Command::Confirm {
                ids: vec![id.clone()],
            },
            4,
        )
        .unwrap();
        end(&mut ledger, &old, 5);
        let letter = ledger.letters.iter().find(|l| l.id == id).unwrap();
        assert!(letter.waiting_answer && letter.answer_wait_ended.is_none());
        end(&mut ledger, &renewed, 6);
        assert!(!ledger.letters[0].waiting_answer);
    }

    /// `here` is the caller's own live registration, and a caller with
    /// none learns why: ended, its pane's session moved on, or never there.
    #[test]
    fn here_is_the_callers_own_live_registration() {
        let mut ledger = Ledger::default();
        let observer = record("observer", "observer-session", None);
        let caller = observer.actor.clone();
        assert_eq!(
            here(&ledger, &caller).err(),
            Some("participant_unavailable")
        );
        let id = register(&mut ledger, observer, &caller);
        let child = record("child", "child-session", Some(id.clone()));
        let child_actor = child.actor.clone();
        register(&mut ledger, child, &caller);
        assert_eq!(here(&ledger, &caller).unwrap().id, id);
        assert_eq!(
            here(&ledger, &child_actor).unwrap().name.as_deref(),
            Some("child-agent")
        );
        // A remote participant on a pane of the same name is not this caller.
        let mut remote = caller.clone();
        remote.device_id = "mini".into();
        assert_eq!(
            here(&ledger, &remote).err(),
            Some("participant_unavailable")
        );
        // The pane's session moved on: its record belongs to the old one.
        let mut handed_off = caller.clone();
        handed_off.session = crate::wire::session_digest("observer-after-compaction");
        assert_eq!(
            here(&ledger, &handed_off).err(),
            Some("participant_session_changed")
        );
        ledger
            .agents
            .iter_mut()
            .find(|record| record.id == id)
            .unwrap()
            .ended = true;
        assert_eq!(here(&ledger, &caller).err(), Some("participant_ended"));
        let mut twice = ledger.agents[1].clone();
        twice.id = "agent-copy".into();
        ledger.agents.push(twice);
        assert_eq!(
            here(&ledger, &child_actor).err(),
            Some("ambiguous_participant")
        );
    }

    #[test]
    fn registration_check_is_read_only_and_execution_identity_replays_after_reload() {
        let mut ledger = Ledger::default();
        let parent = record("parent", "native-parent", None);
        let actor = parent.actor.clone();
        let before = ledger.clone();
        apply(
            &mut ledger,
            &actor,
            &Mutation::Register {
                record: parent.clone(),
                check: true,
            },
            1,
        )
        .unwrap();
        assert_eq!(ledger, before);
        let id = register(&mut ledger, parent.clone(), &actor);
        assert_eq!(resolve_actor(&ledger, &id), Some(&actor));
        let bytes = ledger.bytes().unwrap();
        let mut restored: Ledger = serde_json::from_slice(&bytes).unwrap();
        restored.validate().unwrap();
        assert_eq!(register(&mut restored, parent, &actor), id);
        assert_eq!(restored.agents.len(), 1);
        let child = record("child", "native-child", Some(id.clone()));
        let child_id = register(&mut restored, child, &actor);
        assert_eq!(
            restored
                .agents
                .iter()
                .find(|record| record.id == child_id)
                .unwrap()
                .parent,
            Some(id)
        );
    }
    /// A mission's workers past its cap are refused with the cap named,
    /// while asking again for a worker it already has still answers it, and
    /// an ended worker frees its place (PRD software-factory-conductor B12).
    #[test]
    fn a_mission_spawn_past_its_cap_is_refused_and_an_ended_worker_frees_a_place() {
        let mut ledger = Ledger::default();
        let conductor = record("conductor", "native-conductor", None);
        let caller = conductor.actor.clone();
        let id = register(&mut ledger, conductor, &caller);
        let worker = |intent: &str| {
            let mut spawn = command(intent);
            if let Command::Spawn { mission: slot, .. } = &mut spawn {
                *slot = Some(crate::runtime::missions::MissionAdmission {
                    mission: "mission-1".into(),
                    cap: 1,
                });
            }
            Mutation::Reserve {
                parent: id.clone(),
                command: spawn,
            }
        };
        let first = apply(&mut ledger, &caller, &worker("mission-1:a"), 2).unwrap();
        assert_eq!(first["mission"], "mission-1");
        assert_eq!(mission_workers(&ledger, "mission-1", 6), 1);
        let refused = apply(&mut ledger, &caller, &worker("mission-1:b"), 3).unwrap_err();
        assert_eq!(split_refusal(&refused), ("mission_cap_reached", Some("1")));
        assert_eq!(
            apply(&mut ledger, &caller, &worker("mission-1:a"), 4).unwrap()["id"],
            first["id"]
        );
        // Another mission and an ordinary spawn are not counted here.
        assert_eq!(mission_workers(&ledger, "mission-2", 6), 0);
        apply(
            &mut ledger,
            &caller,
            &Mutation::Reserve {
                parent: id.clone(),
                command: command("plain"),
            },
            5,
        )
        .unwrap();
        assert_eq!(mission_workers(&ledger, "mission-1", 6), 1);
        // The worker came up and then ended: its place is free again.
        let child = register(
            &mut ledger,
            record("child", "native-child", Some(id.clone())),
            &caller,
        );
        let spawn = ledger
            .spawns
            .iter_mut()
            .find(|spawn| spawn.intent == "mission-1:a")
            .unwrap();
        spawn.child = Some(child.clone());
        spawn.completed = true;
        assert_eq!(mission_workers(&ledger, "mission-1", 6), 1);
        ledger
            .agents
            .iter_mut()
            .find(|agent| agent.id == child)
            .unwrap()
            .ended = true;
        assert_eq!(mission_workers(&ledger, "mission-1", 6), 0);
        apply(&mut ledger, &caller, &worker("mission-1:b"), 6).unwrap();
        // A start that never produced its worker stops holding the place
        // once it is long past, so a retry under a new intent can start.
        let later = 6 + MISSION_RESERVATION_HOLD_MS;
        assert_eq!(mission_workers(&ledger, "mission-1", later - 1), 1);
        assert_eq!(mission_workers(&ledger, "mission-1", later), 0);
        apply(&mut ledger, &caller, &worker("mission-1:c"), later).unwrap();
        // The start that stopped holding its place comes back while `c` holds
        // the one place: it waits rather than run a second worker, and takes
        // the place again once `c` is gone.
        assert_eq!(
            split_refusal(&apply(&mut ledger, &caller, &worker("mission-1:b"), later).unwrap_err()),
            ("mission_cap_reached", Some("1"))
        );
        let gone = later + MISSION_RESERVATION_HOLD_MS;
        apply(&mut ledger, &caller, &worker("mission-1:b"), gone).unwrap();
        assert_eq!(mission_workers(&ledger, "mission-1", gone), 1);
    }
    /// A session that takes a mission over gets its workers: asking a start
    /// again under the same intent answers the worker it already has, the
    /// worker is its child, and the watch on the worker, which a report
    /// ends and warnings go to, is its own (B43).
    #[test]
    fn a_mission_taken_over_hands_its_workers_to_the_new_conductor() {
        let mut ledger = Ledger::default();
        let former = record("former", "native-former", None);
        let former_actor = former.actor.clone();
        let former_id = register(&mut ledger, former, &former_actor);
        let worker_spawn = || {
            let mut spawn = command("mission-1:api");
            if let Command::Spawn { mission: slot, .. } = &mut spawn {
                *slot = Some(crate::runtime::missions::MissionAdmission {
                    mission: "mission-1".into(),
                    cap: 2,
                });
            }
            spawn
        };
        let first = apply(
            &mut ledger,
            &former_actor,
            &Mutation::Reserve {
                parent: former_id.clone(),
                command: worker_spawn(),
            },
            2,
        )
        .unwrap();
        let child = register(
            &mut ledger,
            record("worker", "native-worker", Some(former_id.clone())),
            &former_actor,
        );
        let child_actor = ledger
            .agents
            .iter()
            .find(|agent| agent.id == child)
            .unwrap()
            .actor
            .clone();
        let watch = watch::start(&mut ledger, &former_actor, &child_actor, 3).unwrap();
        let spawn = ledger
            .spawns
            .iter_mut()
            .find(|spawn| spawn.intent == "mission-1:api")
            .unwrap();
        spawn.child = Some(child.clone());
        spawn.completed = true;
        spawn.auto_watch_id = Some(watch.id.clone());
        // The former Conductor's pane is gone; another session registers.
        ledger
            .agents
            .iter_mut()
            .find(|agent| agent.id == former_id)
            .unwrap()
            .ended = true;
        let heir = record("heir", "native-heir", None);
        let heir_actor = heir.actor.clone();
        let heir_id = register(&mut ledger, heir, &heir_actor);

        let adopt = Mutation::AdoptMissionWorkers {
            mission: "mission-1".into(),
            conductor: heir_id.clone(),
        };
        assert_eq!(
            apply(&mut ledger, &heir_actor, &adopt, 4).unwrap_err(),
            "parent_authority_required",
            "only Hide hands workers over"
        );
        let hide = Actor::hide("conductor", "node");
        assert_eq!(apply(&mut ledger, &hide, &adopt, 4).unwrap()["moved"], 1);

        let again = apply(
            &mut ledger,
            &heir_actor,
            &Mutation::Reserve {
                parent: heir_id.clone(),
                command: worker_spawn(),
            },
            5,
        )
        .unwrap();
        assert_eq!(again["id"], first["id"], "the same worker, not a second");
        assert_eq!(again["child"], child.as_str());
        let worker = ledger
            .agents
            .iter()
            .find(|agent| agent.id == child)
            .unwrap();
        assert_eq!(worker.parent.as_deref(), Some(heir_id.as_str()));
        let moved = ledger.watches.iter().find(|w| w.id == watch.id).unwrap();
        assert!(moved.parent.same_identity(&heir_actor));
        assert!(moved.generation > watch.generation);
        // Asked again, nothing moves twice.
        assert_eq!(apply(&mut ledger, &hide, &adopt, 6).unwrap()["moved"], 0);
    }

    /// The session that took a mission over may register only to start a
    /// worker: that start hands it the workers first, one started without a
    /// parent included, so a retry under the same intent is the worker it
    /// already has and the ledger stays valid.
    #[test]
    fn a_new_conductors_first_start_takes_the_missions_workers() {
        let mut ledger = Ledger::default();
        let former = record("former", "native-former", None);
        let former_actor = former.actor.clone();
        let former_id = register(&mut ledger, former, &former_actor);
        let handoff = |mission: &str| {
            let mut spawn = command("mission-1:docs");
            if let Command::Spawn {
                mission: slot,
                parent,
                ..
            } = &mut spawn
            {
                *parent = None;
                *slot = Some(crate::runtime::missions::MissionAdmission {
                    mission: mission.into(),
                    cap: 2,
                });
            }
            spawn
        };
        let first = apply(
            &mut ledger,
            &former_actor,
            &Mutation::Reserve {
                parent: former_id.clone(),
                command: handoff("mission-1"),
            },
            2,
        )
        .unwrap();
        let child_record = record("docs", "native-docs", None);
        let child_actor = child_record.actor.clone();
        let child = register(&mut ledger, child_record, &child_actor);
        ledger
            .agents
            .iter_mut()
            .find(|agent| agent.id == child)
            .unwrap()
            .origin = Some(former_id.clone());
        let spawn = ledger
            .spawns
            .iter_mut()
            .find(|spawn| spawn.intent == "mission-1:docs")
            .unwrap();
        spawn.child = Some(child.clone());
        spawn.pane = Some("docs".into());
        spawn.path = Some("/fixture/docs".into());
        spawn.completed = true;
        validate_records(&ledger).unwrap();
        ledger
            .agents
            .iter_mut()
            .find(|agent| agent.id == former_id)
            .unwrap()
            .ended = true;
        let heir = record("heir", "native-heir", None);
        let heir_actor = heir.actor.clone();
        let heir_id = register(&mut ledger, heir, &heir_actor);

        let again = apply(
            &mut ledger,
            &heir_actor,
            &Mutation::Reserve {
                parent: heir_id.clone(),
                command: handoff("mission-1"),
            },
            3,
        )
        .unwrap();
        assert_eq!(again["id"], first["id"]);
        let worker = ledger
            .agents
            .iter()
            .find(|agent| agent.id == child)
            .unwrap();
        assert_eq!(worker.origin.as_deref(), Some(heir_id.as_str()));
        validate_records(&ledger).unwrap();
    }

    #[test]
    fn a_parent_intent_has_one_durable_child_and_other_parents_have_their_own() {
        let mut ledger = Ledger::default();
        let one = record("one", "native-one", None);
        let caller_one = one.actor.clone();
        let id_one = register(&mut ledger, one, &caller_one);
        let two = record("two", "native-two", None);
        let caller_two = two.actor.clone();
        let id_two = register(&mut ledger, two, &caller_two);
        let reserve = Mutation::Reserve {
            parent: id_one.clone(),
            command: command("same"),
        };
        let first = apply(&mut ledger, &caller_one, &reserve, 2).unwrap();
        let bytes = ledger.bytes().unwrap();
        let mut restored: Ledger = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            apply(&mut restored, &caller_one, &reserve, 3).unwrap(),
            first
        );
        let second = apply(
            &mut restored,
            &caller_two,
            &Mutation::Reserve {
                parent: id_two,
                command: command("same"),
            },
            3,
        )
        .unwrap();
        assert_ne!(first["id"], second["id"]);
        assert_eq!(restored.spawns.len(), 2);
        let id = first["id"].as_str().unwrap().to_owned();
        apply(
            &mut restored,
            &caller_one,
            &Mutation::Advance {
                seed_tab: None,
                id: id.clone(),
                path: Some("/fixture/topic".into()),
                pane: Some("child-pane".into()),
                terminal: None,
                shell: None,
                child: None,
            },
            4,
        )
        .unwrap();
        assert_eq!(
            apply(&mut restored, &caller_one, &reserve, 5).unwrap()["pane"],
            "child-pane"
        );
        assert!(
            apply(
                &mut restored,
                &caller_two,
                &Mutation::Advance {
                    seed_tab: None,
                    id,
                    path: None,
                    pane: Some("wrong".into()),
                    terminal: None,
                    shell: None,
                    child: None
                },
                5
            )
            .is_err()
        );
    }
    #[test]
    fn ending_a_child_requires_its_parent_and_stops_the_matching_watch() {
        let mut ledger = Ledger::default();
        let parent = record("parent", "native-parent", None);
        let actor = parent.actor.clone();
        let id = register(&mut ledger, parent, &actor);
        let child = record("child", "native-child", Some(id.clone()));
        let child_id = register(&mut ledger, child, &actor);
        let target = resolve_actor(&ledger, &child_id).unwrap().clone();
        watch::start(&mut ledger, &actor, &target, 2).unwrap();
        assert_eq!(ledger.watches.len(), 1);
        let stranger = record("stranger", "native-stranger", None).actor;
        assert!(
            apply(
                &mut ledger,
                &stranger,
                &Mutation::End {
                    id: child_id.clone(),
                    actor: None
                },
                3
            )
            .is_err()
        );
        apply(
            &mut ledger,
            &actor,
            &Mutation::End {
                id: child_id.clone(),
                actor: Some(id),
            },
            3,
        )
        .unwrap();
        assert!(ledger.watches.is_empty());
        assert!(resolve_actor(&ledger, &child_id).is_none());
        assert!(
            ledger
                .agents
                .iter()
                .find(|record| record.id == child_id)
                .unwrap()
                .ended
        );
    }
    #[test]
    fn changed_intent_payload_or_invalid_persisted_session_is_refused() {
        let mut ledger = Ledger::default();
        let parent = record("parent", "session", None);
        let actor = parent.actor.clone();
        let id = register(&mut ledger, parent, &actor);
        apply(
            &mut ledger,
            &actor,
            &Mutation::Reserve {
                parent: id.clone(),
                command: command("key"),
            },
            1,
        )
        .unwrap();
        let mut changed = command("key");
        if let Command::Spawn { parent, .. } = &mut changed {
            *parent = None;
        }
        assert_eq!(
            apply(
                &mut ledger,
                &actor,
                &Mutation::Reserve {
                    parent: id,
                    command: changed
                },
                1
            )
            .unwrap_err(),
            "intent_conflict"
        );
        ledger.agents[0].session = "replaced".into();
        assert!(ledger.bytes().is_err());
    }
    fn pending_spawn() -> (Ledger, Actor, String, String, AgentRecord) {
        pending_spawn_mode(SpawnMode::Delegation, "local")
    }

    fn pending_spawn_mode(
        mode: SpawnMode,
        device: &str,
    ) -> (Ledger, Actor, String, String, AgentRecord) {
        let mut ledger = Ledger::default();
        let mut parent = record("parent", "native-parent", None);
        parent.machine = device.into();
        parent.actor.device_id = device.into();
        let actor = parent.actor.clone();
        let parent_id = register(&mut ledger, parent, &actor);
        let mut command = command("once");
        if mode == SpawnMode::Handoff
            && let Command::Spawn { parent, .. } = &mut command
        {
            *parent = None;
        }
        let reserved = apply(
            &mut ledger,
            &actor,
            &Mutation::Reserve {
                parent: parent_id.clone(),
                command,
            },
            1,
        )
        .unwrap();
        let id = reserved["id"].as_str().unwrap().to_owned();
        apply(
            &mut ledger,
            &actor,
            &Mutation::Advance {
                seed_tab: None,
                id: id.clone(),
                path: Some("/fixture/topic".into()),
                pane: Some("worker-pane".into()),
                terminal: None,
                shell: None,
                child: None,
            },
            2,
        )
        .unwrap();
        let mut child = record(
            "worker-pane",
            "native-child",
            mode.responsibility(&parent_id),
        );
        child.name = Some("worker".into());
        child.origin = mode.origin(&parent_id);
        child.machine = device.into();
        child.actor.device_id = device.into();
        child.project = Some("/fixture/topic".into());
        (ledger, actor, parent_id, id, child)
    }

    /// A spawn's brief is part of its intent, is marked submitted once so a
    /// retry never types it twice, and becomes the child's first letter when
    /// the child binds: delivered and confirmed under the spawn's intent,
    /// from the parent, so the ledger and a later wake carry it.
    #[test]
    fn a_spawns_brief_is_one_delivered_letter_under_its_intent() {
        let mut ledger = Ledger::default();
        let parent = record("parent", "native-parent", None);
        let actor = parent.actor.clone();
        let parent_id = register(&mut ledger, parent, &actor);
        let briefed = |brief: &str| {
            let mut command = command("mission-1:api");
            if let Command::Spawn { brief: slot, .. } = &mut command {
                *slot = Some(brief.into());
            }
            command
        };
        let reserve = |ledger: &mut Ledger, command: Command, now: u64| {
            apply(
                ledger,
                &actor,
                &Mutation::Reserve {
                    parent: parent_id.clone(),
                    command,
                },
                now,
            )
        };
        assert_eq!(
            reserve(&mut ledger, briefed("  \n"), 1).unwrap_err(),
            "invalid_brief"
        );
        assert_eq!(
            reserve(
                &mut ledger,
                briefed(&"x".repeat(crate::delivery::BODY_LIMIT + 1)),
                1
            )
            .unwrap_err(),
            "invalid_brief"
        );
        let reserved = reserve(
            &mut ledger,
            briefed("Node api: add the call.\nReport by letter."),
            1,
        )
        .unwrap();
        let spawn = reserved["id"].as_str().unwrap().to_owned();
        assert_eq!(
            reserved["brief"],
            "Node api: add the call.\nReport by letter."
        );
        assert!(
            reserved["briefed"].is_null(),
            "not submitted yet: {reserved}"
        );
        assert_eq!(
            reserve(&mut ledger, briefed("Another brief."), 2).unwrap_err(),
            "intent_conflict"
        );
        assert_eq!(
            reserve(&mut ledger, command("mission-1:api"), 2).unwrap_err(),
            "intent_conflict",
            "the same intent without the brief is another spawn"
        );
        // Nothing is marked submitted before the agent was started.
        assert_eq!(
            apply(
                &mut ledger,
                &actor,
                &Mutation::Briefed { id: spawn.clone() },
                2
            )
            .unwrap_err(),
            "spawn_incomplete"
        );
        apply(
            &mut ledger,
            &actor,
            &Mutation::Advance {
                seed_tab: None,
                id: spawn.clone(),
                path: Some("/fixture/topic".into()),
                pane: Some("worker-pane".into()),
                terminal: None,
                shell: None,
                child: None,
            },
            2,
        )
        .unwrap();
        apply(
            &mut ledger,
            &actor,
            &Mutation::Typed { id: spawn.clone() },
            3,
        )
        .unwrap();
        let marked = apply(
            &mut ledger,
            &actor,
            &Mutation::Briefed { id: spawn.clone() },
            3,
        )
        .unwrap();
        assert_eq!(marked["briefed"], true);
        assert!(
            ledger.letters.is_empty(),
            "no letter before the child is known"
        );

        let mut child = record("worker-pane", "native-child", Some(parent_id.clone()));
        child.name = Some("worker".into());
        child.project = Some("/fixture/topic".into());
        let bound = apply(
            &mut ledger,
            &actor,
            &Mutation::BindChild {
                id: spawn.clone(),
                record: child.clone(),
            },
            4,
        )
        .unwrap();
        let child_id = bound["child"].as_str().unwrap().to_owned();
        assert_eq!(ledger.letters.len(), 1);
        let letter = &ledger.letters[0];
        assert_eq!(letter.intent, "mission-1:api");
        assert_eq!(letter.kind, "request");
        assert_eq!(letter.body, "Node api: add the call.\nReport by letter.");
        assert!(letter.sender.same_identity(&actor));
        assert!(letter.recipient.same_identity(&child.actor));
        assert_eq!(letter.state, crate::delivery::ledger::State::Delivered);
        assert_eq!(letter.hook_confirmed, Some(true));
        assert!(letter.bell_sent);
        assert_eq!(letter.created_at_unix_ms, 4);
        // Bound again on a retry, the letter is not made twice.
        let again = apply(
            &mut ledger,
            &actor,
            &Mutation::BindChild {
                id: spawn.clone(),
                record: child,
            },
            5,
        )
        .unwrap();
        assert_eq!(again["child"], child_id);
        assert_eq!(ledger.letters.len(), 1);
        // A spawn without a brief makes no letter.
        let (mut plain, caller, _, plain_spawn, plain_child) = pending_spawn();
        apply(
            &mut plain,
            &caller,
            &Mutation::BindChild {
                id: plain_spawn,
                record: plain_child,
            },
            3,
        )
        .unwrap();
        assert!(plain.letters.is_empty());
        ledger.validate().unwrap();
    }

    #[test]
    fn both_spawn_modes_bind_and_replay_known_agent_spellings() {
        // B5/D-08 applies to native observations and retries of the same intent.
        for mode in [SpawnMode::Delegation, SpawnMode::Handoff] {
            for (requested, reported) in [
                ("CODEX", "codex"),
                ("codex", " Codex"),
                ("claude_code", "CLAUDE"),
            ] {
                let (mut ledger, caller, parent, spawn, mut child) =
                    pending_spawn_mode(mode, "local");
                ledger.spawns[0].kind = requested.into();
                child.actor.kind = reported.into();
                let bound = apply(
                    &mut ledger,
                    &caller,
                    &Mutation::BindChild {
                        id: spawn.clone(),
                        record: child,
                    },
                    3,
                )
                .unwrap();
                let registered = ledger
                    .agents
                    .iter()
                    .find(|record| Some(record.id.as_str()) == bound["child"].as_str())
                    .unwrap();
                assert_eq!(registered.actor.kind, reported);
                assert_eq!(registered.parent, mode.responsibility(&parent));
                assert_eq!(registered.origin, mode.origin(&parent));
                let mut retry = command("once");
                if let Command::Spawn { kind, parent, .. } = &mut retry {
                    *kind = reported.into();
                    if mode == SpawnMode::Handoff {
                        *parent = None;
                    }
                }
                let replay = apply(
                    &mut ledger,
                    &caller,
                    &Mutation::Reserve {
                        parent,
                        command: retry,
                    },
                    4,
                )
                .unwrap();
                assert_eq!(replay["id"], spawn);
                assert_eq!(replay["kind"], requested);
                assert_eq!(ledger.spawns.len(), 1);
            }
        }
    }

    /// Pi and omp name a session by its file, in a folder named after the
    /// checkout's path, so a child in a deep checkout reports a session far
    /// past a key's 256 bytes; it binds and reads back like any other, up to
    /// Herdr's own 4096-byte bound for a session path.
    #[test]
    fn a_child_whose_session_is_a_deep_session_file_binds_and_reads_back() {
        let file = "2026-10-18T00-00-00-000Z_0199f0aa-1111-7222-8333-444455556666.jsonl";
        let folder = "--fixture-home-projects-a-very-deep-checkout-path--";
        let deep = format!("/fixture/home/.pi/agent/sessions/{folder}/{file}");
        let longest = format!(
            "/{}/{file}",
            "x".repeat(crate::delivery::SESSION_LIMIT - file.len() - 2)
        );
        for session in [
            deep.clone(),
            format!("{}{deep}", "/deep".repeat(60)),
            longest,
        ] {
            let (mut ledger, caller, parent, spawn, mut child) = pending_spawn();
            ledger.spawns[0].kind = "pi".into();
            child.session = session.clone();
            child.actor.kind = "pi".into();
            child.actor.session = crate::wire::session_digest(&session);
            let bound = apply(
                &mut ledger,
                &caller,
                &Mutation::BindChild {
                    id: spawn,
                    record: child,
                },
                3,
            )
            .unwrap_or_else(|error| panic!("{} bytes: {error}", session.len()));
            let bytes = ledger
                .bytes()
                .unwrap_or_else(|error| panic!("{} bytes: {error}", session.len()));
            let mut read: Ledger = serde_json::from_slice(&bytes).unwrap();
            read.validate().unwrap();
            let registered = read
                .agents
                .iter_mut()
                .find(|record| Some(record.id.as_str()) == bound["child"].as_str())
                .unwrap();
            assert_eq!(registered.session, session);
            assert_eq!(registered.parent.as_ref(), Some(&parent));
            let past = format!("{session}{}", "x".repeat(crate::delivery::SESSION_LIMIT));
            registered.session = past[..=crate::delivery::SESSION_LIMIT].to_owned();
            registered.actor.session = crate::wire::session_digest(&registered.session);
            assert!(read.bytes().is_err());
        }
    }

    /// A kind this build does not know keeps only a short printable form.
    #[test]
    fn an_unknown_task_kind_is_kept_short_and_printable() {
        let long = format!("future\u{7}_kind\n{}", "x".repeat(200));
        let TaskKind::Unknown(kept) = TaskKind::from_code(&long) else {
            panic!("a kind with no home is unknown");
        };
        assert_eq!(kept.chars().count(), 64);
        assert!(kept.starts_with("future_kind"));
        assert!(!kept.chars().any(char::is_control));
        assert_eq!(
            TaskKind::from_code("memory_promote"),
            TaskKind::MemoryPromote
        );
    }

    #[test]
    fn spawn_binding_keeps_other_agents_and_unknown_spellings_distinct() {
        for mode in [SpawnMode::Delegation, SpawnMode::Handoff] {
            for (requested, reported) in [("codex", "claude"), ("future", "FUTURE")] {
                let (mut ledger, caller, parent, spawn, mut child) =
                    pending_spawn_mode(mode, "local");
                ledger.spawns[0].kind = requested.into();
                child.actor.kind = reported.into();
                assert_eq!(
                    apply(
                        &mut ledger,
                        &caller,
                        &Mutation::BindChild {
                            id: spawn,
                            record: child,
                        },
                        3
                    ),
                    Err("child_identity_changed".into())
                );
                assert_eq!(ledger.agents.len(), 1);
                assert!(ledger.spawns[0].child.is_none());
                let mut retry = command("once");
                if let Command::Spawn { kind, parent, .. } = &mut retry {
                    *kind = reported.into();
                    if mode == SpawnMode::Handoff {
                        *parent = None;
                    }
                }
                assert_eq!(
                    apply(
                        &mut ledger,
                        &caller,
                        &Mutation::Reserve {
                            parent,
                            command: retry
                        },
                        4
                    ),
                    Err("intent_conflict".into())
                );
            }
        }
    }

    #[test]
    fn pane_retirement_preserves_spawn_receipts_and_responsibility() {
        for device in ["local", "connected-device"] {
            for mode in [SpawnMode::Delegation, SpawnMode::Handoff] {
                let (mut ledger, caller, origin, spawn, child) = pending_spawn_mode(mode, device);
                let expected_parent = match mode {
                    SpawnMode::Delegation => Some(origin.clone()),
                    SpawnMode::Handoff => None,
                };
                let scope = child.host_scope.clone();
                let child_actor = child.actor.clone();
                let bound = apply(
                    &mut ledger,
                    &caller,
                    &Mutation::BindChild {
                        id: spawn.clone(),
                        record: child,
                    },
                    3,
                )
                .unwrap();
                let child_id = bound["child"].as_str().unwrap().to_owned();
                let complete = Mutation::Complete { id: spawn };
                let receipt = apply(&mut ledger, &caller, &complete, 4).unwrap();
                assert_eq!(
                    receipt["auto_watch_id"].is_null(),
                    mode == SpawnMode::Handoff
                );
                if mode == SpawnMode::Handoff {
                    watch::start(&mut ledger, &caller, &child_actor, 5).unwrap();
                }
                let live = ledger.clone();
                let read = PaneRead {
                    host_scope: Some(scope.clone()),
                    floor: Some(ledger.next_id),
                    panes: Some([caller.pane_id.clone()].into()),
                };
                let gone = gone_registrations(&ledger, device, &read, None)
                    .into_iter()
                    .collect();
                assert_eq!(end_gone(&mut ledger, &gone, 1).len(), 1);
                let ended = ledger.agents.iter().find(|r| r.id == child_id).unwrap();
                assert!(ended.ended);
                assert_eq!(ended.parent, expected_parent);
                assert_eq!(view(ended, &ledger)["origin"], origin);
                assert!(ledger.watches.is_empty());
                assert!(!ledger.agents.iter().find(|r| r.id == origin).unwrap().ended);
                let mut restored: Ledger =
                    serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
                let before = restored.clone();
                assert_eq!(
                    apply(&mut restored, &caller, &complete, 6).unwrap(),
                    receipt
                );
                assert_eq!(restored, before);

                // Losing the spawner's pane ends that registration alone;
                // provenance and responsibility are not cascade authority.
                let mut ledger = live;
                let read = PaneRead {
                    host_scope: Some(scope),
                    floor: Some(ledger.next_id),
                    panes: Some([child_actor.pane_id.clone()].into()),
                };
                let gone = gone_registrations(&ledger, device, &read, None)
                    .into_iter()
                    .collect();
                assert_eq!(end_gone(&mut ledger, &gone, 1).len(), 1);
                assert!(ledger.agents.iter().find(|r| r.id == origin).unwrap().ended);
                let child = ledger.agents.iter().find(|r| r.id == child_id).unwrap();
                assert!(!child.ended);
                assert_eq!(child.parent, expected_parent);
                assert_eq!(view(child, &ledger)["origin"], origin);
                assert_eq!(ledger.watches.len(), 1);
            }
        }
    }

    #[test]
    fn handoff_is_an_independent_root_with_provenance_and_only_self_end_authority() {
        use crate::delivery::mailbox;
        for device in ["local", "connected-device"] {
            let (ledger, actor, origin, spawn, child) =
                pending_spawn_mode(SpawnMode::Handoff, device);
            let child_actor = child.actor.clone();
            let mut ledger: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
            let before = ledger.clone();
            assert_eq!(
                apply(
                    &mut ledger,
                    &actor,
                    &Mutation::Reserve {
                        parent: origin.clone(),
                        command: command("once")
                    },
                    3
                )
                .unwrap_err(),
                "intent_conflict"
            );
            assert_eq!(ledger, before);
            let bind = Mutation::BindChild {
                id: spawn.clone(),
                record: child,
            };
            let bound = apply(&mut ledger, &actor, &bind, 3).unwrap();
            let child = bound["child"].as_str().unwrap().to_owned();
            let complete = Mutation::Complete { id: spawn };
            let receipt = apply(&mut ledger, &actor, &complete, 4).unwrap();
            assert!(receipt["auto_watch_id"].is_null());
            let child_view = view(
                ledger
                    .agents
                    .iter()
                    .find(|record| record.id == child)
                    .unwrap(),
                &ledger,
            );
            assert!(child_view["parent"].is_null());
            assert_eq!(child_view["origin"], origin);
            assert!(child_view["watch"].is_null());
            assert!(view(&ledger.agents[0], &ledger)["origin"].is_null());
            let before = ledger.clone();
            assert_eq!(
                apply(
                    &mut ledger,
                    &actor,
                    &Mutation::End {
                        id: child.clone(),
                        actor: None
                    },
                    5
                )
                .unwrap_err(),
                "parent_authority_required"
            );
            assert_eq!(ledger, before);
            mailbox::send(
                &mut ledger,
                &child_actor,
                &actor,
                "report",
                "finished",
                "report",
                None,
                6,
            )
            .unwrap();
            let manual = watch::start(&mut ledger, &actor, &child_actor, 7).unwrap();
            let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
            assert_eq!(
                apply(&mut restored, &actor, &bind, 8).unwrap()["child"],
                child
            );
            assert_eq!(apply(&mut restored, &actor, &complete, 8).unwrap(), receipt);
            assert_eq!(restored.watches[0].id, manual.id);
            apply(
                &mut restored,
                &actor,
                &Mutation::End {
                    id: origin.clone(),
                    actor: None,
                },
                9,
            )
            .unwrap();
            let live = restored
                .agents
                .iter()
                .find(|record| record.id == child)
                .unwrap();
            assert!(!live.ended);
            assert_eq!(view(live, &restored)["origin"], origin);
            apply(
                &mut restored,
                &child_actor,
                &Mutation::End {
                    id: child,
                    actor: None,
                },
                10,
            )
            .unwrap();
            assert!(restored.watches.is_empty());
        }
    }

    #[test]
    fn handoff_self_registration_preserves_provenance_without_transferring_responsibility() {
        let (mut ledger, caller, origin, spawn, child) =
            pending_spawn_mode(SpawnMode::Handoff, "local");
        let mut registration = child.clone();
        registration.origin = None; // The public register command has no origin input.
        let id = apply(
            &mut ledger,
            &caller,
            &Mutation::BindChild {
                id: spawn,
                record: child,
            },
            3,
        )
        .unwrap()["child"]
            .as_str()
            .unwrap()
            .to_owned();
        let before = ledger.clone();
        for check in [false, true] {
            let reply = apply(
                &mut ledger,
                &registration.actor,
                &Mutation::Register {
                    record: registration.clone(),
                    check,
                },
                4,
            )
            .unwrap();
            assert_eq!(reply["id"], id);
            assert_eq!(reply["origin"], origin);
            assert!(reply["parent"].is_null());
            assert_eq!(ledger, before);
            for changed in [
                AgentRecord {
                    name: Some("changed".into()),
                    ..registration.clone()
                },
                AgentRecord {
                    parent: Some(origin.clone()),
                    ..registration.clone()
                },
                AgentRecord {
                    origin: Some("someone-else".into()),
                    ..registration.clone()
                },
            ] {
                assert!(
                    apply(
                        &mut ledger,
                        &registration.actor,
                        &Mutation::Register {
                            record: changed,
                            check
                        },
                        5,
                    )
                    .is_err()
                );
                assert_eq!(ledger, before);
            }
        }
    }

    #[test]
    fn version_one_ledger_without_mode_or_origin_preserves_delegation_and_watches() {
        let (mut ledger, actor, parent, spawn, child) = pending_spawn();
        apply(
            &mut ledger,
            &actor,
            &Mutation::BindChild {
                id: spawn.clone(),
                record: child,
            },
            3,
        )
        .unwrap();
        let complete = Mutation::Complete { id: spawn };
        let receipt = apply(&mut ledger, &actor, &complete, 4).unwrap();
        let mut legacy = serde_json::to_value(&ledger).unwrap();
        for record in legacy["agents"].as_array_mut().unwrap() {
            record.as_object_mut().unwrap().remove("origin");
        }
        for record in legacy["spawns"].as_array_mut().unwrap() {
            record.as_object_mut().unwrap().remove("mode");
            record["no_watch"] = json!(false);
        }
        let mut restored: Ledger = serde_json::from_value(legacy).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored, ledger);
        assert_eq!(restored.spawns[0].mode, SpawnMode::Delegation);
        assert_eq!(view(&restored.agents[1], &restored)["origin"], parent);
        let before = restored.clone();
        assert_eq!(apply(&mut restored, &actor, &complete, 5).unwrap(), receipt);
        assert_eq!(restored, before);

        // Completed old opt-outs remain completed without installing a watch.
        let mut legacy = serde_json::to_value(&ledger).unwrap();
        legacy["watches"] = json!([]);
        legacy["spawns"][0].as_object_mut().unwrap().remove("mode");
        legacy["spawns"][0]["no_watch"] = json!(true);
        legacy["spawns"][0]["auto_watch_id"] = Value::Null;
        let mut restored: Ledger = serde_json::from_value(legacy).unwrap();
        restored.validate().unwrap();
        let before = restored.clone();
        let receipt = apply(&mut restored, &actor, &complete, 5).unwrap();
        assert!(receipt["auto_watch_id"].is_null());
        assert_eq!(restored, before);
    }

    #[test]
    fn handoff_receipt_requires_its_caller_at_every_transition() {
        let (mut ledger, _, caller, spawn, child) = pending_spawn_mode(SpawnMode::Handoff, "local");
        let stranger = record("stranger", "native-stranger", None).actor;
        for mutation in [
            Mutation::Reserve {
                parent: caller,
                command: command("once"),
            },
            Mutation::Advance {
                seed_tab: None,
                id: spawn.clone(),
                path: Some("/fixture/topic".into()),
                pane: None,
                terminal: None,
                shell: None,
                child: None,
            },
            Mutation::BindChild {
                id: spawn.clone(),
                record: child,
            },
            Mutation::Complete { id: spawn },
        ] {
            let before = ledger.clone();
            assert_eq!(
                apply(&mut ledger, &stranger, &mutation, 3).unwrap_err(),
                "parent_authority_required"
            );
            assert_eq!(ledger, before);
        }
    }

    #[test]
    fn interrupted_child_binding_preserves_an_end_and_completion_receipt_after_reload() {
        let (mut ledger, actor, parent, spawn, child) = pending_spawn();
        let bind = Mutation::BindChild {
            id: spawn.clone(),
            record: child,
        };
        let bound = apply(&mut ledger, &actor, &bind, 3).unwrap();
        let child = bound["child"].as_str().unwrap().to_owned();
        apply(
            &mut ledger,
            &actor,
            &Mutation::End {
                id: child.clone(),
                actor: Some(parent),
            },
            4,
        )
        .unwrap();
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        let next_id = restored.next_id;
        assert_eq!(
            apply(&mut restored, &actor, &bind, 5).unwrap()["child"],
            child
        );
        let complete = Mutation::Complete { id: spawn };
        let receipt = apply(&mut restored, &actor, &complete, 6).unwrap();
        assert_eq!(receipt["completed"], true);
        assert!(receipt["auto_watch_id"].is_null());
        assert!(resolve_actor(&restored, &child).is_none());
        assert!(restored.watches.is_empty());
        assert_eq!(restored.agents.len(), 2);
        assert_eq!(restored.next_id, next_id);
        let mut restored: Ledger = serde_json::from_slice(&restored.bytes().unwrap()).unwrap();
        let before = restored.clone();
        assert_eq!(apply(&mut restored, &actor, &complete, 7).unwrap(), receipt);
        assert_eq!(restored, before);
    }

    #[test]
    fn completion_and_initial_watch_commit_together_and_failure_is_resumable() {
        let (mut ledger, actor, _, spawn, child) = pending_spawn();
        apply(
            &mut ledger,
            &actor,
            &Mutation::BindChild {
                id: spawn.clone(),
                record: child,
            },
            3,
        )
        .unwrap();
        for index in 0..crate::delivery::WATCH_LIMIT {
            let target = record(&format!("other-{index}"), "session", None).actor;
            watch::start(&mut ledger, &actor, &target, 4).unwrap();
        }
        let complete = Mutation::Complete { id: spawn };
        let before = ledger.clone();
        assert_eq!(
            apply(&mut ledger, &actor, &complete, 5).unwrap_err(),
            "capacity"
        );
        assert_eq!(ledger, before);
        let released = ledger.watches[0].id.clone();
        watch::stop(&mut ledger, &actor, &released).unwrap();
        let receipt = apply(&mut ledger, &actor, &complete, 6).unwrap();
        let id = receipt["auto_watch_id"].as_str().unwrap();
        assert_eq!(receipt["completed"], true);
        assert!(ledger.watches.iter().any(|watch| watch.id == id));
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        watch::stop(&mut restored, &actor, id).unwrap();
        let before = restored.clone();
        assert_eq!(apply(&mut restored, &actor, &complete, 7).unwrap(), receipt);
        assert_eq!(restored, before);
    }
    #[test]
    fn report_confirmed_before_spawn_completion_prevents_initial_watch_installation() {
        use crate::delivery::mailbox;
        for acknowledged in [false, true] {
            let (mut ledger, actor, _, spawn, child) = pending_spawn();
            let child_actor = child.actor.clone();
            apply(
                &mut ledger,
                &actor,
                &Mutation::BindChild {
                    id: spawn.clone(),
                    record: child,
                },
                3,
            )
            .unwrap();
            let report = crate::delivery::mailbox::send(
                &mut ledger,
                &child_actor,
                &actor,
                "done",
                "completed",
                "report",
                None,
                4,
            )
            .unwrap();
            mailbox::apply(
                &mut ledger,
                &actor,
                None,
                &mailbox::Command::Confirm {
                    ids: vec![report.id.clone()],
                },
                5,
            )
            .unwrap();
            if acknowledged {
                mailbox::apply(
                    &mut ledger,
                    &actor,
                    None,
                    &mailbox::Command::Ack { id: report.id },
                    6,
                )
                .unwrap();
            }
            let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
            let complete = Mutation::Complete { id: spawn };
            let receipt = apply(&mut restored, &actor, &complete, 7).unwrap();
            assert!(restored.watches.is_empty());
            assert_eq!(receipt["completed"], true);
            assert!(receipt["auto_watch_id"].is_null());
            let before = restored.clone();
            assert_eq!(apply(&mut restored, &actor, &complete, 8).unwrap(), receipt);
            assert_eq!(restored, before);
            // Only an explicit watch start opens a fresh observation after a
            // confirmed report; completing this intent does not remove it.
            let watch = watch::start(&mut restored, &actor, &child_actor, 9).unwrap();
            apply(&mut restored, &actor, &complete, 10).unwrap();
            assert_eq!(restored.watches[0].id, watch.id);
        }
    }

    #[test]
    fn pending_report_installs_initial_watch_then_confirmation_closes_it() {
        use crate::delivery::mailbox;
        let (mut ledger, actor, _, spawn, child) = pending_spawn();
        let child_actor = child.actor.clone();
        apply(
            &mut ledger,
            &actor,
            &Mutation::BindChild {
                id: spawn.clone(),
                record: child,
            },
            3,
        )
        .unwrap();
        let report = crate::delivery::mailbox::send(
            &mut ledger,
            &child_actor,
            &actor,
            "done",
            "completed",
            "report",
            None,
            4,
        )
        .unwrap();
        let complete = Mutation::Complete { id: spawn };
        let receipt = apply(&mut ledger, &actor, &complete, 5).unwrap();
        assert_eq!(ledger.watches.len(), 1);
        assert_eq!(receipt["auto_watch_id"], ledger.watches[0].id);
        mailbox::apply(
            &mut ledger,
            &actor,
            None,
            &mailbox::Command::Confirm {
                ids: vec![report.id],
            },
            6,
        )
        .unwrap();
        assert!(ledger.watches.is_empty());
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        let before = restored.clone();
        assert_eq!(apply(&mut restored, &actor, &complete, 7).unwrap(), receipt);
        assert_eq!(restored, before);
    }

    #[test]
    fn acknowledged_report_prevents_the_initial_watch_unless_its_receipt_is_unknown() {
        use crate::delivery::mailbox;
        // An acknowledgement now is the receipt; one a build before this
        // left without a receipt (`false`) or with none recorded (`null`)
        // proves nothing until actual confirmation.
        for receipt in [Some(true), Some(false), None] {
            let (mut ledger, parent, _, spawn, child) = pending_spawn();
            let child_actor = child.actor.clone();
            apply(
                &mut ledger,
                &parent,
                &Mutation::BindChild {
                    id: spawn.clone(),
                    record: child,
                },
                3,
            )
            .unwrap();
            let report = crate::delivery::mailbox::send(
                &mut ledger,
                &child_actor,
                &parent,
                "done",
                "completed",
                "report",
                None,
                4,
            )
            .unwrap();
            let intake = mailbox::pull(&ledger, &parent).unwrap();
            assert_eq!(intake.ids.as_slice(), std::slice::from_ref(&report.id));
            assert_eq!(
                mailbox::apply(
                    &mut ledger,
                    &parent,
                    None,
                    &mailbox::Command::Ack {
                        id: report.id.clone()
                    },
                    5,
                )
                .unwrap()["hook_confirmed"],
                true
            );
            ledger.letters[0].hook_confirmed = receipt;
            ledger.letters[0].finished_at_unix_ms = (receipt != Some(false)).then_some(5);
            let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
            let complete = Mutation::Complete { id: spawn };
            let receipt_known = receipt == Some(true);
            let completed = apply(&mut restored, &parent, &complete, 6).unwrap();
            assert_eq!(restored.watches.len(), usize::from(!receipt_known));
            if receipt_known {
                assert!(completed["auto_watch_id"].is_null());
                continue;
            }
            assert_eq!(completed["auto_watch_id"], restored.watches[0].id);
            let confirm = mailbox::Command::Confirm { ids: intake.ids };
            mailbox::apply(&mut restored, &parent, None, &confirm, 7).unwrap();
            assert!(restored.watches.is_empty());
            let rearmed = watch::start(&mut restored, &parent, &child_actor, 8).unwrap();
            let mut restored: Ledger = serde_json::from_slice(&restored.bytes().unwrap()).unwrap();
            let before = restored.clone();
            assert_eq!(
                apply(&mut restored, &parent, &complete, 9).unwrap(),
                completed
            );
            mailbox::apply(&mut restored, &parent, None, &confirm, 10).unwrap();
            assert_eq!(restored, before);
            assert_eq!(restored.watches[0].id, rearmed.id);
        }
    }

    fn woken(from: &Actor, pane: &str) -> Rebind {
        Rebind {
            from: from.clone(),
            to: Actor {
                pane_id: pane.into(),
                ..from.clone()
            },
            pane: pane.into(),
            terminal_id: None,
        }
    }

    #[test]
    fn a_woken_pane_takes_over_the_registration_its_letters_watch_and_spawn_receipt() {
        let (mut ledger, parent, _, spawn, child) = pending_spawn();
        let from = child.actor.clone();
        let bound = Mutation::BindChild {
            id: spawn.clone(),
            record: child,
        };
        apply(&mut ledger, &parent, &bound, 3).unwrap();
        let watch = watch::start(&mut ledger, &parent, &from, 4).unwrap();
        let ask = crate::delivery::mailbox::send(
            &mut ledger,
            &parent,
            &from,
            "ask",
            "body",
            "request",
            None,
            5,
        )
        .unwrap();
        let report = crate::delivery::mailbox::send(
            &mut ledger,
            &from,
            &parent,
            "done",
            "body",
            "report",
            None,
            6,
        )
        .unwrap();
        let record = ledger
            .agents
            .iter()
            .find(|r| r.actor == from)
            .unwrap()
            .clone();
        let before = ledger.clone();

        let rebinds =
            std::collections::BTreeMap::from([(record.id.clone(), woken(&from, "woken"))]);
        assert_eq!(
            rebind(&mut ledger, &rebinds),
            [(record.id.clone(), from.clone())]
        );

        let moved = ledger.agents.iter().find(|r| r.id == record.id).unwrap();
        assert_eq!(moved.pane, "woken");
        assert_eq!(moved.actor.pane_id, "woken");
        assert_eq!(moved.actor.session, from.session);
        // Who it is and who is responsible for it do not move.
        assert_eq!(
            (&moved.id, &moved.name, &moved.parent, moved.ended),
            (&record.id, &record.name, &record.parent, false)
        );
        let to = &moved.actor;
        let ledger_watch = ledger.watches.iter().find(|w| w.id == watch.id).unwrap();
        // They follow the registration by identity; the names they carry are
        // the wire names they were written with.
        assert!(
            ledger_watch.target.same_identity(to) && ledger_watch.parent.same_identity(&parent)
        );
        let letter = |id: &str| ledger.letters.iter().find(|l| l.id == id).unwrap();
        assert!(letter(&ask.id).recipient.same_identity(to));
        assert!(letter(&report.id).sender.same_identity(to));
        assert_eq!(
            ledger
                .spawns
                .iter()
                .find(|s| s.id == spawn)
                .unwrap()
                .pane
                .as_deref(),
            Some("woken")
        );
        ledger.validate().unwrap();
        assert_eq!(ledger.watches.len(), before.watches.len());
        // Moving it again is no change: the registration is no longer held
        // by the execution the move names.
        let settled = ledger.clone();
        assert!(rebind(&mut ledger, &rebinds).is_empty());
        assert_eq!(ledger, settled);
    }

    #[test]
    fn a_registration_ended_or_pane_taken_by_another_registration_does_not_move() {
        let (mut ledger, parent, _, spawn, child) = pending_spawn();
        let from = child.actor.clone();
        let bound = Mutation::BindChild {
            id: spawn,
            record: child,
        };
        apply(&mut ledger, &parent, &bound, 3).unwrap();
        let id = ledger
            .agents
            .iter()
            .find(|r| r.actor == from)
            .unwrap()
            .id
            .clone();
        // Another live registration already sits on the woken pane.
        let other = register(
            &mut ledger,
            record("woken", "other-session", None),
            &Actor {
                pane_id: "woken".into(),
                name: "woken".into(),
                kind: "codex".into(),
                device_id: "local".into(),
                session: crate::wire::session_digest("other-session"),
            },
        );
        let before = ledger.clone();
        let rebinds = std::collections::BTreeMap::from([(id.clone(), woken(&from, "woken"))]);
        assert!(rebind(&mut ledger, &rebinds).is_empty());
        assert_eq!(ledger, before);
        assert!(!other.is_empty());
        // An ended registration is not revived by a late move.
        end_record(&mut ledger, &id, 9);
        ledger.agents.retain(|r| r.pane != "woken");
        let ended = ledger.clone();
        assert!(rebind(&mut ledger, &rebinds).is_empty());
        assert_eq!(ledger, ended);
    }
}
