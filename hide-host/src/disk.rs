//! Worktree disk usage, measured on the node that holds the checkouts. The
//! core asks with `disk_usage` from its background reader and hears each
//! checkout as it finishes.
//!
//! A checkout is measured in layers (`disk_layers`): the walk sorts each
//! allocated block into source, a build-cache or dependency folder a tool
//! makes again, or an ignored folder nothing vouches for.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use hide_node_link::disk::{DiskUsage, Layer};
use hide_platform::fs::identity::FileId;
use hide_platform::fs::space;

use crate::disk_layers::{FolderTally, Siblings, Tally, base_rules, layer_of, rules_in};
use crate::index::IgnoreRules;

/// One checkout's share of a request: entries it may visit, and how long.
const ENTRY_LIMIT: usize = 1_000_000;
const TIME_LIMIT: Duration = Duration::from_secs(30);
/// The whole request's bound, whatever the number of checkouts: entries
/// visited, distinct hard-linked inodes remembered, and time. A checkout the
/// request cannot reach in time is an unavailable row, never a larger number.
const REQUEST_ENTRY_LIMIT: usize = 10_000_000;
const REQUEST_SEEN_LIMIT: usize = 1_000_000;
const REQUEST_TIME_LIMIT: Duration = Duration::from_secs(300);

/// What one request has used across all its checkouts. Only a file with more
/// than one link needs remembering to be counted once.
struct RequestBudget {
    started: std::time::Instant,
    visited: usize,
    seen: HashSet<FileId>,
    entry_limit: usize,
    seen_limit: usize,
    time_limit: Duration,
}

impl RequestBudget {
    fn new(entry_limit: usize, seen_limit: usize, time_limit: Duration) -> Self {
        Self {
            started: std::time::Instant::now(),
            visited: 0,
            seen: HashSet::new(),
            entry_limit,
            seen_limit,
            time_limit,
        }
    }

    fn spent(&self) -> bool {
        self.visited > self.entry_limit
            || self.seen.len() >= self.seen_limit
            || self.started.elapsed() > self.time_limit
    }
}

/// Why a measurement has no total: a code for the log, the words the row keeps.
struct Failure {
    code: &'static str,
    reason: String,
}

impl Failure {
    fn new(code: &'static str, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }

    fn incomplete(error: std::io::Error) -> Self {
        Self::new(
            "unreadable",
            format!("Disk measurement incomplete: {error}"),
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiskRequest {
    /// Checkout roots and the shared Git directory, partitioned without overlap.
    pub paths: Vec<PathBuf>,
    /// The entries of `paths` that are a shared Git directory: measured as one
    /// size, never sorted into layers.
    pub shared_git: Vec<PathBuf>,
}

#[cfg(test)]
fn read(request: &DiskRequest) -> Vec<DiskUsage> {
    read_with(request, |_| true)
}

/// Bytes free to an unprivileged writer on the volume holding `path`.
pub fn volume_free_bytes(path: &Path) -> Option<u64> {
    space::free_bytes(path).ok()
}

/// Where a walked entry is counted.
#[derive(Clone, Copy)]
enum Bucket {
    Source,
    /// The top ignored folder at this index of the tally.
    Folder(usize),
}

struct Item {
    path: PathBuf,
    bucket: Bucket,
    /// The ignore rules of the folder above; a folder adds its own `.gitignore`.
    rules: Option<Rc<IgnoreRules>>,
}

/// Measures every requested root, calling `finished` with each as it is done;
/// `finished` answering `false` stops before the next root, which the caller
/// no longer wants. Each root has its own entry and time limit; one that runs
/// out is the only one without a total.
pub fn read_with(
    request: &DiskRequest,
    finished: impl FnMut(&DiskUsage) -> bool,
) -> Vec<DiskUsage> {
    read_limited(
        request,
        RequestBudget::new(REQUEST_ENTRY_LIMIT, REQUEST_SEEN_LIMIT, REQUEST_TIME_LIMIT),
        finished,
    )
}

fn read_limited(
    request: &DiskRequest,
    mut budget: RequestBudget,
    mut finished: impl FnMut(&DiskUsage) -> bool,
) -> Vec<DiskUsage> {
    // The deepest explicitly requested root owns its subtree. Shared Git and
    // nested linked worktrees are therefore excluded from the main component.
    // One inode set also counts hard links once across sibling components.
    let mut roots = request.paths.clone();
    roots.sort_by(|a, b| {
        b.components()
            .count()
            .cmp(&a.components().count())
            .then(a.cmp(b))
    });
    roots.dedup();
    let root_set: HashSet<_> = roots.iter().cloned().collect();
    let measured_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|t| t.as_millis() as u64);
    let free = request
        .paths
        .first()
        .and_then(|path| volume_free_bytes(path));
    let total = request
        .paths
        .first()
        .and_then(|path| space::total_bytes(path).ok());
    let mut result = Vec::new();
    for root in &roots {
        let layered = !request.shared_git.contains(root);
        let row = measure_root(root, &root_set, &mut budget, layered);
        let row = DiskUsage {
            measured_at_unix_ms: measured_at,
            volume_free_bytes: free,
            volume_total_bytes: total,
            ..row
        };
        let go_on = finished(&row);
        result.push(row);
        if !go_on {
            break;
        }
    }
    // Preserve caller order for stable identities and unchanged snapshot reuse.
    result.sort_by_key(|row| {
        request
            .paths
            .iter()
            .position(|p| Some(p.to_string_lossy().as_ref()) == row.path.as_deref())
    });
    result
}

fn measure_root(
    root: &PathBuf,
    root_set: &HashSet<PathBuf>,
    budget: &mut RequestBudget,
    layered: bool,
) -> DiskUsage {
    let started = std::time::Instant::now();
    let mut visited = 0usize;
    let mut bytes = 0u64;
    let mut children = BTreeMap::<String, u64>::new();
    let mut tally = Tally::default();
    let mut failure: Option<Failure> = None;
    let base = layered.then(|| Rc::new(base_rules(root, exclude_dir(root).as_deref())));
    let mut stack = vec![Item {
        path: root.clone(),
        bucket: Bucket::Source,
        rules: base,
    }];
    // Reject alias roots rather than following a symlink outside the
    // declared measurement boundary. Descendant links count their own
    // allocated blocks and are never traversed.
    if hide_platform::fs::identity::canonical(root).is_ok_and(|canonical| canonical != *root) {
        failure = Some(Failure::new(
            "alias",
            "The measurement root is an alias. Refresh with the canonical checkout path.",
        ));
        stack.clear();
    }
    if budget.spent() {
        failure = Some(Failure::new(
            "request_limit",
            "The measurement request ran past its limits before reaching this checkout. Measure again.",
        ));
        stack.clear();
    }
    while let Some(item) = stack.pop() {
        let Item {
            path,
            bucket,
            rules,
        } = item;
        visited += 1;
        budget.visited += 1;
        if visited > ENTRY_LIMIT || started.elapsed() > TIME_LIMIT {
            failure = Some(Failure::new(
                "limit",
                "Measurement exceeded its 30 second / one million entry limit. This component is unavailable; measure again after reducing the folder size.",
            ));
            break;
        }
        if budget.spent() {
            failure = Some(Failure::new(
                "request_limit",
                "The measurement request ran past its limits. This component is unavailable; measure again.",
            ));
            break;
        }
        if path != *root && root_set.contains(&path) {
            continue;
        }
        let usage = match space::usage_nofollow(&path) {
            Ok(value) => value,
            Err(error) => {
                failure = Some(Failure::incomplete(error));
                break;
            }
        };
        // Only a file with several links can be reached twice.
        if usage.links > 1 && !usage.is_dir && !budget.seen.insert(usage.id) {
            continue;
        }
        let allocated = usage.allocated;
        bytes = bytes.saturating_add(allocated);
        match bucket {
            Bucket::Source => tally.source_bytes = tally.source_bytes.saturating_add(allocated),
            Bucket::Folder(index) => {
                let folder = &mut tally.folders[index];
                folder.bytes = folder.bytes.saturating_add(allocated);
                if path.file_name().is_some_and(|name| name == ".git") {
                    folder.repository = true;
                }
            }
        }
        if let Ok(relative) = path.strip_prefix(root)
            && let Some(name) = relative.components().next()
        {
            *children
                .entry(name.as_os_str().to_string_lossy().into_owned())
                .or_default() += allocated;
        }
        if usage.is_dir {
            match std::fs::read_dir(&path) {
                Ok(entries) => {
                    let mut descendants = Vec::new();
                    for entry in entries {
                        if descendants.len() + stack.len() + visited >= ENTRY_LIMIT {
                            failure = Some(Failure::new(
                                "limit",
                                "Measurement exceeded its one million entry limit.",
                            ));
                            break;
                        }
                        match entry {
                            Ok(entry) => descendants.push((entry.path(), entry.file_type().ok())),
                            Err(error) => failure = Some(Failure::incomplete(error)),
                        }
                    }
                    descendants.sort_by(|a, b| a.0.cmp(&b.0));
                    // Inside a counted folder nothing is sorted again.
                    let sorted = match (bucket, rules) {
                        (Bucket::Source, Some(rules)) => {
                            let here = Rc::new(rules_in(&rules, &path));
                            let siblings =
                                Siblings::new(descendants.iter().filter_map(|(child, _)| {
                                    child.file_name().map(|n| n.to_string_lossy().into_owned())
                                }));
                            Some((here, siblings))
                        }
                        _ => None,
                    };
                    for (child, kind) in descendants.into_iter().rev() {
                        let (bucket, rules) = match (&sorted, bucket) {
                            (Some((rules, siblings)), Bucket::Source) => {
                                let bucket = sort_child(&child, kind, rules, siblings, &mut tally);
                                (bucket, Some(Rc::clone(rules)))
                            }
                            _ => (bucket, None),
                        };
                        stack.push(Item {
                            path: child,
                            bucket,
                            rules,
                        });
                    }
                }
                Err(error) => {
                    failure = Some(Failure::incomplete(error));
                    break;
                }
            }
        }
    }
    let largest = children.into_iter().max_by_key(|(_, size)| *size);
    let (layers, folders) = if failure.is_none() && layered {
        let (layers, folders) = tally.finish(root);
        (Some(layers), folders)
    } else {
        (None, Vec::new())
    };
    DiskUsage {
        path: Some(root.to_string_lossy().into_owned()),
        total_bytes: failure.is_none().then_some(bytes),
        largest_child_name: largest.as_ref().map(|(name, _)| name.clone()),
        largest_child_bytes: largest.map(|(_, size)| size),
        unavailable_code: failure.as_ref().map(|failure| failure.code.to_owned()),
        unavailable_reason: failure.map(|failure| failure.reason),
        layers,
        folders,
        ..Default::default()
    }
}

/// Where one child of a source folder is counted: source, or a new top
/// ignored folder of the tally.
fn sort_child(
    child: &Path,
    kind: Option<std::fs::FileType>,
    rules: &IgnoreRules,
    siblings: &Siblings,
    tally: &mut Tally,
) -> Bucket {
    if child.file_name().is_some_and(|name| name == ".git") {
        return Bucket::Source;
    }
    let is_dir = kind.is_some_and(|kind| kind.is_dir());
    if !rules.ignores(child, is_dir) {
        return Bucket::Source;
    }
    // A link, a file, or a folder nothing vouches for is `other`; only a
    // real folder can be a layer.
    let layer: Option<Layer> = if is_dir {
        layer_of(child, siblings)
    } else {
        None
    };
    tally.folders.push(FolderTally {
        path: child.to_path_buf(),
        layer,
        bytes: 0,
        repository: false,
    });
    Bucket::Folder(tally.folders.len() - 1)
}

/// The shared Git directory's `info` folder, where `exclude` lives.
fn exclude_dir(root: &Path) -> Option<PathBuf> {
    let repository = hide_project::git::discover(root)?;
    Some(repository.common_dir.join("info"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_disk_partitions_nested_worktrees_and_shared_git_once() {
        let (_folder, root) = fixture("partition");
        std::fs::create_dir_all(root.join("linked")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("main-data"), vec![1; 8192]).unwrap();
        std::fs::write(root.join("linked/data"), vec![2; 16384]).unwrap();
        std::fs::write(root.join(".git/data"), vec![3; 32768]).unwrap();
        let expected = allocated(&[
            root.clone(),
            root.join("linked"),
            root.join(".git"),
            root.join("main-data"),
            root.join("linked/data"),
            root.join(".git/data"),
        ]);
        let measurements = read(&DiskRequest {
            paths: vec![root.clone(), root.join("linked"), root.join(".git")],
            ..Default::default()
        });
        let total: u64 = measurements.iter().map(|d| d.total_bytes.unwrap()).sum();
        assert_eq!(
            total, expected,
            "Every allocated block belongs to one project component"
        );
    }

    #[test]
    fn hardlinks_count_once_and_symlinks_do_not_escape_or_cycle() {
        use hide_platform::fs::link::create_link;
        let (_folder, root) = fixture("links");
        let (_outside, outside) = fixture("outside");
        std::fs::create_dir_all(root.join("linked")).unwrap();
        std::fs::write(root.join("linked/data"), vec![2; 16384]).unwrap();
        std::fs::hard_link(root.join("linked/data"), root.join("alias")).unwrap();
        std::fs::write(outside.join("secret"), vec![3; 65536]).unwrap();
        create_link(&outside, &root.join("escape")).unwrap();
        create_link(&root, &root.join("cycle")).unwrap();
        let expected = allocated(&[
            root.clone(),
            root.join("linked"),
            root.join("linked/data"),
            root.join("escape"),
            root.join("cycle"),
        ]);
        let values = read(&DiskRequest {
            paths: vec![root.clone(), root.join("linked")],
            ..Default::default()
        });
        assert_eq!(
            values.iter().map(|v| v.total_bytes.unwrap()).sum::<u64>(),
            expected
        );
        assert_eq!(
            values[1].total_bytes,
            Some(allocated(&[root.join("linked"), root.join("linked/data")]))
        );
        let alias = read(&DiskRequest {
            paths: vec![root.join("escape")],
            ..Default::default()
        });
        assert_eq!(alias[0].total_bytes, None);
        assert!(alias[0].unavailable_reason.is_some());
        assert!(outside.join("secret").exists());
    }

    #[test]
    fn partial_measurement_retains_target_failure_without_a_complete_total() {
        let (_folder, root) = fixture("partial");
        std::fs::write(root.join("data"), vec![1; 8192]).unwrap();
        let rows = read(&DiskRequest {
            paths: vec![root.clone(), root.join("missing")],
            ..Default::default()
        });
        assert!(rows[0].total_bytes.is_some());
        assert!(rows[1].total_bytes.is_none());
        assert!(rows[1].unavailable_reason.is_some());
        assert_eq!(
            rows.iter().map(|row| row.total_bytes).sum::<Option<u64>>(),
            None
        );
    }

    /// A new folder and its real path; the test keeps the folder.
    fn fixture(name: &str) -> (tempfile::TempDir, PathBuf) {
        let folder = tempfile::Builder::new()
            .prefix(&format!("hide-disk-{name}-"))
            .tempdir()
            .unwrap();
        let root = std::fs::canonicalize(folder.path()).unwrap();
        (folder, root)
    }

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![7; bytes]).unwrap();
    }

    fn allocated(paths: &[PathBuf]) -> u64 {
        paths
            .iter()
            .map(|p| space::usage_nofollow(p).unwrap().allocated)
            .sum()
    }

    fn layered_checkout(root: &Path) {
        write(&root.join("Cargo.toml"), 100);
        write(&root.join("src/main.rs"), 5000);
        write(
            &root.join(".gitignore"),
            b"target/\n/agents/\n/dist/\n".len(),
        );
        std::fs::write(root.join(".gitignore"), "target/\n/agents/\n/dist/\n").unwrap();
        write(&root.join("target/debug/big"), 40_000);
        std::fs::write(
            root.join("target/CACHEDIR.TAG"),
            "Signature: 8a477f597d28d172789f06886806bc55\n",
        )
        .unwrap();
        write(&root.join("web/package.json"), 100);
        std::fs::write(root.join("web/.gitignore"), "node_modules\ndist\n").unwrap();
        write(&root.join("web/node_modules/dep/index.js"), 20_000);
        write(&root.join("web/dist/app.js"), 9_000);
        write(&root.join("agents/runs/log"), 30_000);
        write(&root.join("dist/handmade"), 6_000);
    }

    #[test]
    fn a_checkout_is_sorted_into_source_two_layers_and_the_rest() {
        let (_folder, root) = fixture("layers");
        layered_checkout(&root);
        let rows = read(&DiskRequest {
            paths: vec![root.clone()],
            ..Default::default()
        });
        let row = &rows[0];
        let layers = row.layers.as_ref().expect("a checkout row has layers");
        // Everything below a vouched folder, the folder itself included.
        let build_cache = allocated(&[
            root.join("target"),
            root.join("target/debug"),
            root.join("target/debug/big"),
            root.join("target/CACHEDIR.TAG"),
            root.join("web/dist"),
            root.join("web/dist/app.js"),
        ]);
        let dependencies = allocated(&[
            root.join("web/node_modules"),
            root.join("web/node_modules/dep"),
            root.join("web/node_modules/dep/index.js"),
        ]);
        // `/dist` has no package.json beside it, and `agents` has no rule.
        let other = allocated(&[
            root.join("agents"),
            root.join("agents/runs"),
            root.join("agents/runs/log"),
            root.join("dist"),
            root.join("dist/handmade"),
        ]);
        assert_eq!(layers.build_cache.bytes, build_cache);
        assert_eq!(layers.build_cache.folders, 2);
        assert_eq!(layers.build_cache.largest_name.as_deref(), Some("target"));
        assert_eq!(layers.dependencies.bytes, dependencies);
        assert_eq!(
            layers.dependencies.largest_name.as_deref(),
            Some("web/node_modules")
        );
        assert_eq!(layers.other.bytes, other);
        assert_eq!(layers.other.folders, 2);
        assert_eq!(layers.other.largest_name.as_deref(), Some("agents"));
        let total = row.total_bytes.unwrap();
        assert_eq!(
            layers.build_cache.bytes
                + layers.dependencies.bytes
                + layers.other.bytes
                + layers.source_bytes,
            total,
            "every allocated block is in exactly one place"
        );
        assert!(
            layers.source_bytes >= allocated(&[root.join("src/main.rs"), root.join("Cargo.toml")])
        );
        let mut kept: Vec<_> = row.folders.iter().map(|f| f.path.clone()).collect();
        kept.sort();
        assert_eq!(
            kept,
            vec![
                root.join("target"),
                root.join("web/dist"),
                root.join("web/node_modules")
            ]
        );
    }

    #[test]
    fn a_folder_holding_another_repository_is_not_a_layer() {
        let (_folder, root) = fixture("repository");
        write(&root.join("pyproject.toml"), 100);
        std::fs::write(root.join(".gitignore"), ".venv\n").unwrap();
        write(&root.join(".venv/lib/x.py"), 10_000);
        write(&root.join(".venv/src/pkg/.git/HEAD"), 100);
        let rows = read(&DiskRequest {
            paths: vec![root.clone()],
            ..Default::default()
        });
        let layers = rows[0].layers.as_ref().unwrap();
        assert_eq!(layers.dependencies.folders, 0);
        assert_eq!(layers.other.folders, 1);
        assert!(rows[0].folders.is_empty(), "nothing to remove is kept");
    }

    #[test]
    fn the_shared_git_directory_has_a_size_and_no_layers() {
        let (_folder, root) = fixture("shared-git");
        write(&root.join("wt/Cargo.toml"), 100);
        write(&root.join("common/objects/pack"), 20_000);
        let rows = read(&DiskRequest {
            paths: vec![root.join("wt"), root.join("common")],
            shared_git: vec![root.join("common")],
        });
        assert!(rows[0].layers.is_some());
        assert!(rows[1].layers.is_none());
        assert!(rows[1].total_bytes.is_some());
    }

    #[test]
    fn each_root_is_reported_as_it_finishes_and_a_failed_one_only_loses_itself() {
        let (_folder, root) = fixture("progress");
        write(&root.join("a/file"), 4096);
        let mut order = Vec::new();
        let rows = read_with(
            &DiskRequest {
                paths: vec![root.join("a"), root.join("missing")],
                ..Default::default()
            },
            |row| {
                order.push((row.path.clone().unwrap(), row.total_bytes.is_some()));
                true
            },
        );
        assert_eq!(order.len(), 2);
        assert_eq!(order.iter().filter(|(_, measured)| *measured).count(), 1);
        assert!(rows[0].layers.is_some() && rows[1].layers.is_none());
    }

    #[test]
    fn a_request_that_runs_past_its_own_limit_leaves_the_checkouts_it_did_not_reach_unavailable() {
        let (_folder, root) = fixture("request-limit");
        for name in ["a", "b", "c"] {
            for n in 0..5 {
                write(&root.join(name).join(format!("file{n}")), 100);
            }
        }
        // Each checkout is well inside its own limit; the request as a whole is not.
        let rows = read_limited(
            &DiskRequest {
                paths: vec![root.join("a"), root.join("b"), root.join("c")],
                ..Default::default()
            },
            RequestBudget::new(8, 1_000, Duration::from_secs(60)),
            |_| true,
        );
        let measured = rows.iter().filter(|row| row.total_bytes.is_some()).count();
        assert!(measured < 3, "the request stopped: {measured} measured");
        let cut = rows.iter().find(|row| row.total_bytes.is_none()).unwrap();
        assert!(cut.layers.is_none() && cut.unavailable_reason.is_some());
    }

    /// PRD project-panel-results B20: a caller that no longer wants the
    /// measurement stops it before the next checkout.
    #[test]
    fn a_caller_that_stops_wanting_the_measurement_ends_it_before_the_next_checkout() {
        let (_folder, root) = fixture("stop");
        for name in ["a", "b", "c"] {
            write(&root.join(name).join("file"), 4096);
        }
        let mut reported = 0;
        let rows = read_with(
            &DiskRequest {
                paths: vec![root.join("a"), root.join("b"), root.join("c")],
                ..Default::default()
            },
            |_| {
                reported += 1;
                false
            },
        );
        assert_eq!((reported, rows.len()), (1, 1));
    }

    #[test]
    fn a_measurement_carries_the_free_space_of_its_volume() {
        let (_folder, root) = fixture("free");
        let rows = read(&DiskRequest {
            paths: vec![root.clone()],
            ..Default::default()
        });
        assert!(rows[0].volume_free_bytes.is_some_and(|free| free > 0));
        assert!(rows[0].volume_total_bytes >= rows[0].volume_free_bytes);
    }

    #[test]
    fn an_unreadable_request_measures_nothing() {
        let measured = read(&DiskRequest {
            paths: vec![PathBuf::from("/definitely/not/here/hide-test")],
            ..Default::default()
        });
        let measured = &measured[0];
        assert_eq!(measured.total_bytes, None);
        assert!(measured.unavailable_reason.is_some());
    }

    #[test]
    fn no_selection_measures_nothing_at_all() {
        assert!(read(&DiskRequest::default()).is_empty());
    }
}
