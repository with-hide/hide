//! Every worktree of every open repository, with the counts the sidebar row
//! and the summary card show.
//!
//! A worktree earns a row because git lists it, not because Herdr has a pane
//! in it: that is the whole point of the projects tree, and it is why this
//! reader exists rather than the catalog forking `git` itself. Each read runs
//! on a worker thread ([`crate::reader::BackgroundRead`]), so a repository
//! with many worktrees costs wall time on that thread and never coordinator
//! latency.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hide_node_link::cleanup::RepositoryDirs;
use hide_node_link::protocol::Call;
use hide_node_link::worktrees::{GIT_WATCH_LIMIT, GitWatchReport};

use crate::model::{
    ProjectWorktreesSnapshot, UnpushedSnapshot, WorktreeCatalogSnapshot,
    WorktreeDeletionGateSnapshot, WorktreeSnapshot,
};
use crate::reader::BackgroundRead;

/// What to describe: one entry per project the navigator is showing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorktreeRequest {
    pub projects: Vec<WorktreeProjectRequest>,
    /// Bumped when something invalidates every answer at once - a manual
    /// refresh, most of all. A changed request is always due, so this is how
    /// an unchanged project list still gets re-read.
    pub generation: u64,
    /// The local worktree removals the runtime had settled. It invalidates
    /// no answer - a removal changes its own repository's Git directory, which
    /// that project's freshness key already watches - and travels back with
    /// the answer, so the runtime can tell a read that started before a
    /// removal from one that started after it.
    pub removals: u64,
}

/// One read's catalog and the [`WorktreeRequest::removals`] it started under.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorktreeAnswer {
    pub catalog: WorktreeCatalogSnapshot,
    pub removals: u64,
    /// The input this answer actually read, to keep loading visible when a
    /// later Overview opening overtook an in-flight answer.
    pub request: WorktreeRequest,
    /// The Git facts observed by this read still match the reader's latest
    /// watch state. A ref write can overtake a read without changing the
    /// runtime's explicit request.
    pub observations_current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeProjectRequest {
    /// A path inside the repository; the reader resolves it to the main
    /// worktree itself.
    pub root_path: PathBuf,
    /// The base branch to measure a branch against, when the pull request
    /// lookup has already named one. A branch missing here falls back to the
    /// repository default branch.
    pub bases: BTreeMap<String, String>,
    pub base_override: Option<String>,
    /// The default branch GitHub names for the repository, which the node
    /// prefers over `origin/HEAD` (PRD git-freshness D-07).
    pub remote_default: Option<String>,
    /// An explicit read of this project, independent of other repositories.
    pub generation: u64,
}

/// One project's freshness key: its request and OS watch generation.
/// Each project carries its own key so a commit in one repository re-reads
/// that repository alone. The first version keyed the whole catalog on every
/// project's stamps together, and a checkpoint commit anywhere re-ran
/// `git status` in every registered repository, including one on iCloud
/// whose evicted files that status pulled back down for minutes.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ProjectObservation {
    request: WorktreeProjectRequest,
    git_generation: u64,
}

/// The generation, the settled removals and every project's observation.
type ObservedRequest = (u64, u64, Vec<ProjectObservation>);

/// One project's answer. `None` is a folder that is not a repository, which
/// contributes no project entry at all.
type ProjectAnswer = (PathBuf, Option<ProjectWorktreesSnapshot>);

const GIT_WATCH_DEBOUNCE: Duration = Duration::from_millis(300);
/// How long a Git watch that ended is left before it is started again.
const GIT_WATCH_RETRY: Duration = Duration::from_secs(30);
/// A Git watch runs until it is stopped; this only bounds a node that never
/// answers at all.
const GIT_WATCH_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
/// One repository lookup on the node while the watched set changes.
const REPOSITORY_TIMEOUT: Duration = Duration::from_secs(10);
/// The most reports a Git watch keeps for the coordinator to take. One past
/// it is dropped, and the drop is kept as one mark that every project is read
/// again (`git_watch.mailbox_full`), so a coordinator that falls behind costs
/// a read of each project, never a queue that grows with time.
const GIT_WATCH_MAILBOX: usize = 256;
/// How long the reader's end waits for its stopped watch threads, each of
/// which ends on the node's next report (a heartbeat comes every second).
const GIT_WATCH_STOP_WAIT: Duration = Duration::from_secs(3);

/// One Git watch on the node that holds the repositories, over every
/// project's common directory (`Call::GitWatch`). The coordinator drains its
/// reports on each wake, keeps only the projects whose Git facts changed,
/// and lets its existing background reader do the Git work once a project
/// has been quiet for [`GIT_WATCH_DEBOUNCE`]. A change of the watched set
/// starts a new watch and keeps the old one until the new one is watching,
/// so a repository in both is never unwatched. Any lapse in that cover (a
/// watch that ended or could not start, which is tried again after
/// [`GIT_WATCH_RETRY`]) is owed a read of every project once a watch is
/// watching again, because anything may have changed in between.
/// The Explorer watcher in hided polls only the selected checkout's visible
/// directories, so it cannot observe every registered project's Git refs
/// without idle polling and another daemon-to-core event path.
struct GitWatch {
    node: std::sync::Arc<dyn crate::node_access::NodeLink>,
    /// The forwarding threads of watches that were stopped, until each has
    /// ended: a stopped watch ends on its next report, not at once.
    stopping: Stopping,
    thread_ended: ThreadEnded,
    threads_ended: std::sync::mpsc::Receiver<()>,
    subscription: Option<GitSubscription>,
    /// The watch `subscription` replaced, until `subscription` is watching.
    outgoing: Option<GitSubscription>,
    /// When a watch that ended is started again.
    retry_at: Option<Instant>,
    /// Cover lapsed since the last watch began watching, so every project is
    /// read again once one does.
    owes_reread: bool,
    /// Each watched Git common directory and the projects that share it.
    roots: BTreeMap<PathBuf, Vec<PathBuf>>,
    /// Projects whose Git facts changed, with the last time they did.
    pending: BTreeMap<PathBuf, Instant>,
    registered: BTreeMap<PathBuf, PathBuf>,
    requested_roots: Vec<PathBuf>,
}

/// The forwarding threads of stopped watches, shared with each watch so the
/// one being dropped hands its thread over.
type Stopping = std::sync::Arc<std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>>;

/// Told once by each forwarding thread as it ends, so the reader's end
/// waits for its stopped watches without polling.
type ThreadEnded = std::sync::mpsc::Sender<()>;

/// What a running watch reported and the coordinator has not taken yet, at
/// most [`GIT_WATCH_MAILBOX`] reports.
#[derive(Default)]
struct Mailbox {
    reports: Vec<GitWatchReport>,
    /// A report was dropped because the mailbox was full.
    overflowed: bool,
    /// Why the watch ended, once it has; kept apart from the reports so a
    /// full mailbox never loses it.
    ended: Option<String>,
}

fn lock(mailbox: &std::sync::Mutex<Mailbox>) -> std::sync::MutexGuard<'_, Mailbox> {
    mailbox
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A running `Call::GitWatch` and its reports. Dropping it stops the watch,
/// which ends on the node within one report, and hands its forwarding thread
/// to [`GitWatch::stopping`], which joins it.
struct GitSubscription {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    mailbox: std::sync::Arc<std::sync::Mutex<Mailbox>>,
    /// The node has said it is watching; a change before that may be missed.
    watching: bool,
    thread: Option<std::thread::JoinHandle<()>>,
    stopping: Stopping,
}

impl GitSubscription {
    fn start(
        node: &std::sync::Arc<dyn crate::node_access::NodeLink>,
        common_dirs: Vec<String>,
        stopping: &Stopping,
        thread_ended: &ThreadEnded,
    ) -> Option<Self> {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mailbox = std::sync::Arc::new(std::sync::Mutex::new(Mailbox::default()));
        let node = std::sync::Arc::clone(node);
        let stopped = std::sync::Arc::clone(&stop);
        let forwarded = std::sync::Arc::clone(&mailbox);
        let thread_ended = thread_ended.clone();
        let started = std::thread::Builder::new()
            .name("hide-git-watch".to_owned())
            .spawn(move || {
                let ended = crate::node_access::call_as_with_progress::<(), GitWatchReport>(
                    node.as_ref(),
                    Call::GitWatch { common_dirs },
                    GIT_WATCH_TIMEOUT,
                    |report| {
                        if stopped.load(std::sync::atomic::Ordering::Relaxed) {
                            return false;
                        }
                        // The heartbeat only lets a stopped watch end; the
                        // coordinator has nothing to take from it.
                        if !matches!(report, GitWatchReport::Quiet) {
                            let mut mailbox = lock(&forwarded);
                            if mailbox.reports.len() < GIT_WATCH_MAILBOX {
                                mailbox.reports.push(report);
                            } else {
                                mailbox.overflowed = true;
                            }
                        }
                        true
                    },
                );
                lock(&forwarded).ended = Some(match ended {
                    Ok(()) => "the Git watch ended".to_owned(),
                    Err(error) => error.to_string(),
                });
                let _ = thread_ended.send(());
            });
        match started {
            Ok(thread) => Some(Self {
                stop,
                mailbox,
                watching: false,
                thread: Some(thread),
                stopping: std::sync::Arc::clone(stopping),
            }),
            Err(error) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "worktrees", "kind": "git_watch.start_failed", "message": error.to_string()
                }));
                None
            }
        }
    }
}

impl GitSubscription {
    /// Takes what the watch reported since the last call.
    fn take(&self) -> Mailbox {
        std::mem::take(&mut *lock(&self.mailbox))
    }
}

impl Drop for GitSubscription {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            self.stopping
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(thread);
        }
    }
}

impl GitWatch {
    fn new(node: std::sync::Arc<dyn crate::node_access::NodeLink>) -> Self {
        let (thread_ended, threads_ended) = std::sync::mpsc::channel();
        Self {
            node,
            stopping: Stopping::default(),
            thread_ended,
            threads_ended,
            subscription: None,
            outgoing: None,
            retry_at: None,
            owes_reread: false,
            roots: BTreeMap::new(),
            pending: BTreeMap::new(),
            registered: BTreeMap::new(),
            requested_roots: Vec::new(),
        }
    }

    fn reconcile(&mut self, projects: &[WorktreeProjectRequest]) {
        let requested_roots: Vec<_> = projects
            .iter()
            .map(|project| project.root_path.clone())
            .collect();
        let node = std::sync::Arc::clone(&self.node);
        self.reconcile_with(requested_roots, || {
            projects
                .iter()
                .take(GIT_WATCH_LIMIT)
                .filter_map(|project| {
                    crate::node_access::call_as::<Option<RepositoryDirs>>(
                        node.as_ref(),
                        Call::Repository {
                            path: project.root_path.to_string_lossy().into_owned(),
                        },
                        REPOSITORY_TIMEOUT,
                    )
                    .ok()
                    .flatten()
                    .map(|repository| {
                        (
                            project.root_path.clone(),
                            PathBuf::from(repository.common_dir),
                        )
                    })
                })
                .collect()
        });
    }

    /// Watches `projects`, each with the common directory its own read
    /// already named, so nothing is asked of the node to find them.
    fn reconcile_known(&mut self, projects: BTreeMap<PathBuf, PathBuf>) {
        let requested_roots: Vec<_> = projects.keys().cloned().collect();
        self.reconcile_with(requested_roots, || {
            projects.into_iter().take(GIT_WATCH_LIMIT).collect()
        });
    }

    /// Follows the requested projects; `desired` names each one's common
    /// directory, and is asked only when the requested set changed.
    fn reconcile_with(
        &mut self,
        requested_roots: Vec<PathBuf>,
        desired: impl FnOnce() -> BTreeMap<PathBuf, PathBuf>,
    ) {
        let restart_due = self.subscription.is_none()
            && !self.registered.is_empty()
            && self.retry_at.is_some_and(|at| at <= Instant::now());
        if requested_roots == self.requested_roots {
            if restart_due {
                let common: BTreeSet<_> = self.registered.values().cloned().collect();
                self.start(&common);
            }
            return;
        }
        let requested = requested_roots.len();
        self.requested_roots = requested_roots;
        let desired = desired();
        if requested > GIT_WATCH_LIMIT {
            crate::diagnostic!(serde_json::json!({
                "component": "worktrees", "kind": "git_watch.over_budget",
                "projects": requested, "cap": GIT_WATCH_LIMIT
            }));
        }
        let desired_common: BTreeSet<_> = desired.values().cloned().collect();
        let previous_common: BTreeSet<_> = self.registered.values().cloned().collect();
        if desired_common != previous_common {
            // What the running watch already reported, an end included,
            // belongs to the old set.
            self.collect();
            // A watching one has missed nothing, and stays until its
            // replacement is watching; one not yet watching covers nothing.
            if let Some(running) = self.subscription.take()
                && running.watching
            {
                self.outgoing = Some(running);
            }
            self.start(&desired_common);
        } else if restart_due {
            self.start(&desired_common);
        }
        let mut roots = BTreeMap::<PathBuf, Vec<PathBuf>>::new();
        for (project, common) in &desired {
            roots
                .entry(common.clone())
                .or_default()
                .push(project.clone());
        }
        self.roots = roots;
        self.registered = desired;
        self.pending
            .retain(|project, _| self.registered.contains_key(project));
    }

    /// Starts the watch over `common`, or schedules another try when the
    /// thread cannot start. No common directory is no watch.
    fn start(&mut self, common: &BTreeSet<PathBuf>) {
        self.subscription = None;
        self.retry_at = None;
        if common.is_empty() {
            self.outgoing = None;
            return;
        }
        self.subscription = GitSubscription::start(
            &self.node,
            common
                .iter()
                .map(|common| common.to_string_lossy().into_owned())
                .collect(),
            &self.stopping,
            &self.thread_ended,
        );
        if self.subscription.is_none() {
            self.owes_reread = true;
            self.retry_at = Some(Instant::now() + GIT_WATCH_RETRY);
        }
    }

    /// Moves what the watch reported into `pending`: a changed Git fact
    /// marks the projects of its repository, and lost reports (an overflow,
    /// or a watch restarted after a gap) mark every watched project, because
    /// any of them may have changed. A watch that ended is started again
    /// after [`GIT_WATCH_RETRY`].
    ///
    /// The replacement is drained before the watch it replaces, so a change
    /// the old watch reports between the two drains is still read, and the
    /// old watch goes only after a pass in which its replacement was
    /// already watching, so a change it reports just after the replacement
    /// starts is read too.
    fn collect(&mut self) {
        let now = Instant::now();
        let mut changed = Vec::new();
        let mut every = false;
        let mut ended = None;
        let covered = self
            .subscription
            .as_ref()
            .is_some_and(|subscription| subscription.watching);
        if let Some(subscription) = self.subscription.as_mut() {
            let mailbox = subscription.take();
            for report in mailbox.reports {
                match report {
                    GitWatchReport::Watching { unwatched } => {
                        subscription.watching = true;
                        for (common, message) in unwatched {
                            crate::diagnostic!(serde_json::json!({
                                "component": "worktrees", "kind": "git_watch.project_failed",
                                "common_dir": common, "message": message
                            }));
                        }
                    }
                    GitWatchReport::Changed { common_dirs } => changed.extend(common_dirs),
                    GitWatchReport::Overflow { reason } => {
                        crate::diagnostic!(serde_json::json!({
                            "component": "worktrees", "kind": "git_watch.overflow",
                            "message": reason, "projects": self.registered.len()
                        }));
                        every = true;
                    }
                    GitWatchReport::Quiet => {}
                }
            }
            if mailbox.overflowed {
                crate::diagnostic!(serde_json::json!({
                    "component": "worktrees", "kind": "git_watch.mailbox_full",
                    "cap": GIT_WATCH_MAILBOX, "projects": self.registered.len()
                }));
                every = true;
            }
            ended = mailbox.ended;
        }
        if let Some(outgoing) = self.outgoing.as_mut() {
            let mailbox = outgoing.take();
            for report in mailbox.reports {
                match report {
                    GitWatchReport::Changed { common_dirs } => changed.extend(common_dirs),
                    // The old watch lost changes while it still stood as
                    // cover: its replacement owes every project a read.
                    GitWatchReport::Overflow { reason } => {
                        crate::diagnostic!(serde_json::json!({
                            "component": "worktrees", "kind": "git_watch.overflow",
                            "message": reason, "replaced": true
                        }));
                        self.owes_reread = true;
                    }
                    _ => {}
                }
            }
            if mailbox.overflowed {
                crate::diagnostic!(serde_json::json!({
                    "component": "worktrees", "kind": "git_watch.mailbox_full",
                    "cap": GIT_WATCH_MAILBOX, "replaced": true
                }));
                self.owes_reread = true;
            }
            if mailbox.ended.is_some() {
                crate::diagnostic!(serde_json::json!({
                    "component": "worktrees", "kind": "git_watch.ended", "replaced": true
                }));
                self.outgoing = None;
                self.owes_reread = true;
            }
        }
        if self
            .subscription
            .as_ref()
            .is_some_and(|subscription| subscription.watching)
        {
            every |= std::mem::take(&mut self.owes_reread);
        }
        if covered {
            self.outgoing = None;
        }
        self.mark(&changed, now);
        if every {
            for project in self.registered.keys() {
                self.pending.insert(project.clone(), now);
            }
        }
        if let Some(reason) = ended {
            crate::diagnostic!(serde_json::json!({
                "component": "worktrees", "kind": "git_watch.ended", "message": reason
            }));
            self.subscription = None;
            self.owes_reread = true;
            self.retry_at = Some(now + GIT_WATCH_RETRY);
        }
        self.reap();
    }

    /// Ends the watch while no renderer is attached: nothing takes its
    /// reports then, and a watch left running would keep the node watching
    /// for as long as the window stays closed. The next [`Self::reconcile`]
    /// starts it again, and once it is watching every project is read, as
    /// after any lapse in cover.
    fn rest(&mut self) {
        if self.subscription.is_none() && self.outgoing.is_none() {
            return;
        }
        self.subscription = None;
        self.outgoing = None;
        self.owes_reread = true;
        self.retry_at = Some(Instant::now());
        crate::diagnostic!(serde_json::json!({
            "component": "worktrees", "kind": "git_watch.rested",
            "projects": self.registered.len()
        }));
        self.reap();
    }

    /// Joins the stopped watches' threads that have ended.
    fn reap(&mut self) {
        let mut stopping = self
            .stopping
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (ended, running): (Vec<_>, Vec<_>) = stopping
            .drain(..)
            .partition(std::thread::JoinHandle::is_finished);
        *stopping = running;
        drop(stopping);
        for thread in ended {
            if thread.join().is_err() {
                crate::diagnostic!(serde_json::json!({
                    "component": "worktrees", "kind": "git_watch.thread_panicked"
                }));
            }
        }
    }

    /// Marks the projects of each changed common directory.
    fn mark(&mut self, common_dirs: &[String], now: Instant) {
        for common in common_dirs {
            for project in self.roots.get(Path::new(common)).into_iter().flatten() {
                self.pending.insert(project.clone(), now);
            }
        }
    }

    fn settled(&mut self) -> Vec<PathBuf> {
        self.collect();
        let now = Instant::now();
        let settled: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, last)| now.duration_since(**last) >= GIT_WATCH_DEBOUNCE)
            .map(|(project, _)| project.clone())
            .collect();
        for project in &settled {
            self.pending.remove(project);
        }
        settled
    }

    fn has_pending(&mut self) -> bool {
        self.collect();
        !self.pending.is_empty()
    }
}

impl Drop for GitWatch {
    /// Stops every watch and waits a bounded time for their threads, each of
    /// which ends on the node's next report; one still running then (a node
    /// that sends nothing) is named in the log and left to its call's end.
    fn drop(&mut self) {
        self.subscription = None;
        self.outgoing = None;
        let deadline = Instant::now() + GIT_WATCH_STOP_WAIT;
        loop {
            self.reap();
            let running = self
                .stopping
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len();
            if running == 0 {
                return;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || self.threads_ended.recv_timeout(left).is_err() {
                crate::diagnostic!(serde_json::json!({
                    "component": "worktrees", "kind": "git_watch.stop_unconfirmed",
                    "threads": running
                }));
                return;
            }
        }
    }
}

/// The Git watch on a connected device (PRD git-freshness D-08): the same
/// `Call::GitWatch` over that device's link, following the repositories its
/// worktree read found there, so a branch, worktree or fetched ref that moves
/// on the device is read again by the device catalog without a reconnect.
/// `crate::fetch::FetchPump` drives it off every lock; nothing here blocks.
/// A device, its link, and the repositories its watch follows: each root
/// with the common directory the device named for it.
pub(crate) type DeviceWatchSet = (
    String,
    std::sync::Arc<dyn crate::node_access::NodeLink>,
    BTreeMap<PathBuf, PathBuf>,
);

pub(crate) struct DeviceGitWatch {
    link: std::sync::Arc<dyn crate::node_access::NodeLink>,
    watch: GitWatch,
}

impl DeviceGitWatch {
    pub(crate) fn new(link: std::sync::Arc<dyn crate::node_access::NodeLink>) -> Self {
        Self {
            watch: GitWatch::new(std::sync::Arc::clone(&link)),
            link,
        }
    }

    /// Whether this watch runs over `link`; a new connection needs a new one.
    pub(crate) fn watches_over(
        &self,
        link: &std::sync::Arc<dyn crate::node_access::NodeLink>,
    ) -> bool {
        std::sync::Arc::ptr_eq(&self.link, link)
    }

    /// Follows `projects` (each root with the common directory the device
    /// named for it) and answers the roots whose Git facts changed and have
    /// been quiet for the debounce; every root after a lapse in cover.
    pub(crate) fn changed(&mut self, projects: BTreeMap<PathBuf, PathBuf>) -> Vec<PathBuf> {
        self.watch.reconcile_known(projects);
        self.watch.settled()
    }

    /// Ends the watch while no renderer is attached (`GitWatch::rest`).
    pub(crate) fn rest(&mut self) {
        self.watch.rest();
    }
}

#[cfg(test)]
impl DeviceGitWatch {
    /// The device has said it is watching, so no later change is missed.
    pub(crate) fn watching(&self) -> bool {
        self.watch
            .subscription
            .as_ref()
            .is_some_and(|subscription| subscription.watching)
    }
}

pub struct WorktreeReader {
    inner: BackgroundRead<ObservedRequest, (ObservedRequest, Vec<ProjectAnswer>)>,
    git_watch: GitWatch,
    git_generations: BTreeMap<PathBuf, u64>,
}

impl WorktreeReader {
    /// Reads the repositories of the node `node` links to.
    pub fn new(node: std::sync::Arc<dyn crate::node_access::NodeLink>) -> Self {
        let git_watch = GitWatch::new(std::sync::Arc::clone(&node));
        Self {
            inner: BackgroundRead::on_change(Duration::ZERO, {
                let cache = std::sync::Mutex::new(ProjectCache::new());
                move |request: &ObservedRequest| read_observed(node.as_ref(), &cache, request)
            }),
            git_watch,
            git_generations: BTreeMap::new(),
        }
    }

    /// Ends the Git watch while no renderer is attached; the next
    /// [`Self::read_if_due`] starts it again (`GitWatch::rest`).
    pub fn rest(&mut self) {
        self.git_watch.rest();
    }

    pub fn read_if_due(&mut self, request: WorktreeRequest) -> Option<WorktreeAnswer> {
        self.git_watch.reconcile(&request.projects);
        for project in self.git_watch.settled() {
            let generation = self.git_generations.entry(project).or_insert(0);
            *generation = generation.wrapping_add(1);
        }
        let observations = request
            .projects
            .iter()
            .map(|project| ProjectObservation {
                request: project.clone(),
                git_generation: self
                    .git_generations
                    .get(&project.root_path)
                    .copied()
                    .unwrap_or(0),
            })
            .collect();
        let observed_request = (request.generation, request.removals, observations);
        let (answered_request, answers) = self.inner.poll(observed_request.clone())?;
        let observations_current =
            answered_request == observed_request && !self.git_watch.has_pending();
        let mut projects: Vec<ProjectWorktreesSnapshot> = Vec::new();
        for (_, project) in answers {
            // Two registrations inside one repository describe one project.
            if let Some(project) = project
                && !projects
                    .iter()
                    .any(|existing| existing.root_path == project.root_path)
            {
                projects.push(project);
            }
        }
        self.git_generations
            .retain(|root, _| request.projects.iter().any(|p| &p.root_path == root));
        Some(WorktreeAnswer {
            catalog: WorktreeCatalogSnapshot { projects },
            removals: answered_request.1,
            observations_current,
            request: WorktreeRequest {
                projects: answered_request
                    .2
                    .into_iter()
                    .map(|observation| observation.request)
                    .collect(),
                generation: answered_request.0,
                removals: answered_request.1,
            },
        })
    }
}

/// The last answer per requested root, with the observation that produced it.
type ProjectCache = BTreeMap<PathBuf, (u64, ProjectObservation, Option<ProjectWorktreesSnapshot>)>;

/// The worker's read: every project whose observation moved is read again,
/// every other one is answered from the last read. The cache lives with the
/// worker because only the worker produces what it holds; a project that
/// leaves the request leaves the cache with it.
fn read_observed(
    node: &dyn crate::node_access::NodeLink,
    cache: &std::sync::Mutex<ProjectCache>,
    request: &ObservedRequest,
) -> (ObservedRequest, Vec<ProjectAnswer>) {
    let (generation, removals, observations) = request;
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.retain(|root, _| observations.iter().any(|o| &o.request.root_path == root));
    let answers = observations
        .iter()
        .map(|observation| {
            let root = observation.request.root_path.clone();
            let cached = cache.get(&root);
            if let Some((cached_generation, cached, project)) = cached
                && cached_generation == generation
                && cached == observation
            {
                return (root, project.clone());
            }
            // Only an explicit read (a refresh, an Overview opening) asks
            // every worktree again; a Git change or a new base is read from
            // the worktrees whose inputs moved. So is a project's first read
            // when nothing has asked an explicit one since the core started
            // (both generations start at 0): its node kept what it measured
            // across the restart, and reuses only what did not move.
            let reuse_unchanged = match cached {
                Some((cached_generation, cached, _)) => {
                    cached_generation == generation
                        && cached.request.generation == observation.request.generation
                }
                None => *generation == 0 && observation.request.generation == 0,
            };
            let project = read(node, &observation.request, reuse_unchanged);
            cache.insert(
                root.clone(),
                (*generation, observation.clone(), project.clone()),
            );
            (root, project)
        })
        .collect();
    ((*generation, *removals, observations.clone()), answers)
}

/// Whether a worktree's work is merged: the one judgment the sidebar's
/// landed mark and the disk cleanup review share (PRD git-freshness D-06).
///
/// `in_base` is Git's answer: HEAD is in the repository's default branch,
/// locally or as last fetched. `merged_heads` is GitHub's: the exact head
/// commits of the pull requests it merged into that branch, which is the
/// only proof a squash or rebase merge leaves, and `None` when GitHub has
/// never answered for the repository. A head that moved past its merged pull
/// request is not that pull request's head, and an absent GitHub answer is
/// never read as merged (D-12).
pub(crate) fn merged(
    in_base: Option<bool>,
    head: Option<&str>,
    merged_heads: Option<&BTreeSet<String>>,
) -> Option<bool> {
    if in_base == Some(true) {
        return Some(true);
    }
    if head
        .zip(merged_heads)
        .is_some_and(|(head, heads)| heads.contains(head))
    {
        return Some(true);
    }
    in_base
}

/// The exact head commits of the pull requests GitHub merged into `base`,
/// from the last pull request list GitHub answered for the project, kept
/// while a later read fails (D-12); `None` when it never answered.
pub(crate) fn merged_heads(
    github: Option<&crate::model::GithubProjectSnapshot>,
    base: Option<&str>,
) -> Option<BTreeSet<String>> {
    let github = github.filter(|github| {
        github.pull_requests_read || github.status.last_success_at_unix_ms.is_some()
    })?;
    let base = base?;
    Some(
        github
            .pull_requests
            .iter()
            .filter(|pull_request| {
                pull_request.badge == crate::model::PullRequestBadge::Merged
                    && pull_request.base_branch == base
            })
            .filter_map(|pull_request| pull_request.head_oid.clone())
            .filter(|head| hide_node_link::gh::is_commit(head))
            .collect(),
    )
}

/// The sole deletion policy, consumed by all three presentation surfaces.
///
/// Main, locked or unmeasured worktrees are refused; other loss is a
/// warning the confirmation shows, and the loss itself is a choice the
/// operator makes there (`discard_label`, `can_delete_branch`).
pub fn deletion_gate(
    worktree: &WorktreeSnapshot,
    is_base: bool,
    pane_count: usize,
) -> WorktreeDeletionGateSnapshot {
    let blocked_reason = if worktree.is_main {
        Some("The main worktree cannot be deleted".to_owned())
    } else if let Some(reason) = worktree.lock_reason.as_deref() {
        let name = worktree
            .branch
            .as_deref()
            .or_else(|| {
                Path::new(&worktree.path)
                    .file_name()
                    .and_then(|name| name.to_str())
            })
            .unwrap_or("selected checkout");
        Some(hide_node_link::worktrees::locked_removal_reason(
            name, reason,
        ))
    } else {
        worktree.ignored_scan_unavailable.clone()
    };
    let status_unknown = worktree.unavailable_reason.is_some() && !worktree.missing;
    let files = |count: u32| match count {
        0 => "uncommitted changes".to_owned(),
        1 => "1 changed file".to_owned(),
        count => format!("{count} changed files"),
    };
    let mut warnings = Vec::new();
    if worktree.dirty {
        warnings.push(format!(
            "{} not committed",
            files(worktree.changed_file_count)
        ));
    }
    if worktree.nested {
        warnings.push("another worktree lies inside it".to_owned());
    }
    if is_base {
        warnings.push("holds the base branch".to_owned());
    }
    if status_unknown {
        warnings.push("Git status unavailable".to_owned());
    }
    if !worktree.missing {
        if worktree.merged != Some(true) {
            warnings.push(format!("ahead {} unmerged", worktree.ahead));
        }
        if worktree.upstream_state != "pushed" {
            warnings.push("not pushed".to_owned());
        }
    }
    let mut discard_label = match (worktree.dirty, worktree.nested) {
        (true, true) => Some(format!(
            "Discard {} and the worktree inside it",
            files(worktree.changed_file_count)
        )),
        (true, false) => Some(format!("Discard {}", files(worktree.changed_file_count))),
        (false, true) => Some("Discard the worktree inside it".to_owned()),
        (false, false) => status_unknown.then(|| "Discard any uncommitted changes".to_owned()),
    };
    if !worktree.ignored_repositories.is_empty() {
        let names = worktree.ignored_repositories.join(", ");
        warnings.push(format!(
            "{} ignored repositories",
            worktree.ignored_repositories.len()
        ));
        discard_label = Some(match discard_label {
            Some(label) => format!("{label} and ignored repositories {names}"),
            None => format!("Discard ignored repositories {names}"),
        });
    }
    let can_delete_branch = !worktree.missing && worktree.branch.is_some() && !is_base;
    let branch_warning = match worktree.merged {
        _ if !can_delete_branch => None,
        Some(true) => None,
        Some(false) if worktree.ahead == 0 => {
            Some("Git does not count it as merged; it is deleted anyway".to_owned())
        }
        Some(false) => Some(format!(
            "{} not on {} {} lost with it",
            if worktree.ahead == 1 {
                "1 commit".to_owned()
            } else {
                format!("{} commits", worktree.ahead)
            },
            worktree.base_branch.as_deref().unwrap_or("the base"),
            if worktree.ahead == 1 { "is" } else { "are" },
        )),
        None => Some("Git could not tell whether it is merged; it is deleted anyway".to_owned()),
    };
    WorktreeDeletionGateSnapshot {
        blocked_reason,
        warnings,
        button_label: if pane_count == 1 {
            "Close 1 pane and delete".to_owned()
        } else if pane_count > 0 {
            format!("Close {pane_count} panes and delete")
        } else {
            "Delete worktree…".to_owned()
        },
        can_delete_branch,
        branch_warning,
        discard_label,
    }
}

/// One project's rows: the node's Git facts with this machine's policy and
/// decoration slots on them, asked the way a device's repository is
/// (`Call::Worktrees`). A refusal is the project's unavailable reason.
fn read(
    node: &dyn crate::node_access::NodeLink,
    project: &WorktreeProjectRequest,
    reuse_unchanged: bool,
) -> Option<ProjectWorktreesSnapshot> {
    let unavailable = |reason: String| {
        Some(ProjectWorktreesSnapshot {
            root_path: project.root_path.to_string_lossy().into_owned(),
            unavailable_reason: Some(reason),
            ..Default::default()
        })
    };
    let Some(path) = project.root_path.to_str() else {
        return unavailable(format!(
            "Repository path is not valid UTF-8: {}",
            project.root_path.display()
        ));
    };
    match crate::node_access::call_as::<Option<hide_node_link::worktrees::RepositoryWorktrees>>(
        node,
        hide_node_link::protocol::Call::Worktrees {
            path: path.to_owned(),
            bases: project.bases.clone(),
            base_override: project.base_override.clone(),
            remote_default: project.remote_default.clone(),
            reuse_unchanged,
        },
        WORKTREES_TIMEOUT,
    ) {
        Ok(facts) => facts.map(project_snapshot),
        Err(crate::node_access::LinkError::Refused(error)) => unavailable(error.message),
        Err(error) => unavailable(error.to_string()),
    }
}

use hide_node_link::worktrees::WORKTREES_TIMEOUT;

#[cfg(test)]
fn read_project(
    root: &Path,
    root_path: String,
    bases: &BTreeMap<String, String>,
    base_override: Option<&str>,
) -> ProjectWorktreesSnapshot {
    project_snapshot(hide_host::worktrees::read_project(
        root,
        root_path,
        bases,
        (base_override, None),
        &hide_host::worktrees::Measured::default(),
        false,
    ))
}

/// A repository's host facts as the snapshot's project entry, each row
/// carrying the deletion gate its facts decide. The gate's pane and agent
/// counts are filled when the catalog places the row.
pub fn project_snapshot(
    facts: hide_node_link::worktrees::RepositoryWorktrees,
) -> ProjectWorktreesSnapshot {
    if let Some(reason) = facts.unavailable_reason.as_deref() {
        crate::diagnostic!(serde_json::json!({
            "component": "worktrees",
            "kind": "worktree_list.failed",
            "project": facts.root_path,
            "message": reason,
        }));
    }
    let base_branch = facts.base_branch.clone();
    ProjectWorktreesSnapshot {
        repository: facts
            .origin_url
            .as_deref()
            .and_then(hide_node_link::gh::repository_of_remote),
        shared_git_path: facts.shared_git_path,
        root_path: facts.root_path,
        default_branch: facts.default_branch,
        branches: facts.branches,
        worktrees: facts
            .worktrees
            .into_iter()
            .map(|row| {
                let mut worktree = worktree_snapshot(row);
                worktree.deletion_gate = deletion_gate(
                    &worktree,
                    worktree.branch == base_branch && base_branch.is_some(),
                    0,
                );
                worktree
            })
            .collect(),
        unavailable_reason: facts.unavailable_reason,
        base_branch: facts.base_branch,
        base_branch_fallback: facts.base_branch_fallback,
        base_source: facts.base_source,
        ..Default::default()
    }
}

fn worktree_snapshot(facts: hide_node_link::worktrees::WorktreeFacts) -> WorktreeSnapshot {
    if let Some(reason) = facts.unavailable_reason.as_deref() {
        crate::diagnostic!(serde_json::json!({
            "component": "worktrees",
            "kind": "worktree_status.unavailable",
            "path": facts.path,
            "message": reason,
        }));
    }
    WorktreeSnapshot {
        path: facts.path,
        branch: facts.branch,
        head_sha: facts.head_sha,
        missing: facts.missing,
        is_main: facts.is_main,
        lock_reason: facts.lock_reason,
        ignored_repositories: facts.ignored_repositories,
        ignored_scan_unavailable: facts.ignored_scan_unavailable,
        nested: facts.nested,
        dirty: facts.dirty,
        changed_file_count: facts.changed_file_count,
        base_branch: facts.base_branch,
        ahead: facts.ahead,
        behind: facts.behind,
        added_lines: facts.added_lines,
        removed_lines: facts.removed_lines,
        in_base: facts.merged,
        merged: facts.merged,
        upstream_state: facts.upstream_state,
        unpushed: facts.unpushed.map(|unpushed| UnpushedSnapshot {
            remote: unpushed.remote,
            count: unpushed.count,
        }),
        behind_upstream: facts.behind_upstream,
        created_at_unix_ms: facts.created_at_unix_ms,
        last_commit_unix_seconds: facts.last_commit_unix_seconds,
        last_commit_subject: facts.last_commit_subject,
        last_fetch_at_unix_ms: facts.last_fetch_at_unix_ms,
        measured_at_unix_ms: facts.measured_at_unix_ms,
        unavailable_reason: facts.unavailable_reason,
        ..WorktreeSnapshot::default()
    }
}

#[cfg(test)]
fn git_call_count(root: &Path, command: &str) -> usize {
    hide_host::worktrees::git_calls(root, command)
}

#[cfg(test)]
use hide_host::worktrees::output_within;

#[cfg(test)]
mod tests {
    use super::*;
    use hide_host::worktrees::{
        parse_ahead_behind, parse_shortstat, parse_worktree_list, resolve_base,
    };

    #[test]
    fn merged_is_git_ancestry_or_a_merged_pull_request_at_exactly_this_head() {
        let head = "a".repeat(40);
        let heads = BTreeSet::from([head.clone()]);
        let other = BTreeSet::from(["b".repeat(40)]);
        let empty = BTreeSet::new();
        // (Git's answer, GitHub's heads) -> merged
        for (in_base, github, expected) in [
            (Some(true), None, Some(true)),
            (Some(true), Some(&other), Some(true)),
            // A squash or rebase merge: GitHub's exact head is the proof.
            (Some(false), Some(&heads), Some(true)),
            (None, Some(&heads), Some(true)),
            // A HEAD past its merged pull request is not that pull request.
            (Some(false), Some(&other), Some(false)),
            (Some(false), Some(&empty), Some(false)),
            // No GitHub answer is never read as merged (D-12).
            (Some(false), None, Some(false)),
            (None, None, None),
            (None, Some(&other), None),
        ] {
            assert_eq!(
                merged(in_base, Some(&head), github),
                expected,
                "{in_base:?} {github:?}"
            );
        }
        assert_eq!(merged(Some(false), None, Some(&heads)), Some(false));
    }

    #[test]
    fn merged_heads_are_the_merged_pull_requests_into_the_default_branch() {
        use crate::model::{
            GithubProjectSnapshot, PullRequestBadge, PullRequestChecks, PullRequestSnapshot,
        };
        let pull = |head: char, base: &str, badge| PullRequestSnapshot {
            closing_issues: Vec::new(),
            title: String::new(),
            checks: PullRequestChecks::Unknown,
            number: 1,
            head_branch: "feature".into(),
            base_branch: base.into(),
            url: String::new(),
            badge,
            review: None,
            is_draft: false,
            merged_at_unix_ms: None,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            closed_at_unix_ms: None,
            head_oid: Some(head.to_string().repeat(40)),
            cross_repository: false,
            work: Default::default(),
        };
        let mut github = GithubProjectSnapshot {
            pull_requests: vec![
                pull('a', "develop", PullRequestBadge::Merged),
                pull('b', "main", PullRequestBadge::Merged),
                pull('c', "develop", PullRequestBadge::Open),
                pull('d', "develop", PullRequestBadge::Closed),
            ],
            ..Default::default()
        };
        // GitHub never answered: no heads at all, not an empty set.
        assert_eq!(merged_heads(Some(&github), Some("develop")), None);
        github.pull_requests_read = true;
        assert_eq!(
            merged_heads(Some(&github), Some("develop")),
            Some(BTreeSet::from(["a".repeat(40)]))
        );
        assert_eq!(merged_heads(Some(&github), None), None);
        // A failed later read keeps the last answer it was restored with.
        github.pull_requests_read = false;
        github.status.last_success_at_unix_ms = Some(1);
        assert_eq!(
            merged_heads(Some(&github), Some("main")),
            Some(BTreeSet::from(["b".repeat(40)]))
        );
    }

    #[test]
    fn porcelain_records_name_the_main_worktree_and_each_branch() {
        let output = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\n\
                      worktree /repo.worktrees/feature\nHEAD def\nbranch refs/heads/feature\n\n";
        let listed = parse_worktree_list(output);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].path, PathBuf::from("/repo"));
        assert_eq!(listed[0].branch.as_deref(), Some("main"));
        assert!(listed[0].is_main);
        assert_eq!(listed[1].branch.as_deref(), Some("feature"));
        assert!(!listed[1].is_main);
    }

    /// A detached head has no `branch` line. The record must still produce a
    /// worktree, because the folder is on disk and the row is about the
    /// folder.
    #[test]
    fn a_detached_worktree_is_listed_without_a_branch() {
        let output = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\n\
                      worktree /repo.worktrees/detached\nHEAD def\ndetached\n\n";
        let listed = parse_worktree_list(output);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].branch, None);
        assert_eq!(listed[1].path, PathBuf::from("/repo.worktrees/detached"));
    }

    /// The branch line belongs to the record it follows. Reading it into the
    /// next record would label every worktree with its predecessor's branch.
    #[test]
    fn a_branchless_record_does_not_inherit_the_previous_branch() {
        let output = "worktree /repo\nbranch refs/heads/main\n\n\
                      worktree /a\ndetached\n\n\
                      worktree /b\nbranch refs/heads/second\n\n";
        let listed = parse_worktree_list(output);
        assert_eq!(
            listed
                .iter()
                .map(|entry| entry.branch.as_deref())
                .collect::<Vec<_>>(),
            [Some("main"), None, Some("second")]
        );
    }

    #[test]
    fn left_right_counts_read_the_base_side_as_behind() {
        assert_eq!(parse_ahead_behind("0\t2\n"), (2, 0));
        assert_eq!(parse_ahead_behind("3\t1\n"), (1, 3));
        assert_eq!(parse_ahead_behind(""), (0, 0));
    }

    #[test]
    fn shortstat_reads_each_half_independently() {
        assert_eq!(
            parse_shortstat(" 3 files changed, 42 insertions(+), 7 deletions(-)\n"),
            (42, 7)
        );
        assert_eq!(
            parse_shortstat(" 1 file changed, 5 insertions(+)\n"),
            (5, 0)
        );
        assert_eq!(parse_shortstat(" 1 file changed, 9 deletions(-)\n"), (0, 9));
        assert_eq!(parse_shortstat(""), (0, 0));
    }

    /// A pull request's base wins over the repository default, a branch with
    /// no pull request falls back to the default, and the default branch's own
    /// worktree is compared with nothing.
    #[test]
    fn the_pull_request_base_overrides_the_repository_default() {
        let bases = BTreeMap::from([("feature".to_owned(), "release".to_owned())]);
        assert_eq!(
            resolve_base(Some("feature"), &bases, Some("main")).as_deref(),
            Some("release")
        );
        assert_eq!(
            resolve_base(Some("other"), &bases, Some("main")).as_deref(),
            Some("main")
        );
        assert_eq!(resolve_base(Some("main"), &bases, Some("main")), None);
        assert_eq!(resolve_base(Some("other"), &bases, None), None);
        assert_eq!(
            resolve_base(None, &bases, Some("main")).as_deref(),
            Some("main")
        );
    }

    /// A node whose projects are each their own repository, and whose every
    /// `Call::GitWatch` forwards the reports the test sends on the channel it
    /// prepared for that set of common directories, acknowledging each, until
    /// the test drops the sender. Watch threads start in any order, so a
    /// channel is found by what it watches, not by when it was asked for.
    type Prepared = Vec<(Vec<String>, std::sync::mpsc::Receiver<GitWatchReport>)>;

    struct DrivenWatches {
        watches: std::sync::Arc<std::sync::Mutex<Prepared>>,
        forwarded: std::sync::Mutex<std::sync::mpsc::Sender<()>>,
    }

    impl crate::node_access::NodeLink for DrivenWatches {
        fn call(
            &self,
            call: Call,
            timeout: Duration,
        ) -> Result<hide_node_link::link::LinkAnswer, hide_node_link::link::LinkError> {
            self.call_with_progress(call, timeout, &mut |_| true)
        }

        fn call_with_progress(
            &self,
            call: Call,
            _timeout: Duration,
            progress: &mut dyn FnMut(serde_json::Value) -> bool,
        ) -> Result<hide_node_link::link::LinkAnswer, hide_node_link::link::LinkError> {
            match call {
                Call::Repository { path } => Ok(serde_json::json!(RepositoryDirs {
                    root: path.clone(),
                    git_dir: format!("{path}/.git"),
                    common_dir: format!("{path}/.git"),
                })
                .into()),
                Call::GitWatch { common_dirs } => {
                    let reports = {
                        let mut prepared = self.watches.lock().unwrap();
                        let at = prepared
                            .iter()
                            .position(|(watched, _)| *watched == common_dirs)
                            .unwrap_or_else(|| panic!("no watch prepared for {common_dirs:?}"));
                        prepared.remove(at).1
                    };
                    let forwarded = self.forwarded.lock().unwrap().clone();
                    while let Ok(report) = reports.recv() {
                        let go_on = progress(serde_json::to_value(report).unwrap());
                        forwarded.send(()).unwrap();
                        if !go_on {
                            break;
                        }
                    }
                    Ok(serde_json::Value::Null.into())
                }
                other => panic!("unexpected call {other:?}"),
            }
        }
    }

    struct Driver {
        watch: GitWatch,
        watches: std::sync::Arc<std::sync::Mutex<Prepared>>,
        forwarded: std::sync::mpsc::Receiver<()>,
    }

    impl Driver {
        fn new() -> Self {
            let watches = std::sync::Arc::default();
            let (acknowledge, forwarded) = std::sync::mpsc::channel();
            let node = DrivenWatches {
                watches: std::sync::Arc::clone(&watches),
                forwarded: std::sync::Mutex::new(acknowledge),
            };
            Self {
                watch: GitWatch::new(std::sync::Arc::new(node)),
                watches,
                forwarded,
            }
        }

        /// The sender of the next watch the node starts over the projects
        /// at `roots`.
        fn next_watch(&self, roots: &[&str]) -> std::sync::mpsc::Sender<GitWatchReport> {
            let (sender, reports) = std::sync::mpsc::channel();
            let watched = roots.iter().map(|root| format!("{root}/.git")).collect();
            self.watches.lock().unwrap().push((watched, reports));
            sender
        }

        /// Sends `report` on `on` and collects it once the node forwarded it.
        fn report(&mut self, on: &std::sync::mpsc::Sender<GitWatchReport>, report: GitWatchReport) {
            on.send(report).unwrap();
            self.forwarded
                .recv_timeout(Duration::from_secs(5))
                .expect("the node forwarded the report");
            self.watch.collect();
        }
    }

    fn projects(roots: &[&str]) -> Vec<WorktreeProjectRequest> {
        roots
            .iter()
            .map(|root| WorktreeProjectRequest {
                root_path: PathBuf::from(root),
                bases: BTreeMap::new(),
                base_override: None,
                remote_default: None,
                generation: 0,
            })
            .collect()
    }

    /// A project's first read since the core started reuses what its node
    /// kept across the restart; once a refresh or an Overview opening asked
    /// an explicit read, a first read measures every worktree.
    #[test]
    fn a_first_read_since_start_reuses_and_one_after_an_explicit_ask_does_not() {
        struct Recording(std::sync::Mutex<Vec<bool>>);
        impl crate::node_access::NodeLink for Recording {
            fn call(
                &self,
                call: Call,
                _timeout: Duration,
            ) -> Result<hide_node_link::link::LinkAnswer, hide_node_link::link::LinkError>
            {
                if let Call::Worktrees {
                    reuse_unchanged, ..
                } = call
                {
                    self.0.lock().unwrap().push(reuse_unchanged);
                }
                Ok(serde_json::Value::Null.into())
            }
        }
        let asked = |generation: u64, project_generation: u64| {
            let node = Recording(std::sync::Mutex::default());
            let mut request = projects(&["/a"]).remove(0);
            request.generation = project_generation;
            let observation = ProjectObservation {
                request,
                git_generation: 0,
            };
            read_observed(
                &node,
                &std::sync::Mutex::default(),
                &(generation, 0, vec![observation]),
            );
            node.0.into_inner().unwrap()
        };
        assert_eq!(asked(0, 0), [true], "nothing explicit was asked");
        assert_eq!(asked(1, 0), [false], "a refresh");
        assert_eq!(asked(0, 1), [false], "an Overview opening");
    }

    fn watching() -> GitWatchReport {
        GitWatchReport::Watching {
            unwatched: Vec::new(),
        }
    }

    /// A watch that ended may have missed any change, so once its restart is
    /// watching every project is read again; the first watch reads nothing.
    #[test]
    fn a_git_watch_restarted_after_it_ended_rereads_every_project() {
        let mut driver = Driver::new();
        let first = driver.next_watch(&["/a", "/b"]);
        let both = projects(&["/a", "/b"]);
        driver.watch.reconcile(&both);
        driver.report(&first, watching());
        assert!(driver.watch.pending.is_empty());
        drop(first);
        let deadline = Instant::now() + Duration::from_secs(5);
        while driver.watch.subscription.is_some() {
            assert!(
                Instant::now() < deadline,
                "the ended watch was never collected"
            );
            std::thread::yield_now();
            driver.watch.collect();
        }
        assert!(driver.watch.retry_at.is_some());

        driver.watch.retry_at = Some(Instant::now());
        let second = driver.next_watch(&["/a", "/b"]);
        driver.watch.reconcile(&both);
        assert!(
            driver.watch.pending.is_empty(),
            "nothing is read before the watch is up"
        );
        driver.report(&second, watching());
        assert_eq!(
            driver.watch.pending.keys().cloned().collect::<Vec<_>>(),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    /// A set change while a restarted watch is still starting passes its
    /// owed reread to the replacement.
    #[test]
    fn a_set_change_during_a_restart_keeps_the_owed_reread() {
        let mut driver = Driver::new();
        let first = driver.next_watch(&["/a"]);
        driver.watch.reconcile(&projects(&["/a"]));
        driver.report(&first, watching());
        drop(first);
        let deadline = Instant::now() + Duration::from_secs(5);
        while driver.watch.subscription.is_some() {
            assert!(
                Instant::now() < deadline,
                "the ended watch was never collected"
            );
            std::thread::yield_now();
            driver.watch.collect();
        }
        driver.watch.retry_at = Some(Instant::now());
        let _restarted = driver.next_watch(&["/a"]);
        driver.watch.reconcile(&projects(&["/a"]));
        let replacement = driver.next_watch(&["/a", "/b"]);
        driver.watch.reconcile(&projects(&["/a", "/b"]));
        driver.report(&replacement, watching());
        assert_eq!(
            driver.watch.pending.keys().cloned().collect::<Vec<_>>(),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    /// The old watch ending while its replacement is still starting leaves
    /// nothing watching, so the replacement owes a read of every project.
    #[test]
    fn the_old_watch_ending_before_its_replacement_watches_is_owed_a_reread() {
        let mut driver = Driver::new();
        let first = driver.next_watch(&["/a"]);
        driver.watch.reconcile(&projects(&["/a"]));
        driver.report(&first, watching());
        let second = driver.next_watch(&["/a", "/b"]);
        driver.watch.reconcile(&projects(&["/a", "/b"]));
        drop(first);
        let deadline = Instant::now() + Duration::from_secs(5);
        while driver.watch.outgoing.is_some() {
            assert!(
                Instant::now() < deadline,
                "the old watch's end was never collected"
            );
            std::thread::yield_now();
            driver.watch.collect();
        }
        assert!(driver.watch.pending.is_empty());
        driver.report(&second, watching());
        assert_eq!(
            driver.watch.pending.keys().cloned().collect::<Vec<_>>(),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    /// Changes the old watch lost while it stood as cover through a set
    /// change are owed to every project once the replacement watches.
    #[test]
    fn an_overflow_on_the_old_watch_during_a_set_change_is_owed_a_reread() {
        let mut driver = Driver::new();
        let first = driver.next_watch(&["/a"]);
        driver.watch.reconcile(&projects(&["/a"]));
        driver.report(&first, watching());
        let second = driver.next_watch(&["/a", "/b"]);
        driver.watch.reconcile(&projects(&["/a", "/b"]));
        driver.report(
            &first,
            GitWatchReport::Overflow {
                reason: "queue full".into(),
            },
        );
        assert!(driver.watch.pending.is_empty());
        driver.report(&second, watching());
        assert_eq!(
            driver.watch.pending.keys().cloned().collect::<Vec<_>>(),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    /// A set change while the end of the running watch waits uncollected
    /// does not hand that watch on as cover.
    #[test]
    fn a_set_change_over_an_uncollected_end_is_owed_a_reread() {
        let mut driver = Driver::new();
        let first = driver.next_watch(&["/a"]);
        driver.watch.reconcile(&projects(&["/a"]));
        driver.report(&first, watching());
        drop(first);
        let second = driver.next_watch(&["/a", "/b"]);
        // The end reaches the channel when the node's call returns, which
        // the test cannot observe; reconciling until the old watch is gone
        // either collects it there or sees it hand over and then end.
        driver.watch.reconcile(&projects(&["/a", "/b"]));
        let deadline = Instant::now() + Duration::from_secs(5);
        while driver.watch.outgoing.is_some() {
            assert!(
                Instant::now() < deadline,
                "the old watch's end was never collected"
            );
            std::thread::yield_now();
            driver.watch.collect();
        }
        driver.report(&second, watching());
        assert_eq!(
            driver.watch.pending.keys().cloned().collect::<Vec<_>>(),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    /// Adding a project starts a new watch, and the old one keeps reporting
    /// until the new one is watching, so a change in between is not lost and
    /// nothing is read again for the swap itself.
    #[test]
    fn a_change_while_the_watched_set_changes_is_kept() {
        let mut driver = Driver::new();
        let first = driver.next_watch(&["/a"]);
        driver.watch.reconcile(&projects(&["/a"]));
        driver.report(&first, watching());
        let second = driver.next_watch(&["/a", "/b"]);
        driver.watch.reconcile(&projects(&["/a", "/b"]));
        driver.report(
            &first,
            GitWatchReport::Changed {
                common_dirs: vec!["/a/.git".to_owned()],
            },
        );
        assert_eq!(
            driver.watch.pending.keys().cloned().collect::<Vec<_>>(),
            vec![PathBuf::from("/a")]
        );
        driver.watch.pending.clear();
        driver.report(&second, watching());
        assert!(driver.watch.pending.is_empty());
        driver.watch.collect();
        assert!(driver.watch.outgoing.is_none());
        assert!(driver.watch.pending.is_empty());
    }

    /// The old watch is still read for one pass after its replacement says
    /// it is watching, so a change it reports just then is not dropped with
    /// it.
    #[test]
    fn a_change_the_old_watch_reports_as_its_replacement_starts_is_kept() {
        let mut driver = Driver::new();
        let first = driver.next_watch(&["/a"]);
        driver.watch.reconcile(&projects(&["/a"]));
        driver.report(&first, watching());
        let second = driver.next_watch(&["/a", "/b"]);
        driver.watch.reconcile(&projects(&["/a", "/b"]));
        driver.report(&second, watching());
        assert!(driver.watch.pending.is_empty());
        driver.report(
            &first,
            GitWatchReport::Changed {
                common_dirs: vec!["/a/.git".to_owned()],
            },
        );
        assert_eq!(
            driver.watch.pending.keys().cloned().collect::<Vec<_>>(),
            vec![PathBuf::from("/a")]
        );
        assert!(driver.watch.outgoing.is_none());
    }

    /// A coordinator that falls behind keeps at most [`GIT_WATCH_MAILBOX`]
    /// reports; the ones dropped past it become one read of every project,
    /// named once in the log, and a heartbeat is never kept at all.
    #[test]
    fn a_full_mailbox_keeps_its_cap_and_reads_every_project_once() {
        let mut driver = Driver::new();
        let watch = driver.next_watch(&["/a", "/b"]);
        driver.watch.reconcile(&projects(&["/a", "/b"]));
        driver.report(&watch, watching());
        let forward = |driver: &Driver, report: GitWatchReport| {
            watch.send(report).unwrap();
            driver
                .forwarded
                .recv_timeout(Duration::from_secs(5))
                .expect("the node forwarded the report");
        };
        let held = |driver: &Driver| {
            lock(&driver.watch.subscription.as_ref().unwrap().mailbox)
                .reports
                .len()
        };
        forward(&driver, GitWatchReport::Quiet);
        assert_eq!(held(&driver), 0, "a heartbeat is not kept");
        for _ in 0..GIT_WATCH_MAILBOX + 10 {
            forward(
                &driver,
                GitWatchReport::Changed {
                    common_dirs: vec!["/a/.git".to_owned()],
                },
            );
        }
        assert_eq!(held(&driver), GIT_WATCH_MAILBOX);
        let ((), records) = crate::diagnostics::capture(|| driver.watch.collect());
        assert_eq!(
            driver.watch.pending.keys().cloned().collect::<Vec<_>>(),
            vec![PathBuf::from("/a"), PathBuf::from("/b")],
            "what was dropped may have named any project"
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record["kind"] == "git_watch.mailbox_full")
                .count(),
            1
        );
        assert_eq!(held(&driver), 0);
    }

    /// A watch rested while no renderer is attached ends on the node at its
    /// next report and its thread is joined; the next reconcile starts it
    /// again and, once it is watching, every project is read, since the rest
    /// was a lapse in cover.
    #[test]
    fn a_rested_watch_ends_on_the_node_and_its_restart_reads_every_project() {
        let mut driver = Driver::new();
        let first = driver.next_watch(&["/a", "/b"]);
        let both = projects(&["/a", "/b"]);
        driver.watch.reconcile(&both);
        driver.report(&first, watching());
        driver.watch.rest();
        assert!(driver.watch.subscription.is_none());
        first.send(GitWatchReport::Quiet).unwrap();
        driver
            .forwarded
            .recv_timeout(Duration::from_secs(5))
            .expect("the node forwarded the report");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !driver.watch.stopping.lock().unwrap().is_empty() {
            assert!(
                Instant::now() < deadline,
                "the rested watch's thread never ended"
            );
            std::thread::yield_now();
            driver.watch.reap();
        }
        assert!(
            first.send(GitWatchReport::Quiet).is_err(),
            "the node's watch call has returned"
        );

        let second = driver.next_watch(&["/a", "/b"]);
        driver.watch.reconcile(&both);
        assert!(
            driver.watch.pending.is_empty(),
            "nothing is read before the watch is up"
        );
        driver.report(&second, watching());
        assert_eq!(
            driver.watch.pending.keys().cloned().collect::<Vec<_>>(),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }
}

#[cfg(test)]
#[path = "worktree_behavior_tests.rs"]
mod behavior_tests;
