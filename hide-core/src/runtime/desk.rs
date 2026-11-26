//! The Desk room in the runtime (PRD Desk 1차 B1-B13): the operator's
//! messages, the agent's replies, approval cards, the agent, model and
//! effort chosen, and the settings. The core decides each state here; the
//! worker (`desk/worker.rs`) owns the one Desk process and does every call,
//! and the shell only shows the `desk` and `desk_live` sections and
//! dispatches the `desk_*` events.
//!
//! A permission request is answered here from the choice under Desk's box
//! (D-92): under `다 허용` with the agent's allow-once option at once, under
//! `에이전트대로` by a card of the agent's own options, which goes back to
//! the agent as the one the operator pressed, but a line of `hide` commands,
//! which Hide allows once itself. A request that offers no allow-once
//! option is a card under either. Hide picks no other option for the
//! operator and keeps no allowance of its own. A new conversation, a restart
//! or the process's end closes a waiting card as a refusal.

mod files;
pub(super) mod jobs;
mod named;
mod opens;
mod worker;

use std::collections::{BTreeMap, BTreeSet, HashMap};
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
    DESK_CAP_MAX, DESK_QUEUED_MAX, DeskAgentSnapshot, DeskAgentState, DeskApprovalState,
    DeskAttachment, DeskCardKind, DeskChoiceSnapshot, DeskConversationSnapshot, DeskHostSnapshot,
    DeskHostState, DeskItem, DeskLiveSnapshot, DeskLocation, DeskMention, DeskNotice,
    DeskPermission, DeskQueuedMessage, DeskReplyState, DeskSnapshot,
};
use hide_desk::permission;
use hide_desk::session::{Choice, Choices, OpenError};
use worker::{Order, Report, TurnEnd, TurnOutcome};

/// A mention's id or name; a longer one is refused with its message.
const MAX_MENTION_BYTES: usize = 1024;
/// The line `거절…` sends Desk; a longer one is refused with the answer.
const MAX_REFUSAL_BYTES: usize = 2048;

#[derive(Default)]
pub(super) struct Desk {
    store: Option<DeskStore>,
    stored: Stored,
    worker: Option<worker::Handle>,
    /// How a thread of Desk's other than the worker answers.
    handle: Option<jobs::DeskHandle>,
    agents: BTreeMap<AdapterId, (DeskAgentState, Option<String>)>,
    /// What the catalog said of each agent when the bar was last composed.
    catalog: Vec<CatalogAgent>,
    host: Host,
    /// The turn under way: its tag and the reply written so far.
    turn: Option<Turn>,
    next_tag: u64,
    approvals: Vec<Approval>,
    /// The operator's last refusal in the turn under way, told to the agent
    /// once that turn ends: always when it carries a line (`거절…`), and a
    /// plain one only when the agent wrote nothing after it and no message
    /// of the operator's waits.
    refusal: Option<Refusal>,
    /// Watch asks for Desk's sessions that have no watch yet, by intent;
    /// only sessions the ledger still holds are kept.
    watch_asks: HashMap<String, jobs::WatchAsk>,
    /// The files Desk's tab panel holds open, oldest first (`files.rs`);
    /// the window's, so never stored.
    files: Vec<files::DeskFile>,
    /// What Desk's agent opened with `hide file open`, waiting for a window
    /// showing Desk to open it in the tab panel (`opens.rs`); never stored.
    opens: opens::DeskOpens,
    /// The records of the sessions the conversation names whose pane is
    /// gone, as the link record answered (`named.rs`); never stored.
    records: named::Records,
    /// The agent refused the last start (its sign-in, a process that did
    /// not open): the queued messages wait for the
    /// operator's next send, `다시 시작` or a session that opens, rather
    /// than meet the same refusal one after another.
    held: bool,
    /// Agents that refused Desk for their sign-in since their last session
    /// opened: their word reads signed out whatever Hide AI's read of the
    /// login says, since a login file can outlive its sign-in.
    sign_in_refused: BTreeSet<AdapterId>,
    /// The queued starts the queue handed to be run, in order; tests answer
    /// them themselves.
    #[cfg(test)]
    launched: Vec<String>,
}

/// One of Desk's agents as Hide's catalog says it on the core's own
/// machine: switched on in its kit (D-79) and whether the operator is signed
/// in to it (D-81).
#[derive(Clone, Debug, PartialEq)]
struct CatalogAgent {
    id: AdapterId,
    enabled: bool,
    signed_in: Option<bool>,
}

/// Whether Hide AI's read of an agent says the operator is signed in, as
/// Settings words it: only a read that checked the sign-in says yes, and an
/// unread or failed read says nothing.
fn signed_in(provider: &crate::model::BackgroundAiProviderSnapshot) -> Option<bool> {
    match provider.state.as_str() {
        "needs_login" => Some(false),
        "ready" | "usage_limited" if provider.login_checked => Some(true),
        _ => None,
    }
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
    /// The operator's message the turn answers; none for a turn Desk
    /// started itself to tell the agent of a plain refusal.
    message: Option<u64>,
    text: String,
    cut: bool,
    started_at_ms: u64,
    first_text_at_ms: Option<u64>,
    steps: crate::model::DeskSteps,
    /// What its newest tool call does, until the agent writes again.
    now: Option<crate::model::DeskStepKind>,
}

/// A card waiting for the operator, and how to answer the agent.
struct Approval {
    item: u64,
    rpc_id: Value,
    /// The agent's options, one of which goes back to it.
    options: Vec<permission::Choice>,
    /// What the agent asked to run and what kind of call it is, which a
    /// refusal names.
    command: String,
    kind: permission::CardKind,
}

/// A refusal to send once the turn it ended ends: what was refused and
/// `거절…`'s line, none for a plain refusal.
struct Refusal {
    command: String,
    kind: permission::CardKind,
    text: Option<String>,
    /// How much of the turn's reply had been written when the operator
    /// refused: a plain refusal is sent only if nothing was written after.
    said: usize,
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
    /// A panel width the operator dragged and let go (PRD desk-2-ui B22):
    /// one event per drag.
    Lane(LanePayload),
    /// The files windows' rooms brought into Desk's hold and let go, from
    /// `hided`, which owns each hold (PRD desk-2-ui B19, B36).
    File(files::FilePayload),
    /// A held file that could not be opened, read again (B28).
    FileRetry(files::RetryPayload),
    /// A window opened one of `opens` in its room.
    OpenTaken(opens::TakenPayload),
    /// ✕ on a message waiting for the turn before it: it never goes.
    QueuedRemove(QueuedPayload),
}

#[derive(Debug, Deserialize)]
pub(super) struct QueuedPayload {
    id: u64,
}

/// The width a drag set; the one it did not move is absent.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LanePayload {
    #[serde(default)]
    view_width: Option<u32>,
    #[serde(default)]
    session_width: Option<u32>,
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
    /// The files the composer sent, staged by the window's hided, which is
    /// the only maker of this field (`hided/src/server.rs`,
    /// `handle_desk_send`).
    #[serde(default)]
    files: Vec<DeskAttachment>,
    /// `다시 시도`: the operator's message it sends again, whose files go
    /// with it when this send carries none.
    #[serde(default)]
    again: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ApprovalPayload {
    id: u64,
    /// The id of the agent's option the operator pressed; none is `거절…`
    /// on a card that offers no rejecting option, sent to the agent as
    /// ACP's `cancelled`, and refused on a card that offers one.
    option: Option<String>,
    /// `거절…`'s line, with a refusing option only; absent or blank is a
    /// plain refusal.
    #[serde(default)]
    text: Option<String>,
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
    /// The choice under Desk's box (D-92).
    permission: Option<DeskPermission>,
}

fn present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<AdapterId>>, D::Error> {
    Option::<AdapterId>::deserialize(deserializer).map(Some)
}

impl DeskEvent {
    pub(super) const KINDS: [&'static str; 17] = [
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
        "desk_file",
        "desk_file_retry",
        "desk_open_taken",
        "desk_queued_remove",
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
            "desk_file" => Self::File(parse(payload)?),
            "desk_file_retry" => Self::FileRetry(parse(payload)?),
            "desk_open_taken" => Self::OpenTaken(parse(payload)?),
            "desk_queued_remove" => Self::QueuedRemove(parse(payload)?),
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
                    self.ask_ai_catalog_read();
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
        // A View seen is the View's own state, kept with the View and not in
        // Desk's store, so neither a Desk that is off nor one that is not
        // installed refuses it: the phone's 볼 것 sends the same event for a
        // View it showed (PRD mobile-companion-ui D-15).
        let view_seen = matches!(event, DeskEvent::OutputSeen(_));
        // Letting a file go always lands, so a Desk switched off holds
        // nothing a window let go; `desk_file` opens nothing while it is off.
        // A taken open only leaves the list, which turning Desk off emptied.
        let file_holds = matches!(event, DeskEvent::File(_) | DeskEvent::OpenTaken(_));
        if !view_seen && !file_holds {
            if self.desk.worker.is_none() {
                return refused("desk_unavailable");
            }
            // Switched off, only the switch reaches Desk (D-76).
            if !self.desk.stored.settings.enabled && !matches!(event, DeskEvent::SettingsSet(_)) {
                return refused("desk_off");
            }
        }
        // A card's change moves only the cards: the conversation is not sent again.
        let room = !matches!(
            event,
            DeskEvent::JobCancel(_)
                | DeskEvent::JobRequeue(_)
                | DeskEvent::JobRead(_)
                | DeskEvent::OutputSeen(_)
        );
        // A width moves only the `desk` section's lane; a file Desk holds
        // moves the editor's documents, no section of Desk's.
        let file = matches!(event, DeskEvent::File(_) | DeskEvent::FileRetry(_));
        let jobs = !file && !matches!(event, DeskEvent::Lane(_) | DeskEvent::OpenTaken(_));
        let room = room && !file;
        // An output seen moves the View's seen state, which the core keeps
        // with the View, not in Desk's store; a file held is the window's.
        let stored = !file && !matches!(event, DeskEvent::OutputSeen(_) | DeskEvent::OpenTaken(_));
        let changed = match event {
            DeskEvent::Send(payload) => {
                // Only the Desk composer sends it.
                self.note_operator_active(crate::product_analytics::ActiveSurface::Desktop);
                self.desk_send(payload)
            }
            DeskEvent::Stop => self.desk_stop(),
            DeskEvent::Approval(payload) => self.desk_answer(payload),
            DeskEvent::NewConversation => self.desk_new_conversation(),
            DeskEvent::Restart => self.desk_restart(),
            DeskEvent::Select(payload) => self.desk_select(payload),
            DeskEvent::Install(payload) => {
                self.order(Order::Install {
                    adapter: payload.agent,
                    asked: true,
                });
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
            DeskEvent::File(payload) => self.desk_file(payload),
            DeskEvent::FileRetry(payload) => self.desk_file_retry(payload.into()),
            DeskEvent::OpenTaken(payload) => self.desk_open_taken(payload),
            DeskEvent::QueuedRemove(payload) => self.desk_queued_remove(payload.id),
        };
        let sent = self.desk_send_queued();
        if changed || sent {
            if stored || sent {
                self.save_desk();
            }
            if room || sent {
                self.compose_desk();
            }
            if jobs {
                self.compose_desk_jobs();
            }
        }
        changed || sent
    }

    /// The widths the operator dragged Desk's panels to (B22, B23). A drag
    /// that names no width, or one outside what the core keeps, is refused.
    fn desk_lane(&mut self, payload: LanePayload) -> bool {
        let valid = |width: Option<u32>| {
            width.is_none_or(|width| (1..=crate::model::DESK_LANE_WIDTH_MAX).contains(&width))
        };
        if payload.view_width.is_none() && payload.session_width.is_none()
            || !valid(payload.view_width)
            || !valid(payload.session_width)
        {
            return refused("desk_lane_width_invalid");
        }
        let lane = &mut self.desk.stored.lane;
        let before = *lane;
        lane.view_width = payload.view_width.or(lane.view_width);
        lane.session_width = payload.session_width.or(lane.session_width);
        *lane != before
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
        let files = match payload.again {
            Some(again) if payload.files.is_empty() => self
                .desk
                .stored
                .conversation
                .items
                .iter()
                .find_map(|item| match item {
                    DeskItem::User {
                        id, attachments, ..
                    } if *id == again => Some(attachments.clone()),
                    _ => None,
                })
                .unwrap_or_default(),
            _ => payload.files,
        };
        if (text.is_empty() && files.is_empty()) || text.len() > MAX_MESSAGE_BYTES {
            return refused("message_size");
        }
        // hided names each file, so none holds the quote a prompt wraps a
        // path in (`desk_start_turn`).
        if files.iter().any(|file| file.path.contains('"')) {
            return refused("attachments");
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
        if self.desk_agent_state(agent) != DeskAgentState::Ready {
            return refused("agent_not_ready");
        }
        // While a reply is being written, or earlier messages still wait,
        // the message waits its turn with its files (letter-47148).
        if self.desk.turn.is_some() || !self.desk.stored.queued.is_empty() {
            if self.desk.stored.queued.len() >= DESK_QUEUED_MAX {
                return refused("desk_queued_full");
            }
            let id = self.desk.stored.conversation.next_id();
            self.desk.stored.queued.push(DeskQueuedMessage {
                id,
                text: text.to_owned(),
                at_ms: crate::links::now_ms(),
                mentions: payload.mentions,
                attachments: files,
            });
            self.desk.held = false;
            return true;
        }
        self.desk.held = false;
        self.desk_start_turn(
            agent,
            Some(text.to_owned()),
            text.to_owned(),
            payload.mentions,
            files,
        );
        true
    }

    /// Sends the first queued message once nothing stands before it: no
    /// turn under way or refusal to send, Desk on, its agent ready, and the
    /// process neither stopped (`다시 시작` answers that) nor refused by the
    /// agent (the operator's next send answers that). It is the one place
    /// that decides it, read after every event and every report, so no
    /// change of state is missed (`desk_event`, `desk_report`).
    fn desk_send_queued(&mut self) -> bool {
        if self.desk.stored.queued.is_empty()
            || !self.desk.stored.settings.enabled
            || self.desk.turn.is_some()
            || self.desk.refusal.is_some()
            || self.desk.held
            || self.desk.host.state == Some(DeskHostState::Stopped)
        {
            return false;
        }
        let Some(agent) = self
            .desk
            .stored
            .selection
            .agent
            .filter(|agent| self.desk_agent_state(*agent) == DeskAgentState::Ready)
        else {
            return false;
        };
        let message = self.desk.stored.queued.remove(0);
        self.desk_start_turn(
            agent,
            Some(message.text.clone()),
            message.text,
            message.mentions,
            message.attachments,
        );
        true
    }

    fn desk_turn_tag(&self) -> Option<u64> {
        self.desk.turn.as_ref().map(|turn| turn.tag)
    }

    /// The turn under way ends as `outcome` says: requests the agent made
    /// in it end with it.
    fn desk_close_turn(&mut self, outcome: TurnOutcome) {
        self.close_approvals();
        match outcome {
            TurnOutcome::Done => self.finish_turn(DeskReplyState::Done),
            TurnOutcome::Cancelled | TurnOutcome::Ended => {
                self.finish_turn(DeskReplyState::Stopped);
            }
            TurnOutcome::Failed(reason) => {
                self.finish_turn(DeskReplyState::Stopped);
                let agent = self.selected_agent();
                self.push_notice(DeskNotice::TurnFailed, agent, reason);
            }
        }
    }

    /// The message of the turn under way, which the agent never took (its
    /// sign-in), goes back first in line and out of the
    /// conversation, words, chips and files together, so nothing is lost to
    /// a refused start and nothing comes back to the composer; it may make the line one past its cap,
    /// since the cap only refuses what the operator sends.
    fn desk_requeue_untaken(&mut self) {
        let Some(message) = self.desk.turn.as_ref().and_then(|turn| turn.message) else {
            return;
        };
        let Some(DeskItem::User {
            text,
            at_ms,
            mentions,
            attachments,
            ..
        }) = self
            .desk
            .stored
            .conversation
            .items
            .iter()
            .find(|item| item.id() == message)
            .cloned()
        else {
            return;
        };
        self.desk.stored.conversation.remove(message);
        let id = self.desk.stored.conversation.next_id();
        self.desk.stored.queued.insert(
            0,
            DeskQueuedMessage {
                id,
                text,
                at_ms,
                mentions,
                attachments,
            },
        );
    }

    /// The files Desk still has to hand its agent: every waiting message's
    /// and the turn under way's. hided's keep of Desk's folder never removes
    /// them (`hided/src/attachments.rs`, `commit_named`).
    pub(crate) fn desk_files_in_use(&self) -> Vec<String> {
        let turn = self.desk.turn.as_ref().and_then(|turn| turn.message);
        let turn_files = self
            .desk
            .stored
            .conversation
            .items
            .iter()
            .filter_map(|item| match item {
                DeskItem::User {
                    id, attachments, ..
                } if Some(*id) == turn => Some(attachments),
                _ => None,
            });
        self.desk
            .stored
            .queued
            .iter()
            .map(|message| &message.attachments)
            .chain(turn_files)
            .flatten()
            .map(|file| file.path.clone())
            .collect()
    }

    /// ✕ on a queued message: it leaves the line and never goes. Its files
    /// stay in Desk's folder as a refused send's do, whose cap lets the
    /// oldest go (`hided/src/attachments.rs`, `keep_newest`).
    fn desk_queued_remove(&mut self, id: u64) -> bool {
        let before = self.desk.stored.queued.len();
        self.desk.stored.queued.retain(|message| message.id != id);
        self.desk.stored.queued.len() != before
    }

    /// Starts a turn: `shown` and its files join the conversation as the
    /// operator's message, when there is one, and the agent receives the
    /// files' paths before `sent`, each in double quotes, one space apart: a
    /// prompt, not a shell line, so a path reads as the system spells it.
    fn desk_start_turn(
        &mut self,
        agent: AdapterId,
        shown: Option<String>,
        sent: String,
        mentions: Vec<DeskMention>,
        attachments: Vec<DeskAttachment>,
    ) {
        let files: Vec<String> = attachments.iter().map(|file| file.path.clone()).collect();
        let quoted: String = files.iter().map(|path| format!("\"{path}\" ")).collect();
        let sent = format!("{quoted}{sent}");
        let node = (!files.is_empty()).then(|| self.own_node());
        let carried = (!self.desk.host.open)
            .then(|| self.desk.stored.conversation.carried())
            .flatten();
        let message = shown.map(|text| {
            let id = self.desk.stored.conversation.next_id();
            self.desk.stored.conversation.push(DeskItem::User {
                id,
                text,
                at_ms: crate::links::now_ms(),
                mentions,
                attachments,
            });
            id
        });
        self.desk.next_tag += 1;
        let tag = self.desk.next_tag;
        self.desk.turn = Some(Turn {
            tag,
            agent,
            message,
            text: String::new(),
            cut: false,
            started_at_ms: crate::links::now_ms(),
            first_text_at_ms: None,
            steps: Default::default(),
            now: None,
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
            text: sent,
            carried,
            resume,
            model: self.desk.stored.selection.model.clone(),
            effort: self.desk.stored.selection.effort.clone(),
            placement: match self.desk.stored.settings.location {
                DeskLocation::Desk => hide_desk::Placement::PerTask,
                DeskLocation::Worktree => hide_desk::Placement::Worktree,
                DeskLocation::Checkout => hide_desk::Placement::Checkout,
            },
            files,
            node,
        }));
        self.desk.host.state = Some(if self.desk.host.open {
            DeskHostState::Thinking
        } else {
            DeskHostState::Starting
        });
        self.desk.host.detail = None;
        self.compose_desk_live();
    }

    fn desk_stop(&mut self) -> bool {
        if self.desk.turn.is_none() {
            return false;
        }
        // The operator stopped the turn: a plain refusal it ended on is
        // not told, since the agent is not to go on.
        if self
            .desk
            .refusal
            .as_ref()
            .is_some_and(|refusal| refusal.text.is_none())
        {
            self.desk.refusal = None;
        }
        // The agent answers each request it is waiting on as cancelled.
        let cancelled = json!({"outcome": {"outcome": "cancelled"}});
        for approval in std::mem::take(&mut self.desk.approvals) {
            self.order(Order::Answer {
                rpc_id: approval.rpc_id,
                outcome: cancelled.clone(),
            });
            self.set_approval(approval.item, DeskApprovalState::Closed, None, None);
        }
        self.order(Order::Cancel);
        true
    }

    /// The operator's answer to a card (D-92): the agent's option they
    /// pressed goes back to the agent as it is. A refusing one may carry
    /// `거절…`'s line, which stops the turn so the line reaches Desk as the
    /// next message.
    fn desk_answer(&mut self, payload: ApprovalPayload) -> bool {
        let Some(index) = self
            .desk
            .approvals
            .iter()
            .position(|approval| approval.item == payload.id)
        else {
            return refused("approval_unknown");
        };
        let options = &self.desk.approvals[index].options;
        let pressed = match payload.option.as_deref() {
            Some(id) => match options.iter().find(|option| option.id == id) {
                Some(option) => Some(option.clone()),
                None => return refused("approval_option"),
            },
            None if options.iter().any(|option| option.kind.rejects()) => {
                return refused("approval_option");
            }
            None => None,
        };
        let rejects = pressed.as_ref().is_none_or(|option| option.kind.rejects());
        // One line: the box takes no newline, and a pasted one is a space.
        let text = payload
            .text
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|text| !text.is_empty());
        if text
            .as_ref()
            .is_some_and(|text| text.len() > MAX_REFUSAL_BYTES || !rejects)
        {
            return refused("approval_text");
        }
        let approval = self.desk.approvals.remove(index);
        self.order(Order::Answer {
            rpc_id: approval.rpc_id,
            outcome: outcome(payload.option.as_deref()),
        });
        let state = if rejects {
            DeskApprovalState::Rejected
        } else {
            DeskApprovalState::Allowed
        };
        self.set_approval(payload.id, state, pressed, text.clone());
        if rejects {
            // A line stops the turn so it reaches Desk as the next message;
            // a plain refusal leaves the turn to the agent, and is sent only
            // if the agent ends its turn without a word after it. The last
            // plain refusal is the one told, and a line is never replaced.
            if text.is_some() {
                self.order(Order::Cancel);
            }
            if text.is_some()
                || self
                    .desk
                    .refusal
                    .as_ref()
                    .is_none_or(|refusal| refusal.text.is_none())
            {
                let said = self.desk.turn.as_ref().map_or(0, |turn| turn.text.len());
                self.desk.refusal = Some(Refusal {
                    command: approval.command,
                    kind: approval.kind,
                    text,
                    said,
                });
            }
        }
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
        // A new conversation starts with nothing of the last one waiting.
        self.desk.stored.queued.clear();
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
        self.forget_stopped();
        self.order(Order::Check);
        self.desk.held = false;
        true
    }

    /// `멈췄어요 · 다시 시작` is answered once the process is restarted or a
    /// session opens again: the line and its button leave the conversation,
    /// so no stale `다시 시작` stays live under the new process.
    fn forget_stopped(&mut self) {
        let stopped: Vec<u64> = self
            .desk
            .stored
            .conversation
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    DeskItem::Notice {
                        notice: DeskNotice::HostStopped,
                        ..
                    }
                )
            })
            .map(DeskItem::id)
            .collect();
        for id in stopped {
            self.desk.stored.conversation.remove(id);
        }
    }

    /// Ends the Desk process from the room: the reply under way stops, and
    /// each waiting card closes as a refusal.
    fn end_desk_process(&mut self) {
        self.order(Order::End);
        self.close_approvals();
        self.forget_refusal();
        self.finish_turn(DeskReplyState::Stopped);
        self.desk.host = Host {
            state: Some(DeskHostState::Idle),
            ..Host::default()
        };
    }

    fn close_approvals(&mut self) {
        for approval in std::mem::take(&mut self.desk.approvals) {
            self.set_approval(approval.item, DeskApprovalState::Closed, None, None);
        }
    }

    /// The Desk process ended: a refusal no turn will read, plain or with a
    /// line, is dropped (a line stays on the card that keeps it).
    fn forget_refusal(&mut self) {
        self.desk.refusal = None;
    }

    fn desk_select(&mut self, payload: SelectPayload) -> bool {
        let selection = self.desk.stored.selection.clone();
        if let Some(agent) = payload.agent
            && Some(agent) != selection.agent
        {
            // Only an agent the bar offers is chosen: switched on in the
            // catalog (D-79).
            if self.desk_agent_state(agent) == DeskAgentState::Off {
                return refused("agent_not_offered");
            }
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
        // The next request is answered from it; a waiting card stays.
        if let Some(permission) = payload.permission {
            settings.permission = permission;
        }
        if self.desk.stored.selection.agent.is_none() {
            self.desk.stored.selection.agent = settings.default_agent;
        }
        let enabled = self.desk.stored.settings.enabled;
        // A raised cap frees places: the queue fills them in order.
        if enabled && self.desk.stored.settings.cap > before.cap {
            self.desk_start_queued();
        }
        if enabled != before.enabled {
            if enabled {
                // Every adapter is installed now, not when it is chosen
                // (D-80), and the sign-ins are read (D-81).
                self.order(Order::Check);
                self.ask_ai_catalog_read();
                self.desk_start_queued();
            } else {
                // Off ends the process and its turn; the sessions it started
                // and the conversation stay.
                if self.desk.turn.is_some() {
                    self.order(Order::Cancel);
                }
                self.end_desk_process();
                // An open waiting for a window ends with Desk, so turning it
                // on later shows nothing it opened before.
                self.desk.opens.clear();
                // Every adapter download under way ends with Desk, not later
                // in a folder nobody watches (engineering principle 14).
                self.order(Order::Off);
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

    fn set_approval(
        &mut self,
        item: u64,
        state: DeskApprovalState,
        pressed: Option<permission::Choice>,
        line: Option<String>,
    ) {
        for entry in &mut self.desk.stored.conversation.items {
            if let DeskItem::Approval {
                id,
                state: current,
                chosen,
                answer,
                refusal,
                ..
            } = entry
                && *id == item
            {
                *current = state;
                *chosen = pressed.as_ref().map(|option| option.kind);
                *answer = pressed.as_ref().map(|option| option.name.clone());
                if line.is_some() {
                    *refusal = line.clone();
                }
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
            // Each chip's token is kept as the chip names it, a quoted name without its quotes.
            let text = hide_desk::unquoted_mentions(&turn.text);
            let mentions = hide_desk::session_mentions(&text)
                .into_iter()
                .map(|(pane_id, name)| crate::model::DeskSessionChip {
                    pane_id,
                    name,
                    agent_kind: None,
                    mark: None,
                })
                .collect();
            let missions = hide_desk::mission_mentions(&text)
                .into_iter()
                .map(|(id, title)| crate::model::DeskMissionMention { id, title })
                .collect();
            self.desk.stored.conversation.push(DeskItem::Reply {
                id,
                agent: turn.agent,
                text,
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
                missions,
            });
            // The sessions it names get their cards (D-86).
            self.compose_desk_jobs();
        }
        self.compose_desk_live();
    }

    /// What the worker learned, applied under the lock; then the one place
    /// that decides whether the first queued message goes.
    fn desk_report(&mut self, report: Report) -> bool {
        let changed = self.desk_apply_report(report);
        if self.desk_send_queued() {
            self.save_desk();
            self.compose_desk();
            return true;
        }
        changed
    }

    fn desk_apply_report(&mut self, report: Report) -> bool {
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
            Report::Installing { adapter, asked } => {
                self.desk
                    .agents
                    .insert(adapter, (DeskAgentState::Preparing, None));
                // The installs Desk turned on starts show in the bar alone;
                // a `다시 받기` the operator pressed is answered where it was
                // pressed.
                if asked {
                    self.push_notice(DeskNotice::Preparing, Some(adapter), None);
                    self.save_desk();
                }
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
                self.forget_stopped();
                self.desk.held = false;
                self.desk.sign_in_refused.remove(&adapter);
                if resume_failed {
                    self.push_notice(DeskNotice::ResumeFailed, Some(adapter), None);
                }
                self.save_desk();
            }
            Report::OpenFailed {
                adapter,
                error,
                turn,
            } => {
                // The agent never took the turn's message: it goes back first
                // in line and the turn ends, and `held` keeps the line until
                // the operator sends again or restarts. A turn started since,
                // by `다시 시작` or a new conversation, is not this one.
                self.desk.held = true;
                if turn.is_some() && self.desk_turn_tag() == turn {
                    self.desk_requeue_untaken();
                    self.desk_close_turn(TurnOutcome::Cancelled);
                }
                if error == OpenError::Login {
                    self.desk.sign_in_refused.insert(adapter);
                }
                // A sign-in the agent asked for leaves its state as it is: the
                // bar's sign-in word is Settings' read of the agent (D-81).
                let (state, notice, detail) = match error {
                    OpenError::Login => (
                        self.desk
                            .agents
                            .get(&adapter)
                            .map_or(DeskAgentState::Ready, |(state, _)| *state),
                        DeskNotice::LoginNeeded,
                        Some(adapter.login_command().to_owned()),
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
                let kept = if notice == DeskNotice::LoginNeeded {
                    self.desk
                        .agents
                        .get(&adapter)
                        .and_then(|(_, kept)| kept.clone())
                } else {
                    detail.clone()
                };
                self.desk.agents.insert(adapter, (state, kept));
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
            Report::Step { tag, step } => {
                let Some(turn) = self.desk.turn.as_mut().filter(|turn| turn.tag == tag) else {
                    return false;
                };
                let now = step.map(Into::into);
                if turn.now == now {
                    return false;
                }
                turn.now = now;
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
            Report::AttachmentsGone { tag } => {
                // No agent received the message, and sending it again would
                // meet the same gone file: it stays in the conversation, its
                // words kept, with a line under it that it was not sent, and
                // the next waiting message goes once the worker's next report
                // ends the turn. The operator attaches the file again in a
                // new message.
                let Some(agent) = self
                    .desk
                    .turn
                    .as_ref()
                    .filter(|turn| turn.tag == tag)
                    .map(|turn| turn.agent)
                else {
                    return false;
                };
                self.push_notice(DeskNotice::AttachmentsGone, Some(agent), None);
                self.save_desk();
            }
            Report::TurnEnded { tag, outcome } => {
                if self.desk.turn.as_ref().is_none_or(|turn| turn.tag != tag) {
                    return false;
                }
                // Whether the agent ended its turn on a plain refusal
                // without a word after it: OpenCode's turn ends cancelled
                // there and Antigravity's as answered, while an agent that
                // went on said so itself.
                let silent = matches!(outcome, TurnOutcome::Done | TurnOutcome::Cancelled)
                    && self.desk.turn.as_ref().is_some_and(|turn| {
                        self.desk
                            .refusal
                            .as_ref()
                            .is_some_and(|refusal| !turn.cut && turn.text.len() == refusal.said)
                    });
                self.desk_close_turn(outcome);
                if self.desk.host.open {
                    self.desk.host.state = Some(DeskHostState::Ready);
                }
                // A message the operator queued speaks for them after a
                // plain refusal, so Desk adds no words of its own before it.
                if let Some(refusal) = self.desk.refusal.take()
                    && (refusal.text.is_some() || (silent && self.desk.stored.queued.is_empty()))
                    && let Some(agent) = self.selected_agent()
                {
                    let sent = hide_desk::refusal_message(
                        refusal.kind,
                        &refusal.command,
                        refusal.text.as_deref(),
                    );
                    self.desk_start_turn(agent, refusal.text, sent, Vec::new(), Vec::new());
                }
                self.save_desk();
            }
            Report::Permission {
                rpc_id,
                options,
                card,
                hide_line,
            } => {
                let allow_once = permission::allow_once(&options).map(str::to_owned);
                let answered = match self.desk.stored.settings.permission {
                    DeskPermission::AllowAll => true,
                    DeskPermission::Agent => hide_line,
                };
                if answered && let Some(allow_once) = allow_once {
                    self.order(Order::Answer {
                        rpc_id,
                        outcome: outcome(Some(&allow_once)),
                    });
                    return false;
                }
                let id = self.desk.stored.conversation.next_id();
                self.desk.stored.conversation.push(DeskItem::Approval {
                    id,
                    card: match card.kind {
                        permission::CardKind::Command => DeskCardKind::Command,
                        permission::CardKind::Edit => DeskCardKind::Edit,
                        permission::CardKind::Read => DeskCardKind::Read,
                        permission::CardKind::Other => DeskCardKind::Other,
                    },
                    command: card.command.clone(),
                    folder: card.folder,
                    reason: card.reason,
                    options: options.clone(),
                    state: DeskApprovalState::Waiting,
                    chosen: None,
                    answer: None,
                    refusal: None,
                });
                self.desk.approvals.push(Approval {
                    item: id,
                    rpc_id,
                    options,
                    command: card.command,
                    kind: card.kind,
                });
                self.save_desk();
            }
            Report::Choices { adapter, choices } => {
                if !self.learn_choices(adapter, &choices) {
                    return false;
                }
                self.save_desk();
            }
            Report::Ended {
                unexpected,
                detail,
                turn,
            } => {
                // The turn that ends with the process ends as the report says.
                // A turn started since, as `다시 시작` starts the next queued
                // message beside the old process's end, is not that
                // process's: the worker sends it to the next one.
                if let Some(TurnEnd { tag, outcome }) = turn
                    && self.desk_turn_tag() == Some(tag)
                {
                    self.desk_close_turn(outcome);
                }
                self.close_approvals();
                self.forget_refusal();
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
                    if self.desk.turn.is_none()
                        && self.desk.host.state != Some(DeskHostState::Stopped)
                    {
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
        let catalog = self.desk_catalog();
        self.desk.catalog = catalog.clone();
        let desk = &self.desk;
        let selected = desk.stored.selection.agent;
        let agents = catalog
            .into_iter()
            .filter(|agent| agent.enabled || Some(agent.id) == selected)
            .map(|agent| {
                let id = agent.id;
                let (state, detail) = if agent.enabled {
                    desk.agents
                        .get(&id)
                        .cloned()
                        .unwrap_or((DeskAgentState::Checking, None))
                } else {
                    (DeskAgentState::Off, None)
                };
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
                    signed_in: if desk.sign_in_refused.contains(&id) {
                        Some(false)
                    } else {
                        agent.signed_in
                    },
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
                queued: desk.stored.queued.clone(),
                queued_max: DESK_QUEUED_MAX,
                queued_full: desk.stored.queued.len() >= DESK_QUEUED_MAX,
                queued_held: desk.held && !desk.stored.queued.is_empty(),
            },
            approvals_waiting: desk.approvals.len(),
            settings: desk.stored.settings,
            lane: desk.stored.lane,
            opens: desk.opens.snapshot(),
        }));
    }

    /// Desk's agents as the catalog says them now, in its order.
    fn desk_catalog(&self) -> Vec<CatalogAgent> {
        let node = self.node.as_str();
        AdapterId::all()
            .map(|id| {
                let row = id.catalog();
                CatalogAgent {
                    id,
                    enabled: self
                        .catalog_kit_agent(node, row.id)
                        .is_some_and(|agent| agent.enabled),
                    signed_in: self.catalog_provider(row.id).and_then(signed_in),
                }
            })
            .collect()
    }

    /// This machine's kit or Hide AI's read of its agents moved: the bar is
    /// composed again when what it says of Desk's agents moved.
    pub(in crate::runtime) fn desk_catalog_moved(&mut self) {
        if self.desk.worker.is_some() && self.desk_catalog() != self.desk.catalog {
            self.compose_desk();
        }
    }

    /// Whether Desk can talk to `agent` now: `off` when it is not switched on
    /// in the catalog, else what the worker last found.
    fn desk_agent_state(&self, agent: AdapterId) -> DeskAgentState {
        let enabled = self
            .catalog_kit_agent(self.node.as_str(), agent.catalog().id)
            .is_some_and(|row| row.enabled);
        if !enabled {
            return DeskAgentState::Off;
        }
        self.desk
            .agents
            .get(&agent)
            .map_or(DeskAgentState::Checking, |(state, _)| *state)
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
            step: turn.and_then(|turn| turn.now),
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

    /// Desk installed and on, its process not running a session.
    pub(in crate::runtime) fn close_test_desk(&mut self) {
        self.desk.host.open = false;
    }
}

/// The answer that picks `option`, or cancels when the request offered none.
fn outcome(option: Option<&str>) -> Value {
    match option {
        Some(option) => json!({"outcome": {"outcome": "selected", "optionId": option}}),
        None => json!({"outcome": {"outcome": "cancelled"}}),
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
    use hide_desk::permission::ChoiceKind;

    /// A runtime whose Desk worker is a channel the test reads, with every
    /// agent switched on in the catalog and ready.
    pub(super) fn runtime(root: &std::path::Path) -> (Runtime, Receiver<Order>) {
        let state = root.join("state");
        hide_platform::fs::private::create_dir_all(&state).unwrap();
        let options: crate::model::CoreOptions = serde_json::from_value(json!({
            "schema_version": crate::model::SCHEMA_VERSION, "node_id": "test-node", "home": root,
            "app_state_path": state.join("app.json"),
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
        kit_on(
            &mut runtime,
            &AdapterId::all()
                .map(|agent| agent.catalog().id)
                .collect::<Vec<_>>(),
        );
        runtime.desk_report(Report::Availability(
            AdapterId::all()
                .map(|agent| (agent, DeskAgentState::Ready, None))
                .collect(),
        ));
        (runtime, received)
    }

    /// Switches the agents `ids` names on in this machine's kit, the others
    /// off, as Settings › Agents does.
    fn kit_on(runtime: &mut Runtime, ids: &[&str]) {
        let node = runtime.node.as_str().to_owned();
        let mut kit = runtime.kit_state(&node);
        kit.agents = hide_agent_adapter::ADAPTERS
            .iter()
            .map(|adapter| crate::model::KitAgentSnapshot {
                id: adapter.id.to_owned(),
                label: adapter.label.to_owned(),
                availability: hide_kit::Availability::Available,
                enabled: ids.contains(&adapter.id),
                chosen: false,
                skill: crate::model::KitPieceSnapshot {
                    state: hide_kit::ComponentState::Installed,
                    why: None,
                    location: None,
                },
                hook: None,
                partial: false,
                features: Vec::new(),
                sessions: None,
                update: None,
                doc_url: String::new(),
            })
            .collect();
        runtime.set_kit_state(&node, kit);
    }

    /// Hide AI's read of these agents: `(agent, state, sign-in checked)`.
    fn read(rows: &[(&str, &str, bool)]) -> crate::model::BackgroundAiSnapshot {
        let mut snapshot = crate::model::BackgroundAiSnapshot::unread();
        for (agent, state, checked) in rows {
            let row = snapshot
                .providers
                .iter_mut()
                .find(|row| row.agent == *agent && row.device_id.is_none())
                .unwrap_or_else(|| panic!("no Hide AI row for {agent}"));
            row.state = (*state).to_owned();
            row.login_checked = *checked;
        }
        snapshot
    }

    pub(super) fn event(runtime: &mut Runtime, kind: &str, payload: serde_json::Value) -> bool {
        runtime.desk_event(DeskEvent::decode(kind, payload).expect(kind))
    }

    pub(super) fn sent(received: &Receiver<Order>) -> worker::Send {
        loop {
            match received.try_recv().expect("no Send order") {
                Order::Send(send) => return send,
                _ => continue,
            }
        }
    }

    pub(super) fn items(runtime: &Runtime) -> Vec<DeskItem> {
        runtime
            .snapshot
            .desk
            .as_ref()
            .unwrap()
            .conversation
            .items
            .clone()
    }

    pub(super) fn opened(runtime: &mut Runtime, adapter: AdapterId) {
        runtime.desk_report(Report::Opened {
            adapter,
            session_id: format!("{}-session", adapter.as_str()),
            resume_failed: false,
            choices: Choices::default(),
        });
    }

    /// claude-agent-acp 0.99.1's options for a Bash call, in its order.
    fn claude_options() -> Vec<hide_desk::permission::Choice> {
        serde_json::from_value(json!([
            {"id": "allow-once", "kind": "allow_once", "name": "Yes"},
            {"id": "allow-always", "kind": "allow_always",
                "name": "Yes, and don't ask again for touch commands"},
            {"id": "reject-once", "kind": "reject_once", "name": "No"},
        ]))
        .unwrap()
    }

    /// What Desk's worker reports when the agent asks to run `line` and
    /// offers `options`.
    fn permission_with(line: &str, options: Vec<hide_desk::permission::Choice>) -> Report {
        let action = hide_desk::permission::Action::Command {
            command: line.into(),
            folder: None,
            reason: None,
        };
        let place = hide_desk::permission::Place {
            hide: std::path::Path::new("/app/hide"),
            cwd: "/state/desk/work",
            home: Some("/home/u"),
        };
        Report::Permission {
            rpc_id: json!(7),
            options,
            card: hide_desk::permission::card(&action, &place),
            hide_line: hide_desk::permission::hide_line(line, &place),
        }
    }

    /// The agent asks to run `line`: the card it makes, or `None` when Hide
    /// answered it.
    fn request(runtime: &mut Runtime, line: &str) -> Option<u64> {
        request_with(runtime, line, claude_options())
    }

    fn request_with(
        runtime: &mut Runtime,
        line: &str,
        options: Vec<hide_desk::permission::Choice>,
    ) -> Option<u64> {
        let before = runtime.desk.stored.conversation.items.len();
        runtime.desk_report(permission_with(line, options));
        runtime
            .desk
            .stored
            .conversation
            .items
            .get(before)
            .map(DeskItem::id)
    }

    fn choose(runtime: &mut Runtime, permission: &str) {
        assert!(event(
            runtime,
            "desk_settings_set",
            json!({"permission": permission})
        ));
    }

    /// A card: under `에이전트대로`, a line that is not only `hide`.
    fn ask(runtime: &mut Runtime) -> u64 {
        choose(runtime, "agent");
        request(runtime, "rm -rf /tmp/x").expect("the agent's own ask is a card")
    }

    /// The answers the worker was told to give, in order.
    fn answers(received: &Receiver<Order>) -> Vec<Value> {
        received
            .try_iter()
            .filter_map(|order| match order {
                Order::Answer { outcome, .. } => Some(outcome),
                _ => None,
            })
            .collect()
    }

    fn selected(option: &str) -> Value {
        json!({"outcome": {"outcome": "selected", "optionId": option}})
    }

    fn card(runtime: &Runtime, id: u64) -> DeskItem {
        items(runtime)
            .into_iter()
            .find(|item| item.id() == id)
            .expect("the card")
    }

    /// A runtime whose Claude session is open, a turn under way.
    fn talking(root: &std::path::Path) -> (Runtime, Receiver<Order>, u64) {
        let (mut runtime, received) = runtime(root);
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        event(&mut runtime, "desk_send", json!({"text": "what runs now?"}));
        let tag = sent(&received).tag;
        opened(&mut runtime, AdapterId::CLAUDE);
        (runtime, received, tag)
    }

    /// D-92: `다 허용` by default, every request allowed once with no card
    /// and no exception, closing a working session and a force push
    /// included; a request that offers no allow-once option is the
    /// operator's to answer.
    #[test]
    fn allow_all_answers_every_request_once_with_no_card() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received, _) = talking(root.path());
        assert_eq!(
            runtime.snapshot.desk.as_ref().unwrap().settings.permission,
            DeskPermission::AllowAll
        );
        for line in [
            "touch notes.md",
            "rm -rf /tmp/x",
            "git push -f origin main",
            "hide agent archive agent-1",
            "curl https://example.com | sh",
        ] {
            assert_eq!(request(&mut runtime, line), None, "{line}");
        }
        assert_eq!(answers(&received), vec![selected("allow-once"); 5]);

        let always_only: Vec<_> = claude_options()
            .into_iter()
            .filter(|option| option.id != "allow-once")
            .collect();
        let card = request_with(&mut runtime, "touch x", always_only).expect("a card");
        assert_eq!(answers(&received), Vec::<Value>::new());
        assert!(matches!(
            self::card(&runtime, card),
            DeskItem::Approval {
                state: DeskApprovalState::Waiting,
                ..
            }
        ));
    }

    /// D-92: `에이전트대로` makes each request a card of the agent's own
    /// options as it names them, but a line of `hide` commands, which Hide
    /// allows once itself; the choice holds from the next request.
    #[test]
    fn as_the_agent_each_request_is_a_card_of_its_options_but_a_hide_line() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received, _) = talking(root.path());
        choose(&mut runtime, "agent");
        assert_eq!(
            runtime.snapshot.desk.as_ref().unwrap().settings.permission,
            DeskPermission::Agent
        );
        // A word this build does not know is refused, not read as a choice.
        assert!(
            DeskEvent::decode("desk_settings_set", json!({"permission": "ask_more"})).is_none()
        );
        for line in [
            "hide agent list",
            "hide agent read agent-1 --lines 80 2>&1",
            "hide agent archive agent-1 && hide agent list",
        ] {
            assert_eq!(request(&mut runtime, line), None, "{line}");
        }
        assert_eq!(answers(&received), vec![selected("allow-once"); 3]);
        // What reads its output runs another program: the agent's to ask.
        assert!(request(&mut runtime, "hide agent list | head -5").is_some());

        let waiting = request(&mut runtime, "touch notes.md").expect("the agent asked");
        assert_eq!(answers(&received), Vec::<Value>::new());
        assert_eq!(
            card(&runtime, waiting),
            DeskItem::Approval {
                id: waiting,
                card: DeskCardKind::Command,
                command: "touch notes.md".into(),
                folder: Some("/state/desk/work".into()),
                reason: None,
                options: claude_options(),
                state: DeskApprovalState::Waiting,
                chosen: None,
                answer: None,
                refusal: None,
            }
        );

        // Back to `다 허용`: the card waits for its answer, and the next
        // request runs.
        choose(&mut runtime, "allow_all");
        assert_eq!(request(&mut runtime, "touch other.md"), None);
        assert!(matches!(
            card(&runtime, waiting),
            DeskItem::Approval {
                state: DeskApprovalState::Waiting,
                ..
            }
        ));
    }

    /// The operator's press goes to the agent as that option, whatever its
    /// kind; Hide keeps nothing of it, so the next request is a card again.
    #[test]
    fn the_option_the_operator_presses_goes_to_the_agent_as_it_is() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received, _) = talking(root.path());
        let id = ask(&mut runtime);
        assert_eq!(runtime.snapshot.desk.as_ref().unwrap().approvals_waiting, 1);
        assert!(!event(
            &mut runtime,
            "desk_approval",
            json!({"id": id, "option": "allow-forever"})
        ));
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": id, "option": "allow-always"})
        ));
        assert_eq!(answers(&received), [selected("allow-always")]);
        assert!(matches!(
            card(&runtime, id),
            DeskItem::Approval {
                state: DeskApprovalState::Allowed,
                chosen: Some(ChoiceKind::AllowAlways),
                answer: Some(name),
                ..
            } if name == "Yes, and don't ask again for touch commands"
        ));
        // Answered once only.
        assert!(!event(
            &mut runtime,
            "desk_approval",
            json!({"id": id, "option": "reject-once"})
        ));
        assert!(request(&mut runtime, "rm -rf /tmp/x").is_some());

        let once = request(&mut runtime, "touch y").unwrap();
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": once, "option": "allow-once"})
        ));
        assert!(matches!(
            card(&runtime, once),
            DeskItem::Approval {
                state: DeskApprovalState::Allowed,
                chosen: Some(ChoiceKind::AllowOnce),
                ..
            }
        ));
    }

    /// An option Hide has no word for keeps the agent's own name on the
    /// answered line, and `거절…` ends a card that offers no rejecting
    /// option as ACP's `cancelled`, which a card offering one refuses.
    #[test]
    fn a_card_with_no_rejecting_option_is_ended_by_refusing_it_as_cancelled() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received, _) = talking(root.path());
        choose(&mut runtime, "agent");
        let plan: Vec<hide_desk::permission::Choice> = serde_json::from_value(json!([
            {"id": "plan", "kind": "other", "name": "Keep planning"},
            {"id": "go", "kind": "allow_once", "name": "Yes, start"},
        ]))
        .unwrap();
        let other = request_with(&mut runtime, "rm -rf /tmp/x", plan.clone()).unwrap();
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": other, "option": "plan"})
        ));
        assert!(matches!(
            card(&runtime, other),
            DeskItem::Approval { chosen: Some(ChoiceKind::Other), answer: Some(name), .. }
                if name == "Keep planning"
        ));

        let ended = request_with(&mut runtime, "rm -rf /tmp/x", plan).unwrap();
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": ended, "option": null})
        ));
        assert_eq!(
            answers(&received),
            [
                selected("plan"),
                json!({"outcome": {"outcome": "cancelled"}})
            ]
        );
        assert!(matches!(
            card(&runtime, ended),
            DeskItem::Approval {
                state: DeskApprovalState::Rejected,
                chosen: None,
                answer: None,
                ..
            }
        ));

        let offers = request(&mut runtime, "rm -rf /tmp/x").unwrap();
        assert!(!event(
            &mut runtime,
            "desk_approval",
            json!({"id": offers, "option": null})
        ));
        assert!(matches!(
            card(&runtime, offers),
            DeskItem::Approval {
                state: DeskApprovalState::Waiting,
                ..
            }
        ));
    }

    #[test]
    fn a_refusal_with_a_line_stops_the_turn_and_sends_the_line_to_desk() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received, tag) = talking(root.path());
        let id = ask(&mut runtime);
        // The line is one line, and only a refusing option carries one.
        assert!(!event(
            &mut runtime,
            "desk_approval",
            json!({"id": id, "option": "allow-once", "text": "why"})
        ));
        assert!(!event(
            &mut runtime,
            "desk_approval",
            json!({"id": id, "option": "reject-once", "text": "x".repeat(MAX_REFUSAL_BYTES + 1)})
        ));
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": id, "option": "reject-once", "text": "  끝날 때까지\n기다렸다가 지워줘 "})
        ));
        let orders: Vec<Order> = received.try_iter().collect();
        assert!(orders.iter().any(|order| matches!(order,
            Order::Answer { outcome, .. } if *outcome == selected("reject-once"))));
        assert!(orders.iter().any(|order| matches!(order, Order::Cancel)));
        let line = "끝날 때까지 기다렸다가 지워줘";
        assert!(matches!(
            card(&runtime, id),
            DeskItem::Approval { state: DeskApprovalState::Rejected, refusal: Some(refusal), .. }
                if refusal == line
        ));

        runtime.desk_report(Report::TurnEnded {
            tag,
            outcome: TurnOutcome::Cancelled,
        });
        let send = sent(&received);
        assert_eq!(
            send.text,
            hide_desk::refusal_message(
                hide_desk::permission::CardKind::Command,
                "rm -rf /tmp/x",
                Some(line)
            )
        );
        assert!(matches!(
            items(&runtime).last(),
            Some(DeskItem::User { text, .. }) if text == line
        ));
    }

    #[test]
    fn a_plain_refusal_lets_the_turn_go_on() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received, tag) = talking(root.path());
        said(&mut runtime, tag, "Deleting it now.");
        let id = ask(&mut runtime);
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": id, "option": "reject-once", "text": "   "})
        ));
        let orders: Vec<Order> = received.try_iter().collect();
        assert!(!orders.iter().any(|order| matches!(order, Order::Cancel)));
        // The agent says the refusal itself, as Claude Code, Pi and omp do.
        said(
            &mut runtime,
            tag,
            " It was refused, so nothing was deleted.",
        );
        runtime.desk_report(Report::TurnEnded {
            tag,
            outcome: TurnOutcome::Done,
        });
        assert!(
            !received
                .try_iter()
                .any(|order| matches!(order, Order::Send(_)))
        );
    }

    /// OpenCode ends its turn on a refusal that carries no words (its ACP
    /// reply has no place for them): Desk tells it, so it says so and goes
    /// on as an agent that went on itself would, with no message shown as
    /// the operator's.
    #[test]
    fn a_plain_refusal_the_agent_ended_its_turn_on_is_sent_to_it() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received, tag) = talking(root.path());
        let id = ask(&mut runtime);
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": id, "option": "reject-once"})
        ));
        let orders: Vec<Order> = received.try_iter().collect();
        assert!(!orders.iter().any(|order| matches!(order, Order::Cancel)));
        let shown = items(&runtime).len();
        runtime.desk_report(Report::TurnEnded {
            tag,
            outcome: TurnOutcome::Cancelled,
        });
        let send = sent(&received);
        // It names the refused call and forbids doing it another way.
        assert!(send.text.contains("shell command"), "{}", send.text);
        assert!(send.text.contains("rm -rf /tmp/x"), "{}", send.text);
        assert!(
            send.text.ends_with(hide_desk::REFUSED_DO_NOT_RETRY),
            "{}",
            send.text
        );
        assert_eq!(items(&runtime).len(), shown);
        assert!(matches!(
            card(&runtime, id),
            DeskItem::Approval {
                state: DeskApprovalState::Rejected,
                refusal: None,
                ..
            }
        ));

        // Its answer ends the turn Desk started, and nothing more goes.
        runtime.desk_report(Report::TurnEnded {
            tag: send.tag,
            outcome: TurnOutcome::Done,
        });
        assert!(
            !received
                .try_iter()
                .any(|order| matches!(order, Order::Send(_)))
        );
    }

    /// Antigravity answers its turn as ended after a refusal without a
    /// word about it, its words before the call aside: Desk tells it too.
    #[test]
    fn a_plain_refusal_the_agent_answered_its_turn_after_without_a_word_is_sent_to_it() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received, tag) = talking(root.path());
        said(&mut runtime, tag, "Running the removal directly.");
        let id = ask(&mut runtime);
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": id, "option": "reject-once"})
        ));
        runtime.desk_report(Report::TurnEnded {
            tag,
            outcome: TurnOutcome::Done,
        });
        let send = sent(&received);
        assert!(send.text.contains("rm -rf /tmp/x"), "{}", send.text);
        assert!(
            send.text.ends_with(hide_desk::REFUSED_DO_NOT_RETRY),
            "{}",
            send.text
        );
    }

    /// A message the operator queued during the turn goes after a plain
    /// refusal the agent was silent on, with no words of Desk's first.
    #[test]
    fn a_queued_message_goes_instead_of_a_plain_refusal_s_notice() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received, tag) = talking(root.path());
        let id = ask(&mut runtime);
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": id, "option": "reject-once"})
        ));
        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "clean only dist"})
        ));
        runtime.desk_report(Report::TurnEnded {
            tag,
            outcome: TurnOutcome::Done,
        });
        assert_eq!(sent(&received).text, "clean only dist");
    }

    /// A line the operator refused with is not replaced by a plain refusal
    /// of a card that waited beside it.
    #[test]
    fn a_refusal_s_line_outlives_a_plain_refusal_after_it() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received, tag) = talking(root.path());
        let first = ask(&mut runtime);
        let second = request(&mut runtime, "rm -rf /tmp/y").expect("it asks");
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": first, "option": "reject-once", "text": "keep tmp"})
        ));
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": second, "option": "reject-once"})
        ));
        runtime.desk_report(Report::TurnEnded {
            tag,
            outcome: TurnOutcome::Cancelled,
        });
        let send = sent(&received);
        assert!(send.text.contains("rm -rf /tmp/x"), "{}", send.text);
        assert!(send.text.ends_with("keep tmp"), "{}", send.text);
    }

    /// A reply already at its cap cannot show the words written after a
    /// refusal, so it does not read as silent.
    #[test]
    fn a_reply_at_its_cap_never_reads_as_silent_after_a_refusal() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received, tag) = talking(root.path());
        said(&mut runtime, tag, &"x".repeat(MAX_REPLY_BYTES + 1));
        let id = ask(&mut runtime);
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": id, "option": "reject-once"})
        ));
        said(&mut runtime, tag, "It was refused.");
        runtime.desk_report(Report::TurnEnded {
            tag,
            outcome: TurnOutcome::Done,
        });
        assert!(
            !received
                .try_iter()
                .any(|order| matches!(order, Order::Send(_)))
        );
    }

    /// The agent writes `text` into the turn's reply.
    fn said(runtime: &mut Runtime, tag: u64, text: &str) {
        runtime.desk_report(Report::Text {
            tag,
            text: text.into(),
        });
    }

    #[test]
    fn a_stop_after_a_plain_refusal_tells_the_agent_nothing() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received, tag) = talking(root.path());
        let id = ask(&mut runtime);
        assert!(event(
            &mut runtime,
            "desk_approval",
            json!({"id": id, "option": "reject-once"})
        ));
        assert!(event(&mut runtime, "desk_stop", json!({})));
        runtime.desk_report(Report::TurnEnded {
            tag,
            outcome: TurnOutcome::Cancelled,
        });
        assert!(
            !received
                .try_iter()
                .any(|order| matches!(order, Order::Send(_)))
        );
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
            ("desk_lane", json!({"view_width": 640})),
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
        opened(&mut runtime, AdapterId::CLAUDE);
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
        assert!(orders.iter().any(|order| matches!(order, Order::Off)));
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
        assert_eq!(send.adapter, AdapterId::CLAUDE);
        assert_eq!(send.text, "what runs now?");
        assert_eq!(send.resume, None);

        opened(&mut runtime, AdapterId::CLAUDE);
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
        assert_eq!(live.step, None);
        // The step the worker tells shows until it tells another or none.
        let step = |runtime: &Runtime| runtime.snapshot.desk_live.as_ref().unwrap().step;
        let files = Some(hide_desk::permission::Step::Files);
        assert!(runtime.desk_report(Report::Step {
            tag: send.tag,
            step: files,
        }));
        assert_eq!(step(&runtime), Some(crate::model::DeskStepKind::Files));
        assert!(!runtime.desk_report(Report::Step {
            tag: send.tag,
            step: files,
        }));
        assert!(!runtime.desk_report(Report::Step {
            tag: send.tag + 1,
            step: Some(hide_desk::permission::Step::Letter),
        }));
        assert!(runtime.desk_report(Report::Step {
            tag: send.tag,
            step: None,
        }));
        assert_eq!(step(&runtime), None);
        runtime.desk_report(Report::Text {
            tag: send.tag,
            text: " in @mission(mission-63 결제 흐름 정리 (11-25)).".into(),
        });
        runtime.desk_report(Report::Step {
            tag: send.tag,
            step: Some(hide_desk::permission::Step::Letter),
        });
        assert_eq!(step(&runtime), Some(crate::model::DeskStepKind::Letters));
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
            missions,
            ..
        }) = desk.conversation.items.last()
        else {
            panic!("no reply");
        };
        assert_eq!(
            text,
            "two sessions in @mission(mission-63 결제 흐름 정리 (11-25))."
        );
        assert_eq!(
            missions,
            &[crate::model::DeskMissionMention {
                id: "mission-63".into(),
                title: "결제 흐름 정리 (11-25)".into(),
            }]
        );
        assert!(thought_ms.is_some());
        assert_eq!((steps.sessions, steps.letters), (2, 1));
        let live = runtime.snapshot.desk_live.as_ref().unwrap();
        assert_eq!((live.text.as_deref(), live.step), (None, None));
    }

    /// An agent that quotes a session's name, `@session(h1:p2 "note")`,
    /// gets a chip named `note`, and the reply Desk keeps writes the token
    /// the way the chip names it, so the chip stands where the words named it.
    #[test]
    fn a_quoted_session_name_in_a_reply_is_its_chip_s_name_without_the_quotes() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        event(&mut runtime, "desk_send", json!({"text": "who wrote it?"}));
        let send = sent(&received);
        opened(&mut runtime, AdapterId::CLAUDE);
        runtime.desk_report(Report::Text {
            tag: send.tag,
            text: "@session(h1:p2 \"note-claude\") wrote it.".into(),
        });
        runtime.desk_report(Report::TurnEnded {
            tag: send.tag,
            outcome: TurnOutcome::Done,
        });
        let desk = runtime.snapshot.desk.as_ref().unwrap();
        let Some(DeskItem::Reply { text, mentions, .. }) = desk.conversation.items.last() else {
            panic!("no reply");
        };
        let names: Vec<_> = mentions
            .iter()
            .map(|chip| (chip.pane_id.as_str(), chip.name.as_str()))
            .collect();
        assert_eq!(names, [("h1:p2", "note-claude")]);
        assert_eq!(text, "@session(h1:p2 note-claude) wrote it.");
    }

    #[test]
    fn the_bar_lists_every_agent_switched_on_in_the_catalog_with_its_sign_in_as_settings_reads_it()
    {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, _received) = runtime(root.path());
        // Switched off in Settings › Agents: Grok is left out.
        kit_on(
            &mut runtime,
            &[
                "claude-code",
                "codex",
                "opencode",
                "pi",
                "omp",
                "antigravity",
            ],
        );
        runtime.ingest_background_ai(read(&[
            ("claude-code", "ready", true),
            ("codex", "ready", true),
            ("antigravity", "needs_login", true),
            ("opencode", "ready", false),
        ]));
        let desk = runtime.snapshot.desk.as_ref().unwrap();
        let bar: Vec<(&str, &str, Option<bool>)> = desk
            .agents
            .iter()
            .map(|agent| (agent.id.as_str(), agent.label, agent.signed_in))
            .collect();
        assert_eq!(
            bar,
            [
                ("claude", "Claude Code", Some(true)),
                ("codex", "Codex", Some(true)),
                // A read that could not check the sign-in claims none.
                ("opencode", "OpenCode", None),
                // Not read yet: nothing is claimed.
                ("pi", "Pi", None),
                ("omp", "omp", None),
                ("agy", "Antigravity", Some(false)),
            ]
        );
        // An adapter in place is not a sign-in: Pi is ready and its sign-in
        // still unknown.
        let pi = desk
            .agents
            .iter()
            .find(|agent| agent.id == AdapterId::PI)
            .unwrap();
        assert_eq!((pi.state, pi.signed_in), (DeskAgentState::Ready, None));
        assert!(!event(
            &mut runtime,
            "desk_select",
            json!({"agent": "grok"})
        ));
    }

    #[test]
    fn the_chosen_agent_switched_off_stays_named_and_is_not_sent_to() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "pi"}));
        kit_on(&mut runtime, &["claude-code"]);
        let desk = runtime.snapshot.desk.as_ref().unwrap();
        let shown: Vec<(&str, DeskAgentState)> = desk
            .agents
            .iter()
            .map(|agent| (agent.id.as_str(), agent.state))
            .collect();
        assert_eq!(
            shown,
            [
                ("claude", DeskAgentState::Ready),
                ("pi", DeskAgentState::Off)
            ]
        );
        assert!(!event(&mut runtime, "desk_send", json!({"text": "hi"})));
        assert!(
            received
                .try_iter()
                .all(|order| !matches!(order, Order::Send(_)))
        );
    }

    #[test]
    fn choosing_an_agent_installs_nothing_and_desk_turned_on_looks_at_every_agent() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "codex"}));
        event(
            &mut runtime,
            "desk_settings_set",
            json!({"default_agent": "codex"}),
        );
        assert_eq!(runtime.desk.stored.selection.agent, Some(AdapterId::CODEX));
        assert_eq!(
            runtime.desk.stored.settings.default_agent,
            Some(AdapterId::CODEX)
        );
        assert!(
            received
                .try_iter()
                .all(|order| !matches!(order, Order::Install { .. })),
            "an agent is not installed when chosen (D-80)"
        );
        event(&mut runtime, "desk_settings_set", json!({"enabled": false}));
        event(&mut runtime, "desk_settings_set", json!({"enabled": true}));
        assert!(
            received
                .try_iter()
                .any(|order| matches!(order, Order::Check))
        );
        // `다시 받기` installs the one agent, as the operator's own ask.
        event(&mut runtime, "desk_install", json!({"agent": "codex"}));
        assert!(received.try_iter().any(|order| matches!(
            order,
            Order::Install {
                adapter: AdapterId::CODEX,
                asked: true
            }
        )));
    }

    #[test]
    fn the_installs_desk_turned_on_starts_show_in_the_bar_and_not_in_the_conversation() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, _received) = runtime(root.path());
        let before = runtime.desk.stored.conversation.items.len();
        runtime.desk_report(Report::Installing {
            adapter: AdapterId::CODEX,
            asked: false,
        });
        assert_eq!(runtime.desk.stored.conversation.items.len(), before);
        assert_eq!(
            runtime.desk.agents[&AdapterId::CODEX].0,
            DeskAgentState::Preparing
        );
        runtime.desk_report(Report::Installing {
            adapter: AdapterId::PI,
            asked: true,
        });
        assert!(matches!(
            runtime.desk.stored.conversation.items.last(),
            Some(DeskItem::Notice {
                notice: DeskNotice::Preparing,
                ..
            })
        ));
    }

    #[test]
    fn a_message_codex_could_not_take_for_its_sign_in_points_at_the_operator_s_own_codex_login() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "codex"}));
        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "what runs now"})
        ));
        let send = sent(&received);
        runtime.desk_report(Report::OpenFailed {
            adapter: AdapterId::CODEX,
            error: OpenError::Login,
            turn: Some(send.tag),
        });
        let conversation = &runtime.desk.stored.conversation.items;
        assert!(
            !conversation
                .iter()
                .any(|item| matches!(item, DeskItem::User { .. })),
            "{conversation:?}"
        );
        assert!(conversation.iter().any(|item| matches!(
            item,
            DeskItem::Notice { notice: DeskNotice::LoginNeeded, detail: Some(command), .. }
                if command == "codex login"
        )));
        // The bar's word stays Settings' read; the agent is still ready.
        assert_eq!(
            runtime.desk.agents[&AdapterId::CODEX].0,
            DeskAgentState::Ready
        );
    }

    /// A turn the agent ended with an error shows the agent's own reason
    /// beside `답을 끝내지 못했어요`, and one with nothing to act on shows
    /// none.
    #[test]
    fn a_failed_turn_shows_the_agent_s_own_reason_when_it_gave_one() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "opencode"}));
        for (text, reason) in [
            ("first", Some("Model not found: bai/gone".to_owned())),
            ("second", None),
        ] {
            assert!(event(&mut runtime, "desk_send", json!({"text": text})));
            let send = sent(&received);
            opened(&mut runtime, AdapterId::OPENCODE);
            runtime.desk_report(Report::TurnEnded {
                tag: send.tag,
                outcome: TurnOutcome::Failed(reason.clone()),
            });
            let items = items(&runtime);
            let Some(DeskItem::Notice {
                notice: DeskNotice::TurnFailed,
                detail,
                ..
            }) = items.last()
            else {
                panic!("no failed-turn notice: {items:?}");
            };
            assert_eq!(detail, &reason);
        }
    }

    /// An agent that refuses a turn for its sign-in (a provider with no key)
    /// ends the process with the turn: the message goes back to the input
    /// and the sign-in line shows, as when the session's open is refused,
    /// and the next message starts a process that reads the sign-in again.
    #[test]
    fn a_turn_refused_for_the_sign_in_points_at_the_agent_s_login_and_starts_afresh() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "opencode"}));
        assert!(event(&mut runtime, "desk_send", json!({"text": "hello"})));
        let send = sent(&received);
        opened(&mut runtime, AdapterId::OPENCODE);
        // What the worker reports for the refusal, in its order.
        runtime.desk_report(Report::OpenFailed {
            adapter: AdapterId::OPENCODE,
            error: OpenError::Login,
            turn: Some(send.tag),
        });
        runtime.desk_report(Report::Ended {
            unexpected: false,
            detail: None,
            turn: None,
        });

        let items = items(&runtime);
        assert!(
            !items
                .iter()
                .any(|item| matches!(item, DeskItem::User { .. })),
            "{items:?}"
        );
        assert!(
            items.iter().any(|item| matches!(item,
                DeskItem::Notice { notice: DeskNotice::LoginNeeded, detail: Some(command), .. }
                    if command == "opencode auth login")),
            "{items:?}"
        );
        assert!(
            !items.iter().any(|item| matches!(
                item,
                DeskItem::Notice {
                    notice: DeskNotice::TurnFailed | DeskNotice::HostStopped,
                    ..
                }
            )),
            "{items:?}"
        );
        let desk = runtime.snapshot.desk.as_ref().unwrap();
        assert_eq!(desk.host.state, DeskHostState::Idle);
        assert!(event(&mut runtime, "desk_send", json!({"text": "hello"})));
        assert_eq!(sent(&received).text, "hello");
    }

    /// A file dropped into Desk's composer (PRD desk-2-ui B41): the agent
    /// gets its staged path quoted before the words, as a file dragged onto
    /// a terminal is typed, the bubble keeps the file's name and size and
    /// not the path in its words, and `다시 시도` sends the same files again.
    #[test]
    fn files_sent_to_desk_reach_the_agent_as_quoted_paths_and_stay_on_the_bubble() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        // A prompt, not a shell line: a path reads as the system spells it, `$` and all.
        let shot = root.path().join("desk-attachments").join("a $b.png");
        let log = root.path().join("desk-attachments").join("install.log");
        let quoted = |path: &std::path::Path| format!("\"{}\"", path.display());
        let files = json!([
            {"name": "a $b.png", "bytes": 2048, "path": shot.display().to_string()},
            {"name": "install.log", "bytes": 84, "path": log.display().to_string()},
        ]);
        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "  why does it drop?  ", "files": files})
        ));
        let send = sent(&received);
        assert_eq!(
            send.text,
            format!("{} {} why does it drop?", quoted(&shot), quoted(&log))
        );
        assert_eq!(
            send.files,
            [shot.display().to_string(), log.display().to_string()]
        );
        let bubble = items(&runtime)
            .into_iter()
            .find_map(|item| match item {
                DeskItem::User {
                    id,
                    text,
                    attachments,
                    ..
                } => Some((id, text, attachments)),
                _ => None,
            })
            .expect("the operator's message");
        assert_eq!(bubble.1, "why does it drop?");
        assert_eq!(
            bubble
                .2
                .iter()
                .map(|file| (file.name.as_str(), file.bytes))
                .collect::<Vec<_>>(),
            [("a $b.png", 2048), ("install.log", 84)]
        );

        runtime.desk_report(Report::TurnEnded {
            tag: send.tag,
            outcome: TurnOutcome::Cancelled,
        });
        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "why does it drop?", "again": bubble.0})
        ));
        assert_eq!(
            sent(&received).text,
            format!("{} {} why does it drop?", quoted(&shot), quoted(&log))
        );
    }

    /// The files of a message, as the line keeps them.
    fn queued_files(runtime: &Runtime) -> Vec<(String, Vec<String>)> {
        runtime
            .snapshot
            .desk
            .as_ref()
            .unwrap()
            .conversation
            .queued
            .iter()
            .map(|message| {
                (
                    message.text.clone(),
                    message
                        .attachments
                        .iter()
                        .map(|file| file.path.clone())
                        .collect(),
                )
            })
            .collect()
    }

    fn held(runtime: &Runtime) -> bool {
        runtime
            .snapshot
            .desk
            .as_ref()
            .unwrap()
            .conversation
            .queued_held
    }

    /// A message whose file is gone was never asked of the agent, and asking
    /// again would meet the same gone file: it stays in the conversation
    /// with a line under it that it was not sent, never goes again, and the
    /// message waiting behind it goes; nothing returns to the composer.
    #[test]
    fn a_message_whose_file_is_gone_stays_unsent_in_the_conversation_and_the_line_goes_on() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        let gone = root.path().join("desk-attachments").join("shot.png");
        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "look", "files": [{"name": "shot.png", "bytes": 3, "path": gone.display().to_string()}]})
        ));
        let send = sent(&received);
        assert!(event(&mut runtime, "desk_send", json!({"text": "next"})));
        // The worker finds the file gone and asks no agent.
        runtime.desk_report(Report::AttachmentsGone { tag: send.tag });
        runtime.desk_report(Report::TurnEnded {
            tag: send.tag,
            outcome: TurnOutcome::Cancelled,
        });
        let shown = items(&runtime);
        let message = shown
            .iter()
            .position(|item| matches!(item, DeskItem::User { text, attachments, .. } if text == "look" && attachments.len() == 1))
            .expect("the message stays in the conversation with its file");
        assert!(
            shown[message..].iter().any(|item| matches!(
                item,
                DeskItem::Notice {
                    notice: DeskNotice::AttachmentsGone,
                    ..
                }
            )),
            "{shown:?}"
        );
        assert!(!held(&runtime));
        // The next waiting message went; the gone one is not in the line.
        assert_eq!(sent(&received).text, "next");
        assert!(queued_files(&runtime).is_empty());
        no_send(&received);
    }

    /// letter-47579: a message with a file sent while Desk answers waits in
    /// line with its file, and when the turn before it ends the agent gets
    /// the file's path before its words. Until then the files of the turn
    /// and of the line are the ones Desk's folder must keep.
    #[test]
    fn a_message_with_a_file_sent_while_desk_answers_reaches_the_agent_from_the_line_with_its_path()
    {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        let first = root.path().join("desk-attachments").join("first.png");
        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "one", "files": [{"name": "first.png", "bytes": 5, "path": first.display().to_string()}]})
        ));
        let one = sent(&received);
        opened(&mut runtime, AdapterId::CLAUDE);
        let log = root.path().join("desk-attachments").join("install.log");
        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "and this", "files": [{"name": "install.log", "bytes": 84, "path": log.display().to_string()}]})
        ));
        no_send(&received);
        assert_eq!(
            queued_files(&runtime),
            [("and this".to_owned(), vec![log.display().to_string()])]
        );
        let mut in_use = runtime.desk_files_in_use();
        in_use.sort();
        assert_eq!(
            in_use,
            [first.display().to_string(), log.display().to_string()]
        );

        runtime.desk_report(Report::TurnEnded {
            tag: one.tag,
            outcome: TurnOutcome::Done,
        });
        assert_eq!(runtime.desk_files_in_use(), [log.display().to_string()]);
        let two = sent(&received);
        assert_eq!(two.text, format!("\"{}\" and this", log.display()));
        assert_eq!(two.files, [log.display().to_string()]);
        assert!(items(&runtime).iter().any(|item| matches!(
            item,
            DeskItem::User { text, attachments, .. }
                if text == "and this" && attachments.len() == 1
        )));
    }

    /// letter-47579: a message with a file the agent never took (its
    /// sign-in) comes back first in line with its file and holds the line;
    /// the next send lets it go first, the file's path with it.
    #[test]
    fn a_refused_message_comes_back_first_in_line_with_its_file() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        let shot = root.path().join("desk-attachments").join("shot.png");
        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "look", "files": [{"name": "shot.png", "bytes": 3, "path": shot.display().to_string()}]})
        ));
        let look = sent(&received);
        runtime.desk_report(Report::OpenFailed {
            adapter: AdapterId::CLAUDE,
            error: OpenError::Login,
            turn: Some(look.tag),
        });
        assert!(
            !items(&runtime)
                .iter()
                .any(|item| matches!(item, DeskItem::User { .. })),
            "the message left the conversation"
        );
        assert_eq!(
            queued_files(&runtime),
            [("look".to_owned(), vec![shot.display().to_string()])]
        );
        assert!(held(&runtime));
        no_send(&received);

        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "signed in"})
        ));
        let again = sent(&received);
        assert_eq!(again.text, format!("\"{}\" look", shot.display()));
        assert_eq!(again.files, [shot.display().to_string()]);
        assert_eq!(
            queued_files(&runtime),
            [("signed in".to_owned(), Vec::new())]
        );
    }

    /// Files alone are a message; nothing at all is not.
    #[test]
    fn a_message_of_files_alone_is_sent_and_an_empty_one_is_not() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        assert!(!event(&mut runtime, "desk_send", json!({"text": "  "})));
        let shot = root.path().join("shot.png");
        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "", "files": [{"name": "shot.png", "bytes": 1, "path": shot.display().to_string()}]})
        ));
        assert!(sent(&received).text.ends_with("shot.png\" "));
    }

    fn queued(runtime: &Runtime) -> Vec<(u64, String)> {
        runtime
            .snapshot
            .desk
            .as_ref()
            .unwrap()
            .conversation
            .queued
            .iter()
            .map(|message| (message.id, message.text.clone()))
            .collect()
    }

    fn no_send(received: &Receiver<Order>) {
        assert!(
            !received
                .try_iter()
                .any(|order| matches!(order, Order::Send(_))),
            "a message went"
        );
    }

    /// letter-47148: a message sent while a reply is being written waits in
    /// line, shown apart from the conversation; ✕ takes one out, `중지`
    /// stops only the turn, and each goes, in order, when the turn before
    /// it ends; past eight the next is refused and the composer keeps it.
    #[test]
    fn a_message_sent_during_a_reply_waits_and_goes_when_the_turn_before_it_ends() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "codex"}));
        assert!(event(&mut runtime, "desk_send", json!({"text": "one"})));
        let one = sent(&received);
        opened(&mut runtime, AdapterId::CODEX);
        for text in ["two", "three", "four"] {
            assert!(event(&mut runtime, "desk_send", json!({"text": text})));
        }
        no_send(&received);
        let waiting = queued(&runtime);
        assert_eq!(
            waiting
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>(),
            ["two", "three", "four"]
        );
        assert!(
            !items(&runtime)
                .iter()
                .any(|item| matches!(item, DeskItem::User { text, .. } if text != "one"))
        );
        assert!(event(
            &mut runtime,
            "desk_queued_remove",
            json!({"id": waiting[1].0})
        ));

        assert!(event(&mut runtime, "desk_stop", json!({})));
        runtime.desk_report(Report::TurnEnded {
            tag: one.tag,
            outcome: TurnOutcome::Cancelled,
        });
        let two = sent(&received);
        assert_eq!(two.text, "two");
        runtime.desk_report(Report::TurnEnded {
            tag: two.tag,
            outcome: TurnOutcome::Done,
        });
        let four = sent(&received);
        assert_eq!(four.text, "four");
        assert!(queued(&runtime).is_empty());

        for index in 0..DESK_QUEUED_MAX {
            assert!(event(
                &mut runtime,
                "desk_send",
                json!({"text": format!("more {index}")})
            ));
        }
        assert!(!event(
            &mut runtime,
            "desk_send",
            json!({"text": "one too many"})
        ));
        assert_eq!(queued(&runtime).len(), DESK_QUEUED_MAX);
    }

    /// A queued message is kept with Desk's store, so after hided starts
    /// again it goes once its agent is ready; one the agent refused to start
    /// for its sign-in goes back first and holds the line until the
    /// operator sends again.
    #[test]
    fn a_queued_message_goes_once_the_agent_is_ready_and_waits_after_a_refused_start() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        runtime.desk.stored.queued.push(DeskQueuedMessage {
            id: 90,
            text: "kept".into(),
            at_ms: 0,
            mentions: Vec::new(),
            attachments: Vec::new(),
        });
        let stored: Stored =
            serde_json::from_slice(&serde_json::to_vec(&runtime.desk.stored).unwrap()).unwrap();
        assert_eq!(stored.queued, runtime.desk.stored.queued);
        let ready = |runtime: &mut Runtime, state: DeskAgentState| {
            runtime.desk_report(Report::Availability(vec![(AdapterId::CLAUDE, state, None)]));
        };
        ready(&mut runtime, DeskAgentState::Preparing);
        no_send(&received);
        ready(&mut runtime, DeskAgentState::Ready);
        let kept = sent(&received);
        assert_eq!(kept.text, "kept");

        assert!(event(&mut runtime, "desk_send", json!({"text": "next"})));
        runtime.desk_report(Report::OpenFailed {
            adapter: AdapterId::CLAUDE,
            error: OpenError::Login,
            turn: Some(kept.tag),
        });
        no_send(&received);
        assert!(event(
            &mut runtime,
            "desk_send",
            json!({"text": "after sign-in"})
        ));
        // The one the agent never took goes first, kept by the core.
        assert_eq!(sent(&received).text, "kept");
        assert_eq!(
            queued(&runtime)
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>(),
            ["next", "after sign-in"]
        );
    }

    pub(super) fn sends(received: &Receiver<Order>) -> Vec<String> {
        received
            .try_iter()
            .filter_map(|order| match order {
                Order::Send(send) => Some(send.text),
                _ => None,
            })
            .collect()
    }

    /// S2: a process that ends under a turn ends that turn and wakes no
    /// queued message: a crash, or a cap crossed right after a turn (the
    /// worker measures before it tells the turn's end), leaves the line
    /// for `다시 시작`.
    #[test]
    fn a_process_that_ends_wakes_no_queued_message() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        event(&mut runtime, "desk_send", json!({"text": "one"}));
        let one = sent(&received);
        opened(&mut runtime, AdapterId::CLAUDE);
        event(&mut runtime, "desk_send", json!({"text": "two"}));
        // The cap: the turn's end and the process's in one report.
        runtime.desk_report(Report::Ended {
            unexpected: true,
            detail: Some("the agent left 60 processes".into()),
            turn: Some(TurnEnd {
                tag: one.tag,
                outcome: TurnOutcome::Done,
            }),
        });
        assert_eq!(sends(&received), Vec::<String>::new());
        assert_eq!(queued(&runtime).len(), 1);

        assert!(event(&mut runtime, "desk_restart", json!({})));
        let two = sent(&received);
        assert_eq!(two.text, "two");
        opened(&mut runtime, AdapterId::CLAUDE);
        event(&mut runtime, "desk_send", json!({"text": "three"}));
        // A crash under the turn ends it as stopped and sends nothing.
        runtime.desk_report(Report::Ended {
            unexpected: true,
            detail: Some("killed".into()),
            turn: Some(TurnEnd {
                tag: two.tag,
                outcome: TurnOutcome::Ended,
            }),
        });
        assert!(runtime.desk.turn.is_none());
        assert_eq!(sends(&received), Vec::<String>::new());
    }

    /// S2: `다시 시작` while the process runs a turn sends the next queued
    /// message, and the old process's end, told after, does not close that
    /// new turn, whose answer then joins the conversation.
    #[test]
    fn the_old_process_s_end_leaves_the_turn_a_restart_started() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        event(&mut runtime, "desk_send", json!({"text": "one"}));
        let one = sent(&received);
        opened(&mut runtime, AdapterId::CLAUDE);
        event(&mut runtime, "desk_send", json!({"text": "two"}));
        assert!(event(&mut runtime, "desk_restart", json!({})));
        let two = sent(&received);
        assert_eq!(two.text, "two");
        runtime.desk_report(Report::Ended {
            unexpected: false,
            detail: None,
            turn: Some(TurnEnd {
                tag: one.tag,
                outcome: TurnOutcome::Ended,
            }),
        });
        assert_eq!(
            runtime.desk.turn.as_ref().map(|turn| turn.tag),
            Some(two.tag)
        );
        opened(&mut runtime, AdapterId::CLAUDE);
        runtime.desk_report(Report::Text {
            tag: two.tag,
            text: "응답: 둘".into(),
        });
        runtime.desk_report(Report::TurnEnded {
            tag: two.tag,
            outcome: TurnOutcome::Done,
        });
        assert!(
            items(&runtime)
                .iter()
                .any(|item| matches!(item, DeskItem::Reply { text, .. } if text == "응답: 둘"))
        );
    }

    /// S3: a message the agent never took goes back first in line, out of
    /// the conversation, and goes again once the operator sends; one taken
    /// from the line after hided started again is kept the same way.
    #[test]
    fn a_message_the_agent_never_took_goes_back_first_in_line() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "codex"}));
        event(&mut runtime, "desk_send", json!({"text": "A"}));
        let a = sent(&received);
        event(&mut runtime, "desk_send", json!({"text": "B"}));
        runtime.desk_report(Report::OpenFailed {
            adapter: AdapterId::CODEX,
            error: OpenError::Login,
            turn: Some(a.tag),
        });
        let texts = |runtime: &Runtime| {
            queued(runtime)
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
        };
        assert_eq!(texts(&runtime), ["A", "B"]);
        assert!(
            !items(&runtime)
                .iter()
                .any(|item| matches!(item, DeskItem::User { .. }))
        );
        assert_eq!(sends(&received), Vec::<String>::new());
        event(&mut runtime, "desk_send", json!({"text": "C"}));
        let again = sent(&received);
        assert_eq!(again.text, "A");
        // The process does not open again: back in line, ahead of B and C.
        runtime.desk_report(Report::OpenFailed {
            adapter: AdapterId::CODEX,
            error: OpenError::Failed("the process did not start".into()),
            turn: Some(again.tag),
        });
        assert_eq!(texts(&runtime), ["A", "B", "C"]);
    }

    /// letter-47232: a login file can outlive its sign-in, so Hide AI's
    /// read says signed in while the agent refuses Desk for its sign-in.
    /// Once it refused, the bar says what the refusal said, `로그인 필요`,
    /// until a session of that agent opens again.
    #[test]
    fn an_agent_that_refused_for_its_sign_in_reads_signed_out_until_a_session_opens() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        runtime.ingest_background_ai(read(&[("codex", "ready", true)]));
        let word = |runtime: &Runtime| {
            runtime
                .snapshot
                .desk
                .as_ref()
                .unwrap()
                .agents
                .iter()
                .find(|agent| agent.id == AdapterId::CODEX)
                .and_then(|agent| agent.signed_in)
        };
        assert_eq!(word(&runtime), Some(true));
        event(&mut runtime, "desk_select", json!({"agent": "codex"}));
        event(&mut runtime, "desk_send", json!({"text": "hi"}));
        let first = sent(&received);
        runtime.desk_report(Report::OpenFailed {
            adapter: AdapterId::CODEX,
            error: OpenError::Login,
            turn: Some(first.tag),
        });
        assert_eq!(word(&runtime), Some(false));
        event(
            &mut runtime,
            "desk_send",
            json!({"text": "after codex login"}),
        );
        sent(&received);
        opened(&mut runtime, AdapterId::CODEX);
        assert_eq!(word(&runtime), Some(true));
    }

    /// An open that failed for a turn `다시 시작` already ended gives back
    /// nothing of the turn it started since, whose message was taken from
    /// the line and goes to the next process.
    #[test]
    fn a_late_refused_open_leaves_the_turn_started_since() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "codex"}));
        event(&mut runtime, "desk_send", json!({"text": "A"}));
        let a = sent(&received);
        event(&mut runtime, "desk_send", json!({"text": "B"}));
        assert!(event(&mut runtime, "desk_restart", json!({})));
        let b = sent(&received);
        assert_eq!(b.text, "B");
        runtime.desk_report(Report::OpenFailed {
            adapter: AdapterId::CODEX,
            error: OpenError::Failed("the process did not start".into()),
            turn: Some(a.tag),
        });
        assert_eq!(runtime.desk.turn.as_ref().map(|turn| turn.tag), Some(b.tag));
        assert!(queued(&runtime).is_empty());
        assert!(
            items(&runtime)
                .iter()
                .any(|item| matches!(item, DeskItem::User { text, .. } if text == "B"))
        );
    }

    /// A line held after a sign-in refusal, full to one past its cap, moves
    /// on one press of `다시 시작` (the login line's button): its head goes
    /// at once and the rest one after another, with no new message.
    #[test]
    fn a_held_line_goes_again_on_restart_even_when_full() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "codex"}));
        event(&mut runtime, "desk_send", json!({"text": "A"}));
        let a = sent(&received);
        for n in 0..DESK_QUEUED_MAX {
            assert!(event(
                &mut runtime,
                "desk_send",
                json!({"text": format!("q{n}")})
            ));
        }
        runtime.desk_report(Report::OpenFailed {
            adapter: AdapterId::CODEX,
            error: OpenError::Login,
            turn: Some(a.tag),
        });
        let conversation = |runtime: &Runtime| {
            let desk = runtime.snapshot.desk.as_ref().unwrap();
            (
                desk.conversation.queued.len(),
                desk.conversation.queued_full,
                desk.conversation.queued_held,
            )
        };
        assert_eq!(conversation(&runtime), (DESK_QUEUED_MAX + 1, true, true));
        no_send(&received);

        assert!(event(&mut runtime, "desk_restart", json!({})));
        let again = sent(&received);
        assert_eq!(again.text, "A");
        assert_eq!(conversation(&runtime), (DESK_QUEUED_MAX, true, false));
        opened(&mut runtime, AdapterId::CODEX);
        runtime.desk_report(Report::TurnEnded {
            tag: again.tag,
            outcome: TurnOutcome::Done,
        });
        assert_eq!(sent(&received).text, "q0");
        assert_eq!(conversation(&runtime), (DESK_QUEUED_MAX - 1, false, false));
    }

    /// O1: Desk turned off with a message waiting, and on again, sends it:
    /// the line is read after every change, not after a few chosen events.
    #[test]
    fn a_message_queued_while_desk_was_off_goes_when_it_is_on_again() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        event(&mut runtime, "desk_send", json!({"text": "one"}));
        sent(&received);
        opened(&mut runtime, AdapterId::CLAUDE);
        event(&mut runtime, "desk_send", json!({"text": "two"}));
        assert!(event(
            &mut runtime,
            "desk_settings_set",
            json!({"enabled": false})
        ));
        assert_eq!(sends(&received), Vec::<String>::new());
        assert!(event(
            &mut runtime,
            "desk_settings_set",
            json!({"enabled": true})
        ));
        assert_eq!(sends(&received), ["two"]);
    }

    /// O3: the line's cap and whether it is full ride the conversation, so
    /// the screen can say the line is full.
    #[test]
    fn the_conversation_says_how_many_may_wait_and_when_the_line_is_full() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        event(&mut runtime, "desk_send", json!({"text": "one"}));
        sent(&received);
        let conversation = |runtime: &Runtime| {
            let desk = runtime.snapshot.desk.as_ref().unwrap();
            (desk.conversation.queued_max, desk.conversation.queued_full)
        };
        assert_eq!(conversation(&runtime), (DESK_QUEUED_MAX, false));
        for index in 0..DESK_QUEUED_MAX {
            event(
                &mut runtime,
                "desk_send",
                json!({"text": format!("m{index}")}),
            );
        }
        assert_eq!(conversation(&runtime), (DESK_QUEUED_MAX, true));
        let first = queued(&runtime)[0].0;
        event(&mut runtime, "desk_queued_remove", json!({"id": first}));
        assert_eq!(conversation(&runtime), (DESK_QUEUED_MAX, false));
    }

    fn stopped_notices(runtime: &Runtime) -> usize {
        items(runtime)
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    DeskItem::Notice {
                        notice: DeskNotice::HostStopped,
                        ..
                    }
                )
            })
            .count()
    }

    /// letter-47179: `멈췄어요 · 다시 시작` leaves the conversation once the
    /// operator restarted, and once a session opened again without it, so
    /// no live `다시 시작` stays under a process that runs.
    #[test]
    fn the_stopped_notice_leaves_once_the_process_restarts_or_a_session_opens() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        let stop = |runtime: &mut Runtime| {
            let turn = runtime.desk.turn.as_ref().map(|turn| TurnEnd {
                tag: turn.tag,
                outcome: TurnOutcome::Ended,
            });
            runtime.desk_report(Report::Ended {
                unexpected: true,
                detail: Some("killed".into()),
                turn,
            });
        };
        event(&mut runtime, "desk_send", json!({"text": "알파첫째"}));
        sent(&received);
        opened(&mut runtime, AdapterId::CLAUDE);
        stop(&mut runtime);
        assert_eq!(stopped_notices(&runtime), 1);
        assert!(event(&mut runtime, "desk_restart", json!({})));
        assert_eq!(stopped_notices(&runtime), 0);

        event(&mut runtime, "desk_send", json!({"text": "브라보둘째"}));
        sent(&received);
        opened(&mut runtime, AdapterId::CLAUDE);
        stop(&mut runtime);
        assert_eq!(stopped_notices(&runtime), 1);
        // The next message starts the process again with no restart.
        event(&mut runtime, "desk_send", json!({"text": "찰리셋째"}));
        assert_eq!(sent(&received).resume.as_deref(), Some("claude-session"));
        opened(&mut runtime, AdapterId::CLAUDE);
        assert_eq!(stopped_notices(&runtime), 0);
    }

    #[test]
    fn a_restart_closes_a_waiting_card_and_resumes_the_session_after() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, received) = runtime(root.path());
        event(&mut runtime, "desk_select", json!({"agent": "claude"}));
        event(&mut runtime, "desk_send", json!({"text": "edit it"}));
        let first = sent(&received);
        opened(&mut runtime, AdapterId::CLAUDE);
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
            json!({"id": card, "option": "allow-once"})
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
        opened(&mut runtime, AdapterId::CLAUDE);
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
                to: AdapterId::PI,
                carried: crate::model::DeskCarried::All,
                ..
            })
        ));
        event(&mut runtime, "desk_send", json!({"text": "continue"}));
        let send = sent(&received);
        assert_eq!(send.adapter, AdapterId::PI);
        assert_eq!(
            send.carried.as_deref(),
            Some("Operator: hello\nClaude Code: hi")
        );
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
            AdapterId::PI,
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

    fn lane(runtime: &Runtime) -> crate::model::DeskLane {
        runtime.snapshot.desk.as_ref().unwrap().lane
    }

    #[test]
    fn a_dragged_width_is_kept_in_desks_store_and_the_other_width_stays() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, _) = runtime(root.path());
        assert_eq!(lane(&runtime), crate::model::DeskLane::default());
        assert!(event(&mut runtime, "desk_lane", json!({"view_width": 720})));
        assert!(event(
            &mut runtime,
            "desk_lane",
            json!({"session_width": 480})
        ));
        // Letting go where it already was changes nothing.
        assert!(!event(
            &mut runtime,
            "desk_lane",
            json!({"session_width": 480})
        ));
        let expected = crate::model::DeskLane {
            view_width: Some(720),
            session_width: Some(480),
        };
        assert_eq!(lane(&runtime), expected);
        assert_eq!(
            runtime.desk.stored.lane, expected,
            "the store saves what the room shows"
        );
    }

    #[test]
    fn a_width_event_without_a_width_or_out_of_range_or_from_desk_1_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, _) = runtime(root.path());
        for payload in [
            json!({}),
            json!({"view_width": 0}),
            json!({"session_width": crate::model::DESK_LANE_WIDTH_MAX + 1}),
        ] {
            assert!(
                !event(&mut runtime, "desk_lane", payload.clone()),
                "{payload}"
            );
        }
        // Desk 1차's pin and kept tabs are no event any more.
        for payload in [
            json!({"action": "pin", "pinned": true}),
            json!({"action": "keep", "tab": {"kind": "mission", "mission": "m", "title": "t"}}),
        ] {
            assert!(
                DeskEvent::decode("desk_lane", payload.clone()).is_none(),
                "{payload}"
            );
        }
        assert_eq!(lane(&runtime), crate::model::DeskLane::default());
    }
}
