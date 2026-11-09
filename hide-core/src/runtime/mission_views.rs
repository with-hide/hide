//! What a mission's decision shows the operator to look at (operator
//! decision 2026-11-09, after mission-42's 보기 칸 decision asked the operator
//! to look at captures its card did not link): the Views linked to the
//! question, each a fact recorded when the View was named or opened, never
//! a guess from a name or a time window.
//!
//! A View is linked, in this order, when:
//! 1. the Conductor names it with `hide mission decide --show` (the CLI opens
//!    a path as a View first and sends its id);
//! 2. the Conductor opened it since the mission's question before was asked,
//!    or since the mission was created, and it is still open when `decide`
//!    runs: kept on the mission record, so a restart loses none and no agent
//!    runtime has to report its turns;
//! 3. the stopped node's worker, or an agent under it, opened it in the
//!    node's current attempt.
//!
//! The map lists the linked Views still open, seen or not; with none the
//! operator can ask the Conductor for one page to look at (`자료 요청`),
//! once per question, which never holds the answer back.

use serde::Serialize;
use serde_json::json;

use super::Runtime;
use super::workspace_view::WorkspaceKey;
use crate::delivery::Actor;
use crate::missions::{Mission, ViewLink, WakeEvent, push_view_link};

/// How deep an opener's parents are walked to find the node's worker.
const LINEAGE_LIMIT: usize = 32;

/// Who linked a View to the decision, as its row names it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionViewBy {
    Conductor,
    Worker,
}

/// A View linked to a mission's open decision that is still open, as the
/// decision's card lists it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DecisionView {
    pub device_id: String,
    /// The checkout whose View list holds it.
    pub checkout_path: String,
    pub display_id: String,
    pub kind: crate::view_layout::DisplayKind,
    /// Its name as its tab says it.
    pub label: String,
    /// The operator has not looked at it since it was opened.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unseen: bool,
    pub by: DecisionViewBy,
}

impl Runtime {
    /// An agent's `hide file|diff|browser open` from `pane` landed in View
    /// `view_id` of `key`: a mission's Conductor keeps it for its next
    /// question, and a node's worker, or an agent under it, links it to the
    /// node's attempt.
    pub(super) fn link_mission_view(&mut self, key: &WorkspaceKey, view_id: &str, pane: &str) {
        if self.missions.book.missions.is_empty() {
            return;
        }
        let pane = crate::address::named_pane_id(self.node.as_str(), &key.0, pane);
        let Some(actor) = self
            .delivery_observations
            .get(&pane)
            .map(|observation| observation.actor.clone())
        else {
            return;
        };
        let link = ViewLink {
            device_id: key.0.clone(),
            checkout_path: key.1.clone(),
            display_id: view_id.to_owned(),
            attempt: 0,
        };
        let lineage = self.lineage_of(&actor);
        let mut changed = false;
        for index in 0..self.missions.book.missions.len() {
            let mission = &self.missions.book.missions[index];
            if !mission.status.open() {
                continue;
            }
            if mission.conductor.actor.same_identity(&actor) {
                let mission = &mut self.missions.book.missions[index];
                changed |= push_view_link(&mut mission.conductor_views, link.clone());
                continue;
            }
            let Some(node) = mission.nodes.iter().position(|node| {
                node.status == crate::missions::NodeStatus::Working
                    && node
                        .agent_id
                        .as_deref()
                        .is_some_and(|worker| lineage.iter().any(|id| id == worker))
            }) else {
                continue;
            };
            let target = &mut self.missions.book.missions[index].nodes[node];
            let attempt = ViewLink {
                attempt: target.attempts,
                ..link.clone()
            };
            changed |= push_view_link(&mut target.views, attempt);
        }
        if changed {
            self.save_missions();
        }
    }

    /// The registration `actor` runs as and every parent above it, nearest
    /// first: whose node's attempt a View it opens belongs to.
    fn lineage_of(&self, actor: &Actor) -> Vec<String> {
        let Ok(ledger) = self.delivery_state() else {
            return Vec::new();
        };
        let mut lineage = Vec::new();
        let mut next = ledger
            .agents
            .iter()
            .find(|record| !record.ended && record.actor.same_identity(actor));
        while let Some(record) = next {
            if lineage.len() >= LINEAGE_LIMIT || lineage.contains(&record.id) {
                break;
            }
            lineage.push(record.id.clone());
            next = record
                .parent
                .as_deref()
                .and_then(|parent| ledger.agents.iter().find(|record| record.id == parent));
        }
        lineage
    }

    /// The Views a question of mission `id` shows: the ones `--show` names
    /// in the Conductor's checkout, then, for a new question (`fresh`), the
    /// ones the Conductor opened since the question before that are still
    /// open, which it takes off the mission. An id no View of that checkout
    /// has is refused, and then nothing is taken.
    pub(super) fn question_view_links(
        &mut self,
        id: &str,
        actor: &Actor,
        show: &[String],
        fresh: bool,
    ) -> Result<Vec<ViewLink>, String> {
        let mut links = Vec::new();
        if !show.is_empty() {
            // The checkout the Conductor's pane works in, else the one the
            // mission was created in.
            let place = self
                .conductor_checkout(actor)
                .or_else(|| {
                    let mission = self.missions.book.get(id)?;
                    Some((mission.device_id.clone(), mission.checkout_path.clone()))
                })
                .ok_or("mission_show_unknown")?;
            let layout = self.view_layout_of(&place).ok_or("mission_show_unknown")?;
            for view in show {
                layout.display(view).ok_or("mission_show_unknown")?;
                push_view_link(
                    &mut links,
                    ViewLink {
                        device_id: place.0.clone(),
                        checkout_path: place.1.clone(),
                        display_id: view.clone(),
                        attempt: 0,
                    },
                );
            }
        }
        if fresh {
            let opened = self
                .missions
                .book
                .get_mut(id)
                .map(|mission| std::mem::take(&mut mission.conductor_views))
                .unwrap_or_default();
            for link in opened {
                let key = (link.device_id.clone(), link.checkout_path.clone());
                let open = self
                    .view_layout_of(&key)
                    .is_some_and(|layout| layout.display(&link.display_id).is_some());
                if open {
                    push_view_link(&mut links, link);
                }
            }
        }
        Ok(links)
    }

    /// The checkout the Conductor's pane works in, whose Views `--show`
    /// names, while the catalog holds that pane.
    fn conductor_checkout(&self, actor: &Actor) -> Option<WorkspaceKey> {
        let pane = crate::address::named_pane_id(self.node.as_str(), &actor.device_id, &actor.pane_id);
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == actor.device_id)
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| {
                checkout.tabs.iter().any(|tab| {
                    tab.panes.iter().any(|known| {
                        crate::address::named_pane_id(self.node.as_str(), &actor.device_id, &known.id)
                            == pane
                    })
                })
            })
            .map(|checkout| (actor.device_id.clone(), checkout.path.clone()))
    }

    /// The Views `mission`'s open decision lists: the question's own, then
    /// the stopped node's current attempt's, each once and only while it is
    /// open.
    pub(super) fn decision_views(&self, mission: &Mission) -> Vec<DecisionView> {
        let Some(question) = mission.decision.as_ref().and(mission.question.as_ref()) else {
            return Vec::new();
        };
        let node = question
            .node
            .as_deref()
            .and_then(|key| mission.node(key));
        let links = question
            .views
            .iter()
            .map(|link| (link, DecisionViewBy::Conductor))
            .chain(node.into_iter().flat_map(|node| {
                node.views
                    .iter()
                    .filter(move |link| link.attempt == node.attempts)
                    .map(|link| (link, DecisionViewBy::Worker))
            }));
        let mut views: Vec<DecisionView> = Vec::new();
        for (link, by) in links {
            if views.iter().any(|view| {
                view.device_id == link.device_id
                    && view.checkout_path == link.checkout_path
                    && view.display_id == link.display_id
            }) {
                continue;
            }
            let key = (link.device_id.clone(), link.checkout_path.clone());
            let Some(display) = self
                .view_layout_of(&key)
                .and_then(|layout| layout.display(&link.display_id))
            else {
                continue;
            };
            views.push(DecisionView {
                device_id: link.device_id.clone(),
                checkout_path: link.checkout_path.clone(),
                display_id: link.display_id.clone(),
                kind: display.kind,
                label: super::view_areas::display_label(display),
                unseen: display.unseen.is_some(),
                by,
            });
        }
        views
    }

    /// What a decision's card compares to know its Views moved: the View
    /// layouts' change count while an open decision links any, else zero,
    /// so a mission with none pays nothing for a View's change.
    pub(super) fn decision_views_generation(&self) -> u64 {
        let linked = self.missions.book.missions.iter().any(|mission| {
            mission.decision.is_some()
                && mission.question.as_ref().is_some_and(|question| {
                    !question.views.is_empty()
                        || question
                            .node
                            .as_deref()
                            .and_then(|key| mission.node(key))
                            .is_some_and(|node| !node.views.is_empty())
                })
        });
        if !linked {
            return 0;
        }
        self.workspace_views
            .as_ref()
            .map_or(0, |store| store.generation)
    }

    /// The operator's `자료 요청` on question `key` of mission `id`: the
    /// Conductor is asked once for one page to look at, and asks again with
    /// it. Asking twice, or about a question it changed since, sends nothing.
    pub(super) fn ask_decision_material(&mut self, id: &str, key: u64) -> Result<bool, String> {
        let mission = self.missions.book.get(id).ok_or("mission_unknown")?;
        if !mission.status.open() || mission.decision.is_none() {
            return Err("mission_decision_changed".into());
        }
        let question = mission.question.as_ref().ok_or("mission_decision_changed")?;
        if question.key != key {
            return Err("mission_decision_changed".into());
        }
        if question.material_asked {
            return Ok(false);
        }
        if !self.conductor_present(&mission.conductor.actor) {
            return Err("mission_conductor_gone".into());
        }
        let now = super::memory::now_ms();
        let target = self.missions.book.get_mut(id).ok_or("mission_unknown")?;
        let node = target
            .question
            .as_ref()
            .and_then(|question| question.node.clone())
            .unwrap_or_default();
        if let Some(question) = target.question.as_mut() {
            question.material_asked = true;
        }
        target.updated_at_unix_ms = now;
        let intent = super::missions::record_wake(
            target,
            node,
            None,
            WakeEvent::MaterialAsked,
            None,
            Some(format!("material-{key}")),
        );
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.material_asked",
            "mission": id, "question": key, "intent": intent,
        }));
        self.save_missions();
        self.flush_mission_wakes();
        Ok(true)
    }

    pub(super) fn mission_material_event(&mut self, id: &str, key: u64) -> bool {
        self.ask_decision_material(id, key).unwrap_or_else(|code| {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.material_refused",
                "mission": id, "question": key, "code": code,
            }));
            false
        })
    }
}
