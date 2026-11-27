//! Desk (PRD Desk 1차): the operator talks to an agent of their choice over
//! ACP, and that agent starts and follows real sessions through the `hide`
//! CLI. The ACP boundary is `hide-desk`; the core owns the conversation,
//! the choices, the approvals and the one Desk process (`runtime/desk.rs`),
//! and `hided` installs the [`DeskPlatform`] that gives the process its
//! identity.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::latest_file::{LatestFile, read_capped};
use crate::model::{
    DESK_CAP_MAX, DeskCarried, DeskChoiceSnapshot, DeskItem, DeskNotice, DeskSelection,
    DeskSettings,
};

pub use hide_desk::AdapterId;

/// What `hided` provides Desk: the capability its process acts with, and
/// where it runs from. Every call may touch the disk, so the core makes them
/// on Desk's worker, never under the runtime lock.
pub trait DeskPlatform: Send + Sync {
    /// Issues a capability for a Desk process about to start; its reference
    /// file is what the process's `hide` presents. It is refused until
    /// [`DeskPlatform::bind`] names the process.
    fn issue(&self) -> Result<DeskCredential, String>;
    /// Binds the capability to the process that started, so it is valid
    /// while that process runs and never for another.
    fn bind(&self, credential: &DeskCredential, pid: u32, started: u64) -> Result<(), String>;
    /// Ends the capability: the process ended or is about to.
    fn revoke(&self, credential: &DeskCredential);
    fn places(&self) -> DeskPlaces;
}

/// The context a Desk capability carries: Hide's own `hide:desk` identity on
/// `node`, with no checkout ([`crate::workspace_control::DESK_CALLER`]).
pub fn desk_context(node: &str) -> crate::workspace_control::Context {
    let caller = crate::workspace_control::DESK_CALLER;
    crate::workspace_control::Context {
        device_id: node.into(),
        workspace_id: caller.into(),
        checkout_id: caller.into(),
        checkout_path: String::new(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeskCredential {
    /// The reference file the process's `HIDE_CAP_REF` names.
    pub reference: PathBuf,
    pub nonce: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeskPlaces {
    /// Where adapters are installed (`hide_kit::layout::desk_adapters`).
    pub adapters: PathBuf,
    /// Desk's own folder (`hide_kit::layout::desk_dir`).
    pub desk: PathBuf,
    /// The folder of the `hide` program the Desk agent runs.
    pub hide_dir: PathBuf,
}

/// Items the conversation keeps; older ones are dropped from the front.
pub(crate) const MAX_ITEMS: usize = 500;
/// Text the conversation keeps, summed over its items.
pub(crate) const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
/// One message's text; a longer message is refused.
pub(crate) const MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// Mentions one message names.
pub(crate) const MAX_MENTIONS: usize = 16;
/// The conversation carried to the next agent on a switch; the oldest part
/// is left behind.
pub(crate) const MAX_CARRIED_BYTES: usize = 64 * 1024;
/// One reply's text; the rest of a longer reply is cut.
pub(crate) const MAX_REPLY_BYTES: usize = 256 * 1024;

/// The conversation as Desk keeps it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Conversation {
    pub generation: u64,
    pub items: Vec<DeskItem>,
    pub dropped: u64,
    pub next_id: u64,
}

impl Conversation {
    pub(crate) fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Adds an item and drops the oldest past either cap.
    pub(crate) fn push(&mut self, item: DeskItem) {
        self.items.push(item);
        self.trim();
    }

    fn trim(&mut self) {
        let mut bytes: usize = self.items.iter().map(DeskItem::text_bytes).sum();
        let mut cut = 0;
        while self.items.len() - cut > MAX_ITEMS
            || (bytes > MAX_TEXT_BYTES && self.items.len() - cut > 1)
        {
            bytes -= self.items[cut].text_bytes();
            cut += 1;
        }
        if cut > 0 {
            self.items.drain(..cut);
            self.dropped += cut as u64;
        }
    }

    /// Takes an item out again, such as a message no agent received.
    pub(crate) fn remove(&mut self, id: u64) {
        self.items.retain(|item| item.id() != id);
    }

    /// Starts a new conversation: everything above is gone from the screen.
    pub(crate) fn clear(&mut self) {
        self.generation += 1;
        self.items.clear();
        self.dropped = 0;
    }

    /// The operator's messages and the replies, newest last, as the next
    /// agent reads them; the oldest are left out past
    /// [`MAX_CARRIED_BYTES`].
    pub(crate) fn carried(&self) -> Option<String> {
        self.carry().0
    }

    /// How much of the conversation a switch now would carry.
    pub(crate) fn carried_extent(&self) -> DeskCarried {
        match self.carry() {
            (None, _) => DeskCarried::None,
            (Some(_), true) => DeskCarried::Trimmed,
            (Some(_), false) => DeskCarried::All,
        }
    }

    fn carry(&self) -> (Option<String>, bool) {
        let mut lines = Vec::new();
        let mut bytes = 0;
        let mut trimmed = false;
        for item in self.items.iter().rev() {
            let line = match item {
                DeskItem::User { text, .. } => format!("Operator: {text}"),
                DeskItem::Reply { agent, text, .. } => format!("{}: {text}", agent.label()),
                _ => continue,
            };
            if bytes + line.len() + 1 > MAX_CARRIED_BYTES {
                trimmed = true;
                break;
            }
            bytes += line.len() + 1;
            lines.push(line);
        }
        if lines.is_empty() {
            return (None, trimmed);
        }
        lines.reverse();
        (Some(lines.join("\n")), trimmed || self.dropped > 0)
    }
}

/// The models and efforts an agent's last session offered, kept so the
/// selector lists them before a session opens.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct KnownChoices {
    pub model_config: Option<String>,
    pub models: Vec<DeskChoiceSnapshot>,
    pub effort_config: Option<String>,
    pub efforts: Vec<DeskChoiceSnapshot>,
}

/// Everything Desk keeps across a restart.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Stored {
    pub selection: DeskSelection,
    pub settings: DeskSettings,
    pub conversation: Conversation,
    /// Each agent's last ACP session, loaded again on the next start (B11).
    pub sessions: Vec<(AdapterId, String)>,
    pub choices: Vec<(AdapterId, KnownChoices)>,
    /// Spawns waiting for a place, in order (B21); kept across a restart.
    #[serde(default)]
    pub queue: Vec<QueuedJob>,
    /// What the core remembers of each job beyond the ledger, by intent.
    #[serde(default)]
    pub marks: Vec<JobMark>,
    /// The widths of Desk's two panels (PRD desk-2-ui B23), under a key of
    /// their own: Desk 1차 kept a pin and kept tabs under `lane`, which this
    /// build does not read and the next save leaves out, and Desk 1차 reads a
    /// store with no `lane` as an empty one, so a downgrade keeps the
    /// conversation and queue (B29).
    #[serde(default, rename = "panel_widths")]
    pub lane: crate::model::DeskLane,
    /// Messages the operator sent while a reply was being written, in order,
    /// kept across a restart; each goes once the turn before it ended. An
    /// older Hide reads past the key.
    #[serde(default)]
    pub queued: Vec<crate::model::DeskQueuedMessage>,
    /// The waiting messages stopped with Desk's ■ (`멈춤`, PRD desk-queue
    /// D-10): none goes until the operator sends one or resumes the line.
    #[serde(default)]
    pub queued_paused: bool,
    /// The sessions the conversation's chips name, by pane, the one named
    /// least recently first, so a chip whose pane is gone still reads its
    /// session's record and thread (feedback 2, frame K). An older Hide
    /// reads past the key, and an entry a later Hide wrote in a shape this
    /// one cannot read is dropped alone (`named_leniently`).
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "named_leniently"
    )]
    pub named: Vec<NamedSession>,
    /// Desk's own desk (PRD desk-own-desk D-02, D-10): its View areas, their
    /// tabs and each area's tab in front, the same tree a Workspace's desk
    /// keeps, each file naming its checkout (`Display::origin`). Read on its
    /// own (`views_leniently`): a tree this build cannot read leaves Desk an
    /// empty desk and the rest of the store as it was, and an older Hide
    /// reads past the key.
    #[serde(
        default,
        skip_serializing_if = "desk_views_empty",
        deserialize_with = "views_leniently"
    )]
    pub views: crate::view_layout::Layout,
    /// The sessions 한눈에 lists at Desk's right: the
    /// ones Desk started and the ones the operator opened beside Desk, by
    /// pane, the oldest first, at most [`MAX_GLANCE_SESSIONS`]. Each entry is
    /// read on its own (`glance_leniently`), and an older Hide reads past the
    /// key.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "glance_leniently"
    )]
    pub glance: Vec<GlanceSession>,
}

/// The sessions [`Stored::glance`] keeps; past it the oldest goes
/// (engineering principle 15).
pub(crate) const MAX_GLANCE_SESSIONS: usize = 32;

/// A session 한눈에 lists: its pane, and when Desk started it or the operator
/// last opened it beside Desk.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct GlanceSession {
    pub pane_id: String,
    pub at_ms: u64,
}

/// Reads each kept 한눈에 session on its own, as [`named_leniently`] does,
/// then holds the list to its order, one entry a pane, and its cap.
fn glance_leniently<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<GlanceSession>, D::Error> {
    let entries = match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Array(entries) => entries,
        _ => {
            crate::diagnostic!(serde_json::json!({
                "component": "desk", "kind": "desk_store.glance_unreadable",
            }));
            return Ok(Vec::new());
        }
    };
    let total = entries.len();
    let mut glance: Vec<GlanceSession> = entries
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<GlanceSession>(entry).ok())
        .filter(|entry| !entry.pane_id.is_empty() && entry.pane_id.len() <= 1024)
        .collect();
    if glance.len() < total {
        crate::diagnostic!(serde_json::json!({
            "component": "desk", "kind": "desk_store.glance_dropped",
            "dropped": total - glance.len(),
        }));
    }
    let mut settled = Vec::with_capacity(glance.len());
    glance.sort_by_key(|entry| entry.at_ms);
    for entry in glance {
        settled.retain(|kept: &GlanceSession| kept.pane_id != entry.pane_id);
        settled.push(entry);
    }
    let excess = settled.len().saturating_sub(MAX_GLANCE_SESSIONS);
    settled.drain(..excess);
    Ok(settled)
}

fn desk_views_empty(layout: &crate::view_layout::Layout) -> bool {
    layout.display_count() == 0
}

/// Reads Desk's desk on its own and settles it as a Workspace's stored desk
/// is settled (`Layout::repair`): a View this build would never write (an
/// outside file, a diff, a run's summary, a file with no checkout, a card
/// with no card) goes, and past `MAX_VIEW_DISPLAYS` the Views looked at
/// longest ago go. A tree that does not read at all is an empty desk, said
/// in the log (B29).
fn views_leniently<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<crate::view_layout::Layout, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let mut layout: crate::view_layout::Layout = match serde_json::from_value(value) {
        Ok(layout) => layout,
        Err(error) => {
            crate::diagnostic!(serde_json::json!({
                "component": "desk", "kind": "desk_store.views_unreadable",
                "error": error.to_string(),
            }));
            return Ok(crate::view_layout::Layout::default());
        }
    };
    let notes = settle_desk_views(&mut layout);
    if !notes.is_empty() {
        crate::diagnostic!(serde_json::json!({
            "component": "desk", "kind": "desk_store.views_repaired",
            "notes": notes,
        }));
    }
    Ok(layout)
}

/// What [`views_leniently`] does to a read tree, with a note per repair.
pub(crate) fn settle_desk_views(layout: &mut crate::view_layout::Layout) -> Vec<String> {
    use crate::view_layout::{DisplayKind, MAX_VIEW_DISPLAYS};
    let mut notes = Vec::new();
    let stray: Vec<String> = layout
        .displays()
        .filter(|display| match display.kind {
            DisplayKind::File => display.origin.is_none(),
            DisplayKind::Diff | DisplayKind::External => true,
            // A page names the session's checkout it loads from, or none
            // when Desk opened it itself.
            DisplayKind::Browser => false,
            DisplayKind::Card => display.card.is_none() || display.is_run_summary(),
        })
        .map(|display| display.id.clone())
        .collect();
    for id in &stray {
        layout.remove(id);
    }
    if !stray.is_empty() {
        notes.push(format!("{} views Desk's desk never holds", stray.len()));
    }
    // Past the cap the Views looked at longest ago go, before the tree's
    // own repair, which would cut the last ones in tree order instead.
    let excess = layout.display_count().saturating_sub(MAX_VIEW_DISPLAYS);
    if excess > 0 {
        let mut oldest: Vec<(u64, String)> = layout
            .displays()
            .map(|display| (display.last_focused_unix_ms, display.id.clone()))
            .collect();
        oldest.sort();
        for (_, id) in oldest.into_iter().take(excess) {
            layout.remove(&id);
        }
        notes.push(format!(
            "{excess} views past {MAX_VIEW_DISPLAYS} were closed"
        ));
    }
    notes.extend(layout.repair());
    notes
}

/// The sessions [`Stored::named`] keeps; past it the one named least
/// recently that no chip of the newest lines names goes.
pub(crate) const MAX_NAMED_SESSIONS: usize = 64;

/// Reads each kept session on its own: one a later build wrote with an agent
/// or a field this build does not know is dropped with a diagnostic, never
/// the whole store, which holds the conversation a downgrade keeps (B29).
fn named_leniently<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<NamedSession>, D::Error> {
    let entries = match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Array(entries) => entries,
        _ => {
            crate::diagnostic!(serde_json::json!({
                "component": "desk", "kind": "desk_store.named_unreadable",
            }));
            return Ok(Vec::new());
        }
    };
    let total = entries.len();
    let named: Vec<NamedSession> = entries
        .into_iter()
        .filter_map(|entry| serde_json::from_value(entry).ok())
        .collect();
    if named.len() < total {
        crate::diagnostic!(serde_json::json!({
            "component": "desk", "kind": "desk_store.named_dropped",
            "dropped": total - named.len(),
        }));
    }
    Ok(named)
}

/// A session a chip of the conversation names, as its pane last ran it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct NamedSession {
    pub pane_id: String,
    pub session: crate::links::ledger::SessionIdentity,
    /// The Hide agent kind its row had, which its chip's agent mark draws.
    pub agent_kind: String,
    /// Where its thread is read on its machine; none for an agent Hide reads
    /// no thread of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<crate::runtime::ThreadReference>,
}

impl Stored {
    /// Keeps the sessions the newest lines' chips name, `chipped` newest
    /// first, as their panes run them now: a pane named again moves to the
    /// recent end, taking the new session if it changed. Only the newest
    /// [`MAX_NAMED_SESSIONS`] chipped are kept, and past the cap the one named
    /// least recently that no chip names goes, so an entry in use is never
    /// dropped and the same chips do not churn the store. Whether a session
    /// changed, for a save; an order alone saves nothing.
    pub(crate) fn keep_named(&mut self, mut chipped: Vec<NamedSession>) -> bool {
        chipped.truncate(MAX_NAMED_SESSIONS);
        let mut changed = false;
        for named in chipped.iter().rev() {
            match self
                .named
                .iter()
                .position(|kept| kept.pane_id == named.pane_id)
            {
                Some(at) => {
                    let kept = self.named.remove(at);
                    changed |= kept != *named;
                }
                None => changed = true,
            }
            self.named.push(named.clone());
        }
        // The chipped, at most the cap, now stand at the recent end, so the
        // oldest past the cap are never ones a chip names.
        let excess = self.named.len().saturating_sub(MAX_NAMED_SESSIONS);
        self.named.drain(..excess);
        changed
    }

    /// Keeps `pane` among 한눈에's sessions as of `at_ms`: one kept already
    /// moves to a later `at_ms` and stays at an earlier one, the list stays
    /// oldest first, and past [`MAX_GLANCE_SESSIONS`] the oldest goes, so a
    /// session older than every one of a full list is not taken and the same
    /// sessions asked again never churn the store. Whether it changed.
    pub(crate) fn keep_glance(&mut self, pane: &str, at_ms: u64) -> bool {
        if let Some(at) = self.glance.iter().position(|kept| kept.pane_id == pane) {
            if self.glance[at].at_ms >= at_ms {
                return false;
            }
            self.glance.remove(at);
        } else if self.glance.len() >= MAX_GLANCE_SESSIONS
            && self
                .glance
                .first()
                .is_some_and(|oldest| oldest.at_ms >= at_ms)
        {
            return false;
        }
        let index = self.glance.partition_point(|kept| kept.at_ms <= at_ms);
        self.glance.insert(
            index,
            GlanceSession {
                pane_id: pane.to_owned(),
                at_ms,
            },
        );
        let excess = self.glance.len().saturating_sub(MAX_GLANCE_SESSIONS);
        for dropped in self.glance.drain(..excess) {
            crate::diagnostic!(serde_json::json!({
                "component": "desk_store",
                "kind": "desk_store.glance_oldest_dropped",
                "pane": dropped.pane_id,
            }));
        }
        true
    }

    /// With 한눈에's list full, the oldest kept session that `ended` says has
    /// ended goes, so the next one kept never pushes out a session at work
    /// while an ended one keeps its place (engineering principle 15).
    /// Whether one went.
    pub(crate) fn glance_room(&mut self, ended: impl Fn(&str) -> bool) -> bool {
        if self.glance.len() < MAX_GLANCE_SESSIONS {
            return false;
        }
        let Some(at) = self.glance.iter().position(|kept| ended(&kept.pane_id)) else {
            return false;
        };
        let dropped = self.glance.remove(at);
        crate::diagnostic!(serde_json::json!({
            "component": "desk_store",
            "kind": "desk_store.glance_ended_dropped",
            "pane": dropped.pane_id,
        }));
        true
    }
}

/// Spawns the queue holds; one past it is refused `desk_queue_full`.
pub(crate) const MAX_QUEUE: usize = 64;
/// Jobs remembered; the oldest ended ones are forgotten first.
pub(crate) const MAX_MARKS: usize = 256;

/// A spawn Desk asked for while every place was taken.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(from = "StoredJob", into = "StoredJob")]
pub(crate) struct QueuedJob {
    pub intent: String,
    /// The spawn as Desk asked it, run as `hide:desk` when a place frees.
    pub command: crate::coordination::Command,
    pub queued_at_ms: u64,
    pub state: QueuedState,
    /// The code its start failed with.
    pub code: Option<String>,
}

/// A [`QueuedJob`] as Desk's store keeps it: the profile its spawn names
/// (`Command::Spawn::profile`) beside the spawn rather than in it. Desk
/// before profiles (1.2.4) reads a spawn that names no field it does not
/// know, and a store it cannot read starts Desk empty and is written over;
/// it reads past a field beside the spawn and starts the job with the only
/// profile it has.
#[derive(Serialize, Deserialize)]
struct StoredJob {
    intent: String,
    command: crate::coordination::Command,
    queued_at_ms: u64,
    state: QueuedState,
    #[serde(default)]
    code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
}

impl From<QueuedJob> for StoredJob {
    fn from(job: QueuedJob) -> Self {
        let mut command = job.command;
        let profile = match &mut command {
            crate::coordination::Command::Spawn { profile, .. } => profile.take(),
            _ => None,
        };
        Self {
            intent: job.intent,
            command,
            queued_at_ms: job.queued_at_ms,
            state: job.state,
            code: job.code,
            profile,
        }
    }
}

impl From<StoredJob> for QueuedJob {
    fn from(stored: StoredJob) -> Self {
        let mut command = stored.command;
        if let crate::coordination::Command::Spawn { profile, .. } = &mut command
            && profile.is_none()
        {
            *profile = stored.profile;
        }
        Self {
            intent: stored.intent,
            command,
            queued_at_ms: stored.queued_at_ms,
            state: stored.state,
            code: stored.code,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QueuedState {
    Waiting,
    /// Taken from the queue and being started.
    Starting,
    Cancelled,
    /// Its start failed, whether it came from the queue or Desk's own
    /// spawn; `다시 시도` puts it back in the queue (B22).
    Failed,
}

/// A job's facts the ledger does not hold.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct JobMark {
    pub intent: String,
    /// Its tab was put in the reserved Desk group once; leaving it after that
    /// is the operator's move (B37).
    pub grouped: bool,
    pub moved_out: bool,
    pub closed: bool,
    /// The operator typed into its terminal (B36).
    pub joined: bool,
    /// Desk watches it.
    pub watched: bool,
    /// The operator saw it since it ended.
    pub read: bool,
    /// It was put among 한눈에's sessions once, when a refresh first found
    /// its pane; one pushed out of that list after it is not put back.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub glanced: bool,
    pub changed_at_ms: u64,
}

const SCHEMA_VERSION: u32 = 1;
/// The store's cap: the conversation's text cap with room for its framing.
const READ_CAP: u64 = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct File {
    schema_version: u32,
    #[serde(flatten)]
    stored: Stored,
}

fn encode(stored: &Stored) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&File {
        schema_version: SCHEMA_VERSION,
        stored: stored.clone(),
    })
    .map_err(|error| format!("could not be encoded: {error}"))
}

fn decode(bytes: &[u8]) -> Result<Stored, String> {
    let file: File =
        serde_json::from_slice(bytes).map_err(|error| format!("could not be decoded: {error}"))?;
    if file.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "has schema version {}, expected {SCHEMA_VERSION}",
            file.schema_version
        ));
    }
    let mut stored = file.stored;
    // A notice this build no longer makes is never shown again.
    stored.conversation.items.retain(|item| {
        !matches!(
            item,
            DeskItem::Notice {
                notice: DeskNotice::Retired,
                ..
            }
        )
    });
    stored.conversation.trim();
    stored.settings.cap = stored.settings.cap.clamp(1, DESK_CAP_MAX);
    stored.sessions.truncate(AdapterId::all().count());
    stored.queue.truncate(MAX_QUEUE);
    // A start is a fact of the thread running it, which no hided before
    // this one left behind: read from the file, it waits again, and asking
    // the same intent again converges on what the ledger already holds.
    for job in &mut stored.queue {
        if job.state == QueuedState::Starting {
            job.state = QueuedState::Waiting;
        }
    }
    stored.queued.retain(|message| {
        (!message.text.is_empty() || !message.attachments.is_empty())
            && message.text.len() <= MAX_MESSAGE_BYTES
    });
    // One past the cap: a message the agent did not take goes back ahead
    // of a full line.
    stored.queued.truncate(crate::model::DESK_QUEUED_MAX + 1);
    let excess = stored.marks.len().saturating_sub(MAX_MARKS);
    stored.marks.drain(..excess);
    stored.named.retain(|named| {
        named.pane_id.len() <= 1024
            && named.agent_kind.len() <= 64
            && named
                .thread
                .as_ref()
                .is_none_or(crate::runtime::ThreadReference::bounded)
    });
    let excess = stored.named.len().saturating_sub(MAX_NAMED_SESSIONS);
    stored.named.drain(..excess);
    let width = |width: Option<u32>| {
        width.filter(|width| (1..=crate::model::DESK_LANE_WIDTH_MAX).contains(width))
    };
    stored.lane.view_width = width(stored.lane.view_width);
    stored.lane.session_width = width(stored.lane.session_width);
    stored.choices.truncate(AdapterId::all().count());
    for (_, choices) in &mut stored.choices {
        choices.models.truncate(hide_desk::session::MAX_CHOICES);
        choices.efforts.truncate(hide_desk::session::MAX_CHOICES);
    }
    Ok(stored)
}

#[cfg(test)]
pub(crate) fn encode_for_test(stored: &Stored) -> Vec<u8> {
    encode(stored).expect("encodes")
}

#[cfg(test)]
pub(crate) fn decode_for_test(bytes: &[u8]) -> Result<Stored, String> {
    decode(bytes)
}

/// Desk's file (`hide_kit::layout::desk_store`), written off the lock.
pub(crate) struct DeskStore {
    path: PathBuf,
    file: LatestFile<Stored>,
}

impl DeskStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            file: LatestFile::new(path.clone(), "hide-desk-save", "desk", "desk_store", encode),
            path,
        }
    }

    /// The last run's Desk, read before the runtime lock is taken. A file
    /// that cannot be read starts Desk empty and says why in the log.
    pub(crate) fn restore(&self) -> Stored {
        match read_capped(&self.path, READ_CAP)
            .and_then(|bytes| bytes.map(|b| decode(&b)).transpose())
        {
            Ok(stored) => stored.unwrap_or_default(),
            Err(message) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "desk",
                    "kind": "desk_store.restore_failed",
                    "message": message,
                }));
                Stored::default()
            }
        }
    }

    pub(crate) fn save(&self, stored: Stored) {
        self.file.save(stored);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeskMention, DeskReplyState};

    /// PRD desk-own-desk B29, B30: Desk's desk is read on its own. A tree this
    /// build cannot read is an empty desk and the conversation stays; a
    /// store from before Desk's desk reads as an empty one; a View this
    /// build never writes there goes.
    #[test]
    fn desks_desk_is_read_on_its_own_and_an_unreadable_one_leaves_the_rest() {
        let mut stored = Stored::default();
        stored.conversation.push(user(1, "kept"));
        let mut value: serde_json::Value =
            serde_json::from_slice(&encode(&stored).unwrap()).unwrap();
        value["views"] = serde_json::json!({"root": "not a tree"});
        let read = decode(&serde_json::to_vec(&value).unwrap()).expect("the store reads");
        assert_eq!(read.views.display_count(), 0);
        assert_eq!(read.conversation.items.len(), 1, "the conversation stays");

        value.as_object_mut().unwrap().remove("views");
        let read = decode(&serde_json::to_vec(&value).unwrap()).expect("an older store reads");
        assert_eq!(read.views.display_count(), 0);

        let mut views = crate::view_layout::Layout::default();
        let area = views.active_area().id.clone();
        let mut kept = views.new_display(
            "/p/a.md",
            crate::view_layout::DisplayKind::File,
            None,
            false,
        );
        kept.origin = Some(crate::view_layout::ViewOrigin {
            device_id: "d".into(),
            path: "/p".into(),
        });
        let stray = views.new_display(
            "/p/b.md",
            crate::view_layout::DisplayKind::File,
            None,
            false,
        );
        let diff = views.new_display(
            "/p/c.md",
            crate::view_layout::DisplayKind::Diff,
            Some(false),
            false,
        );
        for display in [kept, stray, diff] {
            views.append(&area, display).unwrap();
        }
        value["views"] = serde_json::to_value(&views).unwrap();
        let read = decode(&serde_json::to_vec(&value).unwrap()).unwrap();
        let paths: Vec<_> = read
            .views
            .displays()
            .map(|display| display.path.clone())
            .collect();
        assert_eq!(
            paths,
            vec!["/p/a.md".to_owned()],
            "a file with no checkout and a diff go"
        );
    }

    /// PRD desk-own-desk B28, D-10: a saved tree past the 32 Views closes the
    /// ones looked at longest ago as it is read, and a page keeps the
    /// session's checkout it loads from.
    #[test]
    fn a_saved_desk_past_thirty_two_views_reads_back_its_newest_and_a_page_keeps_its_checkout() {
        use crate::view_layout::{DisplayKind, Layout, MAX_VIEW_DISPLAYS, ViewOrigin};
        let origin = ViewOrigin {
            device_id: "d".into(),
            path: "/p".into(),
        };
        let mut views = Layout::default();
        let area = views.active_area().id.clone();
        for n in 0..MAX_VIEW_DISPLAYS {
            let mut file =
                views.new_display(&format!("/p/f{n:02}.md"), DisplayKind::File, None, false);
            file.origin = Some(origin.clone());
            file.last_focused_unix_ms = 10 + n as u64;
            views.append(&area, file).unwrap();
        }
        let mut page = views.new_browser_display("http://localhost:5173/", 1);
        page.origin = Some(origin.clone());
        page.last_focused_unix_ms = 100;
        // The 33rd View, which no tree this build keeps would hold, written
        // into the saved file's one area.
        let mut tree = serde_json::to_value(&views).unwrap();
        fn displays(value: &mut serde_json::Value) -> Option<&mut Vec<serde_json::Value>> {
            match value {
                serde_json::Value::Object(map) => {
                    if map.get("displays").is_some_and(serde_json::Value::is_array) {
                        return map.get_mut("displays")?.as_array_mut();
                    }
                    map.values_mut().find_map(displays)
                }
                serde_json::Value::Array(items) => items.iter_mut().find_map(displays),
                _ => None,
            }
        }
        displays(&mut tree)
            .expect("the area's Views")
            .push(serde_json::to_value(&page).unwrap());
        let mut value: serde_json::Value =
            serde_json::from_slice(&encode(&Stored::default()).unwrap()).unwrap();
        value["views"] = tree;

        let read = decode(&serde_json::to_vec(&value).unwrap()).unwrap();

        assert_eq!(read.views.display_count(), MAX_VIEW_DISPLAYS);
        let paths: Vec<_> = read
            .views
            .displays()
            .map(|display| display.path.clone())
            .collect();
        assert!(!paths.contains(&"/p/f00.md".to_owned()), "the oldest went");
        let page = read
            .views
            .displays()
            .find(|display| display.kind == DisplayKind::Browser)
            .expect("the page stays");
        assert_eq!(page.origin.as_ref(), Some(&origin));
    }

    /// The fields of Desk's store that name a machine, which the node-id
    /// migration does not map yet (with-hide/hide issue 3044): a View's
    /// checkout's machine and the machine of the agent that opened it (Desk's
    /// own agent's is the core's node), a 한눈에 session on a connected device
    /// (its pane id spells the device), a named session's machine and a
    /// queued spawn's machine. A new field that names a machine fails here,
    /// so it reaches that issue's list instead of a renamed device unnoticed.
    const DESK_MACHINE_FIELDS: &[&str] = &[
        "/glance/*/pane_id",
        "/named/*/session/device_id",
        "/queue/*/command/machine",
        "/views/root/area/displays/*/opener/device_id",
        "/views/root/area/displays/*/openers/*/device_id",
        "/views/root/area/displays/*/origin/device_id",
    ];

    #[test]
    fn the_machine_ids_desks_store_saves_are_the_listed_ones() {
        use crate::view_layout::{DisplayKind, ViewOrigin};
        let origin = ViewOrigin {
            device_id: "d".into(),
            path: "/p".into(),
        };
        let mut stored = Stored::default();
        let area = stored.views.active_area().id.clone();
        let mut file = stored
            .views
            .new_display("/p/a.md", DisplayKind::File, None, false);
        file.origin = Some(origin.clone());
        file.opened_by(
            crate::view_feedback::Opener {
                device_id: "d".into(),
                terminal_id: crate::workspace_control::DESK_CALLER.into(),
                session: None,
                resume: None,
                resume_path: None,
                provider: "codex".into(),
                name: "Desk".into(),
                profile: None,
            },
            1,
        );
        stored.views.append(&area, file).unwrap();
        stored.keep_glance("remote:d:pane:p2", 1);
        let mut page = stored
            .views
            .new_browser_display("http://localhost:5173/", 1);
        page.origin = Some(origin);
        stored.views.append(&area, page).unwrap();
        stored.named.push(NamedSession {
            pane_id: "p1".into(),
            session: crate::links::ledger::SessionIdentity {
                device_id: "d".into(),
                agent: "codex".into(),
                session_id: "s1".into(),
            },
            agent_kind: "codex".into(),
            thread: None,
        });
        stored.queue.push(QueuedJob {
            intent: "desk:1".into(),
            command: crate::coordination::Command::Spawn {
                pick: None,
                parent: None,
                machine: Some("d".into()),
                name: "worker".into(),
                intent: "desk:1".into(),
                kind: "codex".into(),
                repo: Some("/r".into()),
                branch: Some("main".into()),
                path: None,
                args: Vec::new(),
                launch: Default::default(),
                task: None,
                brief: Some("go".into()),
                beside: false,
                mission: None,
                plan_node: None,
                plan: None,
                profile: None,
            },
            queued_at_ms: 1,
            state: QueuedState::Waiting,
            code: None,
        });
        let saved: serde_json::Value = serde_json::from_slice(&encode(&stored).unwrap()).unwrap();

        fn walk(
            value: &serde_json::Value,
            at: String,
            found: &mut std::collections::BTreeSet<String>,
        ) {
            match value {
                serde_json::Value::Object(map) => {
                    for (key, value) in map {
                        let here = format!("{at}/{key}");
                        if matches!(key.as_str(), "device_id" | "machine" | "node" | "device") {
                            found.insert(here.clone());
                        }
                        walk(value, here, found);
                    }
                }
                serde_json::Value::Array(items) => {
                    for item in items {
                        walk(item, format!("{at}/*"), found);
                    }
                }
                // A connected device's pane id spells its machine.
                serde_json::Value::String(text) if text.starts_with("remote:") => {
                    found.insert(at);
                }
                _ => {}
            }
        }
        let mut found = std::collections::BTreeSet::new();
        walk(&saved, String::new(), &mut found);

        assert_eq!(
            found.iter().map(String::as_str).collect::<Vec<_>>(),
            DESK_MACHINE_FIELDS,
            "a field of desk.json that names a machine is listed in DESK_MACHINE_FIELDS and issue 3044"
        );
    }

    fn user(id: u64, text: &str) -> DeskItem {
        DeskItem::User {
            id,
            text: text.into(),
            at_ms: 0,
            mentions: Vec::<DeskMention>::new(),
            attachments: Vec::new(),
        }
    }

    #[test]
    fn the_oldest_items_are_dropped_past_the_item_cap() {
        let mut conversation = Conversation::default();
        for id in 0..(MAX_ITEMS as u64 + 3) {
            conversation.push(user(id, "hi"));
        }
        assert_eq!(conversation.items.len(), MAX_ITEMS);
        assert_eq!(conversation.dropped, 3);
        assert_eq!(conversation.items[0].id(), 3);
    }

    #[test]
    fn the_oldest_items_are_dropped_past_the_text_cap_but_the_newest_stays() {
        let mut conversation = Conversation::default();
        let big = "x".repeat(MAX_TEXT_BYTES / 2 + 1);
        conversation.push(user(1, &big));
        conversation.push(user(2, &big));
        assert_eq!(conversation.items.len(), 1);
        assert_eq!(conversation.items[0].id(), 2);
        conversation.push(user(3, &"y".repeat(MAX_TEXT_BYTES + 1)));
        assert_eq!(conversation.items.len(), 1);
        assert_eq!(conversation.items[0].id(), 3);
    }

    #[test]
    fn a_switch_carries_the_newest_messages_within_its_cap() {
        let mut conversation = Conversation::default();
        conversation.push(user(1, &"old ".repeat(MAX_CARRIED_BYTES / 4)));
        conversation.push(user(2, "what runs now?"));
        conversation.push(DeskItem::Reply {
            id: 3,
            agent: AdapterId::CODEX,
            text: "two sessions".into(),
            at_ms: 0,
            state: DeskReplyState::Done,
            thought_ms: None,
            steps: Default::default(),
            mentions: Vec::new(),
            missions: Vec::new(),
            compaction: None,
        });
        let carried = conversation.carried().unwrap();
        assert_eq!(carried, "Operator: what runs now?\nCodex: two sessions");
    }

    /// A queued spawn that names a profile is stored so that Desk 1.2.4
    /// still reads the store: its spawn refuses a field it does not know
    /// (`deny_unknown_fields`), and a store it cannot read starts Desk empty
    /// (with-hide/hide#2281 review). `Released` is that release's spawn as
    /// its reader takes it, kept as it shipped.
    #[test]
    fn a_queued_spawn_with_a_profile_is_stored_as_desk_1_2_4_reads_it() {
        #[allow(dead_code)]
        #[derive(serde::Deserialize)]
        #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
        enum Released {
            Spawn {
                parent: Option<String>,
                machine: Option<String>,
                name: String,
                intent: String,
                kind: String,
                repo: String,
                branch: String,
                path: Option<String>,
                args: Vec<String>,
                #[serde(default)]
                launch: serde_json::Value,
                #[serde(default)]
                task: serde_json::Value,
                #[serde(default)]
                brief: Option<String>,
                #[serde(default)]
                beside: bool,
                #[serde(default)]
                plan_node: Option<String>,
            },
        }
        let spawn = |profile: Option<&str>| crate::coordination::Command::Spawn {
            pick: None,
            parent: None,
            machine: None,
            name: "worker".into(),
            intent: "desk:1".into(),
            kind: "codex".into(),
            repo: Some("/r".into()),
            branch: Some("main".into()),
            path: None,
            args: Vec::new(),
            launch: Default::default(),
            task: None,
            brief: Some("go".into()),
            beside: false,
            mission: None,
            plan_node: None,
            plan: None,
            profile: profile.map(str::to_owned),
        };
        let job = QueuedJob {
            intent: "desk:1".into(),
            command: spawn(Some("work")),
            queued_at_ms: 1,
            state: QueuedState::Waiting,
            code: None,
        };
        let mut stored = Stored::default();
        stored.queue.push(job.clone());

        let written: serde_json::Value = serde_json::from_slice(&encode(&stored).unwrap()).unwrap();
        let command = written["queue"][0]["command"].clone();
        assert!(
            serde_json::from_value::<Released>(command).is_ok(),
            "{written}"
        );
        assert_eq!(written["queue"][0]["profile"], "work");
        assert_eq!(decode(&encode(&stored).unwrap()).unwrap().queue, [job]);
    }

    #[test]
    fn a_store_from_a_newer_schema_is_not_read() {
        let bytes = br#"{"schema_version": 2}"#;
        assert!(decode(bytes).is_err());
        let stored = Stored::default();
        assert_eq!(decode(&encode(&stored).unwrap()).unwrap(), stored);
    }

    #[test]
    fn the_permission_choice_comes_back_after_a_restart_and_a_store_from_before_reads_as_allow_all()
    {
        use crate::model::{DeskApprovalState, DeskCardKind, DeskOptionKind, DeskPermission};
        let mut stored = Stored::default();
        stored.settings.permission = DeskPermission::Agent;
        let restored = decode(&encode(&stored).unwrap()).unwrap();
        assert_eq!(restored.settings.permission, DeskPermission::Agent);

        // A store #2972's build wrote: its permission table, a card it kept
        // allowed for the conversation, and notices this build no longer
        // makes, which go.
        let older = serde_json::json!({
            "schema_version": 1,
            "selection": {"agent": "claude", "model": null, "effort": null},
            "settings": {"enabled": true, "default_agent": "claude", "location": "desk", "cap": 8,
                "permissions": {"hide": "ask", "read": "run", "shell": "ask", "edit": "ask"}},
            "conversation": {"generation": 1, "dropped": 0, "next_id": 3, "items": [
                {"kind": "approval", "id": 1, "card": "command", "command": "touch x",
                    "folder": "/r", "reason": null, "row": "shell",
                    "state": "allowed_conversation", "asks": [{"kind": "row", "value": "shell"}],
                    "subject": null, "refusal": null},
                {"kind": "notice", "id": 2, "notice": "blocked", "agent": "claude", "detail": "x"},
                {"kind": "notice", "id": 3, "notice": "no_ask_mode", "agent": "grok", "detail": null},
            ]},
            "sessions": [],
            "choices": [],
        });
        let restored = decode(&serde_json::to_vec(&older).unwrap()).unwrap();
        assert_eq!(restored.settings.permission, DeskPermission::AllowAll);
        // A word a later Hide wrote restores as the side that asks.
        let mut later = older.clone();
        later["settings"]["permission"] = "ask_more".into();
        let later = decode(&serde_json::to_vec(&later).unwrap()).unwrap();
        assert_eq!(later.settings.permission, DeskPermission::Agent);
        assert_eq!(
            restored.conversation.items,
            [DeskItem::Approval {
                id: 1,
                card: DeskCardKind::Command,
                command: "touch x".into(),
                folder: Some("/r".into()),
                reason: None,
                options: Vec::new(),
                state: DeskApprovalState::Allowed,
                chosen: None,
                answer: None,
                refusal: None,
            }]
        );
        // What this build writes, an older one reads past: no value it
        // does not know.
        let written: serde_json::Value = serde_json::from_slice(
            &encode(&Stored {
                conversation: Conversation {
                    items: vec![DeskItem::Approval {
                        id: 1,
                        card: DeskCardKind::Command,
                        command: "touch x".into(),
                        folder: None,
                        reason: None,
                        options: Vec::new(),
                        state: DeskApprovalState::Allowed,
                        chosen: Some(DeskOptionKind::AllowAlways),
                        answer: Some("Yes, and don't ask again".into()),
                        refusal: None,
                    }],
                    ..Conversation::default()
                },
                ..restored
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(written["conversation"]["items"][0]["state"], "allowed");
    }

    /// The largest card a request can make, its most options each at its
    /// longest, stores and comes back, and its options count against the
    /// conversation's bytes.
    #[test]
    fn a_card_with_the_most_and_longest_options_stores_and_comes_back() {
        use hide_desk::permission::{
            MAX_OPTION_ID_BYTES, MAX_OPTION_NAME_BYTES, MAX_OPTIONS, read_request,
        };

        use crate::model::{DeskApprovalState, DeskCardKind};
        let options: Vec<serde_json::Value> = (0..MAX_OPTIONS + 4)
            .map(|n| {
                serde_json::json!({
                    "optionId": format!("{n:0>MAX_OPTION_ID_BYTES$}"),
                    "kind": "allow_once",
                    "name": "가".repeat(MAX_OPTION_NAME_BYTES),
                })
            })
            .collect();
        let request = read_request(
            &serde_json::json!({"toolCall": {"toolCallId": "t", "kind": "execute", "title": "ls"},
                "options": options}),
            None,
        );
        assert_eq!(request.choices.len(), MAX_OPTIONS);
        let card = DeskItem::Approval {
            id: 1,
            card: DeskCardKind::Command,
            command: "ls".into(),
            folder: None,
            reason: None,
            options: request.choices,
            state: DeskApprovalState::Waiting,
            chosen: None,
            answer: None,
            refusal: None,
        };
        assert!(card.text_bytes() > MAX_OPTIONS * MAX_OPTION_ID_BYTES);
        let mut stored = Stored::default();
        stored.conversation.push(card.clone());
        let restored = decode(&encode(&stored).unwrap()).unwrap();
        assert_eq!(restored.conversation.items, [card]);
    }

    #[test]
    fn the_lanes_widths_come_back_after_a_restart_and_desk_1_still_reads_the_store() {
        /// The store as Desk 1차's reader takes it, kept as it shipped: a
        /// `lane` it requires the fields of, defaulted when absent.
        #[allow(dead_code)]
        #[derive(serde::Deserialize)]
        struct Released {
            conversation: serde_json::Value,
            #[serde(default)]
            lane: Option<ReleasedLane>,
        }
        #[allow(dead_code)]
        #[derive(serde::Deserialize)]
        struct ReleasedLane {
            pinned: bool,
            kept: Vec<serde_json::Value>,
        }
        let stored = Stored {
            lane: crate::model::DeskLane {
                view_width: Some(720),
                session_width: Some(480),
                ..Default::default()
            },
            ..Stored::default()
        };
        let bytes = encode(&stored).unwrap();
        assert_eq!(decode(&bytes).unwrap().lane, stored.lane);
        assert!(
            serde_json::from_slice::<Released>(&bytes).is_ok(),
            "{}",
            String::from_utf8_lossy(&bytes)
        );
    }

    #[test]
    fn a_width_out_of_range_in_the_file_reads_as_unset() {
        let mut file: serde_json::Value =
            serde_json::from_slice(&encode(&Stored::default()).unwrap()).unwrap();
        file["panel_widths"] = serde_json::json!({"view_width": 0, "session_width": 99_999});
        let restored = decode(&serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(restored.lane, crate::model::DeskLane::default());
    }

    /// 한눈에's tab and group folds are read field by field: a value a later
    /// Hide wrote (a third tab, a fold that is no bool) is that field's
    /// default, and the widths and the rest of the store read as they were.
    #[test]
    fn a_glance_field_this_build_cannot_read_is_its_default_and_the_rest_of_the_lane_stays() {
        let mut stored = Stored::default();
        stored.conversation.push(user(1, "kept"));
        let mut file: serde_json::Value =
            serde_json::from_slice(&encode(&stored).unwrap()).unwrap();
        file["panel_widths"] = serde_json::json!({
            "view_width": 720, "session_width": null, "glance_folded": true,
            "glance_tab": "threads",
            "glance_groups": {"turn": "yes", "working": true, "ended": false, "later": true},
        });
        let restored = decode(&serde_json::to_vec(&file).unwrap()).expect("the store reads");
        assert_eq!(
            restored.lane,
            crate::model::DeskLane {
                view_width: Some(720),
                session_width: None,
                glance_folded: true,
                glance_tab: crate::model::GlanceTab::Sessions,
                glance_groups: crate::model::GlanceFolds {
                    turn: false,
                    working: true,
                    ended: false,
                },
            }
        );
        assert_eq!(
            restored.conversation.items.len(),
            1,
            "the conversation stays"
        );
        file["panel_widths"]["glance_groups"] = serde_json::json!(["ended"]);
        let restored = decode(&serde_json::to_vec(&file).unwrap()).expect("the store reads");
        assert_eq!(
            restored.lane.glance_groups,
            crate::model::GlanceFolds::default()
        );
    }

    /// An 끝남 fold that is missing or unreadable starts folded, as 끝남
    /// does, never `bool`'s false, while the folds written as bools and the
    /// rest of the lane and the store read as they were.
    #[test]
    fn a_missing_or_unreadable_ended_fold_starts_folded_and_the_written_folds_stay() {
        let mut stored = Stored::default();
        stored.conversation.push(user(1, "kept"));
        let mut file: serde_json::Value =
            serde_json::from_slice(&encode(&stored).unwrap()).unwrap();
        for ended in [
            None,
            Some(serde_json::json!("later")),
            Some(serde_json::Value::Null),
        ] {
            let mut groups = serde_json::json!({"turn": true, "working": true});
            if let Some(ended) = ended.clone() {
                groups["ended"] = ended;
            }
            file["panel_widths"] = serde_json::json!({
                "view_width": 640, "session_width": null,
                "glance_tab": "files", "glance_groups": groups,
            });
            let restored = decode(&serde_json::to_vec(&file).unwrap()).expect("the store reads");
            assert_eq!(
                restored.lane,
                crate::model::DeskLane {
                    view_width: Some(640),
                    session_width: None,
                    glance_folded: false,
                    glance_tab: crate::model::GlanceTab::Files,
                    glance_groups: crate::model::GlanceFolds {
                        turn: true,
                        working: true,
                        ended: true,
                    },
                },
                "ended {ended:?}"
            );
            assert_eq!(restored.conversation.items.len(), 1, "ended {ended:?}");
        }
    }

    /// B29: a store Desk 1차 wrote, its lane pinned with sixteen kept tabs,
    /// opens as it was but for two unset widths, its conversation, queue,
    /// marks and settings kept, and the next save writes no tab.
    #[test]
    fn an_old_lane_with_a_pin_and_sixteen_kept_tabs_loads_as_two_unset_widths() {
        let output = |index: usize| {
            serde_json::json!({"kind": "output", "job": format!("desk:{index}"), "output": {
                "id": format!("view-{index}"), "kind": "image", "label": "shot.png",
                "thumb": null, "seen": true, "pr_state": null, "device_id": "node",
                "checkout": "/repo", "path": "/repo/shot.png", "url": null,
            }})
        };
        let mission = |index: usize| serde_json::json!({"kind": "mission", "mission": format!("mission-{index}"), "title": "Desk"});
        let kept: Vec<_> = (0..16)
            .map(|index| {
                if index % 2 == 0 {
                    output(index)
                } else {
                    mission(index)
                }
            })
            .collect();
        let mut stored = Stored::default();
        stored.conversation.push(user(1, "what runs now?"));
        stored.settings.cap = 3;
        stored.queue.push(QueuedJob {
            intent: "desk:1".into(),
            command: crate::coordination::Command::Spawn {
                pick: None,
                parent: None,
                machine: None,
                name: "worker".into(),
                intent: "desk:1".into(),
                kind: "codex".into(),
                repo: Some("/r".into()),
                branch: Some("main".into()),
                path: None,
                args: Vec::new(),
                launch: Default::default(),
                task: None,
                brief: Some("go".into()),
                beside: false,
                mission: None,
                plan_node: None,
                plan: None,
                profile: None,
            },
            queued_at_ms: 1,
            state: QueuedState::Waiting,
            code: None,
        });
        stored.marks.push(JobMark {
            intent: "desk:0".into(),
            grouped: true,
            watched: true,
            changed_at_ms: 2,
            ..JobMark::default()
        });
        let mut file: serde_json::Value =
            serde_json::from_slice(&encode(&stored).unwrap()).unwrap();
        file.as_object_mut().unwrap().remove("panel_widths");
        file["lane"] = serde_json::json!({"pinned": true, "kept": kept});

        let restored = decode(&serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(restored, stored);
        let written: serde_json::Value =
            serde_json::from_slice(&encode(&restored).unwrap()).unwrap();
        assert_eq!(written.get("lane"), None);
        assert_eq!(
            written["panel_widths"],
            serde_json::json!({"view_width": null, "session_width": null})
        );
    }
}

#[cfg(test)]
mod restart_tests {
    use super::*;

    /// A spawn a hided ended while it started, read again: no thread will
    /// answer it, so it waits again and the queue goes on, rather than
    /// stand starting for ever and hold the whole queue.
    #[test]
    fn a_start_hided_ended_under_reads_as_waiting_again() {
        let job = |intent: &str, state: QueuedState| QueuedJob {
            intent: intent.into(),
            command: crate::coordination::Command::Spawn {
                pick: None,
                parent: None,
                machine: None,
                name: "worker".into(),
                intent: intent.into(),
                kind: "codex".into(),
                repo: Some("/r".into()),
                branch: Some("main".into()),
                path: None,
                args: Vec::new(),
                launch: Default::default(),
                task: None,
                brief: None,
                beside: false,
                mission: None,
                plan_node: None,
                plan: None,
                profile: None,
            },
            queued_at_ms: 1,
            state,
            code: None,
        };
        let mut stored = Stored::default();
        stored.queue.push(job("starting", QueuedState::Starting));
        stored.queue.push(job("waiting", QueuedState::Waiting));
        stored.queue.push(job("failed", QueuedState::Failed));
        let read = decode(&encode(&stored).unwrap()).unwrap();
        let states: Vec<_> = read
            .queue
            .iter()
            .map(|job| (job.intent.as_str(), job.state))
            .collect();
        assert_eq!(
            states,
            [
                ("starting", QueuedState::Waiting),
                ("waiting", QueuedState::Waiting),
                ("failed", QueuedState::Failed)
            ]
        );
    }

    /// A session a chip names is kept by its pane: the same pane with a new
    /// session takes it in its place, past the cap the one named longest ago
    /// goes, and the store reads back what it kept within the cap (feedback
    /// 2, frame K).
    #[test]
    fn named_sessions_keep_the_newest_sixty_four_and_read_back_within_the_cap() {
        let named = |pane: usize, session: &str| NamedSession {
            pane_id: format!("p{pane}"),
            session: crate::links::ledger::SessionIdentity {
                device_id: "node".into(),
                agent: "codex".into(),
                session_id: session.into(),
            },
            agent_kind: "codex".into(),
            thread: Some(
                serde_json::from_value(serde_json::json!({
                    "agent": "codex", "account": "default", "reference_kind": "id",
                    "reference_value": session, "cwd": "/repo",
                }))
                .unwrap(),
            ),
        };
        let panes = |stored: &Stored| {
            stored
                .named
                .iter()
                .map(|named| named.pane_id.clone())
                .collect::<Vec<_>>()
        };
        let mut stored = Stored::default();
        for pane in 0..=MAX_NAMED_SESSIONS {
            assert!(stored.keep_named(vec![named(pane, &format!("s{pane}"))]));
        }
        assert_eq!(stored.named.len(), MAX_NAMED_SESSIONS);
        assert_eq!(stored.named[0].pane_id, "p1");
        assert_eq!(
            stored.named.last().unwrap().pane_id,
            format!("p{MAX_NAMED_SESSIONS}")
        );
        // Named again, p5 moves to the recent end without a save, and its new session is kept.
        assert!(!stored.keep_named(vec![named(5, "s5")]));
        assert_eq!(stored.named.last().unwrap().pane_id, "p5");
        assert!(stored.keep_named(vec![named(5, "s5-cleared")]));
        assert_eq!(
            stored.named.last().unwrap().session.session_id,
            "s5-cleared"
        );
        // One more named lets p1 go, the least recently named, not p5.
        assert!(stored.keep_named(vec![named(100, "s100")]));
        assert!(
            !panes(&stored).contains(&"p1".to_owned()) && panes(&stored).contains(&"p5".to_owned())
        );
        // A session a chip still names is never the one that goes, though it is the oldest kept.
        let oldest = stored.named[0].clone();
        assert!(stored.keep_named(vec![named(101, "s101"), oldest.clone()]));
        assert_eq!(stored.named.len(), MAX_NAMED_SESSIONS);
        assert!(panes(&stored).contains(&oldest.pane_id));
        // More chips than the cap keep their newest 64, and the same chips again change nothing.
        let many: Vec<_> = (200..200 + MAX_NAMED_SESSIONS + 6)
            .rev()
            .map(|pane| named(pane, &format!("s{pane}")))
            .collect();
        assert!(stored.keep_named(many.clone()));
        assert_eq!(stored.named.len(), MAX_NAMED_SESSIONS);
        assert!(!stored.keep_named(many));
        assert_eq!(
            decode(&encode(&stored).unwrap()).unwrap().named,
            stored.named
        );

        // A file holding more than the cap, from a hand edit, reads its newest.
        let mut over: serde_json::Value =
            serde_json::from_slice(&encode(&stored).unwrap()).unwrap();
        over["named"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(named(99, "s99")).unwrap());
        let read = decode(&serde_json::to_vec(&over).unwrap()).unwrap();
        assert_eq!(read.named.len(), MAX_NAMED_SESSIONS);
        assert_eq!(read.named.last().unwrap().pane_id, "p99");
        assert_eq!(
            read.named[0].pane_id, stored.named[1].pane_id,
            "the oldest kept goes"
        );
    }

    /// A full list makes room by its oldest ended session, not by its
    /// oldest one; with room left, or none ended, nothing goes.
    #[test]
    fn a_full_glance_lets_its_oldest_ended_session_go_first() {
        let mut stored = Stored::default();
        for at in 1..MAX_GLANCE_SESSIONS as u64 {
            stored.keep_glance(&format!("p{at}"), at);
        }
        assert!(!stored.glance_room(|_| true), "room is left");
        stored.keep_glance("p32", 32);
        assert!(!stored.glance_room(|_| false), "none ended");
        assert!(stored.glance_room(|pane| pane == "p7" || pane == "p20"));
        assert_eq!(stored.glance.len(), MAX_GLANCE_SESSIONS - 1);
        assert!(!stored.glance.iter().any(|kept| kept.pane_id == "p7"));
        assert!(stored.glance.iter().any(|kept| kept.pane_id == "p20"));
        assert_eq!(stored.glance[0].pane_id, "p1", "the oldest shown stays");
    }

    /// A later Hide may keep a session with an agent or a field this build
    /// does not know: that entry goes alone, and the conversation, the
    /// queue and the settings a downgrade keeps are read (B29).
    /// 한눈에's sessions: a session asked again moves to its newest time, the
    /// list keeps the newest 32 in order, one older than a full list is not
    /// taken, and the same sessions asked again change nothing to save.
    #[test]
    fn glance_keeps_the_newest_thirty_two_sessions_in_order_without_churn() {
        let mut stored = Stored::default();
        let panes = |stored: &Stored| -> Vec<String> {
            stored
                .glance
                .iter()
                .map(|kept| kept.pane_id.clone())
                .collect()
        };
        for at in 1..=40u64 {
            assert!(stored.keep_glance(&format!("p{at}"), at * 10));
        }
        assert_eq!(stored.glance.len(), MAX_GLANCE_SESSIONS);
        assert_eq!(stored.glance[0].pane_id, "p9", "the oldest eight went");
        assert_eq!(stored.glance.last().unwrap().pane_id, "p40");
        let before = panes(&stored);
        // A session older than every kept one, as Desk's oldest jobs are on
        // each refresh, is not taken and drops nothing.
        assert!(!stored.keep_glance("p1", 10));
        assert_eq!(panes(&stored), before);
        // Asked again at the same or an earlier time, nothing moves.
        assert!(!stored.keep_glance("p20", 200));
        assert!(!stored.keep_glance("p20", 5));
        // Opened beside Desk again later, it is the newest.
        assert!(stored.keep_glance("p20", 1_000));
        assert_eq!(stored.glance.last().unwrap().pane_id, "p20");
        assert_eq!(stored.glance.len(), MAX_GLANCE_SESSIONS);
        // A session started between two kept ones stands between them.
        assert!(stored.keep_glance("between", 155));
        let at = panes(&stored);
        let between = at.iter().position(|pane| pane == "between").unwrap();
        assert_eq!(
            (at[between - 1].as_str(), at[between + 1].as_str()),
            ("p15", "p16")
        );
        assert_eq!(
            decode(&encode(&stored).unwrap()).unwrap().glance,
            stored.glance
        );
    }

    /// A restart reads each 한눈에 entry on its own: one a later build wrote
    /// in a shape this one cannot read goes alone, a pane twice is kept once
    /// at its newest, and past the cap the oldest go; a list that does not
    /// read at all leaves the rest of Desk's store as it was.
    #[test]
    fn glance_is_read_back_entry_by_entry_in_order_and_within_its_cap() {
        let mut file: serde_json::Value =
            serde_json::from_slice(&encode(&Stored::default()).unwrap()).unwrap();
        let mut entries: Vec<serde_json::Value> = (0..40u64)
            .map(|at| serde_json::json!({"pane_id": format!("p{at}"), "at_ms": at}))
            .collect();
        entries.push(serde_json::json!({"pane_id": "p5", "at_ms": 100}));
        entries.push(serde_json::json!({"pane": "later shape"}));
        entries.push(serde_json::json!({"pane_id": "", "at_ms": 101}));
        entries.reverse();
        file["glance"] = serde_json::Value::Array(entries);
        let read = decode(&serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(read.glance.len(), MAX_GLANCE_SESSIONS);
        assert_eq!(read.glance.last().unwrap().pane_id, "p5");
        assert_eq!(read.glance[0].pane_id, "p9");
        assert!(
            read.glance
                .windows(2)
                .all(|pair| pair[0].at_ms <= pair[1].at_ms)
        );
        assert_eq!(
            read.glance
                .iter()
                .filter(|kept| kept.pane_id == "p5")
                .count(),
            1
        );

        file["glance"] = "a shape a later Hide chose".into();
        let mut kept = Stored::default();
        kept.conversation.push(DeskItem::User {
            id: 1,
            text: "kept".into(),
            at_ms: 0,
            mentions: Vec::new(),
            attachments: Vec::new(),
        });
        file["conversation"] = serde_json::to_value(&kept.conversation).unwrap();
        let read = decode(&serde_json::to_vec(&file).unwrap()).unwrap();
        assert!(read.glance.is_empty());
        assert_eq!(read.conversation.items.len(), 1);
    }

    #[test]
    fn a_named_session_this_build_cannot_read_is_dropped_and_the_rest_of_desk_is_restored() {
        let mut stored = Stored::default();
        stored.conversation.push(DeskItem::User {
            id: 1,
            text: "어제 끝난 일 정리해줘".into(),
            at_ms: 0,
            mentions: Vec::new(),
            attachments: Vec::new(),
        });
        let known = serde_json::json!({
            "pane_id": "p1",
            "session": {"device_id": "node", "agent": "codex", "session_id": "s1"},
            "agent_kind": "codex",
            "thread": {"agent": "codex", "account": "default", "reference_kind": "id", "reference_value": "s1", "cwd": "/repo"},
        });
        let mut later_agent = known.clone();
        later_agent["pane_id"] = "p2".into();
        later_agent["thread"]["agent"] = "an-agent-added-later".into();
        let mut later_field = known.clone();
        later_field["pane_id"] = "p3".into();
        later_field["chosen_name"] = "a field added later".into();
        let mut file: serde_json::Value =
            serde_json::from_slice(&encode(&stored).unwrap()).unwrap();
        file["named"] = serde_json::json!([later_agent, known, later_field]);
        let read = decode(&serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(read.conversation.items, stored.conversation.items);
        let panes: Vec<_> = read
            .named
            .iter()
            .map(|named| named.pane_id.as_str())
            .collect();
        assert_eq!(
            panes,
            ["p1", "p3"],
            "an unknown agent drops its entry; an unknown field is read past"
        );

        file["named"] = "a shape a later Hide chose".into();
        let read = decode(&serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(
            (read.conversation.items, read.named),
            (stored.conversation.items, Vec::new())
        );
    }

    /// The waiting messages read back as they were kept, within the line's
    /// cap and the message cap.
    #[test]
    fn queued_messages_are_kept_and_held_to_their_caps_when_read() {
        let message = |id: u64, text: String| crate::model::DeskQueuedMessage {
            id,
            text,
            at_ms: 0,
            mentions: Vec::new(),
            attachments: Vec::new(),
        };
        let mut stored = Stored::default();
        stored
            .queued
            .push(message(1, "x".repeat(MAX_MESSAGE_BYTES + 1)));
        for id in 2..20 {
            stored.queued.push(message(id, format!("m{id}")));
        }
        let read = decode(&encode(&stored).unwrap()).unwrap();
        assert_eq!(read.queued.len(), crate::model::DESK_QUEUED_MAX + 1);
        assert_eq!(read.queued[0].text, "m2");
    }

    /// A waiting message is kept with its files, one of files alone
    /// included (sent with no words, or come back first in line after a
    /// refused start), across hided's restart; a record from before the
    /// files were kept reads as a message with none.
    #[test]
    fn a_waiting_message_s_files_are_kept_across_a_restart_and_an_older_record_reads() {
        let file = |name: &str| crate::model::DeskAttachment {
            name: name.into(),
            bytes: 3,
            path: format!("/state/desk-attachments/r-0-{name}"),
        };
        let mut stored = Stored::default();
        stored.queued.push(crate::model::DeskQueuedMessage {
            id: 1,
            text: "look".into(),
            at_ms: 5,
            mentions: Vec::new(),
            attachments: vec![file("shot.png"), file("install.log")],
        });
        stored.queued.push(crate::model::DeskQueuedMessage {
            id: 2,
            text: String::new(),
            at_ms: 6,
            mentions: Vec::new(),
            attachments: vec![file("only.png")],
        });
        let read = decode(&encode(&stored).unwrap()).unwrap();
        assert_eq!(read.queued, stored.queued);

        // An older build wrote no `attachments`.
        let mut older: serde_json::Value =
            serde_json::from_slice(&encode(&stored).unwrap()).unwrap();
        for message in older["queued"].as_array_mut().unwrap() {
            message.as_object_mut().unwrap().remove("attachments");
        }
        let read = decode(&serde_json::to_vec(&older).unwrap()).unwrap();
        assert_eq!(
            read.queued
                .iter()
                .map(|message| (message.text.as_str(), message.attachments.len()))
                .collect::<Vec<_>>(),
            [("look", 0)]
        );
    }
}
