//! Reporting a pane's agent to the core: its turn and its subagent counts.
//!
//! The report is one hook request to hided (`hide workspace status --hook`,
//! `HookCommand::Status`), which proves the caller is in the pane like every
//! other hook request and hands the report to the core (PRD engine-bundle-1
//! D-7). Nothing is written to Herdr. The sibling `hide` runs within
//! [`REPORT_TIMEOUT`], so a daemon that is gone or does not answer costs the
//! agent's turn that long and no more (B13).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How long one report may hold the agent's hook slot, the `hide` it runs
/// included.
pub const REPORT_TIMEOUT: Duration = Duration::from_secs(2);

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
/// Sends `report` for the pane, until `deadline`. A refusal or a failure to
/// ask comes back as the cause it is recorded under
/// ([`crate::delivery::refusal_cause`]).
pub fn report(report: &HookStatusReport, deadline: Instant) -> Result<(), String> {
    let input = serde_json::to_vec(report).map_err(|_| "format".to_owned())?;
    crate::delivery::run_cli_with_input(&["workspace", "status", "--hook"], &input, deadline)
        .map(|_| ())
        .map_err(|failure| failure.code.to_owned())
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
    fn a_report_without_a_daemon_to_ask_ends_at_once() {
        // No `hide` sits beside a test binary, which is the missing CLI a
        // hook outside a Hide install meets.
        let started = Instant::now();
        let outcome = report(
            &HookStatusReport {
                runtime: "claude".to_owned(),
                version: crate::HOOK_VERSION,
                session: None,
                turn: Some(HookTurn::Working),
                counts: None,
                session_start: false,
            },
            started + REPORT_TIMEOUT,
        );
        assert_eq!(outcome, Err("cli".to_owned()));
        assert!(started.elapsed() < REPORT_TIMEOUT);
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
