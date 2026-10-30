//! Whether GitHub's base of a Project takes a direct push, which decides if
//! a ready Hide AI branch offers `{base}에 바로 올리기` beside `PR 올리기`.
//!
//! The core reads it, never the shell: on a worker, off `Mutex<Runtime>`,
//! through the Project's node's `gh` (`github::base_push`), for each base a
//! ready branch of the opened Project names. An answer is kept for
//! [`FRESH_FOR`], an unreadable one too, so the 제안 tab composing again asks
//! nothing; a push the remote turned away forgets it, so it is read again.

use super::*;
use crate::model::{SuggestionBranchPush, SuggestionsGithub};
use serde_json::json;
use std::time::Duration;
use std::time::Instant;

/// How long an answer stands before a compose reads it again.
const FRESH_FOR: Duration = Duration::from_secs(5 * 60);

/// A base on a Project's machine: the node, the Project's folder and the
/// base branch.
type BaseKey = (String, String, String);

#[derive(Debug, Default)]
pub(super) struct BasePushWork {
    read: HashMap<BaseKey, (SuggestionBranchPush, Instant)>,
    reading: HashSet<BaseKey>,
}

fn key(node: &crate::node::NodeId, root: &str, base: &str) -> BaseKey {
    (node.as_str().to_owned(), root.to_owned(), base.to_owned())
}

impl Runtime {
    /// What GitHub last answered for `base`; `unknown` before an answer.
    pub(super) fn base_push(
        &self,
        node: &crate::node::NodeId,
        root: &str,
        base: &str,
    ) -> SuggestionBranchPush {
        self.base_push_work
            .read
            .get(&key(node, root, base))
            .map_or(SuggestionBranchPush::Unknown, |(push, _)| *push)
    }

    /// Every answer kept for the Project at `root` on `node`, by base.
    pub(super) fn base_push_answers(
        &self,
        node: &crate::node::NodeId,
        root: &str,
    ) -> HashMap<String, SuggestionBranchPush> {
        self.base_push_work
            .read
            .iter()
            .filter(|((at, folder, _), _)| at == node.as_str() && folder == root)
            .map(|((_, _, base), (push, _))| (base.clone(), *push))
            .collect()
    }

    /// Asks GitHub about each of `bases` whose answer is missing or stale and
    /// is not being read already, one worker each.
    pub(super) fn read_base_push(
        &mut self,
        node: &crate::node::NodeId,
        root: &str,
        bases: impl IntoIterator<Item = String>,
    ) {
        for base in bases {
            let base_key = key(node, root, &base);
            let fresh = self
                .base_push_work
                .read
                .get(&base_key)
                .is_some_and(|(_, at)| at.elapsed() < FRESH_FOR);
            if fresh || !self.base_push_work.reading.insert(base_key.clone()) {
                continue;
            }
            let Ok(link) = self.node_link(node.as_str()) else {
                self.base_push_work.reading.remove(&base_key);
                continue;
            };
            let folder = std::path::PathBuf::from(root);
            let carried = base_key.clone();
            let spawned = self.spawn_issue_worker(
                "base-push",
                move || crate::github::base_push(link.as_ref(), &folder, &base),
                move |runtime, answer| {
                    runtime.base_push_work.reading.remove(&carried);
                    let push = match answer {
                        Ok(true) => SuggestionBranchPush::Allowed,
                        Ok(false) => SuggestionBranchPush::PrOnly,
                        Err(code) => {
                            crate::diagnostic!(json!({
                                "component": "suggestions", "kind": "base_push.unread",
                                "node": carried.0, "base": carried.2, "reason": code,
                            }));
                            SuggestionBranchPush::Unknown
                        }
                    };
                    runtime
                        .base_push_work
                        .read
                        .insert(carried, (push, Instant::now()));
                    runtime.compose_suggestions()
                },
            );
            if spawned.is_err() {
                self.base_push_work.reading.remove(&base_key);
            }
        }
    }

    /// Records GitHub's answer for `base` as a read would.
    #[cfg(test)]
    pub(super) fn answer_base_push_for_test(
        &mut self,
        node: &crate::node::NodeId,
        root: &str,
        base: &str,
        push: SuggestionBranchPush,
    ) {
        self.base_push_work
            .read
            .insert(key(node, root, base), (push, Instant::now()));
    }

    /// Drops the answer for `base`, so the next compose asks GitHub again.
    pub(super) fn forget_base_push(&mut self, node: &crate::node::NodeId, root: &str, base: &str) {
        self.base_push_work.read.remove(&key(node, root, base));
    }

    /// Why a merge of a ready branch into `base` is refused, or `None` when
    /// it may go on. Without GitHub the local base is the real one, and only
    /// the local merge is offered. With GitHub connected, its base moves by
    /// pull request or by a push, never by a local merge alone, which would
    /// never reach GitHub and would split the local base from the remote's;
    /// a push goes on only while GitHub said the base takes one. An unread or
    /// unreachable GitHub counts as connected, as `suggestions_github` reads
    /// it, so Hide never merges locally before it knows.
    pub(super) fn merge_refused(
        &self,
        node: &crate::node::NodeId,
        root: &str,
        base: &str,
        push: bool,
    ) -> Option<&'static str> {
        let github = self.suggestions_github(node, root) == SuggestionsGithub::Connected;
        match (push, github) {
            (false, false) => None,
            (false, true) => Some("github_connected"),
            (true, false) => Some("github_missing"),
            (true, true) => match self.base_push(node, root, base) {
                SuggestionBranchPush::Allowed => None,
                SuggestionBranchPush::PrOnly => Some("pr_required"),
                SuggestionBranchPush::Unknown => Some("push_rule_unknown"),
            },
        }
    }
}
