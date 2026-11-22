//! Each gitlab.com project's merge requests, read through the operator's own
//! `glab`.
//!
//! As with GitHub, Hide holds no token and makes no network call of its own:
//! every remote fact comes from a `glab` subprocess on the core's own node,
//! signed in as the person running the app, and "glab is not installed" or
//! "not signed in" are states, not errors. Only gitlab.com is read.
//!
//! A project costs one GraphQL query per pass, beside one version and one
//! sign-in check for the whole pass: the query names the open merge
//! requests with their head pipeline and jobs, their approvals and
//! reviewers, and the most recently updated of any state. `glab mr list`
//! and the REST list carry no pipeline.
//!
//! How each answer is judged is written in `docs/status-model.md`, Code
//! hosts: the checks from the head pipeline alone (never from
//! `detailedMergeStatus`), the review from GraphQL `approved` and the
//! reviewers (never from REST `approvals.approved`, which is true with no
//! approval when none is required).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;

pub(crate) mod host;

use crate::checkout_git::{Ci, CiState};
use crate::code_host::{CardRead, DueRead, HostFailure, HostReads, Place, RepositoryRef};
use crate::model::{
    GithubFailureCategory, GithubProjectSnapshot, GithubStatusSnapshot, MergeQueuePlace,
    MergeState, PR_REVIEWER_LIMIT, PartRead, PrCardFile, PrCardThread, PrCheckCounts,
    PrFailedCheck, PrReviewer, PrReviewerState, PullRequestChecks,
    PullRequestSnapshot, PullRequestWork, RepositoryIdentity, ReviewAsk, ReviewDecision,
    ReviewRequest, ReviewRequestKind,
};
use crate::node_access::{NodeLink, call_as};
use crate::observed::{Observation, now_unix_ms};
use hide_node_link::glab::{
    AUTH_STATUS, GlabAnswer, GlabVersion, MINIMUM_VERSION, VERSION, card_arguments,
    commit_checks_arguments, merge_requests_arguments,
};
use hide_node_link::protocol::Call;

/// The node bounds one `glab` command at fifteen seconds.
const GLAB_CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// A merge request's checks as they were last read, by number and head
/// commit: a merged one's pipeline is not read again, so this is where the
/// checks it was last seen open with are found.
type KnownChecks = HashMap<(u32, String), PullRequestChecks>;

/// Runs one allowed `glab` line on the core's own node.
fn glab(node: &dyn NodeLink, arguments: Vec<String>) -> Result<String, HostFailure> {
    let answer: GlabAnswer = call_as(node, Call::Glab { args: arguments }, GLAB_CALL_TIMEOUT)
        .map_err(|error| HostFailure {
            category: GithubFailureCategory::NetworkOrRateLimit,
            reason: format!("the node did not run glab: {error}"),
        })?;
    match answer {
        GlabAnswer::Output { cut: true, .. } => Err(HostFailure {
            category: GithubFailureCategory::NetworkOrRateLimit,
            reason: format!(
                "glab's answer passed {} bytes and was cut",
                hide_node_link::glab::ANSWER_BYTE_LIMIT
            ),
        }),
        GlabAnswer::Output { stdout, cut: false } => Ok(stdout),
        GlabAnswer::Failed { category, reason } => Err(HostFailure { category, reason }),
    }
}

fn owned(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

/// Whether this machine's `glab` can read gitlab.com now: a release at
/// least [`MINIMUM_VERSION`], signed in. A version line this cannot read
/// goes on with a diagnostic, since the commands still fail if it is too
/// old; a `--version` that failed is followed by `auth status`, which names
/// a missing `glab` or a lapsed login.
fn access(node: &dyn NodeLink) -> Result<(), HostFailure> {
    match glab(node, owned(&VERSION)) {
        Ok(output) => match GlabVersion::of_output(&output) {
            Some(version) if version < MINIMUM_VERSION => {
                return Err(HostFailure {
                    category: GithubFailureCategory::Outdated,
                    reason: format!(
                        "glab {version} is older than {MINIMUM_VERSION}, the oldest release Hide reads gitlab.com with"
                    ),
                });
            }
            Some(_) => {}
            None => crate::diagnostic!(serde_json::json!({
                "component": "gitlab",
                "kind": "glab.version_unread",
                "message": output.lines().next().unwrap_or_default(),
            })),
        },
        Err(failure) if failure.category == GithubFailureCategory::NotInstalled => {
            return Err(failure);
        }
        Err(failure) => crate::diagnostic!(serde_json::json!({
            "component": "gitlab",
            "kind": "glab.version_unread",
            "message": failure.reason,
        })),
    }
    glab(node, owned(&AUTH_STATUS)).map(|_| ())
}

/// gitlab.com's read of its projects ([`HostReads`]): one access check per
/// pass, and the checks each project carries from one answer to the next.
#[derive(Default)]
pub(crate) struct GitlabReads {
    carried: HashMap<PathBuf, KnownChecks>,
}

impl HostReads for GitlabReads {
    fn read(&mut self, node: &dyn NodeLink, due: &[DueRead<'_>]) -> Vec<GithubProjectSnapshot> {
        // One access check for the pass, not one per project: it is the same
        // answer every time, and it counts against the account's hundred
        // requests a minute.
        let access = std::cell::OnceCell::new();
        due.iter()
            .map(|due| {
                let started = Instant::now();
                let known = self.carried.remove(due.root).unwrap_or_default();
                let answer = match access.get_or_init(|| self::access(node)) {
                    Err(failure) => signed_out(due.root_path, failure),
                    Ok(()) => read_project(node, due, &known),
                };
                crate::code_host::read::log_answer(due.generation, &answer, started, None);
                let known = if answer.pull_requests_read {
                    remembered_checks(&answer.pull_requests)
                } else {
                    known
                };
                self.carried.insert(due.root.to_path_buf(), known);
                answer
            })
            .collect()
    }

    fn forget(&mut self, asked: &dyn Fn(&Path) -> bool) {
        self.carried.retain(|root, _| asked(root));
    }
}

/// A project read with no access to gitlab.com: `glab` missing, too old or
/// signed out, which fails every project of the pass alike.
fn signed_out(root_path: &str, failure: &HostFailure) -> GithubProjectSnapshot {
    GithubProjectSnapshot {
        root_path: root_path.to_owned(),
        issues_read: PartRead::Unsupported,
        identity_read: PartRead::NotRead,
        status: GithubStatusSnapshot {
            available: false,
            unavailable_reason: Some(failure.reason.clone()),
            failure_category: Some(failure.category),
            ..GithubStatusSnapshot::default()
        },
        ..GithubProjectSnapshot::default()
    }
}

/// A project whose list read failed: the previous answer is kept as stale by
/// the runtime (`keep_previous_reads`).
fn failed(root_path: &str, failure: HostFailure) -> GithubProjectSnapshot {
    GithubProjectSnapshot {
        root_path: root_path.to_owned(),
        issues_read: PartRead::Unsupported,
        identity_read: PartRead::Failed,
        status: GithubStatusSnapshot {
            available: true,
            stale: true,
            unavailable_reason: Some(failure.reason),
            failure_category: Some(failure.category),
            ..GithubStatusSnapshot::default()
        },
        ..GithubProjectSnapshot::default()
    }
}

/// The repository a place names. gitlab.com's unit never reads a checkout
/// in place ([`crate::code_host::CodeHost::resolves_checkout`]), so the
/// registry hands it a repository; a checkout place is still read by the
/// first remote it lists for gitlab.com.
fn repository_of(place: &Place) -> Option<&RepositoryRef> {
    match place {
        Place::Repository { repository, .. } => Some(repository),
        Place::Checkout { remotes, .. } => remotes.first(),
    }
}

fn read_project(
    node: &dyn NodeLink,
    due: &DueRead<'_>,
    known: &KnownChecks,
) -> GithubProjectSnapshot {
    let Some(repository) = repository_of(due.place) else {
        return failed(
            due.root_path,
            HostFailure {
                category: GithubFailureCategory::NetworkOrRateLimit,
                reason: "the checkout names no gitlab.com project".to_owned(),
            },
        );
    };
    let answer = merge_requests_arguments(&repository.path)
        .ok_or_else(|| HostFailure {
            category: GithubFailureCategory::NetworkOrRateLimit,
            reason: format!("{} is not a project path glab may be asked about", repository.path),
        })
        .and_then(|arguments| glab(node, arguments))
        .and_then(|output| parse_list(&output, known));
    match answer {
        Ok(listed) => listed.into_snapshot(due.root_path),
        Err(failure) => failed(due.root_path, failure),
    }
}

// ---- The list answer -------------------------------------------------------

#[derive(Deserialize)]
struct ListAnswer {
    data: Option<ListData>,
    #[serde(default)]
    errors: Vec<GraphqlError>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListData {
    current_user: Option<User>,
    project: Option<Project>,
}

#[derive(Deserialize)]
struct User {
    username: String,
    #[serde(default)]
    bot: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    id: String,
    full_path: String,
    visibility: Option<String>,
    repository: Option<ProjectRepository>,
    open: Connection<MergeRequest>,
    settled: Connection<MergeRequest>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectRepository {
    root_ref: Option<String>,
}

#[derive(Deserialize)]
struct Connection<T> {
    #[serde(default)]
    count: Option<u32>,
    nodes: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergeRequest {
    iid: String,
    title: String,
    state: String,
    draft: bool,
    web_url: String,
    source_branch: String,
    target_branch: String,
    diff_head_sha: Option<String>,
    source_project_id: Option<u64>,
    target_project_id: Option<u64>,
    created_at: Option<String>,
    updated_at: Option<String>,
    merged_at: Option<String>,
    closed_at: Option<String>,
    author: Option<User>,
    // Only the open list reads these.
    #[serde(default)]
    detailed_merge_status: Option<String>,
    #[serde(default)]
    approved: Option<bool>,
    #[serde(default)]
    approvals_required: Option<u32>,
    #[serde(default)]
    approved_by: Option<Connection<Named>>,
    #[serde(default)]
    reviewers: Option<Connection<Reviewer>>,
    #[serde(default)]
    head_pipeline: Option<Pipeline>,
}

#[derive(Deserialize)]
struct Named {
    username: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reviewer {
    username: String,
    merge_request_interaction: Option<Interaction>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Interaction {
    review_state: Option<String>,
}

#[derive(Deserialize)]
struct Pipeline {
    status: String,
    #[serde(default)]
    jobs: Option<Connection<Job>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    name: Option<String>,
    status: String,
    #[serde(default)]
    allow_failure: bool,
    web_path: Option<String>,
}

/// What one list read found.
#[derive(Debug)]
struct Listed {
    viewer: String,
    identity: RepositoryIdentity,
    pull_requests: Vec<PullRequestSnapshot>,
}

impl Listed {
    fn into_snapshot(self, root_path: &str) -> GithubProjectSnapshot {
        GithubProjectSnapshot {
            root_path: root_path.to_owned(),
            pull_requests_read: true,
            issues_read: PartRead::Unsupported,
            repository: Some(self.identity),
            identity_read: PartRead::Read,
            viewer: Some(self.viewer),
            pull_requests: self.pull_requests,
            status: GithubStatusSnapshot {
                available: true,
                last_success_at_unix_ms: Some(now_unix_ms()),
                ..GithubStatusSnapshot::default()
            },
            ..GithubProjectSnapshot::default()
        }
    }
}

fn unreadable(reason: String) -> HostFailure {
    HostFailure {
        category: GithubFailureCategory::NetworkOrRateLimit,
        reason,
    }
}

/// Reads one list answer. The project's identity is named from the answer
/// itself (`fullPath`), never guessed, so its server is always known.
fn parse_list(output: &str, known: &KnownChecks) -> Result<Listed, HostFailure> {
    let answer: ListAnswer = serde_json::from_str(output)
        .map_err(|error| unreadable(format!("glab returned a list Hide could not read: {error}")))?;
    if let Some(error) = answer.errors.first() {
        return Err(unreadable(format!("gitlab.com answered with an error: {}", error.message)));
    }
    let data = answer
        .data
        .ok_or_else(|| unreadable("gitlab.com answered with no data".to_owned()))?;
    let viewer = data.current_user.ok_or_else(|| HostFailure {
        category: GithubFailureCategory::NotLoggedIn,
        reason: "gitlab.com did not name the signed-in account".to_owned(),
    })?;
    let project = data.project.ok_or_else(|| {
        unreadable("gitlab.com has no such project this account can read".to_owned())
    })?;
    let unit: &dyn crate::code_host::CodeHost = &host::GITLAB;
    let repository = unit
        .repository(&format!("{}/{}", hide_node_link::glab::HOST, project.full_path))
        .ok_or_else(|| unreadable(format!("gitlab.com named a project path Hide cannot read: {}", project.full_path)))?;
    let identity = RepositoryIdentity {
        name: project.full_path.clone(),
        url: Some(unit.repository_url(&repository)),
        repository: Some(repository.clone()),
        id: Some(project.id),
        public: project.visibility.as_deref().map(|visibility| visibility == "public"),
        default_branch: project.repository.and_then(|repository| repository.root_ref),
    };
    let open_total = project.open.count.unwrap_or(0) as usize;
    if open_total > project.open.nodes.len() {
        // Principle 15: the newest part is read, and the log says so.
        crate::diagnostic!(serde_json::json!({
            "component": "gitlab",
            "kind": "pull_requests.list_cut",
            "project": project.full_path,
            "listed": project.open.nodes.len(),
            "open": open_total,
        }));
    }
    let viewer_name = viewer.username;
    let mut pull_requests = Vec::new();
    for listed in project.open.nodes {
        pull_requests.push(pull_request(&repository, listed, Some(&viewer_name), known)?);
    }
    for listed in project.settled.nodes {
        let settled = matches!(listed.state.as_str(), "merged" | "closed");
        let iid = listed.iid.clone();
        if settled
            && !pull_requests
                .iter()
                .any(|known| known.number.to_string() == iid)
        {
            pull_requests.push(pull_request(&repository, listed, Some(&viewer_name), known)?);
        }
    }
    Ok(Listed {
        viewer: viewer_name,
        identity,
        pull_requests,
    })
}

fn pull_request(
    repository: &RepositoryRef,
    listed: MergeRequest,
    viewer: Option<&str>,
    known: &KnownChecks,
) -> Result<PullRequestSnapshot, HostFailure> {
    let number: u32 = listed
        .iid
        .parse()
        .ok()
        .filter(|number| *number > 0)
        .ok_or_else(|| unreadable(format!("gitlab.com named a merge request {:?}", listed.iid)))?;
    let open = listed.state == "opened";
    let head_oid = listed.diff_head_sha.clone().filter(|sha| !sha.is_empty());
    let review = open.then(|| review_decision(&listed)).flatten();
    let state = match listed.state.as_str() {
        "merged" => "MERGED",
        "closed" | "locked" => "CLOSED",
        _ => "OPEN",
    };
    let checks = if open {
        Checks::of(listed.head_pipeline.as_ref())
    } else {
        Checks {
            verdict: head_oid
                .as_ref()
                .and_then(|head| known.get(&(number, head.clone())).copied())
                .unwrap_or(PullRequestChecks::Unknown),
            ..Checks::default()
        }
    };
    let unit: &dyn crate::code_host::CodeHost = &host::GITLAB;
    let reference = crate::code_host::change_request(unit, repository.clone(), number);
    let author = listed.author.as_ref().map(|author| author.username.clone());
    let reviewers = open.then(|| reviewers(&listed));
    let (asked_of_me, review_requests) = if open {
        review_asks(&listed, viewer)
    } else {
        (ReviewAsk::None, Vec::new())
    };
    let mut work = PullRequestWork {
        author_is_bot: listed.author.as_ref().is_some_and(|author| author.bot),
        mine: viewer.zip(author.as_deref()).map(|(viewer, author)| viewer == author),
        author,
        merge_state: if open {
            merge_state(listed.detailed_merge_status.as_deref(), listed.draft, checks.verdict)
        } else {
            MergeState::Unknown
        },
        review_requests,
        asked_of_me,
        // gitlab.com's merge trains are a paid tier's, and Hide does not
        // read them: no merge request waits on a place Hide cannot know.
        queue: MergeQueuePlace::Unsupported,
        required_approvals: listed.approvals_required.filter(|required| *required > 0),
        reviewers_more: reviewers.as_ref().map_or(0, |(_, more)| *more),
        reviewers: reviewers.map(|(reviewers, _)| reviewers),
        ..PullRequestWork::default()
    };
    let verdict = checks.verdict;
    checks.apply(&mut work);
    Ok(PullRequestSnapshot {
        closing_issues: Vec::new(),
        title: listed.title,
        checks: verdict,
        number,
        head_branch: listed.source_branch,
        base_branch: listed.target_branch,
        url: listed.web_url,
        badge: crate::github::badge(state, listed.draft, review),
        review: review.filter(|_| !listed.draft),
        is_draft: listed.draft,
        merged_at_unix_ms: listed.merged_at.as_deref().and_then(crate::github::parse_rfc3339_ms),
        updated_at_unix_ms: listed.updated_at.as_deref().and_then(crate::github::parse_rfc3339_ms),
        created_at_unix_ms: listed.created_at.as_deref().and_then(crate::github::parse_rfc3339_ms),
        closed_at_unix_ms: listed.closed_at.as_deref().and_then(crate::github::parse_rfc3339_ms),
        head_oid,
        cross_repository: listed
            .source_project_id
            .zip(listed.target_project_id)
            .is_some_and(|(source, target)| source != target),
        work,
        reference: Some(reference),
    })
}

// ---- Verdicts ---------------------------------------------------------------

/// What a head pipeline adds up to, by its own status. Its jobs only count
/// and name: a job allowed to fail leaves the pipeline `SUCCESS` (GitLab
/// itself says "passed with warnings"), and a pipeline with more jobs than
/// were read is still judged whole.
#[derive(Debug, Default)]
struct Checks {
    verdict: PullRequestChecks,
    counts: Option<PrCheckCounts>,
    failed: Vec<PrFailedCheck>,
    pending: u32,
    running: u32,
    unfinished: bool,
    cancelled: bool,
}

/// A pipeline or job still on its way: queued, preparing, waiting for a
/// resource or a callback, scheduled, or running.
const RUNNING_STATUSES: [&str; 8] = [
    "CREATED",
    "WAITING_FOR_RESOURCE",
    "PREPARING",
    "WAITING_FOR_CALLBACK",
    "PENDING",
    "RUNNING",
    "SCHEDULED",
    "CANCELING",
];

/// The verdict of a head pipeline status (`docs/status-model.md`, Code
/// hosts). `MANUAL` waits on a person and nothing runs; `SKIPPED` ran
/// nothing; neither reads as a pass.
pub(crate) fn pipeline_verdict(status: Option<&str>) -> PullRequestChecks {
    match status {
        None | Some("SKIPPED") => PullRequestChecks::None,
        Some("SUCCESS") => PullRequestChecks::Passing,
        Some("FAILED" | "CANCELED") => PullRequestChecks::Failed,
        Some("MANUAL") => PullRequestChecks::Pending,
        Some(status) if RUNNING_STATUSES.contains(&status) => PullRequestChecks::Pending,
        Some(_) => PullRequestChecks::Unknown,
    }
}

impl Checks {
    /// `headPipeline` is gitlab.com's own pipeline for the merge request's
    /// head: one whose commit or source commit is the head, so a merged
    /// results pipeline, which runs on a merge commit of its own, is the
    /// head's too. Its commit is therefore not compared with the head here.
    fn of(pipeline: Option<&Pipeline>) -> Self {
        let Some(pipeline) = pipeline else {
            return Self {
                verdict: PullRequestChecks::None,
                ..Self::default()
            };
        };
        let status = pipeline.status.as_str();
        let mut checks = Self {
            verdict: pipeline_verdict(Some(status)),
            cancelled: status == "CANCELED",
            unfinished: RUNNING_STATUSES.contains(&status),
            ..Self::default()
        };
        let Some(jobs) = &pipeline.jobs else {
            return checks;
        };
        let mut counts = PrCheckCounts {
            total: jobs.count.unwrap_or(jobs.nodes.len() as u32),
            ..PrCheckCounts::default()
        };
        for job in &jobs.nodes {
            match job.status.as_str() {
                "SUCCESS" => counts.passed += 1,
                // An allowed failure does not fail the pipeline, so it is
                // not one of its failures either.
                "FAILED" if job.allow_failure => counts.passed += 1,
                "FAILED" | "CANCELED" => {
                    counts.failed += 1;
                    checks.failed.push(PrFailedCheck {
                        name: job.name.clone().filter(|name| !name.trim().is_empty()),
                        url: job
                            .web_path
                            .as_deref()
                            .filter(|path| path.starts_with('/'))
                            .map(|path| format!("https://{}{path}", hide_node_link::glab::HOST)),
                    });
                }
                "SKIPPED" => counts.skipped += 1,
                // An optional manual job is an action anyone may take, not
                // a check the pipeline waits on.
                "MANUAL" if job.allow_failure => counts.skipped += 1,
                "MANUAL" => {
                    counts.waiting += 1;
                    checks.pending += 1;
                }
                "RUNNING" | "CANCELING" => {
                    counts.running += 1;
                    checks.pending += 1;
                    checks.running += 1;
                }
                status if RUNNING_STATUSES.contains(&status) => {
                    counts.waiting += 1;
                    checks.pending += 1;
                    checks.running += 1;
                }
                _ => counts.unknown += 1,
            }
        }
        checks.unfinished |= checks.running > 0;
        // A job set that was cut is counted only as far as it was read: its
        // counts would read as the whole, so none are given.
        if jobs.nodes.len() as u32 >= counts.total {
            checks.counts = Some(counts);
        }
        checks
    }

    fn apply(self, work: &mut PullRequestWork) {
        work.failed_checks = self.failed;
        work.pending_checks = self.pending;
        work.check_counts = self.counts;
        work.running_checks = self.running;
        work.checks_running = self.unfinished;
        work.checks_cancelled = self.cancelled;
    }
}

/// The review that stands on an open merge request: a change request
/// first, then GraphQL `approved`, then review still required
/// (`approvalsRequired` above zero); with nothing required and nobody
/// having approved, none.
fn review_decision(listed: &MergeRequest) -> Option<ReviewDecision> {
    // gitlab.com also says so in the merge status when someone who is no
    // longer listed as a reviewer asked for changes.
    let changes_requested = listed.detailed_merge_status.as_deref() == Some("REQUESTED_CHANGES")
        || listed.reviewers.as_ref().is_some_and(|reviewers| {
            reviewers
                .nodes
                .iter()
                .any(|reviewer| review_state(reviewer) == Some("REQUESTED_CHANGES"))
        });
    if changes_requested {
        Some(ReviewDecision::ChangesRequested)
    } else if listed.approved == Some(true) {
        Some(ReviewDecision::Approved)
    } else if listed.approvals_required.is_some_and(|required| required > 0) {
        Some(ReviewDecision::ReviewRequired)
    } else {
        None
    }
}

fn review_state(reviewer: &Reviewer) -> Option<&str> {
    reviewer
        .merge_request_interaction
        .as_ref()
        .and_then(|interaction| interaction.review_state.as_deref())
}

/// Whether a reviewer is still asked: not yet reviewed, or a review begun
/// and not submitted.
fn still_asked(reviewer: &Reviewer) -> bool {
    matches!(review_state(reviewer), None | Some("UNREVIEWED" | "REVIEW_STARTED"))
}

/// Each reviewer and approver and where they stand, approvals and change
/// requests first, with how many reviewers the page did not hold.
fn reviewers(listed: &MergeRequest) -> (Vec<PrReviewer>, u32) {
    let mut reviewers: Vec<PrReviewer> = Vec::new();
    let mut more = 0;
    if let Some(listed) = &listed.reviewers {
        more = listed
            .count
            .unwrap_or(0)
            .saturating_sub(listed.nodes.len() as u32);
        for reviewer in &listed.nodes {
            let state = match review_state(reviewer) {
                Some("APPROVED") => PrReviewerState::Approved,
                Some("REQUESTED_CHANGES") => PrReviewerState::ChangesRequested,
                Some("REVIEWED") => PrReviewerState::Commented,
                Some("UNAPPROVED") => PrReviewerState::Dismissed,
                _ => PrReviewerState::Requested,
            };
            reviewers.push(PrReviewer {
                login: reviewer.username.clone(),
                team: false,
                state,
                at_unix_ms: None,
            });
        }
    }
    // An approver need not be a reviewer: anyone allowed may approve.
    for approver in listed.approved_by.iter().flat_map(|approved| &approved.nodes) {
        if !reviewers.iter().any(|known| known.login == approver.username) {
            reviewers.push(PrReviewer {
                login: approver.username.clone(),
                team: false,
                state: PrReviewerState::Approved,
                at_unix_ms: None,
            });
        }
    }
    reviewers.sort_by_key(|reviewer| reviewer.state);
    let cut = reviewers.len().saturating_sub(PR_REVIEWER_LIMIT) as u32;
    reviewers.truncate(PR_REVIEWER_LIMIT);
    (reviewers, more + cut)
}

/// Whether a review is asked of the signed-in account, and of whom it is
/// still asked. gitlab.com asks people, never teams. With the account
/// unknown, or the reviewers cut short without it, the answer is unknown.
fn review_asks(listed: &MergeRequest, viewer: Option<&str>) -> (ReviewAsk, Vec<ReviewRequest>) {
    let Some(reviewers) = &listed.reviewers else {
        return (ReviewAsk::Unknown, Vec::new());
    };
    let requests: Vec<ReviewRequest> = reviewers
        .nodes
        .iter()
        .filter(|reviewer| still_asked(reviewer))
        .map(|reviewer| ReviewRequest {
            kind: ReviewRequestKind::User,
            name: reviewer.username.clone(),
        })
        .collect();
    let complete = reviewers.count.unwrap_or(0) as usize <= reviewers.nodes.len();
    let ask = match viewer {
        None => ReviewAsk::Unknown,
        Some(viewer) if requests.iter().any(|request| request.name == viewer) => ReviewAsk::Direct,
        Some(_) if complete => ReviewAsk::None,
        Some(_) => ReviewAsk::Unknown,
    };
    (ask, requests)
}

/// How an open merge request stands against its base, from
/// `detailedMergeStatus` (`docs/status-model.md`, Code hosts). It never
/// stands in for the checks: a failed pipeline in a project that does not
/// require one to pass is `MERGEABLE`, which is GitHub's `UNSTABLE` (merge
/// allowed, a check not passing), as is any other pipeline not passing.
pub(crate) fn merge_state(
    status: Option<&str>,
    draft: bool,
    checks: PullRequestChecks,
) -> MergeState {
    if draft {
        return MergeState::Draft;
    }
    match status {
        Some("MERGEABLE") => match checks {
            PullRequestChecks::Passing | PullRequestChecks::None => MergeState::Clean,
            _ => MergeState::Unstable,
        },
        Some("DRAFT_STATUS") => MergeState::Draft,
        Some("CONFLICT" | "BROKEN_STATUS") => MergeState::Dirty,
        Some("NEED_REBASE") => MergeState::Behind,
        // Still computed: the merge request stays where it was.
        None | Some("CHECKING" | "UNCHECKED" | "PREPARING" | "NOT_OPEN") => MergeState::Unknown,
        // Every other status names something that blocks the merge: a
        // pipeline that must pass, approvals, open discussions, a change
        // request, a blocking merge request, a policy, a lock.
        Some(_) => MergeState::Blocked,
    }
}

fn remembered_checks(pull_requests: &[PullRequestSnapshot]) -> KnownChecks {
    pull_requests
        .iter()
        .filter(|pull_request| pull_request.checks != PullRequestChecks::Unknown)
        .filter_map(|pull_request| {
            Some((
                (pull_request.number, pull_request.head_oid.clone()?),
                pull_request.checks,
            ))
        })
        .collect()
}

// ---- The card ---------------------------------------------------------------

/// Reads merge request `number`'s card: its head, description, changed
/// files and unresolved discussions. A merge is not an operation gitlab.com
/// declares in this round, so the card names no merge part.
pub(crate) fn read_card(
    node: &dyn NodeLink,
    repository: &RepositoryRef,
    number: u32,
) -> Result<CardRead, String> {
    let arguments = card_arguments(&repository.path, number)
        .ok_or_else(|| "the project cannot be asked about a card".to_owned())?;
    let output = glab(node, arguments).map_err(|failure| failure.reason)?;
    parse_card(&output)
}

fn parse_card(output: &str) -> Result<CardRead, String> {
    let value: serde_json::Value = serde_json::from_str(output)
        .map_err(|error| format!("glab returned a card Hide could not read: {error}"))?;
    let merge_request = &value["data"]["project"]["mergeRequest"];
    if merge_request.is_null() {
        return Err("gitlab.com returned no such merge request".to_owned());
    }
    let mut read = CardRead {
        head: merge_request["diffHeadSha"].as_str().map(str::to_owned),
        ..CardRead::default()
    };
    match &merge_request["description"] {
        serde_json::Value::String(body) => {
            read.body = Some(crate::github::card::capped_body(body));
        }
        serde_json::Value::Null => read.body = Some(String::new()),
        _ => read.failed.push("body"),
    }
    match merge_request["diffStats"].as_array() {
        Some(nodes) => {
            let files: Vec<PrCardFile> = nodes
                .iter()
                .filter_map(|node| {
                    let path = node["path"].as_str()?;
                    Some(PrCardFile {
                        path: path.to_owned(),
                        additions: node["additions"].as_u64().unwrap_or(0) as u32,
                        deletions: node["deletions"].as_u64().unwrap_or(0) as u32,
                        generated: crate::github::card::is_generated(&[], path),
                    })
                })
                .collect();
            read.files_total = merge_request["diffStatsSummary"]["fileCount"]
                .as_u64()
                .unwrap_or(files.len() as u64) as u32;
            read.files = Some(
                files
                    .into_iter()
                    .take(hide_node_link::gh::CARD_FILE_LIMIT)
                    .collect(),
            );
        }
        None => read.failed.push("files"),
    }
    match merge_request["discussions"]["nodes"].as_array() {
        Some(nodes) => {
            read.threads = Some(
                nodes
                    .iter()
                    .filter(|node| {
                        node["resolvable"].as_bool() == Some(true)
                            && node["resolved"].as_bool() == Some(false)
                    })
                    .map(|node| {
                        let first = &node["notes"]["nodes"][0];
                        let position = &first["position"];
                        PrCardThread {
                            reviewer: first["author"]["username"].as_str().map(str::to_owned),
                            path: position["newPath"]
                                .as_str()
                                .or(position["oldPath"].as_str())
                                .unwrap_or_default()
                                .to_owned(),
                            line: position["newLine"].as_u64().map(|line| line as u32),
                            text: crate::github::card::first_sentence(
                                first["body"].as_str().unwrap_or_default(),
                            ),
                        }
                    })
                    .collect(),
            );
        }
        None => read.failed.push("threads"),
    }
    Ok(read)
}

// ---- Checks of exact commits ------------------------------------------------

/// What each of `commits` has for checks in `repository`: its latest
/// pipeline's status, with the pipeline's commit checked against the one
/// asked about.
pub(crate) fn read_commit_checks(
    node: &dyn NodeLink,
    repository: &RepositoryRef,
    commits: &[String],
) -> Result<Vec<Ci>, String> {
    let arguments = commit_checks_arguments(&repository.path, commits)
        .ok_or("Invalid exact-SHA check targets")?;
    let output = glab(node, arguments).map_err(|failure| failure.reason)?;
    let unit: &dyn crate::code_host::CodeHost = &host::GITLAB;
    parse_commit_checks(&output, commits, |sha| unit.commit_url(repository, sha))
}

fn parse_commit_checks(
    output: &str,
    commits: &[String],
    commit_url: impl Fn(&str) -> String,
) -> Result<Vec<Ci>, String> {
    let value: serde_json::Value =
        serde_json::from_str(output).map_err(|error| error.to_string())?;
    if value["errors"].as_array().is_some_and(|errors| !errors.is_empty()) {
        return Err("Provider returned check errors".into());
    }
    let project = &value["data"]["project"];
    if !project.is_object() {
        return Err("Provider returned no project".into());
    }
    commits
        .iter()
        .enumerate()
        .map(|(index, sha)| {
            let nodes = project[format!("c{index}")]["nodes"]
                .as_array()
                .ok_or("Provider omitted a commit result")?;
            let state = match nodes.first() {
                // A commit with no pipeline, or one not pushed.
                None => CiState::None,
                Some(pipeline) => {
                    if pipeline["sha"].as_str() != Some(sha.as_str()) {
                        return Err("Provider returned a different commit identity".into());
                    }
                    match pipeline_verdict(pipeline["status"].as_str()) {
                        PullRequestChecks::Passing => CiState::Passing,
                        PullRequestChecks::Failed => CiState::Failed,
                        PullRequestChecks::Pending => CiState::Pending,
                        PullRequestChecks::None => CiState::None,
                        PullRequestChecks::Unknown => CiState::Unknown,
                    }
                }
            };
            Ok(Ci {
                sha: sha.clone(),
                state,
                observed: Some(Observation::fresh(now_unix_ms())),
                url: Some(commit_url(sha)),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
