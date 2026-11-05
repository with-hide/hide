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

use std::collections::{BTreeMap, HashSet};

use hide_node_link::gh::HEAD_RUNS_SUITE_LIMIT;
use serde::Deserialize;

use super::{FAILED_CONCLUSIONS, parse_rfc3339_ms};
use crate::model::PullRequestChecks;

/// What one commit's workflow runs add up to, and which runs decide it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HeadRuns {
    pub checks: PullRequestChecks,
    /// The runs whose jobs a pull request's card counts: each workflow's
    /// unfinished runs, else the run that decides it. `None` when the runs
    /// were not judged, so no job is set aside.
    pub counted: Option<HashSet<u64>>,
}

impl HeadRuns {
    const UNKNOWN: Self = Self {
        checks: PullRequestChecks::Unknown,
        counted: None,
    };
}

/// What each of `commits`' runs add up to, in the order the query named them,
/// from a `HideHeadRuns` answer about `repository`. A commit GitHub does not know, or one
/// with more runs than were read, is `Unknown`; an answer that does not read
/// is an error.
pub(crate) fn parse_head_runs(
    output: &str,
    repository: &str,
    commits: &[String],
) -> Result<Vec<HeadRuns>, String> {
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
    let mut answered = answer
        .data
        .repository
        .ok_or("the runs query found no repository")?;
    commits
        .iter()
        .enumerate()
        .map(|(index, head)| {
            let commit = answered
                .remove(&format!("c{index}"))
                .ok_or_else(|| format!("the runs query left out commit {index}"))?;
            let Some(commit) = commit else {
                return Ok(HeadRuns::UNKNOWN);
            };
            if commit.suites.total > HEAD_RUNS_SUITE_LIMIT {
                crate::diagnostic!(serde_json::json!({
                    "component": "github",
                    "kind": "checks.runs_capped",
                    "repository": repository,
                    "head": head,
                    "runs": commit.suites.total,
                    "cap": HEAD_RUNS_SUITE_LIMIT,
                }));
                return Ok(HeadRuns::UNKNOWN);
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
    #[serde(rename = "databaseId")]
    id: u64,
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

fn verdict(suites: &[Suite]) -> Result<HeadRuns, String> {
    let mut workflows: BTreeMap<u64, Vec<(&Suite, &WorkflowRun)>> = BTreeMap::new();
    for suite in suites {
        if let Some(run) = &suite.run
            && judges_the_pull_request(&run.event)
        {
            workflows
                .entry(run.workflow.id)
                .or_default()
                .push((suite, run));
        }
    }
    let mut judged = HeadRuns {
        checks: PullRequestChecks::None,
        counted: Some(HashSet::new()),
    };
    for runs in workflows.values() {
        let (checks, counted) = workflow_verdict(runs)?;
        judged.checks = worse(judged.checks, checks);
        judged.counted.get_or_insert_default().extend(counted);
    }
    Ok(judged)
}

/// One workflow's verdict and the runs it was read from.
fn workflow_verdict(
    runs: &[(&Suite, &WorkflowRun)],
) -> Result<(PullRequestChecks, Vec<u64>), String> {
    let unfinished: Vec<u64> = runs
        .iter()
        .filter(|(suite, _)| suite.status != "COMPLETED")
        .map(|(_, run)| run.id)
        .collect();
    if !unfinished.is_empty() {
        return Ok((PullRequestChecks::Pending, unfinished));
    }
    let mut last: Option<(u64, &Suite, u64)> = None;
    let mut last_cancelled: Option<(u64, u64)> = None;
    for (suite, run) in runs {
        let at = parse_rfc3339_ms(&suite.updated_at)
            .ok_or_else(|| format!("a run's time does not read: {}", suite.updated_at))?;
        if suite.conclusion.as_deref() == Some("CANCELLED") {
            if last_cancelled.is_none_or(|(before, _)| at >= before) {
                last_cancelled = Some((at, run.id));
            }
        } else if last.is_none_or(|(before, ..)| at >= before) {
            last = Some((at, suite, run.id));
        }
    }
    let Some((_, suite, id)) = last else {
        // Every run was cancelled and none replaced it: the last of them is
        // what failed.
        return Ok((
            PullRequestChecks::Failed,
            last_cancelled.map(|(_, id)| id).into_iter().collect(),
        ));
    };
    let checks = match suite.conclusion.as_deref() {
        Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => PullRequestChecks::Passing,
        Some(value) if FAILED_CONCLUSIONS.contains(&value) => PullRequestChecks::Failed,
        _ => PullRequestChecks::Unknown,
    };
    Ok((checks, vec![id]))
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
        let run = parse_rfc3339_ms(updated).expect("a time");
        format!(
            r#"{{"status":"{status}","conclusion":{conclusion},"updatedAt":"{updated}","workflowRun":{{"databaseId":{run},"event":"{event}","workflow":{{"databaseId":{workflow}}}}}}}"#
        )
    }

    fn judged(suites: &[String]) -> PullRequestChecks {
        let answer = format!(
            r#"{{"data":{{"repository":{{"c0":{{"checkSuites":{{"totalCount":{},"nodes":[{}]}}}}}}}}}}"#,
            suites.len(),
            suites.join(",")
        );
        parse_head_runs(&answer, "acme/app", &commits(1)).expect("an answer")[0].checks
    }

    const VERIFY: u64 = 22;

    fn commits(count: usize) -> Vec<String> {
        (0..count).map(|index| format!("{index:040x}")).collect()
    }

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
            parse_head_runs(&answer, "acme/app", &commits(2)).expect("an answer"),
            [HeadRuns::UNKNOWN, HeadRuns::UNKNOWN]
        );
        assert!(
            parse_head_runs(&answer, "acme/app", &commits(3)).is_err(),
            "a commit left out"
        );
        assert!(parse_head_runs("{}", "acme/app", &commits(1)).is_err());
    }
}
