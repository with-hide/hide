//! What the Factory host reads from and asks of the runtime. Each call takes
//! the lock for owned data only; no subprocess, file or network work happens
//! here (docs/ARCHITECTURE.md).

use std::collections::HashMap;

use crate::delivery::worker::{Authority, Prepared};
use crate::delivery::{Actor, Command};
use crate::runtime::delivery::Observation;
use crate::workspace_control::Query;

use super::Runtime;
use crate::agent_sleep::{DormantPhase, DormantRecord, FactoryWorker, SleepId};
use crate::factory::screen::{
    ActionAnswer, FactorySection, FactoryTaskSection, REQUEST_ID_LIMIT, ScreenRequest,
};
use crate::model::Edited;

/// `factory_action`: one person's action on a Factory screen, the stage-1
/// command as the CLI sends it (PRD software-factory-ui).
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FactoryActionPayload {
    request_id: String,
    command: hide_factory::Command,
}

/// `factory_task_open`: the Task page a screen shows.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FactoryTaskOpenPayload {
    factory: String,
    task: String,
}

/// A worker pane as the runtime sees it now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkerProbe {
    /// Herdr shows an agent in the pane.
    pub present: bool,
    /// A close Hide asked for is in flight for the pane, so its agent
    /// vanishing is that close, not the worker going away.
    pub closing: bool,
    /// The agent sleeps (or its end is in flight).
    pub asleep: bool,
    /// What the agent is doing, from the status model's axes; work it
    /// delegated counts as working.
    pub activity: crate::agent_state::AgentUse,
    /// When the core saw the agent's state last change (D-52); `None`
    /// before the core has observed it.
    pub changed_at_unix_ms: Option<u64>,
}

/// A Factory worker's pane, as the Factory host publishes it (#857).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FactoryPane {
    pub kind: String,
    pub started_at: u64,
    pub worker: FactoryWorker,
}

/// What the Factory host reads of a worker whose conversation was kept when
/// its pane closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DormantWorker {
    pub id: SleepId,
    pub worker: FactoryWorker,
    /// The pane the worker had; the Task still names it until it is bound.
    pub old_pane: String,
    pub kind: String,
    pub state: DormantState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DormantState {
    /// The conversation runs in this pane, confirmed by the core.
    /// `continued`: the registration the worker held is now the pane's own
    /// (the delivery store moved it with its letters and watch), so the
    /// Factory keeps the same agent id instead of registering a new one.
    Woken { pane: String, continued: bool },
    /// The wake did not complete; the core keeps the conversation.
    WakeFailed(&'static str),
}

/// How a Factory wake of one worker went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FactoryWake {
    /// The core is waking the worker; its pane is bound when it is back.
    Asked,
    /// The worker's sleep is still landing or the core is busy: ask again.
    Later,
    /// Nothing sleeps: the worker's agent runs in its pane.
    Awake,
    Refused(&'static str),
}

/// What a diagnosis may read about a worker (D-37, D-52): the user turn
/// and last answer its adapter declares, from the agent's row, and where
/// to read its screen off the lock.
pub(crate) struct WorkerTextSources {
    pub user_turn: Option<String>,
    pub last_answer: Option<String>,
    pub raw_pane: Option<String>,
    pub connector: Option<std::sync::Arc<dyn hide_herdr_client::ApiConnector>>,
}

/// A worker's machine as the Factory host reaches it, off the lock.
pub(crate) struct WorkerReach {
    pub connector: std::sync::Arc<dyn hide_herdr_client::ApiConnector>,
    pub node: std::sync::Arc<dyn crate::node_access::NodeLink>,
    /// The worker's pane as its machine's Herdr names it.
    pub herdr_pane: Option<String>,
}

/// A worker's pane as the core names it: Herdr's own id on the core's
/// machine, under its node's prefix on another (`WorkerRef::node`).
pub(crate) fn factory_pane_id(node: Option<&str>, herdr_pane: &str) -> String {
    match node {
        None => herdr_pane.to_owned(),
        Some(node) => super::operations::remote_pane_id(node, herdr_pane),
    }
}

/// The reverse of [`factory_pane_id`]; `None` for a pane another machine
/// names.
pub(crate) fn factory_herdr_pane<'a>(node: Option<&str>, pane: &'a str) -> Option<&'a str> {
    match node {
        None => Some(pane).filter(|pane| !super::is_remote_scoped_pane_id(pane)),
        Some(node) => super::remote_pane_source_id(node, pane),
    }
}

/// How far up the spawn lineage a caller is followed to its worker.
const LINEAGE_LIMIT: usize = 16;

impl Runtime {
    /// Factory authority is narrower than the shared node file readers:
    /// only the core's own node and an inbound node may run its work.
    pub(crate) fn factory_linked_node(
        &self,
        device: &str,
    ) -> Result<super::hosts::LinkedNode, String> {
        self.factory_node(device)
            .map_err(|refusal| refusal.reason)?;
        self.linked_node(device).map_err(str::to_owned)
    }

    /// What a worker start on the machine `node` names holds on to: that
    /// machine's Herdr and node, the core's own or a node linked to it (Q17).
    pub(crate) fn factory_start_control(
        &self,
        node: Option<&str>,
    ) -> Result<crate::factory::StartControl, String> {
        let device = node.unwrap_or(self.node.as_str());
        let own = node.is_none();
        let connector = self.delivery_connector(device).ok_or(if own {
            "herdr_unavailable"
        } else {
            "device_unavailable"
        })?;
        let (_, node) = self.factory_linked_node(device)?;
        Ok(crate::factory::StartControl {
            device: device.to_owned(),
            connector,
            node,
            generation: own.then_some(self.live_generation),
        })
    }

    /// Whether `control` still names the connection its machine has: a
    /// Herdr or link that came back since is another connection.
    pub(crate) fn factory_start_control_current(
        &self,
        control: &crate::factory::StartControl,
    ) -> bool {
        control
            .generation
            .is_none_or(|generation| generation == self.live_generation)
            && self
                .delivery_connector(&control.device)
                .is_some_and(|connector| std::sync::Arc::ptr_eq(&connector, &control.connector))
            && self
                .factory_linked_node(&control.device)
                .is_ok_and(|(_, node)| std::sync::Arc::ptr_eq(&node, &control.node))
    }

    /// The node a Factory a screen makes for a project on `device` names:
    /// none for the core's own machine, the node for one that dials the
    /// core. A device the core dials keeps no Factory (Q17).
    pub(crate) fn factory_node(
        &self,
        device: &str,
    ) -> Result<Option<String>, hide_factory::Refusal> {
        const NEXT: &str = "Choose a project on the core's machine or on a machine linked to it";
        if device == self.node.as_str() {
            return Ok(None);
        }
        match self.link_origin(device) {
            Some(crate::model::LinkOrigin::Inbound) => Ok(Some(device.to_owned())),
            Some(_) => Err(hide_factory::Refusal::new(
                "factory_device_unsupported",
                NEXT,
            )),
            None => Err(hide_factory::Refusal::new("factory_device_unknown", NEXT)),
        }
    }

    /// Where a worker's machine work goes: its machine's Herdr and node,
    /// and its pane as that Herdr names it.
    pub(crate) fn factory_worker_reach(
        &self,
        worker: &hide_factory::model::WorkerRef,
    ) -> Result<WorkerReach, String> {
        let device = worker.node.as_deref().unwrap_or(self.node.as_str());
        let connector = self
            .delivery_connector(device)
            .ok_or("device_unavailable")?;
        let (_, node) = self.factory_linked_node(device)?;
        let herdr_pane = match &worker.pane {
            Some(pane) => Some(
                factory_herdr_pane(worker.node.as_deref(), pane)
                    .ok_or("worker_pane_invalid")?
                    .to_owned(),
            ),
            None => None,
        };
        Ok(WorkerReach {
            connector,
            node,
            herdr_pane,
        })
    }

    pub(crate) fn set_factory_screen_port(&mut self, port: crate::factory::ScreenPort) {
        self.factory_screen = Some(port);
    }

    /// The Factory host's changed values; it builds and compares them off
    /// the lock, so this only swaps them in under a new edit number.
    pub(crate) fn set_factory_screen(
        &mut self,
        summary: Option<std::sync::Arc<hide_factory::FactorySummary>>,
        task: Option<Option<FactoryTaskSection>>,
    ) {
        if let Some(summary) = summary {
            self.snapshot
                .factory
                .get_or_insert_with(Edited::default)
                .edit()
                .summary = Some(summary);
            self.refresh_agent_scopes();
        }
        if let Some(task) = task {
            self.snapshot.factory_task = task.map(Edited::new);
        }
    }

    pub(crate) fn factory_answered(&mut self, answer: ActionAnswer) {
        self.snapshot
            .factory
            .get_or_insert_with(Edited::<FactorySection>::default)
            .edit()
            .push_answer(answer);
    }

    /// Hands a screen action to the engine thread; a queue that cannot take
    /// it is answered at once on the same request id.
    pub(super) fn factory_action(&mut self, payload: FactoryActionPayload) -> bool {
        if payload.request_id.is_empty() || payload.request_id.len() > REQUEST_ID_LIMIT {
            crate::diagnostic!(serde_json::json!({
                "component": "factory",
                "kind": "screen.request_id_invalid",
                "length": payload.request_id.len(),
            }));
            return false;
        }
        let request_id = payload.request_id.clone();
        let sent = match &self.factory_screen {
            Some(port) => port.send(ScreenRequest::Action {
                request_id: payload.request_id,
                command: payload.command,
            }),
            None => Err("factory_unavailable"),
        };
        match sent {
            Ok(()) => false,
            Err(reason) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "factory",
                    "kind": "screen.action_refused",
                    "request_id": request_id,
                    "reason": reason,
                }));
                // Spelled out so the screen's refusal labels can be held to them.
                let refusal = if reason == "factory_busy" {
                    hide_factory::Refusal::new("factory_busy", "Try again in a moment")
                } else {
                    hide_factory::Refusal::new("factory_unavailable", "See the diagnostic log")
                };
                self.factory_answered(ActionAnswer {
                    request_id,
                    answer: refusal.to_json(),
                });
                true
            }
        }
    }

    pub(super) fn factory_task_open(&mut self, payload: FactoryTaskOpenPayload) -> bool {
        self.factory_screen_send(ScreenRequest::OpenTask {
            factory: payload.factory,
            task: payload.task,
        })
    }

    pub(super) fn factory_task_close(&mut self) -> bool {
        self.factory_screen_send(ScreenRequest::CloseTask)
    }

    /// Remembers the Home agent pane started as the Factory secretary (B23).
    /// A pane no tab holds is refused and logged; the screen starts again.
    pub(super) fn factory_secretary_set(&mut self, pane_id: String) -> bool {
        if !self.pane_listed(&pane_id) {
            crate::diagnostic!(serde_json::json!({
                "component": "factory",
                "kind": "screen.secretary_unlisted",
                "pane_id": pane_id,
            }));
            return false;
        }
        if self.snapshot.ui_state.factory_secretary_pane.as_deref() == Some(pane_id.as_str()) {
            return false;
        }
        self.snapshot.ui_state.factory_secretary_pane = Some(pane_id);
        self.persist_ui_state();
        true
    }

    /// A page the queue could not take is logged; the screen keeps its
    /// skeleton and the next open asks again (design #13).
    fn factory_screen_send(&mut self, request: ScreenRequest) -> bool {
        let sent = match &self.factory_screen {
            Some(port) => port.send(request),
            None => Err("factory_unavailable"),
        };
        if let Err(reason) = sent {
            crate::diagnostic!(serde_json::json!({
                "component": "factory",
                "kind": "screen.page_refused",
                "reason": reason,
            }));
        }
        false
    }

    /// Checks a caller on the core's machine or a node linked to it like a
    /// delivery request does and names the pane and checkout it speaks from.
    /// A checkout-bound caller is the operator acting without a pane: its
    /// hint is never identity or lineage.
    pub(crate) fn factory_caller(
        &self,
        device: &str,
        caller: &str,
        expected: &crate::workspace_control::Context,
        hint: Option<&str>,
    ) -> Result<crate::factory::FactoryCaller, String> {
        self.factory_linked_node(device)
            .map_err(|_| "factory_local_only")?;
        let context = self
            .workspace_control_query(device, caller, Query::Info)
            .map_err(|refusal| refusal.reason.to_owned())?
            .context;
        if context != *expected {
            return Err("caller_context_changed".into());
        }
        let node = (device != self.node.as_str()).then(|| device.to_owned());
        let pane = match crate::workspace_control::Caller::parse(caller) {
            crate::workspace_control::Caller::Pane(pane) => Some(pane.to_owned()),
            crate::workspace_control::Caller::Checkout { .. } => None,
        };
        // A hint naming another pane than a pane caller's own can only make
        // the caller a worker, never an operator. A node's hint may name the
        // pane as its own Herdr does.
        let claimed = pane
            .as_deref()
            .and(hint)
            .map(|hint| match node.as_deref() {
                Some(node) if factory_herdr_pane(Some(node), hint).is_none() => {
                    factory_pane_id(Some(node), hint)
                }
                _ => hint.to_owned(),
            })
            .filter(|hint| pane.as_deref() != Some(hint.as_str()));
        let ancestors = pane
            .as_deref()
            .map_or_else(crate::factory::Lineage::none, |pane| {
                self.factory_lineage(pane)
            });
        Ok(crate::factory::FactoryCaller {
            node,
            pane,
            cwd: Some(context.checkout_path.clone()).filter(|path| !path.is_empty()),
            claimed,
            ancestors,
        })
    }

    pub(crate) fn factory_question_caller(
        &self,
        device: &str,
        caller: &str,
        expected: &crate::workspace_control::Context,
        session: &str,
        runtime: &str,
        terminal_id: &str,
    ) -> Result<crate::factory::QuestionCaller, String> {
        // Factory workers run on the core's machine or a node linked to
        // it (Q17). A pane on a device the core dials cannot be one of
        // them, and must not open an SSH API channel under this short
        // question budget.
        if self.factory_linked_node(device).is_err() {
            return Err("factory_guard_local_only".into());
        }
        let crate::workspace_control::Caller::Pane(pane) =
            crate::workspace_control::Caller::parse(caller)
        else {
            return Err("agent_pane_required".into());
        };
        let context = self
            .workspace_control_query(device, caller, Query::Info)
            .map_err(|_| "factory_guard_context_changed")?
            .context;
        if context != *expected
            || !crate::delivery::valid_session(session)
            || !crate::delivery::valid_key(terminal_id)
        {
            return Err("factory_guard_context_changed".into());
        }
        let observation = self
            .delivery_observations
            .get(pane)
            .ok_or("factory_guard_native_unavailable")?;
        let kind =
            hide_agent_adapter::direct_ask_kind(runtime).ok_or("factory_guard_runtime_invalid")?;
        let actor = &observation.actor;
        actor.require_native_identity()?;
        if actor.device_id != device
            || actor.kind != kind
            || crate::wire::session_digest(session) != actor.session
        {
            return Err("factory_guard_native_changed".into());
        }
        Ok(crate::factory::QuestionCaller {
            actor: actor.clone(),
            context,
            raw_pane: observation.raw_pane_id.clone(),
            terminal_id: terminal_id.to_owned(),
            connector: self
                .delivery_connector(device)
                .ok_or("factory_guard_disconnected")?,
        })
    }

    /// Join the accepted spawn to the independently re-read execution.
    /// An ended ledger record can still be the current cancelled/revived
    /// worker; its native identity, not its ended bit, decides that join.
    pub(crate) fn factory_question_current(
        &self,
        caller: &crate::factory::QuestionCaller,
        worker: &hide_factory::model::WorkerRef,
    ) -> Result<(), String> {
        let actor = &caller.actor;
        // Context and connection are re-read without invoking a transport.
        let context = self
            .workspace_control_query(&actor.device_id, &actor.pane_id, Query::Info)
            .map_err(|_| "factory_guard_context_changed")?
            .context;
        let native = self
            .delivery_observations
            .get(&actor.pane_id)
            .ok_or("factory_guard_native_changed")?;
        let connector = self
            .delivery_connector(&actor.device_id)
            .ok_or("factory_guard_disconnected")?;
        if context != caller.context
            || !native.actor.same_identity(actor)
            || native.actor.kind != actor.kind
            || native.actor.name != actor.name
            || native.raw_pane_id != caller.raw_pane
            || !std::sync::Arc::ptr_eq(&connector, &caller.connector)
        {
            return Err("factory_guard_native_changed".into());
        }
        let ledger = self.delivery_state()?;
        let record = worker
            .agent
            .as_deref()
            .and_then(|id| ledger.agents.iter().find(|a| a.id == id))
            .ok_or("factory_guard_spawn_unavailable")?;
        let parent = record
            .parent
            .as_deref()
            .and_then(|id| ledger.agents.iter().find(|a| a.id == id))
            .ok_or("factory_guard_spawn_unavailable")?;
        let native_context = self.coordination_context(&actor.device_id)?;
        if worker.pane.as_deref() != Some(actor.pane_id.as_str())
            || worker.runtime.as_str() != actor.kind
            || !record.actor.same_identity(actor)
            || record.actor.kind != actor.kind
            // The ledger keeps the pane as its own Herdr names it; a node's
            // pane is qualified only in the core's ids.
            || record.pane != caller.raw_pane
            || record.native_machine != native_context.machine
            || record.host_scope != native_context.host_scope
            || !parent.actor.same_identity(&crate::delivery::Actor::factory(
                &worker.factory,
                self.node.as_str(),
            ))
            || !parent.actor.code_owned()
        {
            return Err("factory_guard_spawn_changed".into());
        }
        Ok(())
    }

    /// The agents above `pane` in the spawn lineage, nearest first, from
    /// the ledger in memory; at most [`LINEAGE_LIMIT`] steps.
    fn factory_lineage(&self, pane: &str) -> crate::factory::Lineage {
        let mut lineage = crate::factory::Lineage {
            complete: false,
            ..crate::factory::Lineage::none()
        };
        let Ok(ledger) = self.delivery_state() else {
            return lineage;
        };
        // The newest record on the pane, ended or not: an agent can end its
        // own record and keep running, so ending it never makes an operator.
        // Nor does registering again without a parent: the same agent's
        // newest record that names one still decides where it sits.
        let newest = ledger
            .agents
            .iter()
            .rev()
            .find(|agent| agent.actor.pane_id == pane);
        let start = newest.and_then(|newest| {
            ledger
                .agents
                .iter()
                .rev()
                .filter(|agent| {
                    agent.actor.pane_id == pane && agent.actor.same_identity(&newest.actor)
                })
                .find(|agent| agent.parent.is_some())
                .or(Some(newest))
        });
        let mut next = match start {
            Some(agent) => {
                lineage.factory_spawned |= agent.actor.code_owned();
                agent.parent.clone()
            }
            None => None,
        };
        while let Some(id) = next {
            if lineage.agents.len() >= LINEAGE_LIMIT || lineage.agents.contains(&id) {
                return lineage;
            }
            let Some(agent) = ledger.agents.iter().find(|agent| agent.id == id) else {
                lineage.agents.push(id);
                return lineage;
            };
            lineage.agents.push(id);
            lineage.factory_spawned |= agent.actor.code_owned();
            // An ended agent's pane may be someone else's now.
            if !agent.ended {
                lineage.panes.push(agent.actor.pane_id.clone());
            }
            next = agent.parent.clone();
        }
        lineage.complete = true;
        lineage
    }

    /// A delivery command the Factory sends as its own recipient, to the pane
    /// `target` when the command has one.
    pub(crate) fn factory_prepare(
        &self,
        id: &str,
        target: Option<&str>,
        command: Command,
    ) -> Result<Prepared, String> {
        let (client, authority, actor) = self.factory_delivery(id)?;
        let target = match target {
            Some(pane) => Some(
                self.delivery_observations
                    .get(pane)
                    .cloned()
                    .ok_or("target_unavailable")?,
            ),
            None => None,
        };
        Ok(Prepared::new(client, authority, actor, target, command))
    }

    /// A worker's report to its Factory as a ledger letter from the worker's
    /// own pane (PRD software-factory: ask/block/propose/done use delivery).
    pub(crate) fn factory_worker_letter(
        &self,
        pane: &str,
        factory: &str,
        command: Command,
    ) -> Result<Prepared, String> {
        let observation = self
            .delivery_observations
            .get(pane)
            .ok_or("agent_pane_required")?;
        let actor = observation.actor.clone();
        actor.require_native_identity()?;
        let context = self
            .workspace_control_query(&actor.device_id, pane, Query::Info)
            .map_err(|_| "agent_pane_required")?
            .context;
        let recipient = Actor::factory(factory, self.node.as_str());
        if !self.factory_recipient_current(&recipient) {
            return Err("target_unavailable".into());
        }
        Ok(Prepared::new(
            self.delivery_client.clone().ok_or("delivery_unavailable")?,
            Authority {
                caller: pane.to_owned(),
                context,
            },
            actor,
            Some(Observation::code_owned(recipient)),
            command,
        ))
    }

    pub(crate) fn factory_worker_probe(&self, pane: &str) -> WorkerProbe {
        use crate::agent_state::AgentUse;
        // A close-pane sleep keeps the conversation under the pane it left.
        let asleep = self
            .snapshot
            .ui_state
            .agent_sleep
            .records
            .contains_key(pane)
            || self.dormant_of_pane(pane, self.agent_row(pane)).is_some();
        let agent = self.agent_row(pane);
        WorkerProbe {
            present: agent.is_some(),
            closing: self.panes_closing.contains(pane) || self.factory_closes_sent.contains(pane),
            asleep,
            activity: match agent {
                None => AgentUse::Unknown,
                Some(agent) if crate::agent_state::has_work_in_progress(agent) => AgentUse::Working,
                Some(agent) => AgentUse::of(&agent.demand, agent.blocked, &agent.activity),
            },
            changed_at_unix_ms: agent.and_then(|agent| agent.changed_at_unix_ms),
        }
    }

    /// Owned copies of what a worker's adapter declares it reports, and
    /// the connection its screen is read through; nothing is read here.
    pub(crate) fn factory_worker_texts(&self, pane: &str) -> WorkerTextSources {
        use hide_agent_adapter::Capability;
        let agent = self.agent_row(pane);
        let factory = agent
            .and_then(|agent| hide_agent_adapter::adapter(&agent.agent_kind))
            .map(|row| row.factory);
        let user_turn = factory
            .filter(|f| matches!(f.user_turn, Capability::Available(_)))
            .and(agent)
            .and_then(|agent| agent.user_turn.as_ref())
            .and_then(|turn| turn.content.as_ref())
            .map(|content| {
                let mut text = content.text().to_owned();
                for choice in content.choices() {
                    text.push_str("\n- ");
                    text.push_str(choice);
                }
                text
            });
        let last_answer = factory
            .filter(|f| matches!(f.turn_end_and_answer, Capability::Available(_)))
            .and(agent)
            .and_then(|agent| agent.request.as_ref())
            .and_then(|request| request.reply.as_ref())
            .map(|reply| reply.text.clone());
        WorkerTextSources {
            user_turn,
            last_answer,
            raw_pane: self
                .delivery_observations
                .get(pane)
                .map(|observation| observation.raw_pane_id.clone()),
            connector: self.delivery_connector(
                super::agent_sleep::pane_device(pane).unwrap_or(self.node.as_str()),
            ),
        }
    }

    /// Panes the operator closed in Hide: a Factory worker among them pauses
    /// its Task instead of being started again (D-26).
    pub(crate) fn factory_panes_closed(&mut self, panes: &[String]) {
        if panes.is_empty() {
            return;
        }
        let Some(port) = &self.factory_screen else {
            return;
        };
        // The panes wait here until the engine takes them, so a tick that
        // reads a worker after the close guard cleared still sees it
        // closing, and a full queue loses no close: the engine also takes
        // them before every tick.
        if port.panes_closed() {
            self.factory_closes_sent.extend(panes.iter().cloned());
        } else {
            crate::diagnostic!(serde_json::json!({
                "component": "factory",
                "kind": "worker.close_unsent",
                "panes": panes,
                "reason": "factory_unavailable",
            }));
        }
    }

    /// The closes the engine has not taken yet, handed over once.
    pub(crate) fn factory_closes_take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.factory_closes_sent)
            .into_iter()
            .collect()
    }

    /// The dormant conversation that holds `pane`'s execution: one saved for
    /// the pane, whose agent is either gone or the same conversation. A later
    /// agent Herdr put in a reused pane id is neither.
    fn dormant_of_pane(
        &self,
        pane: &str,
        agent: Option<&crate::model::SidebarAgentSnapshot>,
    ) -> Option<(&SleepId, &DormantRecord)> {
        self.snapshot
            .ui_state
            .agent_sleep
            .dormant
            .iter()
            .filter(|(_, record)| {
                record.old_pane_id == pane
                    && record.node_id == self.node.as_str()
                    && agent.is_none_or(|agent| {
                        agent.session_id.as_deref() == Some(record.native_session_id.as_str())
                    })
            })
            .max_by_key(|(_, record)| record.since_unix_ms)
    }

    /// The Factory host's view of its workers' panes, published when it
    /// changes. A close-pane sleep that lands for one of them is bound to its
    /// worker (#857).
    pub(crate) fn set_factory_panes(&mut self, panes: HashMap<String, FactoryPane>) {
        let mut ids = panes.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        self.snapshot.navigator.factory_worker_panes = ids;
        self.factory_panes = panes;
    }

    /// Puts a worker to sleep through the agent sleep path (D-14, B27).
    /// `Ok(false)`: not yet, the agent is still in its turn or the core is
    /// saving other sleeping sessions; ask again.
    pub(crate) fn factory_sleep(&mut self, pane: &str) -> Result<bool, &'static str> {
        if self
            .snapshot
            .ui_state
            .agent_sleep
            .records
            .contains_key(pane)
        {
            return Ok(true);
        }
        let device = super::agent_sleep::pane_device(pane);
        let agent = self.agent_row(pane);
        // A close-pane sleep already saved, or landed.
        if self.dormant_of_pane(pane, agent).is_some() {
            return Ok(true);
        }
        let Some(agent) = agent else {
            return Err("This pane has no agent");
        };
        // The Factory sleeps the agents `Runtime::sleeps` names, and the
        // dialect says whether the pane closes with it.
        if !hide_factory::model::Runtime::parse(&agent.agent_kind)
            .is_some_and(|runtime| runtime.sleeps())
        {
            return Err("agent_cannot_sleep");
        }
        let Some(dialect) =
            hide_agent_adapter::adapter(&agent.agent_kind).and_then(|adapter| adapter.sleep)
        else {
            return Err("agent_cannot_sleep");
        };
        // A saved conversation is kept by the core that owns the pane.
        if dialect.closes_pane_when_sleeping() && device.is_some() {
            return Err("agent_cannot_sleep");
        }
        match crate::agent_sleep::sleep_refusal(agent, self.sleep_machine_of_pane(pane)) {
            None => {}
            Some("This agent is working") | Some("Hide cannot tell what this agent is doing") => {
                return Ok(false);
            }
            Some(reason) => return Err(reason),
        }
        let reachable = match device {
            None => self.live.is_some(),
            Some(device) => self.remote_herdr_api(device).is_some(),
        };
        if !reachable {
            return Err("Putting an agent to sleep requires a live Herdr connection");
        }
        if dialect.closes_pane_when_sleeping() {
            return match self.admit_dormant_sleep(pane, super::unix_milliseconds()) {
                Ok(_) => Ok(true),
                Err(refusal) if refusal.transient => Ok(false),
                Err(refusal) => Err(refusal.message),
            };
        }
        self.request_agent_sleep(pane);
        Ok(true)
    }

    /// Refuses to start a replacement conversation for a worker whose own is
    /// kept under the pane it left: the wake brings that one back.
    pub(crate) fn factory_worker_resume_allowed(
        &self,
        worker: &hide_factory::model::WorkerRef,
    ) -> Result<(), &'static str> {
        let Some(pane) = worker.pane.as_deref() else {
            return Ok(());
        };
        let agent = self
            .snapshot
            .navigator
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane);
        if self.dormant_of_pane(pane, agent).is_some() {
            return Err("worker_dormant");
        }
        Ok(())
    }

    /// Wakes a sleeping worker: in its own pane and session, or, when its
    /// pane closed with the sleep, in the fresh pane the core finds for the
    /// saved conversation (B27, B54, #857).
    pub(crate) fn factory_wake(&mut self, worker: &hide_factory::model::WorkerRef) -> FactoryWake {
        let Some(pane) = worker.pane.as_deref() else {
            return FactoryWake::Awake;
        };
        if self
            .snapshot
            .ui_state
            .agent_sleep
            .records
            .contains_key(pane)
        {
            self.request_agent_wake(super::agent_sleep::AgentWakePayload {
                pane_id: pane.to_owned(),
                fresh: false,
            });
            return FactoryWake::Asked;
        }
        let agent = self
            .snapshot
            .navigator
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane);
        let Some((id, record)) = self
            .dormant_of_pane(pane, agent)
            .filter(|(_, record)| self.dormant_belongs_to(record, worker))
            .map(|(id, record)| (id.clone(), record.clone()))
        else {
            return FactoryWake::Awake;
        };
        match record.phase {
            // The close has not landed: the wake is asked once it has.
            DormantPhase::SavingClose
            | DormantPhase::SavingCloseReady
            | DormantPhase::Closing
            | DormantPhase::CloseUnknown => FactoryWake::Later,
            DormantPhase::SavingWake
            | DormantPhase::Creating
            | DormantPhase::SavingStart
            | DormantPhase::Starting
            | DormantPhase::WakeUnknown
            | DormantPhase::Woken => FactoryWake::Asked,
            DormantPhase::Sleeping | DormantPhase::Failed => {
                if worker.factory.is_empty() {
                    return FactoryWake::Refused("worker_unbound");
                }
                let bound = FactoryWorker {
                    factory: worker.factory.clone(),
                    agent: worker.agent.clone(),
                    name: worker.name.clone(),
                    worktree: worker.worktree.clone(),
                };
                match self.begin_dormant_wake(&id, Some(bound)) {
                    Ok(_) => FactoryWake::Asked,
                    Err(refusal) if refusal.transient => FactoryWake::Later,
                    Err(refusal) => FactoryWake::Refused(refusal.message),
                }
            }
        }
    }

    /// Whether this saved conversation is `worker`'s: bound to it when the
    /// sleep landed, or, for one saved without the Factory knowing, saved
    /// from the pane and kind the worker started after.
    fn dormant_belongs_to(
        &self,
        record: &DormantRecord,
        worker: &hide_factory::model::WorkerRef,
    ) -> bool {
        match &record.factory {
            Some(bound) => bound.factory == worker.factory && bound.agent == worker.agent,
            None => {
                hide_agent_adapter::canonical_kind(&record.kind)
                    == hide_agent_adapter::canonical_kind(worker.runtime.as_str())
                    && record.since_unix_ms >= worker.started_at
            }
        }
    }

    /// The Factory workers whose conversation the core kept and has news of:
    /// woken in a pane to bind, or not woken at all.
    pub(crate) fn factory_dormant_workers(&self) -> Vec<DormantWorker> {
        self.snapshot
            .ui_state
            .agent_sleep
            .dormant
            .iter()
            .filter(|(_, record)| record.node_id == self.node.as_str())
            .filter_map(|(id, record)| {
                let state = match (record.phase, record.wake_pane_id.as_ref()) {
                    (DormantPhase::Woken, Some(pane)) => DormantState::Woken {
                        pane: pane.clone(),
                        continued: self.registration_continued_in(record, pane),
                    },
                    (DormantPhase::Failed, _) if record.closed => {
                        DormantState::WakeFailed("wake_failed")
                    }
                    (DormantPhase::WakeUnknown, _) => DormantState::WakeFailed("wake_unconfirmed"),
                    _ => return None,
                };
                Some(DormantWorker {
                    id: id.clone(),
                    worker: record.factory.clone()?,
                    old_pane: record.old_pane_id.clone(),
                    kind: record.kind.clone(),
                    state,
                })
            })
            .collect()
    }

    /// Whether the registration the sleeping session held now belongs to the
    /// pane it woke in, as the delivery store moved it.
    fn registration_continued_in(&self, record: &DormantRecord, pane: &str) -> bool {
        let (Some(registration), Some(observed), Ok(ledger)) = (
            record.registration.as_ref(),
            self.delivery_observations.get(pane),
            self.delivery_ledger.as_ref(),
        ) else {
            return false;
        };
        ledger.agents.iter().any(|agent| {
            agent.id == registration.id
                && !agent.ended
                && agent.actor.same_identity(&observed.actor)
        })
    }

    /// The Factory has bound a woken worker's pane: the way back is not
    /// needed any more.
    pub(crate) fn factory_dormant_release(&mut self, id: &SleepId) {
        let store = &mut self.snapshot.ui_state.agent_sleep;
        if store
            .dormant
            .get(id)
            .is_some_and(|record| record.phase == DormantPhase::Woken)
        {
            store.dormant.remove(id);
            self.persist_ui_state();
        }
    }

    /// A local issue for a Factory Task, or the one already carrying its
    /// marker (B13, B73), in the store of its project's machine (`machine`,
    /// the Factory's node; `None` or this core's own id is this machine).
    pub(crate) fn factory_local_issue(
        &mut self,
        machine: Option<&str>,
        project: &str,
        title: &str,
        body: &str,
        marker: &str,
    ) -> Result<u32, String> {
        let machine = machine.filter(|node| *node != self.node.as_str());
        if let Ok(store) = &self.local_issues
            && let Some(existing) = store.project(machine, project).and_then(|project| {
                project
                    .issues
                    .iter()
                    .find(|issue| issue.body.contains(marker))
            })
        {
            return Ok(existing.number);
        }
        self.create_local_issue(machine, project, title, body)
    }

    pub(crate) fn factory_local_issue_read(
        &self,
        machine: Option<&str>,
        project: &str,
        number: u32,
    ) -> Option<crate::local_issues::LocalIssue> {
        let machine = machine.filter(|node| *node != self.node.as_str());
        self.local_issues
            .as_ref()
            .ok()
            .and_then(|store| store.issue(machine, project, number))
            .cloned()
    }

    /// Whether a start of `kind` may go: this Mac's kit has answered since
    /// launch, or the start does not need it (`codex_launch`). When not,
    /// asks the kit to read the machine; the Factory starts workers with no
    /// Settings on screen.
    pub(crate) fn factory_kit_read(&mut self, node: Option<&str>, kind: &str) -> bool {
        let device = node.unwrap_or(self.node.as_str());
        if !crate::codex_launch::needs_kit_answer(kind) || self.kit_states.contains_key(device) {
            return true;
        }
        self.request_kit_check();
        false
    }

    /// When `provider`'s usage limit resets, while the usage rows show a
    /// window used up (B58).
    pub(crate) fn factory_usage_limit(&self, provider: &str, now_unix_ms: u64) -> Option<u64> {
        usage_limit(&self.snapshot.usage, provider, now_unix_ms)
    }

    pub(crate) fn factory_ai_settings(&self) -> Option<hide_ai::AiSettings> {
        self.ai_settings.clone()
    }
}

/// The latest reset of a used-up window of `provider` still ahead of now,
/// in milliseconds: the main row or any bucket at 100 percent.
fn usage_limit(
    usage: &crate::usage::model::UsageSnapshot,
    provider: &str,
    now_unix_ms: u64,
) -> Option<u64> {
    usage
        .exhausted_until(provider, now_unix_ms / 1000)
        .map(|seconds| seconds.saturating_mul(1000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::AgentRecord;
    use crate::delivery::ledger::Ledger;
    use crate::model::SidebarAgentSnapshot;
    use crate::usage::model::*;
    use std::sync::Arc;

    fn agent(id: &str, pane: &str, parent: Option<&str>) -> AgentRecord {
        AgentRecord {
            id: id.into(),
            name: Some(id.into()),
            machine: crate::node::TEST_NODE.into(),
            host_scope: "fixture".into(),
            native_machine: "fixture-machine".into(),
            session: id.into(),
            instance: pane.into(),
            pane: pane.into(),
            parent: parent.map(str::to_owned),
            origin: None,
            project: None,
            actor: match pane.strip_prefix("factory:") {
                Some(factory) => Actor::factory(factory, crate::node::TEST_NODE),
                None => Actor {
                    pane_id: pane.into(),
                    name: id.into(),
                    kind: "claude".into(),
                    device_id: crate::node::TEST_NODE.into(),
                    session: crate::wire::session_digest(id),
                },
            },
            ended: false,
            forked: false,
        }
    }

    #[test]
    fn a_question_requires_the_current_spawn_session_even_when_its_ledger_record_ended() {
        let root = tempfile::tempdir().unwrap();
        let herdr = crate::fake_herdr::FakeHerdr::start("question-worker", |method, _| {
            panic!("in-memory authority checks must not call {method}")
        });
        let (runtime, _, _, _) = crate::runtime::delivery::tests::fixture(root.path());
        let mut runtime = runtime.lock().unwrap();
        let observed =
            crate::runtime::delivery::tests::recipient_at_rest(&mut runtime, "native-1", &herdr);
        let context = crate::runtime::delivery::tests::authority(&observed.actor).context;
        let native = runtime
            .coordination_context(crate::node::TEST_NODE)
            .unwrap();
        let mut accepted = agent("accepted-worker", "recipient", Some("factory-parent"));
        accepted.actor = observed.actor.clone();
        accepted.host_scope = native.host_scope;
        accepted.native_machine = native.machine;
        accepted.ended = true;
        runtime.delivery_ledger = Ok(Arc::new(Ledger {
            agents: vec![agent("factory-parent", "factory:f-1", None), accepted],
            ..Ledger::default()
        }));
        let worker = hide_factory::model::WorkerRef {
            node: None,
            factory: "f-1".into(),
            agent: Some("accepted-worker".into()),
            name: "worker".into(),
            pane: Some("recipient".into()),
            runtime: hide_factory::model::Runtime::CODEX,
            worktree: "/checkouts/fixture".into(),
            branch: "task".into(),
            started_at: 1,
            asleep: false,
            model: None,
            effort: None,
        };
        let caller = runtime
            .factory_question_caller(
                crate::node::TEST_NODE,
                "recipient",
                &context,
                "native-1",
                "codex",
                "terminal-1",
            )
            .unwrap();
        assert_eq!(
            runtime.factory_question_current(&caller, &worker),
            Ok(()),
            "cancel/revive may retain this accepted native execution with an ended registration"
        );
        let descendant = hide_factory::model::WorkerRef {
            pane: Some("sender".into()),
            ..worker.clone()
        };
        assert!(
            runtime
                .factory_question_current(&caller, &descendant)
                .is_err()
        );
        let missing = hide_factory::model::WorkerRef {
            agent: Some("not-registered".into()),
            ..worker.clone()
        };
        assert!(runtime.factory_question_current(&caller, &missing).is_err());

        crate::runtime::delivery::tests::recipient_at_rest(&mut runtime, "native-2", &herdr);
        assert!(
            runtime.factory_question_current(&caller, &worker).is_err(),
            "reused pane is a different execution"
        );
        assert!(
            runtime
                .factory_question_caller(
                    crate::node::TEST_NODE,
                    "recipient",
                    &context,
                    "native-1",
                    "codex",
                    "terminal-1",
                )
                .is_err(),
            "the old hook's native generation cannot attest the new execution"
        );
        let replaced = runtime
            .factory_question_caller(
                crate::node::TEST_NODE,
                "recipient",
                &context,
                "native-2",
                "codex",
                "terminal-1",
            )
            .unwrap();
        assert!(
            runtime
                .factory_question_current(&replaced, &worker)
                .is_err(),
            "an ended ledger by itself proves nothing about the replacement"
        );
    }

    #[test]
    fn a_question_caller_named_by_a_deep_session_file_is_the_panes_own() {
        // Pi and omp name a session by its file, which a deep checkout makes
        // longer than 256 bytes (PRD pi-omp-extension D-05).
        let root = tempfile::tempdir().unwrap();
        let herdr = crate::fake_herdr::FakeHerdr::start("question-deep-session", |method, _| {
            panic!("in-memory authority checks must not call {method}")
        });
        let (runtime, _, _, _) = crate::runtime::delivery::tests::fixture(root.path());
        let mut runtime = runtime.lock().unwrap();
        let deep = format!(
            "/fixture-home/.omp/agent/sessions/-{}-/2026-10-08T20-05-41-366Z_01a11d1f.jsonl",
            "deep-checkout-segment".repeat(12)
        );
        assert!(deep.len() > 256);
        let observed =
            crate::runtime::delivery::tests::recipient_at_rest(&mut runtime, &deep, &herdr);
        let context = crate::runtime::delivery::tests::authority(&observed.actor).context;
        let caller = |runtime: &mut Runtime, session: &str| {
            runtime.factory_question_caller(
                crate::node::TEST_NODE,
                "recipient",
                &context,
                session,
                "codex",
                "terminal-1",
            )
        };
        assert!(caller(&mut runtime, &deep).is_ok());
        let past = "x".repeat(crate::delivery::SESSION_LIMIT + 1);
        assert!(matches!(caller(&mut runtime, &past),
            Err(reason) if reason == "factory_guard_context_changed"));
    }

    #[test]
    fn foreign_checkout_and_changed_connection_question_callers_are_unproven() {
        let root = tempfile::tempdir().unwrap();
        let herdr = crate::fake_herdr::FakeHerdr::start("question-authority", |method, _| {
            panic!("refused callers must not open a native request: {method}")
        });
        let (runtime, _, _, _) = crate::runtime::delivery::tests::fixture(root.path());
        let mut runtime = runtime.lock().unwrap();
        let observed =
            crate::runtime::delivery::tests::recipient_at_rest(&mut runtime, "native-1", &herdr);
        let context = crate::runtime::delivery::tests::authority(&observed.actor).context;
        assert!(matches!(runtime.factory_question_caller(
            "foreign-device", "recipient", &context, "native-1", "codex", "terminal-1",
        ), Err(reason) if reason == "factory_guard_local_only"));
        let checkout = crate::workspace_control::checkout_caller_id("cap", "/checkouts/fixture");
        assert!(matches!(runtime.factory_question_caller(
            crate::node::TEST_NODE, &checkout, &context, "native-1", "codex", "terminal-1",
        ), Err(reason) if reason == "agent_pane_required"));
        let caller = runtime
            .factory_question_caller(
                crate::node::TEST_NODE,
                "recipient",
                &context,
                "native-1",
                "codex",
                "terminal-1",
            )
            .unwrap();
        crate::runtime::delivery::tests::recipient_at_rest(&mut runtime, "native-1", &herdr);
        let worker = hide_factory::model::WorkerRef {
            node: None,
            factory: "f-1".into(),
            agent: Some("accepted-worker".into()),
            name: "worker".into(),
            pane: Some("recipient".into()),
            runtime: hide_factory::model::Runtime::CODEX,
            worktree: "/checkouts/fixture".into(),
            branch: "task".into(),
            started_at: 1,
            asleep: false,
            model: None,
            effort: None,
        };
        assert_eq!(
            runtime.factory_question_current(&caller, &worker),
            Err("factory_guard_native_changed".into()),
            "the same native id on a replaced connection cannot reuse its proof"
        );
    }

    #[test]
    fn a_caller_s_lineage_names_every_agent_above_it_and_stops_at_a_loop() {
        let mut runtime = crate::runtime::tests::runtime();
        runtime.delivery_ledger = Ok(Arc::new(Ledger {
            agents: vec![
                agent("agent-factory", "factory:f-1", None),
                agent("agent-worker", "w1:p1", Some("agent-factory")),
                agent("agent-child", "w2:p1", Some("agent-worker")),
                agent("agent-grandchild", "w3:p1", Some("agent-child")),
                agent("agent-a", "w9:p1", Some("agent-b")),
                agent("agent-b", "w9:p2", Some("agent-a")),
            ],
            ..Ledger::default()
        }));
        let full = runtime.factory_lineage("w3:p1");
        assert_eq!(
            full.agents,
            vec!["agent-child", "agent-worker", "agent-factory"]
        );
        assert_eq!(full.panes, vec!["w2:p1", "w1:p1", "factory:f-1"]);
        assert!(full.complete && full.factory_spawned);
        let none = runtime.factory_lineage("w0:p1");
        assert!(none.agents.is_empty() && none.complete);
        let looped = runtime.factory_lineage("w9:p1");
        assert_eq!(
            looped.agents,
            vec!["agent-b", "agent-a"],
            "a loop ends the walk"
        );
        assert!(!looped.complete, "a loop cannot rule out a worker above");
        assert!(!looped.factory_spawned);
    }

    #[test]
    fn a_checkout_bound_caller_is_the_operator_and_its_hint_is_ignored() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, actor, _, _) = crate::runtime::delivery::tests::fixture(root.path());
        let context = crate::runtime::delivery::tests::authority(&actor).context;
        let mut runtime = runtime.lock().unwrap();
        // The hinted pane is a worker's child with a lineage of its own; a
        // checkout caller's hint is not read, so none of it applies.
        runtime.delivery_ledger = Ok(Arc::new(Ledger {
            agents: vec![
                agent("agent-worker", "w1:p1", None),
                agent("agent-sender", "sender", Some("agent-worker")),
            ],
            ..Ledger::default()
        }));
        let checkout = crate::workspace_control::checkout_caller_id("cap", "/checkouts/fixture");
        for hint in [None, Some("sender"), Some("w1:p1")] {
            let caller = runtime
                .factory_caller(crate::node::TEST_NODE, &checkout, &context, hint)
                .unwrap();
            assert_eq!(caller.pane, None, "hint {hint:?}");
            assert_eq!(caller.claimed, None, "hint {hint:?}");
            assert_eq!(
                caller.ancestors,
                crate::factory::Lineage::none(),
                "hint {hint:?}"
            );
            assert_eq!(caller.cwd.as_deref(), Some("/checkouts/fixture"));
        }
        // A pane caller keeps its own pane and lineage; a hint naming another
        // pane can only make it that pane's worker.
        let own = runtime
            .factory_caller(crate::node::TEST_NODE, "sender", &context, Some("sender"))
            .unwrap();
        assert_eq!(own.pane.as_deref(), Some("sender"));
        assert_eq!(own.claimed, None);
        assert_eq!(own.ancestors.agents, vec!["agent-worker"]);
        let other = runtime
            .factory_caller(crate::node::TEST_NODE, "sender", &context, Some("w1:p1"))
            .unwrap();
        assert_eq!(other.pane.as_deref(), Some("sender"));
        assert_eq!(other.claimed.as_deref(), Some("w1:p1"));
    }

    #[test]
    fn an_ended_agent_still_walks_up_but_lends_no_pane() {
        let mut runtime = crate::runtime::tests::runtime();
        let ended = |mut record: AgentRecord| {
            record.ended = true;
            record
        };
        runtime.delivery_ledger = Ok(Arc::new(Ledger {
            agents: vec![
                agent("agent-factory", "factory:f-1", None),
                ended(agent("agent-worker", "w1:p1", Some("agent-factory"))),
                ended(agent("agent-child", "w2:p1", Some("agent-worker"))),
                agent("agent-grandchild", "w3:p1", Some("agent-child")),
            ],
            ..Ledger::default()
        }));
        // An agent that ended its own record still walks up to its worker.
        let ended_child = runtime.factory_lineage("w2:p1");
        assert_eq!(ended_child.agents, vec!["agent-worker", "agent-factory"]);
        assert!(ended_child.complete && ended_child.factory_spawned);
        // Ended ancestors stay in the walk but lend no pane to bind by.
        let live = runtime.factory_lineage("w3:p1");
        assert_eq!(
            live.agents,
            vec!["agent-child", "agent-worker", "agent-factory"]
        );
        assert_eq!(live.panes, vec!["factory:f-1"]);
    }

    #[test]
    fn registering_again_without_a_parent_keeps_an_agent_below_its_worker() {
        let mut runtime = crate::runtime::tests::runtime();
        let mut child = agent("agent-child", "w2:p1", Some("agent-worker"));
        child.ended = true;
        // The same agent, on the same pane and session, registered again
        // with no parent after ending its first record.
        let again = AgentRecord {
            id: "agent-child-again".into(),
            parent: None,
            ..child.clone()
        };
        // A different agent later on a reused pane id is its own.
        let mut reused = agent("agent-other", "w3:p1", None);
        let mut gone = agent("agent-gone", "w3:p1", Some("agent-worker"));
        gone.ended = true;
        reused.ended = false;
        runtime.delivery_ledger = Ok(Arc::new(Ledger {
            agents: vec![
                agent("agent-factory", "factory:f-1", None),
                agent("agent-worker", "w1:p1", Some("agent-factory")),
                child,
                again,
                gone,
                reused,
            ],
            ..Ledger::default()
        }));
        let walked = runtime.factory_lineage("w2:p1");
        assert_eq!(walked.agents, vec!["agent-worker", "agent-factory"]);
        assert!(walked.factory_spawned);
        let other = runtime.factory_lineage("w3:p1");
        assert!(other.agents.is_empty(), "{:?}", other.agents);
        assert!(!other.factory_spawned);
    }

    fn row(provider: &str, used: f64, five_hour: Option<(f64, u64)>) -> AgentUsage {
        let mut windows = vec![WindowReading {
            kind: WindowKind::Weekly,
            scope: Scope::All,
            outcome: WindowOutcome::Open {
                used_percent: used,
                resets_at_s: 5000,
            },
        }];
        if let Some((used_percent, resets_at_s)) = five_hour {
            windows.push(WindowReading {
                kind: WindowKind::FiveHour,
                scope: Scope::All,
                outcome: WindowOutcome::Open {
                    used_percent,
                    resets_at_s,
                },
            });
        }
        AgentUsage {
            agent: provider.into(),
            limits: Cell::Reading {
                value: LimitReading { windows },
                freshness: Freshness::Fresh,
                observed_at_ms: 1,
            },
            consumption: Cell::Unsupported,
        }
    }

    #[test]
    fn a_used_up_window_limits_its_runtime_until_its_reset() {
        let rows = UsageSnapshot {
            nodes: vec![NodeUsage {
                node: crate::node::TEST_NODE.into(),
                history: None,
                agents: vec![
                    row("claude", 40.0, Some((100.0, 2_000))),
                    row("codex", 99.0, None),
                ],
            }],
        };
        assert_eq!(usage_limit(&rows, "claude", 1_000_000), Some(2_000_000));
        assert_eq!(
            usage_limit(&rows, "claude", 2_000_000),
            None,
            "the reset passed"
        );
        assert_eq!(
            usage_limit(&rows, "codex", 1_000_000),
            None,
            "99 percent still runs"
        );
        assert_eq!(
            usage_limit(&UsageSnapshot { nodes: Vec::new() }, "claude", 0),
            None
        );
    }

    /// A worker row with the status model's axes set directly.
    fn worker_row(pane: &str, kind: &str, demand: &str, activity: &str) -> SidebarAgentSnapshot {
        let mut row = crate::sidebar::project_agents(
            crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [
                {"id": pane, "pane_id": pane, "agent_status": "idle", "state_change_seq": 1}
            ]}))
            .unwrap(),
        )
        .agents
        .remove(0);
        row.agent_kind = kind.into();
        row.demand = demand.into();
        row.activity = activity.into();
        row.blocked = false;
        row.group = "seen".into();
        row.changed_at_unix_ms = Some(5_000);
        row
    }

    #[test]
    fn a_worker_rests_only_when_its_row_says_stopped() {
        use crate::agent_state::AgentUse;
        let mut runtime = crate::runtime::tests::runtime();
        runtime.snapshot.navigator.agents = vec![
            worker_row("quiet", "claude", "none", "stopped"),
            worker_row("unsure", "claude", "none", "unknown"),
            worker_row("asking", "claude", "question", "stopped"),
            worker_row("busy", "claude", "none", "working"),
        ];
        let probe = runtime.factory_worker_probe("quiet");
        assert_eq!(
            (probe.present, probe.activity, probe.changed_at_unix_ms),
            (true, AgentUse::Quiet, Some(5_000))
        );
        assert_eq!(
            runtime.factory_worker_probe("unsure").activity,
            AgentUse::Unknown,
            "what Hide cannot tell is never rest (D-52)"
        );
        assert_eq!(
            runtime.factory_worker_probe("asking").activity,
            AgentUse::Waiting
        );
        assert_eq!(
            runtime.factory_worker_probe("busy").activity,
            AgentUse::Working
        );
        let gone = runtime.factory_worker_probe("gone");
        assert_eq!((gone.present, gone.activity), (false, AgentUse::Unknown));
        assert!(!gone.closing);
        runtime.panes_closing.insert("quiet".into());
        assert!(runtime.factory_worker_probe("quiet").closing);
    }

    #[test]
    fn factory_sleep_refuses_an_agent_with_no_adapter_without_touching_its_row() {
        let mut runtime = crate::runtime::tests::live_runtime();
        let mut row = worker_row("worker", "unknown-agent", "none", "stopped");
        row.session_id = Some("11111111-2222-3333-4444-555555555555".into());
        runtime.snapshot.navigator.agents = vec![row];
        let before = serde_json::to_value(runtime.snapshot()).unwrap();
        assert_eq!(runtime.factory_sleep("worker"), Err("agent_cannot_sleep"));
        assert_eq!(serde_json::to_value(runtime.snapshot()).unwrap(), before);
        assert!(runtime.snapshot.ui_state.agent_sleep.dormant.is_empty());
        assert!(runtime.snapshot.ui_state.agent_sleep.records.is_empty());
    }

    #[test]
    fn factory_sleep_still_uses_the_same_pane_for_claude_and_codex() {
        for kind in ["claude", "codex"] {
            let mut runtime = crate::runtime::tests::live_runtime();
            let mut row = worker_row("worker", kind, "none", "stopped");
            row.session_id = Some("11111111-2222-3333-4444-555555555555".into());
            runtime.snapshot.navigator.agents = vec![row];
            assert_eq!(runtime.factory_sleep("worker"), Ok(true));
            let record = &runtime.snapshot.ui_state.agent_sleep.records["worker"];
            assert_eq!(record.kind, kind);
            assert_eq!(record.phase, crate::agent_sleep::SleepPhase::Ending);
            assert!(runtime.snapshot.ui_state.agent_sleep.dormant.is_empty());
            assert!(runtime.panes_closing.is_empty());
        }
    }

    #[test]
    fn a_diagnosis_reads_only_what_the_worker_s_adapter_declares() {
        let mut runtime = crate::runtime::tests::runtime();
        let mut claude = worker_row("claude-pane", "claude", "question", "stopped");
        claude.user_turn = Some(hide_session::turns::UserTurnFact {
            kind: hide_session::turns::UserTurnKind::Question,
            content: Some(hide_session::turns::UserTurnContent::new(
                "Blue or green?",
                ["blue", "green"],
            )),
        });
        let mut unknown = claude.clone();
        unknown.pane_id = "other-pane".into();
        unknown.agent_kind = "some-new-agent".into();
        runtime.snapshot.navigator.agents = vec![claude, unknown];
        let texts = runtime.factory_worker_texts("claude-pane");
        assert_eq!(
            texts.user_turn.as_deref(),
            Some("Blue or green?\n- blue\n- green")
        );
        assert_eq!(
            runtime.factory_worker_texts("other-pane").user_turn,
            None,
            "an agent with no adapter declares nothing"
        );
    }

    #[test]
    fn only_a_start_that_takes_the_codex_flag_waits_for_the_kit() {
        assert!(crate::codex_launch::needs_kit_answer("codex"));
        assert!(!crate::codex_launch::needs_kit_answer("claude"));
        assert!(!crate::codex_launch::needs_kit_answer("unknown-agent"));
    }
}
