//! A pull request's decision card (PRD work-tab-pr-lens D-06, D-21, D-22):
//! what it reads beyond the 5-minute list, once per head commit, and the two
//! GitHub writes the operator can ask from it.
//!
//! Both go through the node's `gh` and its allowlist, so a body's text or a
//! file's name can never become a command line. The writes are conditioned on
//! the head commit the card showed; GitHub refuses them when it has moved.

use hide_node_link::gh::{
    PrMergeMethod, pr_card_query, pr_merge_arguments, pr_update_branch_arguments,
};
use serde_json::Value;

use super::{GhFailure, Where, gh_at, issue_repository, parse_rfc3339_ms};
use crate::model::{PrCardFile, PrCardMerge, PrCardThread};
use crate::node_access::NodeLink;

/// The longest a review thread's first sentence is kept.
const THREAD_TEXT_LIMIT: usize = 160;

/// Lock files that are generated whatever the project's own attributes say.
const LOCK_FILES: [&str; 9] = [
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lockb",
    "Cargo.lock",
    "go.sum",
    "Gemfile.lock",
    "poetry.lock",
    "composer.lock",
];

/// What one card read found. A part that could not be understood is `None`
/// and named in `failed`; the others stand (B52).
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct CardRead {
    pub head: Option<String>,
    pub body: Option<String>,
    pub files: Option<Vec<PrCardFile>>,
    pub files_total: u32,
    pub threads: Option<Vec<PrCardThread>>,
    pub merge: Option<PrCardMerge>,
    pub failed: Vec<&'static str>,
    /// When the head commit's checks last completed: the latest completed
    /// run's time (PRD pr-card-and-session-record-redesign D-15); `None`
    /// while none has completed or the answer lacks it.
    pub checks_completed_at: Option<u64>,
    /// When the review that stands (the latest approval or change request)
    /// was submitted; `None` without one.
    pub review_at: Option<u64>,
    /// The time parts the answer did not carry in a readable shape
    /// (`checks`, `review`): the card stands without them and the caller
    /// logs why (B21). A part that is there and empty is not named.
    pub times_unread: Vec<&'static str>,
}

/// Reads the card of pull request `number`; `Err` when GitHub could not be
/// asked at all, which fails every part.
pub(crate) fn read(node: &dyn NodeLink, place: &Where, number: u32) -> Result<CardRead, String> {
    let repository = issue_repository(node, place)?;
    let query = pr_card_query(&repository, number)
        .ok_or_else(|| "the repository name cannot be asked about a card".to_owned())?;
    let output = gh_at(node, place, &["api", "graphql", "-f", &query]).map_err(|f| f.reason)?;
    parse(&output)
}

pub(crate) fn parse(output: &str) -> Result<CardRead, String> {
    let value: Value = serde_json::from_str(output)
        .map_err(|error| format!("gh returned a card Hide could not read: {error}"))?;
    let repository = &value["data"]["repository"];
    if repository.is_null() {
        return Err("gh returned no repository for the card".to_owned());
    }
    let pull_request = &repository["pullRequest"];
    if pull_request.is_null() {
        return Err("gh returned no such pull request".to_owned());
    }
    let mut read = CardRead {
        head: pull_request["headRefOid"].as_str().map(str::to_owned),
        ..CardRead::default()
    };
    match pull_request["body"].as_str() {
        Some(body) => read.body = Some(capped_body(body)),
        None => read.failed.push("body"),
    }
    let attributes = repository["attributes"]["text"]
        .as_str()
        .unwrap_or_default();
    let patterns = generated_patterns(attributes);
    match pull_request["files"]["nodes"].as_array() {
        Some(nodes) => {
            read.files = Some(
                nodes
                    .iter()
                    .filter_map(|node| {
                        let path = node["path"].as_str()?;
                        Some(PrCardFile {
                            path: path.to_owned(),
                            additions: node["additions"].as_u64().unwrap_or(0) as u32,
                            deletions: node["deletions"].as_u64().unwrap_or(0) as u32,
                            generated: is_generated(&patterns, path),
                        })
                    })
                    .collect(),
            );
            read.files_total = pull_request["files"]["totalCount"]
                .as_u64()
                .unwrap_or(nodes.len() as u64) as u32;
        }
        None => read.failed.push("files"),
    }
    match pull_request["reviewThreads"]["nodes"].as_array() {
        Some(nodes) => {
            read.threads = Some(
                nodes
                    .iter()
                    .filter(|node| node["isResolved"].as_bool() == Some(false))
                    .map(|node| {
                        let first = &node["comments"]["nodes"][0];
                        PrCardThread {
                            reviewer: first["author"]["login"].as_str().map(str::to_owned),
                            path: node["path"].as_str().unwrap_or_default().to_owned(),
                            line: node["line"].as_u64().map(|line| line as u32),
                            text: first_sentence(first["body"].as_str().unwrap_or_default()),
                        }
                    })
                    .collect(),
            );
        }
        None => read.failed.push("threads"),
    }
    let allowed = |field: &str| repository[field].as_bool().unwrap_or(false);
    let default_branch = repository["defaultBranchRef"]["name"].as_str();
    let base = pull_request["baseRefName"].as_str();
    read.checks_completed_at = checks_completed_at(pull_request);
    read.review_at = review_at(pull_request);
    if !pull_request["commits"]["nodes"].is_array() {
        read.times_unread.push("checks");
    }
    if !pull_request["reviews"]["nodes"].is_array() {
        read.times_unread.push("review");
    }
    read.merge = Some(PrCardMerge {
        methods: [
            (PrMergeMethod::Merge, "mergeCommitAllowed"),
            (PrMergeMethod::Squash, "squashMergeAllowed"),
            (PrMergeMethod::Rebase, "rebaseMergeAllowed"),
        ]
        .into_iter()
        .filter(|(_, field)| allowed(field))
        .map(|(method, _)| method)
        .collect(),
        // The repository's queue is its default branch's; a pull request to
        // another base is merged directly.
        queue: !repository["mergeQueue"].is_null()
            && default_branch.is_some()
            && default_branch == base,
        can_merge: matches!(
            repository["viewerPermission"].as_str(),
            Some("WRITE" | "MAINTAIN" | "ADMIN")
        ),
    });
    Ok(read)
}

/// The latest time a check of the head commit completed: a check run's
/// `completedAt`, or a commit status's `createdAt`, which is when its state
/// was set. A run still queued or in progress has none and is passed over.
fn checks_completed_at(pull_request: &Value) -> Option<u64> {
    pull_request["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"]
        .as_array()?
        .iter()
        .filter_map(|node| {
            let at = match node["__typename"].as_str()? {
                "CheckRun" => node["completedAt"].as_str()?,
                "StatusContext" => node["createdAt"].as_str()?,
                _ => return None,
            };
            parse_rfc3339_ms(at)
        })
        .max()
}

/// When the review that decides the pull request was submitted: the latest
/// approval or change request, as reviews come oldest first.
fn review_at(pull_request: &Value) -> Option<u64> {
    pull_request["reviews"]["nodes"]
        .as_array()?
        .iter()
        .rev()
        .find(|node| {
            matches!(
                node["state"].as_str(),
                Some("APPROVED" | "CHANGES_REQUESTED")
            )
        })
        .and_then(|node| parse_rfc3339_ms(node["submittedAt"].as_str()?))
}

/// How much of a body the card keeps: GitHub allows 65,536 characters, and
/// up to eight cards are kept and sent whole, so the tail past this is cut
/// at a character boundary (engineering principle 15).
const BODY_CAP_CHARS: usize = 20_000;

fn capped_body(body: &str) -> String {
    match body.char_indices().nth(BODY_CAP_CHARS) {
        Some((end, _)) => body[..end].to_owned(),
        None => body.to_owned(),
    }
}

fn first_sentence(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let line = line.split_once(". ").map_or(line, |(sentence, _)| sentence);
    match line.char_indices().nth(THREAD_TEXT_LIMIT) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_owned(),
    }
}

/// The patterns of `.gitattributes` lines that mark a path
/// `linguist-generated` (set, not unset and not `=false`).
pub(crate) fn generated_patterns(attributes: &str) -> Vec<String> {
    attributes
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let mut words = line.split_whitespace();
            let pattern = words.next()?;
            let mut generated = false;
            for word in words {
                match word {
                    "linguist-generated" | "linguist-generated=true" => generated = true,
                    "-linguist-generated" | "linguist-generated=false" | "!linguist-generated" => {
                        generated = false
                    }
                    _ => {}
                }
            }
            generated.then(|| pattern.to_owned())
        })
        .collect()
}

pub(crate) fn is_generated(patterns: &[String], path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    LOCK_FILES.contains(&name)
        || patterns
            .iter()
            .any(|pattern| attribute_matches(pattern, path))
}

/// A `.gitattributes` pattern against a path: a pattern with no slash
/// matches a name at any depth, one with a slash is anchored at the
/// repository root, `*` stays inside a name and `**` crosses folders.
fn attribute_matches(pattern: &str, path: &str) -> bool {
    let anchored = pattern.trim_start_matches('/');
    if pattern.starts_with('/') || anchored.contains('/') {
        let anchored = anchored.trim_end_matches('/');
        return glob(anchored.as_bytes(), path.as_bytes())
            || (pattern.ends_with('/') && path.starts_with(&format!("{anchored}/")));
    }
    path.split('/')
        .any(|part| glob(pattern.as_bytes(), part.as_bytes()))
}

fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern {
        [] => text.is_empty(),
        [b'*', b'*', b'/', rest @ ..] => {
            glob(rest, text)
                || text
                    .iter()
                    .position(|byte| *byte == b'/')
                    .is_some_and(|slash| glob(pattern, &text[slash + 1..]))
        }
        [b'*', b'*'] => true,
        [b'*', rest @ ..] => {
            let stop = text
                .iter()
                .position(|byte| *byte == b'/')
                .unwrap_or(text.len());
            (0..=stop).any(|skip| glob(rest, &text[skip..]))
        }
        [b'?', rest @ ..] => text
            .split_first()
            .is_some_and(|(byte, tail)| *byte != b'/' && glob(rest, tail)),
        [byte, rest @ ..] => text
            .split_first()
            .is_some_and(|(found, tail)| found == byte && glob(rest, tail)),
    }
}

/// One GitHub write asked from a card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Write {
    Merge {
        number: u32,
        method: Option<PrMergeMethod>,
        head: String,
    },
    UpdateBranch {
        number: u32,
        head: String,
    },
}

/// Why a write did not happen: a code the card words, and `gh`'s own text for
/// the diagnostic log.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct WriteFailure {
    pub reason: &'static str,
    pub detail: String,
}

/// Runs one write. A merge of a pull request that is already merged is a
/// success, so asking again after a lost answer converges (D-22).
pub(crate) fn run_write(
    node: &dyn NodeLink,
    place: &Where,
    write: &Write,
) -> Result<(), WriteFailure> {
    let refused = |detail: &str| WriteFailure {
        reason: "failed",
        detail: detail.to_owned(),
    };
    let arguments = match write {
        Write::Merge {
            number,
            method,
            head,
        } => pr_merge_arguments(*number, *method, head)
            .ok_or_else(|| refused("the head commit is not a full commit name"))?,
        Write::UpdateBranch { number, head } => {
            let repository = issue_repository(node, place).map_err(|detail| refused(&detail))?;
            pr_update_branch_arguments(&repository, *number, head)
                .ok_or_else(|| refused("the repository or head commit cannot be named"))?
        }
    };
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match gh_at(node, place, &arguments) {
        Ok(_) => Ok(()),
        Err(failure) => match classify(&failure) {
            "already_merged" if matches!(write, Write::Merge { .. }) => Ok(()),
            reason => Err(WriteFailure {
                reason: if reason == "already_merged" {
                    "failed"
                } else {
                    reason
                },
                detail: failure.reason,
            }),
        },
    }
}

/// What a `gh` failure means to the operator, from the text it gave. The
/// text is `gh`'s only channel for it, so this is the one place it is read.
fn classify(failure: &GhFailure) -> &'static str {
    let text = failure.reason.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| text.contains(needle));
    if has(&["already merged"]) {
        "already_merged"
    } else if has(&[
        "head branch was modified",
        "head sha",
        "expected_head_sha",
        "does not match",
        "match-head-commit",
        "head commit",
    ]) {
        "head_moved"
    } else if has(&["merge conflict", "conflict"]) {
        "conflict"
    } else if has(&[
        "not mergeable",
        "required status check",
        "review required",
        "base branch policy",
        "branch protection",
        "merge queue",
    ]) {
        "not_mergeable"
    } else if has(&[
        "resource not accessible",
        "403",
        "forbidden",
        "permission",
        "must have admin",
        "not authorized",
    ]) {
        "denied"
    } else if has(&[
        "timed out",
        "could not resolve",
        "connection",
        "network",
        "dial tcp",
    ]) {
        "unreachable"
    } else {
        "failed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_node_link::gh::GithubFailureCategory;

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn a_body_past_the_cap_is_cut_at_a_character_boundary() {
        let long = "가".repeat(BODY_CAP_CHARS + 5);
        let kept = capped_body(&long);
        assert_eq!(kept.chars().count(), BODY_CAP_CHARS);
        assert_eq!(capped_body("short"), "short");
    }

    const CARD: &str = r##"{"data":{"repository":{
        "mergeCommitAllowed":true,"squashMergeAllowed":true,"rebaseMergeAllowed":false,
        "viewerPermission":"WRITE","defaultBranchRef":{"name":"main"},"mergeQueue":{"id":"MQ"},
        "attributes":{"text":"# generated\n*.snap linguist-generated\ndocs/api/** linguist-generated=true\nkeep.rs linguist-generated=false\n"},
        "pullRequest":{"headRefOid":"0123456789abcdef0123456789abcdef01234567","baseRefName":"main","body":"Hello\n## Plan\nx",
          "files":{"totalCount":3,"nodes":[
            {"path":"src/a.rs","additions":10,"deletions":2},
            {"path":"web/pnpm-lock.yaml","additions":500,"deletions":300},
            {"path":"tests/__snapshots__/x.snap","additions":9,"deletions":0}]},
          "reviewThreads":{"nodes":[
            {"isResolved":false,"path":"src/a.rs","line":7,"comments":{"nodes":[{"author":{"login":"ann"},"body":"Please rename this. It reads badly."}]}},
            {"isResolved":true,"path":"src/b.rs","line":1,"comments":{"nodes":[{"author":{"login":"bob"},"body":"done"}]}}]},
          "commits":{"nodes":[{"commit":{"statusCheckRollup":{"contexts":{"nodes":[
            {"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS","completedAt":"2026-10-16T11:21:00Z"},
            {"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":null,"completedAt":null},
            {"__typename":"StatusContext","state":"SUCCESS","createdAt":"2026-10-16T11:05:30Z"}]}}}}]},
          "reviews":{"nodes":[
            {"state":"CHANGES_REQUESTED","submittedAt":"2026-10-16T09:00:00Z"},
            {"state":"COMMENTED","submittedAt":"2026-10-16T12:00:00Z"},
            {"state":"APPROVED","submittedAt":"2026-10-16T11:30:00Z"}]}}}}}"##;

    #[test]
    fn a_card_read_keeps_the_head_the_files_the_open_threads_and_what_a_merge_may_be() {
        let read = parse(CARD).unwrap();
        assert_eq!(read.head.as_deref(), Some(HEAD));
        assert!(read.body.as_deref().unwrap().contains("## Plan"));
        let files = read.files.unwrap();
        assert_eq!(read.files_total, 3);
        assert_eq!(
            files
                .iter()
                .map(|file| (file.path.as_str(), file.generated))
                .collect::<Vec<_>>(),
            [
                ("src/a.rs", false),
                ("web/pnpm-lock.yaml", true),
                ("tests/__snapshots__/x.snap", true)
            ]
        );
        let threads = read.threads.unwrap();
        assert_eq!(threads.len(), 1, "a resolved thread is not shown");
        assert_eq!(threads[0].reviewer.as_deref(), Some("ann"));
        assert_eq!(threads[0].text, "Please rename this");
        assert_eq!(threads[0].line, Some(7));
        let merge = read.merge.unwrap();
        assert_eq!(
            merge.methods,
            vec![PrMergeMethod::Merge, PrMergeMethod::Squash]
        );
        assert!(merge.queue && merge.can_merge);
        assert!(read.failed.is_empty());
        // The latest completed check, a running one passed over, and the
        // latest decisive review, a comment-only one passed over (D-15).
        assert_eq!(
            read.checks_completed_at,
            parse_rfc3339_ms("2026-10-16T11:21:00Z")
        );
        assert_eq!(read.review_at, parse_rfc3339_ms("2026-10-16T11:30:00Z"));
    }

    #[test]
    fn a_part_the_answer_lacks_is_named_failed_while_the_others_stand() {
        let read = parse(
            r#"{"data":{"repository":{"viewerPermission":"READ","defaultBranchRef":{"name":"main"},"mergeQueue":null,
            "pullRequest":{"headRefOid":"abc","baseRefName":"dev","body":null,"files":null,"reviewThreads":{"nodes":[]}}}}}"#,
        )
        .unwrap();
        assert_eq!(read.failed, vec!["body", "files"]);
        assert_eq!(read.threads, Some(Vec::new()));
        // An answer without the times leaves them absent and names the parts
        // for the log; the card stands (B21).
        assert_eq!((read.checks_completed_at, read.review_at), (None, None));
        assert_eq!(read.times_unread, vec!["checks", "review"]);
        let merge = read.merge.unwrap();
        assert!(!merge.queue && !merge.can_merge && merge.methods.is_empty());
        assert!(parse(r#"{"data":{"repository":null}}"#).is_err());
        assert!(parse(r#"{"data":{"repository":{"pullRequest":null}}}"#).is_err());
        assert!(parse("nope").is_err());
    }

    #[test]
    fn a_queue_on_the_default_branch_does_not_make_another_base_queued() {
        let read =
            parse(&CARD.replace(r#""baseRefName":"main""#, r#""baseRefName":"release""#)).unwrap();
        assert!(!read.merge.unwrap().queue);
    }

    #[test]
    fn generated_files_are_what_the_project_marks_and_the_common_lock_files() {
        let patterns = generated_patterns(
            "# c\n*.min.js linguist-generated\n/dist/ linguist-generated\nsrc/gen/**/*.rs linguist-generated=true\nold.rs -linguist-generated\nplain.rs text\n",
        );
        assert_eq!(patterns, ["*.min.js", "/dist/", "src/gen/**/*.rs"]);
        for generated in [
            "a/b/app.min.js",
            "dist/x/y.js",
            "src/gen/deep/er/z.rs",
            "Cargo.lock",
            "app/yarn.lock",
        ] {
            assert!(is_generated(&patterns, generated), "{generated}");
        }
        for plain in [
            "src/main.rs",
            "src/gen.rs",
            "old.rs",
            "docs/dist.md",
            "src/gen/z.txt",
        ] {
            assert!(!is_generated(&patterns, plain), "{plain}");
        }
    }

    #[test]
    fn a_failure_is_worded_by_what_the_operator_can_do() {
        let failure = |reason: &str| GhFailure {
            category: GithubFailureCategory::NetworkOrRateLimit,
            reason: reason.to_owned(),
        };
        for (text, kind) in [
            ("X Pull request is already merged", "already_merged"),
            (
                "GraphQL: Head branch was modified. Review and try the merge again.",
                "head_moved",
            ),
            (
                "gh: Pull request Head sha didn't match expected_head_sha (HTTP 409)",
                "head_moved",
            ),
            (
                "Pull request is not mergeable: the base branch policy prohibits the merge",
                "not_mergeable",
            ),
            (
                "merge conflict between base and head (HTTP 422)",
                "conflict",
            ),
            (
                "Resource not accessible by personal access token (HTTP 403)",
                "denied",
            ),
            ("gh timed out after 15000 ms", "unreachable"),
            ("something else", "failed"),
        ] {
            assert_eq!(classify(&failure(text)), kind, "{text}");
        }
    }
}
