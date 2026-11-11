//! The runtime half of a session's own plan (PRD hide-plan): the `hide
//! plan` commands, each applied under the lock as one change and handed to
//! the link worker to save; the plans the shell draws; the review a
//! proposed plan waits on, laid on the agent's row as a question so it
//! reaches the band, the sidebar and the bell by the ask's own path (D-12,
//! D-30); and the summary the hook keeps of it (D-14, D-15).
//!
//! A step handed to another agent (PRD hide-plan-delegate) is admitted here
//! before its spawn reaches the ledger, follows its child's letters and end
//! from the published ledger, and asks its child to sleep once its main
//! finished or took it back. A step's evidence is read off the lock by the
//! caller's request worker (`PlanProof`) and the change is judged again
//! under the lock with what it read.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::Serialize;
use serde_json::json;

use super::Runtime;
use crate::coordination::{PlanAdmission, PlanLink, refusal};
use crate::delivery::Actor;
use crate::links::ledger::{LedgerAnswer, LedgerCall, LedgerUpdate, SessionIdentity};
use crate::model::{AskSubject, OpenAskSnapshot, OpenAsksSnapshot, SidebarAgentSnapshot};
use crate::plans::{
    self, ChildPlace, Delegation, DelegationState, Evidence, PLAN_LIMIT, Plan, PlanCommand,
    PlanProof, PlanTurn, PlanView, Proofs, Review, StepStatus,
};
use crate::workspace_control::{Caller, Context, Query};
use hide_agent_hooks::plan_record::{
    ConductorWork, CriteriaLeft, MissionWork, PlanContext, PlanSummary, StepName,
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
    /// that moved is lifted from those rows without waiting for the engine to
    /// send them again. Bounded by the rows shown.
    laid: BTreeMap<String, Laid>,
    /// The core's questions the rows shown were last laid with: a book
    /// edit that leaves them as they were lays nothing again.
    /// None once a sync laid rows from other questions, so the next
    /// refresh relays every row set whatever it reads.
    asked: Option<Vec<CoreAsk>>,
    /// The plan book's, the mission book's, the typed inputs' and the open
    /// questions' edit `asked` was read at.
    made_asks_at: (u64, u64, u64, u64),
    /// Typed input shown to the operator on its pane's row
    /// (`typed_input::input_asks`), as the doorbell's last pass found it,
    /// and an edit that moves whenever it changes.
    pub(crate) input_asks: Vec<super::typed_input::InputAsk>,
    pub(crate) input_edit: u64,
    /// Moves whenever the link summaries the sessions' open questions are
    /// read from change (`Runtime::open_asks`).
    pub(crate) asks_edit: u64,
    /// Typed input whose agent's composer was last read still showing it,
    /// by input id, with the typing it was read after: the proof that it
    /// was not taken (`AskSubject::InputUnconfirmed::held`). Bounded by the
    /// unconfirmed inputs; a later typing or a record makes it stale.
    pub(crate) input_held: BTreeMap<String, u8>,
    /// Typed input whose typings are spent with no record, by input id,
    /// once `input.undelivered` has logged it. Bounded by the unconfirmed
    /// inputs.
    pub(crate) input_undelivered: std::collections::BTreeSet<String>,
    /// Each unconfirmed input's fresh session read, asked once per typing
    /// before the input is judged unconfirmed: the typing and when it was
    /// asked. Bounded by the unconfirmed inputs.
    pub(crate) input_reread: BTreeMap<String, (u8, u64)>,
    /// Each unconfirmed input's last verdict reached while the core had a
    /// turn for its pane, by input id, with the typing it is for: what the
    /// row and `hide agent show` keep saying while the turn is not laid
    /// (`typed::Standing::Unsettled`). Bounded by the unconfirmed inputs.
    pub(crate) input_verdicts: BTreeMap<String, (u8, crate::delivery::typed::Standing)>,
    /// Briefs whose agent's hook reported a turn started after the typing
    /// with no other input to start it, by spawn id, with that typing: the
    /// turn is their record (`input.confirmed` with `by: turn`), kept until
    /// the store confirms it. Bounded by the unconfirmed briefs.
    pub(crate) input_turn_started: BTreeMap<String, u8>,
    /// The panes whose sessions are to be read again, taken by this
    /// machine's session-sync thread on its next tick, each once and at
    /// most `SESSION_REREADS_LIMIT` of them, so a machine whose label reader
    /// never started keeps no more than that; closed panes leave when
    /// another pane is queued (`Runtime::queue_session_reread`).
    pub(crate) session_rereads: Vec<String>,
    /// Whether a pane was refused a place in `session_rereads` since the
    /// reader last took it, so a full list is logged once, not per turn.
    pub(crate) session_rereads_refused: bool,
}

/// Whom the core asks a question of: a session, or the pane of an agent
/// that may have none yet (Codex before its first turn), for typed input
/// it never took.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Asked {
    Session(SessionIdentity),
    Pane(String),
}

/// A question the core asks, when it was asked, and the event that raised
/// it (`ask_id`).
type CoreAsk = (Asked, u64, AskSubject, String);

/// What laying a core question changed on a row: whom the questions laid
/// there were asked of, the question it put there, what it asks, and the
/// demand the row had before.
#[derive(Clone)]
struct Laid {
    asked: Asked,
    turn: hide_session::turns::UserTurnFact,
    subject: AskSubject,
    demand: String,
}

/// What names each of the core's questions: the event that raised it, never
/// its words or when it was laid, so the operator's read of a question
/// holds while it stands and a question raised again or anew is news
/// (docs/status-model.md, A question the core lays stays read). Each names
/// one raise: a plan put up for review again, a mission's next question or
/// next raise, is another.
mod ask_id {
    use crate::links::ledger::SessionIdentity;
    use crate::missions::{Attention, Mission, MissionNode};
    use crate::plans::Plan;

    /// A question the session put to the operator, by the session and the
    /// record's id for the line.
    pub(super) fn open_ask(session: &SessionIdentity, id: &str) -> String {
        format!(
            "ask:{}/{}/{}#{id}",
            session.device_id, session.agent, session.session_id
        )
    }

    /// A plan put up for review, by its session, its instance and when it
    /// was put up; a plan proposed before that was kept is named by its
    /// instance alone.
    pub(super) fn plan_review(plan: &Plan) -> String {
        let session = &plan.session;
        format!(
            "plan:{}/{}/{}:{}:{}",
            session.device_id,
            session.agent,
            session.session_id,
            plan.id(),
            plan.asked_at_unix_ms.unwrap_or_default()
        )
    }

    /// A mission's decision, by its question's key; a decision from before
    /// questions carried one is the mission's only one.
    pub(super) fn decision(mission: &Mission) -> String {
        match &mission.question {
            Some(question) => format!("decision:{}#{}", mission.id, question.key),
            None => format!("decision:{}", mission.id),
        }
    }

    /// A node's attention, by the mission's count of raises that names it.
    pub(super) fn attention(
        mission: &Mission,
        node: &MissionNode,
        attention: &Attention,
    ) -> String {
        format!("attention:{}/{}#{}", mission.id, node.key, attention.key)
    }

    /// Typed input not taken, by the brief's spawn or the letter.
    pub(super) fn input(id: &str) -> String {
        format!("input:{id}")
    }
}

/// The plans the shell draws, one per pane whose session has one (B7).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PlansSnapshot {
    pub items: Vec<PlanView>,
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

/// Whether a core question still stands on `agent`'s row: a question the
/// session put to the operator stands only on a row the operator owns (a
/// delegated row's questions are its parent's to answer) and only until the
/// operator sends the agent something, which Hide AI then reads again at the
/// turn's end (docs/status-model.md, A question still waiting).
fn open_ask_stands(agent: &SidebarAgentSnapshot, subject: &AskSubject, since: u64) -> bool {
    if !matches!(subject, AskSubject::OpenAsk { .. }) {
        return true;
    }
    !agent.delegated
        && agent
            .row_facts
            .as_ref()
            .and_then(|facts| facts.operator_request.as_ref())
            .is_none_or(|request| request.at_unix_ms < since)
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
    /// What the plans still need from the delivery ledger: the spawn and
    /// the child of each step still handed, which the fold of its letters
    /// and end reads (`fold_delegations`). Before the saved plans are read,
    /// every spawn a step started is held with its child instead.
    pub(super) fn plan_holds(
        &self,
        ledger: Option<&crate::delivery::ledger::Ledger>,
        held: &mut crate::coordination::Held,
    ) {
        if self.plans.load != Load::Ready {
            for spawn in ledger
                .into_iter()
                .flat_map(|ledger| &ledger.spawns)
                .filter(|spawn| spawn.plan.is_some())
            {
                held.spawns.insert(spawn.id.clone());
                held.agents.extend(spawn.child.clone());
            }
            return;
        }
        for delegation in self
            .plans
            .book
            .values()
            .flat_map(|plan| &plan.steps)
            .filter(|step| !step.status.finished())
            .filter_map(|step| step.delegation.as_ref())
            .filter(|delegation| delegation.active())
        {
            held.spawns.insert(delegation.spawn.clone());
            held.agents.extend(delegation.agent_id.clone());
        }
    }

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
                // What the ledger said while the plans were being read.
                self.fold_delegations();
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
            .find(|agent| {
                crate::address::same_pane(self.node.as_str(), device, &agent.pane_id, pane)
            })
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
    pub(super) fn live_sessions(&self) -> Vec<(String, SessionIdentity)> {
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

    /// Runs a `hide task` command from `caller`'s pane on `device`, judged
    /// with the evidence `proofs` already read. Answers the command's answer
    /// and whether anything changed, or the reads its evidence still needs,
    /// which the caller makes off the lock before asking again.
    pub(crate) fn plan_command(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        command: PlanCommand,
        proofs: &Proofs,
    ) -> Result<PlanTurn, String> {
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
        let pane = crate::address::named_pane_id(self.node.as_str(), device, pane);
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
        // A step's file is read from the session's checkout (B12).
        let root = context.checkout_path.clone();
        let below = |path: &str| {
            plans::file_below(&root, path).ok_or_else(|| refusal("plan_file_outside", path))
        };
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
                        .map(|plan| self.plan_view(plan, &pane));
                    return Ok(PlanTurn::Answered {
                        answer: json!({"session": session, "plan": plan}),
                        changed: false,
                    });
                }
                (own.clone(), false)
            }
            PlanCommand::Approve { agent } | PlanCommand::Drop { agent }
                if named(self, &agent)?.is_some() =>
            {
                return Err("plan_not_yours".into());
            }
            PlanCommand::Set {
                plan: mut input,
                ask,
                why,
                reason,
            } => {
                for step in &mut input.nodes {
                    if let Some(path) = step.file.take() {
                        step.file = Some(below(&path)?);
                    }
                }
                let previous = self.plans.book.get(&own);
                let mut next = plans::apply_set(
                    previous,
                    &own,
                    &input,
                    ask,
                    why.as_deref(),
                    reason.as_deref(),
                    now,
                )?;
                let before = previous.filter(|plan| !plans::finished(plan));
                let asks = plans::judge(before, &mut next, proofs)?;
                if !asks.is_empty() {
                    return self.plan_proof(device, &root, &own, &next, asks);
                }
                let before = before.cloned();
                if previous.is_none() {
                    self.make_room()?;
                }
                self.keep_plan(&own, before.as_ref(), next)?;
                (own.clone(), true)
            }
            PlanCommand::Node {
                key,
                status,
                line,
                pr,
                file,
            } => {
                let claim = match (pr, file) {
                    (Some(number), _) => Some(Evidence::Pr {
                        repo: String::new(),
                        number,
                    }),
                    (None, Some(path)) => Some(Evidence::File {
                        path: below(&path)?,
                    }),
                    (None, None) => None,
                };
                let plan = self.plans.book.get(&own).ok_or("plan_missing")?;
                match plans::apply_node(plan, &key, status, line.as_deref(), claim, now)? {
                    Some(mut next) => {
                        let asks = plans::judge(Some(plan), &mut next, proofs)?;
                        if !asks.is_empty() {
                            return self.plan_proof(device, &root, &own, &next, asks);
                        }
                        let before = plan.clone();
                        self.keep_plan(&own, Some(&before), next)?;
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
                if let Some(plan) = self.plans.book.get(&own).cloned() {
                    self.save_plan(&own, None)?;
                    self.plans.book.remove(&own);
                    // Its children's steps are gone with it.
                    self.ask_children_to_sleep(&plan, |_, _| true);
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
            .map(|plan| self.plan_view(plan, &pane));
        let context = self.plan_context_of(device, &actor, &session);
        Ok(PlanTurn::Answered {
            answer: json!({"session": session, "plan": plan, "context": context}),
            changed,
        })
    }

    /// The reads `asks` needs, made off the lock on the session's machine;
    /// a machine Hide cannot reach now refuses the change as unconfirmed.
    fn plan_proof(
        &mut self,
        device: &str,
        root: &str,
        session: &SessionIdentity,
        plan: &Plan,
        asks: plans::Asks,
    ) -> Result<PlanTurn, String> {
        let link = self.node_link(device).map_err(|reason| {
            crate::diagnostic!(json!({
                "component": "plans", "kind": "plans.proof_unreachable",
                "plan": plan.id(), "session_id": session.session_id, "device": device,
                "reason": reason,
            }));
            asks.unconfirmed()
        })?;
        let branches = self
            .delivery_state()
            .map(|ledger| {
                ledger
                    .spawns
                    .iter()
                    .filter(|spawn| {
                        spawn
                            .plan
                            .as_ref()
                            .is_some_and(|link| link.parent_session == *session)
                    })
                    .map(|spawn| spawn.branch.clone())
                    .collect()
            })
            .unwrap_or_default();
        Ok(PlanTurn::Prove(PlanProof {
            link,
            root: root.to_owned(),
            branches,
            asks,
            plan: plan.id(),
            session_id: session.session_id.clone(),
        }))
    }

    /// Keeps `next` as `session`'s plan in place of `before`, and asks the
    /// child of each step it finished or took back to sleep (B17, B18).
    fn keep_plan(
        &mut self,
        session: &SessionIdentity,
        before: Option<&Plan>,
        next: Plan,
    ) -> Result<(), String> {
        self.save_plan(session, Some(&next))?;
        let ended = |step: &plans::Step, prior: Option<&plans::Step>| {
            let was_open = prior.is_some_and(|prior| {
                prior.delegation.as_ref().is_some_and(Delegation::active)
                    && prior.status != StepStatus::Done
            });
            was_open
                && (step.status == StepStatus::Done
                    || step
                        .delegation
                        .as_ref()
                        .is_none_or(|delegation| !delegation.active()))
        };
        let steps: BTreeMap<&str, &plans::Step> = before
            .map(|plan| {
                plan.steps
                    .iter()
                    .map(|step| (step.key.as_str(), step))
                    .collect()
            })
            .unwrap_or_default();
        self.ask_children_to_sleep(&next, |step, _| {
            ended(step, steps.get(step.key.as_str()).copied())
        });
        self.plans.book.insert(session.clone(), next);
        Ok(())
    }

    /// Marks for sleep the child of each step of `plan` that `pick` names
    /// (B18): the minute decision ends it once it rests off screen, and a
    /// child that is asleep, gone or set not to sleep is left as it is.
    fn ask_children_to_sleep(
        &mut self,
        plan: &Plan,
        pick: impl Fn(&plans::Step, &Delegation) -> bool,
    ) {
        for step in &plan.steps {
            let Some(delegation) = step.delegation.as_ref() else {
                continue;
            };
            let Some(agent) = delegation.agent_id.as_deref() else {
                continue;
            };
            if !pick(step, delegation)
                || self
                    .missions
                    .sleep_marks
                    .iter()
                    .any(|mark| mark.agent_id == agent)
            {
                continue;
            }
            crate::diagnostic!(json!({
                "component": "plans", "kind": "plans.child_sleep_asked",
                "plan": plan.id(), "step": step.key, "attempt": delegation.attempt,
                "agent_id": agent,
            }));
            self.missions
                .sleep_marks
                .push(super::agent_sleep::missions::SleepMark::plan(
                    agent,
                    &plan.id(),
                    &step.key,
                ));
        }
    }

    /// The summary the prompt hook writes into the pane's record (D-30):
    /// none for a caller Hide cannot place in a pane.
    pub(crate) fn plan_context(&self, device: &str, caller: &str) -> Option<PlanContext> {
        let Caller::Pane(pane) = Caller::parse(caller) else {
            return None;
        };
        let pane = crate::address::named_pane_id(self.node.as_str(), device, pane);
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
        let checker_of = |node: &crate::missions::MissionNode| {
            node.criteria.iter().find_map(|line| {
                line.checkers
                    .iter()
                    .any(|checker| {
                        checker.agent_id.as_deref().is_some_and(|id| {
                            ledger.as_ref().is_some_and(|ledger| {
                                ledger.agents.iter().any(|record| {
                                    record.id == id && record.actor.same_identity(actor)
                                })
                            })
                        })
                    })
                    .then(|| line.key.clone())
            })
        };
        let mission = self
            .missions
            .book
            .missions
            .iter()
            .filter(|mission| mission.status.open())
            .find_map(|mission| {
                let (node, line) = mission.nodes.iter().find_map(|node| {
                    if worker_of(node) {
                        Some((node, None))
                    } else {
                        checker_of(node).map(|line| (node, Some(line)))
                    }
                })?;
                let own: Vec<&str> = node
                    .criteria
                    .iter()
                    .filter(|line| line.kind == crate::mission_criteria::CriterionKind::Own)
                    .filter_map(|line| line.text.as_deref())
                    .collect();
                let criteria = (!mission.exempt).then(|| {
                    let left = self.node_unmet(mission, node);
                    let total = node.criteria.len()
                        + usize::from(
                            left.iter().any(|key| key == "ci")
                                && !node.criteria.iter().any(|line| {
                                    line.kind == crate::mission_criteria::CriterionKind::Ci
                                }),
                        );
                    CriteriaLeft {
                        met: u32::try_from(total.saturating_sub(left.len())).unwrap_or(u32::MAX),
                        total: u32::try_from(total).unwrap_or(u32::MAX),
                        left,
                    }
                });
                Some(MissionWork {
                    mission: mission.id.clone(),
                    node: node.key.clone(),
                    scope: match mission.scope {
                        crate::missions::MissionScope::Pr => "pr",
                        crate::missions::MissionScope::Merge => "merge",
                    }
                    .to_owned(),
                    range: if own.is_empty() {
                        node.title.clone()
                    } else {
                        own.join("; ")
                    },
                    role: if line.is_some() { "checker" } else { "worker" }.to_owned(),
                    line,
                    criteria,
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
            // A node its worker reported done while lines are left (B22).
            let reported = |node: &crate::missions::MissionNode| {
                let Some(ledger) = ledger.as_ref() else {
                    return false;
                };
                let Some(worker) = node
                    .agent_id
                    .as_deref()
                    .and_then(|id| ledger.agents.iter().find(|record| record.id == id))
                else {
                    return false;
                };
                ledger.letters.iter().any(|letter| {
                    letter.kind == "report"
                        && letter.sender.same_identity(&worker.actor)
                        && letter.recipient.same_identity(actor)
                })
            };
            let unmet = if mission.exempt {
                Vec::new()
            } else {
                mission
                    .nodes
                    .iter()
                    .filter(|node| {
                        !node.status.finished()
                            && node.kind != crate::missions::NodeKind::External
                            && reported(node)
                            && !self.node_unmet(mission, node).is_empty()
                    })
                    .map(|node| node.key.clone())
                    .collect()
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
                unmet,
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
        // Rows the engine just sent carry nothing laid.
        for agent in agents.iter() {
            self.plans.laid.remove(&agent.pane_id);
        }
        let asks = self.core_asks();
        if self.plans.asked.as_ref() != Some(&asks) {
            self.plans.asked = None;
        }
        self.lay_reviews_on(device, agents, &asks);
    }

    /// Lays the reviews again on the rows already shown once what the core
    /// asks moved, lifting first what an earlier pass laid, and rebuilds
    /// what reads the rows: the lineage raises the question, the read axis
    /// compares it. A plan or mission edit that asks nothing new costs one
    /// pass over the books, not over the rows.
    pub(super) fn refresh_plan_reviews(&mut self) -> bool {
        if self.plans.edit == self.plans.made_asks_at.0
            && self.missions.book_edit == self.plans.made_asks_at.1
            && self.plans.input_edit == self.plans.made_asks_at.2
            && self.plans.asks_edit == self.plans.made_asks_at.3
        {
            return false;
        }
        self.plans.made_asks_at = (
            self.plans.edit,
            self.missions.book_edit,
            self.plans.input_edit,
            self.plans.asks_edit,
        );
        let asks = self.core_asks();
        if self.plans.asked.as_ref() == Some(&asks) {
            return false;
        }
        self.plans.asked = Some(asks.clone());
        let node = self.node.as_str().to_owned();
        let mut agents = std::mem::take(&mut self.snapshot.navigator.agents);
        let mut moved = self.relay_reviews(&node, &mut agents, &asks);
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
            // The device's session kept as it arrived was laid with the
            // published one and is published from again, so it is relaid
            // the same way, from what was laid before (`laid` then follows
            // the published rows).
            if let Some(mut raw) = self.device_raw_sessions.remove(&target) {
                let laid = self.plans.laid.clone();
                self.relay_reviews(&target, &mut raw.agents, &asks);
                self.plans.laid = laid;
                self.device_raw_sessions.insert(target.clone(), raw);
            }
            moved |= self.relay_reviews(&target, &mut agents, &asks);
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
        // Every machine's rows: a question laid on a device's resting row
        // is news there now, not at the device's next session.
        self.refresh_read_state_everywhere();
        true
    }

    /// Lifts what was laid on `agents` while it is still there, then lays
    /// the reviews again. Whether any row changed.
    fn relay_reviews(
        &mut self,
        device: &str,
        agents: &mut [SidebarAgentSnapshot],
        asks: &[CoreAsk],
    ) -> bool {
        let before = agents.to_vec();
        for agent in agents.iter_mut() {
            agent.open_asks = None;
            let Some(laid) = self.plans.laid.remove(&agent.pane_id) else {
                continue;
            };
            // A question the agent asked since is its own, and stays.
            if agent.user_turn.as_ref() == Some(&laid.turn)
                && agent.ask_subject.as_ref() == Some(&laid.subject)
            {
                agent.user_turn = None;
                agent.ask_subject = None;
                agent.laid_since_unix_ms = None;
                agent.laid_more = 0;
                agent.laid_asks.clear();
                agent.demand = laid.demand;
                crate::agent_state::derive_from_axes(agent);
            }
        }
        self.lay_reviews_on(device, agents, asks);
        before != agents
    }

    /// Lays on each resting row the core's questions for its session: its
    /// plan waiting for review, the open decision of the mission it
    /// conducts (needs-you-cleanup D-12) and the questions it put to the
    /// operator that still wait. One question holds the row, the one asked
    /// first; the others are counted behind it (D-13). The open questions
    /// are also carried on the row whatever it does, for its `?N`.
    fn lay_reviews_on(
        &mut self,
        device: &str,
        agents: &mut [SidebarAgentSnapshot],
        asks: &[CoreAsk],
    ) {
        if asks.is_empty() {
            return;
        }
        for agent in agents.iter_mut() {
            let session = row_session(device, agent);
            // Oldest first, as `core_asks` sorts them.
            let mine: Vec<&CoreAsk> = asks
                .iter()
                .filter(|(asked, ..)| match asked {
                    Asked::Session(asked) => session.as_ref() == Some(asked),
                    Asked::Pane(pane) => *pane == agent.pane_id,
                })
                .filter(|(_, since, subject, _)| open_ask_stands(agent, subject, *since))
                .collect();
            agent.open_asks = session.as_ref().and_then(|session| {
                let items: Vec<OpenAskSnapshot> = mine
                    .iter()
                    .filter_map(|(_, since, subject, _)| match subject {
                        AskSubject::OpenAsk { id, line } => Some(OpenAskSnapshot {
                            id: id.clone(),
                            text: line.clone(),
                            at_unix_ms: *since,
                        }),
                        _ => None,
                    })
                    .collect();
                (!items.is_empty()).then(|| OpenAsksSnapshot {
                    session: session.clone(),
                    items,
                })
            });
            if agent.user_turn.is_some()
                || agent.activity == "working"
                || crate::agent_state::demand_of(agent) != crate::agent_state::AgentDemand::None
            {
                continue;
            }
            let Some((asked, since, subject, _)) = mine.first() else {
                continue;
            };
            let (asked, since, subject, more) =
                (asked.clone(), *since, subject.clone(), mine.len() - 1);
            // The question holds the row as a native one does; it carries
            // no words, only its subject.
            let turn = hide_session::turns::UserTurnFact {
                kind: hide_session::turns::UserTurnKind::Question,
                content: None,
            };
            self.plans.laid.insert(
                agent.pane_id.clone(),
                Laid {
                    asked,
                    turn: turn.clone(),
                    subject: subject.clone(),
                    demand: std::mem::replace(&mut agent.demand, "question".to_owned()),
                },
            );
            agent.user_turn = Some(turn);
            agent.ask_subject = Some(subject);
            agent.laid_since_unix_ms = Some(since);
            agent.laid_more = more;
            agent.laid_asks = mine.iter().map(|(.., id)| id.clone()).collect();
            crate::agent_state::derive_from_axes(agent);
        }
    }

    /// The core's questions, oldest first, each with the session it is asked
    /// for and when: a plan waiting for review and the open decision of a
    /// mission its Conductor's session answers.
    fn core_asks(&self) -> Vec<CoreAsk> {
        let reviews = self
            .plans
            .book
            .iter()
            .filter(|(_, plan)| plan.review == Review::Proposed && !plans::finished(plan))
            .map(|(session, plan)| {
                (
                    Asked::Session(session.clone()),
                    plan.updated_at_unix_ms,
                    review_subject(plan),
                    ask_id::plan_review(plan),
                )
            });
        let decisions = self.missions.book.missions.iter().filter_map(|mission| {
            let line = mission.decision.as_ref()?;
            let session = mission.conductor.session.as_ref()?;
            if !mission.status.open() {
                return None;
            }
            // Answered from the map, it leaves the row at once, while the
            // answer is still on its way to the Conductor (B3).
            if mission.answer.as_ref().is_some_and(|answer| {
                mission
                    .question
                    .as_ref()
                    .is_none_or(|question| question.key == answer.key)
            }) {
                return None;
            }
            // A decision from before questions carried their time is dated
            // by its mission, a stamp no later edit moves.
            let since = mission
                .question
                .as_ref()
                .map_or(mission.created_at_unix_ms, |question| {
                    question.asked_at_unix_ms
                });
            Some((
                Asked::Session(session.clone()),
                since,
                AskSubject::MissionDecision {
                    line: line.clone(),
                    // The open ones: a closed View stays on the card (D-20) but not in the eye's count.
                    views: u32::try_from(
                        self.decision_views(mission)
                            .iter()
                            .filter(|view| !view.closed)
                            .count(),
                    )
                    .unwrap_or(u32::MAX),
                },
                ask_id::decision(mission),
            ))
        });
        let inputs = self
            .plans
            .input_asks
            .iter()
            .map(|(pane, since, subject, id)| {
                (
                    Asked::Pane(pane.clone()),
                    *since,
                    subject.clone(),
                    ask_id::input(id),
                )
            });
        let attentions = self.mission_attention_asks();
        let open = self.open_asks();
        let mut asks: Vec<_> = reviews
            .chain(decisions)
            .chain(inputs)
            .chain(attentions)
            .chain(open)
            .collect();
        asks.sort_by_key(|(_, since, ..)| *since);
        asks
    }

    /// The questions each session put to the operator that still wait, as
    /// its record's `나를 기다리는 것` lists them (the link summaries' `asks`),
    /// while Hide AI reads the sessions' turns: with summaries off nothing
    /// keeps the list, so none is asked (docs/status-model.md, A question
    /// still waiting).
    fn open_asks(&self) -> Vec<CoreAsk> {
        if !super::backlog::summaries_on(self.ai_settings.as_ref()) {
            return Vec::new();
        }
        let Some(summaries) = self.snapshot.link_summaries.as_ref() else {
            return Vec::new();
        };
        let mut seen = std::collections::BTreeSet::new();
        summaries
            .projects
            .values()
            .flat_map(|project| &project.work)
            .filter(|work| !work.asks.is_empty() && seen.insert(&work.session))
            .flat_map(|work| {
                work.asks.iter().map(|ask| {
                    (
                        Asked::Session(work.session.clone()),
                        ask.at_unix_ms,
                        AskSubject::OpenAsk {
                            id: ask.id.clone(),
                            line: ask.text.clone(),
                        },
                        ask_id::open_ask(&work.session, &ask.id),
                    )
                })
            })
            .collect()
    }

    /// The mission nodes that wait on the operator (PRD mission-node-simple
    /// D-05, B14), each laid on the Conductor's row while it holds the
    /// mission, else on the node worker's row while one runs or sleeps.
    fn mission_attention_asks(&self) -> Vec<CoreAsk> {
        let ledger = self.delivery_state().ok();
        let mut out = Vec::new();
        for mission in &self.missions.book.missions {
            if !mission.status.asks() {
                continue;
            }
            for node in &mission.nodes {
                let Some(attention) = &node.attention else {
                    continue;
                };
                let Some(since) = attention.operator_at_unix_ms else {
                    continue;
                };
                let subject = AskSubject::MissionAttention {
                    node: node.title.clone(),
                    reason: attention.reason,
                    unmet: if attention.reason == crate::missions::AttentionReason::CloseRefused {
                        u32::try_from(self.unmet_done_nodes(mission).len()).unwrap_or(u32::MAX)
                    } else {
                        0
                    },
                };
                if let Some(asked) = self.mission_attention_row(ledger.as_deref(), mission, node) {
                    out.push((
                        asked,
                        since,
                        subject,
                        ask_id::attention(mission, node, attention),
                    ));
                }
            }
        }
        out
    }

    /// The Needs You row a node's attention is laid on (PRD
    /// mission-node-simple B5): the Conductor's while it is there, else the
    /// node's worker's while its registration stands. None when neither
    /// stands, and the operator is told by notice instead
    /// (`claim_mission_notices`).
    fn mission_attention_row(
        &self,
        ledger: Option<&crate::delivery::ledger::Ledger>,
        mission: &crate::missions::Mission,
        node: &crate::missions::MissionNode,
    ) -> Option<Asked> {
        let conductor = self
            .conductor_present(&mission.conductor.actor)
            .then(|| mission.conductor.session.clone())
            .flatten()
            .map(Asked::Session);
        conductor.or_else(|| {
            let agent = node.agent_id.as_deref()?;
            let record = ledger?
                .agents
                .iter()
                .find(|record| record.id == agent && !record.ended)?;
            Some(Asked::Pane(record.actor.pane_id.clone()))
        })
    }

    /// Whether a node's attention is on a Needs You row shown now: laid on
    /// the Conductor's or the worker's row, first or counted behind its
    /// first question. A row that works, or holds its own question, carries
    /// none, and a registration with no row shown carries none, so the
    /// mission is its own Needs You item then (PRD mission-node-simple B14).
    pub(super) fn mission_attention_has_row(
        &self,
        ledger: Option<&crate::delivery::ledger::Ledger>,
        mission: &crate::missions::Mission,
        node: &crate::missions::MissionNode,
    ) -> bool {
        self.mission_attention_row(ledger, mission, node)
            .is_some_and(|asked| self.plans.laid.values().any(|laid| laid.asked == asked))
    }

    /// The rows that carry a core question now, which decide where a
    /// mission's attention shows: an input of the mission views.
    pub(super) fn laid_panes(&self) -> Vec<String> {
        self.plans.laid.keys().cloned().collect()
    }

    /// `plan` as the screen draws it from `pane`, each handed step's child
    /// placed where Hide knows it now (B20).
    pub(super) fn plan_view(&self, plan: &Plan, pane: &str) -> PlanView {
        let ledger = self.delivery_state().ok();
        plans::view_with(plan, pane, &|agent| {
            self.child_place(ledger.as_deref(), agent)
        })
    }

    /// Where the child registered as `agent` is now: its pane while its row
    /// is shown, and its session, read from the row or else from the
    /// registration, so a sleeping or closed child's plan is still found.
    fn child_place(
        &self,
        ledger: Option<&crate::delivery::ledger::Ledger>,
        agent: &str,
    ) -> ChildPlace {
        match ledger.and_then(|ledger| ledger.agents.iter().find(|record| record.id == agent)) {
            Some(record) => self.record_place(record),
            None => (None, None),
        }
    }

    /// Where a registration is now: its pane while its row is shown, and its
    /// session, read from the row or else from the registration itself.
    fn record_place(&self, record: &crate::coordination::AgentRecord) -> ChildPlace {
        let row = (!record.ended)
            .then(|| {
                self.machine_agents(&record.machine).iter().find(|row| {
                    crate::address::same_pane(
                        self.node.as_str(),
                        &record.machine,
                        &row.pane_id,
                        &record.actor.pane_id,
                    )
                })
            })
            .flatten();
        let session = row
            .and_then(|row| row_session(&record.machine, row))
            .or_else(|| {
                Some(SessionIdentity {
                    device_id: record.machine.clone(),
                    agent: hide_session::Agent::from_kind(&record.actor.kind)?
                        .as_str()
                        .to_owned(),
                    session_id: record.session.clone(),
                })
            });
        (row.map(|row| row.pane_id.clone()), session)
    }

    /// Follows the published ledger into the plans (B16, B17, B20): a step's
    /// spawn links it to its new attempt, the child's registration is named
    /// once it binds, the attempt's letters between the child and its main
    /// move where it stands, and the child's end without a report closes
    /// it. Each delegation remembers the last letter it read, so a fold after
    /// a restart neither repeats a letter nor loses one; a change is saved
    /// like any other. Whether any plan changed.
    pub(super) fn fold_delegations(&mut self) -> bool {
        if self.plans.load != Load::Ready {
            return false;
        }
        let Ok(ledger) = self.delivery_state() else {
            return false;
        };
        let links: Vec<(&crate::coordination::SpawnRecord, &PlanLink)> = ledger
            .spawns
            .iter()
            .filter_map(|spawn| Some((spawn, spawn.plan.as_ref()?)))
            .collect();
        if links.is_empty() {
            return false;
        }
        let number = |id: &str| {
            id.rsplit_once('-')
                .and_then(|(_, number)| number.parse::<u64>().ok())
                .unwrap_or(0)
        };
        let newest = ledger.letters.last().map_or(0, |letter| number(&letter.id));
        let mut moved: Vec<(SessionIdentity, Plan)> = Vec::new();
        let sessions: BTreeSet<&SessionIdentity> =
            links.iter().map(|(_, link)| &link.parent_session).collect();
        for session in sessions {
            let Some(plan) = self.plans.book.get(session) else {
                continue;
            };
            let id = plan.id();
            let mut next = plan.clone();
            for (spawn, link) in links
                .iter()
                .filter(|(_, link)| link.parent_session == *session && link.plan == id)
            {
                let Some(step) = next.steps.iter_mut().find(|step| step.key == link.step) else {
                    continue;
                };
                match step.delegation.as_mut() {
                    Some(delegation) if delegation.attempt > link.attempt => {}
                    Some(delegation) if delegation.attempt == link.attempt => {
                        if delegation.agent_id.is_none() {
                            delegation.agent_id.clone_from(&spawn.child);
                        }
                    }
                    _ => {
                        crate::diagnostic!(json!({
                            "component": "plans", "kind": "plans.step_handed",
                            "plan": id, "step": step.key, "attempt": link.attempt,
                            "spawn": spawn.id,
                        }));
                        step.delegation = Some(Delegation {
                            attempt: link.attempt,
                            spawn: spawn.id.clone(),
                            name: spawn.name.clone(),
                            agent: spawn.kind.clone(),
                            agent_id: spawn.child.clone(),
                            state: DelegationState::Working,
                            reported_at_unix_ms: None,
                            released: false,
                            // Letters before the attempt are not its own.
                            seen: newest,
                        });
                        if step.status == StepStatus::Planned {
                            step.status = StepStatus::Working;
                        }
                    }
                }
            }
            // The attempt's letters and end, for every step still handed.
            for step in &mut next.steps {
                let Some(delegation) = step.delegation.as_mut().filter(|d| d.active()) else {
                    continue;
                };
                let Some(agent) = delegation.agent_id.as_deref() else {
                    continue;
                };
                let Some(child) = ledger.agents.iter().find(|record| record.id == agent) else {
                    continue;
                };
                let Some(parent) = links
                    .iter()
                    .find(|(spawn, _)| spawn.id == delegation.spawn)
                    .and_then(|(spawn, _)| {
                        ledger
                            .agents
                            .iter()
                            .find(|record| record.id == spawn.parent)
                    })
                else {
                    continue;
                };
                if delegation.seen < newest {
                    let start = ledger
                        .letters
                        .partition_point(|letter| number(&letter.id) <= delegation.seen);
                    for letter in &ledger.letters[start..] {
                        let from_child = letter.sender.same_identity(&child.actor)
                            && letter.recipient.same_identity(&parent.actor);
                        let to_child = letter.sender.same_identity(&parent.actor)
                            && letter.recipient.same_identity(&child.actor);
                        delegation.state = match (from_child, to_child, letter.kind.as_str()) {
                            (true, _, "report") => {
                                delegation.reported_at_unix_ms = Some(letter.created_at_unix_ms);
                                DelegationState::Reported
                            }
                            (true, _, "block") => DelegationState::Blocked,
                            (true, _, "progress") | (_, true, _)
                                if delegation.state == DelegationState::Blocked =>
                            {
                                DelegationState::Working
                            }
                            _ => delegation.state,
                        };
                    }
                    delegation.seen = newest;
                }
                if child.ended && delegation.state != DelegationState::Reported {
                    delegation.state = DelegationState::Closed;
                }
            }
            if next != *plan {
                moved.push((session.clone(), next));
            }
        }
        if moved.is_empty() {
            return false;
        }
        for (session, next) in moved {
            if let Err(code) = self.save_plan(&session, Some(&next)) {
                // Folded again from the ledger at the next publish.
                crate::diagnostic!(json!({
                    "component": "plans", "kind": "plans.fold_unsaved",
                    "session_id": session.session_id, "code": code,
                }));
                continue;
            }
            self.plans.book.insert(session, next);
        }
        self.plans.edit += 1;
        true
    }

    /// The admission of a spawn `actor` asks for with `plan_node` (B1-B4,
    /// B7, B8): none for a spawn that names no step, after refusing any
    /// spawn of a plan's child or a mission's worker or checker; and for a
    /// step, the link to its attempt, which the ledger counts under the cap.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn plan_spawn_admission(
        &self,
        actor: &crate::delivery::Actor,
        device: &str,
        parent: Option<&str>,
        plan_node: Option<&str>,
        conducting: bool,
    ) -> Result<Option<PlanAdmission>, String> {
        let ledger = self.delivery_state()?;
        let own = ledger
            .agents
            .iter()
            .rev()
            .find(|record| !record.ended && record.actor.same_identity(actor))
            .map(|record| record.id.as_str());
        // An agent a plan or a mission started does its own work (D-23).
        let delegated = own.is_some_and(|own| {
            ledger
                .spawns
                .iter()
                .any(|spawn| spawn.plan.is_some() && spawn.child.as_deref() == Some(own))
                || self.missions.book.missions.iter().any(|mission| {
                    mission.status.open()
                        && mission.nodes.iter().any(|node| {
                            node.agent_id.as_deref() == Some(own)
                                || node.criteria.iter().any(|line| {
                                    line.checkers
                                        .iter()
                                        .any(|checker| checker.agent_id.as_deref() == Some(own))
                                })
                        })
                })
        });
        if delegated {
            return Err("plan_delegate_not_permitted".into());
        }
        let Some(key) = plan_node else {
            return Ok(None);
        };
        // A mission's worker is the Conductor's, never a plan step's.
        if conducting {
            return Err("plan_delegate_not_permitted".into());
        }
        if parent.is_none() {
            return Err("plan_delegate_needs_parent".into());
        }
        if self.plans.load != Load::Ready {
            return Err("plan_unavailable".into());
        }
        let session = self
            .caller_session(device, &actor.pane_id, actor)
            .ok_or("plan_session_unknown")?;
        let plan = self.plans.book.get(&session).ok_or("plan_missing")?;
        let step = plan
            .steps
            .iter()
            .find(|step| step.key == key)
            .ok_or_else(|| refusal("plan_node_unknown", key))?;
        if plan.review == Review::Proposed {
            return Err("plan_review_pending".into());
        }
        if matches!(step.status, StepStatus::Done | StepStatus::Dropped) {
            return Err(refusal("plan_node_finished", key));
        }
        if let Some(blocked) = step.depends_on.iter().find(|wanted| {
            plan.steps.iter().any(|other| {
                &other.key == *wanted
                    && !matches!(other.status, StepStatus::Done | StepStatus::Dropped)
            })
        }) {
            return Err(refusal(
                "plan_order_refused",
                &format!("{key} waits on {blocked}"),
            ));
        }
        let id = plan.id();
        // The attempt the step is on: the same while its child works or has
        // reported, the next once it is blocked, closed or taken back. A
        // spawn the ledger holds and the plan has not followed yet is the
        // current one.
        let linked = ledger
            .spawns
            .iter()
            .filter_map(|spawn| spawn.plan.as_ref())
            .filter(|link| link.parent_session == session && link.plan == id && link.step == key)
            .map(|link| link.attempt)
            .max()
            .unwrap_or(0);
        let attempt = match step.delegation.as_ref() {
            Some(delegation) if linked > delegation.attempt => linked,
            Some(delegation)
                if delegation.released
                    || matches!(
                        delegation.state,
                        DelegationState::Blocked | DelegationState::Closed
                    ) =>
            {
                delegation.attempt + 1
            }
            Some(delegation) => delegation.attempt,
            None => linked.max(1),
        };
        let asleep = self.plan_children_away(&ledger, &session);
        Ok(Some(PlanAdmission {
            link: PlanLink {
                plan: id,
                step: key.to_owned(),
                attempt,
                parent_session: session,
            },
            asleep,
        }))
    }

    /// A plan step's child is held to Settings › Mission's worker
    /// candidates as they are now (I-2.9), by the item it starts with,
    /// picked into `pick` as a mission's worker is
    /// (`outside_candidates`).
    pub(crate) fn plan_candidates_admit(
        &self,
        actor: &crate::delivery::Actor,
        worker: &crate::runtime::missions::SpawnWorker<'_>,
        pick: &mut Option<crate::launch::Request>,
    ) -> Result<(), String> {
        let candidates = &self.snapshot.ui_state.conductor_settings.candidates;
        match self.outside_candidates(actor, worker, pick, candidates)? {
            Some(started) => Err(refusal("plan_candidate_refused", &started)),
            None => Ok(()),
        }
    }

    /// The children `session`'s plans handed steps to that are asleep or
    /// closed now, which hold no place under the cap (B5, B6): only a child
    /// whose sleep has landed, or whose pane was closed for Reopen closed
    /// (`closed_registrations.rs`). One whose end still runs, that is
    /// waking, or whose wake failed still holds its place, for the spawn and
    /// the waking letter alike.
    fn plan_children_away(
        &self,
        ledger: &crate::delivery::ledger::Ledger,
        session: &SessionIdentity,
    ) -> Vec<String> {
        let closed = self.closed_registration_ids();
        ledger
            .spawns
            .iter()
            .filter(|spawn| {
                spawn
                    .plan
                    .as_ref()
                    .is_some_and(|link| link.parent_session == *session)
            })
            .filter_map(|spawn| spawn.child.as_deref())
            .filter(|child| {
                ledger.agents.iter().any(|record| {
                    &record.id == child
                        && !record.ended
                        && (closed.contains(&record.id)
                            || self.asleep_route(&record.actor).is_some_and(|(_, stage)| {
                                stage == super::agent_sleep::letters::Stage::Sleeping
                            }))
                })
            })
            .map(str::to_owned)
            .collect()
    }

    /// Whether a letter from `actor` that would wake its sleeping plan child
    /// `recipient` may go (B6): only while its main's plans have a place for
    /// it under the cap.
    pub(crate) fn plan_wake_admission(
        &self,
        actor: &crate::delivery::Actor,
        recipient: &crate::delivery::Actor,
    ) -> Result<(), String> {
        let Ok(ledger) = self.delivery_state() else {
            return Ok(());
        };
        let Some(child) = ledger
            .agents
            .iter()
            .find(|record| !record.ended && record.actor.same_identity(recipient))
        else {
            return Ok(());
        };
        let Some(link) = ledger
            .spawns
            .iter()
            .find(|spawn| spawn.child.as_deref() == Some(child.id.as_str()))
            .and_then(|spawn| spawn.plan.as_ref())
        else {
            return Ok(());
        };
        if self
            .caller_session(&actor.device_id, &actor.pane_id, actor)
            .as_ref()
            != Some(&link.parent_session)
        {
            return Ok(());
        }
        let asleep = self.plan_children_away(&ledger, &link.parent_session);
        if crate::coordination::plan_children(
            &ledger,
            &link.parent_session,
            &asleep,
            super::memory::now_ms(),
        ) >= plans::PLAN_DELEGATE_LIMIT
        {
            crate::diagnostic!(json!({
                "component": "plans", "kind": "plans.wake_refused",
                "plan": link.plan, "step": link.step, "attempt": link.attempt,
                "agent_id": child.id,
            }));
            return Err(refusal(
                "plan_delegate_full",
                &plans::PLAN_DELEGATE_LIMIT.to_string(),
            ));
        }
        Ok(())
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

    /// Makes the plans view again when the book, the panes or the places of
    /// the children they handed steps to moved; with no plan and nothing
    /// shown it does nothing. A plan shown carries its children's plans
    /// too, whether or not their panes are shown, keyed by their sessions
    /// (B20).
    pub(super) fn refresh_plans(&mut self) {
        if self.plans.book.is_empty() && self.snapshot.plans.items.is_empty() {
            return;
        }
        let live = self.live_sessions();
        let ledger = self.delivery_state().ok();
        let children: Vec<String> = live
            .iter()
            .filter_map(|(_, session)| self.plans.book.get(session))
            .flat_map(|plan| &plan.steps)
            .filter_map(|step| step.delegation.as_ref().filter(|d| d.active()))
            .filter_map(|delegation| delegation.agent_id.clone())
            .collect();
        let places: Vec<ChildPlace> = children
            .iter()
            .map(|agent| self.child_place(ledger.as_deref(), agent))
            .collect();
        let print = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for (pane, session) in &live {
                if self.plans.book.contains_key(session) {
                    pane.hash(&mut hasher);
                    session.hash(&mut hasher);
                }
            }
            children.hash(&mut hasher);
            places.hash(&mut hasher);
            hasher.finish()
        };
        if self.plans.made_at == Some((self.plans.edit, print)) {
            return;
        }
        self.plans.made_at = Some((self.plans.edit, print));
        let mut items: Vec<PlanView> = live
            .iter()
            .filter_map(|(pane, session)| Some(self.plan_view(self.plans.book.get(session)?, pane)))
            .collect();
        for (pane, session) in places {
            let Some(session) = session else {
                continue;
            };
            if items.iter().any(|item| item.session == session) {
                continue;
            }
            if let Some(plan) = self.plans.book.get(&session) {
                items.push(self.plan_view(plan, pane.as_deref().unwrap_or_default()));
            }
        }
        self.snapshot.plans.items = items;
    }

    /// The plan of a mission node's worker, for its card and detail (D-17,
    /// D-18): found by its registration's session, so a finished node whose
    /// pane closed keeps the plan while the book still holds it.
    pub(super) fn worker_plan(
        &self,
        record: &crate::coordination::AgentRecord,
    ) -> Option<PlanView> {
        let (pane, session) = self.record_place(record);
        let plan = self.plans.book.get(&session?)?;
        Some(self.plan_view(plan, pane.as_deref().unwrap_or(&record.pane)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The summary the core makes of each contract plan is the one the
    /// hook reads (`contracts/delivery-envelope.json`, `plan_context`).
    #[test]
    fn the_plan_summary_matches_the_delivery_envelope_contract() {
        let contract: serde_json::Value =
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
                asked_at_unix_ms: None,
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

    /// What a full delivery ledger keeps for the plans
    /// (`coordination::room`): the spawn and child of a step still handed,
    /// and none of a finished step's or a taken-back one's; before the saved
    /// plans are read, every spawn a step started, with its child. A mission
    /// book not installed yet holds every spawn a mission started the same
    /// way.
    #[test]
    fn a_full_ledger_keeps_the_spawn_and_child_of_a_step_still_handed() {
        let mut runtime = crate::runtime::tests::runtime();
        let session = SessionIdentity {
            device_id: "mac".into(),
            agent: "claude".into(),
            session_id: "main".into(),
        };
        let step = |key: &str, status: &str, n: u64, released: bool| {
            serde_json::json!({
                "key": key, "title": key, "status": status,
                "delegation": {
                    "attempt": 1, "spawn": format!("spawn-{n}"), "name": "child",
                    "agent": "codex", "agent_id": format!("agent-{}", n + 1),
                    "state": "working", "released": released,
                },
            })
        };
        let plan = Plan {
            session: session.clone(),
            steps: serde_json::from_value(serde_json::json!([
                step("handed", "working", 10, false),
                step("finished", "done", 20, false),
                step("taken", "working", 30, true),
            ]))
            .unwrap(),
            review: Review::Approved,
            ask: None,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 1,
            asked_at_unix_ms: None,
        };
        runtime.plans.book.insert(session.clone(), plan);
        let spawn = |n: u64, link: serde_json::Value| -> crate::coordination::SpawnRecord {
            let mut value = serde_json::json!({
                "id": format!("spawn-{n}"), "parent": "agent-1", "intent": format!("i{n}"),
                "name": "child", "kind": "codex", "repo": "/repo", "branch": "b",
                "requested_path": null, "args": [], "path": null, "pane": null,
                "child": format!("agent-{}", n + 1),
            });
            value
                .as_object_mut()
                .unwrap()
                .extend(link.as_object().unwrap().clone());
            serde_json::from_value(value).unwrap()
        };
        let ledger = crate::delivery::ledger::Ledger {
            spawns: vec![
                spawn(
                    40,
                    serde_json::json!({"plan": {"plan": "1", "step": "s", "attempt": 1,
                    "parent_session": session}}),
                ),
                spawn(50, serde_json::json!({"mission": "mission-1"})),
                spawn(60, serde_json::json!({})),
            ],
            ..Default::default()
        };
        runtime.delivery_ledger = Ok(std::sync::Arc::new(ledger));
        let ids = |held: &crate::coordination::Held| {
            (
                held.spawns.iter().cloned().collect::<Vec<_>>(),
                held.agents.iter().cloned().collect::<Vec<_>>(),
            )
        };
        assert!(runtime.missions.store.is_none());
        runtime.plans.load = Load::Ready;
        assert_eq!(
            ids(&runtime.delivery_held()),
            (
                vec!["spawn-10".to_owned(), "spawn-50".to_owned()],
                vec!["agent-11".to_owned(), "agent-51".to_owned()]
            )
        );
        runtime.plans.load = Load::Idle;
        assert_eq!(
            ids(&runtime.delivery_held()),
            (
                vec!["spawn-40".to_owned(), "spawn-50".to_owned()],
                vec!["agent-41".to_owned(), "agent-51".to_owned()]
            )
        );
    }
}
