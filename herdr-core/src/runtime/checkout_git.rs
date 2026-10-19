use std::time::{Duration, Instant};

use super::Runtime;
use crate::changes::ChannelRef;
use crate::checkout_git::{
    Ci, CiAnswer, CiRequest, CiState, GitAnswer, GitRequest, GitState, Header,
};

#[derive(Default)]
pub(super) struct State {
    pub(super) target: Option<GitRequest>,
    generation: u64,
    pub(super) header: Option<Header>,
    last_git: Option<Instant>,
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
                reason: None,
                ci: None,
            });
            self.checkout_header.last_git = None;
            self.checkout_header.last_ci = None;
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
        let repository = facts
            .origin_url
            .as_deref()
            .and_then(hide_node_link::gh::repository_of_remote)?;
        if !hide_node_link::gh::is_repository(&repository) {
            return None;
        }
        Some(CiRequest {
            git: git.clone(),
            repository,
            sha: facts.head_sha.clone()?,
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
        let before = self.checkout_header.header.clone();
        let header = self
            .checkout_header
            .header
            .as_mut()
            .expect("an active target has a header");
        match answer {
            Ok(facts) => {
                if header.facts.as_ref().is_none_or(|previous| {
                    previous.head_sha != facts.head_sha || previous.origin_url != facts.origin_url
                }) {
                    let readable = facts
                        .origin_url
                        .as_deref()
                        .and_then(hide_node_link::gh::repository_of_remote)
                        .is_some_and(|repository| hide_node_link::gh::is_repository(&repository));
                    header.ci = facts.head_sha.as_ref().map(|sha| Ci {
                        sha: sha.clone(),
                        state: if readable {
                            CiState::Loading
                        } else {
                            CiState::Unavailable
                        },
                        checked_at_unix_ms: None,
                        url: None,
                    });
                    self.checkout_header.last_ci = None;
                }
                header.facts = Some(facts);
                header.state = GitState::Fresh;
                header.reason = None;
                self.checkout_header.last_git = Some(now);
            }
            Err(reason) => {
                header.state = if header.facts.is_some() {
                    GitState::Stale
                } else {
                    GitState::Unavailable
                };
                header.reason = Some(reason);
                if let Some(ci) = header.ci.as_mut() {
                    ci.state = CiState::Stale;
                    ci.url = None;
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
        let changed = header.ci.as_ref() != Some(&answer);
        header.ci = Some(answer);
        self.checkout_header.last_ci = Some(now);
        changed
    }

    /// Explicit fresh-to-stale transitions only; an unchanged Git read updates
    /// its internal deadline without publishing a snapshot every two seconds.
    pub(crate) fn age_checkout_header(&mut self, now: Instant) -> bool {
        let Some(header) = self.checkout_header.header.as_mut() else {
            return false;
        };
        let before = (header.state, header.ci.as_ref().map(|ci| ci.state));
        if self
            .checkout_header
            .last_git
            .is_some_and(|read| now.saturating_duration_since(read) > Duration::from_secs(5))
        {
            header.state = GitState::Stale;
        }
        if (header.state != GitState::Fresh
            || self
                .checkout_header
                .last_ci
                .is_some_and(|read| now.saturating_duration_since(read) > Duration::from_secs(90)))
            && let Some(ci) = header.ci.as_mut()
        {
            ci.state = CiState::Stale;
            ci.url = None;
        }
        (header.state, header.ci.as_ref().map(|ci| ci.state)) != before
    }

    pub(super) fn checkout_header_snapshot(&self, key: &(String, String)) -> Option<Header> {
        let target = self.checkout_header.target.as_ref()?;
        if target.root.device_id != key.0 || target.root.path != key.1 {
            return None;
        }
        self.checkout_header.header.clone()
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
