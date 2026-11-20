//! The box a guest's node keeps its events in while its core is away (PRD
//! device-connect-iroh D-54, D-66, B12, B13).
//!
//! An event is appended to `box.jsonl` and synced before whoever made it
//! is answered, so an answer of `queued` survives the node, its host and
//! the machine. The guest's core says how far it has durably recorded the
//! box ([`EventBox::resume`], [`EventBox::ack`]); what it recorded is
//! dropped here, and everything after it is sent again whenever it asks.
//! Sending twice is safe: the guest's own record and the product's keys
//! make the second one nothing.
//!
//! Two builds of one guest read this file, the node that wrote it and the
//! one that replaces it, so its lines are `hide_peer_protocol::event`'s.
//! A line is kept and sent as the text it was written as: a field a later
//! build added goes to the guest's core unchanged, and a line this build
//! cannot read is left where it is for a build that can.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use hide_peer::protocol::event::{
    ACKED_FILE, Acked, BOX_FILE, BOX_MAX_BYTES, BOX_MAX_EVENTS, BOX_VERSION, Event,
};
use hide_peer::protocol::frame;
use hide_peer::protocol::launch::Position;
use hide_platform::fs::{Access, atomic, private};
use serde::Deserialize;

use super::lock;

/// The room a relayed event leaves in one management message for what the
/// host adds around it.
const RELAY_ROOM: usize = 64;

/// Why an event was not kept.
#[derive(Debug)]
pub enum Refused {
    /// The box holds as many events, or as many bytes, as it may (B13).
    Full,
    /// The event is larger than one message between the machines.
    TooLarge,
    /// The box could not be written.
    Failed(io::Error),
}

/// What every line of the box says, whichever build wrote it.
#[derive(Deserialize)]
struct Head {
    outbox: String,
    seq: u64,
    kind: String,
    key: String,
}

struct Kept {
    seq: u64,
    kind: String,
    key: String,
    /// The line as it is in the file, without its end.
    line: String,
}

struct State {
    outbox: String,
    /// The guest's core recorded the box through this event.
    acked: u64,
    /// The last event numbered.
    last: u64,
    /// The events after `acked`, oldest first.
    kept: VecDeque<Kept>,
    /// The bytes `kept` takes in the file.
    bytes: usize,
    file: File,
    /// The bytes of whole, synced lines in `file`.
    len: u64,
    /// An append failed partway and what it wrote could not be cut then.
    torn: bool,
    /// The guest's core asked for the box and has not gone since: a new
    /// event is sent as it is kept.
    resumed: bool,
}

pub struct EventBox {
    dir: PathBuf,
    state: Mutex<State>,
    /// Writes the host one event, as its line. Called in the box's order.
    send: Box<dyn Fn(&str) + Send + Sync>,
}

fn damaged(dir: &Path, what: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("the event box in {}: {what}", dir.display()),
    )
}

fn fresh_outbox() -> io::Result<String> {
    let mut id = [0u8; 8];
    getrandom::getrandom(&mut id).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(hex::encode(id))
}

fn save_acked(dir: &Path, outbox: &str, seq: u64) -> io::Result<()> {
    let acked = Acked {
        v: BOX_VERSION,
        outbox: outbox.to_owned(),
        seq,
    };
    let bytes = serde_json::to_vec(&acked).map_err(io::Error::other)?;
    atomic::write_file(&dir.join(ACKED_FILE), &bytes, Access::Private).map(drop)
}

impl EventBox {
    /// Opens the box in `dir`, the node's own folder, with what an earlier
    /// node of this guest left in it.
    pub fn open(dir: &Path, send: Box<dyn Fn(&str) + Send + Sync>) -> io::Result<Self> {
        let acked: Option<Acked> = match std::fs::read(dir.join(ACKED_FILE)) {
            Ok(bytes) => Some(
                serde_json::from_slice(&bytes)
                    .map_err(|error| damaged(dir, format!("its place is unreadable: {error}")))?,
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let mut file = private::open_own_file(&dir.join(BOX_FILE), true)?;
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        let mut lines = Vec::new();
        let mut whole = 0usize;
        for line in text.split_inclusive('\n') {
            // A last line a crash cut short was never synced whole, so
            // whoever made it was never answered: it is dropped.
            let Some(line) = line.strip_suffix('\n') else {
                break;
            };
            // A whole line this build cannot read is another build's, and
            // stays: the box is refused rather than numbered around it.
            let head: Head = serde_json::from_str(line)
                .map_err(|error| damaged(dir, format!("a line this build cannot read: {error}")))?;
            whole += line.len() + 1;
            lines.push((
                head.outbox,
                Kept {
                    seq: head.seq,
                    kind: head.kind,
                    key: head.key,
                    line: line.to_owned(),
                },
            ));
        }
        if whole != text.len() {
            file.set_len(whole as u64)?;
            file.sync_all()?;
        }
        file.seek(SeekFrom::End(0))?;
        // The lines say which box this is: a box started fresh is written
        // before its place is, and a stop between the two leaves the place
        // of the box before it.
        let first = lines
            .first()
            .map(|(outbox, first)| (outbox.clone(), first.seq.saturating_sub(1)));
        let (outbox, acked_seq) = match (first, acked) {
            (Some((outbox, _)), Some(acked)) if acked.outbox == outbox => (outbox, acked.seq),
            (Some((outbox, before)), _) => {
                save_acked(dir, &outbox, before)?;
                (outbox, before)
            }
            (None, Some(acked)) => (acked.outbox, acked.seq),
            (None, None) => {
                let outbox = fresh_outbox()?;
                save_acked(dir, &outbox, 0)?;
                (outbox, 0)
            }
        };
        let mut kept = VecDeque::new();
        let mut last = acked_seq;
        let mut bytes = 0;
        for (line_outbox, line) in lines {
            if line_outbox != outbox {
                return Err(damaged(dir, "it holds the events of two boxes"));
            }
            if line.seq <= acked_seq {
                continue;
            }
            if line.seq != last + 1 {
                return Err(damaged(dir, "its events are not numbered in order"));
            }
            last = line.seq;
            bytes += line.line.len() + 1;
            kept.push_back(line);
        }
        Ok(Self {
            dir: dir.to_owned(),
            state: Mutex::new(State {
                outbox,
                acked: acked_seq,
                last,
                kept,
                bytes,
                file,
                len: whole as u64,
                torn: false,
                resumed: false,
            }),
            send,
        })
    }

    /// Keeps one event: appended and synced when this returns. An event of
    /// the same kind and key that is still kept is the same event, so a
    /// send tried again adds nothing.
    pub fn push(
        &self,
        kind: &str,
        key: &str,
        at_unix_ms: u64,
        body: serde_json::Value,
    ) -> Result<(), Refused> {
        let mut state = lock(&self.state);
        if state
            .kept
            .iter()
            .any(|kept| kept.kind == kind && kept.key == key)
        {
            return Ok(());
        }
        let seq = state.last + 1;
        let line = serde_json::to_string(&Event {
            v: BOX_VERSION,
            outbox: state.outbox.clone(),
            seq,
            kind: kind.to_owned(),
            key: key.to_owned(),
            at_unix_ms,
            body,
        })
        .map_err(|error| Refused::Failed(io::Error::other(error)))?;
        if line.len() + RELAY_ROOM > frame::MAX_MESSAGE {
            return Err(Refused::TooLarge);
        }
        if state.kept.len() >= BOX_MAX_EVENTS || state.bytes + line.len() + 1 > BOX_MAX_BYTES {
            return Err(Refused::Full);
        }
        state.append(&line).map_err(Refused::Failed)?;
        state.last = seq;
        state.bytes += line.len() + 1;
        if state.resumed {
            (self.send)(&line);
        }
        state.kept.push_back(Kept {
            seq,
            kind: kind.to_owned(),
            key: key.to_owned(),
            line,
        });
        Ok(())
    }

    /// The guest's core is connected and recorded the box as far as
    /// `position` says: everything after it is sent, and each new event as
    /// it is kept. A core whose place this box cannot follow (it names
    /// another box, or a place this one dropped or never reached) gets
    /// what is kept in a box started fresh, numbered from 1.
    pub fn resume(&self, position: &Position) -> io::Result<()> {
        let mut state = lock(&self.state);
        let follows =
            position.outbox == state.outbox && (state.acked..=state.last).contains(&position.seq);
        // Nothing was dropped yet, so a core that has seen no box, or
        // another one, takes this one from its first event.
        let from_first = position.outbox != state.outbox && state.acked == 0;
        if follows {
            state.acknowledge(&self.dir, position.seq)?;
        } else if !from_first {
            state.start_fresh(&self.dir)?;
        }
        for kept in &state.kept {
            (self.send)(&kept.line);
        }
        state.resumed = true;
        Ok(())
    }

    /// The guest's core recorded the box through `position`. A place in
    /// another box, or past this one's end, says nothing about it.
    pub fn ack(&self, position: &Position) -> io::Result<()> {
        let mut state = lock(&self.state);
        if position.outbox != state.outbox || position.seq > state.last {
            return Ok(());
        }
        state.acknowledge(&self.dir, position.seq)
    }

    /// The guest's core is gone: events are kept until it asks again.
    pub fn away(&self) {
        lock(&self.state).resumed = false;
    }
}

impl State {
    /// Cuts the file back to its last whole line.
    fn cut_to_whole_lines(&mut self) -> io::Result<()> {
        self.file.set_len(self.len)?;
        self.file.seek(SeekFrom::Start(self.len))?;
        self.torn = false;
        Ok(())
    }

    /// Appends one line and syncs it. What a failed write left is cut off
    /// again, now or before the next append, so it never joins the line
    /// after it.
    fn append(&mut self, line: &str) -> io::Result<()> {
        if self.torn {
            self.cut_to_whole_lines()?;
        }
        let written = self
            .file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.write_all(b"\n"))
            .and_then(|()| self.file.sync_all());
        if let Err(error) = written {
            self.torn = true;
            // Best effort here: the next append cuts again when this fails.
            let _ = self.cut_to_whole_lines();
            return Err(error);
        }
        self.len += line.len() as u64 + 1;
        Ok(())
    }

    /// Drops what the guest's core recorded. Its place is saved first: a
    /// stop after it leaves lines the place already covers.
    fn acknowledge(&mut self, dir: &Path, seq: u64) -> io::Result<()> {
        if seq <= self.acked {
            return Ok(());
        }
        save_acked(dir, &self.outbox, seq)?;
        self.acked = seq;
        while self.kept.front().is_some_and(|kept| kept.seq <= seq) {
            if let Some(kept) = self.kept.pop_front() {
                self.bytes -= kept.line.len() + 1;
            }
        }
        if self.kept.is_empty() {
            self.len = 0;
            self.cut_to_whole_lines()?;
            self.file.sync_all()
        } else if self.len as usize - self.bytes > BOX_MAX_BYTES {
            // The recorded lines ahead of the kept ones take a box's worth
            // of room themselves.
            self.rewrite(dir)
        } else {
            Ok(())
        }
    }

    /// Replaces the file with the kept lines.
    fn rewrite(&mut self, dir: &Path) -> io::Result<()> {
        let mut bytes = Vec::with_capacity(self.bytes);
        for kept in &self.kept {
            bytes.extend_from_slice(kept.line.as_bytes());
            bytes.push(b'\n');
        }
        let path = dir.join(BOX_FILE);
        atomic::write_file(&path, &bytes, Access::Private)?;
        self.file = private::open_own_file(&path, true)?;
        self.len = bytes.len() as u64;
        self.file.seek(SeekFrom::Start(self.len))?;
        self.torn = false;
        Ok(())
    }

    /// Numbers the kept events from 1 in a box of a new name. The lines
    /// are written before the place: a stop between the two is read as the
    /// new box with nothing recorded ([`EventBox::open`]).
    fn start_fresh(&mut self, dir: &Path) -> io::Result<()> {
        let outbox = fresh_outbox()?;
        let mut renumbered = VecDeque::with_capacity(self.kept.len());
        let mut bytes = 0;
        for (index, kept) in self.kept.iter().enumerate() {
            let seq = index as u64 + 1;
            // Only the two fields that place the event change; every other
            // one stays as its build wrote it.
            let mut event: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(&kept.line).map_err(io::Error::other)?;
            event.insert("outbox".to_owned(), outbox.clone().into());
            event.insert("seq".to_owned(), seq.into());
            let line = serde_json::to_string(&event).map_err(io::Error::other)?;
            bytes += line.len() + 1;
            renumbered.push_back(Kept {
                seq,
                kind: kept.kind.clone(),
                key: kept.key.clone(),
                line,
            });
        }
        let earlier = std::mem::replace(&mut self.kept, renumbered);
        let earlier_bytes = std::mem::replace(&mut self.bytes, bytes);
        if let Err(error) = self.rewrite(dir) {
            self.kept = earlier;
            self.bytes = earlier_bytes;
            return Err(error);
        }
        save_acked(dir, &outbox, 0)?;
        self.outbox = outbox;
        self.acked = 0;
        self.last = self.kept.len() as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use super::*;

    /// A box in `dir` and the lines it wrote its host.
    fn opened(dir: &Path) -> (EventBox, Arc<Mutex<Vec<Value>>>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&sent);
        let events = EventBox::open(
            dir,
            Box::new(move |line| lock(&heard).push(serde_json::from_str(line).unwrap())),
        )
        .unwrap();
        (events, sent)
    }

    fn letter(events: &EventBox, intent: &str) -> Result<(), Refused> {
        events.push("letter", intent, 7, json!({"intent": intent}))
    }

    fn place(outbox: &str, seq: u64) -> Position {
        Position {
            outbox: outbox.to_owned(),
            seq,
        }
    }

    fn seqs(sent: &Mutex<Vec<Value>>) -> Vec<(u64, String)> {
        lock(sent)
            .iter()
            .map(|event| {
                (
                    event["seq"].as_u64().unwrap(),
                    event["key"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    fn outbox_of(sent: &Mutex<Vec<Value>>) -> String {
        lock(sent)[0]["outbox"].as_str().unwrap().to_owned()
    }

    #[test]
    fn an_event_made_while_the_guest_is_away_waits_and_is_sent_in_order_once_it_asks() {
        let dir = tempfile::tempdir().unwrap();
        let (events, sent) = opened(dir.path());
        letter(&events, "a").unwrap();
        letter(&events, "b").unwrap();
        assert!(
            lock(&sent).is_empty(),
            "nothing is sent to a guest that is away"
        );

        // A core that has seen no box takes this one from its first event.
        events.resume(&place("", 0)).unwrap();
        assert_eq!(seqs(&sent), [(1, "a".into()), (2, "b".into())]);
        // A new event goes at once while the core is there, and waits
        // again once it is gone.
        letter(&events, "c").unwrap();
        assert_eq!(seqs(&sent).last(), Some(&(3, "c".into())));
        events.away();
        letter(&events, "d").unwrap();
        assert_eq!(seqs(&sent).len(), 3);
    }

    #[test]
    fn what_the_guest_recorded_is_dropped_and_the_rest_is_sent_again_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (events, sent) = opened(dir.path());
        for intent in ["a", "b", "c"] {
            letter(&events, intent).unwrap();
        }
        events.resume(&place("", 0)).unwrap();
        let outbox = outbox_of(&sent);
        events.ack(&place(&outbox, 2)).unwrap();
        // An acknowledgement of another box, or of an event never made,
        // says nothing about this one.
        events.ack(&place("other", 3)).unwrap();
        events.ack(&place(&outbox, 9)).unwrap();
        drop(events);

        // The node that replaces it reads the same box: only the event the
        // guest did not record is left, under the number it had.
        let (events, sent) = opened(dir.path());
        events.resume(&place(&outbox, 2)).unwrap();
        assert_eq!(seqs(&sent), [(3, "c".into())]);
        // The guest's place is itself an acknowledgement.
        events.away();
        events.resume(&place(&outbox, 3)).unwrap();
        assert_eq!(seqs(&sent).len(), 1);
        assert_eq!(std::fs::read(dir.path().join(BOX_FILE)).unwrap(), b"");
        letter(&events, "d").unwrap();
        assert_eq!(seqs(&sent).last(), Some(&(4, "d".into())));
    }

    #[test]
    fn a_letter_sent_again_under_the_same_intent_is_one_event() {
        let dir = tempfile::tempdir().unwrap();
        let (events, sent) = opened(dir.path());
        letter(&events, "a").unwrap();
        letter(&events, "a").unwrap();
        events.push("pane_ended", "a", 7, json!({})).unwrap();
        events.resume(&place("", 0)).unwrap();
        assert_eq!(seqs(&sent), [(1, "a".into()), (2, "a".into())]);
        assert_eq!(lock(&sent)[1]["kind"], "pane_ended");
    }

    #[test]
    fn a_full_box_refuses_a_new_event_and_keeps_every_one_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        let (events, sent) = opened(dir.path());
        let body = "x".repeat(frame::MAX_MESSAGE - 4096);
        let mut kept = 0;
        let refused = loop {
            match events.push(
                "letter",
                &format!("intent-{kept}"),
                7,
                json!({"body": body}),
            ) {
                Ok(()) => kept += 1,
                Err(refused) => break refused,
            }
        };
        assert!(matches!(refused, Refused::Full), "{refused:?}");
        assert_eq!(kept, BOX_MAX_BYTES / frame::MAX_MESSAGE);
        // One event over a message's size is refused as that, full or not.
        let over = "x".repeat(frame::MAX_MESSAGE);
        assert!(matches!(
            events.push("letter", "over", 7, json!({"body": over})),
            Err(Refused::TooLarge)
        ));
        events.resume(&place("", 0)).unwrap();
        assert_eq!(seqs(&sent).len(), kept);
        // Room comes back with the guest's acknowledgement.
        let outbox = outbox_of(&sent);
        events.ack(&place(&outbox, 1)).unwrap();
        letter(&events, "after").unwrap();
    }

    #[test]
    fn a_guest_whose_place_the_box_cannot_follow_gets_what_is_kept_numbered_from_one() {
        let dir = tempfile::tempdir().unwrap();
        let (events, sent) = opened(dir.path());
        for intent in ["a", "b", "c"] {
            letter(&events, intent).unwrap();
        }
        events.resume(&place("", 0)).unwrap();
        let outbox = outbox_of(&sent);
        events.ack(&place(&outbox, 1)).unwrap();
        events.away();
        lock(&sent).clear();

        // The guest lost its record: it names no box, and the first event
        // is gone here. What is kept reaches it in a new box it can take
        // from 1, and the product's keys make a repeat nothing.
        events.resume(&place("", 0)).unwrap();
        assert_eq!(seqs(&sent), [(1, "b".into()), (2, "c".into())]);
        let fresh = outbox_of(&sent);
        assert_ne!(fresh, outbox);
        assert_eq!(lock(&sent)[0]["body"], json!({"intent": "b"}));
        drop(events);

        // The new box is what the next node reads.
        let (events, sent) = opened(dir.path());
        events.resume(&place(&fresh, 1)).unwrap();
        assert_eq!(seqs(&sent), [(2, "c".into())]);
    }

    #[test]
    fn a_stop_between_a_fresh_boxs_lines_and_its_place_is_read_as_the_fresh_box() {
        let dir = tempfile::tempdir().unwrap();
        let line = |seq: u64| {
            json!({"v": 1, "outbox": "fresh", "seq": seq, "kind": "letter",
                   "key": format!("k{seq}"), "at_unix_ms": 7, "body": {}})
            .to_string()
        };
        std::fs::write(
            dir.path().join(BOX_FILE),
            format!("{}\n{}\n", line(1), line(2)),
        )
        .unwrap();
        save_acked(dir.path(), "earlier", 40).unwrap();
        let (events, sent) = opened(dir.path());
        events.resume(&place("earlier", 40)).unwrap();
        assert_eq!(seqs(&sent), [(1, "k1".into()), (2, "k2".into())]);
        assert_eq!(outbox_of(&sent), "fresh");
    }

    /// A box as recorded for the contract: its first line as this build
    /// writes one, its second as a later build of the same guest might,
    /// with fields this build does not know.
    const RECORDED: &str = include_str!("../../../contracts/fixtures/peer-link/event-box.jsonl");

    #[test]
    fn a_box_another_build_of_the_guest_wrote_is_sent_as_it_was_written_and_numbered_on() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(BOX_FILE), RECORDED).unwrap();
        let recorded: Vec<Value> = RECORDED
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let (events, sent) = opened(dir.path());
        events.resume(&place("recorded", 0)).unwrap();
        assert_eq!(*lock(&sent), recorded, "every field of every line");

        // This build writes the first line's shape, field for field: the
        // build before it and the one after it read the same.
        let first = &recorded[0];
        events
            .push(
                "letter",
                "w1:p1/report-1b",
                first["at_unix_ms"].as_u64().unwrap(),
                first["body"].clone(),
            )
            .unwrap();
        let mut written = lock(&sent)[2].clone();
        assert_eq!(written["seq"], 3, "numbered after the lines it found");
        written["seq"] = first["seq"].clone();
        written["key"] = first["key"].clone();
        assert_eq!(&written, first);
    }

    #[test]
    fn a_line_a_crash_cut_short_is_dropped_and_a_whole_line_it_cannot_read_refuses_the_box() {
        let dir = tempfile::tempdir().unwrap();
        let (events, _) = opened(dir.path());
        letter(&events, "a").unwrap();
        drop(events);
        let path = dir.path().join(BOX_FILE);
        let whole = std::fs::read(&path).unwrap();
        let mut cut = whole.clone();
        cut.extend_from_slice(br#"{"v":1,"outbox":"#);
        std::fs::write(&path, &cut).unwrap();
        let (events, sent) = opened(dir.path());
        events.resume(&place("", 0)).unwrap();
        assert_eq!(seqs(&sent), [(1, "a".into())]);
        assert_eq!(std::fs::read(&path).unwrap(), whole);
        drop(events);

        // A whole line with no place in a box is not this build's to
        // judge: the file stays as it is.
        let mut unread = whole.clone();
        unread.extend_from_slice(b"{\"later\":true}\n");
        std::fs::write(&path, &unread).unwrap();
        let refused = EventBox::open(dir.path(), Box::new(|_| {})).err().unwrap();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(&path).unwrap(), unread);
    }
}
