//! What the screen offers for importing a machine's Herdr save into the
//! core's own engine, and the import itself (PRD engine-bundle-4-herdr-removal
//! D-7, D-8, B11, B15 to B18). An import stops no Herdr.
//!
//! `hided` installs the [`Machine`] that reads this machine's save and the
//! server on its socket, only on a core that runs its own engine; a device of
//! that engine gets one of its own on each link its engine opens on
//! (`herdr_import::device`), whose node reads it for the core. The core asks each on a worker of its own, one call at a time per
//! machine, never under the runtime lock. What each last saw decides
//! `status.herdr_import` and `status.herdr_import_devices`. A move that is
//! asked for has the core role start again (D-7), so the step shows it
//! moving until this core stops; one that could not be made is looked at
//! again, and the record's result says why.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};
use std::thread;

use super::Runtime;
use crate::handle::ChangeNotifier;
use crate::herdr_import::{Look, Machine, Outcome, Seen, Server, Take, Taken};
use crate::model::{HerdrImportRefusedSnapshot, HerdrImportSnapshot};

/// Whose save: this machine's (`None`) or a device's, by its id.
type Place = Option<String>;

/// The installed machines and the one call each is making.
#[derive(Default)]
pub(super) struct Import {
    worker: Option<Worker>,
    places: BTreeMap<Place, PlaceImport>,
}

struct PlaceImport {
    machine: Arc<dyn Machine>,
    looking: bool,
    /// The operator asked to look again while a look or a move ran.
    look_again: bool,
    /// A move was asked for and has not answered, or answered that the core
    /// role starts again with it.
    taking: bool,
    /// What the last look saw.
    seen: Option<Seen>,
}

impl PlaceImport {
    fn new(machine: Arc<dyn Machine>, seen: Option<Seen>) -> Self {
        Self {
            machine,
            looking: false,
            look_again: false,
            taking: false,
            seen,
        }
    }
}

#[derive(Clone)]
struct Worker {
    /// This machine's, whose restart starts the core role again.
    local: Arc<dyn Machine>,
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
}

/// What the operator pressed (`herdr_import {action, device}`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ImportAction {
    /// An earlier build's move question. This build moves nothing: the press
    /// is refused, and the save is only imported.
    Move,
    /// Import the save, read only: a Herdr that runs on is left untouched and
    /// its conversations wait (D-8).
    Import,
    /// Read the save again: the retry of a save that could not be read.
    Look,
    /// The last move's result was read.
    Continue,
}

impl Runtime {
    pub(crate) fn install_herdr_import(
        &mut self,
        machine: Arc<dyn Machine>,
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
    ) {
        self.herdr_import.worker = Some(Worker {
            local: Arc::clone(&machine),
            runtime,
            notifier,
        });
        self.herdr_import
            .places
            .insert(None, PlaceImport::new(machine, None));
        self.look_at_herdr_save(&None);
    }

    /// This machine's mover, which a device's restarts the core role with;
    /// `None` on a core that does not run its own engine.
    pub(super) fn herdr_import_local(&self) -> Option<Arc<dyn Machine>> {
        self.herdr_import
            .worker
            .as_ref()
            .map(|worker| Arc::clone(&worker.local))
    }

    /// The device's engine opened on a link whose look at its save saw
    /// `seen`: `machine` answers for it until that link ends (B18).
    pub(super) fn install_device_herdr_import(
        &mut self,
        device: &str,
        machine: Arc<dyn Machine>,
        seen: Seen,
    ) -> bool {
        self.herdr_import.places.insert(
            Some(device.to_owned()),
            PlaceImport::new(machine, Some(seen)),
        );
        self.show_herdr_import()
    }

    /// The device's link ended, and its mover with it; a move it asked for
    /// stands in its record.
    pub(super) fn retire_device_herdr_import(&mut self, device: &str) -> bool {
        self.herdr_import
            .places
            .remove(&Some(device.to_owned()))
            .is_some()
            && self.show_herdr_import()
    }

    fn look_at_herdr_save(&mut self, place: &Place) {
        let Some(worker) = self.herdr_import.worker.clone() else {
            return;
        };
        let Some(entry) = self.herdr_import.places.get_mut(place) else {
            return;
        };
        if entry.looking || entry.taking {
            entry.look_again = true;
            return;
        }
        entry.looking = true;
        let machine = Arc::clone(&entry.machine);
        let key = place.clone();
        let spawned = thread::Builder::new()
            .name("hide-core-herdr-import-look".into())
            .spawn(move || {
                let seen =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| machine.look()))
                        .unwrap_or_else(|_| Seen {
                            look: Look::Unreadable("the look stopped unexpectedly".into()),
                            ..Seen::default()
                        });
                answer(&worker, |runtime| {
                    runtime.ingest_herdr_save_of(&key, &machine, seen)
                });
            });
        if let Err(error) = spawned {
            if let Some(entry) = self.herdr_import.places.get_mut(place) {
                entry.looking = false;
            }
            crate::diagnostic!(serde_json::json!({
                "component": "herdr_import",
                "kind": "herdr_import.look_failed",
                "device": place,
                "message": error.to_string(),
            }));
        }
    }

    /// The place's look answered, unless the mover that looked was let go
    /// since.
    fn ingest_herdr_save_of(
        &mut self,
        place: &Place,
        machine: &Arc<dyn Machine>,
        seen: Seen,
    ) -> bool {
        let Some(entry) = self
            .herdr_import
            .places
            .get_mut(place)
            .filter(|entry| Arc::ptr_eq(&entry.machine, machine))
        else {
            return false;
        };
        entry.looking = false;
        entry.seen = Some(seen);
        let again = std::mem::take(&mut entry.look_again);
        let changed = self.show_herdr_import();
        if again {
            self.look_at_herdr_save(place);
        }
        changed
    }

    pub(super) fn herdr_import_action(
        &mut self,
        action: ImportAction,
        device: Option<String>,
    ) -> bool {
        if self.herdr_import.worker.is_none() {
            return refused(action, &device, "this core does not run its own engine");
        }
        let Some(entry) = self.herdr_import.places.get_mut(&device) else {
            return refused(action, &device, "no such machine offers a move");
        };
        match action {
            ImportAction::Look => {
                self.look_at_herdr_save(&device);
                false
            }
            ImportAction::Continue => {
                if entry
                    .seen
                    .as_ref()
                    .is_none_or(|seen| seen.outcome.is_none())
                {
                    return refused(action, &device, "no result waits to be read");
                }
                if let Some(seen) = &mut entry.seen {
                    seen.outcome = None;
                }
                let machine = Arc::clone(&entry.machine);
                let spawned = thread::Builder::new()
                    .name("hide-core-herdr-import-dismiss".into())
                    .spawn(move || machine.dismiss());
                if let Err(error) = spawned {
                    crate::diagnostic!(serde_json::json!({
                        "component": "herdr_import",
                        "kind": "herdr_import.dismiss_failed",
                        "device": device,
                        "message": error.to_string(),
                    }));
                }
                self.show_herdr_import()
            }
            ImportAction::Move => refused(action, &device, "a Herdr save is imported, not moved"),
            ImportAction::Import => {
                if entry.taking {
                    return false;
                }
                let offered = entry
                    .seen
                    .as_ref()
                    .and_then(|seen| shown(seen, false))
                    .is_some_and(|shown| shown.workspaces > 0 || shown.waiting > 0);
                if !offered {
                    return refused(action, &device, "no such import is offered");
                }
                self.take_herdr_save(&device, Take::Import)
            }
        }
    }

    /// Asks for the move on a worker off the lock and shows it moving.
    fn take_herdr_save(&mut self, place: &Place, take: Take) -> bool {
        let Some(worker) = self.herdr_import.worker.clone() else {
            return false;
        };
        let Some(entry) = self.herdr_import.places.get_mut(place) else {
            return false;
        };
        entry.taking = true;
        let machine = Arc::clone(&entry.machine);
        let key = place.clone();
        let spawned = thread::Builder::new()
            .name("hide-core-herdr-import-take".into())
            .spawn(move || {
                // A move that panics still answers, so the step is not held
                // on its spinner.
                let taken =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| machine.take(take)))
                        .unwrap_or_else(|_| Taken::Failed("the move stopped unexpectedly".into()));
                answer(&worker, |runtime| {
                    runtime.ingest_herdr_take_of(&key, &machine, take, taken)
                });
            });
        if let Err(error) = spawned {
            let machine = Arc::clone(&self.herdr_import.places[place].machine);
            return self.ingest_herdr_take_of(
                place,
                &machine,
                take,
                Taken::Failed(error.to_string()),
            );
        }
        self.show_herdr_import()
    }

    fn ingest_herdr_take_of(
        &mut self,
        place: &Place,
        machine: &Arc<dyn Machine>,
        take: Take,
        taken: Taken,
    ) -> bool {
        let outcome = match &taken {
            Taken::Restarting => "restarting",
            Taken::Failed(_) => "failed",
        };
        crate::diagnostic!(serde_json::json!({
            "component": "herdr_import",
            "kind": "herdr_import.take",
            "device": place,
            "take": format!("{take:?}"),
            "outcome": outcome,
            "message": match &taken {
                Taken::Failed(reason) => Some(reason.as_str()),
                _ => None,
            },
        }));
        // The core role starts again with the move: the step moves until
        // this core stops.
        if taken == Taken::Restarting {
            return false;
        }
        let Some(entry) = self
            .herdr_import
            .places
            .get_mut(place)
            .filter(|entry| Arc::ptr_eq(&entry.machine, machine))
        else {
            return false;
        };
        entry.taking = false;
        entry.look_again = false;
        let changed = self.show_herdr_import();
        self.look_at_herdr_save(place);
        changed
    }

    fn show_herdr_import(&mut self) -> bool {
        let mut here = None;
        let mut devices = Vec::new();
        for (place, entry) in &self.herdr_import.places {
            let Some(mut shown) = entry
                .seen
                .as_ref()
                .and_then(|seen| shown(seen, entry.taking))
            else {
                continue;
            };
            match place {
                None => here = Some(shown),
                Some(device) => {
                    shown.device = Some(device.clone());
                    devices.push(shown);
                }
            }
        }
        let status = &mut self.snapshot.status;
        if status.herdr_import == here && status.herdr_import_devices == devices {
            return false;
        }
        status.herdr_import = here;
        status.herdr_import_devices = devices;
        true
    }
}

#[cfg(test)]
impl Runtime {
    /// This machine's look answered.
    pub(super) fn ingest_herdr_save(&mut self, seen: Seen) -> bool {
        let machine = Arc::clone(&self.herdr_import.places[&None].machine);
        self.ingest_herdr_save_of(&None, &machine, seen)
    }
}

const KIND_IMPORT: &str = "import";

/// What the screen shows for `seen`, `None` when there is nothing to show
/// (B11: no save, no question, no line).
fn shown(seen: &Seen, taking: bool) -> Option<HerdrImportSnapshot> {
    let refused = match &seen.outcome {
        Some(Outcome::Moved { refused, .. }) => refused
            .iter()
            .map(|refused| HerdrImportRefusedSnapshot {
                label: refused.label.clone(),
                reason: refused.reason.clone(),
            })
            .collect(),
        _ => Vec::new(),
    };
    let base = HerdrImportSnapshot {
        kind: KIND_IMPORT.to_owned(),
        stage: if taking { "moving" } else { "offered" }.to_owned(),
        workspaces: 0,
        tabs: 0,
        agents: 0,
        waiting: 0,
        herdr_running: seen.server != Server::Stopped,
        failure: None,
        reason: None,
        refused,
        device: None,
    };
    match &seen.look {
        Look::Unreadable(reason) => Some(HerdrImportSnapshot {
            stage: "failed".to_owned(),
            failure: Some("unreadable".to_owned()),
            reason: Some(reason.clone()),
            ..base
        }),
        // Waiting conversations resume only once their Herdr no longer runs,
        // so while another Herdr runs on, a save with nothing else is no offer.
        Look::Offer(offer) if offer.workspaces > 0 || seen.server != Server::Other => {
            Some(HerdrImportSnapshot {
                workspaces: offer.workspaces,
                tabs: offer.tabs,
                agents: offer.agents,
                waiting: offer.waiting,
                ..base
            })
        }
        // An import's left-out workspaces wait to be read once nothing else is
        // offered (B11).
        Look::Nothing | Look::Offer(_) if !base.refused.is_empty() => Some(HerdrImportSnapshot {
            kind: "result".to_owned(),
            stage: "moved".to_owned(),
            ..base
        }),
        Look::Nothing | Look::Offer(_) => None,
    }
}

/// Hands a worker's result to the runtime and announces it.
fn answer(worker: &Worker, ingest: impl FnOnce(&mut Runtime) -> bool) {
    let Some(runtime) = worker.runtime.upgrade() else {
        return;
    };
    let changed = match runtime.lock() {
        Ok(mut guard) => ingest(&mut guard),
        Err(poisoned) => ingest(&mut poisoned.into_inner()),
    };
    drop(runtime);
    if changed {
        worker.notifier.notify();
    }
}

fn refused(action: ImportAction, device: &Place, reason: &str) -> bool {
    crate::diagnostic!(serde_json::json!({
        "component": "herdr_import",
        "kind": "herdr_import.refused",
        "action": format!("{action:?}"),
        "device": device,
        "reason": reason,
    }));
    false
}
