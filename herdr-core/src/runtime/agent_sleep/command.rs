//! `hide agent sleep <id> [--actor <id>]` (PRD mission-worker-sleep D-04,
//! D-20): the pane menu's Sleep agent for a registered agent, asked by the
//! agent itself or by its parent, as `hide agent end` is. It runs under the
//! lock like a mission command, because the end it starts is the runtime's.

use serde_json::{Value, json};

use super::super::Runtime;
use super::letters::Stage;
use super::SleepRefused;
use crate::delivery::Actor;
use crate::model::AgentActionRefusal;
use crate::workspace_control::{Caller, Context, Query};

impl Runtime {
    /// Who `caller` is and the registration `id` it may act on, for a command
    /// that acts on another agent's pane (`hide agent sleep`, `hide agent
    /// archive`): the registration's actor, or why not.
    ///
    /// A pane's command may act on the agent itself or on its child, as `hide
    /// agent end` does. Desk (`desk_may_act`) has no pane: it acts as Hide's
    /// own `hide:desk` while it runs, on any live registration, because the
    /// operator approves each such command on its card before Desk runs it.
    pub(in crate::runtime) fn registered_agent_for(
        &self,
        device: &str,
        caller: &str,
        expected: &Context,
        id: &str,
        acting: Option<&str>,
        desk_may_act: bool,
    ) -> Result<Actor, String> {
        let desk = desk_may_act && Caller::parse(caller) == Caller::Desk;
        let desk_authority = desk.then(|| self.desk_authority(device)).transpose()?;
        let context = match &desk_authority {
            Some((context, _)) => context.clone(),
            None => {
                self.workspace_control_query(device, caller, Query::Info)
                    .map_err(|refusal| refusal.reason.to_owned())?
                    .context
            }
        };
        if context != *expected {
            return Err("caller_context_changed".into());
        }
        let me = match desk_authority {
            Some((_, actor)) => actor,
            None => {
                let Caller::Pane(pane) = Caller::parse(caller) else {
                    return Err("agent_pane_required".into());
                };
                let pane = crate::address::named_pane_id(self.node.as_str(), device, pane);
                self.delivery_observations
                    .get(&pane)
                    .ok_or("agent_pane_required")?
                    .actor
                    .clone()
            }
        };
        me.require_native_identity()?;
        let ledger = self.delivery_state()?;
        let record = ledger
            .agents
            .iter()
            .find(|record| record.id == id)
            .ok_or("agent_unavailable")?;
        // The same authority as `hide agent end`: the agent itself or its
        // parent (D-20).
        let parent = record
            .parent
            .as_deref()
            .and_then(|parent| crate::coordination::resolve_actor(&ledger, parent));
        if !desk
            && !record.actor.same_identity(&me)
            && !parent.is_some_and(|parent| parent.same_identity(&me))
        {
            return Err("parent_authority_required".into());
        }
        if acting.is_some_and(|acting| {
            !crate::coordination::resolve_actor(&ledger, acting)
                .is_some_and(|actor| actor.same_identity(&me))
        }) {
            return Err("actor_identity_conflict".into());
        }
        if record.ended {
            return Err("agent_ended".into());
        }
        Ok(record.actor.clone())
    }

    /// Puts registration `id` to sleep for the agent in `caller`'s pane, or
    /// says why not; asking again while it ends or sleeps answers the same
    /// (B14, B15). Returns the answer and whether anything changed.
    pub(crate) fn agent_sleep_command(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        id: &str,
        acting: Option<&str>,
    ) -> Result<(Value, bool), String> {
        let actor = self.registered_agent_for(device, caller, expected, id, acting, false)?;
        let answer = |state: &str| json!({"id": id, "state": state});
        match self.asleep_route(&actor).map(|(_, stage)| stage) {
            Some(Stage::Ending) => return Ok((answer("ending"), false)),
            Some(Stage::Sleeping | Stage::Failed(_)) => return Ok((answer("asleep"), false)),
            Some(Stage::Waking) => return Err("agent_sleep_waking".into()),
            None => {}
        }
        let target = &actor.pane_id;
        let device = crate::address::pane_device(target);
        let row = self
            .sleep_agents(device)
            .iter()
            .find(|agent| &agent.pane_id == target)
            .ok_or("agent_unavailable")?;
        // The pane hosts another conversation than the registration's.
        if row
            .session_id
            .as_deref()
            .and_then(crate::engine::tokens::session_digest)
            != actor.session
        {
            return Err("agent_unavailable".into());
        }
        if self.agent_sleep_on_screen(device).contains(target) {
            return Err("agent_sleep_on_screen".into());
        }
        let changed = self
            .sleep_pane(target, "command")
            .map_err(|refused| refusal_code(refused).to_owned())?;
        match self.asleep_route(&actor).map(|(_, stage)| stage) {
            Some(Stage::Ending) => Ok((answer("ending"), true)),
            Some(Stage::Sleeping | Stage::Failed(_)) => Ok((answer("asleep"), true)),
            _ => Err(if changed {
                "agent_sleep_not_started"
            } else {
                "agent_sleep_busy"
            }
            .into()),
        }
    }
}

fn refusal_code(refused: SleepRefused) -> &'static str {
    match refused {
        SleepRefused::NoAgent => "agent_unavailable",
        SleepRefused::Unreachable => "agent_sleep_unreachable",
        SleepRefused::Agent(refusal) => match refusal {
            AgentActionRefusal::Device => "agent_sleep_device",
            AgentActionRefusal::Unsupported => "agent_sleep_unsupported",
            AgentActionRefusal::CoreMachineOnly => "agent_sleep_core_machine_only",
            // Never reached: an asleep agent is answered before the check.
            AgentActionRefusal::Asleep => "agent_sleep_waking",
            AgentActionRefusal::Working => "agent_sleep_working",
            AgentActionRefusal::WaitingForYou => "agent_sleep_waiting_for_you",
            AgentActionRefusal::UnknownActivity => "agent_sleep_activity_unknown",
            AgentActionRefusal::NoConversation => "agent_sleep_no_conversation",
        },
    }
}
