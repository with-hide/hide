//! Worktree disk usage, measured on the node that holds the checkouts
//! (`hide_host::disk`) and read here on the existing background worker. Git
//! and Overview share this lane; opening or explicit refresh requests a
//! measurement.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hide_node_link::disk::DiskUsage;
use hide_node_link::protocol::Call;

use crate::model::DiskUsageSnapshot;
use crate::node_access::{NodeLink, call_as, call_as_with_progress};
use crate::reader::BackgroundRead;

/// The node's own bound on a request (five minutes across its checkouts),
/// with room for the answer to arrive.
const MEASURE_TIMEOUT: Duration = Duration::from_secs(330);

/// A volume's free space is one system call.
const FREE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiskRequest {
    /// Checkout roots and the shared Git directory, partitioned without overlap.
    pub paths: Vec<PathBuf>,
    /// The entries of `paths` that are a shared Git directory: measured as one
    /// size, never sorted into layers.
    pub shared_git: Vec<PathBuf>,
    /// Bumped on section opening and explicit refresh.
    pub generation: u64,
}

pub struct DiskReader {
    inner: BackgroundRead<DiskRequest, Vec<DiskUsageSnapshot>>,
    /// Rows the running read has finished, handed out as they arrive.
    finished: Arc<Mutex<Vec<DiskUsageSnapshot>>>,
    /// The last answer with the rows finished since laid over it.
    current: Vec<DiskUsageSnapshot>,
    /// The generation asked for last: a running read of another one stops at
    /// its next finished checkout, so a measurement nothing shows any more
    /// ends (PRD project-panel-results B20).
    wanted: Arc<AtomicU64>,
}

impl DiskReader {
    /// Measures on `node`, the node that holds the checkouts.
    pub fn new(node: Arc<dyn NodeLink>) -> Self {
        let finished = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&finished);
        let wanted = Arc::new(AtomicU64::new(0));
        let asked = Arc::clone(&wanted);
        Self {
            inner: BackgroundRead::on_change(Duration::ZERO, move |request| {
                read_with(node.as_ref(), request, |row| {
                    sink.lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(row.clone());
                    asked.load(Ordering::Acquire) == request.generation
                })
            }),
            finished,
            current: Vec::new(),
            wanted,
        }
    }

    /// The whole answer when a read finishes; while one runs, the previous
    /// answer with each checkout laid over it as soon as it is measured, so a
    /// slow project fills row by row (a partial answer never removes a row).
    pub fn read_if_due(&mut self, request: DiskRequest) -> Option<Vec<DiskUsageSnapshot>> {
        self.poll(request).map(|(rows, _)| rows)
    }

    /// [`Self::read_if_due`], saying whether the rows are a finished read's
    /// whole answer (`true`) or a running read's rows so far.
    pub fn poll(&mut self, request: DiskRequest) -> Option<(Vec<DiskUsageSnapshot>, bool)> {
        self.wanted.store(request.generation, Ordering::Release);
        if let Some(answer) = self.inner.poll(request) {
            self.finished
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
            self.current = answer.clone();
            return Some((answer, true));
        }
        let arrived = std::mem::take(
            &mut *self
                .finished
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        if arrived.is_empty() {
            return None;
        }
        for row in arrived {
            match self.current.iter_mut().find(|held| held.path == row.path) {
                Some(held) => *held = row,
                None => self.current.push(row),
            }
        }
        Some((self.current.clone(), false))
    }
}

#[cfg(test)]
pub(crate) fn read(node: &dyn NodeLink, request: &DiskRequest) -> Vec<DiskUsageSnapshot> {
    read_with(node, request, |_| true)
}

/// Bytes free to an unprivileged writer on the volume holding `path`, as
/// `node` reads it; `None` when it cannot say.
pub(crate) fn volume_free_bytes(node: &dyn NodeLink, path: &Path) -> Option<u64> {
    call_as::<Option<u64>>(
        node,
        Call::VolumeFree {
            path: path.to_string_lossy().into_owned(),
        },
        FREE_TIMEOUT,
    )
    .map_err(|error| {
        crate::diagnostic!(serde_json::json!({
            "component": "disk",
            "kind": "disk.free_failed",
            "path": path,
            "reason": error.to_string(),
        }));
    })
    .ok()
    .flatten()
}

/// Measures every requested root on `node`, calling `finished` with each as
/// it is done; `finished` answering `false` stops the node before its next
/// root. A node that cannot be asked leaves every root unavailable with its
/// reason.
fn read_with(
    node: &dyn NodeLink,
    request: &DiskRequest,
    mut finished: impl FnMut(&DiskUsageSnapshot) -> bool,
) -> Vec<DiskUsageSnapshot> {
    if request.paths.is_empty() {
        return Vec::new();
    }
    let wire = |paths: &[PathBuf]| -> Vec<String> {
        paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    };
    let call = Call::DiskUsage {
        paths: wire(&request.paths),
        shared_git: wire(&request.shared_git),
    };
    let answer =
        call_as_with_progress::<Vec<DiskUsage>, DiskUsage>(node, call, MEASURE_TIMEOUT, |row| {
            finished(&snapshot(row))
        });
    match answer {
        Ok(rows) => {
            for row in &rows {
                if let Some(code) = &row.unavailable_code {
                    crate::diagnostic!(serde_json::json!({
                        "component": "disk",
                        "kind": "disk.measure_failed",
                        "checkout": row.path,
                        "reason_code": code,
                    }));
                }
            }
            rows.into_iter().map(snapshot).collect()
        }
        Err(error) => {
            let reason = format!("Disk usage could not be measured: {error}");
            crate::diagnostic!(serde_json::json!({
                "component": "disk",
                "kind": "disk.measure_failed",
                "reason_code": "node",
                "reason": error.to_string(),
            }));
            request
                .paths
                .iter()
                .map(|path| DiskUsageSnapshot {
                    path: Some(path.to_string_lossy().into_owned()),
                    unavailable_reason: Some(reason.clone()),
                    ..DiskUsageSnapshot::default()
                })
                .collect()
        }
    }
}

/// The node's row as the snapshot carries it.
fn snapshot(row: DiskUsage) -> DiskUsageSnapshot {
    DiskUsageSnapshot {
        measured_at_unix_ms: row.measured_at_unix_ms,
        path: row.path,
        total_bytes: row.total_bytes,
        largest_child_name: row.largest_child_name,
        largest_child_bytes: row.largest_child_bytes,
        unavailable_reason: row.unavailable_reason,
        layers: row.layers,
        volume_free_bytes: row.volume_free_bytes,
        volume_total_bytes: row.volume_total_bytes,
        folders: row.folders,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_access::{LinkAnswer, LinkError};

    /// A node that reports each root before answering them all, or refuses.
    struct Measuring {
        refuse: bool,
    }

    impl NodeLink for Measuring {
        fn call(&self, call: Call, timeout: Duration) -> Result<LinkAnswer, LinkError> {
            self.call_with_progress(call, timeout, &mut |_| true)
        }

        fn call_with_progress(
            &self,
            call: Call,
            _timeout: Duration,
            progress: &mut dyn FnMut(serde_json::Value) -> bool,
        ) -> Result<LinkAnswer, LinkError> {
            let Call::DiskUsage { paths, .. } = call else {
                panic!("disk calls only");
            };
            if self.refuse {
                return Err(LinkError::NotConnected("the node is gone".to_owned()));
            }
            let rows: Vec<DiskUsage> = paths
                .into_iter()
                .map(|path| DiskUsage {
                    path: Some(path),
                    total_bytes: Some(4096),
                    ..DiskUsage::default()
                })
                .collect();
            for row in &rows {
                progress(serde_json::to_value(row).unwrap());
            }
            Ok(LinkAnswer::Parsed(serde_json::to_value(rows).unwrap()))
        }
    }

    fn request() -> DiskRequest {
        DiskRequest {
            paths: vec![PathBuf::from("/repo"), PathBuf::from("/repo.worktrees/a")],
            ..DiskRequest::default()
        }
    }

    #[test]
    fn each_root_arrives_as_the_node_reports_it() {
        let mut arrived = Vec::new();
        let rows = read_with(&Measuring { refuse: false }, &request(), |row| {
            arrived.push(row.path.clone().unwrap());
            true
        });
        assert_eq!(arrived, ["/repo", "/repo.worktrees/a"]);
        assert!(rows.iter().all(|row| row.total_bytes == Some(4096)));
    }

    #[test]
    fn a_node_that_cannot_be_asked_leaves_every_root_unavailable() {
        let rows = read(&Measuring { refuse: true }, &request());
        assert_eq!(rows.len(), 2);
        for row in rows {
            assert_eq!(row.total_bytes, None);
            assert_eq!(
                row.unavailable_reason.as_deref(),
                Some("Disk usage could not be measured: the node is gone")
            );
        }
    }
}
