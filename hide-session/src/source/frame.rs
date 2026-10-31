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
    /// carries a pasted image: read to its newline and dropped.
    Passed,
}

/// The records of `reader` from where it stands, each at most `limit`
/// bytes kept. The caller bounds the whole scan by what `reader` yields
/// (`Read::take`).
pub(crate) struct Head<R> {
    reader: R,
    framer: Framer,
    block: Vec<u8>,
    block_bytes: usize,
    at: usize,
    ended: bool,
}

const HEAD_BLOCK_BYTES: usize = 64 * 1024;

impl<R: Read> Head<R> {
    pub(crate) fn new(reader: R, limit: usize) -> Self {
        Self {
            reader,
            framer: Framer::new(limit, 0),
            block: Vec::new(),
            block_bytes: HEAD_BLOCK_BYTES,
            at: 0,
            ended: false,
        }
    }

    /// Read `bytes` at a time rather than [`HEAD_BLOCK_BYTES`], for a scan
    /// whose reads are charged to a budget.
    pub(crate) fn with_block(mut self, bytes: usize) -> Self {
        self.block_bytes = bytes.max(1);
        self
    }

    /// The bytes of every record handed out so far, through its newline.
    pub(crate) fn consumed(&self) -> u64 {
        self.framer.settled()
    }

    /// The next record, `None` at the end of the reader.
    pub(crate) fn next_record(&mut self) -> io::Result<Option<HeadRecord>> {
        loop {
            match self.framer.next(&self.block, &mut self.at) {
                Some(Frame::Record { bytes, .. }) => {
                    return Ok(Some(HeadRecord::Complete(trim_newline(bytes).to_vec())));
                }
                Some(Frame::Over { ended: true, .. }) => return Ok(Some(HeadRecord::Passed)),
                Some(Frame::Over { .. }) => continue,
                None => {}
            }
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
            if read == 0 {
                self.ended = true;
                let last = if self.framer.passing_from().is_some() {
                    Some(HeadRecord::Passed)
                } else if self.framer.pending().is_empty() {
                    None
                } else {
                    Some(HeadRecord::Unfinished(self.framer.pending().to_vec()))
                };
                let offset = self.framer.offset();
                self.framer.rewind(offset);
                return Ok(last);
            }
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
