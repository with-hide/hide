//! The Desk room in the runtime (PRD Desk 1차 B1-B13): the operator's
//! messages, the agent's replies, approval cards, the agent, model and
//! effort chosen, and the settings. The core decides each state here; the
//! worker (`desk/worker.rs`) owns the one Desk process and does every call,
//! and the shell only shows the `desk` and `desk_live` sections and
//! dispatches the `desk_*` events.
//!
//! A permission request Hide cannot read, or answer with a one-time
//! allowance, does not run (D-60); a card waits for `한 번 허용` or `거절`,
//! and a new conversation, a restart or the process's end closes it as a
//! refusal (D-52). `allow_always` is never chosen.

mod jobs;
mod worker;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, Weak};

use serde::Deserialize;
use serde_json::{Value, json};

use super::Runtime;
use crate::desk::{
    AdapterId, Conversation, DeskPlatform, DeskStore, KnownChoices, MAX_MENTIONS,
    MAX_MESSAGE_BYTES, MAX_REPLY_BYTES, Stored,
};
use crate::handle::ChangeNotifier;
use crate::model::{
    DESK_CAP_MAX, DeskAgentSnapshot, DeskAgentState, DeskApprovalState, DeskChoiceSnapshot,
    DeskConversationSnapshot, DeskHostSnapshot, DeskHostState, DeskItem, DeskLiveSnapshot,
    DeskLocation, DeskMention, DeskNotice, DeskReplyState, DeskSnapshot,
};
use hide_desk::session::{Choice, Choices, OpenError};
use worker::{Order, Report, TurnOutcome};

/// A mention's id or name; a longer one is refused with its message.
const MAX_MENTION_BYTES: usize = 1024;

#[derive(Default)]
pub(super) struct Desk {
    store: Option<DeskStore>,
    stored: Stored,
    worker: Option<worker::Handle>,
    /// How a thread of Desk's other than the worker answers.
    handle: Option<jobs::DeskHandle>,
    agents: BTreeMap<AdapterId, (DeskAgentState, Option<String>)>,
    host: Host,
    /// The turn under way: its tag and the reply written so far.
    turn: Option<Turn>,
    next_tag: u64,
    approvals: Vec<Approval>,
    /// Watch asks for Desk's sessions that have no watch yet, by intent;
    /// only sessions the ledger still holds are kept.
    watch_asks: HashMap<String, jobs::WatchAsk>,
}

#[derive(Default)]
struct Host {
    state: Option<DeskHostState>,
    detail: Option<String>,
    /// A session is open in the Desk process.
    open: bool,
}

struct Turn {
    tag: u64,
    agent: AdapterId,
    text: String,
    cut: bool,
    started_at_ms: u64,
    first_text_at_ms: Option<u64>,
    steps: crate::model::DeskSteps,
}

/// A card waiting for the operator, and how to answer the agent.
struct Approval {
    item: u64,
    rpc_id: Value,
    allow: String,
    reject: Option<String>,
}

/// What the shell asked (`desk_*`).
#[derive(Debug)]
pub(super) enum DeskEvent {
    Send(SendPayload),
    Stop,
    Approval(ApprovalPayload),
    NewConversation,
    Restart,
    Select(SelectPayload),
    Install(InstallPayload),
    SettingsSet(SettingsPayload),
    /// `취소` on a waiting card.
    JobCancel(JobPayload),
    /// `다시 맡기기` on a cancelled card, `다시 시도` on a failed one.
    JobRequeue(JobPayload),
    /// The operator saw an ended job's card.
    JobRead(JobPayload),
    /// The operator looked at a job's output in the lane's 보기 column (B31).
    OutputSeen(OutputPayload),
    /// The lane's pin, or a tab kept or closed in its 보기 column (B31, B32).
    Lane(LanePayload),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(super) enum LanePayload {
    Pin { pinned: bool },
    Keep { tab: crate::model::DeskKeptTab },
    Close { tab: crate::model::DeskKeptTab },
}

#[derive(Debug, Deserialize)]
pub(super) struct OutputPayload {
    device_id: String,
    checkout: String,
    id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct JobPayload {
    id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SendPayload {
    text: String,
    #[serde(default)]
    mentions: Vec<DeskMention>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ApprovalPayload {
    id: u64,
    allow: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct SelectPayload {
    agent: Option<AdapterId>,
    model: Option<String>,
    effort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct InstallPayload {
    agent: AdapterId,
}

#[derive(Debug, Deserialize)]
pub(super) struct SettingsPayload {
    enabled: Option<bool>,
    /// `null` clears it; absent leaves it.
    #[serde(default, deserialize_with = "present")]
    default_agent: Option<Option<AdapterId>>,
    location: Option<DeskLocation>,
    cap: Option<u8>,
}

fn present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<AdapterId>>, D::Error> {
    Option::<AdapterId>::deserialize(deserializer).map(Some)
}

impl DeskEvent {
    pub(super) const KINDS: [&'static str; 13] = [
        "desk_send",
        "desk_stop",
        "desk_approval",
        "desk_new_conversation",
        "desk_restart",
        "desk_select",
        "desk_install",
        "desk_settings_set",
        "desk_job_cancel",
        "desk_job_requeue",
        "desk_job_read",
        "desk_output_seen",
        "desk_lane",
    ];

    pub(super) fn decode(kind: &str, payload: Value) -> Option<Self> {
        fn parse<T: serde::de::DeserializeOwned>(payload: Value) -> Option<T> {
            serde_json::from_value(payload).ok()
        }
        Some(match kind {
            "desk_send" => Self::Send(parse(payload)?),
            "desk_stop" => Self::Stop,
            "desk_approval" => Self::Approval(parse(payload)?),
            "desk_new_conversation" => Self::NewConversation,
            "desk_restart" => Self::Restart,
            "desk_select" => Self::Select(parse(payload)?),
            "desk_install" => Self::Install(parse(payload)?),
            "desk_settings_set" => Self::SettingsSet(parse(payload)?),
            "desk_job_cancel" => Self::JobCancel(parse(payload)?),
            "desk_job_requeue" => Self::JobRequeue(parse(payload)?),
            "desk_job_read" => Self::JobRead(parse(payload)?),
            "desk_output_seen" => Self::OutputSeen(parse(payload)?),
            "desk_lane" => Self::Lane(parse(payload)?),
            _ => return None,
        })
    }
}

impl Runtime {
    /// Desk's store, read before the lock by the caller.
    pub(crate) fn install_desk_store(&mut self, store: DeskStore, stored: Stored) {
        self.desk.store = Some(store);
        self.desk.stored = stored;
        self.compose_desk_sections();
    }

    /// Starts Desk's worker with what `hided` provides, and looks at the
    /// agents.
    pub(crate) fn install_desk(
        &mut self,
        platform: Arc<dyn DeskPlatform>,
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
    ) {
        self.desk.handle = Some(jobs::DeskHandle {
            runtime: runtime.clone(),
            notifier: notifier.clone(),
        });
        match worker::start(platform, runtime, notifier) {
            Ok(handle) => {
                // Desk switched off looks at no agent (D-76).
                if self.desk.stored.settings.enabled {
                    let _ = handle.orders.send(Order::Check);
                }
                self.desk.worker = Some(handle);
            }
            Err(error) => crate::diagnostic!(json!({
                "component": "desk",
                "kind": "desk.worker_start_failed",
                "error": error.to_string(),
            })),
        }
        self.compose_desk_sections();
    }

    pub(super) fn desk_event(&mut self, event: DeskEvent) -> bool {
        if self.desk.worker.is_none() {
            return refused("desk_unavailable");
        }
        // Switched off, only the switch reaches Desk (D-76).
        if !self.desk.stored.settings.enabled && !matches!(event, DeskEvent::SettingsSet(_)) {
            return refused("desk_off");
        }
        // A card's change moves only the cards: the conversation is not sent again.
        let room = !matches!(
            event,
            DeskEvent::JobCancel(_)
                | DeskEvent::JobRequeue(_)
                | DeskEvent::JobRead(_)
                | DeskEvent::OutputSeen(_)
        );
        // An output seen moves the View's seen state, which the core keeps
        // with the View, not in Desk's store.
        let stored = !matches!(event, DeskEvent::OutputSeen(_));
        let changed = match event {
            DeskEvent::Send(payload) => self.desk_send(payload),
            DeskEvent::Stop => self.desk_stop(),
            DeskEvent::Approval(payload) => self.desk_answer(payload.id, payload.allow),
            DeskEvent::NewConversation => self.desk_new_conversation(),
            DeskEvent::Restart => self.desk_restart(),
            DeskEvent::Select(payload) => self.desk_select(payload),
            DeskEvent::Install(payload) => {
                self.order(Order::Install(payload.agent));
                false
            }
            DeskEvent::SettingsSet(payload) => self.desk_settings(payload),
            DeskEvent::JobCancel(payload) => self.desk_job_cancel(&payload.id),
            DeskEvent::JobRequeue(payload) => {
                self.desk_job_requeue(&payload.id) | self.desk_start_queued()
            }
            DeskEvent::JobRead(payload) => self.desk_job_read(&payload.id),
            DeskEvent::OutputSeen(payload) => {
                self.mark_view_seen(&(payload.device_id, payload.checkout), &payload.id)
            }
            DeskEvent::Lane(payload) => self.desk_lane(payload),
        };
        if changed {
            if stored {
                self.save_desk();
            }
            if room {
                self.compose_desk();
            }
            self.compose_desk_jobs();
        }
        changed
    }

    /// The lane's pin, or a kept tab (B31, B32). Keeping a kept tab or
    /// closing one not kept changes nothing; one past the cap is refused.
    fn desk_lane(&mut self, payload: LanePayload) -> bool {
        let lane = &mut self.desk.stored.lane;
        match payload {
            LanePayload::Pin { pinned } => {
                let changed = lane.pinned != pinned;
                lane.pinned = pinned;
                changed
            }
            LanePayload::Keep { tab } => {
                let key = tab.key();
                if lane.kept.iter().any(|kept| kept.key() == key) {
                    return false;
                }
                if lane.kept.len() >= crate::model::DESK_KEPT_TABS_MAX {
                    return refused("desk_tabs_full");
                }
                lane.kept.push(tab);
                true
            }
            LanePayload::Close { tab } => {
                let key = tab.key();
                let before = lane.kept.len();
                lane.kept.retain(|kept| kept.key() != key);
                lane.kept.len() != before
            }
        }
    }

    /// Whether this daemon runs Desk: its queued spawns start as
    /// `hide:desk` whether or not its process runs.
    pub(super) fn desk_installed(&self) -> bool {
        self.desk.worker.is_some()
    }

    /// Whether the Desk process runs with a session open: only then does a
    /// `hide:desk` capability act.
    pub(super) fn desk_running(&self) -> bool {
        self.desk.worker.is_some() && self.desk.host.open
    }

    /// The context and identity a Desk command acts with, on this machine
    /// alone and while the Desk process runs.
    pub(super) fn desk_authority(
        &self,
        device: &str,
    ) -> Result<(crate::workspace_control::Context, crate::delivery::Actor), String> {
        if self.node != device || !self.desk_running() {
            return Err("desk_not_running".into());
        }
        Ok((
            crate::desk::desk_context(self.node.as_str()),
            crate::delivery::Actor::hide(crate::delivery::HIDE_DESK_ROLE, self.node.as_str()),
        ))
    }

    fn order(&self, order: Order) {
        if let Some(worker) = &self.desk.worker {
            let _ = worker.orders.send(order);
        }
    }

    fn desk_send(&mut self, payload: SendPayload) -> bool {
        let text = payload.text.trim();
        if text.is_empty() || text.len() > MAX_MESSAGE_BYTES {
            return refused("message_size");
        }
        if payload.mentions.len() > MAX_MENTIONS
            || payload.mentions.iter().any(|mention| {
                mention.id.is_empty()
                    || mention.id.len() > MAX_MENTION_BYTES
                    || mention.name.len() > MAX_MENTION_BYTES
            })
        {
            return refused("mentions");
        }
        let Some(agent) = self.desk.stored.selection.agent else {
            return refused("no_agent");
        };
        if !matches!(
            self.desk.agents.get(&agent).map(|(state, _)| *state),
            Some(DeskAgentState::Ready | DeskAgentState::LoginNeeded)
        ) {
            return refused("agent_not_ready");
        }
        if self.desk.turn.is_some() {
            return refused("turn_under_way");
        }
        let carried = (!self.desk.host.open)
            .then(|| self.desk.stored.conversation.carried())
            .flatten();
        let id = self.desk.stored.conversation.next_id();
        self.desk.stored.conversation.push(DeskItem::User {
            id,
            text: text.to_owned(),
            at_ms: crate::links::now_ms(),
            mentions: payload.mentions,
        });
        self.desk.next_tag += 1;
        let tag = self.desk.next_tag;
        self.desk.turn = Some(Turn {
            tag,
            agent,
            text: String::new(),
            cut: false,
            started_at_ms: crate::links::now_ms(),
            first_text_at_ms: None,
            steps: Default::default(),
        });
        let resume = self
            .desk
            .stored
            .sessions
            .iter()
            .find(|(adapter, _)| *adapter == agent)
            .map(|(_, session)| session.clone());
        self.order(Order::Send(worker::Send {
            tag,
            adapter: agent,
            text: text.to_owned(),
            carried,
            resume,
            model: self.desk.stored.selection.model.clone(),
            effort: self.desk.stored.selection.effort.clone(),
            placement: match self.desk.stored.settings.location {
                DeskLocation::Desk => hide_desk::Placement::PerTask,
                DeskLocation::Worktree => hide_desk::Placement::Worktree,
                DeskLocation::Checkout => hide_desk::Placement::Checkout,
            },
        }));
        self.desk.host.state = Some(if self.desk.host.open {
            DeskHostState::Thinking
        } else {
            DeskHostState::Starting
        });
        self.desk.host.detail = None;
        self.compose_desk_live();
        true
    }

    fn desk_stop(&mut self) -> bool {
        if self.desk.turn.is_none() {
            return false;
        }
        // The agent answers each request it is waiting on as cancelled.
        let cancelled = json!({"outcome": {"outcome": "cancelled"}});
        for approval in std::mem::take(&mut self.desk.approvals) {
            self.order(Order::Answer {
                rpc_id: approval.rpc_id,
                outcome: cancelled.clone(),
            });
            self.set_approval(approval.item, DeskApprovalState::Closed);
        }
        self.order(Order::Cancel);
        true
    }

    fn desk_answer(&mut self, item: u64, allow: bool) -> bool {
        let Some(index) = self
            .desk
            .approvals
            .iter()
            .position(|approval| approval.item == item)
        else {
            return refused("approval_unknown");
        };
        let approval = self.desk.approvals.remove(index);
        let (option, state) = if allow {
            (Some(approval.allow), DeskApprovalState::Allowed)
        } else {
            (approval.reject, DeskApprovalState::Rejected)
        };
        let outcome = match option {
            Some(option) => json!({"outcome": {"outcome": "selected", "optionId": option}}),
            None => json!({"outcome": {"outcome": "cancelled"}}),
        };
        self.order(Order::Answer {
            rpc_id: approval.rpc_id,
            outcome,
        });
        self.set_approval(item, state);
        true
    }

    fn desk_new_conversation(&mut self) -> bool {
        self.end_desk_process();
        let agent = self.desk.stored.selection.agent;
        self.desk
            .stored
            .sessions
            .retain(|(adapter, _)| Some(*adapter) != agent);
        self.desk.stored.conversation.clear();
        if let Some(default) = self.desk.stored.settings.default_agent
            && Some(default) != agent
        {
            self.desk.stored.selection = crate::model::DeskSelection {
                agent: Some(default),
                ..Default::default()
            };
        }
        true
    }

    fn desk_restart(&mut self) -> bool {
        self.end_desk_process();
        self.order(Order::Check);
        true
    }

    /// Ends the Desk process from the room: the reply under way stops, and
    /// each waiting card closes as a refusal.
    fn end_desk_process(&mut self) {
        self.order(Order::End);
        self.close_approvals();
        self.finish_turn(DeskReplyState::Stopped);
        self.desk.host = Host {
            state: Some(DeskHostState::Idle),
            ..Host::default()
        };
    }

    fn close_approvals(&mut self) {
        for approval in std::mem::take(&mut self.desk.approvals) {
            self.set_approval(approval.item, DeskApprovalState::Closed);
        }
    }

    fn desk_select(&mut self, payload: SelectPayload) -> bool {
        let selection = self.desk.stored.selection.clone();
        if let Some(agent) = payload.agent
            && Some(agent) != selection.agent
        {
            if self.desk.turn.is_some() {
                self.order(Order::Cancel);
            }
            self.end_desk_process();
            let carried = self.desk.stored.conversation.carried_extent();
            let id = self.desk.stored.conversation.next_id();
            self.desk.stored.conversation.push(DeskItem::Switch {
                id,
                from: selection.agent,
                to: agent,
                carried,
            });
            // The new agent's session starts with the conversation carried.
            self.desk
                .stored
                .sessions
                .retain(|(adapter, _)| *adapter != agent);
            self.desk.stored.selection = crate::model::DeskSelection {
                agent: Some(agent),
                model: payload.model,
                effort: payload.effort,
            };
            if matches!(
                self.desk.agents.get(&agent).map(|(state, _)| *state),
                Some(DeskAgentState::NotInstalled | DeskAgentState::InstallFailed)
            ) {
                self.order(Order::Install(agent));
            }
            return true;
        }
        let model = payload.model.or(selection.model.clone());
        let effort = payload.effort.or(selection.effort.clone());
        if model == selection.model && effort == selection.effort {
            return false;
        }
        let known = selection.agent.and_then(|agent| self.known_choices(agent));
        for (config, value, before) in [
            (
                known.as_ref().and_then(|known| known.model_config.clone()),
                &model,
                &selection.model,
            ),
            (
                known.as_ref().and_then(|known| known.effort_config.clone()),
                &effort,
                &selection.effort,
            ),
        ] {
            if let (Some(config_id), Some(value)) = (config, value)
                && value != before.as_ref().unwrap_or(&String::new())
            {
                self.order(Order::SetChoice {
                    config_id,
                    value: value.clone(),
                });
            }
        }
        let id = self.desk.stored.conversation.next_id();
        self.desk.stored.conversation.push(DeskItem::Model {
            id,
            model: model.clone(),
            effort: effort.clone(),
        });
        self.desk.stored.selection.model = model;
        self.desk.stored.selection.effort = effort;
        true
    }

    fn desk_settings(&mut self, payload: SettingsPayload) -> bool {
        let settings = &mut self.desk.stored.settings;
        let before = *settings;
        if let Some(enabled) = payload.enabled {
            settings.enabled = enabled;
        }
        if let Some(agent) = payload.default_agent {
            settings.default_agent = agent;
        }
        if let Some(location) = payload.location {
            settings.location = location;
        }
        if let Some(cap) = payload.cap {
            if !(1..=DESK_CAP_MAX).contains(&cap) {
                return refused("cap_range");
            }
            settings.cap = cap;
        }
        if self.desk.stored.selection.agent.is_none() {
            self.desk.stored.selection.agent = settings.default_agent;
        }
        let enabled = self.desk.stored.settings.enabled;
        if enabled != before.enabled {
            if enabled {
                self.order(Order::Check);
                self.desk_start_queued();
            } else {
                // Off ends the process and its turn; the sessions it started
                // and the conversation stay.
                if self.desk.turn.is_some() {
                    self.order(Order::Cancel);
                }
                self.end_desk_process();
            }
        }
        before != self.desk.stored.settings
    }

    fn known_choices(&self, agent: AdapterId) -> Option<KnownChoices> {
        self.desk
            .stored
            .choices
            .iter()
            .find(|(adapter, _)| *adapter == agent)
            .map(|(_, known)| known.clone())
    }

    fn set_approval(&mut self, item: u64, state: DeskApprovalState) {
        for entry in &mut self.desk.stored.conversation.items {
            if let DeskItem::Approval {
                id, state: current, ..
            } = entry
                && *id == item
            {
                *current = state;
            }
        }
    }

    fn push_notice(
        &mut self,
        notice: DeskNotice,
        agent: Option<AdapterId>,
        detail: Option<String>,
    ) {
        let id = self.desk.stored.conversation.next_id();
        self.desk.stored.conversation.push(DeskItem::Notice {
            id,
            notice,
            agent,
            detail,
        });
    }

    /// The agent the room talks to now, which a notice about the process
    /// names.
    fn selected_agent(&self) -> Option<AdapterId> {
        self.desk.stored.selection.agent
    }

    /// Ends the turn: its reply joins the conversation.
    fn finish_turn(&mut self, state: DeskReplyState) {
        let Some(turn) = self.desk.turn.take() else {
            return;
        };
        if !turn.text.is_empty() {
            let id = self.desk.stored.conversation.next_id();
            let mentions = hide_desk::session_mentions(&turn.text)
                .into_iter()
                .map(|(pane_id, name)| crate::model::DeskSessionChip {
                    pane_id,
                    name,
                    agent_kind: None,
                    mark: None,
                })
                .collect();
            self.desk.stored.conversation.push(DeskItem::Reply {
                id,
                agent: turn.agent,
                text: turn.text,
                at_ms: crate::links::now_ms(),
                state: if turn.cut {
                    DeskReplyState::Stopped
                } else {
                    state
                },
                thought_ms: turn
                    .first_text_at_ms
                    .map(|at| at.saturating_sub(turn.started_at_ms)),
                steps: turn.steps,
                mentions,
            });
        }
        self.compose_desk_live();
    }

    /// What the worker learned, applied under the lock.
    fn desk_report(&mut self, report: Report) -> bool {
        match report {
            Report::Availability(states) => {
                let mut changed = false;
                for (adapter, state, detail) in states {
                    let was = self.desk.agents.get(&adapter).map(|(state, _)| *state);
                    // An install that ended without the agent is said in
                    // the conversation, where its start was.
                    if was == Some(DeskAgentState::Preparing)
                        && matches!(
                            state,
                            DeskAgentState::InstallFailed | DeskAgentState::NodeMissing
                        )
                    {
                        self.push_notice(DeskNotice::InstallFailed, Some(adapter), detail.clone());
                        self.save_desk();
                    }
                    let entry = (state, detail);
                    if self.desk.agents.get(&adapter) != Some(&entry) {
                        self.desk.agents.insert(adapter, entry);
                        changed = true;
                    }
                }
                if !changed {
                    return false;
                }
            }
            Report::Installing(adapter) => {
                self.desk
                    .agents
                    .insert(adapter, (DeskAgentState::Preparing, None));
                self.push_notice(DeskNotice::Preparing, Some(adapter), None);
                self.save_desk();
            }
            Report::Starting => {
                self.desk.host.state = Some(DeskHostState::Starting);
            }
            Report::Opened {
                adapter,
                session_id,
                resume_failed,
                choices,
            } => {
                self.desk.host = Host {
                    state: Some(DeskHostState::Thinking),
                    detail: None,
                    open: true,
                };
                self.desk
                    .agents
                    .insert(adapter, (DeskAgentState::Ready, None));
                self.desk
                    .stored
                    .sessions
                    .retain(|(stored, _)| *stored != adapter);
                self.desk.stored.sessions.push((adapter, session_id));
                self.learn_choices(adapter, &choices);
                if resume_failed {
                    self.push_notice(DeskNotice::ResumeFailed, Some(adapter), None);
                }
                self.save_desk();
            }
            Report::OpenFailed { adapter, error } => {
                let (state, notice, detail) = match error {
                    OpenError::Login => (
                        DeskAgentState::LoginNeeded,
                        DeskNotice::LoginNeeded,
                        Some(adapter.login_command().to_owned()),
                    ),
                    OpenError::AskModeUnconfirmed(detail) => (
                        DeskAgentState::NoAskMode,
                        DeskNotice::NoAskMode,
                        Some(detail),
                    ),
                    OpenError::Failed(detail) => {
                        crate::diagnostic!(json!({
                            "component": "desk",
                            "kind": "desk.open_failed",
                            "adapter": adapter.as_str(),
                            "detail": detail,
                        }));
                        self.desk.host = Host {
                            state: Some(DeskHostState::Stopped),
                            detail: Some(detail.clone()),
                            open: false,
                        };
                        self.push_notice(DeskNotice::HostStopped, Some(adapter), Some(detail));
                        self.save_desk();
                        self.compose_desk();
                        return true;
                    }
                };
                self.desk.agents.insert(adapter, (state, detail.clone()));
                self.desk.host = Host {
                    state: Some(DeskHostState::Idle),
                    ..Host::default()
                };
                self.push_notice(notice, Some(adapter), detail);
                self.save_desk();
            }
            Report::Text { tag, text } => {
                let Some(turn) = self.desk.turn.as_mut().filter(|turn| turn.tag == tag) else {
                    return false;
                };
                turn.first_text_at_ms
                    .get_or_insert_with(crate::links::now_ms);
                if turn.cut {
                    return false;
                }
                let room = MAX_REPLY_BYTES.saturating_sub(turn.text.len());
                if text.len() > room {
                    let mut end = room;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    turn.text.push_str(&text[..end]);
                    turn.cut = true;
                } else {
                    turn.text.push_str(&text);
                }
                self.compose_desk_live();
                return true;
            }
            Report::Steps { tag, steps } => {
                if let Some(turn) = self.desk.turn.as_mut().filter(|turn| turn.tag == tag) {
                    // A start counts once its spawn answered that it started
                    // (`desk_spawn_answered`), so a refused one is not told
                    // as a thread started.
                    for step in steps {
                        if step != hide_desk::permission::Step::Start {
                            turn.steps.count(step);
                        }
                    }
                }
                return false;
            }
            Report::TurnEnded { tag, outcome } => {
                if self.desk.turn.as_ref().is_none_or(|turn| turn.tag != tag) {
                    return false;
                }
                // Requests the agent made in the turn end with it.
                self.close_approvals();
                match outcome {
                    TurnOutcome::Done => self.finish_turn(DeskReplyState::Done),
                    TurnOutcome::Cancelled => self.finish_turn(DeskReplyState::Stopped),
                    TurnOutcome::Failed(detail) => {
                        self.finish_turn(DeskReplyState::Stopped);
                        let agent = self.selected_agent();
                        self.push_notice(DeskNotice::TurnFailed, agent, Some(detail));
                    }
                }
                if self.desk.host.open {
                    self.desk.host.state = Some(DeskHostState::Ready);
                }
                self.save_desk();
            }
            Report::Ask {
                rpc_id,
                allow,
                reject,
                card,
                command,
                folder,
                reason,
            } => {
                let id = self.desk.stored.conversation.next_id();
                self.desk.stored.conversation.push(DeskItem::Approval {
                    id,
                    card,
                    command,
                    folder,
                    reason,
                    state: DeskApprovalState::Waiting,
                });
                self.desk.approvals.push(Approval {
                    item: id,
                    rpc_id,
                    allow,
                    reject,
                });
                self.save_desk();
            }
            Report::Blocked(title) => {
                let agent = self.selected_agent();
                self.push_notice(
                    DeskNotice::Blocked,
                    agent,
                    Some(title).filter(|t| !t.is_empty()),
                );
                self.save_desk();
            }
            Report::Choices { adapter, choices } => {
                if !self.learn_choices(adapter, &choices) {
                    return false;
                }
                self.save_desk();
            }
            Report::Ended { unexpected, detail } => {
                self.close_approvals();
                self.finish_turn(DeskReplyState::Stopped);
                if unexpected {
                    self.desk.host = Host {
                        state: Some(DeskHostState::Stopped),
                        detail: detail.clone(),
                        open: false,
                    };
                    let agent = self.selected_agent();
                    self.push_notice(DeskNotice::HostStopped, agent, detail);
                    self.save_desk();
                } else {
                    self.desk.host.open = false;
                    if self.desk.host.state != Some(DeskHostState::Stopped) {
                        self.desk.host.state = Some(DeskHostState::Idle);
                    }
                }
            }
        }
        self.compose_desk();
        true
    }

    /// Keeps what the session offers to choose; the selection follows the
    /// session's current values where the operator chose none.
    fn learn_choices(&mut self, adapter: AdapterId, choices: &Choices) -> bool {
        let mut known = self.known_choices(adapter).unwrap_or_default();
        let before = known.clone();
        let list = |choice: &Choice| {
            choice
                .values
                .iter()
                .map(|(id, label)| DeskChoiceSnapshot {
                    id: id.clone(),
                    label: label.clone(),
                })
                .collect::<Vec<_>>()
        };
        if let Some(model) = &choices.model {
            known.model_config = Some(model.config_id.clone());
            known.models = list(model);
        }
        if let Some(effort) = &choices.effort {
            known.effort_config = Some(effort.config_id.clone());
            known.efforts = list(effort);
        }
        let selection = &mut self.desk.stored.selection;
        let mut changed = known != before;
        if selection.agent == Some(adapter) {
            for (current, chosen) in [
                (choices.model.as_ref(), &mut selection.model),
                (choices.effort.as_ref(), &mut selection.effort),
            ] {
                if let Some(value) = current.and_then(|choice| choice.current.clone())
                    && chosen.as_ref() != Some(&value)
                {
                    *chosen = Some(value);
                    changed = true;
                }
            }
        }
        self.desk
            .stored
            .choices
            .retain(|(stored, _)| *stored != adapter);
        self.desk.stored.choices.push((adapter, known));
        changed
    }

    fn save_desk(&self) {
        if let Some(store) = &self.desk.store {
            store.save(self.desk.stored.clone());
        }
    }

    /// The `desk` section, a new `Arc` whenever it is composed.
    fn compose_desk(&mut self) {
        if self.desk.worker.is_none() {
            return;
        }
        let desk = &self.desk;
        let agents = AdapterId::ALL
            .into_iter()
            .map(|id| {
                let (state, detail) = desk
                    .agents
                    .get(&id)
                    .cloned()
                    .unwrap_or((DeskAgentState::Checking, None));
                let known = desk
                    .stored
                    .choices
                    .iter()
                    .find(|(adapter, _)| *adapter == id)
                    .map(|(_, known)| known.clone())
                    .unwrap_or_default();
                DeskAgentSnapshot {
                    id,
                    label: id.label(),
                    state,
                    detail,
                    login_command: id.login_command(),
                    models: known.models,
                    efforts: known.efforts,
                }
            })
            .collect();
        let Conversation {
            generation,
            items,
            dropped,
            ..
        } = &desk.stored.conversation;
        self.snapshot.desk = Some(Arc::new(DeskSnapshot {
            selection: desk.stored.selection.clone(),
            agents,
            host: DeskHostSnapshot {
                state: desk.host.state.unwrap_or(DeskHostState::Idle),
                detail: desk.host.detail.clone(),
            },
            conversation: DeskConversationSnapshot {
                generation: *generation,
                items: {
                    let mut items = items.clone();
                    self.desk_fill_chips(&mut items);
                    items
                },
                dropped: *dropped,
            },
            approvals_waiting: desk.approvals.len(),
            settings: desk.stored.settings,
            lane: desk.stored.lane.clone(),
        }));
    }

    /// Every Desk section but the reply being written.
    pub(in crate::runtime) fn compose_desk_sections(&mut self) {
        self.compose_desk();
        self.compose_desk_jobs();
        self.compose_desk_mentions();
    }

    fn compose_desk_live(&mut self) {
        if self.desk.worker.is_none() {
            return;
        }
        let turn = self.desk.turn.as_ref();
        self.snapshot.desk_live = Some(Arc::new(DeskLiveSnapshot {
            text: turn.map(|turn| turn.text.clone()),
            started_at_ms: turn.map(|turn| turn.started_at_ms),
        }));
    }
}

#[cfg(test)]
impl Runtime {
    /// Runs Desk with a worker nobody reads, for the runtime's other tests.
    pub(in crate::runtime) fn start_test_desk(&mut self) {
        let (orders, _unread) = std::sync::mpsc::channel();
        self.desk.worker = Some(worker::Handle { orders });
        self.desk.stored.settings.enabled = true;
        self.compose_desk_sections();
    }

    /// Lets Desk's queue start a waiting spawn on its own thread, as
    /// `install_desk` does; `notifier` hears each answer it applies.
    pub(in crate::runtime) fn hand_test_desk(
        &mut self,
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
    ) {
        self.desk.handle = Some(jobs::DeskHandle { runtime, notifier });
    }

    /// Runs Desk with a session open, so its process's commands reach the
    /// core, and `cap` places.
    pub(in crate::runtime) fn open_test_desk(&mut self, cap: u8) {
        self.start_test_desk();
        self.desk.host.open = true;
        self.desk.stored.settings.cap = cap;
    }
}

fn refused(reason: &str) -> bool {
    crate::diagnostic!(json!({
        "component": "desk",
        "kind": "desk.event_refused",
        "reason": reason,
    }));
    false
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver};

    use serde_json::json;

    use super::*;
    use crate::model::DeskItem;

    /// A runtime whose Desk worker is a channel the test reads, with Claude
    /// and Pi installed.
    pub(super) fn runtime(root: &std::path::Path) -> (Runtime, Receiver<Order>) {
        let state = root.join("state");
        hide_platform::fs::private::create_dir_all(&state).unwrap();
        let options: crate::model::CoreOptions = serde_json::from_value(json!({
            "schema_version": crate::model::SCHEMA_VERSION, "node_id": "test-node", "home": root,
            "herdr_socket_path": null, "app_state_path": state.join("app.json"),
        }))
        .unwrap();
        let mut runtime = Runtime::new(
            options,
            crate::environment::EnvironmentReport {
                statuses: Vec::new(),
                home_path: Some(root.to_owned()),
                codex_home: None,
            },
            Arc::new(hide_node::Local::new(Some(root.to_owned()))),
            crate::node::test_devices(),
        );
        let (orders, received) = mpsc::channel();
        runtime.desk.worker = Some(worker::Handle { orders });
        runtime.desk.stored.settings.enabled = true;
        runtime.desk_report(Report::Availability(vec![
            (AdapterId::Claude, DeskAgentState::Ready, None),
            (AdapterId::Pi, DeskAgentState::Ready, None),
        ]));
        (runtime, received)
    }

    fn event(runtime: &mut Runtime, kind: &str, payload: serde_json::Value) -> bool {
        runtime.desk_event(DeskEvent::decode(kind, payload).expect(kind))
    }

    fn sent(received: &Receiver<Order>) -> worker::Send {
        loop {
            match received.try_recv().expect("no Send order") {
                Order::Send(send) => return send,
                _ => continue,
            }
        }
    }

    fn items(runtime: &Runtime) -> Vec<DeskItem> {
        runtime
            .snapshot
            .desk
            .as_ref()
            .unwrap()
            .conversation
            .items
            .clone()
    }

    fn opened(runtime: &mut Runtime, adapter: AdapterId) {
        runtime.desk_report(Report::Opened {
            adapter,
            session_id: format!("{}-session", adapter.as_str()),
            resume_failed: false,
            choices: Choices::default(),
        });
    }

    fn ask(runtime: &mut Runtime) -> u64 {
        runtime.desk_report(Report::Ask {
            rpc_id: json!(7),
            allow: "allow-once".into(),
            reject: Some("reject-once".into()),
            card: crate::model::DeskCardKind::Command,
            command: "touch x".into(),
            folder: None,
            reason: None,
        });
        items(runtime).last().unwrap().id()
    }

    #[test]
    fn desk_is_off_by_default_and_only_its_switch_reaches_it_while_off() {
        assert!(!crate::model::DeskSettings::default().enabled);
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        runtime.desk.stored.settings.enabled = false;
        for (kind, payload) in [
            ("desk_select", json!({"agent": "claude"})),
            ("desk_send", json!({"text": "start something"})),
            ("desk_lane", json!({"action": "pin", "pinned": true})),
        ] {
            assert!(!event(&mut runtime, kind, payload), "{kind}");
        }
        assert!(received.try_iter().next().is_none());

        assert!(event(
            &mut runtime,
            "desk_settings_set",
            json!({"enabled": true})
        ));
        assert!(matches!(received.try_recv(), Ok(Order::Check)));
        assert!(runtime.snapshot.desk.as_ref().unwrap().settings.enabled);
        assert!(event(
            &mut runtime,
            "desk_select",
            json!({"agent": "claude"})
        ));
    }

    #[test]
    fn switching_desk_off_ends_its_process_and_keeps_its_conversation() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "what runs now?"})
        ));
        sent(&received);
        opened(&mut runtime, AdapterId::Claude);
        let card = ask(&mut runtime);
        assert!(runtime.desk_authority("test-node").is_ok());
        let before = items(&runtime).len();

        assert!(event(
            &mut runtime,
            "desk_settings_set",
            json!({"enabled": false})
        ));
        let orders: Vec<Order> = received.try_iter().collect();
        assert!(orders.iter().any(|order| matches!(order, Order::Cancel)));
        assert!(orders.iter().any(|order| matches!(order, Order::End)));
        assert_eq!(
            runtime.desk_authority("test-node"),
            Err("desk_not_running".into())
        );
        let items = items(&runtime);
        assert!(items.len() >= before);
        assert!(
            items
                .iter()
                .any(|item| matches!(item, DeskItem::User { .. }))
        );
        assert!(items.iter().any(|item| matches!(item,
            DeskItem::Approval { id, state: DeskApprovalState::Closed, .. } if *id == card)));
        assert!(!runtime.snapshot.desk.as_ref().unwrap().settings.enabled);
    }

    #[test]
    fn a_message_starts_the_agent_and_its_reply_joins_the_conversation() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        assert!(event(
            &mut runtime,
            "desk_select",
            json!({"agent": "claude"})
        ));
        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "  what runs now?  "})
        ));
        let send = sent(&received);
        assert_eq!(send.adapter, AdapterId::Claude);
        assert_eq!(send.text, "what runs now?");
        assert_eq!(send.resume, None);
        assert!(!event(&mut runtime, "desk_send", json!({"text": "again"})));

        opened(&mut runtime, AdapterId::Claude);
        runtime.desk_report(Report::Text {
            tag: send.tag,
            text: "two ".into(),
        });
        runtime.desk_report(Report::Text {
            tag: send.tag,
            text: "sessions".into(),
        });
        let live = runtime.snapshot.desk_live.as_ref().unwrap();
        assert_eq!(live.text.as_deref(), Some("two sessions"));
        assert!(live.started_at_ms.is_some());
        runtime.desk_report(Report::Steps {
            tag: send.tag,
            steps: vec![
                hide_desk::permission::Step::Sessions,
                hide_desk::permission::Step::Sessions,
                hide_desk::permission::Step::Letter,
            ],
        });
        runtime.desk_report(Report::TurnEnded {
            tag: send.tag,
            outcome: TurnOutcome::Done,
        });

        let desk = runtime.snapshot.desk.as_ref().unwrap();
        assert_eq!(desk.host.state, DeskHostState::Ready);
        let Some(DeskItem::Reply {
            text,
            state: DeskReplyState::Done,
            thought_ms,
            steps,
            ..
        }) = desk.conversation.items.last()
        else {
            panic!("no reply");
        };
        assert_eq!(text, "two sessions");
        assert!(thought_ms.is_some());
        assert_eq!((steps.sessions, steps.letters), (2, 1));
        assert_eq!(runtime.snapshot.desk_live.as_ref().unwrap().text, None);
    }

    #[test]
    fn codex_is_chosen_installed_and_made_the_default_like_any_agent() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "codex"}));
        event(&mut runtime, "desk_install", json!({"agent": "codex"}));
        event(
            &mut runtime,
            "desk_settings_set",
            json!({"default_agent": "codex"}),
        );
        assert_eq!(runtime.desk.stored.selection.agent, Some(AdapterId::Codex));
        assert_eq!(
            runtime.desk.stored.settings.default_agent,
            Some(AdapterId::Codex)
        );
        assert!(
            received
                .try_iter()
                .any(|order| matches!(order, Order::Install(AdapterId::Codex)))
        );
    }

    #[test]
    fn a_card_answers_the_agent_once_with_the_one_time_allowance() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "pi"}));
        event(&mut runtime, "desk_send", json!({"text": "make a file"}));
        sent(&received);
        opened(&mut runtime, AdapterId::Pi);
        let card = ask(&mut runtime);
        assert_eq!(runtime.snapshot.desk.as_ref().unwrap().approvals_waiting, 1);

        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": card, "allow": true})
        ));
        let answered = received
            .try_iter()
            .find_map(|order| match order {
                Order::Answer { rpc_id, outcome } => Some((rpc_id, outcome)),
                _ => None,
            })
            .expect("no answer");
        assert_eq!(
            answered,
            (
                json!(7),
                json!({"outcome": {"outcome": "selected", "optionId": "allow-once"}})
            )
        );
        assert!(!event(
            &mut runtime,
            "desk_approval",
            json!({"id": card, "allow": false})
        ));
        assert!(matches!(
            items(&runtime).last(),
            Some(DeskItem::Approval {
                state: DeskApprovalState::Allowed,
                ..
            })
        ));
    }

    #[test]
    fn a_restart_closes_a_waiting_card_and_resumes_the_session_after() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        event(&mut runtime, "desk_send", json!({"text": "edit it"}));
        let first = sent(&received);
        opened(&mut runtime, AdapterId::Claude);
        let card = ask(&mut runtime);

        assert!(event(&mut runtime, "desk_restart", json!({})));
        let orders: Vec<Order> = received.try_iter().collect();
        assert!(orders.iter().any(|order| matches!(order, Order::End)));
        assert!(orders.iter().any(|order| matches!(order, Order::Check)));
        let closed = items(&runtime).into_iter().find(|item| item.id() == card);
        assert!(matches!(
            closed,
            Some(DeskItem::Approval {
                state: DeskApprovalState::Closed,
                ..
            })
        ));
        assert!(!event(
            &mut runtime,
            "desk_approval",
            json!({"id": card, "allow": true})
        ));
        // The reply under way stopped with the process.
        runtime.desk_report(Report::TurnEnded {
            tag: first.tag,
            outcome: TurnOutcome::Done,
        });

        event(&mut runtime, "desk_send", json!({"text": "go on"}));
        assert_eq!(sent(&received).resume.as_deref(), Some("claude-session"));
    }

    #[test]
    fn a_switch_carries_the_conversation_to_the_new_agent_s_first_session() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        event(&mut runtime, "desk_send", json!({"text": "hello"}));
        let first = sent(&received);
        opened(&mut runtime, AdapterId::Claude);
        runtime.desk_report(Report::Text {
            tag: first.tag,
            text: "hi".into(),
        });
        runtime.desk_report(Report::TurnEnded {
            tag: first.tag,
            outcome: TurnOutcome::Done,
        });

        assert!(event(&mut runtime, "desk_select", json!({"agent": "pi"})));
        assert!(matches!(
            items(&runtime).last(),
            Some(DeskItem::Switch {
                to: AdapterId::Pi,
                carried: crate::model::DeskCarried::All,
                ..
            })
        ));
        event(&mut runtime, "desk_send", json!({"text": "continue"}));
        let send = sent(&received);
        assert_eq!(send.adapter, AdapterId::Pi);
        assert_eq!(send.carried.as_deref(), Some("Operator: hello\nClaude: hi"));
    }

    #[test]
    fn a_message_past_its_cap_or_without_an_installed_agent_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        assert!(!event(
            &mut runtime,
            "desk_send",
            json!({"text": "no agent yet"})
        ));
        runtime.desk_report(Report::Availability(vec![(
            AdapterId::Pi,
            DeskAgentState::CliMissing,
            Some("pi".into()),
        )]));
        event(&mut runtime, "desk_select", json!({"agent": "pi"}));
        assert!(!event(
            &mut runtime,
            "desk_send",
            json!({"text": "pi is not ready"})
        ));
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        let long = "x".repeat(crate::desk::MAX_MESSAGE_BYTES + 1);
        assert!(!event(&mut runtime, "desk_send", json!({"text": long})));
        assert!(
            received
                .try_iter()
                .all(|order| !matches!(order, Order::Send(_)))
        );
    }

    fn kept_output(job: &str, id: &str) -> serde_json::Value {
        json!({"kind": "output", "job": job, "output": {
            "id": id, "kind": "image", "label": id, "thumb": null, "seen": true, "pr_state": null,
            "device_id": "node", "checkout": "/repo", "path": format!("/repo/{id}"), "url": null,
        }})
    }

    fn lane(runtime: &Runtime) -> crate::model::DeskLane {
        runtime.snapshot.desk.as_ref().unwrap().lane.clone()
    }

    #[test]
    fn the_lanes_pin_and_kept_tabs_are_kept_in_desks_store() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, _) = runtime(root.path());
        // B32: the pin is kept; pressing it again where it is changes nothing.
        assert!(event(
            &mut runtime,
            "desk_lane",
            json!({"action": "pin", "pinned": true})
        ));
        assert!(!event(
            &mut runtime,
            "desk_lane",
            json!({"action": "pin", "pinned": true})
        ));
        // B31: a kept tab is kept once, and closing it lets it go.
        let tab = kept_output("job-1", "compare.png");
        assert!(event(
            &mut runtime,
            "desk_lane",
            json!({"action": "keep", "tab": tab})
        ));
        assert!(!event(
            &mut runtime,
            "desk_lane",
            json!({"action": "keep", "tab": tab})
        ));
        let mission = json!({"kind": "mission", "mission": "mission-1", "title": "Desk 1차"});
        assert!(event(
            &mut runtime,
            "desk_lane",
            json!({"action": "keep", "tab": mission})
        ));
        assert_eq!(lane(&runtime).kept.len(), 2);
        assert!(lane(&runtime).pinned);
        assert!(event(
            &mut runtime,
            "desk_lane",
            json!({"action": "close", "tab": mission})
        ));
        let kept = lane(&runtime);
        assert_eq!(
            kept.kept.iter().map(|tab| tab.key()).collect::<Vec<_>>(),
            ["output:job-1:compare.png"]
        );
        assert_eq!(
            runtime.desk.stored.lane, kept,
            "the store saves what the room shows"
        );
    }

    #[test]
    fn a_tab_past_the_lanes_cap_is_refused_and_the_kept_ones_stay() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, _) = runtime(root.path());
        for index in 0..crate::model::DESK_KEPT_TABS_MAX {
            let tab = kept_output("job", &format!("{index}.png"));
            assert!(event(
                &mut runtime,
                "desk_lane",
                json!({"action": "keep", "tab": tab})
            ));
        }
        let past = kept_output("job", "past.png");
        assert!(!event(
            &mut runtime,
            "desk_lane",
            json!({"action": "keep", "tab": past})
        ));
        let kept = lane(&runtime).kept;
        assert_eq!(kept.len(), crate::model::DESK_KEPT_TABS_MAX);
        assert_eq!(kept[0].key(), "output:job:0.png");
    }
}
