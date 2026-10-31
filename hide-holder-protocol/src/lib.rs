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
//! In version 1 a node sends input, a size, a theme, a redraw, a release and
//! a terminate, and the holder sends the screen as [`Fragment`]s and why it
//! closed. Releasing lets go of the connection and leaves the pane running;
//! terminating ends the pane's program and everything it started, then the
//! holder.
//!
//! Input comes in two kinds (PRD engine-bundle-2-pane-holder D-8): bytes
//! already encoded ([`ClientMessage::Input`]), written as they are, and
//! requests that mean something ([`ClientMessage::Paste`],
//! [`ClientMessage::Mouse`], [`ClientMessage::Scroll`]), which the holder
//! encodes for the pane's settings at that moment. Nothing guesses from
//! bytes what they were.
//!
//! A fragment is display-only ANSI (D-7): it never turns on mouse reports,
//! bracketed paste, a keyboard protocol or focus reports, and never asks the
//! receiver a question, so a terminal that draws it answers nothing. A full
//! fragment starts a new epoch at sequence 0 and gives the same screen
//! whatever the receiver drew before; each fragment after it in the epoch is
//! one more in sequence and applies only on the one before it. A receiver
//! that misses one asks [`ClientMessage::Redraw`], and the next is full.
//!
//! A connection whose first message is an [`Ask`] instead of a hello asks one
//! question, the pane's recent text or its processes, and is answered once
//! and closed; it never takes the holder over and leaves the screen
//! connection and its scroll position as they were (D-11, D-18).

use std::io::{self, Read, Write};
use std::time::Duration;

/// The version of this protocol this build speaks as a holder.
pub const PROTOCOL_VERSION: u16 = 1;

/// The oldest holder version this build speaks to as a node. A holder older
/// than this is reported as unsupported and left running, never ended.
pub const OLDEST_HOLDER_VERSION: u16 = 1;

/// The most bytes one [`Fragment`]'s ANSI carries: what a node can draw at
/// once. A screen that encodes to more is sent as several fragments, split
/// between rows, so no screen is ever refused for its size.
pub const MAX_FRAGMENT: usize = 4 * 1024 * 1024;

/// The most bytes one frame carries, kind byte included: a fragment and its
/// header.
pub const MAX_FRAME: usize = MAX_FRAGMENT + 1024;

/// The most bytes a [`Hello`] frame carries.
pub const MAX_HELLO: usize = 4 * 1024;

/// How long a holder waits for a connection's [`Hello`].
pub const HELLO_LIMIT: Duration = Duration::from_secs(2);

/// The most input bytes one [`ClientMessage::Input`] carries.
pub const MAX_INPUT: usize = 64 * 1024;

/// The most bytes one [`ClientMessage::Paste`] carries.
pub const MAX_PASTE: usize = 512 * 1024;

/// The most bytes a node's message after its hello carries.
const MAX_CLIENT: usize = MAX_PASTE + 64;

/// The most rows one [`Ask::Read`] asks for.
pub const MAX_READ_LINES: u16 = 2000;

/// The most text bytes one [`Answer::Text`] carries; older text past it is
/// left out and the answer is marked truncated.
pub const MAX_READ_BYTES: usize = 1024 * 1024;

/// The most processes one [`Answer::Processes`] names, and the most bytes of
/// one process's name.
pub const MAX_PROCESSES: usize = 256;
pub const MAX_PROCESS_NAME: usize = 256;

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
    // The screen and meaning input: 0x10..=0x1f from a node, 0x90..=0x9f from
    // a holder.
    pub const REDRAW: u8 = 0x10;
    pub const PASTE: u8 = 0x11;
    pub const MOUSE: u8 = 0x12;
    pub const SCROLL: u8 = 0x13;
    pub const THEME: u8 = 0x14;
    /// A first message, like [`HELLO`].
    pub const ASK: u8 = 0x18;
    pub const WELCOME: u8 = 0x81;
    pub const CLOSED: u8 = 0x83;
    pub const FRAGMENT: u8 = 0x90;
    pub const ANSWER: u8 = 0x91;
}

/// Something one side can do that the other must accept before it is used.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    /// The holder sends the screen as [`Fragment`]s.
    Screen,
    /// A capability this build does not know, kept by name so a side can
    /// answer with the ones it does.
    Other(String),
}

impl Capability {
    fn name(&self) -> &str {
        match self {
            Self::Screen => "screen",
            Self::Other(name) => name,
        }
    }

    fn from_name(name: &str) -> Self {
        match name {
            "screen" => Self::Screen,
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
    /// An [`Ask`] outside its bounds, such as more rows than
    /// [`MAX_READ_LINES`].
    AskInvalid,
    /// The holder could not answer an [`Ask`]; the detail says why.
    AskFailed,
}

impl CloseReason {
    const ALL: [Self; 18] = [
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
        Self::SizeInvalid,
        Self::TreeEndUnconfirmed,
        Self::HolderFailed,
        Self::InputFailed,
        Self::ResizeFailed,
        Self::OpeningFull,
        Self::AskInvalid,
        Self::AskFailed,
    ];

    /// The code on the wire, which never changes meaning. 11 is retired
    /// (the raw output queue it named is gone) and never reused; the screen
    /// and its questions number from 40.
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
            Self::SizeInvalid => 12,
            Self::TreeEndUnconfirmed => 13,
            Self::HolderFailed => 14,
            Self::InputFailed => 15,
            Self::ResizeFailed => 16,
            Self::OpeningFull => 17,
            Self::AskInvalid => 40,
            Self::AskFailed => 41,
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
            Self::SizeInvalid => "size_invalid",
            Self::TreeEndUnconfirmed => "tree_end_unconfirmed",
            Self::HolderFailed => "holder_failed",
            Self::InputFailed => "input_failed",
            Self::ResizeFailed => "resize_failed",
            Self::OpeningFull => "opening_full",
            Self::AskInvalid => "ask_invalid",
            Self::AskFailed => "ask_failed",
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

/// A color as red, green and blue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Whether the node's theme is dark or light, which a program can ask
/// (`CSI ? 996 n`) and be told of when it changes (mode 2031).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Unknown,
    Dark,
    Light,
}

/// The colors the node draws the pane with. The holder answers the
/// program's color questions with them until the program sets its own, and
/// again once it resets them (PRD engine-bundle-2-pane-holder D-9). A node
/// sends one after its hello and again whenever its theme changes; a color
/// left `None` is one the node does not know, and a question about it goes
/// unanswered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub cursor: Option<Rgb>,
    /// Palette entries the theme sets; the rest keep the xterm defaults.
    pub palette: Vec<(u8, Rgb)>,
    pub appearance: Appearance,
}

/// Which keys were held, as one bit each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Modifiers(pub u8);

impl Modifiers {
    pub const SHIFT: u8 = 1;
    pub const CTRL: u8 = 2;
    pub const ALT: u8 = 4;
    pub const SUPER: u8 = 8;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// A pointer event over a cell, zero-based. The holder reports it in the
/// pane's mouse mode and format at that moment, or drops it when the
/// program does not track the mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mouse {
    pub action: MouseAction,
    /// `None` for motion with no button held.
    pub button: Option<MouseButton>,
    pub col: u16,
    pub row: u16,
    pub modifiers: Modifiers,
}

/// A wheel turned by `lines`, positive up, over a cell when the node knows
/// which. The holder reports it to a program that tracks the mouse, turns it
/// into arrow keys on the alternate screen with mode 1007, or else moves the
/// screen through its scrollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scroll {
    pub lines: i16,
    pub cell: Option<(u16, u16)>,
    pub modifiers: Modifiers,
}

/// A piece of the screen as display-only ANSI. See the crate documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fragment {
    /// One more for each full fragment, across every connection the holder
    /// serves.
    pub epoch: u64,
    /// 0 for the full fragment that starts the epoch, one more for each
    /// fragment after it.
    pub seq: u64,
    /// Whether this fragment gives the whole screen whatever the receiver
    /// drew before. A full screen that encodes to more than [`MAX_FRAGMENT`]
    /// goes on in the fragments after it, which are not full.
    pub full: bool,
    /// The screen's size when it was drawn.
    pub size: Size,
    pub bytes: Vec<u8>,
}

/// A connection's only question, in place of a hello.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// The last `lines` rows of the screen and its scrollback, as text with
    /// soft wraps joined, at most [`MAX_READ_LINES`].
    Read { lines: u16 },
    /// The pane's shell and the processes in its terminal's foreground.
    Processes,
}

/// Why a holder does not know a pane's foreground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unknown {
    /// This system has no foreground process group for a terminal
    /// (Windows).
    NotOnThisSystem,
    /// The terminal has no foreground group now.
    NoGroup,
    /// The system did not say.
    Unreadable,
}

/// A process in a pane's foreground.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub pid: u32,
    /// `None` when the system did not say.
    pub name: Option<String>,
}

/// What runs in a pane's terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Foreground {
    /// The terminal's foreground process group and its processes.
    Known {
        group: u32,
        members: Vec<Member>,
    },
    Unknown(Unknown),
}

/// The answer to an [`Ask`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Text {
        text: String,
        /// Whether rows the ask reached were left out: more than it asked
        /// for exist, or the text passed [`MAX_READ_BYTES`].
        truncated: bool,
    },
    Processes {
        /// The pane's program, the one the holder started.
        shell: u32,
        foreground: Foreground,
    },
}

/// What a node sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    /// Bytes for the pane's program, written as they are.
    Input(Vec<u8>),
    /// Pasted text, wrapped for bracketed paste only when the program turned
    /// it on.
    Paste(Vec<u8>),
    Mouse(Mouse),
    Scroll(Scroll),
    Resize(Size),
    Theme(Theme),
    /// The next fragment is full: the node lost track of the screen.
    Redraw,
    /// Lets go of the connection; the pane runs on.
    Release,
    /// Ends the pane's program and everything it started, then the holder.
    Terminate,
}

/// A connection's first message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opening {
    Hello(Hello),
    Ask(Ask),
}

/// What a holder sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HolderMessage {
    Welcome(Welcome),
    Fragment(Fragment),
    Answer(Answer),
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

    fn rest_bounded(&mut self, what: &str, bound: usize) -> Result<Vec<u8>, FrameError> {
        if self.0.len() > bound {
            return Err(FrameError::Malformed(format!(
                "{} {what} bytes are past the bound of {bound}",
                self.0.len()
            )));
        }
        let rest = self.0.to_vec();
        self.0 = &[];
        Ok(rest)
    }

    fn rgb(&mut self) -> Result<Option<Rgb>, FrameError> {
        let color = self.take(4)?;
        Ok((color[0] != 0).then_some(Rgb {
            r: color[1],
            g: color[2],
            b: color[3],
        }))
    }

    fn cell(&mut self) -> Result<Option<(u16, u16)>, FrameError> {
        let known = self.u8()? != 0;
        let col = self.u16()?;
        let row = self.u16()?;
        Ok(known.then_some((col, row)))
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

fn put_rgb(payload: &mut Vec<u8>, color: Option<Rgb>) {
    match color {
        Some(Rgb { r, g, b }) => payload.extend_from_slice(&[1, r, g, b]),
        None => payload.extend_from_slice(&[0, 0, 0, 0]),
    }
}

fn put_cell(payload: &mut Vec<u8>, cell: Option<(u16, u16)>) {
    match cell {
        Some((col, row)) => {
            payload.push(1);
            payload.extend_from_slice(&col.to_le_bytes());
            payload.extend_from_slice(&row.to_le_bytes());
        }
        None => payload.extend_from_slice(&[0; 5]),
    }
}

fn too_long(what: &str, length: usize, bound: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{length} {what} bytes are past the bound of {bound}"),
    )
}

/// Writes one message from a node.
pub fn write_client(writer: &mut impl Write, message: &ClientMessage) -> io::Result<()> {
    match message {
        ClientMessage::Input(bytes) => {
            if bytes.len() > MAX_INPUT {
                return Err(too_long("input", bytes.len(), MAX_INPUT));
            }
            write_frame(writer, kind::INPUT, bytes)
        }
        ClientMessage::Paste(bytes) => {
            if bytes.len() > MAX_PASTE {
                return Err(too_long("paste", bytes.len(), MAX_PASTE));
            }
            write_frame(writer, kind::PASTE, bytes)
        }
        ClientMessage::Mouse(mouse) => {
            let action = match mouse.action {
                MouseAction::Press => 0,
                MouseAction::Release => 1,
                MouseAction::Motion => 2,
            };
            let button = match mouse.button {
                None => 0,
                Some(MouseButton::Left) => 1,
                Some(MouseButton::Middle) => 2,
                Some(MouseButton::Right) => 3,
            };
            let mut payload = vec![action, button];
            payload.extend_from_slice(&mouse.col.to_le_bytes());
            payload.extend_from_slice(&mouse.row.to_le_bytes());
            payload.push(mouse.modifiers.0);
            write_frame(writer, kind::MOUSE, &payload)
        }
        ClientMessage::Scroll(scroll) => {
            let mut payload = scroll.lines.to_le_bytes().to_vec();
            put_cell(&mut payload, scroll.cell);
            payload.push(scroll.modifiers.0);
            write_frame(writer, kind::SCROLL, &payload)
        }
        ClientMessage::Resize(size) => {
            let mut payload = size.rows.to_le_bytes().to_vec();
            payload.extend_from_slice(&size.cols.to_le_bytes());
            write_frame(writer, kind::RESIZE, &payload)
        }
        ClientMessage::Theme(theme) => {
            let mut payload = Vec::with_capacity(16 + theme.palette.len() * 4);
            put_rgb(&mut payload, theme.foreground);
            put_rgb(&mut payload, theme.background);
            put_rgb(&mut payload, theme.cursor);
            payload.push(match theme.appearance {
                Appearance::Unknown => 0,
                Appearance::Dark => 1,
                Appearance::Light => 2,
            });
            let palette = &theme.palette[..theme.palette.len().min(256)];
            payload.extend_from_slice(&(palette.len() as u16).to_le_bytes());
            for (index, color) in palette {
                payload.extend_from_slice(&[*index, color.r, color.g, color.b]);
            }
            write_frame(writer, kind::THEME, &payload)
        }
        ClientMessage::Redraw => write_frame(writer, kind::REDRAW, &[]),
        ClientMessage::Release => write_frame(writer, kind::RELEASE, &[]),
        ClientMessage::Terminate => write_frame(writer, kind::TERMINATE, &[]),
    }
}

/// Writes a connection's first message.
pub fn write_opening(writer: &mut impl Write, opening: &Opening) -> io::Result<()> {
    match opening {
        Opening::Hello(hello) => {
            let mut payload = MAGIC.to_vec();
            payload.extend_from_slice(&hello.versions.0.to_le_bytes());
            payload.extend_from_slice(&hello.versions.1.to_le_bytes());
            put_capabilities(&mut payload, &hello.accepts);
            write_frame(writer, kind::HELLO, &payload)
        }
        Opening::Ask(ask) => {
            let mut payload = MAGIC.to_vec();
            payload.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
            match ask {
                Ask::Read { lines } => {
                    payload.push(1);
                    payload.extend_from_slice(&lines.to_le_bytes());
                }
                Ask::Processes => payload.push(2),
            }
            write_frame(writer, kind::ASK, &payload)
        }
    }
}

/// Reads a connection's first message, a [`Hello`] or an [`Ask`] within
/// [`MAX_HELLO`] bytes. The caller bounds the time. An ask is read whatever
/// it asks; [`Ask::check`] says whether the holder answers it.
pub fn read_opening(reader: &mut impl Read) -> Result<Opening, FrameError> {
    let (kind, payload) = read_frame(reader, MAX_HELLO)?;
    if kind != kind::HELLO && kind != kind::ASK {
        return Err(FrameError::Malformed(format!(
            "the first message is of kind {kind:#04x}, not a hello or an ask"
        )));
    }
    let mut payload = Payload(&payload);
    if payload.take(MAGIC.len())? != MAGIC {
        return Err(FrameError::Malformed(
            "the first message is not a node's".to_owned(),
        ));
    }
    let opening = if kind == kind::HELLO {
        let versions = (payload.u16()?, payload.u16()?);
        let accepts = payload.capabilities()?;
        if versions.0 > versions.1 {
            return Err(FrameError::Malformed(format!(
                "the versions {}..{} are empty",
                versions.0, versions.1
            )));
        }
        Opening::Hello(Hello { versions, accepts })
    } else {
        let version = payload.u16()?;
        if version != PROTOCOL_VERSION {
            return Err(FrameError::Malformed(format!(
                "an ask in version {version}, not {PROTOCOL_VERSION}"
            )));
        }
        match payload.u8()? {
            1 => Opening::Ask(Ask::Read {
                lines: payload.u16()?,
            }),
            2 => Opening::Ask(Ask::Processes),
            other => {
                return Err(FrameError::Malformed(format!("an unknown ask {other}")));
            }
        }
    };
    payload.end()?;
    Ok(opening)
}

impl Ask {
    /// Whether the holder answers this ask, or the [`Closed`] that refuses
    /// it.
    pub fn check(&self) -> Result<(), Closed> {
        match self {
            Self::Read { lines } if *lines == 0 || *lines > MAX_READ_LINES => Err(Closed::new(
                CloseReason::AskInvalid,
                format!("{lines} rows asked for, not 1 to {MAX_READ_LINES}"),
            )),
            _ => Ok(()),
        }
    }
}

/// Reads one message from a node after its hello.
pub fn read_client(reader: &mut impl Read) -> Result<ClientMessage, FrameError> {
    let (kind, payload) = read_frame(reader, MAX_CLIENT)?;
    let mut payload = Payload(&payload);
    let message = match kind {
        kind::INPUT => ClientMessage::Input(payload.rest_bounded("input", MAX_INPUT)?),
        kind::PASTE => ClientMessage::Paste(payload.rest_bounded("paste", MAX_PASTE)?),
        kind::MOUSE => {
            let action = match payload.u8()? {
                0 => MouseAction::Press,
                1 => MouseAction::Release,
                2 => MouseAction::Motion,
                other => {
                    return Err(FrameError::Malformed(format!(
                        "an unknown mouse action {other}"
                    )));
                }
            };
            let button = match payload.u8()? {
                0 => None,
                1 => Some(MouseButton::Left),
                2 => Some(MouseButton::Middle),
                3 => Some(MouseButton::Right),
                other => {
                    return Err(FrameError::Malformed(format!(
                        "an unknown mouse button {other}"
                    )));
                }
            };
            ClientMessage::Mouse(Mouse {
                action,
                button,
                col: payload.u16()?,
                row: payload.u16()?,
                modifiers: Modifiers(payload.u8()?),
            })
        }
        kind::SCROLL => ClientMessage::Scroll(Scroll {
            lines: payload.u16()? as i16,
            cell: payload.cell()?,
            modifiers: Modifiers(payload.u8()?),
        }),
        kind::RESIZE => ClientMessage::Resize(payload.size()?),
        kind::THEME => {
            let foreground = payload.rgb()?;
            let background = payload.rgb()?;
            let cursor = payload.rgb()?;
            let appearance = match payload.u8()? {
                0 => Appearance::Unknown,
                1 => Appearance::Dark,
                2 => Appearance::Light,
                other => {
                    return Err(FrameError::Malformed(format!(
                        "an unknown appearance {other}"
                    )));
                }
            };
            let count = payload.u16()? as usize;
            if count > 256 {
                return Err(FrameError::Malformed(format!(
                    "{count} palette entries are past the bound"
                )));
            }
            let palette = (0..count)
                .map(|_| {
                    let entry = payload.take(4)?;
                    Ok((
                        entry[0],
                        Rgb {
                            r: entry[1],
                            g: entry[2],
                            b: entry[3],
                        },
                    ))
                })
                .collect::<Result<_, FrameError>>()?;
            ClientMessage::Theme(Theme {
                foreground,
                background,
                cursor,
                palette,
                appearance,
            })
        }
        kind::REDRAW => ClientMessage::Redraw,
        kind::RELEASE => ClientMessage::Release,
        kind::TERMINATE => ClientMessage::Terminate,
        kind::HELLO | kind::ASK => {
            return Err(FrameError::Malformed(
                "a second first message on one connection".to_owned(),
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
        HolderMessage::Fragment(fragment) => {
            if fragment.bytes.len() > MAX_FRAGMENT {
                return Err(too_long("fragment", fragment.bytes.len(), MAX_FRAGMENT));
            }
            let mut payload = Vec::with_capacity(fragment.bytes.len() + 24);
            payload.extend_from_slice(&fragment.epoch.to_le_bytes());
            payload.extend_from_slice(&fragment.seq.to_le_bytes());
            payload.push(u8::from(fragment.full));
            payload.extend_from_slice(&fragment.size.rows.to_le_bytes());
            payload.extend_from_slice(&fragment.size.cols.to_le_bytes());
            payload.extend_from_slice(&fragment.bytes);
            write_frame(writer, kind::FRAGMENT, &payload)
        }
        HolderMessage::Answer(answer) => {
            let mut payload = Vec::new();
            match answer {
                Answer::Text { text, truncated } => {
                    if text.len() > MAX_READ_BYTES {
                        return Err(too_long("text", text.len(), MAX_READ_BYTES));
                    }
                    payload.push(1);
                    payload.push(u8::from(*truncated));
                    payload.extend_from_slice(&(text.len() as u32).to_le_bytes());
                    payload.extend_from_slice(text.as_bytes());
                }
                Answer::Processes { shell, foreground } => {
                    payload.push(2);
                    payload.extend_from_slice(&shell.to_le_bytes());
                    match foreground {
                        Foreground::Known { group, members } => {
                            payload.push(0);
                            payload.extend_from_slice(&group.to_le_bytes());
                            let members = &members[..members.len().min(MAX_PROCESSES)];
                            payload.extend_from_slice(&(members.len() as u16).to_le_bytes());
                            for member in members {
                                payload.extend_from_slice(&member.pid.to_le_bytes());
                                match &member.name {
                                    Some(name) => {
                                        let mut name = name.clone();
                                        truncate(&mut name, MAX_PROCESS_NAME);
                                        payload.push(1);
                                        put_text(&mut payload, &name);
                                    }
                                    None => payload.push(0),
                                }
                            }
                        }
                        Foreground::Unknown(why) => payload.push(match why {
                            Unknown::NotOnThisSystem => 1,
                            Unknown::NoGroup => 2,
                            Unknown::Unreadable => 3,
                        }),
                    }
                }
            }
            write_frame(writer, kind::ANSWER, &payload)
        }
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
        kind::FRAGMENT => {
            let epoch = payload.u64()?;
            let seq = payload.u64()?;
            let full = payload.u8()? != 0;
            let size = payload.size()?;
            let bytes = payload.rest_bounded("fragment", MAX_FRAGMENT)?;
            HolderMessage::Fragment(Fragment {
                epoch,
                seq,
                full,
                size,
                bytes,
            })
        }
        kind::ANSWER => HolderMessage::Answer(match payload.u8()? {
            1 => {
                let truncated = payload.u8()? != 0;
                let length = payload.u32()? as usize;
                if length > MAX_READ_BYTES {
                    return Err(FrameError::Malformed(format!(
                        "a text of {length} bytes is past its bound"
                    )));
                }
                let text = String::from_utf8(payload.take(length)?.to_vec())
                    .map_err(|_| FrameError::Malformed("a text is not UTF-8".to_owned()))?;
                Answer::Text { text, truncated }
            }
            2 => {
                let shell = payload.u32()?;
                let foreground = match payload.u8()? {
                    0 => {
                        let group = payload.u32()?;
                        let count = payload.u16()? as usize;
                        if count > MAX_PROCESSES {
                            return Err(FrameError::Malformed(format!(
                                "{count} processes are past the bound"
                            )));
                        }
                        let members = (0..count)
                            .map(|_| {
                                let pid = payload.u32()?;
                                let name = match payload.u8()? {
                                    0 => None,
                                    _ => Some(payload.text(MAX_PROCESS_NAME)?),
                                };
                                Ok(Member { pid, name })
                            })
                            .collect::<Result<_, FrameError>>()?;
                        Foreground::Known { group, members }
                    }
                    1 => Foreground::Unknown(Unknown::NotOnThisSystem),
                    2 => Foreground::Unknown(Unknown::NoGroup),
                    3 => Foreground::Unknown(Unknown::Unreadable),
                    other => {
                        return Err(FrameError::Malformed(format!(
                            "an unknown foreground {other}"
                        )));
                    }
                };
                Answer::Processes { shell, foreground }
            }
            other => {
                return Err(FrameError::Malformed(format!("an unknown answer {other}")));
            }
        }),
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

/// How a node's opening of a holder ended without a [`Welcome`] or an
/// [`Answer`].
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

impl From<FrameError> for OpenError {
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::Io(error) => Self::Io(error),
            error => Self::Protocol(error),
        }
    }
}

fn refused(closed: Closed) -> OpenError {
    if closed.reason == CloseReason::UnsupportedVersion && closed.version < OLDEST_HOLDER_VERSION {
        OpenError::Unsupported {
            version: closed.version,
        }
    } else {
        OpenError::Refused(closed)
    }
}

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
    write_opening(&mut bounded, &Opening::Hello(Hello::current(accepts))).map_err(OpenError::Io)?;
    match read_holder(&mut bounded)? {
        HolderMessage::Welcome(welcome) if welcome.version < OLDEST_HOLDER_VERSION => {
            Err(OpenError::Unsupported {
                version: welcome.version,
            })
        }
        HolderMessage::Welcome(welcome) => Ok((stream, welcome)),
        HolderMessage::Closed(closed) => Err(refused(closed)),
        HolderMessage::Fragment(_) | HolderMessage::Answer(_) => Err(OpenError::Protocol(
            FrameError::Malformed("a screen or an answer before the welcome".to_owned()),
        )),
    }
}

/// Asks the holder at `path` one question on a connection of its own, by
/// `deadline`. The connection that holds the pane, if any, goes on as it was.
pub fn ask(
    path: &std::path::Path,
    ask: Ask,
    deadline: std::time::Instant,
) -> Result<Answer, OpenError> {
    let mut stream =
        hide_platform::ipc::LocalStream::connect_until(path, deadline).map_err(OpenError::Io)?;
    stream.set_deadline(deadline);
    write_opening(&mut stream, &Opening::Ask(ask)).map_err(OpenError::Io)?;
    match read_holder(&mut stream)? {
        HolderMessage::Answer(answer) => Ok(answer),
        HolderMessage::Closed(closed) => Err(refused(closed)),
        HolderMessage::Welcome(_) | HolderMessage::Fragment(_) => Err(OpenError::Protocol(
            FrameError::Malformed("an ask answered with a screen".to_owned()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_client(message: ClientMessage) -> ClientMessage {
        let mut bytes = Vec::new();
        write_client(&mut bytes, &message).unwrap();
        read_client(&mut &bytes[..]).unwrap()
    }

    fn round_opening(opening: Opening) -> Opening {
        let mut bytes = Vec::new();
        write_opening(&mut bytes, &opening).unwrap();
        read_opening(&mut &bytes[..]).unwrap()
    }

    fn round_holder(message: HolderMessage) -> HolderMessage {
        let mut bytes = Vec::new();
        write_holder(&mut bytes, &message).unwrap();
        read_holder(&mut &bytes[..]).unwrap()
    }

    #[test]
    fn every_message_reads_back_as_written() {
        for opening in [
            Opening::Hello(Hello::current(vec![
                Capability::Screen,
                Capability::Other("later".to_owned()),
            ])),
            Opening::Ask(Ask::Read { lines: 2000 }),
            Opening::Ask(Ask::Processes),
        ] {
            assert_eq!(round_opening(opening.clone()), opening);
        }
        for message in [
            ClientMessage::Input(b"echo \x1b[A\r".to_vec()),
            ClientMessage::Paste(b"two\nlines\x1b".to_vec()),
            ClientMessage::Mouse(Mouse {
                action: MouseAction::Press,
                button: Some(MouseButton::Right),
                col: 3,
                row: 7,
                modifiers: Modifiers(Modifiers::CTRL | Modifiers::SHIFT),
            }),
            ClientMessage::Mouse(Mouse {
                action: MouseAction::Motion,
                button: None,
                col: 0,
                row: 0,
                modifiers: Modifiers::default(),
            }),
            ClientMessage::Scroll(Scroll {
                lines: -3,
                cell: Some((24, 12)),
                modifiers: Modifiers(Modifiers::ALT),
            }),
            ClientMessage::Scroll(Scroll {
                lines: 5,
                cell: None,
                modifiers: Modifiers::default(),
            }),
            ClientMessage::Resize(Size {
                rows: 40,
                cols: 120,
            }),
            ClientMessage::Theme(Theme {
                foreground: Some(Rgb { r: 1, g: 2, b: 3 }),
                background: None,
                cursor: Some(Rgb { r: 9, g: 8, b: 7 }),
                palette: vec![(0, Rgb::default()), (255, Rgb { r: 255, g: 0, b: 1 })],
                appearance: Appearance::Light,
            }),
            ClientMessage::Redraw,
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
                sends: vec![Capability::Screen],
            }),
            HolderMessage::Fragment(Fragment {
                epoch: 3,
                seq: 9,
                full: true,
                size: Size { rows: 2, cols: 4 },
                bytes: vec![0, 255, 27],
            }),
            HolderMessage::Answer(Answer::Text {
                text: "a\nb".to_owned(),
                truncated: true,
            }),
            HolderMessage::Answer(Answer::Processes {
                shell: 10,
                foreground: Foreground::Known {
                    group: 11,
                    members: vec![
                        Member {
                            pid: 11,
                            name: Some("vim".to_owned()),
                        },
                        Member {
                            pid: 12,
                            name: None,
                        },
                    ],
                },
            }),
            HolderMessage::Answer(Answer::Processes {
                shell: 10,
                foreground: Foreground::Unknown(Unknown::NotOnThisSystem),
            }),
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
        assert_eq!(CloseReason::AskInvalid.code(), 40);
        assert_eq!(CloseReason::from_code(0), None);
        assert_eq!(CloseReason::from_code(11), None);
    }

    #[test]
    fn an_opening_past_its_bound_or_not_an_opening_is_refused() {
        let mut long = Vec::new();
        long.extend_from_slice(&((MAX_HELLO + 1) as u32).to_le_bytes());
        long.push(kind::HELLO);
        long.extend(std::iter::repeat_n(0u8, MAX_HELLO));
        assert!(matches!(
            read_opening(&mut &long[..]),
            Err(FrameError::TooLarge(_))
        ));

        let mut input = Vec::new();
        write_client(&mut input, &ClientMessage::Input(b"x".to_vec())).unwrap();
        assert!(matches!(
            read_opening(&mut &input[..]),
            Err(FrameError::Malformed(_))
        ));

        let mut not_ours = Vec::new();
        write_frame(&mut not_ours, kind::HELLO, b"GET / HTTP/1.1").unwrap();
        assert!(matches!(
            read_opening(&mut &not_ours[..]),
            Err(FrameError::Malformed(_))
        ));
    }

    #[test]
    fn an_ask_past_its_rows_is_refused_with_a_reason() {
        assert!(Ask::Read { lines: 1 }.check().is_ok());
        assert!(
            Ask::Read {
                lines: MAX_READ_LINES
            }
            .check()
            .is_ok()
        );
        for lines in [0, MAX_READ_LINES + 1] {
            let refused = Ask::Read { lines }.check().unwrap_err();
            assert_eq!(refused.reason, CloseReason::AskInvalid);
        }
    }

    #[test]
    fn a_message_past_its_bound_is_not_written_or_read() {
        let paste = ClientMessage::Paste(vec![b'x'; MAX_PASTE + 1]);
        assert!(write_client(&mut Vec::new(), &paste).is_err());
        let mut forged = Vec::new();
        write_frame(&mut forged, kind::INPUT, &vec![b'x'; MAX_INPUT + 1]).unwrap();
        assert!(matches!(
            read_client(&mut &forged[..]),
            Err(FrameError::Malformed(_))
        ));
        let fragment = HolderMessage::Fragment(Fragment {
            epoch: 1,
            seq: 0,
            full: true,
            size: Size { rows: 1, cols: 1 },
            bytes: vec![b'x'; MAX_FRAGMENT + 1],
        });
        assert!(write_holder(&mut Vec::new(), &fragment).is_err());
        let text = HolderMessage::Answer(Answer::Text {
            text: "x".repeat(MAX_READ_BYTES + 1),
            truncated: false,
        });
        assert!(write_holder(&mut Vec::new(), &text).is_err());
    }

    #[test]
    fn a_holder_agrees_only_within_the_nodes_versions_and_sends_what_it_accepts() {
        let hello = Hello {
            versions: (1, 3),
            accepts: vec![Capability::Other("x".to_owned())],
        };
        assert_eq!(
            agree(&hello, &[Capability::Screen]),
            Ok((PROTOCOL_VERSION, Vec::new()))
        );
        let newer = Hello {
            versions: (PROTOCOL_VERSION + 1, PROTOCOL_VERSION + 2),
            accepts: vec![Capability::Screen],
        };
        let refused = agree(&newer, &[Capability::Screen]).unwrap_err();
        assert_eq!(refused.reason, CloseReason::UnsupportedVersion);
        assert_eq!(refused.version, PROTOCOL_VERSION);
    }

    #[test]
    fn a_node_message_a_node_may_not_send_is_refused() {
        let mut bytes = Vec::new();
        write_holder(
            &mut bytes,
            &HolderMessage::Answer(Answer::Text {
                text: String::new(),
                truncated: false,
            }),
        )
        .unwrap();
        assert!(matches!(
            read_client(&mut &bytes[..]),
            Err(FrameError::Malformed(_))
        ));
        for opening in [
            Opening::Hello(Hello::current(Vec::new())),
            Opening::Ask(Ask::Processes),
        ] {
            let mut first = Vec::new();
            write_opening(&mut first, &opening).unwrap();
            assert!(matches!(
                read_client(&mut &first[..]),
                Err(FrameError::Malformed(_))
            ));
        }
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
