//! The replacement shell a close leaves inside the closing tab, which keeps
//! a tab the operator named when its last agent leaves.
use super::*;

impl Runtime {
    /// A tab the operator named through Rename outlives its agents
    /// (docs/UI_BEHAVIOR.md, Agent areas): closing its only pane while that
    /// pane holds an agent leaves a shell in the tab. A spawn's name is not
    /// the operator's; a delegated child's tab and a failed terminal, which
    /// must stay closable, never keep their tab; neither does the leftover
    /// shell.
    pub(super) fn keeps_named_tab(
        &self,
        target: &crate::recent_closed::capture::CloseCaptureTarget,
        tab: &TabSnapshot,
    ) -> bool {
        let crate::recent_closed::capture::CloseCaptureTarget::Pane { pane_id } = target else {
            return false;
        };
        tab.naming.operator_name().is_some()
            && !tab.delegated
            && matches!(tab.panes.as_slice(), [only]
                if only.id == *pane_id && only.failed_terminal_id.is_none())
            && self
                .snapshot
                .navigator
                .agents
                .iter()
                .any(|agent| agent.pane_id == *pane_id)
    }

    /// `shell` is the replacement's pane in the closing tab.
    pub(crate) fn ingest_close_replacement(
        &mut self,
        request: &crate::recent_closed::capture::CloseEffectRequest,
        shell: &str,
        payload: SessionSnapshotPayload,
    ) -> Result<(), String> {
        if request.replacement.is_none() {
            return Err("replacement context is missing".into());
        }
        // A shell made in the closing tab is the close's own even when the
        // close moved on while it was made, so a retry closes beside it.
        self.note_shell_in_tab(&request.key, shell);
        let operation = self
            .close_operations
            .get(&request.key)
            .ok_or("close intent is no longer pending")?;
        if operation.phase != "transmitting"
            || operation.connection_generation != self.live_generation
        {
            return Err("close intent changed while preparing its shell".into());
        }
        self.ingest_shell_in_tab(request, shell, payload)
    }

    /// A shell in the closing tab that carries this close's marker is the
    /// close's own, whether or not the worker could hand it over: the tab's
    /// approved panes grow by it, so a retry closes beside it rather than
    /// reading the tab as changed.
    pub(crate) fn note_shell_in_tab(&mut self, key: &str, shell: &str) {
        let Some(operation) = self.close_operations.get_mut(key) else {
            return;
        };
        if !operation.request.context.replacement_in_tab {
            return;
        }
        operation.replacement_id = Some(shell.to_owned());
        if !operation.scope_pane_ids.iter().any(|id| id == shell) {
            operation.scope_pane_ids.push(shell.to_owned());
            operation.scope_pane_ids.sort();
        }
    }

    /// Whether `pane_id` is the shell a close underway made in its tab, which
    /// the event stream may not have shown yet.
    pub(super) fn close_shell_in_flight(&self, pane_id: &str) -> bool {
        self.close_operations.values().any(|operation| {
            operation.request.context.replacement_in_tab
                && operation.replacement_id.as_deref() == Some(pane_id)
                && matches!(
                    operation.phase.as_str(),
                    "transmitting" | "awaiting_topology"
                )
        })
    }

    /// The shell a named tab's last agent leaves behind is in the tab now,
    /// and it takes the keyboard from the closing pane.
    fn ingest_shell_in_tab(
        &mut self,
        request: &crate::recent_closed::capture::CloseEffectRequest,
        shell: &str,
        payload: SessionSnapshotPayload,
    ) -> Result<(), String> {
        let closing = self
            .close_operations
            .get(&request.key)
            .ok_or("close intent disappeared")?
            .target_id
            .clone();
        self.ingest_session(Ok(payload));
        if self.snapshot.terminal.pane_id.as_deref() == Some(closing.as_str()) {
            self.select_terminal_pane(Some(shell.to_owned()));
        }
        let operation = self
            .close_operations
            .get(&request.key)
            .ok_or("close intent disappeared")?;
        if let Some(message) = self.close_precondition_failure(operation) {
            return Err(message);
        }
        if let Some(operation) = self.close_operations.get_mut(&request.key) {
            operation.deadline_at_unix_ms =
                Some(unix_milliseconds().saturating_add(CLOSE_STAGE_TIMEOUT_MS));
        }
        Ok(())
    }

    pub(super) fn queue_restored_agent_tab(
        &mut self,
        context: &ClosedContext,
        tab_id: &str,
        claim: &str,
    ) -> Result<(), String> {
        let key = (self.node.as_str().to_owned(), context.checkout_path.clone());
        let placement_capacity = self.agent_placement_capacity(Some((&key, claim)));
        let Some(store) = self.workspace_views.as_mut() else {
            return Ok(());
        };
        let view = store.views.entry(&key.0, &key.1);
        let group = context
            .agent_group
            .clone()
            .filter(|id| view.group_layout(Some(id)).is_some());
        let area = (group == context.agent_group)
            .then(|| context.agent_area.clone())
            .flatten()
            .or_else(|| {
                store
                    .views
                    .get(&key.0, &key.1)
                    .and_then(|view| view.group_layout(group.as_deref()))
                    .map(|layout| (layout.tree.active_area.clone(), usize::MAX))
            });
        let Some((area, index)) = area else {
            return Ok(());
        };
        if !placement_capacity && !store.agent_placements.contains_key(tab_id) {
            return Err("Agent placement queue is full".into());
        }
        store
            .agent_placements
            .insert(tab_id.to_owned(), (key, group, area, Some(index)));
        Ok(())
    }

    pub(super) fn retry_agent_close(&mut self, key: &str) -> bool {
        let Some(operation) = self.close_operations.get(key).cloned() else {
            return false;
        };
        if !operation.request.context.replacement_in_tab
            || !matches!(operation.phase.as_str(), "refused" | "failed")
        {
            return false;
        }
        let reuse = operation.replacement_id.as_ref().is_some_and(|id| {
            self.layout_holding_pane(id)
                .is_some_and(|layout| layout.tab_id == operation.scope_id)
        });
        let mut operation = operation;
        if !reuse && let Some(gone) = operation.replacement_id.take() {
            // A shell that left the tab is no longer one of its panes.
            operation.scope_pane_ids.retain(|id| *id != gone);
        }
        if let Some(message) = self.close_precondition_failure(&operation) {
            self.cancel_close_before_effect(key, message);
            return true;
        }
        let Some(context) = self.live.clone() else {
            return false;
        };
        if let Some(current) = self.close_operations.get_mut(key) {
            current.allow_replacement_create = !reuse;
            current.replacement_id = operation.replacement_id.clone();
            current.scope_pane_ids = operation.scope_pane_ids.clone();
            current.phase = "preparing".into();
            current.stage = "capture".into();
            current.message = None;
            current.deadline_at_unix_ms =
                Some(unix_milliseconds().saturating_add(CLOSE_STAGE_TIMEOUT_MS));
            current.connection_generation = self.live_generation;
            current.request.connection_generation = self.live_generation;
        }
        let request = self.close_operations[key].request.clone();
        self.panes_closing.extend(operation.pane_ids);
        // The node refuses keys for a closing pane from now on.
        self.sync_terminal_intents();
        if let Err(message) = crate::recent_closed::capture::spawn_close_capture(context, request) {
            self.fail_close_operation(key, message);
        }
        self.sync_recent_closed_snapshot();
        true
    }

    pub(super) fn dismiss_agent_close(&mut self, key: &str) -> bool {
        let Some(operation) = self.close_operations.get(key).cloned() else {
            return false;
        };
        if !matches!(operation.phase.as_str(), "failed" | "refused") {
            return false;
        }
        self.clear_close_guards(&operation);
        self.close_operations.remove(key);
        self.release_close_registrations(&operation);
        self.close_capture_order.retain(|id| id != key);
        self.promote_close_reservations();
        self.sync_recent_closed_snapshot();
        true
    }
}
