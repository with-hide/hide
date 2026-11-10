//! The state route captured by the kit, independent of an agent's shell.
//!
//! A hook may lose Hide's environment while keeping the shell's XDG state
//! home. The installed command therefore carries the kit's state folder.
//! An explicit pane route still wins; the captured route replaces only the
//! environment-based default, never the daemon's caller authentication.
//!
//! The command also carries the installation that wrote it (`--install-b64`):
//! an account can hold more than one Hide installation, a desktop app and a
//! device helper a Hide on another machine put there, each with its own hook
//! entry, and a pane names the installation that started it
//! ([`INSTALL_VARIABLE`]). An entry acts only in a pane of its own
//! installation, or in one that names none (docs/agent-hooks.md, More than
//! one Hide on an account).

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

pub(crate) const ARGUMENT: &str = "--state-dir-b64";
pub(crate) const INSTALL_ARGUMENT: &str = "--install-b64";
const STATE_VARIABLE: &str = "HIDE_STATE_DIR";
const XDG_VARIABLE: &str = "XDG_STATE_HOME";
/// The variable a pane names the Hide installation that started it by: a
/// pane holder's pane carries its node's ([`APP_INSTALL`] or [`install_of_root`]), and a
/// pane that names none (a Herdr pane, a plain terminal) is anyone's.
pub const INSTALL_VARIABLE: &str = "HIDE_INSTALL";
/// The variable a pane names its installation's own `hide` command by: a
/// `hide` that another installation put first on the pane's `PATH` hands the
/// call to this one, which speaks its own daemon's protocol.
pub const CLI_VARIABLE: &str = "HIDE_INSTALL_CLI";
const ENCODED_LIMIT: usize = 32 * 1024;
/// The longest installation name an entry carries.
const INSTALL_LIMIT: usize = 4 * 1024;

/// The installation of a desktop package's kit: one per account, whose
/// parts are the package's own.
pub const APP_INSTALL: &str = "app";

/// The installation whose kit lives under the helper root `root`, a device
/// helper's or a build Hide placed there: named by the root, which each of
/// its builds shares.
pub fn install_of_root(root: &Path) -> String {
    format!("root:{}", root.display())
}

/// The installed helper, the daemon state folder its kit serves and the
/// installation the kit belongs to.
#[derive(Clone, Copy, Debug)]
pub struct HookTarget<'a> {
    helper: &'a Path,
    state: Option<&'a Path>,
    install: Option<&'a str>,
}

impl<'a> HookTarget<'a> {
    pub fn new(helper: &'a Path, state: &'a Path) -> Result<Self, &'static str> {
        let text = state
            .to_str()
            .ok_or("the hook state folder must be UTF-8")?;
        if !state.is_absolute() || text.contains('\0') {
            return Err("the hook state folder must be an absolute path without NUL bytes");
        }
        if base64::encoded_len(text.len(), false).is_none_or(|length| length > ENCODED_LIMIT) {
            return Err("the hook state folder exceeds the encoded route limit");
        }
        Ok(Self {
            helper,
            state: Some(state),
            install: None,
        })
    }

    /// The same target, its entry naming `install` as the installation that
    /// wrote it.
    pub fn of_install(self, install: &'a str) -> Result<Self, &'static str> {
        if install.is_empty() || install.len() > INSTALL_LIMIT || install.contains('\0') {
            return Err("the installation name must be 1 to 4096 bytes without NUL");
        }
        Ok(Self {
            install: Some(install),
            ..self
        })
    }

    /// The installation this target's entry names, if any.
    pub fn install(self) -> Option<&'a str> {
        self.install
    }

    /// Shell-safe on POSIX and PowerShell, including paths containing quotes,
    /// dollars or spaces. These are arguments, not shell environment syntax.
    pub(crate) fn arguments(self) -> String {
        let route = self.state.map_or_else(String::new, |state| {
            format!(
                " {ARGUMENT} {}",
                URL_SAFE_NO_PAD.encode(
                    state
                        .to_str()
                        .expect("validated hook state folder")
                        .as_bytes()
                )
            )
        });
        let install = self.install.map_or_else(String::new, |install| {
            format!(
                " {INSTALL_ARGUMENT} {}",
                URL_SAFE_NO_PAD.encode(install.as_bytes())
            )
        });
        format!("{route}{install}")
    }
}

impl Deref for HookTarget<'_> {
    type Target = Path;

    fn deref(&self) -> &Path {
        self.helper
    }
}

// Unit tests retain the pre-route byte contracts. Production callers must
// construct a validated target, so no installer can silently omit its route.
#[cfg(test)]
impl<'a> From<&'a Path> for HookTarget<'a> {
    fn from(helper: &'a Path) -> Self {
        Self {
            helper,
            state: None,
            install: None,
        }
    }
}

#[cfg(test)]
impl<'a> From<&'a PathBuf> for HookTarget<'a> {
    fn from(helper: &'a PathBuf) -> Self {
        helper.as_path().into()
    }
}

fn parse(arguments: impl Iterator<Item = String>) -> Result<Option<PathBuf>, ()> {
    let mut arguments = arguments;
    let mut route = None;
    while let Some(argument) = arguments.next() {
        if argument != ARGUMENT {
            continue;
        }
        if route.is_some() {
            return Err(());
        }
        let encoded = arguments.next().ok_or(())?;
        if encoded.len() > ENCODED_LIMIT {
            return Err(());
        }
        let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| ())?;
        let path = PathBuf::from(String::from_utf8(bytes).map_err(|_| ())?);
        HookTarget::new(Path::new(""), &path).map_err(|_| ())?;
        route = Some(path);
    }
    Ok(route)
}

/// The installation an entry's arguments name, `Ok(None)` for an entry an
/// earlier build wrote without one.
fn parse_install(arguments: impl Iterator<Item = String>) -> Result<Option<String>, ()> {
    let mut arguments = arguments;
    let mut install = None;
    while let Some(argument) = arguments.next() {
        if argument != INSTALL_ARGUMENT {
            continue;
        }
        if install.is_some() {
            return Err(());
        }
        let encoded = arguments.next().ok_or(())?;
        if encoded.len() > ENCODED_LIMIT {
            return Err(());
        }
        let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| ())?;
        let name = String::from_utf8(bytes).map_err(|_| ())?;
        if name.is_empty() || name.len() > INSTALL_LIMIT {
            return Err(());
        }
        install = Some(name);
    }
    Ok(install)
}

/// The installation the pane this process runs in names
/// ([`INSTALL_VARIABLE`]), which a request to its daemon carries and its
/// answer must name back ([`crate::hook_request::install`]); `None` for a
/// pane that names none.
pub fn pane_install() -> Option<String> {
    std::env::var(INSTALL_VARIABLE)
        .ok()
        .filter(|install| !install.is_empty())
}

/// The folder the hooks keep their records in for the daemon whose state
/// folder is `state` (subagent counts, plan records, owed confirmations, the
/// failure record and the log): `~/.hide/agent-hooks`, where every build so
/// far kept them, while `state` is the account's own state folder; and
/// `state/agent-hooks` for a daemon whose state folder is elsewhere, so a
/// Hide started with a state folder of its own reads and writes its records
/// there rather than in the operator's `~/.hide` (docs/agent-hooks.md, Where
/// the hooks keep their records). `None` reads the account's own folder.
///
/// The choice is made by the folder, never by which variable named it: a
/// daemon `hide connect` starts is handed the account's own state folder
/// through `HIDE_STATE_DIR`, and that names no move. Hook, node and daemon
/// agree because each asks [`crate::state_folder::state_dir`] about the
/// daemon it serves, and the account's own folder is the one that daemon
/// picks with no `HIDE_STATE_DIR`.
pub fn records_root(home: &Path, state: Option<&Path>) -> PathBuf {
    let account = crate::state_folder::state_dir(
        home,
        None,
        std::env::var(XDG_VARIABLE)
            .ok()
            .filter(|value| !value.is_empty())
            .as_deref(),
    );
    match state.filter(|state| *state != account) {
        Some(state) => state.join("agent-hooks"),
        None => account_records(home),
    }
}

/// The records of a state folder of its own, beside the files it owns.
pub fn records_in(state: &Path) -> PathBuf {
    state.join("agent-hooks")
}

/// The account's own records folder, `~/.hide/agent-hooks`, which builds
/// before the records followed the state folder wrote and this one keeps
/// writing while the state folder is the account's own.
pub fn account_records(home: &Path) -> PathBuf {
    home.join(".hide").join("agent-hooks")
}

/// [`records_root`] for the daemon this process asks ([`state_dir`]): what a
/// hook writes beside its pane records, and what a node or a daemon reading
/// them resolves the same way, since each runs with the environment that
/// names its own state folder. A route that does not read leaves the
/// account's own folder; the request itself fails as `record`.
pub fn records_folder(home: &Path) -> PathBuf {
    records_root(home, state_dir(home).ok().as_deref())
}

/// The folder the hooks keep their records by pane in: the daemon's own for
/// a pane the desktop app started or one that names no installation, and one
/// of its own for a pane another installation started, whose pane ids are
/// that installation's core's and whose records the app's core neither reads
/// nor sweeps.
pub fn pane_records(home: &Path) -> PathBuf {
    pane_records_of(home, std::env::var(INSTALL_VARIABLE).ok().as_deref())
}

/// [`pane_records`] for the panes the installation `install` starts, as a
/// node that starts them reads them; `None` for panes that name none.
pub fn pane_records_of(home: &Path, install: Option<&str>) -> PathBuf {
    records_of(records_folder(home), install)
}

/// The file of `pane_id`'s record in `folder`. A pane id is in Herdr's shape
/// (`w7B:pM`, `h7B:pM`), so it is folded into a file name rather than used
/// as one.
pub fn pane_file(folder: &Path, pane_id: &str) -> PathBuf {
    let safe: String = pane_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    folder.join(format!("{safe}.json"))
}

fn records_of(records: PathBuf, pane: Option<&str>) -> PathBuf {
    match pane {
        Some(install) => match mark(install) {
            Some(mark) => records.join("installs").join(mark),
            None => records,
        },
        None => records,
    }
}

/// The short mark a file or folder of Hide's carries for an installation
/// other than the desktop app, so two installations' never share a name;
/// none for the app, or a caller that names no installation, whose names
/// stay as earlier builds wrote them.
pub fn mark(install: &str) -> Option<String> {
    if install.is_empty() || install == APP_INSTALL {
        return None;
    }
    use sha2::Digest;
    let digest = sha2::Sha256::digest(install.as_bytes());
    Some(
        digest
            .iter()
            .take(8)
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

/// Whether this process runs in a pane another Hide installation started:
/// the pane names an installation ([`INSTALL_VARIABLE`]) and the entry that
/// ran this helper names another. Such a call is that installation's own
/// entry's to answer, so this one does nothing. A pane that names none, an
/// entry an earlier build wrote without an installation, and arguments that
/// do not read are never foreign: the call goes on as before.
pub fn foreign_pane() -> bool {
    foreign(
        std::env::var(INSTALL_VARIABLE).ok().as_deref(),
        std::env::args().skip(1),
    )
}

fn foreign(pane: Option<&str>, arguments: impl Iterator<Item = String>) -> bool {
    let Some(pane) = pane.filter(|value| !value.is_empty()) else {
        return false;
    };
    matches!(parse_install(arguments), Ok(Some(entry)) if entry != pane)
}

/// Read a generated shell command's installation, for the kit's judgement of
/// whose entry it is.
pub(crate) fn install_in_command(command: &str) -> Result<Option<String>, ()> {
    parse_install(
        command
            .split_whitespace()
            .map(|word| word.trim_end_matches(';').to_owned()),
    )
}

/// The daemon state folder this hook's requests go to: the one
/// `HIDE_STATE_DIR` names, else the route the kit installed the hook with,
/// else the account's own ([`crate::state_folder::state_dir`]). An installed
/// route that does not read is `record`: an invalid installed route fails the
/// hook rather than silently contacting another daemon.
pub fn state_dir(home: &Path) -> Result<PathBuf, &'static str> {
    let read = |key: &str| std::env::var(key).ok().filter(|value| !value.is_empty());
    if let Some(state) = read(STATE_VARIABLE) {
        return Ok(PathBuf::from(state));
    }
    if let Some(route) = installed().as_ref().map_err(|_| "record")? {
        return Ok(route.clone());
    }
    Ok(crate::state_folder::state_dir(
        home,
        None,
        read("XDG_STATE_HOME").as_deref(),
    ))
}

/// The route the kit installed this hook with, read from its arguments once.
fn installed() -> &'static Result<Option<PathBuf>, ()> {
    static ROUTE: OnceLock<Result<Option<PathBuf>, ()>> = OnceLock::new();
    ROUTE.get_or_init(|| parse(std::env::args().skip(1)))
}

/// The state folder the kit installed this hook for, when it named one.
pub(crate) fn installed_state_dir() -> Option<PathBuf> {
    installed().as_ref().ok().cloned().flatten()
}

/// Read a generated shell command for install diagnosis only. Execution
/// parses argv instead; the full command must still match the generator.
pub(crate) fn in_command(command: &str) -> Result<Option<PathBuf>, ()> {
    parse(
        command
            .split_whitespace()
            .map(|word| word.trim_end_matches(';').to_owned()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> impl Iterator<Item = String> {
        values.iter().map(|value| (*value).to_owned())
    }

    #[test]
    fn rejects_a_missing_malformed_relative_or_duplicate_route() {
        for values in [
            vec![ARGUMENT],
            vec![ARGUMENT, "!"],
            vec![ARGUMENT, "cmVsYXRpdmU"],
            vec![ARGUMENT, "L29uZQ", ARGUMENT, "L3R3bw"],
        ] {
            assert_eq!(parse(arguments(&values)), Err(()));
        }
    }

    #[test]
    fn an_old_command_keeps_its_environment_route() {
        assert_eq!(parse(arguments(&["hook", "--runtime", "grok"])), Ok(None));
    }

    #[test]
    fn installation_rejects_relative_nul_and_oversized_state_folders() {
        let home = tempfile::tempdir().unwrap();
        let helper = home.path().join("helper");
        for state in [
            PathBuf::from("relative"),
            home.path().join("bad\0route"),
            home.path().join("x".repeat(ENCODED_LIMIT)),
        ] {
            assert!(HookTarget::new(&helper, &state).is_err(), "{state:?}");
        }
        assert!(HookTarget::new(&helper, home.path()).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn installation_rejects_non_utf8_instead_of_capturing_another_path() {
        use std::os::unix::ffi::OsStringExt;
        let state = PathBuf::from(std::ffi::OsString::from_vec(b"/state/\xff".to_vec()));
        assert!(HookTarget::new(Path::new("/helper"), &state).is_err());
    }

    /// An entry acts only in a pane of its own installation; a pane that
    /// names none, and an entry an earlier build wrote, act anywhere.
    #[test]
    fn an_entry_leaves_a_pane_another_installation_started() {
        let root = std::env::temp_dir();
        let helper = root.join("hide-agent-hooks");
        let state = root.join("state");
        let entry = |install: Option<&str>| {
            let target = HookTarget::new(&helper, &state).unwrap();
            let target = match install {
                Some(install) => target.of_install(install).unwrap(),
                None => target,
            };
            target
                .arguments()
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let app = entry(Some(APP_INSTALL));
        let device = entry(Some("root:/helper"));
        assert!(!foreign(Some(APP_INSTALL), app.clone().into_iter()));
        assert!(foreign(Some(APP_INSTALL), device.clone().into_iter()));
        assert!(foreign(Some("root:/helper"), app.clone().into_iter()));
        assert!(!foreign(Some("root:/helper"), device.into_iter()));
        assert!(!foreign(None, app.into_iter()));
        assert!(!foreign(Some(APP_INSTALL), entry(None).into_iter()));
    }

    #[test]
    fn a_pane_of_another_installation_keeps_its_records_apart() {
        let home = Path::new("/home");
        let own = home.join(".hide").join("agent-hooks");
        assert_eq!(records_of(own.clone(), None), own);
        assert_eq!(records_of(own.clone(), Some(APP_INSTALL)), own);
        let device = records_of(own.clone(), Some("root:/helper"));
        assert!(device.starts_with(own.join("installs")));
        assert_ne!(device, records_of(own, Some("root:/other")));
    }

    /// A daemon whose state folder is the account's own keeps the records
    /// where every earlier build kept them; one whose state folder moved
    /// keeps them in that folder, so nothing is written in the operator's
    /// `~/.hide` on its behalf.
    #[test]
    fn a_moved_state_folder_keeps_its_records_and_the_account_s_own_keeps_its_place() {
        let home = Path::new("/home");
        let account = home.join(".hide").join("state");
        let own = home.join(".hide").join("agent-hooks");
        assert_eq!(records_root(home, None), own);
        assert_eq!(records_root(home, Some(&account)), own);
        let moved = Path::new("/elsewhere/state");
        assert_eq!(records_root(home, Some(moved)), moved.join("agent-hooks"));
        assert_eq!(records_in(moved), moved.join("agent-hooks"));
    }
}
