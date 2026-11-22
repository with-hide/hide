use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ledger::{Ledger, encoded_len};
use super::{Actor, FILE_LIMIT, INACTIVITY_MS, QUESTION_WINDOW_MS, SECOND_WARNING_MS, WATCH_LIMIT};

/// The inactivity episode and external notification reservation survive the
/// watch itself, so stopping/restarting or sibling watches cannot resend.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WarningReceipt {
    pub target: Actor,
    pub activity_at_unix_ms: u64,
    pub ordinal: u8,
    #[serde(default)]
    pub parent_notified: bool,
}

impl WarningReceipt {
    pub(crate) fn valid(&self) -> bool {
        self.target.valid() && matches!(self.ordinal, 1 | 2)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Watch {
    pub id: String,
    pub parent: Actor,
    pub target: Actor,
    pub last_activity_at_unix_ms: u64,
    pub first_warning_at_unix_ms: Option<u64>,
    pub warning_count: u8,
    pub activity_failures: u32,
    pub last_status: String,
    pub last_state_change_seq: Option<u64>,
    pub status_changed_at_unix_ms: u64,
    #[serde(default)]
    pub generation: u64,
    /// The operator was told that the observer did not look at this quiet
    /// episode's first warning (`human::claim`); the episode's activity
    /// clears it. Left out of the file while false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub operator_notified: bool,
    /// The engine connection `last_status` and `last_state_change_seq` were
    /// read on ([`StateRead::engine`]). Memory only: a core that starts again
    /// connects to every engine anew, so a loaded watch has none, and its first
    /// reading compares by status alone.
    #[serde(skip)]
    #[schemars(skip)]
    pub last_engine: Option<String>,
}

/// One engine's reading of an agent's state: the connection it was read on
/// (`agent_state::ReadConnection::mark`), its status and its state sequence.
/// The one rule for whether an agent's state moved between two readings,
/// the watch's activity and the delivery observation's clock alike.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StateRead<'a> {
    pub engine: Option<&'a str>,
    pub status: &'a str,
    pub seq: Option<u64>,
}

impl StateRead<'_> {
    /// Whether the reading says where the agent is. A restored pane reports
    /// sequence 0, then `unknown` until its holder has read the screen again
    /// (status-model.md, Across an engine restart); neither is a state.
    pub(crate) fn settled(&self) -> bool {
        self.seq != Some(0) && self.status != "unknown"
    }

    /// Whether this reading is another state than `last`: another status, or
    /// another sequence on the same engine. A sequence compares only with one
    /// read on the same engine, since the next engine counts from the start
    /// again; a watch loaded from the file names no engine, so its first
    /// reading compares by status.
    pub(crate) fn moved_from(&self, last: &StateRead<'_>) -> bool {
        self.status != last.status || (self.engine == last.engine && self.seq != last.seq)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct View {
    pub id: String,
    pub parent_pane_id: String,
    pub target_pane_id: String,
    pub warning_count: u8,
    pub first_warning_at_unix_ms: Option<u64>,
    pub last_activity_at_unix_ms: u64,
}

impl Watch {
    /// The last settled reading the watch took of its target.
    pub(crate) fn last_read(&self) -> StateRead<'_> {
        StateRead {
            engine: self.last_engine.as_deref(),
            status: &self.last_status,
            seq: self.last_state_change_seq,
        }
    }

    pub fn view(&self) -> View {
        View {
            id: self.id.clone(),
            parent_pane_id: self.parent.pane_id.clone(),
            target_pane_id: self.target.pane_id.clone(),
            warning_count: self.warning_count,
            first_warning_at_unix_ms: self.first_warning_at_unix_ms,
            last_activity_at_unix_ms: self.last_activity_at_unix_ms,
        }
    }

    pub fn valid(&self) -> bool {
        super::valid_key(&self.id)
            && self.parent.valid()
            && self.target.valid()
            && self.warning_count <= 2
            && super::valid_key(&self.last_status)
            && (self.warning_count == 0) == self.first_warning_at_unix_ms.is_none()
    }
}

/// What a watch's observer reads of its quiet target in its status cell
/// (`runtime::status_cells`), as of the warning made at `warned`.
pub(crate) fn warning_line(watch: &Watch, warned: u64) -> String {
    let quiet = warned.saturating_sub(watch.last_activity_at_unix_ms);
    // The first line is what a bell names after the target's name.
    let summary = if watch.warning_count == 1 && quiet < 2 * QUESTION_WINDOW_MS {
        format!("Stopped with a question (watch {}).", watch.id)
    } else {
        format!(
            "No activity for {} minutes (watch {}).",
            quiet / 60_000,
            watch.id
        )
    };
    format!(
        "{summary}\nTarget {} in pane {}: read it with `hide pane read {}` before deciding whether it is blocked.",
        watch.target.name, watch.target.pane_id, watch.target.pane_id
    )
}

/// Code-owned observers keep their existing thirty-minute window.
pub fn inactivity_window(watch: &Watch) -> u64 {
    if watch.parent.code_owned() {
        30 * 60_000
    } else {
        INACTIVITY_MS
    }
}

pub fn start(
    ledger: &mut Ledger,
    parent: &Actor,
    target: &Actor,
    activity: u64,
) -> Result<Watch, String> {
    parent.require_native_identity()?;
    if let Some(watch) = ledger
        .watches
        .iter()
        .find(|watch| watch.parent.same_identity(parent) && watch.target.same_identity(target))
    {
        return Ok(watch.clone());
    }
    if ledger.watches.len() >= WATCH_LIMIT {
        return Err("capacity".into());
    }
    let sequence = ledger.next_id;
    ledger.next_id = sequence.checked_add(1).ok_or("capacity")?;
    let watch = Watch {
        id: format!("watch-{sequence}"),
        parent: crate::coordination::named(ledger, parent),
        target: crate::coordination::named(ledger, target),
        last_activity_at_unix_ms: activity,
        first_warning_at_unix_ms: None,
        warning_count: 0,
        activity_failures: 0,
        last_status: "unknown".into(),
        last_state_change_seq: None,
        status_changed_at_unix_ms: activity,
        generation: 0,
        operator_notified: false,
        last_engine: None,
    };
    ledger.watches.push(watch.clone());
    Ok(watch)
}

pub fn stop(ledger: &mut Ledger, parent: &Actor, id: &str) -> Result<(), String> {
    let index = ledger
        .watches
        .iter()
        .position(|watch| watch.id == id && watch.parent.same_identity(parent))
        .ok_or("watch_unavailable")?;
    ledger.watches.remove(index);
    Ok(())
}

pub fn assign(
    ledger: &mut Ledger,
    parent: &Actor,
    id: &str,
    observer: &Actor,
    expected_generation: Option<u64>,
    approval: Option<&str>,
) -> Result<Watch, String> {
    observer.require_native_identity()?;
    if approval.is_some_and(|text| !valid_approval(text)) {
        return Err("invalid_approval".into());
    }
    let approved_observer = parent.same_identity(observer) && approval.is_some();
    if approved_observer && expected_generation.is_none() {
        return Err("expected_generation_required".into());
    }
    // A target key is read as every command reads one
    // (`coordination::resolve_key`): an exact registration id wins.
    let registered = match crate::coordination::resolve_key(ledger, Some(parent), id, id) {
        Ok(Some(crate::coordination::Named::Agent(record))) => Some(record.actor.clone()),
        _ => None,
    };
    let mut matches = ledger.watches.iter().enumerate().filter(|(_, watch)| {
        (watch.parent.same_identity(parent) || approved_observer)
            && (watch.id == id
                || registered
                    .as_ref()
                    .is_some_and(|target| watch.target.same_identity(target)))
    });
    let index = matches
        .next()
        .map(|(index, _)| index)
        .ok_or("watch_unavailable")?;
    if matches.next().is_some() {
        return Err("watch_ambiguous".into());
    }
    let watch = &mut ledger.watches[index];
    if expected_generation.is_some_and(|expected| expected != watch.generation) {
        return Err("stale_generation".into());
    }
    if !watch.parent.same_identity(observer) {
        watch.generation = watch.generation.checked_add(1).ok_or("capacity")?;
        watch.parent = observer.clone();
    }
    Ok(watch.clone())
}

/// Approval permits only the attested new observer to accept a handover.
/// The explicit text is bounded input and is never emitted to diagnostics.
pub fn valid_approval(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

#[derive(Clone)]
pub struct Reading {
    pub id: String,
    pub status: String,
    pub status_available: bool,
    pub state_change_seq: Option<u64>,
    /// The engine connection the status was read on ([`StateRead::engine`]).
    pub engine: Option<String>,
    pub status_changed_at_unix_ms: u64,
    pub session_modified_at_unix_ms: Option<u64>,
    pub failure: Option<String>,
    pub gone: bool,
    /// A caller-selected window that replaces [`inactivity_window`].
    pub inactivity_ms: Option<u64>,
    /// The target's own row says it stopped on a question, neither working
    /// nor held by an engine-blocked prompt.
    pub asking: bool,
}

impl Reading {
    pub(crate) fn read(&self) -> StateRead<'_> {
        StateRead {
            engine: self.engine.as_deref(),
            status: &self.status,
            seq: self.state_change_seq,
        }
    }
}

/// A target whose activity read failed this many ticks in a row is reported.
/// The stored count stops here: past it a read that keeps failing changes
/// nothing the ledger holds, so the store does not rewrite the file for it.
pub const READ_FAILURES_REPORTED: u32 = 3;

#[derive(Default)]
pub struct Tick {
    pub transitions: bool,
    pub failures: Vec<(String, u32)>,
    pub rejections: Vec<(String, String)>,
}

/// A failed metadata read uses only this tick's status evidence. The durable
/// highest accepted activity identifies the episode, never a fallback read.
/// Each warning and its ordinal are admitted together; a rejected warning
/// cannot undo another target's exit or activity reset.
pub fn tick(ledger: &mut Ledger, readings: &[Reading], now: u64) -> Result<Tick, String> {
    let mut result = Tick::default();
    // The file's size is encoded in full once, when a row first changes, and
    // followed from there by bounded row encodings; a tick that changes
    // nothing encodes nothing.
    let mut size = None::<usize>;
    for reading in readings.iter().filter(|reading| reading.gone) {
        if let Some(index) = ledger
            .watches
            .iter()
            .position(|watch| watch.id == reading.id)
        {
            if let Some(size) = &mut size {
                *size -=
                    encoded_len(&ledger.watches[index])? + usize::from(ledger.watches.len() > 1);
            }
            ledger.watches.remove(index);
            result.transitions = true;
        }
    }
    let mut admitted = Vec::with_capacity(readings.len());
    for reading in readings.iter().filter(|reading| !reading.gone) {
        let Some(index) = ledger
            .watches
            .iter()
            .position(|watch| watch.id == reading.id)
        else {
            continue;
        };
        let before = ledger.watches[index].clone();
        let watch = &mut ledger.watches[index];
        let read = reading.read();
        // The target's last settled reading is kept whole, engine, status and
        // sequence: a reconnect's restore steps leave it as it was, and only
        // a state that moved from it is activity.
        if reading.status_available && read.settled() {
            if read.moved_from(&watch.last_read()) {
                watch.status_changed_at_unix_ms = reading.status_changed_at_unix_ms.min(now);
            }
            watch.last_status = reading.status.clone();
            watch.last_state_change_seq = reading.state_change_seq;
            watch.last_engine = reading.engine.clone();
        }
        let activity = if reading.status_available {
            reading
                .session_modified_at_unix_ms
                .unwrap_or(0)
                .max(watch.status_changed_at_unix_ms)
                .min(now)
        } else {
            watch.last_activity_at_unix_ms
        };
        watch.activity_failures = if reading.failure.is_some() {
            watch
                .activity_failures
                .saturating_add(1)
                .min(READ_FAILURES_REPORTED)
        } else {
            0
        };
        if activity > watch.last_activity_at_unix_ms {
            watch.last_activity_at_unix_ms = activity;
            watch.first_warning_at_unix_ms = None;
            watch.warning_count = 0;
            watch.operator_notified = false;
        }
        let (valid, changed) = (watch.valid(), *watch != before);
        let updated_size = if !valid {
            Err("ledger_unavailable".into())
        } else if !changed {
            Ok(size)
        } else if let Some(size) = size {
            resized(
                size,
                encoded_len(&before)?,
                encoded_len(&ledger.watches[index])?,
            )
            .map(Some)
        } else {
            full_size(ledger).map(Some)
        };
        match updated_size {
            Ok(updated_size) => size = updated_size,
            Err(code) => {
                ledger.watches[index] = before;
                result.rejections.push((reading.id.clone(), code));
                continue;
            }
        }
        let watch = &ledger.watches[index];
        if watch.activity_failures >= READ_FAILURES_REPORTED {
            result
                .failures
                .push((watch.id.clone(), watch.activity_failures));
        }
        admitted.push(reading);
    }
    for reading in admitted {
        let Some(index) = ledger
            .watches
            .iter()
            .position(|watch| watch.id == reading.id)
        else {
            continue;
        };
        let watch = &ledger.watches[index];
        let activity = if reading.status_available {
            reading
                .session_modified_at_unix_ms
                .unwrap_or(0)
                .max(watch.status_changed_at_unix_ms)
                .min(now)
        } else {
            watch.last_activity_at_unix_ms
        };
        let early = watch.warning_count == 0 && asks_early(ledger, watch, reading);
        let due = match (watch.warning_count, watch.first_warning_at_unix_ms) {
            (0, _) if early => now.saturating_sub(activity) >= QUESTION_WINDOW_MS,
            (0, _) => {
                now.saturating_sub(activity)
                    >= reading
                        .inactivity_ms
                        .unwrap_or_else(|| inactivity_window(watch))
            }
            (1, Some(first)) => now.saturating_sub(first) >= SECOND_WARNING_MS,
            _ => false,
        };
        if !due {
            continue;
        }
        // The warning is the observer's status cell about its target
        // (`runtime::status_cells`), read from these counts; no letter is
        // written (PRD letters-missions-simplify D-16).
        let before = watch.clone();
        let count = watch.warning_count + 1;
        let watch = &mut ledger.watches[index];
        watch.first_warning_at_unix_ms.get_or_insert(now);
        watch.warning_count = count;
        let resized_now = match size {
            Some(size) => resized(size, encoded_len(&before)?, encoded_len(watch)?),
            None => full_size(ledger),
        };
        match resized_now {
            Ok(updated_size) => {
                size = Some(updated_size);
                result.transitions = true;
            }
            Err(code) => {
                ledger.watches[index] = before;
                result.rejections.push((reading.id.clone(), code));
            }
        }
    }
    Ok(result)
}

/// Whether a pane observer hears after [`QUESTION_WINDOW_MS`] instead of the
/// full window: the target stopped on a question asked only in its own pane.
/// Code-owned observers use their full window, and a question the target
/// sent the observer by letter, still awaiting its answer, already rang the
/// observer's doorbell. A blocked prompt is not the watch's: its parent is
/// told at once (`runtime/blocked_children.rs`).
fn asks_early(ledger: &Ledger, watch: &Watch, reading: &Reading) -> bool {
    if watch.parent.code_owned() || !reading.asking {
        return false;
    }
    !ledger.letters.iter().any(|letter| {
        letter.waiting_answer
            && letter.sender.same_identity(&watch.target)
            && letter.recipient.same_identity(&watch.parent)
    })
}

/// The size of the whole file `ledger` would be, within its limit.
fn full_size(ledger: &Ledger) -> Result<usize, String> {
    ledger.bytes().map(|bytes| bytes.len())
}

fn resized(size: usize, removed: usize, added: usize) -> Result<usize, String> {
    size.checked_sub(removed)
        .and_then(|size| size.checked_add(added))
        .filter(|size| *size <= FILE_LIMIT)
        .ok_or_else(|| "capacity".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn actor(id: &str) -> Actor {
        Actor {
            pane_id: id.into(),
            name: id.into(),
            kind: "codex".into(),
            device_id: "local".into(),
            session: Some(format!("session-{id}")),
        }
    }
    fn reading(id: &str, activity: u64) -> Reading {
        Reading {
            id: id.into(),
            status: "idle".into(),
            status_available: true,
            state_change_seq: Some(activity),
            engine: None,
            status_changed_at_unix_ms: activity,
            session_modified_at_unix_ms: None,
            failure: None,
            gone: false,
            inactivity_ms: None,
            asking: false,
        }
    }

    #[test]
    fn reassigning_a_watch_preserves_activity_and_rejects_stale_or_wrong_observers() {
        let mut ledger = Ledger::default();
        let parent = actor("parent");
        let next = actor("next");
        let target = actor("target");
        let watch = start(&mut ledger, &parent, &target, 10).unwrap();
        tick(&mut ledger, &[reading(&watch.id, 10)], 10 + INACTIVITY_MS).unwrap();
        let before = ledger.watches[0].clone();
        let changed = assign(&mut ledger, &parent, &watch.id, &next, Some(0), None).unwrap();
        assert_eq!(changed.generation, 1);
        assert_eq!(
            changed.last_activity_at_unix_ms,
            before.last_activity_at_unix_ms
        );
        assert_eq!(
            changed.first_warning_at_unix_ms,
            before.first_warning_at_unix_ms
        );
        assert_eq!(changed.warning_count, before.warning_count);
        assert_eq!(
            assign(&mut ledger, &next, &watch.id, &parent, Some(0), None).unwrap_err(),
            "stale_generation"
        );
        assert_eq!(
            assign(&mut ledger, &parent, &watch.id, &parent, Some(1), None).unwrap_err(),
            "watch_unavailable"
        );
        let restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        assert_eq!(restored.watches[0], changed);
    }

    #[test]
    fn the_current_new_observer_accepts_handover_only_with_approval_and_current_generation() {
        let mut ledger = Ledger::default();
        let parent = actor("parent");
        let next = actor("next");
        let stranger = actor("stranger");
        let watch = start(&mut ledger, &parent, &actor("target"), 10).unwrap();
        tick(&mut ledger, &[reading(&watch.id, 10)], 10 + INACTIVITY_MS).unwrap();
        let before = ledger.watches[0].clone();
        for (caller, observer, generation, approval, expected) in [
            (&next, &next, Some(0), None, "watch_unavailable"),
            (
                &next,
                &next,
                None,
                Some("approved"),
                "expected_generation_required",
            ),
            (&next, &next, Some(1), Some("approved"), "stale_generation"),
            (
                &stranger,
                &next,
                Some(0),
                Some("approved"),
                "watch_unavailable",
            ),
            (&next, &next, Some(0), Some(" "), "invalid_approval"),
        ] {
            assert_eq!(
                assign(
                    &mut ledger,
                    caller,
                    &watch.id,
                    observer,
                    generation,
                    approval
                )
                .unwrap_err(),
                expected
            );
            assert_eq!(ledger.watches[0], before);
        }
        let changed = assign(
            &mut ledger,
            &next,
            &watch.id,
            &next,
            Some(0),
            Some("approved"),
        )
        .unwrap();
        let mut expected = before;
        expected.parent = next;
        expected.generation = 1;
        assert_eq!(changed, expected);
        assert_eq!(ledger.watches[0], expected);
        assert!(
            !String::from_utf8(ledger.bytes().unwrap())
                .unwrap()
                .contains("approved")
        );
    }

    #[test]
    fn missing_target_reference_keeps_status_only_watch_without_mailbox_authority() {
        let mut ledger = Ledger::default();
        let parent = actor("parent");
        let mut target = actor("target");
        target.session = None;
        let watch = start(&mut ledger, &parent, &target, 1).unwrap();
        let mut sample = reading(&watch.id, 1);
        sample.failure = Some("session_reference_missing".into());
        let outcome = tick(&mut ledger, &[sample], 1 + INACTIVITY_MS).unwrap();
        assert!(outcome.rejections.is_empty());
        assert_eq!(ledger.watches[0].warning_count, 1);
        assert_eq!(ledger.watches[0].target.session, None);
        assert!(
            ledger.letters.is_empty(),
            "a warning is the parent's status cell, not a letter"
        );
        assert_eq!(
            super::super::mailbox::pull(&ledger, &target).unwrap_err(),
            "native_identity_required"
        );
        let before = ledger.clone();
        assert_eq!(
            start(&mut ledger, &target, &parent, 1).unwrap_err(),
            "native_identity_required"
        );
        assert_eq!(ledger, before);
    }

    /// The first line of each watch's warning standing now, as its
    /// observer's status cell shows it (`runtime::status_cells`).
    fn summaries(ledger: &Ledger) -> Vec<String> {
        ledger
            .watches
            .iter()
            .filter(|watch| watch.warning_count > 0)
            .filter_map(|watch| {
                let first = watch.first_warning_at_unix_ms?;
                let warned = first + u64::from(watch.warning_count - 1) * SECOND_WARNING_MS;
                warning_line(watch, warned)
                    .lines()
                    .next()
                    .map(str::to_owned)
            })
            .collect()
    }

    /// The target as the engine connection `engine` reads it, `status` at
    /// state `seq`, with the observation's clock of that state at `changed`.
    fn read_on(id: &str, engine: &str, status: &str, seq: u64, changed: u64) -> Reading {
        Reading {
            status: status.into(),
            state_change_seq: Some(seq),
            engine: Some(engine.into()),
            ..reading(id, changed)
        }
    }

    /// A device's link coming back at `at`: its new engine restores the
    /// target's pane in steps, sequence 0, then unknown until its holder has
    /// read the screen, then `status` at a sequence of its own, each one new
    /// to the observation, so its clock reads the reconnect.
    fn reconnect(ledger: &mut Ledger, id: &str, link: &str, status: &str, at: u64) {
        for reading in [
            read_on(id, link, status, 0, at),
            read_on(id, link, "unknown", 1, at),
            read_on(id, link, status, 2, at),
        ] {
            tick(ledger, &[reading], at).unwrap();
        }
    }

    /// A target that rested through any number of its device's reconnects is
    /// one quiet episode: its parent gets the first warning at its hour from
    /// the target's last move, the second at its own, and the human notice
    /// stays owed; none of them starts again on a reconnect.
    #[test]
    fn a_resting_target_whose_device_reconnects_warns_its_parent_once_per_quiet() {
        let mut ledger = Ledger::default();
        let id = start(&mut ledger, &actor("parent"), &actor("child"), 10)
            .unwrap()
            .id;
        tick(&mut ledger, &[read_on(&id, "link-1", "done", 57, 10)], 10).unwrap();
        reconnect(&mut ledger, &id, "link-2", "done", 5 * 60_000);
        reconnect(&mut ledger, &id, "link-3", "done", 12 * 60_000);
        let first = 10 + INACTIVITY_MS;
        tick(
            &mut ledger,
            &[read_on(&id, "link-3", "done", 2, 12 * 60_000)],
            first - 1,
        )
        .unwrap();
        assert!(
            summaries(&ledger).is_empty(),
            "no reconnect moved the quiet"
        );
        tick(
            &mut ledger,
            &[read_on(&id, "link-3", "done", 2, 12 * 60_000)],
            first,
        )
        .unwrap();
        assert_eq!(summaries(&ledger).len(), 1);

        reconnect(&mut ledger, &id, "link-4", "done", first + 60_000);
        let watch = &ledger.watches[0];
        assert_eq!(
            (
                watch.warning_count,
                watch.first_warning_at_unix_ms,
                watch.last_activity_at_unix_ms
            ),
            (1, Some(first), 10),
            "the episode and its first warning stand"
        );
        let second = first + SECOND_WARNING_MS;
        tick(
            &mut ledger,
            &[read_on(&id, "link-4", "done", 2, first + 60_000)],
            second,
        )
        .unwrap();
        let watch = &ledger.watches[0];
        assert_eq!(
            (watch.warning_count, watch.last_activity_at_unix_ms),
            (2, 10),
            "two warnings of one episode"
        );
        assert_eq!(
            super::super::human::claim(&mut ledger, second).len(),
            1,
            "the unanswered first warning still owes the operator its notice"
        );
    }

    /// A target that finished while its device was away is seen finished on
    /// the next link: that change is activity once, and the quiet after it
    /// is warned once, at its hour, however often the link comes back.
    #[test]
    fn a_target_seen_finished_after_its_device_reconnects_warns_its_parent_once() {
        let mut ledger = Ledger::default();
        let id = start(&mut ledger, &actor("parent"), &actor("child"), 10)
            .unwrap()
            .id;
        tick(
            &mut ledger,
            &[read_on(&id, "link-1", "working", 57, 10)],
            10,
        )
        .unwrap();
        // Back within the hour: a longer outage is warned while it lasts, by
        // the last state read, a letter of its own.
        let back = 15 * 60_000;
        reconnect(&mut ledger, &id, "link-2", "done", back);
        assert_eq!(ledger.watches[0].last_activity_at_unix_ms, back);
        reconnect(&mut ledger, &id, "link-3", "done", back + 10 * 60_000);
        let hour = back + INACTIVITY_MS;
        let last = read_on(&id, "link-3", "done", 2, back + 10 * 60_000);
        tick(&mut ledger, std::slice::from_ref(&last), hour - 1).unwrap();
        assert!(summaries(&ledger).is_empty());
        tick(&mut ledger, std::slice::from_ref(&last), hour).unwrap();
        reconnect(&mut ledger, &id, "link-4", "done", hour + 60_000);
        tick(&mut ledger, std::slice::from_ref(&last), hour + 2 * 60_000).unwrap();
        assert_eq!(summaries(&ledger).len(), 1);
        assert_eq!(ledger.watches[0].last_activity_at_unix_ms, back);
    }

    /// A sequence compares only with one read on the same engine: the same
    /// status at another sequence is activity on the engine that moved it
    /// (a whole turn between two ticks), and nothing on the next link's
    /// engine, which counts from the start again. A watch loaded from its
    /// file names no engine, and its first reading compares by status.
    #[test]
    fn a_sequence_moves_a_watch_only_on_the_engine_that_counted_it() {
        let mut ledger = Ledger::default();
        let id = start(&mut ledger, &actor("parent"), &actor("child"), 10)
            .unwrap()
            .id;
        tick(&mut ledger, &[read_on(&id, "link-1", "done", 57, 10)], 10).unwrap();
        tick(&mut ledger, &[read_on(&id, "link-1", "done", 60, 50)], 60).unwrap();
        assert_eq!(ledger.watches[0].last_activity_at_unix_ms, 50);
        tick(&mut ledger, &[read_on(&id, "link-2", "done", 3, 70)], 80).unwrap();
        assert_eq!(ledger.watches[0].last_activity_at_unix_ms, 50);

        let mut loaded: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        assert_eq!(loaded.watches[0].last_engine, None);
        tick(&mut loaded, &[read_on(&id, "core-2", "done", 4, 90)], 100).unwrap();
        assert_eq!(loaded.watches[0].last_activity_at_unix_ms, 50);
        tick(
            &mut loaded,
            &[read_on(&id, "core-2", "working", 5, 110)],
            120,
        )
        .unwrap();
        assert_eq!(loaded.watches[0].last_activity_at_unix_ms, 110);
    }

    /// docs/delivery.md, Watch activity and completion: a child that
    /// stopped on a question asked only in its pane is told to its parent
    /// one tick later, once per episode, and the second warning keeps its
    /// hour; new activity starts a new episode.
    #[test]
    fn a_target_stopped_on_a_question_warns_its_parent_after_one_tick_once_per_episode() {
        let mut ledger = Ledger::default();
        let watch = start(&mut ledger, &actor("parent"), &actor("child"), 10).unwrap();
        let asking = |activity: u64| Reading {
            asking: true,
            ..reading(&watch.id, activity)
        };
        tick(&mut ledger, &[asking(10)], 10 + QUESTION_WINDOW_MS - 1).unwrap();
        assert!(summaries(&ledger).is_empty());
        tick(&mut ledger, &[asking(10)], 10 + QUESTION_WINDOW_MS).unwrap();
        let first = format!("Stopped with a question (watch {}).", watch.id);
        assert_eq!(summaries(&ledger), std::slice::from_ref(&first));
        let second = 10 + QUESTION_WINDOW_MS + SECOND_WARNING_MS;
        tick(&mut ledger, &[asking(10)], second - 1).unwrap();
        assert_eq!(summaries(&ledger), std::slice::from_ref(&first));
        tick(&mut ledger, &[asking(10)], second).unwrap();
        assert_eq!(
            summaries(&ledger),
            [format!("No activity for 61 minutes (watch {}).", watch.id)],
            "the second warning updates the one cell"
        );
        let active = second + 1;
        tick(&mut ledger, &[asking(active)], active + QUESTION_WINDOW_MS).unwrap();
        assert_eq!(summaries(&ledger), [first], "a new episode asks again");
    }

    #[test]
    fn a_stopped_target_with_no_question_waits_the_full_twenty_minutes() {
        let mut ledger = Ledger::default();
        let watch = start(&mut ledger, &actor("parent"), &actor("child"), 10).unwrap();
        tick(
            &mut ledger,
            &[reading(&watch.id, 10)],
            10 + QUESTION_WINDOW_MS,
        )
        .unwrap();
        tick(
            &mut ledger,
            &[reading(&watch.id, 10)],
            10 + INACTIVITY_MS - 1,
        )
        .unwrap();
        assert!(summaries(&ledger).is_empty());
        tick(&mut ledger, &[reading(&watch.id, 10)], 10 + INACTIVITY_MS).unwrap();
        assert_eq!(
            summaries(&ledger),
            [format!("No activity for 20 minutes (watch {}).", watch.id)]
        );
    }

    /// A question sent by letter already rang the parent's doorbell.
    #[test]
    fn a_question_sent_by_letter_waits_its_full_window() {
        let mut ledger = Ledger::default();
        let (parent, child) = (actor("parent"), actor("child"));
        let watch = start(&mut ledger, &parent, &child, 10).unwrap();
        let asked = super::super::mailbox::send(
            &mut ledger,
            &child,
            &parent,
            "which-branch",
            "Which branch?",
            "request",
            None,
            10,
        )
        .unwrap();
        let asking = Reading {
            asking: true,
            ..reading(&watch.id, 10)
        };
        tick(
            &mut ledger,
            std::slice::from_ref(&asking),
            10 + QUESTION_WINDOW_MS,
        )
        .unwrap();
        assert!(summaries(&ledger).is_empty());
        super::super::mailbox::apply(
            &mut ledger,
            &child,
            None,
            &super::super::Command::Cancel { id: asked.id },
            20,
        )
        .unwrap();
        tick(&mut ledger, &[asking], 10 + QUESTION_WINDOW_MS).unwrap();
        assert_eq!(
            summaries(&ledger).len(),
            1,
            "a cancelled letter asks nothing"
        );
    }

    #[test]
    fn two_warnings_keep_the_first_deadline_across_restart_and_reset_on_activity() {
        let mut ledger = Ledger::default();
        let watch = start(&mut ledger, &actor("parent"), &actor("target"), 10).unwrap();
        tick(
            &mut ledger,
            &[reading(&watch.id, 10)],
            10 + INACTIVITY_MS - 1,
        )
        .unwrap();
        assert_eq!(ledger.watches[0].warning_count, 0);
        tick(&mut ledger, &[reading(&watch.id, 10)], 10 + INACTIVITY_MS).unwrap();
        assert_eq!(ledger.watches[0].warning_count, 1);
        let mut ledger: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        tick(
            &mut ledger,
            &[reading(&watch.id, 10)],
            10 + INACTIVITY_MS + SECOND_WARNING_MS - 1,
        )
        .unwrap();
        assert_eq!(ledger.watches[0].warning_count, 1);
        tick(
            &mut ledger,
            &[reading(&watch.id, 10)],
            10 + INACTIVITY_MS + SECOND_WARNING_MS,
        )
        .unwrap();
        tick(
            &mut ledger,
            &[reading(&watch.id, 10)],
            10 + INACTIVITY_MS + SECOND_WARNING_MS * 2,
        )
        .unwrap();
        assert_eq!(ledger.watches[0].warning_count, 2, "two warnings at most");
        let active = 10 + INACTIVITY_MS + SECOND_WARNING_MS * 2 + 1;
        tick(&mut ledger, &[reading(&watch.id, active)], active).unwrap();
        assert_eq!(ledger.watches[0].warning_count, 0);
        assert_eq!(ledger.watches[0].first_warning_at_unix_ms, None);
        tick(
            &mut ledger,
            &[reading(&watch.id, active)],
            active + INACTIVITY_MS,
        )
        .unwrap();
        assert_eq!(ledger.watches[0].warning_count, 1);
        assert!(ledger.letters.is_empty(), "no warning is a letter");
    }

    #[test]
    fn bootstrap_of_unchanged_status_keeps_the_durable_activity_clock() {
        let mut ledger = Ledger::default();
        let watch = start(&mut ledger, &actor("parent"), &actor("target"), 10).unwrap();
        tick(&mut ledger, &[reading(&watch.id, 10)], 10 + INACTIVITY_MS).unwrap();
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        let mut bootstrap = reading(&watch.id, 10);
        bootstrap.status_changed_at_unix_ms = 10 + INACTIVITY_MS + SECOND_WARNING_MS;
        tick(
            &mut restored,
            &[bootstrap],
            10 + INACTIVITY_MS + SECOND_WARNING_MS,
        )
        .unwrap();
        assert_eq!(restored.watches[0].warning_count, 2);
        assert_eq!(
            restored.watches[0].first_warning_at_unix_ms,
            Some(10 + INACTIVITY_MS)
        );
    }

    #[test]
    fn reply_does_not_stop_watch_and_failed_reads_do_not_suppress_it() {
        let mut ledger = Ledger::default();
        let parent = actor("parent");
        let target = actor("target");
        let watch = start(&mut ledger, &parent, &target, 1).unwrap();
        let request = super::super::mailbox::send(
            &mut ledger,
            &parent,
            &target,
            "request",
            "work",
            "request",
            None,
            1,
        )
        .unwrap();
        super::super::mailbox::apply(
            &mut ledger,
            &target,
            None,
            &super::super::Command::Reply {
                id: request.id,
                intent: "reply".into(),
                body: "done".into(),
                now: false,
            },
            2,
        )
        .unwrap();
        assert_eq!(ledger.watches.len(), 1);
        let mut reading = reading(&watch.id, 1);
        reading.failure = Some("session_activity_unavailable".into());
        tick(&mut ledger, &[reading], 1 + INACTIVITY_MS).unwrap();
        assert_eq!(
            ledger.watches[0].warning_count, 1,
            "a failed read does not hold the warning back"
        );
        let mut reading = Reading {
            id: watch.id.clone(),
            status: "unknown".into(),
            status_available: true,
            state_change_seq: None,
            engine: None,
            status_changed_at_unix_ms: 1,
            session_modified_at_unix_ms: None,
            failure: None,
            gone: false,
            inactivity_ms: None,
            asking: false,
        };
        reading.gone = true;
        tick(&mut ledger, &[reading], 2 + INACTIVITY_MS).unwrap();
        assert!(ledger.watches.is_empty());
    }

    fn closed_letter(
        ledger: &mut Ledger,
        body: String,
        open: bool,
        now: u64,
    ) -> super::super::ledger::Letter {
        let sequence = ledger.next_id;
        ledger.next_id += 1;
        super::super::ledger::Letter {
            id: format!("letter-{sequence}"),
            intent: format!("fill-{sequence}"),
            sender: actor("filler"),
            recipient: actor("filler"),
            kind: "request".into(),
            body,
            written: None,
            state: if open {
                super::super::ledger::State::Delivered
            } else {
                super::super::ledger::State::Acknowledged
            },
            hook_confirmed: Some(true),
            waiting_answer: open,
            reply_to: None,
            created_at_unix_ms: now,
            finished_at_unix_ms: (!open).then_some(now),
            bell_errors: 0,
            bell_sent: false,
            bell_attempts: Some(0),
            human_notified: false,
            watch_warning: None,
            answer_wait_ended: None,
            bell_line: None,
            bell_turn: None,
            handed: false,
            handed_at_unix_ms: None,
            from: None,
            to: None,
            immediate: false,
            forced: false,
            tool_handovers: 0,
            bell_typed_at_unix_ms: None,
            bell_route: Default::default(),
            input_noticed: 0,
            input_noticed_at_unix_ms: 0,
            bell_failure: None,
            bell_ended: Some(0),
            core_id: None,
        }
    }

    fn fill_file_cap(ledger: &mut Ledger, now: u64) {
        let limit = 16 * 1024 * 1024 - 1;
        let mut size = ledger.bytes().unwrap().len();
        while size < limit {
            let sequence = ledger.next_id;
            let mut letter = closed_letter(ledger, "x".into(), false, now);
            let overhead = encoded_len(&letter).unwrap()
                + usize::from(!ledger.letters.is_empty())
                + ledger.next_id.to_string().len()
                - sequence.to_string().len();
            let mut room = limit - size;
            if room < overhead {
                let removed = overhead - room;
                let last = ledger.letters.last_mut().unwrap();
                last.body.truncate(last.body.len() - removed);
                size -= removed;
                room += removed;
            }
            letter.body = "x".repeat(1 + (room - overhead).min(16 * 1024 - 1));
            size += overhead + letter.body.len() - 1;
            ledger.letters.push(letter);
        }
        assert_eq!(ledger.bytes().unwrap().len(), limit);
    }

    #[test]
    fn warning_capacity_preserves_other_target_exit_and_activity_reset() {
        for cap in ["open", "file"] {
            for reverse in [false, true] {
                let mut ledger = Ledger::default();
                let parent = actor("parent");
                let gone = start(&mut ledger, &parent, &actor("gone"), 1).unwrap();
                let reset = start(&mut ledger, &parent, &actor("resumed"), 1).unwrap();
                let due = start(&mut ledger, &parent, &actor("inactive"), 1).unwrap();
                for watch in &mut ledger.watches {
                    watch.last_status = "idle".into();
                    watch.last_state_change_seq = Some(1);
                }
                ledger.watches[1].warning_count = 1;
                ledger.watches[1].first_warning_at_unix_ms = Some(10);
                let now = 1 + INACTIVITY_MS;
                match cap {
                    "open" => {
                        for _ in 0..1024 {
                            let letter = closed_letter(&mut ledger, "retained".into(), true, now);
                            ledger.letters.push(letter);
                        }
                    }
                    "file" => fill_file_cap(&mut ledger, now),
                    _ => unreachable!(),
                }
                let original_letters = ledger.letters.clone();
                let original_sequence = ledger.next_id;
                let mut exit = reading(&gone.id, 1);
                exit.gone = true;
                let mut resumed = reading(&reset.id, 1);
                resumed.session_modified_at_unix_ms = Some(now);
                let mut readings = vec![exit, resumed, reading(&due.id, 1)];
                if reverse {
                    readings.reverse();
                }
                let outcome = tick(&mut ledger, &readings, now).unwrap();
                assert!(outcome.transitions);
                // A warning writes no letter, so neither cap refuses it.
                assert!(outcome.rejections.is_empty(), "{cap}");
                assert!(!ledger.watches.iter().any(|watch| watch.id == gone.id));
                let reset = ledger
                    .watches
                    .iter()
                    .find(|watch| watch.id == reset.id)
                    .unwrap();
                assert_eq!(reset.warning_count, 0);
                assert_eq!(reset.first_warning_at_unix_ms, None);
                assert_eq!(reset.last_activity_at_unix_ms, now);
                let due = ledger
                    .watches
                    .iter()
                    .find(|watch| watch.id == due.id)
                    .unwrap();
                assert_eq!(due.warning_count, 1, "{cap}");
                assert!(due.first_warning_at_unix_ms.is_some(), "{cap}");
                assert_eq!(ledger.letters, original_letters);
                assert_eq!(ledger.next_id, original_sequence);
                ledger.bytes().unwrap();
            }
        }
    }

    /// A watch due for its first warning on a ledger already at the total
    /// cap, with the target's own reading.
    fn due_watch_at_the_total_cap(now: u64) -> (Ledger, Reading) {
        let mut ledger = Ledger::default();
        let due = start(&mut ledger, &actor("parent"), &actor("inactive"), 1).unwrap();
        ledger.watches[0].last_status = "idle".into();
        ledger.watches[0].last_state_change_seq = Some(1);
        for _ in 0..super::super::LETTER_LIMIT {
            let letter = closed_letter(&mut ledger, "x".into(), false, now);
            ledger.letters.push(letter);
        }
        (ledger, reading(&due.id, 1))
    }

    #[test]
    fn a_warning_at_the_total_letter_cap_writes_no_letter_and_drops_none() {
        let now = 1 + INACTIVITY_MS;
        let (mut ledger, due) = due_watch_at_the_total_cap(now);
        let letters = ledger.letters.clone();
        let outcome = tick(&mut ledger, &[due], now).unwrap();
        assert!(outcome.rejections.is_empty());
        assert_eq!(ledger.letters, letters);
        assert_eq!(ledger.watches[0].warning_count, 1);
        ledger.bytes().unwrap();
    }

    #[test]
    fn a_warning_the_file_cap_refuses_at_the_total_cap_puts_the_dropped_letter_back() {
        let now = 1 + INACTIVITY_MS;
        let (mut ledger, due) = due_watch_at_the_total_cap(now);
        let limit = 16 * 1024 * 1024 - 1;
        let mut room = limit - ledger.bytes().unwrap().len();
        for letter in ledger.letters.iter_mut().skip(1) {
            let add = room.min(16 * 1024 - letter.body.len());
            letter.body.push_str(&"x".repeat(add));
            room -= add;
        }
        assert_eq!(room, 0);
        assert_eq!(ledger.bytes().unwrap().len(), limit);
        let original_letters = ledger.letters.clone();
        let original_sequence = ledger.next_id;
        let outcome = tick(&mut ledger, std::slice::from_ref(&due), now).unwrap();
        assert!(
            outcome
                .rejections
                .contains(&(due.id.clone(), "capacity".into()))
        );
        assert_eq!(ledger.letters, original_letters);
        assert_eq!(ledger.next_id, original_sequence);
    }

    #[test]
    fn file_only_activity_reset_and_failed_read_start_a_distinct_durable_warning_episode() {
        let mut ledger = Ledger::default();
        let parent = actor("parent");
        let watch = start(&mut ledger, &parent, &actor("target"), 1).unwrap();
        let first = 1 + INACTIVITY_MS;
        tick(&mut ledger, &[reading(&watch.id, 1)], first).unwrap();
        assert_eq!(ledger.watches[0].first_warning_at_unix_ms, Some(first));
        ledger.watches[0].operator_notified = true;
        let accepted_activity = first + 1;
        let mut resumed = reading(&watch.id, 1);
        resumed.session_modified_at_unix_ms = Some(accepted_activity);
        tick(&mut ledger, &[resumed], accepted_activity).unwrap();
        assert_eq!(ledger.watches[0].warning_count, 0);
        assert!(
            !ledger.watches[0].operator_notified,
            "a new episode owes again"
        );
        let mut failed = reading(&watch.id, 1);
        failed.failure = Some("session_activity_unavailable".into());
        let new_first = accepted_activity + INACTIVITY_MS;
        tick(&mut ledger, &[failed.clone()], new_first).unwrap();
        assert_eq!(ledger.watches[0].first_warning_at_unix_ms, Some(new_first));
        let mut restored: Ledger = serde_json::from_slice(&ledger.bytes().unwrap()).unwrap();
        tick(
            &mut restored,
            &[failed.clone()],
            new_first + SECOND_WARNING_MS - 1,
        )
        .unwrap();
        assert_eq!(restored.watches[0].warning_count, 1);
        tick(
            &mut restored,
            &[failed.clone()],
            new_first + SECOND_WARNING_MS,
        )
        .unwrap();
        tick(&mut restored, &[failed], new_first + SECOND_WARNING_MS * 2).unwrap();
        assert_eq!(
            restored.watches[0].first_warning_at_unix_ms,
            Some(new_first)
        );
        assert_eq!(restored.watches[0].warning_count, 2);
        assert!(restored.letters.is_empty());
    }
}
