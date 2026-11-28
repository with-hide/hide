//! The result report a closed mission owes the agent that asked for it
//! (PRD device-connect-iroh D-72, B65).
//!
//! A Conductor an agent started with no parent names that agent as its
//! origin: an agent of this core, or the identity another machine's agent
//! stands as here. When its mission closes, by its own `hide mission
//! close` or by Hide once every node is done, the origin is sent the
//! mission's report as a `report` letter, the same on this machine as on
//! another; one for another machine waits in this ledger and goes over
//! the connection between the two cores once.
//!
//! The letter is the Conductor's own, prepared as a `hide letter send`
//! from its pane and sent by the rules every letter of that Conductor is
//! sent under, so nothing here admits or delivers.
//!
//! The debt is the mission's own record (`Mission::origin_report`), saved
//! with the close: a core that ends before the letter is kept reads it
//! again at its start. It is settled only by the letter being kept, or by
//! a refusal that will not change, which the Conductor is told. A store
//! that fails, or a Conductor not in its pane now, leaves it owed, and it
//! is tried again [`AGAIN_AFTER_MS`] later. The same close asked twice is
//! the same letter by its intent, so a letter kept just before the core
//! ended is not written a second time.

use crate::coordination::AgentRecord;
use crate::delivery::ledger::Ledger;
use crate::delivery::worker::Prepared;
use crate::delivery::{Actor, BODY_LIMIT, Command};
use crate::missions::{MissionStatus, OriginReport};
use crate::workspace_control::Query;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::Runtime;

/// How long one report's letter may take to be kept.
const SEND_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

/// How long a report that could not be written yet waits for its next try.
const AGAIN_AFTER_MS: u64 = 30_000;

/// The Conductor is not in its pane now (asleep, or its pane not read
/// yet): its letter is written once it is.
const CONDUCTOR_AWAY: &str = "conductor_away";

/// The Conductor ended with no agent after it: nobody writes its letter.
const CONDUCTOR_ENDED: &str = "conductor_ended";

/// Whether a report refused with `code` may still be written later.
fn later(code: &str) -> bool {
    code == CONDUCTOR_AWAY || crate::core_calls::passes(code)
}

/// What follows a report cut to fit one letter.
const CUT: &str = "\n\n(The report is cut here to fit one letter.)";

/// Who the Conductor `conductor` reports to, as a letter names it: the
/// agent that started it with no parent, by a registration id.
///
/// An agent of this core is named by the registration that runs in its
/// place now; one whose line ended with no agent after it hears nothing.
/// An agent of another machine's core is named by the registration this
/// ledger keeps of it, ended or not. That registration is the agent on the
/// connection its machine proved when it asked, so the report never reads
/// the machine's device id again: another connection that says the same id
/// neither makes it ambiguous nor takes it (`Runtime::core_send_target`),
/// and one paired no longer refuses it.
///
/// `None` for every other Conductor: one typed by the operator, delegated
/// under a parent, a fork, and one Hide or an external tool started.
pub(in crate::runtime) fn origin_target(ledger: &Ledger, conductor: &Actor) -> Option<String> {
    let record = registration(ledger, conductor)?;
    if record.forked {
        // A fork names the session it was forked from, which asked nothing.
        return None;
    }
    let origin = record.origin.as_deref()?;
    let kept = ledger.agents.iter().find(|agent| agent.id == origin)?;
    if kept.actor.core_address().is_some() {
        return Some(kept.id.clone());
    }
    let now = crate::coordination::live_successor(ledger, origin)?.ok()?;
    (!now.actor.code_owned()).then(|| now.id.clone())
}

/// The Conductor's newest registration, a live one first.
fn registration<'a>(ledger: &'a Ledger, conductor: &Actor) -> Option<&'a AgentRecord> {
    ledger
        .agents
        .iter()
        .rev()
        .filter(|record| record.actor.same_identity(conductor))
        .min_by_key(|record| record.ended)
}

/// The report letter for mission `id`, closed as `status` with `report`,
/// to `target`. Its intent names the close by what it said, so the same
/// close asked again is the same letter and a later close with another end
/// or report is a new one.
pub(in crate::runtime) fn letter(
    target: String,
    id: &str,
    title: &str,
    status: MissionStatus,
    report: Option<&str>,
) -> Command {
    let end = match status {
        MissionStatus::Done => "done",
        MissionStatus::MergeWait => "done, waiting for its merges",
        MissionStatus::Cancelled => "cancelled",
        MissionStatus::Planning | MissionStatus::Running => "open",
    };
    let report = report.map(str::trim).filter(|report| !report.is_empty());
    let mut body = match report {
        Some(report) => format!("Mission {id} \"{title}\" closed as {end}.\n\n{report}"),
        None => format!("Mission {id} \"{title}\" closed as {end}, with no report."),
    };
    if body.len() > BODY_LIMIT {
        let mut end = BODY_LIMIT - CUT.len();
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        body.truncate(end);
        body.push_str(CUT);
    }
    let said = format!("{:x}", Sha256::digest(body.as_bytes()));
    Command::Send {
        target,
        intent: format!("mission-report:{id}:{}", &said[..16]),
        body,
        kind: "report".into(),
        now: false,
        force: false,
    }
}

impl Runtime {
    /// Whether a close of a mission `conductor` runs owes its origin a
    /// report. A ledger that cannot be read says nothing of the origin, so
    /// the close owes one until it can.
    pub(super) fn owes_origin_report(&self, conductor: &Actor) -> bool {
        self.delivery_ledger
            .as_ref()
            .map_or(true, |ledger| origin_target(ledger, conductor).is_some())
    }

    /// Mission `id` closed: what its record owes is sent at the next tick.
    pub(super) fn owe_mission_report(&mut self, id: &str) {
        let owed = self
            .missions
            .book
            .get(id)
            .is_some_and(|mission| mission.origin_report == Some(OriginReport::Owed));
        let listed = &mut self.missions.reports_owed;
        if owed && !listed.iter().any(|listed| listed == id) {
            listed.push(id.to_owned());
        }
    }

    /// Sends the reports the closed missions owe, one after another on a
    /// thread of their own, each as its Conductor's own letter, and
    /// settles each by what became of it (`settle_mission_report`).
    pub(super) fn drive_mission_reports(&mut self, now: u64) {
        let missions = &self.missions;
        if missions.reports_owed.is_empty()
            || missions.reporting
            || now < missions.reports_again_unix_ms
        {
            return;
        }
        // A runtime without a worker (a test) sends nothing.
        let Some(runtime) = self
            .worker_context
            .as_ref()
            .map(|context| context.runtime.clone())
        else {
            return;
        };
        let mut letters = Vec::new();
        for id in self.missions.reports_owed.clone() {
            let close = self.mission_close(&id);
            match self.prepare_mission_report(&id) {
                Ok(Some(prepared)) => letters.push((id, close, prepared)),
                Ok(None) => self.settle_mission_report(&id, close, Ok(None), now),
                Err(code) => self.settle_mission_report(&id, close, Err(code), now),
            }
        }
        if letters.is_empty() {
            return;
        }
        struct Ending(std::sync::Weak<std::sync::Mutex<Runtime>>);
        impl Ending {
            fn with(&self, run: impl FnOnce(&mut Runtime)) {
                if let Some(runtime) = self.0.upgrade() {
                    run(&mut runtime.lock().unwrap_or_else(|poison| poison.into_inner()));
                }
            }
        }
        impl Drop for Ending {
            fn drop(&mut self) {
                self.with(|runtime| runtime.missions.reporting = false);
            }
        }
        let spawned = std::thread::Builder::new()
            .name("hide-mission-reports".to_owned())
            .spawn(move || {
                let ending = Ending(runtime);
                for (id, close, prepared) in letters {
                    let kept = prepared.run(SEND_WITHIN).and_then(|letter| {
                        let id = letter["id"].as_str().ok_or("letter_unavailable")?;
                        Ok(Some(id.to_owned()))
                    });
                    ending.with(|runtime| {
                        let now = super::unix_milliseconds();
                        runtime.settle_mission_report(&id, close, kept, now);
                    });
                }
            });
        match spawned {
            Ok(_) => self.missions.reporting = true,
            Err(error) => {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.report_thread_failed",
                    "reason": error.to_string(),
                }));
                self.missions.reports_again_unix_ms = now.saturating_add(AGAIN_AFTER_MS);
            }
        }
    }

    /// The close of mission `id` a report is written for: when it closed.
    fn mission_close(&self, id: &str) -> Option<u64> {
        self.missions.book.get(id)?.closed_at_unix_ms
    }

    /// The report letter closed mission `id` owes, ready to send as its
    /// Conductor's own; `None` when it owes none ([`origin_target`]).
    fn prepare_mission_report(&mut self, id: &str) -> Result<Option<Prepared>, String> {
        let Some(mission) = self.missions.book.get(id) else {
            return Ok(None);
        };
        if matches!(
            mission.status,
            MissionStatus::Planning | MissionStatus::Running
        ) {
            return Ok(None);
        }
        let conductor = mission.conductor.actor.clone();
        let ledger = self.delivery_state()?;
        let Some(target) = origin_target(&ledger, &conductor) else {
            return Ok(None);
        };
        // The Conductor's pane is its caller, as for a letter it sends
        // itself. One not there now writes it once it is back; one that
        // ended writes none.
        if !self.conductor_present(&conductor) {
            let lives = crate::coordination::registered_id(&ledger, &conductor).is_some();
            return Err(if lives {
                CONDUCTOR_AWAY
            } else {
                CONDUCTOR_ENDED
            }
            .into());
        }
        let command = letter(
            target,
            &mission.id,
            &mission.title,
            mission.status,
            mission.report.as_deref(),
        );
        let context = self
            .workspace_control_query(&conductor.device_id, &conductor.pane_id, Query::Info)
            .map_err(|_| CONDUCTOR_AWAY.to_owned())?
            .context;
        self.prepare_delivery(
            &conductor.device_id,
            &conductor.pane_id,
            &context,
            None,
            command,
        )
        .map(Some)
    }

    /// What became of the report mission `id` owes for the close at
    /// `close`: kept as a letter, owed by no one (`Ok(None)`), or refused.
    /// A refusal that may pass leaves the debt and sets the next try; any
    /// other is the report's end, saved on the mission and told to its
    /// Conductor, who can still write the letter itself. A mission closed
    /// again since owes that close's report, which this one does not
    /// settle.
    pub(super) fn settle_mission_report(
        &mut self,
        id: &str,
        close: Option<u64>,
        kept: Result<Option<String>, String>,
        now: u64,
    ) {
        let settled = match kept {
            Ok(letter) => letter.map(|letter| OriginReport::Sent { letter }),
            Err(code) if later(&code) => {
                crate::diagnostic!(json!({
                    "component": "missions", "kind": "mission.report_waits",
                    "mission": id, "code": code,
                }));
                self.missions.reports_again_unix_ms = now.saturating_add(AGAIN_AFTER_MS);
                return;
            }
            Err(reason) => Some(OriginReport::Refused { reason }),
        };
        let owed = self
            .missions
            .book
            .get_mut(id)
            .filter(|mission| mission.origin_report == Some(OriginReport::Owed));
        let Some(mission) = owed else {
            self.missions.reports_owed.retain(|owed| owed != id);
            return;
        };
        if mission.closed_at_unix_ms != close {
            return;
        }
        mission.origin_report = settled.clone();
        let conductor = mission.conductor.actor.clone();
        self.missions.reports_owed.retain(|owed| owed != id);
        self.save_missions();
        if let Some(OriginReport::Refused { reason }) = settled {
            crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.report_refused",
                "mission": id, "reason": reason,
            }));
            // News of a fact that does not change. A cell refused for room
            // is not owed again: the mission keeps the reason.
            let _ = self.post_status(
                &conductor,
                format!("mission-report:{id}"),
                format!(
                    "Hide could not send mission {id}'s result report to the agent that asked for it ({reason}). Send it yourself with `hide letter send <agent> --kind report` if it still matters."
                ),
                true,
                now,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::{CoreAddress, CoreItem};

    fn agent(pane: &str) -> Actor {
        Actor {
            pane_id: pane.into(),
            name: pane.into(),
            kind: "codex".into(),
            device_id: "local".into(),
            session: crate::engine::tokens::session_digest(pane),
        }
    }

    fn record(id: &str, actor: Actor, origin: Option<&str>) -> AgentRecord {
        AgentRecord {
            id: id.into(),
            name: Some(actor.name.clone()),
            machine: "local".into(),
            host_scope: "scope".into(),
            native_machine: "machine".into(),
            session: format!("{id}-session"),
            instance: format!("terminal-{id}"),
            pane: actor.pane_id.clone(),
            parent: None,
            origin: origin.map(str::to_owned),
            project: None,
            actor,
            ended: false,
            terminal_id: None,
            forked: false,
            successor: None,
        }
    }

    /// D-72: the origin is named by its registration, on this machine as
    /// on another; a Conductor nobody handed the work to, a fork, and one
    /// a tool or Hide started report to no one, and neither does one whose
    /// origin on this machine ended with no agent after it.
    #[test]
    fn a_conductor_reports_to_the_registration_of_the_agent_that_started_it() {
        let lead = agent("w1:p2");
        let there = Actor::core(
            CoreAddress::new("macbook", CoreItem::Agent, "agent-7"),
            &"a".repeat(64),
            "MacBook",
            "planner",
            "local",
        );
        let tool = Actor::access("00000000000000a1", "nightly", "local");
        let mut ledger = Ledger {
            agents: vec![
                record("agent-1", agent("w1:p1"), None),
                record("agent-2", there, None),
                record("agent-3", tool, None),
                record("agent-4", lead.clone(), None),
            ],
            ..Default::default()
        };
        assert_eq!(origin_target(&ledger, &lead), None, "typed by the operator");
        let mut started_by = |origin: &str, forked: bool| {
            ledger.agents[3].origin = Some(origin.into());
            ledger.agents[3].forked = forked;
            origin_target(&ledger, &lead)
        };
        assert_eq!(
            started_by("agent-1", false).as_deref(),
            Some("agent-1"),
            "an agent of this machine hears it as one of another does"
        );
        assert_eq!(
            started_by("agent-2", false).as_deref(),
            Some("agent-2"),
            "the registration kept of the agent that asked"
        );
        assert_eq!(started_by("agent-3", false), None, "a tool hears no letter");
        assert_eq!(
            started_by("agent-2", true),
            None,
            "a fork was asked nothing"
        );
        assert_eq!(started_by("agent-9", false), None, "an origin long gone");

        // The origin's conversation ended and another agent took its pane:
        // that agent is told. With no agent after it, nobody is.
        ledger.agents[3].origin = Some("agent-1".into());
        ledger.agents[3].forked = false;
        ledger.agents[0].ended = true;
        assert_eq!(origin_target(&ledger, &lead), None);
        ledger.agents[0].successor = Some("agent-5".into());
        ledger.agents.push(record("agent-5", agent("w1:p1"), None));
        assert_eq!(origin_target(&ledger, &lead).as_deref(), Some("agent-5"));
        // The kept registration of another machine's agent stays the
        // target once its machine is gone, so the refusal is the letter's.
        ledger.agents[3].origin = Some("agent-2".into());
        ledger.agents[1].ended = true;
        assert_eq!(origin_target(&ledger, &lead).as_deref(), Some("agent-2"));
    }

    /// The letter says which mission closed and how, carries the report,
    /// and is the same letter when the same close is asked again.
    #[test]
    fn the_report_letter_names_the_close_and_is_one_letter_per_close() {
        let report = |status, text: Option<&str>| match letter(
            "agent-1".into(),
            "mission-4",
            "Fix the bug",
            status,
            text,
        ) {
            Command::Send {
                target,
                intent,
                body,
                kind,
                ..
            } => {
                assert_eq!((target.as_str(), kind.as_str()), ("agent-1", "report"));
                (intent, body)
            }
            other => panic!("{other:?}"),
        };
        let (intent, body) = report(MissionStatus::Done, Some("Merged as #12.\n"));
        assert_eq!(
            body,
            "Mission mission-4 \"Fix the bug\" closed as done.\n\nMerged as #12."
        );
        assert!(intent.starts_with("mission-report:mission-4:"));
        assert_eq!(
            report(MissionStatus::Done, Some("Merged as #12.")).0,
            intent
        );
        assert_ne!(
            report(MissionStatus::Done, Some("Merged as #13.")).0,
            intent
        );
        assert_ne!(
            report(MissionStatus::Cancelled, Some("Merged as #12.")).0,
            intent
        );
        assert_eq!(
            report(MissionStatus::MergeWait, None).1,
            "Mission mission-4 \"Fix the bug\" closed as done, waiting for its merges, with no report."
        );
        // A report as long as a mission keeps still fits one letter.
        let long = "한".repeat(BODY_LIMIT / 3);
        let (_, body) = report(MissionStatus::Done, Some(&long));
        assert!(body.len() <= BODY_LIMIT);
        assert!(body.ends_with(CUT));
    }
}
