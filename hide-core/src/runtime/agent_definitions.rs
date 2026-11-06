//! The operator's agent definition files as the core last read them (PRD
//! engine-bundle-3-core-owns D-17, B21, B24): the daemon hands the first
//! read in and the state exists from then.
//!
//! A read happens off the lock, on the caller's thread (`hided`'s start and
//! `hide agent catalog`) or on a worker when Settings › Agents opens
//! (`agent_definitions_read`), and only its result is swapped in here. There
//! is no watch: an edited file is read on the next ask. A file left out is
//! logged once per reason, and named in `status.agent_definitions` for its
//! row in Settings.

use super::*;
use crate::model::{AgentDefinitionsSnapshot, RejectedDefinitionSnapshot};
use hide_agent_adapter::definition::Catalog;

pub(super) struct AgentDefinitions {
    home: PathBuf,
    pub(super) catalog: Catalog,
    unreadable: Option<String>,
    /// A worker's read is running; a request meanwhile joins it.
    reading: bool,
}

impl AgentDefinitions {
    /// Why the folder could not be listed, when it could not.
    pub(super) fn unreadable(&self) -> Option<&str> {
        self.unreadable.as_deref()
    }
}

impl Runtime {
    /// Swap in a read of `home`'s definition folder, which turns the
    /// definitions on for the rest of this runtime. A folder that could not
    /// be listed leaves the built-in agents and its reason.
    pub(crate) fn settle_agent_definitions(
        &mut self,
        home: PathBuf,
        read: Result<Catalog, String>,
    ) -> bool {
        let (catalog, unreadable) = match read {
            Ok(catalog) => (catalog, None),
            Err(reason) => (Catalog::read(Vec::new()), Some(reason)),
        };
        let before = self.agent_definitions.as_ref();
        for rejected in &catalog.rejected {
            let known = before.is_some_and(|state| state.catalog.rejected.contains(rejected));
            if !known {
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_definitions",
                    "kind": "agent_definitions.file_left_out",
                    "file": rejected.file,
                    "reason": rejected.reason,
                }));
            }
        }
        if let Some(reason) = &unreadable
            && before.is_none_or(|state| state.unreadable.as_ref() != Some(reason))
        {
            crate::diagnostic!(serde_json::json!({
                "component": "agent_definitions",
                "kind": "agent_definitions.folder_unreadable",
                "reason": reason,
            }));
        }
        let snapshot = Some(AgentDefinitionsSnapshot {
            rejected: catalog
                .rejected
                .iter()
                .map(|rejected| RejectedDefinitionSnapshot {
                    file: rejected.file.clone(),
                    reason: rejected.reason.clone(),
                })
                .collect(),
            unreadable: unreadable.clone(),
        });
        self.agent_definitions = Some(AgentDefinitions {
            home,
            catalog,
            unreadable,
            reading: false,
        });
        if self.snapshot.status.agent_definitions == snapshot {
            return false;
        }
        self.snapshot.status.agent_definitions = snapshot;
        true
    }

    /// The engine of this machine's session and the definitions it starts,
    /// recognizes and reads agents by, handed to it off the lock after each
    /// settle; `None` while the definitions are off or no session runs.
    pub(crate) fn agent_definitions_for_engine(
        &self,
    ) -> Option<(Vec<crate::engine::SharedEngine>, std::sync::Arc<Catalog>)> {
        let state = self.agent_definitions.as_ref()?;
        // This machine's engine, and every device's on the core's own
        // engine, start and read agents by the same definitions.
        let engines: Vec<_> = self
            .live
            .iter()
            .map(|live| std::sync::Arc::clone(&live.engine))
            .chain(self.own_device_engines())
            .collect();
        (!engines.is_empty()).then(|| (engines, std::sync::Arc::new(state.catalog.clone())))
    }

    /// Settings › Agents opened: read the folder again on a worker. Nothing
    /// is read while the definitions are off, or while a read runs.
    pub(super) fn request_agent_definitions(&mut self) -> bool {
        let Some(state) = self.agent_definitions.as_mut() else {
            return false;
        };
        let Some(context) = self.worker_context.clone() else {
            return false;
        };
        if state.reading {
            return false;
        }
        state.reading = true;
        let home = state.home.clone();
        let spawned = thread::Builder::new()
            .name("hide-core-agent-definitions".to_owned())
            .spawn(move || {
                let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    hide_kit::agent_definitions::read(&home)
                }))
                .unwrap_or_else(|_| {
                    Err("the definition folder's read ended unexpectedly".to_owned())
                });
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                if settle_and_hand_off(&runtime, home, read) {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            crate::diagnostic!(serde_json::json!({
                "component": "agent_definitions",
                "kind": "agent_definitions.read_not_started",
                "message": error.to_string(),
            }));
            if let Some(state) = self.agent_definitions.as_mut() {
                state.reading = false;
            }
        }
        false
    }
}

/// Lays a read of the definition folder on the core and hands the settled
/// set to every engine off the runtime lock, as every engine call is made;
/// whether the snapshot changed. The one sequence a read takes, whether the
/// daemon handed it in (`Core::set_agent_definitions`) or a worker made it
/// (`request_agent_definitions`). A poisoned lock settles nothing.
pub(crate) fn settle_and_hand_off(
    runtime: &Mutex<Runtime>,
    home: PathBuf,
    read: Result<Catalog, String>,
) -> bool {
    let Ok(mut guard) = runtime.lock() else {
        return false;
    };
    let changed = guard.settle_agent_definitions(home, read);
    let handoff = guard.agent_definitions_for_engine();
    drop(guard);
    if let Some((engines, catalog)) = handoff {
        for engine in engines {
            engine.set_agent_definitions(Arc::clone(&catalog));
        }
    }
    changed
}
