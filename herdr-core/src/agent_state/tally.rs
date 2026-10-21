//! Scope totals, marks and representative ordering.
mod cleanup;
pub(crate) use cleanup::cleanup_facts;
mod close;
pub(crate) mod lineage;
mod relations;
pub mod scope;
pub mod tasks;
use super::axes::*;
use super::turn::*;
use crate::model::SidebarAgentSnapshot;
use crate::pet::PetSummary;
use crate::sidebar::sync_checkout_purposes;
use serde::Serialize;
use std::collections::BTreeMap;

/// The mark a row draws: whose move is next (docs/status-model.md, One
/// meaning across surfaces). It is one decision for the row's own mark and
/// for every count of marks, so a badge that stands for folded rows says what
/// opening them would show. The shell draws the shape and its colour; no
/// surface draws a mark of its own.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowMark {
    /// The operator's move: a question, an approval or a block, drawn as a
    /// `!` in a warning badge. Read and still unresolved, it stays dimmed.
    Turn,
    /// In progress: the agent itself works. A static three-quarter ring.
    Going,
    /// In progress while the agent waits on what is proven to wake it: a
    /// background task, a working child, an answer being made or a pull
    /// request's checks. The same ring dashed, so a surface with no second
    /// line still tells it from work (status-marks D-25); every count and
    /// precedence takes it as in progress, like `Going`.
    Waiting,
    /// To read, and the label read the turn as finished: a green dot.
    Done,
    /// To read, with no finished verdict: work left, a wait nothing proves,
    /// or a stop nobody judged. A plain dot.
    Unread,
    /// Nothing to do: the agent rests, ready for new work. A faint grey
    /// hollow ring, smaller and thinner than the in-progress one.
    #[default]
    Quiet,
    /// Hide cannot say where the agent stands: no demand and no activity it
    /// can read. No mark, its slot kept, and the row drawn dimmed whole, so
    /// it never reads as an agent at rest.
    Unknown,
    /// Hide put the agent to sleep; opening it wakes it. A grey moon, the
    /// one shape outside the four, counted on no badge (status-marks D-21).
    Asleep,
}

impl RowMark {
    /// The row's mark: an agent asleep draws the moon unless the operator's
    /// turn is on it, which outranks everything (D-10, D-21).
    pub(crate) fn of(agent: &SidebarAgentSnapshot) -> Self {
        match Self::awake(agent) {
            Self::Turn => Self::Turn,
            _ if agent.sleep.is_some() => Self::Asleep,
            mark => mark,
        }
    }

    /// The mark the row would draw awake, which a sleeping row keeps for its
    /// place in a tree and its class in a close list.
    pub(crate) fn awake(agent: &SidebarAgentSnapshot) -> Self {
        let ownership = ownership_of(agent);
        let (demand, activity, _) = axes_of(agent);
        // The group already holds every call on the operator: an own unread
        // demand, a blocked prompt, a raise from below, an escalated child
        // and a home's turn.
        if agent.group == "needs_you" || agent.escalation.is_some() {
            return Self::Turn;
        }
        // A demand the operator has read stays theirs until it is answered;
        // a delegated child's is its parent's (B8, B24).
        if demand != AgentDemand::None && !matches!(ownership, Ownership::Delegated) {
            return Self::Turn;
        }
        if activity == AgentActivity::Working {
            return Self::Going;
        }
        if agent.wait.is_some() {
            return Self::Waiting;
        }
        if agent.group == "done" {
            return if finished_verdict(agent) {
                Self::Done
            } else {
                Self::Unread
            };
        }
        if demand == AgentDemand::None && activity == AgentActivity::Unknown {
            return Self::Unknown;
        }
        Self::Quiet
    }

    pub(crate) fn count_into(self, counts: &mut crate::model::MarkCountsSnapshot) {
        match self {
            Self::Turn => counts.turn += 1,
            Self::Going | Self::Waiting => counts.going += 1,
            Self::Done => counts.done += 1,
            Self::Unread => counts.unread += 1,
            Self::Quiet | Self::Unknown | Self::Asleep => {}
        }
    }
}

/// Where a row stands when one row has to speak for several.
///
/// Group order first, then the worst demand inside it, with an unknown
/// activity ahead of an ordinary idle so missing information is not hidden by
/// a quiet sibling. The Workspace summary chip and the pane header's parent
/// badge both read it, so the two cannot disagree about which child a badge
/// is describing (docs/status-model.md, Workspace aggregation).
pub fn representative_rank(agent: &SidebarAgentSnapshot) -> (u8, u8) {
    rank_within(agent, ownership_of(agent))
}

/// Where a child stands among its siblings, as the pane that spawned them
/// sees it.
///
/// Delegation moves a child's demand off the *operator's* attention groups,
/// not out of existence. Its parent is exactly who is supposed to answer it,
/// so the parent's badge ranks its children as if it owned them; ranking them
/// the way the sidebar does would flatten every child to Working or Seen and
/// leave the badge naming a quiet sibling over the one that failed (PRD B15,
/// B16, D-35, D-38).
pub(crate) fn child_representative_rank(agent: &SidebarAgentSnapshot) -> (u8, u8) {
    rank_within(agent, Ownership::Operator)
}

fn rank_within(agent: &SidebarAgentSnapshot, ownership: Ownership) -> (u8, u8) {
    let (demand, activity, unread) = axes_of(agent);
    let demand_rank = match demand {
        AgentDemand::Error => 0,
        AgentDemand::Approval => 1,
        AgentDemand::Question => 2,
        AgentDemand::None if agent.activity == "unknown" => 3,
        AgentDemand::None => 4,
    };
    let group = agent_group_for(
        demand,
        activity,
        agent.completed,
        unread,
        agent.blocked,
        ownership,
        agent.wait.is_some(),
    );
    (group.rank(), demand_rank)
}

/// Aggregate physical pane ownership, never the visual lineage tree or raised rows.
/// Canonical order breaks ties after group and demand priority.
pub fn sync_checkout_agent_summaries(
    workspaces: &mut [crate::model::WorkspaceSnapshot],
    agents: &[SidebarAgentSnapshot],
) -> bool {
    let owners = workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .enumerate()
        .flat_map(|(index, checkout)| {
            checkout
                .tabs
                .iter()
                .flat_map(|tab| &tab.panes)
                .map(move |pane| (pane.id.as_str(), index))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let count = workspaces
        .iter()
        .map(|workspace| workspace.checkouts.len())
        .sum();
    let mut summaries = vec![crate::model::CheckoutAgentSummary::default(); count];
    let mut ranks = vec![None; count];
    let mut counted = std::collections::HashSet::new();
    for agent in agents {
        let Some(&index) = owners.get(agent.pane_id.as_str()) else {
            continue;
        };
        // A row its home owns is no part of the checkout's count unless it
        // calls the operator out (PRD hide-ai-work-home B2).
        if !listed_for_operator(agent) || !counted.insert(agent.pane_id.as_str()) {
            continue;
        }
        let summary = &mut summaries[index];
        RowMark::of(agent).count_into(&mut summary.marks);
        let group = group_of(agent);
        match group {
            AgentGroup::NeedsYou => summary.needs_you += 1,
            AgentGroup::Done => summary.done += 1,
            AgentGroup::Working => summary.working += 1,
            AgentGroup::Seen => {
                summary.seen += 1;
                if agent.activity == "unknown" && agent.demand == "none" {
                    summary.unknown += 1;
                }
            }
        }
        let rank = representative_rank(agent);
        if ranks[index].is_none_or(|best| rank < best) {
            ranks[index] = Some(rank);
            summary.representative_pane_id = Some(agent.pane_id.clone());
        }
    }
    let mut changed = false;
    for (checkout, summary) in workspaces
        .iter_mut()
        .flat_map(|workspace| &mut workspace.checkouts)
        .zip(summaries)
    {
        if checkout.agent_summary != summary {
            checkout.agent_summary = summary;
            changed = true;
        }
    }
    // The removal confirmation's counts ride the same pass, so the dialog
    // names the panes the close will send and the agents still working in
    // them as of the same projection (D-10). "Running" is the deletion
    // gate's definition: the agent's activity axis says working. Every local
    // row offers `Remove project…`, a row Herdr shows without a registration
    // too (PRD sidebar-context-menus D-14), so every local row carries the
    // gate; a device's rows get theirs where its session is derived
    // (`device_catalog::apply_registrations`), because a remote tree arrives
    // freshly projected on every sync and a count written into it here
    // would read as a change on every tick.
    changed |= sync_workspace_removals(workspaces, agents, true);
    changed |= sync_checkout_purposes(workspaces, agents);
    changed
}

/// Buckets the projected agent list.
///
/// While the herdr connection is down the last valid agent list is retained
/// (so the pet does not blink to empty) but every retained agent counts as
/// disconnected: a stale yellow "act now" badge for a server that is no
/// longer answering is exactly the failure this prevents.
pub fn summarize(agents: &[SidebarAgentSnapshot], connected: bool) -> PetSummary {
    let mut summary = PetSummary::default();
    for agent in agents {
        if !connected {
            summary.disconnected += 1;
            continue;
        }
        // The groups are decided once, in the projection. The pet counts them
        // through the projection's own enum rather than reading tokens, axes,
        // or group names a second time.
        if !listed_for_operator(agent) {
            continue;
        }
        match crate::agent_state::group_of(agent) {
            AgentGroup::NeedsYou => {
                summary.needs_you += 1;
                if crate::agent_state::demand_of(agent) == AgentDemand::Error {
                    summary.error += 1;
                }
            }
            AgentGroup::Done => summary.done += 1,
            AgentGroup::Working => summary.working += 1,
            AgentGroup::Seen => summary.seen += 1,
        }
    }
    summary
}

/// The in-process subagents Hide's hook reports as working, summed over the
/// listed agents. A count is only meaningful while the server is answering,
/// and only a pane that still holds an agent is counted, so a token left on
/// a pane whose agent exited does not linger in the badge.
pub fn subagents_active(
    agents: &[SidebarAgentSnapshot],
    hook_tokens: &BTreeMap<String, crate::agent_hooks::PaneHookTokens>,
    connected: bool,
) -> u32 {
    if !connected {
        return 0;
    }
    agents
        .iter()
        .filter_map(|agent| hook_tokens.get(&agent.pane_id))
        .filter_map(|tokens| tokens.working)
        .fold(0, u32::saturating_add)
}

/// Whether this agent is one the operator still has to act on.
///
/// Read through the projection's own enum, not by matching the group name a
/// second time: a name compared here is a copy of a vocabulary that lives in
/// one place.
pub fn is_unseen(agent: &SidebarAgentSnapshot) -> bool {
    crate::agent_state::group_of(agent) == AgentGroup::NeedsYou
}

/// The unseen panes in click order: the pane whose unseen state was observed
/// first, then snapshot order.
///
/// `observed` holds the first time each pane was seen unseen. It lives in
/// memory only (D-21), so after a restart every pane carries the same first
/// observation and the snapshot's own order decides - which is the approved
/// fallback, not a defect.
pub fn attention_order(
    agents: &[SidebarAgentSnapshot],
    observed: &BTreeMap<String, u64>,
) -> Vec<String> {
    let mut unseen = agents
        .iter()
        .enumerate()
        .filter(|(_, agent)| is_unseen(agent))
        .map(|(index, agent)| {
            (
                observed.get(&agent.pane_id).copied().unwrap_or(u64::MAX),
                index,
                agent.pane_id.clone(),
            )
        })
        .collect::<Vec<_>>();
    unseen.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    unseen.into_iter().map(|(_, _, pane_id)| pane_id).collect()
}

/// Records the first moment each currently-unseen pane became unseen and
/// forgets panes that are no longer unseen. Running it twice with the same
/// agent list leaves the map unchanged.
pub fn observe_unseen(
    observed: &mut BTreeMap<String, u64>,
    agents: &[SidebarAgentSnapshot],
    now_unix_ms: u64,
) {
    let unseen = agents
        .iter()
        .filter(|agent| is_unseen(agent))
        .map(|agent| agent.pane_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    observed.retain(|pane_id, _| unseen.contains(pane_id.as_str()));
    for pane_id in unseen {
        observed.entry(pane_id.to_owned()).or_insert(now_unix_ms);
    }
}

/// The phone-safe projection, built only from accepted core snapshot values.
pub mod phone {
    use std::collections::HashMap;

    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    pub use crate::agent_sleep::{DormantPhase, SleepId};

    /// The groups in the order the phone draws them.
    pub const GROUP_ORDER: [&str; 5] = ["needs_you", "done", "revisit", "working", "seen"];

    #[derive(Clone, Debug, Eq, PartialEq, Serialize)]
    pub struct Line {
        pub text: String,
        /// The desktop row's line mode (`RowLine::mode`), which the phone
        /// words the same way.
        pub mode: String,
        /// `warning` (a request or failed checks), `news` or `subtle`.
        pub tone: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub pull_request: Option<u64>,
    }

    /// What a sleeping agent's pane says about it (`AgentSleepSnapshot`): the
    /// phone words it as the desktop does and refuses input while it is set.
    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    pub struct PhoneSleep {
        /// `sleeping`, `waking` or `failed`.
        pub state: String,
        pub reason: Option<String>,
        pub since_unix_ms: u64,
        pub progress: Option<String>,
    }

    #[derive(Clone, Debug, Eq, PartialEq, Serialize)]
    pub struct PhoneAgent {
        pub device_id: String,
        /// The core's pane id: a Herdr pane id on this Mac, `remote:<device>:pane:<id>` on a device.
        pub pane_id: String,
        /// The pane id of this agent's lineage root on the same device; itself for a root.
        pub root_pane_id: String,
        pub group: String,
        /// The row's mark (`RowMark`): `turn`, `going`, `waiting`, `done`, `unread`, `quiet` or `asleep`.
        pub mark: String,
        /// What an in-progress row waits on (`AgentWait`), which its line
        /// words when it has no sentence.
        pub wait: Option<String>,
        /// How many of its descendants work, for a row waiting on them.
        pub descendants_working: u64,
        /// The background tasks a finished turn left running, which its line
        /// ends with (status-marks D-19); 0 otherwise.
        #[serde(skip_serializing_if = "is_zero")]
        pub background_left: u64,
        pub agent_kind: String,
        pub title: String,
        /// `project · branch`, or the project alone for a plain folder.
        pub place: Option<String>,
        /// The SSH device's name; `None` on this Mac.
        pub device_label: Option<String>,
        /// When the core last saw the agent change state; the phone counts the
        /// elapsed time from it, so time passing sends nothing.
        pub changed_at_unix_ms: Option<u64>,
        pub line: Option<Line>,
        /// The core's chosen sentence, including a raised root's detail that
        /// the compact phone row may not draw. Used only by notice transport.
        #[serde(skip)]
        pub detail: Option<String>,
        pub status_code: String,
        pub demand: String,
        /// Present while the agent's process is ended and its pane holds a
        /// bare shell; nothing may be typed into the pane then.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub sleep: Option<PhoneSleep>,
        /// Whether the compact title is emphasized, independent of the detail view.
        pub emphasized: bool,
        /// Whether opening the phone keeps this root notification. A read root
        /// question deliberately holds it even when the server clears its push.
        pub holds_notification: bool,
        /// The delivery notice is already responsible for this transition: a
        /// root in Needs You only for raised causes that notice announces.
        pub human_notice: bool,
    }

    impl PhoneAgent {
        pub fn key(&self) -> AgentKey {
            AgentKey {
                device_id: self.device_id.clone(),
                pane_id: self.pane_id.clone(),
            }
        }

        pub fn root_key(&self) -> AgentKey {
            AgentKey {
                device_id: self.device_id.clone(),
                pane_id: self.root_pane_id.clone(),
            }
        }
    }

    #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
    pub struct AgentKey {
        pub device_id: String,
        pub pane_id: String,
    }

    impl AgentKey {
        /// The notification tag: one notification per root agent, replaced in place.
        pub fn tag(&self) -> String {
            format!("{}|{}", self.device_id, self.pane_id)
        }

        /// The pane id Herdr knows on the device that owns it.
        pub fn herdr_pane_id(&self) -> &str {
            let prefix = format!("remote:{}:pane:", self.device_id);
            self.pane_id
                .strip_prefix(prefix.as_str())
                .unwrap_or(&self.pane_id)
        }
    }

    /// An archived conversation has an intent identity, never a pane address.
    /// Deserialize only the public subset of the core-owned row; serialization
    /// cannot disclose the saved cwd, native reference, lineage or close key.
    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    pub struct PhoneSleepingSession {
        pub sleep_id: SleepId,
        #[serde(rename(deserialize = "node_id"))]
        pub device_id: String,
        #[serde(rename(deserialize = "kind"))]
        pub agent_kind: String,
        #[serde(rename(deserialize = "identity_label"))]
        pub title: String,
        pub group: String,
        pub phase: DormantPhase,
        pub since_unix_ms: u64,
        pub wake_available: bool,
        pub checking: bool,
        #[serde(rename(deserialize = "reason"), deserialize_with = "has_sleep_reason")]
        pub attention: bool,
    }

    fn has_sleep_reason<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<bool, D::Error> {
        Option::<String>::deserialize(deserializer).map(|reason| reason.is_some())
    }

    #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
    pub struct Group {
        pub group: String,
        pub count: usize,
        pub agents: Vec<PhoneAgent>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        pub sleeping_sessions: Vec<PhoneSleepingSession>,
    }

    #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
    pub struct Projection {
        pub groups: Vec<Group>,
        /// The core's explicit interface language (`ui_state.interface_language`)
        /// as stored, or `None` when the operator follows the system. The phone
        /// resolves the value; it is part of the projection so a change republishes
        /// the frame like any other change.
        pub interface_language: Option<String>,
        /// The core's color theme (`ui_state.color_theme`) as published, so
        /// the phone draws the operator's palette (hide-color-theme D-12);
        /// the phone reads anything but `neutral` as Hide.
        pub color_theme: Option<String>,
    }

    impl Projection {
        pub fn agents(&self) -> impl Iterator<Item = &PhoneAgent> {
            self.groups.iter().flat_map(|group| group.agents.iter())
        }

        pub fn find(&self, key: &AgentKey) -> Option<&PhoneAgent> {
            self.agents()
                .find(|agent| agent.device_id == key.device_id && agent.pane_id == key.pane_id)
        }

        pub fn sleeping(&self, id: &SleepId) -> Option<&PhoneSleepingSession> {
            self.groups
                .iter()
                .flat_map(|group| &group.sleeping_sessions)
                .find(|session| &session.sleep_id == id)
        }
    }

    fn is_zero(value: &u64) -> bool {
        *value == 0
    }

    fn str_of<'a>(value: &'a Value, field: &str) -> &'a str {
        value.get(field).and_then(Value::as_str).unwrap_or("")
    }

    /// The desktop sidebar's second-line rule (`sidebarLine`): a sleeping
    /// row has none unless the operator's turn outranks its moon (D-10), a
    /// waking, failed or unknown one says its state (status-marks B16, B31),
    /// else the core's line, except a quiet sentence, which no row draws at
    /// rest.
    fn line(agent: &Value) -> Option<Line> {
        match str_of(agent, "status_code") {
            "sleeping" if str_of(agent, "mark") != "turn" => return None,
            "waking" | "sleep_failed" | "unknown" => {
                return Some(Line {
                    text: String::new(),
                    mode: "kept".to_owned(),
                    tone: "subtle",
                    pull_request: None,
                });
            }
            _ => {}
        }
        let line = agent.pointer("/state/line")?;
        let mode = str_of(line, "mode");
        if mode.is_empty() || mode == "quiet" {
            return None;
        }
        let tone = match (mode, str_of(line.get("tone")?, "kind")) {
            ("request" | "checks_failed", _) => "warning",
            (_, "news") => "news",
            _ => "subtle",
        };
        Some(Line {
            text: str_of(line, "text").trim().to_owned(),
            mode: mode.to_owned(),
            tone,
            pull_request: line.get("pull_request").and_then(Value::as_u64),
        })
    }

    /// Each pane's `project · branch`, from one device's workspaces, the way
    /// the desktop's `agentPlaces` walks them.
    fn places(workspaces: Option<&Value>) -> HashMap<String, String> {
        let mut places = HashMap::new();
        for workspace in workspaces.and_then(Value::as_array).into_iter().flatten() {
            let label = str_of(workspace, "label");
            let checkouts = workspace
                .get("checkouts")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let folder = workspace.get("is_git").and_then(Value::as_bool) != Some(true)
                && checkouts.len() == 1;
            for checkout in checkouts {
                let place = if folder {
                    label.to_owned()
                } else {
                    let branch = checkout
                        .get("branch")
                        .and_then(Value::as_str)
                        .filter(|branch| !branch.is_empty())
                        .unwrap_or_else(|| str_of(checkout, "label"));
                    format!("{label} · {branch}")
                };
                for tab in checkout
                    .get("tabs")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    for pane in tab
                        .get("panes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(id) = pane.get("id").and_then(Value::as_str) {
                            places.entry(id.to_owned()).or_insert_with(|| place.clone());
                        }
                    }
                }
            }
        }
        places
    }

    fn roots(agents: &[Value]) -> HashMap<String, String> {
        let parents: HashMap<&str, &str> = agents
            .iter()
            .filter_map(|agent| {
                let pane = agent.get("pane_id").and_then(Value::as_str)?;
                let parent = agent
                    .get("lineage_parent_pane_id")
                    .and_then(Value::as_str)
                    .filter(|parent| !parent.is_empty())?;
                Some((pane, parent))
            })
            .collect();
        agents
            .iter()
            .filter_map(|agent| agent.get("pane_id").and_then(Value::as_str))
            .map(|pane| {
                let mut root = pane;
                // A lineage longer than the rows is a cycle; stop where it began.
                for _ in 0..agents.len() {
                    match parents.get(root) {
                        Some(parent) if *parent != pane => root = parent,
                        _ => break,
                    }
                }
                (pane.to_owned(), root.to_owned())
            })
            .collect()
    }

    fn rows(
        agents: &[Value],
        device_id: &str,
        device_label: Option<&str>,
        places: &HashMap<String, String>,
        out: &mut Vec<PhoneAgent>,
    ) {
        let roots = roots(agents);
        for agent in agents {
            // A starred task still waits in 다시 볼 것 after it resolved itself.
            if agent.get("resolved").is_some_and(|value| !value.is_null())
                && agent.pointer("/revisit/starred").and_then(Value::as_bool) != Some(true)
            {
                continue;
            }
            let pane_id = str_of(agent, "pane_id");
            if pane_id.is_empty() {
                continue;
            }
            // `listed_for_operator` on the wire form: a row its home owns is
            // on the phone only while it is in Needs You (PRD
            // hide-ai-work-home B4); its detail stays reachable from there.
            if agent
                .pointer("/hide_ai/home")
                .is_some_and(|home| !home.is_null())
                && !agent
                    .get("delegated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                && str_of(agent, "group") != "needs_you"
            {
                continue;
            }
            let title = agent
                .get("identity_label")
                .and_then(Value::as_str)
                .filter(|title| !title.trim().is_empty())
                .unwrap_or(str_of(agent, "agent_kind"));
            out.push(PhoneAgent {
                device_id: device_id.to_owned(),
                pane_id: pane_id.to_owned(),
                root_pane_id: roots
                    .get(pane_id)
                    .cloned()
                    .unwrap_or_else(|| pane_id.to_owned()),
                // A quiet row the operator starred or read and left
                // unanswered stands in 다시 볼 것 (PRD revisit-group D-10).
                group: match str_of(agent, "group") {
                    "seen" if agent.get("revisit").is_some_and(|value| !value.is_null()) => {
                        "revisit".to_owned()
                    }
                    "working"
                        if str_of(agent, "activity") != "working"
                            && agent.get("revisit").is_some_and(|value| !value.is_null()) =>
                    {
                        "revisit".to_owned()
                    }
                    group => group.to_owned(),
                },
                mark: str_of(agent, "mark").to_owned(),
                wait: agent.get("wait").and_then(Value::as_str).map(str::to_owned),
                descendants_working: agent
                    .pointer("/descendant_counts/working")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                background_left: agent
                    .get("background_left")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                emphasized: matches!(str_of(agent, "group"), "needs_you" | "done"),
                holds_notification: matches!(str_of(agent, "group"), "needs_you" | "done")
                    || matches!(str_of(agent, "demand"), "question" | "approval" | "error"),
                human_notice: str_of(agent, "demand") == "none"
                    && !agent
                        .get("blocked")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    && agent
                        .get("raised")
                        .and_then(Value::as_array)
                        .is_some_and(|raised| {
                            !raised.is_empty()
                                && raised.iter().all(|ask| {
                                    ask.get("human_notice").and_then(Value::as_bool) == Some(true)
                                })
                        }),
                agent_kind: str_of(agent, "agent_kind").to_owned(),
                title: title.to_owned(),
                place: places.get(pane_id).cloned(),
                device_label: device_label.map(str::to_owned),
                changed_at_unix_ms: agent.get("changed_at_unix_ms").and_then(Value::as_u64),
                line: line(agent),
                detail: agent
                    .get("detail")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned),
                status_code: str_of(agent, "status_code").to_owned(),
                demand: agent
                    .get("demand")
                    .and_then(Value::as_str)
                    .unwrap_or("none")
                    .to_owned(),
                sleep: agent
                    .get("sleep")
                    .and_then(|sleep| serde_json::from_value(sleep.clone()).ok()),
            });
        }
    }

    /// The phone projection of a merged `rest` section; `node` is the core's own
    /// machine, whose agents the navigator lists.
    pub fn project(rest: &Value, node: &str) -> Projection {
        let mut agents = Vec::new();
        let local = rest
            .pointer("/navigator/agents")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let local_places = places(rest.pointer("/navigator/workspaces"));
        rows(local, node, None, &local_places, &mut agents);
        let devices = rest
            .pointer("/navigator/devices")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for status in rest
            .pointer("/status/remote")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if str_of(status, "state") != "connected" {
                continue;
            }
            let target = str_of(status, "target_id");
            let label = devices
                .iter()
                .find(|device| str_of(device, "id") == target)
                .map(|device| str_of(device, "label"))
                .filter(|label| !label.is_empty())
                .unwrap_or(target);
            let session = status.get("session");
            let remote_agents = session
                .and_then(|session| session.get("agents"))
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let remote_places = places(session.and_then(|session| session.get("workspaces")));
            rows(
                remote_agents,
                target,
                Some(label),
                &remote_places,
                &mut agents,
            );
        }
        let mut groups: Vec<Group> = GROUP_ORDER
            .iter()
            .map(|group| Group {
                group: (*group).to_owned(),
                count: 0,
                agents: Vec::new(),
                sleeping_sessions: Vec::new(),
            })
            .collect();
        for agent in agents {
            match groups.iter_mut().find(|group| group.group == agent.group) {
                Some(group) => group.agents.push(agent),
                None => groups.push(Group {
                    group: agent.group.clone(),
                    count: 0,
                    agents: vec![agent],
                    sleeping_sessions: Vec::new(),
                }),
            }
        }
        for value in rest
            .pointer("/navigator/sleeping_sessions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let row = match serde_json::from_value::<PhoneSleepingSession>(value.clone()) {
                Ok(row) if row.device_id == node => row,
                Ok(_) => continue,
                Err(_) => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "mobile_phone", "kind": "sleep.projection_invalid",
                    }));
                    continue;
                }
            };
            if let Some(group) = groups.iter_mut().find(|group| group.group == row.group) {
                group.sleeping_sessions.push(row);
            } else {
                crate::diagnostic!(serde_json::json!({
                    "component": "mobile_phone", "kind": "sleep.group_invalid", "sleep_id": row.sleep_id,
                }));
            }
        }
        for group in &mut groups {
            group.count = group.agents.len() + group.sleeping_sessions.len();
        }
        groups.retain(|group| group.count > 0);
        let interface_language = rest
            .pointer("/ui_state/interface_language")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let color_theme = rest
            .pointer("/ui_state/color_theme")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Projection {
            groups,
            interface_language,
            color_theme,
        }
    }
}

#[cfg(test)]
mod scope_tests;

impl crate::model::DescendantCountsSnapshot {
    /// Whether any drawn count is above zero, so a row with descendants that
    /// are all merely ready wears no badge rather than an empty one.
    pub fn any_drawn(&self) -> bool {
        self.error + self.approval + self.question + self.working + self.done > 0
    }
}

/// Existing inactive-checkout exception, independent of Git and focus policy.
pub(crate) fn checkout_has_active_agents(summary: &crate::model::CheckoutAgentSummary) -> bool {
    summary.working > 0 || summary.needs_you > 0
}

/// Removal confirmation counts have the same activity rule on both devices.
pub(crate) fn sync_workspace_removals(
    workspaces: &mut [crate::model::WorkspaceSnapshot],
    agents: &[SidebarAgentSnapshot],
    local_only: bool,
) -> bool {
    let mut changed = false;
    let running = agents
        .iter()
        .filter(|agent| agent.activity == AgentActivity::Working.name())
        .map(|agent| agent.pane_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    for workspace in workspaces
        .iter_mut()
        .filter(|workspace| !local_only || workspace.remote_target_id.is_none())
    {
        let panes = workspace
            .checkouts
            .iter()
            .flat_map(|checkout| &checkout.tabs)
            .flat_map(|tab| &tab.panes);
        let mut removal = crate::model::WorkspaceRemovalGateSnapshot::default();
        for pane in panes {
            removal.pane_count += 1;
            if running.contains(pane.id.as_str()) {
                removal.running_agent_count += 1;
            }
        }
        if workspace.removal != removal {
            workspace.removal = removal;
            changed = true;
        }
    }
    changed
}

pub(crate) fn device_catalog_count(
    workspaces: &[crate::model::WorkspaceSnapshot],
    agents: &[SidebarAgentSnapshot],
    device: &str,
) -> u32 {
    agents
        .iter()
        .filter(|agent| {
            workspaces
                .iter()
                .filter(|w| w.device_id == device)
                .flat_map(|w| &w.checkouts)
                .flat_map(|c| &c.tabs)
                .flat_map(|t| &t.panes)
                .any(|p| p.id == agent.pane_id)
        })
        .count() as u32
}

/// The rail retains the remote session's last row count while disconnected.
pub(crate) fn remote_session_count(session: Option<&crate::model::RemoteSessionSnapshot>) -> u32 {
    session.map_or(0, |s| s.agents.len()).min(u32::MAX as usize) as u32
}
