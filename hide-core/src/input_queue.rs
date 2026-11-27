//! The operator's words held for an agent at work (PRD desk-queue D-01..D-03,
//! D-09, D-12, D-14, D-15, D-17): what a session's input box or a phone's
//! reply sent while the agent's turn ran, kept by the core per pane, so every
//! window and the phone show the same line, and typed one at a time each time
//! the agent rests (`Runtime::send_queued_lines`, under the doorbell's
//! at-rest verdict).
//!
//! This module is the line itself and its file; when a line goes, and how,
//! is the runtime's (`runtime/input_queue.rs`). Every value here is memory
//! the runtime holds under its lock; the file is written off it
//! (`LatestFile`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::latest_file::{LatestFile, read_capped};
use crate::model::{InputQueueCleared, InputQueueLine, InputQueueSnapshot, InputQueueState};

/// Lines one pane holds, Desk's own line's cap (`DESK_QUEUED_MAX`): a send
/// past it is refused and its words stay in the box, since a waiting line is
/// work not done yet and none can be dropped for it (engineering 15).
pub const LINES_MAX: usize = crate::model::DESK_QUEUED_MAX;
/// Files one pane's waiting lines attach together: one send's cap, so the
/// whole line is never more than one send could carry.
pub const FILES_MAX: usize = hide_node_link::attachments::MAX_FILES;
/// Bytes those files hold together, one send's cap.
pub const FILE_BYTES_MAX: u64 = hide_node_link::attachments::MAX_REQUEST_BYTES;
/// How long `모두 지우기` can be taken back (`되돌리기`, D-17).
pub const UNDO_MS: u64 = 10_000;
/// How long a typed line has to show the turn it started: a line whose
/// agent's state moved, or rested this long since, has been taken, and the
/// next may go. A line that starts no turn, such as a command the agent
/// runs at once, moves no state, and would hold the rest otherwise.
pub const SETTLE_MS: u64 = 5_000;

/// Why a line did not go, drawn under it while the line is paused (B23,
/// B24); `contracts/snapshot-wire-enums.json`: `input_queue_failure`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    /// The engine never answered: the line may be in the conversation, so it
    /// is not typed again until the operator says so.
    Uncertain,
    /// The pane's machine could not be reached for its files.
    DeviceUnreachable,
    /// A file the line attached is gone.
    FileMissing,
    /// The pane refused the input.
    Refused,
}

impl Failure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Uncertain => "uncertain",
            Self::DeviceUnreachable => "device_unreachable",
            Self::FileMissing => "file_missing",
            Self::Refused => "refused",
        }
    }
}

/// One held message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub id: u64,
    pub text: String,
    /// The files it attaches, kept in hided's folder for waiting input
    /// (`hide_kit::layout::input_attachments`) until the line goes or is
    /// removed (D-12).
    #[serde(default)]
    pub paths: Vec<String>,
    /// Their bytes together, for the pane's cap.
    #[serde(default)]
    pub bytes: u64,
    /// The id the files are staged on a device under when the line goes: a
    /// UUID, the shape a device's staging takes.
    pub stage_id: String,
    pub at_ms: u64,
    #[serde(default)]
    pub failure: Option<Failure>,
}

/// One pane's line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queue {
    /// The agent the lines were written to: another kind in the pane pauses
    /// them (D-15).
    pub kind: String,
    pub paused: bool,
    pub lines: Vec<Line>,
    /// The engine's state sequence when the line last started waiting: a
    /// turn interrupted at or before it was seen already and pauses nothing.
    #[serde(default)]
    pub baseline_seq: Option<u64>,
    /// The line on its way to the pane.
    #[serde(skip)]
    pub typing: Option<u64>,
    /// The last line typed, until the agent shows its turn (`SETTLE_MS`).
    #[serde(skip)]
    pub typed: Option<Typed>,
    /// What `모두 지우기` took, while it can be put back.
    #[serde(skip)]
    pub cleared: Option<Cleared>,
    /// Why the head line waits, logged once per change (D-18).
    #[serde(skip)]
    pub hold: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Typed {
    pub seq: Option<u64>,
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cleared {
    pub lines: Vec<Line>,
    pub until_ms: u64,
}

/// What the agent in a pane does now, as far as where new words go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    AtRest,
    Working,
    /// A question or a permission waits on the operator (D-23).
    Asking,
}

/// Where the operator's new words go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Typed now, as before the queue.
    Now,
    /// Held at the end of the pane's line.
    Queue,
}

/// How a typed line ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    /// The pane took it.
    Taken,
    /// The agent moved before the words went: nothing was typed, and the
    /// line waits at the head for the next rest (D-14, B22).
    NotNow,
    Failed(Failure),
}

/// Every pane's line, as kept and saved.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queues {
    #[serde(default)]
    pub next_id: u64,
    /// By the pane's id as the core names it (a device's pane as
    /// `remote:<device>:pane:<id>`).
    #[serde(default)]
    pub panes: BTreeMap<String, Queue>,
}

/// A send the line refused, its words staying in the box (B8, B9).
pub const FULL: &str = "queue_full";
pub const FILES_FULL: &str = "queue_files_full";

impl Queues {
    /// Where words sent to `pane` go: behind a line still waiting, behind a
    /// line just typed whose turn has not shown, or behind a turn under way;
    /// typed now otherwise. A paused line holds nothing back: new words go
    /// to an agent at rest at once (D-09), and to a working one at the end
    /// of the paused line. An agent that asks takes them as its answer
    /// (D-23) unless a line waits ahead of them.
    pub fn route(&self, pane: &str, agent: Agent, seq: Option<u64>, now: u64) -> Route {
        let queue = self.panes.get(pane);
        let waiting = queue.is_some_and(|queue| {
            !queue.paused
                && (!queue.lines.is_empty() || queue.typing.is_some() || queue.settling(seq, now))
        });
        if waiting || agent == Agent::Working {
            Route::Queue
        } else {
            Route::Now
        }
    }

    /// Adds `text` with its files to the end of `pane`'s line, refused past
    /// the line's caps. A new line starts at the engine's sequence `seq`.
    #[allow(clippy::too_many_arguments)]
    pub fn push(
        &mut self,
        pane: &str,
        kind: &str,
        text: String,
        paths: Vec<String>,
        bytes: u64,
        stage_id: String,
        seq: Option<u64>,
        now: u64,
    ) -> Result<u64, &'static str> {
        let queue = self.panes.entry(pane.to_owned()).or_insert_with(|| Queue {
            kind: kind.to_owned(),
            paused: false,
            lines: Vec::new(),
            baseline_seq: seq,
            typing: None,
            typed: None,
            cleared: None,
            hold: None,
        });
        if queue.lines.len() >= LINES_MAX {
            return Err(FULL);
        }
        let (files, held) = queue.files();
        if files + paths.len() > FILES_MAX || held.saturating_add(bytes) > FILE_BYTES_MAX {
            return Err(FILES_FULL);
        }
        if queue.lines.is_empty() && !queue.paused {
            queue.kind = kind.to_owned();
            queue.baseline_seq = seq;
        }
        self.next_id += 1;
        let id = self.next_id;
        queue.lines.push(Line {
            id,
            text,
            paths,
            bytes,
            stage_id,
            at_ms: now,
            failure: None,
        });
        Ok(id)
    }

    /// ✕ on a line: it never goes. A line on its way has no ✕ (B7).
    pub fn remove(&mut self, pane: &str, id: u64) -> bool {
        let Some(queue) = self.panes.get_mut(pane) else {
            return false;
        };
        if queue.typing == Some(id) {
            return false;
        }
        let before = queue.lines.len();
        queue.lines.retain(|line| line.id != id);
        let removed = queue.lines.len() != before;
        self.forget_empty();
        removed
    }

    /// The line stops going (■, a turn interrupted in the terminal, another
    /// agent in the pane). Whether it changed.
    pub fn pause(&mut self, pane: &str) -> bool {
        match self.panes.get_mut(pane) {
            Some(queue) if !queue.paused && !queue.lines.is_empty() => {
                queue.paused = true;
                true
            }
            _ => false,
        }
    }

    /// `▶ 이어 보내기`: the paused line waits again, from the engine's
    /// sequence `seq`, so the interruption that paused it pauses it no more
    /// (B15). Each line's failure goes with the pause.
    pub fn resume(&mut self, pane: &str, kind: &str, seq: Option<u64>) -> bool {
        let Some(queue) = self.panes.get_mut(pane).filter(|queue| queue.paused) else {
            return false;
        };
        queue.paused = false;
        queue.kind = kind.to_owned();
        queue.baseline_seq = seq;
        for line in &mut queue.lines {
            line.failure = None;
        }
        true
    }

    /// `모두 지우기`: every line not on its way goes, and can be put back for
    /// `UNDO_MS` (B17). An earlier clear not taken back is let go.
    pub fn clear(&mut self, pane: &str, now: u64) -> bool {
        let Some(queue) = self.panes.get_mut(pane) else {
            return false;
        };
        let typing = queue.typing;
        let (kept, taken): (Vec<_>, Vec<_>) = std::mem::take(&mut queue.lines)
            .into_iter()
            .partition(|line| Some(line.id) == typing);
        queue.lines = kept;
        if taken.is_empty() {
            return false;
        }
        queue.cleared = Some(Cleared {
            lines: taken,
            until_ms: now + UNDO_MS,
        });
        // With the group gone, words sent next wait as a new line does.
        if queue.lines.is_empty() {
            queue.paused = false;
        }
        true
    }

    /// `되돌리기`: the cleared lines come back first, in their order, paused
    /// (B17); refused once the line could not hold them.
    pub fn restore(&mut self, pane: &str, now: u64) -> Result<bool, &'static str> {
        let Some(queue) = self.panes.get_mut(pane) else {
            return Ok(false);
        };
        let Some(cleared) = queue.cleared.take() else {
            return Ok(false);
        };
        if now >= cleared.until_ms {
            return Ok(false);
        }
        if cleared.lines.len() + queue.lines.len() > LINES_MAX {
            queue.cleared = Some(cleared);
            return Err(FULL);
        }
        let mut lines = cleared.lines;
        lines.append(&mut queue.lines);
        queue.lines = lines;
        queue.paused = true;
        Ok(true)
    }

    /// The line due to go now in `pane`: `id` when the operator pressed a
    /// paused line's `보내기`, else the head of a line that waits, once the
    /// last typed line showed its turn. Marks it on its way.
    pub fn take(&mut self, pane: &str, id: Option<u64>, seq: Option<u64>, now: u64) -> Option<Line> {
        let queue = self.panes.get_mut(pane)?;
        if queue.typing.is_some() {
            return None;
        }
        let line = match id {
            Some(id) => queue.lines.iter().find(|line| line.id == id)?,
            None if queue.paused || queue.settling(seq, now) => return None,
            None => queue.lines.first()?,
        }
        .clone();
        queue.typing = Some(line.id);
        Some(line)
    }

    /// Whether `pane` has a line due at its next rest, without taking it.
    pub fn due(&self, pane: &str, seq: Option<u64>, now: u64) -> bool {
        self.panes.get(pane).is_some_and(|queue| {
            !queue.paused
                && queue.typing.is_none()
                && !queue.lines.is_empty()
                && !queue.settling(seq, now)
        })
    }

    /// How the line `id` on its way to `pane` ended; the engine's sequence
    /// `seq` is the one it was typed under. Whether anything changed.
    pub fn settle(&mut self, pane: &str, id: u64, sent: Sent, seq: Option<u64>, now: u64) -> bool {
        let Some(queue) = self.panes.get_mut(pane).filter(|queue| queue.typing == Some(id)) else {
            return false;
        };
        queue.typing = None;
        match sent {
            Sent::Taken => {
                queue.lines.retain(|line| line.id != id);
                queue.typed = Some(Typed { seq, at_ms: now });
            }
            Sent::NotNow => {}
            Sent::Failed(failure) => {
                if let Some(line) = queue.lines.iter_mut().find(|line| line.id == id) {
                    line.failure = Some(failure);
                }
                queue.paused = true;
            }
        }
        self.forget_empty();
        true
    }

    /// Follows the agent in `pane`: another kind there, or none, pauses its
    /// line (D-15, B26); a turn the agent reports interrupted at a sequence
    /// past the line's start pauses it too (D-08, B13). The same agent's new
    /// session goes on. Whether it changed.
    pub fn follow(&mut self, pane: &str, kind: Option<&str>, interrupted_at: Option<u64>) -> bool {
        let Some(queue) = self.panes.get_mut(pane) else {
            return false;
        };
        if queue.paused || queue.lines.is_empty() {
            return false;
        }
        let gone = kind.is_none_or(|kind| {
            hide_agent_adapter::canonical_kind(kind) != hide_agent_adapter::canonical_kind(&queue.kind)
        });
        let interrupted = interrupted_at
            .is_some_and(|seq| queue.baseline_seq.is_none_or(|baseline| seq > baseline));
        if gone || interrupted {
            queue.paused = true;
            return true;
        }
        false
    }

    /// Lets go of what `모두 지우기` took once it can no longer come back, and
    /// of an emptied line. Whether anything went.
    pub fn expire(&mut self, now: u64) -> bool {
        let mut changed = false;
        for queue in self.panes.values_mut() {
            if queue.cleared.as_ref().is_some_and(|cleared| now >= cleared.until_ms) {
                queue.cleared = None;
                changed = true;
            }
        }
        self.forget_empty() || changed
    }

    /// Lets go of the lines of every pane `keep` does not keep: the pane
    /// closed, and its line goes with it (D-15, B27). Whether any went.
    pub fn follow_panes(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let before = self.panes.len();
        self.panes.retain(|pane, _| keep(pane));
        self.panes.len() != before
    }

    /// Whether a line waits in `pane` (not paused): a letter's bell waits
    /// behind the operator's own words (D-13, B29).
    pub fn waits(&self, pane: &str) -> bool {
        self.panes
            .get(pane)
            .is_some_and(|queue| !queue.paused && (!queue.lines.is_empty() || queue.typing.is_some()))
    }

    /// Every file a waiting or cleared line attaches, which hided's folder
    /// for waiting input never removes.
    pub fn files_in_use(&self) -> Vec<String> {
        self.panes
            .values()
            .flat_map(|queue| {
                queue
                    .lines
                    .iter()
                    .chain(queue.cleared.iter().flat_map(|cleared| cleared.lines.iter()))
            })
            .flat_map(|line| line.paths.iter().cloned())
            .collect()
    }

    /// What a screen draws of `pane`'s line; none while it holds nothing to
    /// draw.
    pub fn snapshot(&self, pane: &str) -> Option<InputQueueSnapshot> {
        let queue = self.panes.get(pane)?;
        if queue.lines.is_empty() && queue.cleared.is_none() {
            return None;
        }
        Some(InputQueueSnapshot {
            state: if queue.paused {
                InputQueueState::Paused
            } else {
                InputQueueState::Waiting
            },
            lines: queue
                .lines
                .iter()
                .map(|line| InputQueueLine {
                    id: line.id,
                    text: line.text.clone(),
                    files: line.paths.iter().map(|path| file_name(path)).collect(),
                    sending: queue.typing == Some(line.id),
                    failure: line.failure,
                })
                .collect(),
            max: LINES_MAX,
            full: queue.lines.len() >= LINES_MAX,
            cleared: queue.cleared.as_ref().map(|cleared| InputQueueCleared {
                count: cleared.lines.len(),
                until_unix_ms: cleared.until_ms,
            }),
        })
    }

    fn forget_empty(&mut self) -> bool {
        let before = self.panes.len();
        self.panes.retain(|_, queue| {
            !queue.lines.is_empty() || queue.cleared.is_some() || queue.typing.is_some()
        });
        self.panes.len() != before
    }

    /// The lines as the file keeps them: a line on its way is kept, since
    /// whether it landed is not known until its answer.
    fn bounded(mut self) -> Self {
        self.panes.retain(|pane, queue| {
            queue.lines.retain(|line| {
                !line.text.trim().is_empty() || !line.paths.is_empty()
            });
            queue.lines.truncate(LINES_MAX);
            pane.len() <= 512 && !queue.lines.is_empty()
        });
        self
    }
}

impl Queue {
    /// Whether the last typed line has not shown its turn yet.
    fn settling(&self, seq: Option<u64>, now: u64) -> bool {
        self.typed
            .is_some_and(|typed| typed.seq == seq && now.saturating_sub(typed.at_ms) < SETTLE_MS)
    }

    /// The files the lines attach, and their bytes.
    fn files(&self) -> (usize, u64) {
        self.lines.iter().fold((0, 0), |(files, bytes), line| {
            (files + line.paths.len(), bytes.saturating_add(line.bytes))
        })
    }
}

/// The name a file had, from its kept path: hided keeps a file as
/// `<stage>-<name>`, the stage a 36-character UUID.
fn file_name(path: &str) -> String {
    let base = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path);
    match base.split_at_checked(37) {
        Some((stage, name))
            if stage.ends_with('-')
                && hide_node_link::attachments::valid_request_id(&stage[..36])
                && !name.is_empty() =>
        {
            name.to_owned()
        }
        _ => base.to_owned(),
    }
}

const SCHEMA_VERSION: u32 = 1;
const READ_CAP: u64 = 4 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct File {
    schema_version: u32,
    #[serde(flatten)]
    queues: Queues,
}

fn encode(queues: &Queues) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&File {
        schema_version: SCHEMA_VERSION,
        queues: queues.clone(),
    })
    .map_err(|error| format!("could not be encoded: {error}"))
}

fn decode(bytes: &[u8]) -> Result<Queues, String> {
    let file: File =
        serde_json::from_slice(bytes).map_err(|error| format!("could not be decoded: {error}"))?;
    if file.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "has schema version {}, expected {SCHEMA_VERSION}",
            file.schema_version
        ));
    }
    Ok(file.queues.bounded())
}

/// The lines' file (`hide_kit::layout::input_queue`), written off the lock,
/// so a restart of hided keeps them (D-12, B21).
pub(crate) struct InputQueueStore {
    path: PathBuf,
    file: LatestFile<Queues>,
}

impl InputQueueStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            file: LatestFile::new(
                path.clone(),
                "hide-input-queue-save",
                "input_queue",
                "input_queue",
                encode,
            ),
            path,
        }
    }

    /// The last run's lines, read before the runtime lock is taken. A file
    /// that cannot be read starts with none and says why in the log.
    pub(crate) fn restore(&self) -> Queues {
        match read_capped(&self.path, READ_CAP)
            .and_then(|bytes| bytes.map(|bytes| decode(&bytes)).transpose())
        {
            Ok(queues) => queues.unwrap_or_default(),
            Err(message) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "input_queue",
                    "kind": "input_queue.restore_failed",
                    "message": message,
                }));
                Queues::default()
            }
        }
    }

    pub(crate) fn save(&self, queues: Queues) {
        self.file.save(queues);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANE: &str = "pane";
    const STAGE: &str = "0b9f6c1e-4a3d-4c2b-9d8e-7f6a5b4c3d2e";

    fn push(queues: &mut Queues, text: &str) -> Result<u64, &'static str> {
        queues.push(
            PANE,
            "claude",
            text.to_owned(),
            Vec::new(),
            0,
            STAGE.to_owned(),
            Some(1),
            0,
        )
    }

    fn texts(queues: &Queues) -> Vec<String> {
        queues.panes[PANE]
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect()
    }

    /// B1, B2, B19, D-09, D-23: words wait behind a working agent and behind
    /// a line still waiting; an agent at rest or asking takes them now; a
    /// paused line holds nothing back from an agent at rest.
    #[test]
    fn words_wait_behind_a_turn_or_a_waiting_line_and_go_now_otherwise() {
        let mut queues = Queues::default();
        assert_eq!(queues.route(PANE, Agent::AtRest, Some(1), 0), Route::Now);
        assert_eq!(queues.route(PANE, Agent::Asking, Some(1), 0), Route::Now);
        assert_eq!(queues.route(PANE, Agent::Working, Some(1), 0), Route::Queue);
        push(&mut queues, "first").unwrap();
        assert_eq!(queues.route(PANE, Agent::Asking, Some(1), 0), Route::Queue);
        assert_eq!(queues.route(PANE, Agent::AtRest, Some(1), 0), Route::Queue);
        assert!(queues.pause(PANE));
        assert_eq!(queues.route(PANE, Agent::AtRest, Some(1), 0), Route::Now);
        assert_eq!(queues.route(PANE, Agent::Working, Some(1), 0), Route::Queue);
    }

    /// B3: the head goes first, one at a time; the next waits until the
    /// typed line shows its turn, or `SETTLE_MS` passed with none.
    #[test]
    fn lines_go_one_at_a_time_in_order_after_the_turn_before_shows() {
        let mut queues = Queues::default();
        let first = push(&mut queues, "first").unwrap();
        let second = push(&mut queues, "second").unwrap();
        let line = queues.take(PANE, None, Some(1), 100).unwrap();
        assert_eq!(line.id, first);
        assert!(queues.take(PANE, None, Some(1), 100).is_none(), "one on its way at a time");
        assert!(queues.settle(PANE, first, Sent::Taken, Some(1), 100));
        assert_eq!(texts(&queues), ["second"]);
        assert!(!queues.due(PANE, Some(1), 200), "the turn it started has not shown");
        assert_eq!(
            queues.route(PANE, Agent::AtRest, Some(1), 200),
            Route::Queue,
            "new words wait behind it too"
        );
        assert!(queues.due(PANE, Some(3), 200), "the agent's state moved");
        assert!(queues.due(PANE, Some(1), 100 + SETTLE_MS), "or it settled");
        assert_eq!(queues.take(PANE, None, Some(3), 200).unwrap().id, second);
        assert!(queues.settle(PANE, second, Sent::Taken, Some(3), 200));
        assert!(queues.snapshot(PANE).is_none(), "the last line gone, the group goes");
    }

    /// B8, B9: the ninth line is refused, and so is a line whose files would
    /// take the pane past one send's files.
    #[test]
    fn a_full_line_refuses_and_files_count_together() {
        let mut queues = Queues::default();
        for index in 0..LINES_MAX {
            push(&mut queues, &format!("line {index}")).unwrap();
        }
        assert_eq!(push(&mut queues, "ninth"), Err(FULL));
        assert!(queues.snapshot(PANE).unwrap().full);
        let mut queues = Queues::default();
        let paths = |count: usize| (0..count).map(|index| format!("/f{index}")).collect::<Vec<_>>();
        queues
            .push(PANE, "claude", "a".into(), paths(6), 10, STAGE.into(), None, 0)
            .unwrap();
        assert_eq!(
            queues.push(PANE, "claude", "b".into(), paths(3), 10, STAGE.into(), None, 0),
            Err(FILES_FULL)
        );
        assert_eq!(
            queues.push(
                PANE,
                "claude",
                "c".into(),
                paths(1),
                FILE_BYTES_MAX,
                STAGE.into(),
                None,
                0
            ),
            Err(FILES_FULL)
        );
    }

    /// B7: ✕ takes a line out; the line on its way has none.
    #[test]
    fn a_removed_line_never_goes_and_the_line_on_its_way_stays() {
        let mut queues = Queues::default();
        let first = push(&mut queues, "first").unwrap();
        let second = push(&mut queues, "second").unwrap();
        queues.take(PANE, None, Some(1), 0).unwrap();
        assert!(!queues.remove(PANE, first));
        assert!(queues.remove(PANE, second));
        assert_eq!(texts(&queues), ["first"]);
    }

    /// B13, B15, B26: an interrupted turn and another agent pause the line;
    /// resuming starts from the sequence then, so the same interruption
    /// pauses it no more.
    #[test]
    fn an_interrupt_or_another_agent_pauses_and_resume_waits_again() {
        let mut queues = Queues::default();
        push(&mut queues, "first").unwrap();
        assert!(!queues.follow(PANE, Some("claude"), Some(1)), "seen when the line started");
        assert!(queues.follow(PANE, Some("claude"), Some(2)));
        assert!(queues.panes[PANE].paused);
        assert!(queues.take(PANE, None, Some(2), 0).is_none(), "a paused line does not go");
        assert!(queues.resume(PANE, "claude", Some(2)));
        assert!(!queues.follow(PANE, Some("claude"), Some(2)));
        assert!(queues.follow(PANE, Some("codex"), None), "another agent");
        assert!(queues.resume(PANE, "claude", Some(2)));
        assert!(queues.follow(PANE, None, None), "no agent");
    }

    /// B16: a paused line's `보내기` sends that line alone, and the rest stay
    /// paused.
    #[test]
    fn a_paused_lines_send_takes_that_line_alone() {
        let mut queues = Queues::default();
        push(&mut queues, "first").unwrap();
        let second = push(&mut queues, "second").unwrap();
        queues.pause(PANE);
        let line = queues.take(PANE, Some(second), Some(1), 0).unwrap();
        assert_eq!(line.text, "second");
        queues.settle(PANE, second, Sent::Taken, Some(1), 0);
        assert_eq!(texts(&queues), ["first"]);
        assert!(queues.panes[PANE].paused);
    }

    /// B22, B23, B24: a line the agent moved ahead of stays first and
    /// waiting; one whose landing is not known, or that failed, pauses the
    /// line with its reason and is never typed on its own again.
    #[test]
    fn a_line_not_typed_stays_first_and_a_failure_pauses_with_its_reason() {
        let mut queues = Queues::default();
        let first = push(&mut queues, "first").unwrap();
        queues.take(PANE, None, Some(1), 0).unwrap();
        queues.settle(PANE, first, Sent::NotNow, Some(1), 0);
        assert!(!queues.panes[PANE].paused);
        assert_eq!(queues.take(PANE, None, Some(1), 0).unwrap().id, first);
        queues.settle(PANE, first, Sent::Failed(Failure::Uncertain), Some(1), 0);
        let snapshot = queues.snapshot(PANE).unwrap();
        assert_eq!(snapshot.state, InputQueueState::Paused);
        assert_eq!(snapshot.lines[0].failure, Some(Failure::Uncertain));
        assert!(queues.take(PANE, None, Some(5), 0).is_none());
        queues.resume(PANE, "claude", Some(5));
        assert_eq!(queues.snapshot(PANE).unwrap().lines[0].failure, None);
    }

    /// B17: 모두 지우기 can be taken back for ten seconds, the same lines in
    /// the same order, paused.
    #[test]
    fn cleared_lines_come_back_in_order_within_the_undo_window() {
        let mut queues = Queues::default();
        push(&mut queues, "first").unwrap();
        push(&mut queues, "second").unwrap();
        assert!(queues.clear(PANE, 1_000));
        let snapshot = queues.snapshot(PANE).unwrap();
        assert!(snapshot.lines.is_empty());
        assert_eq!(snapshot.cleared.unwrap().count, 2);
        assert_eq!(queues.restore(PANE, 1_000 + UNDO_MS - 1), Ok(true));
        assert_eq!(texts(&queues), ["first", "second"]);
        assert!(queues.panes[PANE].paused);
        queues.clear(PANE, 2_000);
        assert!(queues.expire(2_000 + UNDO_MS));
        assert!(queues.snapshot(PANE).is_none());
        assert_eq!(queues.restore(PANE, 2_000 + UNDO_MS), Ok(false));
    }

    /// B21: the file keeps the lines and whether they were paused; what was
    /// on its way is kept too, since its landing is not known.
    #[test]
    fn the_file_keeps_the_lines_and_their_state() {
        let mut queues = Queues::default();
        push(&mut queues, "first").unwrap();
        push(&mut queues, "second").unwrap();
        queues.pause(PANE);
        queues.take(PANE, Some(1), Some(1), 0);
        let read = decode(&encode(&queues).unwrap()).unwrap();
        assert_eq!(read.panes[PANE].lines, queues.panes[PANE].lines);
        assert!(read.panes[PANE].paused);
        assert_eq!(read.panes[PANE].typing, None);
        assert_eq!(read.next_id, queues.next_id);
    }

    #[test]
    fn a_file_shows_the_name_it_was_attached_by() {
        assert_eq!(file_name(&format!("/state/input/{STAGE}-notes.md")), "notes.md");
        assert_eq!(file_name("C:\\held\\shot.png"), "shot.png");
    }
}
