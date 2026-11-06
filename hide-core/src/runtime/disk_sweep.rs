//! The Projects tab's disk column (PRD overview-device-tasks D-15, D-25,
//! D-27, B53-B56): while the tab is visible, the device's projects are
//! measured one at a time, in the order the tab shows them, on the
//! coordinator's sweep reader beside the lane card's. A measurement is kept
//! by path while the app runs and is fresh for ten minutes; leaving the tab
//! starts nothing new and stops the project being measured unless the
//! cleanup sheet asked for it. The Project tool watches its one project the
//! same way while it shows (PRD project-panel-results D-15, B20). A device
//! the core dials is never measured.

use super::*;
use crate::model::{DiskUsageSnapshot, ProjectDiskSnapshot};
use serde::Deserialize;

/// How long a measurement is shown without measuring it again.
pub(super) const FRESH_MS: u64 = 10 * 60 * 1000;
/// The projects one watch may name; a device holds far fewer.
const WATCH_LIMIT: usize = 512;

/// `disk_watch`: the projects the Projects tab shows, in its order, or none
/// when it is hidden.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DiskWatchPayload {
    pub(super) workspace_ids: Vec<String>,
}

/// One machine's sweep: this one's (`None`) or a node's that dials this core.
#[derive(Default)]
pub(super) struct Sweep {
    /// Project roots in the order the tab shows them; empty while hidden.
    order: Vec<String>,
    /// Asked out of turn, by the cleanup sheet, whether or not the tab shows.
    first: Option<String>,
    /// The project being measured.
    current: Option<String>,
    /// `current` was asked by the cleanup sheet, which no watch stops.
    current_first: bool,
    /// Measured or refused since the tab was opened; not asked again until
    /// it is opened again.
    done: HashSet<String>,
    generation: u64,
}

pub(super) type Sweeps = HashMap<Option<String>, Sweep>;

/// Stops the project being measured once no watch shows it, unless the
/// cleanup sheet asked for it: the reader's next generation ends the running
/// read before its next checkout, and the project is measured again when a
/// watch shows it (PRD project-panel-results B20).
fn stop_unwatched(node: Option<&str>, sweep: &mut Sweep) {
    let Some(path) = sweep.current.as_ref() else {
        return;
    };
    if sweep.current_first || sweep.order.contains(path) {
        return;
    }
    crate::diagnostic!(serde_json::json!({
        "component": "disk", "kind": "sweep.stopped", "node": node, "project": path,
    }));
    sweep.current = None;
    sweep.generation = sweep.generation.wrapping_add(1);
}

/// Lays newly measured rows over the held ones by path. A row that could
/// not be measured keeps the size it had with the reason beside it (B55).
/// Rows of paths no project holds any more are dropped, so the set stays as
/// large as the projects are.
pub(super) fn merge_rows(
    held: &[DiskUsageSnapshot],
    arrived: Vec<DiskUsageSnapshot>,
    known: &HashSet<&str>,
) -> Vec<DiskUsageSnapshot> {
    let mut rows: Vec<DiskUsageSnapshot> = held
        .iter()
        .filter(|row| row.path.as_deref().is_some_and(|path| known.contains(path)))
        .cloned()
        .collect();
    for row in arrived {
        if !row.path.as_deref().is_some_and(|path| known.contains(path)) {
            continue;
        }
        match rows.iter_mut().find(|kept| kept.path == row.path) {
            Some(kept) if row.total_bytes.is_none() && kept.total_bytes.is_some() => {
                kept.unavailable_reason = row.unavailable_reason;
            }
            Some(kept) => *kept = row,
            None => rows.push(row),
        }
    }
    rows
}

/// A folder project's size: the one row of its folder (B51: `폴더 합계`).
pub(super) fn folder_disk(
    rows: &[DiskUsageSnapshot],
    path: &str,
    measuring: bool,
    queued: bool,
) -> ProjectDiskSnapshot {
    let row = rows.iter().find(|row| row.path.as_deref() == Some(path));
    ProjectDiskSnapshot {
        total_bytes: row.and_then(|row| row.total_bytes),
        unavailable_reason: row.and_then(|row| row.unavailable_reason.clone()),
        measuring,
        queued,
        free_bytes: row.and_then(|row| row.volume_free_bytes),
        volume_total_bytes: row.and_then(|row| row.volume_total_bytes),
        confirmed_bytes: row.and_then(|row| row.total_bytes),
        layers: None,
    }
}

fn fresh(rows: &[DiskUsageSnapshot], paths: &[PathBuf], now: u64) -> bool {
    !paths.is_empty()
        && paths.iter().all(|path| {
            rows.iter().any(|row| {
                row.path.as_deref() == path.to_str()
                    && row.unavailable_reason.is_none()
                    && row
                        .measured_at_unix_ms
                        .is_some_and(|at| at.saturating_add(FRESH_MS) > now)
            })
        })
}

impl Runtime {
    /// The machine a project id is on, if it is one whose disk is measured:
    /// this one's, or a node's that dials this core.
    fn sweep_project(&self, workspace_id: &str) -> Option<(Option<String>, String)> {
        if let Some(workspace) = self.local_project_by_id(workspace_id) {
            return Some((None, workspace.path));
        }
        self.snapshot.status.remote.iter().find_map(|status| {
            let workspace = status
                .session
                .as_ref()?
                .workspaces
                .iter()
                .find(|workspace| workspace.id == workspace_id)?;
            self.link_origin(&status.target_id)
                .is_some_and(LinkOrigin::reports_own_facts)
                .then(|| (Some(status.target_id.clone()), workspace.path.clone()))
        })
    }

    /// `disk_watch`.
    pub(super) fn watch_disk(&mut self, payload: DiskWatchPayload) -> bool {
        if payload.workspace_ids.len() > WATCH_LIMIT {
            crate::diagnostic!(serde_json::json!({
                "component": "disk", "kind": "sweep.watch_capacity",
                "count": payload.workspace_ids.len(), "limit": WATCH_LIMIT,
            }));
            return false;
        }
        let mut machine: Option<Option<String>> = None;
        let mut order = Vec::new();
        for id in &payload.workspace_ids {
            let Some((node, path)) = self.sweep_project(id) else {
                continue;
            };
            if machine.get_or_insert_with(|| node.clone()) == &node && !order.contains(&path) {
                order.push(path);
            }
        }
        // One tab is watched at a time: what it no longer shows, and every
        // other machine's, starts nothing new.
        let mut moved = Vec::new();
        for (node, sweep) in &mut self.disk_sweeps {
            if !sweep.order.is_empty() && machine.as_ref() != Some(node) {
                sweep.order.clear();
                stop_unwatched(node.as_deref(), sweep);
                moved.push(node.clone());
            }
        }
        if let Some(node) = machine {
            let sweep = self.disk_sweeps.entry(node.clone()).or_default();
            if sweep.order != order {
                // A new visit measures what is not fresh again; on a
                // reorder a project the tab still shows keeps its turn
                // taken, so a failing one is not measured again.
                if sweep.order.is_empty() {
                    sweep.done.clear();
                } else {
                    sweep.done.retain(|path| order.contains(path));
                }
                sweep.order = order;
                stop_unwatched(node.as_deref(), sweep);
            }
            moved.push(node.clone());
            self.advance_disk_sweep(node.as_deref());
        }
        for node in moved {
            self.refresh_disk_column(node.as_deref());
        }
        true
    }

    /// The cleanup sheet's sizes are measured again, ahead of the tab's.
    pub(super) fn measure_disk_first(&mut self, node: Option<&str>, path: &str) {
        let sweep = self.disk_sweeps.entry(node.map(str::to_owned)).or_default();
        sweep.done.remove(path);
        if sweep.current.as_deref() != Some(path) {
            sweep.first = Some(path.to_owned());
        }
        self.advance_disk_sweep(node);
        self.refresh_disk_column(node);
    }

    /// The paths that make up a project's size: a Git project's worktrees
    /// and shared Git directory, a folder project's folder.
    fn project_disk_paths(&self, node: Option<&str>, path: &str) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let listed = match node {
            None => self.worktree_catalog.project(path),
            Some(node) => self
                .device_worktrees
                .get(node)
                .and_then(|listed| listed.projects.get(path)),
        };
        let Some(project) = listed else {
            let folder = match node {
                None => self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .any(|w| w.path == path && !w.is_git && w.remote_target_id.is_none()),
                Some(node) => self.node_sessions().any(|(id, session)| {
                    id == node
                        && session
                            .workspaces
                            .iter()
                            .any(|w| w.path == path && !w.is_git)
                }),
            };
            return match folder {
                true => (vec![PathBuf::from(path)], Vec::new()),
                false => (Vec::new(), Vec::new()),
            };
        };
        let mut paths: Vec<_> = project
            .worktrees
            .iter()
            .map(|w| PathBuf::from(&w.path))
            .collect();
        let mut shared = Vec::new();
        if let Some(git) = project.shared_git_path.as_ref().map(PathBuf::from) {
            paths.push(git.clone());
            shared.push(git);
        }
        paths.sort();
        paths.dedup();
        (paths, shared)
    }

    fn disk_rows(&self, node: Option<&str>) -> &[DiskUsageSnapshot] {
        match node {
            None => &self.disk_usage,
            Some(node) => self.device_disk.get(node).map_or(&[], Vec::as_slice),
        }
    }

    /// Starts the next project when none is being measured: the cleanup
    /// sheet's first, then the tab's first that is not fresh. A project the
    /// worktree catalog has not listed yet is passed over, so a catalog that
    /// arrives calls this again.
    pub(super) fn advance_disk_sweep(&mut self, node: Option<&str>) {
        let now = unix_milliseconds();
        let Some(sweep) = self.disk_sweeps.get(&node.map(str::to_owned)) else {
            return;
        };
        if sweep.current.is_some() {
            return;
        }
        let first = sweep.first.clone();
        let order = sweep.order.clone();
        let done = sweep.done.clone();
        let next = first.or_else(|| {
            order.into_iter().find(|path| {
                !done.contains(path) && {
                    let (paths, _) = self.project_disk_paths(node, path);
                    !paths.is_empty() && !fresh(self.disk_rows(node), &paths, now)
                }
            })
        });
        let Some(sweep) = self.disk_sweeps.get_mut(&node.map(str::to_owned)) else {
            return;
        };
        sweep.current_first = sweep.first.take().is_some() && next.is_some();
        if let Some(path) = &next {
            crate::diagnostic!(serde_json::json!({
                "component": "disk", "kind": "sweep.measure", "node": node, "project": path,
            }));
            sweep.generation = sweep.generation.wrapping_add(1);
        }
        sweep.current = next;
    }

    /// What the coordinator's sweep reader measures for `node` now.
    pub(crate) fn disk_sweep_request(&self, node: Option<&str>) -> crate::disk::DiskRequest {
        let Some(sweep) = self.disk_sweeps.get(&node.map(str::to_owned)) else {
            return Default::default();
        };
        let Some(path) = sweep.current.as_deref() else {
            return crate::disk::DiskRequest {
                generation: sweep.generation,
                ..Default::default()
            };
        };
        let (paths, shared_git) = self.project_disk_paths(node, path);
        crate::disk::DiskRequest {
            paths,
            shared_git,
            generation: sweep.generation,
        }
    }

    /// The sweep reader's rows: laid over the held ones, and, once a whole
    /// answer holds every part of the project being measured, the next one.
    pub(crate) fn ingest_swept_disk(
        &mut self,
        node: Option<&str>,
        rows: Vec<DiskUsageSnapshot>,
        whole: bool,
    ) -> bool {
        let finished = whole
            && self
                .disk_sweeps
                .get(&node.map(str::to_owned))
                .and_then(|sweep| sweep.current.clone())
                .is_some_and(|path| {
                    let (paths, _) = self.project_disk_paths(node, &path);
                    paths
                        .iter()
                        .all(|part| rows.iter().any(|row| row.path.as_deref() == part.to_str()))
                });
        let mut changed = match node {
            None => self.merge_local_disk(rows),
            Some(node) => self.merge_device_disk(node, rows),
        };
        if finished && let Some(sweep) = self.disk_sweeps.get_mut(&node.map(str::to_owned)) {
            if let Some(path) = sweep.current.take() {
                sweep.done.insert(path);
            }
            sweep.current_first = false;
            self.advance_disk_sweep(node);
            changed = true;
        }
        if changed {
            self.refresh_disk_column(node);
        }
        changed
    }

    pub(super) fn merge_local_disk(&mut self, rows: Vec<DiskUsageSnapshot>) -> bool {
        let mut known: HashSet<&str> = HashSet::new();
        for project in &self.worktree_catalog.projects {
            known.extend(project.worktrees.iter().map(|w| w.path.as_str()));
            known.extend(project.shared_git_path.as_deref());
        }
        known.extend(
            self.snapshot
                .navigator
                .workspaces
                .iter()
                .filter(|w| w.remote_target_id.is_none())
                .flat_map(|w| {
                    std::iter::once(w.path.as_str())
                        .chain(w.checkouts.iter().map(|c| c.path.as_str()))
                }),
        );
        let merged = merge_rows(&self.disk_usage, rows, &known);
        if merged == self.disk_usage {
            return false;
        }
        self.disk_usage = merged;
        true
    }

    pub(super) fn merge_device_disk(&mut self, node: &str, rows: Vec<DiskUsageSnapshot>) -> bool {
        let mut known: HashSet<&str> = HashSet::new();
        if let Some(listed) = self.device_worktrees.get(node) {
            for project in listed.projects.values() {
                known.extend(project.worktrees.iter().map(|w| w.path.as_str()));
                known.extend(project.shared_git_path.as_deref());
            }
        }
        for (id, session) in self.node_sessions() {
            if id == node {
                known.extend(session.workspaces.iter().map(|w| w.path.as_str()));
            }
        }
        let held = self.device_disk.get(node).map_or(&[][..], Vec::as_slice);
        let merged = merge_rows(held, rows, &known);
        if merged == held {
            return false;
        }
        self.device_disk.insert(node.to_owned(), merged);
        true
    }

    /// Whether the sweep is measuring the project now, and whether it will.
    pub(super) fn disk_sweep_marks(&self, node: Option<&str>, path: &str) -> (bool, bool) {
        let Some(sweep) = self.disk_sweeps.get(&node.map(str::to_owned)) else {
            return (false, false);
        };
        if sweep.current.as_deref() == Some(path) {
            return (true, false);
        }
        let waiting = (sweep.order.iter().any(|p| p == path) && !sweep.done.contains(path))
            || sweep.first.as_deref() == Some(path);
        let queued = waiting && {
            let (paths, _) = self.project_disk_paths(node, path);
            !paths.is_empty() && !fresh(self.disk_rows(node), &paths, unix_milliseconds())
        };
        (false, queued)
    }

    fn refresh_disk_column(&mut self, node: Option<&str>) {
        match node {
            None => {
                self.refresh_worktree_projection();
            }
            Some(node) => {
                self.refresh_device_catalog(node);
            }
        }
    }

    /// A device that left keeps no sweep.
    pub(super) fn forget_disk_sweep(&mut self, node: &str) {
        self.disk_sweeps.remove(&Some(node.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &str, total: Option<u64>, reason: Option<&str>) -> DiskUsageSnapshot {
        DiskUsageSnapshot {
            path: Some(path.into()),
            total_bytes: total,
            unavailable_reason: reason.map(str::to_owned),
            measured_at_unix_ms: Some(1),
            ..Default::default()
        }
    }

    #[test]
    fn a_failed_measurement_keeps_the_last_size_beside_its_reason() {
        let known = HashSet::from(["/a", "/b"]);
        let held = vec![row("/a", Some(10), None), row("/gone", Some(5), None)];
        let merged = merge_rows(
            &held,
            vec![row("/a", None, Some("timed out")), row("/b", Some(3), None)],
            &known,
        );
        assert_eq!(
            merged,
            [
                row("/a", Some(10), Some("timed out")),
                row("/b", Some(3), None)
            ]
        );
    }

    #[test]
    fn a_measurement_is_fresh_for_ten_minutes_and_only_without_a_failure() {
        let mut rows = vec![row("/a", Some(10), None)];
        rows[0].measured_at_unix_ms = Some(1_000);
        let paths = [PathBuf::from("/a")];
        assert!(fresh(&rows, &paths, 1_000 + FRESH_MS - 1));
        assert!(!fresh(&rows, &paths, 1_000 + FRESH_MS));
        rows[0].unavailable_reason = Some("denied".into());
        assert!(!fresh(&rows, &paths, 1_001));
        assert!(
            !fresh(&rows, &[], 1_001),
            "a project with no parts is never fresh"
        );
    }
}
