//! The runtime's half of the advisory GitHub poll (`crate::github_poll`).
//!
//! The poll reads no state of the runtime's and writes none: this half builds
//! the request it reads under the lock from the last answers, and takes back
//! only a list of project paths it saw change. A path in that list is asked
//! for through the rule a look uses (`read_when_looked_at`) without its
//! thirty-second age, so a poll can never ask while a read is out, in failure
//! backoff, or for a repository with no GitHub remote, and never sets
//! `loading`.

use super::projects::{GithubClock, LookRead};
use super::*;
use crate::code_host::{Host, WatchRequest, Watched};

impl Runtime {
    /// The projects to poll now: those whose last read worked and holds open
    /// pull requests, and the project in front even with none, so a pull
    /// request opened elsewhere shows within the minute (PRD git-freshness
    /// D-10); the one in front first, at most the host's
    /// [`crate::code_host::Watch::project_limit`] (a count past it is logged
    /// as `poll.over_limit` when it changes). Only `host`'s projects are in
    /// it. Plain data in hand, with no I/O, so it is built under the lock.
    pub(crate) fn github_poll_request(&mut self, host: Host) -> WatchRequest {
        let Some(watch) = host.unit().watch() else {
            return WatchRequest::default();
        };
        let (project_limit, head_limit) = (watch.project_limit(), watch.head_limit());
        let in_front = self
            .focused_local_checkout()
            .map(|(workspace, _)| workspace.path.clone());
        let mut projects: Vec<Watched> = self
            .github_projects()
            .0
            .into_iter()
            .filter(|workspace| {
                matches!(
                    self.github_clock.get(&workspace.path),
                    Some(GithubClock::Answered {
                        failures: 0,
                        pull_requests_read: true,
                        ..
                    })
                )
            })
            .filter_map(|workspace| {
                self.poll_project(
                    host,
                    head_limit,
                    &workspace.path,
                    in_front.as_ref() == Some(&workspace.path),
                )
            })
            .collect();
        projects.sort_by(|left, right| {
            let rank = |project: &Watched| Some(&project.path) != in_front.as_ref();
            (rank(left), &left.path).cmp(&(rank(right), &right.path))
        });
        let over_limit = projects.len().saturating_sub(project_limit);
        if over_limit != self.github_poll_over_limit {
            self.github_poll_over_limit = over_limit;
            if over_limit > 0 {
                crate::diagnostic!(serde_json::json!({
                    "component": "github",
                    "kind": "poll.over_limit",
                    "limit": project_limit,
                    "with_open_pull_requests": projects.len(),
                    "not_polled": over_limit,
                }));
            }
        }
        projects.truncate(project_limit);
        WatchRequest { projects }
    }

    /// What the watch reads in one project of `host`'s: the repository its
    /// read named, which every pull request of its list is in, and the head
    /// commits of the open ones whose checks had not finished
    /// (`checks_unfinished`, which a mission's CI letter waits on too). A
    /// project with none open is watched only while in front, so the list's
    /// first pull request is seen.
    fn poll_project(
        &self,
        host: Host,
        head_limit: usize,
        path: &str,
        in_front: bool,
    ) -> Option<Watched> {
        let project = self.github.project(path)?;
        let open: Vec<&crate::model::PullRequestSnapshot> = project
            .pull_requests
            .iter()
            .filter(|pull_request| !pull_request.badge.is_settled())
            .collect();
        if open.is_empty() && !in_front {
            return None;
        }
        let repository = project
            .repository
            .as_ref()
            .map(|identity| identity.repository.clone())
            .or_else(|| {
                let (named, repository) = self
                    .code_hosts
                    .repository(project.issues.repository.as_deref()?)?;
                (named == host).then_some(repository)
            })
            .filter(|repository| self.code_hosts.of(repository) == Some(host))?;
        let heads: BTreeSet<String> = open
            .iter()
            .filter(|pull_request| pull_request.checks_unfinished())
            .filter_map(|pull_request| pull_request.head_oid.clone())
            .filter(|head| hide_node_link::gh::is_commit(head))
            .take(head_limit)
            .collect();
        Some(Watched {
            path: path.to_owned(),
            repository,
            heads: heads.into_iter().collect(),
            in_front,
        })
    }

    /// The poll found a change in these projects. They wait in a set bounded
    /// by the projects read, and a path no read project owns is dropped.
    pub(crate) fn note_polled_changes(&mut self, changed: Vec<String>) {
        for path in changed {
            if self.github_clock.contains_key(&path) {
                self.github_polled.insert(path);
            }
        }
    }

    /// The poll is not watching these projects now: it paused one after a
    /// failure, or every one at the rate limit's floor. Their pull request
    /// marks read as stale, as after a failed read, until the next read of
    /// the project answers (PRD git-freshness B23); the reason is in the log.
    pub(crate) fn note_poll_held(&mut self, held: Vec<String>) -> bool {
        let mut changed = false;
        for path in held {
            if let Some(project) = self
                .github
                .projects
                .iter_mut()
                .find(|project| project.root_path == path)
                && !project.status.stale
            {
                project.status.stale = true;
                changed = true;
            }
        }
        if changed {
            self.apply_pull_requests();
            self.refresh_worktree_projection();
        }
        changed
    }

    /// Asks the existing read again for each project the poll saw change, by
    /// the rule a look follows but at any age: a change GitHub reported is
    /// news a read that already answered may predate, and the poll's own
    /// spacing bounds how often it comes. A project with an ask out stays
    /// marked and is asked when that answers; one that cannot be asked
    /// (failed, or no GitHub remote) is let go, since its own retry or a
    /// remote change will read it.
    pub(crate) fn ask_polled_changes(&mut self, now: std::time::Instant) {
        if self.github_polled.is_empty() {
            return;
        }
        let marked: Vec<String> = self.github_polled.iter().cloned().collect();
        for path in marked {
            match self.read_when_asked(&path, now, std::time::Duration::ZERO) {
                LookRead::Later => {}
                LookRead::Asked => {
                    self.github_polled.remove(&path);
                    crate::diagnostic!(serde_json::json!({
                        "component": "github",
                        "kind": "poll.asked",
                        "project": path,
                    }));
                }
                LookRead::Never => {
                    self.github_polled.remove(&path);
                }
            }
        }
    }
}
