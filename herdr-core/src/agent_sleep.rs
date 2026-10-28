//! Agent sleep: ending an agent that has sat untouched, and keeping its
//! conversation to resume in the same pane (PRD agent-sleep).
//!
//! This module decides and remembers; it makes no Herdr call and ends no
//! process. `agent_sleep_herdr.rs` is the one boundary that does, so when
//! Herdr offers a hibernate of its own only that file changes (D-20).
//!
//! The pinned Herdr forgets an agent whose process ended: it drops it from
//! `agent.list` together with its session ref. The record kept here is
//! therefore the only holder of the provider, the conversation id and the
//! row's name while the agent sleeps, and the sleeping row is drawn from it
//! (B10). Nothing here clears or rewrites anything Herdr keeps (B9).

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::model::{
    AgentActionRefusal, AgentSleepActionSnapshot, AgentSleepSnapshot, AgentStatusCode,
    SidebarAgentSnapshot,
};
use crate::sidebar::{
    AgentLabel, SessionAgentPayload, SessionAgentSessionPayload, SessionSnapshotPayload,
};

mod dormant;
#[cfg(test)]
pub(crate) use dormant::fixture_record;
pub use dormant::{
    DormantOwner, DormantPhase, DormantRecord, DormantRegistration, SleepId,
    SleepingSessionSnapshot,
};

/// The Settings choices besides Never, in hours (PRD D-10).
pub const SLEEP_AFTER_CHOICES_HOURS: [u32; 3] = [12, 24, 72];
/// The decision runs at most this often; the coordinator tick is 250 ms.
pub const DECISION_INTERVAL_MS: u64 = 60_000;
/// A pane whose agent would not end is not tried again for this long,
/// unless its state changes first (B8).
pub const SLEEP_RETRY_BACKOFF_MS: u64 = 60 * 60 * 1000;

const REMOTE_PANE_ID_PREFIX: &str = "remote:";

/// The machine an agent runs on, as sleep sees it. The core's own machine
/// and a node that dials the core sleep alike (PRD core-host-node-remote-core
/// B13); a device the core dials does not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SleepMachine {
    Core,
    /// A node whose link reached this core: its screen is the operator's.
    Node,
    Device,
}

/// The panes of one machine's session: the core's own, or a device's under
/// its `remote:<device>:pane:` prefix. A device session carries Herdr's own
/// pane ids, which the store keeps under the prefix.
#[derive(Clone, Copy, Debug)]
pub enum SleepScope<'a> {
    Core,
    Device(&'a str),
}

impl SleepScope<'_> {
    fn holds(self, pane_id: &str) -> bool {
        match self {
            Self::Core => !pane_id.starts_with(REMOTE_PANE_ID_PREFIX),
            Self::Device(prefix) => pane_id.starts_with(prefix),
        }
    }

    /// The pane id the machine's session names `pane_id` by.
    fn session_id(self, pane_id: &str) -> &str {
        match self {
            Self::Core => pane_id,
            Self::Device(prefix) => pane_id.strip_prefix(prefix).unwrap_or(pane_id),
        }
    }
}

pub fn valid_after_hours(hours: Option<u32>) -> bool {
    hours.is_none_or(|hours| SLEEP_AFTER_CHOICES_HOURS.contains(&hours))
}

/// Everything agent sleep keeps between launches, in `core-state.json`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentSleepStore {
    /// Each local agent pane's last state change and last look.
    #[serde(default)]
    pub stamps: BTreeMap<String, PaneStamps>,
    /// The agents Hide ended and holds to resume, by pane.
    #[serde(default)]
    pub records: BTreeMap<String, SleepRecord>,
    /// Sessions whose pane is intentionally closed, addressed by sleep id.
    #[serde(default, deserialize_with = "dormant::deserialize_records")]
    pub dormant: BTreeMap<SleepId, DormantRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PaneStamps {
    /// When Herdr last reported this agent's state change, or when it
    /// appeared (D-04).
    pub changed_at_unix_ms: u64,
    /// When the operator last had this pane on screen; zero for never.
    #[serde(default)]
    pub seen_at_unix_ms: u64,
    /// The state the change stamp describes; a different one is a change.
    pub state: StampedState,
}

impl PaneStamps {
    /// When the agent was last talked to or looked at. The conversation's
    /// own time is the clock once a session read proves it: a relaunch or a
    /// Herdr handoff can report a state for a moment and move the change
    /// stamp, but adds no line to the conversation. The change stamp counts
    /// only until that read lands.
    fn idle_since(&self, agent: &SidebarAgentSnapshot) -> u64 {
        agent
            .last_conversation_at_unix_ms
            .unwrap_or(self.changed_at_unix_ms)
            .max(self.seen_at_unix_ms)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StampedState {
    pub demand: String,
    pub activity: String,
    pub completed: bool,
}

impl StampedState {
    fn of(agent: &SidebarAgentSnapshot) -> Self {
        Self {
            demand: agent.demand.clone(),
            activity: agent.activity.clone(),
            completed: agent.completed,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SleepPhase {
    /// The end is in flight; the agent is still awake and drawn so (B8).
    Ending,
    Sleeping,
    Waking,
    /// The last wake failed; the pane says why and offers Retry (B14).
    Failed,
}

/// How a wake starts the agent again.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeMode {
    /// The same conversation, through the provider's own resume.
    Resume,
    /// Start new session: the same provider with no arguments (B14).
    Fresh,
}

/// One sleeping agent, as much of its row as it takes to draw it again.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SleepRecord {
    pub phase: SleepPhase,
    pub kind: String,
    /// The conversation a wake resumes; none for an agent that slept before
    /// it reported one (`sleeps_without_session`), which wakes as a new one.
    pub session_id: Option<String>,
    /// The agent's Herdr name, when it had one: orchestrators address agents
    /// by name, so a wake restarts it under the same one.
    #[serde(default)]
    pub agent_name: Option<String>,
    pub identity_label: String,
    /// Captured only from a current, guarded core row. Older sleep records
    /// have no label provenance and cannot bring a label back.
    #[serde(default)]
    pub label_owner: Option<String>,
    #[serde(default)]
    pub progress: Option<String>,
    /// The row's ordering key when it slept, so it keeps its place (B10);
    /// thirteen digits are the core's state change time, which the row's
    /// elapsed time is counted from.
    pub last_activity: String,
    /// Herdr's state change sequence of the agent that was ended. A listed
    /// agent on the pane with this sequence is that agent still being
    /// reported; any other sequence is a new agent (B18).
    #[serde(default)]
    pub state_change_seq: Option<u64>,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub parent_pane_id: Option<String>,
    /// The parent's session digest the declaration was written for. A record
    /// from before it was kept has none, and its row is a root until the agent
    /// wakes, because a declaration with no session cannot be proven.
    #[serde(default)]
    pub parent_session: Option<String>,
    /// The source a sleeping fork was forked from and the digest of the
    /// session it hosted (`SidebarAgentSnapshot::declared_fork`), so the row
    /// keeps its fork mark while Herdr lists no agent there.
    #[serde(default)]
    pub fork_source: Option<(String, Option<String>)>,
    pub workspace_label: String,
    #[serde(default)]
    pub cwd: Option<String>,
    pub since_unix_ms: u64,
    /// Why the last wake failed, in the operator's words.
    #[serde(default)]
    pub reason: Option<String>,
    /// A wake asked for while the end was still in flight; it runs as soon
    /// as the end lands, because a wake wins over a sleep (B13).
    #[serde(default)]
    pub pending_wake: Option<WakeMode>,
    /// The model and effort the agent was started with, which its wake
    /// starts it with again, a new session as much as a resumed one. A
    /// record from before it was kept has none and wakes on the CLI's
    /// defaults, as it did.
    #[serde(
        default,
        skip_serializing_if = "hide_agent_adapter::LaunchChoice::is_default"
    )]
    pub launch: hide_agent_adapter::LaunchChoice,
}

impl SleepRecord {
    /// The record a decision starts, taken from the row it decided on.
    pub fn ending(
        agent: &SidebarAgentSnapshot,
        cwd: Option<String>,
        since_unix_ms: u64,
    ) -> Option<Self> {
        if agent.session_id.is_none() && !sleeps_without_session(&agent.agent_kind) {
            return None;
        }
        let session_id = agent.session_id.clone();
        let label_owner = session_id.as_deref().and_then(|session_id| {
            hide_session::label_reference_token(&agent.agent_kind, "id", session_id)
        });
        Some(Self {
            phase: SleepPhase::Ending,
            kind: agent.agent_kind.clone(),
            session_id,
            // A device row's id is the core's own scoped one, never a name;
            // its Herdr name, when someone gave one, is `herdr_name`.
            agent_name: agent.herdr_name.clone().or_else(|| {
                (!agent.pane_id.starts_with(REMOTE_PANE_ID_PREFIX)
                    && agent.id != agent.pane_id
                    && !agent.id.trim().is_empty())
                .then(|| agent.id.clone())
            }),
            identity_label: agent.identity_label.clone(),
            label_owner,
            progress: agent.progress.clone(),
            last_activity: agent.last_activity.clone(),
            state_change_seq: agent.state_change_seq,
            completed: agent.completed,
            parent_pane_id: agent.spawned_from_pane_id.clone(),
            parent_session: agent.declared_parent_session.clone(),
            fork_source: agent.declared_fork.clone(),
            workspace_label: agent.workspace_label.clone(),
            cwd,
            since_unix_ms,
            reason: None,
            pending_wake: None,
            launch: Default::default(),
        })
    }

    /// What the row and the pane draw; nothing while the end is in flight.
    pub fn snapshot(&self) -> Option<AgentSleepSnapshot> {
        let state = match self.phase {
            SleepPhase::Ending => return None,
            SleepPhase::Sleeping => "sleeping",
            SleepPhase::Waking => "waking",
            SleepPhase::Failed => "failed",
        };
        Some(AgentSleepSnapshot {
            state: state.to_owned(),
            reason: (self.phase == SleepPhase::Failed)
                .then(|| self.reason.clone())
                .flatten(),
            since_unix_ms: self.since_unix_ms,
            progress: self.label_owner.as_ref().and(self.progress.clone()),
        })
    }

    /// The agent row Herdr no longer sends, drawn from what the record kept.
    fn payload(
        &self,
        pane_id: &str,
        workspace_label: Option<String>,
        scope: SleepScope<'_>,
    ) -> SessionAgentPayload {
        // Only a label the core had proven for this session comes back.
        let label = self.label_owner.as_ref().map(|_| AgentLabel {
            task: Some(self.identity_label.clone()),
            progress: self.progress.clone(),
            expected_reply: None,
            question: false,
            blocked: false,
        });
        // Any other key is Herdr's padded sequence, which the sequence below
        // reproduces.
        let changed_at_unix_ms = (self.last_activity.len() == 13)
            .then(|| self.last_activity.parse().ok())
            .flatten();
        SessionAgentPayload {
            record: None,
            telemetry_stale: false,
            launch_pending: false,
            id: Some(
                self.agent_name
                    .clone()
                    .unwrap_or_else(|| pane_id.to_owned()),
            ),
            name: self.agent_name.clone(),
            pane_id: Some(pane_id.to_owned()),
            workspace_label: workspace_label.or_else(|| Some(self.workspace_label.clone())),
            cwd: self.cwd.clone(),
            agent: Some(self.kind.clone()),
            agent_status: Some(if self.completed { "done" } else { "idle" }.to_owned()),
            agent_session: self
                .session_id
                .clone()
                .map(|value| SessionAgentSessionPayload {
                    kind: "id".to_owned(),
                    value,
                }),
            spawned_from_pane_id: self
                .parent_pane_id
                .as_deref()
                .map(|parent| scope.session_id(parent).to_owned()),
            spawned_from_machine_id: None,
            declared_parent_session: self.parent_session.clone(),
            lineage_session: self
                .session_id
                .as_deref()
                .and_then(crate::wire::session_digest),
            completion_seq: None,
            state_change_seq: self.state_change_seq,
            tokens: self
                .fork_source
                .as_ref()
                .map(|(source, source_session)| {
                    crate::wire::fork_tokens(
                        scope.session_id(source),
                        self.session_id
                            .as_deref()
                            .and_then(crate::wire::session_digest)
                            .as_deref(),
                        source_session.as_deref(),
                    )
                })
                .unwrap_or_default(),
            label,
            changed_at_unix_ms,
            facts: None,
        }
    }
}

/// The status a sleeping row carries in place of its own (B10).
pub fn status_code(sleep: &AgentSleepSnapshot) -> AgentStatusCode {
    match sleep.state.as_str() {
        "waking" => AgentStatusCode::Waking,
        "failed" => AgentStatusCode::SleepFailed,
        _ => AgentStatusCode::Sleeping,
    }
}

/// What happened to a record while a session update was settled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Settled {
    /// A new agent is running in the pane (a wake, the operator, a restore).
    Woke { pane_id: String, phase: SleepPhase },
    /// The pane is gone, and the record with it (B17).
    Closed { pane_id: String },
}

impl AgentSleepStore {
    /// A store read back at launch. An end that was in flight when the
    /// process stopped cannot be known to have happened, so its record goes
    /// and the pane is read as Herdr reports it; a wake in flight is asleep
    /// until an agent shows up in the pane.
    pub fn after_load(&mut self) {
        for record in self.dormant.values_mut() {
            record.after_load();
        }
        self.records
            .retain(|_, record| record.phase != SleepPhase::Ending);
        for record in self.records.values_mut() {
            if record.phase == SleepPhase::Waking {
                record.phase = SleepPhase::Sleeping;
            }
            record.pending_wake = None;
        }
    }

    /// Reconciles the records of one machine with its session and adds a row
    /// for each sleeping agent Herdr no longer lists. `payload` is that
    /// machine's whole topology, so a record of the machine whose pane it
    /// does not hold is a closed pane; another machine's records stay.
    pub fn settle_payload(
        &mut self,
        payload: &mut SessionSnapshotPayload,
        scope: SleepScope<'_>,
    ) -> Vec<Settled> {
        let live = payload
            .layouts
            .iter()
            .flat_map(|layout| {
                layout
                    .panes
                    .iter()
                    .map(move |pane| (pane.pane_id.as_str(), layout))
            })
            .collect::<HashMap<_, _>>();
        let mut settled = Vec::new();
        let mut woke = Vec::new();
        self.records.retain(|pane_id, record| {
            if !scope.holds(pane_id) {
                return true;
            }
            let session_pane_id = scope.session_id(pane_id);
            if !live.contains_key(session_pane_id) {
                settled.push(Settled::Closed {
                    pane_id: pane_id.clone(),
                });
                return false;
            }
            if record.phase == SleepPhase::Ending {
                return true;
            }
            if payload
                .panes
                .iter()
                .any(|pane| pane.pane_id == session_pane_id && pane.failed_terminal_id.is_some())
            {
                return true;
            }
            // Only an entry that names a provider and carries another state
            // sequence is a new agent. The ended agent can stay listed a
            // moment with its old sequence, and a pane whose metadata a
            // plugin reports stays listed with no provider at all.
            let new_agent = payload.agents.iter().any(|agent| {
                agent.pane_id.as_deref().or(agent.id.as_deref()) == Some(session_pane_id)
                    && agent
                        .agent
                        .as_deref()
                        .is_some_and(|kind| !kind.trim().is_empty())
                    && agent.state_change_seq != record.state_change_seq
            });
            if new_agent {
                settled.push(Settled::Woke {
                    pane_id: pane_id.clone(),
                    phase: record.phase,
                });
                woke.push(pane_id.clone());
                return false;
            }
            true
        });
        // The pane's clock starts again from the agent that came back (B16).
        for pane_id in &woke {
            self.stamps.remove(pane_id);
        }
        self.stamps.retain(|pane_id, _| {
            !scope.holds(pane_id) || live.contains_key(scope.session_id(pane_id))
        });
        let workspace_labels = payload
            .workspaces
            .iter()
            .map(|workspace| (workspace.workspace_id.as_str(), workspace.label.clone()))
            .collect::<HashMap<_, _>>();
        // A sleeping pane's row is drawn from its record alone, whatever
        // Herdr still lists for the pane.
        let mut rows = Vec::new();
        for (pane_id, record) in &self.records {
            if record.phase == SleepPhase::Ending || !scope.holds(pane_id) {
                continue;
            }
            let pane_id = scope.session_id(pane_id);
            payload
                .agents
                .retain(|agent| agent.pane_id.as_deref().or(agent.id.as_deref()) != Some(pane_id));
            let workspace_label = live
                .get(pane_id)
                .and_then(|layout| workspace_labels.get(layout.workspace_id.as_str()))
                .filter(|label| !label.trim().is_empty())
                .cloned();
            rows.push(record.payload(pane_id, workspace_label, scope));
        }
        payload.agents.extend(rows);
        settled
    }

    /// Marks each sleeping row so the projection draws the moon (B10).
    pub fn annotate(&self, agents: &mut [SidebarAgentSnapshot]) {
        for agent in agents {
            agent.sleep = self
                .records
                .get(&agent.pane_id)
                .and_then(SleepRecord::snapshot);
        }
    }

    /// Moves each awake agent's change stamp on one machine when its state
    /// changed or it appeared, and drops the stamps of that machine's panes
    /// that hold no agent now. Returns whether anything moved.
    pub fn stamp(
        &mut self,
        agents: &[SidebarAgentSnapshot],
        scope: SleepScope<'_>,
        now_unix_ms: u64,
    ) -> bool {
        let mut changed = false;
        let mut present = HashSet::new();
        for agent in agents {
            if !scope.holds(&agent.pane_id) {
                continue;
            }
            present.insert(agent.pane_id.as_str());
            if self.records.contains_key(&agent.pane_id) {
                continue;
            }
            let state = StampedState::of(agent);
            match self.stamps.get_mut(&agent.pane_id) {
                Some(stamps) if stamps.state == state => {}
                Some(stamps) => {
                    stamps.state = state;
                    stamps.changed_at_unix_ms = now_unix_ms;
                    changed = true;
                }
                None => {
                    self.stamps.insert(
                        agent.pane_id.clone(),
                        PaneStamps {
                            changed_at_unix_ms: now_unix_ms,
                            seen_at_unix_ms: 0,
                            state,
                        },
                    );
                    changed = true;
                }
            }
        }
        let before = self.stamps.len();
        self.stamps.retain(|pane_id, _| {
            !scope.holds(pane_id)
                || present.contains(pane_id.as_str())
                || self.records.contains_key(pane_id)
        });
        changed | (self.stamps.len() != before)
    }

    /// Records that the operator had these panes on screen now (B4).
    pub fn mark_seen<'a>(
        &mut self,
        pane_ids: impl IntoIterator<Item = &'a str>,
        now_unix_ms: u64,
    ) -> bool {
        let mut changed = false;
        for pane_id in pane_ids {
            if let Some(stamps) = self.stamps.get_mut(pane_id)
                && stamps.seen_at_unix_ms < now_unix_ms
            {
                stamps.seen_at_unix_ms = now_unix_ms;
                changed = true;
            }
        }
        changed
    }

    /// The panes whose agent the decision puts to sleep now (B4, B5, B6).
    pub fn due(
        &self,
        agents: &[SidebarAgentSnapshot],
        machine: SleepMachine,
        on_screen: &HashSet<String>,
        after_hours: Option<u32>,
        backoff_until: &HashMap<String, u64>,
        now_unix_ms: u64,
    ) -> Vec<String> {
        let Some(hours) = after_hours else {
            return Vec::new();
        };
        let after_ms = u64::from(hours) * 60 * 60 * 1000;
        agents
            .iter()
            .filter(|agent| sleep_refusal(agent, machine).is_none())
            .filter(|agent| crate::agent_state::is_seen(agent))
            .filter(|agent| !on_screen.contains(&agent.pane_id))
            .filter(|agent| !self.records.contains_key(&agent.pane_id))
            .filter(|agent| {
                backoff_until
                    .get(&agent.pane_id)
                    .is_none_or(|until| *until <= now_unix_ms)
            })
            .filter(|agent| {
                self.stamps.get(&agent.pane_id).is_some_and(|stamps| {
                    stamps.idle_since(agent).saturating_add(after_ms) <= now_unix_ms
                })
            })
            .map(|agent| agent.pane_id.clone())
            .collect()
    }
}

/// Whether Hide can put an agent of this Herdr kind to sleep and wake it:
/// the kinds whose adapter declares a sleep.
pub(crate) fn sleeps_kind(kind: &str) -> bool {
    hide_agent_adapter::adapter(kind).is_some_and(|row| row.sleep.is_some())
}

/// Whether this kind can sleep before it has reported a conversation.
pub(crate) fn sleeps_without_session(kind: &str) -> bool {
    hide_agent_adapter::adapter(kind)
        .and_then(|adapter| adapter.sleep)
        .is_some_and(hide_agent_adapter::LaunchDialect::sleeps_without_session)
}

/// Whether this kind sleeps by closing its pane (Pi's dormant sleep).
pub(crate) fn closes_pane_when_sleeping(kind: &str) -> bool {
    hide_agent_adapter::adapter(kind)
        .and_then(|adapter| adapter.sleep)
        .is_some_and(|dialect| dialect.closes_pane_when_sleeping())
}

/// Why this agent cannot be put to sleep now, or `None` when it can (B5,
/// B15). The time and the screen are the automatic decision's alone; a
/// person asking from the pane menu has already looked.
pub fn sleep_refusal(
    agent: &SidebarAgentSnapshot,
    machine: SleepMachine,
) -> Option<AgentActionRefusal> {
    if machine == SleepMachine::Device {
        return Some(AgentActionRefusal::Device);
    }
    if !sleeps_kind(&agent.agent_kind) {
        return Some(AgentActionRefusal::Unsupported);
    }
    // A sleep that closes the pane reopens it from the core's own catalog,
    // which holds only the core machine's folders.
    if machine == SleepMachine::Node && closes_pane_when_sleeping(&agent.agent_kind) {
        return Some(AgentActionRefusal::CoreMachineOnly);
    }
    if agent.sleep.is_some() {
        return Some(AgentActionRefusal::Asleep);
    }
    if let Some(reason) = crate::agent_state::rest_refusal(agent) {
        return Some(reason);
    }
    if agent.session_id.is_none() && !sleeps_without_session(&agent.agent_kind) {
        return Some(AgentActionRefusal::NoConversation);
    }
    None
}

/// The pane menu's Sleep agent item for an agent on a machine that sleeps.
pub fn sleep_action(
    agent: &SidebarAgentSnapshot,
    machine: SleepMachine,
) -> Option<AgentSleepActionSnapshot> {
    if machine == SleepMachine::Device || agent.sleep.is_some() {
        return None;
    }
    let reason = sleep_refusal(agent, machine);
    Some(AgentSleepActionSnapshot {
        available: reason.is_none(),
        reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u64 = 60 * 60 * 1000;

    fn agent(pane_id: &str) -> SidebarAgentSnapshot {
        SidebarAgentSnapshot {
            record_connection: None,
            record_summary: None,
            record_metadata: None,
            telemetry_stale: false,
            start: Default::default(),
            close_session: None,
            state: Default::default(),
            resolved: None,
            resolved_recent: false,
            revisit: None,
            escalation: None,
            letters_waiting: None,
            raised: Vec::new(),
            descendant_mark: None,
            descendant_line: None,
            id: pane_id.to_owned(),
            herdr_name: None,
            started_name: None,
            declared_fork: None,
            forked_from: None,
            pane_id: pane_id.to_owned(),
            workspace_label: "Fixture".to_owned(),
            identity_label: "Refactor the parser".to_owned(),
            checkout_label: None,
            agent_kind: "claude".to_owned(),
            demand: "none".to_owned(),
            activity: "stopped".to_owned(),
            completed: false,
            unread: false,
            blocked: false,
            group: "seen".to_owned(),
            mark: Default::default(),
            emphasized: false,
            status_code: AgentStatusCode::Idle,
            requires_close_confirmation: false,
            requires_close_status_check: false,
            progress: Some("Split the lexer".to_owned()),
            expected_reply: None,
            detail: None,
            message: None,
            user_turn: None,
            ask_subject: None,
            status_word_visible: true,
            changed_at_unix_ms: Some(1_788_871_000_000),
            last_prompt_at_unix_ms: None,
            last_conversation_at_unix_ms: None,
            last_activity: "1788871000000".to_owned(),
            state_change_seq: Some(4),
            session_id: Some("session-a".to_owned()),
            own_find: true,
            spawned_from_pane_id: None,
            declared_parent_pane_id: None,
            spawned_from_machine_id: None,
            declared_parent_session: None,
            lineage_session: None,
            server_mark: None,
            delegated: false,
            descendant_counts: crate::model::DescendantCountsSnapshot::default(),
            direct_child_counts: crate::model::DescendantCountsSnapshot::default(),
            wait: None,
            checks_wait: None,
            background_left: 0,
            checks_ended: None,
            descendant_signals: std::collections::BTreeSet::new(),
            lineage_parent_pane_id: None,
            lineage_path_pane_ids: Vec::new(),
            lineage_sibling_pane_ids: Vec::new(),
            lineage_depth: 0,
            lineage_child_pane_ids: Vec::new(),
            close_descendant_pane_ids: Vec::new(),
            lineage_root_checkout_id: None,
            lineage_worktree_badge: None,
            lineage_orphan: false,
            lineage_hint: None,
            raised_hint: None,
            spawn_origin_pane_id: None,
            lineage_collapsed: false,
            sleep: None,
            row_facts: None,
            awaited_replies: Vec::new(),
            hide_ai: None,
            request: None,
            unseen_views: Vec::new(),
            look: None,
        }
    }

    /// A store that first saw `agents` at time zero.
    fn stamped(agents: &[SidebarAgentSnapshot]) -> AgentSleepStore {
        let mut store = AgentSleepStore::default();
        store.stamp(agents, SleepScope::Core, 0);
        store
    }

    fn due_at(store: &AgentSleepStore, agents: &[SidebarAgentSnapshot], now: u64) -> Vec<String> {
        store.due(
            agents,
            SleepMachine::Core,
            &HashSet::new(),
            Some(24),
            &HashMap::new(),
            now,
        )
    }

    fn payload(agents: serde_json::Value, panes: &[&str]) -> SessionSnapshotPayload {
        serde_json::from_value(serde_json::json!({
            "agents": agents,
            "workspaces": [{"workspace_id": "w1", "label": "Alpha"}],
            "layouts": [{"workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24}, "focused_pane_id": panes[0],
                "panes": panes.iter().map(|pane| serde_json::json!({"pane_id": pane,
                    "rect": {"x": 0, "y": 0, "width": 80, "height": 24}})).collect::<Vec<_>>(),
                "splits": []}]
        }))
        .unwrap()
    }

    fn asleep(store: &mut AgentSleepStore, agent: &SidebarAgentSnapshot) {
        let mut record = SleepRecord::ending(agent, Some("/tmp".into()), 5).unwrap();
        record.phase = SleepPhase::Sleeping;
        store.records.insert(agent.pane_id.clone(), record);
    }

    #[test]
    fn an_agent_sleeps_once_the_chosen_hours_pass_after_its_later_change_or_look() {
        let agents = [agent("w1:p1")];
        let mut store = stamped(&agents);
        assert!(due_at(&store, &agents, 24 * HOUR - 1).is_empty());
        assert_eq!(due_at(&store, &agents, 24 * HOUR), ["w1:p1"]);
        store.mark_seen(["w1:p1"], 10 * HOUR);
        assert!(due_at(&store, &agents, 24 * HOUR).is_empty());
        assert_eq!(due_at(&store, &agents, 34 * HOUR), ["w1:p1"]);
        assert!(
            store
                .due(
                    &agents,
                    SleepMachine::Core,
                    &HashSet::new(),
                    None,
                    &HashMap::new(),
                    100 * HOUR,
                )
                .is_empty(),
            "Never sleeps nothing"
        );
    }

    #[test]
    fn the_last_conversation_line_is_the_clock_once_a_session_read_proves_one() {
        let mut agents = [agent("w1:p1")];
        agents[0].last_conversation_at_unix_ms = Some(HOUR);
        let mut store = stamped(&agents);
        // A relaunch or a Herdr handoff reports a state for a moment.
        agents[0].activity = "unknown".to_owned();
        assert!(store.stamp(&agents, SleepScope::Core, 20 * HOUR));
        agents[0].activity = "stopped".to_owned();
        assert!(store.stamp(&agents, SleepScope::Core, 20 * HOUR));
        assert!(due_at(&store, &agents, 25 * HOUR - 1).is_empty());
        assert_eq!(due_at(&store, &agents, 25 * HOUR), ["w1:p1"]);

        agents[0].last_conversation_at_unix_ms = Some(30 * HOUR);
        assert!(due_at(&store, &agents, 53 * HOUR).is_empty());
        assert_eq!(due_at(&store, &agents, 54 * HOUR), ["w1:p1"]);
        store.mark_seen(["w1:p1"], 40 * HOUR);
        assert!(due_at(&store, &agents, 54 * HOUR).is_empty());
        assert_eq!(due_at(&store, &agents, 64 * HOUR), ["w1:p1"]);
    }

    #[test]
    fn without_a_conversation_read_a_state_change_restarts_the_clock() {
        let mut agents = [agent("w1:p1")];
        let mut store = stamped(&agents);
        agents[0].completed = true;
        assert!(store.stamp(&agents, SleepScope::Core, 20 * HOUR));
        assert!(due_at(&store, &agents, 24 * HOUR).is_empty());
        assert_eq!(due_at(&store, &agents, 44 * HOUR), ["w1:p1"]);
    }

    #[test]
    fn only_a_quiet_seen_local_resumable_agent_with_a_conversation_off_screen_sleeps() {
        let quiet = agent("w1:p1");
        let mut working = agent("w1:p2");
        working.activity = "working".into();
        working.group = "working".into();
        let mut asking = agent("w1:p3");
        asking.demand = "question".into();
        asking.group = "needs_you".into();
        let mut unread_done = agent("w1:p4");
        unread_done.completed = true;
        unread_done.unread = true;
        unread_done.group = "done".into();
        let mut unknown = agent("w1:p5");
        unknown.activity = "unknown".into();
        let mut no_session = agent("w1:p6");
        no_session.session_id = None;
        let mut pi = agent("w1:p7");
        pi.agent_kind = "pi".into();
        let mut omp = agent("w1:p11");
        omp.agent_kind = "omp".into();
        let mut grok = agent("w1:p12");
        grok.agent_kind = "grok".into();
        let mut other = agent("w1:p13");
        other.agent_kind = "cursor".into();
        let mut remote = agent("remote:mini:w1:p1");
        remote.pane_id = "remote:mini:w1:p1".into();
        let on_screen = agent("w1:p8");
        let backing_off = agent("w1:p9");
        let mut child = agent("w1:p10");
        child.spawned_from_pane_id = Some("w1:p1".into());
        child.delegated = true;
        let agents = [
            quiet,
            working,
            asking,
            unread_done,
            unknown,
            no_session,
            pi,
            omp,
            grok,
            other,
            remote,
            on_screen,
            backing_off,
            child,
        ];
        let store = stamped(&agents);
        let due = store.due(
            &agents,
            SleepMachine::Core,
            &HashSet::from(["w1:p8".to_owned()]),
            Some(12),
            &HashMap::from([("w1:p9".to_owned(), 13 * HOUR)]),
            12 * HOUR,
        );
        assert_eq!(due, ["w1:p1", "w1:p7", "w1:p11", "w1:p12", "w1:p10"]);
    }

    /// The sleeping row is drawn from the record, so the record has to carry
    /// what proves its parent: the session the declaration was written for.
    #[test]
    fn a_sleeping_child_stays_under_its_parent_only_while_its_record_can_prove_it() {
        let parent_session = crate::wire::session_digest("session-parent");
        let mut parent = agent("w1:p1");
        parent.lineage_session = parent_session.clone();
        let mut child = agent("w1:p2");
        child.spawned_from_pane_id = Some("w1:p1".into());
        child.declared_parent_session = parent_session;

        let lineage_of = |child: &SidebarAgentSnapshot| {
            let mut store = AgentSleepStore::default();
            asleep(&mut store, child);
            let mut session = payload(serde_json::json!([]), &["w1:p1", "w1:p2"]);
            assert!(
                store
                    .settle_payload(&mut session, SleepScope::Core)
                    .is_empty()
            );
            let mut rows = vec![parent.clone()];
            rows.extend(crate::sidebar::project_agents(session).agents);
            crate::agent_state::apply_lineage(&mut rows, &[], &[]);
            rows.remove(1).lineage_parent_pane_id
        };
        assert_eq!(lineage_of(&child).as_deref(), Some("w1:p1"));

        // A record written before the parent's session was kept proves nothing.
        child.declared_parent_session = None;
        assert_eq!(lineage_of(&child), None);
    }

    #[test]
    fn a_sleeping_agent_herdr_stopped_listing_keeps_its_row() {
        let mut parent = agent("w1:p2");
        parent.id = "reviewer".into();
        parent.spawned_from_pane_id = Some("w1:p1".into());
        let mut store = AgentSleepStore::default();
        asleep(&mut store, &parent);
        let mut session = payload(serde_json::json!([]), &["w1:p1", "w1:p2"]);
        assert!(
            store
                .settle_payload(&mut session, SleepScope::Core)
                .is_empty()
        );
        let rows = crate::sidebar::project_agents(session).agents;
        let mut rows = rows;
        store.annotate(&mut rows);
        for row in &mut rows {
            crate::agent_state::rederive(row);
        }
        let row = &rows[0];
        assert_eq!(
            (
                row.id.as_str(),
                row.herdr_name.as_deref(),
                row.pane_id.as_str(),
                row.identity_label.as_str(),
                row.session_id.as_deref(),
                row.spawned_from_pane_id.as_deref(),
                row.mark,
                row.status_code,
            ),
            (
                "reviewer",
                Some("reviewer"),
                "w1:p2",
                "Refactor the parser",
                Some("session-a"),
                Some("w1:p1"),
                crate::agent_state::RowMark::Asleep,
                AgentStatusCode::Sleeping,
            )
        );
        assert_eq!(
            row.sleep.as_ref().map(|sleep| sleep.state.as_str()),
            Some("sleeping")
        );
    }

    #[test]
    fn a_new_agent_in_the_pane_wakes_the_record_and_a_closed_pane_drops_it() {
        let mut store = AgentSleepStore::default();
        asleep(&mut store, &agent("w1:p1"));
        asleep(&mut store, &agent("w1:p2"));
        let stale = serde_json::json!([{"pane_id": "w1:p1", "agent": "claude",
            "agent_status": "idle", "state_change_seq": 4}]);
        let mut session = payload(stale, &["w1:p1", "w1:p2"]);
        assert!(
            store
                .settle_payload(&mut session, SleepScope::Core)
                .is_empty(),
            "the ended agent still reported"
        );
        assert_eq!(session.agents.len(), 2, "both drawn from their records");
        assert!(
            session
                .agents
                .iter()
                .all(|agent| agent.agent.as_deref() == Some("claude"))
        );

        // A plugin's labels keep the pane listed with no provider.
        let labelled = serde_json::json!([{"pane_id": "w1:p1", "agent_status": "unknown",
            "state_change_seq": 7, "tokens": {"task": "Refactor the parser"}}]);
        let mut session = payload(labelled, &["w1:p1", "w1:p2"]);
        assert!(
            store
                .settle_payload(&mut session, SleepScope::Core)
                .is_empty()
        );
        assert_eq!(session.agents.len(), 2);
        assert!(
            session
                .agents
                .iter()
                .all(|agent| agent.agent.as_deref() == Some("claude"))
        );

        let fresh = serde_json::json!([{"pane_id": "w1:p1", "agent": "claude",
            "agent_status": "idle", "state_change_seq": 9}]);
        let mut session = payload(fresh, &["w1:p1"]);
        assert_eq!(
            store.settle_payload(&mut session, SleepScope::Core),
            [
                Settled::Woke {
                    pane_id: "w1:p1".into(),
                    phase: SleepPhase::Sleeping
                },
                Settled::Closed {
                    pane_id: "w1:p2".into()
                },
            ]
        );
        assert!(store.records.is_empty());
        assert_eq!(session.agents.len(), 1, "the new agent alone");
    }

    #[test]
    fn a_relaunch_forgets_an_end_in_flight_and_puts_a_wake_in_flight_back_to_sleep() {
        let mut store = AgentSleepStore::default();
        store.records.insert(
            "w1:p1".into(),
            SleepRecord::ending(&agent("w1:p1"), None, 1).unwrap(),
        );
        asleep(&mut store, &agent("w1:p2"));
        store.records.get_mut("w1:p2").unwrap().phase = SleepPhase::Waking;
        store.after_load();
        assert_eq!(
            store
                .records
                .iter()
                .map(|(pane, record)| (pane.as_str(), record.phase))
                .collect::<Vec<_>>(),
            [("w1:p2", SleepPhase::Sleeping)]
        );
    }

    #[test]
    fn a_nodes_sleeping_pane_is_settled_by_its_own_session_and_no_other() {
        const NODE: &str = "remote:screen:pane:";
        let mut store = AgentSleepStore::default();
        let mut sleeper = agent("remote:screen:pane:w1:p1");
        sleeper.spawned_from_pane_id = Some("remote:screen:pane:w1:p9".into());
        asleep(&mut store, &sleeper);

        let mut core = payload(serde_json::json!([]), &["w1:p1"]);
        assert!(store.settle_payload(&mut core, SleepScope::Core).is_empty());
        assert!(
            core.agents.is_empty(),
            "the core's own w1:p1 is another pane"
        );

        let mut node = payload(serde_json::json!([]), &["w1:p1"]);
        assert!(
            store
                .settle_payload(&mut node, SleepScope::Device(NODE))
                .is_empty()
        );
        assert_eq!(
            node.agents
                .iter()
                .map(|row| (row.pane_id.as_deref(), row.spawned_from_pane_id.as_deref()))
                .collect::<Vec<_>>(),
            [(Some("w1:p1"), Some("w1:p9"))],
            "the row names the pane as the node's Herdr does"
        );

        let mut gone = payload(serde_json::json!([]), &["w1:p2"]);
        assert_eq!(
            store.settle_payload(&mut gone, SleepScope::Device(NODE)),
            [Settled::Closed {
                pane_id: "remote:screen:pane:w1:p1".into()
            }]
        );
    }

    #[test]
    fn the_pane_menu_says_why_an_agent_cannot_sleep() {
        let mut working = agent("w1:p1");
        working.activity = "working".into();
        assert_eq!(
            sleep_action(&working, SleepMachine::Core),
            Some(AgentSleepActionSnapshot {
                available: false,
                reason: Some(AgentActionRefusal::Working),
            })
        );
        assert_eq!(
            sleep_action(&agent("w1:p2"), SleepMachine::Core),
            Some(AgentSleepActionSnapshot {
                available: true,
                reason: None
            })
        );
        let mut remote = agent("remote:mini:pane:w1:p1");
        remote.pane_id = "remote:mini:pane:w1:p1".into();
        assert_eq!(sleep_action(&remote, SleepMachine::Device), None);
        assert_eq!(
            sleep_action(&remote, SleepMachine::Node),
            Some(AgentSleepActionSnapshot {
                available: true,
                reason: None
            })
        );
        let mut pi = remote.clone();
        pi.agent_kind = "pi".into();
        assert_eq!(
            sleep_action(&pi, SleepMachine::Node),
            Some(AgentSleepActionSnapshot {
                available: false,
                reason: Some(AgentActionRefusal::CoreMachineOnly),
            })
        );
    }

    #[test]
    fn agy_sleeps_before_its_first_prompt_and_wakes_as_a_new_conversation() {
        let mut fresh = agent("w1:p1");
        fresh.session_id = None;
        assert_eq!(
            sleep_refusal(&fresh, SleepMachine::Core),
            Some(AgentActionRefusal::NoConversation)
        );
        assert_eq!(SleepRecord::ending(&fresh, None, 1), None);
        fresh.agent_kind = "antigravity".into();
        assert_eq!(sleep_refusal(&fresh, SleepMachine::Core), None);
        let record = SleepRecord::ending(&fresh, None, 1).unwrap();
        assert_eq!(record.session_id, None);
        assert_eq!(record.label_owner, None);
    }
}
