//! The device's install kit, run by the helper for the core (PRD
//! device-parity D-09, D-10, D-16).
//!
//! The core uploads every part of a build into `<root>/<version>/`, the
//! folder this helper runs from. Before it installs, the helper points
//! `<root>/current` at that folder, so the hooks, the `hide` link and the
//! plugin name a path that outlives the build (B16), and removes the builds
//! the link no longer leads to. The root is always the one the running helper
//! was installed under; no request can name another folder. A helper under
//! the default root also retires the old layout's root and bridge folder
//! once nothing names them (`hide_kit::legacy::device`), whose paths are
//! constants of the helper, never a request's.
//!
//! A build that carries Herdr for the device names its copy in `herdr.pin`
//! (`hide_kit::device_herdr`): before the kit applies, the helper points
//! `<root>/herdr/current` at it, and after the kit has applied, the node's
//! Herdr updater (installed by `hided node serve`) looks at the server on the
//! device's registered socket and hands it to that copy when the rules allow
//! (PRD device-herdr-update D-07). A copy is removed only once no process on
//! the device runs it (D-09).
//!
//! A core or a node that runs a build Hide placed under a helper root (a
//! moved core's login item runs one) keeps its kit from that root too, but
//! leaves the builds where the move and the core update put them
//! (`KitPlace::Build`).
//!
//! When the link closes, the serving loop raises its [`Env`]'s stop flag,
//! which ends a child the kit is waiting on, so the helper does not outlive
//! its connection by more than one bounded step (engineering rule 14).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::Value;

use crate::error::{ErrorCode, HostError, HostResult};
use crate::protocol::{KitAction, KitRemoved};
use crate::serve::{Env, KitPlace};

use hide_kit::device_herdr::{self, HerdrUpdate};
pub use hide_kit::is_build_name;

/// What a device's node does about the Herdr server on its registered socket
/// once its kit has applied: given the pinned copy, the socket and the file
/// recording failed handoffs, it answers what it found and did.
pub type HerdrUpdater = fn(pinned: &Path, socket: &Path, record: &Path) -> HerdrUpdate;

struct HerdrPolicy {
    preserve_server: bool,
    updater: Option<HerdrUpdater>,
}

static HERDR_UPDATER: std::sync::OnceLock<HerdrUpdater> = std::sync::OnceLock::new();

/// Installs the node's Herdr updater; the helper's process does this once
/// before it serves. A process that installs none (the core's own node, a
/// test) hands nothing over.
pub fn set_herdr_updater(updater: HerdrUpdater) {
    let _ = HERDR_UPDATER.set(updater);
}

/// Answers one `kit` request for the node `env` describes: the helper root
/// the running helper was installed under, the build a core or node runs
/// from, or the desktop package it runs from.
pub fn handle(
    action: KitAction,
    cli_dir: &str,
    herdr_socket: Option<&str>,
    retirement_projects: &[String],
    preserve_herdr_server: bool,
    env: &Env,
) -> HostResult<Value> {
    if preserve_herdr_server && herdr_socket.is_none() {
        return Err(HostError::new(
            ErrorCode::Unsupported,
            "The saved Herdr session has no resolved socket; reconnect that profile first",
        ));
    }
    match &env.kit {
        KitPlace::Installed => {}
        KitPlace::Build { root, version } => {
            let home = env.home.clone().ok_or_else(|| {
                HostError::new(
                    ErrorCode::Unsupported,
                    "HOME is not set, so Hide cannot tell where to install",
                )
            })?;
            return run_build(
                &Placement {
                    root: root.clone(),
                    version: version.clone(),
                    home,
                },
                action,
                cli_dir,
                herdr_socket,
                retirement_projects,
                Arc::clone(&env.stop),
            );
        }
        KitPlace::Bundled(kit_dir) => {
            return bundled(
                kit_dir,
                env,
                action,
                cli_dir,
                herdr_socket,
                retirement_projects,
            );
        }
        KitPlace::Standalone => {
            return Err(HostError::new(
                ErrorCode::Unsupported,
                hide_kit::STANDALONE_REASON,
            ));
        }
    }
    let home = env.home.as_deref();
    let executable = std::env::current_exe().map_err(|error| {
        HostError::new(
            ErrorCode::Io,
            format!("The helper could not tell where it runs from: {error}"),
        )
    })?;
    let (root, version) = install_layout(&executable)?;
    let home = home.map(Path::to_path_buf).ok_or_else(|| {
        HostError::new(
            ErrorCode::Unsupported,
            "The home folder is unknown, so the helper cannot tell where to install",
        )
    })?;
    run_for_projects(
        &Placement {
            root,
            version,
            home,
        },
        action,
        cli_dir,
        herdr_socket,
        retirement_projects,
        Arc::clone(&env.stop),
        HerdrPolicy {
            preserve_server: preserve_herdr_server,
            updater: HERDR_UPDATER.get().copied(),
        },
    )
}

/// The kit of the machine whose desktop package holds the parts: they stay
/// where the package put them, so nothing is pointed or pruned, and the
/// machine itself is never removed from Hide.
fn bundled(
    kit_dir: &Path,
    env: &Env,
    action: KitAction,
    cli_dir: &str,
    herdr_socket: Option<&str>,
    retirement_projects: &[String],
) -> HostResult<Value> {
    let home = env.home.clone().ok_or_else(|| {
        HostError::new(
            ErrorCode::Unsupported,
            "HOME is not set, so Hide cannot tell where to install",
        )
    })?;
    let inputs = Inputs::read(&home, cli_dir, herdr_socket, retirement_projects)?;
    let mut target =
        hide_kit::local_target(kit_dir, &home, &inputs.herdr_socket, Arc::clone(&env.stop));
    target.cli_dir = inputs.cli_dir;
    target.retirement_projects = inputs.projects;
    let mut report = match action {
        KitAction::Apply | KitAction::Reinstall { .. } => {
            hide_kit::apply(&target, &scope_of(action))
        }
        KitAction::Status => hide_kit::status(&target),
        KitAction::Remove => return Err(not_removed()),
    };
    if let Some(refusal) = report.refused.as_mut() {
        refusal.name_stop(kit_dir, &hide_kit::layout::state_dir_from_process(&home));
    }
    to_value(report)
}

/// A machine that runs Hide itself, as the core or as a node that dials
/// it, keeps its kit when its row goes: it registers again on its next
/// link.
fn not_removed() -> HostError {
    HostError::new(
        ErrorCode::Unsupported,
        "A machine that runs Hide itself is not removed from Hide",
    )
}

/// The kit of a core or node that runs a build Hide placed: it installs
/// from the root's `current`, which it points at this build first so the
/// hooks and the `hide` link name the build that runs, and it removes no
/// build and points no Herdr copy, since the move and the core update own
/// both (`adopt_build`). Public for the core's tests, which place a build
/// folder without a device.
pub fn run_build(
    placement: &Placement,
    action: KitAction,
    cli_dir: &str,
    herdr_socket: Option<&str>,
    retirement_projects: &[String],
    stop: Arc<AtomicBool>,
) -> HostResult<Value> {
    let home = &placement.home;
    let Inputs {
        projects,
        cli_dir,
        herdr_socket,
    } = Inputs::read(home, cli_dir, herdr_socket, retirement_projects)?;
    let mut target = hide_kit::device_target(&placement.root, home, &cli_dir, &herdr_socket, stop);
    target.retirement_projects = projects;
    match action {
        KitAction::Apply | KitAction::Reinstall { .. } => {
            if let Err(refusal) = hide_kit::retirement_preflight(&target) {
                return to_value(hide_kit::KitReport::refused(refusal));
            }
            point_current(&placement.root, &placement.version)?;
            to_value(hide_kit::apply(&target, &scope_of(action)))
        }
        KitAction::Status => to_value(hide_kit::status(&target)),
        KitAction::Remove => Err(not_removed()),
    }
}

/// Where the running helper was installed.
#[derive(Debug)]
pub struct Placement {
    pub root: PathBuf,
    pub version: String,
    pub home: PathBuf,
}

/// The helper root and build folder of a helper at
/// `<root>/<16 hex>/hided`; any other copy, such as one a
/// developer started by hand, runs no kit.
fn install_layout(executable: &Path) -> HostResult<(PathBuf, String)> {
    build_layout(executable).ok_or_else(|| {
        HostError::new(
            ErrorCode::Unsupported,
            format!(
                "{} is not a helper Hide installed, so it does not install the kit",
                executable.display()
            ),
        )
    })
}

/// The programs a build Hide places holds beside its `hided`, which the kit
/// links and names in the hooks it writes.
const BUILD_PROGRAMS: [&str; 2] = ["hide", "hide-agent-hooks"];

/// The root and build of a program Hide placed for the account at `home`:
/// in that account's own helper root (`~/.hide/host-helper`), in a build
/// folder holding the kit's other programs beside it. A copy anywhere else,
/// or a lone `hided` there, is no build Hide placed, so it installs nothing
/// into the account: a development build copied into a folder of that
/// name, or a fixture's build while HOME is the operator's.
pub(crate) fn placed_build(executable: &Path, home: &Path) -> Option<(PathBuf, String)> {
    let (root, version) = build_layout(executable)?;
    let own = hide_kit::layout::helper_root(home);
    let canonical = hide_platform::fs::identity::canonical;
    let in_own_root = root == own
        || matches!((canonical(&root), canonical(&own)), (Ok(root), Ok(own)) if root == own);
    let whole = BUILD_PROGRAMS.iter().all(|program| {
        root.join(&version)
            .join(format!("{program}{}", std::env::consts::EXE_SUFFIX))
            .is_file()
    });
    (in_own_root && whole && executable.is_file()).then_some((root, version))
}

/// The root and build of a program at `<root>/<16 hex>/<program>`, the
/// folder every install of a build into a helper root makes.
pub(crate) fn build_layout(executable: &Path) -> Option<(PathBuf, String)> {
    let version_dir = executable.parent()?;
    let version = version_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| is_build_name(name))?;
    let root = version_dir.parent()?;
    Some((root.to_path_buf(), version.to_owned()))
}

/// Runs `action` for a helper placed at `placement`. Public for the core's
/// tests, which place a helper folder without a device.
pub fn run(
    placement: &Placement,
    action: KitAction,
    cli_dir: &str,
    herdr_socket: Option<&str>,
    stop: Arc<AtomicBool>,
) -> HostResult<Value> {
    run_for_projects(
        placement,
        action,
        cli_dir,
        herdr_socket,
        &[],
        stop,
        HerdrPolicy {
            preserve_server: false,
            updater: HERDR_UPDATER.get().copied(),
        },
    )
}

fn run_for_projects(
    placement: &Placement,
    action: KitAction,
    cli_dir: &str,
    herdr_socket: Option<&str>,
    retirement_projects: &[String],
    stop: Arc<AtomicBool>,
    herdr_policy: HerdrPolicy,
) -> HostResult<Value> {
    let home = &placement.home;
    let Inputs {
        projects,
        cli_dir,
        herdr_socket,
    } = Inputs::read(home, cli_dir, herdr_socket, retirement_projects)?;
    let target = || {
        let mut target = hide_kit::device_target(
            &placement.root,
            home,
            &cli_dir,
            &herdr_socket,
            Arc::clone(&stop),
        );
        target.retirement_projects = projects.clone();
        target
    };
    match action {
        KitAction::Apply | KitAction::Reinstall { .. } => {
            if let Err(refusal) = hide_kit::retirement_preflight(&target()) {
                return to_value(hide_kit::KitReport::refused(refusal));
            }
            // Resolve the required copy before retiring the last usable build.
            let pinned = point_herdr(&placement.root, &placement.version)?;
            point_current(&placement.root, &placement.version)?;
            remove_other_builds(&placement.root, &placement.version);
            let mut report = hide_kit::apply(&target(), &scope_of(action));
            if let (Some(pinned), Some(update)) = (
                pinned.as_deref(),
                herdr_policy
                    .updater
                    .filter(|_| !herdr_policy.preserve_server),
            ) {
                let record =
                    device_herdr::herdr_dir(&placement.root).join(device_herdr::HANDOFF_RECORD);
                report.herdr = Some(Box::new(update(pinned, &herdr_socket, &record)));
            }
            remove_unused_herdr(&placement.root);
            to_value(report)
        }
        KitAction::Status => to_value(hide_kit::status(&target())),
        KitAction::Remove => {
            let kit = hide_kit::remove(&target());
            to_value(KitRemoved {
                kit,
                helper_root: remove_root(&placement.root),
            })
        }
    }
}

/// The paths a `kit` request names, expanded under the node's home.
struct Inputs {
    projects: Vec<PathBuf>,
    cli_dir: PathBuf,
    herdr_socket: PathBuf,
}

impl Inputs {
    fn read(
        home: &Path,
        cli_dir: &str,
        herdr_socket: Option<&str>,
        retirement_projects: &[String],
    ) -> HostResult<Self> {
        let projects = retirement_projects
            .iter()
            .map(|project| expand(project, home))
            .collect::<HostResult<Vec<_>>>()?;
        let cli_dir = expand(cli_dir, home)?;
        let herdr_socket = match herdr_socket {
            Some(socket) => expand(socket, home)?,
            // Herdr's default for this home, in Herdr's own order.
            None => {
                let variables = |name: &str| {
                    if name == hide_platform::host::HOME_VARIABLE {
                        Some(home.as_os_str().to_owned())
                    } else {
                        std::env::var_os(name)
                    }
                };
                hide_platform::host::herdr_socket_from(&variables).map_err(|error| {
                    HostError::new(
                        ErrorCode::Unsupported,
                        format!("Herdr's socket has no location: {error}"),
                    )
                })?
            }
        };
        Ok(Self {
            projects,
            cli_dir,
            herdr_socket,
        })
    }
}

/// What an `apply` or `reinstall` asks the kit to do.
fn scope_of(action: KitAction) -> hide_kit::Scope {
    match action {
        KitAction::Reinstall {
            components,
            agents_on,
            agents_off,
            codex_daemon_off,
        } => {
            let scope = hide_kit::Scope::reinstall(components).merge(hide_kit::Scope::agents(
                agents_on.iter().map(String::as_str),
                agents_off.iter().map(String::as_str),
            ));
            if codex_daemon_off {
                scope.merge(hide_kit::Scope::codex_daemon_off())
            } else {
                scope
            }
        }
        _ => hide_kit::Scope::automatic(),
    }
}

fn to_value(value: impl serde::Serialize) -> HostResult<Value> {
    serde_json::to_value(value).map_err(|error| {
        HostError::new(
            ErrorCode::Io,
            format!("The kit answer could not be encoded: {error}"),
        )
    })
}

/// `~/rest` under `home`, or an absolute path as it is.
fn expand(path: &str, home: &Path) -> HostResult<PathBuf> {
    let expanded = match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        // The core's own node on Windows names its own folders natively.
        None if path.starts_with('/') || Path::new(path).is_absolute() => PathBuf::from(path),
        None => {
            return Err(HostError::new(
                ErrorCode::InvalidPath,
                format!("{path:?} must be absolute or start with ~/"),
            ));
        }
    };
    if expanded
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(HostError::new(
            ErrorCode::InvalidPath,
            format!("{path:?} is not a plain path"),
        ));
    }
    Ok(expanded)
}

/// Points `<root>/current` at `version` through a link made beside it and
/// renamed over it, so the name is never missing in between. On Linux a
/// program started through `current` at that moment can, rarely, be told it
/// does not exist (see `replace_link`).
fn point_current(root: &Path, version: &str) -> HostResult<()> {
    let current = root.join(hide_kit::CURRENT);
    if hide_platform::fs::link::is_link_to(&current, Path::new(version)) {
        return Ok(());
    }
    hide_platform::fs::link::replace_link(Path::new(version), &current).map_err(|error| {
        HostError::new(
            ErrorCode::Io,
            format!(
                "{} could not be pointed at {version}: {error}",
                current.display()
            ),
        )
    })
}

/// The build folders under `root` other than `keep`: only names the core's
/// install creates.
fn builds(root: &Path, keep: Option<&str>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| is_build_name(name) && Some(name) != keep)
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
        })
        .map(|entry| entry.path())
        .collect()
}

/// Removes the builds `current` no longer leads to. A build another
/// connection still runs keeps running: its files stay open until it exits.
fn remove_other_builds(root: &Path, keep: &str) {
    for build in builds(root, Some(keep)) {
        let _ = std::fs::remove_dir_all(build);
    }
}

/// Points `<root>/herdr/current` at the copy this build requires. A missing
/// required copy or refused link replacement fails without removing the
/// last valid link. Only a build that intentionally names no pin removes it.
fn point_herdr(root: &Path, version: &str) -> HostResult<Option<PathBuf>> {
    let folder = device_herdr::herdr_dir(root);
    let current = folder.join(hide_kit::CURRENT);
    let pin_path = root.join(version).join(device_herdr::HERDR_PIN);
    let failure = |reason| HostError::new(ErrorCode::Io, reason);
    let pin = match std::fs::read_to_string(&pin_path) {
        Ok(pin) if is_build_name(pin.trim()) => pin.trim().to_owned(),
        Ok(_) => {
            return Err(failure(
                "The build's Herdr pin is invalid; reconnect to install it again".to_owned(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::symlink_metadata(&current).is_ok_and(|meta| meta.file_type().is_symlink()) {
                hide_platform::fs::link::remove_link(&current).map_err(|error| {
                    failure(format!(
                        "The previous Herdr link could not be removed: {error}"
                    ))
                })?;
            }
            return Ok(None);
        }
        Err(error) => {
            return Err(failure(format!(
                "The build's Herdr pin could not be read: {error}"
            )));
        }
    };
    let copy = folder.join(&pin).join(device_herdr::HERDR_PROGRAM);
    if !copy.is_file() {
        return Err(failure(
            "The required Herdr copy is missing; reconnect to install it again".to_owned(),
        ));
    }
    if !hide_platform::fs::link::is_link_to(&current, Path::new(&pin)) {
        hide_platform::fs::link::replace_link(Path::new(&pin), &current)
            .map_err(|error| failure(format!("The Herdr copy could not be selected: {error}")))?;
    }
    Ok(Some(copy))
}

/// The pinned Herdr copy the build folder `build` names, if any.
fn build_pin(build: &Path) -> Option<String> {
    std::fs::read_to_string(build.join(device_herdr::HERDR_PIN))
        .ok()
        .map(|pin| pin.trim().to_owned())
        .filter(|pin| is_build_name(pin))
}

/// Removes every pinned Herdr copy that no build folder names and no process
/// on the device runs, so a server the operator has not restarted keeps the
/// program it runs (PRD device-herdr-update D-09), and a copy another
/// connection uploaded for its build, whose folder it places first, stays
/// for that build. A copy whose use cannot be read is kept for the next pass.
fn remove_unused_herdr(root: &Path) {
    let named = builds(root, None)
        .iter()
        .filter_map(|build| build_pin(build))
        .collect::<std::collections::HashSet<_>>();
    for copy in builds(&device_herdr::herdr_dir(root), None) {
        let name = copy.file_name().and_then(|name| name.to_str());
        if name.is_some_and(|name| named.contains(name)) || herdr_runs(&copy) {
            continue;
        }
        let _ = std::fs::remove_dir_all(copy);
    }
}

/// Whether a process on the device runs the Herdr in the copy `folder`, or
/// whether that cannot be read.
fn herdr_runs(folder: &Path) -> bool {
    let program = folder.join(device_herdr::HERDR_PROGRAM);
    match hide_platform::fs::identity::file_id(&program) {
        Ok(file) => !matches!(hide_platform::process::running(&file), Ok(pids) if pids.is_empty()),
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

/// A core update's end on the machine running the core (PRD
/// core-host-node-move B10): `<root>/current` leads to `version`, the build
/// the core now runs, and every build but it and `kept` is removed, so the
/// other build of the update stays for the next connection and no more
/// than two are ever kept.
pub fn adopt_build(root: &Path, version: &str, kept: &str) -> HostResult<()> {
    point_current(root, version)?;
    for build in builds(root, Some(version)) {
        if build.file_name().and_then(|name| name.to_str()) != Some(kept) {
            let _ = std::fs::remove_dir_all(build);
        }
    }
    Ok(())
}

/// Removes every build, the `current` link and then the root itself, which
/// is removed only when nothing else is left in it: the root is the folder
/// the operator allowed Hide to own, but a file someone else put there is not
/// Hide's to delete.
fn remove_root(root: &Path) -> hide_kit::RemoveOutcome {
    let mut failures = Vec::new();
    for build in builds(root, None) {
        if let Err(error) = std::fs::remove_dir_all(&build) {
            failures.push(format!("{} stayed: {error}", build.display()));
        }
    }
    let herdr_kept = remove_herdr(root);
    let current = root.join(hide_kit::CURRENT);
    if std::fs::symlink_metadata(&current).is_ok_and(|meta| meta.file_type().is_symlink())
        && let Err(error) = hide_platform::fs::link::remove_link(&current)
    {
        failures.push(format!("{} stayed: {error}", current.display()));
    }
    if !failures.is_empty() {
        return hide_kit::RemoveOutcome::Failed {
            reason: failures.join("; "),
        };
    }
    if let Some(reason) = herdr_kept {
        return hide_kit::RemoveOutcome::Kept { reason };
    }
    match std::fs::remove_dir(root) {
        Ok(()) => hide_kit::RemoveOutcome::Removed,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            hide_kit::RemoveOutcome::Absent
        }
        Err(_) => hide_kit::RemoveOutcome::Kept {
            reason: format!(
                "{} holds files Hide did not put there, so it stays",
                root.display()
            ),
        },
    }
}

/// Takes the pinned Herdr copies off a device leaving Hide: every copy no
/// process runs, the link, the handoff record and the folder. A copy a
/// server still runs stays with the folder (D-09), and the answer says why.
fn remove_herdr(root: &Path) -> Option<String> {
    let folder = device_herdr::herdr_dir(root);
    if !folder.exists() {
        return None;
    }
    let current = folder.join(hide_kit::CURRENT);
    if std::fs::symlink_metadata(&current).is_ok_and(|meta| meta.file_type().is_symlink()) {
        let _ = hide_platform::fs::link::remove_link(&current);
    }
    let _ = std::fs::remove_file(folder.join(device_herdr::HANDOFF_RECORD));
    remove_unused_herdr(root);
    match std::fs::remove_dir(&folder) {
        Ok(()) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(format!(
            "{} stays while a Herdr server on the device runs a copy in it; delete it once that server stops",
            folder.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The external server updater records an attempted handoff. The kit
    /// itself runs against private files and a raised cancellation flag.
    #[test]
    fn saved_session_apply_and_reinstall_leave_the_selected_server_untouched() {
        fn updater(_pinned: &Path, _socket: &Path, record: &Path) -> HerdrUpdate {
            std::fs::write(record, "handoff attempted").unwrap();
            HerdrUpdate::Current {
                version: "fixture".to_owned(),
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let placement = placed(dir.path(), "aaaaaaaaaaaaaaaa");
        let copy = device_herdr::herdr_dir(&placement.root).join("bbbbbbbbbbbbbbbb");
        std::fs::create_dir_all(&copy).unwrap();
        std::fs::write(copy.join(device_herdr::HERDR_PROGRAM), "private fixture").unwrap();
        std::fs::write(
            placement
                .root
                .join(&placement.version)
                .join(device_herdr::HERDR_PIN),
            "bbbbbbbbbbbbbbbb\n",
        )
        .unwrap();
        let record = device_herdr::herdr_dir(&placement.root).join(device_herdr::HANDOFF_RECORD);
        for action in [
            KitAction::Apply,
            KitAction::Reinstall {
                components: Vec::new(),
                agents_on: Vec::new(),
                agents_off: Vec::new(),
                codex_daemon_off: false,
            },
        ] {
            let value = run_for_projects(
                &placement,
                action,
                "~/.local/bin",
                Some("~/review.sock"),
                &[],
                Arc::new(AtomicBool::new(true)),
                HerdrPolicy {
                    preserve_server: true,
                    updater: Some(updater),
                },
            )
            .unwrap();
            let report: hide_kit::KitReport = serde_json::from_value(value).unwrap();
            assert!(report.herdr.is_none());
            assert!(
                !record.exists(),
                "saved-session consent does not permit a server handoff"
            );
        }
        let value = run_for_projects(
            &placement,
            KitAction::Apply,
            "~/.local/bin",
            Some("~/default.sock"),
            &[],
            Arc::new(AtomicBool::new(true)),
            HerdrPolicy {
                preserve_server: false,
                updater: Some(updater),
            },
        )
        .unwrap();
        let report: hide_kit::KitReport = serde_json::from_value(value).unwrap();
        assert!(matches!(
            report.herdr.as_deref(),
            Some(HerdrUpdate::Current { .. })
        ));
        assert_eq!(
            std::fs::read_to_string(record).unwrap(),
            "handoff attempted"
        );
    }

    fn placed(dir: &Path, version: &str) -> Placement {
        let root = dir.join("host-helper");
        std::fs::create_dir_all(root.join(version)).unwrap();
        std::fs::write(root.join(version).join("hided"), b"helper").unwrap();
        Placement {
            root,
            version: version.to_owned(),
            home: dir.join("home"),
        }
    }

    #[test]
    fn only_a_helper_in_a_build_folder_runs_the_kit() {
        let (root, version) = install_layout(Path::new(
            "/home/me/.local/share/hide/host-helper/0123456789abcdef/hided",
        ))
        .unwrap();
        assert_eq!(
            root,
            PathBuf::from("/home/me/.local/share/hide/host-helper")
        );
        assert_eq!(version, "0123456789abcdef");
        for elsewhere in [
            "/work/hide/target/debug/hided",
            "/home/me/.local/share/hide/host-helper/current/hided",
            "/Applications/hide.app/Contents/Resources/hided",
        ] {
            let refused = install_layout(Path::new(elsewhere)).unwrap_err();
            assert_eq!(refused.code, ErrorCode::Unsupported, "{elsewhere}");
        }
    }

    /// Writes each of `programs` into `folder` as this system names a
    /// program, and answers the path of the first.
    fn programs(folder: &Path, programs: &[&str]) -> PathBuf {
        std::fs::create_dir_all(folder).unwrap();
        for program in programs {
            let file = folder.join(format!("{program}{}", std::env::consts::EXE_SUFFIX));
            std::fs::write(file, b"program").unwrap();
        }
        folder.join(format!("{}{}", programs[0], std::env::consts::EXE_SUFFIX))
    }

    /// A core or a node installs the kit only from a program Hide put on
    /// the machine: the app's package on each system (a macOS bundle, a
    /// Linux or Windows package's resources, a Windows build's runtime
    /// copy), or a whole build in the account's own helper root, which a
    /// moved core's login item runs. A copy of a build anywhere else, a
    /// lone `hided` in a build folder, a development build, or a node with
    /// no home installs nothing into the account.
    #[test]
    fn a_core_installs_from_its_package_or_a_placed_build_and_never_from_another_copy() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let place = |executable: &Path| KitPlace::of_program(executable, Some(&home));

        assert_eq!(
            place(Path::new("/Applications/hide.app/Contents/Resources/hided")),
            KitPlace::Bundled(PathBuf::from("/Applications/hide.app/Contents/Resources"))
        );
        let package = dir.path().join("hide-package");
        programs(&package, &["hide"]);
        let resources = package.join("resources");
        let unpacked = programs(&resources, &["hided"]);
        std::fs::write(resources.join("app.asar"), b"shell").unwrap();
        assert_eq!(place(&unpacked), KitPlace::Bundled(resources));
        let copy = home.join(".hide/runtime/0.4.0-0123456789ab");
        let runtime = programs(&copy, &["hided", "hide", "hide-agent-hooks", "herdr"]);
        std::fs::write(copy.join(hide_kit::runtime::MARKER), "0.4.0-0123456789ab").unwrap();
        assert_eq!(place(&runtime), KitPlace::Bundled(copy));

        let helper_root = hide_kit::layout::helper_root(&home);
        let whole = ["hided", "hide", "hide-agent-hooks"];
        let placed = programs(&helper_root.join("0123456789abcdef"), &whole);
        assert_eq!(
            place(&placed),
            KitPlace::Build {
                root: helper_root.clone(),
                version: "0123456789abcdef".to_owned(),
            }
        );
        assert_eq!(KitPlace::of_program(&placed, None), KitPlace::Standalone);

        let stray = programs(&dir.path().join("Downloads/0123456789abcdef"), &whole);
        let lone = programs(&helper_root.join("fedcba9876543210"), &["hided"]);
        let current = helper_root.join("current/hided");
        let debug = programs(&dir.path().join("hide/target/debug"), &whole);
        for elsewhere in [stray, lone, current, debug] {
            assert_eq!(
                place(&elsewhere),
                KitPlace::Standalone,
                "{}",
                elsewhere.display()
            );
        }
    }

    /// A moved core's pass installs from the build it runs and keeps the
    /// build a core update may go back to (`adopt_build`): `current` leads
    /// to the running build before anything is installed, and no other
    /// build is removed. Another kit holds the account and the core is
    /// quitting, so the pass ends there, before any part or child.
    #[test]
    fn a_core_s_kit_points_current_at_its_build_and_removes_no_other() {
        let dir = tempfile::tempdir().unwrap();
        let previous = placed(dir.path(), "aaaaaaaaaaaaaaaa");
        let placement = placed(dir.path(), "bbbbbbbbbbbbbbbb");
        hide_platform::fs::link::create_link(
            Path::new(&previous.version),
            &placement.root.join("current"),
        )
        .unwrap();
        let kit_folder = placement.home.join(".hide/kit");
        hide_platform::fs::private::create_dir_all(&kit_folder).unwrap();
        let held = hide_platform::fs::lock::lock_file(
            hide_platform::fs::private::open_or_create_file(&kit_folder.join(".lock")).unwrap(),
            hide_platform::fs::lock::Mode::Exclusive,
            std::time::Duration::from_secs(5),
            &|| false,
        )
        .unwrap();

        let report = run_build(
            &placement,
            KitAction::Apply,
            "~/.local/bin",
            Some("~/private-herdr.sock"),
            &[],
            Arc::new(AtomicBool::new(true)),
        )
        .unwrap();
        drop(held);
        let report: hide_kit::KitReport = serde_json::from_value(report).unwrap();
        assert!(
            report.components.iter().all(|part| {
                part.state == hide_kit::ComponentState::Failed
                    && part.reason.as_deref() == Some("Hide is quitting")
            }),
            "{report:?}"
        );
        assert!(hide_platform::fs::link::is_link_to(
            &placement.root.join("current"),
            Path::new(&placement.version)
        ));
        assert!(
            placement
                .root
                .join(&previous.version)
                .join("hided")
                .is_file()
        );
        assert!(!placement.home.join(".local/bin").exists());

        let refused = run_build(
            &placement,
            KitAction::Remove,
            "~/.local/bin",
            None,
            &[],
            Arc::default(),
        )
        .unwrap_err();
        assert_eq!(refused.code, ErrorCode::Unsupported);
        assert!(placement.root.join(&placement.version).is_dir());
    }

    #[test]
    fn an_install_points_current_at_the_running_build_and_drops_older_ones() {
        let dir = tempfile::tempdir().unwrap();
        let older = placed(dir.path(), "aaaaaaaaaaaaaaaa");
        let placement = placed(dir.path(), "bbbbbbbbbbbbbbbb");
        std::fs::write(placement.root.join("notes.txt"), b"mine").unwrap();
        hide_platform::fs::link::create_link(
            Path::new(&older.version),
            &placement.root.join("current"),
        )
        .unwrap();

        point_current(&placement.root, &placement.version).unwrap();
        remove_other_builds(&placement.root, &placement.version);

        assert!(hide_platform::fs::link::is_link_to(
            &placement.root.join("current"),
            Path::new("bbbbbbbbbbbbbbbb")
        ));
        assert!(!placement.root.join(&older.version).exists());
        assert!(placement.root.join("current/hided").is_file());
        // Only build folders are Hide's to remove.
        assert!(placement.root.join("notes.txt").is_file());
    }

    #[test]
    fn blocked_retirement_does_not_repoint_current_or_remove_a_build() {
        let directory = tempfile::tempdir().unwrap();
        let older = placed(directory.path(), "aaaaaaaaaaaaaaaa");
        let placement = placed(directory.path(), "bbbbbbbbbbbbbbbb");
        hide_platform::fs::link::create_link(
            Path::new(&older.version),
            &placement.root.join("current"),
        )
        .unwrap();
        let ledger = placement.home.join(".hide/hcoord/ledger.json");
        std::fs::create_dir_all(ledger.parent().unwrap()).unwrap();
        let bytes = serde_json::json!({"schema":"hcoord.ledger.v1","requests":{"open":{"status":"open"}},"watches":{}}).to_string();
        std::fs::write(&ledger, &bytes).unwrap();
        let report = run(
            &placement,
            KitAction::Apply,
            "~/.local/bin",
            Some("~/private-herdr.sock"),
            Arc::default(),
        )
        .unwrap();
        let report: hide_kit::KitReport = serde_json::from_value(report).unwrap();
        let refusal = report.refused.expect("the pass stops before anything");
        assert_eq!(refusal.cause, hide_kit::RefusalCause::Record);
        assert_eq!(refusal.path.as_deref(), ledger.to_str());
        assert!(hide_platform::fs::link::is_link_to(
            &placement.root.join("current"),
            Path::new(&older.version)
        ));
        assert!(placement.root.join(&older.version).is_dir());
        assert_eq!(std::fs::read_to_string(ledger).unwrap(), bytes);
        assert!(!placement.home.join(".hide/kit").exists());
        assert!(!placement.home.join(".local/bin").exists());
    }

    #[test]
    fn a_core_update_keeps_the_build_it_runs_and_the_other_one_of_the_update() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for build in ["aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb", "cccccccccccccccc"] {
            std::fs::create_dir_all(root.join(build)).unwrap();
        }
        adopt_build(root, "cccccccccccccccc", "bbbbbbbbbbbbbbbb").unwrap();
        assert!(hide_platform::fs::link::is_link_to(
            &root.join("current"),
            Path::new("cccccccccccccccc")
        ));
        let mut left: Vec<String> = std::fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["bbbbbbbbbbbbbbbb", "cccccccccccccccc", "current"]);
    }

    #[test]
    fn removal_takes_the_root_only_when_nothing_else_is_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let placement = placed(dir.path(), "bbbbbbbbbbbbbbbb");
        point_current(&placement.root, &placement.version).unwrap();
        std::fs::write(placement.root.join("notes.txt"), b"mine").unwrap();

        let kept = remove_root(&placement.root);
        assert!(
            matches!(kept, hide_kit::RemoveOutcome::Kept { .. }),
            "{kept:?}"
        );
        assert!(!placement.root.join("bbbbbbbbbbbbbbbb").exists());
        assert!(placement.root.join("notes.txt").is_file());

        std::fs::remove_file(placement.root.join("notes.txt")).unwrap();
        assert_eq!(
            remove_root(&placement.root),
            hide_kit::RemoveOutcome::Removed
        );
        assert!(!placement.root.exists());
    }

    /// A pinned Herdr copy under `root` named `id`, made from `program`.
    #[cfg(unix)]
    fn herdr_copy(root: &Path, id: &str, program: &Path) -> PathBuf {
        let folder = device_herdr::herdr_dir(root).join(id);
        std::fs::create_dir_all(&folder).unwrap();
        let copy = folder.join(device_herdr::HERDR_PROGRAM);
        std::fs::write(&copy, std::fs::read(program).unwrap()).unwrap();
        std::fs::set_permissions(
            &copy,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .unwrap();
        copy
    }

    /// Set on a copy of this test binary that stands in for a Herdr server
    /// running a pinned copy: it waits for its parent to end it. A copy of a
    /// system program will not do, because macOS kills a fresh copy of one
    /// as it starts.
    const HELD: &str = "HIDE_HOST_HELD_COPY";

    #[test]
    fn held_copy_role() {
        if std::env::var_os(HELD).is_some() {
            use std::io::Read;
            let _ = std::io::stdin().read(&mut [0]);
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_build_points_herdr_current_at_its_pinned_copy_and_a_build_without_one_takes_it_away() {
        let dir = tempfile::tempdir().unwrap();
        let placement = placed(dir.path(), "bbbbbbbbbbbbbbbb");
        let copy = herdr_copy(&placement.root, "1111111111111111", Path::new("/bin/sh"));
        std::fs::write(
            placement
                .root
                .join(&placement.version)
                .join(device_herdr::HERDR_PIN),
            "1111111111111111\n",
        )
        .unwrap();

        assert_eq!(
            point_herdr(&placement.root, &placement.version).unwrap(),
            Some(copy)
        );
        assert!(device_herdr::pinned_herdr(&placement.root).is_file());

        let without = placed(dir.path(), "cccccccccccccccc");
        assert_eq!(point_herdr(&without.root, &without.version).unwrap(), None);
        assert!(
            std::fs::symlink_metadata(
                device_herdr::herdr_dir(&without.root).join(hide_kit::CURRENT)
            )
            .is_err()
        );
    }

    #[test]
    #[cfg(unix)]
    fn an_incomplete_herdr_delivery_fails_without_changing_either_current_link() {
        let dir = tempfile::tempdir().unwrap();
        let old = placed(dir.path(), "aaaaaaaaaaaaaaaa");
        let copy = pinned_build(&old.root, &old.version, "1111111111111111");
        point_current(&old.root, &old.version).unwrap();
        point_herdr(&old.root, &old.version).unwrap();
        let next = placed(dir.path(), "bbbbbbbbbbbbbbbb");
        std::fs::write(
            next.root.join(&next.version).join(device_herdr::HERDR_PIN),
            "2222222222222222\n",
        )
        .unwrap();
        let result = run_for_projects(
            &next,
            KitAction::Apply,
            "~/.local/bin",
            Some("~/private.sock"),
            &[],
            Arc::new(AtomicBool::new(true)),
            HerdrPolicy {
                preserve_server: false,
                updater: None,
            },
        );
        assert!(result.is_err(), "a required pinned copy did not arrive");
        assert!(hide_platform::fs::link::is_link_to(
            &old.root.join(hide_kit::CURRENT),
            Path::new(&old.version)
        ));
        assert!(hide_platform::fs::link::is_link_to(
            &device_herdr::herdr_dir(&old.root).join(hide_kit::CURRENT),
            Path::new("1111111111111111")
        ));
        assert!(copy.is_file());
    }

    /// A build folder `version` under `root` that names the copy `pin`, and
    /// that copy.
    #[cfg(unix)]
    fn pinned_build(root: &Path, version: &str, pin: &str) -> PathBuf {
        std::fs::create_dir_all(root.join(version)).unwrap();
        std::fs::write(
            root.join(version).join(device_herdr::HERDR_PIN),
            format!("{pin}\n"),
        )
        .unwrap();
        herdr_copy(root, pin, &std::env::current_exe().unwrap())
    }

    /// A process running the copy `program` of this test binary, once it
    /// has replaced its image.
    #[cfg(unix)]
    fn run_copy(program: &Path) -> hide_platform::process::OwnedChild {
        let server = hide_platform::process::OwnedChild::spawn(
            std::process::Command::new(program)
                .args(["--exact", "kit::tests::held_copy_role", "--test-threads=1"])
                .env(HELD, "1")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null()),
        )
        .unwrap();
        let file = hide_platform::fs::identity::file_id(program).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while hide_platform::process::running(&file).unwrap().is_empty() {
            assert!(std::time::Instant::now() < deadline, "the copy never ran");
            std::thread::yield_now();
        }
        server
    }

    #[test]
    #[cfg(unix)]
    fn an_older_herdr_copy_stays_while_a_process_runs_it_and_goes_once_none_does() {
        let dir = tempfile::tempdir().unwrap();
        let placement = placed(dir.path(), "bbbbbbbbbbbbbbbb");
        let root = placement.root;
        let pinned = pinned_build(&root, "bbbbbbbbbbbbbbbb", "2222222222222222");
        // Another connection's build, placed before its apply.
        let arriving = pinned_build(&root, "cccccccccccccccc", "3333333333333333");
        let test_binary = std::env::current_exe().unwrap();
        let older = herdr_copy(&root, "1111111111111111", &test_binary);
        let unused = herdr_copy(&root, "0000000000000000", &test_binary);
        let mut server = run_copy(&older);

        remove_unused_herdr(&root);
        assert!(pinned.is_file());
        assert!(arriving.is_file(), "a copy a build names stays");
        assert!(older.is_file(), "the copy a server runs stays");
        assert!(!unused.parent().unwrap().exists());

        server.kill_tree().unwrap();
        server.wait().unwrap();
        remove_unused_herdr(&root);
        assert!(!older.parent().unwrap().exists());
        assert!(pinned.is_file());
    }

    #[test]
    #[cfg(unix)]
    fn removal_takes_the_herdr_copies_no_server_runs() {
        let dir = tempfile::tempdir().unwrap();
        let placement = placed(dir.path(), "bbbbbbbbbbbbbbbb");
        point_current(&placement.root, &placement.version).unwrap();
        herdr_copy(&placement.root, "1111111111111111", Path::new("/bin/sh"));
        std::fs::write(
            placement
                .root
                .join(&placement.version)
                .join(device_herdr::HERDR_PIN),
            "1111111111111111\n",
        )
        .unwrap();
        point_herdr(&placement.root, &placement.version).unwrap();
        std::fs::write(
            device_herdr::herdr_dir(&placement.root).join(device_herdr::HANDOFF_RECORD),
            b"{}",
        )
        .unwrap();

        assert_eq!(
            remove_root(&placement.root),
            hide_kit::RemoveOutcome::Removed
        );
        assert!(!placement.root.exists());
    }

    #[test]
    #[cfg(unix)]
    fn removal_keeps_the_root_while_a_server_runs_its_herdr_copy_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let placement = placed(dir.path(), "bbbbbbbbbbbbbbbb");
        let copy = pinned_build(&placement.root, &placement.version, "1111111111111111");
        let mut server = run_copy(&copy);

        let outcome = remove_root(&placement.root);
        server.kill_tree().unwrap();
        server.wait().unwrap();
        let hide_kit::RemoveOutcome::Kept { reason } = outcome else {
            panic!("a copy in use keeps the root: {outcome:?}");
        };
        assert!(reason.contains("Herdr server"), "{reason}");
        assert!(copy.is_file());
        assert!(!placement.root.join(&placement.version).exists());
    }

    #[test]
    fn a_request_path_outside_home_notation_is_refused() {
        let home = Path::new("/home/me");
        assert_eq!(
            expand("~/.local/bin", home).unwrap(),
            home.join(".local/bin")
        );
        assert_eq!(expand("/opt/bin", home).unwrap(), PathBuf::from("/opt/bin"));
        for refused in ["bin", "~/../other", "/opt/../etc"] {
            assert_eq!(
                expand(refused, home).unwrap_err().code,
                ErrorCode::InvalidPath,
                "{refused}"
            );
        }
    }
}
