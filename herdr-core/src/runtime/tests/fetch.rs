//! The background fetch's schedule (PRD git-freshness D-01, D-02): what is
//! fetched when, one per node, and what the header then says.

use super::*;
use crate::observed::{Freshness, Observation};
use hide_node_link::worktrees::{FetchFailure, FetchOutcome, FetchSkip};
use std::time::{Duration, Instant};

fn git_project(id: &str, path: &str) -> WorkspaceSnapshot {
    let checkout = checkout(id, &format!("{id}-checkout"), path, None);
    let mut project = workspace(id, id, path, vec![checkout]);
    project.is_git = true;
    project
}

/// Two local repositories with the first in front and a window attached.
fn fixture() -> Runtime {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces =
        vec![git_project("front", "/front"), git_project("back", "/back")];
    runtime.snapshot.navigator.focused_workspace_id = Some("front".into());
    runtime.snapshot.navigator.focused_checkout_id = Some("front-checkout".into());
    runtime.ui_attached = true;
    runtime
}

fn paths(
    jobs: &[(
        crate::runtime::FetchJob,
        std::sync::Arc<dyn crate::node_access::NodeLink>,
    )],
) -> Vec<String> {
    jobs.iter().map(|(job, _)| job.key.1.clone()).collect()
}

#[test]
fn one_fetch_runs_per_node_the_front_repository_first_and_none_without_a_window() {
    let mut runtime = fixture();
    let start = Instant::now();
    runtime.ui_attached = false;
    assert!(runtime.due_fetches(start).is_empty());
    runtime.ui_attached = true;

    let first = runtime.due_fetches(start);
    assert_eq!(paths(&first), ["/front"]);
    // Its node is busy until that fetch answers.
    assert!(runtime.due_fetches(start).is_empty());
    runtime.ingest_fetch(&first[0].0, Ok(FetchOutcome::Fetched), 1_000);
    assert_eq!(paths(&runtime.due_fetches(start)), ["/back"]);
}

#[test]
fn the_front_repository_is_fetched_every_minute_and_the_rest_every_five() {
    let mut runtime = fixture();
    let start = Instant::now();
    let at = |seconds| start + Duration::from_secs(seconds);
    for _ in 0..2 {
        let (job, _) = runtime.due_fetches(start).remove(0);
        runtime.ingest_fetch(&job, Ok(FetchOutcome::Fetched), 1_000);
    }
    assert!(runtime.due_fetches(at(59)).is_empty());
    let (front, _) = runtime.due_fetches(at(60)).remove(0);
    assert_eq!(front.key.1, "/front");
    runtime.ingest_fetch(&front, Ok(FetchOutcome::Fetched), 2_000);
    // At 4:59 only the front repository is due again; the other waits five
    // minutes from its own last fetch.
    let (front, _) = runtime.due_fetches(at(299)).remove(0);
    assert_eq!(front.key.1, "/front");
    runtime.ingest_fetch(&front, Ok(FetchOutcome::Fetched), 3_000);
    assert!(runtime.due_fetches(at(299)).is_empty());
    assert_eq!(paths(&runtime.due_fetches(at(300))), ["/back"]);
}

#[test]
fn a_credential_or_network_failure_waits_for_the_window_and_a_busy_ref_is_no_failure() {
    let mut runtime = fixture();
    let start = Instant::now();
    let (job, _) = runtime.due_fetches(start).remove(0);
    runtime.ingest_fetch(&job, Ok(FetchOutcome::Fetched), 1_000);
    let check = runtime
        .remote_check(crate::node::TEST_NODE, "/front")
        .unwrap();
    assert_eq!(
        (check.auto, check.observed),
        (true, Some(Observation::fresh(1_000)))
    );

    let (back, _) = runtime.due_fetches(start).remove(0);
    runtime.ingest_fetch(&back, Ok(FetchOutcome::Fetched), 1_000);
    let (job, _) = runtime
        .due_fetches(start + Duration::from_secs(60))
        .remove(0);
    assert!(runtime.ingest_fetch(
        &job,
        Ok(FetchOutcome::Failed {
            failure: FetchFailure::Auth,
            reason: "could not read Username".into(),
        }),
        2_000,
    ));
    let check = runtime
        .remote_check(crate::node::TEST_NODE, "/front")
        .unwrap();
    assert_eq!(
        check.observed,
        Some(Observation {
            freshness: Freshness::Failed,
            read_at_unix_ms: Some(1_000)
        })
    );
    // Paused: an hour on, only the other repository is fetched.
    let (back, _) = runtime
        .due_fetches(start + Duration::from_secs(3600))
        .remove(0);
    assert_eq!(back.key.1, "/back");
    runtime.ingest_fetch(&back, Ok(FetchOutcome::Fetched), 2_500);
    assert!(
        runtime
            .due_fetches(start + Duration::from_secs(3600))
            .is_empty()
    );
    // The window comes forward: the front repository is fetched at once.
    runtime.look_for_fetch(start + Duration::from_secs(3600));
    let (job, _) = runtime
        .due_fetches(start + Duration::from_secs(3600))
        .remove(0);
    assert_eq!(job.key.1, "/front");
    // The operator's own fetch held a ref: nothing failed.
    assert!(!runtime.ingest_fetch(
        &job,
        Ok(FetchOutcome::Skipped {
            reason: FetchSkip::Busy,
        }),
        3_000,
    ));
    assert!(
        runtime
            .remote_check(crate::node::TEST_NODE, "/front")
            .unwrap()
            .observed
            .is_some_and(|observed| observed.freshness == Freshness::Failed)
    );
}

#[test]
fn a_repository_whose_fetch_is_turned_off_is_not_fetched_and_shows_the_operators_fetch() {
    let mut runtime = fixture();
    let mut record =
        crate::project_settings::ProjectSettings::new(crate::node::TEST_NODE, "/front");
    record.auto_fetch = Some(false);
    runtime.snapshot.ui_state.project_settings.insert(
        crate::project_settings::key(crate::node::TEST_NODE, "/front"),
        record,
    );
    let start = Instant::now();
    assert_eq!(paths(&runtime.due_fetches(start)), ["/back"]);
    let check = runtime
        .remote_check(crate::node::TEST_NODE, "/front")
        .unwrap();
    assert_eq!((check.auto, check.observed), (false, None));

    // Turned on again, it is fetched on the next pass.
    runtime
        .snapshot
        .ui_state
        .project_settings
        .values_mut()
        .for_each(|record| record.auto_fetch = Some(true));
    let next = runtime.due_fetches(start);
    assert!(next.is_empty(), "the node is still fetching /back");
    let jobs: Vec<_> = runtime.fetch_running_jobs();
    runtime.ingest_fetch(&jobs[0], Ok(FetchOutcome::Fetched), 1_000);
    assert_eq!(paths(&runtime.due_fetches(start)), ["/front"]);
}

#[test]
fn a_ref_lock_that_never_clears_becomes_a_failure_and_a_passing_one_does_not() {
    let mut runtime = fixture();
    let start = Instant::now();
    let busy = || {
        Ok(FetchOutcome::Skipped {
            reason: FetchSkip::Busy,
        })
    };
    let failing = |runtime: &Runtime| {
        runtime
            .remote_check(crate::node::TEST_NODE, "/front")
            .unwrap()
            .observed
            .is_some_and(|observed| observed.freshness == Freshness::Failed)
    };
    let fetch_front = |runtime: &mut Runtime, outcome, at: u64| {
        runtime.look_for_fetch(start + Duration::from_secs(at));
        let (job, _) = runtime
            .due_fetches(start + Duration::from_secs(at))
            .into_iter()
            .find(|(job, _)| job.key.1 == "/front")
            .expect("the front repository is due");
        runtime.ingest_fetch(&job, outcome, at * 1_000)
    };
    runtime.snapshot.navigator.workspaces.truncate(1);
    // Two locked fetches, then one that passes: never a failure.
    assert!(!fetch_front(&mut runtime, busy(), 0));
    assert!(!fetch_front(&mut runtime, busy(), 60));
    fetch_front(&mut runtime, Ok(FetchOutcome::Fetched), 120);
    assert!(!failing(&runtime));
    // Three in a row: the lock is not the operator's fetch.
    assert!(!fetch_front(&mut runtime, busy(), 180));
    assert!(!fetch_front(&mut runtime, busy(), 240));
    assert!(fetch_front(&mut runtime, busy(), 300));
    assert!(failing(&runtime));
}
