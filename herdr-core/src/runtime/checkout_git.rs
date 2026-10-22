use std::time::{Duration, Instant};

use super::Runtime;
use crate::changes::ChannelRef;
use crate::checkout_git::{
    Ci, CiAnswer, CiRequest, CiState, CommitLink, GitAnswer, GitRequest, GitState, Header,
};
use crate::observed::{Freshness, Observation, now_unix_ms};

#[derive(Default)]
pub(super) struct State {
    pub(super) target: Option<GitRequest>,
    generation: u64,
    pub(super) header: Option<Header>,
    repository: Option<String>,
    last_git: Option<Instant>,
    /// When the Git read last succeeded, published only once the read fails
    /// or ages, so an unchanged read every two seconds publishes nothing.
    last_git_unix_ms: Option<u64>,
    last_ci: Option<Instant>,
}

impl Runtime {
    pub(crate) fn prepare_checkout_header(&mut self) -> (Option<GitRequest>, bool) {
        let generation = self.checkout_header.generation;
        let request = self.checkout_git_request();
        (request, generation != self.checkout_header.generation)
    }
    /// No I/O here: the same owning-node channel as Changes, one front only.
    pub(crate) fn checkout_git_request(&mut self) -> Option<GitRequest> {
        let next = (|| {
            if !self.ui_attached {
                return None;
            }
            let (workspace, checkout) = self.front_checkout_owned()?;
            let (project, _) = self.catalog_checkout(&workspace, &checkout)?;
            if !project.is_git || project.is_home {
                return None;
            }
            let root = self.document_root(&workspace, &checkout).ok()?;
            let channel = self.changes_channel(&root.device_id).map(ChannelRef);
            Some(GitRequest {
                root,
                channel,
                generation: self.checkout_header.generation,
            })
        })();
        if self.checkout_header.target != next {
            self.checkout_header.generation = self
                .checkout_header
                .generation
                .checked_add(1)
                .expect("checkout header generation exhausted");
            self.checkout_header.target = next.map(|mut target| {
                target.generation = self.checkout_header.generation;
                target
            });
            self.checkout_header.header = self.checkout_header.target.as_ref().map(|_| Header {
                state: GitState::Loading,
                facts: None,
                observed: None,
                reason: None,
                ci: None,
                recent_ci: Vec::new(),
                remote: None,
                commit_links: Vec::new(),
                commit_pages: false,
            });
            self.checkout_header.last_git = None;
            self.checkout_header.last_git_unix_ms = None;
            self.checkout_header.last_ci = None;
            self.checkout_header.repository = None;
        }
        self.checkout_header.target.clone()
    }

    pub(crate) fn checkout_ci_request(&self) -> Option<CiRequest> {
        let git = self.checkout_header.target.as_ref()?;
        let header = self.checkout_header.header.as_ref()?;
        if header.state != GitState::Fresh {
            return None;
        }
        let facts = header.facts.as_ref()?;
        let repository = self.checkout_header.repository.clone()?;
        Some(CiRequest {
            git: git.clone(),
            repository,
            sha: facts.head_sha.clone()?,
            commits: if facts.recent_commits.is_empty() {
                vec![facts.head_sha.clone()?]
            } else {
                facts
                    .recent_commits
                    .iter()
                    .map(|commit| commit.sha.clone())
                    .collect()
            },
            channel: ChannelRef(self.own_node.clone()),
        })
    }

    pub(crate) fn ingest_checkout_git(
        &mut self,
        (request, answer): GitAnswer,
        now: Instant,
    ) -> bool {
        if self.checkout_header.target.as_ref() != Some(&request) {
            return false;
        }
        let answer = answer.and_then(|facts| {
            let commits = &facts.recent_commits;
            if commits.len() > hide_node_link::worktrees::CHECKOUT_COMMIT_LIMIT
                || (!commits.is_empty()
                    && commits.first().map(|commit| &commit.sha) != facts.head_sha.as_ref())
                || commits
                    .iter()
                    .any(|commit| !hide_node_link::gh::is_commit(&commit.sha))
                || commits
                    .iter()
                    .map(|commit| &commit.sha)
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    != commits.len()
            {
                Err("The checkout returned invalid commit history".into())
            } else {
                Ok(facts)
            }
        });
        let before = self.checkout_header.header.clone();
        let header = self
            .checkout_header
            .header
            .as_mut()
            .expect("an active target has a header");
        match answer {
            Ok(mut facts) => {
                // The node needs the configured URL; the renderer must never receive
                // its userinfo. Keep only the validated provider identity privately.
                let repository = facts
                    .origin_url
                    .take()
                    .as_deref()
                    .and_then(hide_node_link::gh::repository_of_remote)
                    .filter(|repository| hide_node_link::gh::is_repository(repository));
                if self.checkout_header.repository != repository
                    || header.facts.as_ref().is_none_or(|previous| {
                        previous.head_sha != facts.head_sha
                            || previous.recent_commits != facts.recent_commits
                    })
                {
                    let commits: Vec<_> = if facts.recent_commits.is_empty() {
                        facts.head_sha.iter().cloned().collect()
                    } else {
                        facts
                            .recent_commits
                            .iter()
                            .map(|commit| commit.sha.clone())
                            .collect()
                    };
                    header.recent_ci = commits
                        .into_iter()
                        .map(|sha| Ci {
                            sha,
                            state: if repository.is_some() {
                                CiState::Loading
                            } else {
                                CiState::Unavailable
                            },
                            observed: None,
                            url: None,
                        })
                        .collect();
                    header.ci = header.recent_ci.first().cloned();
                    self.checkout_header.last_ci = None;
                }
                self.checkout_header.repository = repository;
                header.facts = Some(facts);
                header.state = GitState::Fresh;
                let read_at = now_unix_ms();
                self.checkout_header.last_git_unix_ms = Some(read_at);
                if header
                    .observed
                    .is_none_or(|observed| observed.freshness != Freshness::Fresh)
                {
                    header.observed = Some(Observation::fresh(read_at));
                }
                header.reason = None;
                self.checkout_header.last_git = Some(now);
            }
            Err(reason) => {
                header.state = if header.facts.is_some() {
                    GitState::Stale
                } else {
                    GitState::Unavailable
                };
                header.observed = Some(Observation {
                    freshness: Freshness::Failed,
                    read_at_unix_ms: self.checkout_header.last_git_unix_ms,
                });
                header.reason = Some(reason);
                // The results stay, for a commit the header no longer knows
                // is still HEAD (D-09).
                for ci in header.ci.iter_mut().chain(header.recent_ci.iter_mut()) {
                    ci.observed = ci.observed.map(Observation::aged);
                }
            }
        }
        before != self.checkout_header.header
    }

    pub(crate) fn ingest_checkout_ci(&mut self, (request, answer): CiAnswer, now: Instant) -> bool {
        if self.checkout_ci_request().as_ref() != Some(&request) {
            return false;
        }
        let header = self
            .checkout_header
            .header
            .as_mut()
            .expect("the CI request has a header");
        let answer = match answer {
            Ok(answer) => answer,
            // GitHub could not be read: each commit keeps the result it last
            // had, marked as a failed read, and one never answered says so
            // (B22).
            Err(_) => request
                .commits
                .iter()
                .map(|sha| {
                    let before = header.recent_ci.iter().find(|ci| &ci.sha == sha);
                    match before.filter(|ci| ci.observed.is_some()) {
                        Some(before) => Ci {
                            observed: Some(Observation::failed(before.observed)),
                            ..before.clone()
                        },
                        None => Ci {
                            sha: sha.clone(),
                            state: CiState::Unavailable,
                            observed: Some(Observation::failed(None)),
                            url: None,
                        },
                    }
                })
                .collect(),
        };
        if answer.iter().map(|ci| &ci.sha).ne(request.commits.iter()) {
            return false;
        }
        let changed = header.recent_ci != answer;
        header.ci = answer.first().cloned();
        header.recent_ci = answer;
        self.checkout_header.last_ci = Some(now);
        changed
    }

    /// Explicit fresh-to-stale transitions only; an unchanged Git read updates
    /// its internal deadline without publishing a snapshot every two seconds.
    pub(crate) fn age_checkout_header(&mut self, now: Instant) -> bool {
        let Some(header) = self.checkout_header.header.as_mut() else {
            return false;
        };
        let states = |header: &Header| {
            (
                header.state,
                header.observed,
                header.ci.as_ref().map(|ci| ci.observed),
                header
                    .recent_ci
                    .iter()
                    .map(|ci| ci.observed)
                    .collect::<Vec<_>>(),
            )
        };
        let before = states(header);
        if self
            .checkout_header
            .last_git
            .is_some_and(|read| now.saturating_duration_since(read) > Duration::from_secs(5))
        {
            header.state = GitState::Stale;
            let read_at_unix_ms = self.checkout_header.last_git_unix_ms;
            header.observed = header.observed.map(|observed| Observation {
                read_at_unix_ms,
                ..observed.aged()
            });
        }
        if header.state != GitState::Fresh
            || self
                .checkout_header
                .last_ci
                .is_some_and(|read| now.saturating_duration_since(read) > Duration::from_secs(90))
        {
            for ci in header.ci.iter_mut().chain(header.recent_ci.iter_mut()) {
                ci.observed = ci.observed.map(Observation::aged);
            }
        }
        states(header) != before
    }

    pub(super) fn checkout_header_snapshot(&self, key: &(String, String)) -> Option<Header> {
        let target = self.checkout_header.target.as_ref()?;
        if target.root.device_id != key.0 || target.root.path != key.1 {
            return None;
        }
        let mut header = self.checkout_header.header.clone()?;
        header.remote = self.remote_check(&key.0, &key.1);
        header.commit_links = commit_links(self.checkout_header.repository.as_deref(), &header);
        header.commit_pages = self.checkout_header.repository.is_some();
        Some(header)
    }

    /// The header the last snapshot carries.
    pub(crate) fn published_checkout_header(&self) -> Option<Header> {
        self.snapshot
            .workspace_view
            .as_ref()
            .and_then(|view| view.git.clone())
    }

    pub(crate) fn refresh_checkout_header(&mut self) -> Option<Header> {
        let key = self
            .snapshot
            .workspace_view
            .as_ref()
            .map(|view| (view.device_id.clone(), view.path.clone()));
        let header = key
            .as_ref()
            .and_then(|key| self.checkout_header_snapshot(key));
        if let Some(view) = self.snapshot.workspace_view.as_mut() {
            view.git = header.clone();
        }
        header
    }
}

/// The GitHub commit page of each recent commit some remote holds and of
/// each incoming one, for a checkout whose origin is a validated GitHub
/// repository; a commit no remote holds has no page to open (D-05).
fn commit_links(repository: Option<&str>, header: &Header) -> Vec<CommitLink> {
    let (Some(repository), Some(facts)) = (repository, header.facts.as_ref()) else {
        return Vec::new();
    };
    facts
        .recent_commits
        .iter()
        .filter(|commit| !facts.unpushed_commits.contains(&commit.sha))
        .chain(&facts.incoming_commits)
        .filter(|commit| hide_node_link::gh::is_commit(&commit.sha))
        .map(|commit| CommitLink {
            sha: commit.sha.clone(),
            url: format!("https://github.com/{repository}/commit/{}", commit.sha),
        })
        .collect()
}
