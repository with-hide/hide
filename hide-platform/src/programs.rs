//! Where the programs an agent runs are looked for, the one lookup every
//! crate uses to say whether an agent's CLI is installed, and the command a
//! node starts any program it names by ([`command`]).
//!
//! The search is the folders the account's login shell puts on its `PATH`,
//! then this process's own `PATH`, then the folders installers put a CLI in
//! that a daemon's `PATH` may not reach. The install kit's "is this agent
//! installed" and Hide AI's "can this agent be asked" both read it, so the two
//! cannot disagree about a program that only the shell's `PATH`, or only an
//! install folder, reaches.
//!
//! Asking the shell is a subprocess with a deadline, so the answer is kept
//! until a startup file changes (below). A caller on a hot path asks only
//! after a first read has warmed it. The same ask reads the environment the
//! shell exports ([`login_shell_environment`]), for a caller that starts an
//! agent the way the operator's terminal would ([`account_environment`]), so
//! no caller starts the shell a second time for it.
//!
//! This is the one place the crate keeps anything between calls: the login
//! shell's answer is a fact about the account, not about a caller, so every
//! caller in the process reads the one cache (a kit worker, Hide AI's
//! backends and a device helper must not each start the operator's shell for
//! the same question), and it is keyed by the inputs it was read for, so no
//! caller's answer stands in for another's.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use crate::host;

mod launcher;

/// How long the login shell has to answer, as long as the desktop host gives
/// it when it asks the same question.
const LOGIN_SHELL_DEADLINE: Duration = Duration::from_secs(10);

/// The startup files a login shell reads that installers edit to put their
/// folder on the `PATH`: zsh's (in the home, and in `~/.config/zsh` for a
/// `ZDOTDIR` there), bash's and sh's, fish's, and the system's own.
const STARTUP_FILES_IN_HOME: &[&str] = &[
    ".zshenv",
    ".zprofile",
    ".zshrc",
    ".zlogin",
    ".config/zsh/.zshenv",
    ".config/zsh/.zprofile",
    ".config/zsh/.zshrc",
    ".config/zsh/.zlogin",
    ".bash_profile",
    ".bash_login",
    ".profile",
    ".bashrc",
    ".config/fish/config.fish",
    ".config/fish/conf.d",
    ".config/fish/fish_variables",
];
const SYSTEM_STARTUP_FILES: &[&str] = &[
    "/etc/paths",
    "/etc/paths.d",
    "/etc/zshenv",
    "/etc/zprofile",
    "/etc/zshrc",
    "/etc/zlogin",
    "/etc/profile",
    "/etc/profile.d",
    "/etc/bashrc",
    "/etc/bash.bashrc",
];

/// Each startup file's write time and length, `None` where there is none.
type StartupStamps = Vec<Option<(Option<SystemTime>, u64)>>;

fn startup_stamps(home: &Path) -> StartupStamps {
    STARTUP_FILES_IN_HOME
        .iter()
        .map(|name| home.join(name))
        .chain(SYSTEM_STARTUP_FILES.iter().map(PathBuf::from))
        .map(|file| {
            std::fs::metadata(file)
                .ok()
                .map(|meta| (meta.modified().ok(), meta.len()))
        })
        .collect()
}

/// The variables an environment holds, each `(name, value)`.
pub type Environment = Arc<[(OsString, OsString)]>;

/// What the login shell answered, with the startup files it was read under
/// and when it was asked.
struct ShellAnswer {
    stamps: StartupStamps,
    path: Option<OsString>,
    environment: Option<Environment>,
    asked: Instant,
}

/// The place one shell and home keep their answer. Its lock is held across
/// the ask, so callers for the same pair wait for one shell, while a caller
/// for another home is never held behind it.
struct Slot {
    shell: PathBuf,
    home: PathBuf,
    answer: Mutex<Option<ShellAnswer>>,
}

/// How many shell and home pairs are remembered. A process runs as one
/// account, so one is the working set; the rest is room for a test that
/// reads several fixture homes, and the oldest goes first past it (resident
/// process rule: cap what grows).
const REMEMBERED: usize = 8;

/// The `PATH` the account's login shell sets up, asked again only when one
/// of its startup files changed (`None` when there is no shell to ask): an installer puts its folder on the `PATH`
/// by editing one, and Settings re-reads the kit every few seconds, which
/// must not start a shell each time. A file those files read in turn is not
/// watched; the next launch or connection asks afresh. A shell that did not
/// answer is asked again after [`UNREAD_RETRY`], and the search goes on
/// without its folders meanwhile; the failure is logged, since there is
/// nothing on screen the operator could do about it (design rule 13).
///
/// A shell and home pair has its own lock, held across the ask, so callers
/// for the same pair that arrive cold together wait for the one shell and
/// read its answer instead of starting one each, and a caller for another
/// home never waits behind it. A caller waits at most the deadline for that,
/// and a caller whose `stop` is raised while it is the one asking ends the
/// shell and leaves nothing remembered.
pub fn login_shell_path(
    home: &Path,
    login_shell: Option<&Path>,
    stop: &AtomicBool,
) -> Option<OsString> {
    asked(home, login_shell, stop)?.path
}

/// Every variable the account's login shell exports
/// ([`host::LoginShell::environment`]), from the same ask, kept and asked
/// again as [`login_shell_path`] is. `None` when there is no shell to ask or
/// it did not list them; why is logged.
pub fn login_shell_environment(
    home: &Path,
    login_shell: Option<&Path>,
    stop: &AtomicBool,
) -> Option<Environment> {
    asked(home, login_shell, stop)?.environment
}

/// What the login shell last answered for `home`, asked as
/// [`login_shell_path`] says; `None` when there is no shell or the owner
/// stopped the ask.
fn asked(home: &Path, login_shell: Option<&Path>, stop: &AtomicBool) -> Option<Answered> {
    static SLOTS: Mutex<Vec<Arc<Slot>>> = Mutex::new(Vec::new());
    let shell = login_shell?;
    let slot = {
        let mut slots = SLOTS.lock().unwrap_or_else(PoisonError::into_inner);
        match slots
            .iter()
            .find(|slot| slot.shell == shell && slot.home == home)
        {
            Some(slot) => Arc::clone(slot),
            None => {
                if slots.len() >= REMEMBERED {
                    slots.remove(0);
                }
                let slot = Arc::new(Slot {
                    shell: shell.to_path_buf(),
                    home: home.to_path_buf(),
                    answer: Mutex::new(None),
                });
                slots.push(Arc::clone(&slot));
                slot
            }
        }
    };
    let mut answer = slot.answer.lock().unwrap_or_else(PoisonError::into_inner);
    let stamps = startup_stamps(home);
    if let Some(known) = answer.as_ref()
        && known.stamps == stamps
        && (known.path.is_some() || known.asked.elapsed() < UNREAD_RETRY)
    {
        return Some(Answered {
            path: known.path.clone(),
            environment: known.environment.clone(),
        });
    }
    let asked = Instant::now();
    let (path, environment) =
        match crate::host::login_shell(shell, home, LOGIN_SHELL_DEADLINE, stop) {
            Ok(read) => {
                let environment = match read.environment {
                    Ok(variables) => Some(Environment::from(variables)),
                    Err(why) => {
                        eprintln!(
                            "platform.login_shell_environment_unread shell={} why={why}",
                            shell.display()
                        );
                        None
                    }
                };
                (Some(read.path), environment)
            }
            // The owner is going away (Hide quitting, a device's
            // connection closed): nothing failed, and nothing is remembered.
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return None,
            Err(error) => {
                eprintln!(
                    "platform.login_shell_unread shell={} kind={:?} elapsed_ms={}",
                    shell.display(),
                    error.kind(),
                    asked.elapsed().as_millis()
                );
                (None, None)
            }
        };
    *answer = Some(ShellAnswer {
        stamps,
        path: path.clone(),
        environment: environment.clone(),
        asked,
    });
    Some(Answered { path, environment })
}

/// One ask's answer, as a caller takes it.
struct Answered {
    path: Option<OsString>,
    environment: Option<Environment>,
}

/// How long an answer that could not be read (the login shell's `PATH`) is
/// not asked for again. One that was read stays until what it
/// came from changes.
const UNREAD_RETRY: Duration = Duration::from_secs(60);

/// The `PATH` a program is found on and run with: the login shell's folders
/// when `shell_path` carries them, then this process's own `PATH`, then the
/// folders installers put a CLI in that such a `PATH` may not reach. A device
/// helper started over SSH has only the system folders. A CLI installed as a
/// script starts its interpreter from the same `PATH` (pnpm's `codex` is a
/// shell script that runs `node`), so a CLI found here is run with this
/// value, never with the daemon's `PATH` alone. A folder named twice is
/// searched once, and a folder that is not absolute (an empty segment, which a
/// program reads as its working directory, or a relative one) is left out, as
/// [`host::find_program`] leaves it out of the search.
pub fn cli_path_with(home: &Path, shell_path: Option<&OsStr>) -> Option<OsString> {
    let inherited = host::login_path().unwrap_or_else(|_| "/usr/bin:/bin".into());
    let mut seen = std::collections::HashSet::new();
    let shell = shell_path.map(std::env::split_paths).into_iter().flatten();
    std::env::join_paths(
        shell
            .chain(std::env::split_paths(&inherited))
            .chain(usual_install_folders(home))
            .filter(|folder| folder.is_absolute() && seen.insert(folder.clone())),
    )
    .ok()
}

/// [`cli_path_with`] with the login shell asked ([`login_shell_path`]).
pub fn search_path(home: &Path, login_shell: Option<&Path>, stop: &AtomicBool) -> Option<OsString> {
    cli_path_with(home, login_shell_path(home, login_shell, stop).as_deref())
}

/// [`search_path`] for the account this process runs as: its home and its
/// login shell. The `PATH` a caller outside the install kit runs a CLI with
/// when it found that CLI by [`find_cli`].
pub fn account_path() -> Option<OsString> {
    static NEVER: AtomicBool = AtomicBool::new(false);
    let home = host::home_dir().ok()?;
    let shell = if cfg!(windows) {
        None
    } else {
        host::default_shell().ok()
    };
    search_path(&home, shell.as_deref(), &NEVER)
}

/// The environment a new terminal of the account this process runs as
/// starts with, which is what an agent the operator starts there has: on
/// macOS and Linux every variable its login shell exports
/// ([`login_shell_environment`], from the ask [`account_path`] makes), and
/// on Windows the environment the system builds for the account's sign-in
/// ([`host::account_environment`]). Never this process's own: a daemon the
/// Dock started lacks what the startup files export, and one a terminal
/// started carries that terminal's session. `None` when it could not be
/// read; why is logged.
pub fn account_environment() -> Option<Environment> {
    static NEVER: AtomicBool = AtomicBool::new(false);
    if cfg!(windows) {
        return match host::account_environment() {
            Ok(variables) => Some(Environment::from(variables)),
            Err(error) => {
                eprintln!(
                    "platform.account_environment_unread kind={:?} error={error}",
                    error.kind()
                );
                None
            }
        };
    }
    let home = host::home_dir().ok()?;
    let shell = host::default_shell().ok();
    login_shell_environment(&home, shell.as_deref(), &NEVER)
}

/// The program `name` on [`account_path`].
pub fn find_cli(name: &str) -> Option<Program> {
    Program::find(&account_path()?, name)
}

/// A command that runs `program` the way the account's own terminal would:
/// a bare name is found on the account's search ([`account_path_now`]), and
/// the program is run with that `PATH`, so what it starts by name in turn
/// (git's hooks, filters and credential helpers, a script's interpreter, a
/// check's tools) is found the same way. A daemon an SSH exec or a login item
/// started has the system's folders alone on its own `PATH`, which miss what
/// the account installed. Every program a node starts by name is started
/// this way. Only a process's first command waits for the login shell, and
/// at most [`FIRST_READ_WITHIN`]; a caller that must have the shell's
/// current answer (a move's checks, a login item's daemon) asks
/// [`account_path`] and runs [`command_on`] with it.
pub fn command(program: impl AsRef<Path>) -> Command {
    command_on(account_path_now().as_deref(), program)
}

/// How long the account's search, as [`account_path_now`] last read it,
/// serves before it is read again: an installer's edit to a startup file
/// reaches the commands started after the next read.
const ACCOUNT_REREAD: Duration = Duration::from_secs(10);

/// How long a process's first command waits for the first read: the login
/// shell's own deadline, with a second for the startup files' stats.
const FIRST_READ_WITHIN: Duration = Duration::from_secs(LOGIN_SHELL_DEADLINE.as_secs() + 1);

/// The account's search as last read, kept for [`command`].
static ACCOUNT: Latest = Latest::new();

/// [`account_path`], read once per process and then again on a thread of its
/// own once it is [`ACCOUNT_REREAD`] old, a caller taking the answer last
/// read meanwhile. Until a first read has finished the caller waits for it,
/// so a fresh process (the `hide` CLI, a short-lived helper) finds on its
/// first command what only the login shell's `PATH` reaches; a daemon starts
/// that read as it comes up ([`warm_account_path`]), so its own first
/// command seldom waits. A read that did not finish in [`FIRST_READ_WITHIN`]
/// leaves the search without the shell's folders.
pub fn account_path_now() -> Option<OsString> {
    ACCOUNT
        .latest(ACCOUNT_REREAD, FIRST_READ_WITHIN, account_path)
        .or_else(|| cli_path_with(&host::home_dir().ok()?, None))
}

/// Starts the first read of the account's search without waiting for it.
pub fn warm_account_path() {
    let mut state = ACCOUNT.state();
    ACCOUNT.start_due(&mut state, ACCOUNT_REREAD, account_path);
}

/// A value read on a thread of its own and kept: a caller takes the last
/// one at once once there is one, and at most one read runs at a time.
struct Latest {
    state: Mutex<LatestState>,
    finished: Condvar,
}

struct LatestState {
    value: Option<OsString>,
    read: Option<Instant>,
    reading: bool,
}

/// A read under way: whether it returns or panics, it is marked finished
/// and its waiters are woken, so a read that failed never holds the next
/// one off.
struct Reading {
    latest: &'static Latest,
    value: Option<Option<OsString>>,
}

impl Drop for Reading {
    fn drop(&mut self) {
        let mut state = self.latest.state();
        if let Some(value) = self.value.take() {
            state.value = value;
        }
        state.read = Some(Instant::now());
        state.reading = false;
        drop(state);
        self.latest.finished.notify_all();
    }
}

impl Latest {
    const fn new() -> Self {
        Self {
            state: Mutex::new(LatestState {
                value: None,
                read: None,
                reading: false,
            }),
            finished: Condvar::new(),
        }
    }

    fn state(&self) -> MutexGuard<'_, LatestState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The value last read. A read of `read` starts when none runs and the
    /// last finished `every` ago or never; while none has finished, the
    /// caller waits up to `first_within` for one.
    fn latest(
        &'static self,
        every: Duration,
        first_within: Duration,
        read: impl FnOnce() -> Option<OsString> + Send + 'static,
    ) -> Option<OsString> {
        let mut state = self.state();
        self.start_due(&mut state, every, read);
        if state.read.is_none() {
            state = self
                .finished
                .wait_timeout_while(state, first_within, |state| {
                    state.read.is_none() && state.reading
                })
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        state.value.clone()
    }

    /// Starts a read of `read` when none runs and the last finished `every`
    /// ago or never; a thread that cannot start is logged and leaves the
    /// next call to try.
    fn start_due(
        &'static self,
        state: &mut LatestState,
        every: Duration,
        read: impl FnOnce() -> Option<OsString> + Send + 'static,
    ) {
        let due = state.read.is_none_or(|read| read.elapsed() >= every);
        if !due || state.reading {
            return;
        }
        let started = std::thread::Builder::new()
            .name("account-search".to_owned())
            .spawn(move || {
                let mut reading = Reading {
                    latest: self,
                    value: None,
                };
                reading.value = Some(read());
            });
        if let Err(error) = &started {
            eprintln!("platform.account_search_unstarted kind={:?}", error.kind());
        }
        state.reading = started.is_ok();
    }
}

/// [`command`] with `search` as the `PATH` `program` is found on and run
/// with. A path is run as given, with the same `PATH`; a name found nowhere
/// is left as named, so its start fails as not found. `None` (no home to
/// search from) leaves both to this process's own `PATH`.
pub fn command_on(search: Option<&OsStr>, program: impl AsRef<Path>) -> Command {
    let named = program.as_ref();
    let Some(search) = search else {
        return Command::new(named);
    };
    let found = (named.components().count() == 1)
        .then(|| host::find_program(search, named.to_str()?))
        .flatten();
    let mut command = Command::new(found.as_deref().unwrap_or(named));
    command.env("PATH", search);
    command
}

/// [`find_cli`] for a given home and login shell.
pub fn find_cli_with(
    home: &Path,
    login_shell: Option<&Path>,
    stop: &AtomicBool,
    name: &str,
) -> Option<Program> {
    Program::find(&search_path(home, login_shell, stop)?, name)
}

/// A program that was found, and the `PATH` it was found on, which it is run
/// with. A CLI installed as a script starts its interpreter by name (nvm's
/// and pnpm's `codex` run `node`), so a program found on the login shell's
/// folders and run with a daemon's `PATH` dies looking for its interpreter,
/// and a version asked of it with another `PATH` may be another copy's.
/// Whatever finds an agent's CLI hands this on, so what is checked, what is
/// asked and what is run are the one program.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Program {
    path: PathBuf,
    search: OsString,
}

impl Program {
    /// The program `name` on `search`, the first folder that holds it.
    pub fn find(search: &OsStr, name: &str) -> Option<Self> {
        Some(Self {
            path: host::find_program(search, name)?,
            search: search.to_owned(),
        })
    }

    /// A program the caller names by file, run with `search`.
    pub fn named(path: PathBuf, search: OsString) -> Self {
        Self { path, search }
    }

    /// The program the file `path` is, run with `search`, when this account
    /// can start it ([`runnable`]); why it cannot otherwise.
    pub fn at(path: &Path, search: OsString) -> Result<Self, String> {
        runnable(path, Some(&search))?;
        Ok(Self {
            path: path.to_path_buf(),
            search,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn into_path(self) -> PathBuf {
        self.path
    }

    /// The `PATH` the program was found on and is run with.
    pub fn search(&self) -> &OsStr {
        &self.search
    }

    /// How the program starts ([`Start`]), read afresh at each ask: an
    /// update of the agent can move the script its launcher runs.
    pub fn start(&self) -> Start {
        start(&self.path, &self.search)
    }

    /// A command that runs the program with the `PATH` it was found on. A
    /// caller that rebuilds the environment sets [`Self::search`] again
    /// after it.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.path);
        command.env("PATH", &self.search);
        command
    }
}

/// How a program that was found starts so that each argument it is given
/// reaches it whole (with-hide/hide-private#573).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Start {
    /// `program` takes the arguments itself, after `leading`: the program
    /// that was found, on every system unless it is a Windows batch file, or
    /// the program npm's Windows launcher runs, with the words the launcher
    /// puts before the caller's ([`launcher`]).
    Direct {
        program: PathBuf,
        leading: Vec<OsString>,
    },
    /// A Windows batch file that is not npm's launcher: Windows runs it
    /// through `cmd.exe`, which reads the whole command line again, at most
    /// 8,191 characters of it, expands `%` in it and reads `"` and, outside
    /// quotes, `& | < > ^` as its own, so an argument that holds them does
    /// not reach the program as it was given. Why it was not read is logged
    /// (`platform.launcher_unresolved`).
    Batch,
}

/// Whether a start typed into a pane's shell names its program's file
/// rather than its name. On Windows PowerShell finds a name by its own rule,
/// a `.ps1` before the `.cmd` beside it, and npm writes both, so the file
/// [`Program::find`] found and [`Program::start`] read is not the one the
/// shell would run by name. Elsewhere the shell finds a name on the `PATH`
/// as [`Program::find`] does.
pub const START_NAMES_FILE: bool = cfg!(windows);

/// How the program at `path` starts when it runs with `search` as its
/// `PATH`: a Windows batch file is read as npm's launcher, whose Node is the
/// one `search` holds, and anything else takes its arguments itself.
pub fn start(path: &Path, search: &OsStr) -> Start {
    let batch = cfg!(windows)
        && path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
            });
    if !batch {
        return Start::Direct {
            program: path.to_path_buf(),
            leading: Vec::new(),
        };
    }
    match launcher::resolve(path, search, &host::path_extensions()) {
        Ok(start) => start,
        Err(reason) => {
            // The reason only: the launcher's path names the account.
            eprintln!("platform.launcher_unresolved reason={reason:?}");
            Start::Batch
        }
    }
}

/// Whether this account can start the file `path` run with `search` as its
/// `PATH` (this process's own when `None`): a file it may run and, for a
/// script that names its interpreter on its first line (`#!`), an
/// interpreter that is such a file too, found on `search` when the script
/// asks `env` for it. A program that passes and still ends at its start (a
/// library its system lacks) fails as the program it is. Why it cannot,
/// naming the file, otherwise.
pub fn runnable(path: &Path, search: Option<&OsStr>) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if !host::is_program(path) {
        return Err(format!(
            "{} is not a program this account may run",
            path.display()
        ));
    }
    let Some(interpreter) = interpreter(path) else {
        return Ok(());
    };
    let found = match interpreter {
        Interpreter::File(file) => host::is_program(&file).then_some(file.clone()).ok_or(file),
        Interpreter::Named(name) => {
            let search = search
                .map(OsStr::to_owned)
                .or_else(|| std::env::var_os("PATH"))
                .unwrap_or_default();
            host::find_program(&search, &name).ok_or_else(|| PathBuf::from(name))
        }
    };
    found.map(drop).map_err(|missing| {
        format!(
            "{} runs {}, which is not a program here",
            path.display(),
            missing.display()
        )
    })
}

/// The interpreter a script's first line names (`#!`).
enum Interpreter {
    /// By its file.
    File(PathBuf),
    /// By the name `env` finds on the `PATH` (`#!/usr/bin/env node`).
    Named(String),
}

/// The interpreter the file at `path` names on its first line, if it is a
/// script that names one. Only Unix starts a script by its first line.
fn interpreter(path: &Path) -> Option<Interpreter> {
    use std::io::Read;
    if cfg!(windows) {
        return None;
    }
    let mut head = [0_u8; 256];
    let read = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    let line = head[..read].strip_prefix(b"#!")?;
    let line = line.split(|byte| *byte == b'\n').next().unwrap_or(line);
    let line = String::from_utf8_lossy(line);
    let mut words = line.split_whitespace();
    let file = PathBuf::from(words.next()?);
    if file.file_name() != Some(OsStr::new("env")) {
        return Some(Interpreter::File(file));
    }
    // `env`'s own options (`-S`) and the variables it sets come first.
    words
        .find(|word| !word.starts_with('-') && !word.contains('='))
        .map(|name| Interpreter::Named(name.to_owned()))
}

/// The folders installers put a CLI in that a daemon's `PATH` may not reach.
fn usual_install_folders(home: &Path) -> [PathBuf; 9] {
    [
        home.join(".local/bin"),
        // agy's Windows installer: `%LOCALAPPDATA%\agy\bin`, whose default is
        // under the profile folder.
        home.join("AppData").join("Local").join("agy").join("bin"),
        // pnpm's global bin folder: `$PNPM_HOME` up to pnpm 10 and
        // `$PNPM_HOME/bin` from pnpm 11, with `$PNPM_HOME` defaulting to
        // `~/Library/pnpm` on macOS and `~/.local/share/pnpm` on Linux.
        home.join("Library/pnpm"),
        home.join("Library/pnpm/bin"),
        home.join(".local/share/pnpm"),
        home.join(".local/share/pnpm/bin"),
        home.join(".npm-global/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    use super::*;

    const LONG: Duration = Duration::from_secs(3600);

    fn latest() -> &'static Latest {
        Box::leak(Box::new(Latest::new()))
    }

    /// Waits until no read runs.
    fn settled(latest: &Latest) {
        let state = latest.state();
        drop(
            latest
                .finished
                .wait_while(state, |state| state.reading)
                .unwrap(),
        );
    }

    /// The first caller waits for the first read and takes what it read;
    /// once there is a value, a caller takes it at once while the next read
    /// is under way, no second read starts beside it, and the caller after
    /// it takes what it read.
    #[test]
    fn only_the_first_caller_waits_and_later_ones_take_the_last_value() {
        let latest = latest();
        assert_eq!(
            latest.latest(LONG, LONG, || Some(OsString::from("/first/bin"))),
            Some(OsString::from("/first/bin"))
        );

        let (release, released) = mpsc::channel::<()>();
        let reads = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&reads);
        let read = move || {
            counted.fetch_add(1, Ordering::SeqCst);
            released.recv().unwrap();
            Some(OsString::from("/second/bin"))
        };
        assert_eq!(
            latest.latest(Duration::ZERO, LONG, read),
            Some(OsString::from("/first/bin")),
            "the second read is under way"
        );
        assert_eq!(
            latest.latest(Duration::ZERO, LONG, || panic!("a second read beside it")),
            Some(OsString::from("/first/bin"))
        );
        release.send(()).unwrap();
        settled(latest);
        assert_eq!(
            latest.latest(LONG, LONG, || panic!("not due")),
            Some(OsString::from("/second/bin"))
        );
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    /// A read that panics is finished all the same: the caller waiting for
    /// it is answered, and the next due read starts.
    #[test]
    fn a_read_that_panics_holds_off_neither_its_caller_nor_the_next_read() {
        let latest = latest();
        assert_eq!(
            latest.latest(LONG, LONG, || panic!("the read failed")),
            None
        );
        assert_eq!(
            latest.latest(Duration::ZERO, LONG, || Some(OsString::from("/read/bin"))),
            None,
            "the read after it is under way"
        );
        settled(latest);
        assert_eq!(
            latest.latest(LONG, LONG, || panic!("not due")),
            Some(OsString::from("/read/bin"))
        );
    }
}
