//! A mission's events and the operator's answer from the banner (PRD
//! mission-plan-visibility D-13, D-15, D-20, D-22, D-30): which change of
//! the record is an event, the decision a `decide` asks, the answer the
//! banner sends, and the events of the mission open in the 보기 slot.
//!
//! An answer is typed into the Conductor's conversation as the operator's
//! own input by the doorbell's thread, under the doorbell's rules: at rest,
//! or after it has worked for as long as a letter waits, never over a draft,
//! a menu or a question of its own (`delivery::doorbell::judge`).

use std::sync::Arc;

use serde::Serialize;
use serde_json::json;

use super::Runtime;
use crate::delivery::Actor;
use crate::mission_events::{AnswerSource, EventKind, EventLog, MissionEvent};
use crate::missions::{
    ANSWER_LIMIT, Answer, AnswerFailure, Mission, MissionNode, MissionStatus, Question,
};
use crate::model::DetailItem;

/// The events of the mission open in the 보기 slot (B18-B21, B31): sent
/// only for that one, so a mission's history never rides every frame.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MissionEventsSnapshot {
    pub mission: Option<String>,
    pub events: Vec<MissionEvent>,
    /// Events dropped past the limit before the first of these (B21).
    pub dropped: u64,
}

/// One answer for the doorbell to type: made under the lock, typed off it.
pub(crate) struct AnswerWork {
    pub mission: String,
    pub key: u64,
    pub conductor: Actor,
    pub at_unix_ms: u64,
    pub line: String,
}

/// The events one plan makes of the plan before it: the first plan, or a
/// replan that changed nodes, with the reason one of them was given (B31).
pub(crate) fn plan_events(before: &Mission, after: &Mission, now: u64) -> Vec<MissionEvent> {
    if before.nodes.is_empty() {
        if after.nodes.is_empty() {
            return Vec::new();
        }
        let mut event = MissionEvent::new(now, EventKind::Planned);
        event.count = u32::try_from(after.nodes.len()).ok();
        return vec![event];
    }
    let same = |a: &MissionNode, b: &MissionNode| {
        a.title == b.title
            && a.kind == b.kind
            && a.depends_on == b.depends_on
            && a.delivery == b.delivery
            && a.worker == b.worker
            && a.criteria == b.criteria
    };
    let changed = after
        .nodes
        .iter()
        .filter(|node| before.node(&node.key).is_none_or(|old| !same(old, node)))
        .count()
        + before
            .nodes
            .iter()
            .filter(|node| after.node(&node.key).is_none())
            .count();
    if changed == 0 {
        return Vec::new();
    }
    let reason = after.nodes.iter().find_map(|node| {
        node.change.as_deref().filter(|change| {
            before.node(&node.key).and_then(|old| old.change.as_deref()) != Some(change)
        })
    });
    let mut event = MissionEvent::new(now, EventKind::Replanned).line(reason);
    event.count = u32::try_from(changed).ok();
    vec![event]
}

/// The events one node change makes (B31): its new status, a further
/// attempt, a pull request it names for the first time, and its commit on
/// main. The same change again makes none.
pub(crate) fn node_events(
    before: &MissionNode,
    after: &MissionNode,
    now: u64,
) -> Vec<MissionEvent> {
    let mut out = Vec::new();
    if after.attempts > before.attempts && after.attempts >= 2 {
        let mut event = MissionEvent::new(now, EventKind::Retried).node(&after.key);
        event.count = Some(u32::from(after.attempts));
        out.push(event);
    }
    if after.status != before.status {
        let mut event = MissionEvent::new(now, EventKind::Node)
            .node(&after.key)
            .line(after.line.as_deref());
        event.status = Some(after.status);
        out.push(event);
    }
    if let Some(number) = after.pr
        && after.pr != before.pr
    {
        let mut event = MissionEvent::new(now, EventKind::PrOpened).node(&after.key);
        event.pr = Some(number);
        out.push(event);
    }
    if after.commit.is_some() && after.commit != before.commit {
        out.push(
            MissionEvent::new(now, EventKind::Landed)
                .node(&after.key)
                .line(after.commit.as_deref()),
        );
    }
    out
}

impl AnswerWork {
    /// An answer waits for a working Conductor as a letter does, from when
    /// the operator gave it (PRD letters-to-busy-agents D-04).
    pub(crate) fn patience(&self) -> crate::delivery::doorbell::Patience {
        crate::delivery::doorbell::Patience {
            arrived_at_unix_ms: self.at_unix_ms,
            now: false,
        }
    }
}

/// The line the doorbell types for an answer: the question it answers, so
/// the Conductor reads which decision it is (B26), then the answer.
fn answer_line(question: &str, answer: &str, language: crate::model::InterfaceLanguage) -> String {
    let question = crate::display_text::one_line(question, 200).unwrap_or_default();
    match language {
        crate::model::InterfaceLanguage::Korean => format!("[판단 답] {question} → {answer}"),
        crate::model::InterfaceLanguage::Japanese => {
            format!("[判断への回答] {question} → {answer}")
        }
        crate::model::InterfaceLanguage::SimplifiedChinese => {
            format!("[决定的回答] {question} → {answer}")
        }
        crate::model::InterfaceLanguage::English => {
            format!("[Decision answer] {question} → {answer}")
        }
    }
}

impl Runtime {
    pub(crate) fn install_mission_events(
        &mut self,
        store: crate::mission_events::MissionEventStore,
        logs: std::collections::HashMap<String, EventLog>,
    ) {
        self.missions.events = logs;
        self.missions.event_store = Some(store);
        self.missions.events_edit = self.missions.events_edit.wrapping_add(1);
    }

    /// Adds `events` to `mission`'s history and hands it to the store. A
    /// full history drops its oldest and logs it; recording never refuses
    /// the change it records (B21).
    pub(crate) fn record_mission_events(&mut self, mission: &str, events: Vec<MissionEvent>) {
        if events.is_empty() {
            return;
        }
        let log = self.missions.events.entry(mission.to_owned()).or_default();
        let before = log.dropped;
        for event in events {
            log.push(event);
        }
        // The first drop and each hundredth after it, so a full log does not
        // write a line per event.
        if log.dropped != before && (before == 0 || log.dropped / 100 != before / 100) {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission_events.dropped",
                "mission": mission, "dropped": log.dropped,
            }));
        }
        let saved = Arc::new(log.clone());
        self.missions.events_edit = self.missions.events_edit.wrapping_add(1);
        if self.missions.unreadable.is_none()
            && let Some(store) = &self.missions.event_store
        {
            store.save(mission, saved);
        }
    }

    /// Deletes the events of missions that are gone (D-22).
    pub(crate) fn forget_mission_events(&mut self, missions: &[String]) {
        for id in missions {
            self.missions.events.remove(id);
            if let Some(store) = &self.missions.event_store {
                store.remove(id);
            }
        }
        self.missions.events_edit = self.missions.events_edit.wrapping_add(1);
    }

    /// `hide mission decide`: sets, changes or clears the decision (B17,
    /// B29, B32). A new question gets a new key, so an answer to the one
    /// before it is refused; an answer from the banner not yet typed is
    /// dropped with it, and a question cleared without one is answered in
    /// the conversation.
    pub(super) fn decide_mission(
        &mut self,
        id: &str,
        line: Option<String>,
        asked: (Option<String>, Vec<String>, Option<u8>, Option<String>),
        now: u64,
    ) -> Result<bool, String> {
        let (why, options, recommend, node) = asked;
        let mission = self.missions.book.get(id).ok_or("mission_unknown")?;
        crate::missions::check_question(
            mission,
            line.as_deref(),
            why.as_deref(),
            &options,
            recommend,
            node.as_deref(),
        )?;
        let current = mission.question.as_ref();
        let same = mission.decision == line
            && current.is_some_and(|question| {
                question.why == why
                    && question.options == options
                    && question.recommend == recommend
                    && question.node == node
            })
            // A decision from before questions keeps its key while its line
            // stays and nothing is added beside it.
            || (mission.decision == line
                && current.is_none()
                && why.is_none()
                && options.is_empty()
                && recommend.is_none()
                && node.is_none());
        if same {
            return Ok(false);
        }
        let mut events = Vec::new();
        let key_before = mission.question.as_ref().map_or(0, |question| question.key);
        let asked = mission
            .question
            .as_ref()
            .and_then(|question| question.node.clone());
        if let Some(answer) = mission
            .answer
            .as_ref()
            .filter(|answer| answer.key == key_before)
        {
            if !answer.delivered {
                // The Conductor changed its question before reading this.
                let mut event =
                    MissionEvent::new(now, EventKind::AnswerUndelivered).line(Some(&answer.text));
                event.node = asked;
                events.push(event);
            }
        } else if line.is_none() && mission.decision.is_some() {
            let mut event = MissionEvent::new(now, EventKind::Answered);
            event.source = Some(AnswerSource::Conversation);
            event.node = asked;
            events.push(event);
        }
        let target = self.missions.book.get_mut(id).ok_or("mission_unknown")?;
        target.decision.clone_from(&line);
        target.answer = None;
        target.answer_failed = None;
        target.question = match &line {
            Some(question) => {
                target.question_seq += 1;
                let mut event = MissionEvent::new(now, EventKind::Asked).line(Some(question));
                event.node.clone_from(&node);
                event.options.clone_from(&options);
                events.push(event);
                Some(Question {
                    key: target.question_seq,
                    why,
                    options,
                    recommend,
                    node,
                    asked_at_unix_ms: now,
                })
            }
            None => None,
        };
        target.updated_at_unix_ms = now;
        self.missions.answers_held.remove(id);
        self.save_missions();
        self.record_mission_events(id, events);
        Ok(true)
    }

    /// The operator's answer from the banner (B26-B28): the choice it
    /// pressed, counted from 1, or what it wrote. Refused when the
    /// Conductor has since changed its question, when no Conductor or no
    /// device is there to read it; the same question answered again keeps
    /// the first answer.
    pub(super) fn answer_mission(
        &mut self,
        id: &str,
        key: u64,
        option: Option<u8>,
        text: Option<String>,
    ) -> Result<bool, String> {
        let now = super::memory::now_ms();
        let mission = self.missions.book.get(id).ok_or("mission_unknown")?;
        if !mission.status.open() || mission.decision.is_none() {
            return Err("mission_decision_changed".into());
        }
        let current = mission.question.as_ref().map_or(0, |question| question.key);
        if current != key {
            return Err("mission_decision_changed".into());
        }
        if mission
            .answer
            .as_ref()
            .is_some_and(|answer| answer.key == key)
        {
            return Ok(false);
        }
        // The doorbell types only into an agent it rings; nothing would ever
        // take this answer, and the banner offers none for it.
        if !crate::delivery::doorbell::bell_target(&mission.conductor.actor.kind) {
            return Err("mission_answer_unsupported".into());
        }
        if !self.conductor_present(&mission.conductor.actor) {
            return Err("mission_conductor_gone".into());
        }
        if self.mission_stale(mission) {
            return Err("mission_device_disconnected".into());
        }
        let text = match (option, text) {
            (Some(choice), None) => mission
                .question
                .as_ref()
                .and_then(|question| question.options.get(usize::from(choice).checked_sub(1)?))
                .cloned()
                .ok_or("mission_answer_invalid")?,
            (None, Some(text)) => {
                let text = text.trim().to_owned();
                if text.is_empty()
                    || text.chars().count() > ANSWER_LIMIT
                    || text.chars().any(char::is_control)
                {
                    return Err("mission_answer_invalid".into());
                }
                text
            }
            _ => return Err("mission_answer_invalid".into()),
        };
        let target = self.missions.book.get_mut(id).ok_or("mission_unknown")?;
        target.answer = Some(Answer {
            key,
            text,
            option,
            at_unix_ms: now,
            delivered: false,
        });
        target.answer_failed = None;
        crate::diagnostic!(json!({
            "component": "missions", "kind": "mission.answer_accepted",
            "mission": id, "key": key,
        }));
        self.save_missions();
        Ok(true)
    }

    /// The banner's answer as one event: a refusal the operator can act on
    /// stays beside the question as its reason; one whose question changed
    /// draws the new question instead, and goes only to the log.
    pub(super) fn mission_answer_event(
        &mut self,
        mission: &str,
        key: u64,
        option: Option<u8>,
        text: Option<String>,
    ) -> bool {
        match self.answer_mission(mission, key, option, text) {
            Ok(changed) => changed,
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.answer_refused",
                    "mission": mission, "key": key, "code": code,
                }));
                let shown = match code.as_str() {
                    "mission_conductor_gone" => Some(AnswerFailure::ConductorGone),
                    "mission_device_disconnected" => Some(AnswerFailure::DeviceDisconnected),
                    _ => None,
                };
                match (self.missions.book.get_mut(mission), shown) {
                    (Some(target), Some(failure)) => {
                        target.answer_failed = Some(failure);
                        self.save_missions();
                        true
                    }
                    _ => false,
                }
            }
        }
    }

    /// The answers the doorbell is to type now. An answer whose Conductor
    /// has been gone for `ABSENT_GRACE_MS` is dropped here: the banner goes
    /// back to before it was sent and the history says it was not delivered
    /// (D-30, B39). A shorter absence, such as the panes not yet read after
    /// a restart or a Herdr reconnect, only waits, as a letter does.
    pub(crate) fn mission_answers_due(&mut self) -> Vec<AnswerWork> {
        let now = super::memory::now_ms();
        let language = self.interface_language();
        let mut due = Vec::new();
        let mut gone = Vec::new();
        let mut absent = Vec::new();
        for mission in &self.missions.book.missions {
            let Some(answer) = mission.answer.as_ref().filter(|answer| !answer.delivered) else {
                continue;
            };
            let asked = mission
                .question
                .as_ref()
                .and_then(|question| question.node.clone());
            if !mission.status.open() {
                gone.push((mission.id.clone(), answer.text.clone(), asked));
                continue;
            }
            if !self.conductor_present(&mission.conductor.actor) {
                let since = self
                    .missions
                    .answer_absent_since
                    .get(&mission.id)
                    .copied()
                    .unwrap_or(now);
                if now.saturating_sub(since) >= ABSENT_GRACE_MS {
                    gone.push((mission.id.clone(), answer.text.clone(), asked));
                } else {
                    absent.push((mission.id.clone(), since));
                }
                continue;
            }
            due.push(AnswerWork {
                mission: mission.id.clone(),
                key: answer.key,
                conductor: mission.conductor.actor.clone(),
                at_unix_ms: answer.at_unix_ms,
                line: answer_line(
                    mission.decision.as_deref().unwrap_or_default(),
                    &answer.text,
                    language,
                ),
            });
        }
        // Only the missions still waiting on an absent Conductor keep the
        // time it went, so the map stays bounded by the open answers.
        self.missions.answer_absent_since = absent.into_iter().collect();
        for (id, text, asked) in gone {
            if let Some(mission) = self.missions.book.get_mut(&id) {
                mission.answer = None;
                mission.answer_failed = Some(AnswerFailure::ConductorGone);
            }
            self.missions.answers_held.remove(&id);
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.answer_undelivered",
                "mission": id, "reason": "conductor_gone",
            }));
            self.save_missions();
            let mut event = MissionEvent::new(now, EventKind::AnswerUndelivered).line(Some(&text));
            event.node = asked;
            self.record_mission_events(&id, vec![event]);
            self.announce_missions();
        }
        due
    }

    /// The final guard before the doorbell types an answer: the pane has not
    /// moved since its verdict and the answer still waits (`delivery_bell_current`).
    pub(crate) fn mission_answer_current(
        &self,
        work: &AnswerWork,
        observed: &crate::runtime::delivery::Observation,
    ) -> Result<(), crate::delivery::doorbell::Hold> {
        use crate::delivery::doorbell::{Hold, judge};
        let current = self.delivery_pane_unmoved(observed)?;
        let waiting = self
            .missions
            .book
            .get(&work.mission)
            .is_some_and(|mission| {
                mission.status.open()
                    && mission
                        .answer
                        .as_ref()
                        .is_some_and(|answer| answer.key == work.key && !answer.delivered)
            });
        if !waiting {
            return Err(Hold::Letter);
        }
        judge(current, work.patience(), super::memory::now_ms())
    }

    /// The doorbell held an answer: the banner shows it waits for the
    /// Conductor's turn (B28).
    pub(crate) fn mission_answer_held(&mut self, mission: &str) -> bool {
        let waiting = self.missions.book.get(mission).is_some_and(|mission| {
            mission.status.open()
                && mission
                    .answer
                    .as_ref()
                    .is_some_and(|answer| !answer.delivered)
        });
        if !waiting || !self.missions.answers_held.insert(mission.to_owned()) {
            return false;
        }
        self.missions.book_edit = self.missions.book_edit.wrapping_add(1);
        self.announce_missions();
        true
    }

    /// The doorbell's thread changed what a mission shows; a command's
    /// change is announced by the event that made it.
    fn announce_missions(&self) {
        if let Some(context) = &self.worker_context {
            context.notifier.notify();
        }
    }

    /// The doorbell typed the answer, or failed to: a typed answer is in the
    /// history as the operator's, from the banner; a failed one goes back
    /// with its reason so the operator can answer again (B28).
    pub(crate) fn mission_answer_sent(
        &mut self,
        id: &str,
        key: u64,
        result: Result<(), &'static str>,
    ) {
        let now = super::memory::now_ms();
        let Some(mission) = self.missions.book.get_mut(id) else {
            return;
        };
        let Some(answer) = mission
            .answer
            .as_mut()
            .filter(|answer| answer.key == key && !answer.delivered)
        else {
            return;
        };
        self.missions.answers_held.remove(id);
        let asked = mission
            .question
            .as_ref()
            .and_then(|question| question.node.clone());
        let mut event = match result {
            Ok(()) => {
                answer.delivered = true;
                let mut event =
                    MissionEvent::new(now, EventKind::Answered).line(Some(&answer.text));
                event.source = Some(AnswerSource::Banner);
                event
            }
            Err(code) => {
                let text = answer.text.clone();
                mission.answer = None;
                mission.answer_failed = Some(AnswerFailure::SendFailed);
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.answer_failed",
                    "mission": id, "key": key, "code": code,
                }));
                MissionEvent::new(now, EventKind::AnswerUndelivered).line(Some(&text))
            }
        };
        event.node = asked;
        self.save_missions();
        self.record_mission_events(id, vec![event]);
        self.announce_missions();
    }

    /// Makes the published events those of the mission open in the 보기
    /// slot, when it or its events moved.
    pub(super) fn refresh_mission_events(&mut self) {
        let slot = &self.snapshot.ui_state.detail;
        let open = slot
            .items
            .last()
            .filter(|_| slot.shown)
            .and_then(|item| match item {
                DetailItem::Mission { mission } => Some(mission.clone()),
                _ => None,
            });
        if self.missions.events_made == (open.clone(), self.missions.events_edit) {
            return;
        }
        let log = open
            .as_ref()
            .and_then(|id| self.missions.events.get(id))
            .cloned()
            .unwrap_or_default();
        let next = MissionEventsSnapshot {
            mission: open.clone(),
            events: log.events,
            dropped: log.dropped,
        };
        if self.snapshot.mission_events != next {
            self.snapshot.mission_events = next;
        }
        self.missions.events_made = (open, self.missions.events_edit);
    }
}

/// What a closed mission adds to its history: its report and, for a
/// cancelled one, the cancel (B24, B31).
pub(crate) fn close_events(before: &Mission, after: &Mission, now: u64) -> Vec<MissionEvent> {
    let mut out = Vec::new();
    if after.report.is_some() && after.report != before.report {
        let first = after
            .report
            .as_deref()
            .and_then(|report| report.lines().find(|line| !line.trim().is_empty()));
        out.push(MissionEvent::new(now, EventKind::Reported).line(first));
    }
    if after.status == MissionStatus::Cancelled && before.status != MissionStatus::Cancelled {
        out.push(MissionEvent::new(now, EventKind::Cancelled));
    }
    out
}

/// How long a Conductor may be missing before an answer waiting for it is
/// dropped as not delivered: longer than a restart takes to read the panes
/// again, or a Herdr reconnect.
const ABSENT_GRACE_MS: u64 = 30_000;

/// Ends a mission's decision with the mission (B29, D-30): clears the
/// question, the answer and why one failed, and records an answer not yet
/// typed as not delivered.
pub(crate) fn end_decision(mission: &mut Mission, now: u64) -> Vec<MissionEvent> {
    let asked = mission
        .question
        .as_ref()
        .and_then(|question| question.node.clone());
    let undelivered = mission
        .answer
        .take()
        .filter(|answer| !answer.delivered)
        .map(|answer| {
            let mut event =
                MissionEvent::new(now, EventKind::AnswerUndelivered).line(Some(&answer.text));
            event.node = asked;
            event
        });
    mission.decision = None;
    mission.question = None;
    mission.answer_failed = None;
    mission.delay = None;
    undelivered.into_iter().collect()
}

/// A node's pull request merged, as GitHub answered (B31).
pub(crate) fn merged_event(node: &MissionNode, number: u32, now: u64) -> MissionEvent {
    let mut event = MissionEvent::new(now, EventKind::PrMerged).node(&node.key);
    event.pr = Some(number);
    event
}
