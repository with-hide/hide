//! gitlab.com's unit against real gitlab.com answers.
//!
//! The fixtures under `hide-core/tests/fixtures/gitlab/` are answers of the
//! one list query, recorded from the private test project's scenes and from
//! the public `gitlab-org/cli` project (forks, change requests, required
//! approvals), with every account name replaced. The expected verdict, mark
//! and list place of each scene are written here from what the scene is,
//! following `docs/status-model.md` (Code hosts), never read back from the
//! implementation.

use std::collections::HashMap;

use super::*;
use crate::agent_state::work::board;
use crate::code_host::CodeHost;
use crate::model::PrState;

const SCENES: &str = include_str!("../../tests/fixtures/gitlab/merge-requests-scenes.json");
const RUNNING: &str = include_str!("../../tests/fixtures/gitlab/merge-requests-running.json");
const FORKS: &str = include_str!("../../tests/fixtures/gitlab/merge-requests-forks.json");
const CARD: &str = include_str!("../../tests/fixtures/gitlab/card.json");
const COMMITS: &str = include_str!("../../tests/fixtures/gitlab/commit-checks.json");

fn listed(fixture: &str) -> Listed {
    parse_list(fixture, &KnownChecks::default()).expect("a recorded answer reads")
}

fn pull(listed: &Listed, number: u32) -> &PullRequestSnapshot {
    listed
        .pull_requests
        .iter()
        .find(|pull| pull.number == number)
        .unwrap_or_else(|| panic!("!{number} is listed"))
}

const NOW: u64 = 1_795_400_000_000;

/// Where the PR list places each pull request (`board::project`), or `None`
/// when the list leaves it out.
fn places(pulls: &[PullRequestSnapshot]) -> HashMap<u32, (&'static str, &'static str)> {
    let mut workspace = crate::workspace::inspect_temporary(
        "/fixture/app",
        "device",
        &crate::workspace::PathIndex::default(),
    );
    workspace.pull_requests = pulls.to_vec();
    let board = board::project(&workspace, &[], &HashMap::new(), NOW);
    board
        .rows
        .iter()
        .map(|row| (row.number, (row.group, row.reason.kind)))
        .collect()
}

/// One scene of the test project and what it must read as.
struct Scene {
    number: u32,
    what: &'static str,
    checks: PullRequestChecks,
    state: PrState,
    merge: MergeState,
    /// The list's group and reason; `None` when the list leaves it out.
    place: Option<(&'static str, &'static str)>,
}

#[test]
fn every_scene_of_the_test_project_reads_as_its_scene() {
    use MergeState::*;
    use PullRequestChecks as C;
    let scenes = [
        // Passed and approved by its author, with no approval required.
        Scene { number: 1, what: "pass", checks: C::Passing, state: PrState::Mergeable, merge: Clean, place: Some(("needs_you", "merge")) },
        // Failed, in a project that does not require the pipeline to pass:
        // gitlab.com says MERGEABLE, which is not a pass.
        Scene { number: 2, what: "fail", checks: C::Failed, state: PrState::Failed, merge: Unstable, place: Some(("needs_you", "merge")) },
        // A job allowed to fail: the pipeline passed with warnings.
        Scene { number: 3, what: "allowed failure", checks: C::Passing, state: PrState::Mergeable, merge: Clean, place: Some(("needs_you", "merge")) },
        // A manual job the pipeline waits on: nothing runs, nothing passed.
        Scene { number: 4, what: "manual", checks: C::Pending, state: PrState::Pending, merge: Unstable, place: Some(("needs_you", "merge")) },
        // Cancelled, in a project that does not require the pipeline to pass.
        Scene { number: 5, what: "cancelled", checks: C::Failed, state: PrState::Failed, merge: Unstable, place: Some(("needs_you", "merge")) },
        // No pipeline at all.
        Scene { number: 6, what: "no pipeline", checks: C::None, state: PrState::Pending, merge: Clean, place: Some(("needs_you", "merge")) },
        // `[skip ci]`: a pipeline that ran nothing.
        Scene { number: 7, what: "skipped", checks: C::None, state: PrState::Pending, merge: Clean, place: Some(("needs_you", "merge")) },
        // Closed: never listed.
        Scene { number: 8, what: "closed", checks: C::Unknown, state: PrState::Closed, merge: Unknown, place: None },
        // Merged.
        Scene { number: 9, what: "merged", checks: C::Unknown, state: PrState::Merged, merge: Unknown, place: Some(("merged", "merged")) },
        // A draft, whatever its pipeline says.
        Scene { number: 10, what: "draft", checks: C::Passing, state: PrState::Draft, merge: Draft, place: Some(("drafts", "draft")) },
        // Passed, with no approval and none required.
        Scene { number: 11, what: "zero approvals required", checks: C::Passing, state: PrState::Mergeable, merge: Clean, place: Some(("needs_you", "merge")) },
    ];
    let listed = listed(SCENES);
    let placed = places(&listed.pull_requests);
    for scene in &scenes {
        let pull = pull(&listed, scene.number);
        let read = (pull.checks, pull.state(), pull.work.merge_state, placed.get(&scene.number).copied());
        assert_eq!(
            read,
            (scene.checks, scene.state, scene.merge, scene.place),
            "!{} ({})",
            scene.number,
            scene.what
        );
    }
}

/// A merge request with no approval required and nobody approving has no
/// review decision: REST's `approvals.approved` would say true here, and
/// GraphQL's `approved` is false until someone approves.
#[test]
fn zero_approvals_required_is_no_review_until_someone_approves() {
    let listed = listed(SCENES);
    assert_eq!(pull(&listed, 11).review, None);
    assert_eq!(pull(&listed, 11).work.required_approvals, None);
    assert_eq!(pull(&listed, 1).review, Some(ReviewDecision::Approved));
}

#[test]
fn a_running_pipeline_is_pending_and_holds_the_head_unfinished() {
    let listed = listed(RUNNING);
    let running = pull(&listed, 5);
    assert_eq!(running.checks, PullRequestChecks::Pending);
    assert_eq!(running.state(), PrState::Pending);
    assert!(running.checks_unfinished());
    assert_eq!(running.work.running_checks, 1);
    assert_eq!(running.work.pending_checks, 1);
    assert_eq!(running.ci_end(), crate::model::CiEnd::Running);
}

#[test]
fn the_cancelled_scene_ends_as_cancelled_and_the_failed_one_names_its_job() {
    let listed = listed(SCENES);
    let cancelled = pull(&listed, 5);
    assert_eq!(
        cancelled.ci_end(),
        crate::model::CiEnd::Ended(crate::missions::CiResult::Cancelled)
    );
    let failed = pull(&listed, 2);
    assert_eq!(failed.ci_end(), crate::model::CiEnd::Ended(crate::missions::CiResult::Failed));
    let names: Vec<_> = failed
        .work
        .failed_checks
        .iter()
        .map(|check| check.name.as_deref())
        .collect();
    assert_eq!(names, [Some("fail")]);
    assert!(
        failed.work.failed_checks[0]
            .url
            .as_deref()
            .is_some_and(|url| url.starts_with("https://gitlab.com/example-user/hide-code-host-check/-/jobs/"))
    );
    // The allowed failure is no failure of the pipeline.
    assert!(pull(&listed, 3).work.failed_checks.is_empty());
    let counts = pull(&listed, 3).work.check_counts.clone().unwrap();
    assert_eq!((counts.total, counts.passed, counts.failed), (2, 2, 0));
}

#[test]
fn the_merge_queue_is_unsupported_not_unknown_on_every_merge_request() {
    for fixture in [SCENES, RUNNING, FORKS] {
        for pull in &listed(fixture).pull_requests {
            assert_eq!(pull.work.queue, MergeQueuePlace::Unsupported, "!{}", pull.number);
        }
    }
}

#[test]
fn the_signed_in_account_decides_mine_and_review_asks() {
    let scenes = listed(SCENES);
    assert_eq!(scenes.viewer, "example-user");
    assert!(scenes.pull_requests.iter().all(|pull| pull.work.mine == Some(true)));
    let forks = listed(FORKS);
    for pull in forks.pull_requests.iter().filter(|pull| !pull.badge.is_settled()) {
        assert_eq!(pull.work.mine, Some(false), "!{}", pull.number);
        assert_eq!(pull.work.asked_of_me, ReviewAsk::None, "!{}", pull.number);
        // Someone else's merge request that asks nothing of the account is
        // not listed.
        assert!(!board::listed(pull), "!{}", pull.number);
    }
}

/// Merge requests from forks and from the project, with change requests and
/// required approvals, from the public project.
#[test]
fn forks_change_requests_and_required_approvals_read_as_their_scenes() {
    use MergeState::*;
    let listed = listed(FORKS);
    let read = |number: u32| {
        let pull = pull(&listed, number);
        (
            pull.cross_repository,
            pull.review,
            pull.state(),
            pull.work.merge_state,
            pull.work.required_approvals,
        )
    };
    // Two reviewers asked for changes.
    assert_eq!(read(4226), (false, Some(ReviewDecision::ChangesRequested), PrState::Failed, Blocked, Some(2)));
    // From a fork; gitlab.com's merge status says changes were asked for.
    assert_eq!(read(4429), (true, Some(ReviewDecision::ChangesRequested), PrState::Failed, Blocked, Some(3)));
    // From a fork, two approvals required and none given.
    assert_eq!(read(4526), (true, Some(ReviewDecision::ReviewRequired), PrState::Pending, Blocked, Some(2)));
    // A draft whose pipeline was skipped.
    assert_eq!(read(4518), (false, None, PrState::Draft, Draft, Some(3)));
    assert_eq!(pull(&listed, 4518).checks, PullRequestChecks::None);
    // One of three required approvals given.
    assert_eq!(read(4505), (false, Some(ReviewDecision::ReviewRequired), PrState::Pending, Blocked, Some(3)));
    // An optional manual job is no check the pipeline waits on.
    assert_eq!(pull(&listed, 4226).checks, PullRequestChecks::Passing);
    assert_eq!(pull(&listed, 4226).work.pending_checks, 0);
    // Reviewers: approvals first, and the one still asked is a request.
    let reviewers = pull(&listed, 4429).work.reviewers.clone().unwrap();
    let states: Vec<_> = reviewers.iter().map(|reviewer| reviewer.state).collect();
    assert_eq!(states, [PrReviewerState::Approved, PrReviewerState::Requested]);
    let requests: Vec<_> = pull(&listed, 4429)
        .work
        .review_requests
        .iter()
        .map(|request| request.name.as_str())
        .collect();
    assert_eq!(requests, ["member-6"]);
    // The merged and closed ones are listed beside the open ones.
    assert_eq!(pull(&listed, 4489).badge, crate::model::PullRequestBadge::Merged);
    assert_eq!(pull(&listed, 4524).badge, crate::model::PullRequestBadge::Closed);
}

/// A fork's merge request of the account itself is placed by its review.
#[test]
fn a_change_request_on_my_own_merge_request_blocks_it() {
    let mut listed = listed(FORKS);
    for pull in &mut listed.pull_requests {
        pull.work.mine = Some(pull.work.author.as_deref() == Some("member-5"));
    }
    let placed = places(&listed.pull_requests);
    assert_eq!(placed.get(&4429), Some(&("blocked", "changes")));
}

#[test]
fn the_identity_and_references_come_from_the_answer_with_their_server() {
    let listed = listed(SCENES);
    let repository = listed.identity.repository.clone().expect("the server is known");
    assert_eq!(
        (repository.host.as_str(), repository.authority.as_str(), repository.path.as_str()),
        ("gitlab", "gitlab.com", "example-user/hide-code-host-check")
    );
    assert_eq!(listed.identity.id.as_deref(), Some("gid://gitlab/Project/89295207"));
    assert_eq!(listed.identity.public, Some(false));
    assert_eq!(listed.identity.default_branch.as_deref(), Some("main"));
    let reference = pull(&listed, 2).reference.clone().unwrap();
    assert_eq!(reference.label, "!2");
    assert_eq!(
        reference.checks_url,
        "https://gitlab.com/example-user/hide-code-host-check/-/merge_requests/2/pipelines"
    );
    assert_eq!(
        reference.files_url,
        "https://gitlab.com/example-user/hide-code-host-check/-/merge_requests/2/diffs"
    );
    assert_eq!(pull(&listed, 2).url, "https://gitlab.com/example-user/hide-code-host-check/-/merge_requests/2");
    // The snapshot names its parts as gitlab.com reads them.
    let snapshot = listed.into_snapshot("/fixture/app");
    assert_eq!(snapshot.issues_read, PartRead::Unsupported);
    assert_eq!(snapshot.identity_read, PartRead::Read);
    assert_eq!(snapshot.read_outcome(), crate::model::GithubReadOutcome::Read);
}

#[test]
fn an_answer_without_the_account_or_the_project_is_a_failure() {
    let signed_out = r#"{"data":{"currentUser":null,"project":null}}"#;
    let failure = parse_list(signed_out, &KnownChecks::default()).unwrap_err();
    assert_eq!(failure.category, GithubFailureCategory::NotLoggedIn);
    let missing = r#"{"data":{"currentUser":{"username":"example-user"},"project":null}}"#;
    assert!(parse_list(missing, &KnownChecks::default()).is_err());
    let complexity = r#"{"errors":[{"message":"Query has complexity of 271, which exceeds max complexity of 250"}]}"#;
    assert!(parse_list(complexity, &KnownChecks::default()).is_err());
    assert!(parse_list("not json", &KnownChecks::default()).is_err());
}

/// A job set the page cut is counted by the pipeline alone, never read as
/// all passed.
#[test]
fn a_cut_job_set_gives_no_counts_and_keeps_the_pipeline_verdict() {
    let mut value: serde_json::Value = serde_json::from_str(SCENES).unwrap();
    for node in value["data"]["project"]["open"]["nodes"].as_array_mut().unwrap() {
        if node["iid"] == "1" {
            node["headPipeline"]["jobs"]["count"] = 80.into();
        }
    }
    let listed = parse_list(&value.to_string(), &KnownChecks::default()).unwrap();
    assert_eq!(pull(&listed, 1).work.check_counts, None);
    assert_eq!(pull(&listed, 1).checks, PullRequestChecks::Passing);
}

#[test]
fn every_pipeline_status_has_a_named_verdict_and_none_but_success_passes() {
    use PullRequestChecks as C;
    let table = [
        (None, C::None),
        (Some("SUCCESS"), C::Passing),
        (Some("FAILED"), C::Failed),
        (Some("CANCELED"), C::Failed),
        (Some("CANCELING"), C::Pending),
        (Some("RUNNING"), C::Pending),
        (Some("PENDING"), C::Pending),
        (Some("CREATED"), C::Pending),
        (Some("PREPARING"), C::Pending),
        (Some("WAITING_FOR_RESOURCE"), C::Pending),
        (Some("WAITING_FOR_CALLBACK"), C::Pending),
        (Some("SCHEDULED"), C::Pending),
        (Some("MANUAL"), C::Pending),
        (Some("SKIPPED"), C::None),
        (Some("SOMETHING_NEW"), C::Unknown),
    ];
    for (status, verdict) in table {
        assert_eq!(pipeline_verdict(status), verdict, "{status:?}");
    }
}

#[test]
fn the_merge_status_never_stands_in_for_the_checks() {
    use MergeState::*;
    use PullRequestChecks as C;
    for (status, checks, merge) in [
        (Some("MERGEABLE"), C::Passing, Clean),
        (Some("MERGEABLE"), C::None, Clean),
        (Some("MERGEABLE"), C::Failed, Unstable),
        (Some("MERGEABLE"), C::Pending, Unstable),
        (Some("MERGEABLE"), C::Unknown, Unstable),
        (Some("PREPARING"), C::Passing, Unknown),
        (Some("UNCHECKED"), C::Passing, Unknown),
        (Some("CHECKING"), C::Passing, Unknown),
        (Some("CONFLICT"), C::Passing, Dirty),
        (Some("NEED_REBASE"), C::Passing, Behind),
        (Some("CI_MUST_PASS"), C::Failed, Blocked),
        (Some("CI_STILL_RUNNING"), C::Pending, Blocked),
        (Some("NOT_APPROVED"), C::Passing, Blocked),
        (Some("DISCUSSIONS_NOT_RESOLVED"), C::Passing, Blocked),
        (Some("DRAFT_STATUS"), C::Passing, Draft),
        (None, C::Passing, Unknown),
    ] {
        assert_eq!(merge_state(status, false, checks), merge, "{status:?} {checks:?}");
    }
    assert_eq!(merge_state(Some("MERGEABLE"), true, C::Passing), Draft);
}

#[test]
fn a_card_reads_its_head_files_and_open_discussions() {
    let card = parse_card(CARD).unwrap();
    assert_eq!(card.head.as_deref(), Some("1487cb8ce0c1f453e6ef90655ec45c0e98c97b7c"));
    assert_eq!(card.body.as_deref(), Some(""));
    assert_eq!(card.files_total, 2);
    let paths: Vec<_> = card.files.unwrap().into_iter().map(|file| file.path).collect();
    assert_eq!(paths, [".gitlab-ci.yml", "scenes/fail.txt"]);
    // The only discussion is a system note, which cannot be resolved.
    assert_eq!(card.threads, Some(Vec::new()));
    assert!(card.failed.is_empty());
    assert_eq!(card.merge, None);
}

#[test]
fn commit_checks_name_each_commit_and_refuse_another_commit() {
    let value: serde_json::Value = serde_json::from_str(COMMITS).unwrap();
    let asked = [
        value["data"]["project"]["c0"]["nodes"][0]["sha"].as_str().unwrap().to_owned(),
        "0000000000000000000000000000000000000000".to_owned(),
    ];
    let url = |sha: &str| format!("https://gitlab.com/x/y/-/commit/{sha}");
    let read = parse_commit_checks(COMMITS, &asked, url).unwrap();
    let states: Vec<_> = read.iter().map(|ci| ci.state).collect();
    assert_eq!(states, [CiState::Failed, CiState::None]);
    assert_eq!(read[0].url.as_deref(), Some(url(&asked[0]).as_str()));
    let other = ["f".repeat(40), asked[1].clone()];
    assert!(parse_commit_checks(COMMITS, &other, url).is_err());
}

#[test]
fn gitlab_com_names_and_addresses_round_trip_and_keep_no_credential() {
    let unit: &dyn CodeHost = &host::GITLAB;
    let repository = unit.repository("gitlab.com/group/sub/project").unwrap();
    assert_eq!(unit.stored_name(&repository), "gitlab.com/group/sub/project");
    for remote in [
        "https://gitlab.com/group/sub/project.git",
        "https://oauth2:glpat-secret@gitlab.com/group/sub/project.git",
        "https://gitlab.com:443/group/sub/project",
        "git@gitlab.com:group/sub/project.git",
        "ssh://git@gitlab.com:22/group/sub/project.git",
        "ssh://git@GitLab.com/group/sub/project/",
    ] {
        let read = unit.remote_repository(remote).unwrap_or_else(|| panic!("{remote}"));
        assert_eq!(read, repository, "{remote}");
        assert!(!format!("{read:?}").contains("secret"));
    }
    for other in [
        "https://github.com/acme/app.git",
        "https://gitlab.example.com/group/project.git",
        "git@gitlab.com:project.git",
        "/local/path/project",
    ] {
        assert_eq!(unit.remote_repository(other), None, "{other}");
    }
    assert_eq!(
        unit.change_request_at("https://gitlab.com/group/sub/project/-/merge_requests/12/diffs"),
        Some((repository.clone(), 12))
    );
    assert_eq!(unit.change_request_at("https://github.com/acme/app/pull/12"), None);
    assert_eq!(unit.number_label(12), "!12");
    assert_eq!(
        unit.commit_url(&repository, "abc"),
        "https://gitlab.com/group/sub/project/-/commit/abc"
    );
    assert_eq!(unit.repository("group/project"), None);
}

/// The registry gives a gitlab.com remote to gitlab.com's unit, never to
/// GitHub's (which would read it as an Enterprise server), and a project
/// whose remotes span both hosts to GitHub, which picks in the checkout.
#[test]
fn the_registry_places_gitlab_com_remotes_with_gitlab() {
    let hosts = crate::code_host::CodeHosts::STANDARD;
    let (host, repository) = hosts.remote("git@gitlab.com:acme/app.git").unwrap();
    assert_eq!((host.key(), repository.authority.as_str()), ("gitlab", "gitlab.com"));
    let root = std::path::Path::new("/fixture/app");
    let only_gitlab = ["https://gitlab.com/acme/sub/app.git".to_owned()];
    let place = hosts
        .place_of_checkout(root, crate::code_host::CheckoutRemotes::Addresses(&only_gitlab))
        .unwrap();
    assert_eq!(place.host().key(), "gitlab");
    assert_eq!(place.repository().map(|repository| repository.path.as_str()), Some("acme/sub/app"));
    let both = [
        "https://gitlab.com/acme/app.git".to_owned(),
        "https://github.com/acme/app.git".to_owned(),
    ];
    let place = hosts
        .place_of_checkout(root, crate::code_host::CheckoutRemotes::Addresses(&both))
        .unwrap();
    assert_eq!(place.host().key(), "github");
    // A stored repository setting names the gitlab.com project outright.
    let place = hosts.place_of("gitlab.com/acme/app").unwrap();
    assert_eq!(place.host().key(), "gitlab");
}

#[test]
fn gitlab_com_declares_only_the_operations_it_offers() {
    let declaration = crate::code_host::declaration(&host::GITLAB);
    assert_eq!(declaration.key, "gitlab");
    assert_eq!(declaration.operations, [crate::code_host::Operation::CommitChecks]);
    assert_eq!(declaration.access.program, "glab");
    assert_eq!(declaration.access.minimum_version, "1.78.0");
    let github = crate::code_host::declaration(&crate::github::host::GITHUB);
    assert_eq!(github.key, "github");
    assert_eq!(github.operations.len(), 12);
    assert_eq!(github.access.program, "gh");
}
