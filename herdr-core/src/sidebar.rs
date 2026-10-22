#[cfg(test)]
use crate::agent_state::sync_checkout_agent_summaries;
#[cfg(test)]
use crate::model::PaneReadRecord;
#[cfg(test)]
use std::collections::HashSet;
use std::collections::{BTreeMap, BTreeSet};

use crate::agent_state::{axes::*, tally::child_representative_rank, turn::*};

use serde::Deserialize;
use serde_json::Value;

use crate::model::{AgentStatusCode, PaneLayoutDirection, SidebarAgentSnapshot};

/// Event provenance belongs to the local replica, not the external wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTabFocus {
    pub generation: u64,
    pub workspace_id: String,
    pub tab_id: String,
    pub revision: u64,
    pub creation: bool,
}

/// How many tab moves are kept on each side: the replica's latest Herdr
/// moves and the core's sent, unanswered tab requests. One value, so any
/// burst the request list can hold is one the moves can answer.
pub(crate) const TAB_FOCUS_LIMIT: usize = 16;

/// The tabs Herdr focused, one `tab_focused` event each, in the order it
/// applied them. A session folds several events into one state, and a state
/// that ends on t2 cannot say whether Herdr went t2 or t2, t3, t2; the moves
/// can. Only the latest `TAB_FOCUS_LIMIT` are kept; `applied` counts all of
/// this replica's, so a reader that consumed `n` knows which are new and
/// whether any it has not seen were dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionTabMoves {
    pub generation: u64,
    pub applied: u64,
    pub recent: std::collections::VecDeque<String>,
}

impl SessionTabMoves {
    pub(crate) fn new(generation: u64) -> Self {
        Self {
            generation,
            ..Self::default()
        }
    }

    pub(crate) fn record(&mut self, tab_id: String) {
        self.applied += 1;
        self.recent.push_back(tab_id);
        if self.recent.len() > TAB_FOCUS_LIMIT {
            self.recent.pop_front();
        }
    }

    /// The moves after the first `consumed` of `generation`, oldest first;
    /// `None` when that generation is gone or some of them were dropped.
    pub(crate) fn since(
        &self,
        generation: u64,
        consumed: u64,
    ) -> Option<impl Iterator<Item = &String>> {
        let unseen = usize::try_from(self.applied.checked_sub(consumed)?).ok()?;
        (generation == self.generation && unseen <= self.recent.len())
            .then(|| self.recent.iter().skip(self.recent.len() - unseen))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct SessionSnapshotPayload {
    #[serde(skip)]
    pub tab_focus: Option<SessionTabFocus>,
    /// Herdr's tab moves, present on a session the event replica published.
    /// A snapshot read on its own has no place in Herdr's event order.
    #[serde(skip)]
    pub tab_moves: Option<SessionTabMoves>,
    #[serde(default)]
    pub focused_pane_id: Option<String>,
    /// The Herdr workspace that holds Herdr's keyboard. Its active tab is the
    /// only tab Herdr can be said to have focused; every other workspace's
    /// `active_tab_id` is that workspace's memory of where it was last.
    #[serde(default)]
    pub focused_workspace_id: Option<String>,
    #[serde(default)]
    pub tabs: Vec<SessionTabPayload>,
    #[serde(default)]
    pub layouts: Vec<SessionLayoutPayload>,
    pub agents: Vec<SessionAgentPayload>,
    #[serde(default)]
    pub panes: Vec<SessionPanePayload>,
    #[serde(default)]
    pub workspaces: Vec<SessionWorkspacePayload>,
    /// The version of the Herdr that answered; the snapshot's own `version`.
    #[serde(default)]
    pub herdr_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SessionWorkspacePayload {
    #[serde(default)]
    pub worktree: Option<crate::domain::WorktreeProjection>,
    pub workspace_id: String,
    #[serde(default)]
    pub label: String,
    /// The tab Herdr reports as active in this workspace. It is the only
    /// authority for which tab is active; the navigator never falls back to a
    /// position.
    #[serde(default)]
    pub active_tab_id: Option<String>,
    /// Workspace metadata is the live purpose authority. Herdr caps token
    /// values at 80 characters before they reach this boundary.
    #[serde(default)]
    pub tokens: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SessionTabPayload {
    #[serde(default)]
    pub number: u32,
    pub tab_id: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SessionLayoutPayload {
    pub workspace_id: String,
    pub tab_id: String,
    pub zoomed: bool,
    pub area: SessionLayoutRect,
    pub focused_pane_id: String,
    pub panes: Vec<SessionLayoutPanePayload>,
    pub splits: Vec<SessionLayoutSplitPayload>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub struct SessionLayoutRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SessionLayoutPanePayload {
    pub pane_id: String,
    pub rect: SessionLayoutRect,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SessionLayoutSplitPayload {
    pub direction: PaneLayoutDirection,
    pub ratio: f32,
    pub rect: SessionLayoutRect,
}

/// The most a label sentence may run to on a row; anything past this is the
/// tooltip's.
const MAX_LABEL_TEXT_CHARS: usize = 80;

/// The longest `expected_reply` a label carries (PRD D-05). The projection
/// cuts at the same length so a reply that outran the rule cannot push the
/// row past one line.
pub const MAX_EXPECTED_REPLY_CHARS: usize = 40;

/// What the core's label worker says about an agent (PRD labels-in-hided
/// D-04): set by `labels::worker::LabelWorker::apply` only while the pane's
/// current session reference proves the session the label was made for.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct AgentLabel {
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub progress: Option<String>,
    #[serde(default)]
    pub expected_reply: Option<String>,
    /// The agent's last message asks the operator something specific.
    #[serde(default)]
    pub question: bool,
    /// The agent stopped and named what stopped it (`end: blocked`). This is
    /// the label's block, not Herdr's `blocked` lifecycle, which is an
    /// approval prompt.
    #[serde(default)]
    pub blocked: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SessionAgentPayload {
    /// The query failed while its topology subscription is still live.
    /// Internal provenance, never supplied by Herdr or a shell client.
    #[serde(skip)]
    pub(crate) telemetry_stale: bool,
    #[serde(default)]
    pub id: Option<String>,
    /// The name Herdr knows the agent by (`agent.start --name`, `agent
    /// rename`); absent when it was given none.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub pane_id: Option<String>,
    #[serde(default)]
    pub workspace_label: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: Option<String>,
    /// Herdr's record of the conversation this agent is running, when it has
    /// one. `kind` says whether `value` is an id or a path; only an id can be
    /// handed to an agent's own fork command.
    #[serde(default)]
    pub agent_session: Option<SessionAgentSessionPayload>,
    /// The pane this agent was spawned from, as hcoord's `parent_pane` token
    /// declares it (Herdr records no lineage). `wire.rs` leaves it empty when
    /// the child's own session no longer matches the one the token was
    /// written for.
    #[serde(default)]
    pub spawned_from_pane_id: Option<String>,
    /// Stable identity of a parent on another machine, recorded by hcoord.
    #[serde(default)]
    pub spawned_from_machine_id: Option<String>,
    /// The digest of the parent's session the relationship was written for
    /// (`wire::session_digest`). `apply_lineage` keeps the parent only while
    /// its pane reports that session; a declared parent without one has
    /// nothing to be compared with and is a root.
    #[serde(default)]
    pub declared_parent_session: Option<String>,
    /// The digest of this pane's own session (`wire::session_digest`), which
    /// the rows that declare it as their parent are compared with.
    #[serde(default)]
    pub lineage_session: Option<String>,
    #[serde(default)]
    pub state_change_seq: Option<u64>,
    /// Herdr's record that the agent's current stop finished work, which its
    /// tab-scoped seen does not clear (`agent_state::agent_completed`).
    #[serde(default)]
    pub completion_seq: Option<u64>,
    #[serde(default)]
    pub tokens: BTreeMap<String, Value>,
    /// The core's label for this agent's current session, if proven.
    #[serde(default)]
    pub label: Option<AgentLabel>,
    /// When the core last saw this agent change state, in epoch ms.
    #[serde(default)]
    pub changed_at_unix_ms: Option<u64>,
    /// What the label worker read of the current session, if proven; laid
    /// on by the label overlay, never read from Herdr.
    #[serde(skip)]
    pub(crate) facts: Option<crate::request_view::RowFacts>,
    /// Display history, never used by agent_demand/activity or close proof.
    #[serde(skip)]
    pub(crate) record: Option<crate::labels::record::RecordBinding>,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, Eq, PartialEq)]
pub struct SessionAgentSessionPayload {
    pub kind: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SessionPanePayload {
    #[serde(default)]
    pub start_claim: Option<String>,
    #[serde(default)]
    pub terminal_id: Option<String>,
    /// Authoritative failed terminal incarnation, independent of agent activity.
    #[serde(default)]
    pub failed_terminal_id: Option<String>,
    #[serde(default)]
    pub foreground_process: Option<String>,
    pub pane_id: String,
    #[serde(default)]
    pub tokens: BTreeMap<String, Value>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// The name the user gave this pane in Herdr, when they gave it one.
    #[serde(default)]
    pub label: Option<String>,
    /// What the program running in the pane set the terminal title to.
    #[serde(default)]
    pub terminal_title: Option<String>,
}
/// Resolves the non-persistent half of the checkout purpose ladder.
///
/// Token and branch-description values were selected while the catalog was
/// built and always win. Agent and pull-request titles are rebuilt whenever
/// either projection changes, so a vanished representative cannot leave a
/// stale sentence on the row.
pub fn sync_checkout_purposes(
    workspaces: &mut [crate::model::WorkspaceSnapshot],
    agents: &[crate::model::SidebarAgentSnapshot],
) -> bool {
    use crate::model::{CheckoutPurposeOrigin, CheckoutPurposeSnapshot};

    let agent_titles = agents
        .iter()
        .map(|agent| (agent.pane_id.as_str(), agent.identity_label.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut changed = false;
    for checkout in workspaces
        .iter_mut()
        .flat_map(|workspace| workspace.checkouts.iter_mut())
    {
        if checkout.purpose.as_ref().is_some_and(|purpose| {
            matches!(
                purpose.origin,
                CheckoutPurposeOrigin::Token | CheckoutPurposeOrigin::BranchDescription
            )
        }) {
            continue;
        }
        let next = checkout
            .agent_summary
            .representative_pane_id
            .as_deref()
            .and_then(|pane_id| agent_titles.get(pane_id).copied())
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(|title| CheckoutPurposeSnapshot {
                text: title.to_owned(),
                origin: CheckoutPurposeOrigin::AgentTitle,
            })
            .or_else(|| {
                checkout
                    .pull_request
                    .as_ref()
                    .map(|pull_request| pull_request.title.trim())
                    .filter(|title| !title.is_empty())
                    .map(|title| CheckoutPurposeSnapshot {
                        text: title.to_owned(),
                        origin: CheckoutPurposeOrigin::PullRequestTitle,
                    })
            });
        if checkout.purpose != next {
            checkout.purpose = next;
            changed = true;
        }
    }
    changed
}

/// One agent that could not be read out of an otherwise valid snapshot.
///
/// A single broken record excludes only itself; the surrounding agents are
/// still projected, and the exclusion is reported rather than swallowed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentExclusion {
    pub source_index: usize,
    pub pane_id: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentProjection {
    pub agents: Vec<SidebarAgentSnapshot>,
    pub excluded: Vec<AgentExclusion>,
}

/// Projects the agent list with every row unread.
///
/// The read axis is a Hide judgment that needs the pane read record, which
/// lives in the runtime, so callers that hold one follow this with
/// [`apply_read_state`]. A caller with no record still gets a coherent
/// projection: unread is the honest answer when nothing says otherwise.
pub fn project_agents(payload: SessionSnapshotPayload) -> AgentProjection {
    let mut agents = Vec::with_capacity(payload.agents.len());
    let mut excluded = Vec::new();
    for (source_index, agent) in payload.agents.into_iter().enumerate() {
        let pane_id =
            non_empty(agent.pane_id.as_deref().or(agent.id.as_deref())).map(str::to_owned);
        match project_agent(agent) {
            Ok(projected) => agents.push(projected),
            Err(reason) => excluded.push(AgentExclusion {
                source_index,
                pane_id,
                reason,
            }),
        }
    }

    // Rows stay in the order Herdr sent them. Ordering is a function of the
    // group, and the group is a function of the read axis, which is decided
    // in `apply_read_state` where the pane read record lives.
    for agent in &mut agents {
        derive_from_axes(agent);
    }
    AgentProjection { agents, excluded }
}

/// One agent, as every surface that names an agent in a line draws it: the
/// pane header's chip row, a breadcrumb step's sibling list, and the
/// Overview worktree row's agent line all read the same fields, so they
/// cannot describe the same agent differently (PRD B5, B10, B34).
pub fn agent_chip(agent: &SidebarAgentSnapshot) -> crate::model::AgentChipSnapshot {
    crate::model::AgentChipSnapshot {
        tone: agent.state.chip_tone,
        pane_id: agent.pane_id.clone(),
        label: agent.identity_label.clone(),
        checkout_label: agent.checkout_label.clone(),
        detail: agent.detail.clone(),
        status_word_visible: agent.status_word_visible,
        agent_kind: agent.agent_kind.clone(),
        demand: agent.demand.clone(),
        activity: agent.activity.clone(),
        emphasized: agent.emphasized,
        mark: agent.mark,
        status_code: agent.status_code,
        delegated: agent.delegated,
    }
}

/// What one pane's header says about the work its agent delegated.
///
/// Returns `None` for a pane with no agent: a shell, an editor or a log has
/// no children to report and gets no mark saying so (PRD B22, D-30).
pub fn project_pane_children(
    agents: &[SidebarAgentSnapshot],
    pane_id: &str,
    tokens: crate::agent_hooks::PaneHookTokens,
    status_of: &dyn Fn(hide_agent_adapter::HookDialect) -> Option<hide_agent_hooks::HookStatus>,
) -> Option<crate::model::PaneChildrenSnapshot> {
    let agent = agents.iter().find(|agent| agent.pane_id == pane_id)?;
    let dialect =
        hide_agent_adapter::adapter(&agent.agent_kind).and_then(|row| row.subagent_counts);
    let status = dialect.and_then(status_of);
    let instrumentation = hide_agent_hooks::diagnosis::instrumentation(
        hide_agent_hooks::diagnosis::PaneObservation {
            dialect,
            token_version: tokens.version,
            working: tokens.working,
            done: tokens.done,
            blocked: tokens.blocked,
        },
        status.as_ref(),
    );
    // Pane children come from the lineage, which Herdr reports directly, so
    // they are known whether or not the hook is installed. Only the
    // in-process count depends on instrumentation (PRD D-24).
    let chips = agent
        .lineage_child_pane_ids
        .iter()
        .filter_map(|child| agents.iter().find(|agent| &agent.pane_id == child))
        .map(agent_chip)
        .collect::<Vec<_>>();
    let representative = agent
        .lineage_child_pane_ids
        .iter()
        .filter_map(|child| agents.iter().find(|agent| &agent.pane_id == child))
        .min_by_key(|child| child_representative_rank(child))
        .map(agent_chip);
    Some(crate::model::PaneChildrenSnapshot {
        instrumented: instrumentation.instrumented,
        uninstrumented_reason: instrumentation
            .reason
            .map(|reason| reason.message().to_owned()),
        uninstrumented_label: instrumentation
            .reason
            .map(|reason| reason.accessibility_label().to_owned()),
        uninstrumented_code: instrumentation
            .reason
            .map(|reason| reason.code().to_owned()),
        chips,
        connection: None,
        representative,
        subagents: crate::model::SubagentCountsSnapshot {
            working: instrumentation.working,
            done: instrumentation.done,
            blocked: instrumentation.blocked,
        },
    })
}

/// [`project_pane_children`] with the pane's connection judged on top of it.
///
/// The connection is read from the observation the children were just
/// projected from (PRD settings-cleanup B26, D-09). A sleeping agent has
/// ended its process, so there is no session for a hook to speak from and
/// nothing to judge until it wakes. `codex_daemon_on` is the machine's Codex
/// shared-server setting.
pub fn project_pane_children_connected(
    agents: &[SidebarAgentSnapshot],
    pane_id: &str,
    tokens: crate::agent_hooks::PaneHookTokens,
    status_of: &dyn Fn(hide_agent_adapter::HookDialect) -> Option<hide_agent_hooks::HookStatus>,
    codex_daemon_on: bool,
) -> Option<crate::model::PaneChildrenSnapshot> {
    let mut children = project_pane_children(agents, pane_id, tokens, status_of)?;
    let runtime = agents
        .iter()
        // Process detection can precede the native session. No session yet
        // is not proof of a failed hook.
        .find(|agent| {
            agent.pane_id == pane_id && agent.sleep.is_none() && agent.close_session.is_some()
        })
        .and_then(|agent| crate::agent_hooks::runtime_of(&agent.agent_kind));
    children.connection = pane_connection(runtime, pane_id, &children, codex_daemon_on);
    Some(children)
}

/// Whether Hide hears one pane's session, and why not.
///
/// Only Claude Code's and Codex's sessions can be reopened to connect them, so
/// no other agent has a connection to judge (B19); an OpenCode session started
/// before Hide's plugin keeps its children mark and reason instead. An agent switched off has no status, and
/// a machine whose hooks Hide has not read leaves the cause unknown rather
/// than naming one (B16).
fn pane_connection(
    runtime: Option<hide_agent_hooks::AgentRuntime>,
    pane_id: &str,
    children: &crate::model::PaneChildrenSnapshot,
    codex_daemon_on: bool,
) -> Option<crate::model::PaneConnectionSnapshot> {
    use crate::model::{PaneConnectionReason, PaneConnectionSnapshot};
    use hide_agent_hooks::AgentRuntime;
    use hide_agent_hooks::diagnosis::UninstrumentedReason as Reason;
    let runtime = runtime?;
    let not_connected = |reason| {
        Some(PaneConnectionSnapshot {
            connected: false,
            // Reopen restarts the session on this Mac's Herdr, so a pane on
            // another device and a missing hook are not its to fix.
            can_reopen: reason != PaneConnectionReason::SetupNeeded
                && !crate::agent_hooks::is_remote_pane(pane_id),
            reason: Some(reason),
            reopen: None,
        })
    };
    if children.instrumented {
        return Some(PaneConnectionSnapshot {
            connected: true,
            can_reopen: false,
            reason: None,
            reopen: None,
        });
    }
    match children
        .uninstrumented_code
        .as_deref()
        .and_then(Reason::from_code)?
    {
        Reason::HooksSwitchedOff | Reason::Unknown => None,
        Reason::SessionPredatesInstall => {
            not_connected(if runtime == AgentRuntime::Codex && codex_daemon_on {
                PaneConnectionReason::CodexSharedServer
            } else {
                PaneConnectionReason::StartedBeforeHide
            })
        }
        Reason::HooksNotInstalled | Reason::ConfigUnreadable | Reason::HookOutdated => {
            not_connected(PaneConnectionReason::SetupNeeded)
        }
    }
}

/// The breadcrumb for one pane: its ancestors root first, each carrying that
/// layer's siblings for the step's dropdown.
///
/// Derived from the current list every time, so a departed ancestor shortens
/// it on the next projection with nothing to repair (PRD B9, D-18).
pub fn project_lineage_path(
    agents: &[SidebarAgentSnapshot],
    pane_id: &str,
) -> Vec<crate::model::LineageStepSnapshot> {
    let Some(agent) = agents.iter().find(|agent| agent.pane_id == pane_id) else {
        return Vec::new();
    };
    if agent.lineage_path_pane_ids.is_empty() {
        // A root has nowhere to go back to, so its header stays plain.
        return Vec::new();
    }
    agent
        .lineage_path_pane_ids
        .iter()
        .chain(std::iter::once(&agent.pane_id))
        .filter_map(|step| agents.iter().find(|agent| &agent.pane_id == step))
        .map(|step| crate::model::LineageStepSnapshot {
            pane_id: step.pane_id.clone(),
            label: agent_chip(step).label,
            siblings: step
                .lineage_sibling_pane_ids
                .iter()
                .filter_map(|sibling| agents.iter().find(|agent| &agent.pane_id == sibling))
                .map(agent_chip)
                .collect(),
        })
        .collect()
}

/// The name of an agent kind as a person says it: the two providers Hide
/// starts by their product names, any other kind exactly as Herdr reports it.
pub(crate) fn provider_name(kind: Option<&str>) -> String {
    match non_empty(kind) {
        Some(kind) => hide_agent_adapter::adapter(kind)
            .map(|row| row.sidebar_label.unwrap_or(row.herdr.name))
            .unwrap_or(kind)
            .to_owned(),
        None => "Agent".to_owned(),
    }
}

fn project_agent(agent: SessionAgentPayload) -> Result<SidebarAgentSnapshot, String> {
    let pane_id = non_empty(agent.pane_id.as_deref().or(agent.id.as_deref()))
        .map(str::to_owned)
        .ok_or_else(|| "session agent is missing a pane id".to_owned())?;
    let last_activity = projected_last_activity(&agent, &pane_id)?;
    let demand = agent_demand(&agent);
    let activity = agent_activity(&agent);
    let completed = agent_completed(&agent);
    let blocked = agent_blocked(&agent);
    let workspace_label = non_empty(agent.workspace_label.as_deref())
        .or_else(|| {
            agent
                .cwd
                .as_deref()
                .and_then(|cwd| cwd.rsplit('/').find(|segment| !segment.trim().is_empty()))
        })
        .unwrap_or("workspace")
        .to_owned();
    // The label's title and sentences, each one line. The worker already
    // bounds them; the cut here is the same bound applied once more so a
    // value that outran it cannot reach a row.
    let label = agent.label.as_ref();
    let task = label.and_then(|label| line_text(label.task.as_deref(), MAX_LABEL_TEXT_CHARS));
    let progress =
        label.and_then(|label| line_text(label.progress.as_deref(), MAX_LABEL_TEXT_CHARS));
    let expected_reply = label
        .and_then(|label| line_text(label.expected_reply.as_deref(), MAX_EXPECTED_REPLY_CHARS));
    let message = label.and_then(agent_message);

    let agent_kind = non_empty(agent.agent.as_deref()).unwrap_or("unknown");
    // Every surface uses one title order, from proven facts
    // alone. Herdr's workspace and control names never enter this ladder.
    let native_title = agent
        .facts
        .as_ref()
        .and_then(|facts| line_text(facts.native_title.as_deref(), MAX_LABEL_TEXT_CHARS));
    let manual_title = agent
        .facts
        .as_ref()
        .and_then(|facts| line_text(facts.native_custom_title.as_deref(), MAX_LABEL_TEXT_CHARS));
    let title = manual_title.or(task).or(native_title).or_else(|| {
        agent.record.as_ref().and_then(|r| {
            line_text(r.metadata.manual_title.as_deref(), MAX_LABEL_TEXT_CHARS)
                .or_else(|| line_text(r.metadata.goal.as_deref(), MAX_LABEL_TEXT_CHARS))
                .or_else(|| line_text(r.metadata.native_title.as_deref(), MAX_LABEL_TEXT_CHARS))
        })
    });
    let identity_label = title.unwrap_or_else(|| provider_name(agent.agent.as_deref()));
    let projected = SidebarAgentSnapshot {
        telemetry_stale: agent.telemetry_stale,
        record_connection: agent.record.as_ref().map(|r| r.connection),
        record_summary: agent
            .record
            .as_ref()
            .and_then(|r| line_text(Some(&r.metadata.summary), MAX_LABEL_TEXT_CHARS)),
        record_metadata: agent.record.map(|r| r.metadata),
        close_session: agent.agent_session.clone(),
        state: Default::default(),
        resolved: None,
        resolved_recent: false,
        revisit: None,
        escalation: None,
        letters_waiting: None,
        raised: Vec::new(),
        descendant_mark: None,
        descendant_line: None,
        id: agent.id.unwrap_or_else(|| pane_id.clone()),
        herdr_name: non_empty(agent.name.as_deref())
            .filter(|name| !crate::fork::hide_made_name(name, agent_kind, &pane_id))
            .map(str::to_owned),
        started_name: non_empty(agent.name.as_deref()).map(str::to_owned),
        declared_fork: crate::wire::fork_declaration(
            &agent.tokens,
            non_empty(agent.lineage_session.as_deref()),
        ),
        // Laid on with the lineage, where the source's row is known.
        forked_from: None,
        pane_id,
        workspace_label,
        checkout_label: None,
        agent_kind: agent_kind.to_owned(),
        demand: demand.name().to_owned(),
        activity: activity.name().to_owned(),
        completed,
        // Every agent is projected unread; the read axis and everything
        // derived from it are set once, by `apply_read_state`, where the
        // pane read record lives.
        unread: true,
        blocked,
        group: String::new(),
        mark: Default::default(),
        emphasized: false,
        status_code: AgentStatusCode::Unknown,
        requires_close_confirmation: false,
        requires_close_status_check: false,
        identity_label,
        progress,
        expected_reply,
        detail: None,
        message,
        user_turn: agent
            .facts
            .as_ref()
            .and_then(|facts| facts.user_turn.clone()),
        ask_subject: None,
        status_word_visible: true,
        changed_at_unix_ms: agent.changed_at_unix_ms,
        last_prompt_at_unix_ms: agent.facts.as_ref().and_then(|facts| {
            facts
                .operator_request
                .iter()
                .chain(facts.other_request.iter())
                .map(|request| request.at_unix_ms)
                .filter(|at| *at > 0)
                .max()
        }),
        last_conversation_at_unix_ms: agent.facts.as_ref().and_then(|facts| {
            facts
                .operator_request
                .iter()
                .chain(facts.other_request.iter())
                .map(|request| request.at_unix_ms)
                .chain(facts.last_reply_at_unix_ms)
                .filter(|at| *at > 0)
                .max()
        }),
        last_activity,
        state_change_seq: agent.state_change_seq,
        session_id: agent
            .facts
            .as_ref()
            .and_then(|facts| facts.native_session_id.clone())
            .or_else(|| {
                if hide_session::Agent::from_kind(agent_kind)
                    .is_some_and(hide_session::Agent::requires_native_proof)
                {
                    return None;
                }
                agent
                    .agent_session
                    .as_ref()
                    .filter(|session| session.kind == "id")
                    .map(|session| session.value.clone())
                    .filter(|value| !value.trim().is_empty())
            }),
        own_find: crate::agent_find::agent_find(agent_kind).is_some(),
        spawned_from_pane_id: non_empty(agent.spawned_from_pane_id.as_deref()).map(str::to_owned),
        declared_parent_pane_id: non_empty(agent.spawned_from_pane_id.as_deref())
            .map(str::to_owned),
        spawned_from_machine_id: non_empty(agent.spawned_from_machine_id.as_deref())
            .map(str::to_owned),
        declared_parent_session: non_empty(agent.declared_parent_session.as_deref())
            .map(str::to_owned),
        lineage_session: non_empty(agent.lineage_session.as_deref()).map(str::to_owned),
        server_mark: crate::wire::server_mark(&agent.tokens),
        delegated: false,
        descendant_counts: crate::model::DescendantCountsSnapshot::default(),
        direct_child_counts: crate::model::DescendantCountsSnapshot::default(),
        wait: None,
        checks_wait: None,
        checks_ended: None,
        background_left: 0,
        descendant_signals: BTreeSet::new(),
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
        row_facts: agent.facts,
        awaited_replies: Vec::new(),
        hide_ai: None,
        request: None,
    };
    Ok(projected)
}

/// The ordering key: when the core saw the agent change state, as thirteen
/// digits, or Herdr's own state change sequence padded to twenty when the
/// core has not observed it.
fn projected_last_activity(agent: &SessionAgentPayload, pane_id: &str) -> Result<String, String> {
    match (agent.changed_at_unix_ms, agent.state_change_seq) {
        (Some(changed), _) => Ok(format!("{changed:013}")),
        (None, Some(sequence)) => Ok(format!("{sequence:020}")),
        (None, None) => Err(format!(
            "agent {pane_id} has neither a state change time nor state_change_seq"
        )),
    }
}

/// One line of label text: whitespace collapsed, empty dropped, cut at
/// `max_chars`.
fn line_text(value: Option<&str>, max_chars: usize) -> Option<String> {
    value.and_then(|value| crate::display_text::one_line(value, max_chars))
}

/// What the agent's label last said (PRD overview-lenses-tiles-agents
/// D-50): the expected reply and the progress whole, request first, each
/// trimmed but not collapsed or cut to a row, joined by a line break, and
/// each held to `MAX_LABEL_TEXT_CHARS` so a sentence cannot grow the
/// snapshot.
fn agent_message(label: &AgentLabel) -> Option<String> {
    let parts: Vec<String> = [label.expected_reply.as_deref(), label.progress.as_deref()]
        .into_iter()
        .flatten()
        .map(|value| {
            value
                .trim()
                .chars()
                .take(MAX_LABEL_TEXT_CHARS)
                .collect::<String>()
        })
        .filter(|value| !value.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// The session every fixture row runs and every fixture declaration was
/// written for (`wire::session_digest`).
#[cfg(test)]
pub(crate) fn fixture_lineage_session() -> String {
    crate::wire::session_digest("fixture-lineage-session").unwrap()
}

/// A declared parent is proven by sessions (`wire::session_holds`), and the
/// fixtures that name parents do not care which session runs where, so each
/// row runs one shared session and each declaration was written for it:
/// whichever row a test makes another's parent still holds. A test about the
/// proof itself sets the sessions it means.
#[cfg(test)]
pub(crate) fn lineage_fixture_value(mut value: Value) -> Value {
    if let Some(agents) = value.get_mut("agents").and_then(Value::as_array_mut) {
        for agent in agents {
            if agent.get("lineage_session").is_none() {
                agent["lineage_session"] = Value::String(fixture_lineage_session());
            }
            let declares_parent = agent
                .get("spawned_from_pane_id")
                .is_some_and(|parent| !parent.is_null());
            if declares_parent && agent.get("declared_parent_session").is_none() {
                agent["declared_parent_session"] = Value::String(fixture_lineage_session());
            }
        }
    }
    value
}

/// Status and lineage fixtures were written as the retired plugin's tokens.
/// This turns them into what the core's label worker lays on the payload:
/// `task`, `progress`, `expected_reply` and a `status_question*` token become
/// the agent's `label`, and an `activity` token its state change time. Other
/// tokens stay, as the payload carries them.
#[cfg(test)]
pub(crate) fn owned_label_fixture<T: serde::de::DeserializeOwned>(
    value: Value,
) -> Result<T, serde_json::Error> {
    let mut value = lineage_fixture_value(value);
    if let Some(agents) = value.get_mut("agents").and_then(Value::as_array_mut) {
        for agent in agents {
            let Some(tokens) = agent.get_mut("tokens").and_then(Value::as_object_mut) else {
                continue;
            };
            let text = |tokens: &mut serde_json::Map<String, Value>, key: &str| {
                tokens
                    .remove(key)
                    .and_then(|value| value.as_str().map(str::to_owned))
            };
            let task = text(tokens, "task");
            let progress = text(tokens, "progress");
            let expected_reply = text(tokens, "expected_reply");
            let question = ["status_question", "status_question_new"]
                .iter()
                .any(|key| tokens.remove(*key).is_some_and(|value| !value.is_null()));
            let activity = text(tokens, "activity").and_then(|value| value.parse::<u64>().ok());
            // The plugin mirrored Herdr's lifecycle into a status token; a
            // fixture that gave only the token meant that lifecycle.
            let present = |name: &str| {
                [name.to_owned(), format!("{name}_new")]
                    .iter()
                    .any(|key| tokens.get(key).is_some_and(|value| !value.is_null()))
            };
            let status = if present("status_working") {
                Some("working")
            } else if present("status_approval") {
                Some("blocked")
            } else if present("status_done") {
                Some("done")
            } else if present("status_idle") || question {
                Some("idle")
            } else {
                None
            };
            tokens.retain(|key, _| !key.starts_with("status_"));
            if let Some(status) = status
                && agent.get("agent_status").is_none_or(Value::is_null)
            {
                agent["agent_status"] = Value::from(status);
            }
            if task.is_some() || progress.is_some() || expected_reply.is_some() || question {
                agent["label"] = serde_json::json!({
                    "task": task,
                    "progress": progress,
                    "expected_reply": expected_reply,
                    "question": question,
                });
            }
            if let Some(activity) = activity {
                agent["changed_at_unix_ms"] = Value::from(activity);
            }
        }
    }
    serde_json::from_value(value)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::agent_state::tally::RowMark;

    #[test]
    fn an_agent_carries_its_state_change_time_only_when_the_worker_measured_one() {
        let agents = project_agents(payload(json!([
            {"pane_id": "measured", "workspace_label": "W", "agent": "codex", "agent_status": "running", "state_change_seq": 1, "tokens": {"activity": "1700000000000"}},
            {"pane_id": "unmeasured", "workspace_label": "W", "agent": "codex", "agent_status": "running", "state_change_seq": 1, "tokens": {}}
        ])))
        .agents;
        let changed_at = |pane: &str| {
            agents
                .iter()
                .find(|agent| agent.pane_id == pane)
                .map(|agent| agent.changed_at_unix_ms)
        };
        assert_eq!(changed_at("measured"), Some(Some(1_700_000_000_000)));
        assert_eq!(changed_at("unmeasured"), Some(None));
    }

    #[test]
    fn prompt_elapsed_uses_the_latest_dated_request_without_moving_state_time() {
        use crate::labels::facts::{Request, Requester};
        let request = |at_unix_ms, requester| Request {
            text: "a native request".to_owned(),
            offset: None,
            cut: false,
            images: 0,
            at_unix_ms,
            requester,
            first: false,
        };
        for (operator, other, expected) in [
            (None, None, None),
            (Some(0), None, None),
            (None, Some(0), None),
            (Some(100), None, Some(100)),
            (None, Some(200), Some(200)),
            (Some(300), Some(200), Some(300)),
            (Some(100), Some(400), Some(400)),
            (Some(100), Some(0), Some(100)),
        ] {
            let mut agent = payload(json!([{
                "pane_id":"p", "agent":"claude", "agent_status":"working",
                "state_change_seq":7, "changed_at_unix_ms":999
            }]))
            .agents
            .remove(0);
            let baseline = project_agent(agent.clone()).unwrap();
            agent.facts = Some(crate::request_view::RowFacts {
                operator_request: operator.map(|at| request(at, Requester::Operator)),
                other_request: other.map(|at| request(at, Requester::Agent)),
                ..Default::default()
            });
            let row = project_agent(agent.clone()).unwrap();
            assert_eq!(row.last_prompt_at_unix_ms, expected);
            assert_eq!(row.last_conversation_at_unix_ms, expected);
            assert_eq!(row.changed_at_unix_ms, Some(999));
            // A later reply moves the conversation's clock, not the prompt's.
            agent.facts.as_mut().unwrap().last_reply_at_unix_ms = Some(500);
            let replied = project_agent(agent.clone()).unwrap();
            assert_eq!(replied.last_prompt_at_unix_ms, expected);
            assert_eq!(replied.last_conversation_at_unix_ms, Some(500));
            assert_eq!(row.last_activity, baseline.last_activity);
            assert_eq!(row.state_change_seq, baseline.state_change_seq);
        }
    }

    #[test]
    fn checkout_purpose_uses_agent_then_pull_request_after_persistent_sources() {
        use crate::model::{
            CheckoutPurposeOrigin, CheckoutPurposeSnapshot, CheckoutSnapshot, PullRequestBadge,
            PullRequestChecks, PullRequestSnapshot, WorkspaceSnapshot,
        };
        let mut checkout = CheckoutSnapshot {
            agent_scope: Default::default(),
            id: "checkout".into(),
            agent_summary: crate::model::CheckoutAgentSummary {
                representative_pane_id: Some("pane".into()),
                ..Default::default()
            },
            pull_request: Some(PullRequestSnapshot {
                closing_issues: Default::default(),
                number: 18,
                title: "Pull request fallback".into(),
                head_branch: "feature".into(),
                base_branch: "main".into(),
                url: "https://example.invalid/pull/18".into(),
                badge: PullRequestBadge::Open,
                review: None,
                checks: PullRequestChecks::default(),
                is_draft: false,
                merged_at_unix_ms: None,
                updated_at_unix_ms: None,
                created_at_unix_ms: None,
                closed_at_unix_ms: None,
                head_oid: None,
                cross_repository: false,
                work: Default::default(),
            }),
            ..Default::default()
        };
        let workspace = |checkout| WorkspaceSnapshot {
            agent_presence: Default::default(),
            agent_scope: Default::default(),
            home_issues: Default::default(),
            pull_requests: Vec::new(),
            tasks: Default::default(),
            id: "project".into(),
            label: "Project".into(),
            path: "/fixture/project".into(),
            remote_target_id: None,
            expanded: true,
            device_id: "local".into(),
            repo_name: "Project".into(),
            is_git: true,
            default_branch: Some("main".into()),
            branches: vec![],
            registered: true,
            temporary: false,
            session_workspace_ids: vec![],
            last_activity_unix_ms: None,
            checkouts: vec![checkout],
            pinned: false,
            is_home: false,
            inactive_checkouts: Default::default(),
            session_folds: Default::default(),
            removal: Default::default(),
            disk: Default::default(),
            cleanup: None,
        };
        let agent = project_agents(payload(json!([{
            "pane_id": "pane",
            "workspace_label": "Workspace",
            "agent": "codex",
            "agent_status": "running",
            "state_change_seq": 1,
            "tokens": {"task": "Agent session title"}
        }])))
        .agents
        .remove(0);
        let mut workspaces = vec![workspace(checkout.clone())];

        assert!(sync_checkout_purposes(&mut workspaces, &[agent]));
        assert_eq!(
            workspaces[0].checkouts[0].purpose,
            Some(CheckoutPurposeSnapshot {
                text: "Agent session title".into(),
                origin: CheckoutPurposeOrigin::AgentTitle,
            })
        );

        checkout.agent_summary.representative_pane_id = None;
        let mut workspaces = vec![workspace(checkout.clone())];
        assert!(sync_checkout_purposes(&mut workspaces, &[]));
        assert_eq!(
            workspaces[0].checkouts[0]
                .purpose
                .as_ref()
                .map(|purpose| purpose.origin),
            Some(CheckoutPurposeOrigin::PullRequestTitle)
        );

        checkout.purpose = Some(CheckoutPurposeSnapshot {
            text: "Live token".into(),
            origin: CheckoutPurposeOrigin::Token,
        });
        let mut workspaces = vec![workspace(checkout)];
        assert!(!sync_checkout_purposes(&mut workspaces, &[]));
        assert_eq!(
            workspaces[0].checkouts[0].purpose.as_ref().unwrap().text,
            "Live token"
        );
    }

    #[test]
    fn workspace_summary_uses_physical_ownership_priority_and_unique_panes() {
        use crate::model::{
            AgentStatusCode, CheckoutSnapshot, PaneSnapshot, TabSnapshot, WorkspaceSnapshot,
        };
        let checkout = |id: &str, panes: &[&str]| CheckoutSnapshot {
            agent_scope: Default::default(),
            id: id.to_owned(),
            tabs: vec![TabSnapshot {
                agent: None,
                naming: Default::default(),
                panes: panes
                    .iter()
                    .map(|id| PaneSnapshot {
                        start_claim: None,
                        terminal_id: None,
                        failed_terminal_id: None,
                        id: (*id).to_owned(),
                        herdr_label: None,
                        terminal_title: None,
                        cwd: "/fixture".to_owned(),
                        status_code: AgentStatusCode::Unknown,
                        requires_close_confirmation: false,
                        requires_close_status_check: false,
                        identity_label: None,
                        activity_at_unix_ms: None,
                        fork: Default::default(),
                        ports: vec![],
                        servers: vec![],
                        children: None,
                        lineage_path: Vec::new(),
                        sleep: None,
                        sleep_action: None,
                    })
                    .collect(),
                id: None,
                workspace_id: None,
                checkout_id: None,
                label: None,
                empty: false,
                delegated: false,
            }],
            ..Default::default()
        };
        let mut workspaces = vec![WorkspaceSnapshot {
            agent_presence: Default::default(),
            agent_scope: Default::default(),
            home_issues: Default::default(),
            pull_requests: Vec::new(),
            tasks: Default::default(),
            checkouts: vec![
                checkout(
                    "main",
                    &[
                        "error", "question", "done", "working", "unknown", "shell", "done",
                    ],
                ),
                checkout("child", &["child"]),
                checkout("empty", &["plain"]),
            ],
            id: "project".to_owned(),
            label: "Project".to_owned(),
            path: "/fixture".to_owned(),
            remote_target_id: None,
            expanded: true,
            device_id: "local".to_owned(),
            repo_name: "Project".to_owned(),
            is_git: true,
            default_branch: None,
            branches: vec![],
            registered: true,
            temporary: false,
            session_workspace_ids: vec![],
            last_activity_unix_ms: None,
            pinned: false,
            is_home: false,
            inactive_checkouts: Default::default(),
            session_folds: Default::default(),
            removal: Default::default(),
            disk: Default::default(),
            cleanup: None,
        }];
        let mut agents = project_agents(payload(json!([
            {"pane_id":"error", "state_change_seq":1, "agent_status":"idle"},
            {"pane_id":"question", "state_change_seq":1, "agent_status":"idle", "tokens":{"status_question":"?"}},
            {"pane_id":"done", "state_change_seq":1, "agent_status":"done"},
            {"pane_id":"working", "state_change_seq":1, "agent_status":"working"},
            {"pane_id":"unknown", "state_change_seq":1, "agent_status":"unknown"},
            {"pane_id":"child", "state_change_seq":1, "agent_status":"blocked", "spawned_from_pane_id":"working"}
        ])))
        .agents;
        // The label's `blocked` end is what sets this demand in the product;
        // the row is set on the axis here so the summary's rule for it stays
        // covered without a label.
        let error = agents.iter_mut().find(|a| a.pane_id == "error").unwrap();
        error.demand = "error".to_owned();
        error.unread = false;
        derive_from_axes(error);
        let duplicate = agents.iter().find(|a| a.pane_id == "done").unwrap().clone();
        agents.push(duplicate);
        assert!(sync_checkout_agent_summaries(&mut workspaces, &agents));
        let summaries = &workspaces[0].checkouts;
        assert_eq!(
            summaries[0].agent_summary.representative_pane_id.as_deref(),
            Some("question")
        );
        assert_eq!(
            (
                summaries[0].agent_summary.needs_you,
                summaries[0].agent_summary.done,
                summaries[0].agent_summary.working,
                summaries[0].agent_summary.seen,
                summaries[0].agent_summary.unknown
            ),
            (1, 1, 1, 2, 1)
        );
        // Each row counts under the mark it draws: the read error is still
        // the operator's turn, the done row no label judged is to read, and
        // the unknown row is quiet, which no badge claims.
        assert_eq!(
            summaries[0].agent_summary.marks,
            crate::model::MarkCountsSnapshot {
                turn: 2,
                going: 1,
                unread: 1,
                ..Default::default()
            }
        );
        assert_eq!(
            summaries[1].agent_summary.representative_pane_id.as_deref(),
            Some("child")
        );
        assert_eq!(summaries[2].agent_summary, Default::default());
        assert!(!sync_checkout_agent_summaries(&mut workspaces, &agents));
        let question = agents.iter_mut().find(|a| a.pane_id == "question").unwrap();
        question.unread = false;
        derive_from_axes(question);
        sync_checkout_agent_summaries(&mut workspaces, &agents);
        assert_eq!(
            workspaces[0].checkouts[0]
                .agent_summary
                .representative_pane_id
                .as_deref(),
            Some("done")
        );
        agents.retain(|a| a.pane_id != "done");
        sync_checkout_agent_summaries(&mut workspaces, &agents);
        assert_eq!(
            workspaces[0].checkouts[0]
                .agent_summary
                .representative_pane_id
                .as_deref(),
            Some("working")
        );
        // The read error and the read question are both the operator's turn;
        // once the error's task is resolved it rests in every list, and the
        // badge stops counting it too.
        assert_eq!(workspaces[0].checkouts[0].agent_summary.marks.turn, 2);
        agents
            .iter_mut()
            .find(|a| a.pane_id == "error")
            .unwrap()
            .resolved = Some(crate::agent_state::sessions::Resolution {
            at_unix_ms: 1,
            source: crate::agent_state::sessions::ResolveSource::Operator,
            session: None,
            activity: String::new(),
        });
        sync_checkout_agent_summaries(&mut workspaces, &agents);
        assert_eq!(workspaces[0].checkouts[0].agent_summary.marks.turn, 1);
    }

    fn payload(agents: Value) -> SessionSnapshotPayload {
        owned_label_fixture(json!({"agents": agents})).expect("valid fixture")
    }

    #[test]
    fn current_label_changes_preserve_read_records_lineage_and_control_names() {
        let snapshot = |task: &str| {
            payload(json!([
                {"pane_id":"root","name":"control-root","agent":"claude","agent_status":"working","state_change_seq":5,
                 "agent_session":{"kind":"id","value":"same-root"},"tokens":{"task":task,"progress":"현재 진행"}},
                {"pane_id":"child","name":"control-child","agent":"claude","agent_status":"idle","state_change_seq":6,
                 "agent_session":{"kind":"id","value":"same-child"},"spawned_from_pane_id":"root","tokens":{"task":task,"status_question_new":"?"}}
            ]))
        };
        let mut before = project_agents(snapshot("기존 현재 작업")).agents;
        let mut records = BTreeMap::new();
        apply_read_state(&mut before, &mut records, &mut connection(), Some("root"));
        let saved = records.clone();
        let mut after = project_agents(snapshot("갱신된 현재 작업")).agents;
        assert!(apply_read_state(&mut after, &mut records, &mut connection(), None).is_empty());
        assert_eq!(records, saved);
        for (before, after) in before.iter().zip(&after) {
            assert_eq!(before.id, after.id);
            assert_eq!(before.unread, after.unread);
            assert_eq!(ownership_of(before), ownership_of(after));
            assert_eq!(before.descendant_counts, after.descendant_counts);
            assert_eq!(before.group, after.group);
        }
        assert_eq!(after[0].identity_label, "갱신된 현재 작업");
    }

    /// AC1. A question comes only from the core's label and outranks the
    /// approval a blocked pane shows; a label without one leaves the demand
    /// to Herdr's lifecycle.
    #[test]
    fn a_labels_question_is_the_only_question_demand() {
        let label = |question: bool| json!({"task": "작업", "question": question});
        let projected = project_agents(payload(json!([
            {"pane_id":"asks","agent_status":"idle","state_change_seq":1,"label":label(true)},
            {"pane_id":"asks-done","agent_status":"done","state_change_seq":2,"label":label(true)},
            {"pane_id":"asks-blocked","agent_status":"blocked","state_change_seq":3,"label":label(true)},
            {"pane_id":"quiet","agent_status":"idle","state_change_seq":4,"label":label(false)},
            {"pane_id":"blocked","agent_status":"blocked","state_change_seq":5,"label":label(false)}
        ])))
        .agents;
        let axes = projected
            .iter()
            .map(|agent| {
                (
                    agent.pane_id.as_str(),
                    (agent.demand.as_str(), agent.activity.as_str(), agent.mark),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(axes["asks"], ("question", "stopped", RowMark::Turn));
        assert_eq!(axes["asks-done"], ("question", "stopped", RowMark::Turn));
        assert_eq!(axes["asks-blocked"], ("question", "unknown", RowMark::Turn));
        assert_eq!(axes["quiet"], ("none", "stopped", RowMark::Quiet));
        assert_eq!(axes["blocked"], ("approval", "unknown", RowMark::Turn));
    }

    /// AC1. Herdr's own lifecycle words, with no plugin token at all. Blocked
    /// is an approval, and `done` and `idle` produce the same activity. They
    /// differ on completion evidence: an idle agent is merely ready, while a
    /// done agent has a result to review. Neither lifecycle decides Hide's
    /// pane-level read state.
    #[test]
    fn axes_map_herdr_lifecycles_with_blocked_as_approval() {
        let projection = project_agents(payload(json!([
            {"pane_id":"working","agent_status":"working","state_change_seq":1},
            {"pane_id":"blocked","agent_status":"blocked","state_change_seq":2},
            {"pane_id":"done","agent_status":"done","state_change_seq":3},
            {"pane_id":"idle","agent_status":"idle","state_change_seq":4},
            {"pane_id":"unknown","agent_status":"unknown","state_change_seq":5}
        ])));

        assert!(projection.excluded.is_empty());
        let axes = projection
            .agents
            .iter()
            .map(|agent| {
                (
                    agent.pane_id.as_str(),
                    (
                        agent.demand.as_str(),
                        agent.activity.as_str(),
                        agent.completed,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(axes["working"], ("none", "working", false));
        assert_eq!(axes["blocked"], ("approval", "unknown", false));
        assert_eq!(axes["done"], ("none", "stopped", true));
        assert_eq!(axes["idle"], ("none", "stopped", false));
        assert_eq!(axes["unknown"], ("none", "unknown", false));
        assert_eq!(
            axes["done"].1, axes["idle"].1,
            "done and idle are both stopped"
        );
        assert_ne!(
            axes["done"].2, axes["idle"].2,
            "only a reported completion can become Done"
        );
        let idle = projection
            .agents
            .iter()
            .find(|agent| agent.pane_id == "idle")
            .expect("idle agent");
        assert_eq!(idle.group, "seen");
        assert_eq!(idle.status_code, AgentStatusCode::Idle);
        assert_eq!(idle.mark, RowMark::Quiet);
        let done = projection
            .agents
            .iter()
            .find(|agent| agent.pane_id == "done")
            .expect("completed agent");
        // Herdr's `done` with no label read: a stop to read, not a finish.
        assert_eq!(done.group, "done");
        assert_eq!(done.status_code, AgentStatusCode::Stopped);
        assert_eq!(done.mark, RowMark::Unread);
        assert_eq!(
            projection.agents.len(),
            5,
            "an agent with no plugin token still projects"
        );
    }

    /// AC1, AC10. A blocked pane sits in Needs You whether or not it has been
    /// read, because the approval prompt is still on screen.
    #[test]
    fn axes_hold_a_blocked_pane_in_needs_you_after_it_is_read() {
        assert_eq!(
            agent_group_for(
                AgentDemand::Approval,
                AgentActivity::Unknown,
                false,
                false,
                true,
                Ownership::Operator,
                false
            ),
            AgentGroup::NeedsYou
        );
        assert_eq!(
            agent_group_for(
                AgentDemand::Approval,
                AgentActivity::Unknown,
                false,
                false,
                false,
                Ownership::Operator,
                false
            ),
            AgentGroup::Seen
        );
    }

    /// AC10. Closing a pane needs confirmation for running work or unresolved
    /// demand. An unknown activity is a separate status-check outcome, not a
    /// confirmation that guesses whether work is running.
    #[test]
    fn axes_require_close_confirmation_or_status_check_from_work_and_demand() {
        let cases = [
            (
                AgentDemand::None,
                AgentActivity::Working,
                false,
                false,
                true,
                false,
            ),
            (
                AgentDemand::Question,
                AgentActivity::Stopped,
                true,
                false,
                true,
                false,
            ),
            (
                AgentDemand::Approval,
                AgentActivity::Unknown,
                false,
                true,
                true,
                false,
            ),
            (
                AgentDemand::None,
                AgentActivity::Stopped,
                true,
                false,
                false,
                false,
            ),
            (
                AgentDemand::None,
                AgentActivity::Stopped,
                false,
                false,
                false,
                false,
            ),
            (
                AgentDemand::Question,
                AgentActivity::Stopped,
                false,
                false,
                true,
                false,
            ),
            (
                AgentDemand::None,
                AgentActivity::Unknown,
                true,
                false,
                false,
                true,
            ),
        ];
        for (demand, activity, unread, blocked, expected, status_check) in cases {
            let _group = agent_group_for(
                demand,
                activity,
                false,
                unread,
                blocked,
                Ownership::Operator,
                false,
            );
            assert_eq!(
                agent_requires_close_confirmation(activity, demand, blocked),
                expected,
                "{demand:?} {activity:?} unread={unread} blocked={blocked}"
            );
            assert_eq!(
                agent_requires_close_status_check(activity, demand, blocked),
                status_check,
                "status check: {demand:?} {activity:?} unread={unread} blocked={blocked}"
            );
        }
    }

    /// AC10. The shell keeps no list of agent state names of its own.
    ///
    /// Five copies of that vocabulary had drifted apart, which is how the pet
    /// dashboard and the sidebar came to disagree about whether a finished
    /// agent needs attention. The shell now reads the values derived here, so
    /// two marks of the old vocabulary are refused: the retired
    /// `unseen_completion` anywhere at all, and any literal array that lists
    /// both `blocked` and `question`, which only the retired state arrays did.
    #[test]
    fn axes_no_state_name_array_remains_in_the_shell() {
        let shell = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../web/src")
            .canonicalize()
            .expect("the shell sits beside the core");
        let mut offenders = Vec::new();
        let mut scanned = 0_usize;
        let mut stack = vec![shell];
        while let Some(entry) = stack.pop() {
            if entry.is_dir() {
                for child in std::fs::read_dir(&entry).expect("readable directory") {
                    stack.push(child.expect("readable entry").path());
                }
                continue;
            }
            if !matches!(
                entry.extension().and_then(|value| value.to_str()),
                Some("ts" | "tsx")
            ) {
                continue;
            }
            scanned += 1;
            let text = std::fs::read_to_string(&entry).expect("readable source");
            if text.contains("unseen_completion") {
                offenders.push(format!(
                    "{}: the retired unseen_completion state",
                    entry.display()
                ));
            }
            let mut rest = text.as_str();
            while let Some(open) = rest.find('[') {
                let after = &rest[open + 1..];
                let Some(close) = after.find(']') else { break };
                let span = &after[..close];
                if span.contains("\"blocked\"") && span.contains("\"question\"") {
                    offenders.push(format!("{}: a literal agent state array", entry.display()));
                }
                rest = &after[close + 1..];
            }
        }
        assert!(scanned > 0, "no shell sources were scanned");
        assert!(
            offenders.is_empty(),
            "the shell still keeps its own agent state vocabulary: {offenders:#?}"
        );
    }

    /// AC10. No view builds a label out of an axis value, so nothing
    /// underscored can reach the screen.
    #[test]
    fn axes_map_to_one_status_code_each() {
        let labels = [
            (
                AgentDemand::Question,
                AgentActivity::Unknown,
                false,
                true,
                AgentStatusCode::Question,
            ),
            (
                AgentDemand::Approval,
                AgentActivity::Unknown,
                false,
                true,
                AgentStatusCode::Approval,
            ),
            (
                AgentDemand::Error,
                AgentActivity::Unknown,
                false,
                true,
                AgentStatusCode::Error,
            ),
            (
                AgentDemand::None,
                AgentActivity::Working,
                false,
                true,
                AgentStatusCode::Working,
            ),
            (
                AgentDemand::None,
                AgentActivity::Stopped,
                true,
                true,
                AgentStatusCode::Stopped,
            ),
            (
                AgentDemand::None,
                AgentActivity::Stopped,
                false,
                true,
                AgentStatusCode::Idle,
            ),
            (
                AgentDemand::None,
                AgentActivity::Stopped,
                true,
                false,
                AgentStatusCode::Idle,
            ),
            (
                AgentDemand::None,
                AgentActivity::Unknown,
                false,
                true,
                AgentStatusCode::Unknown,
            ),
        ];
        for (demand, activity, completed, unread, expected) in labels {
            assert_eq!(
                agent_status_code(demand, activity, completed, unread, false),
                expected
            );
        }
        // Done is only a finish the label read (D-03).
        assert_eq!(
            agent_status_code(AgentDemand::None, AgentActivity::Stopped, true, true, true),
            AgentStatusCode::Done
        );
    }

    /// AC5, R3, R4. Group membership follows the definitions, the order is
    /// group first and most recent activity second, and the label plugin's
    /// `sort_rank` token has no say: every row here carries one that would
    /// invert the result if Hide still read it.
    #[test]
    fn group_order_follows_the_four_groups_then_recent_activity() {
        let mut agents = projected(json!([
            {"pane_id":"read-idle","agent_status":"idle","state_change_seq":1,
             "tokens":{"status_idle":"○","sort_rank":"00","activity":"0000000000009"}},
            {"pane_id":"done-older","agent_status":"done","state_change_seq":2,
             "tokens":{"status_done_new":"●","sort_rank":"00","activity":"0000000000001"}},
            {"pane_id":"working","agent_status":"working","state_change_seq":3,
             "tokens":{"status_working":"●","sort_rank":"99","activity":"0000000000008"}},
            {"pane_id":"done-newer","agent_status":"done","state_change_seq":4,
             "tokens":{"status_done_new":"●","sort_rank":"00","activity":"0000000000002"}},
            {"pane_id":"asking","agent_status":"idle","state_change_seq":5,
             "tokens":{"status_question_new":"?","sort_rank":"99","activity":"0000000000000"}},
            {"pane_id":"blocked","agent_status":"blocked","state_change_seq":6,
             "tokens":{"activity":"0000000000003"}}
        ]));
        let mut records = BTreeMap::new();
        // An ordinary idle pane stays Seen even before a read record exists.
        records.insert(
            "read-idle".to_owned(),
            PaneReadRecord {
                state_change_seq: Some(1),
                session_id: None,
                demand: "none".to_owned(),
                activity: "stopped".to_owned(),
                completed: false,
                descendant_signals: BTreeSet::new(),
                server: Some(SERVER.to_owned()),
                unread: false,
                checks_ended: None,
            },
        );
        apply_read_state(&mut agents, &mut records, &mut connection(), None);

        assert_eq!(
            agents
                .iter()
                .map(|agent| (agent.pane_id.as_str(), agent.group.as_str()))
                .collect::<Vec<_>>(),
            [
                ("blocked", "needs_you"),
                ("asking", "needs_you"),
                ("done-newer", "done"),
                ("done-older", "done"),
                ("working", "working"),
                ("read-idle", "seen"),
            ]
        );
    }

    /// AC5. Two rows in the same group with the same activity keep the order
    /// Herdr sent them in, because the sort is stable and reads no third key.
    #[test]
    fn group_order_keeps_snapshot_order_for_equal_activity() {
        let same = |pane_id: &str| {
            json!({"pane_id": pane_id, "agent_status": "done", "state_change_seq": 1,
                   "tokens": {"status_done_new": "●", "activity": "0000000000004"}})
        };
        let mut agents = projected(json!([same("first"), same("second"), same("third")]));
        let mut records = BTreeMap::new();
        apply_read_state(&mut agents, &mut records, &mut connection(), None);
        assert_eq!(
            agents
                .iter()
                .map(|agent| agent.pane_id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"]
        );
    }

    /// PRD D-01, checkout-workspace-binding D-09: the rolling task is the
    /// title and the provider is the fallback. Herdr's agent name, the
    /// retired `name` token and the Herdr workspace label (`home-graph`,
    /// another checkout's name as often as this one's) are never titles.
    #[test]
    fn identity_ladder_uses_task_then_provider_and_ignores_names_and_workspace_labels() {
        let projected = projected(json!([
            {"pane_id":"p2","id":"impl-x","agent":"claude","workspace_label":"home-graph","tokens":{"status_working":"●","activity":"0000000000002","name":"Hook 버그 확인","task":"hook 보고 경로 수정"}},
            {"pane_id":"p3","id":"sasu-implementor","agent":"claude","workspace_label":"home-graph","tokens":{"status_working":"●","activity":"0000000000003","name":"첫 프롬프트"}},
            {"pane_id":"p4","id":"p4","agent":"codex","workspace_label":"web-shell-pivot-s3","tokens":{"status_working":"●","activity":"0000000000004"}},
            {"pane_id":"p5","id":"p5","agent":"gemini","workspace_label":"home-graph","tokens":{"status_working":"●","activity":"0000000000005"}},
            {"pane_id":"p6","id":"p6","workspace_label":"home-graph","agent_status":"working","tokens":{"activity":"0000000000006"}}
        ]));
        let labels = projected
            .iter()
            .map(|agent| (agent.pane_id.as_str(), agent.identity_label.as_str()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            labels.into_values().collect::<Vec<_>>(),
            ["hook 보고 경로 수정", "Claude", "Codex", "gemini", "Agent"]
        );
    }

    /// status-model.md: manual, goal, automatic, provider for every agent.
    /// Reader capability determines which facts can reach this ladder.
    #[test]
    fn all_shared_surfaces_receive_manual_then_goal_then_automatic_then_provider() {
        for (kind, provider) in [
            ("claude", "Claude"),
            ("codex", "Codex"),
            ("grok", "grok"),
            ("opencode", "OpenCode"),
            ("pi", "pi"),
            ("omp", "omp"),
            ("cursor", "cursor"),
        ] {
            for (manual, goal, automatic, expected) in [
                (
                    Some("운영자 이름"),
                    Some("Hide 작업명"),
                    Some("자동 제목"),
                    "운영자 이름",
                ),
                (None, Some("Hide 작업명"), Some("자동 제목"), "Hide 작업명"),
                (None, None, Some("자동 제목"), "자동 제목"),
                (None, None, None, provider),
                (Some("운영자 이름"), None, Some("자동 제목"), "운영자 이름"),
                (
                    Some(" \n\t"),
                    Some("Hide 작업명"),
                    Some("자동 제목"),
                    "Hide 작업명",
                ),
                (None, Some(" \n\t"), Some("自動 제목"), "自動 제목"),
                (None, None, Some(" \n\t"), provider),
            ] {
                let mut agent: SessionAgentPayload = serde_json::from_value(json!({
                    "pane_id":"p", "id":"control-name", "agent":kind,
                    "workspace_label":"unrelated checkout", "agent_status":"idle", "state_change_seq":1
                }))
                .unwrap();
                agent.label = Some(AgentLabel {
                    task: goal.map(str::to_owned),
                    progress: None,
                    expected_reply: None,
                    question: false,
                    blocked: false,
                });
                agent.facts = Some(crate::request_view::RowFacts {
                    native_title: automatic.map(str::to_owned),
                    native_custom_title: manual.map(str::to_owned),
                    ..Default::default()
                });
                assert_eq!(
                    project_agent(agent).unwrap().identity_label,
                    expected,
                    "{kind}"
                );
            }
        }
    }

    #[test]
    fn a_reported_pi_id_grants_no_fork_until_the_current_reader_proves_it() {
        let mut agent: SessionAgentPayload = serde_json::from_value(json!({
            "pane_id":"p", "agent":"pi", "agent_status":"idle", "state_change_seq":1,
            "agent_session":{"kind":"id", "value":"reported-native-id"}
        }))
        .unwrap();
        assert!(project_agent(agent.clone()).unwrap().session_id.is_none());
        agent.facts = Some(crate::request_view::RowFacts {
            native_session_id: Some("proven-native-id".into()),
            ..Default::default()
        });
        assert_eq!(
            project_agent(agent).unwrap().session_id.as_deref(),
            Some("proven-native-id")
        );
    }

    /// The old `summary` token is not read: a plugin still publishing it
    /// names nothing and prompts nothing (PRD B13).
    #[test]
    fn a_legacy_summary_token_is_ignored() {
        let projected = projected(json!([
            {"pane_id":"p1","id":"p1","agent":"codex","workspace_label":"task-factory","tokens":{"status_idle":"○","activity":"0000000000001","summary":"Check agent-context-labels settings"}}
        ]));
        assert_eq!(projected[0].identity_label, "Codex");
        assert_eq!(projected[0].detail, None);
    }

    /// PRD overview-lenses-tiles-agents D-50: `message` carries both hook
    /// sentences whole, request first and line breaks kept, while `detail`
    /// stays the row's one cut sentence; an agent that reported neither has none.
    #[test]
    fn message_carries_both_hook_sentences_uncut() {
        let long_reply = "가".repeat(60);
        let projected = projected(json!([
            {"pane_id":"p1","id":"p1","workspace_label":"hide","tokens":{"status_working":"●","activity":"0000000000001","progress":"  hook 보고\n경로 교체 중  ","expected_reply":long_reply}},
            {"pane_id":"p2","id":"p2","workspace_label":"hide","tokens":{"status_idle":"○","activity":"0000000000002","progress":"  "}}
        ]));
        assert_eq!(
            projected[0].message.as_deref(),
            Some(format!("{long_reply}\nhook 보고\n경로 교체 중").as_str())
        );
        assert_eq!(projected[1].message, None);
        let wire = serde_json::to_value(&projected[1]).unwrap();
        assert!(
            wire.get("message").is_none(),
            "an agent that said nothing carries no key"
        );
    }

    /// PRD D-05: the sentences are one line each and `expected_reply` is cut
    /// at the plugin's own 40-character rule.
    #[test]
    fn sentences_are_collapsed_and_expected_reply_is_cut_at_forty() {
        let long_reply = "가".repeat(60);
        let projected = projected(json!([
            {"pane_id":"p1","id":"p1","workspace_label":"hide","tokens":{"status_working":"●","activity":"0000000000001","progress":"  hook   보고\n경로 교체 중  ","expected_reply":long_reply}}
        ]));
        assert_eq!(
            projected[0].progress.as_deref(),
            Some("hook 보고 경로 교체 중")
        );
        assert_eq!(
            projected[0]
                .expected_reply
                .as_deref()
                .map(|value| value.chars().count()),
            Some(MAX_EXPECTED_REPLY_CHARS)
        );
    }

    /// PRD D-06: the second line by group. `agent_second_line` is what
    /// `derive_from_axes` writes into `detail` and `status_word_visible`.
    /// An unresolved demand keeps its request in every group, so a read
    /// question and a delegated child's approval still say what they ask
    /// (sidebar-agent-status B7).
    #[test]
    fn second_line_is_chosen_by_group_and_a_request_outlives_reading() {
        use AgentDemand::{None as Quiet, Question};
        let reply = Some("A/B 선택 후 승인");
        let progress = Some("푸시 완료, 승인 대기 중");
        let native = Some("어디에 배포할까요?");
        let asked = (false, Some("A/B 선택 후 승인".to_owned()));
        let said = (false, Some("푸시 완료, 승인 대기 중".to_owned()));
        let quoted = (false, Some("어디에 배포할까요?".to_owned()));
        let cases = [
            (
                AgentGroup::NeedsYou,
                Question,
                reply,
                None,
                progress,
                asked.clone(),
            ),
            (
                AgentGroup::NeedsYou,
                Question,
                None,
                None,
                progress,
                said.clone(),
            ),
            (
                AgentGroup::NeedsYou,
                Question,
                None,
                None,
                None,
                (true, None),
            ),
            // A native question waiting now is what is asked; the label's
            // reply, when it has one, is still the shorter thing to do.
            (
                AgentGroup::NeedsYou,
                Question,
                None,
                native,
                progress,
                quoted.clone(),
            ),
            (
                AgentGroup::NeedsYou,
                Question,
                None,
                native,
                None,
                quoted.clone(),
            ),
            (
                AgentGroup::NeedsYou,
                Question,
                reply,
                native,
                progress,
                asked.clone(),
            ),
            (AgentGroup::Done, Quiet, None, None, progress, said.clone()),
            (
                AgentGroup::Working,
                Quiet,
                reply,
                None,
                progress,
                said.clone(),
            ),
            (AgentGroup::Working, Quiet, None, None, None, (true, None)),
            // A delegated child still running while it asks.
            (
                AgentGroup::Working,
                Question,
                reply,
                None,
                progress,
                asked.clone(),
            ),
            (
                AgentGroup::Working,
                Question,
                None,
                native,
                progress,
                quoted.clone(),
            ),
            (
                AgentGroup::Seen,
                Quiet,
                reply,
                None,
                progress,
                (false, None),
            ),
            // A read question, and a delegated child's question, in Seen.
            (AgentGroup::Seen, Question, reply, None, progress, asked),
            (AgentGroup::Seen, Question, None, None, progress, said),
            (AgentGroup::Seen, Question, None, native, progress, quoted),
            (AgentGroup::Seen, Question, None, None, None, (false, None)),
        ];
        for (group, demand, reply, question, progress, expected) in cases {
            assert_eq!(
                agent_second_line(group, demand, false, reply, question, progress),
                expected,
                "{group:?} {demand:?} reply={reply:?} question={question:?} progress={progress:?}"
            );
        }
    }

    /// The projected row carries the second line: a working row shows its
    /// progress without the word, and the chip every lineage surface draws
    /// says the same thing (PRD B4, D-06).
    #[test]
    fn projected_rows_carry_the_second_line_and_the_chip_repeats_it() {
        let projected = projected(json!([
            {"pane_id":"p1","id":"p1","workspace_label":"hide","agent_status":"working","tokens":{"status_working":"●","activity":"0000000000001","task":"Hook 버그 확인","progress":"hook 보고 경로를 소켓 호출로 교체 중","expected_reply":"무시됨"}},
            {"pane_id":"p2","id":"p2","workspace_label":"hide","tokens":{"status_question_new":"?","activity":"0000000000002","progress":"푸시 완료","expected_reply":"A/B 선택"}},
            {"pane_id":"p3","id":"p3","workspace_label":"hide","tokens":{"status_idle":"○","activity":"0000000000003"}}
        ]));
        assert_eq!(
            projected[0].detail.as_deref(),
            Some("hook 보고 경로를 소켓 호출로 교체 중")
        );
        assert!(!projected[0].status_word_visible);
        assert_eq!(projected[1].detail.as_deref(), Some("A/B 선택"));
        assert!(!projected[1].status_word_visible);
        // A newly opened idle row stays Seen with no second-line sentence.
        assert_eq!(projected[2].detail, None);
        assert!(!projected[2].status_word_visible);
        let chip = agent_chip(&projected[0]);
        assert_eq!(chip.label, "Hook 버그 확인");
        assert_eq!(
            chip.detail.as_deref(),
            Some("hook 보고 경로를 소켓 호출로 교체 중")
        );
        assert!(!chip.status_word_visible);
    }

    #[test]
    fn one_broken_agent_excludes_only_itself_and_names_why() {
        let projection = project_agents(payload(json!([
            {"pane_id":"good","tokens":{"status_working":"●","activity":"0000000000002"}},
            {"pane_id":"unordered","tokens":{"status_idle":"○"}},
            {"tokens":{"status_idle":"○","activity":"0000000000000"}},
            {"pane_id":"also-good","tokens":{"status_idle":"○","activity":"0000000000003"}}
        ])));

        assert_eq!(
            projection
                .agents
                .iter()
                .map(|agent| agent.pane_id.as_str())
                .collect::<Vec<_>>(),
            ["good", "also-good"]
        );
        assert_eq!(projection.excluded.len(), 2);
        assert_eq!(projection.excluded[0].pane_id.as_deref(), Some("unordered"));
        assert!(projection.excluded[0].reason.contains("state_change_seq"));
        assert_eq!(projection.excluded[1].pane_id, None);
        assert!(projection.excluded[1].reason.contains("pane id"));
    }

    #[test]
    fn official_state_change_sequence_orders_an_agent_the_core_has_not_timed() {
        let projection = project_agents(payload(json!([
            {
                "pane_id":"remote",
                "state_change_seq":218,
                "agent_status":"done",
                "tokens":{"task":"finished up"}
            },
            {
                "pane_id":"timed",
                "state_change_seq":219,
                "agent_status":"idle",
                "changed_at_unix_ms": 1_700_000_000_000_u64
            }
        ])));

        assert_eq!(projection.agents[0].last_activity, "00000000000000000218");
        assert_eq!(projection.agents[1].last_activity, "1700000000000");
        assert!(projection.excluded.is_empty());
    }

    fn projected(agents: Value) -> Vec<SidebarAgentSnapshot> {
        project_agents(payload(agents)).agents
    }

    /// The mark of the one Herdr server most read tests run on.
    const SERVER: &str = "server-1";

    /// A connection to that server, whose panes carry no mark yet, so a state
    /// read on it names `SERVER`.
    fn connection() -> ReadConnection {
        ReadConnection::new(SERVER.to_owned())
    }

    fn finished(pane_id: &str, seq: u64) -> Value {
        json!({
            "pane_id": pane_id,
            "workspace_label": "Fixture",
            "agent": "codex",
            "agent_status": "done",
            "state_change_seq": seq,
            "tokens": {"status_done_new": "\u{25cf}", "activity": "0000000000001"}
        })
    }

    /// AC2, SC1. Three panes finish in one tab. Focusing one clears that one
    /// and leaves the other two unread, which is the whole reason Hide keeps a
    /// pane-level record instead of reading Herdr's tab-scoped seen.
    #[test]
    fn read_record_clears_one_pane_of_a_finished_tab() {
        let mut agents = projected(json!([
            finished("a", 1),
            finished("b", 2),
            finished("c", 3)
        ]));
        let mut records = BTreeMap::new();

        apply_read_state(&mut agents, &mut records, &mut connection(), None);
        assert!(
            agents
                .iter()
                .all(|agent| agent.unread && agent.group == "done"),
            "nothing is read before the operator focuses anything"
        );

        apply_read_state(&mut agents, &mut records, &mut connection(), Some("b"));
        let groups = agents
            .iter()
            .map(|agent| (agent.pane_id.as_str(), (agent.unread, agent.group.as_str())))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(groups["a"], (true, "done"));
        assert_eq!(groups["b"], (false, "seen"));
        assert_eq!(groups["c"], (true, "done"));
    }

    /// AC2. Herdr reporting the whole tab as seen, by dropping every pane from
    /// `done` to `idle` and bumping its sequence, does not read anything. Only
    /// Hide's own record does.
    #[test]
    fn read_record_ignores_herdr_marking_the_tab_seen() {
        let mut agents = projected(json!([finished("a", 1), finished("b", 2)]));
        let mut records = BTreeMap::new();
        apply_read_state(&mut agents, &mut records, &mut connection(), Some("b"));

        let seen_by_herdr = json!([
            {"pane_id":"a","agent_status":"idle","state_change_seq":9,
             "tokens":{"status_done":"\u{25cf}","activity":"0000000000001"}},
            {"pane_id":"b","agent_status":"idle","state_change_seq":9,
             "tokens":{"status_done":"\u{25cf}","activity":"0000000000001"}}
        ]);
        let mut agents = projected(seen_by_herdr);
        apply_read_state(&mut agents, &mut records, &mut connection(), None);
        assert!(
            agents.iter().all(|agent| agent.unread),
            "Herdr's tab-scoped seen never decides Hide's read axis"
        );
    }

    /// #901. Herdr marks the pane's tab seen when it focuses it, which drops a
    /// finished pane from `done` to `idle` without moving its sequence. A
    /// demand the operator already read stays read through that, even when it
    /// lands after the operator's focus has left the pane: the completion going
    /// away is not news, the way a descendant's signal going away is not.
    #[test]
    fn a_read_demand_stays_read_when_herdr_marks_its_tab_seen_after_focus_left() {
        let asking = |status: &str| {
            json!([{
                "pane_id":"a","agent_status":status,"state_change_seq":4,
                "tokens":{"status_question_new":"?","activity":"0000000000004"}
            }])
        };
        let mut records = BTreeMap::new();
        let mut finished = projected(asking("done"));
        apply_read_state(&mut finished, &mut records, &mut connection(), Some("a"));
        assert!(!finished[0].unread, "the operator read the question");

        let mut seen = projected(asking("idle"));
        apply_read_state(
            &mut seen,
            &mut records,
            &mut connection(),
            Some("elsewhere"),
        );
        assert!(
            !seen[0].unread,
            "Herdr's seen at the same sequence returns nothing to unread"
        );
        assert_eq!(seen[0].group, "seen");
    }

    /// #901. What does turn a read demand unread again: the agent working
    /// again, a turn between two looks (Herdr's sequence moved), a new session
    /// in the pane, and a completion that appears after the operator read the
    /// pane, including one that appears at the sequence the read was made at.
    /// Herdr's seen keeps the completion (`completion_seq`), so it is not one
    /// going away and coming back.
    #[test]
    fn a_read_demand_turns_unread_again_only_for_news() {
        let state = |status: &str, seq: u64, session: &str| {
            json!([{
                "pane_id":"a","agent_status":status,"state_change_seq":seq,
                "agent_session":{"source":"herdr:codex","agent":"codex","kind":"id","value":session},
                "tokens":{"status_question_new":"?","activity":"0000000000004"}
            }])
        };
        let read = |status: &str| {
            let mut records = BTreeMap::new();
            let mut agents = projected(state(status, 4, "s1"));
            apply_read_state(&mut agents, &mut records, &mut connection(), Some("a"));
            assert!(!agents[0].unread);
            records
        };
        let unread = |records: &mut BTreeMap<String, PaneReadRecord>, status, seq, session| {
            let mut agents = projected(state(status, seq, session));
            apply_read_state(&mut agents, records, &mut connection(), Some("elsewhere"));
            agents[0].unread
        };

        let mut records = read("done");
        assert!(
            unread(&mut records, "working", 4, "s1"),
            "the agent works again"
        );
        let mut records = read("done");
        assert!(
            unread(&mut records, "done", 6, "s1"),
            "a turn ran between two looks"
        );
        let mut records = read("done");
        assert!(
            unread(&mut records, "done", 4, "s2"),
            "the pane holds another session"
        );

        let mut records = read("idle");
        assert!(
            unread(&mut records, "done", 4, "s1"),
            "a completion appearing at the sequence the read was made at is news"
        );
        let mut records = read("done");
        assert!(
            !unread(&mut records, "idle", 4, "s1"),
            "Herdr's seen is not news"
        );
    }

    /// AC3, SC2. A demand raised on the pane the operator is already looking at
    /// is read on arrival; the same demand raised after focus moved away is
    /// unread.
    #[test]
    fn read_record_follows_the_focused_pane() {
        let asking = json!([{
            "pane_id":"a","agent_status":"idle","state_change_seq":2,
            "tokens":{"status_question_new":"?","activity":"0000000000002"}
        }]);
        let mut records = BTreeMap::new();

        let mut watched = projected(asking.clone());
        apply_read_state(&mut watched, &mut records, &mut connection(), Some("a"));
        assert!(!watched[0].unread, "a question on the focused pane is read");
        assert_eq!(watched[0].group, "seen");

        let moved_on = json!([{
            "pane_id":"a","agent_status":"idle","state_change_seq":3,
            "tokens":{"status_question_new":"?","activity":"0000000000003"}
        }]);
        let mut later = projected(moved_on);
        apply_read_state(
            &mut later,
            &mut records,
            &mut connection(),
            Some("elsewhere"),
        );
        assert!(later[0].unread, "a new question raised elsewhere is unread");
        assert_eq!(later[0].group, "needs_you");
    }

    /// AC3. The axes alone can move without Herdr's sequence moving, so a
    /// demand appearing at an unchanged sequence still counts as a change.
    #[test]
    fn read_record_notices_an_axis_change_at_the_same_sequence() {
        let mut records = BTreeMap::new();
        let mut working = projected(json!([{
            "pane_id":"a","agent_status":"working","state_change_seq":7,
            "tokens":{"status_working":"\u{25cf}","activity":"0000000000001"}
        }]));
        apply_read_state(&mut working, &mut records, &mut connection(), Some("a"));
        assert!(!working[0].unread);

        let mut asking = projected(json!([{
            "pane_id":"a","agent_status":"working","state_change_seq":7,
            "tokens":{"status_question_new":"?","status_working":"\u{25cf}","activity":"0000000000001"}
        }]));
        apply_read_state(&mut asking, &mut records, &mut connection(), None);
        assert!(
            asking[0].unread,
            "a new demand is unread even at the same sequence"
        );
    }

    /// AC4. Agent detection can be empty while the pane still exists. Only an
    /// authoritative pane topology may evict its read record, and applying the
    /// same topology twice converges (engineering rule 11).
    #[test]
    fn read_records_are_evicted_only_against_authoritative_pane_topology() {
        let mut agents = projected(json!([finished("a", 1)]));
        let mut records = BTreeMap::new();
        apply_read_state(&mut agents, &mut records, &mut connection(), Some("a"));
        let after_first = records.clone();
        assert!(
            apply_read_state(&mut agents, &mut records, &mut connection(), Some("a")).is_empty()
        );
        assert_eq!(records, after_first, "a repeated apply changes nothing");

        let mut none: Vec<SidebarAgentSnapshot> = Vec::new();
        apply_read_state(&mut none, &mut records, &mut connection(), None);
        assert!(
            records.contains_key("a"),
            "an empty agent list must not wipe a pane record"
        );
        let live = HashSet::from(["a".to_owned()]);
        assert!(prune_read_records(&mut records, &live, ReadRecordScope::Local).is_empty());
        let gone = HashSet::new();
        assert_eq!(
            prune_read_records(&mut records, &gone, ReadRecordScope::Local).len(),
            1
        );
        assert!(
            records.is_empty(),
            "a pane the authoritative topology dropped leaves no record"
        );
    }

    /// PRD sidebar-context-menus D-06: an agent row carries the session id
    /// Herdr recorded, which Copy session id reads, and none when it recorded
    /// none or recorded a path.
    #[test]
    fn an_agent_row_carries_the_session_id_herdr_recorded() {
        let agent = |pane: &str, session: Value| {
            let mut agent = finished(pane, 1);
            agent["agent_session"] = session;
            agent
        };
        let rows = projected(json!([
            agent("a", json!({"kind":"id","value":"session-a"})),
            agent("b", Value::Null),
            agent("c", json!({"kind":"path","value":"/tmp/session.jsonl"})),
        ]));
        assert_eq!(rows.len(), 3);
        let wire = |pane: &str| {
            serde_json::to_value(rows.iter().find(|row| row.pane_id == pane).unwrap()).unwrap()
        };
        assert_eq!(wire("a")["session_id"], "session-a");
        assert!(wire("b").get("session_id").is_none());
        assert!(wire("c").get("session_id").is_none());
    }

    /// One pane as a Herdr server reports it, running session `session`.
    fn pane(
        pane_id: &str,
        status: &str,
        seq: u64,
        completion: Option<u64>,
        session: &str,
    ) -> Value {
        json!({
            "pane_id": pane_id,
            "agent": "claude",
            "agent_status": status,
            "state_change_seq": seq,
            "completion_seq": completion,
            "agent_session": {"kind":"id","value":session},
            "lineage_session": crate::wire::session_digest(session).unwrap(),
            "tokens": {"activity":"0000000000001"}
        })
    }

    /// What Hide keeps about the panes across passes and restarts.
    #[derive(Default)]
    struct Saved {
        records: BTreeMap<String, PaneReadRecord>,
        completions: BTreeMap<String, crate::model::PaneCompletion>,
    }

    /// One sync pass the way the runtime runs it: completions kept first,
    /// then the lineage, then the read axis.
    fn pass(
        rows: Value,
        connection: &mut ReadConnection,
        saved: &mut Saved,
        focused: Option<&str>,
    ) -> Vec<SidebarAgentSnapshot> {
        let mut agents = projected(rows);
        apply_completions(&mut agents, &mut saved.completions, connection);
        apply_lineage(&mut agents, &[], &[]);
        apply_read_state(&mut agents, &mut saved.records, connection, focused);
        agents
    }

    /// A restored Herdr server lists a pane at sequence 0, then `unknown`,
    /// then settled at a sequence of its own (measured on the pinned 0.9.3).
    /// A pane the operator read stays read through each step, keeps the
    /// completion the restored server no longer reports, and is judged on
    /// the new server's sequence from then on.
    #[test]
    fn a_read_pane_stays_read_through_a_restored_servers_steps() {
        let mut saved = Saved::default();
        let mut first = connection();
        pass(
            json!([pane("a", "done", 40, Some(40), "s1")]),
            &mut first,
            &mut saved,
            Some("a"),
        );

        let mut restored = ReadConnection::new("server-2".to_owned());
        for (status, seq) in [("idle", 0), ("unknown", 1), ("idle", 3)] {
            let rows = pass(
                json!([pane("a", status, seq, None, "s1")]),
                &mut restored,
                &mut saved,
                None,
            );
            assert!(!rows[0].unread, "{status} at {seq} stays read");
        }
        let rows = pass(
            json!([pane("a", "idle", 3, None, "s1")]),
            &mut restored,
            &mut saved,
            None,
        );
        assert!(rows[0].completed, "the completion is carried over");
        assert_eq!(rows[0].group, "seen");
        assert_eq!(saved.records["a"].server.as_deref(), Some("server-2"));
        let before = (saved.records.clone(), saved.completions.clone());
        let mut again = projected(json!([pane("a", "idle", 3, None, "s1")]));
        assert!(!apply_completions(
            &mut again,
            &mut saved.completions,
            &restored
        ));
        assert!(apply_read_state(&mut again, &mut saved.records, &mut restored, None).is_empty());
        assert_eq!((saved.records.clone(), saved.completions.clone()), before);

        let rows = pass(
            json!([pane("a", "working", 4, None, "s1")]),
            &mut restored,
            &mut saved,
            None,
        );
        assert!(rows[0].unread, "work after the restore is news");
    }

    /// The restart of 2026-10-15: a restored Codex pane reports `done` again,
    /// which is the completion the operator already read, not a new one.
    #[test]
    fn a_restored_pane_reporting_done_again_stays_read() {
        let mut saved = Saved::default();
        pass(
            json!([pane("a", "done", 40, Some(40), "s1")]),
            &mut connection(),
            &mut saved,
            Some("a"),
        );

        let mut restored = ReadConnection::new("server-2".to_owned());
        pass(
            json!([pane("a", "idle", 0, None, "s1")]),
            &mut restored,
            &mut saved,
            None,
        );
        let rows = pass(
            json!([pane("a", "done", 5, Some(5), "s1")]),
            &mut restored,
            &mut saved,
            None,
        );
        assert!(!rows[0].unread);
        assert_eq!(rows[0].group, "seen");
    }

    /// A pane that was unread when the server went away stays unread on the
    /// restored server, however its sequence lands.
    #[test]
    fn a_pane_unread_before_a_restart_stays_unread() {
        let mut saved = Saved::default();
        let mut first = connection();
        pass(
            json!([pane("a", "done", 40, Some(40), "s1")]),
            &mut first,
            &mut saved,
            Some("a"),
        );
        let rows = pass(
            json!([pane("a", "working", 41, None, "s1")]),
            &mut first,
            &mut saved,
            None,
        );
        assert!(rows[0].unread);
        assert!(
            saved.records["a"].unread,
            "the verdict is saved with the record"
        );

        let mut restored = ReadConnection::new("server-2".to_owned());
        for (status, seq) in [("idle", 0), ("unknown", 1), ("idle", 40), ("idle", 41)] {
            let rows = pass(
                json!([pane("a", status, seq, None, "s1")]),
                &mut restored,
                &mut saved,
                None,
            );
            assert!(rows[0].unread, "{status} at {seq} stays unread");
        }
        let rows = pass(
            json!([pane("a", "idle", 41, None, "s1")]),
            &mut restored,
            &mut saved,
            Some("a"),
        );
        assert!(
            !rows[0].unread,
            "looking at it reads it on the restored server"
        );
    }

    /// What a restored pane shows that the operator had not read: another
    /// session, or a demand it did not have. Either stays unread until read.
    #[test]
    fn a_restored_pane_showing_something_new_is_unread() {
        let asking = |seq: u64| {
            let mut row = pane("a", "idle", seq, None, "s1");
            row["tokens"]["status_question_new"] = json!("?");
            row
        };
        for (restored_row, what) in [
            (pane("a", "idle", 3, None, "s2"), "another session"),
            (asking(3), "a new question"),
        ] {
            let mut saved = Saved::default();
            pass(
                json!([pane("a", "idle", 40, None, "s1")]),
                &mut connection(),
                &mut saved,
                Some("a"),
            );
            let mut restored = ReadConnection::new("server-2".to_owned());
            for _ in 0..2 {
                let rows = pass(
                    json!([restored_row.clone()]),
                    &mut restored,
                    &mut saved,
                    None,
                );
                assert!(rows[0].unread, "{what}");
            }
        }
    }

    /// A record saved before servers were named is judged by sequence as it
    /// always was, once, and then names this connection's server: one that
    /// still matches is read, one that does not is unread rather than read on
    /// a guess, and one the old reconciliation left at sequence 0 is carried.
    #[test]
    fn a_record_saved_before_servers_were_named_is_judged_by_sequence_once() {
        let legacy = |seq: u64| PaneReadRecord {
            state_change_seq: Some(seq),
            session_id: Some("s1".to_owned()),
            demand: "none".to_owned(),
            activity: "stopped".to_owned(),
            completed: true,
            descendant_signals: BTreeSet::new(),
            server: None,
            unread: false,
            checks_ended: None,
        };
        for (saved, unread, what) in [
            (40, false, "the same sequence is still read"),
            (38, true, "a sequence that moved is unread"),
            (0, false, "a placeholder sequence is carried"),
        ] {
            let mut saved_state = Saved::default();
            saved_state.records.insert("a".to_owned(), legacy(saved));
            let mut connection = connection();
            for _ in 0..2 {
                let rows = pass(
                    json!([pane("a", "done", 40, Some(40), "s1")]),
                    &mut connection,
                    &mut saved_state,
                    None,
                );
                assert_eq!(rows[0].unread, unread, "{what}");
            }
            let record = &saved_state.records["a"];
            assert_eq!(record.server.as_deref(), Some(SERVER), "{what}: named");
            assert_eq!(record.unread, unread, "{what}: the verdict is saved");
        }
    }

    /// Hide restarting while its Herdr server keeps running finds its own
    /// mark on the pane, so the server's sequence still compares: a turn
    /// that ran while Hide was away is news. Without the mark the record
    /// would be carried over and that turn absorbed.
    #[test]
    fn a_hide_restart_on_the_same_server_shows_work_done_while_away() {
        let marked = |status: &str, seq: u64| {
            let mut row = pane("a", status, seq, Some(seq), "s1");
            row["tokens"][crate::wire::SERVER_MARK_TOKEN] = json!(SERVER);
            row
        };
        let mut saved = Saved::default();
        pass(
            json!([marked("done", 40)]),
            &mut connection(),
            &mut saved,
            Some("a"),
        );

        let mut reconnected = ReadConnection::new("server-9".to_owned());
        let rows = pass(
            json!([marked("done", 40)]),
            &mut reconnected,
            &mut saved,
            None,
        );
        assert!(!rows[0].unread, "nothing ran while Hide was away");
        let rows = pass(
            json!([marked("done", 44)]),
            &mut reconnected,
            &mut saved,
            None,
        );
        assert!(rows[0].unread, "a turn ran while Hide was away");
    }

    /// PRD B5 across a restore: a child's completion the operator read on
    /// its parent does not light the parent while the restored server has
    /// neither the child's completion nor its lineage tokens back yet, and a
    /// question the child raised while Hide was away still does.
    #[test]
    fn a_childs_signal_through_a_restore_lights_its_parent_only_when_new() {
        let root = |seq: u64| pane("root", "idle", seq, None, "s-root");
        let child = |status: &str, seq: u64, completion: Option<u64>, linked: bool| {
            let mut row = pane("child", status, seq, completion, "s-child");
            if linked {
                row["spawned_from_pane_id"] = json!("root");
                row["declared_parent_session"] =
                    json!(crate::wire::session_digest("s-root").unwrap());
            }
            row
        };
        let mut saved = Saved::default();
        let rows = pass(
            json!([root(40), child("done", 12, Some(12), true)]),
            &mut connection(),
            &mut saved,
            Some("root"),
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.pane_id == "root")
                .unwrap()
                .descendant_signals
                .len(),
            1
        );

        let mut restored = ReadConnection::new("server-2".to_owned());
        let steps = [
            json!([root(0), child("idle", 0, None, false)]),
            json!([root(3), child("unknown", 1, None, false)]),
            json!([root(3), child("idle", 2, None, false)]),
            json!([root(3), child("idle", 2, None, true)]),
        ];
        for (step, rows) in steps.into_iter().enumerate() {
            let rows = pass(rows, &mut restored, &mut saved, None);
            let root = rows.iter().find(|row| row.pane_id == "root").unwrap();
            assert!(!root.unread, "step {step}: the parent stays read");
        }
        let asking = {
            let mut row = child("idle", 3, None, true);
            row["tokens"]["status_question_new"] = json!("?");
            row
        };
        let rows = pass(json!([root(3), asking]), &mut restored, &mut saved, None);
        let root = rows.iter().find(|row| row.pane_id == "root").unwrap();
        assert!(root.unread, "a new question from the child is news");
    }

    /// The ordering key falls back to Herdr's own sequence, zero padded to the
    /// activity token's width, when no label worker timestamp came.
    #[test]
    fn last_activity_falls_back_to_the_herdr_sequence() {
        let projection = project_agents(payload(json!([
            {"pane_id":"working","agent_status":"working","state_change_seq":1}
        ])));
        assert_eq!(projection.agents[0].last_activity, "00000000000000000001");
        assert_eq!(projection.agents[0].state_change_seq, Some(1));
    }
}
