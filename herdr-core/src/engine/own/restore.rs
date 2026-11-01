//! A daemon's engine opening its saved session again (PRD
//! engine-bundle-3-core-owns D-7): the saved session says where every pane
//! goes, and the node's pane records say which panes still run. The engine
//! starts with the saved layout, so a screen draws its tabs at once, and a
//! thread of its own then waits for the node to say which holders it found
//! ([`PaneHolders::recovered`]): a pane whose holder runs is taken as it is,
//! the same pane and the same process; a saved pane whose holder ended (a
//! reboot) or never ran gets a new shell under its own id in its saved
//! folder, or in the home folder with a line saying why when that folder is
//! gone; a holder the saved session does not place (the daemon died before
//! it saved, or the save could not be read) is gathered into a new tab of
//! the workspace its id names, so no running shell is lost or ended. Every
//! id counter is then taken past every id the records hold, so a new pane
//! never meets an old record under its own id.
//!
//! Agent panes open a shell here as well; resuming their sessions is the
//! agent start pull request's.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use hide_node_link::holders::HolderRecovery;

use super::store::{self, Read};
use super::topology::{Pane, Topology};
use super::{Env, OwnEngine};

/// How long the restore waits for the node to finish judging its records.
/// A node judges each record in a few rounds of a bounded budget, so this
/// only runs out when the node is stuck; the saved panes are then marked
/// as not restored rather than started over holders that may still run.
const RECOVERY_WAIT: Duration = Duration::from_secs(120);

/// How long a call that makes a pane waits for the restore: the node's
/// recovery and the restarts after it, [`STARTS_AT_ONCE`] at a time.
pub(super) const BOUND: Duration = Duration::from_secs(180);

/// How many holders a restore starts at once, so a large session coming
/// back after a reboot does not start every shell together.
const STARTS_AT_ONCE: usize = 8;

/// The name of the tab running holders the saved session does not place
/// are gathered in.
const GATHERED_LABEL: &str = "restored";

/// The session saved at `path`, or an empty one when there is none or it
/// cannot be read or does not hold together; a file that cannot be used is
/// moved into `backups` and the reason logged, never deleted (D-6).
pub(super) fn read(path: &Path, backups: &Path) -> Topology {
    match store::read(path, backups) {
        Read::Missing | Read::Unreadable => Topology::default(),
        Read::Saved(saved) => match Topology::from_saved(saved) {
            Ok(topology) => topology,
            Err(why) => {
                store::set_aside_unusable(path, backups, &why);
                Topology::default()
            }
        },
    }
}

/// Starts the restore of the panes `saved` names on a thread of its own.
pub(super) fn start(engine: OwnEngine<'static>, saved: BTreeSet<String>) -> Result<(), String> {
    std::thread::Builder::new()
        .name("hide-own-engine-restore".into())
        .spawn(move || {
            let gate = engine.restore_gate();
            engine.restore(&saved);
            drop(engine);
            gate.finish();
        })
        .map(|_| ())
        .map_err(|error| format!("the session restore thread could not start: {error}"))
}

/// One saved pane to start again: its id and the folder it was saved in.
struct Restart {
    pane: String,
    cwd: String,
}

impl OwnEngine<'static> {
    fn restore(&self, saved: &BTreeSet<String>) {
        let recovery = match self.shared.holders.recovered(RECOVERY_WAIT) {
            Ok(recovery) => recovery,
            Err(error) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "engine.restore_unfinished",
                    "message": error.to_string(),
                }));
                self.mark_unrestored(
                    saved,
                    &format!("the node did not say which panes still run: {error}"),
                );
                return;
            }
        };
        self.adopt(saved, &recovery);
    }

    /// Lays the panes `saved` names out around what `recovery` found.
    fn adopt(&self, saved: &BTreeSet<String>, recovery: &HolderRecovery) {
        let present: BTreeSet<&str> = recovery.present.iter().map(String::as_str).collect();
        let home = self.home().ok();
        let (restarts, orphans) = {
            let mut topology = self.topology();
            topology.count_past(
                recovery
                    .present
                    .iter()
                    .chain(&recovery.ended)
                    .map(String::as_str),
            );
            let placed: BTreeSet<String> = topology.panes().map(|(id, _)| id.to_owned()).collect();
            let restarts: Vec<Restart> = topology
                .panes()
                .filter(|(id, _)| saved.contains(*id) && !present.contains(id))
                .map(|(id, pane)| Restart {
                    pane: id.to_owned(),
                    cwd: pane.cwd.clone(),
                })
                .collect();
            let orphans: Vec<String> = recovery
                .present
                .iter()
                .filter(|id| !placed.contains(*id))
                .cloned()
                .collect();
            (restarts, orphans)
        };
        let failures = self.start_restarts(&restarts, home.as_deref());
        let closed: Vec<String> = {
            let mut topology = self.topology();
            let failed: BTreeSet<&str> = failures.iter().map(|(pane, _)| pane.as_str()).collect();
            // A pane closed while its shell started has nowhere to be.
            let closed = restarts
                .iter()
                .filter(|restart| {
                    !failed.contains(restart.pane.as_str()) && topology.pane(&restart.pane).is_err()
                })
                .map(|restart| restart.pane.clone())
                .collect();
            for (pane, why) in &failures {
                if let Ok(found) = topology.pane_mut(pane) {
                    found.failed = Some(why.clone());
                }
            }
            if !orphans.is_empty() {
                let cwd = home
                    .as_deref()
                    .and_then(|home| hide_platform::path::to_wire(home).ok())
                    .unwrap_or_default();
                let gathered = orphans
                    .iter()
                    .map(|id| (id.clone(), Pane::new(&cwd, None)))
                    .collect();
                let tabs = topology.gather(gathered, GATHERED_LABEL);
                crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "engine.restore_gathered",
                    "panes": orphans,
                    "tabs": tabs,
                }));
            }
            closed
        };
        let _ = self.end_panes(&closed);
        crate::diagnostic!(serde_json::json!({
            "component": "own_engine",
            "kind": "engine.restored",
            "adopted": present.len().saturating_sub(orphans.len()),
            "started": restarts.len(),
            "gathered": orphans.len(),
        }));
        self.changed();
    }

    /// Starts every pane of `restarts`, [`STARTS_AT_ONCE`] at a time, each
    /// in its folder or in `home` when the folder is gone: the panes that
    /// could not start, with why.
    fn start_restarts(&self, restarts: &[Restart], home: Option<&Path>) -> Vec<(String, String)> {
        let next = Mutex::new(restarts.iter());
        let failures = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..STARTS_AT_ONCE.min(restarts.len()) {
                scope.spawn(|| {
                    loop {
                        let Some(restart) = next
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .next()
                        else {
                            break;
                        };
                        let (cwd, notice) = self.start_folder(&restart.cwd, home);
                        if let Err(error) = self.start_pane(
                            &restart.pane,
                            None,
                            &cwd,
                            Env::new(),
                            notice.as_deref(),
                        ) {
                            crate::diagnostic!(serde_json::json!({
                                "component": "own_engine",
                                "kind": "engine.restore_start_failed",
                                "pane_id": restart.pane,
                                "message": error.to_string(),
                            }));
                            failures
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .push((restart.pane.clone(), error.to_string()));
                        }
                    }
                });
            }
        });
        failures
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Where a saved pane whose folder was `cwd` starts again: there, or in
    /// `home` with a line saying why when the node finds no folder there. A
    /// node that cannot answer reads as a folder that is gone
    /// (`node_access::is_directory`), so the shell opens where it can.
    fn start_folder(&self, cwd: &str, home: Option<&Path>) -> (String, Option<String>) {
        let exists = self
            .node()
            .is_ok_and(|node| crate::node_access::is_directory(node.as_ref(), cwd));
        match home.and_then(|home| hide_platform::path::to_wire(home).ok()) {
            Some(home_cwd) if !exists => (
                home_cwd,
                Some(format!(
                    "hide: the folder {cwd} is gone, so this shell opened in your home folder."
                )),
            ),
            _ => (cwd.to_owned(), None),
        }
    }

    /// Every saved pane, when the node never said which run: none is
    /// started over a holder that may still run.
    fn mark_unrestored(&self, saved: &BTreeSet<String>, why: &str) {
        {
            let mut topology = self.topology();
            for pane in saved {
                if let Ok(found) = topology.pane_mut(pane) {
                    found.failed = Some(why.to_owned());
                }
            }
        }
        self.changed();
    }
}
