//! The background fetch's schedule (PRD git-freshness D-01, D-02): which
//! registered repository each node fetches next, and what its last fetch
//! said. The fetches run on `crate::fetch::FetchPump`'s threads, one per
//! node at a time; nothing here does I/O, so it runs under the lock.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hide_node_link::worktrees::{FetchFailure, FetchOutcome, FetchSkip};
use serde::Serialize;

use super::Runtime;

/// The repository in front is fetched at most this often, and again when the
/// operator's window comes forward after this long.
const FRONT_EVERY: Duration = Duration::from_secs(60);
/// Every other registered repository is fetched at most this often.
const OTHERS_EVERY: Duration = Duration::from_secs(5 * 60);
/// After the n-th failure in a row a repository waits at least this long.
const BACKOFF: [Duration; 4] = [
    Duration::from_secs(60),
    Duration::from_secs(2 * 60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(15 * 60),
];
/// The most repositories the fetch follows: the worktree reader's cap. One
/// past it is not fetched and is logged once (`fetch.over_limit`).
pub(crate) const FETCH_REPOSITORY_LIMIT: usize = hide_node_link::worktrees::GIT_WATCH_LIMIT;

/// One repository: the device that holds it and its project path there.
pub(crate) type RepositoryKey = (String, String);

/// One fetch the pump runs. `sequence` tells two fetches of one repository
/// apart, so an answer is applied to the fetch that asked for it only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FetchJob {
    pub(crate) key: RepositoryKey,
    pub(crate) sequence: u64,
}

/// What the header says about the remote of the checkout in front.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct RemoteCheck {
    /// Hide fetches this repository in the background. Off, the time the
    /// remote was last seen is the operator's own fetch (`FETCH_HEAD`).
    pub(crate) auto: bool,
    /// When Hide's fetch last succeeded; absent before the first success.
    pub(crate) checked_at_unix_ms: Option<u64>,
    /// The last fetch failed, so the counts are as of `checked_at_unix_ms`.
    pub(crate) failing: bool,
}

#[derive(Default)]
pub(super) struct State {
    repositories: BTreeMap<RepositoryKey, Repository>,
    /// The fetch out on each node, by device.
    running: BTreeMap<String, FetchJob>,
    sequence: u64,
    over_limit: usize,
}

#[derive(Clone, Debug, Default)]
struct Repository {
    last_attempt: Option<Instant>,
    fetched_at_unix_ms: Option<u64>,
    failures: u32,
    /// The last failure, logged once until it changes or ends.
    failure: Option<FetchFailure>,
    /// A credential or the network failed: nothing is asked again until the
    /// operator's window comes forward (D-02).
    paused: bool,
    /// The window came forward: due regardless of its interval.
    looked: bool,
}

impl Repository {
    fn due(&self, front: bool, now: Instant) -> bool {
        if self.paused {
            return false;
        }
        let Some(last) = self.last_attempt else {
            return true;
        };
        if self.looked {
            return true;
        }
        let every = if front { FRONT_EVERY } else { OTHERS_EVERY };
        let backoff = match self.failures {
            0 => Duration::ZERO,
            failures => BACKOFF[usize::try_from(failures - 1).unwrap_or(usize::MAX).min(3)],
        };
        now.saturating_duration_since(last) >= every.max(backoff)
    }
}

impl Runtime {
    /// The registered Git repositories on every device, the one in front
    /// first, with the Project Settings switch for each.
    fn fetch_repositories(&mut self) -> Vec<(RepositoryKey, bool)> {
        let front = self.front_repository();
        let mut repositories: Vec<(RepositoryKey, bool)> = Vec::new();
        for workspace in self.catalog_workspaces() {
            if !workspace.is_git || workspace.is_home {
                continue;
            }
            let key = (workspace.device_id.clone(), workspace.path.clone());
            if repositories.iter().any(|(known, _)| *known == key) {
                continue;
            }
            let auto = crate::project_settings::auto_fetch(
                &self.snapshot.ui_state.project_settings,
                &key.0,
                &key.1,
            );
            repositories.push((key, auto));
        }
        repositories.sort_by_key(|(key, _)| Some(key) != front.as_ref());
        let over_limit = repositories.len().saturating_sub(FETCH_REPOSITORY_LIMIT);
        if over_limit != self.fetch.over_limit {
            self.fetch.over_limit = over_limit;
            if over_limit > 0 {
                crate::diagnostic!(serde_json::json!({
                    "component": "fetch",
                    "kind": "fetch.over_limit",
                    "limit": FETCH_REPOSITORY_LIMIT,
                    "not_fetched": over_limit,
                }));
            }
        }
        repositories.truncate(FETCH_REPOSITORY_LIMIT);
        repositories
    }

    /// The repository of the checkout in front, by its device and project.
    fn front_repository(&self) -> Option<RepositoryKey> {
        let (workspace, checkout) = self.front_checkout()?;
        let (workspace, _) = self.catalog_checkout(workspace, checkout)?;
        (workspace.is_git && !workspace.is_home)
            .then(|| (workspace.device_id.clone(), workspace.path.clone()))
    }

    /// The fetches to start now: at most one per node, none while no window
    /// is attached (D-02), each with its node's link and the path it fetches.
    pub(crate) fn due_fetches(
        &mut self,
        now: Instant,
    ) -> Vec<(FetchJob, Arc<dyn crate::node_access::NodeLink>)> {
        if !self.ui_attached {
            return Vec::new();
        }
        let repositories = self.fetch_repositories();
        let front = self.front_repository();
        self.fetch.repositories.retain(|key, _| {
            repositories
                .iter()
                .any(|(listed, auto)| listed == key && *auto)
        });
        let mut jobs = Vec::new();
        for (key, auto) in repositories {
            if !auto || self.fetch.running.contains_key(&key.0) {
                continue;
            }
            let repository = self.fetch.repositories.entry(key.clone()).or_default();
            if !repository.due(Some(&key) == front.as_ref(), now) {
                continue;
            }
            // A device whose link is down is not asked; its fetch is due
            // again as soon as the link is back.
            let Ok((_, link)) = self.linked_node(&key.0) else {
                continue;
            };
            let repository = self.fetch.repositories.entry(key.clone()).or_default();
            repository.last_attempt = Some(now);
            repository.looked = false;
            self.fetch.sequence = self.fetch.sequence.wrapping_add(1);
            let job = FetchJob {
                key,
                sequence: self.fetch.sequence,
            };
            self.fetch.running.insert(job.key.0.clone(), job.clone());
            jobs.push((job, link));
        }
        jobs
    }

    /// A fetch ended. Answers whether what the header shows of the remote
    /// may have changed.
    pub(crate) fn ingest_fetch(
        &mut self,
        job: &FetchJob,
        outcome: Result<FetchOutcome, String>,
        now_unix_ms: u64,
    ) -> bool {
        if self.fetch.running.get(&job.key.0) == Some(job) {
            self.fetch.running.remove(&job.key.0);
        }
        let Some(repository) = self.fetch.repositories.get_mut(&job.key) else {
            return false;
        };
        let before = (
            repository.fetched_at_unix_ms,
            repository.failures > 0 || repository.paused,
        );
        let failure = match outcome {
            Ok(FetchOutcome::Fetched) => {
                repository.fetched_at_unix_ms = Some(now_unix_ms);
                None
            }
            // The operator's own fetch is running: theirs brings the refs.
            Ok(FetchOutcome::Skipped {
                reason: FetchSkip::Busy,
            }) => return false,
            Ok(FetchOutcome::Skipped {
                reason: FetchSkip::NoOrigin,
            }) => None,
            Ok(FetchOutcome::Failed { failure, reason }) => Some((failure, reason)),
            Err(reason) => Some((FetchFailure::Other, reason)),
        };
        match failure {
            None => {
                if repository.failure.take().is_some() {
                    crate::diagnostic!(serde_json::json!({
                        "component": "fetch",
                        "kind": "fetch.recovered",
                        "device": job.key.0,
                        "project": job.key.1,
                        "failures": repository.failures,
                    }));
                }
                repository.failures = 0;
                repository.paused = false;
            }
            Some((failure, reason)) => {
                repository.failures = repository.failures.saturating_add(1);
                repository.paused = matches!(failure, FetchFailure::Auth | FetchFailure::Offline);
                if repository.failure != Some(failure) {
                    crate::diagnostic!(serde_json::json!({
                        "component": "fetch",
                        "kind": "fetch.failed",
                        "device": job.key.0,
                        "project": job.key.1,
                        "failure": failure,
                        "reason": reason,
                        "paused_until_focus": repository.paused,
                    }));
                }
                repository.failure = Some(failure);
            }
        }
        before
            != (
                repository.fetched_at_unix_ms,
                repository.failures > 0 || repository.paused,
            )
    }

    /// The operator's window came forward: a fetch that stopped on a
    /// credential or the network may be asked again, and the repository in
    /// front is fetched now when its last try is older than a minute.
    pub(crate) fn look_for_fetch(&mut self, now: Instant) {
        for repository in self.fetch.repositories.values_mut() {
            repository.paused = false;
        }
        if let Some(front) = self.front_repository()
            && let Some(repository) = self.fetch.repositories.get_mut(&front)
            && repository
                .last_attempt
                .is_some_and(|last| now.saturating_duration_since(last) >= FRONT_EVERY)
        {
            repository.looked = true;
        }
    }

    /// What the header says of the remote of the checkout at `path` on
    /// `device`.
    pub(super) fn remote_check(&self, device: &str, path: &str) -> Option<RemoteCheck> {
        let workspace = self.catalog_workspaces().find(|workspace| {
            workspace.device_id == device
                && workspace
                    .checkouts
                    .iter()
                    .any(|checkout| checkout.path == path)
        })?;
        let key = (workspace.device_id.clone(), workspace.path.clone());
        let auto = crate::project_settings::auto_fetch(
            &self.snapshot.ui_state.project_settings,
            &key.0,
            &key.1,
        );
        let repository = self.fetch.repositories.get(&key);
        Some(RemoteCheck {
            auto,
            checked_at_unix_ms: auto
                .then(|| repository.and_then(|repository| repository.fetched_at_unix_ms))
                .flatten(),
            failing: auto
                && repository
                    .is_some_and(|repository| repository.failures > 0 || repository.paused),
        })
    }
}

#[cfg(test)]
impl Runtime {
    pub(super) fn fetch_running_jobs(&self) -> Vec<FetchJob> {
        self.fetch.running.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_is_due_by_its_place_its_failures_and_a_look() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut repository = Repository::default();
        assert!(repository.due(false, start), "never fetched");
        repository.last_attempt = Some(start);
        assert!(!repository.due(true, at(59)));
        assert!(repository.due(true, at(60)));
        assert!(!repository.due(false, at(299)));
        assert!(repository.due(false, at(300)));
        // Failures back off 1, 2, 5 and 15 minutes, never sooner than the
        // repository's own interval.
        for (failures, wait) in [(1, 60), (2, 120), (3, 300), (4, 900), (9, 900)] {
            repository.failures = failures;
            assert!(!repository.due(true, at(wait - 1)), "{failures}");
            assert!(repository.due(true, at(wait)), "{failures}");
        }
        repository.failures = 1;
        assert!(!repository.due(false, at(299)));
        // A credential or network failure waits for the window to come
        // forward, and a look makes the repository due at once.
        repository.paused = true;
        assert!(!repository.due(true, at(10_000)));
        repository.paused = false;
        repository.looked = true;
        assert!(repository.due(false, at(1)));
    }
}
