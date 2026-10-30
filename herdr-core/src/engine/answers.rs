//! What the engine answers, in the core's words: the values a call to
//! [`super::Engine`] returns that no other module already names.

/// What `worktree.open` answered: the bound workspace, its active tab and
/// pane, and whether Herdr had it open before this request.
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

/// One agent as Herdr reports it right now, for the check agent sleep makes
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
    pub terminal_id: String,
    pub name: String,
    pub kind: Option<String>,
    pub session: Option<String>,
    pub status: String,
    pub state_change_seq: u64,
    pub readiness: Readiness,
}

/// What Herdr says about whether an agent takes typed input. Herdr reports
/// readiness only for an agent it started itself (`agent.start`): it sends
/// `launch_pending: true` while that start settles or is blocked, and
/// `interactive_ready: true` once it is active, and omits either flag when it
/// is false. An agent the operator started by typing its program in a shell
/// carries neither flag, so its readiness is unreported, not refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Readiness {
    Ready,
    Unreported,
    NotReady,
    Launching,
}

impl Readiness {
    pub(crate) fn accepts_input(self) -> bool {
        matches!(self, Self::Ready | Self::Unreported)
    }

    pub(crate) fn of(interactive_ready: Option<bool>, launch_pending: Option<bool>) -> Self {
        match (interactive_ready, launch_pending) {
            (_, Some(true)) => Self::Launching,
            (Some(false), _) => Self::NotReady,
            (Some(true), _) => Self::Ready,
            (None, _) => Self::Unreported,
        }
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
    /// Herdr's condition for `agent.start`: the foreground group is the
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

/// The session Herdr reports for a pane's agent.
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

/// What a Herdr server says of itself when pinged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerPong {
    pub version: String,
    pub protocol: u32,
    /// Whether it can hand its panes to another Herdr program
    /// (`server.live_handoff`); Herdr does so on Unix only.
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
}

/// Whether a notification showed: `Err` names why the engine showed none.
pub(crate) type Notified = Result<(), &'static str>;
