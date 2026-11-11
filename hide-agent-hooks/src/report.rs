//! Reporting a pane's agent to the core: its turn and its subagent counts.
//!
//! The report is one hook request to hided (`HookCommand::Status`, which
//! `hide workspace status --hook` also sends), which proves the caller is in
//! the pane like every other hook request and hands the report to the core
//! (PRD engine-bundle-1 D-7). The request ends
//! within [`REPORT_TIMEOUT`], so a daemon that is gone or does not answer
//! costs the agent's turn that long and no more (B13).

use std::fs;
use std::io;
use std::path::Path;
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
    /// At a session's start, the account variables set in the agent's
    /// process (`account::reported_variables`), empty for its default
    /// account: the core reads a pane the operator started by hand in that
    /// account (PRD agent-profiles D-03, D-35). Absent on every other event,
    /// for an agent Hide keeps no accounts of, and from a helper that
    /// predates it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Vec<(String, String)>>,
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
    crate::delivery::ask(
        crate::hook_request::HookCommand::Status {
            report: report.clone(),
        },
        deadline,
    )
    .map(|_| ())
    .map_err(|failure| failure.code.to_owned())
}

/// Which step of one event failed. The failure record carries it, and the
/// journey reads it instead of guessing from the sentence: the same cause
/// text can be a refused pane record or a report the core did not take.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The pane's record could not be written; the report still went.
    Record,
    /// The report did not reach the core; the record was kept.
    Report,
    /// Both failed in one event; each one's own sentence is beside this.
    RecordAndReport,
}

/// The last report that did not reach the core, kept so the failure is a
/// sentence in `doctor` and in Settings rather than a pane that quietly
/// reads as uninstrumented (engineering rules 4 and 10).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportFailure {
    pub pane_id: String,
    /// The adapter id of the agent whose hook reported.
    #[serde(default)]
    pub agent: String,
    pub event: String,
    pub error: String,
    pub at_unix_ms: u64,
    /// Which step failed, so a reader names the cause without reading the
    /// sentence. Absent in a record an earlier build wrote, whose kind is
    /// then unknown rather than guessed from its words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<FailureKind>,
    /// The pane record's own sentence, when that step failed; both are kept
    /// so one failure never hides the other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_error: Option<String>,
    /// The report's own sentence, when that step failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_error: Option<String>,
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

/// What one event's two steps did: keeping the record the counts and the
/// turn are read from, and reporting the session to the core. Both are kept
/// apart so a failure of one never hides the other
/// (engineering rules 4 and 10).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Outcome {
    pub kept: Result<(), String>,
    pub reported: Result<(), String>,
}

/// The outcome of an event that kept its record and reported itself.
pub fn outcome(kept: Result<(), String>, reported: Result<(), String>) -> Outcome {
    Outcome { kept, reported }
}

impl Outcome {
    /// Whether both steps went: the one answer a caller that only says
    /// "reported" reads.
    pub fn is_ok(&self) -> bool {
        self.kept.is_ok() && self.reported.is_ok()
    }

    /// The one sentence the failure record's `error` holds: the step that
    /// failed, or both when both did.
    fn message(&self) -> Option<String> {
        match (&self.kept, &self.reported) {
            (Ok(()), Ok(())) => None,
            (Err(kept), Ok(())) => Some(kept.clone()),
            (Ok(()), Err(reported)) => Some(reported.clone()),
            (Err(kept), Err(reported)) => Some(format!("{kept}; {reported}")),
        }
    }

    /// Which step failed, in the record's own terms.
    fn kind(&self) -> Option<FailureKind> {
        match (&self.kept, &self.reported) {
            (Ok(()), Ok(())) => None,
            (Err(_), Ok(())) => Some(FailureKind::Record),
            (Ok(()), Err(_)) => Some(FailureKind::Report),
            (Err(_), Err(_)) => Some(FailureKind::RecordAndReport),
        }
    }
}

/// Records the outcome of one report for the pane this process runs in: the
/// failure record and its log line go in the folder the hooks' records of
/// this process's daemon and installation live in
/// ([`crate::route::pane_records`]), so a helper's failure and the app's
/// never overwrite each other, and the pane a start waits on is the pane
/// whose failure `doctor`, a node and the journey find.
///
/// A failure replaces the last one and a success clears it, so the file
/// describes the hook's current state rather than its history.
pub fn record_outcome(
    home: &Path,
    pane_id: &str,
    agent: &str,
    event: &str,
    outcome: &Outcome,
) -> io::Result<()> {
    record_outcome_at(
        &crate::route::pane_records(home)?,
        pane_id,
        agent,
        event,
        outcome,
    )
}

/// [`record_outcome`] in a records folder named directly: the daemon's own,
/// as a test names it without reading this process's environment.
pub fn record_outcome_at(
    records: &Path,
    pane_id: &str,
    agent: &str,
    event: &str,
    outcome: &Outcome,
) -> io::Result<()> {
    let path = records.join("last-report-failure.json");
    let Some(error) = outcome.message() else {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
    };
    let failure = ReportFailure {
        pane_id: pane_id.to_owned(),
        agent: agent.to_owned(),
        event: event.to_owned(),
        error,
        at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0),
        kind: outcome.kind(),
        record_error: outcome.kept.as_ref().err().cloned(),
        report_error: outcome.reported.as_ref().err().cloned(),
    };
    // The file holds the hook's state now, which the next success
    // clears; the log keeps every report that did not arrive. Both
    // are written, and a log that could not be is named to the
    // caller rather than dropped, since a failure record that is
    // lost is the thing `doctor` and Settings exist to show
    // (engineering rules 4 and 10).
    let logged = crate::hook_log::append_line(
        &crate::hook_log::log_path_at(records),
        &serde_json::json!({
            "component": "agent_hooks",
            "kind": "report.failed",
            "failure": failure.kind,
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
    let recorded = fs::write(&path, serde_json::to_vec(&failure)?);
    match (logged, recorded) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(log), Ok(())) => Err(log),
        (Ok(()), Err(record)) => Err(record),
        (Err(log), Err(record)) => Err(io::Error::other(format!(
            "the log could not be written ({log}) and neither could the record ({record})"
        ))),
    }
}

/// [`record_outcome`], reporting a record that could not be written on
/// standard error rather than leaving the hook silent: the agent runs the
/// hook's process, and what it prints reaches the pane, so a hook whose
/// records folder is refused still says why (engineering rule 10).
///
/// When the failure record itself cannot be kept, both sentences are
/// printed: the step that failed and the record that could not hold it, so
/// the agent is never left with only the second.
pub fn record_and_say(home: &Path, pane_id: &str, agent: &str, event: &str, outcome: &Outcome) {
    if let Err(error) = record_outcome(home, pane_id, agent, event, outcome) {
        if let Some(original) = outcome.message() {
            eprintln!("hide-agent-hooks: {original}");
        }
        eprintln!("hide-agent-hooks: the hook's failure record was not kept: {error}");
    }
}

/// The last recorded failure for the pane this process runs in, from the
/// folder that pane's hook wrote to ([`crate::route::pane_records`]), or
/// none when the last report succeeded or no report has run.
///
/// A file that cannot be read back is reported as a failure of its own
/// rather than read as "no failure", and a route or a read that fails is
/// that error, not a clean bill (engineering rule 4).
pub fn last_failure(home: &Path) -> io::Result<Option<ReportFailure>> {
    last_failure_at(&crate::route::pane_records(home)?)
}

/// [`last_failure`] in a records folder named directly: the one a daemon's
/// node, doctor and a test read, which is the folder the hook of the same
/// daemon wrote to.
///
/// `NotFound` is the one answer that means absence: no report has failed
/// there, or the file a success removed is gone. Every other read error is
/// kept as this caller's error, so a records folder this process may not
/// read is `Hook records unreadable` in `doctor` and Settings rather than
/// the same empty answer as a hook that never failed (engineering rule 4).
pub fn last_failure_at(records: &Path) -> io::Result<Option<ReportFailure>> {
    let path = records.join("last-report-failure.json");
    let raw = match fs::read(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!("{} could not be read: {error}", path.display()),
            ));
        }
    };
    Ok(match serde_json::from_slice(&raw) {
        Ok(failure) => Some(failure),
        Err(error) => Some(ReportFailure {
            error: format!("{} could not be read back: {error}", path.display()),
            kind: None,
            ..ReportFailure::default()
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> std::path::PathBuf {
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
            account: None,
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
        assert_eq!(
            (bare.session, bare.turn, bare.counts, bare.account),
            (None, None, None, None)
        );
        // A session's start says the account its agent runs in; the default
        // account is an empty list, not an absent one.
        let started: HookStatusReport = serde_json::from_value(json!({
            "runtime": "claude-code", "version": crate::HOOK_VERSION, "session_start": true,
            "account": [["CLAUDE_CONFIG_DIR", "/accounts/claude-a"]],
        }))
        .unwrap();
        assert_eq!(
            started.account,
            Some(vec![(
                "CLAUDE_CONFIG_DIR".to_owned(),
                "/accounts/claude-a".to_owned()
            )])
        );
    }

    #[test]
    fn a_failure_is_recorded_with_its_pane_agent_and_event_and_a_success_clears_it() {
        let root = scratch("failure");
        let failed = outcome(
            Ok(()),
            Err("The hook could not reach Hide: unavailable".to_owned()),
        );
        record_outcome_at(&root, "w1:p1", "claude", "Stop", &failed).expect("record");
        let failure = last_failure_at(&root)
            .expect("records read")
            .expect("a failure is remembered");
        assert_eq!(
            (
                failure.pane_id.as_str(),
                failure.agent.as_str(),
                failure.event.as_str(),
                failure.kind,
            ),
            ("w1:p1", "claude", "Stop", Some(FailureKind::Report))
        );
        assert!(failure.message().contains("w1:p1"));
        record_outcome_at(&root, "w1:p1", "claude", "Stop", &outcome(Ok(()), Ok(())))
            .expect("clear");
        assert!(last_failure_at(&root).expect("records read").is_none());
        // The log keeps the failure the success cleared from the record, and
        // names the step beside the sentence.
        let log = fs::read_to_string(crate::hook_log::log_path_at(&root)).unwrap();
        let line: serde_json::Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert_eq!(
            (
                &line["kind"],
                &line["failure"],
                &line["pane"],
                &line["event"],
                &line["cause"]
            ),
            (
                &json!("report.failed"),
                &json!("report"),
                &json!("w1:p1"),
                &json!("Stop"),
                &json!("The hook could not reach Hide: unavailable")
            )
        );
        fs::remove_dir_all(&root).ok();
    }

    /// A refused pane record and a report that never reached the core each
    /// name their own step, and both sentences are kept apart when both
    /// failed, so one failure never hides the other (engineering rule 4).
    #[test]
    fn each_failing_step_is_named_and_both_are_kept_when_both_fail() {
        let root = scratch("kinds");
        let record_only = outcome(
            Err("the pane's count could not be changed: denied".to_owned()),
            Ok(()),
        );
        record_outcome_at(&root, "w1:p1", "claude", "SessionStart", &record_only).unwrap();
        let failure = last_failure_at(&root).unwrap().unwrap();
        assert_eq!(failure.kind, Some(FailureKind::Record));
        assert_eq!(
            (
                failure.record_error.as_deref(),
                failure.report_error.as_deref()
            ),
            (Some("the pane's count could not be changed: denied"), None)
        );

        let both = outcome(
            Err("the pane's count could not be changed: denied".to_owned()),
            Err("The hook could not reach Hide: unavailable".to_owned()),
        );
        record_outcome_at(&root, "w1:p1", "claude", "SessionStart", &both).unwrap();
        let failure = last_failure_at(&root).unwrap().unwrap();
        assert_eq!(failure.kind, Some(FailureKind::RecordAndReport));
        assert_eq!(
            (
                failure.record_error.as_deref(),
                failure.report_error.as_deref()
            ),
            (
                Some("the pane's count could not be changed: denied"),
                Some("The hook could not reach Hide: unavailable")
            )
        );
        assert_eq!(
            failure.error,
            "the pane's count could not be changed: denied; The hook could not reach Hide: unavailable"
        );
        fs::remove_dir_all(&root).ok();
    }

    /// A record an earlier build wrote holds no kind, and is read back as an
    /// unknown kind with its own sentence, never guessed into a report
    /// failure from its words: the old plugin's turn path wrote record
    /// failures there too.
    #[test]
    fn a_record_without_a_kind_stays_unknown_and_keeps_its_sentence() {
        let root = scratch("legacy");
        let path = root.join("last-report-failure.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            json!({
                "pane_id": "w1:p1", "agent": "opencode", "event": "turn",
                "error": "the pane's record could not be changed: denied", "at_unix_ms": 7,
            })
            .to_string(),
        )
        .unwrap();
        let failure = last_failure_at(&root)
            .expect("records read")
            .expect("read back");
        assert_eq!(failure.kind, None);
        assert_eq!(
            failure.error,
            "the pane's record could not be changed: denied"
        );
        assert_eq!(
            (failure.record_error, failure.report_error),
            (None, None),
            "an earlier record names no step of its own"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_unreadable_failure_record_is_a_failure_not_a_clean_bill() {
        let root = scratch("unreadable");
        let path = root.join("last-report-failure.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not json").unwrap();
        let failure = last_failure_at(&root)
            .expect("records read")
            .expect("reported");
        assert!(failure.error.contains("could not be read back"));
        fs::remove_dir_all(&root).ok();
    }
}
