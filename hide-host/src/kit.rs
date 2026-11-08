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
//! A Herdr copy an earlier build put beside the builds
//! (`hide_kit::device_herdr`) is taken off once no process on the device
//! runs it (PRD engine-bundle-4-herdr-removal D-10): no build brings one now.
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

use hide_kit::device_herdr;
pub use hide_kit::is_build_name;

/// Answers one `kit` request for the node `env` describes: the helper root
/// the running helper was installed under, the build a core or node runs
/// from, or the desktop package it runs from.
pub fn handle(
    action: KitAction,
    cli_dir: &str,
    retirement_projects: &[String],
    env: &Env,
) -> HostResult<Value> {
    match &env.kit {
        KitPlace::Installed => {}
        KitPlace::Build { root, version } => {
            let home = env.home.clone().ok_or_else(|| {
                HostError::new(
                    ErrorCode::Unsupported,
                    "HOME is not set, so Hide cannot tell where to install",
                )
            })?;
            let answer = run_build(
                &Placement {
                    root: root.clone(),
                    version: version.clone(),
                    home,
                },
                action,
                cli_dir,
                retirement_projects,
                Arc::clone(&env.stop),
                crate::profiles::source(env),
            );
            return settled(env, answer);
        }
        KitPlace::Bundled(kit_dir) => {
            return settled(
                env,
                bundled(kit_dir, env, action, cli_dir, retirement_projects),
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
    let answer = run_for_projects(
        &Placement {
            root,
            version,
            home,
        },
        action,
        cli_dir,
        retirement_projects,
        Arc::clone(&env.stop),
        crate::profiles::source(env),
    );
    settled(env, answer)
}

/// A kit answer, after the profiles being removed whose accounts the pass
/// found free of Hide's pieces have left the profile file.
fn settled(env: &Env, answer: HostResult<Value>) -> HostResult<Value> {
    if let Ok(answer) = &answer
        && let Some(profiles) = answer.get("profiles")
        && let Ok(reports) =
            serde_json::from_value::<Vec<hide_kit::ProfileReport>>(profiles.clone())
    {
        crate::profiles::settle_after_kit(env, &reports);
    }
    answer
}

/// The kit target of this node's own installation, for a pass over the
/// profile accounts alone (`hide_kit::apply_profiles`), which names no
/// command folder and changes no build.
pub(crate) fn profile_target(env: &Env) -> HostResult<hide_kit::KitTarget> {
    let home = env.home.clone().ok_or_else(|| {
        HostError::new(
            ErrorCode::Unsupported,
            "HOME is not set, so Hide cannot tell where to install",
        )
    })?;
    let cli_dir = home.join(".local").join("bin");
    let stop = Arc::clone(&env.stop);
    match &env.kit {
        KitPlace::Installed => {
            let executable = std::env::current_exe().map_err(|error| {
                HostError::new(
                    ErrorCode::Io,
                    format!("The helper could not tell where it runs from: {error}"),
                )
            })?;
            let (root, _) = install_layout(&executable)?;
            Ok(hide_kit::device_target(&root, &home, &cli_dir, stop))
        }
        KitPlace::Build { root, .. } => Ok(hide_kit::device_target(root, &home, &cli_dir, stop)),
        KitPlace::Bundled(kit_dir) => Ok(hide_kit::local_target(kit_dir, &home, stop)),
        KitPlace::Standalone => Err(HostError::new(
            ErrorCode::Unsupported,
            hide_kit::STANDALONE_REASON,
        )),
    }
}

/// The kit of the machine whose desktop package holds the parts: they stay
/// where the package put them, so nothing is pointed or pruned, and the
/// machine itself is never removed from Hide.
fn bundled(
    kit_dir: &Path,
    env: &Env,
    action: KitAction,
    cli_dir: &str,
    retirement_projects: &[String],
) -> HostResult<Value> {
    let home = env.home.clone().ok_or_else(|| {
        HostError::new(
            ErrorCode::Unsupported,
            "HOME is not set, so Hide cannot tell where to install",
        )
    })?;
    let inputs = Inputs::read(&home, cli_dir, retirement_projects)?;
    let mut target = hide_kit::local_target(kit_dir, &home, Arc::clone(&env.stop));
    target.cli_dir = inputs.cli_dir;
    target.retirement_projects = inputs.projects;
    target.profiles = crate::profiles::source(env);
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
/// build, since the move and the core update own them (`adopt_build`). Public for the core's tests, which place a build
/// folder without a device.
pub fn run_build(
    placement: &Placement,
    action: KitAction,
    cli_dir: &str,
    retirement_projects: &[String],
    stop: Arc<AtomicBool>,
    profiles: Option<hide_kit::ProfileSource>,
) -> HostResult<Value> {
    let home = &placement.home;
    let Inputs { projects, cli_dir } = Inputs::read(home, cli_dir, retirement_projects)?;
    let mut target = hide_kit::device_target(&placement.root, home, &cli_dir, stop);
    target.retirement_projects = projects;
    target.profiles = profiles;
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
/// The Hide installation a pane holder's pane belongs to, as the pane names
/// it ([`hide_agent_hooks::route::INSTALL_VARIABLE`] and
/// [`hide_agent_hooks::route::CLI_VARIABLE`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneInstall {
    /// The installation's name, the one its hook entries carry.
    pub id: String,
    /// Its own `hide` command, at a path that outlives one build.
    pub cli: PathBuf,
    /// The folder a pane's `PATH` starts with, which holds this
    /// installation's `hide` and nothing else (`hide_kit::current::path_folder`),
    /// so the pane's `hide` is its own whoever holds the account's command
    /// folder; `None` where the pane's `hide` hands itself over instead.
    pub path_first: Option<PathBuf>,
}

impl PaneInstall {
    /// The variables that name this installation in a pane.
    pub fn variables(&self) -> [(std::ffi::OsString, std::ffi::OsString); 2] {
        [
            (
                hide_agent_hooks::route::INSTALL_VARIABLE.into(),
                self.id.clone().into(),
            ),
            (
                hide_agent_hooks::route::CLI_VARIABLE.into(),
                self.cli.clone().into_os_string(),
            ),
        ]
    }
}

/// The installation the program at `executable` belongs to: the desktop
/// package it runs from, or the helper root its build sits in (a device
/// helper's, or a build Hide placed there), whose `current` leads to the
/// build that runs now. A Cargo build or any other copy belongs to none.
pub fn pane_install(executable: &Path) -> Option<PaneInstall> {
    let cli = if cfg!(windows) { "hide.exe" } else { "hide" };
    if let Some(kit_dir) = hide_kit::bundled_kit_dir(executable) {
        return Some(PaneInstall {
            id: hide_agent_hooks::route::APP_INSTALL.to_owned(),
            cli: kit_dir.join(cli),
            path_first: None,
        });
    }
    let (root, _) = build_layout(executable)?;
    Some(PaneInstall {
        id: hide_agent_hooks::route::install_of_root(&root),
        cli: root.join(hide_kit::CURRENT).join(cli),
        path_first: hide_kit::current::path_folder(&root),
    })
}

/// The installation this running program belongs to ([`pane_install`] of
/// its own executable): the one the panes it starts name, and the one its
/// answers to a pane's `hide` confirm.
pub fn running_install() -> Option<PaneInstall> {
    std::env::current_exe()
        .ok()
        .and_then(|program| pane_install(&program))
}

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
    stop: Arc<AtomicBool>,
) -> HostResult<Value> {
    run_for_projects(placement, action, cli_dir, &[], stop, None)
}

fn run_for_projects(
    placement: &Placement,
    action: KitAction,
    cli_dir: &str,
    retirement_projects: &[String],
    stop: Arc<AtomicBool>,
    profiles: Option<hide_kit::ProfileSource>,
) -> HostResult<Value> {
    let home = &placement.home;
    let Inputs { projects, cli_dir } = Inputs::read(home, cli_dir, retirement_projects)?;
    let target = || {
        let mut target =
            hide_kit::device_target(&placement.root, home, &cli_dir, Arc::clone(&stop));
        target.retirement_projects = projects.clone();
        target.profiles = profiles.clone();
        target
    };
    match action {
        KitAction::Apply | KitAction::Reinstall { .. } => {
            if let Err(refusal) = hide_kit::retirement_preflight(&target()) {
                return to_value(hide_kit::KitReport::refused(refusal));
            }
            point_current(&placement.root, &placement.version)?;
            remove_other_builds(&placement.root, &placement.version);
            let report = hide_kit::apply(&target(), &scope_of(action));
            // A copy a server still runs stays, with a record, for a later pass.
            let _ = remove_herdr(&placement.root);
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
}

impl Inputs {
    fn read(home: &Path, cli_dir: &str, retirement_projects: &[String]) -> HostResult<Self> {
        let projects = retirement_projects
            .iter()
            .map(|project| expand(project, home))
            .collect::<HostResult<Vec<_>>>()?;
        let cli_dir = expand(cli_dir, home)?;
        Ok(Self { projects, cli_dir })
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

/// Points `<root>/current` at `version` (`hide_kit::current::point`).
fn point_current(root: &Path, version: &str) -> HostResult<()> {
    hide_kit::current::point(root, version).map_err(|error| {
        HostError::new(
            ErrorCode::Io,
            format!(
                "{} could not be pointed at {version}: {error}",
                root.join(hide_kit::CURRENT).display()
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

/// Removes every Herdr copy no process on the device runs, so a server the
/// operator has not restarted keeps the program it runs (PRD
/// device-herdr-update D-09). A copy whose use cannot be read is kept for the
/// next pass. A copy kept for its use and a removal that fails each leave one
/// record on the node's error stream, its diagnostic log.
fn remove_unused_herdr(root: &Path) {
    for copy in builds(&device_herdr::herdr_dir(root), None) {
        if let Some(reason) = herdr_use(&copy) {
            herdr_copy_record("kit.herdr_copy_kept", &copy, &reason);
            continue;
        }
        if let Err(error) = std::fs::remove_dir_all(&copy) {
            herdr_copy_record("kit.herdr_copy_remove_failed", &copy, &error.to_string());
        }
    }
}

fn herdr_copy_record(kind: &str, copy: &Path, reason: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "kit",
            "kind": kind,
            "copy": copy,
            "reason": reason,
        })
    );
}

/// Why the Herdr copy `folder` stays: a process on the device runs it, or
/// whether one does cannot be read. `None` when nothing runs it.
///
/// On Windows a copy is in use while a process holds any file in it, which
/// a running Herdr's image is: Windows names a process's image only by the
/// path it was started from, and a process whose path cannot be read (one
/// that ends during the look, or that the system will not describe) would
/// otherwise be left out and its copy removed around it, every file but the
/// running image gone. A folder whose files cannot all be asked keeps it.
fn herdr_use(folder: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        match hide_platform::fs::in_use::first_in_use(folder) {
            Ok(held) => held.map(|file| format!("{} is in use", file.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => Some(format!(
                "whether its files are in use could not be read: {error}"
            )),
        }
    }
    #[cfg(not(windows))]
    {
        let program = folder.join(device_herdr::HERDR_PROGRAM);
        match hide_platform::fs::identity::file_id(&program) {
            Ok(file) => match hide_platform::process::running(&file) {
                Ok(pids) if pids.is_empty() => None,
                Ok(pids) => Some(format!("processes {pids:?} run it")),
                Err(error) => Some(format!(
                    "whether a process runs it could not be read: {error}"
                )),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => Some(format!("its program could not be read: {error}")),
        }
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

/// Removes every build, `current` and then the root itself, which
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
    if let Err(error) = hide_kit::current::remove(root) {
        failures.push(format!(
            "{} stayed: {error}",
            root.join(hide_kit::CURRENT).display()
        ));
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

/// Takes the Herdr copies an earlier build put on the device off it, on every
/// pass and when the device leaves Hide: every copy no process runs, the
/// link, the handoff record and the folder. A copy a server still runs stays
/// with the folder (D-09), and the answer says why.
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

    /// A device helper started with a home of its own
    /// (`HIDE_DEV_DEVICE_HOME`) keeps every place its kit and state write
    /// under that home, whatever the account's environment still says: the
    /// start step replaces the platform's home variable, clears
    /// [`HOME_FOLDER_VARIABLES`] and sets each agent's home variable inside
    /// it, and every other variable here names a
    /// folder of the account's (Windows OpenSSH sets `HOME` beside the
    /// `USERPROFILE` the start step replaces, and a kit that read `HOME`
    /// there wrote the account's own hook files).
    #[test]
    fn every_kit_location_follows_the_home_the_platform_names() {
        use crate::serve::{HOME_FOLDER_VARIABLES, agent_home_variables};
        use hide_agent_adapter::{ADAPTERS, AccountPath, NodeEnv, Profile};
        let dir = tempfile::tempdir().unwrap();
        let own = dir.path().join("own");
        let account = dir.path().join("account");
        let inherited = |name: &str| {
            if name == hide_platform::host::HOME_VARIABLE {
                Some(own.clone().into_os_string())
            } else {
                Some(account.join(name).into_os_string())
            }
        };
        // Left as the account has them, the state folder is theirs.
        let env = Env::of_variables(&inherited);
        assert!(!env.state.unwrap().starts_with(&own));

        let agents = agent_home_variables();
        let started = |name: &str| {
            if HOME_FOLDER_VARIABLES.contains(&name) {
                None
            } else if let Some((_, folder)) = agents.iter().find(|(variable, _)| *variable == name)
            {
                Some(own.join(folder).into_os_string())
            } else {
                inherited(name)
            }
        };
        let env = Env::of_variables(&started);
        assert_eq!(env.home.as_deref(), Some(own.as_path()));
        assert_eq!(
            env.state.as_deref(),
            Some(own.join(".hide").join("state").as_path())
        );
        let home = env.home.unwrap();
        let inputs = Inputs::read(&home, "~/.local/bin", &["~/work".to_owned()]).unwrap();
        let mut locations = vec![hide_kit::kit_state_dir(&home), inputs.cli_dir];
        locations.extend(inputs.projects);
        locations.extend(
            hide_agent_hooks::runtime::AgentRuntime::ALL.map(|runtime| runtime.config_path(&home)),
        );
        // Each agent's places as Hide reads them for a pane there: its
        // default account, following the variables the start step sets.
        for agent in ADAPTERS {
            let mut node = NodeEnv::new(Some(home.clone()));
            for (variable, _) in agents.iter() {
                node = node.with_variable(variable, started(variable));
            }
            let account = agent
                .resolve(&Profile::builtin_default(agent.id), &node)
                .unwrap();
            locations.extend(
                AccountPath::ALL
                    .into_iter()
                    .filter_map(|path| account.path(path).map(Path::to_path_buf)),
            );
        }
        for location in locations {
            assert!(
                location.starts_with(&own),
                "{} is outside the helper's home",
                location.display()
            );
        }
    }

    fn placed(dir: &Path, version: &str) -> Placement {
        let root = dir.join("host-helper");
        std::fs::create_dir_all(root.join(version)).unwrap();
        std::fs::write(root.join(version).join("hided"), b"helper").unwrap();
        // What `current` leads to.
        programs(&root.join(version), &["hide", "hide-agent-hooks"]);
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
    /// Whether the `hide` that `<root>/current` leads to is build
    /// `version`'s, as a pane or a hook running it finds.
    fn current_runs(root: &Path, version: &str) -> bool {
        let hide = format!("hide{}", std::env::consts::EXE_SUFFIX);
        std::fs::read(root.join(hide_kit::CURRENT).join(&hide)).ok()
            == Some(std::fs::read(root.join(version).join(&hide)).unwrap())
    }

    fn programs(folder: &Path, programs: &[&str]) -> PathBuf {
        std::fs::create_dir_all(folder).unwrap();
        for program in programs {
            let file = folder.join(format!("{program}{}", std::env::consts::EXE_SUFFIX));
            // Each folder's copy is its own, so a test can tell which
            // build `current` leads to by what its programs hold.
            std::fs::write(file, folder.file_name().unwrap().as_encoded_bytes()).unwrap();
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
        let runtime = programs(&copy, &["hided", "hide", "hide-agent-hooks", "hide-holder"]);
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
            &[],
            Arc::new(AtomicBool::new(true)),
            None,
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
        assert!(current_runs(&placement.root, &placement.version));
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
            &[],
            Arc::default(),
            None,
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

        assert!(current_runs(&placement.root, "bbbbbbbbbbbbbbbb"));
        assert!(!placement.root.join(&older.version).exists());
        assert!(
            placement
                .root
                .join(hide_kit::CURRENT)
                .join(format!("hide{}", std::env::consts::EXE_SUFFIX))
                .is_file()
        );
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
        let report = run(&placement, KitAction::Apply, "~/.local/bin", Arc::default()).unwrap();
        let report: hide_kit::KitReport = serde_json::from_value(report).unwrap();
        let refusal = report.refused.expect("the pass stops before anything");
        assert_eq!(refusal.cause, hide_kit::RefusalCause::Record);
        assert_eq!(refusal.path.as_deref(), ledger.to_str());
        assert!(current_runs(&placement.root, &older.version));
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
            programs(&root.join(build), &["hide", "hide-agent-hooks"]);
        }
        adopt_build(root, "cccccccccccccccc", "bbbbbbbbbbbbbbbb").unwrap();
        assert!(current_runs(root, "cccccccccccccccc"));
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

    /// A Herdr copy an earlier build left under `root` named `id`, made from
    /// `program`.
    fn herdr_copy(root: &Path, id: &str, program: &Path) -> PathBuf {
        let folder = device_herdr::herdr_dir(root).join(id);
        std::fs::create_dir_all(&folder).unwrap();
        let copy = folder.join(device_herdr::HERDR_PROGRAM);
        std::fs::write(&copy, std::fs::read(program).unwrap()).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(
            &copy,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
        )
        .unwrap();
        copy
    }

    /// Set on a copy of this test binary that stands in for a Herdr server
    /// running a copy: it waits for its parent to end it. A copy of a
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
    fn an_apply_takes_off_the_herdr_copy_and_link_an_earlier_build_left() {
        let dir = tempfile::tempdir().unwrap();
        let placement = placed(dir.path(), "bbbbbbbbbbbbbbbb");
        std::fs::create_dir_all(&placement.home).unwrap();
        herdr_copy(&placement.root, "1111111111111111", Path::new("/bin/sh"));
        let folder = device_herdr::herdr_dir(&placement.root);
        hide_platform::fs::link::replace_link(
            Path::new("1111111111111111"),
            &folder.join(hide_kit::CURRENT),
        )
        .unwrap();
        std::fs::write(folder.join(device_herdr::HANDOFF_RECORD), b"{}").unwrap();

        run_for_projects(
            &placement,
            KitAction::Apply,
            "~/.local/bin",
            &[],
            Arc::new(AtomicBool::new(true)),
            None,
        )
        .unwrap();
        assert!(!folder.exists());
        assert!(current_runs(&placement.root, &placement.version));
    }

    /// A process running the copy `program` of this test binary, once it
    /// has replaced its image. On Windows it runs from `program` with the
    /// `.exe` a program there is started by, in the same copy folder.
    fn run_copy(program: &Path) -> hide_platform::process::OwnedChild {
        #[cfg(windows)]
        let program = &{
            let image = program.with_extension("exe");
            std::fs::copy(program, &image).unwrap();
            image
        };
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

    /// On Windows too, where `process::running` reads each process's image
    /// path: a copy a server runs stays whatever that look could read.
    #[test]
    fn an_older_herdr_copy_stays_while_a_process_runs_it_and_goes_once_none_does() {
        let dir = tempfile::tempdir().unwrap();
        let placement = placed(dir.path(), "bbbbbbbbbbbbbbbb");
        let root = placement.root;
        let test_binary = std::env::current_exe().unwrap();
        let older = herdr_copy(&root, "1111111111111111", &test_binary);
        let unused = herdr_copy(&root, "0000000000000000", &test_binary);
        let mut server = run_copy(&older);

        remove_unused_herdr(&root);
        assert!(older.is_file(), "the copy a server runs stays");
        assert!(!unused.parent().unwrap().exists());

        server.kill_tree().unwrap();
        server.wait().unwrap();
        remove_unused_herdr(&root);
        assert!(!older.parent().unwrap().exists());
    }

    #[test]
    #[cfg(unix)]
    fn removal_takes_the_herdr_copies_no_server_runs() {
        let dir = tempfile::tempdir().unwrap();
        let placement = placed(dir.path(), "bbbbbbbbbbbbbbbb");
        point_current(&placement.root, &placement.version).unwrap();
        herdr_copy(&placement.root, "1111111111111111", Path::new("/bin/sh"));
        hide_platform::fs::link::replace_link(
            Path::new("1111111111111111"),
            &device_herdr::herdr_dir(&placement.root).join(hide_kit::CURRENT),
        )
        .unwrap();
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
        let copy = herdr_copy(
            &placement.root,
            "1111111111111111",
            &std::env::current_exe().unwrap(),
        );
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
