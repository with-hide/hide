//! Memory-only admission and observations. Disk, process and helper work is
//! owned by delivery workers, never by the owner-thread caller.

use std::collections::HashSet;
use std::sync::Arc;

use crate::delivery::doorbell::Turn;
use crate::delivery::ledger::Ledger;
use crate::delivery::worker::{Authority, Client, Prepared, PreparedHuman};
use crate::delivery::{Actor, Command};
use crate::labels::overlay::LabelOverlay;
use crate::sidebar::SessionSnapshotPayload;
use crate::workspace_control::{Caller, Context, Query};

use super::{Runtime, unix_milliseconds};

pub(crate) const OBSERVATION_LIMIT: usize = 2048;

#[derive(Clone)]
pub(crate) struct Observation {
    pub actor: Actor,
    pub raw_pane_id: String,
    pub status: String,
    pub state_change_seq: Option<u64>,
    pub status_changed_at_unix_ms: u64,
    /// The last input hide routed to this pane: a key, a paste, a phone
    /// write or an agent-find key.
    pub last_input_at_unix_ms: u64,
    /// The last moment the pane's input was proven submitted: a prompt hook
    /// of the pane's own session, or a phone reply. No key proves it. Input
    /// after it and after `entered_working_at_unix_ms` is an unsent draft.
    pub last_submit_at_unix_ms: u64,
    /// The snapshot that first showed the pane `working` in its current run.
    /// Input hide sent before it was consumed by that work, such as the
    /// answer to a menu.
    pub entered_working_at_unix_ms: u64,
    /// The newest input the operator released by pressing Deliver now on
    /// this pane's waiting letters (`deliver_letters_now`): input up to it is
    /// no draft the bell waits for, since the operator saw the composer above
    /// which the mark sits and chose to ring. Input after it is a draft again.
    pub released_input_at_unix_ms: u64,
    pub session: Option<hide_session::session_activity::SessionActivityRequest>,
    pub host_scope: Option<String>,
    pub turn: Turn,
}

impl Observation {
    /// Input hide routed after the last submission, the last start of work
    /// and the input the operator last released: an unsent draft, a prompt
    /// Esc brought back, a recalled input.
    pub(crate) fn unsent_draft(&self) -> bool {
        self.last_input_at_unix_ms
            > self
                .last_submit_at_unix_ms
                .max(self.entered_working_at_unix_ms)
                .max(self.released_input_at_unix_ms)
    }

    /// A Factory's recipient as a letter target: no pane, no status.
    pub(crate) fn code_owned(actor: Actor) -> Self {
        Self {
            raw_pane_id: actor.pane_id.clone(),
            actor,
            status: "idle".into(),
            state_change_seq: None,
            status_changed_at_unix_ms: 0,
            last_input_at_unix_ms: 0,
            last_submit_at_unix_ms: 0,
            entered_working_at_unix_ms: 0,
            released_input_at_unix_ms: 0,
            session: None,
            host_scope: None,
            turn: Turn::NotReported,
        }
    }
}

impl Runtime {
    /// `labels` is the device's label overlay, whose session reads say what
    /// an agent waits for; without one, an agent whose read reports turns is
    /// not known to wait for nothing.
    pub(crate) fn observe_delivery(
        &mut self,
        device: &str,
        payload: &SessionSnapshotPayload,
        host_scope: Option<&str>,
        labels: Option<&LabelOverlay>,
    ) {
        let now = unix_milliseconds();
        let mut present = HashSet::new();
        let mut excess = false;
        for agent in &payload.agents {
            let (Some(pane), Some(name), Some(kind), Some(status)) = (
                agent.pane_id.as_deref(),
                agent.id.as_deref(),
                agent.agent.as_deref(),
                agent.agent_status.as_deref(),
            ) else {
                continue;
            };
            if ![pane, name, kind, status]
                .into_iter()
                .all(crate::delivery::valid_key)
            {
                continue;
            }
            let pane_id = if self.node == device {
                pane.to_owned()
            } else {
                format!("remote:{device}:pane:{pane}")
            };
            if !self.delivery_observations.contains_key(&pane_id)
                && self.delivery_observations.len() >= OBSERVATION_LIMIT
            {
                excess = true;
                continue;
            }
            present.insert(pane_id.clone());
            let actor = Actor {
                pane_id: pane_id.clone(),
                name: name.to_owned(),
                kind: kind.to_owned(),
                device_id: device.to_owned(),
                session: agent.lineage_session.clone(),
            };
            if !actor.valid() {
                continue;
            }
            let previous = self
                .delivery_observations
                .get(&pane_id)
                .filter(|old| old.actor.same_identity(&actor));
            let persisted = self.delivery_ledger.as_ref().ok().and_then(|ledger| {
                ledger.watches.iter().find(|watch| {
                    watch.target.same_identity(&actor)
                        && watch.last_status == status
                        && watch.last_state_change_seq == agent.state_change_seq
                })
            });
            let changed_at = previous
                .filter(|old| {
                    old.status == status && old.state_change_seq == agent.state_change_seq
                })
                .map(|old| old.status_changed_at_unix_ms)
                .or_else(|| persisted.map(|watch| watch.status_changed_at_unix_ms))
                .unwrap_or(now);
            // Turn reporting is a session-format fact, independent of letters
            // and bell eligibility. An unread turn-capable session stays held.
            let session_kind = hide_session::Agent::from_kind(kind);
            let turn = match session_kind {
                Some(kind) if kind.reports_turns() => labels
                    .and_then(|labels| labels.waiting(agent))
                    .map_or(Turn::Unread, Turn::Read),
                _ => Turn::NotReported,
            };
            let session = session_kind
                .filter(|kind| kind.has_session_file())
                .zip(agent.agent_session.as_ref())
                .filter(|(_, reference)| {
                    reference.value.len() <= 4096
                        && matches!(reference.kind.as_str(), "id" | "path")
                })
                .map(|(agent_kind, reference)| {
                    hide_session::session_activity::SessionActivityRequest {
                        agent: agent_kind,
                        reference_kind: reference.kind.clone(),
                        reference_value: reference.value.clone(),
                        cwd: agent.cwd.clone().filter(|cwd| cwd.len() <= 4096),
                        exact_route: false,
                        expected_id: None,
                    }
                });
            // A pane seen for the first time after a restart, an update or a
            // device's return keeps the draft the last run held in it, unless
            // it is working now, which a submission started.
            let draft_kept = previous.is_none()
                && status != "working"
                && self.composer_marks.iter().any(|mark| mark.matches(&actor));
            let observation = Observation {
                actor,
                raw_pane_id: pane.to_owned(),
                status: status.to_owned(),
                state_change_seq: agent.state_change_seq,
                status_changed_at_unix_ms: changed_at,
                // A pane hide has not seen before has no key it knows of, so
                // every clock starts at this observation: a 30 second grace,
                // and no draft assumed, unless the last run kept one.
                last_input_at_unix_ms: previous.map(|old| old.last_input_at_unix_ms).unwrap_or(now),
                last_submit_at_unix_ms: previous.map(|old| old.last_submit_at_unix_ms).unwrap_or(
                    if draft_kept {
                        now.saturating_sub(1)
                    } else {
                        now
                    },
                ),
                entered_working_at_unix_ms: match previous {
                    Some(old) if status != "working" || old.status == "working" => {
                        old.entered_working_at_unix_ms
                    }
                    _ if draft_kept => now.saturating_sub(1),
                    _ => now,
                },
                released_input_at_unix_ms: previous.map_or(0, |old| old.released_input_at_unix_ms),
                session,
                host_scope: host_scope
                    .filter(|scope| scope.len() <= 4096)
                    .map(str::to_owned),
                turn,
            };
            self.delivery_observations
                .insert(pane_id.clone(), observation);
            self.sync_composer_mark(&pane_id, now);
        }
        self.delivery_observations.retain(|pane, observation| {
            observation.actor.device_id != device || present.contains(pane)
        });
        let observed = &self.delivery_observations;
        self.delivery_input_traced
            .retain(|pane, _| observed.contains_key(pane));
        if excess {
            self.delivery_overflow.insert(device.to_owned());
        } else {
            self.delivery_overflow.remove(device);
        }
        self.delivery_connected.insert(device.to_owned());
        self.observe_registration_panes(device, payload, host_scope);
        // A woken pane is first seen here; the session projection that
        // follows publishes whatever this settles.
        self.settle_dormant_registrations();
    }

    /// A host's session sync is about to ask Herdr for a fresh snapshot.
    /// Until that snapshot publishes, the host's panes are unknown and no
    /// registration on it is ended.
    pub(crate) fn begin_delivery_pane_read(&mut self, device: &str) {
        if !self.delivery_pane_owner_registered(device) {
            return;
        }
        let floor = self
            .delivery_ledger
            .as_ref()
            .ok()
            .map(|ledger| ledger.next_id);
        self.delivery_panes.insert(
            device.to_owned(),
            crate::coordination::PaneRead {
                host_scope: None,
                floor,
                panes: None,
            },
        );
    }

    /// Records the host's published panes and, when they changed, which live
    /// registrations on it lost their pane. A publish whose panes did not
    /// move costs one borrowed set and no copy; the pass over the
    /// registrations runs only when they moved.
    fn observe_registration_panes(
        &mut self,
        device: &str,
        payload: &SessionSnapshotPayload,
        host_scope: Option<&str>,
    ) {
        if !self.delivery_pane_owner_registered(device) {
            return;
        }
        // An agent's pane is one of the panes; a payload of agents alone
        // (a device read before its layout) still names them.
        let panes: HashSet<&str> = payload
            .panes
            .iter()
            .map(|pane| pane.pane_id.as_str())
            .chain(
                payload
                    .agents
                    .iter()
                    .filter_map(|agent| agent.pane_id.as_deref()),
            )
            .collect();
        let read = self.delivery_panes.get(device);
        if read.is_some_and(|read| {
            read.host_scope.as_deref() == host_scope
                && read.panes.as_ref().is_some_and(|known| {
                    known.len() == panes.len() && panes.iter().all(|pane| known.contains(*pane))
                })
        }) {
            return;
        }
        // A transition judged without the ledger would be lost, so the
        // previous read stays until a ledger can judge it.
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return;
        };
        // A sleeping session's pane is closed on purpose and its wake
        // continues the registration, so its absence ends nothing.
        let held: HashSet<&str> = self
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .values()
            .filter_map(|record| record.registration.as_ref())
            .map(|registration| registration.id.as_str())
            .collect();
        let read =
            self.delivery_panes
                .entry(device.to_owned())
                .or_insert(crate::coordination::PaneRead {
                    host_scope: None,
                    floor: None,
                    panes: None,
                });
        read.host_scope = host_scope.map(str::to_owned);
        let previous = read
            .panes
            .replace(panes.into_iter().map(str::to_owned).collect());
        self.registrations_gone.extend(
            crate::coordination::gone_registrations(ledger, device, read, previous.as_ref())
                .into_iter()
                .filter(|(id, _)| !held.contains(id.as_str())),
        );
    }

    /// The registration the live execution in `pane` holds, which a sleep
    /// that closes the pane saves so its wake continues it.
    pub(super) fn delivery_registration_of(
        &self,
        pane: &str,
    ) -> Option<crate::agent_sleep::DormantRegistration> {
        let actor = &self.delivery_observations.get(pane)?.actor;
        self.delivery_ledger
            .as_ref()
            .ok()?
            .agents
            .iter()
            .find(|record| !record.ended && record.actor.same_identity(actor))
            .map(|record| crate::agent_sleep::DormantRegistration {
                id: record.id.clone(),
                actor: record.actor.clone(),
            })
    }

    /// Whether a sleeping session holds `actor`'s registration for its wake.
    fn dormant_holds_actor(&self, actor: &Actor) -> bool {
        self.snapshot
            .ui_state
            .agent_sleep
            .dormant
            .values()
            .filter_map(|record| record.registration.as_ref())
            .any(|registration| registration.actor.same_identity(actor))
    }

    /// The registrations the delivery store has still to point at a woken
    /// pane.
    pub(crate) fn delivery_registration_rebinds(
        &self,
    ) -> std::collections::BTreeMap<String, crate::coordination::Rebind> {
        self.registration_rebinds.clone()
    }

    /// A retired remote coordinator may finish before its off-lock join,
    /// but only a registered device still owns a pane read.
    fn delivery_pane_owner_registered(&self, device: &str) -> bool {
        device == self.node.as_str()
            || self
                .snapshot
                .ui_state
                .device_registrations
                .iter()
                .any(|registration| registration.id == device)
    }

    /// Every machine other than this core's own that holds agents, with
    /// whether a letter can reach it now: the core holds a connection to its
    /// Herdr and its link has not ended (PRD core-host-node-move B13). A
    /// letter to an agent there counts its delivery deadline only while it
    /// is reachable. A node's link ends here at once; the Herdr connection
    /// it carried reads as gone only at the next sync (`ingest_host_closed`).
    pub(crate) fn delivery_reach(&self) -> crate::delivery::ledger::Reach {
        self.snapshot
            .ui_state
            .device_registrations
            .iter()
            .filter(|registration| registration.id != self.node.as_str())
            .map(|registration| {
                let id = registration.id.as_str();
                let reachable = self.remote_herdr_api(id).is_some() && !self.device_host_ended(id);
                (registration.id.clone(), reachable)
            })
            .collect()
    }

    /// The registrations the delivery store has still to end.
    pub(crate) fn delivery_registrations_gone(
        &self,
    ) -> std::collections::BTreeMap<String, crate::coordination::PaneGone> {
        self.registrations_gone.clone()
    }

    /// A key, paste or phone write hide routed to a pane: a draft may have
    /// grown there, so the quiet period restarts. No key ends a draft, not
    /// even Enter: a built-in picker (`/model`, `/resume`) is opened with
    /// Enter and reads `done` to Herdr, so only a prompt hook of the pane's
    /// own session, a phone reply, or the pane entering work proves the
    /// composer was sent.
    ///
    /// `path` names how the input reached the pane (`find`, `attachment`,
    /// `phone`), for the record `trace_delivery_input` leaves.
    pub(crate) fn note_delivery_key(&mut self, pane_id: &str, path: &'static str) {
        self.note_delivery_key_at(pane_id, unix_milliseconds());
        self.trace_delivery_input(pane_id, path);
    }

    /// Leaves one record per pane per second of an input the core noted
    /// itself, so a draft hold can be traced to the input that made it. The
    /// node leaves the same record for the keys it saw (`terminal.input`).
    /// Only observed panes are traced, which bounds the map.
    fn trace_delivery_input(&mut self, pane_id: &str, path: &'static str) {
        const WINDOW_MS: u64 = 1_000;
        if !self.delivery_observations.contains_key(pane_id) {
            return;
        }
        let now = unix_milliseconds();
        let last = self
            .delivery_input_traced
            .entry(pane_id.to_owned())
            .or_insert(0);
        if now.saturating_sub(*last) < WINDOW_MS {
            return;
        }
        *last = now;
        crate::diagnostic!(serde_json::json!({
            "component": "delivery",
            "kind": "delivery.input",
            "pane_id": pane_id,
            "path": path,
        }));
    }

    /// [`Runtime::note_delivery_key`] for a key its node saw typed at
    /// `at_unix_ms`: the node reports a burst's keys once, with the time of
    /// its last, so the quiet period counts from that key (PRD
    /// core-host-node-terminal D-13, B7).
    pub(crate) fn note_delivery_key_at(&mut self, pane_id: &str, at_unix_ms: u64) {
        self.reopen_resolved_session(pane_id);
        if let Some(observation) = self.delivery_observations.get_mut(pane_id) {
            observation.last_input_at_unix_ms = observation.last_input_at_unix_ms.max(at_unix_ms);
            self.sync_composer_mark(pane_id, unix_milliseconds());
        }
    }

    /// The phone's reply is text hide sent and submitted in one write to an
    /// agent waiting on the operator, so it is a key and a submit.
    pub(crate) fn note_delivery_reply(&mut self, pane_id: &str) {
        self.reopen_resolved_session(pane_id);
        if let Some(observation) = self.delivery_observations.get_mut(pane_id) {
            let now = unix_milliseconds();
            observation.last_input_at_unix_ms = now;
            observation.last_submit_at_unix_ms = now;
            self.sync_composer_mark(pane_id, now);
            self.trace_delivery_input(pane_id, "phone_reply");
        }
    }

    /// A prompt hook ran in the pane, so whatever was in its composer was
    /// submitted, whoever typed it and wherever it was typed.
    /// A hook that runs while the pane is already `working` is a queued
    /// prompt being taken up, not the composer being sent: the operator's
    /// newer draft is still there, and the turn's own entry to `working`
    /// already covers whatever was typed before it.
    fn note_prompt_submitted(&mut self, pane_id: &str) {
        self.reopen_resolved_session(pane_id);
        if let Some(observation) = self
            .delivery_observations
            .get_mut(pane_id)
            .filter(|observation| observation.status != "working")
        {
            observation.last_submit_at_unix_ms = unix_milliseconds();
            self.sync_composer_mark(pane_id, unix_milliseconds());
        }
    }

    /// The operator pressed Deliver now on a pane whose letters wait on a
    /// possible draft (docs/delivery.md, Delivering a held letter now). The
    /// press says the composer holds nothing of theirs, so the input before
    /// it no longer holds the bell, and the doorbell's next pass rings with
    /// every guard it always applies and types the same line. A press the
    /// mark no longer stands for (a key reached the pane, the letters went or
    /// were taken in since the last pass) changes nothing and is logged.
    pub(super) fn deliver_letters_now(&mut self, pane_id: &str) -> bool {
        use crate::delivery::doorbell::WAITING_MARK_REST_MS;
        let now = unix_milliseconds();
        let refusal = match (
            self.letters_waiting.get(pane_id).copied(),
            self.delivery_observations.get_mut(pane_id),
        ) {
            (None, _) => Err("no_letter_waiting"),
            (Some(_), None) => Err("pane_unavailable"),
            (Some(_), Some(observation))
                if !observation.unsent_draft()
                    || now.saturating_sub(
                        observation
                            .last_input_at_unix_ms
                            .max(observation.status_changed_at_unix_ms),
                    ) < WAITING_MARK_REST_MS =>
            {
                Err("pane_not_at_rest")
            }
            (Some(letters), Some(observation)) => {
                observation.released_input_at_unix_ms = observation.last_input_at_unix_ms;
                Ok(letters)
            }
        };
        let letters = match refusal {
            Ok(letters) => letters,
            Err(reason) => {
                crate::diagnostic!(serde_json::json!({"component":"delivery",
                    "kind":"deliver_now.refused","pane_id":pane_id,"reason":reason}));
                return false;
            }
        };
        crate::diagnostic!(serde_json::json!({"component":"delivery",
            "kind":"deliver_now.released","pane_id":pane_id,"letters":letters}));
        self.sync_composer_mark(pane_id, now);
        let mut waiting = self.letters_waiting.clone();
        waiting.remove(pane_id);
        self.set_letters_waiting(waiting);
        self.sync_session_state() | self.refresh_agent_scopes()
    }

    pub(crate) fn prepare_delivery(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        hint: Option<&str>,
        command: Command,
    ) -> Result<Prepared, String> {
        let context = self
            .workspace_control_query(device, caller, Query::Info)
            .map_err(|refusal| refusal.reason.to_owned())?
            .context;
        if context != *expected {
            return Err("caller_context_changed".into());
        }
        // Herdr numbers panes per machine and an agent learns the numbers from
        // its own machine's Herdr, so a bare pane id names a pane on the
        // caller's machine; the core's panes are observed under their bare ids
        // and a device's under `remote:<device>:pane:<id>`. A key already
        // spelled `remote:` is a full address and stays as it is.
        let on_core = self.node == device;
        let qualify = |pane: &str| {
            if on_core || pane.starts_with("remote:") {
                pane.to_owned()
            } else {
                format!("remote:{device}:pane:{pane}")
            }
        };
        // Only Hide's pane attestation names the agent a command acts as; a
        // checkout-bound caller has no pane, and a hint cannot lend it one.
        let Caller::Pane(actor_pane) = Caller::parse(caller) else {
            return Err("agent_pane_required".into());
        };
        if hint.is_some_and(|hint| qualify(hint) != actor_pane) {
            return Err("caller_identity_conflict".into());
        }
        let actor = self
            .delivery_observations
            .get(actor_pane)
            .ok_or("agent_pane_required")?
            .actor
            .clone();
        actor.require_native_identity()?;
        if crate::delivery::reserved_name(&actor.name) {
            // A pane named like a Factory would read as one (D-14).
            return Err("reserved_name".into());
        }
        // Only the pane's own session proves its composer was submitted, so a
        // stray `hide inbox --hook` from a tool in the pane clears no draft.
        // The id is not a secret (D-18 external input stays outside this). The id is checked before it is
        // hashed because this runs under the runtime lock.
        if let Command::Pull {
            session,
            prompt_digest,
            tool,
        } = &command
        {
            if prompt_digest
                .as_deref()
                .is_some_and(|digest| !hide_agent_hooks::delivery::valid_prompt_digest(digest))
            {
                return Err("prompt_digest_invalid".into());
            }
            if let Some(session) = session {
                if !crate::delivery::valid_session(session) {
                    return Err("session_invalid".into());
                }
                let own = crate::wire::session_digest(session) == actor.session;
                // A tool event's ask submits nothing: the composer may hold
                // a draft while the agent works (D-09). It must come from
                // the pane's own session, since it hands letters over.
                if *tool {
                    if !own {
                        return Err("session_mismatch".into());
                    }
                } else if own {
                    self.note_prompt_submitted(&actor.pane_id);
                }
            } else if *tool {
                return Err("session_mismatch".into());
            }
        }
        let resolves_to_caller = |key: &str| {
            qualify(key) == actor.pane_id
                || key == actor.name
                || self.delivery_ledger.as_ref().is_ok_and(|ledger| {
                    crate::coordination::resolve_actor(ledger, key)
                        .is_some_and(|registered| registered.same_identity(&actor))
                })
        };
        match &command {
            Command::WatchStart {
                actor: declared,
                observer,
                ..
            } => {
                if declared
                    .as_deref()
                    .is_some_and(|key| !resolves_to_caller(key))
                    || observer
                        .as_deref()
                        .is_some_and(|key| !resolves_to_caller(key))
                {
                    return Err("caller_identity_conflict".into());
                }
            }
            Command::WatchAssign {
                actor: declared, ..
            } if declared
                .as_deref()
                .is_some_and(|key| !resolves_to_caller(key)) =>
            {
                return Err("caller_identity_conflict".into());
            }
            _ => {}
        }
        let repeated = match &command {
            Command::Send { intent, .. } => self.delivery_ledger.as_ref().is_ok_and(|ledger| {
                crate::delivery::mailbox::existing_intent(
                    ledger,
                    &actor,
                    intent,
                    unix_milliseconds(),
                )
                .is_some()
            }),
            _ => false,
        };
        let target = match &command {
            Command::Send { .. } if repeated => None,
            // A tool event's ask is judged by the caller's own pane: whether
            // it works, and since when.
            Command::Pull { tool: true, .. } => self.delivery_observations.get(actor_pane).cloned(),
            Command::Send { target, .. }
            | Command::WatchStart { target, .. }
            | Command::WatchAssign {
                observer: target, ..
            } if target.starts_with(crate::delivery::FACTORY_PREFIX) => {
                // A Factory is addressed by its code-owned name only; no
                // pane can stand in for it.
                let actor = Actor::factory(
                    &target[crate::delivery::FACTORY_PREFIX.len()..],
                    self.node.as_str(),
                );
                if !self.factory_recipient_current(&actor) {
                    return Err("target_unavailable".into());
                }
                Some(Observation::code_owned(actor))
            }
            Command::Send { target, .. }
            | Command::WatchStart { target, .. }
            | Command::WatchAssign {
                observer: target, ..
            } => {
                let registered = self
                    .delivery_ledger
                    .as_ref()
                    .ok()
                    .and_then(|ledger| crate::coordination::resolve_registered(ledger, target));
                let pane = qualify(target);
                let mut matches = self.delivery_observations.values().filter(|observation| {
                    observation.actor.pane_id == pane
                        || observation.actor.name == *target
                        || registered
                            .is_some_and(|registered| registered.same_identity(&observation.actor))
                });
                let first = matches.next().cloned();
                if matches.next().is_some() {
                    return Err("target_ambiguous".into());
                }
                if first.is_none() && !self.delivery_overflow.is_empty() {
                    return Err("capacity".into());
                }
                first
            }
            _ => None,
        };
        if (matches!(&command, Command::Send { .. }) && !repeated)
            || matches!(&command, Command::WatchAssign { .. })
        {
            target
                .as_ref()
                .ok_or("target_unavailable")?
                .actor
                .require_native_identity()?;
        }
        Ok(Prepared::new(
            self.delivery_client.clone().ok_or("delivery_unavailable")?,
            Authority {
                caller: caller.to_owned(),
                context,
            },
            actor,
            target,
            command,
        ))
    }

    pub(crate) fn install_delivery_client(&mut self, client: Client) {
        self.delivery_client = Some(client);
    }

    pub(crate) fn prepare_delivery_human(&self) -> Result<PreparedHuman, String> {
        // A notice claimed by a core that may still be rolled back would be
        // lost with its copy; the core that keeps the ledger claims it.
        if self.effects_held() {
            return Err("core_pending".into());
        }
        self.delivery_state()?;
        Ok(PreparedHuman::new(
            self.delivery_client.clone().ok_or("delivery_unavailable")?,
        ))
    }

    /// Restores the panes the last run held as a draft and keeps the store
    /// to save every later change into.
    pub(crate) fn install_composer_store(
        &mut self,
        store: crate::composer_store::ComposerStore,
        restored: Vec<crate::composer_store::Mark>,
    ) {
        self.composer_marks = restored;
        self.composer_store = Some(store);
    }

    /// Makes the pane's mark say what its clocks say now: marked while an
    /// unsent draft stands in a pane the doorbell would ring, unmarked
    /// otherwise. A pane that is not observed keeps its mark until it comes
    /// back or the mark is old (`UNSEEN_TTL_MS`). Memory only here; the
    /// store's thread writes the file.
    fn sync_composer_mark(&mut self, pane_id: &str, now: u64) {
        use crate::composer_store::{MARK_LIMIT, Mark, UNSEEN_TTL_MS};
        let draft = self
            .delivery_observations
            .get(pane_id)
            .and_then(|observation| {
                (observation.unsent_draft()
                    && crate::delivery::doorbell::bell_target(&observation.actor.kind))
                .then(|| observation.actor.clone())
            });
        let held = self
            .composer_marks
            .iter()
            .position(|mark| mark.pane_id == pane_id);
        let mut changed = false;
        match (draft, held) {
            (Some(actor), Some(index)) if self.composer_marks[index].matches(&actor) => {}
            (Some(actor), Some(index)) => {
                self.composer_marks[index] = Mark::of(&actor, now);
                changed = true;
            }
            (Some(actor), None) => {
                self.composer_marks.push(Mark::of(&actor, now));
                changed = true;
            }
            (None, Some(index)) => {
                self.composer_marks.remove(index);
                changed = true;
            }
            (None, None) => {}
        }
        if !changed {
            return;
        }
        let observed = &self.delivery_observations;
        self.composer_marks.retain(|mark| {
            observed.contains_key(&mark.pane_id)
                || now.saturating_sub(mark.since_unix_ms) < UNSEEN_TTL_MS
        });
        if self.composer_marks.len() > MARK_LIMIT {
            self.composer_marks
                .sort_by_key(|mark| std::cmp::Reverse(mark.since_unix_ms));
            self.composer_marks.truncate(MARK_LIMIT);
        }
        if let Some(store) = &self.composer_store {
            store.save(self.composer_marks.clone());
        }
    }

    pub(crate) fn delivery_state(&self) -> Result<Arc<Ledger>, String> {
        self.delivery_ledger.clone()
    }

    pub(crate) fn naming_client(&self) -> Option<Client> {
        (!self.effects_held())
            .then(|| self.delivery_client.clone())
            .flatten()
    }

    pub(crate) fn invalidate_delivery(&mut self) {
        // Preserve installed bytes for startup recovery. In particular no
        // subsequent pull, tick or bell may acknowledge our old memory image.
        self.delivery_ledger = Err("ledger_unavailable".into());
    }

    pub(crate) fn publish_delivery(&mut self, ledger: Arc<Ledger>, transitions: bool) -> bool {
        let delivered: Vec<_> = ledger
            .letters
            .iter()
            .filter(|letter| {
                letter.state == crate::delivery::ledger::State::Delivered
                    && self.delivery_ledger.as_ref().ok().is_some_and(|previous| {
                        previous
                            .letters
                            .iter()
                            .any(|old| old.id == letter.id && old.state != letter.state)
                    })
            })
            .map(|letter| letter.recipient.pane_id.clone())
            .collect();
        let mut reopened = false;
        for pane in delivered {
            reopened |= self.reopen_resolved_session(&pane);
        }
        let before = self.delivery_ledger.as_ref().ok().map(|ledger| {
            ledger
                .watches
                .iter()
                .map(|watch| watch.view())
                .collect::<Vec<_>>()
        });
        let after = ledger
            .watches
            .iter()
            .map(|watch| watch.view())
            .collect::<Vec<_>>();
        let changed = transitions && before.as_ref() != Some(&after);
        if changed {
            self.snapshot.delivery_watches = after;
        }
        if !self.registrations_gone.is_empty() {
            self.registrations_gone.retain(|id, _| {
                ledger
                    .agents
                    .iter()
                    .any(|record| &record.id == id && !record.ended)
            });
        }
        self.registration_rebinds.retain(|id, rebind| {
            ledger.agents.iter().any(|record| {
                &record.id == id && !record.ended && record.actor.same_identity(&rebind.from)
            })
        });
        // Only a change in who waits for a reply, from whom, and whether that
        // agent took the letter in moves a row's wait, so the lineage is not
        // rebuilt for every other step of a letter's delivery.
        let waiters = |ledger: &Ledger| {
            ledger
                .letters
                .iter()
                .filter(|letter| letter.waiting_answer)
                .map(|letter| {
                    (
                        (letter.sender.pane_id.clone(), letter.sender.session.clone()),
                        (
                            letter.recipient.pane_id.clone(),
                            letter.recipient.session.clone(),
                        ),
                        letter.awaiting_intake(),
                    )
                })
                .collect::<std::collections::BTreeSet<_>>()
        };
        let waiters_moved = self
            .delivery_ledger
            .as_ref()
            .ok()
            .is_none_or(|previous| waiters(previous) != waiters(&ledger));
        // Only a change in what Hide started moves a row's mark or a checkout's
        // group, so a letter's step or a watch never rebuilds the lineage.
        let started = crate::coordination::started::hide_started(&ledger);
        let started_moved = started != self.hide_started;
        // An agent whose kind this build has no home for stays an operator's
        // row (PRD hide-ai-work-home D-19); said once, when it first shows.
        for agent in started.agents.iter().filter(|agent| {
            agent
                .task
                .as_ref()
                .is_none_or(|task| task.kind.home().is_none())
                && !self
                    .hide_started
                    .agents
                    .iter()
                    .any(|known| known.agent_id == agent.agent_id)
        }) {
            crate::diagnostic!(serde_json::json!({
                "component": "coordination",
                "kind": "hide_ai.task_kind_unknown",
                "agent_id": agent.agent_id,
                "task_kind": agent.task.as_ref().map(|task| task.kind.as_str().chars().take(64).collect::<String>()),
            }));
        }
        self.hide_started = started;
        let feedback = self.observe_feedback_letters(&ledger);
        self.delivery_ledger = Ok(ledger);
        let continued = self.settle_dormant_registrations();
        self.feed_link_parents();
        let lineage = (waiters_moved || started_moved) && self.refresh_agent_lineage();
        changed
            | reopened
            | feedback
            | continued
            | lineage
            | self.sync_session_state()
            | self.refresh_agent_scopes()
    }

    /// The Factory host publishes which Factories exist.
    /// The open Factories, each with the inactivity window its workers'
    /// watches use.
    pub(crate) fn set_factory_recipients(
        &mut self,
        factories: std::collections::BTreeMap<String, u64>,
    ) {
        self.factory_recipients = factories;
    }

    /// A code-owned recipient is current while its Factory exists and the
    /// actor is exactly the one the host constructs.
    pub(crate) fn factory_recipient_current(&self, actor: &Actor) -> bool {
        actor
            .pane_id
            .strip_prefix(crate::delivery::FACTORY_PREFIX)
            .is_some_and(|id| {
                self.factory_recipients.contains_key(id)
                    && *actor == Actor::factory(id, self.node.as_str())
            })
    }

    /// The authority a code-owned caller acts with: a Factory host's own
    /// recipient, or Hide's own identity for one of its roles; never a pane's.
    pub(crate) fn code_owned_authority_current(&self, caller: &str, actor: &Actor) -> bool {
        caller == actor.pane_id
            && (self.factory_recipient_current(actor)
                || *actor == Actor::hide(crate::delivery::HIDE_MEMORY_ROLE, self.node.as_str())
                || *actor == Actor::operator(self.node.as_str()))
    }

    /// A delivery command the operator's Send makes, as the operator's own
    /// identity, to the agent in `target` (PRD view-opener-feedback D-17).
    /// Only the shell's `view_feedback_send` reaches this: neither a pane's
    /// scoped connection nor the CLI can represent that event.
    pub(crate) fn operator_prepare(
        &self,
        target: &str,
        command: crate::delivery::Command,
    ) -> Result<crate::delivery::worker::Prepared, String> {
        let actor = Actor::operator(self.node.as_str());
        let context = Context {
            device_id: self.node.as_str().into(),
            workspace_id: actor.pane_id.clone(),
            checkout_id: actor.pane_id.clone(),
            checkout_path: String::new(),
        };
        let target = self
            .delivery_observations
            .get(target)
            .cloned()
            .ok_or("target_unavailable")?;
        Ok(crate::delivery::worker::Prepared::new(
            self.delivery_client.clone().ok_or("delivery_unavailable")?,
            Authority {
                caller: actor.pane_id.clone(),
                context,
            },
            actor,
            Some(target),
            command,
        ))
    }

    /// A delivery client and authority for Hide's own `role`, the origin of
    /// the agents Hide starts for the operator (PRD project-memory-v2 D-25).
    pub(crate) fn hide_delivery(
        &self,
        role: &str,
    ) -> Result<(crate::delivery::worker::Client, Authority, Actor), String> {
        let actor = Actor::hide(role, self.node.as_str());
        let context = Context {
            device_id: self.node.as_str().into(),
            workspace_id: actor.pane_id.clone(),
            checkout_id: actor.pane_id.clone(),
            checkout_path: String::new(),
        };
        Ok((
            self.delivery_client.clone().ok_or("delivery_unavailable")?,
            Authority {
                caller: actor.pane_id.clone(),
                context,
            },
            actor,
        ))
    }

    /// A delivery client and authority for the Factory host (D-14).
    pub(crate) fn factory_delivery(
        &self,
        id: &str,
    ) -> Result<(crate::delivery::worker::Client, Authority, Actor), String> {
        let actor = Actor::factory(id, self.node.as_str());
        if !self.factory_recipient_current(&actor) {
            return Err("factory_unavailable".into());
        }
        let context = Context {
            device_id: self.node.as_str().into(),
            workspace_id: actor.pane_id.clone(),
            checkout_id: actor.pane_id.clone(),
            checkout_path: String::new(),
        };
        Ok((
            self.delivery_client.clone().ok_or("delivery_unavailable")?,
            Authority {
                caller: actor.pane_id.clone(),
                context,
            },
            actor,
        ))
    }

    pub(crate) fn delivery_identity_current(&self, actor: &Actor) -> bool {
        self.delivery_observations
            .get(&actor.pane_id)
            .is_some_and(|observation| observation.actor.same_identity(actor))
    }

    pub(crate) fn delivery_connector(
        &self,
        device: &str,
    ) -> Option<Arc<dyn hide_herdr_client::ApiConnector>> {
        if self.node == device {
            self.live.as_ref().map(|live| live.api_connector.clone())
        } else {
            self.remote_herdr_api(device)
        }
    }

    #[cfg(test)]
    pub(crate) fn delivery_observation(&self, actor: &Actor) -> Option<Observation> {
        self.delivery_observations
            .get(&actor.pane_id)
            .filter(|observation| observation.actor.same_identity(actor))
            .cloned()
    }

    /// The title Hide's screens give `actor` while its pane still hosts it,
    /// for the bell line that names it as a letter's writer. A row with no
    /// title is called by its runtime's name, which tells the operator
    /// nothing about who wrote, so it has none here.
    pub(crate) fn delivery_title(&self, actor: &Actor) -> Option<String> {
        if !self.delivery_identity_current(actor) {
            return None;
        }
        self.agent_row(&actor.pane_id)
            .filter(|row| {
                row.identity_label != crate::sidebar::provider_name(Some(&row.agent_kind))
            })
            .map(|row| row.identity_label.clone())
    }

    /// Whether a bell may be typed into the recipient's pane now for a letter
    /// that waits as `patience` says, and the observation the verdict was
    /// read from. The refusal says which fact
    /// held the letter, for the diagnostic log.
    pub(crate) fn delivery_bell_verdict(
        &self,
        recipient: &Actor,
        patience: crate::delivery::doorbell::Patience,
        now: u64,
    ) -> Result<Observation, crate::delivery::doorbell::Hold> {
        use crate::delivery::doorbell::Hold;
        if self.effects_held() {
            return Err(Hold::CorePending);
        }
        let current = self
            .delivery_observations
            .get(&recipient.pane_id)
            .filter(|observation| observation.actor.device_id == recipient.device_id)
            .ok_or(Hold::Absent)?;
        if !current.actor.same_identity(recipient) {
            return Err(Hold::Session);
        }
        crate::delivery::doorbell::judge(current, patience, now)?;
        Ok(current.clone())
    }

    /// The final memory guard immediately before the off-lock pane write.
    /// The refusal names what moved since the verdict, for the diagnostic
    /// log.
    pub(crate) fn delivery_bell_current(
        &self,
        id: &str,
        observed: &Observation,
        reserved: Option<u8>,
    ) -> Result<(), crate::delivery::doorbell::Hold> {
        use crate::delivery::doorbell::Hold;
        let current = self
            .delivery_observations
            .get(&observed.actor.pane_id)
            .ok_or(Hold::Absent)?;
        if !current.actor.same_identity(&observed.actor) {
            return Err(Hold::Session);
        }
        if current.status != observed.status
            || current.state_change_seq != observed.state_change_seq
        {
            return Err(Hold::Moved);
        }
        if current.last_input_at_unix_ms != observed.last_input_at_unix_ms {
            return Err(Hold::Input);
        }
        let now = unix_milliseconds();
        let letter = self
            .delivery_ledger
            .as_ref()
            .ok()
            .and_then(|ledger| {
                ledger.letters.iter().find(|letter| {
                    letter.id == id
                        && letter.recipient.same_identity(&observed.actor)
                        && letter.state == crate::delivery::ledger::State::Pending
                        && match reserved {
                            Some(attempt) => {
                                (1..=3).contains(&attempt) && letter.attempts() == attempt
                            }
                            None => letter.attempts() < 3,
                        }
                        && letter.within_deadline(&ledger.node_clocks, now)
                })
            })
            .ok_or(Hold::Letter)?;
        crate::delivery::doorbell::judge(
            current,
            crate::delivery::doorbell::Patience::of(letter),
            now,
        )
    }

    pub(crate) fn delivery_watch_work(&mut self) -> Vec<crate::delivery::worker::WatchWork> {
        // No reading, so no warning, until the move's link commits this core.
        if self.effects_held() {
            return Vec::new();
        }
        let Ok(ledger) = self.delivery_state() else {
            return Vec::new();
        };
        ledger
            .watches
            .iter()
            .map(|watch| {
                let current = self.delivery_observations.get(&watch.target.pane_id);
                let observation = current
                    .filter(|observation| {
                        watch.target.require_native_identity().is_ok()
                            && observation.actor.require_native_identity().is_ok()
                            && observation.actor.same_identity(&watch.target)
                    })
                    .cloned();
                // Acquisition or temporary loss of a native reference does
                // not prove an execution ended. Preserve the original binding
                // and clocks, and withhold unproven status/file metadata.
                let proven_absence_or_replacement = match current {
                    None => true,
                    Some(observation) => matches!(
                        (&watch.target.session, &observation.actor.session),
                        (Some(original), Some(current)) if original != current
                    ),
                };
                let gone = proven_absence_or_replacement
                    && self.delivery_connected.contains(&watch.target.device_id)
                    && !self.delivery_overflow.contains(&watch.target.device_id)
                    && !self.dormant_holds_actor(&watch.target);
                let source = crate::delivery::worker::ActivitySource {
                    link: if self.node == watch.target.device_id {
                        Some(Arc::clone(&self.own_node))
                    } else {
                        observation
                            .is_some()
                            .then(|| self.node_link(&watch.target.device_id).ok())
                            .flatten()
                    },
                };
                // The target's own row, while its pane still runs the watched
                // session: stopped on a question, or held at a Herdr-blocked
                // prompt. A blocked one also raises the child to the operator
                // (Delegated escalation); a working one, such as Codex's async
                // question, has not stopped.
                let row = observation
                    .as_ref()
                    .and_then(|_| self.agent_row(&watch.target.pane_id))
                    .filter(|row| row.lineage_session == watch.target.session);
                let blocked = row.is_some_and(|row| row.blocked);
                let asking = row.is_some_and(|row| {
                    row.demand == "question" && row.activity == "stopped" && !row.blocked
                });
                crate::delivery::worker::WatchWork {
                    id: watch.id.clone(),
                    observation,
                    gone,
                    status: watch.last_status.clone(),
                    state_change_seq: watch.last_state_change_seq,
                    status_changed_at_unix_ms: watch.status_changed_at_unix_ms,
                    source,
                    inactivity_ms: watch
                        .parent
                        .pane_id
                        .strip_prefix(crate::delivery::FACTORY_PREFIX)
                        .and_then(|id| self.factory_recipients.get(id).copied()),
                    asking,
                    blocked,
                }
            })
            .collect()
    }
}

impl Runtime {
    pub(crate) fn coordination_fork_context(
        &self,
        pane: &str,
    ) -> Result<(Client, Authority, Actor), String> {
        let actor = self
            .delivery_observations
            .get(pane)
            .ok_or("parent_unavailable")?
            .actor
            .clone();
        let context = self
            .workspace_control_query(&actor.device_id, pane, Query::Info)
            .map_err(|_| "parent_unavailable")?
            .context;
        Ok((
            self.delivery_client.clone().ok_or("delivery_unavailable")?,
            Authority {
                caller: pane.into(),
                context,
            },
            actor,
        ))
    }
    pub(crate) fn coordination_context(&self, device: &str) -> Result<CoordinationContext, String> {
        let on_node = self.node == device;
        let (connector, scope, machine) = if on_node {
            let live = self.live.as_ref().ok_or("herdr_unavailable")?;
            (
                live.api_connector.clone(),
                live.socket_path
                    .to_str()
                    .ok_or("host_scope_unavailable")?
                    .to_owned(),
                self.node.to_string(),
            )
        } else {
            (
                self.remote_herdr_api(device).ok_or("device_unavailable")?,
                device.to_owned(),
                self.device_machine_ids
                    .get(device)
                    .cloned()
                    .ok_or("machine_identity_unavailable")?,
            )
        };
        let node = if on_node {
            Some(self.own_node())
        } else {
            self.ready_device_channels()
                .into_iter()
                .find_map(|(ready, node)| (ready == device).then_some(node))
        };
        Ok(CoordinationContext {
            connector,
            host_scope: scope,
            machine,
            codex: self.codex_daemon(device),
            on_node,
            node,
        })
    }
}

impl Runtime {
    /// The node link of the device a spawn names with `--machine`, or why it
    /// cannot take the spawn: a refusal code the caller can branch on, with
    /// the ids it could have named when the id is not a device at all. The
    /// caller's own device is never named here (the spawn drops it first), and a
    /// caller that is not on this machine may name no other device.
    pub(crate) fn spawn_target(
        &mut self,
        caller_device: &str,
        device: &str,
    ) -> Result<Arc<dyn crate::node_access::NodeLink>, String> {
        // Only an agent on this machine starts work on a device: one on a
        // device would otherwise reach this machine or a third one, and the
        // refusal names no device so it maps nothing.
        if caller_device != self.node.as_str() {
            return Err("machine_not_permitted".into());
        }
        let registered = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .any(|registration| registration.id == device);
        if device != self.node.as_str() && !registered {
            let mut connected: Vec<String> = self
                .snapshot
                .ui_state
                .device_registrations
                .iter()
                .map(|registration| registration.id.clone())
                .chain(std::iter::once(self.node.to_string()))
                .filter(|id| id != caller_device && self.coordination_context(id).is_ok())
                .collect();
            connected.sort();
            return Err(crate::coordination::refusal(
                "machine_unknown",
                &connected.join(", "),
            ));
        }
        self.coordination_context(device)
            .map_err(|_| "machine_unavailable".to_owned())?;
        self.node_link(device)
            .map_err(|_| "machine_unavailable".to_owned())
    }
}

/// What a coordination command needs of the machine it acts on.
pub(crate) struct CoordinationContext {
    pub(crate) connector: Arc<dyn hide_herdr_client::ApiConnector>,
    pub(crate) host_scope: String,
    pub(crate) machine: String,
    pub(crate) codex: crate::codex_launch::CodexDaemon,
    /// The machine is the core's own node, whose Herdr the core reaches directly.
    pub(crate) on_node: bool,
    /// The machine's node, which a start with a long line asks whether the
    /// pane's shell is reading; none for a device whose helper is not
    /// connected now, since a spawn starts no helper.
    pub(crate) node: Option<Arc<dyn crate::node_access::NodeLink>>,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    /// A letter whose wait is over, or that never had one: the bell's
    /// verdict for a pane at rest, where the wait plays no part.
    const WAITED: crate::delivery::doorbell::Patience = crate::delivery::doorbell::Patience {
        arrived_at_unix_ms: 0,
        now: false,
    };
    use crate::delivery::worker::Worker;
    use crate::handle::ChangeNotifier;
    use crate::model::{CoreOptions, SCHEMA_VERSION};
    use crate::sidebar::SessionSnapshotPayload;
    use hide_node_link::terminal::TerminalReport;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// The state a retired coordinator must leave exactly as its owner left it.
    pub(crate) fn coordinator_memory(runtime: &Runtime) -> serde_json::Value {
        let panes = runtime
            .delivery_panes
            .iter()
            .map(|(device, read)| {
                let panes = read
                    .panes
                    .as_ref()
                    .map(|panes| panes.iter().collect::<std::collections::BTreeSet<_>>());
                (
                    device,
                    json!({"scope": read.host_scope, "floor": read.floor, "panes": panes}),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let observations = runtime
            .delivery_observations
            .iter()
            .map(|(pane, observed)| {
                (
                    pane,
                    json!({
                        "actor": observed.actor, "raw_pane": observed.raw_pane_id,
                        "status": observed.status, "sequence": observed.state_change_seq,
                        "changed": observed.status_changed_at_unix_ms,
                        "input": observed.last_input_at_unix_ms,
                        "submit": observed.last_submit_at_unix_ms,
                        "working": observed.entered_working_at_unix_ms,
                        "released": observed.released_input_at_unix_ms,
                        "scope": observed.host_scope, "turn": format!("{:?}", observed.turn),
                        "session": observed.session.as_ref().map(|session| json!({
                            "kind": session.reference_kind, "value": session.reference_value,
                            "cwd": session.cwd,
                        })),
                    }),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let gone = runtime
            .registrations_gone
            .iter()
            .map(|(id, reason)| (id, reason.reason()))
            .collect::<std::collections::BTreeMap<_, _>>();
        json!({
            "panes": panes, "observations": observations, "gone": gone,
            "overflow": runtime.delivery_overflow.iter().collect::<std::collections::BTreeSet<_>>(),
            "connected": runtime.delivery_connected.iter().collect::<std::collections::BTreeSet<_>>(),
            "remote": runtime.snapshot.status.remote,
            "raw_sessions": runtime.device_raw_sessions.iter().collect::<std::collections::BTreeMap<_, _>>(),
            "generations": runtime.remote_connection_generations.iter().collect::<std::collections::BTreeMap<_, _>>(),
        })
    }

    pub(crate) fn coordinator_bootstrap(
        runtime: &mut Runtime,
        device: &str,
        connector: &Arc<dyn hide_herdr_client::ApiConnector>,
    ) {
        let control = runtime
            .remote_controls
            .get(device)
            .expect("installed owner");
        assert!(Arc::ptr_eq(&control.api_connector(), connector));
        let status = runtime
            .snapshot
            .status
            .remote
            .iter_mut()
            .find(|status| status.target_id == device)
            .expect("registered device status");
        // A Reconnect keeps the last session on screen until the new
        // coordinator publishes its own, so only the connection starts over.
        status.state = "not_connected".into();
        status.message = None;
    }

    pub(crate) fn fixture(
        root: &std::path::Path,
    ) -> (Arc<Mutex<Runtime>>, Actor, Observation, PathBuf) {
        let state = root.join("state");
        hide_platform::fs::private::create_dir_all(&state).unwrap();
        let options: CoreOptions = serde_json::from_value(json!({
            "schema_version":SCHEMA_VERSION,"node_id":"test-node","home":root,"herdr_socket_path":null,
            "app_state_path":state.join("app.json"),
            "workspace_views_path":root.join("views.json"),
        }))
        .unwrap();
        let mut runtime = Runtime::new(
            options,
            crate::environment::EnvironmentReport {
                statuses: Vec::new(),
                home_path: Some(root.to_owned()),
                codex_home: None,
            },
            std::sync::Arc::new(hide_node::Local::new(Some(root.to_owned()))),
            crate::node::test_devices(),
        );
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
            {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"recipient-session"},
        ]})).unwrap();
        runtime.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
        runtime.snapshot.status.herdr.state = "connected".into();
        let panes = ["sender", "recipient"]
            .into_iter()
            .map(|id| crate::model::PaneSnapshot {
                terminal_id: None,
                failed_terminal_id: None,
                id: id.into(),
                herdr_label: None,
                terminal_title: None,
                cwd: "/checkouts/fixture".into(),
                status_code: crate::model::AgentStatusCode::Attached,
                requires_close_confirmation: false,
                requires_close_status_check: false,
                identity_label: None,
                activity_at_unix_ms: None,
                fork: Default::default(),
                ports: Vec::new(),
                servers: Vec::new(),
                children: None,
                lineage_path: Vec::new(),
                sleep: None,
                sleep_action: None,
            })
            .collect();
        runtime.snapshot.navigator.workspaces = vec![crate::model::WorkspaceSnapshot {
            agent_presence: Default::default(),
            agent_scope: Default::default(),
            home_issues: Default::default(),
            tasks: Default::default(),
            pull_requests: Vec::new(),
            id: "workspace".into(),
            label: "Fixture".into(),
            path: "/checkouts/fixture".into(),
            remote_target_id: None,
            expanded: true,
            device_id: crate::node::TEST_NODE.into(),
            repo_name: "fixture".into(),
            is_git: false,
            default_branch: None,
            branches: Vec::new(),
            registered: true,
            temporary: false,
            session_workspace_ids: Vec::new(),
            last_activity_unix_ms: None,
            pinned: false,
            is_home: false,
            checkouts: vec![crate::model::CheckoutSnapshot {
                id: "checkout".into(),
                workspace_id: "workspace".into(),
                path: "/checkouts/fixture".into(),
                exists: true,
                has_panes: true,
                tabs: vec![crate::model::TabSnapshot {
                    naming: Default::default(),
                    agent: None,
                    id: Some("tab".into()),
                    workspace_id: Some("workspace".into()),
                    checkout_id: Some("checkout".into()),
                    label: None,
                    empty: false,
                    delegated: false,
                    panes,
                }],
                ..Default::default()
            }],
            inactive_checkouts: Default::default(),
            session_folds: Default::default(),
            removal: Default::default(),
            disk: Default::default(),
            cleanup: None,
        }];
        let actor = Actor {
            pane_id: "sender".into(),
            name: "sender".into(),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: Some("sender-session".into()),
        };
        let target = runtime
            .delivery_observation(&Actor {
                pane_id: "recipient".into(),
                name: "recipient".into(),
                kind: "codex".into(),
                device_id: crate::node::TEST_NODE.into(),
                session: Some("recipient-session".into()),
            })
            .unwrap();
        (
            Arc::new(Mutex::new(runtime)),
            actor,
            target,
            hide_kit::layout::delivery_ledger(&state),
        )
    }

    pub(crate) fn authority(actor: &Actor) -> Authority {
        Authority {
            caller: actor.pane_id.clone(),
            context: Context {
                device_id: crate::node::TEST_NODE.into(),
                workspace_id: "workspace".into(),
                checkout_id: "checkout".into(),
                checkout_path: "/checkouts/fixture".into(),
            },
        }
    }

    /// Puts the fixture's recipient at rest in native session `native`, idle
    /// with no key and no change of status for long past the quiet period,
    /// and makes `herdr` the Herdr this machine's panes are reached through.
    /// The agent rows the sidebar shows on this machine.
    pub(crate) fn show_agents(runtime: &mut Runtime, payload: SessionSnapshotPayload) {
        runtime.snapshot.navigator.agents = crate::sidebar::project_agents(payload).agents;
    }

    pub(crate) fn recipient_at_rest(
        runtime: &mut Runtime,
        native: &str,
        herdr: &crate::fake_herdr::FakeHerdr,
    ) -> Observation {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
            {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"idle","state_change_seq":2,
             "lineage_session":crate::wire::session_digest(native)},
        ]})).unwrap();
        runtime.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
        let observation = runtime.delivery_observations.get_mut("recipient").unwrap();
        read_nothing_waits(observation);
        observation.status_changed_at_unix_ms = 0;
        observation.last_input_at_unix_ms = 0;
        observation.last_submit_at_unix_ms = 0;
        observation.entered_working_at_unix_ms = 0;
        let observation = observation.clone();
        runtime.live = Some(crate::live::LiveContext {
            socket_path: herdr.socket_path().to_path_buf(),
            runtime: std::sync::Weak::new(),
            notifier: ChangeNotifier::noop(),
            api_connector: Arc::new(herdr.connector()),
            node: Arc::new(hide_node::Local::of_process()),
        });
        observation
    }

    /// The pane as the core observes it now.
    pub(crate) fn observed(runtime: &Runtime, pane: &str) -> Observation {
        runtime.delivery_observations[pane].clone()
    }

    /// The pane as the core observes it working since `since`, confirmed by
    /// Herdr, with no key typed since.
    pub(crate) fn observe_working_since(runtime: &mut Runtime, pane: &str, since: u64) {
        let observation = runtime.delivery_observations.get_mut(pane).unwrap();
        observation.status = "working".into();
        observation.entered_working_at_unix_ms = since;
        observation.status_changed_at_unix_ms = since;
        observation.last_input_at_unix_ms = since - 1;
        observation.last_submit_at_unix_ms = since - 1;
    }

    /// The core's Herdr link, as a device's connection makes and drops it.
    pub(crate) fn set_live(runtime: &mut Runtime, herdr: Option<&crate::fake_herdr::FakeHerdr>) {
        runtime.live = herdr.map(|herdr| crate::live::LiveContext {
            socket_path: herdr.socket_path().to_path_buf(),
            runtime: std::sync::Weak::new(),
            notifier: ChangeNotifier::noop(),
            api_connector: Arc::new(herdr.connector()),
            node: Arc::new(hide_node::Local::of_process()),
        });
    }

    fn observe_recipient_status(runtime: &mut Runtime, status: &str, sequence: u64) {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
            {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":status,"state_change_seq":sequence,"lineage_session":"recipient-session"},
        ]})).unwrap();
        runtime.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
        read_nothing_waits(runtime.delivery_observations.get_mut("recipient").unwrap());
    }

    /// The recipient's session read says nothing waits for the operator in
    /// its current state, as the label overlay answers once it has read it;
    /// what the read says is tested on its own
    /// (`the_session_read_decides_the_turn_for_the_current_state_only`).
    pub(crate) fn read_nothing_waits(observation: &mut Observation) {
        observation.turn = Turn::Read(hide_session::turns::Waiting::Nothing);
    }

    /// What the recipient's session read says, as a doorbell test sets it.
    pub(crate) fn recipient_turn(runtime: &mut Runtime, turn: Turn) {
        runtime
            .delivery_observations
            .get_mut("recipient")
            .unwrap()
            .turn = turn;
    }

    /// The three facts the bell reads, per pane, optionally set to zero so a
    /// test sees which of them an event wrote.
    fn clocks(runtime: &mut Runtime, pane: &str, reset: bool) -> (u64, u64, u64) {
        let observation = runtime.delivery_observations.get_mut(pane).unwrap();
        let read = (
            observation.last_input_at_unix_ms,
            observation.last_submit_at_unix_ms,
            observation.entered_working_at_unix_ms,
        );
        if reset {
            observation.last_input_at_unix_ms = 0;
            observation.last_submit_at_unix_ms = 0;
            observation.entered_working_at_unix_ms = 0;
        }
        read
    }

    /// Which clocks an event wrote since the last call, as (key, submit, work).
    fn written(runtime: &mut Runtime) -> (bool, bool, bool) {
        let (input, submit, working) = clocks(runtime, "recipient", true);
        (input > 0, submit > 0, working > 0)
    }

    #[test]
    fn no_hide_key_ends_a_draft_not_even_the_enter_that_opens_a_picker() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        observe_recipient_status(&mut guard, "idle", 2);
        clocks(&mut guard, "recipient", true);
        clocks(&mut guard, "sender", true);

        // `/model` and Enter: the picker is open and Herdr reads the pane
        // `done`, so the bell must keep holding on a draft.
        // The node reports `/model`, Esc and Esc-Return as typing and the
        // Return as a submit; none of them is a delivery submit.
        for submitted in [false, false, false, true] {
            crate::runtime::tests::typed_into(&mut guard, "recipient", submitted);
            assert_eq!(written(&mut guard), (true, false, false), "{submitted}");
        }
        let observation = guard.delivery_observations.get_mut("recipient").unwrap();
        observation.last_input_at_unix_ms = unix_milliseconds().saturating_sub(120_000);
        observation.status_changed_at_unix_ms = unix_milliseconds().saturating_sub(120_000);
        let held = guard
            .delivery_observations
            .get("recipient")
            .map(|observation| {
                crate::delivery::doorbell::judge(observation, WAITED, unix_milliseconds())
            });
        assert_eq!(held, Some(Err(crate::delivery::doorbell::Hold::Draft)));
        assert_eq!(
            clocks(&mut guard, "sender", false),
            (0, 0, 0),
            "another pane's clocks do not move"
        );

        // The phone's reply is a hide key and a submit.
        observe_recipient_status(&mut guard, "idle", 4);
        clocks(&mut guard, "recipient", true);
        guard.dispatch_json(
            &serde_json::to_vec(&json!({
                "schema_version": SCHEMA_VERSION, "kind": "pane_input_submitted",
                "payload": {"pane_id": "recipient"},
            }))
            .unwrap(),
        );
        assert_eq!(written(&mut guard), (true, true, false));
    }

    /// A key's report as the pane's node sends it.
    fn key_report(pane: &str, at_unix_ms: u64, submitted: bool) -> TerminalReport {
        TerminalReport::Input {
            pane: pane.to_owned(),
            at_unix_ms,
            submitted,
            focus: false,
        }
    }

    /// PRD core-host-node-terminal B5: a node reports a key without waiting
    /// for the runtime lock, however long it is held, and a key into the
    /// pane the keyboard is already in publishes nothing.
    #[test]
    fn keys_are_reported_while_the_runtime_lock_is_held_and_publish_nothing() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        let (channel, reports) = crate::terminal_reports::terminal_reports();
        let pump = crate::terminal_reports::ReportPump::spawn(
            reports,
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
        )
        .unwrap();
        let mut guard = runtime.lock().unwrap();
        observe_recipient_status(&mut guard, "idle", 2);
        clocks(&mut guard, "recipient", true);
        let (sent, done) = std::sync::mpsc::channel();
        let node = std::thread::spawn(move || {
            for at in 1..=10_000 {
                hide_node_link::terminal::ReportSink::report(
                    &channel,
                    key_report("recipient", at, false),
                );
            }
            sent.send(()).unwrap();
        });
        // Every report went out while this test still holds the lock.
        done.recv_timeout(Duration::from_secs(30))
            .expect("a report waited for the runtime lock");
        node.join().unwrap();
        assert_eq!(clocks(&mut guard, "recipient", false), (0, 0, 0));
        drop(guard);
        drop(pump);
        let mut guard = runtime.lock().unwrap();
        assert_eq!(clocks(&mut guard, "recipient", false).0, 10_000);
        // A screen's key names the pane it was typed into: the first one
        // moves the keyboard there, and the rest, Enter included, change
        // nothing a screen draws.
        let typed = |at, submitted| TerminalReport::Input {
            pane: "recipient".to_owned(),
            at_unix_ms: at,
            submitted,
            focus: true,
        };
        assert!(guard.ingest_terminal_reports(vec![typed(10_001, false)]));
        assert!(!guard.ingest_terminal_reports(vec![typed(10_002, false)]));
        assert!(!guard.ingest_terminal_reports(vec![typed(10_003, true)]));
    }

    /// A runtime lock held long enough to fill the report queue loses no
    /// pane's state: past the limit a report takes the place of the waiting
    /// one about the same pane, so the core still ends at the node's last
    /// word and the last key's time. The queue fills before its pump starts,
    /// as it does behind a held lock.
    #[test]
    fn reports_past_the_waiting_limit_keep_a_panes_last_state_and_key() {
        use hide_node_link::terminal::{PaneTerminalState, ReportSink};
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        {
            let mut guard = runtime.lock().unwrap();
            observe_recipient_status(&mut guard, "idle", 2);
            clocks(&mut guard, "recipient", true);
        }
        let (channel, reports) = crate::terminal_reports::terminal_reports();
        let state = |state: &str| TerminalReport::State {
            pane: "recipient".to_owned(),
            state: PaneTerminalState {
                state: state.to_owned(),
                mode: Some("control".to_owned()),
                generation: 2,
                attempt: 1,
                message: None,
                exit_category: None,
                retry_decision: "none".to_owned(),
                last_attempt_at_unix_ms: None,
            },
        };
        for at in 1..=20_000 {
            channel.report(key_report("recipient", at, false));
        }
        channel.report(state("controlling"));
        for at in 20_001..=20_100 {
            channel.report(key_report("recipient", at, false));
        }
        channel.report(state("closing"));
        let pump = crate::terminal_reports::ReportPump::spawn(
            reports,
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
        )
        .unwrap();
        drop(pump);
        let mut guard = runtime.lock().unwrap();
        assert_eq!(
            guard
                .terminal_states
                .get("recipient")
                .map(|state| state.state.as_str()),
            Some("closing")
        );
        assert_eq!(clocks(&mut guard, "recipient", false).0, 20_100);
    }

    /// PRD core-host-node-terminal B7: the node reports the first key after
    /// a quiet pane at once and folds the keys behind it into one report a
    /// second later, so a letter that arrives inside that second is held
    /// like one that arrives after it.
    #[test]
    fn a_letter_right_after_the_first_key_on_a_quiet_pane_is_held() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        observe_recipient_status(&mut guard, "idle", 2);
        let observation = guard.delivery_observations.get_mut("recipient").unwrap();
        let long_ago = unix_milliseconds().saturating_sub(120_000);
        observation.last_input_at_unix_ms = long_ago;
        observation.last_submit_at_unix_ms = long_ago;
        observation.status_changed_at_unix_ms = long_ago;
        observation.entered_working_at_unix_ms = long_ago;
        let judge = |runtime: &Runtime, now: u64| {
            crate::delivery::doorbell::judge(
                &runtime.delivery_observations["recipient"],
                WAITED,
                now,
            )
        };
        assert_eq!(judge(&guard, unix_milliseconds()), Ok(()));

        // The first key after the quiet spell is reported as it is typed.
        let first = unix_milliseconds();
        guard.ingest_terminal_reports(vec![key_report("recipient", first, false)]);
        // A second key 200 ms later waits at the node; a letter at 500 ms
        // finds the pane typed into a moment ago and a draft on it.
        assert_eq!(
            judge(&guard, first + 500),
            Err(crate::delivery::doorbell::Hold::Draft)
        );
        // The folded report lands a second after the first key, and the
        // draft still holds the letter once the pane is quiet again.
        guard.ingest_terminal_reports(vec![key_report("recipient", first + 200, false)]);
        assert_eq!(
            judge(&guard, first + 60_000),
            Err(crate::delivery::doorbell::Hold::Draft)
        );
    }

    /// A key's time comes from another clock: one far ahead of the core's
    /// counts as typed now, so the submit after it ends the draft instead
    /// of the pane holding every letter forever.
    #[test]
    fn a_key_from_a_clock_ahead_counts_as_typed_now() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        observe_recipient_status(&mut guard, "idle", 2);
        guard.ingest_terminal_reports(vec![key_report("recipient", u64::MAX, false)]);
        let typed = guard.delivery_observations["recipient"].last_input_at_unix_ms;
        assert!(typed <= unix_milliseconds(), "{typed}");
        guard
            .delivery_observations
            .get_mut("recipient")
            .unwrap()
            .last_submit_at_unix_ms = unix_milliseconds();
        assert_eq!(
            crate::delivery::doorbell::judge(
                &guard.delivery_observations["recipient"],
                WAITED,
                unix_milliseconds() + 120_000
            ),
            Ok(())
        );
    }

    #[test]
    fn a_phone_write_is_input_and_only_a_reply_is_a_submit() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        observe_recipient_status(&mut guard, "idle", 2);
        clocks(&mut guard, "recipient", true);
        guard.dispatch_json(
            &serde_json::to_vec(&json!({
                "schema_version": SCHEMA_VERSION, "kind": "pane_input_sent",
                "payload": {"pane_id": "recipient"},
            }))
            .unwrap(),
        );
        assert_eq!(written(&mut guard), (true, false, false));
    }

    #[test]
    fn entering_work_is_stamped_once_and_staying_in_it_or_leaving_it_is_not() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        observe_recipient_status(&mut guard, "idle", 2);
        clocks(&mut guard, "recipient", true);
        observe_recipient_status(&mut guard, "working", 3);
        assert_eq!(written(&mut guard), (false, false, true));
        observe_recipient_status(&mut guard, "working", 4);
        assert_eq!(written(&mut guard), (false, false, false));
        observe_recipient_status(&mut guard, "blocked", 5);
        assert_eq!(written(&mut guard), (false, false, false));
        // Answering the menu puts the agent back to work: a new entry.
        observe_recipient_status(&mut guard, "working", 6);
        assert_eq!(written(&mut guard), (false, false, true));
        observe_recipient_status(&mut guard, "idle", 7);
        assert_eq!(written(&mut guard), (false, false, false));
    }

    /// The overlay a label worker would publish for a Codex pane whose
    /// session read, under Herdr state `seq`, found a finished plan turn.
    fn plan_overlay(pane: &str, native: &str, seq: u64) -> LabelOverlay {
        use hide_session::turns::TurnMark;
        let turn = Some("turn-1".to_owned());
        read_overlay(
            pane,
            native,
            seq,
            &[
                TurnMark::Started {
                    turn: turn.clone(),
                    mode: hide_session::turns::TurnMode::Plan,
                },
                TurnMark::Plan { turn: turn.clone() },
                TurnMark::Completed { turn },
            ],
        )
    }

    /// The overlay for a Codex pane whose session read, under Herdr state
    /// `seq`, found these records.
    fn read_overlay(
        pane: &str,
        native: &str,
        seq: u64,
        marks: &[hide_session::turns::TurnMark],
    ) -> LabelOverlay {
        let mut turns = hide_session::turns::TurnTracker::default();
        for (offset, mark) in marks.iter().enumerate() {
            turns.fold(offset as u64 * 10, mark);
        }
        let record = crate::labels::store::PaneRecord {
            owner: hide_session::label_reference_token("codex", "id", native),
            turns: Some(turns),
            turns_seq: Some(seq),
            ..Default::default()
        };
        LabelOverlay::of_records([(&pane.to_owned(), &record)], true, false)
    }

    /// PRD codex-plan-approval-hold D-04, D-06, D-07: the bell's turn fact is
    /// the overlay's read for the agent's current state and session; another
    /// state, another session or no overlay is not known for an agent whose
    /// read reports turns, and an agent whose read reports none is unchanged.
    #[test]
    fn the_session_read_decides_the_turn_for_the_current_state_only() {
        use crate::delivery::doorbell::Turn;
        use hide_session::turns::Waiting;
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        let overlay = plan_overlay("recipient", "recipient-native", 2);
        let observe = |guard: &mut Runtime, agent: &str, seq: u64, native: &str, labels| {
            let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"recipient","pane_id":"recipient","agent":agent,"agent_status":"done",
                 "state_change_seq":seq,"lineage_session":"recipient-session",
                 "agent_session":{"kind":"id","value":native}},
            ]}))
            .unwrap();
            guard.observe_delivery(crate::node::TEST_NODE, &payload, None, labels);
            guard.delivery_observations["recipient"].turn
        };
        assert_eq!(
            observe(&mut guard, "codex", 2, "recipient-native", Some(&overlay)),
            Turn::Read(Waiting::PlanApproval)
        );
        assert_eq!(
            observe(&mut guard, "codex", 3, "recipient-native", Some(&overlay)),
            Turn::Unread,
            "a newer state than the read"
        );
        assert_eq!(
            observe(&mut guard, "codex", 2, "another-native", Some(&overlay)),
            Turn::Unread,
            "another session in the pane"
        );
        assert_eq!(
            observe(&mut guard, "codex", 2, "recipient-native", None),
            Turn::Unread
        );
        assert_eq!(
            observe(&mut guard, "claude", 2, "recipient-native", None),
            Turn::Unread,
            "a supported question reader must finish before the bell rings"
        );
        assert_eq!(
            observe(&mut guard, "opencode", 2, "recipient-native", None),
            Turn::Unread,
            "OpenCode's question reader finishes before a bell too"
        );
        assert_eq!(
            observe(&mut guard, "grok", 2, "recipient-native", None),
            Turn::Unread,
            "Grok's question and plan reader finishes before a bell too"
        );
        assert_eq!(
            observe(&mut guard, "cursor", 2, "recipient-native", None),
            Turn::NotReported
        );
        // Adapter D-08/B5: every spelling of one agent reads the same turn.
        for kind in [" Codex", "CODEX", "cOdEx"] {
            assert_eq!(
                observe(&mut guard, kind, 2, "recipient-native", Some(&overlay)),
                Turn::Read(Waiting::PlanApproval),
                "the normalized kind as the bell target and session reader read it"
            );
        }
    }

    /// B5: Herdr can read Codex done before its session file records the end
    /// of the turn. Only a plan-mode turn can end waiting for approval, so an
    /// ordinary turn still running in the file rings as before; an unfinished
    /// plan-mode turn is not known and holds until the next state is read.
    #[test]
    fn a_turn_not_yet_ended_in_the_file_rings_unless_it_runs_in_plan_mode() {
        use crate::delivery::doorbell::Hold;
        use hide_session::turns::{TurnMark, TurnMode};
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, target, _) = fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        let verdict = |guard: &mut Runtime, mode: TurnMode| {
            let started = TurnMark::Started {
                turn: Some("turn-1".to_owned()),
                mode,
            };
            let overlay = read_overlay("recipient", "recipient-native", 2, &[started]);
            let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"done",
                 "state_change_seq":2,"lineage_session":"recipient-session",
                 "agent_session":{"kind":"id","value":"recipient-native"}},
            ]}))
            .unwrap();
            guard.observe_delivery(crate::node::TEST_NODE, &payload, None, Some(&overlay));
            let long_ago = unix_milliseconds().saturating_sub(10 * 60_000);
            let observation = guard.delivery_observations.get_mut("recipient").unwrap();
            observation.last_input_at_unix_ms = long_ago;
            observation.last_submit_at_unix_ms = long_ago;
            observation.status_changed_at_unix_ms = long_ago;
            guard
                .delivery_bell_verdict(&target.actor, WAITED, unix_milliseconds())
                .err()
        };
        assert_eq!(
            verdict(&mut guard, TurnMode::Other),
            None,
            "a default-mode turn rings"
        );
        assert_eq!(
            verdict(&mut guard, TurnMode::Plan),
            Some(Hold::SessionUnread)
        );
        assert_eq!(
            verdict(&mut guard, TurnMode::Unknown),
            Some(Hold::SessionUnread)
        );
    }

    #[test]
    fn the_bell_verdict_names_a_replaced_session_and_an_absent_pane() {
        use crate::delivery::doorbell::Hold;
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, target, _) = fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        observe_recipient_status(&mut guard, "idle", 2);
        let long_ago = unix_milliseconds().saturating_sub(10 * 60_000);
        for observation in guard.delivery_observations.values_mut() {
            observation.last_input_at_unix_ms = long_ago;
            observation.last_submit_at_unix_ms = long_ago;
            observation.status_changed_at_unix_ms = long_ago;
        }
        let now = unix_milliseconds();
        assert!(
            guard
                .delivery_bell_verdict(&target.actor, WAITED, now)
                .is_ok()
        );
        assert_eq!(
            guard
                .delivery_bell_verdict(&target.actor, WAITED, long_ago)
                .err(),
            Some(Hold::Quiet)
        );
        let replaced: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"idle","state_change_seq":2,"lineage_session":"replacement-session"},
        ]})).unwrap();
        guard.observe_delivery(crate::node::TEST_NODE, &replaced, None, None);
        assert_eq!(
            guard
                .delivery_bell_verdict(&target.actor, WAITED, now)
                .err(),
            Some(Hold::Session)
        );
        let empty: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[]})).unwrap();
        guard.observe_delivery(crate::node::TEST_NODE, &empty, None, None);
        assert_eq!(
            guard
                .delivery_bell_verdict(&target.actor, WAITED, now)
                .err(),
            Some(Hold::Absent)
        );
    }

    /// `hide agent show here` answers who the caller is, so only a
    /// pane-bound credential can ask, as for every delivery command, and a
    /// pane-bound one whose hint names another pane is refused. What it
    /// answers is the caller's own record and never another pane's, with no
    /// renderer connected.
    #[test]
    fn only_the_attested_pane_asks_who_it_is() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, target, path) = fixture(root.path());
        let (worker, client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        runtime.lock().unwrap().install_delivery_client(client);
        let context = authority(&target.actor).context;
        let show = |id: &str| Command::Agents {
            command: crate::coordination::Command::Show { id: id.into() },
        };
        let checkout =
            crate::workspace_control::checkout_caller_id(&"a".repeat(32), "/checkouts/fixture");
        let ask = |caller: &str, hint: Option<&str>, id: &str| {
            let prepared = runtime.lock().unwrap().prepare_delivery(
                crate::node::TEST_NODE,
                caller,
                &context,
                hint,
                show(id),
            )?;
            prepared.run(Duration::from_secs(5))
        };
        assert_eq!(
            ask(&checkout, Some("recipient"), "here").err().as_deref(),
            Some("agent_pane_required")
        );
        assert_eq!(
            ask("sender", Some("recipient"), "here").err().as_deref(),
            Some("caller_identity_conflict")
        );
        assert_eq!(
            ask("recipient", None, "here").err().as_deref(),
            Some("participant_unavailable")
        );
        {
            let mut guard = runtime.lock().unwrap();
            let actor = guard.delivery_observations["recipient"].actor.clone();
            let mut ledger = (*guard.delivery_state().unwrap()).clone();
            ledger.agents.push(crate::coordination::AgentRecord {
                id: "agent-7".into(),
                name: Some("recipient".into()),
                machine: crate::node::TEST_NODE.into(),
                host_scope: "fixture-scope".into(),
                native_machine: "fixture-machine".into(),
                session: "recipient-session".into(),
                instance: "terminal-recipient".into(),
                pane: "recipient".into(),
                parent: None,
                origin: None,
                project: None,
                actor,
                ended: false,
                forked: false,
            });
            guard.delivery_ledger = Ok(Arc::new(ledger));
        }
        let own = ask("recipient", Some("recipient"), "here").unwrap();
        assert_eq!(
            (&own["id"], &own["pane"]),
            (&json!("agent-7"), &json!("recipient"))
        );
        // Another pane of the same checkout is not that participant.
        assert_eq!(
            ask("sender", None, "here").err().as_deref(),
            Some("participant_unavailable")
        );
        // Another pane shows the agent by name; a checkout-bound credential
        // shows none, since every agent command needs a pane-bound caller.
        assert_eq!(ask("sender", None, "agent-7").unwrap()["id"], "agent-7");
        assert_eq!(
            ask(&checkout, Some("sender"), "agent-7").err().as_deref(),
            Some("agent_pane_required")
        );
        drop(worker);
    }

    #[test]
    fn only_a_prompt_hook_of_the_panes_own_session_is_a_submission() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, target, path) = fixture(root.path());
        let (worker, client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let mut guard = runtime.lock().unwrap();
        guard.install_delivery_client(client);
        let own = crate::wire::session_digest("hook-session");
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
            {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"idle","state_change_seq":2,"lineage_session":own},
        ]})).unwrap();
        guard.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
        let context = authority(&target.actor).context;
        clocks(&mut guard, "recipient", true);
        guard
            .prepare_delivery(
                crate::node::TEST_NODE,
                "recipient",
                &context,
                None,
                Command::Inbox,
            )
            .unwrap();
        assert_eq!(written(&mut guard), (false, false, false));
        let digest = hide_agent_hooks::delivery::prompt_digest("prompt");
        for (prompt_digest, session, submitted) in [
            (None, Some("hook-session"), true),
            (Some(digest.clone()), Some("hook-session"), true),
            // Any process in the pane can run the hook command; a session
            // that is not the pane's own, or none, clears no draft.
            (None, Some("another-session"), false),
            (Some(digest.clone()), None, false),
        ] {
            guard
                .prepare_delivery(
                    crate::node::TEST_NODE,
                    "recipient",
                    &context,
                    None,
                    Command::Pull {
                        prompt_digest: prompt_digest.clone(),
                        session: session.map(str::to_owned),
                        tool: false,
                    },
                )
                .unwrap();
            assert_eq!(
                written(&mut guard),
                (false, submitted, false),
                "digest {prompt_digest:?} session {session:?}"
            );
        }
        // A digest the hook could not have made is refused, not compared.
        assert_eq!(
            guard
                .prepare_delivery(
                    crate::node::TEST_NODE,
                    "recipient",
                    &context,
                    None,
                    Command::Pull {
                        prompt_digest: Some("prompt".into()),
                        session: None,
                        tool: false,
                    },
                )
                .err()
                .as_deref(),
            Some("prompt_digest_invalid")
        );
        let pull = |session: String| Command::Pull {
            prompt_digest: None,
            session: Some(session),
            tool: false,
        };
        // An id past the session bound is refused before it is hashed under the lock.
        assert_eq!(
            guard
                .prepare_delivery(
                    crate::node::TEST_NODE,
                    "recipient",
                    &context,
                    None,
                    pull("x".repeat(crate::delivery::SESSION_LIMIT + 1))
                )
                .err()
                .as_deref(),
            Some("session_invalid")
        );
        // A hook that runs while the pane works is a queued prompt being taken
        // up; it must not clear a draft typed after the turn began.
        guard
            .delivery_observations
            .get_mut("recipient")
            .unwrap()
            .status = "working".into();
        guard
            .prepare_delivery(
                crate::node::TEST_NODE,
                "recipient",
                &context,
                None,
                pull("hook-session".into()),
            )
            .unwrap();
        assert_eq!(written(&mut guard), (false, false, false));
        drop(guard);
        drop(worker);
    }

    /// PRD letters-to-busy-agents D-02, D-05, D-09: a tool event's ask from
    /// the pane's own session is answered from that pane as the core
    /// observes it, with a `--now` letter at once and one still waiting left
    /// pending; it submits no prompt, and another session's ask is refused.
    #[test]
    fn a_tool_events_ask_hands_a_working_agent_its_due_letters_and_submits_nothing() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, sender, _, path) = fixture(root.path());
        let (worker, client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let mut guard = runtime.lock().unwrap();
        guard.install_delivery_client(client);
        let own = crate::wire::session_digest("hook-session");
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
            {"id":"recipient","pane_id":"recipient","agent":"opencode","agent_status":"working","state_change_seq":2,"lineage_session":own},
        ]})).unwrap();
        guard.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
        let recipient = guard.delivery_observations["recipient"].actor.clone();
        let send = |intent: &str, now: bool| Command::Send {
            target: "recipient".into(),
            intent: intent.into(),
            body: format!("{intent} body"),
            kind: "request".into(),
            now,
        };
        let sender_context = authority(&sender).context;
        let mut sent = Vec::new();
        for (intent, now) in [("waits", false), ("urgent", true)] {
            let prepared = guard
                .prepare_delivery(
                    crate::node::TEST_NODE,
                    "sender",
                    &sender_context,
                    None,
                    send(intent, now),
                )
                .unwrap();
            drop(guard);
            let letter = prepared.run(Duration::from_secs(5)).unwrap();
            assert_eq!(letter["immediate"], now, "{intent}");
            sent.push(letter["id"].as_str().unwrap().to_owned());
            guard = runtime.lock().unwrap();
        }
        let recipient_context = authority(&recipient).context;
        let pull = |session: &str| Command::Pull {
            prompt_digest: None,
            session: Some(session.into()),
            tool: true,
        };
        clocks(&mut guard, "recipient", true);
        let prepared = guard
            .prepare_delivery(
                crate::node::TEST_NODE,
                "recipient",
                &recipient_context,
                None,
                pull("hook-session"),
            )
            .unwrap();
        assert_eq!(
            written(&mut guard),
            (false, false, false),
            "nothing submitted"
        );
        drop(guard);
        let intake = prepared.run(Duration::from_secs(5)).unwrap();
        assert_eq!(intake["ids"], json!([sent[1]]));
        let context = intake["context"].as_str().unwrap();
        assert!(context.contains("urgent body") && !context.contains("waits body"));
        let mut guard = runtime.lock().unwrap();
        assert_eq!(
            guard
                .prepare_delivery(
                    crate::node::TEST_NODE,
                    "recipient",
                    &recipient_context,
                    None,
                    pull("another-session"),
                )
                .err()
                .as_deref(),
            Some("session_mismatch")
        );
        assert_eq!(
            guard
                .prepare_delivery(
                    crate::node::TEST_NODE,
                    "recipient",
                    &recipient_context,
                    None,
                    Command::Pull {
                        prompt_digest: None,
                        session: None,
                        tool: true,
                    },
                )
                .err()
                .as_deref(),
            Some("session_mismatch")
        );
        drop(guard);
        drop(worker);
    }

    #[test]
    fn a_pane_reaches_a_factory_only_while_it_exists_and_only_the_factory_sends_as_it() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let (worker, client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let mut guard = runtime.lock().unwrap();
        guard.install_delivery_client(client);
        let context = guard
            .workspace_control_query(crate::node::TEST_NODE, "sender", Query::Info)
            .unwrap()
            .context;
        let send = |intent: &str| Command::Send {
            target: "factory:f-1".into(),
            intent: intent.into(),
            body: "done".into(),
            kind: "report".into(),
            now: false,
        };
        assert_eq!(
            guard
                .prepare_delivery(
                    crate::node::TEST_NODE,
                    "sender",
                    &context,
                    None,
                    send("early")
                )
                .err()
                .as_deref(),
            Some("target_unavailable"),
            "no Factory f-1 exists yet"
        );
        assert_eq!(
            guard.factory_delivery("f-1").err().as_deref(),
            Some("factory_unavailable")
        );
        guard.set_factory_recipients([("f-1".to_owned(), 30 * 60_000)].into());
        let prepared = guard
            .prepare_delivery(
                crate::node::TEST_NODE,
                "sender",
                &context,
                None,
                send("once"),
            )
            .unwrap();
        drop(guard);
        let result = prepared.run(Duration::from_secs(5)).unwrap();
        assert_eq!(result["recipient"]["pane_id"], "factory:f-1");
        assert_eq!(result["recipient"]["kind"], "factory");

        // The Factory answers as itself; a pane cannot borrow its authority.
        let guard = runtime.lock().unwrap();
        assert!(!guard.code_owned_authority_current(
            "sender",
            &Actor::factory("f-1", crate::node::TEST_NODE)
        ));
        assert!(guard.code_owned_authority_current(
            "factory:f-1",
            &Actor::factory("f-1", crate::node::TEST_NODE)
        ));
        // Hide's own Memory identity answers as itself, and only that role.
        let hide = Actor::hide(crate::delivery::HIDE_MEMORY_ROLE, crate::node::TEST_NODE);
        assert!(guard.code_owned_authority_current("hide:memory", &hide));
        assert!(!guard.code_owned_authority_current("sender", &hide));
        assert!(!guard.code_owned_authority_current(
            "hide:other",
            &Actor::hide("other", crate::node::TEST_NODE)
        ));
        assert!(crate::delivery::reserved_name("hide:memory"));
        // The operator's identity answers only as itself, and no pane can
        // take its name (PRD view-opener-feedback D-17, B35).
        let operator = Actor::operator(crate::node::TEST_NODE);
        assert!(guard.code_owned_authority_current(&operator.pane_id, &operator));
        assert!(!guard.code_owned_authority_current("sender", &operator));
        assert!(crate::delivery::reserved_name("operator"));
        assert!(crate::delivery::reserved_name(" Operator "));
        assert!(crate::delivery::reserved_name("OPERATOR:x"));
        assert!(!crate::delivery::reserved_name("operators"));
        assert!(crate::delivery::reserved_name(&operator.pane_id));
        let forged = Actor {
            session: Some("forged".into()),
            ..Actor::factory("f-1", crate::node::TEST_NODE)
        };
        assert!(!guard.factory_recipient_current(&forged));
        let prepared = guard
            .factory_prepare(
                "f-1",
                Some("sender"),
                Command::Send {
                    target: "sender".into(),
                    intent: "factory-answer".into(),
                    body: "noted".into(),
                    kind: "report".into(),
                    now: false,
                },
            )
            .unwrap();
        drop(guard);
        let result = prepared.run(Duration::from_secs(5)).unwrap();
        assert_eq!(result["sender"]["pane_id"], "factory:f-1");
        let stored = crate::delivery::ledger::load(&path).unwrap();
        assert_eq!(stored.letters.len(), 2);
        drop(worker);
    }

    #[test]
    fn remote_delivery_uses_attested_pane_and_native_kind_without_a_remote_store() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let (worker, client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let caller = "remote:device:pane:sender";
        let context;
        {
            let mut guard = runtime.lock().unwrap();
            guard.install_delivery_client(client);
            let mut remote = guard.snapshot.navigator.workspaces[0].clone();
            remote.device_id = "device".into();
            remote.id = "remote-workspace".into();
            remote.remote_target_id = Some("device".into());
            for tab in &mut remote.checkouts[0].tabs {
                for pane in &mut tab.panes {
                    pane.id = format!("remote:device:pane:{}", pane.id);
                }
            }
            guard.snapshot.navigator.workspaces.push(remote);
            guard
                .snapshot
                .status
                .remote
                .push(crate::model::RemoteStatusSnapshot {
                    target_id: "device".into(),
                    state: "connected".into(),
                    message: None,
                    herdr_version: None,
                    session: None,
                    files: crate::model::RemoteFileListSnapshot::idle(),
                    catalog: Default::default(),
                    unreachable_since_unix_ms: None,
                });
            let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[{"id":"remote-sender","pane_id":"sender","agent":"claude","agent_status":"working","state_change_seq":1,"lineage_session":"remote-session"}]})).unwrap();
            guard.observe_delivery("device", &payload, Some("fixture-device"), None);
            context = guard
                .workspace_control_query("device", caller, Query::Info)
                .unwrap()
                .context;
            assert!(
                guard
                    .prepare_delivery(
                        "device",
                        caller,
                        &context,
                        Some("recipient"),
                        Command::Inbox
                    )
                    .is_err()
            );
            assert!(
                guard
                    .prepare_delivery(
                        "device",
                        caller,
                        &context,
                        Some("remote:other:pane:sender"),
                        Command::Inbox
                    )
                    .is_err()
            );
        }
        let prepared = runtime
            .lock()
            .unwrap()
            .prepare_delivery(
                "device",
                caller,
                &context,
                Some("sender"),
                Command::Send {
                    target: "recipient".into(),
                    intent: "remote-once".into(),
                    body: "done".into(),
                    kind: "report".into(),
                    now: false,
                },
            )
            .unwrap();
        let result = prepared.run(Duration::from_secs(5)).unwrap();
        assert_eq!(result["sender"]["kind"], "claude");
        assert_eq!(result["sender"]["device_id"], "device");
        assert_eq!(result["sender"]["pane_id"], caller);
        let stored = crate::delivery::ledger::load(&path).unwrap();
        assert_eq!(stored.letters.len(), 1);
        assert_eq!(
            stored.letters[0].sender.session.as_deref(),
            Some("remote-session")
        );
        drop(worker);
    }

    /// A core with its own `w1:p2` and `w1:p9`, and a connected device whose
    /// agents sit in `w1:p3` (the caller) and in a `w1:p2` of the device's own.
    fn two_machines_with_one_pane_id(guard: &mut Runtime) -> (String, String) {
        let local: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
            {"id":"core-agent","pane_id":"w1:p2","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"core-session"},
            {"id":"core-only","pane_id":"w1:p9","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"core-only-session"},
        ]})).unwrap();
        guard.observe_delivery(crate::node::TEST_NODE, &local, None, None);
        let mut remote = guard.snapshot.navigator.workspaces[0].clone();
        remote.device_id = "device".into();
        remote.id = "remote-workspace".into();
        remote.remote_target_id = Some("device".into());
        for tab in &mut remote.checkouts[0].tabs {
            for pane in &mut tab.panes {
                pane.id = format!("remote:device:pane:{}", pane.id);
            }
        }
        guard.snapshot.navigator.workspaces.push(remote);
        guard
            .snapshot
            .status
            .remote
            .push(crate::model::RemoteStatusSnapshot {
                target_id: "device".into(),
                state: "connected".into(),
                message: None,
                herdr_version: None,
                session: None,
                files: crate::model::RemoteFileListSnapshot::idle(),
                catalog: Default::default(),
                unreachable_since_unix_ms: None,
            });
        let device: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"device-sender","pane_id":"sender","agent":"claude","agent_status":"working","state_change_seq":1,"lineage_session":"device-sender-session"},
            {"id":"device-agent","pane_id":"w1:p2","agent":"claude","agent_status":"working","state_change_seq":1,"lineage_session":"device-session"},
        ]})).unwrap();
        guard.observe_delivery("device", &device, Some("fixture-device"), None);
        ("sender".to_owned(), "remote:device:pane:sender".to_owned())
    }

    fn report_to(target: &str, intent: &str) -> Command {
        Command::Send {
            target: target.into(),
            intent: intent.into(),
            body: "done".into(),
            kind: "report".into(),
            now: false,
        }
    }

    #[test]
    fn a_bare_pane_id_names_a_pane_on_the_callers_own_machine() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let (worker, client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let (local_caller, device_caller) = {
            let mut guard = runtime.lock().unwrap();
            guard.install_delivery_client(client);
            two_machines_with_one_pane_id(&mut guard)
        };
        let prepare = |device: &str, caller: &str, command: Command| {
            let mut guard = runtime.lock().unwrap();
            let context = guard
                .workspace_control_query(device, caller, Query::Info)
                .unwrap()
                .context;
            guard.prepare_delivery(device, caller, &context, None, command)
        };

        // The device's agent writes `w1:p2`, the number its own Herdr shows,
        // and the core has a different agent under the same number.
        let result = prepare("device", &device_caller, report_to("w1:p2", "from-device"))
            .unwrap()
            .run(Duration::from_secs(5))
            .unwrap();
        assert_eq!(result["recipient"]["pane_id"], "remote:device:pane:w1:p2");
        assert_eq!(result["recipient"]["device_id"], "device");

        // The core's own agent writes the same number and gets its own pane.
        let result = prepare(
            crate::node::TEST_NODE,
            &local_caller,
            report_to("w1:p2", "from-core"),
        )
        .unwrap()
        .run(Duration::from_secs(5))
        .unwrap();
        assert_eq!(result["recipient"]["pane_id"], "w1:p2");
        assert_eq!(result["recipient"]["device_id"], crate::node::TEST_NODE);

        // A pane of another machine is reached by its full address.
        let result = prepare(
            crate::node::TEST_NODE,
            &local_caller,
            report_to("remote:device:pane:w1:p2", "full-address"),
        )
        .unwrap()
        .run(Duration::from_secs(5))
        .unwrap();
        assert_eq!(result["recipient"]["device_id"], "device");

        // A number that exists only on the core is no pane of the device, and
        // the refusal is the one a target nobody holds gets.
        for (command, what) in [
            (report_to("w1:p9", "core-only"), "send"),
            (
                Command::WatchStart {
                    target: "w1:p9".into(),
                    observer: None,
                    actor: None,
                },
                "watch",
            ),
        ] {
            let prepared = prepare("device", &device_caller, command);
            let refused = match prepared {
                Ok(prepared) => prepared.run(Duration::from_secs(5)).err(),
                Err(reason) => Some(reason),
            };
            assert_eq!(
                refused.as_deref(),
                Some("target_unavailable"),
                "{what} to a pane id only the core has"
            );
        }

        // The caller's own bare pane id declares it, as its hint does.
        assert!(
            prepare(
                "device",
                &device_caller,
                Command::WatchStart {
                    target: "w1:p2".into(),
                    observer: Some("sender".into()),
                    actor: Some("sender".into()),
                },
            )
            .is_ok()
        );
        // The core's pane under that number is not the device caller's.
        assert_eq!(
            prepare(
                "device",
                &device_caller,
                Command::WatchStart {
                    target: "w1:p2".into(),
                    observer: Some("w1:p2".into()),
                    actor: None,
                },
            )
            .err()
            .as_deref(),
            Some("caller_identity_conflict")
        );
        let stored = crate::delivery::ledger::load(&path).unwrap();
        assert_eq!(stored.letters.len(), 3);
        drop(worker);
    }

    #[test]
    fn a_checkout_bound_caller_is_refused_delivery_and_agent_commands_with_or_without_a_hint() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, _, path) = fixture(root.path());
        let before = std::fs::read(&path).unwrap();
        let (worker, client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let caller = crate::workspace_control::checkout_caller_id("cap", "/checkouts/fixture");
        let context = authority(&actor).context;
        let commands = [
            Command::Send {
                target: "recipient".into(),
                intent: "checkout-send".into(),
                body: "hello".into(),
                kind: "request".into(),
                now: false,
            },
            Command::Inbox,
            Command::WatchStart {
                target: "recipient".into(),
                observer: None,
                actor: None,
            },
            Command::Agents {
                command: crate::coordination::Command::List,
            },
        ];
        let mut guard = runtime.lock().unwrap();
        guard.install_delivery_client(client);
        for hint in [None, Some("sender"), Some("recipient")] {
            for command in commands.clone() {
                assert_eq!(
                    guard
                        .prepare_delivery(
                            crate::node::TEST_NODE,
                            &caller,
                            &context,
                            hint,
                            command.clone()
                        )
                        .err()
                        .as_deref(),
                    Some("agent_pane_required"),
                    "hint {hint:?} command {command:?}"
                );
            }
        }
        // The pane-bound caller is unchanged: its own pane, and a hint that
        // names another pane is a conflict.
        assert_eq!(
            guard
                .prepare_delivery(
                    crate::node::TEST_NODE,
                    "sender",
                    &context,
                    Some("recipient"),
                    Command::Inbox
                )
                .err()
                .as_deref(),
            Some("caller_identity_conflict")
        );
        let prepared = guard
            .prepare_delivery(
                crate::node::TEST_NODE,
                "sender",
                &context,
                Some("sender"),
                commands[0].clone(),
            )
            .unwrap();
        drop(guard);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let result = prepared.run(Duration::from_secs(5)).unwrap();
        assert_eq!(result["sender"]["pane_id"], "sender");
        drop(worker);
    }

    #[test]
    fn prepared_command_refuses_a_changed_pane_context_without_saving() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, _, path) = fixture(root.path());
        let before = std::fs::read(&path).unwrap();
        let (worker, client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let context = authority(&actor).context;
        let prepared = {
            let mut guard = runtime.lock().unwrap();
            guard.install_delivery_client(client);
            guard
                .prepare_delivery(
                    crate::node::TEST_NODE,
                    &actor.pane_id,
                    &context,
                    None,
                    Command::Send {
                        target: "recipient".into(),
                        intent: "must-refuse".into(),
                        body: "private".into(),
                        kind: "request".into(),
                        now: false,
                    },
                )
                .unwrap()
        };
        runtime.lock().unwrap().snapshot.navigator.workspaces[0].checkouts[0].path =
            "/checkouts/moved".into();
        assert_eq!(
            prepared.run(Duration::from_secs(5)).unwrap_err(),
            "caller_context_changed"
        );
        assert!(
            runtime
                .lock()
                .unwrap()
                .delivery_state()
                .unwrap()
                .letters
                .is_empty()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        drop(worker);
    }

    /// Two agents in panes `lead` and `child` on this machine's Herdr at
    /// `scope`, each registered, with `lead` watching `child`.
    fn registered_pair(runtime: &mut Runtime) -> Ledger {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"lead","pane_id":"lead","agent":"claude","agent_status":"idle","state_change_seq":1,
             "lineage_session":crate::wire::session_digest("lead-native")},
            {"id":"child","pane_id":"child","agent":"codex","agent_status":"working","state_change_seq":1,
             "lineage_session":crate::wire::session_digest("child-native")},
        ]}))
        .unwrap();
        runtime.observe_delivery(crate::node::TEST_NODE, &payload, Some("scope"), None);
        let mut ledger = (*runtime.delivery_state().unwrap()).clone();
        for (id, pane) in [("agent-1", "lead"), ("agent-2", "child")] {
            ledger.agents.push(crate::coordination::AgentRecord {
                id: id.into(),
                // A name that is not the pane id: a pane id is no name.
                name: Some(format!("{pane}-agent")),
                machine: crate::node::TEST_NODE.into(),
                host_scope: "scope".into(),
                native_machine: "fixture-machine".into(),
                session: format!("{pane}-native"),
                instance: format!("terminal-{pane}"),
                pane: pane.into(),
                parent: (pane == "child").then(|| "agent-1".into()),
                origin: None,
                project: None,
                actor: runtime.delivery_observations[pane].actor.clone(),
                ended: false,
                forked: false,
            });
        }
        ledger.next_id = 3;
        let lead = ledger.agents[0].actor.clone();
        let child = ledger.agents[1].actor.clone();
        crate::delivery::watch::start(&mut ledger, &lead, &child, 1).unwrap();
        ledger.validate().unwrap();
        runtime.delivery_ledger = Ok(Arc::new(ledger.clone()));
        ledger
    }

    fn only_lead() -> SessionSnapshotPayload {
        serde_json::from_value(json!({"agents":[
            {"id":"lead","pane_id":"lead","agent":"claude","agent_status":"idle","state_change_seq":1,
             "lineage_session":crate::wire::session_digest("lead-native")},
        ]}))
        .unwrap()
    }

    /// Herdr closing a registered agent's pane ends its registration and the
    /// watches on it in the durable ledger, and frees its name; the agent
    /// whose pane is still open stays registered.
    #[test]
    fn herdr_closing_a_registered_pane_ends_the_registration_and_its_watches() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let (worker, client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        registered_pair(&mut runtime.lock().unwrap());
        runtime.lock().unwrap().observe_delivery(
            crate::node::TEST_NODE,
            &only_lead(),
            Some("scope"),
            None,
        );
        // Any transaction commits the pending ends with it.
        client
            .submit(
                crate::delivery::worker::Effect::HumanClaim,
                Duration::from_secs(5),
            )
            .unwrap();
        let persisted = crate::delivery::ledger::load(&path).unwrap();
        let ended = |ledger: &Ledger| {
            ledger
                .agents
                .iter()
                .map(|record| (record.id.clone(), record.ended))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ended(&persisted),
            [("agent-1".to_owned(), false), ("agent-2".to_owned(), true)]
        );
        assert!(persisted.watches.is_empty());
        let guard = runtime.lock().unwrap();
        assert_eq!(*guard.delivery_state().unwrap(), persisted);
        assert!(guard.delivery_registrations_gone().is_empty());
        assert!(crate::coordination::resolve_actor(&persisted, "child").is_none());
        drop(guard);
        drop(worker);
    }

    /// The same close ends the answer wait of a request the closed pane's agent
    /// held, in the durable ledger, and its sender reads why through `show`.
    #[test]
    fn herdr_closing_a_pane_ends_the_answer_wait_of_the_requests_it_held() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let (worker, client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let (lead, child, request) = {
            let mut guard = runtime.lock().unwrap();
            let mut ledger = registered_pair(&mut guard);
            let lead = ledger.agents[0].actor.clone();
            let child = ledger.agents[1].actor.clone();
            let at = crate::delivery::worker::now();
            let sent = crate::delivery::mailbox::send(
                &mut ledger,
                &lead,
                &child,
                "ask",
                "body",
                "request",
                None,
                at,
            )
            .unwrap();
            crate::delivery::mailbox::apply(
                &mut ledger,
                &child,
                None,
                &crate::delivery::Command::Confirm {
                    ids: vec![sent.id.clone()],
                },
                at,
            )
            .unwrap();
            guard.delivery_ledger = Ok(Arc::new(ledger));
            guard.observe_delivery(crate::node::TEST_NODE, &only_lead(), Some("scope"), None);
            (lead, child, sent.id)
        };
        client
            .submit(
                crate::delivery::worker::Effect::HumanClaim,
                Duration::from_secs(5),
            )
            .unwrap();
        let mut persisted = crate::delivery::ledger::load(&path).unwrap();
        let shown = crate::delivery::mailbox::apply(
            &mut persisted,
            &lead,
            None,
            &crate::delivery::Command::Show { id: request },
            2,
        )
        .unwrap();
        assert_eq!(shown["waiting_answer"], false);
        assert_eq!(shown["answer_wait_ended"], "party_ended");
        assert_eq!(shown["state"], "delivered");
        assert!(!persisted.letters[0].open());
        // The ended agent can no longer be the one who answers it.
        assert!(crate::coordination::resolve_actor(&persisted, &child.name).is_none());
        drop(worker);
    }

    /// A pane read ends only this Herdr's registrations it can vouch for:
    /// none made after its snapshot was asked for, none on another Herdr or
    /// device, and never a Factory's code-owned one, all of whose panes the
    /// snapshot lacks; a registration made before it whose pane it lacks
    /// does end.
    #[test]
    fn a_pane_read_never_ends_a_registration_it_cannot_vouch_for() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        let mut ledger = registered_pair(&mut guard);
        let child = ledger.agents[1].clone();
        let variant = |id: u64, pane: &str| {
            let mut record = child.clone();
            record.id = format!("agent-{id}");
            record.name = Some(format!("{pane}-agent"));
            record.pane = pane.into();
            record.actor.pane_id = pane.into();
            record.parent = None;
            record
        };
        let next = ledger.next_id;
        let mut elsewhere = variant(next, "elsewhere");
        elsewhere.host_scope = "another-herdr".into();
        let mut device = variant(next + 1, "device-pane");
        device.machine = "device".into();
        device.actor.device_id = "device".into();
        device.actor.pane_id = "remote:device:pane:device-pane".into();
        let factory = Actor::factory("f1", crate::node::TEST_NODE);
        let mut owned = variant(next + 2, &factory.pane_id);
        owned.actor = factory;
        ledger.agents.extend([elsewhere, device, owned]);
        ledger.next_id = next + 3;
        guard.delivery_ledger = Ok(Arc::new(ledger.clone()));
        // A reconnect asks for a fresh snapshot, and an agent registers
        // while it is out.
        guard.begin_delivery_pane_read(crate::node::TEST_NODE);
        let late = variant(ledger.next_id, "late");
        ledger.agents.push(late);
        ledger.next_id += 1;
        guard.delivery_ledger = Ok(Arc::new(ledger));
        // The snapshot lists only the lead.
        guard.observe_delivery(crate::node::TEST_NODE, &only_lead(), Some("scope"), None);
        assert_eq!(
            guard
                .delivery_registrations_gone()
                .into_iter()
                .map(|(id, reason)| (id, reason.reason()))
                .collect::<Vec<_>>(),
            [("agent-2".to_owned(), "pane_absent")]
        );
    }

    /// The recipient idle for good, the way an update finds it: a native
    /// session, nothing waiting, no change of status for long.
    fn idle_recipient() -> SessionSnapshotPayload {
        serde_json::from_value(json!({"agents":[
            {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"idle","state_change_seq":2,
             "lineage_session":"recipient-session"},
        ]}))
        .unwrap()
    }

    /// A runtime whose first look at the recipient is `payload`, with the
    /// composer store at `path` installed before it, as a daemon starts.
    fn started_with_store(
        root: &std::path::Path,
        path: &std::path::Path,
        payload: &SessionSnapshotPayload,
    ) -> Arc<Mutex<Runtime>> {
        let (runtime, _, _, _) = fixture(root);
        {
            let mut guard = runtime.lock().unwrap();
            guard.delivery_observations.clear();
            let store = crate::composer_store::ComposerStore::new(path.to_owned());
            let marks = store.restore(unix_milliseconds());
            guard.install_composer_store(store, marks);
            guard.observe_delivery(crate::node::TEST_NODE, payload, None, None);
        }
        runtime
    }

    /// The doorbell's verdict for the recipient, `ago` milliseconds after the
    /// pane last changed, with the session read saying nothing waits.
    fn verdict_for_recipient(
        runtime: &Arc<Mutex<Runtime>>,
    ) -> Result<(), crate::delivery::doorbell::Hold> {
        let mut guard = runtime.lock().unwrap();
        let observation = guard.delivery_observations.get_mut("recipient").unwrap();
        read_nothing_waits(observation);
        observation.status_changed_at_unix_ms = 0;
        let observation = observation.clone();
        crate::delivery::doorbell::judge(&observation, WAITED, unix_milliseconds() + 600_000)
    }

    #[test]
    fn a_draft_the_last_run_held_is_held_by_the_next_and_a_submission_releases_it() {
        use crate::delivery::doorbell::Hold;
        let root = tempfile::tempdir().unwrap();
        let marks = root.path().join("composer-marks.json");
        // The first run: the operator types into an idle pane.
        {
            let runtime = started_with_store(&root.path().join("one"), &marks, &idle_recipient());
            {
                let mut guard = runtime.lock().unwrap();
                let observation = guard.delivery_observations.get_mut("recipient").unwrap();
                observation.last_input_at_unix_ms = 0;
                observation.last_submit_at_unix_ms = 0;
                observation.entered_working_at_unix_ms = 0;
                guard.note_delivery_key_at("recipient", unix_milliseconds());
            }
            assert_eq!(verdict_for_recipient(&runtime), Err(Hold::Draft));
        }
        // The app is replaced: a new runtime sees the pane for the first time.
        let runtime = started_with_store(&root.path().join("two"), &marks, &idle_recipient());
        assert_eq!(
            verdict_for_recipient(&runtime),
            Err(Hold::Draft),
            "the restarted daemon still knows the composer holds a draft"
        );
        // The prompt hook of the pane's own session ends it, and the file
        // stops naming the pane.
        runtime.lock().unwrap().note_prompt_submitted("recipient");
        assert_eq!(verdict_for_recipient(&runtime), Ok(()));
        drop(runtime);
        let restarted = started_with_store(&root.path().join("three"), &marks, &idle_recipient());
        assert_eq!(
            verdict_for_recipient(&restarted),
            Ok(()),
            "a pane that was submitted is not held again"
        );
    }

    #[test]
    fn a_pane_the_last_run_knew_no_draft_in_is_not_held_after_a_restart() {
        let root = tempfile::tempdir().unwrap();
        let marks = root.path().join("composer-marks.json");
        let runtime = started_with_store(&root.path().join("one"), &marks, &idle_recipient());
        assert_eq!(verdict_for_recipient(&runtime), Ok(()));
        assert!(!marks.exists(), "nothing to remember, nothing written");
    }

    #[test]
    fn a_pane_that_is_working_when_the_daemon_returns_drops_the_kept_draft() {
        let root = tempfile::tempdir().unwrap();
        let marks = root.path().join("composer-marks.json");
        {
            let runtime = started_with_store(&root.path().join("one"), &marks, &idle_recipient());
            let mut guard = runtime.lock().unwrap();
            let observation = guard.delivery_observations.get_mut("recipient").unwrap();
            observation.last_input_at_unix_ms = 0;
            observation.last_submit_at_unix_ms = 0;
            observation.entered_working_at_unix_ms = 0;
            guard.note_delivery_key_at("recipient", unix_milliseconds());
        }
        let working: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"working","state_change_seq":3,
             "lineage_session":"recipient-session"},
        ]}))
        .unwrap();
        let runtime = started_with_store(&root.path().join("two"), &marks, &working);
        // The turn that started while the daemon was away took the draft.
        runtime.lock().unwrap().observe_delivery(
            crate::node::TEST_NODE,
            &idle_recipient(),
            None,
            None,
        );
        assert_eq!(verdict_for_recipient(&runtime), Ok(()));
    }

    /// Letters stand on the state folder's durable files on every system:
    /// the core opens its ledger from a real folder, two agents register,
    /// one sends and the other finds the letter in its inbox, and the ledger
    /// read back from disk holds both agents and the letter. The Windows
    /// check runs it, where the ledger once refused to open (`Unsupported`).
    #[test]
    fn agents_register_and_a_letter_reaches_the_inbox_through_the_saved_ledger() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, sender, _, path) = fixture(root.path());
        assert!(runtime.lock().unwrap().delivery_ledger.is_ok());
        let native = |pane: &str, session: &str| {
            json!({"pane_id":pane,"workspace_id":"workspace","tab_id":"tab",
                "terminal_id":format!("terminal-{pane}"),"revision":1,"focused":false,
                "agent_status":"working","agent":"codex",
                "agent_session":{"source":"herdr:codex","agent":"codex","kind":"id","value":session}})
        };
        let herdr =
            crate::fake_herdr::FakeHerdr::start("delivery-register-send-read", move |method, _| {
                assert_eq!(method, "agent.list");
                json!({"type":"agent_list","agents":[
                native("sender", "writer-session"), native("recipient", "reader-session")]})
            });
        let (worker, client) = Worker::spawn(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        {
            let mut guard = runtime.lock().unwrap();
            guard.install_delivery_client(client);
            set_live(&mut guard, Some(&herdr));
            let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,
                    "lineage_session":crate::wire::session_digest("writer-session")},
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"working","state_change_seq":1,
                    "lineage_session":crate::wire::session_digest("reader-session")},
            ]})).unwrap();
            guard.observe_delivery(crate::node::TEST_NODE, &payload, None, None);
        }
        let context = authority(&sender).context;
        let run = |pane: &str, command: Command| {
            let prepared = runtime.lock().unwrap().prepare_delivery(
                crate::node::TEST_NODE,
                pane,
                &context,
                None,
                command,
            )?;
            prepared.run(Duration::from_secs(5))
        };
        let scope = herdr.socket_path().to_str().unwrap().to_owned();
        for (pane, name, session) in [
            ("sender", "writer", "writer-session"),
            ("recipient", "reader", "reader-session"),
        ] {
            let registered = run(
                pane,
                Command::Agents {
                    command: crate::coordination::Command::Register {
                        check: false,
                        machine: None,
                        host_scope: scope.clone(),
                        session: session.into(),
                        instance: format!("terminal-{pane}"),
                        name: name.into(),
                        pane: pane.into(),
                        parent: None,
                        project: None,
                    },
                },
            )
            .unwrap();
            assert_eq!(registered["pane"], pane);
        }

        let letter = run(
            "sender",
            Command::Send {
                target: "reader".into(),
                intent: "durable-letter".into(),
                body: "read me after a restart".into(),
                kind: "request".into(),
                now: false,
            },
        )
        .unwrap();
        let inbox = run("recipient", Command::Inbox).unwrap();
        assert_eq!(inbox.as_array().map(Vec::len), Some(1), "{inbox}");
        assert_eq!(inbox[0]["id"], letter["id"]);
        assert_eq!(inbox[0]["body"], "read me after a restart");
        drop(worker);

        let saved = crate::delivery::ledger::recover(&path).unwrap();
        let mut names: Vec<_> = saved
            .agents
            .iter()
            .filter_map(|agent| agent.name.clone())
            .collect();
        names.sort();
        assert_eq!(names, ["reader", "writer"]);
        assert_eq!(saved.letters.len(), 1);
        assert_eq!(saved.letters[0].body, "read me after a restart");
    }
}
