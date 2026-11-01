//! What a node's pane holders do for the core's own engine (PRD
//! engine-bundle-3-core-owns D-3, D-5): start a pane's holder, end it, read
//! it, list its processes and type into it.
//!
//! The core decides which panes exist and where; the node keeps their
//! programs. The core's engine asks the node through [`PaneHolders`] and
//! never names a holder record, socket or protocol. This machine's node
//! answers it in the daemon's own process; a device's node answers it once
//! its link carries the calls (the device bundle). Nothing here runs unless
//! the development switch `HIDE_DEV_PANE_HOLDER` is on.

/// One pane to start, under the id the core chose for it.
#[derive(Clone, Debug, Eq, PartialEq)]
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

/// What a node found of its panes' holders when it started (PRD
/// engine-bundle-3-core-owns D-7), under the core's pane ids: the panes whose
/// holders ended while no node ran (their program exited, or the machine
/// restarted), and every other pane it holds a record of, whose holder runs
/// or may still run and is never started a second time.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HolderRecovery {
    pub ended: Vec<String>,
    pub present: Vec<String>,
}

/// The processes a pane's terminal holds: its program, which is the shell
/// of a shell pane, and the foreground process group with its members'
/// names, or none when the system cannot say.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HolderProcesses {
    pub shell: u32,
    pub foreground: Option<HolderForeground>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HolderForeground {
    pub group: u32,
    /// Each member's pid and name, in the order the holder listed them.
    pub members: Vec<(u32, String)>,
}

/// Why a holder call did not do what was asked. A caller branches on the
/// kind and shows the message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HolderError {
    /// The node has no such pane: never started, or ended and forgotten.
    NoPane(String),
    /// The node refused, under a stable code: the cap of holders, a pane
    /// started with another command, a holder that could not start.
    Refused { code: &'static str, message: String },
    /// The holder could not be reached or did not answer.
    Unreachable(String),
}

impl std::fmt::Display for HolderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPane(message) | Self::Unreachable(message) => formatter.write_str(message),
            Self::Refused { code, message } => write!(formatter, "{code}: {message}"),
        }
    }
}

impl std::error::Error for HolderError {}

/// A node's pane holders as the core's own engine asks them. Every call
/// blocks until the node answers, so none is made under the runtime lock.
pub trait PaneHolders: Send + Sync {
    /// Starts `start`'s pane and attaches its screen to the node's terminal
    /// service. Asked again for a pane that lives, it answers that pane and
    /// starts no second holder.
    fn start(&self, start: &HolderStart) -> Result<(), HolderError>;
    /// Ends the pane, its program and everything it started, then its
    /// holder.
    fn terminate(&self, pane: &str) -> Result<(), HolderError>;
    /// The pane's last `lines` rows as text, soft wraps joined, and whether
    /// rows were left out.
    fn read(&self, pane: &str, lines: u16) -> Result<(String, bool), HolderError>;
    /// The pane's program and its terminal's foreground processes.
    fn processes(&self, pane: &str) -> Result<HolderProcesses, HolderError>;
    /// Types `bytes` into the pane as they are, through the connection that
    /// holds its screen, in order with every other key typed there.
    fn input(&self, pane: &str, bytes: Vec<u8>) -> Result<(), HolderError>;
    /// What the node's start found of its holders, once it has looked at
    /// every record, waiting at most `timeout` for it: refused
    /// `recovery_unfinished` when it has not finished by then.
    fn recovered(&self, timeout: std::time::Duration) -> Result<HolderRecovery, HolderError>;
}
