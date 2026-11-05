//! What a commit's GitHub Actions runs add up to (docs/status-model.md,
//! Checks of a head commit).
//!
//! A job row cannot say this: GitHub lists the jobs of every run on a commit
//! under the same names, a cancelled run's closing job reads FAILURE, and a
//! run queued before it made a job is not listed at all. So each workflow is
//! judged by its runs, as `HideHeadRuns` reads them (one check suite per run):
//! the run that finished last decides, a cancelled run decides nothing, an
//! unfinished run keeps the workflow pending, and a workflow whose every run
//! was cancelled failed.

use std::collections::BTreeMap;

use hide_node_link::gh::HEAD_RUNS_SUITE_LIMIT;
use serde::Deserialize;

use super::{FAILED_CONCLUSIONS, parse_rfc3339_ms};
use crate::model::PullRequestChecks;

/// The verdict of each of `count` commits, in the order the query named them,
/// from a `HideHeadRuns` answer. A commit GitHub does not know, or one with
/// more runs than were read, is `Unknown`; an answer that does not read is an
/// error.
pub(crate) fn parse_head_runs(
    output: &str,
    count: usize,
) -> Result<Vec<PullRequestChecks>, String> {
    #[derive(Deserialize)]
    struct Answer {
        data: Data,
    }
    #[derive(Deserialize)]
    struct Data {
        repository: Option<BTreeMap<String, Option<Commit>>>,
    }
    #[derive(Deserialize)]
    struct Commit {
        #[serde(rename = "checkSuites")]
        suites: Suites,
    }
    #[derive(Deserialize)]
    struct Suites {
        #[serde(rename = "totalCount")]
        total: usize,
        nodes: Vec<Suite>,
    }
    let answer: Answer = serde_json::from_str(output).map_err(|error| {
        format!("the runs query returned an answer Hide could not read: {error}")
    })?;
    let mut commits = answer
        .data
        .repository
        .ok_or("the runs query found no repository")?;
    (0..count)
        .map(|index| {
            let commit = commits
                .remove(&format!("c{index}"))
                .ok_or_else(|| format!("the runs query left out commit {index}"))?;
            let Some(commit) = commit else {
                return Ok(PullRequestChecks::Unknown);
            };
            if commit.suites.total > HEAD_RUNS_SUITE_LIMIT {
                crate::diagnostic!(serde_json::json!({
                    "component": "github",
                    "kind": "checks.runs_capped",
                    "runs": commit.suites.total,
                    "cap": HEAD_RUNS_SUITE_LIMIT,
                }));
                return Ok(PullRequestChecks::Unknown);
            }
            verdict(&commit.suites.nodes)
        })
        .collect()
}

#[derive(Deserialize)]
struct Suite {
    status: String,
    conclusion: Option<String>,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    /// Absent for another app's suite: its check runs are judged one by one.
    #[serde(rename = "workflowRun")]
    run: Option<WorkflowRun>,
}

#[derive(Deserialize)]
struct WorkflowRun {
    event: String,
    workflow: Workflow,
}

#[derive(Deserialize)]
struct Workflow {
    #[serde(rename = "databaseId")]
    id: u64,
}

/// A run judges the pull request when the pull request or a push to its
/// branch started it, the runs GitHub's own pull request checks list; one
/// started by hand or on a schedule on the same commit is someone else's.
fn judges_the_pull_request(event: &str) -> bool {
    event == "push" || event.starts_with("pull_request")
}

fn verdict(suites: &[Suite]) -> Result<PullRequestChecks, String> {
    let mut workflows: BTreeMap<u64, Vec<&Suite>> = BTreeMap::new();
    for suite in suites {
        if let Some(run) = &suite.run
            && judges_the_pull_request(&run.event)
        {
            workflows.entry(run.workflow.id).or_default().push(suite);
        }
    }
    let mut checks = PullRequestChecks::None;
    for runs in workflows.values() {
        checks = worse(checks, workflow_verdict(runs)?);
    }
    Ok(checks)
}

fn workflow_verdict(runs: &[&Suite]) -> Result<PullRequestChecks, String> {
    if runs.iter().any(|run| run.status != "COMPLETED") {
        return Ok(PullRequestChecks::Pending);
    }
    let mut last: Option<(u64, &Suite)> = None;
    for run in runs {
        if run.conclusion.as_deref() == Some("CANCELLED") {
            continue;
        }
        let at = parse_rfc3339_ms(&run.updated_at)
            .ok_or_else(|| format!("a run's time does not read: {}", run.updated_at))?;
        if last.is_none_or(|(before, _)| at >= before) {
            last = Some((at, run));
        }
    }
    let Some((_, run)) = last else {
        // Every run was cancelled and none replaced it.
        return Ok(PullRequestChecks::Failed);
    };
    Ok(match run.conclusion.as_deref() {
        Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => PullRequestChecks::Passing,
        Some(value) if FAILED_CONCLUSIONS.contains(&value) => PullRequestChecks::Failed,
        _ => PullRequestChecks::Unknown,
    })
}

/// What two parts of one commit's checks add up to: a failure first, then
/// anything unfinished, then anything unrecognised, then a pass.
pub(crate) fn worse(left: PullRequestChecks, right: PullRequestChecks) -> PullRequestChecks {
    use PullRequestChecks::{Failed, None, Passing, Pending, Unknown};
    let rank = |checks| match checks {
        Failed => 4,
        Pending => 3,
        Unknown => 2,
        Passing => 1,
        None => 0,
    };
    if rank(right) > rank(left) {
        right
    } else {
        left
    }
}

/// The word the advisory poll compares verdicts by.
pub(crate) fn word(checks: PullRequestChecks) -> &'static str {
    match checks {
        PullRequestChecks::Failed => "failed",
        PullRequestChecks::Pending => "pending",
        PullRequestChecks::Unknown => "unknown",
        PullRequestChecks::Passing => "passing",
        PullRequestChecks::None => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suite(workflow: u64, event: &str, status: &str, conclusion: &str, updated: &str) -> String {
        format!(
            r#"{{"status":"{status}","conclusion":{conclusion},"updatedAt":"{updated}","workflowRun":{{"event":"{event}","workflow":{{"databaseId":{workflow}}}}}}}"#
        )
    }

    fn judged(suites: &[String]) -> PullRequestChecks {
        let answer = format!(
            r#"{{"data":{{"repository":{{"c0":{{"checkSuites":{{"totalCount":{},"nodes":[{}]}}}}}}}}}}"#,
            suites.len(),
            suites.join(",")
        );
        parse_head_runs(&answer, 1).expect("an answer")[0]
    }

    const VERIFY: u64 = 22;

    /// with-hide/hide #2075's second run: cancelled, then failed, then passed
    /// on the same head, beside a first run that had passed earlier.
    #[test]
    fn the_run_that_finished_last_decides_its_workflow() {
        let first = suite(
            VERIFY,
            "pull_request",
            "COMPLETED",
            r#""SUCCESS""#,
            "2026-11-04T21:44:31Z",
        );
        let failed = suite(
            VERIFY,
            "pull_request",
            "COMPLETED",
            r#""FAILURE""#,
            "2026-11-04T22:09:54Z",
        );
        let passed = suite(
            VERIFY,
            "pull_request",
            "COMPLETED",
            r#""SUCCESS""#,
            "2026-11-04T22:29:52Z",
        );
        assert_eq!(
            judged(&[first.clone(), failed.clone()]),
            PullRequestChecks::Failed
        );
        assert_eq!(
            judged(&[failed, first]),
            PullRequestChecks::Failed,
            "order in the answer does not matter"
        );
        assert_eq!(judged(&[passed]), PullRequestChecks::Passing);
    }

    #[test]
    fn a_run_started_by_hand_or_another_app_does_not_judge_the_pull_request() {
        let dispatched = suite(
            VERIFY + 1,
            "workflow_dispatch",
            "IN_PROGRESS",
            "null",
            "2026-11-04T15:12:28Z",
        );
        let other_app = r#"{"status":"QUEUED","conclusion":null,"updatedAt":"2026-11-04T15:12:28Z","workflowRun":null}"#;
        let passed = suite(
            VERIFY,
            "pull_request",
            "COMPLETED",
            r#""SUCCESS""#,
            "2026-11-04T18:31:38Z",
        );
        assert_eq!(
            judged(&[dispatched, other_app.to_owned(), passed]),
            PullRequestChecks::Passing
        );
        assert_eq!(judged(&[]), PullRequestChecks::None);
    }

    #[test]
    fn a_commit_past_the_runs_cap_or_unknown_to_github_is_not_judged() {
        let answer = format!(
            r#"{{"data":{{"repository":{{"c0":{{"checkSuites":{{"totalCount":{},"nodes":[]}}}},"c1":null}}}}}}"#,
            HEAD_RUNS_SUITE_LIMIT + 1
        );
        assert_eq!(
            parse_head_runs(&answer, 2).expect("an answer"),
            [PullRequestChecks::Unknown, PullRequestChecks::Unknown]
        );
        assert!(parse_head_runs(&answer, 3).is_err(), "a commit left out");
        assert!(parse_head_runs("{}", 1).is_err());
    }
}
