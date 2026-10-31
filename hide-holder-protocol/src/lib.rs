//! What a node and a pane's holder say to each other.
//!
//! A holder keeps one pane's terminal (its PTY, its program and the parsed
//! screen) and lives on while the node that started it ends and starts
//! again. A node reaches it over the local stream (`hide_platform::ipc`) at
//! the path it started the holder with, and holds at most one connection to
//! it at a time: a connection whose first message is accepted takes the
//! holder over, and the connection it replaces is told so and closed.
//!
//! Every message is a frame: a little-endian `u32` length, then a kind byte
//! and that many bytes less one of payload. A frame longer than
//! [`MAX_FRAME`] is refused, so neither side ever holds more than that for
//! one message.
//!
//! The first message on a connection is the node's [`Hello`]: the protocol
//! versions it speaks and the capabilities it accepts. It must arrive whole
//! within [`HELLO_LIMIT`] and [`MAX_HELLO`] bytes, or the holder closes that
//! connection alone, and a connection it already serves goes on. The holder
//! answers [`Welcome`] with the one version both speak, or [`Closed`] with
//! [`CloseReason::UnsupportedVersion`] and its own version when there is
//! none. A side never sends a message the other did not say it accepts.
//!
//! In version 1 a node sends input bytes, a size, a release and a
//! terminate, and the holder sends what the pane's program wrote and why it
//! closed. Releasing lets go of the connection and leaves the pane running;
//! terminating ends the pane's program and everything it started, then the
//! holder.
//!
//! [`Capability::RawOutput`] carries the program's output as the bytes the
//! PTY gave. It is the holder's first output path and a stand-in: the screen
//! the holder parses goes out as display-only screen fragments, which
//! replace it, and once they do this capability is removed, not kept beside
//! them (PRD engine-bundle-2-pane-holder D-7).

use std::io::{self, Read, Write};
use std::time::Duration;

pub mod record;

/// The version of this protocol this build speaks as a holder.
pub const PROTOCOL_VERSION: u16 = 1;

/// The oldest holder version this build speaks to as a node. A holder older
/// than this is reported as unsupported and left running, never ended.
pub const OLDEST_HOLDER_VERSION: u16 = 1;

/// The most bytes one frame carries, kind byte included.
pub const MAX_FRAME: usize = 1024 * 1024;

/// The most bytes a [`Hello`] frame carries.
pub const MAX_HELLO: usize = 4 * 1024;

/// How long a holder waits for a connection's [`Hello`].
pub const HELLO_LIMIT: Duration = Duration::from_secs(2);

/// The most input bytes one [`ClientMessage::Input`] carries.
pub const MAX_INPUT: usize = 64 * 1024;

/// The most capabilities one side names.
pub const MAX_CAPABILITIES: usize = 32;

/// The most bytes of a [`Closed`]'s detail.
pub const MAX_DETAIL: usize = 4 * 1024;

/// The most pids a [`Closed`] names.
pub const MAX_LEFT: usize = 256;

/// The largest terminal a holder keeps: rows, columns, and cells.
pub const MAX_ROWS: u16 = 1024;
pub const MAX_COLS: u16 = 1024;
pub const MAX_CELLS: u32 = 512 * 512;

/// What opens every [`Hello`], so a stream that is not a node is refused at
/// its first bytes.
const MAGIC: &[u8; 8] = b"HIDEHOLD";

mod kind {
    pub const HELLO: u8 = 0x01;
    pub const INPUT: u8 = 0x02;
    pub const RESIZE: u8 = 0x03;
    pub const RELEASE: u8 = 0x04;
    pub const TERMINATE: u8 = 0x05;
    pub const WELCOME: u8 = 0x81;
    pub const OUTPUT: u8 = 0x82;
    pub const CLOSED: u8 = 0x83;
}

/// Something one side can do that the other must accept before it is used.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    /// The holder sends the program's output as raw PTY bytes
    /// ([`HolderMessage::Output`]). See the crate documentation.
    RawOutput,
    /// A capability this build does not know, kept by name so a side can
    /// answer with the ones it does.
    Other(String),
}

impl Capability {
    fn name(&self) -> &str {
        match self {
            Self::RawOutput => "raw_output",
            Self::Other(name) => name,
        }
    }

    fn from_name(name: &str) -> Self {
        match name {
            "raw_output" => Self::RawOutput,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// A terminal's size in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

impl Size {
    /// The size, or why a holder refuses it.
    pub fn checked(rows: u16, cols: u16) -> Result<Self, String> {
        if rows == 0 || cols == 0 {
            return Err(format!("{rows}x{cols} has no cells"));
        }
        if rows > MAX_ROWS || cols > MAX_COLS || u32::from(rows) * u32::from(cols) > MAX_CELLS {
            return Err(format!(
                "{rows}x{cols} is larger than {MAX_ROWS} rows, {MAX_COLS} columns or {MAX_CELLS} cells"
            ));
        }
        Ok(Self { rows, cols })
    }
}

/// A node's first message on a connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    /// The oldest and newest protocol versions the node speaks.
    pub versions: (u16, u16),
    /// What the node accepts from the holder.
    pub accepts: Vec<Capability>,
}

impl Hello {
    /// What this build sends: every holder version it speaks, and `accepts`.
    pub fn current(accepts: Vec<Capability>) -> Self {
        Self {
            versions: (OLDEST_HOLDER_VERSION, PROTOCOL_VERSION),
            accepts,
        }
    }
}

/// The holder's answer to an accepted [`Hello`]: this connection now holds
/// the pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    /// The version both sides speak from here on.
    pub version: u16,
    /// Which connection this is: one more than the one it took over.
    pub generation: u64,
    /// The pid of the pane's program.
    pub program: u32,
    /// The terminal's size now.
    pub size: Size,
    /// What the holder will send, out of what the node accepts.
    pub sends: Vec<Capability>,
}

/// Why a holder closed a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    /// The node let go of the connection; the pane runs on.
    Released,
    /// The node asked the pane to end, and it ended whole.
    Terminated,
    /// The pane's program exited on its own, and what it left ended.
    ProgramExited,
    /// Another connection took the holder over. Not a failure to retry: the
    /// pane is someone else's now.
    TakenOver,
    /// The first message was not a hello this build reads.
    HelloInvalid,
    /// The first message did not arrive within [`HELLO_LIMIT`].
    HelloTimeout,
    /// No version both sides speak; the holder's version is in [`Closed`].
    UnsupportedVersion,
    /// A frame was longer than its bound.
    FrameTooLarge,
    /// A message could not be read, or was one the node may not send.
    ProtocolError,
    /// The node sent input faster than the pane's program took it, past the
    /// holder's input bound.
    InputQueueFull,
    /// The node read output slower than the program wrote it, past the
    /// holder's output bound.
    OutputQueueFull,
    /// A size outside the bounds.
    SizeInvalid,
    /// The pane's processes were asked to end and some still ran when the
    /// bound passed; [`Closed::left`] names them.
    TreeEndUnconfirmed,
    /// The holder could not keep the pane; the detail says why.
    HolderFailed,
    /// The pane's terminal refused the input; the pane runs on, and input a
    /// later connection sends is tried again.
    InputFailed,
    /// The pane's terminal, or the holder's parser of it, refused the size;
    /// the detail says which.
    ResizeFailed,
    /// Too many connections were reading their hello at once; this one was
    /// closed unread. Opening again later is answered.
    OpeningFull,
}

impl CloseReason {
    const ALL: [Self; 17] = [
        Self::Released,
        Self::Terminated,
        Self::ProgramExited,
        Self::TakenOver,
        Self::HelloInvalid,
        Self::HelloTimeout,
        Self::UnsupportedVersion,
        Self::FrameTooLarge,
        Self::ProtocolError,
        Self::InputQueueFull,
        Self::OutputQueueFull,
        Self::SizeInvalid,
        Self::TreeEndUnconfirmed,
        Self::HolderFailed,
        Self::InputFailed,
        Self::ResizeFailed,
        Self::OpeningFull,
    ];

    /// The code on the wire, which never changes meaning.
    pub fn code(self) -> u16 {
        match self {
            Self::Released => 1,
            Self::Terminated => 2,
            Self::ProgramExited => 3,
            Self::TakenOver => 4,
            Self::HelloInvalid => 5,
            Self::HelloTimeout => 6,
            Self::UnsupportedVersion => 7,
            Self::FrameTooLarge => 8,
            Self::ProtocolError => 9,
            Self::InputQueueFull => 10,
            Self::OutputQueueFull => 11,
            Self::SizeInvalid => 12,
            Self::TreeEndUnconfirmed => 13,
            Self::HolderFailed => 14,
            Self::InputFailed => 15,
            Self::ResizeFailed => 16,
            Self::OpeningFull => 17,
        }
    }

    /// The reason a code names; `None` for a code this build does not know.
    pub fn from_code(code: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.code() == code)
    }

    /// The name logs and reports use.
    pub fn name(self) -> &'static str {
        match self {
            Self::Released => "released",
            Self::Terminated => "terminated",
            Self::ProgramExited => "program_exited",
            Self::TakenOver => "taken_over",
            Self::HelloInvalid => "hello_invalid",
            Self::HelloTimeout => "hello_timeout",
            Self::UnsupportedVersion => "unsupported_version",
            Self::FrameTooLarge => "frame_too_large",
            Self::ProtocolError => "protocol_error",
            Self::InputQueueFull => "input_queue_full",
            Self::OutputQueueFull => "output_queue_full",
            Self::SizeInvalid => "size_invalid",
            Self::TreeEndUnconfirmed => "tree_end_unconfirmed",
            Self::HolderFailed => "holder_failed",
            Self::InputFailed => "input_failed",
            Self::ResizeFailed => "resize_failed",
            Self::OpeningFull => "opening_full",
        }
    }
}

/// The holder's last message on a connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Closed {
    pub reason: CloseReason,
    /// The holder's protocol version, so a node can name a holder it does
    /// not speak to.
    pub version: u16,
    /// The pane's program's exit code, when it exited with one.
    pub exit_code: Option<i32>,
    /// The pids still running when an end passed its bound.
    pub left: Vec<u32>,
    /// What happened, for the log; never parsed.
    pub detail: String,
}

impl Closed {
    pub fn new(reason: CloseReason, detail: impl Into<String>) -> Self {
        let mut detail = detail.into();
        truncate(&mut detail, MAX_DETAIL);
        Self {
            reason,
            version: PROTOCOL_VERSION,
            exit_code: None,
            left: Vec::new(),
            detail,
        }
    }
}

/// What a node sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    Hello(Hello),
    /// Bytes for the pane's program, written as they are.
    Input(Vec<u8>),
    Resize(Size),
    /// Lets go of the connection; the pane runs on.
    Release,
    /// Ends the pane's program and everything it started, then the holder.
    Terminate,
}

/// What a holder sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HolderMessage {
    Welcome(Welcome),
    /// What the pane's program wrote, as the PTY gave it.
    Output(Vec<u8>),
    Closed(Closed),
}

/// Why a frame could not be read.
#[derive(Debug)]
pub enum FrameError {
    /// The stream failed or ended.
    Io(io::Error),
    /// The frame was longer than its bound.
    TooLarge(usize),
    /// The frame was not a message this build reads.
    Malformed(String),
}

impl FrameError {
    /// The reason a holder closes with for this error.
    pub fn close_reason(&self) -> CloseReason {
        match self {
            Self::TooLarge(_) => CloseReason::FrameTooLarge,
            Self::Io(error) if error.kind() == io::ErrorKind::TimedOut => CloseReason::HelloTimeout,
            _ => CloseReason::ProtocolError,
        }
    }
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::TooLarge(length) => write!(f, "a frame of {length} bytes is past the bound"),
            Self::Malformed(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn truncate(text: &mut String, bound: usize) {
    if text.len() > bound {
        let mut end = bound;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// Reads one frame of at most `bound` bytes: its kind and its payload.
pub fn read_frame(reader: &mut impl Read, bound: usize) -> Result<(u8, Vec<u8>), FrameError> {
    let mut length = [0u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 {
        return Err(FrameError::Malformed("an empty frame".to_owned()));
    }
    if length > bound {
        return Err(FrameError::TooLarge(length));
    }
    let mut frame = vec![0u8; length];
    reader.read_exact(&mut frame)?;
    let kind = frame.remove(0);
    Ok((kind, frame))
}

fn write_frame(writer: &mut impl Write, kind: u8, payload: &[u8]) -> io::Result<()> {
    let length = payload.len() + 1;
    if length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("a frame of {length} bytes is past the bound"),
        ));
    }
    let mut frame = Vec::with_capacity(length + 4);
    frame.extend_from_slice(&(length as u32).to_le_bytes());
    frame.push(kind);
    frame.extend_from_slice(payload);
    writer.write_all(&frame)?;
    writer.flush()
}

/// A cursor over a payload that refuses to read past its end.
struct Payload<'a>(&'a [u8]);

impl Payload<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], FrameError> {
        if self.0.len() < count {
            return Err(FrameError::Malformed("a message ended early".to_owned()));
        }
        let (head, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, FrameError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, FrameError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, FrameError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, FrameError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn text(&mut self, bound: usize) -> Result<String, FrameError> {
        let length = self.u16()? as usize;
        if length > bound {
            return Err(FrameError::Malformed(format!(
                "a text of {length} bytes is past its bound"
            )));
        }
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| FrameError::Malformed("a text is not UTF-8".to_owned()))
    }

    fn capabilities(&mut self) -> Result<Vec<Capability>, FrameError> {
        let count = self.u8()? as usize;
        if count > MAX_CAPABILITIES {
            return Err(FrameError::Malformed(format!(
                "{count} capabilities are past the bound"
            )));
        }
        (0..count)
            .map(|_| self.text(64).map(|name| Capability::from_name(&name)))
            .collect()
    }

    fn size(&mut self) -> Result<Size, FrameError> {
        let rows = self.u16()?;
        let cols = self.u16()?;
        Ok(Size { rows, cols })
    }

    fn end(self) -> Result<(), FrameError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(FrameError::Malformed(
                "a message carries more than it should".to_owned(),
            ))
        }
    }
}

fn put_text(payload: &mut Vec<u8>, text: &str) {
    payload.extend_from_slice(&(text.len() as u16).to_le_bytes());
    payload.extend_from_slice(text.as_bytes());
}

fn put_capabilities(payload: &mut Vec<u8>, capabilities: &[Capability]) {
    let count = capabilities.len().min(MAX_CAPABILITIES);
    payload.push(count as u8);
    for capability in &capabilities[..count] {
        put_text(payload, capability.name());
    }
}

/// Writes one message from a node.
pub fn write_client(writer: &mut impl Write, message: &ClientMessage) -> io::Result<()> {
    match message {
        ClientMessage::Hello(hello) => {
            let mut payload = MAGIC.to_vec();
            payload.extend_from_slice(&hello.versions.0.to_le_bytes());
            payload.extend_from_slice(&hello.versions.1.to_le_bytes());
            put_capabilities(&mut payload, &hello.accepts);
            write_frame(writer, kind::HELLO, &payload)
        }
        ClientMessage::Input(bytes) => {
            if bytes.len() > MAX_INPUT {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{} input bytes are past the bound of {MAX_INPUT}",
                        bytes.len()
                    ),
                ));
            }
            write_frame(writer, kind::INPUT, bytes)
        }
        ClientMessage::Resize(size) => {
            let mut payload = size.rows.to_le_bytes().to_vec();
            payload.extend_from_slice(&size.cols.to_le_bytes());
            write_frame(writer, kind::RESIZE, &payload)
        }
        ClientMessage::Release => write_frame(writer, kind::RELEASE, &[]),
        ClientMessage::Terminate => write_frame(writer, kind::TERMINATE, &[]),
    }
}

/// Reads a connection's first message, which must be a [`Hello`] within
/// [`MAX_HELLO`] bytes. The caller bounds the time.
pub fn read_hello(reader: &mut impl Read) -> Result<Hello, FrameError> {
    let (kind, payload) = read_frame(reader, MAX_HELLO)?;
    if kind != kind::HELLO {
        return Err(FrameError::Malformed(format!(
            "the first message is of kind {kind:#04x}, not a hello"
        )));
    }
    let mut payload = Payload(&payload);
    if payload.take(MAGIC.len())? != MAGIC {
        return Err(FrameError::Malformed(
            "the first message is not a node's hello".to_owned(),
        ));
    }
    let versions = (payload.u16()?, payload.u16()?);
    let accepts = payload.capabilities()?;
    payload.end()?;
    if versions.0 > versions.1 {
        return Err(FrameError::Malformed(format!(
            "the versions {}..{} are empty",
            versions.0, versions.1
        )));
    }
    Ok(Hello { versions, accepts })
}

/// Reads one message from a node after its hello.
pub fn read_client(reader: &mut impl Read) -> Result<ClientMessage, FrameError> {
    let (kind, payload) = read_frame(reader, MAX_INPUT + 1)?;
    let mut payload = Payload(&payload);
    let message = match kind {
        kind::INPUT => {
            let bytes = payload.0.to_vec();
            payload.0 = &[];
            ClientMessage::Input(bytes)
        }
        kind::RESIZE => ClientMessage::Resize(payload.size()?),
        kind::RELEASE => ClientMessage::Release,
        kind::TERMINATE => ClientMessage::Terminate,
        kind::HELLO => {
            return Err(FrameError::Malformed(
                "a second hello on one connection".to_owned(),
            ));
        }
        other => {
            return Err(FrameError::Malformed(format!(
                "a message of kind {other:#04x} a node may not send"
            )));
        }
    };
    payload.end()?;
    Ok(message)
}

/// Writes one message from a holder.
pub fn write_holder(writer: &mut impl Write, message: &HolderMessage) -> io::Result<()> {
    match message {
        HolderMessage::Welcome(welcome) => {
            let mut payload = welcome.version.to_le_bytes().to_vec();
            payload.extend_from_slice(&welcome.generation.to_le_bytes());
            payload.extend_from_slice(&welcome.program.to_le_bytes());
            payload.extend_from_slice(&welcome.size.rows.to_le_bytes());
            payload.extend_from_slice(&welcome.size.cols.to_le_bytes());
            put_capabilities(&mut payload, &welcome.sends);
            write_frame(writer, kind::WELCOME, &payload)
        }
        HolderMessage::Output(bytes) => write_frame(writer, kind::OUTPUT, bytes),
        HolderMessage::Closed(closed) => {
            let mut payload = closed.reason.code().to_le_bytes().to_vec();
            payload.extend_from_slice(&closed.version.to_le_bytes());
            match closed.exit_code {
                Some(code) => {
                    payload.push(1);
                    payload.extend_from_slice(&code.to_le_bytes());
                }
                None => payload.push(0),
            }
            let left = &closed.left[..closed.left.len().min(MAX_LEFT)];
            payload.extend_from_slice(&(left.len() as u16).to_le_bytes());
            for pid in left {
                payload.extend_from_slice(&pid.to_le_bytes());
            }
            let mut detail = closed.detail.clone();
            truncate(&mut detail, MAX_DETAIL);
            put_text(&mut payload, &detail);
            write_frame(writer, kind::CLOSED, &payload)
        }
    }
}

/// Reads one message from a holder.
pub fn read_holder(reader: &mut impl Read) -> Result<HolderMessage, FrameError> {
    let (kind, payload) = read_frame(reader, MAX_FRAME)?;
    let mut payload = Payload(&payload);
    let message = match kind {
        kind::WELCOME => {
            let version = payload.u16()?;
            let generation = payload.u64()?;
            let program = payload.u32()?;
            let size = payload.size()?;
            let sends = payload.capabilities()?;
            HolderMessage::Welcome(Welcome {
                version,
                generation,
                program,
                size,
                sends,
            })
        }
        kind::OUTPUT => {
            let bytes = payload.0.to_vec();
            payload.0 = &[];
            HolderMessage::Output(bytes)
        }
        kind::CLOSED => {
            let code = payload.u16()?;
            let reason = CloseReason::from_code(code)
                .ok_or_else(|| FrameError::Malformed(format!("an unknown close code {code}")))?;
            let version = payload.u16()?;
            let exit_code = match payload.u8()? {
                0 => None,
                _ => Some(payload.u32()? as i32),
            };
            let count = payload.u16()? as usize;
            if count > MAX_LEFT {
                return Err(FrameError::Malformed(format!(
                    "{count} pids are past the bound"
                )));
            }
            let left = (0..count)
                .map(|_| payload.u32())
                .collect::<Result<_, _>>()?;
            let detail = payload.text(MAX_DETAIL)?;
            HolderMessage::Closed(Closed {
                reason,
                version,
                exit_code,
                left,
                detail,
            })
        }
        other => {
            return Err(FrameError::Malformed(format!(
                "a message of kind {other:#04x} a holder may not send"
            )));
        }
    };
    payload.end()?;
    Ok(message)
}

/// What a holder answers a hello with: the version both speak and what it
/// will send, or the [`Closed`] that refuses it. `holder_sends` is what this
/// holder can send; only what the node accepts of it is sent.
pub fn agree(hello: &Hello, holder_sends: &[Capability]) -> Result<(u16, Vec<Capability>), Closed> {
    let (oldest, newest) = hello.versions;
    if !(oldest..=newest).contains(&PROTOCOL_VERSION) {
        return Err(Closed::new(
            CloseReason::UnsupportedVersion,
            format!("the node speaks {oldest}..{newest}, this holder {PROTOCOL_VERSION}"),
        ));
    }
    let sends = holder_sends
        .iter()
        .filter(|capability| hello.accepts.contains(capability))
        .cloned()
        .collect();
    Ok((PROTOCOL_VERSION, sends))
}

/// How a node's opening of a holder ended without a [`Welcome`].
#[derive(Debug)]
pub enum OpenError {
    /// The stream could not be reached or failed.
    Io(io::Error),
    /// The holder answered something that is not this protocol.
    Protocol(FrameError),
    /// The holder refused the connection.
    Refused(Closed),
    /// The holder speaks a version older than this build speaks to; it was
    /// left running.
    Unsupported { version: u16 },
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "the holder could not be reached: {error}"),
            Self::Protocol(error) => write!(f, "the holder answered out of protocol: {error}"),
            Self::Refused(closed) => write!(
                f,
                "the holder refused the connection: {} ({})",
                closed.reason.name(),
                closed.detail
            ),
            Self::Unsupported { version } => {
                write!(
                    f,
                    "the holder speaks version {version}, older than {OLDEST_HOLDER_VERSION}"
                )
            }
        }
    }
}

impl std::error::Error for OpenError {}

/// Opens a connection to the holder at `path`: connects, says [`Hello`]
/// with `accepts`, and reads the holder's answer, all by `deadline`. The
/// stream it answers has no deadline of its own.
pub fn open(
    path: &std::path::Path,
    accepts: Vec<Capability>,
    deadline: std::time::Instant,
) -> Result<(hide_platform::ipc::LocalStream, Welcome), OpenError> {
    let stream =
        hide_platform::ipc::LocalStream::connect_until(path, deadline).map_err(OpenError::Io)?;
    let mut bounded = stream.duplicate();
    bounded.set_deadline(deadline);
    write_client(&mut bounded, &ClientMessage::Hello(Hello::current(accepts)))
        .map_err(OpenError::Io)?;
    match read_holder(&mut bounded) {
        Ok(HolderMessage::Welcome(welcome)) if welcome.version < OLDEST_HOLDER_VERSION => {
            Err(OpenError::Unsupported {
                version: welcome.version,
            })
        }
        Ok(HolderMessage::Welcome(welcome)) => Ok((stream, welcome)),
        Ok(HolderMessage::Closed(closed))
            if closed.reason == CloseReason::UnsupportedVersion
                && closed.version < OLDEST_HOLDER_VERSION =>
        {
            Err(OpenError::Unsupported {
                version: closed.version,
            })
        }
        Ok(HolderMessage::Closed(closed)) => Err(OpenError::Refused(closed)),
        Ok(HolderMessage::Output(_)) => Err(OpenError::Protocol(FrameError::Malformed(
            "output before the welcome".to_owned(),
        ))),
        Err(FrameError::Io(error)) => Err(OpenError::Io(error)),
        Err(error) => Err(OpenError::Protocol(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_client(message: ClientMessage) -> ClientMessage {
        let mut bytes = Vec::new();
        write_client(&mut bytes, &message).unwrap();
        match &message {
            ClientMessage::Hello(_) => ClientMessage::Hello(read_hello(&mut &bytes[..]).unwrap()),
            _ => read_client(&mut &bytes[..]).unwrap(),
        }
    }

    fn round_holder(message: HolderMessage) -> HolderMessage {
        let mut bytes = Vec::new();
        write_holder(&mut bytes, &message).unwrap();
        read_holder(&mut &bytes[..]).unwrap()
    }

    #[test]
    fn every_message_reads_back_as_written() {
        for message in [
            ClientMessage::Hello(Hello::current(vec![
                Capability::RawOutput,
                Capability::Other("later".to_owned()),
            ])),
            ClientMessage::Input(b"echo \x1b[A\r".to_vec()),
            ClientMessage::Resize(Size {
                rows: 40,
                cols: 120,
            }),
            ClientMessage::Release,
            ClientMessage::Terminate,
        ] {
            assert_eq!(round_client(message.clone()), message);
        }
        let mut closed = Closed::new(CloseReason::TreeEndUnconfirmed, "two left");
        closed.exit_code = Some(-3);
        closed.left = vec![41, 42];
        for message in [
            HolderMessage::Welcome(Welcome {
                version: 1,
                generation: 7,
                program: 1234,
                size: Size { rows: 24, cols: 80 },
                sends: vec![Capability::RawOutput],
            }),
            HolderMessage::Output(vec![0, 255, 27]),
            HolderMessage::Closed(closed),
        ] {
            assert_eq!(round_holder(message.clone()), message);
        }
    }

    #[test]
    fn a_close_code_keeps_its_meaning() {
        for reason in CloseReason::ALL {
            assert_eq!(CloseReason::from_code(reason.code()), Some(reason));
        }
        assert_eq!(CloseReason::TakenOver.code(), 4);
        assert_eq!(CloseReason::TreeEndUnconfirmed.code(), 13);
        assert_eq!(CloseReason::from_code(0), None);
    }

    #[test]
    fn a_hello_past_its_bound_or_not_a_hello_is_refused() {
        let mut long = Vec::new();
        long.extend_from_slice(&((MAX_HELLO + 1) as u32).to_le_bytes());
        long.push(kind::HELLO);
        long.extend(std::iter::repeat_n(0u8, MAX_HELLO));
        assert!(matches!(
            read_hello(&mut &long[..]),
            Err(FrameError::TooLarge(_))
        ));

        let mut input = Vec::new();
        write_client(&mut input, &ClientMessage::Input(b"x".to_vec())).unwrap();
        assert!(matches!(
            read_hello(&mut &input[..]),
            Err(FrameError::Malformed(_))
        ));

        let mut not_ours = Vec::new();
        write_frame(&mut not_ours, kind::HELLO, b"GET / HTTP/1.1").unwrap();
        assert!(matches!(
            read_hello(&mut &not_ours[..]),
            Err(FrameError::Malformed(_))
        ));
    }

    #[test]
    fn a_holder_agrees_only_within_the_nodes_versions_and_sends_what_it_accepts() {
        let hello = Hello {
            versions: (1, 3),
            accepts: vec![Capability::Other("x".to_owned())],
        };
        assert_eq!(
            agree(&hello, &[Capability::RawOutput]),
            Ok((PROTOCOL_VERSION, Vec::new()))
        );
        let newer = Hello {
            versions: (PROTOCOL_VERSION + 1, PROTOCOL_VERSION + 2),
            accepts: vec![Capability::RawOutput],
        };
        let refused = agree(&newer, &[Capability::RawOutput]).unwrap_err();
        assert_eq!(refused.reason, CloseReason::UnsupportedVersion);
        assert_eq!(refused.version, PROTOCOL_VERSION);
    }

    #[test]
    fn a_node_message_a_node_may_not_send_is_refused() {
        let mut bytes = Vec::new();
        write_holder(&mut bytes, &HolderMessage::Output(b"x".to_vec())).unwrap();
        assert!(matches!(
            read_client(&mut &bytes[..]),
            Err(FrameError::Malformed(_))
        ));
        let mut hello = Vec::new();
        write_client(
            &mut hello,
            &ClientMessage::Hello(Hello::current(Vec::new())),
        )
        .unwrap();
        assert!(matches!(
            read_client(&mut &hello[..]),
            Err(FrameError::Malformed(_))
        ));
    }

    #[test]
    fn sizes_outside_the_bounds_are_refused() {
        assert!(Size::checked(24, 80).is_ok());
        assert!(Size::checked(MAX_ROWS, 256).is_ok());
        assert!(Size::checked(0, 80).is_err());
        assert!(Size::checked(MAX_ROWS + 1, 1).is_err());
        assert!(Size::checked(MAX_ROWS, MAX_COLS).is_err());
    }
}
