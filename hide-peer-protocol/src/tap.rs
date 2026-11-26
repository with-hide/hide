//! The tap: how a host's window draws a guest's panes (PRD D-77).
//!
//! Nothing else of the guest crosses into the host build. A frame is the
//! holder protocol's: a little-endian `u32` length, a kind byte, the
//! payload. A pane is a `u8` length and that many UTF-8 bytes; numbers are
//! little-endian. A reader skips a frame of a kind it does not know.
//!
//! The tap is lossy, as the phone relay's is: a node queues at most
//! [`CAP_BYTES`], and when a partial update would pass it the waiting
//! output goes and each pane in it is asked to redraw.

use std::fmt;

pub const MAGIC: [u8; 8] = *b"HIDETAP1";
pub const OLDEST_TAP: u16 = 1;
pub const NEWEST_TAP: u16 = PASTE_TAP;
/// The first tap that carries [`ToNode::Paste`]: a screen's paste, which the
/// node hands the pane's holder to wrap for the program's paste mode
/// (with-hide/hide issue 2952). A node on 1 skips that kind, so a host sends it
/// only to a node that welcomed it at this version or later, and types a
/// paste as [`ToNode::Input`] to one on 1, as it did before.
pub const PASTE_TAP: u16 = 2;

/// The most bytes of output a node keeps waiting for a slow host.
pub const CAP_BYTES: usize = 4 * 1024 * 1024;

/// The most bytes one frame carries, kind byte included: the holder
/// protocol's limit, since an `output` is a holder fragment re-addressed.
pub const MAX_FRAME: usize = 4 * 1024 * 1024 + 1024;

/// The most bytes of a pane's folder a `folder` frame carries.
pub const MAX_FOLDER: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
pub struct TapError(&'static str);

impl fmt::Display for TapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a tap frame is malformed: {}", self.0)
    }
}

impl std::error::Error for TapError {}

/// What the host sends the node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToNode {
    Hello {
        oldest: u16,
        newest: u16,
    },
    Input {
        pane: String,
        bytes: Vec<u8>,
    },
    /// Text a screen pasted, its line breaks carriage returns; from tap 2.
    Paste {
        pane: String,
        text: Vec<u8>,
    },
    Resize {
        pane: String,
        rows: u16,
        cols: u16,
    },
    Terminate {
        pane: String,
    },
    Redraw {
        pane: String,
    },
}

/// What the node sends the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToHost {
    Welcome {
        version: u16,
        panes: Vec<PaneSize>,
    },
    /// A holder fragment, re-addressed by pane, with the size it was drawn
    /// at. `full` starts a pane's picture; a partial one continues it.
    Output {
        pane: String,
        full: bool,
        rows: u16,
        cols: u16,
        bytes: Vec<u8>,
    },
    /// A pane started or was adopted.
    Pane(PaneSize),
    Gone {
        pane: String,
    },
    /// The folder a pane was started in, as the node's machine spells it
    /// (D-55): the host reads from it whose a call proved only by its
    /// folder is. Sent after the pane's `Welcome` entry or `Pane`; a host
    /// that predates it skips the kind, and a pane with none told is in no
    /// folder the host knows.
    Folder {
        pane: String,
        folder: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneSize {
    pub pane: String,
    pub rows: u16,
    pub cols: u16,
}

pub mod kind {
    pub const HELLO: u8 = 0x01;
    pub const INPUT: u8 = 0x02;
    pub const RESIZE: u8 = 0x03;
    pub const PASTE: u8 = 0x04;
    pub const TERMINATE: u8 = 0x05;
    pub const REDRAW: u8 = 0x10;
    pub const WELCOME: u8 = 0x81;
    pub const OUTPUT: u8 = 0x90;
    pub const PANE: u8 = 0x91;
    pub const GONE: u8 = 0x92;
    pub const FOLDER: u8 = 0x93;
}

fn frame(kind: u8, payload: Vec<u8>) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(5 + payload.len());
    bytes.extend_from_slice(&(payload.len() as u32 + 1).to_le_bytes());
    bytes.push(kind);
    bytes.extend_from_slice(&payload);
    bytes
}

fn put_pane(out: &mut Vec<u8>, pane: &str) -> Result<(), TapError> {
    let length = u8::try_from(pane.len()).map_err(|_| TapError("a pane id over 255 bytes"))?;
    out.push(length);
    out.extend_from_slice(pane.as_bytes());
    Ok(())
}

fn put_size(out: &mut Vec<u8>, rows: u16, cols: u16) {
    out.extend_from_slice(&rows.to_le_bytes());
    out.extend_from_slice(&cols.to_le_bytes());
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], TapError> {
        if self.0.len() < count {
            return Err(TapError("the payload ends early"));
        }
        let (head, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, TapError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, TapError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn pane(&mut self) -> Result<String, TapError> {
        let length = self.u8()? as usize;
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| TapError("a pane id is not UTF-8"))
    }

    fn pane_size(&mut self) -> Result<PaneSize, TapError> {
        Ok(PaneSize {
            pane: self.pane()?,
            rows: self.u16()?,
            cols: self.u16()?,
        })
    }

    fn rest(self) -> Vec<u8> {
        self.0.to_vec()
    }
}

/// The length of the kind byte and payload that follow a frame's prefix,
/// refused when it is empty or over [`MAX_FRAME`].
pub fn frame_length(prefix: [u8; 4]) -> Result<usize, TapError> {
    let length = u32::from_le_bytes(prefix) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(TapError("a frame length outside the limit"));
    }
    Ok(length)
}

impl ToNode {
    /// The whole frame, length prefix included.
    pub fn encode(&self) -> Result<Vec<u8>, TapError> {
        let mut out = Vec::new();
        let kind = match self {
            Self::Hello { oldest, newest } => {
                out.extend_from_slice(&MAGIC);
                put_size(&mut out, *oldest, *newest);
                kind::HELLO
            }
            Self::Input { pane, bytes } => {
                put_pane(&mut out, pane)?;
                out.extend_from_slice(bytes);
                kind::INPUT
            }
            Self::Paste { pane, text } => {
                put_pane(&mut out, pane)?;
                out.extend_from_slice(text);
                kind::PASTE
            }
            Self::Resize { pane, rows, cols } => {
                put_pane(&mut out, pane)?;
                put_size(&mut out, *rows, *cols);
                kind::RESIZE
            }
            Self::Terminate { pane } => {
                put_pane(&mut out, pane)?;
                kind::TERMINATE
            }
            Self::Redraw { pane } => {
                put_pane(&mut out, pane)?;
                kind::REDRAW
            }
        };
        Ok(frame(kind, out))
    }

    /// Reads a frame's kind byte and payload; `None` for an unknown kind.
    pub fn decode(kind: u8, payload: &[u8]) -> Result<Option<Self>, TapError> {
        let mut reader = Reader(payload);
        Ok(Some(match kind {
            kind::HELLO => {
                if reader.take(MAGIC.len())? != MAGIC {
                    return Err(TapError("the hello does not start with the tap magic"));
                }
                Self::Hello {
                    oldest: reader.u16()?,
                    newest: reader.u16()?,
                }
            }
            kind::INPUT => Self::Input {
                pane: reader.pane()?,
                bytes: reader.rest(),
            },
            kind::PASTE => Self::Paste {
                pane: reader.pane()?,
                text: reader.rest(),
            },
            kind::RESIZE => Self::Resize {
                pane: reader.pane()?,
                rows: reader.u16()?,
                cols: reader.u16()?,
            },
            kind::TERMINATE => Self::Terminate {
                pane: reader.pane()?,
            },
            kind::REDRAW => Self::Redraw {
                pane: reader.pane()?,
            },
            _ => return Ok(None),
        }))
    }
}

impl ToHost {
    /// The whole frame, length prefix included.
    pub fn encode(&self) -> Result<Vec<u8>, TapError> {
        let mut out = Vec::new();
        let kind = match self {
            Self::Welcome { version, panes } => {
                let count = u16::try_from(panes.len()).map_err(|_| TapError("over 65535 panes"))?;
                put_size(&mut out, *version, count);
                for pane in panes {
                    put_pane(&mut out, &pane.pane)?;
                    put_size(&mut out, pane.rows, pane.cols);
                }
                kind::WELCOME
            }
            Self::Output {
                pane,
                full,
                rows,
                cols,
                bytes,
            } => {
                put_pane(&mut out, pane)?;
                out.push(u8::from(*full));
                put_size(&mut out, *rows, *cols);
                out.extend_from_slice(bytes);
                kind::OUTPUT
            }
            Self::Pane(pane) => {
                put_pane(&mut out, &pane.pane)?;
                put_size(&mut out, pane.rows, pane.cols);
                kind::PANE
            }
            Self::Gone { pane } => {
                put_pane(&mut out, pane)?;
                kind::GONE
            }
            Self::Folder { pane, folder } => {
                if folder.len() > MAX_FOLDER {
                    return Err(TapError("a folder over the limit"));
                }
                put_pane(&mut out, pane)?;
                out.extend_from_slice(folder.as_bytes());
                kind::FOLDER
            }
        };
        if out.len() + 1 > MAX_FRAME {
            return Err(TapError("a frame over the limit"));
        }
        Ok(frame(kind, out))
    }

    /// Reads a frame's kind byte and payload; `None` for an unknown kind.
    pub fn decode(kind: u8, payload: &[u8]) -> Result<Option<Self>, TapError> {
        let mut reader = Reader(payload);
        Ok(Some(match kind {
            kind::WELCOME => {
                let version = reader.u16()?;
                let count = reader.u16()?;
                let panes = (0..count)
                    .map(|_| reader.pane_size())
                    .collect::<Result<_, _>>()?;
                Self::Welcome { version, panes }
            }
            kind::OUTPUT => Self::Output {
                pane: reader.pane()?,
                full: reader.u8()? != 0,
                rows: reader.u16()?,
                cols: reader.u16()?,
                bytes: reader.rest(),
            },
            kind::PANE => Self::Pane(reader.pane_size()?),
            kind::GONE => Self::Gone {
                pane: reader.pane()?,
            },
            kind::FOLDER => {
                let pane = reader.pane()?;
                let folder = reader.rest();
                if folder.len() > MAX_FOLDER {
                    return Err(TapError("a folder over the limit"));
                }
                Self::Folder {
                    pane,
                    folder: String::from_utf8(folder)
                        .map_err(|_| TapError("a folder is not UTF-8"))?,
                }
            }
            _ => return Ok(None),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(frame: &[u8]) -> (u8, &[u8]) {
        let length = frame_length(frame[..4].try_into().unwrap()).unwrap();
        assert_eq!(length, frame.len() - 4);
        (frame[4], &frame[5..])
    }

    #[test]
    fn every_frame_reads_back_as_it_was_written() {
        let pane = || "w1_p2".to_owned();
        for message in [
            ToNode::Hello {
                oldest: 1,
                newest: 1,
            },
            ToNode::Input {
                pane: pane(),
                bytes: b"ls\r".to_vec(),
            },
            ToNode::Paste {
                pane: pane(),
                text: b"one\rtwo".to_vec(),
            },
            ToNode::Resize {
                pane: pane(),
                rows: 40,
                cols: 120,
            },
            ToNode::Terminate { pane: pane() },
            ToNode::Redraw { pane: pane() },
        ] {
            let frame = message.encode().unwrap();
            let (kind, payload) = split(&frame);
            assert_eq!(ToNode::decode(kind, payload), Ok(Some(message)));
        }
        let size = || PaneSize {
            pane: pane(),
            rows: 40,
            cols: 120,
        };
        for message in [
            ToHost::Welcome {
                version: 1,
                panes: vec![size(), size()],
            },
            ToHost::Output {
                pane: pane(),
                full: true,
                rows: 40,
                cols: 120,
                bytes: b"\x1b[2Jhi".to_vec(),
            },
            ToHost::Pane(size()),
            ToHost::Gone { pane: pane() },
            ToHost::Folder {
                pane: pane(),
                folder: "C:/work/shop-api".to_owned(),
            },
        ] {
            let frame = message.encode().unwrap();
            let (kind, payload) = split(&frame);
            assert_eq!(ToHost::decode(kind, payload), Ok(Some(message)));
        }
    }

    #[test]
    fn an_unknown_kind_is_skipped_and_a_short_payload_is_refused() {
        assert_eq!(ToHost::decode(0xf0, b"anything"), Ok(None));
        assert_eq!(ToNode::decode(0x7f, b""), Ok(None));
        assert!(ToHost::decode(kind::PANE, &[5, b'w']).is_err());
        let long = ToHost::Folder {
            pane: "w1_p2".to_owned(),
            folder: "x".repeat(MAX_FOLDER + 1),
        };
        assert!(long.encode().is_err());
        let mut payload = vec![1, b'p'];
        payload.extend(std::iter::repeat_n(b'x', MAX_FOLDER + 1));
        assert!(ToHost::decode(kind::FOLDER, &payload).is_err());
        assert!(ToNode::decode(kind::HELLO, b"NOTATAP!\x01\0\x01\0").is_err());
        assert!(frame_length([0; 4]).is_err());
    }
}
