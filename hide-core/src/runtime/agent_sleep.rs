//! Agent sleep inside the runtime: the minute decision, the visit that wakes,
//! the three events, and the results the engine workers hand back (PRD
//! agent-sleep). What is decided and remembered lives in
//! `crate::agent_sleep`; what touches the engine and processes lives in
//! `crate::agent_sleep_engine`. Nothing here blocks: every end and every wake
//! runs on a worker and comes back through `ingest_agent_sleep_*`.

use serde::Deserialize;

use super::*;
use crate::agent_sleep::{
    Awake, DECISION_INTERVAL_MS, LetterWaits, SLEEP_RETRY_BACKOFF_MS, Settled, SleepCause,
    SleepMachine, SleepPhase, SleepRecord, SleepScope, WakeMode, valid_after_hours,
};
use crate::agent_sleep_engine::{self, SleepTarget, WakeOutcome, WakeRequest};
use crate::model::AgentActionRefusal;

mod command;
mod dormant;
pub(in crate::runtime) mod letters;
pub(in crate::runtime) mod missions;
use dormant::SleepRefusal;
pub(in crate::runtime) use dormant::{Takeover, WOKEN_BINDING_LIMIT_MS};

/// Ends in flight at once. A first enable over a long day's panes would
/// otherwise start one worker per idle agent; the rest wait for the next
/// minute's decision.
const MAX_ENDS_IN_FLIGHT: usize = 4;

/// The ends that may still start now: `MAX_ENDS_IN_FLIGHT` less every end
/// that runs, the idle and mission passes' alike, so both share one cap.
fn ends_left(store: &crate::agent_sleep::AgentSleepStore) -> usize {
    let in_flight = store
        .records
        .values()
        .filter(|record| record.phase == SleepPhase::Ending)
        .count()
        + store
            .dormant
            .values()
            .filter(|record| record.phase.in_flight())
            .count();
    MAX_ENDS_IN_FLIGHT.saturating_sub(in_flight)
}

/// What asking for one agent's end did.
enum SleepStart {
    /// The end is under way, or ended at once with a failure its record
    /// tells; `changed`: the snapshot moved with it.
    Begun { changed: bool },
    /// Nothing to start on this pane now, for the reason named: it is moving
    /// or reopening, closing already, unreachable or gone.
    Skipped(&'static str),
    /// A sleep that closes the pane refused to start.
    Refused(SleepRefusal),
}

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
    pub(crate) fn tick_agent_sleep(&mut self, now_unix_ms: u64) -> bool {
        if self.live.is_none() {
            return false;
        }
        // Letter wakes move with the ledger and the records, not the minute.
        let woke = self.drive_letter_wakes();
        // A mission is looked at when a fact it rests on moved (PRD
        // mission-node-simple D-08), which costs a few comparisons when none
        // did.
        let woke = self.drive_mission_health(now_unix_ms) | woke;
        if now_unix_ms < self.agent_sleep_next_decision_unix_ms {
            return woke;
        }
        self.agent_sleep_next_decision_unix_ms = now_unix_ms.saturating_add(DECISION_INTERVAL_MS);
        // A wake that waited for a connection or a free transition tries
        // again at least once a minute.
        self.letter_wakes.touch();
        let changed =
            self.expire_dormant_confirmation(now_unix_ms) | self.fail_unstarted_wakes(now_unix_ms);
        let changed = self.decide_agent_sleep(None, now_unix_ms) | changed | woke;
        // A node at work long enough with no push warns its Conductor
        // (with-hide/hide-private#552).
        let changed = self.warn_missions(now_unix_ms) | changed;
        // A delay that ran out ends, and a node whose move did not come is
        // raised as stalled.
        let changed = self.stall_missions(now_unix_ms) | changed;
        self.sleep_finished_mission_agents(now_unix_ms) | changed
    }

    /// Fails each wake whose agent has not appeared in its pane by the
    /// start's deadline, with the reason a start that never got ready gives.
    /// The engine answers `agent.start` once it has typed the start line, so a
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
            self.letter_wakes.touch();
            // The engine took the start, so it may have run: not tried again.
            self.letter_wakes.failed_wake(pane_id, false);
            self.agent_wake_started.remove(pane_id);
            if let Some(record) = self.snapshot.ui_state.agent_sleep.records.get_mut(pane_id) {
                let reason = agent_sleep_engine::WAKE_NOT_READY.to_owned();
                record.phase = SleepPhase::Failed;
                record.reason = Some(reason.clone());
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_sleep",
                    "kind": "agent_sleep.wake_failed",
                    "pane_id": pane_id,
                    "provider": record.kind,
                    "reason": reason,
                    "detail": format!(
                        "no agent appeared in the pane within {} s of the engine taking the start",
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

    /// The same decision for another machine, a node that dials this core or
    /// a device it dials, run by that machine's coordinator's tick (PRD
    /// core-host-node-remote-core B13). A machine that is not connected is not
    /// decided on: its rows are its last session's, and an end could not
    /// reach it.
    pub(crate) fn tick_node_agent_sleep(&mut self, device: &str, now_unix_ms: u64) -> bool {
        if self
            .node_sleep_next_decision_unix_ms
            .get(device)
            .is_some_and(|next| now_unix_ms < *next)
            || self.device_engine(device).is_none()
        {
            return false;
        }
        self.node_sleep_next_decision_unix_ms.insert(
            device.to_owned(),
            now_unix_ms.saturating_add(DECISION_INTERVAL_MS),
        );
        self.decide_agent_sleep(Some(device), now_unix_ms)
    }

    /// Puts to sleep what is due on one machine: the core's own (`None`) or
    /// another's. At most `MAX_ENDS_IN_FLIGHT` ends run at once over all of them.
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
        // Sleep set to Off sleeps nothing, so no letter is read for it; a
        // store that could not load keeps no letter.
        let letters = match (
            self.snapshot.ui_state.agent_sleep_after_hours,
            &self.delivery_ledger,
        ) {
            (Some(_), Ok(ledger)) => LetterWaits::of(ledger),
            _ => LetterWaits::default(),
        };
        let queued = self.input_queue_panes();
        let awake = Awake {
            on_screen: &on_screen,
            backoff_until: &backoff_until,
            letters: &letters,
            queued: &queued,
        };
        let store = &self.snapshot.ui_state.agent_sleep;
        let due = store.due(
            self.sleep_agents(device),
            machine,
            &awake,
            self.snapshot.ui_state.agent_sleep_after_hours,
            now_unix_ms,
        );
        for pane_id in due.iter().take(ends_left(store)) {
            changed |= self.begin_agent_sleep(pane_id, "idle", None, now_unix_ms);
        }
        changed |= self.sleep_one_under_pressure(device, machine, &awake, &due, now_unix_ms);
        if seen && !changed {
            self.persist_ui_state();
        }
        changed
    }

    /// Memory pressure's part of the minute decision: while the machine
    /// reports warning or critical, the one agent there that has rested
    /// longest, half an hour at least, sleeps, by the same end and within the
    /// same cap as every other sleep; the hours rule's own picks (`taken`)
    /// are not counted twice. The level is the last answer the machine's
    /// coordinator read off the lock; a failed or stale one sleeps nothing.
    /// A candidate whose end does not start (a close it refuses, a pane
    /// moving or reopening) leaves the minute's sleep to the next oldest, and
    /// is logged only: nothing here was asked for by the operator.
    fn sleep_one_under_pressure(
        &mut self,
        device: Option<&str>,
        machine: SleepMachine,
        awake: &Awake<'_>,
        taken: &[String],
        now_unix_ms: u64,
    ) -> bool {
        // Never turns this rule off with the hours.
        if self.snapshot.ui_state.agent_sleep_after_hours.is_none() {
            return false;
        }
        let machine_id = device.unwrap_or(self.node.as_str());
        let Some(answer) = self
            .memory_pressure
            .get(machine_id)
            .and_then(|reading| reading.pressing(now_unix_ms))
        else {
            return false;
        };
        let machine_id = machine_id.to_owned();
        let store = &self.snapshot.ui_state.agent_sleep;
        if ends_left(store) == 0 {
            return false;
        }
        let candidates = store.pressure_candidates(
            self.sleep_agents(device),
            machine,
            awake,
            taken,
            now_unix_ms,
        );
        for (pane_id, idle_since) in candidates {
            let reason = match self.start_agent_sleep(
                &pane_id,
                "memory_pressure",
                Some(SleepCause::MemoryPressure),
                now_unix_ms,
            ) {
                SleepStart::Begun { changed } => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "agent_sleep",
                        "kind": "agent_sleep.pressure",
                        "machine": machine_id,
                        "level": answer.level.code(),
                        "source": answer.source.code(),
                        "pane_id": pane_id,
                        "idle_minutes": now_unix_ms.saturating_sub(idle_since) / 60_000,
                    }));
                    return changed;
                }
                SleepStart::Skipped(reason) => reason,
                SleepStart::Refused(refusal) => refusal.code,
            };
            crate::diagnostic!(serde_json::json!({
                "component": "agent_sleep",
                "kind": "agent_sleep.pressure_skipped",
                "machine": machine_id,
                "pane_id": pane_id,
                "reason": reason,
            }));
        }
        false
    }

    /// A machine's memory pressure, as its coordinator read it off the lock
    /// (`crate::memory_pressure`); `device` is `None` for this core's own.
    /// Nothing on screen changes and no idle clock moves; a failed read is
    /// logged when it first fails, or fails anew. An answer that lands after
    /// its device was removed is dropped, as its stored one went with it
    /// (`forget_device_sleep`).
    pub(crate) fn ingest_memory_pressure(
        &mut self,
        device: Option<&str>,
        reading: crate::memory_pressure::PressureReading,
    ) {
        if device.is_some_and(|device| !self.device_registration_exists(device)) {
            return;
        }
        let machine = device.unwrap_or(self.node.as_str()).to_owned();
        if let Err(error) = &reading.answer
            && self
                .memory_pressure
                .get(&machine)
                .is_none_or(|last| last.answer.as_ref().err() != Some(error))
        {
            crate::diagnostic!(serde_json::json!({
                "component": "agent_sleep",
                "kind": "agent_sleep.pressure_unread",
                "machine": machine,
                "message": error,
            }));
        }
        self.memory_pressure.insert(machine, reading);
    }

    /// Reconciles the records with a fresh local session before it is
    /// projected, and adds the rows of sleeping agents the engine stopped listing.
    pub(super) fn settle_agent_sleep(&mut self, payload: &mut SessionSnapshotPayload) {
        self.settle_agent_sleep_in(payload, SleepScope::Core);
    }

    /// The same for another machine, on its session as its engine names it,
    /// before the session is scoped to the device.
    pub(crate) fn settle_node_agent_sleep(
        &mut self,
        device: &str,
        payload: &mut SessionSnapshotPayload,
    ) {
        self.settle_agent_sleep_in(payload, SleepScope::Device(device));
    }

    fn settle_agent_sleep_in(
        &mut self,
        payload: &mut SessionSnapshotPayload,
        scope: SleepScope<'_>,
    ) {
        let mark = self.read_mark(match scope {
            SleepScope::Core => self.node.as_str(),
            SleepScope::Device(device) => device,
        });
        let settled =
            self.snapshot
                .ui_state
                .agent_sleep
                .settle_payload(payload, scope, mark.as_deref());
        if settled.is_empty() {
            return;
        }
        // A woken or closed sleeper frees its letter wake's place.
        self.letter_wakes.touch();
        let now = unix_milliseconds();
        for event in &settled {
            if let Settled::Woke { pane_id, .. } = event {
                self.snapshot.ui_state.agent_sleep.came_back(pane_id, now);
            }
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
    pub(super) fn stamp_agent_sleep(
        &mut self,
        agents: &mut [SidebarAgentSnapshot],
        live_pane_ids: Option<&HashSet<String>>,
    ) {
        self.confirm_dormant_wake(agents);
        self.stamp_agent_sleep_in(agents, SleepScope::Core, live_pane_ids);
    }

    /// The same for the rows of another machine.
    pub(super) fn stamp_node_agent_sleep(
        &mut self,
        device: &str,
        agents: &mut [SidebarAgentSnapshot],
        live_pane_ids: &HashSet<String>,
    ) {
        self.stamp_agent_sleep_in(agents, SleepScope::Device(device), Some(live_pane_ids));
    }

    fn stamp_agent_sleep_in(
        &mut self,
        agents: &mut [SidebarAgentSnapshot],
        scope: SleepScope<'_>,
        live_pane_ids: Option<&HashSet<String>>,
    ) {
        self.snapshot.ui_state.agent_sleep.annotate(agents);
        self.lay_wait_lines(agents);
        let store = &mut self.snapshot.ui_state.agent_sleep;
        if store.stamp(agents, scope, live_pane_ids, unix_milliseconds()) {
            self.persist_ui_state();
        }
    }

    /// Another machine's tabs on screen moved: a visit, as a committed local
    /// one is (B12), once that machine's session confirms it. Wakes what
    /// sleeps in the tabs that came on screen and stamps the last look of
    /// those and of the tabs that left.
    pub(super) fn node_agent_sleep_visit(&mut self, device: &str) -> bool {
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
        match self.sleep_pane(pane_id, "manual") {
            Ok(changed) => changed,
            Err(SleepRefused::Unreachable) => {
                self.set_error(
                    "pane.control_unavailable",
                    "Putting an agent to sleep requires a live engine connection",
                    true,
                );
                true
            }
            Err(refused) => {
                self.set_error("agent_sleep.refused", refused.detail(), false);
                true
            }
        }
    }

    /// Starts the end of the agent in `pane_id` by the pane menu's rules,
    /// or says why it may not: the one check the menu and `hide agent
    /// sleep` share (PRD mission-worker-sleep D-10).
    fn sleep_pane(&mut self, pane_id: &str, trigger: &str) -> Result<bool, SleepRefused> {
        let device = crate::address::pane_device(pane_id);
        let machine = self.sleep_machine_of_pane(pane_id);
        let agent = self
            .sleep_agents(device)
            .iter()
            .find(|agent| agent.pane_id == pane_id)
            .ok_or(SleepRefused::NoAgent)?;
        if self
            .snapshot
            .ui_state
            .agent_sleep
            .records
            .contains_key(pane_id)
        {
            return Err(SleepRefused::Agent(AgentActionRefusal::Asleep));
        }
        if let Some(refusal) = crate::agent_sleep::sleep_refusal(agent, machine) {
            return Err(SleepRefused::Agent(refusal));
        }
        let reachable = match device {
            None => self.live.is_some(),
            Some(device) => self.device_engine(device).is_some(),
        };
        if !reachable {
            return Err(SleepRefused::Unreachable);
        }
        Ok(self.begin_agent_sleep(pane_id, trigger, None, unix_milliseconds()))
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
        // A letter that came while the agent was ending may wake it now.
        self.letter_wakes.touch();
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
                record.ended_seq = Some(state_change_seq);
                record.since_unix_ms = now;
                let pending_wake = record.pending_wake.take();
                // Keys for the sleeping pane are dropped at its node from now.
                self.sync_terminal_intents();
                self.mission_sleep_landed(pane_id);
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
                    // The core's own engine, this machine's or another's,
                    // stopped listing the agent before its end landed and
                    // published the session while the record was still
                    // ending, which draws no row, and it publishes again only
                    // on a change; the pane's machine's session is read again
                    // so the sleeping row is drawn from the record. The
                    // engine can still list the agent here, and the published
                    // session stream brings the session that no longer does.
                    self.read_session_of(pane_id);
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
        self.letter_wakes.touch();
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
            WakeOutcome::Failed {
                reason,
                detail,
                passing,
            } => {
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
                    "passing": passing,
                }));
                self.letter_wakes.failed_wake(pane_id, passing);
                self.persist_ui_state();
                self.refresh_sleep_rows_of(pane_id);
                true
            }
        }
    }

    /// Starts the end of the agent in `pane_id`, the one path every sleep
    /// takes; `cause` is kept where its row says why it sleeps.
    /// Starts `pane_id`'s end, and says whether something on screen moved;
    /// a close that refuses to start is told on the snapshot.
    fn begin_agent_sleep(
        &mut self,
        pane_id: &str,
        trigger: &str,
        cause: Option<SleepCause>,
        now_unix_ms: u64,
    ) -> bool {
        match self.start_agent_sleep(pane_id, trigger, cause, now_unix_ms) {
            SleepStart::Begun { changed } => changed,
            SleepStart::Skipped(_) => false,
            SleepStart::Refused(refusal) => {
                self.set_error(refusal.code, refusal.message, refusal.retryable);
                true
            }
        }
    }

    fn start_agent_sleep(
        &mut self,
        pane_id: &str,
        trigger: &str,
        cause: Option<SleepCause>,
        now_unix_ms: u64,
    ) -> SleepStart {
        if self.pane_move_holds_pane(pane_id) {
            return SleepStart::Skipped("pane_moving");
        }
        // A reopen's worker ends and starts the pane's agent itself, through
        // the sleep's own input; the pane sleeps once it is done, as a reopen
        // never starts on a pane that sleeps (`request_pane_reopen`).
        if matches!(
            self.pane_reopens.get(pane_id),
            Some(crate::model::PaneReopenSnapshot::Pending)
        ) {
            return SleepStart::Skipped("reopen_pending");
        }
        let Some(context) = self.sleep_target(pane_id) else {
            return SleepStart::Skipped("unreachable");
        };
        let Some(agent) = self
            .sleep_agents(crate::address::pane_device(pane_id))
            .iter()
            .find(|agent| agent.pane_id == pane_id)
        else {
            return SleepStart::Skipped("not_listed");
        };
        // Only the core's own machine gets here with one (`sleep_refusal`).
        if crate::agent_sleep::closes_pane_when_sleeping(&agent.agent_kind) {
            return match self.admit_dormant_sleep(pane_id, cause, now_unix_ms) {
                Ok(true) => SleepStart::Begun { changed: true },
                Ok(false) => SleepStart::Skipped("closing"),
                Err(refusal) => SleepStart::Refused(refusal),
            };
        }
        let cwd = self.agent_sleep_pane_cwd(pane_id);
        let Some(mut record) = SleepRecord::ending(agent, cwd, now_unix_ms) else {
            return SleepStart::Skipped("no_session");
        };
        record.server_mark = record.server_mark.or_else(|| {
            self.read_mark(crate::address::pane_device(pane_id).unwrap_or(self.node.as_str()))
        });
        record.launch = self.launch_choice_of(agent);
        record.profile = self.launch_profile_of(agent);
        record.cause = cause;
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
        if let Err(message) = agent_sleep_engine::spawn_end(context, pane_id.to_owned(), kind) {
            return SleepStart::Begun {
                changed: self.ingest_agent_sleep_end(pane_id, Err(message)),
            };
        }
        self.persist_ui_state();
        // The pane is still awake and drawn so until the end lands (B8).
        SleepStart::Begun { changed: false }
    }

    /// Asks the session of `pane_id`'s machine again, so its rows are drawn
    /// from the sleep records as they are now.
    fn read_session_of(&mut self, pane_id: &str) {
        match crate::address::pane_device(pane_id) {
            None if self.runs_own_engine() => self.request_republish(),
            None => {}
            Some(device) => {
                if let Some(sync) = self
                    .remote_connections
                    .get(device)
                    .and_then(|connection| connection.sync.as_ref())
                {
                    sync.republish_waker().wake();
                }
            }
        }
    }

    /// The pane of the live agent of `kind` that holds conversation `session`
    /// on `device`'s machine, other than `except`; a sleeping row holds none.
    pub(super) fn live_conversation_pane(
        &self,
        device: &str,
        kind: &str,
        session: &str,
        except: &str,
    ) -> Option<String> {
        let kind = hide_agent_adapter::canonical_kind(kind);
        self.machine_agents(device)
            .iter()
            .find(|agent| {
                agent.pane_id != except
                    && agent.sleep.is_none()
                    && hide_agent_adapter::canonical_kind(&agent.agent_kind) == kind
                    && agent.session_id.as_deref() == Some(session)
            })
            .map(|agent| agent.pane_id.clone())
    }

    /// A sleeping agent whose conversation another pane already runs is not
    /// woken (with-hide/hide#2590): a resume would be a second writer of the
    /// one conversation. Its sleep ends as if the agent had ended there,
    /// leaving the pane its shell, and the operator who asked for it is
    /// taken to the pane that runs it.
    fn sleep_live_elsewhere(&mut self, pane_id: &str, live: String, trigger: &str) -> bool {
        self.snapshot.ui_state.agent_sleep.records.remove(pane_id);
        crate::diagnostic!(serde_json::json!({
            "component": "agent_sleep",
            "kind": "agent_sleep.live_elsewhere",
            "pane_id": pane_id,
            "live_pane_id": live,
            "trigger": trigger,
        }));
        // The pane's shell takes keys again, and its sleeping row goes.
        self.sync_terminal_intents();
        self.refresh_sleep_rows_of(pane_id);
        self.read_session_of(pane_id);
        self.show_live_conversation(live, trigger);
        self.persist_ui_state();
        true
    }

    /// Takes the operator to `live`, the pane that runs the conversation a
    /// wake was asked for; a letter's wake moves no screen.
    pub(super) fn show_live_conversation(&mut self, live: String, trigger: &str) {
        if trigger != "letter" {
            match crate::address::pane_device(&live) {
                None => self.focus_pane(live, PaneFocusOrigin::Operator, None),
                Some(device) => {
                    self.request_remote_control(RemoteControlPayload {
                        target_id: device.to_owned(),
                        request_id: format!("agent-sleep:{}:{live}", unix_milliseconds()),
                        report_pane_focus_outcome: false,
                        focus_device: false,
                        request: RemoteControlRequest::FocusPane { pane_id: live },
                    });
                }
            }
        }
    }

    fn begin_agent_wake(&mut self, pane_id: &str, mode: WakeMode, trigger: &str) -> bool {
        if self.failed_terminal_id(pane_id).is_some() {
            return false;
        }
        let device = crate::address::pane_device(pane_id).unwrap_or(self.node.as_str());
        if let Some(live) = self
            .snapshot
            .ui_state
            .agent_sleep
            .records
            .get(pane_id)
            .filter(|_| mode == WakeMode::Resume)
            .and_then(|record| {
                self.live_conversation_pane(
                    device,
                    &record.kind,
                    record.session_id.as_deref()?,
                    pane_id,
                )
            })
        {
            return self.sleep_live_elsewhere(pane_id, live, trigger);
        }
        let Some(context) = self.sleep_target(pane_id) else {
            return false;
        };
        let codex_daemon = self.codex_daemon_for_pane(pane_id);
        let permission = match self.snapshot.ui_state.agent_sleep.records.get(pane_id) {
            Some(record) => self.agent_permission_for_pane(pane_id, &record.kind),
            None => return false,
        };
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
        let launch = match crate::runtime::agent_choice::launch_arguments(
            &record.kind,
            &record.launch,
            &permission,
        ) {
            Ok(launch) => launch,
            Err(detail) => {
                return self.ingest_agent_wake(
                    pane_id,
                    mode,
                    WakeOutcome::Failed {
                        reason: agent_sleep_engine::WAKE_LAUNCH_CHOICE.to_owned(),
                        detail,
                        passing: false,
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
                            reason: agent_sleep_engine::WAKE_NOT_RESUMED.to_owned(),
                            detail,
                            passing: false,
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
                .unwrap_or_else(|| crate::fork::wake_name(&context.engine_pane_id)),
            mode,
            args,
            cwd: record.cwd.clone(),
            session_id: record.session_id.clone(),
            codex_daemon,
            profile: record.profile.clone(),
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
        let (kind, name, session, profile) = (
            request.kind.clone(),
            request.name.clone(),
            request.session_id.clone(),
            request.profile.clone(),
        );
        if let Err(message) = agent_sleep_engine::spawn_wake(context, request) {
            return self.ingest_agent_wake(
                pane_id,
                mode,
                // The worker never ran, so nothing was typed.
                WakeOutcome::Failed {
                    reason: agent_sleep_engine::WAKE_NOT_READY.to_owned(),
                    detail: message,
                    passing: true,
                },
            );
        }
        let session = (mode == WakeMode::Resume).then_some(session).flatten();
        self.record_pane_launch(
            pane_id,
            &kind,
            &name,
            &choice,
            profile.as_ref(),
            session.as_deref(),
        );
        self.persist_ui_state();
        self.refresh_sleep_rows_of(pane_id);
        true
    }

    /// Draws a record's new phase on the rows of the machine the pane is on.
    fn refresh_sleep_rows_of(&mut self, pane_id: &str) -> bool {
        match crate::address::pane_device(pane_id) {
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
    pub(super) fn refresh_agent_sleep_rows(&mut self) -> bool {
        let mut agents = self.snapshot.navigator.agents.clone();
        self.snapshot.ui_state.agent_sleep.annotate(&mut agents);
        self.lay_wait_lines(&mut agents);
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
    /// A project no list shows (an automation's own folder) is drawn by no
    /// Workspace, though the keys typed into a run's terminal leave the
    /// focus there: only the automation screen draws its panes, and says
    /// which (`viewed_panes`, B17).
    pub(super) fn agent_sleep_on_screen(&self, device: Option<&str>) -> HashSet<String> {
        let tabs = match device {
            None if self.device_in_front(self.node.as_str()) => self.shown_agent_tabs(),
            None => Vec::new(),
            Some(device) => self.shown_node_tabs(device),
        };
        let unlisted: HashSet<&str> = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.unlisted)
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .filter_map(|tab| tab.id.as_deref())
            .collect();
        let viewed = device
            .is_none()
            .then(|| self.viewed_panes().iter().cloned())
            .into_iter()
            .flatten();
        tabs.iter()
            .filter(|tab_id| !unlisted.contains(tab_id.as_str()))
            .flat_map(|tab_id| self.agent_sleep_tab_panes(tab_id))
            .chain(viewed)
            .collect()
    }

    /// The tabs of a node's session on screen: its Agent areas' while it is
    /// in front with them, else the tab its engine has in front.
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

    /// How sleep treats a device: this core's own machine, or another one,
    /// which sleeps alike whichever way its link was made.
    pub(super) fn sleep_machine(&self, device: &str) -> SleepMachine {
        if device == self.node.as_str() {
            SleepMachine::Core
        } else {
            SleepMachine::Node
        }
    }

    pub(super) fn sleep_machine_of_pane(&self, pane_id: &str) -> SleepMachine {
        crate::address::pane_device(pane_id)
            .map_or(SleepMachine::Core, |device| self.sleep_machine(device))
    }

    /// Where a pane's sleep or wake runs: the core's own engine and node, or
    /// those of the machine with the pane while it is connected; `None` for
    /// one that is not, so nothing waits on a link that is down.
    fn sleep_target(&self, pane_id: &str) -> Option<SleepTarget> {
        let Some(device) = crate::address::pane_device(pane_id) else {
            let live = self.live.as_ref()?;
            return Some(SleepTarget {
                engine: Arc::clone(&live.engine),
                node: Arc::clone(&live.node),
                runtime: live.runtime.clone(),
                notifier: live.notifier.clone(),
                engine_pane_id: pane_id.to_owned(),
                input_gate: Arc::clone(&self.input_gate),
            });
        };
        let worker = self.worker_context.as_ref()?;
        Some(SleepTarget {
            engine: self.device_engine(device)?,
            node: super::inbound::InboundLink::for_device(device, worker.runtime.clone()),
            runtime: worker.runtime.clone(),
            notifier: worker.notifier.clone(),
            engine_pane_id: crate::address::pane_of(pane_id, device)?.to_owned(),
            input_gate: Arc::clone(&self.input_gate),
        })
    }

    /// Forgets what sleep kept of a device's panes once the device is
    /// removed; its panes are not touched.
    pub(super) fn forget_device_sleep(&mut self, device: &str) {
        let store = &mut self.snapshot.ui_state.agent_sleep;
        store
            .records
            .retain(|pane_id, _| !crate::address::is_pane_of(pane_id, device));
        self.sync_input_gate();
        let store = &mut self.snapshot.ui_state.agent_sleep;
        store
            .stamps
            .retain(|pane_id, _| !crate::address::is_pane_of(pane_id, device));
        self.agent_sleep_backoff
            .retain(|pane_id, _| !crate::address::is_pane_of(pane_id, device));
        self.node_sleep_next_decision_unix_ms.remove(device);
        self.node_sleep_shown_tabs.remove(device);
        self.memory_pressure.remove(device);
    }

    fn agent_sleep_tab_panes(&self, tab_id: &str) -> Vec<String> {
        let device = crate::address::Scoped::parse(tab_id)
            .filter(|scoped| scoped.kind == crate::address::Kind::Tab)
            .map(|scoped| scoped.device);
        self.sleep_workspaces(device)
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .filter(|tab| tab.id.as_deref() == Some(tab_id))
            .flat_map(|tab| tab.panes.iter().map(|pane| pane.id.clone()))
            .collect()
    }

    fn agent_sleep_pane_cwd(&self, pane_id: &str) -> Option<String> {
        self.sleep_workspaces(crate::address::pane_device(pane_id))
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .find(|pane| pane.id == pane_id)
            .map(|pane| pane.cwd.clone())
            .filter(|cwd| !cwd.trim().is_empty())
    }
}

/// Why an agent was not put to sleep.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SleepRefused {
    NoAgent,
    Agent(AgentActionRefusal),
    Unreachable,
}

impl SleepRefused {
    fn detail(self) -> &'static str {
        match self {
            Self::NoAgent => "This pane has no agent",
            Self::Agent(refusal) => refusal.detail(),
            Self::Unreachable => "Putting an agent to sleep requires a live engine connection",
        }
    }
}
