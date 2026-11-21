//! What a project's summary read of each pull request's sessions, held between
//! passes so a pass reads only the pull requests a change can move
//! (with-hide/hide-private#571).
//!
//! [`summary_scope`](super::summary_scope) decides which projects a pass
//! recomputes; a project with a thousand pull requests would still read every
//! one of them for one session's write. Each pull request's session lines are
//! a function of its own row, the project's places and names, and the rows of
//! the sessions that can join it, so an entry stays true until one of those
//! moves:
//!
//! - its row changed, or the project's places, fences or names did: the entry
//!   is read again by comparing them, not by being told;
//! - a session whose rows were written reaches it, which
//!   [`SessionReach`] names by the sessions a line already holds (what the
//!   write took out), the numbers the session printed, named or reviewed, and
//!   the branches it worked on (what the write put in);
//! - a change no session can place, a failed read or a project's removal
//!   drops every entry, as the pass over every project does.

use super::store::{PrRow, ProjectPass, SessionReach};
use super::{LinkedSession, SessionRole};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The most pull requests one project's entries are held for; one more is read
/// again on every pass, as it would be with no cache, and one diagnostic names
/// the crossing.
pub(super) const PR_CACHE_LIMIT: usize = 8_192;

/// What the summary reads of one session line of a pull request.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SummaryLine {
    /// Every session id of the line: the sessions it joins, oldest first.
    pub(super) ids: Vec<String>,
    /// The line's session made the pull request.
    pub(super) created: bool,
    /// The line did work on the pull request (made it or worked on its
    /// branch), not only printed its address.
    pub(super) worked: bool,
    pub(super) device: String,
    pub(super) agent: String,
    /// The id of the line's newest session.
    pub(super) id: String,
    pub(super) cwd: Option<String>,
    pub(super) started_at: Option<u64>,
}

impl From<&LinkedSession> for SummaryLine {
    fn from(line: &LinkedSession) -> Self {
        Self {
            ids: line.ids.clone(),
            created: line.role == SessionRole::Created,
            worked: line.role == SessionRole::Created || line.on_branch,
            device: line.device_id.clone(),
            agent: line.agent.clone(),
            id: line.id.clone(),
            cwd: line.cwd.clone(),
            started_at: line.started_at_unix_ms,
        }
    }
}

/// One pull request with its session lines, newest first.
#[derive(Debug)]
pub(super) struct PrSummary {
    pub(super) pr: PrRow,
    pub(super) lines: Vec<SummaryLine>,
    /// Every session id of its lines.
    sessions: BTreeSet<String>,
}

/// The entries of one project's pull requests.
#[derive(Debug)]
pub(super) struct PrCache {
    /// What the held entries were read under.
    pass: Option<ProjectPass>,
    held: BTreeMap<(String, u64), Arc<PrSummary>>,
    /// The most entries held.
    limit: usize,
    /// The entries crossed `limit` and the crossing is reported.
    over: bool,
    /// How many pull requests were read, from the cache's start.
    read: usize,
}

impl Default for PrCache {
    fn default() -> Self {
        Self {
            pass: None,
            held: BTreeMap::new(),
            limit: PR_CACHE_LIMIT,
            over: false,
            read: 0,
        }
    }
}

impl PrCache {
    /// A cache that holds at most `limit` entries.
    #[cfg(test)]
    pub(super) fn with_limit(limit: usize) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }

    /// How many pull requests this cache has read from the store.
    pub(super) fn read_count(&self) -> usize {
        self.read
    }

    /// Drops every entry a change reaches (see the module comment).
    pub(super) fn invalidate(&mut self, reach: &SessionReach) {
        if reach.is_empty() {
            return;
        }
        self.held.retain(|(_, number), entry| {
            !reach.numbers.contains(number)
                && !reach.branches.contains(&entry.pr.branch)
                && entry.sessions.is_disjoint(&reach.ids)
        });
    }

    /// Drops every entry.
    pub(super) fn clear(&mut self) {
        self.held.clear();
    }

    /// The entry of each of `prs`, in order: the held one when its row and the
    /// pass it was read under are the same, else `read` of it. A failed read
    /// leaves the cache empty, so the next pass reads every pull request.
    pub(super) fn entries(
        &mut self,
        pass: &ProjectPass,
        prs: Vec<PrRow>,
        mut read: impl FnMut(&PrRow) -> Result<Vec<SummaryLine>, String>,
    ) -> Result<Vec<Arc<PrSummary>>, String> {
        if self.pass.as_ref() != Some(pass) {
            self.held.clear();
            self.pass = Some(pass.clone());
        }
        let mut held = std::mem::take(&mut self.held);
        let mut kept = BTreeMap::new();
        let mut entries = Vec::with_capacity(prs.len());
        let mut over = false;
        for pr in prs {
            let key = (pr.repo.clone(), pr.number);
            let entry = match held.remove(&key).filter(|entry| entry.pr == pr) {
                Some(entry) => entry,
                None => {
                    let lines = read(&pr)?;
                    self.read += 1;
                    let sessions = lines
                        .iter()
                        .flat_map(|line| line.ids.iter().cloned())
                        .collect();
                    Arc::new(PrSummary {
                        pr,
                        lines,
                        sessions,
                    })
                }
            };
            if kept.len() < self.limit {
                kept.insert(key, Arc::clone(&entry));
            } else {
                over = true;
            }
            entries.push(entry);
        }
        self.held = kept;
        if over && !self.over {
            crate::diagnostic!(serde_json::json!({
                "component": "links", "kind": "summary.pr_cache_capacity",
                "limit": self.limit, "project": pass.project_key(),
            }));
        }
        self.over = over;
        Ok(entries)
    }
}
