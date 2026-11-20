//! The result report a closed mission owes the agent that asked for it
//! (PRD device-connect-iroh D-72, B65).
//!
//! A Conductor another machine's agent started here with no parent names
//! that agent as its origin: the identity that core's agent stands as
//! here. When its mission closes, by its own `hide mission close` or by
//! Hide once every node is done, the origin is sent the mission's report
//! as a `report` letter, which waits in this ledger and goes over the
//! connection between the two cores once. An origin on this machine is
//! sent none: the agent that started an independent Conductor here handed
//! the work to the operator, who sees the mission close.
//!
//! The letter is the Conductor's own, prepared as a `hide request send`
//! from its pane and sent by the rules every letter of that Conductor is
//! sent under, so nothing here admits or delivers.

use crate::coordination::AgentRecord;
use crate::delivery::ledger::Ledger;
use crate::delivery::worker::Prepared;
use crate::delivery::{Actor, BODY_LIMIT, Command};
use crate::missions::MissionStatus;
use crate::workspace_control::Query;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::Runtime;

/// How long one report's letter may take to be kept.
const SEND_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

/// What follows a report cut to fit one letter.
const CUT: &str = "\n\n(The report is cut here to fit one letter.)";

/// Who the Conductor `conductor` reports to, as a letter names it: the
/// agent of another machine's core that started it with no parent, by its
/// address on its own core. `None` for every other Conductor: one typed by
/// the operator, delegated under a parent, a fork, and one an agent of this
/// machine, Hide or an external tool started.
pub(in crate::runtime) fn origin_target(ledger: &Ledger, conductor: &Actor) -> Option<String> {
    let record = registration(ledger, conductor)?;
    if record.forked {
        // A fork names the session it was forked from, which asked nothing.
        return None;
    }
    let origin = record.origin.as_deref()?;
    let origin = ledger.agents.iter().find(|agent| agent.id == origin)?;
    origin
        .actor
        .core_address()
        .map(|address| address.to_string())
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
    /// Mission `id` closed: its report is owed at the next tick.
    pub(super) fn owe_mission_report(&mut self, id: &str) {
        if !self.missions.reports_owed.iter().any(|owed| owed == id) {
            self.missions.reports_owed.push(id.to_owned());
        }
    }

    /// Sends the reports the closed missions owe, one after another on a
    /// thread of their own, each as its Conductor's own letter. A report
    /// that cannot be sent (no Conductor in its pane, a recipient that is
    /// gone) is recorded and dropped: its mission keeps the report.
    pub(super) fn drive_mission_reports(&mut self) {
        if self.missions.reports_owed.is_empty() || self.missions.reporting {
            return;
        }
        let owed = std::mem::take(&mut self.missions.reports_owed);
        // A runtime without a worker (a test) sends nothing.
        let Some(runtime) = self
            .worker_context
            .as_ref()
            .map(|context| context.runtime.clone())
        else {
            return;
        };
        let letters: Vec<_> = owed
            .into_iter()
            .filter_map(|id| match self.prepare_mission_report(&id) {
                Ok(prepared) => prepared.map(|prepared| (id, prepared)),
                Err(code) => {
                    unsent(&id, &code);
                    None
                }
            })
            .collect();
        if letters.is_empty() {
            return;
        }
        struct Ending(std::sync::Weak<std::sync::Mutex<Runtime>>);
        impl Drop for Ending {
            fn drop(&mut self) {
                if let Some(runtime) = self.0.upgrade() {
                    runtime
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .missions
                        .reporting = false;
                }
            }
        }
        let spawned = std::thread::Builder::new()
            .name("hide-mission-reports".to_owned())
            .spawn(move || {
                let _ending = Ending(runtime);
                for (id, prepared) in letters {
                    if let Err(code) = prepared.run(SEND_WITHIN) {
                        unsent(&id, &code);
                    }
                }
            });
        match spawned {
            Ok(_) => self.missions.reporting = true,
            Err(error) => crate::diagnostic!(json!({
                "component": "missions", "kind": "mission.report_thread_failed",
                "reason": error.to_string(),
            })),
        }
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
        let command = letter(
            target,
            &mission.id,
            &mission.title,
            mission.status,
            mission.report.as_deref(),
        );
        // The Conductor's pane is its caller, as for a letter it sends
        // itself: one that is gone sends nothing.
        let context = self
            .workspace_control_query(&conductor.device_id, &conductor.pane_id, Query::Info)
            .map_err(|refusal| refusal.reason.to_owned())?
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
}

fn unsent(mission: &str, code: &str) {
    crate::diagnostic!(json!({
        "component": "missions", "kind": "mission.report_unsent",
        "mission": mission, "code": code,
    }));
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
        }
    }

    /// D-72: the origin on another machine is named by its address there;
    /// a Conductor nobody handed the work to, one an agent of this machine
    /// started, a fork, and one a tool or Hide started report to no one.
    #[test]
    fn a_conductor_reports_to_the_agent_of_another_machine_that_started_it() {
        let lead = agent("w1:p2");
        let there = Actor::core(
            CoreAddress::new("macbook", CoreItem::Agent, "agent-7"),
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
            started_by("agent-1", false),
            None,
            "an agent of this machine handed the work to the operator"
        );
        assert_eq!(
            started_by("agent-2", false).as_deref(),
            Some("core:macbook:agent:agent-7")
        );
        assert_eq!(started_by("agent-3", false), None, "a tool hears no letter");
        assert_eq!(
            started_by("agent-2", true),
            None,
            "a fork was asked nothing"
        );
        assert_eq!(started_by("agent-9", false), None, "an origin long gone");
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
