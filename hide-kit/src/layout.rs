//! Where Hide keeps its own files on a machine (PRD hide-home-layout D-01).
//!
//! Everything Hide owns sits under `~/.hide`: the daemon's state folder, a
//! device's helper root, the kit's record and copies, the hook counters and
//! retirement records. Outside it stay only what another program reads at a place
//! it chose (the hook entries and the `hide` link on `PATH`). The
//! legacy spellings are kept here too, for the one-time moves that read them
//! and the kit passes that retire them; nothing else may name them.

use std::path::{Path, PathBuf};

/// The folder under HOME that holds everything Hide owns.
pub const HIDE_HOME: &str = ".hide";

/// A device's helper root unless the daemon was started with another, as a
/// consent records it; `~` is the device account's home (D-11).
pub const HELPER_ROOT: &str = "~/.hide/host-helper";

/// The helper root builds before this layout installed under. A consent for
/// it is carried to [`HELPER_ROOT`] without asking (D-12), and the kit pass
/// retires it once the helper runs from the new root (D-13).
pub const LEGACY_HELPER_ROOT: &str = "~/.local/share/hide/host-helper";

pub fn hide_home(home: &Path) -> PathBuf {
    home.join(HIDE_HOME)
}

/// A disable-only switch, also written by cross-platform test fixtures.
/// Unlike `fixture_home`, this grants no test or service-control authority.
pub fn product_analytics_disabled(home: &Path) -> PathBuf {
    hide_home(home).join("product-analytics-disabled")
}

/// Pinned CLIs shared by local keyless journey runs, never operator executables.
pub fn journey_cli_cache(home: &Path) -> PathBuf {
    hide_home(home).join("cache/journey-clis")
}

/// Four GiB across all pins, including an in-progress installation.
/// Crossing it refuses the run; it never evicts a CLI another run may use.
pub const JOURNEY_CLI_CACHE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// The file that declares a HOME a test fixture's ([`fixture_home`]).
pub const FIXTURE_HOME_MARKER: &str = ".hide-e2e-device-home";

/// Whether `home` is a test fixture's: under `/tmp`, carrying
/// [`FIXTURE_HOME_MARKER`], and not the account's own. Nothing run for a
/// fixture HOME may reach the account's real launchd domain, which
/// `gui/<uid>` is whatever HOME says.
pub fn fixture_home(home: &Path) -> bool {
    let Ok(home) = hide_platform::fs::identity::canonical(home) else {
        return false;
    };
    let temporary = ["/tmp", "/private/tmp"]
        .iter()
        .any(|root| home.starts_with(root));
    let own = hide_platform::user_agents::account_home()
        .and_then(|own| hide_platform::fs::identity::canonical(&own))
        .is_ok_and(|own| own == home);
    temporary && !own && home.join(FIXTURE_HOME_MARKER).is_file()
}

/// The daemon's state folder when nothing relocates it (D-04).
pub fn default_state_dir(home: &Path) -> PathBuf {
    hide_home(home).join("state")
}

/// The state folder builds before this layout used by default; moved once to
/// [`default_state_dir`] by `hide connect` (D-05).
pub fn legacy_state_dir(home: &Path) -> PathBuf {
    home.join(".local/state/hide")
}

/// The environment variable that moves the state folder away from
/// [`default_state_dir`], named so the operator can be told which one to
/// change or unset.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum StateVariable {
    #[serde(rename = "HIDE_STATE_DIR")]
    HideStateDir,
    #[serde(rename = "XDG_STATE_HOME")]
    XdgStateHome,
}

/// `HIDE_STATE_DIR` wins; a set `XDG_STATE_HOME` keeps `$XDG_STATE_HOME/hide`
/// as before and is never moved; otherwise `~/.hide/state` (D-04). Empty
/// values count as unset, as the callers read them.
pub fn state_dir(
    home: &Path,
    hide_state_dir: Option<&str>,
    xdg_state_home: Option<&str>,
) -> PathBuf {
    state_dir_choice(home, hide_state_dir, xdg_state_home).0
}

/// [`state_dir`] with the variable that chose it, `None` for the default.
/// Judged by the folder, not by which variable is set: `hide connect` hands
/// the daemon it starts the folder it chose through `HIDE_STATE_DIR`, so that
/// variable naming the account's own folder chose nothing.
fn state_dir_choice(
    home: &Path,
    hide_state_dir: Option<&str>,
    xdg_state_home: Option<&str>,
) -> (PathBuf, Option<StateVariable>) {
    let (account, account_variable) = match xdg_state_home.filter(|value| !value.is_empty()) {
        Some(xdg) => (
            Path::new(xdg).join("hide"),
            Some(StateVariable::XdgStateHome),
        ),
        None => (default_state_dir(home), None),
    };
    match hide_state_dir.filter(|value| !value.is_empty()) {
        Some(dir) if Path::new(dir) != account => {
            (PathBuf::from(dir), Some(StateVariable::HideStateDir))
        }
        _ => (account, account_variable),
    }
}

/// [`state_dir`] from this process's environment, for a program that has no
/// environment registry of its own (the device helper).
pub fn state_dir_from_process(home: &Path) -> PathBuf {
    state_dir_choice_from_process(home).0
}

/// [`state_dir_from_process`] with the variable that chose it.
pub fn state_dir_choice_from_process(home: &Path) -> (PathBuf, Option<StateVariable>) {
    let read = |key: &str| std::env::var(key).ok();
    state_dir_choice(
        home,
        read("HIDE_STATE_DIR").as_deref(),
        read("XDG_STATE_HOME").as_deref(),
    )
}

/// Where a device's helper and the `hide` command beside it meet (D-06).
pub fn workspace_bridges(state_dir: &Path) -> PathBuf {
    state_dir.join("workspace-bridges")
}

/// The daemon's lifetime lock, shared with the account kit's route ownership check.
pub fn daemon_lock(state_dir: &Path) -> PathBuf {
    state_dir.join("hided.lock")
}

/// Durable local requests and watches; its owner is the core delivery worker.
pub fn delivery_ledger(state_dir: &Path) -> PathBuf {
    state_dir.join("delivery-ledger.json")
}

/// The panes the doorbell holds as an unsent draft, restored when the daemon
/// starts so a restart does not forget what the operator typed; its owner is
/// the core's composer store.
pub fn composer_marks(state_dir: &Path) -> PathBuf {
    state_dir.join("composer-marks.json")
}

/// Each agent version whose composer Hide has checked against its
/// declaration, so a version whose screen did not read empty when it was
/// known to be is never read again; its owner is the core's composer checks.
pub fn composer_checks(state_dir: &Path) -> PathBuf {
    state_dir.join("composer-checks.json")
}

/// Each Conductor's mission: its plan, nodes, reasons and report, kept
/// across a restart; its owner is the core's mission store.
pub fn missions(state_dir: &Path) -> PathBuf {
    state_dir.join("missions.json")
}

/// The last GitHub answer per project, restored when the daemon starts; its
/// owner is the core's GitHub store.
pub fn github_snapshot(state_dir: &Path) -> PathBuf {
    state_dir.join("github-snapshot.json")
}

/// The link record: which sessions made or worked on which pull request,
/// kept after a pane closes; its owner is the core's link worker.
pub fn links_store(state_dir: &Path) -> PathBuf {
    state_dir.join("links.sqlite3")
}

/// Usage events and retained daily totals, owned by the core's usage worker.
pub const USAGE_STORE: &str = "usage.sqlite3";

/// Random product statistics identity, unrelated to host or account identity.
pub const PRODUCT_ANALYTICS_ID: &str = "product-analytics-id";

pub fn product_analytics_id(state_dir: &Path) -> PathBuf {
    state_dir.join(PRODUCT_ANALYTICS_ID)
}

pub fn usage_store(state_dir: &Path) -> PathBuf {
    state_dir.join(USAGE_STORE)
}

/// Screenshots of the operator's View comments, staged by the screen's
/// node when a comment is left and read when it is sent (PRD
/// view-opener-feedback D-23); its owner is that node's
/// `hide_host::feedback`.
pub fn view_feedback_shots(state_dir: &Path) -> PathBuf {
    state_dir.join("view-feedback")
}

/// The attachments of the operator's feedback letters, one owner-only folder
/// per letter on the recipient agent's machine, removed with the letter
/// (PRD view-opener-feedback D-16); its owner is that node's
/// `hide_host::feedback`.
pub fn letter_attachments(state_dir: &Path) -> PathBuf {
    state_dir.join("letter-attachments")
}

/// The Software Factory store: Factories, Tasks, questions, records and
/// events; its owner is the core's Factory engine thread.
pub fn factory_store(state_dir: &Path) -> PathBuf {
    state_dir.join("factory.sqlite3")
}

/// The Factory's private files beside its store: PRD attachments by hash and
/// verify log tails.
pub fn factory_files(state_dir: &Path) -> PathBuf {
    state_dir.join("factory-files")
}

/// The startup files Hide gives a pane's shell so it can tell when the shell
/// has finished starting (`hide_host::shell_integration`): one folder per
/// version of those files, so a newer build never rewrites a file a starting
/// shell may be reading.
pub fn shell_integration(home: &Path) -> PathBuf {
    hide_home(home).join("shell-integration")
}

/// What a node last measured of each repository's worktrees, one file a
/// repository, so a restarted node reuses the facts of a worktree whose
/// inputs did not move; its owner is the node's worktree reader
/// (`hide_host::worktrees::Measured`).
pub fn worktree_facts(state_dir: &Path) -> PathBuf {
    state_dir.join("worktree-facts")
}

/// Where this machine's core runs when it is not here: present only on a
/// node whose core a move placed on another machine; its owner is the
/// daemon's start (`hided::placement`).
pub fn core_placement(state_dir: &Path) -> PathBuf {
    state_dir.join("core-placement.json")
}

/// The record of a core move this machine drives, written before each step
/// that cannot be undone; its owner is the move driver
/// (`hided::core_move::journal`).
pub fn core_move_journal(state_dir: &Path) -> PathBuf {
    state_dir.join("core-move.json")
}

/// The forward moves this machine gave up while their other machine had
/// not said whether they committed, each kept until that machine has no
/// core of it; its owner is the daemon's supervisor
/// (`hided::core_move::abandoned`).
pub fn core_move_abandoned(state_dir: &Path) -> PathBuf {
    state_dir.join("core-move-abandoned.json")
}

/// The record on a machine taking or giving the core in a move: a core
/// waiting for its first link, active, stopped for the move, or retired;
/// its owner is `hided core-move` (`hided::core_move::handover`).
pub fn core_handover(state_dir: &Path) -> PathBuf {
    state_dir.join("core-handover.json")
}

/// The login item that runs the core on `state_dir` after a move placed it
/// on this machine (`hided::login_item`): one per state folder, so a folder
/// other than the account's default never takes the default's item.
pub fn core_login_item(home: &Path, state_dir: &Path) -> String {
    const LABEL: &str = "dev.withhide.core";
    if state_dir == default_state_dir(home) {
        return LABEL.to_owned();
    }
    use sha2::Digest;
    let digest = sha2::Sha256::digest(state_dir.to_string_lossy().as_bytes());
    format!("{LABEL}.{}", hex_prefix(&digest))
}

fn hex_prefix(digest: &[u8]) -> String {
    digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Copies a move makes of this machine's brain state, one folder per move.
pub fn move_staging(state_dir: &Path) -> PathBuf {
    state_dir.join("move-staging")
}

/// Where the machine taking the core receives a move's copy.
pub fn move_incoming(state_dir: &Path) -> PathBuf {
    state_dir.join("move-incoming")
}

/// The brain state a machine held before its core moved away, the latest
/// move's only.
pub fn moved_out(state_dir: &Path) -> PathBuf {
    state_dir.join("moved-out")
}

/// `~/rest` of a helper root spelling under `home`; an absolute spelling as
/// it is.
pub fn expand_home(spelling: &str, home: &Path) -> PathBuf {
    match spelling.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(spelling),
    }
}

pub fn helper_root(home: &Path) -> PathBuf {
    expand_home(HELPER_ROOT, home)
}

pub fn legacy_helper_root(home: &Path) -> PathBuf {
    expand_home(LEGACY_HELPER_ROOT, home)
}

/// The folder each Windows package build's runtime copy sits in (PRD
/// auto-update D-10): outside the install folder, whose processes the
/// installer stops and whose running files Windows cannot replace.
pub const RUNTIME_ROOT: &str = "runtime";

pub fn runtime_root(home: &Path) -> PathBuf {
    hide_home(home).join(RUNTIME_ROOT)
}

/// One build's runtime copy; `build` is `<version>-<hash prefix>`
/// (`crate::runtime::build_name`).
pub fn runtime_dir(home: &Path, build: &str) -> PathBuf {
    runtime_root(home).join(build)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A daemon `hide connect` started always has `HIDE_STATE_DIR`, naming
    /// the folder that connect chose; only a folder other than the account's
    /// own was chosen by it.
    #[test]
    fn a_variable_naming_the_account_s_own_folder_chose_nothing() {
        let home = Path::new("/Users/example");
        let choice = |hide: Option<&str>, xdg: Option<&str>| state_dir_choice(home, hide, xdg);
        assert_eq!(
            choice(Some("/Users/example/.hide/state"), None),
            (PathBuf::from("/Users/example/.hide/state"), None)
        );
        assert_eq!(
            choice(Some("/xdg/hide"), Some("/xdg")),
            (
                PathBuf::from("/xdg/hide"),
                Some(StateVariable::XdgStateHome)
            )
        );
        assert_eq!(
            choice(Some("/Volumes/Team/hide"), None),
            (
                PathBuf::from("/Volumes/Team/hide"),
                Some(StateVariable::HideStateDir)
            )
        );
        assert_eq!(
            choice(Some("/Users/example/.hide/state"), Some("/xdg")),
            (
                PathBuf::from("/Users/example/.hide/state"),
                Some(StateVariable::HideStateDir)
            )
        );
    }

    #[test]
    fn the_state_folder_is_under_hide_unless_something_relocates_it() {
        let home = Path::new("/Users/example");
        assert_eq!(
            state_dir(home, None, None),
            PathBuf::from("/Users/example/.hide/state")
        );
        assert_eq!(
            state_dir(home, Some(""), Some("")),
            state_dir(home, None, None)
        );
        assert_eq!(
            state_dir(home, None, Some("/xdg")),
            PathBuf::from("/xdg/hide")
        );
        assert_eq!(
            state_dir(home, Some("/isolated"), Some("/xdg")),
            PathBuf::from("/isolated")
        );
    }

    #[test]
    fn a_state_folder_other_than_the_default_has_a_login_item_of_its_own() {
        let home = Path::new("/Users/alice");
        let default = core_login_item(home, &default_state_dir(home));
        let other = core_login_item(home, Path::new("/private/tmp/seat/state"));
        assert_eq!(default, "dev.withhide.core");
        assert!(
            other.starts_with("dev.withhide.core.") && other != default,
            "{other}"
        );
        assert_eq!(
            other,
            core_login_item(home, Path::new("/private/tmp/seat/state"))
        );
    }

    #[test]
    fn every_hide_default_sits_under_one_folder() {
        let home = Path::new("/home/me");
        for path in [
            default_state_dir(home),
            helper_root(home),
            crate::kit_state_dir(home),
        ] {
            assert!(path.starts_with("/home/me/.hide"), "{}", path.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn only_a_marked_home_under_tmp_is_a_fixture_home() {
        let marked = tempfile::tempdir_in("/tmp").unwrap();
        std::fs::write(marked.path().join(FIXTURE_HOME_MARKER), "").unwrap();
        let unmarked = tempfile::tempdir_in("/tmp").unwrap();
        assert!(fixture_home(marked.path()));
        assert!(!fixture_home(unmarked.path()));
    }

    /// A fixture HOME's kit has no login agents, so nothing it unloads or
    /// asks reaches the account's real launchd domain.
    #[cfg(unix)]
    #[test]
    fn a_fixture_home_s_kit_never_reaches_launchd() {
        use std::sync::atomic::AtomicBool;
        let marked = tempfile::tempdir_in("/tmp").unwrap();
        std::fs::write(marked.path().join(FIXTURE_HOME_MARKER), "").unwrap();
        let agents = crate::login_agents(marked.path());
        let stop = AtomicBool::new(false);
        assert!(
            !agents
                .is_loaded("dev.withhide.core", marked.path(), &stop)
                .unwrap()
        );
        assert_eq!(
            agents.login(marked.path(), &stop).unwrap_err().kind(),
            std::io::ErrorKind::Unsupported
        );
        agents
            .unload("dev.withhide.core", marked.path(), &stop)
            .unwrap();
    }
}
