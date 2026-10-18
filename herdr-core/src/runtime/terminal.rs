use hide_node_link::terminal::{
    GridSize, PaneTerminalState, TerminalControl, TerminalNode, TerminalReport, TerminalRoutes,
};

use super::*;

/// A wheel carries the pointer's cell and modifiers because Herdr uses them
/// when the application tracks the mouse. Coordinates are zero-based.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ScrollRequest {
    pub(crate) lines: i32,
    pub(crate) column: Option<u16>,
    pub(crate) row: Option<u16>,
    pub(crate) modifiers: u8,
}

/// What the nodes were last told about the panes on screen, the agents
/// asleep and the panes closing; each is sent again only when it changes.
#[derive(Debug, Default)]
pub(super) struct TerminalIntents {
    shown: Vec<String>,
    asleep: HashSet<String>,
    closing: HashSet<String>,
    /// The keyboard's pane the nodes were last told.
    focus: Option<String>,
}

/// The routes a runtime has before its core gives it the nodes': no node
/// takes anything, and each control says so in the log. Only a runtime
/// built outside `Core::create` (a test's) keeps it.
pub(crate) struct NoTerminals;

impl TerminalNode for NoTerminals {
    fn control(&self, control: TerminalControl) {
        crate::diagnostic!(serde_json::json!({
            "component": "terminal",
            "kind": "terminal.no_node",
            "control": serde_json::to_value(&control).ok().and_then(|value| value.get("op").cloned()),
        }));
    }
    fn key(&self, _target: hide_node_link::terminal::KeyTarget, _bytes: Vec<u8>, _at: u64) {}
    fn view(&self, _pane: &str, _size: GridSize, _new_view: bool) {}
    fn redraw(&self, _pane: &str) {}
}

impl TerminalRoutes for NoTerminals {
    fn install_device(&self, _device: &str, _node: Arc<dyn TerminalNode>) {}
    fn remove_device(&self, _device: &str) {}
    fn terminals_unstarted(&self, _device: &str, _reason: &str) {}
}

/// Whether a pane in `state` may be asked to attach: one with a session, or
/// on its way to one, or failed and waiting for the operator, is not.
pub(super) fn attach_allowed(state: &str) -> bool {
    !matches!(
        state,
        "starting"
            | "controlling"
            | "observing"
            | "unavailable"
            | "ended"
            | "closing"
            | "waiting_size"
    )
}

impl Runtime {
    pub(crate) fn ingest_pane_focus_completion(
        &mut self,
        control: PendingPaneFocusControl,
        result: Result<PaneLayoutSnapshot, live::ControlFailure>,
        elapsed_ms: u128,
    ) -> bool {
        if self.pane_focus_in_flight.as_ref() != Some(&control) {
            return false;
        }
        self.pane_focus_in_flight = None;
        let current_connection = control.live_generation == self.live_generation;
        // The answer settles the tab move this focus makes when Herdr was
        // already on its tab. A refusal moved nothing; a lost answer may
        // still land, so its move waits out the deadline.
        if current_connection {
            match &result {
                Ok(_) => self.answer_pane_focus_tab(control.serial),
                Err(error) if !error.is_ambiguous() => {
                    self.drop_pane_focus_tab(Some(control.serial))
                }
                Err(_) => {}
            }
        }
        let latest = current_connection
            && self
                .pending_pane_focus
                .as_ref()
                .is_some_and(|pending| pending.pane_control_serial == Some(control.serial));
        let unknown = result.as_ref().is_err_and(|error| error.is_ambiguous());
        let phase = if unknown {
            "unknown"
        } else if !current_connection {
            "stale_connection"
        } else if !latest {
            "superseded"
        } else if result.is_ok() {
            "confirmed"
        } else {
            "failed"
        };
        crate::diagnostic!(serde_json::json!({
            "component": "pane_focus",
            "kind": "pane.focus.completed",
            "pane_id": control.target_id,
            "serial": control.serial,
            "connection_generation": control.live_generation,
            "phase": phase,
            "duration_ms": elapsed_ms,
            "message": result.as_ref().err().map(live::ControlFailure::message),
        }));
        if unknown {
            let error = result.expect_err("ambiguous focus result");
            // A read deadline ends our wait, not Herdr's external effect.
            // Abort this entire automatic burst instead of running a successor
            // that the unknown older mutation might later overwrite.
            if let Some(pending) = self.pending_pane_focus.take() {
                let message = format!(
                    "{}; focus for {} is unconfirmed and no queued focus was sent. Select the pane again to retry.",
                    error.message(),
                    pending.target_id
                );
                self.finish_pane_focus_request(
                    pending.request_id.as_deref(),
                    &pending.target_id,
                    "failed",
                    Some(message.clone()),
                    true,
                );
                self.push_diagnostic("pane.focus.unknown", message.clone());
                self.set_error("pane.focus_unknown", message, true);
            } else {
                self.push_diagnostic("pane.focus.unknown", error.message());
            }
            return true;
        }
        if latest {
            let pending = self.pending_pane_focus.take().expect("latest focus intent");
            match result {
                Ok(layout) => {
                    // Update the focus memory from the authoritative reads;
                    // geometry still belongs to the sequenced session stream.
                    for stored in &mut self.snapshot.pane_layouts {
                        if stored.workspace_id == layout.workspace_id {
                            self.herdr_active_tab_ids.remove(&stored.tab_id);
                        }
                        if stored.tab_id == layout.tab_id {
                            stored.focused_pane_id = layout.focused_pane_id.clone();
                        }
                    }
                    self.herdr_active_tab_ids.insert(layout.tab_id.clone());
                    self.finish_pane_focus_request(
                        pending.request_id.as_deref(),
                        &pending.target_id,
                        "succeeded",
                        None,
                        false,
                    );
                    self.push_diagnostic(
                        "pane.focus",
                        format!(
                            "Pane {} focus confirmed in {elapsed_ms} ms",
                            pending.target_id
                        ),
                    );
                }
                Err(error) => {
                    let message = error.message().to_owned();
                    self.finish_pane_focus_request(
                        pending.request_id.as_deref(),
                        &pending.target_id,
                        "failed",
                        Some(message.clone()),
                        true,
                    );
                    self.push_diagnostic(
                        "pane.focus.refused",
                        format!(
                            "Herdr did not confirm pane focus {}: {message}; Hide keeps it",
                            pending.target_id
                        ),
                    );
                    self.set_error("pane.focus_failed", message, true);
                }
            }
        } else {
            // A superseded refusal is diagnostic detail, never the newer
            // caller's failure. An old connection cannot confirm its successor.
            self.push_diagnostic(
                "pane.focus.superseded",
                format!(
                    "Pane {} focus result {phase} in {elapsed_ms} ms",
                    control.target_id
                ),
            );
        }
        // A newer intent is still waiting, so it takes the next turn on the
        // lane, on whatever connection is current by then.
        if self.pending_pane_focus.is_some() {
            let _ = self.submit_pane_focus_turn();
        }
        true
    }

    pub(super) fn reconcile_remote_terminal_panes(
        &mut self,
        target_id: &str,
        live_pane_ids: &HashSet<String>,
        active_pane_ids: &HashSet<String>,
    ) -> bool {
        let target_prefix = remote_pane_id_prefix(target_id);
        let belongs_to_target = |pane_id: &str| pane_id.starts_with(&target_prefix);
        let projected_pane_ids = self
            .snapshot
            .terminal
            .panes
            .iter()
            .filter(|pane| belongs_to_target(&pane.pane_id))
            .map(|pane| pane.pane_id.clone())
            .collect::<HashSet<_>>();
        let mut changed = &projected_pane_ids != live_pane_ids;
        // A pane the remote no longer lists loses everything; a pane it lists
        // but does not run a session for keeps its sizes and loses the session.
        changed |= self.retain_terminal_pane_state(|pane_id| {
            !belongs_to_target(pane_id) || live_pane_ids.contains(pane_id)
        });
        changed |= self.retain_terminal_session_state(|pane_id| {
            !belongs_to_target(pane_id) || active_pane_ids.contains(pane_id)
        });

        self.snapshot.terminal.panes.retain(|pane| {
            !belongs_to_target(&pane.pane_id) || live_pane_ids.contains(&pane.pane_id)
        });
        let mut pane_ids = live_pane_ids.iter().cloned().collect::<Vec<_>>();
        pane_ids.sort();
        for pane_id in &pane_ids {
            self.ensure_terminal_pane(pane_id);
        }
        for pane_id in &pane_ids {
            if !active_pane_ids.contains(pane_id) {
                let before = self.terminal_pane_snapshot(pane_id);
                self.sync_transport_projection(pane_id);
                changed |= self.terminal_pane_snapshot(pane_id) != before;
            }
        }
        let mut active_pane_ids = active_pane_ids.iter().cloned().collect::<Vec<_>>();
        active_pane_ids.sort();
        for pane_id in active_pane_ids {
            changed |= self.request_terminal_control(&pane_id);
        }
        changed
    }
    /// Removes every piece of terminal state for panes that no longer exist,
    /// and tells their node to forget them. Keeping this list in one place
    /// prevents a newly added pane-keyed cache from surviving retirement and
    /// being inherited if Herdr reuses an id.
    pub(super) fn retain_terminal_pane_state(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let before = self.terminal_state_len();
        self.retain_terminal_session_state(&keep);
        self.terminal_sizes.retain(|pane_id, _| keep(pane_id));
        let close_held_panes = self
            .close_operations
            .values()
            .flat_map(|operation| operation.pane_ids.iter().cloned())
            .collect::<HashSet<_>>();
        self.panes_closing
            .retain(|pane_id| keep(pane_id) || close_held_panes.contains(pane_id));
        before != self.terminal_state_len()
    }
    /// Ends the terminal session of every pane `keep` rejects while the pane
    /// itself, and so its sizes, stays known: its node forgets the pane.
    pub(super) fn retain_terminal_session_state(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let before = self.terminal_state_len();
        // A pane Herdr named for typed keys is not gone before its layout
        // first carries it (D-11, B9).
        let gone = self
            .terminal_states
            .keys()
            .chain(&self.terminal_attach_requested)
            .filter(|pane_id| !keep(pane_id) && !self.input_requests.awaits_layout(pane_id))
            .cloned()
            .collect::<HashSet<_>>();
        for pane_id in gone {
            self.terminal_states.remove(&pane_id);
            self.terminal_attach_requested.remove(&pane_id);
            self.terminals
                .control(TerminalControl::Forget { pane: pane_id });
        }
        self.reconcile_attachment_target();
        self.wheel_before_attach.retain(|pane_id, _| keep(pane_id));
        self.viewport_scrolls.retain(|pane_id, _| keep(pane_id));
        self.panes_scroll_held.retain(|pane_id| keep(pane_id));
        before != self.terminal_state_len()
    }
    /// Every pane-keyed terminal map, counted together so a retain pass can
    /// report whether it removed anything.
    pub(super) fn terminal_state_len(&self) -> usize {
        self.terminal_states.len()
            + self.terminal_sizes.len()
            + self.wheel_before_attach.len()
            + self.viewport_scrolls.len()
            + self.panes_scroll_held.len()
            + self.panes_closing.len()
    }
    pub(super) fn reconcile_remote_terminal_selection(&mut self) -> bool {
        let focused_device_id = self.snapshot.navigator.focused_device_id.clone();
        let sessions = self
            .snapshot
            .status
            .remote
            .iter()
            .filter_map(|status| {
                status
                    .session
                    .clone()
                    .map(|session| (status.target_id.clone(), session))
            })
            .collect::<Vec<_>>();
        sessions
            .into_iter()
            .fold(false, |changed, (target_id, session)| {
                let (live_pane_ids, active_pane_ids) = remote_terminal_pane_sets(
                    &session,
                    focused_device_id.as_deref() == Some(target_id.as_str()),
                    &self.node_shown_agent_tabs(&target_id),
                );
                self.reconcile_remote_terminal_panes(&target_id, &live_pane_ids, &active_pane_ids)
                    | changed
            })
    }
    pub(super) fn ensure_terminal_pane(&mut self, pane_id: &str) {
        if self
            .snapshot
            .terminal
            .panes
            .iter()
            .any(|pane| pane.pane_id == pane_id)
        {
            return;
        }
        let pane = self.terminal_pane_snapshot(pane_id);
        self.snapshot.terminal.panes.push(pane);
    }
    pub(super) fn refresh_pane_headers(&mut self) -> bool {
        #[cfg(test)]
        {
            self.pane_header_derivations += 1;
        }
        let mut headers = BTreeMap::new();
        let devices = &self.snapshot.navigator.devices;
        let local = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .map(|workspace| (workspace, None));
        let remote = self
            .snapshot
            .status
            .remote
            .iter()
            .filter_map(|remote| remote.session.as_ref().map(|session| (remote, session)))
            .flat_map(|(remote, session)| {
                session.workspaces.iter().map(move |workspace| {
                    (
                        workspace,
                        (remote.state != "connected").then_some(
                            devices
                                .iter()
                                .find(|device| device.id == remote.target_id)
                                .and_then(|device| device.label.as_deref())
                                .unwrap_or(remote.target_id.as_str()),
                        ),
                    )
                })
            });
        let agents: HashMap<_, _> = self
            .snapshot
            .navigator
            .agents
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|remote| remote.session.as_ref())
                    .flat_map(|session| &session.agents),
            )
            .map(|agent| (agent.pane_id.as_str(), agent))
            .collect();
        let transports: HashMap<_, _> = self
            .snapshot
            .terminal
            .panes
            .iter()
            .map(|pane| (pane.pane_id.as_str(), pane))
            .collect();
        for (workspace, offline) in local.chain(remote) {
            let wait = self.helper_link_wait(&workspace.device_id);
            for pane in workspace
                .checkouts
                .iter()
                .flat_map(|checkout| &checkout.tabs)
                .flat_map(|tab| &tab.panes)
            {
                let header = crate::agent_state::header::of(
                    pane,
                    agents.get(pane.id.as_str()).copied(),
                    transports.get(pane.id.as_str()).copied(),
                    workspace,
                    offline,
                );
                headers.insert(
                    pane.id.clone(),
                    match wait {
                        Some(wait) => crate::agent_state::header::waiting_on_link(header, wait),
                        None => header,
                    },
                );
            }
        }
        if self.snapshot.terminal.headers == headers {
            false
        } else {
            self.snapshot.terminal.headers = headers;
            true
        }
    }

    pub(super) fn terminal_pane_snapshot(&self, pane_id: &str) -> TerminalPaneSnapshot {
        let idle = PaneTerminalState {
            state: "idle".to_owned(),
            mode: None,
            generation: 0,
            attempt: 0,
            message: None,
            exit_category: None,
            retry_decision: "automatic_initial".to_owned(),
            last_attempt_at_unix_ms: None,
        };
        let state = self.terminal_states.get(pane_id).unwrap_or(&idle);
        TerminalPaneSnapshot {
            pane_id: pane_id.to_owned(),
            closed: false,
            exit_code: None,
            transport_state: state.state.clone(),
            transport_message: state.message.clone(),
            transport_generation: state.generation,
            transport_attempt: state.attempt,
            transport_last_attempt_at_unix_ms: state.last_attempt_at_unix_ms,
            transport_exit_category: state.exit_category.clone(),
            transport_retry_decision: state.retry_decision.clone(),
            scroll_held_elsewhere: self.panes_scroll_held.contains(pane_id),
        }
    }
    pub(super) fn sync_transport_projection(&mut self, pane_id: &str) {
        let projected = self.terminal_pane_snapshot(pane_id);
        let mut header_changed = false;
        if let Some(pane) = self
            .snapshot
            .terminal
            .panes
            .iter_mut()
            .find(|pane| pane.pane_id == pane_id)
        {
            header_changed = pane.transport_state != projected.transport_state
                || pane.scroll_held_elsewhere != projected.scroll_held_elsewhere;
            *pane = TerminalPaneSnapshot {
                closed: pane.closed,
                exit_code: pane.exit_code,
                ..projected
            };
        }
        if header_changed {
            self.refresh_pane_headers();
        }
    }
    pub(super) fn sync_focused_terminal_projection(&mut self) {
        let previous = (
            self.snapshot.terminal.closed,
            self.snapshot.terminal.exit_code,
        );
        let Some(pane_id) = self.snapshot.terminal.pane_id.as_deref() else {
            self.snapshot.terminal.closed = false;
            self.snapshot.terminal.exit_code = None;
            return;
        };
        if let Some(pane) = self
            .snapshot
            .terminal
            .panes
            .iter()
            .find(|pane| pane.pane_id == pane_id)
        {
            self.snapshot.terminal.closed = pane.closed;
            self.snapshot.terminal.exit_code = pane.exit_code;
        }
        if previous
            != (
                self.snapshot.terminal.closed,
                self.snapshot.terminal.exit_code,
            )
        {
            self.refresh_pane_headers();
        }
    }
    /// Applies a background pane-control result to the owner-thread snapshot.
    /// The child process is never waited on while the caller holds the
    /// runtime lock; completion arrives through the normal change callback.
    /// Records the outcome of a fork worker.
    ///
    /// `herdr agent new` creates the pane and starts the agent in one atomic
    /// call, so a failure leaves nothing behind and there is no half-made pane
    /// to clean up. The reason it failed is reported rather than swallowed.
    /// Records the machine's listeners and re-attributes every pane to them.
    ///
    /// Panes are re-walked here because ports arrive on their own window rather
    /// than with a session snapshot, so a server that started since the last
    /// topology update would otherwise stay invisible until the topology moved.
    pub fn ingest_listening_ports(&mut self, ports: crate::model::ListeningPortsSnapshot) -> bool {
        let observed = !self.snapshot.status.server_discovery.loading;
        if observed && self.listening_ports == ports {
            return false;
        }
        let status_changed = self.snapshot.status.server_discovery.loading
            || self.snapshot.status.server_discovery.failure != ports.unavailable_reason;
        self.snapshot.status.server_discovery.loading = false;
        self.snapshot.status.server_discovery.failure = ports.unavailable_reason.clone();
        self.listening_ports = ports;
        let entries = self.listening_ports.entries.clone();
        let mut changed = status_changed;
        // This machine's listeners are its own panes'; a device's panes carry
        // its own (`ingest_device_listening_ports`) or none.
        for workspace in self
            .snapshot
            .navigator
            .workspaces
            .iter_mut()
            .filter(|workspace| workspace.remote_target_id.is_none())
        {
            for checkout in workspace.checkouts.iter_mut() {
                for tab in checkout.tabs.iter_mut() {
                    for pane in tab.panes.iter_mut() {
                        let attributed = crate::ports::attributed_ports(&pane.cwd, &entries);
                        let servers = crate::ports::attributed_servers(&pane.cwd, &entries);
                        if pane.ports != attributed || pane.servers != servers {
                            pane.servers = servers;
                            pane.ports = attributed;
                            changed = true;
                        }
                    }
                }
            }
        }
        changed
    }
    /// A node that dials this core read its machine's listeners: they are
    /// attributed to that device's panes only, as this machine's are to its
    /// own. A failed read leaves its panes with none, as a failed local read
    /// does, and is logged once until a read succeeds again.
    pub(crate) fn ingest_device_listening_ports(
        &mut self,
        device_id: &str,
        ports: crate::model::ListeningPortsSnapshot,
    ) -> bool {
        if !self
            .link_origin(device_id)
            .is_some_and(LinkOrigin::reports_own_facts)
        {
            return false;
        }
        if let Some(reason) = ports.unavailable_reason.as_deref() {
            if self.device_ports.remove(device_id).is_none() {
                return false;
            }
            crate::diagnostic!(serde_json::json!({
                "component": "ports",
                "kind": "device.read_failed",
                "target": device_id,
                "reason": reason,
            }));
            return self.refresh_device_catalog(device_id);
        }
        if self.device_ports.get(device_id) == Some(&ports.entries) {
            return false;
        }
        self.device_ports
            .insert(device_id.to_owned(), ports.entries);
        self.refresh_device_catalog(device_id)
    }

    /// Stores what a pane search found and moves the viewport to the match.
    ///
    /// The scroll goes through the same router the wheel uses, because Herdr
    /// owns the pane's history and answers a viewport move with a fresh frame
    /// whichever way this client reaches it.
    pub fn ingest_pane_find(
        &mut self,
        pane_id: &str,
        result: Result<live::PaneFindOutcome, String>,
    ) -> bool {
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(message) => {
                self.snapshot.find = PaneFindSnapshot {
                    pane_id: Some(pane_id.to_owned()),
                    unavailable_reason: Some(message),
                    ..PaneFindSnapshot::default()
                };
                return true;
            }
        };
        let next = PaneFindSnapshot {
            pane_id: Some(pane_id.to_owned()),
            term: outcome.term,
            index: outcome.index,
            total: outcome.total,
            truncated: outcome.truncated,
            unavailable_reason: None,
            opened: None,
        };
        let changed = self.snapshot.find != next;
        self.snapshot.find = next;
        if let Some((direction, lines)) = outcome.scroll {
            let lines = i32::from(lines) * if direction == "up" { 1 } else { -1 };
            return self.scroll_pane(
                pane_id,
                ScrollRequest {
                    lines,
                    ..Default::default()
                },
            ) || changed;
        }
        changed
    }
    /// ⌘F on a pane whose agent has its own find: a worker opens the
    /// agent's search, or answers that Hide's find bar should open because
    /// Herdr holds the pane's history (`live::spawn_agent_find`).
    pub(super) fn open_pane_find(&mut self, pane_id: String, request_id: String) -> bool {
        let find = self
            .agent_row(&pane_id)
            .and_then(|agent| crate::agent_find::agent_find(&agent.agent_kind));
        // The agent left between the frame the shell acted on and this event:
        // the pane is a plain terminal again, which the bar searches.
        let Some(find) = find else {
            return self.ingest_agent_find(&pane_id, request_id, Ok(PaneFindRoute::Bar));
        };
        // A device's pane opens through that device's Herdr; one that cannot
        // be reached says why in the bar.
        let route = match self.pane_api_route(&pane_id) {
            Ok(route) => route,
            Err(reason) => return self.ingest_agent_find(&pane_id, request_id, Err(reason)),
        };
        if let Err(message) = live::spawn_agent_find(
            route,
            request_id.clone(),
            find,
            Arc::clone(&self.input_gate),
        ) {
            return self.ingest_agent_find(&pane_id, request_id, Err(message));
        }
        false
    }
    /// Records where a `pane_find_open` sent the search. A failure opens the
    /// bar with the reason, where the operator can search what Herdr holds.
    pub fn ingest_agent_find(
        &mut self,
        pane_id: &str,
        request_id: String,
        result: Result<PaneFindRoute, String>,
    ) -> bool {
        let (route, unavailable_reason) = match result {
            Ok(route) => (route, None),
            Err(message) => (PaneFindRoute::Bar, Some(message)),
        };
        crate::diagnostic!(serde_json::json!({
            "component": "terminal", "kind": "pane.find_opened",
            "pane_id": pane_id, "request_id": request_id, "route": route,
            "reason": unavailable_reason,
        }));
        if route == PaneFindRoute::Agent {
            // The keys went into the agent's own search box.
            self.note_delivery_key(pane_id, "find");
        }
        self.snapshot.find = PaneFindSnapshot {
            pane_id: Some(pane_id.to_owned()),
            unavailable_reason,
            opened: Some(PaneFindOpened { request_id, route }),
            ..PaneFindSnapshot::default()
        };
        true
    }
    /// Records that the operator submitted to an agent pane, for who sent
    /// the message it writes next (PRD overview-request-view D-19). Only the
    /// moment is kept. An Enter at a prompt Herdr reports (`blocked`) answers
    /// that prompt, not the conversation, and is not one. An Enter at a plan
    /// waiting for approval is, whatever Herdr reads its menu as: approving
    /// writes the operator's next message (Codex's "Implement the plan.").
    /// Herdr's own status is the delivery observation's; a pane with none is
    /// judged from its row, where only a resting agent's wait is the plan's.
    pub(super) fn record_operator_submit(&self, pane_id: &str) {
        let Some(services) = self.label_services.as_ref() else {
            return;
        };
        let Some(row) = self.agent_row(pane_id) else {
            return;
        };
        let at_prompt = crate::agent_state::submit_answers_prompt(
            row,
            self.delivery_observations
                .get(pane_id)
                .map(|observation| observation.status.as_str()),
        );
        if at_prompt {
            return;
        }
        services.input.record(
            pane_id,
            unix_milliseconds(),
            crate::agent_state::is_running(row),
        );
    }
    /// The agent row for a pane on this machine or on a device.
    pub(super) fn agent_row(&self, pane_id: &str) -> Option<&SidebarAgentSnapshot> {
        let local = self.snapshot.navigator.agents.iter();
        let devices = self
            .snapshot
            .status
            .remote
            .iter()
            .filter_map(|remote| remote.session.as_ref())
            .flat_map(|session| session.agents.iter());
        local.chain(devices).find(|agent| agent.pane_id == pane_id)
    }
    /// Moves a pane's view by a wheel's signed lines (positive shows older
    /// lines), and whether that changed the snapshot.
    ///
    /// Herdr gives terminal control to one client per pane. The controlling
    /// session writes `terminal.scroll`, which Herdr routes to the program's
    /// mouse handling or the history; the node writes it. A pane another
    /// client controls (another Herdr client on the same server, say) is
    /// observed, and an observer has no writer, so its wheel moves Herdr's
    /// viewport with `pane.scroll` instead; Herdr keeps one viewport per pane,
    /// so both clients see the move. A pane with no session yet keeps the
    /// lines for its first frame. Every scroll producer comes through here,
    /// so none of them drops a wheel.
    pub(super) fn scroll_pane(&mut self, pane_id: &str, request: ScrollRequest) -> bool {
        if request.lines == 0 {
            return false;
        }
        match self
            .terminal_states
            .get(pane_id)
            .map(|state| state.state.as_str())
        {
            Some("controlling") => {
                self.terminals.control(TerminalControl::Scroll {
                    pane: pane_id.to_owned(),
                    lines: request.lines,
                    column: request.column,
                    row: request.row,
                    modifiers: request.modifiers,
                });
                false
            }
            Some("observing") => {
                if let Some(pending) = self.viewport_scrolls.get_mut(pane_id) {
                    *pending = pending.saturating_add(request.lines);
                    return false;
                }
                self.start_viewport_scroll(pane_id, request.lines)
            }
            _ => {
                let first = !self.wheel_before_attach.contains_key(pane_id);
                let pending = self
                    .wheel_before_attach
                    .entry(pane_id.to_owned())
                    .or_insert(0);
                *pending = pending.saturating_add(request.lines);
                if first {
                    self.push_diagnostic(
                        "terminal.scroll_deferred",
                        format!(
                            "Pane {pane_id} was scrolled before its terminal attached; the wheel is sent with its first frame"
                        ),
                    );
                }
                first
            }
        }
    }
    /// Sends the wheel a pane held while it had no session, now that its
    /// first frame shows which mode it attached in.
    fn release_wheel_before_attach(&mut self, pane_id: &str) {
        if self.wheel_before_attach.is_empty() {
            return;
        }
        if let Some(lines) = self.wheel_before_attach.remove(pane_id) {
            self.scroll_pane(
                pane_id,
                ScrollRequest {
                    lines,
                    ..Default::default()
                },
            );
        }
    }
    fn start_viewport_scroll(&mut self, pane_id: &str, lines: i32) -> bool {
        if lines == 0 {
            return false;
        }
        // A pane whose Herdr cannot be reached says so through its own
        // transport state (a device's `disconnected`); the wheel is logged,
        // never read as another client holding the scroll.
        let route = match self.pane_api_route(pane_id) {
            Ok(route) => route,
            Err(reason) => {
                self.push_diagnostic(
                    "terminal.scroll_failed",
                    format!("Pane {pane_id} was not scrolled: {reason}"),
                );
                return true;
            }
        };
        self.viewport_scrolls.insert(pane_id.to_owned(), 0);
        if let Err(message) = live::spawn_viewport_scroll(route, lines) {
            self.viewport_scrolls.remove(pane_id);
            self.set_error("terminal.scroll_failed", message, true);
            return true;
        }
        false
    }
    /// Applies a landed `pane.scroll`: the marker follows whether Herdr moved
    /// anything, and the lines that arrived meanwhile go out as one request.
    pub fn ingest_viewport_scroll(
        &mut self,
        pane_id: &str,
        result: Result<Option<live::PaneScroll>, live::ViewportScrollError>,
    ) -> bool {
        let Some(pending) = self.viewport_scrolls.remove(pane_id) else {
            return false;
        };
        let mut changed = match result {
            Ok(Some(scroll)) if scroll.max_offset_from_bottom > 0 => {
                self.release_scroll_hold(pane_id)
            }
            // An alternate-screen program has no history in Herdr's viewport;
            // only the controlling client's wheel reaches the program itself.
            Ok(_) => self.hold_scroll_elsewhere(
                pane_id,
                "Herdr has no history to move for this pane, and only the client controlling it can scroll the program",
            ),
            Err(live::ViewportScrollError::Refused(message)) => {
                self.hold_scroll_elsewhere(pane_id, &message)
            }
            Err(live::ViewportScrollError::Unreachable(message)) => {
                self.push_diagnostic(
                    "terminal.scroll_failed",
                    format!("Pane {pane_id} was not scrolled: {message}"),
                );
                true
            }
        };
        changed |= self.scroll_pane(
            pane_id,
            ScrollRequest {
                lines: pending,
                ..Default::default()
            },
        );
        changed
    }
    fn hold_scroll_elsewhere(&mut self, pane_id: &str, reason: &str) -> bool {
        if !self.panes_scroll_held.insert(pane_id.to_owned()) {
            return false;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "terminal", "kind": "terminal.scroll_held_elsewhere",
            "pane_id": pane_id, "reason": reason,
        }));
        self.sync_transport_projection(pane_id);
        true
    }
    pub(super) fn release_scroll_hold(&mut self, pane_id: &str) -> bool {
        if !self.panes_scroll_held.remove(pane_id) {
            return false;
        }
        self.sync_transport_projection(pane_id);
        true
    }
    /// Where a pane's socket requests go: this machine's Herdr, or the
    /// device's Herdr under the id that Herdr knows the pane by. The error is
    /// the reason, worded for the operator.
    pub(super) fn pane_api_route(&self, pane_id: &str) -> Result<live::PaneApiRoute, String> {
        if !pane_id.starts_with("remote:") {
            return self
                .live
                .as_ref()
                .map(|context| live::PaneApiRoute::local(context, pane_id))
                .ok_or_else(|| "This pane needs a live Herdr connection".to_owned());
        }
        let (device, source_pane_id) = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .find_map(|device| {
                super::remote_pane_source_id(&device.id, pane_id).map(|source| (device, source))
            })
            .ok_or_else(|| "This pane's device is no longer registered".to_owned())?;
        let connected = self
            .snapshot
            .status
            .remote
            .iter()
            .any(|status| status.target_id == device.id && status.state == "connected");
        let context = self
            .remote_controls
            .get(&device.id)
            .filter(|_| connected)
            .ok_or_else(|| format!("{} is not connected", device.label))?;
        Ok(live::PaneApiRoute::remote(context, pane_id, source_pane_id))
    }
    pub fn ingest_pane_control_result(
        &mut self,
        action: PaneControlAction,
        result: Result<PaneControlOutcome, String>,
        elapsed_ms: u128,
    ) -> bool {
        self.ingest_pane_control_result_with_failure(
            action,
            result.map_err(live::ControlFailure::Definite),
            elapsed_ms,
            None,
        )
    }

    pub(crate) fn ingest_pane_control_failure_with_generation(
        &mut self,
        action: PaneControlAction,
        result: Result<PaneControlOutcome, live::ControlFailure>,
        elapsed_ms: u128,
        connection_generation: Option<u64>,
    ) -> bool {
        self.ingest_pane_control_result_with_failure(
            action,
            result,
            elapsed_ms,
            connection_generation,
        )
    }

    fn ingest_pane_control_result_with_failure(
        &mut self,
        action: PaneControlAction,
        result: Result<PaneControlOutcome, live::ControlFailure>,
        elapsed_ms: u128,
        connection_generation: Option<u64>,
    ) -> bool {
        let pane_operation_id = self
            .pane_operations
            .iter()
            .find(|(_, operation)| {
                operation.kind == action.kind()
                    && operation.target_id == action.pane_id()
                    && operation.phase == "transmitting"
                    && connection_generation
                        .is_none_or(|generation| operation.connection_generation == generation)
            })
            .map(|(id, _)| id.clone());
        if let Some(id) = pane_operation_id {
            return self.ingest_pane_mutation_result(&id, result, elapsed_ms);
        }
        match (action, result) {
            (
                PaneControlAction::Project { pane_id },
                Ok(PaneControlOutcome::Projected { layout }),
            ) => {
                if self.snapshot.terminal.pane_id.as_deref() != Some(pane_id.as_str()) {
                    self.push_diagnostic(
                        "pane.projection.stale",
                        format!("Ignored stale projection for pane {pane_id}"),
                    );
                    return false;
                }
                if !layout.pane_ids().contains(&pane_id.as_str()) {
                    self.set_error(
                        "pane.projection_mismatch",
                        format!("Projected layout does not contain pane {pane_id}"),
                        true,
                    );
                    return true;
                }
                self.push_diagnostic(
                    "pane.projection.ready",
                    format!("Pane {pane_id} projected in {elapsed_ms} ms"),
                );
                crate::diagnostic!(serde_json::json!({
                    "component": "pane_projection",
                    "kind": "pane.projection_ready",
                    "pane_id": pane_id,
                    "duration_ms": elapsed_ms,
                }));
                self.apply_pane_layout(layout, false);
                true
            }
            (PaneControlAction::MoveToNewTab { pane_id, .. }, outcome) => {
                match outcome {
                    Ok(_) => {
                        // The stamp stays until the layout shows the move
                        // (`relocate_delegated_child_panes`).
                        self.push_diagnostic(
                            "lineage.relocated",
                            format!(
                                "Herdr acknowledged moving delegated pane {pane_id} to its own tab in {elapsed_ms} ms"
                            ),
                        );
                    }
                    Err(error) => {
                        // The request's stamp stays, so the next attempt waits
                        // out RELOCATION_RETRY_INTERVAL_MS: dropping it here
                        // re-sent a refused move on every tick.
                        // Deliberately not a `pane.` diagnostic: the pane
                        // header reads those, and this failure must not
                        // appear over a child the operator never asked to
                        // move (PRD B2).
                        self.push_diagnostic(
                            "lineage.relocate_failed",
                            format!(
                                "Could not move delegated pane {pane_id}: {}",
                                error.message()
                            ),
                        );
                    }
                }
                true
            }
            (PaneControlAction::Focus { pane_id }, Ok(PaneControlOutcome::Acknowledged { .. })) => {
                self.push_diagnostic(
                    "pane.focus",
                    format!(
                        "Pane {pane_id} focus acknowledged in {elapsed_ms} ms; awaiting authoritative event"
                    ),
                );
                true
            }
            (
                PaneControlAction::Split {
                    pane_id, direction, ..
                },
                Ok(PaneControlOutcome::Acknowledged { created_pane_id }),
            ) => {
                let Some(created_pane_id) = created_pane_id else {
                    self.set_error(
                        "pane.split_invalid_response",
                        "Pane split completed without a created pane id",
                        true,
                    );
                    return true;
                };
                self.push_diagnostic(
                    format!("pane.split.{}", direction.as_str()),
                    format!(
                        "Pane {pane_id} split {} to {created_pane_id} in {elapsed_ms} ms",
                        direction.as_str()
                    ),
                );
                crate::diagnostic!(serde_json::json!({
                    "component": "pane_control",
                    "kind": "pane.split_ready",
                    "pane_id": pane_id,
                    "created_pane_id": created_pane_id,
                    "direction": direction.as_str(),
                    "duration_ms": elapsed_ms,
                }));
                true
            }
            (
                PaneControlAction::Resize {
                    pane_id,
                    direction,
                    amount,
                },
                Ok(PaneControlOutcome::Acknowledged { .. }),
            ) => {
                self.push_diagnostic(
                    "pane.resize",
                    format!(
                        "Pane {pane_id} resize {} by {amount:.3} acknowledged in {elapsed_ms} ms; awaiting authoritative event",
                        direction.as_str()
                    ),
                );
                true
            }
            (
                PaneControlAction::ToggleZoom { pane_id },
                Ok(PaneControlOutcome::Acknowledged { .. }),
            ) => {
                self.push_diagnostic(
                    "pane.zoom_toggled",
                    format!(
                        "Pane {pane_id} zoom acknowledged in {elapsed_ms} ms; awaiting authoritative event"
                    ),
                );
                crate::diagnostic!(serde_json::json!({
                    "component": "pane_control",
                    "kind": "pane.zoom_ready",
                    "pane_id": pane_id,
                    "duration_ms": elapsed_ms,
                }));
                true
            }
            (PaneControlAction::Close { pane_id }, Ok(PaneControlOutcome::Acknowledged { .. })) => {
                self.push_diagnostic(
                    "pane.close",
                    format!(
                        "Pane {pane_id} close acknowledged in {elapsed_ms} ms; awaiting authoritative event"
                    ),
                );
                crate::diagnostic!(serde_json::json!({
                    "component": "pane_control",
                    "kind": "pane.close_ready",
                    "pane_id": pane_id,
                    "duration_ms": elapsed_ms,
                }));
                true
            }
            (
                PaneControlAction::Resize { pane_id, .. }
                | PaneControlAction::ToggleZoom { pane_id },
                Ok(PaneControlOutcome::Unchanged),
            ) => {
                self.push_diagnostic(
                    "pane.unchanged",
                    format!("Herdr changed nothing for pane {pane_id} in {elapsed_ms} ms"),
                );
                true
            }
            (PaneControlAction::Project { .. }, Ok(PaneControlOutcome::Acknowledged { .. }))
            | (
                PaneControlAction::Project { .. }
                | PaneControlAction::Focus { .. }
                | PaneControlAction::Split { .. }
                | PaneControlAction::Close { .. },
                Ok(PaneControlOutcome::Unchanged),
            )
            | (
                PaneControlAction::Focus { .. }
                | PaneControlAction::Split { .. }
                | PaneControlAction::Resize { .. }
                | PaneControlAction::ToggleZoom { .. }
                | PaneControlAction::Close { .. },
                Ok(PaneControlOutcome::Projected { .. }),
            ) => {
                self.set_error(
                    "pane.control_invalid_outcome",
                    "Pane control returned an outcome for the wrong operation class",
                    false,
                );
                true
            }
            (PaneControlAction::Project { .. }, Err(error)) => {
                self.set_error("pane.projection_failed", error.message().to_owned(), true);
                true
            }
            (PaneControlAction::Focus { pane_id }, Err(error)) => {
                let message = error.message().to_owned();
                // Hide keeps the pane it focused. The refusal is reported and
                // the wait ends, so the next Herdr event naming another pane
                // is read as the authority it is rather than as a late answer.
                self.clear_refused_pane_focus(&pane_id, &message);
                self.set_error("pane.focus_failed", message, true);
                true
            }
            (PaneControlAction::Split { .. }, Err(error)) => {
                if error.is_ambiguous() {
                    self.set_error(
                        "pane.split_unknown",
                        format!(
                            "Pane split result is unknown; no split was resent: {}",
                            error.message()
                        ),
                        true,
                    );
                } else {
                    self.set_error("pane.split_failed", error.message().to_owned(), true);
                }
                true
            }
            (PaneControlAction::Resize { .. }, Err(error)) => {
                if error.is_ambiguous() {
                    self.set_error(
                        "pane.resize_unknown",
                        format!(
                            "Pane resize result is unknown; no resize was resent: {}",
                            error.message()
                        ),
                        true,
                    );
                } else {
                    self.set_error("pane.resize_failed", error.message().to_owned(), true);
                }
                true
            }
            (PaneControlAction::ToggleZoom { .. }, Err(error)) => {
                if error.is_ambiguous() {
                    self.set_error(
                        "pane.zoom_unknown",
                        format!(
                            "Pane zoom result is unknown; no zoom was resent: {}",
                            error.message()
                        ),
                        true,
                    );
                } else {
                    self.set_error("pane.zoom_failed", error.message().to_owned(), true);
                }
                true
            }
            (PaneControlAction::Close { .. }, Err(error)) => {
                if error.is_ambiguous() {
                    self.set_error(
                        "pane.close_unknown",
                        format!(
                            "Pane close result is unknown; no close was resent: {}",
                            error.message()
                        ),
                        true,
                    );
                } else {
                    self.set_error("pane.close_failed", error.message().to_owned(), true);
                }
                true
            }
        }
    }

    /// A selection that moves, goes missing or retires clears only its own
    /// presentation. The terminal pane entries are the session's, not the
    /// selection's: each one leaves when its pane leaves the session, and a
    /// pane outside the attach window keeps its entry as `released`. The
    /// shell disposes a parked terminal whose entry is gone, so emptying the
    /// list for a checkout switch or a pending creation threw away every
    /// retained tab's renderer, and the layouts likewise stay, or the canvas
    /// passes through a blank frame on the way to the tab asked for.
    pub(super) fn clear_selected_terminal_state(&mut self) {
        self.snapshot.zoomed = None;
        self.snapshot.terminal.closed = false;
        self.snapshot.terminal.exit_code = None;
    }
    /// Points the terminal projection at another pane without discarding
    /// anything Herdr has said.
    ///
    /// Zoom is re-read from that pane's own layout rather than carried over,
    /// so leaving a zoomed tab does not leak its zoom into the next one.
    pub(super) fn select_terminal_pane(&mut self, pane_id: Option<String>) {
        let zoomed = pane_id
            .as_deref()
            .and_then(|pane_id| self.layout_holding_pane(pane_id))
            .and_then(|layout| layout.zoomed.then(|| layout.focused_pane_id.clone()));
        let pane_ids = pane_id
            .as_deref()
            .and_then(|pane_id| self.layout_holding_pane(pane_id))
            .map(|layout| {
                layout
                    .pane_ids()
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.snapshot.terminal.pane_id = pane_id.clone();
        self.snapshot.focused.surface = Surface::Terminal;
        self.snapshot.focused.pane_id = pane_id.clone();
        self.snapshot.ui_state.selected_pane_id = pane_id;
        sync_pane_status(
            &mut self.snapshot.navigator.workspaces,
            &self.snapshot.navigator.agents,
            self.snapshot.focused.pane_id.as_deref(),
            crate::agent_sleep::SleepMachine::Core,
        );
        self.sync_active_tab_projection();
        self.snapshot.zoomed = zoomed;
        for pane_id in pane_ids {
            self.ensure_terminal_pane(&pane_id);
        }
        self.sync_focused_terminal_projection();
    }
    /// Typing into a remote pane makes it the terminal pane (`Event::Key`),
    /// while this machine's selection stays in `ui_state.selected_pane_id`.
    /// Coming back to this machine hands the keyboard back to that pane, or to
    /// the focused pane of the tab being drawn, so a split, a zoom or a find
    /// never names a pane this machine's Herdr does not have. The next local
    /// session ingest would repair it too, but an idle session sends nothing.
    pub(super) fn return_keyboard_to_local_pane(&mut self) {
        if !self
            .snapshot
            .terminal
            .pane_id
            .as_deref()
            .is_some_and(is_remote_scoped_pane_id)
        {
            return;
        }
        let selected = self
            .snapshot
            .ui_state
            .selected_pane_id
            .clone()
            .filter(|pane_id| {
                !is_remote_scoped_pane_id(pane_id) && self.layout_holding_pane(pane_id).is_some()
            });
        let drawn = || {
            let checkout_id = self.snapshot.navigator.focused_checkout_id.as_deref()?;
            let tab_id = self
                .snapshot
                .navigator
                .workspaces
                .iter()
                .flat_map(|workspace| workspace.checkouts.iter())
                .find(|checkout| checkout.id == checkout_id)?
                .active_tab_id
                .as_deref()?;
            self.snapshot
                .pane_layouts
                .iter()
                .find(|layout| layout.tab_id == tab_id)
                .map(|layout| layout.focused_pane_id.clone())
        };
        let pane_id = selected.or_else(drawn);
        crate::diagnostic!(serde_json::json!({
            "component": "view_state",
            "kind": "terminal.keyboard_returned_local",
            "from_pane_id": self.snapshot.terminal.pane_id,
            "to_pane_id": pane_id,
        }));
        self.select_terminal_pane(pane_id);
    }
    pub(super) fn reset_terminal_projection(&mut self, pane_id: Option<String>) {
        self.clear_selected_terminal_state();
        self.select_terminal_pane(pane_id);
    }
    /// Opens or focuses the file tab for one path in a checkout Hide is
    /// already showing. The caller owns the context check and the persistence,
    /// because a reveal has already made that decision by the time it gets
    /// here and would otherwise make it twice.
    /// The tab the operator is looking at: the focused checkout's visible tab.
    ///
    /// Another checkout's visible tab is that checkout's memory, not a tab on
    /// screen, so it does not renew an attach.
    pub(crate) fn process_info_attached_tabs(&self, target: Option<&str>) -> Vec<String> {
        let Some(target) = target else {
            return self.recent_visible_tabs.clone();
        };
        if !self.device_in_front(target) {
            return Vec::new();
        }
        self.snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == target)
            .and_then(|status| status.session.as_ref())
            .and_then(|session| {
                session.focused_tab_id.clone().or_else(|| {
                    session
                        .focused_checkout_id
                        .as_ref()
                        .and_then(|checkout| session.active_tab_ids.get(checkout))
                        .cloned()
                })
            })
            .into_iter()
            .collect()
    }

    pub(crate) fn process_info_focused_pane(&self, target: Option<&str>) -> Option<String> {
        match target {
            None => self.snapshot.focused.pane_id.clone(),
            Some(target) => self
                .snapshot
                .status
                .remote
                .iter()
                .find(|status| status.target_id == target)
                .and_then(|status| status.session.as_ref())
                .and_then(|session| session.focused_pane_id.as_deref())
                .and_then(|id| remote_pane_source_id(target, id))
                .map(str::to_owned),
        }
    }

    pub(super) fn focused_visible_tab_id(&self) -> Option<String> {
        self.snapshot
            .navigator
            .focused_checkout_id
            .as_deref()
            .and_then(|checkout_id| self.visible_tab_ids.get(checkout_id))
            .cloned()
    }
    /// Records that a tab was on screen, releases whatever fell out of the
    /// window that leaves, and tells the nodes what is on screen now.
    pub(super) fn track_visible_tab_attachments(&mut self) -> bool {
        let known_tabs = self
            .snapshot
            .pane_layouts
            .iter()
            .map(|layout| layout.tab_id.clone())
            .collect::<HashSet<_>>();
        // A tab Herdr no longer reports cannot come back, so holding its slot
        // would shrink the window for the tabs that can.
        self.recent_visible_tabs
            .retain(|tab_id| known_tabs.contains(tab_id));
        let shown = self.shown_agent_tabs();
        for tab_id in shown.iter().rev() {
            self.recent_visible_tabs.retain(|held| held != tab_id);
            self.recent_visible_tabs.insert(0, tab_id.clone());
        }
        self.recent_visible_tabs
            .truncate(ATTACHED_TAB_LIMIT.max(shown.len()));
        let released = self.release_sessions_outside_attach_window();
        self.sync_terminal_intents() | released
    }
    /// Ends the terminal session of every pane whose tab has left the attach
    /// window.
    ///
    /// The pane keeps its projection entry, carrying `released`, because the
    /// sidebar and the pane header read their state from there and a missing
    /// entry would read as a failure rather than as a pane nobody is watching.
    /// The shell drops the canvas and the buffered bytes on that state, so the
    /// tab redraws from Herdr's own frame when it is next shown.
    pub(super) fn release_sessions_outside_attach_window(&mut self) -> bool {
        let window = self
            .recent_visible_tabs
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let attached = self
            .snapshot
            .pane_layouts
            .iter()
            .filter(|layout| window.contains(&layout.tab_id))
            .flat_map(|layout| layout.pane_ids())
            .map(str::to_owned)
            .collect::<HashSet<_>>();
        // A pane no layout claims is not a pane that left the window; the
        // session reconcile owns those and drops them with the session.
        let placed = self
            .snapshot
            .pane_layouts
            .iter()
            .flat_map(|layout| layout.pane_ids())
            .map(str::to_owned)
            .collect::<HashSet<_>>();
        let releasing = self
            .terminal_states
            .iter()
            .filter(|(_, state)| state.state != "released")
            .map(|(pane_id, _)| pane_id)
            .filter(|pane_id| !pane_id.starts_with("remote:"))
            .filter(|pane_id| placed.contains(*pane_id) && !attached.contains(*pane_id))
            .cloned()
            .collect::<Vec<_>>();
        if releasing.is_empty() {
            return false;
        }
        for pane_id in releasing {
            let message = format!(
                "Pane {pane_id} was detached after its tab left the last {ATTACHED_TAB_LIMIT} shown"
            );
            // The node says the same once it has ended the session; the
            // screen shows it from this event on.
            if let Some(state) = self.terminal_states.get_mut(&pane_id) {
                state.state = "released".to_owned();
                state.message = Some(message.clone());
                state.mode = None;
                state.exit_category = None;
                state.retry_decision = "on_next_visit".to_owned();
            }
            self.terminals.control(TerminalControl::Release {
                pane: pane_id.clone(),
                message,
            });
            self.sync_transport_projection(&pane_id);
            self.push_diagnostic(
                "terminal.session_released",
                format!("Released the terminal session for pane {pane_id}"),
            );
        }
        true
    }
    /// Names the pane whose first frame ends record `id`. A created pane's
    /// node watches for its next frame now; a resized pane's node is asked
    /// when its resize goes out (`send_terminal_size`), since a frame before
    /// that shows the old grid.
    pub(super) fn await_op_frame(&mut self, id: &str, pane_id: &str, after_resize: bool) {
        self.op_timings.await_frame(id, pane_id, after_resize);
        if !after_resize {
            self.terminals.control(TerminalControl::WatchFrame {
                pane: pane_id.to_owned(),
            });
        }
    }

    /// Asks the pane's node to attach it, with the size the pane last had, and
    /// whether that changed what the pane shows. A pane with a session, on
    /// its way to one, or failed and waiting for the operator is left as it
    /// is; its node keeps that rule too, so a repeated sync update asks for
    /// nothing twice.
    pub(super) fn request_terminal_control(&mut self, pane_id: &str) -> bool {
        let allowed = self
            .terminal_states
            .get(pane_id)
            .is_none_or(|state| attach_allowed(&state.state));
        if !allowed {
            return false;
        }
        self.attach_terminal(pane_id, false);
        true
    }

    fn attach_terminal(&mut self, pane_id: &str, manual: bool) {
        // A released pane asked for again is no longer released; its node
        // reports what the attach does next, and one ask is enough.
        if let Some(state) = self
            .terminal_states
            .get_mut(pane_id)
            .filter(|state| state.state == "released")
        {
            state.state = "idle".to_owned();
        }
        let size = self
            .terminal_sizes
            .get(pane_id)
            .map(|&(rows, cols)| GridSize { rows, cols });
        self.terminal_attach_requested.insert(pane_id.to_owned());
        self.terminals.control(TerminalControl::Attach {
            pane: pane_id.to_owned(),
            size,
            manual,
        });
    }

    /// The operator's Reconnect: the pane's node starts again from a first
    /// attempt, whatever it was doing.
    pub(super) fn reconnect_terminal(&mut self, pane_id: &str) {
        self.push_diagnostic(
            "pane.reconnect.requested",
            format!("Reconnect requested for pane {pane_id}"),
        );
        self.attach_terminal(pane_id, true);
    }

    /// Tells the nodes what changed among the panes on screen, the agents
    /// asleep and the panes closing, and asks a shown pane that was released
    /// to attach again. Whether a shown pane's state changed is returned.
    /// The panes whose agent is ending, asleep, waking or failed to wake:
    /// from the moment the end begins until the agent is back, they take no
    /// input from a node's terminal or from a writer that asks the gate.
    fn sleep_record_panes(&self) -> HashSet<String> {
        self.snapshot
            .ui_state
            .agent_sleep
            .records
            .keys()
            .cloned()
            .collect()
    }

    /// Closes the input gate to exactly the panes the sleep records name,
    /// for a change of records that does not reach the nodes' intents.
    pub(super) fn sync_input_gate(&self) {
        self.input_gate.set_closed(&self.sleep_record_panes());
    }

    pub(super) fn sync_terminal_intents(&mut self) -> bool {
        let shown_tabs = self.shown_agent_tabs();
        let mut shown = self
            .snapshot
            .pane_layouts
            .iter()
            .filter(|layout| shown_tabs.contains(&layout.tab_id))
            .flat_map(|layout| layout.pane_ids())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        // The router tells each node which of its own panes are shown.
        shown.sort();
        shown.dedup();
        let mut changed = false;
        if shown != self.terminal_intents.shown {
            // A released pane back on screen attaches again (B10).
            let returning = shown
                .iter()
                .filter(|pane_id| {
                    self.terminal_states
                        .get(*pane_id)
                        .is_some_and(|state| state.state == "released")
                })
                .cloned()
                .collect::<Vec<_>>();
            self.terminals.control(TerminalControl::Shown {
                panes: shown.clone(),
            });
            self.terminal_intents.shown = shown;
            for pane_id in returning {
                self.attach_terminal(&pane_id, false);
                changed = true;
            }
        }
        let asleep = self.sleep_record_panes();
        self.input_gate.set_closed(&asleep);
        if asleep != self.terminal_intents.asleep {
            for pane in asleep.difference(&self.terminal_intents.asleep) {
                self.terminals.control(TerminalControl::Asleep {
                    pane: pane.clone(),
                    asleep: true,
                });
            }
            for pane in self.terminal_intents.asleep.difference(&asleep) {
                self.terminals.control(TerminalControl::Asleep {
                    pane: pane.clone(),
                    asleep: false,
                });
            }
            self.terminal_intents.asleep = asleep;
        }
        let closing = self.panes_closing.clone();
        if closing != self.terminal_intents.closing {
            for pane in closing.difference(&self.terminal_intents.closing) {
                self.terminals.control(TerminalControl::Closing {
                    pane: pane.clone(),
                    closing: true,
                });
            }
            for pane in self.terminal_intents.closing.difference(&closing) {
                self.terminals.control(TerminalControl::Closing {
                    pane: pane.clone(),
                    closing: false,
                });
            }
            self.terminal_intents.closing = closing;
        }
        changed
    }

    /// The pane a key goes to without moving the keyboard: the terminal's,
    /// while the keyboard is in a terminal at all.
    fn terminal_keyboard_pane(&self) -> Option<&str> {
        (self.snapshot.focused.surface == Surface::Terminal)
            .then_some(self.snapshot.terminal.pane_id.as_deref())
            .flatten()
    }

    /// Tells the nodes where the keyboard is when it moved since they were
    /// last told, by a key or not: a Herdr focus move, a close focusing a
    /// neighbour, the editor taking the keyboard. A node then reads the next
    /// key into another pane as moving the keyboard back (B1), however long
    /// the operator kept typing there.
    pub(crate) fn sync_terminal_focus(&mut self) {
        let focus = self.terminal_keyboard_pane();
        if focus == self.terminal_intents.focus.as_deref() {
            return;
        }
        let focus = focus.map(str::to_owned);
        self.terminal_intents.focus = focus.clone();
        self.terminals
            .control(TerminalControl::Focus { pane: focus });
    }

    /// What the nodes reported about their panes since the last batch.
    /// Returns whether the snapshot changed.
    pub fn ingest_terminal_reports(&mut self, reports: Vec<TerminalReport>) -> bool {
        let mut changed = false;
        for report in reports {
            changed |= self.ingest_terminal_report(report);
        }
        changed
    }

    fn ingest_terminal_report(&mut self, report: TerminalReport) -> bool {
        match report {
            TerminalReport::State { pane, state } => {
                // A pane Herdr no longer lists is not brought back by a late
                // report from its node.
                if !self.pane_still_terminal(&pane) {
                    return false;
                }
                if state.state == "controlling" {
                    // This client's own wheel reaches a controlled pane.
                    self.panes_scroll_held.remove(&pane);
                }
                if state.state == "closing" && !self.close_operation_holds_pane(&pane) {
                    self.panes_closing.remove(&pane);
                }
                let generation = state.generation;
                let session = matches!(state.state.as_str(), "controlling" | "observing");
                if self.terminal_states.get(&pane) == Some(&state) {
                    return false;
                }
                self.terminal_states.insert(pane.clone(), state);
                if session {
                    self.adopt_attachment_terminal(&pane, generation);
                    // A file dropped before this session attached is written now.
                    self.reconcile_attachment_target();
                }
                self.ensure_terminal_pane(&pane);
                self.sync_transport_projection(&pane);
                true
            }
            TerminalReport::FirstFrame { pane, .. } => {
                self.release_wheel_before_attach(&pane);
                false
            }
            TerminalReport::FrameShown { pane, at_unix_ms } => {
                let age = unix_milliseconds().saturating_sub(at_unix_ms);
                let at = Instant::now()
                    .checked_sub(std::time::Duration::from_millis(age))
                    .unwrap_or_else(Instant::now);
                // A grid change's record counts only a frame after its
                // resize went out; a frame before that is another watch's.
                self.op_timings.note_frame(&pane, at);
                false
            }
            TerminalReport::Input {
                pane,
                at_unix_ms,
                submitted,
                focus,
            } => {
                // A pane the core does not project is neither typed into
                // nor focused by a node's word, a device's least of all.
                if !self.pane_still_terminal(&pane) {
                    return false;
                }
                // The time comes from a screen's or a device's clock: one
                // ahead of this one would read as a draft typed forever.
                self.note_delivery_key_at(&pane, at_unix_ms.min(unix_milliseconds()));
                if submitted {
                    self.record_operator_submit(&pane);
                }
                if focus && self.terminal_keyboard_pane() != Some(pane.as_str()) {
                    self.snapshot.focused.surface = Surface::Terminal;
                    self.snapshot.focused.pane_id = Some(pane.clone());
                    self.snapshot.terminal.pane_id = Some(pane.clone());
                    self.ensure_terminal_pane(&pane);
                    self.sync_focused_terminal_projection();
                    return true;
                }
                false
            }
            TerminalReport::Error {
                pane: _,
                kind,
                message,
            } => {
                self.set_error(kind, message, true);
                true
            }
            TerminalReport::Note {
                pane: _,
                kind,
                message,
            } => {
                self.push_diagnostic(kind, message);
                true
            }
            TerminalReport::RequestDiscarded { request, reason } => {
                if self.input_requests.discarded_by_node(&request) {
                    self.push_diagnostic(
                        "terminal.input_discarded",
                        format!("Request {request}: {reason}"),
                    );
                    self.set_error("terminal.input_hold_failed",
                        "Queued input could not be delivered. Type it again once the pane is ready.", true);
                    self.sync_input_requests();
                    return true;
                }
                false
            }
            TerminalReport::AttachmentInput {
                intent,
                outcome,
                bytes,
            } => self.attachment_input_refused(&intent, outcome, bytes),
            TerminalReport::AttachmentDelivered { intent, written } => {
                self.attachment_delivered(&intent, written)
            }
        }
    }

    /// Whether the core still projects a terminal for `pane`: one of this
    /// machine's panes in a layout or named by a creation, or a connected
    /// device's pane.
    fn pane_still_terminal(&self, pane: &str) -> bool {
        if pane.starts_with("remote:") {
            return self
                .snapshot
                .terminal
                .panes
                .iter()
                .any(|projected| projected.pane_id == pane);
        }
        self.snapshot.pane_layouts.is_empty()
            || self.layout_holding_pane(pane).is_some()
            || self.input_requests.awaits_layout(pane)
    }
    /// Writes bytes the core makes itself (a click's mouse report) to the
    /// pane, through its node's writer like every key.
    pub(super) fn write_terminal_control(&mut self, pane_id: &str, bytes: &[u8]) {
        if self.close_operation_holds_pane(pane_id) {
            self.set_error(
                "terminal.close_pending",
                format!("Pane {pane_id} is closing; input was not sent"),
                true,
            );
            return;
        }
        self.terminals.control(TerminalControl::Write {
            pane: pane_id.to_owned(),
            data: hide_node_link::terminal::encode_base64(bytes),
        });
    }
}
