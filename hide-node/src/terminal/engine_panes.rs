//! This machine's pane holders as the core's own engine asks them (PRD
//! engine-bundle-3-core-owns D-3, D-5): [`LocalPanes`] answers
//! [`PaneHolders`] over [`Holders`] and the terminal service whose screens
//! they attach to. A pane started with no program runs the account's shell
//! with Hide's shell integration ([`PaneShell::Account`]), as a tab Hide
//! opens does, so it says its folder and marks its prompt on its screen.
//!
//! A daemon builds one beside the holders [`Holders::start_local`] opened.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hide_host::kit::PaneInstall;
use hide_host::shell_integration::{ShellLaunch, ShellReadiness, UserShell};

use super::Service;
use super::holder::{Holders, Renewal};
use super::input::Typed;
use hide_holder_protocol::record::Phase;
use hide_holder_protocol::{Answer, Appearance, Foreground, OpenError, Size, Theme};
use hide_kit::panes::{CloseError, StartRefusal, StartRequest, Started};
use hide_node_link::holders::{
    HolderError, HolderForeground, HolderIntent, HolderProcesses, HolderProof, HolderRecovery,
    HolderRenewal, HolderRequest, HolderReservation, HolderScroll, HolderStart, PaneEnded,
    PaneFactsSink, PaneHolders, PaneInput, RENEWAL_LOST, RENEWAL_UNSETTLED, ScreenRules,
    StyledRead, UNSUPPORTED_HOLDER,
};

/// This machine's holders and the service that shows their screens.
pub struct LocalPanes {
    holders: Arc<Holders>,
    service: Arc<Service>,
    shell: PaneShell,
    install: Option<PaneInstall>,
    owner: Option<Arc<hide_kit::panes::Owner>>,
}

/// How a pane started with no program runs this machine's shell.
#[derive(Clone)]
pub enum PaneShell {
    /// The account's shell with Hide's shell integration, prepared for each
    /// pane as the node prepares it for a tab Hide opens
    /// (`hide_host::shell_integration::prepare`): a pane on the core's own
    /// engine learns its folder from its shell, which
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
                        code: "shell_unavailable".to_owned(),
                        message: format!("the account's shell could not be prepared: {error}"),
                    })
            }
        }
    }
}

impl LocalPanes {
    /// The panes of `holders`, attached to `service`; a pane started with no
    /// program runs `shell`. Its panes name no installation.
    pub fn new(holders: Arc<Holders>, service: Arc<Service>, shell: PaneShell) -> Self {
        holders.own_every_pane();
        Self {
            holders,
            service,
            shell,
            install: None,
            owner: None,
        }
    }

    /// The panes of `holders` on the account `home` belongs to: a pane with
    /// no program of its own runs the account's shell with Hide's
    /// integration, as the node prepares it for a tab. What this machine's
    /// engine and a device's node both run.
    /// Every such pane names the Hide installation this program belongs to,
    /// so a hook entry or a `hide` command another installation wrote leaves
    /// the pane to this one's (docs/agent-hooks.md, More than one Hide on an
    /// account).
    pub fn for_account(holders: Arc<Holders>, service: Arc<Service>, home: PathBuf) -> Self {
        let mut panes = Self::new(
            holders,
            service,
            PaneShell::Account {
                home,
                user: hide_host::shell_integration::UserShell::of_process(),
            },
        );
        panes.install = hide_host::kit::running_install();
        panes
    }

    fn expand(&self, start: &HolderStart) -> Result<StartRequest, HolderError> {
        let size = Size::checked(start.rows, start.cols).map_err(|why| HolderError::Refused {
            code: "invalid_size".to_owned(),
            message: why,
        })?;
        let mut env: Vec<(OsString, OsString)> = Vec::new();
        let program = match (&start.program, &start.notice) {
            (Some(_), Some(_)) => {
                return Err(HolderError::Refused {
                    code: "invalid_program".to_owned(),
                    message: "only a shell pane shows a notice before it starts".to_owned(),
                });
            }
            (Some(program), None) if !program.is_empty() => {
                program.iter().map(OsString::from).collect()
            }
            (Some(_), None) => {
                return Err(HolderError::Refused {
                    code: "invalid_program".to_owned(),
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
                code: "invalid_cwd".to_owned(),
                message: format!("{} is not a folder path: {error}", start.cwd),
            })?;
        // The shell's own variables, then the caller's, then the pane's own
        // name for itself and its installation, which no caller's variable
        // replaces (PRD engine-bundle-3-core-owns D-9): the holder removes
        // every Herdr variable, so a process in the pane names this pane
        // alone.
        let mut env: Vec<(OsString, OsString)> = env
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
            .chain(self.install.iter().flat_map(PaneInstall::variables))
            .collect();
        if let Some(folder) = self
            .install
            .as_ref()
            .and_then(|install| install.path_first.as_deref())
        {
            put_first_on_path(&mut env, folder, std::env::var_os("PATH"))?;
        }
        Ok(StartRequest {
            pane: start.pane.clone(),
            program,
            cwd,
            env,
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
        })
    }

    fn admit(
        &self,
        request: HolderRequest,
        intent: &HolderIntent,
    ) -> Result<hide_kit::panes::Request<'_>, HolderError> {
        self.owner()?
            .admit(request.owner, request.number, &intent.pane, &intent.digest)
            .map_err(|message| HolderError::Refused {
                code: "pane_request_refused".to_owned(),
                message,
            })
    }

    fn owner(&self) -> Result<&hide_kit::panes::Owner, HolderError> {
        self.owner.as_deref().ok_or_else(|| HolderError::Refused {
            code: "pane_owner_unavailable".to_owned(),
            message: "this connection has not acquired its pane records".to_owned(),
        })
    }
}

/// Puts `folder` first on the `PATH` in `env`: the last one `env` names,
/// in any case of the name as Windows reads it, else `inherited`, the one
/// the pane would inherit. The folder appears once, so a pane started from a
/// shell of the same installation, whose `PATH` already holds it, does not
/// grow. A folder `PATH` cannot hold is refused.
fn put_first_on_path(
    env: &mut Vec<(OsString, OsString)>,
    folder: &Path,
    inherited: Option<OsString>,
) -> Result<(), HolderError> {
    let is_path = |name: &OsString| {
        name.to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case("PATH"))
    };
    let named = env
        .iter()
        .rev()
        .find(|(name, _)| is_path(name))
        .map(|(_, value)| value.clone())
        .or(inherited);
    // Windows compares folder names without case.
    let same = |other: &Path| {
        other == folder
            || other
                .to_string_lossy()
                .eq_ignore_ascii_case(&folder.to_string_lossy())
    };
    let folders = std::iter::once(folder.to_path_buf()).chain(
        named
            .iter()
            .flat_map(std::env::split_paths)
            .filter(|other| !same(other)),
    );
    let path = std::env::join_paths(folders).map_err(|error| HolderError::Refused {
        code: "invalid_path".to_owned(),
        message: format!("{} cannot go on a PATH: {error}", folder.display()),
    })?;
    env.retain(|(name, _)| !is_path(name));
    env.push((OsString::from("PATH"), path));
    Ok(())
}

/// The folders the foreground members other than the pane's `shell` work in,
/// each once, as this machine observes them; `None` when one of them could
/// not be read. A member that ended since the holder listed it works nowhere.
fn program_folders(shell: u32, members: impl Iterator<Item = u32>) -> Option<Vec<String>> {
    let mut folders = Vec::new();
    for pid in members.filter(|pid| *pid != shell) {
        match hide_platform::process::observed_cwd_of(pid) {
            Ok(folder) => folders.push(hide_platform::path::to_wire(&folder).ok()?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    folders.sort();
    folders.dedup();
    Some(folders)
}

impl PaneHolders for LocalPanes {
    fn own(
        self: Arc<Self>,
        within: std::time::Duration,
    ) -> Result<Arc<dyn PaneHolders>, HolderError> {
        let owner = Arc::new(
            self.holders
                .acquire(within)
                .map_err(|message| HolderError::Unreachable { message })?,
        );
        let found = self.holders.recover(&owner, &self.service);
        if !found.complete {
            return Err(HolderError::Unreachable {
                message: "the node could not complete its pane record scan".to_owned(),
            });
        }
        // A complete scan retains unreadable IDs in `present` and in the
        // record cap. Quarantine those records, not this connection's
        // healthy panes; starting an unreadable ID still refuses in Panes.
        self.holders.prune_folders();
        Ok(Arc::new(Self {
            holders: Arc::clone(&self.holders),
            service: Arc::clone(&self.service),
            shell: self.shell.clone(),
            install: self.install.clone(),
            owner: Some(owner),
        }))
    }

    fn request(&self) -> Result<HolderRequest, HolderError> {
        // The connection's owner numbers every request of its incarnation,
        // the ones a library caller issues included, so none is issued twice.
        let owner = self.owner()?;
        let number = owner.number().map_err(|message| HolderError::Refused {
            code: "pane_request_refused".to_owned(),
            message,
        })?;
        Ok(HolderRequest {
            owner: owner.incarnation(),
            number,
        })
    }

    fn retire(&self, request: HolderRequest) -> Result<(), HolderError> {
        match self.owner()?.retire(request.owner, request.number) {
            Ok(true) => Ok(()),
            Ok(false) => Err(HolderError::Unreachable {
                message: "the pane request still has workers draining".to_owned(),
            }),
            Err(message) => Err(HolderError::Refused {
                code: "pane_request_refused".to_owned(),
                message,
            }),
        }
    }

    fn reserve(
        &self,
        start: &HolderStart,
        previous: Option<u64>,
        request: HolderRequest,
    ) -> Result<HolderReservation, HolderError> {
        let intent = start.intent()?;
        let guard = self.admit(request, &intent)?;
        // Validate without expanding the shell or modifying configuration.
        Size::checked(start.rows, start.cols).map_err(|message| HolderError::Refused {
            code: "invalid_size".to_owned(),
            message,
        })?;
        if start
            .program
            .as_ref()
            .is_some_and(|program| program.is_empty() || start.notice.is_some())
        {
            return Err(HolderError::Refused {
                code: "invalid_program".to_owned(),
                message: "the pane's program or notice is invalid".to_owned(),
            });
        }
        self.holders
            .reserve(&guard, &intent, previous)
            .map_err(start_refusal)
    }

    fn start_reserved(
        &self,
        start: &HolderStart,
        reserved: HolderReservation,
        request: HolderRequest,
    ) -> Result<(), HolderError> {
        let intent = start.intent()?;
        let guard = self.admit(request, &intent)?;
        match self
            .holders
            .start_reserved(&guard, &self.service, &intent, reserved, || {
                self.expand(start)
                    .map_err(|error| StartRefusal::Failed(format!("{error:?}")))
            }) {
            Ok(Started::Started(record) | Started::Existing(record)) => match record.end {
                None => Ok(()),
                Some(end) => Err(HolderError::Refused {
                    code: "pane_ended".to_owned(),
                    message: format!("pane {} ended ({})", start.pane, end.reason.name()),
                }),
            },
            Err(refusal) => Err(start_refusal(refusal)),
        }
    }

    fn cancel(&self, intent: &HolderIntent, request: HolderRequest) -> Result<(), HolderError> {
        let guard = self.admit(request, intent)?;
        self.holders
            .cancel(&guard, &self.service, intent)
            .map_err(start_refusal)
    }

    fn renew(&self, start: &HolderStart) -> Result<HolderRenewal, HolderError> {
        if start.program.is_some() {
            return Err(HolderError::Refused {
                code: "invalid_program".to_owned(),
                message: "only a shell pane's holder is renewed".to_owned(),
            });
        }
        let intent = start.intent()?;
        match self
            .holders
            .renew(self.owner()?, &self.service, &intent, || {
                self.expand(start).map_err(|error| format!("{error:?}"))
            }) {
            Renewal::Current => Ok(HolderRenewal::Current),
            Renewal::Renewed { from, to } => Ok(HolderRenewal::Renewed { from, to }),
            Renewal::Kept { code, message } => Ok(HolderRenewal::Kept {
                code: code.to_owned(),
                message,
            }),
            Renewal::Lost(message) => Err(HolderError::Refused {
                code: RENEWAL_LOST.to_owned(),
                message,
            }),
            Renewal::Unsettled(message) => Err(HolderError::Refused {
                code: RENEWAL_UNSETTLED.to_owned(),
                message,
            }),
        }
    }

    fn shell_launch(&self) -> Option<ShellLaunch> {
        self.shell
            .launch()
            .ok()
            .filter(|launch| launch.readiness == ShellReadiness::Integrated)
    }

    fn terminate(&self, pane: &str) -> Result<(), HolderError> {
        match self.holders.terminate(self.owner()?, &self.service, pane) {
            Ok(_) => Ok(()),
            Err(CloseError::NoRecord) => Err(HolderError::NoPane {
                message: format!("pane {pane} has no holder on this machine"),
            }),
            Err(error) => Err(HolderError::Refused {
                code: "holder_close_failed".to_owned(),
                message: error.to_string(),
            }),
        }
    }

    fn read(&self, pane: &str, lines: u16) -> Result<(String, bool), HolderError> {
        match self.holders.read(pane, lines).map_err(ask_error)? {
            Answer::Text { text, truncated } => Ok((text, truncated)),
            other => Err(misanswered(pane, "a read", &other)),
        }
    }

    fn read_styled(&self, pane: &str, read: StyledRead) -> Result<(String, bool), HolderError> {
        let rows = match read {
            StyledRead::Screen => None,
            StyledRead::Rows(lines) => Some(lines),
        };
        match self.holders.read_styled(pane, rows) {
            Ok(Answer::Text { text, truncated }) => Ok((text, truncated)),
            Ok(other) => Err(misanswered(pane, "a styled read", &other)),
            Err(error) => Err(newer_ask_error(pane, "styled reads", error)),
        }
    }

    fn read_visible(&self, pane: &str) -> Result<String, HolderError> {
        match self.holders.read_visible(pane) {
            Ok(Answer::Text { text, .. }) => Ok(text),
            Ok(other) => Err(misanswered(pane, "a visible read", &other)),
            Err(error) => Err(newer_ask_error(pane, "visible reads", error)),
        }
    }

    fn scroll(&self, pane: &str, to: Option<u64>) -> Result<HolderScroll, HolderError> {
        match self.holders.scroll(pane, to) {
            Ok(Answer::Scroll {
                offset_from_bottom,
                max_offset_from_bottom,
            }) => Ok(HolderScroll {
                offset_from_bottom,
                max_offset_from_bottom,
            }),
            Ok(other) => Err(misanswered(pane, "a scroll", &other)),
            Err(error) => Err(newer_ask_error(pane, "scroll asks", error)),
        }
    }

    fn processes(
        &self,
        pane: &str,
        within: std::time::Duration,
    ) -> Result<HolderProcesses, HolderError> {
        match self.holders.processes(pane, within).map_err(ask_error)? {
            Answer::Processes { shell, foreground } => Ok(HolderProcesses {
                shell,
                foreground: match foreground {
                    Foreground::Known { group, members } => Some(HolderForeground {
                        group,
                        folders: program_folders(shell, members.iter().map(|member| member.pid)),
                        members: members
                            .into_iter()
                            .map(|member| (member.pid, member.name.unwrap_or_default()))
                            .collect(),
                    }),
                    Foreground::Unknown(_) => None,
                },
            }),
            other => Err(misanswered(pane, "a process ask", &other)),
        }
    }

    fn proof(&self, pane: &str) -> Result<HolderProof, HolderError> {
        let record = self
            .holders
            .record(pane)
            .map_err(|why| HolderError::Unreachable {
                message: format!("pane {pane}'s record: {why}"),
            })?
            .ok_or_else(|| HolderError::NoPane {
                message: format!("pane {pane} has no record"),
            })?;
        match (record.phase, record.program, record.end) {
            (Phase::Running, Some(program), None) => Ok(HolderProof {
                incarnation: record.incarnation,
                program: program.pid,
                started: program.started,
            }),
            (phase, _, _) => Err(HolderError::Refused {
                code: "pane_not_running".to_owned(),
                message: format!("pane {pane}'s record is {phase:?}, not running"),
            }),
        }
    }

    fn input(&self, pane: &str, input: PaneInput) -> Result<(), HolderError> {
        self.service.type_input(pane, typed(input))
    }

    fn letter(
        &self,
        pane: &str,
        text: Vec<PaneInput>,
        submit: PaneInput,
        gap: std::time::Duration,
    ) -> Result<(), HolderError> {
        self.service.type_letter(
            pane,
            text.into_iter().map(typed).collect(),
            typed(submit),
            gap,
        )
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
                code: "recovery_unfinished".to_owned(),
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
        let write = format!(
            "Write-Host {}",
            hide_shell::Syntax::PowerShell.single_quoted(&line)
        );
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
        StartRefusal::Uncertain(message) => {
            return HolderError::Unreachable {
                message: message.clone(),
            };
        }
        StartRefusal::EarlierRuns(_) => "earlier_start_runs",
    };
    HolderError::Refused {
        code: code.to_owned(),
        message: refusal.to_string(),
    }
}

/// A holder that answered an ask with another ask's answer.
fn misanswered(pane: &str, ask: &str, answer: &Answer) -> HolderError {
    let what = match answer {
        Answer::Text { .. } => "text",
        Answer::Processes { .. } => "its processes",
        Answer::Scroll { .. } => "its scroll position",
    };
    HolderError::Unreachable {
        message: format!("pane {pane}'s holder answered {ask} with {what}"),
    }
}

/// An ask's failure, refused [`UNSUPPORTED_HOLDER`] when the holder was
/// built before `asks` were added.
fn newer_ask_error(pane: &str, asks: &str, error: OpenError) -> HolderError {
    match super::holder::too_old(&error) {
        Some(version) => HolderError::Refused {
            code: UNSUPPORTED_HOLDER.to_owned(),
            message: format!("pane {pane}'s holder (protocol {version}) was built before {asks}"),
        },
        None => ask_error(error),
    }
}

fn ask_error(error: OpenError) -> HolderError {
    match error {
        OpenError::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
            HolderError::NoPane {
                message: error.to_string(),
            }
        }
        error => HolderError::Unreachable {
            message: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pane of an installation that names a folder finds that folder first
    /// on its `PATH`, ahead of the caller's or the inherited one, whatever
    /// case the caller wrote the name in, and the rest of the `PATH` stays.
    #[test]
    fn the_installation_s_folder_comes_first_on_the_pane_s_path() {
        let folder = std::env::temp_dir().join("hide-current");
        let first = std::env::temp_dir().join("first");
        let second = std::env::temp_dir().join("second");
        let joined = |folders: &[&PathBuf]| std::env::join_paths(folders).unwrap();

        let mut env = vec![
            (OsString::from("Path"), joined(&[&first, &second])),
            (OsString::from("HOME"), OsString::from("home")),
        ];
        put_first_on_path(&mut env, &folder, Some(joined(&[&second]))).unwrap();
        assert_eq!(
            env,
            [
                (OsString::from("HOME"), OsString::from("home")),
                (OsString::from("PATH"), joined(&[&folder, &first, &second])),
            ]
        );

        let mut env = Vec::new();
        put_first_on_path(&mut env, &folder, Some(joined(&[&second]))).unwrap();
        assert_eq!(env, [(OsString::from("PATH"), joined(&[&folder, &second]))]);

        let mut env = Vec::new();
        put_first_on_path(&mut env, &folder, None).unwrap();
        assert_eq!(env, [(OsString::from("PATH"), joined(&[&folder]))]);

        // A pane started inside one of this installation's panes.
        let mut env = vec![(OsString::from("PATH"), joined(&[&first, &folder, &second]))];
        put_first_on_path(&mut env, &folder, None).unwrap();
        assert_eq!(
            env,
            [(OsString::from("PATH"), joined(&[&folder, &first, &second]))]
        );
    }
}
