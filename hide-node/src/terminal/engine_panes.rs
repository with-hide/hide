//! This machine's pane holders as the core's own engine asks them (PRD
//! engine-bundle-3-core-owns D-3, D-5): [`LocalPanes`] answers
//! [`PaneHolders`] over [`Holders`] and the terminal service whose screens
//! they attach to. A pane started with no program runs this machine's
//! interactive shell ([`interactive_shell`]).
//!
//! Only a daemon with the development switch on builds one, beside the
//! holders [`Holders::start_local`] opened; with it off nothing here runs.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use hide_holder_protocol::{Answer, Appearance, CloseReason, Foreground, OpenError, Size, Theme};
use hide_kit::panes::{CloseError, StartRefusal, StartRequest, Started};
use hide_node_link::holders::{
    HolderError, HolderForeground, HolderProcesses, HolderRecovery, HolderStart, PaneEnded,
    PaneFactsSink, PaneHolders, ScreenRules, StyledRead, UNSUPPORTED_HOLDER,
};
use hide_node_link::terminal::{KeyTarget, TerminalNode};

use super::Service;
use super::holder::Holders;

/// This machine's holders and the service that shows their screens.
pub struct LocalPanes {
    holders: Arc<Holders>,
    service: Arc<Service>,
    shell: Vec<OsString>,
}

impl LocalPanes {
    /// The panes of `holders`, attached to `service`; a pane started with no
    /// program runs `shell`, which a daemon reads once with
    /// [`interactive_shell`].
    pub fn new(holders: Arc<Holders>, service: Arc<Service>, shell: Vec<OsString>) -> Self {
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
            (None, None) => self.shell.clone(),
            (None, Some(notice)) => shell_after_notice(&self.shell, notice),
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
            env: start
                .env
                .iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value)))
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

    fn input(&self, pane: &str, bytes: Vec<u8>) -> Result<(), HolderError> {
        self.service
            .key(KeyTarget::Pane(pane.to_owned()), bytes, super::unix_ms());
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
        let program = shell
            .first()
            .cloned()
            .unwrap_or_else(|| OsString::from("powershell.exe"));
        return vec![
            program,
            OsString::from("-NoLogo"),
            OsString::from("-NoExit"),
            OsString::from("-Command"),
            OsString::from(format!("Write-Host '{}'", line.replace('\'', "''"))),
        ];
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

/// The interactive shell a pane of this machine starts when it is given no
/// program, as Herdr chooses one with no shell configured: on macOS and
/// Linux the account's `$SHELL`, else `/bin/sh`, as a login shell on macOS
/// only; on Windows the first `pwsh.exe` on an absolute `PATH` folder, else
/// `powershell.exe`.
pub fn interactive_shell() -> Vec<OsString> {
    if cfg!(windows) {
        let pwsh = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .filter(|folder| folder.is_absolute())
            .map(|folder| folder.join("pwsh.exe"))
            .find(|candidate| candidate.is_file());
        return vec![
            pwsh.map(PathBuf::into_os_string)
                .unwrap_or_else(|| OsString::from("powershell.exe")),
        ];
    }
    let shell = std::env::var_os("SHELL")
        .filter(|shell| !shell.to_string_lossy().trim().is_empty())
        .unwrap_or_else(|| OsString::from("/bin/sh"));
    if cfg!(target_os = "macos") {
        vec![shell, OsString::from("-l")]
    } else {
        vec![shell]
    }
}
