//! What a node's pane holders do for the core's own engine (PRD
//! engine-bundle-3-core-owns D-3, D-5): start a pane's holder, end it, read
//! it, list its processes and type into it.
//!
//! The core decides which panes exist and where; the node keeps their
//! programs. The core's engine asks the node through [`PaneHolders`] and
//! never names a holder record, socket or protocol. This machine's node
//! answers it in the daemon's own process; a device's node answers it once
//! its link carries the calls (the device bundle).

/// The variable a pane's processes name their pane by, which the node sets
/// in every pane it starts and the CLI, the hook helper and the agent
/// plugins read first (PRD engine-bundle-3-core-owns D-9).
pub use hide_agent_hooks::pane::HIDE_PANE_ID as PANE_ID_VARIABLE;

use serde::{Deserialize, Serialize};

/// One pane to start, under the id the core chose for it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HolderStart {
    /// The pane's id, which is the start's key: the same id asked again
    /// with the same command answers the pane that runs.
    pub pane: String,
    /// The program and its arguments, or the node's interactive shell when
    /// `None`.
    pub program: Option<Vec<String>>,
    /// The folder the program starts in, in the wire spelling
    /// (`hide_platform::path`), which the node turns into its own.
    pub cwd: String,
    /// Variables added to the program's environment.
    pub env: Vec<(String, String)>,
    /// The size the pane starts at, before any screen draws it.
    pub rows: u16,
    pub cols: u16,
    /// One line the pane shows before its shell's first output, such as why
    /// a restored pane opened somewhere else; only a pane started with no
    /// program takes one.
    pub notice: Option<String>,
}

/// A creation request within the recovered node connection, never reused.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HolderRequest {
    pub owner: u64,
    pub number: u64,
}

/// A start's durable identity, computed before shell setup. No command or
/// environment is stored in a pane record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HolderIntent {
    pub pane: String,
    pub digest: String,
    pub cwd: String,
}

impl HolderStart {
    pub fn intent(&self) -> Result<HolderIntent, HolderError> {
        use sha2::{Digest, Sha256};
        let bytes = serde_json::to_vec(self).map_err(|error| HolderError::Refused {
            code: "invalid_start".to_owned(),
            message: error.to_string(),
        })?;
        Ok(HolderIntent {
            pane: self.pane.clone(),
            cwd: self.cwd.clone(),
            digest: format!("{:x}", Sha256::digest(bytes)),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HolderReservation {
    pub incarnation: u64,
}

/// An interrupted existence record whose original process is gone. The
/// replacement must consume this exact incarnation under the same pane ID.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoverablePane {
    pub pane: String,
    /// Legacy records have no recovery folder; only a saved layout can
    /// supply their place and folder.
    pub cwd: Option<String>,
    pub incarnation: u64,
}

/// Told the id of a pane whose program exited on its own
/// ([`PaneHolders::watch_ends`]).
pub type PaneEnded = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// What a node found of its panes' holders when it started (PRD
/// engine-bundle-3-core-owns D-7), under the core's pane ids, every pane it
/// holds a record of in exactly one list.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HolderRecovery {
    pub owner: u64,
    /// Panes that finished while no node ran: their program exited or a node
    /// ended them, so they close, as a pane whose program exits closes.
    pub closed: Vec<String>,
    /// Panes whose holders ended under them while no node ran (the machine
    /// restarted, the holder was lost or failed, a start never finished),
    /// so a saved layout opens a new shell in their place.
    pub ended: Vec<String>,
    /// Panes whose holders run or may still run, never started a second
    /// time.
    pub present: Vec<String>,
    /// A subset of `ended`, including records not yet saved in the layout.
    pub recoverable: Vec<RecoverablePane>,
}

/// Where a pane's screen is in its scrollback, in rows above the bottom
/// (Herdr's `pane.get` scroll metrics).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HolderScroll {
    pub offset_from_bottom: u64,
    pub max_offset_from_bottom: u64,
}

/// The processes a pane's terminal holds: its program, which is the shell
/// of a shell pane, and the foreground process group with its members'
/// names, or none when the system cannot say.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HolderProcesses {
    pub shell: u32,
    pub foreground: Option<HolderForeground>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HolderForeground {
    pub group: u32,
    /// Each member's pid and name, in the order the holder listed them.
    pub members: Vec<(u32, String)>,
    /// The folders the members other than the pane's shell work in, as the
    /// pane's machine observes them (`hide_platform::process::observed_cwd_of`),
    /// in the wire spelling, each once; `None` when it could not say for one
    /// of them, or the node predates this field, which a caller guarding a
    /// folder takes as unknown, never as no folder.
    #[serde(default)]
    pub folders: Option<Vec<String>>,
}

/// A key the engine presses by name, which the pane's holder encodes for
/// its program's keyboard modes when it is written.
pub use hide_holder_protocol::Key as HolderKey;

/// What the engine types into a pane (PRD engine-bundle-3-core-owns D-21):
/// bytes as they are, or a paste and named keys the pane's holder encodes
/// for its program's modes when they are written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "input", rename_all = "snake_case")]
pub enum PaneInput {
    Bytes(#[serde(with = "base64_bytes")] Vec<u8>),
    Paste(#[serde(with = "base64_bytes")] Vec<u8>),
    Keys(Vec<HolderKey>),
}

/// A pane's start and program as its record names them
/// ([`PaneHolders::proof`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HolderProof {
    /// The start of the pane the record is, never reused by another start.
    pub incarnation: u64,
    /// The pane's program, which leads its terminal's session: the shell of
    /// a shell pane.
    pub program: u32,
    /// When the program started, as the system's process start time reads,
    /// so a pid the system gave another process since is told apart.
    pub started: u64,
}

/// What a pane's holder says of its pane now (PRD engine-bundle-3-core-owns
/// D-18, D-21), whole each time any of it changed, so a watcher that missed
/// one is put right by the next. The holder sends it on a connection of its
/// own that takes nothing over and needs no screen, so a pane no screen
/// shows is followed as one that is shown.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaneFacts {
    /// The folder the pane's shell last reported (OSC 7, or OSC 9;9 from
    /// Windows PowerShell's prompt), in the wire spelling; `None` until it
    /// reported one.
    pub cwd: Option<String>,
    /// The terminal's title (OSC 0 or 2), `None` when none was set or the
    /// last one was empty.
    pub title: Option<String>,
    /// The bells the program rang since the holder started.
    pub bells: u64,
    /// The prompts the shell marked (OSC 133;A) since the holder started:
    /// Hide's shell integration marks its first one once the user's own
    /// startup files are done.
    pub prompts: u64,
    /// Whether the program has bracketed paste on (mode 2004), so a text
    /// typed for it as one paste is wrapped as Herdr's `agent.prompt` wraps
    /// it.
    pub bracketed_paste: bool,
    /// The program in the terminal's foreground other than the pane's own
    /// program, `None` while the pane's program holds the terminal or the
    /// system cannot say.
    pub foreground: Option<ForegroundProgram>,
    /// What the pane's screen rules read on its screen.
    pub screen: ScreenRead,
}

/// The program in a pane's foreground, as an agent definition's
/// `executables` recognize it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ForegroundProgram {
    pub pid: u32,
    /// The program's name without its folder or `.exe`; for a runtime that
    /// runs a script (node, bun, deno, python), the script's name without
    /// its folder or extension, as Herdr recognizes a wrapped agent.
    pub name: String,
    /// The installed package, when the holder can identify it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Its arguments, the program first; empty when the system cannot
    /// read them. At most
    /// [`MAX_FOREGROUND_ARGS`] of at most [`MAX_FOREGROUND_ARGV_BYTES`] in
    /// all, as Herdr bounds a resume's arguments.
    pub argv: Vec<String>,
}

/// The most arguments a [`ForegroundProgram`] carries.
pub const MAX_FOREGROUND_ARGS: usize = 64;
/// The most bytes a [`ForegroundProgram`]'s arguments carry in all.
pub const MAX_FOREGROUND_ARGV_BYTES: usize = 8 * 1024;

/// A state a pane's screen rules can read.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenState {
    Idle,
    Working,
    Blocked,
    Unknown,
}

/// What a pane's screen rules read, or why they read nothing.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "read", rename_all = "snake_case")]
pub enum ScreenRead {
    /// The pane was given no rules.
    #[default]
    NoRules,
    /// The state of the highest-priority rule that matched the screen, or
    /// the last such state when the latest match says the screen proves
    /// nothing; `Unknown` when no rule matched.
    Read {
        state: ScreenState,
        rule: Option<String>,
    },
    /// The rules cannot be evaluated on this pane, under a stable code:
    /// [`UNSUPPORTED_HOLDER`], [`NO_RULES_FOR_HOLDER`], [`WATCH_LOST`], or the holder's own
    /// refusal of the rules it was given.
    Unknown { reason: String },
}

/// The pane's holder was built before pane facts: its screen is read by no
/// rules, and its folder, title and foreground are not followed.
pub const UNSUPPORTED_HOLDER: &str = "unsupported_holder";
/// Every rule set for the pane's agent needs a newer evaluator than its
/// holder has.
pub const NO_RULES_FOR_HOLDER: &str = "no_rules_for_holder";
/// The pane's facts are no longer followed: its watch ended some other way
/// than with the pane (another watch took it, or the connection broke).
pub const WATCH_LOST: &str = "watch_lost";

/// One agent's screen rule sets for a pane, highest priority first (the
/// operator's file, then the built-in one, PRD D-20): the node sends its
/// holder the first one the holder's evaluator can run (its
/// `min_engine_version` at most the holder's); when none fits, the pane's
/// screen reads [`NO_RULES_FOR_HOLDER`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScreenRules {
    /// The agent definition's id, for the logs.
    pub agent: String,
    pub sets: Vec<RuleSet>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    /// Where it came from, for the logs: `built-in` or the operator's file.
    pub source: String,
    pub version: String,
    pub min_engine_version: u32,
    /// The rule set as `hide_screen_rules::Manifest` serializes to JSON.
    pub manifest: String,
}

/// A read of a pane's screen with its styling kept, as escape sequences
/// ([`PaneHolders::read_styled`]), as Herdr's `pane.read --format ansi`
/// answers it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "read", content = "rows", rename_all = "snake_case")]
pub enum StyledRead {
    /// The rows the screen shows now (Herdr's `visible` source), which an
    /// agent's composer is judged on.
    Screen,
    /// The last rows of the screen and its scrollback as the terminal drew
    /// them, soft wraps kept (Herdr's `recent` source), at most this many.
    Rows(u16),
}

/// Told each pane's id and its facts ([`PaneHolders::watch_facts`]).
pub type PaneFactsSink = std::sync::Arc<dyn Fn(&str, &PaneFacts) + Send + Sync>;

/// Why a holder call did not do what was asked. A caller branches on the
/// kind and shows the message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum HolderError {
    /// The node has no such pane: never started, or ended and forgotten.
    NoPane { message: String },
    /// The node refused, under a stable code: the cap of holders, a pane
    /// started with another command, a holder that could not start.
    Refused { code: String, message: String },
    /// The holder could not be reached or did not answer, or the node's link
    /// did not carry the call.
    Unreachable { message: String },
}

impl std::fmt::Display for HolderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPane { message } | Self::Unreachable { message } => {
                formatter.write_str(message)
            }
            Self::Refused { code, message } => write!(formatter, "{code}: {message}"),
        }
    }
}

impl std::error::Error for HolderError {}

/// A node's pane holders as the core's own engine asks them. Every call
/// blocks until the node answers, so none is made under the runtime lock.
pub trait PaneHolders: Send + Sync {
    /// Node-side binding for one connection. The returned adapter retains
    /// its state-folder ownership guard, including while a timed-out call
    /// still runs. Only the host's connection slot calls this method.
    fn own(
        self: std::sync::Arc<Self>,
        _within: std::time::Duration,
    ) -> Result<std::sync::Arc<dyn PaneHolders>, HolderError> {
        Err(HolderError::Refused {
            code: "pane_ownership_unsupported".to_owned(),
            message: "this node cannot establish pane ownership".to_owned(),
        })
    }
    /// Issue locally before submission; a definitely unsent request must
    /// also be retired, so a later number cannot strand its gap.
    fn request(&self) -> Result<HolderRequest, HolderError> {
        Err(reservation_unsupported())
    }
    fn retire(&self, _request: HolderRequest) -> Result<(), HolderError> {
        Err(reservation_unsupported())
    }
    /// Persist existence before publication. A restart must explicitly name
    /// the previous incarnation; ordinary retries pass the same intent.
    fn reserve(
        &self,
        _start: &HolderStart,
        _previous: Option<u64>,
        _request: HolderRequest,
    ) -> Result<HolderReservation, HolderError> {
        Err(reservation_unsupported())
    }
    fn start_reserved(
        &self,
        _start: &HolderStart,
        _reserved: HolderReservation,
        _request: HolderRequest,
    ) -> Result<(), HolderError> {
        Err(reservation_unsupported())
    }
    fn cancel(&self, _intent: &HolderIntent, _request: HolderRequest) -> Result<(), HolderError> {
        Err(reservation_unsupported())
    }
    /// How a pane started with no program runs its shell, when it runs one
    /// with Hide's shell integration, which marks its first prompt on its
    /// screen: what a start into such a pane waits for. `None` for a shell
    /// that marks nothing.
    fn shell_launch(&self) -> Option<crate::protocol::ShellLaunch> {
        None
    }
    /// Ends the pane, its program and everything it started, then its
    /// holder.
    fn terminate(&self, pane: &str) -> Result<(), HolderError>;
    /// The pane's last `lines` rows as text, soft wraps joined, and whether
    /// rows were left out.
    fn read(&self, pane: &str, lines: u16) -> Result<(String, bool), HolderError>;
    /// The pane's screen or last rows with their styling, and whether rows
    /// were left out; a holder built before styled reads is refused
    /// [`UNSUPPORTED_HOLDER`].
    fn read_styled(&self, pane: &str, read: StyledRead) -> Result<(String, bool), HolderError>;
    /// The rows the pane's screen shows now, wherever it is scrolled to,
    /// as text with soft wraps joined (Herdr's `visible` read); a holder
    /// built before visible reads is refused [`UNSUPPORTED_HOLDER`].
    fn read_visible(&self, pane: &str) -> Result<String, HolderError>;
    /// Where the pane's screen is in its scrollback, after moving it to
    /// `to` rows above the bottom when asked (Herdr's `pane.get` and
    /// `pane.scroll`); the connection that holds the screen is sent the
    /// moved screen. A holder built before scroll asks is refused
    /// [`UNSUPPORTED_HOLDER`].
    fn scroll(&self, pane: &str, to: Option<u64>) -> Result<HolderScroll, HolderError>;
    /// The pane's program and its terminal's foreground processes, waiting
    /// at most `within` for them, so a caller that reads many panes shares
    /// one deadline among them.
    fn processes(
        &self,
        pane: &str,
        within: std::time::Duration,
    ) -> Result<HolderProcesses, HolderError>;
    /// Which start of the pane runs now and the program it runs, as the
    /// pane's record says (PRD engine-bundle-3-core-owns D-9): what a
    /// process that names the pane is proved against. Read from the record
    /// alone, never from the holder, so a holder that does not answer cannot
    /// vouch for anything; a pane whose program has not started or has ended
    /// is refused `pane_not_running`.
    fn proof(&self, pane: &str) -> Result<HolderProof, HolderError>;
    /// Types `input` into the pane through the connection that holds its
    /// screen, in order with every other key typed there; a pane whose
    /// screen was let go is attached again for it. It is the engine's
    /// typing, never reported as the operator's.
    fn input(&self, pane: &str, input: PaneInput) -> Result<(), HolderError>;
    /// Types a letter: `body`, then `submit` once `gap` has passed, holding
    /// the operator's keys typed into the pane meanwhile and writing them
    /// after the submit in the order typed (PRD engine-bundle-3-core-owns
    /// D-13). Returns once the submit was handed over.
    fn letter(
        &self,
        pane: &str,
        body: PaneInput,
        submit: PaneInput,
        gap: std::time::Duration,
    ) -> Result<(), HolderError>;
    /// What the node's start found of its holders, once it has looked at
    /// every record, waiting at most `timeout` for it: refused
    /// `recovery_unfinished` when it has not finished by then.
    fn recovered(&self, timeout: std::time::Duration) -> Result<HolderRecovery, HolderError>;
    /// Tells `ended`, from now on, the id of each pane whose program exits
    /// on its own while the node holds its screen, so the engine closes it
    /// as Herdr closes a pane whose program ends. A pane ended by
    /// [`PaneHolders::terminate`] is not told.
    fn watch_ends(&self, ended: PaneEnded);
    /// Tells `sink`, from now on, each pane's facts whenever they change,
    /// for every pane the node starts or found running, whether or not a
    /// screen shows it; a pane whose holder does not send facts is told once
    /// with [`ScreenRead::Unknown`] and [`UNSUPPORTED_HOLDER`].
    fn watch_facts(&self, sink: PaneFactsSink);
    /// Gives the pane's holder `rules` to read its screen by, in place of
    /// any it had, or none; kept by the node and given again to a holder it
    /// reaches anew.
    fn set_rules(&self, pane: &str, rules: Option<ScreenRules>) -> Result<(), HolderError>;
}

/// One thing the core's engine asks a node's holders, by the node link
/// (PRD engine-bundle-3-core-owns D-22): each [`PaneHolders`] request as it
/// crosses [`crate::protocol::Call::Pane`], whichever machine the node is.
/// The answer is a [`PaneAnswer`] of the shape the request names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "snake_case")]
pub enum PaneCall {
    /// [`PaneHolders::reserve`]; answers the reservation.
    Reserve {
        start: HolderStart,
        previous: Option<u64>,
        request: HolderRequest,
    },
    StartReserved {
        start: HolderStart,
        reserved: HolderReservation,
        request: HolderRequest,
    },
    Cancel {
        intent: HolderIntent,
        request: HolderRequest,
    },
    Retire {
        request: HolderRequest,
    },
    /// [`PaneHolders::shell_launch`]; answers an optional
    /// [`crate::protocol::ShellLaunch`].
    ShellLaunch,
    /// [`PaneHolders::terminate`]; answers nothing.
    Terminate {
        pane: String,
    },
    /// [`PaneHolders::read`]; answers a [`PaneText`].
    Read {
        pane: String,
        lines: u16,
    },
    /// [`PaneHolders::read_styled`]; answers a [`PaneText`].
    ReadStyled {
        pane: String,
        read: StyledRead,
    },
    /// [`PaneHolders::read_visible`]; answers a [`PaneText`].
    ReadVisible {
        pane: String,
    },
    /// [`PaneHolders::scroll`]; answers a [`HolderScroll`].
    Scroll {
        pane: String,
        to: Option<u64>,
    },
    /// [`PaneHolders::processes`]; answers [`HolderProcesses`].
    Processes {
        pane: String,
    },
    /// [`PaneHolders::proof`]; answers a [`HolderProof`].
    Proof {
        pane: String,
    },
    /// [`PaneHolders::input`]; answers nothing.
    Input {
        pane: String,
        input: PaneInput,
    },
    /// [`PaneHolders::letter`]; answers nothing once the submit was handed
    /// over.
    Letter {
        pane: String,
        body: PaneInput,
        submit: PaneInput,
        gap_ms: u64,
    },
    /// [`PaneHolders::recovered`]; answers [`HolderRecovery`].
    Recovered {
        timeout_ms: u64,
    },
    /// [`PaneHolders::set_rules`]; answers nothing.
    SetRules {
        pane: String,
        rules: Option<ScreenRules>,
    },
}

/// A pane's text as a read answers it, and whether rows were left out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneText {
    pub text: String,
    pub truncated: bool,
}

/// What a node answers a [`PaneCall`] with: the request's answer, or the
/// holders' own refusal, which keeps its kind and code across the link.
pub type PaneAnswer = Result<serde_json::Value, HolderError>;

/// What a node's pane watch ([`crate::protocol::Call::PaneWatch`]) reports,
/// until the core answers a report with false: each pane's facts whenever
/// they change, each pane whose program exited on its own, and, once a
/// second with nothing else to say, that it still watches, so a core that
/// no longer listens is heard within a second.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "report", rename_all = "snake_case")]
pub enum PaneWatchReport {
    Facts { pane: String, facts: PaneFacts },
    Ended { pane: String },
    Quiet,
}

/// How many pane requests ([`crate::protocol::Call::is_pane`]) a node works
/// on at once, and the core admits per node: a restore's eight starts, and
/// the one pane watch beside them.
pub const PANE_LANE: usize = 9;

/// How often a quiet pane watch says it still watches.
pub const PANE_WATCH_QUIET: std::time::Duration = std::time::Duration::from_secs(1);

/// The most reports a node's pane watch keeps waiting for its core: one
/// facts per pane and the panes that ended, for the node's cap of holders.
/// Past it the oldest facts go first (the pane's next facts put it right);
/// an end is never dropped while a facts report can make room.
pub const PANE_WATCH_CAP: usize = 4 * 128;

/// Bytes as base64 text in a JSON line.
mod base64_bytes {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(serde::de::Error::custom)
    }
}

fn reservation_unsupported() -> HolderError {
    HolderError::Refused {
        code: "pane_reservation_unsupported".to_owned(),
        message: "this node cannot reserve a pane before publication".to_owned(),
    }
}
