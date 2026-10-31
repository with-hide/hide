//! Cutting session bytes into records: the one place a JSONL reader decides
//! where a record ends, what a record longer than its cap becomes, and what
//! happens to a last record that has no newline yet.
//!
//! A record is the bytes through its newline. One of at most `limit` bytes
//! is handed over whole; a longer one is handed over piece by piece as it is
//! read and never held, so the cap bounds memory, never what a session may
//! hold. A last record without its newline is held (and not handed over)
//! until a later chunk ends it: it is still being written.

use std::io::{self, Read};

/// Splits a session's bytes into records, one chunk at a time, carrying a
/// record that a chunk cuts into the next chunk.
#[derive(Debug)]
pub(crate) struct Framer {
    limit: usize,
    /// The absolute offset of the next byte fed.
    offset: u64,
    /// The record being read, while it is within `limit`.
    pending: Vec<u8>,
    /// Where the record being passed over began, while one is.
    over: Option<u64>,
    /// The bytes the last frame lends out of the framer.
    lent: Vec<u8>,
}

impl Clone for Framer {
    /// The position and the held record; the bytes a frame lent are not
    /// part of the state.
    fn clone(&self) -> Self {
        Self {
            limit: self.limit,
            offset: self.offset,
            pending: self.pending.clone(),
            over: self.over,
            lent: Vec::new(),
        }
    }
}

/// One step of [`Framer::next`].
#[derive(Debug)]
pub(crate) enum Frame<'a> {
    /// A whole record of at most `limit` bytes, newline included.
    Record { start: u64, bytes: &'a [u8] },
    /// Bytes of a record longer than `limit`, in the order they are read.
    /// The piece that crosses the limit is `first` and carries in `kept`
    /// what the framer held of the record before it; `ended` when the piece
    /// holds the record's newline.
    Over {
        start: u64,
        first: bool,
        kept: &'a [u8],
        bytes: &'a [u8],
        ended: bool,
    },
}

impl Framer {
    /// A framer whose next byte is at `offset`, a record start.
    pub(crate) fn new(limit: usize, offset: u64) -> Self {
        Self {
            limit,
            offset,
            pending: Vec::new(),
            over: None,
            lent: Vec::new(),
        }
    }

    /// A framer resuming inside a record longer than `limit` that began at
    /// `start`, with its next byte at `offset`.
    pub(crate) fn passing(limit: usize, start: u64, offset: u64) -> Self {
        Self {
            over: Some(start),
            ..Self::new(limit, offset)
        }
    }

    /// The offset after every byte fed so far.
    pub(crate) fn offset(&self) -> u64 {
        self.offset
    }

    /// Where a reader that keeps no bytes resumes: the start of a record the
    /// framer holds without its newline, or the offset after every byte fed.
    pub(crate) fn settled(&self) -> u64 {
        self.offset - self.pending.len() as u64
    }

    /// The bytes of the unfinished record the framer holds.
    pub(crate) fn pending(&self) -> &[u8] {
        &self.pending
    }

    /// Where the record being passed over began, while one is.
    pub(crate) fn passing_from(&self) -> Option<u64> {
        self.over
    }

    /// Forget what the framer holds and read on from `offset`, a record start.
    pub(crate) fn rewind(&mut self, offset: u64) {
        self.offset = offset;
        self.pending.clear();
        self.over = None;
    }

    /// Whether [`Self::next`] hands over a frame from `rest`, rather than
    /// holding all of it as the start of a record.
    fn yields(&self, rest: &[u8]) -> bool {
        !rest.is_empty()
            && (self.over.is_some()
                || memchr::memchr(b'\n', rest).is_some()
                || self.pending.len() + rest.len() > self.limit)
    }

    /// The next frame of `chunk` from `*at`, which it moves past the bytes
    /// it took; `None` once the chunk is spent, its last bytes held when they
    /// end no record.
    pub(crate) fn next<'a>(&'a mut self, chunk: &'a [u8], at: &mut usize) -> Option<Frame<'a>> {
        self.lent.clear();
        loop {
            let rest = chunk.get(*at..).filter(|rest| !rest.is_empty())?;
            let (end, ended) = match memchr::memchr(b'\n', rest) {
                Some(newline) => (newline + 1, true),
                None => (rest.len(), false),
            };
            let piece = &rest[..end];
            *at += end;
            let start = self.over.unwrap_or(self.offset - self.pending.len() as u64);
            self.offset += end as u64;
            if self.over.is_some() {
                if ended {
                    self.over = None;
                }
                return Some(Frame::Over {
                    start,
                    first: false,
                    kept: &[],
                    bytes: piece,
                    ended,
                });
            }
            if self.pending.len() + piece.len() > self.limit {
                std::mem::swap(&mut self.lent, &mut self.pending);
                if !ended {
                    self.over = Some(start);
                }
                return Some(Frame::Over {
                    start,
                    first: true,
                    kept: &self.lent,
                    bytes: piece,
                    ended,
                });
            }
            if !ended {
                self.pending.extend_from_slice(piece);
                continue;
            }
            if self.pending.is_empty() {
                return Some(Frame::Record {
                    start,
                    bytes: piece,
                });
            }
            self.pending.extend_from_slice(piece);
            std::mem::swap(&mut self.lent, &mut self.pending);
            return Some(Frame::Record {
                start,
                bytes: &self.lent,
            });
        }
    }
}

/// `record` without its newline (and a carriage return before it).
pub(crate) fn trim_newline(record: &[u8]) -> &[u8] {
    let mut end = record.len();
    while end > 0 && matches!(record[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    &record[..end]
}

/// The records of a reader read forward in blocks: the one loop that reads
/// a session file a block at a time and hands its records over.
pub(crate) struct Forward<R> {
    reader: R,
    framer: Framer,
    block: Vec<u8>,
    block_bytes: usize,
    at: usize,
    ended: bool,
    read: u64,
}

/// The block a forward read takes at a time: a buffered reader's, so a fact
/// on the first records costs one small read.
pub(crate) const BLOCK_BYTES: usize = 8 * 1024;

impl<R: Read> Forward<R> {
    /// The records of `reader`, whose next byte is at `offset`, each at most
    /// `limit` bytes kept.
    pub(crate) fn new(reader: R, limit: usize, offset: u64) -> Self {
        Self::from_framer(reader, Framer::new(limit, offset))
    }

    /// Records read on from where `framer` stands.
    pub(crate) fn from_framer(reader: R, framer: Framer) -> Self {
        Self {
            reader,
            framer,
            block: Vec::new(),
            block_bytes: BLOCK_BYTES,
            at: 0,
            ended: false,
            read: 0,
        }
    }

    /// Read `bytes` at a time rather than [`BLOCK_BYTES`].
    pub(crate) fn with_block(mut self, bytes: usize) -> Self {
        self.block_bytes = bytes.max(1);
        self
    }

    /// The framer's position: where the next record starts once the held
    /// bytes are handed over.
    pub(crate) fn framer(&self) -> &Framer {
        &self.framer
    }

    /// Every byte the reader gave, handed over or not.
    pub(crate) fn read_bytes(&self) -> u64 {
        self.read
    }

    /// The next frame, `None` once the reader is spent; a last record
    /// without its newline is then held by the framer, not handed over.
    pub(crate) fn next_frame(&mut self) -> io::Result<Option<Frame<'_>>> {
        loop {
            if self.framer.yields(&self.block[self.at..]) {
                return Ok(self.framer.next(&self.block, &mut self.at));
            }
            // Bytes that end no record: the framer holds them.
            let _ = self.framer.next(&self.block, &mut self.at);
            if self.ended {
                return Ok(None);
            }
            self.block.resize(self.block_bytes, 0);
            let read = loop {
                match self.reader.read(&mut self.block) {
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    other => break other?,
                }
            };
            self.block.truncate(read);
            self.at = 0;
            self.read += read as u64;
            if read == 0 {
                self.ended = true;
                return Ok(None);
            }
        }
    }
}

/// One record of a forward scan from a session's start ([`Head`]), for the
/// facts a record names (its id, its cwd, its header).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HeadRecord {
    /// A whole record, without its newline.
    Complete(Vec<u8>),
    /// The last bytes read, with no newline yet: a record still being
    /// written, or one the scan's budget cut.
    Unfinished(Vec<u8>),
    /// A record longer than the scan keeps, such as a first message that
    /// carries a pasted image: read to its newline and dropped, or, in a
    /// strict scan, left as soon as it crosses the cap.
    Passed,
}

/// The records of `reader` from where it stands, each at most `limit`
/// bytes kept. The caller bounds the whole scan by what `reader` yields
/// (`Read::take`).
pub(crate) struct Head<R> {
    forward: Forward<R>,
    strict: bool,
    stopped: bool,
}

impl<R: Read> Head<R> {
    pub(crate) fn new(reader: R, limit: usize) -> Self {
        Self {
            forward: Forward::new(reader, limit, 0),
            strict: false,
            stopped: false,
        }
    }

    /// Read `bytes` at a time rather than [`BLOCK_BYTES`], for a scan whose
    /// reads are charged to a budget.
    pub(crate) fn with_block(mut self, bytes: usize) -> Self {
        self.forward = self.forward.with_block(bytes);
        self
    }

    /// Whether a record over the cap ends the scan as it crosses the cap,
    /// rather than being read to its newline: a record that must name the
    /// session is refused, never drained.
    pub(crate) fn set_strict(&mut self, strict: bool) {
        self.strict = strict;
    }

    /// Every byte the scan took into its records, kept or passed over,
    /// including a record it holds without its newline.
    pub(crate) fn fed(&self) -> u64 {
        self.forward.framer().offset()
    }

    /// The next record, `None` at the end of the reader.
    pub(crate) fn next_record(&mut self) -> io::Result<Option<HeadRecord>> {
        if self.stopped {
            return Ok(None);
        }
        let strict = self.strict;
        loop {
            match self.forward.next_frame()? {
                Some(Frame::Record { bytes, .. }) => {
                    return Ok(Some(HeadRecord::Complete(trim_newline(bytes).to_vec())));
                }
                Some(Frame::Over { ended, .. }) if ended || strict => {
                    self.stopped = !ended;
                    return Ok(Some(HeadRecord::Passed));
                }
                Some(Frame::Over { .. }) => continue,
                None => {}
            }
            self.stopped = true;
            let framer = self.forward.framer();
            return Ok(if framer.passing_from().is_some() {
                Some(HeadRecord::Passed)
            } else if framer.pending().is_empty() {
                None
            } else {
                Some(HeadRecord::Unfinished(framer.pending().to_vec()))
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(limit: usize, chunks: &[&[u8]]) -> Vec<String> {
        let mut framer = Framer::new(limit, 0);
        let mut out = Vec::new();
        for chunk in chunks {
            let mut at = 0;
            while let Some(frame) = framer.next(chunk, &mut at) {
                out.push(match frame {
                    Frame::Record { start, bytes } => {
                        format!(
                            "record@{start}:{}",
                            String::from_utf8_lossy(bytes).trim_end()
                        )
                    }
                    Frame::Over {
                        start,
                        first,
                        kept,
                        bytes,
                        ended,
                    } => format!(
                        "over@{start}:{first}:{}+{}:{ended}",
                        kept.len(),
                        bytes.len()
                    ),
                });
            }
        }
        out.push(format!("settled:{}", framer.settled()));
        out
    }

    #[test]
    fn a_record_cut_by_a_chunk_is_handed_over_whole_from_the_next() {
        assert_eq!(
            frames(16, &[b"ab\ncd", b"ef\ngh"]),
            ["record@0:ab", "record@3:cdef", "settled:8"]
        );
    }

    #[test]
    fn a_record_past_the_limit_is_passed_piece_by_piece_and_the_next_is_whole() {
        assert_eq!(
            frames(4, &[b"abc", b"defg", b"h\nij\n"]),
            [
                "over@0:true:3+4:false",
                "over@0:false:0+2:true",
                "record@9:ij",
                "settled:12"
            ]
        );
    }

    #[test]
    fn a_record_of_exactly_the_limit_with_its_newline_is_kept() {
        assert_eq!(
            frames(3, &[b"ab\nabc\n"]),
            ["record@0:ab", "over@3:true:0+4:true", "settled:7"]
        );
    }

    #[test]
    fn a_resumed_pass_ends_at_the_next_newline() {
        let mut framer = Framer::passing(4, 2, 10);
        let mut at = 0;
        let chunk = b"xyz\nok\n";
        assert!(matches!(
            framer.next(chunk, &mut at),
            Some(Frame::Over {
                start: 2,
                first: false,
                ended: true,
                ..
            })
        ));
        assert!(matches!(
            framer.next(chunk, &mut at),
            Some(Frame::Record { start: 14, .. })
        ));
    }

    #[test]
    fn a_head_scan_passes_over_a_long_record_and_keeps_an_unfinished_one() {
        let bytes = format!("{}\n{{\"a\":1}}\r\n{{\"b\"", "x".repeat(200_000));
        let mut head = Head::new(bytes.as_bytes(), 1024);
        assert_eq!(head.next_record().unwrap(), Some(HeadRecord::Passed));
        assert_eq!(
            head.next_record().unwrap(),
            Some(HeadRecord::Complete(b"{\"a\":1}".to_vec()))
        );
        assert_eq!(
            head.next_record().unwrap(),
            Some(HeadRecord::Unfinished(b"{\"b\"".to_vec()))
        );
        assert_eq!(head.next_record().unwrap(), None);
    }

    #[test]
    fn a_head_scan_ending_inside_a_long_record_passes_it() {
        let bytes = "y".repeat(5000);
        let mut head = Head::new(bytes.as_bytes(), 1024);
        assert_eq!(head.next_record().unwrap(), Some(HeadRecord::Passed));
        assert_eq!(head.next_record().unwrap(), None);
    }
}
