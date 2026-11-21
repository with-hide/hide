//! The guest's durable record of a host's events (PRD D-54, D-66).
//!
//! A host numbers each event in its box and sends it; this record is how an
//! event is applied once however often it arrives. For each event the guest
//! appends it here and syncs ([`Inbox::record`]), acknowledges it to the
//! host, and only then applies it ([`Inbox::apply_recorded`]). A host that
//! heard no acknowledgement sends again from the place the guest names
//! ([`Inbox::position`]); what was already recorded is acknowledged again and
//! not applied twice. An event recorded but not applied when the guest
//! stopped is applied at its next start, and the product's own idempotency
//! keys (a letter's intent, a pane's id, a completion's number) make that
//! second apply the same as the first.
//!
//! There is no negotiation about what was sent or is ready: the guest's
//! position is the only state the two share.
//!
//! The file holds two runs of lines: events applied already, then events
//! recorded and not applied. Each run is kept to
//! [`limits::MAX_UNAPPLIED_BYTES`], so the file, and the read of it at the
//! next start, stay under twice that however long a host keeps one event
//! waiting behind the ones it lets through.
//!
//! The owner applies events through [`Apply`], which the daemon implements,
//! so this crate does not link the core. Nothing here runs under the core's
//! lock: recording is a durable write.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use hide_peer_protocol::event::Event;
use hide_platform::fs::{Access, atomic, private};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::limits;

const EVENTS_FILE: &str = "events.jsonl";
const CURSOR_FILE: &str = "cursor.json";

/// What applying an event came to. `Done` and `Unappliable` are final: the
/// event is not applied again.
pub enum Applied {
    Done,
    /// Nothing here can take the event (an unknown kind, a target that is
    /// gone). It was acknowledged; it is logged and left.
    Unappliable(String),
    /// What takes the event is not known yet (the host's panes are still
    /// being read again). The event stays recorded, and it and every one
    /// after it are applied at the next call.
    Later,
}

/// Applies one event to the product. It must be safe to call twice for one
/// event: the product's idempotency key decides.
pub trait Apply {
    fn apply(&mut self, event: &Event) -> Applied;
}

/// What became of an event handed to [`Inbox::record`].
#[derive(Debug, PartialEq, Eq)]
pub enum Recorded {
    /// Appended and synced: acknowledge `seq`, then apply.
    New { seq: u64 },
    /// Already recorded: acknowledge `seq` again and apply nothing.
    Again { seq: u64 },
    /// Not the next event of the box: the host sends again from after
    /// `have`. Nothing is acknowledged.
    Gap { have: u64 },
    /// More events, or more bytes of them, wait to be applied than the
    /// record holds. Nothing is acknowledged; the host keeps the event and
    /// sends it later.
    Full,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Cursor {
    /// The box the positions count in.
    #[serde(default)]
    outbox: String,
    /// The last event applied.
    #[serde(default)]
    applied: u64,
}

/// An event recorded and not applied, with the bytes its line takes in the
/// file.
struct Waiting {
    event: Event,
    bytes: u64,
}

pub struct Inbox {
    dir: PathBuf,
    cursor: Cursor,
    /// The last event appended and synced.
    recorded: u64,
    /// Recorded and not yet applied, oldest first.
    waiting: VecDeque<Waiting>,
    /// The bytes of the waiting events' lines: the end of `events`.
    waiting_bytes: u64,
    /// The most bytes each run of lines may take
    /// ([`limits::MAX_UNAPPLIED_BYTES`]).
    max_bytes: u64,
    events: File,
    /// The bytes of whole, synced lines in `events`.
    len: u64,
    /// An append failed partway and what it wrote could not be cut then.
    torn: bool,
}

impl Inbox {
    /// Opens the record in `dir`, one folder per host. Events a stop left
    /// recorded but not applied are in it again; apply them before reading
    /// the host's stream.
    pub fn open(dir: &Path) -> io::Result<Self> {
        Self::open_capped(dir, limits::MAX_UNAPPLIED_BYTES as u64)
    }

    fn open_capped(dir: &Path, max_bytes: u64) -> io::Result<Self> {
        private::create_dir_all(dir)?;
        let cursor: Cursor = match std::fs::read(dir.join(CURSOR_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("the inbox cursor in {} is damaged: {error}", dir.display()),
                )
            })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Cursor::default(),
            Err(error) => return Err(error),
        };
        let mut events = private::open_own_file(&dir.join(EVENTS_FILE), true)?;
        let mut text = String::new();
        events.read_to_string(&mut text)?;
        let mut waiting = VecDeque::new();
        let mut waiting_bytes = 0u64;
        let mut kept = 0usize;
        for line in text.split_inclusive('\n') {
            // A last line a crash cut short was never synced whole, so it
            // was never acknowledged: it is dropped and the host sends it
            // again.
            let Some(event) = line
                .strip_suffix('\n')
                .and_then(|line| serde_json::from_str::<Event>(line).ok())
            else {
                break;
            };
            kept += line.len();
            if event.outbox == cursor.outbox && event.seq > cursor.applied {
                let bytes = line.len() as u64;
                waiting_bytes += bytes;
                waiting.push_back(Waiting { event, bytes });
            }
        }
        if kept != text.len() {
            events.set_len(kept as u64)?;
            events.sync_all()?;
        }
        events.seek(SeekFrom::End(0))?;
        let recorded = waiting
            .back()
            .map_or(cursor.applied, |waiting| waiting.event.seq);
        Ok(Self {
            dir: dir.to_owned(),
            cursor,
            recorded,
            waiting,
            waiting_bytes,
            max_bytes,
            events,
            len: kept as u64,
            torn: false,
        })
    }

    /// Cuts the file back to its last whole line.
    fn cut_to_whole_lines(&mut self) -> io::Result<()> {
        self.events.set_len(self.len)?;
        self.events.seek(SeekFrom::Start(self.len))?;
        self.torn = false;
        Ok(())
    }

    /// Appends one line and syncs it. A write that fails partway leaves
    /// part of a line; it is cut off again, now or before the next append,
    /// so it never joins the line after it: a joined line reads as damage,
    /// and everything recorded behind it would be dropped at the next open.
    fn append(&mut self, line: &[u8]) -> io::Result<()> {
        if self.torn {
            self.cut_to_whole_lines()?;
        }
        let written = self
            .events
            .write_all(line)
            .and_then(|()| self.events.sync_all());
        if let Err(error) = written {
            self.torn = true;
            // Best effort here: the next append cuts again when this fails.
            let _ = self.cut_to_whole_lines();
            return Err(error);
        }
        self.len += line.len() as u64;
        Ok(())
    }

    /// The box and the last event durably recorded: what the guest tells
    /// the host when a connection starts, so the host sends what follows.
    pub fn position(&self) -> (&str, u64) {
        (&self.cursor.outbox, self.recorded)
    }

    /// How many recorded events are not applied yet.
    pub fn unapplied(&self) -> usize {
        self.waiting.len()
    }

    /// Appends `event` and syncs it, when it is the next of its box.
    pub fn record(&mut self, event: Event) -> io::Result<Recorded> {
        if event.outbox != self.cursor.outbox {
            // A box started fresh counts from 1. Anything else from a box
            // this record has not seen is answered with "have nothing".
            if event.seq != 1 {
                return Ok(Recorded::Gap { have: 0 });
            }
            if !self.waiting.is_empty() {
                return Err(io::Error::other(
                    "events of the earlier box are recorded and not applied yet",
                ));
            }
            self.save_cursor(Cursor {
                outbox: event.outbox.clone(),
                applied: 0,
            })?;
            self.recorded = 0;
        }
        if event.seq <= self.recorded {
            return Ok(Recorded::Again { seq: event.seq });
        }
        if event.seq != self.recorded + 1 {
            return Ok(Recorded::Gap {
                have: self.recorded,
            });
        }
        let mut line = serde_json::to_vec(&event).map_err(io::Error::other)?;
        line.push(b'\n');
        let bytes = line.len() as u64;
        // Every waiting event is in use: the host was told it is recorded
        // and no longer keeps it, so none can go to make room.
        if self.waiting.len() >= limits::MAX_UNAPPLIED
            || self.waiting_bytes + bytes > self.max_bytes
        {
            return Ok(Recorded::Full);
        }
        self.append(&line)?;
        self.recorded = event.seq;
        self.waiting_bytes += bytes;
        self.waiting.push_back(Waiting { event, bytes });
        Ok(Recorded::New { seq: self.recorded })
    }

    /// Applies every recorded event in order and returns how many. Call it
    /// after the acknowledgement went out, and once at start.
    pub fn apply_recorded(&mut self, apply: &mut dyn Apply) -> io::Result<usize> {
        let mut count = 0;
        while let Some(Waiting { event, bytes }) = self.waiting.front() {
            match apply.apply(event) {
                Applied::Done => {}
                Applied::Unappliable(why) => crate::diagnostic!(json!({
                    "component": "peer_inbox",
                    "event": "event.unappliable",
                    "outbox": event.outbox,
                    "seq": event.seq,
                    "kind": event.kind,
                    "reason": why,
                })),
                // Order holds: nothing after it is applied before it.
                Applied::Later => break,
            }
            let applied = event.seq;
            self.waiting_bytes -= bytes;
            self.waiting.pop_front();
            count += 1;
            // Saved after each apply: a stop here repeats at most the one
            // event that was being applied.
            self.save_cursor(Cursor {
                outbox: self.cursor.outbox.clone(),
                applied,
            })?;
        }
        if count > 0 {
            self.drop_applied()?;
        }
        Ok(count)
    }

    /// Drops the applied lines: all of them when nothing waits, and, while
    /// an event waits, once they take more room than the waiting ones may.
    /// The cursor covers them and nothing reads them again; a stop before
    /// this leaves them in the file, and `open` skips them.
    fn drop_applied(&mut self) -> io::Result<()> {
        if self.waiting.is_empty() {
            self.len = 0;
            self.cut_to_whole_lines()?;
            return self.events.sync_all();
        }
        if self.len - self.waiting_bytes <= self.max_bytes {
            return Ok(());
        }
        // Replaces the file with the waiting lines, written again from the
        // events themselves.
        let mut bytes = Vec::with_capacity(self.waiting_bytes as usize);
        let mut ends = Vec::with_capacity(self.waiting.len());
        for waiting in &self.waiting {
            serde_json::to_writer(&mut bytes, &waiting.event).map_err(io::Error::other)?;
            bytes.push(b'\n');
            ends.push(bytes.len() as u64);
        }
        let path = self.dir.join(EVENTS_FILE);
        atomic::write_file(&path, &bytes, Access::Private)?;
        self.events = private::open_own_file(&path, true)?;
        self.len = bytes.len() as u64;
        self.events.seek(SeekFrom::Start(self.len))?;
        self.torn = false;
        let mut start = 0;
        for (waiting, end) in self.waiting.iter_mut().zip(ends) {
            waiting.bytes = end - start;
            start = end;
        }
        self.waiting_bytes = self.len;
        Ok(())
    }

    fn save_cursor(&mut self, cursor: Cursor) -> io::Result<()> {
        let bytes = serde_json::to_vec(&cursor).map_err(io::Error::other)?;
        atomic::write_file(&self.dir.join(CURSOR_FILE), &bytes, Access::Private)?;
        self.cursor = cursor;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(outbox: &str, seq: u64) -> Event {
        Event {
            v: 1,
            outbox: outbox.into(),
            seq,
            kind: "letter".into(),
            key: format!("intent-{seq}"),
            at_unix_ms: 0,
            body: json!({ "seq": seq }),
        }
    }

    /// Applies by key, as the product does: a second apply of one key
    /// changes nothing.
    #[derive(Default)]
    struct ByKey {
        applied: Vec<String>,
        calls: usize,
    }

    impl Apply for ByKey {
        fn apply(&mut self, event: &Event) -> Applied {
            self.calls += 1;
            if event.kind != "letter" {
                return Applied::Unappliable("unknown kind".into());
            }
            if !self.applied.contains(&event.key) {
                self.applied.push(event.key.clone());
            }
            Applied::Done
        }
    }

    #[test]
    fn an_event_sent_twice_is_acknowledged_twice_and_applied_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let mut product = ByKey::default();
        assert_eq!(inbox.position(), ("", 0));

        assert_eq!(
            inbox.record(event("box", 1)).unwrap(),
            Recorded::New { seq: 1 }
        );
        assert_eq!(inbox.apply_recorded(&mut product).unwrap(), 1);
        // The acknowledgement was lost: the host sends it again.
        assert_eq!(
            inbox.record(event("box", 1)).unwrap(),
            Recorded::Again { seq: 1 }
        );
        assert_eq!(inbox.apply_recorded(&mut product).unwrap(), 0);
        // An event ahead of the next one names where to send from.
        assert_eq!(
            inbox.record(event("box", 3)).unwrap(),
            Recorded::Gap { have: 1 }
        );
        assert_eq!(product.applied, ["intent-1"]);
        assert_eq!(product.calls, 1);
        assert_eq!(inbox.position(), ("box", 1));
    }

    #[test]
    fn what_a_failed_append_left_is_cut_before_the_next_event() {
        let dir = tempfile::tempdir().unwrap();
        let mut product = ByKey::default();
        {
            let mut inbox = Inbox::open(dir.path()).unwrap();
            inbox.record(event("box", 1)).unwrap();
            // An append that failed partway and could not be cut then: part
            // of a line is in the file.
            inbox.events.write_all(b"{\"v\":1,\"outbox\":\"bo").unwrap();
            inbox.torn = true;
            assert_eq!(
                inbox.record(event("box", 2)).unwrap(),
                Recorded::New { seq: 2 }
            );
        }
        // Both are whole lines: a restart reads the two, not one line and
        // damage behind it.
        let mut inbox = Inbox::open(dir.path()).unwrap();
        assert_eq!(inbox.position(), ("box", 2));
        assert_eq!(inbox.apply_recorded(&mut product).unwrap(), 2);
        assert_eq!(product.applied, ["intent-1", "intent-2"]);
    }

    #[test]
    fn an_event_recorded_before_a_stop_is_applied_at_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let mut product = ByKey::default();
        {
            let mut inbox = Inbox::open(dir.path()).unwrap();
            inbox.record(event("box", 1)).unwrap();
            inbox.apply_recorded(&mut product).unwrap();
            // Recorded and acknowledged, and the guest stops before applying.
            assert_eq!(
                inbox.record(event("box", 2)).unwrap(),
                Recorded::New { seq: 2 }
            );
        }
        let mut inbox = Inbox::open(dir.path()).unwrap();
        assert_eq!(
            inbox.position(),
            ("box", 2),
            "the host is not asked for it again"
        );
        assert_eq!(inbox.apply_recorded(&mut product).unwrap(), 1);
        assert_eq!(product.applied, ["intent-1", "intent-2"]);
        assert_eq!(
            inbox.record(event("box", 2)).unwrap(),
            Recorded::Again { seq: 2 }
        );
    }

    #[test]
    fn a_line_a_crash_cut_short_is_dropped_and_asked_for_again() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut inbox = Inbox::open(dir.path()).unwrap();
            inbox.record(event("box", 1)).unwrap();
        }
        let path = dir.path().join(EVENTS_FILE);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(br#"{"v":1,"outbox":"box","seq":2,"ki"#);
        std::fs::write(&path, bytes).unwrap();

        let mut inbox = Inbox::open(dir.path()).unwrap();
        assert_eq!(
            inbox.position(),
            ("box", 1),
            "the cut line was never acknowledged"
        );
        assert_eq!(
            inbox.record(event("box", 2)).unwrap(),
            Recorded::New { seq: 2 }
        );
        let mut product = ByKey::default();
        assert_eq!(inbox.apply_recorded(&mut product).unwrap(), 2);
        assert_eq!(product.applied, ["intent-1", "intent-2"]);
    }

    #[test]
    fn an_event_nothing_can_take_is_acknowledged_and_left() {
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let mut product = ByKey::default();
        let mut strange = event("box", 1);
        strange.kind = "from_a_later_build".into();
        assert_eq!(inbox.record(strange).unwrap(), Recorded::New { seq: 1 });
        assert_eq!(
            inbox.record(event("box", 2)).unwrap(),
            Recorded::New { seq: 2 }
        );
        assert_eq!(inbox.apply_recorded(&mut product).unwrap(), 2);
        assert_eq!(
            product.applied,
            ["intent-2"],
            "the next event is not held up"
        );
        assert_eq!(Inbox::open(dir.path()).unwrap().position(), ("box", 2));
    }

    /// Takes every event before `waits_from`; that one and every one after
    /// it wait.
    struct Target {
        waits_from: u64,
        applied: Vec<u64>,
    }

    impl Apply for Target {
        fn apply(&mut self, event: &Event) -> Applied {
            if event.seq >= self.waits_from {
                return Applied::Later;
            }
            self.applied.push(event.seq);
            Applied::Done
        }
    }

    /// The bytes one event's line takes in the file.
    fn line_bytes(seq: u64) -> u64 {
        serde_json::to_vec(&event("box", seq)).unwrap().len() as u64 + 1
    }

    #[test]
    fn an_event_whose_target_is_not_there_yet_waits_with_every_one_after_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut target = Target {
            waits_from: 1,
            applied: Vec::new(),
        };
        {
            let mut inbox = Inbox::open(dir.path()).unwrap();
            inbox.record(event("box", 1)).unwrap();
            inbox.record(event("box", 2)).unwrap();
            assert_eq!(inbox.apply_recorded(&mut target).unwrap(), 0);
            assert_eq!(inbox.position(), ("box", 2), "both stay acknowledged");
        }
        // The next connection finds them recorded and applies them in order.
        let mut inbox = Inbox::open(dir.path()).unwrap();
        target.waits_from = u64::MAX;
        assert_eq!(inbox.apply_recorded(&mut target).unwrap(), 2);
        assert_eq!(target.applied, [1, 2]);
    }

    #[test]
    fn applied_events_are_dropped_from_the_file_while_one_still_waits() {
        // A host that always keeps its newest event waiting behind the
        // ones it lets through, across connections: the applied lines never
        // reach the point where nothing waits.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(EVENTS_FILE);
        let cap = 4 * line_bytes(60);
        let mut target = Target {
            waits_from: 0,
            applied: Vec::new(),
        };
        let mut inbox = Inbox::open_capped(dir.path(), cap).unwrap();
        for seq in 1..=60 {
            if seq % 7 == 0 {
                // The host connects again: the record is read from the file.
                inbox = Inbox::open_capped(dir.path(), cap).unwrap();
                assert_eq!(inbox.position(), ("box", seq - 1));
                assert_eq!(inbox.unapplied(), 1);
            }
            assert_eq!(
                inbox.record(event("box", seq)).unwrap(),
                Recorded::New { seq }
            );
            target.waits_from = seq;
            inbox.apply_recorded(&mut target).unwrap();
            assert_eq!(inbox.unapplied(), 1, "the newest event waits");
            let kept = std::fs::metadata(&path).unwrap().len();
            assert!(
                kept <= 2 * cap,
                "{kept} bytes are kept after event {seq}: over twice the cap of {cap}"
            );
        }
        // The event that waited is still recorded, and no event was applied
        // twice or out of order on the way.
        let mut inbox = Inbox::open_capped(dir.path(), cap).unwrap();
        assert_eq!(inbox.position(), ("box", 60));
        target.waits_from = u64::MAX;
        assert_eq!(inbox.apply_recorded(&mut target).unwrap(), 1);
        assert_eq!(target.applied, (1..=60).collect::<Vec<_>>());
        assert_eq!(
            inbox.record(event("box", 60)).unwrap(),
            Recorded::Again { seq: 60 }
        );
        assert_eq!(
            inbox.record(event("box", 61)).unwrap(),
            Recorded::New { seq: 61 }
        );
    }

    #[test]
    fn an_event_past_the_byte_cap_stays_with_the_host_until_some_are_applied() {
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open_capped(dir.path(), 3 * line_bytes(1)).unwrap();
        let mut target = Target {
            waits_from: 1,
            applied: Vec::new(),
        };
        for seq in 1..=3 {
            assert_eq!(
                inbox.record(event("box", seq)).unwrap(),
                Recorded::New { seq }
            );
        }
        assert_eq!(inbox.apply_recorded(&mut target).unwrap(), 0);
        // Three events wait, far fewer than the record may hold by count.
        assert_eq!(inbox.record(event("box", 4)).unwrap(), Recorded::Full);
        assert_eq!(
            inbox.position(),
            ("box", 3),
            "the host is told to send from after the third"
        );
        // What waited is applied, none of it dropped, and there is room.
        target.waits_from = u64::MAX;
        assert_eq!(inbox.apply_recorded(&mut target).unwrap(), 3);
        assert_eq!(
            inbox.record(event("box", 4)).unwrap(),
            Recorded::New { seq: 4 }
        );
        assert_eq!(inbox.apply_recorded(&mut target).unwrap(), 1);
        assert_eq!(target.applied, [1, 2, 3, 4]);
    }

    #[test]
    fn a_fresh_box_starts_the_count_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut inbox = Inbox::open(dir.path()).unwrap();
        let mut product = ByKey::default();
        inbox.record(event("old", 1)).unwrap();
        inbox.apply_recorded(&mut product).unwrap();
        assert_eq!(
            inbox.record(event("new", 4)).unwrap(),
            Recorded::Gap { have: 0 }
        );
        assert_eq!(
            inbox.record(event("new", 1)).unwrap(),
            Recorded::New { seq: 1 }
        );
        assert_eq!(inbox.position(), ("new", 1));
    }
}
