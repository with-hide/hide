//! What this machine and the account running on it are called, and where
//! their things live: the home folder, the folder an application keeps its
//! state in, the socket the pinned Herdr listens on by default, the programs
//! on the search path, the `PATH` and the environment the login shell sets
//! up, the account's environment on Windows, the default shell, the Trash, the Tailscale CLI, the account's primary language, and
//! the machine's name and identity.
//!
//! Every function answers for the machine this process runs on, from the
//! variables and tables that system defines, and refuses (`NotFound`, or
//! `Unsupported` where the system has no such thing) rather than guessing
//! when they are absent. A device helper running on another machine asks its
//! own copy, so nothing here assumes the machine is a Unix one.
//!
//! The variables read, and what an absent one means:
//!
//! | Variable | System | Absent |
//! | --- | --- | --- |
//! | `HOME` | macOS, Linux | no home folder, so no state, socket or Trash location |
//! | `USERPROFILE` | Windows | no home folder |
//! | `XDG_STATE_HOME` | Linux | state lives under `~/.local/state` |
//! | `LOCALAPPDATA` | Windows | no state folder |
//! | `HERDR_SOCKET_PATH` | all | use `HERDR_SESSION`, or Herdr's account default socket |
//! | `HERDR_SESSION` | all | use Herdr's default session when no socket overrides it |
//! | `XDG_CONFIG_HOME` | all, as Herdr reads it | Herdr's config folder is the system's own |
//! | `APPDATA` | Windows | Herdr's config folder is under `USERPROFILE` |
//! | `PATH` | all | no program is found |
//! | `PATHEXT` | Windows | the extensions Windows has always run |
//! | `SHELL` | macOS, Linux | no default shell |
//! | `ComSpec` | Windows | no default shell |
//! | `ProgramFiles` | Windows | no Tailscale location |
//! | `HIDE_MACHINE_ID` | all | the machine id the system reports |
//! | `LC_ALL`, `LC_MESSAGES`, `LANG` | Linux, read in that order | no primary language |
//! | `WSL_DISTRO_NAME` | Linux | a WSL distribution is told by its kernel release alone |

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

/// The variable that names the account's home folder.
pub const HOME_VARIABLE: &str = if cfg!(windows) { "USERPROFILE" } else { "HOME" };

/// Every variable this module reads, so a caller that keeps a registry of the
/// variables it depends on can check it holds these.
pub const VARIABLES: &[&str] = &[
    HOME_VARIABLE,
    "XDG_STATE_HOME",
    "LOCALAPPDATA",
    "HERDR_SOCKET_PATH",
    "HERDR_SESSION",
    "XDG_CONFIG_HOME",
    "APPDATA",
    "PATH",
    "PATHEXT",
    "SHELL",
    "ComSpec",
    "ProgramFiles",
    MACHINE_ID_VARIABLE,
    "LC_ALL",
    "LC_MESSAGES",
    "LANG",
    "WSL_DISTRO_NAME",
];

/// Names this process's machine for [`machine_id`] in place of the one the
/// system reports: only a test fixture sets it.
pub const MACHINE_ID_VARIABLE: &str = "HIDE_MACHINE_ID";

/// How a caller with its own record of the environment (a registry that
/// validates at start, a test) hands it to the `_from` functions: the value
/// of a variable, `None` when it is unset.
pub type Variables<'a> = &'a dyn Fn(&str) -> Option<OsString>;

fn process(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// The raw value of a variable this process was started with, `None` when
/// unset: the capture for a caller that judges a variable it does not own
/// here, such as an agent's own folder variable
/// (`hide_agent_adapter::AgentAdapter::home`), which decides what the value
/// means.
pub fn variable(name: &str) -> Option<OsString> {
    process(name)
}

/// The account's home folder.
pub fn home_dir() -> io::Result<PathBuf> {
    home_dir_from(&process)
}

/// [`home_dir`] from the caller's variables.
pub fn home_dir_from(variables: Variables) -> io::Result<PathBuf> {
    absolute_variable(variables, HOME_VARIABLE)
}

/// The folder applications keep the account's state in on this system:
/// `~/Library/Application Support` on macOS, `$XDG_STATE_HOME` or
/// `~/.local/state` on Linux, `%LOCALAPPDATA%` on Windows. A caller keeps its
/// own folder inside it.
pub fn state_dir() -> io::Result<PathBuf> {
    state_dir_from(&process)
}

/// [`state_dir`] from the caller's variables.
pub fn state_dir_from(variables: Variables) -> io::Result<PathBuf> {
    if cfg!(windows) {
        absolute_variable(variables, "LOCALAPPDATA")
    } else if !cfg!(target_os = "macos") && nonempty_variable(variables, "XDG_STATE_HOME").is_some()
    {
        absolute_variable(variables, "XDG_STATE_HOME")
    } else {
        Ok(state_dir_under(&home_dir_from(variables)?))
    }
}

/// The state folder this system's convention puts under `home` when no
/// variable moves it: `Library/Application Support` on macOS,
/// `.local/state` on Linux, `AppData\Local` on Windows. It is for a caller
/// whose state follows a home it was handed (an isolated run) rather than
/// the account's.
pub fn state_dir_under(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else if cfg!(windows) {
        home.join("AppData").join("Local")
    } else {
        home.join(".local").join("state")
    }
}

/// The socket the pinned Herdr's default session listens on when nothing
/// overrides it: `herdr.sock` in Herdr's config folder, which is
/// `$XDG_CONFIG_HOME/herdr` wherever that is set, and otherwise
/// `~/.config/herdr` on macOS and Linux and `%APPDATA%\herdr` on Windows
/// (whose socket is a named pipe of that name). `HERDR_SOCKET_PATH` and a
/// named session are the caller's to apply before asking.
pub fn herdr_socket_default() -> io::Result<PathBuf> {
    herdr_socket_default_from(&process)
}

/// [`herdr_socket_default`] from the caller's variables.
pub fn herdr_socket_default_from(variables: Variables) -> io::Result<PathBuf> {
    Ok(herdr_config_dir_from(variables)?.join("herdr.sock"))
}

/// The account socket, honoring the environment override before the
/// named environment session and system default. An explicit CLI session
/// is the caller's and takes precedence over both environment selectors.
pub fn herdr_socket_from(variables: Variables) -> io::Result<PathBuf> {
    if variables("HERDR_SOCKET_PATH").is_some() {
        let socket = absolute_variable(variables, "HERDR_SOCKET_PATH")?;
        if socket
            .as_os_str()
            .to_string_lossy()
            .chars()
            .any(char::is_control)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "HERDR_SOCKET_PATH must be single-line",
            ));
        }
        return Ok(socket);
    }
    let folder = herdr_config_dir_from(variables)?;
    match variables("HERDR_SESSION") {
        None => Ok(folder.join("herdr.sock")),
        Some(session) => {
            let session = session
                .to_str()
                .filter(|session| valid_herdr_session(session))
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "HERDR_SESSION is not a valid session name",
                    )
                })?;
            if session == "default" {
                Ok(folder.join("herdr.sock"))
            } else {
                Ok(folder.join("sessions").join(session).join("herdr.sock"))
            }
        }
    }
}

/// A session name the pinned Herdr accepts, safe as one path component.
pub fn valid_herdr_session(session: &str) -> bool {
    !session.is_empty()
        && session.len() <= 64
        && !matches!(session, "." | "..")
        && session
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

/// Herdr's config folder, resolved in Herdr's own order (`config::io` of the
/// pinned release); where Herdr would fall back to a temporary folder, this
/// says there is none.
pub fn herdr_config_dir_from(variables: Variables) -> io::Result<PathBuf> {
    if variables("XDG_CONFIG_HOME").is_some() {
        return Ok(absolute_variable(variables, "XDG_CONFIG_HOME")?.join("herdr"));
    }
    if cfg!(windows) {
        if variables("APPDATA").is_some() {
            return Ok(absolute_variable(variables, "APPDATA")?.join("herdr"));
        }
        return Ok(home_dir_from(variables)?
            .join("AppData")
            .join("Roaming")
            .join("herdr"));
    }
    Ok(home_dir_from(variables)?.join(".config").join("herdr"))
}

/// The search path this process finds programs on (`PATH`), as whoever
/// started it set it. It starts no shell, so a folder the account's startup
/// files add (`~/.zshrc`, `~/.bashrc`) is on it only when the process was
/// started from a terminal: the desktop app opened from the Dock adds the
/// system folders and the usual install folders, and an SSH exec channel has
/// the system folders alone. [`login_shell`] asks the shell.
pub fn login_path() -> io::Result<OsString> {
    std::env::var_os("PATH")
        .filter(|path| !path.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PATH is not set"))
}

/// The marks the login shell prints its `PATH`, and then its environment,
/// between, so nothing its startup files print before or after can be read
/// as part of either.
const SHELL_PATH_MARK: &str = "__HIDE_LOGIN_PATH__";
const SHELL_ENVIRONMENT_MARK: &str = "__HIDE_LOGIN_ENVIRONMENT__";

/// Variables a shell sets about itself, not about the account: its folder,
/// the one before, its depth and the last command it ran. A program started
/// from the account's environment gets its own.
const SHELL_OWN_VARIABLES: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_"];

/// What the account's login shell sets up.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginShell {
    /// Its `PATH`, which is what the operator's terminal searches.
    pub path: OsString,
    /// Every variable it exports but the shell's own (`PWD`, `OLDPWD`,
    /// `SHLVL`, `_`), as `env` lists them: what the account's startup files
    /// export over the login variables, such as a provider key an agent's
    /// configuration reads. A value that is not UTF-8 is left out. Why it
    /// could not be read when it could not: a shell whose `env` cannot end
    /// each variable with a NUL, or a listing past the output a run keeps
    /// ([`crate::process::RUN_OUTPUT_CAP`]); the `PATH` is read apart from
    /// it, so it still holds.
    pub environment: Result<Vec<(OsString, OsString)>, String>,
}

/// The `PATH` and the environment the account's login shell sets up, which
/// is what the operator's terminal has: `shell -ilc` runs the startup files
/// an installer or the operator edits (`-i` reads `~/.zshrc` and `~/.bashrc`,
/// `-l` the login files) and prints the `PATH` and then `env -0` between
/// marks, past anything those files print.
///
/// The shell gets the account's login variables only
/// ([`crate::process::LOGIN_CHILD_VARIABLES`]), with `HOME` set to `home`,
/// plus the variables that keep Oh My Zsh's update prompt and its tmux
/// autostart from holding an interactive shell open, the ones the desktop
/// host sets when it asks the same question (`desktop/src/main/cli.ts`); so
/// its environment is the account's, never this process's or the terminal's
/// that started it. The shell and everything it started end when `deadline`
/// passes (`TimedOut`) or `stop` is raised (`Interrupted`); a shell that
/// ends without printing the `PATH` marks says `InvalidData`. Windows has no
/// login shell: a process there is given the account's own `Path`
/// ([`account_environment`] reads the rest), so it says `Unsupported`.
pub fn login_shell(
    shell: &Path,
    home: &Path,
    deadline: std::time::Duration,
    stop: &std::sync::atomic::AtomicBool,
) -> io::Result<LoginShell> {
    use crate::process::{RunFailure, restrict_to_login_environment, run_to_end};
    if cfg!(windows) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows has no login shell; a process already has the account's Path",
        ));
    }
    let mut command = std::process::Command::new(shell);
    // `command` skips an alias or a function the startup files named `env`.
    command.arg("-ilc").arg(format!(
        "printf '\\n{SHELL_PATH_MARK}%s{SHELL_PATH_MARK}\\n' \"$PATH\"; printf '{SHELL_ENVIRONMENT_MARK}'; command env -0; printf '{SHELL_ENVIRONMENT_MARK}\\n'"
    ));
    restrict_to_login_environment(&mut command);
    command
        .env(HOME_VARIABLE, home)
        .env("DISABLE_AUTO_UPDATE", "true")
        .env("ZSH_TMUX_AUTOSTARTED", "true")
        .env("ZSH_TMUX_AUTOSTART", "false");
    let finished = run_to_end(&mut command, deadline, stop).map_err(|failure| match failure {
        RunFailure::Start(error) | RunFailure::Wait(error) => error,
        RunFailure::TimedOut => io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{} did not answer in time", shell.display()),
        ),
        RunFailure::Stopped => io::Error::new(io::ErrorKind::Interrupted, "stopped"),
    })?;
    let path = marked(&finished.stdout, SHELL_PATH_MARK)
        .filter(|path| !path.is_empty())
        .map(OsString::from)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{} printed no PATH (exit {:?})",
                    shell.display(),
                    finished.code
                ),
            )
        })?;
    let environment = marked(&finished.stdout, SHELL_ENVIRONMENT_MARK)
        .ok_or_else(|| format!("{} printed no environment", shell.display()))
        .and_then(|listing| {
            listed_environment(listing).ok_or_else(|| {
                format!(
                    "{}'s env did not end each variable with a NUL",
                    shell.display()
                )
            })
        });
    Ok(LoginShell { path, environment })
}

/// The text between the first two `mark`s.
fn marked<'a>(output: &'a str, mark: &str) -> Option<&'a str> {
    let (_, after) = output.split_once(mark)?;
    let (text, _) = after.split_once(mark)?;
    Some(text)
}

/// The variables `env -0` listed: each `NAME=value` ends with a NUL, so a
/// value may hold line breaks. A listing with no NUL came from an `env` that
/// ignored `-0` or failed (a shell always has `HOME`), and is refused.
fn listed_environment(listing: &str) -> Option<Vec<(OsString, OsString)>> {
    if !listing.contains('\0') {
        return None;
    }
    Some(
        listing
            .split('\0')
            .filter_map(|entry| entry.split_once('='))
            .filter(|(name, value)| {
                !name.is_empty()
                    && !value.contains(char::REPLACEMENT_CHARACTER)
                    && !SHELL_OWN_VARIABLES.contains(name)
            })
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect(),
    )
}

/// The environment Windows builds for a new sign-in of the account this
/// process runs as: the system's variables and the account's own, as
/// System Properties and `setx` keep them, which is what Explorer gives a
/// terminal it starts, never this process's own (a variable whoever started
/// this process set, a terminal's session markers). Only Windows builds one:
/// macOS and Linux say `Unsupported`, where [`login_shell`] reads it.
pub fn account_environment() -> io::Result<Vec<(OsString, OsString)>> {
    sys::account_environment()
}

/// The program `name` as this system's shell would find it on `path`: the
/// first folder holding it, where a file without an execute bit is not a
/// program on Unix, and on Windows a name without an extension is tried with
/// each extension `PATHEXT` names.
pub fn find_program(path: &OsStr, name: &str) -> Option<PathBuf> {
    let names = program_names(name);
    std::env::split_paths(path)
        .filter(|folder| folder.is_absolute())
        .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
        .find(|candidate| is_program(candidate))
}

fn program_names(name: &str) -> Vec<OsString> {
    if !cfg!(windows) || Path::new(name).extension().is_some() {
        return vec![OsString::from(name)];
    }
    names_with_extensions(name, &path_extensions())
}

/// The extensions Windows tries on a name without one, as `PATHEXT` lists
/// them, or the ones Windows has always run when it names none.
pub(crate) fn path_extensions() -> String {
    std::env::var("PATHEXT")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_owned())
}

/// `name` with each extension `extensions` lists (`PATHEXT`'s form), in its
/// order.
pub(crate) fn names_with_extensions(name: &str, extensions: &str) -> Vec<OsString> {
    extensions
        .split(';')
        .filter(|extension| extension.starts_with('.'))
        .map(|extension| OsString::from(format!("{name}{}", extension.to_ascii_lowercase())))
        .collect()
}

pub(crate) fn is_program(candidate: &Path) -> bool {
    candidate.is_file() && crate::fs::permissions::is_executable(candidate).unwrap_or(false)
}

/// The shell the account runs commands in by default: `$SHELL` on macOS and
/// Linux, `%ComSpec%` (cmd.exe) on Windows.
pub fn default_shell() -> io::Result<PathBuf> {
    absolute_variable(&process, if cfg!(windows) { "ComSpec" } else { "SHELL" })
}

/// Moves the file or folder at `path` to the system's Trash (the Recycle
/// Bin on Windows, the account's freedesktop trash on Linux), from which the
/// operator can restore it. On macOS the item goes through `NSFileManager`,
/// not Finder: Finder's route runs `osascript` and asks for Automation
/// permission on first use, which an SSH session cannot answer, and the file
/// manager's needs neither a permission nor a subprocess. What it gives up is
/// Finder's "Put Back" on some systems.
pub fn trash(path: &Path) -> io::Result<()> {
    // Only the macOS branch below changes the context.
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut context = trash::TrashContext::default();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        context.set_delete_method(DeleteMethod::NsFileManager);
    }
    context.delete(path).map_err(|error| match error {
        trash::Error::CouldNotAccess { .. } => {
            io::Error::new(io::ErrorKind::PermissionDenied, "it is not accessible")
        }
        trash::Error::TargetedRoot => {
            io::Error::new(io::ErrorKind::InvalidInput, "it is a volume root")
        }
        trash::Error::Unknown { description } | trash::Error::Os { description, .. } => {
            io::Error::other(description)
        }
        other => io::Error::other(other.to_string()),
    })
}

/// Where the Tailscale installer puts its CLI outside the search path: the
/// app bundle's binary on macOS, `%ProgramFiles%\Tailscale\tailscale.exe` on
/// Windows. Linux packages put `tailscale` on the search path, so there is no
/// such place there (`Unsupported`), and [`find_program`] finds it.
pub fn tailscale_cli() -> io::Result<PathBuf> {
    if cfg!(target_os = "macos") {
        Ok(PathBuf::from(
            "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        ))
    } else if cfg!(windows) {
        Ok(absolute_variable(&process, "ProgramFiles")?
            .join("Tailscale")
            .join("tailscale.exe"))
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Linux installs tailscale on the search path",
        ))
    }
}

/// The addresses this machine's network interfaces hold now, loopback
/// left out: a change in the set means the machine moved to another
/// network. `Unsupported` on Windows, where nothing asks yet.
pub fn network_addresses() -> io::Result<std::collections::BTreeSet<std::net::IpAddr>> {
    sys::network_addresses()
}

/// The machine's host name, as the system reports it.
pub fn name() -> io::Result<String> {
    let name = sys::name()?;
    let name = name.trim();
    if name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the system reports an empty host name",
        ));
    }
    Ok(name.to_owned())
}

/// The machine's own identity, which survives a rename and a reinstall of
/// Hide: the hardware UUID on macOS (`IOPlatformUUID`), `/etc/machine-id` on
/// Linux, the `MachineGuid` Windows keeps in its registry.
/// Its trimmed lowercase spelling is shared with stored lineage tokens.
///
/// [`MACHINE_ID_VARIABLE`] names another machine instead, so a test runs a
/// node and the core it dials as two machines on one host.
pub fn machine_id() -> io::Result<String> {
    let id = match nonempty_variable(&process, MACHINE_ID_VARIABLE) {
        Some(id) => id.into_string().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "the machine id variable is not text",
            )
        })?,
        None => sys::machine_id()?,
    };
    let id = id.trim();
    if id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the system reports an empty machine id",
        ));
    }
    Ok(id.to_lowercase())
}

/// The account's primary language as a BCP 47 tag (`en-US`, `ko-KR`,
/// `zh-Hans-CN`): the first of the preferred languages on macOS
/// (`AppleLanguages`), the user locale on Windows (`LocaleName`), and the
/// messages locale on Linux. Only the first is answered; a caller does not
/// search later preferences. `NotFound` when the system names none, as the
/// `C` and `POSIX` locales name none.
pub fn primary_language() -> io::Result<String> {
    if cfg!(any(target_os = "macos", windows)) {
        sys::primary_language()
    } else {
        primary_language_from(&process)
    }
}

/// The messages locale a POSIX system's variables name (`ko_KR.UTF-8` reads
/// as `ko-KR`), the form Linux answers with.
pub fn primary_language_from(variables: Variables) -> io::Result<String> {
    let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| nonempty_variable(variables, name))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no locale variable is set"))?;
    let value = value.to_string_lossy();
    // `language_TERRITORY.codeset@modifier`: the codeset and modifier say
    // nothing about the language.
    let name = value.split(['.', '@']).next().unwrap_or_default();
    if name.is_empty() || name == "C" || name == "POSIX" {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the locale names no language",
        ));
    }
    Ok(name.replace('_', "-"))
}

/// Where this machine shows the drives of the Windows PC it runs on, when it
/// is a WSL distribution (PRD wsl-device B16): `/mnt` unless the
/// distribution's `/etc/wsl.conf` moves them (`[automount]` `root`).
/// `NotFound` on a machine that is not one, `Unsupported` off Linux.
pub fn windows_drives() -> io::Result<String> {
    if !cfg!(target_os = "linux") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "only a Linux machine can be a WSL distribution",
        ));
    }
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease")?;
    let config = match std::fs::read_to_string("/etc/wsl.conf") {
        Ok(config) => Some(config),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    windows_drives_from(&release, &process, config.as_deref())
}

/// [`windows_drives`] from a kernel release, the variables and the text of
/// `/etc/wsl.conf` (`None` when there is none). A WSL kernel names itself
/// `microsoft` in its release (`5.15.153.1-microsoft-standard-WSL2`, WSL1's
/// `4.4.0-19041-Microsoft`), and a shell `wsl.exe` started has
/// `WSL_DISTRO_NAME` set; either says the machine is one. A `root` that is
/// not an absolute path is refused as `InvalidData` rather than guessed past.
pub fn windows_drives_from(
    kernel_release: &str,
    variables: Variables,
    wsl_conf: Option<&str>,
) -> io::Result<String> {
    let wsl = kernel_release.to_ascii_lowercase().contains("microsoft")
        || nonempty_variable(variables, "WSL_DISTRO_NAME").is_some();
    if !wsl {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "this machine is not a WSL distribution",
        ));
    }
    let Some(root) = wsl_conf.and_then(automount_root) else {
        return Ok("/mnt".to_owned());
    };
    let trimmed = root.trim_end_matches('/');
    if !root.starts_with('/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("/etc/wsl.conf names automount root {root:?}, which is not an absolute path"),
        ));
    }
    Ok(if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    })
}

/// The `root` key of `wsl.conf`'s `[automount]` section, without quotes.
fn automount_root(config: &str) -> Option<String> {
    let mut section = String::new();
    let mut root = None;
    for line in config.lines() {
        let line = line.split(['#', ';']).next().unwrap_or_default().trim();
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = name.trim().to_ascii_lowercase();
        } else if section == "automount"
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("root")
        {
            root = Some(value.trim().trim_matches('"').to_owned());
        }
    }
    root.filter(|root| !root.is_empty())
}

fn nonempty_variable(variables: Variables, name: &str) -> Option<OsString> {
    variables(name).filter(|value| !value.is_empty())
}

/// The variable `name` as an absolute path: `NotFound` when it is unset or
/// empty, `InvalidInput` when it names a relative path.
fn absolute_variable(variables: Variables, name: &str) -> io::Result<PathBuf> {
    let value = nonempty_variable(variables, name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{name} is not set")))?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} is not an absolute path"),
        ));
    }
    Ok(path)
}

#[cfg(unix)]
mod sys {
    use std::io;

    pub(super) fn account_environment() -> io::Result<Vec<(std::ffi::OsString, std::ffi::OsString)>>
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "macOS and Linux build no sign-in environment; the login shell sets it up",
        ))
    }

    pub(super) fn network_addresses() -> io::Result<std::collections::BTreeSet<std::net::IpAddr>> {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        let mut first: *mut libc::ifaddrs = std::ptr::null_mut();
        // SAFETY: getifaddrs fills `first` with a list it allocates, which
        // freeifaddrs releases below on every path.
        if unsafe { libc::getifaddrs(&mut first) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut addresses = std::collections::BTreeSet::new();
        let mut entry = first;
        while !entry.is_null() {
            // SAFETY: `entry` is a node of the list getifaddrs returned, which
            // lives until freeifaddrs.
            let interface = unsafe { &*entry };
            entry = interface.ifa_next;
            let up = interface.ifa_flags & libc::IFF_UP as libc::c_uint != 0;
            let loopback = interface.ifa_flags & libc::IFF_LOOPBACK as libc::c_uint != 0;
            if !up || loopback || interface.ifa_addr.is_null() {
                continue;
            }
            // SAFETY: a non-null ifa_addr points at a sockaddr whose family
            // says which sockaddr it is.
            let family = i32::from(unsafe { (*interface.ifa_addr).sa_family });
            let address = match family {
                libc::AF_INET => {
                    // SAFETY: an AF_INET address is a sockaddr_in.
                    let address = unsafe { &*(interface.ifa_addr as *const libc::sockaddr_in) };
                    IpAddr::V4(Ipv4Addr::from(u32::from_be(address.sin_addr.s_addr)))
                }
                libc::AF_INET6 => {
                    // SAFETY: an AF_INET6 address is a sockaddr_in6.
                    let address = unsafe { &*(interface.ifa_addr as *const libc::sockaddr_in6) };
                    IpAddr::V6(Ipv6Addr::from(address.sin6_addr.s6_addr))
                }
                _ => continue,
            };
            addresses.insert(address);
        }
        // SAFETY: `first` came from getifaddrs and is freed once.
        unsafe { libc::freeifaddrs(first) };
        Ok(addresses)
    }

    pub(super) fn name() -> io::Result<String> {
        let mut buffer = [0u8; 256];
        // SAFETY: the buffer outlives the call and its length is passed with it.
        if unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let end = buffer
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(buffer.len());
        Ok(String::from_utf8_lossy(&buffer[..end]).into_owned())
    }

    #[cfg(target_os = "macos")]
    pub(super) fn machine_id() -> io::Result<String> {
        let output = std::process::Command::new("/usr/sbin/ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "ioreg exited with {}",
                output.status
            )));
        }
        machine_id_from_ioreg(&String::from_utf8_lossy(&output.stdout))
    }

    #[cfg(target_os = "macos")]
    pub(super) fn primary_language() -> io::Result<String> {
        let output = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the account names no AppleLanguages",
            ));
        }
        first_apple_language(&String::from_utf8_lossy(&output.stdout))
    }

    /// The first entry of `defaults`'s property-list array: `(\n "en-US",\n ko\n)`.
    #[cfg(target_os = "macos")]
    fn first_apple_language(output: &str) -> io::Result<String> {
        let first = output
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && *line != "(" && *line != ")")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "AppleLanguages is empty"))?;
        let first = first.trim_end_matches(',');
        let first = first
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(first);
        if first.is_empty() || !first.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "AppleLanguages names an invalid language",
            ));
        }
        Ok(first.to_owned())
    }

    #[cfg(target_os = "macos")]
    fn machine_id_from_ioreg(output: &str) -> io::Result<String> {
        let value = output
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key.trim() == "\"IOPlatformUUID\"").then(|| value.trim())
            })
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "ioreg names no IOPlatformUUID")
            })?;
        value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .filter(|value| !value.trim().is_empty() && !value.contains('"'))
            .map(str::to_owned)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "ioreg reports an invalid IOPlatformUUID string",
                )
            })
    }

    #[cfg(all(test, target_os = "macos"))]
    mod tests {
        use super::*;

        #[test]
        fn the_primary_language_is_the_first_entry_quoted_or_not() {
            assert_eq!(
                first_apple_language("(\n    \"en-US\",\n    \"ko-KR\"\n)\n").unwrap(),
                "en-US"
            );
            assert_eq!(
                first_apple_language("(\n    ko,\n    en\n)\n").unwrap(),
                "ko"
            );
            assert_eq!(
                first_apple_language("(\n)\n").unwrap_err().kind(),
                io::ErrorKind::NotFound
            );
            assert_eq!(
                first_apple_language("(\n    \"en US\"\n)\n")
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidData
            );
        }

        #[test]
        fn ioreg_machine_identity_is_the_quoted_property_value() {
            let output = r#"+-o platform
    {
      "IOPlatformSerialNumber" = "fixture-serial"
      "IOPlatformUUID" = "ABCDEF12-3456-7890-ABCD-EF1234567890"
    }
"#;
            assert_eq!(
                machine_id_from_ioreg(output).unwrap(),
                "ABCDEF12-3456-7890-ABCD-EF1234567890"
            );
        }

        #[test]
        fn ioreg_machine_identity_refuses_missing_and_malformed_properties() {
            for output in [
                "",
                "\"OtherIOPlatformUUID\" = \"fixture\"",
                "\"name\" = \"IOPlatformUUID\"",
            ] {
                assert_eq!(
                    machine_id_from_ioreg(output).unwrap_err().kind(),
                    io::ErrorKind::NotFound
                );
            }
            for value in [
                "unquoted",
                "\"unterminated",
                "\"\"",
                "\"   \"",
                "\"fixture\" \"extra\"",
            ] {
                assert_eq!(
                    machine_id_from_ioreg(&format!("\"IOPlatformUUID\" = {value}"))
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::InvalidData
                );
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn machine_id() -> io::Result<String> {
        std::fs::read_to_string("/etc/machine-id")
    }

    /// Linux reads its locale variables in `host::primary_language`.
    #[cfg(not(target_os = "macos"))]
    pub(super) fn primary_language() -> io::Result<String> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the locale variables name the language",
        ))
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::OsString;
    use std::io;
    use std::os::windows::ffi::OsStringExt;
    use std::ptr::null_mut;

    pub(super) fn network_addresses() -> io::Result<std::collections::BTreeSet<std::net::IpAddr>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the network address set is not read on Windows yet",
        ))
    }

    pub(super) fn account_environment() -> io::Result<Vec<(OsString, OsString)>> {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{TOKEN_DUPLICATE, TOKEN_QUERY};
        use windows_sys::Win32::System::Environment::{
            CreateEnvironmentBlock, DestroyEnvironmentBlock,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        let mut token: HANDLE = null_mut();
        // SAFETY: the current-process pseudo handle is always valid.
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE,
                &mut token,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut block: *mut std::ffi::c_void = null_mut();
        // SAFETY: `token` is open with the rights the call needs, and `block`
        // receives an allocation DestroyEnvironmentBlock frees below. Not
        // inheriting leaves this process's own variables out.
        let made = unsafe { CreateEnvironmentBlock(&mut block, token, 0) };
        let made_error = io::Error::last_os_error();
        // SAFETY: `token` was opened above and is closed once.
        unsafe { CloseHandle(token) };
        if made == 0 {
            return Err(made_error);
        }
        let mut variables = Vec::new();
        let mut cursor = block.cast::<u16>().cast_const();
        loop {
            // SAFETY: the block is a run of NUL-terminated `NAME=value`
            // strings ended by an empty one, so every unit read up to a NUL
            // lies inside it.
            let entry = unsafe {
                let mut length = 0;
                while *cursor.add(length) != 0 {
                    length += 1;
                }
                std::slice::from_raw_parts(cursor, length)
            };
            if entry.is_empty() {
                break;
            }
            // A name starting with `=` is a drive's folder (`=C:`), not a
            // variable.
            if entry[0] != u16::from(b'=')
                && let Some(split) = entry.iter().position(|unit| *unit == u16::from(b'='))
            {
                variables.push((
                    OsString::from_wide(&entry[..split]),
                    OsString::from_wide(&entry[split + 1..]),
                ));
            }
            // SAFETY: the entry's NUL is inside the block, and the block's
            // end is the empty entry after it.
            cursor = unsafe { cursor.add(entry.len() + 1) };
        }
        // SAFETY: `block` came from CreateEnvironmentBlock and is freed once.
        unsafe { DestroyEnvironmentBlock(block) };
        Ok(variables)
    }

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY, RegGetValueW,
    };
    use windows_sys::Win32::System::SystemInformation::{
        ComputerNameDnsHostname, GetComputerNameExW,
    };

    pub(super) fn name() -> io::Result<String> {
        let mut buffer = vec![0u16; 256];
        loop {
            let mut length = buffer.len() as u32;
            // SAFETY: `buffer` holds `length` writable units.
            if unsafe {
                GetComputerNameExW(ComputerNameDnsHostname, buffer.as_mut_ptr(), &mut length)
            } != 0
            {
                return Ok(OsString::from_wide(&buffer[..length as usize])
                    .to_string_lossy()
                    .into_owned());
            }
            let error = io::Error::last_os_error();
            // ERROR_MORE_DATA: `length` now holds the size the name needs.
            if error.raw_os_error() != Some(234) || length as usize <= buffer.len() {
                return Err(error);
            }
            buffer.resize(length as usize, 0);
        }
    }

    pub(super) fn machine_id() -> io::Result<String> {
        let key = widestring::U16CString::from_str("SOFTWARE\\Microsoft\\Cryptography")
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let value = widestring::U16CString::from_str("MachineGuid")
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let mut buffer = vec![0u16; 64];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: the names are NUL-terminated and `buffer` holds `size`
        // writable bytes. The 64-bit view is asked for, so a 32-bit process
        // reads the same id as everything else on the machine.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
                null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let units = (size as usize / 2).saturating_sub(1);
        Ok(OsString::from_wide(&buffer[..units])
            .to_string_lossy()
            .into_owned())
    }

    /// The user locale Windows keeps for the account (`LocaleName`, such as
    /// `ko-KR`).
    pub(super) fn primary_language() -> io::Result<String> {
        let key = widestring::U16CString::from_str("Control Panel\\International")
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let value = widestring::U16CString::from_str("LocaleName")
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let mut buffer = vec![0u16; 86];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: the names are NUL-terminated and `buffer` holds `size`
        // writable bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let units = (size as usize / 2).saturating_sub(1);
        let name = OsString::from_wide(&buffer[..units])
            .to_string_lossy()
            .into_owned();
        if name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the account names no locale",
            ));
        }
        Ok(name)
    }
}
