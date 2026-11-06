//! Which projects a summary pass recomputes (with-hide/hide-private#571).
//!
//! A write marks what it changed: the sessions whose rows it wrote, or every
//! project when what it moved belongs to no one session (a project's facts,
//! lineage, a review, a delete, a prune, a ledger call, a write failure or
//! the store opening). The pass then recomputes a project only when one of
//! those sessions can reach its summary, judged by
//! [`LinkStore::session_reach`] against what the project's last pass read;
//! every other project keeps the summary it has, which is what a pass over it
//! would compute again.

use super::ProjectLinkSummary;
use super::SUMMARY_SESSION_LIMIT;
use super::ledger::SessionIdentity;
use super::store::{DevicePath, LinkStore, ProjectPass};
use super::within;
use std::collections::{BTreeMap, BTreeSet};

/// The most changed sessions held between passes; one more marks every
/// project, as a change with no known session does.
pub(super) const CHANGED_LIMIT: usize = 512;

/// What changed since the last summary pass.
#[derive(Debug, Default)]
pub(super) struct Dirty {
    all: bool,
    sessions: BTreeSet<SessionIdentity>,
}

impl Dirty {
    /// A change whose projects cannot be known: every project is recomputed.
    pub(super) fn all(&mut self) {
        self.all = true;
        self.sessions.clear();
    }

    /// A write to `session`'s rows.
    pub(super) fn session(&mut self, session: SessionIdentity) {
        if self.all {
            return;
        }
        if self.sessions.len() >= CHANGED_LIMIT && !self.sessions.contains(&session) {
            self.all();
            return;
        }
        self.sessions.insert(session);
    }
}

/// The projects one pass recomputes.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Scope {
    All,
    Projects(BTreeSet<String>),
}

impl Scope {
    pub(super) fn includes(&self, key: &str) -> bool {
        match self {
            Self::All => true,
            Self::Projects(keys) => keys.contains(key),
        }
    }
}

/// What a project's last computed summary read and named.
#[derive(Debug)]
struct Placed {
    paths: Vec<DevicePath>,
    repo: Option<String>,
    /// Every session id the summary names.
    sessions: BTreeSet<String>,
    /// A read of the pass failed and its summary holds an older part, or its
    /// session chips reached their limit and may leave a session out: the
    /// project is recomputed on every pass.
    always: bool,
}

/// What each project's last computed summary read, by project key.
#[derive(Debug, Default)]
pub(super) struct Placement(BTreeMap<String, Placed>);

impl Placement {
    /// The projects `dirty` can move, or all when its sessions cannot be
    /// placed.
    pub(super) fn scope(&self, store: &LinkStore, dirty: Dirty) -> Scope {
        if dirty.all {
            return Scope::All;
        }
        let always = self
            .0
            .iter()
            .filter(|(_, placed)| placed.always)
            .map(|(key, _)| key.clone());
        if dirty.sessions.is_empty() {
            return Scope::Projects(always.collect());
        }
        let reach = match store.session_reach(&dirty.sessions) {
            Ok(reach) => reach,
            Err(code) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "links", "kind": "summary.reach_failed", "code": code,
                    "sessions": dirty.sessions.len(),
                }));
                return Scope::All;
            }
        };
        let mut keys: BTreeSet<String> = always.collect();
        for (key, placed) in &self.0 {
            let reached = !placed.sessions.is_disjoint(&reach.ids)
                || reach.owners.contains(key)
                || placed
                    .repo
                    .as_ref()
                    .is_some_and(|repo| reach.repos.contains(repo))
                || reach.places.iter().any(|(device, cwd)| {
                    placed
                        .paths
                        .iter()
                        .any(|(at, path)| at == device && within(cwd, path))
                });
            if reached {
                keys.insert(key.clone());
            }
        }
        Scope::Projects(keys)
    }

    /// Records what a project's pass read (`None` when the store does not
    /// hold the project) and the summary it published.
    pub(super) fn place(
        &mut self,
        key: &str,
        pass: Option<&ProjectPass>,
        summary: &ProjectLinkSummary,
        failed: bool,
    ) {
        let mut sessions: BTreeSet<String> = summary.sessions.keys().cloned().collect();
        sessions.extend(summary.pr_workers.values().flatten().cloned());
        sessions.extend(
            summary
                .work
                .iter()
                .map(|work| work.session.session_id.clone()),
        );
        sessions.extend(
            summary
                .creators
                .iter()
                .map(|creator| creator.source.session_id.clone()),
        );
        self.0.insert(
            key.to_owned(),
            Placed {
                paths: pass.map(|pass| pass.paths().to_vec()).unwrap_or_default(),
                repo: pass.and_then(|pass| pass.repo().map(str::to_owned)),
                always: failed || summary.sessions.len() >= SUMMARY_SESSION_LIMIT,
                sessions,
            },
        );
    }

    /// Forgets the projects the runtime no longer hands in.
    pub(super) fn keep(&mut self, current: &BTreeSet<&str>) {
        self.0.retain(|key, _| current.contains(key.as_str()));
    }
}
