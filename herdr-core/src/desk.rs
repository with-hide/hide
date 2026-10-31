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
    DESK_CAP_MAX, DeskCarried, DeskChoiceSnapshot, DeskItem, DeskSelection, DeskSettings,
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
    /// Desk's own `CODEX_HOME` (`hide_kit::layout::desk_codex_home`).
    pub codex_home: PathBuf,
    /// The operator's own Codex sign-in, which Desk's Codex reads through a
    /// link (`hide_desk::codex::operator_sign_in`), or why it has none.
    pub codex_sign_in: Result<PathBuf, String>,
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
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
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
    /// The lane's pin and kept tabs (B31, B32).
    #[serde(default)]
    pub lane: crate::model::DeskLane,
}

/// Spawns the queue holds; one past it is refused `desk_queue_full`.
pub(crate) const MAX_QUEUE: usize = 64;
/// Jobs remembered; the oldest ended ones are forgotten first.
pub(crate) const MAX_MARKS: usize = 256;

/// A spawn Desk asked for while every place was taken.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct QueuedJob {
    pub intent: String,
    /// The spawn as Desk asked it, run as `hide:desk` when a place frees.
    pub command: crate::coordination::Command,
    pub queued_at_ms: u64,
    pub state: QueuedState,
    /// The code its start failed with.
    #[serde(default)]
    pub code: Option<String>,
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
    /// Its tab was put in the `◆ Desk` group once; leaving it after that
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
    stored.conversation.trim();
    stored.settings.cap = stored.settings.cap.clamp(1, DESK_CAP_MAX);
    stored.sessions.truncate(AdapterId::all().count());
    stored.queue.truncate(MAX_QUEUE);
    let excess = stored.marks.len().saturating_sub(MAX_MARKS);
    stored.marks.drain(..excess);
    let excess = stored
        .lane
        .kept
        .len()
        .saturating_sub(crate::model::DESK_KEPT_TABS_MAX);
    stored.lane.kept.drain(..excess);
    stored.choices.truncate(AdapterId::all().count());
    for (_, choices) in &mut stored.choices {
        choices.models.truncate(hide_desk::session::MAX_CHOICES);
        choices.efforts.truncate(hide_desk::session::MAX_CHOICES);
    }
    Ok(stored)
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

    fn user(id: u64, text: &str) -> DeskItem {
        DeskItem::User {
            id,
            text: text.into(),
            at_ms: 0,
            mentions: Vec::<DeskMention>::new(),
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
        });
        let carried = conversation.carried().unwrap();
        assert_eq!(carried, "Operator: what runs now?\nCodex: two sessions");
    }

    #[test]
    fn a_store_from_a_newer_schema_is_not_read() {
        let bytes = br#"{"schema_version": 2}"#;
        assert!(decode(bytes).is_err());
        let stored = Stored::default();
        assert_eq!(decode(&encode(&stored).unwrap()).unwrap(), stored);
    }

    #[test]
    fn the_lane_comes_back_after_a_restart_within_its_cap() {
        let tab = |index: usize| crate::model::DeskKeptTab::Mission {
            mission: format!("mission-{index}"),
            title: "Desk".into(),
        };
        let mut stored = Stored::default();
        stored.lane.pinned = true;
        stored.lane.kept = (0..crate::model::DESK_KEPT_TABS_MAX + 2).map(tab).collect();
        let restored = decode(&encode(&stored).unwrap()).unwrap();
        assert!(restored.lane.pinned);
        assert_eq!(restored.lane.kept.len(), crate::model::DESK_KEPT_TABS_MAX);
        assert_eq!(
            restored.lane.kept[0],
            tab(2),
            "the oldest kept tabs go first"
        );
    }
}
