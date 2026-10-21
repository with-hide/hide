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

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use crate::delivery::Actor;
use crate::latest_file::{LatestFile, read_capped};

const SCHEMA_VERSION: u32 = 1;
/// Missions kept at once, closed ones included. A mission is deleted with
/// its Conductor's session (D-22), so a full book is refused, never evicted.
pub const MISSION_LIMIT: usize = 128;
/// Nodes in one mission's plan.
pub const NODE_LIMIT: usize = 64;
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
const KEY_LIMIT: usize = 40;
const READ_CAP: u64 = 8 * 1024 * 1024;

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

/// The three kinds of node, each finished by its own rule (D-15).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Change,
    Research,
    Verify,
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
}

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
    /// The node's own finish rule when the plan changed the default (B29).
    #[serde(default)]
    pub criteria: Option<String>,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Mission {
    pub id: String,
    pub title: String,
    /// The main worktree of the project the mission belongs to, on
    /// `device_id`, the key GitHub answers are read by.
    pub project: String,
    /// The catalog project the map's pull request cards open in.
    pub workspace_id: String,
    /// The checkout the mission was created in, where a takeover starts.
    #[serde(default)]
    pub checkout_path: String,
    pub device_id: String,
    pub conductor: Conductor,
    pub status: MissionStatus,
    pub scope: MissionScope,
    pub cap: u8,
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
    /// The map opened itself for the first plan already (D-43).
    #[serde(default)]
    pub opened: bool,
}

impl Mission {
    pub(crate) fn node(&self, key: &str) -> Option<&MissionNode> {
        self.nodes.iter().find(|node| node.key == key)
    }

    /// Why each node cannot start, from a failed or dropped node anywhere
    /// upstream of it.
    pub(crate) fn blocked(&self) -> BTreeMap<String, Blocked> {
        let by_key: BTreeMap<&str, &MissionNode> = self
            .nodes
            .iter()
            .map(|node| (node.key.as_str(), node))
            .collect();
        let mut out = BTreeMap::new();
        for node in &self.nodes {
            if node.status.finished() {
                continue;
            }
            let mut seen = BTreeSet::new();
            let mut stack: Vec<&str> = node.depends_on.iter().map(String::as_str).collect();
            let mut found = None;
            while let Some(key) = stack.pop() {
                if !seen.insert(key) {
                    continue;
                }
                let Some(upstream) = by_key.get(key) else {
                    continue;
                };
                match upstream.status {
                    NodeStatus::Failed => {
                        found = Some(Blocked::UpstreamFailed);
                        break;
                    }
                    NodeStatus::Dropped => found = found.or(Some(Blocked::UpstreamDropped)),
                    _ => stack.extend(upstream.depends_on.iter().map(String::as_str)),
                }
            }
            if let Some(found) = found {
                out.insert(node.key.clone(), found);
            }
        }
        out
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
}

impl Default for ConductorSettings {
    fn default() -> Self {
        Self {
            concurrency: DEFAULT_CAP,
            default_scope: MissionScope::Pr,
            candidates: Vec::new(),
        }
    }
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
        Ok(())
    }
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
    /// Sets or clears the operator's one line (B17).
    Decide {
        #[serde(default)]
        mission: Option<String>,
        #[serde(default)]
        line: Option<String>,
    },
    /// Sets or clears why new nodes wait on usage (B13).
    Delay {
        #[serde(default)]
        mission: Option<String>,
        #[serde(default)]
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
            Self::Decide { mission, line } | Self::Delay { mission, line } => {
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
    #[serde(default)]
    pub criteria: Option<String>,
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
}

fn double<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
}

pub fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= KEY_LIMIT
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

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
        if !valid_key(&node.key) || !keys.insert(node.key.as_str()) {
            return Err("mission_node_key_invalid".into());
        }
        if !valid_text(&node.title, TITLE_LIMIT)
            || !valid_line(&node.criteria, LINE_LIMIT)
            || !valid_line(&node.change, LINE_LIMIT)
            || node.depends_on.len() > DEPENDENCY_LIMIT
            || node.issues.len() > TOUCH_LIMIT
            || node.paths.len() > TOUCH_LIMIT
            || node.paths.iter().any(|path| !valid_text(path, LINE_LIMIT))
        {
            return Err("mission_text_invalid".into());
        }
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
        }
    }
    for node in &input.nodes {
        if node
            .depends_on
            .iter()
            .any(|key| key == &node.key || !keys.contains(key.as_str()))
        {
            return Err("mission_dependency_unknown".into());
        }
    }
    if input
        .merge_order
        .iter()
        .any(|key| !keys.contains(key.as_str()))
    {
        return Err("mission_dependency_unknown".into());
    }
    if cyclic(&input.nodes) {
        return Err("mission_dependency_cycle".into());
    }
    // A node that ran keeps its record; one that is gone from the plan must
    // be dropped first, so a replan cannot hide work that was done (B28).
    for node in &mission.nodes {
        if node.status != NodeStatus::Planned && !keys.contains(node.key.as_str()) {
            return Err("mission_node_in_progress".into());
        }
    }
    let mut next = mission.clone();
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
                criteria: None,
                issues: Vec::new(),
                paths: Vec::new(),
                started_at_unix_ms: None,
                finished_at_unix_ms: None,
                pr_seen: None,
            });
            node.title.clone_from(&input.title);
            node.kind = input.kind;
            node.depends_on.clone_from(&input.depends_on);
            node.delivery = input.delivery;
            node.worker.clone_from(&input.worker);
            node.criteria.clone_from(&input.criteria);
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
    if input.check.is_some() {
        next.check.clone_from(&input.check);
    }
    next.updated_at_unix_ms = now;
    Ok(next)
}

fn cyclic(nodes: &[NodeInput]) -> bool {
    let index: BTreeMap<&str, &NodeInput> =
        nodes.iter().map(|node| (node.key.as_str(), node)).collect();
    // 0 unvisited, 1 on the path, 2 done.
    let mut marks: BTreeMap<&str, u8> = BTreeMap::new();
    fn visit<'a>(
        key: &'a str,
        index: &BTreeMap<&'a str, &'a NodeInput>,
        marks: &mut BTreeMap<&'a str, u8>,
    ) -> bool {
        match marks.get(key) {
            Some(1) => return true,
            Some(2) => return false,
            _ => {}
        }
        marks.insert(key, 1);
        if let Some(node) = index.get(key) {
            for dependency in &node.depends_on {
                if visit(dependency, index, marks) {
                    return true;
                }
            }
        }
        marks.insert(key, 2);
        false
    }
    nodes
        .iter()
        .any(|node| visit(&node.key, &index, &mut marks))
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
    if let Some(Some(worker)) = &patch.worker {
        check_worker(
            &worker.agent,
            worker.model.as_deref(),
            worker.effort.as_deref(),
        )
        .map_err(|code| format!("mission_{code}"))?;
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
    if patch.pr.is_some() && next.pr != node.pr {
        // A new pull request starts its own wakes from what GitHub says now.
        next.pr_seen = None;
    }
    if let Some(attempts) = patch.attempts {
        next.attempts = attempts;
    }
    Ok(next)
}

/// Where another open mission touches the same issue or path as one of this
/// mission's nodes (D-18, B6): `(other mission, its title, what overlaps)`.
pub(crate) fn overlaps(book: &MissionBook, mission: &Mission) -> Vec<Overlap> {
    let mut out = Vec::new();
    for other in &book.missions {
        if other.id == mission.id || !other.status.open() || other.project != mission.project {
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
            "Check depends_on and merge_order: they must name other nodes of the plan without a cycle"
        }
        "mission_delivery_required" | "mission_delivery_invalid" => {
            "Give each change node a delivery (pr or direct) and no other node one"
        }
        "mission_cap_out_of_range" => "Use a concurrency cap from 1 to 16",
        "mission_agent_unknown" | "mission_model_invalid" | "mission_effort_invalid" => {
            "Run hide agent catalog to see the agents, models and efforts this machine takes"
        }
        "mission_text_invalid" | "mission_plan_invalid" => {
            "Check the command's text and JSON and retry"
        }
        "mission_cap_reached" => {
            "Wait for a worker to finish (end its registration with hide agent end <id>) and retry"
        }
        "mission_projects_loading" => {
            "Retry in a few seconds; Hide is still reading this machine's projects"
        }
        "mission_project_unknown" => {
            "Run the command from a pane in a checkout of a registered project"
        }
        "mission_save_failed" | "mission_unavailable" => {
            "Retry; the reason is in Hide's diagnostic log"
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

fn decode(bytes: &[u8]) -> Result<MissionBook, String> {
    let stored: Stored =
        serde_json::from_slice(bytes).map_err(|error| format!("could not be decoded: {error}"))?;
    if stored.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "has schema version {}, expected {SCHEMA_VERSION}",
            stored.schema_version
        ));
    }
    if stored.book.missions.len() > MISSION_LIMIT
        || stored
            .book
            .missions
            .iter()
            .any(|mission| mission.nodes.len() > NODE_LIMIT)
    {
        return Err("holds more missions or nodes than the limits allow".to_owned());
    }
    Ok(stored.book)
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

    pub(crate) fn restore(&self) -> Restored {
        match read_capped(&self.path, READ_CAP) {
            Ok(None) => Restored::Fresh,
            Ok(Some(bytes)) => match decode(&bytes) {
                Ok(book) => Restored::Read(book),
                Err(reason) => Restored::Unreadable(reason),
            },
            Err(reason) => Restored::Unreadable(reason),
        }
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
            nodes: Vec::new(),
            merge_order: Vec::new(),
            check: None,
            github: None,
            decision: None,
            delay: None,
            report: None,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 1,
            started_at_unix_ms: None,
            closed_at_unix_ms: None,
            opened: false,
        }
    }

    fn node(key: &str, kind: NodeKind, depends: &[&str]) -> NodeInput {
        NodeInput {
            key: key.into(),
            title: format!("node {key}"),
            kind,
            depends_on: depends.iter().map(|key| (*key).to_owned()).collect(),
            delivery: (kind == NodeKind::Change).then_some(Delivery::Pr),
            worker: None,
            criteria: None,
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
            model: None,
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
        assert_eq!(decode(&ok).unwrap(), MissionBook::default());
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
