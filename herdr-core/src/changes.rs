//! The right panel's changes view: one checkout's Git working-tree state, read
//! by the host that holds the checkout (PRD S5.5 B19-B22).
//!
//! The read is `hide_host::git` behind the checkout's `NodeLink`: this
//! machine's in process, a device's through its helper, so a local and a
//! device checkout answer one contract. It runs on a reader thread, never
//! under the runtime mutex and never on a Herdr session's coordinator: a
//! device's History does not depend on this machine running Herdr.
//! [`ChangesPump`] drives [`ChangesReader`], which recomputes only when the
//! request changes or the refresh window lapses, and reads nothing at all
//! while neither Changes nor Explorer is visible and no diff tab needs it.
//! Each answer names the device and folder it describes, so the runtime drops
//! one that arrives after the operator moved on. The same read takes the
//! diff of every diff the front Workspace's View areas show (PRD S7 A5), so
//! several diffs on screen cost one read, not one each.

use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread;
use std::time::Duration;

use hide_node_link::ErrorCode;
use hide_node_link::git::{ChangedFile, Changes, DiffTarget, FileStatus};
use hide_node_link::protocol::Call;
use hide_platform::path;

use crate::files::DocumentRoot;
use crate::handle::ChangeNotifier;
use crate::model::{
    ChangedFileDiffSnapshot, ChangedFileSnapshot, ChangedFileStatus, ChangesSnapshot,
    ViewDiffSnapshot,
};
use crate::node_access::{LinkError, NodeLink, call_as};
use crate::reader::BackgroundRead;
use crate::runtime::Runtime;

/// How stale the list may be while the view is open. Short enough that an edit
/// made in a terminal pane shows up by the time the operator looks over, long
/// enough that it is nowhere near a per-tick fork.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// How often the pump asks the runtime what History needs and collects a
/// finished read: the latency between selecting a row and its read starting,
/// and between a read finishing and the screen showing it. One brief lock per
/// wake; the read itself runs on the reader's thread.
const PUMP_TICK: Duration = Duration::from_millis(250);

/// How long one read may take on the checkout's host before it is reported
/// as unavailable. A device's Git runs over its SSH connection.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The host a request reads through, compared by identity: a reconnect gives
/// a device a new channel, which is a new request and reads again.
#[derive(Clone)]
pub struct ChannelRef(pub Arc<dyn NodeLink>);

impl PartialEq for ChannelRef {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for ChannelRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ChannelRef")
    }
}

/// What the runtime wants read: the checkout, the folder under it the view
/// describes, the file whose diff to fetch, and what the committed group is
/// measured against. Absent while neither Changes nor Explorer is showing and
/// no diff tab is active, which keeps the reader out of the common
/// hidden-panel path.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangesRequest {
    /// The checkout's root on its device, which the host opens and runs Git
    /// in.
    pub root: DocumentRoot,
    /// The channel to that device's host, or why there is none now.
    pub channel: Result<ChannelRef, String>,
    /// The folder the view describes: the checkout root, or a registered
    /// folder below it.
    pub root_path: String,
    pub selected_path: Option<String>,
    /// Whether the selection is in the committed group, which decides what
    /// its diff is taken against.
    pub selected_committed: bool,
    /// What the committed group is measured against, from the worktree
    /// reader's answer. `None` lets the host use the repository's default
    /// branch, and without one only the uncommitted group is produced.
    pub base_branch: Option<String>,
    /// The diffs the front Workspace's View areas show, by absolute path,
    /// at most one per area; empty in a shell without View areas.
    pub diffs: Vec<DiffTarget>,
}

impl ChangesRequest {
    pub fn key(&self) -> ChangesKey {
        ChangesKey {
            device_id: self.root.device_id.clone(),
            root_path: self.root_path.clone(),
        }
    }

    /// The History row whose diff this request takes, and its group.
    pub fn selection(&self) -> (Option<String>, bool) {
        (self.selected_path.clone(), self.selected_committed)
    }
}

/// Which device's folder an answer describes. The same path on two devices
/// is two keys.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangesKey {
    pub device_id: String,
    pub root_path: String,
}

pub struct ChangesAnswer {
    /// `None` for the closed view's empty projection.
    pub key: Option<ChangesKey>,
    /// Why the read failed in the host's own words, for the log; the
    /// snapshot carries only its code.
    pub detail: Option<String>,
    /// The row its request asked for (`ChangesRequest::selection`). An
    /// answer lands one wake after its request, so the operator may have
    /// chosen another row since.
    pub selection: (Option<String>, bool),
    pub changes: ChangesSnapshot,
}

pub struct ChangesReader {
    inner: BackgroundRead<Option<ChangesRequest>, ChangesAnswer>,
}

impl ChangesReader {
    pub fn new() -> Self {
        Self {
            inner: BackgroundRead::new(
                REFRESH_INTERVAL,
                Duration::ZERO,
                |request: &Option<ChangesRequest>| {
                    match request {
                        Some(request) => {
                            let (changes, detail) = read(request);
                            ChangesAnswer {
                                key: Some(request.key()),
                                detail,
                                selection: request.selection(),
                                changes,
                            }
                        }
                        // The view is closed, so there is nothing to describe. An
                        // empty projection also drops the retained diff text off
                        // the wire.
                        None => ChangesAnswer {
                            key: None,
                            detail: None,
                            selection: (None, false),
                            changes: ChangesSnapshot::default(),
                        },
                    }
                },
            ),
        }
    }

    /// An answer on the wake its read finishes, `None` otherwise. A changed
    /// request always starts a read, so selecting a file does not wait out
    /// the window.
    pub fn read_if_due(&mut self, request: Option<ChangesRequest>) -> Option<ChangesAnswer> {
        self.inner.poll(request)
    }

    /// Waits for the read in flight, so a test's next poll takes its answer.
    #[cfg(test)]
    pub(crate) fn join_pending(&mut self) {
        self.inner.join_pending();
    }
}

impl Default for ChangesReader {
    fn default() -> Self {
        Self::new()
    }
}

/// The one thread that keeps History current for the checkout in front,
/// whichever device holds it. It lives with the core, not with a Herdr
/// session, so a hided that serves only devices still reads their History.
pub(crate) struct ChangesPump {
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ChangesPump {
    pub fn spawn(runtime: Weak<Mutex<Runtime>>, notifier: ChangeNotifier) -> std::io::Result<Self> {
        let (stop, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("changes-pump".into())
            .spawn(move || {
                let mut reader = ChangesReader::new();
                let mut git = crate::checkout_git::git_reader();
                let mut ci = crate::checkout_git::ci_reader();
                // The closed view's empty projection has landed: until
                // something shows the view again there is nothing to read.
                let mut resting = false;
                while matches!(
                    receiver.recv_timeout(PUMP_TICK),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    let Some(runtime) = runtime.upgrade() else {
                        break;
                    };
                    let Ok((git_request, target_changed)) = runtime
                        .lock()
                        .map(|mut runtime| runtime.prepare_checkout_header())
                    else {
                        break;
                    };
                    let git_answer = git.poll(git_request).flatten();
                    let Ok((ci_request, git_changed)) = runtime.lock().map(|mut runtime| {
                        let changed = git_answer.is_some_and(|answer| {
                            runtime.ingest_checkout_git(answer, std::time::Instant::now())
                        });
                        (runtime.checkout_ci_request(), changed)
                    }) else {
                        break;
                    };
                    let ci_answer = ci.poll(ci_request).flatten();
                    let Ok(header_changed) = runtime.lock().map(|mut runtime| {
                        let ci_changed = ci_answer.is_some_and(|answer| {
                            runtime.ingest_checkout_ci(answer, std::time::Instant::now())
                        });
                        let aged = runtime.age_checkout_header(std::time::Instant::now());
                        let changed = target_changed || git_changed || ci_changed || aged;
                        if changed {
                            runtime.refresh_checkout_header();
                        }
                        changed
                    }) else {
                        break;
                    };
                    if header_changed {
                        notifier.notify();
                    }
                    let Ok(request) = runtime.lock().map(|mut runtime| runtime.changes_request())
                    else {
                        break;
                    };
                    if request.is_none() && resting {
                        continue;
                    }
                    let Some(answer) = reader.read_if_due(request) else {
                        continue;
                    };
                    resting = answer.key.is_none();
                    let Ok(changed) = runtime
                        .lock()
                        .map(|mut runtime| runtime.ingest_changes(answer))
                    else {
                        break;
                    };
                    drop(runtime);
                    if changed {
                        notifier.notify();
                    }
                }
                git.join_pending();
                ci.join_pending();
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for ChangesPump {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Reads `request` on its checkout's host and projects the answer. Blocks on
/// the channel; a refusal or a lost answer is a stated reason, never an empty
/// list, so a failure is not shown as a clean tree. The reason is a code the
/// screen words (`changes.*`, `host.*`, `link.*`); the host's sentence comes
/// back beside the snapshot for the log.
pub fn read(request: &ChangesRequest) -> (ChangesSnapshot, Option<String>) {
    let root_path = request.root_path.clone();
    let unavailable = |code: String, detail: String| {
        let changes = ChangesSnapshot {
            root_path: Some(root_path.clone()),
            unavailable_reason: Some(code),
            ..ChangesSnapshot::default()
        };
        (changes, Some(detail))
    };
    let channel = match &request.channel {
        Ok(channel) => Arc::clone(&channel.0),
        Err(reason) => return unavailable("link.not_connected".to_owned(), reason.clone()),
    };
    let failed = |error: LinkError| {
        let (code, detail) = failed_code(error);
        unavailable(code, detail)
    };
    // Every path here is the checkout device's, in the wire spelling, and is
    // related by names alone, whatever system the device runs.
    let Ok(scope) =
        path::wire_relative(&request.root.path, &request.root_path).map(path::RelPath::into_string)
    else {
        return unavailable(
            "changes.folder_moved".to_owned(),
            "this History folder no longer matches its registered checkout".to_owned(),
        );
    };
    let relative = |path: &str| {
        path::wire_relative(&request.root_path, path)
            .ok()
            .map(path::RelPath::into_string)
    };
    let selected = request.selected_path.as_deref().and_then(relative);
    // A View diff outside the folder History reads cannot be taken here: the
    // helper answers only paths under that folder, and a Workspace's views
    // are shared by every Project registered in its checkout. Its display
    // says so, naming the folder, rather than waiting for text that never
    // comes.
    let mut diffs = Vec::new();
    let mut outside = Vec::new();
    for target in &request.diffs {
        match relative(&target.path) {
            Some(path) => diffs.push(DiffTarget {
                path,
                committed: target.committed,
            }),
            None => outside.push(target),
        }
    }
    let root = match crate::files::root_ref(channel.as_ref(), &request.root) {
        Ok(root) => root,
        Err(error) => return failed(error),
    };
    let answer: Result<Changes, _> = call_as(
        channel.as_ref(),
        Call::Changes {
            root,
            scope,
            selected,
            committed: request.selected_committed,
            base: request.base_branch.clone(),
            diffs,
        },
        READ_TIMEOUT,
    );
    let changes = match answer {
        Ok(changes) => changes,
        // The host answers `Unsupported` only for a folder Git finds no
        // repository for (`hide_host::git::changes`).
        Err(LinkError::Refused(error)) if error.code == ErrorCode::Unsupported => {
            let (changes, detail) =
                unavailable("changes.not_a_repository".to_owned(), error.message);
            return (
                ChangesSnapshot {
                    not_a_repository: true,
                    ..changes
                },
                detail,
            );
        }
        Err(error) => return failed(error),
    };
    let project = |files: Vec<ChangedFile>| {
        files
            .into_iter()
            .map(|file| snapshot_of(&request.root_path, file))
            .collect::<Vec<_>>()
    };
    let entries = project(changes.entries);
    let committed = changes.committed.map(project);
    // A base the host could not resolve means there is nothing to compare
    // against, so the committed group is absent rather than empty.
    let base_branch = committed.as_ref().and(changes.base);
    let diff = changes.diff.map(|diff| ChangedFileDiffSnapshot {
        path: absolute(&request.root_path, &diff.path),
        notice: diff_notice(&diff),
        text: diff.text,
    });
    let diffs = changes
        .diffs
        .into_iter()
        .map(|shown| ViewDiffSnapshot {
            path: absolute(&request.root_path, &shown.diff.path),
            committed: shown.committed,
            notice: diff_notice(&shown.diff),
            text: shown.diff.text,
        })
        .chain(outside.into_iter().map(|target| ViewDiffSnapshot {
            path: target.path.clone(),
            committed: target.committed,
            text: String::new(),
            notice: Some(hide_node_link::git::DiffNotice::Outside {
                root: root_path.clone(),
            }),
        }))
        .collect();
    let changes = ChangesSnapshot {
        root_path: Some(root_path),
        selected_path: diff.as_ref().map(|diff| diff.path.clone()),
        selected_committed: request.selected_committed && diff.is_some(),
        entries,
        committed: committed.unwrap_or_default(),
        base_branch,
        diff,
        diffs,
        unavailable_reason: None,
        not_a_repository: false,
        stale_reason: None,
    };
    (changes, None)
}

/// The code a failed host call shows as, with its sentence for the log. Git
/// failing on the repository is the host's `Io`, which History names as Git's.
fn failed_code(error: LinkError) -> (String, String) {
    match error {
        LinkError::Refused(error) if error.code == ErrorCode::Io => {
            ("changes.git_failed".to_owned(), error.message)
        }
        LinkError::Refused(error) => {
            let code = crate::node_access::link_failure_code(&LinkError::Refused(error.clone()));
            (code, error.message)
        }
        other => (
            crate::node_access::link_failure_code(&other),
            other.to_string(),
        ),
    }
}

/// A path the host answered below `root_path`: both are the device's wire
/// spelling, which joins with `/` whatever system the device runs.
fn absolute(root_path: &str, relative: &str) -> String {
    format!("{}/{relative}", root_path.trim_end_matches('/'))
}

fn snapshot_of(root_path: &str, file: ChangedFile) -> ChangedFileSnapshot {
    ChangedFileSnapshot {
        path: absolute(root_path, &file.path),
        relative_path: file.path,
        previous_relative_path: file.previous,
        status: match file.status {
            FileStatus::Modified => ChangedFileStatus::Modified,
            FileStatus::Added => ChangedFileStatus::Added,
            FileStatus::Deleted => ChangedFileStatus::Deleted,
            FileStatus::Untracked => ChangedFileStatus::Untracked,
            FileStatus::Renamed => ChangedFileStatus::Renamed,
            FileStatus::Conflict => ChangedFileStatus::Conflict,
        },
        added_lines: file.added,
        removed_lines: file.removed,
    }
}

/// What the page says about a diff that is short or absent. A helper of an
/// older build sends only its sentence, which reads as unreadable; a diff Git
/// could not take keeps its reason in the log.
fn diff_notice(diff: &hide_node_link::git::Diff) -> Option<hide_node_link::git::DiffNotice> {
    use hide_node_link::git::DiffNotice;
    let notice = diff
        .kind
        .clone()
        .or_else(|| diff.notice.as_ref().map(|_| DiffNotice::Unreadable));
    if notice == Some(DiffNotice::Unreadable) {
        crate::diagnostic!(serde_json::json!({
            "component": "changes", "kind": "diff_unreadable",
            "path": diff.path, "reason": diff.notice,
        }));
    }
    notice
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(directory: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {arguments:?}");
    }

    fn request(
        checkout: &std::path::Path,
        folder: &std::path::Path,
        selected: Option<&std::path::Path>,
    ) -> ChangesRequest {
        ChangesRequest {
            root: DocumentRoot {
                device_id: crate::node::TEST_NODE.to_owned(),
                path: checkout.to_string_lossy().into_owned(),
                identity: None,
            },
            channel: Ok(ChannelRef(Arc::new(hide_node::Local::of_process()))),
            root_path: folder.to_string_lossy().into_owned(),
            selected_path: selected.map(|path| path.to_string_lossy().into_owned()),
            selected_committed: false,
            base_branch: None,
            diffs: Vec::new(),
        }
    }

    #[test]
    fn a_registered_folder_projects_absolute_paths_under_itself() {
        let temporary = tempfile::tempdir().unwrap();
        let checkout = temporary.path().canonicalize().unwrap();
        git(&checkout, &["init", "-q"]);
        let folder = checkout.join("registered");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("inside.txt"), "INSIDE\n").unwrap();
        std::fs::write(checkout.join("outside.txt"), "OUTSIDE\n").unwrap();

        let (listed, _) = read(&request(
            &checkout,
            &folder,
            Some(&folder.join("inside.txt")),
        ));
        assert_eq!(listed.unavailable_reason, None);
        assert_eq!(listed.entries.len(), 1);
        assert_eq!(listed.entries[0].relative_path, "inside.txt");
        assert_eq!(
            listed.entries[0].path,
            folder.join("inside.txt").to_string_lossy()
        );
        assert_eq!(listed.entries[0].status, ChangedFileStatus::Untracked);
        let diff = listed.diff.unwrap();
        assert_eq!(diff.path, folder.join("inside.txt").to_string_lossy());
        assert!(diff.text.contains("INSIDE"));
        assert_eq!(listed.selected_path.as_deref(), Some(diff.path.as_str()));
        assert!(listed.committed.is_empty());
        assert_eq!(listed.base_branch, None);
    }

    /// A5, contract 3.2, B16: a View display names its diff by absolute
    /// path, the helper answers under the folder History reads, and the
    /// answer comes back under the path the display asked with. A diff
    /// outside that folder is not taken, and its display says why and names
    /// the folder, so the empty diff never reads as a file without changes.
    #[test]
    fn view_diffs_come_back_under_their_paths_and_one_outside_names_the_folder() {
        let temporary = tempfile::tempdir().unwrap();
        let checkout = temporary.path().canonicalize().unwrap();
        git(&checkout, &["init", "-q"]);
        let folder = checkout.join("registered");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("inside.txt"), "INSIDE\n").unwrap();
        std::fs::write(checkout.join("outside.txt"), "OUTSIDE\n").unwrap();
        let inside = folder.join("inside.txt").to_string_lossy().into_owned();
        let outside = checkout.join("outside.txt").to_string_lossy().into_owned();
        let mut asked = request(&checkout, &folder, None);
        asked.diffs = [&inside, &outside]
            .into_iter()
            .map(|path| DiffTarget {
                path: path.clone(),
                committed: false,
            })
            .collect();

        let (listed, _) = read(&asked);
        let [taken, left] = listed.diffs.as_slice() else {
            panic!("two View diffs: {:?}", listed.diffs);
        };
        assert_eq!(taken.path, inside);
        assert!(taken.text.contains("INSIDE"), "{}", taken.text);
        assert_eq!(taken.notice, None);
        assert_eq!(
            (left.path.as_str(), left.text.as_str()),
            (outside.as_str(), "")
        );
        assert_eq!(
            left.notice,
            Some(hide_node_link::git::DiffNotice::Outside {
                root: folder.to_string_lossy().into_owned()
            }),
            "the notice names the folder History reads"
        );
    }

    #[test]
    fn a_host_that_cannot_answer_states_why_instead_of_a_clean_tree() {
        let temporary = tempfile::tempdir().unwrap();
        let folder = temporary.path().canonicalize().unwrap();
        let mut unready = request(&folder, &folder, None);
        unready.channel = Err("The device helper is not ready".to_owned());
        let (projected, detail) = read(&unready);
        assert_eq!(
            projected.unavailable_reason.as_deref(),
            Some("link.not_connected")
        );
        assert_eq!(detail.as_deref(), Some("The device helper is not ready"));
        assert!(projected.entries.is_empty());

        // A folder with its own `.git` pointing nowhere is never a
        // repository, whatever contains the temporary directory.
        std::fs::write(folder.join(".git"), "gitdir: /nonexistent\n").unwrap();
        let (projected, _) = read(&request(&folder, &folder, None));
        assert!(projected.unavailable_reason.is_some());
        assert_eq!(projected.root_path.as_deref(), folder.to_str());
    }

    #[test]
    fn only_a_folder_git_finds_no_repository_for_is_not_a_repository() {
        let temporary = tempfile::tempdir().unwrap();
        let folder = temporary.path().canonicalize().unwrap();
        let mut unready = request(&folder, &folder, None);
        unready.channel = Err("The device helper is not ready".to_owned());
        assert!(!read(&unready).0.not_a_repository);

        // A `.git` pointing nowhere, whatever contains the temporary directory.
        std::fs::write(folder.join(".git"), "gitdir: /nonexistent\n").unwrap();
        assert!(read(&request(&folder, &folder, None)).0.not_a_repository);

        // Git finds this repository and then fails on its index: the
        // operator's to repair.
        std::fs::remove_file(folder.join(".git")).unwrap();
        git(&folder, &["init", "-q"]);
        std::fs::write(folder.join(".git/index"), "not an index").unwrap();
        let (broken, detail) = read(&request(&folder, &folder, None));
        assert_eq!(
            broken.unavailable_reason.as_deref(),
            Some("changes.git_failed"),
            "{broken:?}"
        );
        assert!(
            detail.is_some_and(|reason| reason.starts_with("git status failed")),
            "the log keeps Git's own words"
        );
        assert!(!broken.not_a_repository);
    }

    #[test]
    fn a_closed_view_publishes_an_empty_projection_without_a_key() {
        let mut reader = ChangesReader::new();
        // The first read starts the worker; once it has ended, the next read
        // hands its answer back.
        let answer = reader
            .read_if_due(None)
            .or_else(|| {
                reader.inner.join_pending();
                reader.read_if_due(None)
            })
            .expect("the reader answers a closed view");
        assert_eq!(answer.key, None);
        assert_eq!(answer.changes, ChangesSnapshot::default());
    }
}
