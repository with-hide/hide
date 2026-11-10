//! A node's human gate at run time (PRD mission-human-gate D-02, D-09-D-12,
//! D-21-D-24): when a gate is reached, the decision the core raises for
//! it, the one entry the operator's answer passes or reopens it through,
//! whether it came from the decision card, the phone or the Conductor's
//! pane, and the gate's state as the map reads it.
//!
//! A gate is a node's `human` line (`mission_criteria`), so the core's
//! refusal of a node's done while a line is unmet already holds the node
//! and every node behind it. Nothing here runs for a node without one: a
//! mission with no gate is read, raised and settled as before (D-24).
//!
//! Everything runs under the lock and reads only what the core holds: the
//! book, the lines' heads as `node_unmet` judges them and the Conductor's
//! presence. No process, file or network is touched; the save and the
//! letters go through the book's save thread and the mission wakes.

use serde::Serialize;
use serde_json::{Value, json};

use super::Runtime;
use crate::delivery::Actor;
use crate::mission_criteria::{CriterionKind, CriterionState, GATE_KEY, GateVia};
use crate::mission_events::{AnswerSource, EventKind, MissionEvent};
use crate::missions::{
    AttentionReason, Mission, MissionNode, MissionStatus, NodePatch, NodeStatus, PendingWake,
    Question, WakeEvent, answer_text,
};
use crate::model::InterfaceLanguage;

/// How long a reached gate waits before the map says it has waited long
/// (D-22): nothing passes or is sent on its own then.
pub(crate) const LONG_WAIT_MS: u64 = 24 * 60 * 60_000;

/// The operator's answer to a gate (D-11).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GateAnswer {
    /// `계속 진행`: the line passes.
    Pass,
    /// `고칠 점 있음` or a written answer: the words go to the node's worker
    /// as a defect and the gate stays open.
    Fix,
}

/// Where a node's gate stands, derived from its line and the open decision
/// (D-09), never stored apart.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateState {
    /// Its node's other lines are not all met yet.
    Ahead,
    /// Reached: the node waits on the operator's answer.
    Open,
    /// The operator passed it.
    Passed,
}

/// A node's gate as the map, the phone and `hide mission show` read it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GateView {
    pub state: GateState,
    /// What the operator looks at, as the plan names it.
    pub look: String,
    /// The key of the mission's decision that asks it now; none while it
    /// waits for the decision to be free, or once it passed.
    pub ask: Option<u64>,
    /// When it was asked, while it is open and asked: the waiting time.
    pub since_unix_ms: Option<u64>,
    /// When it has waited long (`오래 기다림`, D-22): the shell compares it
    /// with its own clock, so no timer of the core's moves it.
    pub long_wait_at_unix_ms: Option<u64>,
    /// Where the operator's pass came from, and their words.
    pub via: Option<GateVia>,
    pub said: Option<String>,
}

/// The two fixed choices of a gate's decision, in the interface language;
/// `직접 답하기` is the card's written answer, which every decision has.
fn gate_options(language: InterfaceLanguage) -> Vec<String> {
    let (pass, fix) = match language {
        InterfaceLanguage::Korean => ("계속 진행", "고칠 점 있음"),
        InterfaceLanguage::Japanese => ("このまま進める", "直す点がある"),
        InterfaceLanguage::SimplifiedChinese => ("继续进行", "有需要修改的地方"),
        InterfaceLanguage::English => ("Go on", "Something to fix"),
    };
    vec![pass.to_owned(), fix.to_owned()]
}

/// A gate's question (D-16, B16): what to look at, then a request to check
/// it and report back.
fn gate_question(look: &str, language: InterfaceLanguage) -> String {
    match language {
        InterfaceLanguage::Korean => format!("{look} 확인하고 알려 주세요"),
        InterfaceLanguage::Japanese => format!("{look}を確認して知らせてください"),
        InterfaceLanguage::SimplifiedChinese => format!("请查看{look}后告诉我"),
        InterfaceLanguage::English => format!("Check {look} and tell me"),
    }
}

/// Whether `question` is the one `node`'s gate is asked by.
fn asks(question: Option<&Question>, node: &MissionNode) -> bool {
    let Some(line) = crate::mission_criteria::gate(&node.criteria) else {
        return false;
    };
    question.is_some_and(|question| {
        line.asked == Some(question.key) && question.node.as_deref() == Some(node.key.as_str())
    })
}

/// The node whose gate the mission's open decision asks, if it asks one.
pub(super) fn gate_asked(mission: &Mission) -> Option<&MissionNode> {
    let question = mission.question.as_ref()?;
    mission.decision.as_ref()?;
    mission.nodes.iter().find(|node| asks(Some(question), node))
}

impl Runtime {
    /// Whether `node`'s gate is reached now (D-10): it works, its gate is
    /// open, and every other line is met, the CI GitHub has not reported
    /// included, as the done refusal judges them.
    pub(super) fn gate_reached(&self, mission: &Mission, node: &MissionNode) -> bool {
        node.status == NodeStatus::Working
            && crate::mission_criteria::gate(&node.criteria)
                .is_some_and(|line| line.state != CriterionState::Pass)
            && self.node_unmet(mission, node) == [GATE_KEY]
    }

    /// Whether `node`'s gate waits on the operator now: reached, or asked by
    /// the open decision, in a mission that runs.
    fn gate_open(&self, mission: &Mission, node: &MissionNode) -> bool {
        matches!(
            mission.status,
            MissionStatus::Running | MissionStatus::MergeWait
        ) && node.status == NodeStatus::Working
            && crate::mission_criteria::gate(&node.criteria)
                .is_some_and(|line| line.state != CriterionState::Pass)
            && (asks(mission.question.as_ref(), node) || self.gate_reached(mission, node))
    }

    /// `node`'s gate as the map reads it; none for a node without one.
    pub(super) fn gate_view(&self, mission: &Mission, node: &MissionNode) -> Option<GateView> {
        let line = crate::mission_criteria::gate(&node.criteria)?;
        let look = line.text.clone().unwrap_or_default();
        if line.state == CriterionState::Pass {
            return Some(GateView {
                state: GateState::Passed,
                look,
                ask: None,
                since_unix_ms: None,
                long_wait_at_unix_ms: None,
                via: line.via,
                said: line.evidence.clone(),
            });
        }
        let open = self.gate_open(mission, node);
        let asked = mission
            .question
            .as_ref()
            .filter(|_| open && asks(mission.question.as_ref(), node));
        // Asked or not, a gate no Conductor stands for is waited on from the
        // raise that makes it the mission's own item.
        let since = asked.map(|question| question.asked_at_unix_ms).or_else(|| {
            node.attention
                .as_ref()
                .filter(|attention| open && attention.reason == AttentionReason::HumanGate)
                .map(|attention| attention.since_unix_ms)
        });
        Some(GateView {
            state: if open {
                GateState::Open
            } else {
                GateState::Ahead
            },
            look,
            ask: asked.map(|question| question.key),
            since_unix_ms: since,
            long_wait_at_unix_ms: since.map(|since| since.saturating_add(LONG_WAIT_MS)),
            via: None,
            said: None,
        })
    }

    /// The one entry the operator's answer to a gate goes through (D-11,
    /// D-12, D-21), whether it came from the decision card, the phone or
    /// the Conductor relaying their words from its pane: `Pass` passes the
    /// line with the words and where they came from; `Fix` leaves it open
    /// and sends the words to the node's worker as a defect, so the node
    /// works again and its gate is raised anew once its lines are met. The
    /// gate's decision and its raise go with either, and the Conductor is
    /// told. A gate not waiting on the operator, or an answer with no
    /// words, is refused with its reason and passes nothing.
    pub(super) fn settle_gate(
        &mut self,
        id: &str,
        key: &str,
        answer: GateAnswer,
        via: GateVia,
        words: &str,
        now: u64,
    ) -> Result<bool, String> {
        let words = match answer_text(words) {
            Some(words) => words,
            None if words.trim().is_empty() => return Err("mission_gate_words_missing".into()),
            None => return Err("mission_answer_invalid".into()),
        };
        let mission = self.missions.book.get(id).ok_or("mission_unknown")?;
        match mission.status {
            MissionStatus::Cancelled => return Err("mission_cancelled".into()),
            MissionStatus::Done => return Err("mission_closed".into()),
            _ => {}
        }
        let at = mission
            .nodes
            .iter()
            .position(|node| node.key == key)
            .ok_or("mission_node_unknown")?;
        let node = &mission.nodes[at];
        let line = crate::mission_criteria::gate(&node.criteria)
            .ok_or_else(|| crate::coordination::refusal("mission_gate_none", key))?;
        // The same pass again changes nothing (engineering principle 11).
        if line.state == CriterionState::Pass {
            return match answer {
                GateAnswer::Pass => Ok(false),
                GateAnswer::Fix => Err(crate::coordination::refusal("mission_gate_not_open", key)),
            };
        }
        if !self.gate_open(mission, node) {
            return Err(crate::coordination::refusal("mission_gate_not_open", key));
        }
        let asked = asks(mission.question.as_ref(), node)
            .then(|| mission.question.as_ref().map_or(0, |question| question.key));
        let before = node.clone();
        let mut next = node.clone();
        let mut events = Vec::new();
        let mut answered = MissionEvent::new(now, EventKind::Answered)
            .node(key)
            .line(Some(&words));
        answered.source = Some(match via {
            GateVia::Card | GateVia::Phone => AnswerSource::Banner,
            GateVia::Pane => AnswerSource::Conversation,
        });
        answered.criterion = Some(GATE_KEY.to_owned());
        events.push(answered);
        let defect = match answer {
            GateAnswer::Pass => {
                let line = next
                    .criteria
                    .iter_mut()
                    .find(|line| line.kind == CriterionKind::Human)
                    .ok_or("mission_gate_none")?;
                line.pass_gate(words.clone(), via, now);
                None
            }
            GateAnswer::Fix => {
                let defect = crate::mission_criteria::push_defect(
                    &mut next.criteria,
                    words.clone(),
                    None,
                    crate::mission_criteria::operator(now),
                )?;
                if let Some(line) = next.criteria.iter_mut().find(|line| line.key == defect) {
                    line.via = Some(via);
                }
                if let Some(line) = next
                    .criteria
                    .iter_mut()
                    .find(|line| line.kind == CriterionKind::Human)
                {
                    line.asked = None;
                }
                next.reopened = next.reopened.saturating_add(1);
                if next.status != NodeStatus::Working {
                    next = crate::missions::apply_patch(
                        &next,
                        &NodePatch {
                            status: Some(NodeStatus::Working),
                            ..NodePatch::default()
                        },
                        now,
                    )?;
                }
                let mut event = MissionEvent::new(now, EventKind::Reopened)
                    .node(key)
                    .line(Some(&words));
                event.count = Some(u32::from(next.reopened));
                events.push(event);
                events.extend(
                    super::mission_events::node_events(&before, &next, now)
                        .into_iter()
                        .filter(|event| event.kind == EventKind::Node),
                );
                Some(defect)
            }
        };
        // Its raise goes with the answer, and is raised again by the next
        // arrival rather than held back as one already handled.
        super::mission_health::settle(id, &mut next, "operator");
        let cause = crate::missions::cause_key(AttentionReason::HumanGate, None, None);
        next.handled.retain(|handled| *handled != cause);
        let target = self.missions.book.get_mut(id).ok_or("mission_unknown")?;
        if let Some(asked) = asked {
            super::mission_criteria::note_decided(target, asked, Some(key));
            target.decision = None;
            target.question = None;
            target.answer = None;
            target.answer_failed = None;
        }
        let worker = next.agent_id.clone();
        target.nodes[at] = next;
        target.updated_at_unix_ms = now;
        let gate_wake = |to: Option<String>| PendingWake {
            seq: 0,
            node: key.to_owned(),
            pr: None,
            event: match answer {
                GateAnswer::Pass => WakeEvent::GatePassed,
                GateAnswer::Fix => WakeEvent::GateFix,
            },
            head: None,
            to,
            reason: None,
            why: None,
            action: None,
            warning: None,
            ci: None,
            key: Some(format!(
                "gate:{key}:{}:{}:{now}",
                match answer {
                    GateAnswer::Pass => "pass",
                    GateAnswer::Fix => "fix",
                },
                via.as_str()
            )),
        };
        let mut wakes = vec![gate_wake(None)];
        // The words reach the node's worker themselves, so a gate answered
        // with no Conductor there still sends the node back to its work.
        if answer == GateAnswer::Fix
            && let Some(worker) = worker
        {
            wakes.push(gate_wake(Some(worker)));
        }
        for mut wake in wakes {
            target.wake_seq += 1;
            wake.seq = target.wake_seq;
            super::missions::push_wake(target, wake);
        }
        self.missions.answers_held.remove(id);
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.gate_settled",
            "mission": id, "node": key, "via": via.as_str(), "defect": defect,
            "answer": match answer { GateAnswer::Pass => "pass", GateAnswer::Fix => "fix" },
        }));
        self.record_mission_events(id, events);
        self.flush_mission_wakes();
        Ok(true)
    }

    /// `hide mission gate` (D-21, B8, B21): the Conductor relays the
    /// operator's answer given in its pane, with their words quoted. Only
    /// the Conductor relays, and never without the words: the gate is the
    /// operator's, and the record keeps what they said.
    pub(super) fn relay_gate(
        &mut self,
        actor: &Actor,
        named: Option<&str>,
        node: &str,
        (pass, said): (bool, String),
        now: u64,
    ) -> Result<(Value, bool), String> {
        let ledger = self.delivery_state().ok();
        let caller = super::mission_criteria::Caller::of(
            actor,
            ledger.as_deref(),
            &self.missions.book.missions,
        );
        let id = match named {
            Some(id) => self
                .missions
                .book
                .get(id)
                .ok_or("mission_unknown")?
                .id
                .clone(),
            None => self
                .missions
                .book
                .conducted_open(actor)
                .ok_or("mission_none")?
                .id
                .clone(),
        };
        let mission = self.missions.book.get(&id).ok_or("mission_unknown")?;
        if !mission.conductor.actor.same_identity(actor) {
            return Err("mission_not_conductor".into());
        }
        let answer = if pass {
            GateAnswer::Pass
        } else {
            GateAnswer::Fix
        };
        let changed = self.settle_gate(&id, node, answer, GateVia::Pane, &said, now)?;
        Ok((
            json!({"mission": self.mission_view_for(&id, &caller)}),
            changed,
        ))
    }

    /// Brings `id`'s gates to where its nodes stand (D-10, D-17, D-23),
    /// after each health evaluation: a decision asking a gate that no
    /// longer waits goes; a reached gate asks the operator through the
    /// mission's decision once that is free, its two fixed choices and the
    /// Views linked as a `decide` links them; and with no Conductor there a
    /// gate that waits is the mission's own raise to the operator, which a
    /// Conductor coming back takes off again, since the decision on its row
    /// says it. A mission without a gate is left as it is. Whether the book
    /// changed.
    pub(super) fn raise_gates(&mut self, id: &str, now: u64) -> bool {
        let Some(mission) = self.missions.book.get(id) else {
            return false;
        };
        let gated = mission
            .nodes
            .iter()
            .any(|node| crate::mission_criteria::gate(&node.criteria).is_some());
        if !gated || self.mission_stale(mission) {
            return false;
        }
        let present = mission.status.open() && self.conductor_present(&mission.conductor.actor);
        let stale_ask = gate_asked(mission)
            .filter(|node| !self.gate_open(mission, node))
            .map(|node| node.key.clone());
        let reached: Vec<String> = mission
            .nodes
            .iter()
            .filter(|node| self.gate_reached(mission, node))
            .map(|node| node.key.clone())
            .collect();
        let open: Vec<String> = mission
            .nodes
            .iter()
            .filter(|node| self.gate_open(mission, node))
            .map(|node| node.key.clone())
            .collect();
        let mut changed = false;
        let mut events = Vec::new();
        if let Some(key) = stale_ask {
            let target = self.missions.book.get_mut(id).expect("read above");
            target.decision = None;
            target.question = None;
            target.answer = None;
            target.answer_failed = None;
            if let Some(line) = target
                .nodes
                .iter_mut()
                .find(|node| node.key == key)
                .and_then(|node| {
                    node.criteria
                        .iter_mut()
                        .find(|line| line.kind == CriterionKind::Human)
                })
            {
                line.asked = None;
            }
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.gate_withdrawn",
                "mission": id, "node": key,
            }));
            changed = true;
        }
        let free = self
            .missions
            .book
            .get(id)
            .is_some_and(|mission| mission.decision.is_none() && mission.status.open());
        if free && let Some(key) = reached.first() {
            changed |= self.ask_gate(id, key, now, &mut events);
        }
        let target = self.missions.book.get_mut(id).expect("read above");
        let cause = crate::missions::cause_key(AttentionReason::HumanGate, None, None);
        for node in &mut target.nodes {
            let raised = node
                .attention
                .as_ref()
                .is_some_and(|attention| attention.reason == AttentionReason::HumanGate);
            if raised && (present || !open.contains(&node.key)) {
                super::mission_health::settle(id, node, "hide");
                node.handled.retain(|handled| *handled != cause);
                changed = true;
            }
        }
        if !present {
            for key in &open {
                let raised = super::mission_health::raise(
                    target,
                    key,
                    AttentionReason::HumanGate,
                    None,
                    now,
                );
                changed |= !raised.is_empty();
                events.extend(raised);
            }
        }
        if changed {
            target.updated_at_unix_ms = now;
            self.record_mission_events(id, events);
            self.save_missions();
        }
        changed
    }

    /// Asks the operator about node `key`'s gate as the mission's decision
    /// (D-04, D-16, B5, B16): `<what to look at> 확인하고 알려 주세요` with
    /// `계속 진행` and `고칠 점 있음`, in the interface language, linked to
    /// the Views the Conductor and the node's worker opened as a `decide`
    /// for that node links them.
    fn ask_gate(&mut self, id: &str, key: &str, now: u64, events: &mut Vec<MissionEvent>) -> bool {
        let language = self.interface_language();
        let Some(mission) = self.missions.book.get(id) else {
            return false;
        };
        let Some(look) = mission
            .node(key)
            .and_then(|node| crate::mission_criteria::gate(&node.criteria))
            .and_then(|line| line.text.clone())
        else {
            return false;
        };
        let conductor = mission.conductor.actor.clone();
        let views = match self.question_view_links(id, &conductor, &[], Some(key), true) {
            Ok(views) => views,
            Err(code) => {
                // A link that cannot be made never keeps the gate from
                // asking: the operator is asked with nothing linked.
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.gate_views_failed",
                    "mission": id, "node": key, "code": code,
                }));
                Vec::new()
            }
        };
        let Some(target) = self.missions.book.get_mut(id) else {
            return false;
        };
        let line = gate_question(&look, language);
        let options = gate_options(language);
        target.question_seq += 1;
        let asked = target.question_seq;
        let mut event = MissionEvent::new(now, EventKind::Asked).line(Some(&line));
        event.node = Some(key.to_owned());
        event.options.clone_from(&options);
        events.push(event);
        target.decision = Some(line);
        target.answer = None;
        target.answer_failed = None;
        target.question = Some(Question {
            key: asked,
            why: None,
            options,
            recommend: None,
            node: Some(key.to_owned()),
            asked_at_unix_ms: now,
            views,
            material_asked: false,
        });
        if let Some(line) = target
            .nodes
            .iter_mut()
            .find(|node| node.key == key)
            .and_then(|node| {
                node.criteria
                    .iter_mut()
                    .find(|line| line.kind == CriterionKind::Human)
            })
        {
            line.asked = Some(asked);
        }
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.gate_asked",
            "mission": id, "node": key, "question": asked,
        }));
        true
    }

    /// The operator's answer from the card or the phone to a gate's
    /// decision, which `answer_mission` hands here: the first choice
    /// passes, the second or a written answer sends its words back as a
    /// defect (D-11). Answered whether or not a Conductor is there, since
    /// the core settles it itself.
    pub(super) fn answer_gate(
        &mut self,
        id: &str,
        node: &str,
        (option, text): (Option<u8>, Option<String>),
        via: GateVia,
    ) -> Result<bool, String> {
        let now = super::memory::now_ms();
        let options = self
            .missions
            .book
            .get(id)
            .and_then(|mission| mission.question.as_ref())
            .map(|question| question.options.clone())
            .unwrap_or_default();
        let (answer, words) = match (option, text) {
            (Some(1), None) => (GateAnswer::Pass, options.first().cloned()),
            (Some(2), None) => (GateAnswer::Fix, options.get(1).cloned()),
            (None, Some(text)) => (GateAnswer::Fix, Some(text)),
            _ => return Err("mission_answer_invalid".into()),
        };
        let words = words.ok_or("mission_answer_invalid")?;
        self.settle_gate(id, node, answer, via, &words, now)
    }
}

/// The newest defect the operator's answer to `node`'s gate wrote, with its
/// key: what a gate's fix letter quotes.
pub(super) fn operator_defect(node: &MissionNode) -> Option<(&str, &str)> {
    node.criteria
        .iter()
        .filter(|line| {
            line.kind == CriterionKind::Defect && line.found_by.is_none() && line.via.is_some()
        })
        .max_by_key(|line| line.by.as_ref().map_or(0, |by| by.at_unix_ms))
        .map(|line| (line.key.as_str(), line.text.as_deref().unwrap_or_default()))
}
