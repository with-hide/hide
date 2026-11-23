//! One gitlab.com merge request read by its number, and what rests on that
//! read: the merge at the head the operator saw, the merge proof and the
//! read by number.
//!
//! The merge is the one write Hide makes on gitlab.com, and what happened is
//! decided by the merge request's state read before and after it, never by
//! `glab`'s exit code or its text: gitlab.com answers a moved head with 409
//! and exit 1, while `glab` exits 0 both when it merged and when it only set
//! auto-merge. How each state is judged is written in `docs/status-model.md`,
//! Code hosts.

use hide_node_link::change_request::PrMergeMethod;
use hide_node_link::glab::{
    MERGED_LIMIT, merge_arguments, merge_base_arguments, merge_request_arguments, merged_arguments,
};
use serde::Deserialize;

use super::budget::{self, Share};
use super::{Refusal, glab, glab_taken, repository_of};
use crate::code_host::{
    ChangeHead, ChangeHeadFailure, HeadMerge, MergeProof, MergedHead, MergedHeads, Place,
    RepositoryRef, Standing, WriteFailure,
};
use crate::model::GithubFailureCategory;
use crate::node_access::NodeLink;

/// The requests one merge takes from the operator's share together: the
/// read before, the merge line (`glab` reads the merge request, then sends
/// the merge and retries it twice on a 406), and at most two reads after.
const MERGE_REQUESTS: usize = 7;

/// How a merge request stands, as gitlab.com names it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum State {
    Opened,
    Closed,
    /// Being merged now: gitlab.com holds it while the merge runs.
    Locked,
    Merged,
}

/// One merge request as the REST list answers it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct Item {
    state: State,
    /// Its head commit.
    sha: String,
    #[serde(default)]
    merge_commit_sha: Option<String>,
    #[serde(default)]
    squash_commit_sha: Option<String>,
    /// Auto-merge is set: gitlab.com merges it when its pipeline passes.
    #[serde(default)]
    merge_when_pipeline_succeeds: bool,
    #[serde(default)]
    detailed_merge_status: Option<String>,
    #[serde(default)]
    has_conflicts: bool,
    source_branch: String,
    target_branch: String,
    source_project_id: u64,
    target_project_id: u64,
}

impl Item {
    fn at(&self, head: &str) -> bool {
        self.sha.eq_ignore_ascii_case(head)
    }
}

fn parse_items(output: &str) -> Result<Vec<Item>, String> {
    serde_json::from_str(output)
        .map_err(|error| format!("glab returned merge requests Hide could not read: {error}"))
}

/// The merge request `number` of `repository`, `None` when it has none.
fn read(
    node: &dyn NodeLink,
    repository: &RepositoryRef,
    number: u32,
    share: Option<Share>,
) -> Result<Option<Item>, Refusal> {
    let arguments = merge_request_arguments(&repository.path, number).ok_or_else(|| {
        Refusal::Unreadable(format!(
            "merge request {number} of {} cannot be named",
            repository.path
        ))
    })?;
    let output = match share {
        Some(share) => glab(node, arguments, share)?,
        None => glab_taken(node, arguments)?,
    };
    let mut items = parse_items(&output).map_err(Refusal::Unreadable)?;
    match items.len() {
        0 | 1 => Ok(items.pop()),
        listed => Err(Refusal::Unreadable(format!(
            "gitlab.com named {listed} merge requests for number {number}"
        ))),
    }
}

fn repository(place: &Place) -> Result<&RepositoryRef, String> {
    repository_of(place).ok_or_else(|| "the checkout names no gitlab.com project".to_owned())
}

/// The answer once the merge line may have left and nothing says whether it
/// landed: the host's outcome is unknown, never a failure.
const UNKNOWN: &str = "unreachable";

/// What the read before a merge allows.
#[derive(Debug, Eq, PartialEq)]
enum Before {
    /// Merged at the head the operator saw: asked again, it is done.
    Done,
    /// Nothing is written, for this reason.
    Refused(&'static str),
    Write,
}

fn before(item: Option<&Item>, head: &str) -> Before {
    let Some(item) = item else {
        return Before::Refused("not_mergeable");
    };
    match item.state {
        State::Merged if item.at(head) => Before::Done,
        // Merged at another head, or open at another: what the operator
        // judged is not what would merge or did.
        State::Merged => Before::Refused("head_moved"),
        _ if !item.at(head) => Before::Refused("head_moved"),
        State::Closed => Before::Refused("not_mergeable"),
        // A merge is running and has not landed yet: its outcome is not
        // known, and nothing is written over it.
        State::Locked => Before::Refused(UNKNOWN),
        State::Opened => Before::Write,
    }
}

/// What the merge request's state after the merge line says happened; the
/// line's own answer goes to the log only. A state that cannot say whether
/// the merge landed (no such merge request in the answer, or gitlab.com
/// still merging it) is `unreachable`: the outcome is unknown, and asking
/// again reads it before writing anything.
fn after(item: Option<&Item>, head: &str) -> Result<(), &'static str> {
    let Some(item) = item else {
        return Err(UNKNOWN);
    };
    if !item.at(head) {
        return Err("head_moved");
    }
    match item.state {
        State::Merged => Ok(()),
        State::Closed => Err("not_mergeable"),
        State::Locked => Err(UNKNOWN),
        State::Opened if item.merge_when_pipeline_succeeds => Err("auto_merge"),
        State::Opened => Err(open_reason(item)),
    }
}

/// Why an open merge request at the head was not merged, from what
/// gitlab.com says stands in its way.
fn open_reason(item: &Item) -> &'static str {
    match item.detailed_merge_status.as_deref() {
        _ if item.has_conflicts => "conflict",
        Some("conflict") => "conflict",
        // Nothing gitlab.com names is in its way, yet it did not merge.
        Some("mergeable") => "failed",
        // Still being worked out on gitlab.com's side: not a verdict.
        None | Some("checking" | "unchecked" | "preparing" | "approvals_syncing") => "failed",
        Some(_) => "not_mergeable",
    }
}

/// gitlab.com's merge: the merge request's own merge, now, only at `head`,
/// its requests taken from `account` ([`budget::ACCOUNT`] outside tests).
fn merge(
    node: &dyn NodeLink,
    place: &Place,
    number: u32,
    method: Option<PrMergeMethod>,
    head: &str,
    account: &std::sync::Mutex<budget::Budget>,
) -> Result<(), WriteFailure> {
    // Whether the merge request is to be read again now is this unit's
    // call: when its state may have moved (a moved head, a merge running or
    // landed unseen, auto-merge set), the card and the list are read again.
    let answer = |reason: &'static str, detail: String, read_again: bool| WriteFailure {
        reason,
        detail,
        read_again,
    };
    let refused = |reason: &'static str, detail: String| answer(reason, detail, false);
    // gitlab.com has no merge queue to join, and its squash and rebase are
    // the merge request's and the project's settings, not a choice at the
    // button: one asked of it (an old phone's queue join, a GitHub method)
    // is not offered.
    if method != Some(PrMergeMethod::Merge) {
        return Err(refused(
            "unsupported",
            format!("gitlab.com merges a merge request its own way, not by {method:?}"),
        ));
    }
    let repository = repository(place).map_err(|detail| refused("failed", detail))?;
    let line = merge_arguments(&repository.path, number, head)
        .ok_or_else(|| refused("failed", "the merge cannot be named".to_owned()))?;
    let room = account
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take_many(std::time::Instant::now(), Share::Opened, MERGE_REQUESTS);
    if !room {
        return Err(refused("budget_spent", Refusal::Spent.reason()));
    }
    let unread = |refusal: Refusal| match refusal {
        Refusal::Unsupported(reason) => refused("unsupported", reason),
        Refusal::Failed(failure)
            if failure.category == GithubFailureCategory::NetworkOrRateLimit =>
        {
            refused("unreachable", failure.reason)
        }
        other => refused("failed", other.reason()),
    };
    let first = read(node, repository, number, None).map_err(unread)?;
    match before(first.as_ref(), head) {
        Before::Done => return Ok(()),
        Before::Refused(reason) => {
            return Err(answer(
                reason,
                format!("before writing, gitlab.com read it as {first:?}"),
                matches!(reason, "head_moved" | UNKNOWN),
            ));
        }
        Before::Write => {}
    }
    let wrote = glab_taken(node, line);
    if let Err(Refusal::Unsupported(reason)) = &wrote {
        return Err(refused("unsupported", reason.clone()));
    }
    // The line may have left: a read that cannot answer, for whatever reason,
    // leaves the merge's outcome unknown, not failed.
    let unknown = |refusal: Refusal| answer(UNKNOWN, refusal.reason(), true);
    let mut last = None;
    for _ in 0..2 {
        let item = read(node, repository, number, None).map_err(unknown)?;
        let locked = item
            .as_ref()
            .is_some_and(|item| item.state == State::Locked);
        last = Some(item);
        if !locked {
            break;
        }
    }
    let item = last.flatten();
    after(item.as_ref(), head).map_err(|reason| {
        answer(
            reason,
            format!(
                "the merge line answered {}; gitlab.com then read it as {item:?}",
                match &wrote {
                    Ok(_) => "without an error".to_owned(),
                    Err(refusal) => refusal.reason(),
                }
            ),
            matches!(reason, "head_moved" | "auto_merge" | UNKNOWN),
        )
    })
}

impl HeadMerge for super::host::Gitlab {
    fn merge(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
        method: Option<PrMergeMethod>,
        head: &str,
    ) -> Result<(), WriteFailure> {
        merge(node, place, number, method, head, &budget::ACCOUNT)
    }
}

impl MergeProof for super::host::Gitlab {
    /// The commit it merged as: its merge commit, else its squash commit (a
    /// fast-forward merge of a squash), else its head (a fast-forward).
    fn merge_commit(
        &self,
        node: &dyn NodeLink,
        repository: &RepositoryRef,
        number: u32,
    ) -> Result<Option<String>, String> {
        let item = read(node, repository, number, Some(Share::Background))
            .map_err(|refusal| refusal.reason())?
            .ok_or_else(|| format!("{} has no merge request {number}", repository.path))?;
        Ok((item.state == State::Merged).then(|| {
            item.merge_commit_sha
                .or(item.squash_commit_sha)
                .unwrap_or(item.sha)
        }))
    }

    /// `commit` is in `head`'s history exactly when it is their merge base.
    fn contains_commit(
        &self,
        node: &dyn NodeLink,
        repository: &RepositoryRef,
        commit: &str,
        head: &str,
    ) -> Result<bool, String> {
        #[derive(Deserialize)]
        struct Base {
            id: String,
        }
        let arguments = merge_base_arguments(&repository.path, commit, head)
            .ok_or("the commits cannot be named")?;
        let output =
            glab(node, arguments, Share::Background).map_err(|refusal| refusal.reason())?;
        let base: Base = serde_json::from_str(&output)
            .map_err(|error| format!("glab returned a merge base Hide could not read: {error}"))?;
        Ok(base.id.eq_ignore_ascii_case(commit))
    }

    /// The merged merge requests of `branch`, newest first: at most
    /// [`MERGED_LIMIT`], and a full page says it may have been cut, so an
    /// older merge past it is not known, never disproved.
    fn merged_heads(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        branch: &str,
    ) -> Result<MergedHeads, String> {
        let repository = repository(place)?;
        let arguments = merged_arguments(&repository.path, branch)
            .ok_or_else(|| format!("the branch {branch:?} cannot be named to gitlab.com"))?;
        let output = glab(node, arguments, Share::Background).map_err(|refusal| {
            format!(
                "gitlab.com merge proof is unavailable: {}",
                refusal.reason()
            )
        })?;
        let items = parse_items(&output)?;
        // A full page may have cut an older merge of this branch: the
        // answer says so, and what it holds still proves.
        let cut = items.len() >= MERGED_LIMIT;
        if cut {
            crate::diagnostic!(serde_json::json!({
                "component": "gitlab", "kind": "merge_proof.cut",
                "limit": MERGED_LIMIT,
            }));
        }
        let heads = items
            .into_iter()
            .filter(|item| item.state == State::Merged)
            .map(|item| MergedHead {
                head_oid: item.sha,
                base_ref: item.target_branch,
            })
            .collect();
        Ok(MergedHeads { heads, cut })
    }
}

impl crate::code_host::ByNumber for super::host::Gitlab {
    /// Read on the core's own clock (a mission's waiting node): a background
    /// read of the account's budget.
    fn standing(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
    ) -> Result<Standing, String> {
        let repository = repository(place)?;
        let item = read(node, repository, number, Some(Share::Background))
            .map_err(|refusal| refusal.reason())?
            .ok_or_else(|| format!("{} has no merge request {number}", repository.path))?;
        Ok(match item.state {
            State::Opened | State::Locked => Standing::Open,
            State::Closed => Standing::Closed,
            State::Merged => Standing::Merged,
        })
    }

    /// Read on the core's own clock (a CI wait on a merge request the list
    /// no longer holds): a background read of the account's budget.
    fn as_listed(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
    ) -> Result<crate::model::PullRequestSnapshot, String> {
        let repository = repository(place)?;
        super::listed_one(node, repository, number)
            .map_err(|refusal| refusal.reason())?
            .ok_or_else(|| format!("{} has no merge request {number}", repository.path))
    }

    /// Read when an agent names a merge request (`--pr`): an opened read.
    fn head(
        &self,
        node: &dyn NodeLink,
        place: &Place,
        number: u32,
    ) -> Result<ChangeHead, ChangeHeadFailure> {
        let repository = repository(place).map_err(ChangeHeadFailure::Unread)?;
        let item = read(node, repository, number, Some(Share::Opened))
            .map_err(|refusal| ChangeHeadFailure::Unread(refusal.reason()))?
            .ok_or(ChangeHeadFailure::Missing)?;
        Ok(ChangeHead {
            branch: item.source_branch,
            cross_repository: item.source_project_id != item.target_project_id,
            repository: repository.clone(),
        })
    }
}

#[cfg(test)]
#[path = "merge_request_tests.rs"]
mod tests;
