//! gitlab.com's merge, merge proof and read by number against real
//! gitlab.com answers.
//!
//! The fixtures under `hide-core/tests/fixtures/gitlab/` named
//! `merge-request-*`, `merged-of-branch`, `merge-base-*` and
//! `merge-requests-stacks` were recorded on the private test project on
//! 2026-11-23 with the account name replaced: a merge request open at its head
//! with a pipeline running under "pipelines must succeed" (gitlab.com refused
//! the merge with 405), one merged with the merge request's own squash
//! setting (a squash commit and a merge commit), one merged while its
//! pipeline ran, one where `glab mr merge` without `--auto-merge=false` set
//! auto-merge and exited 0, a branch's merged merge requests, and merge
//! bases. What each outcome must be is written here from what happened on
//! gitlab.com, never read back from the implementation.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Mutex;

use super::*;
use crate::code_host::{ByNumber, CodeHost, CodeHosts, HeadMerge, MergeProof, MergedHeads};
use crate::gitlab::host;
use hide_node_link::glab::GlabAnswer;
use hide_node_link::{ErrorCode, HostError, LinkError};

const OPEN: &str = include_str!("../../tests/fixtures/gitlab/merge-request-open.json");
const MERGED_SQUASH: &str =
    include_str!("../../tests/fixtures/gitlab/merge-request-merged-squash.json");
const MERGED: &str = include_str!("../../tests/fixtures/gitlab/merge-request-merged.json");
const AUTO_MERGE: &str = include_str!("../../tests/fixtures/gitlab/merge-request-auto-merge.json");
const MERGED_OF_BRANCH: &str = include_str!("../../tests/fixtures/gitlab/merged-of-branch.json");
const BASE_CONTAINED: &str = include_str!("../../tests/fixtures/gitlab/merge-base-contained.json");
const BASE_NOT_CONTAINED: &str =
    include_str!("../../tests/fixtures/gitlab/merge-base-not-contained.json");
const LISTED_FAILED: &str =
    include_str!("../../tests/fixtures/gitlab/merge-request-listed-failed.json");
const LISTED_MERGED: &str =
    include_str!("../../tests/fixtures/gitlab/merge-request-listed-merged.json");
const LISTED_MISSING: &str =
    include_str!("../../tests/fixtures/gitlab/merge-request-listed-missing.json");

type Answer = Result<crate::node_access::LinkAnswer, LinkError>;

fn output(stdout: &str) -> Answer {
    Ok(serde_json::to_value(GlabAnswer::Output {
        stdout: stdout.to_owned(),
        cut: false,
    })
    .unwrap()
    .into())
}

/// What `glab` answers on a failed command: its categories are the node's
/// classifier's, which names an unknown HTTP status a network failure.
fn glab_failed(reason: &str) -> Answer {
    Ok(serde_json::to_value(GlabAnswer::Failed {
        category: GithubFailureCategory::NetworkOrRateLimit,
        reason: reason.to_owned(),
    })
    .unwrap()
    .into())
}

fn refused(code: ErrorCode) -> Answer {
    Err(LinkError::Refused(HostError::new(code, "not here")))
}

/// A node that answers each read and each merge line from its queue, in
/// order, and keeps every line it was asked.
#[derive(Default)]
struct Scripted {
    reads: RefCell<VecDeque<Answer>>,
    writes: RefCell<VecDeque<Answer>>,
    asked: RefCell<Vec<Vec<String>>>,
}

// The tests run one call at a time on one thread.
unsafe impl Sync for Scripted {}

impl Scripted {
    fn new(reads: Vec<Answer>, writes: Vec<Answer>) -> Self {
        Self {
            reads: RefCell::new(reads.into()),
            writes: RefCell::new(writes.into()),
            asked: RefCell::default(),
        }
    }

    fn merges_sent(&self) -> usize {
        self.asked
            .borrow()
            .iter()
            .filter(|line| line.first().map(String::as_str) == Some("mr"))
            .count()
    }
}

impl crate::node_access::NodeLink for Scripted {
    fn call(&self, call: hide_node_link::protocol::Call, _timeout: std::time::Duration) -> Answer {
        let hide_node_link::protocol::Call::Glab { args } = call else {
            panic!("only glab is asked: {call:?}");
        };
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        assert!(
            hide_node_link::glab::allowed(&words),
            "every line the core sends is one the node allows: {words:?}"
        );
        let queue = if args[0] == "mr" {
            &self.writes
        } else {
            &self.reads
        };
        self.asked.borrow_mut().push(args);
        queue
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("nothing more was to be asked"))
    }
}

fn place() -> Place {
    let host = CodeHosts::STANDARD.host("gitlab").unwrap();
    Place::Repository {
        host,
        repository: host
            .repository("gitlab.com/example-user/hide-code-host-check")
            .unwrap(),
    }
}

/// The recorded merge request `fixture` with `edit` applied, as a list
/// answer.
fn edited(fixture: &str, edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut value: serde_json::Value = serde_json::from_str(fixture).unwrap();
    edit(&mut value[0]);
    value.to_string()
}

fn head_of(fixture: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(fixture).unwrap();
    value[0]["sha"].as_str().unwrap().to_owned()
}

/// The squash scene as it stood before its merge: open at its head, nothing
/// in its way.
fn open_squash_scene() -> String {
    edited(MERGED_SQUASH, |item| {
        item["state"] = "opened".into();
        item["merge_commit_sha"] = serde_json::Value::Null;
        item["squash_commit_sha"] = serde_json::Value::Null;
        item["detailed_merge_status"] = "mergeable".into();
    })
}

fn merge_with(
    node: &Scripted,
    method: Option<PrMergeMethod>,
    head: &str,
) -> Result<(), WriteFailure> {
    let budget = Mutex::new(budget::Budget::new());
    merge(node, &place(), 21, method, head, &budget)
}

fn reason(result: Result<(), WriteFailure>) -> &'static str {
    result.expect_err("the merge was refused").reason
}

/// gitlab.com merged it at the head shown, and the read after says so.
#[test]
fn a_merge_at_the_head_shown_is_merged_when_the_read_after_says_merged() {
    let head = head_of(MERGED_SQUASH);
    let node = Scripted::new(
        vec![output(&open_squash_scene()), output(MERGED_SQUASH)],
        vec![output("✓ Merged!\n")],
    );
    assert_eq!(merge_with(&node, Some(PrMergeMethod::Merge), &head), Ok(()));
    let asked = node.asked.borrow();
    assert_eq!(asked.len(), 3, "read, merge, read: {asked:?}");
    assert_eq!(
        asked[1],
        merge_arguments("example-user/hide-code-host-check", 21, &head).unwrap()
    );
}

/// The head moved between the read and the write: gitlab.com answers 409
/// and exit 1, and the read after names another head.
#[test]
fn a_head_that_moved_under_the_write_is_head_moved() {
    let head = head_of(MERGED_SQUASH);
    let moved = edited(&open_squash_scene(), |item| {
        item["sha"] = "f".repeat(40).into();
    });
    let node = Scripted::new(
        vec![output(&open_squash_scene()), output(&moved)],
        vec![glab_failed(
            "PUT .../merge_requests/21/merge: 409 {message: SHA does not match HEAD of source branch}",
        )],
    );
    assert_eq!(
        reason(merge_with(&node, Some(PrMergeMethod::Merge), &head)),
        "head_moved"
    );
}

/// A head that is not the one shown, open or merged, is refused before
/// anything is written.
#[test]
fn a_head_other_than_the_one_shown_is_refused_with_nothing_written() {
    let shown = "a".repeat(40);
    for answer in [OPEN, MERGED] {
        let node = Scripted::new(vec![output(answer)], vec![]);
        assert_eq!(
            reason(merge_with(&node, Some(PrMergeMethod::Merge), &shown)),
            "head_moved"
        );
        assert_eq!(node.merges_sent(), 0);
    }
}

/// `glab` exits 0 having only set auto-merge: the read after says open, with
/// auto-merge set, and nothing merged.
#[test]
fn an_exit_zero_that_only_set_auto_merge_is_auto_merge_not_success() {
    let head = head_of(AUTO_MERGE);
    let before = edited(AUTO_MERGE, |item| {
        item["merge_when_pipeline_succeeds"] = false.into();
    });
    let node = Scripted::new(
        vec![output(&before), output(AUTO_MERGE)],
        vec![output("! Pipeline status: running\n✓ Will auto-merge\n")],
    );
    assert_eq!(
        reason(merge_with(&node, Some(PrMergeMethod::Merge), &head)),
        "auto_merge"
    );
}

/// "Pipelines must succeed" and a running pipeline: gitlab.com answers 405,
/// and the read after says the checks are still running.
#[test]
fn a_merge_gitlab_com_will_not_take_yet_is_not_mergeable() {
    let head = head_of(OPEN);
    let node = Scripted::new(
        vec![output(OPEN), output(OPEN)],
        vec![glab_failed(
            "PUT .../merge_requests/23/merge: 405 {message: 405 Method Not Allowed}",
        )],
    );
    assert_eq!(
        reason(merge_with(&node, Some(PrMergeMethod::Merge), &head)),
        "not_mergeable"
    );
}

/// Asking again after it merged at the head shown is a success with no
/// second write; merged at another head it is not.
#[test]
fn asking_again_after_the_merge_converges_without_a_second_write() {
    let node = Scripted::new(vec![output(MERGED_SQUASH)], vec![]);
    assert_eq!(
        merge_with(&node, Some(PrMergeMethod::Merge), &head_of(MERGED_SQUASH)),
        Ok(())
    );
    assert_eq!(node.merges_sent(), 0);
}

/// The write left and its answer was lost with the read after: unreachable,
/// since it may have merged. Asked again, the read before finds it merged at
/// the head: a success, and one write in all.
#[test]
fn a_lost_read_after_a_write_is_unreachable_and_asking_again_succeeds_with_one_write() {
    let head = head_of(MERGED_SQUASH);
    let node = Scripted::new(
        vec![
            output(&open_squash_scene()),
            Err(LinkError::Unknown("the node went away".to_owned())),
            output(MERGED_SQUASH),
        ],
        vec![output("✓ Merged!\n")],
    );
    assert_eq!(
        reason(merge_with(&node, Some(PrMergeMethod::Merge), &head)),
        "unreachable"
    );
    assert_eq!(merge_with(&node, Some(PrMergeMethod::Merge), &head), Ok(()));
    assert_eq!(node.merges_sent(), 1);
}

/// Once the merge line may have left, a read after it that cannot say what
/// happened, for any reason, leaves the outcome not known (`unreachable`,
/// read again at once), never `failed`; and asking again never writes over
/// a merge whose outcome is not known to be "did not happen": still locked,
/// it writes nothing; merged at the head, it succeeds. One write in all.
#[test]
fn an_answer_after_the_write_that_settles_nothing_is_not_known_and_asking_again_writes_nothing() {
    let head = head_of(MERGED_SQUASH);
    let locked = edited(MERGED_SQUASH, |item| item["state"] = "locked".into());
    let too_large: Answer = Ok(serde_json::to_value(GlabAnswer::Output {
        stdout: "[".to_owned(),
        cut: true,
    })
    .unwrap()
    .into());
    let signed_out: Answer = Ok(serde_json::to_value(GlabAnswer::Failed {
        category: GithubFailureCategory::NotLoggedIn,
        reason: "401 Unauthorized".to_owned(),
    })
    .unwrap()
    .into());
    let scenes: Vec<(&str, Vec<Answer>)> = vec![
        ("unreadable", vec![output("<html>a proxy page</html>")]),
        ("too large", vec![too_large]),
        ("sign-in lost", vec![signed_out]),
        ("no such merge request", vec![output("[]")]),
        ("still locked", vec![output(&locked), output(&locked)]),
    ];
    for (scene, after) in scenes {
        let mut reads = vec![output(&open_squash_scene())];
        reads.extend(after);
        // Asked again while gitlab.com still merges it: nothing is written.
        reads.push(output(&locked));
        // Asked again once it landed: a success with nothing written.
        reads.push(output(MERGED_SQUASH));
        let node = Scripted::new(reads, vec![output("✓ Merged!\n")]);
        let first = merge_with(&node, Some(PrMergeMethod::Merge), &head).unwrap_err();
        assert_eq!(
            (first.reason, first.read_again),
            ("unreachable", true),
            "{scene}"
        );
        let running = merge_with(&node, Some(PrMergeMethod::Merge), &head).unwrap_err();
        assert_eq!(running.reason, "unreachable", "{scene}");
        assert_eq!(
            merge_with(&node, Some(PrMergeMethod::Merge), &head),
            Ok(()),
            "{scene}"
        );
        assert_eq!(node.merges_sent(), 1, "{scene}");
    }
}

/// The unit says when the card and the list are read again: after a write
/// whose answer may mean the merge request moved (a moved head, auto-merge
/// set, an outcome not known), and not after a refusal that changed nothing
/// or a read that failed before anything was written.
#[test]
fn the_unit_asks_to_read_again_only_where_the_merge_request_may_have_moved() {
    let head = head_of(MERGED_SQUASH);
    let failure = |reads: Vec<Answer>, writes: Vec<Answer>| {
        let node = Scripted::new(reads, writes);
        let failure = merge_with(&node, Some(PrMergeMethod::Merge), &head).unwrap_err();
        (failure.reason, failure.read_again)
    };
    let moved = edited(&open_squash_scene(), |item| {
        item["sha"] = "b".repeat(40).into()
    });
    assert_eq!(failure(vec![output(&moved)], vec![]), ("head_moved", true));
    let auto = edited(&open_squash_scene(), |item| {
        item["merge_when_pipeline_succeeds"] = true.into();
    });
    assert_eq!(
        failure(
            vec![output(&open_squash_scene()), output(&auto)],
            vec![output("")]
        ),
        ("auto_merge", true)
    );
    let closed = edited(&open_squash_scene(), |item| item["state"] = "closed".into());
    assert_eq!(
        failure(vec![output(&closed)], vec![]),
        ("not_mergeable", false)
    );
    assert_eq!(
        failure(vec![glab_failed("connection reset")], vec![]),
        ("unreachable", false)
    );
}

/// An old phone bundle sends GitHub's shapes: a queue join (no method) or a
/// squash or rebase. gitlab.com offers none of them, so the core refuses
/// with nothing sent; the merge request's own merge is the one it takes.
#[test]
fn an_old_phones_queue_join_or_github_method_is_refused_with_nothing_sent() {
    for method in [
        None,
        Some(PrMergeMethod::Squash),
        Some(PrMergeMethod::Rebase),
    ] {
        let node = Scripted::new(vec![], vec![]);
        assert_eq!(
            reason(merge_with(&node, method, &head_of(OPEN))),
            "unsupported",
            "{method:?}"
        );
        assert!(node.asked.borrow().is_empty());
    }
}

/// A head that is not a full commit name never becomes a line.
#[test]
fn a_head_that_is_no_full_commit_is_refused_with_nothing_sent() {
    for head in [
        "abc",
        "main",
        &"g".repeat(40),
        &format!("--{}", "a".repeat(38)),
    ] {
        let node = Scripted::new(vec![], vec![]);
        assert_eq!(
            reason(merge_with(&node, Some(PrMergeMethod::Merge), head)),
            "failed",
            "{head}"
        );
        assert!(node.asked.borrow().is_empty());
    }
}

/// A node that does not run the read, or runs it and not the merge line,
/// refuses with a typed answer: not offered there, never a failure to retry
/// into a merge without its condition.
#[test]
fn a_node_that_does_not_offer_the_lines_is_unsupported_and_writes_nothing() {
    let node = Scripted::new(vec![refused(ErrorCode::Unsupported)], vec![]);
    assert_eq!(
        reason(merge_with(
            &node,
            Some(PrMergeMethod::Merge),
            &head_of(OPEN)
        )),
        "unsupported"
    );
    assert_eq!(node.merges_sent(), 0);

    let node = Scripted::new(vec![output(OPEN)], vec![refused(ErrorCode::Unsupported)]);
    assert_eq!(
        reason(merge_with(
            &node,
            Some(PrMergeMethod::Merge),
            &head_of(OPEN)
        )),
        "unsupported"
    );
    assert_eq!(node.asked.borrow().len(), 2, "no read after a refused line");
}

/// The merge takes all its requests at once from the operator's share, or
/// none, and then nothing leaves.
#[test]
fn a_merge_without_room_for_its_reads_is_budget_spent_with_nothing_sent() {
    let budget = Mutex::new(budget::Budget::new());
    {
        let mut taken = budget.lock().unwrap();
        while taken.take(std::time::Instant::now(), budget::Share::Opened) {}
    }
    let node = Scripted::new(vec![], vec![]);
    let refused = merge(
        &node,
        &place(),
        21,
        Some(PrMergeMethod::Merge),
        &head_of(OPEN),
        &budget,
    )
    .unwrap_err();
    assert_eq!(refused.reason, "budget_spent");
    assert!(node.asked.borrow().is_empty());
}

/// The unit's own merge, through its declaration, goes the same way.
#[test]
fn gitlab_com_declares_the_merge_its_proof_and_its_read_by_number() {
    let declaration = crate::code_host::declaration(&host::GITLAB);
    for operation in [
        crate::code_host::Operation::Merge,
        crate::code_host::Operation::MergeProof,
        crate::code_host::Operation::ByNumber,
        crate::code_host::Operation::CommitChecks,
    ] {
        assert!(declaration.operations.contains(&operation), "{operation:?}");
    }
    assert!(
        !declaration
            .operations
            .contains(&crate::code_host::Operation::UpdateBranch)
    );
    let node = Scripted::new(vec![output(MERGED_SQUASH)], vec![]);
    assert_eq!(
        HeadMerge::merge(
            CodeHost::merge(&host::GITLAB).unwrap(),
            &node,
            &place(),
            21,
            Some(PrMergeMethod::Merge),
            &head_of(MERGED_SQUASH)
        ),
        Ok(())
    );
}

/// A squash merge's commit is its merge commit, which holds the squash; a
/// fast-forward of a squash would be the squash commit; one not merged has
/// none.
#[test]
fn the_merge_commit_is_the_merge_then_the_squash_then_the_head() {
    let unit = &host::GITLAB;
    let repository = place().repository().unwrap().clone();
    let commit = |answer: &str| {
        let node = Scripted::new(vec![output(answer)], vec![]);
        unit.merge_commit(&node, &repository, 21).unwrap()
    };
    let recorded: serde_json::Value = serde_json::from_str(MERGED_SQUASH).unwrap();
    assert_eq!(
        commit(MERGED_SQUASH).as_deref(),
        recorded[0]["merge_commit_sha"].as_str()
    );
    let fast_forward_squash = edited(MERGED_SQUASH, |item| {
        item["merge_commit_sha"] = serde_json::Value::Null;
    });
    assert_eq!(
        commit(&fast_forward_squash).as_deref(),
        recorded[0]["squash_commit_sha"].as_str()
    );
    let fast_forward = edited(MERGED, |item| {
        item["merge_commit_sha"] = serde_json::Value::Null;
    });
    assert_eq!(commit(&fast_forward), Some(head_of(MERGED)));
    assert_eq!(commit(OPEN), None);
    let node = Scripted::new(vec![output("[]")], vec![]);
    assert!(unit.merge_commit(&node, &repository, 99).is_err());
}

/// `commit` is in `head` when gitlab.com names it their merge base: the
/// squash commit in its merge commit, and not the merge commit in the
/// branch's head.
#[test]
fn a_commit_is_contained_exactly_when_it_is_the_merge_base() {
    let unit = &host::GITLAB;
    let repository = place().repository().unwrap().clone();
    let recorded: serde_json::Value = serde_json::from_str(MERGED_SQUASH).unwrap();
    let squash = recorded[0]["squash_commit_sha"].as_str().unwrap();
    let merge_commit = recorded[0]["merge_commit_sha"].as_str().unwrap();
    let head = recorded[0]["sha"].as_str().unwrap();
    let node = Scripted::new(vec![output(BASE_CONTAINED)], vec![]);
    assert!(
        unit.contains_commit(&node, &repository, squash, merge_commit)
            .unwrap()
    );
    let node = Scripted::new(vec![output(BASE_NOT_CONTAINED)], vec![]);
    assert!(
        !unit
            .contains_commit(&node, &repository, merge_commit, head)
            .unwrap()
    );
}

/// A branch's merged merge requests prove its squash merge by their head and
/// target, as GitHub's merged pull requests do.
#[test]
fn a_branchs_merged_merge_requests_prove_its_heads_and_their_targets() {
    let node = Scripted::new(vec![output(MERGED_OF_BRANCH)], vec![]);
    let heads = host::GITLAB
        .merged_heads(&node, &place(), "w-squash")
        .unwrap();
    assert_eq!(
        heads,
        MergedHeads {
            heads: vec![MergedHead {
                head_oid: head_of(MERGED_SQUASH),
                base_ref: "main".to_owned(),
            }],
            cut: false,
        }
    );
    assert!(host::GITLAB.merged_heads(&node, &place(), "-x").is_err());
}

/// A merge request the list no longer holds, read by its number with what the
/// list reads an open one with, is judged as the list judges it (PRD
/// core-wait D-05): the open one's head pipeline failed on its `fail` job,
/// the merged one stands merged; a number gitlab.com does not have, or a
/// node that refuses the line, is no read, which a CI wait shows as unread.
/// Recorded on the private test project on 2026-11-23 (merge requests 2, 22
/// and 999).
#[test]
fn a_merge_request_read_by_its_number_is_judged_as_the_list_judges_it() {
    use crate::model::{PullRequestBadge, PullRequestChecks};
    let unit = &host::GITLAB;
    let read = |answer: Answer, number: u32| {
        let node = Scripted::new(vec![answer], vec![]);
        let read = unit.as_listed(&node, &place(), number);
        (read, node.asked.borrow().clone())
    };
    let (failed, asked) = read(output(LISTED_FAILED), 2);
    let failed = failed.unwrap();
    assert_eq!(asked.len(), 1);
    assert!(
        asked[0].contains(&"graphql".to_owned()) && asked[0].contains(&"iid=2".to_owned()),
        "{asked:?}"
    );
    assert_eq!(
        (failed.number, failed.checks, failed.badge),
        (2, PullRequestChecks::Failed, PullRequestBadge::Open)
    );
    let names: Vec<_> = failed
        .work
        .failed_checks
        .iter()
        .filter_map(|check| check.name.as_deref())
        .collect();
    assert_eq!(names, ["fail"]);

    let merged = read(output(LISTED_MERGED), 22).0.unwrap();
    assert_eq!(
        (merged.number, merged.badge),
        (22, PullRequestBadge::Merged)
    );

    assert!(read(output(LISTED_MISSING), 999).0.is_err());
    assert!(read(refused(ErrorCode::Unsupported), 2).0.is_err());
}

/// A merge request read by its number stands open, merged or closed, and
/// names its branch; a number gitlab.com does not have is missing, never an
/// unread answer.
#[test]
fn a_merge_request_by_its_number_stands_and_names_its_branch() {
    let unit = &host::GITLAB;
    let standing = |answer: &str| {
        let node = Scripted::new(vec![output(answer)], vec![]);
        unit.standing(&node, &place(), 21).unwrap()
    };
    assert_eq!(standing(OPEN), Standing::Open);
    assert_eq!(standing(MERGED), Standing::Merged);
    assert_eq!(
        standing(&edited(OPEN, |item| item["state"] = "closed".into())),
        Standing::Closed
    );
    assert_eq!(
        standing(&edited(OPEN, |item| item["state"] = "locked".into())),
        Standing::Open
    );

    let node = Scripted::new(vec![output(OPEN)], vec![]);
    let head = unit.head(&node, &place(), 23).unwrap();
    assert_eq!(head.branch, "w-slow-must");
    assert!(!head.cross_repository);
    let fork = edited(OPEN, |item| item["source_project_id"] = 1.into());
    let node = Scripted::new(vec![output(&fork)], vec![]);
    assert!(unit.head(&node, &place(), 23).unwrap().cross_repository);
    let node = Scripted::new(vec![output("[]")], vec![]);
    assert_eq!(
        unit.head(&node, &place(), 999),
        Err(ChangeHeadFailure::Missing)
    );
    let node = Scripted::new(vec![output("{\"message\":\"x\"}")], vec![]);
    assert!(matches!(
        unit.head(&node, &place(), 23),
        Err(ChangeHeadFailure::Unread(_))
    ));
}
