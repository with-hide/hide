//! What the engine answers, in the core's words: the values a call to
//! [`super::Engine`] returns that no other module already names.

/// What `worktree.open` answered: the bound workspace, its active tab and
/// pane, and whether the engine had it open before this request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenedWorktree {
    pub(crate) tab_label: String,
    pub(crate) workspace_id: String,
    pub(crate) label: String,
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
    pub(crate) already_open: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PaneJoinResult {
    pub(crate) changed: bool,
    pub(crate) reason: Option<String>,
    pub(crate) workspace_id: String,
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
    pub(crate) terminal_id: String,
    pub(crate) previous_workspace_id: String,
    pub(crate) previous_tab_id: String,
    pub(crate) previous_pane_id: String,
    pub(crate) closed_tab_id: Option<String>,
    pub(crate) closed_workspace_id: Option<String>,
    /// The tab a move to a new tab made, when it made one.
    pub(crate) created_tab_id: Option<String>,
}

impl PaneJoinResult {
    /// The tab a move to a new tab made: a move the engine declined fails
    /// with its reason, and so does an answer that names no new tab.
    pub(crate) fn created_tab(&self) -> Result<&str, String> {
        if !self.changed {
            return Err(match &self.reason {
                Some(reason) => format!("The engine declined the move: {reason}"),
                None => "The engine declined the move".to_owned(),
            });
        }
        self.created_tab_id
            .as_deref()
            .filter(|tab| !tab.is_empty())
            .ok_or_else(|| "pane.move response is missing created_tab.tab_id".to_owned())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CreatedWorktree {
    pub tab_label: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub path: String,
    pub branch: Option<String>,
}

/// One agent as the engine reports it right now, for the check agent sleep makes
/// just before it ends the agent's process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentState {
    pub(crate) agent: Option<String>,
    pub(crate) status: String,
    pub(crate) state_change_seq: u64,
}

/// Fresh delivery guard, converted here rather than exposing generated types.
pub(crate) struct DeliveryAgent {
    pub pane_id: String,
    pub name: String,
    pub kind: Option<String>,
    pub session: Option<String>,
    pub status: String,
    pub state_change_seq: u64,
    pub readiness: Readiness,
}

/// What the engine says about whether an agent takes typed input at the
/// moment it is asked, for every agent it lists, started by the engine or
/// typed into a shell (the own engine's `PaneAgent::readiness`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Readiness {
    /// What is typed reaches the program.
    Ready,
    /// The program does not read keys, or the screen it drew since it took
    /// them or since its question was answered is still settling.
    NotReady,
    /// A start the engine typed has not settled, nor reached its deadline,
    /// or its screen asks a question.
    Launching,
}

impl Readiness {
    pub(crate) fn accepts_input(self) -> bool {
        self == Self::Ready
    }
}

/// Which process group holds a pane's terminal, which is its shell's, and
/// the processes in the foreground group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PaneProcessGroup {
    pub(crate) shell_pid: Option<u32>,
    pub(crate) foreground_process_group_id: Option<u32>,
    pub(crate) foreground_pids: Vec<u32>,
    /// The foreground processes' names, in the order of `foreground_pids`.
    pub(crate) foreground_names: Vec<String>,
}

impl PaneProcessGroup {
    /// The engine's condition for `agent.start`: the foreground group is the
    /// shell's own and holds nothing but the shell.
    pub(crate) fn shell_holds_terminal(&self) -> bool {
        match self.shell_pid {
            Some(shell) if shell > 1 => {
                self.foreground_process_group_id == Some(shell)
                    && self.foreground_pids.iter().all(|pid| *pid == shell)
            }
            _ => false,
        }
    }

    /// The shell's own program name, read while it is in the foreground.
    pub(crate) fn shell_name(&self) -> Option<&str> {
        let shell = self.shell_pid?;
        self.foreground_pids
            .iter()
            .zip(&self.foreground_names)
            .find(|(pid, _)| **pid == shell)
            .map(|(_, name)| name.trim_start_matches('-'))
    }

    /// The first foreground process that is not the pane's shell.
    pub(crate) fn foreground_program(&self) -> Option<&str> {
        self.foreground_pids
            .iter()
            .zip(&self.foreground_names)
            .find(|(pid, _)| Some(**pid) != self.shell_pid)
            .map(|(_, name)| name.as_str())
    }
}

/// How a pane's agent session is named: by id, or by the path of its file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneSessionKind {
    Id,
    Path,
}

/// The session the engine reports for a pane's agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneSession {
    pub agent: String,
    pub kind: PaneSessionKind,
    pub value: String,
}

/// A pane's folders exactly as the engine reports them: where it was
/// started and where its foreground program is now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PanePlace {
    pub(crate) pane_id: String,
    pub(crate) cwd: Option<String>,
    pub(crate) foreground_cwd: Option<String>,
}

/// A pane's text as the engine read it, and whether it cut the text short.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneText {
    pub text: String,
    pub truncated: bool,
}

/// What an engine says of itself when pinged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerPong {
    pub version: String,
    pub protocol: u32,
    /// Whether it can hand its panes to another engine program
    /// (`server.live_handoff`); Herdr did so on Unix only, and the own engine
    /// does not.
    pub live_handoff: bool,
}

/// One agent to start in a pane's shell: the name the engine knows it by,
/// its kind, the arguments after its program, and how a Codex start reaches
/// its daemon.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentLaunch {
    pub(crate) pane_id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) args: Vec<String>,
    pub(crate) codex_daemon: crate::codex_launch::CodexDaemon,
    /// The start wakes a sleeping session: an agent whose integration
    /// reports its session only once it has loaded it
    /// (`hide_agent_adapter::SessionFormat::reports_session_once_loaded`)
    /// is then known by that report, not by the id its resume names, so
    /// nothing is typed to it before it takes keys. A spawn that resumes is
    /// known by its resume at once, as its caller asked.
    pub(crate) wakes: bool,
    /// The file the start runs in place of the definition's program name,
    /// as the pane's node read it (`StartForm::Direct`); `None` starts the
    /// program by its name.
    pub(crate) program: Option<StartProgram>,
}

/// A program a start names by its file, and the words before its own
/// arguments: npm's Windows launcher read as the program it runs
/// (`hide_platform::programs::Start`). `name` is the program name the node
/// was asked about, so a start whose definition names another program runs
/// that one by its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartProgram {
    pub(crate) name: String,
    pub(crate) program: String,
    pub(crate) leading: Vec<String>,
}

impl StartProgram {
    /// The file a node's start form names, if it names one.
    pub(crate) fn of(name: &str, form: &hide_node_link::protocol::StartForm) -> Option<Self> {
        match form {
            hide_node_link::protocol::StartForm::Direct { program, leading } => Some(Self {
                name: name.to_owned(),
                program: program.clone(),
                leading: leading.clone(),
            }),
            _ => None,
        }
    }
}

/// A start its engine accepted to send ([`super::Engine::prepare_start`]),
/// with how many bytes its line types into the shell.
#[derive(Clone, Debug)]
pub(crate) struct PreparedStart {
    launch: AgentLaunch,
    line: usize,
}

impl PreparedStart {
    pub(in crate::engine) fn new(launch: AgentLaunch, line: usize) -> Self {
        Self { launch, line }
    }

    pub(crate) fn launch(&self) -> &AgentLaunch {
        &self.launch
    }

    /// How many bytes the start types into the shell.
    pub(crate) fn line(&self) -> usize {
        self.line
    }
}

/// What a start answered: the agent's pane, and the terminal it runs in
/// when the engine named one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StartedAgent {
    pub(crate) pane_id: String,
    pub(crate) terminal_id: Option<String>,
}

/// Whose pane an inherited caller reference names now, with its shell and
/// terminal: what a pane's process is checked against before it gets a
/// credential.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneProof {
    pub pane_id: String,
    pub terminal_id: String,
    pub shell_pid: i32,
    /// When the engine recorded the shell starting, as the system's process
    /// start time reads: the core's own engine keeps it in the pane's record,
    /// so a shell pid the system gave another process is refused. `None`
    /// from an engine that names only the pid.
    pub shell_started: Option<u64>,
}

/// Whether a notification showed: `Err` names why the engine showed none.
pub(crate) type Notified = Result<(), &'static str>;
