//! Agent sleep inside the runtime: the minute decision, the visit that wakes,
//! the three events, and the results the Herdr workers hand back (PRD
//! agent-sleep). What is decided and remembered lives in
//! `crate::agent_sleep`; what touches Herdr and processes lives in
//! `crate::agent_sleep_herdr`. Nothing here blocks: every end and every wake
//! runs on a worker and comes back through `ingest_agent_sleep_*`.

use serde::Deserialize;

use super::*;
use crate::agent_sleep::{
    DECISION_INTERVAL_MS, SLEEP_RETRY_BACKOFF_MS, Settled, SleepMachine, SleepPhase, SleepRecord,
    SleepScope, WakeMode, valid_after_hours,
};
use crate::agent_sleep_herdr::{self, SleepTarget, WakeOutcome, WakeRequest};

mod dormant;
pub(in crate::runtime) use dormant::WOKEN_BINDING_LIMIT_MS;

/// Ends in flight at once. A first enable over a long day's panes would
/// otherwise start one worker per idle agent; the rest wait for the next
/// minute's decision.
const MAX_ENDS_IN_FLIGHT: usize = 4;

/// A pane whose agent would not end: left alone until `until`, unless its
/// state changes first, which `changed_at_unix_ms` tells (B8).
#[derive(Clone, Copy, Debug)]
pub(super) struct Backoff {
    until_unix_ms: u64,
    changed_at_unix_ms: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentSleepSetPayload {
    pub(super) after_hours: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentWakePayload {
    pub(super) pane_id: String,
    /// Start new session rather than resuming the conversation (B14).
    #[serde(default)]
    pub(super) fresh: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SleepingSessionPayload {
    pub(super) sleep_id: crate::agent_sleep::SleepId,
}

impl Runtime {
    /// The minute decision (B4-B6) for this core's machine, run by the local
    /// coordinator's tick. A tick inside the minute is one integer comparison.
    /// A core waiting for its move's link puts nothing to sleep.
    pub(crate) fn tick_agent_sleep(&mut self, now_unix_ms: u64) -> bool {
        if now_unix_ms < self.agent_sleep_next_decision_unix_ms
            || self.live.is_none()
            || self.effects_held()
        {
            return false;
        }
        self.agent_sleep_next_decision_unix_ms = now_unix_ms.saturating_add(DECISION_INTERVAL_MS);
        let changed =
            self.expire_dormant_confirmation(now_unix_ms) | self.fail_unstarted_wakes(now_unix_ms);
        self.decide_agent_sleep(None, now_unix_ms) | changed
    }

    /// Fails each wake whose agent has not appeared in its pane by the
    /// start's deadline, with the reason a start that never got ready gives.
    /// Herdr answers `agent.start` once it has typed the start line, so a
    /// line the pane's shell never ran (a Windows prompt reading a leftover
    /// bracketed paste as keys) would leave the row waking for good.
    fn fail_unstarted_wakes(&mut self, now_unix_ms: u64) -> bool {
        let records = &self.snapshot.ui_state.agent_sleep.records;
        self.agent_wake_started.retain(|pane_id, _| {
            records
                .get(pane_id)
                .is_some_and(|record| record.phase == SleepPhase::Waking)
        });
        let deadline_ms =
            u64::try_from(crate::agent_start::AGENT_START_DEADLINE.as_millis()).unwrap_or(u64::MAX);
        let unstarted = self
            .agent_wake_started
            .iter()
            .filter(|(_, started)| now_unix_ms.saturating_sub(**started) >= deadline_ms)
            .map(|(pane_id, _)| pane_id.clone())
            .collect::<Vec<_>>();
        for pane_id in &unstarted {
            self.agent_wake_started.remove(pane_id);
            if let Some(record) = self.snapshot.ui_state.agent_sleep.records.get_mut(pane_id) {
                let reason = "The agent did not become ready in time.".to_owned();
                record.phase = SleepPhase::Failed;
                record.reason = Some(reason.clone());
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_sleep",
                    "kind": "agent_sleep.wake_failed",
                    "pane_id": pane_id,
                    "provider": record.kind,
                    "reason": reason,
                    "detail": format!(
                        "no agent appeared in the pane within {} s of Herdr taking the start",
                        deadline_ms / 1000
                    ),
                }));
            }
            self.refresh_sleep_rows_of(pane_id);
        }
        if unstarted.is_empty() {
            return false;
        }
        self.persist_ui_state();
        true
    }

    /// The same decision for a node that dials this core, run by that node's
    /// coordinator's tick (PRD core-host-node-remote-core B13). A device the
    /// core dials is never decided on.
    pub(crate) fn tick_node_agent_sleep(&mut self, device: &str, now_unix_ms: u64) -> bool {
        if self
            .node_sleep_next_decision_unix_ms
            .get(device)
            .is_some_and(|next| now_unix_ms < *next)
            || self.sleep_machine(device) != SleepMachine::Node
            || self.remote_herdr_api(device).is_none()
            || self.effects_held()
        {
            return false;
        }
        self.node_sleep_next_decision_unix_ms.insert(
            device.to_owned(),
            now_unix_ms.saturating_add(DECISION_INTERVAL_MS),
        );
        self.decide_agent_sleep(Some(device), now_unix_ms)
    }

    /// Puts to sleep what is due on one machine: the core's own (`None`) or a
    /// node's. At most `MAX_ENDS_IN_FLIGHT` ends run at once over all of them.
    fn decide_agent_sleep(&mut self, device: Option<&str>, now_unix_ms: u64) -> bool {
        let mut changed = false;
        let machine = device.map_or(SleepMachine::Core, |device| self.sleep_machine(device));
        let on_screen = self.agent_sleep_on_screen(device);
        let store = &mut self.snapshot.ui_state.agent_sleep;
        let seen = store.mark_seen(on_screen.iter().map(String::as_str), now_unix_ms);
        self.agent_sleep_backoff.retain(|pane_id, backoff| {
            backoff.until_unix_ms > now_unix_ms
                && store
                    .stamps
                    .get(pane_id)
                    .is_some_and(|stamps| stamps.changed_at_unix_ms == backoff.changed_at_unix_ms)
        });
        let backoff_until = self
            .agent_sleep_backoff
            .iter()
            .map(|(pane_id, backoff)| (pane_id.clone(), backoff.until_unix_ms))
            .collect::<HashMap<_, _>>();
        let store = &self.snapshot.ui_state.agent_sleep;
        let due = store.due(
            self.sleep_agents(device),
            machine,
            &on_screen,
            self.snapshot.ui_state.agent_sleep_after_hours,
            &backoff_until,
            now_unix_ms,
        );
        let in_flight = store
            .records
            .values()
            .filter(|record| record.phase == SleepPhase::Ending)
            .count();
        let dormant_in_flight = store
            .dormant
            .values()
            .filter(|record| record.phase.in_flight())
            .count();
        for pane_id in due
            .into_iter()
            .take(MAX_ENDS_IN_FLIGHT.saturating_sub(in_flight + dormant_in_flight))
        {
            changed |= self.begin_agent_sleep(&pane_id, "idle", now_unix_ms);
        }
        if seen && !changed {
            self.persist_ui_state();
        }
        changed
    }

    /// Reconciles the records with a fresh local session before it is
    /// projected, and adds the rows of sleeping agents Herdr stopped listing.
    pub(super) fn settle_agent_sleep(&mut self, payload: &mut SessionSnapshotPayload) {
        self.settle_agent_sleep_in(payload, SleepScope::Core);
    }

    /// The same for a node that dials this core, on its session as its Herdr
    /// names it, before the session is scoped to the device.
    pub(crate) fn settle_node_agent_sleep(
        &mut self,
        device: &str,
        payload: &mut SessionSnapshotPayload,
    ) {
        if self.sleep_machine(device) == SleepMachine::Node {
            let prefix = remote_pane_id_prefix(device);
            self.settle_agent_sleep_in(payload, SleepScope::Device(&prefix));
        }
    }

    fn settle_agent_sleep_in(
        &mut self,
        payload: &mut SessionSnapshotPayload,
        scope: SleepScope<'_>,
    ) {
        let settled = self
            .snapshot
            .ui_state
            .agent_sleep
            .settle_payload(payload, scope);
        if settled.is_empty() {
            return;
        }
        for event in &settled {
            let (pane_id, kind, phase) = match event {
                Settled::Woke { pane_id, phase } => (pane_id, "agent_sleep.awake", Some(*phase)),
                Settled::Closed { pane_id } => (pane_id, "agent_sleep.closed", None),
            };
            crate::diagnostic!(serde_json::json!({
                "component": "agent_sleep",
                "kind": kind,
                "pane_id": pane_id,
                "phase": phase,
            }));
            self.agent_sleep_backoff.remove(pane_id);
        }
        self.persist_ui_state();
        // A woken agent's pane takes keys again (B16).
        self.sync_terminal_intents();
    }

    /// Marks the projected rows that sleep and moves each awake agent's
    /// change stamp. The stamps are persisted, never published.
    pub(super) fn stamp_agent_sleep(&mut self, agents: &mut [SidebarAgentSnapshot]) {
        self.confirm_dormant_wake(agents);
        self.stamp_agent_sleep_in(agents, SleepScope::Core);
    }

    /// The same for the rows of a node that dials this core.
    pub(super) fn stamp_node_agent_sleep(
        &mut self,
        device: &str,
        agents: &mut [SidebarAgentSnapshot],
    ) {
        if self.sleep_machine(device) == SleepMachine::Node {
            let prefix = remote_pane_id_prefix(device);
            self.stamp_agent_sleep_in(agents, SleepScope::Device(&prefix));
        }
    }

    fn stamp_agent_sleep_in(&mut self, agents: &mut [SidebarAgentSnapshot], scope: SleepScope<'_>) {
        let store = &mut self.snapshot.ui_state.agent_sleep;
        store.annotate(agents);
        if store.stamp(agents, scope, unix_milliseconds()) {
            self.persist_ui_state();
        }
    }

    /// A node's Herdr moved the tab on its screen: a visit, as a committed
    /// local one is (B12), once the node's session confirms it. Wakes what
    /// sleeps in the tabs that came on screen and stamps the last look of
    /// those and of the tabs that left.
    pub(super) fn node_agent_sleep_visit(&mut self, device: &str) -> bool {
        if self.sleep_machine(device) != SleepMachine::Node {
            return false;
        }
        let shown = self.shown_node_tabs(device);
        let before = self
            .node_sleep_shown_tabs
            .insert(device.to_owned(), shown.clone())
            .unwrap_or_default();
        if before == shown {
            return false;
        }
        let entered = shown
            .iter()
            .filter(|tab| !before.contains(tab))
            .flat_map(|tab| self.agent_sleep_tab_panes(tab))
            .collect::<Vec<_>>();
        let looked = before
            .iter()
            .filter(|tab| !shown.contains(tab))
            .flat_map(|tab| self.agent_sleep_tab_panes(tab))
            .chain(entered.iter().cloned())
            .collect::<Vec<_>>();
        self.wake_visited(entered, looked)
    }

    /// A committed visit that changed the tab on screen wakes that tab's
    /// sleeping agents (B12, D-12) and stamps the last look of both tabs.
    /// A preview sends no event, so it never gets here.
    pub(super) fn agent_sleep_visit(&mut self, left_tab_id: Option<&str>) -> bool {
        let Some(entered_tab_id) = self.focused_visible_tab_id() else {
            return false;
        };
        if left_tab_id == Some(entered_tab_id.as_str()) || !self.device_in_front(self.node.as_str())
        {
            return false;
        }
        let entered = self.agent_sleep_tab_panes(&entered_tab_id);
        let mut looked = entered.clone();
        if let Some(left) = left_tab_id {
            looked.extend(self.agent_sleep_tab_panes(left));
        }
        self.wake_visited(entered, looked)
    }

    fn wake_visited(&mut self, entered: Vec<String>, looked: Vec<String>) -> bool {
        let mut persist = self
            .snapshot
            .ui_state
            .agent_sleep
            .mark_seen(looked.iter().map(String::as_str), unix_milliseconds());
        let mut changed = false;
        for pane_id in entered {
            if self.failed_terminal_id(&pane_id).is_some() {
                continue;
            }
            match self.snapshot.ui_state.agent_sleep.records.get_mut(&pane_id) {
                Some(record) if record.phase == SleepPhase::Ending => {
                    record.pending_wake = Some(WakeMode::Resume);
                    persist = true;
                }
                Some(record) if record.phase == SleepPhase::Sleeping => {
                    changed |= self.begin_agent_wake(&pane_id, WakeMode::Resume, "visit");
                }
                _ => {}
            }
        }
        if persist && !changed {
            self.persist_ui_state();
        }
        changed
    }

    pub(super) fn set_agent_sleep_after(&mut self, payload: AgentSleepSetPayload) -> bool {
        if !valid_after_hours(payload.after_hours) {
            self.set_error(
                "agent_sleep.invalid_setting",
                format!(
                    "Sleep idle agents after {:?} hours is not one of the choices",
                    payload.after_hours
                ),
                false,
            );
            return true;
        }
        if self.snapshot.ui_state.agent_sleep_after_hours == payload.after_hours {
            return false;
        }
        self.snapshot.ui_state.agent_sleep_after_hours = payload.after_hours;
        // A new setting is decided on at the next tick, not a minute later.
        self.agent_sleep_next_decision_unix_ms = 0;
        self.node_sleep_next_decision_unix_ms.clear();
        crate::diagnostic!(serde_json::json!({
            "component": "agent_sleep",
            "kind": "agent_sleep.setting",
            "after_hours": payload.after_hours,
        }));
        self.persist_ui_state();
        true
    }

    /// Sleep agent from the pane menu (B15): the same end, without the time
    /// or the screen, because the person asking has already looked.
    pub(super) fn request_agent_sleep(&mut self, pane_id: &str) -> bool {
        let device = pane_device(pane_id);
        let machine = self.sleep_machine_of_pane(pane_id);
        let refusal = match self
            .sleep_agents(device)
            .iter()
            .find(|agent| agent.pane_id == pane_id)
        {
            None => Some("This pane has no agent"),
            Some(_)
                if self
                    .snapshot
                    .ui_state
                    .agent_sleep
                    .records
                    .contains_key(pane_id) =>
            {
                Some("This agent is already asleep")
            }
            Some(agent) => crate::agent_sleep::sleep_refusal(agent, machine),
        };
        if let Some(reason) = refusal {
            self.set_error("agent_sleep.refused", reason, false);
            return true;
        }
        let reachable = match device {
            None => self.live.is_some(),
            Some(device) => self.remote_herdr_api(device).is_some(),
        };
        if !reachable {
            self.set_error(
                "pane.control_unavailable",
                "Putting an agent to sleep requires a live Herdr connection",
                true,
            );
            return true;
        }
        self.begin_agent_sleep(pane_id, "manual", unix_milliseconds())
    }

    /// Wake agent, Retry, and Start new session (B12-B14). A wake asked for
    /// while the end is in flight runs when it lands; a second one while a
    /// wake is running is the same intent and starts nothing.
    pub(super) fn request_agent_wake(&mut self, payload: AgentWakePayload) -> bool {
        let mode = if payload.fresh {
            WakeMode::Fresh
        } else {
            WakeMode::Resume
        };
        let Some(record) = self
            .snapshot
            .ui_state
            .agent_sleep
            .records
            .get_mut(&payload.pane_id)
        else {
            self.set_error(
                "agent_sleep.not_asleep",
                format!("Pane {} has no sleeping agent", payload.pane_id),
                false,
            );
            return true;
        };
        match record.phase {
            SleepPhase::Waking => false,
            SleepPhase::Ending => {
                record.pending_wake = Some(mode);
                self.persist_ui_state();
                false
            }
            SleepPhase::Sleeping | SleepPhase::Failed => {
                self.begin_agent_wake(&payload.pane_id, mode, "request")
            }
        }
    }

    /// The end worker's answer. A refusal or a timeout leaves the agent awake
    /// and the pane alone for an hour (B8).
    pub(crate) fn ingest_agent_sleep_end(
        &mut self,
        pane_id: &str,
        result: Result<u64, String>,
    ) -> bool {
        let now = unix_milliseconds();
        let store = &mut self.snapshot.ui_state.agent_sleep;
        let Some(record) = store.records.get_mut(pane_id) else {
            // The pane closed while its agent was ending.
            crate::diagnostic!(serde_json::json!({
                "component": "agent_sleep",
                "kind": "agent_sleep.slept",
                "pane_id": pane_id,
                "outcome": "pane_gone",
            }));
            return false;
        };
        let provider = record.kind.clone();
        match result {
            Ok(state_change_seq) => {
                record.phase = SleepPhase::Sleeping;
                record.state_change_seq = Some(state_change_seq);
                record.since_unix_ms = now;
                let pending_wake = record.pending_wake.take();
                // Keys for the sleeping pane are dropped at its node from now.
                self.sync_terminal_intents();
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_sleep",
                    "kind": "agent_sleep.slept",
                    "pane_id": pane_id,
                    "provider": provider,
                    "outcome": "slept",
                }));
                if let Some(mode) = pending_wake {
                    self.begin_agent_wake(pane_id, mode, "visit_during_end");
                } else {
                    self.persist_ui_state();
                    self.refresh_sleep_rows_of(pane_id);
                }
                true
            }
            Err(reason) => {
                store.records.remove(pane_id);
                // The agent is awake still, so its pane takes input again.
                self.sync_terminal_intents();
                let store = &mut self.snapshot.ui_state.agent_sleep;
                let changed_at_unix_ms = store
                    .stamps
                    .get(pane_id)
                    .map_or(0, |stamps| stamps.changed_at_unix_ms);
                self.agent_sleep_backoff.insert(
                    pane_id.to_owned(),
                    Backoff {
                        until_unix_ms: now.saturating_add(SLEEP_RETRY_BACKOFF_MS),
                        changed_at_unix_ms,
                    },
                );
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_sleep",
                    "kind": "agent_sleep.sleep_failed",
                    "pane_id": pane_id,
                    "provider": provider,
                    "outcome": "left_awake",
                    "reason": reason,
                }));
                self.persist_ui_state();
                false
            }
        }
    }

    /// The wake worker's answer. A started agent is confirmed by the session
    /// update that lists it, which is what clears the record (B16).
    pub(crate) fn ingest_agent_wake(
        &mut self,
        pane_id: &str,
        mode: WakeMode,
        outcome: WakeOutcome,
    ) -> bool {
        let Some(record) = self.snapshot.ui_state.agent_sleep.records.get_mut(pane_id) else {
            crate::diagnostic!(serde_json::json!({
                "component": "agent_sleep",
                "kind": "agent_sleep.woke",
                "pane_id": pane_id,
                "mode": mode,
                "outcome": "confirmed_before_answer",
            }));
            return false;
        };
        let provider = record.kind.clone();
        match outcome {
            WakeOutcome::Started => {
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_sleep",
                    "kind": "agent_sleep.woke",
                    "pane_id": pane_id,
                    "provider": provider,
                    "mode": mode,
                    "outcome": "started",
                }));
                self.agent_wake_started
                    .insert(pane_id.to_owned(), unix_milliseconds());
                false
            }
            WakeOutcome::Failed { reason, detail } => {
                record.phase = SleepPhase::Failed;
                record.reason = Some(reason.clone());
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_sleep",
                    "kind": "agent_sleep.wake_failed",
                    "pane_id": pane_id,
                    "provider": provider,
                    "mode": mode,
                    "reason": reason,
                    "detail": detail,
                }));
                self.persist_ui_state();
                self.refresh_sleep_rows_of(pane_id);
                true
            }
        }
    }

    fn begin_agent_sleep(&mut self, pane_id: &str, trigger: &str, now_unix_ms: u64) -> bool {
        if self.pane_move_holds_pane(pane_id) {
            return false;
        }
        let Some(context) = self.sleep_target(pane_id) else {
            return false;
        };
        let Some(agent) = self
            .sleep_agents(pane_device(pane_id))
            .iter()
            .find(|agent| agent.pane_id == pane_id)
        else {
            return false;
        };
        // Only the core's own machine gets here with one (`sleep_refusal`).
        if crate::agent_sleep::closes_pane_when_sleeping(&agent.agent_kind) {
            return self.begin_dormant_sleep(pane_id, now_unix_ms);
        }
        let cwd = self.agent_sleep_pane_cwd(pane_id);
        let Some(mut record) = SleepRecord::ending(agent, cwd, now_unix_ms) else {
            return false;
        };
        record.launch = self.launch_choice_of(agent);
        let kind = record.kind.clone();
        crate::diagnostic!(serde_json::json!({
            "component": "agent_sleep",
            "kind": "agent_sleep.ending",
            "pane_id": pane_id,
            "provider": kind,
            "trigger": trigger,
        }));
        self.snapshot
            .ui_state
            .agent_sleep
            .records
            .insert(pane_id.to_owned(), record);
        // Closed before the worker starts: from here no writer's input can
        // land behind the end, and the worker waits for those already writing.
        self.sync_terminal_intents();
        if let Err(message) = agent_sleep_herdr::spawn_end(context, pane_id.to_owned(), kind) {
            return self.ingest_agent_sleep_end(pane_id, Err(message));
        }
        self.persist_ui_state();
        // The pane is still awake and drawn so until the end lands (B8).
        false
    }

    fn begin_agent_wake(&mut self, pane_id: &str, mode: WakeMode, trigger: &str) -> bool {
        if self.failed_terminal_id(pane_id).is_some() {
            return false;
        }
        let Some(context) = self.sleep_target(pane_id) else {
            return false;
        };
        let codex_daemon = self.codex_daemon_for_pane(pane_id);
        let Some(record) = self.snapshot.ui_state.agent_sleep.records.get_mut(pane_id) else {
            return false;
        };
        record.phase = SleepPhase::Waking;
        record.reason = None;
        record.pending_wake = None;
        // An agent that slept before it reported a conversation has none to
        // resume, and wakes as a new one in the same checkout (B24).
        let mode = if record.session_id.is_none() {
            WakeMode::Fresh
        } else {
            mode
        };
        let launch =
            match crate::runtime::agent_choice::launch_arguments(&record.kind, &record.launch) {
                Ok(launch) => launch,
                Err(detail) => {
                    return self.ingest_agent_wake(
                    pane_id,
                    mode,
                    WakeOutcome::Failed {
                        reason:
                            "The model or effort it was started with is not one this agent takes."
                                .to_owned(),
                        detail,
                    },
                );
                }
            };
        let choice = record.launch.clone();
        let mut args = match mode {
            WakeMode::Resume => {
                let Some(args) = crate::recent_closed::resume_arguments(&ClosedAgent {
                    kind: record.kind.clone(),
                    session_id: record.session_id.clone(),
                    ..Default::default()
                }) else {
                    // Only Claude and Codex sleep, so this is a record this
                    // build cannot resume; starting it bare would lose the
                    // conversation without saying so.
                    let detail = format!("{} has no resume arguments", record.kind);
                    return self.ingest_agent_wake(
                        pane_id,
                        mode,
                        WakeOutcome::Failed {
                            reason: "The conversation couldn\u{2019}t be resumed.".to_owned(),
                            detail,
                        },
                    );
                };
                args
            }
            WakeMode::Fresh => Vec::new(),
        };
        args.extend(launch);
        let request = WakeRequest {
            pane_id: pane_id.to_owned(),
            kind: record.kind.clone(),
            name: record
                .agent_name
                .clone()
                .unwrap_or_else(|| crate::fork::wake_name(&context.herdr_pane_id)),
            mode,
            args,
            cwd: record.cwd.clone(),
            session_id: record.session_id.clone(),
            codex_daemon,
        };
        crate::diagnostic!(serde_json::json!({
            "component": "agent_sleep",
            "kind": "agent_sleep.waking",
            "pane_id": pane_id,
            "provider": request.kind,
            "mode": mode,
            "trigger": trigger,
        }));
        // The pane's record follows the agent the wake starts: the resumed
        // conversation, or the new one it reports.
        let (kind, name, session) = (
            request.kind.clone(),
            request.name.clone(),
            request.session_id.clone(),
        );
        if let Err(message) = agent_sleep_herdr::spawn_wake(context, request) {
            return self.ingest_agent_wake(
                pane_id,
                mode,
                WakeOutcome::Failed {
                    reason: "The agent did not become ready in time.".to_owned(),
                    detail: message,
                },
            );
        }
        let session = (mode == WakeMode::Resume).then_some(session).flatten();
        self.record_pane_launch(pane_id, &kind, &name, &choice, session.as_deref());
        self.persist_ui_state();
        self.refresh_sleep_rows_of(pane_id);
        true
    }

    /// Draws a record's new phase on the rows of the machine the pane is on.
    fn refresh_sleep_rows_of(&mut self, pane_id: &str) -> bool {
        match pane_device(pane_id) {
            None => self.refresh_agent_sleep_rows(),
            Some(device) => self.refresh_node_sleep_rows(device),
        }
    }

    /// The same for a node's published session; its next session update
    /// draws it again from the records.
    fn refresh_node_sleep_rows(&mut self, device: &str) -> bool {
        let machine = self.sleep_machine(device);
        let store = &self.snapshot.ui_state.agent_sleep;
        let Some(session) = self
            .snapshot
            .status
            .remote
            .iter_mut()
            .find(|status| status.target_id == device)
            .and_then(|status| status.session.as_mut())
        else {
            return false;
        };
        let mut agents = session.agents.clone();
        store.annotate(&mut agents);
        for agent in &mut agents {
            crate::agent_state::rederive(agent);
        }
        if agents == session.agents {
            return false;
        }
        sync_pane_status(
            &mut session.workspaces,
            &agents,
            session.focused_pane_id.as_deref(),
            machine,
        );
        session.agents = agents;
        true
    }

    /// Draws a record's new phase on its row and pane without waiting for
    /// the next session update.
    fn refresh_agent_sleep_rows(&mut self) -> bool {
        let mut agents = self.snapshot.navigator.agents.clone();
        self.snapshot.ui_state.agent_sleep.annotate(&mut agents);
        for agent in &mut agents {
            crate::agent_state::rederive(agent);
        }
        if agents == self.snapshot.navigator.agents {
            return false;
        }
        self.sync_pane_status_from_agents(&agents);
        self.snapshot.navigator.agents = agents;
        // A pane's child chips carry the child's mark.
        self.sync_pane_lineage();
        true
    }

    /// The panes of the tabs on one machine's screen; empty while another
    /// device is in front, because none of this machine's tabs is then.
    fn agent_sleep_on_screen(&self, device: Option<&str>) -> HashSet<String> {
        let tabs = match device {
            None if self.device_in_front(self.node.as_str()) => self.shown_agent_tabs(),
            None => Vec::new(),
            Some(device) => self.shown_node_tabs(device),
        };
        tabs.iter()
            .flat_map(|tab_id| self.agent_sleep_tab_panes(tab_id))
            .collect()
    }

    /// The tabs of a node's session on screen: its Agent areas' while it is
    /// in front with them, else the tab its Herdr has in front.
    fn shown_node_tabs(&self, device: &str) -> Vec<String> {
        if !self.device_in_front(device) {
            return Vec::new();
        }
        if let Some(key) = self.front_workspace_key().filter(|key| key.0 == device)
            && let Some(layout) = self.agent_layout_of(&key)
        {
            return self.shown_agent_layout(&key, layout).shown();
        }
        let Some(session) = self.node_session(device) else {
            return Vec::new();
        };
        session
            .focused_tab_id
            .clone()
            .or_else(|| {
                session
                    .focused_checkout_id
                    .as_deref()
                    .and_then(|checkout| session.active_tab_ids.get(checkout))
                    .cloned()
            })
            .into_iter()
            .collect()
    }

    /// The agents of `device`'s panes as the core holds them: this
    /// machine's own, or the session a node or a device reports.
    pub(super) fn machine_agents(&self, device: &str) -> &[SidebarAgentSnapshot] {
        if device == self.node.as_str() {
            return &self.snapshot.navigator.agents;
        }
        self.node_session(device)
            .map_or(&[], |session| session.agents.as_slice())
    }

    fn node_session(&self, device: &str) -> Option<&RemoteSessionSnapshot> {
        self.snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == device)
            .and_then(|status| status.session.as_ref())
    }

    /// The agents of one machine: the core's own, or a device's session's.
    fn sleep_agents(&self, device: Option<&str>) -> &[SidebarAgentSnapshot] {
        match device {
            None => &self.snapshot.navigator.agents,
            Some(device) => self
                .node_session(device)
                .map_or(&[], |session| session.agents.as_slice()),
        }
    }

    fn sleep_workspaces(&self, device: Option<&str>) -> &[WorkspaceSnapshot] {
        match device {
            None => &self.snapshot.navigator.workspaces,
            Some(device) => self
                .node_session(device)
                .map_or(&[], |session| session.workspaces.as_slice()),
        }
    }

    /// How sleep treats a device: this core's own machine, a node that dials
    /// it, or a device it dials.
    pub(super) fn sleep_machine(&self, device: &str) -> SleepMachine {
        if device == self.node.as_str() {
            SleepMachine::Core
        } else if self.link_origin(device) == Some(&LinkOrigin::Inbound) {
            SleepMachine::Node
        } else {
            SleepMachine::Device
        }
    }

    pub(super) fn sleep_machine_of_pane(&self, pane_id: &str) -> SleepMachine {
        pane_device(pane_id).map_or(SleepMachine::Core, |device| self.sleep_machine(device))
    }

    /// Where a pane's sleep or wake runs: the core's own Herdr and node, or
    /// those of the node that dials the core with the pane.
    fn sleep_target(&self, pane_id: &str) -> Option<SleepTarget> {
        let Some(device) = pane_device(pane_id) else {
            let live = self.live.as_ref()?;
            return Some(SleepTarget {
                api_connector: Arc::clone(&live.api_connector),
                node: Arc::clone(&live.node),
                runtime: live.runtime.clone(),
                notifier: live.notifier.clone(),
                herdr_pane_id: pane_id.to_owned(),
                input_gate: Arc::clone(&self.input_gate),
            });
        };
        if self.sleep_machine(device) != SleepMachine::Node {
            return None;
        }
        let worker = self.worker_context.as_ref()?;
        Some(SleepTarget {
            api_connector: self.remote_herdr_api(device)?,
            node: super::inbound::InboundLink::for_device(device, worker.runtime.clone()),
            runtime: worker.runtime.clone(),
            notifier: worker.notifier.clone(),
            herdr_pane_id: remote_pane_source_id(device, pane_id)?.to_owned(),
            input_gate: Arc::clone(&self.input_gate),
        })
    }

    /// Forgets what sleep kept of a device's panes once the device is
    /// removed; its panes are not touched.
    pub(super) fn forget_device_sleep(&mut self, device: &str) {
        let prefix = remote_pane_id_prefix(device);
        let store = &mut self.snapshot.ui_state.agent_sleep;
        store
            .records
            .retain(|pane_id, _| !pane_id.starts_with(&prefix));
        self.sync_input_gate();
        let store = &mut self.snapshot.ui_state.agent_sleep;
        store
            .stamps
            .retain(|pane_id, _| !pane_id.starts_with(&prefix));
        self.agent_sleep_backoff
            .retain(|pane_id, _| !pane_id.starts_with(&prefix));
        self.node_sleep_next_decision_unix_ms.remove(device);
        self.node_sleep_shown_tabs.remove(device);
    }

    fn agent_sleep_tab_panes(&self, tab_id: &str) -> Vec<String> {
        let device = tab_id
            .strip_prefix("remote:")
            .and_then(|rest| rest.split_once(":tab:"))
            .map(|(device, _)| device);
        self.sleep_workspaces(device)
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .filter(|tab| tab.id.as_deref() == Some(tab_id))
            .flat_map(|tab| tab.panes.iter().map(|pane| pane.id.clone()))
            .collect()
    }

    fn agent_sleep_pane_cwd(&self, pane_id: &str) -> Option<String> {
        self.sleep_workspaces(pane_device(pane_id))
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .find(|pane| pane.id == pane_id)
            .map(|pane| pane.cwd.clone())
            .filter(|cwd| !cwd.trim().is_empty())
    }
}

/// The device a pane belongs to, when it is not this core's own.
pub(super) fn pane_device(pane_id: &str) -> Option<&str> {
    pane_id
        .strip_prefix("remote:")
        .and_then(|rest| rest.split_once(":pane:"))
        .map(|(device, _)| device)
}
