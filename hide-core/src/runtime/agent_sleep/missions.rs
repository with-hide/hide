//! Agents put to sleep before their idle hours, by a mark the minute
//! decision spends: a finished mission node's worker and bound checkers (PRD
//! mission-worker-sleep B8-B13), a plan step's child (PRD hide-plan-delegate
//! B18), and a delegated child that has sent its parent its `report`. The
//! minute decision ends each marked agent that rests, off screen, under the
//! same cap as every other end. A mark is spent only when its end lands, so
//! an agent that started a turn between the check and the end is tried again
//! the next minute; a node opened again drops its marks, and a child's next
//! letter to its parent that is not a report, or a `request` or `block` sent
//! to it, drops its report's.

use std::collections::{HashMap, HashSet};

use serde_json::json;

use super::super::Runtime;
use super::ends_left;
use super::letters::Stage;
use crate::delivery::ledger::{Ledger, Letter};
use crate::missions::{Mission, MissionBook, MissionNode};
use crate::model::AgentActionRefusal;

/// What asked an agent to sleep.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum Origin {
    /// Its mission node finished.
    Node { mission: String, node: String },
    /// Its plan step was done or taken back by the plan's main.
    Plan { plan: String, step: String },
    /// Its report to its parent, the letter by id.
    Report { letter: String },
}

impl Origin {
    /// The sleep's trigger, as the `agent_sleep.ending` diagnostic names it.
    pub(in crate::runtime) fn trigger(&self) -> &'static str {
        match self {
            Self::Node { .. } => "mission",
            Self::Plan { .. } => "plan",
            Self::Report { .. } => "report",
        }
    }

    fn fields(&self) -> serde_json::Value {
        match self {
            Self::Node { mission, node } => json!({"mission": mission, "node": node}),
            Self::Plan { plan, step } => json!({"plan": plan, "step": step}),
            Self::Report { letter } => json!({"letter": letter}),
        }
    }
}

/// An agent asked to sleep at its next rest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct SleepMark {
    pub(in crate::runtime) agent_id: String,
    pub(in crate::runtime) origin: Origin,
    /// The pane whose end this mark started and that has not landed yet.
    ending: Option<String>,
}

impl SleepMark {
    /// Asks `agent_id` to sleep for `origin`, one mark per agent: a node's
    /// or a plan step's finish takes the place of a report's mark, which a
    /// later letter could drop, and any other mark already standing stays.
    /// Returns whether the marks changed.
    pub(in crate::runtime) fn ask(marks: &mut Vec<Self>, agent_id: &str, origin: Origin) -> bool {
        match marks.iter_mut().find(|mark| mark.agent_id == agent_id) {
            Some(mark)
                if matches!(mark.origin, Origin::Report { .. })
                    && !matches!(origin, Origin::Report { .. }) =>
            {
                mark.origin = origin;
                true
            }
            Some(_) => false,
            None => {
                marks.push(Self {
                    agent_id: agent_id.to_owned(),
                    origin,
                    ending: None,
                });
                true
            }
        }
    }

    /// The diagnostic `kind` says about this mark.
    fn event(&self, kind: &str) -> serde_json::Value {
        let mut event = json!({
            "component": "agent_sleep", "kind": kind, "agent_id": self.agent_id,
            "origin": self.origin.trigger(),
        });
        if let (Some(event), serde_json::Value::Object(fields)) =
            (event.as_object_mut(), self.origin.fields())
        {
            event.extend(fields);
        }
        event
    }
}

/// The sequence of a letter's id (`letter-<n>`); the ledger keeps its
/// letters in that order.
fn sequence(letter: &Letter) -> Option<u64> {
    letter.id.rsplit_once('-')?.1.parse().ok()
}

/// Whether a node is finished for its agents (D-17): its status is done or
/// dropped, however it got there; every pull request it holds was merged or
/// closed; or its mission was closed.
fn finished(mission: &Mission, node: &MissionNode) -> bool {
    !mission.status.open()
        || node.status.finished()
        || (!node.prs.is_empty()
            && node.prs.iter().all(|number| {
                node.seen(*number)
                    .is_some_and(|seen| seen.merged || seen.closed)
            }))
}

/// Every finished node of the book, by mission and node.
pub(in crate::runtime) fn finished_nodes(book: &MissionBook) -> HashSet<(String, String)> {
    book.missions
        .iter()
        .flat_map(|mission| {
            mission
                .nodes
                .iter()
                .filter(|node| finished(mission, node))
                .map(|node| (mission.id.clone(), node.key.clone()))
        })
        .collect()
}

/// The registrations a finished node puts to sleep: its worker and the
/// checkers bound to its lines, never its mission's Conductor.
fn node_agents<'a>(mission: &'a Mission, node: &'a MissionNode) -> impl Iterator<Item = &'a str> {
    let conductor = mission.conductor.agent_id.as_deref();
    node.agent_id
        .as_deref()
        .into_iter()
        .chain(node.checker_agents())
        .filter(move |id| Some(*id) != conductor)
}

/// What the minute decision does with a mark now.
enum Verdict {
    Sleep(String),
    Keep,
    Drop(&'static str),
}

impl Runtime {
    /// Marks the agents of each node the book now holds finished that it
    /// did not at the last save, and drops the marks of a node opened again
    /// (B10). Run by every save of the book.
    pub(in crate::runtime) fn note_mission_finishes(&mut self) {
        let now = finished_nodes(&self.missions.book);
        if now == self.missions.finished {
            return;
        }
        let missions = &mut self.missions;
        missions.sleep_marks.retain(|mark| match &mark.origin {
            Origin::Node { mission, node } => now.contains(&(mission.clone(), node.clone())),
            Origin::Plan { .. } | Origin::Report { .. } => true,
        });
        for mission in &missions.book.missions {
            for node in &mission.nodes {
                let key = (mission.id.clone(), node.key.clone());
                if !now.contains(&key) || missions.finished.contains(&key) {
                    continue;
                }
                for agent in node_agents(mission, node) {
                    let origin = Origin::Node {
                        mission: mission.id.clone(),
                        node: node.key.clone(),
                    };
                    if SleepMark::ask(&mut missions.sleep_marks, agent, origin) {
                        crate::diagnostic!(json!({
                            "component": "agent_sleep", "kind": "agent_sleep.mission_marked",
                            "mission": mission.id, "node": node.key, "agent_id": agent,
                        }));
                    }
                }
            }
        }
        missions.finished = now;
    }

    /// Reads the letters `next` holds that `previous` did not, in the order
    /// they were sent: a delegated child's `report` to its parent marks it
    /// to sleep, and its later letter to its parent of any other kind, or a
    /// letter sent to it that would wake it asleep
    /// (`Letter::wakes_a_sleeper`), drops that mark. Run by every
    /// publish of the ledger. The ledger this runtime started on is the
    /// first `previous`, so a restart marks nothing for a report it already
    /// held; with none (the ledger could not be read) nothing is new. A
    /// root, a spawn beside its caller (which never has a parent) and an
    /// open mission's Conductor are never marked.
    pub(in crate::runtime) fn note_reports(&mut self, previous: Option<&Ledger>, next: &Ledger) {
        let Some(first) = previous.map(|previous| previous.next_id) else {
            return;
        };
        let new = next
            .letters
            .iter()
            .rev()
            .take_while(|letter| sequence(letter).is_some_and(|sequence| sequence >= first))
            .collect::<Vec<_>>();
        for letter in new.into_iter().rev() {
            if letter.wakes_a_sleeper() {
                self.drop_report_marks(next, letter, |actor| letter.recipient.same_identity(actor));
            }
            let Some(child) = next.agents.iter().rev().find(|record| {
                !record.ended
                    && record.parent.is_some()
                    && record.actor.same_identity(&letter.sender)
            }) else {
                continue;
            };
            let to_parent = next.agents.iter().any(|record| {
                Some(&record.id) == child.parent.as_ref()
                    && record.actor.same_identity(&letter.recipient)
            });
            if !to_parent {
                continue;
            }
            if letter.kind != "report" {
                self.drop_report_marks(next, letter, |actor| child.actor.same_identity(actor));
                continue;
            }
            let conducts = self.missions.book.missions.iter().any(|mission| {
                mission.status.open() && mission.conductor.agent_id.as_deref() == Some(&child.id)
            });
            if conducts {
                continue;
            }
            let origin = Origin::Report {
                letter: letter.id.clone(),
            };
            if SleepMark::ask(&mut self.missions.sleep_marks, &child.id, origin) {
                crate::diagnostic!(json!({
                    "component": "agent_sleep", "kind": "agent_sleep.report_marked",
                    "agent_id": child.id, "letter": letter.id,
                }));
            }
        }
    }

    /// Drops the report marks of the registrations whose agent `addressed`
    /// names, for `letter`.
    fn drop_report_marks(
        &mut self,
        ledger: &Ledger,
        letter: &Letter,
        addressed: impl Fn(&crate::delivery::Actor) -> bool,
    ) {
        self.missions.sleep_marks.retain(|mark| {
            let dropped = matches!(mark.origin, Origin::Report { .. })
                && ledger
                    .agents
                    .iter()
                    .any(|record| record.id == mark.agent_id && addressed(&record.actor));
            if dropped {
                let mut event = mark.event("agent_sleep.report_unmarked");
                event["by"] = json!(letter.id);
                crate::diagnostic!(event);
            }
            !dropped
        });
    }

    /// Spends the mark whose end has just landed on `pane_id` (D-19): the
    /// agent is asleep, and its next wake is not undone by the same finish.
    pub(in crate::runtime) fn mission_sleep_landed(&mut self, pane_id: &str) {
        self.missions
            .sleep_marks
            .retain(|mark| mark.ending.as_deref() != Some(pane_id));
    }

    /// Puts the marked agents that rest to sleep (B8-B13), within the ends
    /// every other sleep counts. Run by the minute decision; a busy agent,
    /// one asking the operator or one on screen keeps its mark for the next
    /// minute, and an agent that can never sleep here loses it.
    pub(in crate::runtime) fn sleep_finished_mission_agents(&mut self, now_unix_ms: u64) -> bool {
        if self.missions.sleep_marks.is_empty() {
            return false;
        }
        if self.snapshot.ui_state.agent_sleep_after_hours.is_none() {
            // Sleep is off: a finish puts nothing to sleep (B11).
            self.missions.sleep_marks.clear();
            return false;
        }
        let Ok(ledger) = self.delivery_ledger.as_ref().map(std::sync::Arc::clone) else {
            return false;
        };
        let mut budget = ends_left(&self.snapshot.ui_state.agent_sleep);
        let mut on_screen: HashMap<Option<String>, HashSet<String>> = HashMap::new();
        let mut changed = false;
        let mut kept = Vec::new();
        for mut mark in std::mem::take(&mut self.missions.sleep_marks) {
            let Some(actor) = ledger
                .agents
                .iter()
                .find(|record| record.id == mark.agent_id && !record.ended)
                .map(|record| record.actor.clone())
            else {
                continue;
            };
            if let Some(pane) = mark.ending.take() {
                match self.asleep_route(&actor).map(|(_, stage)| stage) {
                    // Its end still runs.
                    Some(Stage::Ending) => {
                        mark.ending = Some(pane);
                        kept.push(mark);
                        continue;
                    }
                    // It slept: the mark is spent (B12).
                    Some(_) => continue,
                    // It slept by closing its pane and woke since into
                    // another, which carried its registration: spent too.
                    None if actor.pane_id != pane => continue,
                    // Its end was refused: it is tried again below.
                    None => {}
                }
            }
            let verdict = self.mission_sleep_verdict(&actor, &mut on_screen);
            match verdict {
                Verdict::Keep => kept.push(mark),
                Verdict::Drop(why) => {
                    let mut event = mark.event("agent_sleep.mark_skipped");
                    event["reason"] = json!(why);
                    crate::diagnostic!(event);
                }
                Verdict::Sleep(_) if budget == 0 => kept.push(mark),
                Verdict::Sleep(pane) => {
                    budget -= 1;
                    changed |= self.begin_agent_sleep(&pane, mark.origin.trigger(), now_unix_ms);
                    if self.asleep_route(&actor).is_some() {
                        mark.ending = Some(pane);
                    }
                    kept.push(mark);
                }
            }
        }
        // A save during the pass may have marked more; those keep their place behind.
        kept.append(&mut self.missions.sleep_marks);
        self.missions.sleep_marks = kept;
        changed
    }

    /// Whether the agent `actor` registers may be put to sleep now for its
    /// mark (D-18): the pane menu's rules, off the operator's screen, the
    /// same session still in its pane.
    fn mission_sleep_verdict(
        &self,
        actor: &crate::delivery::Actor,
        on_screen: &mut HashMap<Option<String>, HashSet<String>>,
    ) -> Verdict {
        if actor.code_owned() {
            return Verdict::Drop("code_owned");
        }
        if self.asleep_route(actor).is_some() {
            return Verdict::Drop("asleep");
        }
        let pane = &actor.pane_id;
        let device = crate::address::pane_device(pane);
        let machine = self.sleep_machine_of_pane(pane);
        if device.is_none() && actor.device_id != self.node.as_str() {
            return Verdict::Drop("device");
        }
        let Some(agent) = self
            .sleep_agents(device)
            .iter()
            .find(|agent| &agent.pane_id == pane)
        else {
            // A machine that does not answer now may still list it later.
            if device.is_some_and(|device| self.device_engine(device).is_none()) {
                return Verdict::Keep;
            }
            return Verdict::Drop("pane_gone");
        };
        if agent
            .session_id
            .as_deref()
            .and_then(crate::engine::tokens::session_digest)
            != actor.session
        {
            return Verdict::Drop("another_session");
        }
        match crate::agent_sleep::sleep_refusal(agent, machine) {
            Some(
                AgentActionRefusal::Working
                | AgentActionRefusal::WaitingForYou
                | AgentActionRefusal::UnknownActivity,
            ) => return Verdict::Keep,
            Some(AgentActionRefusal::Unsupported) => return Verdict::Drop("unsupported"),
            Some(AgentActionRefusal::CoreMachineOnly) => return Verdict::Drop("core_machine_only"),
            Some(AgentActionRefusal::Asleep) => return Verdict::Drop("asleep"),
            Some(AgentActionRefusal::NoConversation) => return Verdict::Drop("no_conversation"),
            None => {}
        }
        // A child still moving under it would report to it asleep, and a
        // report wakes no one. A root's row already reads that as working
        // (`children_moving`); a delegated row's does not.
        if agent.descendant_counts.working > 0 {
            return Verdict::Keep;
        }
        let shown = on_screen
            .entry(device.map(str::to_owned))
            .or_insert_with(|| self.agent_sleep_on_screen(device));
        if shown.contains(pane) || self.pane_move_holds_pane(pane) {
            return Verdict::Keep;
        }
        Verdict::Sleep(pane.clone())
    }
}
