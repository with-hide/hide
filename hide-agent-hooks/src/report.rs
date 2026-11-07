//! Reporting a pane's agent to the core: its turn and its subagent counts.
//!
//! The report is one hook request to hided (`HookCommand::Status`, which
//! `hide workspace status --hook` also sends), which proves the caller is in
//! the pane like every other hook request and hands the report to the core
//! (PRD engine-bundle-1 D-7). The request ends
//! within [`REPORT_TIMEOUT`], so a daemon that is gone or does not answer
//! costs the agent's turn that long and no more (B13). A report nothing
//! answered for waits in the state folder's outbox until that machine's
//! daemon or node hands it on ([`crate::outbox`]).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::runtime::{AgentRuntime, HookEvent};

/// How long one report may hold the agent's hook slot.
pub const REPORT_TIMEOUT: Duration = Duration::from_secs(2);

/// What the helper leaves the agent of its hook's time: the process ends
/// and the agent reads its answer within it.
const EXIT_MARGIN: Duration = Duration::from_millis(500);

/// When a report of the hook started at `started` for `event` must end: at
/// most [`REPORT_TIMEOUT`] from `now`, and inside the time the agent gives
/// the hook (`install::hook_timeout`, what the entry asks for), counted
/// from the hook's start rather than from the work before the report.
pub fn deadline(started: Instant, runtime: AgentRuntime, event: HookEvent) -> Instant {
    let budget = Duration::from_secs(crate::install::hook_timeout(runtime, event));
    (Instant::now() + REPORT_TIMEOUT).min(started + budget.saturating_sub(EXIT_MARGIN))
}

/// One hook event's report on the pane's agent, as the core keeps it.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HookStatusReport {
    /// The adapter id of the runtime that reported.
    pub runtime: String,
    /// The hook protocol the helper speaks (`HOOK_VERSION`).
    pub version: u32,
    /// The agent's own id of the session the event came from, when the
    /// hook's input named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The turn this event says the session is in; absent for an event that
    /// says nothing about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<HookTurn>,
    /// The session's subagent counts after this event; absent for an event
    /// whose counts are not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<HookCounts>,
    /// The event a session start is: a new session's report clears the
    /// turn the pane's earlier session left.
    #[serde(default)]
    pub session_start: bool,
    /// The calls whose permission the operator has not answered yet
    /// (`counters::Said::Asks`, `hide_session::calls::key`), oldest first:
    /// the session waits on the operator while any is open, whatever its
    /// turn says. Absent for an event that says nothing of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<Vec<String>>,
    /// When the hook made the report, in microseconds of its machine's
    /// clock, which [`report`] stamps: a report replayed from the outbox
    /// is applied only when it is newer than the session's last, so one
    /// sent twice changes the core once and an older one never undoes a
    /// later one. Absent from a hook older than the outbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_unix_us: Option<u64>,
}

/// Where a session's turn stands, as its hook saw it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookTurn {
    /// A prompt was submitted, or a tool runs.
    Working,
    /// The turn ended.
    Done,
    /// The agent asks the operator for a permission or an answer.
    Waiting,
    /// The agent waits at its prompt without having finished a turn: the
    /// operator interrupted it. A turn that already ended stays ended.
    Idle,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HookCounts {
    pub working: u32,
    pub done: u32,
}
/// Sends `report` for the pane, stamped with when it was made, until
/// `deadline`. A refusal or a failure to ask comes back as the cause it is
/// recorded under ([`crate::delivery::refusal_cause`]); one that nothing
/// answered for ([`crate::outbox::WAITED_OUT`]) also waits in the outbox
/// for the core to come back.
pub fn report(report: &HookStatusReport, deadline: Instant) -> Result<(), String> {
    let mut report = report.clone();
    if report.at_unix_us.is_none() {
        report.at_unix_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|made| u64::try_from(made.as_micros()).ok());
    }
    let sent = crate::delivery::ask(
        crate::hook_request::HookCommand::Status {
            report: report.clone(),
        },
        deadline,
    )
    .map(|_| ())
    .map_err(|failure| failure.code.to_owned());
    if let Err(cause) = &sent
        && crate::outbox::WAITED_OUT.contains(&cause.as_str())
    {
        crate::outbox::keep(report);
    }
    sent
}

/// The last report that did not reach the core, kept so the failure is a
/// sentence in `doctor` and in Settings rather than a pane that quietly
/// reads as uninstrumented (engineering rules 4 and 10).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportFailure {
    pub pane_id: String,
    /// The adapter id of the agent whose hook reported.
    #[serde(default)]
    pub agent: String,
    pub event: String,
    pub error: String,
    pub at_unix_ms: u64,
}

impl ReportFailure {
    /// The one sentence both surfaces show.
    pub fn message(&self) -> String {
        format!(
            "The hook could not report to Hide for pane {} on {}: {}",
            self.pane_id, self.event, self.error
        )
    }
}

fn failure_path(home: &Path) -> PathBuf {
    home.join(".hide")
        .join("agent-hooks")
        .join("last-report-failure.json")
}

/// Records the outcome of one report: a failure replaces the last one, a
/// success clears it, so the file describes the hook's current state rather
/// than its history.
pub fn record_outcome(
    home: &Path,
    pane_id: &str,
    agent: &str,
    event: &str,
    outcome: &Result<(), String>,
) -> io::Result<()> {
    let path = failure_path(home);
    match outcome {
        Ok(()) => match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
        Err(error) => {
            let failure = ReportFailure {
                pane_id: pane_id.to_owned(),
                agent: agent.to_owned(),
                event: event.to_owned(),
                error: error.clone(),
                at_unix_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_millis() as u64)
                    .unwrap_or(0),
            };
            // The file holds the hook's state now, which the next success
            // clears; the log keeps every report that did not arrive.
            let _ = crate::hook_log::append_line(
                &crate::hook_log::log_path(home),
                &serde_json::json!({
                    "component": "agent_hooks",
                    "kind": "report.failed",
                    "pane": failure.pane_id,
                    "agent": failure.agent,
                    "event": failure.event,
                    "cause": failure.error,
                    "at_unix_ms": failure.at_unix_ms,
                })
                .to_string(),
            );
            if let Some(parent) = path.parent() {
                hide_platform::fs::private::create_dir_all(parent)?;
            }
            fs::write(&path, serde_json::to_vec(&failure)?)
        }
    }
}

/// The last recorded failure, or none when the last report succeeded or no
/// report has run. A file that cannot be read back is reported as a failure
/// of its own rather than read as "no failure".
pub fn last_failure(home: &Path) -> Option<ReportFailure> {
    let path = failure_path(home);
    match fs::read(&path) {
        Ok(raw) => match serde_json::from_slice(&raw) {
            Ok(failure) => Some(failure),
            Err(error) => Some(ReportFailure {
                pane_id: String::new(),
                agent: String::new(),
                event: String::new(),
                error: format!("{} could not be read back: {error}", path.display()),
                at_unix_ms: 0,
            }),
        },
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hide-agent-hooks-report-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch directory");
        root
    }

    #[test]
    fn a_report_names_the_agent_its_session_turn_and_counts() {
        let report = HookStatusReport {
            runtime: "claude".to_owned(),
            version: crate::HOOK_VERSION,
            session: Some("s-1".to_owned()),
            turn: Some(HookTurn::Waiting),
            counts: Some(HookCounts {
                working: 2,
                done: 5,
            }),
            session_start: false,
            waiting: None,
            at_unix_us: None,
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({"runtime": "claude", "version": crate::HOOK_VERSION, "session": "s-1",
                "turn": "waiting", "counts": {"working": 2, "done": 5}, "session_start": false})
        );
        // What an event says nothing about stays out of the report.
        let bare: HookStatusReport =
            serde_json::from_value(json!({"runtime": "grok", "version": crate::HOOK_VERSION}))
                .unwrap();
        assert_eq!((bare.session, bare.turn, bare.counts), (None, None, None));
    }

    #[test]
    fn a_failure_is_recorded_with_its_pane_agent_and_event_and_a_success_clears_it() {
        let root = scratch("failure");
        let outcome = Err("unavailable".to_owned());
        record_outcome(&root, "w1:p1", "claude", "Stop", &outcome).expect("record");
        let failure = last_failure(&root).expect("a failure is remembered");
        assert_eq!(
            (
                failure.pane_id.as_str(),
                failure.agent.as_str(),
                failure.event.as_str()
            ),
            ("w1:p1", "claude", "Stop")
        );
        assert!(failure.message().contains("w1:p1"));
        record_outcome(&root, "w1:p1", "claude", "Stop", &Ok(())).expect("clear");
        assert_eq!(last_failure(&root), None);
        // The log keeps the failure the success cleared from the record.
        let log = fs::read_to_string(crate::hook_log::log_path(&root)).unwrap();
        let line: serde_json::Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert_eq!(
            (&line["kind"], &line["pane"], &line["event"], &line["cause"]),
            (
                &json!("report.failed"),
                &json!("w1:p1"),
                &json!("Stop"),
                &json!("unavailable")
            )
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_unreadable_failure_record_is_a_failure_not_a_clean_bill() {
        let root = scratch("unreadable");
        let path = failure_path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not json").unwrap();
        let failure = last_failure(&root).expect("reported");
        assert!(failure.error.contains("could not be read back"));
        fs::remove_dir_all(&root).ok();
    }
}
