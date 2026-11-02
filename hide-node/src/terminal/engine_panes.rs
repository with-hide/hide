//! This machine's pane holders as the core's own engine asks them (PRD
//! engine-bundle-3-core-owns D-3, D-5): [`LocalPanes`] answers
//! [`PaneHolders`] over [`Holders`] and the terminal service whose screens
//! they attach to. A pane started with no program runs the account's shell
//! with Hide's shell integration ([`PaneShell::Account`]), as a tab Hide
//! opens does, so it says its folder and marks its prompt on its screen.
//!
//! Only a daemon with the development switch on builds one, beside the
//! holders [`Holders::start_local`] opened; with it off nothing here runs.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use hide_host::shell_integration::{ShellLaunch, ShellReadiness, UserShell};

use super::Service;
use super::holder::Holders;
use super::input::Typed;
use hide_holder_protocol::record::Phase;
use hide_holder_protocol::{Answer, Appearance, CloseReason, Foreground, OpenError, Size, Theme};
use hide_kit::panes::{CloseError, StartRefusal, StartRequest, Started};
use hide_node_link::holders::{
    HolderError, HolderForeground, HolderProcesses, HolderProof, HolderRecovery, HolderStart,
    PaneEnded, PaneFactsSink, PaneHolders, PaneInput, ScreenRules, StyledRead, UNSUPPORTED_HOLDER,
};

/// This machine's holders and the service that shows their screens.
pub struct LocalPanes {
    holders: Arc<Holders>,
    service: Arc<Service>,
    shell: PaneShell,
}

/// How a pane started with no program runs this machine's shell.
pub enum PaneShell {
    /// The account's shell with Hide's shell integration, prepared for each
    /// pane as the node prepares it for a tab Hide opens
    /// (`hide_host::shell_integration::prepare`): a pane on the core's own
    /// engine has no Herdr to read its folder from its process, so its shell
    /// says it on the screen.
    Account { home: PathBuf, user: UserShell },
    /// This command line, with no integration (the engine contract's).
    Fixed(Vec<OsString>),
}

impl PaneShell {
    fn launch(&self) -> Result<ShellLaunch, HolderError> {
        match self {
            Self::Fixed(command) => Ok(ShellLaunch {
                command: command
                    .iter()
                    .map(|word| word.to_string_lossy().into_owned())
                    .collect(),
                readiness: ShellReadiness::Plain,
                env: Default::default(),
            }),
            Self::Account { home, user } => {
                let executable = std::env::current_exe().ok();
                hide_host::shell_integration::prepare(Some(home), user, executable.as_deref())
                    .map_err(|error| HolderError::Refused {
                        code: "shell_unavailable",
                        message: format!("the account's shell could not be prepared: {error}"),
                    })
            }
        }
    }
}

impl LocalPanes {
    /// The panes of `holders`, attached to `service`; a pane started with no
    /// program runs `shell`.
    pub fn new(holders: Arc<Holders>, service: Arc<Service>, shell: PaneShell) -> Self {
        Self {
            holders,
            service,
            shell,
        }
    }
}

impl PaneHolders for LocalPanes {
    fn start(&self, start: &HolderStart) -> Result<(), HolderError> {
        let size = Size::checked(start.rows, start.cols).map_err(|why| HolderError::Refused {
            code: "invalid_size",
            message: why,
        })?;
        let mut env: Vec<(OsString, OsString)> = Vec::new();
        let program = match (&start.program, &start.notice) {
            (Some(_), Some(_)) => {
                return Err(HolderError::Refused {
                    code: "invalid_program",
                    message: "only a shell pane shows a notice before it starts".to_owned(),
                });
            }
            (Some(program), None) if !program.is_empty() => {
                program.iter().map(OsString::from).collect()
            }
            (Some(_), None) => {
                return Err(HolderError::Refused {
                    code: "invalid_program",
                    message: "the pane's program is empty".to_owned(),
                });
            }
            (None, notice) => {
                let launch = self.shell.launch()?;
                env.extend(
                    launch
                        .env
                        .into_iter()
                        .map(|(name, value)| (OsString::from(name), OsString::from(value))),
                );
                let shell: Vec<OsString> = launch.command.into_iter().map(OsString::from).collect();
                match notice {
                    None => shell,
                    Some(notice) => shell_after_notice(&shell, notice),
                }
            }
        };
        let cwd =
            hide_platform::path::from_wire(&start.cwd).map_err(|error| HolderError::Refused {
                code: "invalid_cwd",
                message: format!("{} is not a folder path: {error}", start.cwd),
            })?;
        let request = StartRequest {
            pane: start.pane.clone(),
            program,
            cwd,
            // The shell's own variables, then the caller's, then the pane's
            // own name for itself, which no caller's variable replaces (PRD
            // engine-bundle-3-core-owns D-9): the holder removes every
            // Herdr variable, so a process in the pane names this pane alone.
            env: env
                .into_iter()
                .chain(
                    start
                        .env
                        .iter()
                        .map(|(name, value)| (OsString::from(name), OsString::from(value))),
                )
                .chain([(
                    OsString::from(hide_node_link::holders::PANE_ID_VARIABLE),
                    OsString::from(&start.pane),
                )])
                .collect(),
            size,
            // The screen's colors reach a holder by a theme message once a
            // screen draws it; until then its color answers are the xterm
            // defaults.
            theme: Theme {
                foreground: None,
                background: None,
                cursor: None,
                palette: Vec::new(),
                appearance: Appearance::Unknown,
            },
        };
        match self.holders.start(&self.service, &request) {
            Ok(Started::Started(record) | Started::Existing(record)) => match record.end {
                None => Ok(()),
                Some(end) => Err(HolderError::Refused {
                    code: "pane_ended",
                    message: format!("pane {} ended ({})", start.pane, end.reason.name()),
                }),
            },
            Err(refusal) => Err(start_refusal(refusal)),
        }
    }

    fn shell_launch(&self) -> Option<ShellLaunch> {
        self.shell
            .launch()
            .ok()
            .filter(|launch| launch.readiness == ShellReadiness::Integrated)
    }

    fn terminate(&self, pane: &str) -> Result<(), HolderError> {
        match self.holders.terminate(&self.service, pane) {
            Ok(_) => Ok(()),
            Err(CloseError::NoRecord) => Err(HolderError::NoPane(format!(
                "pane {pane} has no holder on this machine"
            ))),
            Err(error) => Err(HolderError::Refused {
                code: "holder_close_failed",
                message: error.to_string(),
            }),
        }
    }

    fn read(&self, pane: &str, lines: u16) -> Result<(String, bool), HolderError> {
        match self.holders.read(pane, lines).map_err(ask_error)? {
            Answer::Text { text, truncated } => Ok((text, truncated)),
            Answer::Processes { .. } => Err(HolderError::Unreachable(format!(
                "pane {pane}'s holder answered a read with its processes"
            ))),
        }
    }

    fn read_styled(&self, pane: &str, read: StyledRead) -> Result<(String, bool), HolderError> {
        let rows = match read {
            StyledRead::Screen => None,
            StyledRead::Rows(lines) => Some(lines),
        };
        match self.holders.read_styled(pane, rows) {
            Ok(Answer::Text { text, truncated }) => Ok((text, truncated)),
            Ok(Answer::Processes { .. }) => Err(HolderError::Unreachable(format!(
                "pane {pane}'s holder answered a styled read with its processes"
            ))),
            // A holder built before styled reads reads the ask as a first
            // message it does not know.
            Err(OpenError::Refused(closed)) if closed.reason == CloseReason::HelloInvalid => {
                Err(HolderError::Refused {
                    code: UNSUPPORTED_HOLDER,
                    message: format!(
                        "pane {pane}'s holder was built before styled reads: {}",
                        closed.detail
                    ),
                })
            }
            Err(error) => Err(ask_error(error)),
        }
    }

    fn processes(&self, pane: &str) -> Result<HolderProcesses, HolderError> {
        match self.holders.processes(pane).map_err(ask_error)? {
            Answer::Processes { shell, foreground } => Ok(HolderProcesses {
                shell,
                foreground: match foreground {
                    Foreground::Known { group, members } => Some(HolderForeground {
                        group,
                        members: members
                            .into_iter()
                            .map(|member| (member.pid, member.name.unwrap_or_default()))
                            .collect(),
                    }),
                    Foreground::Unknown(_) => None,
                },
            }),
            Answer::Text { .. } => Err(HolderError::Unreachable(format!(
                "pane {pane}'s holder answered a process ask with text"
            ))),
        }
    }

    fn proof(&self, pane: &str) -> Result<HolderProof, HolderError> {
        let record = self
            .holders
            .record(pane)
            .map_err(|why| HolderError::Unreachable(format!("pane {pane}'s record: {why}")))?
            .ok_or_else(|| HolderError::NoPane(format!("pane {pane} has no record")))?;
        match (record.phase, record.program, record.end) {
            (Phase::Running, Some(program), None) => Ok(HolderProof {
                incarnation: record.incarnation,
                program: program.pid,
                started: program.started,
            }),
            (phase, _, _) => Err(HolderError::Refused {
                code: "pane_not_running",
                message: format!("pane {pane}'s record is {phase:?}, not running"),
            }),
        }
    }

    fn input(&self, pane: &str, input: PaneInput) -> Result<(), HolderError> {
        self.service.type_input(pane, typed(input));
        Ok(())
    }

    fn letter(
        &self,
        pane: &str,
        body: PaneInput,
        submit: PaneInput,
        gap: std::time::Duration,
    ) -> Result<(), HolderError> {
        self.service
            .type_letter(pane, typed(body), typed(submit), gap);
        Ok(())
    }

    fn watch_ends(&self, ended: PaneEnded) {
        self.holders.watch_ends(ended);
    }

    fn watch_facts(&self, sink: PaneFactsSink) {
        self.holders.watch_facts(sink);
    }

    fn set_rules(&self, pane: &str, rules: Option<ScreenRules>) -> Result<(), HolderError> {
        self.holders.set_rules(pane, rules)
    }

    fn recovered(&self, timeout: std::time::Duration) -> Result<HolderRecovery, HolderError> {
        self.holders
            .recovered(timeout)
            .ok_or_else(|| HolderError::Refused {
                code: "recovery_unfinished",
                message: format!(
                    "the node had not looked at every pane's record within {} s",
                    timeout.as_secs()
                ),
            })
    }
}

/// `shell` started so that it first prints `notice` as a line of its own:
/// on macOS and Linux a POSIX `sh` prints it and replaces itself with the
/// shell, so the pane's program is the shell; on Windows PowerShell writes it
/// and stays open.
fn shell_after_notice(shell: &[OsString], notice: &str) -> Vec<OsString> {
    let line: String = notice.chars().filter(|c| !c.is_control()).collect();
    if cfg!(windows) {
        let write = format!("Write-Host '{}'", line.replace('\'', "''"));
        let mut program = shell.to_vec();
        // A shell started with a command of its own (the integration's)
        // writes the line first, in that same command.
        match program
            .iter()
            .position(|word| word.eq_ignore_ascii_case("-Command"))
            .filter(|at| at + 1 < program.len())
        {
            Some(at) => {
                let command = format!("{write}; {}", program[at + 1].to_string_lossy());
                program[at + 1] = OsString::from(command);
            }
            None => program.extend(
                ["-NoLogo", "-NoExit", "-Command"]
                    .map(OsString::from)
                    .into_iter()
                    .chain([OsString::from(write)]),
            ),
        }
        return program;
    }
    let mut program = vec![
        OsString::from("/bin/sh"),
        OsString::from("-c"),
        OsString::from("printf '%s\\n' \"$1\"; shift; exec \"$@\""),
        OsString::from("hide-notice"),
        OsString::from(line),
    ];
    program.extend(shell.iter().cloned());
    program
}

/// The engine's input in the service's words.
fn typed(input: PaneInput) -> Typed {
    match input {
        PaneInput::Bytes(bytes) => Typed::Bytes(bytes),
        PaneInput::Paste(text) => Typed::Paste(text),
        PaneInput::Keys(keys) => Typed::Keys(keys),
    }
}

fn start_refusal(refusal: StartRefusal) -> HolderError {
    let code = match &refusal {
        StartRefusal::InvalidPane(_) => "invalid_pane",
        StartRefusal::Cap { .. } => "holder_cap",
        StartRefusal::Conflict(_) => "pane_conflict",
        StartRefusal::RecordUnreadable(_) => "record_unreadable",
        StartRefusal::OtherOwner(_) => "other_owner",
        StartRefusal::Failed(_) => "holder_start_failed",
        StartRefusal::EarlierRuns(_) => "earlier_start_runs",
    };
    HolderError::Refused {
        code,
        message: refusal.to_string(),
    }
}

fn ask_error(error: OpenError) -> HolderError {
    match error {
        OpenError::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
            HolderError::NoPane(error.to_string())
        }
        error => HolderError::Unreachable(error.to_string()),
    }
}
