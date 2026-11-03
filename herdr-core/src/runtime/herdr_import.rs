//! What the screen offers for moving this machine's Herdr save into the
//! core's own engine, and the move itself (PRD engine-bundle-4-herdr-removal
//! D-6 to D-8, B2 to B5, B11, B14 to B17).
//!
//! `hided` installs the [`Machine`] that reads the save and the server on
//! its socket and makes the move, only on a core that runs its own engine;
//! the core asks it on a worker of its own, one call at a time, when it is
//! installed and when the operator asks, never under the runtime lock. What
//! it last saw decides `status.herdr_import`. A move that is asked for has
//! the core role start again (D-7), so the step shows it moving until this
//! core stops; one that could not be made is looked at again, and the
//! record's result says why.

use std::sync::{Arc, Mutex, Weak};
use std::thread;

use super::Runtime;
use crate::handle::ChangeNotifier;
use crate::herdr_import::{Look, Machine, Outcome, Seen, Server, Take, Taken};
use crate::model::{HerdrImportRefusedSnapshot, HerdrImportSnapshot};

/// The installed machine and the one call it is making.
#[derive(Default)]
pub(super) struct Import {
    worker: Option<Worker>,
    looking: bool,
    /// The operator asked to look again while a look or a move ran.
    look_again: bool,
    /// A move was asked for and has not answered, or answered that the core
    /// role starts again with it.
    taking: bool,
    /// What the last look saw.
    seen: Option<Seen>,
}

#[derive(Clone)]
struct Worker {
    machine: Arc<dyn Machine>,
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
}

/// What the operator pressed (`herdr_import {action}`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ImportAction {
    /// Stop Hide's Herdr and move its session (D-6).
    Move,
    /// Import a save no Hide Herdr runs, read only (D-8).
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
            machine,
            runtime,
            notifier,
        });
        self.look_at_herdr_save();
    }

    fn look_at_herdr_save(&mut self) {
        let import = &mut self.herdr_import;
        let Some(worker) = import.worker.clone() else {
            return;
        };
        if import.looking || import.taking {
            import.look_again = true;
            return;
        }
        import.looking = true;
        let spawned = thread::Builder::new()
            .name("herdr-core-herdr-import-look".into())
            .spawn(move || {
                let seen = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker.machine.look()
                }))
                .unwrap_or_else(|_| Seen {
                    look: Look::Unreadable("the look stopped unexpectedly".into()),
                    ..Seen::default()
                });
                answer(&worker, |runtime| runtime.ingest_herdr_save(seen));
            });
        if let Err(error) = spawned {
            self.herdr_import.looking = false;
            crate::diagnostic!(serde_json::json!({
                "component": "herdr_import",
                "kind": "herdr_import.look_failed",
                "message": error.to_string(),
            }));
        }
    }

    pub(super) fn ingest_herdr_save(&mut self, seen: Seen) -> bool {
        let import = &mut self.herdr_import;
        import.looking = false;
        import.seen = Some(seen);
        let changed = self.show_herdr_import();
        if std::mem::take(&mut self.herdr_import.look_again) {
            self.look_at_herdr_save();
        }
        changed
    }

    pub(super) fn herdr_import_action(&mut self, action: ImportAction) -> bool {
        if self.herdr_import.worker.is_none() {
            return refused(action, "this core does not run its own engine");
        }
        match action {
            ImportAction::Look => {
                self.look_at_herdr_save();
                false
            }
            ImportAction::Continue => {
                let Some(worker) = self.herdr_import.worker.clone() else {
                    return false;
                };
                if self
                    .herdr_import
                    .seen
                    .as_ref()
                    .is_none_or(|seen| seen.outcome.is_none())
                {
                    return refused(action, "no result waits to be read");
                }
                if let Some(seen) = &mut self.herdr_import.seen {
                    seen.outcome = None;
                }
                let spawned = thread::Builder::new()
                    .name("herdr-core-herdr-import-dismiss".into())
                    .spawn(move || worker.machine.dismiss());
                if let Err(error) = spawned {
                    crate::diagnostic!(serde_json::json!({
                        "component": "herdr_import",
                        "kind": "herdr_import.dismiss_failed",
                        "message": error.to_string(),
                    }));
                }
                self.show_herdr_import()
            }
            ImportAction::Move | ImportAction::Import => {
                if self.herdr_import.taking {
                    return false;
                }
                let (kind, take) = match action {
                    ImportAction::Move => (KIND_MOVE, Take::Move),
                    _ => (KIND_IMPORT, Take::Import),
                };
                let offered = self
                    .snapshot
                    .status
                    .herdr_import
                    .as_ref()
                    .is_some_and(|shown| shown.kind == kind && shown.workspaces > 0);
                if !offered {
                    return refused(action, "no such move is offered");
                }
                self.take_herdr_save(take)
            }
        }
    }

    /// Asks for the move on a worker off the lock and shows it moving.
    fn take_herdr_save(&mut self, take: Take) -> bool {
        let Some(worker) = self.herdr_import.worker.clone() else {
            return false;
        };
        self.herdr_import.taking = true;
        let spawned = thread::Builder::new()
            .name("herdr-core-herdr-import-take".into())
            .spawn(move || {
                // A move that panics still answers, so the step is not held
                // on its spinner.
                let taken = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker.machine.take(take)
                }))
                .unwrap_or_else(|_| Taken::Failed("the move stopped unexpectedly".into()));
                answer(&worker, |runtime| runtime.ingest_herdr_take(take, taken));
            });
        if let Err(error) = spawned {
            return self.ingest_herdr_take(take, Taken::Failed(error.to_string()));
        }
        self.show_herdr_import()
    }

    pub(super) fn ingest_herdr_take(&mut self, take: Take, taken: Taken) -> bool {
        let outcome = match &taken {
            Taken::Restarting => "restarting",
            Taken::NotStopped => "not_stopped",
            Taken::Failed(_) => "failed",
        };
        crate::diagnostic!(serde_json::json!({
            "component": "herdr_import",
            "kind": "herdr_import.take",
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
        self.herdr_import.taking = false;
        let changed = self.show_herdr_import();
        self.herdr_import.look_again = false;
        self.look_at_herdr_save();
        changed
    }

    fn show_herdr_import(&mut self) -> bool {
        let next = self
            .herdr_import
            .seen
            .as_ref()
            .and_then(|seen| shown(seen, self.herdr_import.taking));
        if self.snapshot.status.herdr_import == next {
            return false;
        }
        self.snapshot.status.herdr_import = next;
        true
    }
}

const KIND_MOVE: &str = "move";
const KIND_IMPORT: &str = "import";

/// What the screen shows for `seen`, `None` when there is nothing to show
/// (B11: no save, no question, no line).
fn shown(seen: &Seen, taking: bool) -> Option<HerdrImportSnapshot> {
    let kind = if seen.server == Server::Hide {
        KIND_MOVE
    } else {
        KIND_IMPORT
    };
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
        kind: kind.to_owned(),
        stage: if taking { "moving" } else { "offered" }.to_owned(),
        workspaces: 0,
        tabs: 0,
        agents: 0,
        herdr_running: seen.server != Server::Stopped,
        failure: None,
        reason: None,
        refused,
    };
    match &seen.look {
        Look::Unreadable(reason) => Some(HerdrImportSnapshot {
            stage: "failed".to_owned(),
            failure: Some("unreadable".to_owned()),
            reason: Some(reason.clone()),
            ..base
        }),
        Look::Offer(offer) => {
            let not_stopped =
                !taking && kind == KIND_MOVE && seen.outcome == Some(Outcome::NotStopped);
            let stage = if not_stopped {
                "failed".to_owned()
            } else {
                base.stage.clone()
            };
            Some(HerdrImportSnapshot {
                stage,
                failure: not_stopped.then(|| "not_stopped".to_owned()),
                workspaces: offer.workspaces,
                tabs: offer.tabs,
                agents: offer.agents,
                ..base
            })
        }
        // A move's left-out workspaces wait to be read once nothing else is
        // offered (B11).
        Look::Nothing if !base.refused.is_empty() => Some(HerdrImportSnapshot {
            kind: "result".to_owned(),
            stage: "moved".to_owned(),
            ..base
        }),
        Look::Nothing => None,
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

fn refused(action: ImportAction, reason: &str) -> bool {
    crate::diagnostic!(serde_json::json!({
        "component": "herdr_import",
        "kind": "herdr_import.refused",
        "action": format!("{action:?}"),
        "reason": reason,
    }));
    false
}
