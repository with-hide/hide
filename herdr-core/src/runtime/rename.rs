//! A rename keeps the requested text separate from the committed tab name.
use super::*;
use crate::model::TabRenameSnapshot;

pub(super) struct PendingRename {
    receipt: TabRenameSnapshot,
    previous: String,
    target: Option<String>,
    connection: u64,
    confirmed: bool,
}

impl PendingRename {
    fn apply(&mut self, tab: &mut TabSnapshot) {
        if tab.id.as_deref() != Some(&self.receipt.tab_id) || self.confirmed {
            return;
        }
        let raw = if self.receipt.phase == "succeeded" {
            &self.receipt.label
        } else {
            &self.previous
        };
        tab.naming.raw = raw.clone();
        tab.label = Some(crate::model::display_tab_label(
            raw,
            0,
            Some(&tab.naming.automatic),
            None,
        ));
    }
}

pub(super) fn refresh_strip_labels(checkout: &mut CheckoutSnapshot) {
    for entry in &mut checkout.strip {
        if entry.kind == StripTabKind::Herdr
            && let Some(tab) = checkout
                .tabs
                .iter()
                .find(|tab| tab.id.as_deref() == Some(&entry.source_id))
        {
            entry.label = tab.label.clone().unwrap_or_default();
        }
    }
}

impl Runtime {
    pub(super) fn apply_tab_rename(&mut self, tab: &mut TabSnapshot) {
        if let Some(pending) = &mut self.pending_tab_rename {
            if tab.id.as_deref() == Some(&pending.receipt.tab_id)
                && pending.receipt.phase == "succeeded"
                && tab.naming.raw == pending.receipt.label
            {
                pending.confirmed = true;
            }
            pending.apply(tab);
        }
        tab.naming.by_operator = tab.naming.named_by(
            tab.id.as_deref(),
            &self.snapshot.ui_state.operator_tab_names,
        );
    }

    /// Remembers the name the operator gave a tab of this machine, through
    /// Rename or a reopen of a tab they had named, or forgets it when
    /// `name` is none (a Rename that cleared it). Past the limit, the names
    /// of tabs that are gone leave first; with no room left the name is not
    /// remembered and the tab behaves as unnamed, which the log records.
    pub(super) fn remember_operator_tab_name(&mut self, tab_id: &str, name: Option<&str>) {
        let names = &self.snapshot.ui_state.operator_tab_names;
        if names.get(tab_id).map(String::as_str) == name {
            return;
        }
        if let Some(name) = name {
            if !names.contains_key(tab_id) && names.len() >= crate::model::OPERATOR_TAB_NAME_LIMIT {
                let live = self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .filter(|workspace| workspace.remote_target_id.is_none())
                    .flat_map(|workspace| &workspace.checkouts)
                    .flat_map(|checkout| &checkout.tabs)
                    .filter_map(|tab| tab.id.clone())
                    .collect::<HashSet<_>>();
                self.snapshot
                    .ui_state
                    .operator_tab_names
                    .retain(|id, _| live.contains(id));
            }
            let names = &mut self.snapshot.ui_state.operator_tab_names;
            if !names.contains_key(tab_id) && names.len() >= crate::model::OPERATOR_TAB_NAME_LIMIT {
                self.push_diagnostic(
                    "tab.operator_name.capacity",
                    format!("Tab {tab_id}: no room to remember the name the operator gave it"),
                );
                return;
            }
            names.insert(tab_id.to_owned(), name.to_owned());
        } else {
            self.snapshot.ui_state.operator_tab_names.remove(tab_id);
        }
        self.persist_ui_state();
    }

    pub(super) fn rename_tab(&mut self, payload: RenameTabPayload) -> bool {
        if self
            .pending_tab_rename
            .as_ref()
            .is_some_and(|pending| pending.receipt.phase == "pending")
        {
            self.snapshot.status.tab_rename = Some(TabRenameSnapshot {
                request_id: payload.request_id,
                tab_id: payload.tab_id,
                label: payload.label,
                phase: "failed".into(),
            });
            self.push_diagnostic("tab.rename.busy", "A tab rename is already pending");
            return true;
        }
        let local = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.remote_target_id.is_none())
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .find(|tab| tab.id.as_deref() == Some(&payload.tab_id))
            .cloned();
        let remote = self.snapshot.status.remote.iter().find_map(|status| {
            status
                .session
                .as_ref()?
                .workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
                .flat_map(|checkout| &checkout.tabs)
                .find(|tab| tab.id.as_deref() == Some(&payload.tab_id))
                .map(|tab| (status.target_id.clone(), tab.clone()))
        });
        let (target, tab) = match (local, remote) {
            (Some(tab), _) => (None, Some(tab)),
            (None, Some((target, tab))) => (Some(target), Some(tab)),
            _ => (None, None),
        };
        let connection = target
            .as_ref()
            .map(|target| {
                self.remote_connection_generations
                    .get(target)
                    .copied()
                    .unwrap_or(0)
            })
            .unwrap_or(self.live_generation);
        let receipt = TabRenameSnapshot {
            request_id: payload.request_id.clone(),
            tab_id: payload.tab_id.clone(),
            label: payload.label.trim().to_owned(),
            phase: "pending".into(),
        };
        self.snapshot.status.tab_rename = Some(receipt.clone());
        self.pending_tab_rename = Some(PendingRename {
            receipt,
            previous: tab
                .as_ref()
                .map(|tab| tab.naming.raw.clone())
                .unwrap_or_default(),
            target: target.clone(),
            connection,
            confirmed: false,
        });
        let spawned = (|| {
            if tab.is_none() {
                return Err("The tab is no longer available".to_owned());
            }
            let tab_id = match target.as_deref() {
                Some(target) => crate::address::tab_of(&payload.tab_id, target)
                    .ok_or_else(|| "The tab is not scoped to this host".to_owned())?
                    .to_owned(),
                None => payload.tab_id,
            };
            let action = RemoteControlAction::RenameTab {
                tab_id,
                label: payload.label.trim().to_owned(),
                request_id: payload.request_id.clone(),
            };
            match target.as_deref() {
                None => self.submit_local_control(action),
                Some(target) => {
                    let context = self
                        .remote_controls
                        .get(target)
                        .cloned()
                        .ok_or_else(|| "The remote host is unavailable".to_owned())?;
                    live::spawn_remote_control(
                        context,
                        payload.request_id.clone(),
                        action,
                        connection,
                    )
                }
            }
        })();
        if let Err(message) = spawned {
            self.ingest_tab_rename_result(&payload.request_id, Err(message));
        }
        true
    }

    pub(super) fn ingest_tab_rename_result(
        &mut self,
        request_id: &str,
        result: Result<(), String>,
    ) -> bool {
        let Some(mut pending) = self.pending_tab_rename.take() else {
            return false;
        };
        if pending.receipt.request_id != request_id {
            self.pending_tab_rename = Some(pending);
            return false;
        }
        let generation = pending
            .target
            .as_ref()
            .map(|target| {
                self.remote_connection_generations
                    .get(target)
                    .copied()
                    .unwrap_or(0)
            })
            .unwrap_or(self.live_generation);
        let result = if generation == pending.connection {
            result
        } else {
            Err("The host connection changed before rename completed".into())
        };
        pending.receipt.phase = if result.is_ok() {
            "succeeded"
        } else {
            "failed"
        }
        .into();
        match result {
            Err(message) => self.push_diagnostic(
                "tab.rename.failed",
                format!("Tab {}: {message}", pending.receipt.tab_id),
            ),
            // A device's tab never outlives its agents, so only this
            // machine's names are remembered.
            Ok(()) if pending.target.is_none() => {
                let given = crate::model::TabNaming {
                    raw: pending.receipt.label.clone(),
                    ..Default::default()
                };
                self.remember_operator_tab_name(&pending.receipt.tab_id, given.manual_name());
            }
            Ok(()) => {}
        }
        if self
            .snapshot
            .status
            .tab_rename
            .as_ref()
            .is_some_and(|receipt| receipt.request_id == request_id)
        {
            self.snapshot.status.tab_rename = Some(pending.receipt.clone());
        }
        let names = &self.snapshot.ui_state.operator_tab_names;
        for workspace in &mut self.snapshot.navigator.workspaces {
            for checkout in &mut workspace.checkouts {
                for tab in &mut checkout.tabs {
                    pending.apply(tab);
                    tab.naming.by_operator = tab.naming.named_by(tab.id.as_deref(), names);
                }
                refresh_strip_labels(checkout);
            }
        }
        for remote in &mut self.snapshot.status.remote {
            if let Some(session) = &mut remote.session {
                for workspace in &mut session.workspaces {
                    for checkout in &mut workspace.checkouts {
                        for tab in &mut checkout.tabs {
                            pending.apply(tab);
                        }
                        refresh_strip_labels(checkout);
                    }
                }
            }
        }
        pending.apply(&mut self.snapshot.tab);
        self.pending_tab_rename = Some(pending);
        true
    }
}
