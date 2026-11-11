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
/// The variable a pane names the Hide installation that started it by: a
/// pane holder's pane carries its node's ([`APP_INSTALL`] or [`install_of_root`]), and a
/// pane that names none (an agent outside Hide, a plain terminal) is anyone's.
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

/// The records folder of the state folder `state`, refusing one that cannot
/// be read rather than answering the account's own.
///
/// A state folder other than the account's own must be a folder this process
/// can see: one that is missing, one it may not read, and one that is not a
/// folder at all each keep their own failure rather than folding into `false`
/// and becoming `~/.hide/agent-hooks` (engineering rule 4). A hook that
/// answered the account's folder instead would write into another daemon's
/// records, where nothing reads them and the operator's file changes for no
/// reason. The account's own folder is answered without a look at the disk:
/// it holds the records whether or not the daemon has made its state folder
/// yet, and that is where every build so far wrote them.
pub fn records_folder_of(home: &Path, state: &Path) -> std::io::Result<PathBuf> {
    if state == crate::state_folder::default_state_dir(home) {
        return Ok(account_records(home));
    }
    // The kind is the kernel's and stays it: a missing folder, one this
    // process may not read and one that is not a folder are three different
    // answers, and the sentence beside the kind is what names the folder
    // `doctor` and the log show (engineering rules 4 and 10).
    //
    // The records folder below it is looked at, not only the state folder: a
    // mode that denies this process its contents still lets `metadata`
    // succeed on the state folder alone on macOS and Linux, because stat
    // needs the parent's search bit and not the folder's own read bit, so a
    // hook and a `doctor` asked that folder would answer a records path they
    // cannot read and then lose every read below it.
    //
    // What is looked at is the records folder and never a listing of the
    // state folder: a hook may run where the state folder is searchable and
    // its records folder is granted while the state folder's own contents
    // are not readable, which is how the agent journey's write sandbox hands
    // a pane's hook exactly the folder its records live in. Listing the state
    // folder refused every hook in that run, so this asks only for the one
    // entry it names. A folder nothing made yet is `NotFound` and is the
    // records path; every other refusal is kept as this process's own error.
    let metadata = std::fs::metadata(state).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "the hooks' state folder {} could not be read: {error}",
                state.display()
            ),
        )
    })?;
    if !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            format!(
                "the hooks' state folder {} is not a folder",
                state.display()
            ),
        ));
    }
    let records = records_in(state);
    match std::fs::metadata(&records) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                format!(
                    "the hooks' records folder {} is not a folder",
                    records.display()
                ),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(std::io::Error::new(
                error.kind(),
                format!(
                    "the hooks' records folder {} could not be read: {error}",
                    records.display()
                ),
            ));
        }
    }
    Ok(records)
}

/// [`records_folder_of`] for the daemon this process asks ([`state_dir`]):
/// what a hook writes beside its pane records, and what a node or a daemon
/// reading them resolves the same way, since each runs with the environment
/// that names its own state folder.
///
/// A route that does not read is an error here and `record` at the request,
/// and so is a state folder the caller cannot see; neither falls back to the
/// account's folder.
pub fn records_folder(home: &Path) -> std::io::Result<PathBuf> {
    let state = state_dir(home).map_err(|cause| {
        std::io::Error::other(format!("the hook's state route could not be read: {cause}"))
    })?;
    records_folder_of(home, &state)
}

/// The records folder of the daemon whose state folder is `state`, for the
/// installation `install`: the one place that decides where a record lives.
///
/// Every writer and every reader calls this with the state folder and the
/// installation it owns, never with this process's environment: a hook passes
/// the daemon its own route names ([`pane_records`] is that call), a node and
/// the core pass the state folder they serve and the installation whose panes
/// they start, and `doctor` passes the ones its own environment names. The
/// failure record, the log, the counts, the plans and the owed confirmations
/// are all under the folder this answers, so a helper's failure and the
/// app's never overwrite each other and a reader of one daemon never reads
/// another's (docs/agent-hooks.md, Where the hooks keep their records).
///
/// A state folder that does not read is that error here, never the account's
/// folder.
pub fn records_of_state(
    home: &Path,
    state: Option<&Path>,
    install: Option<&str>,
) -> std::io::Result<PathBuf> {
    let records = match state {
        Some(state) => records_folder_of(home, state)?,
        None => account_records(home),
    };
    Ok(pane_records_at(&records, install))
}

/// The folder the hooks of this process keep their records by pane in: what
/// [`records_of_state`] answers for the daemon this process asks
/// ([`records_folder`]) and the installation its pane names
/// ([`pane_install`]).
///
/// This is the one reader of this process's environment left in the records
/// route, and only a hook or a helper that has no caller to be handed a state
/// folder calls it: a node, the core and `doctor` name what they own instead.
pub fn pane_records(home: &Path) -> std::io::Result<PathBuf> {
    Ok(pane_records_at(
        &records_folder(home)?,
        pane_install().as_deref(),
    ))
}

/// The file of `pane_id`'s record in `folder`. A pane id has a colon in it
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

/// The records of the panes the installation `install` starts, under a
/// records folder resolved for this process ([`records_folder`]) or named by
/// a caller that already resolved one.
pub fn pane_records_at(records: &Path, install: Option<&str>) -> PathBuf {
    match install.and_then(mark) {
        Some(mark) => records.join("installs").join(mark),
        None => records.to_path_buf(),
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
        assert_eq!(pane_records_at(&own, None), own);
        assert_eq!(pane_records_at(&own, Some(APP_INSTALL)), own);
        let device = pane_records_at(&own, Some("root:/helper"));
        assert!(device.starts_with(own.join("installs")));
        assert_ne!(device, pane_records_at(&own, Some("root:/other")));
    }

    /// A daemon whose state folder is the account's own keeps the records
    /// where every earlier build kept them; one whose state folder moved
    /// keeps them in that folder, so nothing is written in the operator's
    /// `~/.hide` on its behalf.
    #[test]
    fn a_moved_state_folder_keeps_its_records_and_the_account_s_own_keeps_its_place() {
        let home = Path::new("/home");
        let default = home.join(".hide").join("state");
        let own = home.join(".hide").join("agent-hooks");
        assert_eq!(records_of_state(home, None, None).unwrap(), own);
        assert_eq!(records_of_state(home, Some(&default), None).unwrap(), own);
        assert_eq!(
            records_of_state(home, Some(&default), Some(APP_INSTALL)).unwrap(),
            own
        );
        let root = tempfile::tempdir().unwrap();
        let moved = root.path().join("state");
        std::fs::create_dir_all(&moved).unwrap();
        assert_eq!(
            records_of_state(home, Some(&moved), None).unwrap(),
            moved.join("agent-hooks")
        );
        assert_eq!(records_in(&moved), moved.join("agent-hooks"));
    }

    /// A state folder the caller can see is answered with the records folder
    /// in it, and the account's own is answered without a look at the disk:
    /// it holds the records whether or not the daemon made it yet.
    #[test]
    fn a_state_folder_this_process_can_see_keeps_its_records_and_the_account_s_own_needs_no_disk() {
        let home = tempfile::tempdir().unwrap();
        let home_path = home.path();
        let account = crate::state_folder::default_state_dir(home_path);
        assert_eq!(
            records_folder_of(home_path, &account).unwrap(),
            account_records(home_path)
        );
        let moved = home_path.join("elsewhere");
        std::fs::create_dir_all(&moved).unwrap();
        assert_eq!(
            records_folder_of(home_path, &moved).unwrap(),
            records_in(&moved)
        );
    }

    /// A state folder that is missing, one that is a file and not a folder,
    /// and one the process may not read each keep their own failure rather
    /// than folding into the account's folder: a hook that answered the
    /// account's folder instead would write into another daemon's records
    /// (engineering rule 4).
    #[cfg(unix)]
    #[test]
    fn a_state_folder_that_does_not_read_is_its_own_failure_and_never_the_accounts() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().unwrap();
        let home_path = home.path();
        let missing = home_path.join("not-there");
        let error = records_folder_of(home_path, &missing).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);

        let file = home_path.join("a-file");
        std::fs::write(&file, b"").unwrap();
        let error = records_folder_of(home_path, &file).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotADirectory);

        // A folder this process may not enter: the step that needs it fails,
        // and the failure is `PermissionDenied`, never `NotFound` (which
        // would read as a folder nothing made) and never the account's
        // folder. A run whose account may read its own locked folder cannot
        // see this failure at all, and the test says so rather than passing
        // for the wrong reason: the check that proves it for a real caller is
        // `records_route::each_way_a_state_route_fails_is_the_diagnosiss_own_error_and_never_the_accounts_folder`.
        let locked = home_path.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let guarded = records_folder_of(home_path, &locked);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        let error = guarded.expect_err(
            "a state folder this process may not read is an error, not a folder it answered",
        );
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::PermissionDenied,
            "the folder's own kind, not the account's folder and not NotFound"
        );
    }

    /// A state folder this process may search but not list still answers its
    /// records folder, which is what a hook inside the agent journey's write
    /// sandbox is handed: the profile grants reading the state folder's own
    /// metadata and reading and writing below `<state>/agent-hooks`, and
    /// listing the state folder itself is refused. Asking for a listing there
    /// refused every hook in that run, so this asks only for the one entry the
    /// route names.
    #[cfg(unix)]
    #[test]
    fn a_state_folder_that_may_be_searched_and_not_listed_still_answers_its_records() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().unwrap();
        let home_path = home.path();
        let limited = home_path.join("limited");
        let records = records_in(&limited);
        std::fs::create_dir_all(&records).unwrap();
        // Search without read: a name below it resolves, its own entries do
        // not list, the way the sandbox's rules leave the state folder.
        std::fs::set_permissions(&limited, std::fs::Permissions::from_mode(0o111)).unwrap();
        let listed = std::fs::read_dir(&limited).is_ok();
        let answer = records_folder_of(home_path, &limited);
        std::fs::set_permissions(&limited, std::fs::Permissions::from_mode(0o700)).unwrap();

        // An account that may list it anyway (root) cannot meet the refused
        // listing at all, and is told so rather than counted as proof.
        if listed {
            eprintln!(
                "not proved here: this account may list a mode 111 folder, so the search-without-\
                 listing case is unproven in this run"
            );
        }
        assert_eq!(answer.expect("the records folder is answered"), records);
    }

    /// The one rule a hook and the daemon that reads what it wrote both
    /// apply: a pane other than the app's keeps its records under the
    /// installation's mark, inside the folder its state folder names.
    #[test]
    fn a_state_folder_and_an_installation_compose_by_one_rule() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let moved = root.path().join("elsewhere").join("state");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&moved).unwrap();
        let app = records_of_state(&home, Some(&moved), None).unwrap();
        assert_eq!(app, moved.join("agent-hooks"));
        let device = records_of_state(&home, Some(&moved), Some("root:/helper")).unwrap();
        assert!(device.starts_with(moved.join("agent-hooks").join("installs")));
    }
}
