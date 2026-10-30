//! The Projects tab's disk sweep (PRD overview-device-tasks B53-B55): one
//! project at a time in the tab's order, kept and fresh for ten minutes,
//! nothing new once the tab is hidden, and a failure keeps the last size.

use super::*;
use crate::model::DiskUsageSnapshot;

fn folders() -> Runtime {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = ["a", "b", "c"]
        .map(|name| {
            let path = format!("/{name}");
            workspace(name, name, &path, vec![checkout(name, name, &path, None)])
        })
        .to_vec();
    runtime
}

fn measuring(runtime: &Runtime) -> Vec<String> {
    runtime
        .disk_sweep_request(None)
        .paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

fn measured(path: &str, bytes: Option<u64>, reason: Option<&str>) -> Vec<DiskUsageSnapshot> {
    vec![DiskUsageSnapshot {
        path: Some(path.into()),
        total_bytes: bytes,
        unavailable_reason: reason.map(str::to_owned),
        measured_at_unix_ms: Some(unix_milliseconds()),
        volume_free_bytes: Some(5),
        volume_total_bytes: Some(100),
        ..Default::default()
    }]
}

fn disk(runtime: &Runtime, id: &str) -> crate::model::ProjectDiskSnapshot {
    runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .find(|workspace| workspace.id == id)
        .unwrap()
        .disk
        .clone()
}

fn watch(runtime: &mut Runtime, ids: &[&str]) {
    runtime.watch_disk(super::super::disk_sweep::DiskWatchPayload {
        workspace_ids: ids.iter().map(|id| (*id).to_owned()).collect(),
    });
}

#[test]
fn the_tab_measures_its_projects_one_at_a_time_in_its_order() {
    let mut runtime = folders();
    watch(&mut runtime, &["b", "a"]);
    assert_eq!(measuring(&runtime), ["/b"]);
    assert!(disk(&runtime, "b").measuring);
    assert!(disk(&runtime, "a").queued);

    // A running read's rows so far do not move the sweep on.
    runtime.ingest_swept_disk(None, measured("/b", Some(7), None), false);
    assert_eq!(measuring(&runtime), ["/b"]);
    runtime.ingest_swept_disk(None, measured("/b", Some(7), None), true);
    assert_eq!(measuring(&runtime), ["/a"]);
    let b = disk(&runtime, "b");
    assert_eq!(
        (b.total_bytes, b.measuring, b.queued),
        (Some(7), false, false)
    );
    assert_eq!((b.free_bytes, b.volume_total_bytes), (Some(5), Some(100)));

    // Leaving the tab lets the running one finish and starts nothing new.
    watch(&mut runtime, &[]);
    runtime.ingest_swept_disk(None, measured("/a", None, Some("timed out")), true);
    assert!(measuring(&runtime).is_empty());
    assert_eq!(
        disk(&runtime, "a").unavailable_reason.as_deref(),
        Some("timed out")
    );

    // Opened again inside ten minutes: the fresh one is not measured again,
    // the failed one is.
    watch(&mut runtime, &["b", "a"]);
    assert_eq!(measuring(&runtime), ["/a"]);
}

#[test]
fn a_failed_measurement_keeps_the_last_size_and_the_sweep_goes_on() {
    let mut runtime = folders();
    runtime.ingest_swept_disk(None, measured("/a", Some(9), None), false);
    let mut stale = measured("/a", Some(9), None);
    stale[0].measured_at_unix_ms = Some(1);
    runtime.disk_usage = stale;
    watch(&mut runtime, &["a", "c"]);
    assert_eq!(
        measuring(&runtime),
        ["/a"],
        "a value older than ten minutes is measured again"
    );
    runtime.ingest_swept_disk(None, measured("/a", None, Some("denied")), true);
    let a = disk(&runtime, "a");
    assert_eq!(a.total_bytes, Some(9));
    assert_eq!(a.unavailable_reason.as_deref(), Some("denied"));
    assert_eq!(measuring(&runtime), ["/c"]);
}

#[test]
fn the_cleanup_sheet_s_project_is_measured_first_even_with_the_tab_hidden() {
    let mut runtime = folders();
    runtime.measure_disk_first(None, "/c");
    assert_eq!(measuring(&runtime), ["/c"]);
    runtime.ingest_swept_disk(None, measured("/c", Some(1), None), true);
    assert!(measuring(&runtime).is_empty());
}

#[test]
fn a_git_project_watched_before_its_worktrees_are_listed_is_measured_once_they_are() {
    let mut runtime = runtime();
    let mut repo = workspace(
        "r",
        "repo",
        "/repo",
        vec![checkout("r", "repo", "/repo", None)],
    );
    repo.is_git = true;
    runtime.snapshot.navigator.workspaces = vec![repo];
    watch(&mut runtime, &["r"]);
    assert!(
        measuring(&runtime).is_empty(),
        "nothing names its paths yet"
    );
    runtime.ingest_worktrees(
        crate::model::WorktreeCatalogSnapshot {
            projects: vec![crate::model::ProjectWorktreesSnapshot {
                root_path: "/repo".to_owned(),
                worktrees: vec![crate::model::WorktreeSnapshot {
                    path: "/repo".to_owned(),
                    is_main: true,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        },
        0,
    );
    assert_eq!(measuring(&runtime), ["/repo"]);
    assert!(disk(&runtime, "r").measuring);
}

#[test]
fn a_reorder_of_the_tab_does_not_measure_a_failed_project_again() {
    let mut runtime = folders();
    watch(&mut runtime, &["a", "b"]);
    runtime.ingest_swept_disk(None, measured("/a", None, Some("denied")), true);
    assert_eq!(measuring(&runtime), ["/b"]);
    runtime.ingest_swept_disk(None, measured("/b", Some(3), None), true);
    assert!(measuring(&runtime).is_empty());
    // An agent's state moved the rows; nothing new is measured.
    watch(&mut runtime, &["b", "a"]);
    assert!(measuring(&runtime).is_empty());
    // A project the tab shows for the first time is.
    watch(&mut runtime, &["b", "a", "c"]);
    assert_eq!(measuring(&runtime), ["/c"]);
}

/// PRD project-panel-results B20: the project the lane measures stops when
/// the last watch on it goes, before its answer, and is measured anew when a
/// watch shows it again; a project measured while it showed is not.
#[test]
fn a_measurement_no_watch_shows_stops_and_starts_again_when_shown() {
    let mut runtime = folders();
    watch(&mut runtime, &["a", "b"]);
    assert_eq!(measuring(&runtime), ["/a"]);
    watch(&mut runtime, &["b"]);
    assert_eq!(measuring(&runtime), ["/b"], "the unwatched read stops");
    runtime.ingest_swept_disk(None, measured("/b", Some(3), None), true);
    watch(&mut runtime, &[]);
    assert!(measuring(&runtime).is_empty());
    watch(&mut runtime, &["a", "b"]);
    assert_eq!(
        measuring(&runtime),
        ["/a"],
        "the fresh one is not measured again"
    );
}
