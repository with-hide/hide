//! A paired machine's core stays reachable with no window open (PRD
//! device-connect-iroh D-41, B30): once the machine has finished its first
//! pairing, either way, the account's login item runs its core from a build
//! in the install root (`hide_kit::layout::core_root`), and once the last
//! paired machine is gone the item and its marker go. A machine that never
//! pairs gets nothing.
//!
//! [`reconcile`] is the one decision: it reads the folder's peer records
//! and puts the item in or takes it out. The daemon runs it once at its
//! start, and the pairing after every change to the machines this one is
//! paired with ([`Follower`]); a paired core also never ends for idle
//! ([`paired`], `server::watch_idle`). Paired means a finished pairing,
//! never one still waiting (`peer_host::pairing_confirmed`).
//!
//! A core moved here before 1.2.0 is kept the same way while the machine it
//! was moved off still dials it as a node (D-46): that
//! machine can only split from it by reaching it, so its login item is not
//! taken out before the two are paired.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use hide_platform::user_agents::{Linger, UserAgents};
use serde::Serialize;
use serde_json::json;

/// Whether the folder's core is kept on, the one answer the item follows:
/// it has finished a pairing (`peer_host::pairing_confirmed`), or it is a
/// core moved here before 1.2.0 that a node still dials
/// (`hide_core::dialing_nodes`).
pub fn paired(state_dir: &Path) -> Result<bool, String> {
    if crate::peer_host::pairing_confirmed(state_dir)
        .map_err(|error| format!("the peer records: {error}"))?
    {
        return Ok(true);
    }
    hide_core::dialing_nodes(&state_dir.join(hide_core::node_migration::CORE_STATE))
        .map(|nodes| !nodes.is_empty())
}

/// What [`reconcile`] found or did.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Outcome {
    /// The item was put in and runs `program`. `warning` is what the
    /// operator can do for it, a line the device's details show (B30).
    Installed {
        program: PathBuf,
        warning: Option<Warning>,
    },
    /// Paired, and the item is in place.
    Kept,
    /// No longer paired: the item and its marker were taken out.
    Removed,
    /// Not paired, and no item.
    Nothing,
    /// Paired, and no login item is kept: this system has none, or HOME is
    /// not the account's own home ([`agents_for`]). `reason` says which.
    Unsupported { reason: String },
}

/// What stops the item from doing all it should, and the command that fixes
/// it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Warning {
    /// `linger`: the core stops with the account's last session (Linux).
    pub kind: String,
    pub command: String,
}

/// The core that asks: the program it runs, and whether its login item
/// started it, in which case the item is only forgotten on removal, since
/// unloading it would stop the asking core midway.
pub struct Asker<'a> {
    pub program: &'a Path,
    pub under_item: bool,
}

/// Puts the login item in when the folder is paired and it is not there,
/// takes it and its marker out when the folder is not paired and it is.
pub fn reconcile(
    agents: &UserAgents,
    home: &Path,
    state_dir: &Path,
    asker: &Asker<'_>,
    install: impl FnOnce(&Path) -> Result<PathBuf, String>,
) -> Result<Outcome, String> {
    let paired = paired(state_dir)?;
    if let Err(error) = agents.supported() {
        return Ok(if paired {
            Outcome::Unsupported {
                reason: error.to_string(),
            }
        } else {
            Outcome::Nothing
        });
    }
    let installed = crate::login_item::installed(agents, home, state_dir)?;
    match (paired, installed) {
        (true, true) => {
            // Kept only while the item can run the core: one the system no
            // longer holds is loaded again.
            crate::login_item::hold(agents, home, state_dir)?;
            Ok(Outcome::Kept)
        }
        (false, false) => Ok(Outcome::Nothing),
        (false, true) => {
            if asker.under_item {
                crate::login_item::forget(agents, home, state_dir)?;
            } else {
                crate::login_item::remove(agents, home, state_dir)?;
            }
            Ok(Outcome::Removed)
        }
        (true, false) => {
            // Asked before a build is copied: a Mac with no one logged in
            // would copy one on every change only to refuse the item.
            if agents
                .login(home, &AtomicBool::new(false))
                .map_err(|error| format!("the login session: {error}"))?
                == hide_platform::user_agents::Login::LoggedOut
            {
                return Err(
                    "no GUI session runs for this account: the login item waits for a login"
                        .to_owned(),
                );
            }
            let program = install(&hide_kit::layout::core_root(home))?;
            // The asking core goes on; the item's core waits on the
            // folder's lock until it exits.
            crate::login_item::adopt(agents, home, state_dir, &program)?;
            let warning = match agents.linger(home, &AtomicBool::new(false)) {
                Ok(Linger::On) => None,
                Ok(Linger::NeedsAdministrator { command }) => Some(Warning {
                    kind: "linger".to_owned(),
                    command,
                }),
                Err(error) if error.kind() == std::io::ErrorKind::Unsupported => None,
                Err(error) => return Err(format!("linger: {error}")),
            };
            Ok(Outcome::Installed { program, warning })
        }
    }
}

/// The account's login items for `home`: the system's only when `home` is
/// the account's own home as the system names it, and none for any other
/// HOME (`UserAgents::of_home`). The system's items are the account's
/// whatever HOME says, so a test's HOME, or an isolated candidate's, would
/// otherwise load its core into the account's real ones.
pub fn agents_for(home: &Path) -> UserAgents {
    UserAgents::of_home(home)
}

/// The app's package whose core this process is, when it runs from a copy
/// of a build in the account's install root (`~/.hide/core`, where a paired
/// machine's login item runs its core) on a state folder the app opened
/// with its package still there (`hide_kit::app`). That core is the app's
/// own, run from a copy so the app can be replaced under it: it answers as
/// the app's installation, so the panes the app's own daemon started keep
/// reaching it and the ones it starts reach the app's daemon after it, and
/// it installs the app's kit. Read once, at the core's start
/// ([`adopt_app`]); `None` for any other program or folder.
pub fn app_package() -> Option<&'static Path> {
    APP_PACKAGE.get().and_then(Option::as_deref)
}

static APP_PACKAGE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();

/// Reads [`app_package`] for the core about to start on `state_dir` and, for
/// the app's, has this process answer as the app's installation
/// (`hide_host::kit::answer_as`) before anything starts a pane. A core run
/// from a build in the install root answers the panes that name either of
/// its two names whichever it starts panes under
/// (`hide_host::kit::also_answer_for`): the app's package can come or go
/// between two starts of the same login item, and the panes, which outlive
/// a start, keep the name they were started under. A note that cannot be
/// read leaves the core its own installation and is logged.
pub fn adopt_app(home: &Path, state_dir: &Path) {
    let Some(program) = std::env::current_exe()
        .ok()
        .filter(|program| in_core_root(program, home))
    else {
        let _ = APP_PACKAGE.set(None);
        return;
    };
    let package = noted_package(state_dir);
    if let Some(own) = hide_host::kit::pane_install(&program) {
        hide_host::kit::also_answer_for(vec![
            own.id,
            hide_agent_hooks::route::APP_INSTALL.to_owned(),
        ]);
    }
    if let Some(package) = &package {
        let cli = package.join(format!("hide{}", std::env::consts::EXE_SUFFIX));
        hide_host::kit::answer_as(Some(hide_host::kit::PaneInstall {
            id: hide_agent_hooks::route::APP_INSTALL.to_owned(),
            cli,
            path_first: None,
        }));
    }
    let _ = APP_PACKAGE.set(package);
}

/// Whether `program` is a build in `home`'s install root, where a paired
/// machine's login item runs its core from.
fn in_core_root(program: &Path, home: &Path) -> bool {
    let canonical = hide_platform::fs::identity::canonical;
    let (Ok(root), Ok(program)) = (
        canonical(&hide_kit::layout::core_root(home)),
        canonical(program),
    ) else {
        return false;
    };
    program.parent().is_some_and(|build| {
        build.parent() == Some(root.as_path())
            && build
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(hide_kit::is_build_name)
    })
}

/// The package the app's note on `state_dir` names, when it is still there.
fn noted_package(state_dir: &Path) -> Option<PathBuf> {
    match hide_kit::app::package(state_dir) {
        Ok(package) => package,
        Err(message) => {
            hide_core::diagnostic!(serde_json::json!({
                "component": "always_on",
                "kind": "always_on.app_note_unread",
                "message": message,
            }));
            None
        }
    }
}

/// This build, put in a version folder of `root` from the package beside
/// `program` (`HelperPackages::install_here`), as an update puts its build.
pub fn install_this_build(program: &Path, root: &Path) -> Result<PathBuf, String> {
    hide_node::ssh::host::HelperPackages::new(
        program.parent().map(Path::to_path_buf),
        crate::cli::RELEASE,
    )
    .install_here(root)
}

/// The daemon's login item kept to its folder's peer records: the daemon
/// asks at its start, and the pairing after every change to the machines
/// this one is paired with (`pairing::Pairing::paired_changed`).
#[derive(Clone)]
pub struct Follower {
    home: PathBuf,
    state_dir: PathBuf,
    /// Whether the login item started this core ([`Asker::under_item`]).
    under_item: bool,
}

impl Follower {
    pub fn new(home: PathBuf, state_dir: PathBuf, under_item: bool) -> Self {
        Self {
            home,
            state_dir,
            under_item,
        }
    }

    /// [`reconcile`] on a thread of its own, since putting a build in place
    /// copies it, one at a time in this process; the outcome goes to the
    /// log.
    pub fn peers_changed(&self) {
        let Self {
            home,
            state_dir,
            under_item,
        } = self.clone();
        let spawned = std::thread::Builder::new()
            .name("always-on".to_owned())
            .spawn(move || {
                let _one = RECONCILING
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let outcome = std::env::current_exe()
                    .map_err(|error| format!("this hided has no path: {error}"))
                    .and_then(|program| {
                        let asker = Asker {
                            program: &program,
                            under_item,
                        };
                        reconcile(&agents_for(&home), &home, &state_dir, &asker, |root| {
                            install_this_build(asker.program, root)
                        })
                    });
                // A failure that holds (no systemd, a Mac with no one logged
                // in) is logged once, not at every connection change.
                let mut last = LAST_FAILURE
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match outcome {
                    Ok(Outcome::Kept | Outcome::Nothing) => *last = None,
                    // Why a paired machine keeps no item holds as a failure
                    // does, and is logged once.
                    Ok(Outcome::Unsupported { reason }) => {
                        if last.as_deref() != Some(reason.as_str()) {
                            hide_core::diagnostic!(json!({
                                "component": "always_on",
                                "kind": "always_on.declined",
                                "reason": reason,
                            }));
                            *last = Some(reason);
                        }
                    }
                    Ok(outcome) => {
                        *last = None;
                        hide_core::diagnostic!(json!({
                            "component": "always_on",
                            "kind": "always_on.reconciled",
                            "outcome": outcome,
                        }));
                    }
                    Err(message) => {
                        if last.as_deref() != Some(message.as_str()) {
                            hide_core::diagnostic!(json!({
                                "component": "always_on",
                                "kind": "always_on.failed",
                                "message": message,
                            }));
                            *last = Some(message);
                        }
                    }
                }
            });
        if let Err(error) = spawned {
            hide_core::diagnostic!(json!({
                "component": "always_on",
                "kind": "always_on.failed",
                "message": error.to_string(),
            }));
        }
    }
}

/// One reconcile at a time in this process: two would both install.
static RECONCILING: Mutex<()> = Mutex::new(());
/// The last failure logged, so one that holds is logged once.
static LAST_FAILURE: Mutex<Option<String>> = Mutex::new(None);

pub mod cli;

#[cfg(test)]
mod tests;
