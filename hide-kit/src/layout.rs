//! Where Hide keeps its own files on a machine (PRD hide-home-layout D-01).
//!
//! Everything Hide owns sits under `~/.hide`: the daemon's state folder, a
//! device's helper root, the kit's record and copies, the hook counters and
//! retirement records. Outside it stay only what another program reads at a place
//! it chose (the hook entries and the `hide` link on `PATH`). The
//! legacy spellings are kept here too, for the one-time moves that read them
//! and the kit passes that retire them; nothing else may name them.

use std::path::{Path, PathBuf};

/// A device's helper root unless the daemon was started with another, as a
/// consent records it; `~` is the device account's home (D-11).
pub const HELPER_ROOT: &str = "~/.hide/host-helper";

/// The helper root builds before this layout installed under. A consent for
/// it is carried to [`HELPER_ROOT`] without asking (D-12), and the kit pass
/// retires it once the helper runs from the new root (D-13).
pub const LEGACY_HELPER_ROOT: &str = "~/.local/share/hide/host-helper";

/// A disable-only switch, also written by cross-platform test fixtures.
/// Unlike `fixture_home`, this grants no test or service-control authority.
pub fn product_analytics_disabled(home: &Path) -> PathBuf {
    hide_home(home).join("product-analytics-disabled")
}

/// What a device's helper writes before it sets the old `~/hide` folder aside
/// (PRD retire-hide-home): the name it is set aside under and its identity,
/// the only claim on a folder of that name. Removed when the retirement ends.
pub fn home_retirement(home: &Path) -> PathBuf {
    hide_home(home).join("home-retiring.json")
}

/// Where the helper writes the old Home's marker whole before it renames it
/// back into a folder that lost it; only the helper writes here.
pub fn home_retirement_marker(home: &Path) -> PathBuf {
    hide_home(home).join("home-retiring-marker.json")
}

/// Pinned CLIs shared by local keyless journey runs, never operator executables.
pub fn journey_cli_cache(home: &Path) -> PathBuf {
    hide_home(home).join("cache/journey-clis")
}

/// Release-matched foreign device kits. Two platform slots, one staging
/// folder, and one download at a time; see `hide_node::ssh::host`.
pub fn device_bundle_cache(home: &Path) -> PathBuf {
    hide_home(home).join("cache/device-bundles")
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

/// The state folder builds before this layout used by default; moved once to
/// [`default_state_dir`] by `hide connect` (D-05).
pub fn legacy_state_dir(home: &Path) -> PathBuf {
    home.join(".local/state/hide")
}

// The state folder's rule is named under the kit, in the hook crate, so a
// hook finds its daemon by the same rule `hide` does.
pub use hide_agent_hooks::state_folder::{
    HIDE_HOME, StateVariable, default_state_dir, hide_home, state_dir, state_dir_choice,
};

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

/// Where a device's helper and the `hide` command beside it meet (D-06);
/// named beside the hook request that looks there, which the kit's crates
/// sit above.
pub use hide_agent_hooks::hook_request::workspace_bridges;

/// The letter confirmations a pane's hook could not send in time, kept for
/// the pane's node to hand the core, one record per pane, at most 64 and
/// none past a letter's deadline; named beside the hooks that keep them,
/// which the kit's crates sit above (`docs/delivery.md`, A confirmation that
/// missed its hook). `owed_confirmations` is the folder a hook writes, for
/// its own pane, and `owed_confirmations_in` the one a node reads, under the
/// records folder it already resolved from the state folder and installation
/// it serves, so the route decides that folder once and this only names what
/// sits below it.
pub use hide_agent_hooks::owed::{
    folder as owed_confirmations, folder_in as owed_confirmations_in,
};

/// The daemon's lifetime lock, shared with the account kit's route ownership check.
pub fn daemon_lock(state_dir: &Path) -> PathBuf {
    state_dir.join("hided.lock")
}

/// When the daemon last asked for its own restart because a pane's holder
/// died without saying why (`hided::holder_restart`): its modified time is
/// the only thing read, so a daemon never asks twice in a short while.
pub fn holder_restart(state_dir: &Path) -> PathBuf {
    state_dir.join("holder-restart")
}

/// Held while a restart request is checked and recorded, so concurrent
/// losses ask once (`hided::holder_restart`).
pub fn holder_restart_lock(state_dir: &Path) -> PathBuf {
    state_dir.join("holder-restart.lock")
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

/// Every Conductor's missions in one book, as builds before the mission
/// folder kept them: read once to move them into [`missions_dir`], never
/// written again, and left for a downgrade; its owner is the core's mission
/// store.
pub fn missions(state_dir: &Path) -> PathBuf {
    state_dir.join("missions.json")
}

/// The folder of each Conductor's mission, one file per mission (its plan,
/// nodes, reasons and report) beside [`mission_index`]; its owner is the
/// core's mission store.
pub const MISSIONS_DIR: &str = "missions";

pub fn missions_dir(state_dir: &Path) -> PathBuf {
    state_dir.join(MISSIONS_DIR)
}

/// The mission folder's index: the next mission id and one line per
/// mission, written last when the folder is first filled, so a folder
/// without it is a move that did not finish.
pub fn mission_index(missions_dir: &Path) -> PathBuf {
    missions_dir.join(MISSION_INDEX)
}

pub const MISSION_INDEX: &str = "index.json";

/// One mission's file in the mission folder. A mission id is a key
/// (`hide_core::missions::valid_key`), so it names one file there.
pub fn mission_file(missions_dir: &Path, id: &str) -> PathBuf {
    missions_dir.join(format!("{id}.json"))
}

/// The schema 1 mission book as it was before its first conversion to
/// node criteria, kept once so a downgrade can be restored by hand; its
/// owner is the core's mission store.
pub fn missions_before_criteria(state_dir: &Path) -> PathBuf {
    state_dir.join("missions.v1.json")
}

/// The schema 2 mission book as it was before nodes took three states and a
/// pull request list, kept once so a downgrade can be restored by hand; its
/// owner is the core's mission store.
pub fn missions_before_simple(state_dir: &Path) -> PathBuf {
    state_dir.join("missions.v2.json")
}

/// The schema 3 mission book as it was before a node could carry a human
/// gate, kept once so a downgrade can be restored by hand; its owner is the
/// core's mission store.
pub fn missions_before_gates(state_dir: &Path) -> PathBuf {
    state_dir.join("missions.v3.json")
}

/// The legacy book's histories, retained unchanged for a downgrade after
/// migration. Only the first migration reads these files.
pub const MISSION_EVENTS: &str = "mission-events";

pub fn mission_events(state_dir: &Path) -> PathBuf {
    state_dir.join(MISSION_EVENTS)
}

/// Histories owned by the folder book, isolated from a downgraded build's
/// sweep and reused mission ids; the mission event store owns them.
pub fn mission_histories(state_dir: &Path) -> PathBuf {
    missions_dir(state_dir).join(MISSION_HISTORIES)
}

pub const MISSION_HISTORIES: &str = "events";

pub fn mission_history(missions_dir: &Path, id: &str) -> PathBuf {
    missions_dir
        .join(MISSION_HISTORIES)
        .join(format!("{id}.json"))
}

/// The last GitHub answer per project, restored when the daemon starts; its
/// owner is the core's GitHub store.
pub fn github_snapshot(state_dir: &Path) -> PathBuf {
    state_dir.join("github-snapshot.json")
}

/// What each pane's agent hook last reported about its session (runtime,
/// hook version, subagent counts), restored when the daemon starts; its
/// owner is the core's hook facts (`hide_core::agent_hooks`).
pub fn hook_facts(state_dir: &Path) -> PathBuf {
    state_dir.join("hook-facts.json")
}

/// The link record: which sessions made or worked on which pull request,
/// kept after a pane closes; its owner is the core's link worker.
pub fn links_store(state_dir: &Path) -> PathBuf {
    state_dir.join("links.sqlite3")
}

/// Hide AI's suggestion records, notebooks and their lines' stories, per
/// Project; its owner is the core's suggestion worker.
pub fn suggestions_store(state_dir: &Path) -> PathBuf {
    state_dir.join(SUGGESTIONS_STORE)
}

pub const SUGGESTIONS_STORE: &str = "suggestions.sqlite3";

/// Desk's own folder (PRD Desk 1차): the store of its conversation and
/// settings, and each Desk process's working folder, launcher and markers.
pub fn desk_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("desk")
}

/// Desk's conversation, choices and settings; its owner is the core's Desk.
pub fn desk_store(state_dir: &Path) -> PathBuf {
    desk_dir(state_dir).join("desk.json")
}

/// Desk's own `CODEX_HOME`: the configuration and rules Desk writes for its
/// Codex, Codex's own records of Desk's sessions, and `auth.json`, a link to
/// the operator's own Codex sign-in. Desk's Codex reads nothing else of the
/// operator's `~/.codex`.
pub fn desk_codex_home(state_dir: &Path) -> PathBuf {
    desk_dir(state_dir).join("codex")
}

/// The ACP adapters Desk installs from their pinned locks, one folder per
/// adapter version (`<id>-<version>`). Account-wide like the helper root: an
/// installed version is the same for every state folder.
pub fn desk_adapters(home: &Path) -> PathBuf {
    hide_home(home).join("desk-adapters")
}

/// The operator's own agent definition files (`<id>.toml`), read after the
/// built-in ones (PRD engine-bundle-3-core-owns D-17,
/// `hide_agent_adapter::definition`). Account-wide: one machine's agents are
/// the same for every state folder.
pub fn agent_definitions(home: &Path) -> PathBuf {
    hide_home(home).join("agents")
}

/// The Hide global instruction every session started in a Hide pane receives
/// at its start, in every Project (PRD memory-self-improve D-23, D-29): Hide's
/// own file, which the operator may edit and the core writes for a fix the
/// operator applied; never one of an agent's own global files.
pub fn global_instruction(state_dir: &Path) -> PathBuf {
    state_dir.join(GLOBAL_INSTRUCTION)
}

/// Its name, the global instruction guard's (`hide_agent_hooks::global_guard`).
pub const GLOBAL_INSTRUCTION: &str = hide_agent_hooks::global_guard::FILE_NAME;

/// The machine's agent profiles, owned by the node that runs on it (PRD
/// agent-profiles D-04; `hide_host::profiles`).
pub fn agent_profiles(state_dir: &Path) -> PathBuf {
    state_dir.join("agent-profiles.json")
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

/// Files the operator attached to a terminal of this machine's panes from a
/// window on another machine, one owner-only file each, kept 24 hours (PRD
/// wsl-device B10); its owner is that node's `hide_host::attachments`.
pub fn terminal_attachments(state_dir: &Path) -> PathBuf {
    state_dir.join("terminal-attachments")
}

/// Where a device's terminal attachments were staged over SSH before its
/// link carried them. Its node only removes the files there that have
/// outlived their 24 hours, and the folder once it is empty.
pub fn legacy_terminal_attachments(home: &Path) -> PathBuf {
    home.join(".hide-terminal-attachments")
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

/// The folder each pane holder build's version folder sits in (PRD
/// engine-bundle-2-pane-holder D-16): outside the app, so an update can
/// replace the app while holders run, and apart from [`RUNTIME_ROOT`], so
/// neither folder's cleanup ever reaches the other's.
pub const HOLDERS_ROOT: &str = "holders";

pub fn holders_root(home: &Path) -> PathBuf {
    hide_home(home).join(HOLDERS_ROOT)
}

/// One holder build's version folder; `name` is `v<version>-<content hash>`
/// (`crate::holders::version_name`).
pub fn holder_dir(home: &Path, name: &str) -> PathBuf {
    holders_root(home).join(name)
}

/// The folder of the core's own engine in a state folder (PRD
/// engine-bundle-3-core-owns D-6): what that engine's session is keyed by,
/// as a Herdr session was by its socket.
pub fn engine_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("engine")
}

/// The saved session of the core's own engine (PRD engine-bundle-3-core-owns
/// D-6): its workspaces, tabs, pane trees and panes, read once when the
/// daemon starts and replaced whole after every change.
pub fn engine_session(state_dir: &Path) -> PathBuf {
    engine_session_in(&engine_dir(state_dir))
}

/// Where a saved session the engine could not read is kept, the newest
/// three, rather than deleted.
pub fn engine_backups(state_dir: &Path) -> PathBuf {
    engine_backups_in(&engine_dir(state_dir))
}

/// The saved session of the engine whose folder is `engine_dir`, this
/// machine's or a device's.
pub fn engine_session_in(engine_dir: &Path) -> PathBuf {
    engine_dir.join("session.json")
}

/// Where the engine whose folder is `engine_dir` keeps a saved session it
/// could not read.
pub fn engine_backups_in(engine_dir: &Path) -> PathBuf {
    engine_dir.join("backups")
}

/// The import record of the engine whose folder is `engine_dir`, this
/// machine's or a device's (PRD engine-bundle-4-herdr-removal D-25): what of
/// that machine's Herdr session is not moved into the engine yet, what was,
/// the conversations waiting to resume and whether the move was committed.
pub fn herdr_import_in(engine_dir: &Path) -> PathBuf {
    engine_dir.join("herdr-import.json")
}

/// A device's Herdr save as its node read it when a move of it was asked,
/// kept in the device's engine folder `engine_dir` until the core's next
/// start moves it (PRD engine-bundle-4-herdr-removal D-10).
pub fn herdr_save_kept_in(engine_dir: &Path) -> PathBuf {
    engine_dir.join("herdr-save.json")
}

/// The folder of the core's own engine for the device `device`, in the
/// core's state folder (PRD engine-bundle-3-core-owns D-22): the core keeps
/// each machine's session, a device's as its own, and the device keeps only
/// its panes' holders. Refused for an id that is not one folder name.
pub fn device_engine_dir(state_dir: &Path, device: &str) -> Result<PathBuf, String> {
    let plain = !device.is_empty()
        && device.len() <= 128
        && !device.starts_with('.')
        && device
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if !plain {
        return Err(format!(
            "the device id {device:?} is not one folder name, so its engine has no folder"
        ));
    }
    Ok(engine_dir(state_dir).join("devices").join(device))
}

/// Where the core's own engine makes a repository's worktree named `name`
/// (PRD engine-bundle-3-core-owns D-28): `~/.hide/worktrees/<repo>/<name>`
/// under `home`, the wire spelling of the home of the machine the worktree
/// is on, which may be another machine's. A worktree another engine made
/// elsewhere (Herdr's `~/.herdr/worktrees`) stays where it is and is opened
/// by its path.
pub fn engine_worktree(
    home: &str,
    repo: &str,
    name: &str,
) -> Result<String, hide_platform::path::PathError> {
    let relative = hide_platform::path::RelPath::parse(&format!(".hide/worktrees/{repo}/{name}"))?;
    Ok(hide_platform::path::wire_join(home, &relative))
}

/// The state folder of a device's node, where the pane holders it runs for
/// a core on another machine keep their records, sockets and logs (PRD
/// engine-bundle-3-core-owns D-22): apart from the account's own Hide state
/// folder, so a Hide running on the device itself and the node its core
/// started never reconcile each other's panes.
pub fn device_node_state(home: &Path) -> PathBuf {
    hide_home(home).join("device-node")
}

/// The folder a state folder's holders listen and log in, one socket and
/// one log per pane.
pub fn holder_panes(state_dir: &Path) -> PathBuf {
    state_dir.join("holders")
}

pub fn holder_socket(state_dir: &Path, pane: &str) -> PathBuf {
    holder_panes(state_dir).join(format!("{pane}.sock"))
}

pub fn holder_log(state_dir: &Path, pane: &str) -> PathBuf {
    holder_panes(state_dir).join(format!("{pane}.log"))
}

/// The folder of a state folder's pane records (`crate::panes`), the
/// holders' owner (PRD engine-bundle-2-pane-holder D-14).
pub fn pane_records(state_dir: &Path) -> PathBuf {
    holder_panes(state_dir).join("records")
}

pub fn pane_record(state_dir: &Path, pane: &str) -> PathBuf {
    pane_records(state_dir).join(format!("{pane}.json"))
}

/// The one lock every change of a pane record is made under.
pub fn pane_records_lock(state_dir: &Path) -> PathBuf {
    pane_records(state_dir).join(".lock")
}

/// Connection ownership of node record mutations. Never remove or replace
/// this file: all processes must lock the same inode across reconnects.
pub fn pane_owner_lock(state_dir: &Path) -> PathBuf {
    pane_records(state_dir).join(".owner-lock")
}

/// The next incarnation no process has taken from the folder yet, which no
/// earlier start of the folder had.
pub fn pane_incarnations(state_dir: &Path) -> PathBuf {
    pane_records(state_dir).join("next")
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
            holders_root(home),
        ] {
            assert!(path.starts_with("/home/me/.hide"), "{}", path.display());
        }
        // Neither the runtime copies nor the holder folders are inside the
        // other, so neither's cleanup can reach the other's folders.
        assert!(!holders_root(home).starts_with(runtime_root(home)));
        assert!(!runtime_root(home).starts_with(holders_root(home)));
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
