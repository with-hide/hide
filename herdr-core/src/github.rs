//! Each open repository's pull requests, read through the operator's own `gh`.
//!
//! Hide holds no GitHub token and makes no network call: every remote fact
//! here comes from a `gh` subprocess that authenticates as the person running
//! the app. That is the whole security boundary, and it is why "gh is not
//! installed" and "gh is not logged in" are first-class states rather than
//! errors - they are the normal condition of a machine that has not opted in.
//!
//! Like the worktree reader, the subprocess runs on a worker thread: `gh pr
//! list` reaches the network and routinely takes a second or more, which would
//! otherwise be a second of coordinator latency for every Herdr pane event.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Deserialize;

pub(crate) mod card;

use crate::model::{
    ExtraQuery, GithubFailureCategory, GithubProjectSnapshot, GithubSnapshot, GithubStatusSnapshot,
    MergeState, PrFailedCheck, PullRequestBadge, PullRequestChecks, PullRequestSnapshot,
    PullRequestWork, ReviewDecision, ReviewRequest, ReviewRequestKind,
};
use crate::node_access::NodeLink;
use crate::reader::BackgroundRead;
use hide_node_link::gh::{
    GhAnswer, PR_FEEDBACK_FIELDS, REVIEW_ASKED_FIELDS, REVIEW_ASKED_SEARCH, SEARCH_ISSUE_FIELDS,
    SEARCH_LIMIT, SEARCH_PR_FIELDS, VIEWER_LOGIN, is_number,
};
pub(crate) use hide_node_link::gh::{SEARCH_QUERY_LIMIT, SEARCH_REPOSITORY_LIMIT};
use hide_node_link::git::GitCommand;
use hide_node_link::protocol::Call;

/// The node bounds one `gh` command at fifteen seconds.
const GH_CALL_TIMEOUT: Duration = Duration::from_secs(20);

pub(crate) use hide_node_link::gh::PULL_REQUEST_LIMIT;
use hide_node_link::gh::{
    ISSUE_LIST_FIELDS, ISSUE_LIST_LIMIT, ISSUE_LIST_PROJECT_FIELDS, ISSUE_LIST_SEARCH,
    MERGED_PROOF_FIELDS, MERGED_PROOF_LIMIT, OPEN_CHECK_FIELDS, PULL_REQUEST_FIELDS,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GithubProjectRequest {
    pub links: Vec<crate::issues::IssueReference>,
    /// A path inside the repository.
    pub root: PathBuf,
    /// The GitHub repository of a checkout on another machine, whose folder
    /// this machine cannot read: `gh` is asked about it by name, and `root`
    /// is that machine's path, reported as it is.
    pub repository: Option<String>,
    /// Bumped on Git section opening and explicit header refresh.
    /// Each repository's generation coalesces repeated requests independently.
    pub generation: u64,
    /// The repositories the checkout's remotes name, as its node read them
    /// from the repository's config (`PathIndex::remotes`); `None` when it
    /// did not say. An empty list is answered "no GitHub remote" with no `gh`
    /// call.
    pub remotes: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GithubRequest {
    pub projects: Vec<GithubProjectRequest>,
}

/// Why one project is read on this pass, stated in the read log so "did the
/// refresh button / section opening actually
/// re-read?" is answered afterwards without a debugger (G5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReadReason {
    First,
    Generation,
}

impl ReadReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Generation => "generation",
        }
    }
}

/// Whether a project must be read again, given what the reader last answered
/// for it: nothing yet or an answer for an older request. An answer for the
/// same request is reused indefinitely, which
/// is what keeps one project's trigger from re-reading every other project.
fn read_reason(cached: Option<u64>, generation: u64) -> Option<ReadReason> {
    match cached {
        None => Some(ReadReason::First),
        Some(read_generation) if read_generation != generation => Some(ReadReason::Generation),
        Some(_) => None,
    }
}

/// A pull request's check rollup as it was last read, by number and head
/// commit. A settled pull request's checks are not asked for again, so this
/// is where one that was read while it was open is still found.
type KnownChecks = HashMap<(u32, String), PullRequestChecks>;

struct CachedProject {
    generation: u64,
    repository: Option<String>,
    answer: GithubProjectSnapshot,
    /// Kept across a failed read, so a pull request that merged while GitHub
    /// was unreachable still has the checks it was last seen with.
    checks: KnownChecks,
}

/// The last answer per requested root, kept on the worker's side so a pass
/// re-reads only the projects that are due and hands the rest back as they
/// were. It is the reader's own state, never the runtime's: the runtime
/// receives a whole snapshot every time and does not know which half is new.
type Cache = Arc<Mutex<HashMap<PathBuf, CachedProject>>>;

pub struct GithubReader {
    inner: BackgroundRead<GithubRequest, GithubAnswer>,
}

pub struct GithubAnswer {
    pub request: GithubRequest,
    pub snapshot: GithubSnapshot,
}

impl GithubReader {
    /// A reader whose `gh` and git run on `node`, the core's own machine,
    /// where the operator's GitHub login is.
    pub fn new(node: Arc<dyn NodeLink>) -> Self {
        let cache: Cache = Arc::default();
        Self {
            inner: BackgroundRead::on_change(Duration::ZERO, move |request: &GithubRequest| {
                GithubAnswer {
                    request: request.clone(),
                    snapshot: read(&cache, node.as_ref(), request),
                }
            }),
        }
    }

    pub fn read_if_due(&mut self, request: GithubRequest) -> Option<GithubAnswer> {
        self.inner.poll(request)
    }
}

fn read(
    cache: &Mutex<HashMap<PathBuf, CachedProject>>,
    node: &dyn NodeLink,
    request: &GithubRequest,
) -> GithubSnapshot {
    // The worker is the only thread that touches the cache, and one worker
    // runs at a time; the lock exists so the closure can be shared with it.
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    cache.retain(|root, _| request.projects.iter().any(|project| &project.root == root));
    let due: Vec<(&GithubProjectRequest, ReadReason)> = request
        .projects
        .iter()
        .filter_map(|project| {
            let cached = cache
                .get(&project.root)
                .filter(|cached| cached.repository == project.repository)
                .map(|cached| cached.generation);
            read_reason(cached, project.generation).map(|reason| (project, reason))
        })
        .collect();
    if !due.is_empty() {
        // One line per pass naming each project read and why, with the
        // generation that asked for it.
        crate::diagnostic!(serde_json::json!({
            "component": "github",
            "kind": "pull_requests.read",
            "projects": due
                .iter()
                .map(|(project, reason)| serde_json::json!({
                    "project": project.root.to_string_lossy(),
                    "generation": project.generation,
                    "reason": reason.as_str(),
                }))
                .collect::<Vec<_>>(),
        }));
        // One authentication check for the pass, not one per repository: `gh
        // auth status` is the same answer every time and it is the expensive
        // half of an unauthenticated machine's cost. A pass that reads only
        // repositories with no GitHub remote asks nothing.
        let authentication = std::cell::OnceCell::new();
        for (project, _) in due {
            let known = cache
                .get(&project.root)
                .filter(|cached| cached.repository == project.repository)
                .map(|cached| cached.checks.clone())
                .unwrap_or_default();
            let answer = read_root(node, &authentication, project, &known);
            let checks = carried_checks(&answer, known);
            cache.insert(
                project.root.clone(),
                CachedProject {
                    generation: project.generation,
                    repository: project.repository.clone(),
                    answer,
                    checks,
                },
            );
        }
    }
    let mut projects: Vec<GithubProjectSnapshot> = Vec::new();
    for project in &request.projects {
        let Some(cached) = cache.get(&project.root) else {
            continue;
        };
        // Two navigator entries inside one repository share its answer.
        if cached.answer.root_path.is_empty()
            || projects
                .iter()
                .any(|known| known.root_path == cached.answer.root_path)
        {
            continue;
        }
        projects.push(cached.answer.clone());
    }
    GithubSnapshot { projects }
}

/// One repository's answer, or an empty `root_path` when the path is not
/// inside a git repository at all and so has nothing to report.
fn read_root(
    node: &dyn NodeLink,
    authentication: &std::cell::OnceCell<Result<Option<String>, GhFailure>>,
    request: &GithubProjectRequest,
    known: &KnownChecks,
) -> GithubProjectSnapshot {
    let started = Instant::now();
    let (place, root_path) = match &request.repository {
        Some(repository) => (
            Where::Repository(repository.clone()),
            hide_platform::path::to_wire_lossy(&request.root),
        ),
        None => {
            let Some(main) = main_worktree(node, &request.root) else {
                return GithubProjectSnapshot::default();
            };
            let root_path = hide_platform::path::to_wire_lossy(&main);
            (Where::Checkout(main), root_path)
        }
    };
    // A checkout whose remotes name no repository `gh` could read is
    // answered from its node's config, with no `gh` call: asking only
    // failed, and was asked again forever.
    let no_remote = request.repository.is_none()
        && request.remotes.as_deref().is_some_and(<[String]>::is_empty);
    let project = if no_remote {
        no_github_remote(root_path)
    } else {
        match authentication.get_or_init(|| self::authentication(node)) {
            Err(reason) => GithubProjectSnapshot {
                root_path,
                status: GithubStatusSnapshot {
                    available: false,
                    unavailable_reason: Some(reason.reason.clone()),
                    failure_category: Some(reason.category),
                    ..GithubStatusSnapshot::default()
                },
                ..GithubProjectSnapshot::default()
            },
            Ok(viewer) => read_project(
                node,
                &place,
                root_path,
                viewer.as_deref(),
                &request.links,
                known,
            ),
        }
    };
    // A failure and an empty answer are both stated, separately: an empty
    // list with no reason is a repository with no pull requests.
    crate::diagnostic!(serde_json::json!({
        "component": "github",
        "kind": if project.read_outcome() == crate::model::GithubReadOutcome::NoRemote {
            "pull_requests.no_remote"
        } else if project.status.unavailable_reason.is_some() {
            "pull_requests.failed"
        } else if project.pull_requests.is_empty() {
            "pull_requests.empty"
        } else {
            "pull_requests.ok"
        },
        "generation": request.generation,
        "project": project.root_path,
        "available": project.status.available,
        "pull_requests": project.pull_requests.len(),
        "duration_ms": started.elapsed().as_millis() as u64,
        "message": project.status.unavailable_reason,
    }));
    project
}

/// Authentication is read-only and uses the same bounded subprocess as PR
/// listing. `gh auth status` failing is the machine's refusal and fails every
/// read. The signed-in account's login, read once per pass, only tells the
/// operator's own pull requests from others' (PRD work-tab-pr-lens D-25), so
/// failing to read it is `Ok(None)`: the read goes on with "mine" unknown
/// (issue #1189).
fn authentication(node: &dyn NodeLink) -> Result<Option<String>, GhFailure> {
    gh(node, None, &["auth", "status"])?;
    let login = match gh(node, None, &VIEWER_LOGIN) {
        Ok(login) => login,
        Err(reason) => {
            unread(ExtraQuery::Account, &reason.reason);
            return Ok(None);
        }
    };
    let login = login.trim();
    if login.is_empty() {
        unread(ExtraQuery::Account, "gh did not name the signed-in account");
        return Ok(None);
    }
    Ok(Some(login.to_owned()))
}

/// States that an extra question went unanswered, with the reason, for the
/// diagnostic log (design principle 13): the operator sees which question,
/// not `gh`'s words.
fn unread(query: ExtraQuery, reason: &str) {
    crate::diagnostic!(serde_json::json!({
        "component": "github",
        "kind": "pull_requests.extra_unread",
        "query": query,
        "message": reason,
    }));
}

fn read_project(
    node: &dyn NodeLink,
    root: &Where,
    root_path: String,
    viewer: Option<&str>,
    links: &[crate::issues::IssueReference],
    known: &KnownChecks,
) -> GithubProjectSnapshot {
    let run = |arguments: &[&str]| gh_at(node, root, arguments);
    let mut pull_requests = match list_pull_requests(&run, known) {
        Ok(value) => value,
        Err(reason) if reason.category == GithubFailureCategory::NoGithubRemote => {
            return no_github_remote(root_path);
        }
        Err(reason) => {
            return GithubProjectSnapshot {
                root_path,
                status: failed(reason),
                ..GithubProjectSnapshot::default()
            };
        }
    };
    // The pull request list alone decides whether the read succeeds. The
    // account, the review asks and the merge queue each fill the judgments
    // derived from them, and a failed one leaves those unknown on every pull
    // request, never "no" (issue #1189, replacing PRD work-tab-pr-lens D-25's
    // all-or-nothing rule).
    let repository = issue_repository(node, root).map_err(GhFailure::network);
    let unread_queries = add_extra_facts(&run, repository, viewer, &mut pull_requests);
    match read_issues(node, root, links) {
        Ok((issues, warning)) => GithubProjectSnapshot {
            root_path,
            viewer: viewer.map(str::to_owned),
            issues,
            pull_requests,
            pull_requests_read: true,
            issues_read: true,
            status: GithubStatusSnapshot {
                available: true,
                loading: false,
                stale: false,
                last_success_at_unix_ms: Some(now_unix_ms()),
                unavailable_reason: warning
                    .as_ref()
                    .map(|reason| format!("Project metadata unavailable: {}", reason.reason)),
                failure_category: warning.map(|reason| reason.category),
                unread_queries,
            },
        },
        Err(reason) => GithubProjectSnapshot {
            root_path,
            viewer: viewer.map(str::to_owned),
            pull_requests,
            pull_requests_read: true,
            status: GithubStatusSnapshot {
                unread_queries,
                ..failed(reason)
            },
            ..Default::default()
        },
    }
}

/// Fills what the three extra questions tell about each pull request, and
/// returns the ones that went unanswered. Each is asked and judged on its own:
///
/// - the account decides "mine";
/// - the review-requested search, with the account and the request list,
///   decides whether a review was asked of the account or its team;
/// - the facts query decides the merge queue place and the time a review was
///   asked.
///
/// A judgment whose question failed stays unknown on every pull request, and
/// one the list itself answers is still made (a review request naming the
/// account, a request list with nobody who could be the account or its team).
fn add_extra_facts(
    run: &(impl Fn(&[&str]) -> Result<String, GhFailure> + Sync),
    repository: Result<String, GhFailure>,
    viewer: Option<&str>,
    pull_requests: &mut [PullRequestSnapshot],
) -> Vec<ExtraQuery> {
    let mut unread_queries = Vec::new();
    if viewer.is_none() {
        unread_queries.push(ExtraQuery::Account);
    }
    let asked = match read_review_asked(run) {
        Ok(asked) => Some(asked),
        Err(reason) => {
            unread(ExtraQuery::ReviewRequests, &reason.reason);
            unread_queries.push(ExtraQuery::ReviewRequests);
            None
        }
    };
    let facts = repository.and_then(|repository| read_facts(run, &repository));
    let facts = match facts {
        Ok(facts) => Some(facts),
        Err(reason) => {
            unread(ExtraQuery::MergeQueue, &reason.reason);
            unread_queries.push(ExtraQuery::MergeQueue);
            None
        }
    };
    for pull_request in pull_requests {
        let open = is_open(pull_request);
        let number = pull_request.number;
        let work = &mut pull_request.work;
        work.mine = viewer.map(|viewer| work.author.as_deref() == Some(viewer));
        work.asked_of_me = review_ask(
            open,
            &work.review_requests,
            viewer,
            asked.as_ref().map(|asked| asked.contains(&number)),
        );
        if let Some(facts) = &facts {
            let fact = facts.get(&number);
            work.queue = match fact.and_then(|fact| fact.queue_position) {
                Some(position) => crate::model::MergeQueuePlace::At(position),
                None => crate::model::MergeQueuePlace::NotQueued,
            };
            work.review_requested_at_unix_ms = fact.and_then(|fact| fact.review_requested_at);
        }
    }
    unread_queries
}

/// Whether a review was asked of the signed-in account or a team of it.
///
/// "Directly" is a review request naming the login; a request that `gh`'s
/// `review-requested:@me` search finds with no such name is the team's
/// (PRD work-tab-pr-lens D-15). `found` is whether that search found the pull
/// request, `None` when the search failed, and `viewer` is `None` when the
/// account is unknown. What the request list alone settles is settled: a
/// request naming the account is direct, and a list with no request that could
/// be the account (a team's, or any when the account is unknown) was asked of
/// neither.
fn review_ask(
    open: bool,
    requests: &[crate::model::ReviewRequest],
    viewer: Option<&str>,
    found: Option<bool>,
) -> crate::model::ReviewAsk {
    use crate::model::{ReviewAsk, ReviewRequestKind};
    if !open {
        return ReviewAsk::None;
    }
    let named = viewer.map(|viewer| {
        requests
            .iter()
            .any(|request| request.kind == ReviewRequestKind::User && request.name == viewer)
    });
    match (named, found) {
        (Some(true), _) => ReviewAsk::Direct,
        // The search finds every request made of the account or its teams.
        (_, Some(false)) => ReviewAsk::None,
        (Some(false), Some(true)) => ReviewAsk::Team,
        (Some(false), None) => {
            if requests
                .iter()
                .any(|request| request.kind == ReviewRequestKind::Team)
            {
                ReviewAsk::Unknown
            } else {
                ReviewAsk::None
            }
        }
        (None, Some(true)) => ReviewAsk::Unknown,
        (None, None) => {
            if requests.is_empty() {
                ReviewAsk::None
            } else {
                ReviewAsk::Unknown
            }
        }
    }
}

/// The numbers of the open pull requests `gh` finds a review asked for, of the
/// account or of a team of it.
fn read_review_asked(
    run: &(impl Fn(&[&str]) -> Result<String, GhFailure> + Sync),
) -> Result<std::collections::HashSet<u32>, GhFailure> {
    let asked = run(&[
        "pr",
        "list",
        "--state",
        "open",
        "--search",
        REVIEW_ASKED_SEARCH,
        "--limit",
        PULL_REQUEST_LIMIT,
        "--json",
        REVIEW_ASKED_FIELDS,
    ])?;
    parse_review_asked(&asked).map_err(GhFailure::network)
}

fn parse_review_asked(output: &str) -> Result<std::collections::HashSet<u32>, String> {
    #[derive(Deserialize)]
    struct Row {
        number: u32,
    }
    let rows: Vec<Row> = serde_json::from_str(output)
        .map_err(|error| format!("gh pr list returned review asks Hide could not read: {error}"))?;
    Ok(rows.into_iter().map(|row| row.number).collect())
}

/// What the facts query knows of one open pull request.
struct PullRequestFacts {
    queue_position: Option<u32>,
    review_requested_at: Option<u64>,
}

/// The newest open pull requests of the repository with their place in its
/// merge queue and when a review was last asked; a repository without a queue
/// has no positions.
fn read_facts(
    run: &(impl Fn(&[&str]) -> Result<String, GhFailure> + Sync),
    repository: &str,
) -> Result<HashMap<u32, PullRequestFacts>, GhFailure> {
    let query = hide_node_link::gh::pr_facts_query(repository).ok_or_else(|| {
        GhFailure::network("the repository name cannot be asked about its queue".to_owned())
    })?;
    let answer = run(&["api", "graphql", "-f", &query])?;
    parse_facts(&answer).map_err(GhFailure::network)
}

fn parse_facts(output: &str) -> Result<HashMap<u32, PullRequestFacts>, String> {
    #[derive(Deserialize)]
    struct Answer {
        data: Data,
    }
    #[derive(Deserialize)]
    struct Data {
        repository: Option<Repository>,
    }
    #[derive(Deserialize)]
    struct Repository {
        #[serde(rename = "pullRequests")]
        pull_requests: Nodes,
    }
    #[derive(Deserialize)]
    struct Nodes {
        nodes: Vec<Node>,
    }
    #[derive(Deserialize)]
    struct Node {
        number: u32,
        #[serde(rename = "mergeQueueEntry")]
        entry: Option<Entry>,
        #[serde(rename = "timelineItems")]
        asked: Option<Asked>,
    }
    #[derive(Deserialize)]
    struct Entry {
        position: u32,
    }
    #[derive(Deserialize)]
    struct Asked {
        nodes: Vec<AskedNode>,
    }
    #[derive(Deserialize)]
    struct AskedNode {
        #[serde(rename = "createdAt")]
        created_at: Option<String>,
    }
    let answer: Answer = serde_json::from_str(output)
        .map_err(|error| format!("gh returned a merge queue Hide could not read: {error}"))?;
    let repository = answer
        .data
        .repository
        .ok_or_else(|| "gh returned no repository for the merge queue".to_owned())?;
    Ok(repository
        .pull_requests
        .nodes
        .into_iter()
        .map(|node| {
            (
                node.number,
                PullRequestFacts {
                    queue_position: node.entry.map(|entry| entry.position),
                    review_requested_at: node
                        .asked
                        .and_then(|asked| asked.nodes.into_iter().next())
                        .and_then(|asked| asked.created_at)
                        .as_deref()
                        .and_then(parse_rfc3339_ms),
                },
            )
        })
        .collect())
}

/// Every pull request, with the checks of the open ones.
///
/// Two lists, asked for at the same time under their own 15-second limits:
/// the 200 newest pull requests of any state without their checks, and the
/// open ones with nothing but their checks. A settled pull request's checks
/// are not asked for again: the ones read while it was open (`known`, by
/// number and head commit) stay, and without them its checks are `Unknown`,
/// never a guess. Either list failing fails the read, so half an answer is
/// never taken for a repository with no pull requests.
fn list_pull_requests(
    run: &(impl Fn(&[&str]) -> Result<String, GhFailure> + Sync),
    known: &KnownChecks,
) -> Result<Vec<PullRequestSnapshot>, GhFailure> {
    let list = |state: &str, fields: &str| {
        run(&[
            "pr",
            "list",
            "--state",
            state,
            "--limit",
            PULL_REQUEST_LIMIT,
            "--json",
            fields,
        ])
    };
    let (every, open) = std::thread::scope(|scope| {
        let open = scope.spawn(|| list("open", OPEN_CHECK_FIELDS));
        let every = list("all", PULL_REQUEST_FIELDS);
        (every, open.join())
    });
    let every = every?;
    let open = open.map_err(|_| {
        GhFailure::network("gh open pull request checks reader failed".to_owned())
    })??;
    let mut pull_requests = parse_pull_requests(&every).map_err(GhFailure::network)?;
    let mut open_checks = parse_open_checks(&open).map_err(GhFailure::network)?;
    for pull_request in &mut pull_requests {
        // An open pull request's checks are the open list's or nothing: a
        // remembered value could be of a commit it has since moved past.
        if is_open(pull_request) {
            match open_checks.remove(&pull_request.number) {
                Some(open) => {
                    pull_request.checks = open.rollup;
                    pull_request.work.failed_checks = open.failed;
                    pull_request.work.pending_checks = open.pending;
                    pull_request.work.check_counts = open.counts;
                    pull_request.work.running_checks = open.running;
                }
                None => pull_request.checks = PullRequestChecks::Unknown,
            }
        } else {
            let head = pull_request.head_oid.clone();
            pull_request.checks = head
                .and_then(|head| known.get(&(pull_request.number, head)).copied())
                .unwrap_or(PullRequestChecks::Unknown);
        }
    }
    Ok(pull_requests)
}

/// The checks to keep for the next read: this answer's, or, when it never
/// got as far as a pull request list, the ones kept before it.
fn carried_checks(answer: &GithubProjectSnapshot, previous: KnownChecks) -> KnownChecks {
    if answer.pull_requests_read {
        remembered_checks(&answer.pull_requests)
    } else {
        previous
    }
}

/// Every check that was actually read, by number and head commit.
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

/// Optional Project fields may be inaccessible with otherwise valid issue
/// permissions. Retry exactly once without them, and retain the diagnostic.
fn with_optional_projects<T>(
    mut read: impl FnMut(bool) -> Result<T, GhFailure>,
) -> Result<(T, Option<GhFailure>), GhFailure> {
    match read(true) {
        Ok(value) => Ok((value, None)),
        Err(reason) => {
            crate::diagnostic!(
                serde_json::json!({"component":"github","kind":"project_metadata.unavailable","message":reason.reason})
            );
            read(false).map(|value| (value, Some(reason)))
        }
    }
}

fn read_issues(
    node: &dyn NodeLink,
    root: &Where,
    links: &[crate::issues::IssueReference],
) -> Result<(crate::issues::ProjectIssuesSnapshot, Option<GhFailure>), GhFailure> {
    let repository_json = gh_at(
        node,
        root,
        &[
            "repo",
            "view",
            "--json",
            hide_node_link::gh::REPOSITORY_FIELDS,
        ],
    )?;
    let repository: serde_json::Value = serde_json::from_str(&repository_json)
        .map_err(|error| GhFailure::network(format!("GitHub repository response: {error}")))?;
    // An empty repository has no default branch, which GitHub answers as
    // `null` or an empty name.
    let default_branch = repository
        .get("defaultBranchRef")
        .and_then(|reference| reference.get("name"))
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    let repository_id = repository
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    // `PUBLIC`, `PRIVATE` or `INTERNAL`; anything else is not known (D-35).
    let public = match repository
        .get("visibility")
        .and_then(serde_json::Value::as_str)
    {
        Some("PUBLIC") => Some(true),
        Some("PRIVATE" | "INTERNAL") => Some(false),
        other => {
            crate::diagnostic!(
                serde_json::json!({"component":"github","kind":"repository.visibility_unknown","value":other})
            );
            None
        }
    };
    let repository = repository
        .get("nameWithOwner")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| GhFailure::network("GitHub repository identity is missing".into()))?
        .to_owned();
    crate::issues::IssueReference::parse(&format!("{repository}#1"), None)
        .map_err(GhFailure::network)?;
    let (listed, mut warning) = with_optional_projects(|include_projects| {
        list_issues(&|arguments| gh_at(node, root, arguments), include_projects)
    })?;
    let mut overflow = listed.iter().filter(|issue| issue.state == "OPEN").count()
        > crate::issues::ISSUE_LIMIT
        || listed.iter().filter(|issue| issue.state != "OPEN").count() > crate::issues::ISSUE_LIMIT;
    let mut issues = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let links: Vec<_> = links
        .iter()
        .filter(|reference| seen.insert((*reference).clone()))
        .take(crate::issues::ISSUE_LIMIT)
        .collect();
    let missing: Vec<_> = links
        .iter()
        .copied()
        .filter(|reference| !listed.iter().any(|issue| &issue.reference == *reference))
        .collect();
    // Closed and cross-repository links are read together in one bounded
    // GraphQL query, rather than spawning one subprocess per card.
    let linked = if missing.is_empty() {
        Vec::new()
    } else {
        let (linked, linked_warning) = read_linked_issues(node, root, &missing)?;
        if warning.is_none() {
            warning = linked_warning;
        }
        linked
    };
    for reference in links {
        if let Some(issue) = listed
            .iter()
            .chain(linked.iter())
            .find(|issue| &issue.reference == reference)
        {
            issues.push(issue.clone());
        }
    }
    for issue in listed {
        if issues
            .iter()
            .any(|known| known.reference == issue.reference)
        {
            continue;
        }
        if issues
            .iter()
            .filter(|known| (known.state == "OPEN") == (issue.state == "OPEN"))
            .count()
            >= crate::issues::ISSUE_LIMIT
        {
            overflow = true;
            continue;
        }
        issues.push(issue);
    }
    // Every kept issue's blockers in one bounded query, never one per card. A
    // failure keeps each issue's earlier blockers (`ingest_github`) and is
    // said beside the tasks, not in place of them.
    let dependency_issues: Vec<_> = issues
        .iter()
        .filter(|issue| issue.state == "OPEN")
        .cloned()
        .collect();
    let dependencies_failure = match read_dependencies(node, root, &dependency_issues) {
        Ok(mut dependencies) => {
            for issue in &mut issues {
                let known = dependencies.remove(&issue.reference).unwrap_or_default();
                issue.blocked_by = known.blocked_by;
                issue.sub_issues = known.sub_issues;
            }
            None
        }
        Err(reason) => {
            crate::diagnostic!(serde_json::json!({
                "component": "github",
                "kind": "issue_dependencies.unavailable",
                "category": reason.category.english(),
                "message": reason.reason,
            }));
            Some(reason.reason)
        }
    };
    Ok((
        crate::issues::ProjectIssuesSnapshot {
            repository: Some(repository),
            issues,
            overflow,
            dependencies_failure,
            repository_id,
            default_branch,
            public,
        },
        warning,
    ))
}

/// The open issues, most recently changed first (an ordinary `gh issue list`
/// sorts by creation), with their labels in the same answer, so every card
/// shows them without a read of its own. One more than `ISSUE_LIMIT` is asked
/// for, so the sentinel proves overflow.
fn list_issues(
    run: &impl Fn(&[&str]) -> Result<String, GhFailure>,
    include_projects: bool,
) -> Result<Vec<crate::issues::IssueSnapshot>, GhFailure> {
    let fields = if include_projects {
        ISSUE_LIST_PROJECT_FIELDS
    } else {
        ISSUE_LIST_FIELDS
    };
    let mut issues = Vec::new();
    // Independent bounds preserve the open list when recent closed work is
    // busy. Each sentinel is retained until read_issues reports overflow.
    for state in ["open", "closed"] {
        let output = run(&[
            "issue",
            "list",
            "--state",
            state,
            "--limit",
            ISSUE_LIST_LIMIT,
            "--search",
            ISSUE_LIST_SEARCH,
            "--json",
            fields,
        ])?;
        issues.extend(crate::issues::parse_issues(&output).map_err(GhFailure::network)?);
    }
    Ok(issues)
}

/// How many labels of one issue the linked-issue query reads, the number
/// `gh issue list --json labels` asks GitHub for.
const LABELS_PER_ISSUE: usize = 100;

/// How many blockers of one issue a read keeps; GitHub orders them, and an
/// issue blocked by more than this is drawn with the first ones.
const BLOCKERS_PER_ISSUE: usize = 20;

/// How many sub-issues of one issue a read lists: GitHub's own limit per
/// issue, so the list is never shorter than the count GitHub reports.
const SUB_ISSUES_PER_ISSUE: usize = 100;

/// What GitHub relates to one issue besides its pull requests.
#[derive(Default)]
struct IssueDependencies {
    blocked_by: Vec<crate::issues::IssueReference>,
    sub_issues: crate::issues::SubIssuesSnapshot,
}

type Dependencies = std::collections::BTreeMap<crate::issues::IssueReference, IssueDependencies>;

/// The open issues blocking each of `issues` (`Issue.blockedBy`) and their
/// sub-issues (`Issue.subIssuesSummary`, `Issue.subIssues`), read in one
/// GraphQL query grouped by repository. A closed blocker no longer blocks and
/// is left out.
fn read_dependencies(
    node: &dyn NodeLink,
    root: &Where,
    issues: &[crate::issues::IssueSnapshot],
) -> Result<Dependencies, GhFailure> {
    if issues.is_empty() {
        return Ok(Dependencies::new());
    }
    let query = dependency_query(issues)?;
    let output = gh_at(
        node,
        root,
        &["api", "graphql", "-f", &format!("query={query}")],
    )?;
    parse_dependencies(&output)
}

fn dependency_query(issues: &[crate::issues::IssueSnapshot]) -> Result<String, GhFailure> {
    let mut repositories: std::collections::BTreeMap<&str, Vec<u32>> =
        std::collections::BTreeMap::new();
    for issue in issues {
        repositories
            .entry(issue.reference.repository.as_str())
            .or_default()
            .push(issue.reference.number);
    }
    let mut query = String::from("query HideIssueDependencies {");
    for (index, (repository, numbers)) in repositories.iter().enumerate() {
        let valid = crate::issues::IssueReference::parse(&format!("{repository}#1"), None)
            .map_err(GhFailure::network)?;
        let (owner, name) = valid
            .repository
            .split_once('/')
            .expect("validated repository");
        query.push_str(&format!(
            "r{index}:repository(owner:\"{owner}\",name:\"{name}\"){{nameWithOwner"
        ));
        for number in numbers {
            query.push_str(&format!(" i{number}:issue(number:{number}){{number blockedBy(first:{BLOCKERS_PER_ISSUE}){{nodes{{number state repository{{nameWithOwner}}}}}} subIssuesSummary{{total completed}} subIssues(first:{SUB_ISSUES_PER_ISSUE}){{nodes{{number title state repository{{nameWithOwner}}}}}}}}"));
        }
        query.push('}');
    }
    query.push('}');
    Ok(query)
}

fn parse_dependencies(output: &str) -> Result<Dependencies, GhFailure> {
    let answer: serde_json::Value =
        serde_json::from_str(output).map_err(|error| GhFailure::network(error.to_string()))?;
    if answer.get("errors").is_some() {
        return Err(GhFailure::network(
            "GitHub could not read the issue dependencies".into(),
        ));
    }
    let data = answer
        .get("data")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| GhFailure::network("GitHub issue dependency response has no data".into()))?;
    let reference = |repository: &str, number: &serde_json::Value| {
        let number = number.as_u64().ok_or_else(|| {
            GhFailure::network("GitHub returned an issue without a number".into())
        })?;
        crate::issues::IssueReference::parse(&format!("{repository}#{number}"), None)
            .map_err(GhFailure::network)
    };
    let mut dependencies = Dependencies::new();
    for repository in data.values() {
        let Some(fields) = repository.as_object() else {
            continue;
        };
        let name = fields
            .get("nameWithOwner")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                GhFailure::network("GitHub dependency response names no repository".into())
            })?;
        for issue in fields.values().filter(|value| value.is_object()) {
            let blocked = reference(name, &issue["number"])?;
            let mut open = Vec::new();
            for node in issue
                .pointer("/blockedBy/nodes")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                if node["state"] != "OPEN" {
                    continue;
                }
                let owner = node
                    .pointer("/repository/nameWithOwner")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(name);
                open.push(reference(owner, &node["number"])?);
            }
            let count = |field: &str| {
                issue
                    .pointer(&format!("/subIssuesSummary/{field}"))
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|count| u32::try_from(count).ok())
                    .ok_or_else(|| {
                        GhFailure::network(
                            "GitHub returned an issue without sub-issue counts".into(),
                        )
                    })
            };
            let mut listed = Vec::new();
            for node in issue
                .pointer("/subIssues/nodes")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                let owner = node
                    .pointer("/repository/nameWithOwner")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(name);
                listed.push(crate::issues::SubIssueSnapshot {
                    reference: reference(owner, &node["number"])?,
                    title: node["title"].as_str().unwrap_or_default().to_owned(),
                    open: node["state"] == "OPEN",
                });
            }
            let sub_issues = crate::issues::SubIssuesSnapshot {
                total: count("total")?,
                completed: count("completed")?,
                listed,
            };
            dependencies.insert(
                blocked,
                IssueDependencies {
                    blocked_by: open,
                    sub_issues,
                },
            );
        }
    }
    Ok(dependencies)
}

pub(crate) fn read_linked_issue(
    node: &dyn NodeLink,
    place: &Where,
    reference: &crate::issues::IssueReference,
) -> Result<crate::issues::IssueSnapshot, String> {
    read_linked_issues(node, place, &[reference])
        .map_err(|error| error.reason)?
        .0
        .into_iter()
        .next()
        .ok_or_else(|| "GitHub issue was not found".to_owned())
}

/// Creates an issue in the repository `place` names (the backlog's add, and
/// a pull request's 새 이슈 만들기), one of Hide's writes to GitHub with
/// `write_closing_line` and `close_issue`. `gh issue create` prints the new
/// issue's URL, which names it.
pub(crate) fn create_issue(
    node: &dyn NodeLink,
    place: &Where,
    title: &str,
    body: &str,
) -> Result<crate::issues::IssueSnapshot, String> {
    create_issue_failing(node, place, title, body).map_err(|failure| match failure {
        CreateFailure::Gh(failure) => failure.reason,
        CreateFailure::Unreadable(reason) => reason,
    })
}

/// Why a create did not name its issue: `gh` failed, or it answered with no
/// issue Hide could read, which may still have been made.
#[derive(Clone, Debug)]
pub(crate) enum CreateFailure {
    Gh(GhFailure),
    Unreadable(String),
}

/// [`create_issue`] keeping why `gh` failed, which the backlog shows.
pub(crate) fn create_issue_failing(
    node: &dyn NodeLink,
    place: &Where,
    title: &str,
    body: &str,
) -> Result<crate::issues::IssueSnapshot, CreateFailure> {
    let output = gh_at(
        node,
        place,
        &["issue", "create", "--title", title, "--body", body],
    )
    .map_err(CreateFailure::Gh)?;
    let url = output
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with("https://github.com/"))
        .ok_or_else(|| {
            CreateFailure::Unreadable(format!(
                "gh issue create did not print the new issue's URL: {}",
                output.trim()
            ))
        })?;
    let reference =
        crate::issues::IssueReference::parse(url, None).map_err(CreateFailure::Unreadable)?;
    Ok(crate::issues::IssueSnapshot {
        reference,
        title: title.to_owned(),
        url: url.to_owned(),
        state: "OPEN".into(),
        project_status: None,
        updated_at_unix_ms: Some(now_unix_ms()),
        created_at_unix_ms: None,
        closed_at_unix_ms: None,
        blocked_by: Vec::new(),
        sub_issues: Default::default(),
        labels: Vec::new(),
        author: None,
        assignees: Vec::new(),
    })
}

pub(crate) fn issue_repository(node: &dyn NodeLink, place: &Where) -> Result<String, String> {
    let output = gh_at(node, place, &["repo", "view", "--json", "nameWithOwner"])
        .map_err(|_| "issue_github_unavailable")?;
    let value: serde_json::Value =
        serde_json::from_str(&output).map_err(|_| "issue_repository_invalid")?;
    value
        .get("nameWithOwner")
        .and_then(serde_json::Value::as_str)
        .filter(|name| crate::issues::is_repository(name))
        .map(str::to_owned)
        .ok_or_else(|| "issue_repository_invalid".into())
}

/// Reconcile an uncertain create through the repository connection, not
/// the eventually updated search index. Crossing the page bound refuses a
/// new create unless this page already proves the attempted time is covered.
pub(crate) fn reconcile_issue_attempt(
    node: &dyn NodeLink,
    place: &Where,
    repository: &str,
    title: &str,
    since: u64,
    claimed: &[u64],
) -> Result<Option<crate::issues::IssueSnapshot>, String> {
    let query =
        hide_node_link::gh::issue_attempt_query(repository).ok_or("issue_repository_invalid")?;
    let output = gh_at(node, place, &["api", "graphql", "-f", &query])
        .map_err(|_| "issue_github_unavailable")?;
    let value: serde_json::Value =
        serde_json::from_str(&output).map_err(|_| "issue_reconcile_invalid")?;
    if value
        .get("errors")
        .is_some_and(|errors| errors.as_array().is_none_or(|errors| !errors.is_empty()))
    {
        return Err("issue_reconcile_failed".into());
    }
    let connection = value
        .pointer("/data/repository/issues")
        .ok_or("issue_reconcile_invalid")?;
    let nodes = connection
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .ok_or("issue_reconcile_invalid")?;
    if nodes.len() > 100 {
        return Err("issue_reconcile_capacity".into());
    }
    let more = connection
        .pointer("/pageInfo/hasNextPage")
        .and_then(serde_json::Value::as_bool)
        .ok_or("issue_reconcile_invalid")?;
    // GitHub timestamps have second precision. Include the attempted second.
    let since = since / 1000 * 1000;
    let mut oldest = u64::MAX;
    let mut found = None;
    for node in nodes {
        let created = node
            .get("createdAt")
            .and_then(serde_json::Value::as_str)
            .and_then(parse_rfc3339_ms)
            .ok_or("issue_reconcile_invalid")?;
        oldest = oldest.min(created);
        let authored = node
            .get("viewerDidAuthor")
            .and_then(serde_json::Value::as_bool)
            .ok_or("issue_reconcile_invalid")?;
        if created < since
            || !authored
            || node.get("title").and_then(serde_json::Value::as_str) != Some(title)
            || node
                .get("number")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|number| claimed.contains(&number))
        {
            continue;
        }
        let url = node
            .get("url")
            .and_then(serde_json::Value::as_str)
            .ok_or("issue_reconcile_invalid")?;
        let reference = crate::issues::IssueReference::parse(url, None)?;
        if !reference.repository.eq_ignore_ascii_case(repository)
            || node.get("number").and_then(serde_json::Value::as_u64)
                != Some(u64::from(reference.number))
        {
            return Err("issue_reconcile_identity_changed".into());
        }
        if found.is_some() {
            return Err("issue_reconcile_ambiguous".into());
        }
        found = Some(crate::issues::IssueSnapshot {
            reference,
            title: title.into(),
            url: url.into(),
            state: node
                .get("state")
                .and_then(serde_json::Value::as_str)
                .filter(|state| ["OPEN", "CLOSED"].contains(state))
                .ok_or("issue_reconcile_invalid")?
                .into(),
            project_status: None,
            updated_at_unix_ms: Some(now_unix_ms()),
            created_at_unix_ms: Some(created),
            closed_at_unix_ms: None,
            blocked_by: Vec::new(),
            sub_issues: Default::default(),
            labels: Vec::new(),
            author: None,
            assignees: Vec::new(),
        });
    }
    if found.is_none() && more && oldest >= since {
        crate::diagnostic!(
            serde_json::json!({"component":"session_work","kind":"issue_reconcile.capacity","limit":100})
        );
        return Err("issue_reconcile_capacity".into());
    }
    Ok(found)
}

/// One issue as its panel reads it when it opens (PRD overview-lenses-issues
/// D-40), and the Start dialog's first prompt: the body, labels, author,
/// assignees and an oldest-first page of actual comments, in one fixed
/// GraphQL read on the caller's worker (overview-redesign B14).
pub(crate) fn issue_detail(
    node: &dyn NodeLink,
    place: &Where,
    reference: &crate::issues::IssueReference,
    after: Option<&str>,
) -> Result<crate::tasks::TaskDetail, String> {
    let arguments =
        hide_node_link::gh::issue_detail_arguments(&reference.repository, reference.number, after)
            .ok_or_else(|| "The issue detail cursor is invalid".to_owned())?;
    let output = gh_at(
        node,
        place,
        &arguments.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .map_err(|error| error.reason)?;
    let detail = parse_issue_detail(&output)?;
    if detail.comments_has_more && after.is_some() && detail.comments_cursor.as_deref() == after {
        return Err("GitHub returned a comment cursor that did not advance".into());
    }
    Ok(detail)
}

fn parse_issue_detail(output: &str) -> Result<crate::tasks::TaskDetail, String> {
    #[derive(serde::Deserialize)]
    struct Person {
        login: String,
    }
    #[derive(serde::Deserialize)]
    struct Label {
        name: String,
        #[serde(default)]
        color: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct Comment {
        #[serde(default)]
        author: Option<Person>,
        #[serde(default, rename = "createdAt")]
        created_at: Option<String>,
        #[serde(default)]
        body: String,
    }
    #[derive(serde::Deserialize)]
    struct Nodes<T> {
        nodes: Vec<T>,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PageInfo {
        end_cursor: Option<String>,
        has_next_page: bool,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Comments {
        total_count: u32,
        nodes: Vec<Comment>,
        page_info: PageInfo,
    }
    #[derive(serde::Deserialize)]
    struct Viewed {
        body: String,
        labels: Nodes<Label>,
        #[serde(default)]
        author: Option<Person>,
        assignees: Nodes<Person>,
        comments: Comments,
        #[serde(default, rename = "createdAt")]
        created_at: Option<String>,
    }
    let answer: serde_json::Value = serde_json::from_str(output)
        .map_err(|error| format!("GitHub issue detail could not be read: {error}"))?;
    if answer.get("errors").is_some() {
        return Err("GitHub could not read the issue detail".into());
    }
    let viewed: Viewed = serde_json::from_value(
        answer
            .pointer("/data/repository/issue")
            .cloned()
            .ok_or_else(|| "GitHub returned no issue detail".to_owned())?,
    )
    .map_err(|error| format!("GitHub issue detail could not be read: {error}"))?;
    let limited = viewed
        .comments
        .nodes
        .iter()
        .any(|comment| comment.body.chars().count() > crate::tasks::COMMENT_BODY_LIMIT);
    if viewed.comments.nodes.len() > crate::tasks::DETAIL_COMMENTS
        || (viewed.comments.page_info.has_next_page
            && !viewed
                .comments
                .page_info
                .end_cursor
                .as_deref()
                .is_some_and(hide_node_link::gh::issue_detail_cursor_valid))
    {
        return Err("GitHub returned an invalid comment page".into());
    }
    Ok(crate::tasks::TaskDetail {
        body: viewed.body,
        labels: viewed
            .labels
            .nodes
            .into_iter()
            .map(|label| crate::tasks::TaskLabel::from_source(label.name, label.color))
            .collect(),
        author: viewed.author.map(|person| person.login),
        created_at_unix_ms: viewed.created_at.as_deref().and_then(parse_rfc3339_ms),
        assignees: viewed
            .assignees
            .nodes
            .into_iter()
            .map(|person| person.login)
            .collect(),
        comment_count: Some(viewed.comments.total_count),
        comments_cursor: viewed.comments.page_info.end_cursor,
        comments_has_more: viewed.comments.page_info.has_next_page,
        comments_limited: limited,
        comments: viewed
            .comments
            .nodes
            .into_iter()
            .map(|comment| crate::tasks::TaskComment {
                author: comment.author.map(|person| person.login),
                created_at_unix_ms: comment.created_at.as_deref().and_then(parse_rfc3339_ms),
                body: crate::tasks::capped_comment(&comment.body),
            })
            .collect(),
    })
}

#[derive(Clone, Copy)]
enum SearchKind {
    Pr,
    Issue,
}

impl SearchKind {
    fn subcommand(self) -> &'static str {
        match self {
            Self::Pr => "prs",
            Self::Issue => "issues",
        }
    }

    fn fields(self) -> &'static str {
        match self {
            Self::Pr => SEARCH_PR_FIELDS,
            Self::Issue => SEARCH_ISSUE_FIELDS,
        }
    }

    fn wire(self) -> &'static str {
        match self {
            Self::Pr => "pr",
            Self::Issue => "issue",
        }
    }
}

/// A project a search may cover: where it lives, and its `owner/name` when
/// the GitHub reader has already resolved it.
#[derive(Clone, Debug)]
pub(crate) struct SearchTarget {
    pub(crate) root: std::path::PathBuf,
    pub(crate) repository: Option<String>,
}

/// Searches pull requests and issues for `query` in each target's
/// repository, on the caller's worker. A project the reader has not read yet
/// is resolved here with the same `gh repo view` the reader runs, so the
/// search never depends on what was read earlier. See `search_with`.
pub(crate) fn search(
    node: &dyn NodeLink,
    targets: &[SearchTarget],
    query: &str,
) -> Result<Vec<crate::model::GithubSearchResult>, String> {
    let repositories = resolve_repositories(|cwd, arguments| gh(node, cwd, arguments), targets)?;
    search_with(|arguments| gh(node, None, arguments), &repositories, query)
}

/// The targets' repositories, once each by `owner/name` (GitHub names ignore
/// case), in order. A project with no GitHub remote is left out; a project
/// whose repository could not be read for any other reason fails the search,
/// which then cannot say it covered every project.
fn resolve_repositories(
    run: impl Fn(Option<&Path>, &[&str]) -> Result<String, GhFailure>,
    targets: &[SearchTarget],
) -> Result<Vec<String>, String> {
    let mut repositories: Vec<String> = Vec::new();
    for target in targets {
        let repository = match &target.repository {
            Some(repository) => repository.clone(),
            None => match run(
                Some(&target.root),
                &["repo", "view", "--json", "nameWithOwner"],
            )
            .and_then(|output| {
                serde_json::from_str::<serde_json::Value>(&output)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("nameWithOwner")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    })
                    .filter(|name| hide_node_link::gh::is_repository(name))
                    .ok_or_else(|| GhFailure::network("repository identity is missing".into()))
            }) {
                Ok(repository) => repository,
                // A project with no GitHub remote is not one to search; any
                // other failure means the search cannot say it covered it.
                Err(failure) if failure.category == GithubFailureCategory::NoGithubRemote => {
                    continue;
                }
                Err(failure) => {
                    return Err(format!(
                        "gh repo view in {}: {}",
                        target.root.display(),
                        failure.reason
                    ));
                }
            },
        };
        if !repositories
            .iter()
            .any(|known| known.eq_ignore_ascii_case(&repository))
        {
            repositories.push(repository);
        }
    }
    Ok(repositories)
}

/// Searches pull requests and issues for `query` in all the repositories at
/// once: one `gh search prs` and one `gh search issues`, however many
/// repositories there are (GitHub's search allows about 30 calls a minute).
/// The query goes as separate words, so it matches the way a search box does,
/// not as one quoted phrase. At most `SEARCH_LIMIT` of each come back, pull
/// requests first. A failed call fails the whole search, naming the call.
fn search_with(
    run: impl Fn(&[&str]) -> Result<String, GhFailure>,
    repositories: &[String],
    query: &str,
) -> Result<Vec<crate::model::GithubSearchResult>, String> {
    if repositories.is_empty() {
        return Ok(Vec::new());
    }
    let limit = SEARCH_LIMIT.to_string();
    let mut found: Vec<Vec<crate::model::GithubSearchResult>> = Vec::new();
    for kind in [SearchKind::Pr, SearchKind::Issue] {
        let mut arguments = vec!["search", kind.subcommand()];
        for repository in repositories {
            arguments.extend(["--repo", repository]);
        }
        arguments.extend(["--limit", &limit, "--json", kind.fields(), "--"]);
        arguments.extend(query.split_whitespace());
        let output = run(&arguments)
            .map_err(|error| format!("gh search {}: {}", kind.subcommand(), error.reason))?;
        let mut hits = parse_search(&output, kind, repositories)?;
        hits.truncate(SEARCH_LIMIT);
        found.push(hits);
    }
    Ok(found.into_iter().flatten().collect())
}

/// One `gh search` answer. A hit outside `repositories` is dropped: a
/// `repo:` qualifier in the query widens GitHub's search to that repository
/// too, and a search only covers the projects registered here.
fn parse_search(
    output: &str,
    kind: SearchKind,
    repositories: &[String],
) -> Result<Vec<crate::model::GithubSearchResult>, String> {
    #[derive(Deserialize)]
    struct Found {
        number: u32,
        title: String,
        state: String,
        url: String,
        #[serde(default, rename = "isDraft")]
        is_draft: bool,
        repository: FoundRepository,
    }
    #[derive(Deserialize)]
    struct FoundRepository {
        #[serde(rename = "nameWithOwner")]
        name_with_owner: String,
    }
    let found = serde_json::from_str::<Vec<Found>>(output).map_err(|error| {
        format!(
            "gh search {} returned output Hide could not read: {error}",
            kind.subcommand()
        )
    })?;
    let mut results = Vec::new();
    for hit in found {
        let repository = hit.repository.name_with_owner;
        if !repositories
            .iter()
            .any(|known| known.eq_ignore_ascii_case(&repository))
        {
            continue;
        }
        let state = hit.state.to_ascii_lowercase();
        if !matches!(state.as_str(), "open" | "closed" | "merged") {
            return Err(format!(
                "gh search {} returned the state {:?}",
                kind.subcommand(),
                hit.state
            ));
        }
        // The link is drawn from data, so only a github.com address passes.
        if !hit.url.starts_with("https://github.com/") {
            return Err(format!(
                "gh search {} returned a link that is not on github.com",
                kind.subcommand()
            ));
        }
        results.push(crate::model::GithubSearchResult {
            kind: kind.wire().into(),
            repository,
            number: hit.number,
            title: hit.title,
            url: hit.url,
            pr_state: matches!(kind, SearchKind::Pr).then(|| {
                crate::model::PrState::of(
                    badge(&state, hit.is_draft, None),
                    hit.is_draft,
                    crate::model::PullRequestChecks::Unknown,
                    None,
                )
            }),
            state,
        });
    }
    Ok(results)
}

/// What a pull request says for the work handed on from it (PRD
/// overview-lenses-prs): its body, which a new issue made from it starts
/// with (B12), and its failed checks and the reviews still asking for
/// changes, which an agent it is handed to starts from (D-46).
#[derive(Debug)]
pub(crate) struct PrFeedback {
    pub(crate) body: String,
    pub(crate) failed_checks: Vec<crate::model::PrFailedCheck>,
    pub(crate) change_requests: Vec<crate::model::PrChangeRequest>,
}

/// `PrFeedback`, in one `gh pr view` at `place` on the caller's worker.
pub(crate) fn pr_feedback(
    node: &dyn NodeLink,
    place: &Where,
    number: u32,
) -> Result<PrFeedback, String> {
    let number = number.to_string();
    let output = gh_at(
        node,
        place,
        &["pr", "view", &number, "--json", PR_FEEDBACK_FIELDS],
    )
    .map_err(|error| error.reason)?;
    parse_pr_feedback(&output)
}

fn parse_pr_feedback(output: &str) -> Result<PrFeedback, String> {
    #[derive(serde::Deserialize)]
    struct Person {
        login: String,
    }
    #[derive(serde::Deserialize)]
    struct Review {
        #[serde(default)]
        author: Option<Person>,
        #[serde(default)]
        state: String,
        #[serde(default)]
        body: String,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Viewed {
        // All three are what was asked for: an answer without them is no
        // answer, never a pull request with nothing failing.
        body: String,
        status_check_rollup: Vec<GhCheck>,
        reviews: Vec<Review>,
    }
    let viewed = serde_json::from_str::<Viewed>(output)
        .map_err(|error| format!("gh pr view returned output Hide could not read: {error}"))?;
    let failed = viewed
        .status_check_rollup
        .iter()
        .filter_map(GhCheck::failure)
        .collect();
    // A reviewer's change request stands until that reviewer's next review
    // approves or dismisses it; a comment-only review leaves it standing, as
    // GitHub's own review decision reads it. Reviews come oldest first.
    let mut standing: Vec<(Option<String>, Review)> = Vec::new();
    for review in viewed.reviews {
        let author = review.author.as_ref().map(|person| person.login.clone());
        match review.state.as_str() {
            "CHANGES_REQUESTED" => {
                standing.retain(|(known, _)| *known != author);
                standing.push((author, review));
            }
            "APPROVED" | "DISMISSED" => standing.retain(|(known, _)| *known != author),
            _ => {}
        }
    }
    let requests = standing
        .into_iter()
        .map(|(author, review)| crate::model::PrChangeRequest {
            author,
            body: review.body.trim().to_owned(),
        })
        .collect();
    Ok(PrFeedback {
        body: viewed.body,
        failed_checks: failed,
        change_requests: requests,
    })
}

/// The log of CI job `job`'s failed steps, which the node answers cut to its
/// end (`hide_node_link::gh::JOB_LOG_TAIL_BYTES`), for a Hide AI suggestion
/// investigation's CI read (PRD hide-ai-suggestions B7).
pub(crate) fn failed_job_log(
    node: &dyn NodeLink,
    place: &Where,
    job: u64,
) -> Result<String, GhFailure> {
    let job = job.to_string();
    gh_at(node, place, &["run", "view", "--job", &job, "--log-failed"])
}

/// Writes `Closes #issue` at the end of pull request `number`'s body so
/// GitHub closes the issue when it merges (PRD overview-lenses-prs D-13,
/// D-47): the body is read first, and a body that already closes the issue
/// is left as it is, so a retry never adds a second line. This is Hide's one
/// write to a pull request; `place` is where `gh` reads its repository.
pub(crate) fn write_closing_line(
    node: &dyn NodeLink,
    place: &Where,
    number: u32,
    issue: u32,
) -> Result<bool, String> {
    write_closing_line_with(|arguments| gh_at(node, place, arguments), number, issue)
}

/// `write_closing_line` through `gh`, which a test answers with a fixture.
fn write_closing_line_with(
    gh: impl Fn(&[&str]) -> Result<String, GhFailure>,
    number: u32,
    issue: u32,
) -> Result<bool, String> {
    let number = number.to_string();
    let output = gh(&["pr", "view", &number, "--json", "body"]).map_err(|error| error.reason)?;
    #[derive(serde::Deserialize)]
    struct Viewed {
        #[serde(default)]
        body: String,
    }
    let body = serde_json::from_str::<Viewed>(&output)
        .map_err(|error| format!("gh pr view returned output Hide could not read: {error}"))?
        .body;
    if closes_issue(&body, issue) {
        return Ok(false);
    }
    let next = with_closing_line(&body, issue);
    gh(&["pr", "edit", &number, "--body", &next]).map_err(|error| error.reason)?;
    Ok(true)
}

/// The keywords GitHub reads as closing an issue from a pull request's body.
const CLOSING_KEYWORDS: [&str; 9] = [
    "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
];

/// Whether `body` already closes issue `issue` of the pull request's own
/// repository: a closing keyword followed by `#N`, `owner/repo#N` or the
/// issue's URL. GitHub's own rule is prose, so this reads it word by word at
/// this one boundary.
pub(crate) fn closes_issue(body: &str, issue: u32) -> bool {
    let words: Vec<&str> = body.split_whitespace().collect();
    words.windows(2).any(|pair| {
        let keyword = pair[0].trim_end_matches(':').to_ascii_lowercase();
        CLOSING_KEYWORDS.contains(&keyword.as_str()) && referenced_issue(pair[1]) == Some(issue)
    })
}

/// The issue number a word names: `#12`, `owner/repo#12` or
/// `https://github.com/owner/repo/issues/12`, trailing punctuation aside.
fn referenced_issue(word: &str) -> Option<u32> {
    let word = word.trim_end_matches(['.', ',', ';', ':', ')', '!', '?']);
    let number = if let Some(rest) = word.strip_prefix("https://github.com/") {
        rest.split_once("/issues/")?.1
    } else {
        let (repository, number) = word.rsplit_once('#')?;
        if !(repository.is_empty() || repository.split('/').count() == 2) {
            return None;
        }
        number
    };
    if !is_number(number) {
        return None;
    }
    number.parse().ok()
}

/// `body` with `Closes #issue` on its own line after a blank line.
fn with_closing_line(body: &str, issue: u32) -> String {
    let body = body.trim_end();
    if body.is_empty() {
        format!("Closes #{issue}")
    } else {
        format!("{body}\n\nCloses #{issue}")
    }
}

fn read_linked_issues(
    node: &dyn NodeLink,
    root: &Where,
    links: &[&crate::issues::IssueReference],
) -> Result<(Vec<crate::issues::IssueSnapshot>, Option<GhFailure>), GhFailure> {
    with_optional_projects(|include_projects| {
        let query = issue_query(links, include_projects)?;
        let output = gh_at(
            node,
            root,
            &["api", "graphql", "-f", &format!("query={query}")],
        )?;
        parse_linked_issues(&output)
    })
}

fn issue_query(
    links: &[&crate::issues::IssueReference],
    include_projects: bool,
) -> Result<String, GhFailure> {
    let mut query = String::from("query HideLinkedIssues {");
    let projects = if include_projects {
        " projectItems(first:100){nodes{status:fieldValueByName(name:\"Status\"){... on ProjectV2ItemFieldSingleSelectValue{name}}}}"
    } else {
        ""
    };
    for (index, reference) in links.iter().enumerate() {
        let valid = crate::issues::IssueReference::parse(&reference.token(), None)
            .map_err(GhFailure::network)?;
        let (owner, name) = valid
            .repository
            .split_once('/')
            .expect("validated repository");
        query.push_str(&format!("r{index}:repository(owner:\"{owner}\",name:\"{name}\"){{issue(number:{}){{number title url state updatedAt createdAt closedAt author{{login}} assignees(first:100){{nodes{{login}}}} labels(first:{LABELS_PER_ISSUE}){{nodes{{name color}}}}{projects}}}}}", valid.number));
    }
    query.push('}');
    Ok(query)
}

/// Each linked issue GitHub answered. A link whose repository or issue does
/// not resolve (gone, renamed away, private to this login, or never real) is
/// `null` beside a `NOT_FOUND` error while the others are answered, so it is
/// left out and the rest are kept: one such link once failed the project's
/// whole issue read. Any other error fails the read.
fn parse_linked_issues(output: &str) -> Result<Vec<crate::issues::IssueSnapshot>, GhFailure> {
    let answer: serde_json::Value =
        serde_json::from_str(output).map_err(|error| GhFailure::network(error.to_string()))?;
    if let Some(errors) = answer.get("errors") {
        let not_found = |error: &serde_json::Value| {
            error.get("type").and_then(serde_json::Value::as_str) == Some("NOT_FOUND")
        };
        if !errors
            .as_array()
            .is_some_and(|errors| errors.iter().all(not_found))
        {
            return Err(GhFailure::network(
                "GitHub could not resolve the linked issues".into(),
            ));
        }
    }
    let data = answer
        .get("data")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| GhFailure::network("GitHub linked issue response has no data".into()))?;
    let mut issues = Vec::new();
    let mut unavailable = 0_usize;
    for repository in data.values() {
        let Some(mut issue) = repository
            .get("issue")
            .filter(|issue| !issue.is_null())
            .cloned()
        else {
            unavailable += 1;
            continue;
        };
        // GraphQL nests a connection's items under `nodes`; `gh issue list`
        // prints them as the list itself, which `parse_issues` reads.
        for connection in ["projectItems", "labels", "assignees"] {
            let items = issue
                .pointer(&format!("/{connection}/nodes"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!([]));
            issue[connection] = items;
        }
        issues.push(issue);
    }
    if unavailable > 0 {
        crate::diagnostic!(
            serde_json::json!({"component":"github","kind":"linked_issue.unavailable","count":unavailable})
        );
    }
    crate::issues::parse_issues(&serde_json::Value::Array(issues).to_string())
        .map_err(GhFailure::network)
}

/// A repository with nothing GitHub can answer for: an answer, not a failure
/// (design principle 13), so it is neither stale nor retried, and the screen
/// draws no GitHub state for it.
fn no_github_remote(root_path: String) -> GithubProjectSnapshot {
    GithubProjectSnapshot {
        root_path,
        status: GithubStatusSnapshot {
            available: true,
            failure_category: Some(GithubFailureCategory::NoGithubRemote),
            ..GithubStatusSnapshot::default()
        },
        ..GithubProjectSnapshot::default()
    }
}

fn failed(reason: GhFailure) -> GithubStatusSnapshot {
    GithubStatusSnapshot {
        available: true,
        loading: false,
        stale: true,
        last_success_at_unix_ms: None,
        unavailable_reason: Some(reason.reason),
        failure_category: Some(reason.category),
        unread_queries: Vec::new(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPullRequest {
    title: String,
    number: u32,
    head_ref_name: String,
    #[serde(default)]
    head_ref_oid: Option<String>,
    #[serde(default)]
    is_cross_repository: bool,
    base_ref_name: String,
    state: String,
    review_decision: Option<String>,
    is_draft: bool,
    url: String,
    merged_at: Option<String>,
    updated_at: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    closed_at: Option<String>,
    #[serde(default)]
    closing_issues_references: Vec<GhIssueReference>,
    #[serde(default)]
    author: Option<GhAuthor>,
    #[serde(default)]
    merge_state_status: Option<String>,
    #[serde(default)]
    review_requests: Vec<GhReviewRequest>,
}

#[derive(Debug, Deserialize)]
struct GhAuthor {
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    is_bot: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum GhReviewRequest {
    User {
        login: String,
    },
    Team {
        #[serde(default)]
        slug: Option<String>,
        #[serde(default)]
        name: Option<String>,
    },
    #[serde(other)]
    Other,
}

/// One row of the open list: a number and its checks, nothing else.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhOpenChecks {
    number: u32,
    status_check_rollup: Option<Vec<GhCheck>>,
}

#[derive(Debug, Deserialize)]
struct GhIssueReference {
    url: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MergedPullRequestProof {
    pub head_oid: String,
    /// The branch the pull request merged into. Only `main` proves anything
    /// about main; a stacked pull request merged into its parent branch does not.
    pub base_ref: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhMergedPullRequestProof {
    head_ref_oid: String,
    base_ref_name: String,
}

/// Exact head commits and base branches of the merged pull requests on one
/// branch.
///
/// Cleanup uses this only when ordinary Git ancestry cannot prove that a
/// worktree is merged. A branch name alone is never deletion evidence: the
/// caller matches the head commit and the base.
pub(crate) fn merged_pull_request_proofs(
    node: &dyn NodeLink,
    place: &Where,
    branch: &str,
) -> Result<Vec<MergedPullRequestProof>, String> {
    let listed = gh_at(
        node,
        place,
        &[
            "pr",
            "list",
            "--state",
            "merged",
            "--head",
            branch,
            "--limit",
            MERGED_PROOF_LIMIT,
            "--json",
            MERGED_PROOF_FIELDS,
        ],
    )
    .map_err(|failure| {
        format!(
            "GitHub merge proof is unavailable ({}): {}",
            failure.category.english(),
            failure.reason
        )
    })?;
    parse_merged_pull_request_proofs(&listed)
}

fn parse_merged_pull_request_proofs(output: &str) -> Result<Vec<MergedPullRequestProof>, String> {
    let listed: Vec<GhMergedPullRequestProof> = serde_json::from_str(output)
        .map_err(|error| format!("gh pr list returned merge proof Hide could not read: {error}"))?;
    Ok(listed
        .into_iter()
        .map(|proof| MergedPullRequestProof {
            head_oid: proof.head_ref_oid,
            base_ref: proof.base_ref_name,
        })
        .collect())
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum GhCheck {
    CheckRun {
        status: String,
        conclusion: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default, rename = "detailsUrl")]
        details_url: Option<String>,
    },
    StatusContext {
        state: String,
        #[serde(default)]
        context: Option<String>,
        #[serde(default, rename = "targetUrl")]
        target_url: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

/// A finished check run that did not pass.
const FAILED_CONCLUSIONS: [&str; 6] = [
    "FAILURE",
    "TIMED_OUT",
    "CANCELLED",
    "ACTION_REQUIRED",
    "STARTUP_FAILURE",
    "STALE",
];

/// A commit status that did not pass.
const FAILED_STATES: [&str; 2] = ["FAILURE", "ERROR"];

impl GhCheck {
    /// A check that has not finished.
    fn is_pending(&self) -> bool {
        match self {
            GhCheck::CheckRun { status, .. } => matches!(
                status.as_str(),
                "QUEUED" | "IN_PROGRESS" | "WAITING" | "PENDING" | "REQUESTED"
            ),
            GhCheck::StatusContext { state, .. } => {
                matches!(state.as_str(), "PENDING" | "EXPECTED")
            }
            GhCheck::Unknown => false,
        }
    }

    /// A pending check that has reported: a required check GitHub only
    /// expects (`EXPECTED`) may never run, so it is not running.
    fn is_running(&self) -> bool {
        match self {
            GhCheck::StatusContext { state, .. } => state == "PENDING",
            check => check.is_pending(),
        }
    }

    /// The check by the name GitHub shows and the page that says why, when
    /// it failed; `None` for any other check.
    fn failure(&self) -> Option<crate::model::PrFailedCheck> {
        let (name, url) = match self {
            GhCheck::CheckRun {
                status,
                conclusion,
                name,
                details_url,
            } if status == "COMPLETED"
                && conclusion
                    .as_deref()
                    .is_some_and(|value| FAILED_CONCLUSIONS.contains(&value)) =>
            {
                (name, details_url)
            }
            GhCheck::StatusContext {
                state,
                context,
                target_url,
            } if FAILED_STATES.contains(&state.as_str()) => (context, target_url),
            _ => return None,
        };
        Some(crate::model::PrFailedCheck {
            name: name.clone().filter(|name| !name.trim().is_empty()),
            url: url.clone().filter(|url| url.starts_with("https://")),
        })
    }
}

fn rollup_checks(checks: Option<&[GhCheck]>) -> PullRequestChecks {
    let Some(checks) = checks else {
        return PullRequestChecks::Unknown;
    };
    if checks.is_empty() {
        return PullRequestChecks::None;
    }
    let mut pending = false;
    let mut unknown = false;
    for check in checks {
        match check {
            GhCheck::CheckRun {
                status, conclusion, ..
            } if status == "COMPLETED" => match conclusion.as_deref() {
                Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => {}
                Some(value) if FAILED_CONCLUSIONS.contains(&value) => {
                    return PullRequestChecks::Failed;
                }
                _ => unknown = true,
            },
            GhCheck::CheckRun { status, .. }
                if matches!(
                    status.as_str(),
                    "QUEUED" | "IN_PROGRESS" | "WAITING" | "PENDING" | "REQUESTED"
                ) =>
            {
                pending = true
            }
            GhCheck::StatusContext { state, .. } => match state.as_str() {
                "SUCCESS" => {}
                value if FAILED_STATES.contains(&value) => return PullRequestChecks::Failed,
                "PENDING" | "EXPECTED" => pending = true,
                _ => unknown = true,
            },
            _ => unknown = true,
        }
    }
    if pending {
        PullRequestChecks::Pending
    } else if unknown {
        PullRequestChecks::Unknown
    } else {
        PullRequestChecks::Passing
    }
}

/// Every pull request `gh pr list --json` named, checks not read yet.
///
/// None is dropped: which one belongs to a checkout depends on the checkout's
/// commit, so a branch's older pull requests are candidates too
/// (`pull_request_for_checkout`).
pub fn parse_pull_requests(output: &str) -> Result<Vec<PullRequestSnapshot>, String> {
    let listed: Vec<GhPullRequest> = serde_json::from_str(output)
        .map_err(|error| format!("gh pr list returned output Hide could not read: {error}"))?;
    listed.into_iter().map(project).collect()
}

/// The open list's rows as checks by pull request number. A row whose rollup
/// is absent is `Unknown`, as it always was.
fn parse_open_checks(output: &str) -> Result<HashMap<u32, OpenChecks>, String> {
    let listed: Vec<GhOpenChecks> = serde_json::from_str(output)
        .map_err(|error| format!("gh pr list returned checks Hide could not read: {error}"))?;
    Ok(listed
        .into_iter()
        .map(|row| {
            let rollup = row.status_check_rollup.as_deref();
            (
                row.number,
                OpenChecks {
                    rollup: rollup_checks(rollup),
                    counts: rollup.map(count_checks),
                    failed: rollup
                        .into_iter()
                        .flatten()
                        .filter_map(GhCheck::failure)
                        .collect(),
                    pending: rollup
                        .into_iter()
                        .flatten()
                        .filter(|check| check.is_pending())
                        .count() as u32,
                    running: rollup
                        .into_iter()
                        .flatten()
                        .filter(|check| check.is_running())
                        .count() as u32,
                },
            )
        })
        .collect())
}

/// One open pull request's checks as the open list read them: the rollup, the
/// checks that failed by name, and how many are still running.
struct OpenChecks {
    rollup: PullRequestChecks,
    counts: Option<crate::model::PrCheckCounts>,
    failed: Vec<PrFailedCheck>,
    pending: u32,
    /// The pending checks that reported: all but `EXPECTED`.
    running: u32,
}

fn count_checks(checks: &[GhCheck]) -> crate::model::PrCheckCounts {
    let mut counts = crate::model::PrCheckCounts {
        total: checks.len() as u32,
        ..Default::default()
    };
    for check in checks {
        match check {
            GhCheck::CheckRun {
                status, conclusion, ..
            } if status == "COMPLETED" => match conclusion.as_deref() {
                Some("SUCCESS") => counts.passed += 1,
                Some("SKIPPED" | "NEUTRAL") => counts.skipped += 1,
                Some(value) if FAILED_CONCLUSIONS.contains(&value) => counts.failed += 1,
                _ => counts.unknown += 1,
            },
            GhCheck::CheckRun { status, .. } if status == "IN_PROGRESS" => counts.running += 1,
            GhCheck::CheckRun { .. } if check.is_pending() => counts.waiting += 1,
            GhCheck::StatusContext { state, .. } => match state.as_str() {
                "SUCCESS" => counts.passed += 1,
                value if FAILED_STATES.contains(&value) => counts.failed += 1,
                "PENDING" => counts.running += 1,
                "EXPECTED" => counts.waiting += 1,
                _ => counts.unknown += 1,
            },
            _ => counts.unknown += 1,
        }
    }
    counts
}

fn project(listed: GhPullRequest) -> Result<PullRequestSnapshot, String> {
    let review = review_decision(listed.review_decision.as_deref());
    Ok(PullRequestSnapshot {
        closing_issues: listed
            .closing_issues_references
            .into_iter()
            .map(|issue| crate::issues::IssueReference::parse(&issue.url, None))
            .collect::<Result<_, _>>()?,
        title: listed.title,
        checks: PullRequestChecks::Unknown,
        badge: badge(&listed.state, listed.is_draft, review),
        number: listed.number,
        head_branch: listed.head_ref_name,
        base_branch: listed.base_ref_name,
        url: listed.url,
        review: review.filter(|_| !listed.is_draft && listed.state.eq_ignore_ascii_case("OPEN")),
        is_draft: listed.is_draft,
        merged_at_unix_ms: listed.merged_at.as_deref().and_then(parse_rfc3339_ms),
        updated_at_unix_ms: listed.updated_at.as_deref().and_then(parse_rfc3339_ms),
        created_at_unix_ms: listed.created_at.as_deref().and_then(parse_rfc3339_ms),
        closed_at_unix_ms: listed.closed_at.as_deref().and_then(parse_rfc3339_ms),
        head_oid: listed.head_ref_oid.filter(|oid| !oid.is_empty()),
        cross_repository: listed.is_cross_repository,
        work: PullRequestWork {
            author_is_bot: listed.author.as_ref().is_some_and(|author| author.is_bot),
            author: listed.author.and_then(|author| author.login),
            merge_state: listed
                .merge_state_status
                .as_deref()
                .map_or(MergeState::Unknown, MergeState::parse),
            review_requests: listed
                .review_requests
                .into_iter()
                .filter_map(|request| match request {
                    GhReviewRequest::User { login } => Some(ReviewRequest {
                        kind: ReviewRequestKind::User,
                        name: login,
                    }),
                    GhReviewRequest::Team { slug, name } => {
                        slug.or(name).map(|name| ReviewRequest {
                            kind: ReviewRequestKind::Team,
                            name,
                        })
                    }
                    GhReviewRequest::Other => None,
                })
                .collect(),
            ..PullRequestWork::default()
        },
    })
}

/// The mapping the whole feature's colour scheme rests on.
///
/// A draft is `open` whatever its review decision says, because a draft is not
/// asking to be reviewed. Anything `gh` reports that is not one of the three
/// known states is `open` rather than being dropped: a branch with a pull
/// request must not present as a branch with none.
pub fn badge(state: &str, is_draft: bool, review: Option<ReviewDecision>) -> PullRequestBadge {
    if state.eq_ignore_ascii_case("MERGED") {
        return PullRequestBadge::Merged;
    }
    if state.eq_ignore_ascii_case("CLOSED") {
        return PullRequestBadge::Closed;
    }
    if !is_draft && review.is_some() {
        return PullRequestBadge::Review;
    }
    PullRequestBadge::Open
}

pub fn review_decision(value: Option<&str>) -> Option<ReviewDecision> {
    match value?.to_ascii_uppercase().as_str() {
        "REVIEW_REQUIRED" => Some(ReviewDecision::ReviewRequired),
        "CHANGES_REQUESTED" => Some(ReviewDecision::ChangesRequested),
        "APPROVED" => Some(ReviewDecision::Approved),
        // An empty string is what `gh` reports for a pull request nobody has
        // been asked to review, which is `open`, not an unknown decision.
        _ => None,
    }
}

/// Whether `pull_request` is the work of a checkout on `branch` at commit
/// `head`. Every place a pull request is tied to a checkout asks this and
/// nothing else, because a branch name is not an identity: it is used again.
///
/// - A pull request from a fork never belongs to a local branch of the same
///   name.
/// - An open pull request belongs to the checkout on its head branch.
/// - A merged or closed one belongs only while the checkout's commit is
///   exactly the pull request's head. A branch name reused for new work has
///   a new commit, and the old merge must not follow it.
///
/// A checkout whose commit is not known yet (`head` absent) takes no settled
/// pull request, since it cannot be told from the reused name.
pub(crate) fn belongs_to_checkout(
    pull_request: &PullRequestSnapshot,
    branch: Option<&str>,
    head: Option<&str>,
) -> bool {
    // A fork's branch name says nothing about this repository's branch of
    // the same name, whether the pull request is open or long merged.
    if pull_request.cross_repository || branch != Some(pull_request.head_branch.as_str()) {
        return false;
    }
    if is_open(pull_request) {
        return true;
    }
    matches!((pull_request.head_oid.as_deref(), head), (Some(pull), Some(head)) if pull == head)
}

/// The pull request a checkout shows when several qualify: an open one, then
/// a merged one, then a closed one, the most recently updated first among
/// equals. A branch merged and then reopened for more work shows the work in
/// flight, not the merge behind it.
pub(crate) fn pull_request_for_checkout<'a>(
    pull_requests: &'a [PullRequestSnapshot],
    branch: Option<&str>,
    head: Option<&str>,
) -> Option<&'a PullRequestSnapshot> {
    pull_requests
        .iter()
        .filter(|pull_request| belongs_to_checkout(pull_request, branch, head))
        .min_by(|left, right| precedence(left, right))
}

fn precedence(left: &PullRequestSnapshot, right: &PullRequestSnapshot) -> std::cmp::Ordering {
    let rank = |pull_request: &PullRequestSnapshot| match pull_request.badge {
        PullRequestBadge::Merged => 1,
        PullRequestBadge::Closed => 2,
        PullRequestBadge::Open | PullRequestBadge::Review => 0,
    };
    rank(left)
        .cmp(&rank(right))
        .then_with(|| right.updated_at_unix_ms.cmp(&left.updated_at_unix_ms))
        .then_with(|| right.number.cmp(&left.number))
}

/// One pull request per head branch, for the views that list a branch once
/// (the PRs view, a worktree's base): the one `precedence` puts first,
/// whatever the checkout's commit. Which pull request a checkout holds is
/// `pull_request_for_checkout`'s answer, never this one.
pub(crate) fn preferred_per_branch(
    pull_requests: &[PullRequestSnapshot],
) -> Vec<&PullRequestSnapshot> {
    let mut candidates: Vec<&PullRequestSnapshot> = pull_requests.iter().collect();
    // Open first, then the most recently updated whether merged or closed:
    // the order these lists have always kept, so what they show is unchanged.
    candidates.sort_by(|left, right| {
        left.head_branch
            .cmp(&right.head_branch)
            .then_with(|| is_open(right).cmp(&is_open(left)))
            .then_with(|| right.updated_at_unix_ms.cmp(&left.updated_at_unix_ms))
            .then_with(|| right.number.cmp(&left.number))
    });
    candidates.dedup_by(|later, kept| later.head_branch == kept.head_branch);
    candidates
}

fn is_open(pull_request: &PullRequestSnapshot) -> bool {
    !pull_request.badge.is_settled()
}

/// `gh` timestamps are RFC 3339 in UTC (`2026-09-03T04:15:00Z`). Only the
/// instant is needed - the card renders "N minutes ago" from it - so this
/// converts without pulling in a date library for one format.
pub fn parse_rfc3339_ms(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.len() < 20 {
        return None;
    }
    let bytes = value.as_bytes();
    let number = |start: usize, end: usize| -> Option<u64> {
        std::str::from_utf8(&bytes[start..end]).ok()?.parse().ok()
    };
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000)
}

/// Days since 1970-01-01 for a proleptic Gregorian date at or after it.
///
/// Written as whole leap-year counts rather than an era formula: the inputs
/// are `gh` timestamps, all of them modern, and a reader can check this by
/// eye against a calendar.
fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    const CUMULATIVE: [u64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap_days_before = |year: u64| {
        let previous = year - 1;
        previous / 4 - previous / 100 + previous / 400
    };
    let leap = |year: u64| {
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
    };
    let leaps = leap_days_before(year) - leap_days_before(1970);
    let leap_day_this_year = u64::from(leap(year) && month > 2);
    365 * (year - 1970) + leaps + CUMULATIVE[(month - 1) as usize] + leap_day_this_year + day - 1
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// The main worktree of the repository `path` is in, from its node's git;
/// `None` when `path` is in no repository.
fn main_worktree(node: &dyn NodeLink, path: &Path) -> Option<PathBuf> {
    let call = Call::Git {
        root: path.to_string_lossy().into_owned(),
        command: GitCommand::CommonDir,
    };
    let common: String = crate::node_access::call_as(node, call, GH_CALL_TIMEOUT).ok()?;
    if common.is_empty() {
        return None;
    }
    PathBuf::from(common).parent().map(Path::to_path_buf)
}

#[derive(Clone, Debug)]
pub(crate) struct GhFailure {
    pub(crate) category: GithubFailureCategory,
    pub(crate) reason: String,
}

/// Closes the one issue the backlog made (`되돌리기`, PRD overview-device-tasks
/// D-21). GitHub keeps a closed issue; Hide never deletes one.
pub(crate) fn close_issue(
    node: &dyn NodeLink,
    place: &Where,
    repository: &str,
    number: u32,
) -> Result<(), GhFailure> {
    gh_at(
        node,
        place,
        &["issue", "close", &number.to_string(), "--repo", repository],
    )
    .map(|_| ())
}

impl GhFailure {
    fn network(reason: String) -> Self {
        Self {
            category: GithubFailureCategory::NetworkOrRateLimit,
            reason,
        }
    }
}

/// Where a `gh` command finds its repository: a checkout on the machine it
/// runs on, or, for a checkout on another machine, the repository's name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Where {
    Checkout(PathBuf),
    Repository(String),
}

fn gh_at(node: &dyn NodeLink, place: &Where, arguments: &[&str]) -> Result<String, GhFailure> {
    match place {
        Where::Checkout(root) => gh(node, Some(root), arguments),
        Where::Repository(repository) => run_gh_call(node, None, Some(repository), arguments),
    }
}

fn gh(node: &dyn NodeLink, cwd: Option<&Path>, arguments: &[&str]) -> Result<String, GhFailure> {
    run_gh_call(node, cwd, None, arguments)
}

fn run_gh_call(
    node: &dyn NodeLink,
    cwd: Option<&Path>,
    repository: Option<&str>,
    arguments: &[&str],
) -> Result<String, GhFailure> {
    run_gh_call_at(
        node,
        cwd.map(|cwd| cwd.to_string_lossy()).as_deref(),
        repository,
        arguments,
    )
}

fn run_gh_call_at(
    node: &dyn NodeLink,
    cwd: Option<&str>,
    repository: Option<&str>,
    arguments: &[&str],
) -> Result<String, GhFailure> {
    let call = Call::Gh {
        cwd: cwd.map(str::to_owned),
        args: arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect(),
        repository: repository.map(str::to_owned),
    };
    match crate::node_access::call_as(node, call, GH_CALL_TIMEOUT) {
        Ok(GhAnswer::Output { stdout }) => Ok(stdout),
        Ok(GhAnswer::Failed { category, reason }) => Err(GhFailure { category, reason }),
        Err(error) => Err(GhFailure::network(format!(
            "gh could not be asked for: {error}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture `gh` answers at once; a one-second deadline read a loaded
    /// machine's slow shell start as a timeout, the network category.
    #[cfg(unix)]
    #[test]
    fn optional_project_permission_does_not_prevent_basic_issue_resolution() {
        let reference = crate::issues::IssueReference::parse("acme/project#42", None).unwrap();
        let mut attempts = Vec::new();
        let (issues, warning) = with_optional_projects(|include_projects| {
            attempts.push(include_projects);
            let query = issue_query(&[&reference], include_projects)?;
            if include_projects {
                assert!(query.contains("projectItems"));
                return Err(GhFailure::network("INSUFFICIENT_SCOPES: read:project".into()));
            }
            assert!(!query.contains("projectItems"));
            parse_linked_issues(r#"{"data":{"r0":{"issue":{"number":42,"title":"Task","url":"https://github.com/acme/project/issues/42","state":"OPEN","updatedAt":null}}}}"#)
        }).unwrap();
        assert_eq!(attempts, vec![true, false]);
        assert!(warning.is_some());
        assert_eq!(issues[0].reference, reference);
        assert!(issues[0].project_status.is_none());
    }

    fn label(name: &str, color: Option<&str>) -> crate::tasks::TaskLabel {
        crate::tasks::TaskLabel {
            name: name.into(),
            color: color.map(Into::into),
        }
    }

    #[test]
    fn the_issue_list_brings_each_issues_labels_to_its_task() {
        // What `gh issue list --json ...,labels,...` prints for a label (gh 2.76).
        let listed = r#"[{"number":7,"title":"Fix","url":"https://github.com/acme/app/issues/7","state":"OPEN","labels":[{"id":"LA_1","name":"bug","description":"","color":"d73a4a"},{"id":"LA_2","name":"odd","description":"","color":"red;x"}],"projectItems":[],"updatedAt":"2026-10-01T00:00:00Z"},{"number":8,"title":"Plain","url":"https://github.com/acme/app/issues/8","state":"OPEN","labels":[],"projectItems":[],"updatedAt":"2026-09-30T00:00:00Z"}]"#;
        for include_projects in [true, false] {
            let asked = Mutex::new(Vec::<String>::new());
            let issues = list_issues(
                &|arguments: &[&str]| {
                    let fields =
                        arguments[arguments.iter().position(|a| *a == "--json").unwrap() + 1];
                    asked.lock().unwrap().push(fields.to_owned());
                    Ok(if arguments[3] == "closed" {
                        "[]".to_owned()
                    } else {
                        listed.to_owned()
                    })
                },
                include_projects,
            )
            .expect("the list answers");
            let fields = asked.into_inner().unwrap();
            assert_eq!(
                fields.len(),
                2,
                "one open and one closed list, no read per issue"
            );
            assert!(
                fields[0].split(',').any(|field| field == "labels"),
                "the list asks for labels whether or not it asks for projects: {}",
                fields[0]
            );
            let tasks = crate::tasks::github_tasks(
                &crate::issues::ProjectIssuesSnapshot {
                    repository: Some("acme/app".into()),
                    issues,
                    ..Default::default()
                },
                &GithubStatusSnapshot::default(),
                false,
            )
            .tasks;
            assert_eq!(
                tasks[0].labels,
                vec![label("bug", Some("d73a4a")), label("odd", None)],
                "a colour that is not six hex digits is no colour"
            );
            assert!(tasks[1].labels.is_empty());
            let wire = serde_json::to_value(&tasks).unwrap();
            assert_eq!(
                wire[0]["labels"],
                serde_json::json!([{"name": "bug", "color": "d73a4a"}, {"name": "odd", "color": null}])
            );
            assert!(
                wire[1].get("labels").is_none(),
                "an issue with no label carries none on the wire"
            );
        }
    }

    /// What `gh api graphql` printed on 2026-10-14 (gh 2.76) for one issue
    /// that resolves, a repository that does not and an issue that does not:
    /// the unresolved aliases are `null` beside `NOT_FOUND` errors.
    #[test]
    fn a_linked_issue_that_does_not_resolve_leaves_the_others_read() {
        let partial = r#"{"data":{"r0":{"issue":{"number":12,"title":"Kept","url":"https://github.com/acme/app/issues/12","state":"OPEN","updatedAt":null}},"r1":null,"r2":{"issue":null}},"errors":[{"type":"NOT_FOUND","path":["r1"],"locations":[{"line":1,"column":118}],"message":"Could not resolve to a Repository with the name 'acme/override'."},{"type":"NOT_FOUND","path":["r2","issue"],"locations":[{"line":1,"column":255}],"message":"Could not resolve to an Issue with the number of 999999."}]}"#;
        let issues = parse_linked_issues(partial).expect("the resolved link is read");
        assert_eq!(
            issues
                .iter()
                .map(|issue| issue.reference.token())
                .collect::<Vec<_>>(),
            ["acme/app#12"]
        );
        // Any other error is no answer for the links it names.
        let refused = r#"{"data":{"r0":null},"errors":[{"type":"FORBIDDEN","path":["r0"],"message":"Resource not accessible"}]}"#;
        assert!(parse_linked_issues(refused).is_err());
    }

    #[test]
    fn a_linked_issue_read_by_number_carries_its_labels() {
        let reference = crate::issues::IssueReference::parse("acme/other#12", None).unwrap();
        for include_projects in [true, false] {
            let query = issue_query(&[&reference], include_projects).unwrap();
            assert!(
                query.contains("labels(first:100){nodes{name color}}"),
                "{query}"
            );
        }
        let issues = parse_linked_issues(r#"{"data":{"r0":{"issue":{"number":12,"title":"Elsewhere","url":"https://github.com/acme/other/issues/12","state":"CLOSED","updatedAt":null,"labels":{"nodes":[{"name":"blocked","color":"000000"}]}}},"r1":{"issue":{"number":13,"title":"Bare","url":"https://github.com/acme/other/issues/13","state":"OPEN","updatedAt":null}}}}"#).unwrap();
        let by_number = |number: u32| {
            issues
                .iter()
                .find(|issue| issue.reference.number == number)
                .expect("issue answered")
        };
        assert_eq!(by_number(12).labels, vec![label("blocked", Some("000000"))]);
        assert!(by_number(13).labels.is_empty());
    }

    #[test]
    fn an_issue_panel_read_keeps_a_real_comment_page_and_drops_a_colour_that_is_not_hex() {
        let comment = |n: u32| {
            format!(
                r#"{{"author":{{"login":"c{n}"}},"createdAt":"2026-09-2{n}T00:00:00Z","body":" note {n} "}}"#
            )
        };
        let comments: Vec<String> = (1..=5).map(comment).collect();
        let output = format!(
            r#"{{"data":{{"repository":{{"issue":{{"body":"본문","labels":{{"nodes":[{{"name":"bug","color":"d73a4a"}},{{"name":"odd","color":"red;x"}}]}},"author":{{"login":"example","name":""}},"assignees":{{"nodes":[{{"login":"a1"}}]}},"comments":{{"nodes":[{}],"totalCount":120,"pageInfo":{{"hasNextPage":true,"endCursor":"page-1"}}}},"createdAt":"2026-09-27T00:00:00Z"}}}}}}}}"#,
            comments.join(",")
        );
        let detail = parse_issue_detail(&output).unwrap();
        assert_eq!(detail.body, "본문");
        assert_eq!(
            detail
                .labels
                .iter()
                .map(|label| (label.name.as_str(), label.color.as_deref()))
                .collect::<Vec<_>>(),
            vec![("bug", Some("d73a4a")), ("odd", None)]
        );
        assert_eq!(detail.author.as_deref(), Some("example"));
        assert_eq!(detail.assignees, vec!["a1".to_owned()]);
        assert_eq!(
            detail.created_at_unix_ms,
            parse_rfc3339_ms("2026-09-27T00:00:00Z")
        );
        assert_eq!(detail.comment_count, Some(120));
        assert_eq!(detail.comments_cursor.as_deref(), Some("page-1"));
        assert!(detail.comments_has_more);
        assert_eq!(
            detail
                .comments
                .iter()
                .map(|comment| (comment.author.as_deref(), comment.body.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (Some("c1"), "note 1"),
                (Some("c2"), "note 2"),
                (Some("c3"), "note 3"),
                (Some("c4"), "note 4"),
                (Some("c5"), "note 5")
            ]
        );
        assert!(parse_issue_detail("{}").is_err(), "a body is required");
        for cursor in ["", "한글", &"x".repeat(1025)] {
            let mut malformed: serde_json::Value = serde_json::from_str(&output).unwrap();
            malformed["data"]["repository"]["issue"]["comments"]["pageInfo"]["endCursor"] =
                serde_json::json!(cursor);
            assert!(
                parse_issue_detail(&malformed.to_string()).is_err(),
                "an unusable next cursor must fail at the read"
            );
        }
    }

    #[test]
    fn a_closing_line_is_written_once_however_often_it_is_asked_for() {
        // `gh` over a body kept in memory: `pr view` prints it, `pr edit` replaces it.
        let body = std::cell::RefCell::new("Moves the reader off the lock.\n".to_owned());
        let gh = |arguments: &[&str]| -> Result<String, GhFailure> {
            match arguments {
                ["pr", "view", "12", "--json", "body"] => {
                    Ok(serde_json::json!({ "body": *body.borrow() }).to_string())
                }
                ["pr", "edit", "12", "--body", next] => {
                    *body.borrow_mut() = (*next).to_owned();
                    Ok("https://github.com/acme/app/pull/12".to_owned())
                }
                other => panic!("unexpected gh call {other:?}"),
            }
        };
        assert_eq!(write_closing_line_with(gh, 12, 7), Ok(true));
        assert_eq!(
            *body.borrow(),
            "Moves the reader off the lock.\n\nCloses #7"
        );
        // The retry finds the line and writes nothing (D-47, B14).
        assert_eq!(write_closing_line_with(gh, 12, 7), Ok(false));
        assert_eq!(
            *body.borrow(),
            "Moves the reader off the lock.\n\nCloses #7"
        );
    }

    #[test]
    fn a_body_closes_an_issue_only_by_a_closing_keyword_and_its_number() {
        assert!(closes_issue("Fixes #7.", 7));
        assert!(closes_issue("resolves: acme/app#7", 7));
        assert!(closes_issue(
            "closed https://github.com/acme/app/issues/7",
            7
        ));
        assert!(!closes_issue("Closes #70", 7));
        assert!(!closes_issue("See #7", 7));
        assert!(!closes_issue("Closes a/b/c#7", 7));
        assert_eq!(with_closing_line("  \n", 3), "Closes #3");
    }

    #[test]
    fn feedback_names_the_failed_checks_and_the_change_requests_still_standing() {
        let output = r#"{
            "body": "Reads the remote primary.",
            "statusCheckRollup": [
                {"__typename":"CheckRun","name":"verify","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"https://github.com/acme/app/actions/runs/1"},
                {"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS"},
                {"__typename":"StatusContext","context":"ci/legacy","state":"ERROR","targetUrl":"javascript:alert(1)"},
                {"__typename":"CheckRun","status":"COMPLETED","conclusion":"FAILURE"}
            ],
            "reviews": [
                {"author":{"login":"ana"},"state":"CHANGES_REQUESTED","body":"old ask"},
                {"author":{"login":"ana"},"state":"COMMENTED","body":"note"},
                {"author":{"login":"ana"},"state":"CHANGES_REQUESTED","body":" Split the reader. "},
                {"author":{"login":"bo"},"state":"CHANGES_REQUESTED","body":"rename"},
                {"author":{"login":"bo"},"state":"APPROVED","body":""}
            ]
        }"#;
        let feedback = parse_pr_feedback(output).unwrap();
        assert_eq!(feedback.body, "Reads the remote primary.");
        assert_eq!(
            feedback.failed_checks,
            vec![
                crate::model::PrFailedCheck {
                    name: Some("verify".into()),
                    url: Some("https://github.com/acme/app/actions/runs/1".into()),
                },
                crate::model::PrFailedCheck {
                    name: Some("ci/legacy".into()),
                    url: None,
                },
                // The page words a check GitHub gives no name.
                crate::model::PrFailedCheck {
                    name: None,
                    url: None,
                },
            ]
        );
        assert_eq!(
            feedback.change_requests,
            vec![crate::model::PrChangeRequest {
                author: Some("ana".into()),
                body: "Split the reader.".into(),
            }]
        );
        assert!(parse_pr_feedback("{}").is_err());
    }

    #[test]
    fn ci_rollup_never_reports_absent_pending_or_failed_checks_as_passing() {
        let decode = |json: &str| serde_json::from_str::<Vec<GhCheck>>(json).unwrap();
        assert_eq!(rollup_checks(None), PullRequestChecks::Unknown);
        assert_eq!(rollup_checks(Some(&[])), PullRequestChecks::None);
        assert_eq!(
            rollup_checks(Some(&decode(
                r#"[{"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS"},{"__typename":"StatusContext","state":"SUCCESS"}]"#
            ))),
            PullRequestChecks::Passing
        );
        assert_eq!(
            rollup_checks(Some(&decode(
                r#"[{"__typename":"CheckRun","status":"QUEUED","conclusion":null}]"#
            ))),
            PullRequestChecks::Pending
        );
        assert_eq!(
            rollup_checks(Some(&decode(
                r#"[{"__typename":"CheckRun","status":"IN_PROGRESS"},{"__typename":"StatusContext","state":"ERROR"}]"#
            ))),
            PullRequestChecks::Failed
        );
        assert_eq!(
            rollup_checks(Some(&decode(
                r#"[{"__typename":"CheckRun","status":"COMPLETED","conclusion":"CANCELLED"}]"#
            ))),
            PullRequestChecks::Failed
        );
        assert_eq!(
            rollup_checks(Some(&decode(r#"[{"__typename":"FutureCheck"}]"#))),
            PullRequestChecks::Unknown
        );
    }

    #[test]
    fn a_project_is_read_first_then_only_when_its_generation_moves() {
        assert_eq!(read_reason(None, 0), Some(ReadReason::First));
        assert_eq!(read_reason(Some(0), 1), Some(ReadReason::Generation));
        assert_eq!(read_reason(Some(1), 1), None);
        assert_eq!(read_reason(Some(1), 1), None);
    }

    fn listed(
        number: u32,
        branch: &str,
        state: &str,
        review: Option<&str>,
        draft: bool,
        updated: &str,
    ) -> String {
        format!(
            r#"{{"title":"Fixture PR","statusCheckRollup":[],"number":{number},"headRefName":"{branch}","baseRefName":"main","state":"{state}","reviewDecision":{review},"isDraft":{draft},"url":"https://example.invalid/{number}","mergedAt":null,"updatedAt":"{updated}"}}"#,
            review = review
                .map(|value| format!("\"{value}\""))
                .unwrap_or_else(|| "null".to_owned())
        )
    }

    #[test]
    fn every_state_and_review_combination_maps_to_its_badge() {
        assert_eq!(badge("MERGED", false, None), PullRequestBadge::Merged);
        assert_eq!(badge("CLOSED", false, None), PullRequestBadge::Closed);
        assert_eq!(badge("OPEN", false, None), PullRequestBadge::Open);
        for decision in [
            ReviewDecision::ReviewRequired,
            ReviewDecision::ChangesRequested,
            ReviewDecision::Approved,
        ] {
            assert_eq!(
                badge("OPEN", false, Some(decision)),
                PullRequestBadge::Review,
                "an open reviewed pull request is a review badge"
            );
            // A draft is not asking for review, whatever GitHub recorded.
            assert_eq!(badge("OPEN", true, Some(decision)), PullRequestBadge::Open);
        }
        // A merged pull request stays merged even if a review is recorded.
        assert_eq!(
            badge("MERGED", false, Some(ReviewDecision::Approved)),
            PullRequestBadge::Merged
        );
    }

    #[test]
    fn the_three_review_decisions_are_kept_apart_and_anything_else_is_none() {
        assert_eq!(
            review_decision(Some("REVIEW_REQUIRED")),
            Some(ReviewDecision::ReviewRequired)
        );
        assert_eq!(
            review_decision(Some("CHANGES_REQUESTED")),
            Some(ReviewDecision::ChangesRequested)
        );
        assert_eq!(
            review_decision(Some("APPROVED")),
            Some(ReviewDecision::Approved)
        );
        assert_eq!(review_decision(Some("")), None);
        assert_eq!(review_decision(None), None);
    }

    fn candidate(
        number: u32,
        branch: &str,
        badge: PullRequestBadge,
        head: Option<&str>,
        updated: u64,
    ) -> PullRequestSnapshot {
        let mut pull_request = parse_pull_requests(&format!(
            "[{}]",
            listed(number, branch, "OPEN", None, false, "2026-09-01T10:00:00Z")
        ))
        .expect("gh output parses")
        .remove(0);
        pull_request.badge = badge;
        pull_request.head_oid = head.map(str::to_owned);
        pull_request.updated_at_unix_ms = Some(updated);
        pull_request
    }

    fn picked(
        pull_requests: &[PullRequestSnapshot],
        branch: &str,
        head: Option<&str>,
    ) -> Option<u32> {
        pull_request_for_checkout(pull_requests, Some(branch), head)
            .map(|pull_request| pull_request.number)
    }

    #[test]
    fn a_listed_pull_request_keeps_its_head_commit_and_whether_it_came_from_a_fork() {
        let output = r#"[{"title":"t","number":7,"headRefName":"feature","headRefOid":"abc123","isCrossRepository":true,"baseRefName":"main","state":"OPEN","reviewDecision":"","isDraft":false,"url":"https://example.invalid/7","mergedAt":null,"updatedAt":"2026-09-01T10:00:00Z"},{"title":"t","number":8,"headRefName":"other","baseRefName":"main","state":"OPEN","reviewDecision":"","isDraft":false,"url":"https://example.invalid/8","mergedAt":null,"updatedAt":"2026-09-01T10:00:00Z"}]"#;
        let listed = parse_pull_requests(output).expect("gh output parses");
        assert_eq!(listed[0].head_oid.as_deref(), Some("abc123"));
        assert!(listed[0].cross_repository);
        assert_eq!(listed[1].head_oid, None);
        assert!(!listed[1].cross_repository);
    }

    #[test]
    fn every_pull_request_of_a_branch_is_kept_for_the_checkout_to_choose_from() {
        let output = format!(
            "[{},{}]",
            listed(9, "feature", "MERGED", None, false, "2026-09-03T10:00:00Z"),
            listed(4, "feature", "OPEN", None, false, "2026-09-01T10:00:00Z"),
        );
        let listed = parse_pull_requests(&output).expect("gh output parses");
        assert_eq!(
            listed.iter().map(|pr| pr.number).collect::<Vec<_>>(),
            vec![9, 4]
        );
    }

    #[test]
    fn an_open_pull_request_beats_a_merged_one_beats_a_closed_one_for_the_same_checkout() {
        let pull_requests = [
            candidate(1, "feature", PullRequestBadge::Closed, Some("head"), 90),
            candidate(2, "feature", PullRequestBadge::Merged, Some("head"), 80),
            candidate(3, "feature", PullRequestBadge::Open, Some("head"), 10),
        ];
        assert_eq!(picked(&pull_requests, "feature", Some("head")), Some(3));
        assert_eq!(
            picked(&pull_requests[..2], "feature", Some("head")),
            Some(2)
        );
        assert_eq!(
            picked(&pull_requests[..1], "feature", Some("head")),
            Some(1)
        );
    }

    #[test]
    fn among_equals_the_most_recently_updated_wins() {
        let pull_requests = [
            candidate(1, "feature", PullRequestBadge::Merged, Some("head"), 10),
            candidate(2, "feature", PullRequestBadge::Merged, Some("head"), 20),
            candidate(3, "feature", PullRequestBadge::Merged, Some("head"), 20),
        ];
        assert_eq!(picked(&pull_requests, "feature", Some("head")), Some(3));
        assert_eq!(
            picked(&pull_requests[..2], "feature", Some("head")),
            Some(2)
        );
    }

    #[test]
    fn an_old_merged_pull_request_does_not_follow_a_branch_name_used_again() {
        let pull_requests = [candidate(
            1,
            "feature",
            PullRequestBadge::Merged,
            Some("old-work"),
            10,
        )];
        assert_eq!(picked(&pull_requests, "feature", Some("new-work")), None);
        assert_eq!(picked(&pull_requests, "feature", Some("old-work")), Some(1));
    }

    #[test]
    fn a_settled_pull_request_needs_a_known_commit_on_both_sides() {
        let closed = candidate(1, "feature", PullRequestBadge::Closed, Some("head"), 10);
        let unread = candidate(2, "feature", PullRequestBadge::Merged, None, 10);
        assert_eq!(picked(std::slice::from_ref(&closed), "feature", None), None);
        assert_eq!(
            picked(std::slice::from_ref(&unread), "feature", Some("head")),
            None
        );
        assert_eq!(
            picked(std::slice::from_ref(&closed), "feature", Some("head")),
            Some(1)
        );
    }

    #[test]
    fn a_settled_pull_request_names_its_branch_as_well_as_its_commit() {
        // A fast-forward merge leaves main on the pull request's head commit;
        // a new branch cut from main there is not that pull request's work.
        let merged = [candidate(
            1,
            "old-feature",
            PullRequestBadge::Merged,
            Some("head"),
            10,
        )];
        assert_eq!(picked(&merged, "new-feature", Some("head")), None);
        assert_eq!(pull_request_for_checkout(&merged, None, Some("head")), None);
    }

    #[test]
    fn an_open_pull_request_belongs_by_branch_whatever_the_checkout_commit_is() {
        let pull_requests = [candidate(
            1,
            "feature",
            PullRequestBadge::Open,
            Some("pushed"),
            10,
        )];
        assert_eq!(
            picked(&pull_requests, "feature", Some("unpushed-commit")),
            Some(1)
        );
        assert_eq!(picked(&pull_requests, "feature", None), Some(1));
        assert_eq!(picked(&pull_requests, "other", Some("pushed")), None);
    }

    #[test]
    fn an_open_pull_request_from_a_fork_never_belongs_to_a_branch_of_the_same_name() {
        let mut from_fork = candidate(1, "main", PullRequestBadge::Open, Some("head"), 10);
        from_fork.cross_repository = true;
        assert_eq!(
            picked(std::slice::from_ref(&from_fork), "main", Some("head")),
            None
        );
        from_fork.cross_repository = false;
        assert_eq!(
            picked(std::slice::from_ref(&from_fork), "main", Some("head")),
            Some(1)
        );
    }

    #[test]
    fn a_settled_pull_request_from_a_fork_never_belongs_either() {
        // A fork's `main` merged by fast-forward leaves local main on its head.
        let mut from_fork = candidate(1, "main", PullRequestBadge::Merged, Some("head"), 10);
        from_fork.cross_repository = true;
        assert_eq!(
            picked(std::slice::from_ref(&from_fork), "main", Some("head")),
            None
        );
    }

    #[test]
    fn a_list_shows_the_newer_closed_pull_request_over_an_older_merged_one() {
        let pull_requests = [
            candidate(1, "alpha", PullRequestBadge::Merged, Some("a"), 10),
            candidate(2, "alpha", PullRequestBadge::Closed, Some("b"), 20),
        ];
        let numbers: Vec<u32> = preferred_per_branch(&pull_requests)
            .into_iter()
            .map(|pull_request| pull_request.number)
            .collect();
        assert_eq!(numbers, vec![2]);
    }

    #[test]
    fn each_branch_lists_its_preferred_pull_request_once() {
        let pull_requests = [
            candidate(1, "alpha", PullRequestBadge::Merged, Some("a"), 10),
            candidate(2, "alpha", PullRequestBadge::Open, Some("b"), 5),
            candidate(3, "beta", PullRequestBadge::Merged, Some("c"), 20),
        ];
        let numbers: Vec<u32> = preferred_per_branch(&pull_requests)
            .into_iter()
            .map(|pull_request| pull_request.number)
            .collect();
        assert_eq!(numbers, vec![2, 3]);
    }

    /// A response Hide cannot read is a stated failure, never an empty list
    /// that would present every branch as having no pull request.
    #[test]
    fn unreadable_output_is_a_reason_rather_than_no_pull_requests() {
        let failure = parse_pull_requests("{\"unexpected\":true}").expect_err("malformed");
        assert!(failure.contains("could not read"), "{failure}");
    }

    #[test]
    fn an_empty_list_is_a_real_answer() {
        assert_eq!(parse_pull_requests("[]").expect("empty list"), Vec::new());
    }

    /// A `gh` that answers the two lists from fixtures and records what it
    /// was asked, so the read can be judged by what it returns and asks for.
    struct TwoLists {
        every: Result<String, GhFailure>,
        open: Result<String, GhFailure>,
        asked: Mutex<Vec<Vec<String>>>,
    }

    impl TwoLists {
        fn new(every: &str, open: &str) -> Self {
            Self {
                every: Ok(every.to_owned()),
                open: Ok(open.to_owned()),
                asked: Mutex::default(),
            }
        }

        fn run(&self, arguments: &[&str]) -> Result<String, GhFailure> {
            self.asked.lock().unwrap().push(
                arguments
                    .iter()
                    .map(|argument| (*argument).to_owned())
                    .collect(),
            );
            let state = arguments[arguments.iter().position(|a| *a == "--state").unwrap() + 1];
            match state {
                "all" => self.every.clone(),
                "open" => self.open.clone(),
                other => panic!("unexpected state {other}"),
            }
        }

        fn read(&self, known: &KnownChecks) -> Result<Vec<PullRequestSnapshot>, GhFailure> {
            list_pull_requests(&|arguments| self.run(arguments), known)
        }
    }

    fn listed_at(number: u32, branch: &str, state: &str, head: &str) -> String {
        format!(
            r#"{{"title":"t","number":{number},"headRefName":"{branch}","headRefOid":"{head}","isCrossRepository":false,"baseRefName":"main","state":"{state}","reviewDecision":"","isDraft":false,"url":"https://example.invalid/{number}","mergedAt":null,"updatedAt":"2026-09-01T10:00:00Z"}}"#
        )
    }

    fn rollup(number: u32, conclusion: &str) -> String {
        format!(
            r#"{{"number":{number},"statusCheckRollup":[{{"__typename":"CheckRun","status":"COMPLETED","conclusion":"{conclusion}"}}]}}"#
        )
    }

    fn checks_of(pull_requests: &[PullRequestSnapshot], number: u32) -> PullRequestChecks {
        pull_requests
            .iter()
            .find(|pull_request| pull_request.number == number)
            .expect("pull request listed")
            .checks
    }

    #[test]
    fn open_pull_requests_take_their_checks_from_the_open_list_and_settled_ones_read_unknown() {
        let gh = TwoLists::new(
            &format!(
                "[{},{},{}]",
                listed_at(1, "a", "OPEN", "h1"),
                listed_at(2, "b", "MERGED", "h2"),
                listed_at(3, "c", "CLOSED", "h3"),
            ),
            &format!("[{}]", rollup(1, "FAILURE")),
        );
        let read = gh.read(&KnownChecks::new()).expect("both lists answer");
        assert_eq!(checks_of(&read, 1), PullRequestChecks::Failed);
        assert_eq!(checks_of(&read, 2), PullRequestChecks::Unknown);
        assert_eq!(checks_of(&read, 3), PullRequestChecks::Unknown);
    }

    #[test]
    fn a_settled_pull_request_keeps_the_checks_read_for_its_number_and_head_commit() {
        let gh = TwoLists::new(
            &format!(
                "[{},{}]",
                listed_at(2, "b", "MERGED", "h2"),
                listed_at(3, "c", "MERGED", "moved-on"),
            ),
            "[]",
        );
        let known: KnownChecks = [
            ((2, "h2".to_owned()), PullRequestChecks::Passing),
            ((3, "h3".to_owned()), PullRequestChecks::Failed),
        ]
        .into_iter()
        .collect();
        let read = gh.read(&known).expect("both lists answer");
        assert_eq!(checks_of(&read, 2), PullRequestChecks::Passing);
        assert_eq!(
            checks_of(&read, 3),
            PullRequestChecks::Unknown,
            "checks read at another head commit are not this commit's"
        );
    }

    #[test]
    fn an_open_pull_request_never_borrows_remembered_checks() {
        let gh = TwoLists::new(&format!("[{}]", listed_at(1, "a", "OPEN", "h1")), "[]");
        let known: KnownChecks = [((1, "h1".to_owned()), PullRequestChecks::Passing)]
            .into_iter()
            .collect();
        let read = gh.read(&known).expect("both lists answer");
        assert_eq!(checks_of(&read, 1), PullRequestChecks::Unknown);
    }

    #[test]
    fn what_was_read_is_kept_for_the_next_read_and_survives_a_failed_one() {
        let gh = TwoLists::new(
            &format!("[{}]", listed_at(1, "a", "OPEN", "h1")),
            &format!("[{}]", rollup(1, "SUCCESS")),
        );
        let read = gh.read(&KnownChecks::new()).expect("both lists answer");
        let answer = GithubProjectSnapshot {
            pull_requests: read,
            pull_requests_read: true,
            ..GithubProjectSnapshot::default()
        };
        let kept = carried_checks(&answer, KnownChecks::new());
        assert_eq!(
            kept.get(&(1, "h1".to_owned())),
            Some(&PullRequestChecks::Passing)
        );
        let failed = GithubProjectSnapshot::default();
        assert_eq!(carried_checks(&failed, kept.clone()), kept);
    }

    #[test]
    fn the_two_lists_ask_for_what_each_is_for() {
        let gh = TwoLists::new("[]", "[]");
        gh.read(&KnownChecks::new()).expect("both lists answer");
        let asked = gh.asked.lock().unwrap();
        let fields = |state: &str| -> String {
            let call = asked
                .iter()
                .find(|call| call.iter().any(|argument| argument == state))
                .expect("the list was asked for");
            call[call.iter().position(|a| a == "--json").unwrap() + 1].clone()
        };
        assert_eq!(asked.len(), 2);
        assert!(
            !fields("all").contains("statusCheckRollup"),
            "settled pull requests' checks are not asked for"
        );
        assert!(
            fields("all").contains("headRefOid") && fields("all").contains("isCrossRepository")
        );
        assert_eq!(fields("open"), "number,statusCheckRollup");
    }

    #[test]
    fn either_list_failing_fails_the_read_instead_of_reading_as_no_pull_requests() {
        let mut gh = TwoLists::new("[]", "[]");
        gh.open = Err(GhFailure::network("open list down".to_owned()));
        let failure = gh.read(&KnownChecks::new()).expect_err("open list failed");
        assert_eq!(failure.reason, "open list down");

        let mut gh = TwoLists::new("[]", "[]");
        gh.every = Err(GhFailure::network("all list down".to_owned()));
        let failure = gh.read(&KnownChecks::new()).expect_err("all list failed");
        assert_eq!(failure.reason, "all list down");

        let gh = TwoLists::new("[]", "{\"unexpected\":true}");
        let failure = gh.read(&KnownChecks::new()).expect_err("unreadable checks");
        assert!(
            failure.reason.contains("could not read"),
            "{}",
            failure.reason
        );
    }

    /// The two reads have to overlap or the saving is gone: each waits for
    /// the other to have started, and a read that runs them one after the
    /// other gives up after the deadline instead of hanging.
    #[test]
    fn the_two_lists_are_asked_for_at_the_same_time() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let started = AtomicUsize::new(0);
        let run = |_: &[&str]| -> Result<String, GhFailure> {
            started.fetch_add(1, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(10);
            while started.load(Ordering::SeqCst) < 2 {
                if Instant::now() > deadline {
                    return Err(GhFailure::network(
                        "the other list was never asked".to_owned(),
                    ));
                }
                std::thread::yield_now();
            }
            Ok("[]".to_owned())
        };
        list_pull_requests(&run, &KnownChecks::new()).expect("both lists were in flight together");
    }

    #[test]
    fn merged_pull_request_proof_keeps_exact_head_and_base() {
        let proofs = parse_merged_pull_request_proofs(
            r#"[{"headRefOid":"branch-head","baseRefName":"main"},{"headRefOid":"other-head","baseRefName":"ci/platform-completion"}]"#,
        )
        .expect("merge proof parses");
        assert_eq!(
            proofs,
            vec![
                MergedPullRequestProof {
                    head_oid: "branch-head".into(),
                    base_ref: "main".into(),
                },
                MergedPullRequestProof {
                    head_oid: "other-head".into(),
                    base_ref: "ci/platform-completion".into(),
                },
            ]
        );
    }

    #[test]
    fn timestamps_convert_to_the_instant_the_card_counts_from() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_ms("2026-09-03T04:15:00Z"),
            Some(1_788_408_900_000)
        );
        assert_eq!(parse_rfc3339_ms("not a timestamp"), None);
    }

    #[test]
    fn issue_dependencies_are_read_in_one_query_and_only_open_blockers_block() {
        let issue = |repository: &str, number| crate::issues::IssueSnapshot {
            reference: crate::issues::IssueReference {
                repository: repository.into(),
                number,
            },
            title: String::new(),
            url: String::new(),
            state: "OPEN".into(),
            project_status: None,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            closed_at_unix_ms: None,
            blocked_by: Vec::new(),
            sub_issues: Default::default(),
            labels: Vec::new(),
            author: None,
            assignees: Vec::new(),
        };
        let query = dependency_query(&[
            issue("acme/app", 171),
            issue("acme/app", 172),
            issue("acme/other", 9),
        ])
        .unwrap();
        assert!(query.starts_with("query HideIssueDependencies {"));
        assert_eq!(query.matches(":repository(").count(), 2);
        assert!(query.contains("i171:issue(number:171)") && query.contains("i9:issue(number:9)"));
        assert!(query.contains("blockedBy(first:20)"));
        assert!(query.contains("subIssuesSummary{total completed} subIssues(first:100)"));
        assert!(hide_node_link::gh::allowed(&[
            "api",
            "graphql",
            "-f",
            &format!("query={query}")
        ]));

        let answer = serde_json::json!({"data": {
            "r0": {"nameWithOwner": "acme/app",
                "i171": {"number": 171, "blockedBy": {"nodes": [
                    {"number": 170, "state": "OPEN", "repository": {"nameWithOwner": "acme/app"}},
                    {"number": 160, "state": "CLOSED", "repository": {"nameWithOwner": "acme/app"}},
                ]}, "subIssuesSummary": {"total": 3, "completed": 1},
                "subIssues": {"nodes": [
                    {"number": 173, "title": "Done piece", "state": "CLOSED", "repository": {"nameWithOwner": "acme/app"}},
                    {"number": 174, "title": "Open piece", "state": "OPEN", "repository": {"nameWithOwner": "acme/app"}},
                    {"number": 5, "title": "Other repository", "state": "OPEN", "repository": {"nameWithOwner": "acme/other"}},
                ]}},
                "i172": {"number": 172, "blockedBy": {"nodes": [
                    {"number": 3, "state": "OPEN", "repository": {"nameWithOwner": "acme/other"}},
                ]}, "subIssuesSummary": {"total": 0, "completed": 0}, "subIssues": {"nodes": []}}},
            "r1": {"nameWithOwner": "acme/other", "i9": null},
        }});
        let dependencies = parse_dependencies(&answer.to_string()).unwrap();
        let reference = |value: &str| crate::issues::IssueReference::parse(value, None).unwrap();
        assert_eq!(
            dependencies[&reference("acme/app#171")].blocked_by,
            vec![reference("acme/app#170")]
        );
        assert_eq!(
            dependencies[&reference("acme/app#172")].blocked_by,
            vec![reference("acme/other#3")]
        );
        let sub_issues = &dependencies[&reference("acme/app#171")].sub_issues;
        assert_eq!((sub_issues.total, sub_issues.completed), (3, 1));
        assert_eq!(
            sub_issues
                .listed
                .iter()
                .map(|sub| (sub.reference.token(), sub.title.as_str(), sub.open))
                .collect::<Vec<_>>(),
            vec![
                ("acme/app#173".to_owned(), "Done piece", false),
                ("acme/app#174".to_owned(), "Open piece", true),
                ("acme/other#5".to_owned(), "Other repository", true),
            ]
        );
        assert_eq!(dependencies[&reference("acme/app#172")].sub_issues.total, 0);
        assert_eq!(
            dependencies.len(),
            2,
            "an issue GitHub could not return has no entry"
        );
    }

    #[test]
    fn an_issue_dependency_error_is_a_failure_not_an_empty_answer() {
        let errors =
            serde_json::json!({"errors": [{"message": "Field 'blockedBy' doesn't exist"}]});
        assert!(parse_dependencies(&errors.to_string()).is_err());
        assert!(parse_dependencies("not json").is_err());
        let no_counts = serde_json::json!({"data": {"r0": {"nameWithOwner": "acme/app",
            "i1": {"number": 1, "blockedBy": {"nodes": []}}}}});
        assert!(
            parse_dependencies(&no_counts.to_string()).is_err(),
            "an issue without sub-issue counts is a failure, not zero sub-issues"
        );
    }

    /// What `gh search prs` and `gh search issues` printed against cli/cli
    /// with `--json isDraft,number,repository,state,title,url` (gh 2.76), plus
    /// a hit from another repository, which a `repo:` qualifier in the query
    /// brings in.
    const SEARCH_PRS_OUTPUT: &str = r#"[{"isDraft":true,"number":10253,"repository":{"name":"cli","nameWithOwner":"cli/cli"},"state":"open","title":"Include headRepositoryId when creating a new PR","url":"https://github.com/cli/cli/pull/10253"},{"isDraft":false,"number":14054,"repository":{"name":"cli","nameWithOwner":"cli/cli"},"state":"closed","title":"fix: report actual error when auth status token check fails for non-401","url":"https://github.com/cli/cli/pull/14054"},{"isDraft":false,"number":13949,"repository":{"name":"cli","nameWithOwner":"cli/cli"},"state":"merged","title":"merged one","url":"https://github.com/cli/cli/pull/13949"},{"isDraft":false,"number":309,"repository":{"name":"go-gh","nameWithOwner":"cli/go-gh"},"state":"open","title":"elsewhere","url":"https://github.com/cli/go-gh/pull/309"}]"#;
    const SEARCH_ISSUES_OUTPUT: &str = r#"[{"number":14046,"repository":{"name":"cli","nameWithOwner":"cli/cli"},"state":"open","title":"Unable to use `gh pr view --web` when sandbox (Docker sbx) remote exists","url":"https://github.com/cli/cli/issues/14046"}]"#;

    #[test]
    fn search_hits_read_as_gh_prints_them_and_stay_inside_the_repository_asked() {
        use crate::model::PrState;
        let prs = parse_search(SEARCH_PRS_OUTPUT, SearchKind::Pr, &["cli/cli".to_owned()]).unwrap();
        assert_eq!(
            prs.iter()
                .map(|hit| (
                    hit.kind.as_str(),
                    hit.number,
                    hit.state.as_str(),
                    hit.pr_state
                ))
                .collect::<Vec<_>>(),
            vec![
                ("pr", 10253, "open", Some(PrState::Draft)),
                ("pr", 14054, "closed", Some(PrState::Closed)),
                ("pr", 13949, "merged", Some(PrState::Merged)),
            ],
            "the go-gh hit is another repository's"
        );
        assert_eq!(prs[0].repository, "cli/cli");
        assert_eq!(prs[0].url, "https://github.com/cli/cli/pull/10253");
        let issues = parse_search(
            SEARCH_ISSUES_OUTPUT,
            SearchKind::Issue,
            &["CLI/cli".to_owned()],
        )
        .unwrap();
        assert_eq!(issues.len(), 1, "GitHub names ignore case");
        assert_eq!(
            (
                issues[0].kind.as_str(),
                issues[0].pr_state,
                issues[0].number
            ),
            ("issue", None, 14046)
        );
        assert_eq!(
            parse_search("[]", SearchKind::Pr, &["cli/cli".to_owned()]).unwrap(),
            vec![]
        );
    }

    #[test]
    fn a_search_answer_hide_cannot_read_is_a_failure_not_an_empty_result() {
        for output in [
            "",
            "{}",
            "not json",
            r#"[{"number":1,"repository":{"nameWithOwner":"a/b"},"state":"open","title":"t"}]"#,
            r#"[{"number":1,"repository":{"nameWithOwner":"a/b"},"state":"draft","title":"t","url":"https://github.com/a/b/pull/1"}]"#,
            r#"[{"number":1,"repository":{"nameWithOwner":"a/b"},"state":"open","title":"t","url":"javascript:alert(1)"}]"#,
        ] {
            assert!(
                parse_search(output, SearchKind::Pr, &["a/b".to_owned()]).is_err(),
                "{output:?}"
            );
        }
    }

    fn hit_json(repository: &str, kind: &str, number: u32) -> String {
        let path = if kind == "pr" { "pull" } else { "issues" };
        format!(
            r#"{{"isDraft":false,"number":{number},"repository":{{"nameWithOwner":"{repository}"}},"state":"open","title":"t{number}","url":"https://github.com/{repository}/{path}/{number}"}}"#
        )
    }

    #[test]
    fn a_search_makes_one_call_per_kind_naming_every_repository_with_the_query_as_words() {
        let calls = std::cell::RefCell::new(Vec::new());
        let run = |arguments: &[&str]| {
            calls.borrow_mut().push(arguments.join(" "));
            let kind = if arguments[1] == "prs" { "pr" } else { "issue" };
            // Thirty hits a call: more than the cap, across both repositories.
            Ok(format!(
                "[{}]",
                (0..30)
                    .map(|n| hit_json(if n % 2 == 0 { "acme/a" } else { "acme/b" }, kind, 100 + n))
                    .collect::<Vec<_>>()
                    .join(",")
            ))
        };
        let repositories = vec!["acme/a".to_owned(), "acme/b".to_owned()];
        let found = search_with(run, &repositories, "reader  --limit").unwrap();
        assert_eq!(found.len(), 40);
        let kinds: Vec<_> = found.iter().map(|hit| hit.kind.as_str()).collect();
        assert_eq!(kinds[..20], ["pr"; 20][..], "pull requests first");
        assert_eq!(kinds[20..], ["issue"; 20][..]);
        let calls = calls.into_inner();
        assert_eq!(
            calls,
            vec![
                "search prs --repo acme/a --repo acme/b --limit 20 --json isDraft,number,repository,state,title,url -- reader --limit",
                "search issues --repo acme/a --repo acme/b --limit 20 --json number,repository,state,title,url -- reader --limit",
            ],
            "two calls however many repositories, the query as separate words"
        );
        assert_eq!(search_with(|_| Ok("[]".into()), &[], "q").unwrap(), vec![]);
    }

    #[test]
    fn one_failed_search_call_fails_the_whole_search_and_names_the_call() {
        let run = |arguments: &[&str]| {
            if arguments[1] == "issues" {
                Err(GhFailure::network(
                    "HTTP 403: API rate limit exceeded".into(),
                ))
            } else {
                Ok("[]".to_owned())
            }
        };
        let repositories = vec!["acme/a".to_owned(), "acme/b".to_owned()];
        let error = search_with(run, &repositories, "q").unwrap_err();
        assert!(error.contains("gh search issues"), "{error}");
        assert!(error.contains("rate limit"), "{error}");
    }

    #[test]
    fn an_unread_project_is_resolved_by_the_search_and_an_unreadable_one_fails_it() {
        let target = |root: &str, repository: Option<&str>| SearchTarget {
            root: root.into(),
            repository: repository.map(str::to_owned),
        };
        let run = |cwd: Option<&Path>, arguments: &[&str]| {
            assert_eq!(arguments, ["repo", "view", "--json", "nameWithOwner"]);
            match cwd.and_then(Path::to_str) {
                Some("/unread") => Ok(r#"{"nameWithOwner":"ACME/Known"}"#.to_owned()),
                Some("/new") => Ok(r#"{"nameWithOwner":"acme/new"}"#.to_owned()),
                Some("/gitlab") => Err(GhFailure {
                    category: GithubFailureCategory::NoGithubRemote,
                    reason: "none of the git remotes point to a known GitHub host".into(),
                }),
                _ => Err(GhFailure::network("HTTP 502".into())),
            }
        };
        // Resolved ones join the known ones once, ignoring case; a project with
        // no GitHub remote is not one to search.
        let found = resolve_repositories(
            run,
            &[
                target("/known", Some("acme/known")),
                target("/unread", None),
                target("/new", None),
                target("/gitlab", None),
            ],
        )
        .unwrap();
        assert_eq!(found, vec!["acme/known", "acme/new"]);
        assert!(
            resolve_repositories(run, &[target("/gitlab", None)])
                .unwrap()
                .is_empty()
        );
        // Any other failure fails the search: it cannot say it covered the project.
        let error = resolve_repositories(run, &[target("/new", None), target("/broken", None)])
            .unwrap_err();
        assert!(
            error.contains("/broken") && error.contains("502"),
            "{error}"
        );
    }

    const SAMPLE_LIST: &str = r#"[
      {"number":7,"title":"Add thing","headRefName":"thing","headRefOid":"0123456789abcdef0123456789abcdef01234567","baseRefName":"main","state":"OPEN","reviewDecision":"","isDraft":false,"url":"https://github.com/acme/app/pull/7","mergedAt":null,"updatedAt":"2026-10-14T01:00:00Z","createdAt":"2026-10-13T01:00:00Z","closedAt":null,"closingIssuesReferences":[],"isCrossRepository":false,
       "author":{"id":"x","is_bot":false,"login":"hoyeon","name":"H"},"mergeStateStatus":"BEHIND",
       "reviewRequests":[{"__typename":"User","login":"hoyeon"},{"__typename":"Team","name":"Core","slug":"core"},{"__typename":"Mannequin"}]},
      {"number":8,"title":"Bump","headRefName":"dependabot/x","baseRefName":"main","state":"OPEN","reviewDecision":"","isDraft":false,"url":"https://github.com/acme/app/pull/8","mergedAt":null,"updatedAt":null,"closingIssuesReferences":[],
       "author":{"is_bot":true,"login":"app/dependabot"},"mergeStateStatus":"SOMETHING_NEW","reviewRequests":[]},
      {"number":9,"title":"Old gh","headRefName":"old","baseRefName":"main","state":"OPEN","reviewDecision":"","isDraft":true,"url":"https://github.com/acme/app/pull/9","mergedAt":null,"updatedAt":null,"closingIssuesReferences":[]}
    ]"#;

    #[test]
    fn a_listed_pull_request_keeps_who_wrote_it_how_it_stands_and_whom_a_review_was_asked_of() {
        let pull_requests = parse_pull_requests(SAMPLE_LIST).unwrap();
        let first = &pull_requests[0].work;
        assert_eq!(first.author.as_deref(), Some("hoyeon"));
        assert!(!first.author_is_bot);
        assert_eq!(first.merge_state, MergeState::Behind);
        assert_eq!(
            first.review_requests,
            vec![
                ReviewRequest {
                    kind: ReviewRequestKind::User,
                    name: "hoyeon".into()
                },
                ReviewRequest {
                    kind: ReviewRequestKind::Team,
                    name: "core".into()
                },
            ],
            "a request of a kind Hide does not know is dropped, not guessed"
        );
        let bot = &pull_requests[1].work;
        assert!(bot.author_is_bot);
        assert_eq!(
            bot.merge_state,
            MergeState::Unknown,
            "a value Hide does not know keeps the place"
        );
        assert_eq!(pull_requests[2].work, PullRequestWork::default());
    }

    #[test]
    fn the_open_list_names_failed_checks_and_counts_running_ones() {
        let output = r#"[{"number":7,"statusCheckRollup":[
          {"__typename":"CheckRun","status":"COMPLETED","conclusion":"FAILURE","name":"unit","detailsUrl":"https://ci/1"},
          {"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":null,"name":"e2e"},
          {"__typename":"StatusContext","state":"PENDING","context":"deploy"},
          {"__typename":"StatusContext","state":"ERROR","context":"lint"},
          {"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS","name":"fmt"}]}]"#;
        let checks = parse_open_checks(output).unwrap();
        let seven = &checks[&7];
        assert_eq!(seven.rollup, PullRequestChecks::Failed);
        assert_eq!(
            seven
                .failed
                .iter()
                .map(|check| check.name.as_deref().unwrap_or_default())
                .collect::<Vec<_>>(),
            ["unit", "lint"]
        );
        assert_eq!(seven.pending, 2);
        assert_eq!(
            seven.counts,
            Some(crate::model::PrCheckCounts {
                total: 5,
                passed: 1,
                failed: 2,
                running: 2,
                ..Default::default()
            })
        );
    }

    #[test]
    fn check_counts_keep_waiting_skipped_and_unread_distinct_from_success() {
        let checks = parse_open_checks(
            r#"[
          {"number":1}, {"number":2,"statusCheckRollup":[]},
          {"number":3,"statusCheckRollup":[
            {"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS"},
            {"__typename":"CheckRun","status":"COMPLETED","conclusion":"SKIPPED"},
            {"__typename":"CheckRun","status":"COMPLETED","conclusion":"NEUTRAL"},
            {"__typename":"CheckRun","status":"COMPLETED","conclusion":"TIMED_OUT"},
            {"__typename":"CheckRun","status":"IN_PROGRESS"},
            {"__typename":"CheckRun","status":"QUEUED"},
            {"__typename":"StatusContext","state":"EXPECTED"},
            {"__typename":"StatusContext","state":"PENDING"},
            {"__typename":"StatusContext","state":"SUCCESS"},
            {"__typename":"StatusContext","state":"ERROR"},
            {"__typename":"CheckRun","status":"COMPLETED","conclusion":"FUTURE_CONCLUSION"}
          ]}]
        "#,
        )
        .unwrap();
        assert_eq!(
            checks[&1].counts, None,
            "an omitted rollup is unread, not zero checks"
        );
        assert_eq!(
            checks[&2].counts,
            Some(crate::model::PrCheckCounts::default())
        );
        let counts = checks[&3].counts.as_ref().unwrap();
        assert_eq!(
            *counts,
            crate::model::PrCheckCounts {
                total: 11,
                passed: 2,
                failed: 2,
                running: 2,
                waiting: 2,
                skipped: 2,
                unknown: 1,
            }
        );
        let wire = serde_json::to_value(PullRequestWork {
            check_counts: Some(counts.clone()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(wire["check_counts"]["total"], 11);
        assert_eq!(wire["check_counts"]["passed"], 2);
        assert!(
            serde_json::to_value(PullRequestWork::default())
                .unwrap()
                .get("check_counts")
                .is_none()
        );
    }

    const ASKED: &str = r#"[{"number":7},{"number":8}]"#;
    const FACTS: &str = r#"{"data":{"repository":{"pullRequests":{"nodes":[{"number":7,"mergeQueueEntry":null,"timelineItems":{"nodes":[{"createdAt":"2026-10-14T01:00:00Z"}]}},{"number":8,"mergeQueueEntry":{"position":3},"timelineItems":{"nodes":[]}}]}}}}"#;

    /// A `gh` that answers the review search and the facts query, each unless
    /// named in `failing`.
    fn gh_answering(
        failing: &'static [ExtraQuery],
    ) -> impl Fn(&[&str]) -> Result<String, GhFailure> + Sync {
        move |arguments: &[&str]| match arguments {
            ["pr", "list", ..] if failing.contains(&ExtraQuery::ReviewRequests) => {
                Err(GhFailure::network("search rate limited".to_owned()))
            }
            ["pr", "list", ..] => Ok(ASKED.to_owned()),
            ["api", "graphql", "-f", query] if failing.contains(&ExtraQuery::MergeQueue) => {
                assert!(query.contains("HidePrFacts"));
                Err(GhFailure::network("graphql schema differs".to_owned()))
            }
            ["api", "graphql", "-f", query] => {
                assert!(query.contains("HidePrFacts"));
                Ok(FACTS.to_owned())
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    /// Pull request 7 (written by hoyeon; asked of hoyeon and of team core),
    /// 8 (a bot's; no request) and 9 (a draft; no request).
    fn sample_with_requests() -> Vec<PullRequestSnapshot> {
        let mut pull_requests = parse_pull_requests(SAMPLE_LIST).unwrap();
        pull_requests[1].work.review_requests.clear();
        pull_requests
    }

    #[test]
    fn extra_facts_tell_mine_direct_and_team_asks_and_the_queue_place() {
        let mut pull_requests = sample_with_requests();
        let unread = add_extra_facts(
            &gh_answering(&[]),
            Ok("acme/app".to_owned()),
            Some("hoyeon"),
            &mut pull_requests,
        );
        assert_eq!(unread, Vec::<ExtraQuery>::new());
        assert_eq!(pull_requests[0].work.mine, Some(true));
        assert_eq!(
            pull_requests[0].work.asked_of_me,
            crate::model::ReviewAsk::Direct
        );
        assert_eq!(
            pull_requests[0].work.queue,
            crate::model::MergeQueuePlace::NotQueued
        );
        assert_eq!(pull_requests[1].work.mine, Some(false));
        assert_eq!(
            pull_requests[1].work.asked_of_me,
            crate::model::ReviewAsk::Team,
            "found by the search without a request naming the login"
        );
        assert_eq!(
            pull_requests[1].work.queue,
            crate::model::MergeQueuePlace::At(3)
        );
        assert_eq!(
            pull_requests[0].work.review_requested_at_unix_ms,
            parse_rfc3339_ms("2026-10-14T01:00:00Z")
        );
        assert_eq!(pull_requests[1].work.review_requested_at_unix_ms, None);
        assert_eq!(
            pull_requests[2].work.asked_of_me,
            crate::model::ReviewAsk::None
        );
    }

    #[test]
    fn a_failed_account_leaves_mine_unknown_and_judges_what_it_can() {
        let mut pull_requests = sample_with_requests();
        let unread = add_extra_facts(
            &gh_answering(&[]),
            Ok("acme/app".to_owned()),
            None,
            &mut pull_requests,
        );
        assert_eq!(unread, vec![ExtraQuery::Account]);
        for pull_request in &pull_requests {
            assert_eq!(pull_request.work.mine, None, "#{}", pull_request.number);
        }
        // Without the account a request cannot be told direct from a team's.
        assert_eq!(
            pull_requests[0].work.asked_of_me,
            crate::model::ReviewAsk::Unknown
        );
        // The search finds 8 and nothing in its list could name the account.
        assert_eq!(
            pull_requests[1].work.asked_of_me,
            crate::model::ReviewAsk::Unknown
        );
        assert_eq!(
            pull_requests[2].work.asked_of_me,
            crate::model::ReviewAsk::None
        );
        assert_eq!(
            pull_requests[1].work.queue,
            crate::model::MergeQueuePlace::At(3)
        );
    }

    #[test]
    fn a_failed_review_search_leaves_team_asks_unknown_and_keeps_the_rest() {
        let mut pull_requests = sample_with_requests();
        let unread = add_extra_facts(
            &gh_answering(&[ExtraQuery::ReviewRequests]),
            Ok("acme/app".to_owned()),
            Some("hoyeon"),
            &mut pull_requests,
        );
        assert_eq!(unread, vec![ExtraQuery::ReviewRequests]);
        assert_eq!(pull_requests[0].work.mine, Some(true));
        assert_eq!(
            pull_requests[0].work.asked_of_me,
            crate::model::ReviewAsk::Direct,
            "the request list names the account without the search"
        );
        assert_eq!(
            pull_requests[1].work.asked_of_me,
            crate::model::ReviewAsk::None,
            "no request that could be the account or a team"
        );
        assert_eq!(
            pull_requests[0].work.queue,
            crate::model::MergeQueuePlace::NotQueued
        );
        assert_eq!(
            pull_requests[1].work.queue,
            crate::model::MergeQueuePlace::At(3)
        );
    }

    #[test]
    fn a_failed_queue_query_leaves_the_queue_unknown_and_keeps_the_rest() {
        let mut pull_requests = sample_with_requests();
        let unread = add_extra_facts(
            &gh_answering(&[ExtraQuery::MergeQueue]),
            Ok("acme/app".to_owned()),
            Some("hoyeon"),
            &mut pull_requests,
        );
        assert_eq!(unread, vec![ExtraQuery::MergeQueue]);
        for pull_request in &pull_requests {
            assert_eq!(
                pull_request.work.queue,
                crate::model::MergeQueuePlace::Unknown,
                "#{}",
                pull_request.number
            );
            assert_eq!(pull_request.work.review_requested_at_unix_ms, None);
        }
        assert_eq!(pull_requests[0].work.mine, Some(true));
        assert_eq!(
            pull_requests[0].work.asked_of_me,
            crate::model::ReviewAsk::Direct
        );
        assert_eq!(
            pull_requests[1].work.asked_of_me,
            crate::model::ReviewAsk::Team
        );
    }

    #[test]
    fn a_repository_name_that_cannot_be_read_fails_only_the_queue_query() {
        let mut pull_requests = sample_with_requests();
        let unread = add_extra_facts(
            &gh_answering(&[]),
            Err(GhFailure::network("no repository".to_owned())),
            Some("hoyeon"),
            &mut pull_requests,
        );
        assert_eq!(unread, vec![ExtraQuery::MergeQueue]);
        assert_eq!(
            pull_requests[1].work.asked_of_me,
            crate::model::ReviewAsk::Team
        );
    }

    #[test]
    fn all_three_failing_leave_every_judgment_unknown_and_the_list_whole() {
        let mut pull_requests = sample_with_requests();
        let unread = add_extra_facts(
            &gh_answering(&[ExtraQuery::ReviewRequests, ExtraQuery::MergeQueue]),
            Ok("acme/app".to_owned()),
            None,
            &mut pull_requests,
        );
        assert_eq!(
            unread,
            vec![
                ExtraQuery::Account,
                ExtraQuery::ReviewRequests,
                ExtraQuery::MergeQueue
            ]
        );
        assert_eq!(pull_requests.len(), 3);
        for pull_request in &pull_requests {
            assert_eq!(pull_request.work.mine, None);
            assert_eq!(
                pull_request.work.queue,
                crate::model::MergeQueuePlace::Unknown
            );
        }
        assert_eq!(
            pull_requests[0].work.asked_of_me,
            crate::model::ReviewAsk::Unknown
        );
        assert_eq!(
            pull_requests[1].work.asked_of_me,
            crate::model::ReviewAsk::None,
            "a pull request nobody was asked to review was asked of nobody"
        );
    }

    #[test]
    fn the_request_list_and_the_search_settle_a_review_ask_in_every_combination() {
        use crate::model::ReviewAsk::{self, Direct, None as NotAsked, Team, Unknown};
        // (open, requests, account, found by the search, the answer)
        type Case = (
            bool,
            Vec<ReviewRequest>,
            Option<&'static str>,
            Option<bool>,
            ReviewAsk,
        );
        let user = |name: &str| ReviewRequest {
            kind: ReviewRequestKind::User,
            name: name.to_owned(),
        };
        let team = ReviewRequest {
            kind: ReviewRequestKind::Team,
            name: "core".to_owned(),
        };
        let table: Vec<Case> = vec![
            (false, vec![user("me")], Some("me"), Some(true), NotAsked),
            (true, vec![user("me")], Some("me"), Some(true), Direct),
            (true, vec![user("me")], Some("me"), None, Direct),
            (true, vec![user("me")], Some("me"), Some(false), Direct),
            (true, vec![team.clone()], Some("me"), Some(true), Team),
            (true, vec![team.clone()], Some("me"), Some(false), NotAsked),
            (true, vec![team.clone()], Some("me"), None, Unknown),
            (true, vec![user("ann")], Some("me"), None, NotAsked),
            (true, vec![], Some("me"), Some(true), Team),
            (true, vec![], Some("me"), None, NotAsked),
            (true, vec![user("me")], None, Some(true), Unknown),
            (true, vec![user("me")], None, Some(false), NotAsked),
            (true, vec![user("me")], None, None, Unknown),
            (true, vec![], None, None, NotAsked),
            (true, vec![], None, Some(true), Unknown),
        ];
        for (open, requests, account, found, expected) in table {
            assert_eq!(
                review_ask(open, &requests, account, found),
                expected,
                "open {open}, requests {requests:?}, account {account:?}, found {found:?}"
            );
        }
    }

    #[test]
    fn a_merge_queue_answer_hide_cannot_read_is_an_error_not_an_empty_queue() {
        assert!(parse_facts("not json").is_err());
        assert!(parse_facts(r#"{"data":{"repository":null}}"#).is_err());
    }
}
