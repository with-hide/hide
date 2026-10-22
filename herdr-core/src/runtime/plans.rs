//! The runtime half of a session's own plan (PRD hide-plan): the `hide
//! plan` commands, each applied under the lock as one change and handed to
//! the link worker to save; the plans the shell draws; the review a
//! proposed plan waits on, laid on the agent's row as a question so it
//! reaches the band, the sidebar and the bell by the ask's own path (D-12,
//! D-30); and the summary the hook keeps of it (D-14, D-15).

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;
use serde_json::{Value, json};

use super::Runtime;
use crate::delivery::Actor;
use crate::links::ledger::{LedgerAnswer, LedgerCall, LedgerUpdate, SessionIdentity};
use crate::model::{AskSubject, SidebarAgentSnapshot};
use crate::plans::{self, PLAN_LIMIT, Plan, PlanCommand, PlanView, Review, view_of};
use crate::workspace_control::{Caller, Context, Query};
use hide_agent_hooks::plan_record::{
    ConductorWork, MissionWork, PlanContext, PlanSummary, StepName,
};

/// Whether the saved plans have been read into the core.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Load {
    #[default]
    Idle,
    Loading,
    Ready,
}

#[derive(Default)]
pub(crate) struct PlanRuntime {
    pub(crate) book: BTreeMap<SessionIdentity, Plan>,
    load: Load,
    /// Moves on every change to the book, so the published view is made
    /// again only when it or the panes moved.
    pub(crate) edit: u64,
    /// The book's edit and the panes' fingerprint the view was made at.
    made_at: Option<(u64, u64)>,
    /// The review questions laid on the rows shown, by pane, so a review
    /// that moved is lifted from those rows without waiting for Herdr to
    /// send them again. Bounded by the rows shown.
    laid: BTreeMap<String, Laid>,
    /// The book's edit the rows shown were last laid at.
    laid_at: u64,
}

/// What laying a review changed on a row: the question it put there, what
/// it asks, and the demand the row had before.
struct Laid {
    turn: hide_session::turns::UserTurnFact,
    subject: AskSubject,
    demand: String,
}

/// The plans the shell draws, one per pane whose session has one (B7).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PlansSnapshot {
    pub items: Vec<PlanView>,
}

/// Whether a row's pane id names `pane`, which a registration may hold
/// raw (`w1:p2`) and a device's rows hold qualified
/// (`remote:<device>:pane:w1:p2`).
fn same_pane(row: &str, pane: &str) -> bool {
    let raw = |id: &str| {
        id.rsplit_once(":pane:")
            .map_or(id, |(_, raw)| raw)
            .to_owned()
    };
    row == pane || raw(row) == raw(pane)
}

/// The identity a row's session is saved under.
fn row_session(device: &str, agent: &SidebarAgentSnapshot) -> Option<SessionIdentity> {
    Some(SessionIdentity {
        device_id: device.to_owned(),
        agent: hide_session::Agent::from_kind(&agent.agent_kind)?
            .as_str()
            .to_owned(),
        session_id: agent.session_id.clone()?,
    })
}

/// What a plan that waits for the operator's review asks on its row; the
/// band, the second line, the bell and the phone each word it (B14).
pub(crate) fn review_subject(plan: &Plan) -> AskSubject {
    AskSubject::PlanReview {
        steps: plans::progress(plan).total,
    }
}

fn step_name(plan: &Plan, key: Option<&str>) -> Option<StepName> {
    let step = plan
        .steps
        .iter()
        .find(|step| Some(step.key.as_str()) == key)?;
    Some(StepName {
        key: step.key.clone(),
        title: step.title.clone(),
    })
}

/// The summary the hook keeps of `plan` (D-14).
pub(crate) fn summary_of(plan: &Plan) -> PlanSummary {
    let progress = plans::progress(plan);
    PlanSummary {
        review: match plan.review {
            Review::Approved => "approved",
            Review::Proposed => "proposed",
        }
        .to_owned(),
        current: step_name(plan, progress.current.as_deref()),
        next: step_name(plan, progress.next.as_deref()),
        done: progress.done,
        total: progress.total,
        finished: progress.finished,
        updated_at_unix_ms: plan.updated_at_unix_ms,
    }
}

impl Runtime {
    /// Reads the saved plans into the core, once the link worker is there,
    /// and again after a read that failed when a command needs them.
    pub(super) fn restore_plans(&mut self) {
        if self.plans.load != Load::Idle {
            return;
        }
        let Some(client) = self.links_work.client.clone() else {
            return;
        };
        self.plans.load = Load::Loading;
        let spawned = self.spawn_issue_worker(
            "plans-restore",
            move || client.call(LedgerCall::Plans),
            |runtime, answer| runtime.ingest_plans(answer),
        );
        if let Err(code) = spawned {
            self.plans.load = Load::Idle;
            crate::diagnostic!(
                json!({"component":"plans","kind":"plans.restore_failed","code":code})
            );
        }
    }

    fn ingest_plans(&mut self, answer: Result<LedgerAnswer, String>) -> bool {
        match answer {
            Ok(LedgerAnswer::Plans(saved)) => {
                // A plan set before the read answered is newer than the
                // saved one.
                for plan in saved.into_iter().rev().take(PLAN_LIMIT) {
                    self.plans.book.entry(plan.session.clone()).or_insert(plan);
                }
                self.plans.load = Load::Ready;
                self.plans.edit += 1;
                true
            }
            Ok(_) => {
                self.plans.load = Load::Idle;
                false
            }
            Err(code) => {
                self.plans.load = Load::Idle;
                crate::diagnostic!(
                    json!({"component":"plans","kind":"plans.restore_failed","code":code})
                );
                false
            }
        }
    }

    /// The pane's own session, as the plan is keyed: the row's, or the
    /// caller's native identity while the row lags it.
    fn caller_session(&self, device: &str, pane: &str, actor: &Actor) -> Option<SessionIdentity> {
        self.machine_agents(device)
            .iter()
            .find(|agent| same_pane(&agent.pane_id, pane))
            .and_then(|agent| row_session(device, agent))
            .or_else(|| {
                Some(SessionIdentity {
                    device_id: device.to_owned(),
                    agent: hide_session::Agent::from_kind(&actor.kind)?
                        .as_str()
                        .to_owned(),
                    session_id: actor.session.clone()?,
                })
            })
    }

    /// The registration `key` names, as a pane on its machine and the
    /// session that pane runs.
    fn agent_session(&self, key: &str) -> Option<(Actor, String, SessionIdentity)> {
        let ledger = self.delivery_state().ok()?;
        let record = ledger.agents.iter().rev().find(|record| {
            !record.ended && (record.id == key || record.name.as_deref() == Some(key))
        })?;
        let pane = record.actor.pane_id.clone();
        let session = self.caller_session(&record.machine, &pane, &record.actor)?;
        Some((record.actor.clone(), pane, session))
    }

    /// Every pane on screen whose agent runs a session, by session.
    fn live_sessions(&self) -> Vec<(String, SessionIdentity)> {
        let local = self.snapshot.navigator.agents.iter().filter_map(|agent| {
            Some((
                agent.pane_id.clone(),
                row_session(self.node.as_str(), agent)?,
            ))
        });
        let remote = self.snapshot.status.remote.iter().flat_map(|remote| {
            remote
                .session
                .iter()
                .flat_map(|session| &session.agents)
                .filter_map(|agent| {
                    Some((
                        agent.pane_id.clone(),
                        row_session(&remote.target_id, agent)?,
                    ))
                })
        });
        local.chain(remote).collect()
    }

    /// Saves `plan` (or its absence) for `session` through the link worker.
    fn save_plan(&self, session: &SessionIdentity, plan: Option<&Plan>) -> Result<(), String> {
        let client = self.links_work.client.as_ref().ok_or("plan_unavailable")?;
        client
            .ledger(LedgerUpdate::Plan {
                session: session.clone(),
                plan: plan.cloned().map(Box::new),
            })
            .map_err(|code| {
                crate::diagnostic!(json!({"component":"plans","kind":"plans.save_refused","code":code,"session_id":session.session_id}));
                "plan_unavailable".to_owned()
            })
    }

    /// Room for a new session's plan: the plan updated longest ago whose
    /// session runs in no pane goes, with a diagnostic.
    fn make_room(&mut self) -> Result<(), String> {
        if self.plans.book.len() < PLAN_LIMIT {
            return Ok(());
        }
        let live: HashSet<SessionIdentity> = self
            .live_sessions()
            .into_iter()
            .map(|(_, session)| session)
            .collect();
        let oldest = self
            .plans
            .book
            .values()
            .filter(|plan| !live.contains(&plan.session))
            .min_by_key(|plan| plan.updated_at_unix_ms)
            .map(|plan| plan.session.clone())
            .ok_or("plan_full")?;
        self.save_plan(&oldest, None)?;
        self.plans.book.remove(&oldest);
        crate::diagnostic!(
            json!({"component":"plans","kind":"plans.evicted","session_id":oldest.session_id,"limit":PLAN_LIMIT})
        );
        Ok(())
    }

    /// Runs a `hide plan` command from `caller`'s pane on `device`.
    /// Returns the answer and whether anything changed.
    pub(crate) fn plan_command(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        command: PlanCommand,
    ) -> Result<(Value, bool), String> {
        if !command.valid() {
            return Err("plan_invalid".into());
        }
        let context = self
            .workspace_control_query(device, caller, Query::Info)
            .map_err(|refusal| refusal.reason.to_owned())?
            .context;
        if context != *expected {
            return Err("caller_context_changed".into());
        }
        let Caller::Pane(pane) = Caller::parse(caller) else {
            return Err("agent_pane_required".into());
        };
        let pane = if device == self.node.as_str() || pane.starts_with("remote:") {
            pane.to_owned()
        } else {
            format!("remote:{device}:pane:{pane}")
        };
        let actor = self
            .delivery_observations
            .get(&pane)
            .ok_or("agent_pane_required")?
            .actor
            .clone();
        if self.plans.load != Load::Ready {
            self.restore_plans();
            return Err("plan_unavailable".into());
        }
        let own = self
            .caller_session(device, &pane, &actor)
            .ok_or("plan_session_unknown")?;
        let now = super::memory::now_ms();
        // A named agent other than the caller only reads (D-05, B3).
        let named = |runtime: &Self,
                     agent: &Option<String>|
         -> Result<Option<(String, SessionIdentity)>, String> {
            let Some(key) = agent else {
                return Ok(None);
            };
            match runtime.agent_session(key) {
                Some((named, _, session)) if named.same_identity(&actor) || session == own => {
                    Ok(None)
                }
                Some((_, pane, session)) => Ok(Some((pane, session))),
                None => Err("plan_agent_unknown".into()),
            }
        };
        let (session, changed) = match command {
            PlanCommand::Show { agent } => {
                if let Some((pane, session)) = named(self, &agent)? {
                    let plan = self
                        .plans
                        .book
                        .get(&session)
                        .map(|plan| view_of(plan, &pane));
                    return Ok((json!({"session": session, "plan": plan}), false));
                }
                (own.clone(), false)
            }
            PlanCommand::Approve { agent } | PlanCommand::Drop { agent }
                if named(self, &agent)?.is_some() =>
            {
                return Err("plan_not_yours".into());
            }
            PlanCommand::Set {
                plan,
                ask,
                why,
                reason,
            } => {
                let previous = self.plans.book.get(&own);
                let next = plans::apply_set(
                    previous,
                    &own,
                    &plan,
                    ask,
                    why.as_deref(),
                    reason.as_deref(),
                    now,
                )?;
                if previous.is_none() {
                    self.make_room()?;
                }
                self.save_plan(&own, Some(&next))?;
                self.plans.book.insert(own.clone(), next);
                (own.clone(), true)
            }
            PlanCommand::Node { key, status, line } => {
                let plan = self.plans.book.get(&own).ok_or("plan_missing")?;
                match plans::apply_node(plan, &key, status, line.as_deref(), now)? {
                    Some(next) => {
                        self.save_plan(&own, Some(&next))?;
                        self.plans.book.insert(own.clone(), next);
                        (own.clone(), true)
                    }
                    None => (own.clone(), false),
                }
            }
            PlanCommand::Approve { .. } => {
                let plan = self.plans.book.get(&own).ok_or("plan_missing")?;
                if plan.review == Review::Approved {
                    (own.clone(), false)
                } else {
                    let mut next = plan.clone();
                    next.review = Review::Approved;
                    next.ask = None;
                    next.updated_at_unix_ms = now;
                    self.save_plan(&own, Some(&next))?;
                    self.plans.book.insert(own.clone(), next);
                    (own.clone(), true)
                }
            }
            // Dropping a plan that is not there again changes nothing.
            PlanCommand::Drop { .. } => {
                if self.plans.book.contains_key(&own) {
                    self.save_plan(&own, None)?;
                    self.plans.book.remove(&own);
                    (own.clone(), true)
                } else {
                    (own.clone(), false)
                }
            }
        };
        if changed {
            self.plans.edit += 1;
            crate::diagnostic!(
                json!({"component":"plans","kind":"plans.changed","pane_id":pane,"session_id":session.session_id})
            );
        }
        let plan = self
            .plans
            .book
            .get(&session)
            .map(|plan| view_of(plan, &pane));
        let context = self.plan_context_of(device, &actor, &session);
        Ok((
            json!({"session": session, "plan": plan, "context": context}),
            changed,
        ))
    }

    /// The summary the prompt hook writes into the pane's record (D-30):
    /// none for a caller Hide cannot place in a pane.
    pub(crate) fn plan_context(&self, device: &str, caller: &str) -> Option<PlanContext> {
        let Caller::Pane(pane) = Caller::parse(caller) else {
            return None;
        };
        let pane = if device == self.node.as_str() || pane.starts_with("remote:") {
            pane.to_owned()
        } else {
            format!("remote:{device}:pane:{pane}")
        };
        let actor = &self.delivery_observations.get(&pane)?.actor;
        let session = self.caller_session(device, &pane, actor)?;
        Some(self.plan_context_of(device, actor, &session))
    }

    fn plan_context_of(
        &self,
        _device: &str,
        actor: &Actor,
        session: &SessionIdentity,
    ) -> PlanContext {
        let ledger = self.delivery_state().ok();
        let worker_of = |node: &crate::missions::MissionNode| {
            node.agent_id.as_deref().is_some_and(|id| {
                ledger.as_ref().is_some_and(|ledger| {
                    ledger
                        .agents
                        .iter()
                        .any(|record| record.id == id && record.actor.same_identity(actor))
                })
            })
        };
        let mission = self
            .missions
            .book
            .missions
            .iter()
            .filter(|mission| mission.status.open())
            .find_map(|mission| {
                let node = mission.nodes.iter().find(|node| worker_of(node))?;
                Some(MissionWork {
                    mission: mission.id.clone(),
                    node: node.key.clone(),
                    scope: match mission.scope {
                        crate::missions::MissionScope::Pr => "pr",
                        crate::missions::MissionScope::Merge => "merge",
                    }
                    .to_owned(),
                    range: node.criteria.clone().unwrap_or_else(|| node.title.clone()),
                })
            });
        let conductor = self.missions.book.conducted_open(actor).map(|mission| {
            let count = |pick: fn(crate::missions::NodeStatus) -> bool| {
                u32::try_from(
                    mission
                        .nodes
                        .iter()
                        .filter(|node| pick(node.status))
                        .count(),
                )
                .unwrap_or(u32::MAX)
            };
            ConductorWork {
                mission: mission.id.clone(),
                done: count(|status| status == crate::missions::NodeStatus::Done),
                nodes: count(|status| status != crate::missions::NodeStatus::Dropped),
                working: count(|status| status == crate::missions::NodeStatus::Working),
                letters: ledger.as_ref().map_or(0, |ledger| {
                    u32::try_from(
                        ledger
                            .letters
                            .iter()
                            .filter(|letter| {
                                letter.recipient.same_identity(actor)
                                    && letter.state == crate::delivery::ledger::State::Pending
                            })
                            .count(),
                    )
                    .unwrap_or(u32::MAX)
                }),
                decision: mission.decision.is_some(),
            }
        });
        PlanContext {
            plan: self.plans.book.get(session).map(summary_of),
            mission,
            conductor,
        }
    }

    /// Lays a proposed plan on its agent's row as the question it is (B14,
    /// D-30): the row then waits on the operator, carrying the review as
    /// its subject, which the band, the second line, the bell and the phone
    /// word in their own language. The agent's own question comes first,
    /// and a working agent asks nothing.
    pub(super) fn lay_plan_reviews(&mut self, device: &str, agents: &mut [SidebarAgentSnapshot]) {
        // Rows Herdr just sent carry nothing laid.
        for agent in agents.iter() {
            self.plans.laid.remove(&agent.pane_id);
        }
        self.lay_reviews_on(device, agents);
    }

    /// Lays the reviews again on the rows already shown once the book moved,
    /// lifting first what an earlier pass laid, and rebuilds what reads the
    /// rows: the lineage raises the question, the read axis compares it.
    pub(super) fn refresh_plan_reviews(&mut self) -> bool {
        if self.plans.laid_at == self.plans.edit {
            return false;
        }
        self.plans.laid_at = self.plans.edit;
        let node = self.node.as_str().to_owned();
        let mut agents = std::mem::take(&mut self.snapshot.navigator.agents);
        let mut moved = self.relay_reviews(&node, &mut agents);
        self.snapshot.navigator.agents = agents;
        for at in 0..self.snapshot.status.remote.len() {
            let remote = &mut self.snapshot.status.remote[at];
            let target = remote.target_id.clone();
            let Some(mut agents) = remote
                .session
                .as_mut()
                .map(|session| std::mem::take(&mut session.agents))
            else {
                continue;
            };
            moved |= self.relay_reviews(&target, &mut agents);
            if let Some(session) = self.snapshot.status.remote[at].session.as_mut() {
                session.agents = agents;
            }
        }
        // A pane that closed since it was laid on is not shown to lift from.
        let snapshot = &self.snapshot;
        self.plans.laid.retain(|pane, _| {
            snapshot
                .navigator
                .agents
                .iter()
                .any(|agent| agent.pane_id == *pane)
                || snapshot.status.remote.iter().any(|remote| {
                    remote.session.as_ref().is_some_and(|session| {
                        session.agents.iter().any(|agent| agent.pane_id == *pane)
                    })
                })
        });
        if !moved {
            return false;
        }
        self.refresh_agent_lineage();
        self.refresh_pane_read_state();
        true
    }

    /// Lifts what was laid on `agents` while it is still there, then lays
    /// the reviews again. Whether any row changed.
    fn relay_reviews(&mut self, device: &str, agents: &mut [SidebarAgentSnapshot]) -> bool {
        let before = agents.to_vec();
        for agent in agents.iter_mut() {
            let Some(laid) = self.plans.laid.remove(&agent.pane_id) else {
                continue;
            };
            // A question the agent asked since is its own, and stays.
            if agent.user_turn.as_ref() == Some(&laid.turn)
                && agent.ask_subject == Some(laid.subject)
            {
                agent.user_turn = None;
                agent.ask_subject = None;
                agent.demand = laid.demand;
                crate::agent_state::derive_from_axes(agent);
            }
        }
        self.lay_reviews_on(device, agents);
        before != agents
    }

    fn lay_reviews_on(&mut self, device: &str, agents: &mut [SidebarAgentSnapshot]) {
        if !self
            .plans
            .book
            .values()
            .any(|plan| plan.review == Review::Proposed)
        {
            return;
        }
        for agent in agents.iter_mut() {
            if agent.user_turn.is_some()
                || agent.activity == "working"
                || crate::agent_state::demand_of(agent) != crate::agent_state::AgentDemand::None
            {
                continue;
            }
            let Some(plan) = row_session(device, agent)
                .and_then(|session| self.plans.book.get(&session))
                .filter(|plan| plan.review == Review::Proposed && !plans::finished(plan))
            else {
                continue;
            };
            // The question holds the row as a native one does; it carries
            // no words, only its subject.
            let turn = hide_session::turns::UserTurnFact {
                kind: hide_session::turns::UserTurnKind::Question,
                content: None,
            };
            let subject = review_subject(plan);
            self.plans.laid.insert(
                agent.pane_id.clone(),
                Laid {
                    turn: turn.clone(),
                    subject,
                    demand: std::mem::replace(&mut agent.demand, "question".to_owned()),
                },
            );
            agent.user_turn = Some(turn);
            agent.ask_subject = Some(subject);
            crate::agent_state::derive_from_axes(agent);
        }
    }

    /// Drops the plans of sessions that were deleted or pruned; their rows
    /// went with the session's other rows in the same store.
    pub(crate) fn drop_plans_of(&mut self, sessions: &[(String, String)]) -> bool {
        let before = self.plans.book.len();
        self.plans.book.retain(|session, _| {
            !sessions
                .iter()
                .any(|(machine, id)| session.device_id == *machine && session.session_id == *id)
        });
        let dropped = before - self.plans.book.len();
        if dropped > 0 {
            self.plans.edit += 1;
            // A slot item of a session that is gone would draw nothing.
            self.drop_detail_items(|item| match item {
                crate::model::DetailItem::Plan { session } => {
                    !sessions.iter().any(|(machine, id)| {
                        session.device_id == *machine && session.session_id == *id
                    })
                }
                _ => true,
            });
        }
        dropped > 0
    }

    /// Makes the plans view again when the book or the panes moved; with no
    /// plan and nothing shown it does nothing.
    pub(super) fn refresh_plans(&mut self) {
        if self.plans.book.is_empty() && self.snapshot.plans.items.is_empty() {
            return;
        }
        let live = self.live_sessions();
        let print = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for (pane, session) in &live {
                if self.plans.book.contains_key(session) {
                    pane.hash(&mut hasher);
                    session.hash(&mut hasher);
                }
            }
            hasher.finish()
        };
        if self.plans.made_at == Some((self.plans.edit, print)) {
            return;
        }
        self.plans.made_at = Some((self.plans.edit, print));
        self.snapshot.plans.items = live
            .iter()
            .filter_map(|(pane, session)| Some(view_of(self.plans.book.get(session)?, pane)))
            .collect();
    }

    /// The plan of the worker in `pane` on `machine`, for a mission node's
    /// card and detail (D-17, D-18).
    pub(super) fn worker_plan(&self, machine: &str, pane: &str) -> Option<PlanView> {
        let agent = self
            .machine_agents(machine)
            .iter()
            .find(|agent| same_pane(&agent.pane_id, pane))?;
        let session = row_session(machine, agent)?;
        Some(view_of(self.plans.book.get(&session)?, &agent.pane_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The summary the core makes of each contract plan is the one the
    /// hook reads (`contracts/delivery-envelope.json`, `plan_context`).
    #[test]
    fn the_plan_summary_matches_the_delivery_envelope_contract() {
        let contract: Value =
            serde_json::from_str(include_str!("../../../contracts/delivery-envelope.json"))
                .unwrap();
        let mut checked = 0;
        for example in contract["plan_context"]["examples"].as_array().unwrap() {
            let Some(steps) = example.get("steps") else {
                continue;
            };
            let plan = Plan {
                session: SessionIdentity {
                    device_id: "mac".into(),
                    agent: "claude".into(),
                    session_id: "s1".into(),
                },
                steps: serde_json::from_value(steps.clone()).unwrap(),
                review: if example["ask"] == true {
                    Review::Proposed
                } else {
                    Review::Approved
                },
                ask: None,
                created_at_unix_ms: 0,
                updated_at_unix_ms: example["updated_at_unix_ms"].as_u64().unwrap(),
            };
            assert_eq!(
                serde_json::to_value(summary_of(&plan)).unwrap(),
                example["context"]["plan"],
                "{}",
                example["name"]
            );
            checked += 1;
        }
        assert!(checked >= 2);
    }
}
