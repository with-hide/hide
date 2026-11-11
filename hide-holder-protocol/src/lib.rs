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
//! a terminate, and the holder sends the screen as [`Fragment`]s, what it
//! refused while the pane runs on as [`Notice`]s to a node that accepts
//! [`Capability::Notices`], what the program put on the clipboard as
//! [`HolderMessage::Clipboard`] to a node that accepts
//! [`Capability::Clipboard`], and why it closed. Releasing lets go of the connection and leaves the pane running;
//! terminating ends the pane's program and everything it started, then the
//! holder.
//!
//! Input comes in two kinds (PRD engine-bundle-2-pane-holder D-8): bytes
//! already encoded ([`ClientMessage::Input`]), written as they are, and
//! requests that mean something ([`ClientMessage::Paste`],
//! [`ClientMessage::Mouse`], [`ClientMessage::Scroll`], and
//! [`ClientMessage::Key`] to a holder that names [`Capability::Keys`]),
//! which the holder encodes for the pane's settings at that moment. Nothing
//! guesses from bytes what they were. Every message that becomes the
//! program's input is written in the order the connection sent it (PRD
//! engine-bundle-3-core-owns D-12).
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
//! question, the pane's recent or visible text, its processes or its scroll
//! position,
//! and is answered once and closed; it never takes the holder over and
//! leaves the screen connection and its scroll position as they were
//! (D-11, D-18), except [`Ask::ScrollTo`], which moves the screen as any
//! client may ask, so a pane search can show its match.
//!
//! A connection whose first message is a watch ([`Opening::Watch`]) follows
//! the pane's facts (PRD engine-bundle-3-core-owns D-18, D-21): it takes
//! nothing over and needs no screen, the holder answers [`Watching`] with the
//! version both speak and the version of its screen rule evaluator, and,
//! when the node accepts [`Capability::Facts`], sends the pane's [`Facts`]
//! whole each time any of them changed. On it a node gives the holder the
//! screen rules to read its screen by ([`WatchMessage::Rules`]). A holder
//! keeps one watch at a time; a newer one replaces the older, which is
//! closed [`CloseReason::TakenOver`]. A holder built before watches closes
//! the opening [`CloseReason::HelloInvalid`], which a node reads as a holder
//! that does not send facts.

use std::io::{self, Read, Write};
use std::time::Duration;

pub mod record;
pub mod styled;

/// The version of this protocol this build speaks as a holder.
pub const PROTOCOL_VERSION: u16 = 1;

/// The oldest holder version this build speaks to as a node. A holder older
/// than this is reported as unsupported and left running, never ended.
/// Raising it makes an app update cut off the panes that run, which the
/// ready-update card of every older build promises it does not
/// (`docs/UI_BEHAVIOR.md`, Help menu).
pub const OLDEST_HOLDER_VERSION: u16 = 1;

/// The most bytes one [`Fragment`]'s ANSI carries: what a node can draw at
/// once. A screen that encodes to more is sent as several fragments, split
/// between rows, so no screen is ever refused for its size.
pub const MAX_FRAGMENT: usize = 4 * 1024 * 1024;

/// The most bytes one frame carries, kind byte included: a fragment and its
/// header.
pub const MAX_FRAME: usize = MAX_FRAGMENT + 1024;

/// The most keys one [`ClientMessage::Key`] carries.
pub const MAX_KEYS: usize = 256;

/// The most bytes a [`Hello`] frame carries.
pub const MAX_HELLO: usize = 4 * 1024;

/// How long a holder waits for a connection's [`Hello`].
pub const HELLO_LIMIT: Duration = Duration::from_secs(2);

/// The most input bytes one [`ClientMessage::Input`] carries.
pub const MAX_INPUT: usize = 64 * 1024;

/// The most bytes one [`ClientMessage::Paste`] carries.
pub const MAX_PASTE: usize = 512 * 1024;

/// The most bytes a [`Submit`]'s submitting part carries, and the most its
/// text carries beyond [`MAX_PASTE`]: a focus report before it and a paste
/// boundary after it, as an agent prompt sends them.
pub const MAX_SUBMIT_ENTER: usize = 64;

/// The most parts a [`Submit`]'s text has.
pub const MAX_SUBMIT_PARTS: usize = 4;

/// The longest gap a [`Submit`] asks for between its text and its submit.
pub const MAX_SUBMIT_GAP: Duration = Duration::from_secs(2);

/// The most bytes a node's message after its hello carries: a
/// [`ClientMessage::Submit`] with the longest text and submit, and each
/// part's tag and length.
const MAX_CLIENT: usize = MAX_PASTE + 2 * MAX_SUBMIT_ENTER + 64;

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

/// The most bytes one [`HolderMessage::Clipboard`] carries: the most the
/// terminal parser takes from one OSC 52 write.
pub const MAX_CLIPBOARD: usize = 192 * 1024;

/// The most bytes of a [`Facts`]'s folder.
pub const MAX_CWD: usize = 4 * 1024;
/// The most characters of a [`Facts`]'s title, the bound on a title the
/// holder keeps; four bytes a character at most.
pub const MAX_TITLE_CHARS: usize = 256;
const MAX_TITLE: usize = MAX_TITLE_CHARS * 4;
/// The most arguments a [`Program`] carries, and their bytes in all, the
/// bounds on a resume's arguments.
pub const MAX_ARGS: usize = 64;
pub const MAX_ARGV_BYTES: usize = 8 * 1024;
/// The most bytes of a rule's id a [`ScreenRead`] names.
pub const MAX_RULE_ID: usize = 256;
/// The most bytes of a rule set a node gives a holder.
pub const MAX_RULES: usize = 1024 * 1024;

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
    pub const KEY: u8 = 0x15;
    pub const SUBMIT: u8 = 0x16;
    /// A first message, like [`HELLO`].
    pub const ASK: u8 = 0x18;
    /// A first message, like [`HELLO`]: a connection that follows the facts.
    pub const WATCH: u8 = 0x19;
    /// A watch's rule set, from a node.
    pub const RULES: u8 = 0x1a;
    pub const WELCOME: u8 = 0x81;
    pub const CLOSED: u8 = 0x83;
    /// The answer to a watch.
    pub const WATCHING: u8 = 0x84;
    pub const FRAGMENT: u8 = 0x90;
    pub const ANSWER: u8 = 0x91;
    pub const NOTICE: u8 = 0x92;
    pub const FACTS: u8 = 0x93;
    pub const CLIPBOARD: u8 = 0x94;
}

/// Something one side can do that the other must accept before it is used.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    /// The holder sends the screen as [`Fragment`]s.
    Screen,
    /// The holder sends [`Notice`]s. A node that does not accept them, as
    /// one built before they existed does not, is sent none, and the holder
    /// logs what it would have said.
    Notices,
    /// The holder sends the pane's [`Facts`] on a watch connection.
    Facts,
    /// Facts may append the foreground script's installed package. A
    /// holder sends this extension only to a watcher accepting it.
    ProgramPackage,
    /// The holder reads [`ClientMessage::Key`]; a node sends it only to a
    /// holder whose welcome names this, since an older holder reads it as a
    /// message it does not know and closes the connection.
    Keys,
    /// The holder reads PageUp and PageDown in a named key frame.
    PageKeys,
    /// The holder says on a watch whether its terminal edits lines
    /// ([`Facts::line_editing`]); a node that does not accept it, as one
    /// built before it does not, is sent facts without it.
    LineEditing,
    /// The holder says on a watch whether the system could not say what
    /// holds the terminal ([`Facts::foreground_unread`]), which a node built
    /// before it reads as the pane's own program holding it.
    ForegroundUnread,
    /// Facts include whether the pane uses the alternate screen. Older
    /// watchers receive no extension and keep the original frame shape.
    AlternateScreen,
    /// The holder reads [`ClientMessage::Submit`]; a node sends it only to a
    /// holder whose welcome names this.
    Submit,
    /// The holder sends what the program wrote to the clipboard (OSC 52) as
    /// [`HolderMessage::Clipboard`]. A node that does not accept it, as one
    /// built before it does not, is sent none, and the write is dropped.
    Clipboard,
    /// The holder reads a [`Mouse`] that carries [`Mouse::outside`]; a node
    /// sends one only to a holder whose welcome names this, since an older
    /// holder reads the longer message as malformed and closes the
    /// connection.
    MouseOutside,
    /// A capability this build does not know, kept by name so a side can
    /// answer with the ones it does.
    Other(String),
}

impl Capability {
    fn name(&self) -> &str {
        match self {
            Self::Screen => "screen",
            Self::Notices => "notices",
            Self::Facts => "facts",
            Self::ProgramPackage => "program_package",
            Self::Keys => "keys",
            Self::PageKeys => "page_keys",
            Self::LineEditing => "line_editing",
            Self::ForegroundUnread => "foreground_unread",
            Self::AlternateScreen => "alternate_screen",
            Self::Submit => "submit",
            Self::Clipboard => "clipboard",
            Self::MouseOutside => "mouse_outside",
            Self::Other(name) => name,
        }
    }

    fn from_name(name: &str) -> Self {
        match name {
            "screen" => Self::Screen,
            "notices" => Self::Notices,
            "facts" => Self::Facts,
            "program_package" => Self::ProgramPackage,
            "keys" => Self::Keys,
            "page_keys" => Self::PageKeys,
            "line_editing" => Self::LineEditing,
            "foreground_unread" => Self::ForegroundUnread,
            "alternate_screen" => Self::AlternateScreen,
            "submit" => Self::Submit,
            "clipboard" => Self::Clipboard,
            "mouse_outside" => Self::MouseOutside,
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

/// The holder's answer to a watch: the version both speak, the version of
/// its screen rule evaluator (a rule set whose `min_engine_version` is
/// higher is one it cannot read), and what it will send of what the node
/// accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watching {
    pub version: u16,
    pub rules_engine: u32,
    pub sends: Vec<Capability>,
}

/// What a holder says of its pane now, whole each time any of it changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// The folder the shell last reported (OSC 7, or OSC 9;9), as reported.
    pub cwd: Option<String>,
    /// The terminal's title (OSC 0 or 2), none when unset or emptied.
    pub title: Option<String>,
    /// The bells the program rang, and the prompts its shell marked
    /// (OSC 133;A), since the holder started.
    pub bells: u64,
    pub prompts: u64,
    /// Whether the program has bracketed paste on (mode 2004).
    pub bracketed_paste: bool,
    /// The program in the terminal's foreground other than the pane's own.
    pub foreground: Option<Program>,
    pub screen: ScreenRead,
    /// Whether the terminal edits input a line at a time before its program
    /// reads it, in every facts frame of a watch that accepted
    /// [`Capability::LineEditing`] and in no other (`None`).
    pub line_editing: Option<LineEditing>,
    /// Whether the system could not say what holds the terminal, so
    /// [`Self::foreground`] is none for want of an answer rather than
    /// because the pane's own program holds it, in every facts frame of a
    /// watch that accepted [`Capability::ForegroundUnread`] and in no other
    /// (`None`).
    pub foreground_unread: Option<bool>,
    /// Present exactly when the watch accepted [`Capability::AlternateScreen`].
    pub alternate_screen: Option<bool>,
}

/// The facts extensions a watch agreed: each is in every facts frame of that
/// watch and in no other, in this order after the screen.
#[derive(Clone, Copy, Debug, Default)]
struct Extensions {
    line_editing: bool,
    foreground_unread: bool,
    alternate_screen: bool,
}

impl Extensions {
    fn of(agreed: &[Capability]) -> Self {
        Self {
            line_editing: agreed.contains(&Capability::LineEditing),
            foreground_unread: agreed.contains(&Capability::ForegroundUnread),
            alternate_screen: agreed.contains(&Capability::AlternateScreen),
        }
    }
}

/// Whether a terminal edits input a line at a time before its program reads
/// it (canonical mode): until a program turns that off to read keys, as every
/// agent's screen does, what is typed waits in the terminal's line and an
/// Enter in it reaches the program as a line break.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEditing {
    /// The program reads keys.
    ReadsKeys,
    /// The terminal edits lines.
    Edits,
    /// The system keeps no such mode (Windows) or the terminal could not be
    /// asked.
    Unknown,
}

impl LineEditing {
    fn code(self) -> u8 {
        match self {
            Self::ReadsKeys => 0,
            Self::Edits => 1,
            Self::Unknown => 2,
        }
    }

    fn from_code(code: u8) -> Result<Self, FrameError> {
        match code {
            0 => Ok(Self::ReadsKeys),
            1 => Ok(Self::Edits),
            2 => Ok(Self::Unknown),
            other => Err(FrameError::Malformed(format!(
                "an unknown line editing {other}"
            ))),
        }
    }
}

/// A program in a pane's foreground.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub pid: u32,
    /// Its executable or runtime script name.
    pub name: String,
    /// The installed package, a second clue after the executable name.
    /// Only sent when the watch accepts [`Capability::ProgramPackage`].
    pub package: Option<String>,
    /// Its arguments, the program first; empty when the system cannot read them.
    pub argv: Vec<String>,
}

/// A state the screen rules read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenState {
    Idle,
    Working,
    Blocked,
    Unknown,
}

/// What the screen rules read, or why they read nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ScreenRead {
    /// The holder was given no rules.
    #[default]
    NoRules,
    /// The state the rules read and the rule that last moved it.
    Read {
        state: ScreenState,
        rule: Option<String>,
    },
    /// The rules cannot be read, and why.
    Unknown { reason: String },
}

/// What a node sends on a watch connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchMessage {
    /// The rule set to read the screen by, as `hide_screen_rules::Manifest`
    /// serializes to JSON, in place of any before it; none takes them away.
    Rules(Option<String>),
    /// Lets go of the watch.
    Release,
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

/// What a [`Notice`] says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeReason {
    /// The pane's program asked questions whose answers waited for it past
    /// the holder's answer bound, since it was not reading its input; the
    /// answers of the read that crossed it were refused whole, and those of
    /// every read after it are, until the program takes what waits.
    AnswersRefused,
    /// The program took its waiting answers again; the notice counts the
    /// answers refused since [`NoticeReason::AnswersRefused`].
    AnswersResumed,
    /// A reason this build does not know, by its code, so a node reads a
    /// newer holder's notice without failing the connection.
    Other(u16),
}

impl NoticeReason {
    pub fn code(self) -> u16 {
        match self {
            Self::AnswersRefused => 1,
            Self::AnswersResumed => 2,
            Self::Other(code) => code,
        }
    }

    pub fn from_code(code: u16) -> Self {
        match code {
            1 => Self::AnswersRefused,
            2 => Self::AnswersResumed,
            other => Self::Other(other),
        }
    }

    /// The name logs use.
    pub fn name(self) -> &'static str {
        match self {
            Self::AnswersRefused => "answers_refused",
            Self::AnswersResumed => "answers_resumed",
            Self::Other(_) => "unknown",
        }
    }
}

/// Something the holder refused while the pane runs on, told to the
/// connection that holds the pane when it accepts [`Capability::Notices`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub reason: NoticeReason,
    /// How many of what the reason names, and their bytes.
    pub count: u64,
    pub bytes: u64,
    /// What happened, for the log; never parsed.
    pub detail: String,
}

impl Notice {
    pub fn new(reason: NoticeReason, count: u64, bytes: u64, detail: impl Into<String>) -> Self {
        let mut detail = detail.into();
        truncate(&mut detail, MAX_DETAIL);
        Self {
            reason,
            count,
            bytes,
            detail,
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
/// passes the one it has when it starts the holder (`--theme`, in
/// [`Theme::to_arg`]'s form), so the program's first question is answered
/// with it, and sends one whenever its theme changes. A color left `None` is
/// one the node does not know, and a question about it is answered with the
/// parser's own default, as every question is when no theme was given, so a
/// program never waits for an answer that does not come.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub cursor: Option<Rgb>,
    /// Palette entries the theme sets; the rest keep the xterm defaults.
    pub palette: Vec<(u8, Rgb)>,
    pub appearance: Appearance,
}

impl Theme {
    fn put(&self, payload: &mut Vec<u8>) {
        put_rgb(payload, self.foreground);
        put_rgb(payload, self.background);
        put_rgb(payload, self.cursor);
        payload.push(match self.appearance {
            Appearance::Unknown => 0,
            Appearance::Dark => 1,
            Appearance::Light => 2,
        });
        let palette = &self.palette[..self.palette.len().min(256)];
        payload.extend_from_slice(&(palette.len() as u16).to_le_bytes());
        for (index, color) in palette {
            payload.extend_from_slice(&[*index, color.r, color.g, color.b]);
        }
    }

    fn take(payload: &mut Payload<'_>) -> Result<Self, FrameError> {
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
        Ok(Self {
            foreground,
            background,
            cursor,
            palette,
            appearance,
        })
    }

    /// The theme as the holder's `--theme` argument: the message's bytes
    /// in hex, at most 2,070 characters.
    pub fn to_arg(&self) -> String {
        let mut payload = Vec::new();
        self.put(&mut payload);
        payload.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Reads a `--theme` argument [`Theme::to_arg`] wrote.
    pub fn from_arg(text: &str) -> Result<Self, String> {
        if !text.len().is_multiple_of(2) || text.len() > 2 * (16 + 256 * 4) {
            return Err("the theme is not a theme argument".to_owned());
        }
        let bytes = (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(text.get(at..at + 2).unwrap_or("zz"), 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "the theme is not hex".to_owned())?;
        let mut payload = Payload(&bytes);
        let theme = Self::take(&mut payload).map_err(|error| error.to_string())?;
        payload.end().map_err(|error| error.to_string())?;
        Ok(theme)
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// The cell under the pointer when a held button dragged it past the
    /// grid's edge, so a column or row below zero or past the last one;
    /// `col` and `row` then name the grid's cell nearest it. `None` over the
    /// grid. Sent only to a holder that names [`Capability::MouseOutside`].
    pub outside: Option<(i32, i32)>,
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
    /// The rows the screen shows now, wherever it is scrolled to, with
    /// their styling as escape sequences. A holder built before styled reads refuses it
    /// `hello_invalid`.
    Screen,
    /// The last `lines` rows of the screen and its scrollback, soft wraps
    /// joined, with their styling as escape sequences, at most
    /// [`MAX_READ_LINES`]; the rows are the ones [`Ask::Read`] reads. A
    /// holder built before it joined them answers the rows as drawn.
    Rows { lines: u16 },
    /// Where the screen is scrolled to in its scrollback, answered
    /// [`Answer::Scroll`].
    Scroll,
    /// Moves the screen to `offset_from_bottom` rows above the bottom of
    /// its scrollback, past the top clamped to it, and answers where it is
    /// then; the connection that holds the pane is
    /// sent the moved screen. The only ask that changes the screen.
    ScrollTo { offset_from_bottom: u64 },
    /// The rows the screen shows now, wherever it is scrolled to, as text
    /// with soft wraps joined.
    Visible,
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
    /// The screen's place in its scrollback, in rows above the bottom.
    Scroll {
        offset_from_bottom: u64,
        max_offset_from_bottom: u64,
    },
}

/// A key by its name, which the holder encodes for the program's keyboard
/// settings at that moment (PRD engine-bundle-3-core-owns D-21): an arrow in
/// application cursor mode, Escape under a keyboard protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    Escape,
    Enter,
    Tab,
    ShiftTab,
    Up,
    Down,
    Left,
    Right,
    Backspace,
    CtrlC,
    PageUp,
    PageDown,
}

impl Key {
    const ALL: [Self; 12] = [
        Self::Escape,
        Self::Enter,
        Self::Tab,
        Self::ShiftTab,
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::Backspace,
        Self::CtrlC,
        Self::PageUp,
        Self::PageDown,
    ];

    fn code(self) -> u8 {
        match self {
            Self::Escape => 0,
            Self::Enter => 1,
            Self::Tab => 2,
            Self::ShiftTab => 3,
            Self::Up => 4,
            Self::Down => 5,
            Self::Left => 6,
            Self::Right => 7,
            Self::Backspace => 8,
            Self::CtrlC => 9,
            Self::PageUp => 10,
            Self::PageDown => 11,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.code() == code)
    }
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
    /// Keys pressed in order, at most [`MAX_KEYS`], to a holder that names
    /// [`Capability::Keys`].
    Key(Vec<Key>),
    /// One submission to a holder that names [`Capability::Submit`].
    Submit(Submit),
    Resize(Size),
    Theme(Theme),
    /// The next fragment is full: the node lost track of the screen.
    Redraw,
    /// Lets go of the connection; the pane runs on.
    Release,
    /// Ends the pane's program and everything it started, then the holder.
    Terminate,
}

/// One submission, as an agent prompt makes it, never without text
/// or a submit: the holder writes
/// `text`'s parts to the program in order and flushes them, waits `gap`,
/// then writes `enter`,
/// and the connection's input that came after waits until `enter` was
/// written, so nothing lands between them and a program that reads keys
/// arriving together as a paste still submits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submit {
    /// At most [`MAX_SUBMIT_PARTS`], [`MAX_PASTE`] and [`MAX_SUBMIT_ENTER`]
    /// bytes together.
    pub text: Vec<SubmitPart>,
    /// At most [`MAX_SUBMIT_GAP`].
    pub gap: Duration,
    pub enter: SubmitPart,
}

/// What one part of a [`Submit`] types, encoded the way the message of the
/// same kind is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmitPart {
    Input(Vec<u8>),
    Paste(Vec<u8>),
    Keys(Vec<Key>),
}

impl SubmitPart {
    /// The bytes or keys it carries, as its bound counts them.
    pub fn len(&self) -> usize {
        match self {
            Self::Input(bytes) | Self::Paste(bytes) => bytes.len(),
            Self::Keys(keys) => keys.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Puts it in `payload` and answers how much of `bound` it took.
    fn put(&self, payload: &mut Vec<u8>, bound: usize) -> io::Result<usize> {
        let (tag, bytes): (u8, Vec<u8>) = match self {
            Self::Input(bytes) => (0, bytes.clone()),
            Self::Paste(bytes) => (1, bytes.clone()),
            Self::Keys(keys) => (2, keys.iter().map(|key| key.code()).collect()),
        };
        let bound = if tag == 2 { bound.min(MAX_KEYS) } else { bound };
        if bytes.len() > bound {
            return Err(too_long("submit part", bytes.len(), bound));
        }
        payload.push(tag);
        payload.extend_from_slice(&u32::try_from(bytes.len()).expect("bounded").to_le_bytes());
        payload.extend_from_slice(&bytes);
        Ok(bytes.len())
    }

    fn take(payload: &mut Payload<'_>, bound: usize) -> Result<Self, FrameError> {
        let tag = payload.u8()?;
        let length = usize::try_from(payload.u32()?).unwrap_or(usize::MAX);
        let bound = if tag == 2 { bound.min(MAX_KEYS) } else { bound };
        if length > bound {
            return Err(FrameError::Malformed(format!(
                "a submit part of {length} bytes, past {bound}"
            )));
        }
        let bytes = payload.take(length)?.to_vec();
        Ok(match tag {
            0 => Self::Input(bytes),
            1 => Self::Paste(bytes),
            2 => Self::Keys(
                bytes
                    .into_iter()
                    .map(|code| {
                        Key::from_code(code)
                            .ok_or_else(|| FrameError::Malformed(format!("an unknown key {code}")))
                    })
                    .collect::<Result<_, _>>()?,
            ),
            other => {
                return Err(FrameError::Malformed(format!(
                    "an unknown submit part {other}"
                )));
            }
        })
    }
}

/// A connection's first message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opening {
    Hello(Hello),
    Ask(Ask),
    /// A connection that follows the pane's facts, with the versions the
    /// node speaks and what it accepts.
    Watch(Hello),
}

/// What a holder sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HolderMessage {
    Welcome(Welcome),
    Fragment(Fragment),
    Answer(Answer),
    Notice(Notice),
    /// The text the program last wrote to the clipboard, at most
    /// [`MAX_CLIPBOARD`] bytes, sent once to the connection that holds the
    /// pane and never again: it is no part of the screen, so a redraw or a
    /// takeover does not repeat it.
    Clipboard(Vec<u8>),
    Watching(Watching),
    Facts(Facts),
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
    write_frame_parts(writer, kind, &[payload])
}

/// Writes one frame whose payload is `parts` in order, copied once into the
/// frame: a fragment's screen bytes go in after its header without first
/// being gathered into a payload of their own.
fn write_frame_parts(writer: &mut impl Write, kind: u8, parts: &[&[u8]]) -> io::Result<()> {
    let length = parts.iter().map(|part| part.len()).sum::<usize>() + 1;
    if length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("a frame of {length} bytes is past the bound"),
        ));
    }
    let mut frame = Vec::with_capacity(length + 4);
    frame.extend_from_slice(&(length as u32).to_le_bytes());
    frame.push(kind);
    for part in parts {
        frame.extend_from_slice(part);
    }
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
            if let Some((col, row)) = mouse.outside {
                payload.extend_from_slice(&col.to_le_bytes());
                payload.extend_from_slice(&row.to_le_bytes());
            }
            write_frame(writer, kind::MOUSE, &payload)
        }
        ClientMessage::Scroll(scroll) => {
            let mut payload = scroll.lines.to_le_bytes().to_vec();
            put_cell(&mut payload, scroll.cell);
            payload.push(scroll.modifiers.0);
            write_frame(writer, kind::SCROLL, &payload)
        }
        ClientMessage::Key(keys) => {
            if keys.len() > MAX_KEYS {
                return Err(too_long("keys", keys.len(), MAX_KEYS));
            }
            let payload: Vec<u8> = keys.iter().map(|key| key.code()).collect();
            write_frame(writer, kind::KEY, &payload)
        }
        ClientMessage::Submit(submit) => {
            if submit.gap > MAX_SUBMIT_GAP {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("a submit gap of {:?}, past {MAX_SUBMIT_GAP:?}", submit.gap),
                ));
            }
            let gap = u16::try_from(submit.gap.as_millis()).expect("bounded above");
            let mut payload = gap.to_le_bytes().to_vec();
            if submit.text.len() > MAX_SUBMIT_PARTS {
                return Err(too_long(
                    "submit text parts",
                    submit.text.len(),
                    MAX_SUBMIT_PARTS,
                ));
            }
            if submit.text.iter().all(SubmitPart::is_empty) || submit.enter.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a submission types text and a submit",
                ));
            }
            payload.push(u8::try_from(submit.text.len()).expect("bounded above"));
            let mut left = MAX_PASTE + MAX_SUBMIT_ENTER;
            for part in &submit.text {
                left -= part.put(&mut payload, left)?;
            }
            submit.enter.put(&mut payload, MAX_SUBMIT_ENTER)?;
            write_frame(writer, kind::SUBMIT, &payload)
        }
        ClientMessage::Resize(size) => {
            let mut payload = size.rows.to_le_bytes().to_vec();
            payload.extend_from_slice(&size.cols.to_le_bytes());
            write_frame(writer, kind::RESIZE, &payload)
        }
        ClientMessage::Theme(theme) => {
            let mut payload = Vec::with_capacity(16 + theme.palette.len() * 4);
            theme.put(&mut payload);
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
        Opening::Ask(ask) => write_ask(writer, ask, PROTOCOL_VERSION),
        Opening::Watch(hello) => {
            let mut payload = MAGIC.to_vec();
            payload.extend_from_slice(&hello.versions.0.to_le_bytes());
            payload.extend_from_slice(&hello.versions.1.to_le_bytes());
            put_capabilities(&mut payload, &hello.accepts);
            write_frame(writer, kind::WATCH, &payload)
        }
    }
}

/// Writes an [`Ask`] opening in `version`, the version the holder that reads
/// it speaks: a holder answers an ask only in its own version, so a node asks
/// an older holder in that holder's version. The bytes are the same in every
/// version this build speaks to.
pub fn write_ask(writer: &mut impl Write, ask: &Ask, version: u16) -> io::Result<()> {
    let mut payload = MAGIC.to_vec();
    payload.extend_from_slice(&version.to_le_bytes());
    match ask {
        Ask::Read { lines } => {
            payload.push(1);
            payload.extend_from_slice(&lines.to_le_bytes());
        }
        Ask::Processes => payload.push(2),
        Ask::Screen => payload.push(3),
        Ask::Rows { lines } => {
            payload.push(4);
            payload.extend_from_slice(&lines.to_le_bytes());
        }
        Ask::Scroll => payload.push(5),
        Ask::Visible => payload.push(7),
        Ask::ScrollTo { offset_from_bottom } => {
            payload.push(6);
            payload.extend_from_slice(&offset_from_bottom.to_le_bytes());
        }
    }
    write_frame(writer, kind::ASK, &payload)
}

/// Reads a connection's first message, a [`Hello`] or an [`Ask`] within
/// [`MAX_HELLO`] bytes. The caller bounds the time. An ask is read whatever
/// it asks; [`Ask::check`] says whether the holder answers it.
pub fn read_opening(reader: &mut impl Read) -> Result<Opening, FrameError> {
    let (kind, payload) = read_frame(reader, MAX_HELLO)?;
    if kind != kind::HELLO && kind != kind::ASK && kind != kind::WATCH {
        return Err(FrameError::Malformed(format!(
            "the first message is of kind {kind:#04x}, not a hello, an ask or a watch"
        )));
    }
    let mut payload = Payload(&payload);
    if payload.take(MAGIC.len())? != MAGIC {
        return Err(FrameError::Malformed(
            "the first message is not a node's".to_owned(),
        ));
    }
    let opening = if kind == kind::HELLO || kind == kind::WATCH {
        let versions = (payload.u16()?, payload.u16()?);
        let accepts = payload.capabilities()?;
        if versions.0 > versions.1 {
            return Err(FrameError::Malformed(format!(
                "the versions {}..{} are empty",
                versions.0, versions.1
            )));
        }
        let hello = Hello { versions, accepts };
        if kind == kind::HELLO {
            Opening::Hello(hello)
        } else {
            Opening::Watch(hello)
        }
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
            3 => Opening::Ask(Ask::Screen),
            4 => Opening::Ask(Ask::Rows {
                lines: payload.u16()?,
            }),
            5 => Opening::Ask(Ask::Scroll),
            6 => Opening::Ask(Ask::ScrollTo {
                offset_from_bottom: payload.u64()?,
            }),
            7 => Opening::Ask(Ask::Visible),
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
            Self::Read { lines } | Self::Rows { lines }
                if *lines == 0 || *lines > MAX_READ_LINES =>
            {
                Err(Closed::new(
                    CloseReason::AskInvalid,
                    format!("{lines} rows asked for, not 1 to {MAX_READ_LINES}"),
                ))
            }
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
            let (col, row, modifiers) = (payload.u16()?, payload.u16()?, payload.u8()?);
            let outside = if payload.0.is_empty() {
                None
            } else {
                Some((payload.u32()? as i32, payload.u32()? as i32))
            };
            ClientMessage::Mouse(Mouse {
                action,
                button,
                col,
                row,
                modifiers: Modifiers(modifiers),
                outside,
            })
        }
        kind::SCROLL => ClientMessage::Scroll(Scroll {
            lines: payload.u16()? as i16,
            cell: payload.cell()?,
            modifiers: Modifiers(payload.u8()?),
        }),
        kind::KEY => {
            let codes = payload.rest_bounded("keys", MAX_KEYS)?;
            let keys = codes
                .into_iter()
                .map(|code| {
                    Key::from_code(code)
                        .ok_or_else(|| FrameError::Malformed(format!("an unknown key {code}")))
                })
                .collect::<Result<_, _>>()?;
            ClientMessage::Key(keys)
        }
        kind::SUBMIT => {
            let gap = Duration::from_millis(u64::from(payload.u16()?));
            if gap > MAX_SUBMIT_GAP {
                return Err(FrameError::Malformed(format!(
                    "a submit gap of {gap:?}, past {MAX_SUBMIT_GAP:?}"
                )));
            }
            let parts = usize::from(payload.u8()?);
            if parts > MAX_SUBMIT_PARTS {
                return Err(FrameError::Malformed(format!(
                    "a submit of {parts} text parts, past {MAX_SUBMIT_PARTS}"
                )));
            }
            let mut left = MAX_PASTE + MAX_SUBMIT_ENTER;
            let text: Vec<SubmitPart> = (0..parts)
                .map(|_| {
                    let part = SubmitPart::take(&mut payload, left)?;
                    left -= part.len();
                    Ok(part)
                })
                .collect::<Result<_, FrameError>>()?;
            let enter = SubmitPart::take(&mut payload, MAX_SUBMIT_ENTER)?;
            if text.iter().all(SubmitPart::is_empty) || enter.is_empty() {
                return Err(FrameError::Malformed(
                    "a submission without text or a submit".to_owned(),
                ));
            }
            ClientMessage::Submit(Submit { text, gap, enter })
        }
        kind::RESIZE => ClientMessage::Resize(payload.size()?),
        kind::THEME => ClientMessage::Theme(Theme::take(&mut payload)?),
        kind::REDRAW => ClientMessage::Redraw,
        kind::RELEASE => ClientMessage::Release,
        kind::TERMINATE => ClientMessage::Terminate,
        kind::HELLO | kind::ASK | kind::WATCH => {
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
            let mut header = Vec::with_capacity(21);
            header.extend_from_slice(&fragment.epoch.to_le_bytes());
            header.extend_from_slice(&fragment.seq.to_le_bytes());
            header.push(u8::from(fragment.full));
            header.extend_from_slice(&fragment.size.rows.to_le_bytes());
            header.extend_from_slice(&fragment.size.cols.to_le_bytes());
            write_frame_parts(writer, kind::FRAGMENT, &[&header, &fragment.bytes])
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
                Answer::Scroll {
                    offset_from_bottom,
                    max_offset_from_bottom,
                } => {
                    payload.push(3);
                    payload.extend_from_slice(&offset_from_bottom.to_le_bytes());
                    payload.extend_from_slice(&max_offset_from_bottom.to_le_bytes());
                }
            }
            write_frame(writer, kind::ANSWER, &payload)
        }
        HolderMessage::Notice(notice) => {
            let mut payload = notice.reason.code().to_le_bytes().to_vec();
            payload.extend_from_slice(&notice.count.to_le_bytes());
            payload.extend_from_slice(&notice.bytes.to_le_bytes());
            let mut detail = notice.detail.clone();
            truncate(&mut detail, MAX_DETAIL);
            put_text(&mut payload, &detail);
            write_frame(writer, kind::NOTICE, &payload)
        }
        HolderMessage::Clipboard(text) => {
            if text.len() > MAX_CLIPBOARD {
                return Err(too_long("clipboard", text.len(), MAX_CLIPBOARD));
            }
            write_frame(writer, kind::CLIPBOARD, text)
        }
        HolderMessage::Watching(watching) => {
            let mut payload = watching.version.to_le_bytes().to_vec();
            payload.extend_from_slice(&watching.rules_engine.to_le_bytes());
            put_capabilities(&mut payload, &watching.sends);
            write_frame(writer, kind::WATCHING, &payload)
        }
        HolderMessage::Facts(facts) => {
            let mut payload = Vec::new();
            facts.put(&mut payload);
            write_frame(writer, kind::FACTS, &payload)
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

/// Reads one message from a holder, on a connection that agreed no facts
/// extension beyond the package.
pub fn read_holder(reader: &mut impl Read) -> Result<HolderMessage, FrameError> {
    read_holder_agreed(reader, Extensions::default())
}

/// Reads one message on a watch whose [`Watching`] named `agreed`: a facts
/// extension is in a frame exactly when its watch agreed it, so it is read
/// only then.
pub fn read_watch_holder(
    reader: &mut impl Read,
    agreed: &[Capability],
) -> Result<HolderMessage, FrameError> {
    read_holder_agreed(reader, Extensions::of(agreed))
}

fn read_holder_agreed(
    reader: &mut impl Read,
    agreed: Extensions,
) -> Result<HolderMessage, FrameError> {
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
            3 => Answer::Scroll {
                offset_from_bottom: payload.u64()?,
                max_offset_from_bottom: payload.u64()?,
            },
            other => {
                return Err(FrameError::Malformed(format!("an unknown answer {other}")));
            }
        }),
        kind::NOTICE => {
            let reason = NoticeReason::from_code(payload.u16()?);
            let count = payload.u64()?;
            let bytes = payload.u64()?;
            let detail = payload.text(MAX_DETAIL)?;
            HolderMessage::Notice(Notice {
                reason,
                count,
                bytes,
                detail,
            })
        }
        kind::CLIPBOARD => {
            HolderMessage::Clipboard(payload.rest_bounded("clipboard", MAX_CLIPBOARD)?)
        }
        kind::WATCHING => {
            let version = payload.u16()?;
            let rules_engine = payload.u32()?;
            let sends = payload.capabilities()?;
            HolderMessage::Watching(Watching {
                version,
                rules_engine,
                sends,
            })
        }
        kind::FACTS => HolderMessage::Facts(Facts::take(&mut payload, agreed)?),
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
    // The opening is bounded on a handle of its own; the connection goes on
    // past the deadline.
    let mut bounded = stream.duplicate();
    stream.clear_deadline();
    write_opening(&mut bounded, &Opening::Hello(Hello::current(accepts))).map_err(OpenError::Io)?;
    match read_holder(&mut bounded)? {
        HolderMessage::Welcome(welcome) if welcome.version < OLDEST_HOLDER_VERSION => {
            Err(OpenError::Unsupported {
                version: welcome.version,
            })
        }
        HolderMessage::Welcome(welcome) => Ok((stream, welcome)),
        HolderMessage::Closed(closed) => Err(refused(closed)),
        HolderMessage::Fragment(_)
        | HolderMessage::Answer(_)
        | HolderMessage::Notice(_)
        | HolderMessage::Clipboard(_)
        | HolderMessage::Watching(_)
        | HolderMessage::Facts(_) => Err(OpenError::Protocol(FrameError::Malformed(
            "a screen, an answer, a notice or facts before the welcome".to_owned(),
        ))),
    }
}

/// Asks the holder at `path` one question on a connection of its own, in
/// `version` (the one its pane's record names), by `deadline`. The
/// connection that holds the pane, if any, goes on as it was.
pub fn ask(
    path: &std::path::Path,
    ask: Ask,
    version: u16,
    deadline: std::time::Instant,
) -> Result<Answer, OpenError> {
    if version < OLDEST_HOLDER_VERSION {
        return Err(OpenError::Unsupported { version });
    }
    let mut stream =
        hide_platform::ipc::LocalStream::connect_until(path, deadline).map_err(OpenError::Io)?;
    stream.set_deadline(deadline);
    write_ask(&mut stream, &ask, version).map_err(OpenError::Io)?;
    match read_holder(&mut stream)? {
        HolderMessage::Answer(answer) => Ok(answer),
        HolderMessage::Closed(closed) => Err(refused(closed)),
        HolderMessage::Welcome(_)
        | HolderMessage::Fragment(_)
        | HolderMessage::Notice(_)
        | HolderMessage::Clipboard(_)
        | HolderMessage::Watching(_)
        | HolderMessage::Facts(_) => Err(OpenError::Protocol(FrameError::Malformed(
            "an ask answered with a screen, a notice or facts".to_owned(),
        ))),
    }
}

/// Opens a watch of the holder at `path`: connects, says the watch with
/// `accepts`, and reads the holder's answer, all by `deadline`. A holder
/// built before watches refuses it [`CloseReason::HelloInvalid`]
/// ([`OpenError::Refused`]). The stream it answers has no deadline of its
/// own.
pub fn watch(
    path: &std::path::Path,
    accepts: Vec<Capability>,
    deadline: std::time::Instant,
) -> Result<(hide_platform::ipc::LocalStream, Watching), OpenError> {
    let stream =
        hide_platform::ipc::LocalStream::connect_until(path, deadline).map_err(OpenError::Io)?;
    let mut bounded = stream.duplicate();
    stream.clear_deadline();
    write_opening(&mut bounded, &Opening::Watch(Hello::current(accepts))).map_err(OpenError::Io)?;
    match read_holder(&mut bounded)? {
        HolderMessage::Watching(watching) if watching.version < OLDEST_HOLDER_VERSION => {
            Err(OpenError::Unsupported {
                version: watching.version,
            })
        }
        HolderMessage::Watching(watching) => Ok((stream, watching)),
        HolderMessage::Closed(closed) => Err(refused(closed)),
        HolderMessage::Welcome(_)
        | HolderMessage::Fragment(_)
        | HolderMessage::Answer(_)
        | HolderMessage::Notice(_)
        | HolderMessage::Clipboard(_)
        | HolderMessage::Facts(_) => Err(OpenError::Protocol(FrameError::Malformed(
            "a watch answered with something other than watching".to_owned(),
        ))),
    }
}

/// Writes one message a node sends on a watch connection.
pub fn write_watch(writer: &mut impl Write, message: &WatchMessage) -> io::Result<()> {
    match message {
        WatchMessage::Rules(rules) => {
            let mut payload = Vec::new();
            match rules {
                Some(rules) => {
                    if rules.len() > MAX_RULES {
                        return Err(too_long("rule set", rules.len(), MAX_RULES));
                    }
                    payload.push(1);
                    payload.extend_from_slice(rules.as_bytes());
                }
                None => payload.push(0),
            }
            write_frame(writer, kind::RULES, &payload)
        }
        WatchMessage::Release => write_frame(writer, kind::RELEASE, &[]),
    }
}

/// Reads one message a node sends on a watch connection.
pub fn read_watch(reader: &mut impl Read) -> Result<WatchMessage, FrameError> {
    let (kind, payload) = read_frame(reader, MAX_RULES + 64)?;
    let mut payload = Payload(&payload);
    let message = match kind {
        kind::RULES => match payload.u8()? {
            0 => WatchMessage::Rules(None),
            _ => {
                let rules = payload.rest_bounded("rule set", MAX_RULES)?;
                WatchMessage::Rules(Some(
                    String::from_utf8(rules)
                        .map_err(|_| FrameError::Malformed("a rule set is not UTF-8".to_owned()))?,
                ))
            }
        },
        kind::RELEASE => WatchMessage::Release,
        other => {
            return Err(FrameError::Malformed(format!(
                "a message of kind {other:#04x} a node may not send on a watch"
            )));
        }
    };
    payload.end()?;
    Ok(message)
}

/// A text cut to at most `bound` bytes on a character boundary.
fn bounded(text: &str, bound: usize) -> String {
    let mut text = text.to_owned();
    truncate(&mut text, bound);
    text
}

fn put_optional_text(payload: &mut Vec<u8>, text: Option<&str>, bound: usize) {
    match text {
        Some(text) => {
            payload.push(1);
            put_text(payload, &bounded(text, bound));
        }
        None => payload.push(0),
    }
}

impl Payload<'_> {
    fn optional_text(&mut self, bound: usize) -> Result<Option<String>, FrameError> {
        Ok(match self.u8()? {
            0 => None,
            _ => Some(self.text(bound)?),
        })
    }
}

impl ScreenState {
    fn code(self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Working => 1,
            Self::Blocked => 2,
            Self::Unknown => 3,
        }
    }

    fn from_code(code: u8) -> Result<Self, FrameError> {
        Ok(match code {
            0 => Self::Idle,
            1 => Self::Working,
            2 => Self::Blocked,
            3 => Self::Unknown,
            other => {
                return Err(FrameError::Malformed(format!(
                    "an unknown screen state {other}"
                )));
            }
        })
    }
}

impl Facts {
    /// Writes the facts, each part cut to its bound: a title to
    /// [`MAX_TITLE_CHARS`] characters, a program's arguments to the first
    /// [`MAX_ARGS`] that fit in [`MAX_ARGV_BYTES`].
    fn put(&self, payload: &mut Vec<u8>) {
        put_optional_text(payload, self.cwd.as_deref(), MAX_CWD);
        let title = self
            .title
            .as_deref()
            .map(|title| title.chars().take(MAX_TITLE_CHARS).collect::<String>());
        put_optional_text(payload, title.as_deref(), MAX_TITLE);
        payload.extend_from_slice(&self.bells.to_le_bytes());
        payload.extend_from_slice(&self.prompts.to_le_bytes());
        payload.push(u8::from(self.bracketed_paste));
        match &self.foreground {
            Some(program) => {
                payload.push(1);
                payload.extend_from_slice(&program.pid.to_le_bytes());
                put_text(payload, &bounded(&program.name, MAX_PROCESS_NAME));
                let mut total = 0;
                let args: Vec<&String> = program
                    .argv
                    .iter()
                    .take(MAX_ARGS)
                    .take_while(|arg| {
                        total += arg.len();
                        total <= MAX_ARGV_BYTES
                    })
                    .collect();
                payload.extend_from_slice(&(args.len() as u16).to_le_bytes());
                for arg in args {
                    put_text(payload, arg);
                }
            }
            None => payload.push(0),
        }
        match &self.screen {
            ScreenRead::NoRules => payload.push(0),
            ScreenRead::Read { state, rule } => {
                payload.push(1);
                payload.push(state.code());
                put_optional_text(payload, rule.as_deref(), MAX_RULE_ID);
            }
            ScreenRead::Unknown { reason } => {
                payload.push(2);
                put_text(payload, &bounded(reason, MAX_DETAIL));
            }
        }
        // Present exactly when the watch agreed it, before the package,
        // so each extension is read only where it was agreed.
        if let Some(editing) = self.line_editing {
            payload.push(editing.code());
        }
        if let Some(unread) = self.foreground_unread {
            payload.push(u8::from(unread));
        }
        if let Some(alternate) = self.alternate_screen {
            payload.push(u8::from(alternate));
        }
        // No extension is the original facts frame. Older watchers are
        // handed a program with no package by the negotiated writer.
        if let Some(package) = self.foreground.as_ref().and_then(|p| p.package.as_deref()) {
            put_text(payload, &bounded(package, MAX_PROCESS_NAME));
        }
    }

    /// Reads facts whose frame carries the extensions its watch agreed.
    fn take(payload: &mut Payload<'_>, agreed: Extensions) -> Result<Self, FrameError> {
        let cwd = payload.optional_text(MAX_CWD)?;
        let title = payload.optional_text(MAX_TITLE)?;
        let bells = payload.u64()?;
        let prompts = payload.u64()?;
        let bracketed_paste = payload.u8()? != 0;
        let mut foreground = match payload.u8()? {
            0 => None,
            _ => {
                let pid = payload.u32()?;
                let name = payload.text(MAX_PROCESS_NAME)?;
                let count = payload.u16()? as usize;
                if count > MAX_ARGS {
                    return Err(FrameError::Malformed(format!(
                        "{count} arguments are past the bound"
                    )));
                }
                let argv: Vec<String> = (0..count)
                    .map(|_| payload.text(MAX_ARGV_BYTES))
                    .collect::<Result<_, _>>()?;
                if argv.iter().map(String::len).sum::<usize>() > MAX_ARGV_BYTES {
                    return Err(FrameError::Malformed(
                        "a program's arguments are past their bound".to_owned(),
                    ));
                }
                Some(Program {
                    pid,
                    name,
                    package: None,
                    argv,
                })
            }
        };
        let screen = match payload.u8()? {
            0 => ScreenRead::NoRules,
            1 => ScreenRead::Read {
                state: ScreenState::from_code(payload.u8()?)?,
                rule: payload.optional_text(MAX_RULE_ID)?,
            },
            2 => ScreenRead::Unknown {
                reason: payload.text(MAX_DETAIL)?,
            },
            other => {
                return Err(FrameError::Malformed(format!(
                    "an unknown screen read {other}"
                )));
            }
        };
        let line_editing = if agreed.line_editing {
            Some(LineEditing::from_code(payload.u8()?)?)
        } else {
            None
        };
        let foreground_unread = if agreed.foreground_unread {
            let unread = payload.u8()? != 0;
            if unread && foreground.is_some() {
                return Err(FrameError::Malformed(
                    "a foreground program the system could not say".to_owned(),
                ));
            }
            Some(unread)
        } else {
            None
        };
        let alternate_screen = if agreed.alternate_screen {
            Some(payload.u8()? != 0)
        } else {
            None
        };
        if !payload.0.is_empty() {
            let program = foreground.as_mut().ok_or_else(|| {
                FrameError::Malformed("a package without a foreground program".to_owned())
            })?;
            program.package = Some(payload.text(MAX_PROCESS_NAME)?);
        }
        Ok(Self {
            cwd,
            title,
            bells,
            prompts,
            bracketed_paste,
            foreground,
            screen,
            line_editing,
            foreground_unread,
            alternate_screen,
        })
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

    /// The extensions a watch agreed sit between the screen and the package,
    /// in every facts frame of that watch and in no other: facts without
    /// them are the frame an older holder sends, and only a reader that
    /// agreed them reads them.
    #[test]
    fn facts_extensions_ride_only_on_a_watch_that_agreed_them() {
        let facts = Facts {
            foreground: Some(Program {
                pid: 7,
                name: "pi".to_owned(),
                package: Some("@scope/pi".to_owned()),
                argv: Vec::new(),
            }),
            ..Facts::default()
        };
        let mut original = Vec::new();
        write_holder(&mut original, &HolderMessage::Facts(facts.clone())).unwrap();
        assert_eq!(
            read_holder(&mut &original[..]).unwrap(),
            HolderMessage::Facts(facts.clone())
        );
        let agreed = [
            Capability::Facts,
            Capability::ProgramPackage,
            Capability::LineEditing,
            Capability::ForegroundUnread,
            Capability::AlternateScreen,
        ];
        for editing in [
            LineEditing::ReadsKeys,
            LineEditing::Edits,
            LineEditing::Unknown,
        ] {
            let sent = Facts {
                line_editing: Some(editing),
                foreground_unread: Some(false),
                alternate_screen: Some(editing == LineEditing::ReadsKeys),
                ..facts.clone()
            };
            let mut bytes = Vec::new();
            write_holder(&mut bytes, &HolderMessage::Facts(sent.clone())).unwrap();
            assert_eq!(bytes.len(), original.len() + 3);
            assert_eq!(
                read_watch_holder(&mut &bytes[..], &agreed).unwrap(),
                HolderMessage::Facts(sent)
            );
        }
        let unread = Facts {
            line_editing: Some(LineEditing::Unknown),
            foreground_unread: Some(true),
            alternate_screen: Some(true),
            ..Facts::default()
        };
        let mut bytes = Vec::new();
        write_holder(&mut bytes, &HolderMessage::Facts(unread.clone())).unwrap();
        assert_eq!(
            read_watch_holder(&mut &bytes[..], &agreed).unwrap(),
            HolderMessage::Facts(unread)
        );
        // A frame that lacks what its watch agreed is broken, not facts that
        // do not know; so is a program the system could not say.
        let mut bare = Vec::new();
        write_holder(&mut bare, &HolderMessage::Facts(Facts::default())).unwrap();
        assert!(read_watch_holder(&mut &bare[..], &agreed).is_err());
        let mut both = Vec::new();
        let forged = Facts {
            line_editing: Some(LineEditing::Edits),
            foreground_unread: Some(true),
            ..facts
        };
        write_holder(&mut both, &HolderMessage::Facts(forged)).unwrap();
        assert!(read_watch_holder(&mut &both[..], &agreed).is_err());
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
            Opening::Ask(Ask::Screen),
            Opening::Ask(Ask::Rows { lines: 2000 }),
            Opening::Ask(Ask::Scroll),
            Opening::Ask(Ask::Visible),
            Opening::Ask(Ask::ScrollTo {
                offset_from_bottom: u64::MAX,
            }),
            Opening::Watch(Hello::current(vec![Capability::Facts])),
        ] {
            assert_eq!(round_opening(opening.clone()), opening);
        }
        for message in [
            WatchMessage::Rules(Some("{\"version\":\"1\"}".to_owned())),
            WatchMessage::Rules(None),
            WatchMessage::Release,
        ] {
            let mut bytes = Vec::new();
            write_watch(&mut bytes, &message).unwrap();
            assert_eq!(read_watch(&mut &bytes[..]).unwrap(), message);
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
                outside: None,
            }),
            ClientMessage::Mouse(Mouse {
                action: MouseAction::Motion,
                button: None,
                col: 0,
                row: 0,
                modifiers: Modifiers::default(),
                outside: None,
            }),
            ClientMessage::Mouse(Mouse {
                action: MouseAction::Motion,
                button: Some(MouseButton::Left),
                col: 4,
                row: 0,
                modifiers: Modifiers::default(),
                outside: Some((4, -3)),
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
            ClientMessage::Key(Key::ALL.to_vec()),
            ClientMessage::Submit(Submit {
                text: vec![
                    SubmitPart::Input(b"\x1b[I".to_vec()),
                    SubmitPart::Paste(b"two\nlines".to_vec()),
                    SubmitPart::Keys(vec![Key::Right]),
                ],
                gap: Duration::from_millis(300),
                enter: SubmitPart::Keys(vec![Key::Enter]),
            }),
            ClientMessage::Submit(Submit {
                text: vec![SubmitPart::Input(b"hello".to_vec())],
                gap: MAX_SUBMIT_GAP,
                enter: SubmitPart::Input(b"\r".to_vec()),
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
                sends: vec![Capability::Screen, Capability::Notices],
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
            HolderMessage::Answer(Answer::Scroll {
                offset_from_bottom: 3,
                max_offset_from_bottom: 40,
            }),
            HolderMessage::Notice(Notice::new(
                NoticeReason::AnswersRefused,
                1,
                70_000,
                "past 64 KiB",
            )),
            HolderMessage::Notice(Notice::new(NoticeReason::AnswersResumed, 12, 96, "")),
            // A newer holder's reason reads as itself, not as a broken frame.
            HolderMessage::Notice(Notice::new(NoticeReason::Other(77), 0, 0, "later")),
            HolderMessage::Clipboard(b"copied \x00 text".to_vec()),
            HolderMessage::Clipboard(vec![b'x'; MAX_CLIPBOARD]),
            HolderMessage::Watching(Watching {
                version: 1,
                rules_engine: 3,
                sends: vec![Capability::Facts],
            }),
            HolderMessage::Facts(Facts::default()),
            HolderMessage::Facts(Facts {
                cwd: Some("file:///Users/me/a%20b".to_owned()),
                title: Some("⠋ task".to_owned()),
                bells: 2,
                prompts: 1,
                bracketed_paste: true,
                foreground: Some(Program {
                    pid: 42,
                    name: "gemini".to_owned(),
                    package: Some("@google/gemini-cli".to_owned()),
                    argv: vec!["node".to_owned(), "/opt/gemini.js".to_owned()],
                }),
                screen: ScreenRead::Read {
                    state: ScreenState::Working,
                    rule: Some("osc_title_working".to_owned()),
                },
                line_editing: None,
                foreground_unread: None,
                alternate_screen: None,
            }),
            HolderMessage::Facts(Facts {
                screen: ScreenRead::Unknown {
                    reason: "rule x could not be compiled".to_owned(),
                },
                ..Facts::default()
            }),
            HolderMessage::Closed(closed),
        ] {
            assert_eq!(round_holder(message.clone()), message);
        }
    }

    #[test]
    fn a_theme_passes_as_a_start_argument() {
        let theme = Theme {
            foreground: Some(Rgb { r: 1, g: 2, b: 3 }),
            background: None,
            cursor: Some(Rgb {
                r: 255,
                g: 0,
                b: 16,
            }),
            palette: (0..=255)
                .map(|index| {
                    (
                        index,
                        Rgb {
                            r: index,
                            g: 0,
                            b: 1,
                        },
                    )
                })
                .collect(),
            appearance: Appearance::Light,
        };
        let arg = theme.to_arg();
        assert!(arg.len() <= 2 * (16 + 256 * 4), "{}", arg.len());
        assert_eq!(Theme::from_arg(&arg), Ok(theme));
        for wrong in ["", "0", "zz", &format!("{arg}00")] {
            assert!(Theme::from_arg(wrong).is_err(), "{wrong:?}");
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
        assert!(Ask::Screen.check().is_ok());
        assert!(
            Ask::Rows {
                lines: MAX_READ_LINES
            }
            .check()
            .is_ok()
        );
        for lines in [0, MAX_READ_LINES + 1] {
            let refused = Ask::Read { lines }.check().unwrap_err();
            assert_eq!(refused.reason, CloseReason::AskInvalid);
            let refused = Ask::Rows { lines }.check().unwrap_err();
            assert_eq!(refused.reason, CloseReason::AskInvalid);
        }
    }

    /// A holder built before an ask was added reads it as a first message
    /// it does not know, which it closes `hello_invalid`.
    #[test]
    fn an_ask_this_holder_does_not_know_is_not_read() {
        let mut payload = MAGIC.to_vec();
        payload.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        payload.push(8);
        let mut bytes = Vec::new();
        write_frame(&mut bytes, kind::ASK, &payload).unwrap();
        assert!(matches!(
            read_opening(&mut &bytes[..]),
            Err(FrameError::Malformed(_))
        ));
    }

    /// A submission without text or a submit would hold the holder's writer
    /// for its gap and count nothing against its queue's cap.
    #[test]
    fn a_submission_without_text_or_a_submit_is_not_written_or_read() {
        for (text, enter) in [
            (
                vec![SubmitPart::Input(Vec::new())],
                SubmitPart::Keys(vec![Key::Enter]),
            ),
            (Vec::new(), SubmitPart::Keys(vec![Key::Enter])),
            (
                vec![SubmitPart::Input(b"hi".to_vec())],
                SubmitPart::Input(Vec::new()),
            ),
        ] {
            let submit = ClientMessage::Submit(Submit {
                text,
                gap: MAX_SUBMIT_GAP,
                enter,
            });
            assert!(write_client(&mut Vec::new(), &submit).is_err());
        }
        // Gap 2000 ms, one empty text part, then an Enter.
        let mut payload = 2000u16.to_le_bytes().to_vec();
        payload.extend([1, 0, 0, 0, 0, 0]);
        payload.extend([2, 1, 0, 0, 0, Key::Enter.code()]);
        let mut forged = Vec::new();
        write_frame(&mut forged, kind::SUBMIT, &payload).unwrap();
        assert!(matches!(
            read_client(&mut &forged[..]),
            Err(FrameError::Malformed(_))
        ));
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
        let copied = HolderMessage::Clipboard(vec![b'x'; MAX_CLIPBOARD + 1]);
        assert!(write_holder(&mut Vec::new(), &copied).is_err());
        let mut forged = Vec::new();
        write_frame(&mut forged, kind::CLIPBOARD, &vec![b'x'; MAX_CLIPBOARD + 1]).unwrap();
        assert!(matches!(
            read_holder(&mut &forged[..]),
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
    fn facts_accept_the_original_frame_and_bound_the_optional_package() {
        let facts = Facts {
            foreground: Some(Program {
                pid: 42,
                name: "cli".to_owned(),
                package: None,
                argv: vec!["node".to_owned()],
            }),
            ..Facts::default()
        };
        let mut original = Vec::new();
        facts.put(&mut original);
        // Version 1 before package support: absent cwd/title, zero clocks
        // and mode, then pid, name, argv and NoRules, with no trailer.
        let mut old_wire = vec![0; 19];
        old_wire.extend_from_slice(&[
            1, 42, 0, 0, 0, 3, 0, b'c', b'l', b'i', 1, 0, 4, 0, b'n', b'o', b'd', b'e', 0,
        ]);
        assert_eq!(original, old_wire);
        let mut payload = Payload(&original);
        assert_eq!(
            Facts::take(&mut payload, Extensions::default()).unwrap(),
            facts
        );
        payload.end().unwrap();

        // This is the extension's wire format, appended after the old
        // frame: a length-prefixed full package name, not another name.
        let mut extended = original.clone();
        put_text(&mut extended, "@scope/tool");
        let mut payload = Payload(&extended);
        let read = Facts::take(&mut payload, Extensions::default()).unwrap();
        let program = read.foreground.unwrap();
        assert_eq!(program.name, "cli");
        assert_eq!(program.package.as_deref(), Some("@scope/tool"));
        payload.end().unwrap();

        put_text(&mut original, &"x".repeat(MAX_PROCESS_NAME + 1));
        assert!(Facts::take(&mut Payload(&original), Extensions::default()).is_err());
        let mut no_program = Vec::new();
        Facts::default().put(&mut no_program);
        put_text(&mut no_program, "@scope/tool");
        assert!(Facts::take(&mut Payload(&no_program), Extensions::default()).is_err());
    }

    #[test]
    fn facts_past_their_bounds_are_cut_on_the_way_out_and_refused_on_the_way_in() {
        let facts = Facts {
            title: Some("t".repeat(MAX_TITLE_CHARS + 10)),
            foreground: Some(Program {
                pid: 1,
                name: "x".to_owned(),
                package: None,
                argv: (0..MAX_ARGS + 5).map(|i| format!("{i:0>200}")).collect(),
            }),
            ..Facts::default()
        };
        let HolderMessage::Facts(read) = round_holder(HolderMessage::Facts(facts)) else {
            panic!("not facts");
        };
        assert_eq!(read.title.unwrap().chars().count(), MAX_TITLE_CHARS);
        let argv = read.foreground.unwrap().argv;
        assert_eq!(argv.len(), MAX_ARGV_BYTES / 200, "the arguments that fit");
        assert!(argv.iter().map(String::len).sum::<usize>() <= MAX_ARGV_BYTES);

        let rules = WatchMessage::Rules(Some("x".repeat(MAX_RULES + 1)));
        assert!(write_watch(&mut Vec::new(), &rules).is_err());
        let mut forged = Vec::new();
        let mut payload = vec![1];
        payload.extend(std::iter::repeat_n(b'x', MAX_RULES + 1));
        write_frame(&mut forged, kind::RULES, &payload).unwrap();
        assert!(read_watch(&mut &forged[..]).is_err());
        // A screen message is no watch message, and a watch is no message
        // after a hello.
        let mut input = Vec::new();
        write_client(&mut input, &ClientMessage::Input(b"x".to_vec())).unwrap();
        assert!(matches!(
            read_watch(&mut &input[..]),
            Err(FrameError::Malformed(_))
        ));
        let mut watch = Vec::new();
        write_opening(&mut watch, &Opening::Watch(Hello::current(Vec::new()))).unwrap();
        assert!(matches!(
            read_client(&mut &watch[..]),
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
