//! Memory-only admission and observations. Disk, process and helper work is
//! owned by delivery workers, never by the owner-thread caller.

use std::collections::HashSet;
use std::sync::Arc;

use crate::delivery::correspondence::Correspondence;
use crate::delivery::doorbell::Turn;
use crate::delivery::ledger::{BELLS, Ledger};
use crate::delivery::mailbox::Exchange;
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
    /// This incarnation must already have been observed before a passive
    /// post-tool event can name it. A freshness TTL alone proves no identity.
    pub observed_since_unix_ms: u64,
    pub terminal_id: Option<String>,
    /// The engine's own `agent_status`, read only as a gate on input: whether
    /// the engine knows the agent's state, holds it at its blocked prompt (where
    /// `agent.prompt` refuses), and the clocks of a draft and of the quiet
    /// period. Whether the agent works or waits is `projected`, never this.
    pub status: String,
    /// Keep identity and input clocks while a read is delayed, but admit no bell.
    pub telemetry_stale: bool,
    pub state_change_seq: Option<u64>,
    /// The engine connection `status` and `state_change_seq` were read on
    /// (`agent_state::ReadConnection::mark`), which a sequence is compared
    /// within (`delivery::watch::StateRead`).
    pub engine: Option<String>,
    pub status_changed_at_unix_ms: u64,
    /// The last input hide routed to this pane: a key, a paste, a phone
    /// write or an agent-find key.
    pub last_input_at_unix_ms: u64,
    /// When the core last read this agent's state (the observe pass that
    /// laid it); it holds until a later read changes it.
    pub observed_at_unix_ms: u64,
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
    /// How many inputs were noted here, each counted once whatever its
    /// clock says: a writer that read it before an off-lock write knows
    /// after the write whether anyone else's input came meanwhile
    /// (`delivery_pane_unmoved`, `note_pane_send_write`).
    pub input_generation: u64,
    /// The `input_generation` a `hide pane send`'s own write left. While it
    /// is still the newest, the newest input is that sender's and no
    /// person's draft (`Observation::person_draft`); any input noted after
    /// it, or during that write, is not.
    pub pane_send_generation: Option<u64>,
    /// The last input that answered the agent's screen: a key the operator
    /// typed, a phone reply, or a `hide pane send` whose every write landed
    /// (`note_pane_send_answered`). A brief or letter typed before it may be
    /// typed again (`typed::retype_due`); a send that stopped partway or
    /// lost its reply landed input and answered nothing.
    pub answered_at_unix_ms: u64,
    /// The last prompt hook of the pane's own session that ran at rest; 0
    /// until one runs. A turn it started that has ended with no key since
    /// leaves the composer known empty, the moment an agent version's
    /// composer declaration is checked (`composer_check_due`).
    pub hook_submit_at_unix_ms: u64,
    /// The last moment Hide itself typed into the pane: a bell, a mission
    /// answer, a brief typed again or its Enter, a 마무리 맡기기's words
    /// (`Runtime::note_hide_typing`). It is no person's draft, so it holds no
    /// bell, but the composer shows it until the agent takes it, so a hook
    /// moment before it is no known-empty moment (`composer_check_due`).
    pub hide_typed_at_unix_ms: u64,
    pub session: Option<hide_session::session_activity::SessionActivityRequest>,
    /// The session a thread of this pane reads, for every agent with a
    /// thread reader, OpenCode's database included.
    pub thread: Option<super::session_thread::ThreadReference>,
    pub host_scope: Option<String>,
    pub turn: Turn,
    /// The turn the core decides for the agent (`agent_state::turn_projection`),
    /// laid where the hook's and the session read's facts are laid
    /// (`lay_delivery_turns`). It stands only for the engine state it was laid
    /// under ([`Self::projected_turn`]).
    pub projected: Option<ProjectedTurn>,
    /// The observe pass that last saw the pane (`observe_delivery`). The same
    /// step under the lock lays that pass's turn, so a turn laid by an
    /// earlier pass, which stands for facts this pass may have moved (a
    /// question just read), is never read with this observation.
    pub observed_generation: u64,
    /// Why the session the pane runs cannot be read, while its turn is
    /// [`Turn::Unread`] because the last read refused for a reason reading
    /// again cannot clear (the reader's code). Letters wait on it, so their
    /// senders are told and the row is marked (`unreadable_letters.rs`).
    pub unreadable: Option<String>,
}

/// One `observe_delivery` pass, as the token the same step under the lock
/// lays its turns with (`Runtime::ingest_observed_session` for this machine,
/// [`Runtime::observe_device_session`] for a device). Only that step lays
/// the pane's turn, from the payload and the label overlay that pass read: an
/// ingest of another payload, such as an editor's close status with the
/// overlay the runtime held before, certifies nothing (`lay_delivery_turns`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "only the pass's own step lays its turns"]
pub(super) struct ObservedPass(u64);

/// The core's turn for an agent, as laid on its observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProjectedTurn {
    /// The engine's `state_change_seq` of the agent the turn was decided from.
    pub seq: Option<u64>,
    pub turn: crate::agent_state::TurnProjection,
    /// The observe pass whose facts the turn was laid from
    /// ([`Observation::observed_generation`]).
    pub generation: u64,
    /// When the core's activity last changed on this pane; `None` while it
    /// is the one first laid, whose start the engine's own clocks say.
    pub changed_at_unix_ms: Option<u64>,
}

impl ProjectedTurn {
    /// Since when the agent works, for the wait a letter makes mid-turn.
    pub(crate) fn working_since(&self, observation: &Observation) -> u64 {
        self.changed_at_unix_ms
            .unwrap_or(observation.entered_working_at_unix_ms)
    }

    /// The last change of the pane's state, the engine's or the core's turn, which
    /// the quiet period counts from.
    pub(crate) fn changed_at(&self, observation: &Observation) -> u64 {
        observation
            .status_changed_at_unix_ms
            .max(self.changed_at_unix_ms.unwrap_or(0))
    }
}

/// Each agent's turn as the core decides it, by the agent's pane id in
/// `payload`, which must already carry the hook's and the session read's
/// facts.
pub(crate) fn delivery_turns(
    payload: &SessionSnapshotPayload,
) -> Vec<(String, Option<u64>, crate::agent_state::TurnProjection)> {
    payload
        .agents
        .iter()
        .filter_map(|agent| {
            let pane = agent.pane_id.as_deref()?;
            Some((
                pane.to_owned(),
                agent.state_change_seq,
                crate::agent_state::turn_projection(agent),
            ))
        })
        .collect()
}

impl Observation {
    /// The core's turn for the pass the pane was last observed in: none
    /// while the read is stale, and none when that pass laid no turn for
    /// the pane (a pane a move this machine runs draws as it was before it,
    /// `overlay_pane_moves`), since an earlier pass's turn may stand for
    /// facts this one moved (a question read while the engine's state stays the
    /// same).
    pub(crate) fn projected_turn(&self) -> Option<&ProjectedTurn> {
        self.projected.as_ref().filter(|projected| {
            !self.telemetry_stale
                && projected.generation == self.observed_generation
                && projected.seq == self.state_change_seq
        })
    }

    /// The name the agent answers to, apart from where it sits: an agent
    /// the engine gave no name is called by its pane id (`actor.name` restates
    /// the pane), and a pane id is per machine, so it is no name that a
    /// caller on another machine could mean.
    pub(crate) fn given_name(&self) -> Option<&str> {
        (self.actor.name != self.raw_pane_id).then_some(self.actor.name.as_str())
    }

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

    /// The engine's reading of the agent this observation holds.
    pub(crate) fn read(&self) -> crate::delivery::watch::StateRead<'_> {
        crate::delivery::watch::StateRead {
            engine: self.engine.as_deref(),
            status: &self.status,
            seq: self.state_change_seq,
        }
    }

    /// A draft a person may be writing: an unsent draft
    /// ([`Observation::unsent_draft`]) whose newest input is not what a
    /// `hide pane send` wrote. The one rule a pane send is held by.
    pub(crate) fn person_draft(&self) -> bool {
        self.unsent_draft() && self.pane_send_generation != Some(self.input_generation)
    }
}

impl Runtime {
    /// `labels` is the device's label overlay, whose session reads say what
    /// an agent waits for; without one, an agent whose read reports turns is
    /// not known to wait for nothing.
    ///
    /// A sync pass calls it only inside the step that lays the pass's turns
    /// (`ingest_observed_session`, [`Self::observe_device_session`]): a tool
    /// or turn-end ask, or a bell, that met an observation whose turn is not
    /// laid yet would read the agent as not working (with-hide/hide-private#560).
    pub(super) fn observe_delivery(
        &mut self,
        device: &str,
        payload: &SessionSnapshotPayload,
        host_scope: Option<&str>,
        labels: Option<&LabelOverlay>,
    ) -> ObservedPass {
        self.continue_moved_registrations(device, payload, host_scope);
        self.delivery_observe_generation += 1;
        let generation = self.delivery_observe_generation;
        let now = unix_milliseconds();
        let terminals: std::collections::HashMap<_, _> = payload
            .panes
            .iter()
            .map(|pane| (pane.pane_id.as_str(), pane.terminal_id.as_deref()))
            .collect();
        let mut present = HashSet::new();
        let mut excess = false;
        let mut unreadable_moved = false;
        let engine = self.read_mark(device);
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
            let pane_id = crate::address::pane_id_on(self.node.as_str(), device, pane);
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
            let terminal_id = terminals.get(pane).copied().flatten().map(str::to_owned);
            let observed_since_unix_ms = previous
                .filter(|old| {
                    old.terminal_id == terminal_id
                        && old.host_scope.as_deref() == host_scope
                        && hide_agent_adapter::canonical_kind(&old.actor.kind)
                            == hide_agent_adapter::canonical_kind(&actor.kind)
                        && old.session.as_ref().map(|reference| {
                            (
                                reference.reference_kind.as_str(),
                                reference.reference_value.as_str(),
                            )
                        }) == agent
                            .agent_session
                            .as_ref()
                            .map(|reference| (reference.kind.as_str(), reference.value.as_str()))
                })
                .map_or(now, |old| old.observed_since_unix_ms);
            // The ledger's watches are read only for an agent whose last
            // observation does not already say when its state began: a
            // resting pass looks no agent up in them.
            let read = crate::delivery::watch::StateRead {
                engine: engine.as_deref(),
                status,
                seq: agent.state_change_seq,
            };
            let changed_at = previous
                .filter(|old| !read.moved_from(&old.read()))
                .map(|old| old.status_changed_at_unix_ms)
                .or_else(|| {
                    self.delivery_ledger
                        .as_ref()
                        .ok()
                        .and_then(|ledger| {
                            ledger.watches.iter().find(|watch| {
                                watch.target.same_identity(&actor)
                                    && !read.moved_from(&watch.last_read())
                            })
                        })
                        .map(|watch| watch.status_changed_at_unix_ms)
                })
                .unwrap_or(now);
            // Turn reporting is a session-format fact, independent of letters
            // and bell eligibility. An unread turn-capable session stays held.
            let session_kind = hide_session::Agent::from_kind(kind);
            let turn = match session_kind {
                Some(kind) if kind.reports_turns() => labels
                    .and_then(|labels| labels.waiting(agent))
                    .map_or(Turn::Unread, |_| Turn::Read),
                _ => Turn::NotReported,
            };
            let unreadable = (turn == Turn::Unread)
                .then(|| labels.and_then(|labels| labels.refusal(agent)))
                .flatten()
                .map(str::to_owned);
            unreadable_moved |=
                previous.and_then(|old| old.unreadable.as_deref()) != unreadable.as_deref();
            // Every read of the agent's session is made in the account its
            // profile names (PRD agent-profiles B19).
            let account = self.pane_account(
                &pane_id,
                kind,
                agent.name.as_deref(),
                agent
                    .agent_session
                    .as_ref()
                    .map(|reference| reference.value.as_str()),
            );
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
                        account: account.clone(),
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
                observed_since_unix_ms,
                terminal_id,
                status: status.to_owned(),
                telemetry_stale: agent.telemetry_stale,
                state_change_seq: agent.state_change_seq,
                engine: engine.clone(),
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
                input_generation: previous.map_or(0, |old| old.input_generation),
                pane_send_generation: previous.and_then(|old| old.pane_send_generation),
                answered_at_unix_ms: previous.map_or(0, |old| old.answered_at_unix_ms),
                observed_at_unix_ms: if agent.telemetry_stale {
                    previous.map_or(0, |old| old.observed_at_unix_ms)
                } else {
                    now
                },
                hook_submit_at_unix_ms: previous.map_or(0, |old| old.hook_submit_at_unix_ms),
                hide_typed_at_unix_ms: previous.map_or(0, |old| old.hide_typed_at_unix_ms),
                session,
                thread: super::session_thread::ThreadReference::of(agent, account),
                host_scope: host_scope
                    .filter(|scope| scope.len() <= 4096)
                    .map(str::to_owned),
                turn,
                projected: previous.and_then(|old| old.projected),
                observed_generation: generation,
                unreadable,
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
        if unreadable_moved {
            self.unreadable_letters.touch();
        }
        self.observe_registration_panes(device, payload, host_scope);
        // A woken pane is first seen here; the session projection that
        // follows publishes whatever this settles.
        self.settle_dormant_registrations();
        self.follow_resumed_conductors(device);
        self.follow_session_thread(device, payload);
        ObservedPass(generation)
    }

    /// Observes `payload` and lays the turn the session ingest lays from it,
    /// for a test whose agents carry no hook facts.
    #[cfg(test)]
    pub(crate) fn observe_delivery_laid(
        &mut self,
        device: &str,
        payload: &SessionSnapshotPayload,
        host_scope: Option<&str>,
        labels: Option<&LabelOverlay>,
    ) {
        let pass = self.observe_delivery(device, payload, host_scope, labels);
        let mut laid = payload.clone();
        if let Some(labels) = labels {
            labels.apply(&mut laid);
        }
        self.lay_delivery_turns(device, pass, delivery_turns(&laid));
    }

    /// A device coordinator's pass, in one step under the lock: observes the
    /// device's `payload` for delivery as its engine listed it, draws the
    /// sleeping agents that the engine no longer lists, lays the pass's label
    /// `overlay` and the hook's facts on it, and lays each observed pane's
    /// turn from them. Answers each agent pane's hook tokens, by the device's
    /// own pane id. Memory work only; the session's projection is built
    /// outside the lock.
    pub(crate) fn observe_device_session(
        &mut self,
        device: &str,
        payload: &mut SessionSnapshotPayload,
        overlay: Option<&LabelOverlay>,
    ) -> std::collections::BTreeMap<String, crate::agent_hooks::PaneHookTokens> {
        let pass = self.observe_delivery(device, payload, None, overlay);
        self.settle_node_agent_sleep(device, payload);
        if let Some(overlay) = overlay {
            overlay.apply(payload);
        }
        // The device's hooks report to the core, never to its engine, so their
        // facts are laid here the way the labels are (`agent_hooks::facts`).
        let tokens = self
            .hook_facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .lay(
                payload,
                |pane| crate::address::scoped_pane_id(device, pane),
                |pane| crate::address::is_pane_of(pane, device),
            );
        self.lay_delivery_turns(device, pass, delivery_turns(payload));
        tokens
    }

    /// Lays the core's turn on each pane of `device` that `pass` observed,
    /// from agents that carry the hook's and the session read's facts
    /// (`delivery_turns`) of that same pass. Runs in the step under the lock
    /// that made the observation, where those facts are laid, so a hook
    /// event's republish moves it too.
    pub(super) fn lay_delivery_turns(
        &mut self,
        device: &str,
        pass: ObservedPass,
        turns: impl IntoIterator<Item = (String, Option<u64>, crate::agent_state::TurnProjection)>,
    ) {
        let now = unix_milliseconds();
        let local = device == self.node.as_str();
        for (pane, seq, turn) in turns {
            // A device's panes in the local payload carry no hook facts of
            // their own; the device's coordinator lays them.
            let key = if local {
                if crate::address::is_device_pane(&pane) {
                    continue;
                }
                pane
            } else {
                crate::address::scoped_pane_id(device, &pane)
            };
            let Some(observation) = self
                .delivery_observations
                .get_mut(&key)
                .filter(|observation| observation.observed_generation == pass.0)
            else {
                continue;
            };
            let changed_at_unix_ms = match observation.projected {
                Some(old) if old.turn.activity == turn.activity => old.changed_at_unix_ms,
                Some(_) => Some(now),
                None => None,
            };
            observation.projected = Some(ProjectedTurn {
                seq,
                turn,
                generation: pass.0,
                changed_at_unix_ms,
            });
        }
    }

    /// Continue a registration only when this host still reports its terminal
    /// incarnation and native session. Public pane ids may change when the engine
    /// moves a pane between its workspaces. The saved terminal also lets the
    /// first read after a daemon restart establish that continuity.
    fn continue_moved_registrations(
        &mut self,
        device: &str,
        payload: &SessionSnapshotPayload,
        host_scope: Option<&str>,
    ) {
        // A local observation must identify its socket incarnation. Remote
        // sync is already scoped by the authenticated node, and deliberately
        // carries no local socket path into this core.
        let Some(scope) = host_scope.or_else(|| (device != self.node.as_str()).then_some(device))
        else {
            return;
        };
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return;
        };
        let present: HashSet<_> = payload
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
        let agents: std::collections::HashMap<_, _> = payload
            .agents
            .iter()
            .filter_map(|agent| agent.pane_id.as_deref().map(|pane| (pane, agent)))
            .collect();
        // A duplicated terminal in a malformed/transitional read proves no move.
        let mut terminals = std::collections::HashMap::new();
        for pane in &payload.panes {
            if let Some(terminal) = pane.terminal_id.as_deref().filter(|id| !id.is_empty()) {
                terminals
                    .entry(terminal)
                    .and_modify(|pane: &mut Option<&str>| *pane = None)
                    .or_insert(Some(pane.pane_id.as_str()));
            }
        }
        let mut continuations = Vec::new();
        for record in &ledger.agents {
            if record.ended
                || record.actor.code_owned()
                || record.actor.device_id != device
                || record.host_scope != scope
            {
                continue;
            }
            let pending = self.registration_rebinds.get(&record.id);
            let previous_pane = pending.map_or(record.actor.pane_id.as_str(), |pending| {
                pending.to.pane_id.as_str()
            });
            let previous = self
                .delivery_observations
                .get(previous_pane)
                .filter(|old| old.host_scope.as_deref() == host_scope);
            let Some(terminal) = record
                .terminal_id
                .as_deref()
                .or_else(|| pending.and_then(|pending| pending.terminal_id.as_deref()))
                .or_else(|| previous.and_then(|old| old.terminal_id.as_deref()))
            else {
                continue;
            };
            let Some(Some(pane)) = terminals.get(terminal) else {
                continue;
            };
            if *pane != record.pane && present.contains(record.pane.as_str()) {
                continue;
            }
            let Some(agent) = agents.get(pane) else {
                continue;
            };
            let (Some(name), Some(kind), Some(session)) = (
                agent.id.as_deref(),
                agent.agent.as_deref(),
                agent.lineage_session.as_deref(),
            ) else {
                continue;
            };
            if record.actor.session.as_deref() != Some(session)
                || hide_agent_adapter::canonical_kind(&record.actor.kind)
                    != hide_agent_adapter::canonical_kind(kind)
            {
                continue;
            }
            let pane_id = crate::address::pane_id_on(self.node.as_str(), device, pane);
            let actor = Actor {
                pane_id,
                name: name.to_owned(),
                kind: kind.to_owned(),
                device_id: device.to_owned(),
                session: Some(session.to_owned()),
            };
            if record.actor.same_identity(&actor) && record.terminal_id.as_deref() == Some(terminal)
            {
                continue;
            }
            if !actor.valid()
                || ledger.agents.iter().any(|other| {
                    other.id != record.id && !other.ended && other.actor.same_identity(&actor)
                })
            {
                continue;
            }
            continuations.push((
                record.id.clone(),
                previous_pane.to_owned(),
                crate::coordination::Rebind {
                    from: record.actor.clone(),
                    to: actor,
                    pane: (*pane).to_owned(),
                    terminal_id: Some(terminal.to_owned()),
                },
            ));
        }
        for (id, previous_pane, rebind) in continuations {
            if previous_pane != rebind.to.pane_id
                && let Some(mut observation) = self.delivery_observations.remove(&previous_pane)
            {
                observation.actor = rebind.to.clone();
                observation.raw_pane_id.clone_from(&rebind.pane);
                self.delivery_observations
                    .insert(rebind.to.pane_id.clone(), observation);
            }
            self.registrations_gone.remove(&id);
            self.registration_rebinds.insert(id, rebind);
        }
    }

    /// A host's session sync is about to ask the engine for a fresh snapshot.
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
        // continues the registration, so its absence ends nothing; nor does a
        // pane closed for Reopen closed, whose reopen continues it
        // (`closed_registrations.rs`).
        let reopen_held = self
            .reopen_held_registrations()
            .map(|registration| registration.id.clone())
            .collect::<Vec<_>>();
        let held: HashSet<&str> = self
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .values()
            .filter_map(|record| record.registration.as_ref())
            .chain(self.snapshot.ui_state.session_archive.held_registrations())
            .map(|registration| registration.id.as_str())
            .chain(reopen_held.iter().map(String::as_str))
            .chain(self.registration_rebinds.keys().map(String::as_str))
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

    /// Puts the registrations of the agents a restore did not resume on the
    /// ones the delivery store has still to end (`left`, from the engine's
    /// session read, which names each until an agent shows in its pane). The
    /// pane is there, a shell, so no read lists it gone: the engine says so.
    /// A pane a sleep record holds keeps its registration, which a wake
    /// continues. Said again by every read that names the agent, which costs
    /// a pass over `left` and nothing once the registration has ended.
    pub(crate) fn end_unresumed(
        &mut self,
        device: &str,
        host_scope: Option<&str>,
        payload: &SessionSnapshotPayload,
        left: &[crate::session_sync::NotResumed],
    ) {
        if left.is_empty() || !self.delivery_pane_owner_registered(device) {
            return;
        }
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return;
        };
        let shown: HashSet<&str> = payload
            .agents
            .iter()
            .filter_map(|agent| agent.pane_id.as_deref())
            .collect();
        let asleep = &self.snapshot.ui_state.agent_sleep.records;
        let left: Vec<_> = left
            .iter()
            .filter(|left| !asleep.contains_key(&left.pane_id))
            .cloned()
            .collect();
        let ended = crate::coordination::not_resumed_registrations(
            ledger, device, host_scope, &left, &shown,
        );
        self.registrations_gone.extend(ended);
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

    /// The folder a sleeping or archived session holding `actor`'s
    /// registration runs in, which its wake or resume opens again.
    fn held_actor_folder(&self, actor: &Actor) -> Option<String> {
        let ui = &self.snapshot.ui_state;
        let dormant = ui.agent_sleep.dormant.values().find(|record| {
            record
                .registration
                .as_ref()
                .is_some_and(|registration| registration.actor.same_identity(actor))
        });
        if let Some(record) = dormant {
            return Some(record.cwd.clone());
        }
        ui.session_archive
            .records
            .values()
            .flat_map(|record| &record.members)
            .find(|member| {
                member
                    .registration
                    .as_ref()
                    .is_some_and(|registration| registration.actor.same_identity(actor))
            })
            .map(|member| {
                member
                    .cwd
                    .clone()
                    .unwrap_or_else(|| member.checkout_path.clone())
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
    /// the engine and its link has not ended (PRD core-host-node-move B13). A
    /// letter to an agent there counts its delivery deadline only while it
    /// is reachable. A node's link ends here at once; the engine connection
    /// it carried reads as gone only at the next sync (`ingest_host_closed`).
    pub(crate) fn delivery_reach(&self) -> crate::delivery::ledger::Reach {
        self.snapshot
            .ui_state
            .device_registrations
            .iter()
            .filter(|registration| registration.id != self.node.as_str())
            .map(|registration| {
                let id = registration.id.as_str();
                let reachable = self.device_engine(id).is_some() && !self.device_host_ended(id);
                (registration.id.clone(), reachable)
            })
            .collect()
    }

    /// The registrations and spawn records the core's books still need,
    /// which a full delivery ledger keeps when it makes room
    /// (`coordination::room`): an open mission's and a plan step's still
    /// handed. Read only when a registration or spawn finds the ledger full.
    pub(crate) fn delivery_held(&self) -> Result<crate::coordination::Held, String> {
        let ledger = self.delivery_state()?;
        let mut held = crate::coordination::Held::default();
        held.keep_published(&ledger);
        self.mission_holds(Some(&ledger), &mut held);
        self.plan_holds(Some(&ledger), &mut held);
        Ok(held)
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
    /// Enter and reads `done` to the engine, so only a prompt hook of the pane's
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
            observation.input_generation += 1;
            observation.answered_at_unix_ms = observation.answered_at_unix_ms.max(at_unix_ms);
            self.sync_composer_mark(pane_id, unix_milliseconds());
        }
    }

    /// Hide is about to type into `pane_id` (a bell, a mission answer, a
    /// brief's retyping or Enter, a 마무리 맡기기): noted under the lock before
    /// the off-lock typing, so no composer read after it takes the pane's
    /// last hook moment for a known-empty composer while the agent has not
    /// taken the input yet (`composer_check_due`). Only that input's own
    /// prompt hook, at a later moment, marks the composer empty again.
    /// Typing into a pane that has had no prompt hook yet needs no note,
    /// since its every hook moment comes after it: a spawn's brief, a task
    /// start's first prompt, and agy's `/fork` and the composer clearing
    /// before it in the fork's new pane.
    pub(crate) fn note_hide_typing(&mut self, pane_id: &str) {
        if let Some(observation) = self.delivery_observations.get_mut(pane_id) {
            observation.hide_typed_at_unix_ms = unix_milliseconds();
        }
    }

    /// The input generation of `pane` a writer reads before its off-lock
    /// write, or `None` for a pane Hide observes no agent in.
    pub(crate) fn input_fence(&self, pane_id: &str) -> Option<u64> {
        self.delivery_observations
            .get(pane_id)
            .map(|observation| observation.input_generation)
    }

    /// Whether a person's draft may stand in `pane` now
    /// ([`Observation::person_draft`]).
    pub(crate) fn person_draft(&self, pane_id: &str) -> bool {
        self.delivery_observations
            .get(pane_id)
            .is_some_and(Observation::person_draft)
    }

    /// A `hide pane send` write landed in `pane`, or may have with its reply
    /// lost, after the sender read `fence`. It is input, as any key is, and
    /// answers nothing until the whole send has landed. The answer is the
    /// fence for the sender's next write while no one else's input came
    /// since `fence`, which makes the newest input the sender's own, and
    /// `None` when some did: that input stays theirs, and the send stops.
    pub(crate) fn note_pane_send_write(&mut self, pane_id: &str, fence: u64) -> Option<u64> {
        self.reopen_resolved_session(pane_id);
        let now = unix_milliseconds();
        let Some(observation) = self.delivery_observations.get_mut(pane_id) else {
            return Some(fence);
        };
        let own = observation.input_generation == fence;
        observation.last_input_at_unix_ms = observation.last_input_at_unix_ms.max(now);
        observation.input_generation += 1;
        if own {
            observation.pane_send_generation = Some(observation.input_generation);
        }
        let generation = observation.input_generation;
        self.sync_composer_mark(pane_id, now);
        self.trace_delivery_input(pane_id, "pane_send");
        own.then_some(generation)
    }

    /// A `hide pane send` whose every write landed answered the agent's
    /// screen, which lets typed input be typed again (`typed::retype_due`).
    pub(crate) fn note_pane_send_answered(&mut self, pane_id: &str) {
        if let Some(observation) = self.delivery_observations.get_mut(pane_id) {
            observation.answered_at_unix_ms = observation
                .answered_at_unix_ms
                .max(observation.last_input_at_unix_ms);
        }
    }

    /// A phone's reply or a thread's input is text hide sent and submitted
    /// in one write, so it is a key and a submit; `source` names which.
    pub(crate) fn note_delivery_reply(&mut self, pane_id: &str, source: &'static str) {
        self.reopen_resolved_session(pane_id);
        if let Some(observation) = self.delivery_observations.get_mut(pane_id) {
            let now = unix_milliseconds();
            observation.last_input_at_unix_ms = now;
            observation.input_generation += 1;
            observation.answered_at_unix_ms = now;
            observation.last_submit_at_unix_ms = now;
            self.sync_composer_mark(pane_id, now);
            self.trace_delivery_input(pane_id, source);
        }
    }

    /// A prompt hook ran in the pane, so whatever was in its composer was
    /// submitted, whoever typed it and wherever it was typed.
    /// A hook that runs while the pane is already `working` is a queued
    /// prompt being taken up, not the composer being sent: the operator's
    /// newer draft is still there, and the turn's own entry to `working`
    /// already covers whatever was typed before it.
    ///
    /// The engine can report the turn `working` before the hook that started it
    /// reaches the core. A hook within `HOOK_AFTER_WORK_MS` of that entry,
    /// with no key since it, is that turn's own prompt, so it marks the
    /// moment an agent version's composer is checked at (`composer_check_due`)
    /// as a hook at rest does; a queued prompt was typed after the entry.
    fn note_prompt_submitted(&mut self, pane_id: &str) {
        const HOOK_AFTER_WORK_MS: u64 = 10_000;
        self.reopen_resolved_session(pane_id);
        let now = unix_milliseconds();
        let Some(observation) = self.delivery_observations.get_mut(pane_id) else {
            return;
        };
        if observation.status != "working" {
            observation.last_submit_at_unix_ms = now;
            observation.hook_submit_at_unix_ms = now;
            self.sync_composer_mark(pane_id, now);
        } else if observation.last_input_at_unix_ms <= observation.entered_working_at_unix_ms
            && now.saturating_sub(observation.entered_working_at_unix_ms) <= HOOK_AFTER_WORK_MS
        {
            observation.hook_submit_at_unix_ms = observation.entered_working_at_unix_ms;
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

    /// The agent version Hide knows for `actor`, as the hook diagnosis of
    /// the machine its pane is on read it from the installed runtime's
    /// `--version`: this machine's own, or a registered device's that its
    /// coordinator read ([`Runtime::ingest_device_hook_diagnosis`]). An agent
    /// with no hook runtime, a device not read yet, or a read that found no
    /// version has none known.
    fn composer_version(&self, actor: &Actor) -> Option<String> {
        use hide_agent_hooks::MemoryCompatibility;
        let diagnosis = if self.node == actor.device_id.as_str() {
            self.hook_diagnosis.as_ref()
        } else {
            self.device_hook_diagnoses.get(actor.device_id.as_str())
        }?;
        let id = hide_agent_adapter::adapter(&actor.kind)?.id;
        let row = diagnosis
            .runtimes
            .iter()
            .find(|row| row.runtime.id() == id)?;
        match &row.memory_compatibility {
            MemoryCompatibility::Supported { version } => Some(version.clone()),
            MemoryCompatibility::UpdateRequired {
                installed_version, ..
            } => installed_version.clone(),
        }
        .filter(|version| crate::composer_checks::valid_version(version))
    }

    /// A registered device's hook diagnosis, read on its own machine by the
    /// device's coordinator. Only the answer for a device that is still
    /// registered is kept, so a read that ended after the device was removed
    /// leaves nothing behind. `true` when the kept answer changed.
    pub(crate) fn ingest_device_hook_diagnosis(
        &mut self,
        device_id: &str,
        diagnosis: hide_agent_hooks::Diagnosis,
    ) -> bool {
        if !self.device_registration_exists(device_id)
            || self.device_hook_diagnoses.get(device_id) == Some(&diagnosis)
        {
            return false;
        }
        self.device_hook_diagnoses
            .insert(device_id.to_owned(), diagnosis);
        true
    }

    /// What the doorbell needs to read `observation`'s composer: where to
    /// reach it, the agent's declaration and the version Hide knows for it.
    fn composer_read(
        &self,
        observation: &Observation,
        input_at_unix_ms: u64,
    ) -> Result<crate::delivery::composer::Read, &'static str> {
        let row = hide_agent_adapter::adapter(&observation.actor.kind).ok_or("kind_undeclared")?;
        let composer = row.composer.supported().ok_or("kind_undeclared")?;
        let version = self
            .composer_version(&observation.actor)
            .ok_or("version_unknown")?;
        Ok(crate::delivery::composer::Read {
            observed: observation.clone(),
            agent: row.id,
            composer,
            version,
            input_at_unix_ms,
        })
    }

    /// Whether a pane takes a composer read: the engine reports a current resting
    /// or working state, and neither a key nor
    /// a change of status came in the quiet period.
    fn composer_quiet(observation: &Observation, now: u64) -> bool {
        use crate::delivery::INPUT_QUIET_MS;
        !observation.telemetry_stale
            && matches!(observation.status.as_str(), "idle" | "done" | "working")
            && observation.state_change_seq.is_some()
            && now.saturating_sub(observation.last_input_at_unix_ms) >= INPUT_QUIET_MS
            && now.saturating_sub(observation.status_changed_at_unix_ms) >= INPUT_QUIET_MS
    }

    /// Calibration still needs a known-empty composer after a completed turn.
    fn composer_at_rest(observation: &Observation, now: u64) -> bool {
        matches!(observation.status.as_str(), "idle" | "done")
            && Self::composer_quiet(observation, now)
    }

    /// The composer read that may release `recipient`'s letters held on a
    /// possible draft (docs/delivery.md, Reading the composer). `None` while
    /// the input is not yet quiet on a draft; otherwise the draft generation, with the read or the reason there is
    /// none: no declaration, a version Hide does not know, or one whose
    /// check has not run or has not yet read empty.
    pub(crate) fn composer_release_due(
        &self,
        recipient: &Actor,
        now: u64,
    ) -> Option<(u64, Result<crate::delivery::composer::Read, &'static str>)> {
        if self.effects_held() {
            return None;
        }
        let observation = self
            .delivery_observations
            .get(&recipient.pane_id)
            .filter(|observation| observation.actor.same_identity(recipient))?;
        if !observation.unsent_draft() || !Self::composer_quiet(observation, now) {
            return None;
        }
        let episode = observation.input_generation;
        let read = self
            .composer_read(observation, observation.last_input_at_unix_ms)
            .and_then(
                |read| match self.composer_check(read.agent, &read.version) {
                    Some(check) if check.passed => Ok(read),
                    Some(_) => Err("version_unconfirmed"),
                    None => Err("version_unchecked"),
                },
            );
        Some((episode, read))
    }

    /// The doorbell read `actor`'s composer for the draft that the input at
    /// `observed` described, and the screen showed it empty. Like Deliver
    /// now, input up to it is no draft any more and the pane's composer
    /// mark goes; the doorbell's next look rings with every guard it always
    /// applies. A key or a change of status since the read keeps the hold.
    pub(crate) fn release_empty_composer(&mut self, observed: &Observation, now: u64) -> bool {
        let actor = &observed.actor;
        let unmoved = self.delivery_pane_unmoved(observed).is_ok();
        let Some(observation) =
            self.delivery_observations
                .get_mut(&actor.pane_id)
                .filter(|observation| {
                    unmoved && observation.unsent_draft() && Self::composer_quiet(observation, now)
                })
        else {
            crate::diagnostic!(serde_json::json!({"component":"delivery",
                "kind":"composer.release_refused","pane_id":actor.pane_id,
                "agent":actor.kind,"reason":"input_after_read"}));
            return false;
        };
        observation.released_input_at_unix_ms = observed.last_input_at_unix_ms;
        crate::diagnostic!(serde_json::json!({"component":"delivery",
            "kind":"composer.released","pane_id":actor.pane_id,"agent":actor.kind}));
        self.sync_composer_mark(&actor.pane_id, now);
        true
    }

    /// `agent`'s `version` composer check: `None` when it has not been
    /// checked.
    fn composer_check(&self, agent: &str, version: &str) -> Option<&crate::composer_checks::Check> {
        self.composer_checks
            .iter()
            .find(|check| check.agent == agent && check.version == version)
    }

    /// A pane whose composer is known empty and whose agent version is due a
    /// check: never checked, or not yet read empty and past the wait its
    /// misses earned. The pane's known-empty moment: the turn a prompt hook
    /// of its own session started has ended, neither a key nor Hide's own
    /// typing came since, and it has rested for the quiet period
    /// (docs/delivery.md, Catching an agent's
    /// screen change). One at a time; `attempted` holds the moments already
    /// tried, by pane and hook time, so a pane's moment is read once.
    pub(crate) fn composer_check_due(
        &self,
        attempted: &std::collections::HashMap<String, u64>,
        now: u64,
    ) -> Option<crate::delivery::composer::Read> {
        if self.effects_held() {
            return None;
        }
        self.delivery_observations.values().find_map(|observation| {
            let hook = observation.hook_submit_at_unix_ms;
            let known_empty = hook > 0
                && observation.last_input_at_unix_ms <= hook
                && observation.hide_typed_at_unix_ms <= hook
                && observation.entered_working_at_unix_ms >= hook
                && observation.status_changed_at_unix_ms >= observation.entered_working_at_unix_ms
                && !observation.unsent_draft()
                && Self::composer_at_rest(observation, now);
            if !known_empty || attempted.get(&observation.actor.pane_id) == Some(&hook) {
                return None;
            }
            self.composer_read(observation, hook).ok().filter(|read| {
                self.composer_check(read.agent, &read.version)
                    .is_none_or(|check| check.due(now))
            })
        })
    }

    /// Forgets the check moments of panes that are no longer observed.
    pub(crate) fn retain_composer_attempts(
        &self,
        attempted: &mut std::collections::HashMap<String, u64>,
    ) {
        attempted.retain(|pane, _| self.delivery_observations.contains_key(pane));
    }

    /// Remembers a version's composer check, passed or missed at the step
    /// `miss` names, saves it and returns the version's record.
    pub(crate) fn record_composer_check(
        &mut self,
        agent: &str,
        version: &str,
        miss: Option<&'static str>,
        now: u64,
    ) -> crate::composer_checks::Check {
        let check =
            crate::composer_checks::record(&mut self.composer_checks, agent, version, miss, now);
        if let Some(store) = &self.composer_checks_store {
            store.save(self.composer_checks.clone());
        }
        check
    }

    /// Restores the agent versions earlier runs checked and keeps the store
    /// to save every later check into.
    pub(crate) fn install_composer_checks(
        &mut self,
        store: crate::composer_checks::ComposerChecks,
        restored: Vec<crate::composer_checks::Check>,
    ) {
        self.composer_checks = restored;
        self.composer_checks_store = Some(store);
    }

    /// Where a spawn's child works, named before anything is reserved
    /// (with-hide/hide#2479): with `branch`, a worktree of the repository
    /// `repo` names, as asked; without one, a plain-folder project itself,
    /// the one `repo` names on the child's machine or, with no `repo`, the
    /// caller's own checkout. A Git project keeps both required, since its
    /// agent works in a worktree of its own. A machine the caller cannot
    /// start on is refused as such before any folder there is judged.
    fn spawn_place(
        &mut self,
        caller: &Context,
        machine: Option<&str>,
        repo: Option<String>,
        branch: Option<&str>,
    ) -> Result<String, String> {
        use super::projects::CheckoutKind;
        if branch.is_some() {
            return repo.ok_or_else(|| "invalid_spawn".to_owned());
        }
        let device = machine.unwrap_or(caller.device_id.as_str());
        if device != caller.device_id {
            self.spawn_target(&caller.device_id, device)?;
        }
        let folder = match repo {
            Some(repo) => repo,
            // The caller's checkout is on its own machine.
            None if device == caller.device_id => caller.checkout_path.clone(),
            None => return Err("spawn_repo_required".into()),
        };
        match self.checkout_kind(device, &folder) {
            Some(CheckoutKind::Folder) => Ok(folder),
            Some(CheckoutKind::Unread) => Err("spawn_project_loading".into()),
            Some(CheckoutKind::Unreadable) => Err("spawn_folder_unreadable".into()),
            Some(CheckoutKind::Git) | None => Err("spawn_repo_required".into()),
        }
    }

    pub(crate) fn prepare_delivery(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        hint: Option<&str>,
        mut command: Command,
    ) -> Result<Prepared, String> {
        // The engine numbers panes per machine and an agent learns the numbers from
        // its own machine's engine, so a bare pane id names a pane on the
        // caller's machine; the core's panes are observed under their bare ids
        // and a device's under `remote:<device>:pane:<id>`. A key already
        // already scoped to a device is a full address and stays as it is.
        let node = self.node.as_str().to_owned();
        let qualify = |pane: &str| crate::address::named_pane_id(&node, device, pane);
        // Only Hide's pane attestation names the agent a command acts as; a
        // checkout-bound caller has no pane, and a hint cannot lend it one.
        // The Desk process acts as Hide's own `hide:desk` while it runs.
        let (context, actor) = if Caller::parse(caller) == Caller::Desk {
            if hint.is_some() {
                return Err("caller_identity_conflict".into());
            }
            self.desk_authority(device)?
        } else if let Caller::Access { connection, .. } = Caller::parse(caller) {
            // An external tool acts as its connection's own identity, as
            // Desk does (D-10): never a pane's, so no hint names it. Its
            // credential recorded the context it ran in, which no pane
            // membership can move.
            if hint.is_some() {
                return Err("caller_identity_conflict".into());
            }
            (expected.clone(), self.access_authority(device, connection)?)
        } else {
            let context = self
                .workspace_control_query(device, caller, Query::Info)
                .map_err(|refusal| refusal.reason.to_owned())?
                .context;
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
            (context, actor)
        };
        if context != *expected {
            return Err("caller_context_changed".into());
        }
        // What a pane agent alone may ask is refused to a tool before any
        // of it is read: the tool starts an agent for it (PRD
        // hide-external-control B22, B23).
        crate::runtime::access::admit_tool_command(&actor, &command)?;
        // A machine named by a dialed device's SSH alias is that device's id
        // from here on (`machine_named`).
        if let Command::Agents {
            command:
                crate::coordination::Command::Spawn { machine, .. }
                | crate::coordination::Command::Register { machine, .. },
        } = &mut command
            && let Some(name) = machine.as_mut()
        {
            *name = self.machine_named(name)?;
        }
        let actor_pane = actor.pane_id.as_str();
        // A prompt hook's digest is the agent's own record of the prompt it
        // took, and confirms a brief a spawn typed into this pane. Judged
        // before the native identity: Codex reports its session on the very
        // turn the brief starts, and the engine may not show it yet.
        if let Command::Pull {
            prompt_digest: Some(digest),
            point: hide_node_link::panes::PullPoint::Prompt,
            ..
        } = &command
            && hide_agent_hooks::delivery::valid_prompt_digest(digest)
        {
            self.confirm_brief_by_hook(actor_pane, digest);
        }
        actor.require_native_identity()?;
        if !actor.code_owned() && crate::delivery::reserved_name(&actor.name) {
            // A pane named like a Factory would read as one (D-14); Hide's
            // own actors, such as Desk's `hide:desk`, carry their reserved
            // names.
            return Err("reserved_name".into());
        }
        // A Conductor's spawn is one of its mission's workers: refused before
        // the plan is approved or on another machine, and capped in the
        // ledger (PRD software-factory-conductor B4, B11, B12).
        if let Command::Agents {
            command:
                crate::coordination::Command::Spawn {
                    parent,
                    machine,
                    mission,
                    kind,
                    repo,
                    branch,
                    launch,
                    profile,
                    intent,
                    brief,
                    plan_node,
                    plan,
                    pick,
                    ..
                },
        } = &mut command
        {
            // A plan's child and a mission's worker or checker start no
            // agents, and a step's spawn takes the intent the core makes for
            // its attempt (PRD hide-plan-delegate B1, B7, B8).
            *plan = self
                .plan_spawn_admission(
                    &actor,
                    device,
                    parent.as_deref(),
                    plan_node.as_deref(),
                    self.missions.book.conducted_open(&actor).is_some(),
                )?
                .map(Box::new);
            if let Some(admission) = plan {
                *intent = crate::coordination::plan_intent(&admission.link);
            }
            let ledger = self.delivery_state()?;
            let earlier =
                crate::coordination::earlier_spawn(&ledger, &actor, parent.as_deref(), intent);
            let finished = earlier.is_some_and(|record| record.completed);
            // A folder spawn asked again is the spawn it was: its folder was
            // judged when it was admitted, so one that became a Git
            // repository or lost its row since still answers its agent
            // (engineering 11).
            let admitted = earlier
                .filter(|record| {
                    record.folder
                        && branch.is_none()
                        && repo.as_ref().is_none_or(|repo| *repo == record.repo)
                })
                .map(|record| record.repo.clone());
            let place = match admitted {
                Some(folder) => folder,
                None => {
                    self.spawn_place(&context, machine.as_deref(), repo.take(), branch.as_deref())?
                }
            };
            let repo: &str = repo.insert(place);
            let worker = crate::runtime::missions::SpawnWorker {
                machine: machine.as_deref(),
                kind,
                profile: profile.as_deref(),
                repo,
                launch,
                finished,
            };
            // The item the candidates judge is picked once, here, and the
            // start takes it (PRD agent-profiles-phase2 D-08, B13). A node's
            // worker named by no `--profile` starts as its plan shows it
            // (D-02): that item is the pick, while the spawn keeps the
            // arguments its caller sent, so the same command asked again is
            // the same spawn, and one that finished answers its worker before
            // the plan or the profiles are read again (engineering 11).
            let mut picked = if profile.is_none() && !finished {
                self.planned_request(&actor, machine.as_deref(), kind, intent)?
            } else {
                None
            };
            if plan.is_some() {
                self.plan_candidates_admit(&actor, &worker, &mut picked)?;
            }
            *mission = self
                .mission_spawn_admission(&actor, &worker, &mut picked, intent, brief.is_some())?
                .map(Box::new);
            *pick = picked.map(|request| crate::coordination::Picked(Box::new(request)));
        }
        // Only the pane's own session proves its composer was submitted, so a
        // stray `hide inbox --hook` from a tool in the pane clears no draft.
        // The id is not a secret (D-18 external input stays outside this). The id is checked before it is
        // hashed because this runs under the runtime lock.
        if let Command::Pull {
            session,
            prompt_digest,
            point,
            ..
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
                let own = crate::engine::tokens::session_digest(session) == actor.session;
                // A tool event's ask submits nothing: the composer may hold
                // a draft while the agent works (D-09). It must come from
                // the pane's own session, since it hands letters over.
                if *point != hide_node_link::panes::PullPoint::Prompt {
                    if !own {
                        return Err("session_mismatch".into());
                    }
                } else if own {
                    self.note_prompt_submitted(&actor.pane_id);
                }
            } else if *point != hide_node_link::panes::PullPoint::Prompt {
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
        // A spawn of Desk's past its places waits in Desk's queue (B21).
        let queued = self.desk_spawn_admission(&actor, &command)?;
        // `parent` names the sender's own parent, which a child is never told
        // by id, so its report reaches whoever answers for it; it is read
        // before the retry check, so a retry compares the same recipient. A
        // sender Hide cannot find registered is told why, as `here` is.
        if let Command::Send { target, .. } = &mut command
            && target == crate::coordination::PARENT
        {
            let ledger = self.delivery_state()?;
            *target = crate::coordination::here(&ledger, &actor, unix_milliseconds())?
                .parent
                .clone()
                .ok_or("no_parent")?;
        }
        // A send under an intent its sender already used is a retry only of
        // the same letter; another recipient, kind or body is refused here,
        // before a target is looked for.
        let repeated = match &command {
            Command::Send {
                target,
                intent,
                kind,
                body,
                ..
            } => match self.delivery_ledger.as_ref() {
                Ok(ledger) => crate::delivery::mailbox::retried_send(
                    ledger,
                    &actor,
                    intent,
                    kind,
                    body,
                    None,
                    |stored| {
                        qualify(target) == stored.pane_id
                            || *target == stored.name
                            || crate::coordination::resolve_registered(ledger, target)
                                .is_some_and(|registered| registered.same_identity(stored))
                    },
                    unix_milliseconds(),
                )?
                .is_some(),
                Err(_) => false,
            },
            _ => false,
        };
        let target = match &command {
            Command::Send { .. } if repeated => None,
            // A tool event's ask is judged by the caller's own pane: whether
            // it works, and since when.
            Command::Pull {
                point:
                    hide_node_link::panes::PullPoint::Tool | hide_node_link::panes::PullPoint::TurnEnd,
                ..
            } => self.delivery_observations.get(actor_pane).cloned(),
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
                        || observation.given_name() == Some(target.as_str())
                        || registered
                            .is_some_and(|registered| registered.same_identity(&observation.actor))
                });
                let first = matches.next().cloned();
                if matches.next().is_some() {
                    return Err("target_ambiguous".into());
                }
                // An agent Hide put to sleep takes letters, which wait for
                // its wake (PRD mission-worker-sleep B1), and so does one
                // closed for Reopen closed, whose letters wait for its reopen.
                let first = match (&command, first) {
                    (Command::Send { .. }, None) => {
                        self.delivery_ledger.as_ref().ok().and_then(|ledger| {
                            self.sleeping_target(ledger, registered, &pane)
                                .or_else(|| self.closed_target(ledger, registered, &pane))
                        })
                    }
                    (_, first) => first,
                };
                // A letter that wakes a sleeping plan child waits for a place
                // under its main's cap, and is not kept without one (PRD
                // hide-plan-delegate B6).
                if let (Command::Send { kind, .. }, Some(observation)) = (&command, &first)
                    && matches!(kind.as_str(), "request" | "block")
                    && self.asleep_route(&observation.actor).is_some()
                {
                    self.plan_wake_admission(&actor, &observation.actor)?;
                }
                if first.is_none() && !self.delivery_overflow.is_empty() {
                    return Err("capacity".into());
                }
                first
            }
            _ => None,
        };
        // A report names the Views its sender opened, so the agent that
        // answers for its work finds them (PRD unseen-agent-views D-21).
        if !repeated
            && let Command::Send { kind, body, .. } = &mut command
            && kind == "report"
        {
            self.list_opened_views(&actor.pane_id, body);
        }
        // A mission worker's report or progress often follows a push: its
        // project's pull requests are read again (PRD mission-node-simple
        // D-09, B23).
        if !repeated && let Command::Send { kind, .. } = &command {
            let kind = kind.clone();
            self.note_mission_worker_letter(&actor, &kind);
        }
        if (matches!(&command, Command::Send { .. }) && !repeated)
            || matches!(&command, Command::WatchAssign { .. })
        {
            target
                .as_ref()
                .ok_or("target_unavailable")?
                .actor
                .require_native_identity()?;
        }
        let after = if queued.is_some() {
            self.desk_queued_after()
        } else if !repeated {
            self.desk_after(&actor, &command, target.as_ref())
        } else {
            None
        };
        // A forced letter is typed into a pane that runs now, so its
        // composer can be read; a sleeping recipient is woken by an ordinary
        // letter instead (PRD engine-bundle-1 D-14).
        let forced = match (&command, &target) {
            (Command::Send { force: true, .. }, Some(target)) if !repeated => {
                let live = self
                    .delivery_observations
                    .get(&target.actor.pane_id)
                    .is_some_and(|observation| observation.actor.same_identity(&target.actor));
                if !live {
                    return Err("force_target_asleep".into());
                }
                Some(
                    self.delivery_engine(&target.actor.device_id)
                        .ok_or("delivery_unavailable")?,
                )
            }
            _ => None,
        };
        let also_reads = self.node_exchanges(&actor, &command);
        let prepared = Prepared::new(
            self.delivery_client.clone().ok_or("delivery_unavailable")?,
            Authority {
                caller: caller.to_owned(),
                context,
                also_reads,
            },
            actor,
            target,
            command,
        )
        .worded_in(self.interface_language())
        .then(after);
        let prepared = match forced {
            Some(engine) => prepared.forced_through(engine),
            None => prepared,
        };
        Ok(match queued {
            Some(answer) => prepared.answered(answer),
            None => prepared,
        })
    }

    /// What a `Show` may read beyond its caller's own letters: when the
    /// caller is bound to a mission node, as its worker or as a checker of
    /// one of its lines (its reviewer or verifier), the letters between the
    /// mission's Conductor and each other agent bound to that node, either
    /// way (docs/delivery.md, Who may read a letter). Only the pairs are
    /// worked out here, from the mission book and the registrations; the
    /// store decides the read (`mailbox::show`). An agent of the node that
    /// has since ended still counts, because its letters were written, but
    /// only a reader that runs reads them.
    fn node_exchanges(&self, reader: &Actor, command: &Command) -> Vec<Exchange> {
        let Command::Show { .. } = command else {
            return Vec::new();
        };
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return Vec::new();
        };
        let Some(registration) = ledger
            .agents
            .iter()
            .rev()
            .find(|record| record.actor.same_identity(reader) && !record.ended)
        else {
            return Vec::new();
        };
        self.missions
            .book
            .node_letters_read_by(&registration.id)
            .filter_map(|(other, conductor)| {
                let other = ledger.agents.iter().find(|record| record.id == other)?;
                Some([
                    Exchange {
                        from: other.actor.clone(),
                        to: conductor.clone(),
                    },
                    Exchange {
                        from: conductor.clone(),
                        to: other.actor.clone(),
                    },
                ])
            })
            .flatten()
            .collect()
    }

    pub(crate) fn install_delivery_client(&mut self, client: Client) {
        self.delivery_client = Some(client);
    }

    pub(crate) fn prepare_delivery_human(&mut self) -> Result<PreparedHuman, String> {
        // A notice claimed by a core that may still be rolled back would be
        // lost with its copy; the core that keeps the ledger claims it.
        if self.effects_held() {
            return Err("core_pending".into());
        }
        self.delivery_state()?;
        let client = self.delivery_client.clone().ok_or("delivery_unavailable")?;
        let missions = self.claim_mission_notices();
        Ok(PreparedHuman::new(client, missions))
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
        self.snapshot.overview_graph.available = false;
    }

    /// Publishes `ledger` with its letter index built here, under the lock;
    /// only tests and a restore before the worker runs may pay that.
    #[cfg(test)]
    pub(crate) fn publish_delivery(&mut self, ledger: Arc<Ledger>, transitions: bool) -> bool {
        let index = Arc::new(Correspondence::of(&ledger));
        self.publish_delivery_indexed(ledger, index, transitions)
    }

    /// Publishes a ledger the delivery worker saved, with the letter index it
    /// built from it outside the lock (`delivery::correspondence`).
    pub(crate) fn publish_delivery_indexed(
        &mut self,
        ledger: Arc<Ledger>,
        index: Arc<Correspondence>,
        transitions: bool,
    ) -> bool {
        let now = unix_milliseconds();
        // The turn cards' letters move only when the index's digest does, so
        // a delivery step that changes no line lays no row again.
        let letters_moved = index.digest() != self.letter_index.digest();
        self.letter_index = index;
        // A new letter may wait on a sleeping recipient (PRD
        // mission-worker-sleep B2).
        self.letter_wakes.touch();
        // Or on a session Hide cannot read.
        self.unreadable_letters.touch();
        // One walk of the new letters answers the graph, the newly delivered
        // and the waiters (`ledger_walk`), where each was its own scan and the
        // delivered one scanned the previous ledger per delivered letter.
        let walk = super::ledger_walk::walk(
            self.delivery_ledger
                .as_ref()
                .ok()
                .map(|previous| &**previous),
            &ledger,
            &self.snapshot.overview_graph,
            now,
        );
        let graph_changed = !walk.graph_unchanged;
        if graph_changed {
            let mut graph = crate::model::OverviewGraphSnapshot::from_ledger(&ledger, now);
            graph.sessions = std::mem::take(&mut self.snapshot.overview_graph.sessions);
            self.snapshot.overview_graph = graph;
        }
        let mut reopened = false;
        for pane in &walk.delivered {
            reopened |= self.reopen_resolved_session(pane);
        }
        // The watch views are read only when a transition asked to publish them.
        let changed = transitions && {
            let after = ledger
                .watches
                .iter()
                .map(|watch| watch.view())
                .collect::<Vec<_>>();
            let moved = self.delivery_ledger.as_ref().ok().is_none_or(|before| {
                before.watches.len() != after.len()
                    || before
                        .watches
                        .iter()
                        .zip(&after)
                        .any(|(before, after)| &before.view() != after)
            });
            if moved {
                self.snapshot.delivery_watches = after;
            }
            moved
        };
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
                &record.id == id
                    && !record.ended
                    && record.actor.same_identity(&rebind.from)
                    && (!record.actor.same_identity(&rebind.to)
                        || record.terminal_id != rebind.terminal_id)
            })
        });
        // Only a change in who waits for a reply, from whom, and whether that
        // agent took the letter in moves a row's wait, so the lineage is not
        // rebuilt for every other step of a letter's delivery.
        let waiters_moved = walk.waiters_moved;
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
        if started.access != self.hide_started.access {
            self.access_starts_moved();
        }
        self.hide_started = started;
        let feedback = self.observe_feedback_letters(&ledger);
        // A person's turn declared, replaced or taken down, or its agent's
        // registration moved to another pane, lays the rows again (PRD
        // ask-human).
        let human_moved = self.read_human_turns(&ledger);
        // A child's report to its parent asks it to sleep at its next rest.
        let previous = self.delivery_ledger.as_ref().ok().map(Arc::clone);
        self.note_reports(previous.as_deref(), &ledger);
        self.delivery_ledger = Ok(ledger);
        let continued = self.settle_dormant_registrations();
        self.feed_link_parents();
        let lineage =
            (waiters_moved || started_moved || human_moved) && self.refresh_agent_lineage();
        // A plan step's child moved by its letters or its end (PRD
        // hide-plan-delegate B16, B17).
        let plans = self.fold_delegations();
        changed
            | graph_changed
            | reopened
            | feedback
            | continued
            | lineage
            | plans
            | (human_moved && self.refresh_pane_read_state())
            | (letters_moved && self.sync_request_rows())
            | self.sync_session_state()
            | self.refresh_agent_scopes()
    }

    /// The authority a code-owned caller acts with: a Factory host's own
    /// recipient, or Hide's own identity for one of its roles; never a pane's.
    pub(crate) fn code_owned_authority_current(&self, caller: &str, actor: &Actor) -> bool {
        // An external tool's identity holds while its connection stands at
        // Act, for the credential of that same connection (B12, B13).
        if let Some(connection) = actor.access_connection() {
            return matches!(
                Caller::parse(caller),
                Caller::Access { connection: id, .. } if id == connection
            ) && self
                .access_actor(connection)
                .is_some_and(|(level, current)| {
                    level == crate::access::Level::Act && current.same_identity(actor)
                });
        }
        caller == actor.pane_id
            && (*actor == Actor::hide(crate::delivery::HIDE_MEMORY_ROLE, self.node.as_str())
                || *actor == Actor::hide(super::missions::CONDUCTOR_ROLE, self.node.as_str())
                || *actor
                    == Actor::hide(super::agent_sleep::letters::WAKE_ROLE, self.node.as_str())
                || *actor == Actor::hide(crate::delivery::typed::INPUT_ROLE, self.node.as_str())
                || *actor
                    == Actor::hide(super::unreadable_letters::DELIVERY_ROLE, self.node.as_str())
                || *actor == Actor::hide(super::blocked_children::BLOCKED_ROLE, self.node.as_str())
                || *actor
                    == Actor::hide(
                        super::transition_letters::TRANSITION_ROLE,
                        self.node.as_str(),
                    )
                || (*actor == Actor::hide(crate::delivery::HIDE_DESK_ROLE, self.node.as_str())
                    && self.desk_installed())
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
        let target = self
            .delivery_observations
            .get(target)
            .cloned()
            .ok_or("target_unavailable")?;
        self.operator_prepared(target, command)
    }

    /// The operator's letter to the agent in `target`, awake or asleep: a
    /// Done on its person's turn, which wakes it (PRD ask-human B20).
    pub(super) fn operator_prepare_waking(
        &self,
        target: &str,
        command: crate::delivery::Command,
    ) -> Result<crate::delivery::worker::Prepared, String> {
        let target = match self.delivery_observations.get(target) {
            Some(observation) => observation.clone(),
            None => self
                .delivery_ledger
                .as_ref()
                .ok()
                .and_then(|ledger| {
                    let registered = crate::coordination::resolve_registered(ledger, target);
                    self.sleeping_target(ledger, registered, target)
                })
                .ok_or("target_unavailable")?,
        };
        self.operator_prepared(target, command)
    }

    fn operator_prepared(
        &self,
        target: Observation,
        command: crate::delivery::Command,
    ) -> Result<crate::delivery::worker::Prepared, String> {
        let (client, authority, actor) = self.operator_authority()?;
        Ok(crate::delivery::worker::Prepared::new(
            client,
            authority,
            actor,
            Some(target),
            command,
        ))
    }

    /// The delivery client and the operator's own identity and authority,
    /// which only the shell's events act with.
    pub(crate) fn operator_authority(&self) -> Result<(Client, Authority, Actor), String> {
        let actor = Actor::operator(self.node.as_str());
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
                also_reads: Vec::new(),
            },
            actor,
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
                also_reads: Vec::new(),
            },
            actor,
        ))
    }

    pub(crate) fn delivery_identity_current(&self, actor: &Actor) -> bool {
        self.delivery_observations
            .get(&actor.pane_id)
            .is_some_and(|observation| observation.actor.same_identity(actor))
    }

    /// Running input addresses the pane through its owning engine.
    pub(crate) fn input_target(
        &self,
        observed: &Observation,
    ) -> crate::delivery::ladder::InputTarget {
        crate::delivery::ladder::InputTarget {
            pane_id: observed.actor.pane_id.clone(),
            raw_pane_id: observed.raw_pane_id.clone(),
            kind: observed.actor.kind.clone(),
        }
    }

    pub(crate) fn delivery_engine(&self, device: &str) -> Option<crate::engine::SharedEngine> {
        if self.node == device {
            self.live.as_ref().map(|live| live.engine.clone())
        } else {
            self.device_engine(device)
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

    /// The pane `recipient` runs in while it still hosts the recipient's
    /// session, with the node of its machine, which keeps what the pane's
    /// hooks could not confirm (`Doorbell::settle_owed`). Asking starts no
    /// helper; the node is asked off the lock.
    pub(crate) fn delivery_owed_source(
        &self,
        recipient: &Actor,
    ) -> Option<(Observation, Arc<dyn crate::node_access::NodeLink>)> {
        if self.effects_held() {
            return None;
        }
        let observation =
            self.delivery_observations
                .get(&recipient.pane_id)
                .filter(|observation| {
                    observation.actor.device_id == recipient.device_id
                        && observation.actor.same_identity(recipient)
                })?;
        let (_, link) = self.linked_node(&recipient.device_id).ok()?;
        Some((observation.clone(), link))
    }

    /// The pane as it is now, if nothing moved since `observed`: the same
    /// agent and session, status, state sequence and last input.
    pub(crate) fn delivery_pane_unmoved(
        &self,
        observed: &Observation,
    ) -> Result<&Observation, crate::delivery::doorbell::Hold> {
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
        if current.input_generation != observed.input_generation {
            return Err(Hold::Input);
        }
        Ok(current)
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
        let current = self.delivery_pane_unmoved(observed)?;
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
                                (1..=BELLS).contains(&attempt) && letter.attempts() == attempt
                            }
                            None => letter.attempts() < BELLS,
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
        // A target closed for Reopen closed is neither read nor warned about
        // while it is away: its parent's watch waits for it as it was
        // (`closed_registrations.rs`).
        let away = ledger
            .watches
            .iter()
            .filter(|watch| self.reopen_holds_actor(&watch.target))
            .map(|watch| watch.id.clone())
            .collect::<HashSet<_>>();
        ledger
            .watches
            .iter()
            .filter(|watch| !away.contains(&watch.id))
            .map(|watch| {
                let current = self.delivery_observations.get(&watch.target.pane_id);
                let observation = current
                    .filter(|observation| {
                        !observation.telemetry_stale
                            && watch.target.require_native_identity().is_ok()
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
                    && !self.dormant_holds_actor(&watch.target)
                    && !self.archive_holds_actor(&watch.target)
                    && !ledger.agents.iter().any(|agent| {
                        agent.actor.same_identity(&watch.target)
                            && self.registration_rebinds.contains_key(&agent.id)
                    });
                // The folder the target runs in, or the one its sleeping or
                // archived session would open again, which the tick asks its
                // machine about when the target cannot be read.
                let folder = match &observation {
                    Some(observation) => observation
                        .session
                        .as_ref()
                        .and_then(|session| session.cwd.clone()),
                    None if gone => None,
                    None => self.held_actor_folder(&watch.target),
                };
                let source = crate::delivery::worker::ActivitySource {
                    link: if self.node == watch.target.device_id {
                        Some(Arc::clone(&self.own_node))
                    } else {
                        (observation.is_some() || folder.is_some())
                            .then(|| self.node_link(&watch.target.device_id).ok())
                            .flatten()
                    },
                };
                // The target's own row, while its pane still runs the watched
                // session: stopped on a question. A working one, such as
                // Codex's async question, has not stopped, and one held at a
                // blocked prompt is told to its parent at once by its own
                // note (`blocked_children.rs`).
                let row = observation
                    .as_ref()
                    .and_then(|_| self.agent_row(&watch.target.pane_id))
                    .filter(|row| row.lineage_session == watch.target.session);
                // Input Hide typed and the agent did not confirm is laid on
                // the row as a question (`AskSubject::InputUnconfirmed`), but
                // it is Hide's, which its own notice already told the parent,
                // not one the target asked.
                let asking = row.is_some_and(|row| {
                    row.demand == "question"
                        && row.activity == "stopped"
                        && !row.blocked
                        && !matches!(
                            row.ask_subject,
                            Some(crate::model::AskSubject::InputUnconfirmed { .. })
                        )
                });
                // A mission worker waiting on its pull request's CI, review
                // or merge is not quiet (PRD mission-node-simple D-06, B22).
                let waits = observation.is_some() && self.mission_worker_waits(&watch.target);
                crate::delivery::worker::WatchWork {
                    id: watch.id.clone(),
                    observation,
                    gone,
                    status: watch.last_status.clone(),
                    state_change_seq: watch.last_state_change_seq,
                    status_changed_at_unix_ms: watch.status_changed_at_unix_ms,
                    source,
                    folder,
                    inactivity_ms: None,
                    asking,
                    waits,
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
                also_reads: Vec::new(),
            },
            actor,
        ))
    }
    pub(crate) fn coordination_context(&self, device: &str) -> Result<CoordinationContext, String> {
        let on_node = self.node == device;
        let (connector, scope, machine) = if on_node {
            let live = self.live.as_ref().ok_or("engine_unavailable")?;
            (
                live.engine.clone(),
                live.socket_path
                    .to_str()
                    .ok_or("host_scope_unavailable")?
                    .to_owned(),
                self.node.to_string(),
            )
        } else {
            (
                self.device_engine(device).ok_or("device_unavailable")?,
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
            engine: connector,
            host_scope: scope,
            machine,
            codex: self.codex_daemon(device),
            permissions: self.agent_permissions_of(device),
            on_node,
            node,
        })
    }
}

impl Runtime {
    /// The node link of the device a spawn names with `--machine`, with the
    /// engine that starts the agent there (its definitions say what the
    /// agent's program is called on that machine), or why it cannot take the
    /// spawn: a refusal code the caller can branch on, with the ids it could
    /// have named when the id is not a device at all. The caller's own device
    /// is never named here (the spawn drops it first), and a caller that is
    /// not on this machine may name no other device.
    pub(crate) fn spawn_target(
        &mut self,
        caller_device: &str,
        device: &str,
    ) -> Result<
        (
            Arc<dyn crate::node_access::NodeLink>,
            crate::engine::SharedEngine,
        ),
        String,
    > {
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
        let engine = self
            .coordination_context(device)
            .map_err(|_| "machine_unavailable".to_owned())?
            .engine;
        let link = self
            .node_link(device)
            .map_err(|_| "machine_unavailable".to_owned())?;
        Ok((link, engine))
    }
}

/// What a coordination command needs of the machine it acts on.
pub(crate) struct CoordinationContext {
    pub(crate) engine: crate::engine::SharedEngine,
    pub(crate) host_scope: String,
    pub(crate) machine: String,
    pub(crate) codex: crate::codex_launch::CodexDaemon,
    /// The 권한 of each agent on the machine (`agent_permission`), read as
    /// the spawn asked for it.
    pub(crate) permissions: crate::agent_permission::AgentPermissions,
    /// The machine is the core's own node, whose engine the core reaches directly.
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
        forced: false,
    };
    use crate::delivery::worker::Worker;
    use crate::handle::ChangeNotifier;
    use crate::model::{CoreOptions, SCHEMA_VERSION};
    use crate::sidebar::SessionSnapshotPayload;
    use hide_node_link::panes::HookTurn;
    use hide_node_link::terminal::TerminalReport;
    use hide_session::turns::Waiting;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    pub(crate) fn fixture(
        root: &std::path::Path,
    ) -> (Arc<Mutex<Runtime>>, Actor, Observation, PathBuf) {
        let state = root.join("state");
        hide_platform::fs::private::create_dir_all(&state).unwrap();
        let options: CoreOptions = serde_json::from_value(json!({
            "schema_version":SCHEMA_VERSION,"node_id":"test-node","home":root,
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
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
        runtime.snapshot.status.engine.state = "connected".into();
        let panes = ["sender", "recipient"]
            .into_iter()
            .map(|id| crate::model::PaneSnapshot {
                device_id: Default::default(),
                device_pane_id: Default::default(),
                start_claim: None,
                terminal_id: None,
                failed_terminal_id: None,
                id: id.into(),
                engine_label: None,
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
            also_reads: Vec::new(),
        }
    }

    /// The agent rows the sidebar shows on this machine.
    pub(crate) fn show_agents(runtime: &mut Runtime, payload: SessionSnapshotPayload) {
        runtime.snapshot.navigator.agents = crate::sidebar::project_agents(payload).agents;
    }

    /// Lays on the row of `pane` the core's question about a brief it typed
    /// and the agent did not confirm, as a doorbell pass does
    /// (`Runtime::publish_input_asks`).
    pub(crate) fn lay_unconfirmed_brief(runtime: &mut Runtime, pane: &str) {
        runtime.plans.input_asks = vec![(
            pane.to_owned(),
            1,
            crate::model::AskSubject::InputUnconfirmed {
                input: crate::delivery::typed::InputKind::Brief,
                spent: false,
                held: false,
            },
            "spawn-1".to_owned(),
        )];
        runtime.plans.input_edit += 1;
        assert!(runtime.refresh_plan_reviews(), "the ask is laid on the row");
    }

    /// An archived session whose one member holds `actor`'s registration and
    /// would resume in `folder`.
    pub(crate) fn archive_holding(runtime: &mut Runtime, actor: &Actor, folder: &str) {
        let record = serde_json::from_value(json!({
            "phase":"archived","revision":1,"archived_at_unix_ms":1,
            "project_root":folder,"project_label":"fixture",
            "members":[{"machine":actor.device_id,"pane_id":actor.pane_id,"kind":actor.kind,
                "native_session_id":"recipient-session","cwd":folder,"checkout_path":folder,
                "title":"recipient","registration":{"id":"agent-1","actor":actor},
                "depth":0,"closed":true,"resume":{"state":"idle"}}],
        }))
        .unwrap();
        runtime
            .snapshot
            .ui_state
            .session_archive
            .records
            .insert(crate::session_archive::ArchiveId::new().unwrap(), record);
    }

    pub(crate) fn recipient_at_rest(
        runtime: &mut Runtime,
        native: &str,
        fake: &std::sync::Arc<crate::engine::fake::FakeEngine<'static>>,
    ) -> Observation {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
            {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"idle","state_change_seq":2,
             "lineage_session":crate::engine::tokens::session_digest(native)},
        ]})).unwrap();
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
        let observation = runtime.delivery_observations.get_mut("recipient").unwrap();
        read_nothing_waits(observation);
        observation.status_changed_at_unix_ms = 0;
        observation.last_input_at_unix_ms = 0;
        observation.last_submit_at_unix_ms = 0;
        observation.entered_working_at_unix_ms = 0;
        let observation = observation.clone();
        runtime.live = Some(crate::live::LiveContext {
            socket_path: fake.scope(),
            runtime: std::sync::Weak::new(),
            notifier: ChangeNotifier::noop(),
            engine: fake.engine(),
            node: Arc::new(hide_node::Local::of_process()),
        });
        observation
    }

    /// `pane` conducts a running mission in `/checkouts/fixture` whose
    /// decision `Which base?` the operator answered `main` from the banner.
    pub(crate) fn answered_mission(runtime: &mut Runtime, pane: &str) {
        runtime
            .worktree_catalog
            .projects
            .push(crate::model::ProjectWorktreesSnapshot {
                root_path: "/checkouts/fixture".into(),
                ..Default::default()
            });
        let context = authority(&observed(runtime, pane).actor).context;
        for command in [
            json!({"op": "create", "title": "Fix the bug"}),
            json!({"op": "plan", "plan": {"nodes": [{"key": "fix", "title": "Fix it", "kind": "change", "criteria": ["it works"], "delivery": "pr"}]}}),
            json!({"op": "start"}),
            json!({"op": "decide", "line": "Which base?"}),
        ] {
            runtime
                .mission_command(
                    crate::node::TEST_NODE,
                    pane,
                    &context,
                    serde_json::from_value(command).unwrap(),
                )
                .unwrap();
        }
        assert!(runtime.mission_answer_event("mission-1", 1, None, Some("main".into()), false));
    }

    /// The banner's answer to that mission's decision.
    pub(crate) fn mission_answer(runtime: &Runtime) -> Option<crate::missions::Answer> {
        runtime.missions.book.missions[0].answer.clone()
    }

    /// The pane as the core observes it now.
    pub(crate) fn observed(runtime: &Runtime, pane: &str) -> Observation {
        runtime.delivery_observations[pane].clone()
    }

    /// The pane as the core observes it working since `since`, confirmed by
    /// the engine, with no key typed since.
    pub(crate) fn observe_working_since(runtime: &mut Runtime, pane: &str, since: u64) {
        let observation = runtime.delivery_observations.get_mut(pane).unwrap();
        observation.entered_working_at_unix_ms = since;
        observation.status_changed_at_unix_ms = since;
        observation.last_input_at_unix_ms = since - 1;
        observation.last_submit_at_unix_ms = since - 1;
        observation.engine("working");
    }

    /// The pane as the core observes it after a turn a prompt hook of its
    /// own session started at `hook` has ended, with no key typed since:
    /// its composer is known empty.
    pub(crate) fn observe_turn_ended(runtime: &mut Runtime, pane: &str, hook: u64) {
        let observation = runtime.delivery_observations.get_mut(pane).unwrap();
        observation.last_input_at_unix_ms = hook - 1;
        observation.last_submit_at_unix_ms = hook;
        observation.hook_submit_at_unix_ms = hook;
        observation.entered_working_at_unix_ms = hook + 1;
        observation.status_changed_at_unix_ms = hook + 2;
    }

    /// The engine's state of the pane moves to `status` at `at` without a
    /// read seeing what came between, as a turn that began and ended between
    /// two reads does.
    pub(crate) fn state_moved(runtime: &mut Runtime, pane: &str, status: &str, at: u64) {
        let observation = runtime.delivery_observations.get_mut(pane).unwrap();
        observation.state_change_seq = Some(observation.state_change_seq.unwrap_or(0) + 2);
        observation.status_changed_at_unix_ms = at;
        observation.engine(status);
    }

    /// The pane's turn ends at `at`: the engine reports it idle from then on.
    pub(crate) fn end_turn(runtime: &mut Runtime, pane: &str, at: u64) {
        let observation = runtime.delivery_observations.get_mut(pane).unwrap();
        observation.status_changed_at_unix_ms = at;
        observation.engine("idle");
    }

    /// A prompt hook of the pane's own session reaching the core now.
    pub(crate) fn submit_prompt(runtime: &mut Runtime, pane: &str) {
        runtime.note_prompt_submitted(pane);
    }

    /// The core's engine link, as a device's connection makes and drops it.
    pub(crate) fn set_live(
        runtime: &mut Runtime,
        fake: Option<&std::sync::Arc<crate::engine::fake::FakeEngine<'static>>>,
    ) {
        runtime.live = fake.map(|fake| crate::live::LiveContext {
            socket_path: fake.scope(),
            runtime: std::sync::Weak::new(),
            notifier: ChangeNotifier::noop(),
            engine: fake.engine(),
            node: Arc::new(hide_node::Local::of_process()),
        });
    }

    fn observe_recipient_status(runtime: &mut Runtime, status: &str, sequence: u64) {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
            {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":status,"state_change_seq":sequence,"lineage_session":"recipient-session"},
        ]})).unwrap();
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
        read_nothing_waits(runtime.delivery_observations.get_mut("recipient").unwrap());
    }

    /// The recipient's session read says nothing waits for the operator in
    /// its current state, as the label overlay answers once it has read it;
    /// what the read says is tested on its own
    /// (`the_session_read_decides_the_turn_for_the_current_state_only`).
    pub(crate) fn read_nothing_waits(observation: &mut Observation) {
        observation.reads(Waiting::Nothing);
    }

    /// A hook report of `hook` for `pane`'s session, at `at`, laid by the
    /// next ingest while the engine's state stays as observed: the turn moves on
    /// the hook alone.
    pub(crate) fn lay_hook(runtime: &mut Runtime, pane: &str, hook: HookTurn, at: u64) {
        let observation = &runtime.delivery_observations[pane];
        let observation_generation = observation.observed_generation;
        let mut agent: crate::sidebar::SessionAgentPayload = serde_json::from_value(json!({
            "pane_id": observation.raw_pane_id,
            "agent": observation.actor.kind,
            "agent_status": observation.status,
            "state_change_seq": observation.state_change_seq,
        }))
        .unwrap();
        agent.hook = Some(hook);
        let turn = (
            observation.raw_pane_id.clone(),
            observation.state_change_seq,
            crate::agent_state::turn_projection(&agent),
        );
        let node = runtime.node.clone();
        let pass = ObservedPass(observation_generation);
        runtime.lay_delivery_turns(node.as_str(), pass, [turn]);
        let observation = runtime.delivery_observations.get_mut(pane).unwrap();
        observation.projected.as_mut().unwrap().changed_at_unix_ms = Some(at);
    }

    /// What the recipient's session read says, as a doorbell test sets it:
    /// `None` for a state no read settled.
    pub(crate) fn recipient_reads(runtime: &mut Runtime, wait: Option<Waiting>) {
        let observation = runtime.delivery_observations.get_mut("recipient").unwrap();
        match wait {
            Some(wait) => observation.reads(wait),
            None => observation.turn = Turn::Unread,
        }
    }

    impl Observation {
        /// The pane with the turn the session ingest lays on it: the core's
        /// projection of an agent in the pane's engine state, with `hook` the
        /// turn its session's hook reported and `wait` what its session read
        /// says for that state, laid first, so the engine's clocks say when it
        /// began.
        pub(crate) fn laid(mut self, hook: Option<HookTurn>, wait: Option<Waiting>) -> Self {
            let mut agent: crate::sidebar::SessionAgentPayload = serde_json::from_value(json!({
                "pane_id": self.actor.pane_id,
                "agent": self.actor.kind,
                "agent_status": self.status,
                "state_change_seq": self.state_change_seq,
            }))
            .unwrap();
            agent.hook = hook;
            agent.facts = wait.map(|wait| crate::request_view::RowFacts {
                awaiting_operator: matches!(wait, Waiting::Question | Waiting::PlanApproval),
                user_turn: (wait == Waiting::Question).then_some(
                    hide_session::turns::UserTurnFact {
                        kind: hide_session::turns::UserTurnKind::Question,
                        content: None,
                    },
                ),
                ..Default::default()
            });
            let turn = crate::agent_state::turn_projection(&agent);
            self.projected = Some(ProjectedTurn {
                seq: self.state_change_seq,
                turn,
                generation: self.observed_generation,
                changed_at_unix_ms: None,
            });
            self
        }

        /// The engine reports `status` and the ingest lays the turn again.
        pub(crate) fn engine(&mut self, status: &str) {
            self.status = status.into();
            *self = self.clone().laid(None, None);
        }

        /// The session read settled the state and says `wait`.
        pub(crate) fn reads(&mut self, wait: Waiting) {
            self.turn = Turn::Read;
            *self = self.clone().laid(None, Some(wait));
        }
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
    fn a_delayed_agent_read_keeps_drafts_and_holds_bells_through_equal_recovery() {
        use crate::delivery::doorbell::{Hold, Patience, judge};
        for status in ["idle", "working", "done"] {
            let root = tempfile::tempdir().unwrap();
            let (runtime, parent, recipient, _) = fixture(root.path());
            let mut guard = runtime.lock().unwrap();
            let mut ledger = Ledger::default();
            crate::delivery::watch::start(&mut ledger, &parent, &recipient.actor, 1).unwrap();
            guard.publish_delivery(Arc::new(ledger), false);
            observe_recipient_status(&mut guard, status, 2);
            clocks(&mut guard, "recipient", true);
            crate::runtime::tests::typed_into(&mut guard, "recipient", false);
            let before = clocks(&mut guard, "recipient", false);
            let mut payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":status,
                 "state_change_seq":2,"lineage_session":"recipient-session"}
            ]}))
            .unwrap();
            payload.agents[0].telemetry_stale = true;
            guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
            assert_eq!(clocks(&mut guard, "recipient", false), before);
            let work = guard.delivery_watch_work();
            assert_eq!(work.len(), 1);
            assert!(
                work[0].observation.is_none(),
                "watch cannot confirm retained telemetry"
            );
            assert!(!work[0].gone, "a delayed read is no proof of absence");
            let patience = Patience {
                arrived_at_unix_ms: 0,
                now: true,
                forced: false,
            };
            assert_eq!(
                judge(&observed(&guard, "recipient"), patience, u64::MAX),
                Err(Hold::Status)
            );
            crate::runtime::tests::typed_into(&mut guard, "recipient", false);
            let during = clocks(&mut guard, "recipient", false);
            assert!(during.0 > 0);
            payload.agents[0].telemetry_stale = false;
            guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
            read_nothing_waits(guard.delivery_observations.get_mut("recipient").unwrap());
            assert_eq!(clocks(&mut guard, "recipient", false), during);
            assert_eq!(
                judge(&observed(&guard, "recipient"), patience, u64::MAX),
                Err(Hold::Draft),
                "{status}"
            );
        }
    }

    #[test]
    fn no_hide_key_ends_a_draft_not_even_the_enter_that_opens_a_picker() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        observe_recipient_status(&mut guard, "idle", 2);
        clocks(&mut guard, "recipient", true);
        clocks(&mut guard, "sender", true);

        // `/model` and Enter: the picker is open and the engine reads the pane
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
    /// session read, under engine state `seq`, found a finished plan turn.
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

    /// The overlay for a Codex pane whose session read, under engine state
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
            guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, labels);
            let observation = &guard.delivery_observations["recipient"];
            (
                observation.turn,
                observation
                    .projected_turn()
                    .is_some_and(|projected| projected.turn.operator_wait),
            )
        };
        assert_eq!(
            observe(&mut guard, "codex", 2, "recipient-native", Some(&overlay)),
            (Turn::Read, true)
        );
        assert_eq!(
            observe(&mut guard, "codex", 3, "recipient-native", Some(&overlay)),
            (Turn::Unread, false),
            "a newer state than the read"
        );
        assert_eq!(
            observe(&mut guard, "codex", 2, "another-native", Some(&overlay)),
            (Turn::Unread, false),
            "another session in the pane"
        );
        assert_eq!(
            observe(&mut guard, "codex", 2, "recipient-native", None),
            (Turn::Unread, false)
        );
        assert_eq!(
            observe(&mut guard, "claude", 2, "recipient-native", None),
            (Turn::Unread, false),
            "a supported question reader must finish before the bell rings"
        );
        assert_eq!(
            observe(&mut guard, "opencode", 2, "recipient-native", None),
            (Turn::Unread, false),
            "OpenCode's question reader finishes before a bell too"
        );
        assert_eq!(
            observe(&mut guard, "grok", 2, "recipient-native", None),
            (Turn::Unread, false),
            "Grok's question and plan reader finishes before a bell too"
        );
        assert_eq!(
            observe(&mut guard, "cursor", 2, "recipient-native", None),
            (Turn::NotReported, false)
        );
        // Adapter D-08/B5: every spelling of one agent reads the same turn.
        for kind in [" Codex", "CODEX", "cOdEx"] {
            assert_eq!(
                observe(&mut guard, kind, 2, "recipient-native", Some(&overlay)),
                (Turn::Read, true),
                "the normalized kind as the bell target and session reader read it"
            );
        }
    }

    /// B5: the engine can read Codex done before its session file records the end
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
            guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, Some(&overlay));
            let long_ago = unix_milliseconds().saturating_sub(10 * 60_000);
            let observation = guard.delivery_observations.get_mut("recipient").unwrap();
            observation.last_input_at_unix_ms = long_ago;
            observation.last_submit_at_unix_ms = long_ago;
            observation.status_changed_at_unix_ms = long_ago;
            // The turn changed from the fixture's work with the engine's state.
            if let Some(projected) = observation.projected.as_mut() {
                projected.changed_at_unix_ms = Some(long_ago);
            }
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
        guard.observe_delivery_laid(crate::node::TEST_NODE, &replaced, None, None);
        assert_eq!(
            guard
                .delivery_bell_verdict(&target.actor, WAITED, now)
                .err(),
            Some(Hold::Session)
        );
        let empty: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[]})).unwrap();
        guard.observe_delivery_laid(crate::node::TEST_NODE, &empty, None, None);
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
        let (worker, client) = Worker::store(
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
                terminal_id: None,
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

    /// A child is never told its parent's id, so `parent` names the sender's
    /// own parent as a recipient; a sender with no parent is told it has
    /// none, not that nobody answers.
    #[test]
    fn a_letter_to_parent_reaches_the_senders_own_parent() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, target, path) = fixture(root.path());
        {
            let mut guard = runtime.lock().unwrap();
            // A registration keeps its native session, which the pane's
            // observation carries as its digest.
            let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,
                 "lineage_session":crate::engine::tokens::session_digest("sender-session")},
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"idle","state_change_seq":1,
                 "lineage_session":crate::engine::tokens::session_digest("recipient-session")},
            ]}))
            .unwrap();
            guard.observe_delivery_laid(
                crate::node::TEST_NODE,
                &payload,
                Some("fixture-scope"),
                None,
            );
            let mut ledger = crate::delivery::ledger::Ledger::default();
            for (id, pane, parent) in [
                ("agent-1", "recipient", None),
                ("agent-2", "sender", Some("agent-1")),
            ] {
                ledger.agents.push(crate::coordination::AgentRecord {
                    id: id.into(),
                    name: Some(format!("{pane}-agent")),
                    machine: crate::node::TEST_NODE.into(),
                    host_scope: "fixture-scope".into(),
                    native_machine: "fixture-machine".into(),
                    session: format!("{pane}-session"),
                    instance: format!("terminal-{pane}"),
                    pane: pane.into(),
                    parent: parent.map(str::to_owned),
                    origin: None,
                    project: None,
                    actor: guard.delivery_observations[pane].actor.clone(),
                    ended: false,
                    terminal_id: None,
                    forked: false,
                });
            }
            ledger.next_id = 3;
            ledger.validate().unwrap();
            crate::delivery::ledger::save(&path, &ledger).unwrap();
            guard.delivery_ledger = Ok(Arc::new(crate::delivery::ledger::load(&path).unwrap()));
        }
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        runtime.lock().unwrap().install_delivery_client(client);
        let context = authority(&target.actor).context;
        let send = |caller: &str, intent: &str| {
            let prepared = runtime.lock().unwrap().prepare_delivery(
                crate::node::TEST_NODE,
                caller,
                &context,
                None,
                report_to("parent", intent),
            )?;
            prepared.run(Duration::from_secs(5))
        };
        let result = send("sender", "to-parent").unwrap();
        assert_eq!(result["recipient"]["pane_id"], "recipient");
        // The same letter again is its retry, not a second letter.
        let again = send("sender", "to-parent").unwrap();
        assert_eq!(again["id"], result["id"]);
        assert_eq!(
            send("recipient", "no-parent").err().as_deref(),
            Some("no_parent")
        );
        drop(worker);
    }

    /// A child its spawn started and has not registered yet writes to
    /// `parent` (with-hide/hide#2482). While the spawn is younger than the
    /// time its caller waits for its answer, the child is told its
    /// registration is under way, a refusal that passes; a spawn that old
    /// with no child bound is not going to answer, so the child is no one,
    /// as before. Neither records a letter.
    #[test]
    fn a_letter_to_parent_from_a_child_its_spawn_is_still_registering_is_refused_until_the_spawns_answer_time()
     {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, target, path) = fixture(root.path());
        // The parent in `sender` started the agent in `recipient`, which the
        // engine lists under the spawn's name, with a spawn reserved at
        // `reserved_at`, its pane placed and its start typed, nothing bound.
        let started = |reserved_at: u64| {
            use crate::coordination::{AgentRecord, Mutation, apply};
            let mut guard = runtime.lock().unwrap();
            let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,
                 "lineage_session":crate::engine::tokens::session_digest("sender-session")},
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"working","state_change_seq":1,
                 "lineage_session":crate::engine::tokens::session_digest("recipient-session")},
            ]}))
            .unwrap();
            guard.observe_delivery_laid(
                crate::node::TEST_NODE,
                &payload,
                Some("fixture-scope"),
                None,
            );
            let parent = guard.delivery_observations["sender"].actor.clone();
            let mut ledger = crate::delivery::ledger::Ledger::default();
            ledger.agents.push(AgentRecord {
                id: "agent-1".into(),
                name: Some("lead".into()),
                machine: crate::node::TEST_NODE.into(),
                host_scope: "fixture-scope".into(),
                native_machine: "fixture-machine".into(),
                session: "sender-session".into(),
                instance: "terminal-sender".into(),
                pane: "sender".into(),
                parent: None,
                origin: None,
                project: None,
                actor: parent.clone(),
                ended: false,
                terminal_id: None,
                forked: false,
            });
            ledger.next_id = 2;
            let reserve = Mutation::Reserve {
                parent: "agent-1".into(),
                command: crate::coordination::Command::Spawn {
                    pick: None,
                    mission: None,
                    plan_node: None,
                    plan: None,
                    parent: Some("agent-1".into()),
                    machine: None,
                    name: "recipient".into(),
                    intent: "task-1".into(),
                    kind: "codex".into(),
                    repo: Some("/checkouts/fixture".into()),
                    branch: Some("topic".into()),
                    path: None,
                    args: Vec::new(),
                    launch: Default::default(),
                    task: None,
                    brief: Some("write to your parent first".into()),
                    beside: false,
                    profile: None,
                },
            };
            let spawn = apply(&mut ledger, &parent, &reserve, reserved_at).unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned();
            for step in [
                Mutation::Place {
                    id: spawn.clone(),
                    pane: "recipient".into(),
                },
                Mutation::Typed {
                    id: spawn,
                    profile: None,
                },
            ] {
                apply(&mut ledger, &parent, &step, reserved_at).unwrap();
            }
            ledger.validate().unwrap();
            crate::delivery::ledger::save(&path, &ledger).unwrap();
            guard.delivery_ledger = Ok(Arc::new(crate::delivery::ledger::load(&path).unwrap()));
        };
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        runtime.lock().unwrap().install_delivery_client(client);
        let context = authority(&target.actor).context;
        let send = || {
            let prepared = runtime.lock().unwrap().prepare_delivery(
                crate::node::TEST_NODE,
                "recipient",
                &context,
                None,
                report_to("parent", "plan"),
            )?;
            prepared.run(Duration::from_secs(5))
        };
        let answer_time = crate::coordination::SPAWN_ANSWER_WAIT.as_millis() as u64;
        let now = unix_milliseconds();
        started(now - answer_time + 10_000);
        assert_eq!(send().err().as_deref(), Some("participant_registering"));
        started(now - answer_time - 1);
        assert_eq!(send().err().as_deref(), Some("participant_unavailable"));
        drop(worker);
        assert!(
            crate::delivery::ledger::load(&path)
                .unwrap()
                .letters
                .is_empty(),
            "a refusal records nothing"
        );
    }

    /// `hide agent show` names an agent as a letter names its recipient, by
    /// its id, its name or its pane, and an agent that ended there is
    /// answered `ended` however it is named. Of several registrations a name
    /// or a pane has named, the live one is answered, else the last that
    /// ended; a key that names no agent and no spawn is `agent_unavailable`
    /// (with-hide/hide#2299).
    #[test]
    fn an_agent_that_ended_is_shown_ended_by_its_id_name_or_pane() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, target, path) = fixture(root.path());
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        runtime.lock().unwrap().install_delivery_client(client);
        let context = authority(&target.actor).context;
        let ask = |id: &str| {
            let prepared = runtime.lock().unwrap().prepare_delivery(
                crate::node::TEST_NODE,
                "sender",
                &context,
                None,
                Command::Agents {
                    command: crate::coordination::Command::Show { id: id.into() },
                },
            )?;
            prepared.run(Duration::from_secs(5))
        };
        let shown = |key: &str| {
            let shown = ask(key).unwrap();
            (
                shown["id"].as_str().unwrap().to_owned(),
                shown["runtime"].as_str().unwrap().to_owned(),
                shown["state"]["lifecycle"].as_str().unwrap().to_owned(),
            )
        };
        let ended = |id: &str| (id.to_owned(), "ended".to_owned(), "ended".to_owned());
        let register = |id: &str, pane: &str, ended: bool| {
            let mut guard = runtime.lock().unwrap();
            let actor = guard.delivery_observations[pane].actor.clone();
            let mut ledger = (*guard.delivery_state().unwrap()).clone();
            ledger.agents.push(crate::coordination::AgentRecord {
                id: id.into(),
                name: Some("reviewer".into()),
                machine: crate::node::TEST_NODE.into(),
                host_scope: "fixture-scope".into(),
                native_machine: "fixture-machine".into(),
                session: format!("{pane}-session"),
                instance: format!("terminal-{pane}"),
                pane: pane.into(),
                parent: None,
                origin: None,
                project: None,
                actor,
                ended,
                terminal_id: None,
                forked: false,
            });
            guard.delivery_ledger = Ok(Arc::new(ledger));
        };
        register("agent-7", "recipient", true);
        for key in ["agent-7", "reviewer", "recipient"] {
            assert_eq!(shown(key), ended("agent-7"), "{key}");
        }
        // The name and the pane went to a later agent that ended too: they
        // answer the last, and the earlier stays shown by its id.
        register("agent-8", "recipient", true);
        assert_eq!(shown("reviewer"), ended("agent-8"));
        assert_eq!(shown("recipient"), ended("agent-8"));
        assert_eq!(shown("agent-7"), ended("agent-7"));
        // A live agent of the name is the one the name answers.
        register("agent-9", "sender", false);
        assert_eq!(shown("reviewer").0, "agent-9");
        assert_ne!(shown("reviewer").1, "ended");
        assert_eq!(shown("recipient"), ended("agent-8"));
        // The number alone is no id.
        assert_eq!(ask("7").err().as_deref(), Some("agent_unavailable"));
        drop(worker);
    }

    #[test]
    fn only_a_prompt_hook_of_the_panes_own_session_is_a_submission() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, target, path) = fixture(root.path());
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let mut guard = runtime.lock().unwrap();
        guard.install_delivery_client(client);
        let own = crate::engine::tokens::session_digest("hook-session");
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
            {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"idle","state_change_seq":2,"lineage_session":own},
        ]})).unwrap();
        guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
        let context = authority(&target.actor).context;
        clocks(&mut guard, "recipient", true);
        guard
            .prepare_delivery(
                crate::node::TEST_NODE,
                "recipient",
                &context,
                None,
                Command::Inbox { bundle: true },
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
                        point: hide_node_link::panes::PullPoint::Prompt,
                        bundle: true,
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
                        point: hide_node_link::panes::PullPoint::Prompt,
                        bundle: true,
                    },
                )
                .err()
                .as_deref(),
            Some("prompt_digest_invalid")
        );
        let pull = |session: String| Command::Pull {
            prompt_digest: None,
            session: Some(session),
            point: hide_node_link::panes::PullPoint::Prompt,
            bundle: true,
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

    /// A letter to an agent whose session read refused for a reason reading
    /// again cannot clear is held as unread, and its sender is told once;
    /// the refusal's session being replaced and a read that lands each leave
    /// nothing to tell.
    #[test]
    fn a_letter_held_on_an_unreadable_session_tells_its_sender_once() {
        use crate::delivery::doorbell::{Hold, Patience, Turn};
        let root = tempfile::tempdir().unwrap();
        let (runtime, sender, _, path) = fixture(root.path());
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let mut guard = runtime.lock().unwrap();
        guard.install_delivery_client(client);
        let reason = "session_capacity:line_bytes:262144";
        let refused = LabelOverlay::default().with_refusals([(
            "recipient".to_owned(),
            hide_session::label_reference_token("codex", "id", "recipient-native"),
            reason.to_owned(),
        )]);
        let observe = |guard: &mut Runtime, native: &str, labels: &LabelOverlay| {
            let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working",
                 "state_change_seq":1,"lineage_session":"sender-session"},
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"idle",
                 "state_change_seq":2,"lineage_session":"recipient-session",
                 "agent_session":{"kind":"id","value":native}},
            ]}))
            .unwrap();
            guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, Some(labels));
        };
        observe(&mut guard, "recipient-native", &refused);
        let recipient = guard.delivery_observations["recipient"].actor.clone();
        assert_eq!(guard.delivery_unreadable(&recipient), Some(reason));

        let context = authority(&sender).context;
        let mut letters = Vec::new();
        for intent in ["held", "second"] {
            let prepared = guard
                .prepare_delivery(
                    crate::node::TEST_NODE,
                    "sender",
                    &context,
                    None,
                    Command::Send {
                        target: "recipient".into(),
                        intent: intent.into(),
                        body: format!("{intent} body"),
                        kind: "request".into(),
                        now: false,
                        force: false,
                    },
                )
                .unwrap();
            drop(guard);
            let letter = prepared.run(Duration::from_secs(5)).unwrap();
            letters.push(letter["id"].as_str().unwrap().to_owned());
            guard = runtime.lock().unwrap();
        }
        let ledger = guard.delivery_state().unwrap();
        let held = ledger
            .letters
            .iter()
            .find(|letter| letter.id == letters[0])
            .unwrap();
        assert_eq!(
            guard
                .delivery_bell_verdict(&recipient, Patience::of(held), unix_milliseconds())
                .err(),
            Some(Hold::SessionUnread)
        );

        letters.sort();
        guard.drive_unreadable_letters();
        assert_eq!(guard.unreadable_letters_noted(), letters);
        guard.unreadable_letters.touch();
        guard.drive_unreadable_letters();
        assert_eq!(guard.unreadable_letters_noted(), letters, "told once");

        // The refusal was for the session the pane ran; another one is not
        // known to be unreadable.
        observe(&mut guard, "another-native", &refused);
        assert_eq!(guard.delivery_observations["recipient"].turn, Turn::Unread);
        assert_eq!(guard.delivery_unreadable(&recipient), None);
        // A read that lands takes the refusal away.
        observe(&mut guard, "recipient-native", &LabelOverlay::default());
        assert_eq!(guard.delivery_unreadable(&recipient), None);
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
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let mut guard = runtime.lock().unwrap();
        guard.install_delivery_client(client);
        let own = crate::engine::tokens::session_digest("hook-session");
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
            {"id":"recipient","pane_id":"recipient","agent":"opencode","agent_status":"working","state_change_seq":2,"lineage_session":own},
        ]})).unwrap();
        guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
        let recipient = guard.delivery_observations["recipient"].actor.clone();
        let send = |intent: &str, now: bool| Command::Send {
            target: "recipient".into(),
            intent: intent.into(),
            body: format!("{intent} body"),
            kind: "request".into(),
            now,
            force: false,
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
            point: hide_node_link::panes::PullPoint::Tool,
            bundle: true,
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
                        point: hide_node_link::panes::PullPoint::Tool,
                        bundle: true,
                    },
                )
                .err()
                .as_deref(),
            Some("session_mismatch")
        );
        drop(guard);
        drop(worker);
    }

    /// with-hide/hide-private#560: a sync pass observes a pane for delivery
    /// only in the step under the lock that lays its turn, so a tool or
    /// turn-end ask the agent's hook makes at any moment of the pass is
    /// judged against a laid turn and takes the due letter, rather than
    /// reading the agent as not working and leaving it to the bell.
    #[test]
    fn a_tool_or_turn_end_ask_anywhere_in_a_sync_pass_hands_over_the_due_letter() {
        use hide_node_link::panes::PullPoint;
        for point in [PullPoint::Tool, PullPoint::TurnEnd] {
            let root = tempfile::tempdir().unwrap();
            let (runtime, sender, _, path) = fixture(root.path());
            let (worker, client) = Worker::store(
                Arc::downgrade(&runtime),
                ChangeNotifier::noop(),
                path.clone(),
            )
            .unwrap();
            let mut guard = runtime.lock().unwrap();
            guard.install_delivery_client(client);
            let own = crate::engine::tokens::session_digest("hook-session");
            let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
                {"id":"recipient","pane_id":"recipient","agent":"grok","agent_status":"working","state_change_seq":2,"lineage_session":own},
            ]})).unwrap();
            // The fixture's checkouts are drawn by hand, and an ingest of a
            // payload with no layout would take the callers' context away.
            let drawn = guard.snapshot.navigator.workspaces.clone();
            let ingest = |guard: &mut Runtime| {
                let _ = guard.ingest_observed_session(payload.clone(), None, None, None);
                guard.snapshot.navigator.workspaces = drawn.clone();
            };
            ingest(&mut guard);
            let recipient = guard.delivery_observations["recipient"].actor.clone();
            let prepared = guard
                .prepare_delivery(
                    crate::node::TEST_NODE,
                    "sender",
                    &authority(&sender).context,
                    None,
                    Command::Send {
                        target: "recipient".into(),
                        intent: "urgent".into(),
                        body: "urgent body".into(),
                        kind: "request".into(),
                        now: true,
                        force: false,
                    },
                )
                .unwrap();
            drop(guard);
            let letter = prepared.run(Duration::from_secs(5)).unwrap();
            let ask = || {
                let prepared = runtime
                    .lock()
                    .unwrap()
                    .prepare_delivery(
                        crate::node::TEST_NODE,
                        "recipient",
                        &authority(&recipient).context,
                        None,
                        Command::Pull {
                            prompt_digest: None,
                            session: Some("hook-session".into()),
                            point,
                            bundle: true,
                        },
                    )
                    .unwrap();
                prepared.run(Duration::from_secs(5)).unwrap()["ids"].clone()
            };
            // The next pass has read the engine and takes its catalog inputs
            // under the lock before it ingests.
            runtime.lock().unwrap().begin_catalog_coordinator();
            assert_eq!(ask(), json!([letter["id"]]), "{point:?} before the ingest");
            let observed = runtime.lock().unwrap().delivery_observe_generation;
            ingest(&mut runtime.lock().unwrap());
            assert!(
                runtime.lock().unwrap().delivery_observe_generation > observed,
                "the ingest observed the pane again"
            );
            assert_eq!(ask(), json!([letter["id"]]), "{point:?} after the ingest");
            drop(worker);
        }
    }

    /// with-hide/hide-private#560, a device's side: its coordinator's first
    /// step under the lock observes the device's panes and lays their turns
    /// from the overlay and the hook's facts, so nothing it releases the
    /// lock for (the projection, the ingest) leaves an observation unlaid.
    #[test]
    fn a_device_pass_lays_each_turn_in_the_step_that_observes_it() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"worker","pane_id":"worker","agent":"grok","agent_status":"working","state_change_seq":3,"lineage_session":"worker-session"},
        ]})).unwrap();
        for _ in 0..2 {
            let mut read = payload.clone();
            let _ = guard.observe_device_session("device", &mut read, None);
            let observation =
                &guard.delivery_observations[&crate::address::scoped_pane_id("device", "worker")];
            let projected = observation
                .projected_turn()
                .expect("the step that observed the pane laid its turn");
            assert_eq!(projected.generation, observation.observed_generation);
            assert_eq!(
                projected.turn.activity,
                crate::agent_state::AgentActivity::Working
            );
        }
    }

    #[test]
    fn only_hide_and_the_operator_can_use_their_own_delivery_authority() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        let guard = runtime.lock().unwrap();
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
    }

    #[test]
    fn remote_delivery_uses_attested_pane_and_native_kind_without_a_remote_store() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let (worker, client) = Worker::store(
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
                    session: None,
                    files: crate::model::RemoteFileListSnapshot::idle(),
                    catalog: Default::default(),
                    unreachable_since_unix_ms: None,
                });
            let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[{"id":"remote-sender","pane_id":"sender","agent":"claude","agent_status":"working","state_change_seq":1,"lineage_session":"remote-session"}]})).unwrap();
            guard.observe_delivery_laid("device", &payload, Some("fixture-device"), None);
            // The core's recipient has a name of its own, which is how a
            // device's agent reaches a pane on another machine.
            let core: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
                {"id":"recipient-agent","pane_id":"recipient","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"recipient-session"},
            ]})).unwrap();
            guard.observe_delivery_laid(crate::node::TEST_NODE, &core, None, None);
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
                        Command::Inbox { bundle: true }
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
                        Command::Inbox { bundle: true }
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
                    target: "recipient-agent".into(),
                    intent: "remote-once".into(),
                    body: "done".into(),
                    kind: "report".into(),
                    now: false,
                    force: false,
                },
            )
            .unwrap();
        let result = prepared.run(Duration::from_secs(5)).unwrap();
        assert_eq!(result["sender"]["kind"], "claude");
        assert_eq!(result["sender"]["device_id"], "device");
        assert_eq!(result["sender"]["pane_id"], caller);
        // Stopped before its file is read: the store passes on its own too.
        drop(worker);
        let stored = crate::delivery::ledger::load(&path).unwrap();
        assert_eq!(stored.letters.len(), 1);
        assert_eq!(
            stored.letters[0].sender.session.as_deref(),
            Some("remote-session")
        );
    }

    /// A core with its own `w1:p2`, `w1:p9` and unnamed `w1:p5`, `w1:p6`, and a
    /// connected device whose agents sit in `sender` (the caller), a `w1:p2` and
    /// an unnamed `w1:p5` of its own, and an unnamed `w1:p7`.
    fn two_machines_with_one_pane_id(guard: &mut Runtime) -> (String, String) {
        let local: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"},
            {"id":"core-agent","pane_id":"w1:p2","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"core-session"},
            {"id":"core-only","pane_id":"w1:p9","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"core-only-session"},
            {"id":"w1:p5","pane_id":"w1:p5","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"core-unnamed-session"},
            {"id":"w1:p6","pane_id":"w1:p6","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"core-unnamed-only-session"},
        ]})).unwrap();
        guard.observe_delivery_laid(crate::node::TEST_NODE, &local, None, None);
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
                session: None,
                files: crate::model::RemoteFileListSnapshot::idle(),
                catalog: Default::default(),
                unreachable_since_unix_ms: None,
            });
        let device: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"device-sender","pane_id":"sender","agent":"claude","agent_status":"working","state_change_seq":1,"lineage_session":"device-sender-session"},
            {"id":"device-agent","pane_id":"w1:p2","agent":"claude","agent_status":"working","state_change_seq":1,"lineage_session":"device-session"},
            {"id":"w1:p5","pane_id":"w1:p5","agent":"claude","agent_status":"working","state_change_seq":1,"lineage_session":"device-unnamed-session"},
            {"id":"w1:p7","pane_id":"w1:p7","agent":"claude","agent_status":"working","state_change_seq":1,"lineage_session":"device-unnamed-only-session"},
        ]})).unwrap();
        guard.observe_delivery_laid("device", &device, Some("fixture-device"), None);
        ("sender".to_owned(), "remote:device:pane:sender".to_owned())
    }

    fn report_to(target: &str, intent: &str) -> Command {
        Command::Send {
            target: target.into(),
            intent: intent.into(),
            body: "done".into(),
            kind: "report".into(),
            now: false,
            force: false,
        }
    }

    #[test]
    fn a_bare_pane_id_names_a_pane_on_the_callers_own_machine() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let (worker, client) = Worker::store(
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

        // The device's agent writes `w1:p2`, the number its own engine shows,
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

        // An agent with no name of its own is called by its pane id, which is
        // still a pane address: the caller's machine wins when both have it,
        // and the other machine's agent is not reached when only it has it.
        let result = prepare(
            "device",
            &device_caller,
            report_to("w1:p5", "unnamed-device"),
        )
        .unwrap()
        .run(Duration::from_secs(5))
        .unwrap();
        assert_eq!(result["recipient"]["pane_id"], "remote:device:pane:w1:p5");
        let result = prepare(
            crate::node::TEST_NODE,
            &local_caller,
            report_to("w1:p5", "unnamed-core"),
        )
        .unwrap()
        .run(Duration::from_secs(5))
        .unwrap();
        assert_eq!(result["recipient"]["pane_id"], "w1:p5");
        assert_eq!(
            prepare("device", &device_caller, report_to("w1:p6", "core-unnamed"))
                .err()
                .as_deref(),
            Some("target_unavailable"),
            "the core's unnamed agent is no pane of the device"
        );
        assert_eq!(
            prepare(
                crate::node::TEST_NODE,
                &local_caller,
                report_to("w1:p7", "device-unnamed"),
            )
            .err()
            .as_deref(),
            Some("target_unavailable"),
            "the device's unnamed agent is no pane of the core"
        );

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
        // Stopped before its file is read: the store passes on its own too.
        drop(worker);
        let stored = crate::delivery::ledger::load(&path).unwrap();
        assert_eq!(stored.letters.len(), 5);
    }

    #[test]
    fn a_checkout_bound_caller_is_refused_delivery_and_agent_commands_with_or_without_a_hint() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, _, path) = fixture(root.path());
        let before = std::fs::read(&path).unwrap();
        let (worker, client) = Worker::store(
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
                force: false,
            },
            Command::Inbox { bundle: true },
            Command::WatchStart {
                target: "recipient".into(),
                observer: None,
                actor: None,
            },
            Command::Agents {
                command: crate::coordination::Command::List { live: false },
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
                    Command::Inbox { bundle: true }
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
        let (worker, client) = Worker::store(
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
                        force: false,
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

    /// Two agents in panes `lead` and `child` on this machine's engine at
    /// `scope`, each registered, with `lead` watching `child`.
    fn registered_pair(runtime: &mut Runtime) -> Ledger {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"lead","pane_id":"lead","agent":"claude","agent_status":"idle","state_change_seq":1,
             "lineage_session":crate::engine::tokens::session_digest("lead-native")},
            {"id":"child","pane_id":"child","agent":"codex","agent_status":"working","state_change_seq":1,
             "lineage_session":crate::engine::tokens::session_digest("child-native")},
        ]}))
        .unwrap();
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, Some("scope"), None);
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
                terminal_id: None,
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

    /// Cross-workspace moves change the public address, while the native
    /// terminal, conversation, registration and responsibility remain live.
    #[test]
    fn moved_terminal_keeps_registration_letters_and_watch_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        let old_child;
        {
            let mut guard = runtime.lock().unwrap();
            let mut ledger = registered_pair(&mut guard);
            ledger.agents[1].terminal_id = Some("terminal-child".into());
            old_child = ledger.agents[1].actor.clone();
            let lead = ledger.agents[0].actor.clone();
            crate::delivery::mailbox::send(
                &mut ledger,
                &lead,
                &old_child,
                "keep-request",
                "continue",
                "request",
                None,
                1,
            )
            .unwrap();
            crate::delivery::ledger::save(&path, &ledger).unwrap();
            guard.delivery_ledger = Ok(Arc::new(crate::delivery::ledger::load(&path).unwrap()));
            // A restarted core has no old in-memory observation to guess from.
            guard.delivery_observations.clear();
            let payload = serde_json::from_value(json!({
            "panes":[{"pane_id":"moved-child","terminal_id":"terminal-child"}],
            "agents":[
                {"id":"lead","pane_id":"lead","agent":"claude","agent_status":"idle",
                    "lineage_session":crate::engine::tokens::session_digest("lead-native")},
                {"id":"child","pane_id":"moved-child","agent":"codex","agent_status":"working",
                    "lineage_session":crate::engine::tokens::session_digest("child-native")}
            ]}))
            .unwrap();
            guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, Some("scope"), None);
            assert!(!guard.delivery_registrations_gone().contains_key("agent-2"));
        }
        client
            .submit(
                crate::delivery::worker::Effect::HumanClaim,
                Duration::from_secs(5),
            )
            .unwrap();
        // Stopped before its file is read: the store passes on its own too.
        drop(worker);
        let persisted = crate::delivery::ledger::load(&path).unwrap();
        let child = &persisted.agents[1];
        assert_eq!(child.id, "agent-2");
        assert_eq!(child.pane, "moved-child");
        assert_eq!(child.parent.as_deref(), Some("agent-1"));
        assert_eq!(child.terminal_id.as_deref(), Some("terminal-child"));
        assert!(!child.ended);
        assert_eq!(persisted.watches.len(), 1);
        assert!(persisted.watches[0].target.same_identity(&child.actor));
        assert!(persisted.letters[0].recipient.same_identity(&child.actor));
        assert!(!child.actor.same_identity(&old_child));
        persisted.validate().unwrap();
    }

    #[test]
    fn a_reused_address_or_terminal_does_not_continue_another_session() {
        for (terminal, session, scope) in [
            ("replacement", "child-native", "scope"),
            ("terminal-child", "another-session", "scope"),
            ("terminal-child", "child-native", "another-herdr"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let (runtime, _, _, _) = fixture(root.path());
            let mut guard = runtime.lock().unwrap();
            let mut ledger = registered_pair(&mut guard);
            ledger.agents[1].terminal_id = Some("terminal-child".into());
            guard.delivery_ledger = Ok(Arc::new(ledger));
            let payload = serde_json::from_value(json!({
                "panes":[{"pane_id":"moved-child","terminal_id":terminal}],
                "agents":[{"id":"child","pane_id":"moved-child","agent":"codex",
                    "agent_status":"working","lineage_session":crate::engine::tokens::session_digest(session)}]
            })).unwrap();
            guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, Some(scope), None);
            assert!(
                !guard
                    .delivery_registration_rebinds()
                    .contains_key("agent-2")
            );
        }
    }

    #[test]
    fn remote_pane_move_keeps_the_registered_session_its_letters_and_watch() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        {
            let mut guard = runtime.lock().unwrap();
            let mut ledger = registered_pair(&mut guard);
            let child = &mut ledger.agents[1];
            child.actor.device_id = "device".into();
            child.machine = "device".into();
            child.actor.pane_id = "remote:device:pane:child".into();
            child.host_scope = "device".into();
            child.terminal_id = Some("terminal-child".into());
            let actor = child.actor.clone();
            ledger.watches[0].target = actor.clone();
            let lead = ledger.agents[0].actor.clone();
            crate::delivery::mailbox::send(
                &mut ledger,
                &lead,
                &actor,
                "keep-request",
                "continue",
                "request",
                None,
                1,
            )
            .unwrap();
            crate::delivery::ledger::save(&path, &ledger).unwrap();
            guard.delivery_ledger = Ok(Arc::new(crate::delivery::ledger::load(&path).unwrap()));
            guard.delivery_observations.clear();
            let payload = serde_json::from_value(json!({
                "panes":[{"pane_id":"moved-child","terminal_id":"terminal-child"}],
                "agents":[{"id":"child","pane_id":"moved-child","agent":"codex","agent_status":"working",
                    "lineage_session":crate::engine::tokens::session_digest("child-native")}]
            })).unwrap();
            // This is the Remote coordinator's actual calling convention.
            guard.observe_delivery_laid("device", &payload, None, None);
            assert!(!guard.delivery_registrations_gone().contains_key("agent-2"));
            // Until the store moves the registration, the watch on it still
            // names the old pane, which a watch tick must not read as gone.
            assert!(guard.delivery_watch_work().iter().all(|work| !work.gone));
        }
        client
            .submit(
                crate::delivery::worker::Effect::HumanClaim,
                Duration::from_secs(5),
            )
            .unwrap();
        // Stopped before its file is read: the store passes on its own too.
        drop(worker);
        let persisted = crate::delivery::ledger::load(&path).unwrap();
        let child = &persisted.agents[1];
        assert_eq!(child.actor.pane_id, "remote:device:pane:moved-child");
        assert_eq!(child.parent.as_deref(), Some("agent-1"));
        assert!(!child.ended);
        assert!(persisted.watches[0].target.same_identity(&child.actor));
        assert_eq!(persisted.letters.len(), 1);
        assert!(persisted.letters[0].recipient.same_identity(&child.actor));
        persisted.validate().unwrap();
    }

    fn only_lead() -> SessionSnapshotPayload {
        serde_json::from_value(json!({"agents":[
            {"id":"lead","pane_id":"lead","agent":"claude","agent_status":"idle","state_change_seq":1,
             "lineage_session":crate::engine::tokens::session_digest("lead-native")},
        ]}))
        .unwrap()
    }

    /// The engine closing a registered agent's pane ends its registration and the
    /// watches on it in the durable ledger, and frees its name; the agent
    /// whose pane is still open stays registered.
    #[test]
    fn the_engine_closing_a_registered_pane_ends_the_registration_and_its_watches() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        registered_pair(&mut runtime.lock().unwrap());
        runtime.lock().unwrap().observe_delivery_laid(
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
        // Stopped before its file is read: the store passes on its own too.
        drop(worker);
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
    }

    /// The lead's pane with an agent, and the child's pane a shell: its
    /// engine restored it after a restart and did not resume the agent.
    fn lead_with_the_child_a_shell() -> SessionSnapshotPayload {
        serde_json::from_value(json!({
            "agents": [
                {"id":"lead","pane_id":"lead","agent":"claude","agent_status":"idle","state_change_seq":1,
                 "lineage_session":crate::engine::tokens::session_digest("lead-native")},
            ],
            "panes": [{"pane_id": "lead"}, {"pane_id": "child"}],
        }))
        .unwrap()
    }

    fn not_resumed(pane: &str, session: &str) -> crate::session_sync::NotResumed {
        crate::session_sync::NotResumed {
            pane_id: pane.into(),
            agent: "codex".into(),
            session: Some(session.into()),
            why: "its resume could not be typed (fixture)".into(),
        }
    }

    /// Issue 653: after a reboot a pane can be a shell with the agent's
    /// registration still live, and nothing ever ended it, so `hide agent show`
    /// read it running. The engine names the agent it did not resume, and the
    /// store ends its registration and the watches on it as a closed pane's
    /// would; the agent whose pane runs one stays registered.
    #[test]
    fn an_agent_a_restore_did_not_resume_ends_its_registration_though_its_pane_stays() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        registered_pair(&mut runtime.lock().unwrap());
        {
            let mut guard = runtime.lock().unwrap();
            let shown = lead_with_the_child_a_shell();
            guard.observe_delivery_laid(crate::node::TEST_NODE, &shown, Some("scope"), None);
            assert!(
                guard.delivery_registrations_gone().is_empty(),
                "both panes are there, so no read lists one gone"
            );
            guard.end_unresumed(
                crate::node::TEST_NODE,
                Some("scope"),
                &shown,
                &[
                    not_resumed("child", "child-native"),
                    // The lead's pane runs an agent now, whatever the restore left.
                    not_resumed("lead", "lead-native"),
                ],
            );
            assert_eq!(
                guard
                    .delivery_registrations_gone()
                    .into_iter()
                    .map(|(id, reason)| (id, reason.reason()))
                    .collect::<Vec<_>>(),
                [("agent-2".to_owned(), "agent_not_resumed")]
            );
        }
        client
            .submit(
                crate::delivery::worker::Effect::HumanClaim,
                Duration::from_secs(5),
            )
            .unwrap();
        drop(worker);
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
        // Said again by the next read, which still names it: nothing is left to end.
        let mut guard = runtime.lock().unwrap();
        guard.end_unresumed(
            crate::node::TEST_NODE,
            Some("scope"),
            &lead_with_the_child_a_shell(),
            &[not_resumed("child", "child-native")],
        );
        assert!(guard.delivery_registrations_gone().is_empty());
    }

    /// A registration that belongs to another conversation than the one the
    /// restore did not resume is not that agent's, and one on another engine
    /// or device is not this read's to end.
    #[test]
    fn a_registration_of_another_conversation_or_engine_is_not_ended_by_a_restore() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = fixture(root.path());
        let mut guard = runtime.lock().unwrap();
        registered_pair(&mut guard);
        let shown = lead_with_the_child_a_shell();
        let left = |session: &str| [not_resumed("child", session)];
        // The pane's registration was made for another conversation.
        guard.end_unresumed(
            crate::node::TEST_NODE,
            Some("scope"),
            &shown,
            &left("another"),
        );
        assert!(guard.delivery_registrations_gone().is_empty());
        // Another engine of this machine, and another machine, judge their own.
        guard.end_unresumed(
            crate::node::TEST_NODE,
            Some("another-herdr"),
            &shown,
            &left("child-native"),
        );
        guard.end_unresumed("device", None, &shown, &left("child-native"));
        assert!(guard.delivery_registrations_gone().is_empty());
        // A conversation the save did not name matches the pane's registration.
        let unnamed = [crate::session_sync::NotResumed {
            session: None,
            ..not_resumed("child", "")
        }];
        guard.end_unresumed(crate::node::TEST_NODE, Some("scope"), &shown, &unnamed);
        assert_eq!(guard.delivery_registrations_gone().len(), 1);
    }

    /// The same close ends the answer wait of a request the closed pane's agent
    /// held, in the durable ledger, and its sender reads why through `show`.
    #[test]
    fn the_engine_closing_a_pane_ends_the_answer_wait_of_the_requests_it_held() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, path) = fixture(root.path());
        let (worker, client) = Worker::store(
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
            guard.observe_delivery_laid(crate::node::TEST_NODE, &only_lead(), Some("scope"), None);
            (lead, child, sent.id)
        };
        client
            .submit(
                crate::delivery::worker::Effect::HumanClaim,
                Duration::from_secs(5),
            )
            .unwrap();
        // Stopped before its file is read: the store passes on its own too.
        drop(worker);
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
    }

    /// A pane read ends only this engine's registrations it can vouch for:
    /// none made after its snapshot was asked for, none on another engine or
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
        let factory = Actor::hide("fixture", crate::node::TEST_NODE);
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
        guard.observe_delivery_laid(crate::node::TEST_NODE, &only_lead(), Some("scope"), None);
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
            guard.observe_delivery_laid(crate::node::TEST_NODE, payload, None, None);
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
        runtime.lock().unwrap().observe_delivery_laid(
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
        let fake = crate::engine::fake::FakeEngine::start(move |method, _| {
            assert_eq!(method, "session");
            crate::engine::fake::reply(crate::engine::fake::session(json!({"agents":[
                native("sender", "writer-session"), native("recipient", "reader-session")]})))
        });
        let (worker, client) = Worker::store(
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
            path.clone(),
        )
        .unwrap();
        {
            let mut guard = runtime.lock().unwrap();
            guard.install_delivery_client(client);
            set_live(&mut guard, Some(&fake));
            let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,
                    "lineage_session":crate::engine::tokens::session_digest("writer-session")},
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"working","state_change_seq":1,
                    "lineage_session":crate::engine::tokens::session_digest("reader-session")},
            ]})).unwrap();
            guard.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
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
        let scope = fake.scope().to_str().unwrap().to_owned();
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
                force: false,
            },
        )
        .unwrap();
        let inbox = run("recipient", Command::Inbox { bundle: true }).unwrap();
        let letters = &inbox["letters"];
        assert_eq!(letters.as_array().map(Vec::len), Some(1), "{inbox}");
        assert_eq!(letters[0]["id"], letter["id"]);
        assert_eq!(letters[0]["body"], "read me after a restart");
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

    /// A registered device `id`, reached over SSH, with no helper consent.
    fn register_device(runtime: &mut Runtime, id: &str) {
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: id.to_owned(),
                label: id.to_owned(),
                origin: crate::model::LinkOrigin::Dialed {
                    ssh_alias: id.to_owned(),
                    naming: crate::model::DeviceNaming::Named,
                },
                host_consent: None,
            });
    }

    /// The hook diagnosis of a machine whose Claude Code is `version`.
    fn claude_at(version: &str) -> hide_agent_hooks::Diagnosis {
        use hide_agent_hooks::{
            AgentRuntime, Diagnosis, HOOK_ENTRY_VERSION, HookStatus, MemoryCompatibility,
            RuntimeDiagnosis,
        };
        Diagnosis {
            runtimes: vec![RuntimeDiagnosis {
                runtime: AgentRuntime::ClaudeCode,
                label: "Claude Code".into(),
                path: String::new(),
                status: HookStatus::Installed {
                    version: HOOK_ENTRY_VERSION,
                },
                current_version: HOOK_ENTRY_VERSION,
                memory_compatibility: MemoryCompatibility::Supported {
                    version: version.into(),
                },
            }],
            last_report_failure: None,
            records_error: None,
        }
    }

    /// A Claude Code pane on `device` whose composer may hold a draft: a key
    /// came after its last submission, long enough ago for the input to be
    /// quiet, and the engine reports it at rest.
    fn draft_on_device(runtime: &mut Runtime, device: &str) -> Actor {
        let mut observation = observed(runtime, "recipient");
        observation.actor.device_id = device.to_owned();
        observation.actor.pane_id = format!("remote:{device}:pane:p1");
        observation.actor.kind = "claude".into();
        observation.status = "idle".into();
        observation.state_change_seq = Some(2);
        observation.status_changed_at_unix_ms = 0;
        observation.last_input_at_unix_ms = 1;
        observation.last_submit_at_unix_ms = 0;
        observation.entered_working_at_unix_ms = 0;
        observation.released_input_at_unix_ms = 0;
        assert!(observation.unsent_draft());
        let actor = observation.actor.clone();
        runtime
            .delivery_observations
            .insert(actor.pane_id.clone(), observation);
        actor
    }

    /// A pane on a device is read by the agent version its own machine
    /// reported, the hook diagnosis the device's coordinator reads there
    /// (`ingest_device_hook_diagnosis`): never this machine's, and never
    /// another device's. Without a version the draft such a pane may hold is
    /// never read, so a letter for it waits out its 60 minutes
    /// (docs/delivery.md, Catching an agent's screen change).
    #[test]
    fn a_device_pane_is_read_by_the_version_its_own_machine_reported() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, ..) = fixture(root.path());
        let mut runtime = runtime.lock().unwrap();
        register_device(&mut runtime, "mini");
        register_device(&mut runtime, "box");
        let actor = draft_on_device(&mut runtime, "mini");
        let now = unix_milliseconds();
        let due = |runtime: &Runtime| {
            runtime
                .composer_release_due(&actor, now)
                .map(|(_, read)| read.map(|read| read.version))
        };

        assert_eq!(due(&runtime), Some(Err("version_unknown")));
        // This machine's Claude Code says nothing about the device's.
        runtime.hook_diagnosis = Some(claude_at("9.9.9"));
        assert_eq!(due(&runtime), Some(Err("version_unknown")));
        // Nor does another device's.
        assert!(runtime.ingest_device_hook_diagnosis("box", claude_at("1.0.0")));
        assert_eq!(due(&runtime), Some(Err("version_unknown")));

        // The device's own: known, then checked on a screen that read empty.
        assert!(runtime.ingest_device_hook_diagnosis("mini", claude_at("2.1.319")));
        assert_eq!(due(&runtime), Some(Err("version_unchecked")));
        let agent = hide_agent_adapter::adapter("claude").unwrap().id;
        runtime.record_composer_check(agent, "2.1.319", None, now);
        assert_eq!(due(&runtime), Some(Ok("2.1.319".to_owned())));

        // The same answer read again is no change, and an update is a new
        // version to check.
        assert!(!runtime.ingest_device_hook_diagnosis("mini", claude_at("2.1.319")));
        assert!(runtime.ingest_device_hook_diagnosis("mini", claude_at("2.1.320")));
        assert_eq!(due(&runtime), Some(Err("version_unchecked")));
    }

    /// What a device reported is kept only for a registered device and goes
    /// with it, so an answer that lands after the device was removed leaves
    /// nothing behind and the table is bounded by the device list.
    #[test]
    fn a_device_s_agent_versions_go_with_the_device() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, ..) = fixture(root.path());
        let mut runtime = runtime.lock().unwrap();
        register_device(&mut runtime, "mini");

        assert!(!runtime.ingest_device_hook_diagnosis("ghost", claude_at("2.1.319")));
        assert!(runtime.device_hook_diagnoses.is_empty());

        assert!(runtime.ingest_device_hook_diagnosis("mini", claude_at("2.1.319")));
        assert_eq!(runtime.device_hook_diagnoses.len(), 1);
        runtime.forget_device_host("mini", super::super::devices::Retiring::Removed);
        assert!(runtime.device_hook_diagnoses.is_empty());
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .retain(|device| device.id != "mini");
        assert!(!runtime.ingest_device_hook_diagnosis("mini", claude_at("2.1.319")));
        assert!(runtime.device_hook_diagnoses.is_empty());
    }
}
