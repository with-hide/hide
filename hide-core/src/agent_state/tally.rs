//! Scope totals, marks and representative ordering.
mod cleanup;
pub(crate) use cleanup::cleanup_facts;
mod close;
pub(crate) mod lineage;
pub mod project_status;
mod relations;
pub mod scope;
pub mod tasks;
use super::axes::*;
use super::turn::*;
use crate::model::SidebarAgentSnapshot;
use crate::sidebar::sync_checkout_purposes;
use serde::Serialize;

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
    /// The operator's move is to look: the agent opened a View they have not
    /// seen and nothing else is asked of them (PRD unseen-agent-views D-05,
    /// D-15). The turn's badge with an eye, counted as the turn (B12).
    Look,
}

impl RowMark {
    /// The row's mark: an agent asleep draws the moon unless the operator's
    /// turn is on it, which outranks everything (D-10, D-21).
    pub(crate) fn of(agent: &SidebarAgentSnapshot) -> Self {
        match Self::awake(agent) {
            Self::Turn => Self::Turn,
            Self::Look => Self::Look,
            _ if agent.sleep.is_some() => Self::Asleep,
            mark => mark,
        }
    }

    /// The mark the row would draw awake, which a sleeping row keeps for its
    /// place in a tree and its class in a close list.
    pub(crate) fn awake(agent: &SidebarAgentSnapshot) -> Self {
        let ownership = ownership_of(agent);
        let (demand, activity, _) = axes_of(agent);
        // A tool's agent calls the operator only for a person's turn, its
        // own or raised to it, and a View it opened for them; its question,
        // its block and every other raise are the tool's to answer, so they
        // neither outrank the View nor take the mark (PRD
        // hide-external-control B28, B29).
        let tool = ownership == Ownership::Tool;
        if tool && agent.human_turn.is_some() {
            return Self::Turn;
        }
        // A View to look at is the turn only when nothing else is asked:
        // a block, a question, an approval or a raise outranks it and takes
        // the mark (D-18).
        if agent.look.is_some()
            && (tool
                || (demand == AgentDemand::None
                    && !agent.blocked
                    && agent.raised.is_empty()
                    && agent.escalation.is_none()
                    && agent.declared_human_turn.is_none()))
        {
            return Self::Look;
        }
        // The group already holds every call on the operator: an own unread
        // demand, a blocked prompt, a raise from below, an escalated child
        // and a home's turn.
        if !tool && (agent.group == "needs_you" || agent.escalation.is_some()) {
            return Self::Turn;
        }
        // A demand the operator has read stays theirs until it is answered;
        // a delegated child's is its parent's (B8, B24), and a tool's agent's
        // is the tool's (PRD hide-external-control B28), so it waits on that
        // answer, in progress rather than at rest.
        if demand != AgentDemand::None {
            return if matches!(ownership, Ownership::Delegated | Ownership::Tool) {
                Self::Waiting
            } else {
                Self::Turn
            };
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
            Self::Turn | Self::Look => counts.turn += 1,
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
        // Only an operator row's View is the operator's to look at; a
        // delegated child's ranks among its siblings by its own axes (D-10).
        agent.look.is_some(),
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
        // A resolved row rests in every list, so the badge does not count
        // its mark either.
        if agent.resolved.is_none() {
            RowMark::of(agent).count_into(&mut summary.marks);
        }
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
    // row offers `Remove project…`, a row the engine shows without a registration
    // too (PRD sidebar-context-menus D-14), so every local row carries the
    // gate; a device's rows get theirs where its session is derived
    // (`device_catalog::apply_registrations`), because a remote tree arrives
    // freshly projected on every sync and a count written into it here
    // would read as a change on every tick.
    changed |= sync_workspace_removals(workspaces, agents, true);
    changed |= sync_checkout_purposes(workspaces, agents);
    changed
}

/// Whether any agent the operator is shown is working or waiting on them,
/// read through the projection's own groups. A server that stopped answering
/// says nothing true about its agents, so it reports none.
pub fn any_agent_active(agents: &[SidebarAgentSnapshot], connected: bool) -> bool {
    connected
        && agents.iter().any(|agent| {
            listed_for_operator(agent)
                && matches!(
                    crate::agent_state::group_of(agent),
                    AgentGroup::NeedsYou | AgentGroup::Working
                )
        })
}

/// The phone-safe projection, built only from accepted core snapshot values.
pub mod phone {
    mod missions;
    mod prs;
    mod tree;

    pub use missions::{
        MAX_CLOSED, MAX_OPEN, PhoneAnswer, PhoneAttention, PhoneConductor, PhoneEscalation,
        PhoneMission, PhoneMissions, PhoneNode,
    };
    pub use prs::{
        MAX_FILES, MAX_PRS, MAX_SETTLED, PhoneCard, PhoneChangeRequest, PhoneFeedback, PhoneFile,
        PhoneMerge, PhonePr, PhoneProject, PhonePrs, PhoneReason, PhoneWrite,
    };

    use std::borrow::Cow;
    use std::collections::HashMap;

    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    pub use crate::agent_sleep::{DormantPhase, SleepId};
    use crate::model::AskSubject;

    /// The groups in the order the phone draws them.
    pub const GROUP_ORDER: [&str; 5] = ["needs_you", "done", "revisit", "working", "seen"];

    /// The most bytes one application message to a phone may hold: the
    /// relay's `noise::MAX_MESSAGE`, which refuses a longer one and closes the
    /// session. hided asserts the two equal at compile time, since this crate
    /// does not link the relay's.
    pub const FRAME_LIMIT: usize = 256 * 1024;
    /// The bytes a list (the `agents` groups, the `missions` items) may take
    /// in its frame: the limit less room for the frame's own fields, so a long
    /// list leaves rows out, counted in `omitted`, instead of the relay
    /// refusing the frame and the phone reconnecting into it again
    /// (engineering #15).
    pub const LIST_BUDGET: usize = FRAME_LIMIT - 4 * 1024;

    /// The bytes `value` takes as the JSON a frame carries.
    pub fn wire_len<T: Serialize + ?Sized>(value: &T) -> usize {
        struct Count(usize);
        impl std::io::Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut count = Count(0);
        serde_json::to_writer(&mut count, value).expect("a phone value has only string keys");
        count.0
    }

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
        /// A request the core asks for the agent, which the phone words in
        /// place of `text` (`RowLine::subject`).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub subject: Option<AskSubject>,
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
        /// The core's pane id: an engine pane id on this Mac, `remote:<device>:pane:<id>` on a device.
        pub pane_id: String,
        /// The pane id of this agent's lineage root on the same device; itself for a root.
        pub root_pane_id: String,
        /// Where the list puts the row: the desktop's own group, so a row with
        /// a View to look at stands in Needs You as it does there (PRD
        /// unseen-agent-views D-05; mobile-companion-ui B24).
        pub group: String,
        /// Where the row would stand with nothing to look at, which says
        /// whether it asks something of its own. Equal to `group` for every
        /// row without a View to look at.
        #[serde(skip)]
        pub notice_group: String,
        /// The newest View the operator has not looked at, for a row that has
        /// one: its notice says `볼 것 · <View>` (status-model.md, A View to look at).
        /// Used only by notice transport.
        #[serde(skip)]
        pub look: Option<String>,
        /// In `needs_you` for an ask the operator read and has not answered:
        /// listed there, drawn read, and never announced (needs-you-cleanup
        /// D-10).
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        pub read: bool,
        /// The operator's ★ to revisit the task (PRD revisit-group D-03),
        /// which the phone draws where the desktop row does; absent when the
        /// row carries none, so a phone that predates it reads nothing new.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        pub starred: bool,
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
        /// The descendants still working under a finished turn the operator
        /// has not read, which its line ends with; 0 otherwise.
        #[serde(skip_serializing_if = "is_zero")]
        pub children_left: u64,
        /// The subagents still running under a finished turn the operator
        /// has not read, which its line ends with; 0 otherwise.
        #[serde(skip_serializing_if = "is_zero")]
        pub subagents_left: u64,
        /// How many things its `wait` waits on, which its line counts; 0 for
        /// running checks and on a row that waits for nothing.
        #[serde(skip_serializing_if = "is_zero")]
        pub wait_count: u64,
        /// How many of its own subagents a children wait waits on too, which
        /// its line adds; 0 otherwise.
        #[serde(skip_serializing_if = "is_zero")]
        pub wait_subagents: u64,
        /// Whether the operator has not read the row since it last changed,
        /// which tells the notices a read from a verdict judged again.
        #[serde(skip)]
        pub unread: bool,
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
        /// What the core asks on the row for the agent, which a notice
        /// carries for the receiver to word. Used only by notice transport.
        #[serde(skip)]
        pub subject: Option<AskSubject>,
        /// The person's turn the row leads with (`SidebarAgentSnapshot::
        /// human_turn`, PRD ask-human), which the phone draws with Done.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub human_turn: Option<Value>,
        /// Every declaration on the row and the ones raised to it, so a new
        /// one is announced once even while the row already stands in Needs
        /// You (B5, B17), by declaration id with what each says. Used only by
        /// notice transport.
        #[serde(skip)]
        pub human_turns: std::collections::BTreeMap<String, AskSubject>,
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
        #[serde(skip)]
        pub notification_pending: bool,
        #[serde(skip)]
        pub notification_finalized: bool,
        /// Its live descendants by state, for the pills a parent's row wears
        /// (PRD mobile-companion-ui B2); absent for a row with none.
        #[serde(skip_serializing_if = "Descendants::is_none")]
        pub descendants: Descendants,
        /// The children listed under this row, the sidebar's two levels in
        /// the sidebar's order, at most `SIBLINGS_SHOWN` of them; the rest
        /// wait in `more_children` behind the row's "더 보기 N" (B4).
        #[serde(skip_serializing_if = "Vec::is_empty")]
        pub children: Vec<PhoneAgent>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        pub more_children: Vec<PhoneAgent>,
        /// Whether the row opens its children until the phone's operator
        /// chooses otherwise: a descendant waits for them (B3).
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        pub children_open: bool,
        /// Whether Hide's own delegation made the row, and the panes that
        /// hang under it in the sidebar's child order
        /// (`lineage_child_pane_ids`), so the tree hangs them the same way.
        #[serde(skip)]
        delegated: bool,
        #[serde(skip)]
        child_pane_ids: Vec<String>,
        /// Where the row stands among its siblings (`RowState::tree_rank`).
        #[serde(skip)]
        tree_rank: u8,
        /// The ordering key `sort_agents` orders by (`last_activity`).
        #[serde(skip)]
        activity: String,
    }

    /// What a parent's descendants are doing, counted as its pills draw them.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
    pub struct Descendants {
        /// A question, an approval or a failure waits on the operator.
        pub asking: u64,
        pub working: u64,
        pub done: u64,
    }

    impl Descendants {
        fn is_none(&self) -> bool {
            *self == Self::default()
        }
    }

    impl PhoneAgent {
        fn visit<'a>(&'a self, out: &mut Vec<&'a PhoneAgent>) {
            out.push(self);
            for child in self.children.iter().chain(&self.more_children) {
                child.visit(out);
            }
        }

        pub fn key(&self) -> AgentKey {
            AgentKey {
                device_id: self.device_id.clone(),
                pane_id: self.pane_id.clone(),
            }
        }

        /// This agent and every descendant hung under it.
        fn agents(&self) -> usize {
            1 + self
                .children
                .iter()
                .chain(&self.more_children)
                .map(Self::agents)
                .sum::<usize>()
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

        /// The pane id the engine knows on the device that owns it.
        pub fn engine_pane_id(&self) -> &str {
            crate::address::pane_of(&self.pane_id, &self.device_id).unwrap_or(&self.pane_id)
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

    /// One top row of a group: an agent with its tree, or a sleeping
    /// conversation. Both stand in one order and fold together.
    #[derive(Clone, Debug, Eq, PartialEq, Serialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum GroupRow {
        Agent(Box<PhoneAgent>),
        Sleeping(PhoneSleepingSession),
    }

    impl GroupRow {
        /// The ordering key the group sorts by: an agent's `last_activity`,
        /// a sleeping conversation's time written as thirteen digits.
        fn activity(&self) -> Cow<'_, str> {
            match self {
                Self::Agent(agent) => Cow::Borrowed(&agent.activity),
                Self::Sleeping(session) => Cow::Owned(format!("{:013}", session.since_unix_ms)),
            }
        }

        /// The agents the row stands for: an agent and its tree, or one
        /// sleeping conversation.
        fn agents(&self) -> usize {
            match self {
                Self::Agent(agent) => agent.agents(),
                Self::Sleeping(_) => 1,
            }
        }

        /// Whether the row is asleep, for the fold's "잠든 것 N 포함".
        fn asleep(&self) -> bool {
            match self {
                Self::Agent(agent) => agent.mark == "asleep",
                Self::Sleeping(_) => true,
            }
        }
    }

    /// One group of top rows. `count` is every top row of the group, the
    /// folded ones too (PRD mobile-companion-ui B5); `rows` are the first few
    /// in recency order and `more` the rest, behind the group's "더 보기 N".
    #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
    pub struct Group {
        pub group: String,
        pub count: usize,
        pub rows: Vec<GroupRow>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        pub more: Vec<GroupRow>,
        /// How many of `more` are asleep.
        #[serde(skip_serializing_if = "is_zero_usize")]
        pub more_asleep: usize,
    }

    /// A machine the phone's list draws: the Mac it is paired with and every
    /// device that Mac reaches (PRD mobile-companion-ui B8, B9).
    #[derive(Clone, Debug, Eq, PartialEq, Serialize)]
    pub struct PhoneDevice {
        pub device_id: String,
        /// The device's name; `None` for the paired Mac, which the phone names.
        pub label: Option<String>,
        /// The paired Mac itself.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        pub local: bool,
        pub connected: bool,
        /// Its top rows in 내 차례 and in 작업 중, which add up across
        /// devices to the groups' counts.
        pub needs_you: usize,
        pub working: usize,
    }

    #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
    pub struct Projection {
        pub groups: Vec<Group>,
        pub devices: Vec<PhoneDevice>,
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
        /// Every agent of the list, a parent before its children, shown or
        /// folded.
        pub fn agents(&self) -> impl Iterator<Item = &PhoneAgent> {
            let mut all = Vec::new();
            for row in self
                .groups
                .iter()
                .flat_map(|group| group.rows.iter().chain(&group.more))
            {
                if let GroupRow::Agent(agent) = row {
                    agent.visit(&mut all);
                }
            }
            all.into_iter()
        }

        pub fn find(&self, key: &AgentKey) -> Option<&PhoneAgent> {
            self.agents()
                .find(|agent| agent.device_id == key.device_id && agent.pane_id == key.pane_id)
        }

        pub fn sleeping(&self, id: &SleepId) -> Option<&PhoneSleepingSession> {
            self.groups
                .iter()
                .flat_map(|group| group.rows.iter().chain(&group.more))
                .find_map(|row| match row {
                    GroupRow::Sleeping(session) if &session.sleep_id == id => Some(session),
                    _ => None,
                })
        }
    }

    fn is_zero(value: &u64) -> bool {
        *value == 0
    }

    fn is_zero_usize(value: &usize) -> bool {
        *value == 0
    }

    /// The groups one `agents` frame carries and how many agents it left out.
    #[derive(Debug)]
    pub struct WireGroups<'a> {
        pub groups: Cow<'a, [Group]>,
        pub omitted: usize,
    }

    impl Projection {
        /// The groups as one `agents` frame carries them, within the frame's
        /// list budget beside its devices. The projection itself stays whole,
        /// for the notices and the lookups; only the frame's copy leaves rows
        /// out. A group's `count` stays the true total.
        pub fn wire(&self) -> WireGroups<'_> {
            let beside = wire_len(&self.devices)
                + wire_len(&self.interface_language)
                + wire_len(&self.color_theme);
            fit_groups(&self.groups, LIST_BUDGET.saturating_sub(beside))
        }
    }

    /// Keeps `groups` within `room` bytes of JSON. Every other group loses
    /// its rows before 내 차례 loses one, since a row there waits on the
    /// operator; inside each part, what the phone shows last goes first:
    /// folded top rows, then the children of the rows left, then shown top
    /// rows, each the oldest first and group by group from the end of the
    /// drawing order.
    fn fit_groups(groups: &[Group], room: usize) -> WireGroups<'_> {
        let total = wire_len(groups);
        if total <= room {
            return WireGroups {
                groups: Cow::Borrowed(groups),
                omitted: 0,
            };
        }
        let mut groups = groups.to_vec();
        let mut over = total - room;
        let mut omitted = 0;
        for asking in [false, true] {
            let part = groups
                .iter_mut()
                .rev()
                .filter(|group| (group.group == "needs_you") == asking);
            shed_groups(part, &mut over, &mut omitted);
        }
        for group in &mut groups {
            group.more_asleep = group.more.iter().filter(|row| row.asleep()).count();
        }
        WireGroups {
            groups: Cow::Owned(groups),
            omitted,
        }
    }

    /// The three passes of `fit_groups` over one part of the groups, given
    /// from the end of the drawing order.
    fn shed_groups<'a>(
        groups: impl Iterator<Item = &'a mut Group>,
        over: &mut usize,
        omitted: &mut usize,
    ) {
        let mut groups: Vec<&mut Group> = groups.collect();
        for group in &mut groups {
            shed(&mut group.more, over, omitted, GroupRow::agents);
        }
        for group in &mut groups {
            for row in group.rows.iter_mut().chain(&mut group.more).rev() {
                if let GroupRow::Agent(agent) = row {
                    shed(&mut agent.more_children, over, omitted, PhoneAgent::agents);
                    shed(&mut agent.children, over, omitted, PhoneAgent::agents);
                }
            }
        }
        for group in &mut groups {
            shed(&mut group.rows, over, omitted, GroupRow::agents);
        }
    }

    /// Drops the last items of `list` until `over` bytes are paid, counting
    /// each one's agents in `omitted`. An item takes its comma with it while
    /// others stay; the last one leaves an empty list, or none at all for a
    /// list that is not sent empty, so the count never falls short.
    fn shed<T: Serialize>(
        list: &mut Vec<T>,
        over: &mut usize,
        omitted: &mut usize,
        agents: fn(&T) -> usize,
    ) {
        while *over > 0 {
            let Some(item) = list.pop() else {
                return;
            };
            let comma = usize::from(!list.is_empty());
            *over = over.saturating_sub(wire_len(&item) + comma);
            *omitted += agents(&item);
        }
    }

    /// A line's `subject` or a row's `ask_subject`, as the core wrote it.
    fn subject_of(subject: Option<&Value>) -> Option<AskSubject> {
        AskSubject::deserialize(subject?).ok()
    }

    /// The person's turn the row's own agent declared, as the question it
    /// asks (`SidebarAgentSnapshot::declared_human_turn`).
    fn declared_subject(agent: &Value) -> Option<AskSubject> {
        let declared = agent.get("declared_human_turn")?;
        Some(AskSubject::HumanTurn {
            id: declared.get("id")?.as_str()?.to_owned(),
            kind: serde_json::from_value(declared.get("kind")?.clone()).ok()?,
            place: declared.get("place")?.as_str()?.to_owned(),
            device: declared
                .get("device")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
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
            "sleeping" if !matches!(str_of(agent, "mark"), "turn" | "look") => return None,
            "waking" | "sleep_failed" | "unknown" => {
                return Some(Line {
                    text: String::new(),
                    mode: "kept".to_owned(),
                    tone: "subtle",
                    pull_request: None,
                    subject: None,
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
            subject: subject_of(line.get("subject")),
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
            // The core's `listed_for_operator`, as the row publishes it: a row
            // Hide AI's home or an external tool keeps is on the phone only
            // while it is in Needs You (PRD hide-ai-work-home B4, PRD
            // hide-external-control B26); its detail stays reachable from there.
            if agent.get("unlisted").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let title = agent
                .get("identity_label")
                .and_then(Value::as_str)
                .filter(|title| !title.trim().is_empty())
                .unwrap_or(str_of(agent, "agent_kind"));
            let starred = agent.pointer("/revisit/starred").and_then(Value::as_bool) == Some(true);
            let read_ask = !starred
                && agent.pointer("/revisit/read_ask").and_then(Value::as_bool) == Some(true);
            // A resting row the operator starred stands in 다시 볼 것 (PRD
            // revisit-group D-10), one that works or waits keeps its group
            // (operator decision 2026-11-11); an ask read and left unanswered
            // stays in 내 차례, read (needs-you-cleanup B11).
            let listed = |group: &str| match group {
                "seen" | "working" if read_ask => "needs_you".to_owned(),
                "seen" if starred => "revisit".to_owned(),
                group => group.to_owned(),
            };
            out.push(PhoneAgent {
                device_id: device_id.to_owned(),
                pane_id: pane_id.to_owned(),
                root_pane_id: roots
                    .get(pane_id)
                    .cloned()
                    .unwrap_or_else(|| pane_id.to_owned()),
                // The list follows the desktop's group; notices tell a View
                // to look at from the row's own ask by the group the row would
                // have without it (status-model.md, A View to look at).
                group: listed(str_of(agent, "group")),
                notice_group: listed(
                    agent
                        .pointer("/look/group")
                        .and_then(Value::as_str)
                        .unwrap_or_else(|| str_of(agent, "group")),
                ),
                look: agent
                    .pointer("/look/label")
                    .and_then(Value::as_str)
                    .and_then(crate::agent_state::first_line_cut),
                read: read_ask,
                starred,
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
                children_left: agent
                    .get("children_left")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                subagents_left: agent
                    .get("subagents_left")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                wait_count: agent.get("wait_count").and_then(Value::as_u64).unwrap_or(0),
                wait_subagents: agent
                    .get("wait_subagents")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                unread: agent
                    .get("unread")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                emphasized: matches!(str_of(agent, "group"), "needs_you" | "done"),
                holds_notification: matches!(str_of(agent, "group"), "needs_you" | "done")
                    || matches!(str_of(agent, "demand"), "question" | "approval" | "error"),
                notification_finalized: agent
                    .get("notification_finalized")
                    .and_then(Value::as_bool)
                    == Some(true)
                    && agent.get("user_turn").is_none_or(Value::is_null)
                    && agent.get("ask_subject").is_none_or(Value::is_null)
                    && agent
                        .get("raised")
                        .and_then(Value::as_array)
                        .is_none_or(Vec::is_empty),
                notification_pending: agent.get("notification_pending").and_then(Value::as_bool)
                    == Some(true),
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
                // The person's turn the row leads with, its own or one a
                // descendant raised, is what its notice says (B13, B17).
                subject: match agent.pointer("/human_turn/own").and_then(Value::as_bool) {
                    Some(true) => declared_subject(agent),
                    Some(false) => agent
                        .get("raised")
                        .and_then(Value::as_array)
                        .and_then(|raised| raised.first())
                        .and_then(|lead| subject_of(lead.get("subject"))),
                    None => None,
                }
                .or_else(|| subject_of(agent.get("ask_subject"))),
                human_turn: agent
                    .get("human_turn")
                    .filter(|lead| !lead.is_null())
                    .cloned(),
                human_turns: declared_subject(agent)
                    .into_iter()
                    .chain(
                        agent
                            .get("raised")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|ask| subject_of(ask.get("subject"))),
                    )
                    .filter_map(|subject| match &subject {
                        AskSubject::HumanTurn { id, .. } => Some((id.clone(), subject.clone())),
                        _ => None,
                    })
                    .collect(),
                status_code: str_of(agent, "status_code").to_owned(),
                demand: agent
                    .get("demand")
                    .and_then(Value::as_str)
                    .unwrap_or("none")
                    .to_owned(),
                sleep: agent
                    .get("sleep")
                    .and_then(|sleep| serde_json::from_value(sleep.clone()).ok()),
                descendants: Descendants::of(agent.get("descendant_counts")),
                children: Vec::new(),
                more_children: Vec::new(),
                children_open: false,
                delegated: agent
                    .get("delegated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                child_pane_ids: agent
                    .get("lineage_child_pane_ids")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                tree_rank: agent
                    .pointer("/state/tree_rank")
                    .and_then(Value::as_u64)
                    .and_then(|rank| u8::try_from(rank).ok())
                    .unwrap_or(u8::MAX),
                activity: activity_key(agent),
            });
        }
    }

    /// The row's ordering key (`SidebarAgentSnapshot::last_activity`), or the
    /// thirteen digits of its change time when an older core sent none.
    fn activity_key(agent: &Value) -> String {
        match agent.get("last_activity").and_then(Value::as_str) {
            Some(key) if !key.is_empty() => key.to_owned(),
            _ => agent
                .get("changed_at_unix_ms")
                .and_then(Value::as_u64)
                .map(|ms| format!("{ms:013}"))
                .unwrap_or_default(),
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
        let mut machines = vec![PhoneDevice {
            device_id: node.to_owned(),
            label: None,
            local: true,
            connected: true,
            needs_you: 0,
            working: 0,
        }];
        for status in rest
            .pointer("/status/remote")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let target = str_of(status, "target_id");
            let label = devices
                .iter()
                .find(|device| str_of(device, "id") == target)
                .map(|device| str_of(device, "label"))
                .filter(|label| !label.is_empty())
                .unwrap_or(target);
            let connected = str_of(status, "state") == "connected";
            machines.push(PhoneDevice {
                device_id: target.to_owned(),
                label: Some(label.to_owned()),
                local: false,
                connected,
                needs_you: 0,
                working: 0,
            });
            if !connected {
                continue;
            }
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
        let sleeping: Vec<PhoneSleepingSession> = rest
            .pointer("/navigator/sleeping_sessions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| {
                match serde_json::from_value::<PhoneSleepingSession>(value.clone()) {
                    Ok(row) if row.device_id == node => Some(row),
                    Ok(_) => None,
                    Err(_) => {
                        crate::diagnostic!(serde_json::json!({
                            "component": "mobile_phone", "kind": "sleep.projection_invalid",
                        }));
                        None
                    }
                }
            })
            .collect();
        let groups = tree::groups(agents, sleeping, &GROUP_ORDER);
        for group in &groups {
            for row in group.rows.iter().chain(&group.more) {
                let GroupRow::Agent(agent) = row else {
                    continue;
                };
                let Some(machine) = machines
                    .iter_mut()
                    .find(|machine| machine.device_id == agent.device_id)
                else {
                    continue;
                };
                match group.group.as_str() {
                    "needs_you" => machine.needs_you += 1,
                    "working" => machine.working += 1,
                    _ => {}
                }
            }
        }
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
            devices: machines,
            interface_language,
            color_theme,
        }
    }

    #[cfg(test)]
    mod wire_tests {
        use serde_json::{Value, json};

        use super::*;

        /// The most sleeping conversations the core keeps (`MAX_DORMANT_RECORDS`).
        const SLEEPING: usize = 256;

        fn agent(pane: &str, group: &str, activity: u64, extra: Value) -> Value {
            let long = "가".repeat(200);
            let mut value = json!({
                "pane_id": pane, "group": group, "mark": "going", "identity_label": format!("{pane} {long}"),
                "agent_kind": "claude", "status_code": "idle", "demand": "none",
                "last_activity": format!("{activity:013}"), "changed_at_unix_ms": activity,
                "state": {"tree_rank": 2, "line": {"text": long, "mode": "sentence", "tone": "subtle"}},
            });
            for (key, field) in extra.as_object().unwrap() {
                value[key] = field.clone();
            }
            value
        }

        /// 40 asking roots, a root with 60 delegated children, 140 more
        /// working, 160 seen, and the most sleeping conversations the core keeps.
        fn crowded() -> Value {
            let mut agents = Vec::new();
            for at in 0..40 {
                agents.push(agent(
                    &format!("w1:n{at}"),
                    "needs_you",
                    9_000 + at,
                    json!({}),
                ));
            }
            let kids: Vec<String> = (0..60).map(|at| format!("w1:k{at}")).collect();
            agents.push(agent(
                "w1:root",
                "working",
                8_999,
                json!({"lineage_child_pane_ids": kids}),
            ));
            for (at, kid) in kids.iter().enumerate() {
                agents.push(agent(kid, "working", 8_000 + at as u64, json!({"delegated": true, "lineage_parent_pane_id": "w1:root", "lineage_child_pane_ids": []})));
            }
            for at in 0..140 {
                agents.push(agent(
                    &format!("w1:w{at}"),
                    "working",
                    5_000 + at,
                    json!({}),
                ));
            }
            for at in 0..160 {
                agents.push(agent(&format!("w1:s{at}"), "seen", 1_000 + at, json!({})));
            }
            let sleeping: Vec<Value> = (0..SLEEPING as u64)
                .map(|at| json!({
                    "sleep_id": format!("sleep-{at:x}-1"), "node_id": "local", "kind": "pi",
                    "identity_label": "가".repeat(200), "group": "seen", "phase": "sleeping",
                    "since_unix_ms": at, "wake_available": true, "checking": false, "reason": null,
                }))
                .collect();
            json!({"navigator": {"agents": agents, "sleeping_sessions": sleeping}})
        }

        fn rows_of(groups: &[Group]) -> usize {
            groups
                .iter()
                .flat_map(|group| group.rows.iter().chain(&group.more))
                .map(GroupRow::agents)
                .sum()
        }

        #[test]
        fn a_crowded_list_fits_one_relay_message_and_counts_every_agent_it_leaves_out() {
            let projection = project(&crowded(), "local");
            let total = rows_of(&projection.groups);
            assert_eq!(total, 40 + 61 + 140 + 160 + SLEEPING);
            assert!(
                wire_len(&projection.groups) > FRAME_LIMIT,
                "the fixture must overflow"
            );
            let wire = projection.wire();
            let frame = json!({
                "type": "agents", "groups": wire.groups, "omitted": wire.omitted,
                "devices": projection.devices, "interface_language": projection.interface_language,
                "color_theme": projection.color_theme,
            });
            let bytes = frame.to_string().len();
            assert!(bytes <= FRAME_LIMIT, "{bytes} bytes");
            assert!(wire.omitted > 0);
            assert_eq!(rows_of(&wire.groups) + wire.omitted, total);
            let asking = |groups: &[Group]| {
                groups
                    .iter()
                    .find(|group| group.group == "needs_you")
                    .map(|group| (group.count, group.rows.len() + group.more.len()))
            };
            assert_eq!(
                asking(&wire.groups),
                asking(&projection.groups),
                "내 차례 loses no row"
            );
            for (sent, whole) in wire.groups.iter().zip(&projection.groups) {
                assert_eq!(
                    sent.count, whole.count,
                    "a group's count stays the true total"
                );
                assert_eq!(
                    sent.more_asleep,
                    sent.more.iter().filter(|row| row.asleep()).count()
                );
            }
            assert_eq!(
                rows_of(&projection.groups),
                total,
                "the projection itself stays whole"
            );
        }

        #[test]
        fn every_other_row_goes_before_a_folded_row_that_waits_on_the_operator() {
            // 300 asking roots, short enough to fit alone, and one working
            // root whose 200 resting children push the list over.
            let mut agents: Vec<Value> = (0..300)
                .map(|at| {
                    let mut row = agent(&format!("w1:n{at}"), "needs_you", 9_000 + at, json!({}));
                    row["identity_label"] = json!(format!("ask {at}"));
                    row["state"]["line"]["text"] = json!("?");
                    row
                })
                .collect();
            let kids: Vec<String> = (0..200).map(|at| format!("w1:k{at}")).collect();
            agents.push(agent(
                "w1:root",
                "working",
                8_999,
                json!({"lineage_child_pane_ids": kids}),
            ));
            for (at, kid) in kids.iter().enumerate() {
                agents.push(agent(kid, "seen", 1_000 + at as u64, json!({"delegated": true, "lineage_parent_pane_id": "w1:root", "lineage_child_pane_ids": []})));
            }
            let projection = project(&json!({"navigator": {"agents": agents}}), "local");
            assert!(
                wire_len(&projection.groups) > LIST_BUDGET,
                "the fixture must overflow"
            );
            let wire = projection.wire();
            let frame = json!({"type": "agents", "groups": wire.groups, "omitted": wire.omitted, "devices": projection.devices});
            assert!(frame.to_string().len() <= FRAME_LIMIT);
            assert!(wire.omitted > 0);
            let asking = wire
                .groups
                .iter()
                .find(|group| group.group == "needs_you")
                .unwrap();
            assert_eq!(
                asking.rows.len() + asking.more.len(),
                300,
                "no row waiting on the operator is left out"
            );
        }

        #[test]
        fn a_list_within_the_budget_is_sent_as_it_is() {
            let projection = project(
                &json!({"navigator": {"agents": [agent("w1:p1", "working", 1, json!({}))]}}),
                "local",
            );
            let wire = projection.wire();
            assert!(matches!(wire.groups, Cow::Borrowed(_)));
            assert_eq!(wire.omitted, 0);
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
