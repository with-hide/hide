//! Retires the agent labels Herdr plugin, `hide.agent-context-labels` (PRD
//! labels-in-hided D-12).
//!
//! Labels are made by hided's core now, so every kit pass takes the plugin
//! off the machine: its Herdr link however it was installed, the watcher it
//! left running, the kit's copy, and the plugin's state folder. A watcher
//! left behind would analyze the same turns again. Herdr's server, the agent
//! sessions, Herdr's per-plugin config folder and the operator's
//! `config.toml` are never touched.
//!
//! A part that could not be taken out is reported and tried again on the next
//! pass; the copy and the state folder stay until the link is gone, because
//! they are how a later pass knows the plugin was ever here.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hide_herdr_client::plugin::{InstalledPlugin, PluginSourceKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use hide_platform::process as platform;

use crate::{KitTarget, process};

pub const LABELS_PLUGIN_ID: &str = "hide.agent-context-labels";

/// The standalone hcoord Herdr plugin from before hcoord became part of the
/// kit (PRD hide-home-layout D-07, D-14). A startup hook it left registered
/// would converge the same daemon label on another build.
pub const HCOORD_PLUGIN_ID: &str = "hide.hcoord";

const HERDR_DEADLINE: Duration = Duration::from_secs(5);
const UNINSTALL_DEADLINE: Duration = Duration::from_secs(20);
const LSOF_DEADLINE: Duration = Duration::from_secs(5);
/// How long a watcher has to end on SIGTERM before it is killed.
const WATCHER_EXIT_DEADLINE: Duration = Duration::from_secs(5);

/// What retiring the plugin did on one machine.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Retirement {
    /// Each part found and taken out, in a few words.
    #[serde(default)]
    pub removed: Vec<String>,
    /// Why a part stayed; the next kit pass tries again.
    #[serde(default)]
    pub failures: Vec<String>,
}

impl Retirement {
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.failures.is_empty()
    }
}

/// Where the kit kept the copy Herdr linked.
pub fn labels_home(home: &Path) -> PathBuf {
    crate::record::kit_state_dir(home)
        .join("plugins")
        .join("agent-context-labels")
}

/// The plugin's own state folder, which the core imports once on this Mac
/// before the kit runs (D-11).
pub fn plugin_state_dir(home: &Path) -> PathBuf {
    home.join(".local/state/hide.agent-context-labels")
}

pub(crate) fn retire(target: &KitTarget) -> Retirement {
    let mut outcome = Retirement::default();
    let copy = labels_home(&target.home);
    let state = plugin_state_dir(&target.home);
    // Nothing of the plugin is left: no Herdr call on an ordinary launch.
    if !copy.exists() && !state.exists() {
        return outcome;
    }
    let unlinked = match unlink(target, LABELS_PLUGIN_ID) {
        Ok(Some(source)) => {
            outcome
                .removed
                .push(format!("Herdr plugin link ({source})"));
            true
        }
        Ok(None) => true,
        Err(reason) => {
            outcome.failures.push(reason);
            false
        }
    };
    // The lock is the only handle on a running watcher: the folders stay
    // until it is free, so a pass that could not stop the watcher leaves
    // the next pass a way to find it.
    let stopped = match stop_watchers(target, &state.join("watcher.lock")) {
        Ok(stopped) => {
            outcome.removed.extend(
                stopped
                    .into_iter()
                    .map(|pid| format!("watcher process {pid}")),
            );
            true
        }
        Err(reason) => {
            outcome.failures.push(reason);
            false
        }
    };
    if unlinked && stopped {
        for (folder, name) in [(&copy, "kit copy"), (&state, "state folder")] {
            match std::fs::remove_dir_all(folder) {
                Ok(()) => outcome.removed.push(name.to_owned()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => outcome.failures.push(format!(
                    "the plugin's {name} could not be removed: {}",
                    error.kind()
                )),
            }
        }
    }
    outcome
}

fn request(target: &KitTarget, method: &str, params: Value) -> Result<Value, String> {
    hide_herdr_client::request_with_timeout(&target.herdr_socket, method, params, HERDR_DEADLINE)
        .map_err(|error| request_failure(method, &error))
}

pub(crate) fn herdr_config_dir(target: &KitTarget) -> Result<std::path::PathBuf, String> {
    hide_platform::host::herdr_config_dir_from(&|name| {
        if name == hide_platform::host::HOME_VARIABLE {
            Some(target.home.as_os_str().to_owned())
        } else {
            std::env::var_os(name)
        }
    })
    .map_err(|error| format!("Herdr's config location cannot be resolved: {error}"))
}

fn request_failure(method: &str, error: &hide_herdr_client::ApiError) -> String {
    // A transport message can carry the socket's path (B20); the
    // remote code is the part a reader needs.
    let reason = match &error {
        hide_herdr_client::ApiError::Remote { code, .. } => code.as_str(),
        hide_herdr_client::ApiError::NotRunning(_) => "not running",
        hide_herdr_client::ApiError::Transport(_) => "transport",
        hide_herdr_client::ApiError::Malformed(_) => "malformed",
        hide_herdr_client::ApiError::TooLarge(_) => "answer too large",
    };
    format!("Herdr on this machine did not answer {method}: {reason}")
}

/// An unavailable server uses the pinned Herdr's durable registry and offline
/// uninstall; an unreadable registry still leaves the step for the next pass.
pub(crate) fn retire_hcoord_plugin(target: &KitTarget) -> Retirement {
    let mut outcome = Retirement::default();
    let unlinked = match hide_herdr_client::request_with_timeout(
        &target.herdr_socket,
        "plugin.list",
        json!({"plugin_id": HCOORD_PLUGIN_ID}),
        HERDR_DEADLINE,
    ) {
        Ok(result) => unlink_list(target, HCOORD_PLUGIN_ID, result),
        Err(
            hide_herdr_client::ApiError::NotRunning(_) | hide_herdr_client::ApiError::Transport(_),
        ) => crate::coordination_retirement::retire_offline_plugin(target),
        Err(error) => Err(request_failure("plugin.list", &error)),
    };
    match unlinked {
        Ok(Some(source)) => outcome
            .removed
            .push(format!("hcoord Herdr plugin link ({source})")),
        Ok(None) => {}
        Err(reason) => outcome.failures.push(reason),
    }
    outcome
}

/// Takes a plugin out of Herdr however it was installed. Returns where it
/// came from, or `None` when Herdr has no such plugin.
fn unlink(target: &KitTarget, plugin_id: &str) -> Result<Option<&'static str>, String> {
    let result = request(target, "plugin.list", json!({ "plugin_id": plugin_id }))?;
    unlink_list(target, plugin_id, result)
}

fn unlink_list(
    target: &KitTarget,
    plugin_id: &str,
    result: Value,
) -> Result<Option<&'static str>, String> {
    let plugins = result
        .get("plugins")
        .cloned()
        .ok_or_else(|| "Herdr's plugin list had no plugins".to_owned())?;
    let plugins: Vec<InstalledPlugin> = serde_json::from_value(plugins)
        .map_err(|error| format!("Herdr's plugin list could not be read: {error}"))?;
    let Some(plugin) = plugins
        .into_iter()
        .find(|plugin| plugin.plugin_id == plugin_id)
    else {
        return Ok(None);
    };
    match plugin.source.kind {
        PluginSourceKind::Github => {
            uninstall_managed(target, plugin_id)?;
            Ok(Some("GitHub"))
        }
        PluginSourceKind::Local => {
            request(target, "plugin.unlink", json!({ "plugin_id": plugin_id }))?;
            Ok(Some("linked folder"))
        }
    }
}

/// A GitHub install is Herdr's managed checkout, which only the `herdr` CLI
/// takes out; unlinking it through the socket would leave the checkout.
pub(crate) fn uninstall_managed(target: &KitTarget, plugin_id: &str) -> Result<(), String> {
    let herdr = target.herdr_bin.as_deref().ok_or_else(|| {
        format!("{plugin_id} is registered and no herdr command was found to remove it")
    })?;
    let mut env = vec![(
        "HERDR_SOCKET_PATH".to_owned(),
        target.herdr_socket.display().to_string(),
    )];
    // Pin the same config root the strict offline read used, including
    // Windows APPDATA or USERPROFILE defaults, in the child's clean env.
    let config = herdr_config_dir(target)?;
    env.push((
        "XDG_CONFIG_HOME".to_owned(),
        config.parent().unwrap().to_string_lossy().into_owned(),
    ));
    let finished = process::run(
        herdr,
        &["plugin", "uninstall", plugin_id],
        &env,
        &target.home,
        UNINSTALL_DEADLINE,
        &target.stop,
    )?;
    if finished.succeeded() {
        Ok(())
    } else {
        // Its stderr can name the operator's folders (B20): the exit code
        // is the reason that reaches the log.
        Err(format!(
            "herdr could not remove {plugin_id}: exit {:?}",
            finished.code
        ))
    }
}

/// Ends every process holding this home's watcher lock. A watcher is found by
/// the lock it holds rather than by its executable, which a swap renames to
/// `.old-<pid>` and a GitHub install keeps elsewhere; the lock is under the
/// target's home, so a test home never reaches the operator's watcher.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn stop_watchers(target: &KitTarget, lock: &Path) -> Result<Vec<u32>, String> {
    if !lock.exists() || !lock_is_held(lock)? {
        return Ok(Vec::new());
    }
    // A daemon started from the Dock may have no `/usr/sbin` on its PATH.
    let lsof = ["/usr/sbin/lsof", "/usr/bin/lsof"]
        .into_iter()
        .map(Path::new)
        .find(|path| path.is_file())
        .unwrap_or(Path::new("lsof"));
    let finished = process::run(
        lsof,
        &["-t", &lock.display().to_string()],
        &[],
        &target.home,
        LSOF_DEADLINE,
        &target.stop,
    )
    .map_err(|reason| format!("the labels watcher could not be found: {reason}"))?;
    let pids: Vec<u32> = finished
        .stdout
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .filter(|pid| *pid > 1 && *pid != std::process::id())
        .collect();
    if pids.is_empty() {
        return Err("the labels watcher holds its lock but lsof named no process".to_owned());
    }
    for pid in &pids {
        // A holder that is already gone is the answer wanted.
        let _ = platform::terminate(*pid);
    }
    let started = Instant::now();
    while pids.iter().any(|pid| platform::is_alive(*pid))
        && started.elapsed() < WATCHER_EXIT_DEADLINE
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    for pid in pids.iter().filter(|pid| platform::is_alive(**pid)) {
        let _ = platform::kill_tree(*pid);
    }
    // Stopped means the lock is free, whatever the signals answered: a pid
    // that survived, or a holder lsof did not name, keeps it.
    let started = Instant::now();
    // A lock file that went with its watcher is as free as an unheld one.
    while lock.exists() && lock_is_held(lock)? {
        if started.elapsed() >= WATCHER_EXIT_DEADLINE {
            return Err(format!(
                "the labels watcher still holds its lock after {} signalled process(es)",
                pids.len()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(pids)
}

/// Whether some process holds the watcher's exclusive lock: taking it for a
/// moment answers without asking lsof on the ordinary pass where none runs.
fn lock_is_held(lock: &Path) -> Result<bool, String> {
    use hide_platform::fs::lock::{Mode, Waited, lock_file};
    let file = std::fs::File::open(lock)
        .map_err(|error| format!("the watcher lock could not be opened: {}", error.kind()))?;
    // Dropping the lock a free file hands back releases it again.
    match lock_file(file, Mode::Exclusive, Duration::ZERO, &|| false) {
        Ok(Waited::Locked(_)) => Ok(false),
        Ok(Waited::TimedOut | Waited::Cancelled) => Ok(true),
        Err(error) => Err(format!(
            "the watcher lock could not be checked: {}",
            error.kind()
        )),
    }
}
