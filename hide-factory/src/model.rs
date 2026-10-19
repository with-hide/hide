//! The Factory's values: what a Factory, a Task, its card and its records are.
//!
//! Nothing here performs I/O. The engine changes these values, the store
//! saves them, and the CLI and the stage 2 screens read them.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Minutes and hours as the engine counts them: milliseconds since the epoch.
pub type UnixMs = u64;

pub const MINUTE_MS: u64 = 60_000;
pub const HOUR_MS: u64 = 60 * MINUTE_MS;
pub const DAY_MS: u64 = 24 * HOUR_MS;

/// One Factory per project. Its id is stable for the project's path, so a
/// closed Factory created again for the same project returns with its records
/// (B74).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Factory {
    pub id: String,
    /// Canonical path of the project's primary checkout.
    pub project: String,
    /// The machine that holds the project, by node id; `None` is the core's
    /// own machine, as every Factory made before a core could move was
    /// (PRD core-host-node-move Q17). Its git, quick checks, verify runs and
    /// workers run there; its GitHub work is the core's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// A short name the inbox shows next to each item.
    pub project_name: String,
    pub source: SourceKind,
    /// `owner/name` of a GitHub Factory.
    #[serde(default)]
    pub repo: Option<String>,
    /// The branch Tasks merge into, read at init.
    #[serde(default = "default_branch")]
    pub default_branch: String,
    pub config: Config,
    pub closed: bool,
    pub created_at: UnixMs,
    /// Next `T-<n>` number. Never lowered.
    pub next_task: u32,
    /// Next `L-<n>` number for a local project without its own local issue
    /// store. Never lowered.
    pub next_local_issue: u32,
    /// Auto merge stopped because main verification failed (B44, B47, B48).
    pub main: MainHealth,
    /// When outside work was last read, and how many reads failed in a row
    /// (B65 stale mark after three).
    pub outside_read_at: Option<UnixMs>,
    pub outside_read_failures: u32,
    /// Watch judgments sent today and the day they count for (B69).
    pub watch_day: u64,
    pub watch_sent_today: u32,
    pub watch_last_at: Option<UnixMs>,
    /// The person's approval of this Factory's GitHub reads and writes; no
    /// GitHub write happens without it (D-62).
    #[serde(default)]
    pub github_approval: Option<GithubApproval>,
    /// The operator paused the whole Factory (D-48): no starts, no AI
    /// judgments, no auto merge, workers asleep.
    #[serde(default)]
    pub paused: bool,
    /// Observer calls sent on `observer_day` (local days since the epoch).
    #[serde(default)]
    pub observer_day: u64,
    #[serde(default)]
    pub observer_calls: u32,
    /// The local day the daily cap was last reached (D-34: one activity line
    /// a day).
    #[serde(default)]
    pub observer_cap_notice_day: u64,
    /// What happened in this Factory that no person has to act on, newest
    /// last, at most [`FACTORY_ACTIVITY_LIMIT`] (D-32).
    #[serde(default)]
    pub activity: Vec<Activity>,
    /// GitHub refused this Factory for its sign-in or a permission: one
    /// person's to-do until a recheck passes (D-46).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_block: Option<GithubBlock>,
    /// The holds the recovery schedule works through, one per hold (D-44).
    #[serde(default)]
    pub holds: Vec<Hold>,
    /// Commands a diagnosis named that only a person can run (D-30).
    #[serde(default)]
    pub commands: Vec<CommandToDo>,
    /// Next `C<n>` number of a command to-do. Never lowered.
    #[serde(default)]
    pub next_command: u32,
}

/// The most activity lines a Factory and a Task keep; the oldest go first.
pub const FACTORY_ACTIVITY_LIMIT: usize = 500;
pub const TASK_ACTIVITY_LIMIT: usize = 200;

/// Appends an activity line, dropping the oldest past `limit`.
pub fn push_activity(log: &mut Vec<Activity>, entry: Activity, limit: usize) {
    log.push(entry);
    if log.len() > limit {
        let over = log.len() - limit;
        log.drain(..over);
    }
}

/// One line of a Task's or a Factory's activity log (D-32, D-35): what
/// happened, as a kind and its facts, so a screen says it in the operator's
/// language. Free text in it is the worker's, the AI's or a migrated
/// notice's own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub at: UnixMs,
    /// The Task a Factory line is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(flatten)]
    pub event: ActivityEvent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActivityEvent {
    /// The intake review completed the card: how many criteria it holds,
    /// how many assumptions the review made, and whether a label brought it.
    Intake {
        label: bool,
        criteria: u32,
        assumptions: u32,
    },
    /// A worker started or resumed; `attempt` counts fresh starts.
    Started { resumed: bool },
    /// The worker's report of done (D-36).
    Report { report: WorkerReport },
    /// A pull request was opened or found for the Task.
    PullRequest { number: u64, url: String },
    /// A verification attempt answered: CI or the verify bundle.
    Verification {
        number: u32,
        ci: bool,
        outcome: VerificationOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<String>,
    },
    /// A check sent the work back to the worker with what to fix (D-28).
    SentBack { text: String },
    /// An automatic recovery: running while `outcome` is none (B15).
    Recovery {
        action: RecoveryAction,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<RecoveryOutcome>,
        /// Worktrees a cleanup removed and the bytes it freed (B13).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        removed: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        freed: Option<u64>,
    },
    /// A follow-up candidate was turned into an issue, a Task, or discarded.
    FollowUp {
        discovery: String,
        state: FollowUpState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        issue: Option<String>,
    },
    /// Factory AI decided in a person's place (B21).
    AiDecision { text: String },
    /// Something outside the Factory moved the Task.
    Outside {
        what: OutsideChange,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<String>,
    },
    /// Main verification failed after a Factory merge or an outside push.
    MainBroken {
        by_factory: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<String>,
    },
    /// A worktree a person has to look at was kept (D-58).
    CleanupKept { worktree: String, detail: String },
    /// The watch saw something; `action` ran when it named one (D-29).
    Watch {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<RecoveryAction>,
    },
    /// Factory AI reached today's cap; the rest of the day goes to a person.
    DailyLimit { limit: u32 },
    /// A line in words: a notice from before this log, kept with its words
    /// (D-39), or what the engine tells a person it cannot act on for them,
    /// in the operator's language.
    Note { text: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    Passed,
    Failed,
    Environment,
}

/// Why a request went to a person other than the mode table (B7, B10).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fallback {
    /// The judgment failed or its answer was unusable.
    Failed,
    /// Today's Factory AI calls are used up.
    DailyLimit,
    /// The Factory is paused.
    Paused,
    /// Too many judgments were waiting.
    QueueFull,
    /// Its Task was taken outside or finished while it was being sorted.
    Dropped,
    /// The daemon restarted while it was being sorted.
    Restart,
    /// Factory AI was not sure of the kind.
    Unsure,
    /// The request touches a permission.
    Permission,
    /// Factory AI's answer named none of a closed question's choices.
    NoChoice,
}

impl Fallback {
    pub const ALL: [Self; 9] = [
        Self::Failed,
        Self::DailyLimit,
        Self::Paused,
        Self::QueueFull,
        Self::Dropped,
        Self::Restart,
        Self::Unsure,
        Self::Permission,
        Self::NoChoice,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::DailyLimit => "daily_limit",
            Self::Paused => "paused",
            Self::QueueFull => "queue_full",
            Self::Dropped => "dropped",
            Self::Restart => "restart",
            Self::Unsure => "unsure",
            Self::Permission => "permission",
            Self::NoChoice => "no_choice",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryOutcome {
    /// The hold cleared.
    Improved,
    /// Something moved but the hold stays.
    Partial,
    Unchanged,
}

impl RecoveryOutcome {
    pub const ALL: [Self; 3] = [Self::Improved, Self::Partial, Self::Unchanged];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Improved => "improved",
            Self::Partial => "partial",
            Self::Unchanged => "unchanged",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutsideChange {
    /// An outside pull request closes the Task's issue; the worker stopped.
    ClosingPr,
    /// That pull request merged; the Task is done.
    PrMerged,
    /// The issue closed with no pull request, or lost its label.
    IssueClosed,
    /// A finished Task's issue opened again.
    IssueReopened,
}

/// A worker's report of done in four parts, in the operator's language (D-08, D-36).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerReport {
    /// One line: what came of the work.
    pub result: String,
    #[serde(default)]
    pub changed: Vec<String>,
    #[serde(default)]
    pub verified: Vec<String>,
    #[serde(default)]
    pub unverified: Vec<String>,
    /// The letter as the worker wrote it, for a harness that reports by
    /// letter only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// GitHub refused the Factory's sign-in or a permission (D-46).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubBlock {
    /// `false` for a lost sign-in (401), `true` for a missing permission (403).
    pub forbidden: bool,
    /// The scope a 403 named, when it named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// The step that was refused first.
    pub stage: String,
    pub since: UnixMs,
}

impl GithubBlock {
    /// The command a person runs to give the access back.
    pub fn command(&self) -> String {
        match (&self.scope, self.forbidden) {
            (Some(scope), true) if is_scope(scope) => format!("gh auth refresh -s {scope}"),
            _ => "gh auth login".to_owned(),
        }
    }
}

/// The most bytes a command a person copies may have.
pub const COPY_COMMAND_LIMIT: usize = 400;

/// A command a person is shown to copy, when it is one: one line no longer
/// than [`COPY_COMMAND_LIMIT`] made only of characters drawn as themselves.
/// A diagnosis writes it from facts a worker can shape, and the copy button
/// writes it as it is, so a second line hidden from the screen, a cut
/// marker, or a character drawn as nothing, as a space a shell does not
/// split on, or as a change of direction would make the pasted text differ
/// from the shown one.
pub fn copyable_command(text: &str) -> Option<&str> {
    let text = text.trim();
    (!text.is_empty() && text.len() <= COPY_COMMAND_LIMIT && text.chars().all(drawn_as_itself))
        .then_some(text)
}

/// The plain space, printable ASCII, and letters and digits of any script
/// except the Hangul fillers, which are letters drawn as nothing. Everything
/// else, punctuation outside ASCII included, is refused on the safe side.
fn drawn_as_itself(c: char) -> bool {
    c == ' '
        || c.is_ascii_graphic()
        || (c.is_alphanumeric() && !matches!(c, '\u{115F}' | '\u{1160}' | '\u{3164}' | '\u{FFA0}'))
}

/// A GitHub token scope as gh names one: lowercase words joined by `_` or
/// `:`, short. Nothing else reaches the command a person copies.
pub fn is_scope(scope: &str) -> bool {
    scope.len() < 40
        && scope.starts_with(|c: char| c.is_ascii_lowercase())
        && scope
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == ':')
}

/// What a hold holds back (D-44).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HoldKey {
    /// The machine holds new starts.
    Start { hold: EnvHold },
    /// A cascade of failures halted new starts.
    Halt,
    /// Outside reads keep failing.
    Reads,
    /// A Task stopped for a reason a restart can clear.
    Task { task: String },
}

impl HoldKey {
    /// The hold's name in a to-do, as `hide factory resolve hold:<name>`
    /// takes it.
    pub fn name(&self) -> String {
        match self {
            Self::Start { hold } => format!("start-{}", hold.as_str()),
            Self::Halt => "halt".into(),
            Self::Reads => "reads".into(),
            Self::Task { task } => format!("task-{task}"),
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        if let Some(hold) = name.strip_prefix("start-") {
            return EnvHold::ALL
                .into_iter()
                .find(|h| h.as_str() == hold)
                .map(|hold| Self::Start { hold });
        }
        if let Some(task) = name.strip_prefix("task-").filter(|t| !t.is_empty()) {
            return Some(Self::Task {
                task: task.to_owned(),
            });
        }
        match name {
            "halt" => Some(Self::Halt),
            "reads" => Some(Self::Reads),
            _ => None,
        }
    }
}

/// A hold the recovery schedule works through: one step at 30, 90 and 150
/// minutes, a person at 180 (D-44). Whether a diagnosis is out for its next
/// step is the engine's judgment in flight, never stored, so a restart that
/// lost the answer simply asks again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hold {
    pub key: HoldKey,
    pub since: UnixMs,
    /// Each step taken, in order; its count is the next step's index.
    #[serde(default)]
    pub attempts: Vec<RecoveryAttempt>,
    /// The last diagnosis's cause, for the person's to-do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    #[serde(default)]
    pub phase: HoldPhase,
}

/// Where a hold is in its schedule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldPhase {
    /// Its next step runs when that step's time comes.
    #[default]
    Due,
    /// A step ran at `since` and has its time to work before the next; an
    /// action still unsettled then helped only partly.
    Settling { since: UnixMs },
    /// No action is left to try; it waits for the 180-minute mark (B14).
    Exhausted,
    /// The hold is a person's.
    Escalated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryAttempt {
    pub at: UnixMs,
    /// None when the step found nothing to run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<RecoveryAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RecoveryOutcome>,
}

/// A command only a person can run, which a diagnosis named (B16).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandToDo {
    /// `C<n>`.
    pub id: String,
    pub command: String,
    /// What running it does, in the operator's language.
    pub impact: String,
    /// What needs it, in the operator's language.
    pub cause: String,
    pub at: UnixMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<UnixMs>,
}

impl Factory {
    /// Appends a line to the Factory's activity log (D-32).
    pub fn log(&mut self, at: UnixMs, task: Option<&str>, event: ActivityEvent) {
        push_activity(
            &mut self.activity,
            Activity {
                at,
                task: task.map(str::to_owned),
                event,
            },
            FACTORY_ACTIVITY_LIMIT,
        );
    }

    pub fn hold(&self, key: &HoldKey) -> Option<&Hold> {
        self.holds.iter().find(|hold| &hold.key == key)
    }

    /// Whether the recovery schedule still works on this Task's stop.
    pub fn recovering(&self, task: &str) -> bool {
        self.hold(&HoldKey::Task {
            task: task.to_owned(),
        })
        .is_some_and(|hold| hold.phase != HoldPhase::Escalated)
    }
}

/// Who approved a GitHub Factory's access, for which repository, and when,
/// recorded at `init --confirm` (D-62).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubApproval {
    /// The `gh` login the Factory acts as.
    pub account: String,
    pub repo: String,
    pub at: UnixMs,
}

fn default_branch() -> String {
    "main".into()
}

/// Where Tasks come from and where they merge (D-04, B3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A project with a GitHub remote: issues, labels, pull requests.
    Github,
    /// A project without one: local issues `L-<n>` and a local merge to main.
    Local,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MainHealth {
    pub broken: bool,
    /// The last main commit whose verification passed.
    pub last_green: Option<String>,
    /// Why auto merge stays stopped; empty when it runs.
    pub reason: Option<String>,
    /// A recovery that could not decide or failed waits for a person (B48).
    pub needs_person: bool,
    /// Merges Factory made since the last green, oldest first.
    pub merges_since_green: Vec<LandedMerge>,
    /// A recovery (finding or reverting the failing merge) is running.
    #[serde(default)]
    pub recovering: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandedMerge {
    pub task: String,
    pub sha: String,
    pub at: UnixMs,
}

/// The one verification a Factory chose (D-53).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Verification {
    /// Required checks of the pull request (GitHub only). Empty `checks`
    /// means the branch protection's required checks.
    Ci { checks: Vec<String> },
    /// Commands run in the Task worktree; every one must exit 0.
    Commands { commands: Vec<String> },
    /// No verification: auto merge is not available (B2).
    None,
}

impl Verification {
    /// Whether the Factory verifies at all.
    pub fn configured(&self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMode {
    Auto,
    Manual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

/// An agent a worker runs, by the Herdr kind its adapter declares (D-28).
/// Only an agent whose adapter declares a start is one; what it can do
/// beyond starting (waking, resuming, its texts) is read from the same
/// declaration. Stored as that kind, so `claude` and `codex` records an
/// earlier build wrote read unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Runtime(&'static str);

impl Runtime {
    pub const CLAUDE: Self = Self("claude");
    pub const CODEX: Self = Self("codex");

    pub fn as_str(self) -> &'static str {
        self.0
    }

    /// Any id or alias of an agent whose adapter declares a start.
    pub fn parse(value: &str) -> Option<Self> {
        hide_agent_adapter::start_kind(value).map(Self)
    }

    pub fn adapter(self) -> &'static hide_agent_adapter::AgentAdapter {
        hide_agent_adapter::adapter(self.0).expect("a Runtime is made only from a declared start")
    }

    /// The agent's display name.
    pub fn label(self) -> &'static str {
        self.adapter().label
    }

    /// Every agent a Factory can start, in the adapters' support order.
    pub fn all() -> impl Iterator<Item = Self> {
        hide_agent_adapter::START_KINDS
            .iter()
            .map(|kind| Self(kind))
    }

    /// Whether a Factory can put this worker to sleep (D-28): its adapter
    /// declares a sleep, and one that closes its pane to sleep is bound back
    /// to its worker only for Pi and omp (#857). Grok and OpenCode
    /// close their pane too but keep working through a pause until a task of
    /// their own settles that.
    pub fn sleeps(self) -> bool {
        self.adapter().sleep.is_some_and(|dialect| {
            !dialect.closes_pane_when_sleeping() || matches!(self.0, "pi" | "omp")
        })
    }

    /// How the agent's start takes a model and an effort.
    pub fn launch_options(self) -> Option<hide_agent_adapter::LaunchOptions> {
        self.adapter().start.map(|start| start.options())
    }
}

impl Serialize for Runtime {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for Runtime {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value)
            .ok_or_else(|| serde::de::Error::custom(format!("not a startable agent: {value}")))
    }
}

/// One worker a Factory may start: an agent, optionally its model and
/// effort, and the operator's one line about when it fits (D-41).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerCandidate {
    pub agent: Runtime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default)]
    pub description: String,
}

impl WorkerCandidate {
    pub fn bare(agent: Runtime) -> Self {
        Self {
            agent,
            model: None,
            effort: None,
            description: String::new(),
        }
    }

    /// Refuses, with the reason, a model or effort the agent does not declare.
    pub fn check_launch_choice(&self) -> Result<(), String> {
        if self.model.is_none() && self.effort.is_none() {
            return Ok(());
        }
        let options = self
            .agent
            .launch_options()
            .ok_or_else(|| format!("{} takes no model or effort", self.agent.label()))?;
        options
            .arguments(self.model.as_deref(), self.effort.as_deref())
            .map(drop)
            .map_err(|detail| format!("{}: {detail}", self.agent.label()))
    }
}

/// The most worker candidates one Factory keeps (D-42).
pub const WORKER_CANDIDATE_LIMIT: usize = 5;
/// The longest worker candidate description, in Unicode characters; every
/// intake review carries it.
pub const WORKER_DESCRIPTION_LIMIT: usize = 200;
/// The Observer's daily call cap: default and range (D-17, D-34).
pub const OBSERVER_DAILY_DEFAULT: u32 = 100;
pub const OBSERVER_DAILY_RANGE: std::ops::RangeInclusive<u32> = 1..=1000;

/// Who decides a Factory's decision requests (D-14, D-18): on screen 직접,
/// 함께 and 맡김.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserverMode {
    Manual,
    #[default]
    Assist,
    Autonomous,
}

impl ObserverMode {
    pub const ALL: [Self; 3] = [Self::Manual, Self::Assist, Self::Autonomous];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Assist => "assist",
            Self::Autonomous => "autonomous",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == value)
    }
}

/// The agent, model and effort a Factory's AI judgments run on (D-40).
/// `None` in the config means the app's Hide AI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactoryAi {
    /// A Hide AI provider id (`claude`, `codex`, ...).
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// When a natural-language check runs (D-15).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckPoint {
    Intake,
    AfterDone,
    Periodic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCheck {
    pub at: CheckPoint,
    pub instruction: String,
}

/// The typed recovery actions (D-54). Nothing outside this list runs without
/// a person's approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    RemoveFinishedWorktrees,
    RestartWorker,
    SleepWakeWorker,
    SwitchRuntime,
    RetryReadsAndReconnect,
}

impl RecoveryAction {
    pub const ALL: [Self; 5] = [
        Self::RemoveFinishedWorktrees,
        Self::RestartWorker,
        Self::SleepWakeWorker,
        Self::SwitchRuntime,
        Self::RetryReadsAndReconnect,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::RemoveFinishedWorktrees => "remove_finished_worktrees",
            Self::RestartWorker => "restart_worker",
            Self::SleepWakeWorker => "sleep_wake_worker",
            Self::SwitchRuntime => "switch_runtime",
            Self::RetryReadsAndReconnect => "retry_reads_and_reconnect",
        }
    }
}

/// Every setting a Factory has, with the D-30 defaults (B66).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub verification: Verification,
    pub merge_mode: MergeMode,
    pub merge_method: MergeMethod,
    /// Optional quick check before a merge (D-46), run on the merged tree.
    pub quick_check: Option<String>,
    pub question_deadline_ms: u64,
    pub stall_ms: u64,
    pub no_report_ms: u64,
    pub watch_interval_ms: u64,
    pub watch_daily_limit: u32,
    pub outside_read_ms: u64,
    pub cancel_keep_ms: u64,
    pub done_fold_ms: u64,
    pub archive_fold_ms: u64,
    pub new_task_limit: u32,
    pub verify_failure_limit: u32,
    pub verify_timeout_ms: u64,
    pub disk_floor_bytes: u64,
    /// The first candidate's agent. Read as the only candidate while
    /// `workers` is empty, which is how a Factory made before candidates
    /// reads until the operator next changes its workers (D-42).
    pub default_runtime: Runtime,
    /// Worker candidates, the first the default; empty means one candidate
    /// of `default_runtime` with the CLI's own model and effort.
    pub workers: Vec<WorkerCandidate>,
    pub observer_mode: ObserverMode,
    pub observer_daily_limit: u32,
    pub factory_ai: Option<FactoryAi>,
    pub harness: Option<Harness>,
    pub autonomy: Vec<AutonomyScope>,
    pub autonomy_diff_limit: u32,
    pub recovery: Vec<RecoveryAction>,
    pub risk_paths: Vec<String>,
    pub checks: Vec<UserCheck>,
    pub prd_in_issue: bool,
    pub macos_notifications: bool,
    /// Extra arguments each runtime's worker starts with, such as a
    /// permission mode the operator chose.
    #[serde(default)]
    pub worker_args: std::collections::BTreeMap<String, Vec<String>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            verification: Verification::None,
            merge_mode: MergeMode::Manual,
            merge_method: MergeMethod::Merge,
            quick_check: None,
            question_deadline_ms: 24 * HOUR_MS,
            stall_ms: 30 * MINUTE_MS,
            no_report_ms: 2 * MINUTE_MS,
            watch_interval_ms: 30 * MINUTE_MS,
            watch_daily_limit: 5,
            outside_read_ms: 2 * MINUTE_MS,
            cancel_keep_ms: 7 * DAY_MS,
            done_fold_ms: 3 * DAY_MS,
            archive_fold_ms: 90 * DAY_MS,
            new_task_limit: 3,
            verify_failure_limit: 3,
            verify_timeout_ms: 60 * MINUTE_MS,
            disk_floor_bytes: 20 * 1024 * 1024 * 1024,
            default_runtime: Runtime::CLAUDE,
            workers: Vec::new(),
            observer_mode: ObserverMode::Assist,
            observer_daily_limit: OBSERVER_DAILY_DEFAULT,
            factory_ai: None,
            harness: None,
            autonomy: AutonomyScope::presets(),
            autonomy_diff_limit: 200,
            // Every recovery is on until a person turns one off (D-30).
            recovery: RecoveryAction::ALL.to_vec(),
            risk_paths: Vec::new(),
            checks: Vec::new(),
            prd_in_issue: false,
            macos_notifications: false,
            worker_args: std::collections::BTreeMap::new(),
        }
    }
}

impl Config {
    /// The worker candidates in order, the first the default.
    pub fn candidates(&self) -> Vec<WorkerCandidate> {
        if self.workers.is_empty() {
            vec![WorkerCandidate::bare(self.default_runtime)]
        } else {
            self.workers.clone()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Harness {
    pub name: String,
    /// The natural-language "how to work" preset placed in the worker prompt.
    pub instructions: String,
}

/// A kind of Task a worker may create and start without a person (D-18).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomyScope {
    pub id: String,
    pub description: String,
    pub enabled: bool,
}

impl AutonomyScope {
    /// The presets hide offers; every one starts off (D-18, B32).
    pub fn presets() -> Vec<Self> {
        [
            ("flaky_test", "Fix a flaky test"),
            ("dependency_patch", "Update a dependency by a patch version"),
            ("lint_format", "Lint and format fixes"),
            ("docs_links", "Fix documentation links and typos"),
        ]
        .into_iter()
        .map(|(id, description)| Self {
            id: id.to_owned(),
            description: description.to_owned(),
            enabled: false,
        })
        .collect()
    }
}

/// The Task lifecycle (D-28, Q29) plus the two places the PRD names outside
/// the main line: paused (B54) and merged-awaiting-main (B43).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// 정리 중: the card has open questions or waits for its review.
    Drafting,
    /// 대기: Ready; waits for its predecessors, a slot or the environment.
    Waiting,
    /// 실행 중: a worker holds a slot.
    Running,
    /// 일시정지: a person paused it; slot released, worker asleep.
    Paused,
    /// 막힘: waits for an answer or a predecessor; slot released.
    Blocked,
    /// 검증 중: the worker reported done; Factory verifies.
    Verifying,
    /// 머지 대기: a person merges, requests changes or cancels.
    MergeWaiting,
    /// 머지됨: merged; main verification has not passed yet.
    Landed,
    /// 완료.
    Done,
    /// 멈춤: stopped without report, stalled, or verification failed three times.
    Stopped,
    /// 다시 올리기: reverted from main; runs again on the latest main.
    Relanding,
    /// 밖에서 진행 중: an outside pull request closes this Task's issue.
    Outside,
    /// 취소: kept seven days for revival.
    Cancelled,
}

impl TaskState {
    pub const ALL: [Self; 13] = [
        Self::Drafting,
        Self::Waiting,
        Self::Running,
        Self::Paused,
        Self::Blocked,
        Self::Verifying,
        Self::MergeWaiting,
        Self::Landed,
        Self::Done,
        Self::Stopped,
        Self::Relanding,
        Self::Outside,
        Self::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Drafting => "drafting",
            Self::Waiting => "waiting",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::Verifying => "verifying",
            Self::MergeWaiting => "merge_waiting",
            Self::Landed => "landed",
            Self::Done => "done",
            Self::Stopped => "stopped",
            Self::Relanding => "relanding",
            Self::Outside => "outside",
            Self::Cancelled => "cancelled",
        }
    }

    /// Korean label the operator reads.
    pub fn label(self) -> &'static str {
        match self {
            Self::Drafting => "정리 중",
            Self::Waiting => "대기",
            Self::Running => "실행 중",
            Self::Paused => "일시정지",
            Self::Blocked => "막힘",
            Self::Verifying => "검증 중",
            Self::MergeWaiting => "머지 대기",
            Self::Landed => "머지됨",
            Self::Done => "완료",
            Self::Stopped => "멈춤",
            Self::Relanding => "다시 올리기",
            Self::Outside => "밖에서 진행 중",
            Self::Cancelled => "취소",
        }
    }

    /// Whether the Task holds one of the machine's worker slots.
    pub fn holds_slot(self) -> bool {
        matches!(self, Self::Running | Self::Relanding)
    }

    /// A predecessor in this state lets its dependents start (D-29: merged).
    pub fn merged(self) -> bool {
        matches!(self, Self::Landed | Self::Done)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Column {
    Before,
    Moving,
    Stuck,
    Done,
}

impl Column {
    pub const ALL: [Self; 4] = [Self::Before, Self::Moving, Self::Stuck, Self::Done];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::Moving => "moving",
            Self::Stuck => "stuck",
            Self::Done => "done",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Before => "시작 전",
            Self::Moving => "진행 중",
            Self::Stuck => "멈춤",
            Self::Done => "완료",
        }
    }
}

/// The card's fields (D-06). Goal, criteria, out-of-scope, open decisions and
/// dependencies come from the producer; the review only adds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub title: String,
    pub goal: String,
    /// Intake's one-line description. Old records use the goal at read time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub criteria: Vec<String>,
    pub out_of_scope: Vec<String>,
    pub open_decisions: Vec<String>,
    /// Task ids or issue references this Task waits on.
    pub depends_on: Vec<String>,
    /// Waits on a Task of another repository: shown only (B64).
    pub external: Vec<String>,
}

impl Card {
    pub fn summary(&self) -> String {
        self.summary
            .as_deref()
            .filter(|summary| !summary.trim().is_empty())
            .map(short_summary)
            .unwrap_or_else(|| goal_summary(&self.goal, &self.title))
    }
}

/// A bounded, single-line description; count Unicode characters, not bytes.
pub fn short_summary(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= 60 {
        line
    } else {
        format!("{}…", line.chars().take(59).collect::<String>().trim_end())
    }
}

/// Legacy and failed-review cards need no migration or additional AI call.
pub fn goal_summary(goal: &str, title: &str) -> String {
    let body = goal
        .lines()
        .map(str::trim)
        .filter(|line| {
            let heading = line.trim_matches(['*', '_', ':', ' ']);
            !line.is_empty()
                && !line.starts_with(['#', '<'])
                && !matches!(*line, "---" | "***")
                && heading != title.trim()
                && ![
                    "goal",
                    "summary",
                    "description",
                    "task",
                    "목표",
                    "요약",
                    "설명",
                    "작업",
                ]
                .iter()
                .any(|name| heading.eq_ignore_ascii_case(name))
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut sentence = String::new();
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        sentence.push(ch);
        if matches!(ch, '.' | '!' | '?') && chars.peek().is_none_or(|next| next.is_whitespace()) {
            break;
        }
    }
    short_summary(&sentence)
}

/// What a person sets on a card (D-06); empty means the Factory default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanFields {
    pub review_directly: bool,
    pub priority: i32,
    pub merge_mode: Option<MergeMode>,
    /// Pins the agent's first candidate (`add --runtime`).
    pub runtime: Option<Runtime>,
    /// Pins a candidate by its place in the list, 0 first (`add --worker`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<usize>,
}

/// The record a Task is shown by: its issue once Ready, else its Task id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IssueRef {
    Github { number: u64 },
    Local { number: u32 },
}

impl IssueRef {
    pub fn display(&self) -> String {
        match self {
            Self::Github { number } => format!("#{number}"),
            Self::Local { number } => format!("L-{number}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardChange {
    pub card: Card,
    pub attachment: Option<Attachment>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// Absolute path of the private copy the worker reads.
    pub path: String,
    pub sha256: String,
    pub version: u32,
    pub original: String,
}

/// Where a question came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionOrigin {
    Review,
    Worker,
    Check,
    Engine,
}

/// What answering a question does once chosen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuestionKind {
    /// An open decision while drafting (no deadline progress, D-09).
    Intake,
    /// The review proposed a split (D-55).
    Split { pieces: Vec<SplitPiece> },
    /// A worker question with a default action (B26).
    Default,
    /// A worker question that cannot proceed without an answer (B27).
    Blocking,
    /// A scope change: a person approves a wider scope (B29 ③). A card or
    /// PRD added again while the Task runs carries it here and becomes the
    /// Task's on approval (B16).
    ScopeChange {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        change: Option<Box<CardChange>>,
    },
    /// The new-task cap was reached (B31).
    NewTaskCap,
    /// A worker proposed a new Task outside autonomy (B30).
    ProposedTask {
        draft: Box<Card>,
        /// The prerequisite discovery it answers; on approval the proposer
        /// waits on the new Task (D-16 ④).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        discovery: Option<String>,
    },
    /// A stop, a main break or a recovery that needs a person; the choices
    /// are the actions.
    Action,
    /// The fix Task drafted for a main an outside push broke waits for a
    /// person's confirmation; a labelled issue starts without one (D-01).
    ConfirmCard,
}

impl QuestionKind {
    /// Whether an answer must be one of the listed choices: a worker's
    /// question and an intake question take any text (D-33).
    pub fn closed(&self) -> bool {
        !matches!(self, Self::Intake | Self::Default | Self::Blocking)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitPiece {
    pub title: String,
    pub goal: String,
    pub criteria: Vec<String>,
    /// Indexes of earlier pieces this piece waits on.
    pub after: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub origin: QuestionOrigin,
    pub kind: QuestionKind,
    pub text: String,
    pub suggestion: String,
    pub default_action: Option<String>,
    pub deadline: Option<UnixMs>,
    pub asked_at: UnixMs,
    pub choices: Vec<String>,
    pub answer: Option<Answer>,
    /// The letter a worker asked through, answered by reply.
    pub letter: Option<String>,
    /// Where a decision request was sent and why (D-14, D-18).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<Routing>,
    /// What the question holds up, in the operator's language, when the
    /// judgment that asked it said (D-33).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
    /// A worker's question as Factory AI rewrote it for a person who has not
    /// read the Task, without ids, commands or paths (B22); the worker's own
    /// words stay in `text`, which its record and its answer quote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person_text: Option<String>,
    /// What each choice leads to, in the operator's language (D-33).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcomes: Vec<ChoiceOutcome>,
    /// Links and log paths behind the question.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

/// What answering with one choice leads to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceOutcome {
    pub choice: String,
    pub result: String,
}

/// The five kinds the Observer sorts a decision request into (D-14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionKind {
    /// The answer is already there.
    A,
    /// A technical choice.
    B,
    /// A product or taste choice.
    C,
    /// A permission: cost, sign-in, deletion, security, outside effect, out
    /// of scope, irreversible.
    D,
    /// The card is wrong.
    E,
}

impl DecisionKind {
    pub const ALL: [Self; 5] = [Self::A, Self::B, Self::C, Self::D, Self::E];

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "A" => Some(Self::A),
            "B" => Some(Self::B),
            "C" => Some(Self::C),
            "D" => Some(Self::D),
            "E" => Some(Self::E),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::E => "E",
        }
    }
}

/// Where a decision request went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteTo {
    /// The Observer is judging it; a person may still answer first.
    Pending,
    Person,
    /// The Observer answered or applied its proposal.
    Observer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routing {
    pub to: RouteTo,
    /// The Factory's mode when the request arrived; a later change does not
    /// move it (D-18).
    pub mode: ObserverMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<DecisionKind>,
    /// The Observer's one-line reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Why it went to a person other than the mode table (B7, B10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Fallback>,
    /// E in assist: the Observer's fix, offered as a choice (D-33).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<ObserverProposal>,
    /// A person replaced the Observer's answer.
    #[serde(default)]
    pub overridden: bool,
}

/// What the Observer proposes for a wrong card (D-33).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObserverProposal {
    CardFix { card: Box<Card> },
    NewTask { card: Box<Card>, prerequisite: bool },
}

/// The choice an assist-mode E request carries for the Observer's proposal.
pub const PROPOSAL_CHOICE: &str = "AI 제안 적용";

impl Question {
    pub fn open(&self) -> bool {
        self.answer.is_none()
    }

    /// Open and a person's to answer: not a request the Observer is still
    /// sorting.
    pub fn awaits_person(&self) -> bool {
        self.open()
            && self
                .routing
                .as_ref()
                .is_none_or(|routing| routing.to != RouteTo::Pending)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub text: String,
    /// The suggestion or default chosen, when one was.
    pub chose: Option<String>,
    /// Who relayed it: a pane id for an operator pane, or `deadline`.
    pub relayed_by: String,
    pub at: UnixMs,
}

/// The five discovery classes (D-16, B29).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryClass {
    InScope = 1,
    Decision = 2,
    ScopeChange = 3,
    Prerequisite = 4,
    Unrelated = 5,
}

impl DiscoveryClass {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "in-scope" | "in_scope" => Some(Self::InScope),
            "decision" => Some(Self::Decision),
            "scope-change" | "scope_change" => Some(Self::ScopeChange),
            "prerequisite" => Some(Self::Prerequisite),
            "unrelated" => Some(Self::Unrelated),
            _ => None,
        }
    }

    /// How far the class is from a person: a reclassification may only move
    /// toward a person (B30). ① and ② are the agent's, ③ and ⑤ go to a
    /// person, ④ creates work a person approves.
    pub fn toward_person(self) -> u8 {
        match self {
            Self::InScope => 0,
            Self::Decision => 1,
            Self::Prerequisite => 2,
            Self::ScopeChange => 3,
            Self::Unrelated => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovery {
    pub id: String,
    pub class: DiscoveryClass,
    pub text: String,
    pub at: UnixMs,
    /// The Task a prerequisite proposal created, once it exists.
    pub task: Option<String>,
    /// An unrelated finding is a follow-up candidate a person may turn into
    /// an issue or a Task, or discard (D-05, D-31).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_up: Option<FollowUp>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowUp {
    pub state: FollowUpState,
    /// The issue it became.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueRef>,
    /// The Task it became.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Why the last attempt to make its issue failed; the person may press
    /// again (B19).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// When its state last changed.
    pub at: UnixMs,
}

impl FollowUp {
    pub fn open(at: UnixMs) -> Self {
        Self {
            state: FollowUpState::Open,
            issue: None,
            task: None,
            failure: None,
            at,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowUpState {
    Open,
    /// An issue without the factory label.
    Issue,
    /// A factory-labelled issue that started as a Task.
    Factory,
    Discarded,
}

impl FollowUpState {
    pub const ALL: [Self; 4] = [Self::Open, Self::Issue, Self::Factory, Self::Discarded];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Issue => "issue",
            Self::Factory => "factory",
            Self::Discarded => "discarded",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub text: String,
    pub by: String,
    pub at: UnixMs,
    /// The Observer's kind and one-line reason for a decision it made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<DecisionKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What made it; absent on a record from before sources were kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<DecisionSource>,
    /// The question it answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// A person replaced Factory AI's decision with another answer (D-35).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed: Option<DecisionChange>,
}

/// Where a decision record came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSource {
    /// An answer to a question.
    Answer,
    /// What the intake review assumed instead of asking (D-26).
    Assumption,
    /// A check sent the work back (D-28).
    SendBack,
    /// A worker recorded its own decision.
    Worker,
    /// A person asked for changes at merge.
    RequestChanges,
    /// Factory AI approved a risk-path merge.
    RiskMerge,
}

impl DecisionSource {
    pub const ALL: [Self; 6] = [
        Self::Answer,
        Self::Assumption,
        Self::SendBack,
        Self::Worker,
        Self::RequestChanges,
        Self::RiskMerge,
    ];
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionChange {
    pub by: String,
    pub at: UnixMs,
    /// The decision as Factory AI made it.
    pub from: String,
}

impl DecisionRecord {
    pub fn new(text: String, by: String, at: UnixMs) -> Self {
        Self {
            text,
            by,
            at,
            kind: None,
            reason: None,
            source: None,
            question: None,
            changed: None,
        }
    }

    pub fn with_source(mut self, source: DecisionSource) -> Self {
        self.source = Some(source);
        self
    }

    /// Factory AI made it and no person replaced it.
    pub fn by_ai(&self) -> bool {
        self.by == OBSERVER && self.changed.is_none()
    }

    /// A person may still replace it: Factory AI's answer to a question, an
    /// intake assumption or a send-back (B28).
    pub fn overridable(&self) -> bool {
        self.by_ai()
            && matches!(
                self.source,
                Some(
                    DecisionSource::Answer | DecisionSource::Assumption | DecisionSource::SendBack
                )
            )
    }
}

/// The id a decision is named by: its place in the Task's list, from 1.
/// Decisions are never removed, so the place is stable.
pub fn decision_id(index: usize) -> String {
    format!("R{}", index + 1)
}

pub fn decision_index(id: &str) -> Option<usize> {
    id.strip_prefix('R')?.parse::<usize>().ok()?.checked_sub(1)
}

/// `by` of a decision, an answer or a merge the Observer made (D-19).
pub const OBSERVER: &str = "observer";

/// A person gate that sends an auto Task to merge waiting (D-25, B39, B41).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    ReviewDirectly,
    ApprovedScopeChange,
    BreakingChange,
    NoVerification,
    RiskPath,
    ManualMode,
    OpenQuestion,
    CheckFailed,
    AutonomyDiff,
    DirtyMain,
    MergeRefused,
}

impl Gate {
    pub const ALL: [Self; 11] = [
        Self::ReviewDirectly,
        Self::ApprovedScopeChange,
        Self::BreakingChange,
        Self::NoVerification,
        Self::RiskPath,
        Self::ManualMode,
        Self::OpenQuestion,
        Self::CheckFailed,
        Self::AutonomyDiff,
        Self::DirtyMain,
        Self::MergeRefused,
    ];

    pub fn reason(self) -> &'static str {
        match self {
            Self::ReviewDirectly => "직접 확인 표시",
            Self::ApprovedScopeChange => "승인된 범위 변경",
            Self::BreakingChange => "보고된 breaking change 또는 공개 계약 변경",
            Self::NoVerification => "검증 없음",
            Self::RiskPath => "위험 경로 변경",
            Self::ManualMode => "manual 머지",
            Self::OpenQuestion => "열린 질문",
            Self::CheckFailed => "점검을 하지 못함",
            Self::AutonomyDiff => "자율 처리 diff 상한 초과",
            Self::DirtyMain => "main checkout에 커밋 안 된 변경",
            Self::MergeRefused => "머지가 거절됨",
        }
    }
}

/// Why a Task stopped (D-28, B24, B37, B31).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    NoReport,
    Stalled,
    VerifyFailed,
    NewTaskCap,
    EnvironmentRepeated,
    WorkerStart,
    PublishRefused,
    /// The worker disappeared again after its one automatic restart (D-25).
    WorkerGone,
}

impl StopReason {
    pub const ALL: [Self; 8] = [
        Self::NoReport,
        Self::Stalled,
        Self::VerifyFailed,
        Self::NewTaskCap,
        Self::EnvironmentRepeated,
        Self::WorkerStart,
        Self::PublishRefused,
        Self::WorkerGone,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::NoReport => "보고 없이 멈춤",
            Self::Stalled => "정체",
            Self::VerifyFailed => "검증 3회 실패",
            Self::NewTaskCap => "새 Task 상한",
            Self::EnvironmentRepeated => "같은 환경 실패 반복",
            Self::WorkerStart => "worker 시작 실패",
            Self::PublishRefused => "push 거절됨",
            Self::WorkerGone => "작업자 사라짐",
        }
    }
}

/// Why no new worker starts on this machine (B57).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvHold {
    /// Free disk is below the Factory's floor.
    DiskFloor,
    /// A command failed with no space left on the device.
    DiskFull,
    MemoryCritical,
}

impl EnvHold {
    pub const ALL: [Self; 3] = [Self::DiskFloor, Self::DiskFull, Self::MemoryCritical];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::DiskFloor => "disk_floor",
            Self::DiskFull => "disk_full",
            Self::MemoryCritical => "memory_critical",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::DiskFloor => "디스크 여유가 기준보다 작음",
            Self::DiskFull => "디스크 부족",
            Self::MemoryCritical => "메모리 압박 critical",
        }
    }
}

/// The worker a Task runs, as the runtime adapter reported it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRef {
    /// The Factory that spawned it.
    #[serde(default)]
    pub factory: String,
    /// The machine it runs on: its Factory's `node` when it started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// The coordination agent id, which ends it.
    #[serde(default)]
    pub agent: Option<String>,
    pub name: String,
    pub pane: Option<String>,
    pub runtime: Runtime,
    pub worktree: String,
    pub branch: String,
    pub started_at: UnixMs,
    pub asleep: bool,
    /// The candidate's model and effort it started with; `None` is the
    /// CLI's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// A verification attempt in progress or finished.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    /// Its place in the Task's attempts, from 1. The n/3 count is the
    /// Task's `failures`, not this.
    pub number: u32,
    pub commit: Option<String>,
    pub started_at: UnixMs,
    pub stage: AttemptStage,
    pub outcome: Option<AttemptOutcome>,
    pub log: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptStage {
    /// After done, in the Task worktree or the pull request's checks.
    Task,
    /// Before merge, on the latest main merged in (verify factories, B38).
    PreMerge,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum AttemptOutcome {
    Passed,
    Failed {
        check: String,
        link: String,
    },
    Environment {
        signal: String,
        check: String,
    },
    /// The run was ended before it answered: the Task went back to its
    /// worker, was cancelled, or an outside pull request took it.
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub head: String,
    /// The Factory opened it (B36); otherwise a harness or an outside PR did.
    pub by_factory: bool,
    pub open: bool,
}

/// A Task: the Factory's unit of execution (D-05).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub factory: String,
    /// `T-<n>`.
    pub id: String,
    pub seq: u32,
    pub issue: Option<IssueRef>,
    pub card: Card,
    pub human: HumanFields,
    pub state: TaskState,
    pub state_since: UnixMs,
    pub created_at: UnixMs,
    pub updated_at: UnixMs,
    /// The Task that proposed this one; its worker may not propose (depth 1).
    pub proposed_by: Option<String>,
    /// Autonomy scope id when started without a person (B32).
    pub autonomy: Option<String>,
    pub attachments: Vec<Attachment>,
    pub questions: Vec<Question>,
    pub discoveries: Vec<Discovery>,
    pub decisions: Vec<DecisionRecord>,
    pub flags: Vec<String>,
    pub review: ReviewState,
    pub producer_pane: Option<String>,
    pub worker: Option<WorkerRef>,
    pub attempts: Vec<Attempt>,
    /// n of n/3: verification failures counted against the Task (D-53).
    pub failures: u32,
    /// Environment failures in a row, for the reclassification rule (B60).
    pub environment_failures: u32,
    pub pr: Option<PullRequest>,
    pub merge_sha: Option<String>,
    pub gates: Vec<Gate>,
    pub stop: Option<StopReason>,
    /// What the runtime said when the stop came from outside the Task, such
    /// as a refused worker start; shown beside the stop reason.
    #[serde(default)]
    pub stop_detail: Option<String>,
    /// Worker starts refused or abandoned so far; names the next start's intent.
    #[serde(default)]
    pub spawn_refusals: u32,
    pub breaking: bool,
    pub scope_approved: bool,
    pub new_tasks: u32,
    pub new_task_cap_extended: bool,
    pub cancelled_at: Option<UnixMs>,
    /// State the Task was cancelled from, for revival (B55).
    pub cancelled_from: Option<TaskState>,
    pub purged: bool,
    pub done_at: Option<UnixMs>,
    /// A person has seen the completion (the done column's unread dot).
    pub seen: bool,
    /// The worker's last report: ask, block, propose, decide or done.
    pub last_report_at: Option<UnixMs>,
    /// When the engine last started or woke the worker; a rest that began
    /// before it is not the worker's current one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub woken_at: Option<UnixMs>,
    /// Intent keys of external writes already made (B73).
    pub writes: BTreeSet<String>,
    /// Environment hold: a start was refused by the pre-start check (B57).
    pub held: Option<String>,
    /// The same hold as a code.
    #[serde(default)]
    pub held_code: Option<EnvHold>,
    pub label_path: bool,
    pub source_body_hash: Option<String>,
    /// The intake review's candidate and its reason (D-41).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_pick: Option<WorkerPick>,
    /// The candidate the worker first started with; a retry, an automatic
    /// restart and a resume reuse it (D-42).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launched: Option<WorkerCandidate>,
    /// The wake and diagnosis of a worker resting without a report (D-22,
    /// D-23, D-35); cleared by the next report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<Recovery>,
    /// The rest start seen last and the one before it (D-24).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rest_seen: Option<UnixMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rest_before: Option<UnixMs>,
    /// The Observer's one-line diagnosis of a no-report stop (B23).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<String>,
    /// Automatic restarts of a vanished worker since a person last started
    /// it (D-25).
    #[serde(default)]
    pub auto_restarts: u32,
    /// Why the Task is paused, when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<PauseReason>,
    /// What happened to the Task, newest last, at most
    /// [`TASK_ACTIVITY_LIMIT`] (D-35).
    #[serde(default)]
    pub activity: Vec<Activity>,
    /// The worker's last report of done (D-36).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<WorkerReport>,
    /// Each completion criterion as the last check judged it (D-28).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criteria_check: Vec<CriterionVerdict>,
    /// 결정 필요 items a person answered or pressed for this Task (D-45).
    #[serde(default)]
    pub person_items: u32,
    /// When its first worker started (D-45).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_started_at: Option<UnixMs>,
    /// The recovery schedule restarted its worker; once per Task (D-44).
    #[serde(default)]
    pub recovery_restarted: bool,
    /// A GitHub step for it waits for the Factory's access to come back
    /// (D-46).
    #[serde(default)]
    pub permission_wait: bool,
    /// Its worker's pane runs but the agent has shown no session for ten
    /// minutes: a person looks at the pane, where a first-run prompt may
    /// wait.
    #[serde(default)]
    pub start_waiting: bool,
}

/// One completion criterion as a check judged it (D-28).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriterionVerdict {
    /// The criterion's place in the card's criteria, from 0.
    pub index: usize,
    /// The criterion as the check saw it; a verdict whose criterion the card
    /// no longer has in that place is shown on none.
    pub criterion: String,
    pub state: CriterionState,
    #[serde(default)]
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionState {
    Met,
    Unmet,
    Unknown,
}

impl CriterionState {
    pub const ALL: [Self; 3] = [Self::Met, Self::Unmet, Self::Unknown];

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "met" => Some(Self::Met),
            "unmet" => Some(Self::Unmet),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// The intake review's candidate, by its place in the list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerPick {
    pub index: usize,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recovery {
    /// When the engine woke the worker, or decided it could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub woke_at: Option<UnixMs>,
    /// When the diagnosis was asked, and whether its answer came.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnosed_at: Option<UnixMs>,
    #[serde(default)]
    pub diagnosing: bool,
    /// The one worker text the diagnosis read, if it had any (D-37).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnosed_from: Option<crate::judgment::WorkerTextSource>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    /// A person paused the Task.
    Person,
    /// The operator closed the worker's pane in Hide (D-26).
    PaneClosed,
}

impl PauseReason {
    pub const ALL: [Self; 2] = [Self::Person, Self::PaneClosed];
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// Not reviewed yet.
    #[default]
    Pending,
    /// The judgment is queued or running.
    Requested { at: UnixMs },
    /// The review answered.
    Done { result: ReviewResult },
    /// The review could not run (B19); a person is asked to fix the provider.
    Failed { reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewResult {
    Ready,
    NeedsAnswers,
    Split,
}

impl Task {
    /// Whether a person may still replace this decision of Factory AI's. An
    /// answer to a closed question already ran its choice (a split made its
    /// Tasks, a revert started), which a new record would not undo, so only
    /// an answer to a question that takes words is changeable (B28, B29).
    pub fn decision_changeable(&self, record: &DecisionRecord) -> bool {
        record.overridable()
            && (record.source != Some(DecisionSource::Answer)
                || record.question.as_ref().is_some_and(|id| {
                    self.questions
                        .iter()
                        .any(|q| &q.id == id && !q.kind.closed())
                }))
    }

    /// A new Task in drafting, before its review (D-05).
    pub fn draft(factory: &str, id: &str, seq: u32, card: Card, now: UnixMs) -> Self {
        Self {
            factory: factory.to_owned(),
            id: id.to_owned(),
            seq,
            issue: None,
            card,
            human: HumanFields::default(),
            state: TaskState::Drafting,
            state_since: now,
            created_at: now,
            updated_at: now,
            proposed_by: None,
            autonomy: None,
            attachments: Vec::new(),
            questions: Vec::new(),
            discoveries: Vec::new(),
            decisions: Vec::new(),
            flags: Vec::new(),
            review: ReviewState::Pending,
            producer_pane: None,
            worker: None,
            attempts: Vec::new(),
            failures: 0,
            environment_failures: 0,
            pr: None,
            merge_sha: None,
            gates: Vec::new(),
            stop: None,
            stop_detail: None,
            spawn_refusals: 0,
            breaking: false,
            scope_approved: false,
            new_tasks: 0,
            new_task_cap_extended: false,
            cancelled_at: None,
            cancelled_from: None,
            purged: false,
            done_at: None,
            seen: true,
            last_report_at: None,
            woken_at: None,
            writes: BTreeSet::new(),
            held: None,
            held_code: None,
            label_path: false,
            source_body_hash: None,
            ai_pick: None,
            launched: None,
            recovery: None,
            rest_seen: None,
            rest_before: None,
            diagnosis: None,
            auto_restarts: 0,
            pause_reason: None,
            activity: Vec::new(),
            report: None,
            criteria_check: Vec::new(),
            person_items: 0,
            first_started_at: None,
            recovery_restarted: false,
            permission_wait: false,
            start_waiting: false,
        }
    }

    /// Appends a line to the Task's activity log (D-35).
    pub fn log(&mut self, at: UnixMs, event: ActivityEvent) {
        push_activity(
            &mut self.activity,
            Activity {
                at,
                task: None,
                event,
            },
            TASK_ACTIVITY_LIMIT,
        );
    }

    pub fn display_id(&self) -> String {
        self.issue
            .as_ref()
            .map(IssueRef::display)
            .unwrap_or_else(|| self.id.clone())
    }

    pub fn open_questions(&self) -> impl Iterator<Item = &Question> {
        self.questions.iter().filter(|question| question.open())
    }

    pub fn merge_mode(&self, factory: &Factory) -> MergeMode {
        if !factory.config.verification.configured() {
            return MergeMode::Manual;
        }
        self.human.merge_mode.unwrap_or(factory.config.merge_mode)
    }

    /// The candidate a new worker starts with, its place in the list when it
    /// is one, and whether a person chose it: the started one, else a
    /// person's pin, else the review's pick, else the first (D-41, D-42).
    pub fn candidate(&self, factory: &Factory) -> (WorkerCandidate, Option<usize>, bool) {
        let candidates = factory.config.candidates();
        if let Some(launched) = &self.launched {
            let index = candidates.iter().position(|c| c == launched);
            return (launched.clone(), index, true);
        }
        if let Some(index) = self.human.worker.filter(|i| *i < candidates.len()) {
            return (candidates[index].clone(), Some(index), true);
        }
        if let Some(agent) = self.human.runtime {
            return match candidates.iter().position(|c| c.agent == agent) {
                Some(index) => (candidates[index].clone(), Some(index), true),
                None => (WorkerCandidate::bare(agent), None, true),
            };
        }
        if let Some(pick) = self.ai_pick.as_ref().filter(|p| p.index < candidates.len()) {
            return (candidates[pick.index].clone(), Some(pick.index), false);
        }
        (candidates[0].clone(), Some(0), false)
    }

    /// The card a person must look at: blocked, stopped, merge waiting, or
    /// an open question (D-28, D-47). A Task blocked only on requests the
    /// Observer is still sorting is not yet a person's (D-14), and neither
    /// is a stop the recovery schedule is still working on (D-44):
    /// `recovering` says whether that is so for this Task.
    pub fn needs_person(&self, recovering: bool) -> bool {
        let sorting = self.state == TaskState::Blocked && {
            let mut open = self.open_questions().peekable();
            open.peek().is_some() && open.all(|question| !question.awaits_person())
        };
        let stop_is_persons = self.state != TaskState::Stopped || !recovering;
        (matches!(
            self.state,
            TaskState::Blocked | TaskState::Stopped | TaskState::MergeWaiting
        ) && !sorting
            && stop_is_persons)
            || (self.state == TaskState::Paused
                && self.pause_reason == Some(PauseReason::PaneClosed))
            || self.open_questions().any(Question::awaits_person)
    }

    /// Whether a stop is one the recovery schedule restarts (D-44).
    pub fn recoverable_stop(&self) -> bool {
        self.state == TaskState::Stopped
            && matches!(
                self.stop,
                Some(
                    StopReason::WorkerStart
                        | StopReason::EnvironmentRepeated
                        | StopReason::NoReport
                        | StopReason::Stalled
                )
            )
    }

    /// The decisions Factory AI made that stand.
    pub fn ai_decisions(&self) -> usize {
        self.decisions.iter().filter(|d| d.by_ai()).count()
    }

    pub fn branch_slug(&self) -> String {
        let mut slug = String::new();
        for character in self.card.title.chars() {
            if character.is_ascii_alphanumeric() {
                slug.push(character.to_ascii_lowercase());
            } else if !slug.ends_with('-') && !slug.is_empty() {
                slug.push('-');
            }
            if slug.len() >= 32 {
                break;
            }
        }
        let slug = slug.trim_matches('-').to_owned();
        let number = match &self.issue {
            Some(IssueRef::Github { number }) => number.to_string(),
            Some(IssueRef::Local { number }) => format!("l{number}"),
            None => self.id.to_ascii_lowercase(),
        };
        if slug.is_empty() {
            format!("factory/{number}")
        } else {
            format!("factory/{number}-{slug}")
        }
    }
}

#[cfg(test)]
mod summary_tests {
    use super::*;

    #[test]
    fn a_copied_command_is_exactly_the_one_line_drawn() {
        assert_eq!(copyable_command("  gh auth login "), Some("gh auth login"));
        assert_eq!(copyable_command("ls ~/작업/빌드"), Some("ls ~/작업/빌드"));
        for hidden in [
            "echo \u{202E}txt.hs",
            "a\u{200B}b",
            "echo \u{2066}x\u{2069}",
            "\u{FEFF}gh auth login",
            "one\u{2028}two",
            "one\ntwo",
            "rm\u{E0020}x",
            "ls\u{FE0F} a",
            "ls \u{3164}",
            "ls\u{2800}a",
            "rm\u{00A0}-rf",
            "ls\u{3000}a",
            "a\u{034F}b",
        ] {
            assert_eq!(copyable_command(hidden), None, "{hidden:?}");
        }
    }

    #[test]
    fn factory_sleep_follows_the_sleep_the_adapter_declares() {
        let active = [
            ("claude", true),
            ("codex", true),
            ("pi", true),
            ("omp", true),
            ("grok", false),
            ("opencode", false),
            ("antigravity", true),
        ];
        assert_eq!(Runtime::all().count(), active.len());
        for (kind, supported) in active {
            assert_eq!(Runtime::parse(kind).unwrap().sleeps(), supported, "{kind}");
        }
        assert_eq!(Runtime::parse("cursor"), None, "Cursor is retired");
        // Their manual sleep is declared and closes the pane, so the guard,
        // not a missing declaration, keeps Factory from sleeping them.
        for kind in ["pi", "omp", "opencode"] {
            let sleep = Runtime::parse(kind).unwrap().adapter().sleep;
            assert!(
                sleep.is_some_and(|dialect| dialect.closes_pane_when_sleeping()),
                "{kind}"
            );
        }
    }

    #[test]
    fn an_old_card_skips_title_and_template_headings_and_bounds_unicode() {
        let old = serde_json::json!({"title":"알림 설정", "goal":"알림 설정\n### Goal\n**목표:**\n한 곳에서 알림을 고른다. 다음 문장은 제외한다.","criteria":[],"out_of_scope":[],"open_decisions":[],"depends_on":[],"external":[]});
        let card: Card = serde_json::from_value(old).unwrap();
        assert_eq!(card.summary(), "한 곳에서 알림을 고른다.");
        assert_eq!(
            card.summary, None,
            "reading a legacy card does not migrate it"
        );
        let long = short_summary(&"가".repeat(80));
        assert_eq!(long.chars().count(), 60);
        assert!(long.ends_with('…'));
        assert_eq!(short_summary(" two\n lines  "), "two lines");
    }
}
