//! Lifecycle and publication only. Parsing, SQLite and projections stay on
//! the owned worker; UI hints replace two flags and never enqueue reads.
use std::sync::{Arc, Mutex};

use super::Runtime;
use crate::{
    handle::ChangeNotifier,
    usage::consumption::Publication,
    usage_ledger::worker::{Activity, Worker},
};

pub(crate) fn spawn_worker(
    runtime: &Arc<Mutex<Runtime>>,
    notifier: ChangeNotifier,
) -> Result<Worker, &'static str> {
    let (path, device, own, ai) = {
        let guard = runtime.lock().map_err(|_| "usage_runtime_unavailable")?;
        let directory = guard
            .state_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or("usage_state_unavailable")?;
        (
            hide_kit::layout::usage_store(directory),
            guard.node.to_string(),
            guard.own_node(),
            Arc::clone(&guard.ai_usage),
        )
    };
    let weak = Arc::downgrade(runtime);
    let worker = Worker::spawn(path, device, own, ai, move |report| {
        let value = Publication::prepare(report, crate::links::now_ms());
        if let Some(runtime) = weak.upgrade()
            && let Ok(mut guard) = runtime.lock()
        {
            let changed = guard.ingest_ledger(value);
            drop(guard);
            if changed {
                notifier.notify();
            }
        }
    })?;
    let mut guard = runtime.lock().map_err(|_| "usage_runtime_unavailable")?;
    guard.usage_ledger_client = Some(worker.client());
    guard.feed_usage_ledger();
    guard.feed_usage_projects();
    Ok(worker)
}

impl Runtime {
    pub(crate) fn install_product_analytics(
        &mut self,
        sink: Arc<dyn crate::product_analytics::Sink>,
    ) {
        sink.set_enabled(self.snapshot.ui_state.product_analytics_enabled);
        self.ai_usage.set_product_analytics(Arc::clone(&sink));
        self.product_analytics = Some(sink);
    }

    /// The sink a router is built with, so what it spends is counted.
    pub(crate) fn ai_usage(&self) -> Arc<dyn hide_ai::AiUsageSink> {
        Arc::clone(&self.ai_usage) as Arc<dyn hide_ai::AiUsageSink>
    }

    pub(super) fn feed_usage_ledger(&self) {
        if let Some(client) = &self.usage_ledger_client {
            client.activity(Activity {
                visible: self.usage_window_visible,
                open: self.usage_popover_open,
            });
        }
    }

    /// Names this machine's projects and their checkouts to the ledger
    /// worker, which prices their last seven days (PRD lane-two-columns
    /// B31). Called where the catalog is replaced; an unchanged list costs
    /// one comparison on the worker's mailbox and wakes nothing.
    pub(super) fn feed_usage_projects(&self) {
        let Some(client) = &self.usage_ledger_client else {
            return;
        };
        let node = self.node.as_str();
        client.projects(
            self.snapshot
                .navigator
                .workspaces
                .iter()
                .filter(|project| {
                    !project.is_home
                        && project.remote_target_id.is_none()
                        && project.device_id == node
                })
                .map(|project| crate::usage_ledger::project_cost::ProjectRoots {
                    workspace_id: project.id.clone(),
                    roots: project
                        .checkouts
                        .iter()
                        .map(|checkout| checkout.path.clone())
                        .collect(),
                })
                .collect(),
        );
    }

    pub(crate) fn ingest_ledger(&mut self, value: Publication) -> bool {
        let own = &mut self.snapshot.usage.nodes[0];
        let mut changed = own.history != value.history || own.projects != value.projects;
        own.history = value.history;
        own.projects = value.projects;
        for (name, consumption) in value.agents {
            if let Some(row) = own.agents.iter_mut().find(|row| row.agent == name) {
                changed |= row.consumption != consumption;
                row.consumption = consumption;
            }
        }
        if changed {
            self.usage_edit += 1;
        }
        changed
    }
}
