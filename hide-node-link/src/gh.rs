//! The `gh` commands the core may have its own node run with the operator's
//! GitHub login, and what one answers. The node refuses any other command
//! line ([`allowed`]); the fields and caps below are the only ones a command
//! may name, and the core builds its commands from them.

use serde::{Deserialize, Serialize};

/// Why a GitHub read failed, as a code the screen words (the reason stays
/// `gh`'s own stderr, which is data).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubFailureCategory {
    NotInstalled,
    NotLoggedIn,
    NoGithubRemote,
    NetworkOrRateLimit,
}

impl GithubFailureCategory {
    /// The words for diagnostics and the errors a command reports.
    pub fn english(self) -> &'static str {
        match self {
            Self::NotInstalled => "not installed",
            Self::NotLoggedIn => "not logged in",
            Self::NoGithubRemote => "no GitHub remote",
            Self::NetworkOrRateLimit => "network or rate limit",
        }
    }
}

/// Whether `value` reads as `owner/name` alone, no host: what
/// [`valid_repository`] takes in two parts, so it can be handed to `gh` as a
/// repository.
pub fn is_repository(value: &str) -> bool {
    value.split('/').count() == 2 && valid_repository(value)
}

/// The fields `issue_detail` asks `gh issue view` for, and the only ones
/// `run_gh` lets it ask for.
pub const ISSUE_DETAIL_FIELDS: &str = "body,labels,author,assignees,comments,createdAt";

const ISSUE_DETAIL_QUERY: &str = "query=query HideIssueDetail($owner:String!,$name:String!,$number:Int!,$after:String){repository(owner:$owner,name:$name){issue(number:$number){body createdAt author{login} labels(first:100){nodes{name color}} assignees(first:100){nodes{login}} comments(first:50,after:$after){totalCount nodes{author{login} createdAt body} pageInfo{endCursor hasNextPage}}}}}";

/// Cursors come from GitHub, stay opaque, and cannot grow the next call.
pub fn issue_detail_cursor_valid(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && value.is_ascii()
}

/// A fixed, read-only detail page. Values are separate GraphQL variables.
pub fn issue_detail_arguments(
    repository: &str,
    number: u32,
    after: Option<&str>,
) -> Option<Vec<String>> {
    if !is_repository(repository)
        || number == 0
        || after.is_some_and(|value| !issue_detail_cursor_valid(value))
    {
        return None;
    }
    let (owner, name) = repository.split_once('/')?;
    Some(vec![
        "api".into(),
        "graphql".into(),
        "-f".into(),
        ISSUE_DETAIL_QUERY.into(),
        "-f".into(),
        format!("owner={owner}"),
        "-f".into(),
        format!("name={name}"),
        "-F".into(),
        format!("number={number}"),
        if after.is_some() { "-f" } else { "-F" }.into(),
        format!("after={}", after.unwrap_or("null")),
    ])
}

fn is_issue_detail_call(arguments: &[&str]) -> bool {
    let [
        "api",
        "graphql",
        "-f",
        query,
        "-f",
        owner,
        "-f",
        name,
        "-F",
        number,
        cursor_flag,
        cursor,
    ] = arguments
    else {
        return false;
    };
    if *query != ISSUE_DETAIL_QUERY {
        return false;
    }
    let Some(owner) = owner.strip_prefix("owner=") else {
        return false;
    };
    let Some(name) = name.strip_prefix("name=") else {
        return false;
    };
    let Some(number) = number
        .strip_prefix("number=")
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return false;
    };
    let after = match (*cursor_flag, *cursor) {
        ("-F", "after=null") => None,
        ("-f", value) => match value.strip_prefix("after=") {
            Some(value) => Some(value),
            None => return false,
        },
        _ => return false,
    };
    issue_detail_arguments(&format!("{owner}/{name}"), number, after).is_some_and(|expected| {
        expected
            .iter()
            .map(String::as_str)
            .eq(arguments.iter().copied())
    })
}

/// The fields `search` asks `gh search prs` and `gh search issues` for, and
/// the only ones `run_gh` lets them ask for. The search has no head branch, and
/// only a pull request has `isDraft`.
pub const SEARCH_PR_FIELDS: &str = "isDraft,number,repository,state,title,url";

pub const SEARCH_ISSUE_FIELDS: &str = "number,repository,state,title,url";

/// Most pull requests, and most issues, one search returns in all, and what
/// one `gh search` is asked for per repository.
pub const SEARCH_LIMIT: usize = 20;

/// Longest query, in characters, a search takes.
pub const SEARCH_QUERY_LIMIT: usize = 200;

/// Most repositories one search names (`runtime/issues.rs` caps the projects at the same number).
pub const SEARCH_REPOSITORY_LIMIT: usize = 20;

/// A query as `run_gh` lets it reach `gh`: some text, within the cap.
pub fn is_search_query(query: &str) -> bool {
    !query.trim().is_empty() && query.chars().count() <= SEARCH_QUERY_LIMIT
}

/// The one shape of `gh search prs|issues` Hide runs: `search <kind>`, one
/// `--repo owner/name` per repository, the cap, that kind's fixed fields, then
/// `--` and the query's words, each of which is just text to `gh`.
pub fn is_search_call(arguments: &[&str]) -> bool {
    let [first, kind, rest @ ..] = arguments else {
        return false;
    };
    let fields = match (*first, *kind) {
        ("search", "prs") => SEARCH_PR_FIELDS,
        ("search", "issues") => SEARCH_ISSUE_FIELDS,
        _ => return false,
    };
    let mut rest = rest;
    let mut repositories = 0;
    while let ["--repo", repository, tail @ ..] = rest {
        if !is_repository(repository) {
            return false;
        }
        repositories += 1;
        rest = tail;
    }
    let limit = SEARCH_LIMIT.to_string();
    let ["--limit", count, "--json", requested, "--", words @ ..] = rest else {
        return false;
    };
    (1..=SEARCH_REPOSITORY_LIMIT).contains(&repositories)
        && *count == limit
        && *requested == fields
        && !words.is_empty()
        && words.iter().map(|word| word.chars().count()).sum::<usize>() <= SEARCH_QUERY_LIMIT
        && words.iter().all(|word| is_search_query(word))
}

/// The fields `pr_feedback` asks `gh pr view` for, and the only ones
/// `run_gh` lets it ask for.
pub const PR_FEEDBACK_FIELDS: &str = "body,statusCheckRollup,reviews";

/// What the list of every pull request asks for. `statusCheckRollup` is
/// left out: asking GitHub for the checks of all 200 pull requests, merged
/// and closed ones included, is what made one read take 11 - 14 seconds
/// against the 15-second limit, and nothing draws a settled pull request's
/// checks as news.
///
/// `author`, `mergeStateStatus` and `reviewRequests` are what the Work tab's
/// PRs list groups by (PRD work-tab-pr-lens D-08, D-20): who wrote it, how it
/// stands against its base, and whom a review was asked of.
pub const PULL_REQUEST_FIELDS: &str = "number,title,headRefName,headRefOid,isCrossRepository,baseRefName,state,reviewDecision,isDraft,url,mergedAt,updatedAt,createdAt,closedAt,closingIssuesReferences,author,mergeStateStatus,reviewRequests";

/// The open pull requests a review was asked of the operator or of one of
/// their teams, `gh`'s own `@me` meaning the signed-in account. The list
/// carries numbers only: whether the ask was direct or through a team is
/// read from each pull request's `reviewRequests` against the viewer's login.
pub const REVIEW_ASKED_SEARCH: &str = "review-requested:@me";
pub const REVIEW_ASKED_FIELDS: &str = "number";

/// The second list: only the open pull requests, only their checks.
pub const OPEN_CHECK_FIELDS: &str = "number,statusCheckRollup";

/// Every pull request `gh` will return in one call. Past this, older pull
/// requests are simply absent and their branches read as having none; the
/// limit is stated here so the risk is findable from the code that takes it.
pub const PULL_REQUEST_LIMIT: &str = "200";

/// The open issues one read lists, one past what the screen shows, so the
/// last proves there are more.
pub const ISSUE_LIST_LIMIT: &str = "201";
pub const ISSUE_LIST_SEARCH: &str = "sort:updated-desc";
pub const ISSUE_LIST_FIELDS: &str =
    "number,title,url,state,labels,author,assignees,updatedAt,createdAt,closedAt";
/// The same list with the projects each issue is in.
pub const ISSUE_LIST_PROJECT_FIELDS: &str =
    "number,title,url,state,labels,author,assignees,projectItems,updatedAt,createdAt,closedAt";

/// A fresh creation-ordered read, including closed issues. A search index's
/// absence is not proof that an interrupted create did not reach GitHub.
/// `viewerDidAuthor` keeps an issue someone else filed with the same title
/// from being adopted as the one Hide made.
pub fn issue_attempt_query(repository: &str) -> Option<String> {
    if !is_repository(repository) {
        return None;
    }
    let (owner, name) = repository.split_once('/')?;
    Some(format!(
        "query=query HideSessionIssueAttempts {{ repository(owner:\"{owner}\",name:\"{name}\") {{ issues(first:100,states:[OPEN,CLOSED],orderBy:{{field:CREATED_AT,direction:DESC}}) {{ nodes {{ number title url state createdAt viewerDidAuthor }} pageInfo {{ hasNextPage }} }} }} }}"
    ))
}

fn is_issue_attempt_query(value: &str) -> bool {
    let Some(value) =
        value.strip_prefix("query=query HideSessionIssueAttempts { repository(owner:\"")
    else {
        return false;
    };
    let Some((owner, rest)) = value.split_once("\",name:\"") else {
        return false;
    };
    let Some((name, _)) = rest.split_once('"') else {
        return false;
    };
    issue_attempt_query(&format!("{owner}/{name}")).is_some_and(|query| {
        query == format!("query=query HideSessionIssueAttempts {{ repository(owner:\"{value}")
    })
}

/// The merged pull requests of one branch a worktree cleanup reads as proof
/// the branch landed.
pub const MERGED_PROOF_LIMIT: &str = "100";
pub const MERGED_PROOF_FIELDS: &str = "headRefOid,baseRefName";

/// The signed-in account's login, read once per pass beside `auth status`:
/// it tells the Work tab which pull requests are the operator's own and which
/// review asks were made of them directly.
pub const VIEWER_LOGIN: [&str; 4] = ["api", "user", "--jq", ".login"];

/// A commit as `--match-head-commit` and `expected_head_sha` take it: the
/// full lowercase hex object name, so a branch or a flag never rides there.
pub fn is_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// What one repository's open pull requests add to the 5-minute list: each
/// pull request's place in the merge queue and when a review was last asked
/// of someone on it. The newest hundred only, a cap the screen never states
/// because a queued or just-asked pull request is a recently active one.
pub fn pr_facts_query(repository: &str) -> Option<String> {
    if !is_repository(repository) {
        return None;
    }
    let (owner, name) = repository.split_once('/')?;
    Some(format!(
        "query=query HidePrFacts {{ repository(owner:\"{owner}\",name:\"{name}\") {{ pullRequests(states:OPEN,first:100,orderBy:{{field:UPDATED_AT,direction:DESC}}) {{ nodes {{ number mergeQueueEntry {{ position }} timelineItems(last:1,itemTypes:[REVIEW_REQUESTED_EVENT]) {{ nodes {{ ... on ReviewRequestedEvent {{ createdAt }} }} }} }} }} }} }}"
    ))
}

/// What the card reads about one pull request when it opens and when its head
/// moves: the head commit, the body, the files with their counts, the
/// unresolved review threads, and what the repository lets a merge be.
pub const CARD_FILE_LIMIT: usize = 100;
pub const CARD_THREAD_LIMIT: usize = 50;
/// The head commit's check runs and the reviews the card reads for its timeline
/// (PRD pr-card-and-session-record-redesign D-15): the latest completed check
/// and the latest decisive review.
pub const CARD_CHECK_LIMIT: usize = 100;
pub const CARD_REVIEW_LIMIT: usize = 50;

pub fn pr_card_query(repository: &str, number: u32) -> Option<String> {
    if !is_repository(repository) {
        return None;
    }
    let (owner, name) = repository.split_once('/')?;
    Some(format!(
        "query=query HidePrCard {{ repository(owner:\"{owner}\",name:\"{name}\") {{ mergeCommitAllowed squashMergeAllowed rebaseMergeAllowed viewerPermission defaultBranchRef {{ name }} mergeQueue {{ id }} attributes: object(expression:\"HEAD:.gitattributes\") {{ ... on Blob {{ text }} }} pullRequest(number:{number}) {{ headRefOid baseRefName body files(first:{CARD_FILE_LIMIT}) {{ totalCount nodes {{ path additions deletions }} }} reviewThreads(first:{CARD_THREAD_LIMIT}) {{ nodes {{ isResolved path line comments(first:1) {{ nodes {{ author {{ login }} body }} }} }} }} commits(last:1) {{ nodes {{ commit {{ statusCheckRollup {{ contexts(last:{CARD_CHECK_LIMIT}) {{ nodes {{ __typename ... on CheckRun {{ status conclusion completedAt }} ... on StatusContext {{ state createdAt }} }} }} }} }} }} }} reviews(last:{CARD_REVIEW_LIMIT}) {{ nodes {{ state submittedAt }} }} }} }} }}"
    ))
}

fn is_pr_facts_query(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("query=query HidePrFacts { repository(owner:\"") else {
        return false;
    };
    let Some((owner, rest)) = rest.split_once("\",name:\"") else {
        return false;
    };
    let Some((name, _)) = rest.split_once('"') else {
        return false;
    };
    pr_facts_query(&format!("{owner}/{name}")).is_some_and(|query| query == value)
}

fn is_pr_card_query(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("query=query HidePrCard { repository(owner:\"") else {
        return false;
    };
    let Some((owner, rest)) = rest.split_once("\",name:\"") else {
        return false;
    };
    let Some((name, rest)) = rest.split_once('"') else {
        return false;
    };
    let Some((_, rest)) = rest.split_once("pullRequest(number:") else {
        return false;
    };
    let Some((number, _)) = rest.split_once(')') else {
        return false;
    };
    number
        .parse::<u32>()
        .ok()
        .and_then(|number| pr_card_query(&format!("{owner}/{name}"), number))
        .is_some_and(|query| query == value)
}

/// How a pull request is merged. A repository with a merge queue takes none:
/// `gh` adds the pull request to the queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrMergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl PrMergeMethod {
    pub const fn flag(self) -> &'static str {
        match self {
            Self::Merge => "--merge",
            Self::Squash => "--squash",
            Self::Rebase => "--rebase",
        }
    }
}

/// `gh pr merge N [--method] --match-head-commit SHA`: the one merge write.
/// The head condition is not optional, so a pull request that moved after
/// the card was drawn is refused by GitHub rather than merged unseen.
pub fn pr_merge_arguments(
    number: u32,
    method: Option<PrMergeMethod>,
    head: &str,
) -> Option<Vec<String>> {
    if !is_commit(head) {
        return None;
    }
    let mut arguments = vec!["pr".to_owned(), "merge".to_owned(), number.to_string()];
    if let Some(method) = method {
        arguments.push(method.flag().to_owned());
    }
    arguments.extend(["--match-head-commit".to_owned(), head.to_owned()]);
    Some(arguments)
}

/// `gh api --method PUT repos/OWNER/NAME/pulls/N/update-branch -f
/// expected_head_sha=SHA`: the one branch-update write, which `gh pr
/// update-branch` cannot condition on a head commit.
pub fn pr_update_branch_arguments(
    repository: &str,
    number: u32,
    head: &str,
) -> Option<Vec<String>> {
    if !is_repository(repository) || !is_commit(head) {
        return None;
    }
    Some(vec![
        "api".to_owned(),
        "--method".to_owned(),
        "PUT".to_owned(),
        format!("repos/{repository}/pulls/{number}/update-branch"),
        "-f".to_owned(),
        format!("expected_head_sha={head}"),
    ])
}

fn is_pr_write(arguments: &[&str]) -> bool {
    match arguments {
        ["pr", "merge", number, "--match-head-commit", head] => {
            is_number(number) && is_commit(head)
        }
        ["pr", "merge", number, flag, "--match-head-commit", head] => {
            is_number(number)
                && is_commit(head)
                && [
                    PrMergeMethod::Merge,
                    PrMergeMethod::Squash,
                    PrMergeMethod::Rebase,
                ]
                .iter()
                .any(|method| method.flag() == *flag)
        }
        ["api", "--method", "PUT", path, "-f", field] => {
            let Some(path) = path.strip_prefix("repos/") else {
                return false;
            };
            let Some((repository, number)) = path
                .strip_suffix("/update-branch")
                .and_then(|path| path.rsplit_once("/pulls/"))
            else {
                return false;
            };
            is_repository(repository)
                && is_number(number)
                && field
                    .strip_prefix("expected_head_sha=")
                    .is_some_and(is_commit)
        }
        _ => false,
    }
}

/// A value `gh` reads as the value of the flag before it, never a flag.
fn is_value(argument: &str) -> bool {
    !argument.is_empty() && !argument.starts_with('-')
}

/// A pull request or issue number as `gh` takes it.
pub fn is_number(argument: &str) -> bool {
    !argument.is_empty() && argument.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether `arguments` is one of the `gh` command lines Hide runs: reads,
/// and the writes (docs/ARCHITECTURE.md), a new issue with a title and a
/// body, a pull request's body, and the Work tab's two, a merge and a branch
/// update, each conditioned on the head commit the operator was shown.
/// What a project's GitHub read asks of its repository: its name, the id a
/// rename keeps, and whether it is public (`공개`, PRD overview-device-tasks D-35).
pub const REPOSITORY_FIELDS: &str = "nameWithOwner,id,visibility";

pub fn allowed(arguments: &[&str]) -> bool {
    arguments == ["auth", "status"]
        || is_conditional_read(arguments)
        || is_pull_digest_read(arguments)
        || is_checks_verdict_read(arguments)
        || matches!(
            arguments,
            ["pr", "list", "--state", "open", "--limit", PULL_REQUEST_LIMIT, "--json", OPEN_CHECK_FIELDS]
                | ["pr", "list", "--state", "all", "--limit", PULL_REQUEST_LIMIT, "--json", PULL_REQUEST_FIELDS]
        )
        || matches!(
            arguments,
            ["pr", "list", "--state", "merged", "--head", branch, "--limit", MERGED_PROOF_LIMIT, "--json", MERGED_PROOF_FIELDS]
                if is_value(branch)
        )
        || matches!(
            arguments,
            ["issue", "list", "--state", state, "--limit", ISSUE_LIST_LIMIT, "--search", ISSUE_LIST_SEARCH, "--json", fields]
                if (*state == "open" || *state == "closed") && (*fields == ISSUE_LIST_FIELDS || *fields == ISSUE_LIST_PROJECT_FIELDS)
        )
        || matches!(
            arguments,
            ["pr", "list", "--state", "open", "--search", REVIEW_ASKED_SEARCH, "--limit", PULL_REQUEST_LIMIT, "--json", REVIEW_ASKED_FIELDS]
        )
        || is_pr_write(arguments)
        || arguments == VIEWER_LOGIN
        || arguments == ["repo", "view", "--json", "nameWithOwner"]
        || arguments == ["repo", "view", "--json", REPOSITORY_FIELDS]
        || (arguments.len() == 6
            && arguments[..3] == ["issue", "create", "--title"]
            && arguments[4] == "--body")
        // The backlog's 되돌리기 closes the one issue it made (PRD
        // overview-device-tasks D-21), in the repository it named.
        || matches!(
            arguments,
            ["issue", "close", number, "--repo", repository]
                if is_number(number) && is_repository(repository)
        )
        || (arguments.len() == 5
            && arguments[..2] == ["pr", "edit"]
            && is_number(arguments[2])
            && arguments[3] == "--body")
        || (arguments.len() == 5
            && arguments[..2] == ["pr", "view"]
            && is_number(arguments[2])
            && arguments[3] == "--json"
            && (arguments[4] == "body" || arguments[4] == PR_FEEDBACK_FIELDS))
        || (arguments.len() == 7
            && arguments[..2] == ["issue", "view"]
            && arguments[3] == "--repo"
            && arguments[5..] == ["--json", ISSUE_DETAIL_FIELDS])
        // The search: the registered repositories, the cap, fixed fields, and
        // the query's words after `--` so none can be read as a flag.
        || is_search_call(arguments)
        || is_issue_detail_call(arguments)
        || (arguments.len() == 4
            && arguments[..3] == ["api", "graphql", "-f"]
            && (arguments[3].starts_with("query=query HideLinkedIssues {")
                || arguments[3].starts_with("query=query HideIssueDependencies {")
                || is_issue_attempt_query(arguments[3])
                || is_pr_facts_query(arguments[3])
                || is_pr_card_query(arguments[3])))
}

/// A conditional read prints the response headers and no body: only the
/// entity tag and the rate limit matter to it, and a 304 has no body to read.
pub const CONDITIONAL_READ_BODY: &str = "--silent";

/// The header a conditional read sends with the entity tag it holds.
const IF_NONE_MATCH: &str = "If-None-Match: ";

/// `gh api -i [-H 'If-None-Match: ETAG'] --silent ENDPOINT`: the one way
/// the advisory poll asks GitHub whether a list changed. GitHub answers 304,
/// which does not count against the rate limit, when the tag still holds.
pub fn conditional_read_arguments(endpoint: &str, etag: Option<&str>) -> Option<Vec<String>> {
    if !is_conditional_endpoint(endpoint) || etag.is_some_and(|etag| !is_etag(etag)) {
        return None;
    }
    let mut arguments = vec!["api".to_owned(), "-i".to_owned()];
    if let Some(etag) = etag {
        arguments.push("-H".to_owned());
        arguments.push(format!("{IF_NONE_MATCH}{etag}"));
    }
    arguments.extend([CONDITIONAL_READ_BODY.to_owned(), endpoint.to_owned()]);
    Some(arguments)
}

/// What the poll keeps of the open pull request list: each pull request's
/// number, head commit and draft flag, so a comment, a label, a review or a
/// new `updated_at` does not look like a change. `gh` prints the one string.
pub const PULL_DIGEST_JQ: &str = "map([.number,.head.sha,.draft])|tostring";

/// `gh api --jq PULL_DIGEST_JQ repos/OWNER/NAME/pulls?state=open&per_page=100`:
/// asked after the list's entity tag moved, to learn whether the pull
/// requests themselves did.
pub fn pull_digest_arguments(repository: &str) -> Option<Vec<String>> {
    if !is_repository(repository) {
        return None;
    }
    Some(vec![
        "api".to_owned(),
        "--jq".to_owned(),
        PULL_DIGEST_JQ.to_owned(),
        format!("repos/{repository}/pulls?state=open&per_page=100"),
    ])
}

/// The result a commit's check runs add up to, in the words the PR mark uses,
/// by the same rule as the core's `rollup_checks`: any failed run fails it,
/// else any unfinished run leaves it pending.
pub const CHECK_RUNS_VERDICT_JQ: &str = "[.check_runs[]|if .status!=\"completed\" then \"pending\" elif (.conclusion|IN(\"failure\",\"timed_out\",\"cancelled\",\"action_required\",\"startup_failure\",\"stale\")) then \"failed\" elif (.conclusion|IN(\"success\",\"neutral\",\"skipped\")) then \"passing\" else \"unknown\" end]|if any(.==\"failed\") then \"failed\" elif any(.==\"pending\") then \"pending\" elif any(.==\"unknown\") then \"unknown\" elif length==0 then \"none\" else \"passing\" end";

/// The same for a commit's combined status (a commit with no statuses reports
/// `pending` with a count of zero, which is no result).
pub const COMMIT_STATUS_VERDICT_JQ: &str = "if .total_count==0 then \"none\" else (.state|if .==\"success\" then \"passing\" elif .==\"pending\" then \"pending\" elif IN(\"failure\",\"error\") then \"failed\" else \"unknown\" end) end";

/// Which of a commit's two check lists a verdict read is of.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChecksList {
    CheckRuns,
    Status,
}

/// `gh api --jq VERDICT repos/OWNER/NAME/commits/SHA/(check-runs?per_page=100|status)`:
/// asked after a list's entity tag moved, to learn whether the result the
/// mark shows moved, and not only one job of many.
pub fn checks_verdict_arguments(
    repository: &str,
    commit: &str,
    list: ChecksList,
) -> Option<Vec<String>> {
    if !is_repository(repository) || !is_commit(commit) {
        return None;
    }
    let (filter, tail) = match list {
        ChecksList::CheckRuns => (CHECK_RUNS_VERDICT_JQ, CHECK_RUNS_TAIL),
        ChecksList::Status => (COMMIT_STATUS_VERDICT_JQ, "status"),
    };
    Some(vec![
        "api".to_owned(),
        "--jq".to_owned(),
        filter.to_owned(),
        format!("repos/{repository}/commits/{commit}/{tail}"),
    ])
}

/// The check runs list as the poll reads it: a hundred a page, so a commit
/// with many jobs is not cut at GitHub's default thirty.
pub const CHECK_RUNS_TAIL: &str = "check-runs?per_page=100";

fn is_checks_verdict_read(arguments: &[&str]) -> bool {
    let ["api", "--jq", filter, endpoint] = arguments else {
        return false;
    };
    let Some((repository, rest)) = endpoint
        .strip_prefix("repos/")
        .and_then(|rest| rest.split_once("/commits/"))
    else {
        return false;
    };
    let Some((commit, list)) = rest.split_once('/') else {
        return false;
    };
    is_repository(repository)
        && is_commit(commit)
        && ((list == CHECK_RUNS_TAIL && *filter == CHECK_RUNS_VERDICT_JQ)
            || (list == "status" && *filter == COMMIT_STATUS_VERDICT_JQ))
}

fn is_pull_digest_read(arguments: &[&str]) -> bool {
    matches!(arguments, ["api", "--jq", PULL_DIGEST_JQ, endpoint]
        if endpoint.strip_prefix("repos/")
            .and_then(|rest| rest.strip_suffix("/pulls?state=open&per_page=100"))
            .is_some_and(is_repository))
}

fn is_conditional_read(arguments: &[&str]) -> bool {
    match arguments {
        ["api", "-i", CONDITIONAL_READ_BODY, endpoint] => is_conditional_endpoint(endpoint),
        ["api", "-i", "-H", header, CONDITIONAL_READ_BODY, endpoint] => {
            header.strip_prefix(IF_NONE_MATCH).is_some_and(is_etag)
                && is_conditional_endpoint(endpoint)
        }
        _ => false,
    }
}

/// The three lists the poll watches, in one repository (`owner/name`): its
/// open pull requests, and the check runs and the commit status of one head
/// commit.
pub fn is_conditional_endpoint(endpoint: &str) -> bool {
    let Some(rest) = endpoint.strip_prefix("repos/") else {
        return false;
    };
    let mut parts = rest.splitn(3, '/');
    let (Some(owner), Some(name), Some(tail)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if !is_repository(&format!("{owner}/{name}")) {
        return false;
    }
    tail == "pulls?state=open&per_page=100"
        || tail
            .strip_prefix("commits/")
            .and_then(|tail| tail.split_once('/'))
            .is_some_and(|(commit, list)| {
                is_commit(commit) && (list == CHECK_RUNS_TAIL || list == "status")
            })
}

/// An entity tag as GitHub sends it, `"hex"` or the weak `W/"hex"`: quoted
/// plain characters, so one cannot carry a second header or a flag.
pub fn is_etag(value: &str) -> bool {
    let quoted = value.strip_prefix("W/").unwrap_or(value);
    let Some(inner) = quoted
        .strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
    else {
        return false;
    };
    !inner.is_empty()
        && inner.len() <= 128
        && inner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

/// Whether `value` names a repository as `gh` takes it, `[HOST/]OWNER/NAME`:
/// two or three plain components, none of which can be read as a flag or a
/// path step (`.` or `..`). A name may start with a dot (`acme/.github`),
/// and a host may carry its port (`ghe.example.com:8443`).
pub fn valid_repository(value: &str) -> bool {
    let parts: Vec<&str> = value.split('/').collect();
    let plain = |part: &str| {
        !part.is_empty()
            && !part.starts_with('-')
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    };
    let host = |part: &str| match part.split_once(':') {
        Some((name, port)) => {
            plain(name) && !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit())
        }
        None => plain(part),
    };
    value.len() <= 256
        && match parts.as_slice() {
            [owner, name] => plain(owner) && plain(name),
            [server, owner, name] => host(server) && plain(owner) && plain(name),
            _ => false,
        }
}

/// The repository an `origin` URL names, as [`valid_repository`] takes it:
/// `OWNER/NAME` on github.com and `HOST/OWNER/NAME` elsewhere, from the
/// SSH (`git@host:owner/name.git`, `ssh://git@host/owner/name`) and HTTPS
/// forms. `None` for any other shape.
pub fn repository_of_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url.strip_prefix("https://") {
        // An HTTPS port is the server's own (a GitHub Enterprise Server on
        // 8443), which `gh` needs to reach its API.
        let rest = rest.rsplit_once('@').map_or(rest, |(_, rest)| rest);
        rest.split_once('/')?
    } else if let Some(rest) = url
        .strip_prefix("ssh://")
        .or_else(|| url.strip_prefix("git://"))
    {
        // An SSH or git port is not the API's, so it is left out.
        let rest = rest.rsplit_once('@').map_or(rest, |(_, rest)| rest);
        let (host, path) = rest.split_once('/')?;
        (host.split(':').next()?, path)
    } else {
        let (user_host, path) = url.split_once(':')?;
        let host = user_host
            .rsplit_once('@')
            .map_or(user_host, |(_, host)| host);
        (host, path)
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    if name.contains('/') {
        return None;
    }
    let host = host.to_ascii_lowercase();
    let repository = if host == "github.com" || host == "github.com:443" {
        format!("{owner}/{name}")
    } else {
        format!("{host}/{owner}/{name}")
    };
    valid_repository(&repository).then_some(repository)
}

/// What one `gh` command answered: its output, or why it gave none.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum GhAnswer {
    Output {
        stdout: String,
    },
    Failed {
        category: GithubFailureCategory,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Detail paging may vary only its repository, issue and bounded cursor;
    /// a remote node never accepts an arbitrary query or write (B14).
    #[test]
    fn detail_pages_accept_only_the_fixed_read_query() {
        for after in [None, Some("Y3Vyc29yOjUw")] {
            let args = issue_detail_arguments("acme/app", 42, after).unwrap();
            let refs: Vec<_> = args.iter().map(String::as_str).collect();
            assert!(allowed(&refs));
            let mut changed = args.clone();
            changed[3] = changed[3].replace("comments(first:50", "comments(first:500");
            assert!(!allowed(
                &changed.iter().map(String::as_str).collect::<Vec<_>>()
            ));
            changed[3] =
                "query=mutation { deleteIssue(input:{issueId:\"x\"}) { clientMutationId } }".into();
            assert!(!allowed(
                &changed.iter().map(String::as_str).collect::<Vec<_>>()
            ));
        }
        for after in ["", "한글", &"x".repeat(1025)] {
            assert!(issue_detail_arguments("acme/app", 42, Some(after)).is_none());
        }
        assert!(issue_detail_arguments("acme/app", 0, None).is_none());
        assert!(issue_detail_arguments("acme/app/extra", 42, None).is_none());
    }

    #[test]
    fn a_remote_url_names_its_repository_and_nothing_else_does() {
        for (url, repository) in [
            ("git@github.com:acme/app.git", "acme/app"),
            ("https://github.com/acme/app", "acme/app"),
            ("https://token@github.com/acme/app.git/", "acme/app"),
            ("ssh://git@github.com:22/acme/app.git", "acme/app"),
            (
                "git@ghe.example.com:acme/app.git",
                "ghe.example.com/acme/app",
            ),
            ("git@github.com:acme/.github.git", "acme/.github"),
            (
                "https://ghe.example.com:8443/acme/app.git",
                "ghe.example.com:8443/acme/app",
            ),
            (
                "ssh://git@ghe.example.com:2222/acme/app.git",
                "ghe.example.com/acme/app",
            ),
        ] {
            assert_eq!(
                repository_of_remote(url).as_deref(),
                Some(repository),
                "{url}"
            );
        }
        for url in [
            "/srv/git/app.git",
            "git@github.com:acme/app/extra.git",
            "git@github.com:-acme/app.git",
            "https://github.com/acme",
        ] {
            assert_eq!(repository_of_remote(url), None, "{url}");
        }
        assert!(!valid_repository("acme/../app"));
        assert!(!valid_repository("acme/.."));
        assert!(!valid_repository("./app"));
        assert!(!valid_repository("--repo/app"));
        assert!(!valid_repository("ghe.example.com:/acme/app"));
        assert!(!valid_repository("ghe.example.com:84a3/acme/app"));
        assert!(!valid_repository("acme:1/app"), "a port only on a host");
        assert!(valid_repository("acme/.github"));
    }

    /// `OWNER/NAME` is read by the one validator: what it refuses (a part
    /// that could read as a flag or a path step) is refused here too, and a
    /// host is not taken where only `OWNER/NAME` is.
    #[test]
    fn an_owner_and_name_is_what_the_repository_validator_takes_without_a_host() {
        for value in ["acme/app", "acme/.github", "a_b/c-d.e"] {
            assert!(is_repository(value), "{value}");
        }
        for value in [
            "-acme/app",
            "acme/-app",
            "acme/..",
            "acme",
            "ghe.example.com/acme/app",
            "acme/app/x",
        ] {
            assert!(!is_repository(value), "{value}");
        }
    }

    #[test]
    fn a_search_takes_its_repositories_the_cap_fixed_fields_and_the_query_words_after_the_dashes() {
        let prs = |repositories: &[&'static str], words: &[&'static str]| {
            let mut arguments = vec!["search", "prs"];
            for repository in repositories {
                arguments.extend(["--repo", repository]);
            }
            arguments.extend(["--limit", "20", "--json", SEARCH_PR_FIELDS, "--"]);
            arguments.extend(words);
            arguments
        };
        assert!(allowed(&prs(&["acme/app"], &["reader"])));
        assert!(allowed(&prs(
            &["acme/app", "acme/other"],
            &["two", "words"]
        )));
        assert!(
            allowed(&prs(&["acme/app"], &["--web"])),
            "a word is never a flag"
        );
        assert!(allowed(&[
            "search",
            "issues",
            "--repo",
            "acme/app",
            "--limit",
            "20",
            "--json",
            SEARCH_ISSUE_FIELDS,
            "--",
            "-w",
        ]));
        let twenty = ["acme/app"; SEARCH_REPOSITORY_LIMIT];
        assert!(allowed(&prs(&twenty, &["q"])));
        let long = "x".repeat(SEARCH_QUERY_LIMIT + 1);
        let long: &'static str = Box::leak(long.into_boxed_str());
        let twenty_one = ["acme/app"; SEARCH_REPOSITORY_LIMIT + 1];
        let refused: Vec<Vec<&str>> = vec![
            prs(&["acme/app"], &[""]),
            prs(&["acme/app"], &["  "]),
            prs(&["acme/app"], &[]),
            prs(&["acme/app"], &[long]),
            prs(&["acme/app"], &[&long[..150], &long[..100]]),
            prs(&twenty_one, &["q"]),
            prs(&[], &["q"]),
            prs(&["acme/app/extra"], &["q"]),
            prs(&["acme"], &["q"]),
            prs(&["--repo"], &["q"]),
            prs(&["acme/../app"], &["q"]),
            // A different limit, field list, flag position or subcommand.
            vec![
                "search",
                "prs",
                "--repo",
                "acme/app",
                "--limit",
                "21",
                "--json",
                SEARCH_PR_FIELDS,
                "--",
                "q",
            ],
            vec![
                "search",
                "prs",
                "--repo",
                "acme/app",
                "--limit",
                "20",
                "--json",
                SEARCH_ISSUE_FIELDS,
                "--",
                "q",
            ],
            vec![
                "search",
                "issues",
                "--repo",
                "acme/app",
                "--limit",
                "20",
                "--json",
                SEARCH_PR_FIELDS,
                "--",
                "q",
            ],
            vec![
                "search", "prs", "--repo", "acme/app", "--limit", "20", "--json", "body", "--", "q",
            ],
            vec![
                "search",
                "prs",
                "--limit",
                "20",
                "--repo",
                "acme/app",
                "--json",
                SEARCH_PR_FIELDS,
                "--",
                "q",
            ],
            vec![
                "search",
                "prs",
                "--repo",
                "acme/app",
                "--limit",
                "20",
                "--json",
                SEARCH_PR_FIELDS,
                "q",
                "--",
            ],
            vec![
                "search",
                "prs",
                "--repo",
                "acme/app",
                "--limit",
                "20",
                "--json",
                SEARCH_PR_FIELDS,
                "q",
            ],
            vec![
                "search",
                "code",
                "--repo",
                "acme/app",
                "--limit",
                "20",
                "--json",
                SEARCH_PR_FIELDS,
                "--",
                "q",
            ],
            vec![
                "search", "repos", "--limit", "20", "--json", "name", "--", "q",
            ],
            vec!["search", "prs", "q"],
            vec!["search", "prs", "--web", "q"],
        ];
        for arguments in &refused {
            assert!(!allowed(arguments), "{arguments:?} must be refused");
        }
    }

    /// The list reads are their exact command lines: no other state, limit,
    /// field list or flag, and a branch that would read as a flag is refused.
    #[test]
    fn a_list_read_is_its_exact_command_line() {
        let pr_list = |state: &'static str, fields: &'static str| {
            vec![
                "pr",
                "list",
                "--state",
                state,
                "--limit",
                PULL_REQUEST_LIMIT,
                "--json",
                fields,
            ]
        };
        let merged = |branch: &'static str| {
            vec![
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
            ]
        };
        let issues = |fields: &'static str| {
            vec![
                "issue",
                "list",
                "--state",
                "open",
                "--limit",
                ISSUE_LIST_LIMIT,
                "--search",
                ISSUE_LIST_SEARCH,
                "--json",
                fields,
            ]
        };
        for arguments in [
            vec!["auth", "status"],
            pr_list("open", OPEN_CHECK_FIELDS),
            pr_list("all", PULL_REQUEST_FIELDS),
            merged("factory/1-task"),
            issues(ISSUE_LIST_FIELDS),
            issues(ISSUE_LIST_PROJECT_FIELDS),
        ] {
            assert!(allowed(&arguments), "{arguments:?} must be allowed");
        }
        let mut with_web = pr_list("all", PULL_REQUEST_FIELDS);
        with_web.push("--web");
        let mut token = vec!["auth", "status"];
        token.push("--show-token");
        for arguments in [
            token,
            with_web,
            pr_list("closed", PULL_REQUEST_FIELDS),
            pr_list("open", PULL_REQUEST_FIELDS),
            pr_list("all", "body"),
            merged("--web"),
            merged(""),
            issues("body"),
            vec!["issue", "list"],
            vec!["pr", "list"],
        ] {
            assert!(!allowed(&arguments), "{arguments:?} must be refused");
        }
    }

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

    /// The two new writes are their exact command lines, conditioned on a
    /// full head commit: no merge without the condition, no extra flag that
    /// would bypass checks, delete a branch, or queue an auto-merge.
    #[test]
    fn a_merge_and_a_branch_update_are_their_exact_command_lines_with_a_head_commit() {
        for arguments in [
            vec!["pr", "merge", "7", "--match-head-commit", HEAD],
            vec!["pr", "merge", "7", "--squash", "--match-head-commit", HEAD],
            vec!["pr", "merge", "7", "--merge", "--match-head-commit", HEAD],
            vec!["pr", "merge", "7", "--rebase", "--match-head-commit", HEAD],
        ] {
            assert!(allowed(&arguments), "{arguments:?} must be allowed");
        }
        let update = pr_update_branch_arguments("acme/app", 7, HEAD).unwrap();
        let update: Vec<&str> = update.iter().map(String::as_str).collect();
        assert!(allowed(&update), "{update:?} must be allowed");
        for arguments in [
            vec!["pr", "merge", "7"],
            vec!["pr", "merge", "7", "--squash"],
            vec![
                "pr",
                "merge",
                "7",
                "--squash",
                "--match-head-commit",
                "abc123",
            ],
            vec!["pr", "merge", "7", "--match-head-commit", "main"],
            vec!["pr", "merge", "7", "--admin", "--match-head-commit", HEAD],
            vec!["pr", "merge", "7", "--auto", "--match-head-commit", HEAD],
            vec![
                "pr",
                "merge",
                "7",
                "--delete-branch",
                "--match-head-commit",
                HEAD,
            ],
            vec!["pr", "merge", "-7", "--match-head-commit", HEAD],
            vec![
                "pr",
                "merge",
                "7",
                "--squash",
                "--rebase",
                "--match-head-commit",
                HEAD,
            ],
            vec!["pr", "close", "7"],
            vec![
                "api",
                "--method",
                "PUT",
                "repos/acme/app/pulls/7/update-branch",
            ],
            vec![
                "api",
                "--method",
                "DELETE",
                "repos/acme/app/pulls/7/update-branch",
                "-f",
                "expected_head_sha=0123456789abcdef0123456789abcdef01234567",
            ],
            vec![
                "api",
                "--method",
                "PUT",
                "repos/acme/app/pulls/7/merge",
                "-f",
                "expected_head_sha=0123456789abcdef0123456789abcdef01234567",
            ],
            vec![
                "api",
                "--method",
                "PUT",
                "repos/acme/../app/pulls/7/update-branch",
                "-f",
                "expected_head_sha=0123456789abcdef0123456789abcdef01234567",
            ],
            vec![
                "api",
                "--method",
                "PUT",
                "repos/acme/app/pulls/7/update-branch",
                "-f",
                "expected_head_sha=main",
            ],
        ] {
            assert!(!allowed(&arguments), "{arguments:?} must be refused");
        }
        assert_eq!(pr_merge_arguments(7, None, "main"), None);
        assert_eq!(pr_update_branch_arguments("-acme/app", 7, HEAD), None);
        assert_eq!(pr_update_branch_arguments("acme/app", 7, "HEAD"), None);
    }

    /// The viewer and queue read and the card read are their exact queries
    /// for a repository (and a number), never text a caller edited.
    #[test]
    fn the_queue_and_card_queries_are_only_the_ones_this_crate_builds() {
        let queue = pr_facts_query("acme/app").unwrap();
        let card = pr_card_query("acme/app", 12).unwrap();
        for query in [&queue, &card] {
            assert!(allowed(&["api", "graphql", "-f", query]), "{query}");
        }
        let edited_queue = queue.replace("number", "number body");
        let edited_card = card.replace("body ", "body mergeCommit ");
        let other_number = card.replace("number:12", "number:12) { id } other(number:1");
        for query in [&edited_queue, &edited_card, &other_number] {
            assert!(!allowed(&["api", "graphql", "-f", query]), "{query}");
        }
        assert_eq!(pr_card_query("-acme/app", 1), None);
    }

    #[test]
    fn the_review_asked_search_is_its_exact_command_line() {
        let ask = |search: &'static str| {
            vec![
                "pr",
                "list",
                "--state",
                "open",
                "--search",
                search,
                "--limit",
                PULL_REQUEST_LIMIT,
                "--json",
                REVIEW_ASKED_FIELDS,
            ]
        };
        assert!(allowed(&ask(REVIEW_ASKED_SEARCH)));
        assert!(!allowed(&ask("author:someone")));
        assert!(!allowed(&[
            "pr",
            "list",
            "--state",
            "open",
            "--search",
            REVIEW_ASKED_SEARCH
        ]));
    }

    #[test]
    fn a_conditional_read_is_a_list_of_one_repository_with_an_optional_plain_etag() {
        const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
        let pulls = "repos/acme/app/pulls?state=open&per_page=100";
        let runs = format!("repos/acme/app/commits/{COMMIT}/check-runs?per_page=100");
        let status = format!("repos/acme/app/commits/{COMMIT}/status");
        for endpoint in [pulls, runs.as_str(), status.as_str()] {
            for etag in [None, Some("W/\"abc123\""), Some("\"abc-1.2:3_4\"")] {
                let arguments = conditional_read_arguments(endpoint, etag).expect(endpoint);
                let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
                assert!(allowed(&borrowed), "{borrowed:?}");
            }
        }
        for endpoint in [
            "repos/acme/app/pulls?state=all&per_page=100",
            "repos/acme/app/pulls",
            "repos/acme/app/commits/main/status",
            "repos/acme/app/commits/0123/check-runs",
            "repos/acme/app/issues",
            "repos/acme/../pulls?state=open&per_page=100",
            "repos/ghe.example.com/acme/app/pulls?state=open&per_page=100",
            "user",
            "graphql",
        ] {
            assert!(
                conditional_read_arguments(endpoint, None).is_none(),
                "{endpoint}"
            );
            assert!(
                !allowed(&["api", "-i", "--jq", "length", endpoint])
                    && !allowed(&["api", "-i", "--silent", "--jq", "length", endpoint]),
                "{endpoint}"
            );
        }
        for etag in [
            "",
            "abc",
            "\"\"",
            "W/abc",
            "\"a b\"",
            "\"a\\nX-Other: 1\"",
            "\"--flag\"; rm",
        ] {
            assert!(
                conditional_read_arguments(pulls, Some(etag)).is_none(),
                "{etag:?}"
            );
        }
    }

    #[test]
    fn a_conditional_read_refuses_any_other_header_flag_or_filter() {
        let pulls = "repos/acme/app/pulls?state=open&per_page=100";
        assert!(!allowed(&[
            "api",
            "-i",
            "-H",
            "Authorization: x",
            "--silent",
            pulls
        ]));
        assert!(!allowed(&["api", "-i", "--jq", "length", pulls]));
        assert!(!allowed(&["api", "--silent", pulls]));
        assert!(!allowed(&["api", "-i", "--paginate", "--silent", pulls]));
        assert!(!allowed(&[
            "api",
            "-i",
            "-H",
            "If-None-Match: \"abc\"",
            "-H",
            "X-Other: 1",
            "--silent",
            pulls
        ]));
    }

    #[test]
    fn the_pull_digest_read_is_the_open_list_of_one_repository_with_the_fixed_filter() {
        let arguments = pull_digest_arguments("acme/app").expect("a repository");
        let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
        assert!(allowed(&borrowed), "{borrowed:?}");
        assert!(pull_digest_arguments("../app").is_none());
        assert!(pull_digest_arguments("ghe.example.com/acme/app").is_none());
        let pulls = "repos/acme/app/pulls?state=open&per_page=100";
        assert!(!allowed(&["api", "--jq", ".[]", pulls]));
        assert!(!allowed(&[
            "api",
            "--jq",
            PULL_DIGEST_JQ,
            "repos/acme/app/pulls?state=all"
        ]));
        assert!(!allowed(&[
            "api",
            "--jq",
            PULL_DIGEST_JQ,
            "repos/../x/pulls?state=open&per_page=100"
        ]));
        assert!(!allowed(&[
            "api",
            "--paginate",
            "--jq",
            PULL_DIGEST_JQ,
            pulls
        ]));
    }

    #[test]
    fn a_checks_verdict_read_is_one_commit_list_with_its_own_fixed_filter() {
        const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
        for list in [ChecksList::CheckRuns, ChecksList::Status] {
            let arguments = checks_verdict_arguments("acme/app", COMMIT, list).expect("a commit");
            let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
            assert!(allowed(&borrowed), "{borrowed:?}");
        }
        assert!(checks_verdict_arguments("acme/app", "main", ChecksList::Status).is_none());
        assert!(checks_verdict_arguments("../app", COMMIT, ChecksList::Status).is_none());
        // A filter belongs to its own list, and no other filter is run.
        let runs = format!("repos/acme/app/commits/{COMMIT}/check-runs?per_page=100");
        let status = format!("repos/acme/app/commits/{COMMIT}/status");
        assert!(!allowed(&["api", "--jq", COMMIT_STATUS_VERDICT_JQ, &runs]));
        assert!(!allowed(&["api", "--jq", CHECK_RUNS_VERDICT_JQ, &status]));
        assert!(!allowed(&["api", "--jq", ".check_runs", &runs]));
        assert!(!allowed(&[
            "api",
            "--jq",
            CHECK_RUNS_VERDICT_JQ,
            "repos/acme/app/commits/main/check-runs?per_page=100"
        ]));
    }
}
