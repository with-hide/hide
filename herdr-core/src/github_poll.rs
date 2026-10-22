//! The advisory poll: every minute, ask GitHub whether the lists behind a
//! project's pull request marks changed, and say so.
//!
//! It never writes pull request state. It reads three kinds of list with the
//! entity tag it last saw (`If-None-Match`), through the operator's own `gh`:
//! a repository's open pull requests, and the check runs and the commit status
//! of the head commit of an open pull request whose checks were pending. A 304
//! means nothing changed and costs no rate limit; a different tag means
//! "something changed", and the owner then asks the existing full read
//! through the rule a look uses (`Runtime::ask_polled_changes`).
//!
//! The poll is contained so that nothing it does can reach the existing read,
//! the screen, or another project:
//! - it holds no `Runtime`; its only output is a list of project paths;
//! - a failure pauses that project alone, with a backoff, and is written to the
//!   diagnostic log once per kind, never to the screen;
//! - the five-minute clock and its failure backoff are the runtime's and are
//!   never touched here;
//! - below a tenth of the rate limit left, nothing is polled until it resets.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hide_node_link::gh::{
    CHECK_RUNS_TAIL, ChecksList, GhAnswer, checks_verdict_arguments, conditional_read_arguments,
    pull_digest_arguments,
};
use hide_node_link::protocol::Call;

use crate::model::GithubFailureCategory;
use crate::node_access::NodeLink;
use crate::reader::BackgroundRead;

/// How often a pass runs.
const POLL_EVERY: Duration = Duration::from_secs(60);

/// The most projects one pass reads, the one in front first.
pub(crate) const POLL_PROJECT_LIMIT: usize = 8;

/// The most pull requests per project whose checks are read.
pub(crate) const POLL_PENDING_LIMIT: usize = 4;

/// A pass that has run this long stops and leaves the rest to the next, so a
/// slow network never makes passes overlap.
const POLL_PASS_BUDGET: Duration = Duration::from_secs(45);

/// A failed project waits this long, then twice as long, up to the cap.
const POLL_BACKOFF_FIRST: Duration = Duration::from_secs(60);
const POLL_BACKOFF_LAST: Duration = Duration::from_secs(15 * 60);

/// Polling stops when fewer than one in this many requests remain of the
/// limit. The poll spends a handful of full answers an hour; ten percent
/// (500 of the 5,000 an account has) leaves the operator's other tools the
/// rest of the hour.
const RATE_FLOOR_DIVISOR: u64 = 10;

/// What a `gh` command may take.
const ASK_TIMEOUT: Duration = Duration::from_secs(20);

/// What one pass reads: at most `POLL_PROJECT_LIMIT` projects.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct PollRequest {
    pub(crate) projects: Vec<PollProject>,
}

impl PollRequest {
    pub(crate) fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }
}

/// One project the poll watches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PollProject {
    /// The project's root path, which names it to the runtime.
    pub(crate) path: String,
    /// The lowercase `owner/name` its open pull requests are in.
    pub(crate) repository: String,
    /// The head commits of open pull requests whose checks were pending, at
    /// most `POLL_PENDING_LIMIT`.
    pub(crate) heads: Vec<String>,
}

/// The projects a pass found a change in, and those it is not watching now.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct PollAnswer {
    pub(crate) changed: Vec<String>,
    /// Paused after a failure, or every project at the rate limit's floor:
    /// their marks are stale until a read answers (PRD git-freshness B23).
    pub(crate) held: Vec<String>,
}

/// A failed `gh` command.
#[derive(Clone, Debug)]
pub(crate) struct AskFailure {
    pub(crate) category: GithubFailureCategory,
    pub(crate) reason: String,
}

type Ask<'a> = &'a dyn Fn(&[String]) -> Result<String, AskFailure>;

/// Drives the poll on one worker thread, one `gh` at a time.
pub struct GithubPollReader {
    inner: BackgroundRead<PollRequest, PollAnswer>,
    cancel: Arc<AtomicBool>,
    /// A non-empty request was polled and the empty one that follows has not
    /// been, so a read still running is collected and the tags are let go.
    active: bool,
}

impl GithubPollReader {
    /// A reader whose `gh` runs on `node`, where the operator's login is.
    pub fn new(node: Arc<dyn NodeLink>) -> Self {
        let state = Mutex::new(PollState::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        Self {
            inner: BackgroundRead::new(POLL_EVERY, POLL_EVERY, move |request: &PollRequest| {
                let clock = Clock {
                    now: Instant::now(),
                    unix: unix_seconds(),
                };
                let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                pass(
                    &mut state,
                    request,
                    &|arguments| ask(node.as_ref(), arguments),
                    clock,
                    &worker_cancel,
                )
            }),
            cancel,
            active: false,
        }
    }

    /// The projects changed since the pass before, when a pass finished on
    /// this wake; nothing otherwise. An empty request reads nothing.
    pub fn read_if_due(&mut self, request: PollRequest) -> Option<PollAnswer> {
        if request.is_empty() && !self.active {
            return None;
        }
        self.active = !request.is_empty();
        self.inner.poll(request)
    }
}

impl Drop for GithubPollReader {
    /// The worker ends with its owner: it looks at this flag before each
    /// `gh`, so it stops after the one it is in, which the node bounds.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn ask(node: &dyn NodeLink, arguments: &[String]) -> Result<String, AskFailure> {
    let call = Call::Gh {
        cwd: None,
        args: arguments.to_vec(),
        repository: None,
    };
    match crate::node_access::call_as(node, call, ASK_TIMEOUT) {
        Ok(GhAnswer::Output { stdout }) => Ok(stdout),
        Ok(GhAnswer::Failed { category, reason }) => Err(AskFailure { category, reason }),
        Err(error) => Err(AskFailure {
            category: GithubFailureCategory::NetworkOrRateLimit,
            reason: format!("gh could not be asked for: {error}"),
        }),
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The two clocks a pass reads, handed in so a test moves them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Clock {
    pub(crate) now: Instant,
    pub(crate) unix: u64,
}

/// What the poll remembers between passes, all in memory.
#[derive(Debug, Default)]
pub(crate) struct PollState {
    /// The entity tag last seen for each endpoint, kept only for the
    /// endpoints of the last request: at most
    /// `POLL_PROJECT_LIMIT * (1 + 2 * POLL_PENDING_LIMIT)` of them.
    tags: HashMap<String, String>,
    /// A digest of each watched project's open pull request list: number,
    /// head commit and draft flag of every pull request, and nothing else, so
    /// a comment, a label or a review that moves the list's tag asks nothing.
    /// At most `POLL_PROJECT_LIMIT` entries, kept for the last request's projects.
    digests: HashMap<String, u64>,
    /// What each watched checks list last added up to (`passing`, `pending`,
    /// `failed`, `unknown` or `none`), by endpoint; at most one per endpoint of
    /// the last request.
    verdicts: HashMap<String, &'static str>,
    /// The result a head commit's two lists add up to, by project path and
    /// head, which is what the mark shows: only a move here asks a read.
    combined: HashMap<(String, String), &'static str>,
    /// The projects waiting out a failure, at most `POLL_PROJECT_LIMIT`.
    pauses: HashMap<String, Pause>,
    /// Polling is stopped until this unix second: the rate limit's floor.
    rate_until: Option<u64>,
}

#[derive(Debug)]
struct Pause {
    failures: u32,
    until: Instant,
    kind: &'static str,
}

/// Reads every project of `request` once and says which changed.
pub(crate) fn pass(
    state: &mut PollState,
    request: &PollRequest,
    ask: Ask<'_>,
    clock: Clock,
    cancel: &AtomicBool,
) -> PollAnswer {
    let wanted: BTreeSet<String> = request.projects.iter().flat_map(endpoints).collect();
    state.tags.retain(|endpoint, _| wanted.contains(endpoint));
    let watched = |path: &String| request.projects.iter().any(|project| &project.path == path);
    state.digests.retain(|path, _| watched(path));
    state
        .verdicts
        .retain(|endpoint, _| wanted.contains(endpoint));
    state.combined.retain(|(path, head), _| {
        request
            .projects
            .iter()
            .any(|project| &project.path == path && project.heads.contains(head))
    });
    state.pauses.retain(|path, _| watched(path));
    let mut answer = PollAnswer::default();
    if let Some(until) = state.rate_until {
        if clock.unix < until {
            answer.held = request
                .projects
                .iter()
                .map(|project| project.path.clone())
                .collect();
            return answer;
        }
        state.rate_until = None;
    }
    let started = Instant::now();
    'projects: for project in &request.projects {
        if state
            .pauses
            .get(&project.path)
            .is_some_and(|pause| clock.now < pause.until)
        {
            continue;
        }
        let mut changed = false;
        let mut causes: Vec<&'static str> = Vec::new();
        let mut checks_tag_moved = false;
        let mut failure = None;
        for endpoint in endpoints(project) {
            if cancel.load(Ordering::Relaxed) || started.elapsed() >= POLL_PASS_BUDGET {
                break 'projects;
            }
            let known = state.tags.get(&endpoint).cloned();
            let Some(arguments) = conditional_read_arguments(&endpoint, known.as_deref()) else {
                failure = Some("endpoint_refused");
                break;
            };
            match ask(&arguments) {
                Ok(stdout) => {
                    let Some(reply) = Reply::parse(&stdout) else {
                        failure = Some("reply_unreadable");
                        break;
                    };
                    let Some(tag) = reply.tag else {
                        failure = Some("reply_without_tag");
                        break;
                    };
                    let moved = known.as_ref().is_none_or(|known| !same_tag(known, &tag));
                    if moved && is_list(&endpoint) {
                        // The list's tag moves for a comment as well as a push.
                        // Only a pull request opened, closed, pushed to or
                        // flipped to or from draft is a change worth a read.
                        let digest = match pull_digest(ask, &project.repository) {
                            Ok(digest) => digest,
                            Err(refused) => {
                                failure = Some(failure_kind(refused.category));
                                break;
                            }
                        };
                        let before = state.digests.insert(project.path.clone(), digest);
                        if known.is_some() && before != Some(digest) {
                            changed = true;
                            causes.push("pull_requests");
                        } else if known.is_some() {
                            // What a read on every tag move would have asked for.
                            crate::diagnostic!(serde_json::json!({
                                "component": "github",
                                "kind": "poll.list_tag_only",
                                "project": project.path,
                            }));
                        }
                    } else if moved {
                        checks_tag_moved |= known.is_some();
                        // One job of many finishing moves the list's tag and
                        // not the result the mark shows; read what it adds up to.
                        let list = if endpoint.ends_with("/status") {
                            ChecksList::Status
                        } else {
                            ChecksList::CheckRuns
                        };
                        match checks_verdict(ask, &project.repository, &endpoint, list) {
                            Ok(verdict) => {
                                state.verdicts.insert(endpoint.clone(), verdict);
                            }
                            Err(refused) => {
                                failure = Some(failure_kind(refused.category));
                                break;
                            }
                        }
                    }
                    state.tags.insert(endpoint, tag);
                    if let (Some(limit), Some(remaining), Some(reset)) =
                        (reply.limit, reply.remaining, reply.reset)
                        && remaining.saturating_mul(RATE_FLOOR_DIVISOR) < limit
                    {
                        state.rate_until = Some(reset.max(clock.unix + 60));
                        crate::diagnostic!(serde_json::json!({
                            "component": "github",
                            "kind": "poll.rate_floor",
                            "limit": limit,
                            "remaining": remaining,
                            "reset_unix": reset,
                        }));
                        if changed {
                            answer.changed.push(project.path.clone());
                        }
                        break 'projects;
                    }
                }
                // `gh` exits 1 on a 304 and says so on stderr: the tag still
                // holds, nothing changed, and nothing was spent.
                Err(refused) if refused.reason.contains("HTTP 304") => {}
                Err(refused) => {
                    failure = Some(failure_kind(refused.category));
                    break;
                }
            }
        }
        if failure.is_none() {
            for head in &project.heads {
                let key = (project.path.clone(), head.clone());
                let result = combine(
                    state
                        .verdicts
                        .get(&checks_endpoint(project, head, ChecksList::CheckRuns)),
                    state
                        .verdicts
                        .get(&checks_endpoint(project, head, ChecksList::Status)),
                );
                if state
                    .combined
                    .insert(key, result)
                    .is_some_and(|before| before != result)
                {
                    changed = true;
                    causes.push("checks");
                }
            }
            if checks_tag_moved && !causes.contains(&"checks") {
                // What a read on every tag move would have asked for.
                crate::diagnostic!(serde_json::json!({
                    "component": "github",
                    "kind": "poll.checks_tag_only",
                    "project": project.path,
                }));
            }
        }
        if changed {
            crate::diagnostic!(serde_json::json!({
                "component": "github",
                "kind": "poll.changed",
                "project": project.path,
                "cause": causes,
            }));
            answer.changed.push(project.path.clone());
        }
        match failure {
            None => {
                state.pauses.remove(&project.path);
            }
            Some(kind) => pause(state, project, kind, clock.now),
        }
    }
    answer.held = request
        .projects
        .iter()
        .filter(|project| state.rate_until.is_some() || state.pauses.contains_key(&project.path))
        .map(|project| project.path.clone())
        .collect();
    answer
}

/// Puts one project aside after a failure, for twice as long as the last
/// time, and states a kind of failure once for as long as it lasts.
fn pause(state: &mut PollState, project: &PollProject, kind: &'static str, now: Instant) {
    let failures = state
        .pauses
        .get(&project.path)
        .map_or(0, |pause| pause.failures)
        .saturating_add(1);
    let delay = POLL_BACKOFF_FIRST
        .saturating_mul(1u32.checked_shl(failures - 1).unwrap_or(u32::MAX))
        .min(POLL_BACKOFF_LAST);
    let repeated = state
        .pauses
        .get(&project.path)
        .is_some_and(|pause| pause.kind == kind);
    if !repeated {
        crate::diagnostic!(serde_json::json!({
            "component": "github",
            "kind": "poll.failed",
            "project": project.path,
            "failure": kind,
            "delay_ms": delay.as_millis(),
        }));
    }
    state.pauses.insert(
        project.path.clone(),
        Pause {
            failures,
            until: now + delay,
            kind,
        },
    );
}

fn failure_kind(category: GithubFailureCategory) -> &'static str {
    match category {
        GithubFailureCategory::NotInstalled => "gh_missing",
        GithubFailureCategory::NotLoggedIn => "not_logged_in",
        GithubFailureCategory::NoGithubRemote => "no_remote",
        GithubFailureCategory::NetworkOrRateLimit => "network_or_rate_limit",
    }
}

/// The lists one project's marks come from.
fn endpoints(project: &PollProject) -> Vec<String> {
    let repository = &project.repository;
    let mut endpoints = vec![format!("repos/{repository}/pulls?state=open&per_page=100")];
    for head in &project.heads {
        endpoints.push(checks_endpoint(project, head, ChecksList::CheckRuns));
        endpoints.push(checks_endpoint(project, head, ChecksList::Status));
    }
    endpoints
}

fn checks_endpoint(project: &PollProject, head: &str, list: ChecksList) -> String {
    let repository = &project.repository;
    match list {
        ChecksList::CheckRuns => format!("repos/{repository}/commits/{head}/{CHECK_RUNS_TAIL}"),
        ChecksList::Status => format!("repos/{repository}/commits/{head}/status"),
    }
}

/// What one checks list adds up to, from the one word `gh` prints.
fn checks_verdict(
    ask: Ask<'_>,
    repository: &str,
    endpoint: &str,
    list: ChecksList,
) -> Result<&'static str, AskFailure> {
    let unreadable = |reason: &str| AskFailure {
        category: GithubFailureCategory::NetworkOrRateLimit,
        reason: reason.to_owned(),
    };
    let commit = endpoint
        .split("/commits/")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .ok_or_else(|| unreadable("endpoint_refused"))?;
    let arguments = checks_verdict_arguments(repository, commit, list)
        .ok_or_else(|| unreadable("endpoint_refused"))?;
    match ask(&arguments)?.trim() {
        "passing" => Ok("passing"),
        "pending" => Ok("pending"),
        "failed" => Ok("failed"),
        "unknown" => Ok("unknown"),
        "none" => Ok("none"),
        _ => Err(unreadable("verdict_unreadable")),
    }
}

/// The result a commit's check runs and statuses add up to, as the core's
/// rollup does: a failure first, then anything unfinished, then anything
/// unrecognised.
fn combine(runs: Option<&&'static str>, status: Option<&&'static str>) -> &'static str {
    let both = [runs.copied(), status.copied()];
    for word in ["failed", "pending", "unknown", "passing"] {
        if both.contains(&Some(word)) {
            return word;
        }
    }
    "none"
}

fn is_list(endpoint: &str) -> bool {
    endpoint.ends_with("/pulls?state=open&per_page=100")
}

/// The digest of a repository's open pull requests, from the one string `gh`
/// prints for `PULL_DIGEST_JQ`.
fn pull_digest(ask: Ask<'_>, repository: &str) -> Result<u64, AskFailure> {
    let unreadable = |reason: &str| AskFailure {
        category: GithubFailureCategory::NetworkOrRateLimit,
        reason: reason.to_owned(),
    };
    let arguments =
        pull_digest_arguments(repository).ok_or_else(|| unreadable("endpoint_refused"))?;
    let text = ask(&arguments)?;
    let text = text.trim();
    if !text.starts_with('[') || !text.ends_with(']') {
        return Err(unreadable("digest_unreadable"));
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(text, &mut hasher);
    Ok(std::hash::Hasher::finish(&hasher))
}

/// A weak tag and the strong tag of the same content name one thing.
fn same_tag(left: &str, right: &str) -> bool {
    left.trim_start_matches("W/") == right.trim_start_matches("W/")
}

/// What a 200 answer's headers say, as `gh api -i` prints them.
struct Reply {
    tag: Option<String>,
    limit: Option<u64>,
    remaining: Option<u64>,
    reset: Option<u64>,
}

impl Reply {
    fn parse(stdout: &str) -> Option<Self> {
        let mut lines = stdout.lines();
        let status = lines.next()?;
        if status.split_whitespace().nth(1) != Some("200") {
            return None;
        }
        let mut reply = Self {
            tag: None,
            limit: None,
            remaining: None,
            reset: None,
        };
        for line in lines {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                break;
            }
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match name.to_ascii_lowercase().as_str() {
                "etag" => reply.tag = Some(value.to_owned()),
                "x-ratelimit-limit" => reply.limit = value.parse().ok(),
                "x-ratelimit-remaining" => reply.remaining = value.parse().ok(),
                "x-ratelimit-reset" => reply.reset = value.parse().ok(),
                _ => {}
            }
        }
        Some(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

    fn project(path: &str, heads: &[&str]) -> PollProject {
        PollProject {
            path: path.to_owned(),
            repository: format!("acme/{}", path.trim_start_matches('/')),
            heads: heads.iter().map(|head| (*head).to_owned()).collect(),
        }
    }

    fn request(projects: Vec<PollProject>) -> PollRequest {
        PollRequest { projects }
    }

    fn ok(tag: &str) -> Result<String, AskFailure> {
        ok_with(tag, 4000, 5000)
    }

    fn ok_with(tag: &str, remaining: u64, limit: u64) -> Result<String, AskFailure> {
        Ok(format!(
            "HTTP/2.0 200 OK\r\nEtag: W/\"{tag}\"\r\nX-Ratelimit-Limit: {limit}\r\n\
             X-Ratelimit-Remaining: {remaining}\r\nX-Ratelimit-Reset: 2000\r\n\r\n3\n"
        ))
    }

    /// What `gh` prints for the digest filter over these pull requests.
    fn digest_of(pulls: &[(u32, &str, bool)]) -> Result<String, AskFailure> {
        let items: Vec<String> = pulls
            .iter()
            .map(|(number, sha, draft)| format!("[{number},\"{sha}\",{draft}]"))
            .collect();
        Ok(format!("[{}]\n", items.join(",")))
    }

    fn not_modified() -> Result<String, AskFailure> {
        Err(AskFailure {
            category: GithubFailureCategory::NetworkOrRateLimit,
            reason: "gh: HTTP 304".to_owned(),
        })
    }

    fn broken() -> Result<String, AskFailure> {
        Err(AskFailure {
            category: GithubFailureCategory::NetworkOrRateLimit,
            reason: "gh: HTTP 502".to_owned(),
        })
    }

    /// A GitHub that answers each endpoint from a script and records every
    /// command line it was asked.
    struct Script {
        asked: RefCell<Vec<Vec<String>>>,
        answers: RefCell<HashMap<String, VecDeque<Result<String, AskFailure>>>>,
    }

    impl Script {
        fn new() -> Self {
            Self {
                asked: RefCell::default(),
                answers: RefCell::default(),
            }
        }

        fn will(&self, endpoint_part: &str, answer: Result<String, AskFailure>) {
            self.answers
                .borrow_mut()
                .entry(endpoint_part.to_owned())
                .or_default()
                .push_back(answer);
        }

        fn ask(&self, arguments: &[String]) -> Result<String, AskFailure> {
            self.asked.borrow_mut().push(arguments.to_vec());
            let digest = arguments.iter().any(|argument| argument == "--jq");
            let endpoint = arguments.last().cloned().unwrap_or_default();
            let mut answers = self.answers.borrow_mut();
            let queue = answers
                .iter_mut()
                .find(|(part, _)| match part.strip_prefix("digest:") {
                    Some(part) => digest && endpoint.contains(part),
                    None => !digest && endpoint.contains(part.as_str()),
                })
                .map(|(_, queue)| queue);
            queue.and_then(VecDeque::pop_front).unwrap_or_else(|| {
                if digest && endpoint.contains("/commits/") {
                    Ok("passing\n".to_owned())
                } else if digest {
                    digest_of(&[(1, "aaaa", false)])
                } else {
                    not_modified()
                }
            })
        }

        /// The conditional reads asked, by endpoint; digest reads are counted apart.
        fn asked_endpoints(&self) -> Vec<String> {
            self.asked
                .borrow()
                .iter()
                .filter(|arguments| !arguments.iter().any(|argument| argument == "--jq"))
                .filter_map(|arguments| arguments.last().cloned())
                .collect()
        }

        fn digest_reads(&self) -> usize {
            self.asked
                .borrow()
                .iter()
                .filter(|arguments| arguments.iter().any(|argument| argument == "--jq"))
                .count()
        }

        fn take_asked(&self) -> Vec<Vec<String>> {
            std::mem::take(&mut *self.asked.borrow_mut())
        }
    }

    fn run(
        state: &mut PollState,
        script: &Script,
        request: &PollRequest,
        now: Instant,
        unix: u64,
    ) -> PollAnswer {
        pass(
            state,
            request,
            &|arguments| script.ask(arguments),
            Clock { now, unix },
            &AtomicBool::new(false),
        )
    }

    #[test]
    fn the_first_sight_is_a_baseline_and_a_304_after_it_is_no_change() {
        let script = Script::new();
        script.will("/app/pulls", ok("one"));
        let mut state = PollState::default();
        let now = Instant::now();
        let request = request(vec![project("/app", &[])]);
        assert!(
            run(&mut state, &script, &request, now, 1000)
                .changed
                .is_empty()
        );
        let first = script.take_asked();
        assert_eq!(first.len(), 2, "the list, then its digest as the baseline");
        assert!(
            !first[0]
                .iter()
                .any(|argument| argument.starts_with("If-None-Match")),
            "a first sight sends no tag: {first:?}"
        );
        assert!(
            run(&mut state, &script, &request, now, 1060)
                .changed
                .is_empty()
        );
        let second = script.take_asked();
        assert!(
            second[0].contains(&"If-None-Match: W/\"one\"".to_owned()),
            "the tag it saw comes back: {second:?}"
        );
    }

    #[test]
    fn a_changed_list_names_its_project_and_no_other() {
        let script = Script::new();
        script.will("/app/pulls", ok("one"));
        script.will("/app/pulls", ok("two"));
        script.will("/other/pulls", ok("same"));
        script.will("/other/pulls", ok("same2"));
        script.will("digest:/app/pulls", digest_of(&[(1, "aaaa", false)]));
        script.will("digest:/app/pulls", digest_of(&[(1, "bbbb", false)]));
        script.will("digest:/other/pulls", digest_of(&[(2, "cccc", false)]));
        script.will("digest:/other/pulls", digest_of(&[(2, "cccc", false)]));
        let mut state = PollState::default();
        let now = Instant::now();
        let request = request(vec![project("/app", &[]), project("/other", &[])]);
        run(&mut state, &script, &request, now, 1000);
        let answer = run(&mut state, &script, &request, now, 1060);
        assert_eq!(answer.changed, vec!["/app".to_owned()]);
    }

    #[test]
    fn a_comment_that_moves_the_tag_but_not_the_digest_asks_nothing_and_keeps_the_new_tag() {
        let script = Script::new();
        script.will("/app/pulls", ok("one"));
        script.will("/app/pulls", ok("two"));
        script.will(
            "digest:/app/pulls",
            digest_of(&[(1, "aaaa", false), (2, "bbbb", true)]),
        );
        script.will(
            "digest:/app/pulls",
            digest_of(&[(1, "aaaa", false), (2, "bbbb", true)]),
        );
        let mut state = PollState::default();
        let now = Instant::now();
        let request = request(vec![project("/app", &[])]);
        run(&mut state, &script, &request, now, 1000);
        let answer = run(&mut state, &script, &request, now, 1060);
        assert!(
            answer.changed.is_empty(),
            "same pull requests, same heads, same drafts"
        );
        assert_eq!(
            script.digest_reads(),
            2,
            "the digest is read when the tag moves, not on a 304"
        );
        script.take_asked();
        run(&mut state, &script, &request, now, 1120);
        assert!(
            script.take_asked()[0].contains(&"If-None-Match: W/\"two\"".to_owned()),
            "the next minute sends the tag the comment moved it to"
        );
        assert_eq!(script.digest_reads(), 0, "a 304 reads no digest");
    }

    #[test]
    fn a_new_push_a_draft_flip_an_opened_and_a_closed_pull_request_each_ask() {
        let base = [(1, "aaaa", false), (2, "bbbb", false)];
        for (what, next) in [
            ("a push", vec![(1, "zzzz", false), (2, "bbbb", false)]),
            ("a draft flip", vec![(1, "aaaa", true), (2, "bbbb", false)]),
            (
                "an opened pull request",
                vec![(3, "cccc", false), (1, "aaaa", false), (2, "bbbb", false)],
            ),
            ("a pull request leaving the list", vec![(1, "aaaa", false)]),
        ] {
            let script = Script::new();
            script.will("/app/pulls", ok("one"));
            script.will("/app/pulls", ok("two"));
            script.will("digest:/app/pulls", digest_of(&base));
            script.will("digest:/app/pulls", digest_of(&next));
            let mut state = PollState::default();
            let now = Instant::now();
            let request = request(vec![project("/app", &[])]);
            run(&mut state, &script, &request, now, 1000);
            let answer = run(&mut state, &script, &request, now, 1060);
            assert_eq!(answer.changed, vec!["/app".to_owned()], "{what}");
        }
    }

    #[test]
    fn a_digest_that_cannot_be_read_is_a_failure_and_the_tag_is_not_kept() {
        let script = Script::new();
        script.will("/app/pulls", ok("one"));
        script.will("digest:/app/pulls", Ok("not a list".to_owned()));
        let mut state = PollState::default();
        let answer = run(
            &mut state,
            &script,
            &request(vec![project("/app", &[])]),
            Instant::now(),
            1000,
        );
        assert!(answer.changed.is_empty());
        assert_eq!(state.pauses["/app"].kind, "network_or_rate_limit");
        assert!(
            state.tags.is_empty(),
            "a baseline that was not completed is not kept"
        );
    }

    #[test]
    fn a_pending_checks_commit_is_watched_through_its_runs_and_its_status() {
        let script = Script::new();
        let mut state = PollState::default();
        let now = Instant::now();
        let request = request(vec![project("/app", &[HEAD])]);
        run(&mut state, &script, &request, now, 1000);
        assert_eq!(
            script.asked_endpoints(),
            vec![
                "repos/acme/app/pulls?state=open&per_page=100".to_owned(),
                format!("repos/acme/app/commits/{HEAD}/check-runs?per_page=100"),
                format!("repos/acme/app/commits/{HEAD}/status"),
            ]
        );
    }

    /// One pass over a project with one pending head, the runs list's tag moving
    /// and the runs list adding up to `after`, after a first pass that added up
    /// to `before`.
    fn checks_answer(before: &str, after: &str) -> Vec<String> {
        let script = Script::new();
        script.will("/app/commits", ok("runs-1"));
        script.will("/app/commits", ok("status-1"));
        script.will("/app/commits", ok("runs-2"));
        script.will("digest:check-runs", Ok(format!("{before}\n")));
        script.will("digest:check-runs", Ok(format!("{after}\n")));
        script.will("digest:/status", Ok("none\n".to_owned()));
        let mut state = PollState::default();
        let now = Instant::now();
        let request = request(vec![project("/app", &[HEAD])]);
        run(&mut state, &script, &request, now, 1000);
        run(&mut state, &script, &request, now, 1060).changed
    }

    #[test]
    fn one_job_finishing_while_others_are_pending_asks_nothing() {
        assert!(checks_answer("pending", "pending").is_empty());
    }

    #[test]
    fn the_last_job_finishing_asks_one_read_and_so_does_a_failure() {
        assert_eq!(checks_answer("pending", "passing"), ["/app"]);
        assert_eq!(checks_answer("pending", "failed"), ["/app"]);
        assert_eq!(
            checks_answer("failed", "pending"),
            ["/app"],
            "a re-run starting"
        );
        assert!(checks_answer("passing", "passing").is_empty());
    }

    #[test]
    fn a_status_and_runs_that_add_up_to_the_same_mark_ask_nothing() {
        // Runs pass while a status stays failed: the mark stays failed.
        let script = Script::new();
        script.will("/app/commits", ok("runs-1"));
        script.will("/app/commits", ok("status-1"));
        script.will("/app/commits", ok("runs-2"));
        script.will("digest:check-runs", Ok("failed\n".to_owned()));
        script.will("digest:check-runs", Ok("passing\n".to_owned()));
        script.will("digest:/status", Ok("failed\n".to_owned()));
        let mut state = PollState::default();
        let now = Instant::now();
        let request = request(vec![project("/app", &[HEAD])]);
        run(&mut state, &script, &request, now, 1000);
        assert!(
            run(&mut state, &script, &request, now, 1060)
                .changed
                .is_empty()
        );
    }

    #[test]
    fn a_verdict_that_cannot_be_read_is_a_failure_and_no_baseline_is_kept() {
        let script = Script::new();
        script.will("/app/commits", ok("runs-1"));
        script.will("digest:check-runs", Ok("maybe\n".to_owned()));
        let mut state = PollState::default();
        let answer = run(
            &mut state,
            &script,
            &request(vec![project("/app", &[HEAD])]),
            Instant::now(),
            1000,
        );
        assert!(answer.changed.is_empty());
        assert_eq!(state.pauses["/app"].kind, "network_or_rate_limit");
        assert!(state.combined.is_empty());
    }

    #[test]
    fn a_failure_pauses_that_project_alone_and_it_comes_back_after_its_backoff() {
        let script = Script::new();
        script.will("/bad/pulls", broken());
        let mut state = PollState::default();
        let now = Instant::now();
        let request = request(vec![project("/bad", &[]), project("/good", &[])]);
        run(&mut state, &script, &request, now, 1000);
        script.take_asked();
        run(
            &mut state,
            &script,
            &request,
            now + Duration::from_secs(59),
            1059,
        );
        assert_eq!(
            script.asked_endpoints(),
            vec!["repos/acme/good/pulls?state=open&per_page=100".to_owned()],
            "the failed project is not asked while its pause lasts, the other is"
        );
        script.take_asked();
        run(
            &mut state,
            &script,
            &request,
            now + Duration::from_secs(60),
            1060,
        );
        assert!(
            script
                .asked_endpoints()
                .iter()
                .any(|endpoint| endpoint.contains("/bad/")),
            "after the first pause it is asked again"
        );
    }

    #[test]
    fn each_failure_waits_twice_as_long_up_to_fifteen_minutes() {
        let script = Script::new();
        let mut state = PollState::default();
        let now = Instant::now();
        let request = request(vec![project("/bad", &[])]);
        let mut waits = Vec::new();
        for _ in 0..7 {
            script.will("/bad/pulls", broken());
            let before = state.pauses.get("/bad").map_or(0, |pause| pause.failures);
            let at = state.pauses.get("/bad").map_or(now, |pause| pause.until);
            run(&mut state, &script, &request, at, 1000);
            let pause = state.pauses.get("/bad").expect("paused");
            assert_eq!(pause.failures, before + 1);
            waits.push(pause.until.duration_since(at).as_secs());
        }
        assert_eq!(waits, vec![60, 120, 240, 480, 900, 900, 900]);
    }

    #[test]
    fn a_failing_project_changes_nothing_a_caller_can_see_and_a_success_ends_its_pause() {
        let script = Script::new();
        script.will("/app/pulls", broken());
        script.will("/app/pulls", ok("one"));
        let mut state = PollState::default();
        let now = Instant::now();
        let request = request(vec![project("/app", &[])]);
        let answer = run(&mut state, &script, &request, now, 1000);
        assert!(answer.changed.is_empty());
        assert_eq!(answer.held, ["/app"], "a paused project's marks go stale");
        assert!(state.pauses.contains_key("/app"));
        let later = state.pauses["/app"].until;
        let answer = run(&mut state, &script, &request, later, 1100);
        assert!(state.pauses.is_empty(), "a good answer ends the pause");
        assert!(answer.held.is_empty());
    }

    #[test]
    fn below_a_tenth_of_the_limit_nothing_is_polled_until_it_resets() {
        let script = Script::new();
        script.will("/a/pulls", ok_with("one", 499, 5000));
        let mut state = PollState::default();
        let now = Instant::now();
        let request = request(vec![project("/a", &[]), project("/b", &[])]);
        run(&mut state, &script, &request, now, 1000);
        assert_eq!(
            script.asked_endpoints().len(),
            1,
            "the second project is not asked"
        );
        assert_eq!(state.rate_until, Some(2000));
        script.take_asked();
        let answer = run(&mut state, &script, &request, now, 1999);
        assert!(
            script.asked_endpoints().is_empty(),
            "stopped until the reset"
        );
        assert_eq!(
            answer.held,
            ["/a", "/b"],
            "every mark goes stale at the floor"
        );
        run(&mut state, &script, &request, now, 2000);
        assert!(
            !script.asked_endpoints().is_empty(),
            "polling resumes at the reset"
        );
        assert_eq!(state.rate_until, None);
    }

    #[test]
    fn exactly_a_tenth_left_is_not_below_the_floor() {
        let script = Script::new();
        script.will("/a/pulls", ok_with("one", 500, 5000));
        let mut state = PollState::default();
        run(
            &mut state,
            &script,
            &request(vec![project("/a", &[]), project("/b", &[])]),
            Instant::now(),
            1000,
        );
        assert_eq!(state.rate_until, None);
        assert_eq!(script.asked_endpoints().len(), 2);
    }

    #[test]
    fn tags_are_kept_only_for_what_the_request_still_names() {
        let script = Script::new();
        script.will("/a/pulls", ok("a"));
        script.will("/b/pulls", ok("b"));
        let mut state = PollState::default();
        let now = Instant::now();
        run(
            &mut state,
            &script,
            &request(vec![project("/a", &[]), project("/b", &[])]),
            now,
            1000,
        );
        assert_eq!(state.tags.len(), 2);
        run(
            &mut state,
            &script,
            &request(vec![project("/a", &[])]),
            now,
            1060,
        );
        assert_eq!(state.tags.len(), 1);
        assert!(state.tags.keys().all(|endpoint| endpoint.contains("/a/")));
    }

    #[test]
    fn a_reply_without_a_tag_or_in_a_form_not_read_is_a_failure_not_a_change() {
        let script = Script::new();
        script.will("/a/pulls", Ok("HTTP/2.0 200 OK\r\n\r\n3\n".to_owned()));
        script.will("/b/pulls", Ok("not headers".to_owned()));
        let mut state = PollState::default();
        let answer = run(
            &mut state,
            &script,
            &request(vec![project("/a", &[]), project("/b", &[])]),
            Instant::now(),
            1000,
        );
        assert!(answer.changed.is_empty());
        assert_eq!(state.pauses["/a"].kind, "reply_without_tag");
        assert_eq!(state.pauses["/b"].kind, "reply_unreadable");
    }

    #[test]
    fn a_cancelled_pass_asks_nothing() {
        let script = Script::new();
        let mut state = PollState::default();
        let cancel = AtomicBool::new(true);
        pass(
            &mut state,
            &request(vec![project("/a", &[])]),
            &|arguments| script.ask(arguments),
            Clock {
                now: Instant::now(),
                unix: 1000,
            },
            &cancel,
        );
        assert!(script.asked_endpoints().is_empty());
    }
}
