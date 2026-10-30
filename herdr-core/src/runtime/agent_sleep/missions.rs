//! A finished mission node's agents put to sleep (PRD mission-worker-sleep
//! B8-B13): the save that tells a node has just finished marks its worker
//! and its bound checkers, and the minute decision ends each marked agent
//! that rests, off screen, under the same cap as every other end. A mark is
//! spent only when its end lands, so an agent that started a turn between
//! the check and the end is tried again the next minute; a node opened
//! again drops its marks.

use std::collections::{HashMap, HashSet};

use serde_json::json;

use super::super::Runtime;
use super::letters::Stage;
use super::{MAX_ENDS_IN_FLIGHT, pane_device};
use crate::agent_sleep::{SleepMachine, SleepPhase};
use crate::missions::{Mission, MissionBook, MissionNode};
use crate::model::AgentActionRefusal;

/// An agent a node's finish asks to put to sleep.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct SleepMark {
    pub(in crate::runtime) agent_id: String,
    mission: String,
    node: String,
    /// The pane whose end this mark started and that has not landed yet.
    ending: Option<String>,
}

/// Whether a node is finished for its agents (D-17): its status is done,
/// failed or dropped, however it got there; its pull request was merged or
/// closed; or its mission was closed.
fn finished(mission: &Mission, node: &MissionNode) -> bool {
    !mission.status.open()
        || node.status.finished()
        || node
            .pr_seen
            .as_ref()
            .is_some_and(|seen| seen.merged || seen.closed)
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
        .chain(
            node.criteria
                .iter()
                .flat_map(|line| line.checkers.iter())
                .filter_map(|checker| checker.agent_id.as_deref()),
        )
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
        missions
            .sleep_marks
            .retain(|mark| now.contains(&(mark.mission.clone(), mark.node.clone())));
        for mission in &missions.book.missions {
            for node in &mission.nodes {
                let key = (mission.id.clone(), node.key.clone());
                if !now.contains(&key) || missions.finished.contains(&key) {
                    continue;
                }
                for agent in node_agents(mission, node) {
                    if missions
                        .sleep_marks
                        .iter()
                        .any(|mark| mark.agent_id == agent)
                    {
                        continue;
                    }
                    crate::diagnostic!(json!({
                        "component": "agent_sleep", "kind": "agent_sleep.mission_marked",
                        "mission": mission.id, "node": node.key, "agent_id": agent,
                    }));
                    missions.sleep_marks.push(SleepMark {
                        agent_id: agent.to_owned(),
                        mission: mission.id.clone(),
                        node: node.key.clone(),
                        ending: None,
                    });
                }
            }
        }
        missions.finished = now;
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
        let store = &self.snapshot.ui_state.agent_sleep;
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
        let mut budget = MAX_ENDS_IN_FLIGHT.saturating_sub(in_flight);
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
                    // Its end was refused: it is tried again below.
                    None => {}
                }
            }
            let verdict = self.mission_sleep_verdict(&actor, &mut on_screen);
            match verdict {
                Verdict::Keep => kept.push(mark),
                Verdict::Drop(why) => crate::diagnostic!(json!({
                    "component": "agent_sleep", "kind": "agent_sleep.mission_skipped",
                    "mission": mark.mission, "node": mark.node, "agent_id": mark.agent_id,
                    "reason": why,
                })),
                Verdict::Sleep(_) if budget == 0 => kept.push(mark),
                Verdict::Sleep(pane) => {
                    budget -= 1;
                    changed |= self.begin_agent_sleep(&pane, "mission", now_unix_ms);
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
    /// node's finish (D-18): the pane menu's rules, off the operator's
    /// screen, the same session still in its pane.
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
        let device = pane_device(pane);
        let machine = self.sleep_machine_of_pane(pane);
        if machine == SleepMachine::Device
            || device.is_none() && actor.device_id != self.node.as_str()
        {
            return Verdict::Drop("device");
        }
        let Some(agent) = self
            .sleep_agents(device)
            .iter()
            .find(|agent| &agent.pane_id == pane)
        else {
            return Verdict::Drop("pane_gone");
        };
        if agent
            .session_id
            .as_deref()
            .and_then(crate::wire::session_digest)
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
            Some(AgentActionRefusal::Device) => return Verdict::Drop("device"),
            Some(AgentActionRefusal::Unsupported) => return Verdict::Drop("unsupported"),
            Some(AgentActionRefusal::CoreMachineOnly) => return Verdict::Drop("core_machine_only"),
            Some(AgentActionRefusal::Asleep) => return Verdict::Drop("asleep"),
            Some(AgentActionRefusal::NoConversation) => return Verdict::Drop("no_conversation"),
            None => {}
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
