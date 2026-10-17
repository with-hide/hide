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
//! after a first read has warmed it.
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
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use crate::host;

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

/// What the login shell answered, with the startup files it was read under
/// and when it was asked.
struct ShellAnswer {
    stamps: StartupStamps,
    path: Option<OsString>,
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
        return known.path.clone();
    }
    let asked = Instant::now();
    let path = match crate::host::login_shell_path(shell, home, LOGIN_SHELL_DEADLINE, stop) {
        Ok(path) => Some(path),
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
            None
        }
    };
    *answer = Some(ShellAnswer {
        stamps,
        path: path.clone(),
        asked,
    });
    path
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

/// The program `name` on [`account_path`].
pub fn find_cli(name: &str) -> Option<PathBuf> {
    host::find_program(&account_path()?, name)
}

/// A command that runs `program` the way the account's own terminal would:
/// a bare name is found on [`account_path`], and the program is run with
/// that `PATH`, so what it starts by name in turn (git's hooks, filters and
/// credential helpers, a script's interpreter, a check's tools) is found the
/// same way. A daemon an SSH exec or a login item started has the system's
/// folders alone on its own `PATH`, which miss what the account installed.
/// Every program a node starts by name is started this way.
pub fn command(program: impl AsRef<Path>) -> Command {
    command_on(account_path().as_deref(), program)
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
) -> Option<PathBuf> {
    host::find_program(&search_path(home, login_shell, stop)?, name)
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
