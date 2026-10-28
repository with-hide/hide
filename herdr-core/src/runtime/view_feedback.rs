//! Who opened each View and the operator's comments on Views, in the core
//! (PRD view-opener-feedback).
//!
//! The opener is written only where Hide proved the agent: a pane caller of
//! a workspace control open, and a terminal link the operator clicked in an
//! agent's pane (D-02). It is kept by terminal and session and found again
//! each time it is shown (D-23, B40).
//!
//! A Send runs on one feedback worker, off the runtime lock: it reads the
//! screenshots from the node that staged them, places them on the
//! recipient's machine, and only then stores the letter, as the operator's
//! code-owned identity (D-17). The comments leave their View once the letter
//! is stored (D-13), and the ledger's word on the letter decides what
//! becomes of them: delivered drops them, expired brings them back marked
//! (B29). The worker waits on a moving core's effect hold (B41).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hide_node_link::feedback::{FeedbackCall, File, MAX_CALL_BYTES, Placed};
use hide_node_link::protocol::Call;
use serde::Deserialize;

use super::view_areas::ViewWorkspace;
use super::workspace_view::WorkspaceKey;
use super::*;
use crate::delivery::ledger::{Ledger, State as LetterState};
use crate::model::{
    OpenedViewSnapshot, ViewCommentSnapshot, ViewFeedbackGroupSnapshot, ViewFeedbackSendReason,
    ViewFeedbackSendSnapshot, ViewFeedbackSendState, ViewFeedbackSnapshot, ViewLayoutSnapshot,
    ViewNodeSnapshot, ViewOpenerSnapshot,
};
use crate::node_access::{LinkError, NodeLink, call_as};
use crate::view_feedback::{
    Anchor, COMMENTS_PER_CHECKOUT, COMMENTS_PER_VIEW, Comment, CommentState, Feedback, LetterItem,
    Opener, Shot, Target, WAITING_SENDS, WaitingSend,
};
use crate::view_layout::DisplayKind;

/// Terminal-link opens waiting for their document to land, and how long one
/// waits (a read on a slow device).
const PENDING_OPENERS: usize = 32;
const PENDING_OPENER_MS: u64 = 60_000;
/// Jobs the feedback worker holds; past it a removal is left to the next
/// start's sweep and a Send is refused.
const QUEUED_JOBS: usize = 64;
/// How long `Reopen and send` waits for the resumed session (B20).
const REOPEN_MS: u64 = 90_000;
const NODE_CALL: Duration = Duration::from_secs(30);
const LETTER_CALL: Duration = Duration::from_secs(15);
const REVISION_CHARS: usize = 128;

/// What the core holds for View feedback that is not saved.
#[derive(Default)]
pub(super) struct FeedbackRuntime {
    pending_openers: VecDeque<PendingOpener>,
    /// The comment the last delete removed, where it stood, for Undo (D-25).
    deleted: Option<(WorkspaceKey, usize, Comment)>,
    /// The last Send of each checkout, while the screen says it.
    sends: HashMap<WorkspaceKey, ViewFeedbackSendSnapshot>,
    /// What that Send asked for, which Retry asks again (B27).
    asked: HashMap<WorkspaceKey, AskedSend>,
    /// Comments a running Send carries, by checkout and id.
    in_flight: HashSet<(WorkspaceKey, String)>,
    reopens: Vec<Reopen>,
    jobs: VecDeque<Job>,
    running: bool,
    workers: Vec<thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    /// Machines whose orphaned feedback files this run has swept (D-16).
    swept: HashSet<String>,
}

struct AskedSend {
    request_id: String,
    group: String,
    display_id: Option<String>,
    to: SendTo,
}

struct PendingOpener {
    key: WorkspaceKey,
    path: String,
    kind: DisplayKind,
    /// The agent whose terminal the link was in; `None` for the operator's
    /// own open, which makes the View the operator's (PRD session-files D-10).
    opener: Option<Opener>,
    at: u64,
}

struct Reopen {
    key: WorkspaceKey,
    request_id: String,
    comments: Vec<String>,
    opener: Opener,
    deadline: u64,
}

enum Job {
    Send(Box<SendJob>),
    Remove {
        link: Arc<dyn NodeLink>,
        device: String,
        call: FeedbackCall,
    },
}

struct SendJob {
    key: WorkspaceKey,
    request_id: String,
    intent: String,
    comments: Vec<Comment>,
    views: Vec<Option<String>>,
    checkout: String,
    target_pane: String,
    device: String,
    recipient: Arc<dyn NodeLink>,
    shots: HashMap<String, Arc<dyn NodeLink>>,
}

enum SendOutcome {
    Sent { letter: String },
    Failed(ViewFeedbackSendReason, String),
}

#[derive(Debug, Deserialize)]
pub(super) struct ViewCommentPayload {
    workspace: ViewWorkspace,
    #[serde(flatten)]
    action: CommentAction,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum CommentAction {
    Add {
        display_id: String,
        anchor: Anchor,
        note: String,
        #[serde(default)]
        shot: Option<Shot>,
        #[serde(default)]
        revision: Option<String>,
    },
    Edit {
        comment_id: String,
        note: String,
    },
    Delete {
        comment_id: String,
    },
    Undo {
        comment_id: String,
    },
    /// The shell found the comment's lines again elsewhere, or not at all
    /// (D-14).
    Relocate {
        comment_id: String,
        #[serde(default)]
        anchor: Option<Anchor>,
        changed: bool,
    },
}

#[derive(Debug, Deserialize)]
pub(super) struct ViewFeedbackSendPayload {
    workspace: ViewWorkspace,
    /// The Send's own id: a Retry sends it again, and its intent makes the
    /// retry return the letter the first try stored (principle 11).
    request_id: String,
    /// The recipient group, `""` for comments on Views no agent opened.
    group: String,
    /// `This view only` (B16).
    #[serde(default)]
    display_id: Option<String>,
    to: SendTo,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SendTo {
    /// The group's opener, wherever its session is now.
    Default,
    /// Another agent of the checkout (B17, B18).
    Pane { pane_id: String },
    /// Start the closed opener's session again, then send (B20).
    Reopen,
    /// The Send `request_id` again, to whom and for which Views it first
    /// asked (B27); `group` and `display_id` are not read.
    Retry,
}

impl Runtime {
    // ---- Openers ----

    /// The agent in `pane_id` as a View's opener: its machine, its Herdr
    /// terminal and its session. A pane with no agent, or one Hide holds no
    /// terminal for, opens Views as nobody (D-19).
    pub(super) fn opener_of_pane(&self, pane_id: &str) -> Option<Opener> {
        self.pane_opener(pane_id)
            .map_err(|reason| {
                crate::diagnostic!(serde_json::json!({
                    "component": "view_feedback",
                    "kind": "opener.none",
                    "pane": pane_id,
                    "reason": reason,
                }));
            })
            .ok()
    }

    fn pane_opener(&self, pane_id: &str) -> Result<Opener, &'static str> {
        let device = super::agent_sleep::pane_device(pane_id)
            .unwrap_or(self.node.as_str())
            .to_owned();
        let agent = self
            .machine_agents(&device)
            .iter()
            .find(|agent| agent.pane_id == pane_id)
            .ok_or("no_agent")?;
        hide_session::Agent::from_kind(&agent.agent_kind).ok_or("unknown_agent")?;
        let terminal_id = self
            .catalog_workspaces()
            .filter(|workspace| workspace.device_id == device)
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .flat_map(|tab| &tab.panes)
            .find(|pane| pane.id == pane_id)
            .ok_or("no_pane")?
            .terminal_id
            .clone()
            .ok_or("no_terminal")?;
        let resume_path = agent
            .close_session
            .as_ref()
            .filter(|session| session.kind == "path")
            .map(|session| session.value.clone());
        Ok(Opener {
            device_id: device,
            terminal_id,
            session: agent.lineage_session.clone(),
            resume: agent.session_id.clone(),
            resume_path,
            provider: agent.agent_kind.clone(),
            name: agent.identity_label.clone(),
        })
    }

    /// The agent an opener is now: the same terminal with the same session,
    /// else the same session in another pane (D-10). Never by pane id: a
    /// restarted Herdr gives the id to another shell (B40).
    fn opener_now(&self, opener: &Opener) -> Option<&crate::model::SidebarAgentSnapshot> {
        let agents = self.machine_agents(&opener.device_id);
        let in_terminal = self
            .catalog_workspaces()
            .filter(|workspace| workspace.device_id == opener.device_id)
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .flat_map(|tab| &tab.panes)
            .find(|pane| pane.terminal_id.as_deref() == Some(opener.terminal_id.as_str()))
            .and_then(|pane| agents.iter().find(|agent| agent.pane_id == pane.id))
            .filter(|agent| {
                agent.agent_kind == opener.provider && agent.lineage_session == opener.session
            });
        in_terminal.or_else(|| {
            let session = opener.session.as_ref()?;
            agents
                .iter()
                .find(|agent| agent.lineage_session.as_ref() == Some(session))
        })
    }

    fn opener_resumable(opener: &Opener) -> bool {
        let Some(resume) = opener.resume.as_deref() else {
            return false;
        };
        if super::projects::resume_session_arguments(&opener.provider, resume).is_none() {
            return false;
        }
        match hide_session::Agent::from_kind(&opener.provider) {
            Some(agent) if agent.requires_native_proof() => {
                agent.proof_reference_kind() == "id" || opener.resume_path.is_some()
            }
            Some(_) => true,
            None => false,
        }
    }

    pub(super) fn view_opener_snapshot(&self, opener: &Opener) -> ViewOpenerSnapshot {
        let now = self.opener_now(opener);
        ViewOpenerSnapshot {
            name: now.map_or_else(|| opener.name.clone(), |agent| agent.identity_label.clone()),
            provider: opener.provider.clone(),
            device_id: opener.device_id.clone(),
            pane_id: now.map(|agent| agent.pane_id.clone()),
            resumable: now.is_none() && Self::opener_resumable(opener),
        }
    }

    /// Marks the View `view_id` of `key` as opened by `opener` (D-03), and
    /// puts it in that session's Files strip (PRD session-files D-26); with
    /// no opener the operator opened it, and it is in every strip (D-10).
    pub(super) fn stamp_view_opener(
        &mut self,
        key: &WorkspaceKey,
        view_id: &str,
        opener: Option<Opener>,
    ) {
        let stamped = self.change_view_layout(key, |layout, _| {
            let display = layout
                .display_mut(view_id)
                .ok_or_else(|| crate::view_layout::LayoutError::UnknownDisplay(view_id.into()))?;
            let changed = match opener {
                Some(opener) => {
                    let changed = display.opener.as_ref() != Some(&opener)
                        || display.openers.last() != Some(&opener);
                    if changed {
                        display.opened_by(opener);
                    }
                    changed
                }
                None => !std::mem::replace(&mut display.operator, true),
            };
            Ok(((), changed))
        });
        if let Err(error) = stamped {
            crate::diagnostic!(serde_json::json!({
                "component": "view_feedback",
                "kind": "opener.stamp_failed",
                "device": key.0,
                "view": view_id,
                "error": error.kind(),
            }));
        }
    }

    /// A pane caller's `hide file open` or `hide diff open` marks the View
    /// it landed in. A checkout caller, and a pane with no agent, is nobody
    /// (D-19), and the View is the operator's (PRD session-files D-28).
    pub(super) fn stamp_control_opener(
        &mut self,
        key: &WorkspaceKey,
        caller_id: &str,
        action: &crate::workspace_control::Action,
        view_id: &str,
    ) {
        use crate::workspace_control::{Action, Caller};
        if !matches!(
            action,
            Action::OpenFile { .. } | Action::OpenDiff { .. } | Action::OpenBrowser { .. }
        ) {
            return;
        }
        let opener = match Caller::parse(caller_id) {
            Caller::Pane(pane) => self.opener_of_pane(pane),
            Caller::Checkout { .. } => None,
        };
        // An agent's open is for the operator to look at (PRD
        // unseen-agent-views D-06); nobody's is the operator's own, and a
        // page's own new target, opened by a `hide browser` action, is not
        // an open.
        let agent = opener.is_some() && super::workspace_control::lane_call(action);
        self.stamp_view_opener(key, view_id, opener);
        if agent {
            self.mark_view_unseen(key, view_id);
        }
    }

    /// A terminal link the operator clicked in an agent's pane names the
    /// pane; the View the file lands in, now or once it is read, is that
    /// agent's (D-02, B2). Any other open of the operator's, from the
    /// Explorer, ⌘K or a plain shell's link, makes it the operator's (PRD
    /// session-files D-10, D-28).
    pub(super) fn expect_view_opener(
        &mut self,
        key: &WorkspaceKey,
        path: &str,
        kind: DisplayKind,
        pane: Option<&str>,
    ) {
        let opener = pane.and_then(|pane| self.opener_of_pane(pane));
        let pending = &mut self.view_feedback.pending_openers;
        pending.retain(|old| !(old.key == *key && old.path == path && old.kind == kind));
        if pending.len() >= PENDING_OPENERS {
            pending.pop_front();
        }
        pending.push_back(PendingOpener {
            key: key.clone(),
            path: path.to_owned(),
            kind,
            opener,
            at: unix_milliseconds(),
        });
    }

    /// A document placed in a View: an open that expected it marks the View
    /// it shows in.
    pub(super) fn claim_view_opener(
        &mut self,
        key: &WorkspaceKey,
        path: &str,
        kind: DisplayKind,
        committed: Option<bool>,
    ) -> bool {
        let now = unix_milliseconds();
        let pending = &mut self.view_feedback.pending_openers;
        pending.retain(|old| now.saturating_sub(old.at) < PENDING_OPENER_MS);
        let Some(index) = pending
            .iter()
            .position(|old| old.key == *key && old.path == path && old.kind == kind)
        else {
            return false;
        };
        let opener = pending.remove(index).expect("found above").opener;
        let Some(view_id) = self.view_layout_of(key).and_then(|layout| {
            let area = layout.active_area();
            area.active
                .as_ref()
                .and_then(|id| area.displays.iter().find(|display| &display.id == id))
                .filter(|display| display.shows(path, kind, committed))
                .or_else(|| layout.displays().find(|d| d.shows(path, kind, committed)))
                .map(|display| display.id.clone())
        }) else {
            return false;
        };
        self.stamp_view_opener(key, &view_id, opener);
        true
    }

    // ---- Comments ----

    pub(super) fn apply_view_comment(&mut self, payload: ViewCommentPayload) -> bool {
        let key = (payload.workspace.device_id, payload.workspace.path);
        // A refused add's screenshot was staged for it alone.
        let staged = match &payload.action {
            CommentAction::Add {
                shot: Some(shot), ..
            } if crate::view_feedback::shot_name(&shot.name) => Some(shot.clone()),
            _ => None,
        };
        match self.change_view_comment(&key, payload.action) {
            Ok(changed) => {
                if changed {
                    self.persist_feedback(&key);
                }
                changed
            }
            Err((kind, message)) => {
                if let Some(shot) = staged {
                    self.queue_removal(
                        &shot.node,
                        FeedbackCall::RemoveShots {
                            names: vec![shot.name],
                        },
                    );
                }
                self.set_error(kind, message, false);
                true
            }
        }
    }

    fn change_view_comment(
        &mut self,
        key: &WorkspaceKey,
        action: CommentAction,
    ) -> Result<bool, (&'static str, String)> {
        let now = unix_milliseconds();
        let known_node = |runtime: &Self, node: &str| {
            node == runtime.node.as_str()
                || runtime
                    .snapshot
                    .ui_state
                    .device_registrations
                    .iter()
                    .any(|registration| registration.id == node)
        };
        match action {
            CommentAction::Add {
                display_id,
                anchor,
                note,
                shot,
                revision,
            } => {
                let display = self
                    .view_layout_of(key)
                    .and_then(|layout| layout.display(&display_id))
                    .cloned()
                    .ok_or_else(|| {
                        (
                            "view_comment.unknown_view",
                            "The View this comment was left on is closed".to_owned(),
                        )
                    })?;
                let anchor = anchor
                    .admitted(display.kind)
                    .map_err(|kind| (kind, "This comment does not fit its View".to_owned()))?;
                let note = crate::view_feedback::note(&note)
                    .ok_or(("view_comment.empty", "A comment needs a note".to_owned()))?;
                if let Some(shot) = &shot
                    && (!crate::view_feedback::shot_name(&shot.name)
                        || !known_node(self, &shot.node))
                {
                    return Err((
                        "view_comment.invalid_shot",
                        "The screenshot is not one Hide staged".to_owned(),
                    ));
                }
                let feedback = &self.feedback_of(key).ok_or((
                    "view_comment.unknown_view",
                    "The View this comment was left on is closed".to_owned(),
                ))?;
                if feedback.on(&display).count() >= COMMENTS_PER_VIEW {
                    return Err((
                        "view_comment.full",
                        format!(
                            "This view has {COMMENTS_PER_VIEW} comments. Send or delete some to add more."
                        ),
                    ));
                }
                if feedback.comments.len() >= COMMENTS_PER_CHECKOUT {
                    return Err((
                        "view_comment.checkout_full",
                        format!(
                            "This checkout holds {COMMENTS_PER_CHECKOUT} comments. Send or delete some to add more."
                        ),
                    ));
                }
                let feedback = self.feedback_mut(key);
                let id = feedback.mint();
                feedback.comments.push(Comment {
                    id,
                    target: Target::of(&display),
                    display_id: (display.kind == DisplayKind::Browser).then(|| display.id.clone()),
                    recipient: display.opener.clone(),
                    anchor,
                    note,
                    shot,
                    changed: false,
                    revision: revision
                        .filter(|revision| revision.chars().count() <= REVISION_CHARS),
                    created_at_unix_ms: now,
                    state: CommentState::Unsent,
                });
                Ok(true)
            }
            CommentAction::Edit { comment_id, note } => {
                let note = crate::view_feedback::note(&note)
                    .ok_or(("view_comment.empty", "A comment needs a note".to_owned()))?;
                let comment = self.open_comment_mut(key, &comment_id)?;
                let changed = comment.note != note;
                comment.note = note;
                Ok(changed)
            }
            CommentAction::Relocate {
                comment_id,
                anchor,
                changed,
            } => {
                let comment = self.open_comment_mut(key, &comment_id)?;
                let kind = comment.target.kind;
                let anchor = anchor
                    .map(|anchor| anchor.admitted(kind))
                    .transpose()
                    .map_err(|kind| (kind, "This comment does not fit its View".to_owned()))?;
                let mut moved = comment.changed != changed;
                comment.changed = changed;
                if let Some(anchor) = anchor
                    && comment.anchor != anchor
                {
                    comment.anchor = anchor;
                    moved = true;
                }
                Ok(moved)
            }
            CommentAction::Delete { comment_id } => {
                self.open_comment_mut(key, &comment_id)?;
                if self
                    .view_feedback
                    .in_flight
                    .contains(&(key.clone(), comment_id.clone()))
                {
                    return Err((
                        "view_comment.sending",
                        "This comment is being sent".to_owned(),
                    ));
                }
                let feedback = self.feedback_mut(key);
                let index = feedback
                    .comments
                    .iter()
                    .position(|comment| comment.id == comment_id)
                    .expect("found above");
                let comment = feedback.comments.remove(index);
                // Only one delete can be undone; the one before it lets its
                // screenshot go.
                if let Some((_, _, before)) =
                    self.view_feedback
                        .deleted
                        .replace((key.clone(), index, comment))
                {
                    self.release_shots(std::slice::from_ref(&before));
                }
                Ok(true)
            }
            CommentAction::Undo { comment_id } => {
                let Some((deleted_key, index, comment)) = self.view_feedback.deleted.take() else {
                    return Ok(false);
                };
                if deleted_key != *key || comment.id != comment_id {
                    self.view_feedback.deleted = Some((deleted_key, index, comment));
                    return Ok(false);
                }
                // Undo keeps the caps an add keeps: a View that filled up
                // since the delete takes it back no more than a new one.
                let view_full = self.view_layout_of(key).is_some_and(|layout| {
                    layout.displays().any(|display| {
                        comment.on(display)
                            && self.feedback_of(key).is_some_and(|feedback| {
                                feedback.on(display).count() >= COMMENTS_PER_VIEW
                            })
                    })
                });
                if view_full {
                    self.release_shots(std::slice::from_ref(&comment));
                    return Err((
                        "view_comment.full",
                        format!(
                            "This view has {COMMENTS_PER_VIEW} comments. Send or delete some to add more."
                        ),
                    ));
                }
                let feedback = self.feedback_mut(key);
                if feedback.comments.len() >= COMMENTS_PER_CHECKOUT {
                    self.release_shots(std::slice::from_ref(&comment));
                    return Err((
                        "view_comment.checkout_full",
                        "This checkout holds as many comments as it can".to_owned(),
                    ));
                }
                let index = index.min(feedback.comments.len());
                feedback.comments.insert(index, comment);
                Ok(true)
            }
        }
    }

    fn feedback_of(&self, key: &WorkspaceKey) -> Option<&Feedback> {
        self.workspace_views
            .as_ref()?
            .views
            .get(&key.0, &key.1)
            .map(|view| &view.feedback)
    }

    fn feedback_mut(&mut self, key: &WorkspaceKey) -> &mut Feedback {
        &mut self
            .workspace_views
            .as_mut()
            .expect("a View exists for a comment")
            .views
            .entry(&key.0, &key.1)
            .feedback
    }

    fn open_comment_mut(
        &mut self,
        key: &WorkspaceKey,
        id: &str,
    ) -> Result<&mut Comment, (&'static str, String)> {
        self.workspace_views
            .as_mut()
            .and_then(|store| store.views.get_mut(&key.0, &key.1))
            .and_then(|view| {
                view.feedback
                    .comments
                    .iter_mut()
                    .find(|comment| comment.id == id && comment.open())
            })
            .ok_or((
                "view_comment.unknown",
                "This comment is no longer on its View".to_owned(),
            ))
    }

    fn persist_feedback(&mut self, _key: &WorkspaceKey) {
        if let Some(store) = self.workspace_views.as_mut() {
            store.generation += 1;
        }
        self.persist_workspace_views();
    }

    // ---- Send ----

    pub(super) fn apply_view_feedback_send(&mut self, payload: ViewFeedbackSendPayload) -> bool {
        let key = (
            payload.workspace.device_id.clone(),
            payload.workspace.path.clone(),
        );
        if let Err((kind, message)) = self.start_view_feedback_send(&key, payload) {
            self.set_error(kind, message, false);
        }
        true
    }

    fn start_view_feedback_send(
        &mut self,
        key: &WorkspaceKey,
        payload: ViewFeedbackSendPayload,
    ) -> Result<(), (&'static str, String)> {
        let ViewFeedbackSendPayload {
            request_id,
            mut group,
            mut display_id,
            mut to,
            ..
        } = payload;
        if !hide_node_link::attachments::valid_request_id(&request_id) {
            return Err(("view_feedback.invalid_request", "The Send has no id".into()));
        }
        if matches!(to, SendTo::Retry) {
            let asked = self
                .view_feedback
                .asked
                .get(key)
                .filter(|asked| asked.request_id == request_id)
                .ok_or((
                    "view_feedback.nothing",
                    "That Send is no longer here to retry".into(),
                ))?;
            group = asked.group.clone();
            display_id = asked.display_id.clone();
            to = asked.to.clone();
        }
        // A Send already under way is not started twice.
        if self.view_feedback.sends.get(key).is_some_and(|send| {
            send.request_id == request_id
                && matches!(
                    send.state,
                    ViewFeedbackSendState::Sending | ViewFeedbackSendState::Reopening
                )
        }) {
            return Ok(());
        }
        let display = match display_id.as_deref() {
            Some(id) => Some(
                self.view_layout_of(key)
                    .and_then(|layout| layout.display(id))
                    .cloned()
                    .ok_or(("view_feedback.unknown_view", "That View is closed".into()))?,
            ),
            None => None,
        };
        let feedback = self.feedback_of(key).ok_or((
            "view_feedback.nothing",
            "There are no comments to send".into(),
        ))?;
        let comments: Vec<Comment> = feedback
            .comments
            .iter()
            .filter(|comment| {
                comment.open()
                    && comment.group().unwrap_or_default() == group
                    && display.as_ref().is_none_or(|display| comment.on(display))
                    && !self
                        .view_feedback
                        .in_flight
                        .contains(&(key.clone(), comment.id.clone()))
            })
            .cloned()
            .collect();
        if comments.is_empty() {
            return Err((
                "view_feedback.nothing",
                "There are no comments to send".into(),
            ));
        }
        // Only a Send that went ahead is one Retry can ask again; a refused
        // one leaves the last such Send to retry.
        let asked = AskedSend {
            request_id: request_id.clone(),
            group: group.clone(),
            display_id: display_id.clone(),
            to: to.clone(),
        };
        let started = self.route_view_feedback_send(key, request_id, comments, to);
        if started.is_ok() {
            self.view_feedback.asked.insert(key.clone(), asked);
        }
        started
    }

    /// Sends `comments` where `to` says: the group's opener, a chosen pane,
    /// or the opener reopened.
    fn route_view_feedback_send(
        &mut self,
        key: &WorkspaceKey,
        request_id: String,
        comments: Vec<Comment>,
        to: SendTo,
    ) -> Result<(), (&'static str, String)> {
        let group_opener = comments
            .iter()
            .find_map(|comment| comment.recipient.clone());
        match to {
            SendTo::Default => {
                let opener = group_opener.ok_or((
                    "view_feedback.no_recipient",
                    "Choose an agent to send these comments to".into(),
                ))?;
                self.send_to_opener(key, request_id, comments, opener)
            }
            SendTo::Pane { pane_id } => {
                if self.pane_checkout_key(&pane_id).as_ref() != Some(key) {
                    return Err((
                        "view_feedback.unknown_agent",
                        "That agent is not in this checkout".into(),
                    ));
                }
                let opener = self.opener_of_pane(&pane_id).ok_or((
                    "view_feedback.unknown_agent",
                    "That agent is not in this checkout".into(),
                ))?;
                self.queue_send(key, request_id, comments, &opener, &pane_id)
            }
            SendTo::Reopen => {
                let opener = group_opener.ok_or((
                    "view_feedback.no_recipient",
                    "Choose an agent to send these comments to".into(),
                ))?;
                self.reopen_and_send(key, request_id, comments, opener)
            }
            SendTo::Retry => unreachable!("a Retry takes what its Send asked above"),
        }
    }

    /// To the group's opener: its pane now, a wait while its device is away
    /// (B28), or closed (B19).
    fn send_to_opener(
        &mut self,
        key: &WorkspaceKey,
        request_id: String,
        comments: Vec<Comment>,
        opener: Opener,
    ) -> Result<(), (&'static str, String)> {
        if let Some(pane) = self.opener_now(&opener).map(|agent| agent.pane_id.clone()) {
            return self.queue_send(key, request_id, comments, &opener, &pane);
        }
        if !self.device_present(&opener.device_id) {
            return self.wait_for_device(key, request_id, comments, opener);
        }
        self.set_send(
            key,
            &request_id,
            ViewFeedbackSendState::Failed,
            comments.len(),
            &opener,
            None,
            Some(ViewFeedbackSendReason::Closed),
        );
        Ok(())
    }

    /// Whether the core reads `device`'s agents now: its own machine, or a
    /// device whose link is up and whose session has arrived.
    fn device_present(&self, device: &str) -> bool {
        device == self.node.as_str()
            || (self.linked_node(device).is_ok()
                && self
                    .snapshot
                    .status
                    .remote
                    .iter()
                    .any(|remote| remote.target_id == device && remote.session.is_some()))
    }

    fn wait_for_device(
        &mut self,
        key: &WorkspaceKey,
        request_id: String,
        comments: Vec<Comment>,
        opener: Opener,
    ) -> Result<(), (&'static str, String)> {
        let feedback = self.feedback_mut(key);
        if feedback.waiting.len() >= WAITING_SENDS {
            return Err((
                "view_feedback.waiting_full",
                "Too many Sends wait for their device. Send again once it is back.".into(),
            ));
        }
        let ids: Vec<String> = comments.iter().map(|comment| comment.id.clone()).collect();
        for comment in &mut feedback.comments {
            if ids.contains(&comment.id) {
                comment.state = CommentState::Waiting {
                    request_id: request_id.clone(),
                };
            }
        }
        feedback.waiting.push(WaitingSend {
            request_id: request_id.clone(),
            comments: ids,
            to: opener.clone(),
            since_unix_ms: unix_milliseconds(),
        });
        let device = self.device_label(&opener.device_id);
        self.set_send(
            key,
            &request_id,
            ViewFeedbackSendState::Waiting,
            comments.len(),
            &opener,
            device,
            None,
        );
        crate::diagnostic!(serde_json::json!({
            "component": "view_feedback",
            "kind": "send.waiting_for_device",
            "request_id": request_id,
            "device": opener.device_id,
        }));
        self.persist_feedback(key);
        Ok(())
    }

    fn reopen_and_send(
        &mut self,
        key: &WorkspaceKey,
        request_id: String,
        comments: Vec<Comment>,
        opener: Opener,
    ) -> Result<(), (&'static str, String)> {
        if self.opener_now(&opener).is_some() {
            return self.send_to_opener(key, request_id, comments, opener);
        }
        if !Self::opener_resumable(&opener) {
            return Err((
                "view_feedback.not_resumable",
                format!("{} cannot be reopened", opener.name),
            ));
        }
        let start_request = format!("view-feedback-reopen-{request_id}");
        let before = self.snapshot.status.last_error.clone();
        self.agent_start_in_checkout(super::events::AgentStartInCheckoutPayload {
            target: None,
            confirm_only: false,
            checkout_path: Some(key.1.clone()),
            device_id: Some(key.0.clone()),
            home: false,
            provider: opener.provider.clone(),
            prompt: None,
            model: None,
            request_id: Some(start_request.clone()),
            resume_session_id: opener.resume.clone(),
            resume_session_path: opener.resume_path.clone(),
            stay_behind: false,
        });
        let refused = self.snapshot.status.last_error != before
            && self
                .snapshot
                .status
                .last_error
                .as_ref()
                .is_some_and(|error| error.request_id.as_deref() == Some(start_request.as_str()));
        if refused {
            crate::diagnostic!(serde_json::json!({
                "component": "view_feedback",
                "kind": "send.reopen_refused",
                "request_id": request_id,
                "device": opener.device_id,
            }));
            self.set_send(
                key,
                &request_id,
                ViewFeedbackSendState::Failed,
                comments.len(),
                &opener,
                None,
                Some(ViewFeedbackSendReason::ReopenFailed),
            );
            return Ok(());
        }
        self.set_send(
            key,
            &request_id,
            ViewFeedbackSendState::Reopening,
            comments.len(),
            &opener,
            None,
            None,
        );
        for comment in &comments {
            self.view_feedback
                .in_flight
                .insert((key.clone(), comment.id.clone()));
        }
        self.view_feedback.reopens.push(Reopen {
            key: key.clone(),
            request_id,
            comments: comments.into_iter().map(|comment| comment.id).collect(),
            opener,
            deadline: unix_milliseconds() + REOPEN_MS,
        });
        Ok(())
    }

    /// The checkout whose tabs hold `pane_id`.
    fn pane_checkout_key(&self, pane_id: &str) -> Option<WorkspaceKey> {
        self.catalog_workspaces().find_map(|workspace| {
            workspace
                .checkouts
                .iter()
                .find(|checkout| {
                    checkout
                        .tabs
                        .iter()
                        .any(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
                })
                .map(|checkout| (workspace.device_id.clone(), checkout.path.clone()))
        })
    }

    fn device_label(&self, device: &str) -> Option<String> {
        (device != self.node.as_str()).then(|| {
            self.snapshot
                .ui_state
                .device_registrations
                .iter()
                .find(|registration| registration.id == device)
                .map_or_else(
                    || device.to_owned(),
                    |registration| registration.label.clone(),
                )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn set_send(
        &mut self,
        key: &WorkspaceKey,
        request_id: &str,
        state: ViewFeedbackSendState,
        count: usize,
        to: &Opener,
        device: Option<String>,
        reason: Option<ViewFeedbackSendReason>,
    ) {
        let pane_id = self.opener_now(to).map(|agent| agent.pane_id.clone());
        let name = self
            .opener_now(to)
            .map_or_else(|| to.name.clone(), |agent| agent.identity_label.clone());
        self.view_feedback.sends.insert(
            key.clone(),
            ViewFeedbackSendSnapshot {
                request_id: request_id.to_owned(),
                state,
                count,
                to: name,
                pane_id,
                device,
                reason,
                at_unix_ms: unix_milliseconds(),
            },
        );
    }

    /// Hands a Send to the feedback worker: what it carries, read under the
    /// lock, and the links its files travel on.
    fn queue_send(
        &mut self,
        key: &WorkspaceKey,
        request_id: String,
        comments: Vec<Comment>,
        to: &Opener,
        target_pane: &str,
    ) -> Result<(), (&'static str, String)> {
        if self.view_feedback.jobs.len() >= QUEUED_JOBS {
            return Err((
                "view_feedback.busy",
                "Hide is still sending earlier comments. Try again in a moment.".into(),
            ));
        }
        let device = super::agent_sleep::pane_device(target_pane)
            .unwrap_or(self.node.as_str())
            .to_owned();
        let recipient = match self.linked_node(&device) {
            Ok((_, link)) => link,
            Err(_) => return self.wait_for_device(key, request_id, comments, to.clone()),
        };
        let mut shots = HashMap::new();
        for shot in comments.iter().filter_map(|comment| comment.shot.as_ref()) {
            if !shots.contains_key(&shot.node)
                && let Ok((_, link)) = self.linked_node(&shot.node)
            {
                shots.insert(shot.node.clone(), link);
            }
        }
        let layout = self.view_layout_of(key);
        let views = comments
            .iter()
            .map(|comment| {
                layout.and_then(|layout| {
                    layout
                        .displays()
                        .find(|display| comment.on(display))
                        .map(|display| display.id.clone())
                })
            })
            .collect();
        let checkout = self
            .catalog_workspaces()
            .filter(|workspace| workspace.device_id == key.0)
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| checkout.path == key.1)
            .map(|checkout| checkout.label.clone())
            .unwrap_or_else(|| super::workspace_view::file_label(&key.1));
        for comment in &comments {
            self.view_feedback
                .in_flight
                .insert((key.clone(), comment.id.clone()));
        }
        self.set_send(
            key,
            &request_id,
            ViewFeedbackSendState::Sending,
            comments.len(),
            to,
            None,
            None,
        );
        self.view_feedback
            .jobs
            .push_back(Job::Send(Box::new(SendJob {
                key: key.clone(),
                intent: format!("{}{request_id}", hide_node_link::feedback::LETTER_PREFIX),
                request_id,
                comments,
                views,
                checkout,
                target_pane: target_pane.to_owned(),
                device,
                recipient,
                shots,
            })));
        self.start_feedback_worker();
        Ok(())
    }

    /// The Send's end, under the lock: the comments leave their View once
    /// the letter is stored, or stay with the reason (D-13, B27).
    fn finish_send(&mut self, job: &SendJob, outcome: SendOutcome) {
        for comment in &job.comments {
            self.view_feedback
                .in_flight
                .remove(&(job.key.clone(), comment.id.clone()));
        }
        let name = self
            .machine_agents(&job.device)
            .iter()
            .find(|agent| agent.pane_id == job.target_pane)
            .map(|agent| agent.identity_label.clone())
            .unwrap_or_default();
        let ids: HashSet<&str> = job.comments.iter().map(|c| c.id.as_str()).collect();
        let count = job.comments.len();
        match outcome {
            SendOutcome::Sent { letter } => {
                if let Some(store) = self.workspace_views.as_mut()
                    && let Some(view) = store.views.get_mut(&job.key.0, &job.key.1)
                {
                    for comment in &mut view.feedback.comments {
                        if ids.contains(comment.id.as_str()) {
                            comment.state = CommentState::Sent {
                                intent: job.intent.clone(),
                                letter: letter.clone(),
                                seen: false,
                            };
                        }
                    }
                    view.feedback
                        .waiting
                        .retain(|send| send.request_id != job.request_id);
                }
                crate::diagnostic!(serde_json::json!({
                    "component": "view_feedback",
                    "kind": "send.sent",
                    "request_id": job.request_id,
                    "letter": letter,
                    "device": job.device,
                    "comments": count,
                }));
                self.view_feedback.sends.insert(
                    job.key.clone(),
                    ViewFeedbackSendSnapshot {
                        request_id: job.request_id.clone(),
                        state: ViewFeedbackSendState::Sent,
                        count,
                        to: name,
                        pane_id: Some(job.target_pane.clone()),
                        device: None,
                        reason: None,
                        at_unix_ms: unix_milliseconds(),
                    },
                );
            }
            SendOutcome::Failed(reason, detail) => {
                // A Send that waited for its device puts its comments back.
                if let Some(store) = self.workspace_views.as_mut()
                    && let Some(view) = store.views.get_mut(&job.key.0, &job.key.1)
                {
                    for comment in &mut view.feedback.comments {
                        if ids.contains(comment.id.as_str())
                            && matches!(comment.state, CommentState::Waiting { .. })
                        {
                            comment.state = CommentState::Unsent;
                        }
                    }
                    view.feedback
                        .waiting
                        .retain(|send| send.request_id != job.request_id);
                }
                crate::diagnostic!(serde_json::json!({
                    "component": "view_feedback",
                    "kind": "send.failed",
                    "request_id": job.request_id,
                    "reason": reason,
                    "device": job.device,
                    "views": job.views.iter().flatten().collect::<Vec<_>>(),
                    "error": detail,
                }));
                let device = matches!(
                    reason,
                    ViewFeedbackSendReason::AttachmentsFull
                        | ViewFeedbackSendReason::LettersFull
                        | ViewFeedbackSendReason::ShotCopy
                )
                .then(|| self.device_label(&job.device))
                .flatten();
                self.view_feedback.sends.insert(
                    job.key.clone(),
                    ViewFeedbackSendSnapshot {
                        request_id: job.request_id.clone(),
                        state: ViewFeedbackSendState::Failed,
                        count,
                        to: name,
                        pane_id: Some(job.target_pane.clone()),
                        device,
                        reason: Some(reason),
                        at_unix_ms: unix_milliseconds(),
                    },
                );
            }
        }
        self.persist_feedback(&job.key);
    }

    // ---- Following letters and devices ----

    /// What the ledger says of the operator's letters (B29, B33): a
    /// delivered letter's comments go, an expired one's come back marked,
    /// and a letter that left the ledger takes its attachments with it.
    pub(super) fn observe_feedback_letters(&mut self, ledger: &Ledger) -> bool {
        let operator = |letter: &&crate::delivery::ledger::Letter| {
            letter.sender.kind == crate::delivery::OPERATOR_KIND
                && hide_node_link::feedback::valid_letter_folder(&letter.intent)
        };
        let gone: Vec<(String, String)> = self
            .delivery_ledger
            .as_ref()
            .ok()
            .map(|before| {
                before
                    .letters
                    .iter()
                    .filter(operator)
                    .filter(|old| !ledger.letters.iter().any(|letter| letter.id == old.id))
                    .map(|old| (old.recipient.device_id.clone(), old.intent.clone()))
                    .collect()
            })
            .unwrap_or_default();
        for (device, letter) in gone {
            self.queue_removal(&device, FeedbackCall::Remove { letter });
        }
        let letters: HashMap<&str, LetterState> = ledger
            .letters
            .iter()
            .filter(operator)
            .map(|letter| (letter.id.as_str(), letter.state))
            .collect();
        let mut released = Vec::new();
        let mut changed_keys = Vec::new();
        if let Some(store) = self.workspace_views.as_mut() {
            for view in &mut store.views.workspaces {
                let mut changed = false;
                view.feedback.comments.retain_mut(|comment| {
                    let CommentState::Sent { letter, seen, .. } = &mut comment.state else {
                        return true;
                    };
                    match letters.get(letter.as_str()) {
                        Some(LetterState::Delivered | LetterState::Acknowledged) => {
                            released.push(comment.clone());
                            changed = true;
                            false
                        }
                        Some(
                            LetterState::Undelivered
                            | LetterState::Expired
                            | LetterState::Cancelled,
                        ) => {
                            crate::diagnostic!(serde_json::json!({
                                "component": "view_feedback",
                                "kind": "letter.not_delivered",
                                "letter": letter,
                                "device": view.device_id,
                            }));
                            comment.state = CommentState::NotDelivered {
                                letter: letter.clone(),
                            };
                            changed = true;
                            true
                        }
                        Some(LetterState::Pending) => {
                            if !*seen {
                                *seen = true;
                                changed = true;
                            }
                            true
                        }
                        // A letter the ledger showed and no longer holds
                        // finished; one it never showed is not read yet.
                        None if *seen => {
                            released.push(comment.clone());
                            changed = true;
                            false
                        }
                        None => true,
                    }
                });
                if changed {
                    changed_keys.push((view.device_id.clone(), view.path.clone()));
                }
            }
        }
        self.release_shots(&released);
        let changed = !changed_keys.is_empty();
        if changed {
            if let Some(store) = self.workspace_views.as_mut() {
                store.generation += 1;
            }
            self.persist_workspace_views();
        }
        changed
    }

    /// Moves what waits on a machine: a Send whose device came back (B28),
    /// a reopened session that started (B20), a sweep owed to a machine
    /// that connected, and a reopen past its deadline.
    pub(super) fn tick_view_feedback(&mut self, now: u64) -> bool {
        let mut changed = false;
        self.sweep_feedback_files();
        // Waiting Sends whose device is here now.
        let waiting: Vec<(WorkspaceKey, WaitingSend)> = self
            .workspace_views
            .iter()
            .flat_map(|store| &store.views.workspaces)
            .flat_map(|view| {
                view.feedback
                    .waiting
                    .iter()
                    .map(|send| ((view.device_id.clone(), view.path.clone()), send.clone()))
            })
            .filter(|(_, send)| self.device_present(&send.to.device_id))
            .collect();
        for (key, send) in waiting {
            changed = true;
            let comments: Vec<Comment> = self
                .feedback_of(&key)
                .map(|feedback| {
                    feedback
                        .comments
                        .iter()
                        .filter(|comment| send.comments.contains(&comment.id))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let live = self
                .opener_now(&send.to)
                .map(|agent| agent.pane_id.clone())
                .filter(|pane| self.delivery_observations.contains_key(pane));
            match live {
                Some(pane) => {
                    let feedback = self.feedback_mut(&key);
                    feedback
                        .waiting
                        .retain(|waiting| waiting.request_id != send.request_id);
                    if let Err((kind, message)) =
                        self.queue_send(&key, send.request_id.clone(), comments, &send.to, &pane)
                    {
                        self.unwait(&key, &send.request_id);
                        self.set_error(kind, message, false);
                    }
                }
                None => {
                    self.unwait(&key, &send.request_id);
                    self.set_send(
                        &key,
                        &send.request_id,
                        ViewFeedbackSendState::Failed,
                        comments.len(),
                        &send.to,
                        None,
                        Some(ViewFeedbackSendReason::Closed),
                    );
                }
            }
            self.persist_feedback(&key);
        }
        // Reopened sessions.
        let reopens = std::mem::take(&mut self.view_feedback.reopens);
        for reopen in reopens {
            let live = self
                .opener_now(&reopen.opener)
                .map(|agent| agent.pane_id.clone())
                .filter(|pane| self.delivery_observations.contains_key(pane));
            if let Some(pane) = live {
                changed = true;
                for id in &reopen.comments {
                    self.view_feedback
                        .in_flight
                        .remove(&(reopen.key.clone(), id.clone()));
                }
                let comments: Vec<Comment> = self
                    .feedback_of(&reopen.key)
                    .map(|feedback| {
                        feedback
                            .comments
                            .iter()
                            .filter(|comment| {
                                comment.open() && reopen.comments.contains(&comment.id)
                            })
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                if comments.is_empty() {
                    continue;
                }
                if let Err((kind, message)) = self.queue_send(
                    &reopen.key,
                    reopen.request_id.clone(),
                    comments,
                    &reopen.opener,
                    &pane,
                ) {
                    self.set_error(kind, message, false);
                }
            } else if now >= reopen.deadline {
                changed = true;
                for id in &reopen.comments {
                    self.view_feedback
                        .in_flight
                        .remove(&(reopen.key.clone(), id.clone()));
                }
                crate::diagnostic!(serde_json::json!({
                    "component": "view_feedback",
                    "kind": "send.reopen_timed_out",
                    "request_id": reopen.request_id,
                    "device": reopen.opener.device_id,
                }));
                self.set_send(
                    &reopen.key,
                    &reopen.request_id,
                    ViewFeedbackSendState::Failed,
                    reopen.comments.len(),
                    &reopen.opener,
                    None,
                    Some(ViewFeedbackSendReason::ReopenFailed),
                );
            } else {
                self.view_feedback.reopens.push(reopen);
            }
        }
        changed
    }

    fn unwait(&mut self, key: &WorkspaceKey, request_id: &str) {
        let feedback = self.feedback_mut(key);
        feedback
            .waiting
            .retain(|waiting| waiting.request_id != request_id);
        for comment in &mut feedback.comments {
            if matches!(&comment.state, CommentState::Waiting { request_id: id } if id == request_id)
            {
                comment.state = CommentState::Unsent;
            }
        }
    }

    /// Lets go of comments' screenshots on the machines that staged them.
    fn release_shots(&mut self, comments: &[Comment]) {
        let mut by_node: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for shot in comments.iter().filter_map(|comment| comment.shot.as_ref()) {
            by_node
                .entry(shot.node.clone())
                .or_default()
                .push(shot.name.clone());
        }
        for (node, names) in by_node {
            self.queue_removal(&node, FeedbackCall::RemoveShots { names });
        }
    }

    fn queue_removal(&mut self, device: &str, call: FeedbackCall) {
        let link = match self.linked_node(device) {
            Ok((_, link)) => link,
            Err(reason) => {
                // The machine's next sweep removes what this could not.
                crate::diagnostic!(serde_json::json!({
                    "component": "view_feedback",
                    "kind": "files.remove_deferred",
                    "device": device,
                    "reason": reason,
                }));
                return;
            }
        };
        if self.view_feedback.jobs.len() >= QUEUED_JOBS {
            crate::diagnostic!(serde_json::json!({
                "component": "view_feedback",
                "kind": "files.remove_deferred",
                "device": device,
                "reason": "queue_full",
            }));
            return;
        }
        self.view_feedback.jobs.push_back(Job::Remove {
            link,
            device: device.to_owned(),
            call,
        });
        self.start_feedback_worker();
    }

    /// Once per run and machine, once the ledger and the Workspace views are
    /// read: removes the screenshots no comment holds and the letter folders
    /// no letter holds (D-16, B33).
    fn sweep_feedback_files(&mut self) {
        let Ok(ledger) = self.delivery_ledger.clone() else {
            return;
        };
        let Some(store) = self.workspace_views.as_ref() else {
            return;
        };
        let mut machines = vec![self.node.as_str().to_owned()];
        machines.extend(
            self.snapshot
                .ui_state
                .device_registrations
                .iter()
                .map(|registration| registration.id.clone()),
        );
        // Every tick asks; only a machine not yet swept and linked now has
        // anything to do, and most ticks have none.
        machines.retain(|machine| {
            !self.view_feedback.swept.contains(machine) && self.linked_node(machine).is_ok()
        });
        if machines.is_empty() {
            return;
        }
        let comments: Vec<&Comment> = store
            .views
            .workspaces
            .iter()
            .flat_map(|view| &view.feedback.comments)
            .chain(
                self.view_feedback
                    .deleted
                    .iter()
                    .map(|(_, _, comment)| comment),
            )
            .collect();
        let mut sweeps = Vec::new();
        for machine in machines {
            let shots = comments
                .iter()
                .filter_map(|comment| comment.shot.as_ref())
                .filter(|shot| shot.node == machine)
                .map(|shot| shot.name.clone())
                .collect();
            let letters = ledger
                .letters
                .iter()
                .filter(|letter| {
                    letter.recipient.device_id == machine
                        && hide_node_link::feedback::valid_letter_folder(&letter.intent)
                })
                .map(|letter| letter.intent.clone())
                .collect();
            sweeps.push((machine, shots, letters));
        }
        for (machine, shots, letters) in sweeps {
            self.view_feedback.swept.insert(machine.clone());
            self.queue_removal(&machine, FeedbackCall::SweepShots { keep: shots });
            self.queue_removal(&machine, FeedbackCall::Sweep { keep: letters });
        }
    }

    // ---- The worker ----

    fn start_feedback_worker(&mut self) {
        if self.view_feedback.running || self.view_feedback.jobs.is_empty() {
            return;
        }
        let Some(context) = self.worker_context.clone() else {
            return;
        };
        self.view_feedback
            .workers
            .retain(|worker| !worker.is_finished());
        let stop = Arc::clone(&self.view_feedback.stop);
        let effects = Arc::clone(&self.effects);
        match thread::Builder::new()
            .name("herdr-core-view-feedback".into())
            .spawn(move || feedback_worker(context, effects, stop))
        {
            Ok(worker) => {
                self.view_feedback.running = true;
                self.view_feedback.workers.push(worker);
            }
            Err(error) => crate::diagnostic!(serde_json::json!({
                "component": "view_feedback",
                "kind": "worker.start_failed",
                "error": error.to_string(),
            })),
        }
    }

    /// Stops the feedback worker for the core's end; the caller joins what
    /// this returns outside the lock.
    pub(crate) fn take_view_feedback_workers(&mut self) -> Vec<thread::JoinHandle<()>> {
        self.view_feedback.stop.store(true, Ordering::Release);
        self.view_feedback.jobs.clear();
        std::mem::take(&mut self.view_feedback.workers)
    }

    // ---- The snapshot ----

    /// What the front Workspace's feedback row, pins and pane lists read.
    pub(super) fn view_feedback_snapshot(
        &self,
        key: &WorkspaceKey,
        layout: &ViewLayoutSnapshot,
    ) -> Option<ViewFeedbackSnapshot> {
        let mut displays = Vec::new();
        collect_displays(&layout.root, &mut displays);
        let mut opened: Vec<OpenedViewSnapshot> = Vec::new();
        let views = self.view_layout_of(key);
        for display in &displays {
            let Some(pane) = display.opener.as_ref().and_then(|o| o.pane_id.clone()) else {
                continue;
            };
            let same = opened.iter_mut().find(|row| {
                row.pane_id == pane
                    && row.kind == display.kind
                    && row.path == display.path
                    && row.url == display.url
            });
            match same {
                Some(_) => {}
                None => opened.push(OpenedViewSnapshot {
                    pane_id: pane,
                    view_id: display.id.clone(),
                    kind: display.kind,
                    label: display.label.clone(),
                    path: display.path.clone(),
                    url: display.url.clone(),
                    comments: display.comments,
                    unseen: views
                        .and_then(|views| views.display(&display.id))
                        .is_some_and(|stored| stored.unseen.is_some()),
                }),
            }
        }
        let feedback = self.feedback_of(key);
        let send = self.view_feedback.sends.get(key).cloned();
        let deleted = self
            .view_feedback
            .deleted
            .as_ref()
            .filter(|(deleted, _, _)| deleted == key)
            .map(|(_, _, comment)| comment.id.clone());
        let stored = self
            .view_layout_of(key)
            .map(|layout| layout.displays().collect::<Vec<_>>())
            .unwrap_or_default();
        let mut comments = Vec::new();
        let mut groups: Vec<ViewFeedbackGroupSnapshot> = Vec::new();
        let mut targets: HashMap<String, Vec<&Target>> = HashMap::new();
        if let Some(feedback) = feedback {
            for comment in feedback.comments.iter().filter(|comment| comment.open()) {
                let group = comment.group().unwrap_or_default();
                let number = {
                    let row = match groups.iter().position(|row| row.key == group) {
                        Some(index) => &mut groups[index],
                        None => {
                            groups.push(ViewFeedbackGroupSnapshot {
                                key: group.clone(),
                                recipient: comment
                                    .recipient
                                    .as_ref()
                                    .map(|opener| self.view_opener_snapshot(opener)),
                                comments: 0,
                                views: 0,
                                not_delivered: 0,
                                display_ids: Vec::new(),
                            });
                            groups.last_mut().expect("pushed above")
                        }
                    };
                    row.comments += 1;
                    if matches!(comment.state, CommentState::NotDelivered { .. }) {
                        row.not_delivered += 1;
                    }
                    let seen = targets.entry(group.clone()).or_default();
                    if !seen.contains(&&comment.target) {
                        seen.push(&comment.target);
                        row.views += 1;
                    }
                    row.comments
                };
                let display_ids: Vec<String> = stored
                    .iter()
                    .filter(|display| comment.on(display))
                    .map(|display| display.id.clone())
                    .collect();
                if display_ids.is_empty() {
                    continue;
                }
                let row = groups
                    .iter_mut()
                    .find(|row| row.key == group)
                    .expect("made above");
                for id in &display_ids {
                    if !row.display_ids.contains(id) {
                        row.display_ids.push(id.clone());
                    }
                }
                comments.push(ViewCommentSnapshot {
                    id: comment.id.clone(),
                    display_ids,
                    kind: comment.target.kind,
                    path: comment.target.path.clone(),
                    url: comment.target.url.clone(),
                    committed: comment.target.committed,
                    anchor: comment.anchor.clone(),
                    note: comment.note.clone(),
                    shot: comment.shot.is_some(),
                    changed: comment.changed,
                    not_delivered: matches!(comment.state, CommentState::NotDelivered { .. }),
                    revision: comment.revision.clone(),
                    group,
                    number,
                });
            }
        }
        if comments.is_empty()
            && groups.is_empty()
            && opened.is_empty()
            && send.is_none()
            && deleted.is_none()
        {
            return None;
        }
        Some(ViewFeedbackSnapshot {
            comments,
            groups,
            opened,
            send,
            deleted,
        })
    }
}

fn collect_displays<'a>(
    node: &'a ViewNodeSnapshot,
    out: &mut Vec<&'a crate::model::ViewDisplaySnapshot>,
) {
    match node {
        ViewNodeSnapshot::Area(area) => out.extend(area.displays.iter()),
        ViewNodeSnapshot::Split(split) => {
            collect_displays(&split.first, out);
            collect_displays(&split.second, out);
        }
    }
}

fn feedback_worker(
    context: RuntimeWorkerContext,
    effects: Arc<crate::effects::EffectHold>,
    stop: Arc<AtomicBool>,
) {
    loop {
        let Some(runtime) = context.runtime.upgrade() else {
            return;
        };
        let job = {
            let mut guard = runtime.lock().unwrap_or_else(|poison| poison.into_inner());
            match guard.view_feedback.jobs.pop_front() {
                Some(job) if !stop.load(Ordering::Acquire) => job,
                _ => {
                    guard.view_feedback.running = false;
                    return;
                }
            }
        };
        drop(runtime);
        // Nothing leaves this machine while a moving core may still be
        // rolled back (B41).
        if !effects.wait_released(&stop) {
            if let Some(runtime) = context.runtime.upgrade() {
                runtime
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .view_feedback
                    .running = false;
            }
            return;
        }
        match job {
            Job::Remove { link, device, call } => {
                if let Err(error) = call_as::<serde_json::Value>(
                    link.as_ref(),
                    Call::Feedback { call: call.clone() },
                    NODE_CALL,
                ) {
                    crate::diagnostic!(serde_json::json!({
                        "component": "view_feedback",
                        "kind": "files.remove_failed",
                        "device": device,
                        "call": removal_name(&call),
                        "error": error.to_string(),
                    }));
                }
            }
            Job::Send(job) => {
                let outcome = run_send(&job, &context.runtime, &stop);
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                runtime
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .finish_send(&job, outcome);
                drop(runtime);
                context.notifier.notify();
            }
        }
    }
}

fn removal_name(call: &FeedbackCall) -> &'static str {
    match call {
        FeedbackCall::ReadShots { .. } => "read_shots",
        FeedbackCall::RemoveShots { .. } => "remove_shots",
        FeedbackCall::SweepShots { .. } => "sweep_shots",
        FeedbackCall::Place { .. } => "place",
        FeedbackCall::Remove { .. } => "remove",
        FeedbackCall::Sweep { .. } => "sweep",
    }
}

/// One Send, off the lock: screenshots read where they were staged and
/// placed on the recipient's machine, the letter rendered, and stored as
/// the operator's. Every step repeats safely under the same intent, so a
/// core that stops between two calls leaves the comments for the next Send
/// rather than waiting out each call's timeout.
fn run_send(job: &SendJob, runtime: &Weak<Mutex<Runtime>>, stop: &AtomicBool) -> SendOutcome {
    let stopped = || {
        SendOutcome::Failed(
            ViewFeedbackSendReason::LetterUnsaved,
            "Hide stopped before the letter was stored".into(),
        )
    };
    let mut files = Vec::new();
    for comment in &job.comments {
        let Some(shot) = &comment.shot else {
            continue;
        };
        if stop.load(Ordering::Acquire) {
            return stopped();
        }
        let Some(link) = job.shots.get(&shot.node) else {
            return SendOutcome::Failed(
                ViewFeedbackSendReason::ShotCopy,
                format!("the screenshot's machine {} is not connected", shot.node),
            );
        };
        match call_as::<Vec<File>>(
            link.as_ref(),
            Call::Feedback {
                call: FeedbackCall::ReadShots {
                    names: vec![shot.name.clone()],
                },
            },
            NODE_CALL,
        ) {
            Ok(mut read) if read.len() == 1 => files.push(read.remove(0)),
            // A screenshot its machine no longer holds cannot come back: the
            // comment goes without it, and the log says so.
            Err(LinkError::Refused(error)) if error.code == hide_node_link::ErrorCode::NotFound => {
                crate::diagnostic!(serde_json::json!({
                    "component": "view_feedback",
                    "kind": "send.shot_missing",
                    "request_id": job.request_id,
                    "device": shot.node,
                }));
            }
            Ok(_) => {
                return SendOutcome::Failed(
                    ViewFeedbackSendReason::ShotCopy,
                    "the screenshot read answered no file".into(),
                );
            }
            Err(error) => {
                return SendOutcome::Failed(ViewFeedbackSendReason::ShotCopy, error.to_string());
            }
        }
    }
    let mut placed: HashMap<String, String> = HashMap::new();
    for chunk in chunks(files) {
        if stop.load(Ordering::Acquire) {
            return stopped();
        }
        let names: Vec<String> = chunk.iter().map(|file| file.name.clone()).collect();
        match place(job, chunk) {
            Ok(paths) => placed.extend(names.into_iter().zip(paths)),
            Err(outcome) => return outcome,
        }
    }
    let items: Vec<LetterItem<'_>> = job
        .comments
        .iter()
        .zip(&job.views)
        .map(|(comment, view)| LetterItem {
            comment,
            view: view.as_deref(),
            shot: comment
                .shot
                .as_ref()
                .and_then(|shot| placed.get(&shot.name))
                .map(String::as_str),
        })
        .collect();
    let mut body = crate::view_feedback::letter(&job.checkout, &items);
    if body.len() > crate::delivery::BODY_LIMIT {
        let file = File::new(hide_node_link::feedback::FEEDBACK_FILE, body.as_bytes());
        let path = match place(job, vec![file]) {
            Ok(paths) => paths.into_iter().next().unwrap_or_default(),
            Err(outcome) => return outcome,
        };
        let views = {
            let mut seen = Vec::new();
            for comment in &job.comments {
                if !seen.contains(&&comment.target) {
                    seen.push(&comment.target);
                }
            }
            seen.len()
        };
        body =
            crate::view_feedback::overflow_letter(&job.checkout, job.comments.len(), views, &path);
    }
    if stop.load(Ordering::Acquire) {
        return stopped();
    }
    let Some(runtime) = runtime.upgrade() else {
        return stopped();
    };
    let prepared = runtime
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .operator_prepare(
            &job.target_pane,
            crate::delivery::Command::Send {
                target: job.target_pane.clone(),
                intent: job.intent.clone(),
                body,
                kind: "report".into(),
                now: false,
            },
        );
    drop(runtime);
    let result = prepared.and_then(|prepared| prepared.run(LETTER_CALL));
    match result {
        Ok(letter) => match letter["id"].as_str() {
            Some(id) => SendOutcome::Sent {
                letter: id.to_owned(),
            },
            None => SendOutcome::Failed(
                ViewFeedbackSendReason::LetterUnsaved,
                "the ledger answered no letter".into(),
            ),
        },
        Err(error) if error == "capacity" => {
            SendOutcome::Failed(ViewFeedbackSendReason::LettersFull, error)
        }
        Err(error) => SendOutcome::Failed(ViewFeedbackSendReason::LetterUnsaved, error),
    }
}

fn place(job: &SendJob, files: Vec<File>) -> Result<Vec<String>, SendOutcome> {
    match call_as::<Placed>(
        job.recipient.as_ref(),
        Call::Feedback {
            call: FeedbackCall::Place {
                letter: job.intent.clone(),
                files,
            },
        },
        NODE_CALL,
    ) {
        Ok(placed) => Ok(placed.paths),
        Err(LinkError::Refused(error)) if error.message == hide_node_link::feedback::PLACE_FULL => {
            Err(SendOutcome::Failed(
                ViewFeedbackSendReason::AttachmentsFull,
                error.message,
            ))
        }
        Err(error) => Err(SendOutcome::Failed(
            ViewFeedbackSendReason::ShotCopy,
            error.to_string(),
        )),
    }
}

/// Files in calls that each stay under the node line's cap.
fn chunks(files: Vec<File>) -> Vec<Vec<File>> {
    let mut chunks: Vec<Vec<File>> = Vec::new();
    let mut size = 0;
    for file in files {
        let len = file.len();
        if chunks.is_empty() || size + len > MAX_CALL_BYTES {
            chunks.push(Vec::new());
            size = 0;
        }
        size += len;
        chunks.last_mut().expect("pushed above").push(file);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::ChangeNotifier;
    use serde_json::json;

    const CHECKOUT: &str = "/checkouts/fixture";
    const SHOT: &str = "shot-0f8fad5b-d9cb-469f-a165-70867728950e.png";

    /// A core whose recipient pane runs a Codex agent in terminal `term-r`,
    /// with a feedback-capable node and the delivery and feedback workers.
    fn send_fixture(
        root: &std::path::Path,
    ) -> (
        Arc<Mutex<Runtime>>,
        crate::delivery::worker::Worker,
        std::path::PathBuf,
    ) {
        let (runtime, _, _, path) = super::super::delivery::tests::fixture(root);
        let state = root.join("state");
        let (worker, client) = crate::delivery::worker::Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path,
        )
        .unwrap();
        {
            let mut guard = runtime.lock().unwrap();
            guard.install_delivery_client(client);
            guard.install_worker_context(Arc::downgrade(&runtime), ChangeNotifier::noop());
            guard.own_node = Arc::new(
                hide_node::Local::new(Some(root.to_owned())).keeping_feedback_in(state.clone()),
            );
            for pane in guard.snapshot.navigator.workspaces[0].checkouts[0].tabs[0]
                .panes
                .iter_mut()
            {
                pane.terminal_id = Some(format!("term-{}", pane.id));
            }
            guard.snapshot.navigator.agents = vec![agent("recipient", "recipient-session")];
        }
        (runtime, worker, state)
    }

    fn agent(pane: &str, session: &str) -> crate::model::SidebarAgentSnapshot {
        let mut agent = project_agents(
            crate::sidebar::owned_label_fixture(json!({"agents": [
                {"id": pane, "pane_id": pane, "agent_status": "idle", "state_change_seq": 1}
            ]}))
            .unwrap(),
        )
        .agents
        .remove(0);
        agent.agent_kind = "codex".into();
        agent.lineage_session = Some(session.into());
        agent.session_id = Some(format!("native-{pane}"));
        agent.identity_label = format!("Agent {pane}");
        agent
    }

    fn key() -> WorkspaceKey {
        (crate::node::TEST_NODE.into(), CHECKOUT.into())
    }

    /// A file View the recipient opened, by the path a pane caller's open
    /// takes.
    fn opened_view(runtime: &mut Runtime) -> String {
        opened_view_at(runtime, "src/login.ts")
    }

    fn opened_view_at(runtime: &mut Runtime, path: &str) -> String {
        let key = key();
        let view = runtime
            .change_view_layout(&key, |layout, stamp| {
                let display = layout.new_display(path, DisplayKind::File, None, false);
                let id = display.id.clone();
                let area = layout.active_area().id.clone();
                layout.insert(&area, display, stamp)?;
                Ok((id, true))
            })
            .unwrap();
        let open = crate::workspace_control::Action::OpenFile {
            path: path.into(),
        };
        runtime.stamp_control_opener(&key, "recipient", &open, &view);
        view
    }

    fn comment(runtime: &mut Runtime, view: &str, note: &str, shot: bool) -> bool {
        let mut payload = json!({
            "workspace": {"device_id": crate::node::TEST_NODE, "path": CHECKOUT},
            "action": "add",
            "display_id": view,
            "anchor": {"kind": "lines", "start": 3, "end": 4, "lines": ["a", "b"]},
            "note": note,
        });
        if shot {
            payload["shot"] = json!({"node": crate::node::TEST_NODE, "name": SHOT});
        }
        runtime.apply_view_comment(serde_json::from_value(payload).unwrap())
    }

    fn send(runtime: &mut Runtime, request: &str, to: serde_json::Value) {
        runtime.apply_view_feedback_send(
            serde_json::from_value(json!({
                "workspace": {"device_id": crate::node::TEST_NODE, "path": CHECKOUT},
                "request_id": request,
                "group": format!("{}/term-recipient", crate::node::TEST_NODE),
                "to": to,
            }))
            .unwrap(),
        );
    }

    fn settled(runtime: &Arc<Mutex<Runtime>>) -> ViewFeedbackSendSnapshot {
        let send = || {
            runtime
                .lock()
                .unwrap()
                .view_feedback
                .sends
                .get(&key())
                .cloned()
        };
        super::super::tests::wait_for("the Send to settle", || {
            send().is_some_and(|send| send.state != ViewFeedbackSendState::Sending)
        });
        send().unwrap()
    }

    #[test]
    fn a_pane_callers_open_marks_its_view_by_terminal_and_session_never_by_pane_id() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, worker, _) = send_fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        let view = opened_view(&mut guard);
        let opener = guard
            .view_layout_of(&key())
            .unwrap()
            .display(&view)
            .unwrap()
            .opener
            .clone()
            .expect("a pane caller's open is its agent's");
        assert_eq!(opener.terminal_id, "term-recipient");
        assert_eq!(opener.session.as_deref(), Some("recipient-session"));
        assert_eq!(
            guard.view_opener_snapshot(&opener).pane_id.as_deref(),
            Some("recipient")
        );
        // A checkout caller (D-19) and a pane with no agent mark nothing.
        let checkout = crate::workspace_control::checkout_caller_id(&"a".repeat(32), CHECKOUT);
        let open = crate::workspace_control::Action::OpenFile {
            path: "src/login.ts".into(),
        };
        assert!(guard.opener_of_pane("sender").is_none());
        guard.stamp_control_opener(&key(), &checkout, &open, &view);
        // A restarted Herdr gives the pane id to another shell: the mark
        // does not follow the id (B40)...
        guard.snapshot.navigator.agents = vec![agent("recipient", "another-session")];
        guard.snapshot.navigator.workspaces[0].checkouts[0].tabs[0].panes[1].terminal_id =
            Some("term-new".into());
        let closed = guard.view_opener_snapshot(&opener);
        assert_eq!(closed.pane_id, None);
        assert!(closed.resumable, "a Codex session can be reopened");
        // ...but the same session in another pane is the same agent (D-10).
        guard
            .snapshot
            .navigator
            .agents
            .push(agent("sender", "recipient-session"));
        assert_eq!(
            guard.view_opener_snapshot(&opener).pane_id.as_deref(),
            Some("sender")
        );
        drop(guard);
        drop(worker);
    }

    #[test]
    fn the_operators_address_clears_a_pages_mark() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, worker, _) = send_fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        let opener = guard.opener_of_pane("recipient");
        let page = guard
            .change_view_layout(&key(), |layout, stamp| {
                let mut display = layout.new_browser_display("http://localhost:5173/", 1);
                display.opener = opener;
                let id = display.id.clone();
                let area = layout.active_area().id.clone();
                layout.insert(&area, display, stamp)?;
                Ok((id, true))
            })
            .unwrap();
        guard.navigate_browser(&key(), &page, "http://localhost:5173/other".into());
        let display = guard
            .view_layout_of(&key())
            .unwrap()
            .display(&page)
            .unwrap();
        assert_eq!(display.url.as_deref(), Some("http://localhost:5173/other"));
        assert!(
            display.opener.is_none(),
            "the operator's address is the operator's (B4)"
        );
        drop(guard);
        drop(worker);
    }

    #[test]
    fn a_view_holds_fifty_comments_and_undo_puts_a_deleted_one_back_in_place() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, worker, _) = send_fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        let view = opened_view(&mut guard);
        for index in 0..COMMENTS_PER_VIEW {
            comment(&mut guard, &view, &format!("note {index}"), false);
        }
        comment(&mut guard, &view, "one too many", false);
        assert_eq!(
            guard
                .snapshot
                .status
                .last_error
                .as_ref()
                .map(|error| error.kind.as_str()),
            Some("view_comment.full")
        );
        let feedback = guard.feedback_of(&key()).unwrap();
        assert_eq!(feedback.comments.len(), COMMENTS_PER_VIEW);
        let second = feedback.comments[1].id.clone();
        let delete = |id: &str, action: &str| -> ViewCommentPayload {
            serde_json::from_value(json!({
                "workspace": {"device_id": crate::node::TEST_NODE, "path": CHECKOUT},
                "action": action, "comment_id": id,
            }))
            .unwrap()
        };
        guard.apply_view_comment(delete(&second, "delete"));
        assert_eq!(
            guard.feedback_of(&key()).unwrap().comments.len(),
            COMMENTS_PER_VIEW - 1
        );
        guard.apply_view_comment(delete(&second, "undo"));
        let feedback = guard.feedback_of(&key()).unwrap();
        assert_eq!(
            feedback.comments[1].id, second,
            "back in the same place (B43)"
        );
        assert_eq!(feedback.comments[1].note, "note 1");
        drop(guard);
        drop(worker);
    }

    #[test]
    fn a_send_places_its_screenshot_stores_one_operator_letter_and_follows_the_ledger() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, worker, state) = send_fixture(root.path());
        let shots = hide_kit::layout::view_feedback_shots(&state);
        hide_platform::fs::private::create_dir_all(&shots).unwrap();
        std::fs::write(shots.join(SHOT), b"\x89PNG fixture").unwrap();
        let view = {
            let mut guard = runtime.lock().unwrap();
            let view = opened_view(&mut guard);
            comment(&mut guard, &view, "Disable the button while loading.", true);
            comment(&mut guard, &view, "Match the input height.", false);
            send(
                &mut guard,
                "6f8fad5b-d9cb-469f-a165-70867728950e",
                json!({"kind": "default"}),
            );
            view
        };
        let sent = settled(&runtime);
        assert_eq!(sent.state, ViewFeedbackSendState::Sent, "{sent:?}");
        assert_eq!(
            (sent.count, sent.pane_id.as_deref()),
            (2, Some("recipient"))
        );
        let mut guard = runtime.lock().unwrap();
        let ledger = guard.delivery_state().unwrap();
        let letter = ledger
            .letters
            .iter()
            .find(|letter| letter.intent.starts_with("view-feedback-"))
            .expect("one operator letter");
        assert_eq!(letter.sender.kind, crate::delivery::OPERATOR_KIND);
        assert_eq!(letter.recipient.pane_id, "recipient");
        assert!(!letter.waiting_answer, "feedback asks no reply (D-12)");
        assert!(
            letter
                .body
                .starts_with("Feedback from the operator: 2 comments on 1 view in ")
        );
        let shot_line = letter
            .body
            .lines()
            .find_map(|line| line.trim().strip_prefix("Screenshot: "))
            .expect("the screenshot's placed path");
        let placed = std::path::Path::new(shot_line);
        assert_eq!(std::fs::read(placed).unwrap(), b"\x89PNG fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(placed).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "only its owner reads it (B33)");
        }
        // The comments left their View with the letter (D-13).
        let display = guard
            .view_layout_of(&key())
            .unwrap()
            .display(&view)
            .unwrap()
            .clone();
        assert_eq!(guard.feedback_of(&key()).unwrap().on(&display).count(), 0);
        // The letter expires undelivered: the comments come back marked (B29).
        let mut expired = (*ledger).clone();
        let letter_id = letter.id.clone();
        for letter in &mut expired.letters {
            if letter.id == letter_id {
                letter.state = LetterState::Undelivered;
            }
        }
        guard.publish_delivery(Arc::new(expired.clone()), true);
        let back: Vec<_> = guard
            .feedback_of(&key())
            .unwrap()
            .on(&display)
            .cloned()
            .collect();
        assert_eq!(back.len(), 2);
        assert!(
            back.iter()
                .all(|comment| matches!(comment.state, CommentState::NotDelivered { .. }))
        );
        // The letter leaves the ledger: its attachments go with it (B33).
        expired.letters.retain(|letter| letter.id != letter_id);
        guard.publish_delivery(Arc::new(expired), true);
        drop(guard);
        let folder = placed.parent().unwrap().to_owned();
        super::super::tests::wait_for("the letter's folder to be removed", || !folder.exists());
        drop(worker);
    }

    #[test]
    fn a_full_attachments_folder_keeps_the_comments_with_the_reason() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, worker, state) = send_fixture(root.path());
        let shots = hide_kit::layout::view_feedback_shots(&state);
        hide_platform::fs::private::create_dir_all(&shots).unwrap();
        std::fs::write(shots.join(SHOT), vec![7u8; 1024]).unwrap();
        // The recipient's machine already holds its 256 MiB of attachments.
        let full = hide_kit::layout::letter_attachments(&state)
            .join("view-feedback-1f8fad5b-d9cb-469f-a165-70867728950e");
        hide_platform::fs::private::create_dir_all(&full).unwrap();
        let file = std::fs::File::create(full.join(SHOT)).unwrap();
        file.set_len(hide_node_link::feedback::MAX_PLACED_BYTES)
            .unwrap();
        {
            let mut guard = runtime.lock().unwrap();
            let view = opened_view(&mut guard);
            comment(&mut guard, &view, "Too big.", true);
            send(
                &mut guard,
                "2f8fad5b-d9cb-469f-a165-70867728950e",
                json!({"kind": "default"}),
            );
        }
        let failed = settled(&runtime);
        assert_eq!(failed.state, ViewFeedbackSendState::Failed);
        assert_eq!(failed.reason, Some(ViewFeedbackSendReason::AttachmentsFull));
        let guard = runtime.lock().unwrap();
        assert!(guard.feedback_of(&key()).unwrap().comments[0].open());
        assert!(
            !guard
                .delivery_state()
                .unwrap()
                .letters
                .iter()
                .any(|letter| letter.intent.starts_with("view-feedback-")),
            "no letter without its attachments (D-13)"
        );
        drop(guard);
        drop(worker);
    }

    #[test]
    fn a_retry_sends_again_only_the_views_and_recipient_its_send_asked_for() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, worker, state) = send_fixture(root.path());
        let shots = hide_kit::layout::view_feedback_shots(&state);
        hide_platform::fs::private::create_dir_all(&shots).unwrap();
        std::fs::write(shots.join(SHOT), vec![7u8; 1024]).unwrap();
        let full = hide_kit::layout::letter_attachments(&state)
            .join("view-feedback-1f8fad5b-d9cb-469f-a165-70867728950e");
        hide_platform::fs::private::create_dir_all(&full).unwrap();
        let file = std::fs::File::create(full.join(SHOT)).unwrap();
        file.set_len(hide_node_link::feedback::MAX_PLACED_BYTES)
            .unwrap();
        let request = "3f8fad5b-d9cb-469f-a165-70867728950e";
        let workspace = json!({"device_id": crate::node::TEST_NODE, "path": CHECKOUT});
        let (here, there) = {
            let mut guard = runtime.lock().unwrap();
            let here = opened_view_at(&mut guard, "src/login.ts");
            let there = opened_view_at(&mut guard, "src/signup.ts");
            comment(&mut guard, &here, "Here.", true);
            comment(&mut guard, &there, "There.", false);
            // This view only, to the agent chosen in the menu.
            guard.apply_view_feedback_send(
                serde_json::from_value(json!({
                    "workspace": workspace, "request_id": request,
                    "group": format!("{}/term-recipient", crate::node::TEST_NODE),
                    "display_id": here, "to": {"kind": "pane", "pane_id": "recipient"},
                }))
                .unwrap(),
            );
            (here, there)
        };
        assert_eq!(settled(&runtime).state, ViewFeedbackSendState::Failed);
        std::fs::remove_dir_all(&full).unwrap();
        // Retry names only the Send; what it asked comes from the core.
        runtime.lock().unwrap().apply_view_feedback_send(
            serde_json::from_value(json!({
                "workspace": workspace, "request_id": request, "group": "", "to": {"kind": "retry"},
            }))
            .unwrap(),
        );
        let sent = settled(&runtime);
        assert_eq!(sent.state, ViewFeedbackSendState::Sent, "{sent:?}");
        assert_eq!(
            (sent.count, sent.pane_id.as_deref()),
            (1, Some("recipient"))
        );
        let guard = runtime.lock().unwrap();
        let layout = guard.view_layout_of(&key()).unwrap();
        let on = |id: &str| {
            let display = layout.display(id).unwrap().clone();
            guard.feedback_of(&key()).unwrap().on(&display).count()
        };
        assert_eq!(
            (on(&here), on(&there)),
            (0, 1),
            "the other View's comment stays"
        );
        drop(guard);
        drop(worker);
    }

    #[test]
    fn a_refused_send_leaves_the_last_send_to_retry() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, worker, state) = send_fixture(root.path());
        let shots = hide_kit::layout::view_feedback_shots(&state);
        hide_platform::fs::private::create_dir_all(&shots).unwrap();
        std::fs::write(shots.join(SHOT), vec![7u8; 1024]).unwrap();
        let full = hide_kit::layout::letter_attachments(&state)
            .join("view-feedback-1f8fad5b-d9cb-469f-a165-70867728950e");
        hide_platform::fs::private::create_dir_all(&full).unwrap();
        let file = std::fs::File::create(full.join(SHOT)).unwrap();
        file.set_len(hide_node_link::feedback::MAX_PLACED_BYTES)
            .unwrap();
        let request = "5f8fad5b-d9cb-469f-a165-70867728950e";
        let workspace = json!({"device_id": crate::node::TEST_NODE, "path": CHECKOUT});
        {
            let mut guard = runtime.lock().unwrap();
            let here = opened_view_at(&mut guard, "src/login.ts");
            let empty = opened_view_at(&mut guard, "src/signup.ts");
            comment(&mut guard, &here, "Here.", true);
            guard.apply_view_feedback_send(
                serde_json::from_value(json!({
                    "workspace": workspace, "request_id": request,
                    "group": format!("{}/term-recipient", crate::node::TEST_NODE),
                    "display_id": here, "to": {"kind": "pane", "pane_id": "recipient"},
                }))
                .unwrap(),
            );
            drop(guard);
            assert_eq!(settled(&runtime).state, ViewFeedbackSendState::Failed);
            // A Send from a View with nothing to send is refused, and asks nothing.
            runtime.lock().unwrap().apply_view_feedback_send(
                serde_json::from_value(json!({
                    "workspace": workspace, "request_id": "6f8fad5b-d9cb-469f-a165-70867728950e",
                    "group": "", "display_id": empty, "to": {"kind": "default"},
                }))
                .unwrap(),
            );
            // Nor does one to an agent that is not here.
            runtime.lock().unwrap().apply_view_feedback_send(
                serde_json::from_value(json!({
                    "workspace": workspace, "request_id": "7f8fad5b-d9cb-469f-a165-70867728950e",
                    "group": format!("{}/term-recipient", crate::node::TEST_NODE),
                    "to": {"kind": "pane", "pane_id": "nobody"},
                }))
                .unwrap(),
            );
        }
        std::fs::remove_dir_all(&full).unwrap();
        runtime.lock().unwrap().apply_view_feedback_send(
            serde_json::from_value(json!({
                "workspace": workspace, "request_id": request, "group": "", "to": {"kind": "retry"},
            }))
            .unwrap(),
        );
        let sent = settled(&runtime);
        assert_eq!(sent.state, ViewFeedbackSendState::Sent, "{sent:?}");
        assert_eq!(sent.request_id, request);
        drop(worker);
    }

    #[test]
    fn a_retry_with_no_send_of_that_id_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, worker, _) = send_fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        let view = opened_view(&mut guard);
        comment(&mut guard, &view, "Here.", false);
        send(
            &mut guard,
            "4f8fad5b-d9cb-469f-a165-70867728950e",
            json!({"kind": "retry"}),
        );
        assert!(!guard.view_feedback.sends.contains_key(&key()));
        assert_eq!(
            guard
                .feedback_of(&key())
                .unwrap()
                .comments
                .iter()
                .filter(|comment| comment.open())
                .count(),
            1
        );
        drop(guard);
        drop(worker);
    }

    #[test]
    fn undo_into_a_view_that_filled_since_the_delete_is_refused_and_lets_its_screenshot_go() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, worker, state) = send_fixture(root.path());
        let shots = hide_kit::layout::view_feedback_shots(&state);
        hide_platform::fs::private::create_dir_all(&shots).unwrap();
        std::fs::write(shots.join(SHOT), b"\x89PNG fixture").unwrap();
        let mut guard = runtime.lock().unwrap();
        let view = opened_view(&mut guard);
        comment(&mut guard, &view, "with a screenshot", true);
        for index in 1..COMMENTS_PER_VIEW {
            comment(&mut guard, &view, &format!("note {index}"), false);
        }
        let first = guard.feedback_of(&key()).unwrap().comments[0].id.clone();
        let change = |id: &str, action: &str| -> ViewCommentPayload {
            serde_json::from_value(json!({
                "workspace": {"device_id": crate::node::TEST_NODE, "path": CHECKOUT},
                "action": action, "comment_id": id,
            }))
            .unwrap()
        };
        guard.apply_view_comment(change(&first, "delete"));
        comment(&mut guard, &view, "the fiftieth again", false);
        guard.apply_view_comment(change(&first, "undo"));
        assert_eq!(
            guard
                .snapshot
                .status
                .last_error
                .as_ref()
                .map(|error| error.kind.as_str()),
            Some("view_comment.full")
        );
        assert_eq!(
            guard.feedback_of(&key()).unwrap().comments.len(),
            COMMENTS_PER_VIEW
        );
        drop(guard);
        let shot = shots.join(SHOT);
        super::super::tests::wait_for("the screenshot to be removed", || !shot.exists());
        drop(worker);
    }

    #[test]
    fn a_refused_adds_staged_screenshot_is_removed() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, worker, state) = send_fixture(root.path());
        let shots = hide_kit::layout::view_feedback_shots(&state);
        hide_platform::fs::private::create_dir_all(&shots).unwrap();
        let shot = shots.join(SHOT);
        std::fs::write(&shot, b"\x89PNG fixture").unwrap();
        {
            let mut guard = runtime.lock().unwrap();
            let view = opened_view(&mut guard);
            for index in 0..COMMENTS_PER_VIEW {
                comment(&mut guard, &view, &format!("note {index}"), false);
            }
            assert!(comment(&mut guard, &view, "one too many", true));
            assert_eq!(
                guard.feedback_of(&key()).unwrap().comments.len(),
                COMMENTS_PER_VIEW
            );
        }
        super::super::tests::wait_for("the screenshot to be removed", || !shot.exists());
        drop(worker);
    }
}
