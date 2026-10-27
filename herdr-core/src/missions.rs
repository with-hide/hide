//! A Conductor's mission (PRD software-factory-conductor D-02, D-22): one
//! agent session plans a piece of work with the operator, starts the workers
//! that carry it out, replans by what it is told, and reports. The core keeps
//! the record so that it outlives the Conductor's pane (D-11) and draws it in
//! the 보기 slot as a map (D-14).
//!
//! The record is the core's: the Conductor changes it only through the
//! pane-bound `hide mission` commands (D-23), each one event applied under
//! the runtime lock, and the file is written by its own save thread
//! (`LatestFile`). What a node should do next, how a failure is retried and
//! when to ask the operator are the Conductor's judgement (D-37); the record
//! holds only the facts the map shows and the contracts Hide enforces: one
//! Conductor per mission, no worker before the plan is approved, and the
//! mission's concurrency cap (D-45).
//!
//! Since schema 2 every node carries its finish lines
//! (`mission_criteria`, PRD mission-map-git-criteria D-09), and the core
//! refuses a node's done and a mission's close while a line is unmet
//! (D-12). A mission read from a schema 1 book is `exempt` from that gate
//! (D-19) and keeps the sentence it had as its node's one line.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use crate::delivery::Actor;
use crate::latest_file::{LatestFile, read_capped};
use crate::mission_criteria::{CheckInput, Criterion, Rule};

const SCHEMA_VERSION: u32 = 2;
/// The schema before node criteria, read and converted (D-19).
const SCHEMA_BEFORE_CRITERIA: u32 = 1;
/// Answered decisions a mission keeps for a skip of a protected line; past
/// this the oldest goes and is logged.
pub const DECIDED_LIMIT: usize = 32;
/// Missions kept at once, closed ones included. A mission is deleted with
/// its Conductor's session (D-22), so a full book is refused, never evicted.
pub const MISSION_LIMIT: usize = 128;
/// Nodes in one mission's plan, the cap every dependency graph shares.
pub use crate::dag::NODE_LIMIT;
/// The highest concurrency cap a mission or the settings may name.
pub const CAP_LIMIT: u8 = 16;
/// The concurrency cap a fresh install starts with (D-08).
pub const DEFAULT_CAP: u8 = 4;
/// Worker candidates in Settings › Conductor (D-31).
pub const CANDIDATE_LIMIT: usize = 16;
/// Dependencies one node may name.
const DEPENDENCY_LIMIT: usize = 16;
/// Issue numbers and paths a node may declare it touches (B6).
const TOUCH_LIMIT: usize = 32;
const TITLE_LIMIT: usize = 200;
const LINE_LIMIT: usize = 600;
const REPORT_LIMIT: usize = 16 * 1024;
/// Choices one decision offers the operator (PRD mission-plan-visibility
/// B32), each short enough to be a button.
pub const OPTION_LIMIT: usize = 5;
const OPTION_TEXT_LIMIT: usize = 120;
/// The longest answer the operator writes in the banner (B27), counted in
/// characters as the banner counts them.
pub const ANSWER_LIMIT: usize = 500;
const READ_CAP: u64 = 8 * 1024 * 1024;
/// The size a change may bring the saved book to: under the read cap with
/// room for the file's own framing, so what is written is always read back.
const WRITE_BUDGET: usize = 7 * 1024 * 1024;

/// Where a mission stands. A cancelled mission is an end that nothing
/// reopens (D-51); a done one reopens on a further instruction or a change
/// to one of its pull requests (D-12, D-39).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionStatus {
    Planning,
    Running,
    /// A PR-scope mission whose report is given while a stack still waits
    /// for the operator's merge (D-28, B41).
    MergeWait,
    Done,
    Cancelled,
}

impl MissionStatus {
    /// Whether the mission still runs work, which is when its cap applies.
    pub fn open(self) -> bool {
        matches!(self, Self::Planning | Self::Running | Self::MergeWait)
    }
}

/// How far the mission goes (D-04, D-05): up to ready pull requests or
/// branches, or through the merge into main.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionScope {
    #[default]
    Pr,
    Merge,
}

/// The kinds of node, each finished by its own rule (D-15). An external
/// node is a pull request or a branch outside the mission that Hide watches
/// until it reaches main (PRD mission-map-git-criteria D-20).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Change,
    Research,
    Verify,
    External,
}

/// What an external node watches (D-20, B19).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum External {
    Pr(u32),
    Branch(String),
}

/// How a change node reaches main (D-16).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    Pr,
    Direct,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Planned,
    Working,
    /// Waiting on something the Conductor names in the node's line, such as
    /// a lower pull request's merge or a free slot under the cap.
    Waiting,
    Review,
    /// Stopped while the operator decides (D-10, B27).
    Held,
    Done,
    Failed,
    /// Left out of the mission by a replan (D-30).
    Dropped,
}

impl NodeStatus {
    fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Dropped)
    }
}

/// Why a node cannot start, derived from its dependencies (D-46, B51).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocked {
    UpstreamFailed,
    UpstreamDropped,
}

/// The agent the plan assigns to a node (D-08, D-34).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Worker {
    pub agent: String,
    #[serde(default)]
    pub model: Option<String>,
    /// None leaves the effort to the agent's default; `effort_auto` says
    /// the Conductor chose it rather than the operator's candidate.
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub effort_auto: bool,
}

/// The GitHub facts of a node's pull request as Hide last told the
/// Conductor about them, so a restart neither repeats nor misses a wake
/// (D-39, engineering #11).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrSeen {
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub changes_requested: bool,
    /// Closed without its merge: nothing is left to wait on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
}

/// One change of a node's pull request the Conductor is still to be told
/// about. It stays in the record until its letter is in the Conductor's
/// inbox, so a failed send, a crash or no Conductor present delays the wake
/// and never loses it (D-39, engineering #11).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PendingWake {
    /// The mission's count of wakes, so the same event twice (changes
    /// asked for again on one head) is two letters, not one.
    pub seq: u64,
    pub node: String,
    /// The pull request the news is about; none for an outside branch.
    pub pr: Option<u32>,
    pub event: WakeEvent,
    #[serde(default)]
    pub head: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeEvent {
    Merged,
    ChangesRequested,
    NewHead,
    /// An outside node's pull request merged or its branch reached main, so
    /// the nodes waiting on it can start (B19).
    OutsideDone,
    /// An outside pull request closed without a merge (B20).
    OutsideFailed,
}

impl WakeEvent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Merged => "merged",
            Self::ChangesRequested => "changes_requested",
            Self::NewHead => "new_head",
            Self::OutsideDone => "outside_done",
            Self::OutsideFailed => "outside_failed",
        }
    }
}

/// A verify node's result (PRD mission-plan-visibility B33).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
}

/// How many of a verify node's checks passed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Checks {
    pub passed: u32,
    pub total: u32,
}

/// What the Conductor asks the operator beside its question (PRD
/// mission-plan-visibility D-13, B25, B32). `key` names this question, so an
/// answer to one the Conductor has since changed is refused (B26).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Question {
    pub key: u64,
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// The recommended option, counted from 1 as the command names it.
    #[serde(default)]
    pub recommend: Option<u8>,
    /// The node the decision stopped.
    #[serde(default)]
    pub node: Option<String>,
    pub asked_at_unix_ms: u64,
}

/// The operator's answer to the question with `key`, from the banner (B26):
/// kept until it is typed into the Conductor's conversation, so a restart
/// neither loses nor repeats it (engineering #11).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Answer {
    pub key: u64,
    pub text: String,
    /// The option it chose, counted from 1; none for a written answer.
    #[serde(default)]
    pub option: Option<u8>,
    pub at_unix_ms: u64,
    #[serde(default)]
    pub delivered: bool,
    /// Typing it may have reached the conversation without Herdr saying so:
    /// it is not typed again until the operator sends it again (B26).
    #[serde(default)]
    pub unconfirmed: bool,
}

/// Why the operator's last answer from the banner did not reach the
/// Conductor (B28, B39).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerFailure {
    /// No Conductor runs the mission to read it.
    ConductorGone,
    /// The device the mission's project is on does not answer.
    DeviceDisconnected,
    /// Typing it into the conversation failed.
    SendFailed,
}

/// Wakes a mission keeps unsent; past this the oldest is dropped and
/// logged, and the Conductor's own `hide mission show` still reads the
/// pull request as it is.
pub const PENDING_WAKE_LIMIT: usize = 32;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MissionNode {
    pub key: String,
    pub title: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub status: NodeStatus,
    #[serde(default)]
    pub delivery: Option<Delivery>,
    #[serde(default)]
    pub worker: Option<Worker>,
    /// The worker's registration in the delivery ledger, once spawned.
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub pr: Option<u32>,
    /// The pull request a stacked one is based on (B15).
    #[serde(default)]
    pub base_pr: Option<u32>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub attempts: u8,
    /// What the node does now, its conclusion or its verification result.
    #[serde(default)]
    pub line: Option<String>,
    /// What a replan changed in this node and why (B18).
    #[serde(default)]
    pub change: Option<String>,
    #[serde(default)]
    pub review: Option<String>,
    /// The node's finish lines (PRD mission-map-git-criteria D-09).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criteria: Vec<Criterion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<External>,
    /// How often a later node's defect opened it again (D-21).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reopened: u8,
    #[serde(default)]
    pub issues: Vec<u32>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub started_at_unix_ms: Option<u64>,
    #[serde(default)]
    pub finished_at_unix_ms: Option<u64>,
    #[serde(default)]
    pub pr_seen: Option<PrSeen>,
    /// A direct change's commit on main (B22, B33).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The checkout the node's check command last passed or failed in, on
    /// the mission's machine: its head now is what the line is compared
    /// with, whoever works the node then (B7, D-16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// The proof that a direct change's branch is in main, which a mission
    /// waiting for its branches waits on (B17, B18): of that branch only,
    /// while the node stays done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_main: Option<Landed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<Checks>,
}

/// The session that conducts a mission (D-23). `actor` is the delivery
/// identity a command is checked against; `session` names the native
/// conversation, which the archive deletes the mission with (D-22).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Conductor {
    pub actor: Actor,
    #[serde(default)]
    pub session: Option<crate::links::ledger::SessionIdentity>,
    #[serde(default)]
    pub agent_id: Option<String>,
}

fn is_zero(value: &u8) -> bool {
    *value == 0
}

/// A question the operator answered that named a node: the decision a skip
/// of that node's check command or CI rests on (D-13, B12).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Decided {
    pub key: u64,
    pub node: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Mission {
    pub id: String,
    pub title: String,
    /// The main worktree of the project the mission belongs to, on
    /// `device_id`, the key GitHub answers are read by.
    pub project: String,
    /// The catalog project the map's pull request cards open in. A core
    /// move that finds the registration gone writes `null`, which reads as
    /// none rather than making the whole book unreadable.
    #[serde(default, deserialize_with = "null_is_empty")]
    pub workspace_id: String,
    /// The checkout the mission was created in, where a takeover starts.
    #[serde(default)]
    pub checkout_path: String,
    pub device_id: String,
    pub conductor: Conductor,
    pub status: MissionStatus,
    pub scope: MissionScope,
    pub cap: u8,
    /// Settings › Conductor's worker candidates when the mission was
    /// created (D-34, B48); a book written before them has none, which
    /// admits any agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Candidate>,
    /// Settings › Conductor's review and verify defaults when the mission
    /// was created (D-22, B36); none for a mission from before them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkers: Option<CheckerDefaults>,
    /// Made before node criteria: no node needs one and nothing is gated
    /// on them (D-19, B23).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exempt: bool,
    /// The answered questions that named a node, newest last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decided: Vec<Decided>,
    #[serde(default)]
    pub nodes: Vec<MissionNode>,
    /// The order the plan merges in, by node key (D-09).
    #[serde(default)]
    pub merge_order: Vec<String>,
    /// The plan's check command (D-26).
    #[serde(default)]
    pub check: Option<String>,
    /// The GitHub account the plan uses, as `gh auth status` named it (D-54).
    #[serde(default)]
    pub github: Option<String>,
    /// What the operator has to decide now; the map's one line (B17).
    #[serde(default)]
    pub decision: Option<String>,
    /// What the Conductor said beside that line; none for a mission written
    /// before it, whose decision is a line alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<Question>,
    /// How many questions this mission has asked; the next one's key.
    #[serde(default)]
    pub question_seq: u64,
    /// The operator's answer from the banner to the current question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Answer>,
    /// Why the last answer from the banner did not reach the Conductor,
    /// shown once beside the question until the next answer (B28).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_failed: Option<AnswerFailure>,
    /// Why new nodes wait on usage (B13).
    #[serde(default)]
    pub delay: Option<String>,
    #[serde(default)]
    pub report: Option<String>,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    #[serde(default)]
    pub started_at_unix_ms: Option<u64>,
    #[serde(default)]
    pub closed_at_unix_ms: Option<u64>,
    /// The wakes not yet in the Conductor's inbox, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wakes: Vec<PendingWake>,
    /// How many wakes this mission has had; the next one's `seq`.
    #[serde(default)]
    pub wake_seq: u64,
}

impl Mission {
    pub(crate) fn node(&self, key: &str) -> Option<&MissionNode> {
        self.nodes.iter().find(|node| node.key == key)
    }

    /// Why each node cannot start, from a failed or dropped node anywhere
    /// upstream of it.
    pub(crate) fn blocked(&self) -> BTreeMap<String, Blocked> {
        let mut out = BTreeMap::new();
        for node in &self.nodes {
            if node.status.finished() {
                continue;
            }
            let mut found = None;
            crate::dag::walk_upstream(&self.nodes, node, |upstream| match upstream.status {
                NodeStatus::Failed => {
                    found = Some(Blocked::UpstreamFailed);
                    crate::dag::Step::Stop
                }
                NodeStatus::Dropped => {
                    found = found.or(Some(Blocked::UpstreamDropped));
                    crate::dag::Step::Skip
                }
                _ => crate::dag::Step::Continue,
            });
            if let Some(found) = found {
                out.insert(node.key.clone(), found);
            }
        }
        out
    }
}

impl crate::dag::Node for MissionNode {
    fn key(&self) -> &str {
        &self.key
    }
    fn depends_on(&self) -> &[String] {
        &self.depends_on
    }
}

impl crate::dag::Node for NodeInput {
    fn key(&self) -> &str {
        &self.key
    }
    fn depends_on(&self) -> &[String] {
        &self.depends_on
    }
}

/// Settings › Conductor (D-31): the values a new plan starts from. A plan
/// changed in conversation keeps its own values (B3, B48).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConductorSettings {
    #[serde(default = "default_cap")]
    pub concurrency: u8,
    #[serde(default)]
    pub default_scope: MissionScope,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    /// Who reviews and verifies a node by default (PRD
    /// mission-map-git-criteria D-22, B36).
    #[serde(default)]
    pub checkers: CheckerDefaults,
}

impl Default for ConductorSettings {
    fn default() -> Self {
        Self {
            concurrency: DEFAULT_CAP,
            default_scope: MissionScope::Pr,
            candidates: Vec::new(),
            checkers: CheckerDefaults::default(),
        }
    }
}

/// The review and verify defaults a new mission starts from; a node's plan
/// may name others, which the plan shows (D-22).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckerDefaults {
    #[serde(default)]
    pub review: CheckDefault,
    #[serde(default)]
    pub verify: CheckDefault,
}

/// How a node is checked by other agents when the plan says nothing else
/// (B36): the Conductor's judgement, never, always one, or several deciding
/// by `rule`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckMode {
    #[default]
    Conductor,
    None,
    One,
    Many,
}

/// The most checkers a default names.
pub const CHECK_COUNT_LIMIT: u8 = crate::mission_criteria::CHECKER_LIMIT as u8;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckDefault {
    #[serde(default)]
    pub mode: CheckMode,
    #[serde(default)]
    pub rule: Rule,
    /// How many check when `mode` is `many`.
    #[serde(default = "default_check_count")]
    pub count: u8,
    /// The agent, model and effort a checker starts with; none leaves each
    /// to the Conductor.
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// Prefer a model of another company than the one that made the work.
    #[serde(default = "default_other_vendor")]
    pub other_vendor: bool,
}

impl Default for CheckDefault {
    fn default() -> Self {
        Self {
            mode: CheckMode::Conductor,
            rule: Rule::All,
            count: default_check_count(),
            agent: None,
            model: None,
            effort: None,
            other_vendor: default_other_vendor(),
        }
    }
}

fn default_check_count() -> u8 {
    2
}

fn default_other_vendor() -> bool {
    true
}

fn default_cap() -> u8 {
    DEFAULT_CAP
}

/// One worker candidate (D-31, D-34): an agent, its model or the CLI's
/// default, a fixed effort or none for the Conductor to choose, and what it
/// is for.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub agent: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub purpose: String,
}

impl Candidate {
    /// Whether a worker started as `kind` with `launch` is this candidate
    /// (D-34): the same agent, its model where the candidate fixes one or
    /// the CLI's default where it names none, and its effort only where the
    /// candidate fixes one; an automatic effort is the Conductor's choice.
    pub(crate) fn admits(&self, kind: &str, launch: &hide_agent_adapter::LaunchChoice) -> bool {
        hide_agent_adapter::canonical_kind(&self.agent) == hide_agent_adapter::canonical_kind(kind)
            && self.model == launch.model
            && self
                .effort
                .as_ref()
                .is_none_or(|effort| launch.effort.as_ref() == Some(effort))
    }
}

impl ConductorSettings {
    /// Checks a value before it is kept: a cap in range, known agents, and
    /// each model and effort one the agent takes.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(1..=CAP_LIMIT).contains(&self.concurrency) {
            return Err("conductor_settings.cap_out_of_range".into());
        }
        if self.candidates.len() > CANDIDATE_LIMIT {
            return Err("conductor_settings.too_many_candidates".into());
        }
        for candidate in &self.candidates {
            check_worker(
                &candidate.agent,
                candidate.model.as_deref(),
                candidate.effort.as_deref(),
            )
            .map_err(|code| format!("conductor_settings.{code}"))?;
            if candidate.purpose.len() > LINE_LIMIT
                || candidate.purpose.chars().any(char::is_control)
            {
                return Err("conductor_settings.purpose_invalid".into());
            }
        }
        for check in [&self.checkers.review, &self.checkers.verify] {
            if !(2..=CHECK_COUNT_LIMIT).contains(&check.count) {
                return Err("conductor_settings.check_count_out_of_range".into());
            }
            if check.agent.is_none() && (check.model.is_some() || check.effort.is_some()) {
                return Err("conductor_settings.check_agent_missing".into());
            }
            if let Some(agent) = &check.agent {
                check_worker(agent, check.model.as_deref(), check.effort.as_deref())
                    .map_err(|code| format!("conductor_settings.{code}"))?;
            }
        }
        Ok(())
    }
}

/// Whether `agent` takes a model at its start, by its own declaration.
pub(crate) fn takes_model(agent: &str) -> bool {
    hide_agent_adapter::adapter(agent)
        .and_then(|row| row.start)
        .is_some_and(|dialect| !dialect.options().model.is_empty())
}

/// Whether `agent` is an agent Hide starts, and `model` and `effort` are
/// values it takes, by the agent's own declaration.
pub(crate) fn check_worker(
    agent: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<(), String> {
    let Some(dialect) = hide_agent_adapter::adapter(agent).and_then(|row| row.start) else {
        return Err("agent_unknown".into());
    };
    let options = dialect.options();
    if model
        .is_some_and(|model| options.model.is_empty() || !hide_agent_adapter::valid_model(model))
    {
        return Err("model_invalid".into());
    }
    if effort.is_some_and(|effort| !options.efforts.contains(&effort)) {
        return Err("effort_invalid".into());
    }
    Ok(())
}

/// Every mission the core keeps, and the next id to give.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MissionBook {
    #[serde(default)]
    pub missions: Vec<Mission>,
    #[serde(default)]
    pub next: u64,
}

impl MissionBook {
    pub(crate) fn get(&self, id: &str) -> Option<&Mission> {
        self.missions.iter().find(|mission| mission.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: &str) -> Option<&mut Mission> {
        self.missions.iter_mut().find(|mission| mission.id == id)
    }

    /// The mission `actor` conducts and still runs, if any.
    pub(crate) fn conducted_open(&self, actor: &Actor) -> Option<&Mission> {
        self.missions
            .iter()
            .find(|mission| mission.status.open() && mission.conductor.actor.same_identity(actor))
    }

    /// The mission a command from `actor` means when it names none: the one
    /// it conducts that still runs, else its newest done one.
    pub(crate) fn conducted(&self, actor: &Actor) -> Option<&Mission> {
        self.conducted_open(actor).or_else(|| {
            self.missions
                .iter()
                .filter(|mission| {
                    mission.status == MissionStatus::Done
                        && mission.conductor.actor.same_identity(actor)
                })
                .max_by_key(|mission| mission.updated_at_unix_ms)
        })
    }

    pub(crate) fn allocate(&mut self) -> String {
        self.next += 1;
        format!("mission-{}", self.next)
    }
}

/// A command a Conductor's pane sends (`hide mission …`). A command that
/// names no mission means the one the caller conducts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum MissionCommand {
    Create {
        title: String,
        #[serde(default)]
        scope: Option<MissionScope>,
        #[serde(default)]
        cap: Option<u8>,
        #[serde(default)]
        github: Option<String>,
    },
    Plan {
        #[serde(default)]
        mission: Option<String>,
        plan: PlanInput,
    },
    Node {
        #[serde(default)]
        mission: Option<String>,
        node: String,
        patch: NodePatch,
    },
    Start {
        #[serde(default)]
        mission: Option<String>,
    },
    /// Sets or clears the operator's one line (B17), with its reason,
    /// choices, recommended choice and stopped node (B32).
    Decide {
        #[serde(default)]
        mission: Option<String>,
        /// Named every time: `null` clears, so a request that leaves it
        /// out is refused rather than read as a clear.
        #[serde(deserialize_with = "present")]
        line: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        why: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recommend: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node: Option<String>,
    },
    /// Sets or clears why new nodes wait on usage (B13).
    Delay {
        #[serde(default)]
        mission: Option<String>,
        /// Named every time: `null` clears, so a request that leaves it
        /// out is refused rather than read as a clear.
        #[serde(deserialize_with = "present")]
        line: Option<String>,
    },
    Close {
        #[serde(default)]
        mission: Option<String>,
        end: MissionEnd,
        #[serde(default)]
        report: Option<String>,
    },
    Reopen {
        #[serde(default)]
        mission: Option<String>,
    },
    /// The owner's result for one finish line of a node (PRD
    /// mission-map-git-criteria D-23, B5): the node's worker, a checker the
    /// Conductor bound, or the Conductor for a node it works itself.
    Record {
        #[serde(default)]
        mission: Option<String>,
        node: String,
        line: String,
        pass: bool,
        /// The one line the result rests on, which every record carries
        /// (B5); a frame without one is the core's own refusal, with its
        /// way on, rather than one it cannot read.
        #[serde(default)]
        evidence: String,
        /// The commit a check command ran on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        commit: Option<String>,
        /// This run is the rerun of a failure on the same commit (D-14).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        rerun: bool,
    },
    /// The Conductor goes past a line with its reason (B11, B12); a check
    /// command or CI line also names the operator's answered decision.
    Skip {
        #[serde(default)]
        mission: Option<String>,
        node: String,
        line: String,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision: Option<u64>,
    },
    /// A later node found a defect in `node`: it works again with the
    /// defect as a line of its own (D-21, B21).
    Defect {
        #[serde(default)]
        mission: Option<String>,
        node: String,
        found_by: String,
        line: String,
    },
    Take {
        mission: String,
    },
    Show {
        #[serde(default)]
        mission: Option<String>,
    },
    List,
    Settings,
}

impl MissionCommand {
    /// Whether the command can change the record; the others only read it.
    pub(crate) fn mutates(&self) -> bool {
        !matches!(self, Self::Show { .. } | Self::List | Self::Settings)
    }

    /// A command small enough and well-formed enough to take to the core.
    pub fn valid(&self) -> bool {
        let id_ok = |id: &Option<String>| id.as_deref().is_none_or(valid_key);
        match self {
            Self::Create { title, github, .. } => {
                valid_text(title, TITLE_LIMIT)
                    && github.as_deref().is_none_or(|g| valid_text(g, TITLE_LIMIT))
            }
            Self::Plan { mission, .. }
            | Self::Start { mission }
            | Self::Reopen { mission }
            | Self::Show { mission } => id_ok(mission),
            Self::Node { mission, node, .. } => id_ok(mission) && valid_key(node),
            Self::Record {
                mission,
                node,
                line,
                evidence,
                commit,
                ..
            } => {
                // A blank line is the core's own refusal, with its way on.
                id_ok(mission)
                    && valid_key(node)
                    && valid_key(line)
                    && (evidence.trim().is_empty() || valid_text(evidence, LINE_LIMIT))
                    && commit.as_deref().is_none_or(valid_commit)
            }
            Self::Skip {
                mission,
                node,
                line,
                reason,
                ..
            } => id_ok(mission) && valid_key(node) && valid_key(line) && reason.len() <= LINE_LIMIT,
            Self::Defect {
                mission,
                node,
                found_by,
                line,
            } => {
                id_ok(mission)
                    && valid_key(node)
                    && valid_key(found_by)
                    && valid_text(line, LINE_LIMIT)
            }
            Self::Decide {
                mission,
                line,
                why,
                options,
                node,
                ..
            } => {
                // The choices' count and length are refused with their own
                // reasons (`check_question`); here only a frame no command
                // could mean is.
                id_ok(mission)
                    && line
                        .as_deref()
                        .is_none_or(|line| valid_text(line, LINE_LIMIT))
                    && valid_line(why, LINE_LIMIT)
                    && options.len() <= NODE_LIMIT
                    && node.as_deref().is_none_or(valid_key)
            }
            Self::Delay { mission, line } => {
                id_ok(mission)
                    && line
                        .as_deref()
                        .is_none_or(|line| valid_text(line, LINE_LIMIT))
            }
            Self::Close {
                mission, report, ..
            } => {
                id_ok(mission)
                    && report
                        .as_deref()
                        .is_none_or(|report| valid_text(report, REPORT_LIMIT))
            }
            Self::Take { mission } => valid_key(mission),
            Self::List | Self::Settings => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionEnd {
    Done,
    MergeWait,
    Cancelled,
}

/// A whole plan, which replaces the one before it (D-10): a node keeps what
/// its worker did when its key stays, a planned node that is left out goes,
/// and a node that already ran must be dropped first rather than vanish.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanInput {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub scope: Option<MissionScope>,
    #[serde(default)]
    pub cap: Option<u8>,
    #[serde(default)]
    pub github: Option<String>,
    #[serde(default)]
    pub check: Option<String>,
    #[serde(default)]
    pub merge_order: Vec<String>,
    pub nodes: Vec<NodeInput>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeInput {
    pub key: String,
    pub title: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub delivery: Option<Delivery>,
    #[serde(default)]
    pub worker: Option<Worker>,
    /// The node's own finish lines, at most three (PRD
    /// mission-map-git-criteria D-09). One sentence, as a plan written
    /// before them names it, is one line.
    #[serde(default, deserialize_with = "lines")]
    pub criteria: Vec<String>,
    /// The reviews and verifications other agents give (D-07).
    #[serde(default)]
    pub checks: Vec<CheckInput>,
    /// What an external node watches (D-20).
    #[serde(default)]
    pub external: Option<External>,
    #[serde(default)]
    pub issues: Vec<u32>,
    #[serde(default)]
    pub paths: Vec<String>,
    /// What a replan changed here and why (B18).
    #[serde(default)]
    pub change: Option<String>,
}

/// A change to one node. An absent field is kept, `null` clears it, so a
/// field the sender leaves unset is never written: `null` would clear it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<NodeStatus>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_id: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub pr: Option<Option<u32>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub base_pr: Option<Option<u32>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub branch: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u8>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub line: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub change: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub review: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub worker: Option<Option<Worker>>,
    /// A direct change's commit on main (B33).
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub commit: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub verdict: Option<Option<Verdict>>,
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub checks: Option<Option<Checks>>,
    /// The registration the Conductor started as the node's next reviewer
    /// or verifier (PRD mission-map-git-criteria B3, B4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier: Option<String>,
}

/// A node's criteria: a list of lines, one sentence, or none.
fn lines<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Lines {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Option::<Lines>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(Lines::One(line)) => vec![line],
        Some(Lines::Many(lines)) => lines,
    })
}

fn null_is_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Option::<String>::deserialize(deserializer).map(Option::unwrap_or_default)
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

fn double<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
}

pub use crate::dag::valid_key;

fn valid_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= limit
        && !value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}

fn valid_line(value: &Option<String>, limit: usize) -> bool {
    value
        .as_deref()
        .is_none_or(|value| valid_text(value, limit))
}

/// The plan `input` would make of `mission`, or why it is refused.
pub(crate) fn apply_plan(
    mission: &Mission,
    input: &PlanInput,
    now: u64,
) -> Result<Mission, String> {
    if input.nodes.len() > NODE_LIMIT {
        return Err("mission_too_many_nodes".into());
    }
    if !valid_line(&input.title, TITLE_LIMIT)
        || !valid_line(&input.github, TITLE_LIMIT)
        || !valid_line(&input.check, LINE_LIMIT)
    {
        return Err("mission_text_invalid".into());
    }
    if input.cap.is_some_and(|cap| !(1..=CAP_LIMIT).contains(&cap)) {
        return Err("mission_cap_out_of_range".into());
    }
    let mut keys = BTreeSet::new();
    for node in &input.nodes {
        if crate::dag::admit_key(&mut keys, &node.key).is_err() {
            return Err("mission_node_key_invalid".into());
        }
        if !valid_text(&node.title, TITLE_LIMIT)
            || node
                .criteria
                .iter()
                .any(|line| !valid_text(line, LINE_LIMIT))
            || !valid_line(&node.change, LINE_LIMIT)
            || node.depends_on.len() > DEPENDENCY_LIMIT
            || node.issues.len() > TOUCH_LIMIT
            || node.paths.len() > TOUCH_LIMIT
            || node.paths.iter().any(|path| !valid_text(path, LINE_LIMIT))
        {
            return Err("mission_text_invalid".into());
        }
        check_node_kind(node)?;
        if node.criteria.len() > crate::mission_criteria::OWN_LIMIT {
            return Err(crate::coordination::refusal(
                "mission_criteria_too_many",
                &node.key,
            ));
        }
        if node.criteria.is_empty() && node.kind != NodeKind::External && !mission.exempt {
            return Err(crate::coordination::refusal(
                "mission_criteria_missing",
                &node.key,
            ));
        }
        check_checkers(node)?;
        if node.kind == NodeKind::Change && node.delivery.is_none() {
            return Err("mission_delivery_required".into());
        }
        if node.kind != NodeKind::Change && node.delivery.is_some() {
            return Err("mission_delivery_invalid".into());
        }
        if let Some(worker) = &node.worker {
            check_worker(
                &worker.agent,
                worker.model.as_deref(),
                worker.effort.as_deref(),
            )
            .map_err(|code| format!("mission_{code}"))?;
            // A worker that takes a model names one, so the map and the
            // start never read an empty choice; an agent that takes none
            // keeps `null` (PRD mission fixes (b)).
            if worker.model.is_none() && takes_model(&worker.agent) {
                return Err("mission_model_missing".into());
            }
        }
    }
    let edges = crate::dag::check_edges(&input.nodes);
    if matches!(edges, Err(crate::dag::Refusal::Unknown { .. })) {
        return Err("mission_dependency_unknown".into());
    }
    // Each node at most once, so the order is never longer than the plan.
    let mut ordered = BTreeSet::new();
    if input
        .merge_order
        .iter()
        .any(|key| !keys.contains(key.as_str()) || !ordered.insert(key.as_str()))
    {
        return Err("mission_dependency_unknown".into());
    }
    if edges.is_err() {
        return Err("mission_dependency_cycle".into());
    }
    // A node that ran keeps its record; one that is gone from the plan must
    // be dropped first, so a replan cannot hide work that was done (B28).
    // A planned node that already names its worker, pull request or branch
    // was started too, whatever its status still says.
    for node in &mission.nodes {
        let started = node.status != NodeStatus::Planned
            || node.agent_id.is_some()
            || node.pr.is_some()
            || node.branch.is_some();
        if started && !keys.contains(node.key.as_str()) {
            return Err("mission_node_in_progress".into());
        }
    }
    let mut next = mission.clone();
    if input.check.is_some() {
        next.check.clone_from(&input.check);
    }
    let with_check = next.check.is_some() && !mission.exempt;
    let same_check = next.check == mission.check;
    next.nodes = input
        .nodes
        .iter()
        .map(|input| {
            let mut node = mission.node(&input.key).cloned().unwrap_or(MissionNode {
                key: input.key.clone(),
                title: input.title.clone(),
                kind: input.kind,
                depends_on: Vec::new(),
                status: NodeStatus::Planned,
                delivery: None,
                worker: None,
                agent_id: None,
                pr: None,
                base_pr: None,
                branch: None,
                attempts: 0,
                line: None,
                change: None,
                review: None,
                criteria: Vec::new(),
                external: None,
                reopened: 0,
                issues: Vec::new(),
                paths: Vec::new(),
                started_at_unix_ms: None,
                finished_at_unix_ms: None,
                pr_seen: None,
                commit: None,
                folder: None,
                in_main: None,
                verdict: None,
                checks: None,
            });
            node.title.clone_from(&input.title);
            node.kind = input.kind;
            node.depends_on.clone_from(&input.depends_on);
            node.delivery = input.delivery;
            node.worker.clone_from(&input.worker);
            node.criteria = crate::mission_criteria::plan_lines(
                &node.criteria,
                &input.criteria,
                &input.checks,
                with_check && input.kind == NodeKind::Change,
                same_check,
            );
            node.external.clone_from(&input.external);
            node.issues.clone_from(&input.issues);
            node.paths.clone_from(&input.paths);
            if input.change.is_some() {
                node.change.clone_from(&input.change);
            }
            node
        })
        .collect();
    next.merge_order.clone_from(&input.merge_order);
    if let Some(title) = &input.title {
        next.title.clone_from(title);
    }
    if let Some(scope) = input.scope {
        next.scope = scope;
    }
    if let Some(cap) = input.cap {
        next.cap = cap;
    }
    if input.github.is_some() {
        next.github.clone_from(&input.github);
    }
    next.updated_at_unix_ms = now;
    Ok(next)
}

/// An external node names one pull request or branch and nothing a worker
/// would do (B19); no other node names one.
fn check_node_kind(node: &NodeInput) -> Result<(), String> {
    let external = node.kind == NodeKind::External;
    let named = match &node.external {
        None => false,
        Some(External::Pr(number)) => *number > 0,
        Some(External::Branch(branch)) => {
            valid_text(branch, TITLE_LIMIT) && !branch.chars().any(char::is_whitespace)
        }
    };
    let bare = node.worker.is_none()
        && node.delivery.is_none()
        && node.criteria.is_empty()
        && node.checks.is_empty();
    if external != node.external.is_some() || (external && (!named || !bare)) {
        return Err(crate::coordination::refusal(
            "mission_external_invalid",
            &node.key,
        ));
    }
    Ok(())
}

/// A node's reviews and verifications: each kind at most once, each with
/// one to `CHECKER_LIMIT` agents the machine takes (B3).
fn check_checkers(node: &NodeInput) -> Result<(), String> {
    let mut kinds = BTreeSet::new();
    for check in &node.checks {
        if !kinds.insert(check.kind.kind() as u8)
            || check.by.is_empty()
            || check.by.len() > crate::mission_criteria::CHECKER_LIMIT
        {
            return Err(crate::coordination::refusal(
                "mission_checkers_invalid",
                &node.key,
            ));
        }
        for worker in &check.by {
            check_worker(
                &worker.agent,
                worker.model.as_deref(),
                worker.effort.as_deref(),
            )
            .map_err(|code| format!("mission_{code}"))?;
            if worker.model.is_none() && takes_model(&worker.agent) {
                return Err("mission_model_missing".into());
            }
        }
    }
    Ok(())
}

/// `patch` applied to `node`, or why it is refused.
pub(crate) fn apply_patch(
    node: &MissionNode,
    patch: &NodePatch,
    now: u64,
) -> Result<MissionNode, String> {
    let text_ok = |value: &Option<Option<String>>| {
        value
            .as_ref()
            .is_none_or(|value| valid_line(value, LINE_LIMIT))
    };
    if !text_ok(&patch.line) || !text_ok(&patch.change) || !text_ok(&patch.review) {
        return Err("mission_text_invalid".into());
    }
    if patch
        .branch
        .as_ref()
        .is_some_and(|branch| !valid_line(branch, TITLE_LIMIT))
        || patch.agent_id.as_ref().is_some_and(|id| {
            id.as_deref()
                .is_some_and(|id| !crate::delivery::valid_key(id))
        })
    {
        return Err("mission_text_invalid".into());
    }
    // A commit is a direct change's and a verdict a verify node's (B33).
    let direct = node.kind == NodeKind::Change && node.delivery == Some(Delivery::Direct);
    if (patch.commit.is_some() && !direct)
        || ((patch.verdict.is_some() || patch.checks.is_some()) && node.kind != NodeKind::Verify)
    {
        return Err("mission_field_kind_invalid".into());
    }
    if patch.commit.as_ref().is_some_and(|commit| {
        commit
            .as_deref()
            .is_some_and(|commit| !valid_commit(commit))
    }) {
        return Err("mission_commit_invalid".into());
    }
    if patch.checks.as_ref().is_some_and(|checks| {
        checks.is_some_and(|checks| checks.total == 0 || checks.passed > checks.total)
    }) {
        return Err("mission_checks_invalid".into());
    }
    if let Some(Some(worker)) = &patch.worker {
        check_worker(
            &worker.agent,
            worker.model.as_deref(),
            worker.effort.as_deref(),
        )
        .map_err(|code| format!("mission_{code}"))?;
    }
    // An external node is done when Hide reads it in main (B19).
    if node.kind == NodeKind::External
        && (patch.status == Some(NodeStatus::Done)
            || patch.worker.is_some()
            || patch.agent_id.is_some())
    {
        return Err(crate::coordination::refusal(
            "mission_external_invalid",
            &node.key,
        ));
    }
    let mut next = node.clone();
    if let Some(status) = patch.status {
        if status == NodeStatus::Working && next.started_at_unix_ms.is_none() {
            next.started_at_unix_ms = Some(now);
        }
        if status.finished() && !next.status.finished() {
            next.finished_at_unix_ms = Some(now);
        } else if !status.finished() {
            next.finished_at_unix_ms = None;
        }
        next.status = status;
    }
    macro_rules! take {
        ($field:ident) => {
            if let Some(value) = &patch.$field {
                next.$field.clone_from(value);
            }
        };
    }
    take!(agent_id);
    take!(pr);
    take!(base_pr);
    take!(branch);
    take!(line);
    take!(change);
    take!(review);
    take!(worker);
    take!(commit);
    take!(verdict);
    take!(checks);
    if patch.pr.is_some() && next.pr != node.pr {
        // A new pull request starts its own wakes from what GitHub says now.
        next.pr_seen = None;
    }
    if let Some(attempts) = patch.attempts {
        next.attempts = attempts;
    }
    // A branch read in main proves that branch for that finish only: another
    // branch, or the node at work again, waits for a read of its own (B17).
    if next.branch != node.branch || next.status != NodeStatus::Done {
        next.in_main = None;
    }
    Ok(next)
}

/// A direct change's branch as Hide last read it in main: the branch, and
/// its commit then, which is in main.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Landed {
    pub branch: String,
    pub commit: String,
}

/// A commit as Git names one: 7 to 64 hexadecimal digits.
pub fn valid_commit(value: &str) -> bool {
    (7..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The question a `decide` asks, or why it is refused (B32): its choices
/// within their count and length, a recommendation that names one of them,
/// and a node of the plan. A clear carries nothing beside it.
pub(crate) fn check_question(
    mission: &Mission,
    line: Option<&str>,
    why: Option<&str>,
    options: &[String],
    recommend: Option<u8>,
    node: Option<&str>,
) -> Result<(), String> {
    if line.is_none()
        && (why.is_some() || !options.is_empty() || recommend.is_some() || node.is_some())
    {
        return Err("mission_question_without_line".into());
    }
    if options.len() > OPTION_LIMIT {
        return Err("mission_options_too_many".into());
    }
    if options
        .iter()
        .any(|option| !valid_text(option, OPTION_TEXT_LIMIT) || option.contains('\n'))
    {
        return Err("mission_option_invalid".into());
    }
    if recommend.is_some_and(|choice| choice == 0 || usize::from(choice) > options.len()) {
        return Err("mission_recommend_invalid".into());
    }
    if node.is_some_and(|key| mission.node(key).is_none()) {
        return Err("mission_node_unknown".into());
    }
    Ok(())
}

/// Where another open mission touches the same issue or path as one of this
/// mission's nodes (D-18, B6): `(other mission, its title, what overlaps)`.
pub(crate) fn overlaps(book: &MissionBook, mission: &Mission) -> Vec<Overlap> {
    let mut out = Vec::new();
    for other in &book.missions {
        // A path names a project on one machine only.
        if other.id == mission.id
            || !other.status.open()
            || other.project != mission.project
            || other.device_id != mission.device_id
        {
            continue;
        }
        let issues: BTreeSet<u32> = other
            .nodes
            .iter()
            .filter(|node| node.status != NodeStatus::Dropped)
            .flat_map(|node| node.issues.iter().copied())
            .collect();
        let paths: Vec<&str> = other
            .nodes
            .iter()
            .filter(|node| node.status != NodeStatus::Dropped)
            .flat_map(|node| node.paths.iter().map(String::as_str))
            .collect();
        let mut shared_issues = BTreeSet::new();
        let mut shared_paths = BTreeSet::new();
        for node in &mission.nodes {
            shared_issues.extend(
                node.issues
                    .iter()
                    .copied()
                    .filter(|issue| issues.contains(issue)),
            );
            for path in &node.paths {
                if paths.iter().any(|theirs| overlapping(path, theirs)) {
                    shared_paths.insert(path.clone());
                }
            }
        }
        if !shared_issues.is_empty() || !shared_paths.is_empty() {
            out.push(Overlap {
                mission: other.id.clone(),
                title: other.title.clone(),
                issues: shared_issues.into_iter().collect(),
                paths: shared_paths.into_iter().collect(),
            });
        }
    }
    out
}

/// Whether two declared paths name the same file or one holds the other.
fn overlapping(a: &str, b: &str) -> bool {
    let a = a.trim_end_matches('/');
    let b = b.trim_end_matches('/');
    a == b
        || a.strip_prefix(b).is_some_and(|rest| rest.starts_with('/'))
        || b.strip_prefix(a).is_some_and(|rest| rest.starts_with('/'))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Overlap {
    pub mission: String,
    pub title: String,
    pub issues: Vec<u32>,
    pub paths: Vec<String>,
}

/// What an agent should do after a refusal, by its code.
pub fn next_action(code: &str) -> &'static str {
    match code {
        "mission_unknown" => "Run hide mission list to see the missions of this project",
        "mission_none" => "Start one with hide mission create --title <title>",
        "mission_conducted" => {
            "Another session conducts this mission; ask the operator before taking it over"
        }
        "mission_not_conductor" => {
            "Only the mission's Conductor can change it; take it over with hide mission take <id> once it has none"
        }
        "mission_already_conducting" => {
            "Close or cancel the mission you conduct before creating another"
        }
        "mission_cancelled" => "A cancelled mission does not reopen; create a new mission",
        "mission_closed" => "Reopen the mission with hide mission reopen before changing it",
        "mission_not_started" => {
            "Show the plan to the operator and run hide mission start after they approve it"
        }
        "mission_no_nodes" => "Set a plan with hide mission plan before starting",
        "mission_full" => "Ask the operator to delete old Conductor sessions from the archive",
        "mission_too_many_nodes" => "Plan at most 64 nodes; merge or split the work differently",
        "mission_node_unknown" => "Run hide mission show to see the plan's node keys",
        "mission_node_in_progress" => {
            "Drop the node with hide mission node <key> --status dropped before removing it from the plan"
        }
        "mission_dependency_unknown" | "mission_dependency_cycle" => {
            "Check depends_on and merge_order: they must name other nodes of the plan, merge_order each at most once, without a cycle"
        }
        "mission_delivery_required" | "mission_delivery_invalid" => {
            "Give each change node a delivery (pr or direct) and no other node one"
        }
        "mission_cap_out_of_range" => "Use a concurrency cap from 1 to 16",
        "mission_agent_unknown" | "mission_model_invalid" | "mission_effort_invalid" => {
            "Run hide agent catalog to see the agents, models and efforts this machine takes"
        }
        "mission_model_missing" => {
            "Name the model of every worker that takes one: fill it from the candidate's fixed model, your choice from the catalog, or the catalog's cli_default_model"
        }
        "mission_question_without_line" => {
            "Give --why, --option, --recommend and --node with --line; --clear takes none of them"
        }
        "mission_options_too_many" => "Offer at most 5 options; leave the rest to the conversation",
        "mission_option_invalid" => "Write each option as one line of at most 120 characters",
        "mission_recommend_invalid" => {
            "Recommend an option by its number, counted from 1 in the order of --option"
        }
        "mission_field_kind_invalid" => {
            "Record --commit on a direct change node and --verdict or --checks on a verify node"
        }
        "mission_commit_invalid" => "Give the commit as 7 to 64 hexadecimal digits",
        "mission_checks_invalid" => {
            "Give --checks as <passed>/<total> with total above 0 and passed at most total"
        }
        "mission_text_invalid" | "mission_plan_invalid" => {
            "Check the command's text and JSON and retry"
        }
        "mission_criteria_missing" => {
            "Give the node one to three criteria of its own in its plan entry (\"criteria\": [\"...\"]); an external node takes none"
        }
        "mission_criteria_too_many" => {
            "Give the node at most three criteria of its own; Hide adds the check command, CI and the reviews itself"
        }
        "mission_checkers_invalid" => {
            "Name each of review and verify at most once per node, each with one to five agents in \"by\""
        }
        "mission_external_invalid" => {
            "An external node names one pull request (\"external\": {\"pr\": <n>}) or branch (\"external\": {\"branch\": \"<name>\"}) and no worker, delivery, criteria or checks; Hide marks it done when it reaches main, and you can only drop it"
        }
        "mission_checker_is_worker" => {
            "A node's checker is another agent than its worker: start a new agent for the review or verification and bind that one"
        }
        "mission_checker_unknown" => {
            "Bind a checker the node's plan names: its review or verify lists an unbound agent of the same kind as the registration (hide mission show)"
        }
        "mission_criterion_unknown" => {
            "Run hide mission show to see the node's lines and their keys (c1, check, ci, review, verify, d1…)"
        }
        "mission_criterion_not_yours" => {
            "Record only your own lines: a worker its node's own lines, check command and defects, a checker its review or verification; Hide records the CI, and the Conductor can only skip"
        }
        "mission_commit_required" => {
            "Give --commit <sha>, the commit the check command ran on (git rev-parse HEAD in the node's worktree)"
        }
        "mission_rerun_spent" => {
            "This commit already had its one rerun: fix the failure, commit, and run the check again on the new commit"
        }
        "mission_evidence_missing" => {
            "Give --evidence with the one line the result rests on: the command and what it printed, or what the review read"
        }
        "mission_checkout_unknown" => {
            "Record the check command from a pane in the checkout it ran in, on the mission's machine: Hide holds the line against that checkout's later commits"
        }
        "mission_skip_reason_missing" => "Give the reason with --reason <text>",
        "mission_criteria_protected" => {
            "The check command and CI are skipped only on the operator's answer: ask with hide mission decide --line <question> --node <key>, and once answered skip with --decision <its key>"
        }
        "mission_criteria_unmet" => {
            "Finish the remaining lines first: have the owner finish the check and record it, skip a line with hide mission skip <node> <line> --reason <text>, or ask the operator with hide mission decide --node <key>"
        }
        "mission_nodes_unfinished" => {
            "Every node is done or dropped before the mission closes as done or merge_wait: finish them, drop them with hide mission node <key> --status dropped, or close as cancelled"
        }
        "mission_defects_too_many" => {
            "The node holds 16 defects; ask the operator how to go on with it"
        }
        "mission_candidate_refused" => {
            "Start the worker as one of the operator's candidates: run hide mission settings for the agents, models and efforts it allows"
        }
        "mission_cap_reached" => {
            "Wait for a worker to finish (end its registration with hide agent end <id>) and retry; a start that failed stops counting after 10 minutes"
        }
        "mission_projects_loading" => {
            "Retry in a few seconds; Hide is still reading this machine's projects"
        }
        "mission_project_unknown" => {
            "Run the command from a pane in a checkout of a registered project"
        }
        "mission_unavailable" => {
            "Stop and tell the operator: Hide could not read its mission record (the reason is in Hide's diagnostic log); it is kept as it is, and Hide reads it again when it restarts"
        }
        "agent_pane_required" => "Run the command from an agent's pane",
        "native_identity_required" => "Retry after the agent's session is observed",
        "caller_context_changed" => "Run hide workspace info and retry",
        _ => "Retry; the reason is in Hide's diagnostic log",
    }
}

#[derive(Deserialize, Serialize)]
struct Stored {
    schema_version: u32,
    #[serde(flatten)]
    book: MissionBook,
}

fn encode(book: &Arc<MissionBook>) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&Stored {
        schema_version: SCHEMA_VERSION,
        book: MissionBook::clone(book),
    })
    .map_err(|error| format!("could not be encoded: {error}"))
}

/// A book read back, and the schema it was written in.
struct Decoded {
    book: MissionBook,
    from: u32,
}

fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let decoded = |error: serde_json::Error| format!("could not be decoded: {error}");
    let mut value: serde_json::Value = serde_json::from_slice(bytes).map_err(decoded)?;
    let from = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or("has no schema version")?;
    match from {
        SCHEMA_VERSION => {}
        SCHEMA_BEFORE_CRITERIA => convert_before_criteria(&mut value)?,
        other => {
            return Err(format!(
                "has schema version {other}, expected {SCHEMA_VERSION}"
            ));
        }
    }
    let stored: Stored = serde_json::from_value(value).map_err(decoded)?;
    if stored.book.missions.len() > MISSION_LIMIT
        || stored
            .book
            .missions
            .iter()
            .any(|mission| mission.nodes.len() > NODE_LIMIT)
    {
        return Err("holds more missions or nodes than the limits allow".to_owned());
    }
    Ok(Decoded {
        book: stored.book,
        from,
    })
}

/// A schema 1 book as schema 2 reads it (D-19, B23): every mission was made
/// before node criteria and is exempt from their gate, and a node's one
/// sentence, where it had one, is its one line.
fn convert_before_criteria(value: &mut serde_json::Value) -> Result<(), String> {
    let missions = match value.get_mut("missions") {
        None | Some(serde_json::Value::Null) => return Ok(()),
        Some(serde_json::Value::Array(missions)) => missions,
        Some(_) => return Err("has a mission list that is not a list".to_owned()),
    };
    for mission in missions {
        let mission = mission
            .as_object_mut()
            .ok_or("has a mission that is not an object")?;
        mission.insert("exempt".into(), serde_json::Value::Bool(true));
        let Some(nodes) = mission
            .get_mut("nodes")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        for node in nodes {
            let node = node
                .as_object_mut()
                .ok_or("has a node that is not an object")?;
            let lines = match node.remove("criteria") {
                None | Some(serde_json::Value::Null) => Vec::new(),
                Some(serde_json::Value::String(text)) => {
                    let mut line =
                        Criterion::new("c1", crate::mission_criteria::CriterionKind::Own);
                    line.text = Some(text);
                    vec![line]
                }
                Some(_) => return Err("has a node criteria that is not a sentence".to_owned()),
            };
            node.insert(
                "criteria".into(),
                serde_json::to_value(lines).map_err(|error| error.to_string())?,
            );
        }
    }
    Ok(())
}

/// The size of the book as it would be saved, counted without keeping the
/// bytes and stopped just past `limit`, so a check costs at most that much
/// encoding.
fn saved_size(book: &MissionBook, limit: usize) -> usize {
    struct Counter(usize, usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            if self.0 > self.1 {
                return Err(std::io::Error::other("over budget"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0, limit);
    let _ = serde_json::to_writer(&mut counter, book);
    counter.0
}

/// Whether the book, as it would be saved, stays within what a start reads
/// back.
pub(crate) fn fits(book: &MissionBook) -> bool {
    saved_size(book, WRITE_BUDGET) <= WRITE_BUDGET
}

/// Whether a change from `before` to `after` may be kept: one that stays in
/// the budget, or one that makes a book already past it smaller, so a
/// close or a smaller plan is never refused.
pub(crate) fn may_keep(before: &MissionBook, after: &MissionBook) -> bool {
    // A book past the budget came from a file the read cap let in, so both
    // are counted to just past that cap.
    let limit = READ_CAP as usize;
    fits(after) || saved_size(after, limit) < saved_size(before, limit)
}

/// Where a restored book came from: a file this build cannot use is kept
/// aside rather than replaced, so a downgrade does not erase missions.
pub(crate) enum Restored {
    Fresh,
    Read(MissionBook),
    Unreadable(String),
}

pub(crate) struct MissionStore {
    path: PathBuf,
    file: LatestFile<Arc<MissionBook>>,
}

impl MissionStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            file: LatestFile::new(
                path.clone(),
                "hide-mission-save",
                "missions",
                "missions",
                encode,
            ),
            path,
        }
    }

    /// Reads the book at launch, before the runtime lock is taken. A schema
    /// 1 book is converted, and the file it came from is copied aside once
    /// before anything writes schema 2 over it.
    pub(crate) fn restore(&self) -> Restored {
        match read_capped(&self.path, READ_CAP) {
            Ok(None) => Restored::Fresh,
            Ok(Some(bytes)) => match decode(&bytes) {
                Ok(Decoded { book, from }) => {
                    if from != SCHEMA_VERSION {
                        self.keep_before_conversion(&bytes, from);
                    }
                    Restored::Read(book)
                }
                Err(reason) => Restored::Unreadable(reason),
            },
            Err(reason) => Restored::Unreadable(reason),
        }
    }

    /// The copy a downgrade is restored from; an existing copy is the
    /// earliest one and is kept.
    fn keep_before_conversion(&self, bytes: &[u8], from: u32) {
        let Some(directory) = self.path.parent() else {
            return;
        };
        let backup = hide_kit::layout::missions_before_criteria(directory);
        let outcome = crate::latest_file::keep_first_copy(&backup, bytes);
        crate::diagnostic!(serde_json::json!({
            "component": "missions",
            "kind": "missions.converted",
            "from": from,
            "to": SCHEMA_VERSION,
            "backup": backup.to_string_lossy(),
            "written": outcome.as_ref().ok(),
            "message": outcome.as_ref().err().map(ToString::to_string),
        }));
    }

    pub(crate) fn save(&self, book: &MissionBook) {
        self.file.save(Arc::new(book.clone()));
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(pane: &str) -> Actor {
        Actor {
            pane_id: pane.into(),
            name: "conductor".into(),
            kind: "claude".into(),
            device_id: "node-a".into(),
            session: Some("s".into()),
        }
    }

    fn mission() -> Mission {
        Mission {
            id: "mission-1".into(),
            title: "issues".into(),
            project: "/repo".into(),
            workspace_id: "ws-1".into(),
            checkout_path: "/repo".into(),
            device_id: "node-a".into(),
            conductor: Conductor {
                actor: actor("w1:p1"),
                session: None,
                agent_id: None,
            },
            status: MissionStatus::Planning,
            scope: MissionScope::Pr,
            cap: 4,
            candidates: Vec::new(),
            checkers: None,
            exempt: false,
            decided: Vec::new(),
            nodes: Vec::new(),
            merge_order: Vec::new(),
            check: None,
            github: None,
            decision: None,
            question: None,
            question_seq: 0,
            answer: None,
            answer_failed: None,
            delay: None,
            report: None,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 1,
            started_at_unix_ms: None,
            closed_at_unix_ms: None,
            wakes: Vec::new(),
            wake_seq: 0,
        }
    }

    /// A book is only kept when its save stays under what a start reads
    /// back, so the file Hide writes never comes back unreadable.
    #[test]
    fn a_book_past_what_a_start_reads_back_does_not_fit() {
        let mut large = mission();
        large.report = Some("x".repeat(WRITE_BUDGET / 2));
        let mut book = MissionBook::default();
        book.missions.push(large.clone());
        assert!(fits(&book));
        let before = book.clone();
        book.missions.push(large);
        assert!(!fits(&book));
        assert!(!may_keep(&before, &book));
        // A change that makes a book already past the budget smaller is kept.
        let mut over = book.clone();
        over.missions.push(over.missions[0].clone());
        let mut smaller = over.clone();
        smaller.missions[2].report = Some("x".into());
        assert!(!fits(&smaller));
        assert!(may_keep(&over, &smaller));
        // A move that finds a mission's registration gone writes null.
        let mut stored = serde_json::to_value(&book.missions[0]).unwrap();
        stored["workspace_id"] = serde_json::Value::Null;
        // A book saved while the first plan still opened the map carries
        // its `opened` mark, which reads as nothing now (mission-pane-chip D-11).
        stored["opened"] = serde_json::Value::Bool(true);
        let read: Mission = serde_json::from_value(stored).unwrap();
        assert_eq!(read.workspace_id, "");
        assert!((WRITE_BUDGET as u64) < READ_CAP);
    }

    fn node(key: &str, kind: NodeKind, depends: &[&str]) -> NodeInput {
        NodeInput {
            key: key.into(),
            title: format!("node {key}"),
            kind,
            depends_on: depends.iter().map(|key| (*key).to_owned()).collect(),
            delivery: (kind == NodeKind::Change).then_some(Delivery::Pr),
            worker: None,
            criteria: vec![format!("{key} works")],
            checks: Vec::new(),
            external: None,
            issues: Vec::new(),
            paths: Vec::new(),
            change: None,
        }
    }

    fn plan(nodes: Vec<NodeInput>) -> PlanInput {
        PlanInput {
            title: None,
            scope: None,
            cap: None,
            github: None,
            check: None,
            merge_order: Vec::new(),
            nodes,
        }
    }

    #[test]
    fn a_plan_with_a_cycle_or_an_unknown_dependency_is_refused() {
        let cycle = plan(vec![
            node("a", NodeKind::Change, &["b"]),
            node("b", NodeKind::Change, &["a"]),
        ]);
        assert_eq!(
            apply_plan(&mission(), &cycle, 2).unwrap_err(),
            "mission_dependency_cycle"
        );
        let unknown = plan(vec![node("a", NodeKind::Change, &["z"])]);
        assert_eq!(
            apply_plan(&mission(), &unknown, 2).unwrap_err(),
            "mission_dependency_unknown"
        );
        let mut repeated = plan(vec![node("a", NodeKind::Change, &[])]);
        repeated.merge_order = vec!["a".into(), "a".into()];
        assert_eq!(
            apply_plan(&mission(), &repeated, 2).unwrap_err(),
            "mission_dependency_unknown"
        );
    }

    #[test]
    fn only_a_change_node_carries_a_delivery() {
        let mut research = node("r", NodeKind::Research, &[]);
        research.delivery = Some(Delivery::Pr);
        assert_eq!(
            apply_plan(&mission(), &plan(vec![research]), 2).unwrap_err(),
            "mission_delivery_invalid"
        );
        let mut change = node("c", NodeKind::Change, &[]);
        change.delivery = None;
        assert_eq!(
            apply_plan(&mission(), &plan(vec![change]), 2).unwrap_err(),
            "mission_delivery_required"
        );
    }

    #[test]
    fn a_replan_keeps_what_a_node_did_and_refuses_to_drop_a_running_node_silently() {
        let first = apply_plan(
            &mission(),
            &plan(vec![
                node("a", NodeKind::Change, &[]),
                node("b", NodeKind::Change, &["a"]),
            ]),
            2,
        )
        .unwrap();
        let mut running = first.clone();
        running.nodes[0] = apply_patch(
            &running.nodes[0],
            &NodePatch {
                status: Some(NodeStatus::Working),
                pr: Some(Some(7)),
                ..NodePatch::default()
            },
            3,
        )
        .unwrap();
        let again = apply_plan(
            &running,
            &plan(vec![
                node("a", NodeKind::Change, &[]),
                node("c", NodeKind::Research, &[]),
            ]),
            4,
        )
        .unwrap();
        assert_eq!(again.nodes[0].pr, Some(7));
        assert_eq!(again.nodes[0].status, NodeStatus::Working);
        assert!(again.node("b").is_none(), "a planned node left out goes");
        let without = apply_plan(&running, &plan(vec![node("c", NodeKind::Research, &[])]), 4);
        assert_eq!(without.unwrap_err(), "mission_node_in_progress");
        // A node whose status still says planned but that names its pull
        // request ran too.
        let mut named = first.clone();
        named.nodes[1].pr = Some(8);
        let dropped = apply_plan(&named, &plan(vec![node("a", NodeKind::Change, &[])]), 4);
        assert_eq!(dropped.unwrap_err(), "mission_node_in_progress");
    }

    #[test]
    fn a_failed_or_dropped_node_holds_everything_downstream_of_it() {
        let mut m = apply_plan(
            &mission(),
            &plan(vec![
                node("a", NodeKind::Change, &[]),
                node("b", NodeKind::Change, &["a"]),
                node("c", NodeKind::Verify, &["b"]),
                node("d", NodeKind::Change, &[]),
            ]),
            2,
        )
        .unwrap();
        m.nodes[0].status = NodeStatus::Failed;
        let blocked = m.blocked();
        assert_eq!(blocked.get("b"), Some(&Blocked::UpstreamFailed));
        assert_eq!(blocked.get("c"), Some(&Blocked::UpstreamFailed));
        assert_eq!(blocked.get("d"), None);
        m.nodes[0].status = NodeStatus::Dropped;
        assert_eq!(m.blocked().get("c"), Some(&Blocked::UpstreamDropped));
    }

    #[test]
    fn a_decision_or_delay_without_its_line_is_refused_and_null_clears() {
        for op in ["decide", "delay"] {
            assert!(
                serde_json::from_value::<MissionCommand>(serde_json::json!({"op": op})).is_err()
            );
            assert!(
                serde_json::from_value::<MissionCommand>(
                    serde_json::json!({"op": op, "line": null})
                )
                .is_ok()
            );
        }
    }

    #[test]
    fn a_patch_null_clears_and_an_absent_field_is_kept() {
        let m = apply_plan(&mission(), &plan(vec![node("a", NodeKind::Change, &[])]), 2).unwrap();
        let set: NodePatch =
            serde_json::from_value(serde_json::json!({"line": "running tests", "pr": 3})).unwrap();
        let after = apply_patch(&m.nodes[0], &set, 3).unwrap();
        assert_eq!(after.line.as_deref(), Some("running tests"));
        let clear: NodePatch = serde_json::from_value(serde_json::json!({"line": null})).unwrap();
        let cleared = apply_patch(&after, &clear, 4).unwrap();
        assert_eq!(cleared.line, None);
        assert_eq!(cleared.pr, Some(3));
    }

    #[test]
    fn a_patch_sent_as_the_cli_sends_it_keeps_what_it_does_not_name() {
        let mut planned = node("a", NodeKind::Change, &[]);
        planned.worker = Some(Worker {
            agent: "claude".into(),
            model: Some("opus".into()),
            effort: None,
            effort_auto: false,
        });
        let m = apply_plan(&mission(), &plan(vec![planned]), 2).unwrap();
        // The CLI builds the patch in Rust and the daemon reads it back from JSON.
        let wire = |patch: NodePatch| -> NodePatch {
            serde_json::from_value(serde_json::to_value(patch).unwrap()).unwrap()
        };
        let set = wire(NodePatch {
            pr: Some(Some(3)),
            agent_id: Some(Some("agent-3".into())),
            ..Default::default()
        });
        let after = apply_patch(&m.nodes[0], &set, 3).unwrap();
        let status = wire(NodePatch {
            status: Some(NodeStatus::Review),
            ..Default::default()
        });
        let reviewed = apply_patch(&after, &status, 4).unwrap();
        assert_eq!(
            reviewed.worker.as_ref().map(|worker| worker.agent.as_str()),
            Some("claude")
        );
        assert_eq!(reviewed.pr, Some(3));
        assert_eq!(reviewed.agent_id.as_deref(), Some("agent-3"));
        let cleared = apply_patch(
            &reviewed,
            &wire(NodePatch {
                pr: Some(None),
                ..Default::default()
            }),
            5,
        )
        .unwrap();
        assert_eq!(cleared.pr, None);
        assert_eq!(cleared.agent_id.as_deref(), Some("agent-3"));
    }

    #[test]
    fn another_open_mission_on_the_same_issue_or_folder_is_an_overlap() {
        let mut book = MissionBook::default();
        let mut other = mission();
        other.id = "mission-2".into();
        other.title = "other".into();
        other.status = MissionStatus::Running;
        let mut theirs = node("x", NodeKind::Change, &[]);
        theirs.issues = vec![12];
        theirs.paths = vec!["web/src".into()];
        other = apply_plan(&other, &plan(vec![theirs]), 2).unwrap();
        book.missions.push(other);
        let mut mine = node("y", NodeKind::Change, &[]);
        mine.paths = vec!["web/src/App.tsx".into()];
        let m = apply_plan(&mission(), &plan(vec![mine]), 2).unwrap();
        let found = overlaps(&book, &m);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].paths, vec!["web/src/App.tsx".to_owned()]);
        // The same path on another machine is another folder.
        let mut elsewhere = m.clone();
        elsewhere.device_id = "node-b".into();
        assert!(overlaps(&book, &elsewhere).is_empty());
        let mut unrelated = node("z", NodeKind::Change, &[]);
        unrelated.paths = vec!["web/srcs".into()];
        let m = apply_plan(&mission(), &plan(vec![unrelated]), 2).unwrap();
        assert!(overlaps(&book, &m).is_empty());
    }

    #[test]
    fn a_book_past_its_limits_or_of_another_version_is_not_used() {
        let book = MissionBook {
            missions: vec![mission(); MISSION_LIMIT + 1],
            next: 1,
        };
        let bytes = encode(&Arc::new(book)).unwrap();
        assert!(decode(&bytes).is_err());
        let other = serde_json::to_vec(
            &serde_json::json!({"schema_version": 99, "missions": [], "next": 0}),
        )
        .unwrap();
        assert!(decode(&other).is_err());
        let ok = encode(&Arc::new(MissionBook::default())).unwrap();
        assert_eq!(decode(&ok).unwrap().book, MissionBook::default());
    }

    #[test]
    fn a_plan_names_one_to_three_criteria_per_node_and_an_external_node_none() {
        let refused = |plan: PlanInput| {
            let code = apply_plan(&mission(), &plan, 2).unwrap_err();
            crate::coordination::split_refusal(&code).0.to_owned()
        };
        let mut bare = node("a", NodeKind::Change, &[]);
        bare.criteria.clear();
        assert_eq!(
            refused(plan(vec![bare.clone()])),
            "mission_criteria_missing"
        );
        let mut four = node("a", NodeKind::Change, &[]);
        four.criteria = vec!["1".into(), "2".into(), "3".into(), "4".into()];
        assert_eq!(refused(plan(vec![four])), "mission_criteria_too_many");
        // A mission made before criteria plans as it did.
        let mut old = mission();
        old.exempt = true;
        assert!(apply_plan(&old, &plan(vec![bare]), 2).is_ok());
        let mut external = node("pr", NodeKind::External, &[]);
        external.delivery = None;
        external.criteria.clear();
        external.external = Some(External::Pr(1232));
        let mut with_worker = external.clone();
        with_worker.criteria = vec!["x".into()];
        assert_eq!(refused(plan(vec![with_worker])), "mission_external_invalid");
        let mut unnamed = external.clone();
        unnamed.external = None;
        assert_eq!(refused(plan(vec![unnamed])), "mission_external_invalid");
        let mut named = node("a", NodeKind::Change, &["pr"]);
        named.external = Some(External::Branch("feat/x".into()));
        assert_eq!(
            refused(plan(vec![external.clone(), named])),
            "mission_external_invalid"
        );
        let waits = node("a", NodeKind::Change, &["pr"]);
        let planned = apply_plan(&mission(), &plan(vec![external, waits]), 2).unwrap();
        assert!(planned.nodes[0].criteria.is_empty());
        // Each of review and verify at most once.
        let mut twice = node("a", NodeKind::Change, &[]);
        let review = CheckInput {
            kind: crate::mission_criteria::CheckKind::Review,
            rule: Rule::All,
            by: vec![Worker {
                agent: "codex".into(),
                model: Some("gpt-6-astra".into()),
                effort: None,
                effort_auto: false,
            }],
        };
        twice.checks = vec![review.clone(), review];
        assert_eq!(refused(plan(vec![twice])), "mission_checkers_invalid");
    }

    #[test]
    fn a_plan_with_a_check_command_adds_it_to_change_nodes_and_keeps_what_was_recorded() {
        let mut input = plan(vec![
            node("a", NodeKind::Change, &[]),
            node("r", NodeKind::Research, &[]),
        ]);
        input.check = Some("cargo test".into());
        let mut planned = apply_plan(&mission(), &input, 2).unwrap();
        let keys = |node: &MissionNode| -> Vec<String> {
            node.criteria.iter().map(|line| line.key.clone()).collect()
        };
        assert_eq!(keys(&planned.nodes[0]), ["c1", "check"]);
        assert_eq!(keys(&planned.nodes[1]), ["c1"]);
        planned.nodes[0].criteria[0].state = crate::mission_criteria::CriterionState::Pass;
        // A plan given as one sentence, as one written before criteria is.
        let again: NodeInput = serde_json::from_value(serde_json::json!({
            "key": "a", "title": "node a", "kind": "change", "delivery": "pr",
            "criteria": "a works"
        }))
        .unwrap();
        let replanned = apply_plan(&planned, &plan(vec![again]), 3).unwrap();
        assert_eq!(
            replanned.nodes[0].criteria[0].state,
            crate::mission_criteria::CriterionState::Pass
        );
    }

    /// D-19, B23: a schema 1 book reads as exempt missions whose node keeps
    /// its sentence as its one line, and its file is copied aside once.
    #[test]
    fn a_schema_one_book_is_converted_and_its_file_kept_aside() {
        let mut node = serde_json::to_value(mission()).unwrap();
        node["nodes"] = serde_json::json!([
            {"key": "a", "title": "A", "kind": "change", "depends_on": [], "status": "done",
             "delivery": "pr", "worker": null, "agent_id": null, "pr": 3, "base_pr": null,
             "branch": null, "attempts": 0, "line": null, "change": null, "review": null,
             "criteria": "CI and review", "issues": [], "paths": [],
             "started_at_unix_ms": null, "finished_at_unix_ms": null, "pr_seen": null},
            {"key": "b", "title": "B", "kind": "research", "depends_on": [], "status": "planned",
             "criteria": null}
        ]);
        let mut v1 = node.clone();
        v1.as_object_mut().unwrap().remove("exempt");
        let bytes = serde_json::to_vec(
            &serde_json::json!({"schema_version": 1, "next": 1, "missions": [v1]}),
        )
        .unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("missions.json");
        std::fs::write(&path, &bytes).unwrap();
        let store = MissionStore::new(path.clone());
        let Restored::Read(book) = store.restore() else {
            panic!("a schema 1 book reads");
        };
        let read = &book.missions[0];
        assert!(read.exempt);
        assert_eq!(read.nodes[0].criteria.len(), 1);
        assert_eq!(
            read.nodes[0].criteria[0].text.as_deref(),
            Some("CI and review")
        );
        assert!(read.nodes[1].criteria.is_empty());
        let backup = hide_kit::layout::missions_before_criteria(folder.path());
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);
        // The first copy is the one kept.
        std::fs::write(&path, b"{\"schema_version\": 1, \"missions\": []}").unwrap();
        assert!(matches!(store.restore(), Restored::Read(_)));
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);
        // What is saved is schema 2 and reads back the same.
        let saved = encode(&Arc::new(book.clone())).unwrap();
        let again = decode(&saved).unwrap();
        assert_eq!((again.from, again.book), (SCHEMA_VERSION, book));
    }

    #[test]
    fn settings_refuse_an_agent_or_effort_the_agent_does_not_take() {
        let mut settings = ConductorSettings::default();
        assert!(settings.validate().is_ok());
        settings.concurrency = 0;
        assert!(settings.validate().is_err());
        settings.concurrency = 4;
        settings.candidates.push(Candidate {
            agent: "opencode".into(),
            model: None,
            effort: Some("high".into()),
            purpose: String::new(),
        });
        assert_eq!(
            settings.validate().unwrap_err(),
            "conductor_settings.effort_invalid"
        );
        settings.candidates[0] = Candidate {
            agent: "claude".into(),
            model: Some("opus".into()),
            effort: Some("high".into()),
            purpose: "screens".into(),
        };
        assert!(settings.validate().is_ok());
    }
}
